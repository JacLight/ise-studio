//! Photogrammetry's geometry after the reconstruction engine is done: the
//! cameras it solved (COLMAP's text model, Apple Object Capture's poses),
//! giving the model its true size and its floor, and saying how well the
//! photos covered the object.
//!
//! Photos have no scale, so a reconstruction comes out in arbitrary units
//! and orientation. Three ways give it millimetres: a known distance between
//! two points picked on it, the printed marker sheet (see [`crate::markers`])
//! seen in the photos and triangulated through the solved cameras (a
//! similarity fit of its corners to where they are printed gives scale, the
//! floor and the axes at once), or depth the photos carried (the engine's
//! own). The floor alone comes from a plane fit (RANSAC, then least squares).

use crate::V3;
use crate::linalg::{add, cross, dot, norm, scale, sub, sym_eigen3, unit};
use crate::markers::{Detection, Sheet};
use serde::Serialize;

/// A solved camera: the picture's size, its lens, and where it stood.
#[derive(Clone, Debug, Serialize)]
pub struct Camera {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    /// Radial distortion (k1, k2) and tangential (p1, p2), OpenCV's model.
    pub dist: [f64; 4],
    /// World to camera (x right, y down, z forward): x_cam = R·x + t.
    pub rotation: [[f64; 3]; 3],
    pub translation: V3,
}

impl Camera {
    /// Where the camera stood, world.
    pub fn centre(&self) -> V3 {
        let r = &self.rotation;
        let t = self.translation;
        // −Rᵀ t
        [-(r[0][0] * t[0] + r[1][0] * t[1] + r[2][0] * t[2]), -(r[0][1] * t[0] + r[1][1] * t[1] + r[2][1] * t[2]), -(r[0][2] * t[0] + r[1][2] * t[1] + r[2][2] * t[2])]
    }

    /// The direction it looks, world.
    pub fn forward(&self) -> V3 {
        self.rotation[2]
    }

    fn distort(&self, x: f64, y: f64) -> (f64, f64) {
        let [k1, k2, p1, p2] = self.dist;
        let r2 = x * x + y * y;
        let radial = 1.0 + k1 * r2 + k2 * r2 * r2;
        (x * radial + 2.0 * p1 * x * y + p2 * (r2 + 2.0 * x * x), y * radial + p1 * (r2 + 2.0 * y * y) + 2.0 * p2 * x * y)
    }

    /// A world point's pixel (None behind the camera).
    pub fn project(&self, p: V3) -> Option<[f64; 2]> {
        let r = &self.rotation;
        let c = add([dot(r[0], p), dot(r[1], p), dot(r[2], p)], self.translation);
        if c[2] <= 1e-12 {
            return None;
        }
        let (x, y) = self.distort(c[0] / c[2], c[1] / c[2]);
        Some([self.fx * x + self.cx, self.fy * y + self.cy])
    }

    /// The ray through a pixel: (camera centre, unit direction), world.
    pub fn ray(&self, px: [f64; 2]) -> (V3, V3) {
        let (xd, yd) = ((px[0] - self.cx) / self.fx, (px[1] - self.cy) / self.fy);
        // Undo the distortion by fixed-point iteration.
        let (mut x, mut y) = (xd, yd);
        for _ in 0..20 {
            let (dx, dy) = self.distort(x, y);
            x += xd - dx;
            y += yd - dy;
        }
        let r = &self.rotation;
        let d = [r[0][0] * x + r[1][0] * y + r[2][0], r[0][1] * x + r[1][1] * y + r[2][1], r[0][2] * x + r[1][2] * y + r[2][2]];
        (self.centre(), unit(d))
    }
}

/// A rotation from a unit quaternion (w, x, y, z).
pub fn quat_rotation(q: [f64; 4]) -> [[f64; 3]; 3] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt().max(1e-300);
    let [w, x, y, z] = q.map(|v| v / n);
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/// COLMAP's sparse model in its text form (`cameras.txt`, `images.txt`,
/// `points3D.txt`): the cameras, the points with their colours, and each
/// point's mean reprojection error in pixels.
pub struct ColmapModel {
    pub cameras: Vec<Camera>,
    pub points: Vec<V3>,
    pub colors: Vec<[u8; 3]>,
    pub errors: Vec<f64>,
    /// How many photos saw each point.
    pub track: Vec<usize>,
}

