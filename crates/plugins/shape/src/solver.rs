//! The sketch solver, in the manner of FreeCAD's planegcs (LGPL; written
//! afresh here, not copied): every constraint is a few equations in the
//! points' coordinates and the curves' sizes; the sketch splits into
//! independent groups (what no constraint links solves apart), and each
//! group is solved from where it is drawn by damped Gauss–Newton steps of
//! least length, so the drawing moves as little as it must.
//!
//! At the solution the equations' Jacobian, taken a constraint at a time in
//! the order they were added, says the rest: its rank gives the degrees of
//! freedom left; a constraint whose rows add no rank is redundant (others
//! already fix what it would), or part-redundant when only some of them do;
//! where those rows cannot be met, the constraints they depend on conflict;
//! and a coordinate the rows do not reach is free to move.

use crate::curves::{Geo, perp};
use crate::sketch::{Constraint, Curve, Geometry, Role, Solved, cross, dist, dot, geo_of, len, sub};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::{PI, TAU};

type P = [f64; 2];

/// Rows of an equation at or below this (mm) are met.
const MET: f64 = 1e-6;

/// Unknowns: every point's x and y, every circle's radius and conic's minor semi-axis.
struct Vars {
    point: HashMap<u32, usize>,
    size: HashMap<u32, usize>,
    values: Vec<f64>,
}

fn vars(g: &Geometry) -> Vars {
    let mut v = Vars { point: HashMap::new(), size: HashMap::new(), values: Vec::new() };
    for p in &g.points {
        v.point.insert(p.id, v.values.len());
        v.values.extend([p.x, p.y]);
    }
    for c in &g.curves {
        if let Some(s) = c.size() {
            v.size.insert(c.id(), v.values.len());
            v.values.push(s);
        }
    }
    v
}

fn unit(v: P) -> P {
    let l = len(v).max(1e-12);
    [v[0] / l, v[1] / l]
}

/// A conic in its own frame: centre (vertex for a parabola), major axis
/// direction, and its sizes (a, b; for a parabola a is the focal length).
struct Conic {
    kind: ConicKind,
    c: P,
    u: P,
    a: f64,
    b: f64,
}

#[derive(PartialEq)]
enum ConicKind {
    Ellipse,
    Hyperbola,
    Parabola,
}

impl Conic {
    fn local(&self, p: P) -> (f64, f64) {
        let d = sub(p, self.c);
        (dot(d, self.u), cross(self.u, d))
    }
    /// Zero on the curve, about the distance off it (mm) near it.
    fn implicit(&self, p: P) -> f64 {
        let (x, y) = self.local(p);
        let (a, b) = (self.a, self.b);
        match self.kind {
            ConicKind::Ellipse => (x * x / (a * a) + y * y / (b * b) - 1.0) * (a * b / (a + b)),
            ConicKind::Hyperbola => (x * x / (a * a) - y * y / (b * b) - 1.0) * (a / 2.0),
            ConicKind::Parabola => x - y * y / (4.0 * a),
        }
    }
    /// The direction along the curve at (or near) `p`: square to its gradient.
    fn tangent(&self, p: P) -> P {
        let (x, y) = self.local(p);
        let (a, b) = (self.a, self.b);
        let (gx, gy) = match self.kind {
            ConicKind::Ellipse => (x / (a * a), y / (b * b)),
            ConicKind::Hyperbola => (x / (a * a), -y / (b * b)),
            ConicKind::Parabola => (1.0, -y / (2.0 * a)),
        };
        // Local tangent (−gy, gx), back to the sketch.
        let v = perp(self.u);
        let (tx, ty) = (-gy, gx);
        [self.u[0] * tx + v[0] * ty, self.u[1] * tx + v[1] * ty]
    }
}

struct Ctx<'a> {
    vars: &'a Vars,
    dims: &'a BTreeMap<String, f64>,
    curves: HashMap<u32, &'a Curve>,
    /// Where everything was drawn (what a blocked curve holds to).
    x0: &'a [f64],
}

