//! Throwaway: scan INBOX for a message whose decoded HTML contains a
//! given substring. Helps pinpoint which message has weird image URLs.

use std::path::Path;

use anyhow::Result;
use mail_parser::MimeHeaders;

fn main() -> Result<()> {
    let needle = std::env::args().nth(1).expect("usage: mail-find <substring>");
    let mailroot = "/data/mail/namecheap/INBOX";
    for sub in ["cur", "new"] {
        let dir = Path::new(mailroot).join(sub);
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            let msg = match mail::parse_file(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mut matched = false;
            if let Some(p) = msg.html_part(0) {
                if let Some(s) = p.text_contents() {
                    if s.contains(&needle) {
                        matched = true;
                    }
                }
            }
            if let Some(p) = msg.text_part(0) {
                if let Some(s) = p.text_contents() {
                    if s.contains(&needle) {
                        matched = true;
                    }
                }
            }
            if matched {
                let subject = msg.subject().unwrap_or("(no subject)");
                println!("{}", path.display());
                println!("  Subject: {subject}");
            }
        }
    }
    Ok(())
}