pub fn read_colmap(cameras_txt: &str, images_txt: &str, points_txt: &str) -> Result<ColmapModel, String> {
    use std::collections::HashMap;
    let mut lenses: HashMap<u64, (u32, u32, [f64; 4], [f64; 4])> = HashMap::new();
    for line in cameras_txt.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let num = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let (id, w, h) = (f[0].parse::<u64>().map_err(|e| e.to_string())?, num(2) as u32, num(3) as u32);
        let (k, d) = match f[1] {
            "SIMPLE_PINHOLE" => ([num(4), num(4), num(5), num(6)], [0.0; 4]),
            "PINHOLE" => ([num(4), num(5), num(6), num(7)], [0.0; 4]),
            "SIMPLE_RADIAL" => ([num(4), num(4), num(5), num(6)], [num(7), 0.0, 0.0, 0.0]),
            "RADIAL" => ([num(4), num(4), num(5), num(6)], [num(7), num(8), 0.0, 0.0]),
            "OPENCV" | "FULL_OPENCV" => ([num(4), num(5), num(6), num(7)], [num(8), num(9), num(10), num(11)]),
            other => return Err(format!("COLMAP camera model {other} is not read yet (SIMPLE_PINHOLE, PINHOLE, SIMPLE_RADIAL, RADIAL, OPENCV)")),
        };
        lenses.insert(id, (w, h, k, d));
    }
    let mut cameras = Vec::new();
    let mut lines = images_txt.lines().map(str::trim).filter(|l| !l.starts_with('#'));
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let _points_line = lines.next();
        if f.len() < 10 {
            continue;
        }
        let v: Vec<f64> = f[1..8].iter().map(|s| s.parse::<f64>().unwrap_or(0.0)).collect();
        let cam: u64 = f[8].parse().unwrap_or(0);
        let Some((w, h, k, d)) = lenses.get(&cam) else { continue };
        cameras.push(Camera { name: f[9..].join(" "), width: *w, height: *h, fx: k[0], fy: k[1], cx: k[2], cy: k[3], dist: *d, rotation: quat_rotation([v[0], v[1], v[2], v[3]]), translation: [v[4], v[5], v[6]] });
    }
    cameras.sort_by(|a, b| a.name.cmp(&b.name));
    let (mut points, mut colors, mut errors, mut track) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for line in points_txt.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 {
            continue;
        }
        let n = |i: usize| f[i].parse::<f64>().unwrap_or(0.0);
        points.push([n(1), n(2), n(3)]);
        colors.push([n(4) as u8, n(5) as u8, n(6) as u8]);
        errors.push(n(7));
        track.push((f.len() - 8) / 2);
    }
    Ok(ColmapModel { cameras, points, colors, errors, track })
}

/// The point nearest a set of rays (least squares), and the rays' RMS
/// distance from it.
pub fn triangulate(rays: &[(V3, V3)]) -> Option<(V3, f64)> {
    if rays.len() < 2 {
        return None;
    }
    let mut a = [[0.0; 3]; 3];
    let mut b = [0.0; 3];
    for (o, d) in rays {
        for i in 0..3 {
            for j in 0..3 {
                let m = if i == j { 1.0 } else { 0.0 } - d[i] * d[j];
                a[i][j] += m;
                b[i] += m * o[j];
            }
        }
    }
    let flat: Vec<f64> = a.iter().flatten().copied().collect();
    let x = crate::linalg::solve_dense(&flat, &b, 3)?;
    let p = [x[0], x[1], x[2]];
    let rms = (rays.iter().map(|(o, d)| norm(cross(sub(p, *o), *d)).powi(2)).sum::<f64>() / rays.len() as f64).sqrt();
    Some((p, rms))
}

/// Eigen-decomposition of a symmetric n × n matrix (cyclic Jacobi): values
/// and vectors (columns of the returned row-major matrix).
fn sym_eigen(mut a: Vec<f64>, n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |j| *j != i).map(move |j| (i, j))).map(|(i, j)| a[i * n + j].abs()).sum();
        if off < 1e-300 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p * n + q].abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * a[p * n + q]);
                let t = if theta == 0.0 { 1.0 } else { theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt()) };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    ((0..n).map(|i| a[i * n + i]).collect(), v)
}

