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
    let mut runs = if let Some(text) = body_text_only(msg) {
        let pairs = body_html_only(msg)
            .map(|h| image_pairs_from_html(&h))
            .unwrap_or_default();
        let cleaned = strip_parenthesised_urls(&text);
        splice_text_with_images(&cleaned, &pairs)
    } else if let Some(html) = body_html_only(msg) {
        render_html(&html)
    } else {
        Vec::new()
    };
    for run in &mut runs {
        if let BodyRun::Text(lines) = run {
            collapse_blank_lines(lines);
        }
    }
    let mut prev_ended_blank = true;
    for run in &mut runs {
        match run {
            BodyRun::Text(lines) => {
                if prev_ended_blank {
                    while lines.first().is_some_and(line_is_blank) {
                        lines.remove(0);
                    }
                }
                prev_ended_blank = lines.last().is_none_or(line_is_blank);
            }
            BodyRun::Image(_) | BodyRun::Url(_) => prev_ended_blank = false,
        }
    }
    runs.retain(|r| match r {
        BodyRun::Text(lines) => !lines.is_empty(),
        _ => true,
    });
    runs
}

fn is_visible_char(c: char) -> bool {
    !c.is_whitespace()
        && !matches!(
            c,
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{034F}' | '│'
        )
}

fn line_is_blank(l: &Line<'static>) -> bool {
    l.spans
        .iter()
        .all(|s| !s.content.chars().any(is_visible_char))
}

fn collapse_blank_lines(lines: &mut Vec<Line<'static>>) {
    let mut prev_blank = false;
    lines.retain(|l| {
        let blank = line_is_blank(l);
        let keep = !(blank && prev_blank);
        prev_blank = blank;
        keep
    });
}

fn plain_lines(text: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|l| Line::from(l.to_string()))
        .collect()
}

