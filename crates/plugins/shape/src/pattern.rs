//! Patterns and mirrors in a sketch: real copies of the chosen curves, in a
//! row, round a centre, or reflected about a line.
//!
//! Copies stay linked by default. Every copied point is held to the one
//! before it by a constraint (a step along the pattern's direction line, a
//! turn round its centre, or a reflection), so editing the original moves
//! the copies, and the spacing or angle is an ordinary dimension: typed,
//! scrubbed or driven by a formula. A linear pattern's direction is a
//! construction line from the original to its first copy (its length is the
//! spacing); a circular pattern's step is the angle between two construction
//! lines from the centre. The count is fixed when the pattern is made.

use crate::pathfinder::curve_points;
use crate::sketch::{Constraint, Curve};
use crate::trim::{Ctx, P, bad, cross, dist, dot, sub};
use de_core::schemars::{self, JsonSchema};
use de_core::{Command, CommandError, StableId, Tx, Value, json};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// In a row: `count` copies `spacing` mm apart along `direction`.
    Linear,
    /// Round `center`: `count` copies `angle` degrees apart.
    Circular,
}

/// A point given by its id, or by where it is (sketch mm).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PointRef {
    Id(u32),
    At([f64; 2]),
}

fn yes() -> bool {
    true
}

/// Copies curves in a row or round a centre.
#[derive(Deserialize, JsonSchema)]
pub struct PatternSketch {
    pub target: StableId,
    pub curves: Vec<u32>,
    pub kind: PatternKind,
    /// How many in all, the original included (2 or more).
    pub count: u32,
    /// Linear: mm from one copy to the next.
    pub spacing: Option<f64>,
    /// Linear: which way the row runs (default along x).
    pub direction: Option<[f64; 2]>,
    /// Circular: degrees from one copy to the next, anticlockwise (default 360 / count).
    pub angle: Option<f64>,
    /// Circular: the centre, a point or where it is (default the sketch origin).
    pub center: Option<PointRef>,
    /// Keep the copies following the original (default true).
    #[serde(default = "yes")]
    pub linked: bool,
    /// Linear: rows of the row (a rectangular array), the first included.
    pub rows: Option<u32>,
    /// mm from one row to the next.
    pub row_spacing: Option<f64>,
    /// Which way the rows step (default square to `direction`, anticlockwise).
    pub row_direction: Option<[f64; 2]>,
}

/// Copies curves reflected about a line of the sketch.
#[derive(Deserialize, JsonSchema)]
pub struct MirrorSketch {
    pub target: StableId,
    pub curves: Vec<u32>,
    /// The mirror line (a line of the sketch, often a construction line).
    pub line: u32,
    #[serde(default = "yes")]
    pub linked: bool,
}

/// The curves to copy, checked, and their points in order of first use.
fn chosen(cx: &Ctx, ids: &[u32], skip: Option<u32>) -> Result<(Vec<Curve>, Vec<u32>), CommandError> {
    let mut curves = vec![];
    let want: BTreeSet<u32> = ids.iter().copied().filter(|i| Some(*i) != skip).collect();
    for id in &want {
        curves.push(cx.sketch.geometry.curves.iter().find(|c| c.id() == *id).cloned().ok_or_else(|| bad(format!("no curve {id}")))?);
    }
    if curves.is_empty() {
        return Err(bad("select the curves to copy"));
    }
    let mut points = vec![];
    for c in &curves {
        for p in curve_points(c) {
            if !points.contains(&p) {
                points.push(p);
            }
        }
    }
    Ok((curves, points))
}

/// One copy of the curves, its points mapped by `map` (new curve ids; arcs reversed when `flip`).
fn copy_curves(cx: &mut Ctx, curves: &[Curve], map: &BTreeMap<u32, u32>, flip: bool) -> Vec<(u32, u32)> {
    let mut made = vec![];
    for c in curves {
        let id = cx.id();
        let mut copy = c.mapped(id, &|p| map[&p]);
        if flip {
            copy = copy.flipped();
        }
        if let Some(r) = cx.radii.get(&c.id()) {
            copy.set_size(*r);
        }
        cx.sketch.geometry.curves.push(copy);
        made.push((c.id(), id));
    }
    made
}

