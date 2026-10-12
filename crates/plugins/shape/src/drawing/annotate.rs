//! What a draughtsman adds on top of the views: tolerances on dimensions
//! (±, limit deviations, ISO 286 fits) written the ISO 129-1 way,
//! geometrical tolerances (ISO 1101 feature control frames and ISO 5459
//! datum feature symbols), and detail views: a circle of a view drawn again
//! at a larger standard scale, in free space on the sheet.

use super::*;

/// A dimension's tolerance.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Tolerance {
    /// ± this much (mm).
    Symmetric { value: f64 },
    /// Upper and lower deviations (mm, signed: +0.1 and −0.05).
    Limits { upper: f64, lower: f64 },
    /// An ISO 286 tolerance class: "H7" for a hole, "g6" for a shaft.
    Fit { fit: String },
}

/// A detail view: the circle of radius `radius` about `center` (model mm
/// along the view's axes) drawn again larger, labelled DETAIL B (5:1).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct DetailSpec {
    pub view: ViewName,
    pub center: [f64; 2],
    pub radius: f64,
    /// The detail's own scale (5 for 5:1); a standard one that fits is chosen when absent.
    #[serde(default)]
    pub scale: Option<f64>,
    /// Where the person put the enlarged view: its centre on the sheet (sheet
    /// mm); it is found free space when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<[f64; 2]>,
}

/// Geometrical characteristics (ISO 1101).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GdtSymbol {
    Straightness,
    #[default]
    Flatness,
    Circularity,
    Cylindricity,
    LineProfile,
    SurfaceProfile,
    Parallelism,
    Perpendicularity,
    Angularity,
    Position,
    #[serde(alias = "coaxiality")]
    Concentricity,
    Symmetry,
    CircularRunout,
    TotalRunout,
}

/// Material condition on a tolerance (ISO 2692): Ⓜ or Ⓛ.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    /// Maximum material requirement.
    Mmc,
    /// Least material requirement.
    Lmc,
}

/// A feature control frame pointing at a point of a view (model mm along its axes).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct FrameSpec {
    pub view: ViewName,
    pub at: [f64; 2],
    pub symbol: GdtSymbol,
    /// The tolerance zone's size (mm).
    pub tolerance: f64,
    /// A cylindrical zone: the value is written ⌀0.05.
    #[serde(default)]
    pub diameter_zone: bool,
    /// Datum references in order of precedence ("A", "B", "C"; "A-B" for a common datum).
    #[serde(default)]
    pub datums: Vec<String>,
    #[serde(default)]
    pub modifier: Option<Modifier>,
    /// Where the person put the frame: its lower left corner from the point
    /// it applies to (sheet mm); it is found free space when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<[f64; 2]>,
}

/// A datum feature symbol (ISO 5459) on a point of a view.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct DatumSpec {
    pub view: ViewName,
    pub at: [f64; 2],
    pub letter: String,
    /// Where the person put the lettered square: its centre from the point
    /// on the feature (sheet mm); straight out from the feature when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<[f64; 2]>,
}

/// Where a detail view landed: a model point p (mm along its view's axes)
/// shows at `at + p · scale`, inside the circle of `radius` about `centre`.
#[derive(Clone, Debug, Serialize)]
pub struct DetailFrame {
    /// Its index in the details.
    pub index: usize,
    pub letter: String,
    pub view: ViewName,
    pub at: [f64; 2],
    pub scale: f64,
    pub axes: [usize; 2],
    pub centre: [f64; 2],
    pub radius: f64,
}

/// A fit on holes of one diameter: their callout reads ⌀16 H7 with its deviations.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct HoleFit {
    pub diameter: f64,
    pub fit: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Detail,
    Frame,
    Datum,
    /// A hole callout (carries the hole's diameter).
    Hole,
    /// A surface texture symbol.
    Finish,
    /// A weld symbol.
    Weld,
}

/// Where an annotation landed: its index in the options' list and its box on the sheet.
#[derive(Clone, Debug, Serialize)]
pub struct AnnotationMark {
    pub kind: MarkKind,
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diameter: Option<f64>,
    pub bounds: [f64; 4],
    /// What moves when it is dragged (sheet mm): a frame's lower left corner,
    /// a datum's square's centre, a detail view's centre.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<[f64; 2]>,
    /// The point on the view it belongs to (sheet mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f64; 2]>,
}

impl DimSpec {
    /// The length the dimension states (model mm).
    pub fn value(&self) -> f64 {
        match self.kind {
            DimKind::Horizontal => (self.a[0] - self.b[0]).abs(),
            DimKind::Vertical => (self.a[1] - self.b[1]).abs(),
            DimKind::Aligned => (self.a[0] - self.b[0]).hypot(self.a[1] - self.b[1]),
        }
    }
}

fn letter_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 5 && s.split('-').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_uppercase()))
}

/// Gives datum `from` the letter `to`: its symbols and every frame's
/// references to it (also inside a common datum, A-B). How many references changed.
pub fn rename_datum(frames: &mut [FrameSpec], datums: &mut [DatumSpec], from: &str, to: &str) -> usize {
    for d in datums.iter_mut().filter(|d| d.letter == from) {
        d.letter = to.to_string();
    }
    let mut changed = 0;
    for f in frames.iter_mut() {
        for r in f.datums.iter_mut() {
            let parts: Vec<&str> = r.split('-').collect();
            if parts.contains(&from) {
                *r = parts.iter().map(|p| if *p == from { to } else { p }).collect::<Vec<_>>().join("-");
                changed += 1;
            }
        }
    }
    changed
}

/// Checks a drawing setup's annotations before it is kept.
pub fn validate(dimensions: &[DimSpec], details: &[DetailSpec], frames: &[FrameSpec], datums: &[DatumSpec], hole_fits: &[HoleFit]) -> Result<(), String> {
    let finite = |p: Option<[f64; 2]>| p.is_none_or(|p| p.iter().all(|v| v.is_finite()));
    if !(details.iter().all(|d| finite(d.place)) && frames.iter().all(|f| finite(f.place)) && datums.iter().all(|d| finite(d.place))) {
        return Err("a placement is two numbers of sheet mm".into());
    }
    for d in dimensions {
        if let Some(i) = d.detail {
            match details.get(i) {
                None => return Err(format!("a dimension is in detail {}, and there are {} details", i, details.len())),
                Some(s) if s.view != d.view => return Err("a dimension in a detail view is on the detail's view".into()),
                _ => {}
            }
        }
        match &d.tolerance {
            Some(Tolerance::Symmetric { value }) if !(value.is_finite() && *value > 0.0) => return Err("a ± tolerance is a positive number of mm".into()),
            Some(Tolerance::Limits { upper, lower }) if !(upper.is_finite() && lower.is_finite() && upper > lower) => return Err("limit deviations need the upper above the lower".into()),
            Some(Tolerance::Fit { fit }) => {
                fits::deviations(fit, d.value())?;
            }
            _ => {}
        }
    }
    for d in details {
        if !(d.radius.is_finite() && d.radius > 0.0) {
            return Err("a detail's radius is a positive number of mm".into());
        }
        if d.scale.is_some_and(|k| !(k.is_finite() && k > 0.0)) {
            return Err("a detail's scale is a positive number (5 for 5:1)".into());
        }
    }
    for f in frames {
        if !(f.tolerance.is_finite() && f.tolerance > 0.0) {
            return Err("a geometrical tolerance is a positive number of mm".into());
        }
        if f.datums.len() > 3 || !f.datums.iter().all(|l| letter_ok(l)) {
            return Err("datum references are up to three capital letters (A, B, C, or A-B)".into());
        }
    }
    for d in datums {
        if !letter_ok(&d.letter) {
            return Err(format!("“{}” is not a datum letter (A, B, C…)", d.letter));
        }
    }
    for h in hole_fits {
        if !(h.diameter > 0.0) {
            return Err("a hole fit needs the hole's diameter".into());
        }
        fits::deviations(&h.fit, h.diameter)?;
    }
    Ok(())
}

// ---- Tolerances on values ---------------------------------------------------

/// A value as written on the drawing: the nominal (with ⌀ and the fit's
/// class), and stacked deviations after it.
pub(super) struct Label {
    pub main: String,
    pub devs: Option<[String; 2]>,
    /// The deviations follow a tolerance class: in parentheses (ISO 129-1).
    pub paren: bool,
}