/// True if a URL's path (before query string) ends with a common image
/// extension. Filters out Outlook-style inbox deep links that get
/// rewritten as image srcs but aren't actually images.
fn looks_like_image_url(url: &str) -> bool {
    let cut = url.find('?').or_else(|| url.find('#')).unwrap_or(url.len());
    let lower = url[..cut].to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".ico"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// One `<img>` from the HTML alternative: candidate anchor substrings to
/// match against the plain text body (in priority order), and a
/// `BodyRun` ready to splice in at the first matching anchor.
///
/// Anchor priorities:
/// 1. Parent `<a href>` URL (when the image is inside a link) — most
///    marketing-banner images live inside clickable links, and the
///    plain text version usually echoes that URL standalone.
/// 2. `[alt]` if the image has alt text.
struct ImagePair {
    anchors: Vec<String>,
    run: BodyRun,
}

fn image_pairs_from_html(html: &str) -> Vec<ImagePair> {
    // Go straight at the HTML rather than roundtripping through markdown:
    // html-to-markdown collapses images-inside-links with empty alt down
    // to just the link, dropping the image.
    use std::sync::OnceLock;
    static LINKED_RE: OnceLock<regex::Regex> = OnceLock::new();
    static IMG_RE: OnceLock<regex::Regex> = OnceLock::new();
    static ATTR_RE: OnceLock<regex::Regex> = OnceLock::new();
    let linked_re = LINKED_RE.get_or_init(|| {
        regex::Regex::new(r#"(?is)<a\b([^>]*)>(.*?)</a>"#).unwrap()
    });
    let img_re =
        IMG_RE.get_or_init(|| regex::Regex::new(r#"(?is)<img\b([^>]*)>"#).unwrap());
    let attr_re = ATTR_RE.get_or_init(|| {
        regex::Regex::new(r#"(?is)\b(src|alt|href|title)\s*=\s*"([^"]*)""#).unwrap()
    });

    #[derive(Default)]
    struct AttrSet {
        src: Option<String>,
        alt: String,
        href: Option<String>,
        title: Option<String>,
    }
    let read_attrs = |attrs: &str| -> AttrSet {
        let mut out = AttrSet::default();
        for cap in attr_re.captures_iter(attrs) {
            let name = cap.get(1).unwrap().as_str().to_ascii_lowercase();
            let val = cap.get(2).unwrap().as_str().to_string();
            match name.as_str() {
                "src" => out.src = Some(val),
                "alt" => out.alt = val,
                "href" => out.href = Some(val),
                "title" => out.title = Some(val),
                _ => {}
            }
        }
        out
    };

    let build_run = |url: &str| -> Option<BodyRun> {
        if let Some(cid) = url.strip_prefix("cid:") {
            Some(BodyRun::Image(cid.to_string()))
        } else if (url.starts_with("http://") || url.starts_with("https://"))
            && looks_like_image_url(url)
        {
            Some(BodyRun::Url(url.to_string()))
        } else {
            None
        }
    };

    // Pass 1: `<a ...> ... <img ...> ... </a>` blocks. Capture the href
    // as a primary anchor for any contained images.
    let mut pairs: Vec<ImagePair> = Vec::new();
    let mut consumed_starts: Vec<usize> = Vec::new();
    for cap in linked_re.captures_iter(html) {
        let a_attrs = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let inner_match = cap.get(2);
        let inner = inner_match.map(|m| m.as_str()).unwrap_or("");
        let inner_start_in_html = inner_match.map(|m| m.start()).unwrap_or(0);
        let parent = read_attrs(a_attrs);
        for img in img_re.captures_iter(inner) {
            consumed_starts.push(inner_start_in_html + img.get(0).unwrap().start());
            let attrs = read_attrs(img.get(1).map(|m| m.as_str()).unwrap_or(""));
            let Some(url) = attrs.src else { continue };
            let Some(run) = build_run(&url) else { continue };
            let mut anchors = Vec::new();
            if let Some(h) = parent.href.as_ref() {
                anchors.push(h.clone());
            }
            if let Some(t) = parent.title.as_ref() {
                anchors.push(t.clone());
            }
            if !attrs.alt.is_empty() {
                anchors.push(format!("[{}]", attrs.alt));
            }
            pairs.push(ImagePair { anchors, run });
        }
    }

    // Pass 2: `<img>` tags NOT inside an `<a>` we already consumed.
    for img in img_re.captures_iter(html) {
        let start = img.get(0).unwrap().start();
        if consumed_starts.iter().any(|&s| s == start) {
            continue;
        }
        let attrs = read_attrs(img.get(1).map(|m| m.as_str()).unwrap_or(""));
        let Some(url) = attrs.src else { continue };
        let Some(run) = build_run(&url) else { continue };
        let mut anchors = Vec::new();
        if !attrs.alt.is_empty() {
            anchors.push(format!("[{}]", attrs.alt));
        }
        pairs.push(ImagePair { anchors, run });
    }

    pairs
}

/// Walk the plain-text body left to right, looking for any unused
/// `[alt-text]` anchor that matches one of the HTML images. Each match
/// splits the text into a Text run + the matching image run + the rest
/// of the text. Unmatched images get appended at the tail so they're
/// still discoverable.
fn splice_text_with_images(text: &str, pairs: &[ImagePair]) -> Vec<BodyRun> {
    // Phase 1: anchor matching. For each image, find the earliest position
    // at which any of its anchors appears anywhere in the text.
    let mut matches: Vec<(usize, usize, usize)> = Vec::new(); // (pos, len, idx)
    let mut unanchored: Vec<usize> = Vec::new();
    for (i, pair) in pairs.iter().enumerate() {
        let mut best: Option<(usize, usize)> = None;
        for anchor in &pair.anchors {
            if anchor.is_empty() {
                continue;
            }
            if let Some(pos) = text.find(anchor.as_str()) {
                if best.map_or(true, |b| pos < b.0) {
                    best = Some((pos, anchor.len()));
                }
            }
        }
        match best {
            Some((pos, len)) => matches.push((pos, len, i)),
            None => unanchored.push(i),
        }
    }

    // Sort anchor matches by position; drop overlapping ones (kick them
    // back into unanchored so they get positional treatment).
    matches.sort_by_key(|m| m.0);
    let mut filtered: Vec<(usize, usize, usize)> = Vec::new();
    let mut last_end = 0;
    for m in matches {
        if m.0 >= last_end {
            last_end = m.0 + m.1;
            filtered.push(m);
        } else {
            unanchored.push(m.2);
        }
    }

    // Phase 2: positional fallback. For each remaining image (in source
    // order), find the next standalone-URL line in the text that isn't
    // already inside an anchored splice region.
    let url_positions = standalone_url_positions(text);
    let mut next_url = 0;
    let mut positional: Vec<(usize, usize, usize)> = Vec::new();
    for img_idx in unanchored.iter().copied() {
        while next_url < url_positions.len() {
            let (start, end) = url_positions[next_url];
            next_url += 1;
            // Skip URLs that fall inside an already-anchored region.
            if filtered
                .iter()
                .any(|m| start >= m.0 && start < m.0 + m.1)
            {
                continue;
            }
            positional.push((start, end - start, img_idx));
            break;
        }
    }

    // Phase 3: merge anchored + positional splices in text order, walk
    // through emitting Text and image runs.
    let mut all: Vec<(usize, usize, usize)> = filtered;
    all.extend(positional);
    all.sort_by_key(|m| m.0);

    let mut runs: Vec<BodyRun> = Vec::new();
    let mut used = vec![false; pairs.len()];
    let mut cursor = 0;
    for (pos, len, idx) in all {
        if pos < cursor {
            // Overlap with a previous splice — drop this one.
            continue;
        }
        let prefix = &text[cursor..pos];
        if !prefix.is_empty() {
            runs.push(BodyRun::Text(plain_lines(prefix)));
        }
        runs.push(pairs[idx].run.clone());
        used[idx] = true;
        cursor = pos + len;
    }
    let tail = &text[cursor..];
    if !tail.is_empty() {
        runs.push(BodyRun::Text(plain_lines(tail)));
    }

    // Phase 4: any images still unmatched (e.g. ran out of URL lines)
    // get appended at the bottom under a separator so they're not lost.
    let leftovers: Vec<&ImagePair> = pairs
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (!used[i]).then_some(p))
        .collect();
    if !leftovers.is_empty() {
        let sep = Line::from(Span::styled(
            "── images ──",
            Style::default().fg(Color::DarkGray),
        ));
        runs.push(BodyRun::Text(vec![Line::default(), sep, Line::default()]));
        for p in leftovers {
            runs.push(p.run.clone());
        }
    }

    if runs.is_empty() {
        runs.push(BodyRun::Text(plain_lines("")));
    }
    runs
}

/// Find positions of lines in `text` that contain only an http(s) URL
/// (with surrounding whitespace allowed). Returns (start, end) byte
/// offsets of the URL itself within the line.
fn standalone_url_positions(text: &str) -> Vec<(usize, usize)> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?m)^[[:space:]]*(https?://\S+)[[:space:]]*$").unwrap()
    });
    re.captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| (m.start(), m.end())))
        .collect()
}

