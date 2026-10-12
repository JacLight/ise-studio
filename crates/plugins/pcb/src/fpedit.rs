//! Footprint editing tools: the footprint checker (KiCad's "Check footprint"
//! behaviour, written here from its documented checks plus the board rules a
//! fab enforces: shorted and too-close pads, rings and drills, courtyards that
//! miss copper, silkscreen on pads, broken custom outlines, paste outside its
//! pad, holes outside their copper, pad numbering), pad renumbering, and SVG
//! drawings of footprints and symbols for export.

use crate::footprints::{Footprint, Pad, PadKind, PadShape, pad_polygon, pad_prim};
use crate::geom::{self, P, Prim};
use crate::libedit::{Finding, finding};
use crate::Rules;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

fn pad_name(p: &Pad, i: usize) -> String {
    if p.number.is_empty() { format!("unnumbered pad #{}", i + 1) } else { format!("pad {}", p.number) }
}

/// Self-intersection of a closed polygon (any two non-adjacent edges crossing).
fn self_intersects(poly: &[P]) -> bool {
    let n = poly.len();
    for i in 0..n {
        let (a0, a1) = (poly[i], poly[(i + 1) % n]);
        for j in i + 1..n {
            if j == i || (j + 1) % n == i || (i + 1) % n == j {
                continue;
            }
            let (b0, b1) = (poly[j], poly[(j + 1) % n]);
            let d = |p: P, q: P, r: P| geom::cross(geom::sub(q, p), geom::sub(r, p));
            let (d1, d2, d3, d4) = (d(b0, b1, a0), d(b0, b1, a1), d(a0, a1, b0), d(a0, a1, b1));
            if ((d1 > 1e-12 && d2 < -1e-12) || (d1 < -1e-12 && d2 > 1e-12)) && ((d3 > 1e-12 && d4 < -1e-12) || (d3 < -1e-12 && d4 > 1e-12)) {
                return true;
            }
        }
    }
    false
}

