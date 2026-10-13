//! Board graphics: lines, arcs, circles, rectangles, polygons, Bézier curves
//! and ellipses, text (the built-in stroke font or a real font — Inter is
//! bundled — plain or knocked out of a filled box), images traced to art, and
//! dimensions (aligned, orthogonal, radial, leader, centre mark), each on any
//! layer: copper (a logo, a net tie, a heat spreader: real copper, checked
//! and filled around like any other), silkscreen, fabrication, mask, paste,
//! courtyard or the user and comment layers.
//!
//! Each graphic is its own entity (`pcb.graphic`) resolved into strokes (all
//! of the graphic's line width) and filled regions in board coordinates;
//! that is what the editor draws, what the Gerber, IPC-2581, ODB++ and KiCad
//! exports write, and what copper connectivity and clearance see. Text on a
//! bottom layer reads mirrored (correctly from below) unless told otherwise.

use crate::bitmap::{BitmapRequest, art};
use crate::geom::{self, P, Prim};
use crate::{Board, bad, board_of};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, bevy_ecs::{self, component::Component}, json, schemars};
use manifold_csg::{CrossSection, FillRule, JoinType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Non-copper layers a graphic can be on (copper layers are the board's: top, in1 …, bottom).
pub const LAYERS: [&str; 12] = ["silk_top", "silk_bottom", "fab_top", "fab_bottom", "mask_top", "mask_bottom", "paste_top", "paste_bottom", "courtyard_top", "courtyard_bottom", "user", "comments"];

/// A layer name as Forma calls it, from Forma's or KiCad's ("F.SilkS", "B.Fab", "Dwgs.User", "F.Cu", "In2.Cu").
pub fn normalize_layer(l: &str) -> String {
    let t = l.trim();
    let k = t.to_ascii_lowercase();
    let m = match k.as_str() {
        "f.silks" | "f.silkscreen" | "silk" | "silkscreen" | "top_silk" => "silk_top",
        "b.silks" | "b.silkscreen" | "bottom_silk" => "silk_bottom",
        "f.fab" | "fab" => "fab_top",
        "b.fab" => "fab_bottom",
        "f.mask" => "mask_top",
        "b.mask" => "mask_bottom",
        "f.paste" => "paste_top",
        "b.paste" => "paste_bottom",
        "f.crtyd" | "f.courtyard" => "courtyard_top",
        "b.crtyd" | "b.courtyard" => "courtyard_bottom",
        "dwgs.user" | "user.drawings" | "drawings" => "user",
        "cmts.user" | "user.comments" => "comments",
        "f.cu" => "top",
        "b.cu" => "bottom",
        _ => {
            if let Some(n) = k.strip_prefix("in").and_then(|r| r.strip_suffix(".cu")) {
                return format!("in{n}");
            }
            return k;
        }
    };
    m.to_string()
}

/// Whether a layer is seen from below (text on it reads mirrored).
pub fn is_bottom(layer: &str) -> bool {
    layer == "bottom" || layer.ends_with("_bottom")
}

/// The layer on the other side ("silk_top" ↔ "silk_bottom", "in1" ↔ "in(n−2)" …).
pub fn flip_layer(layer: &str, copper: &[&str]) -> String {
    if let Some(s) = layer.strip_suffix("_top") {
        return format!("{s}_bottom");
    }
    if let Some(s) = layer.strip_suffix("_bottom") {
        return format!("{s}_top");
    }
    if let Some(i) = copper.iter().position(|c| *c == layer) {
        return copper[copper.len() - 1 - i].to_string();
    }
    layer.to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DimStyle {
    /// Along the line between the two points.
    #[default]
    Aligned,
    /// Horizontal (or `vertical`) distance between the two points.
    Orthogonal,
    /// A circle's radius: points are the centre and a point on it.
    Radial,
    /// An arrow from the first point through the others to a text.
    Leader,
    /// A centre mark: points are the centre and a point on the circle.
    Center,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    #[default]
    Mm,
    Inch,
    Mil,
}

fn d_one() -> f64 {
    1.0
}
fn d_two() -> f64 {
    2.0
}
fn d_prec() -> u8 {
    2
}
fn d_margin() -> f64 {
    0.3
}
fn d_weight() -> f64 {
    600.0
}
fn d_end() -> f64 {
    360.0
}
fn d_mid() -> u8 {
    128
}

/// What a graphic is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// A polyline (2 or more points).
    Line { points: Vec<[f64; 2]> },
    /// An arc counter-clockwise from `start` to `end` degrees around `center`;
    /// or give `through` as [start point, a point on the arc, end point].
    Arc {
        #[serde(default)]
        center: [f64; 2],
        #[serde(default)]
        radius: f64,
        #[serde(default)]
        start: f64,
        #[serde(default)]
        end: f64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        through: Vec<[f64; 2]>,
    },
    Circle { center: [f64; 2], radius: f64 },
    /// Corner to corner; `corner` rounds them.
    Rect {
        from: [f64; 2],
        to: [f64; 2],
        #[serde(default)]
        corner: f64,
    },
    Polygon { points: Vec<[f64; 2]> },
    /// A cubic Bézier: start, two control points, end.
    Bezier { points: Vec<[f64; 2]> },
    /// An ellipse (or elliptic arc from `start` to `end` degrees), turned `rotation` degrees.
    Ellipse {
        center: [f64; 2],
        rx: f64,
        ry: f64,
        #[serde(default)]
        rotation: f64,
        #[serde(default)]
        start: f64,
        #[serde(default = "d_end")]
        end: f64,
    },
    /// Text: `size` is the capital height (mm). `font` "stroke" (default, the
    /// fab-safe single-line font, capitals) or "Inter" (bundled; any weight)
    /// or a stored font's hash. `knockout` cuts the letters out of a filled box
    /// `margin` larger (copper or silk logos that read on any colour).
    Text {
        text: String,
        at: [f64; 2],
        #[serde(default = "d_one")]
        size: f64,
        #[serde(default)]
        rotation: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        font: Option<String>,
        /// 100 to 900 for real fonts.
        #[serde(default = "d_weight")]
        weight: f64,
        #[serde(default)]
        align: Align,
        #[serde(default)]
        knockout: bool,
        #[serde(default = "d_margin")]
        margin: f64,
        /// Mirror the text; default: mirrored on bottom layers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mirror: Option<bool>,
    },
    /// A picture (grey pixels, rows top to bottom, base64) traced to art `size` mm wide, centred on `at`.
    Image {
        at: [f64; 2],
        size: f64,
        /// [width, height] in pixels.
        px: [u32; 2],
        pixels: String,
        #[serde(default = "d_mid")]
        threshold: u8,
        #[serde(default)]
        negative: bool,
        #[serde(default)]
        rotation: f64,
        #[serde(default)]
        mirror: bool,
    },
    /// A dimension. `points`: aligned/orthogonal [a, b]; radial and center [centre, point on circle];
    /// leader [arrow tip, …, text point]. `text` overrides the value ("<value>" stands for it).
    Dimension {
        #[serde(default)]
        style: DimStyle,
        points: Vec<[f64; 2]>,
        /// Distance of the dimension line from the measured points (mm).
        #[serde(default = "d_two")]
        offset: f64,
        /// Orthogonal: measure vertically.
        #[serde(default)]
        vertical: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default)]
        units: Units,
        #[serde(default = "d_prec")]
        precision: u8,
        #[serde(default = "d_one")]
        text_size: f64,
    },
}

