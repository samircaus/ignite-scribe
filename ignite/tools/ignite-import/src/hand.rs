//! **Handwriting**: a single-line font with lowercase, written as ordinary ink.
//!
//! Original glyph data (no third-party font). Each glyph is SVG path data on a grid where the
//! capital height is 10, the x-height 6 (top at y = 4), the baseline y = 10 and descenders reach
//! y = 14 (y grows downwards), plus an advance width. Curves are flattened by [`crate::svg`], and
//! the text is slanted slightly so it reads as handwriting rather than lettering. The result is
//! pen ink on the Kindle: erasable and movable like your own writing.
//!
//! [`layout`] wraps words to a width and honours newlines; [`measure`] gives the size first so
//! a planner can find room on the page.

use crate::svg::{path_polylines, Poly};

const CAP: f32 = 10.0;
const GAP: f32 = 2.5; // between letters
const SPACE: f32 = 7.0;
/// Baseline to baseline, in glyph units (capital 10, descender 4, a little air).
pub const LINE: f32 = 17.0;
/// Rightward lean per unit of height above the baseline.
const SLANT: f32 = 0.12;
/// Curve flattening, in glyph units.
const SEG: f32 = 0.45;

/// Extra space between two upright strokes (u|b, n|l, H|I…), which otherwise read as one letter.
const STEM_GAP: f32 = 1.0;

const DOTS_LOW: &str = "M1.6 2.2 L1.6 2.6 M4.4 2.2 L4.4 2.6";