impl Ctx<'_> {
    fn p(&self, x: &[f64], id: u32) -> Option<P> {
        let i = *self.vars.point.get(&id)?;
        Some([x[i], x[i + 1]])
    }
    fn is_point(&self, id: u32) -> bool {
        self.vars.point.contains_key(&id)
    }
    fn size(&self, x: &[f64], id: u32) -> Option<f64> {
        self.vars.size.get(&id).map(|i| x[*i])
    }
    fn line(&self, x: &[f64], id: u32) -> Option<(P, P)> {
        match self.curves.get(&id)? {
            Curve::Line { a, b, .. } => Some((self.p(x, *a)?, self.p(x, *b)?)),
            _ => None,
        }
    }
    /// Center and radius of a circle or arc.
    fn round(&self, x: &[f64], id: u32) -> Option<(P, f64)> {
        match self.curves.get(&id)? {
            Curve::Circle { id, center, .. } => Some((self.p(x, *center)?, self.size(x, *id)?)),
            Curve::Arc { center, start, .. } => {
                let c = self.p(x, *center)?;
                Some((c, dist(c, self.p(x, *start)?)))
            }
            _ => None,
        }
    }
    fn conic(&self, x: &[f64], id: u32) -> Option<Conic> {
        match self.curves.get(&id)? {
            Curve::Ellipse { id, center, major, .. } | Curve::EllipseArc { id, center, major, .. } | Curve::HyperbolaArc { id, center, major, .. } => {
                let (c, m) = (self.p(x, *center)?, self.p(x, *major)?);
                let kind = if matches!(self.curves.get(id)?, Curve::HyperbolaArc { .. }) { ConicKind::Hyperbola } else { ConicKind::Ellipse };
                Some(Conic { kind, c, u: unit(sub(m, c)), a: dist(c, m).max(1e-9), b: self.size(x, *id)?.abs().max(1e-9) })
            }
            Curve::ParabolaArc { vertex, focus, .. } => {
                let (v, f) = (self.p(x, *vertex)?, self.p(x, *focus)?);
                Some(Conic { kind: ConicKind::Parabola, c: v, u: unit(sub(f, v)), a: dist(v, f).max(1e-9), b: 0.0 })
            }
            _ => None,
        }
    }
    fn geo(&self, x: &[f64], id: u32) -> Option<Geo> {
        let c = self.curves.get(&id)?;
        geo_of(c, &|p| self.p(x, p), self.size(x, id))
    }
    fn dim(&self, key: &str) -> Result<f64, String> {
        self.dims.get(key).copied().ok_or_else(|| format!("no dimension “{key}”"))
    }
    /// The unknowns a point or curve is made of.
    fn vars_of(&self, id: u32) -> Vec<usize> {
        if let Some(i) = self.vars.point.get(&id) {
            return vec![*i, i + 1];
        }
        let Some(c) = self.curves.get(&id) else { return vec![] };
        let mut out: Vec<usize> = c.points().iter().filter_map(|p| self.vars.point.get(p)).flat_map(|i| [*i, i + 1]).collect();
        out.extend(self.vars.size.get(&id));
        out
    }
    fn is_end(&self, curve: u32, point: u32) -> bool {
        self.curves.get(&curve).and_then(|c| c.ends()).is_some_and(|(a, b)| a == point || b == point)
    }

    /// Signed distance of a point from a curve (whole conics; zero on it).
    fn on_curve(&self, x: &[f64], point: u32, curve: u32) -> Result<f64, String> {
        let p = self.p(x, point).ok_or("no such point")?;
        if let Some((a, b)) = self.line(x, curve) {
            return Ok(cross(unit(sub(b, a)), sub(p, a)));
        }
        if let Some((c, r)) = self.round(x, curve) {
            return Ok(dist(p, c) - r);
        }
        if let Some(k) = self.conic(x, curve) {
            return Ok(k.implicit(p));
        }
        let g = self.geo(x, curve).ok_or("no such curve")?;
        Ok(g.signed_distance(p, false))
    }

    /// The direction along a curve at a point on it (or its end).
    fn dir_at(&self, x: &[f64], curve: u32, j: u32) -> Result<P, String> {
        let jp = self.p(x, j).ok_or("no such point")?;
        let c = *self.curves.get(&curve).ok_or("no such curve")?;
        Ok(match c {
            Curve::Line { a, b, .. } => sub(self.p(x, *b).unwrap(), self.p(x, *a).unwrap()),
            Curve::Circle { center, .. } | Curve::Arc { center, .. } => perp(sub(jp, self.p(x, *center).unwrap())),
            Curve::Bezier { a, b, c1, c2, .. } if *a == j || *b == j => {
                let h = if *a == j { sub(self.p(x, *c1).unwrap(), jp) } else { sub(jp, self.p(x, *c2).unwrap()) };
                if len(h) > 1e-9 {
                    h
                } else {
                    let g = self.geo(x, curve).ok_or("no such curve")?;
                    g.deriv(if *a == j { g.range().0 } else { g.range().1 })
                }
            }
            Curve::Ellipse { .. } | Curve::EllipseArc { .. } | Curve::HyperbolaArc { .. } | Curve::ParabolaArc { .. } => self.conic(x, curve).unwrap().tangent(jp),
            _ => {
                let g = self.geo(x, curve).ok_or("the curve cannot be worked out")?;
                let (t0, t1) = g.range();
                let t = match c.ends() {
                    Some((a, _)) if a == j => t0,
                    Some((_, b)) if b == j => t1,
                    _ => g.closest(jp, false),
                };
                g.deriv(t)
            }
        })
    }

    /// The sizes `Equal` compares: a length, a radius, or both semi-axes.
    fn sizes(&self, x: &[f64], id: u32) -> Option<Vec<f64>> {
        if let Some((p, q)) = self.line(x, id) {
            return Some(vec![dist(p, q)]);
        }
        if let Some((_, r)) = self.round(x, id) {
            return Some(vec![r]);
        }
        let k = self.conic(x, id)?;
        Some(if k.kind == ConicKind::Parabola { vec![k.a] } else { vec![k.a, k.b] })
    }

    /// The gap a `Distance` measures between two things.
    fn distance(&self, x: &[f64], a: u32, b: u32) -> Result<f64, String> {
        if a == b {
            let (p, q) = self.line(x, a).ok_or("a length is of a line")?;
            return Ok(dist(p, q));
        }
        let (pa, pb) = (self.is_point(a), self.is_point(b));
        if pa && pb {
            return Ok(dist(self.p(x, a).unwrap(), self.p(x, b).unwrap()));
        }
        if pa || pb {
            let (pt, cv) = if pa { (a, b) } else { (b, a) };
            return Ok(self.on_curve(x, pt, cv)?.abs());
        }
        match ((self.line(x, a), self.round(x, a)), (self.line(x, b), self.round(x, b))) {
            ((Some((p, q)), _), (Some((r, s)), _)) => {
                // Parallel lines: the gap from line a to the middle of line b.
                let m = [(r[0] + s[0]) / 2.0, (r[1] + s[1]) / 2.0];
                Ok(cross(unit(sub(q, p)), sub(m, p)).abs())
            }
            ((Some((p, q)), _), (None, Some((c, r)))) | ((None, Some((c, r))), (Some((p, q)), _)) => Ok(cross(unit(sub(q, p)), sub(c, p)).abs() - r),
            ((None, Some((c1, r1))), (None, Some((c2, r2)))) => {
                let d = dist(c1, c2);
                // Edge to edge: apart, or one inside the other.
                Ok(if d + r1.min(r2) <= r1.max(r2) { (r1 - r2).abs() - d } else { d - r1 - r2 })
            }
            _ => Err("a distance is between points, lines, circles and arcs, or a point and a curve".into()),
        }
    }
}

