//! Reading design intent back out of scan data: planes, spheres,
//! cylinders, cones and tori fitted to points by least squares
//! (Levenberg–Marquardt on true geometric distances, started from closed
//! forms and the points' normals), and found among clutter and other
//! surfaces by RANSAC (Fischler and Bolles 1981; minimal samples with
//! normals after Schnabel, Wahl and Klein 2007). The fitted numbers are
//! what an exact part is then built from.

use crate::V3;
use crate::linalg::*;
use crate::points::{Cloud, Orient};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Plane,
    Sphere,
    Cylinder,
    Cone,
    Torus,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Plane, Kind::Sphere, Kind::Cylinder, Kind::Cone, Kind::Torus];

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s.to_ascii_lowercase().as_str() {
            "plane" => Kind::Plane,
            "sphere" => Kind::Sphere,
            "cylinder" => Kind::Cylinder,
            "cone" => Kind::Cone,
            "torus" => Kind::Torus,
            _ => return None,
        })
    }
}

/// A fitted surface. Axes are unit vectors; `start`/`length` give the
/// extent of the fitted points along a cylinder's or cone's axis.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Primitive {
    Plane { point: V3, normal: V3 },
    Sphere { center: V3, radius: f64 },
    /// `base` is on the axis at the fitted points' lowest extent.
    Cylinder { base: V3, axis: V3, radius: f64, length: f64 },
    /// Radius `radius` at `base`, growing at `half_angle` (radians) along the axis for `length`; `apex` where it meets the axis.
    Cone { apex: V3, axis: V3, half_angle: f64, base: V3, radius: f64, length: f64 },
    Torus { center: V3, axis: V3, major: f64, minor: f64 },
}

impl Primitive {
    pub fn kind(&self) -> Kind {
        match self {
            Primitive::Plane { .. } => Kind::Plane,
            Primitive::Sphere { .. } => Kind::Sphere,
            Primitive::Cylinder { .. } => Kind::Cylinder,
            Primitive::Cone { .. } => Kind::Cone,
            Primitive::Torus { .. } => Kind::Torus,
        }
    }

    /// Signed distance of a point from the surface (positive outside a
    /// sphere, cylinder, cone or torus; above a plane).
    pub fn distance(&self, p: V3) -> f64 {
        match *self {
            Primitive::Plane { point, normal } => dot(sub(p, point), normal),
            Primitive::Sphere { center, radius } => dist(p, center) - radius,
            Primitive::Cylinder { base, axis, radius, .. } => {
                let v = sub(p, base);
                norm(sub(v, scale(axis, dot(v, axis)))) - radius
            }
            Primitive::Cone { apex, axis, half_angle, .. } => {
                let v = sub(p, apex);
                let along = dot(v, axis);
                let radial = norm(sub(v, scale(axis, along)));
                radial * half_angle.cos() - along * half_angle.sin()
            }
            Primitive::Torus { center, axis, major, minor } => {
                let v = sub(p, center);
                let h = dot(v, axis);
                let rho = norm(sub(v, scale(axis, h)));
                ((rho - major).powi(2) + h * h).sqrt() - minor
            }
        }
    }

    /// The surface's normal at (near) a point.
    pub fn normal(&self, p: V3) -> V3 {
        let e = 1e-5;
        let d = self.distance(p);
        unit([self.distance(add(p, [e, 0.0, 0.0])) - d, self.distance(add(p, [0.0, e, 0.0])) - d, self.distance(add(p, [0.0, 0.0, e])) - d])
    }
}

/// A fit and how good it is.
#[derive(Clone, Debug, Serialize)]
pub struct Fit {
    pub primitive: Primitive,
    /// RMS and largest |distance| of the points used (mm).
    pub rms: f64,
    pub max: f64,
    /// Points used (inliers, for RANSAC).
    pub points: usize,
    /// Of how many.
    pub of: usize,
    #[serde(skip)]
    pub inliers: Vec<u32>,
}

fn residuals(prim: &Primitive, pts: &[V3]) -> (f64, f64) {
    let d: Vec<f64> = pts.iter().map(|p| prim.distance(*p)).collect();
    let rms = (d.iter().map(|v| v * v).sum::<f64>() / d.len().max(1) as f64).sqrt();
    (rms, d.iter().fold(0.0, |m, v| m.max(v.abs())))
}

/// A unit vector from two angles, and back.
fn dir(theta: f64, phi: f64) -> V3 {
    [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()]
}
fn angles(a: V3) -> (f64, f64) {
    (a[2].clamp(-1.0, 1.0).acos(), a[1].atan2(a[0]))
}

