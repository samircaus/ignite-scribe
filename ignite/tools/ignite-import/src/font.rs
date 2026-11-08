//! A tiny built-in **single-line stroke font** (original glyph data, no third-party font).
//!
//! Each glyph is a few polylines on an 8-wide × 12-tall grid, origin top-left, y down. Because
//! the glyphs are centre-line strokes, headings drawn with it become ordinary ink on the Kindle:
//! erasable, lasso-movable and undoable, like handwriting. Lowercase maps to uppercase.

type Glyph = &'static [&'static [(f32, f32)]];

const GLYPH_H: f32 = 12.0;
const ADVANCE: f32 = 11.0;
const SPACE: f32 = 6.0;

fn glyph(c: char) -> Option<Glyph> {
    Some(match c.to_ascii_uppercase() {
        'A' => &[&[(0., 12.), (4., 0.), (8., 12.)], &[(1.4, 8.), (6.6, 8.)]],
        'B' => &[
            &[
                (0., 12.),
                (0., 0.),
                (5., 0.),
                (7.5, 1.5),
                (7.5, 4.5),
                (5., 6.),
                (0., 6.),
            ],
            &[(5., 6.), (8., 7.5), (8., 10.5), (5.5, 12.), (0., 12.)],
        ],
        'C' => &[&[
            (8., 2.),
            (6., 0.),
            (2., 0.),
            (0., 2.),
            (0., 10.),
            (2., 12.),
            (6., 12.),
            (8., 10.),
        ]],
        'D' => &[&[
            (0., 0.),
            (0., 12.),
            (5., 12.),
            (8., 9.),
            (8., 3.),
            (5., 0.),
            (0., 0.),
        ]],
        'E' => &[
            &[(8., 0.), (0., 0.), (0., 12.), (8., 12.)],
            &[(0., 6.), (6., 6.)],
        ],
        'F' => &[&[(8., 0.), (0., 0.), (0., 12.)], &[(0., 6.), (6., 6.)]],
        'G' => &[&[
            (8., 2.),
            (6., 0.),
            (2., 0.),
            (0., 2.),
            (0., 10.),
            (2., 12.),
            (6., 12.),
            (8., 10.),
            (8., 7.),
            (4., 7.),
        ]],
        'H' => &[
            &[(0., 0.), (0., 12.)],
            &[(8., 0.), (8., 12.)],
            &[(0., 6.), (8., 6.)],
        ],
        'I' => &[
            &[(1., 0.), (7., 0.)],
            &[(4., 0.), (4., 12.)],
            &[(1., 12.), (7., 12.)],
        ],
        'J' => &[&[(8., 0.), (8., 10.), (6., 12.), (2., 12.), (0., 10.)]],
        'K' => &[
            &[(0., 0.), (0., 12.)],
            &[(8., 0.), (0., 7.)],
            &[(2.5, 5.2), (8., 12.)],
        ],
        'L' => &[&[(0., 0.), (0., 12.), (8., 12.)]],
        'M' => &[&[(0., 12.), (0., 0.), (4., 7.), (8., 0.), (8., 12.)]],
        'N' => &[&[(0., 12.), (0., 0.), (8., 12.), (8., 0.)]],
        'O' | '0' => &[&[
            (2., 0.),
            (6., 0.),
            (8., 2.),
            (8., 10.),
            (6., 12.),
            (2., 12.),
            (0., 10.),
            (0., 2.),
            (2., 0.),
        ]],
        'P' => &[&[
            (0., 12.),
            (0., 0.),
            (5.5, 0.),
            (8., 2.),
            (8., 5.),
            (5.5, 7.),
            (0., 7.),
        ]],
        'Q' => &[
            &[
                (2., 0.),
                (6., 0.),
                (8., 2.),
                (8., 10.),
                (6., 12.),
                (2., 12.),
                (0., 10.),
                (0., 2.),
                (2., 0.),
            ],
            &[(5., 9.), (8.5, 12.5)],
        ],
        'R' => &[
            &[
                (0., 12.),
                (0., 0.),
                (5.5, 0.),
                (8., 2.),
                (8., 5.),
                (5.5, 7.),
                (0., 7.),
            ],
            &[(4., 7.), (8., 12.)],
        ],
        'S' => &[&[
            (8., 2.),
            (6., 0.),
            (2., 0.),
            (0., 2.),
            (0., 4.5),
            (2., 6.),
            (6., 6.),
            (8., 7.5),
            (8., 10.),
            (6., 12.),
            (2., 12.),
            (0., 10.),
        ]],
        'T' => &[&[(0., 0.), (8., 0.)], &[(4., 0.), (4., 12.)]],
        'U' => &[&[
            (0., 0.),
            (0., 10.),
            (2., 12.),
            (6., 12.),
            (8., 10.),
            (8., 0.),
        ]],
        'V' => &[&[(0., 0.), (4., 12.), (8., 0.)]],
        'W' => &[&[(0., 0.), (2., 12.), (4., 5.), (6., 12.), (8., 0.)]],
        'X' => &[&[(0., 0.), (8., 12.)], &[(8., 0.), (0., 12.)]],
        'Y' => &[&[(0., 0.), (4., 6.), (8., 0.)], &[(4., 6.), (4., 12.)]],
        'Z' => &[&[(0., 0.), (8., 0.), (0., 12.), (8., 12.)]],
        '1' => &[
            &[(1.5, 2.5), (4., 0.), (4., 12.)],
            &[(1.5, 12.), (6.5, 12.)],
        ],
        '2' => &[&[
            (0., 2.),
            (2., 0.),
            (6., 0.),
            (8., 2.),
            (8., 4.5),
            (0., 12.),
            (8., 12.),
        ]],
        '3' => &[
            &[
                (0., 1.5),
                (2., 0.),
                (6., 0.),
                (8., 2.),
                (8., 4.5),
                (6., 6.),
                (3., 6.),
            ],
            &[
                (6., 6.),
                (8., 7.5),
                (8., 10.),
                (6., 12.),
                (2., 12.),
                (0., 10.5),
            ],
        ],
        '4' => &[&[(6., 12.), (6., 0.), (0., 8.5), (8., 8.5)]],
        '5' => &[&[
            (8., 0.),
            (0., 0.),
            (0., 5.5),
            (5., 5.),
            (8., 7.),
            (8., 10.),
            (6., 12.),
            (2., 12.),
            (0., 10.5),
        ]],
        '6' => &[&[
            (7., 1.),
            (5., 0.),
            (2.5, 0.),
            (0., 3.),
            (0., 10.),
            (2., 12.),
            (6., 12.),
            (8., 10.),
            (8., 7.5),
            (6., 5.5),
            (2., 5.5),
            (0., 7.5),
        ]],
        '7' => &[&[(0., 0.), (8., 0.), (3., 12.)]],
        '8' => &[
            &[
                (2., 0.),
                (6., 0.),
                (8., 2.),
                (8., 4.),
                (6., 6.),
                (2., 6.),
                (0., 4.),
                (0., 2.),
                (2., 0.),
            ],
            &[
                (2., 6.),
                (0., 8.),
                (0., 10.),
                (2., 12.),
                (6., 12.),
                (8., 10.),
                (8., 8.),
                (6., 6.),
            ],
        ],
        '9' => &[&[
            (8., 4.5),
            (6., 6.5),
            (2., 6.5),
            (0., 4.5),
            (0., 2.),
            (2., 0.),
            (6., 0.),
            (8., 2.),
            (8., 10.),
            (6., 12.),
            (3., 12.),
            (1., 11.),
        ]],
        '.' => &[&[(3.6, 11.0), (4.4, 12.0)]],
        ',' => &[&[(4., 11.), (3., 13.5)]],
        '-' => &[&[(1., 6.), (7., 6.)]],
        ':' => &[&[(4., 3.5), (4.1, 4.5)], &[(4., 9.), (4.1, 10.)]],
        '!' => &[&[(4., 0.), (4., 8.)], &[(4., 11.), (4.1, 12.)]],
        '?' => &[
            &[
                (0., 2.),
                (2., 0.),
                (6., 0.),
                (8., 2.),
                (8., 4.),
                (4., 7.),
                (4., 8.5),
            ],
            &[(4., 11.), (4.1, 12.)],
        ],
        '\'' => &[&[(4., 0.), (3.5, 3.)]],
        '/' => &[&[(8., 0.), (0., 12.)]],
        '(' => &[&[(6., 0.), (3., 3.), (3., 9.), (6., 12.)]],
        ')' => &[&[(2., 0.), (5., 3.), (5., 9.), (2., 12.)]],
        '+' => &[&[(4., 3.), (4., 9.)], &[(1., 6.), (7., 6.)]],
        _ => return None,
    })
}

