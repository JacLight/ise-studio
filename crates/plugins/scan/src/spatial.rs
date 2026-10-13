//! Spatial search: a k-d tree over points (nearest neighbours, radius
//! queries) and a bounding-volume tree over triangles (closest point,
//! signed distance by angle-weighted pseudo-normals after Bærentzen and
//! Aanæs 2005, overlap candidates).

use crate::V3;
use crate::linalg::*;
use crate::mesh::Mesh;
use std::collections::HashMap;

/// A k-d tree over points (indices into the slice it was built from).
pub struct KdTree {
    points: Vec<V3>,
    /// Point indices, arranged so each node's points are a contiguous run.
    index: Vec<u32>,
    nodes: Vec<KdNode>,
}

struct KdNode {
    start: u32,
    end: u32,
    axis: u8,
    split: f64,
    left: u32,
    right: u32,
}

const LEAF: usize = 12;

impl KdTree {
    pub fn new(points: &[V3]) -> KdTree {
        let mut t = KdTree { points: points.to_vec(), index: (0..points.len() as u32).collect(), nodes: Vec::new() };
        if !points.is_empty() {
            t.build(0, points.len());
        }
        t
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    fn build(&mut self, start: usize, end: usize) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(KdNode { start: start as u32, end: end as u32, axis: 0, split: 0.0, left: u32::MAX, right: u32::MAX });
        if end - start <= LEAF {
            return id;
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for &i in &self.index[start..end] {
            let p = self.points[i as usize];
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap();
        let midi = (start + end) / 2;
        let pts = &self.points;
        self.index[start..end].select_nth_unstable_by(midi - start, |&a, &b| pts[a as usize][axis].total_cmp(&pts[b as usize][axis]));
        let split = self.points[self.index[midi] as usize][axis];
        let left = self.build(start, midi);
        let right = self.build(midi, end);
        let n = &mut self.nodes[id as usize];
        n.axis = axis as u8;
        n.split = split;
        n.left = left;
        n.right = right;
        id
    }

    /// The nearest point: (index, squared distance).
    pub fn nearest(&self, q: V3) -> Option<(u32, f64)> {
        let mut best = (u32::MAX, f64::INFINITY);
        if !self.nodes.is_empty() {
            self.nearest_in(0, q, &mut best);
        }
        (best.0 != u32::MAX).then_some(best)
    }

    fn nearest_in(&self, node: u32, q: V3, best: &mut (u32, f64)) {
        let n = &self.nodes[node as usize];
        if n.left == u32::MAX {
            for &i in &self.index[n.start as usize..n.end as usize] {
                let d = dist2(self.points[i as usize], q);
                if d < best.1 {
                    *best = (i, d);
                }
            }
            return;
        }
        let diff = q[n.axis as usize] - n.split;
        let (first, second) = if diff < 0.0 { (n.left, n.right) } else { (n.right, n.left) };
        self.nearest_in(first, q, best);
        if diff * diff < best.1 {
            self.nearest_in(second, q, best);
        }
    }

    /// The `k` nearest points, nearest first: (index, squared distance).
    pub fn knn(&self, q: V3, k: usize) -> Vec<(u32, f64)> {
        let mut heap: Vec<(u32, f64)> = Vec::with_capacity(k + 1);
        if !self.nodes.is_empty() && k > 0 {
            self.knn_in(0, q, k, &mut heap);
        }
        heap.sort_by(|a, b| a.1.total_cmp(&b.1));
        heap
    }

    fn knn_in(&self, node: u32, q: V3, k: usize, heap: &mut Vec<(u32, f64)>) {
        let n = &self.nodes[node as usize];
        let worst = |h: &Vec<(u32, f64)>| if h.len() < k { f64::INFINITY } else { h.iter().map(|e| e.1).fold(0.0, f64::max) };
        if n.left == u32::MAX {
            for &i in &self.index[n.start as usize..n.end as usize] {
                let d = dist2(self.points[i as usize], q);
                if heap.len() < k {
                    heap.push((i, d));
                } else if d < worst(heap) {
                    let w = heap.iter().enumerate().max_by(|a, b| a.1.1.total_cmp(&b.1.1)).unwrap().0;
                    heap[w] = (i, d);
                }
            }
            return;
        }
        let diff = q[n.axis as usize] - n.split;
        let (first, second) = if diff < 0.0 { (n.left, n.right) } else { (n.right, n.left) };
        self.knn_in(first, q, k, heap);
        if diff * diff < worst(heap) {
            self.knn_in(second, q, k, heap);
        }
    }

    /// Every point within `r` of `q`.
    pub fn within(&self, q: V3, r: f64, out: &mut Vec<u32>) {
        out.clear();
        if !self.nodes.is_empty() {
            self.within_in(0, q, r * r, out);
        }
    }

    fn within_in(&self, node: u32, q: V3, r2: f64, out: &mut Vec<u32>) {
        let n = &self.nodes[node as usize];
        if n.left == u32::MAX {
            for &i in &self.index[n.start as usize..n.end as usize] {
                if dist2(self.points[i as usize], q) <= r2 {
                    out.push(i);
                }
            }
            return;
        }
        let diff = q[n.axis as usize] - n.split;
        let (first, second) = if diff < 0.0 { (n.left, n.right) } else { (n.right, n.left) };
        self.within_in(first, q, r2, out);
        if diff * diff <= r2 {
            self.within_in(second, q, r2, out);
        }
    }
}

/// Where on a triangle the closest point lies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Feature {
    Face,
    /// The edge from corner k to corner k+1.
    Edge(u8),
    Vertex(u8),
}

/// The closest point of triangle abc to p, and on which feature.
pub fn closest_on_triangle(p: V3, a: V3, b: V3, c: V3) -> (V3, Feature) {
    // Ericson, Real-Time Collision Detection, 5.1.5.
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, Feature::Vertex(0));
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, Feature::Vertex(1));
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (add(a, scale(ab, v)), Feature::Edge(0));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, Feature::Vertex(2));
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (add(a, scale(ac, w)), Feature::Edge(2));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (add(b, scale(sub(c, b), w)), Feature::Edge(1));
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    (add(a, add(scale(ab, v), scale(ac, w))), Feature::Face)
}

