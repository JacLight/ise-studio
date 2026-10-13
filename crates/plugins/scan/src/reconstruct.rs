//! Surface reconstruction from oriented points by Poisson's equation
//! (Kazhdan, Bolitho and Hoppe 2006): the points' normals are the gradient
//! of an inside/outside function; its Laplacian is their divergence, solved
//! here on a regular grid by multigrid V-cycles, and the surface is the level
//! of that function at the points, extracted by marching tetrahedra. The
//! result is closed (watertight) unless asked to be trimmed back where the
//! scan has no points.

use crate::V3;
use crate::linalg::*;
use crate::mesh::Mesh;
use crate::points::{Cloud, Orient};
use crate::spatial::KdTree;
use rayon::prelude::*;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Grid of 2^depth cells along the longest side (5…9).
    pub depth: u32,
    /// Drop triangles farther than this many point spacings from any point
    /// (0: keep the closed surface).
    pub trim: f64,
    /// Taubin smoothing passes afterward.
    pub smooth: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { depth: 7, trim: 0.0, smooth: 2 }
    }
}

/// A node grid of n×n×n (n = 2^k + 1), zero on its faces.
struct Grid {
    n: usize,
    v: Vec<f32>,
}

impl Grid {
    fn new(n: usize) -> Grid {
        Grid { n, v: vec![0.0; n * n * n] }
    }
    #[inline]
    fn at(&self, i: usize, j: usize, k: usize) -> usize {
        (k * self.n + j) * self.n + i
    }
}

/// Red-black Gauss–Seidel sweeps of Δu = f (7-point stencil, spacing h).
fn smooth(u: &mut Grid, f: &Grid, h2: f32, sweeps: usize) {
    let n = u.n;
    for _ in 0..sweeps {
        for colour in 0..2 {
            // Planes of k are independent within one colour (neighbours in k have the other colour).
            let ptr = u.v.as_mut_ptr() as usize;
            (1..n - 1).into_par_iter().for_each(|k| {
                let p = ptr as *mut f32;
                for j in 1..n - 1 {
                    let start = 1 + ((j + k + colour) % 2);
                    let mut i = start;
                    while i < n - 1 {
                        let c = (k * n + j) * n + i;
                        // SAFETY: each thread writes only its own colour's cells in plane k and reads neighbours of the other colour.
                        unsafe {
                            let s = *p.add(c - 1) + *p.add(c + 1) + *p.add(c - n) + *p.add(c + n) + *p.add(c - n * n) + *p.add(c + n * n);
                            *p.add(c) = (s - h2 * f.v[c]) / 6.0;
                        }
                        i += 2;
                    }
                }
            });
        }
    }
}

fn residual(u: &Grid, f: &Grid, h2: f32) -> Grid {
    let n = u.n;
    let mut r = Grid::new(n);
    r.v.par_chunks_mut(n * n).enumerate().for_each(|(k, plane)| {
        if k == 0 || k == n - 1 {
            return;
        }
        for j in 1..n - 1 {
            for i in 1..n - 1 {
                let c = (k * n + j) * n + i;
                let lap = (u.v[c - 1] + u.v[c + 1] + u.v[c - n] + u.v[c + n] + u.v[c - n * n] + u.v[c + n * n] - 6.0 * u.v[c]) / h2;
                plane[j * n + i] = f.v[c] - lap;
            }
        }
    });
    r
}

/// Full-weighting restriction to the coarse grid ((n+1)/2 nodes a side).
fn restrict(r: &Grid) -> Grid {
    let nc = (r.n + 1) / 2;
    let mut c = Grid::new(nc);
    let w = [0.25f32, 0.5, 0.25];
    c.v.par_chunks_mut(nc * nc).enumerate().for_each(|(k, plane)| {
        if k == 0 || k == nc - 1 {
            return;
        }
        for j in 1..nc - 1 {
            for i in 1..nc - 1 {
                let mut s = 0.0;
                for dz in 0..3 {
                    for dy in 0..3 {
                        for dx in 0..3 {
                            s += w[dx] * w[dy] * w[dz] * r.v[r.at(2 * i + dx - 1, 2 * j + dy - 1, 2 * k + dz - 1)];
                        }
                    }
                }
                plane[j * nc + i] = s;
            }
        }
    });
    c
}

