//! Tracing pictures into outlines: a logo, a line drawing, a silhouette.
//!
//! The picture's ink (dark, or opaque on a transparent background) is
//! softened by a pixel or so, then contoured where it crosses the threshold
//! (marching squares, with each crossing placed between pixel centres by
//! interpolation, so antialiased edges land to a fraction of a pixel and
//! jagged ones come out smooth). Contours run with the ink on their left, so
//! outlines are counter-clockwise and holes clockwise, then are thinned to
//! within the tolerance (Douglas–Peucker) and specks are dropped.

use crate::image::{Channel, Field, Picture};

#[derive(Clone, Debug)]
pub struct Spec {
    /// Size in mm; the other side follows the picture's proportions.
    pub width: f64,
    pub height: f64,
    /// Ink above this (0–1) is inside.
    pub threshold: f64,
    /// Trace the light parts instead.
    pub invert: bool,
    /// Gaussian softening, mm (rounds pixel steps; 0 keeps every step).
    pub smooth: f64,
    /// Outlines and holes smaller than this (mm²) are dropped.
    pub min_area: f64,
    /// Largest distance the thinned outline may stray, mm.
    pub tolerance: f64,
    /// None: alpha if the picture has see-through pixels, else darkness.
    pub channel: Option<Channel>,
}

/// The ink as 0–1 values (1 is ink).
pub fn ink(picture: &Picture, spec: &Spec) -> Field {
    let (mut f, dark) = match spec.channel {
        Some(Channel::Brightness) => (picture.field(Channel::Brightness), true),
        Some(c) => (picture.field(c), false),
        None if picture.has_alpha() => (picture.field(Channel::Alpha), false),
        None => (picture.field(Channel::Brightness), true),
    };
    if dark != spec.invert {
        for v in &mut f.v {
            *v = 1.0 - *v;
        }
    }
    f
}

/// Closed outlines in mm, centred on the origin, +y up: counter-clockwise
/// around ink, clockwise around holes.
pub fn trace(picture: &Picture, spec: &Spec) -> Result<Vec<Vec<[f64; 2]>>, String> {
    if !(spec.width > 0.0 && spec.height > 0.0) {
        return Err("trace: the size must be positive".into());
    }
    let mut f = ink(picture, spec);
    let px = spec.width / picture.width as f64;
    f.blur(spec.smooth / px);
    let loops = contours(&f, spec.threshold as f32);
    let (sx, sy) = (spec.width / f.w as f64, spec.height / f.h as f64);
    let mut out = Vec::new();
    for l in loops {
        // Pixel coordinates (y down) to mm (y up, centred): mirroring y keeps
        // ink on the left as seen, which is counter-clockwise in mm.
        let pts: Vec<[f64; 2]> = l.iter().map(|p| [(p[0] - f.w as f64 / 2.0) * sx, (f.h as f64 / 2.0 - p[1]) * sy]).collect();
        let pts = simplify(&pts, spec.tolerance.max(1e-6));
        if pts.len() >= 3 && area(&pts).abs() >= spec.min_area {
            out.push(pts);
        }
    }
    Ok(out)
}

/// Signed area: positive counter-clockwise.
pub fn area(p: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (u, v) = (p[i], p[(i + 1) % p.len()]);
        a += u[0] * v[1] - v[0] * u[1];
    }
    a / 2.0
}