/// A bounding-volume tree over a mesh's triangles.
pub struct TriBvh {
    pub mesh: Mesh,
    nodes: Vec<BvhNode>,
    order: Vec<u32>,
    face_normals: Vec<V3>,
    vertex_normals: Vec<V3>,
    edge_normals: HashMap<(u32, u32), V3>,
}

struct BvhNode {
    lo: V3,
    hi: V3,
    start: u32,
    end: u32,
    left: u32,
    right: u32,
}

/// The closest point of a mesh to a query.
#[derive(Clone, Copy, Debug)]
pub struct Closest {
    pub point: V3,
    pub triangle: u32,
    pub distance: f64,
    /// Negative inside a closed, outward-wound mesh (behind the surface).
    pub signed: f64,
}

impl TriBvh {
    pub fn new(mesh: &Mesh) -> TriBvh {
        let n = mesh.triangles.len();
        let face_normals: Vec<V3> = (0..n).map(|t| mesh.face_normal(t)).collect();
        let vertex_normals = mesh.vertex_normals();
        let mut edge_normals: HashMap<(u32, u32), V3> = HashMap::new();
        for (t, tri) in mesh.triangles.iter().enumerate() {
            for k in 0..3 {
                let e = crate::mesh::edge_key(tri[k], tri[(k + 1) % 3]);
                let s = edge_normals.entry(e).or_insert([0.0; 3]);
                *s = add(*s, face_normals[t]);
            }
        }
        let mut b = TriBvh { mesh: mesh.clone(), nodes: Vec::new(), order: (0..n as u32).collect(), face_normals, vertex_normals, edge_normals };
        if n > 0 {
            let centroids: Vec<V3> = (0..n).map(|t| {
                let [a, bb, c] = mesh.corners(t);
                scale(add(add(a, bb), c), 1.0 / 3.0)
            }).collect();
            b.build(0, n, &centroids);
        }
        b
    }

    fn bounds_of(&self, start: usize, end: usize) -> (V3, V3) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &t in &self.order[start..end] {
            for p in self.mesh.corners(t as usize) {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        (lo, hi)
    }

    fn build(&mut self, start: usize, end: usize, centroids: &[V3]) -> u32 {
        let (lo, hi) = self.bounds_of(start, end);
        let id = self.nodes.len() as u32;
        self.nodes.push(BvhNode { lo, hi, start: start as u32, end: end as u32, left: u32::MAX, right: u32::MAX });
        if end - start <= 4 {
            return id;
        }
        let axis = (0..3).max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b]))).unwrap();
        let mid = (start + end) / 2;
        self.order[start..end].select_nth_unstable_by(mid - start, |&a, &b| centroids[a as usize][axis].total_cmp(&centroids[b as usize][axis]));
        let left = self.build(start, mid, centroids);
        let right = self.build(mid, end, centroids);
        self.nodes[id as usize].left = left;
        self.nodes[id as usize].right = right;
        id
    }

    fn box_dist2(lo: V3, hi: V3, p: V3) -> f64 {
        let mut d = 0.0;
        for k in 0..3 {
            let v = if p[k] < lo[k] { lo[k] - p[k] } else if p[k] > hi[k] { p[k] - hi[k] } else { 0.0 };
            d += v * v;
        }
        d
    }

    /// The closest point on the mesh to `p`, with its signed distance.
    pub fn closest(&self, p: V3) -> Option<Closest> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best = (f64::INFINITY, 0u32, [0.0; 3], Feature::Face);
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if Self::box_dist2(node.lo, node.hi, p) >= best.0 {
                continue;
            }
            if node.left == u32::MAX {
                for &t in &self.order[node.start as usize..node.end as usize] {
                    let [a, b, c] = self.mesh.corners(t as usize);
                    let (q, f) = closest_on_triangle(p, a, b, c);
                    let d = dist2(p, q);
                    if d < best.0 {
                        best = (d, t, q, f);
                    }
                }
            } else {
                let (l, r) = (node.left, node.right);
                let dl = Self::box_dist2(self.nodes[l as usize].lo, self.nodes[l as usize].hi, p);
                let dr = Self::box_dist2(self.nodes[r as usize].lo, self.nodes[r as usize].hi, p);
                if dl < dr {
                    stack.push(r);
                    stack.push(l);
                } else {
                    stack.push(l);
                    stack.push(r);
                }
            }
        }
        let (d2, t, q, f) = best;
        let tri = self.mesh.triangles[t as usize];
        let normal = match f {
            Feature::Face => self.face_normals[t as usize],
            Feature::Edge(k) => self.edge_normals.get(&crate::mesh::edge_key(tri[k as usize], tri[(k as usize + 1) % 3])).copied().unwrap_or(self.face_normals[t as usize]),
            Feature::Vertex(k) => self.vertex_normals[tri[k as usize] as usize],
        };
        let d = d2.sqrt();
        let side = dot(sub(p, q), normal);
        Some(Closest { point: q, triangle: t, distance: d, signed: if side < 0.0 { -d } else { d } })
    }

    /// Triangles whose boxes overlap a box.
    pub fn overlapping(&self, lo: V3, hi: V3, out: &mut Vec<u32>) {
        out.clear();
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if (0..3).any(|k| node.lo[k] > hi[k] || node.hi[k] < lo[k]) {
                continue;
            }
            if node.left == u32::MAX {
                out.extend_from_slice(&self.order[node.start as usize..node.end as usize]);
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
    }
}

