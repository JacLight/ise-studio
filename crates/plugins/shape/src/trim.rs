//! Trim and extend for sketches, as in a drafting program: trimming removes
//! the piece of a curve between the nearest crossings either side of where
//! you click (a lone piece goes entirely); extending carries a line's or an
//! arc's nearer end on until it meets another curve. New ends sit on the
//! curve that cut them (a constraint keeps them there), so the sketch still
//! holds together when it is changed later.

use crate::params;
use crate::pathfinder::{curve_points, refs};
use crate::sketch::{Constraint, Curve, Geometry, Point, Sketch, bezier_at, solve};
use de_core::schemars::{self, JsonSchema};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

/// Removes the piece of `curve` around `at` between its nearest crossings with other curves.
#[derive(Deserialize, JsonSchema)]
pub struct TrimSketch {
    pub target: StableId,
    pub curve: u32,
    /// Where on the curve (sketch mm): picks the piece to remove.
    pub at: [f64; 2],
}

/// Carries the end of a line or arc nearer `at` on to the next curve it meets.
#[derive(Deserialize, JsonSchema)]
pub struct ExtendSketch {
    pub target: StableId,
    pub curve: u32,
    pub at: [f64; 2],
}

pub(crate) fn bad(text: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, text.into())
}

pub(crate) type P = [f64; 2];
const EPS: f64 = 1e-7;
const BEZIER_STEPS: usize = 96;

pub(crate) fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
pub(crate) fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub(crate) fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
pub(crate) fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
pub(crate) fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
pub(crate) fn wrap(a: f64) -> f64 {
    a.rem_euclid(TAU)
}

/// A curve as plain geometry, for finding crossings.
#[derive(Clone, Debug)]
pub(crate) enum Prim {
    Seg(P, P),
    /// Centre, radius, start angle and counter-clockwise sweep (TAU for a full circle).
    Circ(P, f64, f64, f64),
    /// A Bézier, sampled.
    Poly(Vec<P>),
}

impl Prim {
    pub(crate) fn on_arc(&self, p: P) -> bool {
        match self {
            Prim::Circ(c, _, a0, sweep) => *sweep >= TAU - 1e-12 || wrap((p[1] - c[1]).atan2(p[0] - c[0]) - a0) <= sweep + 1e-9,
            _ => true,
        }
    }
    /// Where along the curve a point is: 0–1 for lines and Béziers, the angle past the start for circles and arcs.
    pub(crate) fn param(&self, p: P) -> f64 {
        match self {
            Prim::Seg(a, b) => {
                let d = sub(*b, *a);
                dot(sub(p, *a), d) / dot(d, d).max(1e-300)
            }
            Prim::Circ(c, _, a0, _) => wrap((p[1] - c[1]).atan2(p[0] - c[0]) - a0),
            Prim::Poly(pts) => {
                let (mut best, mut at) = (f64::INFINITY, 0.0);
                for (i, w) in pts.windows(2).enumerate() {
                    let d = sub(w[1], w[0]);
                    let u = (dot(sub(p, w[0]), d) / dot(d, d).max(1e-300)).clamp(0.0, 1.0);
                    let k = dist(p, lerp(w[0], w[1], u));
                    if k < best {
                        best = k;
                        at = (i as f64 + u) / (pts.len() - 1) as f64;
                    }
                }
                at
            }
        }
    }
    fn end(&self) -> f64 {
        match self {
            Prim::Circ(_, _, _, sweep) => *sweep,
            _ => 1.0,
        }
    }
    fn segments(&self) -> Vec<(P, P)> {
        match self {
            Prim::Seg(a, b) => vec![(*a, *b)],
            Prim::Poly(pts) => pts.windows(2).map(|w| (w[0], w[1])).collect(),
            Prim::Circ(..) => vec![],
        }
    }
}