/// A number to `places` decimals, up to 4, without trailing zeros past them.
fn decimals(v: f64) -> usize {
    let s = format!("{:.4}", v.abs());
    let t = s.trim_end_matches('0');
    t.split('.').nth(1).map_or(0, str::len)
}

fn deviation(v: f64, places: usize) -> String {
    if v.abs() < 5e-7 {
        "0".into()
    } else {
        format!("{}{:.*}", if v > 0.0 { "+" } else { "−" }, places, v.abs())
    }
}

/// How a value with its tolerance is written.
pub(super) fn label(value: f64, diameter: bool, tol: Option<&Tolerance>) -> Label {
    let nominal = format!("{}{}", if diameter { "⌀" } else { "" }, length_text(value));
    let plain = |main: String| Label { main, devs: None, paren: false };
    let limits = |main: String, upper: f64, lower: f64, paren: bool| {
        if upper > 0.0 && (upper + lower).abs() < 5e-7 {
            let pm = format!("±{:.*}", decimals(upper), upper);
            return plain(if paren { format!("{main} ({pm})") } else { format!("{main} {pm}") });
        }
        let places = decimals(upper).max(decimals(lower)).max(1);
        Label { main, devs: Some([deviation(upper, places), deviation(lower, places)]), paren }
    };
    match tol {
        None => plain(nominal),
        Some(Tolerance::Symmetric { value: t }) => plain(format!("{nominal} ±{:.*}", decimals(*t), t.abs())),
        Some(Tolerance::Limits { upper, lower }) => limits(nominal, *upper, *lower, false),
        Some(Tolerance::Fit { fit }) => match fits::deviations(fit, value) {
            Ok((u, l)) => limits(format!("{nominal} {}", fit.trim()), u, l, true),
            Err(_) => plain(format!("{nominal} {}", fit.trim())),
        },
    }
}

/// Drawing symbols in lettering that no standard font has (ISO 129-1 hole
/// callouts): counterbore, countersink and depth.
pub(super) const GLYPHS: [char; 3] = ['⌴', '⌵', '↧'];

/// Writes text, drawing the callout symbols in it as strokes (the rest as text runs).
pub(super) fn push_text(d: &mut Drawing, t: Text) {
    if !t.text.contains(GLYPHS) {
        d.texts.push(t);
        return;
    }
    let total = text_width(&t);
    let x0 = match t.anchor {
        Anchor::Start => 0.0,
        Anchor::Middle => -total / 2.0,
        Anchor::End => -total,
    };
    let (sn, cs) = t.angle.to_radians().sin_cos();
    let p = |x: f64, y: f64| [t.at[0] + x * cs - y * sn, t.at[1] + x * sn + y * cs];
    let h = t.size;
    let gw = glyph_width(h);
    let mut x = x0;
    let mut run = String::new();
    let flush = |d: &mut Drawing, run: &mut String, x: &mut f64| {
        if run.is_empty() {
            return;
        }
        let piece = Text { at: p(*x, 0.0), size: h, text: std::mem::take(run), anchor: Anchor::Start, angle: t.angle, bold: t.bold };
        *x += text_width(&piece);
        d.texts.push(piece);
    };
    for ch in t.text.chars() {
        if !GLYPHS.contains(&ch) {
            run.push(ch);
            continue;
        }
        flush(d, &mut run, &mut x);
        let g = |u: f64, v: f64| p(x + u * gw, v * h);
        let lines: Vec<Vec<[f64; 2]>> = match ch {
            '⌴' => vec![vec![g(0.12, 0.95), g(0.12, 0.0), g(0.88, 0.0), g(0.88, 0.95)]],
            '⌵' => vec![vec![g(0.08, 0.95), g(0.5, 0.0), g(0.92, 0.95)]],
            _ => vec![vec![g(0.5, 1.0), g(0.5, 0.0)], vec![g(0.26, 0.4), g(0.5, 0.0), g(0.74, 0.4)], vec![g(0.15, 0.0), g(0.85, 0.0)]],
        };
        for points in lines {
            d.strokes.push(Stroke { pen: Pen::Thin, points });
        }
        x += gw;
    }
    flush(d, &mut run, &mut x);
}

/// How wide a callout symbol is at cap height `h` (mm).
pub(super) fn glyph_width(h: f64) -> f64 {
    h * 1.05
}

const DEV_GAP: f64 = 1.0; // nominal to deviations
const SAG: f64 = 0.6; // how far a drawn parenthesis bows

fn dev_size(size: f64) -> f64 {
    (size * 5.0 / 7.0).max(1.8)
}

/// How wide a label is along its baseline (mm).
pub(super) fn label_width(l: &Label, size: f64) -> f64 {
    let t = |s: &str, size: f64| text_width(&Text { at: [0.0; 2], size, text: s.into(), anchor: Anchor::Start, angle: 0.0, bold: false });
    let mut w = t(&l.main, size);
    if let Some([u, lo]) = &l.devs {
        let wd = t(u, dev_size(size)).max(t(lo, dev_size(size)));
        w += DEV_GAP + wd + if l.paren { 2.0 * SAG + 1.4 } else { 0.0 };
    }
    w
}

/// Writes a label with its baseline through `at` (anchored there), turned `angle`°.
pub(super) fn place_label(d: &mut Drawing, l: &Label, at: [f64; 2], anchor: Anchor, angle: f64, size: f64) {
    if l.devs.is_none() {
        push_text(d, Text { at, size, text: l.main.clone(), anchor, angle, bold: false });
        return;
    }
    let (sn, cs) = angle.to_radians().sin_cos();
    let p = |x: f64, y: f64| [at[0] + x * cs - y * sn, at[1] + x * sn + y * cs];
    let w = label_width(l, size);
    let x0 = match anchor {
        Anchor::Start => 0.0,
        Anchor::Middle => -w / 2.0,
        Anchor::End => -w,
    };
    push_text(d, Text { at: p(x0, 0.0), size, text: l.main.clone(), anchor: Anchor::Start, angle, bold: false });
    let ds = dev_size(size);
    let Some([upper, lower]) = &l.devs else { return };
    let tw = |s: &str| text_width(&Text { at: [0.0; 2], size: ds, text: s.into(), anchor: Anchor::Start, angle: 0.0, bold: false });
    let mut x = x0 + text_width(&Text { at: [0.0; 2], size, text: l.main.clone(), anchor: Anchor::Start, angle: 0.0, bold: false }) + DEV_GAP;
    // Lower deviation on the nominal's baseline, the upper one above it.
    let (yl, yu) = (-0.3, -0.3 + ds + 0.9);
    let (pb, pt) = (yl - 0.6, yu + ds + 0.6);
    let paren = |d: &mut Drawing, x: f64, open: bool| {
        // An arc through the chord ends, bowed SAG outward.
        let h = pt - pb;
        let r = (h * h / 4.0 + SAG * SAG) / (2.0 * SAG);
        let a = (h / 2.0 / r).asin();
        let (cx, cy) = if open { (x + r, (pb + pt) / 2.0) } else { (x + SAG - r, (pb + pt) / 2.0) };
        let points = (0..=12)
            .map(|i| {
                let t = -a + 2.0 * a * i as f64 / 12.0;
                if open { p(cx - r * t.cos(), cy + r * t.sin()) } else { p(cx + r * t.cos(), cy + r * t.sin()) }
            })
            .collect();
        d.strokes.push(Stroke { pen: Pen::Thin, points });
    };
    if l.paren {
        paren(d, x, true);
        x += SAG + 0.7;
    }
    let wd = tw(upper).max(tw(lower));
    d.texts.push(Text { at: p(x, yu), size: ds, text: upper.clone(), anchor: Anchor::Start, angle, bold: false });
    d.texts.push(Text { at: p(x, yl), size: ds, text: lower.clone(), anchor: Anchor::Start, angle, bold: false });
    if l.paren {
        paren(d, x + wd + 0.7, false);
    }
}

// ---- Free space on the sheet -------------------------------------------------

/// The sheet in 1 mm cells, set where something is drawn.
#[derive(Clone)]
pub(super) struct Grid {
    cols: usize,
    rows: usize,
    occ: Vec<u8>,
    sat: Vec<u32>,
    fresh: bool,
}

