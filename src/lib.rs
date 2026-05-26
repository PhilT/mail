//! Shared library for the `mail-*` binaries.

use std::fs;
use std::path::{Path, PathBuf};

pub use mail_parser::{Message, MessageParser};

/// A lightweight summary of a Maildir message — enough to populate a list
/// view without having to keep the full parsed message in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub path: PathBuf,
    pub date: i64,             // unix epoch from the Date header
    pub from: String,          // already formatted, e.g. "Foo <foo@bar>"
    pub subject: String,
    pub has_attachment: bool,
}

/// Read and parse a Maildir message file. The returned `Message` owns its
/// byte buffer so the caller doesn't have to keep one alive separately.
pub fn parse_file(path: &Path) -> anyhow::Result<Message<'static>> {
    let bytes = fs::read(path)?;
    let parsed = MessageParser::default()
        .parse(&bytes)
        .ok_or_else(|| anyhow::anyhow!("could not parse message at {}", path.display()))?;
    Ok(parsed.into_owned())
}

/// List every message in `<mailroot>/INBOX/{cur,new}`, sorted by Date header
/// ascending (oldest first). Messages with an unparseable Date land at the
/// end, ordered by file mtime.
pub fn list_inbox(mailroot: &Path) -> anyhow::Result<Vec<Envelope>> {
    let mut out = Vec::new();
    for sub in ["INBOX/cur", "INBOX/new"] {
        let dir = mailroot.join(sub);
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let msg = match MessageParser::default().parse(&bytes) {
                Some(m) => m,
                None => continue,
            };
            out.push(Envelope {
                path: path.clone(),
                date: msg.date().map(|d| d.to_timestamp()).unwrap_or(i64::MAX),
                from: format_first_addr(&msg),
                subject: msg.subject().unwrap_or("").to_string(),
                has_attachment: msg.attachment_count() > 0,
            });
        }
    }
    out.sort_by_key(|e| e.date);
    Ok(out)
}

fn format_first_addr(msg: &Message<'_>) -> String {
    let Some(addr) = msg.from() else {
        return String::new();
    };
    let Some(first) = addr.first() else {
        return String::new();
    };
    let name = first.name.as_deref().unwrap_or("");
    let email = first.address.as_deref().unwrap_or("");
    if name.is_empty() {
        email.to_string()
    } else {
        format!("{name} <{email}>")
    }
}

/// Move a message into `<mailroot>/Archive/cur/`, stripping the source
/// folder's `,U=N` UID hint from the filename (Maildir UIDs are per-folder,
/// reusing one across folders collides). Returns the destination path.
pub fn archive(src: &Path, mailroot: &Path) -> anyhow::Result<PathBuf> {
    move_to_folder(src, &mailroot.join("Archive/cur"))
}

/// Move a message into `<mailroot>/Trash/cur/` with the same UID-strip
/// treatment as [`archive`]. Returns the destination path.
pub fn delete(src: &Path, mailroot: &Path) -> anyhow::Result<PathBuf> {
    move_to_folder(src, &mailroot.join("Trash/cur"))
}

fn move_to_folder(src: &Path, dest_dir: &Path) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(dest_dir)?;
    let name = src
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("source has no filename: {}", src.display()))?
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("source filename is not UTF-8"))?;
    let stripped = strip_uid_hint(name);
    let dest = dest_dir.join(&stripped);
    fs::rename(src, &dest)?;
    Ok(dest)
}

fn strip_uid_hint(name: &str) -> String {
    // Maildir name format: "<unique>,U=<n>:<flags>" or "<unique>:<flags>".
    // Remove the ",U=<digits>" segment if present, keeping everything else.
    if let Some(start) = name.find(",U=") {
        let after = &name[start + 3..];
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| start + 3 + i)
            .unwrap_or(name.len());
        let mut out = String::with_capacity(name.len());
        out.push_str(&name[..start]);
        out.push_str(&name[end..]);
        out
    } else {
        name.to_string()
    }
}