/// Normals for points that have none (unoriented; fits use them up to sign).
fn normals_for(c: &Cloud) -> Vec<V3> {
    if c.normals.len() == c.len() {
        return c.normals.clone();
    }
    let mut c2 = c.clone();
    crate::points::estimate_normals(&mut c2, 12, Orient::None);
    c2.normals
}

pub fn fit_plane(pts: &[V3]) -> Primitive {
    let (c, cov) = covariance(pts);
    let (_, v) = sym_eigen3(cov);
    Primitive::Plane { point: c, normal: v[0] }
}

/// The circle through points in a plane (algebraic, Kåsa): centre and radius.
fn circle_2d(p: &[[f64; 2]]) -> Option<([f64; 2], f64)> {
    let mut a = [0.0; 9];
    let mut b = [0.0; 3];
    for q in p {
        let row = [q[0], q[1], 1.0];
        let rhs = -(q[0] * q[0] + q[1] * q[1]);
        for i in 0..3 {
            b[i] += row[i] * rhs;
            for j in 0..3 {
                a[i * 3 + j] += row[i] * row[j];
            }
        }
    }
    let x = solve_dense(&a, &b, 3)?;
    let c = [-x[0] / 2.0, -x[1] / 2.0];
    let r2 = c[0] * c[0] + c[1] * c[1] - x[2];
    (r2 > 0.0).then(|| (c, r2.sqrt()))
}

pub fn fit_sphere(pts: &[V3]) -> Option<Primitive> {
    // Algebraic start: x² + y² + z² + Dx + Ey + Fz + G = 0.
    let mut a = [0.0; 16];
    let mut b = [0.0; 4];
    for p in pts {
        let row = [p[0], p[1], p[2], 1.0];
        let rhs = -dot(*p, *p);
        for i in 0..4 {
            b[i] += row[i] * rhs;
            for j in 0..4 {
                a[i * 4 + j] += row[i] * row[j];
            }
        }
    }
    let x = solve_dense(&a, &b, 4)?;
    let c = [-x[0] / 2.0, -x[1] / 2.0, -x[2] / 2.0];
    let r = (dot(c, c) - x[3]).max(1e-12).sqrt();
    let (p, _) = levenberg_marquardt(vec![c[0], c[1], c[2], r], 100, |q, out| {
        out.clear();
        out.extend(pts.iter().map(|p| dist(*p, [q[0], q[1], q[2]]) - q[3]));
    });
    Some(Primitive::Sphere { center: [p[0], p[1], p[2]], radius: p[3].abs() })
}

/// Extent of points along an axis through `origin`: (lowest, highest).
fn extent(pts: &[V3], origin: V3, axis: V3) -> (f64, f64) {
    pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
        let t = dot(sub(*p, origin), axis);
        (lo.min(t), hi.max(t))
    })
}

/// The circle the points make seen along an axis (stray points trimmed, three rounds).
fn cylinder_from(axis: V3, pts: &[V3]) -> Option<(V3, f64)> {
    let (u, v) = frame(axis);
    let mut flat: Vec<[f64; 2]> = pts.iter().map(|p| [dot(*p, u), dot(*p, v)]).collect();
    let (mut c, mut r) = circle_2d(&flat)?;
    for _ in 0..3 {
        let d: Vec<f64> = flat.iter().map(|q| ((q[0] - c[0]).hypot(q[1] - c[1]) - r).abs()).collect();
        let mut s = d.clone();
        s.sort_by(|a, b| a.total_cmp(b));
        let limit = (3.0 * 1.4826 * s[s.len() / 2]).max(1e-12);
        let keep: Vec<[f64; 2]> = flat.iter().zip(&d).filter(|(_, e)| **e <= limit).map(|(q, _)| *q).collect();
        if keep.len() == flat.len() || keep.len() < 3 {
            break;
        }
        flat = keep;
        (c, r) = circle_2d(&flat)?;
    }
    Some((add(scale(u, c[0]), scale(v, c[1])), r))
}