/// Adds the trilinear interpolation of a coarse correction.
fn prolong_add(u: &mut Grid, e: &Grid) {
    let n = u.n;
    u.v.par_chunks_mut(n * n).enumerate().for_each(|(k, plane)| {
        if k == 0 || k == n - 1 {
            return;
        }
        for j in 1..n - 1 {
            for i in 1..n - 1 {
                let (ci, cj, ck) = (i / 2, j / 2, k / 2);
                let (fi, fj, fk) = ((i % 2) as f32 * 0.5, (j % 2) as f32 * 0.5, (k % 2) as f32 * 0.5);
                let g = |a: usize, b: usize, c: usize| e.v[e.at(a.min(e.n - 1), b.min(e.n - 1), c.min(e.n - 1))];
                let mut s = 0.0;
                for (dz, wz) in [(0, 1.0 - fk), (1, fk)] {
                    for (dy, wy) in [(0, 1.0 - fj), (1, fj)] {
                        for (dx, wx) in [(0, 1.0 - fi), (1, fi)] {
                            if wx * wy * wz > 0.0 {
                                s += wx * wy * wz * g(ci + dx, cj + dy, ck + dz);
                            }
                        }
                    }
                }
                plane[j * n + i] += s;
            }
        }
    });
}

fn v_cycle(u: &mut Grid, f: &Grid, h: f32) {
    let h2 = h * h;
    if u.n <= 5 {
        smooth(u, f, h2, 50);
        return;
    }
    smooth(u, f, h2, 3);
    let r = residual(u, f, h2);
    let rc = restrict(&r);
    let mut ec = Grid::new(rc.n);
    v_cycle(&mut ec, &rc, 2.0 * h);
    prolong_add(u, &ec);
    smooth(u, f, h2, 3);
}

/// The closed surface through oriented points (normals are estimated when the cloud has none).
pub fn poisson(cloud: &Cloud, o: &Options) -> Result<Mesh, String> {
    if cloud.len() < 20 {
        return Err("reconstruction needs at least 20 points".into());
    }
    let mut c = cloud.clone();
    if c.normals.len() != c.len() {
        crate::points::estimate_normals(&mut c, 12, Orient::Outward);
    }
    let depth = o.depth.clamp(4, 9);
    let cells = 1usize << depth;
    let n = cells + 1;
    let (lo, hi) = c.bounds();
    let size = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max).max(1e-9);
    // Margin so the surface never touches the grid's (zero) faces.
    let pad = size * 0.12;
    let span = size + 2.0 * pad;
    let h = span / cells as f64;
    let centre = mid(lo, hi);
    let origin = sub(centre, [span / 2.0; 3]);
    // Divergence of the normal field, splatted onto the nodes (the weak form: −⟨V, ∇φ⟩ per node's hat function φ).
    let mut f = Grid::new(n);
    let spacing = c.spacing().max(1e-12);
    let weight = (spacing * spacing) as f32;
    for (p, nrm) in c.positions.iter().zip(&c.normals) {
        let g = [0, 1, 2].map(|k| (p[k] - origin[k]) / h);
        let base = g.map(|v| (v.floor() as isize).clamp(0, cells as isize - 1) as usize);
        let t = [0, 1, 2].map(|k| g[k] - base[k] as f64);
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    let w = |d: usize, t: f64| if d == 1 { t } else { 1.0 - t };
                    let dw = |d: usize| if d == 1 { 1.0 / h } else { -1.0 / h };
                    let grad = [dw(dx) * w(dy, t[1]) * w(dz, t[2]), w(dx, t[0]) * dw(dy) * w(dz, t[2]), w(dx, t[0]) * w(dy, t[1]) * dw(dz)];
                    let (i, j, k) = (base[0] + dx, base[1] + dy, base[2] + dz);
                    let at = f.at(i, j, k);
                    // Scaled to a density per node volume so the solve's units do not depend on the grid.
                    f.v[at] -= (dot(*nrm, grad) as f32) * weight / (h * h * h) as f32;
                }
            }
        }
    }
    let mut u = Grid::new(n);
    let hf = h as f32;
    for _ in 0..12 {
        v_cycle(&mut u, &f, hf);
    }
    // The surface's level: the function's mean at the points.
    let sample = |p: V3| -> f64 {
        let g = [0, 1, 2].map(|k| (p[k] - origin[k]) / h);
        let base = g.map(|v| (v.floor() as isize).clamp(0, cells as isize - 1) as usize);
        let t = [0, 1, 2].map(|k| g[k] - base[k] as f64);
        let mut s = 0.0;
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    let w = (if dx == 1 { t[0] } else { 1.0 - t[0] }) * (if dy == 1 { t[1] } else { 1.0 - t[1] }) * (if dz == 1 { t[2] } else { 1.0 - t[2] });
                    s += w * u.v[u.at(base[0] + dx, base[1] + dy, base[2] + dz)] as f64;
                }
            }
        }
        s
    };
    let iso = c.positions.iter().map(|p| sample(*p)).sum::<f64>() / c.len() as f64;
    let mut m = marching_tets(&u, iso as f32, origin, h);
    if m.triangles.is_empty() {
        return Err("no surface found: are the normals consistent?".into());
    }
    // Keep the piece the points lie on (the solve can leave small bubbles far from the data).
    let mut pieces = m.split_components();
    pieces.truncate(1);
    m = pieces.pop().unwrap_or_default();
    if m.volume() < 0.0 {
        m.flip();
    }
    if o.smooth > 0 {
        crate::smooth::smooth(&mut m, &crate::smooth::Options { iterations: o.smooth, ..Default::default() });
    }
    if o.trim > 0.0 {
        let tree = KdTree::new(&c.positions);
        let limit = o.trim * spacing;
        let keep: Vec<bool> = m.triangles.par_iter().map(|t| {
            let cen = scale(add(add(m.positions[t[0] as usize], m.positions[t[1] as usize]), m.positions[t[2] as usize]), 1.0 / 3.0);
            tree.nearest(cen).is_some_and(|(_, d2)| d2.sqrt() <= limit)
        }).collect();
        let mut i = 0;
        m.triangles.retain(|_| {
            i += 1;
            keep[i - 1]
        });
        m.compact();
    }
    Ok(m)
}

