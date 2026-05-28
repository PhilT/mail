const HEADERS_MIME: &str = "MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n";

#[test]
fn injects_three_mime_headers_before_body_boundary() {
    let input = b"From: a@b\r\nSubject: hi\r\n\r\nBody.\r\n";
    let out = mail::ensure_mime_headers(input);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("MIME-Version: 1.0"));
    assert!(out.contains("Content-Type: text/plain; charset=utf-8"));
    assert!(out.contains("Content-Transfer-Encoding: 8bit"));
    let hb = out.find("\r\n\r\n").expect("header/body boundary");
    assert!(out[..hb].contains("MIME-Version"));
    assert!(out[hb..].contains("Body."));
}

#[test]
fn handles_unix_line_endings() {
    let input = b"From: a@b\nSubject: hi\n\nBody.\n";
    let out = mail::ensure_mime_headers(input);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("MIME-Version: 1.0"));
    let hb = out.find("\n\n").expect("header/body boundary");
    assert!(out[..hb].contains("MIME-Version"));
    assert!(out[hb..].contains("Body."));
}

#[test]
fn leaves_template_alone_when_content_type_already_set() {
    let input = format!(
        "From: a@b\r\nContent-Type: text/html\r\n\r\n<p>hi</p>\r\n"
    );
    let out = mail::ensure_mime_headers(input.as_bytes());
    let out = String::from_utf8(out).unwrap();
    assert_eq!(out, input);
}

#[test]
fn passes_through_when_no_body_boundary() {
    let input = b"only headers no body";
    let out = mail::ensure_mime_headers(input);
    assert_eq!(out, input);
}

#[test]
fn does_not_double_inject_when_mime_version_set_but_content_type_missing() {
    let input = b"From: a@b\r\nMIME-Version: 1.0\r\n\r\nBody.\r\n";
    let out = mail::ensure_mime_headers(input);
    let out = String::from_utf8(out).unwrap();
    assert_eq!(out.matches("MIME-Version").count(), 1);
    assert!(out.contains("Content-Type: text/plain; charset=utf-8"));
}
