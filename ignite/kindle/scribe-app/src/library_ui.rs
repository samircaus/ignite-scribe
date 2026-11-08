//! Shared look for the library screens (Notebooks and Books).
//!
//! One header (left control, Notebooks | Books tabs, actions), one 4 x 3 grid of portrait covers
//! with a two-line caption, and one pager. Notebook covers are first-page thumbnails, book
//! covers are rendered first pages, and folders are drawn folders. E-ink first: black, white
//! and one light gray, no gradients, every control at least 130 px.

use kindle_platform::FbGeometry;

use crate::ui_gfx::{sans_wrap, Canvas, Rect, BLACK, GRAY, WHITE};

pub const MARGIN: u32 = 60;
pub const BAR_Y: u32 = 36;
pub const BAR_H: u32 = 130;
pub const BAR_GAP: u32 = 20;
pub const DIVIDER_Y: u32 = 196;
pub const GRID_TOP: u32 = 236;
pub const COLS: usize = 4;
pub const GRID_ROWS: usize = 3;
/// Items (covers or folders) per library page.
pub const ITEMS_PER_PAGE: usize = COLS * GRID_ROWS;
const COL_GAP: u32 = 44;
const ROW_GAP: u32 = 36;
/// Caption under each cover: title line + detail line.
pub const CAPTION_H: u32 = 110;
/// The second sheet drawn behind a multi-page notebook.
const STACK: u32 = 10;
pub const PAGER_H: u32 = 150;
const PAGER_BOTTOM: u32 = 40;
pub const PAGER_BUTTON_W: u32 = 300;
/// "More" target at the caption's right edge (also covers the cover's bottom-right corner).
pub const MENU_HIT: u32 = 130;
/// Secondary text.
pub const DARK: u8 = 0x50;
/// Light fill (folder bodies, inactive surfaces).
pub const LIGHT: u8 = 0xE4;
const TAB_NOTEBOOKS_W: u32 = 260;
const TAB_BOOKS_W: u32 = 210;
const TITLE_PX: f32 = 38.0;
const DETAIL_PX: f32 = 31.0;

/// 8-bit gray cover pixels, row-major, `w * h` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverImage {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

/// One grid cell: the whole tap target, the cover box, the caption and the "more" target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub frame: Rect,
    pub cover: Rect,
    pub caption: Rect,
    pub menu: Rect,
}

fn pager_y(geo: &FbGeometry) -> u32 {
    geo.yres.saturating_sub(PAGER_H + PAGER_BOTTOM)
}

/// Cell `slot` (0..ITEMS_PER_PAGE) of a library page.
pub fn cell(geo: &FbGeometry, slot: usize) -> Cell {
    let width = geo.xres.saturating_sub(2 * MARGIN);
    let cols = COLS as u32;
    let rows = GRID_ROWS as u32;
    let cell_w = width.saturating_sub((cols - 1) * COL_GAP) / cols;
    let bottom = pager_y(geo).saturating_sub(24);
    let cell_h = bottom.saturating_sub(GRID_TOP + (rows - 1) * ROW_GAP) / rows;
    let (col, row) = ((slot % COLS) as u32, (slot / COLS) as u32);
    let frame = Rect::new(
        MARGIN + col * (cell_w + COL_GAP),
        GRID_TOP + row * (cell_h + ROW_GAP),
        cell_w,
        cell_h,
    );
    let (cw, ch) = cover_size_in(cell_w, cell_h);
    let cover = Rect::new(frame.x, frame.y, cw, ch);
    let caption_y = frame.y + cell_h.saturating_sub(CAPTION_H);
    let caption = Rect::new(frame.x, caption_y, cell_w, CAPTION_H);
    let menu = Rect::new(
        frame.right().saturating_sub(MENU_HIT),
        frame.bottom().saturating_sub(MENU_HIT),
        MENU_HIT,
        MENU_HIT,
    );
    Cell {
        frame,
        cover,
        caption,
        menu,
    }
}

/// 3:4 portrait cover fitted in the cell above the caption, leaving room for the stack sheet.
fn cover_size_in(cell_w: u32, cell_h: u32) -> (u32, u32) {
    let box_w = cell_w.saturating_sub(STACK);
    let box_h = cell_h.saturating_sub(CAPTION_H + STACK + 6);
    if box_w * 4 <= box_h * 3 {
        (box_w, box_w * 4 / 3)
    } else {
        (box_h * 3 / 4, box_h)
    }
}

