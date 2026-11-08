//! Lazy, replica-local page thumbnails for the link-target picker.
//!
//! Derived cache only: never snapshot authority, never `background.raw`. Generation is
//! proportional to the visible grid, not to every page in the notebook.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::SystemTime;

use notebook_core::{Stroke, CANONICAL_HEIGHT, CANONICAL_WIDTH};

use crate::store::{load_page_journals_read_only, load_page_texts, page_dir};
use crate::text_paint::paint_texts_8bpp;
use crate::ui_gfx::WHITE;

pub const THUMB_W: u32 = 186;
pub const THUMB_H: u32 = 248;
const CACHE_NAME: &str = ".scribe-thumb";
/// Library cover cache (first page at grid size).
const COVER_CACHE_NAME: &str = ".scribe-cover";

fn file_stamp(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(m) => {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}:{}", m.len(), t)
        }
        Err(_) => "0:0".into(),
    }
}

fn fingerprint(dir: &Path) -> String {
    let mut s = String::new();
    for name in [
        "page.meta",
        "strokes.jsonl",
        "edits.jsonl",
        "texts.jsonl",
        "links.jsonl",
        "background.raw",
    ] {
        s.push_str(name);
        s.push('=');
        s.push_str(&file_stamp(&dir.join(name)));
        s.push(';');
    }
    s
}

#[derive(Clone, Copy)]
struct Size {
    w: u32,
    h: u32,
    cache: &'static str,
    /// Stroke dot size in pixels.
    pen: u32,
}

const THUMB: Size = Size {
    w: THUMB_W,
    h: THUMB_H,
    cache: CACHE_NAME,
    pen: 1,
};

fn write_cache(dir: &Path, fp: &str, pixels: &[u8], size: Size) -> Result<(), String> {
    let (tw, th) = (size.w, size.h);
    let tmp = dir.join(format!("{}.tmp", size.cache));
    {
        let mut f = File::create(&tmp).map_err(|e| e.to_string())?;
        writeln!(f, "SCRIBE_THUMB_v1").map_err(|e| e.to_string())?;
        writeln!(f, "{fp}").map_err(|e| e.to_string())?;
        writeln!(f, "{tw} {th}").map_err(|e| e.to_string())?;
        f.write_all(pixels).map_err(|e| e.to_string())?;
    }
    let dest = dir.join(size.cache);
    #[cfg(windows)]
    if dest.exists() {
        let _ = fs::remove_file(&dest);
    }
    fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

fn read_cache(dir: &Path, fp: &str, size: Size) -> Option<Vec<u8>> {
    let (tw, th) = (size.w, size.h);
    let mut f = File::open(dir.join(size.cache)).ok()?;
    let mut header = String::new();
    let mut buf = [0u8; 1];
    // Read three newline-terminated header lines.
    let mut lines = Vec::new();
    for _ in 0..3 {
        header.clear();
        loop {
            f.read_exact(&mut buf).ok()?;
            if buf[0] == b'\n' {
                break;
            }
            header.push(buf[0] as char);
        }
        lines.push(header.clone());
    }
    if lines[0] != "SCRIBE_THUMB_v1" || lines[1] != fp {
        return None;
    }
    let mut dims = lines[2].split_whitespace();
    let w: u32 = dims.next()?.parse().ok()?;
    let h: u32 = dims.next()?.parse().ok()?;
    if w != tw || h != th {
        return None;
    }
    let mut pixels = vec![0u8; (tw * th) as usize];
    f.read_exact(&mut pixels).ok()?;
    Some(pixels)
}

fn subsample_background(full: &[u8], fw: u32, fh: u32, size: Size) -> Vec<u8> {
    let (tw, th) = (size.w, size.h);
    let mut out = vec![WHITE; (tw * th) as usize];
    if fw == 0 || fh == 0 || full.is_empty() {
        return out;
    }
    for y in 0..th {
        let sy = y * fh / th;
        for x in 0..tw {
            let sx = x * fw / tw;
            let i = (sy * fw + sx) as usize;
            if i < full.len() {
                out[(y * tw + x) as usize] = full[i];
            }
        }
    }
    out
}

fn draw_strokes(out: &mut [u8], strokes: &[Stroke], size: Size) {
    for s in strokes {
        let pts = &s.points;
        if pts.is_empty() {
            continue;
        }
        let mut prev = pts[0];
        for p in pts.iter().skip(1) {
            draw_line(out, prev.x, prev.y, p.x, p.y, size);
            prev = *p;
        }
        plot(out, prev.x, prev.y, size);
    }
}

fn plot(out: &mut [u8], x: f32, y: f32, size: Size) {
    let (tw, th) = (size.w, size.h);
    let px = (x / CANONICAL_WIDTH * tw as f32).round() as i32;
    let py = (y / CANONICAL_HEIGHT * th as f32).round() as i32;
    if px < 0 || py < 0 {
        return;
    }
    let (px, py) = (px as u32, py as u32);
    if px >= tw || py >= th {
        return;
    }
    for dy in 0..size.pen {
        for dx in 0..size.pen {
            if px + dx < tw && py + dy < th {
                out[((py + dy) * tw + px + dx) as usize] = 0x00;
            }
        }
    }
}

fn draw_line(out: &mut [u8], x0: f32, y0: f32, x1: f32, y1: f32, size: Size) {
    let step = (CANONICAL_WIDTH / size.w as f32 * 0.5).min(40.0);
    let n = ((x1 - x0).abs().max((y1 - y0).abs()) / step)
        .ceil()
        .max(1.0) as i32;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        plot(out, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, size);
    }
}

