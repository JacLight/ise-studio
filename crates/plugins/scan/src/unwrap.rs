//! Unwrapping a curved surface flat (fabric, leather, foam, thin skins), as
//! FreeCAD's MeshPart unwrap: least squares conformal maps (Lévy, Petitjean,
//! Ray and Maillot 2002) keep angles, with two boundary points pinned; the
//! result is scaled to the surface's own area and its stretch reported, so
//! a pattern cut from it knows how far it will have to give.

use crate::linalg::*;
use crate::mesh::Mesh;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Flat {
    /// Each vertex flat (mm), same order and triangles as the input.
    pub uv: Vec<[f64; 2]>,
    #[serde(skip)]
    pub triangles: Vec<[u32; 3]>,
    /// The outline(s): each boundary loop flat, closed.
    pub outlines: Vec<Vec<[f64; 2]>>,
    pub area_3d: f64,
    pub area_flat: f64,
    /// Per-triangle area change, worst and area-weighted mean (|flat/3D − 1|).
    pub area_distortion_max: f64,
    pub area_distortion_mean: f64,
    /// Edge length change, worst and mean (|flat/3D − 1|).
    pub length_distortion_max: f64,
    pub length_distortion_mean: f64,
    /// Width and height of the flat pattern (mm).
    pub size: [f64; 2],
}

