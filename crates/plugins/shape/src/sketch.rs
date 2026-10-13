//! Sketches: 2D geometry drawn by people, held exact by constraints.
//!
//! A sketch is points and curves on a plane (lines, circles and arcs,
//! ellipses and arcs of ellipses, hyperbolas and parabolas, cubic Béziers
//! and B-splines of any degree, rational or not, open or periodic), plus
//! constraints (coincident, point on curve, horizontal, vertical, parallel,
//! perpendicular, tangent, equal, symmetric about a line or a point, fixed,
//! block, internal alignment of a conic's axes and foci, Snell's law, and the
//! steps of linear and circular patterns) and dimensions. Every dimension is
//! a parameter of the sketch, so it can be typed, scrubbed, driven by a
//! formula (`=Desk size.width - 40`) or read by other shapes, the same as any
//! other parameter. A dimension can be a reference (measured, not driving),
//! and any constraint can be switched off without losing it. Nothing follows
//! anything implicitly.
//!
//! The solver (solver.rs) moves the points as little as possible to satisfy
//! every constraint. It reports how many degrees of freedom remain and which
//! points can still move, and which constraints conflict, are redundant or
//! are malformed.
//!
//! Closed loops become profiles: outer outlines counter-clockwise, holes
//! clockwise, arcs kept as arcs and conics and splines as exact rational
//! B-splines for the exact kernel.

use crate::curves::{Geo, Nurbs};
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub use crate::solver::solve;

/// Where a sketch lies: an origin and two in-plane axes (mm, unit vectors).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plane {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
}

impl Default for Plane {
    fn default() -> Self {
        Plane { origin: [0.0; 3], x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0] }
    }
}

