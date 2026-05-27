//! Body rendering pipeline: HTML → Markdown → pulldown-cmark events →
//! styled `BodyRun`s (text runs carry ratatui `Line`s with bold / italic /
//! heading colour baked in; images become `BodyRun::Image` / `::Url`).
//!
//! Plain-text-only messages bypass markdown entirely and are emitted as a
//! single text run with no styling.

use mail_parser::Message;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{body_html_only, body_text_only, BodyRun};

/// Width handed to the markdown converter's wrap option. The TUI re-wraps
/// visually anyway; this just keeps the markdown intermediate from being
/// one huge line, which makes pulldown-cmark events more predictable.
const RENDER_WIDTH: usize = 120;

/// Build the body run sequence for a parsed message.
///
/// - When a real `text/plain` part exists and there's no HTML alternative
///   (or the HTML carries no images), it wins: emitted as a single
///   un-styled `BodyRun::Text` run.
/// - When the message has a real `text/html` part with at least one
///   image, the HTML drives the output via the markdown pipeline.
/// - HTML-only messages always go through the markdown pipeline.
pub fn render_body(msg: &Message<'_>) -> Vec<BodyRun> {
    // Plain-text-with-both: many senders ship a clean text/plain alongside
    // a layout-table HTML version whose only visible content is the
    // template logo and footer. Use the text for the body, then splice any
    // images the HTML mentions inline at their `[alt-text]` anchor in the
    // text (the convention most senders follow). Unmatched images get
    // appended at the end as a fallback so they're not lost.
    if let Some(text) = body_text_only(msg) {
        let pairs = body_html_only(msg)
            .map(|h| image_pairs_from_html(&h))
            .unwrap_or_default();
        return splice_text_with_images(&text, &pairs);
    }
    // HTML-only: full markdown pipeline so images appear inline.
    if let Some(html) = body_html_only(msg) {
        return render_html(&html);
    }
    Vec::new()
}

fn plain_lines(text: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|l| Line::from(l.to_string()))
        .collect()
}

/// One `<img>` from the HTML alternative: its alt-text (for matching the
/// `[alt]` anchor in the plain text body) and a `BodyRun` ready to splice
/// in at that position.
struct ImagePair {
    alt: String,
    run: BodyRun,
}

fn image_pairs_from_html(html: &str) -> Vec<ImagePair> {
    let options = html_to_markdown_rs::ConversionOptions::builder().build();
    let Ok(result) = html_to_markdown_rs::convert(html, Some(options)) else {
        return Vec::new();
    };
    let md = result.content.unwrap_or_default();

    // Plain CommonMark — ENABLE_TABLES messes with `| | ![alt](url)`-shaped
    // lines that html-to-markdown emits for layout-table HTML.
    let mut pairs = Vec::new();
    let mut pending: Vec<(String, String)> = Vec::new(); // (alt_so_far, url)
    for event in Parser::new(&md) {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                pending.push((String::new(), dest_url.into_string()));
            }
            Event::End(TagEnd::Image) => {
                if let Some((alt, url)) = pending.pop() {
                    let run = if let Some(cid) = url.strip_prefix("cid:") {
                        BodyRun::Image(cid.to_string())
                    } else if url.starts_with("http://") || url.starts_with("https://") {
                        BodyRun::Url(url)
                    } else {
                        continue;
                    };
                    pairs.push(ImagePair { alt, run });
                }
            }
            // Inside an image's alt text — accumulate.
            Event::Text(t) | Event::Code(t) if !pending.is_empty() => {
                pending.last_mut().unwrap().0.push_str(&t);
            }
            _ => {}
        }
    }
    pairs
}

/// Walk the plain-text body left to right, looking for any unused
/// `[alt-text]` anchor that matches one of the HTML images. Each match
/// splits the text into a Text run + the matching image run + the rest
/// of the text. Unmatched images get appended at the tail so they're
/// still discoverable.
fn splice_text_with_images(text: &str, pairs: &[ImagePair]) -> Vec<BodyRun> {
    let mut runs: Vec<BodyRun> = Vec::new();
    let mut used = vec![false; pairs.len()];
    let mut cursor = 0;

    loop {
        // Find the earliest unused `[alt]` occurrence at or after `cursor`.
        let mut best: Option<(usize, usize, usize)> = None; // (pos, len, idx)
        for (i, pair) in pairs.iter().enumerate() {
            if used[i] || pair.alt.is_empty() {
                continue;
            }
            let needle = format!("[{}]", pair.alt);
            if let Some(rel) = text[cursor..].find(&needle) {
                let pos = cursor + rel;
                if best.map_or(true, |b| pos < b.0) {
                    best = Some((pos, needle.len(), i));
                }
            }
        }

        let Some((pos, needle_len, idx)) = best else { break };
        let prefix = &text[cursor..pos];
        if !prefix.is_empty() {
            runs.push(BodyRun::Text(plain_lines(prefix)));
        }
        runs.push(pairs[idx].run.clone());
        used[idx] = true;
        cursor = pos + needle_len;
    }

    let tail = &text[cursor..];
    if !tail.is_empty() {
        runs.push(BodyRun::Text(plain_lines(tail)));
    }

    // Fallback: any images we couldn't anchor go at the bottom so they're
    // not silently dropped.
    let unmatched: Vec<&ImagePair> = pairs
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (!used[i]).then_some(p))
        .collect();
    if !unmatched.is_empty() {
        let sep = Line::from(Span::styled(
            "── images ──",
            Style::default().fg(Color::DarkGray),
        ));
        runs.push(BodyRun::Text(vec![Line::default(), sep, Line::default()]));
        for p in unmatched {
            runs.push(p.run.clone());
        }
    }

    if runs.is_empty() {
        // Empty body — preserve at least one empty Text run so callers
        // don't have to handle Vec::new() specially.
        runs.push(BodyRun::Text(plain_lines("")));
    }
    runs
}

