//! Decimation by quadric error metrics (Garland and Heckbert 1997): edges
//! collapse cheapest first, each vertex placed where it strays least from
//! the planes of the triangles it came from. Open rims are held by planes
//! square to them, and no collapse may fold a triangle over or pinch the
//! surface (the link condition).

use crate::V3;
use crate::linalg::*;
use crate::mesh::Mesh;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

type Quadric = [f64; 10];

fn plane_quadric(n: V3, d: f64, w: f64) -> Quadric {
    let [a, b, c] = n;
    [a * a * w, a * b * w, a * c * w, a * d * w, b * b * w, b * c * w, b * d * w, c * c * w, c * d * w, d * d * w]
}

fn qadd(a: &mut Quadric, b: &Quadric) {
    for i in 0..10 {
        a[i] += b[i];
    }
}

fn qerror(q: &Quadric, p: V3) -> f64 {
    let [x, y, z] = p;
    q[0] * x * x + 2.0 * q[1] * x * y + 2.0 * q[2] * x * z + 2.0 * q[3] * x + q[4] * y * y + 2.0 * q[5] * y * z + 2.0 * q[6] * y + q[7] * z * z + 2.0 * q[8] * z + q[9]
}

fn qoptimal(q: &Quadric) -> Option<V3> {
    let a = [q[0], q[1], q[2], q[1], q[4], q[5], q[2], q[5], q[7]];
    let det = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[1] * (a[3] * a[8] - a[5] * a[6]) + a[2] * (a[3] * a[7] - a[4] * a[6]);
    let scale_ = a[0].abs().max(a[4].abs()).max(a[8].abs()).max(1e-300);
    if det.abs() < 1e-9 * scale_ * scale_ * scale_ {
        return None;
    }
    solve_dense(&a, &[-q[3], -q[6], -q[8]], 3).map(|v| [v[0], v[1], v[2]])
}

#[derive(PartialEq)]
struct Candidate {
    cost: f64,
    u: u32,
    v: u32,
    stamp: (u32, u32),
    at: V3,
}

impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost)
    }
}

/// A mesh being collapsed: positions, live triangles and who uses each vertex.
pub(crate) struct Collapser {
    pub pos: Vec<V3>,
    pub tris: Vec<[u32; 3]>,
    pub alive: Vec<bool>,
    pub vtris: Vec<Vec<u32>>,
    pub live: usize,
}

impl Collapser {
    pub fn new(m: &Mesh) -> Collapser {
        Collapser { pos: m.positions.clone(), tris: m.triangles.clone(), alive: vec![true; m.triangles.len()], vtris: m.vertex_faces(), live: m.triangles.len() }
    }

    pub fn neighbours(&self, v: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for &t in &self.vtris[v as usize] {
            if self.alive[t as usize] {
                for &u in &self.tris[t as usize] {
                    if u != v && !out.contains(&u) {
                        out.push(u);
                    }
                }
            }
        }
        out
    }

    fn edge_faces(&self, u: u32, v: u32) -> usize {
        self.vtris[u as usize].iter().filter(|&&t| self.alive[t as usize] && self.tris[t as usize].contains(&v)).count()
    }

    /// Open (one triangle) edges at a vertex.
    pub fn on_boundary(&self, v: u32) -> bool {
        self.neighbours(v).iter().any(|&u| self.edge_faces(v, u) == 1)
    }

