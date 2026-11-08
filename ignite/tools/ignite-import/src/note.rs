//! Markdown note (Obsidian flavour) → page spec, laid out with the Kindle's own text metrics.
//!
//! Every block becomes a text object (display-only on the Kindle), so the pages read like the
//! note. Structure that should stay *writable* is drawn as ink instead: task checkboxes are real
//! rectangles you can tick with the pen, rules and quote bars are ink lines. Pagination uses
//! [`crate::layout`], which mirrors the Kindle's wrap, so text never runs off a page or into the
//! chrome bands (the builder re-checks that anyway).
//!
//! Supported: frontmatter (dropped), `#`–`######` headings, paragraphs (single newlines kept, as
//! Obsidian shows them), `-`/`*`/`+`/`1.` lists with nesting, `- [ ]`/`- [x]` tasks, `>` quotes
//! and callouts, fenced code, `---` rules, tables (kept as text). Inline markup is reduced to its
//! text (`**b**`, `*i*`, `` `c` ``, `[t](url)`, `[[a|b]]`, `==h==`, `~~s~~`, `%%comment%%`).
//! Embeds (`![[x]]`, `![](x)`) become a placeholder line: images are not sent yet.

use sync_protocol::CANONICAL_PAGE_WIDTH as W;

/// Kindle title limit (`sync_protocol::content::MAX_TITLE_CHARS`, not re-exported).
const MAX_TITLE_CHARS: usize = 96;

use crate::build::safe_y_range;
use crate::layout::{missing_glyph, wrap_lines, LINE_HEIGHT_EM};
use crate::spec::{Coords, NotebookIn, PageIn, Paper, Shape, Spec, StrokeIn};

pub const BODY: f32 = 360.0;
const H_SIZES: [f32; 6] = [640.0, 520.0, 440.0, 400.0, 380.0, 360.0];
const MARGIN_X: f32 = 0.07 * W;
const INDENT: f32 = 0.045 * W;
/// Hard cap so a huge note cannot flood the Kindle.
pub const MAX_PAGES: usize = 80;

#[derive(Clone, Debug)]
pub struct NoteOptions {
    pub title: Option<String>,
    pub paper: Paper,
    /// Blank writing pages appended after the note (same paper).
    pub extra_pages: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Heading(usize),
    Para,
    Item { depth: usize, task: Option<bool> },
    Quote,
    Code,
    Rule,
}

#[derive(Clone, Debug)]
struct Block {
    kind: Kind,
    text: String,
}

fn strip_frontmatter(md: &str) -> &str {
    let Some(rest) = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"))
    else {
        return md;
    };
    let mut off = 0;
    for line in rest.split_inclusive('\n') {
        off += line.len();
        if line.trim_end() == "---" {
            return &rest[off..];
        }
    }
    md
}

