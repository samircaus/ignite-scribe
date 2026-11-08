//! Gate F11 notebook chrome: live framebuffer controls only.
//!
//! Two reserved panel bands (top and bottom). A pen contact that *starts* inside a band is UI
//! and never begins a stroke. Stored stroke geometry is not remapped or shifted.
//!
//! Chrome is painted after background + strokes and is never part of `background.raw`,
//! `strokes.jsonl`, or `edits.jsonl`.

use std::cell::Cell;

use kindle_platform::{DirtyRegion, FbGeometry};

use crate::notebook::{page_index, PersistedNotebookManifestV1};
use crate::ui_gfx::{Canvas, Rect, UiPoint, BLACK, WHITE};

pub const TOP_BAND_H: u32 = 130;
pub const BOTTOM_BAND_H: u32 = 120;
const RULE_H: u32 = 4;

thread_local! {
    /// Transient on-screen Erase mode (never persisted). The UI thread owns it: it is flipped by
    /// the Erase chrome button, read when chrome is painted and when a pen Down is routed.
    static ERASE_MODE: Cell<bool> = const { Cell::new(false) };
}

/// Whether the pen tip currently acts as the eraser (on-screen Erase mode).
pub fn erase_mode() -> bool {
    ERASE_MODE.with(Cell::get)
}

pub fn set_erase_mode(on: bool) {
    ERASE_MODE.with(|m| m.set(on));
}

/// Positional page state painted in the bottom band (1-based `index`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageChrome {
    pub index: usize,
    pub count: usize,
}

impl PageChrome {
    pub fn label(&self) -> String {
        format!("{} of {}", self.index, self.count)
    }

    pub fn can_previous(&self) -> bool {
        self.index > 1
    }
}