    /// Whether collapsing v into u (u moved to `at`) keeps the surface a
    /// manifold and folds no triangle by more than `max_turn` (cosine).
    pub fn can_collapse(&self, u: u32, v: u32, at: V3, min_cos: f64) -> bool {
        let shared = self.edge_faces(u, v);
        if shared == 0 || shared > 2 {
            return false;
        }
        let nu = self.neighbours(u);
        let nv = self.neighbours(v);
        let common = nu.iter().filter(|x| nv.contains(x)).count();
        if common != shared {
            return false;
        }
        // An edge joining two rims (not lying on one) would pinch.
        if shared == 2 && self.on_boundary(u) && self.on_boundary(v) {
            return false;
        }
        // A closed piece reduced to a tetrahedron stops.
        if nu.len() <= 3 && nv.len() <= 3 && shared == 2 {
            return false;
        }
        for (w, other) in [(u, v), (v, u)] {
            for &t in &self.vtris[w as usize] {
                if !self.alive[t as usize] || self.tris[t as usize].contains(&other) {
                    continue;
                }
                let tri = self.tris[t as usize];
                let p = tri.map(|x| self.pos[x as usize]);
                let before = tri_normal(p[0], p[1], p[2]);
                let q = tri.map(|x| if x == w { at } else { self.pos[x as usize] });
                let after = tri_normal(q[0], q[1], q[2]);
                let (lb, la) = (norm(before), norm(after));
                if la < 1e-14 * (lb + 1e-300) || dot(before, after) < min_cos * lb * la {
                    return false;
                }
            }
        }
        true
    }

    /// Collapses v into u, moving u to `at`.
    pub fn collapse(&mut self, u: u32, v: u32, at: V3) {
        self.pos[u as usize] = at;
        let vt = std::mem::take(&mut self.vtris[v as usize]);
        for t in vt {
            if !self.alive[t as usize] {
                continue;
            }
            if self.tris[t as usize].contains(&u) {
                self.alive[t as usize] = false;
                self.live -= 1;
            } else {
                for x in self.tris[t as usize].iter_mut() {
                    if *x == v {
                        *x = u;
                    }
                }
                self.vtris[u as usize].push(t);
            }
        }
        self.vtris[u as usize].retain(|&t| self.alive[t as usize]);
    }

    pub fn finish(self) -> Mesh {
        let tris = self.tris.iter().zip(&self.alive).filter(|(_, a)| **a).map(|(t, _)| *t).collect();
        Mesh::new(self.pos, tris).compacted()
    }
}

/// How far to decimate.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    /// Triangles to end with.
    pub triangles: usize,
    /// Stop before any collapse that moves the surface more than this (mm).
    pub max_error: Option<f64>,
}