/// Where a segment (as a whole line when `infinite`) meets another curve.
pub(crate) fn seg_hits(a: P, b: P, other: &Prim, infinite: bool) -> Vec<P> {
    let d = sub(b, a);
    let within = |t: f64| infinite || (-EPS..=1.0 + EPS).contains(&t);
    let mut out = vec![];
    match other {
        Prim::Circ(c, r, ..) => {
            let f = sub(a, *c);
            let (qa, qb, qc) = (dot(d, d), 2.0 * dot(f, d), dot(f, f) - r * r);
            let disc = qb * qb - 4.0 * qa * qc;
            if disc >= -1e-12 && qa > 1e-300 {
                let s = disc.max(0.0).sqrt();
                for t in [(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)] {
                    let p = lerp(a, b, t);
                    if within(t) && other.on_arc(p) {
                        out.push(p);
                    }
                }
            }
        }
        _ => {
            for (c, e) in other.segments() {
                let f = sub(e, c);
                let den = cross(d, f);
                if den.abs() < 1e-14 {
                    continue;
                }
                let t = cross(sub(c, a), f) / den;
                let u = cross(sub(c, a), d) / den;
                if within(t) && (-EPS..=1.0 + EPS).contains(&u) {
                    out.push(lerp(a, b, t));
                }
            }
        }
    }
    out
}

pub(crate) fn circ_hits(c1: P, r1: f64, other: &Prim) -> Vec<P> {
    match other {
        Prim::Circ(c2, r2, ..) => {
            let d = dist(c1, *c2);
            if d < 1e-12 || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
                return vec![];
            }
            let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = (r1 * r1 - a * a).max(0.0).sqrt();
            let m = lerp(c1, *c2, a / d);
            let n = [-(c2[1] - c1[1]) / d, (c2[0] - c1[0]) / d];
            let pts = if h < 1e-9 { vec![m] } else { vec![[m[0] + n[0] * h, m[1] + n[1] * h], [m[0] - n[0] * h, m[1] - n[1] * h]] };
            pts.into_iter().filter(|p| other.on_arc(*p)).collect()
        }
        _ => other.segments().iter().flat_map(|(a, b)| seg_hits(*a, *b, &Prim::Circ(c1, r1, 0.0, TAU), false)).collect(),
    }
}

/// Every crossing of `t` with `other`, as (param on t, point).
fn crossings(t: &Prim, other: &Prim) -> Vec<(f64, P)> {
    let pts: Vec<P> = match t {
        Prim::Seg(a, b) => seg_hits(*a, *b, other, false),
        Prim::Circ(c, r, ..) => circ_hits(*c, *r, other).into_iter().filter(|p| t.on_arc(*p)).collect(),
        Prim::Poly(_) => t.segments().iter().flat_map(|(a, b)| seg_hits(*a, *b, other, false)).collect(),
    };
    pts.into_iter().map(|p| (t.param(p), p)).collect()
}

pub(crate) struct Ctx {
    pub(crate) sketch: Sketch,
    pub(crate) dims: BTreeMap<String, f64>,
    pub(crate) at: BTreeMap<u32, P>,
    pub(crate) radii: BTreeMap<u32, f64>,
    next: u32,
    /// Points some curve used before the change.
    used_before: BTreeSet<u32>,
    /// Every constraint held before the change.
    met_before: bool,
}

