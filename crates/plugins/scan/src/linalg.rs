//! Small vector algebra, symmetric eigen-decomposition (Jacobi), dense
//! least squares for fitting, and a conjugate-gradient solver for the large
//! sparse systems (unwrapping, B-spline fitting, Poisson).

use crate::V3;

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn dist(a: V3, b: V3) -> f64 {
    norm(sub(a, b))
}
pub fn dist2(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}
pub fn unit(a: V3) -> V3 {
    let n = norm(a);
    if n > 1e-300 { scale(a, 1.0 / n) } else { [0.0, 0.0, 0.0] }
}
pub fn lerp(a: V3, b: V3, t: f64) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
pub fn mid(a: V3, b: V3) -> V3 {
    lerp(a, b, 0.5)
}

/// Any unit vector square to `n`.
pub fn perpendicular(n: V3) -> V3 {
    let a = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    unit(cross(n, a))
}

/// Two unit vectors completing `n` to a right-handed frame.
pub fn frame(n: V3) -> (V3, V3) {
    let u = perpendicular(n);
    (u, cross(n, u))
}

/// Twice the triangle's area, along its normal.
pub fn tri_normal(a: V3, b: V3, c: V3) -> V3 {
    cross(sub(b, a), sub(c, a))
}

/// Eigenvalues (ascending) and unit eigenvectors (columns, in the same order)
/// of a symmetric 3×3 matrix, by cyclic Jacobi rotations.
pub fn sym_eigen3(m: [[f64; 3]; 3]) -> ([f64; 3], [V3; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < 1e-300 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for row in v.iter_mut() {
                let (vkp, vkq) = (row[p], row[q]);
                row[p] = c * vkp - s * vkq;
                row[q] = s * vkp + c * vkq;
            }
        }
    }
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let vals = order.map(|i| a[i][i]);
    let vecs = order.map(|i| unit([v[0][i], v[1][i], v[2][i]]));
    (vals, vecs)
}

/// The centroid and covariance of points.
pub fn covariance(points: &[V3]) -> (V3, [[f64; 3]; 3]) {
    let n = points.len().max(1) as f64;
    let c = scale(points.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / n);
    let mut m = [[0.0; 3]; 3];
    for p in points {
        let d = sub(*p, c);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += d[i] * d[j];
            }
        }
    }
    for row in m.iter_mut() {
        for v in row.iter_mut() {
            *v /= n;
        }
    }
    (c, m)
}

