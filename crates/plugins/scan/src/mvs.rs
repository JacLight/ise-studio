//! Dense multi-view stereo on the CPU: a depth and a surface normal for
//! every pixel of every photo by PatchMatch (Bleyer, Rhemann and Rother
//! 2011, in the multi-view form of Galliani, Lasinger and Schindler 2015 and
//! Schönberger et al. 2016), each photo matched against the few others that
//! see the same surface from usefully different angles; then the depth maps
//! fused into one oriented, coloured point cloud wherever several photos
//! agree.
//!
//! COLMAP does this only with CUDA; this runs on any computer. The cameras
//! come from structure from motion, undistorted (pinhole); the depth range
//! of each photo and the choice of its partners come from the sparse points.
//!
//! Per pixel the state is a small plane (a depth along the pixel's ray and a
//! normal). A hypothesis is scored by warping a window of the photo into each
//! partner through the plane's homography and comparing them by normalised
//! cross-correlation; the best half of the partners count (the rest may not
//! see that spot). Hypotheses spread from neighbours on a checkerboard (half
//! the pixels at a time, so the passes run in parallel) and are refined by
//! ever smaller random changes.

use crate::V3;
use crate::photo::Camera;
use crate::points::Cloud;
use rayon::prelude::*;

/// A photo as matched: grey levels and colours (8 bits each), row by row.
pub struct Photo {
    pub width: usize,
    pub height: usize,
    pub gray: Vec<u8>,
    pub rgb: Vec<[u8; 3]>,
}

impl Photo {
    /// From 8-bit colour, shrunk by whole factors (box filter) until its longer side is at most `max`.
    pub fn new(width: usize, height: usize, rgb: Vec<[u8; 3]>, max: usize) -> Photo {
        let f = width.max(height).div_ceil(max.max(1)).max(1);
        let (w, h) = (width / f, height / f);
        let rgb = if f == 1 {
            rgb
        } else {
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    let mut s = [0u32; 3];
                    for dy in 0..f {
                        for dx in 0..f {
                            let p = rgb[(y * f + dy) * width + x * f + dx];
                            for k in 0..3 {
                                s[k] += p[k] as u32;
                            }
                        }
                    }
                    out.push(s.map(|v| (v / (f * f) as u32) as u8));
                }
            }
            out
        };
        let gray = rgb.iter().map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).round() as u8).collect();
        Photo { width: w, height: h, gray, rgb }
    }

    fn at(&self, x: f32, y: f32) -> Option<f32> {
        if !(x >= 0.0 && y >= 0.0 && x < (self.width - 1) as f32 && y < (self.height - 1) as f32) {
            return None;
        }
        let (x0, y0) = (x as usize, y as usize);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let i = y0 * self.width + x0;
        let g = |k: usize| self.gray[k] as f32 * (1.0 / 255.0);
        let top = g(i) + (g(i + 1) - g(i)) * fx;
        let bottom = g(i + self.width) + (g(i + self.width + 1) - g(i + self.width)) * fx;
        Some(top + (bottom - top) * fy)
    }

    /// Grey level 0…1 of a pixel.
    fn grey(&self, i: usize) -> f32 {
        self.gray[i] as f32 * (1.0 / 255.0)
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Partner photos per photo.
    pub sources: usize,
    /// PatchMatch rounds (each: both colours of the checkerboard, then refinement).
    pub iterations: usize,
    /// Window half-size and sampling step, pixels (radius 4, step 2: 5 × 5 samples over 9 × 9 pixels).
    pub radius: i32,
    pub step: i32,
    /// Largest matching cost kept (1 − NCC): 0.5 keeps correlations above 0.5.
    pub max_cost: f32,
    /// Other photos whose depth must agree for a point to be kept.
    pub consistent: usize,
    /// Relative depth agreement for that.
    pub tolerance: f32,
    /// Fuse every n-th pixel (each way).
    pub fuse_step: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { sources: 5, iterations: 4, radius: 4, step: 2, max_cost: 0.5, consistent: 2, tolerance: 0.01, fuse_step: 2 }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Report {
    pub photos: usize,
    pub matched: usize,
    pub skipped: Vec<String>,
    /// Pixels with a confident depth, over all photos.
    pub depths: usize,
    pub points: usize,
    pub seconds: f64,
}

/// A pinhole camera at a photo's size.
#[derive(Clone, Copy)]
struct Cam {
    fx: f64,
    fy: f64,
    cx: f64,
    cy: f64,
    r: [[f64; 3]; 3],
    t: V3,
    c: V3,
    w: usize,
    h: usize,
}

impl Cam {
    fn new(c: &Camera, p: &Photo) -> Cam {
        let (sx, sy) = (p.width as f64 / c.width.max(1) as f64, p.height as f64 / c.height.max(1) as f64);
        // COLMAP's lens puts a pixel's centre at +0.5 (the image's corner is 0); here pixel x is at x.
        Cam { fx: c.fx * sx, fy: c.fy * sy, cx: c.cx * sx - 0.5, cy: c.cy * sy - 0.5, r: c.rotation, t: c.translation, c: c.centre(), w: p.width, h: p.height }
    }

    fn to_cam(&self, p: V3) -> V3 {
        let r = &self.r;
        [dot(r[0], p) + self.t[0], dot(r[1], p) + self.t[1], dot(r[2], p) + self.t[2]]
    }

    fn to_world(&self, q: V3) -> V3 {
        let r = &self.r;
        let d = [q[0] - self.t[0], q[1] - self.t[1], q[2] - self.t[2]];
        [r[0][0] * d[0] + r[1][0] * d[1] + r[2][0] * d[2], r[0][1] * d[0] + r[1][1] * d[1] + r[2][1] * d[2], r[0][2] * d[0] + r[1][2] * d[1] + r[2][2] * d[2]]
    }

    fn rotate_to_world(&self, n: V3) -> V3 {
        let r = &self.r;
        [r[0][0] * n[0] + r[1][0] * n[1] + r[2][0] * n[2], r[0][1] * n[0] + r[1][1] * n[1] + r[2][1] * n[2], r[0][2] * n[0] + r[1][2] * n[1] + r[2][2] * n[2]]
    }

    fn pixel(&self, q: V3) -> Option<[f64; 2]> {
        (q[2] > 1e-12).then(|| [self.fx * q[0] / q[2] + self.cx, self.fy * q[1] / q[2] + self.cy])
    }

    /// The ray through a pixel at depth 1 (camera frame).
    fn ray(&self, x: f64, y: f64) -> V3 {
        [(x - self.cx) / self.fx, (y - self.cy) / self.fy, 1.0]
    }
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: V3) -> V3 {
    let n = dot(a, a).sqrt().max(1e-300);
    [a[0] / n, a[1] / n, a[2] / n]
}

/// A small fast random number generator (xorshift), seeded per pixel so runs repeat.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03 | 1)
    }
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn signed(&mut self) -> f64 {
        2.0 * self.next() - 1.0
    }
}

