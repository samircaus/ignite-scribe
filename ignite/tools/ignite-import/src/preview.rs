//! SVG preview of a plan, so a spec can be checked without a server or a Kindle.
//! Approximates the Kindle raster (round 5 px-wide lines on the 15624 × 20832 page); text
//! objects are drawn as plain SVG text and are only indicative.

use std::fmt::Write;

use sync_protocol::{CANONICAL_PAGE_HEIGHT, CANONICAL_PAGE_WIDTH};

use crate::build::PagePreview;
use crate::paper::SPACING_PX;
use crate::spec::Paper;

/// Canonical units per panel pixel.
const PX: f32 = CANONICAL_PAGE_WIDTH / 1860.0;

pub fn page_svg(page: &PagePreview) -> String {
    let (w, h) = (CANONICAL_PAGE_WIDTH, CANONICAL_PAGE_HEIGHT);
    let mut s = String::new();
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{}\" height=\"{}\">\n<rect width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>\n",
        w / 20.0,
        h / 20.0
    );
    let step = SPACING_PX as f32 * PX;
    match page.paper {
        Paper::Blank => {}
        Paper::Lined => {
            let mut y = 200.0 * PX;
            while y < h {
                let _ = writeln!(s, "<line x1=\"0\" x2=\"{w}\" y1=\"{y}\" y2=\"{y}\" stroke=\"#aaa\" stroke-width=\"{}\"/>", 2.0 * PX);
                y += step;
            }
        }
        Paper::Grid => {
            let mut y = (200 % SPACING_PX) as f32 * PX;
            while y < h {
                let _ = writeln!(s, "<line x1=\"0\" x2=\"{w}\" y1=\"{y}\" y2=\"{y}\" stroke=\"#ddd\" stroke-width=\"{}\"/>", 2.0 * PX);
                y += step;
            }
            let mut x = step / 2.0;
            while x < w {
                let _ = writeln!(s, "<line y1=\"0\" y2=\"{h}\" x1=\"{x}\" x2=\"{x}\" stroke=\"#ddd\" stroke-width=\"{}\"/>", 2.0 * PX);
                x += step;
            }
        }
        Paper::Dot => {
            let mut y = (200 % SPACING_PX + SPACING_PX / 2) as f32 * PX;
            while y < h {
                let mut x = step / 2.0;
                while x < w {
                    let _ = writeln!(
                        s,
                        "<circle cx=\"{x}\" cy=\"{y}\" r=\"{}\" fill=\"#888\"/>",
                        2.0 * PX
                    );
                    x += step;
                }
                y += step;
            }
        }
    }
    let _ = writeln!(
        s,
        "<g fill=\"none\" stroke=\"#000\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">",
        5.0 * PX
    );
    for st in &page.strokes {
        if st.len() == 1 {
            let _ = writeln!(
                s,
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#000\" stroke=\"none\"/>",
                st[0].0,
                st[0].1,
                2.5 * PX
            );
            continue;
        }
        let pts: Vec<String> = st.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
        let _ = writeln!(s, "<polyline points=\"{}\"/>", pts.join(" "));
    }
    s.push_str("</g>\n");
    for t in &page.texts {
        let _ = writeln!(
            s,
            "<text x=\"{}\" y=\"{}\" font-size=\"{}\" font-family=\"sans-serif\" fill=\"#555\">{}</text>",
            t.x,
            t.y + t.font_size,
            t.font_size,
            esc(&t.content.replace('\n', " "))
        );
    }
    s.push_str("</svg>\n");
    s
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
