//! Mesh repair, as FreeCAD's Mesh repair commands: duplicate and degenerate
//! triangles removed, non-manifold edges and vertices untangled, normals
//! made to agree and point outward, stray pieces dropped, self-crossing
//! triangles cut out, holes filled. Each step reports what it changed.

use crate::analyze::{self, Report};
use crate::mesh::{Mesh, edge_key};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// What a repair does (the default: everything, holes up to 2000 edges).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    /// Vertices closer than this (mm) are one (0: a millionth of the size).
    pub weld: f64,
    pub degenerate: bool,
    pub duplicates: bool,
    pub non_manifold: bool,
    /// Wind every triangle the same way as its neighbours, outward.
    pub harmonize: bool,
    /// Turn everything inside out (after harmonizing).
    pub flip: bool,
    /// Drop pieces with fewer triangles than this fraction of the largest (0: keep all).
    pub min_component: f64,
    /// Keep only the largest piece.
    pub largest_only: bool,
    /// Cut out triangles that pass through others (the holes left are filled).
    pub self_intersections: bool,
    /// Fill holes whose rim has at most this many edges (0: leave holes).
    pub fill_holes: usize,
    /// Refine and fair the patches (else flat lids).
    pub fair: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { weld: 0.0, degenerate: true, duplicates: true, non_manifold: true, harmonize: true, flip: false, min_component: 0.0, largest_only: false, self_intersections: true, fill_holes: 2000, fair: true }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Repaired {
    pub before: Report,
    pub after: Report,
    /// Each step that changed something, and how many items.
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Step {
    pub step: String,
    pub count: usize,
}

pub fn remove_degenerate(m: &mut Mesh) -> usize {
    let bad: HashSet<u32> = analyze::degenerate_faces(m).into_iter().collect();
    drop_faces(m, &bad)
}

pub fn remove_duplicates(m: &mut Mesh) -> usize {
    let bad: HashSet<u32> = analyze::duplicate_faces(m).into_iter().collect();
    drop_faces(m, &bad)
}

fn drop_faces(m: &mut Mesh, bad: &HashSet<u32>) -> usize {
    let before = m.triangles.len();
    let mut t = 0u32;
    m.triangles.retain(|_| {
        let keep = !bad.contains(&t);
        t += 1;
        keep
    });
    before - m.triangles.len()
}

/// Edges with three or more triangles keep the two that face each other
/// best; vertices joining separate fans are split into one vertex per fan.
pub fn fix_non_manifold(m: &mut Mesh) -> usize {
    let mut fixed = 0;
    let ef = m.edge_faces();
    let mut bad = HashSet::new();
    for faces in ef.values().filter(|f| f.len() > 2) {
        // Keep the largest triangle and the one bending least from it, opposite in direction.
        let mut by_area: Vec<u32> = faces.clone();
        by_area.sort_by(|a, b| m.face_area(*b as usize).total_cmp(&m.face_area(*a as usize)));
        let keep0 = by_area[0];
        let n0 = m.face_normal(keep0 as usize);
        let keep1 = by_area[1..].iter().copied().max_by(|a, b| crate::linalg::dot(n0, m.face_normal(*a as usize)).total_cmp(&crate::linalg::dot(n0, m.face_normal(*b as usize)))).unwrap();
        for &f in faces {
            if f != keep0 && f != keep1 && bad.insert(f) {
                fixed += 1;
            }
        }
    }
    drop_faces(m, &bad);
    let nm = analyze::non_manifold_vertices(m);
    if !nm.is_empty() {
        let vf = m.vertex_faces();
        for v in nm {
            // Fans around v, through edges that contain v.
            let faces = &vf[v as usize];
            let mut fan_of: HashMap<u32, usize> = HashMap::new();
            let mut fans = 0;
            for &start in faces {
                if fan_of.contains_key(&start) {
                    continue;
                }
                let mut stack = vec![start];
                fan_of.insert(start, fans);
                while let Some(f) = stack.pop() {
                    for &g in faces {
                        if fan_of.contains_key(&g) {
                            continue;
                        }
                        let shared = m.triangles[f as usize].iter().filter(|&&u| u != v && m.triangles[g as usize].contains(&u)).count();
                        if shared > 0 {
                            fan_of.insert(g, fans);
                            stack.push(g);
                        }
                    }
                }
                fans += 1;
            }
            for fan in 1..fans {
                let copy = m.positions.len() as u32;
                m.positions.push(m.positions[v as usize]);
                for (&f, &k) in &fan_of {
                    if k == fan {
                        for u in m.triangles[f as usize].iter_mut() {
                            if *u == v {
                                *u = copy;
                            }
                        }
                    }
                }
            }
            fixed += 1;
        }
    }
    fixed
}

/// Every triangle wound as its neighbours (each piece by its majority), and
/// closed pieces turned outward. Returns the triangles turned.
pub fn harmonize_normals(m: &mut Mesh) -> usize {
    let flipped = analyze::flipped_faces(m);
    for &t in &flipped {
        m.triangles[t as usize].swap(1, 2);
    }
    let mut turned = flipped.len();
    // Closed pieces enclosing negative volume are inside out.
    let (id, n) = m.components();
    let ef = m.edge_faces();
    let mut vol = vec![0.0; n];
    let mut closed = vec![true; n];
    for (t, tri) in m.triangles.iter().enumerate() {
        let [a, b, c] = tri.map(|v| m.positions[v as usize]);
        vol[id[t] as usize] += crate::linalg::dot(a, crate::linalg::cross(b, c)) / 6.0;
        for k in 0..3 {
            if ef.get(&edge_key(tri[k], tri[(k + 1) % 3])).is_some_and(|f| f.len() != 2) {
                closed[id[t] as usize] = false;
            }
        }
    }
    for (t, tri) in m.triangles.iter_mut().enumerate() {
        let c = id[t] as usize;
        // Open pieces have no inside; a negative volume still says "mostly turned in".
        if vol[c] < 0.0 && (closed[c] || vol[c].abs() > 0.0) {
            tri.swap(1, 2);
            turned += 1;
        }
    }
    turned
}

/// Drops pieces smaller than `fraction` of the largest (or all but the
/// largest). Returns the pieces dropped.
pub fn remove_components(m: &mut Mesh, fraction: f64, largest_only: bool) -> usize {
    let (id, n) = m.components();
    if n <= 1 {
        return 0;
    }
    let mut size = vec![0usize; n];
    for c in &id {
        size[*c as usize] += 1;
    }
    let largest = (0..n).max_by_key(|&c| size[c]).unwrap();
    let keep: Vec<bool> = (0..n).map(|c| if largest_only { c == largest } else { size[c] as f64 >= fraction * size[largest] as f64 }).collect();
    let mut t = 0usize;
    m.triangles.retain(|_| {
        let k = keep[id[t] as usize];
        t += 1;
        k
    });
    keep.iter().filter(|k| !**k).count()
}

/// Cuts out every triangle that passes through another. Returns how many.
pub fn remove_self_intersections(m: &mut Mesh) -> usize {
    let (pairs, _) = analyze::self_intersections(m, usize::MAX);
    let bad: HashSet<u32> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
    drop_faces(m, &bad)
}

/// Runs the repair steps in order and reports before, after and each change.
pub fn repair(mesh: &Mesh, o: &Options) -> (Mesh, Repaired) {
    let before = analyze::analyze(mesh);
    let mut m = mesh.clone();
    let mut steps = Vec::new();
    let mut note = |step: &str, count: usize| {
        if count > 0 {
            steps.push(Step { step: step.into(), count });
        }
    };
    let (n0, t0) = (m.positions.len(), m.triangles.len());
    m.weld(if o.weld > 0.0 { o.weld } else { m.weld_eps() });
    note("vertices welded", n0 - m.positions.len());
    // Welding drops triangles whose corners became one: degenerate ones.
    let collapsed = t0 - m.triangles.len();
    if o.degenerate {
        note("degenerate triangles removed", collapsed + remove_degenerate(&mut m));
    }
    if o.duplicates {
        note("duplicate triangles removed", remove_duplicates(&mut m));
    }
    if o.non_manifold {
        note("non-manifold edges and vertices fixed", fix_non_manifold(&mut m));
    }
    if o.largest_only || o.min_component > 0.0 {
        note("small pieces removed", remove_components(&mut m, o.min_component, o.largest_only));
    }
    if o.self_intersections {
        // Cut the crossing triangles and close the gaps, a few rounds (a
        // patch can meet another crossing).
        let mut cut_total = 0;
        for round in 0..4 {
            let cut = remove_self_intersections(&mut m);
            cut_total += cut;
            if cut == 0 {
                break;
            }
            if o.non_manifold {
                fix_non_manifold(&mut m);
            }
            if round < 3 && o.fill_holes > 0 {
                if o.harmonize {
                    harmonize_normals(&mut m);
                }
                crate::holes::fill_holes(&mut m, o.fill_holes, false);
            }
        }
        note("self-intersecting triangles cut out", cut_total);
    }
    if o.harmonize {
        note("triangles turned to agree", harmonize_normals(&mut m));
    }
    if o.fill_holes > 0 {
        let f = crate::holes::fill_holes(&mut m, o.fill_holes, o.fair);
        note("holes filled", f.filled);
        note("holes too large to fill", f.skipped);
        if o.harmonize && f.filled > 0 {
            harmonize_normals(&mut m);
        }
    }
    if o.flip {
        m.flip();
        note("triangles flipped", m.triangles.len());
    }
    let unused = {
        let n = m.positions.len();
        m.compact();
        n - m.positions.len()
    };
    note("unused vertices removed", unused);
    let after = analyze::analyze(&m);
    (m, Repaired { before, after, steps })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linalg::scale;
    use crate::mesh::{cube, uv_sphere};

    #[test]
    fn a_broken_scan_comes_out_a_solid() {
        let mut m = uv_sphere(15.0, 24, 48);
        // Holes, a flipped patch, duplicates, a degenerate sliver, debris, and inside out.
        m.triangles.drain(100..112);
        for t in 300..340 {
            m.triangles[t].swap(1, 2);
        }
        m.triangles.push(m.triangles[500]);
        m.triangles.push([10, 11, 11]);
        let mut debris = cube(0.5, 0.5, 0.5);
        debris.transform(&[[1.0, 0.0, 0.0, 40.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]);
        m.merge(&debris);
        m.flip();
        let (fixed, rep) = repair(&m, &Options { min_component: 0.05, ..Default::default() });
        assert!(!rep.before.solid);
        assert!(rep.before.flipped >= 40, "{:?}", rep.before);
        assert!(rep.after.solid, "{:?} {:?}", rep.after, rep.steps);
        assert_eq!(rep.after.components, 1);
        assert!(fixed.volume() > 0.0, "turned outward");
        let want = uv_sphere(15.0, 24, 48).volume();
        assert!((fixed.volume() - want).abs() / want < 0.01);
        let step = |s: &str| rep.steps.iter().find(|x| x.step == s).map(|x| x.count).unwrap_or(0);
        assert_eq!(step("duplicate triangles removed"), 1, "{:?}", rep.steps);
        assert_eq!(step("degenerate triangles removed"), 1);
        assert_eq!(step("small pieces removed"), 1);
        assert!(step("holes filled") >= 1);
    }

    #[test]
    fn a_dent_pushed_through_the_far_side_is_cut_out_and_closed() {
        // A scanning spike: one vertex driven through the sphere and out the other side.
        let mut m = uv_sphere(15.0, 24, 48);
        let v = 200;
        m.positions[v] = scale(m.positions[v], -1.3);
        let r = analyze::analyze(&m);
        assert!(r.self_intersections > 0);
        let (fixed, rep) = repair(&m, &Options::default());
        assert_eq!(rep.after.self_intersections, 0, "{:?}", rep.steps);
        assert!(rep.after.solid, "{:?} {:?}", rep.after, rep.steps);
        let want = uv_sphere(15.0, 24, 48).volume();
        assert!((fixed.volume() - want).abs() / want < 0.02, "{} vs {want}", fixed.volume());
    }
}
