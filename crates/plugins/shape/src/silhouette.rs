//! Comparing the model with a reference picture: the model drawn square to a
//! canvas (orthographic, from the side its picture is seen from) into the
//! picture's own pixels, its silhouette set against the object's in the
//! photo. Reports how well they overlap (intersection over union, the
//! measure silhouette fitting uses), where they differ (each region of extra
//! or missing material, how big and how thick, in the model's millimetres)
//! and draws the overlay: the photo, the model over it, extra material red,
//! missing material blue.

use crate::canvas::{Canvas, apply};
use crate::image::Picture;
use crate::render::Mesh;
use serde::Serialize;

/// Where the model and the photo disagree.
#[derive(Clone, Debug, Serialize)]
pub struct Region {
    /// "extra": the model has material the photo does not show; "missing":
    /// the photo shows the object where the model has none.
    pub kind: &'static str,
    pub area_mm2: f64,
    /// How thick the disagreement is at its thickest (mm): how far an edge
    /// has to move.
    pub depth_mm: f64,
    /// Its middle, in the canvas plane (mm) and in the world.
    pub plane: [f64; 2],
    pub at: [f64; 3],
    /// Its extent in the canvas plane [min x, min y, max x, max y] (mm).
    pub bounds: [f64; 4],
}

#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    pub canvas: String,
    pub view: Option<String>,
    /// Intersection over union of the two silhouettes (1 is a perfect match).
    pub iou: f64,
    /// Share of the model's silhouette the photo also covers.
    pub precision: f64,
    /// Share of the photo's silhouette the model also covers.
    pub recall: f64,
    /// The thickest disagreement (mm).
    pub worst_mm: f64,
    /// Mean distance between the two outlines (mm).
    pub mean_mm: f64,
    pub photo_area_mm2: f64,
    pub model_area_mm2: f64,
    /// Overall size of each silhouette in the plane (mm), [width, height].
    pub photo_size_mm: [f64; 2],
    pub model_size_mm: [f64; 2],
    pub mm_per_px: f64,
    /// Largest first.
    pub regions: Vec<Region>,
    #[serde(skip)]
    pub png: Vec<u8>,
    pub size: [usize; 2],
}

/// Options for [`compare`].
#[derive(Clone, Debug)]
pub struct Options {
    /// Longest side of the working grid in pixels.
    pub max_edge: usize,
    /// Draw the overlay picture.
    pub picture: bool,
    /// Regions smaller than this many mm² are not listed.
    pub min_region_mm2: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_edge: 1000, picture: true, min_region_mm2: 4.0 }
    }
}

struct Grid {
    w: usize,
    h: usize,
    /// Working pixel (i, j) covers picture pixels from x0 + i·k to x0 + (i+1)·k.
    x0: f64,
    y0: f64,
    k: f64,
}

impl Grid {
    fn centre(&self, i: usize, j: usize) -> [f64; 2] {
        [self.x0 + (i as f64 + 0.5) * self.k, self.y0 + (j as f64 + 0.5) * self.k]
    }
}

