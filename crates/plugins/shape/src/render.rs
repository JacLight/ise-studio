//! Pictures of the model, drawn by the engine itself: what `./forma look`
//! shows the AI, so it can compare what it built against a reference
//! picture without a browser open.
//!
//! An orthographic camera fitted to the model, a depth-buffered rasteriser
//! at twice the output size (averaged down, so edges are smooth), soft
//! two-light shading with a little shine on metals, and dark outlines where
//! the depth jumps, the surface turns sharply or one part meets another, as
//! a technical illustration draws them.

/// One part to draw, in world millimetres.
pub struct Mesh {
    pub positions: Vec<f32>,
    /// Per-vertex normals (or empty: flat-shaded).
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    /// sRGB, 0–1.
    pub color: [f32; 3],
    pub metal: bool,
}

/// Where the camera looks from.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// From the camera towards the model.
    pub dir: [f64; 3],
    pub up: [f64; 3],
}

impl View {
    /// "iso" (front-right, from above), "front", "back", "left", "right",
    /// "top", "bottom", or "<azimuth>,<elevation>" in degrees (azimuth 0
    /// looks from the front, −Y; 90 from the right, +X; elevation up from level).
    pub fn parse(name: &str) -> Result<View, String> {
        let from = |az: f64, el: f64| {
            let (a, e) = (az.to_radians(), el.to_radians());
            // The camera's position direction; it looks the opposite way.
            let p = [a.sin() * e.cos(), -a.cos() * e.cos(), e.sin()];
            [-p[0], -p[1], -p[2]]
        };
        let (dir, up) = match name {
            "iso" | "" => (from(35.0, 30.0), [0.0, 0.0, 1.0]),
            "front" => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            "back" => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
            "left" => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            "right" => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            "top" => ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            "bottom" => ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            other => {
                let v: Vec<f64> = other.split(',').map(|s| s.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|_| format!("unknown view “{other}” (iso, front, back, left, right, top, bottom, or azimuth,elevation in degrees)"))?;
                if v.len() != 2 {
                    return Err(format!("unknown view “{other}”"));
                }
                let up = if v[1].abs() > 89.9 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, 1.0] };
                (from(v[0], v[1]), up)
            }
        };
        Ok(View { dir, up })
    }
}

/// What a picture shows: its pixels and how they map to the model.
pub struct Picture {
    pub png: Vec<u8>,
    /// Millimetres per output pixel.
    pub mm_per_px: f64,
    /// The model's bounds, world mm.
    pub bounds: [f64; 6],
    pub triangles: usize,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// The triangles of a mesh whose own box meets the box `f` (near enough,
/// and never dropping one that shows in it).
fn crop(m: &Mesh, f: [f64; 6]) -> Mesh {
    let p = |i: u32| {
        let i = i as usize * 3;
        [m.positions[i] as f64, m.positions[i + 1] as f64, m.positions[i + 2] as f64]
    };
    let mut indices = vec![];
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        if (0..3).all(|k| a[k].max(b[k]).max(c[k]) >= f[k] && a[k].min(b[k]).min(c[k]) <= f[k + 3]) {
            indices.extend_from_slice(t);
        }
    }
    Mesh { positions: m.positions.clone(), normals: m.normals.clone(), indices, color: m.color, metal: m.metal }
}

/// Draws the meshes as seen from `view`, `width × height` pixels.
pub fn render(meshes: &[Mesh], view: View, width: u32, height: u32) -> Result<Picture, String> {
    render_framed(meshes, view, width, height, None)
}

