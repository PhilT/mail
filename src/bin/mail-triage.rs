//! Triage every unactioned message in INBOX one at a time.
//!
//! Render-and-block: each iteration draws the current message (fixed header,
//! scrollable body with inline images and a scrollbar, fixed footer of
//! actions), reads one key, routes it through `mail::app::App`, then
//! performs the requested file move via `mail::archive` / `mail::delete`
//! and advances.
//!
//! Defaults to `/data/mail/namecheap` as the mailroot; override with
//! `MAILROOT=/path mail-triage`. Replies go through himalaya's default
//! account unless `MAIL_ACCOUNT` names another.

use std::env;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
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
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};

const DEFAULT_MAILROOT: &str = "/data/mail/namecheap";

enum RenderedRun {
    Text(Vec<ratatui::text::Line<'static>>),
    Image { proto: SlicedProtocol, height: u16 },
}

struct LoadedMessage {
    index: usize,
    show_remote: bool,
    has_remote: bool,
    runs: Vec<RenderedRun>,
    attachment_names: Vec<String>,
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
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let mut sends = Vec::new();
    let result = run(terminal, &mut app, &envelopes, &mailroot, picker, &mut sends);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    // Returning from main ends the process, and with it any reply still sending.
    if sends.iter().any(|s| !s.is_finished()) {
        println!("waiting for replies to finish sending...");
    }
    for send in sends {
        let _ = send.join();
    }
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
    picker: Picker,
    sends: &mut Vec<JoinHandle<()>>,
) -> Result<()> {
    let mut cache: Option<LoadedMessage> = None;
    const CHORD_TIMEOUT: Duration = Duration::from_secs(5);
    const IDLE_POLL: Duration = Duration::from_secs(3600);

    while !app.is_done() {
        let env = &envelopes[app.current];
        let loaded = load_message(&mut cache, app.current, env, &picker, app.show_remote);

        // Compute max_scroll from the loaded runs at the current viewport
        // width so `G` (jump-to-bottom) knows the floor. We don't have a
        // Frame here, but the terminal size is enough.
        let term_size = terminal.size()?;
        app.max_scroll = compute_max_scroll(&loaded.runs, term_size.width, term_size.height);

        terminal.draw(|frame| draw(frame, env, loaded, app))?;

        let timeout = if app.pending_d { CHORD_TIMEOUT } else { IDLE_POLL };
        if !event::poll(timeout)? {
            if app.pending_d {
                app.cancel_chord();
            }
            continue;
        }

        let input = match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match map_key(key) {
                Some(i) => i,
                None => continue,
            },
            Event::Mouse(m) => match map_mouse(m) {
                Some(i) => i,
                None => continue,
            },
            _ => continue,
        };

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
            AppAction::SaveAttachments => {
                let downloads = PathBuf::from("/data/downloads");
                app.flash = Some(match mail::parse_file(&env.path) {
                    Ok(msg) => match mail::save_attachments(&msg, &downloads) {
                        Ok(paths) if paths.is_empty() => "no attachments to save".to_string(),
                        Ok(paths) => format!("saved {} to {}", paths.len(), downloads.display()),
                        Err(e) => format!("save failed: {e}"),
                    },
                    Err(e) => format!("parse failed: {e}"),
                });
            }
            AppAction::Reply => match edit_reply(&mut terminal, &env.path) {
                EditOutcome::Saved(body) => {
                    mail::archive(&env.path, mailroot)?;
                    cache = None;
                    let subj = env.subject.clone();
                    app.advance(AppAction::Archive);
                    notify(&format!(
                        "<span foreground='orange'>Sending email ⏵⏵ {}</span>",
                        pango_escape(&subj)
                    ));
                    sends.push(std::thread::spawn(move || {
                        let result = mail::compose::send(&body).map(|_| mail::compose::sync());
                        let esc = pango_escape(&subj);
                        let msg = match result {
                            Ok(_) => format!(
                                "<span foreground='green'>Sent email ⏵⏵ {esc}</span>"
                            ),
                            Err(e) => format!(
                                "<span foreground='red'>Send failed ⏵⏵ {esc}</span>\n{e}"
                            ),
                        };
                        notify(&msg);
                    }));
                }
                EditOutcome::Cancelled => {
                    app.flash = Some("reply cancelled".to_string());
                }
                EditOutcome::Failed(why) => {
                    app.flash = Some(format!("reply failed: {why}"));
                }
            },
            AppAction::None
            | AppAction::ScrollUp
            | AppAction::ScrollDown
            | AppAction::ScrollToTop
            | AppAction::ScrollToBottom => {}
        }
    }
    Ok(())
}