impl Grid {
    pub(super) fn new(w: f64, h: f64) -> Grid {
        let (cols, rows) = (w.ceil() as usize + 1, h.ceil() as usize + 1);
        Grid { cols, rows, occ: vec![0; cols * rows], sat: vec![], fresh: false }
    }
    pub(super) fn range(&self, r: [f64; 4]) -> Option<(usize, usize, usize, usize)> {
        let x0 = r[0].floor().max(0.0) as usize;
        let y0 = r[1].floor().max(0.0) as usize;
        let x1 = (r[2].ceil().max(0.0) as usize).min(self.cols);
        let y1 = (r[3].ceil().max(0.0) as usize).min(self.rows);
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }
    pub(super) fn mark(&mut self, r: [f64; 4]) {
        if let Some((x0, y0, x1, y1)) = self.range(r) {
            for y in y0..y1 {
                self.occ[y * self.cols + x0..y * self.cols + x1].fill(1);
            }
            self.fresh = false;
        }
    }
    pub(super) fn mark_line(&mut self, a: [f64; 2], b: [f64; 2], pad: f64) {
        let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / 0.5).ceil() as usize + 1;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            self.mark([p[0] - pad, p[1] - pad, p[0] + pad, p[1] + pad]);
        }
    }
    pub(super) fn mark_drawn(&mut self, d: &Drawing, strokes: std::ops::Range<usize>, texts: std::ops::Range<usize>) {
        for s in &d.strokes[strokes] {
            for w in s.points.windows(2) {
                self.mark_line(w[0], w[1], 0.4);
            }
        }
        for t in &d.texts[texts] {
            self.mark(text_box(t));
        }
    }
    /// Nothing drawn along the line from a to b, leaving out `skip_a` and `skip_b` mm at its ends.
    pub(super) fn line_free(&mut self, a: [f64; 2], b: [f64; 2], skip_a: f64, skip_b: f64) -> bool {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if l <= skip_a + skip_b {
            return true;
        }
        let n = ((l - skip_a - skip_b) / 0.7).ceil() as usize;
        (0..=n).all(|i| {
            let t = (skip_a + (l - skip_a - skip_b) * i as f64 / n.max(1) as f64) / l;
            let p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            self.free([p[0] - 0.2, p[1] - 0.2, p[0] + 0.2, p[1] + 0.2])
        })
    }
    /// The grid as a PGM picture (debugging).
    pub(super) fn dump(&self, path: &str) {
        let mut out = format!("P5 {} {} 255\n", self.cols, self.rows).into_bytes();
        for y in (0..self.rows).rev() {
            out.extend(self.occ[y * self.cols..(y + 1) * self.cols].iter().map(|v| if *v > 0 { 0u8 } else { 255 }));
        }
        let _ = std::fs::write(path, out);
    }
    /// How many cells in the box have something drawn (cells off the sheet count as drawn).
    pub(super) fn used(&mut self, r: [f64; 4]) -> usize {
        let off = r[0] < 0.0 || r[1] < 0.0 || r[2] > (self.cols - 1) as f64 || r[3] > (self.rows - 1) as f64;
        if self.free(r) {
            return if off { usize::MAX / 2 } else { 0 };
        }
        let Some((x0, y0, x1, y1)) = self.range(r) else { return 0 };
        let w = self.cols + 1;
        (self.sat[y1 * w + x1] + self.sat[y0 * w + x0] - self.sat[y0 * w + x1] - self.sat[y1 * w + x0]) as usize + if off { usize::MAX / 2 } else { 0 }
    }
    /// How many drawn cells the line from a to b passes over, leaving out `skip_a` and `skip_b` mm at its ends.
    pub(super) fn hits(&mut self, a: [f64; 2], b: [f64; 2], skip_a: f64, skip_b: f64) -> usize {
        let l = (b[0] - a[0]).hypot(b[1] - a[1]);
        if l <= skip_a + skip_b {
            return 0;
        }
        let n = ((l - skip_a - skip_b) / 0.7).ceil() as usize;
        (0..=n)
            .filter(|i| {
                let t = (skip_a + (l - skip_a - skip_b) * *i as f64 / n.max(1) as f64) / l;
                let p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                !self.free([p[0] - 0.2, p[1] - 0.2, p[0] + 0.2, p[1] + 0.2])
            })
            .count()
    }
    /// Nothing drawn in the box (and it is on the sheet).
    pub(super) fn free(&mut self, r: [f64; 4]) -> bool {
        if r[0] < 0.0 || r[1] < 0.0 || r[2] > (self.cols - 1) as f64 || r[3] > (self.rows - 1) as f64 {
            return false;
        }
        if !self.fresh {
            let w = self.cols + 1;
            self.sat = vec![0; w * (self.rows + 1)];
            for y in 0..self.rows {
                let mut run = 0;
                for x in 0..self.cols {
                    run += self.occ[y * self.cols + x] as u32;
                    self.sat[(y + 1) * w + x + 1] = self.sat[y * w + x + 1] + run;
                }
            }
            self.fresh = true;
        }
        let Some((x0, y0, x1, y1)) = self.range(r) else { return true };
        let w = self.cols + 1;
        self.sat[y1 * w + x1] + self.sat[y0 * w + x0] == self.sat[y0 * w + x1] + self.sat[y1 * w + x0]
    }
}

/// What layout() hands over: where the views' geometry strokes are, the
/// section's cut outline (sheet mm, on the front view), and areas kept clear.
pub(super) struct Geometry {
    pub strokes: std::ops::Range<usize>,
    pub section: Vec<[[f64; 2]; 2]>,
    /// The sheet frame's inside: [x0, y0, x1, y1].
    pub frame: [f64; 4],
    /// The title block and parts list.
    pub reserved: Vec<[f64; 4]>,
}

fn grow(r: [f64; 4], m: f64) -> [f64; 4] {
    [r[0] - m, r[1] - m, r[2] + m, r[3] + m]
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn within(r: [f64; 4], f: [f64; 4]) -> bool {
    r[0] >= f[0] && r[1] >= f[1] && r[2] <= f[2] && r[3] <= f[3]
}

fn bounds_of(d: &Drawing, strokes: std::ops::Range<usize>, texts: std::ops::Range<usize>, fills: std::ops::Range<usize>) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.strokes[strokes].iter().flat_map(|s| &s.points).chain(d.fills[fills].iter().flatten()) {
        b = union(b, [p[0], p[1], p[0], p[1]]);
    }
    for t in &d.texts[texts] {
        b = union(b, text_box(t));
    }
    b
}

fn circle(c: [f64; 2], r: f64) -> Vec<[f64; 2]> {
    let n = ((r * 6.0) as usize).clamp(24, 160);
    (0..=n).map(|i| i as f64 / n as f64 * std::f64::consts::TAU).map(|a| [c[0] + r * a.cos(), c[1] + r * a.sin()]).collect()
}

/// The parts of a polyline inside a circle.
pub(super) fn clip_to_circle(points: &[[f64; 2]], c: [f64; 2], r: f64) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = vec![];
    let mut open = false;
    for w in points.windows(2) {
        let (p, q) = (w[0], w[1]);
        let dv = [q[0] - p[0], q[1] - p[1]];
        let f = [p[0] - c[0], p[1] - c[1]];
        let a = dv[0] * dv[0] + dv[1] * dv[1];
        let b = 2.0 * (f[0] * dv[0] + f[1] * dv[1]);
        let cc = f[0] * f[0] + f[1] * f[1] - r * r;
        let span = if a < 1e-18 {
            (cc <= 0.0).then_some((0.0, 1.0))
        } else {
            let disc = b * b - 4.0 * a * cc;
            if disc <= 0.0 {
                None
            } else {
                let s = disc.sqrt();
                let (t0, t1) = (((-b - s) / (2.0 * a)).max(0.0), ((-b + s) / (2.0 * a)).min(1.0));
                (t0 < t1).then_some((t0, t1))
            }
        };
        let Some((t0, t1)) = span else {
            open = false;
            continue;
        };
        let at = |t: f64| [p[0] + t * dv[0], p[1] + t * dv[1]];
        if open && t0 <= 1e-12 {
            out.last_mut().unwrap().push(at(t1));
        } else {
            out.push(vec![at(t0), at(t1)]);
        }
        open = t1 >= 1.0 - 1e-12;
    }
    out.retain(|l| l.len() >= 2 && l.windows(2).any(|w| (w[0][0] - w[1][0]).hypot(w[0][1] - w[1][1]) > 1e-6));
    out
}

// ---- Geometrical tolerances ----------------------------------------------------

const CELL_H: f64 = 7.0; // frame height: twice the 3.5 mm lettering (ISO 1101)

