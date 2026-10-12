//! Bitmap to component: a logo or picture (grey pixels, thresholded) traced
//! into outlines and made into a footprint for the silkscreen (filled with
//! strokes of the silk line width, outlines on the edge) or for exposed copper
//! (custom pads with no net and no paste; holes in the art kept by a hairline
//! cut to the outer edge), or into a schematic symbol (a graphic, not a part).
//!
//! Tracing follows pixel edges with the dark area on the left, so outer
//! outlines run counter-clockwise and holes clockwise (y up), then the
//! staircase is smoothed by Douglas–Peucker at a fraction of a pixel.

use crate::footprints::{Footprint, Pad, PadExtra, PadKind, PadShape};
use crate::geom::{self, P};
use crate::symbols::{Fill, Gfx, SymbolDef};
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What to make from an image.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct BitmapRequest {
    pub width: u32,
    pub height: u32,
    /// Grey levels, one byte per pixel, rows top to bottom, base64.
    pub pixels: String,
    /// Pixels darker than this (0–255) are art (default 128).
    #[serde(default = "mid")]
    pub threshold: u8,
    /// Light pixels are art instead.
    #[serde(default)]
    pub negative: bool,
    /// Width of the result, mm (height follows the image).
    pub size: f64,
    /// silk (front silkscreen), copper (exposed copper), or symbol.
    #[serde(default = "silk")]
    pub layer: String,
    pub name: Option<String>,
    /// Silkscreen stroke width (default 0.15 mm).
    pub line: Option<f64>,
}

fn mid() -> u8 {
    128
}
fn silk() -> String {
    "silk".into()
}

pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err("pixels: not base64".into()),
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Closed outlines of the art, in pixel units, y up (row 0 at the top of the image is the highest).
pub fn trace(w: usize, h: usize, on: &dyn Fn(usize, usize) -> bool) -> Vec<Vec<P>> {
    let filled = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && on(x as usize, y as usize);
    // Directed edges on the pixel lattice (x right, y down), art on the left in y-down terms = right turn;
    // we orient so that, after flipping y, outer loops are counter-clockwise.
    let mut next: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !filled(x, y) {
                continue;
            }
            // Walk clockwise around the pixel in y-down coordinates (= counter-clockwise once y is flipped).
            if !filled(x, y - 1) {
                next.entry((x + 1, y)).or_default().push((x, y));
            }
            if !filled(x - 1, y) {
                next.entry((x, y)).or_default().push((x, y + 1));
            }
            if !filled(x, y + 1) {
                next.entry((x, y + 1)).or_default().push((x + 1, y + 1));
            }
            if !filled(x + 1, y) {
                next.entry((x + 1, y + 1)).or_default().push((x + 1, y));
            }
        }
    }
    let mut loops = Vec::new();
    let mut starts: Vec<(i64, i64)> = next.keys().copied().collect();
    starts.sort();
    for s in starts {
        while next.get(&s).is_some_and(|v| !v.is_empty()) {
            let mut pts = vec![s];
            let mut cur = s;
            let mut prev_dir: Option<(i64, i64)> = None;
            loop {
                let outs = next.get_mut(&cur).unwrap();
                // At a pinch (two ways out), turn the same way every time so diagonal pixels stay apart.
                let k = if outs.len() > 1 {
                    let d = prev_dir.unwrap_or((1, 0));
                    let mut best = 0;
                    let mut best_score = i64::MIN;
                    for (i, o) in outs.iter().enumerate() {
                        let nd = (o.0 - cur.0, o.1 - cur.1);
                        let turn = d.0 * nd.1 - d.1 * nd.0; // y down: positive = right turn
                        if turn > best_score {
                            best_score = turn;
                            best = i;
                        }
                    }
                    best
                } else {
                    0
                };
                let nxt = outs.swap_remove(k);
                prev_dir = Some((nxt.0 - cur.0, nxt.1 - cur.1));
                cur = nxt;
                if cur == s {
                    break;
                }
                pts.push(cur);
                if pts.len() > 4 * (w + 1) * (h + 1) {
                    break;
                }
            }
            // Keep corners only, flip y.
            let n = pts.len();
            let mut out: Vec<P> = Vec::new();
            for i in 0..n {
                let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
                if (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0) != 0 {
                    out.push([b.0 as f64, (h as i64 - b.1) as f64]);
                }
            }
            if out.len() >= 3 {
                loops.push(out);
            }
        }
    }
    loops
}

