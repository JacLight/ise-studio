//! Isotropic remeshing (Botsch and Kobbelt 2004): long edges split, short
//! ones collapsed, edges swapped toward six neighbours a vertex, vertices
//! relaxed along the surface and pulled back onto the original. The result
//! has near-equilateral triangles of one size, as FEA and smoothing want.

use crate::decimate::Collapser;
use crate::linalg::*;
use crate::mesh::{Mesh, edge_key};
use crate::spatial::TriBvh;
use std::collections::{HashMap, HashSet};

/// Remeshes to triangles with edges about `target` long.
pub fn remesh(m: &Mesh, target: f64, iterations: usize) -> Mesh {
    let reference = TriBvh::new(m);
    let mut cur = m.clone();
    let (hi, lo) = (target * 4.0 / 3.0, target * 4.0 / 5.0);
    for _ in 0..iterations {
        split_long(&mut cur, hi);
        collapse_short(&mut cur, lo, hi);
        flip_valence(&mut cur);
        relax(&mut cur, &reference);
    }
    cur
}

fn boundary_vertices(m: &Mesh) -> Vec<bool> {
    let mut b = vec![false; m.positions.len()];
    for (e, f) in m.edge_faces() {
        if f.len() == 1 {
            b[e.0 as usize] = true;
            b[e.1 as usize] = true;
        }
    }
    b
}

/// Rim vertices and vertices on sharp edges (over 40° between faces): they
/// move only by splitting along their edge, so corners and creases stay crisp.
fn fixed_vertices(m: &Mesh) -> (Vec<bool>, HashSet<(u32, u32)>) {
    let mut b = vec![false; m.positions.len()];
    let mut sharp = HashSet::new();
    let cos = 40f64.to_radians().cos();
    for (e, f) in m.edge_faces() {
        let crease = f.len() != 2 || dot(m.face_normal(f[0] as usize), m.face_normal(f[1] as usize)) < cos;
        if crease {
            b[e.0 as usize] = true;
            b[e.1 as usize] = true;
            sharp.insert(e);
        }
    }
    (b, sharp)
}

/// Splits every edge longer than `hi` (repeatedly) at its middle.
pub fn split_long(m: &mut Mesh, hi: f64) {
    for _ in 0..12 {
        let ef = m.edge_faces();
        let mut long: Vec<((u32, u32), f64)> = ef
            .keys()
            .map(|&(a, b)| ((a, b), dist(m.positions[a as usize], m.positions[b as usize])))
            .filter(|(_, l)| *l > hi)
            .collect();
        if long.is_empty() {
            return;
        }
        long.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut touched = vec![false; m.triangles.len()];
        for ((a, b), _) in long {
            let faces = &ef[&(a, b)];
            if faces.iter().any(|&f| touched[f as usize]) {
                continue;
            }
            let v = m.positions.len() as u32;
            m.positions.push(mid(m.positions[a as usize], m.positions[b as usize]));
            for &f in faces {
                touched[f as usize] = true;
                let tri = m.triangles[f as usize];
                // Rotate so the split edge is tri[0]→tri[1].
                let k = (0..3).find(|&k| edge_key(tri[k], tri[(k + 1) % 3]) == (a, b)).unwrap();
                let (x, y, z) = (tri[k], tri[(k + 1) % 3], tri[(k + 2) % 3]);
                m.triangles[f as usize] = [x, v, z];
                m.triangles.push([v, y, z]);
                touched.push(true);
            }
        }
    }
}

fn collapse_short(m: &mut Mesh, lo: f64, hi: f64) {
    let (boundary, _) = fixed_vertices(m);
    let mut c = Collapser::new(m);
    let mut edges: Vec<((u32, u32), f64)> = m.edge_faces().keys().map(|&(a, b)| ((a, b), dist(m.positions[a as usize], m.positions[b as usize]))).filter(|(_, l)| *l < lo).collect();
    edges.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut dead = HashSet::new();
    for ((a, b), _) in edges {
        if dead.contains(&a) || dead.contains(&b) {
            continue;
        }
        let (pa, pb) = (c.pos[a as usize], c.pos[b as usize]);
        if dist(pa, pb) >= lo {
            continue;
        }
        // Rim vertices stay put: collapse into them, never two rims together.
        let (u, v, at) = match (boundary[a as usize], boundary[b as usize]) {
            (true, true) => continue,
            (true, false) => (a, b, pa),
            (false, true) => (b, a, pb),
            (false, false) => (a, b, mid(pa, pb)),
        };
        // No edge may come out too long.
        if c.neighbours(v).iter().chain(c.neighbours(u).iter()).any(|&w| w != u && w != v && dist(c.pos[w as usize], at) > hi) {
            continue;
        }
        if !c.can_collapse(u, v, at, 0.5) {
            continue;
        }
        c.collapse(u, v, at);
        dead.insert(v);
    }
    *m = c.finish();
}