/// As `render`, framing only the box `[x0, y0, z0, x1, y1, z1]` (mm) when
/// given: a close look at a detail (a vernier's reading, a snap fit). What
/// lies wholly outside the box is left out, so nothing in front hides it.
pub fn render_framed(meshes: &[Mesh], view: View, width: u32, height: u32, frame: Option<[f64; 6]>) -> Result<Picture, String> {
    let cropped: Vec<Mesh>;
    let meshes = match frame {
        Some(f) => {
            cropped = meshes.iter().map(|m| crop(m, f)).filter(|m| !m.indices.is_empty()).collect();
            if cropped.is_empty() {
                return Err("nothing inside that box".into());
            }
            &cropped[..]
        }
        None => meshes,
    };
    const SS: usize = 2;
    let (w, h) = (width as usize * SS, height as usize * SS);
    if meshes.iter().all(|m| m.indices.is_empty()) {
        return Err("nothing to draw".into());
    }
    let d = unit(view.dir);
    let right = unit(cross(d, view.up));
    let up = cross(right, d);
    // Fit: the projected extent of every vertex, with a margin.
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut b = [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN];
    // Framing a box: its eight corners are what must fit.
    let corners: Option<Vec<f32>> = frame.map(|f| (0..8).flat_map(|i: usize| [f[if i & 1 == 0 { 0 } else { 3 }] as f32, f[if i & 2 == 0 { 1 } else { 4 }] as f32, f[if i & 4 == 0 { 2 } else { 5 }] as f32]).collect());
    let fit_points: Vec<&[f32]> = match &corners {
        Some(c) => vec![&c[..]],
        None => meshes.iter().map(|m| &m.positions[..]).collect(),
    };
    for pts in fit_points {
        for p in pts.chunks_exact(3) {
            let p = [p[0] as f64, p[1] as f64, p[2] as f64];
            let (x, y) = (dot(p, right), dot(p, up));
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
            for k in 0..3 {
                b[k] = b[k].min(p[k]);
                b[k + 3] = b[k + 3].max(p[k]);
            }
        }
    }
    let span = [(hi[0] - lo[0]).max(1e-6), (hi[1] - lo[1]).max(1e-6)];
    let scale = (w as f64 / (span[0] * 1.12)).min(h as f64 / (span[1] * 1.12));
    let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let to_screen = |p: [f64; 3]| -> [f64; 3] {
        [(dot(p, right) - centre[0]) * scale + w as f64 / 2.0, h as f64 / 2.0 - (dot(p, up) - centre[1]) * scale, dot(p, d)]
    };
    let mut depth = vec![f32::INFINITY; w * h];
    let mut normal = vec![[0.0f32; 3]; w * h];
    let mut part = vec![u32::MAX; w * h];
    let mut triangles = 0;
    for (mi, m) in meshes.iter().enumerate() {
        let vert = |i: u32| {
            let i = i as usize * 3;
            [m.positions[i] as f64, m.positions[i + 1] as f64, m.positions[i + 2] as f64]
        };
        let screen: Vec<[f64; 3]> = (0..m.positions.len() / 3).map(|i| to_screen(vert(i as u32))).collect();
        for t in m.indices.chunks_exact(3) {
            triangles += 1;
            let (a, bb, c) = (screen[t[0] as usize], screen[t[1] as usize], screen[t[2] as usize]);
            let area = (bb[0] - a[0]) * (c[1] - a[1]) - (bb[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-12 {
                continue;
            }
            let face = unit(cross(sub(vert(t[1]), vert(t[0])), sub(vert(t[2]), vert(t[0]))));
            let x0 = a[0].min(bb[0]).min(c[0]).floor().max(0.0) as usize;
            let x1 = (a[0].max(bb[0]).max(c[0]).ceil() as usize).min(w - 1);
            let y0 = a[1].min(bb[1]).min(c[1]).floor().max(0.0) as usize;
            let y1 = (a[1].max(bb[1]).max(c[1]).ceil() as usize).min(h - 1);
            if x0 > x1 || y0 > y1 {
                continue;
            }
            for y in y0..=y1 {
                let py = y as f64 + 0.5;
                for x in x0..=x1 {
                    let px = x as f64 + 0.5;
                    let w0 = ((bb[0] - px) * (c[1] - py) - (bb[1] - py) * (c[0] - px)) / area;
                    let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                        continue;
                    }
                    let z = (w0 * a[2] + w1 * bb[2] + w2 * c[2]) as f32;
                    let k = y * w + x;
                    if z < depth[k] {
                        depth[k] = z;
                        let n = if m.normals.len() == m.positions.len() {
                            let nv = |i: u32| {
                                let i = i as usize * 3;
                                [m.normals[i] as f64, m.normals[i + 1] as f64, m.normals[i + 2] as f64]
                            };
                            let (na, nb, nc) = (nv(t[0]), nv(t[1]), nv(t[2]));
                            unit([w0 * na[0] + w1 * nb[0] + w2 * nc[0], w0 * na[1] + w1 * nb[1] + w2 * nc[1], w0 * na[2] + w1 * nb[2] + w2 * nc[2]])
                        } else {
                            face
                        };
                        // Seen from behind (inside a cut part): face the camera.
                        let n = if dot(n, d) > 0.0 { [-n[0], -n[1], -n[2]] } else { n };
                        normal[k] = [n[0] as f32, n[1] as f32, n[2] as f32];
                        part[k] = mi as u32;
                    }
                }
            }
        }
    }
    // Shade: a key light from the upper left (raking enough to bring out
    // reliefs), a fill from the right, sky ambient; balanced, as the view's
    // lights are, so a face turned to the camera shows its material's own
    // colour and the sides step down from it.
    let key = unit([-0.55 * right[0] + 0.6 * up[0] - 0.6 * d[0], -0.55 * right[1] + 0.6 * up[1] - 0.6 * d[1], -0.55 * right[2] + 0.6 * up[2] - 0.6 * d[2]]);
    let fill = unit([0.6 * right[0] - 0.2 * up[0] - 0.75 * d[0], 0.6 * right[1] - 0.2 * up[1] - 0.75 * d[1], 0.6 * right[2] - 0.2 * up[2] - 0.75 * d[2]]);
    let half = unit([key[0] - d[0], key[1] - d[1], key[2] - d[2]]);
    let lin = |c: f32| c.powf(2.2);
    let srgb = |c: f32| c.clamp(0.0, 1.0).powf(1.0 / 2.2);
    let size = span[0].max(span[1]);
    let jump = (size * scale * 0.004).max(1.5) as f32 / scale as f32; // depth step that draws a line, mm
    let mut rgb = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            if part[k] == u32::MAX {
                // A soft studio backdrop, lighter at the top.
                let t = y as f32 / h as f32;
                let c = 0.86 - 0.1 * t;
                rgb[k] = [c, c + 0.008, c + 0.022];
                continue;
            }
            let m = &meshes[part[k] as usize];
            let n = normal[k];
            let n = [n[0] as f64, n[1] as f64, n[2] as f64];
            let diffuse = 0.85 * dot(n, key).max(0.0) + 0.25 * dot(n, fill).max(0.0) + 0.2 + 0.06 * dot(n, [0.0, 0.0, 1.0]).max(0.0);
            let spec = dot(n, half).max(0.0).powf(if m.metal { 24.0 } else { 48.0 }) * if m.metal { 0.45 } else { 0.12 };
            let base = m.color.map(lin);
            rgb[k] = [0, 1, 2].map(|i| base[i] * diffuse as f32 + spec as f32);
            rgb[k] = rgb[k].map(srgb);
        }
    }
    // Outlines: where the depth jumps, the surface folds, or parts meet.
    let edge = |k: usize, j: usize| -> bool {
        if part[k] != part[j] {
            return true;
        }
        if part[k] == u32::MAX {
            return false;
        }
        if (depth[k] - depth[j]).abs() > jump {
            return true;
        }
        let (a, b) = (normal[k], normal[j]);
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2] < 0.82
    };
    let mut lines = vec![0.0f32; w * h];
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            let k = y * w + x;
            if edge(k, k + 1) || edge(k, k + w) {
                lines[k] = 1.0;
            }
        }
    }
    // Down to the output size, lines drawn over the shading.
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let mut c = [0.0f32; 3];
            let mut ink = 0.0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let k = (y * SS + sy) * w + x * SS + sx;
                    for i in 0..3 {
                        c[i] += rgb[k][i];
                    }
                    ink += lines[k];
                }
            }
            let n = (SS * SS) as f32;
            let a = (ink / n * 0.85).min(0.85);
            let line = [0.16f32, 0.17, 0.19];
            for i in 0..3 {
                let v = c[i] / n * (1.0 - a) + line[i] * a;
                out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            out.push(255);
        }
    }
    Ok(Picture { png: crate::image::encode_png(width, height, &out), mm_per_px: SS as f64 / scale, bounds: b, triangles })
}