/// Reduce inline markup to plain text.
pub fn inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let find = |from: usize, pat: &str| -> Option<usize> {
        let p: Vec<char> = pat.chars().collect();
        (from..c.len().saturating_sub(p.len() - 1)).find(|&k| c[k..k + p.len()] == p[..])
    };
    'outer: while i < c.len() {
        let rest_starts = |p: &str| c[i..].iter().take(p.chars().count()).copied().eq(p.chars());
        if rest_starts("%%") {
            if let Some(e) = find(i + 2, "%%") {
                i = e + 2;
                continue;
            }
        }
        if rest_starts("![[") {
            if let Some(e) = find(i + 3, "]]") {
                let name: String = c[i + 3..e].iter().collect();
                out.push_str(&format!(
                    "[image: {}]",
                    name.split('|').next().unwrap_or("")
                ));
                i = e + 2;
                continue;
            }
        }
        if rest_starts("[[") {
            if let Some(e) = find(i + 2, "]]") {
                let inner: String = c[i + 2..e].iter().collect();
                let shown = inner.rsplit('|').next().unwrap_or(&inner);
                out.push_str(
                    shown
                        .split('#')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(shown),
                );
                i = e + 2;
                continue;
            }
        }
        if c[i] == '!' && c.get(i + 1) == Some(&'[') {
            if let Some(m) = find(i + 2, "](") {
                if let Some(e) = find(m + 2, ")") {
                    let alt: String = c[i + 2..m].iter().collect();
                    out.push_str(&format!(
                        "[image: {}]",
                        if alt.is_empty() { "embedded" } else { &alt }
                    ));
                    i = e + 1;
                    continue;
                }
            }
        }
        if c[i] == '[' {
            if let Some(m) = find(i + 1, "](") {
                if let Some(e) = find(m + 2, ")") {
                    if !c[i + 1..m].contains(&'\n') {
                        out.extend(c[i + 1..m].iter());
                        i = e + 1;
                        continue;
                    }
                }
            }
        }
        for pat in ["**", "__", "==", "~~"] {
            if rest_starts(pat) {
                if let Some(e) = find(i + 2, pat) {
                    out.push_str(&inline(&c[i + 2..e].iter().collect::<String>()));
                    i = e + 2;
                    continue 'outer;
                }
            }
        }
        if c[i] == '`' {
            if let Some(e) = find(i + 1, "`") {
                out.extend(c[i + 1..e].iter());
                i = e + 1;
                continue;
            }
        }
        let boundary_before = i == 0 || !c[i - 1].is_alphanumeric();
        if c[i] == '_' && boundary_before && c.get(i + 1).is_some_and(|n| !n.is_whitespace()) {
            if let Some(e) = (i + 1..c.len())
                .find(|&k| c[k] == '_' && c.get(k + 1).is_none_or(|n| !n.is_alphanumeric()))
            {
                out.push_str(&inline(&c[i + 1..e].iter().collect::<String>()));
                i = e + 1;
                continue;
            }
        }
        if c[i] == '*' && c.get(i + 1).is_some_and(|n| !n.is_whitespace()) {
            if let Some(e) = find(i + 1, "*") {
                out.push_str(&inline(&c[i + 1..e].iter().collect::<String>()));
                i = e + 1;
                continue;
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

fn list_marker(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let t = line.trim_start();
    let depth = line[..indent]
        .chars()
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum::<usize>()
        / 2;
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(m) {
            return Some((depth, r));
        }
    }
    let digits = t.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") ")) {
        return Some((depth, &t[..]));
    }
    None
}

