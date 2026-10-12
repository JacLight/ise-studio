//! Sketch tools beyond drawing a curve at a time, as a drafting program has
//! them: ready-made shapes (polylines, rectangles centred or rounded,
//! regular polygons, slots and arc slots, circles and arcs through three
//! points, conics, B-splines through points), split, join, B-spline editing
//! (degree, knots, weights, conversion), move / rotate / scale with copies,
//! carbon copies of other sketches (linked through their dimensions), a
//! checker that finds and mends gaps, degenerate and duplicate geometry,
//! removing axis alignment, restoring a conic's or spline's internal
//! geometry, and switching constraints between driving and reference, on and
//! off, and curves to construction.
//!
//! Each is one command, so one undo step, and each leaves a sketch the solver
//! holds together like any other.

use crate::curves::{Geo, Nurbs, interpolate, perp};
use crate::params;
use crate::pattern::PointRef;
use crate::sketch::{Constraint, Curve, Plane, Role, Sketch, cross, dist, dot, len, solve, sub};
use crate::trim::{Ctx, P, bad, piece_of};
use de_core::schemars::{self, JsonSchema};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};

fn unit(v: P) -> P {
    let l = len(v).max(1e-300);
    [v[0] / l, v[1] / l]
}
fn add(p: P, v: P, s: f64) -> P {
    [p[0] + v[0] * s, p[1] + v[1] * s]
}
fn polar(c: P, r: f64, a: f64) -> P {
    [c[0] + r * a.cos(), c[1] + r * a.sin()]
}

/// A point given, or made where it is given.
fn point_of(cx: &mut Ctx, r: &PointRef, made: &mut Vec<u32>) -> Result<(u32, P), CommandError> {
    match r {
        PointRef::Id(id) => Ok((*id, *cx.at.get(id).ok_or_else(|| bad(format!("no point {id}")))?)),
        PointRef::At(p) => {
            if !(p[0].is_finite() && p[1].is_finite()) {
                return Err(bad("points need finite coordinates"));
            }
            let id = cx.point(*p);
            made.push(id);
            Ok((id, *p))
        }
    }
}

/// Where a point is, without making it.
fn place(cx: &Ctx, r: &PointRef) -> Result<P, CommandError> {
    match r {
        PointRef::Id(id) => cx.at.get(id).copied().ok_or_else(|| bad(format!("no point {id}"))),
        PointRef::At(p) => Ok(*p),
    }
}

/// A point at `p` unless the reference names one (then that point).
fn point_at(cx: &mut Ctx, r: &PointRef, p: P, made: &mut Vec<u32>) -> Result<u32, CommandError> {
    match r {
        PointRef::Id(id) => {
            if !cx.at.contains_key(id) {
                return Err(bad(format!("no point {id}")));
            }
            Ok(*id)
        }
        PointRef::At(_) => {
            let id = cx.point(p);
            made.push(id);
            Ok(id)
        }
    }
}

fn level(c: &mut Ctx, line: u32, a: P, b: P) {
    let d = sub(b, a);
    let l = len(d).max(1e-300);
    if d[1].abs() < 1e-9 * l {
        c.sketch.geometry.constraints.push(Constraint::Horizontal { line });
    } else if d[0].abs() < 1e-9 * l {
        c.sketch.geometry.constraints.push(Constraint::Vertical { line });
    }
}

fn line(cx: &mut Ctx, a: u32, b: u32, construction: bool, curves: &mut Vec<u32>) -> u32 {
    let id = cx.id();
    cx.sketch.geometry.curves.push(Curve::Line { id, a, b, construction });
    let (pa, pb) = (cx.p(a), cx.p(b));
    level(cx, id, pa, pb);
    curves.push(id);
    id
}

fn arc(cx: &mut Ctx, center: u32, start: u32, end: u32, construction: bool, curves: &mut Vec<u32>) -> u32 {
    let id = cx.id();
    cx.sketch.geometry.curves.push(Curve::Arc { id, center, start, end, construction });
    curves.push(id);
    id
}

fn tangent(cx: &mut Ctx, a: u32, b: u32, at: u32) {
    cx.sketch.geometry.constraints.push(Constraint::Tangent { a, b, at: Some(at) });
}

/// The centre of the circle through three points.
fn circumcenter(a: P, b: P, c: P) -> Result<P, CommandError> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    let scale = len(sub(b, a)).max(len(sub(c, a))).max(1e-9);
    if d.abs() < 1e-9 * scale * scale {
        return Err(bad("those three points lie on a line: no circle passes through them"));
    }
    let (a2, b2, c2) = (dot(a, a), dot(b, b), dot(c, c));
    Ok([(a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d, (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d])
}

/// A ready-made shape to add to a sketch. Points are a point of the sketch
/// (its id: the shape uses it, so it is joined there) or a place `[x, y]`.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    Point { at: PointRef },
    Line { a: PointRef, b: PointRef },
    /// Lines point to point; `closed` joins the last to the first.
    Polyline {
        points: Vec<PointRef>,
        #[serde(default)]
        closed: bool,
    },
    /// Four lines, level and upright, from two opposite corners.
    Rectangle { corners: [PointRef; 2] },
    /// A rectangle about its centre (opposite corners held symmetric about it).
    CenteredRectangle { center: PointRef, corner: PointRef },
    /// A rectangle with its corners rounded (tangent arcs, one radius dimension).
    RoundedRectangle { corners: [PointRef; 2], radius: f64 },
    /// A regular polygon: `sides` equal sides, corners on a construction
    /// circle about `center`, one corner at `corner`.
    Polygon { center: PointRef, corner: PointRef, sides: u32 },
    /// Two half-circles about `a` and `b` joined by tangent lines; its width (2 × radius) a dimension.
    Slot { a: PointRef, b: PointRef, radius: f64 },
    /// A slot curved round `center`, from `start` to `end` (anticlockwise), `width` wide.
    ArcSlot { center: PointRef, start: PointRef, end: PointRef, width: f64 },
    Circle { center: PointRef, radius: f64 },
    /// The circle through three points.
    Circle3 { points: [PointRef; 3] },
    /// Anticlockwise about `center` from `start` to (towards) `end`.
    Arc { center: PointRef, start: PointRef, end: PointRef },
    /// From `start` to `end` through `through`.
    Arc3 { start: PointRef, end: PointRef, through: PointRef },
    /// About `center`, one semi-axis to `major`; the other `minor` mm, or
    /// through the point `through` (default half the first).
    Ellipse {
        center: PointRef,
        major: PointRef,
        minor: Option<f64>,
        through: Option<[f64; 2]>,
    },
    /// Part of that ellipse, anticlockwise from (towards) `start` to `end`.
    EllipseArc {
        center: PointRef,
        major: PointRef,
        minor: Option<f64>,
        through: Option<[f64; 2]>,
        start: PointRef,
        end: PointRef,
    },
    /// Part of the hyperbola branch about `center` with its vertex at
    /// `major`; `minor` its other semi-axis, or through `through` (default
    /// the first), from (towards) `start` to `end`.
    HyperbolaArc {
        center: PointRef,
        major: PointRef,
        minor: Option<f64>,
        through: Option<[f64; 2]>,
        start: PointRef,
        end: PointRef,
    },
    /// Part of the parabola with this vertex and focus, from (towards) `start` to `end`.
    ParabolaArc { vertex: PointRef, focus: PointRef, start: PointRef, end: PointRef },
    /// A B-spline on these poles (control points); degree default 3 (less
    /// for few poles); `weights` make it rational; `periodic` closes it smoothly.
    Bspline {
        poles: Vec<PointRef>,
        degree: Option<u32>,
        weights: Option<Vec<f64>>,
        #[serde(default)]
        periodic: bool,
    },
    /// A B-spline through these points (held on it), open or closed smoothly.
    Interpolate {
        points: Vec<PointRef>,
        #[serde(default)]
        periodic: bool,
    },
}

/// `sketch.add`: a ready-made shape, constrained as drawn.
#[derive(Deserialize, JsonSchema)]
pub struct AddToSketch {
    pub target: StableId,
    pub shape: Shape,
    /// Draw it as construction (guides that make no outline).
    #[serde(default)]
    pub construction: bool,
}

