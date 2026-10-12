//! Drafting in model space: the 2D-and-3D geometry a designer lays out
//! around the parts — points, lines, wires, rectangles, polygons, circles,
//! arcs, ellipses and B-splines anywhere in 3D; text, leader labels and
//! dimensions that measure the model and follow it; hatches on closed
//! outlines; named working planes to draw on; layers that group anything
//! for visibility and colour; arrays of parts (linear, polar, along a path)
//! whose spacing links like any parameter; and conversion of planar draft
//! geometry into a sketch, to extrude or constrain.
//!
//! Draft objects are not features of a part and nothing hangs off them
//! implicitly: a part uses one only through an explicit link (a path array
//! names its path, a sketch made from them is a copy).

pub mod array;
pub mod geom;

use de_assembly::{Instance, transform::Transform};
use de_core::{
    Command, CommandError, Derived, ErrorCode, IdIndex, Persist, Plugin, Registry, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use geom::{BSpline, Frame, V3, add, cross, dist, dot, mul, norm, sub, unit};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub struct DraftPlugin;

impl Plugin for DraftPlugin {
    fn name(&self) -> &'static str {
        "draft"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<DraftObject>()
            .component::<Layer>()
            .component::<WorkingPlane>()
            .component::<array::Array>()
            .command::<CreateDraft>()
            .command::<UpdateDraft>()
            .command::<ToSketch>()
            .command::<CreateLayer>()
            .command::<UpdateLayer>()
            .command::<CreatePlane>()
            .command::<UpdatePlane>()
            .command::<array::CreateArray>()
            .command::<array::UpdateArray>()
            .command::<array::DeleteArray>()
            .derived(Derived { name: "draft.shape", inputs: &[DraftObject::KIND], reads_dependencies: true, compute: draft_shape })
            // After assembly.placed: copies in an array are placed by it.
            .derived(Derived { name: "draft.array_placed", inputs: &[Instance::KIND], reads_dependencies: true, compute: array::member_placed });
    }
}

// ---- Data ------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    Point,
    Line,
    Wire,
    Rectangle,
    Polygon,
    Circle,
    Arc,
    Ellipse,
    Bspline,
    Text,
    Label,
    Dimension,
}

impl DraftKind {
    pub fn word(self) -> &'static str {
        match self {
            DraftKind::Point => "Point",
            DraftKind::Line => "Line",
            DraftKind::Wire => "Wire",
            DraftKind::Rectangle => "Rectangle",
            DraftKind::Polygon => "Polygon",
            DraftKind::Circle => "Circle",
            DraftKind::Arc => "Arc",
            DraftKind::Ellipse => "Ellipse",
            DraftKind::Bspline => "B-spline",
            DraftKind::Text => "Text",
            DraftKind::Label => "Label",
            DraftKind::Dimension => "Dimension",
        }
    }
}

/// What a dimension measures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    /// Straight-line distance between its two points.
    #[default]
    Aligned,
    X,
    Y,
    Z,
    /// Angle at the first point between the other two (degrees).
    Angle,
    /// From a centre (first point) to a point on the curve.
    Radius,
    Diameter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hatch {
    /// "lines" or "cross".
    #[serde(default = "lines")]
    pub pattern: String,
    /// Degrees from the outline's plane x direction.
    #[serde(default = "deg45")]
    pub angle: f64,
    /// mm between lines.
    #[serde(default = "three")]
    pub spacing: f64,
}

fn lines() -> String {
    "lines".into()
}
fn deg45() -> f64 {
    45.0
}
fn three() -> f64 {
    3.0
}

/// A piece of draft geometry or annotation in model space (mm).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DraftObject {
    pub name: String,
    pub kind: DraftKind,
    /// Defining points: a point's position, a line's ends, a wire's or
    /// B-spline's points, text's place, a label's text place then the point
    /// it points at, a dimension's ends (angle: vertex, then the two sides).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<V3>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<V3>,
    /// Normal of a circle's, arc's, ellipse's or polygon's plane (default +Z).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<V3>,
    /// The direction angles are measured from (and an ellipse's major axis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_dir: Option<V3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// An ellipse's minor radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius2: Option<f64>,
    /// An arc's start and end angles (degrees, counter-clockwise about the normal from x_dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sides: Option<u32>,
    /// A B-spline through its points (true) or shaped by them as control points.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degree: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Text height (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<Measure>,
    /// Where a dimension line sits: this far from the points it measures (mm, in model space).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<V3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hatch: Option<Hatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The whole object is in this part's own coordinates and moves with it
    /// (a top-level instance or a placed shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<StableId>,
    /// Per point: the part that point is on (overrides `on`); a dimension
    /// between two parts follows both.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Option<StableId>>,
}

impl Persist for DraftObject {
    const KIND: &'static str = "draft.object";
}

/// A layer: a named set of anything (parts, instances, draft objects,
/// sketches) shown, hidden, coloured or locked together. Membership is
/// explicit; something is in at most one layer.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Layer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default)]
    pub members: Vec<StableId>,
}

fn yes() -> bool {
    true
}

impl Persist for Layer {
    const KIND: &'static str = "draft.layer";
}

/// A named plane to draw and sketch on: origin, x and y (unit, square) —
/// the same as a sketch's `plane`, so it can be passed straight to `sketch.create`.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkingPlane {
    pub name: String,
    pub origin: V3,
    pub x: V3,
    pub y: V3,
}

impl Persist for WorkingPlane {
    const KIND: &'static str = "draft.plane";
}

