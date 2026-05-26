//! Phase A: take a Maildir file path as a CLI arg, print its decoded headers
//! and plaintext body. Pure mail-parser smoke test — no TUI yet.

use std::path::PathBuf;

use anyhow::{Context, Result};
use mail_parser::Address;

fn main() -> Result<()> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .context("usage: mail-triage <maildir-file>")?
        .into();

    let msg = mail::parse_file(&path)?;

    println!("From:    {}", format_addrs(msg.from()));
    println!("To:      {}", format_addrs(msg.to()));
    println!(
        "Date:    {}",
        msg.date().map(|d| d.to_rfc3339()).unwrap_or_default()
    );
    println!(
        "Subject: {}",
        msg.subject().unwrap_or("(no subject)")
    );
    println!();
    println!("----");
    println!();

    // body_text(0) auto-converts HTML→text when the message only has HTML.
    match msg.body_text(0) {
        Some(body) => println!("{body}"),
        None => println!("(no text body)"),
    }

    Ok(())
}

fn format_addrs(addr: Option<&Address>) -> String {
    let Some(addr) = addr else {
        return String::new();
    };
    addr.iter()
        .map(|a| {
            let name = a.name.as_deref().unwrap_or("");
            let email = a.address.as_deref().unwrap_or("");
            if name.is_empty() {
                email.to_string()
            } else {
                format!("{name} <{email}>")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