impl Command for AddToSketch {
    const NAME: &'static str = "sketch.add";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut owned = Ctx::load(tx, self.target)?;
        let (mut pts, mut curves) = (vec![], vec![]);
        let k = self.construction;
        let cx = &mut owned;
        match &self.shape {
            Shape::Point { at } => {
                let p = place(cx, at)?;
                let id = cx.point(p);
                pts.push(id);
            }
            Shape::Line { a, b } => {
                let (a, b) = (point_of(cx, a, &mut pts)?.0, point_of(cx, b, &mut pts)?.0);
                if a == b {
                    return Err(bad("a line needs two different points"));
                }
                line(cx, a, b, k, &mut curves);
            }
            Shape::Polyline { points, closed } => {
                if points.len() < 2 {
                    return Err(bad("a polyline needs two points or more"));
                }
                let ids: Vec<u32> = points.iter().map(|r| point_of(cx, r, &mut pts).map(|x| x.0)).collect::<Result<_, _>>()?;
                for w in ids.windows(2) {
                    if w[0] != w[1] {
                        line(cx, w[0], w[1], k, &mut curves);
                    }
                }
                if *closed && ids.len() > 2 && ids[0] != *ids.last().unwrap() {
                    line(cx, *ids.last().unwrap(), ids[0], k, &mut curves);
                }
            }
            Shape::Rectangle { corners } => {
                let (a, b) = (place(cx, &corners[0])?, place(cx, &corners[1])?);
                if (a[0] - b[0]).abs() < 1e-9 || (a[1] - b[1]).abs() < 1e-9 {
                    return Err(bad("a rectangle needs two corners apart in both directions"));
                }
                let p0 = point_at(cx, &corners[0], a, &mut pts)?;
                let p1 = cx.point([b[0], a[1]]);
                let p2 = point_at(cx, &corners[1], b, &mut pts)?;
                let p3 = cx.point([a[0], b[1]]);
                pts.extend([p1, p3]);
                for (s, e) in [(p0, p1), (p1, p2), (p2, p3), (p3, p0)] {
                    line(cx, s, e, k, &mut curves);
                }
            }
            Shape::CenteredRectangle { center, corner } => {
                let (c, q) = (place(cx, center)?, place(cx, corner)?);
                let d = sub(q, c);
                if d[0].abs() < 1e-9 || d[1].abs() < 1e-9 {
                    return Err(bad("a rectangle needs a corner off both axes through its centre"));
                }
                let cid = point_at(cx, center, c, &mut pts)?;
                let p0 = point_at(cx, corner, q, &mut pts)?;
                let p1 = cx.point([c[0] - d[0], c[1] + d[1]]);
                let p2 = cx.point([c[0] - d[0], c[1] - d[1]]);
                let p3 = cx.point([c[0] + d[0], c[1] - d[1]]);
                pts.extend([p1, p2, p3]);
                for (s, e) in [(p0, p1), (p1, p2), (p2, p3), (p3, p0)] {
                    line(cx, s, e, k, &mut curves);
                }
                cx.sketch.geometry.constraints.push(Constraint::Symmetric { a: p0, b: p2, line: cid });
            }
            Shape::RoundedRectangle { corners, radius } => {
                let (a, b) = (place(cx, &corners[0])?, place(cx, &corners[1])?);
                let (x0, x1, y0, y1) = (a[0].min(b[0]), a[0].max(b[0]), a[1].min(b[1]), a[1].max(b[1]));
                let r = *radius;
                if !(r > 0.0) || 2.0 * r >= (x1 - x0).min(y1 - y0) {
                    return Err(bad("the corner radius must be more than 0 and less than half the shorter side"));
                }
                let mk = |cx: &mut Ctx, p: P, pts: &mut Vec<u32>| {
                    let id = cx.point(p);
                    pts.push(id);
                    id
                };
                // Ends of the four sides, anticlockwise from the bottom.
                let b0 = mk(cx, [x0 + r, y0], &mut pts);
                let b1 = mk(cx, [x1 - r, y0], &mut pts);
                let r0 = mk(cx, [x1, y0 + r], &mut pts);
                let r1 = mk(cx, [x1, y1 - r], &mut pts);
                let t0 = mk(cx, [x1 - r, y1], &mut pts);
                let t1 = mk(cx, [x0 + r, y1], &mut pts);
                let l0 = mk(cx, [x0, y1 - r], &mut pts);
                let l1 = mk(cx, [x0, y0 + r], &mut pts);
                let centres = [[x1 - r, y0 + r], [x1 - r, y1 - r], [x0 + r, y1 - r], [x0 + r, y0 + r]].map(|c| mk(cx, c, &mut pts));
                let bottom = line(cx, b0, b1, k, &mut curves);
                let c0 = arc(cx, centres[0], b1, r0, k, &mut curves);
                let right = line(cx, r0, r1, k, &mut curves);
                let c1 = arc(cx, centres[1], r1, t0, k, &mut curves);
                let top = line(cx, t0, t1, k, &mut curves);
                let c2 = arc(cx, centres[2], t1, l0, k, &mut curves);
                let left = line(cx, l0, l1, k, &mut curves);
                let c3 = arc(cx, centres[3], l1, b0, k, &mut curves);
                for (l, c, at) in [(bottom, c0, b1), (right, c0, r0), (right, c1, r1), (top, c1, t0), (top, c2, t1), (left, c2, l0), (left, c3, l1), (bottom, c3, b0)] {
                    tangent(cx, l, c, at);
                }
                for c in [c1, c2, c3] {
                    cx.sketch.geometry.constraints.push(Constraint::Equal { a: c0, b: c });
                }
                let key = cx.dimension("Corner radius", r, "mm");
                cx.sketch.geometry.constraints.push(Constraint::Radius { curve: c0, key });
            }
            Shape::Polygon { center, corner, sides } => {
                let n = *sides as usize;
                if !(3..=64).contains(&n) {
                    return Err(bad("a polygon has 3 to 64 sides"));
                }
                let (c, q) = (place(cx, center)?, place(cx, corner)?);
                let r = dist(c, q);
                if r < 1e-9 {
                    return Err(bad("the corner must be away from the centre"));
                }
                let cid = point_at(cx, center, c, &mut pts)?;
                let a0 = (q[1] - c[1]).atan2(q[0] - c[0]);
                let mut ids = vec![point_at(cx, corner, q, &mut pts)?];
                for i in 1..n {
                    let id = cx.point(polar(c, r, a0 + TAU * i as f64 / n as f64));
                    pts.push(id);
                    ids.push(id);
                }
                // Equal sides on a circle: no side held level (the polygon turns freely).
                let mut sides = vec![];
                for i in 0..n {
                    let l = cx.id();
                    cx.sketch.geometry.curves.push(Curve::Line { id: l, a: ids[i], b: ids[(i + 1) % n], construction: k });
                    curves.push(l);
                    sides.push(l);
                }
                let circle = cx.id();
                cx.sketch.geometry.curves.push(Curve::Circle { id: circle, center: cid, radius: r, construction: true });
                curves.push(circle);
                for &p in &ids {
                    cx.sketch.geometry.constraints.push(Constraint::OnCurve { point: p, curve: circle });
                }
                for &s in &sides[1..] {
                    cx.sketch.geometry.constraints.push(Constraint::Equal { a: sides[0], b: s });
                }
            }
            Shape::Slot { a, b, radius } => {
                let (pa, pb) = (place(cx, a)?, place(cx, b)?);
                let r = *radius;
                if !(r > 0.0) || dist(pa, pb) < 1e-9 {
                    return Err(bad("a slot needs two centres apart and a radius"));
                }
                let ca = point_at(cx, a, pa, &mut pts)?;
                let cb = point_at(cx, b, pb, &mut pts)?;
                let d = unit(sub(pb, pa));
                let nrm = perp(d);
                let mk = |cx: &mut Ctx, p: P, pts: &mut Vec<u32>| {
                    let id = cx.point(p);
                    pts.push(id);
                    id
                };
                let (a_up, a_dn, b_up, b_dn) = (mk(cx, add(pa, nrm, r), &mut pts), mk(cx, add(pa, nrm, -r), &mut pts), mk(cx, add(pb, nrm, r), &mut pts), mk(cx, add(pb, nrm, -r), &mut pts));
                let low = cx.id();
                cx.sketch.geometry.curves.push(Curve::Line { id: low, a: a_dn, b: b_dn, construction: k });
                curves.push(low);
                // Level or upright along its length when drawn so (the other side follows by tangency).
                level(cx, low, add(pa, nrm, -r), add(pb, nrm, -r));
                let cap_b = arc(cx, cb, b_dn, b_up, k, &mut curves);
                let high = cx.id();
                cx.sketch.geometry.curves.push(Curve::Line { id: high, a: b_up, b: a_up, construction: k });
                curves.push(high);
                let cap_a = arc(cx, ca, a_up, a_dn, k, &mut curves);
                for (l, c, at) in [(low, cap_b, b_dn), (high, cap_b, b_up), (high, cap_a, a_up), (low, cap_a, a_dn)] {
                    tangent(cx, l, c, at);
                }
                cx.sketch.geometry.constraints.push(Constraint::Equal { a: cap_a, b: cap_b });
                let key = cx.dimension("Slot width", 2.0 * r, "mm");
                cx.sketch.geometry.constraints.push(Constraint::Diameter { curve: cap_a, key });
            }
            Shape::ArcSlot { center, start, end, width } => {
                let (c, s, e) = (place(cx, center)?, place(cx, start)?, place(cx, end)?);
                let big = dist(c, s);
                let w = *width;
                if !(w > 0.0) || w >= 2.0 * big {
                    return Err(bad("an arc slot needs a width more than 0 and less than twice its radius"));
                }
                let (t0, mut t1) = ((s[1] - c[1]).atan2(s[0] - c[0]), (e[1] - c[1]).atan2(e[0] - c[0]));
                while t1 <= t0 + 1e-9 {
                    t1 += TAU;
                }
                let cid = point_at(cx, center, c, &mut pts)?;
                let h = w / 2.0;
                let mk = |cx: &mut Ctx, p: P, pts: &mut Vec<u32>| {
                    let id = cx.point(p);
                    pts.push(id);
                    id
                };
                let cs = mk(cx, polar(c, big, t0), &mut pts);
                let ce = mk(cx, polar(c, big, t1), &mut pts);
                let (os, oe, is, ie) = (mk(cx, polar(c, big + h, t0), &mut pts), mk(cx, polar(c, big + h, t1), &mut pts), mk(cx, polar(c, big - h, t0), &mut pts), mk(cx, polar(c, big - h, t1), &mut pts));
                let outer = arc(cx, cid, os, oe, k, &mut curves);
                let cap_e = arc(cx, ce, oe, ie, k, &mut curves);
                let inner = arc(cx, cid, is, ie, k, &mut curves);
                let cap_s = arc(cx, cs, is, os, k, &mut curves);
                // The caps' centres stay on one circle about the centre.
                arc(cx, cid, cs, ce, true, &mut curves);
                for (a, b, at) in [(outer, cap_e, oe), (cap_e, inner, ie), (inner, cap_s, is), (cap_s, outer, os)] {
                    tangent(cx, a, b, at);
                }
                cx.sketch.geometry.constraints.push(Constraint::Equal { a: cap_s, b: cap_e });
                let key = cx.dimension("Slot width", w, "mm");
                cx.sketch.geometry.constraints.push(Constraint::Diameter { curve: cap_e, key });
            }
            Shape::Circle { center, radius } => {
                if !(*radius > 0.0) {
                    return Err(bad("a circle needs a positive radius"));
                }
                let c = point_of(cx, center, &mut pts)?.0;
                let id = cx.id();
                cx.sketch.geometry.curves.push(Curve::Circle { id, center: c, radius: *radius, construction: k });
                curves.push(id);
            }
            Shape::Circle3 { points } => {
                let q: Vec<P> = points.iter().map(|r| place(cx, r)).collect::<Result<_, _>>()?;
                let c = circumcenter(q[0], q[1], q[2])?;
                let cid = cx.point(c);
                pts.push(cid);
                let id = cx.id();
                cx.sketch.geometry.curves.push(Curve::Circle { id, center: cid, radius: dist(c, q[0]), construction: k });
                curves.push(id);
                // Points of the sketch it was drawn through stay on it.
                for r in points {
                    if let PointRef::Id(p) = r {
                        cx.sketch.geometry.constraints.push(Constraint::OnCurve { point: *p, curve: id });
                    }
                }
            }
            Shape::Arc { center, start, end } => {
                let (c, s, e) = (place(cx, center)?, place(cx, start)?, place(cx, end)?);
                let r = dist(c, s);
                if r < 1e-9 {
                    return Err(bad("an arc's start must be away from its centre"));
                }
                let cid = point_at(cx, center, c, &mut pts)?;
                let sid = point_at(cx, start, s, &mut pts)?;
                let eid = point_at(cx, end, polar(c, r, (e[1] - c[1]).atan2(e[0] - c[0])), &mut pts)?;
                arc(cx, cid, sid, eid, k, &mut curves);
            }
            Shape::Arc3 { start, end, through } => {
                let (s, e, t) = (place(cx, start)?, place(cx, end)?, place(cx, through)?);
                let c = circumcenter(s, e, t)?;
                let cid = cx.point(c);
                pts.push(cid);
                let sid = point_at(cx, start, s, &mut pts)?;
                let eid = point_at(cx, end, e, &mut pts)?;
                // Anticlockwise from start to end if that passes through `through`, else the other way.
                let ang = |p: P| (p[1] - c[1]).atan2(p[0] - c[0]);
                let (at, a1) = ((ang(t) - ang(s)).rem_euclid(TAU), (ang(e) - ang(s)).rem_euclid(TAU));
                let (from, to) = if at < a1 { (sid, eid) } else { (eid, sid) };
                let id = arc(cx, cid, from, to, k, &mut curves);
                if let PointRef::Id(p) = through {
                    cx.sketch.geometry.constraints.push(Constraint::OnCurve { point: *p, curve: id });
                }
            }
            Shape::Ellipse { center, major, minor, through } | Shape::EllipseArc { center, major, minor, through, .. } | Shape::HyperbolaArc { center, major, minor, through, .. } => {
                let hyperbola = matches!(self.shape, Shape::HyperbolaArc { .. });
                let (c, m) = (place(cx, center)?, place(cx, major)?);
                let a = dist(c, m);
                if a < 1e-9 {
                    return Err(bad("the axis point must be away from the centre"));
                }
                let u = unit(sub(m, c));
                let local = |p: P| (dot(sub(p, c), u), cross(u, sub(p, c)));
                let b = match (minor, through) {
                    (Some(b), _) => *b,
                    (None, Some(t)) => {
                        let (x, y) = local(*t);
                        let q = if hyperbola { x * x / (a * a) - 1.0 } else { 1.0 - x * x / (a * a) };
                        if q <= 1e-9 || y.abs() < 1e-9 {
                            return Err(bad(if hyperbola { "that point is not beyond the vertex, off the axis: no hyperbola passes through it" } else { "that point is not within the major axis, off it: no ellipse passes through it" }));
                        }
                        y.abs() / q.sqrt()
                    }
                    (None, None) => if hyperbola { a } else { a / 2.0 },
                };
                if !(b > 0.0) {
                    return Err(bad("the other semi-axis must be positive"));
                }
                let cid = point_at(cx, center, c, &mut pts)?;
                let mid = point_at(cx, major, m, &mut pts)?;
                let id = cx.id();
                let curve = match &self.shape {
                    Shape::Ellipse { .. } => Curve::Ellipse { id, center: cid, major: mid, minor: b, construction: k },
                    Shape::EllipseArc { start, end, .. } | Shape::HyperbolaArc { start, end, .. } => {
                        let v = perp(u);
                        let on = |p: P| -> P {
                            let (x, y) = local(p);
                            if hyperbola {
                                let t = (y / b).asinh();
                                add(add(c, u, a * t.cosh()), v, b * t.sinh())
                            } else {
                                let t = (y / b).atan2(x / a);
                                add(add(c, u, a * t.cos()), v, b * t.sin())
                            }
                        };
                        let (s, e) = (place(cx, start)?, place(cx, end)?);
                        let sid = point_at(cx, start, on(s), &mut pts)?;
                        let eid = point_at(cx, end, on(e), &mut pts)?;
                        if sid == eid {
                            return Err(bad("an arc needs two different ends"));
                        }
                        if hyperbola {
                            Curve::HyperbolaArc { id, center: cid, major: mid, minor: b, start: sid, end: eid, construction: k }
                        } else {
                            Curve::EllipseArc { id, center: cid, major: mid, minor: b, start: sid, end: eid, construction: k }
                        }
                    }
                    _ => unreachable!(),
                };
                cx.sketch.geometry.curves.push(curve);
                curves.push(id);
            }
            Shape::ParabolaArc { vertex, focus, start, end } => {
                let (v, f) = (place(cx, vertex)?, place(cx, focus)?);
                let fl = dist(v, f);
                if fl < 1e-9 {
                    return Err(bad("the focus must be away from the vertex"));
                }
                let u = unit(sub(f, v));
                let on = |p: P| -> P {
                    let s = cross(u, sub(p, v));
                    add(add(v, u, s * s / (4.0 * fl)), perp(u), s)
                };
                let vid = point_at(cx, vertex, v, &mut pts)?;
                let fid = point_at(cx, focus, f, &mut pts)?;
                let (s, e) = (place(cx, start)?, place(cx, end)?);
                let sid = point_at(cx, start, on(s), &mut pts)?;
                let eid = point_at(cx, end, on(e), &mut pts)?;
                if sid == eid {
                    return Err(bad("an arc needs two different ends"));
                }
                let id = cx.id();
                cx.sketch.geometry.curves.push(Curve::ParabolaArc { id, vertex: vid, focus: fid, start: sid, end: eid, construction: k });
                curves.push(id);
            }
            Shape::Bspline { poles, degree, weights, periodic } => {
                let n = poles.len();
                let degree = degree.unwrap_or(3.min(n.saturating_sub(1) as u32).max(1));
                let ids: Vec<u32> = poles.iter().map(|r| point_of(cx, r, &mut pts).map(|x| x.0)).collect::<Result<_, _>>()?;
                let id = cx.id();
                cx.sketch.geometry.curves.push(Curve::BSpline { id, poles: ids, degree, weights: weights.clone().unwrap_or_default(), knots: vec![], mults: vec![], periodic: *periodic, construction: k });
                curves.push(id);
            }
            Shape::Interpolate { points, periodic } => {
                let at: Vec<P> = points.iter().map(|r| place(cx, r)).collect::<Result<_, _>>()?;
                let (degree, poles, knots) = interpolate(&at, *periodic).map_err(bad)?;
                let through: Vec<u32> = points.iter().zip(&at).map(|(r, p)| point_at(cx, r, *p, &mut pts)).collect::<Result<_, _>>()?;
                let id = cx.id();
                let curve = if *periodic {
                    let ids: Vec<u32> = poles.iter().map(|q| cx.point(*q)).collect();
                    pts.extend(&ids);
                    Curve::BSpline { id, poles: ids, degree: degree as u32, weights: vec![], knots: vec![], mults: vec![], periodic: true, construction: k }
                } else {
                    let n = Nurbs::new(degree, poles, knots);
                    let (a, b) = (through[0], *through.last().unwrap());
                    let c = cx.spline(id, &n, a, b, k);
                    pts.extend(c.points().iter().filter(|p| **p != a && **p != b));
                    c
                };
                cx.sketch.geometry.curves.push(curve);
                curves.push(id);
                // The points it was drawn through stay on it (its ends are its own).
                let ends = (through[0], *through.last().unwrap());
                for &p in &through {
                    if *periodic || (p != ends.0 && p != ends.1) {
                        cx.sketch.geometry.constraints.push(Constraint::OnCurve { point: p, curve: id });
                    }
                }
            }
        }
        let target = self.target;
        let out = json!({ "id": target, "curves": curves, "points": pts });
        owned.save(tx, target, &BTreeSet::new(), false, out)
    }
}