impl WorkingPlane {
    pub fn frame(&self) -> Frame {
        Frame { origin: self.origin, x: self.x, y: self.y }
    }
}

// ---- Resolved geometry --------------------------------------------------------------------------

/// Derived: a draft object where it is now, ready to draw and measure.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct DraftShape {
    /// The defining points in project coordinates.
    pub points: Vec<V3>,
    /// Polylines to draw.
    pub lines: Vec<Vec<V3>>,
    pub closed: bool,
    /// Total length of its lines (mm).
    pub length: f64,
    /// Area inside, for a closed planar outline (mm²).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    /// Its plane, when it has one: origin, x, y.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plane: Option<[V3; 3]>,
    /// Hatch line segments.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hatch: Vec<[V3; 2]>,
    /// Where text goes, and what it says (text, labels, dimensions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<(V3, String)>,
    /// A dimension's value (mm, or degrees for an angle).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Arrowheads: (tip, direction it points).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub arrows: Vec<(V3, V3)>,
}

/// Where a part's own coordinates sit in the project: a top-level instance
/// (or one inside an assembly placed exactly once), or a placed shape.
pub fn frame_of(world: &World, id: StableId) -> Option<Transform> {
    let e = world.resource::<IdIndex>().get(id)?;
    if let Some(inst) = world.get::<Instance>(e) {
        let mut t = de_assembly::placement(world, e)?;
        let mut parent = inst.parent;
        let mut guard = 0;
        while let Some(def) = parent {
            guard += 1;
            if guard > 32 {
                break;
            }
            let holders: Vec<(StableId, Entity)> = world.resource::<IdIndex>().iter().filter(|(_, x)| world.get::<Instance>(*x).is_some_and(|i| i.of == def)).collect();
            let [(_, he)] = holders.as_slice() else { break };
            let outer = de_assembly::placement(world, *he)?;
            t = t.within(&outer);
            parent = world.get::<Instance>(*he).and_then(|i| i.parent);
        }
        return Some(t);
    }
    if world.get::<de_shape::ShapeProgram>(e).is_some()
        && let Some(p) = world.get::<de_solid::Placement>(e)
    {
        use de_core::LengthUnit::Mm;
        return Some(Transform::translation([p.x.to(Mm), p.y.to(Mm), p.z.to(Mm)]));
    }
    None
}

/// A draft object's geometry in project coordinates (anchors applied).
#[derive(Clone, Debug)]
pub struct Placed {
    pub points: Vec<V3>,
    pub center: Option<V3>,
    pub normal: V3,
    pub x_dir: Option<V3>,
}

pub fn placed(world: &World, o: &DraftObject) -> Result<Placed, String> {
    let whole = match o.on {
        Some(id) => Some(frame_of(world, id).ok_or("the part it is on is gone")?),
        None => None,
    };
    let mut points = Vec::with_capacity(o.points.len());
    for (i, p) in o.points.iter().enumerate() {
        let t = match o.anchors.get(i).copied().flatten() {
            Some(id) => Some(frame_of(world, id).ok_or("a part one of its points is on is gone")?),
            None => whole,
        };
        points.push(t.map(|t| t.apply(*p)).unwrap_or(*p));
    }
    let turn = |v: V3| whole.map(|t| t.turn(v)).unwrap_or(v);
    Ok(Placed {
        points,
        center: o.center.map(|c| whole.map(|t| t.apply(c)).unwrap_or(c)),
        normal: turn(o.normal.unwrap_or([0.0, 0.0, 1.0])),
        x_dir: o.x_dir.map(turn),
    })
}

fn fmt_mm(v: f64) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// The B-spline a draft object describes, in project coordinates.
pub fn spline_of(o: &DraftObject, pts: &[V3]) -> Option<BSpline> {
    let deg = o.degree.unwrap_or(3).clamp(1, 5) as usize;
    if o.fit {
        let mut p = pts.to_vec();
        if o.closed && p.len() > 2 {
            p.push(p[0]);
        }
        BSpline::through(&p, deg)
    } else if o.closed {
        BSpline::periodic(pts.to_vec(), deg)
    } else {
        BSpline::clamped(pts.to_vec(), deg)
    }
}

