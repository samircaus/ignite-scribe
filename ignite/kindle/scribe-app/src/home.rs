//! Gate F11 framebuffer-native Home/library view.
//!
//! A grid of covers (`library_ui`): folders first, then notebooks, `ITEMS_PER_PAGE` per page
//! with deterministic viewport paging. Notebook covers are first-page thumbnails supplied by
//! the caller (derived cache only; Home itself never reads a notebook). The viewport is
//! transient UI state; it never changes `library.meta`. This renderer paints only the live
//! framebuffer; callers submit one GC16 FULL refresh afterward.
//!
//! Folders (Kindle-local, `folders.rs`): the main grid shows folder tiles first, then the
//! notebooks in no folder; items page together, so a folder is never repeated on later pages.
//! Opening a folder shows its notebooks in the same grid, with a back arrow.

use std::collections::BTreeMap;

use kindle_platform::FbGeometry;

use crate::library::LibraryRepository;
use crate::library_ui::{self as lui, CoverImage, LeftKind, BAR_GAP, BAR_H, BAR_Y, MARGIN};
use crate::ui_gfx::{Canvas, Rect, UiPoint, BLACK, GLYPH_H, GLYPH_W, WHITE};

pub use crate::folders::FolderInfo;
pub use crate::library_ui::ITEMS_PER_PAGE;

/// Exit pictogram drawn in one glyph cell at this scale (50x70), centred in a BAR_H square.
const EXIT_SCALE: u32 = 10;
const NEW_W: u32 = 250;
/// D1: the Sync entry point (product mode only), left of "+ NEW".
const SYNC_W: u32 = 200;
const TOOLS_W: u32 = BAR_H;
pub const SEARCH_PLACEHOLDER: &str = "Search notebooks...";

/// First-page covers by notebook id (only the visible page needs entries).
pub type HomeCovers = BTreeMap<String, CoverImage>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppView {
    Notebook,
    Home,
    /// D1: server discovery / pairing / sync status. Never a writing surface.
    Sync,
    /// Tools (Import Kindle notebooks). Never a writing surface.
    Tools,
    /// Replica-local preferences (side-button action). Never a writing surface.
    Preferences,
    /// Read-only picker for choosing a page-link target. Never a writing surface.
    LinkTargetPicker,
    #[cfg(feature = "reader")]
    Books,
    #[cfg(feature = "reader")]
    Reader,
}

impl AppView {
    /// The single routing gate used before any page gesture state machine sees input.
    pub fn allows_page_input(self) -> bool {
        matches!(self, Self::Notebook)
    }
}

/// One notebook as Home shows it. `page_count` is `None` when its manifest is unreadable
/// (rendered conservatively; never repaired from Home).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeCard {
    pub notebook_id: String,
    pub title: String,
    pub page_count: Option<usize>,
}

/// Read-only snapshot of the library for Home rendering and hit-testing.
pub fn home_cards(library: &LibraryRepository) -> Vec<HomeCard> {
    library
        .manifest()
        .notebook_ids
        .iter()
        .map(|id| {
            let summary = library.home_summaries().get(id);
            HomeCard {
                notebook_id: id.clone(),
                title: summary
                    .map(|s| s.title.clone())
                    .unwrap_or_else(|| "?".into()),
                page_count: summary.and_then(|s| s.page_count),
            }
        })
        .collect()
}

/// One Home grid item: a folder (index into the folder list) or a notebook (index into the
/// library-order card list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HomeRow {
    Folder(usize),
    Notebook(usize),
}

/// Items of the current view. Main grid (`open == None`): every folder, then every notebook in
/// no folder, in library order. Inside a folder: that folder's notebooks, in library order.
pub fn home_rows(cards: &[HomeCard], folders: &[FolderInfo], open: Option<&str>) -> Vec<HomeRow> {
    match open.and_then(|id| folders.iter().find(|f| f.id == id)) {
        Some(f) => cards
            .iter()
            .enumerate()
            .filter(|(_, c)| f.notebook_ids.contains(&c.notebook_id))
            .map(|(i, _)| HomeRow::Notebook(i))
            .collect(),
        None => {
            let foldered: std::collections::HashSet<&str> = folders
                .iter()
                .flat_map(|f| f.notebook_ids.iter().map(String::as_str))
                .collect();
            (0..folders.len())
                .map(HomeRow::Folder)
                .chain(
                    cards
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| !foldered.contains(c.notebook_id.as_str()))
                        .map(|(i, _)| HomeRow::Notebook(i)),
                )
                .collect()
        }
    }
}

