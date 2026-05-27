#[test]
fn host_only_no_extension() {
    assert_eq!(
        mail::url_label("https://example.com/foo/bar"),
        "https://example.com"
    );
}

#[test]
fn strips_www_prefix() {
    assert_eq!(
        mail::url_label("https://www.example.com"),
        "https://example.com"
    );
}

#[test]
fn captures_extension_with_query() {
    assert_eq!(
        mail::url_label("https://cdn.example.com/path/image.png?cb=1"),
        "(png) https://cdn.example.com"
    );
}

#[test]
fn captures_extension_with_fragment() {
    assert_eq!(
        mail::url_label("https://x.test/a/b.jpeg#anchor"),
        "(jpeg) https://x.test"
    );
}

#[test]
fn http_scheme_preserved() {
    assert_eq!(
        mail::url_label("http://insecure.example.com/x.gif"),
        "(gif) http://insecure.example.com"
    );
}

#[test]
fn schemeless_input_defaults_to_https() {
    assert_eq!(
        mail::url_label("example.com/foo.png"),
        "(png) https://example.com"
    );
}

#[test]
fn no_path_just_host() {
    assert_eq!(
        mail::url_label("https://example.com"),
        "https://example.com"
    );
}