fn glyph(c: char) -> Option<(&'static str, f32)> {
    Some(match c {
        'a' => ("M6 7 A3 3 0 0 0 0 7 A3 3 0 0 0 6 7 M6 4 L6 10", 6.),
        'b' => ("M0 0 L0 10 M0 7 A3 3 0 0 1 6 7 A3 3 0 0 1 0 7", 6.),
        'c' => ("M5.3 5.07 A3 3 0 1 0 5.3 8.93", 5.6),
        'd' => ("M6 7 A3 3 0 0 0 0 7 A3 3 0 0 0 6 7 M6 0 L6 10", 6.),
        'e' => ("M0.2 7 L6 7 A3 3 0 1 0 5.3 8.93", 6.),
        'f' => ("M5.5 0.8 Q4.6 0 3.6 0 Q2.5 0 2.5 1.6 L2.5 10 M0.4 4 L5 4", 5.5),
        'g' => ("M6 7 A3 3 0 0 0 0 7 A3 3 0 0 0 6 7 M6 4 L6 12 Q6 14 3 14 Q1.2 14 0.4 13", 6.),
        'h' => ("M0 0 L0 10 M0 6.5 Q0.8 4 3 4 Q6 4 6 6.5 L6 10", 6.),
        'i' => ("M0.8 4 L0.8 10 M0.8 1.8 L0.8 2.3", 1.6),
        'j' => ("M2.6 4 L2.6 12 Q2.6 14 0.4 14 M2.6 1.8 L2.6 2.3", 3.2),
        'k' => ("M0 0 L0 10 M5 4 L0 7.6 M1.8 6.3 L5.4 10", 5.4),
        'l' => ("M0.8 0 L0.8 8.8 Q0.8 10 2 10", 2.2),
        'm' => (
            "M0 4 L0 10 M0 6.2 Q0.5 4 2.3 4 Q4.5 4 4.5 6.2 L4.5 10 M4.5 6.2 Q5 4 6.8 4 Q9 4 9 6.2 L9 10",
            9.,
        ),
        'n' => ("M0 4 L0 10 M0 6.5 Q0.8 4 3 4 Q6 4 6 6.5 L6 10", 6.),
        'o' => ("M6 7 A3 3 0 0 0 0 7 A3 3 0 0 0 6 7", 6.),
        'p' => ("M0 4 L0 14 M0 7 A3 3 0 0 1 6 7 A3 3 0 0 1 0 7", 6.),
        'q' => ("M6 7 A3 3 0 0 0 0 7 A3 3 0 0 0 6 7 M6 4 L6 14", 6.),
        'r' => ("M0 4 L0 10 M0 6.6 Q0.9 4 3.4 4 Q4.4 4 5 4.5", 5.),
        's' => (
            "M5 4.8 Q4.1 4 2.6 4 Q0.3 4 0.3 5.5 Q0.3 6.8 2.6 7 Q5 7.2 5 8.5 Q5 10 2.6 10 Q0.8 10 0 9.2",
            5.,
        ),
        't' => ("M2 1.5 L2 8.8 Q2 10 3.4 10 L4.4 10 M0 4 L4.4 4", 4.4),
        'u' => ("M0 4 L0 7.5 Q0 10 3 10 Q6 10 6 7.5 M6 4 L6 10", 6.),
        'v' => ("M0 4 L3 10 L6 4", 6.),
        'w' => ("M0 4 L2 10 L4 5 L6 10 L8 4", 8.),
        'x' => ("M0 4 L5.4 10 M5.4 4 L0 10", 5.4),
        'y' => ("M0 4 L3.1 9.8 M6 4 L1.8 14", 6.),
        'z' => ("M0 4 L5.4 4 L0 10 L5.4 10", 5.4),

        'A' => ("M0 10 L3.5 0 L7 10 M1.3 6.5 L5.7 6.5", 7.),
        'B' => (
            "M0 10 L0 0 L3.5 0 Q6 0 6 2.5 Q6 5 3.5 5 L0 5 M3.5 5 Q6.5 5 6.5 7.5 Q6.5 10 3.5 10 L0 10",
            6.5,
        ),
        'C' => ("M7 1.5 Q6 0 4 0 Q0 0 0 5 Q0 10 4 10 Q6 10 7 8.5", 7.),
        'D' => ("M0 0 L0 10 L3 10 Q7 10 7 5 Q7 0 3 0 L0 0", 7.),
        'E' => ("M6 0 L0 0 L0 10 L6 10 M0 5 L4.5 5", 6.),
        'F' => ("M6 0 L0 0 L0 10 M0 5 L4.5 5", 6.),
        'G' => ("M7 1.5 Q6 0 4 0 Q0 0 0 5 Q0 10 4 10 Q7 10 7 6.5 L7 5.5 L4 5.5", 7.),
        'H' => ("M0 0 L0 10 M7 0 L7 10 M0 5 L7 5", 7.),
        'I' => ("M0 0 L3 0 M1.5 0 L1.5 10 M0 10 L3 10", 3.),
        'J' => ("M5 0 L5 7 Q5 10 2.5 10 Q0 10 0 8", 5.),
        'K' => ("M0 0 L0 10 M6 0 L0 6.5 M2 4.6 L6.5 10", 6.5),
        'L' => ("M0 0 L0 10 L5.5 10", 5.5),
        'M' => ("M0 10 L0 0 L4 7 L8 0 L8 10", 8.),
        'N' => ("M0 10 L0 0 L7 10 L7 0", 7.),
        'O' => ("M8 5 A4 5 0 0 0 0 5 A4 5 0 0 0 8 5", 8.),
        'P' => ("M0 10 L0 0 L3.5 0 Q6.5 0 6.5 2.75 Q6.5 5.5 3.5 5.5 L0 5.5", 6.5),
        'Q' => ("M8 5 A4 5 0 0 0 0 5 A4 5 0 0 0 8 5 M5 7.6 L8.4 11", 8.),
        'R' => ("M0 10 L0 0 L3.5 0 Q6.5 0 6.5 2.75 Q6.5 5.5 3.5 5.5 L0 5.5 M3.5 5.5 L6.5 10", 6.5),
        'S' => (
            "M6 1.3 Q5 0 3 0 Q0.3 0 0.3 2.6 Q0.3 4.6 3 5 Q6 5.4 6 7.6 Q6 10 3 10 Q1 10 0 8.7",
            6.,
        ),
        'T' => ("M0 0 L7 0 M3.5 0 L3.5 10", 7.),
        'U' => ("M0 0 L0 6.5 Q0 10 3.5 10 Q7 10 7 6.5 L7 0", 7.),
        'V' => ("M0 0 L3.5 10 L7 0", 7.),
        'W' => ("M0 0 L2.5 10 L5 3 L7.5 10 L10 0", 10.),
        'X' => ("M0 0 L7 10 M7 0 L0 10", 7.),
        'Y' => ("M0 0 L3.5 5 L7 0 M3.5 5 L3.5 10", 7.),
        'Z' => ("M0 0 L7 0 L0 10 L7 10", 7.),

        '0' => ("M6 5 A3 5 0 0 0 0 5 A3 5 0 0 0 6 5", 6.),
        '1' => ("M1 2 L3.4 0 L3.4 10", 4.4),
        '2' => ("M0.3 2.5 Q0.8 0 3 0 Q6 0 6 2.8 Q6 5 0 10 L6 10", 6.),
        '3' => (
            "M0.3 1.5 Q1.3 0 3 0 Q5.8 0 5.8 2.4 Q5.8 4.8 2.5 4.8 Q6.2 4.8 6.2 7.4 Q6.2 10 3 10 Q1 10 0 8.5",
            6.2,
        ),
        '4' => ("M5 10 L5 0 L0 7 L6.5 7", 6.5),
        '5' => ("M6 0 L1 0 L0.5 4.5 Q1.5 3.8 3 3.8 Q6.2 3.8 6.2 6.9 Q6.2 10 3 10 Q1 10 0 8.7", 6.2),
        '6' => ("M5.5 0.8 Q4.5 0 3.2 0 Q0 0 0 6 Q0 10 3 10 Q6 10 6 7 Q6 4.2 3 4.2 Q0.8 4.2 0 6", 6.),
        '7' => ("M0 0 L6 0 L2 10", 6.),
        '8' => (
            "M3 4.6 Q0.4 4.6 0.4 2.3 Q0.4 0 3 0 Q5.6 0 5.6 2.3 Q5.6 4.6 3 4.6 Q0 4.6 0 7.3 Q0 10 3 10 Q6 10 6 7.3 Q6 4.6 3 4.6",
            6.,
        ),
        '9' => ("M6 4 Q6 5.8 3 5.8 Q0 5.8 0 2.9 Q0 0 3 0 Q6 0 6 4 Q6 10 2.8 10 Q1.3 10 0.5 9.2", 6.),

        '.' => ("M0.5 9.5 L0.5 10", 1.),
        ',' => ("M0.8 9.4 L0.1 11.6", 1.),
        ';' => ("M0.8 4.6 L0.8 5.1 M0.8 9.4 L0.1 11.6", 1.),
        ':' => ("M0.5 4.6 L0.5 5.1 M0.5 9.5 L0.5 10", 1.),
        '!' => ("M0.5 0 L0.5 7.2 M0.5 9.5 L0.5 10", 1.),
        '?' => ("M0 2.2 Q0.5 0 2.8 0 Q5.3 0 5.3 2.3 Q5.3 4 3 5 Q2.5 5.3 2.5 6.8 M2.5 9.5 L2.5 10", 5.3),
        '-' => ("M0 6.5 L4 6.5", 4.),
        '\'' => ("M0.5 0 L0.3 2.5", 0.8),
        '"' => ("M0.5 0 L0.3 2.5 M2 0 L1.8 2.5", 2.3),
        '(' => ("M2.5 -0.5 Q0 2.5 0 5.2 Q0 8 2.5 11", 2.5),
        ')' => ("M0 -0.5 Q2.5 2.5 2.5 5.2 Q2.5 8 0 11", 2.5),
        '/' => ("M5 0 L0 10", 5.),
        '+' => ("M3 3.5 L3 9.5 M0 6.5 L6 6.5", 6.),
        '=' => ("M0 5.3 L5 5.3 M0 7.7 L5 7.7", 5.),
        '*' => ("M2.5 2 L2.5 7 M0.3 3.3 L4.7 5.7 M4.7 3.3 L0.3 5.7", 5.),
        '%' => ("M6 0 L0 10 M2.4 1.6 A1.2 1.2 0 0 0 0 1.6 A1.2 1.2 0 0 0 2.4 1.6 M6 8.4 A1.2 1.2 0 0 0 3.6 8.4 A1.2 1.2 0 0 0 6 8.4", 6.),
        '&' => ("M6.5 10 L1.5 3.5 Q0.8 2.5 0.8 1.8 Q0.8 0 2.6 0 Q4.4 0 4.4 1.8 Q4.4 3.6 1.8 5.4 Q0 6.6 0 8 Q0 10 2.5 10 Q4.6 10 6.5 6.5", 6.5),
        '#' => ("M2 0.5 L1 9.5 M5 0.5 L4 9.5 M0 3.5 L6 3.5 M0 6.5 L6 6.5", 6.),
        '@' => ("M6 6.5 A2 2 0 1 0 6 6.6 L6 8 Q6 9 7 9 Q8.5 9 8.5 6 Q8.5 1.5 4.5 1.5 Q0 1.5 0 6 Q0 10.5 4.5 10.5 Q6 10.5 7 10", 8.5),
        '→' => ("M0 6.5 L8.5 6.5 M5.2 3.6 L8.5 6.5 L5.2 9.4", 8.5),
        '←' => ("M8.5 6.5 L0 6.5 M3.3 3.6 L0 6.5 L3.3 9.4", 8.5),
        '•' => ("M1 6 L1.6 6.6 M1.6 6 L1 6.6", 2.6),
        '°' => ("M2.4 1.4 A1.2 1.2 0 0 0 0 1.4 A1.2 1.2 0 0 0 2.4 1.4", 2.4),
        '€' => ("M7 1.5 Q6 0 4.5 0 Q1 0 1 5 Q1 10 4.5 10 Q6 10 7 8.5 M0 4 L5 4 M0 6 L5 6", 7.),
        'ß' => ("M0 10 L0 2.5 Q0 0 2.8 0 Q5.3 0 5.3 2.3 Q5.3 4.4 3 4.6 Q6 4.8 6 7.4 Q6 10 3 10 L2.2 10", 6.),
        _ => return None,
    })
}