/// A camera for photo-like pictures: where it stands, what it looks at,
/// and its vertical field of view.
#[derive(Clone, Copy, Debug)]
pub struct PhotoCamera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    /// Vertical field of view, degrees.
    pub fov: f64,
}

impl PhotoCamera {
    /// Orbiting a point: azimuth (0 from the front, −Y; 90 from the right,
    /// +X), elevation up from level, distance (mm).
    pub fn orbit(centre: [f64; 3], azimuth: f64, elevation: f64, distance: f64, fov: f64) -> PhotoCamera {
        let (a, e) = (azimuth.to_radians(), elevation.to_radians());
        let p = [a.sin() * e.cos(), -a.cos() * e.cos(), e.sin()];
        PhotoCamera { eye: [centre[0] + p[0] * distance, centre[1] + p[1] * distance, centre[2] + p[2] * distance], target: centre, fov }
    }
}

/// What a photo-like picture shows besides the model.
#[derive(Clone, Debug)]
pub struct PhotoScene {
    /// A sprayed-speckle texture on the model (as parts are prepared for
    /// scanning), so photos of plain surfaces have features to match.
    pub speckle: bool,
    /// The printed marker sheet under the model, centred on it.
    pub sheet: Option<de_scan::markers::Sheet>,
    /// The key light's azimuth, degrees.
    pub light: f64,
}

