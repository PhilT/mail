//! Triage every unactioned message in INBOX one at a time.
//!
//! Render-and-block: each iteration draws the current message (fixed header,
//! scrollable body with inline images and a scrollbar, fixed footer of
//! actions), reads one key, routes it through `mail::app::App`, then
//! performs the requested file move via `mail::archive` / `mail::delete`
//! and advances.
//!
//! Defaults to `/data/mail/namecheap` as the mailroot; override with
//! `MAILROOT=/path mail-triage`.

use std::env;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use image::ImageReader;
use mail::app::{App, AppAction, Input};
use mail::{BodyRun, Envelope};
use ratatui::layout::{Constraint, Direction, Layout, Rect, Size as LayoutSize};
use ratatui::text::Line;
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::DefaultTerminal;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

const DEFAULT_MAILROOT: &str = "/data/mail/namecheap";

/// One rendered slice of a message body: either pre-styled text lines or
/// an inline image already encoded for the active terminal.
enum RenderedRun {
    Text(Vec<ratatui::text::Line<'static>>),
    Image {
        source: image::DynamicImage,
        full: Protocol,
        height: u16,
        cropped: std::collections::HashMap<u16, Protocol>,
    },
}

struct LoadedMessage {
    index: usize,
    show_remote: bool,
    has_remote: bool,
    runs: Vec<RenderedRun>,
}

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
    // Pick the graphics protocol BEFORE entering raw/alt-screen mode, since
    // from_query_stdio writes/reads control sequences on stdio and that
    // interacts badly with ratatui's terminal setup.
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let terminal = ratatui::init();
    let result = run(terminal, &mut app, &envelopes, &mailroot, picker);
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
    mut picker: Picker,
) -> Result<()> {
    let mut cache: Option<LoadedMessage> = None;
    const CHORD_TIMEOUT: Duration = Duration::from_secs(5);
    const IDLE_POLL: Duration = Duration::from_secs(3600);

    while !app.is_done() {
        let env = &envelopes[app.current];
        let loaded = load_message(&mut cache, app.current, env, &mut picker, app.show_remote);

        // Compute max_scroll from the loaded runs at the current viewport
        // width so `G` (jump-to-bottom) knows the floor. We don't have a
        // Frame here, but the terminal size is enough.
        let term_size = terminal.size()?;
        app.max_scroll = compute_max_scroll(&loaded.runs, term_size.width, term_size.height);

        terminal.draw(|frame| draw(frame, env, loaded, app, &mut picker))?;

        let timeout = if app.pending_d { CHORD_TIMEOUT } else { IDLE_POLL };
        if !event::poll(timeout)? {
            if app.pending_d {
                app.cancel_chord();
            }
            continue;
        }

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
            AppAction::ToggleRemoteImages => {
                // Force the cache rebuild so the next draw fetches (or
                // hides) the remote images per the new show_remote flag.
                cache = None;
            }
            AppAction::None
            | AppAction::ScrollUp
            | AppAction::ScrollDown
            | AppAction::ScrollToTop
            | AppAction::ScrollToBottom => {}
        }
    }
    Ok(())
}

fn load_message<'a>(
    cache: &'a mut Option<LoadedMessage>,
    idx: usize,
    env: &Envelope,
    picker: &mut Picker,
    show_remote: bool,
) -> &'a mut LoadedMessage {
    let needs_rebuild = match cache.as_ref() {
        Some(l) => l.index != idx || l.show_remote != show_remote,
        None => true,
    };
    if needs_rebuild {
        let (runs, has_remote) = build_runs(env, picker, show_remote);
        *cache = Some(LoadedMessage {
            index: idx,
            show_remote,
            has_remote,
            runs,
        });
    }
    cache.as_mut().unwrap()
}

