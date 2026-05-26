//! Public API tests for `mail::inline_images`.

use mail_parser::MessageParser;

/// `SGVsbG9JbWFnZQ==` is base64 for `HelloImage` — content doesn't have
/// to be a real PNG, just bytes whose decoding we can assert on.
const HTML_WITH_INLINE_IMAGE: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/related; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>Look: <img src=\"cid:foo@example.com\"></p></body></html>\r\n\
--BOUND\r\n\
Content-Type: image/png\r\n\
Content-ID: <foo@example.com>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
SGVsbG9JbWFnZQ==\r\n\
--BOUND--\r\n";

const TWO_INLINE_IMAGES: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/related; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/html\r\n\
\r\n\
<html><body><img src=\"cid:a\"><img src=\"cid:b\"></body></html>\r\n\
--BOUND\r\n\
Content-Type: image/png\r\n\
Content-ID: <a>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
QQ==\r\n\
--BOUND\r\n\
Content-Type: image/jpeg\r\n\
Content-ID: <b>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
Qg==\r\n\
--BOUND--\r\n";

const PLAIN_NO_IMAGES: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
just text, no images.\r\n";

const NON_IMAGE_ATTACHMENT_IS_SKIPPED: &[u8] = b"From: a@b.com\r\n\
Date: Mon, 1 Jan 2024 10:00:00 +0000\r\n\
Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\
\r\n\
--BOUND\r\n\
Content-Type: text/plain\r\n\
\r\n\
text part\r\n\
--BOUND\r\n\
Content-Type: application/pdf\r\n\
Content-ID: <doc>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
UERG\r\n\
--BOUND--\r\n";

#[test]
fn extracts_single_inline_image() {
    let msg = MessageParser::default().parse(HTML_WITH_INLINE_IMAGE).unwrap();
    let imgs = mail::inline_images(&msg);
    assert_eq!(imgs.len(), 1);
    let bytes = imgs.get("foo@example.com").expect("missing cid");
    assert_eq!(bytes, b"HelloImage");
}

#[test]
fn extracts_multiple_inline_images() {
    let msg = MessageParser::default().parse(TWO_INLINE_IMAGES).unwrap();
    let imgs = mail::inline_images(&msg);
    assert_eq!(imgs.len(), 2);
    assert_eq!(imgs.get("a").unwrap(), b"A");
    assert_eq!(imgs.get("b").unwrap(), b"B");
}

#[test]
fn plain_message_has_no_inline_images() {
    let msg = MessageParser::default().parse(PLAIN_NO_IMAGES).unwrap();
    let imgs = mail::inline_images(&msg);
    assert!(imgs.is_empty());
}

#[test]
fn skips_non_image_parts_even_with_content_id() {
    let msg = MessageParser::default()
        .parse(NON_IMAGE_ATTACHMENT_IS_SKIPPED)
        .unwrap();
    let imgs = mail::inline_images(&msg);
    assert!(imgs.is_empty(), "non-image cid leaked through: {imgs:?}");
}