/// A similarity: x ↦ s·R·x + t.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Similarity {
    pub scale: f64,
    pub rotation: [[f64; 3]; 3],
    pub translation: V3,
}

impl Similarity {
    pub fn identity() -> Similarity {
        Similarity { scale: 1.0, rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], translation: [0.0; 3] }
    }
    pub fn apply(&self, p: V3) -> V3 {
        let r = &self.rotation;
        add(scale([dot(r[0], p), dot(r[1], p), dot(r[2], p)], self.scale), self.translation)
    }
    /// As a 3 × 4 affine matrix (rows).
    pub fn matrix(&self) -> [[f64; 4]; 3] {
        let (r, s, t) = (&self.rotation, self.scale, self.translation);
        [0, 1, 2].map(|i| [r[i][0] * s, r[i][1] * s, r[i][2] * s, t[i]])
    }
    /// b after a.
    pub fn then(&self, b: &Similarity) -> Similarity {
        let r = crate::linalg::mat_mul(&b.rotation, &self.rotation);
        Similarity { scale: self.scale * b.scale, rotation: r, translation: b.apply(self.translation) }
    }
}

/// The similarity carrying `src` onto `dst` best (Horn's quaternion method,
/// Umeyama's scale), and the RMS distance left.
pub fn fit_similarity(src: &[V3], dst: &[V3]) -> Option<(Similarity, f64)> {
    let n = src.len();
    if n < 3 || dst.len() != n {
        return None;
    }
    let cs = scale(src.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / n as f64);
    let cd = scale(dst.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / n as f64);
    let mut m = [[0.0; 3]; 3];
    let mut ss = 0.0;
    for (a, b) in src.iter().zip(dst) {
        let (a, b) = (sub(*a, cs), sub(*b, cd));
        ss += dot(a, a);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += a[i] * b[j];
            }
        }
    }
    if ss < 1e-300 {
        return None;
    }
    let [[sxx, sxy, sxz], [syx, syy, syz], [szx, szy, szz]] = m;
    let nmat = vec![
        sxx + syy + szz, syz - szy, szx - sxz, sxy - syx,
        syz - szy, sxx - syy - szz, sxy + syx, szx + sxz,
        szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy,
        sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz,
    ];
    let (vals, vecs) = sym_eigen(nmat, 4);
    let k = (0..4).max_by(|a, b| vals[*a].total_cmp(&vals[*b]))?;
    let q = [vecs[k], vecs[4 + k], vecs[8 + k], vecs[12 + k]];
    let r = quat_rotation(q);
    let mut num = 0.0;
    for (a, b) in src.iter().zip(dst) {
        let (a, b) = (sub(*a, cs), sub(*b, cd));
        num += dot(b, [dot(r[0], a), dot(r[1], a), dot(r[2], a)]);
    }
    let s = num / ss;
    let t = sub(cd, scale([dot(r[0], cs), dot(r[1], cs), dot(r[2], cs)], s));
    let sim = Similarity { scale: s, rotation: r, translation: t };
    let rms = (src.iter().zip(dst).map(|(a, b)| crate::linalg::dist2(sim.apply(*a), *b)).sum::<f64>() / n as f64).sqrt();
    Some((sim, rms))
}

/// How a reconstruction got its size, and how sure that is.
#[derive(Clone, Debug, Serialize)]
pub struct Scaling {
    /// "markers", "distance", "depth" or "none".
    pub method: String,
    /// Model units to millimetres (and, for markers, the turn onto the sheet).
    pub transform: Similarity,
    /// One standard deviation of the scale, relative (0.002 = 0.2 %).
    pub uncertainty: f64,
    pub detail: String,
}