/// Residuals of one constraint (zero when it holds), in millimetres.
fn residual(ctx: &Ctx, x: &[f64], c: &Constraint, out: &mut Vec<f64>) -> Result<(), String> {
    let miss = || format!("a constraint refers to something that is not in the sketch: {c:?}");
    match c {
        Constraint::Reference { .. } | Constraint::Inactive { .. } => {}
        Constraint::Coincident { a, b } => {
            let (p, q) = (ctx.p(x, *a).ok_or_else(miss)?, ctx.p(x, *b).ok_or_else(miss)?);
            out.extend([p[0] - q[0], p[1] - q[1]]);
        }
        Constraint::Horizontal { line } => {
            let (a, b) = ctx.line(x, *line).ok_or("horizontal is for lines")?;
            out.push(a[1] - b[1]);
        }
        Constraint::Vertical { line } => {
            let (a, b) = ctx.line(x, *line).ok_or("vertical is for lines")?;
            out.push(a[0] - b[0]);
        }
        Constraint::Parallel { a, b } => {
            let (p, q) = ctx.line(x, *a).ok_or("parallel is for lines")?;
            let (r, s) = ctx.line(x, *b).ok_or("parallel is for lines")?;
            // Scaled by a length so the residual is in millimetres, like the rest.
            let scale = len(sub(q, p)).min(len(sub(s, r))).max(1.0);
            out.push(cross(unit(sub(q, p)), unit(sub(s, r))) * scale);
        }
        Constraint::Perpendicular { a, b, at } => {
            let shared = at.or_else(|| shared_end(ctx, *a, *b));
            match (ctx.line(x, *a), ctx.line(x, *b), shared) {
                (Some((p, q)), Some((r, s)), _) => {
                    let scale = len(sub(q, p)).min(len(sub(s, r))).max(1.0);
                    out.push(dot(unit(sub(q, p)), unit(sub(s, r))) * scale);
                    if let Some(j) = *at {
                        for k in [*a, *b] {
                            if !ctx.is_end(k, j) {
                                out.push(ctx.on_curve(x, j, k)?);
                            }
                        }
                    }
                }
                (Some((p, q)), None, None) | (None, Some((p, q)), None) if ctx.round(x, if ctx.line(x, *a).is_some() { *b } else { *a }).is_some() => {
                    // A line square to a circle runs through its centre.
                    let (c, _) = ctx.round(x, if ctx.line(x, *a).is_some() { *b } else { *a }).unwrap();
                    out.push(cross(unit(sub(q, p)), sub(c, p)));
                }
                (_, _, Some(j)) => {
                    let (u, v) = (ctx.dir_at(x, *a, j)?, ctx.dir_at(x, *b, j)?);
                    out.push(dot(unit(u), unit(v)) * len(u).min(len(v)).clamp(1.0, 100.0));
                    for k in [*a, *b] {
                        if !ctx.is_end(k, j) {
                            out.push(ctx.on_curve(x, j, k)?);
                        }
                    }
                }
                _ => return Err("perpendicular is for two lines, a line and a circle, or two curves at a point".into()),
            }
        }
        Constraint::Equal { a, b } => {
            // Two pieces of one conic (sharing its centre and axis point): only their other semi-axes.
            let axis = |id: u32| match ctx.curves.get(&id) {
                Some(Curve::Ellipse { center, major, .. } | Curve::EllipseArc { center, major, .. } | Curve::HyperbolaArc { center, major, .. }) => Some((*center, *major)),
                _ => None,
            };
            if let (Some(p), Some(q)) = (axis(*a), axis(*b))
                && p == q
            {
                out.push(ctx.size(x, *a).ok_or_else(miss)? - ctx.size(x, *b).ok_or_else(miss)?);
                return Ok(());
            }
            let (sa, sb) =(ctx.sizes(x, *a).ok_or("equal is for lines, circles, arcs and conics")?, ctx.sizes(x, *b).ok_or("equal is for lines, circles, arcs and conics")?);
            match (sa.len(), sb.len()) {
                (n, m) if n == m => out.extend(sa.iter().zip(&sb).map(|(p, q)| p - q)),
                // A circle and an ellipse: both semi-axes the radius.
                (1, 2) => out.extend(sb.iter().map(|q| sa[0] - q)),
                (2, 1) => out.extend(sa.iter().map(|p| p - sb[0])),
                _ => return Err("those two cannot be equal".into()),
            }
        }
        Constraint::OnCurve { point, curve } => out.push(ctx.on_curve(x, *point, *curve)?),
        Constraint::Tangent { a, b, at } => tangent(ctx, x, *a, *b, *at, out)?,
        Constraint::Midpoint { point, line } => {
            let p = ctx.p(x, *point).ok_or_else(miss)?;
            let (a, b) = ctx.line(x, *line).ok_or("midpoint is of a line")?;
            out.extend([p[0] - (a[0] + b[0]) / 2.0, p[1] - (a[1] + b[1]) / 2.0]);
        }
        Constraint::Fixed { point, x: fx, y: fy } => {
            let p = ctx.p(x, *point).ok_or_else(miss)?;
            out.extend([p[0] - fx, p[1] - fy]);
        }
        Constraint::Block { curve } => {
            if !ctx.curves.contains_key(curve) {
                return Err(miss());
            }
            out.extend(ctx.vars_of(*curve).into_iter().map(|k| x[k] - ctx.x0[k]));
        }
        Constraint::Distance { a, b, key } => out.push(ctx.distance(x, *a, *b)? - ctx.dim(key)?),
        Constraint::HorizontalDistance { a, b, key } | Constraint::VerticalDistance { a, b, key } => {
            let v = ctx.dim(key)?;
            let k = if matches!(c, Constraint::HorizontalDistance { .. }) { 0 } else { 1 };
            let q = ctx.p(x, *b).ok_or_else(miss)?;
            // The same point twice: its own coordinate, from the origin.
            let from = if a == b { 0.0 } else { ctx.p(x, *a).ok_or_else(miss)?[k] };
            out.push((q[k] - from) - v);
        }
        Constraint::Length { line, key } => {
            let (p, q) = ctx.line(x, *line).ok_or("a length is of a line")?;
            out.push(dist(p, q) - ctx.dim(key)?);
        }
        Constraint::Radius { curve, key } | Constraint::Diameter { curve, key } => {
            let v = ctx.dim(key)?;
            let (_, r) = ctx.round(x, *curve).ok_or("a radius is of a circle or arc")?;
            out.push(if matches!(c, Constraint::Radius { .. }) { r - v } else { 2.0 * r - v });
        }
        Constraint::Angle { a, b, key } => {
            let v = ctx.dim(key)?.to_radians();
            if a == b {
                // An arc's sweep.
                let Some(Geo::Arc(_, r, a0, a1)) = ctx.geo(x, *a).filter(|_| matches!(ctx.curves.get(a), Some(Curve::Arc { .. }))) else {
                    return Err("an angle is between two lines, or an arc's sweep".into());
                };
                out.push((a1 - a0 - v) * r.max(1.0));
                return Ok(());
            }
            let (p, q) = ctx.line(x, *a).ok_or("an angle is between two lines")?;
            let (r, s) = ctx.line(x, *b).ok_or("an angle is between two lines")?;
            let (u, w) = (sub(q, p), sub(s, r));
            let ang = cross(u, w).atan2(dot(u, w));
            // Wrapped difference, scaled to millimetres by the shorter line.
            let d = (ang - v + PI).rem_euclid(TAU) - PI;
            out.push(d * len(u).min(len(w)).max(1.0));
        }
        Constraint::Translate { a, b, along } => {
            let (p, q) = (ctx.p(x, *a).ok_or_else(miss)?, ctx.p(x, *b).ok_or_else(miss)?);
            let (s, e) = ctx.line(x, *along).ok_or_else(miss)?;
            out.extend([q[0] - p[0] - (e[0] - s[0]), q[1] - p[1] - (e[1] - s[1])]);
        }
        Constraint::Rotate { a, b, from, to } => {
            let (p, q) = (ctx.p(x, *a).ok_or_else(miss)?, ctx.p(x, *b).ok_or_else(miss)?);
            let (c, r0) = ctx.line(x, *from).ok_or_else(miss)?;
            let (s1, r1) = ctx.line(x, *to).ok_or_else(miss)?;
            let (u, w) = (sub(r0, c), sub(r1, s1));
            let ang = cross(u, w).atan2(dot(u, w));
            let (sn, cs) = ang.sin_cos();
            let d = sub(p, c);
            out.extend([q[0] - (c[0] + d[0] * cs - d[1] * sn), q[1] - (c[1] + d[0] * sn + d[1] * cs)]);
        }
        Constraint::Symmetric { a, b, line } => {
            let (p, q) = (ctx.p(x, *a).ok_or_else(miss)?, ctx.p(x, *b).ok_or_else(miss)?);
            let m = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            if let Some(o) = ctx.p(x, *line) {
                // About a point: it is their midpoint.
                out.extend([m[0] - o[0], m[1] - o[1]]);
            } else {
                let (s, e) = ctx.line(x, *line).ok_or("symmetric is about a line or a point")?;
                let d = unit(sub(e, s));
                out.extend([cross(d, sub(m, s)), dot(d, sub(q, p))]);
            }
        }
        Constraint::Snell { a, b, boundary, at, key } => {
            let ratio = ctx.dim(key)?;
            let j = ctx.p(x, *at).ok_or_else(miss)?;
            let ray = |id: u32| -> Result<P, String> {
                match ctx.curves.get(&id) {
                    Some(Curve::Line { a, b, .. }) if *a == *at => Ok(sub(j, ctx.p(x, *b).unwrap())),
                    Some(Curve::Line { a, b, .. }) if *b == *at => Ok(sub(j, ctx.p(x, *a).unwrap())),
                    _ => Err("Snell's law takes two lines (rays) that end at the point on the boundary".into()),
                }
            };
            // Arriving along a (towards the point), leaving along b (away from it).
            let (d1, d2) = (ray(*a)?, ray(*b)?);
            let d2 = [-d2[0], -d2[1]];
            let n = unit(perp(ctx.dir_at(x, *boundary, *at)?));
            let scale = len(d1).min(len(d2)).max(1.0);
            out.push((cross(n, unit(d1)) - ratio * cross(n, unit(d2))) * scale);
            if !ctx.is_end(*boundary, *at) {
                out.push(ctx.on_curve(x, *at, *boundary)?);
            }
        }
        Constraint::InternalAlignment { curve, element, role, index } => {
            if let Role::Knot = role {
                let Some(Geo::Nurbs(n)) = ctx.geo(x, *curve).filter(|_| matches!(ctx.curves.get(curve), Some(Curve::BSpline { .. }))) else {
                    return Err("a knot is of a B-spline".into());
                };
                let (k, _) = n.knots_mults();
                let i = index.ok_or("a knot alignment needs the knot's index")? as usize;
                let u = *k.get(i).ok_or("no such knot")?;
                let (q, p) = (n.point(u), ctx.p(x, *element).ok_or_else(miss)?);
                out.extend([p[0] - q[0], p[1] - q[1]]);
                return Ok(());
            }
            let k = ctx.conic(x, *curve).ok_or("internal alignment is for conics and B-splines")?;
            let v = perp(k.u);
            let at = |s: f64, t: f64| [k.c[0] + k.u[0] * s + v[0] * t, k.c[1] + k.u[1] * s + v[1] * t];
            let focal = match k.kind {
                ConicKind::Ellipse => (k.a * k.a - k.b * k.b).max(0.0).sqrt(),
                ConicKind::Hyperbola => (k.a * k.a + k.b * k.b).sqrt(),
                ConicKind::Parabola => k.a,
            };
            let mut hold = |p: P, q: P| out.extend([p[0] - q[0], p[1] - q[1]]);
            match role {
                Role::MajorAxis | Role::MinorAxis => {
                    if k.kind == ConicKind::Parabola {
                        return Err("a parabola has no axis ends".into());
                    }
                    let (p, q) = ctx.line(x, *element).ok_or("an axis is a line")?;
                    let (e0, e1) = if *role == Role::MajorAxis { (at(-k.a, 0.0), at(k.a, 0.0)) } else { (at(0.0, -k.b), at(0.0, k.b)) };
                    hold(p, e0);
                    hold(q, e1);
                }
                Role::Focus1 | Role::Focus2 => {
                    if k.kind == ConicKind::Parabola && *role == Role::Focus2 {
                        return Err("a parabola has one focus".into());
                    }
                    let p = ctx.p(x, *element).ok_or("a focus is a point")?;
                    hold(p, at(if *role == Role::Focus1 { focal } else { -focal }, 0.0));
                }
                Role::Knot => unreachable!(),
            }
        }
    }
    Ok(())
}

