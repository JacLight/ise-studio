//! Length tuning: meanders on a track, on both tracks of a differential pair,
//! or on one track of a pair to take out its skew (`pcb.tune`).
//!
//! A meander is a path of straight runs and turns, laid along a straight
//! segment of the track and spliced into it: trombone (U-shaped legs, corners
//! round, chamfered 45° or square) or sawtooth. The same path offset to each
//! side gives a pair's two tracks: a turn's radius grows or shrinks by the
//! offset and the runs keep their lengths, so a trombone adds exactly the same
//! length to both. The amplitude is solved (bisection on the exact length,
//! arcs counted as arcs) so the net lands on its target; the fewest legs
//! that reach it are used, each placement checked against everything else on
//! the layer and the board edge, and longer segments are tried first.

use crate::board::{self, BoardView, TrackView};
use crate::geom::{self, P};
use crate::pns::{self, Node, Ref};
use crate::signal::{self, PairView};
use crate::{Board, Track, bad, board_of};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f64::consts::FRAC_PI_2;

// ---- Paths ----------------------------------------------------------------------

/// One step of a path: a straight run, or a turn of `a` radians (left positive) on radius `r` (0: a sharp corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Line(f64),
    Turn { r: f64, a: f64 },
}

fn left(dir: f64) -> P {
    [-dir.sin(), dir.cos()]
}

/// Runs shortened (or lengthened) at sharp corners for a path offset `d` to the left.
fn adjusted(steps: &[Step], d: f64) -> Vec<Step> {
    let mut out = steps.to_vec();
    for i in 0..steps.len() {
        if let Step::Turn { r, a } = steps[i] {
            // A sharp corner, or a round one the offset folds through (inside, beyond its radius): mitred.
            if r > 0.0 && r - d * a.signum() >= 0.0 {
                continue;
            }
            let e = (d - r * a.signum()) * (a / 2.0).tan();
            if let Some(j) = (0..i).rev().find(|j| matches!(out[*j], Step::Line(_))) {
                if let Step::Line(l) = &mut out[j] {
                    *l -= e;
                }
            }
            if let Some(j) = (i + 1..steps.len()).find(|j| matches!(out[*j], Step::Line(_))) {
                if let Step::Line(l) = &mut out[j] {
                    *l -= e;
                }
            }
        }
    }
    out
}

/// The path's corners and bulges from `start` heading `dir`, offset `d` to the left.
pub fn realize(steps: &[Step], start: P, dir: f64, d: f64) -> (Vec<P>, Vec<f64>) {
    let mut p = geom::add(start, [left(dir)[0] * d, left(dir)[1] * d]);
    let mut dir = dir;
    let mut pts = vec![p];
    let mut bulges = Vec::new();
    let adj = adjusted(steps, d);
    let n = adj.len();
    for (i, s) in adj.into_iter().enumerate() {
        let last = i + 1 == n;
        match s {
            Step::Line(l) => {
                if l.abs() < 1e-9 {
                    continue;
                }
                p = geom::add(p, [dir.cos() * l, dir.sin() * l]);
                // A mitre folding back at either end: the track's straight run absorbs it.
                if l < 0.0 && pts.len() == 1 {
                    pts[0] = p;
                    continue;
                }
                if l < 0.0 && last {
                    p = *pts.last().unwrap();
                    continue;
                }
                pts.push(p);
                bulges.push(0.0);
            }
            Step::Turn { r, a } => {
                let rr = r - d * a.signum();
                if r <= 0.0 || rr <= 1e-9 {
                    dir += a;
                    continue;
                }
                let n = left(dir);
                let c = geom::add(p, [n[0] * rr * a.signum(), n[1] * rr * a.signum()]);
                let v = geom::sub(p, c);
                let (cs, sn) = (a.cos(), a.sin());
                p = geom::add(c, [v[0] * cs - v[1] * sn, v[0] * sn + v[1] * cs]);
                pts.push(p);
                bulges.push((a / 4.0).tan());
                dir += a;
            }
        }
    }
    (pts, bulges)
}

/// Length of the path offset `d` to the left.
pub fn length(steps: &[Step], d: f64) -> f64 {
    adjusted(steps, d)
        .iter()
        .map(|s| match *s {
            Step::Line(l) => l,
            Step::Turn { r, a } => {
                let rr = r - d * a.signum();
                if r <= 0.0 || rr <= 0.0 { 0.0 } else { rr * a.abs() }
            }
        })
        .sum()
}

// ---- Patterns ----------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// U-shaped legs (the usual meander).
    #[default]
    Trombone,
    /// A zigzag.
    Sawtooth,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    #[default]
    Round,
    /// 45° chamfers.
    Chamfer,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SideSel {
    /// Whichever side has room (away from the partner when tuning skew).
    #[default]
    Auto,
    Left,
    Right,
}