pub fn fit_cylinder(pts: &[V3], normals: &[V3]) -> Option<Primitive> {
    // The axis is square to every normal: the normals' least-spread direction.
    let (_, cov) = covariance_about_zero(normals);
    let (_, ev) = sym_eigen3(cov);
    let mut best: Option<(f64, Vec<f64>)> = None;
    // Start from the normals' axis, and (noisy normals) from the points' long axis.
    let (_, pc) = covariance(pts);
    let (_, pv) = sym_eigen3(pc);
    for axis0 in [ev[0], pv[2]] {
        let Some((c0, r0)) = cylinder_from(axis0, pts) else { continue };
        let (t0, p0) = angles(axis0);
        let (u, v) = frame(axis0);
        // Only points near the starting cylinder steer the refinement.
        let near: Vec<V3> = {
            let d: Vec<f64> = pts.iter().map(|p| {
                let w = sub(*p, c0);
                (norm(sub(w, scale(axis0, dot(w, axis0)))) - r0).abs()
            }).collect();
            let mut s = d.clone();
            s.sort_by(|a, b| a.total_cmp(b));
            let limit = (3.0 * 1.4826 * s[s.len() / 2]).max(1e-9);
            let k: Vec<V3> = pts.iter().zip(&d).filter(|(_, e)| **e <= limit).map(|(p, _)| *p).collect();
            if k.len() >= 6 { k } else { pts.to_vec() }
        };
        let pts = &near;
        let start = vec![t0, p0, dot(c0, u), dot(c0, v), r0];
        let (p, rms) = levenberg_marquardt(start, 200, |q, out| {
            let a = dir(q[0], q[1]);
            // The axis point moves in the starting frame's plane.
            let o = add(scale(u, q[2]), scale(v, q[3]));
            out.clear();
            out.extend(pts.iter().map(|p| {
                let w = sub(*p, o);
                norm(sub(w, scale(a, dot(w, a)))) - q[4]
            }));
        });
        if best.as_ref().is_none_or(|b| rms < b.0) {
            best = Some((rms, p.into_iter().chain([u, v].into_iter().flatten()).collect()));
        }
    }
    let (_, q) = best?;
    let a = dir(q[0], q[1]);
    let (u, v) = ([q[5], q[6], q[7]], [q[8], q[9], q[10]]);
    let o = add(scale(u, q[2]), scale(v, q[3]));
    let (lo, hi) = extent(pts, o, a);
    Some(Primitive::Cylinder { base: add(o, scale(a, lo)), axis: a, radius: q[4].abs(), length: hi - lo })
}

fn covariance_about_zero(v: &[V3]) -> (V3, [[f64; 3]; 3]) {
    let mut m = [[0.0; 3]; 3];
    for n in v {
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += n[i] * n[j];
            }
        }
    }
    ([0.0; 3], m)
}

fn cone_primitive(apex: V3, axis: V3, half: f64, pts: &[V3]) -> Primitive {
    let (lo, hi) = extent(pts, apex, axis);
    let lo = lo.max(0.0);
    Primitive::Cone { apex, axis, half_angle: half, base: add(apex, scale(axis, lo)), radius: lo * half.tan(), length: hi - lo }
}

pub fn fit_cone(pts: &[V3], normals: &[V3]) -> Option<Primitive> {
    // Normals make one angle with the axis: their tips lie on a circle whose
    // plane is square to the axis. Normals' signs are arbitrary, so align them first.
    let (_, cov) = covariance_about_zero(normals);
    let (_, ev) = sym_eigen3(cov);
    let mut best: Option<(f64, Primitive)> = None;
    for guess in [ev[0], ev[1], ev[2]] {
        let ns: Vec<V3> = normals.iter().map(|n| if dot(*n, guess) < 0.0 { scale(*n, -1.0) } else { *n }).collect();
        let (_, c2) = covariance(&ns);
        let (_, e2) = sym_eigen3(c2);
        let mut axis = e2[0];
        // Apex: on every tangent plane (least squares).
        let mut a = [0.0; 9];
        let mut b = [0.0; 3];
        for (p, n) in pts.iter().zip(&ns) {
            let d = dot(*n, *p);
            for i in 0..3 {
                b[i] += n[i] * d;
                for j in 0..3 {
                    a[i * 3 + j] += n[i] * n[j];
                }
            }
        }
        let Some(x) = solve_dense(&a, &b, 3) else { continue };
        let apex = [x[0], x[1], x[2]];
        // Point the axis from the apex into the points.
        let centre = scale(pts.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / pts.len() as f64);
        if dot(sub(centre, apex), axis) < 0.0 {
            axis = scale(axis, -1.0);
        }
        let half0 = {
            let s: f64 = pts.iter().map(|p| {
                let v = sub(*p, apex);
                let along = dot(v, axis);
                norm(sub(v, scale(axis, along))).atan2(along)
            }).sum();
            (s / pts.len() as f64).clamp(0.01, 1.55)
        };
        let (t0, p0) = angles(axis);
        let (p, rms) = levenberg_marquardt(vec![apex[0], apex[1], apex[2], t0, p0, half0], 200, |q, out| {
            let prim = Primitive::Cone { apex: [q[0], q[1], q[2]], axis: dir(q[3], q[4]), half_angle: q[5], base: [0.0; 3], radius: 0.0, length: 0.0 };
            out.clear();
            out.extend(pts.iter().map(|p| prim.distance(*p)));
        });
        if !(p[5] > 0.0 && p[5] < 1.56) {
            continue;
        }
        let prim = cone_primitive([p[0], p[1], p[2]], dir(p[3], p[4]), p[5], pts);
        if best.as_ref().is_none_or(|b| rms < b.0) {
            best = Some((rms, prim));
        }
    }
    best.map(|b| b.1)
}

