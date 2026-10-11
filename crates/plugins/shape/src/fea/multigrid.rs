//! Multigrid for the voxel equations, used to precondition conjugate
//! gradients.
//!
//! Plain Jacobi-CG needs thousands of steps on thin parts: a plate bends
//! easily but each cube is stiff, and local smoothing passes that bending
//! along one cube per step. Multigrid fixes the long-range part on coarser
//! grids. Each level has cubes twice the size of the one below, built
//! exactly from it (Galerkin: Pᵀ·K·P, with trilinear interpolation P), so
//! partly filled cubes, held nodes and thin walls carry down faithfully.
//! Damped Jacobi smooths on every level; the coarsest is solved directly.
//! One V-cycle with equal smoothing before and after is symmetric, as
//! conjugate gradients needs.

use super::{CORNERS, Model};
use rayon::prelude::*;
use std::collections::HashMap;

/// Jacobi damping and sweeps before and after each coarse correction.
const OMEGA: f64 = 0.6;
const SWEEPS: usize = 2;
/// Unknowns at or below which a level is solved directly.
const DIRECT: usize = 1800;
const MAX_LEVELS: usize = 10;

fn offset(a: usize) -> [usize; 3] {
    CORNERS[a].map(|c| (c > 0.0) as usize)
}

fn node_id(n: [usize; 3], p: [usize; 3]) -> usize {
    p[0] + (n[0] + 1) * (p[1] + (n[1] + 1) * p[2])
}

fn coarser(n: [usize; 3]) -> [usize; 3] {
    n.map(|v| v.div_ceil(2))
}

/// Interpolation from a coarse cube's 8 corners to the corners of its child
/// at `o` (0 or 1 along each axis): `s[a][b]` is coarse corner b's share in
/// child corner a.
fn child_interp(o: [usize; 3]) -> [[f64; 8]; 8] {
    let mut s = [[0.0; 8]; 8];
    for (a, row) in s.iter_mut().enumerate() {
        let ca = offset(a);
        for (b, v) in row.iter_mut().enumerate() {
            let cb = offset(b);
            *v = (0..3)
                .map(|d| match (o[d] + ca[d], cb[d]) {
                    (0, 0) | (2, 1) => 1.0,
                    (1, _) => 0.5,
                    _ => 0.0,
                })
                .product();
        }
    }
    s
}

/// out += (S⊗I₃)ᵀ · K · (S⊗I₃), all 24 × 24 row-major.
fn galerkin(k: &[f64], s: &[[f64; 8]; 8], out: &mut [f64]) {
    let mut x = [0.0f64; 576];
    for i in 0..24 {
        for b in 0..8 {
            for d in 0..3 {
                let mut v = 0.0;
                for (a, row) in s.iter().enumerate() {
                    if row[b] != 0.0 {
                        v += k[i * 24 + 3 * a + d] * row[b];
                    }
                }
                x[i * 24 + 3 * b + d] = v;
            }
        }
    }
    for b in 0..8 {
        for d in 0..3 {
            let r = 3 * b + d;
            for j in 0..24 {
                let mut v = 0.0;
                for (a, row) in s.iter().enumerate() {
                    if row[b] != 0.0 {
                        v += row[b] * x[(3 * a + d) * 24 + j];
                    }
                }
                out[r * 24 + j] += v;
            }
        }
    }
}

/// One coarse level: its cubes, each with a 24 × 24 stiffness.
struct Level {
    n: [usize; 3],
    nodes: usize,
    cells: Vec<[usize; 3]>,
    corners: Vec<[usize; 8]>,
    /// First coarse level only: how much material each of a cube's 8 children
    /// holds, applied through the 8 shared child matrices (saves memory).
    weights: Vec<[f64; 8]>,
    child: Vec<Vec<f64>>,
    /// Cube matrices kept whole: every cube on deeper levels; on the first,
    /// the part from children that touch held nodes.
    mats: Vec<Option<Box<[f64]>>>,
    inv_diag: Vec<f64>,
}

