//! Pathfinder and offset for sketches, as in a drawing program: unite,
//! subtract or intersect closed outlines, or add an outline a set distance
//! outside (or inside) one. The work is exact: outlines become faces in
//! OpenCascade, the boolean or offset runs on true lines, arcs and Béziers,
//! and the result comes back as sketch curves (lines, arcs, circles and
//! cubic Béziers) the solver and the STEP export use like any other.

use crate::params;
use crate::sketch::{Constraint, Curve, Geometry, Loop, Point, Segment, Sketch, area, loops_of_curves, polygon, profile_data, reverse, solve};
use de_core::schemars::{self, JsonSchema};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Pathfinder {
    /// One outline covering all of them.
    Unite,
    /// The largest outline with the others cut out of it.
    Subtract,
    /// Only where they all overlap.
    Intersect,
}

/// Unite, subtract or intersect the closed outlines the given curves belong to;
/// the result replaces them.
#[derive(Deserialize, JsonSchema)]
pub struct CombineSketch {
    pub target: StableId,
    pub op: Pathfinder,
    /// Any curves of the outlines to combine (an outline is taken whole).
    pub curves: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Corners {
    /// Corners pushed outwards become arcs.
    #[default]
    Round,
    /// Corners stay sharp (the sides extended to meet).
    Sharp,
}

/// Adds an outline `distance` mm outside the selected ones (negative: inside).
#[derive(Deserialize, JsonSchema)]
pub struct OffsetSketch {
    pub target: StableId,
    pub curves: Vec<u32>,
    pub distance: f64,
    #[serde(default)]
    pub corners: Corners,
}

fn bad(text: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, text.into())
}

