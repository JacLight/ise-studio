//! Projecting solids into a sketch: the outline a part shows looking
//! straight at the sketch plane (its edges and the silhouettes of its curved
//! faces, by OpenCascade's exact hidden-line removal), or the section where
//! the plane cuts it. Each projected edge becomes a real sketch curve: a
//! line, arc or circle where it is one (to within a micron), else cubic
//! Béziers fitted within the display tolerance. By default the curves are
//! reference geometry, pinned where the part put them, to draw against.

use crate::exact::evaluate_exact_in;
use crate::sketch::{Constraint, Curve, Point};
use crate::trim::{Ctx, P, bad, dist, dot, sub};
use de_core::schemars::{self, JsonSchema};
use de_core::{Command, CommandError, ErrorCode, IdIndex, StableId, Tx, Value, json};
use de_occt::Arena;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMode {
    /// What the part shows looking straight at the sketch plane.
    #[default]
    Outline,
    /// Where the sketch plane cuts the part.
    Section,
}

/// One shape or several.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Sources {
    One(StableId),
    Many(Vec<StableId>),
}

fn yes() -> bool {
    true
}

/// Projects parts' edges onto a sketch's plane as sketch curves.
#[derive(Deserialize, JsonSchema)]
pub struct ProjectSketch {
    pub target: StableId,
    /// The shape (or shapes) to project.
    pub from: Sources,
    /// Only this part of a program that makes several.
    pub part: Option<String>,
    #[serde(default)]
    pub mode: ProjectMode,
    /// Outline: also the edges hidden behind the part (default only what shows).
    #[serde(default)]
    pub hidden: bool,
    /// Make them construction curves (guides that never form a profile).
    #[serde(default)]
    pub construction: bool,
    /// Pin them where the part put them, as reference geometry (default true).
    #[serde(default = "yes")]
    pub fixed: bool,
}

/// A projected edge as sketch geometry.
#[derive(Clone, Debug)]
enum Fit {
    Line(P, P),
    /// Centre, start, end: anticlockwise from start to end.
    Arc(P, P, P),
    Circle(P, f64),
    Bezier([P; 4]),
}

fn len(v: P) -> f64 {
    v[0].hypot(v[1])
}

fn unit(v: P) -> P {
    let l = len(v).max(1e-300);
    [v[0] / l, v[1] / l]
}

