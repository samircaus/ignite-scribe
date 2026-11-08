//! Feature-gated MuPDF reader: fixed-layout PDF and CBZ comics, and reflowable EPUB laid out
//! at one fixed page size (so remembered page numbers stay valid).
//!
//! Document pixels are confined to panel rows 140..2340. The surrounding bands are live UI
//! chrome and are never sourced from the PDF. Reader position is replica-local and atomically
//! persisted outside the notebook library.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use kindle_platform::FbGeometry;
use mupdf::{Colorspace, Context, Document, Matrix};
use serde::{Deserialize, Serialize};

use crate::library_ui::{self as lui, CoverImage, LeftKind};
use crate::ui_gfx::{Canvas, Rect, UiPoint, BLACK, GRAY, WHITE};

pub const DOCUMENTS_ROOT: &str = "/mnt/us/documents";
pub const BOOKS_ROOT: &str = "/mnt/us/scribe-app/books";
pub const SAFE_TOP: u32 = 140;
pub const SAFE_BOTTOM: u32 = 2340;
const PAGE_MARGIN: u32 = 60;
const BOOKS_PER_PAGE: usize = lui::ITEMS_PER_PAGE;
const COVERS_DIR: &str = "reader-covers";
const POSITIONS_VERSION: u32 = 1;
pub const POSITIONS_FILE: &str = "reader-positions.json";

/// EPUB pages are laid out at this size in points and drawn at `EPUB_ZOOM` px/pt, which fills
/// the reader's document box (1740 x 2200 px on the Scribe) at about the panel's 300 ppi.
const EPUB_ZOOM: f32 = 4.0;
/// Body text em in layout points: 13 pt x 4 px/pt = 52 px (about 4.4 mm) on the Scribe,
/// close to the Kindle's own default reading size.
const EPUB_EM: f32 = 13.0;
/// Reading defaults for reflowable books (applies to MuPDF's HTML engine only).
const EPUB_CSS: &str = "@page { margin: 0.5em 1.4em } \
    body { margin: 0; line-height: 1.38 } \
    p { margin-top: 0; margin-bottom: 0.35em }";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookKind {
    Pdf,
    Epub,
    Comic,
}

impl BookKind {
    fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "epub" => Some(Self::Epub),
            "cbz" => Some(Self::Comic),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Epub => "EPUB",
            Self::Comic => "Comic",
        }
    }
}

/// Books folder for your own documents and reports (Ignite books, non-Calibre files).
pub const MY_FILES: &str = "My files";
/// Books folder for a Calibre-managed collection ("Title - Author (123).epub").
pub const LIBRARY: &str = "Library";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookEntry {
    pub path: PathBuf,
    pub title: String,
    pub kind: BookKind,
    /// Books folder it is listed in (display only; files never move).
    pub group: String,
}

impl BookEntry {
    fn from_path(path: PathBuf) -> Self {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let kind = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(BookKind::from_extension)
            .unwrap_or(BookKind::Pdf);
        Self {
            title: clean_title(stem),
            path,
            kind,
            group: MY_FILES.into(),
        }
    }
}

/// Calibre appends its library id: "Title - Author (123)".
fn calibre_id_suffix(stem: &str) -> Option<usize> {
    let t = stem.trim_end();
    let open = t.rfind(" (")?;
    let inner = t[open + 2..].strip_suffix(')')?;
    (!inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit())).then_some(open)
}

/// Folder for a book at `rel` (relative to its scan root): its top-level subfolder, else
/// My files for Ignite's own books folder and personal files, Library for Calibre books.
fn group_for(rel: &Path, mine_root: bool) -> String {
    let mut parts = rel.components();
    let first = parts.next();
    if parts.next().is_some() {
        if let Some(dir) = first.and_then(|c| c.as_os_str().to_str()) {
            return dir.to_string();
        }
    }
    let stem = rel.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if !mine_root && calibre_id_suffix(stem).is_some() {
        LIBRARY.into()
    } else {
        MY_FILES.into()
    }
}

/// Human title from a file name: Calibre's "Title_ Subtitle - Author (123)" loses the library
/// id and gets its colon back; a name without spaces ("long-reads-on-ai") gets them.
fn clean_title(stem: &str) -> String {
    let mut t = stem.trim().to_string();
    if let Some(open) = calibre_id_suffix(&t) {
        t.truncate(open);
    }
    let t = if t.contains(' ') {
        t.replace("_ ", ": ").replace('_', " ")
    } else {
        t.replace(['_', '-'], " ")
    };
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        "Untitled".into()
    } else {
        t
    }
}

/// Open any supported book; reflowable formats are laid out at the reader's fixed page size.
fn open_document(path: &Path) -> Result<Document, String> {
    let mut document =
        Document::open(path.to_string_lossy().as_ref()).map_err(|e| format!("open book: {e}"))?;
    if document.is_reflowable().map_err(|e| e.to_string())? {
        let mut ctx = Context::get();
        if ctx.user_css().as_deref() != Some(EPUB_CSS) {
            ctx.set_user_css(EPUB_CSS).map_err(|e| e.to_string())?;
        }
        let w = (1860 - 2 * PAGE_MARGIN) as f32 / EPUB_ZOOM;
        let h = (SAFE_BOTTOM - SAFE_TOP) as f32 / EPUB_ZOOM;
        document
            .layout(w, h, EPUB_EM)
            .map_err(|e| format!("lay out book: {e}"))?;
    }
    Ok(document)
}

pub fn scan_books() -> Result<Vec<BookEntry>, String> {
    scan_books_in(&[
        (PathBuf::from(BOOKS_ROOT), true),
        (PathBuf::from(DOCUMENTS_ROOT), false),
    ])
}