/// A graphic on a board.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Graphic {
    pub board: StableId,
    pub layer: String,
    #[serde(flatten)]
    pub shape: Shape,
    /// Line width (mm); text strokes too.
    #[serde(default = "d_width")]
    pub width: f64,
    /// Closed shapes (circle, rectangle, polygon, ellipse) filled.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub filled: bool,
    /// Copper graphics may join a net.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

fn d_width() -> f64 {
    0.15
}

impl de_core::Persist for Graphic {
    const KIND: &'static str = "pcb.graphic";
}

/// A graphic resolved into board coordinates.
#[derive(Clone, Debug, Serialize)]
pub struct GraphicView {
    pub id: StableId,
    pub layer: String,
    pub kind: &'static str,
    pub net: Option<String>,
    /// Width of every stroke.
    pub width: f64,
    /// Polylines drawn `width` wide.
    pub strokes: Vec<Vec<P>>,
    /// Filled regions: islands of contours (outer counter-clockwise, holes clockwise).
    pub fills: Vec<Vec<Vec<P>>>,
    pub bbox: [f64; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A dimension's measured value (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}

const TOL: f64 = 0.01;

fn arc_pts(c: P, r: f64, a0: f64, a1: f64) -> Vec<P> {
    let sweep = a1 - a0;
    let n = ((sweep.abs() / 360.0) * geom::segments_for(r.max(0.01), TOL) as f64).ceil().max(2.0) as usize;
    (0..=n).map(|i| {
        let a = (a0 + sweep * i as f64 / n as f64).to_radians();
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    }).collect()
}

/// Centre, radius and angles (start < end, counter-clockwise) of the arc through three points.
pub fn arc_through(a: P, m: P, b: P) -> Option<(P, f64, f64, f64)> {
    let d = 2.0 * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
    if d.abs() < 1e-12 {
        return None;
    }
    let s = |p: P| p[0] * p[0] + p[1] * p[1];
    let c = [(s(a) * (m[1] - b[1]) + s(m) * (b[1] - a[1]) + s(b) * (a[1] - m[1])) / d, (s(a) * (b[0] - m[0]) + s(m) * (a[0] - b[0]) + s(b) * (m[0] - a[0])) / d];
    let r = geom::dist(c, a);
    let ang = |p: P| (p[1] - c[1]).atan2(p[0] - c[0]).to_degrees();
    let (aa, am, ab) = (ang(a), ang(m), ang(b));
    let norm = |x: f64| x.rem_euclid(360.0);
    // Counter-clockwise from a to b passing m, else the other way round.
    if norm(am - aa) <= norm(ab - aa) { Some((c, r, aa, aa + norm(ab - aa))) } else { Some((c, r, ab, ab + norm(aa - ab))) }
}

fn closed(mut p: Vec<P>) -> Vec<P> {
    if p.len() >= 2 && geom::dist(p[0], *p.last().unwrap()) > 1e-9 {
        p.push(p[0]);
    }
    p
}

fn section_of(loops: &[Vec<P>], rule: FillRule) -> CrossSection {
    let polys: Vec<Vec<P>> = loops.iter().filter(|l| l.len() >= 3).cloned().collect();
    if polys.is_empty() { CrossSection::empty() } else { CrossSection::from_polygons_with_fill_rule(&polys, rule) }
}

/// Islands of contours of a section.
pub fn islands(cs: &CrossSection) -> Vec<Vec<Vec<P>>> {
    cs.decompose().iter().map(geom::contours).filter(|c| !c.is_empty()).collect()
}

fn strokes_section(strokes: &[Vec<P>], width: f64) -> CrossSection {
    let mut parts = Vec::new();
    for s in strokes {
        if s.len() == 1 {
            parts.push(Prim::circle(s[0], width / 2.0).section(0.0, TOL));
        }
        for w in s.windows(2) {
            parts.push(Prim::capsule(w[0], w[1], width / 2.0).section(0.0, TOL));
        }
    }
    if parts.is_empty() { CrossSection::empty() } else { CrossSection::batch_union(&parts) }
}

/// Places local text geometry: mirror, turn, move.
fn placer(at: P, rotation: f64, mirror: bool) -> impl Fn(P) -> P {
    move |p: P| {
        let q = if mirror { [-p[0], p[1]] } else { p };
        geom::add(at, geom::rotate(q, rotation))
    }
}

/// Strokes or fills of text (local to its anchor, before placing).
fn text_local(text: &str, size: f64, font: Option<&str>, weight: f64, align: Align, width: f64) -> Result<(Vec<Vec<P>>, Option<CrossSection>), String> {
    if !(size > 0.0 && size <= 100.0) {
        return Err("text size is above 0 and up to 100 mm".into());
    }
    let stroke = font.is_none_or(|f| f.eq_ignore_ascii_case("stroke") || f.eq_ignore_ascii_case("default") || f.is_empty());
    if stroke {
        let lines: Vec<&str> = text.split('\n').collect();
        let step = size * 1.6;
        let mut out = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let w = crate::font::width(line, size);
            let dx = match align {
                Align::Left => w / 2.0,
                Align::Center => 0.0,
                Align::Right => -w / 2.0,
            };
            let y = (lines.len() as f64 - 1.0) * step / 2.0 - i as f64 * step;
            out.extend(crate::font::strokes(line, [dx, y], size, false));
        }
        let _ = width;
        return Ok((out, None));
    }
    let spec = de_shape::text::Spec {
        text: text.to_string(),
        size,
        font: font.map(|f| f.to_string()),
        weight,
        align: match align {
            Align::Left => de_shape::text::Align::Start,
            Align::Center => de_shape::text::Align::Center,
            Align::Right => de_shape::text::Align::End,
        },
        baseline: de_shape::text::Baseline::Middle,
        spacing: 0.0,
        line_height: 1.4,
        arc: None,
        tolerance: TOL,
    };
    let (loops, _) = de_shape::text::outlines(&spec)?;
    Ok((vec![], Some(section_of(&loops, FillRule::NonZero))))
}

fn fmt_value(v: f64, units: Units, precision: u8) -> String {
    let (x, unit) = match units {
        Units::Mm => (v, " mm"),
        Units::Inch => (v / 25.4, " in"),
        Units::Mil => (v / 0.0254, " mil"),
    };
    format!("{:.*}{unit}", precision.min(6) as usize, x)
}

fn arrow(tip: P, from: P, len: f64) -> Vec<Vec<P>> {
    let d = geom::sub(from, tip);
    let l = geom::len(d).max(1e-9);
    let u = [d[0] / l * len, d[1] / l * len];
    vec![vec![geom::add(tip, geom::rotate(u, 20.0)), tip, geom::add(tip, geom::rotate(u, -20.0))]]
}

/// Readable angle for text along a direction (never upside down).
fn readable(deg: f64) -> f64 {
    let a = deg.rem_euclid(360.0);
    if a > 90.0 + 1e-9 && a <= 270.0 + 1e-9 { a - 180.0 } else { a }
}

fn text_strokes(text: &str, at: P, size: f64, rotation: f64, mirror: bool) -> Vec<Vec<P>> {
    let f = placer(at, rotation, mirror);
    crate::font::strokes(text, [0.0, 0.0], size, false).into_iter().map(|s| s.into_iter().map(&f).collect()).collect()
}

/// Strokes, fills, measured value and text of a graphic.
pub fn resolve(id: StableId, g: &Graphic) -> Result<GraphicView, String> {
    let mut strokes: Vec<Vec<P>> = Vec::new();
    let mut fill: Option<CrossSection> = None;
    let mut value = None;
    let mut text_out = None;
    let w = g.width;
    let bottom = is_bottom(&g.layer);
    let kind: &'static str = match &g.shape {
        Shape::Line { .. } => "line",
        Shape::Arc { .. } => "arc",
        Shape::Circle { .. } => "circle",
        Shape::Rect { .. } => "rect",
        Shape::Polygon { .. } => "polygon",
        Shape::Bezier { .. } => "bezier",
        Shape::Ellipse { .. } => "ellipse",
        Shape::Text { .. } => "text",
        Shape::Image { .. } => "image",
        Shape::Dimension { .. } => "dimension",
    };
    let closed_shape = |pts: Vec<P>, strokes: &mut Vec<Vec<P>>, fill: &mut Option<CrossSection>| {
        if g.filled {
            let mut p = pts.clone();
            if geom::polygon_area(&p) < 0.0 {
                p.reverse();
            }
            *fill = Some(section_of(&[p], FillRule::NonZero));
        }
        strokes.push(closed(pts));
    };
    match &g.shape {
        Shape::Line { points } => {
            if points.len() < 2 {
                return Err("a line needs 2 or more points".into());
            }
            strokes.push(points.clone());
        }
        Shape::Arc { center, radius, start, end, through } => {
            let (c, r, a0, a1) = if through.len() == 3 {
                arc_through(through[0], through[1], through[2]).ok_or("the three arc points are on one line")?
            } else {
                let mut e = *end;
                while e <= *start {
                    e += 360.0;
                }
                (*center, *radius, *start, e)
            };
            if !(r > 0.0) {
                return Err("an arc needs a radius above 0".into());
            }
            strokes.push(arc_pts(c, r, a0, a1));
        }
        Shape::Circle { center, radius } => {
            if !(*radius > 0.0) {
                return Err("a circle needs a radius above 0".into());
            }
            let mut p = arc_pts(*center, *radius, 0.0, 360.0);
            p.pop();
            closed_shape(p, &mut strokes, &mut fill);
        }
        Shape::Rect { from, to, corner } => {
            let (x0, y0, x1, y1) = (from[0].min(to[0]), from[1].min(to[1]), from[0].max(to[0]), from[1].max(to[1]));
            if x1 - x0 < 1e-6 || y1 - y0 < 1e-6 {
                return Err("a rectangle needs width and height".into());
            }
            let p = geom::round_corners(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], *corner, TOL);
            closed_shape(p, &mut strokes, &mut fill);
        }
        Shape::Polygon { points } => {
            if points.len() < 3 {
                return Err("a polygon needs 3 or more points".into());
            }
            closed_shape(points.clone(), &mut strokes, &mut fill);
        }
        Shape::Bezier { points } => {
            if points.len() != 4 {
                return Err("a Bézier curve has 4 points: start, two controls, end".into());
            }
            let [a, b, c, d] = [points[0], points[1], points[2], points[3]];
            let n = 48;
            strokes.push((0..=n).map(|i| {
                let t = i as f64 / n as f64;
                let u = 1.0 - t;
                let k = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                [k[0] * a[0] + k[1] * b[0] + k[2] * c[0] + k[3] * d[0], k[0] * a[1] + k[1] * b[1] + k[2] * c[1] + k[3] * d[1]]
            }).collect());
        }
        Shape::Ellipse { center, rx, ry, rotation, start, end } => {
            if !(*rx > 0.0 && *ry > 0.0) {
                return Err("an ellipse needs both radii above 0".into());
            }
            let mut e = *end;
            while e <= *start {
                e += 360.0;
            }
            let full = e - start >= 360.0 - 1e-9;
            let n = 96;
            let mut pts: Vec<P> = (0..=n).map(|i| {
                let a = (start + (e - start) * i as f64 / n as f64).to_radians();
                geom::add(*center, geom::rotate([rx * a.cos(), ry * a.sin()], *rotation))
            }).collect();
            if full {
                pts.pop();
                closed_shape(pts, &mut strokes, &mut fill);
            } else {
                strokes.push(pts);
            }
        }
        Shape::Text { text, at, size, rotation, font, weight, align, knockout, margin, mirror } => {
            if text.trim().is_empty() {
                return Err("text needs some text".into());
            }
            text_out = Some(text.clone());
            let m = mirror.unwrap_or(bottom);
            let (local_strokes, local_fill) = text_local(text, *size, font.as_deref(), *weight, *align, w)?;
            let f = placer(*at, *rotation, m);
            let place_cs = |c: &CrossSection| {
                let c = if m { c.mirror(1.0, 0.0) } else { c.clone() };
                c.rotate(*rotation).translate(at[0], at[1])
            };
            if *knockout {
                // The box around the ink in the text's own frame, less the ink, then placed.
                let ink = match &local_fill {
                    Some(c) => c.clone(),
                    None => strokes_section(&local_strokes, w),
                };
                let b = ink.bounds_rect2();
                let mg = margin.max(0.0);
                let boxed = vec![[b.min_x - mg, b.min_y - mg], [b.max_x + mg, b.min_y - mg], [b.max_x + mg, b.max_y + mg], [b.min_x - mg, b.max_y + mg]];
                fill = Some(place_cs(&section_of(&[boxed], FillRule::NonZero).difference(&ink)));
            } else if let Some(c) = &local_fill {
                fill = Some(place_cs(c));
            } else {
                strokes = local_strokes.iter().map(|s| s.iter().map(|p| f(*p)).collect()).collect();
            }
        }
        Shape::Image { at, size, px, pixels, threshold, negative, rotation, mirror } => {
            let req = BitmapRequest { width: px[0], height: px[1], pixels: pixels.clone(), threshold: *threshold, negative: *negative, size: *size, layer: "silk".into(), name: None, line: None };
            let (outers, holes, _) = art(&req)?;
            let loops: Vec<Vec<P>> = outers.into_iter().chain(holes).collect();
            let mut c = section_of(&loops, FillRule::EvenOdd);
            if *mirror {
                c = c.mirror(1.0, 0.0);
            }
            fill = Some(c.rotate(*rotation).translate(at[0], at[1]));
        }
        Shape::Dimension { style, points, offset, vertical, text, units, precision, text_size } => {
            let need = if *style == DimStyle::Leader { 2 } else { 2 };
            if points.len() < need {
                return Err("a dimension needs 2 points (a leader 2 or more)".into());
            }
            let (a, b) = (points[0], points[1]);
            let al = text_size * 0.8;
            let label = |v: f64| -> String {
                let s = fmt_value(v, *units, *precision);
                match text {
                    Some(t) if t.contains("<value>") => t.replace("<value>", &s),
                    Some(t) => t.clone(),
                    None => s,
                }
            };
            match style {
                DimStyle::Aligned | DimStyle::Orthogonal => {
                    // Measured points a', b' and the direction of measurement.
                    let (pa, pb, dir) = if *style == DimStyle::Aligned {
                        let d = geom::sub(b, a);
                        let l = geom::len(d);
                        if l < 1e-9 {
                            return Err("the dimension's two points are the same".into());
                        }
                        (a, b, [d[0] / l, d[1] / l])
                    } else if *vertical {
                        (a, [a[0], b[1]], [0.0, if b[1] >= a[1] { 1.0 } else { -1.0 }])
                    } else {
                        (a, [b[0], a[1]], [if b[0] >= a[0] { 1.0 } else { -1.0 }, 0.0])
                    };
                    let v = geom::dist(pa, pb);
                    value = Some(v);
                    let n = [-dir[1], dir[0]];
                    let sgn = if *offset >= 0.0 { 1.0 } else { -1.0 };
                    let off = |p: P, d: f64| [p[0] + n[0] * d, p[1] + n[1] * d];
                    // For orthogonal, the extension from b goes from the real point b.
                    let (la, lb) = (off(pa, *offset), off(pb, *offset));
                    let ext = 0.5 * sgn;
                    strokes.push(vec![off(a, 0.2 * sgn), off(la, ext)]);
                    let b_base = if *style == DimStyle::Aligned { b } else { b };
                    let lb_from = b_base;
                    let d_b = geom::dot(geom::sub(lb, lb_from), n);
                    strokes.push(vec![off(lb_from, 0.2 * sgn), off(lb_from, d_b + ext)]);
                    strokes.push(vec![la, lb]);
                    strokes.extend(arrow(la, lb, al));
                    strokes.extend(arrow(lb, la, al));
                    let s = label(v);
                    let mid = [(la[0] + lb[0]) / 2.0, (la[1] + lb[1]) / 2.0];
                    let at = off(mid, sgn * (text_size * 0.5 + w + 0.3));
                    let ang = readable(dir[1].atan2(dir[0]).to_degrees());
                    strokes.extend(text_strokes(&s, at, *text_size, ang, bottom));
                    text_out = Some(s);
                }
                DimStyle::Radial => {
                    let r = geom::dist(a, b);
                    if r < 1e-9 {
                        return Err("a radial dimension needs the centre and a point on the circle".into());
                    }
                    value = Some(r);
                    let d = [(b[0] - a[0]) / r, (b[1] - a[1]) / r];
                    let knee = [b[0] + d[0] * offset.abs(), b[1] + d[1] * offset.abs()];
                    let s = format!("R {}", label(r));
                    let tw = crate::font::width(&s, *text_size);
                    let dirx = if d[0] >= 0.0 { 1.0 } else { -1.0 };
                    let shelf = [knee[0] + dirx * (tw + 0.6), knee[1]];
                    strokes.push(vec![b, knee, shelf]);
                    strokes.extend(arrow(b, knee, al));
                    strokes.extend(text_strokes(&s, [(knee[0] + shelf[0]) / 2.0, knee[1] + text_size * 0.5 + w + 0.3], *text_size, 0.0, bottom));
                    text_out = Some(s);
                }
                DimStyle::Leader => {
                    let s = text.clone().unwrap_or_default();
                    let last = *points.last().unwrap();
                    let prev = points[points.len() - 2];
                    let dirx = if last[0] >= prev[0] { 1.0 } else { -1.0 };
                    let tw = crate::font::width(&s, *text_size);
                    let mut line = points.clone();
                    if !s.is_empty() {
                        line.push([last[0] + dirx * (tw + 0.6), last[1]]);
                        strokes.extend(text_strokes(&s, [last[0] + dirx * (tw / 2.0 + 0.3), last[1] + text_size * 0.5 + w + 0.3], *text_size, 0.0, bottom));
                    }
                    strokes.extend(arrow(points[0], points[1], al));
                    strokes.push(line);
                    text_out = Some(s);
                }
                DimStyle::Center => {
                    let r = geom::dist(a, b);
                    value = Some(r);
                    strokes.push(vec![[a[0] - r, a[1]], [a[0] + r, a[1]]]);
                    strokes.push(vec![[a[0], a[1] - r], [a[0], a[1] + r]]);
                }
            }
        }
    }
    let fills = fill.map(|c| islands(&c)).unwrap_or_default();
    let mut bbox = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in strokes.iter().flatten() {
        bbox = [bbox[0].min(p[0] - w / 2.0), bbox[1].min(p[1] - w / 2.0), bbox[2].max(p[0] + w / 2.0), bbox[3].max(p[1] + w / 2.0)];
    }
    for p in fills.iter().flatten().flatten() {
        bbox = [bbox[0].min(p[0]), bbox[1].min(p[1]), bbox[2].max(p[0]), bbox[3].max(p[1])];
    }
    if !bbox[0].is_finite() {
        return Err("the graphic draws nothing".into());
    }
    Ok(GraphicView { id, layer: g.layer.clone(), kind, net: g.net.clone(), width: w, strokes, fills, bbox, text: text_out, value, locked: g.locked })
}

