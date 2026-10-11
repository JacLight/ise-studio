//! Engineering drawings: a sheet with the part's orthographic views (hidden
//! lines dashed), an isometric view, its overall dimensions and a title
//! block, laid out at a standard scale. The views come from the exact solid
//! (OpenCascade's hidden-line removal), so they are what the STEP file is.
//! The drawing is plain lines and text in sheet millimetres; it is written
//! as a vector PDF or a DXF here, and drawn as SVG by the app.

use crate::ShapeProgram;
use de_core::schemars::{self, JsonSchema};
use de_occt::{Arena, Bool, Shape, ViewLine};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

mod annotate;
pub mod fits;
pub use annotate::{rename_datum, validate, AnnotationMark, DatumSpec, DetailFrame, DetailSpec, FrameSpec, GdtSymbol, HoleFit, MarkKind, Modifier, Tolerance};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
pub enum Sheet {
    A4,
    #[default]
    A3,
    A2,
    A1,
    A0,
}

impl Sheet {
    /// Landscape width and height (mm, ISO 216).
    pub fn size(self) -> [f64; 2] {
        match self {
            Sheet::A4 => [297.0, 210.0],
            Sheet::A3 => [420.0, 297.0],
            Sheet::A2 => [594.0, 420.0],
            Sheet::A1 => [841.0, 594.0],
            Sheet::A0 => [1189.0, 841.0],
        }
    }
}

/// Where the views go around the front view (ISO 128-30).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    /// European: the top view below the front, the right view on its left.
    #[default]
    First,
    /// American: the top view above the front, the right view on its right.
    Third,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct Options {
    #[serde(default)]
    pub sheet: Sheet,
    #[serde(default)]
    pub projection: Projection,
    /// Only this part of an assembly (all of it when absent).
    #[serde(default)]
    pub part: Option<String>,
    /// Show the front view cut through (section A–A), this far back through
    /// the part (percent of its depth).
    #[serde(default)]
    pub section: Option<f64>,
    /// Dimensions the person added.
    #[serde(default)]
    pub dimensions: Vec<DimSpec>,
    /// Detail views: circles of a view drawn again larger.
    #[serde(default)]
    pub details: Vec<DetailSpec>,
    /// Geometrical tolerances (feature control frames).
    #[serde(default)]
    pub frames: Vec<FrameSpec>,
    /// Datum feature symbols.
    #[serde(default)]
    pub datums: Vec<DatumSpec>,
    /// Fits on holes, by diameter.
    #[serde(default)]
    pub hole_fits: Vec<HoleFit>,
    /// Which views the sheet shows (all four when empty).
    #[serde(default)]
    pub views: Vec<ViewKind>,
    /// The views' scale (snapped to a standard one); the largest that fits when absent.
    #[serde(default)]
    pub scale: Option<f64>,
}

/// A view a sheet can show: the three orthographic views and the isometric.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Front,
    Top,
    Right,
    Iso,
}

/// One of the three orthographic views.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewName {
    #[default]
    Front,
    Top,
    Right,
}

