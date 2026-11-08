//! Spec → validated `SyncEnvelopeV1` list (+ background blobs). Pure: no network, no clock
//! beyond the caller's ID source, so it is deterministic under test.
//!
//! Operation order (all additive, nothing existing is touched):
//! `CreateNotebook`(first page) → `CreatePage`×(n−1, each after the previous) → per page,
//! `AddStroke`s bottom-to-top (shapes, strokes, labels) → `AddText`s.

use std::collections::HashSet;

use sync_protocol::{
    normalize_text_content, LibraryId, NotebookId, OperationId, PageId, PageSpec, RenderProfileRef,
    ReplicaId, StrokeId, StrokeStyle, SyncEnvelopeV1, SyncOperationV1, SyncPoint, SyncStroke,
    SyncText, TextFace, TextId, CANONICAL_PAGE_HEIGHT, CANONICAL_PAGE_WIDTH, ENVELOPE_VERSION,
};
use uuid::Uuid;

use crate::font;
use crate::paper::{self, Blob};
use crate::spec::{Coords, Paper, Shape, Spec, Style};

/// Everything the builder needs from the target library.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub library_id: LibraryId,
    pub replica_id: ReplicaId,
    /// `client_seq` of the first envelope (the replica's next sequence number).
    pub first_client_seq: u64,
    /// Sealed render profile to give every new page. Copy it from an existing page of the
    /// library so ink pigment/polarity matches the device exactly.
    pub render: RenderProfileRef,
    /// Place the new notebook after this one (`None` = first position).
    pub after_notebook_id: Option<NotebookId>,
}

/// What a page will look like, in canonical units (for the SVG preview and for summaries).
#[derive(Clone, Debug)]
pub struct PagePreview {
    pub paper: Paper,
    pub strokes: Vec<Vec<(f32, f32)>>,
    pub texts: Vec<SyncText>,
}

pub struct Plan {
    pub notebook_id: NotebookId,
    pub envelopes: Vec<SyncEnvelopeV1>,
    /// Distinct backgrounds to upload before the envelopes (deduplicated by SHA-256).
    pub blobs: Vec<Blob>,
    pub pages: Vec<PagePreview>,
}

impl Plan {
    pub fn stroke_count(&self) -> usize {
        self.pages.iter().map(|p| p.strokes.len()).sum()
    }
    pub fn text_count(&self) -> usize {
        self.pages.iter().map(|p| p.texts.len()).sum()
    }
}

struct Scale {
    unit: f32,
}

impl Scale {
    fn new(c: &Coords) -> Self {
        Self {
            unit: match c {
                Coords::PageWidth => CANONICAL_PAGE_WIDTH,
                Coords::Canonical => 1.0,
            },
        }
    }
    fn pt(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.unit, y * self.unit)
    }
}

const ELLIPSE_SEGMENTS: usize = 64;

/// The sealed profile ignite-web gives browser-created pages (black ink, pressure curve). Used
/// when the library has no page to copy a profile from, and for offline dry runs.
pub fn default_render() -> RenderProfileRef {
    RenderProfileRef {
        profile_version: 2,
        ink_byte: 0x00,
        legacy_fixed_radius: 2,
        transform_id: "scribe1-portrait-identity-v1".into(),
        rasterizer_id: "scribe-raster-v2-pressure".into(),
        pressure: Some(sync_protocol::PressureCurveRef {
            q_r1_max: 820,
            q_r2_max: 2866,
            q_r3_max: 3685,
            min_radius: 1,
            max_radius: 4,
            missing_fallback_radius: 2,
        }),
    }
}

/// Canonical units per panel row (20832 / 2480 = 8.4).
const ROW: f32 = CANONICAL_PAGE_HEIGHT / paper::PANEL_H as f32;
/// Widest sealed pen radius in panel px, so a stroke's ink never reaches into a band.
const INK_MARGIN_PX: f32 = 4.0;

/// Vertical range (canonical units) where ink and text may go. Ignite draws its title/Undo
/// chrome over panel rows 0..130 and its page label over 2360..2480; the importer keeps
/// everything between rows 140 and 2340 (same rule as the paper backgrounds, `paper.rs`), so
/// nothing generated hides under, or collides with, the Kindle's own chrome.
pub fn safe_y_range() -> (f32, f32) {
    (
        (paper::SAFE_TOP_PX as f32 + INK_MARGIN_PX) * ROW,
        (paper::SAFE_BOTTOM_PX as f32 - INK_MARGIN_PX) * ROW,
    )
}