/// Lay `text` out as polylines. `(x, y)` is the top-left of the first capital letter and
/// `cap_height` the capital height, all in the caller's units. Control characters and glyphs
/// the font lacks are an error (listed), never silently dropped.
pub fn layout(text: &str, x: f32, y: f32, cap_height: f32) -> Result<Vec<Vec<(f32, f32)>>, String> {
    if !(cap_height.is_finite() && cap_height > 0.0) {
        return Err(format!(
            "label size {cap_height:?} must be a positive number"
        ));
    }
    let s = cap_height / GLYPH_H;
    let mut out = Vec::new();
    let mut pen = x;
    let mut missing = Vec::new();
    for c in text.chars() {
        if c == ' ' {
            pen += SPACE * s;
            continue;
        }
        match glyph(c) {
            Some(strokes) => {
                for poly in strokes {
                    out.push(
                        poly.iter()
                            .map(|&(gx, gy)| (pen + gx * s, y + gy * s))
                            .collect(),
                    );
                }
                pen += ADVANCE * s;
            }
            None => missing.push(c),
        }
    }
    if !missing.is_empty() {
        let list: String = missing.iter().collect();
        return Err(format!(
            "label {text:?} uses characters the built-in stroke font lacks: {list:?}"
        ));
    }
    Ok(out)
}