fn shared_end(ctx: &Ctx, a: u32, b: u32) -> Option<u32> {
    let ends = |id: u32| ctx.curves.get(&id).and_then(|c| c.ends()).map(|(p, q)| vec![p, q]).unwrap_or_default();
    let eb = ends(b);
    ends(a).into_iter().find(|p| eb.contains(p))
}

/// Tangency: at a point (one smooth join, the point held on both), or apart.
fn tangent(ctx: &Ctx, x: &[f64], a: u32, b: u32, at: Option<u32>, out: &mut Vec<f64>) -> Result<(), String> {
    let shared = at.or_else(|| shared_end(ctx, a, b));
    let kind = |id: u32| ctx.curves.get(&id).copied();
    let simple = |id: u32| matches!(kind(id), Some(Curve::Line { .. } | Curve::Arc { .. } | Curve::Circle { .. }));
    if let Some(j) = shared {
        let jp = ctx.p(x, j).ok_or("no such point")?;
        if simple(a) && simple(b) {
            // Lines and arcs keep their own forms: the radius square to the
            // line, or the centres in line with the point.
            match (ctx.line(x, a), ctx.line(x, b)) {
                (Some((p, q)), None) | (None, Some((p, q))) => {
                    let round = if ctx.line(x, a).is_some() { b } else { a };
                    let (c, _) = ctx.round(x, round).unwrap();
                    out.push(dot(unit(sub(q, p)), sub(c, jp)));
                }
                (None, None) => {
                    let (c1, _) = ctx.round(x, a).unwrap();
                    let (c2, _) = ctx.round(x, b).unwrap();
                    out.push(cross(unit(sub(c1, jp)), sub(c2, jp)));
                }
                _ => return Err("two lines cannot be tangent".into()),
            }
        } else {
            // A smooth join: the two curves leave the point along one line.
            let (u, v) = (ctx.dir_at(x, a, j)?, ctx.dir_at(x, b, j)?);
            let scale = len(u).min(len(v)).clamp(1.0, 100.0);
            out.push(cross(unit(u), unit(v)) * scale);
        }
        if at.is_some() {
            for k in [a, b] {
                if !ctx.is_end(k, j) {
                    out.push(ctx.on_curve(x, j, k)?);
                }
            }
        }
        return Ok(());
    }
    // Apart.
    let line_and = |l: u32, o: u32| -> Option<Result<f64, String>> {
        let (p, q) = ctx.line(x, l)?;
        if ctx.line(x, o).is_some() {
            return Some(Err("two lines cannot be tangent".into()));
        }
        let n = perp(unit(sub(q, p)));
        if let Some((c, r)) = ctx.round(x, o) {
            return Some(Ok(cross(unit(sub(q, p)), sub(c, p)).abs() - r));
        }
        if let Some(k) = ctx.conic(x, o) {
            let (lu, lv, off) = (dot(n, k.u), dot(n, perp(k.u)), dot(n, sub(k.c, p)));
            return Some(Ok(match k.kind {
                // The ellipse's reach towards the line equals its centre's distance.
                ConicKind::Ellipse => off.abs() - ((k.a * lu).powi(2) + (k.b * lv).powi(2)).sqrt(),
                ConicKind::Hyperbola => {
                    let d = (k.a * lu).powi(2) - (k.b * lv).powi(2);
                    if d >= 0.0 { off.abs() - d.sqrt() } else { off.abs() + (-d).sqrt() }
                }
                // lX + mY + k = 0 touches X = Y²/4f when m²f = l·k.
                ConicKind::Parabola => lv * lv * k.a - lu * off,
            }));
        }
        // A spline: its nearest reach towards the line, where it runs along it.
        let g = ctx.geo(x, o)?;
        let f = |t: f64| dot(n, sub(g.point(t), p));
        let (t0, t1) = g.range();
        let mut best = (f64::INFINITY, t0);
        for i in 0..=96 {
            let t = t0 + (t1 - t0) * i as f64 / 96.0;
            if f(t).abs() < best.0 {
                best = (f(t).abs(), t);
            }
        }
        let mut t = best.1;
        let h = 1e-6 * (t1 - t0);
        for _ in 0..30 {
            let gt = dot(n, g.deriv(t));
            let gp = (dot(n, g.deriv((t + h).min(t1))) - dot(n, g.deriv((t - h).max(t0)))) / (2.0 * h);
            if gp.abs() < 1e-300 {
                break;
            }
            let next = (t - gt / gp).clamp(t0, t1);
            if (next - t).abs() < 1e-14 * (1.0 + t.abs()) {
                break;
            }
            t = next;
        }
        Some(Ok(f(t)))
    };
    if let Some(r) = line_and(a, b).or_else(|| line_and(b, a)) {
        out.push(r?);
        return Ok(());
    }
    match (ctx.round(x, a), ctx.round(x, b)) {
        (Some((c1, r1)), Some((c2, r2))) => {
            // Outside tangency when apart, inside when one holds the other.
            let d = dist(c1, c2);
            out.push(if d > r1.max(r2) { d - (r1 + r2) } else { d - (r1 - r2).abs() });
        }
        (Some((c, r)), None) | (None, Some((c, r))) => {
            // A circle and a conic or spline: the nearest reach from its centre is its radius.
            let other = if ctx.round(x, a).is_some() { b } else { a };
            let g = ctx.geo(x, other).ok_or("no such curve")?;
            let whole = ctx.conic(x, other).is_some();
            let t = g.closest(c, whole);
            out.push(dist(c, g.point(t)) - r);
        }
        _ => return Err("those two are tangent only at a point: give the point (`at`) where they touch".into()),
    }
    Ok(())
}

