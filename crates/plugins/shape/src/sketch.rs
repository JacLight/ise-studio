//! Sketches: 2D geometry drawn by people, held exact by constraints.
//!
//! A sketch is points, lines, circles and arcs on a plane, plus constraints
//! (coincident, horizontal, vertical, parallel, perpendicular, equal,
//! tangent, point on curve, fixed, symmetric, and the steps of linear and
//! circular patterns) and dimensions. Every dimension is a
//! parameter of the sketch, so it can be typed, scrubbed, driven by a formula
//! (`=Desk size.width - 40`) or read by other shapes, the same as any other
//! parameter. Nothing follows anything implicitly.
//!
//! The solver moves the points as little as possible to satisfy every
//! constraint (damped least squares from where they are), so editing one
//! dimension leaves the rest of the drawing where it was. It reports how many
//! degrees of freedom remain, and which constraints conflict when they do.
//!
//! Closed loops become profiles: outer outlines counter-clockwise, holes
//! clockwise, arcs kept as arcs for the exact kernel.

use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Point {
    pub id: u32,
    pub x: f64,
    pub y: f64,
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
}

impl Curve {
    pub fn id(&self) -> u32 {
        match self {
            Curve::Line { id, .. } | Curve::Arc { id, .. } | Curve::Circle { id, .. } | Curve::Bezier { id, .. } => *id,
        }
    }
    pub fn construction(&self) -> bool {
        match self {
            Curve::Line { construction, .. } | Curve::Arc { construction, .. } | Curve::Circle { construction, .. } | Curve::Bezier { construction, .. } => *construction,
        }
    }
}

/// A dimension's value: a sketch parameter (by key), so it can be linked.
pub type DimKey = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    Coincident { a: u32, b: u32 },
    Horizontal { line: u32 },
    Vertical { line: u32 },
    Parallel { a: u32, b: u32 },
    Perpendicular { a: u32, b: u32 },
    /// Same length (lines) or radius (circles, arcs).
    Equal { a: u32, b: u32 },
    /// A point on a line (extended) or a circle or arc.
    OnCurve { point: u32, curve: u32 },
    /// A line tangent to a circle or arc, or two circles or arcs tangent; or,
    /// where two curves share an end (Béziers included), one smooth join.
    /// `at` names the shared point when they share both ends (a two-curve loop).
    Tangent {
        a: u32,
        b: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<u32>,
    },
    Midpoint { point: u32, line: u32 },
    /// Pins a point where it is.
    Fixed { point: u32, x: f64, y: f64 },
    /// Point-to-point distance, or a line's length (a = b = the line).
    Distance { a: u32, b: u32, key: DimKey },
    /// Signed X or Y offset from point a to point b.
    HorizontalDistance { a: u32, b: u32, key: DimKey },
    VerticalDistance { a: u32, b: u32, key: DimKey },
    Length { line: u32, key: DimKey },
    Radius { curve: u32, key: DimKey },
    Diameter { curve: u32, key: DimKey },
    /// Angle from line a to line b, degrees.
    Angle { a: u32, b: u32, key: DimKey },
    /// Point b is point a moved by line `along` (its start to its end): one
    /// step of a linear pattern. Editing the original moves its copies.
    Translate { a: u32, b: u32, along: u32 },
    /// Point b is point a turned about the start of line `from` by the angle
    /// from line `from` to line `to`: one step of a circular pattern.
    Rotate { a: u32, b: u32, from: u32, to: u32 },
    /// Points a and b mirror each other about line `line` (extended).
    Symmetric { a: u32, b: u32, line: u32 },
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
            | Constraint::Angle { key, .. } => Some(key),
            _ => None,
        }
    }
}

/// The geometry and constraints of a sketch (its plane and dimensions live on the component).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Geometry {
    pub points: Vec<Point>,
    pub curves: Vec<Curve>,
    pub constraints: Vec<Constraint>,
}

// ---- Solving -------------------------------------------------------------------

/// What solving found.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Solved {
    /// Points where the constraints put them.
    pub points: BTreeMap<u32, [f64; 2]>,
    /// Circle radii.
    pub radii: BTreeMap<u32, f64>,
    /// Degrees of freedom left (0 = fully constrained).
    pub free: usize,
    /// Indices of constraints that cannot all hold together (empty when solved).
    pub conflicts: Vec<usize>,
    /// Largest remaining error (mm).
    pub residual: f64,
}

/// Unknowns: every point's x and y, every circle's radius.
struct Vars {
    point: HashMap<u32, usize>,
    radius: HashMap<u32, usize>,
    values: Vec<f64>,
}

