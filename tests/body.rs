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

/// Marketing senders (eBay etc.) put raw CSS into their text/plain
/// alternative. The HTML version is cleaner once run through html2text,
/// so we prefer HTML when both exist.
const MULTIPART_BOTH: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
.foo { color: red; } Plain text mixed with raw CSS noise.\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>Clean HTML version.</p></body></html>\r\n\
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

/// Marketing mail (eBay-style) often has <style> tags inside <body>,
/// sometimes wrapped in MSO conditional comments. html2text leaves their
/// contents in the output, so the render leaks CSS rules. We pre-strip.
const HTML_STYLE_IN_BODY: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body>\
<!--[if !mso]><!-- -->\
<style>.preheaderMod, .preheaderMod div { color: #111820; }</style>\
<!--<![endif]-->\
<p>Real text before.</p>\
<style>@media (prefers-color-scheme:dark) { .foo {background: #000;} }</style>\
<p>Real text after.</p>\
<script>alert('xss');</script>\
</body></html>\r\n";

#[test]
fn html_strips_style_and_script_anywhere_in_body() {
    let msg = MessageParser::default().parse(HTML_STYLE_IN_BODY).unwrap();
    let out = mail::render_body(&msg);
    assert!(out.contains("Real text before."), "got: {out:?}");
    assert!(out.contains("Real text after."), "got: {out:?}");
    assert!(!out.contains(".preheaderMod"), "<style> bled through: {out:?}");
    assert!(!out.contains("prefers-color-scheme"), "<style> bled through: {out:?}");
    assert!(!out.contains("alert"), "<script> bled through: {out:?}");
}

#[test]
fn multipart_prefers_html2text_rendered_html_over_noisy_plaintext() {
    let msg = MessageParser::default().parse(MULTIPART_BOTH).unwrap();
    let out = mail::render_body(&msg);
    assert!(out.contains("Clean HTML version."), "got: {out:?}");
    assert!(!out.contains(".foo"), "noisy plain leaked through: {out:?}");
}