/// What a dimension measures as drawn (mm, degrees, or a ratio).
fn measure(ctx: &Ctx, x: &[f64], c: &Constraint) -> Option<f64> {
    let c = c.inner();
    let key = c.key()?;
    match c {
        Constraint::Angle { a, b, .. } => {
            if a == b {
                let Geo::Arc(_, _, a0, a1) = ctx.geo(x, *a)? else { return None };
                return Some((a1 - a0).to_degrees());
            }
            let (p, q) = ctx.line(x, *a)?;
            let (r, s) = ctx.line(x, *b)?;
            let (u, w) = (sub(q, p), sub(s, r));
            Some(cross(u, w).atan2(dot(u, w)).to_degrees())
        }
        Constraint::Snell { a, b, boundary, at, .. } => {
            let mut zero = ctx.dims.clone();
            zero.insert(key.to_string(), 0.0);
            let z = Ctx { dims: &zero, vars: ctx.vars, curves: ctx.curves.clone(), x0: ctx.x0 };
            let mut one = zero.clone();
            one.insert(key.to_string(), 1.0);
            let o = Ctx { dims: &one, vars: ctx.vars, curves: ctx.curves.clone(), x0: ctx.x0 };
            let (mut r0, mut r1) = (vec![], vec![]);
            let snell = Constraint::Snell { a: *a, b: *b, boundary: *boundary, at: *at, key: key.to_string() };
            residual(&z, x, &snell, &mut r0).ok()?;
            residual(&o, x, &snell, &mut r1).ok()?;
            // r = s1 − ratio·s2: ratio = s1 / s2.
            let (s1, s2) = (r0[0], r0[0] - r1[0]);
            (s2.abs() > 1e-12).then(|| s1 / s2)
        }
        _ => {
            let mut zero = ctx.dims.clone();
            zero.insert(key.to_string(), 0.0);
            let z = Ctx { dims: &zero, vars: ctx.vars, curves: ctx.curves.clone(), x0: ctx.x0 };
            let mut r = vec![];
            residual(&z, x, c, &mut r).ok()?;
            r.first().copied()
        }
    }
}

/// One equation block: a constraint's rows, or the rows a curve keeps for
/// itself (an arc's ends on one circle, a conic arc's ends on the conic).
struct Eq<'a> {
    /// The constraint's index; none for a curve's own rows.
    owner: Option<usize>,
    what: EqKind<'a>,
    rows: usize,
    deps: Vec<usize>,
}