/// HTML → Markdown via `html-to-markdown-rs`, then walk pulldown-cmark
/// events to build `BodyRun`s with styled text.
fn render_html(html: &str) -> Vec<BodyRun> {
    let options = html_to_markdown_rs::ConversionOptions::builder()
        .wrap(true)
        .wrap_width(RENDER_WIDTH)
        .build();
    let md = match html_to_markdown_rs::convert(html, Some(options)) {
        Ok(r) => r.content.unwrap_or_default(),
        Err(_) => return vec![BodyRun::Text(plain_lines(html))],
    };

    // Debug: if MAIL_TRIAGE_DUMP is set, dump the markdown intermediate to
    // /tmp/mail-triage-last.md so we can inspect what html-to-markdown
    // emitted for problem messages.
    if std::env::var_os("MAIL_TRIAGE_DUMP").is_some() {
        let _ = std::fs::write("/tmp/mail-triage-last.md", &md);
        let _ = std::fs::write("/tmp/mail-triage-last.html", html);
    }

    let mut builder = RunBuilder::default();
    // Tables and strikethrough are common in HTML-derived markdown (the
    // converter emits GFM tables for any <table>). Without ENABLE_TABLES,
    // table content surfaces as raw text with `|` separators.
    let parser_options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(&md, parser_options);
    for event in parser {
        builder.handle(event);
    }
    builder.finish()
}

#[derive(Default)]
struct RunBuilder {
    runs: Vec<BodyRun>,
    lines: Vec<Line<'static>>,
    current_line: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    /// URLs of open `<a>` links, in nesting order. Popped on End(Link) and
    /// appended as a dim "(url)" suffix so the destination is visible.
    link_urls: Vec<String>,
    in_image: u32,
    list_depth: u32,
    table_depth: u32,
    cell_count_in_row: u32,
    /// Was the very last *emitted* event a horizontal rule? Used to
    /// collapse runs of consecutive rules (common in template HTML).
    last_emitted_was_rule: bool,
}

impl RunBuilder {
    fn current_style(&self) -> Style {
        *self.style_stack.last().unwrap_or(&Style::default())
    }

