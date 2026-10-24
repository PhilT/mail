//! Writing and sending a message, shared by `mail-send` and `mail-triage`'s
//! reply. The draft is edited in $EDITOR, then a menu offers Send, Attach,
//! Edit or Discard. Sending goes through `himalaya template send`, which turns
//! the MML `<#part>` lines appended here into attachments.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

/// Where the attachment picker looks. `/data/mail` is left out.
const ATTACH_ROOT: &str = "/data";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Send,
    Attach,
    Edit,
    Discard,
}

impl Choice {
    pub fn from_key(c: char) -> Option<Choice> {
        match c {
            's' => Some(Choice::Send),
            'a' => Some(Choice::Attach),
            'e' => Some(Choice::Edit),
            'd' => Some(Choice::Discard),
            _ => None,
        }
    }
}

/// Files to attach, kept out of the draft so Edit shows only what was typed.
#[derive(Debug, Default)]
pub struct Attachments(Vec<PathBuf>);

impl Attachments {
    /// MML quotes the filename but has no escape inside the quotes, so a path
    /// containing `"` or `\` can't be written and is refused.
    pub fn add(&mut self, path: PathBuf) -> Result<()> {
        let Some(s) = path.to_str() else {
            bail!("can't attach {}: the path isn't UTF-8", path.display());
        };
        if s.contains(['"', '\\']) {
            bail!("can't attach {s}: MML can't quote a path containing \" or \\");
        }
        if !self.0.contains(&path) {
            self.0.push(path);
        }
        Ok(())
    }

    pub fn names(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
            .collect()
    }

    pub fn apply(&self, draft: &str) -> String {
        let mut out = draft.to_string();
        if self.0.is_empty() {
            return out;
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for path in &self.0 {
            out.push_str(&format!("<#part filename=\"{}\"><#/part>\n", path.display()));
        }
        out
    }
}

/// Opens `draft` in $EDITOR, then shows the menu until Send or Discard.
/// Returns the template to send, or None when discarded. Quitting the editor
/// with an error status (`:cq`) discards on the first edit and goes back to
/// the menu on later ones. Expects the terminal in cooked mode.
pub fn edit(draft: &Path) -> Result<Option<String>> {
    if !run_editor(draft)? {
        return Ok(None);
    }
    let mut attachments = Attachments::default();
    loop {
        let names = attachments.names();
        if !names.is_empty() {
            println!("Attached: {}", names.join(", "));
        }
        println!("[s]end  [a]ttach  [e]dit  [d]iscard");
        match read_choice()? {
            Choice::Send => {
                let text = fs::read_to_string(draft).context("read draft")?;
                return Ok(Some(attachments.apply(&text)));
            }
            Choice::Attach => {
                for path in pick_files()? {
                    if let Err(e) = attachments.add(path) {
                        println!("{e}");
                    }
                }
            }
            Choice::Edit => {
                run_editor(draft)?;
            }
            Choice::Discard => {
                println!("Discard this draft? [y/n]");
                if read_key()?.code == KeyCode::Char('y') {
                    return Ok(None);
                }
            }
        }
    }
}

/// fzf over everyone this account has sent mail to. Enter takes the
/// highlighted address, or what was typed when nothing matches; Alt-Enter
/// always takes what was typed. None on Esc.
pub fn pick_recipient() -> Result<Option<String>> {
    let me = Command::new("notmuch")
        .args(["config", "get", "user.primary_email"])
        .output()
        .context("spawn notmuch")?;
    let me = String::from_utf8_lossy(&me.stdout).trim().to_string();
    let mut addresses = Command::new("notmuch")
        .args(["address", "--output=recipients", "--deduplicate=address"])
        .arg(format!("from:{me}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn notmuch")?;
    let picked = fzf(
        addresses.stdout.take().expect("stdout is piped"),
        &[
            "--prompt",
            "To: ",
            "--header",
            "Enter picks · Alt-Enter uses what you typed · Esc cancels",
            "--bind",
            "enter:accept-or-print-query,alt-enter:print-query",
        ],
    )?;
    let _ = addresses.kill();
    let _ = addresses.wait();
    let to = picked.trim();
    Ok((!to.is_empty()).then(|| to.to_string()))
}

/// `himalaya <args>`, using the account MAIL_ACCOUNT names when it's set and
/// himalaya's default account otherwise.
pub fn himalaya(args: &[&str]) -> Command {
    let mut cmd = Command::new("himalaya");
    cmd.args(args);
    if let Some(account) = std::env::var_os("MAIL_ACCOUNT") {
        cmd.arg("--account").arg(account);
    }
    cmd
}

/// Sends an MML template and saves a copy to the account's Sent folder.
pub fn send(template: &str) -> Result<()> {
    let mut child = himalaya(&["template", "send"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn himalaya")?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(template.as_bytes())
        .context("write to himalaya")?;
    let out = child.wait_with_output().context("wait for himalaya")?;
    if !out.status.success() {
        bail!(
            "himalaya template send exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Runs a mail-sync cycle so the Sent copy reaches IMAP.
pub fn sync() {
    let _ = Command::new("/run/current-system/sw/bin/mail-sync")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// False when the editor exits with an error status.
fn run_editor(path: &Path) -> Result<bool> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
    let status = Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("spawn {editor}"))?;
    Ok(status.success())
}

/// fzf over every file under ATTACH_ROOT, skipping hidden and git-ignored
/// files. Tab marks several; Esc picks none.
fn pick_files() -> Result<Vec<PathBuf>> {
    let mut files = Command::new("fd")
        .args(["--type", "f", "--exclude", "/mail", ".", ATTACH_ROOT])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn fd")?;
    let picked = fzf(
        files.stdout.take().expect("stdout is piped"),
        &[
            "--multi",
            "--prompt",
            "Attach: ",
            "--header",
            "Tab marks several · Enter attaches · Esc cancels",
        ],
    )?;
    let _ = files.kill();
    let _ = files.wait();
    Ok(picked.lines().map(PathBuf::from).collect())
}

/// fzf draws on the terminal and prints only the selection to stdout.
fn fzf(input: impl Into<Stdio>, args: &[&str]) -> Result<String> {
    let out = Command::new("fzf")
        .args(args)
        .stdin(input)
        .stderr(Stdio::inherit())
        .output()
        .context("spawn fzf")?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn read_choice() -> Result<Choice> {
    loop {
        let key = read_key()?;
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(Choice::Discard);
        }
        if let KeyCode::Char(c) = key.code {
            if let Some(choice) = Choice::from_key(c) {
                return Ok(choice);
            }
        }
    }
}

fn read_key() -> Result<KeyEvent> {
    enable_raw_mode().context("enable raw mode")?;
    let key = loop {
        match event::read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => break Ok(k),
            Ok(_) => continue,
            Err(e) => break Err(e).context("read key"),
        }
    };
    let _ = disable_raw_mode();
    key
}
