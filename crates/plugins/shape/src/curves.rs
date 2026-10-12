//! Sketch curves as geometry: lines, circular arcs, conics (ellipse,
//! hyperbola, parabola), Béziers and rational B-splines, evaluated along
//! their length, searched for the point nearest another, split, and turned
//! into the exact rational B-splines the kernel and the STEP file take.
//!
//! Conics are exact as rational quadratics (an ellipse is four of them, each
//! a quarter turn), so an ellipse in a sketch is an ellipse in the part and
//! in the STEP file, not a polygon or a fit.

use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, TAU};

pub type P = [f64; 2];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: P) -> f64 {
    a[0].hypot(a[1])
}
fn add_scaled(p: P, v: P, s: f64) -> P {
    [p[0] + v[0] * s, p[1] + v[1] * s]
}
/// The axis square to `u`, turned a quarter anticlockwise.
pub fn perp(u: P) -> P {
    [-u[1], u[0]]
}

/// A rational B-spline, clamped: it starts at its first pole and ends at its last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Nurbs {
    pub degree: usize,
    pub poles: Vec<P>,
    pub weights: Vec<f64>,
    /// The full knot vector: poles + degree + 1 values, each end repeated degree + 1 times.
    pub knots: Vec<f64>,
}

/// Homogeneous point: (x·w, y·w, w).
type H = [f64; 3];