pub fn shape_of(world: &World, o: &DraftObject) -> Result<DraftShape, String> {
    let g = placed(world, o)?;
    let pts = &g.points;
    let mut out = DraftShape { points: pts.clone(), closed: o.closed, ..Default::default() };
    let frame = |c: V3| Frame::new(c, g.normal, g.x_dir).ok_or_else(|| "its normal is zero".to_string());
    let tol = 0.01;
    match o.kind {
        DraftKind::Point => {}
        DraftKind::Line => out.lines.push(pts.clone()),
        DraftKind::Wire | DraftKind::Rectangle => {
            let mut l = pts.clone();
            if o.closed && l.len() > 2 {
                l.push(l[0]);
            }
            out.lines.push(l);
        }
        DraftKind::Polygon => {
            let f = frame(g.center.ok_or("a polygon needs a centre")?)?;
            let (r, n) = (o.radius.unwrap_or(10.0), o.sides.unwrap_or(6).max(3));
            let mut l: Vec<V3> = (0..=n).map(|i| {
                let t = (360.0 * i as f64 / n as f64 + o.start.unwrap_or(0.0)).to_radians();
                f.at(r * t.cos(), r * t.sin())
            }).collect();
            l[n as usize] = l[0];
            out.points = l[..n as usize].to_vec();
            out.closed = true;
            out.lines.push(l);
        }
        DraftKind::Circle => {
            let c = g.center.ok_or("a circle needs a centre")?;
            let r = o.radius.ok_or("a circle needs a radius")?;
            let f = frame(c)?;
            out.lines.push(geom::arc_points(&f, r, 0.0, 360.0, geom::segments_for(r, tol)));
            out.closed = true;
        }
        DraftKind::Arc => {
            let c = g.center.ok_or("an arc needs a centre")?;
            let r = o.radius.ok_or("an arc needs a radius")?;
            let f = frame(c)?;
            let (a0, mut a1) = (o.start.unwrap_or(0.0), o.end.unwrap_or(90.0));
            while a1 <= a0 {
                a1 += 360.0;
            }
            let n = ((geom::segments_for(r, tol) as f64) * (a1 - a0) / 360.0).ceil().max(4.0) as usize;
            let l = geom::arc_points(&f, r, a0, a1, n);
            out.points = vec![l[0], *l.last().unwrap()];
            out.lines.push(l);
        }
        DraftKind::Ellipse => {
            let c = g.center.ok_or("an ellipse needs a centre")?;
            let (rx, ry) = (o.radius.ok_or("an ellipse needs radius (major)")?, o.radius2.ok_or("an ellipse needs radius2 (minor)")?);
            let f = frame(c)?;
            let (a0, mut a1) = (o.start.unwrap_or(0.0), o.end.unwrap_or(360.0));
            while a1 <= a0 {
                a1 += 360.0;
            }
            let n = ((geom::segments_for(rx.max(ry), tol) as f64) * (a1 - a0) / 360.0).ceil().max(8.0) as usize;
            out.lines.push(geom::ellipse_points(&f, rx, ry, a0, a1, n));
            out.closed = (a1 - a0 - 360.0).abs() < 1e-9;
        }
        DraftKind::Bspline => {
            let s = spline_of(o, pts).ok_or("a B-spline needs two or more distinct points")?;
            out.lines.push(s.sample(24));
        }
        DraftKind::Text => {
            let at = *pts.first().ok_or("text needs a place")?;
            out.label = Some((at, o.text.clone().unwrap_or_default()));
        }
        DraftKind::Label => {
            let (at, to) = (*pts.first().ok_or("a label needs a place")?, *pts.get(1).ok_or("a label needs the point it points at")?);
            out.lines.push(vec![to, at]);
            if let Some(d) = unit(sub(to, at)) {
                out.arrows.push((to, d));
            }
            out.label = Some((at, o.text.clone().unwrap_or_default()));
        }
        DraftKind::Dimension => dimension(o, pts, &mut out)?,
    }
    out.length = out.lines.iter().map(|l| geom::length(l)).sum();
    // A plane, area and hatch for flat closed outlines.
    let all: Vec<V3> = out.lines.iter().flatten().copied().collect();
    let f = match (o.kind, g.center) {
        (DraftKind::Circle | DraftKind::Arc | DraftKind::Ellipse | DraftKind::Polygon, Some(c)) => frame(c).ok(),
        _ => geom::plane_of(&all).and_then(|f| Frame::new(f.origin, f.normal(), None)),
    };
    if let Some(f) = f {
        let flat = all.iter().all(|p| f.height(*p).abs() < 1e-6 * (1.0 + norm(*p)));
        if flat {
            out.plane = Some([f.origin, f.x, f.y]);
            if out.closed && let Some(l) = out.lines.first() {
                out.area = Some(geom::area(&f, l));
                if let Some(h) = &o.hatch {
                    out.hatch = geom::hatch(&f, &out.lines, h.angle, h.spacing);
                    if h.pattern == "cross" {
                        out.hatch.extend(geom::hatch(&f, &out.lines, h.angle + 90.0, h.spacing));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn dimension(o: &DraftObject, pts: &[V3], out: &mut DraftShape) -> Result<(), String> {
    let m = o.measure.unwrap_or_default();
    let off = o.offset.unwrap_or([0.0; 3]);
    let text = |v: f64, unit: &str| match &o.text {
        Some(t) if t.contains("<>") => t.replace("<>", &format!("{}{unit}", fmt_mm(v))),
        Some(t) if !t.is_empty() => t.clone(),
        _ => format!("{}{unit}", fmt_mm(v)),
    };
    match m {
        Measure::Angle => {
            let [v, a, b] = [pts.first(), pts.get(1), pts.get(2)].map(|p| p.copied());
            let (v, a, b) = (v.ok_or("an angle needs a vertex")?, a.ok_or("an angle needs two sides")?, b.ok_or("an angle needs two sides")?);
            let (da, db) = (unit(sub(a, v)).ok_or("a side has no length")?, unit(sub(b, v)).ok_or("a side has no length")?);
            let ang = dot(da, db).clamp(-1.0, 1.0).acos().to_degrees();
            let r = if norm(off) > 0.0 { norm(off) } else { dist(a, v).min(dist(b, v)) * 0.6 };
            let n = unit(cross(da, db)).unwrap_or([0.0, 0.0, 1.0]);
            let f = Frame::new(v, n, Some(da)).ok_or("the sides are parallel")?;
            let arc = geom::arc_points(&f, r, 0.0, ang, 32);
            out.arrows.push((arc[0], unit(sub(arc[0], arc[1])).unwrap_or(da)));
            let k = arc.len() - 1;
            out.arrows.push((arc[k], unit(sub(arc[k], arc[k - 1])).unwrap_or(db)));
            out.label = Some((arc[16], text(ang, "°")));
            out.lines.push(vec![v, add(v, mul(da, r * 1.1))]);
            out.lines.push(vec![v, add(v, mul(db, r * 1.1))]);
            out.lines.push(arc);
            out.value = Some(ang);
        }
        Measure::Radius | Measure::Diameter => {
            let (c, p) = (*pts.first().ok_or("a radius needs a centre")?, *pts.get(1).ok_or("a radius needs a point on the curve")?);
            let r = dist(c, p);
            let d = unit(sub(p, c)).ok_or("the point is on the centre")?;
            let (from, word, v) = if m == Measure::Diameter { (sub(c, mul(d, r)), "⌀", 2.0 * r) } else { (c, "R", r) };
            out.lines.push(vec![from, p]);
            out.arrows.push((p, d));
            if m == Measure::Diameter {
                out.arrows.push((from, mul(d, -1.0)));
            }
            out.label = Some((add(add(c, mul(d, r * 0.5)), off), format!("{word}{}", text(v, ""))));
            out.value = Some(v);
        }
        _ => {
            let (a, b) = (*pts.first().ok_or("a dimension needs two points")?, *pts.get(1).ok_or("a dimension needs two points")?);
            let axis = match m {
                Measure::X => Some([1.0, 0.0, 0.0]),
                Measure::Y => Some([0.0, 1.0, 0.0]),
                Measure::Z => Some([0.0, 0.0, 1.0]),
                _ => None,
            };
            // Where the dimension line's ends are: the points carried out by the offset,
            // and for X/Y/Z both set level along the other axes.
            let (p, q, v) = match axis {
                None => (add(a, off), add(b, off), dist(a, b)),
                Some(ax) => {
                    let v = dot(sub(b, a), ax);
                    let p = add(a, off);
                    (p, add(p, mul(ax, v)), v.abs())
                }
            };
            out.lines.push(vec![a, p]);
            out.lines.push(vec![b, q]);
            out.lines.push(vec![p, q]);
            if let Some(d) = unit(sub(q, p)) {
                out.arrows.push((p, mul(d, -1.0)));
                out.arrows.push((q, d));
            }
            out.label = Some((mul(add(p, q), 0.5), text(v, " mm")));
            out.value = Some(v);
        }
    }
    Ok(())
}

fn draft_shape(world: &World, e: Entity) -> Option<Result<DraftShape, String>> {
    let o = world.get::<DraftObject>(e)?;
    Some(shape_of(world, o))
}

// ---- Editing ---------------------------------------------------------------------------------------

fn bad(m: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, m)
}

fn missing(id: StableId, what: &str) -> CommandError {
    CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a {what}"))
}

/// What can be given to create or change a draft object.
#[derive(Deserialize, JsonSchema, Default)]
pub struct DraftFields {
    #[serde(default)]
    pub name: Option<String>,
    /// [x, y, z] in project coordinates, or [u, v] on `plane`.
    #[serde(default)]
    pub points: Option<Vec<Vec<f64>>>,
    #[serde(default)]
    pub closed: Option<bool>,
    #[serde(default)]
    pub center: Option<Vec<f64>>,
    #[serde(default)]
    pub normal: Option<V3>,
    #[serde(default)]
    pub x_dir: Option<V3>,
    #[serde(default)]
    pub radius: Option<f64>,
    #[serde(default)]
    pub radius2: Option<f64>,
    #[serde(default)]
    pub start: Option<f64>,
    #[serde(default)]
    pub end: Option<f64>,
    #[serde(default)]
    pub sides: Option<u32>,
    #[serde(default)]
    pub fit: Option<bool>,
    #[serde(default)]
    pub degree: Option<u32>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub measure: Option<Measure>,
    #[serde(default)]
    pub offset: Option<V3>,
    /// A hatch on a closed outline; null removes it.
    #[serde(default, deserialize_with = "some")]
    pub hatch: Option<Option<Hatch>>,
    #[serde(default)]
    pub color: Option<String>,
    /// The part the object is on (its coordinates are that part's own); null: none.
    #[serde(default, deserialize_with = "some")]
    pub on: Option<Option<StableId>>,
    #[serde(default)]
    pub anchors: Option<Vec<Option<StableId>>>,
    /// A working plane: `points` and `center` are [u, v] (or [u, v, w]) on it,
    /// and `normal` / `x_dir` default to its own.
    #[serde(default)]
    pub plane: Option<StableId>,
}

fn some<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// Applies fields to an object (plane coordinates converted), then checks it.
fn apply_fields(tx: &Tx, o: &mut DraftObject, f: DraftFields) -> Result<(), CommandError> {
    let plane = match f.plane {
        Some(id) => Some(tx.get::<WorkingPlane>(id).ok_or_else(|| missing(id, "working plane"))?.frame()),
        None => None,
    };
    let to3 = |v: &Vec<f64>| -> Result<V3, CommandError> {
        if !finite(v) {
            return Err(bad("coordinates must be finite numbers"));
        }
        match (plane, v.len()) {
            (Some(p), 2 | 3) => Ok(add(p.at(v[0], v[1]), mul(p.normal(), v.get(2).copied().unwrap_or(0.0)))),
            (None, 3) => Ok([v[0], v[1], v[2]]),
            (None, 2) => Ok([v[0], v[1], 0.0]),
            _ => Err(bad("a point is [x, y, z] (or [u, v] on a plane)")),
        }
    };
    if let Some(n) = f.name {
        o.name = n.trim().to_string();
    }
    if let Some(p) = &f.points {
        o.points = p.iter().map(to3).collect::<Result<_, _>>()?;
    }
    if let Some(c) = &f.center {
        o.center = Some(to3(c)?);
    }
    if let Some(p) = plane {
        o.normal = Some(f.normal.unwrap_or(p.normal()));
        o.x_dir = Some(f.x_dir.unwrap_or(p.x));
    } else {
        if f.normal.is_some() {
            o.normal = f.normal;
        }
        if f.x_dir.is_some() {
            o.x_dir = f.x_dir;
        }
    }
    macro_rules! set {
        ($($field:ident),*) => { $( if f.$field.is_some() { o.$field = f.$field; } )* };
    }
    set!(radius, radius2, start, end, sides, degree, text, size, measure, offset, color);
    if let Some(c) = f.closed {
        o.closed = c;
    }
    if let Some(v) = f.fit {
        o.fit = v;
    }
    if let Some(h) = f.hatch {
        o.hatch = h;
    }
    if let Some(on) = f.on {
        o.on = on;
    }
    if let Some(a) = f.anchors {
        o.anchors = a;
    }
    // Three points make arcs and circles.
    if matches!(o.kind, DraftKind::Arc | DraftKind::Circle) && o.points.len() == 3 {
        let [a, b, c] = [o.points[0], o.points[1], o.points[2]];
        let (center, r, n) = geom::circle_through(a, b, c).ok_or_else(|| bad("the three points are on a line"))?;
        let fr = Frame::new(center, n, Some(sub(a, center))).unwrap();
        let (u, v) = fr.local(c);
        o.center = Some(center);
        o.radius = Some(r);
        o.normal = Some(n);
        o.x_dir = Some(fr.x);
        if o.kind == DraftKind::Arc {
            o.start = Some(0.0);
            let mut e = v.atan2(u).to_degrees();
            if e <= 0.0 {
                e += 360.0;
            }
            o.end = Some(e);
        }
        o.points.clear();
    }
    // Two corners make a rectangle on its plane.
    if o.kind == DraftKind::Rectangle && o.points.len() == 2 {
        let fr = Frame::new(o.points[0], o.normal.unwrap_or([0.0, 0.0, 1.0]), o.x_dir).ok_or_else(|| bad("the rectangle's normal is zero"))?;
        let (u, v) = fr.local(o.points[1]);
        if u.abs() < 1e-9 || v.abs() < 1e-9 {
            return Err(bad("the rectangle's corners must differ across its plane"));
        }
        o.points = vec![fr.at(0.0, 0.0), fr.at(u, 0.0), fr.at(u, v), fr.at(0.0, v)];
        o.closed = true;
    }
    check(o)
}

fn check(o: &DraftObject) -> Result<(), CommandError> {
    let n = o.points.len();
    let need = |ok: bool, m: &str| if ok { Ok(()) } else { Err(bad(format!("{}: {m}", o.kind.word()))) };
    match o.kind {
        DraftKind::Point => need(n == 1, "give one point")?,
        DraftKind::Line => need(n == 2, "give two points")?,
        DraftKind::Wire => need(n >= 2 && (!o.closed || n >= 3), "give two or more points (three to close it)")?,
        DraftKind::Rectangle => need(n == 4, "give two corners")?,
        DraftKind::Polygon => need(o.center.is_some() && o.radius.is_some_and(|r| r > 0.0) && o.sides.is_none_or(|s| (3..=1000).contains(&s)), "give a centre, a radius and 3 or more sides")?,
        DraftKind::Circle => need(o.center.is_some() && o.radius.is_some_and(|r| r > 0.0), "give a centre and a radius, or three points")?,
        DraftKind::Arc => need(o.center.is_some() && o.radius.is_some_and(|r| r > 0.0), "give a centre, a radius, start and end, or three points")?,
        DraftKind::Ellipse => need(o.center.is_some() && o.radius.is_some_and(|r| r > 0.0) && o.radius2.is_some_and(|r| r > 0.0), "give a centre, radius (major) and radius2 (minor)")?,
        DraftKind::Bspline => need(n >= 2 && (!o.closed || n >= 3), "give two or more points (three to close it)")?,
        DraftKind::Text => need(n == 1 && o.text.as_ref().is_some_and(|t| !t.trim().is_empty()), "give a place and the text")?,
        DraftKind::Label => need(n == 2 && o.text.as_ref().is_some_and(|t| !t.trim().is_empty()), "give the text's place, the point it points at, and the text")?,
        DraftKind::Dimension => need(if o.measure == Some(Measure::Angle) { n == 3 } else { n == 2 }, "give two points (an angle: the vertex and one point on each side)")?,
    }
    if o.size.is_some_and(|s| !(s > 0.0)) {
        return Err(bad("text size must be positive"));
    }
    if let Some(h) = &o.hatch {
        if !(h.spacing > 0.0) || !h.angle.is_finite() || !["lines", "cross"].contains(&h.pattern.as_str()) {
            return Err(bad("a hatch has a pattern (lines or cross), an angle and a positive spacing"));
        }
    }
    if o.name.is_empty() {
        return Err(bad("a draft object needs a name"));
    }
    Ok(())
}

/// Keeps an object's dependencies on the parts it is on.
fn track_anchors(tx: &mut Tx, id: StableId, before: &[StableId], o: &DraftObject) -> Result<(), CommandError> {
    let mut now: Vec<StableId> = o.anchors.iter().flatten().copied().chain(o.on).collect();
    now.sort_by_key(|x| x.to_string());
    now.dedup();
    for a in &now {
        if frame_of(tx.world(), *a).is_none() {
            return Err(bad(format!("{a} is not a top-level instance or a placed shape: draft objects follow those")));
        }
    }
    for b in before.iter().filter(|b| !now.contains(b)) {
        tx.remove_dependency(id, *b)?;
    }
    for a in &now {
        tx.add_dependency(id, *a)?;
    }
    Ok(())
}

fn anchors_of(o: &DraftObject) -> Vec<StableId> {
    o.anchors.iter().flatten().copied().chain(o.on).collect()
}

fn next_name(tx: &Tx, kind: DraftKind) -> String {
    let used: std::collections::HashSet<String> = tx.world().resource::<IdIndex>().iter().filter_map(|(_, e)| tx.world().get::<DraftObject>(e).map(|o| o.name.clone())).collect();
    (1..).map(|n| format!("{} {n}", kind.word())).find(|n| !used.contains(n)).unwrap()
}

/// `draft.create`: a point, line, wire, rectangle, polygon, circle, arc,
/// ellipse, B-spline, text, label or dimension in model space.
#[derive(Deserialize, JsonSchema)]
pub struct CreateDraft {
    pub kind: DraftKind,
    #[serde(flatten)]
    pub fields: DraftFields,
    /// A layer to put it in.
    #[serde(default)]
    pub layer: Option<StableId>,
}

impl Command for CreateDraft {
    const NAME: &'static str = "draft.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut o = DraftObject {
            name: String::new(),
            kind: self.kind,
            points: vec![],
            closed: false,
            center: None,
            normal: None,
            x_dir: None,
            radius: None,
            radius2: None,
            start: None,
            end: None,
            sides: None,
            fit: false,
            degree: None,
            text: None,
            size: None,
            measure: None,
            offset: None,
            hatch: None,
            color: None,
            on: None,
            anchors: vec![],
        };
        if self.kind == DraftKind::Bspline && self.fields.fit.is_none() {
            o.fit = true; // through the points unless told otherwise
        }
        if self.kind == DraftKind::Polygon && self.fields.sides.is_none() {
            o.sides = Some(6);
        }
        let given_name = self.fields.name.clone();
        apply_fields(tx, &mut o, DraftFields { name: Some(given_name.unwrap_or_else(|| next_name(tx, self.kind))), ..self.fields })?;
        let id = tx.create();
        track_anchors(tx, id, &[], &o)?;
        let shape = shape_of(tx.world(), &o).map_err(bad)?;
        tx.set(id, o)?;
        if let Some(layer) = self.layer {
            tx.run("draft.layer.update", json!({ "target": layer, "add": [id] }))?;
        }
        Ok(json!({ "id": id, "length": shape.length, "area": shape.area, "value": shape.value }))
    }
}

/// `draft.update`: change any of a draft object's fields.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateDraft {
    pub target: StableId,
    #[serde(flatten)]
    pub fields: DraftFields,
}

impl Command for UpdateDraft {
    const NAME: &'static str = "draft.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut o = tx.get::<DraftObject>(self.target).cloned().ok_or_else(|| missing(self.target, "draft object"))?;
        let before = anchors_of(&o);
        apply_fields(tx, &mut o, self.fields)?;
        track_anchors(tx, self.target, &before, &o)?;
        let shape = shape_of(tx.world(), &o).map_err(bad)?;
        tx.set(self.target, o)?;
        Ok(json!({ "id": self.target, "length": shape.length, "area": shape.area, "value": shape.value }))
    }
}

// ---- Draft to sketch ------------------------------------------------------------------------------

/// `draft.to_sketch`: flat draft geometry (lines, wires, rectangles,
/// polygons, circles, arcs, ellipses, B-splines, points) as a new sketch on
/// their common plane, curves joined where they meet: lines, arcs and
/// circles as they are, ellipses and B-splines as exact cubic Béziers
/// (ellipses to within 0.03 %). Text, labels and dimensions are left out.
/// `delete: true` removes the draft objects it used.
#[derive(Deserialize, JsonSchema)]
pub struct ToSketch {
    pub targets: Vec<StableId>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub delete: bool,
}

struct Builder {
    frame: Frame,
    points: Vec<(u32, f64, f64)>,
    curves: Vec<Value>,
    next: u32,
}

impl Builder {
    fn point(&mut self, p: V3) -> u32 {
        let (u, v) = self.frame.local(p);
        if let Some((id, _, _)) = self.points.iter().find(|(_, x, y)| (x - u).abs() < 1e-6 && (y - v).abs() < 1e-6) {
            return *id;
        }
        self.next += 1;
        self.points.push((self.next, u, v));
        self.next
    }
    /// A free point (handles and centres are never merged with ends).
    fn free(&mut self, p: V3) -> u32 {
        let (u, v) = self.frame.local(p);
        self.next += 1;
        self.points.push((self.next, u, v));
        self.next
    }
    fn curve(&mut self, mut c: Value) {
        self.next += 1;
        c["id"] = json!(self.next);
        self.curves.push(c);
    }
    fn line(&mut self, a: V3, b: V3) {
        if dist(a, b) < 1e-9 {
            return;
        }
        let (a, b) = (self.point(a), self.point(b));
        self.curve(json!({ "kind": "line", "a": a, "b": b }));
    }
    fn bezier(&mut self, b: [V3; 4]) {
        let (a, e) = (self.point(b[0]), self.point(b[3]));
        let (c1, c2) = (self.free(b[1]), self.free(b[2]));
        self.curve(json!({ "kind": "bezier", "a": a, "b": e, "c1": c1, "c2": c2 }));
    }
}

/// Cubic Béziers for an elliptical arc (quarter turns or less each).
fn ellipse_beziers(f: &Frame, rx: f64, ry: f64, a0: f64, a1: f64) -> Vec<[V3; 4]> {
    let n = ((a1 - a0) / 90.0).ceil().max(1.0) as usize;
    let step = (a1 - a0) / n as f64;
    let k = 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
    (0..n)
        .map(|i| {
            let (t0, t1) = ((a0 + step * i as f64).to_radians(), (a0 + step * (i + 1) as f64).to_radians());
            let p = |t: f64| (rx * t.cos(), ry * t.sin());
            let d = |t: f64| (-rx * t.sin(), ry * t.cos());
            let (p0, p1, d0, d1) = (p(t0), p(t1), d(t0), d(t1));
            [f.at(p0.0, p0.1), f.at(p0.0 + k * d0.0, p0.1 + k * d0.1), f.at(p1.0 - k * d1.0, p1.1 - k * d1.1), f.at(p1.0, p1.1)]
        })
        .collect()
}

impl Command for ToSketch {
    const NAME: &'static str = "draft.to_sketch";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.targets.is_empty() {
            return Err(bad("name the draft objects to turn into a sketch"));
        }
        let mut objs = Vec::new();
        for id in &self.targets {
            let o = tx.get::<DraftObject>(*id).cloned().ok_or_else(|| missing(*id, "draft object"))?;
            let g = placed(tx.world(), &o).map_err(bad)?;
            let s = shape_of(tx.world(), &o).map_err(bad)?;
            objs.push((*id, o, g, s));
        }
        let geometric = |k: DraftKind| !matches!(k, DraftKind::Text | DraftKind::Label | DraftKind::Dimension);
        // The common plane: the first object that has one, else through all their points.
        let all: Vec<V3> = objs.iter().filter(|o| geometric(o.1.kind)).flat_map(|o| o.3.lines.iter().flatten().copied().chain(o.3.points.iter().copied())).collect();
        let frame = objs
            .iter()
            .filter(|o| geometric(o.1.kind))
            .find_map(|o| o.3.plane.map(|[origin, x, y]| Frame { origin, x, y }))
            .or_else(|| geom::plane_of(&all))
            .ok_or_else(|| bad("these lie on one line: there is no plane to sketch on (add a point off the line)"))?;
        let off = all.iter().map(|p| frame.height(*p).abs()).fold(0.0, f64::max);
        if off > 1e-4 {
            return Err(bad(format!("these are not on one plane (one is {:.3} mm off it): a sketch is flat", off)));
        }
        let mut b = Builder { frame, points: vec![], curves: vec![], next: 0 };
        let mut skipped = Vec::new();
        let same_way = |n: V3| dot(n, frame.normal()) >= 0.0;
        for (_, o, g, s) in &objs {
            match o.kind {
                DraftKind::Point => {
                    b.point(s.points[0]);
                }
                DraftKind::Line | DraftKind::Wire | DraftKind::Rectangle | DraftKind::Polygon => {
                    for l in &s.lines {
                        for w in l.windows(2) {
                            b.line(w[0], w[1]);
                        }
                    }
                }
                DraftKind::Circle => {
                    let c = b.free(g.center.unwrap());
                    b.curve(json!({ "kind": "circle", "center": c, "radius": o.radius.unwrap() }));
                }
                DraftKind::Arc => {
                    let l = &s.lines[0];
                    let (mut a, mut e) = (l[0], *l.last().unwrap());
                    if !same_way(g.normal) {
                        std::mem::swap(&mut a, &mut e); // counter-clockwise in the sketch's plane
                    }
                    let c = b.free(g.center.unwrap());
                    let (a, e) = (b.point(a), b.point(e));
                    b.curve(json!({ "kind": "arc", "center": c, "start": a, "end": e }));
                }
                DraftKind::Ellipse => {
                    let f = Frame::new(g.center.unwrap(), g.normal, g.x_dir).unwrap();
                    let (a0, mut a1) = (o.start.unwrap_or(0.0), o.end.unwrap_or(360.0));
                    while a1 <= a0 {
                        a1 += 360.0;
                    }
                    for bz in ellipse_beziers(&f, o.radius.unwrap(), o.radius2.unwrap(), a0, a1) {
                        b.bezier(bz);
                    }
                }
                DraftKind::Bspline => {
                    let sp = spline_of(o, &g.points).ok_or_else(|| bad(format!("{}: not a curve", o.name)))?;
                    for bz in sp.beziers() {
                        b.bezier(bz);
                    }
                }
                DraftKind::Text | DraftKind::Label | DraftKind::Dimension => skipped.push(o.name.clone()),
            }
        }
        if b.curves.is_empty() && b.points.is_empty() {
            return Err(bad("nothing here makes sketch geometry (text, labels and dimensions do not)"));
        }
        let name = self.name.unwrap_or_else(|| {
            let used: std::collections::HashSet<String> = tx.world().resource::<IdIndex>().iter().filter_map(|(_, e)| tx.world().get::<de_shape::sketch::Sketch>(e).map(|s| s.name.clone())).collect();
            (1..).map(|n| format!("Draft sketch {n}")).find(|n| !used.contains(n)).unwrap()
        });
        let points: Vec<Value> = b.points.iter().map(|(id, x, y)| json!({ "id": id, "x": x, "y": y })).collect();
        let out = tx.run(
            "sketch.create",
            json!({ "name": name, "plane": { "origin": frame.origin, "x": frame.x, "y": frame.y }, "geometry": { "points": points, "curves": b.curves, "constraints": [] } }),
        )?;
        if self.delete {
            for (id, o, ..) in &objs {
                if geometric(o.kind) {
                    tx.delete(*id)?;
                }
            }
        }
        Ok(json!({ "id": out["id"], "name": name, "curves": b.curves.len(), "skipped": skipped }))
    }
}