/// `sketch.split`: a curve cut in two at a point on it (a circle or ellipse
/// into two halves, a closed spline opened there).
#[derive(Deserialize, JsonSchema)]
pub struct SplitSketch {
    pub target: StableId,
    pub curve: u32,
    /// Where (sketch mm): the nearest point on the curve.
    pub at: [f64; 2],
}

impl Command for SplitSketch {
    const NAME: &'static str = "sketch.split";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let curve = cx.sketch.geometry.curves.iter().find(|c| c.id() == self.curve).cloned().ok_or_else(|| bad(format!("no curve {}", self.curve)))?;
        let geo = cx.geo(&curve).ok_or_else(|| bad("that curve cannot be worked out"))?;
        let (t0, t1) = geo.range();
        let t = geo.closest(self.at, false);
        let closed = geo.closed() || curve.ends().is_some_and(|(a, b)| a == b);
        if !closed && ((t - t0).abs() < 1e-9 * (t1 - t0).abs().max(1.0) || (t1 - t).abs() < 1e-9 * (t1 - t0).abs().max(1.0)) {
            return Err(bad("split inside the curve, not at its end"));
        }
        let id = curve.id();
        let k = curve.construction();
        let p = geo.point(t);
        let m = cx.point(p);
        let mut made = vec![id];
        let replace = |cx: &mut Ctx, c: Curve| {
            let g = &mut cx.sketch.geometry;
            let i = g.curves.iter().position(|x| x.id() == id).unwrap();
            g.curves[i] = c;
        };
        match &curve {
            Curve::Line { a, b, .. } => {
                replace(&mut cx, Curve::Line { id, a: *a, b: m, construction: k });
                let nid = cx.id();
                cx.sketch.geometry.curves.push(Curve::Line { id: nid, a: m, b: *b, construction: k });
                // The second half keeps the first's level or upright.
                let lv: Vec<Constraint> = cx
                    .sketch
                    .geometry
                    .constraints
                    .iter()
                    .filter_map(|c| match c {
                        Constraint::Horizontal { line } if *line == id => Some(Constraint::Horizontal { line: nid }),
                        Constraint::Vertical { line } if *line == id => Some(Constraint::Vertical { line: nid }),
                        _ => None,
                    })
                    .collect();
                if lv.is_empty() {
                    // Still one line.
                    cx.sketch.geometry.constraints.push(Constraint::Parallel { a: id, b: nid });
                }
                cx.sketch.geometry.constraints.extend(lv);
                made.push(nid);
            }
            Curve::Arc { center, start, end, .. } => {
                replace(&mut cx, Curve::Arc { id, center: *center, start: *start, end: m, construction: k });
                let nid = cx.id();
                cx.sketch.geometry.curves.push(Curve::Arc { id: nid, center: *center, start: m, end: *end, construction: k });
                made.push(nid);
            }
            Curve::Circle { center, .. } => {
                let o = cx.point(geo.point(t + PI));
                replace(&mut cx, Curve::Arc { id, center: *center, start: m, end: o, construction: k });
                let nid = cx.id();
                cx.sketch.geometry.curves.push(Curve::Arc { id: nid, center: *center, start: o, end: m, construction: k });
                made.push(nid);
            }
            Curve::Ellipse { center, major, .. } => {
                let o = cx.point(geo.point(t + PI));
                let minor = cx.radii.get(&id).copied().unwrap_or(1.0);
                replace(&mut cx, Curve::EllipseArc { id, center: *center, major: *major, minor, start: m, end: o, construction: k });
                let nid = cx.id();
                cx.sketch.geometry.curves.push(Curve::EllipseArc { id: nid, center: *center, major: *major, minor, start: o, end: m, construction: k });
                cx.sketch.geometry.constraints.push(Constraint::Equal { a: id, b: nid });
                made.push(nid);
            }
            Curve::Bezier { a, b, c1, c2, .. } => {
                let ctrl = [cx.p(*a), cx.p(*c1), cx.p(*c2), cx.p(*b)];
                let (l, r) = split_bezier(ctrl, t);
                let (h1, h2, h3, h4) = (cx.point(l[1]), cx.point(l[2]), cx.point(r[1]), cx.point(r[2]));
                replace(&mut cx, Curve::Bezier { id, a: *a, b: m, c1: h1, c2: h2, construction: k });
                let nid = cx.id();
                cx.sketch.geometry.curves.push(Curve::Bezier { id: nid, a: m, b: *b, c1: h3, c2: h4, construction: k });
                // Smooth where it was cut.
                cx.sketch.geometry.constraints.push(Constraint::Tangent { a: id, b: nid, at: Some(m) });
                made.push(nid);
            }
            Curve::BSpline { periodic: true, .. } => {
                let c = piece_of(&mut cx, &curve, &geo, id, m, m, t, t, k);
                replace(&mut cx, c);
            }
            _ => {
                let (s, e) = curve.ends().ok_or_else(|| bad("that curve cannot be split"))?;
                let first = piece_of(&mut cx, &curve, &geo, id, s, m, t0, t, k);
                replace(&mut cx, first);
                let nid = cx.id();
                let second = piece_of(&mut cx, &curve, &geo, nid, m, e, t, t1, k);
                let sized = second.size().is_some();
                cx.sketch.geometry.curves.push(second);
                if sized {
                    cx.sketch.geometry.constraints.push(Constraint::Equal { a: id, b: nid });
                }
                if matches!(curve, Curve::BSpline { .. }) {
                    cx.sketch.geometry.constraints.push(Constraint::Tangent { a: id, b: nid, at: Some(m) });
                }
                made.push(nid);
            }
        }
        let target = self.target;
        cx.finish_with(tx, target, id, json!({ "id": target, "curves": made, "point": m }))
    }
}