fn flip_valence(m: &mut Mesh) {
    let boundary = boundary_vertices(m);
    let (_, sharp) = fixed_vertices(m);
    let mut valence = vec![0i32; m.positions.len()];
    let mut edge_tris: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (t, tri) in m.triangles.iter().enumerate() {
        for k in 0..3 {
            edge_tris.entry(edge_key(tri[k], tri[(k + 1) % 3])).or_default().push(t);
        }
    }
    for &(a, b) in edge_tris.keys() {
        valence[a as usize] += 1;
        valence[b as usize] += 1;
    }
    let ideal = |v: u32| if boundary[v as usize] { 4 } else { 6 };
    let keys: Vec<(u32, u32)> = edge_tris.keys().copied().collect();
    for (a, b) in keys {
        let Some(faces) = edge_tris.get(&(a, b)).cloned() else { continue };
        if faces.len() != 2 || sharp.contains(&(a, b)) {
            continue;
        }
        let (t0, t1) = (faces[0], faces[1]);
        let tri0 = m.triangles[t0];
        let tri1 = m.triangles[t1];
        if !tri0.contains(&a) || !tri0.contains(&b) || !tri1.contains(&a) || !tri1.contains(&b) {
            continue;
        }
        let c = *tri0.iter().find(|&&v| v != a && v != b).unwrap();
        let d = *tri1.iter().find(|&&v| v != a && v != b).unwrap();
        if c == d || edge_tris.contains_key(&edge_key(c, d)) {
            continue;
        }
        let dev = |v: u32, delta: i32| ((valence[v as usize] + delta) - ideal(v)).abs();
        let before = dev(a, 0) + dev(b, 0) + dev(c, 0) + dev(d, 0);
        let after = dev(a, -1) + dev(b, -1) + dev(c, 1) + dev(d, 1);
        if after >= before {
            continue;
        }
        let forward = (0..3).any(|k| tri0[k] == a && tri0[(k + 1) % 3] == b);
        let (a2, b2) = if forward { (a, b) } else { (b, a) };
        let n0 = [a2, d, c];
        let n1 = [b2, c, d];
        // The swap must not fold the surface.
        let p = |t: [u32; 3]| t.map(|x| m.positions[x as usize]);
        let old = add(tri_normal(p(tri0)[0], p(tri0)[1], p(tri0)[2]), tri_normal(p(tri1)[0], p(tri1)[1], p(tri1)[2]));
        let (q0, q1) = (p(n0), p(n1));
        let (m0, m1) = (tri_normal(q0[0], q0[1], q0[2]), tri_normal(q1[0], q1[1], q1[2]));
        if dot(m0, old) <= 0.0 || dot(m1, old) <= 0.0 || dot(unit(m0), unit(m1)) < 0.5 {
            continue;
        }
        m.triangles[t0] = n0;
        m.triangles[t1] = n1;
        edge_tris.remove(&(a, b));
        edge_tris.insert(edge_key(c, d), vec![t0, t1]);
        for (e, from, to) in [(edge_key(a2, d), t1, t0), (edge_key(b2, c), t0, t1)] {
            if let Some(list) = edge_tris.get_mut(&e) {
                for x in list.iter_mut() {
                    if *x == from {
                        *x = to;
                    }
                }
            }
        }
        valence[a as usize] -= 1;
        valence[b as usize] -= 1;
        valence[c as usize] += 1;
        valence[d as usize] += 1;
    }
}

fn relax(m: &mut Mesh, reference: &TriBvh) {
    let (boundary, _) = fixed_vertices(m);
    let nb = m.vertex_neighbours();
    let normals = m.vertex_normals();
    let moved: Vec<_> = (0..m.positions.len())
        .map(|v| {
            let p = m.positions[v];
            if boundary[v] || nb[v].is_empty() {
                return p;
            }
            let avg = scale(nb[v].iter().fold([0.0; 3], |s, &u| add(s, m.positions[u as usize])), 1.0 / nb[v].len() as f64);
            let d = sub(avg, p);
            let n = normals[v];
            let q = add(p, sub(d, scale(n, dot(d, n))));
            reference.closest(q).map(|c| c.point).unwrap_or(q)
        })
        .collect();
    // A move that would tip one of the vertex's triangles over is not made.
    let vf = m.vertex_faces();
    for v in 0..m.positions.len() {
        if moved[v] == m.positions[v] {
            continue;
        }
        let ok = vf[v].iter().all(|&t| {
            let tri = m.triangles[t as usize];
            let old = tri.map(|x| m.positions[x as usize]);
            let new = tri.map(|x| if x as usize == v { moved[v] } else { m.positions[x as usize] });
            let (a, b) = (tri_normal(old[0], old[1], old[2]), tri_normal(new[0], new[1], new[2]));
            dot(unit(a), unit(b)) > 0.5
        });
        if ok {
            m.positions[v] = moved[v];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::analyze;
    use crate::mesh::cylinder;

    #[test]
    fn a_cylinder_of_slivers_becomes_even_triangles_on_the_same_surface() {
        // Long thin side triangles and fans on the caps.
        let m = cylinder(10.0, 30.0, 48, 1);
        let r = remesh(&m, 2.0, 5);
        let rep = analyze(&r);
        assert!(rep.solid, "{rep:?}");
        let lens: Vec<f64> = r.triangles.iter().flat_map(|t| (0..3).map(move |k| (t[k], t[(k + 1) % 3]))).map(|(a, b)| dist(r.positions[a as usize], r.positions[b as usize])).collect();
        let mean = lens.iter().sum::<f64>() / lens.len() as f64;
        let within = lens.iter().filter(|l| **l > 2.0 * 0.6 && **l < 2.0 * 1.4).count() as f64 / lens.len() as f64;
        assert!((mean - 2.0).abs() < 0.3, "mean edge {mean}");
        assert!(within > 0.9, "{:.1}% of edges near the target", within * 100.0);
        // On the original surface.
        let bvh = TriBvh::new(&m);
        let worst = r.positions.iter().map(|p| bvh.closest(*p).unwrap().distance).fold(0.0, f64::max);
        assert!(worst < 1e-6, "{worst}");
        assert!((r.volume() - m.volume()).abs() / m.volume() < 0.01);
    }
}
