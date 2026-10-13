//! Hole filling after Liepa (2003), "Filling holes in meshes": each loop of
//! open edges is triangulated (minimum area and dihedral, by dynamic
//! programming, for loops up to a few dozen edges; smallest-angle ear
//! clipping beyond), then refined to the rim's edge length and faired by
//! relaxing the new vertices, so a patched scan has no flat lid.

use crate::V3;
use crate::linalg::*;
use crate::mesh::{Mesh, edge_key};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Filled {
    pub filled: usize,
    /// Holes left because their rim has more edges than allowed.
    pub skipped: usize,
    pub triangles_added: usize,
}

/// Fills every hole whose rim has at most `max_edges` edges.
pub fn fill_holes(m: &mut Mesh, max_edges: usize, refine: bool) -> Filled {
    let loops = m.boundary_loops();
    let mut out = Filled::default();
    for lp in loops {
        if lp.len() < 3 {
            continue;
        }
        if lp.len() > max_edges {
            out.skipped += 1;
            continue;
        }
        let before = m.triangles.len();
        let patch = triangulate(m, &lp);
        let first = m.triangles.len();
        m.triangles.extend(patch);
        if refine && lp.len() > 3 {
            refine_patch(m, &lp, first);
        }
        out.triangles_added += m.triangles.len() - before;
        out.filled += 1;
    }
    out
}

/// The triangles closing one loop (wound to match the rim).
fn triangulate(m: &Mesh, lp: &[u32]) -> Vec<[u32; 3]> {
    if lp.len() <= 60 { min_weight(m, lp) } else { ear_clip(m, lp) }
}

/// The normal of the rim's triangle at each loop edge (for the dihedral term).
fn rim_normals(m: &Mesh, lp: &[u32]) -> HashMap<(u32, u32), V3> {
    let mut out = HashMap::new();
    let ef = m.edge_faces();
    for i in 0..lp.len() {
        let e = edge_key(lp[i], lp[(i + 1) % lp.len()]);
        if let Some(f) = ef.get(&e).and_then(|f| f.first()) {
            out.insert(e, m.face_normal(*f as usize));
        }
    }
    out
}

fn min_weight(m: &Mesh, lp: &[u32]) -> Vec<[u32; 3]> {
    let n = lp.len();
    let p = |i: usize| m.positions[lp[i] as usize];
    let rim = rim_normals(m, lp);
    // Best (dihedral, area) for each sub-polygon i..j, and its split.
    let mut w = vec![(0.0f64, 0.0f64); n * n];
    let mut split = vec![usize::MAX; n * n];
    let normal_of = |i: usize, k: usize, j: usize| unit(tri_normal(p(i), p(k), p(j)));
    for len in 2..n {
        for i in 0..n - len {
            let j = i + len;
            let mut best = (f64::INFINITY, f64::INFINITY);
            let mut arg = usize::MAX;
            for k in i + 1..j {
                let nrm = normal_of(i, k, j);
                let area = norm(tri_normal(p(i), p(k), p(j))) / 2.0;
                // The worst bend against the neighbours this triangle meets.
                let mut bend: f64 = 0.0;
                for (a, b) in [(i, k), (k, j)] {
                    let other = if b == a + 1 {
                        rim.get(&edge_key(lp[a], lp[b])).copied()
                    } else {
                        let s = split[a * n + b];
                        (s != usize::MAX).then(|| normal_of(a, s, b))
                    };
                    if let Some(o) = other {
                        bend = bend.max(1.0 - dot(nrm, o));
                    }
                }
                if j == n - 1 && i == 0 {
                    if let Some(o) = rim.get(&edge_key(lp[j], lp[i])) {
                        bend = bend.max(1.0 - dot(nrm, *o));
                    }
                }
                let left = w[i * n + k];
                let right = w[k * n + j];
                let cand = (bend.max(left.0).max(right.0), area + left.1 + right.1);
                // Bends within a whisker count as equal; then the smaller area wins.
                let better = cand.0 < best.0 - 1e-6 || ((cand.0 - best.0).abs() <= 1e-6 && cand.1 < best.1);
                if better {
                    best = cand;
                    arg = k;
                }
            }
            w[i * n + j] = best;
            split[i * n + j] = arg;
        }
    }
    let mut out = Vec::new();
    let mut stack = vec![(0usize, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let k = split[i * n + j];
        if k == usize::MAX {
            continue;
        }
        out.push([lp[i], lp[k], lp[j]]);
        stack.push((i, k));
        stack.push((k, j));
    }
    out
}

/// Advancing front: repeatedly cut off the corner with the smallest angle.
fn ear_clip(m: &Mesh, lp: &[u32]) -> Vec<[u32; 3]> {
    let mut ring: Vec<u32> = lp.to_vec();
    let mut out = Vec::new();
    // The loop's mean plane normal, to tell convex corners from reflex ones.
    let c = scale(ring.iter().fold([0.0; 3], |s, &v| add(s, m.positions[v as usize])), 1.0 / ring.len() as f64);
    let mut nrm = [0.0; 3];
    for i in 0..ring.len() {
        let (a, b) = (m.positions[ring[i] as usize], m.positions[ring[(i + 1) % ring.len()] as usize]);
        nrm = add(nrm, cross(sub(a, c), sub(b, c)));
    }
    let nrm = unit(nrm);
    while ring.len() > 3 {
        let n = ring.len();
        let mut best = (f64::INFINITY, 0usize);
        for i in 0..n {
            let (a, b, cc) = (m.positions[ring[(i + n - 1) % n] as usize], m.positions[ring[i] as usize], m.positions[ring[(i + 1) % n] as usize]);
            let u = unit(sub(a, b));
            let v = unit(sub(cc, b));
            let mut angle = dot(u, v).clamp(-1.0, 1.0).acos();
            // Corners turning the other way are reflex.
            if dot(cross(sub(b, a), sub(cc, b)), nrm) < 0.0 {
                angle = std::f64::consts::TAU - angle;
            }
            if angle < best.0 {
                best = (angle, i);
            }
        }
        let i = best.1;
        out.push([ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]]);
        ring.remove(i);
    }
    out.push([ring[0], ring[1], ring[2]]);
    out
}