/// Copper pieces of a graphic for connectivity and clearance: stroke segments and fill triangles.
pub fn copper_prims(v: &GraphicView) -> Vec<Prim> {
    let mut out = Vec::new();
    for s in &v.strokes {
        if s.len() == 1 {
            out.push(Prim::circle(s[0], v.width / 2.0));
        }
        for w in s.windows(2) {
            out.push(Prim::capsule(w[0], w[1], v.width / 2.0));
        }
    }
    for island in &v.fills {
        out.extend(triangles(island));
    }
    out
}

/// Convex pieces (triangles) exactly covering an island of contours.
pub fn triangles(island: &[Vec<P>]) -> Vec<Prim> {
    let flat: Vec<P> = island.iter().flatten().copied().collect();
    match manifold_csg::triangulate_polygons(island, 1e-9) {
        Some(tris) => tris.iter().map(|t| Prim { pts: vec![flat[t[0] as usize], flat[t[1] as usize], flat[t[2] as usize]], r: 0.0 }).collect(),
        None => island.first().map(|o| vec![Prim { pts: geom::convex_hull(o), r: 0.0 }]).unwrap_or_default(),
    }
}

/// The graphic as a cross-section (strokes swept, fills), grown by `grow`.
pub fn section(v: &GraphicView, grow: f64) -> CrossSection {
    let mut parts = vec![strokes_section(&v.strokes, v.width + 2.0 * grow)];
    for island in &v.fills {
        let c = section_of(island, FillRule::EvenOdd);
        parts.push(if grow > 0.0 { c.offset(grow, JoinType::Round, 2.0, geom::segments_for(grow, TOL) as i32) } else { c });
    }
    CrossSection::batch_union(&parts)
}

