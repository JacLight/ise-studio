//! An indexed triangle mesh and the bookkeeping every operation shares:
//! welding, edges and their faces, components, normals, area and volume.

use crate::V3;
use crate::linalg::*;
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<V3>,
    pub triangles: Vec<[u32; 3]>,
}

/// An undirected edge, smaller vertex first.
pub fn edge_key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

impl Mesh {
    pub fn new(positions: Vec<V3>, triangles: Vec<[u32; 3]>) -> Mesh {
        Mesh { positions, triangles }
    }

    /// From flat arrays (x, y, z, … and a, b, c, …).
    pub fn from_flat(positions: &[f64], triangles: &[u32]) -> Mesh {
        Mesh {
            positions: positions.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect(),
            triangles: triangles.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect(),
        }
    }

    pub fn flat_positions(&self) -> Vec<f64> {
        self.positions.iter().flatten().copied().collect()
    }

    pub fn flat_triangles(&self) -> Vec<u32> {
        self.triangles.iter().flatten().copied().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    pub fn corners(&self, t: usize) -> [V3; 3] {
        let [a, b, c] = self.triangles[t];
        [self.positions[a as usize], self.positions[b as usize], self.positions[c as usize]]
    }

    /// Twice the area, along the normal.
    pub fn face_cross(&self, t: usize) -> V3 {
        let [a, b, c] = self.corners(t);
        tri_normal(a, b, c)
    }

    pub fn face_normal(&self, t: usize) -> V3 {
        unit(self.face_cross(t))
    }

    pub fn face_area(&self, t: usize) -> f64 {
        norm(self.face_cross(t)) / 2.0
    }

    pub fn area(&self) -> f64 {
        (0..self.triangles.len()).map(|t| self.face_area(t)).sum()
    }

    /// Signed volume (positive for a closed mesh wound outward).
    pub fn volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.positions[i as usize]);
                dot(a, cross(b, c)) / 6.0
            })
            .sum()
    }

    pub fn bounds(&self) -> (V3, V3) {
        bounds(&self.positions)
    }

    /// The diagonal of the bounding box.
    pub fn size(&self) -> f64 {
        let (lo, hi) = self.bounds();
        dist(lo, hi)
    }

    /// Mean edge length.
    pub fn mean_edge(&self) -> f64 {
        let mut s = 0.0;
        let mut n = 0usize;
        for t in &self.triangles {
            for k in 0..3 {
                s += dist(self.positions[t[k] as usize], self.positions[t[(k + 1) % 3] as usize]);
                n += 1;
            }
        }
        if n == 0 { 0.0 } else { s / n as f64 }
    }

    /// Angle-weighted vertex normals.
    pub fn vertex_normals(&self) -> Vec<V3> {
        let mut n = vec![[0.0; 3]; self.positions.len()];
        for t in 0..self.triangles.len() {
            let tri = self.triangles[t];
            let fnorm = self.face_normal(t);
            let p = self.corners(t);
            for k in 0..3 {
                let e1 = unit(sub(p[(k + 1) % 3], p[k]));
                let e2 = unit(sub(p[(k + 2) % 3], p[k]));
                let angle = dot(e1, e2).clamp(-1.0, 1.0).acos();
                let i = tri[k] as usize;
                n[i] = add(n[i], scale(fnorm, angle));
            }
        }
        n.into_iter().map(unit).collect()
    }

    /// Every undirected edge and the triangles that use it.
    pub fn edge_faces(&self) -> HashMap<(u32, u32), Vec<u32>> {
        let mut m: HashMap<(u32, u32), Vec<u32>> = HashMap::with_capacity(self.triangles.len() * 2);
        for (t, tri) in self.triangles.iter().enumerate() {
            for k in 0..3 {
                m.entry(edge_key(tri[k], tri[(k + 1) % 3])).or_default().push(t as u32);
            }
        }
        m
    }

    /// The triangles at each vertex.
    pub fn vertex_faces(&self) -> Vec<Vec<u32>> {
        let mut vf = vec![Vec::new(); self.positions.len()];
        for (t, tri) in self.triangles.iter().enumerate() {
            for &v in tri {
                vf[v as usize].push(t as u32);
            }
        }
        vf
    }

    /// The vertices joined to each vertex by an edge.
    pub fn vertex_neighbours(&self) -> Vec<Vec<u32>> {
        let mut nb: Vec<Vec<u32>> = vec![Vec::new(); self.positions.len()];
        for tri in &self.triangles {
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                if !nb[a as usize].contains(&b) {
                    nb[a as usize].push(b);
                }
                if !nb[b as usize].contains(&a) {
                    nb[b as usize].push(a);
                }
            }
        }
        nb
    }

    /// Triangles grouped into pieces connected through shared edges (an id per triangle, and how many).
    pub fn components(&self) -> (Vec<u32>, usize) {
        let ef = self.edge_faces();
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); self.triangles.len()];
        for faces in ef.values() {
            for &a in faces {
                for &b in faces {
                    if a != b {
                        adj[a as usize].push(b);
                    }
                }
            }
        }
        let mut id = vec![u32::MAX; self.triangles.len()];
        let mut n = 0u32;
        let mut stack = Vec::new();
        for s in 0..self.triangles.len() {
            if id[s] != u32::MAX {
                continue;
            }
            id[s] = n;
            stack.push(s as u32);
            while let Some(t) = stack.pop() {
                for &u in &adj[t as usize] {
                    if id[u as usize] == u32::MAX {
                        id[u as usize] = n;
                        stack.push(u);
                    }
                }
            }
            n += 1;
        }
        (id, n as usize)
    }

    /// The triangles of one piece each, largest (by triangles) first.
    pub fn split_components(&self) -> Vec<Mesh> {
        let (id, n) = self.components();
        let mut groups: Vec<Vec<[u32; 3]>> = vec![Vec::new(); n];
        for (t, c) in id.iter().enumerate() {
            groups[*c as usize].push(self.triangles[t]);
        }
        let mut out: Vec<Mesh> = groups.into_iter().map(|g| Mesh { positions: self.positions.clone(), triangles: g }.compacted()).collect();
        out.sort_by(|a, b| b.triangles.len().cmp(&a.triangles.len()));
        out
    }

    /// Vertices no triangle uses dropped, the rest renumbered.
    pub fn compacted(mut self) -> Mesh {
        self.compact();
        self
    }

    pub fn compact(&mut self) {
        let mut map = vec![u32::MAX; self.positions.len()];
        let mut pos = Vec::new();
        for t in self.triangles.iter_mut() {
            for v in t.iter_mut() {
                let i = *v as usize;
                if map[i] == u32::MAX {
                    map[i] = pos.len() as u32;
                    pos.push(self.positions[i]);
                }
                *v = map[i];
            }
        }
        self.positions = pos;
    }

    /// Vertices closer than `eps` merged (mesh files often repeat a vertex
    /// for every triangle); triangles that collapse dropped.
    pub fn weld(&mut self, eps: f64) {
        let eps = eps.max(1e-12);
        let key = |p: V3| [(p[0] / eps).round() as i64, (p[1] / eps).round() as i64, (p[2] / eps).round() as i64];
        let mut cells: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
        let mut out: Vec<V3> = Vec::new();
        let mut remap = Vec::with_capacity(self.positions.len());
        for &p in &self.positions {
            let k = key(p);
            // A neighbour cell may hold a point within eps (rounding splits them).
            let mut found = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = cells.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) {
                            for &i in list {
                                if dist2(out[i as usize], p) <= eps * eps {
                                    found = Some(i);
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
            let i = found.unwrap_or_else(|| {
                out.push(p);
                let i = (out.len() - 1) as u32;
                cells.entry(k).or_default().push(i);
                i
            });
            remap.push(i);
        }
        self.positions = out;
        for t in self.triangles.iter_mut() {
            *t = t.map(|v| remap[v as usize]);
        }
        self.triangles.retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
    }

    /// A weld tolerance for this mesh: a millionth of its size.
    pub fn weld_eps(&self) -> f64 {
        (self.size() * 1e-7).max(1e-9)
    }

    /// Every triangle turned the other way.
    pub fn flip(&mut self) {
        for t in self.triangles.iter_mut() {
            t.swap(1, 2);
        }
    }

    /// Another mesh's triangles added (as one mesh).
    pub fn merge(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.triangles.extend(other.triangles.iter().map(|t| t.map(|v| v + base)));
    }

    /// Moved by a 3×4 affine matrix (rows).
    pub fn transform(&mut self, m: &[[f64; 4]; 3]) {
        for p in self.positions.iter_mut() {
            *p = apply_affine(m, *p);
        }
        // A mirroring transform turns the triangles inside out: put them back.
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if det < 0.0 {
            self.flip();
        }
    }

    /// The vertex loops along open edges (each loop's vertices in order).
    pub fn boundary_loops(&self) -> Vec<Vec<u32>> {
        boundary_loops(self)
    }
}

pub fn apply_affine(m: &[[f64; 4]; 3], p: V3) -> V3 {
    [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3])
}