/// Marching squares over pixel centres (pixel (x, y)'s centre at x + ½, y + ½),
/// with a ring of empty pixels around so every contour closes. Ink is on the
/// left as the picture is seen (y down here, so the cross product of travel
/// and the way to the ink is negative).
pub(crate) fn contours(f: &Field, t: f32) -> Vec<Vec<[f64; 2]>> {
    let (w, h) = (f.w as isize, f.h as isize);
    let val = |x: isize, y: isize| -> f32 { if x < 0 || y < 0 || x >= w || y >= h { 0.0 } else { f.v[(y * w + x) as usize] } };
    let inside = |x: isize, y: isize| val(x, y) > t;
    // Cells span sample (x, y) to (x + 1, y + 1), x and y from −1 to w − 1 / h − 1.
    let cw = w + 1;
    // Crossing ids: horizontal edge from (x, y) to (x + 1, y), vertical from (x, y) to (x, y + 1).
    let hid = |x: isize, y: isize| (((y + 1) * (cw + 1) + (x + 1)) * 2) as usize;
    let vid = |x: isize, y: isize| (((y + 1) * (cw + 1) + (x + 1)) * 2 + 1) as usize;
    let point = |id: usize| -> [f64; 2] {
        let cell = (id / 2) as isize;
        let (x, y) = (cell % (cw + 1) - 1, cell / (cw + 1) - 1);
        let (x1, y1) = if id % 2 == 0 { (x + 1, y) } else { (x, y + 1) };
        let (a, b) = (val(x, y), val(x1, y1));
        let s = if (b - a).abs() > 1e-12 { ((t - a) / (b - a)).clamp(0.0, 1.0) as f64 } else { 0.5 };
        let (px, py) = (x as f64 + 0.5 + (x1 - x) as f64 * s, y as f64 + 0.5 + (y1 - y) as f64 * s);
        // The ring outside the picture closes contours on its edge.
        [px.clamp(0.0, w as f64), py.clamp(0.0, h as f64)]
    };
    let mut next: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for y in -1..h {
        for x in -1..w {
            let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)];
            let ins = corners.map(|(cx, cy)| inside(cx, cy));
            let count = ins.iter().filter(|b| **b).count();
            if count == 0 || count == 4 {
                continue;
            }
            // Edges by the corners they join: top (0–1), right (1–2), bottom (3–2), left (0–3).
            let edge = [hid(x, y), vid(x + 1, y), hid(x, y + 1), vid(x, y)];
            // Each corner and its two edges.
            let around = [(0usize, [3usize, 0usize]), (1, [0, 1]), (2, [1, 2]), (3, [2, 3])];
            let mut cuts: Vec<(usize, [usize; 2])> = Vec::new();
            if count == 2 && ins[0] == ins[2] {
                // Saddle: the centre decides which pair of corners joins.
                let centre = (val(x, y) + val(x + 1, y) + val(x + 1, y + 1) + val(x, y + 1)) / 4.0 > t;
                for (c, e) in around {
                    if ins[c] != centre {
                        cuts.push((c, e));
                    }
                }
            } else if count == 1 || count == 3 {
                // One corner differs from the rest: cut it off.
                let odd = (0..4).find(|&c| ins[c] == (count == 1)).unwrap();
                cuts.push((odd, around[odd].1));
            } else {
                // Two neighbours inside: a line across the cell.
                let crossed: Vec<usize> = (0..4).filter(|&e| ins[[0, 1, 3, 0][e]] != ins[[1, 2, 2, 3][e]]).collect();
                let c = (0..4).find(|&c| ins[c]).unwrap();
                cuts.push((usize::MAX - c, [crossed[0], crossed[1]]));
            }
            for (c, [e0, e1]) in cuts {
                let (a, b) = (edge[e0], edge[e1]);
                let (pa, pb) = (point(a), point(b));
                // A point on the ink side: the inside corner, or across from an outside one.
                let (corner, ink_side) = if c > 3 { (usize::MAX - c, true) } else { (c, ins[c]) };
                let (cx, cy) = corners[corner];
                let q = [cx as f64 + 0.5, cy as f64 + 0.5];
                let cross = (pb[0] - pa[0]) * (q[1] - pa[1]) - (pb[1] - pa[1]) * (q[0] - pa[0]);
                let ink_left = (cross < 0.0) == ink_side;
                if ink_left { next.insert(a, b) } else { next.insert(b, a) };
            }
        }
    }
    let mut loops = Vec::new();
    let mut starts: Vec<usize> = next.keys().copied().collect();
    starts.sort_unstable();
    for s in starts {
        if !next.contains_key(&s) {
            continue;
        }
        let mut l = Vec::new();
        let mut at = s;
        while let Some(n) = next.remove(&at) {
            let p = point(at);
            if l.last().is_none_or(|q: &[f64; 2]| (q[0] - p[0]).abs() + (q[1] - p[1]).abs() > 1e-9) {
                l.push(p);
            }
            at = n;
        }
        if l.len() >= 3 {
            loops.push(l);
        }
    }
    loops
}

