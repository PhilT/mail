//! Public API tests for INBOX listing.

use std::fs;

use tempfile::TempDir;

fn write_msg(dir: &std::path::Path, name: &str, date: &str, subject: &str) {
    let body = format!(
        "From: a@b.com\r\nDate: {date}\r\nSubject: {subject}\r\n\r\nhi\r\n"
    );
    fs::write(dir.join(name), body).unwrap();
}

#[test]
fn empty_inbox_returns_empty_list() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("INBOX/cur")).unwrap();
    fs::create_dir_all(tmp.path().join("INBOX/new")).unwrap();
    let envs = mail::list_inbox(tmp.path()).unwrap();
    assert!(envs.is_empty());
}

#[test]
fn sorts_by_date_ascending() {
    let tmp = TempDir::new().unwrap();
    let cur = tmp.path().join("INBOX/cur");
    fs::create_dir_all(&cur).unwrap();

    // Deliberately write them out of order on disk so a naive impl would
    // return filesystem order rather than date order.
    write_msg(&cur, "z", "Wed, 3 Jan 2024 10:00:00 +0000", "third");
    write_msg(&cur, "a", "Mon, 1 Jan 2024 10:00:00 +0000", "first");
    write_msg(&cur, "m", "Tue, 2 Jan 2024 10:00:00 +0000", "second");

    let envs = mail::list_inbox(tmp.path()).unwrap();
    let subjects: Vec<_> = envs.iter().map(|e| e.subject.as_str()).collect();
    assert_eq!(subjects, ["first", "second", "third"]);
}

#[test]
fn includes_new_folder() {
    let tmp = TempDir::new().unwrap();
    let cur = tmp.path().join("INBOX/cur");
    let new = tmp.path().join("INBOX/new");
    fs::create_dir_all(&cur).unwrap();
    fs::create_dir_all(&new).unwrap();

    write_msg(&cur, "a", "Mon, 1 Jan 2024 10:00:00 +0000", "from cur");
    write_msg(&new, "b", "Tue, 2 Jan 2024 10:00:00 +0000", "from new");

    let envs = mail::list_inbox(tmp.path()).unwrap();
    let subjects: Vec<_> = envs.iter().map(|e| e.subject.as_str()).collect();
    assert_eq!(subjects, ["from cur", "from new"]);
}

#[test]
fn envelope_carries_file_path() {
    let tmp = TempDir::new().unwrap();
    let cur = tmp.path().join("INBOX/cur");
    fs::create_dir_all(&cur).unwrap();
    write_msg(&cur, "msgfile", "Mon, 1 Jan 2024 10:00:00 +0000", "hi");

    let envs = mail::list_inbox(tmp.path()).unwrap();
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0].path, cur.join("msgfile"));
}