/// A geometrical characteristic's symbol, drawn about `c` (3.5 mm lettering), as thin strokes.
fn symbol(d: &mut Drawing, sym: GdtSymbol, c: [f64; 2]) {
    let mut line = |pts: Vec<[f64; 2]>| d.strokes.push(Stroke { pen: Pen::Thin, points: pts.into_iter().map(|p| [c[0] + p[0], c[1] + p[1]]).collect() });
    let ring = |r: f64, cx: f64, cy: f64| circle([cx, cy], r).into_iter().collect::<Vec<_>>();
    let (s60, c60) = 60f64.to_radians().sin_cos();
    match sym {
        GdtSymbol::Straightness => line(vec![[-2.0, 0.0], [2.0, 0.0]]),
        GdtSymbol::Flatness => line(vec![[-2.3, -1.1], [1.0, -1.1], [2.3, 1.1], [-1.0, 1.1], [-2.3, -1.1]]),
        GdtSymbol::Circularity => line(ring(1.75, 0.0, 0.0)),
        GdtSymbol::Cylindricity => {
            let r = 1.35;
            line(ring(r, 0.0, 0.0));
            // Two tangents at 60°, either side of the circle.
            let n = [-s60, c60];
            for k in [-1.0, 1.0] {
                let o = [n[0] * r * k, n[1] * r * k];
                line(vec![[o[0] - c60 * 2.1, o[1] - s60 * 2.1], [o[0] + c60 * 2.1, o[1] + s60 * 2.1]]);
            }
        }
        GdtSymbol::LineProfile | GdtSymbol::SurfaceProfile => {
            let arc: Vec<[f64; 2]> = (0..=24).map(|i| std::f64::consts::PI * i as f64 / 24.0).map(|a| [2.0 * a.cos(), -1.0 + 2.0 * a.sin()]).collect();
            if sym == GdtSymbol::SurfaceProfile {
                let mut closed = arc;
                closed.push([2.0, -1.0]);
                line(closed);
            } else {
                line(arc);
            }
        }
        GdtSymbol::Parallelism => {
            for x in [-0.8, 0.8] {
                line(vec![[x - c60 * 2.1, -s60 * 2.1], [x + c60 * 2.1, s60 * 2.1]]);
            }
        }
        GdtSymbol::Perpendicularity => {
            line(vec![[0.0, -1.75], [0.0, 1.9]]);
            line(vec![[-2.0, -1.75], [2.0, -1.75]]);
        }
        GdtSymbol::Angularity => {
            let (s30, c30) = 30f64.to_radians().sin_cos();
            line(vec![[-2.1 + 4.4 * c30, -1.5 + 4.4 * s30], [-2.1, -1.5], [2.3, -1.5]]);
        }
        GdtSymbol::Position => {
            line(ring(1.35, 0.0, 0.0));
            line(vec![[-2.2, 0.0], [2.2, 0.0]]);
            line(vec![[0.0, -2.2], [0.0, 2.2]]);
        }
        GdtSymbol::Concentricity => {
            line(ring(0.9, 0.0, 0.0));
            line(ring(1.9, 0.0, 0.0));
        }
        GdtSymbol::Symmetry => {
            line(vec![[-1.4, 1.1], [1.4, 1.1]]);
            line(vec![[-2.2, 0.0], [2.2, 0.0]]);
            line(vec![[-1.4, -1.1], [1.4, -1.1]]);
        }
        GdtSymbol::CircularRunout | GdtSymbol::TotalRunout => {
            // Arrows at 45°, open heads; total run-out has two on a base line.
            let arrow = |line: &mut dyn FnMut(Vec<[f64; 2]>), x: f64| {
                let (a, b) = ([x - 1.2, -1.6], [x + 1.4, 1.0]);
                line(vec![a, b]);
                let dir = std::f64::consts::FRAC_PI_4;
                for k in [-1.0, 1.0] {
                    let t = dir + std::f64::consts::PI + k * 0.4;
                    line(vec![b, [b[0] + 1.2 * t.cos(), b[1] + 1.2 * t.sin()]]);
                }
            };
            if sym == GdtSymbol::CircularRunout {
                arrow(&mut line, 0.0);
            } else {
                arrow(&mut line, -0.9);
                arrow(&mut line, 1.1);
                line(vec![[-2.1, -1.6], [-0.1 + 0.2, -1.6]]);
            }
        }
    }
}

/// A tolerance value as a frame writes it: 0.05, 0.1, 0.012.
fn tol_text(v: f64) -> String {
    let s = format!("{v:.4}");
    let t = s.trim_end_matches('0').trim_end_matches('.');
    if t.starts_with('.') { format!("0{t}") } else { t.to_string() }
}

fn plain_width(s: &str, size: f64) -> f64 {
    text_width(&Text { at: [0.0; 2], size, text: s.into(), anchor: Anchor::Start, angle: 0.0, bold: false })
}

/// The compartments of a frame: symbol, tolerance, then one per datum.
fn frame_cells(f: &FrameSpec) -> (String, Vec<f64>) {
    let value = format!("{}{}", if f.diameter_zone { "⌀" } else { "" }, tol_text(f.tolerance));
    let mut widths = vec![CELL_H, plain_width(&value, 3.5) + 3.4 + if f.modifier.is_some() { 4.0 } else { 0.0 }];
    for l in &f.datums {
        widths.push((plain_width(l, 3.5) + 3.0).max(CELL_H));
    }
    (value, widths)
}

/// Draws a feature control frame with its lower left at `o`.
fn draw_frame(d: &mut Drawing, f: &FrameSpec, o: [f64; 2]) {
    let (value, widths) = frame_cells(f);
    let w: f64 = widths.iter().sum();
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![o, [o[0] + w, o[1]], [o[0] + w, o[1] + CELL_H], [o[0], o[1] + CELL_H], o] });
    let mut x = o[0];
    let mut edges = vec![];
    for cw in &widths {
        edges.push((x, *cw));
        x += cw;
        if x < o[0] + w - 1e-9 {
            d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, o[1]], [x, o[1] + CELL_H]] });
        }
    }
    let base = o[1] + (CELL_H - 3.5) / 2.0;
    symbol(d, f.symbol, [edges[0].0 + CELL_H / 2.0, o[1] + CELL_H / 2.0]);
    let (tx, tw) = edges[1];
    let text_w = plain_width(&value, 3.5);
    let mod_w = if f.modifier.is_some() { 4.0 } else { 0.0 };
    let start = tx + (tw - text_w - mod_w) / 2.0;
    d.texts.push(Text { at: [start, base], size: 3.5, text: value, anchor: Anchor::Start, angle: 0.0, bold: false });
    if let Some(m) = f.modifier {
        let c = [start + text_w + 0.6 + 1.8, o[1] + CELL_H / 2.0];
        d.strokes.push(Stroke { pen: Pen::Thin, points: circle(c, 1.8) });
        d.texts.push(Text { at: [c[0], c[1] - 1.0], size: 2.0, text: if m == Modifier::Mmc { "M" } else { "L" }.into(), anchor: Anchor::Middle, angle: 0.0, bold: false });
    }
    for (l, (cx, cw)) in f.datums.iter().zip(&edges[2..]) {
        d.texts.push(Text { at: [cx + cw / 2.0, base], size: 3.5, text: l.clone(), anchor: Anchor::Middle, angle: 0.0, bold: false });
    }
}

/// The side of the view a point is nearest to, outward first: (direction, distance).
/// With whether the point lies on that side's outline (a leader from there crosses nothing of the view).
fn sides(p: [f64; 2], b: [f64; 4]) -> Vec<([f64; 2], bool)> {
    let mut s = vec![([0.0, 1.0], b[3] - p[1]), ([0.0, -1.0], p[1] - b[1]), ([1.0, 0.0], b[2] - p[0]), ([-1.0, 0.0], p[0] - b[0])];
    s.sort_by(|a, b| a.1.total_cmp(&b.1));
    s.into_iter().map(|(o, gap)| (o, gap < 0.6)).collect()
}

/// A frame's leader: straight down (or up) from under the frame when the
/// feature is beneath it, else squared off the frame's nearer end; it ends
/// at the feature, where the arrow goes.
fn frame_leader(at: [f64; 2], w: f64, p: [f64; 2]) -> Vec<[f64; 2]> {
    let mid = at[1] + CELL_H / 2.0;
    if p[0] >= at[0] + 1.0 && p[0] <= at[0] + w - 1.0 {
        let y = if p[1] < at[1] { at[1] } else { at[1] + CELL_H };
        vec![[p[0], y], p]
    } else if p[0] < at[0] {
        vec![[at[0], mid], [at[0] - 3.0, mid], p]
    } else {
        vec![[at[0] + w, mid], [at[0] + w + 3.0, mid], p]
    }
}

