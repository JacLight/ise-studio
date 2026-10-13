//! Inspection: how far each point of a scan (or vertex of a mesh) lies from
//! the CAD part it should match, signed (outside the material positive,
//! inside negative), with the statistics and a pass/fail against a
//! tolerance, as FreeCAD's Inspection workbench. Scans rarely arrive in the
//! part's coordinates, so they can first be best-fit aligned by iterative
//! closest points (point-to-plane, Chen and Medioni 1992; outlier pairs
//! trimmed so a partial scan still fits).

use crate::V3;
use crate::linalg::*;
use crate::mesh::Mesh;
use crate::spatial::TriBvh;
use rayon::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Stats {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// Mean of |deviation|.
    pub mean_abs: f64,
    pub rms: f64,
    pub std: f64,
    /// 95% of points deviate less than this (|deviation|).
    pub p95: f64,
    /// The worst point and its deviation.
    pub worst: V3,
    pub worst_deviation: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Deviation {
    pub stats: Stats,
    pub tolerance: f64,
    /// Share of points within ±tolerance (0…1).
    pub within: f64,
    pub over: usize,
    pub under: usize,
    pub passed: bool,
    /// Counts in bins of width tolerance/2 from −4·tol to +4·tol (16 bins;
    /// the ends collect everything past them).
    pub histogram: Vec<usize>,
    /// The signed deviation of every point (mm), in order.
    #[serde(skip)]
    pub values: Vec<f64>,
}

pub fn stats(points: &[V3], values: &[f64]) -> Stats {
    let n = values.len().max(1) as f64;
    let mean = values.iter().sum::<f64>() / n;
    let mean_abs = values.iter().map(|v| v.abs()).sum::<f64>() / n;
    let rms = (values.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
    let std = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
    let mut abs: Vec<f64> = values.iter().map(|v| v.abs()).collect();
    abs.sort_by(|a, b| a.total_cmp(b));
    let p95 = abs.get(((abs.len() as f64 * 0.95).ceil() as usize).saturating_sub(1).min(abs.len().saturating_sub(1))).copied().unwrap_or(0.0);
    let worst = (0..values.len()).max_by(|&a, &b| values[a].abs().total_cmp(&values[b].abs()));
    Stats {
        count: values.len(),
        min: values.iter().copied().fold(f64::INFINITY, f64::min),
        max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        mean,
        mean_abs,
        rms,
        std,
        p95,
        worst: worst.map(|i| points[i]).unwrap_or([0.0; 3]),
        worst_deviation: worst.map(|i| values[i]).unwrap_or(0.0),
    }
}

/// Each point's signed distance to the reference surface, and the verdict.
/// `required` is the share of points that must lie within ±tolerance (1.0: all).
pub fn deviation(points: &[V3], reference: &TriBvh, tolerance: f64, required: f64) -> Deviation {
    let values: Vec<f64> = points.par_iter().map(|p| reference.closest(*p).map(|c| c.signed).unwrap_or(f64::NAN)).collect();
    let values: Vec<f64> = values.into_iter().map(|v| if v.is_finite() { v } else { 0.0 }).collect();
    let over = values.iter().filter(|v| **v > tolerance).count();
    let under = values.iter().filter(|v| **v < -tolerance).count();
    let within = 1.0 - (over + under) as f64 / values.len().max(1) as f64;
    let mut histogram = vec![0usize; 16];
    let bin = (tolerance / 2.0).max(1e-12);
    for v in &values {
        let b = ((v / bin).floor() as i64 + 8).clamp(0, 15) as usize;
        histogram[b] += 1;
    }
    Deviation { stats: stats(points, &values), tolerance, within, over, under, passed: within + 1e-12 >= required, histogram, values }
}

/// A rigid motion as a 3×4 matrix (rows).
pub type Rigid = [[f64; 4]; 3];

pub const IDENTITY: Rigid = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

pub fn apply(t: &Rigid, p: V3) -> V3 {
    crate::mesh::apply_affine(t, p)
}

/// a then b.
pub fn compose(b: &Rigid, a: &Rigid) -> Rigid {
    let mut out = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..4 {
            out[i][j] = (0..3).map(|k| b[i][k] * a[k][j]).sum::<f64>() + if j == 3 { b[i][3] } else { 0.0 };
        }
    }
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct Alignment {
    pub transform: Rigid,
    /// RMS distance of the pairs kept, before and after (mm).
    pub rms_before: f64,
    pub rms_after: f64,
    pub iterations: usize,
}

/// Best-fit rigid alignment of points onto a surface. `initial` starts it
/// (identity when the scan is roughly in place); with `coarse`, the
/// principal axes are matched first (four candidate turns, the best kept).
pub fn align(points: &[V3], reference: &TriBvh, coarse: bool, max_iter: usize) -> Alignment {
    // Work on up to 5000 points.
    let step = (points.len() / 5000).max(1);
    let src: Vec<V3> = points.iter().step_by(step).copied().collect();
    let rms_of = |t: &Rigid| -> f64 {
        let d: Vec<f64> = src.par_iter().map(|p| reference.closest(apply(t, *p)).map(|c| c.distance).unwrap_or(0.0)).collect();
        (d.iter().map(|v| v * v).sum::<f64>() / d.len().max(1) as f64).sqrt()
    };
    let rms_before = rms_of(&IDENTITY);
    let mut starts = vec![IDENTITY];
    if coarse {
        let (cs, ms) = covariance(&src);
        let ref_pts: Vec<V3> = reference.mesh.positions.clone();
        let (cr, mr) = covariance(&ref_pts);
        let (_, es) = sym_eigen3(ms);
        let (_, er) = sym_eigen3(mr);
        for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
            // Source axes onto reference axes (right-handed both).
            let a = [scale(es[2], sx), scale(es[1], sy), cross(scale(es[2], sx), scale(es[1], sy))];
            let b = [er[2], er[1], cross(er[2], er[1])];
            let mut r = [[0.0; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    r[i][j] = (0..3).map(|k| b[k][i] * a[k][j]).sum();
                }
            }
            let t = sub(cr, mat_vec(&r, cs));
            starts.push([[r[0][0], r[0][1], r[0][2], t[0]], [r[1][0], r[1][1], r[1][2], t[1]], [r[2][0], r[2][1], r[2][2], t[2]]]);
        }
    }
    let mut best: Option<(f64, Rigid, usize)> = None;
    for start in starts {
        let (t, it) = icp(&src, reference, start, max_iter);
        let r = rms_of(&t);
        if best.as_ref().is_none_or(|b| r < b.0) {
            best = Some((r, t, it));
        }
    }
    let (rms_after, transform, iterations) = best.unwrap();
    Alignment { transform, rms_before, rms_after, iterations }
}

fn icp(src: &[V3], reference: &TriBvh, mut t: Rigid, max_iter: usize) -> (Rigid, usize) {
    let normals: Vec<V3> = (0..reference.mesh.triangles.len()).map(|f| reference.mesh.face_normal(f)).collect();
    for it in 0..max_iter {
        let pairs: Vec<(V3, V3, V3, f64)> = src
            .par_iter()
            .filter_map(|p| {
                let q = apply(&t, *p);
                reference.closest(q).map(|c| (q, c.point, normals[c.triangle as usize], c.distance))
            })
            .collect();
        if pairs.len() < 6 {
            return (t, it);
        }
        // Trim: pairs far beyond the typical distance are not the same surface.
        let mut d: Vec<f64> = pairs.iter().map(|p| p.3).collect();
        d.sort_by(|a, b| a.total_cmp(b));
        let median = d[d.len() / 2];
        let limit = (median * 3.0).max(1e-9);
        // Point-to-plane: minimise Σ((p + ω×p + v − q)·n)² over ω, v.
        let mut ata = [0.0; 36];
        let mut atb = [0.0; 6];
        for (p, q, n, dd) in &pairs {
            if *dd > limit {
                continue;
            }
            let c = cross(*p, *n);
            let row = [c[0], c[1], c[2], n[0], n[1], n[2]];
            let r = dot(sub(*q, *p), *n);
            for i in 0..6 {
                atb[i] += row[i] * r;
                for j in 0..6 {
                    ata[i * 6 + j] += row[i] * row[j];
                }
            }
        }
        for i in 0..6 {
            ata[i * 6 + i] += 1e-12;
        }
        let Some(x) = solve_dense(&ata, &atb, 6) else { return (t, it) };
        let angle = norm([x[0], x[1], x[2]]);
        let r = if angle > 1e-15 { axis_angle([x[0], x[1], x[2]], angle) } else { [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };
        let step: Rigid = [[r[0][0], r[0][1], r[0][2], x[3]], [r[1][0], r[1][1], r[1][2], x[4]], [r[2][0], r[2][1], r[2][2], x[5]]];
        t = compose(&step, &t);
        if angle < 1e-9 && norm([x[3], x[4], x[5]]) < 1e-9 {
            return (t, it + 1);
        }
    }
    (t, max_iter)
}

/// The vertices of a mesh (for inspecting a mesh rather than a cloud).
pub fn mesh_points(m: &Mesh) -> Vec<V3> {
    m.positions.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;
    use crate::points::sample_mesh;

    #[test]
    fn a_bulging_part_shows_its_deviation_and_fails_a_tight_tolerance() {
        let cad = cube(40.0, 30.0, 20.0);
        let bvh = TriBvh::new(&cad);
        let mut scan = sample_mesh(&cad, 20000);
        // The top face bulges up to 0.3 mm in the middle (a warped print).
        for p in scan.positions.iter_mut() {
            if (p[2] - 20.0).abs() < 1e-9 {
                let (u, v) = (p[0] / 40.0, p[1] / 30.0);
                p[2] += 0.3 * (std::f64::consts::PI * u).sin() * (std::f64::consts::PI * v).sin();
            }
        }
        let d = deviation(&scan.positions, &bvh, 0.1, 1.0);
        assert!((d.stats.max - 0.3).abs() < 0.01, "{:?}", d.stats);
        assert!(d.stats.min.abs() < 1e-9);
        assert!(!d.passed);
        assert!(d.over > 0 && d.under == 0);
        let loose = deviation(&scan.positions, &bvh, 0.35, 1.0);
        assert!(loose.passed && loose.within == 1.0);
        // Inside is negative.
        let inside = deviation(&[[20.0, 15.0, 19.5]], &bvh, 0.1, 1.0);
        assert!((inside.values[0] + 0.5).abs() < 1e-12);
        assert_eq!(d.histogram.iter().sum::<usize>(), scan.len());
    }

    #[test]
    fn a_moved_scan_is_aligned_back() {
        let cad = crate::mesh::cylinder(10.0, 40.0, 64, 8);
        let mut cad2 = cad.clone();
        // An asymmetric part (a cylinder with a block on its side).
        let mut block = cube(8.0, 4.0, 10.0);
        block.transform(&[[1.0, 0.0, 0.0, 6.0], [0.0, 1.0, 0.0, -2.0], [0.0, 0.0, 1.0, 5.0]]);
        cad2.merge(&block);
        let bvh = TriBvh::new(&cad2);
        let mut scan = sample_mesh(&cad2, 8000);
        let r = axis_angle(unit([0.3, -0.2, 1.0]), 0.12);
        let moved: Rigid = [[r[0][0], r[0][1], r[0][2], 1.5], [r[1][0], r[1][1], r[1][2], -0.8], [r[2][0], r[2][1], r[2][2], 0.6]];
        scan.transform(&moved);
        let a = align(&scan.positions, &bvh, false, 60);
        assert!(a.rms_before > 0.5, "{}", a.rms_before);
        assert!(a.rms_after < 0.05, "rms after alignment {}", a.rms_after);
        let back: Vec<V3> = scan.positions.iter().map(|p| apply(&a.transform, *p)).collect();
        let d = deviation(&back, &bvh, 0.1, 0.99);
        assert!(d.passed, "{:?}", d.stats);
    }
}