impl ViewName {
    /// The model axes across and up the view (0 x, 1 y, 2 z).
    pub fn axes(self) -> [usize; 2] {
        match self {
            ViewName::Front => [0, 2],
            ViewName::Top => [0, 1],
            ViewName::Right => [1, 2],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DimKind {
    /// Across the view.
    #[default]
    Horizontal,
    /// Up the view.
    Vertical,
    /// Straight from one point to the other.
    Aligned,
}

/// A dimension between two points of a view, kept in model millimetres so
/// it stays on the part when the sheet or scale changes.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct DimSpec {
    pub view: ViewName,
    /// The two points, as model coordinates along the view's axes.
    pub a: [f64; 2],
    pub b: [f64; 2],
    #[serde(default)]
    pub kind: DimKind,
    /// How far the dimension line stands off the points (sheet mm; negative: the other side).
    #[serde(default = "default_offset")]
    pub offset: f64,
    /// Its tolerance: ±, limit deviations, or an ISO 286 fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<Tolerance>,
    /// A diameter: the value is written ⌀25.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diameter: bool,
    /// Drawn in this detail view (its index in the details) instead of the
    /// view itself: the points are still model mm along the view's axes, so
    /// the value is the true size, whatever the detail's scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<usize>,
}

fn default_offset() -> f64 {
    10.0
}

/// One row of the parts list.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PartRow {
    pub name: String,
    pub quantity: usize,
    pub material: String,
    pub mass: String,
}

/// Where a view sits on the sheet: a model point p shows at
/// `at + [p[axes[0]], p[axes[1]]] · scale`.
#[derive(Clone, Debug, Serialize)]
pub struct ViewFrame {
    pub view: ViewName,
    pub at: [f64; 2],
    pub scale: f64,
    pub axes: [usize; 2],
    /// The view's outline on the sheet: [x0, y0, x1, y1].
    pub bounds: [f64; 4],
}

/// What the title block says.
#[derive(Clone, Debug, Default)]
pub struct Title {
    pub title: String,
    pub number: String,
    pub material: String,
    pub mass: String,
    pub drawn_by: String,
    pub date: String,
    /// The parts list (shown when there are two or more parts).
    pub parts: Vec<PartRow>,
    /// This sheet's number and how many sheets the drawing has (1 of 1 when 0).
    pub page: usize,
    pub pages: usize,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Pen {
    /// Visible edges: wide (0.5 mm).
    Visible,
    /// Hidden edges: dashed, narrow.
    Hidden,
    /// Dimension and extension lines, cells of the title block: narrow (0.25 mm).
    Thin,
    /// Centre lines: narrow long-dash-dot (chain) lines.
    Center,
    /// The sheet frame: widest (0.7 mm).
    Frame,
}

impl Pen {
    pub fn width(self) -> f64 {
        match self {
            Pen::Visible => 0.5,
            Pen::Hidden => 0.25,
            Pen::Thin | Pen::Center => 0.25,
            Pen::Frame => 0.7,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Stroke {
    pub pen: Pen,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug, Serialize)]
pub struct Text {
    /// The baseline point the anchor refers to (sheet mm, y up).
    pub at: [f64; 2],
    /// Cap height (mm).
    pub size: f64,
    pub text: String,
    pub anchor: Anchor,
    /// Degrees anticlockwise.
    pub angle: f64,
    pub bold: bool,
}

/// A drawing in sheet millimetres, origin at the bottom left.
#[derive(Clone, Debug, Serialize)]
pub struct Drawing {
    pub width: f64,
    pub height: f64,
    /// Drawing size over real size (0.5 for 1:2).
    pub scale: f64,
    pub scale_text: String,
    pub strokes: Vec<Stroke>,
    /// Filled shapes (arrowheads).
    pub fills: Vec<Vec<[f64; 2]>>,
    pub texts: Vec<Text>,
    /// Where the orthographic views are, for picking points on them.
    pub views: Vec<ViewFrame>,
    /// Where each of the person's dimensions landed: its index in
    /// `Options::dimensions` and its box on the sheet.
    pub marks: Vec<DimMark>,
    /// Where the isometric view is, when there is room for one.
    pub iso: Option<IsoFrame>,
    /// The item balloons on the isometric view, one per row of the parts list.
    pub balloons: Vec<Balloon>,
    /// Where the details, frames, datums and hole callouts landed.
    pub annotations: Vec<AnnotationMark>,
    /// Where the detail views landed, for picking points on them.
    pub details: Vec<DetailFrame>,
    /// This sheet's number and how many there are.
    pub page: usize,
    pub pages: usize,
    /// What could not go on the sheet.
    pub warnings: Vec<String>,
}

impl Drawing {
    /// An empty sheet.
    pub fn blank(width: f64, height: f64) -> Drawing {
        Drawing { width, height, scale: 1.0, scale_text: String::new(), strokes: vec![], fills: vec![], texts: vec![], views: vec![], marks: vec![], iso: None, balloons: vec![], annotations: vec![], details: vec![], page: 1, pages: 1, warnings: vec![] }
    }
}

/// Where the isometric view sits: a model point p shows at
/// `at + iso_point(p) · scale`.
#[derive(Clone, Debug, Serialize)]
pub struct IsoFrame {
    pub at: [f64; 2],
    pub scale: f64,
    /// Its outline on the sheet: [x0, y0, x1, y1].
    pub bounds: [f64; 4],
}

/// A numbered balloon (ISO 6433): its parts-list item, the circle, and the
/// point on the part its leader ends on (sheet mm).
#[derive(Clone, Debug, Serialize)]
pub struct Balloon {
    pub item: usize,
    pub centre: [f64; 2],
    pub radius: f64,
    pub anchor: [f64; 2],
}

#[derive(Clone, Debug, Serialize)]
pub struct DimMark {
    pub dimension: usize,
    pub bounds: [f64; 4],
}

/// Standard scales (ISO 5455), largest first.
const SCALES: [(f64, &str); 16] = [
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

struct View {
    lines: Vec<ViewLine>,
    min: [f64; 2],
    max: [f64; 2],
}

impl View {
    fn of(lines: Vec<ViewLine>) -> View {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in lines.iter().flat_map(|l| &l.points) {
            for k in 0..2 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        if !min[0].is_finite() {
            (min, max) = ([0.0; 2], [0.0; 2]);
        }
        View { lines, min, max }
    }
    fn size(&self) -> [f64; 2] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1]]
    }
}

/// Lengths as a drawing shows them: whole millimetres, or to 0.1 / 0.01 when they need it.
pub fn length_text(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    let s = if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else if (r * 10.0 - (r * 10.0).round()).abs() < 1e-9 { format!("{r:.1}") } else { format!("{r:.2}") };
    s
}

/// The drawing of a program (its exact solid), on a sheet.
pub fn drawing(program: &ShapeProgram, options: &Options, title: &Title) -> Result<Drawing, String> {
    let mut all = drawings(program, &[(options.clone(), title.clone())])?;
    Ok(all.remove(0))
}

/// The sheets of a drawing, each with its own options and title block, from
/// one evaluation of the program. A feature control frame may refer to a
/// datum marked on any of the sheets of the same part.
pub fn drawings(program: &ShapeProgram, sheets: &[(Options, Title)]) -> Result<Vec<Drawing>, String> {
    let values = program.values();
    let tolerance = program.display_tolerance();
    let (mut arena, exact) = crate::exact::evaluate_exact_in(Arena::new(), &program.name, &program.source, &values, tolerance, &program.inputs).map_err(|(_, e)| e)?;
    let mut out = vec![];
    for (options, title) in sheets {
        let letters: Vec<&str> = sheets.iter().filter(|(o, _)| o.part == options.part).flat_map(|(o, _)| o.datums.iter().map(|d| d.letter.as_str())).collect();
        let parts: Vec<(String, Shape)> = exact.parts.iter().filter(|p| options.part.as_ref().is_none_or(|n| &p.name == n)).map(|p| (p.name.clone(), p.shape)).collect();
        if parts.is_empty() {
            return Err(match &options.part {
                Some(n) => format!("no part called “{n}”"),
                None => "the program made nothing to draw".into(),
            });
        }
        let shape = if parts.len() == 1 { parts[0].1 } else { arena.compound(&parts.iter().map(|p| p.1).collect::<Vec<_>>())? };
        let mut d = layout(&mut arena, shape, &parts, options, title)?;
        for f in &options.frames {
            for r in f.datums.iter().flat_map(|r| r.split('-')) {
                if !letters.contains(&r) {
                    let w = format!("the {} frame refers to datum {r}, which no datum symbol marks", format!("{:?}", f.symbol).to_lowercase());
                    if !d.warnings.contains(&w) {
                        d.warnings.push(w);
                    }
                }
            }
        }
        out.push(d);
    }
    Ok(out)
}

/// The isometric view's direction and up (the view looks down at the front right).
fn iso_frame() -> ([f64; 3], [f64; 3]) {
    let k = 1.0 / 3f64.sqrt();
    ([-k, k, -k], [0.0, 0.0, 1.0])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Where a model point lands in a hidden-line view looking along `dir` with
/// `up` up the page, in the frame [`Arena::hlr_view`] draws in: x across and
/// y up the view (mm), and how near the eye it is (larger is nearer).
pub fn view_point(dir: [f64; 3], up: [f64; 3], p: [f64; 3]) -> [f64; 3] {
    let toward_eye = unit([-dir[0], -dir[1], -dir[2]]);
    let right = unit(cross(up, toward_eye));
    let upward = cross(toward_eye, right);
    let dot = |a: [f64; 3]| a[0] * p[0] + a[1] * p[1] + a[2] * p[2];
    [dot(right), dot(upward), dot(toward_eye)]
}

/// Where the isometric view shows a model point (view mm, as [`view_point`]).
pub fn iso_point(p: [f64; 3]) -> [f64; 2] {
    let (dir, up) = iso_frame();
    let v = view_point(dir, up, p);
    [v[0], v[1]]
}

/// Lays out the views of `shape` on the sheet. `parts` are its parts by
/// name, for the balloons that key the isometric to the parts list.
pub fn layout(arena: &mut Arena, shape: Shape, parts: &[(String, Shape)], options: &Options, title: &Title) -> Result<Drawing, String> {
    let b = arena.bounds(shape)?;
    let size3 = [b[3] - b[0], b[4] - b[1], b[5] - b[2]];
    let deflection = (size3.iter().cloned().fold(0.0, f64::max) / 2000.0).clamp(0.005, 1.0);
    let shown = |k: ViewKind| options.views.is_empty() || options.views.contains(&k);
    let (has_f, has_t, has_r, has_i) = (shown(ViewKind::Front), shown(ViewKind::Top), shown(ViewKind::Right), shown(ViewKind::Iso));
    let mut warnings = vec![];
    // A view the sheet leaves out is an empty one: no size, nothing drawn.
    let view = |arena: &mut Arena, dir: [f64; 3], up: [f64; 3], on: bool| if on { arena.hlr_view(shape, dir, up, deflection).map(View::of) } else { Ok(View::of(vec![])) };
    // Section A–A: the front view of what lies behind the cutting plane, no hidden lines.
    let cut = options.section.filter(|_| has_f).map(|p| b[1] + size3[1] * (p / 100.0).clamp(0.01, 0.99));
    if options.section.is_some() && !has_f {
        warnings.push("section A–A is the front view, which this sheet leaves out".to_string());
    }
    let front = match cut {
        None => view(arena, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], has_f)?,
        Some(y) => {
            let m = size3.iter().cloned().fold(0.0, f64::max) + 1.0;
            let keep = arena.cube(size3[0] + 2.0 * m, b[4] - y + m, size3[2] + 2.0 * m, false)?;
            let keep = arena.translate(keep, [b[0] - m, y, b[2] - m])?;
            let behind = arena.boolean(Bool::Intersection, shape, keep)?;
            let mut v = View::of(arena.hlr_view(behind, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], deflection)?);
            v.lines.retain(|l| !l.hidden);
            v
        }
    };
    let top = view(arena, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], has_t)?;
    let right = view(arena, [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], has_r)?;
    let (iso_dir, iso_up) = iso_frame();
    let iso = view(arena, iso_dir, iso_up, has_i)?;
    let has_ortho = has_f || has_t || has_r;