/// A photo-like picture and the camera that took it (pinhole, no distortion).
pub struct Photo {
    pub png: Vec<u8>,
    /// Focal length in pixels, and the principal point.
    pub focal: f64,
    pub centre: [f64; 2],
    /// World to camera (x right, y down, z forward), rows; and the shift.
    pub rotation: [[f64; 3]; 3],
    pub translation: [f64; 3],
    pub bounds: [f64; 6],
}

fn hash3(x: i64, y: i64, z: i64, seed: u64) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ (y as u64).wrapping_mul(0xC2B2AE3D27D4EB4F) ^ (z as u64).wrapping_mul(0x165667B19E3779F9) ^ seed;
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise, 0…1.
fn noise(p: [f64; 3], seed: u64) -> f64 {
    let f = p.map(f64::floor);
    let t = [p[0] - f[0], p[1] - f[1], p[2] - f[2]].map(|v| v * v * (3.0 - 2.0 * v));
    let (x, y, z) = (f[0] as i64, f[1] as i64, f[2] as i64);
    let mut s = 0.0;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let w = (if dx == 1 { t[0] } else { 1.0 - t[0] }) * (if dy == 1 { t[1] } else { 1.0 - t[1] }) * (if dz == 1 { t[2] } else { 1.0 - t[2] });
                s += w * hash3(x + dx, y + dy, z + dz, seed);
            }
        }
    }
    s
}