impl Plane {
    pub fn normal(&self) -> [f64; 3] {
        let (a, b) = (self.x, self.y);
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }
    /// A sketch point in the world (mm).
    pub fn to_world(&self, p: [f64; 2]) -> [f64; 3] {
        [0, 1, 2].map(|k| self.origin[k] + self.x[k] * p[0] + self.y[k] * p[1])
    }
    /// A world point in sketch coordinates (dropped square onto the plane).
    pub fn to_local(&self, w: [f64; 3]) -> [f64; 2] {
        let d = [w[0] - self.origin[0], w[1] - self.origin[1], w[2] - self.origin[2]];
        [d[0] * self.x[0] + d[1] * self.x[1] + d[2] * self.x[2], d[0] * self.y[0] + d[1] * self.y[1] + d[2] * self.y[2]]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Point {
    pub id: u32,
    pub x: f64,
    pub y: f64,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Curve {
    Line { id: u32, a: u32, b: u32, #[serde(default)] construction: bool },
    /// Counter-clockwise from `start` to `end` around `center`.
    Arc { id: u32, center: u32, start: u32, end: u32, #[serde(default)] construction: bool },
    Circle { id: u32, center: u32, radius: f64, #[serde(default)] construction: bool },
    /// A cubic Bézier from `a` to `b`, shaped by its handles `c1` (leaving `a`)
    /// and `c2` (arriving at `b`). All four are points: the solver moves them
    /// like any other, and constraints and dimensions can hold them.
    Bezier { id: u32, a: u32, b: u32, c1: u32, c2: u32, #[serde(default)] construction: bool },
    /// A whole ellipse: its centre, `major` the end of one semi-axis (its
    /// length and direction), `minor` the other semi-axis (mm; the solver
    /// moves it like a circle's radius).
    Ellipse { id: u32, center: u32, major: u32, minor: f64, #[serde(default)] construction: bool },
    /// Part of an ellipse, counter-clockwise from `start` to `end` (both held on it).
    EllipseArc { id: u32, center: u32, major: u32, minor: f64, start: u32, end: u32, #[serde(default)] construction: bool },
    /// Part of one branch of a hyperbola: `center`, `major` its vertex (the
    /// branch opens away from the centre through it), `minor` the conjugate
    /// semi-axis; from `start` to `end`, both held on it.
    HyperbolaArc { id: u32, center: u32, major: u32, minor: f64, start: u32, end: u32, #[serde(default)] construction: bool },
    /// Part of a parabola with this `vertex` and `focus`, from `start` to `end` (both held on it).
    ParabolaArc { id: u32, vertex: u32, focus: u32, start: u32, end: u32, #[serde(default)] construction: bool },
    /// A B-spline on its control points (`poles`, points the solver moves).
    /// `degree` 1 or more; `weights` (one per pole, default all 1) make it
    /// rational (NURBS); `knots` and `mults` are the distinct knots and how
    /// often each repeats (default uniform, the ends repeated degree + 1
    /// times so the curve starts at the first pole and ends at the last).
    /// `periodic`: closed and smooth all round (uniform knots).
    #[serde(rename = "bspline")]
    BSpline {
        id: u32,
        poles: Vec<u32>,
        degree: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        weights: Vec<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        knots: Vec<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mults: Vec<u32>,
        #[serde(default, skip_serializing_if = "is_false")]
        periodic: bool,
        #[serde(default)]
        construction: bool,
    },
}

impl Curve {
    pub fn id(&self) -> u32 {
        match self {
            Curve::Line { id, .. }
            | Curve::Arc { id, .. }
            | Curve::Circle { id, .. }
            | Curve::Bezier { id, .. }
            | Curve::Ellipse { id, .. }
            | Curve::EllipseArc { id, .. }
            | Curve::HyperbolaArc { id, .. }
            | Curve::ParabolaArc { id, .. }
            | Curve::BSpline { id, .. } => *id,
        }
    }
    pub fn construction(&self) -> bool {
        match self {
            Curve::Line { construction, .. }
            | Curve::Arc { construction, .. }
            | Curve::Circle { construction, .. }
            | Curve::Bezier { construction, .. }
            | Curve::Ellipse { construction, .. }
            | Curve::EllipseArc { construction, .. }
            | Curve::HyperbolaArc { construction, .. }
            | Curve::ParabolaArc { construction, .. }
            | Curve::BSpline { construction, .. } => *construction,
        }
    }
    pub fn set_construction(&mut self, on: bool) {
        match self {
            Curve::Line { construction, .. }
            | Curve::Arc { construction, .. }
            | Curve::Circle { construction, .. }
            | Curve::Bezier { construction, .. }
            | Curve::Ellipse { construction, .. }
            | Curve::EllipseArc { construction, .. }
            | Curve::HyperbolaArc { construction, .. }
            | Curve::ParabolaArc { construction, .. }
            | Curve::BSpline { construction, .. } => *construction = on,
        }
    }
    pub fn set_id(&mut self, new: u32) {
        match self {
            Curve::Line { id, .. }
            | Curve::Arc { id, .. }
            | Curve::Circle { id, .. }
            | Curve::Bezier { id, .. }
            | Curve::Ellipse { id, .. }
            | Curve::EllipseArc { id, .. }
            | Curve::HyperbolaArc { id, .. }
            | Curve::ParabolaArc { id, .. }
            | Curve::BSpline { id, .. } => *id = new,
        }
    }
    /// Every point the curve is built on: its ends first, then centres, handles, poles.
    pub fn points(&self) -> Vec<u32> {
        match self {
            Curve::Line { a, b, .. } => vec![*a, *b],
            Curve::Arc { center, start, end, .. } => vec![*center, *start, *end],
            Curve::Circle { center, .. } => vec![*center],
            Curve::Bezier { a, b, c1, c2, .. } => vec![*a, *b, *c1, *c2],
            Curve::Ellipse { center, major, .. } => vec![*center, *major],
            Curve::EllipseArc { center, major, start, end, .. } | Curve::HyperbolaArc { center, major, start, end, .. } => vec![*center, *major, *start, *end],
            Curve::ParabolaArc { vertex, focus, start, end, .. } => vec![*vertex, *focus, *start, *end],
            Curve::BSpline { poles, .. } => poles.clone(),
        }
    }
    /// Where it starts and ends (none for a closed curve).
    pub fn ends(&self) -> Option<(u32, u32)> {
        match self {
            Curve::Line { a, b, .. } | Curve::Bezier { a, b, .. } => Some((*a, *b)),
            Curve::Arc { start, end, .. } | Curve::EllipseArc { start, end, .. } | Curve::HyperbolaArc { start, end, .. } | Curve::ParabolaArc { start, end, .. } => Some((*start, *end)),
            Curve::BSpline { poles, periodic: false, .. } => Some((poles[0], *poles.last()?)),
            _ => None,
        }
    }
    /// Its own size, held by the solver like a point (a circle's radius, a conic's minor semi-axis).
    pub fn size(&self) -> Option<f64> {
        match self {
            Curve::Circle { radius, .. } => Some(*radius),
            Curve::Ellipse { minor, .. } | Curve::EllipseArc { minor, .. } | Curve::HyperbolaArc { minor, .. } => Some(*minor),
            _ => None,
        }
    }
    pub fn set_size(&mut self, v: f64) {
        match self {
            Curve::Circle { radius, .. } => *radius = v,
            Curve::Ellipse { minor, .. } | Curve::EllipseArc { minor, .. } | Curve::HyperbolaArc { minor, .. } => *minor = v,
            _ => {}
        }
    }
    /// The same curve on other points (`map` from old point ids to new).
    pub fn mapped(&self, id: u32, map: &dyn Fn(u32) -> u32) -> Curve {
        let mut c = self.clone();
        c.set_id(id);
        match &mut c {
            Curve::Line { a, b, .. } => (*a, *b) = (map(*a), map(*b)),
            Curve::Arc { center, start, end, .. } => (*center, *start, *end) = (map(*center), map(*start), map(*end)),
            Curve::Circle { center, .. } => *center = map(*center),
            Curve::Bezier { a, b, c1, c2, .. } => (*a, *b, *c1, *c2) = (map(*a), map(*b), map(*c1), map(*c2)),
            Curve::Ellipse { center, major, .. } => (*center, *major) = (map(*center), map(*major)),
            Curve::EllipseArc { center, major, start, end, .. } | Curve::HyperbolaArc { center, major, start, end, .. } => {
                (*center, *major, *start, *end) = (map(*center), map(*major), map(*start), map(*end))
            }
            Curve::ParabolaArc { vertex, focus, start, end, .. } => (*vertex, *focus, *start, *end) = (map(*vertex), map(*focus), map(*start), map(*end)),
            Curve::BSpline { poles, .. } => {
                for p in poles.iter_mut() {
                    *p = map(*p);
                }
            }
        }
        c
    }
    /// The same curve reflected (it runs the other way round: arcs swap their ends).
    pub fn flipped(&self) -> Curve {
        let mut c = self.clone();
        match &mut c {
            Curve::Arc { start, end, .. } | Curve::EllipseArc { start, end, .. } => std::mem::swap(start, end),
            _ => {}
        }
        c
    }
}

/// A dimension's value: a sketch parameter (by key), so it can be linked.
pub type DimKey = String;

/// What an internal-alignment constraint ties to a conic or spline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// A line from end to end of the major axis (through the centre).
    MajorAxis,
    /// A line from end to end of the minor axis.
    MinorAxis,
    /// A point at the focus on the `major` side.
    Focus1,
    /// A point at the other focus.
    Focus2,
    /// A point at knot `index` of a B-spline.
    Knot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    Coincident { a: u32, b: u32 },
    Horizontal { line: u32 },
    Vertical { line: u32 },
    Parallel { a: u32, b: u32 },
    /// Two lines square; a line through a circle's or arc's centre; or, at
    /// point `at` (a shared end, or a point held on both), two curves square.
    Perpendicular {
        a: u32,
        b: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<u32>,
    },
    /// Same length (lines), radius (circles, arcs) or both semi-axes (ellipses).
    Equal { a: u32, b: u32 },
    /// A point on a curve: a line (extended), a circle, an arc's or a conic's
    /// whole curve, a Bézier or a B-spline.
    OnCurve { point: u32, curve: u32 },
    /// Two curves tangent. Apart: a line and a circle, arc or conic, or two
    /// circles or arcs. At point `at` (or where they share an end): any two
    /// curves, one smooth join; a point that is not an end of a curve is
    /// held on it too (tangent via a point).
    Tangent {
        a: u32,
        b: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<u32>,
    },
    Midpoint { point: u32, line: u32 },
    /// Pins a point where it is.
    Fixed { point: u32, x: f64, y: f64 },
    /// Holds a whole curve as it is (its points and size), so it can be
    /// drawn against without dimensioning it.
    Block { curve: u32 },
    /// Distance between any two of: points, lines (parallel: the gap; a
    /// point and a line: square to it), circles and arcs (edge to edge), and
    /// a point and any curve. A line's length when a = b = the line.
    Distance { a: u32, b: u32, key: DimKey },
    /// Signed X or Y offset from point a to point b; a point's own X or Y
    /// (from the sketch origin) when a = b.
    HorizontalDistance { a: u32, b: u32, key: DimKey },
    VerticalDistance { a: u32, b: u32, key: DimKey },
    Length { line: u32, key: DimKey },
    Radius { curve: u32, key: DimKey },
    Diameter { curve: u32, key: DimKey },
    /// Angle from line a to line b, degrees; an arc's sweep when a = b = the arc.
    Angle { a: u32, b: u32, key: DimKey },
    /// Point b is point a moved by line `along` (its start to its end): one
    /// step of a linear pattern. Editing the original moves its copies.
    Translate { a: u32, b: u32, along: u32 },
    /// Point b is point a turned about the start of line `from` by the angle
    /// from line `from` to line `to`: one step of a circular pattern.
    Rotate { a: u32, b: u32, from: u32, to: u32 },
    /// Points a and b mirror each other about `line` (a line, extended), or
    /// about a point (`line` names a point: it is their midpoint).
    Symmetric { a: u32, b: u32, line: u32 },
    /// Snell's law: ray `a` arrives at point `at` on `boundary` and ray `b`
    /// leaves it; n₁·sin θ₁ = n₂·sin θ₂, `key` holding n₂ / n₁.
    Snell { a: u32, b: u32, boundary: u32, at: u32, key: DimKey },
    /// Construction geometry tied to a conic's axes or foci, or a point to a
    /// B-spline's knot (`index`, counting distinct knots from 0).
    InternalAlignment {
        curve: u32,
        element: u32,
        role: Role,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u32>,
    },
    /// A dimension that measures and does not drive (its key's value is
    /// what the drawing measures).
    Reference { of: Box<Constraint> },
    /// A constraint switched off: kept, shown, and not solved.
    Inactive { of: Box<Constraint> },
}

impl Constraint {
    pub fn key(&self) -> Option<&str> {
        match self {
            Constraint::Distance { key, .. }
            | Constraint::HorizontalDistance { key, .. }
            | Constraint::VerticalDistance { key, .. }
            | Constraint::Length { key, .. }
            | Constraint::Radius { key, .. }
            | Constraint::Diameter { key, .. }
            | Constraint::Angle { key, .. }
            | Constraint::Snell { key, .. } => Some(key),
            Constraint::Reference { of } | Constraint::Inactive { of } => of.key(),
            _ => None,
        }
    }
    /// The constraint itself, out of a reference or off switch.
    pub fn inner(&self) -> &Constraint {
        match self {
            Constraint::Reference { of } | Constraint::Inactive { of } => of.inner(),
            c => c,
        }
    }
    /// The points and curves it refers to.
    pub fn ids(&self) -> Vec<u32> {
        match self {
            Constraint::Coincident { a, b }
            | Constraint::Parallel { a, b }
            | Constraint::Equal { a, b }
            | Constraint::Distance { a, b, .. }
            | Constraint::HorizontalDistance { a, b, .. }
            | Constraint::VerticalDistance { a, b, .. }
            | Constraint::Angle { a, b, .. } => vec![*a, *b],
            Constraint::Perpendicular { a, b, at } | Constraint::Tangent { a, b, at } => [*a, *b].into_iter().chain(*at).collect(),
            Constraint::Horizontal { line } | Constraint::Vertical { line } => vec![*line],
            Constraint::OnCurve { point, curve } => vec![*point, *curve],
            Constraint::Midpoint { point, line } => vec![*point, *line],
            Constraint::Fixed { point, .. } => vec![*point],
            Constraint::Block { curve } => vec![*curve],
            Constraint::Length { line, .. } => vec![*line],
            Constraint::Radius { curve, .. } | Constraint::Diameter { curve, .. } => vec![*curve],
            Constraint::Translate { a, b, along } => vec![*a, *b, *along],
            Constraint::Rotate { a, b, from, to } => vec![*a, *b, *from, *to],
            Constraint::Symmetric { a, b, line } => vec![*a, *b, *line],
            Constraint::Snell { a, b, boundary, at, .. } => vec![*a, *b, *boundary, *at],
            Constraint::InternalAlignment { curve, element, .. } => vec![*curve, *element],
            Constraint::Reference { of } | Constraint::Inactive { of } => of.ids(),
        }
    }
    /// The same constraint on other points and curves.
    pub fn mapped(&self, map: &dyn Fn(u32) -> u32) -> Constraint {
        let mut v = serde_json::to_value(self).unwrap();
        fn walk(v: &mut serde_json::Value, map: &dyn Fn(u32) -> u32) {
            if let serde_json::Value::Object(m) = v {
                for (k, x) in m.iter_mut() {
                    match k.as_str() {
                        "kind" | "key" | "x" | "y" | "role" | "index" => {}
                        "of" => walk(x, map),
                        _ => {
                            if let Some(n) = x.as_u64() {
                                *x = serde_json::Value::from(map(n as u32));
                            }
                        }
                    }
                }
            }
        }
        walk(&mut v, map);
        serde_json::from_value(v).unwrap()
    }
}

/// The geometry and constraints of a sketch (its plane and dimensions live on the component).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Geometry {
    pub points: Vec<Point>,
    pub curves: Vec<Curve>,
    pub constraints: Vec<Constraint>,
}

impl Geometry {
    /// The next free id for a point or curve.
    pub fn next_id(&self) -> u32 {
        self.points.iter().map(|p| p.id).chain(self.curves.iter().map(Curve::id)).max().unwrap_or(0) + 1
    }
}

// ---- Solving -------------------------------------------------------------------

/// What solving found.
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct Solved {
    /// Points where the constraints put them.
    pub points: BTreeMap<u32, [f64; 2]>,
    /// Circle radii, and the minor semi-axes of ellipses and hyperbolas.
    pub radii: BTreeMap<u32, f64>,
    /// Degrees of freedom left (0 = fully constrained).
    pub free: usize,
    /// Indices of constraints that cannot all hold together (empty when solved).
    pub conflicts: Vec<usize>,
    /// Indices of constraints that hold but add nothing (others already
    /// fix what they would): removing them changes nothing.
    #[serde(default)]
    pub redundant: Vec<usize>,
    /// Constraints part of which is already fixed by others (e.g. a
    /// coincidence where one coordinate is already held).
    #[serde(default)]
    pub partially_redundant: Vec<usize>,
    /// Constraints that make no sense for what they refer to (e.g. two
    /// lines tangent); they are left out of the solve.
    #[serde(default)]
    pub malformed: Vec<usize>,
    /// Points that can still move (some freedom left in them).
    #[serde(default)]
    pub free_points: Vec<u32>,
    /// Curves that can still move or change size.
    #[serde(default)]
    pub free_curves: Vec<u32>,
    /// Every dimension's value as the drawing measures it, by key (a
    /// reference dimension's only value).
    #[serde(default)]
    pub measured: BTreeMap<String, f64>,
    /// Largest remaining error (mm).
    pub residual: f64,
}

// ---- Curve geometry ---------------------------------------------------------------

/// A curve as geometry, given where its points are (`at`) and its size (`size`).
pub fn geo_of(c: &Curve, at: &dyn Fn(u32) -> Option<[f64; 2]>, size: Option<f64>) -> Option<Geo> {
    use std::f64::consts::TAU;
    let ang = |c: [f64; 2], p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
    Some(match c {
        Curve::Line { a, b, .. } => Geo::Line(at(*a)?, at(*b)?),
        Curve::Circle { center, .. } => Geo::Arc(at(*center)?, size?, 0.0, TAU),
        Curve::Arc { center, start, end, .. } => {
            let (cc, s, e) = (at(*center)?, at(*start)?, at(*end)?);
            let a0 = ang(cc, s);
            let mut sweep = (ang(cc, e) - a0).rem_euclid(TAU);
            if sweep < 1e-12 {
                sweep = TAU;
            }
            Geo::Arc(cc, dist(cc, s), a0, a0 + sweep)
        }
        Curve::Bezier { a, b, c1, c2, .. } => Geo::Nurbs(Nurbs::new(3, vec![at(*a)?, at(*c1)?, at(*c2)?, at(*b)?], vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0])),
        Curve::Ellipse { center, major, .. } | Curve::EllipseArc { center, major, .. } | Curve::HyperbolaArc { center, major, .. } => {
            let (cc, m) = (at(*center)?, at(*major)?);
            let a = dist(cc, m).max(1e-12);
            let u = [(m[0] - cc[0]) / a, (m[1] - cc[1]) / a];
            let b = size?.abs().max(1e-12);
            let local = |p: [f64; 2]| {
                let d = sub(p, cc);
                (dot(d, u), cross(u, d))
            };
            match c {
                Curve::Ellipse { .. } => Geo::Ellipse(cc, u, a, b, 0.0, TAU),
                Curve::EllipseArc { start, end, .. } => {
                    let t = |p: [f64; 2]| {
                        let (x, y) = local(p);
                        (y / b).atan2(x / a)
                    };
                    let t0 = t(at(*start)?);
                    let mut sweep = (t(at(*end)?) - t0).rem_euclid(TAU);
                    if sweep < 1e-12 {
                        sweep = TAU;
                    }
                    Geo::Ellipse(cc, u, a, b, t0, t0 + sweep)
                }
                Curve::HyperbolaArc { start, end, .. } => {
                    let t = |p: [f64; 2]| (local(p).1 / b).asinh();
                    Geo::Hyperbola(cc, u, a, b, t(at(*start)?), t(at(*end)?))
                }
                _ => unreachable!(),
            }
        }
        Curve::ParabolaArc { vertex, focus, start, end, .. } => {
            let (v, f) = (at(*vertex)?, at(*focus)?);
            let fl = dist(v, f).max(1e-12);
            let u = [(f[0] - v[0]) / fl, (f[1] - v[1]) / fl];
            let s = |p: [f64; 2]| cross(u, sub(p, v));
            Geo::Parabola(v, u, fl, s(at(*start)?), s(at(*end)?))
        }
        Curve::BSpline { poles, degree, weights, knots, mults, periodic, .. } => {
            let pts: Option<Vec<[f64; 2]>> = poles.iter().map(|p| at(*p)).collect();
            Geo::Nurbs(bspline_nurbs(&pts?, *degree as usize, weights, knots, mults, *periodic).ok()?)
        }
    })
}

/// A B-spline's exact clamped form from its stored description.
pub fn bspline_nurbs(poles: &[[f64; 2]], degree: usize, weights: &[f64], knots: &[f64], mults: &[u32], periodic: bool) -> Result<Nurbs, String> {
    let n = poles.len();
    if degree < 1 {
        return Err("a B-spline needs a degree of 1 or more".into());
    }
    let w: Vec<f64> = if weights.is_empty() { vec![1.0; n] } else { weights.to_vec() };
    if w.len() != n || w.iter().any(|w| !(*w > 0.0) || !w.is_finite()) {
        return Err("a B-spline needs one positive weight per pole".into());
    }
    if periodic {
        if n < degree + 1 || n < 3 {
            return Err(format!("a closed B-spline of degree {degree} needs at least {} poles", (degree + 1).max(3)));
        }
        return Ok(Nurbs::periodic(degree, poles, &w));
    }
    if n < degree + 1 {
        return Err(format!("a B-spline of degree {degree} needs at least {} poles", degree + 1));
    }
    let flat = if knots.is_empty() {
        let spans = n - degree;
        let mut k = vec![0.0; degree + 1];
        k.extend((1..spans).map(|i| i as f64));
        k.extend(std::iter::repeat_n(spans as f64, degree + 1));
        k
    } else {
        if knots.len() != mults.len() {
            return Err("a B-spline needs one multiplicity per knot".into());
        }
        if knots.windows(2).any(|w| !(w[1] > w[0])) {
            return Err("a B-spline's knots must increase".into());
        }
        if mults.first() != Some(&(degree as u32 + 1)) || mults.last() != Some(&(degree as u32 + 1)) {
            return Err(format!("a B-spline's end knots repeat degree + 1 = {} times (it starts and ends at its end poles)", degree + 1));
        }
        if mults[1..mults.len() - 1].iter().any(|m| *m < 1 || *m as usize > degree) {
            return Err(format!("a B-spline's inner knots repeat 1 to {degree} times"));
        }
        let total: u32 = mults.iter().sum();
        if total as usize != n + degree + 1 {
            return Err(format!("a B-spline with {n} poles of degree {degree} needs knots repeating {} times in all (it has {total})", n + degree + 1));
        }
        Nurbs::flat_knots(knots, mults)
    };
    Ok(Nurbs { degree, poles: poles.to_vec(), weights: w, knots: flat })
}

pub(crate) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
pub(crate) fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
pub(crate) fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
pub(crate) fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub(crate) fn len(a: [f64; 2]) -> f64 {
    dot(a, a).sqrt()
}

// ---- Profiles ------------------------------------------------------------------

/// One edge of a loop, in sketch coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Segment {
    Line { from: [f64; 2], to: [f64; 2] },
    /// A cubic Bézier from `from` to `to` with handles `c1` and `c2`.
    Bezier { from: [f64; 2], to: [f64; 2], c1: [f64; 2], c2: [f64; 2] },
    /// From `from` to `to` around `center`, counter-clockwise when `ccw`.
    Arc { from: [f64; 2], to: [f64; 2], center: [f64; 2], ccw: bool },
    Circle { center: [f64; 2], radius: f64 },
    /// An exact rational B-spline (conics, splines), clamped: from its first pole to its last.
    Nurbs(Nurbs),
}

/// A closed loop: counter-clockwise for an outline, clockwise for a hole.
pub type Loop = Vec<Segment>;

fn seg_points(s: &Segment, tolerance: f64, out: &mut Vec<[f64; 2]>) {
    match s {
        Segment::Line { from, .. } => out.push(*from),
        Segment::Bezier { from, to, c1, c2 } => {
            // Chords within `tolerance`: a cubic's chords stray at most ¾·L/n²,
            // L the larger second difference of its control points.
            let d2 = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| len([p[0] - 2.0 * q[0] + r[0], p[1] - 2.0 * q[1] + r[1]]);
            let l = d2(*from, *c1, *c2).max(d2(*c1, *c2, *to));
            let n = ((0.75 * l / tolerance.max(1e-9)).sqrt().ceil() as usize).clamp(1, 4096);
            for i in 0..n {
                out.push(bezier_at(*from, *c1, *c2, *to, i as f64 / n as f64));
            }
        }
        Segment::Arc { from, to, center, ccw } => {
            let r = dist(*from, *center);
            let a0 = (from[1] - center[1]).atan2(from[0] - center[0]);
            let mut a1 = (to[1] - center[1]).atan2(to[0] - center[0]);
            if *ccw {
                while a1 <= a0 {
                    a1 += 2.0 * std::f64::consts::PI;
                }
            } else {
                while a1 >= a0 {
                    a1 -= 2.0 * std::f64::consts::PI;
                }
            }
            let n = segments_for(r, tolerance);
            let steps = (((a1 - a0).abs() / (2.0 * std::f64::consts::PI)) * n as f64).ceil().max(1.0) as usize;
            for i in 0..steps {
                let a = a0 + (a1 - a0) * i as f64 / steps as f64;
                out.push([center[0] + r * a.cos(), center[1] + r * a.sin()]);
            }
        }
        Segment::Nurbs(n) => {
            let mut pts = n.polyline(tolerance);
            pts.pop();
            out.extend(pts);
        }
        Segment::Circle { center, radius } => {
            // A negative radius marks a clockwise circle (a hole).
            let (r, dir) = (radius.abs(), radius.signum());
            let n = segments_for(r, tolerance);
            for i in 0..n {
                let a = dir * 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                out.push([center[0] + r * a.cos(), center[1] + r * a.sin()]);
            }
        }
    }
}

/// Loops in the exact kernel's profile format: per loop the number of
/// segments, then per segment its kind and numbers (see `occ_sketch_profile`).
pub fn profile_data(loops: &[Loop]) -> Vec<f64> {
    let mut data = Vec::new();
    for l in loops {
        data.push(l.len() as f64);
        for seg in l {
            match seg {
                Segment::Line { from, to } => data.extend([0.0, from[0], from[1], to[0], to[1]]),
                Segment::Bezier { from, to, c1, c2 } => data.extend([3.0, from[0], from[1], c1[0], c1[1], c2[0], c2[1], to[0], to[1]]),
                Segment::Arc { from, to, center, ccw } => data.extend([1.0, from[0], from[1], to[0], to[1], center[0], center[1], if *ccw { 1.0 } else { 0.0 }]),
                Segment::Circle { center, radius } => data.extend([2.0, center[0], center[1], *radius]),
                Segment::Nurbs(n) => {
                    // 4, degree, poles, (x, y, weight) each, distinct knots, then each one's multiplicity.
                    let (k, m) = n.knots_mults();
                    data.extend([4.0, n.degree as f64, n.poles.len() as f64]);
                    for (p, w) in n.poles.iter().zip(&n.weights) {
                        data.extend([p[0], p[1], *w]);
                    }
                    data.push(k.len() as f64);
                    data.extend(&k);
                    data.extend(m.iter().map(|m| *m as f64));
                }
            }
        }
    }
    data
}

/// The point at parameter `t` (0 to 1) of a cubic Bézier.
pub fn bezier_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]]
}

/// Segments for a full circle so chords stay within `tolerance` (as in the prelude).
pub fn segments_for(r: f64, tolerance: f64) -> usize {
    let r = r.abs();
    if r <= tolerance {
        return 8;
    }
    ((std::f64::consts::PI / (1.0 - tolerance / r).acos()).ceil() as usize).clamp(8, 4096)
}

/// A loop as a polygon within `tolerance` of its arcs.
pub fn polygon(l: &Loop, tolerance: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    for s in l {
        seg_points(s, tolerance, &mut out);
    }
    out
}

pub(crate) fn area(poly: &[[f64; 2]]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| cross(poly[i], poly[(i + 1) % n])).sum::<f64>() / 2.0
}

fn contains(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

pub(crate) fn reverse(l: &Loop) -> Loop {
    l.iter()
        .rev()
        .map(|s| match s {
            Segment::Line { from, to } => Segment::Line { from: *to, to: *from },
            Segment::Bezier { from, to, c1, c2 } => Segment::Bezier { from: *to, to: *from, c1: *c2, c2: *c1 },
            Segment::Arc { from, to, center, ccw } => Segment::Arc { from: *to, to: *from, center: *center, ccw: !ccw },
            Segment::Circle { center, radius } => Segment::Circle { center: *center, radius: -radius },
            Segment::Nurbs(n) => Segment::Nurbs(n.reversed()),
        })
        .collect()
}

/// The closed loops of a solved sketch (construction geometry left out),
/// oriented for filling: outlines counter-clockwise, holes clockwise, nested
/// by containment. Open chains are ignored (and reported).
pub fn loops(g: &Geometry, s: &Solved) -> (Vec<Loop>, usize) {
    let (found, open) = loops_of_curves(g, s);
    (found.into_iter().map(|(l, _)| l).collect(), open)
}

/// The closed loops, each with the curves it is made of (as `loops` orients them).
pub fn loops_of_curves(g: &Geometry, s: &Solved) -> (Vec<(Loop, Vec<u32>)>, usize) {
    let pt = |id: u32| s.points.get(&id).copied().unwrap_or([0.0; 2]);
    let mut found: Vec<Loop> = Vec::new();
    let mut found_ids: Vec<Vec<u32>> = Vec::new();
    // Edges between endpoints; coincident points count as one node.
    let mut canon: HashMap<u32, u32> = HashMap::new();
    for c in &g.constraints {
        if let Constraint::Coincident { a, b } = c {
            let (ra, rb) = (root(&canon, *a), root(&canon, *b));
            if ra != rb {
                canon.insert(ra.max(rb), ra.min(rb));
            }
        }
    }
    // Points drawn on top of each other are one node too, even unconstrained.
    let mut ids: Vec<u32> = g.points.iter().map(|p| p.id).collect();
    ids.sort();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            let (pa, pb) = (pt(a), pt(b));
            if (pa[0] - pb[0]).hypot(pa[1] - pb[1]) < 1e-6 {
                let (ra, rb) = (root(&canon, a), root(&canon, b));
                if ra != rb {
                    canon.insert(ra.max(rb), ra.min(rb));
                }
            }
        }
    }
    let node = |id: u32| root(&canon, id);
    struct Edge {
        a: u32,
        b: u32,
        seg: Segment,
        id: u32,
    }
    let mut edges: Vec<Edge> = Vec::new();
    for c in g.curves.iter().filter(|c| !c.construction()) {
        match c {
            // A line of no length (a double click, a stray tap) is not an edge.
            Curve::Line { a, b, .. } if node(*a) == node(*b) => {}
            // A stray spline of no length is no edge either.
            Curve::Line { id, a, b, .. } => edges.push(Edge { a: node(*a), b: node(*b), seg: Segment::Line { from: pt(*a), to: pt(*b) }, id: *id }),
            Curve::Bezier { id, a, b, c1, c2, .. } => edges.push(Edge { a: node(*a), b: node(*b), seg: Segment::Bezier { from: pt(*a), to: pt(*b), c1: pt(*c1), c2: pt(*c2) }, id: *id }),
            Curve::Arc { id, center, start, end, .. } => {
                edges.push(Edge { a: node(*start), b: node(*end), seg: Segment::Arc { from: pt(*start), to: pt(*end), center: pt(*center), ccw: true }, id: *id })
            }
            Curve::Circle { id, center, .. } => {
                let r = s.radii.get(id).copied().unwrap_or(0.0);
                if r > 0.0 {
                    found.push(vec![Segment::Circle { center: pt(*center), radius: r }]);
                    found_ids.push(vec![*id]);
                }
            }
            // Conics and splines go to the kernel as exact rational B-splines.
            other => {
                let Some(geo) = geo_of(other, &|p| s.points.get(&p).copied(), s.radii.get(&other.id()).copied()) else { continue };
                let n = geo.to_nurbs();
                match other.ends() {
                    Some((a, b)) if node(a) != node(b) || !geo.closed() => edges.push(Edge { a: node(a), b: node(b), seg: Segment::Nurbs(n), id: other.id() }),
                    // Closed on itself (a whole ellipse, a periodic spline): a loop of its own.
                    _ if geo.closed() => {
                        found.push(vec![Segment::Nurbs(n)]);
                        found_ids.push(vec![other.id()]);
                    }
                    _ => {}
                }
            }
        }
    }
    // Trim dead ends (strokes that run off an outline and stop) so a walk
    // round the outline never wanders down one; each trimmed chain counts as open.
    let mut used = vec![false; edges.len()];
    let mut open = 0;
    loop {
        let mut degree: HashMap<u32, usize> = HashMap::new();
        for (i, e) in edges.iter().enumerate() {
            if !used[i] && e.a != e.b {
                *degree.entry(e.a).or_default() += 1;
                *degree.entry(e.b).or_default() += 1;
            }
        }
        let ends: Vec<usize> = (0..edges.len()).filter(|&i| !used[i] && edges[i].a != edges[i].b && (degree[&edges[i].a] == 1 || degree[&edges[i].b] == 1)).collect();
        if ends.is_empty() {
            break;
        }
        for i in ends {
            // Count each chain once: where it meets the rest, or is all there is.
            if degree[&edges[i].a] != 2 && degree[&edges[i].b] != 2 {
                open += 1;
            }
            used[i] = true;
        }
    }
    // Walk chains: from an unused edge, follow the other edge at each node.
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut chain = vec![edges[start].seg.clone()];
        let mut ids = vec![edges[start].id];
        let first = edges[start].a;
        let mut at = edges[start].b;
        let mut closed = at == first;
        while !closed {
            let next = (0..edges.len()).find(|&i| !used[i] && (edges[i].a == at || edges[i].b == at));
            let Some(i) = next else { break };
            used[i] = true;
            ids.push(edges[i].id);
            if edges[i].a == at {
                chain.push(edges[i].seg.clone());
                at = edges[i].b;
            } else {
                chain.push(reverse(&vec![edges[i].seg.clone()]).remove(0));
                at = edges[i].a;
            }
            closed = at == first;
        }
        if closed {
            // A loop of no width (curves drawn over each other and back) folds
            // away to nothing: it encloses nothing, so it is no profile.
            let merged = merge_collinear(chain);
            if polygon(&merged, 0.01).len() >= 3 && area(&polygon(&merged, 0.01)).abs() > 1e-12 {
                found.push(merged);
                found_ids.push(ids);
            }
        } else {
            open += 1
        }
    }
    // Orient: outlines CCW, holes CW (a loop inside an odd number of others is a hole).
    let polys: Vec<Vec<[f64; 2]>> = found.iter().map(|l| polygon(l, 0.01)).collect();
    let mut out = Vec::new();
    for (i, l) in found.iter().enumerate() {
        let l = l.clone();
        let probe = polys[i][0];
        let depth = polys.iter().enumerate().filter(|(j, p)| *j != i && contains(p, probe) && area(p).abs() > area(&polys[i]).abs()).count();
        let ccw = area(&polys[i]) > 0.0;
        let want_ccw = depth % 2 == 0;
        out.push((if ccw == want_ccw { l } else { reverse(&l) }, found_ids[i].clone()));
    }
    (out, open)
}

/// A loop with each run of collinear lines made one line, so a side drawn in
/// several strokes is one edge of the solid, and a stroke that runs past a
/// corner and back (a spike of no width) folds away.
fn merge_collinear(chain: Vec<Segment>) -> Vec<Segment> {
    let along = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
        lu > 0.0 && lv > 0.0 && (u[0] * v[1] - u[1] * v[0]).abs() < 1e-9 * lu * lv
    };
    let short = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6;
    let mut out: Vec<Segment> = Vec::with_capacity(chain.len());
    for seg in chain {
        let mut seg = seg;
        // Fold into the previous line while they run along one line.
        while let (Some(Segment::Line { from, to }), Segment::Line { to: next, .. }) = (out.last(), &seg) {
            if !along(*from, *to, *next) {
                break;
            }
            seg = Segment::Line { from: *from, to: *next };
            out.pop();
        }
        if let Segment::Line { from, to } = &seg
            && short(*from, *to)
        {
            continue;
        }
        out.push(seg);
    }
    // The run may wrap round the start.
    while out.len() > 2
        && let (Segment::Line { from: a, to: b }, Segment::Line { from: b0, to: c }) = (&out[out.len() - 1], &out[0])
        && short(*b, *b0)
        && along(*a, *b, *c)
    {
        let (a, c) = (*a, *c);
        out.pop();
        if short(a, c) {
            out.remove(0);
        } else {
            out[0] = Segment::Line { from: a, to: c };
        }
    }
    out
}

