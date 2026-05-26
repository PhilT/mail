//! Public API tests for the archive / delete file-move helpers.

use std::fs;

use tempfile::TempDir;

fn setup_mailroot() -> TempDir {
    let tmp = TempDir::new().unwrap();
    for sub in [
        "INBOX/cur",
        "INBOX/new",
        "Archive/cur",
        "Archive/new",
        "Trash/cur",
        "Trash/new",
    ] {
        fs::create_dir_all(tmp.path().join(sub)).unwrap();
    }
    tmp
}

#[test]
fn archive_moves_to_archive_cur() {
    let tmp = setup_mailroot();
    let src = tmp.path().join("INBOX/cur").join("1234.foo.host:2,S");
    fs::write(&src, b"From: a\r\n\r\nhi\r\n").unwrap();

    let dst = mail::archive(&src, tmp.path()).unwrap();

    assert!(!src.exists(), "src should have been moved away");
    assert!(dst.exists());
    assert_eq!(dst.parent().unwrap(), tmp.path().join("Archive/cur"));
}

#[test]
fn archive_strips_uid_hint_from_filename() {
    let tmp = setup_mailroot();
    let src = tmp
        .path()
        .join("INBOX/cur")
        .join("1234.foo.host,U=42:2,RS");
    fs::write(&src, b"hi").unwrap();

    let dst = mail::archive(&src, tmp.path()).unwrap();

    assert_eq!(
        dst.file_name().unwrap().to_str().unwrap(),
        "1234.foo.host:2,RS",
    );
}

#[test]
fn archive_preserves_flags() {
    let tmp = setup_mailroot();
    let src = tmp
        .path()
        .join("INBOX/new")
        .join("1234.foo.host,U=42:2,RSF");
    fs::write(&src, b"hi").unwrap();

    let dst = mail::archive(&src, tmp.path()).unwrap();

    // Flags `:2,RSF` must survive the move; only the UID hint should be gone.
    assert!(dst.file_name().unwrap().to_str().unwrap().ends_with(":2,RSF"));
}

#[test]
fn delete_moves_to_trash_cur() {
    let tmp = setup_mailroot();
    let src = tmp.path().join("INBOX/cur").join("9999.bar.host:2,S");
    fs::write(&src, b"hi").unwrap();

    let dst = mail::delete(&src, tmp.path()).unwrap();

    assert!(!src.exists());
    assert!(dst.exists());
    assert_eq!(dst.parent().unwrap(), tmp.path().join("Trash/cur"));
}

#[test]
fn delete_strips_uid_hint() {
    let tmp = setup_mailroot();
    let src = tmp
        .path()
        .join("INBOX/cur")
        .join("9999.bar.host,U=7:2,S");
    fs::write(&src, b"hi").unwrap();

    let dst = mail::delete(&src, tmp.path()).unwrap();

    assert_eq!(
        dst.file_name().unwrap().to_str().unwrap(),
        "9999.bar.host:2,S",
    );
}