fn strip_parenthesised_urls(text: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?s)\(\s*https?://[^)\s]+(?:\s+[^)]+)?\s*\)").unwrap()
    });
    re.replace_all(text, "").into_owned()
}

fn host_label(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.trim_start_matches("www.");
    if host.is_empty() {
        None
    } else {
        Some(format!("[{host}]"))
    }
}

fn strip_head_metadata(html: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(concat!(
            r"(?is)<head\b[^>]*>.*?</head>",
            r"|<style\b[^>]*>.*?</style>",
            r"|<script\b[^>]*>.*?</script>",
            r"|<title\b[^>]*>.*?</title>",
            r"|<meta\b[^>]*/?>",
            r"|<link\b[^>]*/?>",
        ))
        .unwrap()
    });
    re.replace_all(html, "").into_owned()
}

fn is_layout_table_html(html: &str) -> bool {
    use std::sync::OnceLock;
    static TABLE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static TH_RE: OnceLock<regex::Regex> = OnceLock::new();
    let table_re = TABLE_RE.get_or_init(|| regex::Regex::new(r"(?i)<table\b").unwrap());
    let th_re = TH_RE.get_or_init(|| regex::Regex::new(r"(?i)<th\b").unwrap());
    table_re.is_match(html) && !th_re.is_match(html)
}

fn flatten_layout_tables(html: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?i)</?(table|tbody|thead|tfoot|tr|td|th)\b[^>]*>").unwrap()
    });
    re.replace_all(html, |c: &regex::Captures| {
        if c.get(0).unwrap().as_str().starts_with("</") {
            "</div>"
        } else {
            "<div>"
        }
    })
    .into_owned()
}

fn render_html(html: &str) -> Vec<BodyRun> {
    let mut prepped = strip_head_metadata(html);
    if is_layout_table_html(&prepped) {
        prepped = flatten_layout_tables(&prepped);
    }
    let html = prepped.as_str();
    let options = html_to_markdown_rs::ConversionOptions::builder()
        .wrap(true)
        .wrap_width(RENDER_WIDTH)
        // Marketing email HTML nests images inside <td>/<tr>/<table> for
        // layout; the default keep_inline_images_in list is too narrow
        // and drops them. Override with every container we plausibly see.
        .keep_inline_images_in(
            ["p", "div", "td", "th", "tr", "table", "tbody", "thead",
             "li", "ul", "ol", "a", "span", "section", "article",
             "header", "footer", "main", "body", "blockquote"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
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
    link_urls: Vec<(String, usize, usize)>,
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
                self.push_style(|s| {
                    s.fg(Color::Blue).add_modifier(Modifier::UNDERLINED)
                });
                self.link_urls.push((
                    dest_url.into_string(),
                    self.current_line.len(),
                    self.runs.len(),
                ));
            }
            Event::End(TagEnd::Link) => {
                if let Some((url, start_line_len, start_runs_len)) = self.link_urls.pop() {
                    let emitted_inline = self.current_line.len() > start_line_len;
                    let emitted_image = self.runs.len() > start_runs_len;
                    if !emitted_inline && !emitted_image {
                        let label = host_label(&url).unwrap_or(url);
                        let style = self.current_style();
                        self.current_line.push(Span::styled(label, style));
                    }
                }
                self.pop_style();
            }

            Event::Start(Tag::Image { dest_url, .. }) => {
                self.in_image += 1;
                self.flush_text_run();
                let url = dest_url.into_string();
                if let Some(cid) = url.strip_prefix("cid:") {
                    self.runs.push(BodyRun::Image(cid.to_string()));
                } else if (url.starts_with("http://") || url.starts_with("https://"))
                    && looks_like_image_url(&url)
                {
                    self.runs.push(BodyRun::Url(url));
                }
                // Other URLs (e.g. Outlook deep links) drop silently.
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