/// Partners for each photo: the others sharing the most sparse points seen
/// at a useful angle (a few degrees to a few tens of degrees apart).
fn partners(cams: &[Cam], sparse: &[V3], k: usize) -> Vec<Vec<usize>> {
    let n = cams.len();
    let seen: Vec<Vec<usize>> = sparse
        .par_iter()
        .map(|p| {
            (0..n)
                .filter(|&i| {
                    let q = cams[i].to_cam(*p);
                    cams[i].pixel(q).is_some_and(|px| px[0] >= 0.0 && px[1] >= 0.0 && px[0] < cams[i].w as f64 && px[1] < cams[i].h as f64)
                })
                .collect()
        })
        .collect();
    let mut score = vec![vec![0.0f64; n]; n];
    let mut shared = vec![vec![0usize; n]; n];
    for (pi, views) in seen.iter().enumerate() {
        let p = sparse[pi];
        for (a, &i) in views.iter().enumerate() {
            let ri = unit([p[0] - cams[i].c[0], p[1] - cams[i].c[1], p[2] - cams[i].c[2]]);
            for &j in &views[a + 1..] {
                let rj = unit([p[0] - cams[j].c[0], p[1] - cams[j].c[1], p[2] - cams[j].c[2]]);
                let angle = dot(ri, rj).clamp(-1.0, 1.0).acos().to_degrees();
                let w = if angle < 3.0 {
                    (angle / 3.0).powi(2)
                } else if angle <= 30.0 {
                    1.0
                } else {
                    (-((angle - 30.0) / 15.0).powi(2)).exp()
                };
                score[i][j] += w;
                score[j][i] += w;
                shared[i][j] += 1;
                shared[j][i] += 1;
            }
        }
    }
    (0..n)
        .map(|i| {
            let mut order: Vec<usize> = (0..n).filter(|&j| j != i && shared[i][j] >= 8 && score[i][j] > 1.0).collect();
            order.sort_by(|&a, &b| score[i][b].total_cmp(&score[i][a]));
            order.truncate(k);
            order
        })
        .collect()
}