/// The level set u = iso by marching tetrahedra (six per cube, split along
/// the cube's main diagonal so neighbouring cubes agree), wound outward
/// from where u is lower.
fn marching_tets(u: &Grid, iso: f32, origin: V3, h: f64) -> Mesh {
    const CORNER: [[usize; 3]; 8] = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];
    const TETS: [[usize; 4]; 6] = [[0, 5, 1, 6], [0, 1, 2, 6], [0, 2, 3, 6], [0, 3, 7, 6], [0, 7, 4, 6], [0, 4, 5, 6]];
    let n = u.n;
    let pos = |id: usize| -> V3 {
        let (i, j, k) = (id % n, (id / n) % n, id / (n * n));
        [origin[0] + i as f64 * h, origin[1] + j as f64 * h, origin[2] + k as f64 * h]
    };
    // Per plane of cubes, in parallel; vertices shared through edge keys afterwards.
    let planes: Vec<Vec<[(usize, usize); 3]>> = (0..n - 1)
        .into_par_iter()
        .map(|k| {
            let mut tris = Vec::new();
            for j in 0..n - 1 {
                for i in 0..n - 1 {
                    let ids: [usize; 8] = CORNER.map(|c| ((k + c[2]) * n + j + c[1]) * n + i + c[0]);
                    let vals: [f32; 8] = ids.map(|id| u.v[id] - iso);
                    if vals.iter().all(|v| *v < 0.0) || vals.iter().all(|v| *v >= 0.0) {
                        continue;
                    }
                    for t in TETS {
                        let tv = t.map(|c| (ids[c], vals[c]));
                        let inside: Vec<usize> = (0..4).filter(|&q| tv[q].1 < 0.0).collect();
                        let outside: Vec<usize> = (0..4).filter(|&q| tv[q].1 >= 0.0).collect();
                        let e = |a: usize, b: usize| {
                            let (x, y) = (tv[a].0, tv[b].0);
                            if x < y { (x, y) } else { (y, x) }
                        };
                        match inside.len() {
                            1 | 3 => {
                                let (one, rest) = if inside.len() == 1 { (inside[0], &outside) } else { (outside[0], &inside) };
                                tris.push([e(one, rest[0]), e(one, rest[1]), e(one, rest[2])]);
                            }
                            2 => {
                                let (a, b, c2, d) = (inside[0], inside[1], outside[0], outside[1]);
                                tris.push([e(a, c2), e(a, d), e(b, d)]);
                                tris.push([e(a, c2), e(b, d), e(b, c2)]);
                            }
                            _ => {}
                        }
                    }
                }
            }
            tris
        })
        .collect();
    let mut index: HashMap<(usize, usize), u32> = HashMap::new();
    let mut m = Mesh::default();
    for tris in planes {
        for t in tris {
            let ids = t.map(|(a, b)| {
                *index.entry((a, b)).or_insert_with(|| {
                    let (va, vb) = (u.v[a] - iso, u.v[b] - iso);
                    let s = (va / (va - vb)).clamp(0.0, 1.0) as f64;
                    m.positions.push(lerp(pos(a), pos(b), s));
                    (m.positions.len() - 1) as u32
                })
            });
            if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
                continue;
            }
            // Wind each triangle so it faces from low u to high u.
            let p = ids.map(|v| m.positions[v as usize]);
            let nrm = tri_normal(p[0], p[1], p[2]);
            let cen = scale(add(add(p[0], p[1]), p[2]), 1.0 / 3.0);
            let g = gradient(u, cen, origin, h);
            m.triangles.push(if dot(nrm, g) >= 0.0 { ids } else { [ids[0], ids[2], ids[1]] });
        }
    }
    m.weld(h * 1e-6);
    // Marching leaves slivers where the surface passes near grid nodes:
    // their short edges collapse away.
    tidy(&mut m, h * 0.2);
    m
}