/// Common typographic characters mapped to ones the font draws.
fn normalise(c: char) -> char {
    match c {
        '\u{2018}' | '\u{2019}' | '`' | '´' => '\'',
        '\u{201C}' | '\u{201D}' | '„' | '«' | '»' => '"',
        '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
        '\u{00A0}' | '\t' => ' ',
        '·' => '•',
        _ => c,
    }
}

/// Glyph strokes (in glyph units, origin at the glyph's top-left on the capital line) and
/// advance for one character, including umlauts built from their base letter plus two dots.
fn strokes(c: char) -> Option<(Vec<Poly>, f32)> {
    let (base, dots_y) = match c {
        'ä' => ('a', 0.),
        'ö' => ('o', 0.),
        'ü' => ('u', 0.),
        'Ä' => ('A', -3.0),
        'Ö' => ('O', -3.0),
        'Ü' => ('U', -3.0),
        _ => (c, f32::NAN),
    };
    let (d, w) = glyph(base)?;
    let mut polys = path_polylines(d, SEG).ok()?;
    if !dots_y.is_nan() {
        let off = (w - 6.) / 2.;
        for p in path_polylines(DOTS_LOW, SEG).ok()? {
            polys.push(p.into_iter().map(|(x, y)| (x + off, y + dots_y)).collect());
        }
    }
    Some((polys, w))
}