fn lerp3(a: H, b: H, t: f64) -> H {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// The non-zero basis functions of degree `p` at `u` (Cox–de Boor), and the span they start at.
pub fn basis(knots: &[f64], p: usize, u: f64) -> (usize, Vec<f64>) {
    let n = knots.len() - p - 2; // last pole index
    let k = span_of(knots, p, n, u);
    let mut nb = vec![0.0; p + 1];
    let (mut left, mut right) = (vec![0.0; p + 1], vec![0.0; p + 1]);
    nb[0] = 1.0;
    for j in 1..=p {
        left[j] = u - knots[k + 1 - j];
        right[j] = knots[k + j] - u;
        let mut saved = 0.0;
        for r in 0..j {
            let den = right[r + 1] + left[j - r];
            let tmp = if den.abs() < 1e-300 { 0.0 } else { nb[r] / den };
            nb[r] = saved + right[r + 1] * tmp;
            saved = left[j - r] * tmp;
        }
        nb[j] = saved;
    }
    (k - p, nb)
}

/// The knot span holding `u`: `k` with knots[k] ≤ u < knots[k+1], kept within the curve.
fn span_of(knots: &[f64], p: usize, n: usize, u: f64) -> usize {
    if u >= knots[n + 1] {
        // The last span with any length.
        let mut k = n;
        while k > p && knots[k] >= knots[n + 1] {
            k -= 1;
        }
        return k;
    }
    if u <= knots[p] {
        let mut k = p;
        while k < n && knots[k + 1] <= knots[p] {
            k += 1;
        }
        return k;
    }
    let (mut lo, mut hi) = (p, n + 1);
    let mut mid = (lo + hi) / 2;
    while u < knots[mid] || u >= knots[mid + 1] {
        if u < knots[mid] {
            hi = mid;
        } else {
            lo = mid;
        }
        mid = (lo + hi) / 2;
    }
    mid
}

impl Nurbs {
    /// A plain (non-rational) B-spline.
    pub fn new(degree: usize, poles: Vec<P>, knots: Vec<f64>) -> Nurbs {
        let weights = vec![1.0; poles.len()];
        Nurbs { degree, poles, weights, knots }
    }

    pub fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.poles.len()])
    }

    fn h(&self, i: usize) -> H {
        let w = self.weights[i];
        [self.poles[i][0] * w, self.poles[i][1] * w, w]
    }

    fn hpoint(&self, u: f64) -> H {
        let p = self.degree;
        let n = self.poles.len() - 1;
        let k = span_of(&self.knots, p, n, u);
        let mut d: Vec<H> = (0..=p).map(|j| self.h(j + k - p)).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = j + k - p;
                let den = self.knots[j + 1 + k - r] - self.knots[i];
                let a = if den.abs() < 1e-300 { 0.0 } else { (u - self.knots[i]) / den };
                d[j] = lerp3(d[j - 1], d[j], a);
            }
        }
        d[p]
    }

    pub fn point(&self, u: f64) -> P {
        let h = self.hpoint(u);
        [h[0] / h[2], h[1] / h[2]]
    }

    /// The exact first derivative: the homogeneous curve's derivative (a
    /// spline one degree lower) by the quotient rule.
    pub fn deriv(&self, u: f64) -> P {
        let p = self.degree;
        let h = self.hpoint(u);
        let n = self.poles.len();
        let q: Vec<H> = (0..n - 1)
            .map(|i| {
                let den = self.knots[i + p + 1] - self.knots[i + 1];
                let f = if den.abs() < 1e-300 { 0.0 } else { p as f64 / den };
                let (a, b) = (self.h(i), self.h(i + 1));
                [(b[0] - a[0]) * f, (b[1] - a[1]) * f, (b[2] - a[2]) * f]
            })
            .collect();
        let dh = if p == 1 {
            let k = span_of(&self.knots, p, n - 1, u);
            q[k - 1]
        } else {
            let d = Nurbs { degree: p - 1, poles: vec![[0.0; 2]; n - 1], weights: vec![1.0; n - 1], knots: self.knots[1..self.knots.len() - 1].to_vec() };
            d.eval_h(&q, u)
        };
        let w = h[2];
        [(dh[0] * w - h[0] * dh[2]) / (w * w), (dh[1] * w - h[1] * dh[2]) / (w * w)]
    }

    /// De Boor on homogeneous control points `q` with this curve's knots and degree.
    fn eval_h(&self, q: &[H], u: f64) -> H {
        let p = self.degree;
        let n = q.len() - 1;
        let k = span_of(&self.knots, p, n, u);
        let mut d: Vec<H> = (0..=p).map(|j| q[j + k - p]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = j + k - p;
                let den = self.knots[j + 1 + k - r] - self.knots[i];
                let a = if den.abs() < 1e-300 { 0.0 } else { (u - self.knots[i]) / den };
                d[j] = lerp3(d[j - 1], d[j], a);
            }
        }
        d[p]
    }

    /// The distinct knots and how many times each repeats.
    pub fn knots_mults(&self) -> (Vec<f64>, Vec<u32>) {
        let (mut k, mut m): (Vec<f64>, Vec<u32>) = (vec![], vec![]);
        for &t in &self.knots {
            match k.last() {
                Some(&l) if (t - l).abs() <= 1e-12 * (1.0 + l.abs()) => *m.last_mut().unwrap() += 1,
                _ => {
                    k.push(t);
                    m.push(1);
                }
            }
        }
        (k, m)
    }

    /// The full knot vector from distinct knots and multiplicities.
    pub fn flat_knots(knots: &[f64], mults: &[u32]) -> Vec<f64> {
        knots.iter().zip(mults).flat_map(|(k, m)| std::iter::repeat_n(*k, *m as usize)).collect()
    }

    /// The same curve with knot `u` inserted once (Boehm), inside the domain.
    pub fn insert(&self, u: f64) -> Nurbs {
        let p = self.degree;
        let n = self.poles.len() - 1;
        let k = span_of(&self.knots, p, n, u);
        let mut q: Vec<H> = Vec::with_capacity(n + 2);
        for i in 0..=n + 1 {
            if i + p <= k {
                q.push(self.h(i));
            } else if i > k {
                q.push(self.h(i - 1));
            } else {
                let den = self.knots[i + p] - self.knots[i];
                let a = if den.abs() < 1e-300 { 0.0 } else { (u - self.knots[i]) / den };
                q.push(lerp3(self.h(i - 1), self.h(i), a));
            }
        }
        let mut knots = self.knots.clone();
        knots.insert(k + 1, u);
        from_h(p, &q, knots)
    }

    /// How many times `u` is a knot already.
    pub fn multiplicity(&self, u: f64) -> usize {
        self.knots.iter().filter(|k| (**k - u).abs() <= 1e-12 * (1.0 + u.abs())).count()
    }

    /// Two curves, before and after `u` (strictly inside the domain).
    pub fn split(&self, u: f64) -> (Nurbs, Nurbs) {
        let p = self.degree;
        let mut c = self.clone();
        let have = c.multiplicity(u);
        for _ in have..p {
            c = c.insert(u);
        }
        // Snap the inserted values to u exactly, so the split index is found.
        for k in &mut c.knots {
            if (*k - u).abs() <= 1e-12 * (1.0 + u.abs()) {
                *k = u;
            }
        }
        let f = c.knots.iter().position(|k| *k == u).unwrap();
        let mut lk: Vec<f64> = c.knots[..f].to_vec();
        lk.extend(std::iter::repeat_n(u, p + 1));
        let left = Nurbs { degree: p, poles: c.poles[..f].to_vec(), weights: c.weights[..f].to_vec(), knots: lk };
        let mut rk: Vec<f64> = vec![u; p + 1];
        rk.extend_from_slice(&c.knots[f + p..]);
        let right = Nurbs { degree: p, poles: c.poles[f - 1..].to_vec(), weights: c.weights[f - 1..].to_vec(), knots: rk };
        (left, right)
    }

    /// The same curve run the other way.
    pub fn reversed(&self) -> Nurbs {
        let (a, b) = (self.knots[0], *self.knots.last().unwrap());
        Nurbs {
            degree: self.degree,
            poles: self.poles.iter().rev().copied().collect(),
            weights: self.weights.iter().rev().copied().collect(),
            knots: self.knots.iter().rev().map(|k| a + b - k).collect(),
        }
    }

    /// A periodic (closed, smooth all round) B-spline on uniform knots,
    /// written as a clamped one that starts and ends at the same point.
    pub fn periodic(degree: usize, poles: &[P], weights: &[f64]) -> Nurbs {
        let n = poles.len();
        let p = degree;
        let mut ext: Vec<P> = poles.to_vec();
        let mut w: Vec<f64> = weights.to_vec();
        for i in 0..p {
            ext.push(poles[i % n]);
            w.push(weights[i % n]);
        }
        let knots: Vec<f64> = (0..ext.len() + p + 1).map(|i| i as f64).collect();
        let c = Nurbs { degree: p, poles: ext, weights: w, knots };
        let (_, right) = c.split(p as f64);
        let (mid, _) = right.split((n + p) as f64);
        mid
    }

    /// The same curve one degree higher (exact: its knots each repeat once more).
    pub fn elevated(&self) -> Nurbs {
        let (k, m) = self.knots_mults();
        let last = m.len() - 1;
        let mults: Vec<u32> = m.iter().enumerate().map(|(i, &x)| if i == 0 || i == last { self.degree as u32 + 2 } else { x + 1 }).collect();
        let knots = Nurbs::flat_knots(&k, &mults);
        self.refit(self.degree + 1, knots)
    }

    /// The nearest curve one degree lower (approximate unless the curve allows it).
    pub fn lowered(&self) -> Option<Nurbs> {
        if self.degree <= 1 {
            return None;
        }
        let q = self.degree - 1;
        let (k, m) = self.knots_mults();
        let last = m.len() - 1;
        let mults: Vec<u32> = m.iter().enumerate().map(|(i, &x)| if i == 0 || i == last { q as u32 + 1 } else { x.saturating_sub(1).min(q as u32) }).collect();
        let (k, mults): (Vec<f64>, Vec<u32>) = k.into_iter().zip(mults).filter(|(_, m)| *m > 0).unzip();
        Some(self.refit(q, Nurbs::flat_knots(&k, &mults)))
    }

    /// The nearest curve of `degree` on `knots` (exact when the curve lies in
    /// that space), found in homogeneous coordinates: interpolation at the
    /// Greville points when the space can hold the curve, else least squares
    /// over many samples with the ends held.
    pub fn refit(&self, degree: usize, knots: Vec<f64>) -> Nurbs {
        let count = knots.len() - degree - 1;
        let (a, b) = self.domain();
        let map = |t: f64| a + (t - knots[0]) / (knots[knots.len() - 1] - knots[0]) * (b - a);
        // Samples: Greville points plus many more for a least-squares fit.
        let greville: Vec<f64> = (0..count).map(|i| knots[i + 1..=i + degree].iter().sum::<f64>() / degree.max(1) as f64).collect();
        let dense: Vec<f64> = (0..=count * 8).map(|i| knots[0] + (knots[knots.len() - 1] - knots[0]) * i as f64 / (count * 8) as f64).collect();
        let rows: Vec<(Vec<f64>, H)> = dense
            .iter()
            .chain(&greville)
            .map(|&t| {
                let (s, nb) = basis(&knots, degree, t);
                let mut row = vec![0.0; count];
                for (j, v) in nb.iter().enumerate() {
                    row[s + j] = *v;
                }
                (row, self.hpoint(map(t)))
            })
            .collect();
        // Normal equations, with the two ends held exactly (big weight).
        let mut m = vec![vec![0.0; count]; count];
        let mut rhs = vec![[0.0; 3]; count];
        for (row, h) in &rows {
            for i in 0..count {
                if row[i] == 0.0 {
                    continue;
                }
                for j in 0..count {
                    m[i][j] += row[i] * row[j];
                }
                for c in 0..3 {
                    rhs[i][c] += row[i] * h[c];
                }
            }
        }
        let big = 1e8;
        let (e0, e1) = (self.hpoint(a), self.hpoint(b));
        m[0][0] += big;
        m[count - 1][count - 1] += big;
        for c in 0..3 {
            rhs[0][c] += big * e0[c];
            rhs[count - 1][c] += big * e1[c];
        }
        let mut q = vec![[0.0; 3]; count];
        for c in 0..3 {
            let col: Vec<f64> = rhs.iter().map(|r| r[c]).collect();
            if let Some(x) = solve_dense(m.clone(), col) {
                for i in 0..count {
                    q[i][c] = x[i];
                }
            }
        }
        // Back to the curve's own parameters.
        let knots = knots.iter().map(|t| map(*t)).collect();
        from_h(degree, &q, knots)
    }

    /// A polyline within about `tolerance` of the curve.
    pub fn polyline(&self, tolerance: f64) -> Vec<P> {
        let (k, _) = self.knots_mults();
        let mut out = vec![];
        for w in k.windows(2) {
            // Steps per span from its control polygon's bend.
            let (s, _) = basis(&self.knots, self.degree, (w[0] + w[1]) / 2.0);
            let pts = &self.poles[s..=s + self.degree];
            let bend: f64 = pts.windows(3).map(|q| len([q[0][0] - 2.0 * q[1][0] + q[2][0], q[0][1] - 2.0 * q[1][1] + q[2][1]])).fold(0.0, f64::max);
            let wmax = self.weights.iter().fold(1.0f64, |m, w| m.max(*w)) / self.weights.iter().fold(f64::INFINITY, |m, w| m.min(*w)).max(1e-9);
            let n = ((bend * wmax * self.degree as f64 / tolerance.max(1e-9)).sqrt().ceil() as usize).clamp(2, 512);
            for i in 0..n {
                out.push(self.point(w[0] + (w[1] - w[0]) * i as f64 / n as f64));
            }
        }
        out.push(self.point(self.domain().1));
        out
    }
}