    let [w, h] = options.sheet.size();
    let mut d = Drawing::blank(w, h);
    d.page = title.page.max(1);
    d.pages = title.pages.max(d.page);
    d.warnings = warnings;
    // Frame: 20 mm binding margin on the left, 10 mm elsewhere (ISO 5457).
    let (fx0, fy0, fx1, fy1) = (20.0, 10.0, w - 10.0, h - 10.0);
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[fx0, fy0], [fx1, fy0], [fx1, fy1], [fx0, fy1], [fx0, fy0]] });
    let block = [180.0f64.min(fx1 - fx0), 32.0];
    // The parts list sits on the title block, one row per part plus a header.
    let list_h = if title.parts.len() >= 2 { 6.0 * (title.parts.len() + 1) as f64 } else { 0.0 };
    // With a parts list, the isometric keeps a margin round it for its balloons.
    let ballooned = list_h > 0.0 && parts.len() >= 2;
    let room = if ballooned { BALLOON_GAP + BALLOON_R + 2.0 } else { 0.0 };

    // The views in a grid: front in the middle, the others where the projection puts them.
    let gap = 22.0; // sheet mm between views: room for a dimension
    let (fw, fh) = (front.size()[0], front.size()[1]);
    let (tw, th) = (top.size()[0], top.size()[1]);
    let (rw, rh) = (right.size()[0], right.size()[1]);
    let (iw, ih) = (iso.size()[0], iso.size()[1]);
    // Gaps between the columns and rows of views only where both sides have one.
    let gx = if has_r && (has_f || has_t) { gap } else { 0.0 };
    let gy = if has_t && (has_f || has_r) { gap } else { 0.0 };
    // The area above the title block, inside the frame, less a margin for dimensions.
    let area = [fx1 - fx0 - 2.0 * gap, fy1 - fy0 - block[1] - list_h - 2.0 * gap];
    // The main scale fits the three orthographic views; the isometric takes
    // whatever scale fits the room left beside them (and says so). A sheet
    // of the isometric alone draws it at the main scale.
    let reserve = if has_i { 40.0 } else { 0.0 };
    let fits = |s: f64| {
        if has_ortho {
            (rw + fw.max(tw)) * s + gx <= area[0] - reserve && (fh.max(rh) + th) * s + gy <= area[1]
        } else {
            iw * s <= area[0] + gap - 2.0 * room && ih * s <= area[1] + gap - 8.0 - 2.0 * room
        }
    };
    let auto = SCALES.iter().find(|(s, _)| fits(*s)).copied().unwrap_or(SCALES[SCALES.len() - 1]);
    let (scale, scale_text) = match options.scale {
        None => auto,
        Some(want) => {
            let near = SCALES.iter().copied().min_by(|a, b| (a.0.ln() - want.ln()).abs().total_cmp(&(b.0.ln() - want.ln()).abs())).unwrap_or(auto);
            if fits(near.0) {
                near
            } else {
                d.warnings.push(format!("the views do not fit the sheet at {}: drawn at {}", near.1, auto.1));
                auto
            }
        }
    };
    d.scale = scale;
    d.scale_text = scale_text.to_string();
    let s = scale;
    // Columns: right view then front/top (first angle), or front/top then right view (third).
    let row_front = fh.max(rh) * s;
    let mid_w = fw.max(tw) * s;
    let main_w = rw * s + mid_w + gx;
    let total_h = row_front + th * s + gy;
    // The isometric goes where it comes out largest: the empty corner of the
    // grid (beside the top view, under or over the side view), or a column
    // right of the views. Its scale is the largest standard one that fits.
    let iso_fit = |w: f64, h: f64| SCALES.iter().find(|(k, _)| *k <= s && iw * k <= w - 2.0 * room && ih * k <= h - 8.0 - 2.0 * room).copied();
    let corner = if has_i && has_ortho { iso_fit(rw * s, th * s) } else { None };
    let column = if !has_i {
        None
    } else if has_ortho {
        iso_fit(area[0] - main_w - gap, area[1])
    } else {
        iso_fit(area[0] + gap, area[1] + gap)
    };
    let in_corner = match (corner, column) {
        (Some(a), Some(b)) => a.0 >= b.0,
        (a, _) => a.is_some(),
    };
    // Centred across the sheet when nothing stands beside them.
    let x0 = if in_corner || !has_i { fx0 + gap + (area[0] - main_w).max(0.0) / 2.0 } else { fx0 + gap };
    let y0 = fy0 + block[1] + list_h + gap + (area[1] - total_h).max(0.0) / 2.0;
    let (col_side, col_main) = match options.projection {
        Projection::First => (x0, x0 + rw * s + gx),
        Projection::Third => (x0 + mid_w + gx, x0),
    };
    let (row_top, row_main) = match options.projection {
        Projection::First => (y0, y0 + th * s + gy),
        Projection::Third => (y0 + row_front + gy, y0),
    };
    let column_x = if has_ortho { x0 + main_w + gap } else { fx0 + gap };
    let (iso_cell, iso_scale) = if in_corner { ([col_side, row_top, col_side + rw * s, row_top + th * s], corner) } else { ([column_x, fy0 + block[1] + list_h + gap, fx1 - gap, fy1 - gap], column) };
    // Where each view's lower left lands (sheet mm), the views centred in their cells.
    let place = |v: &View, cell: [f64; 2], cw: f64, ch: f64, k: f64| [cell[0] + (cw - v.size()[0] * k) / 2.0 - v.min[0] * k, cell[1] + (ch - v.size()[1] * k) / 2.0 - v.min[1] * k];
    let at_front = place(&front, [col_main, row_main], mid_w, row_front, s);
    let at_top = place(&top, [col_main, row_top], mid_w, th * s, s);
    let at_right = place(&right, [col_side, row_main], rw * s, row_front, s);
    let geometry_start = d.strokes.len();
    let mut section_segments = vec![];
    let ortho = [(ViewName::Front, &front, at_front, has_f), (ViewName::Top, &top, at_top, has_t), (ViewName::Right, &right, at_right, has_r)];
    for (_, v, at, _) in ortho.iter().filter(|o| o.3) {
        for l in &v.lines {
            d.strokes.push(Stroke { pen: if l.hidden { Pen::Hidden } else { Pen::Visible }, points: l.points.iter().map(|p| [at[0] + p[0] * s, at[1] + p[1] * s]).collect() });
        }
    }
    for (name, v, at, _) in ortho.iter().filter(|o| o.3) {
        let (name, at) = (*name, *at);
        d.views.push(ViewFrame { view: name, at, scale: s, axes: name.axes(), bounds: [at[0] + v.min[0] * s, at[1] + v.min[1] * s, at[0] + v.max[0] * s, at[1] + v.max[1] * s] });
    }
    if let Some(y) = cut {
        // Hatch the cut faces, and mark the cutting plane on the top view.
        let mesh = arena.mesh(shape, deflection)?;
        let segments: Vec<[[f64; 2]; 2]> = slice(&mesh.positions, &mesh.triangles, y + size3[1] * 1e-7).into_iter().map(|[p, q]| [[at_front[0] + p[0] * s, at_front[1] + p[1] * s], [at_front[0] + q[0] * s, at_front[1] + q[1] * s]]).collect();
        hatch(&mut d, &segments, 2.5);
        section_segments = segments;
        let line_y = at_top[1] + y * s;
        let (xa, xb) = (at_top[0] + b[0] * s - 8.0, at_top[0] + b[3] * s + 8.0);
        d.strokes.push(Stroke { pen: Pen::Center, points: vec![[xa + 5.0, line_y], [xb - 5.0, line_y]] });
        for (x, inward) in [(xa, 1.0), (xb, -1.0)] {
            d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[x, line_y], [x + inward * 5.0, line_y]] });
            // The arrows point the way the section is looked at (+y, up this view).
            d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x + inward * 1.5, line_y], [x + inward * 1.5, line_y + 6.0]] });
            arrow(&mut d, [x + inward * 1.5, line_y + 7.0], [0.0, 1.0]);
            d.texts.push(Text { at: [x + inward * 1.5 - inward * 3.5, line_y + 3.0], size: 3.5, text: "A".into(), anchor: Anchor::Middle, angle: 0.0, bold: true });
        }
    }
    let hole_views: Vec<([f64; 2], usize)> = [(at_front, 1, has_f), (at_top, 2, has_t), (at_right, 0, has_r)].into_iter().filter(|v| v.2).map(|v| (v.0, v.1)).collect();
    holes(&mut d, arena, shape, deflection, s, hole_views, &options.hole_fits);
    let geometry_end = d.strokes.len();
    if let Some((k, k_text)) = iso_scale {
        let at = place(&iso, [iso_cell[0] + room, iso_cell[1] + 8.0 + room], iso_cell[2] - iso_cell[0] - 2.0 * room, iso_cell[3] - iso_cell[1] - 8.0 - 2.0 * room, k);
        // Hidden lines are left out of the isometric.
        for l in iso.lines.iter().filter(|l| !l.hidden) {
            d.strokes.push(Stroke { pen: Pen::Visible, points: l.points.iter().map(|p| [at[0] + p[0] * k, at[1] + p[1] * k]).collect() });
        }
        let outline = [at[0] + iso.min[0] * k, at[1] + iso.min[1] * k, at[0] + iso.max[0] * k, at[1] + iso.max[1] * k];
        d.iso = Some(IsoFrame { at, scale: k, bounds: outline });
        let mut label_y = outline[1] - 6.0;
        if ballooned {
            balloons(&mut d, arena, parts, &title.parts, deflection, &iso, at, k, outline);
            // Under the ring of balloons.
            label_y = label_y.min(outline[1] - BALLOON_GAP - BALLOON_R - 6.0);
        }
        let text = if k == s { "ISOMETRIC".to_string() } else { format!("ISOMETRIC {k_text}") };
        d.texts.push(Text { at: [(outline[0] + outline[2]) / 2.0, label_y], size: 2.5, text, anchor: Anchor::Middle, angle: 0.0, bold: false });
    }
    // Overall dimensions: width over the front view, height beside it, depth beside the top view.
    let box_of = |v: &View, at: [f64; 2]| ([at[0] + v.min[0] * s, at[1] + v.min[1] * s], [at[0] + v.max[0] * s, at[1] + v.max[1] * s]);
    let (f0, f1) = box_of(&front, at_front);
    let (t0, t1) = box_of(&top, at_top);
    let above_front = match options.projection {
        Projection::First => true,  // the top view is below
        Projection::Third => false, // the top view is above: dimension below instead
    };
    let (r0, r1) = box_of(&right, at_right);
    if has_f {
        if above_front {
            dim_h(&mut d, f0[0], f1[0], f1[1], f1[1] + 9.0, size3[0]);
        } else {
            dim_h(&mut d, f0[0], f1[0], f0[1], f0[1] - 9.0, size3[0]);
        }
        dim_v(&mut d, f0[1], f1[1], f1[0], f1[0] + 9.0, size3[2]);
    } else {
        // Without the front view: the width over the top view, the height beside the side view.
        if has_t {
            dim_h(&mut d, t0[0], t1[0], t1[1], t1[1] + 9.0, size3[0]);
        }
        if has_r {
            dim_v(&mut d, r0[1], r1[1], r1[0], r1[0] + 9.0, size3[2]);
        }
    }
    if has_t {
        // Clear of the cutting-plane arrows when there is a section.
        dim_v(&mut d, t0[1], t1[1], t1[0], t1[0] + if cut.is_some() { 23.0 } else { 9.0 }, size3[1]);
    } else if has_r {
        dim_h(&mut d, r0[0], r1[0], r1[1], r1[1] + 9.0, size3[1]);
    }
    if cut.is_some() {
        let y = if above_front { f0[1] - 7.0 } else { f0[1] - 16.0 };
        d.texts.push(Text { at: [(f0[0] + f1[0]) / 2.0, y], size: 3.5, text: "SECTION A–A".into(), anchor: Anchor::Middle, angle: 0.0, bold: true });
    }
    // The person's own dimensions.
    for (index, dim) in options.dimensions.iter().enumerate() {
        if dim.detail.is_some() {
            continue; // drawn with its detail view
        }
        let Some(frame) = d.views.iter().find(|v| v.view == dim.view).cloned() else {
            d.warnings.push(format!("a dimension is on the {:?} view, which this sheet leaves out", dim.view).to_lowercase());
            continue;
        };
        let on_sheet = |p: [f64; 2]| [frame.at[0] + p[0] * s, frame.at[1] + p[1] * s];
        user_dim(&mut d, index, dim, on_sheet(dim.a), on_sheet(dim.b));
    }

    title_block(&mut d, [fx1 - block[0], fy0], block, title, options.projection);
    if list_h > 0.0 {
        parts_list(&mut d, [fx1 - block[0], fy0 + block[1]], block[0], &title.parts);
    }
    d.texts.push(Text {
        at: [fx1 - block[0], fy0 + block[1] + list_h + 3.0],
        size: 2.5,
        text: "Dimensions in mm · General tolerances ISO 2768-m · Do not scale the drawing".into(),
        anchor: Anchor::Start,
        angle: 0.0,
        bold: false,
    });
    let reserved = vec![[fx1 - block[0] - 2.0, fy0, fx1, fy0 + block[1] + list_h + 7.0]];
    annotate::annotate(&mut d, options, &annotate::Geometry { strokes: geometry_start..geometry_end, section: section_segments, frame: [fx0, fy0, fx1, fy1], reserved });
    Ok(d)
}

