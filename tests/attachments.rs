use mail_parser::MessageParser;
use std::fs;
use tempfile::tempdir;

const WITH_PDF: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Subject: invoice\r\n\
Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
See attached.\r\n\
--BOUND\r\n\
Content-Type: application/pdf; name=\"invoice.pdf\"\r\n\
Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--BOUND--\r\n";

const PLAIN_NO_ATTACH: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello.\r\n";

#[test]
fn lists_attachments_with_filename_and_type() {
    let msg = MessageParser::default().parse(WITH_PDF).unwrap();
    let atts = mail::attachments(&msg);
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0].filename.as_deref(), Some("invoice.pdf"));
    assert_eq!(atts[0].content_type, "application/pdf");
    assert_eq!(&atts[0].bytes[..5], b"%PDF-");
}

#[test]
fn no_attachments_returns_empty() {
    let msg = MessageParser::default().parse(PLAIN_NO_ATTACH).unwrap();
    assert!(mail::attachments(&msg).is_empty());
}

#[test]
fn save_attachments_writes_files_and_returns_paths() {
    let dir = tempdir().unwrap();
    let msg = MessageParser::default().parse(WITH_PDF).unwrap();
    let paths = mail::save_attachments(&msg, dir.path()).unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].file_name().unwrap(), "invoice.pdf");
    let written = fs::read(&paths[0]).unwrap();
    assert_eq!(&written[..5], b"%PDF-");
}

#[test]
fn save_attachments_uniquifies_on_collision() {
    let dir = tempdir().unwrap();
    let msg = MessageParser::default().parse(WITH_PDF).unwrap();
    let first = mail::save_attachments(&msg, dir.path()).unwrap();
    let second = mail::save_attachments(&msg, dir.path()).unwrap();
    assert_ne!(first[0], second[0]);
    assert!(second[0].exists());
}