/// How a meander is shaped.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub style: Style,
    pub corner: Corner,
    /// Centre-to-centre spacing of neighbouring legs.
    pub spacing: f64,
    /// Corner radius (round) or chamfer leg; limited by the amplitude and spacing.
    pub radius: f64,
    /// Smallest corner radius (a pair's offset), so the inner track's corners stay real.
    pub min_radius: f64,
}

/// Distance along the baseline one period takes.
pub fn period(sh: &Shape) -> f64 {
    match sh.style {
        Style::Trombone => 2.0 * sh.spacing,
        Style::Sawtooth => sh.spacing,
    }
}

fn corner(sh: &Shape, amp: f64, sign: f64) -> (Vec<Step>, f64) {
    let room = (amp / 2.0).min(sh.spacing / 2.0);
    match sh.corner {
        Corner::Round => {
            let r = sh.radius.min(room).max(sh.min_radius.min(room));
            (vec![Step::Turn { r, a: sign * FRAC_PI_2 }], r)
        }
        Corner::Chamfer => {
            // Offset by `min_radius`, each run between 45° turns shrinks by 2·d·tan 22.5°: keep them all real.
            let shrink = 2.0 * (std::f64::consts::PI / 8.0).tan() * sh.min_radius;
            let c = sh.radius.min(room - shrink / 2.0).max(shrink / std::f64::consts::SQRT_2);
            (vec![Step::Turn { r: 0.0, a: sign * FRAC_PI_2 / 2.0 }, Step::Line(c * std::f64::consts::SQRT_2), Step::Turn { r: 0.0, a: sign * FRAC_PI_2 / 2.0 }], c)
        }
        Corner::Square => (vec![Step::Turn { r: 0.0, a: sign * FRAC_PI_2 }], 0.0),
    }
}

/// `n` periods of amplitude `amp` to side `side` (+1 left, −1 right) of the baseline.
pub fn pattern(sh: &Shape, n: usize, amp: f64, side: f64) -> Vec<Step> {
    // Empty runs at each end take up a mitred first or last corner when the path is offset.
    let mut out = vec![Step::Line(0.0)];
    match sh.style {
        Style::Trombone => {
            let (c1, k) = corner(sh, amp, side);
            let (c2, _) = corner(sh, amp, -side);
            for _ in 0..n {
                out.extend(c1.iter().copied());
                out.push(Step::Line(amp - 2.0 * k));
                out.extend(c2.iter().copied());
                out.push(Step::Line(sh.spacing - 2.0 * k));
                out.extend(c2.iter().copied());
                out.push(Step::Line(amp - 2.0 * k));
                out.extend(c1.iter().copied());
                out.push(Step::Line(sh.spacing - 2.0 * k));
            }
        }
        Style::Sawtooth => {
            let half = sh.spacing / 2.0;
            let a = amp.atan2(half);
            let l = amp.hypot(half) / 2.0;
            for _ in 0..n {
                out.push(Step::Turn { r: 0.0, a: side * a });
                out.push(Step::Line(l));
                out.push(Step::Turn { r: 0.0, a: -side * 2.0 * a });
                out.push(Step::Line(2.0 * l));
                out.push(Step::Turn { r: 0.0, a: side * 2.0 * a });
                out.push(Step::Line(l));
                out.push(Step::Turn { r: 0.0, a: -side * a });
            }
        }
    }
    out.push(Step::Line(0.0));
    out
}

/// Length `n` periods of amplitude `amp` add over the straight run they replace (on the centreline).
pub fn added(sh: &Shape, n: usize, amp: f64) -> f64 {
    length(&pattern(sh, n, amp, 1.0), 0.0) - n as f64 * period(sh)
}

/// The amplitude in [lo, hi] at which `n` periods add `want`.
pub fn amplitude_for(sh: &Shape, n: usize, want: f64, lo: f64, hi: f64) -> Option<f64> {
    if added(sh, n, hi) + 1e-9 < want || added(sh, n, lo) > want + 1e-9 {
        return None;
    }
    let (mut a, mut b) = (lo, hi);
    for _ in 0..100 {
        let m = (a + b) / 2.0;
        if added(sh, n, m) < want {
            a = m;
        } else {
            b = m;
        }
    }
    Some((a + b) / 2.0)
}

// ---- Splicing into a track ------------------------------------------------------------