fn root(m: &HashMap<u32, u32>, mut id: u32) -> u32 {
    while let Some(next) = m.get(&id) {
        id = *next;
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(id: u32, x: f64, y: f64) -> Point {
        Point { id, x, y }
    }
    fn line(id: u32, a: u32, b: u32) -> Curve {
        Curve::Line { id, a, b, construction: false }
    }

    /// A rough rectangle, drawn by hand, with a width and height and its corner pinned.
    fn rectangle() -> Geometry {
        Geometry {
            points: vec![pt(1, 0.3, -0.2), pt(2, 98.0, 1.0), pt(3, 101.0, 52.0), pt(4, -1.0, 49.0)],
            curves: vec![line(10, 1, 2), line(11, 2, 3), line(12, 3, 4), line(13, 4, 1)],
            constraints: vec![
                Constraint::Fixed { point: 1, x: 0.0, y: 0.0 },
                Constraint::Horizontal { line: 10 },
                Constraint::Horizontal { line: 12 },
                Constraint::Vertical { line: 11 },
                Constraint::Vertical { line: 13 },
                Constraint::Length { line: 10, key: "width".into() },
                Constraint::Length { line: 11, key: "height".into() },
            ],
        }
    }

    fn dims(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn stray_strokes_do_not_spoil_an_outline() {
        // A rectangle whose top was drawn past the corner and back (a spike),
        // with a zero-length tap, and a dead-end chain hanging off one side.
        let g = Geometry {
            points: vec![pt(1, 60.0, 60.0), pt(2, 44.8, 60.0), pt(4, 133.8, 60.0), pt(6, 133.8, 30.0), pt(8, 60.0, 30.0), pt(11, 133.8, 50.0), pt(13, 130.0, 50.0), pt(15, 130.0, 40.0), pt(17, 130.0, 40.0), pt(20, 60.0, 30.0)],
            curves: vec![line(3, 1, 2), line(5, 2, 4), line(7, 4, 6), line(9, 6, 8), line(10, 8, 1), line(12, 4, 11), line(14, 11, 13), line(16, 13, 15), line(18, 15, 17), line(19, 8, 20)],
            constraints: vec![],
        };
        let s = solve(&g, &dims(&[])).unwrap();
        let (loops, open) = loops(&g, &s);
        assert_eq!((loops.len(), open), (1, 1), "one outline; the hanging chain is open");
        assert_eq!(loops[0].len(), 4, "four clean sides: {:?}", loops[0]);
        assert!((area(&polygon(&loops[0], 0.01)) - 73.8 * 30.0).abs() < 1e-6);
    }

    #[test]
    fn a_hand_drawn_rectangle_snaps_to_its_dimensions() {
        let g = rectangle();
        let s = solve(&g, &dims(&[("width", 120.0), ("height", 60.0)])).unwrap();
        assert!(s.residual < 1e-8, "{}", s.residual);
        assert_eq!(s.free, 0, "fully constrained");
        let p3 = s.points[&3];
        assert!((p3[0] - 120.0).abs() < 1e-6 && (p3[1] - 60.0).abs() < 1e-6, "{p3:?}");
        let (loops, open) = loops(&g, &s);
        assert_eq!((loops.len(), open), (1, 0));
        assert!((area(&polygon(&loops[0], 0.01)) - 7200.0).abs() < 1e-6, "an outline, counter-clockwise");
    }

    #[test]
    fn missing_dimensions_leave_freedom_and_conflicts_are_named() {
        let mut g = rectangle();
        g.constraints.pop(); // no height
        let s = solve(&g, &dims(&[("width", 120.0)])).unwrap();
        assert_eq!(s.free, 1, "the height is still free");
        // A vertical side cannot also be horizontal.
        g.constraints.push(Constraint::Horizontal { line: 11 });
        g.constraints.push(Constraint::Length { line: 11, key: "height".into() });
        let s = solve(&g, &dims(&[("width", 120.0), ("height", 60.0)])).unwrap();
        assert!(s.residual > 1e-3);
        assert!(!s.conflicts.is_empty());
    }

    #[test]
    fn holes_and_arcs_become_profiles() {
        // A 100 × 60 plate with a Ø20 hole and one rounded corner (R10 arc).
        let mut g = rectangle();
        g.points.push(pt(20, 50.0, 30.0));
        g.curves.push(Curve::Circle { id: 30, center: 20, radius: 8.0, construction: false });
        g.constraints.push(Constraint::Diameter { curve: 30, key: "hole".into() });
        let s = solve(&g, &dims(&[("width", 100.0), ("height", 60.0), ("hole", 20.0)])).unwrap();
        assert!((s.radii[&30] - 10.0).abs() < 1e-6);
        let (loops, _) = loops(&g, &s);
        assert_eq!(loops.len(), 2);
        let areas: Vec<f64> = loops.iter().map(|l| area(&polygon(l, 0.001))).collect();
        assert!(areas.iter().any(|a| (*a - 6000.0).abs() < 1e-6), "{areas:?}");
        assert!(areas.iter().any(|a| (*a + std::f64::consts::PI * 100.0).abs() < 0.05), "the hole runs clockwise: {areas:?}");
    }

    #[test]
    fn bezier_outlines_solve_and_join_smoothly() {
        // A "D": a straight back from (0,0) to (0,100), and a Bézier bowing out to the right.
        let g = Geometry {
            points: vec![pt(1, 0.0, 0.0), pt(2, 0.0, 100.0), pt(3, 60.0, 110.0), pt(4, 70.0, -5.0)],
            curves: vec![line(10, 1, 2), Curve::Bezier { id: 11, a: 2, b: 1, c1: 3, c2: 4, construction: false }],
            constraints: vec![
                Constraint::Fixed { point: 1, x: 0.0, y: 0.0 },
                Constraint::Fixed { point: 2, x: 0.0, y: 100.0 },
                // Smooth where the curve leaves the top of the back.
                Constraint::Tangent { a: 10, b: 11, at: Some(2) },
            ],
        };
        let s = solve(&g, &dims(&[])).unwrap();
        assert!(s.conflicts.is_empty() && s.residual < 1e-6, "{s:?}");
        // Smooth at the top: the handle leaving (0,100) lies along the back's line (x = 0).
        let c1 = s.points[&3];
        assert!(c1[0].abs() < 1e-6, "{c1:?}");
        let (l, open) = loops(&g, &s);
        assert_eq!((l.len(), open), (1, 0));
        assert!(matches!(l[0].iter().find(|seg| matches!(seg, Segment::Bezier { .. })), Some(_)));
        let poly = polygon(&l[0], 0.01);
        assert!(poly.len() > 10, "the curve is followed closely");
    }

    #[test]
    fn arcs_and_tangents_solve() {
        // A slot: two lines joined by two semicircular arcs, tangent at the joins.
        let g = Geometry {
            points: vec![pt(1, 0.0, 0.0), pt(2, 50.0, 0.0), pt(3, 50.0, 20.0), pt(4, 0.0, 20.0), pt(5, 50.0, 10.0), pt(6, 0.0, 10.0)],
            curves: vec![
                line(10, 1, 2),
                Curve::Arc { id: 11, center: 5, start: 2, end: 3, construction: false },
                line(12, 3, 4),
                Curve::Arc { id: 13, center: 6, start: 4, end: 1, construction: false },
            ],
            constraints: vec![
                Constraint::Fixed { point: 6, x: 0.0, y: 10.0 },
                Constraint::Horizontal { line: 10 },
                Constraint::Horizontal { line: 12 },
                Constraint::Tangent { a: 10, b: 11, at: None },
                Constraint::Tangent { a: 12, b: 11, at: None },
                Constraint::Tangent { a: 12, b: 13, at: None },
                Constraint::Tangent { a: 10, b: 13, at: None },
                Constraint::Radius { curve: 11, key: "r".into() },
                Constraint::Equal { a: 11, b: 13 },
                Constraint::Length { line: 10, key: "l".into() },
            ],
        };
        let s = solve(&g, &dims(&[("r", 12.0), ("l", 60.0)])).unwrap();
        assert!(s.residual < 1e-8, "{}", s.residual);
        assert_eq!(s.free, 0, "every join tangent: fully constrained");
        let (loops, open) = loops(&g, &s);
        assert_eq!((loops.len(), open), (1, 0));
        let a = area(&polygon(&loops[0], 0.0001));
        let exact = 60.0 * 24.0 + std::f64::consts::PI * 144.0;
        assert!((a - exact).abs() < 0.05, "{a} vs {exact}");
    }
}

// ---- In the project ------------------------------------------------------------

use crate::{Param, ResolvedParams, check_params, params};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json,
};

/// A sketch as stored: its plane, geometry, constraints and dimensions.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sketch {
    pub name: String,
    #[serde(default)]
    pub plane: Plane,
    #[serde(default)]
    pub geometry: Geometry,
    /// The dimensions, by key (a constraint's `key`): values, ranges, formulas.
    #[serde(default)]
    pub params: Vec<Param>,
}