impl Graphic {
    /// Every point mapped by `f` (a move, turn or mirror); `rot` degrees are
    /// added to angles; `mirror` mirrors in x (before turning).
    pub fn map(&mut self, f: &dyn Fn(P) -> P, rot: f64, mirror: bool) {
        let m = |v: &mut Vec<[f64; 2]>| v.iter_mut().for_each(|p| *p = f(*p));
        match &mut self.shape {
            Shape::Line { points } | Shape::Polygon { points } | Shape::Bezier { points } => m(points),
            Shape::Dimension { points, offset, .. } => {
                m(points);
                if mirror {
                    *offset = -*offset;
                }
            }
            Shape::Arc { center, start, end, through, .. } => {
                *center = f(*center);
                m(through);
                if mirror {
                    let (s, e) = (180.0 - *end, 180.0 - *start);
                    *start = s;
                    *end = e;
                }
                *start += rot;
                *end += rot;
            }
            Shape::Circle { center, .. } => *center = f(*center),
            Shape::Rect { from, to, corner } => {
                if (rot.rem_euclid(90.0)).abs() < 1e-9 || (rot.rem_euclid(90.0) - 90.0).abs() < 1e-9 {
                    *from = f(*from);
                    *to = f(*to);
                } else {
                    let (a, b) = (*from, *to);
                    let mut points: Vec<P> = geom::round_corners(&[[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]], *corner, TOL);
                    m(&mut points);
                    self.shape = Shape::Polygon { points };
                }
            }
            Shape::Ellipse { center, rotation, start, end, .. } => {
                *center = f(*center);
                if mirror {
                    *rotation = -*rotation;
                    let (s, e) = (180.0 - *end, 180.0 - *start);
                    *start = s;
                    *end = e;
                }
                *rotation += rot;
            }
            Shape::Text { at, rotation, mirror: mm, .. } => {
                *at = f(*at);
                if mirror {
                    *rotation = -*rotation;
                    *mm = Some(!mm.unwrap_or(is_bottom(&self.layer)));
                }
                *rotation += rot;
            }
            Shape::Image { at, rotation, mirror: mm, .. } => {
                *at = f(*at);
                if mirror {
                    *rotation = -*rotation;
                    *mm = !*mm;
                }
                *rotation += rot;
            }
        }
    }

