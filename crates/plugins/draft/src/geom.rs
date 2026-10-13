//! Curve math for drafting: frames, circles through three points, B-splines
//! (through points or by control points, open or closed) and their Bézier
//! pieces, and hatching of closed outlines.

pub type V3 = [f64; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
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
pub fn unit(a: V3) -> Option<V3> {
    let n = norm(a);
    (n > 1e-12).then(|| mul(a, 1.0 / n))
}
pub fn dist(a: V3, b: V3) -> f64 {
    norm(sub(a, b))
}

/// A plane frame: origin, x and y in the plane (unit, square), normal = x × y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: V3,
    pub x: V3,
    pub y: V3,
}

impl Frame {
    pub fn normal(&self) -> V3 {
        cross(self.x, self.y)
    }
    /// A frame with this normal; `x` is projected into the plane (or a sensible one chosen).
    pub fn new(origin: V3, normal: V3, x: Option<V3>) -> Option<Frame> {
        let n = unit(normal)?;
        let guess = x.and_then(|x| unit(sub(x, mul(n, dot(x, n))))).unwrap_or_else(|| {
            // World X unless the normal runs along it, then world Y.
            let w = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            unit(sub(w, mul(n, dot(w, n)))).unwrap()
        });
        let y = cross(n, guess);
        Some(Frame { origin, x: guess, y })
    }
    pub fn at(&self, u: f64, v: f64) -> V3 {
        add(self.origin, add(mul(self.x, u), mul(self.y, v)))
    }
    pub fn local(&self, p: V3) -> (f64, f64) {
        let d = sub(p, self.origin);
        (dot(d, self.x), dot(d, self.y))
    }
    pub fn height(&self, p: V3) -> f64 {
        dot(sub(p, self.origin), self.normal())
    }
}

/// The plane through points (None when they are all on a line).
pub fn plane_of(points: &[V3]) -> Option<Frame> {
    let p0 = *points.first()?;
    let far = points.iter().copied().max_by(|a, b| dist(*a, p0).total_cmp(&dist(*b, p0)))?;
    let x = unit(sub(far, p0))?;
    let mut best: Option<(f64, V3)> = None;
    for p in points {
        let c = cross(x, sub(*p, p0));
        let n = norm(c);
        if best.is_none_or(|(b, _)| n > b) {
            best = Some((n, c));
        }
    }
    let (n, c) = best?;
    if n < 1e-9 * dist(far, p0).max(1.0) {
        return None;
    }
    Frame::new(p0, c, Some(x))
}

/// The circle through three points: centre, radius and the plane's normal
/// (a, b, c run counter-clockwise about it).
pub fn circle_through(a: V3, b: V3, c: V3) -> Option<(V3, f64, V3)> {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let n = cross(ab, ac);
    let nn = dot(n, n);
    if nn < 1e-18 {
        return None;
    }
    let center = add(a, mul(add(mul(cross(n, ab), dot(ac, ac)), mul(cross(ac, n), dot(ab, ab))), 0.5 / nn));
    Some((center, dist(center, a), unit(n)?))
}

/// Points on an arc of the circle (centre, radius) in a frame, from `a0` to `a1` degrees.
pub fn arc_points(f: &Frame, r: f64, a0: f64, a1: f64, segments: usize) -> Vec<V3> {
    (0..=segments)
        .map(|i| {
            let t = (a0 + (a1 - a0) * i as f64 / segments as f64).to_radians();
            f.at(r * t.cos(), r * t.sin())
        })
        .collect()
}

pub fn ellipse_points(f: &Frame, rx: f64, ry: f64, a0: f64, a1: f64, segments: usize) -> Vec<V3> {
    (0..=segments)
        .map(|i| {
            let t = (a0 + (a1 - a0) * i as f64 / segments as f64).to_radians();
            f.at(rx * t.cos(), ry * t.sin())
        })
        .collect()
}