fn split_bezier(p: [P; 4], t: f64) -> ([P; 4], [P; 4]) {
    let lerp = |a: P, b: P| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let (a, b, c) = (lerp(p[0], p[1]), lerp(p[1], p[2]), lerp(p[2], p[3]));
    let (d, e) = (lerp(a, b), lerp(b, c));
    let f = lerp(d, e);
    ([p[0], a, d, f], [f, e, c, p[3]])
}

/// A curve as an exact B-spline (lines, arcs, conics and Béziers too).
fn nurbs_of(cx: &Ctx, c: &Curve) -> Result<Nurbs, CommandError> {
    let g = cx.geo(c).ok_or_else(|| bad("that curve cannot be worked out"))?;
    Ok(match g {
        Geo::Nurbs(n) => n,
        other => other.to_nurbs(),
    })
}

/// `sketch.join`: two curves that meet end to end become one B-spline.
#[derive(Deserialize, JsonSchema)]
pub struct JoinSketch {
    pub target: StableId,
    pub curves: [u32; 2],
}

impl Command for JoinSketch {
    const NAME: &'static str = "sketch.join";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let find = |id: u32| cx.sketch.geometry.curves.iter().find(|c| c.id() == id).cloned().ok_or_else(|| bad(format!("no curve {id}")));
        let (a, b) = (find(self.curves[0])?, find(self.curves[1])?);
        if a.id() == b.id() {
            return Err(bad("join two different curves"));
        }
        let ((s1, e1), (s2, e2)) = (a.ends().ok_or_else(|| bad("a closed curve has no end to join"))?, b.ends().ok_or_else(|| bad("a closed curve has no end to join"))?);
        let (na, nb) = (nurbs_of(&cx, &a)?, nurbs_of(&cx, &b)?);
        let near = |p: u32, q: u32| p == q || dist(cx.p(p), cx.p(q)) < 1e-6;
        let (first, second, start, end) = if near(e1, s2) {
            (na, nb, s1, e2)
        } else if near(e1, e2) {
            (na, nb.reversed(), s1, s2)
        } else if near(s1, s2) {
            (na.reversed(), nb, e1, e2)
        } else if near(s1, e2) {
            (nb, na, s2, e1)
        } else {
            return Err(bad("the two curves must meet end to end"));
        };
        let joined = crate::curves::join(&first, &second);
        let id = a.id();
        let c = cx.spline(id, &joined, start, end, a.construction() && b.construction());
        let g = &mut cx.sketch.geometry;
        g.curves.retain(|c| c.id() != b.id());
        let i = g.curves.iter().position(|x| x.id() == id).unwrap();
        g.curves[i] = c;
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target, "curve": id }))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SplineOp {
    IncreaseDegree,
    DecreaseDegree,
    /// A knot where the curve passes nearest `at`.
    InsertKnot,
    /// Knot `knot` (counting distinct knots from 0) repeated once more (the curve less smooth there).
    IncreaseMultiplicity,
    /// Knot `knot` repeated once less (removed at 1): the nearest curve.
    DecreaseMultiplicity,
    /// Pole `pole` (its index along the spline) weighted `weight`.
    SetWeight,
    /// Any curve turned into a B-spline of the same shape.
    Convert,
}