/// A photo-like picture: a perspective camera, matte surfaces lit by fixed
/// lights, optionally speckled and standing on the marker sheet on a
/// wood-grain table, so photogrammetry can be tried on a model whose true
/// shape is known.
pub fn render_photo(meshes: &[Mesh], cam: PhotoCamera, width: u32, height: u32, scene: &PhotoScene) -> Result<Photo, String> {
    const SS: usize = 2;
    let (w, h) = (width as usize * SS, height as usize * SS);
    if meshes.iter().all(|m| m.indices.is_empty()) {
        return Err("nothing to draw".into());
    }
    let mut b = [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN];
    for m in meshes {
        for p in m.positions.chunks_exact(3) {
            for k in 0..3 {
                b[k] = b[k].min(p[k] as f64);
                b[k + 3] = b[k + 3].max(p[k] as f64);
            }
        }
    }
    let fwd = unit(sub(cam.target, cam.eye));
    let right = unit(cross(fwd, [0.0, 0.0, 1.0]));
    let right = if dot(right, right) < 0.5 { [1.0, 0.0, 0.0] } else { right };
    let down = cross(fwd, right);
    let focal = (h as f64 / 2.0) / (cam.fov.to_radians() / 2.0).tan();
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let to_cam = |p: [f64; 3]| {
        let d = sub(p, cam.eye);
        [dot(d, right), dot(d, down), dot(d, fwd)]
    };
    let near = 1.0;
    let mut depth = vec![f32::INFINITY; w * h];
    let mut world = vec![[0.0f32; 3]; w * h];
    let mut normal = vec![[0.0f32; 3]; w * h];
    let mut part = vec![u32::MAX; w * h];
    for (mi, m) in meshes.iter().enumerate() {
        let vert = |i: u32| {
            let i = i as usize * 3;
            [m.positions[i] as f64, m.positions[i + 1] as f64, m.positions[i + 2] as f64]
        };
        let cams: Vec<[f64; 3]> = (0..m.positions.len() / 3).map(|i| to_cam(vert(i as u32))).collect();
        for t in m.indices.chunks_exact(3) {
            let c3 = [cams[t[0] as usize], cams[t[1] as usize], cams[t[2] as usize]];
            if c3.iter().any(|c| c[2] < near) {
                continue;
            }
            let s = c3.map(|c| [cx + focal * c[0] / c[2], cy + focal * c[1] / c[2]]);
            let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[1][1] - s[0][1]) * (s[2][0] - s[0][0]);
            if area.abs() < 1e-12 {
                continue;
            }
            let wv = [vert(t[0]), vert(t[1]), vert(t[2])];
            let face = unit(cross(sub(wv[1], wv[0]), sub(wv[2], wv[0])));
            let x0 = s.iter().map(|p| p[0]).fold(f64::MAX, f64::min).floor().max(0.0) as usize;
            let x1 = (s.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil().max(0.0) as usize).min(w - 1);
            let y0 = s.iter().map(|p| p[1]).fold(f64::MAX, f64::min).floor().max(0.0) as usize;
            let y1 = (s.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil().max(0.0) as usize).min(h - 1);
            if x0 > x1 || y0 > y1 {
                continue;
            }
            let iz = c3.map(|c| 1.0 / c[2]);
            for y in y0..=y1 {
                let py = y as f64 + 0.5;
                for x in x0..=x1 {
                    let px = x as f64 + 0.5;
                    let l0 = ((s[1][0] - px) * (s[2][1] - py) - (s[1][1] - py) * (s[2][0] - px)) / area;
                    let l1 = ((s[2][0] - px) * (s[0][1] - py) - (s[2][1] - py) * (s[0][0] - px)) / area;
                    let l2 = 1.0 - l0 - l1;
                    if l0 < -1e-9 || l1 < -1e-9 || l2 < -1e-9 {
                        continue;
                    }
                    // Perspective-correct: weights over 1/z.
                    let q = [l0 * iz[0], l1 * iz[1], l2 * iz[2]];
                    let z = 1.0 / (q[0] + q[1] + q[2]);
                    let k = y * w + x;
                    if (z as f32) < depth[k] {
                        depth[k] = z as f32;
                        let wp = [0, 1, 2].map(|i| (q[0] * wv[0][i] + q[1] * wv[1][i] + q[2] * wv[2][i]) * z);
                        world[k] = wp.map(|v| v as f32);
                        let n = if dot(face, fwd) > 0.0 { [-face[0], -face[1], -face[2]] } else { face };
                        normal[k] = n.map(|v| v as f32);
                        part[k] = mi as u32;
                    }
                }
            }
        }
    }
    let lin = |c: f32| c.powf(2.2);
    let srgb = |c: f32| c.clamp(0.0, 1.0).powf(1.0 / 2.2);
    let la = scene.light.to_radians();
    let key = unit([la.sin() * 0.6, -la.cos() * 0.6, 0.8]);
    let fill = unit([-la.sin() * 0.5, la.cos() * 0.5, 0.6]);
    let ground = b[2];
    let centre = [(b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0];
    let speckle = |p: [f64; 3]| -> f32 {
        let fine = noise([p[0] / 0.9, p[1] / 0.9, p[2] / 0.9], 7);
        let coarse = noise([p[0] / 3.5, p[1] / 3.5, p[2] / 3.5], 11);
        let dots = if fine > 0.62 { 0.25 } else { 1.0 };
        (0.45 + 0.55 * coarse) as f32 * dots
    };
    let mut rgb = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            // The ray through this pixel, for the floor.
            let d = unit([0, 1, 2].map(|i| right[i] * (x as f64 + 0.5 - cx) / focal + down[i] * (y as f64 + 0.5 - cy) / focal + fwd[i]));
            let t_floor = if d[2] < -1e-9 { (ground - cam.eye[2]) / d[2] } else { f64::INFINITY };
            if part[k] == u32::MAX && !t_floor.is_finite() {
                let c = 0.8 - 0.15 * (y as f32 / h as f32);
                rgb[k] = [c, c, c + 0.02];
                continue;
            }
            let floor_depth = t_floor * dot(d, fwd);
            if part[k] == u32::MAX || (floor_depth as f32) < depth[k] - 1e-3 {
                let p = [cam.eye[0] + d[0] * t_floor, cam.eye[1] + d[1] * t_floor, ground];
                let (sx, sy) = (p[0] - centre[0], p[1] - centre[1]);
                let base = match &scene.sheet {
                    Some(sheet) if sx.abs() < 148.5 && sy.abs() < 105.0 => {
                        if sheet.ink([sx, sy]) {
                            [0.04, 0.04, 0.045]
                        } else {
                            [0.88, 0.88, 0.86]
                        }
                    }
                    _ => {
                        // A wood-grain table.
                        let g = (noise([p[0] / 40.0, p[1] / 3.0, 0.0], 3) * 0.6 + noise([p[0] / 6.0, p[1] / 1.2, 1.0], 5) * 0.4) as f32;
                        [0.45 + 0.25 * g, 0.32 + 0.18 * g, 0.2 + 0.1 * g]
                    }
                };
                let fade = (1.0 / (1.0 + (sx * sx + sy * sy).sqrt() / 900.0)) as f32;
                rgb[k] = base.map(|c| srgb(lin(c) * (0.55 + 0.4 * key[2] as f32) * fade));
                continue;
            }
            let m = &meshes[part[k] as usize];
            let n = normal[k].map(|v| v as f64);
            let diffuse = 0.75 * dot(n, key).max(0.0) + 0.25 * dot(n, fill).max(0.0) + 0.18;
            let tex = if scene.speckle { speckle(world[k].map(|v| v as f64)) } else { 1.0 };
            rgb[k] = m.color.map(|c| srgb(lin(c) * tex * diffuse as f32));
        }
    }
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let mut c = [0.0f32; 3];
            for sy in 0..SS {
                for sx in 0..SS {
                    let k = (y * SS + sy) * w + x * SS + sx;
                    for i in 0..3 {
                        c[i] += rgb[k][i];
                    }
                }
            }
            for v in c {
                out.push((v / (SS * SS) as f32 * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            out.push(255);
        }
    }
    let rotation = [right, down, fwd];
    let translation = [-dot(right, cam.eye), -dot(down, cam.eye), -dot(fwd, cam.eye)];
    Ok(Photo { png: crate::image::encode_png(width, height, &out), focal: focal / SS as f64, centre: [width as f64 / 2.0, height as f64 / 2.0], rotation, translation, bounds: b })
}

