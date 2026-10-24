//! Runs the real himalaya against a throwaway config whose sendmail backend
//! writes the compiled message to a file, so nothing leaves the machine.

use std::fs;
use tempfile::tempdir;

#[test]
fn send_compiles_attachments_into_mime_parts() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    for sub in ["cur", "new", "tmp"] {
        fs::create_dir_all(root.join("maildir/Sent").join(sub)).unwrap();
    }
    let attachment = root.join("my invoice.pdf");
    fs::write(&attachment, b"%PDF-1.4\n").unwrap();
    let out = root.join("sent.eml");
    let config = root.join("config.toml");
    fs::write(
        &config,
        format!(
            "[accounts.test]\n\
             default = true\n\
             email = \"test@example.com\"\n\
             backend.type = \"maildir\"\n\
             backend.root-dir = \"{}\"\n\
             message.send.backend.type = \"sendmail\"\n\
             message.send.backend.cmd = \"tee '{}'\"\n",
            root.join("maildir").display(),
            out.display()
        ),
    )
    .unwrap();
    std::env::set_var("HIMALAYA_CONFIG", &config);

    let template = format!(
        "From: test@example.com\nTo: someone@example.com\nSubject: invoice\n\n\
         See attached.\n<#part filename=\"{}\"><#/part>\n",
        attachment.display()
    );
    mail::compose::send(&template).unwrap();

    let sent = fs::read_to_string(&out).unwrap();
    assert!(sent.contains("Content-Type: multipart/mixed"));
    assert!(sent.contains("Content-Disposition: attachment; filename=\"my invoice.pdf\""));
    assert!(sent.contains("See attached."));
}
