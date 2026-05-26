//! Shared library for the `mail-*` binaries.

pub mod app;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use mail_parser::{Message, MessageParser};
use mail_parser::MimeHeaders;

/// A lightweight summary of a Maildir message — enough to populate a list
/// view without having to keep the full parsed message in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub path: PathBuf,
    pub date_epoch: i64,        // for sorting; i64::MAX when unparseable
    pub date_display: String,   // pre-rendered, e.g. "2026-05-26T11:36+00:00"
    pub from: String,           // pre-rendered, e.g. "Foo <foo@bar>"
    pub subject: String,
    pub has_attachment: bool,
}

/// Width in columns used when laying out HTML-only body text. Wide enough
/// not to wrap aggressively; the TUI will re-wrap visually anyway.
const HTML_RENDER_WIDTH: usize = 120;

/// Pick the cleanest plaintext rendering of the message body.
///
/// Prefers the HTML part rendered through `html2text` when one exists —
/// many marketing senders (eBay, retailers) bundle raw CSS into their
/// text/plain alternative so it's not really "plain", whereas html2text
/// strips `<style>` / `<script>` cleanly. Falls back to the text/plain
/// part only when there's no HTML alternative.
pub fn render_body(msg: &Message<'_>) -> String {
    if msg.html_body_count() > 0 {
        if let Some(html) = msg.body_html(0) {
            return html2text::from_read(html.as_bytes(), HTML_RENDER_WIDTH)
                .unwrap_or_else(|_| html.into_owned());
        }
    }
    if msg.text_body_count() > 0 {
        if let Some(text) = msg.body_text(0) {
            return text.into_owned();
        }
    }
    String::new()
}

/// Collect every `image/*` MIME part that carries a `Content-ID`, keyed by
/// the bare CID (angle brackets stripped). The bytes are the decoded body
/// of the part — ready to write to a tempfile and hand to a renderer.
///
/// Used to resolve `<img src="cid:foo">` references in HTML bodies.
pub fn inline_images(msg: &Message<'_>) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    for part in msg.parts.iter() {
        let Some(ctype) = part.content_type() else { continue };
        if !ctype.ctype().eq_ignore_ascii_case("image") {
            continue;
        }
        let Some(cid) = part.content_id() else { continue };
        let cid = cid.trim_start_matches('<').trim_end_matches('>').to_string();
        out.insert(cid, part.contents().to_vec());
    }
    out
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
            let dt = msg.date();
            out.push(Envelope {
                path: path.clone(),
                date_epoch: dt.map(|d| d.to_timestamp()).unwrap_or(i64::MAX),
                date_display: dt.map(|d| d.to_rfc3339()).unwrap_or_default(),
                from: format_first_addr(&msg),
                subject: msg.subject().unwrap_or("").to_string(),
                has_attachment: msg.attachment_count() > 0,
            });
        }
    }
    out.sort_by_key(|e| e.date_epoch);
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
