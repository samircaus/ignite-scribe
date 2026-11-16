//! A small SVG subset turned into centre-line polylines (ink strokes).
//!
//! Supported: `<path d>` (M L H V C S Q T A Z, absolute and relative), `<line>`, `<polyline>`,
//! `<polygon>`, `<rect>`, `<circle>`, `<ellipse>`, nested `<g>`/`<svg>` with `transform`
//! (`matrix`, `translate`, `scale`, `rotate`, `skewX`, `skewY`) and the root `viewBox`.
//! Everything is drawn as an outline: fill, stroke width and colour are ignored, because the
//! result is ordinary pen ink. `<text>`, `<image>`, `<defs>`, `<style>` and similar are skipped
//! with a warning. No external crates: a tolerant tag scanner, not a full XML parser.

pub type Poly = Vec<(f32, f32)>;

// ---------------------------------------------------------------- affine transforms

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine([f32; 6]); // a b c d e f: x' = a x + c y + e, y' = b x + d y + f

impl Affine {
    pub const ID: Affine = Affine([1., 0., 0., 1., 0., 0.]);
    pub fn apply(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
    /// `self` then `next` (next applied after self) is `next.mul(self)`; here: self ∘ other.
    pub fn mul(&self, o: &Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = o.0;
        Affine([
            a * a2 + c * b2,
            b * a2 + d * b2,
            a * c2 + c * d2,
            b * c2 + d * d2,
            a * e2 + c * f2 + e,
            b * e2 + d * f2 + f,
        ])
    }
}

fn parse_transform(s: &str) -> Result<Affine, String> {
    let mut m = Affine::ID;
    let mut rest = s.trim();
    while !rest.is_empty() {
        let open = rest
            .find('(')
            .ok_or_else(|| format!("transform {s:?}: expected '('"))?;
        let close = rest[open..]
            .find(')')
            .map(|i| i + open)
            .ok_or_else(|| format!("transform {s:?}: expected ')'"))?;
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let v = numbers(&rest[open + 1..close])?;
        let t = match (name, v.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine([*a, *b, *c, *d, *e, *f]),
            ("translate", [x]) => Affine([1., 0., 0., 1., *x, 0.]),
            ("translate", [x, y]) => Affine([1., 0., 0., 1., *x, *y]),
            ("scale", [k]) => Affine([*k, 0., 0., *k, 0., 0.]),
            ("scale", [x, y]) => Affine([*x, 0., 0., *y, 0., 0.]),
            ("rotate", [deg]) => rotate(*deg),
            ("rotate", [deg, cx, cy]) => Affine([1., 0., 0., 1., *cx, *cy])
                .mul(&rotate(*deg))
                .mul(&Affine([1., 0., 0., 1., -cx, -cy])),
            ("skewX", [deg]) => Affine([1., 0., deg.to_radians().tan(), 1., 0., 0.]),
            ("skewY", [deg]) => Affine([1., deg.to_radians().tan(), 0., 1., 0., 0.]),
            _ => return Err(format!("transform {s:?}: unsupported {name}({v:?})")),
        };
        m = m.mul(&t);
        rest = rest[close + 1..].trim();
    }
    Ok(m)
}

fn rotate(deg: f32) -> Affine {
    let (s, c) = deg.to_radians().sin_cos();
    Affine([c, s, -s, c, 0., 0.])
}

// ---------------------------------------------------------------- numbers and path data

/// SVG number list: separators are whitespace/commas, and a sign or a second '.' starts a
/// new number ("1-2" = 1, -2; "1.5.5" = 1.5, 0.5).
fn numbers(s: &str) -> Result<Vec<f32>, String> {
    let mut sc = Scanner::new(s);
    let mut out = Vec::new();
    loop {
        sc.skip_sep();
        if sc.done() {
            return Ok(out);
        }
        out.push(sc.number()?);
    }
}

struct Scanner<'a> {
    b: &'a [u8],
    i: usize,
    src: &'a str,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            b: s.as_bytes(),
            i: 0,
            src: s,
        }
    }
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
    fn skip_sep(&mut self) {
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_whitespace() || self.b[self.i] == b',')
        {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn number(&mut self) -> Result<f32, String> {
        self.skip_sep();
        let start = self.i;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.i += 1;
        }
        let mut dot = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' && !dot {
                dot = true;
                self.i += 1;
            } else {
                break;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(), Some(b'-' | b'+')) {
                self.i += 1;
            }
            if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.i += 1;
                }
            } else {
                self.i = save;
            }
        }
        let t = &self.src[start..self.i];
        t.parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("bad number {t:?} in {:?}", self.src))
    }
    /// Arc flags may be written without separators ("a1 1 0 01 5 5").
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_sep();
        match self.peek() {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(format!("bad arc flag in {:?}", self.src)),
        }
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn segments(len: f32, seg: f32) -> usize {
    ((len / seg.max(1e-6)).ceil() as usize).clamp(2, 96)
}