/// Scan `(root, mine)` folders; `mine` roots (Ignite's books folder) list as My files.
pub fn scan_books_in(roots: &[(PathBuf, bool)]) -> Result<Vec<BookEntry>, String> {
    let mut books = Vec::new();
    let mut seen = HashSet::new();
    for (root, mine) in roots {
        if !root.exists() {
            continue;
        }
        let canonical_root =
            fs::canonicalize(root).map_err(|e| format!("scan {}: {e}", root.display()))?;
        let mut paths = Vec::new();
        walk_books(&canonical_root, &canonical_root, 0, &mut paths)?;
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            let rel = path
                .strip_prefix(&canonical_root)
                .unwrap_or(&path)
                .to_path_buf();
            let mut book = BookEntry::from_path(path);
            book.group = group_for(&rel, *mine);
            books.push(book);
        }
    }
    books.sort_by_key(|b| b.title.to_lowercase());
    books.truncate(500);
    Ok(books)
}

fn walk_books(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > 5 || out.len() >= 500 {
        return Ok(());
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) if dir != root => return Ok(()),
        Err(error) => return Err(format!("scan {}: {error}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("scan {}: {e}", dir.display()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || name.ends_with(".sdr") {
            continue;
        }
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk_books(root, &path, depth + 1, out)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|e| BookKind::from_extension(e).is_some())
        {
            let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
            if canonical.starts_with(root) {
                out.push(canonical);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReaderUiState {
    pub books: Vec<BookEntry>,
    /// Open Books folder (transient); `None` shows the folders.
    pub folder: Option<String>,
    /// Cover, page count and reading position by book path (visible page only).
    pub meta: BTreeMap<PathBuf, BookMeta>,
    pub selected: usize,
    pub page: usize,
    pub current_title: String,
    pub current_page: usize,
    pub current_pages: usize,
}

impl ReaderUiState {
    pub fn set_books(&mut self, books: Vec<BookEntry>) {
        self.books = books;
        let keep: HashSet<&PathBuf> = self.books.iter().map(|b| &b.path).collect();
        self.meta.retain(|path, _| keep.contains(path));
        self.selected = self.selected.min(self.books.len().saturating_sub(1));
        if self
            .folder
            .as_ref()
            .is_some_and(|f| !self.books.iter().any(|b| &b.group == f))
        {
            self.folder = None;
        }
        self.page = self.page.min(self.page_count().saturating_sub(1));
    }

    /// Folders with their book counts: My files, Library, then the rest alphabetically.
    pub fn groups(&self) -> Vec<(String, usize)> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for b in &self.books {
            *counts.entry(b.group.as_str()).or_default() += 1;
        }
        let rank = |g: &str| match g {
            MY_FILES => 0,
            LIBRARY => 1,
            _ => 2,
        };
        let mut groups: Vec<(String, usize)> = counts
            .into_iter()
            .map(|(g, n)| (g.to_string(), n))
            .collect();
        groups.sort_by_key(|(g, _)| (rank(g), g.to_lowercase()));
        groups
    }

    /// What the grid lists: the open folder's books, else folders (or all books when there is
    /// only one folder).
    pub fn items(&self) -> Vec<BooksItem> {
        match &self.folder {
            Some(f) => (0..self.books.len())
                .filter(|i| &self.books[*i].group == f)
                .map(BooksItem::Book)
                .collect(),
            None => {
                let groups = self.groups();
                if groups.len() > 1 {
                    (0..groups.len()).map(BooksItem::Folder).collect()
                } else {
                    (0..self.books.len()).map(BooksItem::Book).collect()
                }
            }
        }
    }

    pub fn page_count(&self) -> usize {
        self.items().len().div_ceil(BOOKS_PER_PAGE).max(1)
    }

    /// Open folder `index` (into [`Self::groups`]) at its first page.
    pub fn open_folder(&mut self, index: usize) {
        if let Some((name, _)) = self.groups().get(index) {
            self.folder = Some(name.clone());
            self.page = 0;
        }
    }

    /// Back to the folders, on the page that shows the folder just left.
    pub fn close_folder(&mut self) {
        let left = self.folder.take();
        let groups = self.groups();
        self.page = left
            .and_then(|f| groups.iter().position(|(g, _)| *g == f))
            .map_or(0, |i| i / BOOKS_PER_PAGE);
    }

    pub fn open(&mut self, title: String, page: usize, pages: usize) {
        self.current_title = title;
        self.current_page = page;
        self.current_pages = pages;
    }

    pub fn update_page(&mut self, page: usize, pages: usize) {
        self.current_page = page;
        self.current_pages = pages;
    }
}

/// What the Books grid shows for one book (derived, replica-local).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookMeta {
    pub cover: Option<CoverImage>,
    pub pages: Option<usize>,
    /// Zero-based remembered page, when the book has been opened.
    pub position: Option<usize>,
}

impl BookMeta {
    fn detail(&self, kind: BookKind) -> String {
        match (self.position, self.pages) {
            (Some(p), Some(n)) if p > 0 => format!("Page {} of {n}", p + 1),
            (Some(p), None) if p > 0 => format!("{} · page {}", kind.label(), p + 1),
            (_, Some(1)) => "1 page".into(),
            (_, Some(n)) if kind == BookKind::Pdf => format!("{n} pages"),
            (_, Some(n)) => format!("{} · {n} pages", kind.label()),
            _ => kind.label().into(),
        }
    }

    fn progress(&self) -> Option<f32> {
        match (self.position, self.pages) {
            (Some(p), Some(n)) if p > 0 && n > 1 => Some((p + 1) as f32 / n as f32),
            _ => None,
        }
    }
}

/// One Books grid cell: a folder (index into `groups()`) or a book (index into `books`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooksItem {
    Folder(usize),
    Book(usize),
}

impl ReaderUiState {
    /// Items on the current grid page.
    pub fn visible_items(&self) -> Vec<BooksItem> {
        let items = self.items();
        let start = (self.page * BOOKS_PER_PAGE).min(items.len());
        let end = (start + BOOKS_PER_PAGE).min(items.len());
        items[start..end].to_vec()
    }