    /// A point that stands for the graphic (its anchor).
    pub fn anchor(&self) -> P {
        match &self.shape {
            Shape::Line { points } | Shape::Polygon { points } | Shape::Bezier { points } | Shape::Dimension { points, .. } => points.first().copied().unwrap_or_default(),
            Shape::Arc { center, through, .. } => through.first().copied().unwrap_or(*center),
            Shape::Circle { center, .. } | Shape::Ellipse { center, .. } => *center,
            Shape::Rect { from, to, .. } => [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0],
            Shape::Text { at, .. } | Shape::Image { at, .. } => *at,
        }
    }
}

pub(crate) fn check_graphic(board: &Board, g: &mut Graphic) -> Result<GraphicView, CommandError> {
    g.layer = normalize_layer(&g.layer);
    let copper = board.copper_layers();
    if !copper.contains(&g.layer.as_str()) && !LAYERS.contains(&g.layer.as_str()) {
        return Err(bad(format!("layer “{}” is not on this board (use {}, or {})", g.layer, copper.join(", "), LAYERS.join(", "))));
    }
    if !(g.width.is_finite() && g.width > 0.0 && g.width <= 20.0) {
        return Err(bad("a line width is above 0 and up to 20 mm"));
    }
    if g.net.is_some() && !copper.contains(&g.layer.as_str()) {
        g.net = None;
    }
    let mut pts: Vec<P> = vec![g.anchor()];
    if let Shape::Line { points } | Shape::Polygon { points } = &g.shape {
        pts.extend(points.iter().copied());
    }
    crate::check_points(&pts, "graphic")?;
    resolve(StableId::default(), g).map_err(bad)
}