impl Persist for Sketch {
    const KIND: &'static str = "shape.sketch";
}

/// Derived: where the constraints put everything, and the closed loops.
#[derive(Component, Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolvedSketch {
    pub solved: Solved,
    pub loops: Vec<Loop>,
    /// Chains that do not close (they make no profile).
    pub open: usize,
}

pub(crate) fn solve_sketch(world: &World, e: Entity) -> Option<Result<SolvedSketch, String>> {
    let s = world.get::<Sketch>(e)?;
    let dims = match world.get::<ResolvedParams>(e) {
        Some(r) => r.0.clone(),
        None => return Some(Err("dimensions not worked out yet".into())),
    };
    // A sketch whose constraints conflict is still solved as nearly as it
    // can be, so the drawing can show which ones conflict; what reads it
    // (programs, boards) refuses it until they are put right.
    Some(solve(&s.geometry, &dims).map(|solved| {
        let (loops, open) = loops(&s.geometry, &solved);
        SolvedSketch { solved, loops, open }
    }))
}

/// Why a solved sketch cannot be used yet: its conflicting constraints, named.
pub fn conflict_error(s: &Sketch, solved: &Solved) -> Option<String> {
    if solved.conflicts.is_empty() {
        return None;
    }
    let names: Vec<String> = solved.conflicts.iter().filter_map(|i| s.geometry.constraints.get(*i)).map(describe).collect();
    Some(format!("these constraints cannot all hold: {}", names.join("; ")))
}