/// Moves a box (x0, y0, x1, y1) the least that puts it inside `f`: how far, or None when it is too big.
fn keep_inside(r: [f64; 4], f: [f64; 4]) -> Option<[f64; 2]> {
    if r[2] - r[0] > f[2] - f[0] || r[3] - r[1] > f[3] - f[1] {
        return None;
    }
    Some([(f[0] - r[0]).max(0.0) + (f[2] - r[2]).min(0.0), (f[1] - r[1]).max(0.0) + (f[3] - r[3]).min(0.0)])
}

fn place_frame(d: &mut Drawing, grid: &mut Grid, words: &mut Grid, g: &Geometry, f: &FrameSpec, index: usize) -> bool {
    let Some(v) = d.views.iter().find(|v| v.view == f.view).cloned() else { return false };
    let p = [v.at[0] + f.at[0] * v.scale, v.at[1] + f.at[1] * v.scale];
    let (_, widths) = frame_cells(f);
    let w: f64 = widths.iter().sum();
    let vb = v.bounds;
    // Where the person put it (kept on the sheet), else the best free spot.
    let placed = f.place.and_then(|o| {
        let at = [p[0] + o[0], p[1] + o[1]];
        let shift = keep_inside([at[0], at[1], at[0] + w, at[1] + CELL_H], grow(g.frame, -1.5))?;
        Some(([at[0] + shift[0], at[1] + shift[1]], 0.0))
    });
    if let Some((at, _)) = placed {
        if !grid.free(grow([at[0], at[1], at[0] + w, at[1] + CELL_H], 0.5)) {
            d.warnings.push(format!("the {} frame where it was put covers other lines", format!("{:?}", f.symbol).to_lowercase()));
        }
        draw_placed_frame(d, grid, words, f, index, at, w, p);
        return true;
    }
    // Of the free spots whose leader crosses no lettering, the one with the
    // shortest leader (the nearest side of the view a little preferred).
    let mut spot: Option<([f64; 2], f64)> = None;
    let around = sides(p, vb);
    let on_outline = around.iter().any(|(_, on)| *on);
    for (rank, (o, on)) in around.into_iter().enumerate() {
        // From a side the feature is not on, the leader crosses the view: only when nothing else will do.
        let penalty = if on_outline && !on { 60.0 } else { rank as f64 * 6.0 };
        for dist in [6.0, 10.0, 15.0, 21.0, 28.0, 36.0] {
            let options: Vec<[f64; 2]> = if o[1] != 0.0 {
                let y0 = if o[1] > 0.0 { vb[3] + dist } else { vb[1] - dist - CELL_H };
                [p[0] - CELL_H / 2.0, p[0] + 8.0, p[0] - w - 8.0, p[0] + 16.0, p[0] - w - 16.0].into_iter().map(|x| [x, y0]).collect()
            } else {
                let x0 = if o[0] > 0.0 { vb[2] + dist } else { vb[0] - dist - w };
                [p[1] - CELL_H / 2.0, p[1] + 6.0, p[1] - CELL_H - 6.0, p[1] + 13.0, p[1] - CELL_H - 13.0].into_iter().map(|y| [x0, y]).collect()
            };
            for at in options {
                let r = [at[0], at[1], at[0] + w, at[1] + CELL_H];
                if !within(grow(r, 1.5), g.frame) || !grid.free(grow(r, 1.2)) {
                    continue;
                }
                let leader = frame_leader(at, w, p);
                if !leader.windows(2).all(|s| words.line_free(s[0], s[1], 0.0, 1.5)) {
                    continue;
                }
                let length: f64 = leader.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum::<f64>() + penalty;
                if spot.is_none_or(|(_, l)| length < l - 1e-9) {
                    spot = Some((at, length));
                }
            }
        }
    }
    let Some((at, _)) = spot else { return false };
    draw_placed_frame(d, grid, words, f, index, at, w, p);
    true
}

/// Draws a frame with its lower left at `at`, its leader to `p`, and marks it.
#[allow(clippy::too_many_arguments)]
fn draw_placed_frame(d: &mut Drawing, grid: &mut Grid, words: &mut Grid, f: &FrameSpec, index: usize, at: [f64; 2], w: f64, p: [f64; 2]) {
    let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
    draw_frame(d, f, at);
    let leader = frame_leader(at, w, p);
    let n = leader.len();
    let (a, b) = (leader[n - 2], leader[n - 1]);
    let l = (b[0] - a[0]).hypot(b[1] - a[1]);
    if l > 1e-6 {
        d.strokes.push(Stroke { pen: Pen::Thin, points: leader.clone() });
        arrow(d, b, [(b[0] - a[0]) / l, (b[1] - a[1]) / l]);
    }
    grid.mark_drawn(d, s0..d.strokes.len(), t0..d.texts.len());
    words.mark_drawn(d, s0..s0, t0..d.texts.len());
    let bounds = bounds_of(d, s0..d.strokes.len(), t0..d.texts.len(), f0..d.fills.len());
    d.annotations.push(AnnotationMark { kind: MarkKind::Frame, index, diameter: None, bounds, origin: Some(at), anchor: Some(p) });
}

fn place_datum(d: &mut Drawing, grid: &mut Grid, words: &mut Grid, g: &Geometry, s: &DatumSpec, index: usize) -> bool {
    let Some(v) = d.views.iter().find(|v| v.view == s.view).cloned() else { return false };
    let p = [v.at[0] + s.at[0] * v.scale, v.at[1] + s.at[1] * v.scale];
    let side = (plain_width(&s.letter, 3.5) + 3.0).max(CELL_H);
    // Straight out from the outline the feature is on (any side for a feature inside the view).
    let around = sides(p, v.bounds);
    let on_outline = around.iter().any(|(_, on)| *on);
    let mut spot = None;
    // Where the person put the square (kept on the sheet): the leader leaves
    // the feature square to the way it mostly lies.
    if let Some(off) = s.place {
        let c = [p[0] + off[0], p[1] + off[1]];
        if let Some(shift) = keep_inside([c[0] - side / 2.0, c[1] - CELL_H / 2.0, c[0] + side / 2.0, c[1] + CELL_H / 2.0], grow(g.frame, -1.5)) {
            let c = [c[0] + shift[0], c[1] + shift[1]];
            let (dx, dy) = (c[0] - p[0], c[1] - p[1]);
            let o = if dx.abs() < 1e-9 && dy.abs() < 1e-9 { [0.0, 1.0] } else if dx.abs() > dy.abs() { [dx.signum(), 0.0] } else { [0.0, dy.signum()] };
            let r = [c[0] - side / 2.0, c[1] - CELL_H / 2.0, c[0] + side / 2.0, c[1] + CELL_H / 2.0];
            if !grid.free(grow(r, 0.5)) {
                d.warnings.push(format!("datum {} where it was put covers other lines", s.letter));
            }
            spot = Some((o, c, r));
        }
    }
    'search: for (o, on) in around.into_iter().filter(|_| spot.is_none()) {
        if on_outline && !on {
            continue;
        }
        for len in [4.0, 7.0, 11.0, 16.0, 22.0, 30.0, 40.0] {
            let c = [p[0] + o[0] * (3.0 + len + CELL_H / 2.0), p[1] + o[1] * (3.0 + len + CELL_H / 2.0)];
            let r = [c[0] - side / 2.0, c[1] - CELL_H / 2.0, c[0] + side / 2.0, c[1] + CELL_H / 2.0];
            let near = [c[0] - o[0] * CELL_H / 2.0, c[1] - o[1] * CELL_H / 2.0];
            if within(grow(r, 1.5), g.frame) && grid.free(grow(r, 1.0)) && words.line_free(p, near, 3.0, 0.0) {
                spot = Some((o, c, r));
                break 'search;
            }
        }
    }
    let Some((o, c, r)) = spot else { return false };
    let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
    // A filled triangle on the feature, a line to the lettered square.
    let t = [-o[1], o[0]];
    let apex = [p[0] + o[0] * 3.0, p[1] + o[1] * 3.0];
    d.fills.push(vec![[p[0] + t[0] * 1.75, p[1] + t[1] * 1.75], [p[0] - t[0] * 1.75, p[1] - t[1] * 1.75], apex]);
    let edge = [c[0] - o[0] * if o[0] != 0.0 { side / 2.0 } else { CELL_H / 2.0 }, c[1] - o[1] * CELL_H / 2.0];
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![apex, edge] });
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]], [r[0], r[1]]] });
    d.texts.push(Text { at: [c[0], c[1] - 1.75], size: 3.5, text: s.letter.clone(), anchor: Anchor::Middle, angle: 0.0, bold: false });
    grid.mark_drawn(d, s0..d.strokes.len(), t0..d.texts.len());
    words.mark_drawn(d, s0..s0, t0..d.texts.len());
    let bounds = bounds_of(d, s0..d.strokes.len(), t0..d.texts.len(), f0..d.fills.len());
    d.annotations.push(AnnotationMark { kind: MarkKind::Datum, index, diameter: None, bounds, origin: Some(c), anchor: Some(p) });
    true
}