/// Douglas–Peucker on a closed loop.
pub fn simplify(poly: &[P], tol: f64) -> Vec<P> {
    if poly.len() <= 4 {
        return poly.to_vec();
    }
    // Split at the two farthest-apart points, simplify each half.
    let a = 0;
    let b = (1..poly.len()).max_by(|i, j| geom::dist(poly[a], poly[*i]).total_cmp(&geom::dist(poly[a], poly[*j]))).unwrap();
    let mut first: Vec<P> = poly[a..=b].to_vec();
    let mut second: Vec<P> = poly[b..].to_vec();
    second.push(poly[a]);
    first = dp(&first, tol);
    second = dp(&second, tol);
    first.pop();
    second.pop();
    first.extend(second);
    first
}

fn dp(pts: &[P], tol: f64) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (mut idx, mut far) = (0, 0.0);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = geom::point_seg_dist(*p, a, b);
        if d > far {
            far = d;
            idx = i;
        }
    }
    if far <= tol {
        return vec![a, b];
    }
    let mut l = dp(&pts[..=idx], tol);
    let r = dp(&pts[idx..], tol);
    l.pop();
    l.extend(r);
    l
}

/// Join each hole to its outer outline by a hairline cut, so one outline carries both (for pads and symbol fills).
pub fn keyhole(outers: Vec<Vec<P>>, holes: Vec<Vec<P>>) -> Vec<Vec<P>> {
    let mut outs = outers;
    for hole in holes {
        let Some(&probe) = hole.first() else { continue };
        let Some(oi) = outs.iter().position(|o| geom::in_polygon(probe, o)).or(if outs.is_empty() { None } else { Some(0) }) else { continue };
        let o = &outs[oi];
        // Closest pair of vertices.
        let mut best = (0, 0, f64::INFINITY);
        for (i, p) in o.iter().enumerate() {
            for (j, q) in hole.iter().enumerate() {
                let d = geom::dist(*p, *q);
                if d < best.2 {
                    best = (i, j, d);
                }
            }
        }
        let (i, j, _) = best;
        let mut merged: Vec<P> = o[..=i].to_vec();
        for k in 0..=hole.len() {
            merged.push(hole[(j + k) % hole.len()]);
        }
        merged.push(o[i]);
        merged.extend_from_slice(&o[i + 1..]);
        outs[oi] = merged;
    }
    outs
}

/// The traced art scaled to mm, centred on the origin: (outer outlines, holes).
pub fn art(req: &BitmapRequest) -> Result<(Vec<Vec<P>>, Vec<Vec<P>>, [f64; 2]), String> {
    let (w, h) = (req.width as usize, req.height as usize);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err("the image is 1 to 4096 pixels each way".into());
    }
    if !(req.size > 0.5 && req.size <= 300.0) {
        return Err("size (the width in mm) is 0.5 to 300".into());
    }
    let px = base64_decode(&req.pixels)?;
    if px.len() < w * h {
        return Err(format!("{} pixels given for a {w} × {h} image", px.len()));
    }
    let on = |x: usize, y: usize| (px[y * w + x] < req.threshold) != req.negative;
    let loops = trace(w, h, &on);
    let scale = req.size / w as f64;
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let mut outers = Vec::new();
    let mut holes = Vec::new();
    for l in loops {
        let s = simplify(&l, 0.8);
        if s.len() < 3 {
            continue;
        }
        let mm: Vec<P> = s.iter().map(|p| [r4((p[0] - cx) * scale), r4((p[1] - cy) * scale)]).collect();
        let a = geom::polygon_area(&mm);
        if a.abs() < scale * scale * 0.75 {
            continue;
        }
        if a > 0.0 { outers.push(mm) } else { holes.push(mm) }
    }
    if outers.is_empty() {
        return Err("nothing to trace at that threshold (try another, or negative)".into());
    }
    Ok((outers, holes, [req.size, req.size * h as f64 / w as f64]))
}