/// The mesh reduced to about `target.triangles` triangles.
pub fn decimate(m: &Mesh, target: Target) -> Mesh {
    let mut c = Collapser::new(m);
    let mut q = vec![[0.0; 10]; c.pos.len()];
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for (t, tri) in m.triangles.iter().enumerate() {
        let p = m.corners(t);
        let n = tri_normal(p[0], p[1], p[2]);
        let area = norm(n) / 2.0;
        let n = unit(n);
        let pq = plane_quadric(n, -dot(n, p[0]), area);
        for &v in tri {
            qadd(&mut q[v as usize], &pq);
        }
        for k in 0..3 {
            *edges.entry(crate::mesh::edge_key(tri[k], tri[(k + 1) % 3])).or_default() += 1;
        }
    }
    // Rims hold: a heavy plane through each open edge, square to its triangle.
    for (t, tri) in m.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            if edges.get(&crate::mesh::edge_key(a, b)) == Some(&1) {
                let (pa, pb) = (m.positions[a as usize], m.positions[b as usize]);
                let e = sub(pb, pa);
                let n = unit(cross(e, m.face_normal(t)));
                let w = 1000.0 * dot(e, e);
                let pq = plane_quadric(n, -dot(n, pa), w);
                qadd(&mut q[a as usize], &pq);
                qadd(&mut q[b as usize], &pq);
            }
        }
    }
    let mut stamp = vec![0u32; c.pos.len()];
    let mut heap = BinaryHeap::new();
    let candidate = |c: &Collapser, q: &[Quadric], stamp: &[u32], u: u32, v: u32| -> Candidate {
        let mut s = q[u as usize];
        qadd(&mut s, &q[v as usize]);
        let (pu, pv) = (c.pos[u as usize], c.pos[v as usize]);
        let mut best = (qerror(&s, pu), pu);
        for p in [pv, mid(pu, pv)].into_iter().chain(qoptimal(&s)) {
            let e = qerror(&s, p);
            if e < best.0 {
                best = (e, p);
            }
        }
        Candidate { cost: best.0.max(0.0), u, v, stamp: (stamp[u as usize], stamp[v as usize]), at: best.1 }
    };
    for &(a, b) in edges.keys() {
        heap.push(candidate(&c, &q, &stamp, a, b));
    }
    let area: f64 = m.area().max(1e-300);
    while c.live > target.triangles {
        let Some(cand) = heap.pop() else { break };
        if cand.stamp != (stamp[cand.u as usize], stamp[cand.v as usize]) {
            continue;
        }
        if let Some(e) = target.max_error {
            // Quadric error is area-weighted squared distance: compare per unit area of the vertices' faces.
            let local: f64 = c.vtris[cand.u as usize].iter().chain(&c.vtris[cand.v as usize]).filter(|&&t| c.alive[t as usize]).map(|&t| {
                let p = c.tris[t as usize].map(|x| c.pos[x as usize]);
                norm(tri_normal(p[0], p[1], p[2])) / 2.0
            }).sum::<f64>().max(area * 1e-12);
            if cand.cost / local > e * e {
                continue;
            }
        }
        if !c.can_collapse(cand.u, cand.v, cand.at, 0.2) {
            continue;
        }
        c.collapse(cand.u, cand.v, cand.at);
        let qv = q[cand.v as usize];
        qadd(&mut q[cand.u as usize], &qv);
        stamp[cand.u as usize] += 1;
        stamp[cand.v as usize] += 1;
        for w in c.neighbours(cand.u) {
            heap.push(candidate(&c, &q, &stamp, cand.u, w));
        }
    }
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::analyze;
    use crate::mesh::uv_sphere;
    use crate::spatial::TriBvh;

    #[test]
    fn a_sphere_decimated_to_a_tenth_stays_round_and_closed() {
        let m = uv_sphere(20.0, 100, 200);
        let n = m.triangles.len();
        let d = decimate(&m, Target { triangles: n / 10, max_error: None });
        let r = analyze(&d);
        assert!(r.solid, "{r:?}");
        assert!(d.triangles.len() <= n / 10 + 2, "{}", d.triangles.len());
        // Every vertex still on the sphere (within 0.5% of the radius).
        let worst = d.positions.iter().map(|p| (norm(*p) - 20.0).abs()).fold(0.0, f64::max);
        assert!(worst < 0.1, "{worst}");
        let want = m.volume();
        assert!((d.volume() - want).abs() / want < 0.01);
    }

    #[test]
    fn flat_regions_collapse_to_almost_nothing_and_keep_their_edges() {
        // A finely triangulated flat plate (open): the rim must survive.
        let n = 40;
        let mut p = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                p.push([i as f64, j as f64, 0.0]);
            }
        }
        let idx = |i: u32, j: u32| j * (n as u32 + 1) + i;
        let mut t = Vec::new();
        for j in 0..n as u32 {
            for i in 0..n as u32 {
                t.push([idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
                t.push([idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
            }
        }
        let m = Mesh::new(p, t);
        let d = decimate(&m, Target { triangles: 2, max_error: Some(1e-6) });
        assert!(d.triangles.len() < 400, "{}", d.triangles.len());
        assert!((d.area() - 1600.0).abs() < 1e-6, "area kept: {}", d.area());
        let bvh = TriBvh::new(&d);
        for q in [[0.0, 0.0, 0.0], [40.0, 40.0, 0.0], [20.0, 0.0, 0.0]] {
            assert!(bvh.closest(q).unwrap().distance < 1e-9);
        }
    }
}
