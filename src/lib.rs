//! Shared library for the `mail-*` binaries.

pub mod app;

mod render;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub use mail_parser::{Message, MessageParser};
use mail_parser::MimeHeaders;
use ratatui::text::Line;

pub use render::render_body;

pub fn ensure_mime_headers(template: &[u8]) -> Vec<u8> {
    let (eol, boundary) = if let Some(pos) = find_subsequence(template, b"\r\n\r\n") {
        ("\r\n", pos)
    } else if let Some(pos) = find_subsequence(template, b"\n\n") {
        ("\n", pos)
    } else {
        return template.to_vec();
    };

    let headers = &template[..boundary];
    let lower: String = headers.iter().map(|b| b.to_ascii_lowercase() as char).collect();
    if lower.contains("\ncontent-type:") || lower.starts_with("content-type:") {
        return template.to_vec();
    }

    let mut injected = String::new();
    if !lower.contains("\nmime-version:") && !lower.starts_with("mime-version:") {
        injected.push_str("MIME-Version: 1.0");
        injected.push_str(eol);
    }
    injected.push_str("Content-Type: text/plain; charset=utf-8");
    injected.push_str(eol);
    injected.push_str("Content-Transfer-Encoding: 8bit");

    let mut out = Vec::with_capacity(template.len() + injected.len() + eol.len());
    out.extend_from_slice(&template[..boundary]);
    out.extend_from_slice(eol.as_bytes());
    out.extend_from_slice(injected.as_bytes());
    out.extend_from_slice(&template[boundary..]);
    out
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

pub fn url_label(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s, r),
        None => ("https", url),
    };
    let host_end = rest
        .find(|c: char| c == '/' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    let host = rest[..host_end].trim_start_matches("www.");
    let path = &rest[host_end..];
    let cut = path.find(|c: char| c == '?' || c == '#').unwrap_or(path.len());
    let ext = std::path::Path::new(&path[..cut])
        .extension()
        .and_then(|e| e.to_str());
    match ext {
        Some(e) => format!("({e}) {scheme}://{host}"),
        None => format!("{scheme}://{host}"),
    }
}

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

/// A single segment of a rendered body. Text runs carry pre-styled
/// `ratatui` lines (bold/italic/heading colour applied during parse).
/// Image runs are placeholders to be replaced by an actual image widget
/// at draw time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyRun {
    Text(Vec<Line<'static>>),
    /// Inline image referenced by Content-ID (cid:...).
    Image(String),
    /// Remote image referenced by absolute http(s) URL. Fetched on demand
    /// by the TUI when the user opts in.
    Url(String),
}

/// Return the message's explicit `text/plain` body part, or `None` if the
/// message has only HTML. (mail-parser indexes the HTML part into
/// `text_body` as a fallback, so we check the part's real subtype.)
pub(crate) fn body_text_only(msg: &Message<'_>) -> Option<String> {
    let part = msg.text_part(0)?;
    let sub = part
        .content_type()
        .and_then(|c| c.subtype())
        .unwrap_or("plain");
    if !sub.eq_ignore_ascii_case("plain") {
        return None;
    }
    part.text_contents().map(|s| s.to_string())
}

/// Return the message's `text/html` body part as a string, or `None` if
/// the message has no real HTML part.
pub(crate) fn body_html_only(msg: &Message<'_>) -> Option<String> {
    let part = msg.html_part(0)?;
    let sub = part.content_type()?.subtype()?;
    if !sub.eq_ignore_ascii_case("html") {
        return None;
    }
    part.text_contents().map(|s| s.to_string())
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

pub fn attachments(msg: &Message<'_>) -> Vec<Attachment> {
    msg.attachments()
        .map(|part| {
            let content_type = part
                .content_type()
                .map(|c| {
                    let main = c.ctype();
                    match c.subtype() {
                        Some(sub) => format!("{main}/{sub}"),
                        None => main.to_string(),
                    }
                })
                .unwrap_or_else(|| "application/octet-stream".to_string());
            Attachment {
                filename: part.attachment_name().map(|s| s.to_string()),
                content_type,
                bytes: part.contents().to_vec(),
            }
        })
        .collect()
}

pub fn save_attachments(msg: &Message<'_>, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let mut out = Vec::new();
    for (i, att) in attachments(msg).into_iter().enumerate() {
        let base = att
            .filename
            .as_deref()
            .map(sanitize_filename)
            .unwrap_or_else(|| format!("attachment-{i}"));
        let dest = unique_path(dir, &base);
        fs::write(&dest, &att.bytes)?;
        out.push(dest);
    }
    Ok(out)
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect()
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..=9999 {
        let p = dir.join(format!("{stem}-{n}{ext}"));
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{stem}-{}{ext}", std::process::id()))
}

/// Collect every `image/*` MIME part that carries a `Content-ID`, keyed by
/// the bare CID (angle brackets stripped). The bytes are the decoded body
/// of the part — ready to write to a tempfile and hand to a renderer.
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
