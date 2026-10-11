//! Engineering drawings: a sheet with the part's orthographic views (hidden
//! lines dashed), an isometric view, its overall dimensions and a title
//! block, laid out at a standard scale. The views come from the exact solid
//! (OpenCascade's hidden-line removal), so they are what the STEP file is.
//! The drawing is plain lines and text in sheet millimetres; it is written
//! as a vector PDF or a DXF here, and drawn as SVG by the app.

use crate::ShapeProgram;
use de_core::schemars::{self, JsonSchema};
use de_occt::{Arena, Shape, ViewLine};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

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

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
pub struct Options {
    #[serde(default)]
    pub sheet: Sheet,
    #[serde(default)]
    pub projection: Projection,
    /// Only this part of an assembly (all of it when absent).
    #[serde(default)]
    pub part: Option<String>,
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
    let values = program.values();
    let tolerance = program.display_tolerance();
    let (mut arena, exact) = crate::exact::evaluate_exact_in(Arena::new(), &program.name, &program.source, &values, tolerance, &program.inputs).map_err(|(_, e)| e)?;
    let parts: Vec<Shape> = exact.parts.iter().filter(|p| options.part.as_ref().is_none_or(|n| &p.name == n)).map(|p| p.shape).collect();
    if parts.is_empty() {
        return Err(match &options.part {
            Some(n) => format!("no part called “{n}”"),
            None => "the program made nothing to draw".into(),
        });
    }
    let shape = if parts.len() == 1 { parts[0] } else { arena.compound(&parts)? };
    layout(&mut arena, shape, options, title)
}