/// Segments for a full turn of radius r so chords stay within `tol` mm.
pub fn segments_for(r: f64, tol: f64) -> usize {
    if r <= tol {
        return 8;
    }
    let step = 2.0 * (1.0 - tol / r).clamp(-1.0, 1.0).acos();
    ((std::f64::consts::TAU / step).ceil() as usize).clamp(12, 720)
}

// ---- B-splines ------------------------------------------------------------------------------

/// A B-spline: degree, control points and knots (len = n + degree + 1).
#[derive(Clone, Debug, PartialEq)]
pub struct BSpline {
    pub degree: usize,
    pub ctrl: Vec<V3>,
    pub knots: Vec<f64>,
}

impl BSpline {
    /// Clamped, uniform: passes through the first and last control points.
    pub fn clamped(ctrl: Vec<V3>, degree: usize) -> Option<BSpline> {
        if ctrl.len() < 2 {
            return None;
        }
        let p = degree.min(ctrl.len() - 1).max(1);
        let n = ctrl.len();
        let inner = n - p - 1;
        let mut knots = vec![0.0; p + 1];
        for i in 1..=inner {
            knots.push(i as f64 / (inner + 1) as f64);
        }
        knots.extend(std::iter::repeat_n(1.0, p + 1));
        Some(BSpline { degree: p, ctrl, knots })
    }

    /// Closed: the control polygon wraps around (periodic, smooth at the seam).
    pub fn periodic(ctrl: Vec<V3>, degree: usize) -> Option<BSpline> {
        if ctrl.len() < 3 {
            return None;
        }
        let p = degree.min(ctrl.len() - 1).max(1);
        let mut c = ctrl.clone();
        c.extend(ctrl.iter().take(p).copied());
        let m = c.len() + p + 1;
        let knots: Vec<f64> = (0..m).map(|i| i as f64).collect();
        Some(BSpline { degree: p, ctrl: c, knots })
    }

    /// The cubic (or lower) B-spline through these points: chord-length
    /// parameters, averaged knots, one linear solve.
    pub fn through(points: &[V3], degree: usize) -> Option<BSpline> {
        let n = points.len();
        if n < 2 {
            return None;
        }
        let p = degree.min(n - 1).max(1);
        let mut t = vec![0.0; n];
        let total: f64 = points.windows(2).map(|w| dist(w[0], w[1]).sqrt()).sum();
        if total <= 0.0 {
            return None;
        }
        for i in 1..n {
            t[i] = t[i - 1] + dist(points[i - 1], points[i]).sqrt() / total;
        }
        t[n - 1] = 1.0;
        let mut knots = vec![0.0; p + 1];
        for j in 1..n - p {
            knots.push(t[j..j + p].iter().sum::<f64>() / p as f64);
        }
        knots.extend(std::iter::repeat_n(1.0, p + 1));
        // Basis matrix N[i][j] = N_j(t_i); solve N · P = Q for each coordinate.
        let mut a = vec![vec![0.0; n]; n];
        for (i, ti) in t.iter().enumerate() {
            let span = find_span(n, p, *ti, &knots);
            let b = basis(span, *ti, p, &knots);
            for (k, v) in b.iter().enumerate() {
                a[i][span - p + k] = *v;
            }
        }
        let ctrl = solve(a, points)?;
        Some(BSpline { degree: p, ctrl, knots })
    }

    fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.ctrl.len()])
    }

    pub fn point(&self, u: f64) -> V3 {
        let p = self.degree;
        let n = self.ctrl.len();
        let span = find_span(n, p, u, &self.knots);
        let b = basis(span, u, p, &self.knots);
        let mut out = [0.0; 3];
        for (k, v) in b.iter().enumerate() {
            out = add(out, mul(self.ctrl[span - p + k], *v));
        }
        out
    }

    /// Points along the curve, `per_span` per knot span.
    pub fn sample(&self, per_span: usize) -> Vec<V3> {
        let (a, b) = self.domain();
        let spans = self.knots.windows(2).filter(|w| w[1] > w[0] && w[0] >= a && w[1] <= b).count().max(1);
        let n = spans * per_span;
        (0..=n).map(|i| self.point(a + (b - a) * i as f64 / n as f64)).collect()
    }

    /// The curve as cubic Bézier pieces [start, c1, c2, end] (raised to
    /// degree 3 if lower), by knot insertion: exact.
    pub fn beziers(&self) -> Vec<[V3; 4]> {
        let mut s = self.clone();
        let (a, b) = s.domain();
        // Trim a periodic curve to its domain by inserting its ends fully.
        let interior: Vec<f64> = {
            let mut k: Vec<f64> = s.knots.iter().copied().filter(|k| *k > a && *k < b).collect();
            k.dedup();
            k
        };
        for u in [a, b].into_iter().chain(interior.iter().copied()) {
            while s.knots.iter().filter(|k| (**k - u).abs() < 1e-12).count() < s.degree {
                s.insert(u);
            }
        }
        let p = s.degree;
        // Collect the Bézier segments between distinct knots in [a, b].
        let mut out = Vec::new();
        let mut i = 0;
        while i + p < s.ctrl.len() {
            let (k0, k1) = (s.knots[i + p], s.knots[i + p + 1]);
            if k1 > k0 && k0 >= a - 1e-12 && k1 <= b + 1e-12 {
                let seg: Vec<V3> = s.ctrl[i..=i + p].to_vec();
                out.push(raise_to_cubic(&seg));
            }
            i += 1;
        }
        out
    }

    /// Boehm's knot insertion.
    fn insert(&mut self, u: f64) {
        let p = self.degree;
        let n = self.ctrl.len();
        let k = find_span(n, p, u, &self.knots);
        let mut q = Vec::with_capacity(n + 1);
        for i in 0..=n {
            if i + p <= k {
                q.push(self.ctrl[i]);
            } else if i > k {
                q.push(self.ctrl[i - 1]);
            } else {
                let d = self.knots[i + p] - self.knots[i];
                let a = if d.abs() < 1e-15 { 0.0 } else { (u - self.knots[i]) / d };
                q.push(add(mul(self.ctrl[i - 1], 1.0 - a), mul(self.ctrl[i], a)));
            }
        }
        self.knots.insert(k + 1, u);
        self.ctrl = q;
    }
}

fn raise_to_cubic(seg: &[V3]) -> [V3; 4] {
    match seg.len() {
        2 => [seg[0], add(seg[0], mul(sub(seg[1], seg[0]), 1.0 / 3.0)), add(seg[0], mul(sub(seg[1], seg[0]), 2.0 / 3.0)), seg[1]],
        3 => [seg[0], add(seg[0], mul(sub(seg[1], seg[0]), 2.0 / 3.0)), add(seg[2], mul(sub(seg[1], seg[2]), 2.0 / 3.0)), seg[2]],
        _ => [seg[0], seg[1], seg[2], seg[3]],
    }
}

fn find_span(n: usize, p: usize, u: f64, knots: &[f64]) -> usize {
    if u >= knots[n] {
        // the last span with length
        let mut s = n - 1;
        while s > p && knots[s] >= knots[s + 1] {
            s -= 1;
        }
        return s;
    }
    if u <= knots[p] {
        return p;
    }
    let (mut lo, mut hi) = (p, n);
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

fn basis(span: usize, u: f64, p: usize, knots: &[f64]) -> Vec<f64> {
    let mut n = vec![0.0; p + 1];
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];
    n[0] = 1.0;
    for j in 1..=p {
        left[j] = u - knots[span + 1 - j];
        right[j] = knots[span + j] - u;
        let mut saved = 0.0;
        for r in 0..j {
            let d = right[r + 1] + left[j - r];
            let t = if d.abs() < 1e-15 { 0.0 } else { n[r] / d };
            n[r] = saved + right[r + 1] * t;
            saved = left[j - r] * t;
        }
        n[j] = saved;
    }
    n
}