/// One of the person's dimensions, between two sheet points, and its mark.
fn user_dim(d: &mut Drawing, index: usize, dim: &DimSpec, pa: [f64; 2], pb: [f64; 2]) {
    let value = dim.value();
    if value <= 1e-9 {
        return;
    }
    let (strokes, texts, fills) = (d.strokes.len(), d.texts.len(), d.fills.len());
    dim_between(d, pa, pb, dim.kind, dim.offset, &annotate::label(value, dim.diameter, dim.tolerance.as_ref()));
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.strokes[strokes..].iter().flat_map(|s| &s.points).chain(d.fills[fills..].iter().flatten()) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    for t in &d.texts[texts..] {
        let r = text_box(t);
        b = [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])];
    }
    d.marks.push(DimMark { dimension: index, bounds: b });
}

/// Balloon radius and how far balloon centres stand off the view (sheet mm).
const BALLOON_R: f64 = 4.0;
const BALLOON_GAP: f64 = 10.0;

/// Item balloons round the isometric view, one per row of the parts list:
/// each leader ends on the part where it shows widest, the balloons ring
/// the view in the order their parts lie round it so no two leaders cross.
#[allow(clippy::too_many_arguments)]
fn balloons(d: &mut Drawing, arena: &mut Arena, parts: &[(String, Shape)], rows: &[PartRow], deflection: f64, iso: &View, at: [f64; 2], k: f64, outline: [f64; 4]) {
    let (dir, up) = iso_frame();
    let shapes: Vec<Shape> = parts.iter().map(|p| p.1).collect();
    let seen = visible_points(arena, &shapes, deflection, dir, up, iso.min, iso.max);
    let mut items: Vec<(usize, [f64; 2])> = vec![];
    for (i, row) in rows.iter().enumerate() {
        let copies: Vec<usize> = (0..parts.len()).filter(|&j| parts[j].0 == row.name).collect();
        // The copy that shows most; one hidden behind the others gets its middle.
        let best = copies.iter().filter_map(|&j| seen[j]).max_by(|a, b| a.1.total_cmp(&b.1)).map(|v| v.0);
        let point = best.or_else(|| {
            let b = arena.bounds(parts[*copies.first()?].1).ok()?;
            Some(iso_point([(b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0, (b[2] + b[5]) / 2.0]))
        });
        if let Some(p) = point {
            items.push((i + 1, [at[0] + p[0] * k, at[1] + p[1] * k]));
        }
    }
    let anchors: Vec<[f64; 2]> = items.iter().map(|i| i.1).collect();
    let centres = balloon_ring(&anchors, outline, BALLOON_R, BALLOON_GAP);
    for ((item, a), c) in items.into_iter().zip(centres) {
        let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
        let len = dx.hypot(dy);
        if len > BALLOON_R {
            d.strokes.push(Stroke { pen: Pen::Thin, points: vec![a, [c[0] - dx / len * BALLOON_R, c[1] - dy / len * BALLOON_R]] });
        }
        // The leader ends in a dot on the part (ISO 128-22).
        d.fills.push((0..12).map(|i| circle_point(a, 0.75, i as f64 / 12.0)).collect());
        d.strokes.push(Stroke { pen: Pen::Thin, points: (0..=48).map(|i| circle_point(c, BALLOON_R, i as f64 / 48.0)).collect() });
        d.texts.push(Text { at: [c[0], c[1] - 1.75], size: 3.5, text: item.to_string(), anchor: Anchor::Middle, angle: 0.0, bold: false });
        d.balloons.push(Balloon { item, centre: c, radius: BALLOON_R, anchor: a });
    }
}

fn circle_point(c: [f64; 2], r: f64, turn: f64) -> [f64; 2] {
    let (s, co) = (turn * std::f64::consts::TAU).sin_cos();
    [c[0] + r * co, c[1] + r * s]
}

/// Each shape's most visible point in a view, and how far it is from the
/// edge of what shows of it (view mm); None when nothing of it shows. A depth
/// buffer over the view (the meshes projected as the hidden-line view
/// projects them), then per shape the pixel deepest inside one of its faces
/// as they show (so a leader lands on a face, not on the edge between two).
fn visible_points(arena: &mut Arena, shapes: &[Shape], deflection: f64, dir: [f64; 3], up: [f64; 3], min: [f64; 2], max: [f64; 2]) -> Vec<Option<([f64; 2], f64)>> {
    let px = ((max[0] - min[0]).max(max[1] - min[1]) / 360.0).max(1e-6);
    let origin = [min[0] - 2.0 * px, min[1] - 2.0 * px];
    let (nx, ny) = (((max[0] - min[0]) / px) as usize + 5, ((max[1] - min[1]) / px) as usize + 5);
    let mut depth = vec![f64::NEG_INFINITY; nx * ny];
    let mut owner = vec![usize::MAX; nx * ny];
    let mut normal = vec![[0.0f64; 3]; nx * ny];
    for (i, &s) in shapes.iter().enumerate() {
        let Ok(mesh) = arena.mesh(s, deflection) else { continue };
        let v: Vec<[f64; 3]> = mesh
            .positions
            .chunks_exact(3)
            .map(|p| {
                let q = view_point(dir, up, [p[0], p[1], p[2]]);
                [(q[0] - origin[0]) / px, (q[1] - origin[1]) / px, q[2]]
            })
            .collect();
        for t in mesh.triangles.chunks_exact(3) {
            let [a, b, c] = [v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]];
            let corner = |k: u32| [mesh.positions[k as usize * 3], mesh.positions[k as usize * 3 + 1], mesh.positions[k as usize * 3 + 2]];
            let (p0, p1, p2) = (corner(t[0]), corner(t[1]), corner(t[2]));
            let facing = cross([p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]], [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]]);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-12 {
                continue; // seen edge on
            }
            let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
            let x1 = (a[0].max(b[0]).max(c[0]).ceil().max(0.0) as usize).min(nx - 1);
            let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
            let y1 = (a[1].max(b[1]).max(c[1]).ceil().max(0.0) as usize).min(ny - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = [x as f64 + 0.5, y as f64 + 0.5];
                    // Barycentric weights of the pixel centre.
                    let wa = ((b[0] - p[0]) * (c[1] - p[1]) - (b[1] - p[1]) * (c[0] - p[0])) / area;
                    let wb = ((c[0] - p[0]) * (a[1] - p[1]) - (c[1] - p[1]) * (a[0] - p[0])) / area;
                    let wc = 1.0 - wa - wb;
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    let z = wa * a[2] + wb * b[2] + wc * c[2];
                    let k = y * nx + x;
                    if z > depth[k] {
                        depth[k] = z;
                        owner[k] = i;
                        normal[k] = facing;
                    }
                }
            }
        }
    }
    for n in normal.iter_mut().filter(|n| n.iter().any(|v| *v != 0.0)) {
        *n = unit(*n);
    }
    // Faces meet where the surface turns sharply (more than 30°); a curved
    // face's facets turn far less from one to the next.
    let other = |a: usize, b: usize| owner[a] != owner[b] || normal[a][0] * normal[b][0] + normal[a][1] * normal[b][1] + normal[a][2] * normal[b][2] < 0.866;
    // Distance (pixels) to the nearest pixel showing another face: a
    // two-pass chamfer transform from the edges of every region at once.
    let mut dist = vec![f64::INFINITY; nx * ny];
    for y in 0..ny {
        for x in 0..nx {
            let k = y * nx + x;
            if x == 0 || y == 0 || x == nx - 1 || y == ny - 1 || [k - 1, k + 1, k - nx, k + nx].iter().any(|&n| other(n, k)) {
                dist[k] = 0.0;
            }
        }
    }
    let diag = std::f64::consts::SQRT_2;
    for y in 1..ny - 1 {
        for x in 1..nx - 1 {
            let k = y * nx + x;
            dist[k] = dist[k].min(dist[k - 1] + 1.0).min(dist[k - nx] + 1.0).min(dist[k - nx - 1] + diag).min(dist[k - nx + 1] + diag);
        }
    }
    for y in (1..ny - 1).rev() {
        for x in (1..nx - 1).rev() {
            let k = y * nx + x;
            dist[k] = dist[k].min(dist[k + 1] + 1.0).min(dist[k + nx] + 1.0).min(dist[k + nx + 1] + diag).min(dist[k + nx - 1] + diag);
        }
    }
    let mut best: Vec<Option<([f64; 2], f64)>> = vec![None; shapes.len()];
    for (k, &o) in owner.iter().enumerate() {
        if o == usize::MAX || best[o].is_some_and(|b| b.1 >= dist[k] * px) {
            continue;
        }
        let (x, y) = (k % nx, k / nx);
        best[o] = Some(([origin[0] + (x as f64 + 0.5) * px, origin[1] + (y as f64 + 0.5) * px], dist[k] * px));
    }
    best
}