/// Flattens a mesh with at least one open edge loop (a disc-like patch).
pub fn unwrap(m: &Mesh) -> Result<Flat, String> {
    let loops = m.boundary_loops();
    if loops.is_empty() {
        return Err("the surface is closed: unwrap a region of it (one with an edge), or cut it open first".into());
    }
    let (_, pieces) = m.components();
    if pieces > 1 {
        return Err(format!("the surface is in {pieces} pieces: unwrap one at a time"));
    }
    let nv = m.positions.len();
    // Pins: the two boundary vertices farthest apart.
    let rim: Vec<u32> = loops.iter().max_by_key(|l| l.len()).unwrap().clone();
    let mut pin = (rim[0], rim[0], 0.0);
    let step = (rim.len() / 200).max(1);
    for i in (0..rim.len()).step_by(step) {
        for j in (0..rim.len()).step_by(step) {
            let d = dist(m.positions[rim[i] as usize], m.positions[rim[j] as usize]);
            if d > pin.2 {
                pin = (rim[i], rim[j], d);
            }
        }
    }
    let (p0, p1, len) = pin;
    if p0 == p1 {
        return Err("the surface is too small to unwrap".into());
    }
    // Unknowns: u then v for every vertex; pinned ones moved to the right-hand side.
    let fixed = |v: u32| -> Option<[f64; 2]> {
        if v == p0 {
            Some([0.0, 0.0])
        } else if v == p1 {
            Some([len, 0.0])
        } else {
            None
        }
    };
    let mut index = vec![usize::MAX; nv];
    let mut free = 0;
    for v in 0..nv as u32 {
        if fixed(v).is_none() {
            index[v as usize] = free;
            free += 1;
        }
    }
    let n = 2 * free;
    let mut a = Sparse::new(n);
    let mut b = vec![0.0; n];
    for t in 0..m.triangles.len() {
        let tri = m.triangles[t];
        let p = m.corners(t);
        // The triangle in its own plane.
        let e1 = sub(p[1], p[0]);
        let nrm = tri_normal(p[0], p[1], p[2]);
        let d = norm(nrm);
        if d < 1e-300 {
            continue;
        }
        let x = unit(e1);
        let y = unit(cross(nrm, x));
        let q: [[f64; 2]; 3] = p.map(|c| [dot(sub(c, p[0]), x), dot(sub(c, p[0]), y)]);
        let s = 1.0 / d.sqrt();
        // W_j = (x_k − x_l) + i(y_k − y_l), (j, k, l) cyclic.
        let w: [[f64; 2]; 3] = std::array::from_fn(|j| {
            let (k, l) = ((j + 1) % 3, (j + 2) % 3);
            [(q[k][0] - q[l][0]) * s, (q[k][1] - q[l][1]) * s]
        });
        // Two real rows: Re(Σ W U) and Im(Σ W U), U = u + iv.
        for row in 0..2 {
            // Coefficients on u_j and v_j.
            let coeffs: Vec<(u32, f64, f64)> = (0..3)
                .map(|j| {
                    let (wr, wi) = (w[j][0], w[j][1]);
                    if row == 0 { (tri[j], wr, -wi) } else { (tri[j], wi, wr) }
                })
                .collect();
            // Accumulate AᵀA and −Aᵀ(pinned part).
            let mut rhs = 0.0;
            for &(v, cu, cv) in &coeffs {
                if let Some(f) = fixed(v) {
                    rhs -= cu * f[0] + cv * f[1];
                }
            }
            for &(vi, cui, cvi) in &coeffs {
                if fixed(vi).is_some() {
                    continue;
                }
                let (iu, iv) = (index[vi as usize], free + index[vi as usize]);
                b[iu] += cui * rhs;
                b[iv] += cvi * rhs;
                for &(vj, cuj, cvj) in &coeffs {
                    if fixed(vj).is_some() {
                        continue;
                    }
                    let (ju, jv) = (index[vj as usize], free + index[vj as usize]);
                    a.add(iu, ju, cui * cuj);
                    a.add(iu, jv, cui * cvj);
                    a.add(iv, ju, cvi * cuj);
                    a.add(iv, jv, cvi * cvj);
                }
            }
        }
    }
    // Start from a projection onto the best-fit plane (CG converges from there quickly).
    let (c, cov) = covariance(&m.positions);
    let (_, ev) = sym_eigen3(cov);
    let mut x = vec![0.0; n];
    for v in 0..nv {
        if index[v] != usize::MAX {
            let r = sub(m.positions[v], c);
            x[index[v]] = dot(r, ev[2]);
            x[free + index[v]] = dot(r, ev[1]);
        }
    }
    a.solve_cg(&b, &mut x, 1e-10, 20 * n.max(100));
    let mut uv: Vec<[f64; 2]> = (0..nv as u32).map(|v| fixed(v).unwrap_or_else(|| [x[index[v as usize]], x[free + index[v as usize]]])).collect();
    // Mirror if it came out flipped (most triangles wound backwards).
    let signed = |uv: &[[f64; 2]], t: &[u32; 3]| {
        let (a, b, c) = (uv[t[0] as usize], uv[t[1] as usize], uv[t[2] as usize]);
        ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) / 2.0
    };
    let flipped = m.triangles.iter().filter(|t| signed(&uv, t) < 0.0).count();
    if flipped * 2 > m.triangles.len() {
        for p in uv.iter_mut() {
            p[1] = -p[1];
        }
    }
    // Scale to the surface's own area.
    let area_3d = m.area();
    let area_flat: f64 = m.triangles.iter().map(|t| signed(&uv, t).abs()).sum();
    let k = (area_3d / area_flat.max(1e-300)).sqrt();
    for p in uv.iter_mut() {
        p[0] *= k;
        p[1] *= k;
    }
    let mut amax: f64 = 0.0;
    let mut asum = 0.0;
    for (t, tri) in m.triangles.iter().enumerate() {
        let a3 = m.face_area(t);
        if a3 <= 0.0 {
            continue;
        }
        let r = (signed(&uv, tri).abs() / a3 - 1.0).abs();
        amax = amax.max(r);
        asum += r * a3;
    }
    let mut lmax: f64 = 0.0;
    let mut lsum = 0.0;
    let mut ln = 0usize;
    for (a_, b_) in m.edge_faces().keys().copied() {
        let l3 = dist(m.positions[a_ as usize], m.positions[b_ as usize]);
        if l3 <= 0.0 {
            continue;
        }
        let (pa, pb) = (uv[a_ as usize], uv[b_ as usize]);
        let l2 = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt();
        let r = (l2 / l3 - 1.0).abs();
        lmax = lmax.max(r);
        lsum += r;
        ln += 1;
    }
    // Turn the pattern square: its long direction along x.
    {
        let pts: Vec<crate::V3> = uv.iter().map(|p| [p[0], p[1], 0.0]).collect();
        let (c, cov) = covariance(&pts);
        let (_, ev) = sym_eigen3(cov);
        let (ax, ay) = (ev[2], [-ev[2][1], ev[2][0], 0.0]);
        for p in uv.iter_mut() {
            let r = [p[0] - c[0], p[1] - c[1], 0.0];
            *p = [dot(r, ax), dot(r, ay)];
        }
    }
    // Rest the pattern on its corner at the origin.
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &uv {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    for p in uv.iter_mut() {
        p[0] -= lo[0];
        p[1] -= lo[1];
    }
    let outlines = loops.iter().map(|l| l.iter().map(|&v| uv[v as usize]).collect()).collect();
    Ok(Flat {
        uv,
        triangles: m.triangles.clone(),
        outlines,
        area_3d,
        area_flat: area_3d,
        area_distortion_max: amax,
        area_distortion_mean: asum / area_3d.max(1e-300),
        length_distortion_max: lmax,
        length_distortion_mean: lsum / ln.max(1) as f64,
        size: [hi[0] - lo[0], hi[1] - lo[1]],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cylinder_side_unrolls_to_a_rectangle_without_stretch() {
        // An open tube r=10, h=30 cut along one line (a developable surface).
        let (segs, stacks) = (64usize, 10usize);
        let mut p = Vec::new();
        for i in 0..=stacks {
            for j in 0..=segs {
                let a = std::f64::consts::PI * 1.5 * j as f64 / segs as f64; // 270° of it, so it is open
                p.push([10.0 * a.cos(), 10.0 * a.sin(), 30.0 * i as f64 / stacks as f64]);
            }
        }
        let w = segs as u32 + 1;
        let mut t = Vec::new();
        for i in 0..stacks as u32 {
            for j in 0..segs as u32 {
                let (a, b, c, d) = (i * w + j, i * w + j + 1, (i + 1) * w + j + 1, (i + 1) * w + j);
                t.push([a, b, c]);
                t.push([a, c, d]);
            }
        }
        let m = Mesh::new(p, t);
        let f = unwrap(&m).unwrap();
        let arc = 10.0 * std::f64::consts::PI * 1.5 * (std::f64::consts::PI * 1.5 / segs as f64 / 2.0).sin() / (std::f64::consts::PI * 1.5 / segs as f64 / 2.0);
        let (long, short) = (f.size[0].max(f.size[1]), f.size[0].min(f.size[1]));
        assert!((long - arc).abs() < 0.05 * arc, "{long} vs {arc}");
        assert!((short - 30.0).abs() < 1.5, "{short}");
        assert!(f.length_distortion_max < 0.01, "{}", f.length_distortion_max);
        assert!(f.area_distortion_max < 0.01, "{}", f.area_distortion_max);
    }

    #[test]
    fn a_dome_flattens_with_measured_stretch_and_closed_surfaces_are_refused() {
        let s = crate::mesh::uv_sphere(20.0, 30, 60);
        let mut cap = s.clone();
        cap.triangles.retain(|t| t.iter().all(|&v| s.positions[v as usize][2] > 10.0));
        cap.compact();
        let f = unwrap(&cap).unwrap();
        // A spherical cap cannot flatten without stretch: some, but bounded.
        assert!(f.area_distortion_max > 0.01 && f.area_distortion_mean < 0.3, "{} {}", f.area_distortion_max, f.area_distortion_mean);
        assert_eq!(f.outlines.len(), 1);
        assert!(unwrap(&s).is_err());
    }
}
