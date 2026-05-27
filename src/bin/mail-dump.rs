//! Throwaway: read one Maildir file, dump its html body + html-to-markdown
//! intermediate to /tmp/mail-triage-last.{html,md}. For debugging the
//! pipeline-C renderer on real-world messages without driving the TUI.

use std::path::PathBuf;

use anyhow::{Context, Result};
use mail_parser::MimeHeaders;

fn main() -> Result<()> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .context("usage: mail-dump <maildir-file>")?
        .into();
    let msg = mail::parse_file(&path)?;

    // Dump the text/plain part too, if any.
    if let Some(text_part) = msg.text_part(0) {
        if let Some(sub) = text_part.content_type().and_then(|c| c.subtype()) {
            println!("text part subtype = {sub}");
        }
        if let Some(text) = text_part.text_contents() {
            std::fs::write("/tmp/mail-triage-last.txt", text)?;
            println!("text bytes: {}  (-> /tmp/mail-triage-last.txt)", text.len());
        }
    }

    let part = msg.html_part(0).context("no html part")?;
    let sub = part
        .content_type()
        .and_then(|c| c.subtype())
        .context("no content type subtype")?;
    if !sub.eq_ignore_ascii_case("html") {
        anyhow::bail!("not text/html; subtype={sub}");
    }
    let html = part.text_contents().context("empty html part")?;
    std::fs::write("/tmp/mail-triage-last.html", html)?;

    let options = html_to_markdown_rs::ConversionOptions::builder()
        .wrap(true)
        .wrap_width(120)
        .build();
    let md = html_to_markdown_rs::convert(html, Some(options))
        .context("convert")?
        .content
        .unwrap_or_default();
    std::fs::write("/tmp/mail-triage-last.md", &md)?;

    println!("html bytes: {}", html.len());
    println!("md   bytes: {}", md.len());

    // And what does render_body think?
    let runs = mail::render_body(&msg);
    println!("render_body produced {} run(s):", runs.len());
    for (i, r) in runs.iter().enumerate() {
        match r {
            mail::BodyRun::Text(lines) => {
                let chars: usize = lines
                    .iter()
                    .flat_map(|l| l.spans.iter().map(|s| s.content.chars().count()))
                    .sum();
                println!("  [{i}] Text: {} line(s), {} chars", lines.len(), chars);
            }
            mail::BodyRun::Image(c) => println!("  [{i}] Image cid: {c}"),
            mail::BodyRun::Url(u) => println!("  [{i}] Url: {u}"),
        }
    }

    println!("written to /tmp/mail-triage-last.{{html,md,txt}}");
    Ok(())
}