impl Ctx {
    pub(crate) fn load(tx: &Tx, target: StableId) -> Result<Ctx, CommandError> {
        let mut sketch = tx.get::<Sketch>(target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a sketch")))?;
        let dims = params::resolve(tx.world(), &sketch.params).map_err(bad)?;
        let solved = solve(&sketch.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        // Start from where things are now.
        for p in &mut sketch.geometry.points {
            if let Some(q) = solved.points.get(&p.id) {
                (p.x, p.y) = (q[0], q[1]);
            }
        }
        let next = sketch.geometry.points.iter().map(|p| p.id).chain(sketch.geometry.curves.iter().map(Curve::id)).max().unwrap_or(0) + 1;
        let used_before = sketch.geometry.curves.iter().flat_map(curve_points).collect();
        let met_before = solved.conflicts.is_empty();
        Ok(Ctx { sketch, dims, at: solved.points, radii: solved.radii, next, used_before, met_before })
    }
    pub(crate) fn p(&self, id: u32) -> P {
        self.at[&id]
    }
    pub(crate) fn id(&mut self) -> u32 {
        self.next += 1;
        self.next - 1
    }
    pub(crate) fn point(&mut self, p: P) -> u32 {
        let id = self.id();
        self.sketch.geometry.points.push(Point { id, x: p[0], y: p[1] });
        self.at.insert(id, p);
        id
    }
    pub(crate) fn prim(&self, c: &Curve) -> Prim {
        match c {
            Curve::Line { a, b, .. } => Prim::Seg(self.p(*a), self.p(*b)),
            Curve::Circle { id, center, radius, .. } => Prim::Circ(self.p(*center), self.radii.get(id).copied().unwrap_or(*radius), 0.0, TAU),
            Curve::Arc { center, start, end, .. } => {
                let (c, s, e) = (self.p(*center), self.p(*start), self.p(*end));
                let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
                let sweep = wrap((e[1] - c[1]).atan2(e[0] - c[0]) - a0);
                Prim::Circ(c, dist(c, s), a0, if sweep < 1e-12 { TAU } else { sweep })
            }
            Curve::Bezier { a, b, c1, c2, .. } => {
                let (p0, p1, p2, p3) = (self.p(*a), self.p(*c1), self.p(*c2), self.p(*b));
                Prim::Poly((0..=BEZIER_STEPS).map(|i| bezier_at(p0, p1, p2, p3, i as f64 / BEZIER_STEPS as f64)).collect())
            }
        }
    }
    /// Crossings of curve `id` with every other curve: (param, point, the other curve).
    fn crossings(&self, id: u32) -> Vec<(f64, P, u32)> {
        let g = &self.sketch.geometry;
        let me = g.curves.iter().find(|c| c.id() == id).unwrap();
        let t = self.prim(me);
        let mut out = vec![];
        for c in g.curves.iter().filter(|c| c.id() != id) {
            for (s, p) in crossings(&t, &self.prim(c)) {
                out.push((s, p, c.id()));
            }
        }
        out
    }
    /// A new end at `p` on curve `on`: an existing end of `on` if it is there, else a new point held on `on`.
    pub(crate) fn end_on(&mut self, p: P, on: u32) -> u32 {
        let c = self.sketch.geometry.curves.iter().find(|c| c.id() == on).cloned().unwrap();
        let ends: Vec<u32> = match &c {
            Curve::Line { a, b, .. } | Curve::Bezier { a, b, .. } => vec![*a, *b],
            Curve::Arc { start, end, .. } => vec![*start, *end],
            Curve::Circle { .. } => vec![],
        };
        if let Some(e) = ends.into_iter().find(|e| dist(self.p(*e), p) < 1e-6) {
            return e;
        }
        let id = self.point(p);
        if !matches!(c, Curve::Bezier { .. }) {
            self.sketch.geometry.constraints.push(Constraint::OnCurve { point: id, curve: on });
        }
        id
    }
    fn finish(mut self, tx: &mut Tx, target: StableId, changed: u32, made: Vec<u32>) -> Result<Value, CommandError> {
        // What changed shape loses the constraints that fixed its old size or
        // ends; points no curve uses any more go, with what referred to them.
        let g = &mut self.sketch.geometry;
        g.constraints.retain(|c| {
            !(matches!(c, Constraint::Length { line, .. } if *line == changed)
                || matches!(c, Constraint::Distance { a, b, .. } if *a == changed && *b == changed)
                || matches!(c, Constraint::Midpoint { line, .. } if *line == changed)
                || matches!(c, Constraint::Equal { a, b } if (*a == changed || *b == changed) && g.curves.iter().any(|k| k.id() == changed && matches!(k, Curve::Line { .. }))))
        });
        self.save(tx, target, &BTreeSet::new(), false, json!({ "id": target, "curves": made }))
    }

    /// A new dimension of the sketch (key d1, d2, … as the editor names
    /// them), `label` numbered after it. Returns its key.
    pub(crate) fn dimension(&mut self, label: &str, value: f64, unit: &str) -> String {
        let mut n = 1;
        while self.sketch.params.iter().any(|p| p.key == format!("d{n}")) {
            n += 1;
        }
        let key = format!("d{n}");
        self.sketch.params.push(crate::Param { key: key.clone(), label: format!("{label} {n}"), value, unit: unit.into(), min: (unit == "mm").then_some(0.001), max: None, step: None, expr: None });
        self.dims.insert(key.clone(), value);
        key
    }

    /// Tidies up and stores the sketch: points no curve uses any more go
    /// (except `keep`), and constraints on anything gone go with it, with the
    /// dimensions only they carried. `strict` refuses a change that leaves
    /// constraints unmet that were met before.
    pub(crate) fn save(mut self, tx: &mut Tx, target: StableId, keep: &BTreeSet<u32>, strict: bool, out: Value) -> Result<Value, CommandError> {
        let g = &mut self.sketch.geometry;
        let used: BTreeSet<u32> = g.curves.iter().flat_map(curve_points).collect();
        let gone_points: BTreeSet<u32> = self.used_before.iter().copied().filter(|p| !used.contains(p) && !keep.contains(p)).collect();
        let curves: BTreeSet<u32> = g.curves.iter().map(Curve::id).collect();
        let points: BTreeSet<u32> = g.points.iter().map(|p| p.id).filter(|p| !gone_points.contains(p)).collect();
        g.points.retain(|p| !gone_points.contains(&p.id));
        // Constraints on removed curves or points go with them.
        g.constraints.retain(|c| refs(c).iter().all(|r| curves.contains(r) || points.contains(r)));
        let keys: BTreeSet<String> = g.constraints.iter().filter_map(|c| c.key().map(str::to_string)).collect();
        self.sketch.params.retain(|p| keys.contains(&p.key));
        let solved = solve(&self.sketch.geometry, &self.dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        if strict && !solved.conflicts.is_empty() && self.met_before {
            return Err(CommandError::new(ErrorCode::InvalidGeometry, "that cannot hold together with the sketch's other constraints and dimensions"));
        }
        // Store where everything now is, so the drawing is right before the next solve.
        for p in &mut self.sketch.geometry.points {
            if let Some(q) = solved.points.get(&p.id) {
                (p.x, p.y) = (q[0], q[1]);
            }
        }
        tx.set(target, self.sketch)?;
        Ok(out)
    }
}

/// A Bézier's control points split at t: (left, right).
fn split_bezier(p: [P; 4], t: f64) -> ([P; 4], [P; 4]) {
    let (a, b, c) = (lerp(p[0], p[1], t), lerp(p[1], p[2], t), lerp(p[2], p[3], t));
    let (d, e) = (lerp(a, b, t), lerp(b, c, t));
    let f = lerp(d, e, t);
    ([p[0], a, d, f], [f, e, c, p[3]])
}

impl Command for TrimSketch {
    const NAME: &'static str = "sketch.trim";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let curve = cx.sketch.geometry.curves.iter().find(|c| c.id() == self.curve).cloned().ok_or_else(|| bad(format!("no curve {}", self.curve)))?;
        let prim = cx.prim(&curve);
        let click = prim.param(self.at).clamp(0.0, prim.end());
        let full = matches!(curve, Curve::Circle { .. });
        // Crossings strictly inside the curve (its own ends do not cut it).
        let mut cuts: Vec<(f64, P, u32)> = cx.crossings(self.curve).into_iter().filter(|(t, ..)| full || (*t > 1e-6 * prim.end() && *t < prim.end() * (1.0 - 1e-6))).collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        cuts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
        let before = cuts.iter().filter(|c| c.0 < click).next_back().cloned();
        let after = cuts.iter().find(|c| c.0 > click).cloned();
        let g = &mut cx.sketch.geometry;
        let remove_whole = |g: &mut Geometry| g.curves.retain(|c| c.id() != self.curve);
        let mut made = vec![];
        match curve {
            Curve::Circle { id, center, construction, .. } => {
                if cuts.len() < 2 {
                    remove_whole(g);
                } else {
                    // The piece between the crossings either side (round the circle) goes; an arc stays.
                    let lo = before.unwrap_or_else(|| cuts.last().cloned().unwrap());
                    let hi = after.unwrap_or_else(|| cuts.first().cloned().unwrap());
                    let start = cx.end_on(hi.1, hi.2);
                    let end = cx.end_on(lo.1, lo.2);
                    let g = &mut cx.sketch.geometry;
                    let i = g.curves.iter().position(|c| c.id() == id).unwrap();
                    g.curves[i] = Curve::Arc { id, center, start, end, construction };
                    // A circle's radius dimension carries over to the arc; it is the same curve id.
                    made.push(id);
                }
            }
            Curve::Line { id, a, b, construction } => {
                if before.is_none() && after.is_none() {
                    remove_whole(g);
                } else {
                    let pieces: Vec<(u32, u32)> = {
                        let mut v = vec![];
                        if let Some(lo) = &before {
                            let e = cx.end_on(lo.1, lo.2);
                            v.push((a, e));
                        }
                        if let Some(hi) = &after {
                            let s = cx.end_on(hi.1, hi.2);
                            v.push((s, b));
                        }
                        v
                    };
                    let level = cx.sketch.geometry.constraints.iter().find_map(|c| match c {
                        Constraint::Horizontal { line } if *line == id => Some(true),
                        Constraint::Vertical { line } if *line == id => Some(false),
                        _ => None,
                    });
                    let g = &mut cx.sketch.geometry;
                    let i = g.curves.iter().position(|c| c.id() == id).unwrap();
                    g.curves[i] = Curve::Line { id, a: pieces[0].0, b: pieces[0].1, construction };
                    made.push(id);
                    if let Some(&(s, e)) = pieces.get(1) {
                        let nid = cx.id();
                        cx.sketch.geometry.curves.push(Curve::Line { id: nid, a: s, b: e, construction });
                        match level {
                            Some(true) => cx.sketch.geometry.constraints.push(Constraint::Horizontal { line: nid }),
                            Some(false) => cx.sketch.geometry.constraints.push(Constraint::Vertical { line: nid }),
                            None => {}
                        }
                        made.push(nid);
                    }
                }
            }
            Curve::Arc { id, center, start, end, construction } => {
                if before.is_none() && after.is_none() {
                    remove_whole(g);
                } else {
                    let mut pieces = vec![];
                    if let Some(lo) = &before {
                        let e = cx.end_on(lo.1, lo.2);
                        pieces.push((start, e));
                    }
                    if let Some(hi) = &after {
                        let s = cx.end_on(hi.1, hi.2);
                        pieces.push((s, end));
                    }
                    let g = &mut cx.sketch.geometry;
                    let i = g.curves.iter().position(|c| c.id() == id).unwrap();
                    g.curves[i] = Curve::Arc { id, center, start: pieces[0].0, end: pieces[0].1, construction };
                    made.push(id);
                    if let Some(&(s, e)) = pieces.get(1) {
                        let nid = cx.id();
                        cx.sketch.geometry.curves.push(Curve::Arc { id: nid, center, start: s, end: e, construction });
                        made.push(nid);
                    }
                }
            }
            Curve::Bezier { id, a, b, c1, c2, construction } => {
                if before.is_none() && after.is_none() {
                    remove_whole(g);
                } else {
                    let ctrl = [cx.p(a), cx.p(c1), cx.p(c2), cx.p(b)];
                    let mut pieces: Vec<[P; 4]> = vec![];
                    if let Some(lo) = &before {
                        pieces.push(split_bezier(ctrl, lo.0).0);
                    }
                    if let Some(hi) = &after {
                        pieces.push(split_bezier(ctrl, hi.0).1);
                    }
                    let mut first = true;
                    for (k, piece) in pieces.iter().enumerate() {
                        // The kept piece's outer end is the old end; its cut end and handles are new.
                        let left = k == 0 && before.is_some();
                        let (pa, pb) = if left { (a, cx.point(piece[3])) } else { (cx.point(piece[0]), b) };
                        let (h1, h2) = (cx.point(piece[1]), cx.point(piece[2]));
                        let cid = if first { id } else { cx.id() };
                        first = false;
                        let g = &mut cx.sketch.geometry;
                        g.curves.retain(|c| c.id() != cid);
                        g.curves.push(Curve::Bezier { id: cid, a: pa, b: pb, c1: h1, c2: h2, construction });
                        made.push(cid);
                    }
                }
            }
        }
        cx.finish(tx, self.target, self.curve, made)
    }
}

impl Command for ExtendSketch {
    const NAME: &'static str = "sketch.extend";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let curve = cx.sketch.geometry.curves.iter().find(|c| c.id() == self.curve).cloned().ok_or_else(|| bad(format!("no curve {}", self.curve)))?;
        // The end nearer the click, and the curves that end is shared with.
        let (end_id, hit) = match &curve {
            Curve::Line { a, b, .. } => {
                let (pa, pb) = (cx.p(*a), cx.p(*b));
                let (end, from, to) = if dist(self.at, pa) < dist(self.at, pb) { (*a, pb, pa) } else { (*b, pa, pb) };
                // Along the line past this end: the nearest crossing.
                let len = dist(from, to);
                let mut best: Option<(f64, P, u32)> = None;
                for c in cx.sketch.geometry.curves.iter().filter(|c| c.id() != self.curve) {
                    for p in seg_hits(from, to, &cx.prim(c), true) {
                        let t = dot(sub(p, from), sub(to, from)) / (len * len);
                        if t > 1.0 + 1e-6 && best.as_ref().is_none_or(|b| t < b.0) {
                            best = Some((t, p, c.id()));
                        }
                    }
                }
                (end, best.map(|b| (b.1, b.2)))
            }
            Curve::Arc { center, start, end, .. } => {
                let prim = cx.prim(&curve);
                let Prim::Circ(c, r, a0, sweep) = prim else { unreachable!() };
                let at_start = dist(self.at, cx.p(*start)) < dist(self.at, cx.p(*end));
                let _ = center;
                let mut best: Option<(f64, P, u32)> = None;
                for k in cx.sketch.geometry.curves.iter().filter(|k| k.id() != self.curve) {
                    for p in circ_hits(c, r, &cx.prim(k)) {
                        let ang = wrap((p[1] - c[1]).atan2(p[0] - c[0]) - a0);
                        // How far past the chosen end, going away from the arc.
                        let past = if at_start { TAU - ang } else { ang - sweep };
                        if past > 1e-6 && past < TAU - sweep - 1e-9 && best.as_ref().is_none_or(|b| past < b.0) {
                            best = Some((past, p, k.id()));
                        }
                    }
                }
                (if at_start { *start } else { *end }, best.map(|b| (b.1, b.2)))
            }
            _ => return Err(bad("only lines and arcs extend")),
        };
        let Some((p, on)) = hit else { return Err(bad("nothing to extend to: no curve lies ahead of that end")) };
        let shared = cx.sketch.geometry.curves.iter().filter(|c| c.id() != self.curve && curve_points(c).contains(&end_id)).count();
        if shared > 0 {
            return Err(bad("that end is joined to another curve: extend a free end"));
        }
        // The end moves onto the curve it meets (no longer pinned where it was).
        cx.sketch.geometry.constraints.retain(|c| !matches!(c, Constraint::Fixed { point, .. } if *point == end_id));
        let cutter = cx.sketch.geometry.curves.iter().find(|c| c.id() == on).cloned().unwrap();
        let ends: Vec<u32> = match &cutter {
            Curve::Line { a, b, .. } | Curve::Bezier { a, b, .. } => vec![*a, *b],
            Curve::Arc { start, end, .. } => vec![*start, *end],
            Curve::Circle { .. } => vec![],
        };
        let join = ends.into_iter().find(|e| dist(cx.p(*e), p) < 1e-6);
        let g = &mut cx.sketch.geometry;
        match join {
            // Meets the other curve's end: share that point.
            Some(e) => {
                for c in g.curves.iter_mut().filter(|c| c.id() == self.curve) {
                    match c {
                        Curve::Line { a, b, .. } => *(if *a == end_id { a } else { b }) = e,
                        Curve::Arc { start, end, .. } => *(if *start == end_id { start } else { end }) = e,
                        _ => {}
                    }
                }
            }
            // Meets it part way: our end moves there and is held on it.
            None => {
                if let Some(q) = g.points.iter_mut().find(|q| q.id == end_id) {
                    (q.x, q.y) = (p[0], p[1]);
                }
                if !matches!(cutter, Curve::Bezier { .. }) {
                    g.constraints.push(Constraint::OnCurve { point: end_id, curve: on });
                }
            }
        }
        cx.finish(tx, self.target, self.curve, vec![self.curve])
    }
}
