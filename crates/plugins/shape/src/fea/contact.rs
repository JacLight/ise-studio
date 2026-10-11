//! Contact between parts studied together.
//!
//! Each part has its own nodes on the shared grid, so two parts that touch
//! are joined only where this module joins them. Where a cube of one part
//! faces a cube of another (side by side, or across one empty cube), their
//! facing corners are paired; each pair has a normal (from the first part
//! into the second, averaged over the cube faces it serves), the share of
//! touching area it stands for, and its starting gap, measured from how full
//! the cubes either side are (so a contact plane through the middle of a
//! cube still starts closed).
//!
//! A pair in contact is a stiff spring along its normal (the penalty), with
//! the force it already carries kept as a multiplier and refined each round
//! (augmented Lagrange), so the parts end up neither overlapping nor pulled
//! together by the spring: the force is right however stiff the spring.
//! Contact only pushes: a pair whose force would pull opens, and a pair that
//! overlaps closes, and the rounds go on until no pair changes and the
//! forces settle. With friction, a closed pair also grips sideways up to
//! μ × its push, then slips with that force. Bonded pairs are tied in every
//! direction and never open.
//!
//! The springs go into the stiffness, and the multigrid carries them down to
//! its coarse grids (see `multigrid`), so the solve stays quick even when one
//! part is held only through another.

use super::{ContactKind, ContactSpec, Elastic, Model, Spring};
use serde::Serialize;
use std::collections::HashMap;

/// A contact spring's stiffness per touching area, in units of E/h. Stiff
/// enough that each round cuts the remaining overlap several times over,
/// soft enough to keep the equations well conditioned.
const PENALTY: f64 = 20.0;
/// Rounds of opening, closing and refining the forces.
const ROUNDS: usize = 60;
/// The forces have settled when no pair changes by more than this share of the largest.
const SETTLED: f64 = 1e-4;

/// Two facing nodes of different parts.
pub(super) struct Pair {
    /// The parts, and the node of each.
    p: usize,
    q: usize,
    a: usize,
    b: usize,
    /// From part p into part q.
    n: [f64; 3],
    /// Gap before loading (mm).
    g0: f64,
    /// Touching area it stands for (mm²).
    area: f64,
    /// Penalty spring (N/mm).
    k: f64,
    kind: ContactKind,
    mu: f64,
    /// A press fit: the overlap as drawn is squeezed out (g0 < 0).
    press: bool,
}

/// A cube face where two parts meet: the cube on the first part's side, which
/// way the face looks (0 −x, 1 +x, 2 −y, 3 +y, 4 −z, 5 +z), and its corner pairs.
struct Meeting {
    cell: [usize; 3],
    dir: u8,
    pairs: [usize; 4],
}