/// A constraint in a few words, for messages.
pub fn describe(c: &Constraint) -> String {
    let v = serde_json::to_value(c).unwrap_or_default();
    let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("constraint").replace('_', " ");
    let ids: Vec<String> = c.ids().iter().map(|i| i.to_string()).collect();
    let key = c.key().map(|k| format!(" “{k}”")).unwrap_or_default();
    format!("{kind}{key} on {}", ids.join(", "))
}

/// A plane from a name ("XY", "XZ", "YZ") and an offset along its normal (mm).
pub fn preset_plane(name: &str, offset: f64) -> Result<Plane, String> {
    let (x, y) = match name {
        "XY" => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        "XZ" => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        "YZ" => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        other => return Err(format!("unknown plane {other} (use XY, XZ or YZ, or give origin, x and y)")),
    };
    let mut p = Plane { origin: [0.0; 3], x, y };
    let n = p.normal();
    p.origin = [n[0] * offset, n[1] * offset, n[2] * offset];
    Ok(p)
}

/// A plane through `origin` square to `normal`, its x along `x` (flattened
/// into the plane) or the world axis lying flattest in it, moved `offset`
/// along the normal.
pub fn plane_from_normal(origin: [f64; 3], normal: [f64; 3], x: Option<[f64; 3]>, offset: f64) -> Result<Plane, String> {
    let l = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let nl = l(normal);
    if !(nl > 1e-12) || origin.iter().any(|v| !v.is_finite()) {
        return Err("a plane needs a finite origin and a normal of some length".into());
    }
    let n = normal.map(|v| v / nl);
    let flat = |v: [f64; 3]| {
        let d = v[0] * n[0] + v[1] * n[1] + v[2] * n[2];
        [v[0] - d * n[0], v[1] - d * n[1], v[2] - d * n[2]]
    };
    let candidates = x.into_iter().chain([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let mut xd = None;
    for c in candidates {
        let f = flat(c);
        if l(f) > 0.1 * l(c).max(1e-12) {
            xd = Some(f.map(|v| v / l(f)));
            break;
        }
    }
    let xd = xd.ok_or("no direction lies in that plane")?;
    let y = [n[1] * xd[2] - n[2] * xd[1], n[2] * xd[0] - n[0] * xd[2], n[0] * xd[1] - n[1] * xd[0]];
    Ok(Plane { origin: [0, 1, 2].map(|k| origin[k] + n[k] * offset), x: xd, y })
}

fn check_plane(p: &Plane) -> Result<(), CommandError> {
    let bad = |m: &str| Err(CommandError::new(ErrorCode::InvalidParameters, m.to_string()));
    let l = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if p.origin.iter().chain(&p.x).chain(&p.y).any(|v| !v.is_finite()) {
        return bad("the plane must be finite");
    }
    if (l(p.x) - 1.0).abs() > 1e-6 || (l(p.y) - 1.0).abs() > 1e-6 || (p.x[0] * p.y[0] + p.x[1] * p.y[1] + p.x[2] * p.y[2]).abs() > 1e-6 {
        return bad("a plane's x and y must be unit length and square to each other");
    }
    Ok(())
}

pub(crate) fn check_geometry(g: &Geometry, params: &[Param]) -> Result<(), CommandError> {
    let bad = |m: String| Err(CommandError::new(ErrorCode::InvalidParameters, m));
    let points: std::collections::HashSet<u32> = g.points.iter().map(|p| p.id).collect();
    if points.len() != g.points.len() {
        return bad("two points share an id".into());
    }
    if g.points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return bad("every point needs finite coordinates".into());
    }
    let mut curves = std::collections::HashSet::new();
    for c in &g.curves {
        if !curves.insert(c.id()) || points.contains(&c.id()) {
            return bad(format!("id {} is used twice", c.id()));
        }
        if let Some(r) = c.size()
            && !(r > 0.0 && r.is_finite())
        {
            return bad(format!("curve {} needs a positive {}", c.id(), if matches!(c, Curve::Circle { .. }) { "radius" } else { "minor semi-axis" }));
        }
        let refs = c.points();
        if let Some(p) = refs.iter().find(|p| !points.contains(p)) {
            return bad(format!("curve {} uses point {p}, which is not in the sketch", c.id()));
        }
        if let Curve::BSpline { poles, degree, weights, knots, mults, periodic, .. } = c {
            let at: Vec<[f64; 2]> = poles.iter().map(|id| g.points.iter().find(|p| p.id == *id).map(|p| [p.x, p.y]).unwrap()).collect();
            bspline_nurbs(&at, *degree as usize, weights, knots, mults, *periodic).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, format!("B-spline {}: {e}", c.id())))?;
        }
    }
    let known: std::collections::HashSet<u32> = points.iter().copied().chain(curves.iter().copied()).collect();
    for c in &g.constraints {
        if let Some(r) = c.ids().into_iter().find(|r| !known.contains(r)) {
            return bad(format!("a constraint ({}) refers to {r}, which is not in the sketch", describe(c)));
        }
    }
    for k in g.constraints.iter().filter_map(|c| c.key()) {
        if !params.iter().any(|p| p.key == k) {
            return bad(format!("a dimension uses “{k}”, which is not one of the sketch's dimensions"));
        }
    }
    Ok(())
}

