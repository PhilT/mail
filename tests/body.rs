//! Public API tests for `mail::render_body`. Verifies that:
//! - real text/plain parts are surfaced verbatim (no extra rendering),
//! - HTML-only messages get cleaned up — <style>/<script> contents stripped,
//! - multipart messages with both text and html prefer the text part.

use mail_parser::MessageParser;

const PLAIN_ONLY: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello plain world.\r\n";

const HTML_ONLY: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><head>\
<style>body { color: red; background: blue; }\n@media (max-width: 600px) { .foo {display:none;} }</style>\
<script>alert('hi');</script>\
</head><body><p>Hello html body.</p></body></html>\r\n";

const MULTIPART_BOTH: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Plain text version is preferred.\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>HTML version, should not be used.</p></body></html>\r\n\
--BOUND--\r\n";

#[test]
fn plaintext_only_is_surfaced_verbatim() {
    let msg = MessageParser::default().parse(PLAIN_ONLY).unwrap();
    let out = mail::render_body(&msg);
    assert!(out.contains("Hello plain world."), "got: {out:?}");
}

#[test]
fn html_only_strips_style_and_script_content() {
    let msg = MessageParser::default().parse(HTML_ONLY).unwrap();
    let out = mail::render_body(&msg);
    assert!(out.contains("Hello html body."), "got: {out:?}");
    assert!(!out.contains("color: red"), "<style> bled through: {out:?}");
    assert!(!out.contains("@media"), "<style> bled through: {out:?}");
    assert!(!out.contains("alert"), "<script> bled through: {out:?}");
}

#[test]
fn multipart_prefers_plaintext_over_html() {
    let msg = MessageParser::default().parse(MULTIPART_BOTH).unwrap();
    let out = mail::render_body(&msg);
    assert!(out.contains("Plain text version is preferred."), "got: {out:?}");
    assert!(!out.contains("HTML version"), "got: {out:?}");
}