impl Level {
    fn matrix(&self, e: usize, out: &mut [f64; 576]) {
        out.fill(0.0);
        if !self.weights.is_empty() {
            for (o, &w) in self.weights[e].iter().enumerate() {
                if w != 0.0 {
                    out.iter_mut().zip(&self.child[o]).for_each(|(a, b)| *a += w * b);
                }
            }
        }
        if let Some(m) = &self.mats[e] {
            out.iter_mut().zip(m.iter()).for_each(|(a, b)| *a += b);
        }
    }

    fn finish(n: [usize; 3], cells: Vec<[usize; 3]>, weights: Vec<[f64; 8]>, child: Vec<Vec<f64>>, mats: Vec<Option<Box<[f64]>>>) -> Level {
        let corners: Vec<[usize; 8]> = cells.iter().map(|c| std::array::from_fn(|a| node_id(n, std::array::from_fn(|d| c[d] + offset(a)[d])))).collect();
        let nodes = (n[0] + 1) * (n[1] + 1) * (n[2] + 1);
        let mut level = Level { n, nodes, cells, corners, weights, child, mats, inv_diag: vec![] };
        let mut diag = vec![0.0; 3 * nodes];
        let mut m = [0.0; 576];
        for e in 0..level.cells.len() {
            level.matrix(e, &mut m);
            for (a, &node) in level.corners[e].iter().enumerate() {
                for d in 0..3 {
                    diag[3 * node + d] += m[(3 * a + d) * 25];
                }
            }
        }
        let big = diag.iter().cloned().fold(0.0, f64::max);
        level.inv_diag = diag.iter().map(|&v| if v > big * 1e-12 { 1.0 / v } else { 0.0 }).collect();
        level
    }

    /// The first coarse level, from the fine voxel model.
    fn from_fine(m: &Model) -> Level {
        let nc = coarser(m.grid.n);
        let interp: Vec<[[f64; 8]; 8]> = (0..8).map(|o| child_interp([o & 1, (o >> 1) & 1, (o >> 2) & 1])).collect();
        let child: Vec<Vec<f64>> = interp
            .iter()
            .map(|s| {
                let mut out = vec![0.0; 576];
                galerkin(&m.ke, s, &mut out);
                out
            })
            .collect();
        let mut slot: HashMap<[usize; 3], usize> = HashMap::new();
        let mut cells = vec![];
        let mut weights: Vec<[f64; 8]> = vec![];
        let mut mats: Vec<Option<Box<[f64]>>> = vec![];
        for (e, &[i, j, k]) in m.cells.iter().enumerate() {
            let c = *slot.entry([i / 2, j / 2, k / 2]).or_insert_with(|| {
                cells.push([i / 2, j / 2, k / 2]);
                weights.push([0.0; 8]);
                mats.push(None);
                cells.len() - 1
            });
            let o = (i & 1) | (j & 1) << 1 | (k & 1) << 2;
            let free: [bool; 24] = std::array::from_fn(|r| m.free(3 * m.corners[e][r / 3] + r % 3));
            if free.iter().all(|&f| f) {
                weights[c][o] += m.weight[e];
            } else {
                // Held nodes take no part: drop their rows and columns first.
                let w = m.weight[e];
                let km: Vec<f64> = (0..576).map(|x| if free[x / 24] && free[x % 24] { w * m.ke[x] } else { 0.0 }).collect();
                galerkin(&km, &interp[o], mats[c].get_or_insert_with(|| vec![0.0; 576].into_boxed_slice()));
            }
        }
        Level::finish(nc, cells, weights, child, mats)
    }