/// One curve from `a` then `b` (b starting where a ends): the lower
/// degree raised to match, the join a knot repeated `degree` times.
pub fn join(a: &Nurbs, b: &Nurbs) -> Nurbs {
    let (mut a, mut b) = (a.clone(), b.clone());
    while a.degree < b.degree {
        a = a.elevated();
    }
    while b.degree < a.degree {
        b = b.elevated();
    }
    let p = a.degree;
    // The same curve with its weights scaled, so they agree at the join.
    let s = a.weights.last().unwrap() / b.weights[0];
    let shift = a.knots.last().unwrap() - b.knots[0];
    let mut knots = a.knots[..a.knots.len() - 1].to_vec();
    knots.extend(b.knots[p + 1..].iter().map(|k| k + shift));
    let mut poles = a.poles.clone();
    poles.extend_from_slice(&b.poles[1..]);
    let mut weights = a.weights.clone();
    weights.extend(b.weights[1..].iter().map(|w| w * s));
    Nurbs { degree: p, poles, weights, knots }
}

fn from_h(degree: usize, q: &[H], knots: Vec<f64>) -> Nurbs {
    Nurbs {
        degree,
        poles: q.iter().map(|h| [h[0] / h[2], h[1] / h[2]]).collect(),
        weights: q.iter().map(|h| h[2]).collect(),
        knots,
    }
}