// ---- Layers and working planes ----------------------------------------------------------------------

/// `draft.layer.create`: a layer, optionally with members.
#[derive(Deserialize, JsonSchema)]
pub struct CreateLayer {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub members: Vec<StableId>,
}

fn take_from_other_layers(tx: &mut Tx, except: Option<StableId>, ids: &[StableId]) -> Result<(), CommandError> {
    let layers: Vec<(StableId, Layer)> = tx.world().resource::<IdIndex>().iter().filter(|(id, _)| Some(*id) != except).filter_map(|(id, e)| tx.world().get::<Layer>(e).map(|l| (id, l.clone()))).collect();
    for (id, mut l) in layers {
        let n = l.members.len();
        l.members.retain(|m| !ids.contains(m));
        if l.members.len() != n {
            tx.set(id, l)?;
        }
    }
    Ok(())
}

fn check_color(c: &Option<String>) -> Result<(), CommandError> {
    if let Some(c) = c
        && c.chars().count() > 40
    {
        return Err(bad("a colour is a name or #hex"));
    }
    Ok(())
}

impl Command for CreateLayer {
    const NAME: &'static str = "draft.layer.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(bad("a layer needs a name"));
        }
        check_color(&self.color)?;
        for m in &self.members {
            tx.entity(*m)?;
        }
        take_from_other_layers(tx, None, &self.members)?;
        let id = tx.create();
        let mut members = self.members;
        members.dedup();
        tx.set(id, Layer { name, color: self.color.filter(|c| !c.is_empty()), visible: self.visible.unwrap_or(true), locked: self.locked.unwrap_or(false), members })?;
        Ok(json!({ "id": id }))
    }
}