fn band_error(what: &str, y: f32, unit: f32) -> String {
    let (top, bottom) = safe_y_range();
    format!(
        "{what} at y={} reaches into Ignite's reserved top/bottom band; keep content between y={:.4} and y={:.4}",
        y / unit,
        top / unit,
        bottom / unit
    )
}

/// `page_width`: the page width expressed in the spec's units (sizes the arrow head).
fn shape_polylines(shape: &Shape, page_width: f32) -> Vec<Vec<(f32, f32)>> {
    match *shape {
        Shape::Rect { x, y, w, h } => {
            vec![vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]]
        }
        Shape::Ellipse { cx, cy, rx, ry } => {
            let pts = (0..=ELLIPSE_SEGMENTS)
                .map(|i| {
                    let a = i as f32 / ELLIPSE_SEGMENTS as f32 * std::f32::consts::TAU;
                    (cx + rx * a.cos(), cy + ry * a.sin())
                })
                .collect();
            vec![pts]
        }
        Shape::Line { from, to } => vec![vec![(from[0], from[1]), (to[0], to[1])]],
        Shape::Arrow { from, to } => {
            let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
            let len = (dx * dx + dy * dy).sqrt();
            let mut out = vec![vec![(from[0], from[1]), (to[0], to[1])]];
            if len > 0.0 {
                let head = (0.03 * page_width).min(0.3 * len);
                let (ux, uy) = (dx / len, dy / len);
                for sign in [1.0f32, -1.0] {
                    // Rotate the reversed direction by ±28°.
                    let a = 28f32.to_radians() * sign;
                    let (c, s) = (a.cos(), a.sin());
                    let (rx, ry) = (-ux * c + uy * s, -ux * s - uy * c);
                    out.push(vec![(to[0], to[1]), (to[0] + rx * head, to[1] + ry * head)]);
                }
            }
            out
        }
    }
}