/// `sketch.bspline`: edit a B-spline's degree, knots or weights, or turn a curve into one.
#[derive(Deserialize, JsonSchema)]
pub struct EditSpline {
    pub target: StableId,
    pub curve: u32,
    pub op: SplineOp,
    pub at: Option<[f64; 2]>,
    pub knot: Option<u32>,
    pub pole: Option<u32>,
    pub weight: Option<f64>,
}

impl Command for EditSpline {
    const NAME: &'static str = "sketch.bspline";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let curve = cx.sketch.geometry.curves.iter().find(|c| c.id() == self.curve).cloned().ok_or_else(|| bad(format!("no curve {}", self.curve)))?;
        let id = curve.id();
        let k = curve.construction();
        let is_spline = matches!(curve, Curve::BSpline { .. });
        if !is_spline && self.op != SplineOp::Convert {
            return Err(bad("that is not a B-spline (convert it first)"));
        }
        if is_spline && self.op == SplineOp::Convert {
            return Err(bad("that is a B-spline already"));
        }
        if self.op == SplineOp::SetWeight {
            let (i, w) = (self.pole.ok_or_else(|| bad("give the pole's index"))? as usize, self.weight.ok_or_else(|| bad("give the weight"))?);
            if !(w > 0.0 && w.is_finite()) {
                return Err(bad("a weight must be positive"));
            }
            for c in &mut cx.sketch.geometry.curves {
                if let Curve::BSpline { id: cid, poles, weights, .. } = c
                    && *cid == id
                {
                    if i >= poles.len() {
                        return Err(bad(format!("the spline has {} poles", poles.len())));
                    }
                    if weights.is_empty() {
                        *weights = vec![1.0; poles.len()];
                    }
                    weights[i] = w;
                }
            }
            let target = self.target;
            return cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target, "curve": id }));
        }
        let n = nurbs_of(&cx, &curve)?;
        let next = match self.op {
            SplineOp::IncreaseDegree => n.elevated(),
            SplineOp::DecreaseDegree => n.lowered().ok_or_else(|| bad("a degree 1 spline cannot go lower"))?,
            SplineOp::InsertKnot => {
                let at = self.at.ok_or_else(|| bad("give where (`at`) to insert the knot"))?;
                let u = Geo::Nurbs(n.clone()).closest(at, false);
                let (a, b) = n.domain();
                if u <= a + 1e-9 * (b - a) || u >= b - 1e-9 * (b - a) {
                    return Err(bad("insert a knot inside the curve, not at its end"));
                }
                if n.multiplicity(u) >= n.degree {
                    return Err(bad("that knot already repeats as often as the degree allows"));
                }
                n.insert(u)
            }
            SplineOp::IncreaseMultiplicity | SplineOp::DecreaseMultiplicity => {
                let (knots, mults) = n.knots_mults();
                let i = self.knot.ok_or_else(|| bad("give the knot's index"))? as usize;
                if i == 0 || i + 1 >= knots.len() {
                    return Err(bad(format!("give an inner knot: 1 to {}", knots.len().saturating_sub(2))));
                }
                if self.op == SplineOp::IncreaseMultiplicity {
                    if mults[i] as usize >= n.degree {
                        return Err(bad("that knot already repeats as often as the degree allows"));
                    }
                    n.insert(knots[i])
                } else {
                    let mut m = mults.clone();
                    m[i] -= 1;
                    let (k2, m2): (Vec<f64>, Vec<u32>) = knots.iter().copied().zip(m).filter(|(_, m)| *m > 0).unzip();
                    n.refit(n.degree, Nurbs::flat_knots(&k2, &m2))
                }
            }
            SplineOp::SetWeight => unreachable!(),
            SplineOp::Convert => n,
        };
        // Ends stay the points they were (a closed curve gets one point for both).
        let (s, e) = match curve.ends() {
            Some(ends) => ends,
            None => {
                let p = cx.point(next.poles[0]);
                (p, p)
            }
        };
        let c = cx.spline(id, &next, s, e, k);
        let g = &mut cx.sketch.geometry;
        let i = g.curves.iter().position(|x| x.id() == id).unwrap();
        g.curves[i] = c;
        // What held the old shape's size or centre does not apply to a spline.
        g.constraints.retain(|c| {
            !matches!(c.inner(), Constraint::Radius { curve, .. } | Constraint::Diameter { curve, .. } if *curve == id)
                && !matches!(c.inner(), Constraint::Length { line, .. } if *line == id)
                && !matches!(c.inner(), Constraint::Horizontal { line } | Constraint::Vertical { line } if *line == id)
                && !matches!(c.inner(), Constraint::Equal { a, b } | Constraint::Parallel { a, b } | Constraint::Perpendicular { a, b, .. } if *a == id || *b == id)
                && !matches!(c.inner(), Constraint::Midpoint { line, .. } if *line == id)
                && !matches!(c.inner(), Constraint::InternalAlignment { curve, .. } if *curve == id)
        });
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target, "curve": id }))
    }
}

/// `sketch.transform`: move, turn and scale curves and points (about
/// `center`, default the middle of what is chosen), in place or as copies.
#[derive(Deserialize, JsonSchema)]
pub struct TransformSketch {
    pub target: StableId,
    #[serde(default)]
    pub curves: Vec<u32>,
    /// Loose points to move too.
    #[serde(default)]
    pub points: Vec<u32>,
    #[serde(rename = "move")]
    pub translate: Option<[f64; 2]>,
    /// Degrees, anticlockwise.
    pub rotate: Option<f64>,
    pub scale: Option<f64>,
    pub center: Option<[f64; 2]>,
    /// 0 (default): move them. n: make n copies, each moved once more than the last.
    #[serde(default)]
    pub copies: u32,
}