pub(super) struct Found {
    pairs: Vec<Pair>,
    meetings: Vec<Meeting>,
    /// Some parts overlap rather than touch.
    pub overlap: bool,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Which parts are held: by a support of their own (`held`), or glued
    /// by a bonded contact to a part that is.
    pub fn anchored(&self, mut held: Vec<bool>) -> Vec<bool> {
        loop {
            let mut more = false;
            for pr in self.pairs.iter().filter(|pr| pr.kind == ContactKind::Bonded) {
                if held[pr.p] != held[pr.q] {
                    held[pr.p] = true;
                    held[pr.q] = true;
                    more = true;
                }
            }
            if !more {
                return held;
            }
        }
    }
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn norm3(v: &[f64]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The pairs between every two parts whose surfaces face each other across
/// at most about one cube (`fills`: how full of each part every cube is).
///
/// For a press fit the gap is measured on the parts' own surfaces instead
/// (`meshes`): at each pair, the signed distances to the two surfaces add
/// up to the gap, negative where they overlap (the interference), and the
/// pair pushes along the surfaces' true normal, standing for the true area
/// of surface its staircase of cube faces covers.
pub(super) fn find(m: &Model, fills: &[Vec<f32>], materials: &[Elastic], names: &[String], specs: &[ContactSpec], meshes: &[(&[f32], &[u32])]) -> Found {
    let grid = &m.grid;
    let (h, n) = (grid.h, grid.n);
    let mut owner = vec![usize::MAX; n[0] * n[1] * n[2]];
    for (c, &p) in m.cells.iter().zip(&m.part) {
        owner[grid.cell(c[0], c[1], c[2])] = p;
    }
    let index = |c: [i64; 3]| -> Option<usize> { (0..3).all(|d| c[d] >= 0 && (c[d] as usize) < n[d]).then(|| grid.cell(c[0] as usize, c[1] as usize, c[2] as usize)) };
    let fill = |p: usize, c: [i64; 3]| index(c).map_or(0.0, |i| fills[p][i] as f64);
    let kind_of = |p: usize, q: usize| -> (ContactKind, f64, bool) {
        specs
            .iter()
            .find(|s| (s.parts[0] == names[p] && s.parts[1] == names[q]) || (s.parts[0] == names[q] && s.parts[1] == names[p]))
            .map_or((ContactKind::Frictionless, 0.0, false), |s| (s.kind, if s.kind == ContactKind::Friction { s.friction.max(0.0) } else { 0.0 }, s.press_fit && s.kind != ContactKind::Bonded))
    };
    struct Acc {
        p: usize,
        q: usize,
        a: usize,
        b: usize,
        area: f64,
        normal: [f64; 3],
        gap: f64,
        /// Cube-face area along each axis.
        dirs: [f64; 3],
    }
    let mut slot: HashMap<(usize, usize), usize> = HashMap::new();
    let mut acc: Vec<Acc> = vec![];
    let mut meetings = vec![];
    let mut overlap = false;
    let quarter = h * h / 4.0;
    for (&c, &p) in m.cells.iter().zip(&m.part) {
        for dir in 0..6u8 {
            let d = (dir / 2) as usize;
            let s: i64 = if dir % 2 == 0 { -1 } else { 1 };
            let step = |k: i64| -> [i64; 3] {
                let mut v = c.map(|x| x as i64);
                v[d] += s * k;
                v
            };
            let Some(i1) = index(step(1)) else { continue };
            // The other part's cube: next door, or across one empty cube.
            let (q, far) = if owner[i1] != usize::MAX {
                (owner[i1], 1)
            } else {
                match index(step(2)) {
                    Some(i2) if owner[i2] != usize::MAX => (owner[i2], 2),
                    _ => continue,
                }
            };
            if q <= p {
                continue; // the same part, or met from the other side
            }
            // Each part's surface along this line, from how full the cubes are.
            let c0 = step(0);
            let gap = if far == 1 {
                h * (2.0 - fill(p, c0) - fill(p, step(1)) - fill(q, step(1)) - fill(q, c0))
            } else {
                h * (3.0 - fill(p, c0) - fill(p, step(1)) - fill(q, step(2)) - fill(q, step(1)))
            };
            if gap < -0.25 * h && !kind_of(p, q).2 {
                overlap = true;
            }
            if gap > h {
                continue;
            }
            // The face's four corners on p, and the facing ones on q.
            let mut ids = [0usize; 4];
            let others: Vec<usize> = (0..3).filter(|&x| x != d).collect();
            for (corner, id) in ids.iter_mut().enumerate() {
                let mut g = c;
                if s > 0 {
                    g[d] += 1;
                }
                g[others[0]] += corner & 1;
                g[others[1]] += corner >> 1;
                let mut gq = g;
                if far == 2 {
                    gq[d] = (gq[d] as i64 + s) as usize;
                }
                let (a, b) = (m.blocks[p].node(g), m.blocks[q].node(gq));
                let i = *slot.entry((a, b)).or_insert_with(|| {
                    acc.push(Acc { p, q, a, b, area: 0.0, normal: [0.0; 3], gap: 0.0, dirs: [0.0; 3] });
                    acc.len() - 1
                });
                let e = &mut acc[i];
                e.area += quarter;
                e.normal[d] += s as f64 * quarter;
                e.gap += gap.max(0.0) * quarter;
                e.dirs[d] += quarter;
                *id = i;
            }
            meetings.push(Meeting { cell: c, dir, pairs: ids });
        }
    }
    // Press fits: the gap, normal and area from the surfaces themselves, at each pair's midpoint.
    let mut surfaces: HashMap<usize, (f64, [f64; 3])> = HashMap::new();
    let parts = meshes.len();
    for p in 0..parts {
        for q in p + 1..parts {
            if !kind_of(p, q).2 {
                continue;
            }
            let which: Vec<usize> = (0..acc.len()).filter(|&i| acc[i].p == p && acc[i].q == q).collect();
            let mid: Vec<[f64; 3]> = which.iter().map(|&i| {
                let (a, b) = (m.position(acc[i].a), m.position(acc[i].b));
                [0, 1, 2].map(|d| (a[d] + b[d]) / 2.0)
            }).collect();
            let on_p = super::surface_near(&mid, meshes[p].0, meshes[p].1, 3.0 * h);
            let on_q = super::surface_near(&mid, meshes[q].0, meshes[q].1, 3.0 * h);
            for (k, &i) in which.iter().enumerate() {
                let (sp, _, np) = on_p[k];
                let (sq, _, nq) = on_q[k];
                if !(sp.is_finite() && sq.is_finite()) {
                    continue;
                }
                // Out of p is into q: the two outward normals, one turned round, averaged.
                let n = [0, 1, 2].map(|d| np[d] - nq[d]);
                let l = norm(n);
                if l > 1e-9 {
                    surfaces.insert(i, (sp + sq, n.map(|v| v / l)));
                }
            }
        }
    }
    let pairs = acc
        .into_iter()
        .enumerate()
        .map(|(i, e)| {
            let len = norm(e.normal);
            // Faces facing opposite ways cancel (a sliver between two faces): keep the first's axis.
            let n = if len > 1e-9 * e.area { e.normal.map(|v| v / len) } else { [0.0, 0.0, 1.0] };
            let (ep, eq) = (materials[e.p].e, materials[e.q].e);
            let stiff = 2.0 * ep * eq / (ep + eq);
            let (kind, mu, press) = kind_of(e.p, e.q);
            let (g0, n, area) = match surfaces.get(&i) {
                // The staircase covers the surface's area once per axis, by the normal's share along it.
                Some(&(g, n)) => (g, n, (0..3).map(|d| e.dirs[d] * n[d].abs()).sum::<f64>().max(1e-3 * e.area)),
                None => (e.gap / e.area, n, e.area),
            };
            Pair { p: e.p, q: e.q, a: e.a, b: e.b, n, g0, area, k: PENALTY * stiff * area / h, kind, mu, press: press && surfaces.contains_key(&i) }
        })
        .collect();
    Found { pairs, meetings, overlap }
}

/// Whether a pair pushes, and with friction whether it grips or slips.
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Open,
    Closed,
    Slip,
}

pub(super) struct Outcome {
    pub rounds: usize,
    pub iterations: usize,
    /// Every linear solve settled.
    pub converged: bool,
    /// The contact rounds settled.
    pub settled: bool,
    /// Per pair: the push (N; negative: a bonded pair pulling), the sideways
    /// force on the second part (N), the gap at the end (mm), and its state.
    normal: Vec<f64>,
    shear: Vec<[f64; 3]>,
    gap: Vec<f64>,
    state: Vec<State>,
}

/// Two unit vectors square to `n` and to each other.
fn tangents(n: [f64; 3]) -> [[f64; 3]; 2] {
    let a = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let cross = |u: [f64; 3], v: [f64; 3]| [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let t = cross(a, n);
    let l = norm(t);
    let t1 = t.map(|v| v / l);
    [t1, cross(n, t1)]
}

/// A spring of stiffness k on the pair's relative movement along `dir`.
fn link(pr: &Pair, dir: [f64; 3], k: f64) -> Spring {
    let mut g = Vec::with_capacity(6);
    for d in 0..3 {
        if dir[d] != 0.0 {
            g.push((3 * pr.b + d, dir[d]));
            g.push((3 * pr.a + d, -dir[d]));
        }
    }
    Spring { k, g }
}

/// A slipping pair's way of slipping (its grip, made unit), and square to it in the contact plane.
fn slip_axes(pr: &Pair, grip: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let l = norm(grip).max(1e-300);
    let e = grip.map(|v| v / l);
    let n = pr.n;
    (e, [n[1] * e[2] - n[2] * e[1], n[2] * e[0] - n[0] * e[2], n[0] * e[1] - n[1] * e[0]])
}

fn springs(pairs: &[Pair], state: &[State], grip: &[[f64; 3]]) -> Vec<Spring> {
    let mut out = vec![];
    for ((pr, &st), &g) in pairs.iter().zip(state).zip(grip) {
        if st == State::Open {
            continue;
        }
        match pr.kind {
            ContactKind::Bonded => out.extend([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].map(|e| link(pr, e, pr.k))),
            ContactKind::Frictionless => out.push(link(pr, pr.n, pr.k)),
            ContactKind::Friction => {
                // Gripping: held sideways by a spring. Slipping: free the way it
                // slips (the force there is μ × the push), held square to it.
                out.push(link(pr, pr.n, pr.k));
                if pr.mu > 0.0 {
                    if st == State::Slip {
                        out.push(link(pr, slip_axes(pr, g).1, pr.k));
                    } else {
                        out.extend(tangents(pr.n).map(|t| link(pr, t, pr.k)));
                    }
                }
            }
        }
    }
    out
}

/// Solves with contact from `u`: rounds of a linear solve with the closed
/// pairs' springs and forces, then each pair's force refined and its state
/// updated, until nothing changes.
pub(super) fn solve(m: &mut Model, f: &[f64], u: &mut [f64], found: &Found) -> Outcome {
    let pairs = &found.pairs;
    let h = m.grid.h;
    let mut lam = vec![0.0; pairs.len()];
    // Sideways grip (friction), or the whole tie force (bonded), on part q.
    let mut tie = vec![[0.0; 3]; pairs.len()];
    // With friction: how far the pair has slipped (the anchor of its sideways spring).
    let mut slid = vec![[0.0; 3]; pairs.len()];
    // Touching pairs start closed; bonded ones stay so.
    let mut state: Vec<State> = pairs.iter().map(|pr| if pr.kind == ContactKind::Bonded || pr.g0 <= 1e-6 * h { State::Closed } else { State::Open }).collect();
    let mut shown: Option<Vec<(State, [f64; 3])>> = None;
    let (mut iterations, mut converged, mut settled, mut rounds) = (0, true, false, 0);
    let mut gap = vec![0.0; pairs.len()];
    let mut slid_before = false;
    // The load (and the squeeze of press fits), to tell a part drifting off on its weak tie.
    let squeeze: f64 = pairs.iter().map(|pr| pr.k * (-pr.g0).max(0.0)).sum();
    let applied: f64 = (f.chunks_exact(3).map(norm3).sum::<f64>() + squeeze).max(1e-12);
    while rounds < ROUNDS {
        rounds += 1;
        // The springs change when a pair opens, closes, or slips another way.
        let key: Vec<(State, [f64; 3])> = state.iter().zip(&tie).map(|(&s, &t)| (s, if s == State::Slip { slip_axes(&pairs[0], t).0 } else { [0.0; 3] })).collect();
        if shown.as_ref() != Some(&key) {
            m.set_springs(springs(pairs, &state, &tie));
            shown = Some(key);
        }
        let mut rhs = f.to_vec();
        for (i, pr) in pairs.iter().enumerate() {
            if state[i] == State::Open {
                continue;
            }
            let sideways: [f64; 3] = if state[i] == State::Slip {
                let (e, square) = slip_axes(pr, tie[i]);
                let along = dot(slid[i], square);
                std::array::from_fn(|d| pr.mu * lam[i] * e[d] + pr.k * along * square[d])
            } else {
                slid[i].map(|v| pr.k * v)
            };
            let force: [f64; 3] = match pr.kind {
                ContactKind::Bonded => tie[i],
                _ => std::array::from_fn(|d| (lam[i] - pr.k * pr.g0) * pr.n[d] + sideways[d]),
            };
            for d in 0..3 {
                rhs[3 * pr.b + d] += force[d];
                rhs[3 * pr.a + d] -= force[d];
            }
        }
        let (it, ok) = m.cg(&rhs, u, 1e-9);
        iterations += it;
        converged &= ok;
        let mut changed = false;
        let (mut moved, mut biggest) = (0.0f64, 0.0f64);
        for (i, pr) in pairs.iter().enumerate() {
            let w: [f64; 3] = std::array::from_fn(|d| u[3 * pr.b + d] - u[3 * pr.a + d]);
            let dn = dot(w, pr.n);
            gap[i] = pr.g0 + dn;
            if pr.kind == ContactKind::Bonded {
                let next: [f64; 3] = std::array::from_fn(|d| tie[i][d] - pr.k * w[d]);
                moved = moved.max(norm(std::array::from_fn(|d| next[d] - tie[i][d])));
                biggest = biggest.max(norm(next));
                tie[i] = next;
                continue;
            }
            let push = lam[i] - pr.k * gap[i];
            let wt: [f64; 3] = std::array::from_fn(|d| w[d] - dn * pr.n[d]);
            let gripping = pr.kind == ContactKind::Friction && pr.mu > 0.0;
            let (next_lam, next_tie, next_state) = if push > 0.0 {
                if gripping {
                    // The sideways spring's pull; past μ × the push it slips, its anchor following.
                    let trial: [f64; 3] = std::array::from_fn(|d| -pr.k * (wt[d] - slid[i][d]));
                    let (size, cap) = (norm(trial), pr.mu * push);
                    if size <= cap {
                        (push, trial, State::Closed)
                    } else {
                        let grip = trial.map(|v| v * cap / size);
                        slid[i] = std::array::from_fn(|d| wt[d] + grip[d] / pr.k);
                        (push, grip, State::Slip)
                    }
                } else {
                    (push, [0.0; 3], State::Closed)
                }
            } else {
                // Apart: should it touch again, it grips from where it is then.
                if gripping {
                    slid[i] = wt;
                }
                (0.0, [0.0; 3], State::Open)
            };
            moved = moved.max((next_lam - lam[i]).abs() + norm(std::array::from_fn(|d| next_tie[d] - tie[i][d])));
            biggest = biggest.max(next_lam + norm(next_tie));
            changed |= next_state != state[i];
            lam[i] = next_lam;
            tie[i] = next_tie;
            state[i] = next_state;
        }
        if !changed && moved <= SETTLED * biggest {
            settled = true;
            break;
        }
        // A part held by nothing but contacts that slip everywhere, or that
        // let it go, has no resting place: it slides or lifts away.
        let parts = m.blocks.len();
        let sliding = slips(pairs, &state, parts).iter().enumerate().any(|(p, &s)| s && m.floating[p].is_some()) || (0..parts).any(|p| m.tie_pull(p, u) > super::DRIFT * applied);
        if sliding && slid_before {
            settled = true;
            break;
        }
        slid_before = sliding;
    }
    let normal = pairs.iter().enumerate().map(|(i, pr)| if pr.kind == ContactKind::Bonded { dot(tie[i], pr.n) } else { lam[i] }).collect();
    let shear = pairs
        .iter()
        .enumerate()
        .map(|(i, pr)| {
            let t = tie[i];
            let along = dot(t, pr.n);
            std::array::from_fn(|d| t[d] - along * pr.n[d])
        })
        .collect();
    Outcome { rounds, iterations, converged, settled, normal, shear, gap, state }
}

/// Per part: everywhere it presses on another part it slips (so, held by
/// nothing else, it slides away).
pub(super) fn slipping(found: &Found, out: &Outcome, parts: usize) -> Vec<bool> {
    slips(&found.pairs, &out.state, parts)
}

fn slips(pairs: &[Pair], state: &[State], parts: usize) -> Vec<bool> {
    let mut slips = vec![false; parts];
    let mut grips = vec![false; parts];
    for (pr, &st) in pairs.iter().zip(state) {
        for p in [pr.p, pr.q] {
            match st {
                State::Slip => slips[p] = true,
                State::Closed => grips[p] = true,
                State::Open => {}
            }
        }
    }
    (0..parts).map(|p| slips[p] && !grips[p]).collect()
}

/// Contact pressure on one cube face where two parts meet.
#[derive(Clone, Debug, Serialize)]
pub struct ContactFace {
    /// The cube on the first part's side, and which way the face looks
    /// (0 −x, 1 +x, 2 −y, 3 +y, 4 −z, 5 +z).
    pub cell: [u32; 3],
    pub dir: u8,
    /// MPa: positive pushing, negative a bonded face pulling.
    pub pressure: f32,
    /// Index into `Contacts::pairs`.
    pub pair: u16,
}

/// What passes between two parts.
#[derive(Clone, Debug, Serialize)]
pub struct ContactPair {
    pub parts: [String; 2],
    pub kind: ContactKind,
    pub friction: f64,
    /// Facing area within reach (mm²), and how much of it presses at the end.
    pub area: f64,
    pub pressing: f64,
    /// Of the pressing area, how much slips (friction).
    pub slipping: f64,
    /// The total push between them (N), and the sideways force (N).
    pub normal_force: f64,
    pub shear_force: f64,
    pub max_pressure: f64,
    /// A press fit: the interference as drawn (mm: the overlap of the
    /// surfaces, averaged over where they overlap) and the mean pressure
    /// over the pressing area it gives (MPa).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub press_fit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interference: Option<f64>,
    pub mean_pressure: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Contacts {
    pub faces: Vec<ContactFace>,
    /// Where touching faces came apart: x, y, z (mm) and the gap opened (mm).
    pub opened: Vec<[f32; 4]>,
    pub max_pressure: f64,
    pub pairs: Vec<ContactPair>,
    pub rounds: usize,
    pub settled: bool,
}

pub(super) fn report(m: &Model, found: &Found, out: &Outcome, names: &[String]) -> Contacts {
    let pairs = &found.pairs;
    let h = m.grid.h;
    let mut which: HashMap<(usize, usize), usize> = HashMap::new();
    let mut summary: Vec<ContactPair> = vec![];
    let mut shear: Vec<[f64; 3]> = vec![];
    // Per pair of parts: overlap × area, and the area overlapping (press fits).
    let mut overlap: Vec<(f64, f64)> = vec![];
    for (i, pr) in pairs.iter().enumerate() {
        let s = *which.entry((pr.p, pr.q)).or_insert_with(|| {
            summary.push(ContactPair { parts: [names[pr.p].clone(), names[pr.q].clone()], kind: pr.kind, friction: pr.mu, area: 0.0, pressing: 0.0, slipping: 0.0, normal_force: 0.0, shear_force: 0.0, max_pressure: 0.0, press_fit: false, interference: None, mean_pressure: 0.0 });
            shear.push([0.0; 3]);
            overlap.push((0.0, 0.0));
            summary.len() - 1
        });
        let e = &mut summary[s];
        e.area += pr.area;
        if out.state[i] != State::Open {
            e.pressing += pr.area;
        }
        if out.state[i] == State::Slip {
            e.slipping += pr.area;
        }
        e.normal_force += out.normal[i];
        for d in 0..3 {
            shear[s][d] += out.shear[i][d];
        }
        if pr.press {
            e.press_fit = true;
            if pr.g0 < 0.0 {
                overlap[s].0 += -pr.g0 * pr.area;
                overlap[s].1 += pr.area;
            }
        }
    }
    for ((e, t), o) in summary.iter_mut().zip(&shear).zip(&overlap) {
        e.shear_force = norm(*t);
        e.mean_pressure = if e.pressing > 0.0 { e.normal_force / e.pressing } else { 0.0 };
        if e.press_fit {
            e.interference = Some(if o.1 > 0.0 { o.0 / o.1 } else { 0.0 });
        }
    }
    let mut faces = Vec::with_capacity(found.meetings.len());
    let mut max_pressure = 0.0f64;
    for f in &found.meetings {
        let pressure = f.pairs.iter().map(|&i| out.normal[i] / pairs[i].area).sum::<f64>() / 4.0;
        let s = which[&(pairs[f.pairs[0]].p, pairs[f.pairs[0]].q)];
        summary[s].max_pressure = summary[s].max_pressure.max(pressure);
        max_pressure = max_pressure.max(pressure);
        faces.push(ContactFace { cell: f.cell.map(|v| v as u32), dir: f.dir, pressure: pressure as f32, pair: s as u16 });
    }
    // Touching at the start, apart at the end.
    let opened: Vec<[f32; 4]> = pairs
        .iter()
        .enumerate()
        .filter(|&(i, pr)| pr.g0 <= 0.05 * h && out.state[i] == State::Open && out.gap[i] > 0.0)
        .map(|(i, pr)| {
            let (a, b) = (m.position(pr.a), m.position(pr.b));
            [((a[0] + b[0]) / 2.0) as f32, ((a[1] + b[1]) / 2.0) as f32, ((a[2] + b[2]) / 2.0) as f32, out.gap[i] as f32]
        })
        .collect();
    // Plenty to show where it opened, without flooding the view.
    let step = opened.len().div_ceil(4000).max(1);
    let opened = opened.into_iter().step_by(step).collect();
    Contacts { faces, opened, max_pressure, pairs: summary, rounds: out.rounds, settled: out.settled }
}