/// Solves A·X = B (dense, partial pivoting) for the three columns of B.
fn solve(mut a: Vec<Vec<f64>>, b: &[V3]) -> Option<Vec<V3>> {
    let n = a.len();
    let mut x: Vec<V3> = b.to_vec();
    for c in 0..n {
        let piv = (c..n).max_by(|i, j| a[*i][c].abs().total_cmp(&a[*j][c].abs()))?;
        if a[piv][c].abs() < 1e-14 {
            return None;
        }
        a.swap(c, piv);
        x.swap(c, piv);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            if f == 0.0 {
                continue;
            }
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            x[r] = sub(x[r], mul(x[c], f));
        }
    }
    for c in (0..n).rev() {
        let mut s = x[c];
        for k in c + 1..n {
            s = sub(s, mul(x[k], a[c][k]));
        }
        x[c] = mul(s, 1.0 / a[c][c]);
    }
    Some(x)
}

// ---- Hatching -------------------------------------------------------------------------------

/// Hatch lines inside closed outlines (even-odd) in a frame: parallel lines
/// at `angle` degrees, `spacing` apart, as world segments.
pub fn hatch(f: &Frame, outlines: &[Vec<V3>], angle: f64, spacing: f64) -> Vec<[V3; 2]> {
    if spacing <= 0.0 {
        return Vec::new();
    }
    let (s, c) = angle.to_radians().sin_cos();
    // Work in coordinates turned so hatch lines run along u.
    let rot = |(u, v): (f64, f64)| (u * c + v * s, -u * s + v * c);
    let back = |(u, v): (f64, f64)| (u * c - v * s, u * s + v * c);
    let polys: Vec<Vec<(f64, f64)>> = outlines.iter().map(|o| o.iter().map(|p| rot(f.local(*p))).collect()).collect();
    let (vmin, vmax) = polys.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), (_, v)| (a.min(*v), b.max(*v)));
    if !vmin.is_finite() || (vmax - vmin) / spacing > 5000.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut v = (vmin / spacing).floor() * spacing + spacing * 0.5;
    while v < vmax {
        let mut xs = Vec::new();
        for poly in &polys {
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if (a.1 <= v) != (b.1 <= v) {
                    xs.push(a.0 + (v - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            let (p, q) = (back((pair[0], v)), back((pair[1], v)));
            out.push([f.at(p.0, p.1), f.at(q.0, q.1)]);
        }
        v += spacing;
    }
    out
}

/// Area inside a closed outline lying in a frame.
pub fn area(f: &Frame, outline: &[V3]) -> f64 {
    let pts: Vec<(f64, f64)> = outline.iter().map(|p| f.local(*p)).collect();
    let mut a = 0.0;
    for i in 0..pts.len() {
        let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
        a += p.0 * q.1 - q.0 * p.1;
    }
    (a / 2.0).abs()
}

pub fn length(points: &[V3]) -> f64 {
    points.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// The point and unit tangent at arc length `s` along a polyline.
pub fn along(points: &[V3], s: f64) -> Option<(V3, V3)> {
    let mut left = s.max(0.0);
    for w in points.windows(2) {
        let d = dist(w[0], w[1]);
        if d <= 1e-12 {
            continue;
        }
        if left <= d {
            return Some((add(w[0], mul(sub(w[1], w[0]), left / d)), mul(sub(w[1], w[0]), 1.0 / d)));
        }
        left -= d;
    }
    let n = points.len();
    let (a, b) = (points.get(n.wrapping_sub(2))?, points.get(n - 1)?);
    Some((*b, unit(sub(*b, *a))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        dist(a, b) < 1e-9
    }

    #[test]
    fn circles_and_frames() {
        let (c, r, n) = circle_through([10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [-10.0, 0.0, 0.0]).unwrap();
        assert!(close(c, [0.0; 3]) && (r - 10.0).abs() < 1e-12 && close(n, [0.0, 0.0, 1.0]));
        assert!(circle_through([0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]).is_none());
        let f = Frame::new([1.0, 2.0, 3.0], [0.0, 0.0, 2.0], None).unwrap();
        assert!(close(f.x, [1.0, 0.0, 0.0]) && close(f.normal(), [0.0, 0.0, 1.0]));
        let p = f.at(3.0, 4.0);
        assert_eq!(f.local(p), (3.0, 4.0));
        let pl = plane_of(&[[0.0, 0.0, 5.0], [10.0, 0.0, 5.0], [10.0, 10.0, 5.0]]).unwrap();
        assert!(pl.height([3.0, 7.0, 5.0]).abs() < 1e-12);
        assert!(plane_of(&[[0.0; 3], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]]).is_none());
    }

    #[test]
    fn bsplines() {
        let pts = vec![[0.0, 0.0, 0.0], [10.0, 5.0, 0.0], [20.0, -3.0, 0.0], [30.0, 8.0, 2.0], [40.0, 0.0, 0.0]];
        let s = BSpline::through(&pts, 3).unwrap();
        // It passes through every point it was given.
        for p in &pts {
            let sampled = s.sample(200);
            let d = sampled.iter().map(|q| dist(*q, *p)).fold(f64::INFINITY, f64::min);
            assert!(d < 0.05, "{p:?} missed by {d}");
        }
        assert!(close(s.point(0.0), pts[0]) && close(s.point(1.0), pts[4]));
        // Bézier pieces trace the same curve, joined end to end.
        let bz = s.beziers();
        assert_eq!(bz.len(), 2, "two interior knots… three spans? {}", bz.len());
        for w in bz.windows(2) {
            assert!(close(w[0][3], w[1][0]));
        }
        let bez = |b: &[V3; 4], t: f64| {
            let u = 1.0 - t;
            add(add(mul(b[0], u * u * u), mul(b[1], 3.0 * u * u * t)), add(mul(b[2], 3.0 * u * t * t), mul(b[3], t * t * t)))
        };
        assert!(close(bz[0][0], pts[0]) && close(bz.last().unwrap()[3], pts[4]));
        let mid = bez(&bz[0], 0.5);
        let d = s.sample(400).iter().map(|q| dist(*q, mid)).fold(f64::INFINITY, f64::min);
        assert!(d < 0.01, "Bézier off the spline by {d}");
        // Control-point splines: clamped ends; periodic ones close smoothly.
        let c = BSpline::clamped(vec![[0.0; 3], [5.0, 10.0, 0.0], [10.0, 0.0, 0.0]], 3).unwrap();
        assert_eq!(c.degree, 2);
        assert!(close(c.point(1.0), [10.0, 0.0, 0.0]));
        assert!(close(c.beziers()[0][0], [0.0; 3]));
        let sq = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0]];
        let p = BSpline::periodic(sq, 3).unwrap();
        let samp = p.sample(10);
        assert!(close(samp[0], *samp.last().unwrap()), "closed");
        let pieces = p.beziers();
        assert_eq!(pieces.len(), 4);
        assert!(close(pieces[0][0], pieces[3][3]));
    }

    #[test]
    fn hatching_and_paths() {
        let f = Frame::new([0.0; 3], [0.0, 0.0, 1.0], None).unwrap();
        let sq = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0]];
        let h = hatch(&f, &[sq.clone()], 0.0, 1.0);
        assert_eq!(h.len(), 10);
        assert!(h.iter().all(|s| (dist(s[0], s[1]) - 10.0).abs() < 1e-9));
        let hole = vec![[3.0, 3.0, 0.0], [7.0, 3.0, 0.0], [7.0, 7.0, 0.0], [3.0, 7.0, 0.0]];
        let total: f64 = hatch(&f, &[sq.clone(), hole], 0.0, 1.0).iter().map(|s| dist(s[0], s[1])).sum();
        assert!((total - (100.0 - 16.0)).abs() < 1e-9, "{total}");
        let diag: f64 = hatch(&f, &[sq.clone()], 45.0, 0.5).iter().map(|s| dist(s[0], s[1])).sum();
        assert!((diag * 0.5 - 100.0).abs() < 2.0, "{diag}");
        assert_eq!(area(&f, &sq), 100.0);
        let (p, t) = along(&sq, 15.0).unwrap();
        assert!(close(p, [10.0, 5.0, 0.0]) && close(t, [0.0, 1.0, 0.0]));
    }
}
