//! Cross sections of a mesh by planes: each triangle crossing the plane
//! gives a segment, and segments are chained end to end into closed loops
//! (a closed mesh) or open polylines (a scan with gaps), as FreeCAD's
//! Mesh_CrossSections and MeshPart_SectionByPlane.

use crate::V3;
use crate::linalg::*;
use crate::mesh::Mesh;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize)]
pub struct Polyline {
    pub points: Vec<V3>,
    /// In the plane's own coordinates (x along `u`, y along `v`).
    pub flat: Vec<[f64; 2]>,
    pub closed: bool,
    /// Length (mm), and for closed loops the enclosed area (mm², positive
    /// counter-clockwise seen from the normal).
    pub length: f64,
    pub area: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Section {
    pub origin: V3,
    pub normal: V3,
    pub u: V3,
    pub v: V3,
    pub polylines: Vec<Polyline>,
}

/// The mesh cut by the plane through `origin` square to `normal`.
pub fn section(m: &Mesh, origin: V3, normal: V3) -> Section {
    let n = unit(normal);
    let (u, v) = frame(n);
    let d: Vec<f64> = m.positions.iter().map(|p| dot(sub(*p, origin), n)).collect();
    let size = m.size().max(1e-9);
    let eps = size * 1e-12;
    // Points on edges, keyed by the edge (so neighbours share them exactly).
    let mut at_edge: HashMap<(u32, u32), usize> = HashMap::new();
    let mut points: Vec<V3> = Vec::new();
    let mut point = |a: u32, b: u32, points: &mut Vec<V3>| -> usize {
        let key = crate::mesh::edge_key(a, b);
        *at_edge.entry(key).or_insert_with(|| {
            let (da, db) = (d[key.0 as usize], d[key.1 as usize]);
            let t = da / (da - db);
            points.push(lerp(m.positions[key.0 as usize], m.positions[key.1 as usize], t));
            points.len() - 1
        })
    };
    let mut segs: Vec<(usize, usize)> = Vec::new();
    for tri in &m.triangles {
        // Vertices exactly on the plane count as just above it (no double crossings).
        let s = tri.map(|i| if d[i as usize] > eps { 1 } else if d[i as usize] < -eps { -1 } else { 1 });
        if s[0] == s[1] && s[1] == s[2] {
            continue;
        }
        let mut ends = Vec::new();
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            if s[k] != s[(k + 1) % 3] {
                ends.push((point(a, b, &mut points), s[k]));
            }
        }
        if ends.len() == 2 {
            // Orient so loops run counter-clockwise about the normal on an outward-wound solid.
            let (p, q) = if ends[0].1 > 0 { (ends[0].0, ends[1].0) } else { (ends[1].0, ends[0].0) };
            if p != q {
                segs.push((p, q));
            }
        }
    }
    let mut polylines = Vec::new();
    let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut prev: HashMap<usize, usize> = HashMap::new();
    for &(a, b) in &segs {
        next.entry(a).or_default().push(b);
        prev.insert(b, a);
    }
    let mut used: HashMap<(usize, usize), bool> = HashMap::new();
    // Open chains first (from points nothing leads into), then loops.
    let mut starts: Vec<usize> = segs.iter().map(|s| s.0).filter(|a| !prev.contains_key(a)).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut all: Vec<usize> = segs.iter().map(|s| s.0).collect();
    all.sort_unstable();
    all.dedup();
    starts.extend(all);
    for s in starts {
        loop {
            let Some(first) = next.get(&s).and_then(|l| l.iter().find(|&&b| !used.contains_key(&(s, b))).copied()) else { break };
            let mut chain = vec![s, first];
            used.insert((s, first), true);
            let mut cur = first;
            let mut closed = false;
            while let Some(n) = next.get(&cur).and_then(|l| l.iter().find(|&&b| !used.contains_key(&(cur, b))).copied()) {
                used.insert((cur, n), true);
                if n == s {
                    closed = true;
                    break;
                }
                chain.push(n);
                cur = n;
            }
            let pts: Vec<V3> = chain.iter().map(|&i| points[i]).collect();
            let flat: Vec<[f64; 2]> = pts.iter().map(|p| {
                let r = sub(*p, origin);
                [dot(r, u), dot(r, v)]
            }).collect();
            let mut length: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
            let mut area = 0.0;
            if closed {
                length += dist(pts[pts.len() - 1], pts[0]);
                for i in 0..flat.len() {
                    let (a, b) = (flat[i], flat[(i + 1) % flat.len()]);
                    area += a[0] * b[1] - b[0] * a[1];
                }
                area /= 2.0;
            }
            if pts.len() >= 2 {
                polylines.push(Polyline { points: pts, flat, closed, length, area });
            }
        }
    }
    Section { origin, normal: n, u, v, polylines }
}

/// Sections at even steps along a direction across the whole mesh.
pub fn sections(m: &Mesh, normal: V3, count: usize) -> Vec<Section> {
    let n = unit(normal);
    let (lo, hi) = m.positions.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(dot(*p, n)), b.max(dot(*p, n))));
    (0..count)
        .map(|i| {
            let t = lo + (hi - lo) * (i as f64 + 0.5) / count as f64;
            section(m, scale(n, t), n)
        })
        .collect()
}

