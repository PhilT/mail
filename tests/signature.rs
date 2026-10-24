//! signature() runs the real himalaya against a throwaway config, in its own
//! test binary because HIMALAYA_CONFIG is process-wide.

use std::fs;
use tempfile::tempdir;

const REPLY: &str = "From: Me <me@example.com>\n\
Subject: Re: hi\n\
In-Reply-To: <abc@example.com>\n\
\n\
On Mon, 1 Jan 2024, Someone wrote:\n\
> Hello\n";

#[test]
fn with_signature_puts_it_between_the_typing_line_and_the_quote() {
    assert_eq!(
        mail::compose::with_signature(REPLY, "Regards\nPhil\n"),
        "From: Me <me@example.com>\n\
         Subject: Re: hi\n\
         In-Reply-To: <abc@example.com>\n\
         \n\
         \n\
         \n\
         Regards\n\
         Phil\n\
         \n\
         On Mon, 1 Jan 2024, Someone wrote:\n\
         > Hello\n"
    );
}

#[test]
fn with_signature_leaves_the_reply_alone_when_there_is_no_signature() {
    assert_eq!(mail::compose::with_signature(REPLY, ""), REPLY);
}

#[test]
fn signature_is_the_accounts_signature_without_the_delimiter() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "[accounts.test]\n\
             default = true\n\
             email = \"test@example.com\"\n\
             signature-delim = \"-- \\n\"\n\
             signature = \"Regards\\nPhil Thompson\\nFounder & Director\\n\"\n\
             backend.type = \"maildir\"\n\
             backend.root-dir = \"{}\"\n",
            dir.path().display()
        ),
    )
    .unwrap();
    std::env::set_var("HIMALAYA_CONFIG", &config);

    assert_eq!(
        mail::compose::signature().unwrap(),
        "Regards\nPhil Thompson\nFounder & Director\n"
    );
}