/// Where the balloons for `anchors` go: on a ring `gap` outside `outline`
/// ([x0, y0, x1, y1]), each first where the ray from the view's middle
/// through its anchor meets the ring, then spread apart along it (keeping
/// their order round it), and any two whose leaders still cross swapped.
pub fn balloon_ring(anchors: &[[f64; 2]], outline: [f64; 4], r: f64, gap: f64) -> Vec<[f64; 2]> {
    let n = anchors.len();
    if n == 0 {
        return vec![];
    }
    let ring = [outline[0] - gap, outline[1] - gap, outline[2] + gap, outline[3] + gap];
    let (w, h) = (ring[2] - ring[0], ring[3] - ring[1]);
    let perimeter = 2.0 * (w + h);
    let centre = [(ring[0] + ring[2]) / 2.0, (ring[1] + ring[3]) / 2.0];
    // A point round the ring, by its distance anticlockwise from the lower left corner.
    let point = |t: f64| {
        let t = t.rem_euclid(perimeter);
        if t < w {
            [ring[0] + t, ring[1]]
        } else if t < w + h {
            [ring[2], ring[1] + t - w]
        } else if t < 2.0 * w + h {
            [ring[2] - (t - w - h), ring[3]]
        } else {
            [ring[0], ring[3] - (t - 2.0 * w - h)]
        }
    };
    let ideal: Vec<f64> = anchors
        .iter()
        .map(|a| {
            let (mut dx, mut dy) = (a[0] - centre[0], a[1] - centre[1]);
            if dx.hypot(dy) < 1e-9 {
                (dx, dy) = (-1.0, 1.0);
            }
            // Out along the ray to whichever side it meets first.
            let (sx, sy) = (w / 2.0 / dx.abs().max(1e-12), h / 2.0 / dy.abs().max(1e-12));
            if sx <= sy {
                let y = centre[1] + dy * sx;
                if dx > 0.0 { w + y - ring[1] } else { 2.0 * w + h + ring[3] - y }
            } else {
                let x = centre[0] + dx * sy;
                if dy < 0.0 { x - ring[0] } else { w + h + ring[2] - x }
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| ideal[a].total_cmp(&ideal[b]));
    // Spread them along the ring; round a corner the straight distance is
    // shorter than the distance along, so widen the spacing until it is enough.
    let mut spacing = 2.0 * r + 2.0;
    let mut t = ideal.clone();
    for _ in 0..12 {
        let space = spacing.min(perimeter / n as f64);
        t = ideal.clone();
        if n > 1 {
            for _ in 0..1000 {
                let mut moved = false;
                for j in 0..n {
                    let (a, b) = (order[j], order[(j + 1) % n]);
                    let between = if j + 1 == n { t[b] + perimeter - t[a] } else { t[b] - t[a] };
                    if between < space - 1e-6 {
                        let push = (space - between) / 2.0 + 1e-7;
                        t[a] -= push;
                        t[b] += push;
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        let pts: Vec<[f64; 2]> = t.iter().map(|&v| point(v)).collect();
        let clear = (0..n).all(|i| (i + 1..n).all(|j| (pts[i][0] - pts[j][0]).hypot(pts[i][1] - pts[j][1]) >= 2.0 * r + 1.0));
        if clear || space < spacing {
            break;
        }
        spacing *= 1.15;
    }
    let mut out: Vec<[f64; 2]> = t.iter().map(|&v| point(v)).collect();
    // Swapping the ends of two crossing leaders always shortens them, so this ends.
    for _ in 0..100 {
        let mut swapped = false;
        for i in 0..n {
            for j in i + 1..n {
                if segments_cross(anchors[i], out[i], anchors[j], out[j]) {
                    out.swap(i, j);
                    swapped = true;
                }
            }
        }
        if !swapped {
            break;
        }
    }
    out
}

/// Whether segments pq and rs cross (properly, not just touching).
pub fn segments_cross(p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]) -> bool {
    let side = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let (d1, d2, d3, d4) = (side(p, q, r), side(p, q, s), side(r, s, p), side(r, s, q));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Holes and round bosses facing a view: centre marks in that view, and one
/// ⌀ callout per size (in the plan view if they show there).
/// `views` are each view's placement and the axis it looks along (0 x, 1 y, 2 z).
fn holes(d: &mut Drawing, arena: &mut Arena, shape: Shape, deflection: f64, s: f64, views: Vec<([f64; 2], usize)>, fits: &[HoleFit]) {
    let Ok(edges) = arena.edges(shape, deflection) else { return };
    // Full circles only (an arc is a rounded edge, not a hole).
    let circles: Vec<([f64; 3], [f64; 3], f64)> = edges
        .iter()
        .filter_map(|e| match e.kind {
            de_occt::EdgeKind::Circle { center, axis, radius } => {
                let (a, b) = (e.points.first()?, e.points.last()?);
                (e.points.len() > 2 && (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]) < 1e-6).then_some((center, axis, radius))
            }
            _ => None,
        })
        .collect();
    // A view's sheet coordinates of a model point.
    let project = |p: [f64; 3], axis: usize| -> [f64; 2] {
        match axis {
            1 => [p[0], p[2]], // front: x right, z up
            2 => [p[0], p[1]], // top: x right, y up
            _ => [p[1], p[2]], // right: y right, z up
        }
    };
    let mut called: Vec<f64> = vec![];
    // The plan view first, so callouts land there when they can.
    let mut order = views;
    order.sort_by_key(|(_, axis)| if *axis == 2 { 0 } else { 1 });
    for (at, axis) in order {
        let mut seen: Vec<([f64; 2], f64)> = vec![];
        for (c, ax, r) in &circles {
            if ax[axis].abs() < 0.999 {
                continue;
            }
            let p = project(*c, axis);
            if seen.iter().any(|(q, rq)| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-4 && (rq - r).abs() < 1e-4) {
                continue;
            }
            // Too small to read at this scale (a chamfer's sliver): left out.
            if r * s < 1.0 {
                continue;
            }
            seen.push((p, *r));
        }
        for (p, r) in &seen {
            let c = [at[0] + p[0] * s, at[1] + p[1] * s];
            let reach = r * s + 2.5;
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![[c[0] - reach, c[1]], [c[0] + reach, c[1]]] });
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![[c[0], c[1] - reach], [c[0], c[1] + reach]] });
        }
        // Holes seen from the side: a centre line along the axis, end to end.
        let along: Vec<(usize, f64)> = (0..3).filter(|k| *k != axis).map(|k| (k, 0.0)).collect();
        let mut runs: Vec<(usize, [f64; 2], f64, f64, f64)> = vec![]; // (axis k, centre across, from, to, r)
        for (c, ax, r) in &circles {
            let Some(&(k, _)) = along.iter().find(|(k, _)| ax[*k].abs() > 0.999) else { continue };
            let p = project(*c, axis);
            // In view coordinates, the axis runs along x when k is the view's right, else along y.
            let horizontal = matches!((axis, k), (1, 0) | (2, 0) | (0, 1));
            let (t, across) = if horizontal { (p[0], [p[1], 0.0]) } else { (p[1], [p[0], 0.0]) };
            match runs.iter_mut().find(|(kk, a, _, _, _)| *kk == k && (a[0] - across[0]).abs() < 1e-4) {
                Some(run) => {
                    run.2 = run.2.min(t);
                    run.3 = run.3.max(t);
                    run.4 = run.4.max(*r);
                }
                None => runs.push((k, across, t, t, *r)),
            }
        }
        for (k, across, from, to, _) in runs {
            if to - from < 1e-6 {
                continue;
            }
            let horizontal = matches!((axis, k), (1, 0) | (2, 0) | (0, 1));
            let (a, b) = if horizontal { ([from, across[0]], [to, across[0]]) } else { ([across[0], from], [across[0], to]) };
            let (a, b) = ([at[0] + a[0] * s, at[1] + a[1] * s], [at[0] + b[0] * s, at[1] + b[1] * s]);
            let dir = if horizontal { [1.0, 0.0] } else { [0.0, 1.0] };
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![[a[0] - dir[0] * 2.5, a[1] - dir[1] * 2.5], [b[0] + dir[0] * 2.5, b[1] + dir[1] * 2.5]] });
        }
        // One callout per size not yet called out, on its first hole.
        let mut sizes: Vec<f64> = seen.iter().map(|(_, r)| (r * 1000.0).round() / 1000.0).collect();
        sizes.sort_by(|a, b| b.total_cmp(a));
        sizes.dedup();
        for r in sizes.into_iter().take(6) {
            if called.iter().any(|q| (q - r).abs() < 1e-3) {
                continue;
            }
            called.push(r);
            let same: Vec<&([f64; 2], f64)> = seen.iter().filter(|(_, q)| (q - r).abs() < 1e-3).collect();
            let fit = fits.iter().find(|f| (f.diameter - 2.0 * r).abs() < 1e-3).map(|f| Tolerance::Fit { fit: f.fit.clone() });
            let mut label = annotate::label(2.0 * r, true, fit.as_ref());
            if same.len() > 1 {
                label.main = format!("{}× {}", same.len(), label.main);
            }
            let tall = if fit.is_some() { 7.0 } else { 5.0 };
            // Try each hole of this size, leaders at several angles and lengths;
            // take the first spot whose label overlaps no other text.
            let mut placed = false;
            'spot: for (p, _) in &same {
                let c = [at[0] + p[0] * s, at[1] + p[1] * s];
                for reach in [7.0, 12.0, 18.0] {
                    for deg in [45.0f64, 135.0, -45.0, -135.0, 60.0, 120.0, -60.0, -120.0] {
                        let (sn, cs) = deg.to_radians().sin_cos();
                        let tip = [c[0] + r * s * cs, c[1] + r * s * sn];
                        let knee = [tip[0] + reach * cs, tip[1] + reach * sn];
                        let right = cs >= 0.0;
                        let width = annotate::label_width(&label, 3.5);
                        let end = [knee[0] + if right { width + 2.0 } else { -(width + 2.0) }, knee[1]];
                        let bx = [knee[0].min(end[0]) - 0.5, knee[1] - 0.5, knee[0].max(end[0]) + 0.5, knee[1] + tall];
                        if bx[0] < 0.0 || bx[2] > d.width || bx[1] < 0.0 || bx[3] > d.height || d.texts.iter().any(|o| overlaps(bx, text_box(o))) {
                            continue;
                        }
                        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![tip, knee, end] });
                        arrow(d, tip, [-cs, -sn]);
                        annotate::place_label(d, &label, [knee[0] + if right { 1.0 } else { -1.0 }, knee[1] + 1.0], if right { Anchor::Start } else { Anchor::End }, 0.0, 3.5);
                        d.annotations.push(AnnotationMark { kind: MarkKind::Hole, index: 0, diameter: Some((2.0 * r * 1000.0).round() / 1000.0), bounds: bx, origin: None, anchor: Some(c) });
                        placed = true;
                        break 'spot;
                    }
                }
            }
            let _ = placed;
        }
    }
}