/// Build the plan. `mint` supplies fresh UUIDs (v7 in production, sequential in tests).
pub fn build(spec: &Spec, ctx: &Ctx, mint: &mut dyn FnMut() -> Uuid) -> Result<Plan, String> {
    let sc = Scale::new(&spec.coords);
    let mut seq = ctx.first_client_seq;
    let mut envelopes: Vec<SyncEnvelopeV1> = Vec::new();
    let mut blobs: Vec<Blob> = Vec::new();
    let mut seen_blobs = HashSet::new();
    let mut previews = Vec::new();

    let mut emit = |op: SyncOperationV1, mint: &mut dyn FnMut() -> Uuid| -> Result<(), String> {
        let env = SyncEnvelopeV1 {
            v: ENVELOPE_VERSION,
            operation_id: OperationId::from_uuid(mint()).map_err(|e| e.to_string())?,
            library_id: ctx.library_id,
            replica_id: ctx.replica_id,
            client_seq: seq,
            operation: op,
        };
        env.validate().map_err(|e| format!("envelope {seq}: {e}"))?;
        envelopes.push(env);
        seq += 1;
        Ok(())
    };

    let notebook_id = NotebookId::from_uuid(mint()).map_err(|e| e.to_string())?;
    let mut page_ids: Vec<PageId> = Vec::new();
    let mut page_specs: Vec<PageSpec> = Vec::new();
    for page in &spec.pages {
        let (background, blob) = paper::background(page.paper);
        if let Some(b) = blob {
            if seen_blobs.insert(b.sha256.clone()) {
                blobs.push(b);
            }
        }
        let pid = PageId::from_uuid(mint()).map_err(|e| e.to_string())?;
        page_ids.push(pid);
        page_specs.push(PageSpec {
            page_id: pid,
            background,
            render: ctx.render.clone(),
        });
    }

    emit(
        SyncOperationV1::CreateNotebook {
            notebook_id,
            title: spec.notebook.title.clone(),
            after_notebook_id: ctx.after_notebook_id,
            first_page: page_specs[0].clone(),
        },
        mint,
    )?;
    for i in 1..page_specs.len() {
        emit(
            SyncOperationV1::CreatePage {
                notebook_id,
                page: page_specs[i].clone(),
                after_page_id: Some(page_ids[i - 1]),
            },
            mint,
        )?;
    }

    for (pi, page) in spec.pages.iter().enumerate() {
        let here = |what: &str| format!("page {} {what}", pi + 1);
        let mut polys: Vec<(Style, Vec<Vec<f32>>)> = Vec::new();

        for (si, shape) in page.shapes.iter().enumerate() {
            let _ = si;
            // Spec units per page width: 1 for page-width coords, 15624 for canonical.
            for poly in shape_polylines(shape, CANONICAL_PAGE_WIDTH / sc.unit) {
                polys.push((
                    Style::Fixed,
                    poly.into_iter().map(|(x, y)| vec![x, y]).collect(),
                ));
            }
        }
        for s in &page.strokes {
            polys.push((s.style, s.points.clone()));
        }
        for (li, label) in page.labels.iter().enumerate() {
            let lines = font::layout(&label.text, label.x, label.y, label.size)
                .map_err(|e| here(&format!("label {}: {e}", li + 1)))?;
            for poly in lines {
                polys.push((
                    Style::Fixed,
                    poly.into_iter().map(|(x, y)| vec![x, y]).collect(),
                ));
            }
        }

        let mut preview = PagePreview {
            paper: page.paper,
            strokes: Vec::new(),
            texts: Vec::new(),
        };
        for (n, (style, pts)) in polys.iter().enumerate() {
            let mut out: Vec<SyncPoint> = Vec::with_capacity(pts.len());
            for p in pts {
                if p.len() != 2 && p.len() != 3 {
                    return Err(here(&format!(
                        "stroke {}: a point needs [x, y] or [x, y, pressure], got {} values",
                        n + 1,
                        p.len()
                    )));
                }
                let (x, y) = sc.pt(p[0], p[1]);
                if !(x.is_finite() && y.is_finite())
                    || !(0.0..=CANONICAL_PAGE_WIDTH).contains(&x)
                    || !(0.0..=CANONICAL_PAGE_HEIGHT).contains(&y)
                {
                    return Err(here(&format!(
                        "stroke {}: point ({}, {}) is outside the page",
                        n + 1,
                        p[0],
                        p[1]
                    )));
                }
                let (top, bottom) = safe_y_range();
                if !(top..=bottom).contains(&y) {
                    return Err(here(&band_error(&format!("stroke {}", n + 1), y, sc.unit)));
                }
                let pressure = match style {
                    Style::Fixed => None,
                    Style::Pressure => Some(p.get(2).copied().unwrap_or(0.5)),
                };
                let sp = SyncPoint::new(x, y, pressure);
                if out.last().is_some_and(|l| l.x == sp.x && l.y == sp.y) {
                    continue;
                }
                out.push(sp);
            }
            if out.is_empty() {
                return Err(here(&format!("stroke {} has no points", n + 1)));
            }
            if *style == Style::Pressure && ctx.render.pressure.is_none() {
                return Err(here(
                    "uses pressure strokes but the library's render profile has no pressure curve",
                ));
            }
            let id = StrokeId::from_uuid(mint()).map_err(|e| e.to_string())?;
            preview
                .strokes
                .push(out.iter().map(|p| (p.x, p.y)).collect());
            emit(
                SyncOperationV1::AddStroke {
                    notebook_id,
                    page_id: page_ids[pi],
                    stroke: SyncStroke {
                        stroke_id: id,
                        root_id: id,
                        style: match style {
                            Style::Fixed => StrokeStyle::LegacyFixedV1,
                            Style::Pressure => StrokeStyle::PressureWidthV1,
                        },
                        points: out,
                    },
                },
                mint,
            )?;
        }

        for (ti, t) in page.texts.iter().enumerate() {
            let text = SyncText {
                text_id: TextId::from_uuid(mint()).map_err(|e| e.to_string())?,
                x: t.x * sc.unit,
                y: t.y * sc.unit,
                width: t.width * sc.unit,
                font_size: t.font_size * sc.unit,
                face: TextFace::DefaultSansV1,
                content: normalize_text_content(&t.content),
            };
            let (top, bottom) = safe_y_range();
            let height = crate::layout::layout_height(&text.content, text.font_size, text.width);
            if text.y < top || text.y + height > bottom {
                let y = if text.y < top {
                    text.y
                } else {
                    text.y + height
                };
                return Err(here(&band_error(&format!("text {}", ti + 1), y, sc.unit)));
            }
            preview.texts.push(text.clone());
            emit(
                SyncOperationV1::AddText {
                    notebook_id,
                    page_id: page_ids[pi],
                    text,
                },
                mint,
            )
            .map_err(|e| here(&format!("text {}: {e}", ti + 1)))?;
        }
        previews.push(preview);
    }

    Ok(Plan {
        notebook_id,
        envelopes,
        blobs,
        pages: previews,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Spec;
    use sync_protocol::PressureCurveRef;

    pub fn render(pressure: bool) -> RenderProfileRef {
        RenderProfileRef {
            profile_version: 2,
            ink_byte: 0x00,
            legacy_fixed_radius: 2,
            transform_id: "scribe1-portrait-identity-v1".into(),
            rasterizer_id: "scribe-raster-v2-pressure".into(),
            pressure: pressure.then_some(PressureCurveRef {
                q_r1_max: 820,
                q_r2_max: 2866,
                q_r3_max: 3685,
                min_radius: 1,
                max_radius: 4,
                missing_fallback_radius: 2,
            }),
        }
    }

    fn ctx(pressure: bool) -> Ctx {
        let mut m = counter();
        Ctx {
            library_id: LibraryId::from_uuid(m()).unwrap(),
            replica_id: ReplicaId::from_uuid(m()).unwrap(),
            first_client_seq: 7,
            render: render(pressure),
            after_notebook_id: None,
        }
    }

    fn counter() -> impl FnMut() -> Uuid {
        let mut n: u64 = 0;
        move || {
            n += 1;
            let mut b = [0u8; 16];
            b[0] = 0x01;
            b[6] = 0x70;
            b[8] = 0x80;
            b[9..16].copy_from_slice(&n.to_be_bytes()[1..]);
            Uuid::from_bytes(b)
        }
    }

    const POC: &str = r#"{
      "notebook": {"title": "PoC"},
      "pages": [
        {"paper": "lined",
         "labels": [{"text": "Hello", "x": 0.08, "y": 0.10, "size": 0.04}],
         "shapes": [{"kind":"rect","x":0.1,"y":0.3,"w":0.3,"h":0.15},
                    {"kind":"arrow","from":[0.4,0.375],"to":[0.6,0.375]},
                    {"kind":"ellipse","cx":0.75,"cy":0.375,"rx":0.12,"ry":0.08}],
         "texts": [{"x":0.1,"y":0.6,"width":0.5,"font_size":0.03,"content":"note"}]},
        {"paper": "lined"},
        {"paper": "blank", "strokes": [{"points": [[0.1,0.9],[0.9,0.9]]}]}
      ]}"#;

    #[test]
    fn builds_ordered_valid_envelopes() {
        let spec = Spec::parse(POC.as_bytes()).unwrap();
        let mut m = counter();
        for _ in 0..10 {
            m();
        }
        let plan = build(&spec, &ctx(true), &mut m).unwrap();
        // 1 CreateNotebook + 2 CreatePage + (4 label letters' strokes counted below) …
        assert!(matches!(
            plan.envelopes[0].operation,
            SyncOperationV1::CreateNotebook { .. }
        ));
        assert!(matches!(
            plan.envelopes[1].operation,
            SyncOperationV1::CreatePage { .. }
        ));
        assert!(matches!(
            plan.envelopes[2].operation,
            SyncOperationV1::CreatePage { .. }
        ));
        for (i, e) in plan.envelopes.iter().enumerate() {
            assert_eq!(e.client_seq, 7 + i as u64, "dense client_seq");
            e.to_canonical_bytes().unwrap();
        }
        assert_eq!(plan.pages.len(), 3);
        assert_eq!(plan.text_count(), 1);
        // rect(1) + arrow(3) + ellipse(1) + "HELLO" glyph strokes on page 1; 1 stroke on page 3.
        let hello = font::layout("HELLO", 0.0, 0.0, 1.0).unwrap().len();
        assert_eq!(plan.pages[0].strokes.len(), 5 + hello);
        assert_eq!(plan.pages[2].strokes.len(), 1);
        // Two lined pages share one blob.
        assert_eq!(plan.blobs.len(), 1);
    }

    #[test]
    fn page_links_chain_in_order() {
        let spec = Spec::parse(POC.as_bytes()).unwrap();
        let plan = build(&spec, &ctx(true), &mut counter()).unwrap();
        let created: Vec<_> = plan
            .envelopes
            .iter()
            .filter_map(|e| match &e.operation {
                SyncOperationV1::CreateNotebook { first_page, .. } => {
                    Some((first_page.page_id, None))
                }
                SyncOperationV1::CreatePage {
                    page,
                    after_page_id,
                    ..
                } => Some((page.page_id, *after_page_id)),
                _ => None,
            })
            .collect();
        assert_eq!(created[1].1, Some(created[0].0));
        assert_eq!(created[2].1, Some(created[1].0));
    }

    #[test]
    fn rejects_points_outside_the_page_instead_of_clamping() {
        let spec = Spec::parse(
            br#"{"notebook":{"title":"x"},"pages":[{"strokes":[{"points":[[0.5,0.5],[1.2,0.5]]}]}]}"#,
        )
        .unwrap();
        let e = build(&spec, &ctx(true), &mut counter()).err().unwrap();
        assert!(e.contains("outside the page"), "{e}");
    }

    #[test]
    fn rejects_bad_point_arity_and_unknown_fields() {
        let spec = Spec::parse(
            br#"{"notebook":{"title":"x"},"pages":[{"strokes":[{"points":[[0.5]]}]}]}"#,
        )
        .unwrap();
        assert!(build(&spec, &ctx(true), &mut counter()).is_err());
        assert!(Spec::parse(br#"{"notebook":{"title":"x"},"pages":[{}],"bogus":1}"#).is_err());
        assert!(Spec::parse(br#"{"notebook":{"title":"x"},"pages":[]}"#).is_err());
    }

    #[test]
    fn pressure_strokes_need_a_pressure_profile() {
        let spec = Spec::parse(
            br#"{"notebook":{"title":"x"},"pages":[{"strokes":[{"style":"pressure","points":[[0.1,0.1,0.2],[0.2,0.2,0.9]]}]}]}"#,
        )
        .unwrap();
        assert!(build(&spec, &ctx(true), &mut counter()).is_ok());
        let e = build(&spec, &ctx(false), &mut counter()).err().unwrap();
        assert!(e.contains("pressure"), "{e}");
    }

    #[test]
    fn canonical_coords_and_title_rules() {
        let spec = Spec::parse(
            br#"{"coords":"canonical","notebook":{"title":"x"},"pages":[{"strokes":[{"points":[[100,1300],[15000,19000]]}]}]}"#,
        )
        .unwrap();
        let plan = build(&spec, &ctx(true), &mut counter()).unwrap();
        assert_eq!(
            plan.pages[0].strokes[0],
            vec![(100.0, 1300.0), (15000.0, 19000.0)]
        );
        let bad = Spec::parse(br#"{"notebook":{"title":"  "},"pages":[{}]}"#).unwrap();
        assert!(build(&bad, &ctx(true), &mut counter()).is_err());
    }

    #[test]
    fn keeps_ink_and_text_out_of_the_chrome_bands() {
        for bad in [
            r#"{"notebook":{"title":"x"},"pages":[{"strokes":[{"points":[[0.5,0.05],[0.5,0.3]]}]}]}"#,
            r#"{"notebook":{"title":"x"},"pages":[{"strokes":[{"points":[[0.5,0.5],[0.5,1.3]]}]}]}"#,
            r#"{"notebook":{"title":"x"},"pages":[{"labels":[{"text":"TOP","x":0.1,"y":0.02,"size":0.03}]}]}"#,
            r#"{"notebook":{"title":"x"},"pages":[{"texts":[{"x":0.1,"y":1.24,"width":0.5,"font_size":0.03,"content":"low"}]}]}"#,
        ] {
            let spec = Spec::parse(bad.as_bytes()).unwrap();
            let e = build(&spec, &ctx(true), &mut counter()).err().unwrap();
            assert!(e.contains("reserved top/bottom band"), "{e}");
        }
        let (top, bottom) = safe_y_range();
        assert!(top >= 140.0 * ROW && bottom <= 2340.0 * ROW);
    }

    #[test]
    fn identical_consecutive_points_are_collapsed() {
        let spec = Spec::parse(
            br#"{"notebook":{"title":"x"},"pages":[{"strokes":[{"points":[[0.1,0.1],[0.1,0.1],[0.2,0.2]]}]}]}"#,
        )
        .unwrap();
        let plan = build(&spec, &ctx(true), &mut counter()).unwrap();
        assert_eq!(plan.pages[0].strokes[0].len(), 2);
    }
}