/// The footprint checker, against a board's (or the default) rules.
pub fn check_footprint(f: &Footprint, rules: &Rules) -> Vec<Finding> {
    let mut out = Vec::new();
    if f.name.trim().is_empty() {
        out.push(finding("error", "no_name", "the footprint needs a name", vec![], None));
    }
    if f.pads.is_empty() {
        if f.silk.is_empty() {
            out.push(finding("error", "no_pads", "the footprint has no pads", vec![], None));
        }
        return out;
    }
    let prims: Vec<Prim> = f.pads.iter().map(|p| pad_prim(p, [0.0, 0.0], 0.0, false)).collect();
    for (i, p) in f.pads.iter().enumerate() {
        let name = pad_name(p, i);
        if !(p.size[0] > 0.0 && p.size[1] > 0.0 && p.size[0].is_finite() && p.size[1].is_finite()) {
            out.push(finding("error", "pad_size", format!("{name} needs a positive size"), vec![p.number.clone()], Some(p.at)));
            continue;
        }
        match (p.kind, p.drill) {
            (PadKind::Smd, Some(_)) => out.push(finding("error", "smd_drill", format!("{name} is SMD but has a drill"), vec![p.number.clone()], Some(p.at))),
            (PadKind::Tht | PadKind::Npth, None) => out.push(finding("error", "no_drill", format!("{name} is through-hole but has no drill"), vec![p.number.clone()], Some(p.at))),
            (PadKind::Tht, Some(d)) => {
                let off = p.extra.drill_offset.unwrap_or([0.0, 0.0]);
                // Per axis, so a slot in an oval pad is measured along and across.
                let ring = ((p.size[0] - d[0]) / 2.0 - off[0].abs()).min((p.size[1] - d[1]) / 2.0 - off[1].abs());
                if ring <= 0.0 {
                    out.push(finding("error", "hole_outside_pad", format!("{name}: the ø{} hole is not inside its copper", d[0].max(d[1])), vec![p.number.clone()], Some(p.at)));
                } else if ring + 1e-9 < rules.min_annular {
                    out.push(finding("error", "annular_ring", format!("{name}: ring {:.3} mm is under the {} mm minimum", ring, rules.min_annular), vec![p.number.clone()], Some(p.at)));
                }
                if d[0].min(d[1]) + 1e-9 < rules.min_drill {
                    out.push(finding("warning", "small_drill", format!("{name}: ø{} hole is under the {} mm minimum drill", d[0].min(d[1]), rules.min_drill), vec![p.number.clone()], Some(p.at)));
                }
                if let Some(s) = &p.extra.stack {
                    for (what, l) in [("inner", &s.inner), ("opposite", &s.opposite)] {
                        if let Some(l) = l {
                            if l.size[0].min(l.size[1]) <= d[0].max(d[1]) {
                                out.push(finding("error", "stack_size", format!("{name}: its {what}-layer pad is no bigger than the hole"), vec![p.number.clone()], Some(p.at)));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if p.shape == PadShape::Custom {
            let poly = &p.polygon;
            if poly.len() < 3 {
                out.push(finding("error", "custom_outline", format!("{name} is custom but has no outline (3 or more points)"), vec![p.number.clone()], Some(p.at)));
            } else if geom::polygon_area(poly).abs() < 1e-6 {
                out.push(finding("error", "custom_outline", format!("{name}'s outline has no area"), vec![p.number.clone()], Some(p.at)));
            } else if self_intersects(poly) {
                out.push(finding("error", "custom_outline", format!("{name}'s outline crosses itself"), vec![p.number.clone()], Some(p.at)));
            }
        }
        for (k, w) in p.paste.iter().enumerate() {
            let wb = [w[0] - w[2] / 2.0, w[1] - w[3] / 2.0, w[0] + w[2] / 2.0, w[1] + w[3] / 2.0];
            let pb = prims[i].bbox();
            if wb[0] < pb[0] - 1e-6 || wb[1] < pb[1] - 1e-6 || wb[2] > pb[2] + 1e-6 || wb[3] > pb[3] + 1e-6 {
                out.push(finding("warning", "paste_outside", format!("{name}: paste window {} reaches outside the pad", k + 1), vec![p.number.clone()], Some([w[0], w[1]])));
            }
        }
        for (what, v, max) in [("thermal_gap", p.extra.thermal_gap, 5.0), ("thermal_spoke", p.extra.thermal_spoke, 5.0)] {
            if let Some(v) = v {
                if !(v > 0.0 && v <= max) {
                    out.push(finding("error", "thermal", format!("{name}: {what} is above 0 and up to {max} mm"), vec![p.number.clone()], Some(p.at)));
                }
            }
        }
    }
    // Pad pairs: shorts between different numbers, too close for the fab, or one number apart.
    let copper: Vec<usize> = (0..f.pads.len()).filter(|i| f.pads[*i].kind != PadKind::Npth).collect();
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for &i in &copper {
        if !f.pads[i].number.is_empty() {
            groups.entry(f.pads[i].number.as_str()).or_default().push(i);
        }
    }
    for (a_i, &a) in copper.iter().enumerate() {
        for &b in &copper[a_i + 1..] {
            let (pa, pb) = (&f.pads[a], &f.pads[b]);
            if pa.number == pb.number && !pa.number.is_empty() {
                continue;
            }
            let gap = geom::gap(&prims[a], &prims[b]);
            let both_numbered = !pa.number.is_empty() && !pb.number.is_empty();
            let same_copper = pa.at == pb.at && pa.size == pb.size && pa.shape == pb.shape && pa.rotation == pb.rotation;
            if same_copper && both_numbered {
                out.push(finding("note", "shared_copper", format!("{} and {} are one piece of copper with two numbers: they share a net", pad_name(pa, a), pad_name(pb, b)), vec![pa.number.clone(), pb.number.clone()], Some(pa.at)));
            } else if gap <= 0.0 && both_numbered {
                out.push(finding("error", "pads_short", format!("{} and {} overlap: their copper is one piece, a short", pad_name(pa, a), pad_name(pb, b)), vec![pa.number.clone(), pb.number.clone()], Some(pa.at)));
            } else if gap > 0.0 && gap + 1e-9 < rules.min_space {
                out.push(finding("warning", "pads_close", format!("{} and {} are {:.3} mm apart, under the {} mm clearance", pad_name(pa, a), pad_name(pb, b), gap, rules.min_space), vec![pa.number.clone(), pb.number.clone()], Some(pa.at)));
            }
        }
    }
    for (n, list) in &groups {
        if list.len() > 1 {
            // Pads sharing a number that do not touch rely on the board to join them.
            let touching = list.iter().all(|i| list.iter().any(|j| i != j && geom::gap(&prims[*i], &prims[*j]) <= 0.0));
            if !touching && f.pads[list[0]].kind == PadKind::Smd && f.pads[list[1]].kind == PadKind::Smd {
                out.push(finding("note", "shared_number", format!("{} pads are numbered {n}: they join one net, so tracks must connect them", list.len()), vec![n.to_string()], Some(f.pads[list[0]].at)));
            }
        }
    }
    let numbered: Vec<&Pad> = f.pads.iter().filter(|p| !p.number.is_empty() && p.kind != PadKind::Npth).collect();
    let nums: BTreeSet<u32> = numbered.iter().filter_map(|p| p.number.parse().ok()).collect();
    if !nums.is_empty() && nums.len() * 2 >= numbered.len() {
        if !nums.contains(&1) && f.kind != "hole" {
            out.push(finding("warning", "no_pin_1", "no pad 1: placement and assembly find pin 1 by it", vec![], None));
        }
        let max = *nums.iter().max().unwrap();
        let missing: Vec<String> = (1..=max).filter(|n| !nums.contains(n)).map(|n| n.to_string()).collect();
        if !missing.is_empty() && missing.len() <= 32 {
            out.push(finding("note", "numbering_gap", format!("pad numbers {} are missing (1…{max})", missing.join(", ")), missing, None));
        }
    }
    // Courtyard: given, it must hold every pad and the body.
    let mut cb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in &prims {
        let b = p.bbox();
        cb = [cb[0].min(b[0]), cb[1].min(b[1]), cb[2].max(b[2]), cb[3].max(b[3])];
    }
    match f.courtyard {
        Some(c) => {
            if c[0] > cb[0] + 1e-6 || c[1] > cb[1] + 1e-6 || c[2] < cb[2] - 1e-6 || c[3] < cb[3] - 1e-6 {
                out.push(finding("error", "courtyard", "the courtyard does not hold every pad: parts would be placed onto its copper", vec![], None));
            }
            for b in &f.body {
                let bb = [b.at[0] - b.size[0] / 2.0, b.at[1] - b.size[1] / 2.0, b.at[0] + b.size[0] / 2.0, b.at[1] + b.size[1] / 2.0];
                if c[0] > bb[0] + 1e-6 || c[1] > bb[1] + 1e-6 || c[2] < bb[2] - 1e-6 || c[3] < bb[3] - 1e-6 {
                    out.push(finding("warning", "courtyard_body", "the body reaches outside the courtyard", vec![], None));
                    break;
                }
            }
        }
        None => out.push(finding("note", "courtyard_auto", "no courtyard given: it is worked out from pads and body + 0.25 mm", vec![], None)),
    }
    // Silkscreen on copper.
    let keep = 0.1 + rules.silk_width / 2.0;
    let mut on_pads: BTreeSet<String> = BTreeSet::new();
    for line in &f.silk {
        for w in line.windows(2) {
            let probe = Prim::capsule(w[0], w[1], 0.0);
            for (i, p) in f.pads.iter().enumerate() {
                if p.kind == PadKind::Npth && p.drill.is_none() {
                    continue;
                }
                if geom::gap(&prims[i], &probe) < keep {
                    on_pads.insert(pad_name(p, i));
                }
            }
        }
    }
    if !on_pads.is_empty() {
        let list: Vec<String> = on_pads.into_iter().collect();
        out.push(finding("warning", "silk_on_pad", format!("silkscreen runs over {} (fabs clip it off, so it prints broken)", list.join(", ")), list, None));
    }
    if f.body.is_empty() && f.kind != "hole" {
        out.push(finding("note", "no_body", "no body: the 3D board shows no part here and height checks see nothing", vec![], None));
    }
    out
}

/// Renumber pads in reading order (top to bottom, left to right) or keep the order given, from `start` with an optional prefix.
pub fn enumerate_pads(f: &mut Footprint, start: u32, prefix: &str, reading_order: bool) {
    let mut idx: Vec<usize> = (0..f.pads.len()).filter(|i| f.pads[*i].kind != PadKind::Npth).collect();
    if reading_order {
        idx.sort_by(|a, b| {
            let (pa, pb) = (f.pads[*a].at, f.pads[*b].at);
            (-pa[1]).partial_cmp(&-pb[1]).unwrap().then(pa[0].partial_cmp(&pb[0]).unwrap())
        });
    }
    for (k, i) in idx.into_iter().enumerate() {
        f.pads[i].number = format!("{prefix}{}", start + k as u32);
    }
}

fn n(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn path(pts: &[P], close: bool, flip: bool) -> String {
    let mut s = String::new();
    for (i, p) in pts.iter().enumerate() {
        let _ = write!(s, "{}{} {} ", if i == 0 { "M" } else { "L" }, n(p[0]), n(if flip { -p[1] } else { p[1] }));
    }
    if close {
        s.push('Z');
    }
    s
}

/// A footprint drawn as SVG (y up turned to SVG's y down): copper, holes, silk, courtyard, body outline, pad numbers.
pub fn footprint_svg(f: &Footprint) -> String {
    let cy = f.courtyard_rect();
    let m = 1.0;
    let (x0, y0, w, h) = (cy[0] - m, -cy[3] - m, cy[2] - cy[0] + 2.0 * m, cy[3] - cy[1] + 2.0 * m);
    let mut s = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}mm" height="{}mm"><title>{}</title><rect x="{}" y="{}" width="{}" height="{}" fill="#0b1a12"/>"##, n(x0), n(y0), n(w), n(h), n(w), n(h), xml(&f.name), n(x0), n(y0), n(w), n(h));
    for b in &f.body {
        let _ = write!(s, r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#7a8a99" stroke-width="0.05" stroke-dasharray="0.2 0.15"/>"##, n(b.at[0] - b.size[0] / 2.0), n(-(b.at[1] + b.size[1] / 2.0)), n(b.size[0]), n(b.size[1]));
    }
    for p in &f.pads {
        let poly = pad_polygon(p, [0.0, 0.0], 0.0, false);
        let outline = if poly.is_empty() { pad_prim(p, [0.0, 0.0], 0.0, false).outline(0.01) } else { poly };
        let fill = if p.kind == PadKind::Npth { "#0b1a12" } else { "#c8a040" };
        let _ = write!(s, r##"<path d="{}" fill="{fill}"/>"##, path(&outline, true, true));
        if let Some(d) = p.drill {
            let off = p.extra.drill_offset.map(|o| geom::rotate(o, p.rotation)).unwrap_or([0.0, 0.0]);
            let c = geom::add(p.at, off);
            let _ = write!(s, r##"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="#101010" stroke="#555" stroke-width="0.02"/>"##, n(c[0]), n(-c[1]), n(d[0] / 2.0), n(d[1] / 2.0));
        }
        if !p.number.is_empty() {
            let size = (p.size[0].min(p.size[1]) * 0.6).clamp(0.2, 1.2);
            let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" font-family="Helvetica,Arial,sans-serif" text-anchor="middle" dominant-baseline="central" fill="#1d1d1d">{}</text>"##, n(p.at[0]), n(-p.at[1]), n(size), xml(&p.number));
        }
    }
    for l in f.silk_lines() {
        let _ = write!(s, r##"<path d="{}" fill="none" stroke="#f0f0f0" stroke-width="0.15" stroke-linecap="round" stroke-linejoin="round"/>"##, path(&l, false, true));
    }
    let _ = write!(s, r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#e05cc8" stroke-width="0.05"/>"##, n(cy[0]), n(-cy[3]), n(cy[2] - cy[0]), n(cy[3] - cy[1]));
    s.push_str("</svg>");
    s
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A symbol drawn as SVG from its sheet drawing (mm, y down as on the sheet).
pub fn symbol_svg(name: &str, draw: &[crate::schematic_view::Draw], bbox: [f64; 4]) -> String {
    use crate::schematic_view::Draw;
    let m = 1.5;
    let (x0, y0, w, h) = (bbox[0] - m, bbox[1] - m, bbox[2] - bbox[0] + 2.0 * m, bbox[3] - bbox[1] + 2.0 * m);
    let ink = |k: &str| match k {
        "body" => "#8b0000",
        "pin" => "#8b0000",
        "pin_number" => "#a03030",
        "pin_name" => "#006464",
        "reference" => "#006464",
        "value" => "#006464",
        "power" => "#8b0000",
        _ => "#333",
    };
    let fill = |f: &str, k: &str| match f {
        "body" => "#ffffc2".to_string(),
        "ink" => ink(k).to_string(),
        _ => "none".to_string(),
    };
    let mut s = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}mm" height="{}mm"><title>{}</title><rect x="{}" y="{}" width="{}" height="{}" fill="#fff"/>"##, n(x0), n(y0), n(w), n(h), n(w), n(h), xml(name), n(x0), n(y0), n(w), n(h));
    for d in draw {
        match d {
            Draw::Line { p, w, ink: k } => {
                let _ = write!(s, r##"<path d="{}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"##, path(p, false, false), ink(k), n(*w));
            }
            Draw::Poly { p, w, ink: k, fill: f } => {
                let _ = write!(s, r##"<path d="{}" fill="{}" stroke="{}" stroke-width="{}" stroke-linejoin="round"/>"##, path(p, true, false), fill(f, k), ink(k), n(*w));
            }
            Draw::Circle { c, r, w, ink: k, fill: f } => {
                let _ = write!(s, r##"<circle cx="{}" cy="{}" r="{}" fill="{}" stroke="{}" stroke-width="{}"/>"##, n(c[0]), n(c[1]), n(*r), fill(f, k), ink(k), n(*w));
            }
            Draw::Text { at, s: t, size, angle, anchor, ink: k } => {
                let _ = write!(s, r##"<text x="{}" y="{}" font-size="{}" font-family="Helvetica,Arial,sans-serif" text-anchor="{}" dominant-baseline="central" fill="{}" transform="rotate({} {} {})">{}</text>"##, n(at[0]), n(at[1]), n(*size), anchor, ink(k), n(-angle), n(at[0]), n(at[1]), xml(t));
            }
        }
    }
    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprints;

    fn codes(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|x| x.code).collect()
    }

    #[test]
    fn built_in_footprints_have_no_errors() {
        for n in footprints::names() {
            let f = footprints::get(&n).unwrap();
            let errs: Vec<Finding> = check_footprint(&f, &Rules::default()).into_iter().filter(|x| x.severity == "error").collect();
            assert!(errs.is_empty(), "{n}: {errs:?}");
        }
    }

    #[test]
    fn checker_finds_shorts_rings_courtyards_and_outlines() {
        let mut f = footprints::get("SOIC-8").unwrap();
        f.pads[1].at = [f.pads[0].at[0], f.pads[0].at[1] - 0.3]; // 2 into 1
        let c = codes(&check_footprint(&f, &Rules::default()));
        assert!(c.contains(&"pads_short"), "{c:?}");
        let mut h = footprints::get("PinHeader_1x02_2.54").unwrap();
        h.pads[0].drill = Some([1.6, 1.6]); // 1.7 pad: 0.05 ring
        h.courtyard = Some([-0.5, -0.5, 0.5, 0.5]);
        let c = codes(&check_footprint(&h, &Rules::default()));
        assert!(c.contains(&"annular_ring") && c.contains(&"courtyard"), "{c:?}");
        let mut q = footprints::get("R_0603").unwrap();
        q.pads[0].shape = PadShape::Custom;
        q.pads[0].polygon = vec![[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]];
        q.silk = vec![vec![[-1.0, 0.0], [1.0, 0.0]]];
        let c = codes(&check_footprint(&q, &Rules::default()));
        assert!(c.contains(&"custom_outline") && c.contains(&"silk_on_pad"), "{c:?}");
    }

    #[test]
    fn chamfer_and_trapezoid_outlines() {
        let mut p = footprints::get("R_0603").unwrap().pads[0].clone();
        p.size = [1.0, 2.0];
        p.extra.chamfer = Some(footprints::Chamfer { ratio: 0.25, corners: vec!["top_left".into(), "bottom_right".into()] });
        p.normalize();
        assert_eq!(p.shape, PadShape::Custom);
        assert_eq!(p.polygon.len(), 6);
        // Area: 2 − 2 · ½ · 0.25² .
        assert!((geom::polygon_area(&p.polygon) - (2.0 - 0.0625)).abs() < 1e-9);
        let mut t = footprints::get("R_0603").unwrap().pads[0].clone();
        t.size = [2.0, 1.0];
        t.extra.trapezoid = Some([0.0, 0.4]);
        t.normalize();
        // Bottom edge 2.4, top 1.6: area (2.4 + 1.6) / 2 · 1.
        assert!((geom::polygon_area(&t.polygon) - 2.0).abs() < 1e-9);
        assert!((t.polygon[1][0] - t.polygon[0][0] - 2.4).abs() < 1e-9);
    }

    #[test]
    fn renumbering_in_reading_order() {
        let mut f = footprints::get("SOIC-8").unwrap();
        enumerate_pads(&mut f, 1, "", true);
        // Top row first: old 1 (left) then 8 (right).
        assert_eq!(f.pads[0].number, "1");
        assert_eq!(f.pads[7].number, "2");
        let svg = footprint_svg(&f);
        assert!(svg.starts_with("<svg") && svg.contains("<path"));
    }
}
