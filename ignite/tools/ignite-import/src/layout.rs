//! Text-object wrapping with the Kindle's exact metrics, so generated pages can be paginated
//! and checked against the chrome bands before anything is sent.
//!
//! Mirrors `scribe-app/src/text_paint.rs` (`wrap_lines`, `text_layout_height`): same font files
//! (`default-sans-v1` = Source Sans 3 Regular, Noto Emoji as cmap fallback), same advance
//! measurement at a 100 px reference size, same greedy wrap that breaks after spaces and keeps
//! an over-long run on its own line. If that file changes, change this one too.

use std::sync::OnceLock;

use fontdue::{Font, FontSettings};

const DEFAULT_SANS_V1_TTF: &[u8] = include_bytes!("../../../assets/typography/default-sans-v1.ttf");
const NOTO_EMOJI_TTF: &[u8] =
    include_bytes!("../../../assets/typography/noto-emoji/NotoEmoji[wght].ttf");

/// Line height as a multiple of `font_size` (Kindle `LINE_HEIGHT_EM`).
pub const LINE_HEIGHT_EM: f32 = 1.2;
const METRIC_PX: f32 = 100.0;

static PRIMARY: OnceLock<Font> = OnceLock::new();
static EMOJI: OnceLock<Font> = OnceLock::new();

fn primary() -> &'static Font {
    PRIMARY.get_or_init(|| {
        Font::from_bytes(DEFAULT_SANS_V1_TTF, FontSettings::default())
            .expect("default-sans-v1.ttf must parse")
    })
}

fn emoji() -> &'static Font {
    EMOJI.get_or_init(|| {
        Font::from_bytes(NOTO_EMOJI_TTF, FontSettings::default())
            .expect("NotoEmoji[wght].ttf must parse")
    })
}

fn is_variation_selector(ch: char) -> bool {
    matches!(ch as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

fn cmap(font: &Font, ch: char) -> Option<u16> {
    let idx = font.lookup_glyph_index(ch);
    (idx != 0 || ch == '\0').then_some(idx)
}

fn advance(ch: char, next_is_emoji_vs: bool, font_size: f32) -> f32 {
    if is_variation_selector(ch) {
        return 0.0;
    }
    let (font, index) = if next_is_emoji_vs {
        match cmap(emoji(), ch) {
            Some(i) => (emoji(), i),
            None => (primary(), cmap(primary(), ch).unwrap_or(0)),
        }
    } else if let Some(i) = cmap(primary(), ch) {
        (primary(), i)
    } else if let Some(i) = cmap(emoji(), ch) {
        (emoji(), i)
    } else {
        (primary(), 0)
    };
    font.metrics_indexed(index, METRIC_PX).advance_width / METRIC_PX * font_size
}

/// True when neither face has a glyph for `ch` (the Kindle would paint a `.notdef` box).
pub fn missing_glyph(ch: char) -> bool {
    !(ch == '\n'
        || ch == ' '
        || is_variation_selector(ch)
        || cmap(primary(), ch).is_some()
        || cmap(emoji(), ch).is_some())
}

/// Width of one line (no wrapping) in canonical units.
pub fn line_width(s: &str, font_size: f32) -> f32 {
    let chars: Vec<char> = s.chars().collect();
    (0..chars.len())
        .map(|i| advance(chars[i], chars.get(i + 1) == Some(&'\u{FE0F}'), font_size))
        .sum()
}

/// Wrap `content` into lines that each measure `<= width` (Kindle `wrap_lines`).
pub fn wrap_lines(content: &str, font_size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in content.split('\n') {
        if para.is_empty() {
            lines.push(String::new());
            continue;
        }
        let chars: Vec<char> = para.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let (mut end, mut last_break, mut w) = (i, None, 0.0f32);
            while end < chars.len() {
                let ch = chars[end];
                let adv = advance(ch, chars.get(end + 1) == Some(&'\u{FE0F}'), font_size);
                if w + adv > width && end > i {
                    break;
                }
                w += adv;
                end += 1;
                if ch == ' ' {
                    last_break = Some(end);
                }
            }
            if end == i {
                end = i + 1;
            } else if end < chars.len() {
                if let Some(b) = last_break {
                    if b > i {
                        end = b;
                    }
                }
            }
            lines.push(chars[i..end].iter().collect());
            i = end;
        }
    }
    lines
}

/// Height of the text box the Kindle lays out (`text_layout_height`).
pub fn layout_height(content: &str, font_size: f32, width: f32) -> f32 {
    wrap_lines(content, font_size, width).len().max(1) as f32 * LINE_HEIGHT_EM * font_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_spaces_and_keeps_newlines() {
        let w = line_width("Hello world", 340.0);
        let lines = wrap_lines("Hello world\n\nnext", 340.0, w * 0.75);
        assert_eq!(lines, vec!["Hello ", "world", "", "next"]);
        assert_eq!(
            wrap_lines("Hello world", 340.0, w + 1.0),
            vec!["Hello world"]
        );
    }

    #[test]
    fn overlong_run_is_broken_not_dropped() {
        let lines = wrap_lines("abcdefghijklmnop", 340.0, line_width("abcd", 340.0));
        assert!(lines.len() >= 4);
        assert_eq!(lines.concat(), "abcdefghijklmnop");
    }

    #[test]
    fn height_counts_lines() {
        assert!((layout_height("x", 100.0, 1000.0) - 120.0).abs() < 0.01);
        assert!((layout_height("x\ny", 100.0, 1000.0) - 240.0).abs() < 0.01);
    }

    #[test]
    fn knows_which_glyphs_exist() {
        assert!(!missing_glyph('a'));
        assert!(!missing_glyph('ü'));
        assert!(!missing_glyph('❤'));
        assert!(missing_glyph('\u{E000}'));
    }
}