/// Track corners with `path` (which starts and ends on segment `seg`) put in place of that stretch.
pub fn splice(pts: &[P], bulges: &[f64], seg: usize, path: &[P], pbul: &[f64]) -> (Vec<P>, Vec<f64>) {
    let (a, b) = (pts[seg], pts[seg + 1]);
    let along = |q: P| geom::dot(geom::sub(q, a), geom::sub(b, a));
    let (path, pbul): (Vec<P>, Vec<f64>) = if along(path[0]) > along(*path.last().unwrap()) {
        (path.iter().rev().copied().collect(), pbul.iter().rev().map(|x| -x).collect())
    } else {
        (path.to_vec(), pbul.to_vec())
    };
    let bul = |i: usize| bulges.get(i).copied().unwrap_or(0.0);
    let mut out_p: Vec<P> = pts[..=seg].to_vec();
    let mut out_b: Vec<f64> = (0..seg).map(bul).collect();
    if geom::dist(a, path[0]) > 1e-9 {
        out_b.push(0.0);
        out_p.push(path[0]);
    }
    out_p.extend(path.iter().skip(1).copied());
    out_b.extend(pbul.iter().copied());
    if geom::dist(*path.last().unwrap(), b) > 1e-9 {
        out_b.push(0.0);
        out_p.push(b);
    }
    for i in seg + 1..pts.len() - 1 {
        out_b.push(bul(i));
        out_p.push(pts[i + 1]);
    }
    (out_p, out_b)
}

// ---- Planning ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TuneMode {
    /// Bring the net (or pair, or group) to its length.
    #[default]
    Length,
    /// Even out a pair's two tracks: meander the shorter one.
    Skew,
}