/// Solves a square dense system (Gaussian elimination, partial pivoting).
pub fn solve_dense(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row][row];
    }
    Some(x)
}

/// A B-spline through `points` (cubic, or lower for few points): open with
/// chord-length spacing, or closed and smooth all round (periodic). Returns
/// the poles (and for an open curve its knot vector).
pub fn interpolate(points: &[P], periodic: bool) -> Result<(usize, Vec<P>, Vec<f64>), String> {
    let n = points.len();
    if n < 2 || (periodic && n < 3) {
        return Err("a curve through points needs at least two points (three when closed)".into());
    }
    if periodic {
        // Uniform cubic: (P[i-1] + 4 P[i] + P[i+1]) / 6 = Q[i], all round.
        let p = 3;
        let mut m = vec![vec![0.0; n]; n];
        for i in 0..n {
            m[i][(i + n - 1) % n] += 1.0 / 6.0;
            m[i][i] += 4.0 / 6.0;
            m[i][(i + 1) % n] += 1.0 / 6.0;
        }
        let xs = solve_dense(m.clone(), points.iter().map(|q| q[0]).collect()).ok_or("the points do not make a curve")?;
        let ys = solve_dense(m, points.iter().map(|q| q[1]).collect()).ok_or("the points do not make a curve")?;
        // The curve passes Q[i] at the start of span i; shift so pole i-1, i, i+1 shape Q[i].
        let mut poles: Vec<P> = (0..n).map(|i| [xs[i], ys[i]]).collect();
        // Periodic evaluation starts at pole 0's span, which passes (P0 + 4 P1 + P2)/6 = Q1: rotate.
        poles.rotate_right(1);
        return Ok((p, poles, vec![]));
    }
    let p = 3.min(n - 1);
    // Chord-length parameters.
    let mut t = vec![0.0; n];
    for i in 1..n {
        t[i] = t[i - 1] + len(sub(points[i], points[i - 1])).max(1e-12);
    }
    let total = t[n - 1];
    for v in &mut t {
        *v /= total;
    }
    // Knots by averaging (Piegl & Tiller 9.8).
    let mut knots = vec![0.0; p + 1];
    for j in 1..n - p {
        knots.push(t[j..j + p].iter().sum::<f64>() / p as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, p + 1));
    let mut m = vec![vec![0.0; n]; n];
    for (i, &u) in t.iter().enumerate() {
        let (s, nb) = basis(&knots, p, u);
        for (j, v) in nb.iter().enumerate() {
            m[i][s + j] = *v;
        }
    }
    let xs = solve_dense(m.clone(), points.iter().map(|q| q[0]).collect()).ok_or("the points do not make a curve")?;
    let ys = solve_dense(m, points.iter().map(|q| q[1]).collect()).ok_or("the points do not make a curve")?;
    Ok((p, (0..n).map(|i| [xs[i], ys[i]]).collect(), knots))
}