/// How a sketch's plane is given: a name and offset, or a full plane.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PlaneSpec {
    Named { name: String, #[serde(default)] offset: f64 },
    Full(Plane),
    /// Through `origin`, square to `normal` (e.g. a face of a part: its
    /// centre and outward normal). `x` suggests the sketch's x direction
    /// (default: the world axis lying flattest in the plane).
    Normal {
        origin: [f64; 3],
        normal: [f64; 3],
        #[serde(default)]
        x: Option<[f64; 3]>,
        #[serde(default)]
        offset: f64,
    },
}

impl PlaneSpec {
    fn plane(self) -> Result<Plane, CommandError> {
        let p = match self {
            PlaneSpec::Named { name, offset } => preset_plane(&name, offset).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?,
            PlaneSpec::Full(p) => p,
            PlaneSpec::Normal { origin, normal, x, offset } => plane_from_normal(origin, normal, x, offset).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?,
        };
        check_plane(&p)?;
        Ok(p)
    }
}

/// `sketch.create`: a sketch on a plane, with its geometry, constraints and dimensions.
#[derive(Deserialize, JsonSchema)]
pub struct CreateSketch {
    pub name: String,
    pub plane: Option<PlaneSpec>,
    #[serde(default)]
    pub geometry: Geometry,
    #[serde(default)]
    pub params: Vec<Param>,
}