/// Path data → polylines in the path's own coordinates. `seg` is the target segment length
/// for curves (smaller = smoother).
pub fn path_polylines(d: &str, seg: f32) -> Result<Vec<Poly>, String> {
    let mut sc = Scanner::new(d);
    let mut out: Vec<Poly> = Vec::new();
    let mut cur: Poly = Vec::new();
    let (mut p, mut start) = ((0f32, 0f32), (0f32, 0f32));
    let mut last_ctrl: Option<(f32, f32)> = None; // for S/T reflection
    let mut last_cmd = b' ';
    let mut cmd = b' ';
    let flush = |cur: &mut Poly, out: &mut Vec<Poly>| {
        if cur.len() >= 2 {
            out.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    loop {
        sc.skip_sep();
        let Some(c) = sc.peek() else { break };
        if c.is_ascii_alphabetic() {
            cmd = c;
            sc.i += 1;
        } else if cmd == b' ' {
            return Err(format!("path {d:?} must start with a command"));
        } else if cmd == b'M' {
            cmd = b'L'; // implicit lineto after moveto
        } else if cmd == b'm' {
            cmd = b'l';
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { p } else { (0., 0.) };
        let pt = |sc: &mut Scanner| -> Result<(f32, f32), String> {
            let x = sc.number()?;
            let y = sc.number()?;
            Ok((x + base.0, y + base.1))
        };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                flush(&mut cur, &mut out);
                p = pt(&mut sc)?;
                start = p;
                cur.push(p);
                last_ctrl = None;
            }
            b'L' => {
                p = pt(&mut sc)?;
                cur.push(p);
                last_ctrl = None;
            }
            b'H' => {
                let x = sc.number()? + base.0;
                p = (x, p.1);
                cur.push(p);
                last_ctrl = None;
            }
            b'V' => {
                let y = sc.number()? + base.1;
                p = (p.0, y);
                cur.push(p);
                last_ctrl = None;
            }
            b'Z' => {
                if cur.len() >= 1 && cur.last() != Some(&start) {
                    cur.push(start);
                }
                flush(&mut cur, &mut out);
                p = start;
                cur.push(p);
                last_ctrl = None;
                last_cmd = b'Z';
                // Z takes no arguments; another command letter must follow.
                cmd = b' ';
                continue;
            }
            b'C' | b'S' => {
                let c1 = if cmd.to_ascii_uppercase() == b'C' {
                    pt(&mut sc)?
                } else {
                    match (last_ctrl, last_cmd.to_ascii_uppercase()) {
                        (Some(lc), b'C' | b'S') => (2. * p.0 - lc.0, 2. * p.1 - lc.1),
                        _ => p,
                    }
                };
                let c2 = pt(&mut sc)?;
                let e = pt(&mut sc)?;
                if cur.is_empty() {
                    cur.push(p);
                }
                let n = segments(dist(p, c1) + dist(c1, c2) + dist(c2, e), seg);
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let u = 1. - t;
                    let w = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
                    cur.push((
                        w[0] * p.0 + w[1] * c1.0 + w[2] * c2.0 + w[3] * e.0,
                        w[0] * p.1 + w[1] * c1.1 + w[2] * c2.1 + w[3] * e.1,
                    ));
                }
                last_ctrl = Some(c2);
                p = e;
            }
            b'Q' | b'T' => {
                let c1 = if cmd.to_ascii_uppercase() == b'Q' {
                    pt(&mut sc)?
                } else {
                    match (last_ctrl, last_cmd.to_ascii_uppercase()) {
                        (Some(lc), b'Q' | b'T') => (2. * p.0 - lc.0, 2. * p.1 - lc.1),
                        _ => p,
                    }
                };
                let e = pt(&mut sc)?;
                if cur.is_empty() {
                    cur.push(p);
                }
                let n = segments(dist(p, c1) + dist(c1, e), seg);
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let u = 1. - t;
                    cur.push((
                        u * u * p.0 + 2. * u * t * c1.0 + t * t * e.0,
                        u * u * p.1 + 2. * u * t * c1.1 + t * t * e.1,
                    ));
                }
                last_ctrl = Some(c1);
                p = e;
            }
            b'A' => {
                let rx = sc.number()?;
                let ry = sc.number()?;
                let rot = sc.number()?;
                let large = sc.flag()?;
                let sweep = sc.flag()?;
                let e = pt(&mut sc)?;
                if cur.is_empty() {
                    cur.push(p);
                }
                arc(&mut cur, p, rx, ry, rot, large, sweep, e, seg);
                last_ctrl = None;
                p = e;
            }
            other => {
                return Err(format!(
                    "path {d:?}: unsupported command {:?}",
                    other as char
                ))
            }
        }
        // Repeated argument groups keep the same command (handled at the loop top).
        last_cmd = cmd;
    }
    flush(&mut cur, &mut out);
    Ok(out)
}