impl Command for PatternSketch {
    const NAME: &'static str = "sketch.pattern";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if let Some(rows) = self.rows.filter(|r| *r > 1) {
            // A rectangular array: the row, then rows of it.
            if self.kind != PatternKind::Linear {
                return Err(bad("rows are for a linear pattern"));
            }
            let d = self.direction.unwrap_or([1.0, 0.0]);
            let across = self.row_direction.unwrap_or([-d[1], d[0]]);
            let spacing = self.row_spacing.ok_or_else(|| bad("rows need a row spacing"))?;
            let mut all = self.curves.clone();
            let row = PatternSketch { rows: None, row_spacing: None, row_direction: None, ..self };
            let (target, linked) = (row.target, row.linked);
            let first = if row.count > 1 { row.apply(tx)? } else { json!({ "curves": [] }) };
            all.extend(first["curves"].as_array().into_iter().flatten().filter_map(|v| v.as_u64()).map(|v| v as u32));
            let next = PatternSketch { target, curves: all, kind: PatternKind::Linear, count: rows, spacing: Some(spacing), direction: Some(across), angle: None, center: None, linked, rows: None, row_spacing: None, row_direction: None };
            let second = next.apply(tx)?;
            let mut made: Vec<Value> = first["curves"].as_array().cloned().unwrap_or_default();
            made.extend(second["curves"].as_array().cloned().unwrap_or_default());
            return Ok(json!({ "id": target, "curves": made, "key": first.get("key").cloned().unwrap_or(Value::Null), "row_key": second["key"] }));
        }
        if self.count < 2 {
            return Err(bad("a pattern needs a count of 2 or more (the original included)"));
        }
        if self.count > 200 {
            return Err(bad("at most 200 in a pattern"));
        }
        let mut cx = Ctx::load(tx, self.target)?;
        let (curves, points) = chosen(&cx, &self.curves, None)?;
        // Where copy k puts a point, and (circular) the centre.
        let (center, step): (Option<(u32, P)>, Box<dyn Fn(P, u32) -> P>) = match self.kind {
            PatternKind::Linear => {
                let spacing = self.spacing.ok_or_else(|| bad("a linear pattern needs a spacing"))?;
                if !(spacing.is_finite() && spacing.abs() > 1e-9) {
                    return Err(bad("a linear pattern needs a spacing other than zero"));
                }
                let d = self.direction.unwrap_or([1.0, 0.0]);
                let l = d[0].hypot(d[1]);
                if !(l > 1e-12) {
                    return Err(bad("the direction needs a length"));
                }
                let v = [d[0] / l * spacing, d[1] / l * spacing];
                (None, Box::new(move |p: P, k: u32| [p[0] + v[0] * k as f64, p[1] + v[1] * k as f64]))
            }
            PatternKind::Circular => {
                let angle = self.angle.unwrap_or(360.0 / self.count as f64);
                if !(angle.is_finite() && angle.abs() > 1e-9) {
                    return Err(bad("a circular pattern needs an angle other than zero"));
                }
                let (cid, c) = match &self.center {
                    Some(PointRef::Id(id)) => (Some(*id), *cx.at.get(id).ok_or_else(|| bad(format!("no point {id}")))?),
                    Some(PointRef::At(at)) => (None, *at),
                    None => (None, [0.0, 0.0]),
                };
                // A centre on an existing point is that point (one of the copied curves' own first).
                let on = |id: &u32| cx.at.get(id).is_some_and(|q| dist(*q, c) < 1e-6);
                let cid = match cid.or_else(|| points.iter().copied().find(|id| on(id))).or_else(|| cx.at.keys().copied().find(|id| on(id))) {
                    Some(id) => id,
                    None => {
                        let id = cx.point(c);
                        if self.linked {
                            cx.sketch.geometry.constraints.push(Constraint::Fixed { point: id, x: c[0], y: c[1] });
                        }
                        id
                    }
                };
                let a = angle.to_radians();
                (Some((cid, c)), Box::new(move |p: P, k: u32| {
                    let (s, co) = (a * k as f64).sin_cos();
                    let d = sub(p, c);
                    [c[0] + d[0] * co - d[1] * s, c[1] + d[0] * s + d[1] * co]
                }))
            }
        };
        // Copies, each point held to its predecessor.
        let fixed_point = center.map(|(id, _)| id);
        let mut made = vec![];
        let mut copies: Vec<BTreeMap<u32, u32>> = vec![];
        for k in 1..self.count {
            let mut map = BTreeMap::new();
            for p in &points {
                if Some(*p) == fixed_point {
                    map.insert(*p, *p);
                    continue;
                }
                let q = step(cx.p(*p), k);
                map.insert(*p, cx.point(q));
            }
            let pairs = copy_curves(&mut cx, &curves, &map, false);
            if self.linked {
                for (old, new) in &pairs {
                    if curves.iter().find(|c| c.id() == *old).is_some_and(|c| c.size().is_some()) {
                        cx.sketch.geometry.constraints.push(Constraint::Equal { a: *old, b: *new });
                    }
                }
            }
            made.extend(pairs.iter().map(|p| p.1));
            copies.push(map);
        }
        let mut key = None;
        if self.linked {
            // The original's first point not at the centre carries the step's construction.
            // (Away from the centre, so the step's construction has a direction.)
            let centre_at = center.map(|(_, c)| c);
            let base = *points.iter().find(|p| Some(**p) != fixed_point && centre_at.is_none_or(|c| dist(cx.p(**p), c) > 1e-6)).ok_or_else(|| bad("nothing to pattern but the centre"))?;
            let first = copies[0][&base];
            let mut cons = vec![];
            let steps: Vec<Constraint>;
            match (self.kind, center) {
                (PatternKind::Circular, Some((cid, _))) => {
                    let (l0, l1) = (cx.id(), cx.id());
                    cx.sketch.geometry.curves.push(Curve::Line { id: l0, a: cid, b: base, construction: true });
                    cx.sketch.geometry.curves.push(Curve::Line { id: l1, a: cid, b: first, construction: true });
                    let k = cx.dimension("Pattern angle", self.angle.unwrap_or(360.0 / self.count as f64), "deg");
                    cons.push(Constraint::Angle { a: l0, b: l1, key: k.clone() });
                    cons.push(Constraint::Equal { a: l0, b: l1 });
                    key = Some(k);
                    steps = link_steps(&points, &copies, base, fixed_point, |a, b| Constraint::Rotate { a, b, from: l0, to: l1 });
                }
                _ => {
                    let l = cx.id();
                    cx.sketch.geometry.curves.push(Curve::Line { id: l, a: base, b: first, construction: true });
                    let v = sub(cx.p(first), cx.p(base));
                    if v[1].abs() < 1e-9 * v[0].abs() {
                        cons.push(Constraint::Horizontal { line: l });
                    } else if v[0].abs() < 1e-9 * v[1].abs() {
                        cons.push(Constraint::Vertical { line: l });
                    }
                    let k = cx.dimension("Pattern spacing", self.spacing.unwrap_or(0.0).abs(), "mm");
                    cons.push(Constraint::Length { line: l, key: k.clone() });
                    key = Some(k);
                    steps = link_steps(&points, &copies, base, fixed_point, |a, b| Constraint::Translate { a, b, along: l });
                }
            }
            cx.sketch.geometry.constraints.extend(cons);
            cx.sketch.geometry.constraints.extend(steps);
        }
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), true, json!({ "id": target, "curves": made, "key": key }))
    }
}