/// Width of `text` at `cap_height` (for centring/wrapping in generators).
pub fn text_width(text: &str, cap_height: f32) -> f32 {
    let s = cap_height / GLYPH_H;
    text.chars()
        .map(|c| if c == ' ' { SPACE } else { ADVANCE })
        .sum::<f32>()
        * s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_glyph_has_two_plus_point_strokes() {
        for c in ('A'..='Z').chain('0'..='9').chain(".,-:!?'/()+".chars()) {
            let g = glyph(c).unwrap_or_else(|| panic!("no glyph for {c:?}"));
            assert!(!g.is_empty(), "{c:?}");
            for poly in g {
                assert!(poly.len() >= 2, "{c:?} has a degenerate stroke");
            }
        }
    }

    #[test]
    fn lowercase_maps_to_uppercase_and_scales() {
        let lo = layout("a", 0.0, 0.0, 12.0).unwrap();
        let up = layout("A", 0.0, 0.0, 12.0).unwrap();
        assert_eq!(lo, up);
        let half = layout("A", 0.0, 0.0, 6.0).unwrap();
        assert_eq!(half[0][1], (2.0, 0.0));
    }

    #[test]
    fn advance_and_origin() {
        let l = layout("II", 100.0, 50.0, 12.0).unwrap();
        // 'I' has three strokes; the second letter starts one advance (11) to the right.
        assert_eq!(l.len(), 6);
        assert_eq!(l[3][0], (101.0 + 11.0, 50.0));
    }

    #[test]
    fn unsupported_characters_are_reported_not_dropped() {
        let e = layout("HI @#", 0.0, 0.0, 1.0).unwrap_err();
        assert!(e.contains("@#"), "{e}");
        assert!(layout("A", 0.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn width_matches_layout() {
        assert_eq!(text_width("A B", 12.0), 11.0 + 6.0 + 11.0);
    }
}