/// SVG elliptical arc (endpoint parameterisation, spec F.6.5/F.6.6).
#[allow(clippy::too_many_arguments)]
fn arc(
    cur: &mut Poly,
    p0: (f32, f32),
    rx: f32,
    ry: f32,
    rot_deg: f32,
    large: bool,
    sweep: bool,
    p1: (f32, f32),
    seg: f32,
) {
    let (mut rx, mut ry) = (rx.abs() as f64, ry.abs() as f64);
    if rx == 0. || ry == 0. || p0 == p1 {
        cur.push(p1);
        return;
    }
    let (x0, y0, x1, y1) = (p0.0 as f64, p0.1 as f64, p1.0 as f64, p1.1 as f64);
    let phi = (rot_deg as f64).to_radians();
    let (sp, cp) = phi.sin_cos();
    let dx = (x0 - x1) / 2.;
    let dy = (y0 - y1) / 2.;
    let x1p = cp * dx + sp * dy;
    let y1p = -sp * dx + cp * dy;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1. {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut co = (num / den).max(0.).sqrt();
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1p / ry;
    let cyp = -co * ry * x1p / rx;
    let cx = cp * cxp - sp * cyp + (x0 + x1) / 2.;
    let cy = sp * cxp + cp * cyp + (y0 + y1) / 2.;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let t1 = ang(1., 0., (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dt = ang(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dt > 0. {
        dt -= std::f64::consts::TAU;
    } else if sweep && dt < 0. {
        dt += std::f64::consts::TAU;
    }
    let len = (dt.abs() * (rx + ry) / 2.) as f32;
    let n = segments(len, seg);
    for k in 1..=n {
        let t = t1 + dt * k as f64 / n as f64;
        let (st, ct) = t.sin_cos();
        let x = cp * rx * ct - sp * ry * st + cx;
        let y = sp * rx * ct + cp * ry * st + cy;
        cur.push((x as f32, y as f32));
    }
    // Land exactly on the endpoint.
    if let Some(last) = cur.last_mut() {
        *last = p1;
    }
}

// ---------------------------------------------------------------- documents

/// The drawing's own coordinate box (viewBox, or width/height, or the content's bounds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub struct Drawing {
    pub polys: Vec<Poly>,
    /// Per polyline: whether it comes from a `<text>` label.
    pub text: Vec<bool>,
    pub view: ViewBox,
    pub warnings: Vec<String>,
}

struct Tag<'a> {
    name: String,
    attrs: Vec<(String, String)>,
    closing: bool,
    self_closing: bool,
    /// Byte offsets of the tag's '<' and just past its '>'.
    start: usize,
    end: usize,
    _src: &'a str,
}

impl Tag<'_> {
    fn get(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }
    fn num(&self, k: &str) -> Result<f32, String> {
        match self.get(k) {
            None => Ok(0.),
            Some(v) => {
                let v = v.trim().trim_end_matches("px");
                v.parse::<f32>()
                    .ok()
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| format!("<{}> {k}={v:?} is not a plain number", self.name))
            }
        }
    }
}