/// A curve as geometry, its parameter running from `range().0` to `range().1`.
#[derive(Clone, Debug)]
pub enum Geo {
    /// From a to b; t from 0 to 1.
    Line(P, P),
    /// Centre, radius, start and end angle (end > start; a whole circle spans TAU).
    Arc(P, f64, f64, f64),
    /// Centre, major axis direction (unit), semi-axes a and b, and the range
    /// of the angle t: c + a cos t·u + b sin t·v.
    Ellipse(P, P, f64, f64, f64, f64),
    /// One branch: c + a cosh t·u + b sinh t·v.
    Hyperbola(P, P, f64, f64, f64, f64),
    /// Vertex, axis (unit, towards the focus), focal length f, range of s:
    /// vertex + s²/(4f)·u + s·v.
    Parabola(P, P, f64, f64, f64),
    Nurbs(Nurbs),
}

impl Geo {
    pub fn range(&self) -> (f64, f64) {
        match self {
            Geo::Line(..) => (0.0, 1.0),
            Geo::Arc(_, _, a0, a1) | Geo::Ellipse(_, _, _, _, a0, a1) | Geo::Hyperbola(_, _, _, _, a0, a1) | Geo::Parabola(_, _, _, a0, a1) => (*a0, *a1),
            Geo::Nurbs(n) => n.domain(),
        }
    }