/// The part of a mesh behind a plane (FreeCAD's Mesh_TrimByPlane): triangles
/// in front of it dropped, those crossing it cut along it. The cut is left
/// open (a scan trimmed, not a solid capped).
pub fn trim(m: &Mesh, origin: V3, normal: V3) -> Mesh {
    let n = unit(normal);
    let d: Vec<f64> = m.positions.iter().map(|p| dot(sub(*p, origin), n)).collect();
    let mut out = Mesh { positions: m.positions.clone(), triangles: Vec::new() };
    let mut cut: HashMap<(u32, u32), u32> = HashMap::new();
    let mut at = |a: u32, b: u32, out: &mut Mesh| -> u32 {
        let key = crate::mesh::edge_key(a, b);
        *cut.entry(key).or_insert_with(|| {
            let (da, db) = (d[key.0 as usize], d[key.1 as usize]);
            out.positions.push(lerp(m.positions[key.0 as usize], m.positions[key.1 as usize], da / (da - db)));
            (out.positions.len() - 1) as u32
        })
    };
    for t in &m.triangles {
        let keep = t.map(|v| d[v as usize] <= 0.0);
        match keep.iter().filter(|k| **k).count() {
            3 => out.triangles.push(*t),
            0 => {}
            count => {
                // Rotate so the odd corner is first.
                let odd = if count == 1 { keep.iter().position(|k| *k).unwrap() } else { keep.iter().position(|k| !*k).unwrap() };
                let (a, b, c) = (t[odd], t[(odd + 1) % 3], t[(odd + 2) % 3]);
                let (ab, ac) = (at(a, b, &mut out), at(a, c, &mut out));
                if count == 1 {
                    out.triangles.push([a, ab, ac]);
                } else {
                    out.triangles.push([ab, b, c]);
                    out.triangles.push([ab, c, ac]);
                }
            }
        }
    }
    out.compacted()
}

/// A polyline laid onto a mesh (FreeCAD's MeshPart_CurveOnMesh): densified
/// to `step` mm and each point moved to the closest point of the surface.
pub fn project_curve(bvh: &crate::spatial::TriBvh, points: &[V3], step: f64) -> Vec<V3> {
    let mut dense = Vec::new();
    for w in points.windows(2) {
        let n = (dist(w[0], w[1]) / step.max(1e-9)).ceil().max(1.0) as usize;
        for i in 0..n {
            dense.push(lerp(w[0], w[1], i as f64 / n as f64));
        }
    }
    if let Some(last) = points.last() {
        dense.push(*last);
    }
    dense.into_iter().filter_map(|p| bvh.closest(p).map(|c| c.point)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{cube, cylinder};

    #[test]
    fn a_cylinder_cut_across_gives_one_circle() {
        let m = cylinder(10.0, 20.0, 128, 4);
        let s = section(&m, [0.0, 0.0, 7.3], [0.0, 0.0, 1.0]);
        assert_eq!(s.polylines.len(), 1);
        let p = &s.polylines[0];
        assert!(p.closed);
        assert!((p.length - std::f64::consts::TAU * 10.0).abs() < 0.05, "{}", p.length);
        assert!((p.area - std::f64::consts::PI * 100.0).abs() < 0.5, "{}", p.area);
    }

    #[test]
    fn trimming_keeps_what_is_behind_the_plane_and_curves_lie_on_the_surface() {
        let m = cylinder(10.0, 20.0, 64, 4);
        let t = trim(&m, [0.0, 0.0, 7.0], [0.0, 0.0, 1.0]);
        assert!(t.positions.iter().all(|p| p[2] <= 7.0 + 1e-9));
        // The side's area below z = 7 and the bottom cap.
        let side = 64.0 * 2.0 * 10.0 * (std::f64::consts::PI / 64.0).sin() * 7.0;
        let cap = 0.5 * 64.0 * 100.0 * (std::f64::consts::TAU / 64.0).sin();
        assert!((t.area() - side - cap).abs() < 1e-6, "{} vs {}", t.area(), side + cap);
        assert_eq!(t.boundary_loops().len(), 1);
        let bvh = crate::spatial::TriBvh::new(&m);
        let c = project_curve(&bvh, &[[12.0, 0.0, 5.0], [0.0, 12.0, 15.0]], 0.5);
        assert!(c.len() > 30);
        assert!(c.iter().all(|p| bvh.closest(*p).unwrap().distance < 1e-9));
    }

    #[test]
    fn an_open_mesh_gives_open_polylines_and_a_box_a_rectangle() {
        let mut m = cube(10.0, 20.0, 30.0);
        let s = section(&m, [0.0, 0.0, 15.0], [0.0, 0.0, 1.0]);
        assert_eq!(s.polylines.len(), 1);
        assert!((s.polylines[0].area - 200.0).abs() < 1e-9);
        // Remove the +x side: the section opens.
        m.triangles.retain(|t| !t.iter().all(|&v| m.positions[v as usize][0] > 5.0));
        let s = section(&m, [0.0, 0.0, 15.0], [0.0, 0.0, 1.0]);
        assert_eq!(s.polylines.len(), 1);
        assert!(!s.polylines[0].closed);
        assert!((s.polylines[0].length - 40.0).abs() < 1e-9);
    }
}
