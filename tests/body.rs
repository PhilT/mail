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

/// Most senders ship a well-formed text/plain alongside an HTML version;
/// the plain version is generally cleaner output.
const MULTIPART_CLEAN: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello,\r\n\r\nThis is a normal plaintext message with no CSS rules.\r\nCheers.\r\n\
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

/// A multipart/related message with one HTML body that references an
/// inline image via `cid:`. Body rendering should leave a marker where the
/// image was so the TUI can paint the real image at that position later.
const HTML_WITH_CID_IMG: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/related; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>Before</p><p><img src=\"cid:logo@x.com\"></p><p>After</p></body></html>\r\n\
--BOUND\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@x.com>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
QQ==\r\n\
--BOUND--\r\n";

const HTML_WITHOUT_IMAGES: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html\r\n\
\r\n\
<html><body><p>Just text.</p></body></html>\r\n";

const HTML_WITH_REMOTE_IMG: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html\r\n\
\r\n\
<html><body><p>Hi</p><img src=\"https://example.com/x.png\"></body></html>\r\n";

#[test]
fn cid_image_replaced_with_marker() {
    let msg = MessageParser::default().parse(HTML_WITH_CID_IMG).unwrap();
    let body = mail::render_body(&msg);
    assert!(body.contains("[[IMG:logo@x.com]]"), "got: {body:?}");
    assert!(body.contains("Before"), "got: {body:?}");
    assert!(body.contains("After"), "got: {body:?}");
}

#[test]
fn no_marker_when_no_inline_images() {
    let msg = MessageParser::default().parse(HTML_WITHOUT_IMAGES).unwrap();
    let body = mail::render_body(&msg);
    assert!(!body.contains("[[IMG:"), "spurious marker: {body:?}");
}

#[test]
fn remote_image_url_emits_url_marker() {
    let msg = MessageParser::default().parse(HTML_WITH_REMOTE_IMG).unwrap();
    let body = mail::render_body(&msg);
    assert!(
        body.contains("[[URL:https://example.com/x.png]]"),
        "missing URL marker: {body:?}",
    );
    // The cid: marker is for embedded images only — no URL should ever
    // land in an IMG: marker.
    assert!(!body.contains("[[IMG:"), "wrong marker for remote image: {body:?}");
}

/// Marketing mail typically uses tables for layout. html2text renders
/// multi-cell rows side-by-side, which breaks substring markers spanning
/// multiple lines (each cell's `[[URL:...]]` ends up interleaved with
/// other cells' content). Make sure both URLs come through as distinct
/// runs without bleed.
const TABLE_WITH_TWO_IMG_URLS: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html\r\n\
\r\n\
<html><body><table><tr>\
<td><img src=\"https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png\"></td>\
<td><img src=\"https://example.com/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.png\"></td>\
</tr></table></body></html>\r\n";

#[test]
#[ignore = "html2text interleaves table cells, breaking substring markers; fix pending in Phase G"]
fn table_layout_with_multiple_remote_imgs_yields_distinct_url_runs() {
    let msg = MessageParser::default()
        .parse(TABLE_WITH_TWO_IMG_URLS)
        .unwrap();
    let body = mail::render_body(&msg);
    let runs = mail::split_body(&body);
    let urls: Vec<&str> = runs
        .iter()
        .filter_map(|r| match r {
            mail::BodyRun::Url(u) => Some(u.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        urls,
        vec![
            "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png",
            "https://example.com/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.png",
        ],
        "wanted two clean URL runs; body: {body:?}",
    );
    // And no literal marker text should leak into Text runs.
    for run in &runs {
        if let mail::BodyRun::Text(t) = run {
            assert!(!t.contains("[[URL:"), "marker leaked into text: {t:?}");
            assert!(!t.contains("[[IMG:"), "marker leaked into text: {t:?}");
        }
    }
}

#[test]
fn multipart_prefers_real_plaintext_over_html() {
    let msg = MessageParser::default().parse(MULTIPART_CLEAN).unwrap();
    let out = mail::render_body(&msg);
    assert!(out.contains("normal plaintext message"), "got: {out:?}");
    assert!(!out.contains("HTML version"), "html leaked: {out:?}");
}