fn vars(g: &Geometry) -> Vars {
    let mut v = Vars { point: HashMap::new(), radius: HashMap::new(), values: Vec::new() };
    for p in &g.points {
        v.point.insert(p.id, v.values.len());
        v.values.extend([p.x, p.y]);
    }
    for c in &g.curves {
        if let Curve::Circle { id, radius, .. } = c {
            v.radius.insert(*id, v.values.len());
            v.values.push(*radius);
        }
    }
    v
}

struct Ctx<'a> {
    g: &'a Geometry,
    vars: &'a Vars,
    dims: &'a BTreeMap<String, f64>,
    curves: HashMap<u32, &'a Curve>,
}

impl Ctx<'_> {
    fn p(&self, x: &[f64], id: u32) -> Option<[f64; 2]> {
        let i = *self.vars.point.get(&id)?;
        Some([x[i], x[i + 1]])
    }
    fn line(&self, x: &[f64], id: u32) -> Option<([f64; 2], [f64; 2])> {
        match self.curves.get(&id)? {
            Curve::Line { a, b, .. } => Some((self.p(x, *a)?, self.p(x, *b)?)),
            _ => None,
        }
    }
    /// Center and radius of a circle or arc.
    fn round(&self, x: &[f64], id: u32) -> Option<([f64; 2], f64)> {
        match self.curves.get(&id)? {
            Curve::Circle { id, center, .. } => Some((self.p(x, *center)?, x[*self.vars.radius.get(id)?])),
            Curve::Arc { center, start, .. } => {
                let c = self.p(x, *center)?;
                let s = self.p(x, *start)?;
                Some((c, dist(c, s)))
            }
            _ => None,
        }
    }
    fn dim(&self, key: &str) -> Option<f64> {
        self.dims.get(key).copied()
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: [f64; 2]) -> f64 {
    dot(a, a).sqrt()
}