fn r4(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// Fill polygons (even–odd) with horizontal strokes `line` wide, spaced to overlap, plus the outlines.
pub fn hatch(outers: &[Vec<P>], holes: &[Vec<P>], line: f64) -> Vec<Vec<P>> {
    let all: Vec<&Vec<P>> = outers.iter().chain(holes.iter()).collect();
    let (mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in all.iter().flat_map(|l| l.iter()) {
        y0 = y0.min(p[1]);
        y1 = y1.max(p[1]);
    }
    let step = line * 0.8;
    let mut out: Vec<Vec<P>> = Vec::new();
    let mut y = y0 + line / 2.0;
    while y <= y1 - line / 2.0 + 1e-9 {
        let mut xs: Vec<f64> = Vec::new();
        for l in &all {
            let n = l.len();
            for i in 0..n {
                let (a, b) = (l[i], l[(i + 1) % n]);
                if (a[1] <= y && b[1] > y) || (b[1] <= y && a[1] > y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        for pair in xs.chunks(2) {
            if pair.len() == 2 {
                let (a, b) = (pair[0] + line / 2.0, pair[1] - line / 2.0);
                let (a, b) = if b > a { (a, b) } else { let m = (pair[0] + pair[1]) / 2.0; (m, m + 1e-3) };
                out.push(vec![[r4(a), r4(y)], [r4(b), r4(y)]]);
            }
        }
        y += step;
    }
    for l in all {
        let mut c = l.clone();
        c.push(l[0]);
        out.push(c);
    }
    out
}

/// A footprint (silk or copper) or a symbol from an image.
pub enum Made {
    Footprint(Footprint),
    Symbol(SymbolDef),
}

pub fn make(req: &BitmapRequest) -> Result<Made, String> {
    let (outers, holes, size) = art(req)?;
    let name = req.name.clone().unwrap_or_else(|| format!("Logo_{}x{}mm", fmt(size[0]), fmt(size[1])));
    let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
    let cy = [-hx - 0.25, -hy - 0.25, hx + 0.25, hy + 0.25];
    match req.layer.as_str() {
        "silk" => {
            let line = req.line.unwrap_or(0.15).clamp(0.08, 1.0);
            let silk = hatch(&outers, &holes, line);
            Ok(Made::Footprint(Footprint {
                name,
                description: format!("Silkscreen logo, {} × {} mm ({} strokes of {line} mm)", fmt(size[0]), fmt(size[1]), silk.len()),
                source: format!("traced from a {} × {} pixel image at threshold {}{}", req.width, req.height, req.threshold, if req.negative { ", negative" } else { "" }),
                pads: vec![],
                courtyard: Some(cy),
                silk,
                body: vec![],
                kind: "logo".into(),
            }))
        }
        "copper" => {
            let pads = keyhole(outers, holes)
                .into_iter()
                .filter_map(|poly| {
                    let b = bbox(&poly);
                    let c = [r4((b[0] + b[2]) / 2.0), r4((b[1] + b[3]) / 2.0)];
                    let local: Vec<P> = poly.iter().map(|p| [r4(p[0] - c[0]), r4(p[1] - c[1])]).collect();
                    let size = [r4(b[2] - b[0]), r4(b[3] - b[1])];
                    (size[0] > 0.0 && size[1] > 0.0).then(|| Pad { number: String::new(), kind: PadKind::Smd, shape: PadShape::Custom, size, at: c, rotation: 0.0, drill: None, corner: None, paste: vec![], polygon: local, extra: PadExtra { no_paste: true, ..Default::default() } })
                })
                .collect();
            Ok(Made::Footprint(Footprint {
                name,
                description: format!("Copper logo (exposed, no net, no paste), {} × {} mm", fmt(size[0]), fmt(size[1])),
                source: format!("traced from a {} × {} pixel image at threshold {}{}", req.width, req.height, req.threshold, if req.negative { ", negative" } else { "" }),
                pads,
                courtyard: Some(cy),
                silk: vec![],
                body: vec![],
                kind: "logo".into(),
            }))
        }
        "symbol" => {
            // Sheet coordinates are y down.
            let flip = |l: Vec<P>| l.into_iter().map(|p| [p[0], -p[1]]).collect::<Vec<P>>();
            let gfx: Vec<Gfx> = keyhole(outers, holes).into_iter().map(|l| Gfx::Poly { pts: flip(l), fill: Fill::Outline }).collect();
            let mut s = crate::symbols::get("PWR_FLAG").unwrap();
            s.name = name;
            s.description = format!("Logo, {} × {} mm (a drawing, not a part)", fmt(size[0]), fmt(size[1]));
            s.prefix = "#LOGO".into();
            s.category = "Graphic".into();
            s.value = String::new();
            s.keywords = vec!["logo".into(), "graphic".into()];
            s.pins = vec![];
            s.units = vec![gfx];
            Ok(Made::Symbol(s))
        }
        other => Err(format!("layer is silk, copper or symbol, not {other}")),
    }
}

fn bbox(p: &[P]) -> [f64; 4] {
    p.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])])
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in bytes.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    s.push(A[(n >> (18 - 6 * i)) as usize & 63] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    /// A 20 × 20 ring: dark between radius 5 and 9 around the centre.
    fn ring() -> BitmapRequest {
        let mut px = vec![255u8; 400];
        for y in 0..20 {
            for x in 0..20 {
                let d = ((x as f64 - 9.5).powi(2) + (y as f64 - 9.5).powi(2)).sqrt();
                if (5.0..9.0).contains(&d) {
                    px[y * 20 + x] = 0;
                }
            }
        }
        BitmapRequest { width: 20, height: 20, pixels: b64(&px), threshold: 128, negative: false, size: 10.0, layer: "copper".into(), name: None, line: None }
    }

    #[test]
    fn base64_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(base64_decode(&b64(&data)).unwrap(), data);
    }

    #[test]
    fn a_ring_traces_to_an_outline_and_a_hole() {
        let (outers, holes, size) = art(&ring()).unwrap();
        assert_eq!(outers.len(), 1);
        assert_eq!(holes.len(), 1);
        assert_eq!(size, [10.0, 10.0]);
        // Area near π(9² − 5²)·(0.5 mm/px)² ≈ 44 mm².
        let a = geom::polygon_area(&outers[0]) + geom::polygon_area(&holes[0]);
        assert!((a - 44.0).abs() < 6.0, "{a}");
    }

    #[test]
    fn copper_logo_is_one_keyholed_pad_without_paste() {
        let Made::Footprint(f) = make(&ring()).unwrap() else { panic!() };
        assert_eq!(f.pads.len(), 1);
        let p = &f.pads[0];
        assert!(p.extra.no_paste && p.number.is_empty() && p.shape == PadShape::Custom);
        let a = geom::polygon_area(&p.polygon);
        assert!((a - 44.0).abs() < 6.0, "{a}");
        assert!(crate::check_footprint(&f.name, &f).is_ok());
    }

    #[test]
    fn silk_logo_fills_with_strokes() {
        let mut r = ring();
        r.layer = "silk".into();
        let Made::Footprint(f) = make(&r).unwrap() else { panic!() };
        // 10 mm tall, strokes every 0.12 mm: about 80 rows, two spans through the hole.
        assert!(f.silk.len() > 100, "{}", f.silk.len());
        assert!(f.silk.iter().all(|l| l.iter().all(|p| p[0].abs() <= 5.01 && p[1].abs() <= 5.01)));
        r.negative = true;
        r.layer = "symbol".into();
        let Made::Symbol(s) = make(&r).unwrap() else { panic!() };
        assert!(s.virtual_part && s.pins.is_empty());
    }
}