impl Command for TransformSketch {
    const NAME: &'static str = "sketch.transform";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let g = &cx.sketch.geometry;
        let chosen: Vec<Curve> = self.curves.iter().map(|id| g.curves.iter().find(|c| c.id() == *id).cloned().ok_or_else(|| bad(format!("no curve {id}")))).collect::<Result<_, _>>()?;
        let mut points: Vec<u32> = vec![];
        for p in chosen.iter().flat_map(|c| c.points()).chain(self.points.iter().copied()) {
            if !cx.at.contains_key(&p) {
                return Err(bad(format!("no point {p}")));
            }
            if !points.contains(&p) {
                points.push(p);
            }
        }
        if points.is_empty() {
            return Err(bad("choose curves or points to transform"));
        }
        let scale = self.scale.unwrap_or(1.0);
        if !(scale > 0.0 && scale.is_finite()) {
            return Err(bad("scale by a positive factor"));
        }
        if self.copies > 200 {
            return Err(bad("at most 200 copies"));
        }
        let rot = self.rotate.unwrap_or(0.0);
        let mv = self.translate.unwrap_or([0.0, 0.0]);
        let centre = self.center.unwrap_or_else(|| {
            let xs: Vec<f64> = points.iter().map(|p| cx.p(*p)[0]).collect();
            let ys: Vec<f64> = points.iter().map(|p| cx.p(*p)[1]).collect();
            let mm = |v: &[f64]| (v.iter().copied().fold(f64::INFINITY, f64::min) + v.iter().copied().fold(f64::NEG_INFINITY, f64::max)) / 2.0;
            [mm(&xs), mm(&ys)]
        });
        // Step k: scale and turn about the centre, then move; k times over.
        let step = |p: P, k: u32| -> P {
            let mut q = p;
            for _ in 0..k {
                let d = sub(q, centre);
                let (s, c) = rot.to_radians().sin_cos();
                let r = [(d[0] * c - d[1] * s) * scale, (d[0] * s + d[1] * c) * scale];
                q = [centre[0] + r[0] + mv[0], centre[1] + r[1] + mv[1]];
            }
            q
        };
        // Level and upright lines after turning k steps: kept, swapped or let go.
        let turned = |k: u32| ((rot * k as f64).rem_euclid(180.0) / 90.0).round() as i64 % 2 == 1;
        let square = |k: u32| ((rot * k as f64) / 90.0 - ((rot * k as f64) / 90.0).round()).abs() < 1e-9;
        let in_set: BTreeSet<u32> = points.iter().copied().chain(chosen.iter().map(Curve::id)).collect();
        let is_internal = |c: &Constraint| c.ids().iter().all(|i| in_set.contains(i));
        let size_dims = |c: &Constraint| matches!(c.inner(), Constraint::Distance { .. } | Constraint::Length { .. } | Constraint::Radius { .. } | Constraint::Diameter { .. } | Constraint::HorizontalDistance { .. } | Constraint::VerticalDistance { .. });
        let mut made_curves = vec![];
        let mut made_points = vec![];
        let mut linked = vec![];
        if self.copies == 0 {
            for p in &points {
                let q = step(cx.p(*p), 1);
                cx.at.insert(*p, q);
                if let Some(pt) = cx.sketch.geometry.points.iter_mut().find(|x| x.id == *p) {
                    (pt.x, pt.y) = (q[0], q[1]);
                }
            }
            for c in &mut cx.sketch.geometry.curves {
                if in_set.contains(&c.id())
                    && let Some(r) = c.size()
                {
                    c.set_size(r * scale);
                }
            }
            let mut keep = vec![];
            for c in std::mem::take(&mut cx.sketch.geometry.constraints) {
                match &c {
                    Constraint::Fixed { point, .. } if in_set.contains(point) => {
                        let q = cx.p(*point);
                        keep.push(Constraint::Fixed { point: *point, x: q[0], y: q[1] });
                    }
                    Constraint::Horizontal { line } | Constraint::Vertical { line } if in_set.contains(line) && !square(1) => {}
                    Constraint::Horizontal { line } if in_set.contains(line) && turned(1) => keep.push(Constraint::Vertical { line: *line }),
                    Constraint::Vertical { line } if in_set.contains(line) && turned(1) => keep.push(Constraint::Horizontal { line: *line }),
                    _ => keep.push(c),
                }
            }
            cx.sketch.geometry.constraints = keep;
            if (scale - 1.0).abs() > 1e-12 {
                for c in cx.sketch.geometry.constraints.clone() {
                    if is_internal(&c) && size_dims(&c) && let Some(key) = c.key() {
                        if let Some(p) = cx.sketch.params.iter_mut().find(|p| p.key == key) {
                            if p.expr.is_some() {
                                linked.push(p.label.clone());
                            } else {
                                p.value *= scale;
                                cx.dims.insert(key.to_string(), p.value);
                            }
                        }
                    }
                }
            }
        } else {
            let internal: Vec<Constraint> = cx.sketch.geometry.constraints.iter().filter(|c| is_internal(c)).cloned().collect();
            for k in 1..=self.copies {
                let mut map = BTreeMap::new();
                for p in &points {
                    let q = step(cx.p(*p), k);
                    let id = cx.point(q);
                    map.insert(*p, id);
                    made_points.push(id);
                }
                let mut cmap = BTreeMap::new();
                for c in &chosen {
                    let nid = cx.id();
                    let mut copy = c.mapped(nid, &|p| map[&p]);
                    if let Some(r) = cx.radii.get(&c.id()) {
                        copy.set_size(r * scale.powi(k as i32));
                    }
                    cmap.insert(c.id(), nid);
                    cx.sketch.geometry.curves.push(copy);
                    made_curves.push(nid);
                }
                let f = |i: u32| map.get(&i).or_else(|| cmap.get(&i)).copied().unwrap_or(i);
                for c in &internal {
                    let mut c = match c.mapped(&f) {
                        Constraint::Fixed { point, .. } => {
                            let q = cx.p(point);
                            Constraint::Fixed { point, x: q[0], y: q[1] }
                        }
                        Constraint::Horizontal { .. } | Constraint::Vertical { .. } if !square(k) => continue,
                        Constraint::Horizontal { line } if turned(k) => Constraint::Vertical { line },
                        Constraint::Vertical { line } if turned(k) => Constraint::Horizontal { line },
                        other => other,
                    };
                    // Dimensions: their own copies, scaled.
                    if let Some(key) = c.key().map(str::to_string) {
                        let p = cx.sketch.params.iter().find(|p| p.key == key).cloned();
                        let Some(p) = p else { continue };
                        let v = if size_dims(&c) { cx.dims.get(&key).copied().unwrap_or(p.value) * scale.powi(k as i32) } else { cx.dims.get(&key).copied().unwrap_or(p.value) };
                        let label = p.label.split(' ').next().unwrap_or("Dimension").to_string();
                        let nk = cx.dimension(&label, v, &p.unit);
                        c = rekey(&c, &nk);
                    }
                    cx.sketch.geometry.constraints.push(c);
                }
            }
        }
        let target = self.target;
        let out = json!({ "id": target, "curves": made_curves, "points": made_points, "linked_kept": linked });
        cx.save(tx, target, &BTreeSet::new(), false, out)
    }
}

/// The same dimension under another key.
fn rekey(c: &Constraint, key: &str) -> Constraint {
    let mut v = serde_json::to_value(c).unwrap();
    fn walk(v: &mut serde_json::Value, key: &str) {
        if let serde_json::Value::Object(m) = v {
            if m.contains_key("key") {
                m.insert("key".into(), json!(key));
            }
            if let Some(of) = m.get_mut("of") {
                walk(of, key);
            }
        }
    }
    walk(&mut v, key);
    serde_json::from_value(v).unwrap()
}

/// A sketch, or several.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Sources {
    One(StableId),
    Many(Vec<StableId>),
}

/// `sketch.carbon_copy`: another sketch's geometry, constraints and
/// dimensions brought in (several sketches: merged). Linked (default), the
/// copies' dimensions are formulas on the source's, so they follow when it
/// changes. The planes must be parallel; the copy lands where the source
/// lies, seen square to this sketch.
#[derive(Deserialize, JsonSchema)]
pub struct CarbonCopy {
    pub target: StableId,
    pub from: Sources,
    #[serde(default = "yes")]
    pub linked: bool,
}

fn yes() -> bool {
    true
}

impl Command for CarbonCopy {
    const NAME: &'static str = "sketch.carbon_copy";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let target = self.target;
        let sources = match self.from {
            Sources::One(s) => vec![s],
            Sources::Many(v) => v,
        };
        let mut cx = Ctx::load(tx, target)?;
        let old_params = cx.sketch.params.clone();
        let mut made = vec![];
        let mut dropped = 0;
        for src in sources {
            if src == target {
                return Err(bad("a sketch cannot copy itself"));
            }
            let s = tx.get::<Sketch>(src).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{src} is not a sketch")))?;
            let dims = params::resolve(tx.world(), &s.params).map_err(bad)?;
            let solved = solve(&s.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
            // The map from the source's plane to this one: an isometry when the planes are parallel.
            let (from, to): (&Plane, Plane) = (&s.plane, cx.sketch.plane.clone());
            let lin = |v: P| {
                let w = [from.x[0] * v[0] + from.y[0] * v[1], from.x[1] * v[0] + from.y[1] * v[1], from.x[2] * v[0] + from.y[2] * v[1]];
                [w[0] * to.x[0] + w[1] * to.x[1] + w[2] * to.x[2], w[0] * to.y[0] + w[1] * to.y[1] + w[2] * to.y[2]]
            };
            let (mx, my) = (lin([1.0, 0.0]), lin([0.0, 1.0]));
            if (len(mx) - 1.0).abs() > 1e-6 || (len(my) - 1.0).abs() > 1e-6 || dot(mx, my).abs() > 1e-6 {
                return Err(bad(format!("“{}” lies on a plane not parallel to this sketch's: copy from a parallel one (or project it)", s.name)));
            }
            let mirrored = cross(mx, my) < 0.0;
            let axis_kept = mx[1].abs() < 1e-9;
            let axis_swapped = mx[0].abs() < 1e-9;
            let map_pt = |p: P| to.to_local(from.to_world(p));
            let mut ids = BTreeMap::new();
            for p in &s.geometry.points {
                let q = map_pt(solved.points.get(&p.id).copied().unwrap_or([p.x, p.y]));
                let id = cx.point(q);
                ids.insert(p.id, id);
            }
            for c in &s.geometry.curves {
                let nid = cx.id();
                ids.insert(c.id(), nid);
                let mut copy = c.mapped(nid, &|p| ids.get(&p).copied().unwrap_or(p));
                if mirrored {
                    copy = copy.flipped();
                }
                if let Some(r) = solved.radii.get(&c.id()) {
                    copy.set_size(*r);
                }
                cx.sketch.geometry.curves.push(copy);
                made.push(nid);
            }
            let f = |i: u32| ids.get(&i).copied().unwrap_or(i);
            for c in &s.geometry.constraints {
                let mut c = c.mapped(&f);
                // What depends on the sketch's axes follows the map between them.
                let fix_axes = |c: Constraint| -> Option<Constraint> {
                    Some(match c {
                        Constraint::Horizontal { line } if axis_swapped => Constraint::Vertical { line },
                        Constraint::Vertical { line } if axis_swapped => Constraint::Horizontal { line },
                        Constraint::Horizontal { .. } | Constraint::Vertical { .. } if !axis_kept && !axis_swapped => return None,
                        Constraint::HorizontalDistance { a, b, ref key } | Constraint::VerticalDistance { a, b, ref key } if a != b => {
                            let key = key.clone();
                            let horiz = matches!(c, Constraint::HorizontalDistance { .. });
                            if !axis_kept && !axis_swapped {
                                return None;
                            }
                            // The axis it now runs along, and whether it runs backwards.
                            let along = if horiz { mx } else { my };
                            let now_horiz = if axis_kept { horiz } else { !horiz };
                            let backwards = along[if now_horiz { 0 } else { 1 }] < 0.0;
                            let (a, b) = if backwards { (b, a) } else { (a, b) };
                            if now_horiz { Constraint::HorizontalDistance { a, b, key } } else { Constraint::VerticalDistance { a, b, key } }
                        }
                        Constraint::Angle { a, b, key } if mirrored && a != b => Constraint::Angle { a: b, b: a, key },
                        Constraint::Fixed { point, .. } => {
                            let q = cx.p(point);
                            Constraint::Fixed { point, x: q[0], y: q[1] }
                        }
                        other => other,
                    })
                };
                c = match c {
                    Constraint::Reference { of } => match fix_axes(*of) {
                        Some(i) => Constraint::Reference { of: Box::new(i) },
                        None => {
                            dropped += 1;
                            continue;
                        }
                    },
                    Constraint::Inactive { of } => match fix_axes(*of) {
                        Some(i) => Constraint::Inactive { of: Box::new(i) },
                        None => {
                            dropped += 1;
                            continue;
                        }
                    },
                    other => match fix_axes(other) {
                        Some(i) => i,
                        None => {
                            dropped += 1;
                            continue;
                        }
                    },
                };
                if let Some(key) = c.key().map(str::to_string) {
                    let Some(p) = s.params.iter().find(|p| p.key == key) else { continue };
                    let label = p.label.split(' ').next().unwrap_or("Dimension").to_string();
                    let value = dims.get(&key).copied().unwrap_or(p.value);
                    let nk = cx.dimension(&label, value, &p.unit);
                    // A point's own X or Y moves with the plane's origin: it cannot follow by formula.
                    let coordinate = matches!(c.inner(), Constraint::HorizontalDistance { a, b, .. } | Constraint::VerticalDistance { a, b, .. } if a == b);
                    if self.linked && !coordinate && !matches!(c, Constraint::Reference { .. }) {
                        if let Some(q) = cx.sketch.params.iter_mut().find(|q| q.key == nk) {
                            q.expr = Some(format!("={}.{}", s.name, key));
                        }
                    }
                    c = rekey(&c, &nk);
                }
                cx.sketch.geometry.constraints.push(c);
            }
        }
        // Formulas to ids, and this sketch now reading its sources.
        let mut ps = cx.sketch.params.clone();
        params::link(tx, target, &old_params, &mut ps)?;
        cx.sketch.params = ps;
        cx.dims = params::resolve(tx.world(), &cx.sketch.params).map_err(bad)?;
        cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target, "curves": made, "dropped": dropped }))
    }
}