pub fn bounds(points: &[V3]) -> (V3, V3) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if points.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    (lo, hi)
}

/// Open edges (used by one triangle) chained into loops, following each
/// triangle's winding (so a hole's loop runs against its rim's triangles).
fn boundary_loops(m: &Mesh) -> Vec<Vec<u32>> {
    let ef = m.edge_faces();
    // Directed open edges as their triangle runs them.
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    for tri in &m.triangles {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            if ef.get(&edge_key(a, b)).is_some_and(|f| f.len() == 1) {
                // The hole runs the other way round.
                next.entry(b).or_default().push(a);
            }
        }
    }
    let mut loops = Vec::new();
    let starts: Vec<u32> = {
        let mut s: Vec<u32> = next.keys().copied().collect();
        s.sort_unstable();
        s
    };
    for s in starts {
        while next.get(&s).is_some_and(|v| !v.is_empty()) {
            let mut lp = vec![s];
            let mut cur = s;
            loop {
                let Some(list) = next.get_mut(&cur) else { break };
                let Some(n) = list.pop() else { break };
                if n == s {
                    break;
                }
                // A vertex the boundary passes twice: split the loop there.
                if let Some(pos) = lp.iter().position(|&v| v == n) {
                    let inner: Vec<u32> = lp.drain(pos..).collect();
                    if inner.len() >= 3 {
                        loops.push(inner);
                    }
                }
                lp.push(n);
                cur = n;
                if lp.len() > m.positions.len() + 1 {
                    break;
                }
            }
            if lp.len() >= 3 {
                loops.push(lp);
            }
        }
    }
    loops
}