/// Scale (and floor, and axes) from the marker sheet seen in the photos:
/// each printed corner seen by two or more solved cameras is triangulated,
/// then the sheet's printed corners fitted onto them. `detections` pairs a
/// camera index with what was found in its photo.
pub fn marker_scaling(cameras: &[Camera], detections: &[(usize, Vec<Detection>)], sheet: &Sheet) -> Result<(Scaling, serde_json::Value), String> {
    use std::collections::BTreeMap;
    let mut rays: BTreeMap<(usize, usize), Vec<(V3, V3, usize, [f64; 2])>> = BTreeMap::new();
    for (ci, found) in detections {
        let Some(cam) = cameras.get(*ci) else { continue };
        for d in found {
            if sheet.corners(d.id).is_none() {
                continue;
            }
            for (j, px) in d.corners.iter().enumerate() {
                let (o, dir) = cam.ray(*px);
                rays.entry((d.id, j)).or_default().push((o, dir, *ci, *px));
            }
        }
    }
    let mut src = Vec::new();
    let mut dst = Vec::new();
    let mut reproj = Vec::new();
    let mut markers = std::collections::BTreeSet::new();
    for ((id, j), r) in &rays {
        if r.len() < 2 {
            continue;
        }
        // The rays must cross at an angle to fix a point.
        let spread = r.iter().flat_map(|a| r.iter().map(move |b| dot(a.1, b.1))).fold(1.0f64, f64::min);
        if spread > (3.0f64).to_radians().cos() {
            continue;
        }
        let Some((p, _)) = triangulate(&r.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>()) else { continue };
        let errs: Vec<f64> = r.iter().filter_map(|(_, _, ci, px)| cameras[*ci].project(p).map(|q| ((q[0] - px[0]).powi(2) + (q[1] - px[1]).powi(2)).sqrt())).collect();
        let e = (errs.iter().map(|v| v * v).sum::<f64>() / errs.len().max(1) as f64).sqrt();
        if e > 4.0 {
            continue;
        }
        reproj.push(e);
        let c = sheet.corners(*id).unwrap()[*j];
        src.push(p);
        dst.push([c[0], c[1], 0.0]);
        markers.insert(*id);
    }
    if markers.len() < 2 || src.len() < 6 {
        return Err(format!("the marker sheet was not seen well enough: {} marker(s) located in 3D (two or more are needed, each in two or more photos at different angles)", markers.len()));
    }
    // Fit, drop corners far off, fit again.
    let (mut sim, mut rms) = fit_similarity(&src, &dst).ok_or("the marker corners are degenerate")?;
    for _ in 0..2 {
        let keep: Vec<usize> = (0..src.len()).filter(|&i| crate::linalg::dist(sim.apply(src[i]), dst[i]) < (3.0 * rms).max(sheet.marker * 0.02)).collect();
        if keep.len() < 6 || keep.len() == src.len() {
            break;
        }
        src = keep.iter().map(|&i| src[i]).collect();
        dst = keep.iter().map(|&i| dst[i]).collect();
        (sim, rms) = fit_similarity(&src, &dst).ok_or("the marker corners are degenerate")?;
    }
    // Spread of the printed points about their centre: the lever the scale rests on.
    let cd = scale(dst.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / dst.len() as f64);
    let spread = (dst.iter().map(|p| crate::linalg::dist2(*p, cd)).sum::<f64>() / dst.len() as f64).sqrt();
    let uncertainty = rms / (spread * (dst.len() as f64).sqrt()).max(1e-9);
    let mean_reproj = reproj.iter().sum::<f64>() / reproj.len().max(1) as f64;
    let report = serde_json::json!({
        "markers": markers.into_iter().collect::<Vec<_>>(),
        "corners": src.len(),
        "fitRmsMm": rms,
        "reprojectionPx": mean_reproj,
        "photosWithMarkers": detections.iter().filter(|(_, d)| !d.is_empty()).count(),
    });
    Ok((
        Scaling {
            method: "markers".into(),
            transform: sim,
            uncertainty,
            detail: format!("{} marker corners on the sheet fitted to {:.3} mm RMS; scale ±{:.2} %; the sheet is the floor (z = 0) and its centre the origin", src.len(), rms, uncertainty * 100.0),
        },
        report,
    ))
}

/// Scale from a known distance between two picked points: the factor and
/// its uncertainty given how precisely each point was picked (model units).
pub fn distance_scaling(a: V3, b: V3, distance_mm: f64, pick: f64) -> Result<Scaling, String> {
    let d = crate::linalg::dist(a, b);
    if !(d > 0.0) || !(distance_mm > 0.0) {
        return Err("pick two different points and give their distance (mm)".into());
    }
    let s = distance_mm / d;
    let uncertainty = std::f64::consts::SQRT_2 * pick.max(0.0) / d;
    Ok(Scaling {
        method: "distance".into(),
        transform: Similarity { scale: s, ..Similarity::identity() },
        uncertainty,
        detail: format!("{distance_mm} mm between two picked points ({d:.4} units apart): ×{s:.5}, ±{:.2} % from picking to ±{:.3} units", uncertainty * 100.0, pick),
    })
}