/// Lays out the views of `shape` on the sheet.
pub fn layout(arena: &mut Arena, shape: Shape, options: &Options, title: &Title) -> Result<Drawing, String> {
    let b = arena.bounds(shape)?;
    let size3 = [b[3] - b[0], b[4] - b[1], b[5] - b[2]];
    let deflection = (size3.iter().cloned().fold(0.0, f64::max) / 2000.0).clamp(0.005, 1.0);
    let view = |arena: &mut Arena, dir: [f64; 3], up: [f64; 3]| arena.hlr_view(shape, dir, up, deflection).map(View::of);
    let front = view(arena, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])?;
    let top = view(arena, [0.0, 0.0, -1.0], [0.0, 1.0, 0.0])?;
    let right = view(arena, [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0])?;
    let k = 1.0 / 3f64.sqrt();
    let iso = view(arena, [-k, k, -k], [0.0, 0.0, 1.0])?;

    let [w, h] = options.sheet.size();
    let mut d = Drawing { width: w, height: h, scale: 1.0, scale_text: String::new(), strokes: vec![], fills: vec![], texts: vec![] };
    // Frame: 20 mm binding margin on the left, 10 mm elsewhere (ISO 5457).
    let (fx0, fy0, fx1, fy1) = (20.0, 10.0, w - 10.0, h - 10.0);
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[fx0, fy0], [fx1, fy0], [fx1, fy1], [fx0, fy1], [fx0, fy0]] });
    let block = [180.0f64.min(fx1 - fx0), 32.0];

    // The views in a grid: front in the middle, the others where the projection puts them.
    let gap = 22.0; // sheet mm between views: room for a dimension
    let (fw, fh) = (front.size()[0], front.size()[1]);
    let (tw, th) = (top.size()[0], top.size()[1]);
    let (rw, rh) = (right.size()[0], right.size()[1]);
    let (iw, ih) = (iso.size()[0], iso.size()[1]);
    // The area above the title block, inside the frame, less a margin for dimensions.
    let area = [fx1 - fx0 - 2.0 * gap, fy1 - fy0 - block[1] - 2.0 * gap];
    // The main scale fits the three orthographic views; the isometric takes
    // whatever scale fits the room left beside them (and says so).
    let fits = |s: f64| (rw + fw.max(tw)) * s + gap <= area[0] - 40.0 && (fh.max(rh) + th) * s + gap <= area[1];
    let (scale, scale_text) = SCALES.iter().find(|(s, _)| fits(*s)).copied().unwrap_or(SCALES[SCALES.len() - 1]);
    d.scale = scale;
    d.scale_text = scale_text.to_string();
    let s = scale;
    // Columns: right view then front/top (first angle), or front/top then right view (third).
    let row_front = fh.max(rh) * s;
    let mid_w = fw.max(tw) * s;
    let main_w = rw * s + mid_w + gap;
    let total_h = row_front + th * s + gap;
    // The isometric goes where it comes out largest: the empty corner of the
    // grid (beside the top view, under or over the side view), or a column
    // right of the views. Its scale is the largest standard one that fits.
    let iso_fit = |w: f64, h: f64| SCALES.iter().find(|(k, _)| *k <= s && iw * k <= w && ih * k <= h - 8.0).copied();
    let corner = iso_fit(rw * s, th * s);
    let column = iso_fit(area[0] - main_w - gap, area[1]);
    let in_corner = match (corner, column) {
        (Some(a), Some(b)) => a.0 >= b.0,
        (a, _) => a.is_some(),
    };
    let x0 = if in_corner { fx0 + gap + (area[0] - main_w).max(0.0) / 2.0 } else { fx0 + gap };
    let y0 = fy0 + block[1] + gap + (area[1] - total_h).max(0.0) / 2.0;
    let (col_side, col_main) = match options.projection {
        Projection::First => (x0, x0 + rw * s + gap),
        Projection::Third => (x0 + mid_w + gap, x0),
    };
    let (row_top, row_main) = match options.projection {
        Projection::First => (y0, y0 + th * s + gap),
        Projection::Third => (y0 + row_front + gap, y0),
    };
    let (iso_cell, iso_scale) = if in_corner { ([col_side, row_top, col_side + rw * s, row_top + th * s], corner) } else { ([x0 + main_w + gap, fy0 + block[1] + gap, fx1 - gap, fy1 - gap], column) };
    // Where each view's lower left lands (sheet mm), the views centred in their cells.
    let place = |v: &View, cell: [f64; 2], cw: f64, ch: f64, k: f64| [cell[0] + (cw - v.size()[0] * k) / 2.0 - v.min[0] * k, cell[1] + (ch - v.size()[1] * k) / 2.0 - v.min[1] * k];
    let at_front = place(&front, [col_main, row_main], mid_w, row_front, s);
    let at_top = place(&top, [col_main, row_top], mid_w, th * s, s);
    let at_right = place(&right, [col_side, row_main], rw * s, row_front, s);
    for (v, at) in [(&front, at_front), (&top, at_top), (&right, at_right)] {
        for l in &v.lines {
            d.strokes.push(Stroke { pen: if l.hidden { Pen::Hidden } else { Pen::Visible }, points: l.points.iter().map(|p| [at[0] + p[0] * s, at[1] + p[1] * s]).collect() });
        }
    }
    holes(&mut d, arena, shape, deflection, s, [(at_front, 1), (at_top, 2), (at_right, 0)]);
    if let Some((k, k_text)) = iso_scale {
        let at = place(&iso, [iso_cell[0], iso_cell[1] + 8.0], iso_cell[2] - iso_cell[0], iso_cell[3] - iso_cell[1] - 8.0, k);
        // Hidden lines are left out of the isometric.
        for l in iso.lines.iter().filter(|l| !l.hidden) {
            d.strokes.push(Stroke { pen: Pen::Visible, points: l.points.iter().map(|p| [at[0] + p[0] * k, at[1] + p[1] * k]).collect() });
        }
        let text = if k == s { "ISOMETRIC".to_string() } else { format!("ISOMETRIC {k_text}") };
        d.texts.push(Text { at: [at[0] + (iso.min[0] + iso.max[0]) / 2.0 * k, at[1] + iso.min[1] * k - 6.0], size: 2.5, text, anchor: Anchor::Middle, angle: 0.0, bold: false });
    }
    // Overall dimensions: width over the front view, height beside it, depth beside the top view.
    let box_of = |v: &View, at: [f64; 2]| ([at[0] + v.min[0] * s, at[1] + v.min[1] * s], [at[0] + v.max[0] * s, at[1] + v.max[1] * s]);
    let (f0, f1) = box_of(&front, at_front);
    let (t0, t1) = box_of(&top, at_top);
    let above_front = match options.projection {
        Projection::First => true,  // the top view is below
        Projection::Third => false, // the top view is above: dimension below instead
    };
    if above_front {
        dim_h(&mut d, f0[0], f1[0], f1[1], f1[1] + 9.0, size3[0]);
    } else {
        dim_h(&mut d, f0[0], f1[0], f0[1], f0[1] - 9.0, size3[0]);
    }
    dim_v(&mut d, f0[1], f1[1], f1[0], f1[0] + 9.0, size3[2]);
    dim_v(&mut d, t0[1], t1[1], t1[0], t1[0] + 9.0, size3[1]);

    title_block(&mut d, [fx1 - block[0], fy0], block, title, options.projection);
    d.texts.push(Text {
        at: [fx1 - block[0], fy0 + block[1] + 3.0],
        size: 2.5,
        text: "Dimensions in mm · General tolerances ISO 2768-m · Do not scale the drawing".into(),
        anchor: Anchor::Start,
        angle: 0.0,
        bold: false,
    });
    Ok(d)
}

