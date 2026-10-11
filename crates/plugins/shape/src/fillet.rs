//! Rounding and bevelling sketch corners, as in a drafting program: a fillet
//! replaces the corner where two curves meet with an arc tangent to both, a
//! chamfer with a straight cut; both curves are trimmed back (or carried on)
//! to meet it. Lines, arcs and circles round in any pairing.
//!
//! The result stays a constrained sketch: the round is held tangent to both
//! curves and its radius is a dimension (a sketch parameter, so it can be
//! typed, scrubbed or linked to a formula); a chamfer's setback is one too.
//! Where the old corner point carried anything (a dimension, a pin, a join),
//! it stays as a virtual sharp: a point held on both curves (extended), so
//! what was measured to the corner still is.

use crate::sketch::{Constraint, Curve};
use crate::trim::{Ctx, P, Prim, bad, circ_hits, cross, dist, dot, seg_hits, sub, wrap};
use de_core::schemars::{self, JsonSchema};
use de_core::{Command, CommandError, StableId, Tx, Value, json};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::f64::consts::TAU;

/// Rounds a corner: the corner point two curves share, or two curves (which
/// need not touch: they are trimmed or carried on to the round).
#[derive(Deserialize, JsonSchema)]
pub struct FilletSketch {
    pub target: StableId,
    /// The corner: a point exactly two lines or arcs end at.
    pub point: Option<u32>,
    /// Or the two curves to round between.
    pub curves: Option<Vec<u32>>,
    /// For two curves that cross: a point on the piece of each to keep (sketch mm), in the order of `curves`.
    pub picks: Option<Vec<[f64; 2]>>,
    pub radius: f64,
}

/// Bevels a corner: each curve is cut back `distance` mm from the corner and a line joins the cuts.
#[derive(Deserialize, JsonSchema)]
pub struct ChamferSketch {
    pub target: StableId,
    pub point: Option<u32>,
    pub curves: Option<Vec<u32>>,
    pub picks: Option<Vec<[f64; 2]>>,
    pub distance: f64,
}

/// One of the two curves at a corner.
struct Side {
    curve: Curve,
    prim: Prim,
    /// The end that gives way to the round (none for a whole circle).
    cut: Option<u32>,
    /// The end that stays.
    keep: Option<u32>,
    /// A whole circle: a point on it, picking which way round from the corner the round goes.
    pick: Option<P>,
}

fn ends(c: &Curve) -> Option<(u32, u32)> {
    match c {
        Curve::Line { a, b, .. } => Some((*a, *b)),
        Curve::Arc { start, end, .. } => Some((*start, *end)),
        _ => None,
    }
}

fn angle(c: P, p: P) -> f64 {
    (p[1] - c[1]).atan2(p[0] - c[0])
}

fn unit(v: P) -> P {
    let l = v[0].hypot(v[1]).max(1e-300);
    [v[0] / l, v[1] / l]
}

/// A curve's whole line or circle (an arc's circle), for meeting points past its ends.
fn whole(p: &Prim) -> Prim {
    match p {
        Prim::Circ(c, r, ..) => Prim::Circ(*c, *r, 0.0, TAU),
        other => other.clone(),
    }
}

/// Where two curves' whole lines or circles meet.
fn meets(a: &Prim, b: &Prim) -> Vec<P> {
    match (whole(a), whole(b)) {
        (Prim::Seg(p, q), o) => seg_hits(p, q, &o, true),
        (o, Prim::Seg(p, q)) => seg_hits(p, q, &o, true),
        (Prim::Circ(c, r, ..), o) => circ_hits(c, r, &o),
        _ => vec![],
    }
}