/// Load or build a thumbnail for one page. Missing journals yield plain paper.
pub fn load_or_build(notebook_dir: &Path, page_local: &str, panel_w: u32, panel_h: u32) -> Vec<u8> {
    load_or_build_sized(notebook_dir, page_local, panel_w, panel_h, THUMB)
}

/// Library cover (`w x h`, normally `library_ui::cover_size`) for one page, cached beside the
/// picker thumbnail. Strokes are drawn two pixels wide so handwriting stays legible.
pub fn load_or_build_cover(
    notebook_dir: &Path,
    page_local: &str,
    panel_w: u32,
    panel_h: u32,
    w: u32,
    h: u32,
) -> Vec<u8> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    /// Covers already decoded this session, by page dir (Home redraws often: menus, paging).
    static MEMO: Mutex<Option<HashMap<std::path::PathBuf, (String, u32, u32, Vec<u8>)>>> =
        Mutex::new(None);
    let size = Size {
        w,
        h,
        cache: COVER_CACHE_NAME,
        pen: 2,
    };
    let dir = page_dir(notebook_dir, page_local);
    let fp = fingerprint(&dir);
    if let Ok(memo) = MEMO.lock() {
        if let Some((f, mw, mh, px)) = memo.as_ref().and_then(|m| m.get(&dir)) {
            if *f == fp && (*mw, *mh) == (w, h) {
                return px.clone();
            }
        }
    }
    let pixels = load_or_build_sized(notebook_dir, page_local, panel_w, panel_h, size);
    if let Ok(mut memo) = MEMO.lock() {
        let m = memo.get_or_insert_with(HashMap::new);
        if m.len() >= 48 {
            m.clear();
        }
        m.insert(dir, (fp, w, h, pixels.clone()));
    }
    pixels
}

fn load_or_build_sized(
    notebook_dir: &Path,
    page_local: &str,
    panel_w: u32,
    panel_h: u32,
    size: Size,
) -> Vec<u8> {
    let dir = page_dir(notebook_dir, page_local);
    let fp = fingerprint(&dir);
    if let Some(cached) = read_cache(&dir, &fp, size) {
        return cached;
    }
    let mut pixels = vec![WHITE; (size.w * size.h) as usize];
    let bg_path = dir.join("background.raw");
    if let Ok(raw) = fs::read(&bg_path) {
        let packed = (panel_w as usize).saturating_mul(panel_h as usize);
        if raw.len() == packed {
            pixels = subsample_background(&raw, panel_w, panel_h, size);
        }
    }
    if let Ok(journals) = load_page_journals_read_only(notebook_dir, page_local) {
        draw_strokes(&mut pixels, &journals.strokes, size);
    }
    if let Ok(texts) = load_page_texts(&dir) {
        paint_texts_8bpp(&mut pixels, size.w, size.h, &texts);
    }
    let _ = write_cache(&dir, &fp, &pixels, size);
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsample_is_thumb_sized() {
        let full = vec![0x80u8; 1860 * 2480];
        let t = subsample_background(&full, 1860, 2480, THUMB);
        assert_eq!(t.len(), (THUMB_W * THUMB_H) as usize);
        assert!(t.iter().all(|b| *b == 0x80));
    }

    #[test]
    fn text_only_page_thumbnail_is_not_blank() {
        use sync_protocol::{TextFace, TextId};
        use uuid::Uuid;

        let dir = std::env::temp_dir().join(format!(
            "scribe-thumb-text-{}-{}",
            std::process::id(),
            "only"
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let page = dir.join("page-1");
        fs::create_dir_all(&page).unwrap();
        let mut b = [0u8; 16];
        b[0] = 0x01;
        b[6] = 0x70;
        b[8] = 0x80;
        b[15] = 1;
        let text = sync_protocol::SyncText {
            text_id: TextId::from_uuid(Uuid::from_bytes(b)).unwrap(),
            x: 200.0,
            y: 400.0,
            width: 8000.0,
            font_size: 340.0,
            face: TextFace::DefaultSansV1,
            content: "Hello".into(),
        };
        crate::store::write_page_texts(&page, std::slice::from_ref(&text)).unwrap();
        let pixels = load_or_build(&dir, "page-1", 1860, 2480);
        assert!(
            pixels.iter().any(|p| *p != WHITE),
            "typed text must darken the thumbnail"
        );
        let cached = load_or_build(&dir, "page-1", 1860, 2480);
        assert_eq!(cached, pixels);
        let _ = fs::remove_dir_all(&dir);
    }
}
