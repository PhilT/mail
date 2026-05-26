//! Shared library for the `mail-*` binaries.
//!
//! Right now this re-exports `mail-parser` and provides a thin convenience
//! wrapper for reading a Maildir file from disk. As more binaries land
//! (mail-archive, mail-expunge-old, ...) this module grows the shared
//! domain types — Envelope, action helpers, notmuch queries, etc.

use std::fs;
use std::path::Path;

pub use mail_parser::{Message, MessageParser};

/// Read and parse a Maildir message file. Owns the parsed `Message` so the
/// caller doesn't have to keep the byte buffer alive separately.
pub fn parse_file(path: &Path) -> anyhow::Result<Message<'static>> {
    let bytes = fs::read(path)?;
    let parsed = MessageParser::default()
        .parse(&bytes)
        .ok_or_else(|| anyhow::anyhow!("could not parse message at {}", path.display()))?;
    Ok(parsed.into_owned())
}
