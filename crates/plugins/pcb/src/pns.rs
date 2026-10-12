//! The interactive router: the geometry under hand routing and dragging.
//!
//! Written from the published behaviour of push-and-shove routers (KiCad's
//! PNS: line placer, walkaround, shove, optimiser), not from their code.
//!
//! Every obstacle — a pad, a track segment, a via, a hole — is seen by the
//! track being laid as an octagon: the obstacle grown by the clearance the two
//! nets need plus half the track's width, its sides on the eight routing
//! directions (squares in 90° mode). A centreline that stays on or outside
//! every octagon keeps every rule, and one that follows an octagon's sides
//! stays at 45°.
//!
//! - *Walkaround*: the line is laid as a 45° dogleg; wherever it enters an
//!   octagon, the part inside is replaced by the octagon's boundary, one way
//!   round or the other, again and again until nothing is entered. Tried
//!   always-left, always-right and shorter-each-time; the shortest that keeps
//!   every rule wins, then the optimiser smooths it. When no way round exists
//!   the line stops short of the first obstacle.
//! - *Shove*: the line walks around what cannot move (pads, holes, arcs) and
//!   pushes the tracks and vias in its way. A pushed track bends round the
//!   pusher's octagons on the side it already lies (never crossing it), and
//!   round any pad it is pushed against; a pushed via steps out the shortest
//!   way and drags its tracks. Whatever moved pushes in turn. It is worked out
//!   afresh from the board for every cursor position, so when the head draws
//!   back the pushed copper springs back. When pushing cannot work (a track end
//!   or a pad in the way) the router walks around instead.
//! - *Highlight*: the line goes where it is put; what it breaks is reported.
//! - The *optimiser* replaces runs of corners by one dogleg wherever that keeps
//!   the rules (fewer corners, shorter).

use crate::board::{BoardView, layer_index};
use crate::drc::gap_point;
use crate::geom::{self, P, Prim};
use crate::Board;
use de_core::{StableId, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, VecDeque};

/// Octagons sit this much beyond the rule, so a line along one never reads as a violation.
const MARGIN: f64 = 0.002;
const EPS: f64 = 1e-6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Go round obstacles.
    #[default]
    Walkaround,
    /// Push tracks and vias aside (round pads), walking round when they cannot move.
    Shove,
    /// Go where put; report collisions.
    Highlight,
}

/// Line angles: 45° (default), 90° or free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Angle {
    #[default]
    #[serde(rename = "45")]
    Diagonal,
    #[serde(rename = "90")]
    Orthogonal,
    #[serde(rename = "free")]
    Free,
}

impl Angle {
    fn dirs(self) -> usize {
        if self == Angle::Orthogonal { 4 } else { 8 }
    }
}

// ---- Shapes ----------------------------------------------------------------

/// A rounded convex (as `geom::Prim`) with its hull and box worked out once.
#[derive(Clone, Debug)]
pub struct Shape {
    pub hull: Vec<P>,
    pub r: f64,
    pub bbox: [f64; 4],
}

impl Shape {
    pub fn of(p: &Prim) -> Shape {
        Shape { hull: geom::convex_hull(&p.pts), r: p.r, bbox: p.bbox() }
    }
    pub fn capsule(a: P, b: P, r: f64) -> Shape {
        Shape::of(&Prim::capsule(a, b, r))
    }
    pub fn circle(c: P, r: f64) -> Shape {
        Shape::of(&Prim::circle(c, r))
    }
    pub fn prim(&self) -> Prim {
        Prim { pts: self.hull.clone(), r: self.r }
    }
}

fn edges_of(h: &[P]) -> Vec<(P, P)> {
    match h.len() {
        0 => vec![],
        1 => vec![(h[0], h[0])],
        2 => vec![(h[0], h[1])],
        n => (0..n).map(|i| (h[i], h[(i + 1) % n])).collect(),
    }
}

/// Gap between two shapes (as `geom::gap`, with hulls already made).
pub fn gap(a: &Shape, b: &Shape) -> f64 {
    if (a.hull.len() >= 3 && b.hull.iter().any(|p| geom::in_convex(*p, &a.hull))) || (b.hull.len() >= 3 && a.hull.iter().any(|p| geom::in_convex(*p, &b.hull))) {
        return -(a.r + b.r) - 1e-6;
    }
    let mut d = f64::INFINITY;
    for (a0, a1) in edges_of(&a.hull) {
        for (b0, b1) in edges_of(&b.hull) {
            d = d.min(geom::seg_seg_dist(a0, a1, b0, b1));
        }
    }
    d - a.r - b.r
}

/// The octagon (4 directions: the square) around `s` grown by `grow`, counter-clockwise.
pub fn hull(s: &Shape, grow: f64, dirs: usize) -> Vec<P> {
    let normals: Vec<P> = (0..dirs).map(|i| {
        let a = std::f64::consts::TAU * i as f64 / dirs as f64;
        let (y, x) = a.sin_cos();
        [snap(x), snap(y)]
    }).collect();
    let support: Vec<f64> = normals.iter().map(|n| s.hull.iter().map(|p| geom::dot(*p, *n)).fold(f64::NEG_INFINITY, f64::max) + s.r + grow).collect();
    let mut out: Vec<P> = Vec::with_capacity(dirs);
    for i in 0..dirs {
        let j = (i + 1) % dirs;
        let (a, b) = (normals[i], normals[j]);
        let det = geom::cross(a, b);
        let p = [(support[i] * b[1] - support[j] * a[1]) / det, (a[0] * support[j] - b[0] * support[i]) / det];
        if out.last().is_none_or(|q| geom::dist(*q, p) > 1e-9) {
            out.push(p);
        }
    }
    if out.len() > 1 && geom::dist(out[0], *out.last().unwrap()) < 1e-9 {
        out.pop();
    }
    out
}

fn snap(v: f64) -> f64 {
    if v.abs() < 1e-12 { 0.0 } else { v }
}

/// Strictly inside a counter-clockwise convex polygon.
fn inside(p: P, poly: &[P]) -> bool {
    let n = poly.len();
    n >= 3
        && (0..n).all(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let e = geom::sub(b, a);
            geom::cross(e, geom::sub(p, a)) / geom::len(e).max(1e-12) > 1e-7
        })
}