/// Collapses edges shorter than `lo` (to their middles) where that keeps the surface sound.
fn tidy(m: &mut Mesh, lo: f64) {
    let mut c = crate::decimate::Collapser::new(m);
    let mut edges: Vec<((u32, u32), f64)> = m.edge_faces().keys().map(|&(a, b)| ((a, b), dist(m.positions[a as usize], m.positions[b as usize]))).filter(|(_, l)| *l < lo).collect();
    edges.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut dead = std::collections::HashSet::new();
    for ((a, b), _) in edges {
        if dead.contains(&a) || dead.contains(&b) {
            continue;
        }
        let at = mid(c.pos[a as usize], c.pos[b as usize]);
        if c.can_collapse(a, b, at, 0.3) {
            c.collapse(a, b, at);
            dead.insert(b);
        }
    }
    *m = c.finish();
}

fn gradient(u: &Grid, p: V3, origin: V3, h: f64) -> V3 {
    let n = u.n;
    let g = [0, 1, 2].map(|k| ((p[k] - origin[k]) / h).round().clamp(1.0, (n - 2) as f64) as usize);
    let at = |i: usize, j: usize, k: usize| u.v[u.at(i, j, k)] as f64;
    [
        at(g[0] + 1, g[1], g[2]) - at(g[0] - 1, g[1], g[2]),
        at(g[0], g[1] + 1, g[2]) - at(g[0], g[1] - 1, g[2]),
        at(g[0], g[1], g[2] + 1) - at(g[0], g[1], g[2] - 1),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::analyze;
    use crate::mesh::uv_sphere;
    use crate::points::sample_mesh;

    #[test]
    fn a_sampled_sphere_comes_back_closed_and_round() {
        let s = uv_sphere(25.0, 40, 80);
        let mut c = sample_mesh(&s, 6000);
        c.normals.clear();
        let m = poisson(&c, &Options { depth: 6, ..Default::default() }).unwrap();
        let r = analyze(&m);
        assert!(r.solid, "{r:?}");
        let radii: Vec<f64> = m.positions.iter().map(|p| norm(*p)).collect();
        let mean = radii.iter().sum::<f64>() / radii.len() as f64;
        let worst = radii.iter().map(|r| (r - 25.0).abs()).fold(0.0, f64::max);
        assert!((mean - 25.0).abs() < 0.4, "mean radius {mean}");
        assert!(worst < 1.2, "worst {worst}");
    }

    #[test]
    fn trimming_opens_where_there_are_no_points() {
        let s = uv_sphere(25.0, 40, 80);
        let mut c = sample_mesh(&s, 6000);
        // Only the lower half was scanned.
        let keep: Vec<u32> = (0..c.len() as u32).filter(|&i| c.positions[i as usize][2] < 5.0).collect();
        c = c.select(&keep);
        let m = poisson(&c, &Options { depth: 6, trim: 2.5, ..Default::default() }).unwrap();
        let r = analyze(&m);
        assert!(r.holes >= 1, "{r:?}");
        assert!(m.positions.iter().all(|p| p[2] < 12.0));
    }
}
