//! The page-spec input format.
//!
//! Coordinates are in **page-width units** by default: `x` runs `0.0..=1.0` across the page and
//! `y` runs `0.0..=1.3333` down it (the page is 3:4, canonical 15624 × 20832). One unit is the
//! same distance horizontally and vertically, so a circle stays a circle and sizes (label height,
//! rectangle width) mean the same in both directions. Set `"coords": "canonical"` to give raw
//! page units instead.
//!
//! ```json
//! { "notebook": { "title": "PoC" },
//!   "pages": [ { "paper": "lined",
//!     "labels": [ { "text": "HELLO", "x": 0.08, "y": 0.10, "size": 0.04 } ],
//!     "shapes": [ { "kind": "rect", "x": 0.1, "y": 0.3, "w": 0.3, "h": 0.15 },
//!                 { "kind": "arrow", "from": [0.4, 0.375], "to": [0.6, 0.375] } ],
//!     "strokes": [ { "points": [[0.1, 0.9], [0.9, 0.9]] } ] } ] }
//! ```

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Coords {
    /// `x` in `0..=1` of page width; `y` in page-width units (`0..=1.3333`).
    #[default]
    PageWidth,
    /// Raw canonical page units (`0..=15624` × `0..=20832`).
    Canonical,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    #[serde(default)]
    pub coords: Coords,
    pub notebook: NotebookIn,
    pub pages: Vec<PageIn>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NotebookIn {
    pub title: String,
}

/// Paper background. `blank` needs no blob; the others upload one shared 4.6 MB gray8 blob.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    #[default]
    Blank,
    Lined,
    Dot,
    Grid,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PageIn {
    #[serde(default)]
    pub paper: Paper,
    /// Drawn first (bottom).
    #[serde(default)]
    pub shapes: Vec<Shape>,
    #[serde(default)]
    pub strokes: Vec<StrokeIn>,
    /// Drawn last (top). Rendered with the built-in single-line stroke font as real ink strokes.
    #[serde(default)]
    pub labels: Vec<Label>,
    /// Text objects. Display-only on the Kindle (editable in ignite-web), default sans face.
    #[serde(default)]
    pub texts: Vec<TextIn>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Uniform line width (the page profile's fixed radius). Best for diagrams and lettering.
    #[default]
    Fixed,
    /// Width follows per-point pressure (third value of each point; default 0.5).
    Pressure,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StrokeIn {
    #[serde(default)]
    pub style: Style,
    /// `[x, y]` or `[x, y, pressure]`.
    pub points: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub text: String,
    pub x: f32,
    pub y: f32,
    /// Capital-letter height, in the spec's coordinate units.
    pub size: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32 },
    Line { from: [f32; 2], to: [f32; 2] },
    Arrow { from: [f32; 2], to: [f32; 2] },
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextIn {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub font_size: f32,
    pub content: String,
}

impl Spec {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let spec: Spec =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid page spec: {e}"))?;
        if spec.pages.is_empty() {
            return Err("invalid page spec: a notebook needs at least one page".into());
        }
        Ok(spec)
    }
}