    /// The next coarser level.
    fn coarsen(&self) -> Level {
        let nc = coarser(self.n);
        let interp: Vec<[[f64; 8]; 8]> = (0..8).map(|o| child_interp([o & 1, (o >> 1) & 1, (o >> 2) & 1])).collect();
        let mut slot: HashMap<[usize; 3], usize> = HashMap::new();
        let mut cells = vec![];
        let mut children: Vec<Vec<(usize, usize)>> = vec![];
        for (e, &[i, j, k]) in self.cells.iter().enumerate() {
            let c = *slot.entry([i / 2, j / 2, k / 2]).or_insert_with(|| {
                cells.push([i / 2, j / 2, k / 2]);
                children.push(vec![]);
                cells.len() - 1
            });
            children[c].push((e, (i & 1) | (j & 1) << 1 | (k & 1) << 2));
        }
        let mats: Vec<Option<Box<[f64]>>> = children
            .par_iter()
            .map(|kids| {
                let mut out = vec![0.0; 576];
                let mut m = [0.0; 576];
                for &(e, o) in kids {
                    self.matrix(e, &mut m);
                    galerkin(&m, &interp[o], &mut out);
                }
                Some(out.into_boxed_slice())
            })
            .collect();
        Level::finish(nc, cells, vec![], vec![], mats)
    }

    fn apply(&self, x: &[f64]) -> Vec<f64> {
        let n3 = 3 * self.nodes;
        let mut y = (0..self.cells.len())
            .into_par_iter()
            .fold(
                || (vec![0.0; n3], [0.0; 576]),
                |(mut y, mut m), e| {
                    self.matrix(e, &mut m);
                    let c = &self.corners[e];
                    let mut xe = [0.0; 24];
                    for (a, &node) in c.iter().enumerate() {
                        xe[3 * a..3 * a + 3].copy_from_slice(&x[3 * node..3 * node + 3]);
                    }
                    for (a, &node) in c.iter().enumerate() {
                        for d in 0..3 {
                            let row = &m[(3 * a + d) * 24..(3 * a + d + 1) * 24];
                            y[3 * node + d] += row.iter().zip(&xe).map(|(p, q)| p * q).sum::<f64>();
                        }
                    }
                    (y, m)
                },
            )
            .map(|(y, _)| y)
            .reduce(|| vec![0.0; n3], |mut a, b| {
                a.iter_mut().zip(&b).for_each(|(p, q)| *p += q);
                a
            });
        y.iter_mut().zip(&self.inv_diag).for_each(|(v, d)| {
            if *d == 0.0 {
                *v = 0.0
            }
        });
        y
    }

    fn active(&self) -> usize {
        self.inv_diag.iter().filter(|&&d| d > 0.0).count()
    }
}

/// Along one axis: the coarse nodes a fine node is interpolated from.
fn shares(i: usize) -> ([usize; 2], [f64; 2], usize) {
    if i % 2 == 0 { ([i / 2, 0], [1.0, 0.0], 1) } else { ([i / 2, i / 2 + 1], [0.5, 0.5], 2) }
}

/// Fine values from coarse ones (trilinear).
fn prolong(nf: [usize; 3], nc: [usize; 3], xc: &[f64]) -> Vec<f64> {
    let plane = 3 * (nf[0] + 1) * (nf[1] + 1);
    let mut xf = vec![0.0; plane * (nf[2] + 1)];
    xf.par_chunks_mut(plane).enumerate().for_each(|(k, out)| {
        let (ks, kw, kn) = shares(k);
        for j in 0..=nf[1] {
            let (js, jw, jn) = shares(j);
            for i in 0..=nf[0] {
                let (is, iw, inn) = shares(i);
                let o = 3 * (i + (nf[0] + 1) * j);
                for c in 0..kn {
                    for b in 0..jn {
                        for a in 0..inn {
                            let w = iw[a] * jw[b] * kw[c];
                            let s = 3 * node_id(nc, [is[a], js[b], ks[c]]);
                            for d in 0..3 {
                                out[o + d] += w * xc[s + d];
                            }
                        }
                    }
                }
            }
        }
    });
    xf
}