/// A box's mesh (min corner at the origin), wound outward: for tests and examples.
pub fn cube(x: f64, y: f64, z: f64) -> Mesh {
    let p = vec![[0.0, 0.0, 0.0], [x, 0.0, 0.0], [x, y, 0.0], [0.0, y, 0.0], [0.0, 0.0, z], [x, 0.0, z], [x, y, z], [0.0, y, z]];
    let t = vec![[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
    Mesh::new(p, t)
}

/// A UV sphere, wound outward.
pub fn uv_sphere(r: f64, rings: usize, segments: usize) -> Mesh {
    let mut p = vec![[0.0, 0.0, r]];
    for i in 1..rings {
        let th = std::f64::consts::PI * i as f64 / rings as f64;
        for j in 0..segments {
            let ph = std::f64::consts::TAU * j as f64 / segments as f64;
            p.push([r * th.sin() * ph.cos(), r * th.sin() * ph.sin(), r * th.cos()]);
        }
    }
    p.push([0.0, 0.0, -r]);
    let s = segments as u32;
    let ring = |i: u32, j: u32| 1 + i * s + (j % s);
    let mut t = Vec::new();
    for j in 0..s {
        t.push([0, ring(0, j), ring(0, j + 1)]);
    }
    for i in 0..(rings as u32 - 2) {
        for j in 0..s {
            t.push([ring(i, j), ring(i + 1, j), ring(i + 1, j + 1)]);
            t.push([ring(i, j), ring(i + 1, j + 1), ring(i, j + 1)]);
        }
    }
    let last = p.len() as u32 - 1;
    for j in 0..s {
        t.push([last, ring(rings as u32 - 2, j + 1), ring(rings as u32 - 2, j)]);
    }
    Mesh::new(p, t)
}

/// A closed cylinder along z from 0 to h, wound outward.
pub fn cylinder(r: f64, h: f64, segments: usize, stacks: usize) -> Mesh {
    let s = segments as u32;
    let mut p = Vec::new();
    for i in 0..=stacks {
        let z = h * i as f64 / stacks as f64;
        for j in 0..segments {
            let a = std::f64::consts::TAU * j as f64 / segments as f64;
            p.push([r * a.cos(), r * a.sin(), z]);
        }
    }
    let at = |i: u32, j: u32| i * s + (j % s);
    let mut t = Vec::new();
    for i in 0..stacks as u32 {
        for j in 0..s {
            t.push([at(i, j), at(i, j + 1), at(i + 1, j + 1)]);
            t.push([at(i, j), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    let bottom = p.len() as u32;
    p.push([0.0, 0.0, 0.0]);
    let top = p.len() as u32;
    p.push([0.0, 0.0, h]);
    for j in 0..s {
        t.push([bottom, at(0, j + 1), at(0, j)]);
        t.push([top, at(stacks as u32, j), at(stacks as u32, j + 1)]);
    }
    Mesh::new(p, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_close_with_the_right_volume() {
        assert!((cube(2.0, 3.0, 4.0).volume() - 24.0).abs() < 1e-12);
        let s = uv_sphere(10.0, 64, 128);
        let want = 4.0 / 3.0 * std::f64::consts::PI * 1000.0;
        assert!((s.volume() - want).abs() / want < 0.01, "{}", s.volume());
        let c = cylinder(5.0, 10.0, 64, 3);
        assert!(c.volume() > 0.0);
        for m in [cube(1.0, 1.0, 1.0), s, c] {
            assert!(m.edge_faces().values().all(|f| f.len() == 2));
            assert!(m.boundary_loops().is_empty());
        }
    }

    #[test]
    fn welding_joins_a_soup_and_holes_show_as_loops() {
        let c = cube(1.0, 1.0, 1.0);
        let mut soup = Mesh::default();
        for t in &c.triangles {
            let base = soup.positions.len() as u32;
            for &v in t {
                soup.positions.push(c.positions[v as usize]);
            }
            soup.triangles.push([base, base + 1, base + 2]);
        }
        assert_eq!(soup.positions.len(), 36);
        soup.weld(1e-9);
        assert_eq!(soup.positions.len(), 8);
        soup.triangles.remove(0);
        let loops = soup.boundary_loops();
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 3);
    }
}
