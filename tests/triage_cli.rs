//! Public-surface tests for the `mail-triage` binary: run it and assert on
//! what it prints to stdout. No reach-in to private helpers — the contract
//! we test is the CLI output.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(args: &[&str]) -> (String, String, i32) {
    let bin = env!("CARGO_BIN_EXE_mail-triage");
    let out = Command::new(bin)
        .args(args)
        .output()
        .expect("could not run mail-triage");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn prints_decoded_headers_and_body() {
    let (stdout, _stderr, code) = run(&[fixture("simple.eml").to_str().unwrap()]);
    assert_eq!(code, 0, "exit non-zero: stdout={stdout:?}");

    assert!(stdout.contains("From:    Test Sender <sender@example.com>"), "stdout: {stdout}");
    assert!(
        stdout.contains("To:      Phil <phil@example.com>, list@example.com"),
        "stdout: {stdout}",
    );
    assert!(stdout.contains("Date:    2025-01-01T12:00:00Z"), "stdout: {stdout}");
    // RFC 2047 encoded subjects must be decoded for display.
    assert!(stdout.contains("Subject: Greetings ☕"), "stdout: {stdout}");
    assert!(stdout.contains("Hello from a fixture message."), "stdout: {stdout}");
}

#[test]
fn missing_arg_fails() {
    let (_stdout, stderr, code) = run(&[]);
    assert_ne!(code, 0);
    assert!(stderr.contains("usage:"), "stderr: {stderr}");
}

#[test]
fn missing_file_fails() {
    let (_stdout, stderr, code) = run(&["/no/such/file/anywhere"]);
    assert_ne!(code, 0);
    assert!(!stderr.is_empty(), "expected some error on stderr");
}