/// How far along the kept piece of `s` (from the corner `j`) the point `t`
/// lies, if it lies on it: millimetres along a line, arc length on an arc.
fn along(cx: &Ctx, s: &Side, j: P, t: P) -> Option<f64> {
    match (&s.prim, s.keep) {
        (Prim::Seg(..), Some(k)) => {
            let k = cx.p(k);
            let d = sub(k, j);
            let l2 = dot(d, d);
            let u = dot(sub(t, j), d) / l2.max(1e-300);
            (u > 1e-9 && u <= 1.0 + 1e-9).then(|| u * l2.sqrt())
        }
        (Prim::Circ(c, r, ..), Some(k)) => {
            let (aj, ak, at) = (angle(*c, j), angle(*c, cx.p(k)), angle(*c, t));
            // The kept piece runs from the corner towards the kept end through the arc.
            let ccw = matches!(s.curve, Curve::Arc { end, .. } if end == k);
            let (span, here) = if ccw { (wrap(ak - aj), wrap(at - aj)) } else { (wrap(aj - ak), wrap(aj - at)) };
            let span = if span < 1e-12 { TAU } else { span };
            (here > 1e-9 && here <= span + 1e-9).then_some(here * r)
        }
        // A whole circle stays whole: anywhere on it will do, on the picked side of the corner if one is picked.
        (Prim::Circ(c, r, ..), None) => {
            let d = wrap(angle(*c, t) - angle(*c, j));
            match s.pick {
                Some(q) => {
                    let ccw = wrap(angle(*c, q) - angle(*c, j)) < std::f64::consts::PI;
                    let (here, ok) = if ccw { (d, d < std::f64::consts::PI) } else { (TAU - d, d > std::f64::consts::PI) };
                    (ok && here > 1e-9).then_some(here * r)
                }
                None => Some(d.min(TAU - d) * r),
            }
        }
        _ => None,
    }
}

/// The corner: where it is, the point there (if the curves share one), and the two sides.
fn corner(cx: &Ctx, point: Option<u32>, curves: Option<&[u32]>, picks: Option<&[[f64; 2]]>) -> Result<(P, Option<u32>, [Side; 2]), CommandError> {
    let g = &cx.sketch.geometry;
    let find = |id: u32| g.curves.iter().find(|c| c.id() == id).cloned().ok_or_else(|| bad(format!("no curve {id}")));
    let check = |c: &Curve| match c {
        Curve::Bezier { .. } => Err(bad("corners round between lines, arcs and circles, not Béziers")),
        _ => Ok(()),
    };
    let shared_side = |c: Curve, j: u32| -> Side {
        let (a, b) = ends(&c).unwrap();
        let (cut, keep) = if a == j { (a, b) } else { (b, a) };
        Side { prim: cx.prim(&c), curve: c, cut: Some(cut), keep: Some(keep), pick: None }
    };
    // A corner point, or two curves that share one.
    let shared = match (point, curves) {
        (Some(j), _) => {
            let at: Vec<Curve> = g.curves.iter().filter(|c| ends(c).is_some_and(|(a, b)| a == j || b == j) || matches!(c, Curve::Bezier { a, b, .. } if *a == j || *b == j)).cloned().collect();
            if !g.points.iter().any(|p| p.id == j) {
                return Err(bad(format!("no point {j}")));
            }
            if at.len() != 2 {
                return Err(bad(format!("a corner is where exactly two curves meet; {} end at that point", at.len())));
            }
            Some((j, at[0].clone(), at[1].clone()))
        }
        (None, Some([a, b])) => {
            let (ca, cb) = (find(*a)?, find(*b)?);
            match (ends(&ca), ends(&cb)) {
                (Some((a0, a1)), Some((b0, b1))) => [a0, a1].into_iter().find(|p| *p == b0 || *p == b1).map(|j| (j, ca, cb)),
                _ => None,
            }
        }
        (None, Some(_)) => return Err(bad("give two curves")),
        (None, None) => return Err(bad("give the corner point, or two curves")),
    };
    if let Some((j, a, b)) = shared {
        check(&a)?;
        check(&b)?;
        if a.id() == b.id() {
            return Err(bad("pick two different curves"));
        }
        return Ok((cx.p(j), Some(j), [shared_side(a, j), shared_side(b, j)]));
    }
    // Two curves apart (or crossing): the corner is where their lines or circles meet.
    let ids = curves.unwrap();
    let (ca, cb) = (find(ids[0])?, find(ids[1])?);
    check(&ca)?;
    check(&cb)?;
    if ca.id() == cb.id() {
        return Err(bad("pick two different curves"));
    }
    let (pa, pb) = (cx.prim(&ca), cx.prim(&cb));
    let near = |c: &Curve, q: P| -> f64 {
        match ends(c) {
            Some((a, b)) => dist(cx.p(a), q).min(dist(cx.p(b), q)),
            None => 0.0,
        }
    };
    let score = |q: P| match picks {
        Some([u, v]) => dist(q, *u) + dist(q, *v),
        _ => near(&ca, q) + near(&cb, q),
    };
    let j = meets(&pa, &pb).into_iter().min_by(|x, y| score(*x).total_cmp(&score(*y))).ok_or_else(|| bad("those curves never meet, so they make no corner"))?;
    let side = |c: Curve, prim: Prim, pick: Option<P>| -> Side {
        let Some((a, b)) = ends(&c) else { return Side { curve: c, prim, cut: None, keep: None, pick } };
        let (pa, pb) = (cx.p(a), cx.p(b));
        // Which end stays: the one on the picked side of the corner, else the farther one.
        let keep_a = match (pick, &prim) {
            (Some(q), Prim::Seg(..)) => dot(sub(pa, j), sub(q, j)) > dot(sub(pb, j), sub(q, j)),
            (Some(q), Prim::Circ(cc, ..)) => {
                // Along the arc from the corner: is the pick on the way to the start (clockwise)?
                let (aj, aq, astart) = (angle(*cc, j), angle(*cc, q), angle(*cc, pa));
                wrap(aj - aq) <= wrap(aj - astart) + 1e-12
            }
            _ => dist(pa, j) > dist(pb, j),
        };
        let (keep, cut) = if keep_a { (a, b) } else { (b, a) };
        Side { curve: c, prim, cut: Some(cut), keep: Some(keep), pick: None }
    };
    let pick = |i: usize| picks.and_then(|p| p.get(i).copied());
    Ok((j, None, [side(ca, pa, pick(0)), side(cb, pb, pick(1))]))
}