    /// Book indices on the current grid page.
    pub fn visible(&self) -> Vec<usize> {
        self.visible_items()
            .into_iter()
            .filter_map(|i| match i {
                BooksItem::Book(b) => Some(b),
                BooksItem::Folder(_) => None,
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooksHit {
    /// Notebooks tab: back to Home.
    Back,
    /// Door icon (top-left, as on Notebooks): leave the app.
    Exit,
    Open(usize),
    OpenFolder(usize),
    /// Inside a folder: back to the folders.
    CloseFolder,
    Previous,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BooksLayout {
    pub back: Rect,
    /// The "Notebooks" tab (goes back like `back`).
    pub notebooks_tab: Rect,
    /// `(book index, cell frame)` of the books on this page.
    pub cards: Vec<(usize, Rect)>,
    /// `(folder index, cell frame)` of the folders on this page.
    pub folders: Vec<(usize, Rect)>,
    pub cells: Vec<(BooksItem, lui::Cell)>,
    pub in_folder: bool,
    pub previous: Rect,
    pub next: Rect,
}

impl BooksLayout {
    pub fn new(geo: &FbGeometry, state: &ReaderUiState) -> Self {
        let cells: Vec<(BooksItem, lui::Cell)> = state
            .visible_items()
            .into_iter()
            .enumerate()
            .map(|(slot, item)| (item, lui::cell(geo, slot)))
            .collect();
        let (previous, _, next) = lui::pager(geo);
        let pick = |want_book: bool| {
            cells
                .iter()
                .filter_map(|(item, c)| match (item, want_book) {
                    (BooksItem::Book(i), true) | (BooksItem::Folder(i), false) => {
                        Some((*i, c.frame))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        Self {
            back: lui::left_control(),
            notebooks_tab: lui::tabs().0,
            cards: pick(true),
            folders: pick(false),
            cells,
            in_folder: state.folder.is_some(),
            previous,
            next,
        }
    }

    pub fn hit(&self, point: UiPoint, state: &ReaderUiState) -> Option<BooksHit> {
        if self.back.contains(point.x, point.y) {
            return Some(if self.in_folder {
                BooksHit::CloseFolder
            } else {
                BooksHit::Exit
            });
        }
        if !self.in_folder && self.notebooks_tab.contains(point.x, point.y) {
            return Some(BooksHit::Back);
        }
        if let Some((i, _)) = self
            .folders
            .iter()
            .find(|(_, r)| r.contains(point.x, point.y))
        {
            return Some(BooksHit::OpenFolder(*i));
        }
        if self.previous.contains(point.x, point.y) && state.page > 0 {
            return Some(BooksHit::Previous);
        }
        if self.next.contains(point.x, point.y) && state.page + 1 < state.page_count() {
            return Some(BooksHit::Next);
        }
        self.cards
            .iter()
            .find(|(_, r)| r.contains(point.x, point.y))
            .map(|(i, _)| BooksHit::Open(*i))
    }
}

pub fn render_books(fb: &mut [u8], geo: &FbGeometry, state: &ReaderUiState) -> Result<(), String> {
    let layout = BooksLayout::new(geo, state);
    let mut c = Canvas::new(fb, geo)?;
    c.fill_all(WHITE);
    lui::draw_left(
        &mut c,
        if state.folder.is_some() {
            LeftKind::Back
        } else {
            LeftKind::Exit
        },
    );
    let shown = match &state.folder {
        Some(f) => state.books.iter().filter(|b| &b.group == f).count(),
        None => state.books.len(),
    };
    let count = match shown {
        1 => "1 book".to_string(),
        n => format!("{n} books"),
    };
    let right = geo.xres.saturating_sub(lui::MARGIN);
    let left = match &state.folder {
        Some(f) => {
            let x = lui::left_control().right() + lui::BAR_GAP;
            c.sans_left(
                Rect::new(x, lui::BAR_Y, right.saturating_sub(x + 300), lui::BAR_H),
                58.0,
                f,
                BLACK,
                true,
            );
            right.saturating_sub(280)
        }
        None => {
            lui::draw_tabs(&mut c, true);
            lui::tabs().1.right() + 40
        }
    };
    let info = Rect::new(left, lui::BAR_Y, right.saturating_sub(left), lui::BAR_H);
    let w = Canvas::sans_width(34.0, &count, false).min(info.w);
    c.sans_left(
        Rect::new(info.right() - w, info.y, w, info.h),
        34.0,
        &count,
        lui::DARK,
        false,
    );
    lui::draw_divider(&mut c, geo);
    let empty = BookMeta::default();
    let groups = state.groups();
    for (item, cell) in &layout.cells {
        let index = match item {
            BooksItem::Folder(g) => {
                let (name, n) = &groups[*g];
                lui::draw_folder_cover(&mut c, cell, *n, ("book", "books"));
                lui::draw_caption(&mut c, cell, name, "Folder", false);
                continue;
            }
            BooksItem::Book(i) => i,
        };
        let book = &state.books[*index];
        let meta = state.meta.get(&book.path).unwrap_or(&empty);
        lui::draw_book_cover(
            &mut c,
            cell,
            meta.cover.as_ref(),
            &book.title,
            book.kind.label(),
            meta.progress(),
            *index == state.selected,
        );
        lui::draw_caption(&mut c, cell, &book.title, &meta.detail(book.kind), false);
    }
    if state.books.is_empty() {
        let r = Rect::new(120, lui::GRID_TOP + 300, geo.xres - 240, 120);
        c.sans_centered(r, 50.0, "No books yet", BLACK, true);
        c.sans_centered(
            Rect::new(r.x, r.bottom() + 10, r.w, 70),
            36.0,
            "Copy PDF, EPUB or CBZ files to the Kindle's documents folder.",
            lui::DARK,
            false,
        );
    }
    lui::draw_pager(&mut c, geo, state.page, state.page_count());
    Ok(())
}

/// Stable 64-bit FNV-1a (cache file names must not change between builds).
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn file_stamp(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(m) => {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            format!("{}:{t}", m.len())
        }
        Err(_) => "0:0".into(),
    }
}

const COVER_MAGIC: &str = "IGNITE_BOOK_COVER_v1";

fn read_cover_cache(
    path: &Path,
    stamp: &str,
    want: (u32, u32),
) -> Option<(Option<CoverImage>, usize)> {
    let raw = fs::read(path).ok()?;
    let mut parts = raw.splitn(4, |b| *b == b'\n');
    let magic = std::str::from_utf8(parts.next()?).ok()?;
    let fp = std::str::from_utf8(parts.next()?).ok()?;
    let dims = std::str::from_utf8(parts.next()?).ok()?;
    let pixels = parts.next()?;
    if magic != COVER_MAGIC || fp != stamp {
        return None;
    }
    let mut it = dims.split_whitespace().map(|v| v.parse::<u32>().ok());
    let (w, h, pages, bw, bh) = (
        it.next()??,
        it.next()??,
        it.next()??,
        it.next()??,
        it.next()??,
    );
    if (bw, bh) != want {
        return None;
    }
    if w == 0 || h == 0 {
        return Some((None, pages as usize));
    }
    (pixels.len() == (w * h) as usize).then(|| {
        (
            Some(CoverImage {
                w,
                h,
                pixels: pixels.to_vec(),
            }),
            pages as usize,
        )
    })
}

fn write_cover_cache(
    path: &Path,
    stamp: &str,
    want: (u32, u32),
    cover: Option<&CoverImage>,
    pages: usize,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let (w, h) = cover.map_or((0, 0), |c| (c.w, c.h));
    let tmp = path.with_extension("tmp");
    let mut f = File::create(&tmp).map_err(|e| e.to_string())?;
    write!(
        f,
        "{COVER_MAGIC}\n{stamp}\n{w} {h} {pages} {} {}\n",
        want.0, want.1
    )
    .map_err(|e| e.to_string())?;
    if let Some(c) = cover {
        f.write_all(&c.pixels).map_err(|e| e.to_string())?;
    }
    drop(f);
    fs::rename(tmp, path).map_err(|e| e.to_string())
}

/// Render page 1 of `path` to fit `w x h` (gray) and count its pages. Reflowable books return
/// 0 pages: counting them lays out the whole book, which waits until it is opened.
pub fn render_cover(path: &Path, w: u32, h: u32) -> Result<(CoverImage, usize), String> {
    let document = open_document(path)?;
    let pages = if document.is_reflowable().map_err(|e| e.to_string())? {
        0
    } else {
        let n = usize::try_from(document.page_count().map_err(|e| e.to_string())?)
            .map_err(|_| "invalid page count")?;
        if n == 0 {
            return Err("book has no pages".into());
        }
        n
    };
    let page = document.load_page(0).map_err(|e| e.to_string())?;
    let b = page.bounds().map_err(|e| e.to_string())?;
    let (pw, ph) = (b.x1 - b.x0, b.y1 - b.y0);
    if pw <= 0.0 || ph <= 0.0 {
        return Err("PDF page has invalid bounds".into());
    }
    let zoom = (w as f32 / pw).min(h as f32 / ph);
    let pixmap = page
        .to_pixmap(
            &Matrix::new_scale(zoom, zoom),
            &Colorspace::device_gray(),
            false,
            true,
        )
        .map_err(|e| format!("render cover: {e}"))?;
    let (cw, ch) = (pixmap.width().min(w), pixmap.height().min(h));
    let stride = pixmap.stride() as usize;
    let n = pixmap.n() as usize;
    let samples = pixmap.samples();
    let mut pixels = Vec::with_capacity((cw * ch) as usize);
    for y in 0..ch as usize {
        for x in 0..cw as usize {
            pixels.push(samples[y * stride + x * n]);
        }
    }
    Ok((
        CoverImage {
            w: cw,
            h: ch,
            pixels,
        },
        pages,
    ))
}

pub struct ReaderSession {
    document: Document,
    pub entry: BookEntry,
    page: usize,
    pages: usize,
}

impl ReaderSession {
    pub fn open(entry: BookEntry, remembered_page: usize) -> Result<Self, String> {
        let document = open_document(&entry.path)?;
        let pages = usize::try_from(document.page_count().map_err(|e| e.to_string())?)
            .map_err(|_| "invalid page count")?;
        if pages == 0 {
            return Err("book has no pages".into());
        }
        Ok(Self {
            document,
            entry,
            page: remembered_page.min(pages - 1),
            pages,
        })
    }

    pub fn page(&self) -> usize {
        self.page + 1
    }
    pub fn pages(&self) -> usize {
        self.pages
    }
    pub fn previous(&mut self) -> bool {
        if self.page == 0 {
            false
        } else {
            self.page -= 1;
            true
        }
    }
    pub fn next(&mut self) -> bool {
        if self.page + 1 >= self.pages {
            false
        } else {
            self.page += 1;
            true
        }
    }

    pub fn render(&mut self, fb: &mut [u8], geo: &FbGeometry) -> Result<RenderTiming, String> {
        let t0 = Instant::now();
        let page = self
            .document
            .load_page(self.page as i32)
            .map_err(|e| format!("load PDF page: {e}"))?;
        let bounds = page.bounds().map_err(|e| e.to_string())?;
        let pw = bounds.x1 - bounds.x0;
        let ph = bounds.y1 - bounds.y0;
        if pw <= 0.0 || ph <= 0.0 {
            return Err("PDF page has invalid bounds".into());
        }
        let box_w = geo.xres.saturating_sub(2 * PAGE_MARGIN);
        let box_h = SAFE_BOTTOM - SAFE_TOP;
        let zoom = (box_w as f32 / pw).min(box_h as f32 / ph);
        let pixmap = page
            .to_pixmap(
                &Matrix::new_scale(zoom, zoom),
                &Colorspace::device_gray(),
                false,
                true,
            )
            .map_err(|e| format!("render PDF page: {e}"))?;
        let render_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let mut canvas = Canvas::new(fb, geo)?;
        canvas.fill_all(WHITE);
        drop(canvas);
        let width = pixmap.width().min(box_w);
        let height = pixmap.height().min(box_h);
        let x0 = (geo.xres - width) / 2;
        let samples = pixmap.samples();
        let stride = pixmap.stride() as usize;
        let channels = pixmap.n() as usize;
        for y in 0..height as usize {
            let dst = (SAFE_TOP as usize + y) * geo.line_length as usize + x0 as usize;
            let src = y * stride;
            for x in 0..width as usize {
                fb[dst + x] = samples[src + x * channels];
            }
        }
        render_reader_chrome(fb, geo, &self.entry.title, self.page(), self.pages())?;
        Ok(RenderTiming {
            render_ms,
            pixmap_width: pixmap.width(),
            pixmap_height: pixmap.height(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderTiming {
    pub render_ms: f64,
    pub pixmap_width: u32,
    pub pixmap_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderHit {
    Books,
    Previous,
    Next,
}

pub fn reader_hit(
    geo: &FbGeometry,
    point: UiPoint,
    page: usize,
    pages: usize,
) -> Option<ReaderHit> {
    if Rect::new(0, 0, 330, SAFE_TOP).contains(point.x, point.y) {
        return Some(ReaderHit::Books);
    }
    if Rect::new(0, SAFE_BOTTOM, 380, geo.yres - SAFE_BOTTOM).contains(point.x, point.y) && page > 1
    {
        return Some(ReaderHit::Previous);
    }
    if Rect::new(geo.xres - 380, SAFE_BOTTOM, 380, geo.yres - SAFE_BOTTOM)
        .contains(point.x, point.y)
        && page < pages
    {
        return Some(ReaderHit::Next);
    }
    if point.y >= SAFE_TOP && point.y < SAFE_BOTTOM {
        if point.x < geo.xres * 35 / 100 && page > 1 {
            return Some(ReaderHit::Previous);
        }
        if point.x > geo.xres * 65 / 100 && page < pages {
            return Some(ReaderHit::Next);
        }
    }
    None
}

fn render_reader_chrome(
    fb: &mut [u8],
    geo: &FbGeometry,
    title: &str,
    page: usize,
    pages: usize,
) -> Result<(), String> {
    let mut c = Canvas::new(fb, geo)?;
    c.fill(Rect::new(0, 0, geo.xres, SAFE_TOP), WHITE);
    c.fill(
        Rect::new(0, SAFE_BOTTOM, geo.xres, geo.yres - SAFE_BOTTOM),
        WHITE,
    );
    c.fill(Rect::new(0, SAFE_TOP - 4, geo.xres, 4), BLACK);
    c.fill(Rect::new(0, SAFE_BOTTOM, geo.xres, 4), BLACK);
    c.line(28.0, 70.0, 50.0, 48.0, 6.0, BLACK);
    c.line(28.0, 70.0, 50.0, 92.0, 6.0, BLACK);
    c.sans_left(Rect::new(62, 18, 220, 104), 34.0, "Books", BLACK, true);
    c.sans_centered(
        Rect::new(330, 12, geo.xres - 660, 70),
        40.0,
        title,
        BLACK,
        true,
    );
    c.sans_centered(
        Rect::new(500, 78, geo.xres - 1000, 42),
        27.0,
        &format!("Page {page} of {pages}"),
        GRAY,
        false,
    );
    if page > 1 {
        c.sans_centered(
            Rect::new(24, SAFE_BOTTOM + 18, 340, 100),
            32.0,
            &format!("‹ Page {}", page - 1),
            BLACK,
            true,
        );
    }
    if page < pages {
        c.sans_centered(
            Rect::new(geo.xres - 364, SAFE_BOTTOM + 18, 340, 100),
            32.0,
            &format!("Page {} ›", page + 1),
            BLACK,
            true,
        );
    }
    c.sans_centered(
        Rect::new(700, SAFE_BOTTOM + 18, 460, 100),
        30.0,
        &format!("{page} / {pages}"),
        BLACK,
        true,
    );
    Ok(())
}

#[derive(Default, Serialize, Deserialize)]
struct PositionFile {
    v: u32,
    positions: BTreeMap<String, usize>,
    /// Page counts learned when a book was opened (reflowable books have none before).
    #[serde(default)]
    pages: BTreeMap<String, usize>,
}

pub struct ReaderRuntime {
    pub session: Option<ReaderSession>,
    positions_path: PathBuf,
    covers_dir: PathBuf,
    positions: PositionFile,
}

impl ReaderRuntime {
    pub fn new(positions_path: PathBuf) -> Self {
        let positions = fs::read(&positions_path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<PositionFile>(&raw).ok())
            .filter(|file| file.v == POSITIONS_VERSION)
            .unwrap_or_else(|| PositionFile {
                v: POSITIONS_VERSION,
                positions: BTreeMap::new(),
                pages: BTreeMap::new(),
            });
        let covers_dir = positions_path.with_file_name(COVERS_DIR);
        Self {
            session: None,
            positions_path,
            covers_dir,
            positions,
        }
    }

    pub fn open(&mut self, entry: BookEntry) -> Result<(usize, usize), String> {
        let key = entry.path.to_string_lossy().into_owned();
        let remembered = self.positions.positions.get(&key).copied().unwrap_or(0);
        let session = ReaderSession::open(entry, remembered)?;
        let page = session.page();
        let pages = session.pages();
        self.session = Some(session);
        if let Err(error) = self.save() {
            eprintln!("reader_position_save_failed detail={error}");
        }
        Ok((page, pages))
    }

    pub fn close(&mut self) {
        self.session = None;
    }

    /// Fill covers, page counts and positions for the visible grid page. Covers come from the
    /// on-disk cache (keyed by path, size and mtime) or are rendered once with MuPDF.
    pub fn prepare_library(&self, state: &mut ReaderUiState, geo: &FbGeometry) {
        let want = lui::cover_size(geo);
        let t0 = Instant::now();
        let mut rendered = 0;
        for index in state.visible() {
            let book = state.books[index].clone();
            let key = book.path.to_string_lossy().into_owned();
            let position = self.positions.positions.get(&key).copied();
            let known_pages = self.positions.pages.get(&key).copied();
            if let Some(meta) = state.meta.get_mut(&book.path) {
                meta.position = position;
                meta.pages = meta.pages.or(known_pages);
                continue;
            }
            let stamp = file_stamp(&book.path);
            let cache = self.covers_dir.join(format!("{:016x}.cover", fnv1a(&key)));
            let (cover, pages) = match read_cover_cache(&cache, &stamp, want) {
                Some((cover, pages)) => (cover, (pages > 0).then_some(pages)),
                None => {
                    rendered += 1;
                    let made = render_cover(&book.path, want.0, want.1);
                    let (cover, pages) = match made {
                        Ok((cover, pages)) => (Some(cover), pages),
                        Err(error) => {
                            eprintln!("reader_cover_failed file={:?} detail={error}", book.path);
                            (None, 0)
                        }
                    };
                    if let Err(error) =
                        write_cover_cache(&cache, &stamp, want, cover.as_ref(), pages)
                    {
                        eprintln!("reader_cover_cache_failed detail={error}");
                    }
                    (cover, (pages > 0).then_some(pages))
                }
            };
            let pages = pages.or(known_pages);
            state.meta.insert(
                book.path.clone(),
                BookMeta {
                    cover,
                    pages,
                    position,
                },
            );
        }
        println!(
            "reader_covers visible={} rendered={rendered} ms={}",
            state.visible().len(),
            t0.elapsed().as_millis()
        );
    }

    pub fn turn(&mut self, next: bool) -> Result<bool, String> {
        let Some(session) = self.session.as_mut() else {
            return Err("reader has no open book".into());
        };
        let moved = if next {
            session.next()
        } else {
            session.previous()
        };
        if moved {
            if let Err(error) = self.save() {
                eprintln!("reader_position_save_failed detail={error}");
            }
        }
        Ok(moved)
    }

    pub fn page_state(&self) -> Option<(usize, usize)> {
        self.session.as_ref().map(|s| (s.page(), s.pages()))
    }

    fn save(&mut self) -> Result<(), String> {
        let Some(session) = self.session.as_ref() else {
            return Ok(());
        };
        let key = session.entry.path.to_string_lossy().into_owned();
        self.positions.pages.insert(key.clone(), session.pages);
        self.positions.positions.insert(key, session.page);
        if let Some(parent) = self.positions_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = self.positions_path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.positions).map_err(|e| e.to_string())?;
        let mut file = File::create(&tmp).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(tmp, &self.positions_path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_ui::{UiCommand, UiState};
    use crate::home::{HomeHit, HomeLayout};
    use crate::touch::{classify_origin, FingerOrigin};

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

    #[test]
    fn scan_finds_pdf_only_and_ignores_hidden_and_symlinked_content() {
        let root = std::env::temp_dir().join(format!("ignite-reader-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::create_dir_all(root.join(".cache")).unwrap();
        fs::write(root.join("A.pdf"), b"pdf").unwrap();
        fs::write(root.join("folder/B.PDF"), b"pdf").unwrap();
        fs::write(root.join("folder/C - Author (12).epub"), b"epub").unwrap();
        fs::write(root.join("folder/D.CBZ"), b"cbz").unwrap();
        fs::write(root.join("folder/no.txt"), b"txt").unwrap();
        fs::write(root.join("folder/no.cbr"), b"cbr").unwrap();
        fs::write(root.join(".cache/hidden.pdf"), b"pdf").unwrap();
        fs::write(root.join("E - Author (3).epub"), b"epub").unwrap();
        let books = scan_books_in(&[(root.clone(), false)]).unwrap();
        assert_eq!(
            books
                .iter()
                .map(|b| (b.title.as_str(), b.kind, b.group.as_str()))
                .collect::<Vec<_>>(),
            [
                ("A", BookKind::Pdf, MY_FILES),
                ("B", BookKind::Pdf, "folder"),
                ("C - Author", BookKind::Epub, "folder"),
                ("D", BookKind::Comic, "folder"),
                ("E - Author", BookKind::Epub, LIBRARY),
            ]
        );
        // Ignite's own books folder is always My files, Calibre-style name or not.
        let mine = scan_books_in(&[(root.clone(), true)]).unwrap();
        assert_eq!(
            mine.iter().find(|b| b.title == "E - Author").unwrap().group,
            MY_FILES
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn titles_come_from_file_names_without_library_noise() {
        assert_eq!(
            clean_title("Dune - Frank Herbert (87)"),
            "Dune - Frank Herbert"
        );
        assert_eq!(
            clean_title("4-Hour Body_ An Uncommon Guide - Timothy Ferriss (22)"),
            "4-Hour Body: An Uncommon Guide - Timothy Ferriss"
        );
        assert_eq!(
            clean_title("Long-reads-culture-history-music-AI-future"),
            "Long reads culture history music AI future"
        );
        assert_eq!(clean_title("Nausicaa_v01"), "Nausicaa v01");
        assert_eq!(clean_title("Report (draft)"), "Report (draft)");
        assert_eq!(clean_title("  "), "Untitled");
    }

    #[test]
    fn detail_line_names_the_format_until_pages_are_known() {
        let m = BookMeta::default();
        assert_eq!(m.detail(BookKind::Epub), "EPUB");
        let m = BookMeta {
            pages: Some(300),
            ..BookMeta::default()
        };
        assert_eq!(m.detail(BookKind::Pdf), "300 pages");
        assert_eq!(m.detail(BookKind::Epub), "EPUB · 300 pages");
        let m = BookMeta {
            position: Some(9),
            ..BookMeta::default()
        };
        assert_eq!(m.detail(BookKind::Epub), "EPUB · page 10");
        let m = BookMeta {
            position: Some(9),
            pages: Some(300),
            ..BookMeta::default()
        };
        assert_eq!(m.detail(BookKind::Epub), "Page 10 of 300");
    }

    /// `READER_PREVIEW_EPUB=/path/book.epub`: lays it out, renders cover + page 3 as PGMs.
    #[test]
    #[ignore]
    fn dump_reader_epub_preview() {
        let dir = std::env::var("READER_PREVIEW_DIR").unwrap_or_else(|_| "/tmp".into());
        let book =
            PathBuf::from(std::env::var("READER_PREVIEW_EPUB").expect("READER_PREVIEW_EPUB"));
        let g = geo();
        let t0 = Instant::now();
        let (cover, pages) = render_cover(&book, 392, 523).unwrap();
        println!(
            "cover {}x{} pages={pages} ms={}",
            cover.w,
            cover.h,
            t0.elapsed().as_millis()
        );
        let t1 = Instant::now();
        let mut session = ReaderSession::open(BookEntry::from_path(book), 0).unwrap();
        println!(
            "open pages={} ms={}",
            session.pages(),
            t1.elapsed().as_millis()
        );
        for n in 0..3 {
            let mut fb = vec![WHITE; g.smem_len as usize];
            session.render(&mut fb, &g).unwrap();
            write_pgm(
                Path::new(&dir).join(format!("epub-page{}.pgm", n + 1)),
                &fb,
                &g,
            );
            session.next();
        }
        let mut state = ReaderUiState::default();
        state.set_books(vec![session.entry.clone()]);
        state.meta.insert(
            session.entry.path.clone(),
            BookMeta {
                cover: Some(cover),
                pages: Some(session.pages()),
                position: Some(3),
            },
        );
        let mut fb = vec![WHITE; g.smem_len as usize];
        render_books(&mut fb, &g, &state).unwrap();
        write_pgm(Path::new(&dir).join("epub-books.pgm"), &fb, &g);
    }

    #[test]
    fn reader_taps_never_turn_from_the_chrome_or_center_content() {
        let g = geo();
        assert_eq!(
            reader_hit(&g, UiPoint { x: 20, y: 70 }, 2, 4),
            Some(ReaderHit::Books)
        );
        assert_eq!(reader_hit(&g, UiPoint { x: 900, y: 900 }, 2, 4), None);
        assert_eq!(
            reader_hit(&g, UiPoint { x: 1700, y: 900 }, 2, 4),
            Some(ReaderHit::Next)
        );
        assert_eq!(
            reader_hit(&g, UiPoint { x: 100, y: 900 }, 2, 4),
            Some(ReaderHit::Previous)
        );
    }

    #[test]
    fn home_books_button_opens_books_and_reader_content_never_becomes_lasso_content() {
        let g = geo();
        let mut ui = UiState::launch_home(0, 0);
        let home = HomeLayout::new(&g, &ui.home, 0);
        let point = UiPoint {
            x: home.books_button.x + home.books_button.w / 2,
            y: home.books_button.y + home.books_button.h / 2,
        };
        assert_eq!(home.hit(point, &ui.home, 0), Some(HomeHit::Books));
        assert_eq!(
            ui.on_tap(&g, point, &[], "Notebook 1", None),
            UiCommand::OpenBooks
        );
        ui.enter_reader("Book".into(), 1, 3);
        assert_eq!(
            classify_origin(&ui, &g, UiPoint { x: 900, y: 800 }),
            FingerOrigin::ReaderContent
        );
    }

    #[test]
    fn books_grid_shares_the_library_header_and_opens_by_cell() {
        let g = geo();
        let mut state = ReaderUiState::default();
        state.set_books(
            (1..=14)
                .map(|n| BookEntry {
                    path: format!("/b{n}.pdf").into(),
                    title: format!("Book {n}"),
                    kind: BookKind::Pdf,
                    group: LIBRARY.into(),
                })
                .collect(),
        );
        let layout = BooksLayout::new(&g, &state);
        assert_eq!(layout.cards.len(), lui::ITEMS_PER_PAGE);
        let center = |r: Rect| UiPoint {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        };
        assert_eq!(
            layout.hit(center(layout.back), &state),
            Some(BooksHit::Exit)
        );
        assert_eq!(
            layout.hit(center(lui::tabs().0), &state),
            Some(BooksHit::Back)
        );
        assert_eq!(
            layout.hit(center(layout.cards[5].1), &state),
            Some(BooksHit::Open(5))
        );
        assert_eq!(
            layout.hit(center(layout.next), &state),
            Some(BooksHit::Next)
        );
        assert_eq!(layout.hit(center(layout.previous), &state), None);
        // Home's Books tab and the Books screen's tab are the same rect.
        let home = HomeLayout::new(&g, &crate::home::HomeState::selecting(0, 1), 1);
        assert_eq!(home.books_button, lui::tabs().1);
        let mut fb = vec![0x77; g.smem_len as usize];
        render_books(&mut fb, &g, &state).unwrap();
        assert!(fb[..g.line_length as usize * 300].contains(&BLACK));
    }

    #[test]
    fn books_group_into_folders_and_folders_open_and_close() {
        let g = geo();
        let mut state = ReaderUiState::default();
        let book = |n: usize, group: &str| BookEntry {
            path: format!("/b{n}.epub").into(),
            title: format!("Book {n}"),
            kind: BookKind::Epub,
            group: group.into(),
        };
        let mut books: Vec<BookEntry> = (0..20).map(|n| book(n, LIBRARY)).collect();
        books.push(book(20, "Comics"));
        books.push(book(21, MY_FILES));
        state.set_books(books);
        assert_eq!(
            state.groups(),
            [
                (MY_FILES.into(), 1),
                (LIBRARY.into(), 20),
                ("Comics".into(), 1)
            ]
        );
        // Top level: three folder tiles, one page.
        let layout = BooksLayout::new(&g, &state);
        assert_eq!(layout.folders.len(), 3);
        assert!(layout.cards.is_empty());
        let center = |r: Rect| UiPoint {
            x: r.x + r.w / 2,
            y: r.y + r.h / 2,
        };
        assert_eq!(
            layout.hit(center(layout.folders[1].1), &state),
            Some(BooksHit::OpenFolder(1))
        );
        // Library: 20 books over two pages; back closes the folder, not Books.
        state.open_folder(1);
        assert_eq!(state.folder.as_deref(), Some(LIBRARY));
        assert_eq!(state.page_count(), 2);
        let layout = BooksLayout::new(&g, &state);
        assert_eq!(layout.cards.len(), lui::ITEMS_PER_PAGE);
        assert_eq!(
            layout.hit(center(layout.back), &state),
            Some(BooksHit::CloseFolder)
        );
        assert_eq!(
            layout.hit(center(layout.cards[0].1), &state),
            Some(BooksHit::Open(0))
        );
        state.close_folder();
        assert_eq!(state.folder, None);
        let layout = BooksLayout::new(&g, &state);
        assert_eq!(
            layout.hit(center(layout.back), &state),
            Some(BooksHit::Exit)
        );
        // A rescan without that folder falls back to the folder list.
        state.open_folder(2);
        state.set_books((0..3).map(|n| book(n, LIBRARY)).collect());
        assert_eq!(state.folder, None);
        assert_eq!(
            state.items().len(),
            3,
            "one folder only: books are listed directly"
        );
    }

    #[test]
    fn cover_cache_round_trips_and_rejects_stale_stamps() {
        let dir = std::env::temp_dir().join(format!("ignite-cover-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("x.cover");
        let img = CoverImage {
            w: 3,
            h: 2,
            pixels: vec![1, 2, 3, 4, 5, 6],
        };
        write_cover_cache(&path, "10:20", (40, 50), Some(&img), 7).unwrap();
        assert_eq!(
            read_cover_cache(&path, "10:20", (40, 50)),
            Some((Some(img), 7))
        );
        assert_eq!(read_cover_cache(&path, "10:21", (40, 50)), None);
        assert_eq!(read_cover_cache(&path, "10:20", (41, 50)), None);
        write_cover_cache(&path, "1:1", (40, 50), None, 0).unwrap();
        assert_eq!(read_cover_cache(&path, "1:1", (40, 50)), Some((None, 0)));
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
        let _ = fs::remove_dir_all(dir);
    }

    /// `READER_PREVIEW_DIR=/tmp READER_PREVIEW_PDF=/path/file.pdf cargo test -p scribe-app
    /// --lib --features reader dump_reader_ -- --ignored`
    #[test]
    #[ignore]
    fn dump_reader_library_preview() {
        let dir = std::env::var("READER_PREVIEW_DIR").unwrap_or_else(|_| "/tmp".into());
        let g = geo();
        let mut state = ReaderUiState::default();
        let titles = [
            "The Pragmatic Programmer",
            "Atomic Habits",
            "Rust in Action",
            "Deep Work",
            "A very long technical manual title that wraps",
            "Kindle Scribe User Guide",
            "Notes on Design",
            "Poems",
        ];
        state.set_books(
            titles
                .iter()
                .enumerate()
                .map(|(n, t)| BookEntry {
                    path: format!("/book-{n}.pdf").into(),
                    title: (*t).into(),
                    kind: if n % 3 == 2 {
                        BookKind::Epub
                    } else {
                        BookKind::Pdf
                    },
                    group: if n < 2 {
                        MY_FILES
                    } else if n < 6 {
                        LIBRARY
                    } else {
                        "Comics"
                    }
                    .into(),
                })
                .collect(),
        );
        let (w, h) = lui::cover_size(&g);
        for (n, book) in state.books.clone().iter().enumerate() {
            if n % 3 == 2 {
                continue; // no cover: typeset fallback
            }
            let (cw, ch) = if n == 4 {
                (w, w * 3 / 4)
            } else {
                (w, h * 97 / 100)
            };
            let mut px = vec![WHITE; (cw * ch) as usize];
            for y in 0..ch {
                for x in 0..cw {
                    let band = y < ch / 3;
                    let v = if band {
                        0x30 + ((x + n as u32 * 40) % 200) as u8 / 4
                    } else if (y / 18) % 3 == 0 && x > 30 && x < cw - 30 && y > ch / 2 {
                        0x60
                    } else {
                        WHITE
                    };
                    px[(y * cw + x) as usize] = v;
                }
            }
            state.meta.insert(
                book.path.clone(),
                BookMeta {
                    cover: Some(CoverImage {
                        w: cw,
                        h: ch,
                        pixels: px,
                    }),
                    pages: Some(120 + n * 31),
                    position: (n % 2 == 0).then_some(10 + n * 9),
                },
            );
        }
        let mut fb = vec![WHITE; g.smem_len as usize];
        render_books(&mut fb, &g, &state).unwrap();
        write_pgm(Path::new(&dir).join("reader-books-folders.pgm"), &fb, &g);
        state.open_folder(1);
        render_books(&mut fb, &g, &state).unwrap();
        write_pgm(Path::new(&dir).join("reader-books.pgm"), &fb, &g);
        let mut empty = ReaderUiState::default();
        empty.set_books(Vec::new());
        render_books(&mut fb, &g, &empty).unwrap();
        write_pgm(Path::new(&dir).join("reader-books-empty.pgm"), &fb, &g);
    }

    #[test]
    #[ignore]
    fn dump_reader_page_preview() {
        let dir = std::env::var("READER_PREVIEW_DIR").unwrap_or_else(|_| "/tmp".into());
        let pdf = std::env::var("READER_PREVIEW_PDF").expect("READER_PREVIEW_PDF");
        let g = geo();
        let mut session = ReaderSession::open(BookEntry::from_path(PathBuf::from(pdf)), 0).unwrap();
        let mut fb = vec![WHITE; g.smem_len as usize];
        session.render(&mut fb, &g).unwrap();
        write_pgm(Path::new(&dir).join("reader-page.pgm"), &fb, &g);
    }

    #[test]
    #[ignore]
    fn reader_position_round_trip_with_pdf() {
        let pdf = PathBuf::from(std::env::var("READER_PREVIEW_PDF").expect("READER_PREVIEW_PDF"));
        let dir =
            std::env::temp_dir().join(format!("ignite-reader-position-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let positions = dir.join(POSITIONS_FILE);
        let entry = BookEntry::from_path(pdf);

        let mut first = ReaderRuntime::new(positions.clone());
        let (_, pages) = first.open(entry.clone()).unwrap();
        assert!(pages >= 2, "position test needs a multi-page PDF");
        assert!(first.turn(true).unwrap());
        assert_eq!(first.page_state(), Some((2, pages)));
        drop(first);

        let mut reopened = ReaderRuntime::new(positions);
        assert_eq!(reopened.open(entry).unwrap(), (2, pages));
        let _ = fs::remove_dir_all(dir);
    }

    fn write_pgm(path: PathBuf, fb: &[u8], geo: &FbGeometry) {
        let mut out = format!("P5\n{} {}\n255\n", geo.xres, geo.yres).into_bytes();
        for y in 0..geo.yres as usize {
            let row = y * geo.line_length as usize;
            out.extend_from_slice(&fb[row..row + geo.xres as usize]);
        }
        fs::write(path, out).unwrap();
    }
}