/// Douglas–Peucker on a closed loop: corners kept, points within `eps` of the
/// line through their neighbours dropped.
pub fn simplify(p: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    if p.len() < 4 {
        return p.to_vec();
    }
    // Split at the point farthest from the first, so both halves are open chains.
    let far = (1..p.len()).max_by(|&a, &b| d2(p[0], p[a]).total_cmp(&d2(p[0], p[b]))).unwrap();
    let mut keep = vec![false; p.len() + 1];
    keep[0] = true;
    keep[far] = true;
    keep[p.len()] = true;
    let at = |i: usize| p[i % p.len()];
    let mut stack = vec![(0, far), (far, p.len())];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (at(a), at(b));
        let (mut worst, mut wi) = (0.0, 0);
        for i in a + 1..b {
            let d = segment_distance(at(i), pa, pb);
            if d > worst {
                worst = d;
                wi = i;
            }
        }
        if worst > eps {
            keep[wi] = true;
            stack.push((a, wi));
            stack.push((wi, b));
        }
    }
    (0..p.len()).filter(|&i| keep[i]).map(|i| p[i]).collect()
}

fn d2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

fn segment_distance(q: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 { (((q[0] - a[0]) * ab[0] + (q[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
    d2(q, [a[0] + ab[0] * t, a[1] + ab[1] * t]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: usize, h: usize, f: impl Fn(f64, f64) -> u8) -> Picture {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = f(x as f64 + 0.5, y as f64 + 0.5);
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        crate::image::decode(&crate::image::encode_png(w as u32, h as u32, &px)).unwrap()
    }

    fn spec(w: f64, h: f64) -> Spec {
        Spec { width: w, height: h, threshold: 0.5, invert: false, smooth: 0.0, min_area: 0.0, tolerance: 0.01, channel: None }
    }

    #[test]
    fn a_ring_traces_to_an_outline_and_a_hole() {
        // A black ring, outer radius 40 px, inner 20 px, on white; 100 px = 50 mm.
        let p = picture(100, 100, |x, y| {
            let r = ((x - 50.0).powi(2) + (y - 50.0).powi(2)).sqrt();
            if (20.0..=40.0).contains(&r) { 0 } else { 255 }
        });
        let loops = trace(&p, &spec(50.0, 50.0)).unwrap();
        assert_eq!(loops.len(), 2);
        let mut areas: Vec<f64> = loops.iter().map(|l| area(l)).collect();
        areas.sort_by(f64::total_cmp);
        // Outline counter-clockwise (positive), hole clockwise; areas near π r².
        let (outer, hole) = (std::f64::consts::PI * 20.0f64.powi(2), std::f64::consts::PI * 10.0f64.powi(2));
        assert!(areas[0] < 0.0 && areas[1] > 0.0);
        assert!((areas[1] - outer).abs() / outer < 0.03, "{} vs {outer}", areas[1]);
        assert!((-areas[0] - hole).abs() / hole < 0.05, "{} vs {hole}", -areas[0]);
    }

    #[test]
    fn straight_edges_thin_to_their_corners() {
        // A black rectangle 60 × 20 px, antialiasing-free; 1 px = 1 mm.
        let p = picture(100, 50, |x, y| if (20.0..80.0).contains(&x) && (15.0..35.0).contains(&y) { 0 } else { 255 });
        let loops = trace(&p, &spec(100.0, 50.0)).unwrap();
        assert_eq!(loops.len(), 1);
        // Marching squares chamfers each corner by half a pixel: 8 points at most.
        assert!(loops[0].len() <= 8, "{} points", loops[0].len());
        assert!((area(&loops[0]) - 60.0 * 20.0).abs() < 2.0, "{}", area(&loops[0]));
    }

    #[test]
    fn specks_are_dropped_and_softening_rounds_steps() {
        let p = picture(60, 60, |x, y| if (x - 3.0).abs() < 1.0 && (y - 3.0).abs() < 1.0 || ((x - 30.0).powi(2) + (y - 30.0).powi(2)).sqrt() < 15.0 { 0 } else { 255 });
        let mut s = spec(60.0, 60.0);
        s.min_area = 10.0;
        assert_eq!(trace(&p, &s).unwrap().len(), 1);
        s.smooth = 1.0;
        let l = &trace(&p, &s).unwrap()[0];
        let r: Vec<f64> = l.iter().map(|q| (q[0].powi(2) + q[1].powi(2)).sqrt()).collect();
        let (lo, hi) = r.iter().fold((f64::MAX, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(hi - lo < 0.8, "radius varies {lo}…{hi}");
    }
}