/// The circle through three points.
fn circumcircle(a: P, b: P, c: P) -> Option<(P, f64)> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() < 1e-12 {
        return None;
    }
    let (a2, b2, c2) = (dot(a, a), dot(b, b), dot(c, c));
    let x = (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d;
    let y = (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d;
    Some(([x, y], dist([x, y], a)))
}

fn bez(p: &[P; 4], t: f64) -> P {
    crate::sketch::bezier_at(p[0], p[1], p[2], p[3], t)
}

/// A polyline sampled on a true curve, as the simplest exact sketch curves:
/// a line or a circle or arc when every sample lies on one, else Béziers.
fn fit(points: &[P], tolerance: f64) -> Vec<Fit> {
    let mut pts: Vec<P> = Vec::with_capacity(points.len());
    for p in points {
        if pts.last().is_none_or(|q| dist(*q, *p) > 1e-9) {
            pts.push(*p);
        }
    }
    if pts.len() < 2 {
        return vec![];
    }
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    let span = pts.iter().fold(0.0f64, |m, p| m.max(dist(*p, first)));
    if span < 1e-6 {
        return vec![];
    }
    let exact = 1e-6 * span.max(1.0);
    let closed = pts.len() > 3 && dist(first, last) < exact;
    // A line: every sample on the chord.
    if !closed {
        let d = unit(sub(last, first));
        if pts.iter().all(|p| crate::trim::cross(d, sub(*p, first)).abs() < exact) {
            return vec![Fit::Line(first, last)];
        }
    }
    // A circle or arc: every sample the same distance from one centre.
    if pts.len() >= 3 {
        let n = pts.len() - usize::from(closed);
        let (i, j, k) = if closed { (0, n / 3, 2 * n / 3) } else { (0, (pts.len() - 1) / 2, pts.len() - 1) };
        if let Some((c, r)) = circumcircle(pts[i], pts[j], pts[k])
            && r < 1e6 * span.max(1.0)
            && pts.iter().all(|p| (dist(*p, c) - r).abs() < 1e-5 * r.max(1.0))
        {
            if closed {
                return vec![Fit::Circle(c, r)];
            }
            // Which way round the samples run.
            let mut turn = 0.0;
            for w in pts.windows(2) {
                let (a0, a1) = ((w[0][1] - c[1]).atan2(w[0][0] - c[0]), (w[1][1] - c[1]).atan2(w[1][0] - c[0]));
                turn += (a1 - a0 + PI).rem_euclid(TAU) - PI;
            }
            return vec![if turn > 0.0 { Fit::Arc(c, first, last) } else { Fit::Arc(c, last, first) }];
        }
    }
    let t1 = unit(sub(pts[1], pts[0]));
    let t2 = unit(sub(pts[pts.len() - 2], pts[pts.len() - 1]));
    let mut out = vec![];
    fit_cubic(&pts, t1, t2, tolerance.max(1e-4), 0, &mut out);
    out.into_iter().map(Fit::Bezier).collect()
}

/// Cubic Béziers through samples within `err` (after Schneider's "An
/// algorithm for automatically fitting digitized curves", Graphics Gems 1990).
fn fit_cubic(pts: &[P], t1: P, t2: P, err: f64, depth: usize, out: &mut Vec<[P; 4]>) {
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let chord = dist(a, b);
    if pts.len() == 2 {
        out.push([a, [a[0] + t1[0] * chord / 3.0, a[1] + t1[1] * chord / 3.0], [b[0] + t2[0] * chord / 3.0, b[1] + t2[1] * chord / 3.0], b]);
        return;
    }
    // Parameters by distance along the samples.
    let mut u = vec![0.0];
    for w in pts.windows(2) {
        u.push(u.last().unwrap() + dist(w[0], w[1]));
    }
    let total = *u.last().unwrap();
    for v in &mut u {
        *v /= total.max(1e-300);
    }
    let mut curve = generate(pts, &u, t1, t2);
    let (mut worst, mut split) = max_error(pts, &curve, &u);
    if worst > err && worst < err * 16.0 {
        // Close: improve the parameters (Newton steps) and try again.
        for _ in 0..6 {
            for (i, p) in pts.iter().enumerate() {
                u[i] = newton(&curve, *p, u[i]);
            }
            curve = generate(pts, &u, t1, t2);
            (worst, split) = max_error(pts, &curve, &u);
            if worst <= err {
                break;
            }
        }
    }
    if worst <= err || depth > 24 {
        out.push(curve);
        return;
    }
    let split = split.clamp(1, pts.len() - 2);
    let tc = unit(sub(pts[split - 1], pts[split + 1]));
    fit_cubic(&pts[..=split], t1, tc, err, depth + 1, out);
    fit_cubic(&pts[split..], [-tc[0], -tc[1]], t2, err, depth + 1, out);
}

fn generate(pts: &[P], u: &[f64], t1: P, t2: P) -> [P; 4] {
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (p, &t) in pts.iter().zip(u) {
        let s = 1.0 - t;
        let (b0, b1, b2, b3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
        let (a1, a2) = ([t1[0] * b1, t1[1] * b1], [t2[0] * b2, t2[1] * b2]);
        c00 += dot(a1, a1);
        c01 += dot(a1, a2);
        c11 += dot(a2, a2);
        let tmp = sub(*p, [a[0] * (b0 + b1) + b[0] * (b2 + b3), a[1] * (b0 + b1) + b[1] * (b2 + b3)]);
        x0 += dot(a1, tmp);
        x1 += dot(a2, tmp);
    }
    let det = c00 * c11 - c01 * c01;
    let chord = dist(a, b);
    let (mut al, mut ar) = if det.abs() > 1e-12 { ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det) } else { (chord / 3.0, chord / 3.0) };
    if al < 1e-6 * chord || ar < 1e-6 * chord {
        (al, ar) = (chord / 3.0, chord / 3.0);
    }
    [a, [a[0] + t1[0] * al, a[1] + t1[1] * al], [b[0] + t2[0] * ar, b[1] + t2[1] * ar], b]
}

fn max_error(pts: &[P], c: &[P; 4], u: &[f64]) -> (f64, usize) {
    let mut worst = (0.0, pts.len() / 2);
    for (i, (p, &t)) in pts.iter().zip(u).enumerate().skip(1).take(pts.len().saturating_sub(2)) {
        let d = dist(bez(c, t), *p);
        if d > worst.0 {
            worst = (d, i);
        }
    }
    worst
}

fn newton(c: &[P; 4], p: P, t: f64) -> f64 {
    let d1: Vec<P> = (0..3).map(|i| [3.0 * (c[i + 1][0] - c[i][0]), 3.0 * (c[i + 1][1] - c[i][1])]).collect();
    let d2: Vec<P> = (0..2).map(|i| [2.0 * (d1[i + 1][0] - d1[i][0]), 2.0 * (d1[i + 1][1] - d1[i][1])]).collect();
    let q = bez(c, t);
    let s = 1.0 - t;
    let q1 = [d1[0][0] * s * s + 2.0 * d1[1][0] * s * t + d1[2][0] * t * t, d1[0][1] * s * s + 2.0 * d1[1][1] * s * t + d1[2][1] * t * t];
    let q2 = [d2[0][0] * s + d2[1][0] * t, d2[0][1] * s + d2[1][1] * t];
    let diff = sub(q, p);
    let den = dot(q1, q1) + dot(diff, q2);
    if den.abs() < 1e-12 { t } else { (t - dot(diff, q1) / den).clamp(0.0, 1.0) }
}

/// Whether two fitted curves are the same (either way round).
fn same(a: &Fit, b: &Fit) -> bool {
    let e = 1e-6;
    let close = |p: P, q: P| dist(p, q) < e;
    match (a, b) {
        (Fit::Line(p, q), Fit::Line(r, s)) => (close(*p, *r) && close(*q, *s)) || (close(*p, *s) && close(*q, *r)),
        (Fit::Arc(c, s, t), Fit::Arc(d, u, v)) => close(*c, *d) && close(*s, *u) && close(*t, *v),
        (Fit::Circle(c, r), Fit::Circle(d, s)) => close(*c, *d) && (r - s).abs() < e,
        (Fit::Bezier(p), Fit::Bezier(q)) => (0..4).all(|i| close(p[i], q[i])) || (0..4).all(|i| close(p[i], q[3 - i])),
        _ => false,
    }
}

impl Command for ProjectSketch {
    const NAME: &'static str = "sketch.project";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let plane = cx.sketch.plane.clone();
        let n = plane.normal();
        let ids = match &self.from {
            Sources::One(id) => vec![*id],
            Sources::Many(v) => v.clone(),
        };
        if ids.is_empty() {
            return Err(bad("pick a part to project"));
        }
        // Every part, exactly, where it stands.
        let mut arena = Arena::new();
        let mut shapes = vec![];
        let mut tolerance: f64 = 1.0;
        for id in &ids {
            let world = tx.world();
            let entity = world.resource::<IdIndex>().get(*id).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} does not exist")))?;
            let program = crate::resolved_program(world, entity).ok_or_else(|| bad(format!("{id} is not a part made by a program: only those project")))?;
            let at = tx.get::<de_solid::Placement>(*id).map(|p| [p.x, p.y, p.z].map(|l| l.canonical() * 1000.0)).unwrap_or([0.0; 3]);
            let values: Vec<(String, f64)> = program.params.iter().map(|p| (p.key.clone(), p.value)).collect();
            tolerance = tolerance.min(program.display_tolerance());
            let (back, exact) = evaluate_exact_in(arena, &program.name, &program.source, &values, program.display_tolerance(), &program.inputs).map_err(|(_, e)| CommandError::new(ErrorCode::InvalidGeometry, format!("{}: {e}", program.name)))?;
            arena = back;
            let mut found = false;
            for p in &exact.parts {
                if self.part.as_ref().is_some_and(|want| *want != p.name) {
                    continue;
                }
                found = true;
                let s = if at.iter().any(|v| v.abs() > 0.0) { arena.translate(p.shape, at).map_err(bad)? } else { p.shape };
                shapes.push(s);
            }
            if !found {
                return Err(bad(format!("{} has no part named {}", program.name, self.part.clone().unwrap_or_default())));
            }
        }
        let all = if shapes.len() == 1 { shapes[0] } else { arena.compound(&shapes).map_err(bad)? };
        // Polylines on the true edges, in sketch coordinates.
        let ox = dot3(plane.origin, plane.x);
        let oy = dot3(plane.origin, plane.y);
        let polylines: Vec<Vec<P>> = match self.mode {
            ProjectMode::Outline => {
                // Looking at the plane from its front (along −normal), x across and y up: the view's axes are the sketch's.
                let lines = arena.hlr_view(all, [-n[0], -n[1], -n[2]], plane.y, tolerance).map_err(bad)?;
                lines.into_iter().filter(|l| self.hidden || !l.hidden).map(|l| l.points.into_iter().map(|p| [p[0] - ox, p[1] - oy]).collect()).collect()
            }
            ProjectMode::Section => {
                let cut = arena.section(all, plane.origin, n).map_err(bad)?;
                let edges = arena.edges(cut, tolerance).map_err(bad)?;
                edges
                    .into_iter()
                    .filter(|e| e.points.len() >= 2)
                    .map(|e| e.points.into_iter().map(|p| [dot3(p, plane.x) - ox, dot3(p, plane.y) - oy]).collect())
                    .collect()
            }
        };
        let mut fits: Vec<Fit> = vec![];
        for pl in &polylines {
            for f in fit(pl, tolerance) {
                if !fits.iter().any(|g| same(g, &f)) {
                    fits.push(f);
                }
            }
        }
        if fits.is_empty() {
            return Err(bad(match self.mode {
                ProjectMode::Outline => "nothing to project: the part shows no edges from this plane",
                ProjectMode::Section => "the sketch plane does not cut the part",
            }));
        }
        // Into the sketch: ends shared where projected edges meet.
        let first_new = cx.sketch.geometry.points.len();
        let construction = self.construction;
        let mut made = vec![];
        let mut new_points: BTreeSet<u32> = BTreeSet::new();
        let end = |cx: &mut Ctx, at: P, share: bool, new_points: &mut BTreeSet<u32>| -> u32 {
            if share && let Some(p) = cx.sketch.geometry.points[first_new..].iter().find(|p| dist([p.x, p.y], at) < 1e-6) {
                return p.id;
            }
            let id = cx.point(at);
            new_points.insert(id);
            id
        };
        let mut level = vec![];
        let mut circles = vec![];
        for f in &fits {
            let id;
            let curve = match *f {
                Fit::Line(a, b) => {
                    let (pa, pb) = (end(&mut cx, a, true, &mut new_points), end(&mut cx, b, true, &mut new_points));
                    id = cx.id();
                    let d = sub(b, a);
                    if d[1].abs() < 1e-9 * len(d) {
                        level.push(Constraint::Horizontal { line: id });
                    } else if d[0].abs() < 1e-9 * len(d) {
                        level.push(Constraint::Vertical { line: id });
                    }
                    Curve::Line { id, a: pa, b: pb, construction }
                }
                Fit::Arc(c, s, e) => {
                    let (ps, pe) = (end(&mut cx, s, true, &mut new_points), end(&mut cx, e, true, &mut new_points));
                    let pc = end(&mut cx, c, false, &mut new_points);
                    id = cx.id();
                    Curve::Arc { id, center: pc, start: ps, end: pe, construction }
                }
                Fit::Circle(c, r) => {
                    let pc = end(&mut cx, c, false, &mut new_points);
                    id = cx.id();
                    circles.push((id, r));
                    Curve::Circle { id, center: pc, radius: r, construction }
                }
                Fit::Bezier(q) => {
                    let (pa, pb) = (end(&mut cx, q[0], true, &mut new_points), end(&mut cx, q[3], true, &mut new_points));
                    let (h1, h2) = (end(&mut cx, q[1], false, &mut new_points), end(&mut cx, q[2], false, &mut new_points));
                    id = cx.id();
                    Curve::Bezier { id, a: pa, b: pb, c1: h1, c2: h2, construction }
                }
            };
            cx.sketch.geometry.curves.push(curve);
            made.push(id);
        }
        if self.fixed {
            // Reference geometry: pinned, a circle's size held by a dimension.
            let pins: Vec<Constraint> = cx.sketch.geometry.points.iter().filter(|p| new_points.contains(&p.id)).map(|&Point { id, x, y }| Constraint::Fixed { point: id, x, y }).collect();
            cx.sketch.geometry.constraints.extend(pins);
            for (id, r) in circles {
                let key = cx.dimension("Projected ⌀", 2.0 * r, "mm");
                cx.sketch.geometry.constraints.push(Constraint::Diameter { curve: id, key });
            }
        } else {
            cx.sketch.geometry.constraints.extend(level);
        }
        let target = self.target;
        let count = made.len();
        cx.save(tx, target, &BTreeSet::new(), false, json!({ "id": target, "curves": made, "count": count }))
    }
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