enum EditOutcome {
    Saved(String),
    Cancelled,
    Failed(String),
}

fn edit_reply(terminal: &mut DefaultTerminal, msg_path: &std::path::Path) -> EditOutcome {
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };

    let outcome = (|| -> Result<EditOutcome, String> {
        let msg = mail::parse_file(msg_path).map_err(|e| format!("parse: {e}"))?;
        let mid = msg
            .message_id()
            .ok_or_else(|| "no Message-ID header".to_string())?
            .trim_matches(|c: char| c == '<' || c == '>')
            .to_string();

        let template = std::process::Command::new("notmuch")
            .args(["reply", "--format=default", &format!("id:{mid}")])
            .output()
            .map_err(|e| format!("notmuch reply: {e}"))?;
        if !template.status.success() {
            return Err(format!(
                "notmuch reply exited {}: {}",
                template.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&template.stderr).trim()
            ));
        }

        let signature = mail::compose::signature().map_err(|e| format!("{e:#}"))?;
        let draft =
            mail::compose::with_signature(&String::from_utf8_lossy(&template.stdout), &signature);

        let tmp = std::env::temp_dir()
            .join(format!("mail-triage-reply-{}.eml", std::process::id()));
        std::fs::write(&tmp, draft).map_err(|e| format!("write tmp: {e}"))?;

        let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        let _ = disable_raw_mode();

        let edited = mail::compose::edit(&tmp);

        let _ = enable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture);
        let _ = terminal.clear();
        let _ = std::fs::remove_file(&tmp);

        match edited.map_err(|e| format!("{e:#}"))? {
            Some(body) => Ok(EditOutcome::Saved(body)),
            None => Ok(EditOutcome::Cancelled),
        }
    })();

    match outcome {
        Ok(o) => o,
        Err(e) => EditOutcome::Failed(e),
    }
}

fn pango_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn notify(body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=mail-triage", "--transient", "Mail", body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn load_message<'a>(
    cache: &'a mut Option<LoadedMessage>,
    idx: usize,
    env: &Envelope,
    picker: &Picker,
    show_remote: bool,
) -> &'a mut LoadedMessage {
    let needs_rebuild = match cache.as_ref() {
        Some(l) => l.index != idx || l.show_remote != show_remote,
        None => true,
    };
    if needs_rebuild {
        let (runs, has_remote, attachment_names) = build_runs(env, picker, show_remote);
        *cache = Some(LoadedMessage {
            index: idx,
            show_remote,
            has_remote,
            runs,
            attachment_names,
        });
    }
    cache.as_mut().unwrap()
}

fn build_runs(
    env: &Envelope,
    picker: &Picker,
    show_remote: bool,
) -> (Vec<RenderedRun>, bool, Vec<String>) {
    let Ok(msg) = mail::parse_file(&env.path) else {
        return (
            vec![RenderedRun::Text(plain_line("(body extraction failed)"))],
            false,
            Vec::new(),
        );
    };
    let raw_runs = mail::render_body(&msg);
    let cids = mail::inline_images(&msg);
    let attachment_names: Vec<String> = mail::attachments(&msg)
        .into_iter()
        .enumerate()
        .map(|(i, a)| a.filename.unwrap_or_else(|| format!("attachment-{i}")))
        .collect();

    let mut out: Vec<RenderedRun> = Vec::with_capacity(raw_runs.len());
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
                        "\u{1F304} {}",
                        mail::url_label(&url)
                    ))));
                    continue;
                }
                match fetch_remote(&url) {
                    Ok(bytes) => match build_image_run(picker, Some(&bytes)) {
                        Some(run) => out.push(run),
                        None => out.push(RenderedRun::Text(plain_line(&format!(
                            "\u{1F304} {} (decode failed)",
                            mail::url_label(&url)
                        )))),
                    },
                    Err(why) => out.push(RenderedRun::Text(plain_line(&format!(
                        "\u{1F304} {} ({why})",
                        mail::url_label(&url)
                    )))),
                }
            }
        }
    }
    (out, has_remote, attachment_names)
}


fn plain_line(s: &str) -> Vec<ratatui::text::Line<'static>> {
    vec![ratatui::text::Line::from(s.to_string())]
}