fn build_runs(
    env: &Envelope,
    picker: &mut Picker,
    show_remote: bool,
) -> (Vec<RenderedRun>, bool) {
    let Ok(msg) = mail::parse_file(&env.path) else {
        return (
            vec![RenderedRun::Text(plain_line("(body extraction failed)"))],
            false,
        );
    };
    let raw_runs = mail::render_body(&msg);
    let cids = mail::inline_images(&msg);

    let mut out = Vec::with_capacity(raw_runs.len());
    let mut has_remote = false;
    for r in raw_runs {
        match r {
            BodyRun::Text(lines) => out.push(RenderedRun::Text(lines)),
            BodyRun::Image(cid) => {
                match build_image_run(picker, cids.get(&cid).map(|v| v.as_slice())) {
                    Some(run) => out.push(run),
                    None => out.push(RenderedRun::Text(plain_line(&format!(
                        "[image: {cid} unavailable]"
                    )))),
                }
            }
            BodyRun::Url(url) => {
                has_remote = true;
                if !show_remote {
                    out.push(RenderedRun::Text(plain_line(&format!(
                        "[remote image: {url} — press 'r' to load]"
                    ))));
                    continue;
                }
                match fetch_remote(&url) {
                    Ok(bytes) => match build_image_run(picker, Some(&bytes)) {
                        Some(run) => out.push(run),
                        None => out.push(RenderedRun::Text(plain_line(&format!(
                            "[remote image decode failed: {url}]"
                        )))),
                    },
                    Err(why) => out.push(RenderedRun::Text(plain_line(&format!(
                        "[remote image failed ({why}): {url}]"
                    )))),
                }
            }
        }
    }
    (out, has_remote)
}

fn plain_line(s: &str) -> Vec<ratatui::text::Line<'static>> {
    vec![ratatui::text::Line::from(s.to_string())]
}

/// Browser-flavored UA so tracking-pixel servers don't reject us. Lots of
/// senders quietly 403 anything that looks like a script.
const FETCH_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