/// Whether the glyph has a tall upright stroke on its left and on its right edge.
fn stems(polys: &[Poly], w: f32) -> (bool, bool) {
    let (mut left, mut right) = (false, false);
    for p in polys {
        for s in p.windows(2) {
            let ((x0, y0), (x1, y1)) = (s[0], s[1]);
            if (x0 - x1).abs() > 0.3 || (y0 - y1).abs() < 3.5 {
                continue;
            }
            left |= x0.max(x1) < 0.9;
            right |= x0.min(x1) > w - 0.9;
        }
    }
    (left, right)
}

/// Space before `next`, given the letter before it.
fn gap(prev: Option<char>, next: char) -> f32 {
    let Some(prev) = prev else { return 0. };
    let edge = |c: char| strokes(c).map(|(p, w)| stems(&p, w));
    match (edge(prev), edge(next)) {
        (Some((_, true)), Some((true, _))) => GAP + STEM_GAP,
        _ => GAP,
    }
}

/// Characters [`layout`] cannot draw (after normalisation), in order of appearance, deduplicated.
pub fn missing(text: &str) -> Vec<char> {
    let mut out = Vec::new();
    for c in text.chars().map(normalise) {
        if c != ' ' && c != '\n' && c != '\r' && strokes(c).is_none() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

fn word_width(word: &str) -> f32 {
    let mut w = 0.;
    let mut prev = None;
    for c in word.chars().map(normalise) {
        w += gap(prev, c) + strokes(c).map(|s| s.1).unwrap_or(0.);
        prev = Some(c);
    }
    w
}

/// Lines after wrapping, in glyph units: each line is its words.
fn wrap(text: &str, max_units: Option<f32>) -> Vec<Vec<String>> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line: Vec<String> = Vec::new();
        let mut width = 0.;
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let ww = word_width(word);
            let extra = if line.is_empty() { ww } else { SPACE + ww };
            if let Some(max) = max_units {
                if !line.is_empty() && width + extra > max {
                    lines.push(std::mem::take(&mut line));
                    width = 0.;
                    line.push(word.to_string());
                    width += ww;
                    continue;
                }
            }
            line.push(word.to_string());
            width += extra;
        }
        lines.push(line);
    }
    lines
}

