//! Mesh evaluation, as FreeCAD's Mesh_Evaluation: everything that keeps a
//! mesh from being a clean closed solid, counted and located.

use crate::V3;
use crate::linalg::*;
use crate::mesh::{Mesh, edge_key};
use crate::spatial::{TriBvh, triangles_intersect};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// What is wrong with a mesh (and what is right).
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Report {
    pub vertices: usize,
    pub triangles: usize,
    /// Separate pieces (connected through edges).
    pub components: usize,
    /// Holes: loops of open edges.
    pub holes: usize,
    /// Edges used by one triangle only.
    pub open_edges: usize,
    /// Edges shared by three or more triangles.
    pub non_manifold_edges: usize,
    /// Vertices where separate fans of triangles meet at a point.
    pub non_manifold_vertices: usize,
    /// Triangles wound against their neighbours (turned inside out),
    /// counted on the smaller side of each disagreement.
    pub flipped: usize,
    /// Triangles with no area (collapsed to a line or point).
    pub degenerate: usize,
    /// Triangles that repeat another (same three vertices).
    pub duplicate: usize,
    /// Pairs of triangles passing through each other.
    pub self_intersections: usize,
    /// The self-intersection search stopped early (very large meshes).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub intersections_capped: bool,
    /// Vertices no triangle uses.
    pub unused_vertices: usize,
    /// Closed, manifold, consistently wound, no self-intersections: a solid.
    pub solid: bool,
    pub area: f64,
    /// Signed volume (meaningful when closed).
    pub volume: f64,
    pub bounds: [f64; 6],
    /// Where the problems are, for the view: a point per item (capped).
    pub at: Problems,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Problems {
    /// Each hole's rim as a closed polyline.
    pub holes: Vec<Vec<V3>>,
    pub non_manifold_edges: Vec<[V3; 2]>,
    pub non_manifold_vertices: Vec<V3>,
    pub flipped: Vec<V3>,
    pub degenerate: Vec<V3>,
    pub self_intersections: Vec<V3>,
}

const SHOW: usize = 500;

/// Triangles with (relative) zero area.
pub fn degenerate_faces(m: &Mesh) -> Vec<u32> {
    let mut out = Vec::new();
    for t in 0..m.triangles.len() {
        let tri = m.triangles[t];
        if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
            out.push(t as u32);
            continue;
        }
        let [a, b, c] = m.corners(t);
        let l = dist(a, b).max(dist(b, c)).max(dist(a, c));
        if norm(m.face_cross(t)) <= 1e-10 * l * l {
            out.push(t as u32);
        }
    }
    out
}

/// Triangles repeating an earlier one (any rotation or winding).
pub fn duplicate_faces(m: &Mesh) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (t, tri) in m.triangles.iter().enumerate() {
        let mut k = *tri;
        k.sort_unstable();
        if !seen.insert(k) {
            out.push(t as u32);
        }
    }
    out
}

/// Vertices where two or more separate fans meet.
pub fn non_manifold_vertices(m: &Mesh) -> Vec<u32> {
    let vf = m.vertex_faces();
    let mut out = Vec::new();
    for (v, faces) in vf.iter().enumerate() {
        if faces.len() < 2 {
            continue;
        }
        // Fans: faces around v connected through edges that contain v.
        let mut group: HashMap<u32, u32> = faces.iter().map(|&f| (f, f)).collect();
        fn find(g: &mut HashMap<u32, u32>, x: u32) -> u32 {
            let p = g[&x];
            if p == x {
                return x;
            }
            let r = find(g, p);
            g.insert(x, r);
            r
        }
        let mut by_other: HashMap<u32, Vec<u32>> = HashMap::new();
        for &f in faces {
            for &u in &m.triangles[f as usize] {
                if u as usize != v {
                    by_other.entry(u).or_default().push(f);
                }
            }
        }
        for fs in by_other.values() {
            for w in fs.windows(2) {
                let (a, b) = (find(&mut group, w[0]), find(&mut group, w[1]));
                if a != b {
                    group.insert(a, b);
                }
            }
        }
        let roots: HashSet<u32> = faces.iter().map(|&f| find(&mut group, f)).collect();
        if roots.len() > 1 {
            out.push(v as u32);
        }
    }
    out
}

/// Triangles wound against the majority of their component: a
/// breadth-first walk agrees each neighbour with the last, and the smaller
/// camp of each component is the flipped one.
pub fn flipped_faces(m: &Mesh) -> Vec<u32> {
    let ef = m.edge_faces();
    let n = m.triangles.len();
    let mut side = vec![0i8; n];
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    for s in 0..n {
        if side[s] != 0 {
            continue;
        }
        side[s] = 1;
        queue.push_back(s as u32);
        let mut members = vec![s as u32];
        while let Some(t) = queue.pop_front() {
            let tri = m.triangles[t as usize];
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let Some(faces) = ef.get(&edge_key(a, b)) else { continue };
                if faces.len() != 2 {
                    continue;
                }
                let u = if faces[0] == t { faces[1] } else { faces[0] };
                if side[u as usize] != 0 {
                    continue;
                }
                // Neighbours agree when they run the shared edge opposite ways.
                let ut = m.triangles[u as usize];
                let same_dir = (0..3).any(|j| ut[j] == a && ut[(j + 1) % 3] == b);
                side[u as usize] = if same_dir { -side[t as usize] } else { side[t as usize] };
                members.push(u);
                queue.push_back(u);
            }
        }
        let neg = members.iter().filter(|&&t| side[t as usize] < 0).count();
        let flip_side = if neg * 2 <= members.len() { -1 } else { 1 };
        out.extend(members.into_iter().filter(|&t| side[t as usize] == flip_side));
    }
    out
}