/// `pcb.graphic.create`: a graphic on a board layer. `kind` is line, arc,
/// circle, rect, polygon, bezier, ellipse, text, image or dimension, with
/// that shape's fields (see the schema).
#[derive(Deserialize, JsonSchema)]
pub struct CreateGraphic {
    pub board: StableId,
    /// A copper layer, or silk_top, silk_bottom, fab_top, fab_bottom, mask_top, mask_bottom, paste_top, paste_bottom, courtyard_top, courtyard_bottom, user, comments (KiCad names work too).
    pub layer: String,
    #[serde(flatten)]
    pub shape: Shape,
    pub width: Option<f64>,
    #[serde(default)]
    pub filled: bool,
    pub net: Option<String>,
    #[serde(default)]
    pub locked: bool,
}

impl Command for CreateGraphic {
    const NAME: &'static str = "pcb.graphic.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        let silk = self.layer.contains("silk") || self.layer.to_ascii_lowercase().contains("silks");
        let mut g = Graphic { board: self.board, layer: self.layer, shape: self.shape, width: self.width.unwrap_or(if silk { board.rules.silk_width } else { 0.15 }), filled: self.filled, net: crate::clean(self.net), locked: self.locked };
        let v = check_graphic(&board, &mut g)?;
        let id = tx.create();
        tx.set(id, g)?;
        tx.add_dependency(self.board, id)?;
        Ok(json!({ "id": id, "layer": v.layer, "bbox": v.bbox, "value": v.value, "text": v.text }))
    }
}