/// Holes and round bosses facing a view: centre marks in that view, and one
/// ⌀ callout per size (in the plan view if they show there).
/// `views` are each view's placement and the axis it looks along (0 x, 1 y, 2 z).
fn holes(d: &mut Drawing, arena: &mut Arena, shape: Shape, deflection: f64, s: f64, views: [([f64; 2], usize); 3]) {
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
            let text = if same.len() > 1 { format!("{}× ⌀{}", same.len(), length_text(2.0 * r)) } else { format!("⌀{}", length_text(2.0 * r)) };
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
                        let mut t = Text { at: [0.0, knee[1] + 1.0], size: 3.5, text: text.clone(), anchor: if right { Anchor::Start } else { Anchor::End }, angle: 0.0, bold: false };
                        let width = text_width(&t);
                        let end = [knee[0] + if right { width + 2.0 } else { -(width + 2.0) }, knee[1]];
                        t.at[0] = knee[0] + if right { 1.0 } else { -1.0 };
                        let bx = [knee[0].min(end[0]) - 0.5, knee[1] - 0.5, knee[0].max(end[0]) + 0.5, knee[1] + 5.0];
                        if bx[0] < 0.0 || bx[2] > d.width || bx[1] < 0.0 || bx[3] > d.height || d.texts.iter().any(|o| overlaps(bx, text_box(o))) {
                            continue;
                        }
                        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![tip, knee, end] });
                        arrow(d, tip, [-cs, -sn]);
                        d.texts.push(t);
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

/// A horizontal dimension from x0 to x1, measured at `from_y`, its line at `y`.
fn dim_h(d: &mut Drawing, x0: f64, x1: f64, from_y: f64, y: f64, value: f64) {
    let up = if y > from_y { 1.0 } else { -1.0 };
    for x in [x0, x1] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, from_y + up * 1.0], [x, y + up * 2.0]] });
    }
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x0, y], [x1, y]] });
    arrow(d, [x0, y], [-1.0, 0.0]);
    arrow(d, [x1, y], [1.0, 0.0]);
    d.texts.push(Text { at: [(x0 + x1) / 2.0, y + 1.2], size: 3.5, text: length_text(value), anchor: Anchor::Middle, angle: 0.0, bold: false });
}

/// A vertical dimension from y0 to y1, measured at `from_x`, its line at `x`.
fn dim_v(d: &mut Drawing, y0: f64, y1: f64, from_x: f64, x: f64, value: f64) {
    let right = if x > from_x { 1.0 } else { -1.0 };
    for y in [y0, y1] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[from_x + right * 1.0, y], [x + right * 2.0, y]] });
    }
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, y0], [x, y1]] });
    arrow(d, [x, y0], [0.0, -1.0]);
    arrow(d, [x, y1], [0.0, 1.0]);
    // Read from the right (ISO 129): the text runs up the page, left of the line.
    d.texts.push(Text { at: [x - 1.2, (y0 + y1) / 2.0], size: 3.5, text: length_text(value), anchor: Anchor::Middle, angle: 90.0, bold: false });
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
    let sheet = format!("{:?} · 1 of 1", sheet_name(d.width));
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
            '–' => 0x96,
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

/// Approximate width of Helvetica text (mm), for anchoring.
fn text_width(t: &Text) -> f64 {
    t.text.chars().map(|c| if c.is_ascii_uppercase() || c.is_ascii_digit() { 0.72 } else if c == ' ' || c == '.' || c == '·' { 0.3 } else { 0.6 }).sum::<f64>() * t.size * if t.bold { 1.05 } else { 1.0 }
}

/// The drawing as a one-page vector PDF.
pub fn pdf(d: &Drawing) -> Vec<u8> {
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
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>", d.width * PT, d.height * PT),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_string(),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content),
    ];
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
        if f.len() < 3 {
            continue;
        }
        pair(0, "SOLID");
        pair(8, "DIMENSIONS");
        for (k, p) in [f[0], f[1], f[2], f[2]].iter().enumerate() {
            pair(10 + k as i32, &format!("{:.4}", p[0]));
            pair(20 + k as i32, &format!("{:.4}", p[1]));
        }
    }
    for t in &d.texts {
        pair(0, "TEXT");
        pair(8, "TEXT");
        pair(10, &format!("{:.4}", t.at[0]));
        pair(20, &format!("{:.4}", t.at[1]));
        pair(40, &format!("{:.3}", t.size));
        pair(1, &t.text.replace('·', "-").replace('⌀', "%%c").replace('°', "%%d"));
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