/// The item showing card `card` in this view: its own cover, or (main grid) its folder.
pub fn row_of_card(
    rows: &[HomeRow],
    cards: &[HomeCard],
    folders: &[FolderInfo],
    card: usize,
) -> Option<usize> {
    let id = &cards.get(card)?.notebook_id;
    let folder = folders.iter().position(|f| f.notebook_ids.contains(id));
    rows.iter().position(|r| match r {
        HomeRow::Notebook(i) => *i == card,
        HomeRow::Folder(f) => folder == Some(*f),
    })
}

/// Notebook ids whose covers the current viewport page shows.
pub fn visible_notebook_ids(
    rows: &[HomeRow],
    cards: &[HomeCard],
    state: &HomeState,
) -> Vec<String> {
    rows[state.visible_range(rows.len())]
        .iter()
        .filter_map(|r| match r {
            HomeRow::Notebook(i) => cards.get(*i).map(|c| c.notebook_id.clone()),
            HomeRow::Folder(_) => None,
        })
        .collect()
}

fn plain_rows(total: usize) -> Vec<HomeRow> {
    (0..total).map(HomeRow::Notebook).collect()
}

pub fn viewport_page_count(total: usize) -> usize {
    total.div_ceil(ITEMS_PER_PAGE).max(1)
}

/// Transient Home selection + viewport page. Selection starts on the active notebook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HomeState {
    pub selected_index: usize,
    pub page: usize,
    /// D1: show the SYNC button (product mode with a sync directory).
    pub sync: bool,
    /// Move the viewport to the selected notebook's item at the next [`HomeState::settle`]
    /// (with folders, items are not library indices).
    pub reveal: bool,
}

impl HomeState {
    pub fn for_active(library: &LibraryRepository) -> Self {
        let ids = &library.manifest().notebook_ids;
        let index = ids
            .iter()
            .position(|id| id == library.active_notebook_id())
            .unwrap_or(0);
        Self::selecting(index, ids.len())
    }

    pub fn selecting(index: usize, total: usize) -> Self {
        let mut s = Self {
            selected_index: index,
            page: 0,
            sync: false,
            reveal: true,
        };
        s.normalize(total);
        s.page = s.selected_index / ITEMS_PER_PAGE;
        s
    }

    /// Keep selection and viewport inside the library. Empty viewport pages are impossible.
    pub fn normalize(&mut self, total: usize) {
        self.selected_index = self.selected_index.min(total.saturating_sub(1));
        self.page = self.page.min(viewport_page_count(total) - 1);
    }

    pub fn ensure_selected_visible(&mut self, total: usize) {
        self.normalize(total);
        self.page = self.selected_index / ITEMS_PER_PAGE;
        self.reveal = true;
    }

    /// Fit the viewport to the laid-out `rows`. Selection stays a library index; a pending
    /// reveal moves the viewport to the item showing the selected notebook (or its folder).
    pub fn settle(&mut self, rows: &[HomeRow], cards: &[HomeCard], folders: &[FolderInfo]) {
        self.selected_index = self.selected_index.min(cards.len().saturating_sub(1));
        if std::mem::take(&mut self.reveal) {
            self.page = row_of_card(rows, cards, folders, self.selected_index)
                .map_or(0, |row| row / ITEMS_PER_PAGE);
        }
        self.page = self.page.min(viewport_page_count(rows.len()) - 1);
    }

    /// F10 developer cursor controls: wrap through the full manifest order.
    pub fn select_next(&mut self, total: usize) {
        if total > 0 {
            self.selected_index = (self.selected_index + 1) % total;
            self.ensure_selected_visible(total);
        }
    }

    pub fn select_previous(&mut self, total: usize) {
        if total > 0 {
            self.selected_index = (self.selected_index + total - 1) % total;
            self.ensure_selected_visible(total);
        }
    }

    /// Item indices shown on the current viewport page.
    pub fn visible_range(&self, total: usize) -> std::ops::Range<usize> {
        let start = (self.page * ITEMS_PER_PAGE).min(total);
        start..(start + ITEMS_PER_PAGE).min(total)
    }