/// Each copied point held to the same point one copy before (the first copy
/// of `base` is held by the step's construction instead).
fn link_steps(points: &[u32], copies: &[BTreeMap<u32, u32>], base: u32, fixed: Option<u32>, make: impl Fn(u32, u32) -> Constraint) -> Vec<Constraint> {
    let mut out = vec![];
    for (k, map) in copies.iter().enumerate() {
        for p in points {
            if Some(*p) == fixed || (k == 0 && *p == base) {
                continue;
            }
            let before = if k == 0 { *p } else { copies[k - 1][p] };
            out.push(make(before, map[p]));
        }
    }
    out
}

impl Command for MirrorSketch {
    const NAME: &'static str = "sketch.mirror";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut cx = Ctx::load(tx, self.target)?;
        let Some(Curve::Line { a, b, .. }) = cx.sketch.geometry.curves.iter().find(|c| c.id() == self.line).cloned() else {
            return Err(bad(format!("the mirror must be a line of the sketch; {} is not", self.line)));
        };
        let (curves, points) = chosen(&cx, &self.curves, Some(self.line))?;
        let (s, e) = (cx.p(a), cx.p(b));
        let len = dist(s, e);
        if len < 1e-9 {
            return Err(bad("the mirror line has no length"));
        }
        let d = [(e[0] - s[0]) / len, (e[1] - s[1]) / len];
        let mut map = BTreeMap::new();
        let mut links = vec![];
        for p in &points {
            let q = cx.p(*p);
            let off = sub(q, s);
            let along = dot(off, d);
            let side = cross(d, off);
            if side.abs() < 1e-6 {
                // On the mirror: its own reflection, held there.
                map.insert(*p, *p);
                if self.linked && *p != a && *p != b {
                    links.push(Constraint::OnCurve { point: *p, curve: self.line });
                }
                continue;
            }
            let foot = [s[0] + d[0] * along, s[1] + d[1] * along];
            let r = [2.0 * foot[0] - q[0], 2.0 * foot[1] - q[1]];
            let id = cx.point(r);
            map.insert(*p, id);
            links.push(Constraint::Symmetric { a: *p, b: id, line: self.line });
        }
        let pairs = copy_curves(&mut cx, &curves, &map, true);
        if self.linked {
            for (old, new) in &pairs {
                if curves.iter().find(|c| c.id() == *old).is_some_and(|c| c.size().is_some()) {
                    links.push(Constraint::Equal { a: *old, b: *new });
                }
            }
            cx.sketch.geometry.constraints.extend(links);
        }
        let made: Vec<u32> = pairs.iter().map(|p| p.1).collect();
        let target = self.target;
        cx.save(tx, target, &BTreeSet::new(), true, json!({ "id": target, "curves": made }))
    }
}