/// `pcb.tune`: length-tune a net, a differential pair (both tracks meandered
/// together, then the skew taken out) or every member of a length group, to
/// `target` (for a group, its target or its longest member; for a pair in
/// skew mode, its longer track). Meanders go on the net's straight segments
/// (or only `track`/`segment`), trombone or sawtooth, round, chamfered or
/// square corners, at most `amplitude` high, legs `spacing` apart.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct Tune {
    pub board: StableId,
    pub net: Option<String>,
    pub pair: Option<String>,
    pub group: Option<String>,
    pub target: Option<f64>,
    pub mode: Option<TuneMode>,
    pub style: Option<Style>,
    pub corner: Option<Corner>,
    /// Highest meander (mm); default 2 (3 for a pair).
    pub amplitude: Option<f64>,
    /// Leg pitch (mm); default 3 track widths (a pair: its pitch plus 3 widths).
    pub spacing: Option<f64>,
    /// Corner radius or chamfer (mm); default half the spacing.
    pub radius: Option<f64>,
    pub side: Option<SideSel>,
    /// Close enough (mm), default 0.01.
    pub tolerance: Option<f64>,
    /// Only meander this track (and its segment `segment`).
    pub track: Option<StableId>,
    pub segment: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Edit {
    pub id: StableId,
    pub points: Vec<P>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bulges: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NetTune {
    pub net: String,
    pub before: f64,
    pub after: f64,
    pub target: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TunePlan {
    /// Every net reached its target (within tolerance).
    pub ok: bool,
    pub edits: Vec<Edit>,
    pub nets: Vec<NetTune>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skew: Option<f64>,
    pub notes: Vec<String>,
}

struct Work<'a> {
    board: &'a Board,
    v: &'a BoardView,
    node: Node,
    geo: BTreeMap<StableId, (Vec<P>, Vec<f64>)>,
    notes: Vec<String>,
    only: Option<(StableId, Option<usize>)>,
}

/// A straight segment of a track that a meander can go on.
struct Seg {
    id: StableId,
    k: usize,
    a: P,
    b: P,
}

impl<'a> Work<'a> {
    fn track(&self, id: StableId) -> &'a TrackView {
        self.v.tracks.iter().find(|t| t.id == id).unwrap()
    }

    fn geom(&self, t: &TrackView) -> (Vec<P>, Vec<f64>) {
        match self.geo.get(&t.id) {
            Some(g) => g.clone(),
            None => {
                let (p, b) = signal::geometry(t);
                (p.to_vec(), b.to_vec())
            }
        }
    }

    fn net_length(&self, net: &str) -> f64 {
        let base = self.v.signal.lengths.iter().find(|l| l.net == net).map(|l| l.length).unwrap_or(0.0);
        let delta: f64 = self
            .v
            .tracks
            .iter()
            .filter(|t| t.net.as_deref() == Some(net) && self.geo.contains_key(&t.id))
            .map(|t| {
                let (p, b) = &self.geo[&t.id];
                signal::path_length(p, b) - signal::track_length(t)
            })
            .sum();
        base + delta
    }

    fn segments(&self, net: &str) -> Vec<Seg> {
        let mut out = Vec::new();
        for t in self.v.tracks.iter().filter(|t| t.net.as_deref() == Some(net)) {
            if self.only.is_some_and(|(id, _)| id != t.id) {
                continue;
            }
            let (p, b) = self.geom(t);
            for k in 0..p.len().saturating_sub(1) {
                if b.get(k).copied().unwrap_or(0.0).abs() > 1e-12 {
                    continue;
                }
                if let Some((_, Some(s))) = self.only {
                    // A segment index names the track as it was given.
                    if !self.geo.contains_key(&t.id) && s != k {
                        continue;
                    }
                }
                out.push(Seg { id: t.id, k, a: p[k], b: p[k + 1] });
            }
        }
        out.sort_by(|x, y| geom::dist(y.a, y.b).total_cmp(&geom::dist(x.a, x.b)));
        out
    }

    fn commit(&mut self, id: StableId, pts: Vec<P>, bul: Vec<f64>) {
        if let Some(i) = self.node.line_by_id(id) {
            self.node.lines[i].points = pns::flatten(&pts, &bul);
            self.node.lines[i].arcs = true;
            self.node.lines[i].locked = true;
        }
        self.geo.insert(id, (pts, bul));
    }

    fn layer(&self, t: &TrackView) -> usize {
        board::layer_index(&self.v.layers, &t.layer).unwrap_or(0)
    }

    /// Meander `net` by `want` mm; `away` keeps the meanders on the side away from that net (a pair's partner).
    fn single(&mut self, net: &str, want: f64, opts: &Opts, away: Option<&str>) -> f64 {
        let mut left_to_add = want;
        let mut guard = 0;
        while left_to_add > opts.tol && guard < 40 {
            guard += 1;
            let mut placed = false;
            for s in self.segments(net) {
                let t = self.track(s.id);
                let w = t.width;
                // Legs three widths apart, closer where the run is short (small skew bumps).
                let spacings: Vec<f64> = match opts.spacing {
                    Some(x) => vec![x.max(w * 1.5)],
                    None => vec![3.0 * w, 2.2 * w],
                };
                for spacing in spacings {
                    let sh = Shape { style: opts.style, corner: opts.corner, spacing, radius: 0.0, min_radius: 0.0 };
                    let sh = Shape { radius: opts.radius.unwrap_or(sh.spacing / 2.0), ..sh };
                    let l = geom::dist(s.a, s.b);
                    let margin = w + sh.spacing / 2.0;
                    let per = period(&sh);
                    let room = l - 2.0 * margin;
                    if room < per {
                        continue;
                    }
                    let nmax = (room / per).floor() as usize;
                    let amax = opts.amplitude.unwrap_or(2.0);
                    let dir = (s.b[1] - s.a[1]).atan2(s.b[0] - s.a[0]);
                    let sides: Vec<f64> = match (opts.side, away) {
                        (SideSel::Left, _) => vec![1.0],
                        (SideSel::Right, _) => vec![-1.0],
                        (SideSel::Auto, Some(o)) => vec![self.side_away(&s, o)],
                        (SideSel::Auto, None) => vec![1.0, -1.0],
                    };
                    let full = added(&sh, nmax, amax);
                    let goal = left_to_add.min(full);
                    // Fewest legs that reach it; fall back to more, lower legs when the room is taken.
                    let n0 = (1..=nmax).find(|n| added(&sh, *n, amax) + 1e-9 >= goal).unwrap_or(nmax);
                    'n: for n in n0..=nmax {
                        let Some(amp) = amplitude_for(&sh, n, goal, 1e-4, amax) else { continue };
                        for &side in &sides {
                            let steps = pattern(&sh, n, amp, side);
                            let off = (l - n as f64 * per) / 2.0;
                            let u = [(s.b[0] - s.a[0]) / l, (s.b[1] - s.a[1]) / l];
                            let start = geom::add(s.a, [u[0] * off, u[1] * off]);
                            let (mp, mb) = realize(&steps, start, dir, 0.0);
                            let flat = pns::flatten(&mp, &mb);
                            let layer = self.layer(t);
                            if !self.node.path_clear(&flat, w, layer, &t.net, &|_| false) {
                                continue;
                            }
                            let (p, b) = self.geom(t);
                            let (np, nb) = splice(&p, &b, s.k, &mp, &mb);
                            let gained = signal::path_length(&np, &nb) - signal::path_length(&p, &b);
                            self.commit(s.id, np, nb);
                            left_to_add -= gained;
                            placed = true;
                            break 'n;
                        }
                    }
                    if placed {
                        break;
                    }
                }
                if placed {
                    break;
                }
            }
            if !placed {
                break;
            }
        }
        if left_to_add > opts.tol {
            self.notes.push(format!("{net}: {:.3} mm more needed, no room left for meanders (raise the amplitude, free a straight run, or route it longer)", left_to_add));
        }
        want - left_to_add
    }

    /// +1 when `other`'s copper is to the right of segment `s` (so meander left), else −1.
    fn side_away(&self, s: &Seg, other: &str) -> f64 {
        let mid = [(s.a[0] + s.b[0]) / 2.0, (s.a[1] + s.b[1]) / 2.0];
        let d = geom::sub(s.b, s.a);
        let mut best = (f64::INFINITY, 1.0);
        for t in self.v.tracks.iter().filter(|t| t.net.as_deref() == Some(other)) {
            for w in t.points.windows(2) {
                let dist = geom::point_seg_dist(mid, w[0], w[1]);
                if dist < best.0 {
                    let q = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
                    best = (dist, if geom::cross(d, geom::sub(q, s.a)) > 0.0 { -1.0 } else { 1.0 });
                }
            }
        }
        best.1
    }

    /// Meander both tracks of a pair together by `want` mm.
    fn pair(&mut self, p: &PairView, want: f64, opts: &Opts) -> f64 {
        let mut left_to_add = want;
        let mut guard = 0;
        while left_to_add > opts.tol && guard < 40 {
            guard += 1;
            let mut placed = false;
            let ps = self.segments(&p.p);
            let ns: Vec<Seg> = {
                let keep = self.only.take();
                let v = self.segments(&p.n);
                self.only = keep;
                v
            };
            'seg: for sp in &ps {
                let tp = self.track(sp.id);
                let lp = geom::dist(sp.a, sp.b);
                let u = [(sp.b[0] - sp.a[0]) / lp, (sp.b[1] - sp.a[1]) / lp];
                for sn in &ns {
                    let tn = self.track(sn.id);
                    if tn.layer != tp.layer {
                        continue;
                    }
                    let un = geom::sub(sn.b, sn.a);
                    let ln = geom::len(un);
                    if geom::cross(u, [un[0] / ln, un[1] / ln]).abs() > 1e-3 {
                        continue;
                    }
                    let side_n = geom::cross(u, geom::sub(sn.a, sp.a));
                    let cc = side_n.abs();
                    let gap = cc - (tp.width + tn.width) / 2.0;
                    if gap < 0.0 || gap > (tp.width + tn.width) * 3.0 + 0.5 {
                        continue;
                    }
                    let (t0, t1) = (geom::dot(geom::sub(sn.a, sp.a), u), geom::dot(geom::sub(sn.b, sp.a), u));
                    let (lo, hi) = (t0.min(t1).max(0.0), t0.max(t1).min(lp));
                    let w = tp.width.max(tn.width);
                    let half = cc / 2.0;
                    let spacing = opts.spacing.unwrap_or(cc + 3.0 * w).max(cc + w * 1.5);
                    let sh = Shape { style: Style::Trombone, corner: if opts.corner == Corner::Square { Corner::Square } else { opts.corner }, spacing, radius: opts.radius.unwrap_or(spacing / 2.0).max(half), min_radius: half };
                    let per = period(&sh);
                    let margin = w + spacing / 2.0;
                    let room = hi - lo - 2.0 * margin;
                    if room < per {
                        continue;
                    }
                    let nmax = (room / per).floor() as usize;
                    let amax = opts.amplitude.unwrap_or(3.0);
                    let amin = 2.0 * half + 1e-4;
                    if amax <= amin {
                        continue;
                    }
                    let full = added(&sh, nmax, amax);
                    let goal = left_to_add.min(full);
                    let n0 = (1..=nmax).find(|n| added(&sh, *n, amax) + 1e-9 >= goal).unwrap_or(nmax);
                    let dir = u[1].atan2(u[0]);
                    let towards_n = side_n.signum();
                    let sides: Vec<f64> = match opts.side {
                        SideSel::Left => vec![1.0],
                        SideSel::Right => vec![-1.0],
                        SideSel::Auto => vec![1.0, -1.0],
                    };
                    for n in n0..=nmax {
                        let Some(amp) = amplitude_for(&sh, n, goal, amin, amax) else { continue };
                        for &side in &sides {
                            let steps = pattern(&sh, n, amp, side);
                            let off = lo + (hi - lo - n as f64 * per) / 2.0;
                            let centre = geom::add(sp.a, [u[0] * off + left(dir)[0] * towards_n * half, u[1] * off + left(dir)[1] * towards_n * half]);
                            let (pp, pb) = realize(&steps, centre, dir, -towards_n * half);
                            let (np, nb) = realize(&steps, centre, dir, towards_n * half);
                            let layer = self.layer(tp);
                            let (fp, fnn) = (pns::flatten(&pp, &pb), pns::flatten(&np, &nb));
                            let on = |q: P, a: P, b: P| geom::point_seg_dist(q, a, b) < 1e-6;
                            let (ip, inn) = (self.node.line_by_id(sp.id), self.node.line_by_id(sn.id));
                            let node = &self.node;
                            let skip_n = |r: Ref| match r {
                                Ref::Line(i, k) if Some(i) == inn => {
                                    let l = &node.lines[i].points;
                                    on(l[k], sn.a, sn.b) && on(l[k + 1], sn.a, sn.b)
                                }
                                _ => false,
                            };
                            let skip_p = |r: Ref| match r {
                                Ref::Line(i, k) if Some(i) == ip => {
                                    let l = &node.lines[i].points;
                                    on(l[k], sp.a, sp.b) && on(l[k + 1], sp.a, sp.b)
                                }
                                _ => false,
                            };
                            if !node.path_clear(&fp, tp.width, layer, &tp.net, &skip_n) || !node.path_clear(&fnn, tn.width, layer, &tn.net, &skip_p) {
                                continue;
                            }
                            let (gp, gb) = self.geom(tp);
                            let (sp2, sb2) = splice(&gp, &gb, sp.k, &pp, &pb);
                            let (gn, gnb) = self.geom(tn);
                            let (sn2, snb2) = splice(&gn, &gnb, sn.k, &np, &nb);
                            let gained = signal::path_length(&sp2, &sb2) - signal::path_length(&gp, &gb);
                            self.commit(sp.id, sp2, sb2);
                            self.commit(sn.id, sn2, snb2);
                            left_to_add -= gained;
                            placed = true;
                            break 'seg;
                        }
                    }
                }
            }
            if !placed {
                break;
            }
        }
        if left_to_add > opts.tol {
            self.notes.push(format!("pair {}: {:.3} mm more needed, no coupled straight run with room for meanders", p.name, left_to_add));
        }
        want - left_to_add
    }
}

