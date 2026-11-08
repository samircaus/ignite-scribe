//! Procedural gray8 page backgrounds.
//!
//! Wire encoding is `0x00` black … `0xff` white (see `sync_protocol::BackgroundEncoding`); the
//! Kindle applies its own panel polarity when materializing. These patterns are light grays so
//! ink stays dominant on E Ink. **Not yet confirmed on a physical Kindle** (only blank pages are
//! part of the proof of concept); ignite-web renders the same blobs for a host-side check.

use sync_protocol::{sha256_hex, BackgroundEncoding, BackgroundFormat, BackgroundRef, Sha256Hex};

use crate::spec::Paper;

/// Scribe 1st-gen panel (matches `notebook_core::PANEL_WIDTH_PX/HEIGHT_PX`).
pub const PANEL_W: u32 = 1860;
pub const PANEL_H: u32 = 2480;
/// ≈ 7 mm at the panel's ~300 ppi.
pub const SPACING_PX: u32 = 83;
const TOP_MARGIN_PX: u32 = 200;
/// Ignite reserves a top band (`chrome::TOP_BAND_H` = 130 rows) and a bottom band
/// (`BOTTOM_BAND_H` = 120 rows, from row 2360) for its own title/Undo/page-label chrome. The
/// Kindle refuses (and halts inbound sync on!) any background whose bottom band is not one
/// uniform paper colour, so papers are drawn only between these rows and the bands stay pure
/// white with a margin.
pub const SAFE_TOP_PX: u32 = 140;
pub const SAFE_BOTTOM_PX: u32 = 2340;

pub struct Blob {
    pub sha256: Sha256Hex,
    pub bytes: Vec<u8>,
    pub reference: BackgroundRef,
}

fn put(px: &mut [u8], x: u32, y: u32, v: u8) {
    if x < PANEL_W && (SAFE_TOP_PX..SAFE_BOTTOM_PX).contains(&y) {
        let i = (y * PANEL_W + x) as usize;
        px[i] = px[i].min(v);
    }
}

fn hline(px: &mut [u8], y: u32, thick: u32, v: u8) {
    for dy in 0..thick {
        for x in 0..PANEL_W {
            put(px, x, y + dy, v);
        }
    }
}

fn vline(px: &mut [u8], x: u32, thick: u32, v: u8) {
    for dx in 0..thick {
        for y in 0..PANEL_H {
            put(px, x + dx, y, v);
        }
    }
}

/// Render `paper` to packed gray8 (`PANEL_W × PANEL_H`, stride = width). `Blank` is `None`:
/// it is expressed as `BackgroundRef::Blank` and carries no bytes anywhere.
pub fn render(paper: Paper) -> Option<Vec<u8>> {
    let mut px = vec![0xFFu8; (PANEL_W * PANEL_H) as usize];
    match paper {
        Paper::Blank => return None,
        Paper::Lined => {
            let mut y = TOP_MARGIN_PX;
            while y < SAFE_BOTTOM_PX {
                hline(&mut px, y, 2, 0x90);
                y += SPACING_PX;
            }
        }
        Paper::Grid => {
            let mut y = TOP_MARGIN_PX % SPACING_PX;
            while y < PANEL_H {
                hline(&mut px, y, 2, 0xC8);
                y += SPACING_PX;
            }
            let mut x = SPACING_PX / 2;
            while x < PANEL_W {
                vline(&mut px, x, 2, 0xC8);
                x += SPACING_PX;
            }
        }
        Paper::Dot => {
            let mut y = TOP_MARGIN_PX % SPACING_PX + SPACING_PX / 2;
            while y < PANEL_H {
                let mut x = SPACING_PX / 2;
                while x < PANEL_W {
                    for dy in 0..4 {
                        for dx in 0..4 {
                            put(&mut px, x + dx, y + dy, 0x60);
                        }
                    }
                    x += SPACING_PX;
                }
                y += SPACING_PX;
            }
        }
    }
    Some(px)
}

/// The page background for `paper`: `Blank` or a content-addressed blob (shared by every page
/// that uses the same paper).
pub fn background(paper: Paper) -> (BackgroundRef, Option<Blob>) {
    match render(paper) {
        None => (BackgroundRef::Blank, None),
        Some(bytes) => {
            let sha256 = sha256_hex(&bytes);
            let reference = BackgroundRef::Blob {
                sha256: sha256.clone(),
                byte_len: bytes.len() as u64,
                format: BackgroundFormat {
                    encoding: BackgroundEncoding::Gray8,
                    width: PANEL_W,
                    height: PANEL_H,
                    stride: PANEL_W,
                },
            };
            (
                reference.clone(),
                Some(Blob {
                    sha256,
                    bytes,
                    reference,
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_has_no_blob() {
        let (r, b) = background(Paper::Blank);
        assert_eq!(r, BackgroundRef::Blank);
        assert!(b.is_none());
    }

    #[test]
    fn patterned_papers_validate_and_are_deterministic() {
        for p in [Paper::Lined, Paper::Grid, Paper::Dot] {
            let (r1, b1) = background(p);
            let (r2, _) = background(p);
            assert_eq!(r1, r2, "{p:?} must be deterministic");
            r1.validate().unwrap();
            let b = b1.unwrap();
            assert_eq!(b.bytes.len(), (PANEL_W * PANEL_H) as usize);
            assert!(b.bytes.iter().any(|&v| v != 0xFF), "{p:?} drew nothing");
            assert!(
                b.bytes.iter().all(|&v| v >= 0x60),
                "{p:?} too dark for paper"
            );
        }
    }

    #[test]
    fn chrome_bands_stay_pure_white() {
        // Mirrors the Kindle's materializer rule (sync_materialize.rs / page_label.rs): the
        // reserved bands must be untouched paper, or the whole inbound sync halts.
        for p in [Paper::Lined, Paper::Grid, Paper::Dot] {
            let px = render(p).unwrap();
            let w = PANEL_W as usize;
            assert!(
                px[..SAFE_TOP_PX as usize * w].iter().all(|&b| b == 0xFF),
                "{p:?} touches the top band"
            );
            assert!(
                px[SAFE_BOTTOM_PX as usize * w..].iter().all(|&b| b == 0xFF),
                "{p:?} touches the bottom band"
            );
            assert!(SAFE_BOTTOM_PX <= PANEL_H - 120 && SAFE_TOP_PX >= 130);
        }
    }

    #[test]
    fn papers_differ() {
        let a = background(Paper::Lined).1.unwrap().sha256;
        let b = background(Paper::Grid).1.unwrap().sha256;
        let c = background(Paper::Dot).1.unwrap().sha256;
        assert!(a != b && b != c && a != c);
    }
}