    pub fn can_page_previous(&self) -> bool {
        self.page > 0
    }

    pub fn can_page_next(&self, total: usize) -> bool {
        self.page + 1 < viewport_page_count(total)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeHit {
    /// Doorway icon (top-left): leave the app cleanly.
    Exit,
    /// Inside a folder the top-left control goes back to the main grid.
    Back,
    /// Folder tile (index into the folder list): open the folder.
    OpenFolder(usize),
    /// Folder tile menu (index into the folder list).
    FolderMenu(usize),
    New,
    /// Library index (manifest order), not grid slot.
    OpenCard(usize),
    CardMenu(usize),
    PagePrevious,
    PageNext,
    /// D1: open the Sync screen.
    Sync,
    /// Feature-gated MuPDF library (the "Books" tab).
    #[cfg(feature = "reader")]
    Books,
    /// Tools screen (Import Kindle notebooks, ...).
    Tools,
    /// Title / search area: consumed, does nothing (search is not implemented yet).
    SearchPlaceholder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeLayout {
    /// Exit icon: exactly one glyph cell (scale `EXIT_SCALE`), centred in `exit_hit`.
    pub exit_icon: Rect,
    /// Square touch target around `exit_icon`.
    pub exit_hit: Rect,
    /// Flexible-width, inert title/search area.
    pub search: Rect,
    pub new_button: Rect,
    /// The "Books" tab of the Notebooks | Books switch.
    #[cfg(feature = "reader")]
    pub books_button: Rect,
    /// The "Notebooks" tab (current view; inert).
    #[cfg(feature = "reader")]
    pub notebooks_tab: Rect,
    /// D1: hit/drawn only when `HomeState::sync`.
    pub sync_button: Rect,
    pub tools_button: Rect,
    /// `(library index, cell rect, menu rect)` for the visible viewport page.
    pub cards: Vec<(usize, Rect, Rect)>,
    /// `(folder index, cell rect, menu rect)` for the visible viewport page.
    pub folders: Vec<(usize, Rect, Rect)>,
    /// Grid cells (same order as the visible items) for drawing.
    pub cells: Vec<(HomeRow, lui::Cell)>,
    /// Items in the current view (paging bound).
    pub rows_len: usize,
    /// Inside a folder: top-left is Back, the title area shows the folder title.
    pub in_folder: bool,
    pub pager_previous: Rect,
    pub pager_next: Rect,
    pub pager_label: Rect,
}

impl HomeLayout {
    /// Notebooks only (no folders): one item per library notebook.
    pub fn new(geo: &FbGeometry, state: &HomeState, total: usize) -> Self {
        Self::for_rows(geo, state, &plain_rows(total), false)
    }

    pub fn for_rows(geo: &FbGeometry, state: &HomeState, all: &[HomeRow], in_folder: bool) -> Self {
        let mut cards = Vec::new();
        let mut folders = Vec::new();
        let mut cells = Vec::new();
        for (slot, index) in state.visible_range(all.len()).enumerate() {
            let cell = lui::cell(geo, slot);
            match &all[index] {
                HomeRow::Notebook(i) => cards.push((*i, cell.frame, cell.menu)),
                HomeRow::Folder(f) => folders.push((*f, cell.frame, cell.menu)),
            }
            cells.push((all[index].clone(), cell));
        }
        let exit_hit = lui::left_control();
        let (icon_w, icon_h) = (GLYPH_W * EXIT_SCALE, GLYPH_H * EXIT_SCALE);
        let exit_icon = Rect::new(
            exit_hit.x + (BAR_H - icon_w) / 2,
            exit_hit.y + (BAR_H - icon_h) / 2,
            icon_w,
            icon_h,
        );
        let right = geo.xres.saturating_sub(MARGIN);
        let tools_button = Rect::new(right.saturating_sub(TOOLS_W), BAR_Y, TOOLS_W, BAR_H);
        let new_button = Rect::new(
            tools_button.x.saturating_sub(BAR_GAP + NEW_W),
            BAR_Y,
            NEW_W,
            BAR_H,
        );
        let sync_button = Rect::new(
            new_button.x.saturating_sub(BAR_GAP + SYNC_W),
            BAR_Y,
            SYNC_W,
            BAR_H,
        );
        #[cfg(feature = "reader")]
        let (notebooks_tab, books_button) = lui::tabs();
        #[cfg(feature = "reader")]
        let search_left = if in_folder {
            exit_hit.right() + BAR_GAP
        } else {
            books_button.right() + BAR_GAP
        };
        #[cfg(not(feature = "reader"))]
        let search_left = exit_hit.right() + BAR_GAP;
        let search_right = if state.sync {
            sync_button.x
        } else {
            new_button.x
        }
        .saturating_sub(BAR_GAP);
        let search = Rect::new(
            search_left,
            BAR_Y,
            search_right.saturating_sub(search_left),
            BAR_H,
        );
        let (pager_previous, pager_label, pager_next) = lui::pager(geo);
        Self {
            exit_icon,
            exit_hit,
            search,
            new_button,
            #[cfg(feature = "reader")]
            books_button,
            #[cfg(feature = "reader")]
            notebooks_tab,
            sync_button,
            tools_button,
            cards,
            folders,
            cells,
            rows_len: all.len(),
            in_folder,
            pager_previous,
            pager_next,
            pager_label,
        }
    }

    /// `_total` is kept for callers; paging is bounded by the laid-out items.
    pub fn hit(&self, p: UiPoint, state: &HomeState, _total: usize) -> Option<HomeHit> {
        let total = self.rows_len;
        if self.exit_hit.contains(p.x, p.y) {
            return Some(if self.in_folder {
                HomeHit::Back
            } else {
                HomeHit::Exit
            });
        }
        if self.new_button.contains(p.x, p.y) {
            return Some(HomeHit::New);
        }
        #[cfg(feature = "reader")]
        if !self.in_folder {
            if self.books_button.contains(p.x, p.y) {
                return Some(HomeHit::Books);
            }
            if self.notebooks_tab.contains(p.x, p.y) {
                return Some(HomeHit::SearchPlaceholder);
            }
        }
        if state.sync && self.sync_button.contains(p.x, p.y) {
            return Some(HomeHit::Sync);
        }
        if self.tools_button.contains(p.x, p.y) {
            return Some(HomeHit::Tools);
        }
        if self.search.contains(p.x, p.y) {
            return Some(HomeHit::SearchPlaceholder);
        }
        for (index, tile, menu) in &self.folders {
            if menu.contains(p.x, p.y) {
                return Some(HomeHit::FolderMenu(*index));
            }
            if tile.contains(p.x, p.y) {
                return Some(HomeHit::OpenFolder(*index));
            }
        }
        for (index, card, menu) in &self.cards {
            if menu.contains(p.x, p.y) {
                return Some(HomeHit::CardMenu(*index));
            }
            if card.contains(p.x, p.y) {
                return Some(HomeHit::OpenCard(*index));
            }
        }
        if self.pager_previous.contains(p.x, p.y) && state.can_page_previous() {
            return Some(HomeHit::PagePrevious);
        }
        if self.pager_next.contains(p.x, p.y) && state.can_page_next(total) {
            return Some(HomeHit::PageNext);
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HomeRenderSummary {
    pub visible_entries: usize,
    pub total_entries: usize,
    pub viewport_page: usize,
    pub viewport_pages: usize,
}

pub fn pages_label(count: Option<usize>) -> String {
    match count {
        Some(1) => "1 page".into(),
        Some(n) => format!("{n} pages"),
        None => "? pages".into(),
    }
}

/// Notebooks only (no folders, no covers).
pub fn render_home(
    fb: &mut [u8],
    geo: &FbGeometry,
    cards: &[HomeCard],
    state: &HomeState,
) -> Result<HomeRenderSummary, String> {
    render_home_view(fb, geo, cards, &[], None, state, &HomeCovers::new())
}

/// Home with folders: the main grid (`open == None`) or one open folder.
pub fn render_home_view(
    fb: &mut [u8],
    geo: &FbGeometry,
    cards: &[HomeCard],
    folders: &[FolderInfo],
    open: Option<&str>,
    state: &HomeState,
    covers: &HomeCovers,
) -> Result<HomeRenderSummary, String> {
    let open_folder = open.and_then(|id| folders.iter().find(|f| f.id == id));
    let rows = home_rows(cards, folders, open_folder.map(|f| f.id.as_str()));
    let total = rows.len();
    let mut state = *state;
    state.settle(&rows, cards, folders);
    let layout = HomeLayout::for_rows(geo, &state, &rows, open_folder.is_some());
    let mut c = Canvas::new(fb, geo)?;
    // Full white clear: no stale notebook page, modal, or previous Home frame survives.
    c.fill_all(WHITE);
    if let Some(f) = open_folder {
        lui::draw_left(&mut c, LeftKind::Back);
        c.sans_left(layout.search.inset(4), 58.0, &f.title, BLACK, true);
    } else {
        lui::draw_left(&mut c, LeftKind::Exit);
        #[cfg(feature = "reader")]
        lui::draw_tabs(&mut c, false);
        #[cfg(not(feature = "reader"))]
        c.sans_left(layout.search.inset(4), 62.0, "Notebooks", BLACK, true);
    }
    if state.sync {
        let r = Rect::new(
            layout.sync_button.x,
            layout.sync_button.y + 20,
            layout.sync_button.w,
            layout.sync_button.h - 40,
        );
        c.frame_round(r, (r.h / 2) as f32, 4, BLACK, WHITE);
        c.sans_centered(r, 38.0, "Sync", BLACK, true);
    }
    let nb = Rect::new(
        layout.new_button.x,
        layout.new_button.y + 20,
        layout.new_button.w,
        layout.new_button.h - 40,
    );
    c.fill_round(nb, (nb.h / 2) as f32, BLACK);
    let (px, py) = (nb.x as f32 + 58.0, (nb.y + nb.h / 2) as f32);
    c.line(px - 17.0, py, px + 17.0, py, 7.0, WHITE);
    c.line(px, py - 17.0, px, py + 17.0, 7.0, WHITE);
    c.sans_centered(
        Rect::new(nb.x + 84, nb.y, nb.w - 104, nb.h),
        40.0,
        "New",
        WHITE,
        true,
    );
    let tb = Rect::new(
        layout.tools_button.x + 11,
        layout.tools_button.y + 20,
        layout.tools_button.w - 22,
        layout.tools_button.h - 40,
    );
    c.frame_round(tb, (tb.h / 2) as f32, 4, BLACK, WHITE);
    let (gx, gy) = ((tb.x + tb.w / 2) as f32, (tb.y + tb.h / 2) as f32);
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::FRAC_PI_4;
        c.line(
            gx + 15.0 * a.cos(),
            gy + 15.0 * a.sin(),
            gx + 25.0 * a.cos(),
            gy + 25.0 * a.sin(),
            8.0,
            BLACK,
        );
    }
    c.disc(gx, gy, 18.0, BLACK);
    c.disc(gx, gy, 8.0, WHITE);
    lui::draw_divider(&mut c, geo);

    if total == 0 {
        let msg = if open_folder.is_some() {
            "This folder is empty"
        } else {
            "No notebooks yet"
        };
        let r = Rect::new(
            MARGIN,
            lui::GRID_TOP + 300,
            geo.xres.saturating_sub(2 * MARGIN),
            120,
        );
        c.sans_centered(r, 50.0, msg, BLACK, true);
        c.sans_centered(
            Rect::new(r.x, r.bottom() + 10, r.w, 70),
            36.0,
            "Tap New to start one.",
            lui::DARK,
            false,
        );
    }

    for (item, cell) in &layout.cells {
        match item {
            HomeRow::Folder(fi) => {
                let f = &folders[*fi];
                lui::draw_folder_cover(
                    &mut c,
                    cell,
                    f.notebook_ids.len(),
                    ("notebook", "notebooks"),
                );
                lui::draw_caption(&mut c, cell, &f.title, "Folder", true);
            }
            HomeRow::Notebook(i) => {
                let entry = &cards[*i];
                let selected = *i == state.selected_index;
                lui::draw_notebook_cover(
                    &mut c,
                    cell,
                    covers.get(&entry.notebook_id),
                    entry.page_count.is_some_and(|n| n > 1),
                    selected,
                );
                lui::draw_caption(
                    &mut c,
                    cell,
                    &entry.title,
                    &pages_label(entry.page_count),
                    true,
                );
            }
        }
    }

    let pages = viewport_page_count(total);
    lui::draw_pager(&mut c, geo, state.page, pages);

    Ok(HomeRenderSummary {
        visible_entries: layout.cards.len() + layout.folders.len(),
        total_entries: total,
        viewport_page: state.page,
        viewport_pages: pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_gfx::panel_geo;

    fn cards(n: usize) -> Vec<HomeCard> {
        (1..=n)
            .map(|i| HomeCard {
                notebook_id: format!("notebook-{i}"),
                title: format!("Notebook {i}"),
                page_count: Some(i),
            })
            .collect()
    }

    /// Writes `home-<state>.pgm` previews into `$CHROME_PREVIEW_DIR` (design check):
    /// `cargo test -p scribe-app --lib dump_home_preview -- --ignored`.
    #[test]
    #[ignore]
    fn dump_home_preview() {
        let dir = std::env::var("CHROME_PREVIEW_DIR").unwrap_or_else(|_| "/tmp".into());
        let g = panel_geo();
        let mut entries = cards(14);
        let titles = [
            "Tasks",
            "Notebook 1",
            "Creative Journal",
            "Ignite PoC",
            "Paper test",
            "Meeting notes 2026-11-05 with a very long title",
            "Ideas",
            "Reading",
            "Sketches",
            "Groceries",
            "Physics",
            "Travel",
            "Garden",
            "Letters",
        ];
        for (e, t) in entries.iter_mut().zip(titles) {
            e.title = t.into();
        }
        entries[0].page_count = Some(1);
        let folders = vec![
            FolderInfo {
                id: "f1".into(),
                title: "Work".into(),
                notebook_ids: vec!["notebook-13".into(), "notebook-14".into()],
            },
            FolderInfo {
                id: "f2".into(),
                title: "Personal".into(),
                notebook_ids: vec![],
            },
        ];
        // Fake covers: handwriting-like squiggles on white.
        let (w, h) = lui::cover_size(&g);
        let mut covers = HomeCovers::new();
        for (k, e) in entries.iter().enumerate().take(5) {
            let mut px = vec![WHITE; (w * h) as usize];
            for line in 0..(4 + k * 2) {
                let y0 = 40 + line as u32 * 34;
                for x in 30..(w - 40 - (line as u32 * 13 % 90)) {
                    let y = y0 + ((x as f32 / 7.0 + line as f32).sin() * 6.0) as u32;
                    for d in 0..2 {
                        if y + d < h {
                            px[((y + d) * w + x) as usize] = 0x10;
                        }
                    }
                }
            }
            covers.insert(e.notebook_id.clone(), CoverImage { w, h, pixels: px });
        }
        let mut state = HomeState::selecting(0, entries.len());
        state.sync = true;
        let mut fb = vec![0x55; g.smem_len as usize];
        render_home_view(&mut fb, &g, &entries, &folders, None, &state, &covers).unwrap();
        write_pgm(&format!("{dir}/home-list.pgm"), &fb, &g);
        render_home_view(&mut fb, &g, &entries, &folders, Some("f1"), &state, &covers).unwrap();
        write_pgm(&format!("{dir}/home-folder.pgm"), &fb, &g);
        let menu = crate::modal::Modal::CardMenu {
            notebook_id: "notebook-1".into(),
            title: "Tasks".into(),
        };
        render_home_view(&mut fb, &g, &entries, &folders, None, &state, &covers).unwrap();
        crate::modal::render_dialog(&mut fb, &g, &crate::modal::dialog_spec(&menu).unwrap())
            .unwrap();
        write_pgm(&format!("{dir}/home-menu.pgm"), &fb, &g);
        let confirm = crate::modal::Modal::ConfirmDeleteNotebook {
            notebook_id: "notebook-1".into(),
            title: "Tasks".into(),
        };
        render_home_view(&mut fb, &g, &entries, &folders, None, &state, &covers).unwrap();
        crate::modal::render_dialog(&mut fb, &g, &crate::modal::dialog_spec(&confirm).unwrap())
            .unwrap();
        write_pgm(&format!("{dir}/home-confirm.pgm"), &fb, &g);
        let page = crate::modal::Modal::PageMenu {
            page_number: 1,
            page_count: 3,
        };
        render_home_view(&mut fb, &g, &entries, &folders, None, &state, &covers).unwrap();
        crate::modal::render_dialog(&mut fb, &g, &crate::modal::dialog_spec(&page).unwrap())
            .unwrap();
        write_pgm(&format!("{dir}/home-pagemenu.pgm"), &fb, &g);
    }

    fn write_pgm(path: &str, fb: &[u8], g: &FbGeometry) {
        let mut out = format!("P5\n{} {}\n255\n", g.xres, g.yres).into_bytes();
        for y in 0..g.yres as usize {
            let row = y * g.line_length as usize;
            out.extend_from_slice(&fb[row..row + g.xres as usize]);
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn renders_cards_and_replacement_glyph_without_touching_stride_padding() {
        let g = panel_geo();
        let mut fb = vec![0x55; g.smem_len as usize];
        let mut entries = cards(3);
        entries[1].title = "Café".into();
        let summary = render_home(&mut fb, &g, &entries, &HomeState::selecting(0, 3)).unwrap();
        assert_eq!(summary.total_entries, 3);
        assert_eq!(summary.visible_entries, 3);
        assert!(fb.contains(&0x00));
        for y in 0..g.yres as usize {
            let padding =
                &fb[y * g.line_length as usize + g.xres as usize..(y + 1) * g.line_length as usize];
            assert!(padding.iter().all(|b| *b == 0x55));
        }

        let mut g2 = vec![0x55u8; fb.len()];
        kindle_platform::copy_logical_rect_to_hw(
            &fb,
            &mut g2,
            g.line_length,
            0,
            0,
            g.xres,
            g.yres,
            false,
        )
        .unwrap();
        assert_eq!(g2, fb);

        let mut g1 = vec![0x55u8; fb.len()];
        kindle_platform::copy_logical_rect_to_hw(
            &fb,
            &mut g1,
            g.line_length,
            0,
            0,
            g.xres,
            g.yres,
            true,
        )
        .unwrap();
        for y in 0..g.yres as usize {
            let row = y * g.line_length as usize;
            for x in 0..g.xres as usize {
                assert_eq!(g1[row + x], !fb[row + x]);
            }
            assert!(g1[row + g.xres as usize..row + g.line_length as usize]
                .iter()
                .all(|b| *b == 0x55));
        }
    }

    #[test]
    fn compact_bar_orders_controls_and_keeps_comfortable_targets() {
        let g = panel_geo();
        let mut st = HomeState::selecting(0, 3);
        st.sync = true;
        let l = HomeLayout::new(&g, &st, 3);
        for r in [
            l.exit_hit,
            l.search,
            l.sync_button,
            l.new_button,
            l.tools_button,
        ] {
            assert!(r.w >= 130 && r.h >= 130, "{r:?}");
        }
        // [exit] [search] [sync] [+ new] [tools], left to right, no overlap.
        assert!(l.exit_hit.right() <= l.search.x);
        assert!(l.search.right() <= l.sync_button.x);
        assert!(l.sync_button.right() <= l.new_button.x);
        assert!(l.new_button.right() <= l.tools_button.x);
        assert!(l.tools_button.right() <= g.xres - MARGIN);
        assert!(l.tools_button.bottom() < lui::DIVIDER_Y && lui::DIVIDER_Y < lui::GRID_TOP);
        let c = |r: Rect| UiPoint {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        };
        assert_eq!(l.hit(c(l.tools_button), &st, 3), Some(HomeHit::Tools));
        assert_eq!(l.hit(c(l.search), &st, 3), Some(HomeHit::SearchPlaceholder));
        assert_eq!(l.hit(c(l.sync_button), &st, 3), Some(HomeHit::Sync));
        // Without sync the search placeholder takes the freed space.
        st.sync = false;
        let l2 = HomeLayout::new(&g, &st, 3);
        assert!(l2.search.w > l.search.w);
        assert_eq!(
            l2.hit(c(l.sync_button), &st, 3),
            Some(HomeHit::SearchPlaceholder)
        );
    }

    #[test]
    fn home_rejects_all_page_input_routing() {
        assert!(AppView::Notebook.allows_page_input());
        assert!(!AppView::Home.allows_page_input());
        assert!(!AppView::Tools.allows_page_input());
        assert!(!AppView::Preferences.allows_page_input());
        assert!(!AppView::LinkTargetPicker.allows_page_input());
        assert!(!AppView::Sync.allows_page_input());
    }
}