/// A text's box on the sheet (x0, y0, x1, y1), for keeping labels apart.
fn text_box(t: &Text) -> [f64; 4] {
    let w = text_width(t);
    let (x0, x1) = match t.anchor {
        Anchor::Start => (t.at[0], t.at[0] + w),
        Anchor::Middle => (t.at[0] - w / 2.0, t.at[0] + w / 2.0),
        Anchor::End => (t.at[0] - w, t.at[0]),
    };
    if t.angle.abs() > 45.0 {
        // Turned upright: the box turns with it (about its anchor).
        let (cx, cy) = (t.at[0], t.at[1]);
        return [cx - t.size * 1.2, cy - (cx - x0), cx + 0.2, cy + (x1 - cx)];
    }
    [x0, t.at[1] - t.size * 0.25, x1, t.at[1] + t.size * 1.2]
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn arrow(d: &mut Drawing, tip: [f64; 2], dir: [f64; 2]) {
    // A filled arrowhead 3 mm long, 1 mm wide (ISO 129).
    let (l, half) = (3.0, 0.5);
    let n = [-dir[1], dir[0]];
    let back = [tip[0] - dir[0] * l, tip[1] - dir[1] * l];
    d.fills.push(vec![tip, [back[0] + n[0] * half, back[1] + n[1] * half], [back[0] - n[0] * half, back[1] - n[1] * half]]);
}

/// A horizontal dimension from x0 to x1, measured at `from_y`, its line at `y`
/// (a short one with its arrows and value outside, as any other).
fn dim_h(d: &mut Drawing, x0: f64, x1: f64, from_y: f64, y: f64, value: f64) {
    dim_between(d, [x0, from_y], [x1, from_y], DimKind::Horizontal, y - from_y, &annotate::label(value, false, None));
}

/// A vertical dimension from y0 to y1, measured at `from_x`, its line at `x`;
/// read from the right (ISO 129): the text runs up the page, left of the line.
fn dim_v(d: &mut Drawing, y0: f64, y1: f64, from_x: f64, x: f64, value: f64) {
    dim_between(d, [from_x, y0], [from_x, y1], DimKind::Vertical, from_x - x, &annotate::label(value, false, None));
}

/// Where a closed mesh crosses the plane y = `y`: segments in (x, z).
fn slice(positions: &[f64], triangles: &[u32], y: f64) -> Vec<[[f64; 2]; 2]> {
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i], positions[i + 1], positions[i + 2]]
    };
    let mut out = vec![];
    for t in triangles.chunks_exact(3) {
        let v = [p(t[0]), p(t[1]), p(t[2])];
        let mut hits = vec![];
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let (da, db) = (v[a][1] - y, v[b][1] - y);
            if (da < 0.0) != (db < 0.0) {
                let k = da / (da - db);
                hits.push([v[a][0] + k * (v[b][0] - v[a][0]), v[a][2] + k * (v[b][2] - v[a][2])]);
            }
        }
        if hits.len() == 2 {
            out.push([hits[0], hits[1]]);
        }
    }
    out
}

/// 45° hatching inside the region the segments bound (even–odd), `spacing` mm apart.
fn hatch(d: &mut Drawing, segments: &[[[f64; 2]; 2]], spacing: f64) {
    for [a, b] in hatch_lines(segments, spacing) {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![a, b] });
    }
}

fn hatch_lines(segments: &[[[f64; 2]; 2]], spacing: f64) -> Vec<[[f64; 2]; 2]> {
    let mut out = vec![];
    if segments.is_empty() {
        return out;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in segments.iter().flatten() {
        lo = lo.min(p[0] - p[1]);
        hi = hi.max(p[0] - p[1]);
    }
    let step = spacing * std::f64::consts::SQRT_2;
    // Offset off the grid so lines do not pass exactly through corners.
    let mut c = lo + step * 0.437;
    while c < hi {
        let mut cross: Vec<[f64; 2]> = segments
            .iter()
            .filter_map(|[p, q]| {
                let (fp, fq) = (p[0] - p[1] - c, q[0] - q[1] - c);
                if (fp < 0.0) == (fq < 0.0) {
                    return None;
                }
                let k = fp / (fp - fq);
                Some([p[0] + k * (q[0] - p[0]), p[1] + k * (q[1] - p[1])])
            })
            .collect();
        cross.sort_by(|a, b| (a[0] + a[1]).total_cmp(&(b[0] + b[1])));
        for pair in cross.chunks_exact(2) {
            out.push([pair[0], pair[1]]);
        }
        c += step;
    }
    out
}

/// A dimension between two sheet points, its line `offset` mm off them.
fn dim_between(d: &mut Drawing, pa: [f64; 2], pb: [f64; 2], kind: DimKind, offset: f64, label: &annotate::Label) {
    let (u, n) = match kind {
        DimKind::Horizontal => ([1.0, 0.0], [0.0, 1.0]),
        DimKind::Vertical => ([0.0, 1.0], [-1.0, 0.0]),
        DimKind::Aligned => {
            let l = (pb[0] - pa[0]).hypot(pb[1] - pa[1]).max(1e-9);
            let u = [(pb[0] - pa[0]) / l, (pb[1] - pa[1]) / l];
            (u, [-u[1], u[0]])
        }
    };
    let dot = |p: [f64; 2], v: [f64; 2]| p[0] * v[0] + p[1] * v[1];
    // The line stands `offset` beyond the farther point (along n).
    let level = if offset >= 0.0 { dot(pa, n).max(dot(pb, n)) + offset } else { dot(pa, n).min(dot(pb, n)) + offset };
    let side = offset.signum();
    let foot = |p: [f64; 2]| {
        let k = level - dot(p, n);
        [p[0] + n[0] * k, p[1] + n[1] * k]
    };
    let (ea, eb) = (foot(pa), foot(pb));
    for (p, e) in [(pa, ea), (pb, eb)] {
        if (dot(e, n) - dot(p, n)).abs() > 1.5 {
            d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[p[0] + n[0] * side, p[1] + n[1] * side], [e[0] + n[0] * side * 2.0, e[1] + n[1] * side * 2.0]] });
        }
    }
    let along = if dot(eb, u) >= dot(ea, u) { u } else { [-u[0], -u[1]] };
    let length = (eb[0] - ea[0]).hypot(eb[1] - ea[1]);
    if length >= 7.0 {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![ea, eb] });
        arrow(d, ea, [-along[0], -along[1]]);
        arrow(d, eb, along);
    } else {
        // Too short for arrows inside: they point in from outside (ISO 129-1).
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[ea[0] - along[0] * 5.0, ea[1] - along[1] * 5.0], [eb[0] + along[0] * 5.0, eb[1] + along[1] * 5.0]] });
        arrow(d, ea, along);
        arrow(d, eb, [-along[0], -along[1]]);
    }
    // Text above the line, upright or reading from the right (ISO 129).
    let mut angle = along[1].atan2(along[0]).to_degrees();
    if angle > 90.0 + 1e-6 {
        angle -= 180.0;
    } else if angle <= -90.0 + 1e-6 {
        angle += 180.0;
    }
    let up = [-(angle.to_radians().sin()), angle.to_radians().cos()];
    let mid = [(ea[0] + eb[0]) / 2.0 + up[0] * 1.2, (ea[1] + eb[1]) / 2.0 + up[1] * 1.2];
    let width = annotate::label_width(label, 3.5);
    if width + 2.0 <= length {
        annotate::place_label(d, label, mid, Anchor::Middle, angle, 3.5);
    } else {
        // No room between the extension lines: the value goes beyond the
        // end it reads towards, over the dimension line drawn on under it.
        let read = [angle.to_radians().cos(), angle.to_radians().sin()];
        let end = if dot(eb, read) >= dot(ea, read) { eb } else { ea };
        let reach = if length >= 7.0 { 1.5 } else { 5.0 };
        let start = [end[0] + read[0] * reach, end[1] + read[1] * reach];
        let tail = [start[0] + read[0] * (width + 1.0), start[1] + read[1] * (width + 1.0)];
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![end, tail] });
        annotate::place_label(d, label, [start[0] + up[0] * 1.2, start[1] + up[1] * 1.2], Anchor::Start, angle, 3.5);
    }
}

