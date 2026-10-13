//! Discrete curvature per vertex (Meyer, Desbrun, Schröder and Barr 2003):
//! mean curvature from the cotangent Laplace–Beltrami operator, Gaussian
//! curvature from the angle deficit, both over mixed Voronoi areas; the
//! principal curvatures follow from the two.

use crate::linalg::*;
use crate::mesh::Mesh;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Curvature {
    /// Mean curvature (1/mm; positive where the surface bulges outward).
    pub mean: Vec<f64>,
    pub gaussian: Vec<f64>,
    /// Principal curvatures, larger first.
    pub k1: Vec<f64>,
    pub k2: Vec<f64>,
}

pub fn curvature(m: &Mesh) -> Curvature {
    let n = m.positions.len();
    let mut area = vec![0.0; n];
    let mut lap = vec![[0.0; 3]; n];
    let mut angles = vec![0.0; n];
    let normals = m.vertex_normals();
    for (t, tri) in m.triangles.iter().enumerate() {
        let p = m.corners(t);
        let ang: [f64; 3] = std::array::from_fn(|k| {
            let u = unit(sub(p[(k + 1) % 3], p[k]));
            let v = unit(sub(p[(k + 2) % 3], p[k]));
            dot(u, v).clamp(-1.0, 1.0).acos()
        });
        let tarea = norm(tri_normal(p[0], p[1], p[2])) / 2.0;
        if tarea < 1e-300 {
            continue;
        }
        let obtuse = ang.iter().position(|a| *a > std::f64::consts::FRAC_PI_2);
        for k in 0..3 {
            let (i, j, o) = (k, (k + 1) % 3, (k + 2) % 3);
            // Edge i–j, opposite corner o.
            let cot = 1.0 / ang[o].tan();
            let e = sub(p[j], p[i]);
            let (vi, vj) = (tri[i] as usize, tri[j] as usize);
            lap[vi] = add(lap[vi], scale(e, cot));
            lap[vj] = sub(lap[vj], scale(e, cot));
            angles[tri[k] as usize] += ang[k];
            // Mixed area.
            let a = match obtuse {
                None => {
                    let (c1, c2) = (1.0 / ang[(k + 1) % 3].tan(), 1.0 / ang[(k + 2) % 3].tan());
                    (dist2(p[k], p[(k + 1) % 3]) * c2 + dist2(p[k], p[(k + 2) % 3]) * c1) / 8.0
                }
                Some(ob) if ob == k => tarea / 2.0,
                Some(_) => tarea / 4.0,
            };
            area[tri[k] as usize] += a;
        }
    }
    let boundary: Vec<bool> = {
        let mut b = vec![false; n];
        for lp in m.boundary_loops() {
            for v in lp {
                b[v as usize] = true;
            }
        }
        b
    };
    let mut c = Curvature { mean: vec![0.0; n], gaussian: vec![0.0; n], k1: vec![0.0; n], k2: vec![0.0; n] };
    for v in 0..n {
        if area[v] < 1e-300 || boundary[v] {
            continue;
        }
        let k = scale(lap[v], 1.0 / (2.0 * area[v]));
        // The Laplacian points inward on a convex bulge: sign it against the normal.
        let h = norm(k) / 2.0 * if dot(k, normals[v]) < 0.0 { 1.0 } else { -1.0 };
        let g = (std::f64::consts::TAU - angles[v]) / area[v];
        let disc = (h * h - g).max(0.0).sqrt();
        c.mean[v] = h;
        c.gaussian[v] = g;
        c.k1[v] = h + disc;
        c.k2[v] = h - disc;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{cylinder, uv_sphere};

    #[test]
    fn a_sphere_has_curvature_one_over_its_radius() {
        let m = uv_sphere(10.0, 60, 120);
        let c = curvature(&m);
        let mean = c.mean.iter().sum::<f64>() / c.mean.len() as f64;
        let gauss = c.gaussian.iter().sum::<f64>() / c.gaussian.len() as f64;
        assert!((mean - 0.1).abs() < 0.002, "{mean}");
        assert!((gauss - 0.01).abs() < 0.001, "{gauss}");
    }

    #[test]
    fn a_cylinder_side_curves_one_way_only() {
        let m = cylinder(5.0, 20.0, 96, 20);
        let c = curvature(&m);
        // Side vertices away from the rims.
        let side: Vec<usize> = (0..m.positions.len()).filter(|&v| m.positions[v][2] > 3.0 && m.positions[v][2] < 17.0 && norm([m.positions[v][0], m.positions[v][1], 0.0]) > 4.9).collect();
        let k1 = side.iter().map(|&v| c.k1[v]).sum::<f64>() / side.len() as f64;
        let k2 = side.iter().map(|&v| c.k2[v]).sum::<f64>() / side.len() as f64;
        assert!((k1 - 0.2).abs() < 0.01, "{k1}");
        assert!(k2.abs() < 0.01, "{k2}");
    }
}