    pub fn point(&self, t: f64) -> P {
        match self {
            Geo::Line(a, b) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            Geo::Arc(c, r, ..) => [c[0] + r * t.cos(), c[1] + r * t.sin()],
            Geo::Ellipse(c, u, a, b, ..) => add_scaled(add_scaled(*c, *u, a * t.cos()), perp(*u), b * t.sin()),
            Geo::Hyperbola(c, u, a, b, ..) => add_scaled(add_scaled(*c, *u, a * t.cosh()), perp(*u), b * t.sinh()),
            Geo::Parabola(v, u, f, ..) => add_scaled(add_scaled(*v, *u, t * t / (4.0 * f)), perp(*u), t),
            Geo::Nurbs(n) => n.point(t),
        }
    }

    pub fn deriv(&self, t: f64) -> P {
        match self {
            Geo::Line(a, b) => sub(*b, *a),
            Geo::Arc(_, r, ..) => [-r * t.sin(), r * t.cos()],
            Geo::Ellipse(_, u, a, b, ..) => add_scaled([u[0] * -a * t.sin(), u[1] * -a * t.sin()], perp(*u), b * t.cos()),
            Geo::Hyperbola(_, u, a, b, ..) => add_scaled([u[0] * a * t.sinh(), u[1] * a * t.sinh()], perp(*u), b * t.cosh()),
            Geo::Parabola(_, u, f, ..) => add_scaled([u[0] * t / (2.0 * f), u[1] * t / (2.0 * f)], perp(*u), 1.0),
            Geo::Nurbs(n) => n.deriv(t),
        }
    }

    /// Whether the curve closes on itself (a whole circle or ellipse, a closed spline).
    pub fn closed(&self) -> bool {
        let (a, b) = self.range();
        match self {
            Geo::Arc(..) | Geo::Ellipse(..) => b - a >= TAU - 1e-12,
            Geo::Nurbs(n) => len(sub(n.poles[0], *n.poles.last().unwrap())) < 1e-9,
            _ => false,
        }
    }

    /// Points at even steps of the parameter (n + 1 of them, ends included).
    pub fn sample(&self, n: usize) -> Vec<P> {
        let (a, b) = self.range();
        (0..=n).map(|i| self.point(a + (b - a) * i as f64 / n as f64)).collect()
    }

    /// The parameter of the nearest point to `p`. `whole`: on the whole
    /// conic (round past an arc's ends), else within the curve's range.
    pub fn closest(&self, p: P, whole: bool) -> f64 {
        let (mut a, mut b) = self.range();
        match self {
            Geo::Line(s, e) => {
                let d = sub(*e, *s);
                let t = dot(sub(p, *s), d) / dot(d, d).max(1e-300);
                return if whole { t } else { t.clamp(0.0, 1.0) };
            }
            Geo::Arc(c, _, a0, a1) => {
                let ang = (p[1] - c[1]).atan2(p[0] - c[0]);
                let t = a0 + (ang - a0).rem_euclid(TAU);
                if whole || t <= *a1 {
                    return t;
                }
                // Past the end: whichever end is nearer.
                return if (t - a1) < (a0 + TAU - t) { *a1 } else { *a0 };
            }
            Geo::Ellipse(..) if whole => {
                a = 0.0;
                b = TAU;
            }
            Geo::Hyperbola(c, u, _, bb, ..) if whole => {
                let y = dot(sub(p, *c), perp(*u)) / bb;
                let g = y.asinh();
                a = g - 2.0;
                b = g + 2.0;
            }
            Geo::Parabola(v, u, ..) if whole => {
                let s = dot(sub(p, *v), perp(*u));
                let w = s.abs().max(1.0);
                a = s - w;
                b = s + w;
            }
            _ => {}
        }
        let n = 64;
        let mut best = (f64::INFINITY, a);
        for i in 0..=n {
            let t = a + (b - a) * i as f64 / n as f64;
            let d = len(sub(self.point(t), p));
            if d < best.0 {
                best = (d, t);
            }
        }
        // Newton on (C(t) − p)·C'(t) = 0.
        let mut t = best.1;
        let h = 1e-6 * (b - a).abs().max(1e-9);
        for _ in 0..30 {
            let g = |t: f64| dot(sub(self.point(t), p), self.deriv(t));
            let (g0, g1) = (g(t), (g(t + h) - g(t - h)) / (2.0 * h));
            if g1.abs() < 1e-300 {
                break;
            }
            let mut next = t - g0 / g1;
            let periodic = matches!(self, Geo::Ellipse(..)) && whole;
            if !periodic {
                next = next.clamp(a.min(b), a.max(b));
            }
            if (next - t).abs() < 1e-14 * (1.0 + t.abs()) {
                t = next;
                break;
            }
            // Only take steps that bring the point nearer.
            if len(sub(self.point(next), p)) > len(sub(self.point(t), p)) + 1e-12 {
                break;
            }
            t = next;
        }
        t
    }