/// The parts list on top of the title block, numbered from the bottom up (ISO 7573).
fn parts_list(d: &mut Drawing, at: [f64; 2], width: f64, rows: &[PartRow]) {
    let [x, y] = at;
    let row_h = 6.0;
    let h = row_h * (rows.len() + 1) as f64;
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[x, y], [x + width, y], [x + width, y + h], [x, y + h], [x, y]] });
    let cols = [0.0, 0.08, 0.52, 0.62, 0.84].map(|c| x + c * width);
    for c in &cols[1..] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[*c, y], [*c, y + h]] });
    }
    for r in 1..=rows.len() {
        let ry = y + r as f64 * row_h;
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, ry], [x + width, ry]] });
    }
    let cell = |d: &mut Drawing, r: usize, values: [&str; 5], bold: bool| {
        let ry = y + r as f64 * row_h + 1.9;
        for (k, v) in values.iter().enumerate() {
            let (cx, anchor) = if k == 0 || k == 2 { ((cols[k] + if k + 1 < 5 { cols[k + 1] } else { x + width }) / 2.0, Anchor::Middle) } else { (cols[k] + 1.5, Anchor::Start) };
            d.texts.push(Text { at: [cx, ry], size: if bold { 2.0 } else { 2.5 }, text: v.to_string(), anchor, angle: 0.0, bold });
        }
    };
    // The header is the top row.
    cell(d, rows.len(), ["ITEM", "NAME", "QTY", "MATERIAL", "MASS EACH"], true);
    for (i, row) in rows.iter().enumerate() {
        cell(d, i, [&(i + 1).to_string(), &row.name, &row.quantity.to_string(), &row.material, &row.mass], false);
    }
}

fn title_block(d: &mut Drawing, at: [f64; 2], size: [f64; 2], t: &Title, projection: Projection) {
    let [x, y] = at;
    let [w, h] = size;
    let line = |d: &mut Drawing, a: [f64; 2], b: [f64; 2], pen: Pen| d.strokes.push(Stroke { pen, points: vec![a, b] });
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]] });
    // Rows from the bottom: the title, then material/mass/scale/projection, then who/when.
    let r1 = y + 14.0;
    let r2 = y + 23.0;
    line(d, [x, r1], [x + w, r1], Pen::Thin);
    line(d, [x, r2], [x + w, r2], Pen::Thin);
    let cell = |d: &mut Drawing, x0: f64, y0: f64, label: &str, value: &str, size: f64, bold: bool| {
        d.texts.push(Text { at: [x0 + 1.5, y0 + if size > 4.0 { 9.0 } else { 6.2 }], size: 1.8, text: label.to_uppercase(), anchor: Anchor::Start, angle: 0.0, bold: false });
        d.texts.push(Text { at: [x0 + 1.5, y0 + 1.8], size, text: value.to_string(), anchor: Anchor::Start, angle: 0.0, bold });
    };
    // Bottom row: title (wide) and drawing number.
    let split = x + w * 0.66;
    line(d, [split, y], [split, r1], Pen::Thin);
    cell(d, x, y, "Title", &t.title, 5.0, true);
    cell(d, split, y, "Drawing no.", &t.number, 3.0, false);
    // Middle row: material, mass, scale, projection.
    let cols = [x, x + w * 0.38, x + w * 0.58, x + w * 0.76];
    for c in &cols[1..] {
        line(d, [*c, r1], [*c, r2], Pen::Thin);
    }
    cell(d, cols[0], r1, "Material", &t.material, 2.5, false);
    cell(d, cols[1], r1, "Mass", &t.mass, 2.5, false);
    cell(d, cols[2], r1, "Scale", &d.scale_text.clone(), 2.5, false);
    projection_symbol(d, [cols[3] + (x + w - cols[3]) / 2.0, r1 + 4.5], projection);
    // Top row: drawn by, date, sheet.
    let tops = [x, x + w * 0.38, x + w * 0.66];
    for c in &tops[1..] {
        line(d, [*c, r2], [*c, y + h], Pen::Thin);
    }
    cell(d, tops[0], r2, "Drawn by", &t.drawn_by, 2.5, false);
    cell(d, tops[1], r2, "Date", &t.date, 2.5, false);
    let sheet = format!("{:?} · {} of {}", sheet_name(d.width), d.page.max(1), d.pages.max(d.page).max(1));
    cell(d, tops[2], r2, "Sheet", &sheet, 2.5, false);
}

fn sheet_name(width: f64) -> Sheet {
    [Sheet::A4, Sheet::A3, Sheet::A2, Sheet::A1, Sheet::A0].into_iter().find(|s| (s.size()[0] - width).abs() < 1.0).unwrap_or_default()
}

/// The projection symbol (ISO 128-30): a truncated cone and its end view.
fn projection_symbol(d: &mut Drawing, c: [f64; 2], projection: Projection) {
    let (h, small, len) = (3.2, 1.6, 6.0);
    // The cone's side view: narrow end on the left.
    let (cone_x, circles_x) = match projection {
        Projection::First => (c[0] - 5.5, c[0] + 4.5),
        Projection::Third => (c[0] - 1.5, c[0] - 6.5),
    };
    d.strokes.push(Stroke {
        pen: Pen::Thin,
        points: vec![[cone_x, c[1] - small], [cone_x + len, c[1] - h], [cone_x + len, c[1] + h], [cone_x, c[1] + small], [cone_x, c[1] - small]],
    });
    for r in [h, small] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: (0..=48).map(|i| (i as f64) / 48.0 * std::f64::consts::TAU).map(|a| [circles_x + r * a.cos(), c[1] + r * a.sin()]).collect() });
    }
}

// ---- Writers ------------------------------------------------------------------

const PT: f64 = 72.0 / 25.4; // points per millimetre

/// Text in PDF's WinAnsi encoding (the standard fonts), escaped for a string.
fn pdf_text(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        let byte: u32 = match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
                continue;
            }
            '·' => 0xB7,
            '°' => 0xB0,
            '×' => 0xD7,
            '±' => 0xB1,
            'Ø' | '⌀' => 0xD8,
            '–' | '−' => 0x96,
            '—' => 0x97,
            '“' => 0x93,
            '”' => 0x94,
            '’' => 0x92,
            c if (c as u32) < 0x80 => {
                out.push(c);
                continue;
            }
            c if (c as u32) < 0x100 => c as u32,
            _ => '?' as u32,
        };
        let _ = write!(out, "\\{byte:03o}");
    }
    out
}