/// Splits the patch's large triangles at their centres until they match the
/// rim's edges, swaps edges to keep them well shaped, then relaxes the new
/// vertices so the patch continues the surface around it smoothly.
fn refine_patch(m: &mut Mesh, lp: &[u32], first: usize) {
    let n = lp.len();
    let rim_len: f64 = (0..n).map(|i| dist(m.positions[lp[i] as usize], m.positions[lp[(i + 1) % n] as usize])).sum::<f64>() / n as f64;
    let target = rim_len * rim_len * 3f64.sqrt() / 4.0;
    let first_vertex = m.positions.len();
    for _ in 0..12 {
        let mut split_any = false;
        let count = m.triangles.len();
        for t in first..count {
            if m.face_area(t) > 2.0 * target {
                let [a, b, c] = m.triangles[t];
                let centre = scale(add(add(m.positions[a as usize], m.positions[b as usize]), m.positions[c as usize]), 1.0 / 3.0);
                let v = m.positions.len() as u32;
                m.positions.push(centre);
                m.triangles[t] = [a, b, v];
                m.triangles.push([b, c, v]);
                m.triangles.push([c, a, v]);
                split_any = true;
            }
        }
        relax_edges(m, first);
        if !split_any {
            break;
        }
    }
    if first_vertex == m.positions.len() {
        return;
    }
    // Fair: first a membrane (umbrella relaxation, rim fixed), then thin-plate
    // (Kobbelt's umbrella² iteration) over the whole mesh's neighbourhoods, so
    // the patch carries on the curvature and slope around the hole.
    let nb = m.vertex_neighbours();
    let umbrella = |pos: &[V3], v: usize| -> V3 {
        let ns = &nb[v];
        if ns.is_empty() {
            return [0.0; 3];
        }
        sub(scale(ns.iter().fold([0.0; 3], |s, &u| add(s, pos[u as usize])), 1.0 / ns.len() as f64), pos[v])
    };
    for _ in 0..80 {
        for v in first_vertex..m.positions.len() {
            let u = umbrella(&m.positions, v);
            m.positions[v] = add(m.positions[v], u);
        }
    }
    // Thin plate: the new vertices placed so the umbrella (discrete
    // Laplacian) of every vertex they touch, rim and ring included, is as
    // small as it can be in the least-squares sense; one sparse solve per axis.
    let free = m.positions.len() - first_vertex;
    let var = |v: usize| (v >= first_vertex).then(|| v - first_vertex);
    let mut rows: Vec<usize> = (first_vertex..m.positions.len()).collect();
    for v in first_vertex..m.positions.len() {
        rows.extend(nb[v].iter().map(|&u| u as usize));
    }
    rows.sort_unstable();
    rows.dedup();
    let mut a = crate::linalg::Sparse::new(free);
    let mut rhs = vec![[0.0; 3]; free];
    for &r in &rows {
        let ns = &nb[r];
        if ns.is_empty() {
            continue;
        }
        let w = 1.0 / ns.len() as f64;
        // U(r) = Σ w·x_u − x_r: split into unknowns and known positions.
        let mut coeffs: Vec<(usize, f64)> = Vec::new();
        let mut known = [0.0; 3];
        for &u in ns {
            match var(u as usize) {
                Some(i) => coeffs.push((i, w)),
                None => known = add(known, scale(m.positions[u as usize], w)),
            }
        }
        match var(r) {
            Some(i) => coeffs.push((i, -1.0)),
            None => known = sub(known, m.positions[r]),
        }
        for &(i, ci) in &coeffs {
            rhs[i] = sub(rhs[i], scale(known, ci));
            for &(j, cj) in &coeffs {
                a.add(i, j, ci * cj);
            }
        }
    }
    for k in 0..3 {
        let b: Vec<f64> = rhs.iter().map(|v| v[k]).collect();
        let mut x: Vec<f64> = (first_vertex..m.positions.len()).map(|v| m.positions[v][k]).collect();
        a.solve_cg(&b, &mut x, 1e-10, 20 * free.max(50));
        for (i, v) in x.into_iter().enumerate() {
            m.positions[first_vertex + i][k] = v;
        }
    }
}