/// Residuals of one constraint (zero when it holds), in millimetres.
fn residual(ctx: &Ctx, x: &[f64], c: &Constraint, out: &mut Vec<f64>) -> Result<(), String> {
    let miss = || format!("a constraint refers to something that is not in the sketch: {c:?}");
    let unit = |v: [f64; 2]| {
        let l = len(v).max(1e-12);
        [v[0] / l, v[1] / l]
    };
    match c {
        Constraint::Coincident { a, b } => {
            let (p, q) = (ctx.p(x, *a).ok_or_else(miss)?, ctx.p(x, *b).ok_or_else(miss)?);
            out.extend([p[0] - q[0], p[1] - q[1]]);
        }
        Constraint::Horizontal { line } => {
            let (a, b) = ctx.line(x, *line).ok_or_else(miss)?;
            out.push(a[1] - b[1]);
        }
        Constraint::Vertical { line } => {
            let (a, b) = ctx.line(x, *line).ok_or_else(miss)?;
            out.push(a[0] - b[0]);
        }
        Constraint::Parallel { a, b } | Constraint::Perpendicular { a, b } => {
            let (p, q) = ctx.line(x, *a).ok_or_else(miss)?;
            let (r, s) = ctx.line(x, *b).ok_or_else(miss)?;
            let (u, v) = (unit(sub(q, p)), unit(sub(s, r)));
            // Scaled by a length so the residual is in millimetres, like the rest.
            let scale = len(sub(q, p)).min(len(sub(s, r))).max(1.0);
            out.push(if matches!(c, Constraint::Parallel { .. }) { cross(u, v) } else { dot(u, v) } * scale);
        }
        Constraint::Equal { a, b } => {
            let size = |id: u32| -> Option<f64> {
                match ctx.line(x, id) {
                    Some((p, q)) => Some(dist(p, q)),
                    None => ctx.round(x, id).map(|(_, r)| r),
                }
            };
            out.push(size(*a).ok_or_else(miss)? - size(*b).ok_or_else(miss)?);
        }
        Constraint::OnCurve { point, curve } => {
            let p = ctx.p(x, *point).ok_or_else(miss)?;
            if let Some((a, b)) = ctx.line(x, *curve) {
                out.push(cross(unit(sub(b, a)), sub(p, a)));
            } else {
                let (c, r) = ctx.round(x, *curve).ok_or_else(miss)?;
                out.push(dist(p, c) - r);
            }
        }
        Constraint::Tangent { a, b, at } => {
            // Where the two curves share an end point, tangency is a direction
            // at that point (the radius square to the line, or the centres in
            // line); that holds firmly. Apart, it is a distance.
            let ends = |id: u32| -> Vec<u32> {
                match ctx.curves.get(&id) {
                    Some(Curve::Line { a, b, .. }) | Some(Curve::Bezier { a, b, .. }) => vec![*a, *b],
                    Some(Curve::Arc { start, end, .. }) => vec![*start, *end],
                    _ => vec![],
                }
            };
            let shared = at.or_else(|| ends(*a).into_iter().find(|p| ends(*b).contains(p)));
            let is_bezier = |id: u32| matches!(ctx.curves.get(&id), Some(Curve::Bezier { .. }));
            if is_bezier(*a) || is_bezier(*b) {
                // A smooth join: the two curves leave the shared point along one line.
                let j = shared.ok_or("a Bézier is tangent only where it meets the other curve")?;
                let jp = ctx.p(x, j).ok_or_else(miss)?;
                let dir = |id: u32| -> Option<[f64; 2]> {
                    match ctx.curves.get(&id)? {
                        Curve::Line { a, b, .. } => Some(sub(ctx.p(x, *b)?, ctx.p(x, *a)?)),
                        Curve::Bezier { a, c1, c2, .. } => Some(if *a == j { sub(ctx.p(x, *c1)?, jp) } else { sub(jp, ctx.p(x, *c2)?) }),
                        Curve::Arc { center, .. } => {
                            let r = sub(jp, ctx.p(x, *center)?);
                            Some([-r[1], r[0]])
                        }
                        Curve::Circle { .. } => None,
                    }
                };
                let (u, v) = (dir(*a).ok_or_else(miss)?, dir(*b).ok_or_else(miss)?);
                let scale = len(u).min(len(v)).max(1.0);
                out.push(cross(unit(u), unit(v)) * scale);
                return Ok(());
            }
            match (ctx.line(x, *a), ctx.line(x, *b)) {
                (Some((p, q)), None) | (None, Some((p, q))) => {
                    let round = if ctx.line(x, *a).is_some() { *b } else { *a };
                    let (c, r) = ctx.round(x, round).ok_or_else(miss)?;
                    match shared.and_then(|id| ctx.p(x, id)) {
                        Some(j) => out.push(dot(unit(sub(q, p)), sub(c, j))),
                        None => out.push(cross(unit(sub(q, p)), sub(c, p)).abs() - r),
                    }
                }
                (None, None) => {
                    let (c1, r1) = ctx.round(x, *a).ok_or_else(miss)?;
                    let (c2, r2) = ctx.round(x, *b).ok_or_else(miss)?;
                    match shared.and_then(|id| ctx.p(x, id)) {
                        Some(j) => out.push(cross(unit(sub(c1, j)), sub(c2, j))),
                        None => {
                            // Outside tangency when apart, inside when one holds the other.
                            let d = dist(c1, c2);
                            out.push(if d > r1.max(r2) { d - (r1 + r2) } else { d - (r1 - r2).abs() });
                        }
                    }
                }
                _ => return Err("two lines cannot be tangent".into()),
            }
        }
        Constraint::Midpoint { point, line } => {
            let p = ctx.p(x, *point).ok_or_else(miss)?;
            let (a, b) = ctx.line(x, *line).ok_or_else(miss)?;
            out.extend([p[0] - (a[0] + b[0]) / 2.0, p[1] - (a[1] + b[1]) / 2.0]);
        }
        Constraint::Fixed { point, x: fx, y: fy } => {
            let p = ctx.p(x, *point).ok_or_else(miss)?;
            out.extend([p[0] - fx, p[1] - fy]);
        }
        Constraint::Distance { a, b, key } => {
            let v = ctx.dim(key).ok_or_else(|| format!("no dimension “{key}”"))?;
            let d = if a == b {
                let (p, q) = ctx.line(x, *a).ok_or_else(miss)?;
                dist(p, q)
            } else {
                dist(ctx.p(x, *a).ok_or_else(miss)?, ctx.p(x, *b).ok_or_else(miss)?)
            };
            out.push(d - v);
        }
        Constraint::HorizontalDistance { a, b, key } | Constraint::VerticalDistance { a, b, key } => {
            let v = ctx.dim(key).ok_or_else(|| format!("no dimension “{key}”"))?;
            let (p, q) = (ctx.p(x, *a).ok_or_else(miss)?, ctx.p(x, *b).ok_or_else(miss)?);
            let k = if matches!(c, Constraint::HorizontalDistance { .. }) { 0 } else { 1 };
            out.push((q[k] - p[k]) - v);
        }
        Constraint::Length { line, key } => {
            let v = ctx.dim(key).ok_or_else(|| format!("no dimension “{key}”"))?;
            let (p, q) = ctx.line(x, *line).ok_or_else(miss)?;
            out.push(dist(p, q) - v);
        }
        Constraint::Radius { curve, key } | Constraint::Diameter { curve, key } => {
            let v = ctx.dim(key).ok_or_else(|| format!("no dimension “{key}”"))?;
            let (_, r) = ctx.round(x, *curve).ok_or_else(miss)?;
            out.push(if matches!(c, Constraint::Radius { .. }) { r - v } else { 2.0 * r - v });
        }
        Constraint::Angle { a, b, key } => {
            let v = ctx.dim(key).ok_or_else(|| format!("no dimension “{key}”"))?.to_radians();
            let (p, q) = ctx.line(x, *a).ok_or_else(miss)?;
            let (r, s) = ctx.line(x, *b).ok_or_else(miss)?;
            let (u, w) = (sub(q, p), sub(s, r));
            let ang = cross(u, w).atan2(dot(u, w));
            // Wrapped difference, scaled to millimetres by the shorter line.
            let mut d = ang - v;
            while d > std::f64::consts::PI {
                d -= 2.0 * std::f64::consts::PI;
            }
            while d < -std::f64::consts::PI {
                d += 2.0 * std::f64::consts::PI;
            }
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
            let (s, e) = ctx.line(x, *line).ok_or_else(miss)?;
            let d = unit(sub(e, s));
            let m = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            out.extend([cross(d, sub(m, s)), dot(d, sub(q, p))]);
        }
    }
    Ok(())
}