pub fn fit_torus(pts: &[V3], normals: &[V3]) -> Option<Primitive> {
    // Start 1: the points' own plane (a whole ring lies flat about its
    // axis), the circle through them in it, and the tube from their spread.
    let core_circle = |core: &[V3]| -> Option<(V3, V3, f64)> {
        let (c, cov) = covariance(core);
        let (_, ev) = sym_eigen3(cov);
        let axis = ev[0];
        let (u, v) = frame(axis);
        let flat: Vec<[f64; 2]> = core.iter().map(|p| [dot(sub(*p, c), u), dot(sub(*p, c), v)]).collect();
        let (cc, big) = circle_2d(&flat)?;
        Some((add(c, add(scale(u, cc[0]), scale(v, cc[1]))), axis, big))
    };
    let tube = |center: V3, axis: V3, big: f64| -> f64 {
        let s: f64 = pts.iter().map(|p| {
            let w = sub(*p, center);
            let h = dot(w, axis);
            ((norm(sub(w, scale(axis, h))) - big).powi(2) + h * h).sqrt()
        }).sum();
        s / pts.len() as f64
    };
    let mut starts = Vec::new();
    if let Some((c, a, big)) = core_circle(pts) {
        let small = tube(c, a, big);
        starts.push((c, a, big, small));
        // Start 2 (a partial ring, a fillet): each point stepped back along its
        // normal, on whichever side lands nearer the first core circle.
        let core: Vec<V3> = pts.iter().zip(normals).map(|(p, n)| {
            let (q1, q2) = (sub(*p, scale(*n, small)), add(*p, scale(*n, small)));
            let off = |q: V3| {
                let w = sub(q, c);
                let h = dot(w, a);
                (norm(sub(w, scale(a, h))) - big).powi(2) + h * h
            };
            if off(q1) <= off(q2) { q1 } else { q2 }
        }).collect();
        if let Some((c2, a2, big2)) = core_circle(&core) {
            starts.push((c2, a2, big2, tube(c2, a2, big2)));
        }
    }
    let mut best: Option<(f64, Primitive)> = None;
    for (c, a, big, small) in starts {
        let (t0, p0) = angles(a);
        let (p, rms) = levenberg_marquardt(vec![c[0], c[1], c[2], t0, p0, big, small], 200, |q, out| {
            let prim = Primitive::Torus { center: [q[0], q[1], q[2]], axis: dir(q[3], q[4]), major: q[5], minor: q[6] };
            out.clear();
            out.extend(pts.iter().map(|p| prim.distance(*p)));
        });
        let prim = Primitive::Torus { center: [p[0], p[1], p[2]], axis: dir(p[3], p[4]), major: p[5].abs(), minor: p[6].abs() };
        if best.as_ref().is_none_or(|b| rms < b.0) {
            best = Some((rms, prim));
        }
    }
    best.map(|b| b.1)
}

/// Least-squares fit of one kind to all the points.
///
/// Robust to a few points from neighbouring surfaces: after each fit, points
/// further than three robust standard deviations (1.4826 × the median
/// residual) are set aside and the fit is made again, up to three times.
pub fn fit(cloud: &Cloud, kind: Kind) -> Option<Fit> {
    let pts = &cloud.positions;
    if pts.len() < 6 {
        return None;
    }
    let normals = if kind == Kind::Plane || kind == Kind::Sphere { Vec::new() } else { normals_for(cloud) };
    let all: Vec<u32> = (0..pts.len() as u32).collect();
    let mut used = all.clone();
    let mut best = fit_once(pts, &normals, kind, &used)?;
    for _ in 0..3 {
        let mut r: Vec<f64> = pts.iter().map(|p| best.primitive.distance(*p).abs()).collect();
        let mut sorted = r.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = sorted[sorted.len() / 2];
        let limit = (3.0 * 1.4826 * median).max(1e-9);
        let keep: Vec<u32> = all.iter().copied().filter(|&i| r[i as usize] <= limit).collect();
        r.clear();
        if keep.len() == used.len() || keep.len() < 6 || keep.len() * 2 < pts.len() {
            break;
        }
        used = keep;
        match fit_once(pts, &normals, kind, &used) {
            Some(f) => best = f,
            None => break,
        }
    }
    best.of = pts.len();
    Some(best)
}

