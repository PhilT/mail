//! Triage-app state machine. Owns:
//!
//! - which envelope is current,
//! - the `dd` chord pending flag,
//! - the current body scroll offset,
//! - per-session action counters and the quit flag.
//!
//! It does NOT do file I/O — the caller (main loop) gets back an
//! [`AppAction`] from [`App::handle`], performs the relevant file move via
//! `mail::archive` / `mail::delete`, then calls [`App::advance`] to move
//! to the next message. Splitting it this way keeps the state machine
//! pure and testable without a terminal.

/// Input event the app understands. Keep this decoupled from any specific
/// terminal library so the binary can map crossterm `KeyEvent`s into it
/// and tests can construct them directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Char(char),
    Up,
    Down,
}

/// What the caller should do in response to a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    None,
    Archive,
    Delete,
    Skip,
    Quit,
    ScrollUp,
    ScrollDown,
    ToggleRemoteImages,
}

#[derive(Debug, Clone)]
pub struct App {
    pub total: usize,
    pub current: usize,
    pub pending_d: bool,
    pub scroll: u16,
    pub show_remote: bool,
    pub n_archived: u32,
    pub n_deleted: u32,
    pub n_skipped: u32,
    pub quit: bool,
}

impl App {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            current: 0,
            pending_d: false,
            scroll: 0,
            show_remote: false,
            n_archived: 0,
            n_deleted: 0,
            n_skipped: 0,
            quit: false,
        }
    }

    pub fn is_done(&self) -> bool {
        self.quit || self.current >= self.total
    }

    pub fn handle(&mut self, input: Input) -> AppAction {
        // Any key other than another 'd' cancels a pending dd chord.
        if self.pending_d && input != Input::Char('d') {
            self.pending_d = false;
        }

        match input {
            Input::Char('x') => AppAction::Archive,
            Input::Char('d') if self.pending_d => {
                self.pending_d = false;
                AppAction::Delete
            }
            Input::Char('d') => {
                self.pending_d = true;
                AppAction::None
            }
            Input::Char(' ') => AppAction::Skip,
            Input::Char('r') => {
                self.show_remote = !self.show_remote;
                AppAction::ToggleRemoteImages
            }
            Input::Char('q') => {
                self.quit = true;
                AppAction::Quit
            }
            Input::Char('j') | Input::Down => {
                self.scroll = self.scroll.saturating_add(1);
                AppAction::ScrollDown
            }
            Input::Char('k') | Input::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                AppAction::ScrollUp
            }
            _ => AppAction::None,
        }
    }

    /// Cancel any pending dd chord. The main loop calls this when its
    /// `event::poll` times out — letting half-pressed chords expire
    /// instead of waiting for the user to press a second key forever.
    pub fn cancel_chord(&mut self) {
        self.pending_d = false;
    }

    /// Called by the main loop after Archive/Delete/Skip is performed.
    /// Increments the matching counter, resets per-message UI state, and
    /// moves to the next message (saturating at `total` so accidental
    /// extra calls don't push past the end).
    pub fn advance(&mut self, action: AppAction) {
        match action {
            AppAction::Archive => self.n_archived += 1,
            AppAction::Delete => self.n_deleted += 1,
            AppAction::Skip => self.n_skipped += 1,
            _ => {}
        }
        if self.current < self.total {
            self.current += 1;
        }
        self.scroll = 0;
        self.pending_d = false;
        self.show_remote = false;
    }
}