/// Helvetica and Helvetica-Bold advance widths (1/1000 em) for ' ' to '~' (the AFM metrics).
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Width of Helvetica text (mm), for anchoring and keeping labels apart.
fn text_width(t: &Text) -> f64 {
    let table = if t.bold { &HELVETICA_BOLD } else { &HELVETICA };
    let em: u32 = t
        .text
        .chars()
        .map(|c| match c {
            ' '..='~' => table[c as usize - 32] as u32,
            '·' | '’' => 278,
            '°' => 400,
            '×' | '±' => 584,
            'Ø' | '⌀' => 778,
            '–' | '−' => 556,
            '—' => 1000,
            _ => 556,
        })
        .sum();
    // The size is the cap height; Helvetica's caps are 0.718 em.
    em as f64 / 1000.0 * t.size / 0.718
}

/// The drawing as a one-page vector PDF.
pub fn pdf(d: &Drawing) -> Vec<u8> {
    pdf_pages(std::slice::from_ref(d))
}

/// Sheets as one vector PDF, a page each (each page its sheet's size).
pub fn pdf_pages(sheets: &[Drawing]) -> Vec<u8> {
    // 1 catalogue, 2 page tree, 3 and 4 the fonts, then a page and its content per sheet.
    let mut objects = vec![String::new(), String::new(), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_string()];
    let mut kids = vec![];
    for d in sheets {
        let content = pdf_content(d);
        let page = objects.len() + 1;
        kids.push(format!("{page} 0 R"));
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>", d.width * PT, d.height * PT, page + 1));
        objects.push(format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content));
    }
    objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".to_string();
    objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), sheets.len());
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = write!(tail, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
    out.extend(tail.as_bytes());
    out
}

/// A sheet's page content: strokes, fills and text in PDF operators.
fn pdf_content(d: &Drawing) -> String {
    let mut content = String::new();
    let _ = writeln!(content, "1 J 1 j");
    for st in &d.strokes {
        if st.points.len() < 2 {
            continue;
        }
        let _ = write!(content, "{:.3} w ", st.pen.width() * PT);
        if st.pen == Pen::Hidden {
            let _ = write!(content, "[{:.2} {:.2}] 0 d ", 3.0 * PT, 1.5 * PT);
        } else if st.pen == Pen::Center {
            let _ = write!(content, "[{:.2} {:.2} {:.2} {:.2}] 0 d ", 8.0 * PT, 1.5 * PT, 0.5 * PT, 1.5 * PT);
        } else {
            content.push_str("[] 0 d ");
        }
        for (i, p) in st.points.iter().enumerate() {
            let _ = write!(content, "{:.3} {:.3} {} ", p[0] * PT, p[1] * PT, if i == 0 { "m" } else { "l" });
        }
        content.push_str("S\n");
    }
    for f in &d.fills {
        for (i, p) in f.iter().enumerate() {
            let _ = write!(content, "{:.3} {:.3} {} ", p[0] * PT, p[1] * PT, if i == 0 { "m" } else { "l" });
        }
        content.push_str("h f\n");
    }
    for t in &d.texts {
        // Font size from cap height (Helvetica caps are 0.718 em).
        let size = t.size / 0.718 * PT;
        let shift = match t.anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => -text_width(t) / 2.0,
            Anchor::End => -text_width(t),
        };
        let (sn, cs) = t.angle.to_radians().sin_cos();
        let (x, y) = (t.at[0] + shift * cs, t.at[1] + shift * sn);
        let _ = writeln!(content, "BT /{} {:.2} Tf {:.4} {:.4} {:.4} {:.4} {:.3} {:.3} Tm ({}) Tj ET", if t.bold { "F2" } else { "F1" }, size, cs, sn, -sn, cs, x * PT, y * PT, pdf_text(&t.text));
    }
    content
}

/// Files in a zip archive, stored as they are (no compression): one DXF per sheet.
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let crc_table: Vec<u32> = (0..256u32)
        .map(|n| {
            let mut c = n;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            c
        })
        .collect();
    let crc = |data: &[u8]| !data.iter().fold(!0u32, |c, &b| crc_table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8));
    let mut out = vec![];
    let mut central = vec![];
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend(x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_le_bytes());
    for (name, data) in files {
        let (c, size, offset) = (crc(data), data.len() as u32, out.len() as u32);
        // Local header: version 2.0, UTF-8 names, stored, a fixed 1980 timestamp.
        le32(&mut out, 0x0403_4B50);
        for v in [20, 0x0800, 0, 0, 0x21] {
            le16(&mut out, v);
        }
        for v in [c, size, size] {
            le32(&mut out, v);
        }
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend(name.as_bytes());
        out.extend(data);
        le32(&mut central, 0x0201_4B50);
        for v in [20, 20, 0x0800, 0, 0, 0x21] {
            le16(&mut central, v);
        }
        for v in [c, size, size] {
            le32(&mut central, v);
        }
        for v in [name.len() as u16, 0, 0, 0, 0] {
            le16(&mut central, v);
        }
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend(name.as_bytes());
    }
    let (start, len) = (out.len() as u32, central.len() as u32);
    out.extend(&central);
    le32(&mut out, 0x0605_4B50);
    for v in [0, 0, files.len() as u16, files.len() as u16] {
        le16(&mut out, v);
    }
    le32(&mut out, len);
    le32(&mut out, start);
    le16(&mut out, 0);
    out
}

/// The drawing as a DXF (R12, millimetres): layers VISIBLE, HIDDEN (dashed),
/// DIMENSIONS, FRAME and TEXT, so a CAD or laser program can pick them apart.
pub fn dxf(d: &Drawing) -> String {
    let mut o = String::new();
    let mut pair = |code: i32, value: &str| {
        let _ = writeln!(o, "{code}\n{value}");
    };
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    pair(9, "$INSUNITS");
    pair(70, "4");
    pair(9, "$EXTMIN");
    pair(10, "0");
    pair(20, "0");
    pair(9, "$EXTMAX");
    pair(10, &format!("{:.3}", d.width));
    pair(20, &format!("{:.3}", d.height));
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "TABLES");
    pair(0, "TABLE");
    pair(2, "LTYPE");
    pair(70, "3");
    for (name, text, pattern) in [("CONTINUOUS", "Solid line", vec![]), ("HIDDEN", "Hidden __ __ __", vec![3.0, -1.5]), ("CENTER", "Center ____ _ ____", vec![8.0, -1.5, 0.5, -1.5])] {
        pair(0, "LTYPE");
        pair(2, name);
        pair(70, "0");
        pair(3, text);
        pair(72, "65");
        pair(73, &pattern.len().to_string());
        pair(40, &format!("{:.3}", pattern.iter().map(|v: &f64| v.abs()).sum::<f64>()));
        for v in pattern {
            pair(49, &format!("{v:.3}"));
        }
    }
    pair(0, "ENDTAB");
    pair(0, "TABLE");
    pair(2, "LAYER");
    pair(70, "6");
    for (name, colour, ltype) in [("VISIBLE", "7", "CONTINUOUS"), ("HIDDEN", "8", "HIDDEN"), ("CENTER", "1", "CENTER"), ("DIMENSIONS", "4", "CONTINUOUS"), ("FRAME", "7", "CONTINUOUS"), ("TEXT", "7", "CONTINUOUS")] {
        pair(0, "LAYER");
        pair(2, name);
        pair(70, "0");
        pair(62, colour);
        pair(6, ltype);
    }
    pair(0, "ENDTAB");
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for st in &d.strokes {
        let layer = match st.pen {
            Pen::Visible => "VISIBLE",
            Pen::Hidden => "HIDDEN",
            Pen::Thin => "DIMENSIONS",
            Pen::Center => "CENTER",
            Pen::Frame => "FRAME",
        };
        for w in st.points.windows(2) {
            pair(0, "LINE");
            pair(8, layer);
            pair(10, &format!("{:.4}", w[0][0]));
            pair(20, &format!("{:.4}", w[0][1]));
            pair(11, &format!("{:.4}", w[1][0]));
            pair(21, &format!("{:.4}", w[1][1]));
        }
    }
    for f in &d.fills {
        // Arrowheads: SOLID wants four corners (the last repeated for a triangle).
        // Rounder fills (leader dots) as a fan of them.
        for i in 1..f.len().saturating_sub(1) {
            pair(0, "SOLID");
            pair(8, "DIMENSIONS");
            for (k, p) in [f[0], f[i], f[i + 1], f[i + 1]].iter().enumerate() {
                pair(10 + k as i32, &format!("{:.4}", p[0]));
                pair(20 + k as i32, &format!("{:.4}", p[1]));
            }
        }
    }
    for t in &d.texts {
        pair(0, "TEXT");
        pair(8, "TEXT");
        pair(10, &format!("{:.4}", t.at[0]));
        pair(20, &format!("{:.4}", t.at[1]));
        pair(40, &format!("{:.3}", t.size));
        pair(1, &t.text.replace('·', "-").replace('−', "-").replace('⌀', "%%c").replace('°', "%%d").replace('±', "%%p"));
        if t.angle != 0.0 {
            pair(50, &format!("{:.3}", t.angle));
        }
        let h = match t.anchor {
            Anchor::Start => 0,
            Anchor::Middle => 1,
            Anchor::End => 2,
        };
        if h != 0 {
            pair(72, &h.to_string());
            pair(11, &format!("{:.4}", t.at[0]));
            pair(21, &format!("{:.4}", t.at[1]));
        }
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    o
}