/// One least-squares fit to the points at `used`.
fn fit_once(all_pts: &[V3], all_normals: &[V3], kind: Kind, used: &[u32]) -> Option<Fit> {
    let pts: Vec<V3> = used.iter().map(|&i| all_pts[i as usize]).collect();
    let normals: Vec<V3> = if all_normals.is_empty() { Vec::new() } else { used.iter().map(|&i| all_normals[i as usize]).collect() };
    let pts = &pts;
    let prim = match kind {
        Kind::Plane => Some(fit_plane(pts)),
        Kind::Sphere => fit_sphere(pts),
        Kind::Cylinder => fit_cylinder(pts, &normals),
        Kind::Cone => fit_cone(pts, &normals),
        Kind::Torus => fit_torus(pts, &normals),
    }?;
    let (rms, max) = residuals(&prim, pts);
    Some(Fit { primitive: prim, rms, max, points: pts.len(), of: pts.len(), inliers: used.to_vec() })
}

/// Every kind fitted; the best first, preferring the simpler surface when
/// it fits nearly as well (a huge cylinder is a plane).
pub fn fit_best(cloud: &Cloud) -> Vec<Fit> {
    let mut fits: Vec<Fit> = Kind::ALL.par_iter().filter_map(|k| fit(cloud, *k)).collect();
    let size = {
        let (lo, hi) = cloud.bounds();
        dist(lo, hi).max(1e-9)
    };
    let rank = |f: &Fit| -> f64 {
        let order = Kind::ALL.iter().position(|k| *k == f.primitive.kind()).unwrap() as f64;
        // Each step up in complexity must earn a 30% better fit.
        (f.rms + size * 1e-9) * 1.3f64.powf(order)
    };
    fits.sort_by(|a, b| rank(a).total_cmp(&rank(b)));
    fits
}

/// RANSAC options.
#[derive(Clone, Copy, Debug)]
pub struct Ransac {
    /// Points within this distance (mm) count as on the surface.
    pub threshold: f64,
    /// And their normal within this angle (degrees) of the surface's.
    pub angle: f64,
    pub iterations: usize,
}

impl Default for Ransac {
    fn default() -> Self {
        Ransac { threshold: 0.1, angle: 25.0, iterations: 600 }
    }
}

/// A surface from a minimal sample of points with normals.
fn from_sample(kind: Kind, p: &[V3], n: &[V3]) -> Option<Primitive> {
    match kind {
        Kind::Plane => {
            let nrm = unit(cross(sub(p[1], p[0]), sub(p[2], p[0])));
            (norm(nrm) > 0.5).then_some(Primitive::Plane { point: p[0], normal: nrm })
        }
        Kind::Sphere => {
            // The closest approach of the two normal lines is the centre.
            let (c, _) = line_closest(p[0], n[0], p[1], n[1])?;
            let r = dist(c, p[0]);
            ((dist(c, p[1]) - r).abs() < 0.1 * r).then_some(Primitive::Sphere { center: c, radius: r })
        }
        Kind::Cylinder => {
            let axis = unit(cross(n[0], n[1]));
            if norm(axis) < 0.5 {
                return None;
            }
            // Project onto the plane square to the axis: the normal lines meet at the centre.
            let flat = |v: V3| sub(v, scale(axis, dot(v, axis)));
            let (c, _) = line_closest(flat(p[0]), unit(flat(n[0])), flat(p[1]), unit(flat(n[1])))?;
            let r = dist(c, flat(p[0]));
            Some(Primitive::Cylinder { base: c, axis, radius: r, length: 0.0 })
        }
        Kind::Cone => {
            // Apex on the three tangent planes; axis square to the plane of the normals' tips.
            let a: Vec<f64> = n[..3].iter().flatten().copied().collect();
            let b = [dot(n[0], p[0]), dot(n[1], p[1]), dot(n[2], p[2])];
            let x = solve_dense(&a, &b, 3)?;
            let apex = [x[0], x[1], x[2]];
            let mut axis = unit(cross(sub(n[1], n[0]), sub(n[2], n[0])));
            if norm(axis) < 0.5 {
                return None;
            }
            let c = scale(add(add(p[0], p[1]), p[2]), 1.0 / 3.0);
            if dot(sub(c, apex), axis) < 0.0 {
                axis = scale(axis, -1.0);
            }
            let v = sub(p[0], apex);
            let along = dot(v, axis);
            let half = norm(sub(v, scale(axis, along))).atan2(along);
            (half > 0.02 && half < 1.5).then_some(Primitive::Cone { apex, axis, half_angle: half, base: apex, radius: 0.0, length: 0.0 })
        }
        Kind::Torus => None,
    }
}