/// Pairs of triangles that pass through each other (neighbours sharing a
/// vertex are skipped). Stops at `cap` pairs.
pub fn self_intersections(m: &Mesh, cap: usize) -> (Vec<(u32, u32)>, bool) {
    let bvh = TriBvh::new(m);
    let found: Vec<(u32, u32)> = (0..m.triangles.len())
        .into_par_iter()
        .flat_map_iter(|t| {
            let tri = m.triangles[t];
            let c = m.corners(t);
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for p in c {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let mut cand = Vec::new();
            bvh.overlapping(lo, hi, &mut cand);
            cand.into_iter()
                .filter(move |&u| (u as usize) > t)
                .filter(move |&u| {
                    let ut = m.triangles[u as usize];
                    !ut.iter().any(|v| tri.contains(v))
                })
                .filter(move |&u| triangles_intersect(c, m.corners(u as usize)))
                .map(move |u| (t as u32, u))
                .collect::<Vec<_>>()
        })
        .collect();
    let capped = found.len() > cap;
    (found.into_iter().take(cap).collect(), capped)
}

/// The full evaluation.
pub fn analyze(m: &Mesh) -> Report {
    let ef = m.edge_faces();
    let open: Vec<(u32, u32)> = ef.iter().filter(|(_, f)| f.len() == 1).map(|(e, _)| *e).collect();
    let nm_edges: Vec<(u32, u32)> = ef.iter().filter(|(_, f)| f.len() > 2).map(|(e, _)| *e).collect();
    let loops = m.boundary_loops();
    let nm_vertices = non_manifold_vertices(m);
    let flipped = flipped_faces(m);
    let degenerate = degenerate_faces(m);
    let duplicate = duplicate_faces(m);
    let (inter, capped) = self_intersections(m, 100_000);
    let used: HashSet<u32> = m.triangles.iter().flatten().copied().collect();
    let (_, components) = m.components();
    let (lo, hi) = m.bounds();
    let p = |v: u32| m.positions[v as usize];
    let centre = |t: u32| {
        let [a, b, c] = m.corners(t as usize);
        scale(add(add(a, b), c), 1.0 / 3.0)
    };
    let solid = !m.triangles.is_empty() && open.is_empty() && nm_edges.is_empty() && nm_vertices.is_empty() && flipped.is_empty() && inter.is_empty() && degenerate.is_empty() && duplicate.is_empty();
    Report {
        vertices: m.positions.len(),
        triangles: m.triangles.len(),
        components,
        holes: loops.len(),
        open_edges: open.len(),
        non_manifold_edges: nm_edges.len(),
        non_manifold_vertices: nm_vertices.len(),
        flipped: flipped.len(),
        degenerate: degenerate.len(),
        duplicate: duplicate.len(),
        self_intersections: inter.len(),
        intersections_capped: capped,
        unused_vertices: m.positions.len() - used.len(),
        solid,
        area: m.area(),
        volume: m.volume(),
        bounds: [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]],
        at: Problems {
            holes: loops.iter().take(SHOW).map(|l| l.iter().map(|&v| p(v)).collect()).collect(),
            non_manifold_edges: nm_edges.iter().take(SHOW).map(|&(a, b)| [p(a), p(b)]).collect(),
            non_manifold_vertices: nm_vertices.iter().take(SHOW).map(|&v| p(v)).collect(),
            flipped: flipped.iter().take(SHOW).map(|&t| centre(t)).collect(),
            degenerate: degenerate.iter().take(SHOW).map(|&t| centre(t)).collect(),
            self_intersections: inter.iter().take(SHOW).map(|&(a, b)| mid(centre(a), centre(b))).collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    #[test]
    fn a_clean_cube_is_a_solid() {
        let r = analyze(&cube(10.0, 10.0, 10.0));
        assert!(r.solid, "{r:?}");
        assert_eq!((r.holes, r.flipped, r.components), (0, 0, 1));
        assert!((r.volume - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn every_defect_is_counted() {
        let mut m = cube(10.0, 10.0, 10.0);
        // A hole: one triangle gone.
        m.triangles.remove(2);
        // A flipped triangle.
        m.triangles[0].swap(1, 2);
        // A duplicate and a degenerate one.
        m.triangles.push(m.triangles[5]);
        m.positions.push([5.0, 5.0, 5.0]);
        m.triangles.push([0, 1, 1]);
        // A second piece crossing the first.
        let mut other = cube(4.0, 4.0, 20.0);
        other.transform(&[[1.0, 0.0, 0.0, 3.0], [0.0, 1.0, 0.0, 3.0], [0.0, 0.0, 1.0, -5.0]]);
        m.merge(&other);
        let r = analyze(&m);
        assert_eq!(r.holes, 1, "{r:?}");
        assert_eq!(r.flipped, 1);
        assert_eq!(r.duplicate, 1);
        assert_eq!(r.degenerate, 1);
        assert!(r.non_manifold_edges >= 1, "the duplicate makes an edge with three faces");
        assert!(r.self_intersections > 0);
        assert_eq!(r.components, 2);
        assert_eq!(r.unused_vertices, 1);
        assert!(!r.solid);
    }
}
