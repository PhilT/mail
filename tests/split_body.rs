//! Public API tests for `mail::split_body` — turns a rendered body string
//! (with `[[IMG:cid]]` markers from render_body) into a sequence of runs
//! that the TUI can lay out as alternating text/image widgets.

use mail::BodyRun;

#[test]
fn empty_body_returns_no_runs() {
    assert!(mail::split_body("").is_empty());
}

#[test]
fn no_markers_returns_single_text_run() {
    let runs = mail::split_body("just plain text\nover two lines\n");
    assert_eq!(
        runs,
        vec![BodyRun::Text("just plain text\nover two lines\n".into())]
    );
}

#[test]
fn marker_in_middle_splits_into_three_runs() {
    let runs = mail::split_body("before\n[[IMG:logo@x]]\nafter\n");
    assert_eq!(
        runs,
        vec![
            BodyRun::Text("before\n".into()),
            BodyRun::Image("logo@x".into()),
            BodyRun::Text("\nafter\n".into()),
        ]
    );
}

#[test]
fn marker_at_start_no_leading_text_run() {
    let runs = mail::split_body("[[IMG:a]]rest\n");
    assert_eq!(
        runs,
        vec![BodyRun::Image("a".into()), BodyRun::Text("rest\n".into())],
    );
}

#[test]
fn marker_at_end_no_trailing_text_run() {
    let runs = mail::split_body("body\n[[IMG:a]]");
    assert_eq!(
        runs,
        vec![BodyRun::Text("body\n".into()), BodyRun::Image("a".into())],
    );
}

#[test]
fn two_markers_produce_five_alternating_runs() {
    let runs = mail::split_body("A[[IMG:x]]B[[IMG:y]]C");
    assert_eq!(
        runs,
        vec![
            BodyRun::Text("A".into()),
            BodyRun::Image("x".into()),
            BodyRun::Text("B".into()),
            BodyRun::Image("y".into()),
            BodyRun::Text("C".into()),
        ]
    );
}

#[test]
fn adjacent_markers_have_no_text_between() {
    let runs = mail::split_body("[[IMG:a]][[IMG:b]]");
    assert_eq!(
        runs,
        vec![BodyRun::Image("a".into()), BodyRun::Image("b".into())],
    );
}

#[test]
fn url_marker_becomes_url_run() {
    let runs = mail::split_body("see [[URL:https://example.com/x.png]] here");
    assert_eq!(
        runs,
        vec![
            BodyRun::Text("see ".into()),
            BodyRun::Url("https://example.com/x.png".into()),
            BodyRun::Text(" here".into()),
        ]
    );
}

#[test]
fn mixed_cid_and_url_markers_in_one_body() {
    let runs = mail::split_body("a[[IMG:c1]]b[[URL:https://x.com/a.png]]c");
    assert_eq!(
        runs,
        vec![
            BodyRun::Text("a".into()),
            BodyRun::Image("c1".into()),
            BodyRun::Text("b".into()),
            BodyRun::Url("https://x.com/a.png".into()),
            BodyRun::Text("c".into()),
        ]
    );
}