/// Whether two triangles cross (share no vertex positions; touching at a
/// point or along an edge counts only when they truly pass through).
pub fn triangles_intersect(a: [V3; 3], b: [V3; 3]) -> bool {
    // Each triangle's edges against the other triangle (segment-triangle,
    // strict interior), which catches every proper crossing.
    let seg_tri = |p: V3, q: V3, t: &[V3; 3]| -> bool {
        let e1 = sub(t[1], t[0]);
        let e2 = sub(t[2], t[0]);
        let d = sub(q, p);
        let h = cross(d, e2);
        let det = dot(e1, h);
        let scale_ = norm(e1) * norm(e2) * norm(d);
        if det.abs() <= 1e-12 * scale_ {
            return false;
        }
        let f = 1.0 / det;
        let s = sub(p, t[0]);
        let u = f * dot(s, h);
        let eps = 1e-9;
        if u <= eps || u >= 1.0 - eps {
            return false;
        }
        let qv = cross(s, e1);
        let v = f * dot(d, qv);
        if v <= eps || u + v >= 1.0 - eps {
            return false;
        }
        let w = f * dot(e2, qv);
        w > eps && w < 1.0 - eps
    };
    (0..3).any(|k| seg_tri(a[k], a[(k + 1) % 3], &b)) || (0..3).any(|k| seg_tri(b[k], b[(k + 1) % 3], &a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kd_tree_matches_brute_force() {
        let pts: Vec<V3> = (0..2000).map(|i| {
            let f = i as f64;
            [(f * 0.618).fract() * 10.0, (f * 0.414).fract() * 10.0, (f * 0.732).fract() * 10.0]
        }).collect();
        let t = KdTree::new(&pts);
        let q = [3.3, 4.4, 5.5];
        let brute = (0..pts.len()).min_by(|&a, &b| dist2(pts[a], q).total_cmp(&dist2(pts[b], q))).unwrap();
        assert_eq!(t.nearest(q).unwrap().0 as usize, brute);
        let k = t.knn(q, 10);
        assert_eq!(k[0].0 as usize, brute);
        assert_eq!(k.len(), 10);
        let mut w = Vec::new();
        t.within(q, 1.5, &mut w);
        assert_eq!(w.len(), pts.iter().filter(|p| dist2(**p, q) <= 2.25).count());
    }

    #[test]
    fn signed_distance_to_a_cube() {
        let b = TriBvh::new(&crate::mesh::cube(10.0, 10.0, 10.0));
        let c = b.closest([5.0, 5.0, 12.0]).unwrap();
        assert!((c.signed - 2.0).abs() < 1e-12);
        let c = b.closest([5.0, 5.0, 9.0]).unwrap();
        assert!((c.signed + 1.0).abs() < 1e-12);
        // Past a corner: outside, at the corner's distance.
        let c = b.closest([11.0, 11.0, 11.0]).unwrap();
        assert!((c.signed - 3f64.sqrt()).abs() < 1e-12);
        let c = b.closest([10.5, 5.0, 10.5]).unwrap();
        assert!(c.signed > 0.0);
    }

    #[test]
    fn crossing_triangles_are_found() {
        let a = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let b = [[0.5, 0.5, -1.0], [0.5, 0.5, 1.0], [1.5, 0.2, 0.0]];
        assert!(triangles_intersect(a, b));
        let c = [[0.5, 0.5, 0.5], [0.5, 0.5, 1.0], [1.5, 0.2, 0.7]];
        assert!(!triangles_intersect(a, c));
    }
}