struct Opts {
    style: Style,
    corner: Corner,
    amplitude: Option<f64>,
    spacing: Option<f64>,
    radius: Option<f64>,
    side: SideSel,
    tol: f64,
}

pub fn plan(b: &Board, v: &BoardView, t: &Tune) -> Result<TunePlan, String> {
    let opts = Opts {
        style: t.style.unwrap_or_default(),
        corner: t.corner.unwrap_or_default(),
        amplitude: t.amplitude,
        spacing: t.spacing,
        radius: t.radius,
        side: t.side.unwrap_or_default(),
        tol: t.tolerance.unwrap_or(0.01).max(1e-6),
    };
    for (x, w) in [(t.amplitude, "amplitude"), (t.spacing, "spacing"), (t.radius, "radius")] {
        if x.is_some_and(|x| !(x.is_finite() && x > 0.0 && x <= 50.0)) {
            return Err(format!("{w} is above 0 and at most 50 mm"));
        }
    }
    if t.target.is_some_and(|x| !(x.is_finite() && x > 0.0)) {
        return Err("target is a length above 0 (mm)".into());
    }
    let mut w = Work { board: b, v, node: Node::new(b, v), geo: BTreeMap::new(), notes: vec![], only: t.track.map(|id| (id, t.segment)) };
    let _ = w.board;
    if let Some(id) = t.track {
        if !v.tracks.iter().any(|x| x.id == id) {
            return Err(format!("{id} is not a track on this board"));
        }
    }
    let mode = t.mode.unwrap_or_default();
    let mut nets: Vec<NetTune> = Vec::new();
    let mut tune_net = |w: &mut Work, net: &str, target: f64, away: Option<&str>| {
        let before = w.net_length(net);
        let want = target - before;
        if want < -opts.tol {
            w.notes.push(format!("{net} is {:.3} mm, already {:.3} mm longer than {:.3} mm: shorten its route", before, -want, target));
        } else if want > opts.tol {
            w.single(net, want, &opts, away);
        }
        nets.push(NetTune { net: net.to_string(), before, after: w.net_length(net), target });
    };
    let find_pair = |name: &str| v.signal.pairs.iter().find(|p| p.name.eq_ignore_ascii_case(name) || p.p == name || p.n == name).cloned();
    let pair = t.pair.as_deref().map(|n| find_pair(n).ok_or_else(|| format!("no differential pair {n} (pairs: {})", v.signal.pairs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))).transpose()?;
    let mut skew = None;
    if let Some(p) = pair {
        let (lp, ln) = (w.net_length(&p.p), w.net_length(&p.n));
        if mode == TuneMode::Length {
            let target = t.target.or_else(|| v.signal.groups.iter().find(|g| g.members.iter().any(|m| m.net == p.p)).map(|g| g.target)).ok_or("give the pair a target length (or put it in a length group)")?;
            let want = target - lp.max(ln);
            if want > opts.tol {
                w.pair(&p, want, &opts);
            } else if want < -opts.tol {
                w.notes.push(format!("pair {} is already {:.3} mm longer than {:.3} mm", p.name, -want, target));
            }
        }
        // The skew: the shorter track meandered on the side away from its partner.
        let (lp2, ln2) = (w.net_length(&p.p), w.net_length(&p.n));
        let (short, long, other) = if lp2 < ln2 { (&p.p, ln2, &p.n) } else { (&p.n, lp2, &p.p) };
        let keep_only = w.only.take();
        let mut skew_opts = Opts { amplitude: t.amplitude.or(Some(((p.gap.unwrap_or(0.15) + 0.2) * 1.5).min(1.0))), ..opts };
        skew_opts.style = Style::Trombone;
        if (lp2 - ln2).abs() > opts.tol {
            let before = w.net_length(short);
            w.single(short, long - before, &skew_opts, Some(other));
        }
        w.only = keep_only;
        let (lp3, ln3) = (w.net_length(&p.p), w.net_length(&p.n));
        nets.push(NetTune { net: p.p.clone(), before: lp, after: lp3, target: lp3.max(ln3) });
        nets.push(NetTune { net: p.n.clone(), before: ln, after: ln3, target: lp3.max(ln3) });
        skew = Some((lp3 - ln3).abs());
    } else if let Some(gname) = &t.group {
        let g = v.signal.groups.iter().find(|g| g.name.eq_ignore_ascii_case(gname)).ok_or_else(|| format!("no length group {gname}"))?;
        let target = t.target.unwrap_or(g.target);
        let tol = g.tolerance.map(|x| x.min(opts.tol.max(x / 4.0))).unwrap_or(opts.tol);
        let gopts = Opts { tol, ..opts };
        let mut done: Vec<String> = Vec::new();
        for m in &g.members {
            if done.contains(&m.net) {
                continue;
            }
            if let Some(p) = v.signal.pairs.iter().find(|p| (p.p == m.net || p.n == m.net) && g.members.iter().any(|x| x.net == p.p) && g.members.iter().any(|x| x.net == p.n)) {
                let sub = Tune { pair: Some(p.name.clone()), group: None, target: Some(target), ..t.clone() };
                let r = plan(b, v, &sub)?;
                w.notes.extend(r.notes);
                for e in r.edits {
                    w.commit(e.id, e.points, e.bulges);
                }
                nets.extend(r.nets);
                done.push(p.p.clone());
                done.push(p.n.clone());
                continue;
            }
            let before = w.net_length(&m.net);
            let want = target - before;
            if want > gopts.tol {
                w.single(&m.net, want, &gopts, None);
            } else if want < -gopts.tol {
                w.notes.push(format!("{} is {:.3} mm, already longer than the target {:.3} mm", m.net, before, target));
            }
            nets.push(NetTune { net: m.net.clone(), before, after: w.net_length(&m.net), target });
            done.push(m.net.clone());
        }
    } else {
        let net = match (&t.net, t.track) {
            (Some(n), _) => n.clone(),
            (None, Some(id)) => v.tracks.iter().find(|x| x.id == id).and_then(|x| x.net.clone()).ok_or("the track has no net")?,
            _ => return Err("say what to tune: net, pair, group or track".into()),
        };
        if !v.nets.iter().any(|n| n.name == net) {
            return Err(format!("no net {net} on this board"));
        }
        if let Some(p) = find_pair(&net).filter(|_| mode == TuneMode::Skew) {
            return plan(b, v, &Tune { pair: Some(p.name), net: None, ..t.clone() });
        }
        let target = t.target.or_else(|| v.signal.groups.iter().find(|g| g.members.iter().any(|m| m.net == net)).map(|g| g.target)).ok_or("give a target length (or put the net in a length group)")?;
        tune_net(&mut w, &net, target, None);
    }
    let edits: Vec<Edit> = w.geo.iter().map(|(id, (p, bl))| Edit { id: *id, points: p.clone(), bulges: if bl.iter().all(|x| x.abs() < 1e-12) { vec![] } else { bl.clone() } }).collect();
    let tol = opts.tol.max(t.tolerance.unwrap_or(0.0));
    let ok = nets.iter().all(|n| (n.after - n.target).abs() <= tol + 1e-6 || n.after > n.target) && skew.is_none_or(|s| s <= tol + 1e-6);
    let r6 = |x: f64| (x * 1e6).round() / 1e6;
    let nets = nets.into_iter().map(|n| NetTune { before: r6(n.before), after: r6(n.after), target: r6(n.target), ..n }).collect();
    Ok(TunePlan { ok, edits, nets, skew: skew.map(r6), notes: w.notes })
}