// ---- Detail views ---------------------------------------------------------------

/// Standard scales a detail may take (ISO 5455), largest first.
const DETAIL_SCALES: [(f64, &str); 18] = [
    (50.0, "50:1"),
    (20.0, "20:1"),
    (10.0, "10:1"),
    (5.0, "5:1"),
    (2.0, "2:1"),
    (1.0, "1:1"),
    (0.5, "1:2"),
    (0.2, "1:5"),
    (0.1, "1:10"),
    (0.05, "1:20"),
    (0.02, "1:50"),
    (0.01, "1:100"),
    (0.005, "1:200"),
    (0.002, "1:500"),
    (0.001, "1:1000"),
    (0.0005, "1:2000"),
    (0.0002, "1:5000"),
    (0.0001, "1:10000"),
];

/// Letters for views (ISO 128-30 leaves out I, O and Q).
fn view_letters(section: bool) -> impl Iterator<Item = char> {
    ('A'..='Z').filter(move |c| !matches!(c, 'I' | 'O' | 'Q') && !(section && *c == 'A'))
}

/// Lays out the person's datums, feature control frames and detail views in
/// the sheet's free space, each where it reads best and over nothing else.
pub(super) fn annotate(d: &mut Drawing, options: &Options, g: &Geometry) {
    let mut grid = Grid::new(d.width, d.height);
    // Lettering only, which leaders must not cross.
    let mut words = Grid::new(d.width, d.height);
    for t in &d.texts {
        words.mark(grow(text_box(t), 0.3));
    }
    let is_frame = |s: &Stroke| s.pen == Pen::Frame && s.points.len() == 5 && (s.points[0][0] - g.frame[0]).abs() < 1e-6 && (s.points[0][1] - g.frame[1]).abs() < 1e-6 && (s.points[2][0] - g.frame[2]).abs() < 1e-6;
    for s in d.strokes.iter().filter(|s| !is_frame(s)) {
        for w in s.points.windows(2) {
            grid.mark_line(w[0], w[1], 0.4);
        }
    }
    for f in &d.fills {
        for p in f {
            grid.mark([p[0] - 0.3, p[1] - 0.3, p[0] + 0.3, p[1] + 0.3]);
        }
    }
    for t in &d.texts {
        grid.mark(grow(text_box(t), 0.3));
    }
    // Ink only, for letters that sit on a view (a detail's circle letter).
    let mut ink = grid.clone();
    for v in &d.views {
        grid.mark(grow(v.bounds, 1.0));
    }
    // The isometric (visible lines drawn after the orthographic views' geometry) as a block.
    let iso = bounds_of(d, 0..0, 0..0, 0..0);
    let mut iso = iso;
    for s in d.strokes[g.strokes.end..].iter().filter(|s| s.pen == Pen::Visible) {
        for p in &s.points {
            iso = union(iso, [p[0], p[1], p[0], p[1]]);
        }
    }
    if iso[0].is_finite() {
        grid.mark(grow(iso, 1.0));
    }
    for r in &g.reserved {
        grid.mark(*r);
    }


    // The detail circles on their views first (they belong to the view).
    let section = options.section.is_some();
    let mut letters = view_letters(section);
    let mut sources = vec![];
    for (index, spec) in options.details.iter().enumerate() {
        let Some(v) = d.views.iter().find(|v| v.view == spec.view).cloned() else {
            d.warnings.push(format!("a detail is on the {:?} view, which this sheet leaves out", spec.view).to_lowercase());
            continue;
        };
        let Some(letter) = letters.next() else { break };
        let c = [v.at[0] + spec.center[0] * v.scale, v.at[1] + spec.center[1] * v.scale];
        let r = spec.radius * v.scale;
        let (s0, t0) = (d.strokes.len(), d.texts.len());
        d.strokes.push(Stroke { pen: Pen::Thin, points: circle(c, r) });
        let mut best = None;
        for reach in [2.6, 5.0] {
            for deg in [45.0f64, 135.0, -45.0, -135.0, 90.0, 0.0, 180.0, -90.0] {
                let (sn, cs) = deg.to_radians().sin_cos();
                let at = [c[0] + (r + reach) * cs + 1.6 * cs, c[1] + (r + reach) * sn + if sn < -0.1 { -3.5 } else if sn.abs() <= 0.1 { -1.75 } else { 0.0 }];
                let t = Text { at, size: 3.5, text: letter.to_string(), anchor: Anchor::Middle, angle: 0.0, bold: true };
                let first = best.is_none();
                if ink.free(grow(text_box(&t), 0.3)) {
                    best = Some(t);
                    break;
                }
                if first {
                    best = Some(t);
                }
            }
            if best.as_ref().is_some_and(|t| ink.free(grow(text_box(t), 0.3))) {
                break;
            }
        }
        d.texts.extend(best);
        grid.mark_drawn(d, s0..d.strokes.len(), t0..d.texts.len());
        ink.mark_drawn(d, s0..d.strokes.len(), t0..d.texts.len());
        words.mark_drawn(d, s0..s0, t0..d.texts.len());
        sources.push(Source { index, spec: spec.clone(), v, letter, c, r });
    }
    for dim in &options.dimensions {
        if let Some(i) = dim.detail {
            let w = format!("a dimension is in detail {}, which is not on this sheet", i + 1);
            if !sources.iter().any(|s| s.index == i) && !d.warnings.contains(&w) {
                d.warnings.push(w);
            }
        }
    }

    // Details the person put somewhere go there before anything finds its own room.
    let (placed, free): (Vec<Source>, Vec<Source>) = sources.into_iter().partition(|s| s.spec.place.is_some());
    for src in &placed {
        if !place_detail(d, &mut grid, g, options, src) {
            d.warnings.push(format!("no room on the sheet for detail {}: try a smaller circle or a larger sheet", src.letter));
        }
    }
    for (index, s) in options.datums.iter().enumerate() {
        if !place_datum(d, &mut grid, &mut words, g, s, index) {
            let view = format!("{:?}", s.view).to_lowercase();
            d.warnings.push(if d.views.iter().any(|v| v.view == s.view) { format!("no room for datum {} beside the {view} view", s.letter) } else { format!("datum {} is on the {view} view, which this sheet leaves out", s.letter) });
        }
    }
    for (index, f) in options.frames.iter().enumerate() {
        if !place_frame(d, &mut grid, &mut words, g, f, index) {
            let symbol = format!("{:?}", f.symbol).to_lowercase();
            let view = format!("{:?}", f.view).to_lowercase();
            d.warnings.push(if d.views.iter().any(|v| v.view == f.view) { format!("no room for the {symbol} frame beside the {view} view") } else { format!("the {symbol} frame is on the {view} view, which this sheet leaves out") });
        }
    }
    for f in &options.finishes {
        if !place_finish(d, &mut grid, &mut words, g, f) {
            let view = format!("{:?}", f.view).to_lowercase();
            d.warnings.push(if d.views.iter().any(|v| v.view == f.view) { format!("no room for the Ra {} symbol beside the {view} view", length_text(f.roughness.ra)) } else { format!("a surface texture symbol is on the {view} view, which this sheet leaves out") });
        }
    }
    for w in &options.welds {
        if !place_weld(d, &mut grid, &mut words, g, w) {
            let view = format!("{:?}", w.view).to_lowercase();
            d.warnings.push(if d.views.iter().any(|v| v.view == w.view) { format!("no room for a weld symbol beside the {view} view") } else { format!("a weld symbol is on the {view} view, which this sheet leaves out") });
        }
    }
    // Then the other details, each at the largest scale that finds room.
    for src in &free {
        if !place_detail(d, &mut grid, g, options, src) {
            d.warnings.push(format!("no room on the sheet for detail {}: try a smaller circle or a larger sheet", src.letter));
        }
    }
}

