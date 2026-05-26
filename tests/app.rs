//! Public API tests for the triage app's state machine.
//!
//! No terminal: we feed input events and assert on observable state and on
//! the action returned to the caller. Rendering itself is eyeballed in the
//! real TUI; this exercises the parts that decide *what* to render and
//! *what* to do.

use mail::app::{App, AppAction, Input};

#[test]
fn new_inits_at_first_message() {
    let app = App::new(5);
    assert_eq!(app.current, 0);
    assert_eq!(app.total, 5);
    assert!(!app.pending_d);
    assert_eq!(app.scroll, 0);
    assert!(!app.is_done());
}

#[test]
fn empty_inbox_is_already_done() {
    let app = App::new(0);
    assert!(app.is_done());
}

#[test]
fn x_returns_archive() {
    let mut app = App::new(3);
    assert!(matches!(app.handle(Input::Char('x')), AppAction::Archive));
}

#[test]
fn first_d_returns_none_and_pends_chord() {
    let mut app = App::new(3);
    let act = app.handle(Input::Char('d'));
    assert!(matches!(act, AppAction::None));
    assert!(app.pending_d);
}

#[test]
fn dd_chord_returns_delete_and_clears_pending() {
    let mut app = App::new(3);
    app.handle(Input::Char('d'));
    let act = app.handle(Input::Char('d'));
    assert!(matches!(act, AppAction::Delete));
    assert!(!app.pending_d);
}

#[test]
fn dx_cancels_chord_then_archives() {
    let mut app = App::new(3);
    app.handle(Input::Char('d'));
    let act = app.handle(Input::Char('x'));
    assert!(matches!(act, AppAction::Archive));
    assert!(!app.pending_d);
}

#[test]
fn space_returns_skip() {
    let mut app = App::new(3);
    assert!(matches!(app.handle(Input::Char(' ')), AppAction::Skip));
}

#[test]
fn q_returns_quit_and_marks_done() {
    let mut app = App::new(3);
    assert!(matches!(app.handle(Input::Char('q')), AppAction::Quit));
    assert!(app.quit);
    assert!(app.is_done());
}

#[test]
fn j_and_down_both_scroll_down() {
    let mut app = App::new(3);
    app.handle(Input::Char('j'));
    assert_eq!(app.scroll, 1);
    app.handle(Input::Down);
    assert_eq!(app.scroll, 2);
}

#[test]
fn k_and_up_both_scroll_up_saturating() {
    let mut app = App::new(3);
    app.handle(Input::Char('k')); // already at 0, must stay at 0
    assert_eq!(app.scroll, 0);
    app.handle(Input::Char('j'));
    app.handle(Input::Char('j'));
    app.handle(Input::Up);
    assert_eq!(app.scroll, 1);
}

#[test]
fn advance_bumps_current_resets_scroll_and_counts() {
    let mut app = App::new(3);
    app.handle(Input::Char('j'));
    assert_eq!(app.scroll, 1);

    app.advance(AppAction::Archive);
    assert_eq!(app.current, 1);
    assert_eq!(app.scroll, 0);
    assert_eq!(app.n_archived, 1);
}

#[test]
fn advance_with_each_action_kind_counts_correctly() {
    let mut app = App::new(10);
    app.advance(AppAction::Archive);
    app.advance(AppAction::Archive);
    app.advance(AppAction::Delete);
    app.advance(AppAction::Skip);
    assert_eq!(app.n_archived, 2);
    assert_eq!(app.n_deleted, 1);
    assert_eq!(app.n_skipped, 1);
    assert_eq!(app.current, 4);
}

#[test]
fn advance_past_last_marks_done() {
    let mut app = App::new(2);
    app.advance(AppAction::Skip);
    app.advance(AppAction::Skip);
    assert!(app.is_done());
}

#[test]
fn advance_doesnt_overflow_past_total() {
    let mut app = App::new(2);
    app.advance(AppAction::Skip);
    app.advance(AppAction::Skip);
    // an extra advance shouldn't crash or push current past total
    app.advance(AppAction::Skip);
    assert_eq!(app.current, 2);
}

#[test]
fn unrecognised_key_is_noop_and_clears_chord() {
    let mut app = App::new(3);
    app.handle(Input::Char('d'));
    assert!(app.pending_d);
    app.handle(Input::Char('z'));
    assert!(!app.pending_d);
}
