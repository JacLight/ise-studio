//! Coded square markers for photogrammetry: printed on a sheet the object
//! stands on, found in the photos, they give a reconstruction its size and
//! its floor.
//!
//! A marker is a 6 × 6 grid of cells: a one-cell black border around 4 × 4
//! data bits (ArUco's layout), printed with a white margin. The codes are
//! Forma's own dictionary, chosen once here (deterministically) so that no
//! code is within 3 bit flips of another or of a turn of itself: a
//! detection names its marker and which way round it lies, and survives one
//! misread cell. Detection follows the usual method (Garrido-Jurado et al.
//! 2014): adaptive threshold, connected dark regions, a quadrilateral from
//! each region's hull with its sides refined by line fits, a homography to
//! sample the cells, the code looked up in each of the four turns.

use crate::linalg::solve_dense;
use serde::Serialize;
use std::sync::OnceLock;

/// Cells across a marker, border included.
pub const GRID: usize = 6;
/// Data bits across.
pub const BITS: usize = 4;
/// Markers in the dictionary.
pub const COUNT: usize = 32;

fn rotate(code: u16) -> u16 {
    // A quarter turn clockwise: new (r, c) = old (n−1−c, r).
    let mut out = 0u16;
    for r in 0..BITS {
        for c in 0..BITS {
            if bit(code, BITS - 1 - c, r) {
                out |= 1 << (15 - (r * BITS + c));
            }
        }
    }
    out
}

fn bit(code: u16, r: usize, c: usize) -> bool {
    code >> (15 - (r * BITS + c)) & 1 == 1
}

/// The dictionary's codes (row-major bits, the top-left first).
pub fn dictionary() -> &'static [u16] {
    static DICT: OnceLock<Vec<u16>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut out: Vec<u16> = Vec::new();
        let mut i: u32 = 1;
        while out.len() < COUNT && i < 65536 {
            let c = (i.wrapping_mul(40503) & 0xffff) as u16;
            i += 1;
            let ones = c.count_ones();
            if !(5..=11).contains(&ones) {
                continue;
            }
            let r1 = rotate(c);
            let r2 = rotate(r1);
            let r3 = rotate(r2);
            if [r1, r2, r3].iter().any(|r| (r ^ c).count_ones() < 4) {
                continue;
            }
            if out.iter().all(|o| [c, r1, r2, r3].iter().all(|r| (r ^ o).count_ones() >= 4)) {
                out.push(c);
            }
        }
        out
    })
}

/// A marker's cells, row by row from the top: true is black.
pub fn cells(id: usize) -> [[bool; GRID]; GRID] {
    let code = dictionary()[id % COUNT];
    let mut g = [[true; GRID]; GRID];
    for r in 0..BITS {
        for c in 0..BITS {
            g[r + 1][c + 1] = bit(code, r, c);
        }
    }
    g
}

/// The printed marker sheet: where each marker's centre is (mm, from the
/// sheet's centre, x right, y towards the top edge) and how big they are.
#[derive(Clone, Debug, Serialize)]
pub struct Sheet {
    /// Marker side, mm (the black square).
    pub marker: f64,
    pub markers: Vec<(usize, [f64; 2])>,
    /// The check bar printed on it, mm.
    pub bar: f64,
}

impl Default for Sheet {
    /// Six 30 mm markers round a 250 × 170 mm field, fitting A4 and Letter;
    /// the object stands in the middle.
    fn default() -> Sheet {
        Sheet { marker: 30.0, markers: vec![(0, [-110.0, 70.0]), (1, [0.0, 70.0]), (2, [110.0, 70.0]), (3, [110.0, -70.0]), (4, [0.0, -70.0]), (5, [-110.0, -70.0])], bar: 100.0 }
    }
}

impl Sheet {
    /// A marker's corners on the sheet (top-left, top-right, bottom-right, bottom-left), mm.
    pub fn corners(&self, id: usize) -> Option<[[f64; 2]; 4]> {
        let (_, c) = self.markers.iter().find(|(i, _)| *i == id)?;
        let h = self.marker / 2.0;
        Some([[c[0] - h, c[1] + h], [c[0] + h, c[1] + h], [c[0] + h, c[1] - h], [c[0] - h, c[1] - h]])
    }