/// Fail-closed: unknown `page_id` never silently becomes page 1.
pub fn page_chrome_for(
    manifest: &PersistedNotebookManifestV1,
    page_id: &str,
) -> Result<PageChrome, String> {
    let Some(i) = page_index(manifest, page_id) else {
        return Err(format!(
            "REFUSE PAGE CHROME: manifest does not contain requested page id {page_id}"
        ));
    };
    Ok(PageChrome {
        index: i + 1,
        count: manifest.page_ids.len(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeHit {
    Home,
    Undo,
    /// The Eraser half of the Pen | Eraser switch (turns Erase mode on).
    Erase,
    /// The Pen half of the Pen | Eraser switch (turns Erase mode off).
    Pen,
    PageMenu,
    PreviousPage,
    NextPage,
    /// Inside a reserved band but not on a control: consumed, no action.
    Inert,
}

/// Modern chrome (2026-11 redesign): one slim bar per band, Source Sans labels, drawn icons.
/// Top: "‹ Notebooks" · title + "Page n of m" · Pen | Eraser · Undo · menu.
/// Bottom: "‹ Page n-1" · page dots · "Page n+1 ›" (or "+ New page" on the last page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromeLayout {
    pub top_band: Rect,
    pub bottom_band: Rect,
    /// "‹ Notebooks" (back to Home).
    pub home: Rect,
    pub title: Rect,
    pub undo: Rect,
    /// Pen | Eraser switch: outer frame, Pen half, Eraser half.
    pub tool: Rect,
    pub pen: Rect,
    pub erase: Rect,
    pub menu: Rect,
    pub previous: Rect,
    pub next: Rect,
    pub page_label: Rect,
}

const CONTROL_H: u32 = 92;
const PILL_H: u32 = 88;
const SEGMENT_W: u32 = 112;
const ICON_W: u32 = 104;
const LABEL_PX: f32 = 34.0;
const TITLE_PX: f32 = 42.0;
const SUBTITLE_PX: f32 = 28.0;
const DARK_GRAY: u8 = 0x55;

impl ChromeLayout {
    pub fn new(geo: &FbGeometry) -> Self {
        let w = geo.xres;
        let top = Rect::new(0, 0, w, TOP_BAND_H);
        let bottom_y = geo.yres.saturating_sub(BOTTOM_BAND_H);
        let bottom = Rect::new(0, bottom_y, w, BOTTOM_BAND_H);
        let cy = (TOP_BAND_H - RULE_H).saturating_sub(CONTROL_H) / 2;
        let home = Rect::new(16, cy, 300, CONTROL_H);
        let menu = Rect::new(w.saturating_sub(16 + ICON_W), cy, ICON_W, CONTROL_H);
        let undo = Rect::new(menu.x.saturating_sub(8 + ICON_W), cy, ICON_W, CONTROL_H);
        let tool = Rect::new(
            undo.x.saturating_sub(24 + 2 * SEGMENT_W),
            cy,
            2 * SEGMENT_W,
            CONTROL_H,
        );
        let pen = Rect::new(tool.x, cy, SEGMENT_W, CONTROL_H);
        let erase = Rect::new(tool.x + SEGMENT_W, cy, SEGMENT_W, CONTROL_H);
        let title_x = home.right() + 24;
        let title = Rect::new(title_x, cy, tool.x.saturating_sub(24 + title_x), CONTROL_H);
        let py = bottom_y + RULE_H + (BOTTOM_BAND_H - RULE_H).saturating_sub(PILL_H) / 2;
        let previous = Rect::new(16, py, 330, PILL_H);
        let next = Rect::new(w.saturating_sub(16 + 330), py, 330, PILL_H);
        let page_label = Rect::new(
            previous.right() + 24,
            py,
            next.x.saturating_sub(previous.right() + 48),
            PILL_H,
        );
        Self {
            top_band: top,
            bottom_band: bottom,
            home,
            title,
            undo,
            tool,
            pen,
            erase,
            menu,
            previous,
            next,
            page_label,
        }
    }

    /// `None` = normal page content (ink engine owns it).
    pub fn hit(&self, p: UiPoint) -> Option<ChromeHit> {
        let in_top = self.top_band.contains(p.x, p.y);
        let in_bottom = self.bottom_band.contains(p.x, p.y);
        if !in_top && !in_bottom {
            return None;
        }
        let hit = [
            (self.home, ChromeHit::Home),
            (self.undo, ChromeHit::Undo),
            (self.pen, ChromeHit::Pen),
            (self.erase, ChromeHit::Erase),
            (self.menu, ChromeHit::PageMenu),
            (self.previous, ChromeHit::PreviousPage),
            (self.next, ChromeHit::NextPage),
        ]
        .into_iter()
        .find(|(r, _)| r.contains(p.x, p.y))
        .map(|(_, h)| h)
        .unwrap_or(ChromeHit::Inert);
        Some(hit)
    }

    /// Whether a live-ink dirty region overlaps a band (chrome needs repainting).
    pub fn damaged_by(&self, dirty: DirtyRegion) -> bool {
        if dirty.x < 0 || dirty.y < 0 {
            return true;
        }
        let r = Rect::new(dirty.x as u32, dirty.y as u32, dirty.width, dirty.height);
        r.intersects(&self.top_band) || r.intersects(&self.bottom_band)
    }
}

fn chevron(c: &mut Canvas<'_>, cx: f32, cy: f32, left: bool, ink: u8) {
    let d = if left { 1.0 } else { -1.0 };
    c.polyline(
        &[
            (cx + 9.0 * d, cy - 17.0),
            (cx - 8.0 * d, cy),
            (cx + 9.0 * d, cy + 17.0),
        ],
        6.0,
        ink,
    );
}

fn pen_icon(c: &mut Canvas<'_>, r: Rect, ink: u8) {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    // Pencil body along the diagonal, a cap line and the tip.
    c.line(cx - 15.0, cy + 15.0, cx + 13.0, cy - 13.0, 11.0, ink);
    c.line(cx + 6.0, cy - 20.0, cx + 20.0, cy - 6.0, 4.0, inverse(ink));
    c.disc(cx - 20.0, cy + 20.0, 4.0, ink);
}

fn eraser_icon(c: &mut Canvas<'_>, r: Rect, ink: u8) {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    // A tilted eraser block (outline) on a base line.
    let p = [
        (cx - 20.0, cy + 6.0),
        (cx - 2.0, cy - 12.0),
        (cx + 14.0, cy + 4.0),
        (cx - 4.0, cy + 22.0),
        (cx - 20.0, cy + 6.0),
    ];
    c.polyline(&p, 4.5, ink);
    c.line(cx - 11.0, cy - 3.0, cx + 5.0, cy + 13.0, 4.5, ink);
    c.line(cx - 2.0, cy + 22.0, cx + 22.0, cy + 22.0, 4.5, ink);
}

fn undo_icon(c: &mut Canvas<'_>, r: Rect, ink: u8) {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    // Hooked arrow: arc from the right back to the left, arrow head on the left.
    let mut arc = Vec::new();
    for i in 0..=16 {
        let a = std::f32::consts::PI * (i as f32 / 16.0) - std::f32::consts::FRAC_PI_2;
        arc.push((cx + 2.0 + 16.0 * a.cos(), cy + 2.0 + 14.0 * a.sin()));
    }
    let mut path = vec![(cx - 18.0, cy - 12.0)];
    path.extend(arc);
    path.push((cx - 6.0, cy + 16.0));
    c.polyline(&path, 5.0, ink);
    c.polyline(
        &[
            (cx - 8.0, cy - 23.0),
            (cx - 19.0, cy - 12.0),
            (cx - 8.0, cy - 1.0),
        ],
        5.0,
        ink,
    );
}

fn inverse(ink: u8) -> u8 {
    if ink == BLACK {
        WHITE
    } else {
        BLACK
    }
}

/// Up to this many pages show as dots; more show "n / m".
const MAX_DOTS: usize = 12;

/// Paint both chrome bands over the current framebuffer contents. Rows outside the bands are
/// never touched, so page content in the writable area is preserved exactly.
pub fn render_notebook_chrome(
    fb: &mut [u8],
    geo: &FbGeometry,
    title: &str,
    page: PageChrome,
) -> Result<ChromeLayout, String> {
    let layout = ChromeLayout::new(geo);
    let mut c = Canvas::new(fb, geo)?;
    c.fill(layout.top_band, WHITE);
    c.fill(layout.bottom_band, WHITE);
    c.fill(Rect::new(0, TOP_BAND_H - RULE_H, geo.xres, RULE_H), BLACK);
    c.fill(Rect::new(0, layout.bottom_band.y, geo.xres, RULE_H), BLACK);

    // ‹ Notebooks
    let h = layout.home;
    chevron(
        &mut c,
        h.x as f32 + 24.0,
        (h.y + h.h / 2) as f32,
        true,
        BLACK,
    );
    let label_w = Canvas::sans_width(LABEL_PX, "Notebooks", true) + 2;
    c.sans_centered(
        Rect::new(h.x + 50, h.y, label_w, h.h),
        LABEL_PX,
        "Notebooks",
        BLACK,
        true,
    );

    // Title and page position.
    let t = layout.title;
    c.sans_centered(
        Rect::new(t.x, t.y + 4, t.w, 50),
        TITLE_PX,
        title,
        BLACK,
        true,
    );
    let sub = if erase_mode() {
        "Eraser on · tap the pen to write".to_string()
    } else {
        format!("Page {} of {}", page.index, page.count)
    };
    c.sans_centered(
        Rect::new(t.x, t.y + 54, t.w, 34),
        SUBTITLE_PX,
        &sub,
        if erase_mode() { BLACK } else { DARK_GRAY },
        erase_mode(),
    );

    // Pen | Eraser switch: the active half is filled black.
    let erasing = erase_mode();
    c.frame_round(layout.tool, 20.0, 4, BLACK, WHITE);
    let (on, off) = if erasing {
        (layout.erase, layout.pen)
    } else {
        (layout.pen, layout.erase)
    };
    let _ = off;
    c.fill_round(on.inset(4), 16.0, BLACK);
    // Square off the inner edge of the filled half so the two halves meet in a straight line.
    if erasing {
        c.fill(Rect::new(on.x, on.y + 4, 18, on.h - 8), BLACK);
    } else {
        c.fill(Rect::new(on.right() - 18, on.y + 4, 18, on.h - 8), BLACK);
    }
    c.fill(
        Rect::new(layout.erase.x - 2, layout.tool.y, 4, layout.tool.h),
        BLACK,
    );
    pen_icon(&mut c, layout.pen, if erasing { BLACK } else { WHITE });
    eraser_icon(&mut c, layout.erase, if erasing { WHITE } else { BLACK });

    undo_icon(&mut c, layout.undo, BLACK);
    let m = layout.menu;
    for i in 0..3 {
        c.disc(
            (m.x + m.w / 2) as f32 + (i as f32 - 1.0) * 18.0,
            (m.y + m.h / 2) as f32,
            5.5,
            BLACK,
        );
    }

    // Bottom: previous, position, next / new page.
    if page.can_previous() {
        let p = layout.previous;
        c.frame_round(p, (p.h / 2) as f32, 4, BLACK, WHITE);
        chevron(
            &mut c,
            p.x as f32 + 34.0,
            (p.y + p.h / 2) as f32,
            true,
            BLACK,
        );
        let label = format!("Page {}", page.index - 1);
        c.sans_centered(
            Rect::new(p.x + 56, p.y, p.w - 76, p.h),
            LABEL_PX,
            &label,
            BLACK,
            true,
        );
    }
    let n = layout.next;
    if page.index < page.count {
        c.frame_round(n, (n.h / 2) as f32, 4, BLACK, WHITE);
        chevron(
            &mut c,
            n.right() as f32 - 34.0,
            (n.y + n.h / 2) as f32,
            false,
            BLACK,
        );
        let label = format!("Page {}", page.index + 1);
        c.sans_centered(
            Rect::new(n.x + 20, n.y, n.w - 76, n.h),
            LABEL_PX,
            &label,
            BLACK,
            true,
        );
    } else {
        c.fill_round(n, (n.h / 2) as f32, BLACK);
        let (px, py) = (n.x as f32 + 46.0, (n.y + n.h / 2) as f32);
        c.line(px - 14.0, py, px + 14.0, py, 6.0, WHITE);
        c.line(px, py - 14.0, px, py + 14.0, 6.0, WHITE);
        c.sans_centered(
            Rect::new(n.x + 66, n.y, n.w - 86, n.h),
            LABEL_PX,
            "New page",
            WHITE,
            true,
        );
    }
    let pl = layout.page_label;
    if page.count <= MAX_DOTS {
        let gap = 34.0;
        let total = gap * (page.count.saturating_sub(1)) as f32;
        let x0 = (pl.x + pl.w / 2) as f32 - total / 2.0;
        let cy = (pl.y + pl.h / 2) as f32;
        for i in 0..page.count {
            let x = x0 + gap * i as f32;
            if i + 1 == page.index {
                c.disc(x, cy, 13.0, BLACK);
            } else {
                c.disc(x, cy, 10.0, BLACK);
                c.disc(x, cy, 6.0, WHITE);
            }
        }
    } else {
        c.sans_centered(
            pl,
            LABEL_PX,
            &format!("{} / {}", page.index, page.count),
            BLACK,
            true,
        );
    }
    Ok(layout)
}

#[cfg(test)]
mod erase_button_tests {
    use super::*;

    fn geo() -> FbGeometry {
        FbGeometry {
            xres: 1860,
            yres: 2480,
            xres_virtual: 1872,
            yres_virtual: 4960,
            xoffset: 0,
            yoffset: 0,
            bits_per_pixel: 8,
            line_length: 1872,
            smem_len: 1872 * 4960,
        }
    }

    fn centre(r: Rect) -> UiPoint {
        UiPoint {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        }
    }

    /// Writes `chrome-<state>.pgm` previews into `$CHROME_PREVIEW_DIR` (design check, not a
    /// regression test): `cargo test -p scribe-app --lib dump_chrome_preview -- --ignored`.
    #[test]
    #[ignore]
    fn dump_chrome_preview() {
        let dir = std::env::var("CHROME_PREVIEW_DIR").unwrap_or_else(|_| "/tmp".into());
        let g = geo();
        for (name, erase, page) in [
            ("middle", false, PageChrome { index: 2, count: 5 }),
            ("last", false, PageChrome { index: 5, count: 5 }),
            ("erase", true, PageChrome { index: 1, count: 2 }),
        ] {
            set_erase_mode(erase);
            let mut fb = vec![0xEEu8; g.smem_len as usize];
            render_notebook_chrome(&mut fb, &g, "Tasks", page).unwrap();
            let mut out = format!("P5\n{} {}\n255\n", g.xres, g.yres).into_bytes();
            for y in 0..g.yres as usize {
                let row = y * g.line_length as usize;
                out.extend_from_slice(&fb[row..row + g.xres as usize]);
            }
            std::fs::write(format!("{dir}/chrome-{name}.pgm"), out).unwrap();
        }
        set_erase_mode(false);
    }

    #[test]
    fn erase_button_fits_the_top_band_without_overlapping_neighbours() {
        let l = ChromeLayout::new(&geo());
        assert!(l.erase.y + l.erase.h <= TOP_BAND_H - RULE_H);
        assert!(
            l.pen.x >= l.title.x + l.title.w,
            "title must end before the Pen | Eraser switch"
        );
        assert_eq!(l.pen.right(), l.erase.x, "the two halves touch");
        assert!(
            l.erase.x + l.erase.w <= l.undo.x,
            "Erase must end before Undo"
        );
        assert!(l.home.right() <= l.title.x);
        assert!(
            l.title.w >= 500,
            "title keeps a usable width, got {}",
            l.title.w
        );
    }

    #[test]
    fn tapping_erase_hits_erase_and_neighbours_keep_their_targets() {
        let l = ChromeLayout::new(&geo());
        assert_eq!(l.hit(centre(l.erase)), Some(ChromeHit::Erase));
        assert_eq!(l.hit(centre(l.pen)), Some(ChromeHit::Pen));
        assert_eq!(l.hit(centre(l.undo)), Some(ChromeHit::Undo));
        assert_eq!(l.hit(centre(l.home)), Some(ChromeHit::Home));
        assert_eq!(l.hit(centre(l.menu)), Some(ChromeHit::PageMenu));
        assert_eq!(l.hit(centre(l.title)), Some(ChromeHit::Inert));
    }

    #[test]
    fn button_is_inverted_only_while_erase_mode_is_on() {
        let g = geo();
        let l = ChromeLayout::new(&g);
        // An interior pixel of the Eraser half, left of its icon and away from the frame.
        let (x, y) = (l.erase.x + 20, l.erase.y + l.erase.h / 2);
        let at = |fb: &[u8]| fb[y as usize * g.line_length as usize + x as usize];
        let page = PageChrome { index: 1, count: 2 };

        set_erase_mode(false);
        let mut fb = vec![0x80u8; g.smem_len as usize];
        render_notebook_chrome(&mut fb, &g, "T", page).unwrap();
        assert_eq!(at(&fb), WHITE);

        set_erase_mode(true);
        let mut fb = vec![0x80u8; g.smem_len as usize];
        render_notebook_chrome(&mut fb, &g, "T", page).unwrap();
        assert_eq!(at(&fb), BLACK);

        set_erase_mode(false);
        assert!(!erase_mode());
    }
}
