//! Public API tests for `mail::render_body`.
//!
//! Each test calls `render_body(&msg) -> Vec<BodyRun>` and asserts on
//! either the flattened text content (`flatten_text`) or the sequence of
//! image refs that should appear.

use mail::BodyRun;
use mail_parser::MessageParser;

// ---- helpers ----------------------------------------------------------

/// Concatenate every text run's content into a single string. Span styles
/// are ignored; this is for "did the right text come through" checks.
fn flatten_text(runs: &[BodyRun]) -> String {
    let mut out = String::new();
    for run in runs {
        if let BodyRun::Text(lines) = run {
            for line in lines {
                for span in &line.spans {
                    out.push_str(&span.content);
                }
                out.push('\n');
            }
        }
    }
    out
}

fn image_refs(runs: &[BodyRun]) -> Vec<&str> {
    runs.iter()
        .filter_map(|r| match r {
            BodyRun::Image(c) => Some(c.as_str()),
            _ => None,
        })
        .collect()
}

fn url_refs(runs: &[BodyRun]) -> Vec<&str> {
    runs.iter()
        .filter_map(|r| match r {
            BodyRun::Url(u) => Some(u.as_str()),
            _ => None,
        })
        .collect()
}

// ---- fixtures ---------------------------------------------------------

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

const HTML_WITH_CID_IMG: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/related; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>Before</p><p><img src=\"cid:logo@x.com\" alt=\"logo\"></p><p>After</p></body></html>\r\n\
--BOUND\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@x.com>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
QQ==\r\n\
--BOUND--\r\n";

const HTML_WITH_REMOTE_IMG: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html\r\n\
\r\n\
<html><body><p>Hi</p><img src=\"https://example.com/x.png\" alt=\"x\"></body></html>\r\n";

const MULTIPART_CLEAN: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello,\r\n\r\nThis is a normal plaintext message with no images.\r\nCheers.\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>HTML version, should not be used.</p></body></html>\r\n\
--BOUND--\r\n";

const MULTIPART_HTML_HAS_IMG: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
plain text without image\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>html with image</p><img src=\"https://example.com/x.png\" alt=\"x\"></body></html>\r\n\
--BOUND--\r\n";

/// Senders commonly drop the image's alt-text into the text/plain version
/// as `[alt]` exactly where the image lives in the HTML. We can use that
/// to splice the image into the text-rendered body inline.
const MULTIPART_ALT_PLACEHOLDER: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Before the picture.\r\n\
[Company Logo]\r\n\
After the picture.\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>Before the picture.</p><img src=\"https://example.com/logo.png\" alt=\"Company Logo\"><p>After the picture.</p></body></html>\r\n\
--BOUND--\r\n";

// ---- tests ------------------------------------------------------------

#[test]
fn plaintext_only_is_surfaced_verbatim() {
    let msg = MessageParser::default().parse(PLAIN_ONLY).unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(text.contains("Hello plain world."), "got: {text:?}");
}

#[test]
fn html_only_strips_style_and_script_content() {
    let msg = MessageParser::default().parse(HTML_ONLY).unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(text.contains("Hello html body."), "got: {text:?}");
    assert!(!text.contains("color: red"), "<style> bled through: {text:?}");
    assert!(!text.contains("@media"), "<style> bled through: {text:?}");
    assert!(!text.contains("alert"), "<script> bled through: {text:?}");
}

#[test]
fn cid_image_becomes_image_run() {
    let msg = MessageParser::default().parse(HTML_WITH_CID_IMG).unwrap();
    let runs = mail::render_body(&msg);
    assert_eq!(image_refs(&runs), vec!["logo@x.com"], "runs: {runs:?}");
    let text = flatten_text(&runs);
    assert!(text.contains("Before"), "got: {text:?}");
    assert!(text.contains("After"), "got: {text:?}");
}

#[test]
fn remote_image_becomes_url_run() {
    let msg = MessageParser::default().parse(HTML_WITH_REMOTE_IMG).unwrap();
    let runs = mail::render_body(&msg);
    assert_eq!(
        url_refs(&runs),
        vec!["https://example.com/x.png"],
        "runs: {runs:?}",
    );
    // And there should be no IMG cid: ref leaked.
    assert!(image_refs(&runs).is_empty(), "runs: {runs:?}");
}

#[test]
fn multipart_with_no_html_image_prefers_plaintext() {
    let msg = MessageParser::default().parse(MULTIPART_CLEAN).unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(text.contains("normal plaintext message"), "got: {text:?}");
    assert!(!text.contains("HTML version"), "html leaked: {text:?}");
}

#[test]
fn alt_placeholder_in_plaintext_splices_image_inline() {
    let msg = MessageParser::default()
        .parse(MULTIPART_ALT_PLACEHOLDER)
        .unwrap();
    let runs = mail::render_body(&msg);

    // Expected sequence: Text("Before..."), Url(...), Text("After...").
    let kinds: Vec<&str> = runs
        .iter()
        .map(|r| match r {
            BodyRun::Text(_) => "T",
            BodyRun::Image(_) => "I",
            BodyRun::Url(_) => "U",
        })
        .collect();
    assert_eq!(kinds, vec!["T", "U", "T"], "runs: {runs:?}");

    assert_eq!(url_refs(&runs), vec!["https://example.com/logo.png"]);

    // The [Company Logo] placeholder should NOT appear in any Text run —
    // it's been consumed by the image splice.
    let text = flatten_text(&runs);
    assert!(!text.contains("[Company Logo]"), "alt marker leaked: {text:?}");
    assert!(text.contains("Before the picture."), "got: {text:?}");
    assert!(text.contains("After the picture."), "got: {text:?}");
}

#[test]
fn images_without_matching_alt_placeholder_are_appended() {
    // The text/plain part has no "[x]" anchor for the image, so we fall
    // back to appending it after the body so it's at least not lost.
    let msg = MessageParser::default().parse(MULTIPART_HTML_HAS_IMG).unwrap();
    let runs = mail::render_body(&msg);

    let text = flatten_text(&runs);
    assert!(text.contains("plain text without image"), "got: {text:?}");
    assert!(!text.contains("html with image"), "html body leaked: {text:?}");

    assert_eq!(
        url_refs(&runs),
        vec!["https://example.com/x.png"],
        "runs: {runs:?}",
    );
}