/// Width and height the text takes at capital height `size`, wrapped to `width` if given, in
/// the caller's units. Height runs from the top of the capitals to the lowest descender.
pub fn measure(text: &str, size: f32, width: Option<f32>) -> (f32, f32) {
    let s = size / CAP;
    let lines = wrap(text, width.map(|w| w / s));
    let w = lines
        .iter()
        .map(|l| {
            l.iter().map(|x| word_width(x)).sum::<f32>() + SPACE * l.len().saturating_sub(1) as f32
        })
        .fold(0f32, f32::max);
    let h = (lines.len().max(1) - 1) as f32 * LINE + 14.;
    (w * s, h * s)
}

/// Lay out `text` as handwriting polylines. `(x, y)` is the top-left of the first line's
/// capitals, `size` the capital height and `width` (optional) the wrap width, all in the
/// caller's units. Characters the font lacks are an error listing them, never dropped.
pub fn layout(
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    width: Option<f32>,
) -> Result<Vec<Poly>, String> {
    if !(size.is_finite() && size > 0.) {
        return Err(format!("writing size {size:?} must be a positive number"));
    }
    let gone = missing(text);
    if !gone.is_empty() {
        let list: String = gone.iter().collect();
        return Err(format!(
            "writing uses characters the handwriting font lacks: {list:?}"
        ));
    }
    let s = size / CAP;
    let mut out = Vec::new();
    for (li, line) in wrap(text, width.map(|w| w / s)).iter().enumerate() {
        let top = li as f32 * LINE;
        let mut pen = 0.;
        for (wi, word) in line.iter().enumerate() {
            if wi > 0 {
                pen += SPACE;
            }
            let mut prev = None;
            for c in word.chars().map(normalise) {
                pen += gap(prev, c);
                prev = Some(c);
                let (polys, adv) = strokes(c).expect("checked by missing()");
                for p in polys {
                    out.push(
                        p.into_iter()
                            .map(|(gx, gy)| {
                                let lean = (CAP - gy) * SLANT;
                                (x + (pen + gx + lean) * s, y + (top + gy) * s)
                            })
                            .collect(),
                    );
                }
                pen += adv;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_parses_and_stays_in_its_box() {
        let all = ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .chain(".,;:!?-'\"()/+=*%&#@→←•°€ßäöüÄÖÜ".chars());
        for c in all {
            let (polys, w) = strokes(c).unwrap_or_else(|| panic!("no glyph for {c:?}"));
            assert!(!polys.is_empty(), "{c:?}");
            for p in &polys {
                assert!(p.len() >= 2, "{c:?}");
                for &(x, y) in p {
                    assert!((-0.6..=w + 1.0).contains(&x), "{c:?} x={x}");
                    assert!((-3.6..=14.1).contains(&y), "{c:?} y={y}");
                }
            }
        }
    }

    #[test]
    fn lowercase_is_its_own_shape_and_sits_on_the_baseline() {
        let lo = layout("o", 0., 0., 10., None).unwrap();
        let up = layout("O", 0., 0., 10., None).unwrap();
        assert_ne!(lo, up);
        let ys: Vec<f32> = lo.iter().flatten().map(|p| p.1).collect();
        let (min, max) = ys
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
        assert!(
            (min - 4.).abs() < 0.05 && (max - 10.).abs() < 0.05,
            "{min} {max}"
        );
    }

    #[test]
    fn wraps_words_and_honours_newlines() {
        let one = measure("hello world", 10., None);
        let wrapped = measure("hello world", 10., Some(one.0 * 0.7));
        assert!((wrapped.1 - (LINE + 14.)).abs() < 1e-3, "{wrapped:?}");
        assert!(wrapped.0 < one.0);
        assert_eq!(measure("a\nb\nc", 10., None).1, 2. * LINE + 14.);
        // Every stroke of the wrapped layout fits the wrap width (plus the slant).
        let w = one.0 * 0.7;
        for p in layout("hello world", 0., 0., 10., Some(w)).unwrap() {
            for (x, _) in p {
                assert!(x <= w + CAP * SLANT + 0.01, "{x} > {w}");
            }
        }
    }

    #[test]
    fn typographic_characters_are_mapped_and_unknown_ones_reported() {
        assert!(layout(
            "it\u{2019}s \u{201C}fine\u{201D} \u{2014} ok",
            0.,
            0.,
            1.,
            None
        )
        .is_ok());
        let e = layout("snow ☃ and 漢", 0., 0., 1., None).unwrap_err();
        assert!(e.contains('☃') && e.contains('漢'), "{e}");
        assert!(layout("a", 0., 0., 0., None).is_err());
    }

    #[test]
    fn upright_neighbours_get_extra_space() {
        let e = |c| {
            let (p, w) = strokes(c).unwrap();
            stems(&p, w)
        };
        assert_eq!(e('u'), (true, true));
        assert_eq!(e('b'), (true, false));
        assert_eq!(e('o'), (false, false));
        assert_eq!(gap(Some('u'), 'b'), GAP + STEM_GAP);
        assert_eq!(gap(Some('o'), 'b'), GAP);
        assert_eq!(gap(None, 'b'), 0.);
        assert!(word_width("ub") > word_width("ob"));
    }

    #[test]
    fn scales_and_offsets() {
        let a = layout("l", 100., 50., 20., None).unwrap();
        // 'l' starts at glyph (0.8, 0): slanted by 10*0.12, scaled by 2, offset.
        assert!((a[0][0].0 - (100. + (0.8 + 1.2) * 2.)).abs() < 1e-3);
        assert!((a[0][0].1 - 50.).abs() < 1e-3);
    }
}