/// The closest points of two lines (point, direction): their midpoint and gap.
fn line_closest(p1: V3, d1: V3, p2: V3, d2: V3) -> Option<(V3, f64)> {
    let r = sub(p1, p2);
    let (a, b, c, d, e) = (dot(d1, d1), dot(d1, d2), dot(d2, d2), dot(d1, r), dot(d2, r));
    let den = a * c - b * b;
    if den.abs() < 1e-12 {
        return None;
    }
    let s = (b * e - c * d) / den;
    let t = (a * e - b * d) / den;
    let q1 = add(p1, scale(d1, s));
    let q2 = add(p2, scale(d2, t));
    Some((mid(q1, q2), dist(q1, q2)))
}

fn inliers(prim: &Primitive, pts: &[V3], normals: &[V3], o: &Ransac, active: &[u32]) -> Vec<u32> {
    let cos = o.angle.to_radians().cos();
    active
        .par_iter()
        .copied()
        .filter(|&i| {
            let p = pts[i as usize];
            prim.distance(p).abs() <= o.threshold && dot(prim.normal(p), normals[i as usize]).abs() >= cos
        })
        .collect()
}

/// A tiny deterministic random sequence (xorshift), so a scan fits the same every time.
struct Rng(u64);
impl Rng {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// The best surface of one kind among `active` points (indices), by RANSAC
/// then refined by least squares on its inliers.
pub fn ransac(cloud: &Cloud, normals: &[V3], kind: Kind, o: &Ransac, active: &[u32]) -> Option<Fit> {
    let pts = &cloud.positions;
    if active.len() < 10 {
        return None;
    }
    let need = match kind {
        Kind::Plane | Kind::Cone => 3,
        Kind::Sphere | Kind::Cylinder => 2,
        Kind::Torus => {
            // No minimal sample: least squares over the points, then inliers.
            let sub_cloud = Cloud { positions: active.iter().map(|&i| pts[i as usize]).collect(), normals: active.iter().map(|&i| normals[i as usize]).collect(), colors: vec![] };
            let f = fit(&sub_cloud, Kind::Torus)?;
            let ins = inliers(&f.primitive, pts, normals, o, active);
            return finish(f.primitive, pts, normals, o, ins, active);
        }
    };
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (active.len() as u64) ^ ((kind as u64) << 32));
    // Score samples on a subset for speed, then recount.
    let probe: Vec<u32> = if active.len() > 4000 { (0..4000).map(|i| active[i * active.len() / 4000]).collect() } else { active.to_vec() };
    let mut best: Option<(usize, Primitive)> = None;
    for _ in 0..o.iterations {
        let idx: Vec<usize> = (0..need).map(|_| active[rng.next(active.len())] as usize).collect();
        let ps: Vec<V3> = idx.iter().map(|&i| pts[i]).collect();
        let ns: Vec<V3> = idx.iter().map(|&i| normals[i]).collect();
        let Some(prim) = from_sample(kind, &ps, &ns) else { continue };
        let score = inliers(&prim, pts, normals, o, &probe).len();
        if best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, prim));
        }
    }
    let (_, prim) = best?;
    let ins = inliers(&prim, pts, normals, o, active);
    finish(prim, pts, normals, o, ins, active)
}