/// A photo's depths and normals (camera frame); depth 0 where unknown.
struct DepthMap {
    depth: Vec<f32>,
    /// Unit normals packed to 8 bits per component.
    normal: Vec<[i8; 3]>,
}

fn pack(n: V3) -> [i8; 3] {
    n.map(|v| (v * 127.0).round().clamp(-127.0, 127.0) as i8)
}

fn unpack(n: [i8; 3]) -> V3 {
    unit(n.map(|v| v as f64 / 127.0))
}

/// One photo's relation to a partner: x_s = R·x_r + t, and the partner's lens.
struct Pair {
    r: [[f64; 3]; 3],
    t: V3,
    fx: f64,
    fy: f64,
    cx: f64,
    cy: f64,
}

fn relative(a: &Cam, b: &Cam) -> Pair {
    // x_b = R_b R_aᵀ (x_a − t_a) + t_b
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| b.r[i][k] * a.r[j][k]).sum();
        }
    }
    let rt = [dot(r[0], a.t), dot(r[1], a.t), dot(r[2], a.t)];
    Pair { r, t: [b.t[0] - rt[0], b.t[1] - rt[1], b.t[2] - rt[2]], fx: b.fx, fy: b.fy, cx: b.cx, cy: b.cy }
}

struct Matcher<'a> {
    cam: Cam,
    photo: &'a Photo,
    pairs: Vec<(Pair, &'a Photo)>,
    /// Window offsets (dx, dy), and 1/σ of each pixel's plain window (NaN: no texture, not matched).
    offsets: Vec<(i32, i32)>,
    inv_std: Vec<f32>,
    radius: i32,
    near: f64,
    far: f64,
}

/// A pixel's window in its own photo: grey levels and bilateral weights
/// (samples like the centre in grey and near it count more, so a window
/// straddling an object's edge matches the object's side, not the
/// background's), with the weighted mean and 1/σ.
struct Window {
    a: [f32; MAX_SAMPLES],
    w: [f32; MAX_SAMPLES],
    sum_w: f32,
    mean: f32,
    inv_std: f32,
}

const MAX_SAMPLES: usize = 81;