enum EqKind<'a> {
    Own(&'a Curve),
    Of(&'a Constraint),
}

fn eval(ctx: &Ctx, x: &[f64], e: &EqKind, out: &mut Vec<f64>) -> Result<(), String> {
    match e {
        EqKind::Of(c) => residual(ctx, x, c, out),
        EqKind::Own(c) => {
            match c {
                Curve::Arc { center, start, end, .. } => {
                    let (cc, s, en) = (ctx.p(x, *center).ok_or("arc without center")?, ctx.p(x, *start).ok_or("arc without start")?, ctx.p(x, *end).ok_or("arc without end")?);
                    out.push(dist(cc, en) - dist(cc, s));
                }
                Curve::EllipseArc { id, start, end, .. } | Curve::HyperbolaArc { id, start, end, .. } | Curve::ParabolaArc { id, start, end, .. } => {
                    out.push(ctx.on_curve(x, *start, *id)?);
                    out.push(ctx.on_curve(x, *end, *id)?);
                }
                _ => {}
            }
            Ok(())
        }
    }
}

fn find(parent: &mut Vec<usize>, mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Solves a sketch: the smallest move of its points that satisfies every
/// constraint, given the dimensions' values (mm, degrees), and what the
/// constraints leave free, repeat or contradict.
pub fn solve(g: &Geometry, dims: &BTreeMap<String, f64>) -> Result<Solved, String> {
    let v = vars(g);
    let ctx = Ctx { vars: &v, dims, curves: g.curves.iter().map(|c| (c.id(), c)).collect(), x0: &v.values };
    let n = v.values.len();
    let x0 = v.values.clone();

    // The equations: curves' own first (so a user's constraint, not a curve, is what repeats).
    let mut eqs: Vec<Eq> = vec![];
    let mut malformed = vec![];
    let mut scratch = vec![];
    for c in &g.curves {
        scratch.clear();
        let what = EqKind::Own(c);
        eval(&ctx, &x0, &what, &mut scratch)?;
        if !scratch.is_empty() {
            eqs.push(Eq { owner: None, what, rows: scratch.len(), deps: ctx.vars_of(c.id()) });
        }
    }
    for (i, c) in g.constraints.iter().enumerate() {
        scratch.clear();
        let what = EqKind::Of(c);
        match eval(&ctx, &x0, &what, &mut scratch) {
            Ok(()) if scratch.iter().all(|r| r.is_finite()) => {
                if !scratch.is_empty() {
                    let mut deps: Vec<usize> = c.ids().into_iter().flat_map(|id| ctx.vars_of(id)).collect();
                    deps.sort();
                    deps.dedup();
                    eqs.push(Eq { owner: Some(i), what, rows: scratch.len(), deps });
                }
            }
            _ => malformed.push(i),
        }
    }

    // Independent groups: unknowns linked by any equation solve together.
    let mut parent: Vec<usize> = (0..n).collect();
    for e in &eqs {
        for w in e.deps.windows(2) {
            let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            if a != b {
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, e) in eqs.iter().enumerate() {
        if let Some(&d) = e.deps.first() {
            groups.entry(find(&mut parent, d)).or_default().push(i);
        }
    }

    let mut x = x0.clone();
    let mut conflicts: BTreeSet<usize> = BTreeSet::new();
    let mut redundant: BTreeSet<usize> = BTreeSet::new();
    let mut partial: BTreeSet<usize> = BTreeSet::new();
    let mut free_var = vec![true; n];
    let mut free = 0;
    let mut covered = vec![false; n];
    let mut residual_max: f64 = 0.0;
    for list in groups.values() {
        let group: Vec<&Eq> = list.iter().map(|i| &eqs[*i]).collect();
        let mut cols: Vec<usize> = group.iter().flat_map(|e| e.deps.iter().copied()).collect();
        cols.sort();
        cols.dedup();
        for &k in &cols {
            covered[k] = true;
        }
        let r = solve_group(&ctx, &group, &cols, &mut x)?;
        residual_max = residual_max.max(r.iter().fold(0.0, |m, v| m.max(v.abs())));
        let a = analyse(&ctx, &group, &cols, &x, &r)?;
        free += cols.len() - a.rank;
        for (k, f) in cols.iter().zip(&a.free) {
            free_var[*k] = *f;
        }
        conflicts.extend(a.conflicts);
        redundant.extend(a.redundant);
        partial.extend(a.partial);
    }
    free += covered.iter().filter(|c| !**c).count();

    let mut points = BTreeMap::new();
    for p in &g.points {
        let i = v.point[&p.id];
        points.insert(p.id, [x[i], x[i + 1]]);
    }
    let radii = v.size.iter().map(|(id, i)| (*id, x[*i])).collect();
    let free_points: Vec<u32> = g.points.iter().filter(|p| free_var[v.point[&p.id]] || free_var[v.point[&p.id] + 1]).map(|p| p.id).collect();
    let free_curves: Vec<u32> = g.curves.iter().filter(|c| ctx.vars_of(c.id()).iter().any(|k| free_var[*k])).map(Curve::id).collect();
    let measured = g.constraints.iter().filter_map(|c| Some((c.key()?.to_string(), measure(&ctx, &x, c)?))).collect();
    let redundant: Vec<usize> = redundant.difference(&conflicts).copied().collect();
    let partially_redundant: Vec<usize> = partial.difference(&conflicts).filter(|i| !redundant.contains(i)).copied().collect();
    Ok(Solved {
        points,
        radii,
        free,
        conflicts: conflicts.into_iter().collect(),
        redundant,
        partially_redundant,
        malformed,
        free_points,
        free_curves,
        measured,
        residual: residual_max,
    })
}

fn group_residuals(ctx: &Ctx, group: &[&Eq], x: &[f64]) -> Result<Vec<f64>, String> {
    let mut r = vec![];
    for e in group {
        let before = r.len();
        eval(ctx, x, &e.what, &mut r)?;
        if r.len() - before != e.rows {
            // A constraint whose row count changed with the shape (it cannot).
            r.resize(before + e.rows, 0.0);
        }
    }
    Ok(r)
}

/// The group's Jacobian (rows × its columns), by central differences, one equation at a time.
fn jacobian(ctx: &Ctx, group: &[&Eq], cols: &[usize], x: &[f64]) -> Result<Vec<Vec<f64>>, String> {
    let col: HashMap<usize, usize> = cols.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let mut j = vec![];
    let mut xp = x.to_vec();
    let (mut rp, mut rm) = (vec![], vec![]);
    for e in group {
        let base = j.len();
        j.extend(std::iter::repeat_n(vec![0.0; cols.len()], e.rows));
        for &k in &e.deps {
            let h = 1e-7 * x[k].abs().max(1.0);
            xp[k] = x[k] + h;
            rp.clear();
            eval(ctx, &xp, &e.what, &mut rp)?;
            xp[k] = x[k] - h;
            rm.clear();
            eval(ctx, &xp, &e.what, &mut rm)?;
            xp[k] = x[k];
            for row in 0..e.rows.min(rp.len()).min(rm.len()) {
                j[base + row][col[&k]] = (rp[row] - rm[row]) / (2.0 * h);
            }
        }
    }
    Ok(j)
}

/// Damped Gauss–Newton steps of least length until the group holds (or
/// can come no nearer). Returns the residuals where it stopped.
fn solve_group(ctx: &Ctx, group: &[&Eq], cols: &[usize], x: &mut [f64]) -> Result<Vec<f64>, String> {
    let norm = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut r = group_residuals(ctx, group, x)?;
    let (m, nv) = (r.len(), cols.len());
    let mut lambda = 1e-3;
    for _ in 0..200 {
        if norm(&r).sqrt() < 1e-10 || m == 0 {
            break;
        }
        let j = jacobian(ctx, group, cols, x)?;
        // Few equations: dx = −Jᵀ(JJᵀ + λ)⁻¹r (the least move); many: (JᵀJ + λ)dx = −Jᵀr.
        let wide = m <= nv;
        let size = if wide { m } else { nv };
        let mut a = vec![vec![0.0; size]; size];
        let mut b = vec![0.0; size];
        if wide {
            for p in 0..m {
                for q in p..m {
                    let s: f64 = j[p].iter().zip(&j[q]).map(|(u, v)| u * v).sum();
                    a[p][q] = s;
                    a[q][p] = s;
                }
                b[p] = r[p];
            }
        } else {
            for (row, ri) in j.iter().zip(&r) {
                for p in 0..nv {
                    if row[p] == 0.0 {
                        continue;
                    }
                    b[p] -= row[p] * ri;
                    for q in 0..nv {
                        a[p][q] += row[p] * row[q];
                    }
                }
            }
        }
        let mut improved = false;
        for _ in 0..14 {
            let mut damped = a.clone();
            for (i, row) in damped.iter_mut().enumerate() {
                row[i] += lambda * (1.0 + a[i][i]);
            }
            let Some(y) = crate::curves::solve_dense(damped, b.clone()) else {
                lambda *= 10.0;
                continue;
            };
            let dx: Vec<f64> = if wide { (0..nv).map(|c| -(0..m).map(|p| j[p][c] * y[p]).sum::<f64>()).collect() } else { y };
            let mut trial = x.to_vec();
            for (c, d) in cols.iter().zip(&dx) {
                trial[*c] += d;
            }
            let rt = group_residuals(ctx, group, &trial)?;
            if rt.iter().all(|v| v.is_finite()) && norm(&rt) < norm(&r) {
                x.copy_from_slice(&trial);
                r = rt;
                lambda = (lambda / 3.0).max(1e-12);
                improved = true;
                break;
            }
            lambda *= 4.0;
        }
        if !improved {
            break;
        }
    }
    Ok(r)
}

struct Analysis {
    rank: usize,
    free: Vec<bool>,
    conflicts: Vec<usize>,
    redundant: Vec<usize>,
    partial: Vec<usize>,
}

/// Rank, redundancy, conflicts and free unknowns of a solved group: its
/// Jacobian's rows orthogonalised in order (modified Gram–Schmidt, twice),
/// each dependent row traced back to the earlier rows it repeats.
fn analyse(ctx: &Ctx, group: &[&Eq], cols: &[usize], x: &[f64], r: &[f64]) -> Result<Analysis, String> {
    let j = jacobian(ctx, group, cols, x)?;
    let nv = cols.len();
    let owner_of: Vec<Option<usize>> = group.iter().flat_map(|e| std::iter::repeat_n(e.owner, e.rows)).collect();
    let eq_of: Vec<usize> = group.iter().enumerate().flat_map(|(i, e)| std::iter::repeat_n(i, e.rows)).collect();
    // Orthonormal basis of the independent rows, and each basis vector as a
    // combination of those rows (so a dependent row can be traced to them).
    let mut basis: Vec<Vec<f64>> = vec![];
    let mut combo: Vec<Vec<f64>> = vec![];
    let mut indep: Vec<usize> = vec![];
    let mut dependent: Vec<(usize, Vec<usize>)> = vec![];
    let mut dep_rows = vec![0usize; group.len()];
    for (row, v) in j.iter().enumerate() {
        let n0 = len_n(v);
        let mut w = v.clone();
        let mut coef = vec![0.0; basis.len()];
        for _ in 0..2 {
            for (k, q) in basis.iter().enumerate() {
                let c: f64 = w.iter().zip(q).map(|(a, b)| a * b).sum();
                coef[k] += c;
                for (wi, qi) in w.iter_mut().zip(q) {
                    *wi -= c * qi;
                }
            }
        }
        let nw = len_n(&w);
        if nw <= 1e-8 * n0.max(1e-300) || n0 < 1e-12 {
            // Which independent rows it repeats.
            let mut over = vec![0.0; indep.len()];
            for (k, c) in coef.iter().enumerate() {
                for (i, v) in combo[k].iter().enumerate() {
                    over[i] += c * v;
                }
            }
            let big = over.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            let on: Vec<usize> = over.iter().enumerate().filter(|(_, v)| v.abs() > 1e-6 * big && big > 0.0).map(|(i, _)| indep[i]).collect();
            dependent.push((row, on));
            dep_rows[eq_of[row]] += 1;
            continue;
        }
        // c_new = (e_row − Σ coef_k · combo_k) / |w|
        let mut c = vec![0.0; indep.len() + 1];
        c[indep.len()] = 1.0;
        for (k, cf) in coef.iter().enumerate() {
            for (i, v) in combo[k].iter().enumerate() {
                c[i] -= cf * v;
            }
        }
        for v in &mut c {
            *v /= nw;
        }
        for old in &mut combo {
            old.push(0.0);
        }
        combo.push(c);
        basis.push(w.iter().map(|v| v / nw).collect());
        indep.push(row);
    }
    let rank = basis.len();
    let free: Vec<bool> = (0..nv).map(|k| 1.0 - basis.iter().map(|q| q[k] * q[k]).sum::<f64>() > 1e-6).collect();

    let mut conflicts = BTreeSet::new();
    let unmet = r.iter().any(|v| v.abs() > MET);
    for (row, on) in &dependent {
        let rows: Vec<usize> = std::iter::once(*row).chain(on.iter().copied()).collect();
        if unmet && rows.iter().any(|i| r[*i].abs() > MET) {
            conflicts.extend(rows.iter().filter_map(|i| owner_of[*i]));
        }
    }
    if unmet && conflicts.is_empty() {
        // Not met and nothing repeats: name what is left unmet.
        conflicts.extend(r.iter().enumerate().filter(|(_, v)| v.abs() > MET).filter_map(|(i, _)| owner_of[i]));
    }
    let mut redundant = vec![];
    let mut partial = vec![];
    for (i, e) in group.iter().enumerate() {
        let Some(owner) = e.owner else { continue };
        if dep_rows[i] == e.rows {
            redundant.push(owner);
        } else if dep_rows[i] > 0 {
            partial.push(owner);
        }
    }
    Ok(Analysis { rank, free, conflicts: conflicts.into_iter().collect(), redundant, partial })
}

fn len_n(v: &[f64]) -> f64 {
    v.iter().map(|a| a * a).sum::<f64>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Point, Role};

    fn pt(id: u32, x: f64, y: f64) -> Point {
        Point { id, x, y }
    }
    fn line(id: u32, a: u32, b: u32) -> Curve {
        Curve::Line { id, a, b, construction: false }
    }
    fn dims(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    /// A rectangle: four lines, fixed corner, all square.
    fn rect() -> Geometry {
        Geometry {
            points: vec![pt(1, 0.0, 0.0), pt(2, 98.0, 1.0), pt(3, 101.0, 52.0), pt(4, -1.0, 49.0)],
            curves: vec![line(10, 1, 2), line(11, 2, 3), line(12, 3, 4), line(13, 4, 1)],
            constraints: vec![
                Constraint::Fixed { point: 1, x: 0.0, y: 0.0 },
                Constraint::Horizontal { line: 10 },
                Constraint::Horizontal { line: 12 },
                Constraint::Vertical { line: 11 },
                Constraint::Vertical { line: 13 },
            ],
        }
    }

    #[test]
    fn freedoms_and_free_points_are_reported() {
        let g = rect();
        let s = solve(&g, &dims(&[])).unwrap();
        assert_eq!(s.free, 2, "width and height");
        assert!(!s.free_points.contains(&1), "the pinned corner cannot move");
        assert!(s.free_points.contains(&3));
        assert!(s.redundant.is_empty() && s.conflicts.is_empty());
    }

    #[test]
    fn redundant_constraints_are_found_and_kept_apart_from_conflicts() {
        let mut g = rect();
        // Parallel adds nothing: both lines are already horizontal.
        g.constraints.push(Constraint::Parallel { a: 10, b: 12 });
        let s = solve(&g, &dims(&[])).unwrap();
        assert_eq!(s.redundant, vec![5], "{s:?}");
        assert!(s.conflicts.is_empty());
        assert_eq!(s.free, 2);
        // A width given twice, consistently: redundant; inconsistently: both conflict.
        g.constraints.push(Constraint::Length { line: 10, key: "w".into() });
        g.constraints.push(Constraint::Length { line: 12, key: "w2".into() });
        let s = solve(&g, &dims(&[("w", 100.0), ("w2", 100.0)])).unwrap();
        assert!(s.redundant.contains(&7), "{s:?}");
        let s = solve(&g, &dims(&[("w", 100.0), ("w2", 80.0)])).unwrap();
        assert!(s.conflicts.contains(&6) && s.conflicts.contains(&7), "{s:?}");
        assert!(!s.conflicts.contains(&0), "the pin has nothing to do with it: {s:?}");
    }

    #[test]
    fn a_coincidence_on_a_held_axis_is_partly_redundant() {
        let g = Geometry {
            points: vec![pt(1, 0.0, 0.0), pt(2, 10.0, 0.1), pt(3, 12.0, -0.2)],
            curves: vec![line(5, 1, 2)],
            constraints: vec![
                Constraint::Fixed { point: 1, x: 0.0, y: 0.0 },
                Constraint::Horizontal { line: 5 },
                Constraint::VerticalDistance { a: 1, b: 3, key: "h".into() },
                Constraint::Coincident { a: 2, b: 3 },
            ],
        };
        let s = solve(&g, &dims(&[("h", 0.0)])).unwrap();
        assert!(s.redundant.is_empty() && s.conflicts.is_empty(), "{s:?}");
        assert_eq!(s.partially_redundant, vec![3], "its y is held already: {s:?}");
    }

    #[test]
    fn reference_dimensions_measure_and_inactive_ones_rest() {
        let mut g = rect();
        g.constraints.push(Constraint::Length { line: 10, key: "w".into() });
        g.constraints.push(Constraint::Reference { of: Box::new(Constraint::Length { line: 11, key: "h".into() }) });
        g.constraints.push(Constraint::Inactive { of: Box::new(Constraint::Length { line: 12, key: "x".into() }) });
        let s = solve(&g, &dims(&[("w", 120.0), ("h", 5.0), ("x", 3.0)])).unwrap();
        assert!(s.conflicts.is_empty(), "{s:?}");
        assert!((s.measured["w"] - 120.0).abs() < 1e-6);
        assert!((s.measured["h"] - 51.0).abs() < 1.5, "the height as drawn: {}", s.measured["h"]);
        assert_eq!(s.free, 1, "the reference does not hold the height");
    }

    #[test]
    fn ellipse_tangent_to_lines_and_aligned_foci() {
        // An ellipse in a 100 × 60 box: tangent to all four sides.
        let mut g = rect();
        g.constraints.push(Constraint::Length { line: 10, key: "w".into() });
        g.constraints.push(Constraint::Length { line: 11, key: "h".into() });
        g.points.extend([pt(20, 45.0, 28.0), pt(21, 90.0, 30.0), pt(22, 0.0, 0.0), pt(23, 0.0, 0.0), pt(24, 10.0, 25.0)]);
        g.curves.push(Curve::Ellipse { id: 30, center: 20, major: 21, minor: 25.0, construction: false });
        g.curves.push(Curve::Line { id: 31, a: 22, b: 23, construction: true });
        for side in [10, 11, 12, 13] {
            g.constraints.push(Constraint::Tangent { a: side, b: 30, at: None });
        }
        g.constraints.push(Constraint::Horizontal { line: 31 });
        g.constraints.push(Constraint::InternalAlignment { curve: 30, element: 31, role: Role::MajorAxis, index: None });
        g.constraints.push(Constraint::InternalAlignment { curve: 30, element: 24, role: Role::Focus1, index: None });
        let s = solve(&g, &dims(&[("w", 100.0), ("h", 60.0)])).unwrap();
        assert!(s.residual < 1e-8 && s.conflicts.is_empty(), "{s:?}");
        let c = s.points[&20];
        assert!((c[0] - 50.0).abs() < 1e-6 && (c[1] - 30.0).abs() < 1e-6, "centred: {c:?}");
        assert!((s.radii[&30] - 30.0).abs() < 1e-6, "minor 30: {}", s.radii[&30]);
        let f = s.points[&24];
        assert!((f[0] - (50.0 + 40.0)).abs() < 1e-6, "focus at c + √(50² − 30²): {f:?}");
        let ends = (s.points[&22], s.points[&23]);
        assert!((ends.0[0] - 0.0).abs() < 1e-6 && (ends.1[0] - 100.0).abs() < 1e-6, "{ends:?}");
    }

    #[test]
    fn snells_law_bends_the_ray() {
        // A flat boundary along y = 0; a ray from above to the origin, out below.
        let g = Geometry {
            points: vec![pt(1, -100.0, 0.0), pt(2, 100.0, 0.0), pt(3, -50.0, 50.0), pt(4, 0.0, 0.0), pt(5, 30.0, -60.0)],
            curves: vec![line(10, 1, 2), line(11, 3, 4), line(12, 4, 5)],
            constraints: vec![
                Constraint::Fixed { point: 1, x: -100.0, y: 0.0 },
                Constraint::Fixed { point: 2, x: 100.0, y: 0.0 },
                Constraint::Fixed { point: 3, x: -50.0, y: 50.0 },
                Constraint::Fixed { point: 4, x: 0.0, y: 0.0 },
                Constraint::Distance { a: 4, b: 5, key: "l".into() },
                Constraint::Snell { a: 11, b: 12, boundary: 10, at: 4, key: "n".into() },
            ],
        };
        let s = solve(&g, &dims(&[("l", 50.0), ("n", 1.5)])).unwrap();
        assert!(s.residual < 1e-8, "{s:?}");
        let q = s.points[&5];
        let sin2 = q[0] / 50.0;
        assert!((sin2 - (45f64.to_radians().sin() / 1.5)).abs() < 1e-6, "sin θ₂ = sin 45° / 1.5: {q:?}");
        assert!(q[1] < 0.0);
        assert!((s.measured["n"] - 1.5).abs() < 1e-6);
    }

    #[test]
    fn symmetric_about_a_point_and_block() {
        let g = Geometry {
            points: vec![pt(1, 3.0, 4.0), pt(2, 10.0, 1.0), pt(3, 0.0, 0.0), pt(4, 20.0, 5.0)],
            curves: vec![line(10, 3, 4)],
            constraints: vec![Constraint::Block { curve: 10 }, Constraint::Symmetric { a: 1, b: 2, line: 3 }, Constraint::Fixed { point: 1, x: 3.0, y: 4.0 }],
        };
        let s = solve(&g, &dims(&[])).unwrap();
        assert!(s.residual < 1e-9);
        let near = |a: [f64; 2], b: [f64; 2]| dist(a, b) < 1e-9;
        assert!(near(s.points[&2], [-3.0, -4.0]), "{:?}", s.points[&2]);
        assert!(near(s.points[&4], [20.0, 5.0]), "the blocked line stays");
        assert_eq!(s.free, 0);
    }

    #[test]
    fn splines_and_conic_arcs_hold_points_and_join_smoothly() {
        // A B-spline from the end of a line, tangent there; a point held on it.
        let g = Geometry {
            points: vec![pt(1, 0.0, 0.0), pt(2, 20.0, 0.0), pt(3, 30.0, 4.0), pt(4, 45.0, 20.0), pt(5, 50.0, 40.0), pt(6, 40.0, 15.0)],
            curves: vec![line(10, 1, 2), Curve::BSpline { id: 11, poles: vec![2, 3, 4, 5], degree: 3, weights: vec![], knots: vec![], mults: vec![], periodic: false, construction: false }],
            constraints: vec![
                Constraint::Fixed { point: 1, x: 0.0, y: 0.0 },
                Constraint::Horizontal { line: 10 },
                Constraint::Tangent { a: 10, b: 11, at: None },
                Constraint::OnCurve { point: 6, curve: 11 },
            ],
        };
        let s = solve(&g, &dims(&[])).unwrap();
        assert!(s.residual < 1e-8, "{s:?}");
        assert!((s.points[&3][1]).abs() < 1e-6, "the second pole lines up with the line: {:?}", s.points[&3]);
        // A parabola arc with its ends held on it.
        let g = Geometry {
            points: vec![pt(1, 0.0, 0.0), pt(2, 0.0, 5.0), pt(3, -20.0, 18.0), pt(4, 15.0, 12.0)],
            curves: vec![Curve::ParabolaArc { id: 10, vertex: 1, focus: 2, start: 3, end: 4, construction: false }],
            constraints: vec![Constraint::Fixed { point: 1, x: 0.0, y: 0.0 }, Constraint::Fixed { point: 2, x: 0.0, y: 5.0 }],
        };
        let s = solve(&g, &dims(&[])).unwrap();
        assert!(s.residual < 1e-8);
        for p in [3, 4] {
            let q = s.points[&p];
            assert!((q[1] - q[0] * q[0] / 20.0).abs() < 1e-6, "on y = x²/20: {q:?}");
        }
        assert_eq!(s.free, 2, "each end slides along it");
    }

    #[test]
    fn malformed_constraints_are_named_not_fatal() {
        let mut g = rect();
        g.constraints.push(Constraint::Tangent { a: 10, b: 11, at: None });
        let s = solve(&g, &dims(&[])).unwrap();
        assert_eq!(s.malformed, vec![5]);
    }

    #[test]
    fn many_parts_solve_apart() {
        // 300 pinned points and a free rectangle: quick, and only the rectangle is free.
        let mut g = rect();
        for i in 0..300 {
            g.points.push(pt(1000 + i, i as f64, 7.0));
            g.constraints.push(Constraint::Fixed { point: 1000 + i, x: i as f64, y: 7.0 });
        }
        let t = std::time::Instant::now();
        let s = solve(&g, &dims(&[])).unwrap();
        assert!(t.elapsed().as_millis() < 2000, "{:?}", t.elapsed());
        assert_eq!(s.free, 2);
    }
}