fn parse(md: &str) -> Vec<Block> {
    let body = strip_frontmatter(md).replace("\r\n", "\n");
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut quote: Vec<String> = Vec::new();
    let mut lines = body.lines().peekable();
    let flush = |blocks: &mut Vec<Block>, buf: &mut Vec<String>, kind: Kind| {
        if !buf.is_empty() {
            let text = buf.join("\n");
            if !text.trim().is_empty() {
                blocks.push(Block { kind, text });
            }
            buf.clear();
        }
    };
    while let Some(line) = lines.next() {
        let t = line.trim();
        if !t.starts_with('>') {
            flush(&mut blocks, &mut quote, Kind::Quote);
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut blocks, &mut para, Kind::Para);
            let fence = &t[..3];
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with(fence) {
                    break;
                }
                code.push(l.to_string());
            }
            if !code.is_empty() {
                blocks.push(Block {
                    kind: Kind::Code,
                    text: code.join("\n"),
                });
            }
            continue;
        }
        if t.is_empty() {
            flush(&mut blocks, &mut para, Kind::Para);
            continue;
        }
        if t.chars().all(|ch| ch == '-' || ch == '*' || ch == '_')
            && t.len() >= 3
            && !t.contains(' ')
        {
            flush(&mut blocks, &mut para, Kind::Para);
            blocks.push(Block {
                kind: Kind::Rule,
                text: String::new(),
            });
            continue;
        }
        let hashes = t.chars().take_while(|&ch| ch == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut blocks, &mut para, Kind::Para);
            blocks.push(Block {
                kind: Kind::Heading(hashes),
                text: inline(t[hashes..].trim()),
            });
            continue;
        }
        if let Some(q) = t.strip_prefix('>') {
            flush(&mut blocks, &mut para, Kind::Para);
            let q = q.strip_prefix(' ').unwrap_or(q);
            // Callout header `[!note] Title` → just the title (or the type).
            let q = if let Some(r) = q.strip_prefix("[!") {
                match r.find(']') {
                    Some(e) => {
                        let title = r[e + 1..].trim_start_matches(['-', '+']).trim();
                        if title.is_empty() {
                            r[..e].to_string()
                        } else {
                            title.to_string()
                        }
                    }
                    None => q.to_string(),
                }
            } else {
                q.to_string()
            };
            quote.push(inline(&q));
            continue;
        }
        if let Some((depth, rest)) = list_marker(line) {
            flush(&mut blocks, &mut para, Kind::Para);
            let (task, text) = if let Some(r) = rest.strip_prefix("[ ] ") {
                (Some(false), r)
            } else if let Some(r) = rest
                .strip_prefix("[x] ")
                .or_else(|| rest.strip_prefix("[X] "))
            {
                (Some(true), r)
            } else {
                (None, rest)
            };
            blocks.push(Block {
                kind: Kind::Item { depth, task },
                text: inline(text.trim_end()),
            });
            continue;
        }
        // Lazy continuation of a list item.
        if line.starts_with("  ") && para.is_empty() {
            if let Some(Block {
                kind: Kind::Item { .. },
                text,
            }) = blocks.last_mut()
            {
                text.push('\n');
                text.push_str(&inline(t));
                continue;
            }
        }
        para.push(inline(line.trim_end()));
    }
    flush(&mut blocks, &mut para, Kind::Para);
    flush(&mut blocks, &mut quote, Kind::Quote);
    blocks
}

struct Pages {
    paper: Paper,
    pages: Vec<PageIn>,
    y: f32,
    top: f32,
    bottom: f32,
}

impl Pages {
    fn new(paper: Paper) -> Self {
        let (safe_top, safe_bottom) = safe_y_range();
        let top = safe_top + 0.025 * W;
        Self {
            paper,
            pages: vec![PageIn {
                paper,
                ..Default::default()
            }],
            y: top,
            top,
            bottom: safe_bottom - 0.01 * W,
        }
    }
    fn page(&mut self) -> &mut PageIn {
        self.pages.last_mut().unwrap()
    }
    fn new_page(&mut self) -> Result<(), String> {
        if self.pages.len() >= MAX_PAGES {
            return Err(format!(
                "the note needs more than {MAX_PAGES} Kindle pages; split it first"
            ));
        }
        self.pages.push(PageIn {
            paper: self.paper,
            ..Default::default()
        });
        self.y = self.top;
        Ok(())
    }
    fn room(&self) -> f32 {
        self.bottom - self.y
    }
    fn at_top(&self) -> bool {
        self.y <= self.top + 1.0
    }
}

/// Wrapped lines with a flag for "this line ends a source paragraph".
fn visual_lines(text: &str, size: f32, width: f32) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let lines = wrap_lines(para, size, width);
        let n = lines.len();
        out.extend(lines.into_iter().enumerate().map(|(i, l)| (l, i + 1 == n)));
    }
    out
}

fn join(lines: &[(String, bool)]) -> String {
    let mut s = String::new();
    for (i, (l, end)) in lines.iter().enumerate() {
        s.push_str(l);
        if *end && i + 1 < lines.len() {
            s.push('\n');
        }
    }
    s
}