/// A curve's lines or circles offset by `r` either side.
enum Off {
    Line(P, P),
    Circ(P, f64),
}

fn offsets(s: &Side, r: f64) -> Vec<Off> {
    match &s.prim {
        Prim::Seg(a, b) => {
            let d = unit(sub(*b, *a));
            let n = [-d[1], d[0]];
            [1.0, -1.0].iter().map(|k| Off::Line([a[0] + k * r * n[0], a[1] + k * r * n[1]], d)).collect()
        }
        Prim::Circ(c, rr, ..) => {
            let mut v = vec![Off::Circ(*c, rr + r)];
            if rr - r > 1e-9 {
                v.push(Off::Circ(*c, rr - r));
            }
            v
        }
        Prim::Poly(_) => vec![],
    }
}

fn off_meets(a: &Off, b: &Off) -> Vec<P> {
    let full = |c: P, r: f64| Prim::Circ(c, r, 0.0, TAU);
    match (a, b) {
        (Off::Line(p, d), Off::Line(q, e)) => {
            let den = cross(*d, *e);
            if den.abs() < 1e-12 {
                return vec![];
            }
            let t = cross(sub(*q, *p), *e) / den;
            vec![[p[0] + d[0] * t, p[1] + d[1] * t]]
        }
        (Off::Line(p, d), Off::Circ(c, r)) | (Off::Circ(c, r), Off::Line(p, d)) => seg_hits(*p, [p[0] + d[0], p[1] + d[1]], &full(*c, *r), true),
        (Off::Circ(c, r), Off::Circ(k, s)) => circ_hits(*c, *r, &full(*k, *s)),
    }
}

/// Where a circle of radius `r` centred at `o` touches the curve.
fn touch(s: &Side, o: P) -> P {
    match &s.prim {
        Prim::Seg(a, b) => {
            let d = unit(sub(*b, *a));
            let t = dot(sub(o, *a), d);
            [a[0] + d[0] * t, a[1] + d[1] * t]
        }
        Prim::Circ(c, rr, ..) => {
            let d = unit(sub(o, *c));
            [c[0] + d[0] * rr, c[1] + d[1] * rr]
        }
        Prim::Poly(_) => o,
    }
}