/// Coarse values from fine ones: the transpose of `prolong`.
fn restrict(nf: [usize; 3], nc: [usize; 3], xf: &[f64]) -> Vec<f64> {
    let plane = 3 * (nc[0] + 1) * (nc[1] + 1);
    let mut xc = vec![0.0; plane * (nc[2] + 1)];
    let near = |c: usize, n: usize| -> Vec<(usize, f64)> {
        let mut v = vec![(2 * c, 1.0)];
        if c > 0 {
            v.push((2 * c - 1, 0.5));
        }
        if 2 * c < n {
            v.push((2 * c + 1, 0.5));
        }
        v.retain(|&(i, _)| i <= n);
        v
    };
    xc.par_chunks_mut(plane).enumerate().for_each(|(k, out)| {
        let kk = near(k, nf[2]);
        for j in 0..=nc[1] {
            let jj = near(j, nf[1]);
            for i in 0..=nc[0] {
                let ii = near(i, nf[0]);
                let o = 3 * (i + (nc[0] + 1) * j);
                for &(fk, wk) in &kk {
                    for &(fj, wj) in &jj {
                        for &(fi, wi) in &ii {
                            let w = wi * wj * wk;
                            let s = 3 * node_id(nf, [fi, fj, fk]);
                            for d in 0..3 {
                                out[o + d] += w * xf[s + d];
                            }
                        }
                    }
                }
            }
        }
    });
    xc
}

/// Dense Cholesky of the coarsest level.
struct Direct {
    dofs: Vec<usize>,
    l: Vec<f64>,
}

impl Direct {
    fn new(level: &Level) -> Direct {
        let dofs: Vec<usize> = (0..3 * level.nodes).filter(|&i| level.inv_diag[i] > 0.0).collect();
        let mut at = vec![usize::MAX; 3 * level.nodes];
        for (p, &d) in dofs.iter().enumerate() {
            at[d] = p;
        }
        let n = dofs.len();
        let mut a = vec![0.0; n * n];
        let mut m = [0.0; 576];
        for e in 0..level.cells.len() {
            level.matrix(e, &mut m);
            let c = &level.corners[e];
            for r in 0..24 {
                let pr = at[3 * c[r / 3] + r % 3];
                if pr == usize::MAX {
                    continue;
                }
                for s in 0..24 {
                    let ps = at[3 * c[s / 3] + s % 3];
                    if ps != usize::MAX {
                        a[pr * n + ps] += m[r * 24 + s];
                    }
                }
            }
        }
        // In place, lower triangle. A pivot that vanishes (a loose sliver the
        // coarse grid cannot hold) is pinned instead of failing.
        let big = (0..n).map(|i| a[i * n + i]).fold(0.0, f64::max);
        for j in 0..n {
            let mut d = a[j * n + j] - (0..j).map(|k| a[j * n + k] * a[j * n + k]).sum::<f64>();
            if d <= big * 1e-10 {
                d = big;
            }
            let d = d.sqrt();
            a[j * n + j] = d;
            let (head, tail) = a.split_at_mut((j + 1) * n);
            let rj = &head[j * n..j * n + j];
            tail.par_chunks_mut(n).for_each(|ri| {
                let s: f64 = ri[..j].iter().zip(rj).map(|(p, q)| p * q).sum();
                ri[j] = (ri[j] - s) / d;
            });
        }
        Direct { dofs, l: a }
    }

    fn solve(&self, r: &[f64]) -> Vec<f64> {
        let n = self.dofs.len();
        let mut y: Vec<f64> = self.dofs.iter().map(|&d| r[d]).collect();
        for i in 0..n {
            let s: f64 = (0..i).map(|k| self.l[i * n + k] * y[k]).sum();
            y[i] = (y[i] - s) / self.l[i * n + i];
        }
        for i in (0..n).rev() {
            let s: f64 = (i + 1..n).map(|k| self.l[k * n + i] * y[k]).sum();
            y[i] = (y[i] - s) / self.l[i * n + i];
        }
        let mut x = vec![0.0; r.len()];
        for (p, &d) in self.dofs.iter().enumerate() {
            x[d] = y[p];
        }
        x
    }
}