fn finish(prim: Primitive, pts: &[V3], normals: &[V3], o: &Ransac, ins: Vec<u32>, active: &[u32]) -> Option<Fit> {
    let of = active.len();
    if ins.len() < 6 {
        return None;
    }
    // Refine on the inliers, then take the inliers again (twice).
    let mut prim = prim;
    let mut ins = ins;
    for _ in 0..2 {
        let c = Cloud { positions: ins.iter().map(|&i| pts[i as usize]).collect(), normals: ins.iter().map(|&i| normals[i as usize]).collect(), colors: vec![] };
        if let Some(f) = fit(&c, prim.kind()) {
            let again = inliers(&f.primitive, pts, normals, o, &ins);
            if again.len() * 10 >= ins.len() * 9 {
                prim = f.primitive;
            }
        }
        let wider = inliers(&prim, pts, normals, o, active);
        if wider.len() >= ins.len() {
            ins = wider;
        }
    }
    let used: Vec<V3> = ins.iter().map(|&i| pts[i as usize]).collect();
    // Extents along axes from the inliers.
    prim = match prim {
        Primitive::Cylinder { base, axis, radius, .. } => {
            let o0 = sub(base, scale(axis, dot(base, axis)));
            let (lo, hi) = extent(&used, o0, axis);
            Primitive::Cylinder { base: add(o0, scale(axis, lo)), axis, radius, length: hi - lo }
        }
        Primitive::Cone { apex, axis, half_angle, .. } => cone_primitive(apex, axis, half_angle, &used),
        p => p,
    };
    let (rms, max) = residuals(&prim, &used);
    Some(Fit { primitive: prim, rms, max, points: ins.len(), of, inliers: ins })
}