fn bbox_of(pts: &[P]) -> [f64; 4] {
    pts.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

// ---- Paths -----------------------------------------------------------------

/// The eight routing directions.
fn is_octilinear(v: P) -> bool {
    let l = geom::len(v);
    l < 1e-9 || v[0].abs() < 1e-7 * l || v[1].abs() < 1e-7 * l || (v[0].abs() - v[1].abs()).abs() < 1e-7 * l
}

/// From `a` to `b` in the line mode: 45° (diagonal first or straight first),
/// 90° (horizontal first or vertical first) or straight.
pub fn connect(a: P, b: P, angle: Angle, first: bool) -> Vec<P> {
    let d = geom::sub(b, a);
    match angle {
        Angle::Free => vec![a, b],
        Angle::Orthogonal => {
            if d[0].abs() < 1e-9 || d[1].abs() < 1e-9 {
                vec![a, b]
            } else if first {
                vec![a, [b[0], a[1]], b]
            } else {
                vec![a, [a[0], b[1]], b]
            }
        }
        Angle::Diagonal => {
            if is_octilinear(d) {
                return vec![a, b];
            }
            let m = d[0].abs().min(d[1].abs());
            let diag = [m * d[0].signum(), m * d[1].signum()];
            if first { vec![a, geom::add(a, diag), b] } else { vec![a, geom::sub(b, diag), b] }
        }
    }
}

/// Repeated points, straight-through corners and spikes (a line doubling back on itself) removed.
pub fn clean(points: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for p in points {
        if out.last().is_some_and(|q| geom::dist(*q, *p) < 1e-7) {
            continue;
        }
        out.push(*p);
        while out.len() >= 3 {
            let n = out.len();
            let (a, b, c) = (out[n - 3], out[n - 2], out[n - 1]);
            let (u, v) = (geom::sub(b, a), geom::sub(c, b));
            if geom::cross(u, v).abs() <= 1e-9 * (geom::len(u) * geom::len(v)).max(1e-12) {
                out.remove(n - 2);
                if geom::dist(out[out.len() - 2], out[out.len() - 1]) < 1e-7 {
                    out.pop();
                }
            } else {
                break;
            }
        }
    }
    out
}

pub fn length(points: &[P]) -> f64 {
    points.windows(2).map(|w| geom::dist(w[0], w[1])).sum()
}

fn point_at(path: &[P], s: f64) -> P {
    let k = (s.floor() as usize).min(path.len() - 2);
    let t = s - k as f64;
    let (a, b) = (path[k], path[k + 1]);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Where the path goes inside `poly`: (first entry, last exit) as path parameters (segment + t).
fn pass_through(path: &[P], poly: &[P]) -> Option<(f64, f64)> {
    let mut s: Vec<f64> = vec![0.0, (path.len() - 1) as f64];
    let n = poly.len();
    for (k, w) in path.windows(2).enumerate() {
        let r = geom::sub(w[1], w[0]);
        for i in 0..n {
            let (c, d) = (poly[i], poly[(i + 1) % n]);
            let e = geom::sub(d, c);
            let den = geom::cross(r, e);
            if den.abs() < 1e-12 {
                continue;
            }
            let ca = geom::sub(c, w[0]);
            let t = geom::cross(ca, e) / den;
            let u = geom::cross(ca, r) / den;
            if (-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u) {
                s.push(k as f64 + t.clamp(0.0, 1.0));
            }
        }
    }
    s.sort_by(f64::total_cmp);
    s.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let mut first = None;
    let mut last = 0.0;
    for w in s.windows(2) {
        if w[1] - w[0] < 1e-9 {
            continue;
        }
        if inside(point_at(path, (w[0] + w[1]) / 2.0), poly) {
            first.get_or_insert(w[0]);
            last = w[1];
        }
    }
    first.map(|f| (f, last))
}

fn nearest_edge(poly: &[P], p: P) -> usize {
    let n = poly.len();
    (0..n).min_by(|a, b| geom::point_seg_dist(p, poly[*a], poly[(*a + 1) % n]).total_cmp(&geom::point_seg_dist(p, poly[*b], poly[(*b + 1) % n]))).unwrap_or(0)
}

/// Along the boundary of `poly` from `pe` (on edge `ei`) to `px` (on edge `ej`).
fn around(poly: &[P], pe: P, ei: usize, px: P, ej: usize, ccw: bool) -> Vec<P> {
    let n = poly.len();
    let along = |p: P, i: usize| geom::dot(geom::sub(p, poly[i]), geom::sub(poly[(i + 1) % n], poly[i]));
    let mut out = vec![pe];
    if ccw {
        if !(ei == ej && along(px, ei) >= along(pe, ei)) {
            let mut i = ei;
            for _ in 0..=n {
                i = (i + 1) % n;
                out.push(poly[i]);
                if i == ej {
                    break;
                }
            }
        }
    } else if !(ei == ej && along(px, ei) <= along(pe, ei)) {
        let mut i = ei;
        for _ in 0..=n {
            out.push(poly[i]);
            if i == (ej + 1) % n {
                break;
            }
            i = (i + n - 1) % n;
        }
    }
    out.push(px);
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Way {
    Shorter,
    Ccw,
    Cw,
}

/// The path bent round every octagon it enters (ends stay). `prefer` ranks the two ways round
/// (true first), then length. None when it keeps going round or an end is inside one.
pub fn walk(path: &[P], hulls: &[Vec<P>], way: Way, prefer: &dyn Fn(&[P]) -> bool) -> Option<Vec<P>> {
    let mut path = clean(path);
    if path.len() < 2 {
        return Some(path);
    }
    let boxes: Vec<[f64; 4]> = hulls.iter().map(|h| bbox_of(h)).collect();
    let mut hits = vec![0usize; hulls.len()];
    for _ in 0..400 {
        let pb = bbox_of(&path);
        let mut first: Option<(f64, f64, usize)> = None;
        for (h, poly) in hulls.iter().enumerate() {
            if poly.len() < 3 || !geom::bbox_overlap(&pb, &boxes[h], 1e-6) {
                continue;
            }
            if let Some((se, sx)) = pass_through(&path, poly) {
                if first.is_none_or(|f| se < f.0) {
                    first = Some((se, sx, h));
                }
            }
        }
        let Some((se, sx, h)) = first else { return Some(path) };
        let last = (path.len() - 1) as f64;
        if se < 1e-9 || sx > last - 1e-9 {
            return None; // an end is inside
        }
        hits[h] += 1;
        if hits[h] > 8 {
            return None;
        }
        let poly = &hulls[h];
        let (pe, px) = (point_at(&path, se), point_at(&path, sx));
        let (ei, ej) = (nearest_edge(poly, pe), nearest_edge(poly, px));
        let n = path.len();
        let ke = (se.floor() as usize).min(n - 2);
        let kx = (sx.floor() as usize).min(n - 2);
        let splice = |mid: Vec<P>| -> Vec<P> {
            let mut out: Vec<P> = path[..=ke].to_vec();
            out.extend(mid);
            out.extend_from_slice(&path[kx + 1..]);
            clean(&out)
        };
        let a = splice(around(poly, pe, ei, px, ej, true));
        let b = splice(around(poly, pe, ei, px, ej, false));
        path = match way {
            Way::Ccw => a,
            Way::Cw => b,
            Way::Shorter => {
                let (pa, pb) = (prefer(&a), prefer(&b));
                if pa != pb {
                    if pa { a } else { b }
                } else if length(&a) <= length(&b) {
                    a
                } else {
                    b
                }
            }
        };
    }
    None
}

/// Fewer corners: each run of corners replaced by one connection wherever `ok` allows.
pub fn smooth(path: &[P], angle: Angle, ok: &dyn Fn(&[P]) -> bool) -> Vec<P> {
    let path = clean(path);
    let n = path.len();
    if n < 3 {
        return path;
    }
    let mut out = vec![path[0]];
    let mut i = 0;
    while i < n - 1 {
        let mut jumped = false;
        'far: for j in (i + 2..n).rev() {
            let now = length(&path[i..=j]);
            for first in [false, true] {
                let c = connect(path[i], path[j], angle, first);
                if c.len() < j - i + 1 && length(&c) <= now + 1e-9 && ok(&c) {
                    out.extend_from_slice(&c[1..]);
                    i = j;
                    jumped = true;
                    break 'far;
                }
            }
        }
        if !jumped {
            out.push(path[i + 1]);
            i += 1;
        }
    }
    clean(&out)
}

/// The longest start of `path` that `ok` allows (the line stops short of an obstacle).
pub fn trim(path: &[P], ok: &dyn Fn(&[P]) -> bool) -> Vec<P> {
    let mut out = vec![path[0]];
    for w in path.windows(2) {
        let mut probe = out.clone();
        probe.push(w[1]);
        if ok(&probe) {
            out.push(w[1]);
            continue;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            let mut probe = out.clone();
            probe.push([w[0][0] + (w[1][0] - w[0][0]) * mid, w[0][1] + (w[1][1] - w[0][1]) * mid]);
            if ok(&probe) { lo = mid } else { hi = mid }
        }
        if lo > 1e-6 {
            out.push([w[0][0] + (w[1][0] - w[0][0]) * lo, w[0][1] + (w[1][1] - w[0][1]) * lo]);
        }
        break;
    }
    out
}

fn crosses(path: &[P], lines: &[(P, P)]) -> bool {
    path.windows(2).any(|w| lines.iter().any(|(a, b)| seg_cross(w[0], w[1], *a, *b)))
}

fn seg_cross(a0: P, a1: P, b0: P, b1: P) -> bool {
    let d1 = geom::cross(geom::sub(a1, a0), geom::sub(b0, a0));
    let d2 = geom::cross(geom::sub(a1, a0), geom::sub(b1, a0));
    let d3 = geom::cross(geom::sub(b1, b0), geom::sub(a0, b0));
    let d4 = geom::cross(geom::sub(b1, b0), geom::sub(a1, b0));
    d1 * d2 < -1e-12 && d3 * d4 < -1e-12
}

// ---- Arcs ------------------------------------------------------------------

pub fn check_bulges(points: &[P], bulges: &[f64]) -> Result<(), String> {
    if bulges.len() > points.len().saturating_sub(1) {
        return Err("bulges: at most one per segment".into());
    }
    if bulges.iter().any(|b| !b.is_finite() || b.abs() > 1.0) {
        return Err("a bulge is tan(sweep / 4): -1 to 1 (up to a half circle)".into());
    }
    Ok(())
}

/// Centre, radius, start angle and sweep of segment a→b with `bulge`.
pub fn arc_of(a: P, b: P, bulge: f64) -> (P, f64, f64, f64) {
    let c = geom::dist(a, b);
    let d = geom::sub(b, a);
    let right = [d[1] / c, -d[0] / c];
    let s = bulge * c / 2.0;
    let mid = geom::add([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], [right[0] * s, right[1] * s]);
    let r = (c * c / 4.0 + s * s) / (2.0 * s.abs());
    let centre = geom::sub(mid, [right[0] * s.signum() * r, right[1] * s.signum() * r]);
    let a0 = (a[1] - centre[1]).atan2(a[0] - centre[0]);
    (centre, r, a0, 4.0 * bulge.atan())
}

/// The point halfway along segment a→b with `bulge`.
pub fn arc_mid(a: P, b: P, bulge: f64) -> P {
    let c = geom::dist(a, b);
    let d = geom::sub(b, a);
    let s = bulge * c / 2.0;
    [(a[0] + b[0]) / 2.0 + d[1] / c * s, (a[1] + b[1]) / 2.0 - d[0] / c * s]
}

/// A polyline with arc segments as points (arcs within 2 µm).
pub fn flatten(points: &[P], bulges: &[f64]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for (i, w) in points.windows(2).enumerate() {
        let b = bulges.get(i).copied().unwrap_or(0.0);
        if out.is_empty() {
            out.push(w[0]);
        }
        if b.abs() < 1e-9 || geom::dist(w[0], w[1]) < 1e-9 {
            out.push(w[1]);
            continue;
        }
        let (c, r, a0, sweep) = arc_of(w[0], w[1], b);
        let step = 2.0 * (1.0 - 0.002 / r).clamp(-1.0, 1.0).acos();
        let n = ((sweep.abs() / step.max(1e-3)).ceil() as usize).clamp(2, 128);
        for k in 1..n {
            let a = a0 + sweep * k as f64 / n as f64;
            out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
        }
        out.push(w[1]);
    }
    if out.is_empty() { points.to_vec() } else { out }
}

/// The bulge of the arc from a to b through q (limited to a half circle).
pub fn bulge_through(a: P, b: P, q: P) -> f64 {
    let c = geom::dist(a, b);
    if c < 1e-9 {
        return 0.0;
    }
    let d = [(b[0] - a[0]) / c, (b[1] - a[1]) / c];
    let right = [d[1], -d[0]];
    let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (qx, qy) = (geom::dot(geom::sub(q, m), d), geom::dot(geom::sub(q, m), right));
    if qy.abs() < 1e-9 {
        return 0.0;
    }
    let h = c / 2.0;
    let k = (qx * qx + qy * qy - h * h) / (2.0 * qy);
    let r = (h * h + k * k).sqrt();
    let s = if qy > 0.0 { k + r } else { k - r };
    (s / h).clamp(-1.0, 1.0)
}

/// Corners rounded to `radius` (less where the legs are short): new corners and bulges.
pub fn fillet(points: &[P], radius: f64) -> (Vec<P>, Vec<f64>) {
    let n = points.len();
    if n < 3 || radius <= 0.0 {
        return (points.to_vec(), vec![0.0; n.saturating_sub(1)]);
    }
    let mut pts = vec![points[0]];
    let mut bulges: Vec<f64> = Vec::new();
    for i in 1..n - 1 {
        let (a, p, c) = (points[i - 1], points[i], points[i + 1]);
        let (u, v) = (geom::sub(p, a), geom::sub(c, p));
        let (lu, lv) = (geom::len(u), geom::len(v));
        let turn = geom::cross(u, v).atan2(geom::dot(u, v));
        if turn.abs() < 1e-3 || lu < 1e-9 || lv < 1e-9 {
            bulges.push(0.0);
            pts.push(p);
            continue;
        }
        let half = (turn.abs() / 2.0).tan();
        let room_u = if i == 1 { lu } else { lu / 2.0 };
        let room_v = if i == n - 2 { lv } else { lv / 2.0 };
        let t = (radius * half).min(room_u).min(room_v);
        let p1 = geom::sub(p, [u[0] / lu * t, u[1] / lu * t]);
        let p2 = geom::add(p, [v[0] / lv * t, v[1] / lv * t]);
        if geom::dist(*pts.last().unwrap(), p1) > 1e-9 {
            bulges.push(0.0);
            pts.push(p1);
        }
        bulges.push((turn / 4.0).tan());
        pts.push(p2);
    }
    if geom::dist(*pts.last().unwrap(), points[n - 1]) > 1e-9 {
        bulges.push(0.0);
        pts.push(points[n - 1]);
    }
    (pts, bulges)
}

// ---- The board as the router sees it ----------------------------------------

#[derive(Clone, Debug)]
pub struct Fixed {
    pub shape: Shape,
    pub mask: u8,
    pub net: Option<String>,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub id: Option<StableId>,
    pub net: Option<String>,
    pub layer: usize,
    pub width: f64,
    pub points: Vec<P>,
    /// Cannot be pushed (the line being laid, a dragged line, or one with arcs).
    pub locked: bool,
    pub arcs: bool,
    pub moved: bool,
}

#[derive(Clone, Debug)]
pub struct Via {
    pub id: Option<StableId>,
    pub net: Option<String>,
    pub at: P,
    pub diameter: f64,
    pub drill: f64,
    pub locked: bool,
    pub moved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ref {
    Fixed(usize),
    Line(usize, usize),
    Via(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pusher {
    Line(usize),
    Via(usize),
}

/// A clearance broken: where, on which layer, by what against what.
#[derive(Clone, Debug, Serialize)]
pub struct Collision {
    pub at: P,
    pub layer: Option<String>,
    pub with: String,
    pub gap: f64,
    pub need: f64,
}

#[derive(Clone)]
pub struct Node {
    pub layers: Vec<String>,
    pub fixed: Vec<Fixed>,
    pub lines: Vec<Line>,
    pub vias: Vec<Via>,
    pub outline: Vec<P>,
    pub cutouts: Vec<Vec<P>>,
    pub edge: f64,
    clearances: HashMap<String, f64>,
    default_clearance: f64,
    min_space: f64,
    reach: f64,
}

impl Node {
    pub fn new(board: &Board, v: &BoardView) -> Node {
        let layers = v.layers.clone();
        let all = ((1u16 << layers.len()) - 1) as u8;
        let mut fixed = Vec::new();
        for p in &v.pads {
            let mut mask = 0u8;
            for l in &p.layers {
                if let Some(k) = layer_index(&layers, l) {
                    mask |= 1 << k;
                }
            }
            if mask != 0 {
                fixed.push(Fixed { shape: Shape::of(&p.prim), mask, net: p.net.clone(), label: format!("{}.{}", p.reference, p.number) });
            }
        }
        // Unplated holes: copper keeps clear of them on every layer.
        for h in v.holes.iter().filter(|h| !h.plated) {
            fixed.push(Fixed { shape: Shape::of(&Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0)), mask: all, net: None, label: format!("hole {}", h.owner) });
        }
        let lines = v
            .tracks
            .iter()
            .filter_map(|t| {
                let layer = layer_index(&layers, &t.layer)?;
                let arcs = !t.bulges.is_empty();
                Some(Line { id: Some(t.id), net: t.net.clone(), layer, width: t.width, points: t.points.clone(), locked: arcs, arcs, moved: false })
            })
            .collect();
        let vias = v.vias.iter().map(|x| Via { id: Some(x.id), net: x.net.clone(), at: x.at, diameter: x.diameter, drill: x.drill, locked: false, moved: false }).collect();
        let mut clearances = HashMap::new();
        for n in &v.nets {
            clearances.insert(n.name.clone(), board.class_of(Some(&n.name)).clearance);
        }
        let default_clearance = board.class_of(None).clearance;
        let reach = board.net_classes.iter().map(|c| c.clearance).fold(board.rules.min_space.max(default_clearance), f64::max);
        Node { layers, fixed, lines, vias, outline: v.outline.clone(), cutouts: v.cutouts.clone(), edge: board.rules.edge_clearance, clearances, default_clearance, min_space: board.rules.min_space, reach }
    }

    pub fn all_layers(&self) -> u8 {
        ((1u16 << self.layers.len()) - 1) as u8
    }

    pub fn clearance(&self, a: &Option<String>, b: &Option<String>) -> f64 {
        let c = |n: &Option<String>| n.as_ref().and_then(|n| self.clearances.get(n)).copied().unwrap_or(self.default_clearance);
        c(a).max(c(b)).max(self.min_space)
    }

    pub fn line_by_id(&self, id: StableId) -> Option<usize> {
        self.lines.iter().position(|l| l.id == Some(id))
    }
    pub fn via_by_id(&self, id: StableId) -> Option<usize> {
        self.vias.iter().position(|v| v.id == Some(id))
    }

    pub fn shape(&self, r: Ref) -> Shape {
        match r {
            Ref::Fixed(i) => self.fixed[i].shape.clone(),
            Ref::Line(i, k) => {
                let l = &self.lines[i];
                Shape::capsule(l.points[k], l.points[k + 1], l.width / 2.0)
            }
            Ref::Via(i) => Shape::circle(self.vias[i].at, self.vias[i].diameter / 2.0),
        }
    }

    fn mask_of(&self, r: Ref) -> u8 {
        match r {
            Ref::Fixed(i) => self.fixed[i].mask,
            Ref::Line(i, _) => 1 << self.lines[i].layer,
            Ref::Via(_) => self.all_layers(),
        }
    }

    fn net_of(&self, r: Ref) -> &Option<String> {
        match r {
            Ref::Fixed(i) => &self.fixed[i].net,
            Ref::Line(i, _) => &self.lines[i].net,
            Ref::Via(i) => &self.vias[i].net,
        }
    }

    pub fn label(&self, r: Ref) -> String {
        match r {
            Ref::Fixed(i) => self.fixed[i].label.clone(),
            Ref::Line(i, _) => format!("track on {}", self.lines[i].net.as_deref().unwrap_or("no net")),
            Ref::Via(i) => format!("via on {}", self.vias[i].net.as_deref().unwrap_or("no net")),
        }
    }

    /// Cannot be pushed.
    pub fn immovable(&self, r: Ref) -> bool {
        match r {
            Ref::Fixed(_) => true,
            Ref::Line(i, _) => self.lines[i].locked || self.lines[i].arcs,
            Ref::Via(i) => self.vias[i].locked,
        }
    }

    /// Everything on `mask` near `bbox`.
    fn near(&self, mask: u8, bbox: [f64; 4], reach: f64) -> Vec<Ref> {
        let mut out = Vec::new();
        for (i, f) in self.fixed.iter().enumerate() {
            if f.mask & mask != 0 && geom::bbox_overlap(&f.shape.bbox, &bbox, reach) {
                out.push(Ref::Fixed(i));
            }
        }
        for (i, l) in self.lines.iter().enumerate() {
            if mask & (1 << l.layer) == 0 {
                continue;
            }
            for (k, w) in l.points.windows(2).enumerate() {
                let b = [w[0][0].min(w[1][0]) - l.width / 2.0, w[0][1].min(w[1][1]) - l.width / 2.0, w[0][0].max(w[1][0]) + l.width / 2.0, w[0][1].max(w[1][1]) + l.width / 2.0];
                if geom::bbox_overlap(&b, &bbox, reach) {
                    out.push(Ref::Line(i, k));
                }
            }
        }
        for (i, v) in self.vias.iter().enumerate() {
            let r = v.diameter / 2.0;
            if geom::bbox_overlap(&[v.at[0] - r, v.at[1] - r, v.at[0] + r, v.at[1] + r], &bbox, reach) {
                out.push(Ref::Via(i));
            }
        }
        out
    }

    /// What `s` (of `net`, on `mask`) comes too close to; `skip` leaves things out.
    pub fn collisions(&self, s: &Shape, mask: u8, net: &Option<String>, skip: &dyn Fn(Ref) -> bool) -> Vec<(Ref, f64, f64)> {
        let mut out = Vec::new();
        for r in self.near(mask, s.bbox, self.reach + 0.01) {
            let other = self.net_of(r);
            if (net.is_some() && other == net) || skip(r) {
                continue;
            }
            let need = self.clearance(net, other);
            let g = gap(s, &self.shape(r));
            if g < need - EPS {
                out.push((r, g, need));
            }
        }
        out
    }

    /// Inside the board, the edge clearance kept.
    pub fn in_board(&self, s: &Shape) -> bool {
        if self.outline.len() < 3 {
            return true;
        }
        let p = s.prim();
        s.hull.iter().all(|q| geom::in_polygon(*q, &self.outline) && !self.cutouts.iter().any(|c| geom::in_polygon(*q, c)))
            && std::iter::once(&self.outline).chain(self.cutouts.iter()).all(|e| geom::gap_to_loop(&p, e) >= self.edge - EPS)
    }

    fn path_in_board(&self, pts: &[P], width: f64) -> bool {
        pts.windows(2).all(|w| self.in_board(&Shape::capsule(w[0], w[1], width / 2.0)))
    }

    /// Every collision of a centreline of `width` on `layer`.
    pub fn path_collisions(&self, pts: &[P], width: f64, layer: usize, net: &Option<String>, skip: &dyn Fn(Ref) -> bool) -> Vec<(Ref, f64, f64, Shape)> {
        let mut out = Vec::new();
        for w in pts.windows(2) {
            let s = Shape::capsule(w[0], w[1], width / 2.0);
            for (r, g, need) in self.collisions(&s, 1 << layer, net, skip) {
                out.push((r, g, need, s.clone()));
            }
        }
        out
    }

    pub fn path_clear(&self, pts: &[P], width: f64, layer: usize, net: &Option<String>, skip: &dyn Fn(Ref) -> bool) -> bool {
        pts.windows(2).all(|w| {
            let s = Shape::capsule(w[0], w[1], width / 2.0);
            self.collisions(&s, 1 << layer, net, skip).is_empty()
        }) && self.path_in_board(pts, width)
    }

    /// Octagons of everything on `layer` that `pick` chooses, as a line of `width` on `net` sees them.
    fn hulls(&self, mask: u8, width: f64, net: &Option<String>, dirs: usize, pick: &dyn Fn(Ref) -> bool) -> Vec<(Ref, Vec<P>)> {
        let all = [f64::NEG_INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::INFINITY];
        let mut out = Vec::new();
        for r in self.near(mask, all, 0.0) {
            let other = self.net_of(r);
            if (net.is_some() && other == net) || !pick(r) {
                continue;
            }
            let grow = self.clearance(net, other) + width / 2.0 + MARGIN;
            out.push((r, hull(&self.shape(r), grow, dirs)));
        }
        out
    }

    fn report(&self, hits: &[(Ref, f64, f64, Shape)], layer: Option<usize>) -> Vec<Collision> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for (r, g, need, s) in hits {
            let key = match r {
                Ref::Line(i, _) => Ref::Line(*i, 0),
                x => *x,
            };
            if !seen.insert(key) {
                continue;
            }
            let at = gap_point(&s.prim(), &self.shape(*r).prim());
            let layer = layer.or_else(|| (0..self.layers.len()).find(|k| self.mask_of(*r) & (1 << k) != 0)).map(|k| self.layers[k].clone());
            out.push(Collision { at, layer, with: self.label(*r), gap: (g.max(-1.0) * 1000.0).round() / 1000.0, need: *need });
        }
        out
    }

    // ---- Shove ----

    fn pusher_shapes(&self, p: Pusher) -> Vec<(Shape, u8)> {
        match p {
            Pusher::Line(i) => {
                let l = &self.lines[i];
                l.points.windows(2).map(|w| (Shape::capsule(w[0], w[1], l.width / 2.0), 1u8 << l.layer)).collect()
            }
            Pusher::Via(i) => vec![(Shape::circle(self.vias[i].at, self.vias[i].diameter / 2.0), self.all_layers())],
        }
    }

    fn pusher_net(&self, p: Pusher) -> Option<String> {
        match p {
            Pusher::Line(i) => self.lines[i].net.clone(),
            Pusher::Via(i) => self.vias[i].net.clone(),
        }
    }

    fn is_pusher(p: Pusher, r: Ref) -> bool {
        matches!((p, r), (Pusher::Line(a), Ref::Line(b, _)) if a == b) || matches!((p, r), (Pusher::Via(a), Ref::Via(b)) if a == b)
    }

    /// Push whatever `start` collides with, and whatever that then collides with.
    pub fn shove(&mut self, start: &[Pusher]) -> Result<(), String> {
        let mut queue: VecDeque<Pusher> = start.iter().copied().collect();
        let mut moves: HashMap<Pusher, usize> = HashMap::new();
        let mut steps = 0;
        while let Some(p) = queue.pop_front() {
            steps += 1;
            if steps > 300 {
                return Err("too much to push".into());
            }
            let shapes = self.pusher_shapes(p);
            let net = self.pusher_net(p);
            let mut lines = BTreeSet::new();
            let mut vias = BTreeSet::new();
            for (s, mask) in &shapes {
                for (r, _, _) in self.collisions(s, *mask, &net, &|r| Self::is_pusher(p, r)) {
                    if self.immovable(r) {
                        return Err(format!("{} is in the way", self.label(r)));
                    }
                    match r {
                        Ref::Line(i, _) => {
                            lines.insert(i);
                        }
                        Ref::Via(i) => {
                            vias.insert(i);
                        }
                        Ref::Fixed(_) => unreachable!(),
                    }
                }
            }
            // Vias first: a track ending on a pushed via goes with it, and may then be clear.
            for i in vias {
                let attached = self.shove_via(i, p)?;
                let n = moves.entry(Pusher::Via(i)).or_default();
                *n += 1;
                if *n > 8 {
                    return Err("vias keep pushing each other".into());
                }
                queue.push_back(Pusher::Via(i));
                queue.extend(attached.into_iter().map(Pusher::Line));
            }
            for i in lines {
                let mask = 1u8 << self.lines[i].layer;
                let still = shapes.iter().any(|(s, m)| m & mask != 0 && !self.collisions(s, mask, &net, &|r| !matches!(r, Ref::Line(j, _) if j == i)).is_empty());
                if !still {
                    continue;
                }
                self.shove_line(i, p)?;
                let n = moves.entry(Pusher::Line(i)).or_default();
                *n += 1;
                if *n > 8 {
                    return Err("tracks keep pushing each other".into());
                }
                queue.push_back(Pusher::Line(i));
            }
        }
        Ok(())
    }

    /// Bend line `li` round `p` on the side it lies, and round anything fixed it then meets.
    fn shove_line(&mut self, li: usize, p: Pusher) -> Result<(), String> {
        let l = self.lines[li].clone();
        let net = self.pusher_net(p);
        let grow = self.clearance(&l.net, &net) + l.width / 2.0 + MARGIN;
        let mask = 1u8 << l.layer;
        let shapes = self.pusher_shapes(p);
        // Only the pusher's pieces that come too close: the rest may lie at exactly the clearance.
        let need = self.clearance(&l.net, &net);
        let caps: Vec<Shape> = l.points.windows(2).map(|w| Shape::capsule(w[0], w[1], l.width / 2.0)).collect();
        let mut hulls: Vec<Vec<P>> = shapes.iter().filter(|(s, m)| m & mask != 0 && caps.iter().any(|c| gap(s, c) < need - EPS)).map(|(s, _)| hull(s, grow, 8)).collect();
        let centre: Vec<(P, P)> = match p {
            Pusher::Line(i) => self.lines[i].points.windows(2).map(|w| (w[0], w[1])).collect(),
            Pusher::Via(_) => vec![],
        };
        let (a, b) = (l.points[0], *l.points.last().unwrap());
        for _ in 0..5 {
            if hulls.iter().any(|h| inside(a, h) || inside(b, h)) {
                return Err(format!("the end of a {} track is in the way", l.net.as_deref().unwrap_or("no-net")));
            }
            let prefer = |c: &[P]| !crosses(c, &centre) && self.path_in_board(c, l.width);
            let walked = walk(&l.points, &hulls, Way::Shorter, &prefer).ok_or("a track cannot be pushed round")?;
            if crosses(&walked, &centre) {
                return Err("a pushed track would cross the line".into());
            }
            if !self.path_in_board(&walked, l.width) {
                return Err("a pushed track would leave the board".into());
            }
            let blockers: Vec<(Ref, f64, f64, Shape)> = self.path_collisions(&walked, l.width, l.layer, &l.net, &|r| matches!(r, Ref::Line(i, _) if i == li) || (!self.immovable(r) && !Self::is_pusher(p, r)));
            if blockers.is_empty() {
                let line = &mut self.lines[li];
                line.points = smooth_pushed(&walked);
                line.moved = true;
                return Ok(());
            }
            for (r, ..) in blockers {
                let g = self.clearance(&l.net, self.net_of(r)) + l.width / 2.0 + MARGIN;
                hulls.push(hull(&self.shape(r), g, 8));
            }
        }
        Err("a pushed track has nowhere to go".into())
    }

    /// Step via `vi` out of `p`'s way the shortest way; its tracks follow. Returns those tracks.
    fn shove_via(&mut self, vi: usize, p: Pusher) -> Result<Vec<usize>, String> {
        let v = self.vias[vi].clone();
        let net = self.pusher_net(p);
        let grow = self.clearance(&v.net, &net) + v.diameter / 2.0 + MARGIN;
        let hulls: Vec<Vec<P>> = self.pusher_shapes(p).iter().map(|(s, _)| hull(s, grow, 8)).collect();
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let mut dirs: Vec<P> = vec![[1.0, 0.0], [d, d], [0.0, 1.0], [-d, d], [-1.0, 0.0], [-d, -d], [0.0, -1.0], [d, -d]];
        // Straight away from the pusher first.
        if let Some(c) = self.pusher_shapes(p).iter().map(|(s, _)| crate::drc::gap_point(&s.prim(), &Prim::circle(v.at, 0.0))).next() {
            let away = geom::sub(v.at, c);
            if geom::len(away) > 1e-9 {
                dirs.insert(0, [away[0] / geom::len(away), away[1] / geom::len(away)]);
            }
        }
        let mut best: Option<(f64, P)> = None;
        for u in dirs {
            let mut t = 0.0;
            for _ in 0..12 {
                let q = [v.at[0] + u[0] * t, v.at[1] + u[1] * t];
                let Some(h) = hulls.iter().find(|h| inside(q, h)) else { break };
                t += exit_along(q, u, h) + 1e-4;
            }
            let q = [v.at[0] + u[0] * t, v.at[1] + u[1] * t];
            if hulls.iter().any(|h| inside(q, h)) || best.is_some_and(|(b, _)| t >= b) {
                continue;
            }
            let s = Shape::circle(q, v.diameter / 2.0);
            if !self.in_board(&s) || !self.collisions(&s, self.all_layers(), &v.net, &|r| r == Ref::Via(vi) || !self.immovable(r) || Self::is_pusher(p, r)).is_empty() {
                continue;
            }
            best = Some((t, q));
        }
        let (_, to) = best.ok_or("a via has nowhere to go")?;
        let attached: Vec<usize> = (0..self.lines.len()).filter(|&i| {
            let l = &self.lines[i];
            l.net == v.net && (geom::dist(l.points[0], v.at) < 1e-6 || geom::dist(*l.points.last().unwrap(), v.at) < 1e-6)
        }).collect();
        for &i in &attached {
            if self.lines[i].locked || self.lines[i].arcs {
                return Err("a via with the line on it cannot move".into());
            }
            let pts = follow_end(&self.lines[i].points, v.at, to);
            let line = &mut self.lines[i];
            line.points = pts;
            line.moved = true;
        }
        self.vias[vi].at = to;
        self.vias[vi].moved = true;
        Ok(attached)
    }
}

/// How far along `u` from `p` (inside) to leave the convex `poly`.
fn exit_along(p: P, u: P, poly: &[P]) -> f64 {
    let n = poly.len();
    let mut t = f64::INFINITY;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let e = geom::sub(b, a);
        let l = geom::len(e).max(1e-12);
        let normal = [e[1] / l, -e[0] / l];
        let du = geom::dot(normal, u);
        if du > 1e-12 {
            t = t.min((geom::dot(normal, a) - geom::dot(normal, p)) / du);
        }
    }
    t.max(0.0)
}

/// A pushed line with its repeated and straight-through corners gone.
fn smooth_pushed(points: &[P]) -> Vec<P> {
    clean(points)
}

/// The end of `points` at `from` moved to `to`, its last leg bent at 45° (keeping the way it arrived).
pub fn follow_end(points: &[P], from: P, to: P) -> Vec<P> {
    let mut pts = points.to_vec();
    let at_start = geom::dist(pts[0], from) < geom::dist(*pts.last().unwrap(), from);
    if at_start {
        pts.reverse();
    }
    let n = pts.len();
    let keep = geom::sub(pts[n - 1], pts[n - 2]);
    let anchor = pts[n - 2];
    pts.truncate(n - 2);
    pts.extend(crate::drag::dogleg(anchor, to, Some(keep)));
    let mut pts = clean(&pts);
    if at_start {
        pts.reverse();
    }
    pts
}

// ---- Laying a line ----------------------------------------------------------

/// One leg laid: its centreline, whether it reached the target and how.
pub struct Laid {
    pub points: Vec<P>,
    pub reached: bool,
    pub mode: Mode,
    /// Why it stopped or fell back.
    pub note: Option<String>,
}

pub struct LegSpec<'a> {
    pub from: P,
    pub to: P,
    pub layer: usize,
    pub width: f64,
    pub net: &'a Option<String>,
    pub mode: Mode,
    pub angle: Angle,
    pub first: bool,
    pub optimize: bool,
}

impl Node {
    /// Walk round what `pick` chooses (things whose octagon holds an end are only reported).
    /// Returns the line and whether it reached its end (an end too close to
    /// another net's copper is stopped short of it).
    fn walkaround(&self, s: &LegSpec, pick: &dyn Fn(Ref) -> bool) -> Result<(Vec<P>, bool), ()> {
        let mask = 1u8 << s.layer;
        let all = self.hulls(mask, s.width, s.net, s.angle.dirs(), pick);
        let at_start: BTreeSet<Ref> = all.iter().filter(|(_, h)| inside(s.from, h)).map(|(r, _)| *r).collect();
        let at_end: BTreeSet<Ref> = all.iter().filter(|(r, h)| inside(s.to, h) && !at_start.contains(r)).map(|(r, _)| *r).collect();
        let hulls: Vec<Vec<P>> = all.into_iter().filter(|(r, _)| !at_start.contains(r) && !at_end.contains(r)).map(|(_, h)| h).collect();
        let skip = |r: Ref| !pick(r) || at_start.contains(&r) || at_end.contains(&r);
        let ok = |c: &[P]| self.path_clear(c, s.width, s.layer, s.net, &skip);
        let strict = |c: &[P]| self.path_clear(c, s.width, s.layer, s.net, &|r: Ref| !pick(r) || at_start.contains(&r));
        let done = |p: Vec<P>| if at_end.is_empty() { (p, true) } else { (trim(&p, &strict), false) };
        let start = connect(s.from, s.to, s.angle, s.first);
        if ok(&start) {
            return Ok(done(if s.optimize { smooth(&start, s.angle, &ok) } else { start }));
        }
        let prefer = |c: &[P]| self.path_in_board(c, s.width);
        let mut best: Option<Vec<P>> = None;
        for way in [Way::Shorter, Way::Ccw, Way::Cw] {
            let Some(p) = walk(&start, &hulls, way, &prefer) else { continue };
            if !ok(&p) {
                continue;
            }
            let p = if s.optimize { smooth(&p, s.angle, &ok) } else { p };
            if best.as_ref().is_none_or(|b| length(&p) < length(b) - 1e-9) {
                best = Some(p);
            }
        }
        best.map(done).ok_or(())
    }

    /// What `p` is too close to for a line of the leg's width to start there.
    fn holding(&self, p: P, s: &LegSpec) -> BTreeSet<Ref> {
        self.hulls(1 << s.layer, s.width, s.net, s.angle.dirs(), &|_| true).into_iter().filter(|(_, h)| inside(p, h)).map(|(r, _)| r).collect()
    }

    /// Lay one leg in the chosen mode. In shove mode the node keeps what was pushed.
    pub fn lay(&mut self, s: &LegSpec) -> Laid {
        let direct = connect(s.from, s.to, s.angle, s.first);
        match s.mode {
            Mode::Highlight => Laid { points: direct, reached: true, mode: Mode::Highlight, note: None },
            Mode::Walkaround => self.lay_walkaround(s, direct),
            Mode::Shove => {
                let fixed_only = |r: Ref| self.immovable(r);
                let head = self.walkaround(s, &fixed_only).ok();
                if let Some((head, reached)) = head {
                    let mut trial = self.clone();
                    trial.lines.push(Line { id: None, net: s.net.clone(), layer: s.layer, width: s.width, points: head.clone(), locked: true, arcs: false, moved: false });
                    let at = trial.lines.len() - 1;
                    match trial.shove(&[Pusher::Line(at)]) {
                        Ok(()) => {
                            trial.lines.pop();
                            *self = trial;
                            let note = (!reached).then(|| "the end is too close to another net: stopped short".into());
                            return Laid { points: head, reached, mode: Mode::Shove, note };
                        }
                        Err(why) => {
                            let mut l = self.lay_walkaround(s, direct);
                            l.note = Some(format!("cannot shove: {why}; walked round"));
                            return l;
                        }
                    }
                }
                let mut l = self.lay_walkaround(s, direct);
                l.note = Some("no way round the pads; walked round".into());
                l
            }
        }
    }

    fn lay_walkaround(&self, s: &LegSpec, direct: Vec<P>) -> Laid {
        match self.walkaround(s, &|_| true) {
            Ok((p, reached)) => Laid { points: p, reached, mode: Mode::Walkaround, note: (!reached).then(|| "the end is too close to another net: stopped short".into()) },
            Err(()) => {
                let held = self.holding(s.from, s);
                let ok = |c: &[P]| self.path_clear(c, s.width, s.layer, s.net, &|r| held.contains(&r));
                let cut = trim(&direct, &ok);
                Laid { points: cut, reached: false, mode: Mode::Walkaround, note: Some("blocked: stopped at the first obstacle".into()) }
            }
        }
    }

    /// Collisions of a line of `width` on `layer` and `net` (things in `skip` left out).
    pub fn line_report(&self, pts: &[P], width: f64, layer: usize, net: &Option<String>, skip: &dyn Fn(Ref) -> bool) -> Vec<Collision> {
        let hits = self.path_collisions(pts, width, layer, net, skip);
        self.report(&hits, Some(layer))
    }

    pub fn via_report(&self, at: P, diameter: f64, net: &Option<String>, skip: &dyn Fn(Ref) -> bool) -> Vec<Collision> {
        let s = Shape::circle(at, diameter / 2.0);
        let hits: Vec<(Ref, f64, f64, Shape)> = self.collisions(&s, self.all_layers(), net, skip).into_iter().map(|(r, g, n)| (r, g, n, s.clone())).collect();
        self.report(&hits, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(pads: Vec<(Prim, &str)>, lines: Vec<(Vec<P>, &str)>, vias: Vec<(P, &str)>) -> Node {
        let mut clearances = HashMap::new();
        for n in ["A", "B", "C"] {
            clearances.insert(n.to_string(), 0.2);
        }
        Node {
            layers: vec!["top".into(), "bottom".into()],
            fixed: pads.into_iter().map(|(p, n)| Fixed { shape: Shape::of(&p), mask: 1, net: Some(n.into()), label: n.into() }).collect(),
            lines: lines.into_iter().map(|(p, n)| Line { id: None, net: Some(n.into()), layer: 0, width: 0.25, points: p, locked: false, arcs: false, moved: false }).collect(),
            vias: vias.into_iter().map(|(at, n)| Via { id: None, net: Some(n.into()), at, diameter: 0.6, drill: 0.3, locked: false, moved: false }).collect(),
            outline: vec![[-50.0, -50.0], [50.0, -50.0], [50.0, 50.0], [-50.0, 50.0]],
            cutouts: vec![],
            edge: 0.3,
            clearances,
            default_clearance: 0.2,
            min_space: 0.15,
            reach: 0.3,
        }
    }

    fn octilinear_path(p: &[P]) -> bool {
        p.windows(2).all(|w| is_octilinear(geom::sub(w[1], w[0])))
    }

    fn spec<'a>(from: P, to: P, net: &'a Option<String>, mode: Mode) -> LegSpec<'a> {
        LegSpec { from, to, layer: 0, width: 0.25, net, mode, angle: Angle::Diagonal, first: false, optimize: true }
    }

    #[test]
    fn octagon_holds_the_grown_shape() {
        let s = Shape::of(&Prim::rect([0.0, 0.0], 2.0, 1.0, 30.0, 0.1));
        let h = hull(&s, 0.3, 8);
        assert_eq!(h.len(), 8);
        assert!(geom::polygon_area(&h) > 0.0, "counter-clockwise");
        for w in 0..8 {
            assert!(is_octilinear(geom::sub(h[(w + 1) % 8], h[w])));
        }
        // Every point of its boundary keeps the grown distance.
        for i in 0..8 {
            let p = h[i];
            let q = [(h[i][0] + h[(i + 1) % 8][0]) / 2.0, (h[i][1] + h[(i + 1) % 8][1]) / 2.0];
            for x in [p, q] {
                assert!(gap(&s, &Shape::circle(x, 0.0)) >= 0.3 - 1e-9);
            }
        }
    }

    #[test]
    fn walkaround_goes_round_a_pad_and_keeps_clearance() {
        let net = Some("A".to_string());
        let n = node(vec![(Prim::rect([5.0, 0.0], 2.0, 2.0, 0.0, 0.0), "B")], vec![], vec![]);
        let mut n2 = n.clone();
        let laid = n2.lay(&spec([0.0, 0.0], [10.0, 0.0], &net, Mode::Walkaround));
        assert!(laid.reached, "{:?}", laid.note);
        assert!(octilinear_path(&laid.points), "{:?}", laid.points);
        assert!(n.path_clear(&laid.points, 0.25, 0, &net, &|_| false), "{:?}", laid.points);
        assert!(length(&laid.points) < 14.0, "short way round: {}", length(&laid.points));
        assert_eq!(laid.points[0], [0.0, 0.0]);
        assert_eq!(*laid.points.last().unwrap(), [10.0, 0.0]);
    }

    #[test]
    fn walkaround_threads_a_row_of_pads() {
        let net = Some("A".to_string());
        // A wall of pads with a gap between the 3rd and 4th.
        let mut pads = Vec::new();
        for i in 0..6 {
            let y = -6.0 + i as f64 * 1.27 + if i >= 3 { 1.2 } else { 0.0 };
            pads.push((Prim::rect([5.0, y], 1.5, 0.6, 0.0, 0.0), "B"));
        }
        let n = node(pads, vec![], vec![]);
        let mut n2 = n.clone();
        let laid = n2.lay(&spec([0.0, -3.0], [10.0, -3.0], &net, Mode::Walkaround));
        assert!(laid.reached, "{:?}", laid.note);
        assert!(n.path_clear(&laid.points, 0.25, 0, &net, &|_| false), "{:?}", laid.points);
        assert!(octilinear_path(&laid.points));
    }

    #[test]
    fn blocked_line_stops_short() {
        let net = Some("A".to_string());
        // A pad of another net sits on the target.
        let n = node(vec![(Prim::rect([10.0, 0.0], 2.0, 2.0, 0.0, 0.0), "B")], vec![], vec![]);
        let mut n2 = n.clone();
        let laid = n2.lay(&spec([0.0, 0.0], [10.0, 0.0], &net, Mode::Walkaround));
        assert!(!laid.reached);
        assert!(n.path_clear(&laid.points, 0.25, 0, &net, &|_| false));
        let end = *laid.points.last().unwrap();
        assert!(end[0] > 8.0 && end[0] < 9.0, "{end:?}");
    }

    #[test]
    fn shove_pushes_a_track_aside_and_keeps_it_clean() {
        let net = Some("A".to_string());
        // A B track running parallel just above where the A line goes.
        let n = node(vec![], vec![(vec![[-5.0, 0.3], [15.0, 0.3]], "B")], vec![]);
        let mut n2 = n.clone();
        let laid = n2.lay(&spec([0.0, 0.0], [10.0, 0.0], &net, Mode::Shove));
        assert_eq!(laid.mode, Mode::Shove, "{:?}", laid.note);
        assert_eq!(laid.points, vec![[0.0, 0.0], [10.0, 0.0]]);
        let pushed = &n2.lines[0];
        assert!(pushed.moved);
        assert_eq!(pushed.points[0], [-5.0, 0.3]);
        assert_eq!(*pushed.points.last().unwrap(), [15.0, 0.3]);
        assert!(octilinear_path(&pushed.points), "{:?}", pushed.points);
        // Pushed up (the side it was on), clear of the new line.
        assert!(pushed.points.iter().all(|p| p[1] >= 0.3 - 1e-9));
        let mut check = n2.clone();
        check.lines.push(Line { id: None, net: net.clone(), layer: 0, width: 0.25, points: laid.points.clone(), locked: true, arcs: false, moved: false });
        assert!(check.path_clear(&pushed.points, 0.25, 0, &Some("B".into()), &|_| false), "{:?}", pushed.points);
        // Spring-back: the board it started from is untouched.
        assert_eq!(n.lines[0].points, vec![[-5.0, 0.3], [15.0, 0.3]]);
    }

    #[test]
    fn shove_propagates_through_two_tracks() {
        let net = Some("A".to_string());
        let n = node(vec![], vec![(vec![[-5.0, 0.3], [15.0, 0.3]], "B"), (vec![[-5.0, 0.75], [15.0, 0.75]], "C")], vec![]);
        let mut n2 = n.clone();
        let laid = n2.lay(&spec([0.0, 0.0], [10.0, 0.0], &net, Mode::Shove));
        assert_eq!(laid.mode, Mode::Shove, "{:?}", laid.note);
        assert!(n2.lines[0].moved && n2.lines[1].moved);
        let (b, c) = (n2.lines[0].clone(), n2.lines[1].clone());
        let mut check = n2.clone();
        check.lines.push(Line { id: None, net: net.clone(), layer: 0, width: 0.25, points: laid.points.clone(), locked: true, arcs: false, moved: false });
        assert!(check.path_clear(&b.points, 0.25, 0, &b.net, &|_| false), "B {:?}", b.points);
        assert!(check.path_clear(&c.points, 0.25, 0, &c.net, &|_| false), "C {:?}", c.points);
    }

    #[test]
    fn shove_moves_a_via_and_its_track() {
        let net = Some("A".to_string());
        let n = node(vec![], vec![(vec![[5.0, 0.3], [5.0, 6.0]], "B")], vec![([5.0, 0.3], "B")]);
        let mut n2 = n.clone();
        let laid = n2.lay(&spec([0.0, 0.0], [10.0, 0.0], &net, Mode::Shove));
        assert_eq!(laid.mode, Mode::Shove, "{:?}", laid.note);
        let v = &n2.vias[0];
        assert!(v.moved && v.at[1] > 0.3, "{:?}", v.at);
        assert!(geom::dist(n2.lines[0].points[0], v.at) < 1e-9, "track follows the via");
        let mut check = n2.clone();
        check.lines.push(Line { id: None, net: net.clone(), layer: 0, width: 0.25, points: laid.points.clone(), locked: true, arcs: false, moved: false });
        assert!(check.via_report(v.at, v.diameter, &v.net, &|r| r == Ref::Via(0)).is_empty());
    }

    #[test]
    fn shove_falls_back_when_a_track_end_is_in_the_way() {
        let net = Some("A".to_string());
        // A B track ending right on the A line's way: it cannot be pushed, so the line walks round.
        let n = node(vec![], vec![(vec![[5.0, 0.2], [5.0, 6.0]], "B")], vec![]);
        let mut n2 = n.clone();
        let laid = n2.lay(&spec([0.0, 0.0], [10.0, 0.0], &net, Mode::Shove));
        assert_eq!(laid.mode, Mode::Walkaround);
        assert!(laid.reached);
        assert!(n.path_clear(&laid.points, 0.25, 0, &net, &|_| false), "{:?}", laid.points);
    }

    #[test]
    fn smoothing_takes_corners_out() {
        let p = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [3.0, 2.0]];
        let s = smooth(&p, Angle::Diagonal, &|_| true);
        assert!(s.len() < p.len(), "{s:?}");
        assert!(length(&s) <= length(&p));
        assert!(octilinear_path(&s));
    }

    #[test]
    fn arcs_round_trip() {
        let (pts, bulges) = fillet(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]], 1.0);
        assert_eq!(pts.len(), 4);
        assert!((bulges[1] - (std::f64::consts::FRAC_PI_8).tan()).abs() < 1e-9);
        let flat = flatten(&pts, &bulges);
        // Every flattened point of the arc is 1 mm from its centre (4, 1).
        for p in &flat[2..flat.len() - 2] {
            assert!((geom::dist(*p, [4.0, 1.0]) - 1.0).abs() < 1e-6, "{p:?}");
        }
        let mid = arc_mid(pts[1], pts[2], bulges[1]);
        assert!((bulge_through(pts[1], pts[2], mid) - bulges[1]).abs() < 1e-9);
    }
}