/// `pcb.graphic.update`: change a graphic: its layer, width, fill, net,
/// lock, or its whole `shape` (an object with `kind` and the shape's fields).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateGraphic {
    pub target: StableId,
    pub layer: Option<String>,
    pub width: Option<f64>,
    pub filled: Option<bool>,
    /// "" for none.
    pub net: Option<String>,
    pub locked: Option<bool>,
    pub shape: Option<Shape>,
    /// Text graphics: new text.
    pub text: Option<String>,
}

impl Command for UpdateGraphic {
    const NAME: &'static str = "pcb.graphic.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut g = tx.get::<Graphic>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a graphic", self.target)))?;
        let board = board_of(tx, g.board)?;
        if let Some(v) = self.layer {
            g.layer = v;
        }
        if let Some(v) = self.width {
            g.width = v;
        }
        if let Some(v) = self.filled {
            g.filled = v;
        }
        if self.net.is_some() {
            g.net = crate::clean(self.net);
        }
        if let Some(v) = self.locked {
            g.locked = v;
        }
        if let Some(s) = self.shape {
            g.shape = s;
        }
        if let Some(t) = self.text {
            match &mut g.shape {
                Shape::Text { text, .. } => *text = t,
                Shape::Dimension { text, .. } => *text = Some(t).filter(|s| !s.is_empty()),
                _ => return Err(bad("only text and dimensions have text")),
            }
        }
        check_graphic(&board, &mut g)?;
        tx.set(self.target, g)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.graphics.edit`: global edit of text and graphics — every graphic on
/// a board matching `layer` and `kind` (both optional) gets the new line
/// width, text size, font or layer.
#[derive(Deserialize, JsonSchema)]
pub struct EditGraphics {
    pub board: StableId,
    pub layer: Option<String>,
    pub kind: Option<String>,
    pub width: Option<f64>,
    pub text_size: Option<f64>,
    pub font: Option<String>,
    pub to_layer: Option<String>,
}

impl Command for EditGraphics {
    const NAME: &'static str = "pcb.graphics.edit";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        let layer = self.layer.as_deref().map(normalize_layer);
        let mut n = 0;
        for (id, mut g) in crate::items(tx.world(), self.board).graphics {
            if g.locked || layer.as_ref().is_some_and(|l| *l != g.layer) {
                continue;
            }
            let k = resolve(id, &g).map(|v| v.kind).unwrap_or("");
            if self.kind.as_ref().is_some_and(|want| want != k) {
                continue;
            }
            if let Some(w) = self.width {
                g.width = w;
            }
            match &mut g.shape {
                Shape::Text { size, font, .. } => {
                    if let Some(s) = self.text_size {
                        *size = s;
                    }
                    if let Some(f) = &self.font {
                        *font = Some(f.clone()).filter(|f| !f.eq_ignore_ascii_case("stroke"));
                    }
                }
                Shape::Dimension { text_size, .. } => {
                    if let Some(s) = self.text_size {
                        *text_size = s;
                    }
                }
                _ => {}
            }
            if let Some(l) = &self.to_layer {
                g.layer = l.clone();
            }
            check_graphic(&board, &mut g)?;
            tx.set(id, g)?;
            n += 1;
        }
        Ok(json!({ "board": self.board, "changed": n }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.component::<Graphic>().command::<CreateGraphic>().command::<UpdateGraphic>().command::<EditGraphics>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(layer: &str, shape: Shape) -> Graphic {
        Graphic { board: StableId::default(), layer: layer.into(), shape, width: 0.15, filled: false, net: None, locked: false }
    }

    #[test]
    fn arc_through_three_points() {
        let (c, r, a0, a1) = arc_through([1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]).unwrap();
        assert!(geom::dist(c, [0.0, 0.0]) < 1e-9 && (r - 1.0).abs() < 1e-9);
        assert!((a0 - 0.0).abs() < 1e-9 && (a1 - 180.0).abs() < 1e-9);
    }

    #[test]
    fn aligned_dimension_measures_and_labels() {
        let v = resolve(StableId::default(), &g("fab_top", Shape::Dimension { style: DimStyle::Aligned, points: vec![[0.0, 0.0], [30.0, 40.0]], offset: 3.0, vertical: false, text: None, units: Units::Mm, precision: 2, text_size: 1.0 })).unwrap();
        assert!((v.value.unwrap() - 50.0).abs() < 1e-9);
        assert_eq!(v.text.as_deref(), Some("50.00 mm"));
        let o = resolve(StableId::default(), &g("fab_top", Shape::Dimension { style: DimStyle::Orthogonal, points: vec![[0.0, 0.0], [30.0, 40.0]], offset: 3.0, vertical: true, text: Some("H <value>".into()), units: Units::Mil, precision: 0, text_size: 1.0 })).unwrap();
        assert!((o.value.unwrap() - 40.0).abs() < 1e-9);
        assert_eq!(o.text.as_deref(), Some("H 1575 mil"));
    }

    #[test]
    fn knockout_text_leaves_holes_in_a_box() {
        let shape = |knockout| Shape::Text { text: "FORMA".into(), at: [10.0, 5.0], size: 2.0, rotation: 0.0, font: None, weight: 600.0, align: Align::Center, knockout, margin: 0.5, mirror: None };
        let plain = resolve(StableId::default(), &g("silk_top", shape(false))).unwrap();
        assert!(plain.fills.is_empty() && !plain.strokes.is_empty());
        let ko = resolve(StableId::default(), &g("silk_top", shape(true))).unwrap();
        assert!(ko.strokes.is_empty() && !ko.fills.is_empty());
        assert!(ko.fills.iter().any(|i| i.len() > 1), "letters cut out of the box");
        let inter = resolve(StableId::default(), &g("top", Shape::Text { text: "Hi".into(), at: [0.0, 0.0], size: 2.0, rotation: 90.0, font: Some("Inter".into()), weight: 700.0, align: Align::Left, knockout: false, margin: 0.3, mirror: None })).unwrap();
        assert!(!inter.fills.is_empty());
        assert!(!copper_prims(&inter).is_empty());
    }

    #[test]
    fn bottom_text_reads_mirrored() {
        let t = |layer: &str| resolve(StableId::default(), &g(layer, Shape::Text { text: "L".into(), at: [0.0, 0.0], size: 1.0, rotation: 0.0, font: None, weight: 400.0, align: Align::Center, knockout: false, margin: 0.3, mirror: None })).unwrap();
        // "L": its foot runs right on top, left (mirrored) on the bottom.
        let foot = |v: &GraphicView| v.strokes[0].last().unwrap()[0] - v.strokes[0][1][0];
        assert!(foot(&t("silk_top")) > 0.0);
        assert!(foot(&t("silk_bottom")) < 0.0);
    }

    #[test]
    fn layers_by_kicad_name() {
        assert_eq!(normalize_layer("F.SilkS"), "silk_top");
        assert_eq!(normalize_layer("In3.Cu"), "in3");
        assert_eq!(flip_layer("in1", &["top", "in1", "in2", "bottom"]), "in2");
        assert_eq!(flip_layer("fab_top", &[]), "fab_bottom");
    }
}