/// Edge swaps inside the patch (from `first`) wherever they make the
/// triangles' opposite angles sum below 180° (Delaunay).
fn relax_edges(m: &mut Mesh, first: usize) {
    for _ in 0..4 {
        let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for t in first..m.triangles.len() {
            let tri = m.triangles[t];
            for k in 0..3 {
                edges.entry(edge_key(tri[k], tri[(k + 1) % 3])).or_default().push(t);
            }
        }
        let mut changed = false;
        let mut touched = vec![false; m.triangles.len()];
        for (&(a, b), faces) in &edges {
            if faces.len() != 2 || touched[faces[0]] || touched[faces[1]] {
                continue;
            }
            let (t0, t1) = (faces[0], faces[1]);
            let c = *m.triangles[t0].iter().find(|&&v| v != a && v != b).unwrap();
            let d = *m.triangles[t1].iter().find(|&&v| v != a && v != b).unwrap();
            if c == d || edges.contains_key(&edge_key(c, d)) {
                continue;
            }
            let p = |v: u32| m.positions[v as usize];
            let angle = |o: u32| {
                let (u, w) = (unit(sub(p(a), p(o))), unit(sub(p(b), p(o))));
                dot(u, w).clamp(-1.0, 1.0).acos()
            };
            if angle(c) + angle(d) <= std::f64::consts::PI + 1e-9 {
                continue;
            }
            // Keep the winding: t0 runs a→b or b→a.
            let tri = m.triangles[t0];
            let forward = (0..3).any(|k| tri[k] == a && tri[(k + 1) % 3] == b);
            let (a, b) = if forward { (a, b) } else { (b, a) };
            m.triangles[t0] = [a, d, c];
            m.triangles[t1] = [b, c, d];
            touched[t0] = true;
            touched[t1] = true;
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::analyze;
    use crate::mesh::uv_sphere;

    #[test]
    fn a_sphere_with_a_cap_cut_off_closes_again_and_keeps_its_volume() {
        let full = uv_sphere(20.0, 40, 64);
        let mut m = full.clone();
        // Cut the cap above z = 17 (a 64-edge hole).
        m.triangles.retain(|t| t.iter().any(|&v| full.positions[v as usize][2] < 17.0));
        m.compact();
        let r = analyze(&m);
        assert_eq!(r.holes, 1);
        let f = fill_holes(&mut m, 1000, true);
        assert_eq!((f.filled, f.skipped), (1, 0));
        let r = analyze(&m);
        assert!(r.solid, "{r:?}");
        // The refined, faired patch lands within 1% of the true volume.
        let want = full.volume();
        let err = (m.volume() - want).abs() / want;
        assert!(err < 0.01, "volume off by {:.3}%", err * 100.0);
    }

    #[test]
    fn small_holes_fill_by_the_minimum_weight_triangulation() {
        let mut m = crate::mesh::cube(10.0, 10.0, 10.0);
        m.triangles.truncate(10);
        let f = fill_holes(&mut m, 100, false);
        assert_eq!(f.filled, 1);
        let r = analyze(&m);
        assert!(r.solid, "{r:?}");
        assert!((m.volume() - 1000.0).abs() < 1e-9);
    }
}