fn attachment_header_lines(
    names: &[String],
    flash: Option<&str>,
) -> Vec<ratatui::text::Line<'static>> {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;
    let mut spans = vec![Span::styled(
        format!("\u{1F4CE} attachments ({})", names.len()),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let Some(msg) = flash {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            msg.to_string(),
            Style::default().fg(Color::Green),
        ));
    }
    let mut lines: Vec<ratatui::text::Line<'static>> = Vec::with_capacity(names.len() + 2);
    lines.push(ratatui::text::Line::from(spans));
    for n in names {
        lines.push(ratatui::text::Line::from(format!("  • {n}")));
    }
    lines.push(ratatui::text::Line::from(""));
    lines
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

fn build_image_run(picker: &Picker, bytes: Option<&[u8]>) -> Option<RenderedRun> {
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

    let proto = SlicedProtocol::new(picker, dyn_img, Some(target)).ok()?;
    let height = proto.size().height;
    Some(RenderedRun::Image { proto, height })
}

fn map_key(key: KeyEvent) -> Option<Input> {
    match key.code {
        KeyCode::Char(c) => Some(Input::Char(c)),
        KeyCode::Up => Some(Input::Up),
        KeyCode::Down => Some(Input::Down),
        _ => None,
    }
}

fn map_mouse(m: MouseEvent) -> Option<Input> {
    match m.kind {
        MouseEventKind::ScrollUp => Some(Input::Up),
        MouseEventKind::ScrollDown => Some(Input::Down),
        _ => None,
    }
}

fn draw(
    frame: &mut ratatui::Frame,
    env: &Envelope,
    loaded: &LoadedMessage,
    app: &App,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let attach_marker = match loaded.attachment_names.len() {
        0 => String::new(),
        n => format!("  \u{1F4CE}{n}"),
    };
    let header_lines = vec![
        Line::from(format!(
            "[{}/{}]{}",
            app.current + 1,
            app.total,
            attach_marker
        )),
        Line::from(format!("From:    {}", env.from)),
        Line::from(format!("Date:    {}", env.date_display)),
        Line::from(format!("Subject: {}", env.subject)),
    ];
    frame.render_widget(
        Paragraph::new(header_lines).block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );

    let body_area = if !loaded.attachment_names.is_empty() {
        let lines = attachment_header_lines(&loaded.attachment_names, app.flash.as_deref());
        let h = lines.len() as u16;
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(h), Constraint::Min(0)])
            .split(chunks[1]);
        frame.render_widget(Paragraph::new(lines), split[0]);
        split[1]
    } else if let Some(msg) = &app.flash {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(chunks[1]);
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("  {msg}"),
                ratatui::style::Style::default().fg(ratatui::style::Color::Green),
            )),
            split[0],
        );
        split[1]
    } else {
        chunks[1]
    };
    draw_body(frame, body_area, &loaded.runs, app.scroll);

    if app.pending_d {
        let line = Line::from(vec![
            ratatui::text::Span::raw("  "),
            ratatui::text::Span::styled(
                "press 'd' again to delete",
                ratatui::style::Style::default()
                    .fg(ratatui::style::Color::Red)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            ratatui::text::Span::raw("  ·  any other key cancels"),
        ]);
        frame.render_widget(Paragraph::new(line), chunks[2]);
    } else {
        let mut parts = vec![
            "[x] archive".to_string(),
            "[dd] delete".to_string(),
            "[space] skip".to_string(),
        ];
        if loaded.has_remote {
            let state = if app.show_remote { "on" } else { "off" };
            parts.push(format!("[i] images: {state}"));
        }
        if !loaded.attachment_names.is_empty() {
            parts.push("[w] save attachments".to_string());
        }
        parts.push("[r] reply".to_string());
        parts.push("[j/k] scroll".to_string());
        parts.push("[q] quit".to_string());
        let footer = format!("  {}", parts.join("  "));
        frame.render_widget(Paragraph::new(footer), chunks[2]);
    }
}

fn draw_body(
    frame: &mut ratatui::Frame,
    area: Rect,
    runs: &[RenderedRun],
    scroll: u16,
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

        match &runs[i] {
            RenderedRun::Text(lines) => {
                let scroll_offset = if i == start_run { skip_in_run } else { 0 };
                let para = Paragraph::new(lines.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((scroll_offset, 0));
                frame.render_widget(para, rect);
            }
            RenderedRun::Image { proto, .. } => {
                let top_skip = if i == start_run { skip_in_run } else { 0 };
                let pos = SignedPosition::from((0, -(top_skip as i16)));
                frame.render_widget(SlicedImage::new(proto, pos), rect);
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