/// Swaps a curve's cut end for a new point; a whole circle instead holds the point on it.
fn reattach(cx: &mut Ctx, s: &Side, to: u32) {
    let id = s.curve.id();
    let Some(cut) = s.cut else {
        cx.sketch.geometry.constraints.push(Constraint::OnCurve { point: to, curve: id });
        return;
    };
    for c in cx.sketch.geometry.curves.iter_mut().filter(|c| c.id() == id) {
        match c {
            Curve::Line { a, b, .. } => *(if *a == cut { a } else { b }) = to,
            Curve::Arc { start, end, .. } => *(if *start == cut { start } else { end }) = to,
            _ => {}
        }
    }
}

/// The constraints that no longer hold once the sides are trimmed, and the
/// corner point: kept as a virtual sharp (held on both curves) when anything
/// refers to it or `always`; then a line's length becomes the distance from
/// the sharp to its kept end, so the drawing measures as before.
fn settle(cx: &mut Ctx, sides: &[Side; 2], j: Option<u32>, at: P, always: bool) -> BTreeSet<u32> {
    let ids = [sides[0].curve.id(), sides[1].curve.id()];
    let is_line = |s: &Side| matches!(s.curve, Curve::Line { .. });
    let g = &cx.sketch.geometry;
    let refers = j.is_some_and(|j| g.constraints.iter().any(|c| crate::pathfinder::refs(c).contains(&j)));
    let lengths = g.constraints.iter().any(|c| match c {
        Constraint::Length { line, .. } => ids.contains(line),
        Constraint::Distance { a, b, .. } => a == b && ids.contains(a),
        _ => false,
    });
    let mut keep = BTreeSet::new();
    let sharp = if refers || lengths || always {
        let j = match j {
            Some(j) => j,
            None => cx.point(at),
        };
        keep.insert(j);
        Some(j)
    } else {
        None
    };
    let kept_end = |s: &Side| s.keep;
    let mut out = vec![];
    for c in std::mem::take(&mut cx.sketch.geometry.constraints) {
        let side_of = |id: u32| sides.iter().find(|s| s.curve.id() == id && is_line(s));
        match &c {
            Constraint::Length { line, key } | Constraint::Distance { a: line, key, .. } if side_of(*line).is_some() && (matches!(c, Constraint::Length { .. }) || matches!(c, Constraint::Distance { a, b, .. } if a == b)) => {
                if let (Some(j), Some(k)) = (sharp, kept_end(side_of(*line).unwrap())) {
                    out.push(Constraint::Distance { a: j, b: k, key: key.clone() });
                }
            }
            Constraint::Midpoint { line, .. } if side_of(*line).is_some() => {}
            Constraint::Equal { a, b } if side_of(*a).is_some() || side_of(*b).is_some() => {}
            // Two curves tangent at the corner have no corner to round.
            Constraint::Tangent { a, b, .. } if ids.contains(a) && ids.contains(b) => {}
            _ => out.push(c),
        }
    }
    cx.sketch.geometry.constraints = out;
    if let Some(j) = sharp {
        for id in ids {
            cx.sketch.geometry.constraints.push(Constraint::OnCurve { point: j, curve: id });
        }
    }
    keep
}