    /// Signed distance from the curve to `p`, square to the curve at its
    /// nearest point (positive to the left, going along it).
    pub fn signed_distance(&self, p: P, whole: bool) -> f64 {
        let t = self.closest(p, whole);
        let d = self.deriv(t);
        let l = len(d).max(1e-300);
        let q = sub(p, self.point(t));
        let along = (d[0] * q[1] - d[1] * q[0]) / l;
        // At an end the nearest point need not be square: use the true distance, signed.
        let gap = len(q);
        if (dot(q, d) / l).abs() > 1e-6 * (1.0 + gap) {
            return gap.copysign(along);
        }
        along
    }

    /// The exact curve as a rational B-spline.
    pub fn to_nurbs(&self) -> Nurbs {
        match self {
            Geo::Line(a, b) => Nurbs::new(1, vec![*a, *b], vec![0.0, 0.0, 1.0, 1.0]),
            Geo::Arc(c, r, a0, a1) => conic_arcs(*a0, *a1, |t| [c[0] + r * t.cos(), c[1] + r * t.sin()], |t| [-r * t.sin(), r * t.cos()]),
            Geo::Ellipse(c, u, a, b, t0, t1) => {
                let g = self.clone();
                let _ = (c, u, a, b);
                conic_arcs(*t0, *t1, |t| g.point(t), |t| g.deriv(t))
            }
            Geo::Hyperbola(c, u, a, b, t0, t1) => {
                // One rational quadratic: the middle pole where the end tangents meet, weight cosh(Δ/2).
                let (tm, half) = ((t0 + t1) / 2.0, (t1 - t0) / 2.0);
                let m = add_scaled(add_scaled(*c, *u, a * tm.cosh() / half.cosh()), perp(*u), b * tm.sinh() / half.cosh());
                Nurbs { degree: 2, poles: vec![self.point(*t0), m, self.point(*t1)], weights: vec![1.0, half.cosh(), 1.0], knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0] }
            }
            Geo::Parabola(v, u, f, s0, s1) => {
                let m = add_scaled(add_scaled(*v, *u, s0 * s1 / (4.0 * f)), perp(*u), (s0 + s1) / 2.0);
                Nurbs::new(2, vec![self.point(*s0), m, self.point(*s1)], vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            }
            Geo::Nurbs(n) => n.clone(),
        }
    }

    /// A polyline within about `tolerance` of the curve.
    pub fn polyline(&self, tolerance: f64) -> Vec<P> {
        match self {
            Geo::Line(a, b) => vec![*a, *b],
            Geo::Nurbs(n) => n.polyline(tolerance),
            _ => self.to_nurbs().polyline(tolerance),
        }
    }
}