pub(super) struct Multigrid {
    levels: Vec<Level>,
    direct: Direct,
    fine_inv: Vec<f64>,
}

impl Multigrid {
    /// None when the model is small enough that Jacobi-CG is as quick.
    pub(super) fn build(m: &Model) -> Option<Multigrid> {
        let free = (0..3 * m.nodes).filter(|&i| m.free(i)).count();
        if free <= 4 * DIRECT {
            return None;
        }
        let mut levels = vec![Level::from_fine(m)];
        while levels.len() < MAX_LEVELS {
            let last = levels.last().unwrap();
            if last.active() <= DIRECT || last.n.iter().all(|&v| v <= 1) {
                break;
            }
            let next = last.coarsen();
            levels.push(next);
        }
        let last = levels.last().unwrap();
        if last.active() > 4 * DIRECT {
            return None; // could not coarsen far enough to solve directly
        }
        let direct = Direct::new(last);
        let fine_inv = (0..3 * m.nodes).map(|i| if m.free(i) && m.diag[i] > 0.0 { 1.0 / m.diag[i] } else { 0.0 }).collect();
        Some(Multigrid { levels, direct, fine_inv })
    }

    /// One V-cycle: an approximate K⁻¹·r.
    pub(super) fn precondition(&self, m: &Model, r: &[f64]) -> Vec<f64> {
        self.cycle(m, 0, r)
    }

    fn dims(&self, m: &Model, l: usize) -> [usize; 3] {
        if l == 0 { m.grid.n } else { self.levels[l - 1].n }
    }

    fn inv(&self, l: usize) -> &[f64] {
        if l == 0 { &self.fine_inv } else { &self.levels[l - 1].inv_diag }
    }

    fn apply(&self, m: &Model, l: usize, x: &[f64]) -> Vec<f64> {
        if l == 0 { m.apply(x) } else { self.levels[l - 1].apply(x) }
    }

    fn smooth(&self, m: &Model, l: usize, r: &[f64], x: &mut [f64]) {
        let ax = self.apply(m, l, x);
        let inv = self.inv(l);
        x.par_iter_mut().enumerate().for_each(|(i, v)| *v += OMEGA * inv[i] * (r[i] - ax[i]));
    }

    fn cycle(&self, m: &Model, l: usize, r: &[f64]) -> Vec<f64> {
        if l == self.levels.len() {
            return self.direct.solve(r);
        }
        let inv = self.inv(l);
        let mut x: Vec<f64> = r.par_iter().zip(inv).map(|(a, d)| OMEGA * a * d).collect();
        for _ in 1..SWEEPS {
            self.smooth(m, l, r, &mut x);
        }
        let ax = self.apply(m, l, &x);
        let res: Vec<f64> = r.par_iter().zip(&ax).map(|(a, b)| a - b).collect();
        let (nf, nc) = (self.dims(m, l), self.dims(m, l + 1));
        let mut rc = restrict(nf, nc, &res);
        let cinv = self.inv(l + 1);
        rc.iter_mut().zip(cinv).for_each(|(v, d)| {
            if *d == 0.0 {
                *v = 0.0
            }
        });
        let ec = self.cycle(m, l + 1, &rc);
        let corr = prolong(nf, nc, &ec);
        x.par_iter_mut().enumerate().for_each(|(i, v)| {
            if inv[i] > 0.0 {
                *v += corr[i]
            }
        });
        for _ in 0..SWEEPS {
            self.smooth(m, l, r, &mut x);
        }
        x
    }
}
