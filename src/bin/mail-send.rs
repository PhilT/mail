//! Write a new message: pick a recipient, type a subject, then edit, attach
//! and send through `mail::compose`. Sends from himalaya's default account
//! unless `MAIL_ACCOUNT` names another.

use std::fs;
use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use mail::compose;

fn main() -> Result<()> {
    let Some(to) = compose::pick_recipient()? else {
        return Ok(());
    };
    if !to.contains('@') {
        bail!("not an address: {to}");
    }
    println!("To: {to}");
    print!("Subject: ");
    io::stdout().flush()?;
    let mut subject = String::new();
    io::stdin().read_line(&mut subject).context("read subject")?;

    let template = compose::himalaya(&[
        "template",
        "write",
        "-H",
        &format!("To:{to}"),
        "-H",
        &format!("Subject:{}", subject.trim()),
    ])
    .output()
    .context("spawn himalaya")?;
    if !template.status.success() {
        bail!(
            "himalaya template write exited {}: {}",
            template.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&template.stderr).trim()
        );
    }

    let draft = std::env::temp_dir().join(format!("mail-send-{}.eml", std::process::id()));
    fs::write(&draft, &template.stdout).context("write draft")?;
    let outcome = compose::edit(&draft);
    let _ = fs::remove_file(&draft);

    match outcome? {
        Some(message) => {
            compose::send(&message)?;
            println!("Sent.");
            compose::sync();
        }
        None => println!("Discarded."),
    }
    Ok(())
}
