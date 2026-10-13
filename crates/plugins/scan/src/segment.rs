//! Segmentation: a mesh split into regions, as FreeCAD's Mesh_Segmentation
//! and Mesh_SegmentationBestFit. Regions grow across edges while the
//! surface bends gently and its curvature stays alike (so a fillet parts
//! from the flat it joins), or are taken as the planes RANSAC finds; each
//! region is then fitted with the primitive that suits it best.

use crate::V3;
use crate::curvature::curvature;
use crate::fit::{self, Kind};
use crate::linalg::*;
use crate::mesh::{Mesh, edge_key};
use crate::points::Cloud;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub id: usize,
    pub triangles: usize,
    pub area: f64,
    /// Mean normal (area-weighted).
    pub normal: V3,
    pub centroid: V3,
    /// The primitive that fits it best, if it has enough points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<fit::Fit>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Segmentation {
    /// The region of each triangle (u32::MAX: none, for plane detection).
    pub labels: Vec<u32>,
    pub segments: Vec<Segment>,
}

/// Region growing: neighbours join while their normals are within `angle`
/// degrees and their mean curvatures differ by less than `curvature_tol`
/// (1/mm; 0 to ignore curvature). Regions under `min_triangles` are merged
/// into their largest neighbour.
pub fn by_curvature(m: &Mesh, angle: f64, curvature_tol: f64, min_triangles: usize) -> Segmentation {
    let n = m.triangles.len();
    let normals: Vec<V3> = (0..n).map(|t| m.face_normal(t)).collect();
    let face_curv: Vec<f64> = if curvature_tol > 0.0 {
        let c = curvature(m);
        m.triangles.iter().map(|t| t.iter().map(|&v| c.mean[v as usize]).sum::<f64>() / 3.0).collect()
    } else {
        vec![0.0; n]
    };
    let ef = m.edge_faces();
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    for faces in ef.values() {
        if faces.len() == 2 {
            adj[faces[0] as usize].push(faces[1]);
            adj[faces[1] as usize].push(faces[0]);
        }
    }
    let cos = angle.to_radians().cos();
    let mut labels = vec![u32::MAX; n];
    let mut count = 0u32;
    let mut stack = Vec::new();
    for s in 0..n {
        if labels[s] != u32::MAX {
            continue;
        }
        labels[s] = count;
        stack.push(s as u32);
        while let Some(t) = stack.pop() {
            for &u in &adj[t as usize] {
                if labels[u as usize] != u32::MAX {
                    continue;
                }
                let bend_ok = dot(normals[t as usize], normals[u as usize]) >= cos;
                let curv_ok = curvature_tol <= 0.0 || (face_curv[t as usize] - face_curv[u as usize]).abs() <= curvature_tol;
                if bend_ok && curv_ok {
                    labels[u as usize] = count;
                    stack.push(u);
                }
            }
        }
        count += 1;
    }
    // Small regions (noise) join the neighbour they share the most edges with.
    if min_triangles > 1 {
        for _ in 0..3 {
            let mut size = vec![0usize; count as usize];
            for l in &labels {
                size[*l as usize] += 1;
            }
            let mut changed = false;
            for t in 0..n {
                let l = labels[t] as usize;
                if size[l] >= min_triangles {
                    continue;
                }
                let best = adj[t].iter().map(|&u| labels[u as usize]).filter(|&x| x as usize != l).max_by_key(|&x| size[x as usize]);
                if let Some(b) = best {
                    if size[b as usize] > size[l] {
                        size[l] -= 1;
                        size[b as usize] += 1;
                        labels[t] = b;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
    finish(m, labels)
}

/// Planes found by RANSAC on the vertices; a triangle belongs to a plane
/// when all three of its corners do.
pub fn by_planes(m: &Mesh, threshold: f64, min_points: usize, max_planes: usize) -> Segmentation {
    let cloud = Cloud { positions: m.positions.clone(), normals: m.vertex_normals(), colors: vec![] };
    let found = fit::detect(&cloud, &[Kind::Plane], &fit::Ransac { threshold, ..Default::default() }, min_points, max_planes);
    let mut vlabel = vec![u32::MAX; m.positions.len()];
    for (i, f) in found.iter().enumerate() {
        for &v in &f.inliers {
            if vlabel[v as usize] == u32::MAX {
                vlabel[v as usize] = i as u32;
            }
        }
    }
    let labels: Vec<u32> = m.triangles.iter().map(|t| if vlabel[t[0] as usize] == vlabel[t[1] as usize] && vlabel[t[1] as usize] == vlabel[t[2] as usize] { vlabel[t[0] as usize] } else { u32::MAX }).collect();
    finish(m, labels)
}

/// Relabels regions by size (largest 0) and describes each, fitting a primitive to its vertices.
fn finish(m: &Mesh, labels: Vec<u32>) -> Segmentation {
    let count = labels.iter().filter(|l| **l != u32::MAX).map(|l| *l as usize + 1).max().unwrap_or(0);
    let mut size = vec![0usize; count];
    for l in labels.iter().filter(|l| **l != u32::MAX) {
        size[*l as usize] += 1;
    }
    let mut order: Vec<usize> = (0..count).filter(|&l| size[l] > 0).collect();
    order.sort_by(|a, b| size[*b].cmp(&size[*a]));
    let mut remap = vec![u32::MAX; count];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new as u32;
    }
    let labels: Vec<u32> = labels.iter().map(|l| if *l == u32::MAX { u32::MAX } else { remap[*l as usize] }).collect();
    let k = order.len();
    let mut tris: Vec<Vec<usize>> = vec![Vec::new(); k];
    for (t, l) in labels.iter().enumerate() {
        if *l != u32::MAX {
            tris[*l as usize].push(t);
        }
    }
    let segments = tris
        .iter()
        .enumerate()
        .map(|(id, ts)| {
            let mut area = 0.0;
            let mut nrm = [0.0; 3];
            let mut cen = [0.0; 3];
            let mut verts: Vec<u32> = Vec::new();
            for &t in ts {
                let a = m.face_area(t);
                area += a;
                nrm = add(nrm, scale(m.face_normal(t), a));
                let [p, q, r] = m.corners(t);
                cen = add(cen, scale(add(add(p, q), r), a / 3.0));
                verts.extend_from_slice(&m.triangles[t]);
            }
            verts.sort_unstable();
            verts.dedup();
            // Fitted to the region's vertices and points spread over its triangles
            // (a box face has only four vertices).
            let fit = (area > 0.0).then(|| {
                let sub = Mesh { positions: m.positions.clone(), triangles: ts.iter().map(|&t| m.triangles[t]).collect() };
                let c = crate::points::sample_mesh(&sub, (verts.len() * 2).clamp(200, 4000));
                fit::fit_best(&c).into_iter().next()
            }).flatten();
            Segment { id, triangles: ts.len(), area, normal: unit(nrm), centroid: if area > 0.0 { scale(cen, 1.0 / area) } else { cen }, fit }
        })
        .collect();
    Segmentation { labels, segments }
}

/// The open-edge loops of a mesh as polylines (FreeCAD's Reen_MeshBoundary).
pub fn boundaries(m: &Mesh) -> Vec<Vec<V3>> {
    m.boundary_loops().into_iter().map(|l| l.into_iter().map(|v| m.positions[v as usize]).collect()).collect()
}

/// Edges where the faces meet at more than `angle` degrees (feature lines).
pub fn sharp_edges(m: &Mesh, angle: f64) -> Vec<[V3; 2]> {
    let cos = angle.to_radians().cos();
    m.edge_faces()
        .into_iter()
        .filter(|(_, f)| f.len() == 2 && dot(m.face_normal(f[0] as usize), m.face_normal(f[1] as usize)) < cos)
        .map(|((a, b), _)| [m.positions[a as usize], m.positions[b as usize]])
        .collect()
}

/// The edge key helper re-exported for callers labelling edges.
pub fn key(a: u32, b: u32) -> (u32, u32) {
    edge_key(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_with_a_boss_splits_into_its_faces_and_the_boss() {
        let mut m = crate::mesh::cube(40.0, 40.0, 10.0);
        let mut boss = crate::mesh::cylinder(6.0, 15.0, 64, 4);
        boss.transform(&[[1.0, 0.0, 0.0, 20.0], [0.0, 1.0, 0.0, 20.0], [0.0, 0.0, 1.0, 10.0]]);
        m.merge(&boss);
        let s = by_curvature(&m, 20.0, 0.0, 1);
        // Six box faces, the boss's side and two caps.
        assert_eq!(s.segments.len(), 9, "{:?}", s.segments.iter().map(|x| (x.triangles, x.fit.as_ref().map(|f| f.primitive.kind()))).collect::<Vec<_>>());
        let side = s.segments.iter().find(|x| x.fit.as_ref().is_some_and(|f| f.primitive.kind() == Kind::Cylinder)).expect("the boss side fits a cylinder");
        let fit::Primitive::Cylinder { radius, .. } = side.fit.as_ref().unwrap().primitive else { panic!() };
        assert!((radius - 6.0).abs() < 0.01, "{radius}");
        let planes = s.segments.iter().filter(|x| x.fit.as_ref().is_some_and(|f| f.primitive.kind() == Kind::Plane)).count();
        assert_eq!(planes, 8);
        assert!(s.labels.iter().all(|l| *l != u32::MAX));
    }
}