    /// Whether a point of the sheet (mm, sheet frame) is printed black.
    pub fn ink(&self, p: [f64; 2]) -> bool {
        let h = self.marker / 2.0;
        for (id, c) in &self.markers {
            let (u, v) = (p[0] - (c[0] - h), (c[1] + h) - p[1]);
            if u >= 0.0 && v >= 0.0 && u < self.marker && v < self.marker {
                let cell = self.marker / GRID as f64;
                return cells(*id)[(v / cell) as usize][(u / cell) as usize];
            }
        }
        false
    }

    /// The sheet as a one-page vector PDF, the markers at true size.
    pub fn pdf(&self, letter: bool) -> Vec<u8> {
        let (pw, ph) = if letter { (279.4, 215.9) } else { (297.0, 210.0) };
        let pt = 72.0 / 25.4;
        let (ox, oy) = (pw / 2.0, ph / 2.0);
        let mut s = String::new();
        s.push_str("0 0 0 rg\n");
        let rect = |s: &mut String, x: f64, y: f64, w: f64, h: f64| s.push_str(&format!("{:.3} {:.3} {:.3} {:.3} re f\n", (ox + x) * pt, (oy + y) * pt, w * pt, h * pt));
        let cell = self.marker / GRID as f64;
        for (id, c) in &self.markers {
            let g = cells(*id);
            for (r, row) in g.iter().enumerate() {
                for (k, black) in row.iter().enumerate() {
                    if *black {
                        rect(&mut s, c[0] - self.marker / 2.0 + k as f64 * cell, c[1] + self.marker / 2.0 - (r + 1) as f64 * cell, cell + 0.05, cell + 0.05);
                    }
                }
            }
            s.push_str(&format!("BT /F1 7 Tf {:.3} {:.3} Td (Forma marker {id}) Tj ET\n", (ox + c[0] - self.marker / 2.0) * pt, (oy + c[1] - self.marker / 2.0 - 4.0) * pt));
        }
        // The check bar, with 10 mm ticks.
        let (bx, by) = (-120.0, -97.0);
        rect(&mut s, bx, by, self.bar, 0.5);
        for i in 0..=(self.bar / 10.0) as usize {
            rect(&mut s, bx + i as f64 * 10.0 - 0.15, by, 0.3, if i % 5 == 0 { 4.0 } else { 2.5 });
        }
        let text = |s: &mut String, x: f64, y: f64, size: f64, t: &str| s.push_str(&format!("BT /F1 {size} Tf {:.3} {:.3} Td ({t}) Tj ET\n", (ox + x) * pt, (oy + y) * pt));
        text(&mut s, bx + self.bar + 3.0, by, 7.0, &format!("{} mm: check it measures {} mm (print at 100%, no fit to page)", self.bar, self.bar));
        text(&mut s, -115.0, 92.0, 9.0, &format!("Forma photogrammetry scale sheet: {} mm markers. Stand the object in the middle; keep the markers in view.", self.marker));
        // A cross at the centre (the origin of the sheet's frame).
        rect(&mut s, -5.0, -0.15, 10.0, 0.3);
        rect(&mut s, -0.15, -5.0, 0.3, 10.0);
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>", pw * pt, ph * pt),
            format!("<< /Length {} >>\nstream\n{s}endstream", s.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }
}

/// A marker found in a picture.
#[derive(Clone, Debug, Serialize)]
pub struct Detection {
    pub id: usize,
    /// The marker's top-left, top-right, bottom-right and bottom-left corners
    /// as printed, in pixels (x right, y down; centres of pixels at +0.5).
    pub corners: [[f64; 2]; 4],
    /// Cells read wrong (0 or 1).
    pub errors: u32,
}

/// A homography mapping (u, v) to pixels.
type H = [f64; 9];

fn homography(src: &[[f64; 2]; 4], dst: &[[f64; 2]; 4]) -> Option<H> {
    let mut a = vec![0.0; 64];
    let mut b = vec![0.0; 8];
    for i in 0..4 {
        let ([x, y], [u, v]) = (src[i], dst[i]);
        let r = 2 * i;
        a[r * 8..r * 8 + 8].copy_from_slice(&[x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y]);
        b[r] = u;
        a[(r + 1) * 8..(r + 1) * 8 + 8].copy_from_slice(&[0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y]);
        b[r + 1] = v;
    }
    let h = solve_dense(&a, &b, 8)?;
    Some([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0])
}

fn map(h: &H, p: [f64; 2]) -> [f64; 2] {
    let w = h[6] * p[0] + h[7] * p[1] + h[8];
    [(h[0] * p[0] + h[1] * p[1] + h[2]) / w, (h[3] * p[0] + h[4] * p[1] + h[5]) / w]
}

fn sample(gray: &[f32], w: usize, h: usize, p: [f64; 2]) -> f32 {
    let (x, y) = ((p[0] - 0.5).clamp(0.0, w as f64 - 1.001), (p[1] - 0.5).clamp(0.0, h as f64 - 1.001));
    let (x0, y0) = (x as usize, y as usize);
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let at = |x: usize, y: usize| gray[y * w + x];
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

fn cross2(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for q in &p {
        while lower.len() >= 2 && cross2(lower[lower.len() - 2], lower[lower.len() - 1], *q) <= 0.0 {
            lower.pop();
        }
        lower.push(*q);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for q in p.iter().rev() {
        while upper.len() >= 2 && cross2(upper[upper.len() - 2], upper[upper.len() - 1], *q) <= 0.0 {
            upper.pop();
        }
        upper.push(*q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn d2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// The four hull points spanning the largest quadrilateral (approximately).
fn quad_of(h: &[[f64; 2]]) -> Option<[[f64; 2]; 4]> {
    if h.len() < 4 {
        return None;
    }
    let c = h.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0] / h.len() as f64, s[1] + p[1] / h.len() as f64]);
    let a = *h.iter().max_by(|p, q| d2(**p, c).total_cmp(&d2(**q, c)))?;
    let b = *h.iter().max_by(|p, q| d2(**p, a).total_cmp(&d2(**q, a)))?;
    let side = |p: &[f64; 2]| cross2(a, b, *p);
    let p1 = *h.iter().max_by(|p, q| side(p).total_cmp(&side(q)))?;
    let p2 = *h.iter().min_by(|p, q| side(p).total_cmp(&side(q)))?;
    if side(&p1) <= 0.0 || side(&p2) >= 0.0 {
        return None;
    }
    // In order round the hull: a, p1, b, p2 (or reversed).
    Some([a, p1, b, p2])
}

fn area(q: &[[f64; 2]; 4]) -> f64 {
    (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum::<f64>() / 2.0
}

/// A line through points: (point on it, unit direction).
fn fit_line(p: &[[f64; 2]]) -> Option<([f64; 2], [f64; 2])> {
    if p.len() < 3 {
        return None;
    }
    let n = p.len() as f64;
    let c = p.iter().fold([0.0, 0.0], |s, q| [s[0] + q[0] / n, s[1] + q[1] / n]);
    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
    for q in p {
        let (dx, dy) = (q[0] - c[0], q[1] - c[1]);
        xx += dx * dx;
        xy += dx * dy;
        yy += dy * dy;
    }
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    Some((c, [angle.cos(), angle.sin()]))
}

fn intersect(a: ([f64; 2], [f64; 2]), b: ([f64; 2], [f64; 2])) -> Option<[f64; 2]> {
    let den = a.1[0] * b.1[1] - a.1[1] * b.1[0];
    if den.abs() < 1e-9 {
        return None;
    }
    let t = ((b.0[0] - a.0[0]) * b.1[1] - (b.0[1] - a.0[1]) * b.1[0]) / den;
    Some([a.0[0] + t * a.1[0], a.0[1] + t * a.1[1]])
}

/// Finds the dictionary's markers in a grey picture (0 black … 1 white,
/// row by row).
pub fn detect(gray: &[f32], w: usize, h: usize) -> Vec<Detection> {
    if gray.len() != w * h || w < 16 || h < 16 {
        return vec![];
    }
    // Adaptive threshold against the local mean (integral image).
    let mut integral = vec![0.0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0.0;
        for x in 0..w {
            row += gray[y * w + x] as f64;
            integral[(y + 1) * (w + 1) + x + 1] = integral[y * (w + 1) + x + 1] + row;
        }
    }
    let r = (w.max(h) / 40).clamp(7, 80);
    let mut dark = vec![false; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let sum = integral[y1 * (w + 1) + x1] - integral[y0 * (w + 1) + x1] - integral[y1 * (w + 1) + x0] + integral[y0 * (w + 1) + x0];
            let mean = sum / ((y1 - y0) * (x1 - x0)) as f64;
            let g = gray[y * w + x] as f64;
            dark[y * w + x] = g + 0.06 < mean || g < 0.12;
        }
    }
    // Connected dark regions.
    let mut label = vec![u32::MAX; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    let min_pixels = (w * h / 40000).max(60);
    for start in 0..w * h {
        if !dark[start] || label[start] != u32::MAX {
            continue;
        }
        let id = start as u32;
        label[start] = id;
        stack.push(start);
        let mut pixels = Vec::new();
        let mut touches = false;
        while let Some(k) = stack.pop() {
            pixels.push(k);
            let (x, y) = (k % w, k / w);
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                touches = true;
                continue;
            }
            for n in [k - 1, k + 1, k - w, k + w] {
                if dark[n] && label[n] == u32::MAX {
                    label[n] = id;
                    stack.push(n);
                }
            }
        }
        if touches || pixels.len() < min_pixels || pixels.len() > w * h / 4 {
            continue;
        }
        // Its edge pixels, and the quadrilateral round them.
        let edge: Vec<[f64; 2]> = pixels
            .iter()
            .filter(|&&k| [k - 1, k + 1, k - w, k + w].iter().any(|&n| label[n] != id))
            .map(|&k| [(k % w) as f64 + 0.5, (k / w) as f64 + 0.5])
            .collect();
        let Some(mut q) = quad_of(&hull(edge.clone())) else { continue };
        if area(&q) < 0.0 {
            q.reverse();
        }
        let a = area(&q);
        let fill = pixels.len() as f64 / a.max(1.0);
        if a < 200.0 || !(0.3..1.1).contains(&fill) || (0..4).any(|i| d2(q[i], q[(i + 1) % 4]) < 100.0) {
            continue;
        }
        // Refine each side with a line through its edge pixels (half a pixel out).
        let mut lines = Vec::new();
        for i in 0..4 {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            let len = d2(a, b).sqrt();
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let out_n = [dir[1], -dir[0]];
            let near: Vec<[f64; 2]> = edge
                .iter()
                .filter(|p| {
                    let (dx, dy) = (p[0] - a[0], p[1] - a[1]);
                    let t = (dx * dir[0] + dy * dir[1]) / len;
                    let off = dx * out_n[0] + dy * out_n[1];
                    (0.12..0.88).contains(&t) && off.abs() < 2.5
                })
                .copied()
                .collect();
            lines.push(fit_line(&near).map(|(c, d)| ([c[0] + 0.5 * out_n[0], c[1] + 0.5 * out_n[1]], d)));
        }
        let mut refined = q;
        if lines.iter().all(Option::is_some) {
            let l: Vec<_> = lines.into_iter().flatten().collect();
            for i in 0..4 {
                if let Some(p) = intersect(l[(i + 3) % 4], l[i]).filter(|p| d2(*p, q[i]) < 16.0) {
                    refined[i] = p;
                }
            }
        }
        if let Some(d) = decode(gray, w, h, &refined) {
            out.push(d);
        }
    }
    // One detection per marker id per place.
    out.sort_by_key(|d| d.errors);
    let mut kept: Vec<Detection> = Vec::new();
    for d in out {
        if !kept.iter().any(|k| k.id == d.id && d2(k.corners[0], d.corners[0]) < 25.0) {
            kept.push(d);
        }
    }
    kept
}

fn decode(gray: &[f32], w: usize, h: usize, q: &[[f64; 2]; 4]) -> Option<Detection> {
    let g = GRID as f64;
    let hm = homography(&[[0.0, 0.0], [g, 0.0], [g, g], [0.0, g]], q)?;
    let cell = |u: f64, v: f64| -> f32 {
        let mut s = 0.0;
        for (du, dv) in [(0.0, 0.0), (-0.2, -0.2), (0.2, -0.2), (-0.2, 0.2), (0.2, 0.2)] {
            s += sample(gray, w, h, map(&hm, [u + du, v + dv]));
        }
        s / 5.0
    };
    let mut border = Vec::new();
    let mut quiet = Vec::new();
    for i in 0..GRID {
        let t = i as f64 + 0.5;
        for (u, v) in [(t, 0.5), (t, g - 0.5), (0.5, t), (g - 0.5, t)] {
            border.push(cell(u, v));
        }
        for (u, v) in [(t, -0.5), (t, g + 0.5), (-0.5, t), (g + 0.5, t)] {
            quiet.push(cell(u, v));
        }
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let (black, white) = (mean(&border), mean(&quiet));
    if white - black < 0.12 {
        return None;
    }
    let thr = (black + white) / 2.0;
    if border.iter().filter(|v| **v > thr).count() > 2 {
        return None;
    }
    let mut code = 0u16;
    for r in 0..BITS {
        for c in 0..BITS {
            let v = cell(c as f64 + 1.5, r as f64 + 1.5);
            if v < thr {
                code |= 1 << (15 - (r * BITS + c));
            }
        }
    }
    let dict = dictionary();
    let mut seen = code;
    for turn in 0..4 {
        // Read from q[0], the code is the printed one turned; undo the turn.
        if let Some((id, errors)) = dict.iter().enumerate().map(|(i, d)| (i, (d ^ seen).count_ones())).min_by_key(|x| x.1).filter(|x| x.1 <= 1) {
            let corners = [0, 1, 2, 3].map(|j| q[(j + turn) % 4]);
            return Some(Detection { id, corners, errors });
        }
        seen = rotate(rotate(rotate(seen)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture of a marker under a homography (marker cells → pixels), with a white margin.
    fn picture(id: usize, w: usize, h: usize, quad: [[f64; 2]; 4]) -> Vec<f32> {
        let g = GRID as f64;
        let inv = homography(&quad, &[[0.0, 0.0], [g, 0.0], [g, g], [0.0, g]]).unwrap();
        let c = cells(id);
        let mut out = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                // 4×4 supersampling.
                let mut s = 0.0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let uv = map(&inv, [x as f64 + (sx as f64 + 0.5) / 4.0, y as f64 + (sy as f64 + 0.5) / 4.0]);
                        let inside = (0.0..g).contains(&uv[0]) && (0.0..g).contains(&uv[1]);
                        s += if inside && c[uv[1] as usize][uv[0] as usize] { 0.05 } else { 0.92 };
                    }
                }
                out[y * w + x] = s / 16.0;
            }
        }
        out
    }

    #[test]
    fn the_dictionary_is_distinct_under_turns() {
        let d = dictionary();
        assert_eq!(d.len(), COUNT);
        for (i, a) in d.iter().enumerate() {
            let r = rotate(*a);
            assert_eq!(rotate(rotate(rotate(r))), *a);
            for b in &d[i + 1..] {
                let mut x = *b;
                for _ in 0..4 {
                    assert!((a ^ x).count_ones() >= 4);
                    x = rotate(x);
                }
            }
        }
    }

    #[test]
    fn markers_are_found_turned_and_in_perspective_with_their_corners() {
        // Each of the four turns: the corners come back as printed.
        let base = [[60.0, 50.0], [190.0, 62.0], [180.0, 200.0], [52.0, 180.0]];
        for turn in 0..4 {
            let quad = [0, 1, 2, 3].map(|j| base[(j + turn) % 4]);
            let img = picture(7, 260, 250, quad);
            let found = detect(&img, 260, 250);
            assert_eq!(found.len(), 1, "turn {turn}: {found:?}");
            assert_eq!(found[0].id, 7);
            for j in 0..4 {
                let e = d2(found[0].corners[j], quad[j]).sqrt();
                assert!(e < 0.6, "turn {turn} corner {j}: off by {e:.2} px ({:?} vs {:?})", found[0].corners[j], quad[j]);
            }
        }
    }

    #[test]
    fn a_sheet_prints_its_markers() {
        let s = Sheet::default();
        let pdf = s.pdf(false);
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
        // The border is black, the quiet zone white.
        let c = s.corners(0).unwrap();
        assert!(s.ink([c[0][0] + 0.5, c[0][1] - 0.5]));
        assert!(!s.ink([c[0][0] - 0.5, c[0][1] + 0.5]));
    }
}
