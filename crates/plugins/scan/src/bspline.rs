//! B-spline surfaces and curves fitted to scan points (FreeCAD's
//! Reen_ApproxSurface and Reen_ApproxCurve): points are given parameters by
//! projecting them on their best-fit plane (surfaces) or by chord length
//! (curves); the control points then solve a least-squares system with a
//! light thin-plate penalty so sparse regions stay smooth (Piegl and Tiller,
//! The NURBS Book, ch. 9). The result is an exact B-spline the exact kernel
//! takes as it is.

use crate::V3;
use crate::linalg::*;
use serde::Serialize;

/// Clamped uniform knots for `n` control points of degree `p` (full vector).
pub fn knots(n: usize, p: usize) -> Vec<f64> {
    let mut k = vec![0.0; p + 1];
    let inner = n - p - 1;
    for i in 1..=inner {
        k.push(i as f64 / (inner + 1) as f64);
    }
    k.extend(std::iter::repeat_n(1.0, p + 1));
    k
}

/// The span index and the p+1 non-zero basis values at t.
pub fn basis(knots: &[f64], n: usize, p: usize, t: f64) -> (usize, Vec<f64>) {
    let t = t.clamp(knots[p], knots[n]);
    let mut span = p;
    while span < n - 1 && t >= knots[span + 1] {
        span += 1;
    }
    let mut b = vec![0.0; p + 1];
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];
    b[0] = 1.0;
    for j in 1..=p {
        left[j] = t - knots[span + 1 - j];
        right[j] = knots[span + j] - t;
        let mut saved = 0.0;
        for r in 0..j {
            let den = right[r + 1] + left[j - r];
            let tmp = if den.abs() > 1e-300 { b[r] / den } else { 0.0 };
            b[r] = saved + right[r + 1] * tmp;
            saved = left[j - r] * tmp;
        }
        b[j] = saved;
    }
    (span, b)
}

/// Distinct knot values and their multiplicities (as the exact kernel takes them).
pub fn unique_knots(k: &[f64]) -> (Vec<f64>, Vec<u32>) {
    let mut vals: Vec<f64> = Vec::new();
    let mut mult: Vec<u32> = Vec::new();
    for &x in k {
        if vals.last().is_some_and(|v| (x - v).abs() < 1e-12) {
            *mult.last_mut().unwrap() += 1;
        } else {
            vals.push(x);
            mult.push(1);
        }
    }
    (vals, mult)
}

#[derive(Clone, Debug, Serialize)]
pub struct Surface {
    pub degree: [usize; 2],
    /// Control points, nu rows of nv (row-major: poles[i * nv + j]).
    pub poles: Vec<V3>,
    pub nu: usize,
    pub nv: usize,
    pub uknots: Vec<f64>,
    pub vknots: Vec<f64>,
    /// How closely it passes the points (mm).
    pub rms: f64,
    pub max: f64,
    pub points: usize,
}

impl Surface {
    pub fn eval(&self, u: f64, v: f64) -> V3 {
        let (su, bu) = basis(&self.uknots, self.nu, self.degree[0], u);
        let (sv, bv) = basis(&self.vknots, self.nv, self.degree[1], v);
        let mut p = [0.0; 3];
        for (a, wu) in bu.iter().enumerate() {
            for (b, wv) in bv.iter().enumerate() {
                let i = su - self.degree[0] + a;
                let j = sv - self.degree[1] + b;
                p = add(p, scale(self.poles[i * self.nv + j], wu * wv));
            }
        }
        p
    }