/// Surfaces found one after another (the one covering most points first,
/// its points then set aside) until fewer than `min_points` remain or
/// `max_shapes` are found: a scan read as the primitives it is made of.
pub fn detect(cloud: &Cloud, kinds: &[Kind], o: &Ransac, min_points: usize, max_shapes: usize) -> Vec<Fit> {
    let normals = normals_for(cloud);
    let mut active: Vec<u32> = (0..cloud.len() as u32).collect();
    let mut out: Vec<Fit> = Vec::new();
    while active.len() >= min_points.max(10) && out.len() < max_shapes {
        let found: Vec<Fit> = kinds.par_iter().filter_map(|k| ransac(cloud, &normals, *k, o, &active)).collect();
        // Most points wins; simpler kinds win ties within 5%.
        let Some(best) = found.into_iter().reduce(|a, b| {
            let (ka, kb) = (Kind::ALL.iter().position(|k| *k == a.primitive.kind()), Kind::ALL.iter().position(|k| *k == b.primitive.kind()));
            if (b.points as f64) > a.points as f64 * 1.05 || ((b.points as f64) > a.points as f64 * 0.95 && kb < ka && b.points >= a.points) { b } else { a }
        }) else {
            break;
        };
        if best.points < min_points {
            break;
        }
        let taken: std::collections::HashSet<u32> = best.inliers.iter().copied().collect();
        active.retain(|i| !taken.contains(i));
        let mut best = best;
        best.inliers.retain(|i| taken.contains(i));
        out.push(best);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::points::sample_mesh;

    fn cloud_of(pts: Vec<V3>, normals: Vec<V3>) -> Cloud {
        Cloud { positions: pts, normals, colors: vec![] }
    }

    /// Small deterministic noise of amplitude `a`.
    fn jitter(p: &mut [V3], a: f64) {
        for (i, q) in p.iter_mut().enumerate() {
            let f = |k: usize| ((((i * 3 + k) as u64).wrapping_mul(2654435761) % 10000) as f64 / 5000.0 - 1.0) * a;
            *q = add(*q, [f(0), f(1), f(2)]);
        }
    }

    #[test]
    fn primitives_come_back_with_their_numbers() {
        // Cylinder r=7.5 along a tilted axis.
        let axis = unit([0.2, -0.3, 1.0]);
        let (u, v) = frame(axis);
        let base = [3.0, -2.0, 1.0];
        let mut pts = Vec::new();
        let mut ns = Vec::new();
        for i in 0..2000 {
            let a = i as f64 * 2.399963;
            let t = (i as f64 * 0.7548776662).fract() * 30.0;
            let rad = add(scale(u, a.cos()), scale(v, a.sin()));
            pts.push(add(add(base, scale(axis, t)), scale(rad, 7.5)));
            ns.push(rad);
        }
        jitter(&mut pts, 0.01);
        let f = fit(&cloud_of(pts.clone(), ns.clone()), Kind::Cylinder).unwrap();
        let Primitive::Cylinder { radius, axis: a, length, .. } = f.primitive else { panic!() };
        assert!((radius - 7.5).abs() < 0.005, "radius {radius}");
        assert!(dot(a, axis).abs() > 0.99999, "axis {a:?}");
        assert!((length - 30.0).abs() < 0.1, "length {length}");
        assert!(f.rms < 0.01, "rms {}", f.rms);

        // Sphere r=12 at (5, 6, 7).
        let mut sp: Vec<V3> = (0..1500).map(|i| {
            let z = 1.0 - 2.0 * (i as f64 + 0.5) / 1500.0;
            let a = i as f64 * 2.399963;
            let r = (1.0 - z * z).sqrt();
            add([5.0, 6.0, 7.0], scale([r * a.cos(), r * a.sin(), z], 12.0))
        }).collect();
        jitter(&mut sp, 0.01);
        let f = fit(&cloud_of(sp, vec![]), Kind::Sphere).unwrap();
        let Primitive::Sphere { center, radius } = f.primitive else { panic!() };
        assert!((radius - 12.0).abs() < 0.003 && dist(center, [5.0, 6.0, 7.0]) < 0.003, "{center:?} {radius}");

        // Cone: half-angle 20°, apex at the origin, axis +z.
        let half = 20f64.to_radians();
        let mut cp = Vec::new();
        let mut cn = Vec::new();
        for i in 0..2000 {
            let a = i as f64 * 2.399963;
            let h = 5.0 + (i as f64 * 0.7548776662).fract() * 20.0;
            let r = h * half.tan();
            cp.push([r * a.cos(), r * a.sin(), h]);
            cn.push(unit([a.cos() * half.cos(), a.sin() * half.cos(), -half.sin()]));
        }
        let f = fit(&cloud_of(cp, cn), Kind::Cone).unwrap();
        let Primitive::Cone { apex, half_angle, axis, .. } = f.primitive else { panic!() };
        assert!((half_angle - half).abs() < 1e-4, "{}", half_angle.to_degrees());
        assert!(norm(apex) < 1e-3 && axis[2] > 0.9999, "{apex:?} {axis:?}");

        // Torus R=20, r=5.
        let mut tp = Vec::new();
        let mut tn = Vec::new();
        for i in 0..3000 {
            let a = i as f64 * 2.399963;
            let b = (i as f64 * 0.7548776662).fract() * std::f64::consts::TAU;
            let n = [b.cos() * a.cos(), b.cos() * a.sin(), b.sin()];
            tp.push(add([20.0 * a.cos(), 20.0 * a.sin(), 0.0], scale(n, 5.0)));
            tn.push(n);
        }
        let f = fit(&cloud_of(tp, tn), Kind::Torus).unwrap();
        let Primitive::Torus { major, minor, axis, .. } = f.primitive else { panic!() };
        assert!((major - 20.0).abs() < 1e-3 && (minor - 5.0).abs() < 1e-3 && axis[2].abs() > 0.9999, "{major} {minor}");
    }

    #[test]
    fn the_best_kind_is_chosen_and_ransac_finds_shapes_among_others() {
        // A box with a cylinder boss: planes and a cylinder.
        let mut m = crate::mesh::cube(40.0, 40.0, 10.0);
        let mut boss = crate::mesh::cylinder(6.0, 15.0, 96, 6);
        boss.transform(&[[1.0, 0.0, 0.0, 20.0], [0.0, 1.0, 0.0, 20.0], [0.0, 0.0, 1.0, 10.0]]);
        m.merge(&boss);
        let c = sample_mesh(&m, 20000);
        let shapes = detect(&c, &[Kind::Plane, Kind::Cylinder, Kind::Sphere], &Ransac { threshold: 0.05, ..Default::default() }, 300, 12);
        let cyl = shapes.iter().find(|f| f.primitive.kind() == Kind::Cylinder).expect("the boss is found");
        let Primitive::Cylinder { radius, .. } = cyl.primitive else { panic!() };
        assert!((radius - 6.0).abs() < 0.02, "{radius}");
        let planes = shapes.iter().filter(|f| f.primitive.kind() == Kind::Plane).count();
        assert!(planes >= 6, "{planes} planes in {:?}", shapes.iter().map(|s| (s.primitive.kind(), s.points)).collect::<Vec<_>>());
        // Only the top face's points: a plane, not a giant sphere or cylinder.
        let top: Vec<u32> = (0..c.len() as u32).filter(|&i| (c.positions[i as usize][2] - 10.0).abs() < 1e-9 && dist(c.positions[i as usize], [20.0, 20.0, 10.0]) > 7.0).collect();
        let best = fit_best(&c.select(&top));
        assert_eq!(best[0].primitive.kind(), Kind::Plane);
    }
}