/// A detail's circle on its view, before the enlarged view is placed.
struct Source {
    index: usize,
    spec: DetailSpec,
    v: ViewFrame,
    letter: char,
    /// The circle on the view (sheet mm).
    c: [f64; 2],
    r: f64,
}

/// The box the detail's own dimensions take at scale `k`, about the enlarged
/// view's centre (sheet mm), when it has any.
fn detail_dims_box(d: &Drawing, options: &Options, index: usize, center: [f64; 2], k: f64) -> Option<[f64; 4]> {
    let mut scratch = Drawing::blank(d.width, d.height);
    let m = |p: [f64; 2]| [(p[0] - center[0]) * k, (p[1] - center[1]) * k];
    for dim in options.dimensions.iter().filter(|x| x.detail == Some(index)) {
        user_dim(&mut scratch, 0, dim, m(dim.a), m(dim.b));
    }
    scratch.marks.iter().map(|m| m.bounds).reduce(union)
}

/// Draws a detail view where the person put it or in the free space nearest
/// its circle, at the largest scale that finds room; with its dimensions.
fn place_detail(d: &mut Drawing, grid: &mut Grid, g: &Geometry, options: &Options, src: &Source) -> bool {
    let Source { index, spec, v, letter, c, r } = src;
    let (index, c, r) = (*index, *c, *r);
    let max_d = d.height * 0.3;
    let s = v.scale;
    let snapped = spec.scale.map(|k| DETAIL_SCALES.iter().copied().min_by(|a, b| (a.0.ln() - k.ln()).abs().total_cmp(&(b.0.ln() - k.ln()).abs())).unwrap());
    // Larger than the view and no larger than reads well on this sheet,
    // from the person's scale down, or from 10× the view down.
    let larger = DETAIL_SCALES.iter().copied().filter(|(k, _)| *k > s * 1.01);
    let from = snapped.map_or(s * 10.0, |w| w.0);
    let mut scales: Vec<(f64, &str)> = larger.clone().filter(|(k, _)| *k <= from + 1e-12 && 2.0 * spec.radius * k <= max_d).collect();
    if scales.is_empty() {
        // A big circle: the least enlargement there is, if the sheet can hold it at all.
        scales.extend(larger.filter(|(k, _)| 2.0 * spec.radius * k + 12.0 <= (g.frame[3] - g.frame[1]).min(g.frame[2] - g.frame[0])).last());
    }
    let inner = grow(g.frame, -2.0);
    for (k, k_text) in scales {
        let big = spec.radius * k;
        let label = format!("DETAIL {letter} ({k_text})");
        let lw = plain_width(&label, 3.5) * 1.05;
        let half = big.max(lw / 2.0) + 1.5;
        let dims = detail_dims_box(d, options, index, spec.center, k);
        // The box: the circle, its label under it, and its dimensions.
        let area = |x: f64, y: f64| {
            let r = [x - half, y - big - 10.0, x + half, y + big + 1.5];
            match dims {
                Some(b) => union(r, grow([x + b[0], y + b[1], x + b[2], y + b[3]], 1.0)),
                None => r,
            }
        };
        let best: Option<[f64; 2]> = match spec.place {
            // Where the person put it, moved onto the sheet if it went off.
            Some(p) => keep_inside(area(p[0], p[1]), inner).map(|m| [p[0] + m[0], p[1] + m[1]]),
            None => {
                let mut best: Option<([f64; 2], f64)> = None;
                let mut y = g.frame[1] + big + 10.0;
                while y <= g.frame[3] - big {
                    let mut x = g.frame[0] + half;
                    while x <= g.frame[2] - half {
                        let r = area(x, y);
                        if within(r, inner) && grid.free(r) {
                            let dist = (x - c[0]).hypot(y - c[1]);
                            if best.is_none_or(|(_, b)| dist < b) {
                                best = Some(([x, y], dist));
                            }
                        }
                        x += 2.0;
                    }
                    y += 2.0;
                }
                best.map(|b| b.0)
            }
        };
        let Some(dc) = best else { continue };
        if spec.place.is_some() && !grid.free(area(dc[0], dc[1])) {
            d.warnings.push(format!("detail {letter} where it was put covers other lines"));
        }
        let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
        let m = k / s;
        let map = |p: [f64; 2]| [dc[0] + (p[0] - c[0]) * m, dc[1] + (p[1] - c[1]) * m];
        // The view's lines and centre lines inside the circle, enlarged.
        let vb = grow(v.bounds, 2.6);
        let inside = |p: &[f64; 2]| p[0] >= vb[0] && p[0] <= vb[2] && p[1] >= vb[1] && p[1] <= vb[3];
        let mut lines = vec![];
        for st in &d.strokes[g.strokes.clone()] {
            if !matches!(st.pen, Pen::Visible | Pen::Hidden | Pen::Center) || !st.points.iter().all(inside) {
                continue;
            }
            for part in clip_to_circle(&st.points, c, r) {
                lines.push(Stroke { pen: st.pen, points: part.into_iter().map(map).collect() });
            }
        }
        d.strokes.extend(lines);
        // The section's hatching, at the sheet's spacing, inside the circle.
        if v.view == ViewName::Front && !g.section.is_empty() {
            let segs: Vec<[[f64; 2]; 2]> = g.section.iter().map(|[p, q]| [map(*p), map(*q)]).collect();
            for [a, b] in hatch_lines(&segs, 2.5) {
                for part in clip_to_circle(&[a, b], dc, big) {
                    d.strokes.push(Stroke { pen: Pen::Thin, points: part });
                }
            }
        }
        d.strokes.push(Stroke { pen: Pen::Thin, points: circle(dc, big) });
        d.texts.push(Text { at: [dc[0], dc[1] - big - 7.0], size: 3.5, text: label, anchor: Anchor::Middle, angle: 0.0, bold: true });
        let bounds = bounds_of(d, s0..d.strokes.len(), t0..d.texts.len(), f0..d.fills.len());
        d.annotations.push(AnnotationMark { kind: MarkKind::Detail, index, diameter: None, bounds, origin: Some(dc), anchor: Some(c) });
        // Its own dimensions, true sizes at the detail's scale.
        let at = [dc[0] - spec.center[0] * k, dc[1] - spec.center[1] * k];
        for (i, dim) in options.dimensions.iter().enumerate().filter(|(_, x)| x.detail == Some(index)) {
            let on = |p: [f64; 2]| [at[0] + p[0] * k, at[1] + p[1] * k];
            user_dim(d, i, dim, on(dim.a), on(dim.b));
        }
        grid.mark(grow(area(dc[0], dc[1]), 0.5));
        d.details.push(DetailFrame { index, letter: letter.to_string(), view: v.view, at, scale: k, axes: v.axes, centre: dc, radius: big });
        return true;
    }
    false
}

// ---- Surface texture and weld symbols ---------------------------------------------