    /// The surface as a grid mesh (n×n quads, two triangles each), for the
    /// mesh kernel and the view.
    pub fn tessellate(&self, n: usize) -> (Vec<V3>, Vec<[u32; 3]>) {
        let mut pts = Vec::with_capacity((n + 1) * (n + 1));
        for i in 0..=n {
            for j in 0..=n {
                pts.push(self.eval(i as f64 / n as f64, j as f64 / n as f64));
            }
        }
        let w = n as u32 + 1;
        let mut tris = Vec::new();
        for i in 0..n as u32 {
            for j in 0..n as u32 {
                let (a, b, c, d) = (i * w + j, (i + 1) * w + j, (i + 1) * w + j + 1, i * w + j + 1);
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        (pts, tris)
    }
}

/// Fits an nu×nv-pole B-spline surface of the given degree to points that
/// form one patch (seen from its best-fit plane without folding over).
pub fn fit_surface(points: &[V3], nu: usize, nv: usize, degree: usize, smoothing: f64) -> Result<Surface, String> {
    let p = degree.clamp(1, 5);
    let (nu, nv) = (nu.max(p + 1), nv.max(p + 1));
    if points.len() < nu * nv / 2 {
        return Err(format!("{} points are too few for {nu}×{nv} control points", points.len()));
    }
    let (c, cov) = covariance(points);
    let (_, ev) = sym_eigen3(cov);
    let (ax, ay) = (ev[2], ev[1]);
    let flat: Vec<[f64; 2]> = points.iter().map(|q| [dot(sub(*q, c), ax), dot(sub(*q, c), ay)]).collect();
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for f in &flat {
        for k in 0..2 {
            lo[k] = lo[k].min(f[k]);
            hi[k] = hi[k].max(f[k]);
        }
    }
    let params: Vec<[f64; 2]> = flat.iter().map(|f| [(f[0] - lo[0]) / (hi[0] - lo[0]).max(1e-300), (f[1] - lo[1]) / (hi[1] - lo[1]).max(1e-300)]).collect();
    let (uk, vk) = (knots(nu, p), knots(nv, p));
    let n = nu * nv;
    let mut a = Sparse::new(n);
    let mut b = vec![[0.0; 3]; n];
    let mut rows = Vec::with_capacity(points.len());
    for (q, t) in points.iter().zip(&params) {
        let (su, bu) = basis(&uk, nu, p, t[0]);
        let (sv, bv) = basis(&vk, nv, p, t[1]);
        let mut row: Vec<(usize, f64)> = Vec::with_capacity((p + 1) * (p + 1));
        for (i, wu) in bu.iter().enumerate() {
            for (j, wv) in bv.iter().enumerate() {
                row.push(((su - p + i) * nv + (sv - p + j), wu * wv));
            }
        }
        for &(i, wi) in &row {
            b[i] = add(b[i], scale(*q, wi));
            for &(j, wj) in &row {
                a.add(i, j, wi * wj);
            }
        }
        rows.push(row);
    }
    // Thin plate on the control net: second differences along u, v (and the twist).
    let lambda = smoothing * points.len() as f64 / n as f64;
    let mut stencil = |cells: &[(usize, f64)]| {
        for &(i, wi) in cells {
            for &(j, wj) in cells {
                a.add(i, j, lambda * wi * wj);
            }
        }
    };
    for i in 0..nu {
        for j in 0..nv {
            let id = |a: usize, b: usize| a * nv + b;
            if i + 2 < nu {
                stencil(&[(id(i, j), 1.0), (id(i + 1, j), -2.0), (id(i + 2, j), 1.0)]);
            }
            if j + 2 < nv {
                stencil(&[(id(i, j), 1.0), (id(i, j + 1), -2.0), (id(i, j + 2), 1.0)]);
            }
            if i + 1 < nu && j + 1 < nv {
                stencil(&[(id(i, j), 1.0), (id(i + 1, j), -1.0), (id(i, j + 1), -1.0), (id(i + 1, j + 1), 1.0)]);
            }
        }
    }
    // Tiny ridge so poles no point touches stay put.
    for i in 0..n {
        a.add(i, i, 1e-9);
    }
    let mut poles = vec![[0.0; 3]; n];
    for k in 0..3 {
        let rhs: Vec<f64> = b.iter().map(|v| v[k]).collect();
        // Start from the plane through the points.
        let mut x: Vec<f64> = (0..n)
            .map(|id| {
                let (i, j) = (id / nv, id % nv);
                let (s, t) = (i as f64 / (nu - 1) as f64, j as f64 / (nv - 1) as f64);
                let q = add(c, add(scale(ax, lo[0] + s * (hi[0] - lo[0])), scale(ay, lo[1] + t * (hi[1] - lo[1]))));
                q[k]
            })
            .collect();
        a.solve_cg(&rhs, &mut x, 1e-12, 5000);
        for i in 0..n {
            poles[i][k] = x[i];
        }
    }
    let mut err2 = 0.0;
    let mut max: f64 = 0.0;
    for (q, row) in points.iter().zip(&rows) {
        let s = row.iter().fold([0.0; 3], |s, &(i, w)| add(s, scale(poles[i], w)));
        let d = dist(s, *q);
        err2 += d * d;
        max = max.max(d);
    }
    Ok(Surface { degree: [p, p], poles, nu, nv, uknots: uk, vknots: vk, rms: (err2 / points.len() as f64).sqrt(), max, points: points.len() })
}

#[derive(Clone, Debug, Serialize)]
pub struct Curve {
    pub degree: usize,
    pub poles: Vec<V3>,
    pub knots: Vec<f64>,
    pub rms: f64,
    pub max: f64,
    /// Points on the curve, evenly in parameter (for drawing it, or for an interpolating spline).
    pub samples: Vec<V3>,
}

/// Fits an `n`-pole B-spline curve through ordered points (chord-length
/// parameters; the ends kept on the first and last points).
pub fn fit_curve(points: &[V3], n: usize, degree: usize) -> Result<Curve, String> {
    let p = degree.clamp(1, 5);
    let n = n.max(p + 1);
    if points.len() < n {
        return Err(format!("{} points are too few for {n} control points", points.len()));
    }
    let mut t = vec![0.0];
    for w in points.windows(2) {
        t.push(t.last().unwrap() + dist(w[0], w[1]));
    }
    let total = t.last().copied().unwrap_or(1.0).max(1e-300);
    for v in t.iter_mut() {
        *v /= total;
    }
    let k = knots(n, p);
    let mut a = vec![0.0; n * n];
    let mut b = vec![[0.0; 3]; n];
    let mut rows = Vec::new();
    for (q, &u) in points.iter().zip(&t) {
        let (s, bv) = basis(&k, n, p, u);
        let row: Vec<(usize, f64)> = bv.iter().enumerate().map(|(i, w)| (s - p + i, *w)).collect();
        for &(i, wi) in &row {
            b[i] = add(b[i], scale(*q, wi));
            for &(j, wj) in &row {
                a[i * n + j] += wi * wj;
            }
        }
        rows.push(row);
    }
    // Ends fixed: heavy weights on the first and last poles.
    let big = 1e6 * points.len() as f64;
    a[0] += big;
    a[n * n - 1] += big;
    b[0] = add(b[0], scale(points[0], big));
    b[n - 1] = add(b[n - 1], scale(points[points.len() - 1], big));
    let mut poles = vec![[0.0; 3]; n];
    for c in 0..3 {
        let x = solve_dense(&a, &b.iter().map(|v| v[c]).collect::<Vec<_>>(), n).ok_or("the curve fit is singular (points repeat?)")?;
        for i in 0..n {
            poles[i][c] = x[i];
        }
    }
    let mut err2 = 0.0;
    let mut max: f64 = 0.0;
    for (q, row) in points.iter().zip(&rows) {
        let s = row.iter().fold([0.0; 3], |s, &(i, w)| add(s, scale(poles[i], w)));
        let d = dist(s, *q);
        err2 += d * d;
        max = max.max(d);
    }
    let samples = (0..=4 * n)
        .map(|i| {
            let (s, bv) = basis(&k, n, p, i as f64 / (4 * n) as f64);
            bv.iter().enumerate().fold([0.0; 3], |acc, (j, w)| add(acc, scale(poles[s - p + j], *w)))
        })
        .collect();
    Ok(Curve { degree: p, poles, knots: k, rms: (err2 / points.len() as f64).sqrt(), max, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saddle_patch_is_fitted_within_microns() {
        // z = (x² − y²)/40 over 40×30, scattered.
        let pts: Vec<V3> = (0..3000).map(|i| {
            let x = (i as f64 * 0.618034).fract() * 40.0 - 20.0;
            let y = (i as f64 * 0.414214).fract() * 30.0 - 15.0;
            [x, y, (x * x - y * y) / 40.0]
        }).collect();
        let s = fit_surface(&pts, 6, 6, 3, 1e-6).unwrap();
        // A quadratic is exactly representable by a cubic B-spline.
        assert!(s.rms < 1e-3, "rms {}", s.rms);
        assert!(s.max < 5e-3, "max {}", s.max);
        let (u, m) = unique_knots(&s.uknots);
        assert_eq!(u.len(), 4);
        assert_eq!(m, vec![4, 1, 1, 4]);
        let (g, _) = s.tessellate(8);
        assert_eq!(g.len(), 81);
    }

    #[test]
    fn a_noisy_arc_gives_a_smooth_curve() {
        let pts: Vec<V3> = (0..200).map(|i| {
            let a = i as f64 / 199.0 * std::f64::consts::PI;
            let noise = ((i * 7919 % 100) as f64 / 100.0 - 0.5) * 0.02;
            [(20.0 + noise) * a.cos(), (20.0 + noise) * a.sin(), 0.0]
        }).collect();
        let c = fit_curve(&pts, 8, 3).unwrap();
        assert!(c.rms < 0.02, "{}", c.rms);
        assert!(dist(c.samples[0], pts[0]) < 1e-3 && dist(*c.samples.last().unwrap(), pts[199]) < 1e-3);
        let worst = c.samples.iter().map(|p| (norm(*p) - 20.0).abs()).fold(0.0, f64::max);
        assert!(worst < 0.05, "{worst}");
    }
}