/// The floor: the largest plane, with the most of what is off it above.
pub struct Floor {
    pub normal: V3,
    pub offset: f64,
    pub inliers: usize,
    pub rms: f64,
}

/// Finds the floor plane (RANSAC over `tolerance`, refined by least squares).
pub fn floor(points: &[V3], tolerance: f64) -> Option<Floor> {
    if points.len() < 50 {
        return None;
    }
    let step = (points.len() / 20000).max(1);
    let pts: Vec<V3> = points.iter().step_by(step).copied().collect();
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    let mut rnd = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut best: Option<(usize, V3, f64)> = None;
    for _ in 0..400 {
        let (a, b, c) = (pts[rnd(pts.len())], pts[rnd(pts.len())], pts[rnd(pts.len())]);
        let n = cross(sub(b, a), sub(c, a));
        if norm(n) < 1e-12 {
            continue;
        }
        let n = unit(n);
        let off = dot(n, a);
        let count = pts.iter().filter(|p| (dot(n, **p) - off).abs() < tolerance).count();
        if best.as_ref().is_none_or(|b| count > b.0) {
            best = Some((count, n, off));
        }
    }
    let (_, mut n, mut off) = best?;
    // Refine: least squares over the inliers, the band narrowed to the plane's own spread.
    let mut tol = tolerance;
    let mut inl: Vec<V3> = Vec::new();
    let mut c = [0.0; 3];
    for _ in 0..6 {
        inl = pts.iter().filter(|p| (dot(n, **p) - off).abs() < tol).copied().collect();
        if inl.len() < 10 {
            return None;
        }
        let (cc, cov) = crate::linalg::covariance(&inl);
        c = cc;
        n = sym_eigen3(cov).1[0];
        off = dot(n, c);
        let mut d: Vec<f64> = inl.iter().map(|p| (dot(n, *p) - off).abs()).collect();
        d.sort_by(|a, b| a.total_cmp(b));
        tol = (d[d.len() / 2] * 4.0).clamp(tolerance * 0.05, tolerance);
    }
    let mut n = n;
    // Up is the side most of the rest stands on.
    let above = pts.iter().filter(|p| dot(n, sub(**p, c)) > tolerance).count();
    let below = pts.iter().filter(|p| dot(n, sub(**p, c)) < -tolerance).count();
    if below > above {
        n = scale(n, -1.0);
    }
    let offset = dot(n, c);
    let rms = (inl.iter().map(|p| (dot(n, *p) - offset).powi(2)).sum::<f64>() / inl.len() as f64).sqrt();
    Some(Floor { normal: n, offset, inliers: inl.len() * step, rms })
}

/// A floor seen from the cameras: up is the side they stand on (one looks
/// down at a table). None when as many as a quarter stand on each side: that
/// plane is a wall or a ground the photos walk across, not a floor under an
/// object.
pub fn face_cameras(f: Floor, eyes: &[V3]) -> Option<Floor> {
    if eyes.is_empty() {
        return Some(f);
    }
    let above = eyes.iter().filter(|e| dot(f.normal, **e) > f.offset).count();
    let below = eyes.len() - above;
    if above.min(below) * 4 >= eyes.len() {
        return None;
    }
    Some(if below > above { Floor { normal: scale(f.normal, -1.0), offset: -f.offset, ..f } } else { f })
}

/// Where the cameras look: the point nearest all their axes, and how far
/// the axes pass from it for their distance (RMS over median). An object
/// photographed round about is near 0; a room, a street or a façade walked
/// through is well over 0.25.
pub struct Aim {
    pub point: V3,
    pub spread: f64,
}

pub fn aim(cameras: &[Camera]) -> Option<Aim> {
    if cameras.len() < 3 {
        return None;
    }
    let rays: Vec<(V3, V3)> = cameras.iter().map(|c| (c.centre(), c.forward())).collect();
    let (point, rms) = triangulate(&rays)?;
    let mut d: Vec<f64> = rays.iter().map(|r| crate::linalg::dist(r.0, point)).collect();
    d.sort_by(|a, b| a.total_cmp(b));
    let median = d[d.len() / 2].max(1e-12);
    // Behind most of them: they do not look at it.
    let ahead = rays.iter().filter(|r| dot(sub(point, r.0), r.1) > 0.0).count();
    Some(Aim { point, spread: if ahead * 2 < rays.len() { f64::INFINITY } else { rms / median } })
}