impl Command for Tune {
    const NAME: &'static str = "pcb.tune";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let b = board_of(tx, self.board)?;
        let v = board::resolve(tx.world(), self.board, &b).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        let p = plan(&b, &v, &self).map_err(bad)?;
        if p.edits.is_empty() && !p.ok {
            return Err(bad(format!("nothing tuned: {}", p.notes.join("; "))));
        }
        for e in &p.edits {
            let mut t = tx.get::<Track>(e.id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a track", e.id)))?;
            t.points = e.points.clone();
            t.bulges = e.bulges.clone();
            tx.set(e.id, t)?;
        }
        Ok(json!(p))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<Tune>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(corner: Corner) -> Shape {
        Shape { style: Style::Trombone, corner, spacing: 0.6, radius: 0.3, min_radius: 0.0 }
    }

    #[test]
    fn trombone_adds_what_the_formula_says() {
        // Round corners of radius r: each period adds 2A − (8 − 2π)r.
        let s = sh(Corner::Round);
        let (a, r) = (1.5, 0.3);
        let want = 2.0 * a - (8.0 - 2.0 * std::f64::consts::PI) * r;
        assert!((added(&s, 1, a) - want).abs() < 1e-9, "{} vs {want}", added(&s, 1, a));
        assert!((added(&s, 5, a) - 5.0 * want).abs() < 1e-9);
        // Square corners add exactly 2A per period.
        assert!((added(&sh(Corner::Square), 3, a) - 6.0 * a).abs() < 1e-9);
        // The realized polyline (arcs measured as arcs) is as long as the path says.
        let steps = pattern(&s, 4, a, 1.0);
        let (p, b) = realize(&steps, [1.0, 2.0], 0.7, 0.0);
        assert!((signal::path_length(&p, &b) - length(&steps, 0.0)).abs() < 1e-9);
        // It ends on the baseline, 4 periods along.
        let end = *p.last().unwrap();
        let along = [1.0 + 4.0 * 1.2 * 0.7f64.cos(), 2.0 + 4.0 * 1.2 * 0.7f64.sin()];
        assert!(geom::dist(end, along) < 1e-9, "{end:?} vs {along:?}");
    }

    #[test]
    fn amplitude_is_solved_to_the_micron() {
        for c in [Corner::Round, Corner::Chamfer, Corner::Square] {
            let s = sh(c);
            for want in [0.05, 0.4, 2.3, 7.77] {
                let n = (1..20).find(|n| added(&s, *n, 2.0) >= want).unwrap();
                let a = amplitude_for(&s, n, want, 1e-4, 2.0).unwrap();
                assert!((added(&s, n, a) - want).abs() < 1e-9, "{c:?} {want}: {}", added(&s, n, a));
            }
        }
        let saw = Shape { style: Style::Sawtooth, ..sh(Corner::Square) };
        let a = amplitude_for(&saw, 3, 1.0, 1e-4, 2.0).unwrap();
        assert!((added(&saw, 3, a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_pair_meander_keeps_both_tracks_equal_and_apart() {
        // Pitch 0.35 mm (0.2 wide, 0.15 gap): offsets ±0.175 from the centreline.
        let half = 0.175;
        for corner in [Corner::Round, Corner::Chamfer, Corner::Square] {
            let s = Shape { style: Style::Trombone, corner, spacing: 0.35 + 0.6, radius: 0.475, min_radius: half };
            let steps = pattern(&s, 3, 1.4, 1.0);
            let (a, b) = (length(&steps, half), length(&steps, -half));
            assert!((a - b).abs() < 1e-9, "{corner:?}: {a} vs {b}");
            assert!((a - length(&steps, 0.0)).abs() < 1e-9, "the centreline's length too");
            let (pa, ba) = realize(&steps, [0.0, 0.0], 0.0, half);
            let (pb, bb) = realize(&steps, [0.0, 0.0], 0.0, -half);
            // Spliced into each member's straight run, both tracks gain the same.
            let run = |y: f64| vec![[-3.0, y], [3.0 * period(&s) + 3.0, y]];
            let (ta, tab) = splice(&run(half), &[], 0, &pa, &ba);
            let (tb, tbb) = splice(&run(-half), &[], 0, &pb, &bb);
            let (ga, gb) = (signal::path_length(&ta, &tab), signal::path_length(&tb, &tbb));
            assert!((ga - gb).abs() < 1e-9, "{corner:?}: {ga} vs {gb}");
            assert!((ga - (3.0 * period(&s) + 6.0) - added(&s, 3, 1.4)).abs() < 1e-9, "{corner:?}: gains what the path says");
            // The two stay one pitch apart everywhere (sampled on the flattened lines).
            let (fa, fb) = (pns::flatten(&ta, &tab), pns::flatten(&tb, &tbb));
            for q in fa.iter().step_by(3) {
                let d = fb.windows(2).map(|w| geom::point_seg_dist(*q, w[0], w[1])).fold(f64::INFINITY, f64::min);
                // Never closer than the pitch (a mitred corner sits further, up to √2 at a square one).
                assert!(d > 2.0 * half - 0.005 && d < 2.0 * half * std::f64::consts::SQRT_2 + 0.005, "{corner:?}: {d}");
            }
        }
    }

    #[test]
    fn splice_keeps_the_track_whole() {
        let pts = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0]];
        let s = sh(Corner::Round);
        let steps = pattern(&s, 2, 1.0, 1.0);
        let (mp, mb) = realize(&steps, [2.0, 0.0], 0.0, 0.0);
        let (np, nb) = splice(&pts, &[], 0, &mp, &mb);
        assert_eq!(np.first(), Some(&[0.0, 0.0]));
        assert_eq!(np.last(), Some(&[10.0, 5.0]));
        assert_eq!(nb.len(), np.len() - 1);
        let gained = signal::path_length(&np, &nb) - 15.0;
        assert!((gained - added(&s, 2, 1.0)).abs() < 1e-9);
        // Spliced against the segment's direction it is reversed (and its arcs turned round).
        let (np2, nb2) = splice(&[[10.0, 0.0], [0.0, 0.0]], &[], 0, &mp, &mb);
        assert!((signal::path_length(&np2, &nb2) - 10.0 - added(&s, 2, 1.0)).abs() < 1e-9);
        assert!(geom::dist(np2[1], *mp.last().unwrap()) < 1e-9);
    }
}