impl Command for CreateSketch {
    const NAME: &'static str = "sketch.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.name.trim().is_empty() || self.name.contains('.') {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "a sketch needs a name without dots"));
        }
        let plane = match self.plane {
            Some(p) => p.plane()?,
            None => Plane::default(),
        };
        check_params(&self.params)?;
        check_geometry(&self.geometry, &self.params)?;
        let mut sketch = Sketch { name: self.name, plane, geometry: self.geometry, params: self.params };
        let id = tx.create();
        params::link(tx, id, &[], &mut sketch.params)?;
        let dims = params::resolve(tx.world(), &sketch.params).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?;
        solve(&sketch.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        tx.set(id, sketch)?;
        Ok(json!({ "id": id }))
    }
}

/// `sketch.update`: replace any of a sketch's name, plane, geometry and dimensions.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateSketch {
    pub target: StableId,
    pub name: Option<String>,
    pub plane: Option<PlaneSpec>,
    pub geometry: Option<Geometry>,
    pub params: Option<Vec<Param>>,
    /// A point dragged by hand: it stays where it was put while everything
    /// else adapts (a smooth point's other handle swings round, as in a
    /// drawing program), and the result becomes the sketch as drawn.
    pub drag: Option<Drag>,
}

/// Where a point was dragged to (sketch coordinates, mm).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct Drag {
    pub point: u32,
    pub x: f64,
    pub y: f64,
}