/// The sketch, its solved loops, and the loops (oriented as solved) the curves belong to.
fn chosen(tx: &Tx, target: StableId, curves: &[u32]) -> Result<(Sketch, std::collections::BTreeMap<String, f64>, Vec<(Loop, Vec<u32>)>), CommandError> {
    let s = tx.get::<Sketch>(target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a sketch")))?;
    let dims = params::resolve(tx.world(), &s.params).map_err(bad)?;
    let solved = solve(&s.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
    let want: BTreeSet<u32> = curves.iter().copied().collect();
    let (loops, _) = loops_of_curves(&s.geometry, &solved);
    let picked: Vec<(Loop, Vec<u32>)> = loops.into_iter().filter(|(_, ids)| ids.iter().any(|i| want.contains(i))).collect();
    if picked.is_empty() {
        return Err(bad("select closed outlines: nothing selected is part of one"));
    }
    Ok((s, dims, picked))
}

/// A loop turned counter-clockwise (an outline on its own).
fn outward(l: &Loop) -> Loop {
    if area(&polygon(l, 0.01)) < 0.0 { reverse(l) } else { l.clone() }
}

/// Loops read back from a profile.
fn parse(n: usize, d: &[f64]) -> Vec<Loop> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    for _ in 0..n {
        let count = d[i] as usize;
        i += 1;
        let mut l = Vec::with_capacity(count);
        for _ in 0..count {
            let p = |k: usize| [d[i + k], d[i + k + 1]];
            match d[i] as i32 {
                0 => {
                    l.push(Segment::Line { from: p(1), to: p(3) });
                    i += 5;
                }
                1 => {
                    l.push(Segment::Arc { from: p(1), to: p(3), center: p(5), ccw: d[i + 7] != 0.0 });
                    i += 8;
                }
                2 => {
                    l.push(Segment::Circle { center: p(1), radius: d[i + 3] });
                    i += 4;
                }
                _ => {
                    l.push(Segment::Bezier { from: p(1), c1: p(3), c2: p(5), to: p(7) });
                    i += 9;
                }
            }
        }
        out.push(l);
    }
    out
}

/// The numbers a constraint refers to (points and curves).
pub(crate) fn refs(c: &Constraint) -> Vec<u32> {
    match serde_json::to_value(c) {
        Ok(Value::Object(m)) => m.iter().filter(|(k, _)| !matches!(k.as_str(), "kind" | "key" | "x" | "y")).filter_map(|(_, v)| v.as_u64().map(|n| n as u32)).collect(),
        _ => vec![],
    }
}

/// The geometry without these curves, the points only they used, and the
/// constraints on any of them (and the dimensions those carried).
fn without(s: &mut Sketch, gone: &BTreeSet<u32>) -> Geometry {
    let g = &s.geometry;
    let curves: Vec<Curve> = g.curves.iter().filter(|c| !gone.contains(&c.id())).cloned().collect();
    let used: BTreeSet<u32> = curves.iter().flat_map(curve_points).collect();
    let removed_points: BTreeSet<u32> = g.curves.iter().filter(|c| gone.contains(&c.id())).flat_map(curve_points).filter(|p| !used.contains(p)).collect();
    let points: Vec<Point> = g.points.iter().filter(|p| !removed_points.contains(&p.id)).cloned().collect();
    let dead = |id: &u32| gone.contains(id) || removed_points.contains(id);
    let constraints: Vec<Constraint> = g.constraints.iter().filter(|c| !refs(c).iter().any(dead)).cloned().collect();
    let keys: BTreeSet<String> = constraints.iter().filter_map(|c| serde_json::to_value(c).ok()?.get("key")?.as_str().map(str::to_string)).collect();
    // Dimensions only removed constraints carried go with them.
    s.params.retain(|p| keys.contains(&p.key));
    Geometry { points, curves, constraints }
}

pub(crate) fn curve_points(c: &Curve) -> Vec<u32> {
    match c {
        Curve::Line { a, b, .. } => vec![*a, *b],
        Curve::Arc { center, start, end, .. } => vec![*center, *start, *end],
        Curve::Circle { center, .. } => vec![*center],
        Curve::Bezier { a, b, c1, c2, .. } => vec![*a, *b, *c1, *c2],
    }
}

/// Adds loops as new curves (sharing points where loops meet); level and
/// upright lines get their constraint. Returns the new curves' ids.
fn add_loops(g: &mut Geometry, loops: &[Loop]) -> Vec<u32> {
    let mut next = g.points.iter().map(|p| p.id).chain(g.curves.iter().map(Curve::id)).max().unwrap_or(0) + 1;
    let mut fresh = move || {
        next += 1;
        next - 1
    };
    let first_new = g.points.len();
    // An end where loops meet is one point; centres and handles are their own.
    let point = |g: &mut Geometry, id: u32, at: [f64; 2], share: bool| -> u32 {
        if share && let Some(p) = g.points[first_new..].iter().find(|p| (p.x - at[0]).hypot(p.y - at[1]) < 1e-6) {
            return p.id;
        }
        g.points.push(Point { id, x: at[0], y: at[1] });
        id
    };
    let mut made = vec![];
    for seg in loops.iter().flatten() {
        let curve = match *seg {
            Segment::Line { from, to } => {
                let (a, b) = (point(g, fresh(), from, true), point(g, fresh(), to, true));
                let id = fresh();
                let len = (to[0] - from[0]).hypot(to[1] - from[1]).max(1.0);
                if (to[1] - from[1]).abs() < 1e-9 * len {
                    g.constraints.push(Constraint::Horizontal { line: id });
                } else if (to[0] - from[0]).abs() < 1e-9 * len {
                    g.constraints.push(Constraint::Vertical { line: id });
                }
                Curve::Line { id, a, b, construction: false }
            }
            Segment::Arc { from, to, center, ccw } => {
                let (from, to) = if ccw { (from, to) } else { (to, from) };
                let (start, end) = (point(g, fresh(), from, true), point(g, fresh(), to, true));
                let center = point(g, fresh(), center, false);
                Curve::Arc { id: fresh(), center, start, end, construction: false }
            }
            Segment::Circle { center, radius } => {
                let center = point(g, fresh(), center, false);
                Curve::Circle { id: fresh(), center, radius: radius.abs(), construction: false }
            }
            Segment::Bezier { from, to, c1, c2 } => {
                let (a, b) = (point(g, fresh(), from, true), point(g, fresh(), to, true));
                let (c1, c2) = (point(g, fresh(), c1, false), point(g, fresh(), c2, false));
                Curve::Bezier { id: fresh(), a, b, c1, c2, construction: false }
            }
        };
        made.push(curve.id());
        g.curves.push(curve);
    }
    made
}

fn finish(tx: &mut Tx, target: StableId, mut s: Sketch, g: Geometry, dims: &std::collections::BTreeMap<String, f64>, made: Vec<u32>) -> Result<Value, CommandError> {
    solve(&g, dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
    s.geometry = g;
    tx.set(target, s)?;
    Ok(json!({ "id": target, "curves": made }))
}

impl Command for CombineSketch {
    const NAME: &'static str = "sketch.combine";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (mut s, dims, picked) = chosen(tx, self.target, &self.curves)?;
        if picked.len() < 2 {
            return Err(bad("select two or more closed outlines to combine"));
        }
        let mut arena = de_occt::Arena::new();
        let mut faces = vec![];
        for (l, _) in &picked {
            let l = outward(l);
            faces.push((area(&polygon(&l, 0.01)), arena.sketch_profile(1, &profile_data(std::slice::from_ref(&l))).map_err(bad)?));
        }
        let shape = match self.op {
            Pathfinder::Unite => arena.union_all(&faces.iter().map(|f| f.1).collect::<Vec<_>>()).map_err(bad)?,
            Pathfinder::Intersect => {
                let mut s = faces[0].1;
                for f in &faces[1..] {
                    s = arena.boolean(de_occt::Bool::Intersection, s, f.1).map_err(bad)?;
                }
                s
            }
            Pathfinder::Subtract => {
                faces.sort_by(|a, b| b.0.total_cmp(&a.0));
                let mut s = faces[0].1;
                for f in &faces[1..] {
                    s = arena.boolean(de_occt::Bool::Difference, s, f.1).map_err(bad)?;
                }
                s
            }
        };
        let (n, data) = arena.profile_loops(shape).map_err(bad)?;
        if n == 0 {
            return Err(bad(match self.op {
                Pathfinder::Intersect => "the outlines do not overlap: nothing is left",
                _ => "nothing is left",
            }));
        }
        let gone: BTreeSet<u32> = picked.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
        let mut g = without(&mut s, &gone);
        let made = add_loops(&mut g, &parse(n, &data));
        finish(tx, self.target, s, g, &dims, made)
    }
}

impl Command for OffsetSketch {
    const NAME: &'static str = "sketch.offset";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if !self.distance.is_finite() || self.distance.abs() < 1e-6 {
            return Err(bad("an offset needs a distance"));
        }
        let (s, dims, picked) = chosen(tx, self.target, &self.curves)?;
        // The picked loops as one profile, holes kept: an outline and its
        // hole offset together, the hole moving the other way.
        let loops: Vec<Loop> = picked.iter().map(|(l, _)| l.clone()).collect();
        let mut arena = de_occt::Arena::new();
        let profile = arena.sketch_profile(loops.len(), &profile_data(&loops)).map_err(bad)?;
        let join = match self.corners {
            Corners::Round => de_occt::Join::Round,
            Corners::Sharp => de_occt::Join::Miter,
        };
        let shape = arena.offset(profile, self.distance, join).map_err(|e| bad(format!("{e}: try a smaller distance")))?;
        let (n, data) = arena.profile_loops(shape).map_err(bad)?;
        if n == 0 {
            return Err(bad("the offset leaves nothing: try a smaller distance"));
        }
        let mut g = s.geometry.clone();
        let made = add_loops(&mut g, &parse(n, &data));
        finish(tx, self.target, s, g, &dims, made)
    }
}