/// Whether the photos are of a scene (walked through) rather than of one object (gone round).
pub fn is_scene(cameras: &[Camera]) -> bool {
    aim(cameras).is_none_or(|a| a.spread > 0.25)
}

/// The turn and shift putting a floor at z = 0, facing up, with the object
/// (what stands above it) centred on the origin.
pub fn level(points: &[V3], f: &Floor, tolerance: f64) -> Similarity {
    let z = f.normal;
    let x = unit(cross(if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }, z));
    let x = unit(cross(z, x));
    let y = cross(z, x);
    let rot = [x, y, z];
    let above: Vec<V3> = points.iter().filter(|p| dot(z, **p) - f.offset > tolerance * 2.0).copied().collect();
    let c = if above.is_empty() { [0.0; 3] } else { scale(above.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / above.len() as f64) };
    let rc = [dot(x, c), dot(y, c)];
    Similarity { scale: 1.0, rotation: rot, translation: [-rc[0], -rc[1], -f.offset] }
}

/// How well the photos went round the object.
#[derive(Clone, Debug, Serialize)]
pub struct Coverage {
    /// Photos in each 30° sector round the up axis (from +x, anticlockwise).
    pub sectors: [usize; 12],
    pub lowest_elevation: f64,
    pub highest_elevation: f64,
    /// Plain-language gaps.
    pub gaps: Vec<String>,
}