/// Lay out `text` at `x`, splitting across pages by whole lines. Returns the (page, y, height)
/// of the first chunk so callers can attach ink (checkboxes, quote bars) to it.
fn place_text(
    p: &mut Pages,
    text: &str,
    x: f32,
    size: f32,
    keep_with_next: f32,
) -> Result<Vec<(usize, f32, f32)>, String> {
    let width = W - MARGIN_X - x;
    let line_h = LINE_HEIGHT_EM * size;
    let mut lines = visual_lines(text, size, width);
    let mut placed = Vec::new();
    while !lines.is_empty() {
        let fit = (p.room() / line_h).floor().max(0.0) as usize;
        let need_whole = lines.len() as f32 * line_h + keep_with_next;
        // Move to a fresh page when nothing fits, when a short block would be split, or when a
        // heading would be stranded without the start of what follows.
        if !p.at_top()
            && (fit == 0
                || (lines.len() <= 3 && fit < lines.len())
                || (keep_with_next > 0.0 && need_whole > p.room()))
        {
            p.new_page()?;
            continue;
        }
        let take = fit.max(1).min(lines.len());
        let chunk: Vec<(String, bool)> = lines.drain(..take).collect();
        let content = join(&chunk);
        let h = take as f32 * line_h;
        if !content.trim().is_empty() {
            let (y, page) = (p.y, p.pages.len() - 1);
            p.page().texts.push(crate::spec::TextIn {
                x,
                y,
                width,
                font_size: size,
                content,
            });
            placed.push((page, y, h));
        }
        p.y += h;
        if !lines.is_empty() {
            p.new_page()?;
        }
    }
    Ok(placed)
}