/// A material's colour by its name, as the view paints it: a colour word in
/// the name wins ("PLA (Bambu, black)"), else the family's own.
pub fn material_color(name: Option<&str>) -> ([f32; 3], bool) {
    const FAMILIES: &[(&str, u32, bool)] = &[
        ("stainless steel", 0xd0d5d9, true),
        ("galvanised steel", 0xb8c0c6, true),
        ("painted steel", 0x34373d, false),
        ("steel", 0xa2a9ae, true),
        ("aluminium", 0xc8d2da, true),
        ("aluminum", 0xc8d2da, true),
        ("brass", 0xc9a85c, true),
        ("copper", 0xc27c56, true),
        ("titanium", 0x9aa2a8, true),
        ("pla", 0x4f7dd9, false),
        ("petg", 0xe86a3a, false),
        ("abs", 0xe7e2d8, false),
        ("nylon", 0xece9e1, false),
        ("acrylic", 0xdfeef5, false),
        ("rubber", 0x25272b, false),
        ("birch ply", 0xe3cb9f, false),
        ("plywood", 0xe3cb9f, false),
        ("walnut", 0x6f4a2f, false),
        ("oak", 0xc69c6d, false),
        ("mdf", 0xb99b77, false),
        ("concrete", 0xb3afa6, false),
        ("glass", 0x8fb9cc, false),
        ("fr4", 0x2f7a4a, false),
        ("gypsum", 0xf4f3ef, false),
        ("brick", 0x985a45, false),
        ("roof tile", 0xa4523a, false),
        ("clay tile", 0xa4523a, false),
        ("timber", 0xc19467, false),
        ("stone", 0xb5aa95, false),
        ("granite", 0x8e8a86, false),
        ("marble", 0xe9e6e0, false),
        ("slate", 0x50565d, false),
        ("plaster", 0xebe6dc, false),
        ("asphalt", 0x3e3f42, false),
        ("block", 0xb2afa7, false),
    ];
    const WORDS: &[(&str, u32)] = &[
        ("dark grey", 0x4a4d52),
        ("dark gray", 0x4a4d52),
        ("black", 0x232427),
        ("charcoal", 0x3a3c40),
        ("grey", 0x8d9196),
        ("gray", 0x8d9196),
        ("silver", 0xc3c7cb),
        ("white", 0xf1f0ec),
        ("ivory", 0xeee8d8),
        ("natural", 0xe8e1cf),
        ("beige", 0xd9c9a8),
        ("red", 0xc8372f),
        ("orange", 0xe8742c),
        ("yellow", 0xf0c430),
        ("gold", 0xc9a24a),
        ("green", 0x3e9a54),
        ("teal", 0x2a8c8c),
        ("cyan", 0x3fb3cf),
        ("blue", 0x3a6fd0),
        ("navy", 0x243a66),
        ("purple", 0x7a4fc4),
        ("pink", 0xe27aa8),
        ("brown", 0x7a5236),
    ];
    let hex = |v: u32| [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0];
    let Some(name) = name else { return (hex(0xd6d4c7), false) };
    let key = name.to_lowercase();
    let (mut color, metal) = FAMILIES.iter().find(|(f, _, _)| key.contains(f)).map(|(_, c, m)| (hex(*c), *m)).unwrap_or((hex(0xc4c9d4), false));
    let words: Vec<&str> = key.split(|c: char| !c.is_alphanumeric()).collect();
    if let Some((_, c)) = WORDS.iter().find(|(wd, _)| if wd.contains(' ') { key.contains(wd) } else { words.contains(wd) }) {
        color = hex(*c);
    }
    (color, metal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_draws_in_the_middle_with_outlines() {
        let cube = manifold_csg::Manifold::cube(40.0, 20.0, 10.0, true);
        let (positions, normals, indices) = crate::runtime::mesh_with_normals(&cube);
        let m = Mesh { positions, normals, indices, color: [0.3, 0.5, 0.85], metal: false };
        let p = render(&[m], View::parse("iso").unwrap(), 400, 300).unwrap();
        let img = crate::image::decode(&p.png).unwrap();
        assert_eq!((img.width, img.height), (400, 300));
        let b = img.channel(crate::image::Channel::Brightness);
        // The corner is backdrop, the middle is the box (darker than the backdrop).
        assert!(b[0] > 0.85 && b[150 * 400 + 200] < 0.8, "{} {}", b[0], b[150 * 400 + 200]);
        assert!((p.bounds[3] - 20.0).abs() < 1e-6 && p.mm_per_px > 0.0);
        assert!(View::parse("30,20").is_ok() && View::parse("sideways").is_err());
    }

    #[test]
    fn a_photo_sees_the_marker_sheet_where_the_camera_says() {
        let cube = manifold_csg::Manifold::cube(40.0, 40.0, 30.0, true);
        let (positions, normals, indices) = crate::runtime::mesh_with_normals(&cube);
        let m = Mesh { positions, normals, indices, color: [0.8, 0.8, 0.8], metal: false };
        let sheet = de_scan::markers::Sheet::default();
        let scene = PhotoScene { speckle: true, sheet: Some(sheet.clone()), light: 30.0 };
        let cam = PhotoCamera::orbit([0.0, 0.0, 0.0], 20.0, 50.0, 420.0, 45.0);
        let p = render_photo(&[m], cam, 800, 600, &scene).unwrap();
        let img = crate::image::decode(&p.png).unwrap();
        let g = img.channel(crate::image::Channel::Brightness);
        let found = de_scan::markers::detect(&g, 800, 600);
        assert!(found.len() >= 4, "{found:?}");
        // Each found corner is where the camera projects the printed one (the floor is at z = −15).
        for d in &found {
            for (j, c) in sheet.corners(d.id).unwrap().iter().enumerate() {
                let wp = [c[0], c[1], -15.0];
                let r = &p.rotation;
                let x = [0, 1, 2].map(|i| r[i][0] * wp[0] + r[i][1] * wp[1] + r[i][2] * wp[2] + p.translation[i]);
                let px = [p.focal * x[0] / x[2] + p.centre[0], p.focal * x[1] / x[2] + p.centre[1]];
                let e = ((px[0] - d.corners[j][0]).powi(2) + (px[1] - d.corners[j][1]).powi(2)).sqrt();
                assert!(e < 1.0, "marker {} corner {j} off by {e:.2} px", d.id);
            }
        }
    }

    #[test]
    fn colour_words_win_over_the_family() {
        assert_eq!(material_color(Some("PLA (Bambu, black)")).0, material_color(Some("black")).0);
        assert!(material_color(Some("Stainless steel 304")).1);
    }
}