/// `draft.layer.update`: rename, recolour, show/hide, lock, add or remove members.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateLayer {
    pub target: StableId,
    #[serde(default)]
    pub name: Option<String>,
    /// "" removes the colour.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub add: Vec<StableId>,
    #[serde(default)]
    pub remove: Vec<StableId>,
}

impl Command for UpdateLayer {
    const NAME: &'static str = "draft.layer.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut l = tx.get::<Layer>(self.target).cloned().ok_or_else(|| missing(self.target, "layer"))?;
        if let Some(n) = self.name {
            let n = n.trim().to_string();
            if n.is_empty() {
                return Err(bad("a layer needs a name"));
            }
            l.name = n;
        }
        if let Some(c) = self.color {
            l.color = Some(c).filter(|c| !c.is_empty());
            check_color(&l.color)?;
        }
        if let Some(v) = self.visible {
            l.visible = v;
        }
        if let Some(v) = self.locked {
            l.locked = v;
        }
        for m in &self.add {
            tx.entity(*m)?;
        }
        take_from_other_layers(tx, Some(self.target), &self.add)?;
        for m in self.add {
            if !l.members.contains(&m) {
                l.members.push(m);
            }
        }
        l.members.retain(|m| !self.remove.contains(m));
        tx.set(self.target, l.clone())?;
        Ok(json!({ "id": self.target, "members": l.members.len() }))
    }
}