impl Command for FilletSketch {
    const NAME: &'static str = "sketch.fillet";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if !(self.radius.is_finite() && self.radius > 0.0) {
            return Err(bad("a fillet needs a radius above zero"));
        }
        let r = self.radius;
        let mut cx = Ctx::load(tx, self.target)?;
        let (j, jid, sides) = corner(&cx, self.point, self.curves.as_deref(), self.picks.as_deref())?;
        // The centre is r from both curves: where their offsets cross. Of
        // those, the one touching both kept pieces nearest the corner.
        let mut best: Option<(f64, P, P, P)> = None;
        for a in offsets(&sides[0], r) {
            for b in offsets(&sides[1], r) {
                for o in off_meets(&a, &b) {
                    let (t1, t2) = (touch(&sides[0], o), touch(&sides[1], o));
                    if (dist(o, t1) - r).abs() > 1e-6 * r.max(1.0) || (dist(o, t2) - r).abs() > 1e-6 * r.max(1.0) || dist(t1, t2) < 1e-9 {
                        continue;
                    }
                    let (Some(s1), Some(s2)) = (along(&cx, &sides[0], j, t1), along(&cx, &sides[1], j, t2)) else { continue };
                    if best.as_ref().is_none_or(|b| s1 + s2 < b.0) {
                        best = Some((s1 + s2, o, t1, t2));
                    }
                }
            }
        }
        let Some((_, o, t1, t2)) = best else { return Err(bad(format!("a {r} mm round does not fit this corner: try a smaller radius"))) };
        let construction = sides[0].curve.construction() && sides[1].curve.construction();
        let keep = settle(&mut cx, &sides, jid, j, false);
        let (p1, p2, pc) = (cx.point(t1), cx.point(t2), cx.point(o));
        reattach(&mut cx, &sides[0], p1);
        reattach(&mut cx, &sides[1], p2);
        // The arc that faces the corner.
        let (a1, a2) = (angle(o, t1), angle(o, t2));
        let sweep = wrap(a2 - a1);
        let mid = |a: f64| [o[0] + r * a.cos(), o[1] + r * a.sin()];
        let forward = dist(mid(a1 + sweep / 2.0), j) <= dist(mid(a2 + (TAU - sweep) / 2.0), j);
        let (start, end) = if forward { (p1, p2) } else { (p2, p1) };
        let id = cx.id();
        cx.sketch.geometry.curves.push(Curve::Arc { id, center: pc, start, end, construction });
        cx.sketch.geometry.constraints.push(Constraint::Tangent { a: sides[0].curve.id(), b: id, at: Some(p1) });
        cx.sketch.geometry.constraints.push(Constraint::Tangent { a: sides[1].curve.id(), b: id, at: Some(p2) });
        let key = cx.dimension("Fillet radius", r, "mm");
        cx.sketch.geometry.constraints.push(Constraint::Radius { curve: id, key: key.clone() });
        let target = self.target;
        cx.save(tx, target, &keep, true, json!({ "id": target, "curves": [id], "key": key, "points": [p1, p2] }))
    }
}

impl Command for ChamferSketch {
    const NAME: &'static str = "sketch.chamfer";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if !(self.distance.is_finite() && self.distance > 0.0) {
            return Err(bad("a chamfer needs a distance above zero"));
        }
        let d = self.distance;
        let mut cx = Ctx::load(tx, self.target)?;
        let (j, jid, sides) = corner(&cx, self.point, self.curves.as_deref(), self.picks.as_deref())?;
        if sides.iter().any(|s| s.cut.is_none()) {
            return Err(bad("a chamfer cuts between lines and arcs: a whole circle has no corner"));
        }
        // Each cut is `d` from the corner along the kept piece.
        let cut = |s: &Side| -> Result<P, CommandError> {
            let found = match &s.prim {
                Prim::Seg(..) => {
                    let k = cx.p(s.keep.unwrap());
                    let u = unit(sub(k, j));
                    vec![[j[0] + u[0] * d, j[1] + u[1] * d]]
                }
                Prim::Circ(c, rr, ..) => circ_hits(*c, *rr, &Prim::Circ(j, d, 0.0, TAU)),
                Prim::Poly(_) => vec![],
            };
            found
                .into_iter()
                .filter_map(|t| along(&cx, s, j, t).map(|a| (a, t)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, t)| t)
                .ok_or_else(|| bad(format!("a {d} mm chamfer is longer than the curves at this corner: try a smaller distance")))
        };
        let (t1, t2) = (cut(&sides[0])?, cut(&sides[1])?);
        let construction = sides[0].curve.construction() && sides[1].curve.construction();
        let keep = settle(&mut cx, &sides, jid, j, true);
        let sharp = *keep.iter().next().unwrap();
        let (p1, p2) = (cx.point(t1), cx.point(t2));
        reattach(&mut cx, &sides[0], p1);
        reattach(&mut cx, &sides[1], p2);
        let id = cx.id();
        cx.sketch.geometry.curves.push(Curve::Line { id, a: p1, b: p2, construction });
        let key = cx.dimension("Chamfer", d, "mm");
        cx.sketch.geometry.constraints.push(Constraint::Distance { a: sharp, b: p1, key: key.clone() });
        cx.sketch.geometry.constraints.push(Constraint::Distance { a: sharp, b: p2, key: key.clone() });
        let target = self.target;
        cx.save(tx, target, &keep, true, json!({ "id": target, "curves": [id], "key": key, "points": [p1, p2] }))
    }
}