/// Pixel size covers are generated at (matches every cell's cover box).
pub fn cover_size(geo: &FbGeometry) -> (u32, u32) {
    let c = cell(geo, 0);
    (c.cover.w, c.cover.h)
}

/// Left control of the header (exit door or back chevron).
pub fn left_control() -> Rect {
    Rect::new(MARGIN, BAR_Y, BAR_H, BAR_H)
}

/// `(notebooks tab, books tab)` touch targets, right of the left control.
pub fn tabs() -> (Rect, Rect) {
    let x = left_control().right() + BAR_GAP;
    (
        Rect::new(x, BAR_Y, TAB_NOTEBOOKS_W, BAR_H),
        Rect::new(x + TAB_NOTEBOOKS_W, BAR_Y, TAB_BOOKS_W, BAR_H),
    )
}

/// `(previous, label, next)` pager rects.
pub fn pager(geo: &FbGeometry) -> (Rect, Rect, Rect) {
    let y = pager_y(geo);
    let cx = geo.xres / 2;
    (
        Rect::new(MARGIN, y, PAGER_BUTTON_W, PAGER_H),
        Rect::new(cx.saturating_sub(400), y, 800, PAGER_H),
        Rect::new(
            geo.xres.saturating_sub(MARGIN + PAGER_BUTTON_W),
            y,
            PAGER_BUTTON_W,
            PAGER_H,
        ),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftKind {
    Exit,
    Back,
}

pub fn draw_left(c: &mut Canvas<'_>, kind: LeftKind) {
    let r = left_control();
    let (cx, cy) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
    match kind {
        LeftKind::Back => {
            c.polyline(
                &[
                    (cx + 10.0, cy - 26.0),
                    (cx - 14.0, cy),
                    (cx + 10.0, cy + 26.0),
                ],
                8.0,
                BLACK,
            );
        }
        LeftKind::Exit => {
            // A door frame with an arrow out.
            let (x0, y0, x1, y1) = (cx - 22.0, cy - 30.0, cx + 22.0, cy + 30.0);
            c.polyline(
                &[(x0 + 20.0, y0), (x0, y0), (x0, y1), (x0 + 20.0, y1)],
                5.0,
                BLACK,
            );
            c.line(x0 + 12.0, cy, x1, cy, 5.0, BLACK);
            c.polyline(
                &[(x1 - 11.0, cy - 11.0), (x1, cy), (x1 - 11.0, cy + 11.0)],
                5.0,
                BLACK,
            );
        }
    }
}

/// Segmented "Notebooks | Books" switch; `books_active` picks the filled segment.
pub fn draw_tabs(c: &mut Canvas<'_>, books_active: bool) {
    let (nb, bk) = tabs();
    let outer = Rect::new(nb.x, nb.y + 20, bk.right() - nb.x, nb.h - 40);
    let radius = (outer.h / 2) as f32;
    c.frame_round(outer, radius, 4, BLACK, WHITE);
    for (r, label, active) in [
        (nb, "Notebooks", !books_active),
        (bk, "Books", books_active),
    ] {
        let seg = Rect::new(r.x + 8, outer.y + 8, r.w - 16, outer.h - 16);
        if active {
            c.fill_round(seg, (seg.h / 2) as f32, BLACK);
        }
        c.sans_centered(seg, 36.0, label, if active { WHITE } else { BLACK }, active);
    }
}

pub fn draw_divider(c: &mut Canvas<'_>, geo: &FbGeometry) {
    c.fill(
        Rect::new(MARGIN, DIVIDER_Y, geo.xres.saturating_sub(2 * MARGIN), 3),
        BLACK,
    );
}

/// Arrows only when there is more than one page; the label always.
pub fn draw_pager(c: &mut Canvas<'_>, geo: &FbGeometry, page: usize, pages: usize) {
    let (prev, label, next) = pager(geo);
    if pages > 1 {
        for (r, left, enabled) in [(prev, true, page > 0), (next, false, page + 1 < pages)] {
            let ink = if enabled { BLACK } else { GRAY };
            let pill = Rect::new(r.x, r.y + 24, r.w, r.h - 48);
            c.frame_round(pill, (pill.h / 2) as f32, 4, ink, WHITE);
            let (cx, cy) = ((pill.x + pill.w / 2) as f32, (pill.y + pill.h / 2) as f32);
            let d = if left { 1.0 } else { -1.0 };
            c.polyline(
                &[
                    (cx + 12.0 * d, cy - 22.0),
                    (cx - 12.0 * d, cy),
                    (cx + 12.0 * d, cy + 22.0),
                ],
                7.0,
                ink,
            );
        }
    }
    c.sans_centered(
        label,
        38.0,
        &format!("{} of {}", page + 1, pages.max(1)),
        BLACK,
        true,
    );
}

/// Draw `img` scaled (box filter down, nearest up) to fit `rect`, centred horizontally and
/// bottom-aligned like a book on a shelf. Returns the rect actually covered.
pub fn blit_fit(c: &mut Canvas<'_>, rect: Rect, img: &CoverImage) -> Rect {
    if img.w == 0 || img.h == 0 || img.pixels.len() < (img.w * img.h) as usize {
        return rect;
    }
    let scale = (rect.w as f32 / img.w as f32).min(rect.h as f32 / img.h as f32);
    let w = ((img.w as f32 * scale).round() as u32).clamp(1, rect.w);
    let h = ((img.h as f32 * scale).round() as u32).clamp(1, rect.h);
    let dst = Rect::new(rect.x + (rect.w - w) / 2, rect.bottom() - h, w, h);
    for y in 0..h {
        let sy0 = y * img.h / h;
        let sy1 = ((y + 1) * img.h / h).max(sy0 + 1).min(img.h);
        for x in 0..w {
            let sx0 = x * img.w / w;
            let sx1 = ((x + 1) * img.w / w).max(sx0 + 1).min(img.w);
            let (mut sum, mut n) = (0u32, 0u32);
            for sy in sy0..sy1 {
                let row = (sy * img.w) as usize;
                for sx in sx0..sx1 {
                    sum += u32::from(img.pixels[row + sx as usize]);
                    n += 1;
                }
            }
            c.put(dst.x + x, dst.y + y, (sum / n.max(1)) as u8);
        }
    }
    dst
}

/// Title (bold) and detail line; `menu` reserves the right edge for the "more" dots.
pub fn draw_caption(c: &mut Canvas<'_>, cell: &Cell, title: &str, detail: &str, menu: bool) {
    let cap = cell.caption;
    let text_w = if menu {
        cap.w.saturating_sub(MENU_HIT - 10)
    } else {
        cap.w
    };
    c.sans_left(
        Rect::new(cap.x + 4, cap.y + 12, text_w, 52),
        TITLE_PX,
        title,
        BLACK,
        true,
    );
    c.sans_left(
        Rect::new(cap.x + 4, cap.y + 64, text_w, 40),
        DETAIL_PX,
        detail,
        DARK,
        false,
    );
    if menu {
        let cx = (cell.menu.x + cell.menu.w / 2 + 14) as f32;
        let cy = (cap.y + 12 + 26) as f32;
        for i in 0..3 {
            c.disc(cx + (i as f32 - 1.0) * 18.0, cy, 6.5, BLACK);
        }
    }
}

/// White sheet with a rounded outline; `selected` thickens it.
fn sheet(c: &mut Canvas<'_>, r: Rect, selected: bool) {
    c.frame_round(r, 12.0, if selected { 8 } else { 3 }, BLACK, WHITE);
}

/// Notebook cover: its first page (or blank ruled paper), a second sheet behind when the
/// notebook has more than one page.
pub fn draw_notebook_cover(
    c: &mut Canvas<'_>,
    cell: &Cell,
    image: Option<&CoverImage>,
    stacked: bool,
    selected: bool,
) {
    let r = cell.cover;
    if stacked {
        let back = Rect::new(r.x + STACK, r.y + STACK, r.w, r.h);
        c.frame_round(back, 12.0, 3, BLACK, LIGHT);
    }
    sheet(c, r, selected);
    let inner = r.inset(if selected { 9 } else { 4 });
    match image {
        Some(img) => {
            // Keep the rounded corners: paint inside a slightly smaller box.
            let area = Rect::new(inner.x + 4, inner.y + 4, inner.w - 8, inner.h - 8);
            blit_fit(c, area, img);
        }
        None => {
            let mut y = r.y + 80;
            while y + 30 < r.bottom() {
                c.fill(Rect::new(r.x + 28, y, r.w.saturating_sub(56), 2), LIGHT);
                y += 44;
            }
        }
    }
}

/// Folder: a tab and a light body showing how many items it holds; `noun` is
/// (singular, plural), e.g. ("notebook", "notebooks").
pub fn draw_folder_cover(c: &mut Canvas<'_>, cell: &Cell, count: usize, noun: (&str, &str)) {
    let r = cell.cover;
    let tab = Rect::new(r.x, r.y + 30, r.w * 2 / 5, 70);
    c.frame_round(tab, 14.0, 4, BLACK, LIGHT);
    let body = Rect::new(r.x, r.y + 70, r.w + STACK, r.h + STACK - 70);
    c.frame_round(body, 16.0, 4, BLACK, LIGHT);
    c.fill(Rect::new(tab.x + 4, body.y, tab.w - 8, 4), LIGHT);
    c.sans_centered(
        Rect::new(body.x, body.y + body.h / 2 - 90, body.w, 140),
        120.0,
        &count.to_string(),
        BLACK,
        true,
    );
    c.sans_centered(
        Rect::new(body.x, body.y + body.h / 2 + 70, body.w, 50),
        34.0,
        if count == 1 { noun.0 } else { noun.1 },
        DARK,
        false,
    );
}

/// Book cover: its rendered first page, or a typeset cover (title + `format`) when none is
/// available. `progress`
/// (0..=1) draws a reading bar along the cover's bottom edge.
pub fn draw_book_cover(
    c: &mut Canvas<'_>,
    cell: &Cell,
    image: Option<&CoverImage>,
    title: &str,
    format: &str,
    progress: Option<f32>,
    selected: bool,
) {
    let r = cell.cover;
    let drawn = match image {
        Some(img) => {
            // Fit the page first, then frame exactly what was drawn.
            let area = Rect::new(r.x, r.y, r.w, r.h);
            let scale =
                (area.w as f32 / img.w.max(1) as f32).min(area.h as f32 / img.h.max(1) as f32);
            let w = ((img.w as f32 * scale) as u32).clamp(40, area.w);
            let h = ((img.h as f32 * scale) as u32).clamp(40, area.h);
            let frame = Rect::new(area.x + (area.w - w) / 2, area.bottom() - h, w, h);
            sheet(c, frame, selected);
            let t = if selected { 8 } else { 3 };
            blit_fit(c, frame.inset(t + 2), img);
            frame
        }
        None => {
            sheet(c, r, selected);
            // Typeset cover: a black spine, the title, a rule and the format.
            c.fill_round(Rect::new(r.x + 3, r.y + 3, 26, r.h - 6), 10.0, BLACK);
            let text_x = r.x + 58;
            let text_w = r.w.saturating_sub(90);
            let mut y = r.y + 70;
            for line in sans_wrap(46.0, title, true, text_w, 5) {
                c.sans_left(Rect::new(text_x, y, text_w, 60), 46.0, &line, BLACK, true);
                y += 62;
            }
            c.fill(Rect::new(text_x, y + 24, 90, 5), BLACK);
            c.sans_left(
                Rect::new(text_x, r.bottom() - 100, text_w, 50),
                30.0,
                format,
                DARK,
                true,
            );
            r
        }
    };
    if let Some(p) = progress {
        let t = if selected { 8 } else { 3 };
        let track = Rect::new(drawn.x + t, drawn.bottom() - t - 16, drawn.w - 2 * t, 16);
        c.fill(track, WHITE);
        c.fill(Rect::new(track.x, track.y, track.w, 2), BLACK);
        let w = ((track.w as f32) * p.clamp(0.0, 1.0)).round() as u32;
        c.fill(Rect::new(track.x, track.y, w.max(6), track.h), BLACK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_gfx::panel_geo;

    #[test]
    fn grid_cells_tile_the_page_without_overlap_and_keep_big_targets() {
        let g = panel_geo();
        let cells: Vec<Cell> = (0..ITEMS_PER_PAGE).map(|i| cell(&g, i)).collect();
        let (prev, _, _) = pager(&g);
        for (i, a) in cells.iter().enumerate() {
            assert!(a.frame.y >= GRID_TOP && a.frame.bottom() < prev.y, "{a:?}");
            assert!(a.frame.right() <= g.xres - MARGIN);
            assert!(a.cover.right() <= a.frame.right() && a.cover.bottom() <= a.caption.y);
            assert!(a.menu.w >= 120 && a.menu.h >= 120, "{:?}", a.menu);
            assert!(a.menu.bottom() <= a.frame.bottom() && a.menu.right() <= a.frame.right());
            assert_eq!(a.cover.w * 4 / 3, a.cover.h);
            for b in &cells[i + 1..] {
                assert!(!a.frame.intersects(&b.frame));
            }
        }
        let (nb, bk) = tabs();
        assert!(left_control().right() <= nb.x && nb.right() <= bk.x);
        assert!(bk.h >= 130 && nb.w >= 130 && bk.w >= 130);
    }
}