/// The photo's object as a mask on the working grid: what stands out from
/// the backdrop, small specks dropped and small enclosed holes (lettering,
/// highlights) filled.
fn photo_mask(ink: &crate::image::Field, g: &Grid) -> Vec<bool> {
    let mut m = vec![false; g.w * g.h];
    let k = g.k;
    for j in 0..g.h {
        for i in 0..g.w {
            // Average the picture pixels this cell covers.
            let (px0, py0) = (g.x0 + i as f64 * k, g.y0 + j as f64 * k);
            let (ax, ay) = (px0.floor().max(0.0) as usize, py0.floor().max(0.0) as usize);
            let (bx, by) = (((px0 + k).ceil() as usize).min(ink.w).max(ax + 1), ((py0 + k).ceil() as usize).min(ink.h).max(ay + 1));
            let (mut s, mut n) = (0.0f32, 0.0f32);
            for y in ay..by.min(ink.h) {
                for x in ax..bx {
                    s += ink.v[y * ink.w + x];
                    n += 1.0;
                }
            }
            m[j * g.w + i] = n > 0.0 && s / n > 0.5;
        }
    }
    let total = m.iter().filter(|v| **v).count();
    // Specks: pieces under 0.5 % of the object.
    let (labels, sizes) = label(&m, g.w, g.h, true);
    for (p, l) in labels.iter().enumerate() {
        if *l != usize::MAX && (sizes[*l] as f64) < 0.005 * total as f64 {
            m[p] = false;
        }
    }
    // Holes: background not reaching the edge and under 0.3 % of the object.
    let bg: Vec<bool> = m.iter().map(|v| !v).collect();
    let (labels, sizes) = label(&bg, g.w, g.h, true);
    let mut edge = vec![false; sizes.len()];
    for i in 0..g.w {
        for j in [0, g.h - 1] {
            if let Some(l) = labels.get(j * g.w + i).filter(|l| **l != usize::MAX) {
                edge[*l] = true;
            }
        }
    }
    for j in 0..g.h {
        for i in [0, g.w - 1] {
            if let Some(l) = labels.get(j * g.w + i).filter(|l| **l != usize::MAX) {
                edge[*l] = true;
            }
        }
    }
    for (p, l) in labels.iter().enumerate() {
        if *l != usize::MAX && !edge[*l] && (sizes[*l] as f64) < 0.003 * total as f64 {
            m[p] = true;
        }
    }
    m
}

/// Connected pieces (4-neighbours) of the set pixels: each pixel's label
/// (usize::MAX when not set) and each label's size.
fn label(m: &[bool], w: usize, h: usize, _four: bool) -> (Vec<usize>, Vec<usize>) {
    let mut labels = vec![usize::MAX; m.len()];
    let mut sizes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..m.len() {
        if !m[start] || labels[start] != usize::MAX {
            continue;
        }
        let id = sizes.len();
        let mut n = 0;
        labels[start] = id;
        stack.push(start);
        while let Some(p) = stack.pop() {
            n += 1;
            let (x, y) = (p % w, p / w);
            let mut visit = |q: usize| {
                if m[q] && labels[q] == usize::MAX {
                    labels[q] = id;
                    stack.push(q);
                }
            };
            if x > 0 {
                visit(p - 1);
            }
            if x + 1 < w {
                visit(p + 1);
            }
            if y > 0 {
                visit(p - w);
            }
            if y + 1 < h {
                visit(p + w);
            }
        }
        sizes.push(n);
    }
    (labels, sizes)
}

