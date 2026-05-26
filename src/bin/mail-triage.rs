//! Triage every unactioned message in INBOX one at a time.
//!
//! Render-and-block: each iteration draws the current message (fixed header,
//! scrollable body with scrollbar, fixed footer of actions), reads one key,
//! routes it through `mail::app::App`, then performs the requested file
//! move via `mail::archive` / `mail::delete` and advances.
//!
//! Defaults to `/data/mail/namecheap` as the mailroot; override with
//! `MAILROOT=/path mail-triage`.

use std::env;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use mail::app::{App, AppAction, Input};
use mail::Envelope;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::DefaultTerminal;

const DEFAULT_MAILROOT: &str = "/data/mail/namecheap";

fn main() -> Result<()> {
    let mailroot: PathBuf = env::var_os("MAILROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MAILROOT));

    let envelopes = mail::list_inbox(&mailroot)?;
    if envelopes.is_empty() {
        println!("INBOX empty.");
        return Ok(());
    }

    let mut app = App::new(envelopes.len());
    let terminal = ratatui::init();
    let result = run(terminal, &mut app, &envelopes, &mailroot);
    ratatui::restore();
    result?;

    println!(
        "summary: archived={}  deleted={}  skipped={}",
        app.n_archived, app.n_deleted, app.n_skipped
    );
    if app.n_archived + app.n_deleted > 0 {
        println!("running mail-sync...");
        let _ = Command::new("/run/current-system/sw/bin/mail-sync").status();
    }
    Ok(())
}

fn run(
    mut terminal: DefaultTerminal,
    app: &mut App,
    envelopes: &[Envelope],
    mailroot: &PathBuf,
) -> Result<()> {
    let mut cache: Option<(usize, String)> = None;

    while !app.is_done() {
        let env = &envelopes[app.current];
        let body = load_body(&mut cache, app.current, env);

        terminal.draw(|frame| draw(frame, env, &body, app))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let Some(input) = map_key(key) else { continue };

        let action = app.handle(input);
        match action {
            AppAction::Archive => {
                mail::archive(&env.path, mailroot)?;
                cache = None;
                app.advance(action);
            }
            AppAction::Delete => {
                mail::delete(&env.path, mailroot)?;
                cache = None;
                app.advance(action);
            }
            AppAction::Skip => {
                cache = None;
                app.advance(action);
            }
            AppAction::Quit => break,
            AppAction::None | AppAction::ScrollUp | AppAction::ScrollDown => {}
        }
    }
    Ok(())
}

fn load_body(cache: &mut Option<(usize, String)>, idx: usize, env: &Envelope) -> String {
    if let Some((i, body)) = cache.as_ref() {
        if *i == idx {
            return body.clone();
        }
    }
    let body = mail::parse_file(&env.path)
        .ok()
        .and_then(|m| m.body_text(0).map(|s| s.into_owned()))
        .unwrap_or_else(|| "(body extraction failed)".to_string());
    *cache = Some((idx, body.clone()));
    body
}

fn map_key(key: KeyEvent) -> Option<Input> {
    match key.code {
        KeyCode::Char(c) => Some(Input::Char(c)),
        KeyCode::Up => Some(Input::Up),
        KeyCode::Down => Some(Input::Down),
        _ => None,
    }
}

fn draw(frame: &mut ratatui::Frame, env: &Envelope, body: &str, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // header
            Constraint::Min(0),    // body
            Constraint::Length(1), // footer
        ])
        .split(frame.area());

    // Header
    let header_lines = vec![
        Line::from(format!("[{}/{}]", app.current + 1, app.total)),
        Line::from(format!("From:    {}", env.from)),
        Line::from(format!("Date:    {}", env.date_display)),
        Line::from(format!("Subject: {}", env.subject)),
    ];
    frame.render_widget(
        Paragraph::new(header_lines).block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );

    // Body
    let body_area = chunks[1];
    let body_widget = Paragraph::new(body.to_owned())
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    frame.render_widget(body_widget, body_area);

    // Scrollbar — estimate content length from wrapped line count.
    let usable_width = body_area.width.saturating_sub(1) as usize;
    let approx_lines: usize = body
        .lines()
        .map(|l| {
            if usable_width == 0 {
                1
            } else {
                (l.chars().count().max(1) + usable_width - 1) / usable_width
            }
        })
        .sum();
    let viewport = body_area.height as usize;
    let max_scroll = approx_lines.saturating_sub(viewport);
    let mut scrollbar_state = ScrollbarState::new(max_scroll).position(app.scroll as usize);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        body_area,
        &mut scrollbar_state,
    );

    // Footer
    let footer_text = if app.pending_d {
        Span::raw("  press 'd' again to delete  ·  any other key cancels")
    } else {
        Span::raw(
            "  [x] archive   [dd] delete   [space] skip   [j/k] scroll   [q] quit",
        )
    };
    frame.render_widget(Paragraph::new(Line::from(footer_text)), chunks[2]);
}