/// An elliptic (or circular) arc as rational quadratics, each at most a quarter turn.
fn conic_arcs(t0: f64, t1: f64, at: impl Fn(f64) -> P, d: impl Fn(f64) -> P) -> Nurbs {
    let pieces = (((t1 - t0).abs() / FRAC_PI_2) - 1e-9).ceil().max(1.0) as usize;
    let step = (t1 - t0) / pieces as f64;
    let w = (step / 2.0).cos();
    let mut poles = vec![at(t0)];
    let mut weights = vec![1.0];
    let mut knots = vec![0.0; 3];
    for i in 0..pieces {
        let (a, b) = (t0 + step * i as f64, t0 + step * (i + 1) as f64);
        // Where the tangents at the ends meet.
        let (pa, pb, da, db) = (at(a), at(b), d(a), d(b));
        let den = da[0] * db[1] - da[1] * db[0];
        let s = ((pb[0] - pa[0]) * db[1] - (pb[1] - pa[1]) * db[0]) / den;
        poles.push(add_scaled(pa, da, s));
        weights.push(w);
        poles.push(pb);
        weights.push(1.0);
        let k = (i + 1) as f64;
        if i + 1 < pieces {
            knots.extend([k, k]);
        }
    }
    knots.extend([pieces as f64; 3]);
    Nurbs { degree: 2, poles, weights, knots }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P) -> bool {
        len(sub(a, b)) < 1e-9
    }

    #[test]
    fn conics_are_exact_as_nurbs() {
        let e = Geo::Ellipse([3.0, -2.0], [0.6, 0.8], 50.0, 20.0, 0.3, 4.0);
        let n = e.to_nurbs();
        for i in 0..=40 {
            let u = n.domain().0 + (n.domain().1 - n.domain().0) * i as f64 / 40.0;
            let q = n.point(u);
            let t = e.closest(q, true);
            assert!(len(sub(e.point(t), q)) < 1e-9, "on the ellipse: {q:?}");
        }
        assert!(close(n.point(n.domain().0), e.point(0.3)) && close(n.point(n.domain().1), e.point(4.0)));
        let h = Geo::Hyperbola([0.0, 0.0], [1.0, 0.0], 10.0, 5.0, -1.2, 0.9);
        let n = h.to_nurbs();
        for i in 0..=20 {
            let q = n.point(i as f64 / 20.0);
            assert!(((q[0] / 10.0).powi(2) - (q[1] / 5.0).powi(2) - 1.0).abs() < 1e-9, "{q:?}");
        }
        let par = Geo::Parabola([1.0, 1.0], [0.0, 1.0], 2.0, -3.0, 5.0);
        let n = par.to_nurbs();
        for i in 0..=20 {
            let q = n.point(i as f64 / 20.0);
            // Opens upward: y − 1 = (x − 1)² / 8.
            assert!((q[1] - 1.0 - (q[0] - 1.0).powi(2) / 8.0).abs() < 1e-9, "{q:?}");
        }
    }

    #[test]
    fn knots_insert_split_and_elevate_without_changing_the_curve() {
        let c = Nurbs::new(3, vec![[0.0, 0.0], [10.0, 20.0], [30.0, 25.0], [40.0, 0.0], [60.0, -10.0]], vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0]);
        let probe = |n: &Nurbs| (0..=30).map(|i| n.point(i as f64 / 30.0)).collect::<Vec<_>>();
        let base = probe(&c);
        for (a, b) in base.iter().zip(probe(&c.insert(0.3))) {
            assert!(close(*a, b));
        }
        let up = c.elevated();
        assert_eq!(up.degree, 4);
        for (a, b) in base.iter().zip(probe(&up)) {
            assert!(len(sub(*a, b)) < 1e-7, "{a:?} {b:?}");
        }
        let (l, r) = c.split(0.4);
        assert!(close(*l.poles.last().unwrap(), c.point(0.4)) && close(r.poles[0], c.point(0.4)));
        assert!(close(l.point(0.2), c.point(0.2)) && close(r.point(0.7), c.point(0.7)));
        let back = c.reversed();
        assert!(close(back.point(0.25), c.point(0.75)));
    }

    #[test]
    fn interpolation_passes_through_its_points() {
        let pts = [[0.0, 0.0], [10.0, 8.0], [25.0, 3.0], [40.0, 12.0], [50.0, 0.0]];
        let (p, poles, knots) = interpolate(&pts, false).unwrap();
        let n = Nurbs::new(p, poles, knots);
        for q in pts {
            let t = Geo::Nurbs(n.clone()).closest(q, false);
            assert!(len(sub(n.point(t), q)) < 1e-6, "{q:?}");
        }
        let (p, poles, _) = interpolate(&pts[..4], true).unwrap();
        let w = vec![1.0; poles.len()];
        let n = Nurbs::periodic(p, &poles, &w);
        assert!(close(n.poles[0], *n.poles.last().unwrap()), "closed");
        for q in &pts[..4] {
            let t = Geo::Nurbs(n.clone()).closest(*q, false);
            assert!(len(sub(n.point(t), *q)) < 1e-6, "{q:?}");
        }
    }
}