impl Matcher<'_> {
    fn window(&self, x: usize, y: usize) -> Option<Window> {
        if !self.inv_std[y * self.photo.width + x].is_finite() {
            return None;
        }
        let centre = self.photo.grey(y * self.photo.width + x);
        let mut win = Window { a: [0.0; MAX_SAMPLES], w: [0.0; MAX_SAMPLES], sum_w: 0.0, mean: 0.0, inv_std: 0.0 };
        let (sigma_g, sigma_d) = (0.1f32, self.radius.max(1) as f32);
        let (mut s, mut ss) = (0.0f32, 0.0f32);
        for (k, &(dx, dy)) in self.offsets.iter().enumerate() {
            let a = self.photo.grey(((y as i32 + dy) as usize) * self.photo.width + (x as i32 + dx) as usize);
            let w = (-(a - centre).powi(2) / (2.0 * sigma_g * sigma_g) - (dx * dx + dy * dy) as f32 / (2.0 * sigma_d * sigma_d)).exp();
            win.a[k] = a;
            win.w[k] = w;
            win.sum_w += w;
            s += w * a;
            ss += w * a * a;
        }
        win.mean = s / win.sum_w;
        let var = (ss / win.sum_w - win.mean * win.mean).max(0.0);
        if var < 1e-5 {
            return None;
        }
        win.inv_std = 1.0 / var.sqrt();
        Some(win)
    }

    /// 1 − weighted NCC of the window at (x, y) through the plane (depth d, normal n) in each partner; the best half averaged.
    fn cost(&self, x: usize, y: usize, win: &Window, d: f64, n: V3) -> f32 {
        if d <= 0.0 {
            return 2.0;
        }
        let q = self.cam.ray(x as f64, y as f64);
        let plane = d * dot(n, q);
        if plane >= -1e-12 {
            return 2.0;
        }
        let (m_a, inv) = (win.mean, win.inv_std);
        let mut costs = [2.0f32; 16];
        for (k, (pair, src)) in self.pairs.iter().enumerate() {
            // H = K_s (R + t nᵀ / plane) K_r⁻¹, applied to q + (dx/fx, dy/fy, 0).
            let mut m = [[0.0f64; 3]; 3];
            for r in 0..3 {
                for c in 0..3 {
                    m[r][c] = pair.r[r][c] + pair.t[r] * n[c] / plane;
                }
            }
            let row = |r: usize, v: V3| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2];
            let base = [row(0, q), row(1, q), row(2, q)];
            let ex = [m[0][0] / self.cam.fx, m[1][0] / self.cam.fx, m[2][0] / self.cam.fx];
            let ey = [m[0][1] / self.cam.fy, m[1][1] / self.cam.fy, m[2][1] / self.cam.fy];
            let (mut sb, mut sbb, mut sab) = (0.0f32, 0.0f32, 0.0f32);
            let mut ok = true;
            for (k, &(dx, dy)) in self.offsets.iter().enumerate() {
                let (dx, dy) = (dx as f64, dy as f64);
                let v = [base[0] + ex[0] * dx + ey[0] * dy, base[1] + ex[1] * dx + ey[1] * dy, base[2] + ex[2] * dx + ey[2] * dy];
                if v[2] <= 1e-12 {
                    ok = false;
                    break;
                }
                let (u, w) = (pair.fx * v[0] / v[2] + pair.cx, pair.fy * v[1] / v[2] + pair.cy);
                let Some(b) = src.at(u as f32, w as f32) else {
                    ok = false;
                    break;
                };
                let wt = win.w[k];
                sb += wt * b;
                sbb += wt * b * b;
                sab += wt * win.a[k] * b;
            }
            if !ok {
                continue;
            }
            let m_b = sb / win.sum_w;
            let var_b = (sbb / win.sum_w - m_b * m_b).max(0.0);
            if var_b < 1e-8 {
                continue;
            }
            let ncc = (sab / win.sum_w - m_a * m_b) * inv / var_b.sqrt();
            costs[k] = (1.0 - ncc).clamp(0.0, 2.0);
        }
        let n = self.pairs.len().min(16);
        let c = &mut costs[..n];
        c.sort_by(|a, b| a.total_cmp(b));
        let best = n.div_ceil(2).max(1);
        c[..best].iter().sum::<f32>() / best as f32
    }

    /// A random plane at a pixel: inverse depth uniform in range, normal within ~60° of facing the camera.
    fn random(&self, x: usize, y: usize, rng: &mut Rng) -> (f64, V3) {
        let inv = 1.0 / self.far + rng.next() * (1.0 / self.near - 1.0 / self.far);
        let q = unit(self.cam.ray(x as f64, y as f64));
        let n = unit([-q[0] + 0.8 * rng.signed(), -q[1] + 0.8 * rng.signed(), -q[2] + 0.8 * rng.signed()]);
        (1.0 / inv, if dot(n, q) < 0.0 { n } else { [-n[0], -n[1], -n[2]] })
    }

    /// A neighbour's plane carried to this pixel: the depth where this pixel's ray meets it.
    fn carry(&self, x: usize, y: usize, from: (usize, usize), d: f64, n: V3) -> Option<f64> {
        let p = {
            let q = self.cam.ray(from.0 as f64, from.1 as f64);
            [q[0] * d, q[1] * d, q[2] * d]
        };
        let q = self.cam.ray(x as f64, y as f64);
        let den = dot(n, q);
        if den.abs() < 1e-9 {
            return None;
        }
        let depth = dot(n, p) / den;
        (depth > self.near * 0.5 && depth < self.far * 2.0).then_some(depth)
    }

    fn run(&self, o: &Options, seed: u64, cancel: &(dyn Fn() -> bool + Sync)) -> Option<DepthMap> {
        let (w, h) = (self.photo.width, self.photo.height);
        let r = self.radius as usize;
        let inside = |x: usize, y: usize| x >= r && y >= r && x + r < w && y + r < h;
        let mut depth = vec![0.0f64; w * h];
        let mut normal = vec![[0.0f64; 3]; w * h];
        let mut cost = vec![2.0f32; w * h];
        // Random start.
        let init: Vec<(f64, V3, f32)> = (0..w * h)
            .into_par_iter()
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let Some(win) = inside(x, y).then(|| self.window(x, y)).flatten() else {
                    return (0.0, [0.0, 0.0, -1.0], 2.0);
                };
                let mut rng = Rng::new(seed ^ (i as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
                let (d, n) = self.random(x, y, &mut rng);
                (d, n, self.cost(x, y, &win, d, n))
            })
            .collect();
        for (i, (d, n, c)) in init.into_iter().enumerate() {
            depth[i] = d;
            normal[i] = n;
            cost[i] = c;
        }
        const NEIGHBOURS: [(i32, i32); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-5, 0), (5, 0), (0, -5), (0, 5)];
        for iteration in 0..o.iterations {
            if cancel() {
                return None;
            }
            let spread = 0.5f64.powi(iteration as i32);
            for colour in 0..2usize {
                let rows: Vec<Vec<(usize, f64, V3, f32)>> = (0..h)
                    .into_par_iter()
                    .map(|y| {
                        let mut out = Vec::new();
                        for x in ((y + colour) % 2..w).step_by(2) {
                            let i = y * w + x;
                            let Some(win) = inside(x, y).then(|| self.window(x, y)).flatten() else { continue };
                            let mut rng = Rng::new(seed ^ ((i as u64) << 8 | (iteration as u64) << 2 | colour as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
                            let (mut bd, mut bn, mut bc) = (depth[i], normal[i], cost[i]);
                            // Propagation: the neighbours' planes.
                            for (dx, dy) in NEIGHBOURS {
                                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                                    continue;
                                }
                                let j = ny as usize * w + nx as usize;
                                if cost[j] >= 2.0 {
                                    continue;
                                }
                                let Some(d) = self.carry(x, y, (nx as usize, ny as usize), depth[j], normal[j]) else { continue };
                                let c = self.cost(x, y, &win, d, normal[j]);
                                if c < bc {
                                    (bd, bn, bc) = (d, normal[j], c);
                                }
                            }
                            // Refinement: smaller and smaller changes, and a fresh guess.
                            let q = unit(self.cam.ray(x as f64, y as f64));
                            let perturb_n = |n: V3, rng: &mut Rng| {
                                let a = 0.6 * spread;
                                let m = unit([n[0] + a * rng.signed(), n[1] + a * rng.signed(), n[2] + a * rng.signed()]);
                                if dot(m, q) < -0.1 { m } else { n }
                            };
                            let perturb_d = |d: f64, rng: &mut Rng| {
                                let inv = 1.0 / d * (1.0 + 0.1 * spread * rng.signed());
                                (inv > 0.0).then(|| (1.0 / inv).clamp(self.near * 0.5, self.far * 2.0)).unwrap_or(d)
                            };
                            let (rd, rn) = self.random(x, y, &mut rng);
                            let pd = perturb_d(bd, &mut rng);
                            let pn = perturb_n(bn, &mut rng);
                            for (d, n) in [(rd, rn), (pd, bn), (bd, pn), (pd, pn)] {
                                let c = self.cost(x, y, &win, d, n);
                                if c < bc {
                                    (bd, bn, bc) = (d, n, c);
                                }
                            }
                            out.push((i, bd, bn, bc));
                        }
                        out
                    })
                    .collect();
                for row in rows {
                    for (i, d, n, c) in row {
                        depth[i] = d;
                        normal[i] = n;
                        cost[i] = c;
                    }
                }
            }
        }
        // Keep the confident ones, not seen edge-on.
        let mut map = DepthMap { depth: vec![0.0; w * h], normal: vec![[0; 3]; w * h] };
        for i in 0..w * h {
            if cost[i] <= o.max_cost && depth[i] > 0.0 {
                let q = unit(self.cam.ray((i % w) as f64, (i / w) as f64));
                if dot(normal[i], q) < -0.2 {
                    map.depth[i] = depth[i] as f32;
                    map.normal[i] = pack(normal[i]);
                }
            }
        }
        Some(map)
    }
}