/// Build the spec. Returns the spec and human-readable warnings (e.g. glyphs the Kindle font
/// lacks, embeds that were replaced by placeholders).
pub fn from_markdown(
    md: &str,
    fallback_title: &str,
    opts: &NoteOptions,
) -> Result<(Spec, Vec<String>), String> {
    let mut blocks = parse(md);
    let first_h1 = blocks.iter().position(|b| b.kind == Kind::Heading(1));
    let mut title = opts
        .title
        .clone()
        .or_else(|| first_h1.filter(|&i| i == 0).map(|i| blocks[i].text.clone()))
        .unwrap_or_else(|| fallback_title.to_string())
        .trim()
        .to_string();
    if title.is_empty() {
        title = "Note".into();
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        title = title.chars().take(MAX_TITLE_CHARS - 1).collect::<String>() + "…";
    }
    if first_h1 != Some(0) {
        blocks.insert(
            0,
            Block {
                kind: Kind::Heading(1),
                text: title.clone(),
            },
        );
    }

    let mut warnings = Vec::new();
    let mut missing: Vec<char> = blocks
        .iter()
        .flat_map(|b| b.text.chars())
        .filter(|&ch| missing_glyph(ch))
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if !missing.is_empty() {
        warnings.push(format!(
            "the Kindle font has no glyph for {:?}; they will show as boxes",
            missing.iter().collect::<String>()
        ));
    }
    let embeds = blocks
        .iter()
        .filter(|b| b.text.contains("[image: "))
        .count();
    if embeds > 0 {
        warnings.push(format!(
            "{embeds} block(s) contain images or embeds, sent as placeholders only"
        ));
    }

    let mut p = Pages::new(opts.paper);
    let mut prev: Option<&Kind> = None;
    for b in &blocks {
        let gap_before = match (&b.kind, prev) {
            (_, None) => 0.0,
            (Kind::Heading(_), _) => 0.9 * BODY,
            (Kind::Item { .. }, Some(Kind::Item { .. })) => 0.15 * BODY,
            _ => 0.55 * BODY,
        };
        if !p.at_top() {
            p.y += gap_before;
        }
        match &b.kind {
            Kind::Heading(level) => {
                let size = H_SIZES[level - 1];
                place_text(&mut p, &b.text, MARGIN_X, size, 2.0 * LINE_HEIGHT_EM * BODY)?;
                if *level <= 2 {
                    // Ink underline: a writable divider that also reads as structure.
                    let y = p.y + 0.12 * size;
                    if y < p.bottom {
                        p.page().shapes.push(Shape::Line {
                            from: [MARGIN_X, y],
                            to: [W - MARGIN_X, y],
                        });
                        p.y = y + 0.1 * size;
                    }
                }
            }
            Kind::Para => {
                place_text(&mut p, &b.text, MARGIN_X, BODY, 0.0)?;
            }
            Kind::Code => {
                place_text(&mut p, &b.text, MARGIN_X + INDENT, 0.9 * BODY, 0.0)?;
            }
            Kind::Quote => {
                let x = MARGIN_X + INDENT;
                for (page, y, h) in place_text(&mut p, &b.text, x, BODY, 0.0)? {
                    let bar = MARGIN_X + 0.35 * INDENT;
                    p.pages[page].shapes.push(Shape::Line {
                        from: [bar, y],
                        to: [bar, y + h],
                    });
                }
            }
            Kind::Item { depth, task } => {
                let base = MARGIN_X + *depth as f32 * INDENT;
                match task {
                    None => {
                        // Marker and text are separate objects so wrapped lines hang-indent.
                        let digits = b.text.chars().take_while(|ch| ch.is_ascii_digit()).count();
                        let (marker, text, offset) = if digits > 0 {
                            (
                                b.text[..digits + 1].to_string(),
                                b.text[digits + 1..].trim_start(),
                                0.8 * INDENT,
                            )
                        } else {
                            ("•".to_string(), b.text.as_str(), 0.55 * INDENT)
                        };
                        let placed = place_text(&mut p, text, base + offset, BODY, 0.0)?;
                        if let Some(&(page, y, _)) = placed.first() {
                            let width = offset.max(crate::layout::line_width(&marker, BODY) + 1.0);
                            p.pages[page].texts.push(crate::spec::TextIn {
                                x: base,
                                y,
                                width,
                                font_size: BODY,
                                content: marker,
                            });
                        }
                    }
                    Some(done) => {
                        let placed = place_text(&mut p, &b.text, base + 0.9 * INDENT, BODY, 0.0)?;
                        if let Some(&(page, y, _)) = placed.first() {
                            let s = 0.72 * BODY;
                            let (bx, by) = (
                                base + 0.05 * INDENT,
                                y + 0.5 * LINE_HEIGHT_EM * BODY - 0.5 * s,
                            );
                            p.pages[page].shapes.push(Shape::Rect {
                                x: bx,
                                y: by,
                                w: s,
                                h: s,
                            });
                            if *done {
                                p.pages[page].strokes.push(StrokeIn {
                                    style: Default::default(),
                                    points: vec![
                                        vec![bx + 0.18 * s, by + 0.52 * s],
                                        vec![bx + 0.42 * s, by + 0.78 * s],
                                        vec![bx + 0.85 * s, by + 0.2 * s],
                                    ],
                                });
                            }
                        }
                    }
                }
            }
            Kind::Rule => {
                if p.room() < BODY {
                    p.new_page()?;
                } else {
                    let y = p.y + 0.3 * BODY;
                    p.page().shapes.push(Shape::Line {
                        from: [MARGIN_X + 2.0 * INDENT, y],
                        to: [W - MARGIN_X - 2.0 * INDENT, y],
                    });
                    p.y = y + 0.3 * BODY;
                }
            }
        }
        prev = Some(&b.kind);
    }
    // Drop a trailing page that only a page break produced.
    if p.pages.len() > 1 {
        let last = p.pages.last().unwrap();
        if last.texts.is_empty() && last.shapes.is_empty() && last.strokes.is_empty() {
            p.pages.pop();
        }
    }
    for _ in 0..opts.extra_pages {
        p.new_page()?;
    }
    Ok((
        Spec {
            coords: Coords::Canonical,
            notebook: NotebookIn { title },
            pages: p.pages,
        },
        warnings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{build, Ctx};
    use sync_protocol::{LibraryId, ReplicaId};
    use uuid::Uuid;

    fn opts() -> NoteOptions {
        NoteOptions {
            title: None,
            paper: Paper::Blank,
            extra_pages: 0,
        }
    }

    fn ctx() -> Ctx {
        Ctx {
            library_id: LibraryId::from_uuid(Uuid::now_v7()).unwrap(),
            replica_id: ReplicaId::from_uuid(Uuid::now_v7()).unwrap(),
            first_client_seq: 1,
            render: crate::build::default_render(),
            after_notebook_id: None,
        }
    }

    #[test]
    fn inline_markup_is_reduced_to_text() {
        assert_eq!(inline("**bold** and *it* `code` [link](http://x) [[Note|alias]] [[Other#h]] ==hi== ~~s~~ %%hidden%%end"),
            "bold and it code link alias Other hi s end");
        assert_eq!(
            inline("![[pic.png]] a_b_c 2*3 _(status)_ snake_case"),
            "[image: pic.png] a_b_c 2*3 (status) snake_case"
        );
    }

    #[test]
    fn parses_obsidian_structure() {
        let md = "---\ntags: [x]\n---\n# Title\n\nIntro line one\nline two\n\n## Tasks\n- [ ] open\n- [x] done\n  - nested\n> [!tip] Remember\n> quoted\n\n```\ncode\n```\n---\n1. first";
        let b = parse(md);
        let kinds: Vec<_> = b.iter().map(|b| b.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Heading(1),
                Kind::Para,
                Kind::Heading(2),
                Kind::Item {
                    depth: 0,
                    task: Some(false)
                },
                Kind::Item {
                    depth: 0,
                    task: Some(true)
                },
                Kind::Item {
                    depth: 1,
                    task: None
                },
                Kind::Quote,
                Kind::Code,
                Kind::Rule,
                Kind::Item {
                    depth: 0,
                    task: None
                },
            ]
        );
        assert_eq!(b[1].text, "Intro line one\nline two");
        assert_eq!(b[6].text, "Remember\nquoted");
        assert_eq!(b[9].text, "1. first");
    }

    #[test]
    fn long_notes_paginate_inside_the_safe_area_and_build() {
        let para =
            "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor. "
                .repeat(12);
        let md = format!(
            "# Long\n\n{}\n\n- [ ] tick me\n- [x] ticked",
            vec![para; 12].join("\n\n")
        );
        let (spec, warnings) = from_markdown(&md, "fallback", &opts()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(spec.pages.len() >= 3, "{} pages", spec.pages.len());
        let (top, bottom) = safe_y_range();
        for page in &spec.pages {
            for t in &page.texts {
                let h = crate::layout::layout_height(&t.content, t.font_size, t.width);
                assert!(t.y >= top && t.y + h <= bottom, "text at {} + {h}", t.y);
            }
        }
        // The builder (with its own band and validity checks) accepts the result.
        let plan = build(&spec, &ctx(), &mut Uuid::now_v7).unwrap();
        assert_eq!(plan.pages.len(), spec.pages.len());
        let all: String = spec
            .pages
            .iter()
            .flat_map(|p| p.texts.iter().map(|t| t.content.clone()))
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(
            all.matches("Lorem").count(),
            144,
            "no text lost across page splits"
        );
        let boxes = spec
            .pages
            .iter()
            .flat_map(|p| &p.shapes)
            .filter(|s| matches!(s, Shape::Rect { .. }))
            .count();
        assert_eq!(boxes, 2, "two task checkboxes drawn as ink");
    }

    #[test]
    fn title_rules() {
        let (s, _) = from_markdown("# From Heading\nbody", "file", &opts()).unwrap();
        assert_eq!(s.notebook.title, "From Heading");
        let (s, _) = from_markdown("body only", "file name", &opts()).unwrap();
        assert_eq!(s.notebook.title, "file name");
        assert_eq!(s.pages[0].texts[0].content, "file name");
        let (s, _) = from_markdown(
            "x",
            "f",
            &NoteOptions {
                title: Some("Given".into()),
                paper: Paper::Lined,
                extra_pages: 2,
            },
        )
        .unwrap();
        assert_eq!(s.notebook.title, "Given");
        assert_eq!(s.pages.len(), 3);
        assert!(s.pages.iter().all(|p| p.paper == Paper::Lined));
    }

    #[test]
    fn warns_about_missing_glyphs_and_embeds() {
        let (_, w) = from_markdown("private \u{E000} use\n\n![[photo.jpg]]", "f", &opts()).unwrap();
        assert_eq!(w.len(), 2, "{w:?}");
    }
}