/// Every residual, including the implicit one each arc carries (its end on its circle).
fn residuals(ctx: &Ctx, x: &[f64], owners: &mut Vec<Option<usize>>) -> Result<Vec<f64>, String> {
    let mut r = Vec::new();
    owners.clear();
    for (i, c) in ctx.g.constraints.iter().enumerate() {
        let before = r.len();
        residual(ctx, x, c, &mut r)?;
        owners.extend(std::iter::repeat_n(Some(i), r.len() - before));
    }
    for c in &ctx.g.curves {
        if let Curve::Arc { center, start, end, .. } = c {
            let (cc, s, e) = (ctx.p(x, *center).ok_or("arc without center")?, ctx.p(x, *start).ok_or("arc without start")?, ctx.p(x, *end).ok_or("arc without end")?);
            r.push(dist(cc, e) - dist(cc, s));
            owners.push(None);
        }
    }
    Ok(r)
}

/// Solves `a x = b` (square, dense) by Gaussian elimination with partial pivoting.
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    Some(x)
}

/// Numerical rank of a matrix (rows of equal length), by elimination with tolerance.
fn rank(mut m: Vec<Vec<f64>>, tol: f64) -> usize {
    if m.is_empty() {
        return 0;
    }
    let cols = m[0].len();
    let mut r = 0;
    for col in 0..cols {
        let Some(pivot) = (r..m.len()).max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs())) else { break };
        if m[pivot][col].abs() <= tol {
            continue;
        }
        m.swap(r, pivot);
        for row in 0..m.len() {
            if row != r {
                let f = m[row][col] / m[r][col];
                if f != 0.0 {
                    for k in col..cols {
                        m[row][k] -= f * m[r][k];
                    }
                }
            }
        }
        r += 1;
        if r == m.len() {
            break;
        }
    }
    r
}