pub fn coverage(centres: &[V3], target: V3, up: V3) -> Coverage {
    let up = unit(up);
    let x = unit(cross(if up[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }, up));
    let x = unit(cross(up, x));
    let y = cross(up, x);
    let mut sectors = [0usize; 12];
    let (mut lo, mut hi) = (90.0f64, -90.0f64);
    for c in centres {
        let d = sub(*c, target);
        let az = dot(d, y).atan2(dot(d, x)).to_degrees().rem_euclid(360.0);
        let el = (dot(d, up) / norm(d).max(1e-12)).clamp(-1.0, 1.0).asin().to_degrees();
        sectors[((az / 30.0) as usize).min(11)] += 1;
        lo = lo.min(el);
        hi = hi.max(el);
    }
    let mut gaps = Vec::new();
    // Runs of empty sectors.
    if centres.is_empty() {
        gaps.push("no cameras were solved".into());
    } else {
        let mut i = 0;
        let start = (0..12).find(|&k| sectors[k] > 0).unwrap_or(0);
        while i < 12 {
            let k = (start + i) % 12;
            if sectors[k] == 0 {
                let mut n = 0;
                while i + n < 12 && sectors[(start + i + n) % 12] == 0 {
                    n += 1;
                }
                gaps.push(format!("no photos from {}°–{}° round the object ({}° gap)", k * 30, ((k + n) * 30) % 360, n * 30));
                i += n;
            } else {
                i += 1;
            }
        }
        if lo > 25.0 {
            gaps.push(format!("every photo looks down from {lo:.0}° or more: the lower sides are thin; add a ring nearer level"));
        }
        if hi < 35.0 {
            gaps.push(format!("no photo from above {hi:.0}°: the top is thin; add a ring looking down"));
        }
    }
    Coverage { sectors, lowest_elevation: lo.min(hi), highest_elevation: hi.max(lo), gaps }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linalg::dist;

    fn look_at(eye: V3, target: V3, f: f64) -> Camera {
        let z = unit(sub(target, eye));
        let x = unit(cross(z, [0.0, 0.0, -1.0]));
        let y = cross(z, x);
        let r = [x, y, z];
        let t = scale([dot(x, eye), dot(y, eye), dot(z, eye)], -1.0);
        Camera { name: String::new(), width: 1200, height: 900, fx: f, fy: f, cx: 600.0, cy: 450.0, dist: [0.05, -0.01, 0.0, 0.0], rotation: r, translation: t }
    }

    #[test]
    fn rays_cross_where_the_point_projects() {
        let cams: Vec<Camera> = (0..6).map(|i| look_at([300.0 * (i as f64).cos(), 300.0 * (i as f64).sin(), 200.0], [0.0; 3], 1100.0)).collect();
        let p = [12.0, -7.0, 30.0];
        let rays: Vec<(V3, V3)> = cams.iter().map(|c| c.ray(c.project(p).unwrap())).collect();
        let (q, rms) = triangulate(&rays).unwrap();
        assert!(dist(p, q) < 1e-6 && rms < 1e-6, "{q:?} {rms}");
    }

    #[test]
    fn a_similarity_is_recovered() {
        let truth = Similarity { scale: 0.037, rotation: quat_rotation([0.8, 0.2, -0.3, 0.4]), translation: [3.0, -1.0, 0.5] };
        let src: Vec<V3> = (0..10).map(|i| [(i as f64 * 1.7).sin() * 50.0, (i as f64 * 0.9).cos() * 30.0, i as f64 * 2.0]).collect();
        let dst: Vec<V3> = src.iter().map(|p| truth.apply(*p)).collect();
        let (s, rms) = fit_similarity(&src, &dst).unwrap();
        assert!((s.scale - 0.037).abs() < 1e-9 && rms < 1e-9, "{s:?}");
        // Coplanar points (the sheet) work as well.
        let flat: Vec<V3> = src.iter().map(|p| [p[0], p[1], 0.0]).collect();
        let dst: Vec<V3> = flat.iter().map(|p| truth.apply(*p)).collect();
        let (s, rms) = fit_similarity(&flat, &dst).unwrap();
        assert!((s.scale - 0.037).abs() < 1e-9 && rms < 1e-9, "{s:?}");
    }

    #[test]
    fn the_marker_sheet_scales_and_levels_a_reconstruction() {
        // A reconstruction in arbitrary units: the world turned, shrunk 40×, shifted.
        let model = Similarity { scale: 1.0 / 40.0, rotation: quat_rotation([0.9, 0.3, 0.1, -0.2]), translation: [1.0, 2.0, -0.5] };
        let sheet = Sheet::default();
        let world_cams: Vec<Camera> = (0..10).map(|i| {
            let a = i as f64 * 0.63;
            look_at([350.0 * a.cos(), 350.0 * a.sin(), 280.0], [0.0; 3], 1400.0)
        }).collect();
        // The cameras as the engine solved them (in model units): the same views.
        let cams: Vec<Camera> = world_cams
            .iter()
            .map(|c| {
                // x_cam = R·x_w + t, x_w = (R_mᵀ(x_m − t_m))/s_m  ⇒  scaled camera.
                let rm = &model.rotation;
                let rmt = [[rm[0][0], rm[1][0], rm[2][0]], [rm[0][1], rm[1][1], rm[2][1]], [rm[0][2], rm[1][2], rm[2][2]]];
                let r = crate::linalg::mat_mul(&c.rotation, &rmt);
                let rt = [dot(r[0], model.translation), dot(r[1], model.translation), dot(r[2], model.translation)];
                // Scale the camera frame by s_m so depths match the model's units.
                let t = sub(scale(c.translation, model.scale), rt);
                Camera { rotation: r, translation: t, ..c.clone() }
            })
            .collect();
        // What each camera sees of the sheet, with ±0.3 px noise.
        let mut noise = 0.0f64;
        let detections: Vec<(usize, Vec<Detection>)> = world_cams
            .iter()
            .enumerate()
            .map(|(ci, c)| {
                let found = sheet
                    .markers
                    .iter()
                    .map(|(id, _)| {
                        let corners = sheet.corners(*id).unwrap().map(|p| {
                            noise += 1.0;
                            let q = c.project([p[0], p[1], 0.0]).unwrap();
                            [q[0] + 0.3 * (noise * 1.7).sin(), q[1] + 0.3 * (noise * 2.3).cos()]
                        });
                        Detection { id: *id, corners, errors: 0 }
                    })
                    .collect();
                (ci, found)
            })
            .collect();
        let (s, report) = marker_scaling(&cams, &detections, &sheet).unwrap();
        assert!((s.transform.scale - 40.0).abs() / 40.0 < 0.002, "scale {} ({report})", s.transform.scale);
        assert!(s.uncertainty < 0.002, "{}", s.detail);
        // A point 50 mm above the sheet's centre comes back there.
        let p_model = model.apply([0.0, 0.0, 50.0]);
        let back = s.transform.apply(p_model);
        assert!(dist(back, [0.0, 0.0, 50.0]) < 0.3, "{back:?}");
    }

    #[test]
    fn two_points_scale_with_their_uncertainty() {
        let s = distance_scaling([0.0; 3], [2.0, 0.0, 0.0], 100.0, 0.01).unwrap();
        assert!((s.transform.scale - 50.0).abs() < 1e-12);
        assert!((s.uncertainty - 0.00707).abs() < 1e-4);
    }

    #[test]
    fn the_floor_is_found_and_levelled() {
        // A tilted floor with a box standing on it.
        let tilt = quat_rotation([0.95, 0.2, 0.1, 0.0]);
        let mut pts = Vec::new();
        for i in 0..80 {
            for j in 0..80 {
                pts.push([i as f64 * 2.0 - 80.0, j as f64 * 2.0 - 80.0, 0.0]);
            }
        }
        for i in 0..20 {
            for j in 0..20 {
                pts.push([10.0 + i as f64, 5.0 + j as f64, 30.0]);
                pts.push([10.0 + i as f64, 5.0, j as f64 * 1.5]);
            }
        }
        let pts: Vec<V3> = pts.iter().map(|p| add(crate::linalg::mat_vec(&tilt, *p), [5.0, 5.0, 5.0])).collect();
        let f = floor(&pts, 0.5).unwrap();
        let l = level(&pts, &f, 0.5);
        let top = l.apply(add(crate::linalg::mat_vec(&tilt, [20.0, 15.0, 30.0]), [5.0, 5.0, 5.0]));
        assert!((top[2] - 30.0).abs() < 1e-6, "{top:?}");
        let ground = l.apply(add(crate::linalg::mat_vec(&tilt, [-50.0, 60.0, 0.0]), [5.0, 5.0, 5.0]));
        assert!(ground[2].abs() < 1e-6);
    }

    #[test]
    fn coverage_names_the_gaps() {
        let centres: Vec<V3> = (0..9).map(|i| {
            let a = (i as f64 * 20.0).to_radians();
            [100.0 * a.cos(), 100.0 * a.sin(), 60.0]
        }).collect();
        let c = coverage(&centres, [0.0; 3], [0.0, 0.0, 1.0]);
        assert!(c.gaps.iter().any(|g| g.contains("gap")), "{:?}", c.gaps);
        assert!(c.lowest_elevation > 25.0 && c.gaps.iter().any(|g| g.contains("lower sides")));
    }

    #[test]
    fn an_object_gone_round_is_not_a_scene_walked_through() {
        // Round an object: every camera looks at it.
        let round: Vec<Camera> = (0..24).map(|i| look_at([300.0 * (i as f64 * 0.26).cos(), 300.0 * (i as f64 * 0.26).sin(), 150.0 + (i % 3) as f64 * 60.0], [0.0, 0.0, 20.0], 1100.0)).collect();
        let a = aim(&round).unwrap();
        assert!(dist(a.point, [0.0, 0.0, 20.0]) < 1.0 && a.spread < 0.01 && !is_scene(&round), "{:?} {}", a.point, a.spread);
        // Along a street, looking at the façades on either side.
        let street: Vec<Camera> = (0..24).map(|i| {
            let x = i as f64 * 500.0;
            look_at([x, 0.0, 1600.0], [x + 300.0, if i % 2 == 0 { 4000.0 } else { -4000.0 }, 1500.0], 1100.0)
        }).collect();
        assert!(is_scene(&street), "spread {}", aim(&street).map(|a| a.spread).unwrap_or(-1.0));
    }

    #[test]
    fn the_floor_faces_the_cameras() {
        let f = Floor { normal: [0.0, 0.0, -1.0], offset: 0.0, inliers: 100, rms: 0.1 };
        // Cameras above z = 0: the floor is turned to face them.
        let eyes: Vec<V3> = (0..10).map(|i| [i as f64, 0.0, 300.0]).collect();
        let up = face_cameras(Floor { ..f }, &eyes).unwrap();
        assert_eq!((up.normal, up.offset), ([0.0, 0.0, 1.0], 0.0));
        // Half above, half below: not a floor under an object.
        let split: Vec<V3> = (0..10).map(|i| [i as f64, 0.0, if i % 2 == 0 { 300.0 } else { -300.0 }]).collect();
        assert!(face_cameras(f, &split).is_none());
    }
}