impl Command for UpdateSketch {
    const NAME: &'static str = "sketch.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = tx.get::<Sketch>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a sketch", self.target)))?;
        if let Some(n) = self.name {
            if n.trim().is_empty() || n.contains('.') {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "a sketch needs a name without dots"));
            }
            s.name = n;
        }
        if let Some(p) = self.plane {
            s.plane = p.plane()?;
        }
        if let Some(g) = self.geometry {
            s.geometry = g;
        }
        if let Some(mut ps) = self.params {
            check_params(&ps)?;
            let old = s.params.clone();
            params::link(tx, self.target, &old, &mut ps)?;
            s.params = ps;
        }
        check_geometry(&s.geometry, &s.params)?;
        let dims = params::resolve(tx.world(), &s.params).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?;
        if let Some(d) = self.drag {
            // A blocked curve stays where it is.
            let blocked = s.geometry.constraints.iter().any(|c| matches!(c, Constraint::Block { curve } if s.geometry.curves.iter().any(|k| k.id() == *curve && k.points().contains(&d.point))));
            if blocked {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "that point belongs to a blocked curve: unblock it to move it"));
            }
            let p = s.geometry.points.iter_mut().find(|p| p.id == d.point).ok_or_else(|| CommandError::new(ErrorCode::InvalidParameters, format!("no point {}", d.point)))?;
            (p.x, p.y) = (d.x, d.y);
            // Solve with the dragged point held, and a handle's anchor too so
            // the curve turns about it as in a drawing program; if that cannot
            // hold, hold the point alone; if even that cannot (it is pinned or
            // dimensioned elsewhere), the plain solve below decides.
            let anchors: Vec<u32> = s
                .geometry
                .curves
                .iter()
                .filter_map(|c| match c {
                    Curve::Bezier { a, c1, .. } if *c1 == d.point => Some(*a),
                    Curve::Bezier { b, c2, .. } if *c2 == d.point => Some(*b),
                    _ => None,
                })
                .collect();
            let at = |id: u32| s.geometry.points.iter().find(|p| p.id == id).map(|p| (p.x, p.y));
            let mut tries = vec![vec![(d.point, d.x, d.y)]];
            if !anchors.is_empty() {
                tries.insert(0, std::iter::once((d.point, d.x, d.y)).chain(anchors.iter().filter_map(|&a| at(a).map(|(x, y)| (a, x, y)))).collect());
            }
            for fixed in tries {
                let mut held = s.geometry.clone();
                held.constraints.extend(fixed.iter().map(|&(point, x, y)| Constraint::Fixed { point, x, y }));
                if let Ok(r) = solve(&held, &dims)
                    && r.conflicts.is_empty()
                {
                    for p in &mut s.geometry.points {
                        if let Some(q) = r.points.get(&p.id) {
                            (p.x, p.y) = (q[0], q[1]);
                        }
                    }
                    break;
                }
            }
        }
        let solved = solve(&s.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        measure_references(&mut s, &solved);
        tx.set(self.target, s)?;
        Ok(json!({ "id": self.target }))
    }
}

/// Reference dimensions hold what the drawing measures, so whatever reads
/// them (a formula elsewhere) gets the measured value.
pub fn measure_references(s: &mut Sketch, solved: &Solved) {
    for c in &s.geometry.constraints {
        if let Constraint::Reference { of } = c
            && let Some(k) = of.key()
            && let Some(v) = solved.measured.get(k)
            && let Some(p) = s.params.iter_mut().find(|p| p.key == k)
        {
            p.value = (*v * 1e9).round() / 1e9;
            // A measurement has no range to keep within.
            (p.min, p.max) = (None, None);
        }
    }
}

/// The sketches a program's source names in `sketch("…")`.
pub fn named_in(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(i) = rest.find("sketch(") {
        rest = &rest[i + 7..];
        let t = rest.trim_start();
        let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'' || *c == '`') else { continue };
        if let Some(end) = t[1..].find(q) {
            out.push(t[1..1 + end].to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Finds the sketches a program reads, by name, keeping links it already has.
pub fn resolve_uses(tx: &Tx, source: &str, existing: &BTreeMap<String, StableId>) -> Result<BTreeMap<String, StableId>, CommandError> {
    let world = tx.world();
    let mut out = BTreeMap::new();
    for name in named_in(source) {
        if let Some(id) = existing.get(&name).filter(|id| tx.get::<Sketch>(**id).is_some()) {
            out.insert(name, *id);
            continue;
        }
        let found: Vec<StableId> = world
            .resource::<IdIndex>()
            .iter()
            .filter(|(_, e)| world.get::<Sketch>(*e).is_some_and(|s| s.name == name))
            .map(|(id, _)| id)
            .collect();
        match found.as_slice() {
            [id] => {
                out.insert(name, *id);
            }
            [] => return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("there is no sketch called “{name}”"))),
            _ => return Err(CommandError::new(ErrorCode::InvalidParameters, format!("more than one sketch is called “{name}”; rename one"))),
        }
    }
    out.extend(crate::canvas::resolve_uses(tx, source, existing)?);
    Ok(out)
}

/// What a program reads from its sketches, as the runtime receives it (JSON).
pub fn inputs(world: &World, uses: &BTreeMap<String, StableId>) -> Result<String, String> {
    let mut m = serde_json::Map::new();
    for (name, id) in uses.iter().filter(|(n, _)| !n.starts_with(crate::canvas::USE_PREFIX)) {
        let e = world.resource::<IdIndex>().get(*id).ok_or_else(|| format!("sketch “{name}” was deleted"))?;
        let sketch = world.get::<Sketch>(e).ok_or_else(|| format!("“{name}” is not a sketch"))?;
        let solved = world.get::<SolvedSketch>(e).ok_or_else(|| format!("sketch “{name}” has no valid solution"))?;
        if let Some(err) = conflict_error(sketch, &solved.solved) {
            return Err(format!("sketch “{name}”: {err}"));
        }
        m.insert(name.clone(), json!({ "plane": sketch.plane, "loops": solved.loops, "open": solved.open }));
    }
    Ok(serde_json::Value::Object(m).to_string())
}