/// Solves a sketch: the smallest move of its points that satisfies every
/// constraint, given the dimensions' values (mm, degrees).
pub fn solve(g: &Geometry, dims: &BTreeMap<String, f64>) -> Result<Solved, String> {
    let v = vars(g);
    let ctx = Ctx { g, vars: &v, dims, curves: g.curves.iter().map(|c| (c.id(), c)).collect() };
    let n = v.values.len();
    let mut x = v.values.clone();
    let mut owners = Vec::new();
    let jacobian = |x: &[f64], r0: &[f64], owners: &mut Vec<Option<usize>>| -> Result<Vec<Vec<f64>>, String> {
        let mut j = vec![vec![0.0; n]; r0.len()];
        let mut xp = x.to_vec();
        for k in 0..n {
            let h = 1e-7 * x[k].abs().max(1.0);
            xp[k] = x[k] + h;
            let r1 = residuals(&ctx, &xp, owners)?;
            xp[k] = x[k];
            for (row, (a, b)) in r1.iter().zip(r0).enumerate() {
                j[row][k] = (a - b) / h;
            }
        }
        Ok(j)
    };

    let mut r = residuals(&ctx, &x, &mut owners)?;
    let norm = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut lambda = 1e-3;
    for _ in 0..200 {
        if norm(&r).sqrt() < 1e-10 || r.is_empty() {
            break;
        }
        let j = jacobian(&x, &r, &mut owners)?;
        // Damped least squares, minimum-norm step: (JᵀJ + λI) dx = −Jᵀr.
        let mut a = vec![vec![0.0; n]; n];
        let mut b = vec![0.0; n];
        for (row, ri) in j.iter().zip(&r) {
            for p in 0..n {
                if row[p] == 0.0 {
                    continue;
                }
                b[p] -= row[p] * ri;
                for q in 0..n {
                    a[p][q] += row[p] * row[q];
                }
            }
        }
        let mut improved = false;
        for _ in 0..12 {
            let mut damped = a.clone();
            for (i, row) in damped.iter_mut().enumerate() {
                row[i] += lambda * (1.0 + a[i][i]);
            }
            let Some(dx) = solve_linear(damped, b.clone()) else {
                lambda *= 10.0;
                continue;
            };
            let trial: Vec<f64> = x.iter().zip(&dx).map(|(a, d)| a + d).collect();
            let rt = residuals(&ctx, &trial, &mut owners)?;
            if norm(&rt) < norm(&r) {
                x = trial;
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

    let residual = r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let j = jacobian(&x, &r, &mut owners)?;
    let free = n.saturating_sub(rank(j.clone(), 1e-6));
    // Conflicts: the constraints still unmet.
    let mut conflicts: Vec<usize> = if residual > 1e-6 {
        r.iter().zip(&owners).filter(|(v, _)| v.abs() > 1e-6).filter_map(|(_, o)| *o).collect()
    } else {
        Vec::new()
    };
    conflicts.dedup();
    let mut points = BTreeMap::new();
    for p in &g.points {
        let i = v.point[&p.id];
        points.insert(p.id, [x[i], x[i + 1]]);
    }
    let radii = v.radius.iter().map(|(id, i)| (*id, x[*i])).collect();
    Ok(Solved { points, radii, free, conflicts, residual })
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
    Some(solve(&s.geometry, &dims).and_then(|solved| {
        if !solved.conflicts.is_empty() {
            let names: Vec<String> = solved.conflicts.iter().filter_map(|i| s.geometry.constraints.get(*i)).map(|c| format!("{c:?}")).collect();
            return Err(format!("these constraints cannot all hold: {}", names.join("; ")));
        }
        let (loops, open) = loops(&s.geometry, &solved);
        Ok(SolvedSketch { solved, loops, open })
    }))
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

fn check_geometry(g: &Geometry, params: &[Param]) -> Result<(), CommandError> {
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
        let refs = match c {
            Curve::Line { a, b, .. } => vec![*a, *b],
            Curve::Bezier { a, b, c1, c2, .. } => vec![*a, *b, *c1, *c2],
            Curve::Arc { center, start, end, .. } => vec![*center, *start, *end],
            Curve::Circle { center, radius, .. } => {
                if !(*radius > 0.0) {
                    return bad(format!("circle {} needs a positive radius", c.id()));
                }
                vec![*center]
            }
        };
        if let Some(p) = refs.iter().find(|p| !points.contains(p)) {
            return bad(format!("curve {} uses point {p}, which is not in the sketch", c.id()));
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
}

impl PlaneSpec {
    fn plane(self) -> Result<Plane, CommandError> {
        let p = match self {
            PlaneSpec::Named { name, offset } => preset_plane(&name, offset).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?,
            PlaneSpec::Full(p) => p,
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
        solve(&s.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        tx.set(self.target, s)?;
        Ok(json!({ "id": self.target }))
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
    Ok(out)
}

/// What a program reads from its sketches, as the runtime receives it (JSON).
pub fn inputs(world: &World, uses: &BTreeMap<String, StableId>) -> Result<String, String> {
    let mut m = serde_json::Map::new();
    for (name, id) in uses {
        let e = world.resource::<IdIndex>().get(*id).ok_or_else(|| format!("sketch “{name}” was deleted"))?;
        let sketch = world.get::<Sketch>(e).ok_or_else(|| format!("“{name}” is not a sketch"))?;
        let solved = world.get::<SolvedSketch>(e).ok_or_else(|| format!("sketch “{name}” has no valid solution"))?;
        m.insert(name.clone(), json!({ "plane": sketch.plane, "loops": solved.loops, "open": solved.open }));
    }
    Ok(serde_json::Value::Object(m).to_string())
}
