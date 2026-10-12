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

/// Draws the meshes as seen from `view`, `width × height` pixels.
pub fn render(meshes: &[Mesh], view: View, width: u32, height: u32) -> Result<Picture, String> {
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
    for m in meshes {
        for p in m.positions.chunks_exact(3) {
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
    // Shade: a raking key light from the upper left (it brings out reliefs and
    // engraving), a fill from the right, sky ambient.
    let key = unit([-0.62 * right[0] + 0.66 * up[0] - 0.42 * d[0], -0.62 * right[1] + 0.66 * up[1] - 0.42 * d[1], -0.62 * right[2] + 0.66 * up[2] - 0.42 * d[2]]);
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
            let diffuse = 0.78 * dot(n, key).max(0.0) + 0.2 * dot(n, fill).max(0.0) + 0.14 + 0.06 * dot(n, [0.0, 0.0, 1.0]).max(0.0);
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

/// A material's colour by its name, as the view paints it: a colour word in
/// the name wins ("PLA (Bambu, black)"), else the family's own.
pub fn material_color(name: Option<&str>) -> ([f32; 3], bool) {
    const FAMILIES: &[(&str, u32, bool)] = &[
        ("stainless steel", 0xd0d5d9, true),
        ("galvanised steel", 0xb8c0c6, true),
        ("painted steel", 0x34373d, false),
        ("steel", 0xa9b6bf, true),
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
        ("concrete", 0xc3bfb6, false),
        ("glass", 0x8fb9cc, false),
        ("fr4", 0x2f7a4a, false),
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
    fn colour_words_win_over_the_family() {
        assert_eq!(material_color(Some("PLA (Bambu, black)")).0, material_color(Some("black")).0);
        assert!(material_color(Some("Stainless steel 304")).1);
    }
}