    fn push_style(&mut self, additional: impl FnOnce(Style) -> Style) {
        let merged = additional(self.current_style());
        self.style_stack.push(merged);
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    fn end_line(&mut self) {
        let spans = std::mem::take(&mut self.current_line);
        self.lines.push(Line::from(spans));
    }

    fn end_block(&mut self) {
        if !self.current_line.is_empty() {
            self.end_line();
        }
        // Blank separator between blocks.
        self.lines.push(Line::default());
    }

    fn flush_text_run(&mut self) {
        if !self.current_line.is_empty() {
            self.end_line();
        }
        if !self.lines.is_empty() {
            let lines = std::mem::take(&mut self.lines);
            self.runs.push(BodyRun::Text(lines));
        }
    }

    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Text(s) | Event::Code(s) if self.in_image > 0 => {
                // Inside an image tag: swallow the alt text — the image run
                // already replaces it.
                let _ = s;
            }
            Event::Text(s) => {
                let style = self.current_style();
                self.current_line.push(Span::styled(s.into_string(), style));
            }
            Event::Code(s) => {
                let style = self.current_style().add_modifier(Modifier::DIM);
                self.current_line
                    .push(Span::styled(format!("`{s}`"), style));
            }
            Event::SoftBreak => {
                self.current_line.push(Span::raw(" "));
            }
            Event::HardBreak => self.end_line(),
            Event::Rule => {
                if !self.current_line.is_empty() {
                    self.end_line();
                }
                if !self.last_emitted_was_rule {
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(40),
                        Style::default().fg(Color::DarkGray),
                    )));
                    self.last_emitted_was_rule = true;
                    return;
                }
                // Already had a rule — collapse consecutive ones.
                return;
            }

            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => self.end_block(),

            Event::Start(Tag::Heading { level, .. }) => {
                let color = match level {
                    pulldown_cmark::HeadingLevel::H1 => Color::Cyan,
                    pulldown_cmark::HeadingLevel::H2 => Color::LightCyan,
                    _ => Color::Blue,
                };
                self.push_style(|s| s.fg(color).add_modifier(Modifier::BOLD));
            }
            Event::End(TagEnd::Heading(_)) => {
                self.pop_style();
                self.end_block();
            }

            Event::Start(Tag::Strong) => {
                self.push_style(|s| s.add_modifier(Modifier::BOLD));
            }
            Event::End(TagEnd::Strong) => self.pop_style(),

            Event::Start(Tag::Emphasis) => {
                self.push_style(|s| s.add_modifier(Modifier::ITALIC));
            }
            Event::End(TagEnd::Emphasis) => self.pop_style(),

            Event::Start(Tag::Strikethrough) => {
                self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT));
            }
            Event::End(TagEnd::Strikethrough) => self.pop_style(),

            Event::Start(Tag::Link { dest_url, .. }) => {
                // Push a link style; the visible text is the link's contents
                // that follow. URL itself appears as a footnote on End.
                self.push_style(|s| {
                    s.fg(Color::Blue).add_modifier(Modifier::UNDERLINED)
                });
                // Remember the URL for the End handler.
                self.style_stack.push(LINK_SENTINEL); // marker layer
                let _ = dest_url; // appended on End via separate stack
                self.link_urls.push(dest_url.into_string());
            }
            Event::End(TagEnd::Link) => {
                // Pop the link-sentinel layer, then the style.
                if let Some(url) = self.link_urls.pop() {
                    let style = Style::default().fg(Color::DarkGray);
                    self.current_line
                        .push(Span::styled(format!(" ({url})"), style));
                }
                self.pop_style(); // sentinel
                self.pop_style(); // style
            }

            Event::Start(Tag::Image { dest_url, .. }) => {
                self.in_image += 1;
                self.flush_text_run();
                let url = dest_url.into_string();
                if let Some(cid) = url.strip_prefix("cid:") {
                    self.runs.push(BodyRun::Image(cid.to_string()));
                } else if url.starts_with("http://") || url.starts_with("https://") {
                    self.runs.push(BodyRun::Url(url));
                } else {
                    // Unknown scheme — drop silently.
                }
            }
            Event::End(TagEnd::Image) => {
                self.in_image = self.in_image.saturating_sub(1);
            }

            Event::Start(Tag::BlockQuote(_)) => {
                self.push_style(|s| s.fg(Color::DarkGray));
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                self.pop_style();
                self.end_block();
            }

            Event::Start(Tag::List(_)) => {
                self.list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                if self.list_depth == 0 {
                    self.end_block();
                }
            }
            Event::Start(Tag::Item) => {
                let indent = "  ".repeat(self.list_depth.saturating_sub(1) as usize);
                self.current_line
                    .push(Span::raw(format!("{indent}• ")));
            }
            Event::End(TagEnd::Item) => {
                if !self.current_line.is_empty() {
                    self.end_line();
                }
            }

            Event::Start(Tag::CodeBlock(_)) => {
                self.push_style(|s| s.add_modifier(Modifier::DIM));
            }
            Event::End(TagEnd::CodeBlock) => {
                self.pop_style();
                self.end_block();
            }

            Event::Start(Tag::Table(_)) => {
                self.table_depth += 1;
                if self.table_depth == 1 {
                    self.end_block();
                }
            }
            Event::End(TagEnd::Table) => {
                self.table_depth = self.table_depth.saturating_sub(1);
                if self.table_depth == 0 {
                    self.end_block();
                }
            }
            Event::Start(Tag::TableHead) => {
                self.cell_count_in_row = 0;
                self.push_style(|s| s.add_modifier(Modifier::BOLD));
            }
            Event::End(TagEnd::TableHead) => {
                self.pop_style();
                if !self.current_line.is_empty() {
                    self.end_line();
                }
            }
            Event::Start(Tag::TableRow) => {
                self.cell_count_in_row = 0;
            }
            Event::End(TagEnd::TableRow) => {
                if !self.current_line.is_empty() {
                    self.end_line();
                }
            }
            Event::Start(Tag::TableCell) => {
                if self.cell_count_in_row > 0 {
                    self.current_line.push(Span::styled(
                        " │ ",
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                self.cell_count_in_row += 1;
            }
            Event::End(TagEnd::TableCell) => {}

            _ => {}
        }
        // Anything that wasn't a Rule resets the dedup latch.
        // (The Rule branches above return early before this.)
        self.last_emitted_was_rule = false;
    }

    fn finish(mut self) -> Vec<BodyRun> {
        self.flush_text_run();
        self.runs
    }
}

/// No-op style layer pushed alongside the link colour layer on Start(Link),
/// popped on End(Link). Lets the URL footnote (held in `link_urls`) ride
/// alongside the visible style so nested links still pop in the right
/// order.
const LINK_SENTINEL: Style = Style::new();