/// Window means and inverse standard deviations of a photo (NaN where it has no texture).
fn window_stats(p: &Photo, offsets: &[(i32, i32)], radius: i32) -> (Vec<f32>, Vec<f32>) {
    let (w, h) = (p.width, p.height);
    let stats: Vec<(f32, f32)> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            if x < radius || y < radius || x + radius >= w as i32 || y + radius >= h as i32 {
                return (0.0, f32::NAN);
            }
            let (mut s, mut ss) = (0.0f32, 0.0f32);
            for &(dx, dy) in offsets {
                let v = p.grey(((y + dy) as usize) * w + (x + dx) as usize);
                s += v;
                ss += v * v;
            }
            let n = offsets.len() as f32;
            let m = s / n;
            let var = (ss / n - m * m).max(0.0);
            // Below about 1 % grey: too flat to match.
            (m, if var > 1e-4 { 1.0 / var.sqrt() } else { f32::NAN })
        })
        .collect();
    stats.into_iter().unzip()
}

/// Dense points from photos with solved pinhole cameras (`cameras[i]` took
/// `photos[i]`; the photo may be smaller than the camera's size, the lens is
/// scaled to it), using the sparse points for each photo's depth range and
/// partners. `progress` gets 0…1; `cancel` stops it between steps.
pub fn densify(cameras: &[Camera], photos: &[Photo], sparse: &[V3], o: &Options, progress: &(dyn Fn(f64) + Sync), cancel: &(dyn Fn() -> bool + Sync)) -> Result<(Cloud, Report), String> {
    let t0 = std::time::Instant::now();
    if cameras.len() != photos.len() {
        return Err("each camera needs its photo".into());
    }
    if cameras.iter().any(|c| c.dist.iter().any(|v| v.abs() > 1e-12)) {
        return Err("dense matching needs undistorted photos (pinhole cameras)".into());
    }
    let n = cameras.len();
    let cams: Vec<Cam> = cameras.iter().zip(photos).map(|(c, p)| Cam::new(c, p)).collect();
    let partners = partners(&cams, sparse, o.sources.clamp(1, 16));
    let mut offsets = Vec::new();
    let mut dy = -o.radius;
    while dy <= o.radius {
        let mut dx = -o.radius;
        while dx <= o.radius {
            offsets.push((dx, dy));
            dx += o.step.max(1);
        }
        dy += o.step.max(1);
    }
    if offsets.len() > MAX_SAMPLES {
        return Err(format!("the matching window has {} samples; at most {MAX_SAMPLES} (a larger step or smaller radius)", offsets.len()));
    }
    let mut report = Report { photos: n, ..Default::default() };
    let mut maps: Vec<Option<DepthMap>> = Vec::with_capacity(n);
    for i in 0..n {
        if cancel() {
            return Err("cancelled".into());
        }
        // Depth range from the sparse points this photo sees.
        let mut depths: Vec<f64> = sparse
            .iter()
            .filter_map(|p| {
                let q = cams[i].to_cam(*p);
                cams[i].pixel(q).filter(|px| px[0] >= 0.0 && px[1] >= 0.0 && px[0] < cams[i].w as f64 && px[1] < cams[i].h as f64).map(|_| q[2])
            })
            .collect();
        if depths.len() < 10 || partners[i].is_empty() {
            report.skipped.push(format!("{}: {}", cameras[i].name, if partners[i].is_empty() { "no partner photo sees the same surface" } else { "too few sparse points in view" }));
            maps.push(None);
            progress((i + 1) as f64 / n as f64 * 0.9);
            continue;
        }
        depths.sort_by(|a, b| a.total_cmp(b));
        let q = |f: f64| depths[((depths.len() - 1) as f64 * f) as usize];
        let (near, far) = (q(0.01) * 0.75, q(0.99) * 1.25);
        let (_, inv_std) = window_stats(&photos[i], &offsets, o.radius);
        let matcher = Matcher {
            cam: cams[i],
            photo: &photos[i],
            pairs: partners[i].iter().map(|&j| (relative(&cams[i], &cams[j]), &photos[j])).collect(),
            offsets: offsets.clone(),
            inv_std,
            radius: o.radius,
            near,
            far,
        };
        let map = matcher.run(o, 0x5EED ^ (i as u64) << 32, cancel);
        if map.is_none() {
            return Err("cancelled".into());
        }
        report.matched += 1;
        report.depths += map.as_ref().map(|m| m.depth.iter().filter(|d| **d > 0.0).count()).unwrap_or(0);
        maps.push(map);
        progress((i + 1) as f64 / n as f64 * 0.9);
    }
    // Fusion: a pixel's point is kept where enough other photos' depths agree.
    let mut related: Vec<Vec<usize>> = partners.clone();
    for (i, ps) in partners.iter().enumerate() {
        for &j in ps {
            if !related[j].contains(&i) {
                related[j].push(i);
            }
        }
    }
    let step = o.fuse_step.max(1);
    let fused: Vec<(Vec<V3>, Vec<V3>, Vec<[u8; 3]>, f64)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (mut pos, mut nor, mut col, mut footprint) = (Vec::new(), Vec::new(), Vec::new(), 0.0);
            let Some(map) = &maps[i] else { return (pos, nor, col, footprint) };
            let cam = &cams[i];
            for y in (0..cam.h).step_by(step) {
                for x in (0..cam.w).step_by(step) {
                    let k = y * cam.w + x;
                    let d = map.depth[k] as f64;
                    if d <= 0.0 {
                        continue;
                    }
                    let q = cam.ray(x as f64, y as f64);
                    let world = cam.to_world([q[0] * d, q[1] * d, q[2] * d]);
                    let n_world = cam.rotate_to_world(unpack(map.normal[k]));
                    // Agreeing: the other photo's depth there matches, its point lands back on this pixel, its normal is alike.
                    let (mut agree, mut sum, mut n_sum) = (0, world, n_world);
                    for &j in &related[i] {
                        let Some(other) = &maps[j] else { continue };
                        let cj = &cams[j];
                        let qj = cj.to_cam(world);
                        let Some(px) = cj.pixel(qj) else { continue };
                        let (u, v) = (px[0].round(), px[1].round());
                        if u < 0.0 || v < 0.0 || u >= cj.w as f64 || v >= cj.h as f64 {
                            continue;
                        }
                        let kj = v as usize * cj.w + u as usize;
                        let dj = other.depth[kj] as f64;
                        if dj <= 0.0 || ((dj - qj[2]) / qj[2]).abs() > o.tolerance as f64 {
                            continue;
                        }
                        let rj = cj.ray(u, v);
                        let back = cj.to_world([rj[0] * dj, rj[1] * dj, rj[2] * dj]);
                        let Some(p) = cam.pixel(cam.to_cam(back)) else { continue };
                        if (p[0] - x as f64).powi(2) + (p[1] - y as f64).powi(2) > 1.5 * 1.5 {
                            continue;
                        }
                        let nj = cj.rotate_to_world(unpack(other.normal[kj]));
                        if dot(nj, n_world) < 0.85 {
                            continue;
                        }
                        agree += 1;
                        sum = [sum[0] + back[0], sum[1] + back[1], sum[2] + back[2]];
                        n_sum = [n_sum[0] + nj[0], n_sum[1] + nj[1], n_sum[2] + nj[2]];
                    }
                    if agree >= o.consistent {
                        let m = 1.0 / (agree + 1) as f64;
                        pos.push([sum[0] * m, sum[1] * m, sum[2] * m]);
                        nor.push(unit(n_sum));
                        col.push(photos[i].rgb[k]);
                        footprint += d / cam.fx * step as f64;
                    }
                }
            }
            (pos, nor, col, footprint)
        })
        .collect();
    let mut cloud = Cloud::default();
    let mut footprint = 0.0;
    for (p, nn, c, f) in fused {
        cloud.positions.extend(p);
        cloud.normals.extend(nn);
        cloud.colors.extend(c);
        footprint += f;
    }
    if cloud.is_empty() {
        return Err("dense matching found no surface several photos agree on: the photos need more texture or overlap".into());
    }
    // Points from several photos land on each other: one per footprint.
    let voxel = footprint / cloud.len() as f64;
    if voxel > 0.0 {
        cloud = crate::points::voxel_downsample(&cloud, voxel);
    }
    progress(1.0);
    report.points = cloud.len();
    report.seconds = t0.elapsed().as_secs_f64();
    Ok((cloud, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grey texture on the surface: smooth noise of the 3D point (so every photo sees the same pattern).
    fn texture(p: V3) -> f64 {
        let f = |a: f64, b: f64, c: f64| (p[0] * a + p[1] * b + p[2] * c).sin();
        0.5 + 0.12 * (f(0.9, 0.3, 0.5) + f(-0.4, 1.3, 0.7) + f(1.7, -1.1, 0.2) + f(0.2, 0.8, -1.9))
    }

    /// The scene: a sphere (radius 20, centre (0, 0, 25)) standing over a plane z = 0.
    fn hit(o: V3, d: V3) -> Option<V3> {
        let c = [0.0, 0.0, 25.0];
        let oc = [o[0] - c[0], o[1] - c[1], o[2] - c[2]];
        let b = dot(oc, d);
        let disc = b * b - (dot(oc, oc) - 400.0);
        let sphere = (disc > 0.0).then(|| -b - disc.sqrt()).filter(|t| *t > 0.0);
        let plane = (d[2].abs() > 1e-9).then(|| -o[2] / d[2]).filter(|t| *t > 0.0);
        let t = match (sphere, plane) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b)?,
        };
        Some([o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t])
    }

    /// A camera on a ring round the origin looking at (0, 0, 20).
    fn camera(azimuth: f64, elevation: f64, w: u32, h: u32) -> Camera {
        let (a, e) = (azimuth.to_radians(), elevation.to_radians());
        let c = [120.0 * e.cos() * a.cos(), 120.0 * e.cos() * a.sin(), 120.0 * e.sin() + 20.0];
        let z = unit([-c[0], -c[1], 20.0 - c[2]]);
        let x = unit([z[1], -z[0], 0.0]);
        let y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
        let r = [x, y, z];
        let t = [-dot(r[0], c), -dot(r[1], c), -dot(r[2], c)];
        Camera { name: format!("{azimuth}_{elevation}"), width: w, height: h, fx: 0.9 * w as f64, fy: 0.9 * w as f64, cx: w as f64 / 2.0, cy: h as f64 / 2.0, dist: [0.0; 4], rotation: r, translation: t }
    }

    fn render(c: &Camera) -> Photo {
        let (w, h) = (c.width as usize, c.height as usize);
        let mut rgb = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (o, d) = c.ray([x as f64 + 0.5, y as f64 + 0.5]);
                let g = hit(o, d).map(texture).unwrap_or(0.5);
                let v = (g.clamp(0.0, 1.0) * 255.0) as u8;
                rgb.push([v, v, v]);
            }
        }
        Photo::new(w, h, rgb, 10_000)
    }

    #[test]
    fn photos_round_a_sphere_give_points_on_it() {
        let mut cameras = Vec::new();
        for (ring, elevation) in [25.0, 50.0].into_iter().enumerate() {
            for k in 0..10 {
                cameras.push(camera(k as f64 * 36.0 + ring as f64 * 18.0, elevation, 200, 150));
            }
        }
        let photos: Vec<Photo> = cameras.iter().map(render).collect();
        // Sparse points: on the sphere and the floor round it.
        let mut sparse = Vec::new();
        for i in 0..400 {
            let (u, v) = (i as f64 * 0.37, i as f64 * 0.61);
            sparse.push([20.0 * u.cos() * v.sin(), 20.0 * u.sin() * v.sin(), 25.0 + 20.0 * v.cos()]);
            sparse.push([(i % 20) as f64 * 4.0 - 40.0, (i / 20) as f64 * 4.0 - 40.0, 0.0]);
        }
        let (cloud, report) = densify(&cameras, &photos, &sparse, &Options::default(), &|_| {}, &|| false).unwrap();
        assert_eq!(report.matched, 20, "{report:?}");
        assert!(cloud.len() > 3000, "{report:?}");
        // Every point on the sphere or the floor, and its normal along the surface's.
        let mut errors: Vec<f64> = Vec::new();
        let mut on_sphere = 0;
        for (p, n) in cloud.positions.iter().zip(&cloud.normals) {
            let r = ((p[0] * p[0] + p[1] * p[1] + (p[2] - 25.0).powi(2)).sqrt() - 20.0).abs();
            let (e, true_n) = if r < p[2].abs() { (r, unit([p[0], p[1], p[2] - 25.0])) } else { (p[2].abs(), [0.0, 0.0, 1.0]) };
            if r < p[2].abs() {
                on_sphere += 1;
            }
            errors.push(e);
            assert!(e > 1.0 || dot(*n, true_n) > 0.5, "normal {n:?} at {p:?}");
        }
        errors.sort_by(|a, b| a.total_cmp(b));
        let median = errors[errors.len() / 2];
        let p95 = errors[errors.len() * 95 / 100];
        assert!(median < 0.08 && p95 < 0.4, "median {median} p95 {p95} of {} points", errors.len());
        assert!(on_sphere > 1000, "{on_sphere} on the sphere");
    }

    #[test]
    fn photos_are_shrunk_by_whole_factors() {
        let p = Photo::new(10, 4, (0..40).map(|i| [i as u8, 0, 0]).collect(), 5);
        assert_eq!((p.width, p.height), (5, 2));
        // The first pixel is the mean of a 2 × 2 block: (0 + 1 + 10 + 11) / 4.
        assert_eq!(p.rgb[0][0], 5);
    }

    #[test]
    fn distorted_cameras_are_refused() {
        let mut c = camera(0.0, 30.0, 40, 30);
        c.dist[0] = 0.1;
        let p = render(&c);
        assert!(densify(&[c], &[p], &[], &Options::default(), &|_| {}, &|| false).unwrap_err().contains("undistorted"));
    }
}