/// Solves the dense system `a x = b` (n×n, row-major) by Gaussian
/// elimination with partial pivoting; None when singular.
pub fn solve_dense(a: &[f64], b: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut m = a.to_vec();
    let mut x = b.to_vec();
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| m[i * n + col].abs().total_cmp(&m[j * n + col].abs()))?;
        if m[piv * n + col].abs() < 1e-300 {
            return None;
        }
        if piv != col {
            for k in 0..n {
                m.swap(piv * n + k, col * n + k);
            }
            x.swap(piv, col);
        }
        for r in col + 1..n {
            let f = m[r * n + col] / m[col * n + col];
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                m[r * n + k] -= f * m[col * n + k];
            }
            x[r] -= f * x[col];
        }
    }
    for col in (0..n).rev() {
        let mut s = x[col];
        for k in col + 1..n {
            s -= m[col * n + k] * x[k];
        }
        x[col] = s / m[col * n + col];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Levenberg–Marquardt on residuals `f(params)` with a numerical Jacobian:
/// the parameters minimising the sum of squares, and its RMS.
pub fn levenberg_marquardt(mut p: Vec<f64>, iterations: usize, f: impl Fn(&[f64], &mut Vec<f64>)) -> (Vec<f64>, f64) {
    let n = p.len();
    let mut r = Vec::new();
    f(&p, &mut r);
    let m = r.len().max(1);
    let mut cost: f64 = r.iter().map(|v| v * v).sum();
    let mut lambda = 1e-3;
    let mut jac = vec![0.0; m * n];
    let mut r2 = Vec::new();
    for _ in 0..iterations {
        // Forward differences, scaled to each parameter.
        for j in 0..n {
            let h = 1e-7 * p[j].abs().max(1e-3);
            let mut q = p.clone();
            q[j] += h;
            f(&q, &mut r2);
            for i in 0..m.min(r2.len()) {
                jac[i * n + j] = (r2[i] - r[i]) / h;
            }
        }
        let mut jtj = vec![0.0; n * n];
        let mut jtr = vec![0.0; n];
        for i in 0..m.min(r.len()) {
            let row = &jac[i * n..i * n + n];
            for a in 0..n {
                jtr[a] += row[a] * r[i];
                for b in a..n {
                    jtj[a * n + b] += row[a] * row[b];
                }
            }
        }
        for a in 0..n {
            for b in 0..a {
                jtj[a * n + b] = jtj[b * n + a];
            }
        }
        let mut improved = false;
        for _ in 0..12 {
            let mut aug = jtj.clone();
            for a in 0..n {
                aug[a * n + a] += lambda * jtj[a * n + a].max(1e-12);
            }
            let Some(step) = solve_dense(&aug, &jtr.iter().map(|v| -v).collect::<Vec<_>>(), n) else {
                lambda *= 10.0;
                continue;
            };
            let q: Vec<f64> = p.iter().zip(&step).map(|(a, b)| a + b).collect();
            f(&q, &mut r2);
            let c: f64 = r2.iter().map(|v| v * v).sum();
            if c.is_finite() && c < cost {
                let small = (cost - c) < 1e-14 * cost.max(1e-300);
                p = q;
                std::mem::swap(&mut r, &mut r2);
                cost = c;
                lambda = (lambda * 0.3).max(1e-12);
                improved = !small;
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    (p, (cost / m as f64).sqrt())
}

/// A sparse symmetric matrix as rows of (column, value).
#[derive(Clone, Debug, Default)]
pub struct Sparse {
    pub rows: Vec<Vec<(usize, f64)>>,
}

impl Sparse {
    pub fn new(n: usize) -> Sparse {
        Sparse { rows: vec![Vec::new(); n] }
    }

    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        let row = &mut self.rows[i];
        match row.iter_mut().find(|(c, _)| *c == j) {
            Some(e) => e.1 += v,
            None => row.push((j, v)),
        }
    }

    pub fn mul(&self, x: &[f64], out: &mut [f64]) {
        for (i, row) in self.rows.iter().enumerate() {
            out[i] = row.iter().map(|(j, v)| v * x[*j]).sum();
        }
    }

    /// Solves `self x = b` by Jacobi-preconditioned conjugate gradients from
    /// `x`; returns the iterations used.
    pub fn solve_cg(&self, b: &[f64], x: &mut [f64], tolerance: f64, max_iter: usize) -> usize {
        let n = b.len();
        let diag: Vec<f64> = self.rows.iter().enumerate().map(|(i, row)| row.iter().find(|(j, _)| *j == i).map(|e| e.1).filter(|v| v.abs() > 1e-300).unwrap_or(1.0)).collect();
        let mut r = vec![0.0; n];
        self.mul(x, &mut r);
        for i in 0..n {
            r[i] = b[i] - r[i];
        }
        let bnorm = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
        let mut z: Vec<f64> = r.iter().zip(&diag).map(|(a, d)| a / d).collect();
        let mut p = z.clone();
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let mut ap = vec![0.0; n];
        for it in 0..max_iter {
            let rn = r.iter().map(|v| v * v).sum::<f64>().sqrt();
            if rn <= tolerance * bnorm {
                return it;
            }
            self.mul(&p, &mut ap);
            let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
            if pap.abs() < 1e-300 {
                return it;
            }
            let alpha = rz / pap;
            for i in 0..n {
                x[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
            }
            for i in 0..n {
                z[i] = r[i] / diag[i];
            }
            let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            let beta = rz2 / rz;
            rz = rz2;
            for i in 0..n {
                p[i] = z[i] + beta * p[i];
            }
        }
        max_iter
    }
}

/// Rotation matrix (rows) from a unit quaternion [w, x, y, z].
pub fn quat_matrix(q: [f64; 4]) -> [[f64; 3]; 3] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt().max(1e-300);
    let [w, x, y, z] = q.map(|v| v / n);
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

pub fn mat_vec(m: &[[f64; 3]; 3], v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

pub fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

/// Rotation (rows) about a unit axis by an angle (radians).
pub fn axis_angle(axis: V3, angle: f64) -> [[f64; 3]; 3] {
    let h = (angle / 2.0).sin();
    let a = unit(axis);
    quat_matrix([(angle / 2.0).cos(), a[0] * h, a[1] * h, a[2] * h])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eigen_of_a_flat_cloud_finds_its_normal() {
        let pts: Vec<V3> = (0..100).map(|i| [(i % 10) as f64, (i / 10) as f64 * 2.0, 0.0]).collect();
        let (_, c) = covariance(&pts);
        let (vals, vecs) = sym_eigen3(c);
        assert!(vals[0].abs() < 1e-12);
        assert!((vecs[0][2].abs() - 1.0).abs() < 1e-9);
        assert!((vecs[2][1].abs() - 1.0).abs() < 1e-9, "the long axis is y");
    }

    #[test]
    fn lm_fits_a_line() {
        let xs: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let (p, rms) = levenberg_marquardt(vec![0.0, 0.0], 50, |p, r| {
            r.clear();
            r.extend(xs.iter().map(|x| p[0] * x + p[1] - (3.0 * x - 2.0)));
        });
        assert!((p[0] - 3.0).abs() < 1e-6 && (p[1] + 2.0).abs() < 1e-6 && rms < 1e-6, "{p:?}");
    }

    #[test]
    fn cg_solves_a_laplacian() {
        let n = 50;
        let mut a = Sparse::new(n);
        for i in 0..n {
            a.add(i, i, 2.0);
            if i > 0 {
                a.add(i, i - 1, -1.0);
            }
            if i + 1 < n {
                a.add(i, i + 1, -1.0);
            }
        }
        let want: Vec<f64> = (0..n).map(|i| (i as f64).sin()).collect();
        let mut b = vec![0.0; n];
        a.mul(&want, &mut b);
        let mut x = vec![0.0; n];
        a.solve_cg(&b, &mut x, 1e-12, 500);
        assert!(x.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-8));
    }
}