fn fetch_remote(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(FETCH_UA)
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let resp = client.get(url).send().map_err(|e| format!("send: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("body: {e}"))
}

fn build_image_run(picker: &mut Picker, bytes: Option<&[u8]>) -> Option<RenderedRun> {
    let bytes = bytes?;
    let dyn_img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;

    let font = picker.font_size();
    let nat_w = dyn_img.width().div_ceil(font.width as u32) as u16;
    let nat_h = dyn_img.height().div_ceil(font.height as u32) as u16;
    let target = LayoutSize::new(nat_w.min(60).max(1), nat_h.max(1));

    let full = picker
        .new_protocol(dyn_img.clone(), target, Resize::Fit(None))
        .ok()?;
    let height = full.size().height;
    Some(RenderedRun::Image {
        source: dyn_img,
        full,
        height,
        cropped: std::collections::HashMap::new(),
    })
}

fn build_cropped_protocol(
    source: &image::DynamicImage,
    skip_rows: u16,
    picker: &mut Picker,
) -> Option<Protocol> {
    let font = picker.font_size();
    let skip_px = (skip_rows as u32).saturating_mul(font.height as u32);
    let h = source.height();
    if skip_px >= h {
        return None;
    }
    let cropped = source.crop_imm(0, skip_px, source.width(), h - skip_px);
    let nat_w = cropped.width().div_ceil(font.width as u32) as u16;
    let nat_h = cropped.height().div_ceil(font.height as u32) as u16;
    let target = LayoutSize::new(nat_w.min(60).max(1), nat_h.max(1));
    picker.new_protocol(cropped, target, Resize::Fit(None)).ok()
}

fn map_key(key: KeyEvent) -> Option<Input> {
    match key.code {
        KeyCode::Char(c) => Some(Input::Char(c)),
        KeyCode::Up => Some(Input::Up),
        KeyCode::Down => Some(Input::Down),
        _ => None,
    }
}

fn draw(
    frame: &mut ratatui::Frame,
    env: &Envelope,
    loaded: &mut LoadedMessage,
    app: &App,
    picker: &mut Picker,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(1),
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
    draw_body(frame, chunks[1], &mut loaded.runs, app.scroll, picker);

    // Footer
    let footer = if app.pending_d {
        "  press 'd' again to delete  ·  any other key cancels".to_string()
    } else {
        let mut parts = vec![
            "[x] archive".to_string(),
            "[dd] delete".to_string(),
            "[space] skip".to_string(),
        ];
        if loaded.has_remote {
            let state = if app.show_remote { "on" } else { "off" };
            parts.push(format!("[r] images: {state}"));
        }
        parts.push("[j/k] scroll".to_string());
        parts.push("[q] quit".to_string());
        format!("  {}", parts.join("  "))
    };
    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

fn draw_body(
    frame: &mut ratatui::Frame,
    area: Rect,
    runs: &mut [RenderedRun],
    scroll: u16,
    picker: &mut Picker,
) {
    if area.height == 0 {
        return;
    }
    // Leave one column on the right for the scrollbar.
    let inner = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };

    // Compute height in cells for each run at the current viewport width.
    let widths_for_wrap = inner.width.max(1) as usize;
    let heights: Vec<u16> = runs
        .iter()
        .map(|r| match r {
            RenderedRun::Text(lines) => wrap_lines_for(lines, widths_for_wrap) as u16,
            RenderedRun::Image { height, .. } => *height,
        })
        .collect();
    let total_height: u16 = heights.iter().copied().fold(0, u16::saturating_add);

    // Find the topmost run and how many of its leading cells to skip.
    let mut acc = 0u16;
    let mut start_run = 0usize;
    let mut skip_in_run = 0u16;
    for (i, h) in heights.iter().enumerate() {
        if acc + h > scroll {
            start_run = i;
            skip_in_run = scroll - acc;
            break;
        }
        acc += h;
        // If we exhaust runs (scrolled past), start_run stays at end.
        start_run = i + 1;
    }

    let mut y = 0u16;
    for i in start_run..runs.len() {
        if y >= inner.height {
            break;
        }
        let run_height = heights[i];
        let visible = if i == start_run {
            run_height.saturating_sub(skip_in_run)
        } else {
            run_height
        };
        let take = visible.min(inner.height - y);
        if take == 0 {
            continue;
        }
        let rect = Rect {
            x: inner.x,
            y: inner.y + y,
            width: inner.width,
            height: take,
        };

        match &mut runs[i] {
            RenderedRun::Text(lines) => {
                let scroll_offset = if i == start_run { skip_in_run } else { 0 };
                let para = Paragraph::new(lines.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((scroll_offset, 0));
                frame.render_widget(para, rect);
            }
            RenderedRun::Image { source, full, cropped, .. } => {
                let top_skip = if i == start_run { skip_in_run } else { 0 };
                let proto = if top_skip > 0 {
                    if !cropped.contains_key(&top_skip) {
                        if let Some(p) = build_cropped_protocol(source, top_skip, picker) {
                            cropped.insert(top_skip, p);
                        }
                    }
                    cropped.get(&top_skip).unwrap_or(full)
                } else {
                    full
                };
                frame.render_widget(Image::new(proto).allow_clipping(true), rect);
            }
        }
        y += take;
    }

    // Scrollbar
    let max_scroll = total_height.saturating_sub(inner.height);
    let mut bar_state = ScrollbarState::new(max_scroll as usize).position(scroll as usize);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        area,
        &mut bar_state,
    );
}

/// Estimate the max scroll offset for the body given the loaded runs and
/// terminal dimensions. Matches the layout draw_body does: top 5 rows are
/// header, bottom row is footer, body fills the middle, and one column on
/// the right is reserved for the scrollbar.
fn compute_max_scroll(runs: &[RenderedRun], term_w: u16, term_h: u16) -> u16 {
    let body_h = term_h.saturating_sub(5 + 1);
    let body_inner_w = term_w.saturating_sub(1).max(1) as usize;
    let total: u16 = runs
        .iter()
        .map(|r| match r {
            RenderedRun::Text(lines) => wrap_lines_for(lines, body_inner_w) as u16,
            RenderedRun::Image { height, .. } => *height,
        })
        .fold(0u16, u16::saturating_add);
    total.saturating_sub(body_h)
}

/// Cheap line-count estimator that matches what `Paragraph::wrap` does
/// closely enough for scroll arithmetic. Sums visible width across each
/// `Line`'s spans, dividing by viewport width to count wrap rows.
fn wrap_lines_for(lines: &[ratatui::text::Line<'static>], width: usize) -> usize {
    if width == 0 {
        return lines.len().max(1);
    }
    lines
        .iter()
        .map(|l| {
            let n: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            if n == 0 {
                1
            } else {
                (n + width - 1) / width
            }
        })
        .sum::<usize>()
        .max(1)
}