fn tags(src: &str) -> Result<Vec<Tag<'_>>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    let b = src.as_bytes();
    while let Some(off) = src[i..].find('<') {
        i += off;
        if src[i..].starts_with("<!--") {
            i = src[i..]
                .find("-->")
                .map(|j| i + j + 3)
                .ok_or("unterminated comment")?;
            continue;
        }
        if src[i..].starts_with("<?") || src[i..].starts_with("<!") {
            i = src[i..]
                .find('>')
                .map(|j| i + j + 1)
                .ok_or("unterminated tag")?;
            continue;
        }
        // Find the closing '>' outside quotes.
        let mut j = i + 1;
        let mut quote: Option<u8> = None;
        while j < b.len() {
            match (quote, b[j]) {
                (None, b'"' | b'\'') => quote = Some(b[j]),
                (Some(q), c) if c == q => quote = None,
                (None, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        if j >= b.len() {
            return Err("unterminated tag".into());
        }
        let inner = &src[i + 1..j];
        let start = i;
        i = j + 1;
        let closing = inner.starts_with('/');
        let self_closing = inner.ends_with('/');
        let inner = inner.trim_start_matches('/').trim_end_matches('/');
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = inner[..name_end]
            .rsplit(':')
            .next()
            .unwrap_or("")
            .to_string();
        let mut attrs = Vec::new();
        let mut rest = &inner[name_end..];
        while let Some(eq) = rest.find('=') {
            let key = rest[..eq]
                .trim()
                .rsplit(':')
                .next()
                .unwrap_or("")
                .to_string();
            let after = rest[eq + 1..].trim_start();
            let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                return Err(format!("<{name}>: attribute {key} needs a quoted value"));
            };
            let end = after[1..]
                .find(q)
                .ok_or_else(|| format!("<{name}>: unterminated attribute {key}"))?;
            attrs.push((key, after[1..1 + end].to_string()));
            rest = &after[1 + end + 1..];
        }
        out.push(Tag {
            name,
            attrs,
            closing,
            self_closing,
            start,
            end: i,
            _src: src,
        });
    }
    Ok(out)
}

const SKIPPED: &[&str] = &[
    "defs",
    "style",
    "title",
    "desc",
    "symbol",
    "clipPath",
    "mask",
    "pattern",
    "marker",
    "image",
    "foreignObject",
    "metadata",
    "linearGradient",
    "radialGradient",
    "filter",
];

/// The characters of a `<text>` element (nested `<tspan>`s included, tags dropped), entities
/// decoded and whitespace collapsed.
fn text_content(src: &str, all: &[Tag<'_>], open: usize) -> String {
    let mut depth = 0;
    let mut raw = String::new();
    let mut from = all[open].end;
    for t in &all[open + 1..] {
        raw.push_str(&src[from..t.start]);
        from = t.end;
        if t.name == "text" {
            if t.closing {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if !t.self_closing {
                depth += 1;
            }
        }
    }
    let raw = raw
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cap height as a share of `font-size` (the em), close to common fonts.
const CAP_PER_EM: f32 = 0.7;

/// A `<text>` element in the handwriting font: `x`/`y` place the baseline, `font-size` (default
/// 16) sets the size, `text-anchor` (start, middle, end) and `dominant-baseline` (middle,
/// central, hanging) align it, as in SVG.
fn text_polylines(src: &str, all: &[Tag<'_>], ti: usize) -> Result<Vec<Poly>, String> {
    let t = &all[ti];
    let text = text_content(src, all, ti);
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let em = match t.get("font-size") {
        None => 16.,
        Some(_) => t.num("font-size")?,
    };
    if !(em > 0.) {
        return Err(format!("<text> font-size {em} must be positive"));
    }
    let cap = em * CAP_PER_EM;
    let (w, _) = crate::hand::measure(&text, cap, None);
    let x = t.num("x")?
        - match t.get("text-anchor").map(str::trim) {
            Some("middle") => w / 2.,
            Some("end") => w,
            _ => 0.,
        };
    let base = t.num("y")?;
    let top = match t.get("dominant-baseline").map(str::trim) {
        Some("middle" | "central") => base - cap / 2.,
        Some("hanging" | "text-before-edge") => base,
        _ => base - cap,
    };
    crate::hand::layout(&text, x, top, cap, None).map_err(|e| format!("<text>: {e}"))
}

/// Parse a drawing. Polylines are in the drawing's user units after transforms.
pub fn parse(src: &str) -> Result<Drawing, String> {
    let all = tags(src)?;
    let mut polys: Vec<Poly> = Vec::new();
    let mut text: Vec<bool> = Vec::new();
    let mut warnings = Vec::new();
    let mut stack: Vec<Affine> = vec![Affine::ID];
    let mut skip: Vec<String> = Vec::new();
    let mut view: Option<ViewBox> = None;
    let mut seen_svg = false;
    for (ti, t) in all.iter().enumerate() {
        if let Some(s) = skip.last() {
            if t.closing && &t.name == s {
                skip.pop();
            } else if !t.closing && !t.self_closing && &t.name == s {
                skip.push(t.name.clone());
            }
            continue;
        }
        if t.closing {
            if matches!(t.name.as_str(), "g" | "svg" | "a") && stack.len() > 1 {
                stack.pop();
            }
            continue;
        }
        if SKIPPED.contains(&t.name.as_str()) {
            if !matches!(
                t.name.as_str(),
                "defs" | "style" | "title" | "desc" | "metadata"
            ) {
                warnings.push(format!("<{}> is not drawn", t.name));
            }
            if !t.self_closing {
                skip.push(t.name.clone());
            }
            continue;
        }
        let own = match t.get("transform") {
            Some(s) => parse_transform(s)?,
            None => Affine::ID,
        };
        let m = stack.last().unwrap().mul(&own);
        if t.name == "svg" && !seen_svg {
            seen_svg = true;
            if let Some(vb) = t.get("viewBox") {
                let v = numbers(vb)?;
                if v.len() != 4 || v[2] <= 0. || v[3] <= 0. {
                    return Err(format!(
                        "viewBox {vb:?} needs four numbers with positive size"
                    ));
                }
                view = Some(ViewBox {
                    x: v[0],
                    y: v[1],
                    w: v[2],
                    h: v[3],
                });
            } else if t.get("width").is_some() && t.get("height").is_some() {
                let (w, h) = (t.num("width")?, t.num("height")?);
                if w > 0. && h > 0. {
                    view = Some(ViewBox { x: 0., y: 0., w, h });
                }
            }
        }
        if matches!(t.name.as_str(), "g" | "svg" | "a") {
            if !t.self_closing {
                stack.push(m);
            }
            continue;
        }
        let seg = view.map(|v| v.w.max(v.h) / 160.).unwrap_or(1.0);
        let local: Vec<Poly> = match t.name.as_str() {
            "path" => path_polylines(t.get("d").unwrap_or(""), seg)?,
            "line" => vec![vec![
                (t.num("x1")?, t.num("y1")?),
                (t.num("x2")?, t.num("y2")?),
            ]],
            "polyline" | "polygon" => {
                let v = numbers(t.get("points").unwrap_or(""))?;
                let mut p: Poly = v.chunks_exact(2).map(|c| (c[0], c[1])).collect();
                if t.name == "polygon" && p.len() > 2 {
                    p.push(p[0]);
                }
                vec![p]
            }
            "rect" => {
                let (x, y, w, h) = (t.num("x")?, t.num("y")?, t.num("width")?, t.num("height")?);
                vec![vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]]
            }
            "text" => {
                if !t.self_closing {
                    skip.push("text".into());
                }
                text_polylines(src, &all, ti)?
            }
            "circle" | "ellipse" => {
                let (cx, cy) = (t.num("cx")?, t.num("cy")?);
                let (rx, ry) = if t.name == "circle" {
                    (t.num("r")?, t.num("r")?)
                } else {
                    (t.num("rx")?, t.num("ry")?)
                };
                let d = format!(
                    "M{} {} A{rx} {ry} 0 1 1 {} {} A{rx} {ry} 0 1 1 {} {}",
                    cx + rx,
                    cy,
                    cx - rx,
                    cy,
                    cx + rx,
                    cy
                );
                path_polylines(&d, seg)?
            }
            other => {
                warnings.push(format!("<{other}> is not supported and was skipped"));
                if !t.self_closing {
                    skip.push(other.to_string());
                }
                continue;
            }
        };
        for p in local {
            if p.len() >= 2 {
                polys.push(p.into_iter().map(|q| m.apply(q)).collect());
                text.push(t.name == "text");
            }
        }
    }
    if !seen_svg {
        return Err("not an SVG document (no <svg> element)".into());
    }
    if polys.is_empty() {
        return Err("the drawing has no supported shapes".into());
    }
    let view = match view {
        Some(v) => v,
        None => {
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for &(x, y) in polys.iter().flatten() {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            ViewBox {
                x: x0,
                y: y0,
                w: (x1 - x0).max(1e-3),
                h: (y1 - y0).max(1e-3),
            }
        }
    };
    Ok(Drawing {
        polys,
        text,
        view,
        warnings,
    })
}

/// Map a drawing into the box at `(x, y)`, `width` wide (and at most `height` tall, keeping
/// the aspect ratio), in the caller's units.
pub fn place(d: &Drawing, x: f32, y: f32, width: f32, height: Option<f32>) -> Vec<Poly> {
    let mut s = width / d.view.w;
    if let Some(h) = height {
        s = s.min(h / d.view.h);
    }
    d.polys
        .iter()
        .map(|p| {
            p.iter()
                .map(|&(px, py)| (x + (px - d.view.x) * s, y + (py - d.view.y) * s))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        dist(a, b) < 1e-3
    }

    #[test]
    fn numbers_split_on_signs_and_second_dots() {
        assert_eq!(
            numbers("1-2 3,4 .5.5 1e2").unwrap(),
            vec![1., -2., 3., 4., 0.5, 0.5, 100.]
        );
    }

    #[test]
    fn lines_relative_moves_and_close() {
        let p = path_polylines("M10 10 l5 0 v5 h-5 z m20 0 L 40 10 50 20", 1.).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            p[0],
            vec![(10., 10.), (15., 10.), (15., 15.), (10., 15.), (10., 10.)]
        );
        // implicit lineto after moveto, relative to the closed subpath's start
        assert_eq!(p[1], vec![(30., 10.), (40., 10.), (50., 20.)]);
    }

    #[test]
    fn curves_end_on_their_endpoints() {
        let p = path_polylines(
            "M0 0 C 0 10 10 10 10 0 S 20 -10 20 0 Q 25 5 30 0 T 40 0",
            1.,
        )
        .unwrap();
        assert_eq!(p.len(), 1);
        let pts = &p[0];
        assert!(pts.iter().any(|&q| close(q, (10., 0.))));
        assert!(pts.iter().any(|&q| close(q, (20., 0.))));
        assert!(close(*pts.last().unwrap(), (40., 0.)));
        assert!(pts.len() > 10);
    }

    #[test]
    fn two_arcs_make_a_circle() {
        let p = path_polylines("M6 7 A3 3 0 0 0 0 7 A3 3 0 0 0 6 7", 0.2).unwrap();
        for &(x, y) in &p[0] {
            assert!((dist((x, y), (3., 7.)) - 3.).abs() < 1e-2, "({x},{y})");
        }
        let ys: Vec<f32> = p[0].iter().map(|q| q.1).collect();
        assert!(ys.iter().any(|&y| y < 4.1) && ys.iter().any(|&y| y > 9.9));
    }

    #[test]
    fn documents_with_groups_transforms_and_skipped_elements() {
        let d = parse(
            r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50">
               <!-- a comment -->
               <g transform="translate(10,5)"><rect x="0" y="0" width="20" height="10" fill="none"/>
                 <g transform="scale(2)"><line x1="0" y1="0" x2="5" y2="0"/></g></g>
               <circle cx="50" cy="25" r="10"/>
               <title>hello <b>world</b></title><image href="x.png"/>
               <polygon points="0,0 10,0 5,8"/></svg>"#,
        )
        .unwrap();
        assert_eq!(
            d.view,
            ViewBox {
                x: 0.,
                y: 0.,
                w: 100.,
                h: 50.
            }
        );
        assert_eq!(d.polys[0][0], (10., 5.));
        assert_eq!(d.polys[1], vec![(10., 5.), (20., 5.)]);
        assert_eq!(d.polys.len(), 4);
        assert_eq!(d.polys[3].last(), Some(&(0., 0.)));
        assert!(d.warnings.iter().any(|w| w.contains("image")));
        let placed = place(&d, 1000., 2000., 200., None);
        assert_eq!(placed[0][0], (1020., 2010.));
    }

    #[test]
    fn rotate_about_a_point() {
        let m = parse_transform("rotate(90 10 10)").unwrap();
        assert!(close(m.apply((20., 10.)), (10., 20.)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("<g/>").is_err());
        assert!(parse("<svg></svg>").is_err());
        assert!(path_polylines("10 10", 1.).is_err());
        assert!(path_polylines("M0 0 X 4", 1.).is_err());
    }

    #[test]
    fn text_is_written_in_the_handwriting_font_and_aligned() {
        let src = "<svg viewBox='0 0 100 50'><rect x='10' y='10' width='80' height='30'/>\
                   <text x='50' y='25' font-size='10' text-anchor='middle' \
                   dominant-baseline='middle'>A<tspan>b</tspan> &amp; c</text></svg>";
        let d = parse(src).unwrap();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let ink: Vec<(f32, f32)> = d.polys[1..].iter().flatten().copied().collect();
        assert!(!ink.is_empty());
        let (x0, x1) = ink
            .iter()
            .fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (y0, y1) = ink
            .iter()
            .fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        // Centred on x = 50 (within the slant), capitals 7 tall centred on y = 25.
        assert!(((x0 + x1) / 2. - 50.).abs() < 1.5, "{x0} {x1}");
        assert!((y0 - 21.5).abs() < 0.2 && y1 < 29.5, "{y0} {y1}");
        assert!(x0 > 10. && x1 < 90.);
        assert_eq!(text_content(src, &tags(src).unwrap(), 2), "Ab & c");
        assert!(parse("<svg><rect width='5' height='5'/><text>☃</text></svg>").is_err());
    }
}