/// A leader from a point on a view out to a horizontal shoulder where a
/// symbol `w` × `h` sits (its lower left at the shoulder's start): the spot
/// clear of everything with the shortest leader, out from the side of the
/// view the point is on. Returns the shoulder's start and which way it runs.
fn leader_spot(grid: &mut Grid, words: &mut Grid, g: &Geometry, p: [f64; 2], bounds: [f64; 4], w: f64, h: f64) -> Option<([f64; 2], bool)> {
    let around = sides(p, bounds);
    let on_outline = around.iter().any(|(_, on)| *on);
    let mut best: Option<(f64, [f64; 2], bool)> = None;
    for (rank, (o, on)) in around.into_iter().enumerate() {
        let penalty = if on_outline && !on { 60.0 } else { rank as f64 * 6.0 };
        let base = o[1].atan2(o[0]).to_degrees();
        for turn in [0.0f64, 30.0, -30.0, 50.0, -50.0] {
            let a = (base + turn).to_radians();
            for reach in [6.0, 10.0, 15.0, 21.0, 28.0] {
                let knee = [p[0] + reach * a.cos(), p[1] + reach * a.sin()];
                let right = a.cos() >= -1e-9;
                let r = if right { [knee[0], knee[1], knee[0] + w, knee[1] + h] } else { [knee[0] - w, knee[1], knee[0], knee[1] + h] };
                if !within(grow(r, 1.0), g.frame) || !grid.free(grow(r, 0.8)) || !words.line_free(p, knee, 1.0, 0.5) {
                    continue;
                }
                let score = reach + penalty + turn.abs() / 10.0 + 4.0 * grid.hits(p, knee, 1.5, 0.8) as f64;
                if best.is_none_or(|b| score < b.0) {
                    best = Some((score, knee, right));
                }
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// A surface texture symbol on a leader (ISO 1302): the arrow on the surface,
/// the symbol on the leader's shoulder with its requirement over it.
fn place_finish(d: &mut Drawing, grid: &mut Grid, words: &mut Grid, g: &Geometry, f: &FinishSpec) -> bool {
    let Some(v) = d.views.iter().find(|v| v.view == f.view).cloned() else { return false };
    let p = [v.at[0] + f.at[0] * v.scale, v.at[1] + f.at[1] * v.scale];
    let text = super::sheet::roughness_text(&f.roughness);
    let tw = plain_width(&text, 3.5);
    let pw = f.roughness.process.as_ref().map_or(0.0, |s| plain_width(s, 2.5));
    let t60 = 60f64.to_radians().tan();
    let (h1, h2) = (5.0, 10.5);
    let w = h1 / t60 + h2 / t60 + tw.max(pw) + 3.0;
    let Some((knee, right)) = leader_spot(grid, words, g, p, v.bounds, w + 2.0, h2 + 4.0) else { return false };
    let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
    let start = if right { knee[0] } else { knee[0] - w - 2.0 };
    let apex = [start + 2.0 + h1 / t60, knee[1]];
    let l = (knee[0] - p[0]).hypot(knee[1] - p[1]).max(1e-9);
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![p, knee] });
    super::arrow(d, p, [(p[0] - knee[0]) / l, (p[1] - knee[1]) / l]);
    // The shoulder runs from the leader to the symbol's point.
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![knee, apex] });
    let top = super::sheet::symbol_shape(d, apex, 3.5, !f.roughness.no_removal, Some(tw.max(pw) + 1.5));
    if f.roughness.no_removal {
        // No material removed: a circle in the V.
        d.strokes.push(Stroke { pen: Pen::Thin, points: circle([apex[0], apex[1] + h1 * 0.55], h1 * 0.33) });
    }
    let x = apex[0] + h1 / t60 + 0.8;
    d.texts.push(Text { at: [x, top[1] - 4.6], size: 3.5, text, anchor: Anchor::Start, angle: 0.0, bold: false });
    if let Some(proc) = &f.roughness.process {
        d.texts.push(Text { at: [x, top[1] + 0.8], size: 2.5, text: proc.clone(), anchor: Anchor::Start, angle: 0.0, bold: false });
    }
    if let Some(lay) = &f.roughness.lay {
        d.texts.push(Text { at: [apex[0] + h2 / t60 * 0.55 + 1.2, apex[1] + 0.4], size: 2.5, text: lay.clone(), anchor: Anchor::Start, angle: 0.0, bold: false });
    }
    grid.mark_drawn(d, s0..d.strokes.len(), t0..d.texts.len());
    words.mark_drawn(d, s0..s0, t0..d.texts.len());
    let bounds = bounds_of(d, s0..d.strokes.len(), t0..d.texts.len(), f0..d.fills.len());
    d.annotations.push(AnnotationMark { kind: MarkKind::Finish, index: 0, diameter: None, bounds, origin: Some(knee), anchor: Some(p) });
    true
}

/// A weld symbol (ISO 2553, system A): the arrow on the joint, a reference
/// line with its dashed identification line under it, the weld's symbol on
/// the arrow side (over the line) or the other side (under the dashed line),
/// its size before and length after; a circle for all round, a flag for a
/// site weld.
fn place_weld(d: &mut Drawing, grid: &mut Grid, words: &mut Grid, g: &Geometry, w: &WeldSpec) -> bool {
    let Some(v) = d.views.iter().find(|v| v.view == w.view).cloned() else { return false };
    let p = [v.at[0] + w.at[0] * v.scale, v.at[1] + w.at[1] * v.scale];
    let prefix = match w.kind {
        WeldKind::Fillet => "a",
        WeldKind::Spot | WeldKind::Plug => "d",
        _ => "s",
    };
    let size_text = w.size.map(|s| format!("{prefix}{}", length_text(s)));
    let len_text = w.length.map(length_text);
    let sw = size_text.as_ref().map_or(0.0, |t| plain_width(t, 3.5) + 1.5);
    let lw = len_text.as_ref().map_or(0.0, |t| plain_width(t, 3.5) + 1.5);
    let sym = 5.0;
    let line_w = 6.0 + sw + sym + lw + 6.0;
    // Room under and over the line for symbols on either side, and the flag.
    let (below, above) = (6.5, if w.field { 8.0 } else { 6.0 });
    let Some((spot, right)) = leader_spot(grid, words, g, p, v.bounds, line_w, below + above) else { return false };
    let knee = [spot[0], spot[1] + below];
    let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
    let l = (knee[0] - p[0]).hypot(knee[1] - p[1]).max(1e-9);
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![p, knee] });
    super::arrow(d, p, [(p[0] - knee[0]) / l, (p[1] - knee[1]) / l]);
    let (x0, x1) = if right { (knee[0], knee[0] + line_w) } else { (knee[0] - line_w, knee[0]) };
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x0, knee[1]], [x1, knee[1]]] });
    let dash = knee[1] - 1.2;
    if !w.both_sides {
        d.strokes.push(Stroke { pen: Pen::Hidden, points: vec![[x0, dash], [x1, dash]] });
    }
    // The symbol's slot along the line.
    let sx = x0 + 6.0 + sw;
    let draw = |d: &mut Drawing, base: f64, up: f64| {
        let q = |x: f64, y: f64| [sx + x, base + up * y];
        let lines: Vec<Vec<[f64; 2]>> = match w.kind {
            WeldKind::Fillet => vec![vec![q(0.6, 0.0), q(0.6, 4.0), q(4.6, 0.0)]],
            WeldKind::SquareButt => vec![vec![q(1.6, 0.0), q(1.6, 3.6)], vec![q(3.4, 0.0), q(3.4, 3.6)]],
            WeldKind::VButt => vec![vec![q(0.4, 3.6), q(2.5, 0.0), q(4.6, 3.6)]],
            WeldKind::BevelButt => vec![vec![q(1.0, 3.6), q(1.0, 0.0), q(4.4, 3.6)]],
            WeldKind::Plug => vec![vec![q(0.6, 0.0), q(0.6, 2.6), q(4.4, 2.6), q(4.4, 0.0)]],
            WeldKind::Spot => vec![],
        };
        for points in lines {
            d.strokes.push(Stroke { pen: Pen::Visible, points });
        }
    };
    if w.kind == WeldKind::Spot {
        // A spot weld's circle sits on the line, whichever side.
        d.strokes.push(Stroke { pen: Pen::Visible, points: circle([sx + 2.5, knee[1]], 2.0) });
    } else if w.both_sides {
        draw(d, knee[1], 1.0);
        draw(d, knee[1], -1.0);
    } else if w.other_side {
        draw(d, dash, -1.0);
    } else {
        draw(d, knee[1], 1.0);
    }
    let ty = if w.other_side && !w.both_sides { dash - 4.6 } else { knee[1] + 1.0 };
    if let Some(t) = size_text {
        d.texts.push(Text { at: [sx - 1.0, ty], size: 3.5, text: t, anchor: Anchor::End, angle: 0.0, bold: false });
    }
    if let Some(t) = len_text {
        d.texts.push(Text { at: [sx + sym + 1.0, ty], size: 3.5, text: t, anchor: Anchor::Start, angle: 0.0, bold: false });
    }
    if w.all_around {
        d.strokes.push(Stroke { pen: Pen::Thin, points: circle(knee, 1.8) });
    }
    if w.field {
        // A flag on a staff from the junction.
        let top = [knee[0], knee[1] + 7.0];
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![knee, top] });
        let dir = if right { 1.0 } else { -1.0 };
        d.fills.push(vec![top, [top[0] + dir * 3.5, top[1] - 1.2], [top[0], top[1] - 2.4]]);
    }
    grid.mark_drawn(d, s0..d.strokes.len(), t0..d.texts.len());
    words.mark_drawn(d, s0..s0, t0..d.texts.len());
    let bounds = bounds_of(d, s0..d.strokes.len(), t0..d.texts.len(), f0..d.fills.len());
    d.annotations.push(AnnotationMark { kind: MarkKind::Weld, index: 0, diameter: None, bounds, origin: Some(knee), anchor: Some(p) });
    true
}