/// `draft.plane.create`: a named working plane through `origin` square to
/// `normal`, its x along `x` (projected into it; default world X or Y).
#[derive(Deserialize, JsonSchema)]
pub struct CreatePlane {
    pub name: String,
    pub origin: V3,
    pub normal: V3,
    #[serde(default)]
    pub x: Option<V3>,
}

fn plane_from(name: String, origin: V3, normal: V3, x: Option<V3>) -> Result<WorkingPlane, CommandError> {
    if !finite(&origin) || !finite(&normal) || x.is_some_and(|x| !finite(&x)) {
        return Err(bad("a plane's numbers must be finite"));
    }
    let f = Frame::new(origin, normal, x).ok_or_else(|| bad("a plane needs a non-zero normal"))?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(bad("a working plane needs a name"));
    }
    Ok(WorkingPlane { name, origin: f.origin, x: f.x, y: f.y })
}

impl Command for CreatePlane {
    const NAME: &'static str = "draft.plane.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let p = plane_from(self.name, self.origin, self.normal, self.x)?;
        let id = tx.create();
        tx.set(id, p.clone())?;
        Ok(json!({ "id": id, "plane": { "origin": p.origin, "x": p.x, "y": p.y } }))
    }
}

/// `draft.plane.update`: move, turn or rename a working plane.
#[derive(Deserialize, JsonSchema)]
pub struct UpdatePlane {
    pub target: StableId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub origin: Option<V3>,
    #[serde(default)]
    pub normal: Option<V3>,
    #[serde(default)]
    pub x: Option<V3>,
}

impl Command for UpdatePlane {
    const NAME: &'static str = "draft.plane.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let old = tx.get::<WorkingPlane>(self.target).cloned().ok_or_else(|| missing(self.target, "working plane"))?;
        let p = plane_from(self.name.unwrap_or(old.name.clone()), self.origin.unwrap_or(old.origin), self.normal.unwrap_or(cross(old.x, old.y)), self.x.or(Some(old.x)))?;
        tx.set(self.target, p.clone())?;
        Ok(json!({ "id": self.target, "plane": { "origin": p.origin, "x": p.x, "y": p.y } }))
    }
}