/// `sketch.validate`: what is amiss in a sketch, and with `fix`, mended:
/// ends that nearly meet (joined), curves of no size (removed), curves drawn
/// twice (one removed), constraints given twice (one removed), and with
/// `remove_redundant` the constraints that add nothing.
#[derive(Deserialize, JsonSchema)]
pub struct ValidateSketch {
    pub target: StableId,
    #[serde(default)]
    pub fix: bool,
    #[serde(default)]
    pub remove_redundant: bool,
    /// How near ends must be to count as meant to meet (mm, default 0.01).
    pub tolerance: Option<f64>,
}

impl Command for ValidateSketch {
    const NAME: &'static str = "sketch.validate";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let tol = self.tolerance.unwrap_or(0.01).max(1e-9);
        let mut cx = Ctx::load(tx, self.target)?;
        let dims = cx.dims.clone();
        let solved = solve(&cx.sketch.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        let g = cx.sketch.geometry.clone();
        // Ends joined already: by sharing a point or a coincidence.
        let mut join: BTreeMap<u32, u32> = BTreeMap::new();
        fn root(m: &BTreeMap<u32, u32>, mut i: u32) -> u32 {
            while let Some(n) = m.get(&i) {
                i = *n;
            }
            i
        }
        for c in &g.constraints {
            if let Constraint::Coincident { a, b } = c.inner() {
                let (ra, rb) = (root(&join, *a), root(&join, *b));
                if ra != rb {
                    join.insert(ra.max(rb), ra.min(rb));
                }
            }
        }
        let ends: Vec<u32> = {
            let mut v: Vec<u32> = g.curves.iter().filter_map(|c| c.ends()).flat_map(|(a, b)| [a, b]).collect();
            v.sort();
            v.dedup();
            v
        };
        let mut gaps = vec![];
        for (i, &a) in ends.iter().enumerate() {
            for &b in &ends[i + 1..] {
                let d = dist(cx.p(a), cx.p(b));
                if d < tol && root(&join, a) != root(&join, b) {
                    gaps.push((a, b, d));
                }
            }
        }
        // Curves of no size.
        let mut degenerate = vec![];
        for c in &g.curves {
            let tiny = match c {
                Curve::Line { a, b, .. } => dist(cx.p(*a), cx.p(*b)) < tol,
                Curve::Arc { center, start, end, .. } => dist(cx.p(*center), cx.p(*start)) < tol || dist(cx.p(*start), cx.p(*end)) < tol && *start != *end,
                Curve::Circle { id, .. } | Curve::Ellipse { id, .. } => cx.radii.get(id).is_none_or(|r| *r < tol),
                _ => cx.geo(c).is_none_or(|geo| {
                    let pts = geo.sample(32);
                    pts.windows(2).map(|w| dist(w[0], w[1])).sum::<f64>() < tol
                }),
            };
            if tiny {
                degenerate.push(c.id());
            }
        }
        // Curves drawn twice: the same kind through the same places.
        let sig = |c: &Curve| -> Option<(String, Vec<[i64; 2]>)> {
            let kind = serde_json::to_value(c).ok()?.get("kind")?.as_str()?.to_string();
            let q = |p: P| [(p[0] / tol).round() as i64, (p[1] / tol).round() as i64];
            let mut pts: Vec<[i64; 2]> = c.points().iter().map(|p| q(cx.p(*p))).collect();
            if matches!(c, Curve::Line { .. }) {
                pts.sort();
            }
            if let Some(r) = cx.radii.get(&c.id()) {
                pts.push([(r / tol).round() as i64, 0]);
            }
            Some((kind, pts))
        };
        let mut seen: Vec<(String, Vec<[i64; 2]>)> = vec![];
        let mut duplicates = vec![];
        for c in &g.curves {
            if let Some(s) = sig(c) {
                if seen.contains(&s) {
                    duplicates.push(c.id());
                } else {
                    seen.push(s);
                }
            }
        }
        // Constraints given twice (either way round).
        let norm = |c: &Constraint| -> String {
            let swap = |c: &Constraint| match c {
                Constraint::Coincident { a, b } => Constraint::Coincident { a: *a.min(b), b: *a.max(b) },
                Constraint::Parallel { a, b } => Constraint::Parallel { a: *a.min(b), b: *a.max(b) },
                Constraint::Equal { a, b } => Constraint::Equal { a: *a.min(b), b: *a.max(b) },
                Constraint::Perpendicular { a, b, at } => Constraint::Perpendicular { a: *a.min(b), b: *a.max(b), at: *at },
                Constraint::Tangent { a, b, at } => Constraint::Tangent { a: *a.min(b), b: *a.max(b), at: *at },
                other => other.clone(),
            };
            serde_json::to_string(&swap(c)).unwrap_or_default()
        };
        let mut duplicate_constraints = vec![];
        let mut seen_c: Vec<String> = vec![];
        for (i, c) in g.constraints.iter().enumerate() {
            if c.key().is_some() {
                continue;
            }
            let n = norm(c);
            if seen_c.contains(&n) {
                duplicate_constraints.push(i);
            } else {
                seen_c.push(n);
            }
        }
        // Ends no other curve meets.
        let mut degree: BTreeMap<u32, usize> = BTreeMap::new();
        for c in g.curves.iter().filter(|c| !c.construction()) {
            if let Some((a, b)) = c.ends() {
                *degree.entry(root(&join, a)).or_default() += 1;
                *degree.entry(root(&join, b)).or_default() += 1;
            }
        }
        let open_ends: Vec<u32> = g.curves.iter().filter(|c| !c.construction()).filter_map(|c| c.ends()).flat_map(|(a, b)| [a, b]).filter(|p| degree.get(&root(&join, *p)) == Some(&1)).collect();
        let mut report = json!({
            "id": self.target,
            "gaps": gaps.iter().map(|(a, b, d)| json!({ "a": a, "b": b, "distance": d })).collect::<Vec<_>>(),
            "degenerate": degenerate,
            "duplicates": duplicates,
            "duplicate_constraints": duplicate_constraints,
            "open_ends": open_ends,
            "conflicts": solved.conflicts,
            "redundant": solved.redundant,
            "partially_redundant": solved.partially_redundant,
            "malformed": solved.malformed,
            "free": solved.free,
        });
        if !self.fix && !self.remove_redundant {
            return Ok(report);
        }
        let mut drop_constraints: BTreeSet<usize> = BTreeSet::new();
        if self.fix {
            drop_constraints.extend(duplicate_constraints.iter().copied());
            drop_constraints.extend(solved.malformed.iter().copied());
        }
        if self.remove_redundant {
            drop_constraints.extend(solved.redundant.iter().copied());
        }
        let removed_constraints = drop_constraints.len();
        cx.sketch.geometry.constraints = cx.sketch.geometry.constraints.iter().enumerate().filter(|(i, _)| !drop_constraints.contains(i)).map(|(_, c)| c.clone()).collect();
        let mut joined = 0;
        let mut removed_curves = 0;
        if self.fix {
            let gone: BTreeSet<u32> = degenerate.iter().chain(&duplicates).copied().collect();
            removed_curves = gone.len();
            cx.sketch.geometry.curves.retain(|c| !gone.contains(&c.id()));
            for (a, b, _) in &gaps {
                if cx.sketch.geometry.curves.iter().any(|c| c.points().contains(a)) && cx.sketch.geometry.curves.iter().any(|c| c.points().contains(b)) {
                    cx.sketch.geometry.constraints.push(Constraint::Coincident { a: *a, b: *b });
                    joined += 1;
                }
            }
        }
        report["fixed"] = json!({ "joined": joined, "removed_curves": removed_curves, "removed_constraints": removed_constraints });
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), false, report)
    }
}

