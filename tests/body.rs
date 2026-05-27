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

fn styled_spans<'a>(runs: &'a [BodyRun]) -> Vec<(&'a str, ratatui::style::Style)> {
    let mut out = Vec::new();
    for run in runs {
        if let BodyRun::Text(lines) = run {
            for line in lines {
                for span in &line.spans {
                    out.push((span.content.as_ref(), span.style));
                }
            }
        }
    }
    out
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
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Verify your email</title>\
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

const HTML_WITH_LINK: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>See <a href=\"https://example.com/page\">our page</a> for details.</p></body></html>\r\n";

const HTML_WITH_EMPTY_ANCHOR_LINK: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>Click <a href=\"https://example.com/x\"></a> here.</p></body></html>\r\n";

#[test]
fn link_anchor_text_is_blue_and_underlined() {
    use ratatui::style::{Color, Modifier};
    let msg = MessageParser::default().parse(HTML_WITH_LINK).unwrap();
    let runs = mail::render_body(&msg);
    let anchor = styled_spans(&runs)
        .into_iter()
        .find(|(s, _)| s.contains("our page"))
        .expect("anchor span missing");
    assert_eq!(anchor.1.fg, Some(Color::Blue));
    assert!(anchor.1.add_modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn link_does_not_append_url_footnote() {
    let msg = MessageParser::default().parse(HTML_WITH_LINK).unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(text.contains("our page"), "anchor missing: {text:?}");
    assert!(!text.contains("https://example.com"), "url leaked: {text:?}");
    assert!(!text.contains("(http"), "footnote leaked: {text:?}");
}

#[test]
fn empty_anchor_link_falls_back_to_host_label() {
    let msg = MessageParser::default()
        .parse(HTML_WITH_EMPTY_ANCHOR_LINK)
        .unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(text.contains("[example.com]"), "host fallback missing: {text:?}");
}

const PLAIN_WITH_BLANK_RUNS: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
First.\r\n\
\r\n\
\r\n\
\r\n\
Second.\r\n\
\r\n\
\r\n\
Third.\r\n";

#[test]
fn consecutive_blank_lines_collapse_to_one() {
    let msg = MessageParser::default().parse(PLAIN_WITH_BLANK_RUNS).unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(!text.contains("\n\n\n"), "triple blank survived: {text:?}");
    assert!(text.contains("First."), "text dropped: {text:?}");
    assert!(text.contains("Second."), "text dropped: {text:?}");
    assert!(text.contains("Third."), "text dropped: {text:?}");
}

const PLAIN_WITH_PAREN_URLS: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Rightmove Logo ( https://clicks.rightmove.co.uk/foo?lid=abc )   Start new\r\n\
search ( https://clicks.rightmove.co.uk/bar?lid=def )\r\n\
\r\n\
View all results > (\r\n\
https://clicks.rightmove.co.uk/baz\r\n\
)\r\n\
\r\n\
Plain text (not a url) stays put.\r\n";

#[test]
fn parenthesised_urls_in_plaintext_are_stripped() {
    let msg = MessageParser::default().parse(PLAIN_WITH_PAREN_URLS).unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(!text.contains("https://"), "url leaked: {text:?}");
    assert!(text.contains("Rightmove Logo"), "link text dropped: {text:?}");
    assert!(text.contains("Start new"), "link text dropped: {text:?}");
    assert!(text.contains("View all results"), "link text dropped: {text:?}");
    assert!(
        text.contains("(not a url) stays put."),
        "non-url paren removed: {text:?}",
    );
}

const LAYOUT_TABLE_HTML: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><table><tr><td>\
<p>First paragraph.</p>\
<p>Second paragraph.</p>\
<p>Third paragraph.</p>\
</td></tr></table></body></html>\r\n";

#[test]
fn layout_table_paragraphs_dont_collapse_into_one_line() {
    let msg = MessageParser::default().parse(LAYOUT_TABLE_HTML).unwrap();
    let runs = mail::render_body(&msg);
    let text = flatten_text(&runs);
    assert!(text.contains("First paragraph."), "got: {text:?}");
    assert!(text.contains("Second paragraph."), "got: {text:?}");
    assert!(text.contains("Third paragraph."), "got: {text:?}");
    assert!(
        !text.contains("First paragraph.Second"),
        "paragraphs merged: {text:?}"
    );
}

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
    assert!(!text.contains("viewport"), "<meta> bled through: {text:?}");
    assert!(!text.contains("Verify your email"), "<title> bled through: {text:?}");
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

/// Mailchimp-style fixture: the plain text has standalone destination URLs
/// where images visually live (because Mailchimp pre-resolves the
/// tracker URL when generating text), but the HTML has different tracker
/// URLs for those same `<a href>`s — so href-anchoring can't match. The
/// positional fallback (Nth unanchored image at Nth unused standalone-URL
/// line) is what gives us correct placement.
const MULTIPART_POSITIONAL: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Banner one:\r\n\
https://destination.example.com/one\r\n\
\r\n\
Banner two:\r\n\
https://destination.example.com/two\r\n\
\r\n\
End.\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body>\
<p>Banner one:</p>\
<a href=\"https://tracker.example.com/abc\"><img src=\"https://cdn.example.com/a.png\" alt=\"\"></a>\
<p>Banner two:</p>\
<a href=\"https://tracker.example.com/def\"><img src=\"https://cdn.example.com/b.png\" alt=\"\"></a>\
<p>End.</p></body></html>\r\n\
--BOUND--\r\n";

#[test]
fn positional_fallback_anchors_unmatched_images_to_standalone_url_lines() {
    let msg = MessageParser::default().parse(MULTIPART_POSITIONAL).unwrap();
    let runs = mail::render_body(&msg);

    // Both images should be spliced inline (not appended). In source
    // order: a.png at the first URL line, b.png at the second.
    assert_eq!(
        url_refs(&runs),
        vec![
            "https://cdn.example.com/a.png",
            "https://cdn.example.com/b.png",
        ],
        "runs: {runs:?}",
    );

    // Verify they appear in the correct interleaved positions — text
    // before a.png, then a.png, then b.png's preceding text, then b.png.
    let kinds: Vec<&str> = runs
        .iter()
        .map(|r| match r {
            BodyRun::Text(_) => "T",
            BodyRun::Image(_) => "I",
            BodyRun::Url(_) => "U",
        })
        .collect();
    // No '── images ──' separator should appear since both got placed.
    let text = flatten_text(&runs);
    assert!(!text.contains("images"), "appended-separator leaked: {text:?}");
    // Sequence: text, url, text, url, optional trailing text
    assert!(kinds.starts_with(&["T", "U"]), "kinds: {kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "U").count(), 2);
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