/// Squared Euclidean distance (in grid pixels) from each pixel to the
/// nearest pixel where `feature` is set (Felzenszwalb & Huttenlocher's
/// exact transform, one dimension at a time).
pub fn distance_squared(feature: &[bool], w: usize, h: usize) -> Vec<f64> {
    const INF: f64 = 1e20;
    let mut d: Vec<f64> = feature.iter().map(|f| if *f { 0.0 } else { INF }).collect();
    let n = w.max(h);
    let (mut f, mut out, mut v, mut z) = (vec![0.0; n], vec![0.0; n], vec![0usize; n], vec![0.0; n + 1]);
    let mut pass = |d: &mut Vec<f64>, len: usize, at: &dyn Fn(usize) -> usize| {
        for q in 0..len {
            f[q] = d[at(q)];
        }
        // The lower envelope of the parabolas rooted at each sample.
        let mut k = 0usize;
        v[0] = 0;
        z[0] = f64::NEG_INFINITY;
        z[1] = f64::INFINITY;
        for q in 1..len {
            let mut s = ((f[q] + (q * q) as f64) - (f[v[k]] + (v[k] * v[k]) as f64)) / (2.0 * (q - v[k]) as f64);
            while s <= z[k] {
                k -= 1;
                s = ((f[q] + (q * q) as f64) - (f[v[k]] + (v[k] * v[k]) as f64)) / (2.0 * (q - v[k]) as f64);
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
        }
        k = 0;
        for q in 0..len {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            out[q] = (q as f64 - v[k] as f64).powi(2) + f[v[k]];
        }
        for q in 0..len {
            d[at(q)] = out[q].min(INF);
        }
    };
    for x in 0..w {
        pass(&mut d, h, &|q| q * w + x);
    }
    for y in 0..h {
        pass(&mut d, w, &|q| y * w + q);
    }
    d
}

/// Draws the meshes square to the canvas into its working grid: the
/// silhouette, and for the overlay each pixel's shade and colour (nearest
/// surface to the camera, which stands on the plane's normal side).
fn model_mask(c: &Canvas, meshes: &[Mesh], g: &Grid) -> (Vec<bool>, Vec<[f32; 3]>) {
    let to_px = c.to_pixel();
    let n = c.plane.normal();
    let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let n = n.map(|v| v / nl);
    let mut mask = vec![false; g.w * g.h];
    let mut depth = vec![f64::MIN; g.w * g.h];
    let mut colour = vec![[0.0f32; 3]; g.w * g.h];
    for m in meshes {
        let pts: Vec<[f64; 3]> = m.positions.chunks_exact(3).map(|p| {
            let w = [p[0] as f64, p[1] as f64, p[2] as f64];
            let local = c.plane.to_local(w);
            let px = apply(&to_px, local);
            let d = (w[0] - c.plane.origin[0]) * n[0] + (w[1] - c.plane.origin[1]) * n[1] + (w[2] - c.plane.origin[2]) * n[2];
            [(px[0] - g.x0) / g.k, (px[1] - g.y0) / g.k, d]
        }).collect();
        let world = |i: usize| [m.positions[3 * i] as f64, m.positions[3 * i + 1] as f64, m.positions[3 * i + 2] as f64];
        for t in m.indices.chunks_exact(3) {
            let (a, b, cc) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
            let area = (b[0] - a[0]) * (cc[1] - a[1]) - (b[1] - a[1]) * (cc[0] - a[0]);
            if area.abs() < 1e-12 {
                continue;
            }
            // Face shade: how square the face is to the line of sight.
            let (wa, wb, wc) = (world(t[0] as usize), world(t[1] as usize), world(t[2] as usize));
            let u = [wb[0] - wa[0], wb[1] - wa[1], wb[2] - wa[2]];
            let v = [wc[0] - wa[0], wc[1] - wa[1], wc[2] - wa[2]];
            let fnm = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let fl = (fnm[0] * fnm[0] + fnm[1] * fnm[1] + fnm[2] * fnm[2]).sqrt().max(1e-12);
            let lit = ((fnm[0] * n[0] + fnm[1] * n[1] + fnm[2] * n[2]) / fl).abs() as f32;
            let shade = 0.45 + 0.55 * lit;
            let col = [m.color[0] * shade, m.color[1] * shade, m.color[2] * shade];
            let (lx, hx) = (a[0].min(b[0]).min(cc[0]).floor().max(0.0) as isize, a[0].max(b[0]).max(cc[0]).ceil().min(g.w as f64) as isize);
            let (ly, hy) = (a[1].min(b[1]).min(cc[1]).floor().max(0.0) as isize, a[1].max(b[1]).max(cc[1]).ceil().min(g.h as f64) as isize);
            for y in ly..hy {
                for x in lx..hx {
                    let p = [x as f64 + 0.5, y as f64 + 0.5];
                    let e0 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                    let e1 = (cc[0] - b[0]) * (p[1] - b[1]) - (cc[1] - b[1]) * (p[0] - b[0]);
                    let e2 = (a[0] - cc[0]) * (p[1] - cc[1]) - (a[1] - cc[1]) * (p[0] - cc[0]);
                    let inside = if area > 0.0 { e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0 } else { e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0 };
                    if !inside {
                        continue;
                    }
                    let q = y as usize * g.w + x as usize;
                    mask[q] = true;
                    let (l1, l2) = (e1 / area, e2 / area);
                    let z = l1 * a[2] + l2 * b[2] + (1.0 - l1 - l2) * cc[2];
                    if z > depth[q] {
                        depth[q] = z;
                        colour[q] = col;
                    }
                }
            }
        }
    }
    (mask, colour)
}

/// Outline pixels of a mask: set, with an unset 4-neighbour (or on the edge).
fn outline(m: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut o = vec![false; m.len()];
    for y in 0..h {
        for x in 0..w {
            let p = y * w + x;
            if m[p] && (x == 0 || y == 0 || x + 1 == w || y + 1 == h || !m[p - 1] || !m[p + 1] || !m[p - w] || !m[p + w]) {
                o[p] = true;
            }
        }
    }
    o
}

/// Compares the model (`meshes`, world mm) with the canvas's photo.
pub fn compare(c: &Canvas, meshes: &[Mesh], o: &Options) -> Result<Comparison, String> {
    let picture = crate::image::load(&c.image)?;
    let ink = crate::canvas::object_ink(c, &picture);
    // Work on the part of the picture where the object or the model is (with
    // a margin), at as much of the picture's own resolution as allowed.
    let [rx0, ry0, rx1, ry1] = c.region();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for y in 0..ink.h {
        for x in 0..ink.w {
            if ink.v[y * ink.w + x] > 0.5 {
                lo = [lo[0].min(x as f64), lo[1].min(y as f64)];
                hi = [hi[0].max(x as f64 + 1.0), hi[1].max(y as f64 + 1.0)];
            }
        }
    }
    let to_px = c.to_pixel();
    for m in meshes {
        for p in m.positions.chunks_exact(3) {
            let q = apply(&to_px, c.plane.to_local([p[0] as f64, p[1] as f64, p[2] as f64]));
            if q[0].is_finite() && q[1].is_finite() {
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
        }
    }
    let (x0, y0, x1, y1) = if lo[0] < hi[0] {
        let pad = 0.06 * (hi[0] - lo[0]).max(hi[1] - lo[1]);
        ((lo[0] - pad).max(rx0), (lo[1] - pad).max(ry0), (hi[0] + pad).min(rx1), (hi[1] + pad).min(ry1))
    } else {
        (rx0, ry0, rx1, ry1)
    };
    let (x0, y0, x1, y1) = if x1 - x0 < 4.0 || y1 - y0 < 4.0 { (rx0, ry0, rx1, ry1) } else { (x0, y0, x1, y1) };
    let long = (x1 - x0).max(y1 - y0);
    let k = (long / o.max_edge as f64).max(1.0);
    let g = Grid { w: (((x1 - x0) / k).floor() as usize).max(2), h: (((y1 - y0) / k).floor() as usize).max(2), x0, y0, k };
    let photo = photo_mask(&ink, &g);
    if photo.iter().filter(|v| **v).count() < 16 {
        return Err(format!("canvas “{}”: no object stands out from the photo's backdrop (crop to it, or set its background or threshold)", c.name));
    }
    let (model, colour) = model_mask(c, meshes, &g);
    let (mut inter, mut uni, mut pa, mut ma) = (0usize, 0usize, 0usize, 0usize);
    for i in 0..photo.len() {
        let (p, m) = (photo[i], model[i]);
        inter += (p && m) as usize;
        uni += (p || m) as usize;
        pa += p as usize;
        ma += m as usize;
    }
    let centre = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
    let mm = c.mm_per_px_at(centre) * k;
    let px_area = mm * mm;
    // Outline distances, both ways.
    let (po, mo) = (outline(&photo, g.w, g.h), outline(&model, g.w, g.h));
    let dm = distance_squared(&mo, g.w, g.h);
    let dp = distance_squared(&po, g.w, g.h);
    let (mut sum, mut cnt) = (0.0, 0usize);
    if ma > 0 {
        for i in 0..po.len() {
            if po[i] {
                sum += dm[i].sqrt();
                cnt += 1;
            }
            if mo[i] {
                sum += dp[i].sqrt();
                cnt += 1;
            }
        }
    }
    let mean_mm = if cnt > 0 { sum / cnt as f64 * mm } else { f64::NAN };
    // Regions of disagreement.
    let mut regions = Vec::new();
    let mut tinted = vec![0u8; photo.len()];
    for (kind, set) in [("extra", model.iter().zip(&photo).map(|(m, p)| *m && !*p).collect::<Vec<bool>>()), ("missing", model.iter().zip(&photo).map(|(m, p)| !*m && *p).collect::<Vec<bool>>())] {
        let inside: Vec<bool> = set.iter().map(|v| !v).collect();
        let depth = distance_squared(&inside, g.w, g.h);
        let (labels, sizes) = label(&set, g.w, g.h, true);
        let mut acc: Vec<(f64, f64, f64, [f64; 4])> = vec![(0.0, 0.0, 0.0, [f64::MAX, f64::MAX, f64::MIN, f64::MIN]); sizes.len()];
        let m = c.to_plane();
        for (p, l) in labels.iter().enumerate() {
            if *l == usize::MAX {
                continue;
            }
            tinted[p] = if kind == "extra" { 1 } else { 2 };
            let (i, j) = (p % g.w, p / g.w);
            let q = apply(&m, g.centre(i, j));
            let a = &mut acc[*l];
            a.0 += q[0];
            a.1 += q[1];
            a.2 = a.2.max(depth[p]);
            a.3 = [a.3[0].min(q[0]), a.3[1].min(q[1]), a.3[2].max(q[0]), a.3[3].max(q[1])];
        }
        for (l, n) in sizes.iter().enumerate() {
            let area = *n as f64 * px_area;
            if area < o.min_region_mm2 {
                continue;
            }
            let (sx, sy, d2, b) = acc[l];
            let plane = [sx / *n as f64, sy / *n as f64];
            // The thickest point is the largest circle inside: twice its radius.
            let depth_mm = ((d2.sqrt() * 2.0 - 1.0).max(1.0)) * mm;
            regions.push(Region { kind, area_mm2: area, depth_mm, plane, at: c.plane.to_world(plane), bounds: b });
        }
    }
    regions.sort_by(|a, b| (b.area_mm2 * b.depth_mm).total_cmp(&(a.area_mm2 * a.depth_mm)));
    let worst_mm = regions.iter().map(|r| r.depth_mm).fold(0.0, f64::max);
    regions.truncate(12);
    let extent = |set: &[bool]| -> [f64; 2] {
        let m = c.to_plane();
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for (p, v) in set.iter().enumerate() {
            if *v {
                let q = apply(&m, g.centre(p % g.w, p / g.w));
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
        }
        if lo[0] > hi[0] { [0.0, 0.0] } else { [hi[0] - lo[0] + mm, hi[1] - lo[1] + mm] }
    };
    let png = if o.picture { overlay(&picture, &g, &photo, &model, &colour, &tinted, &po, &mo, &regions, c) } else { Vec::new() };
    Ok(Comparison {
        canvas: c.name.clone(),
        view: c.view.clone(),
        iou: if uni > 0 { inter as f64 / uni as f64 } else { 0.0 },
        precision: if ma > 0 { inter as f64 / ma as f64 } else { 0.0 },
        recall: if pa > 0 { inter as f64 / pa as f64 } else { 0.0 },
        worst_mm,
        mean_mm,
        photo_area_mm2: pa as f64 * px_area,
        model_area_mm2: ma as f64 * px_area,
        photo_size_mm: extent(&photo),
        model_size_mm: extent(&model),
        mm_per_px: mm,
        regions,
        png,
        size: [g.w, g.h],
    })
}

#[allow(clippy::too_many_arguments)]
fn overlay(picture: &Picture, g: &Grid, _photo: &[bool], model: &[bool], colour: &[[f32; 3]], tinted: &[u8], po: &[bool], mo: &[bool], regions: &[Region], c: &Canvas) -> Vec<u8> {
    // Outlines two pixels wide on a large picture, so they read.
    let widen = |o: &[bool]| -> Vec<bool> {
        if g.w.max(g.h) < 500 {
            return o.to_vec();
        }
        (0..o.len()).map(|p| o[p] || (p % g.w + 1 < g.w && o[p + 1]) || (p + g.w < o.len() && o[p + g.w])).collect()
    };
    let (po, mo) = (&widen(po)[..], &widen(mo)[..]);
    let mut px = vec![0u8; g.w * g.h * 4];
    let to_px = c.to_pixel();
    for j in 0..g.h {
        for i in 0..g.w {
            let p = j * g.w + i;
            let [u, v] = g.centre(i, j);
            let (x, y) = ((u.floor().max(0.0) as usize).min(picture.width - 1), (v.floor().max(0.0) as usize).min(picture.height - 1));
            let photo = picture.rgb_at(x, y);
            // The photo, lightened so the model reads over it.
            let mut rgb = photo.map(|v| 0.35 + 0.65 * v);
            if model[p] {
                let m = colour[p];
                rgb = [0, 1, 2].map(|k| rgb[k] * 0.45 + m[k] * 0.55);
            }
            match tinted[p] {
                1 => rgb = [rgb[0] * 0.35 + 0.65 * 0.86, rgb[1] * 0.35 + 0.65 * 0.15, rgb[2] * 0.35 + 0.65 * 0.15],
                2 => rgb = [rgb[0] * 0.35 + 0.65 * 0.12, rgb[1] * 0.35 + 0.65 * 0.42, rgb[2] * 0.35 + 0.65 * 0.92],
                _ => {}
            }
            if po[p] {
                rgb = [0.05, 0.55, 0.2];
            }
            if mo[p] {
                rgb = [0.1, 0.1, 0.12];
            }
            for k in 0..3 {
                px[p * 4 + k] = (rgb[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            px[p * 4 + 3] = 255;
        }
    }
    // A ring round each listed region, numbered by order through its size.
    for r in regions.iter().take(8) {
        let q = apply(&to_px, r.plane);
        let (cx, cy) = ((q[0] - g.x0) / g.k, (q[1] - g.y0) / g.k);
        let radius = ((r.area_mm2.sqrt() / (c.mm_per_px_at(q) * g.k)) * 0.7).clamp(6.0, 60.0);
        let col: [u8; 3] = if r.kind == "extra" { [200, 20, 20] } else { [20, 60, 210] };
        let steps = (radius * 8.0) as usize + 16;
        for s in 0..steps {
            let a = s as f64 / steps as f64 * std::f64::consts::TAU;
            for dr in [0.0, 1.0] {
                let (x, y) = ((cx + (radius + dr) * a.cos()).round(), (cy + (radius + dr) * a.sin()).round());
                if x >= 0.0 && y >= 0.0 && (x as usize) < g.w && (y as usize) < g.h {
                    let p = (y as usize * g.w + x as usize) * 4;
                    px[p..p + 3].copy_from_slice(&col);
                }
            }
        }
    }
    crate::image::encode_png(g.w as u32, g.h as u32, &px)
}

/// Every part of a solid as a mesh to compare (world mm).
pub fn mesh_of(m: &manifold_csg::Manifold, color: [f32; 3]) -> Mesh {
    let (props, n, idx) = m.to_mesh_f64();
    let n = n.max(3);
    let positions: Vec<f32> = props.chunks_exact(n).flat_map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
    Mesh { positions, normals: Vec::new(), indices: idx.iter().map(|i| *i as u32).collect(), color, metal: false }
}