/// `sketch.unalign`: the chosen lines let go of the sketch's axes but keep
/// their angles to one another (level and upright become parallel and
/// perpendicular to the first), so the group can turn as one.
#[derive(Deserialize, JsonSchema)]
pub struct Unalign {
    pub target: StableId,
    pub curves: Vec<u32>,
}

impl Command for Unalign {
    const NAME: &'static str = "sketch.unalign";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let set: BTreeSet<u32> = self.curves.iter().copied().collect();
        let g = &cx.sketch.geometry;
        let axis: Vec<(u32, bool)> = g
            .constraints
            .iter()
            .filter_map(|c| match c {
                Constraint::Horizontal { line } if set.contains(line) => Some((*line, true)),
                Constraint::Vertical { line } if set.contains(line) => Some((*line, false)),
                _ => None,
            })
            .collect();
        let Some(&(reference, ref_level)) = axis.first() else {
            return Err(bad("none of those lines is held level or upright"));
        };
        let mut out = vec![];
        for c in std::mem::take(&mut cx.sketch.geometry.constraints) {
            match &c {
                Constraint::Horizontal { line } | Constraint::Vertical { line } if set.contains(line) => {
                    if *line != reference {
                        let level = matches!(c, Constraint::Horizontal { .. });
                        out.push(if level == ref_level { Constraint::Parallel { a: reference, b: *line } } else { Constraint::Perpendicular { a: reference, b: *line, at: None } });
                    }
                }
                _ => out.push(c),
            }
        }
        cx.sketch.geometry.constraints = out;
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target, "released": axis.len() }))
    }
}

/// `sketch.internal`: a conic's axes and foci, or a B-spline's knots, as
/// construction geometry held to it (what is there already is kept).
#[derive(Deserialize, JsonSchema)]
pub struct RestoreInternal {
    pub target: StableId,
    pub curve: u32,
}

impl Command for RestoreInternal {
    const NAME: &'static str = "sketch.internal";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let curve = cx.sketch.geometry.curves.iter().find(|c| c.id() == self.curve).cloned().ok_or_else(|| bad(format!("no curve {}", self.curve)))?;
        let id = curve.id();
        let have: Vec<(Role, Option<u32>)> = cx
            .sketch
            .geometry
            .constraints
            .iter()
            .filter_map(|c| match c {
                Constraint::InternalAlignment { curve, role, index, .. } if *curve == id => Some((*role, *index)),
                _ => None,
            })
            .collect();
        let (mut curves, mut points) = (vec![], vec![]);
        let geo = cx.geo(&curve).ok_or_else(|| bad("that curve cannot be worked out"))?;
        match (&curve, &geo) {
            (Curve::Ellipse { .. } | Curve::EllipseArc { .. } | Curve::HyperbolaArc { .. }, Geo::Ellipse(c, u, a, b, ..) | Geo::Hyperbola(c, u, a, b, ..)) => {
                let hyperbola = matches!(geo, Geo::Hyperbola(..));
                let v = perp(*u);
                let e = if hyperbola { (a * a + b * b).sqrt() } else { (a * a - b * b).max(0.0).sqrt() };
                for (role, from, to) in [(Role::MajorAxis, add(*c, *u, -a), add(*c, *u, *a)), (Role::MinorAxis, add(*c, v, -b), add(*c, v, *b))] {
                    if have.contains(&(role, None)) {
                        continue;
                    }
                    let (p, q) = (cx.point(from), cx.point(to));
                    let l = cx.id();
                    cx.sketch.geometry.curves.push(Curve::Line { id: l, a: p, b: q, construction: true });
                    cx.sketch.geometry.constraints.push(Constraint::InternalAlignment { curve: id, element: l, role, index: None });
                    curves.push(l);
                    points.extend([p, q]);
                }
                for (role, s) in [(Role::Focus1, e), (Role::Focus2, -e)] {
                    if have.contains(&(role, None)) {
                        continue;
                    }
                    let p = cx.point(add(*c, *u, s));
                    cx.sketch.geometry.constraints.push(Constraint::InternalAlignment { curve: id, element: p, role, index: None });
                    points.push(p);
                }
            }
            (Curve::ParabolaArc { vertex, focus, .. }, _) => {
                // Its vertex and focus are points of its own; the axis joins them.
                let exists = cx.sketch.geometry.curves.iter().any(|c| matches!(c, Curve::Line { a, b, .. } if (*a == *vertex && *b == *focus) || (*a == *focus && *b == *vertex)));
                if !exists {
                    let l = cx.id();
                    cx.sketch.geometry.curves.push(Curve::Line { id: l, a: *vertex, b: *focus, construction: true });
                    curves.push(l);
                }
            }
            (Curve::BSpline { .. }, Geo::Nurbs(n)) => {
                let (knots, _) = n.knots_mults();
                for (i, k) in knots.iter().enumerate() {
                    if have.contains(&(Role::Knot, Some(i as u32))) {
                        continue;
                    }
                    let q = n.point(*k);
                    // The ends are poles already: a knot point there would only repeat them.
                    if i == 0 || i + 1 == knots.len() {
                        continue;
                    }
                    let p = cx.point(q);
                    cx.sketch.geometry.constraints.push(Constraint::InternalAlignment { curve: id, element: p, role: Role::Knot, index: Some(i as u32) });
                    points.push(p);
                }
            }
            _ => return Err(bad("internal geometry is for ellipses, hyperbolas, parabolas and B-splines")),
        }
        let target = self.target;
        cx.save(tx, target, &points.iter().copied().collect(), false, json!({ "id": target, "curves": curves, "points": points }))
    }
}

/// `sketch.constraint`: make a dimension driving or a reference (measured
/// only), or switch any constraint off or on, by its index.
#[derive(Deserialize, JsonSchema)]
pub struct ToggleConstraint {
    pub target: StableId,
    pub index: usize,
    pub driving: Option<bool>,
    pub active: Option<bool>,
}

impl Command for ToggleConstraint {
    const NAME: &'static str = "sketch.constraint";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let c = cx.sketch.geometry.constraints.get(self.index).cloned().ok_or_else(|| bad(format!("no constraint {}", self.index)))?;
        // Unwrap to (active, reference, the constraint).
        let (mut active, mut reference, mut inner) = (true, false, c.clone());
        loop {
            match inner {
                Constraint::Inactive { of } => {
                    active = false;
                    inner = *of;
                }
                Constraint::Reference { of } => {
                    reference = true;
                    inner = *of;
                }
                _ => break,
            }
        }
        if let Some(d) = self.driving {
            if inner.key().is_none() {
                return Err(bad("only dimensions can be references"));
            }
            if d && reference {
                // Driving from what it measures now, so nothing jumps.
                let solved = solve(&cx.sketch.geometry, &cx.dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
                let key = inner.key().unwrap().to_string();
                if let (Some(v), Some(p)) = (solved.measured.get(&key), cx.sketch.params.iter_mut().find(|p| p.key == key)) {
                    p.value = *v;
                    cx.dims.insert(key, *v);
                }
            }
            reference = !d;
        }
        if let Some(a) = self.active {
            active = a;
        }
        let mut next = inner;
        if reference {
            next = Constraint::Reference { of: Box::new(next) };
        }
        if !active {
            next = Constraint::Inactive { of: Box::new(next) };
        }
        cx.sketch.geometry.constraints[self.index] = next;
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target }))
    }
}

/// `sketch.construction`: curves turned into construction (guides that make
/// no outline) or back; with `on` left out, each one switched over.
#[derive(Deserialize, JsonSchema)]
pub struct SetConstruction {
    pub target: StableId,
    pub curves: Vec<u32>,
    pub on: Option<bool>,
}

impl Command for SetConstruction {
    const NAME: &'static str = "sketch.construction";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        for id in &self.curves {
            let c = cx.sketch.geometry.curves.iter_mut().find(|c| c.id() == *id).ok_or_else(|| bad(format!("no curve {id}")))?;
            let on = self.on.unwrap_or(!c.construction());
            c.set_construction(on);
        }
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target }))
    }
}
