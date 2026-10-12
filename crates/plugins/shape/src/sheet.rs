//! Sheet-metal flat patterns: what the laser cuts and the press brake bends.
//!
//! sheet.js works a sheet-metal part out twice from one description, bent and
//! flat (each bend's allowance from its angle, inside radius and K-factor).
//! Here the flat blank becomes the shop's files: a DXF with the cut outline
//! (true circles where a hole is round) and the bend lines on their own
//! layers, an SVG to look at, and the bend table as data for drawings.

use crate::{ShapeProgram, evaluated_at};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write as _;

/// One bend, as sheet.js reports it (mm and degrees, flat-pattern coordinates).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bend {
    /// "B1", "B2", … in the order the flanges were made.
    pub id: String,
    /// The face it bends from, and the face it makes.
    pub from: String,
    pub to: String,
    /// "up" toward the flat pattern's top side (the side the DXF is drawn
    /// from), "down" away from it.
    pub dir: String,
    pub angle: f64,
    /// Inside radius.
    pub radius: f64,
    pub k: f64,
    /// Arc length of the neutral axis: the flat strip the bend takes.
    pub allowance: f64,
    /// What the bend takes off the sum of the outside dimensions.
    pub deduction: f64,
    /// From the outer virtual sharp to the bend's tangent line.
    pub setback: f64,
    /// Length of the bend line.
    pub length: f64,
    /// The bend's centre line in the flat pattern.
    pub line: [[f64; 2]; 2],
    /// Where the bend starts and ends in the flat (its tangent lines).
    pub tangents: [[[f64; 2]; 2]; 2],
    /// Suggested press-brake sequence (1 first).
    pub order: u32,
    #[serde(default)]
    pub hem: bool,
}

/// A formed feature (a louvre) marked for the forming tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub kind: String,
    pub face: String,
    pub outline: Vec<[f64; 2]>,
    pub dir: String,
    pub height: f64,
}

/// A design-rule finding on a sheet-metal part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetCheck {
    /// "error", "warning" or "info".
    pub severity: String,
    /// FLANGE_TOO_SHORT, HOLE_NEAR_BEND, CUT_IN_BEND, HOLE_NEAR_EDGE,
    /// NO_BEND_RELIEF, RELIEF_TOO_NARROW, BEND_RADIUS_TOO_SMALL, FLAT_OVERLAP,
    /// FOLDED_CLASH, HEM_TOO_SHORT, CORNER_UNTREATED, NONSTANDARD_GAUGE,
    /// UNKNOWN_MATERIAL.
    pub code: String,
    pub message: String,
    /// Where, in flat-pattern coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
}

/// A sheet-metal part's flat pattern, ready for the shop.
#[derive(Clone, Debug, Serialize)]
pub struct FlatPattern {
    pub part: String,
    pub material: String,
    pub thickness: f64,
    /// The part's default inside radius and its K-factor.
    pub radius: f64,
    pub k: f64,
    /// Flat blank, x × y (mm), its corner at the origin.
    pub size: [f64; 2],
    /// Blank area (mm²) and the laser's path: every outline's length (mm)
    /// and how many times it pierces (one per closed outline).
    pub area: f64,
    pub cut_length: f64,
    pub pierces: usize,
    /// Closed outlines of the blank: the outside counter-clockwise, holes clockwise.
    pub loops: Vec<Vec<[f64; 2]>>,
    pub bends: Vec<Bend>,
    pub forms: Vec<Form>,
    pub checks: Vec<SheetCheck>,
    /// Everything sheet.js reported (shop, V-die, minimum flange, faces).
    pub data: Value,
}

impl FlatPattern {
    /// The blank's outlines from the flat solid, at mid-thickness.
    pub(crate) fn from_parts(part: &str, flat: &manifold_csg::Manifold, data: &Value) -> Result<FlatPattern, String> {
        let thickness = data["thickness"].as_f64().ok_or("the flat pattern has no thickness")?;
        let loops: Vec<Vec<[f64; 2]>> = flat.slice_at_z(thickness / 2.0).into_iter().filter(|r| r.len() >= 3).collect();
        let cut_length = loops.iter().map(|r| perimeter(r)).sum();
        let list = |key: &str| data.get(key).cloned().unwrap_or(Value::Array(vec![]));
        Ok(FlatPattern {
            part: part.to_string(),
            material: data["material"].as_str().unwrap_or("").to_string(),
            thickness,
            radius: data["radius"].as_f64().unwrap_or(0.0),
            k: data["k"].as_f64().unwrap_or(0.0),
            size: serde_json::from_value(data["flat"]["size"].clone()).map_err(|e| format!("flat size: {e}"))?,
            area: loops.iter().map(|r| ring_area(r)).sum(),
            cut_length,
            pierces: loops.len(),
            loops,
            bends: serde_json::from_value(list("bends")).map_err(|e| format!("bend table: {e}"))?,
            forms: serde_json::from_value(list("forms")).map_err(|e| format!("forms: {e}"))?,
            checks: serde_json::from_value(list("checks")).map_err(|e| format!("checks: {e}"))?,
            data: data.clone(),
        })
    }

    pub fn errors(&self) -> usize {
        self.checks.iter().filter(|c| c.severity == "error").count()
    }
}

/// The flat pattern of a program's sheet-metal part (`part` names it in a
/// multi-part program; leave it out when there is only one).
pub fn flat_pattern(program: &ShapeProgram, part: Option<&str>, tolerance: f64) -> Result<FlatPattern, String> {
    let e = evaluated_at(program, tolerance)?;
    if e.sheets.is_empty() {
        return Err(format!("{} has no sheet-metal part (build it with sheet() or sheetProfile() and return sheet.part(…))", program.name));
    }
    let pick = match part {
        Some(name) => e.sheets.iter().find(|s| s.part == name).ok_or_else(|| {
            let names: Vec<&str> = e.sheets.iter().map(|s| s.part.as_str()).collect();
            format!("“{name}” is not a sheet-metal part of {} (its sheet parts: {})", program.name, names.join(", "))
        })?,
        None if e.sheets.len() == 1 => &e.sheets[0],
        None => {
            let names: Vec<&str> = e.sheets.iter().map(|s| s.part.as_str()).collect();
            return Err(format!("{} has {} sheet-metal parts: pick one ({})", program.name, names.len(), names.join(", ")));
        }
    };
    let name = if pick.part.is_empty() { program.name.as_str() } else { pick.part.as_str() };
    FlatPattern::from_parts(name, &pick.flat, &pick.data)
}

/// Every sheet-metal part of a program, flat.
pub fn flat_patterns(program: &ShapeProgram, tolerance: f64) -> Result<Vec<FlatPattern>, String> {
    let e = evaluated_at(program, tolerance)?;
    e.sheets.iter().map(|s| FlatPattern::from_parts(if s.part.is_empty() { &program.name } else { &s.part }, &s.flat, &s.data)).collect()
}

fn perimeter(ring: &[[f64; 2]]) -> f64 {
    (0..ring.len()).map(|i| dist(ring[i], ring[(i + 1) % ring.len()])).sum()
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1]).sum::<f64>() / 2.0
}

/// A ring that is a circle (every vertex the same distance from the
/// middle): its centre and radius, so the DXF can say CIRCLE.
fn as_circle(ring: &[[f64; 2]]) -> Option<([f64; 2], f64)> {
    if ring.len() < 12 {
        return None;
    }
    // Least-squares circle (Kåsa): x² + y² + D·x + E·y + F = 0, about the mean for conditioning.
    let n = ring.len() as f64;
    let m = [ring.iter().map(|p| p[0]).sum::<f64>() / n, ring.iter().map(|p| p[1]).sum::<f64>() / n];
    let (mut sxx, mut sxy, mut syy, mut sxz, mut syz) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for p in ring {
        let (x, y) = (p[0] - m[0], p[1] - m[1]);
        let z = x * x + y * y;
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sxz += x * z;
        syz += y * z;
    }
    let det = sxx * syy - sxy * sxy;
    if det.abs() < 1e-12 {
        return None;
    }
    // Centre offset (a, b) from the mean solves [sxx sxy; sxy syy]·(2a, 2b) = (sxz, syz).
    let a = (sxz * syy - syz * sxy) / det / 2.0;
    let b = (syz * sxx - sxz * sxy) / det / 2.0;
    let c = [m[0] + a, m[1] + b];
    let radii: Vec<f64> = ring.iter().map(|p| dist(*p, c)).collect();
    let r = radii.iter().sum::<f64>() / n;
    let worst = radii.iter().map(|q| (q - r).abs()).fold(0.0, f64::max);
    // Facets within the export tolerance, and no long chord (a rounded slot's ends are arcs, not a circle).
    let longest = (0..ring.len()).map(|i| dist(ring[i], ring[(i + 1) % ring.len()])).fold(0.0, f64::max);
    // The facets' corners lie on the true circle, chords inside it: its radius is the largest.
    let outer = radii.iter().fold(0.0, |a: f64, q| a.max(*q));
    (worst <= 0.01 + 1e-3 * r && longest < r * 0.8).then_some((c, outer))
}

fn fmt_num(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// The flat pattern as a DXF (R12, millimetres): layer CUT (the outline and
/// every hole; round holes as CIRCLEs), BEND_UP and BEND_DOWN (bend centre
/// lines, as seen from the drawing's side), FORM (louvres for the forming
/// tool, not cut) and BEND_NOTES (each bend's direction, angle and radius).
pub fn dxf(f: &FlatPattern) -> String {
    let mut out = String::new();
    let mut pair = |code: i32, value: &str| {
        let _ = writeln!(out, "{code}\n{value}");
    };
    pair(999, &format!("Forma flat pattern: {} — {} mm {}, K {}, {} bends; CUT = laser, BEND_UP / BEND_DOWN = bend lines seen from this side", f.part, fmt_num(f.thickness), f.material, fmt_num(f.k), f.bends.len()));
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    pair(9, "$INSUNITS");
    pair(70, "4");
    pair(9, "$EXTMIN");
    pair(10, "0");
    pair(20, "0");
    pair(9, "$EXTMAX");
    pair(10, &fmt_num(f.size[0]));
    pair(20, &fmt_num(f.size[1]));
    pair(0, "ENDSEC");
    // Layers with colours a CAM program shows apart: cut white, up blue, down red, forms green.
    pair(0, "SECTION");
    pair(2, "TABLES");
    pair(0, "TABLE");
    pair(2, "LAYER");
    pair(70, "5");
    for (name, colour, line) in [("CUT", 7, "CONTINUOUS"), ("BEND_UP", 5, "CONTINUOUS"), ("BEND_DOWN", 1, "CONTINUOUS"), ("FORM", 3, "CONTINUOUS"), ("BEND_NOTES", 8, "CONTINUOUS")] {
        pair(0, "LAYER");
        pair(2, name);
        pair(70, "0");
        pair(62, &colour.to_string());
        pair(6, line);
    }
    pair(0, "ENDTAB");
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for ring in &f.loops {
        if let Some((c, r)) = as_circle(ring) {
            pair(0, "CIRCLE");
            pair(8, "CUT");
            pair(10, &fmt_num(c[0]));
            pair(20, &fmt_num(c[1]));
            pair(40, &fmt_num(r));
            continue;
        }
        pair(0, "POLYLINE");
        pair(8, "CUT");
        pair(66, "1");
        pair(70, "1");
        for [x, y] in ring {
            pair(0, "VERTEX");
            pair(8, "CUT");
            pair(10, &fmt_num(*x));
            pair(20, &fmt_num(*y));
        }
        pair(0, "SEQEND");
    }
    for b in &f.bends {
        let layer = if b.dir == "up" { "BEND_UP" } else { "BEND_DOWN" };
        pair(0, "LINE");
        pair(8, layer);
        pair(10, &fmt_num(b.line[0][0]));
        pair(20, &fmt_num(b.line[0][1]));
        pair(11, &fmt_num(b.line[1][0]));
        pair(21, &fmt_num(b.line[1][1]));
        let mid = [(b.line[0][0] + b.line[1][0]) / 2.0, (b.line[0][1] + b.line[1][1]) / 2.0];
        let along = (b.line[1][1] - b.line[0][1]).atan2(b.line[1][0] - b.line[0][0]).to_degrees();
        // Text reads left to right, along the bend line.
        let along = if along > 90.0 + 1e-9 { along - 180.0 } else if along <= -90.0 + 1e-9 { along + 180.0 } else { along };
        let height = (f.thickness * 2.0).clamp(1.5, 5.0);
        pair(0, "TEXT");
        pair(8, "BEND_NOTES");
        pair(10, &fmt_num(mid[0]));
        pair(20, &fmt_num(mid[1] + 0.2 * height));
        pair(40, &fmt_num(height));
        pair(1, &format!("{} {} {}%%d R{}", b.id, b.dir.to_uppercase(), fmt_num(b.angle), fmt_num(b.radius)));
        pair(50, &fmt_num(along));
        pair(72, "1");
        pair(11, &fmt_num(mid[0]));
        pair(21, &fmt_num(mid[1] + 0.2 * height));
    }
    for form in &f.forms {
        pair(0, "POLYLINE");
        pair(8, "FORM");
        pair(66, "1");
        pair(70, "1");
        for [x, y] in &form.outline {
            pair(0, "VERTEX");
            pair(8, "FORM");
            pair(10, &fmt_num(*x));
            pair(20, &fmt_num(*y));
        }
        pair(0, "SEQEND");
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    out
}

/// The flat pattern as an SVG (y up, millimetres): the blank, bend lines
/// dashed (up blue, down red) with their notes, louvres green.
pub fn svg(f: &FlatPattern) -> String {
    let m = 6.0 + f.thickness * 4.0;
    let [w, h] = f.size;
    let y = |v: f64| fmt_num(h - v);
    let mut path = String::new();
    for ring in &f.loops {
        for (i, p) in ring.iter().enumerate() {
            let _ = write!(path, "{}{} {} ", if i == 0 { "M" } else { "L" }, fmt_num(p[0]), y(p[1]));
        }
        path.push_str("Z ");
    }
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}" width="{}mm" height="{}mm" font-family="system-ui, sans-serif">"#,
        fmt_num(-m),
        fmt_num(-m),
        fmt_num(w + 2.0 * m),
        fmt_num(h + 2.0 * m),
        fmt_num(w + 2.0 * m),
        fmt_num(h + 2.0 * m)
    );
    let _ = write!(s, r##"<title>{} flat pattern</title><path d="{}" fill="#dfe3e8" fill-rule="evenodd" stroke="#1d2329" stroke-width="{}"/>"##, xml(&f.part), path.trim_end(), fmt_num((f.size[0].max(f.size[1]) / 600.0).max(0.15)));
    let text = (f.thickness * 1.6).clamp(1.6, 4.0);
    for b in &f.bends {
        let colour = if b.dir == "up" { "#1f6feb" } else { "#d1242f" };
        let dash = if b.dir == "up" { "3 1.5" } else { "3 1 0.8 1" };
        let _ = write!(
            s,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{colour}" stroke-width="{}" stroke-dasharray="{dash}"/>"#,
            fmt_num(b.line[0][0]),
            y(b.line[0][1]),
            fmt_num(b.line[1][0]),
            y(b.line[1][1]),
            fmt_num(text / 8.0)
        );
        let mid = [(b.line[0][0] + b.line[1][0]) / 2.0, (b.line[0][1] + b.line[1][1]) / 2.0];
        let along = -(b.line[1][1] - b.line[0][1]).atan2(b.line[1][0] - b.line[0][0]).to_degrees();
        let along = if along > 90.0 { along - 180.0 } else if along < -90.0 { along + 180.0 } else { along };
        let _ = write!(
            s,
            r#"<text x="{x}" y="{yy}" font-size="{}" fill="{colour}" text-anchor="middle" transform="rotate({} {x} {yy})" dy="-0.4">{} {} {}° R{}</text>"#,
            fmt_num(text),
            fmt_num(along),
            b.id,
            b.dir.to_uppercase(),
            fmt_num(b.angle),
            fmt_num(b.radius),
            x = fmt_num(mid[0]),
            yy = y(mid[1]),
        );
    }
    for form in &f.forms {
        let pts: Vec<String> = form.outline.iter().map(|p| format!("{},{}", fmt_num(p[0]), y(p[1]))).collect();
        let _ = write!(s, r##"<polygon points="{}" fill="none" stroke="#1a7f37" stroke-width="{}"/>"##, pts.join(" "), fmt_num(text / 8.0));
    }
    s.push_str("</svg>");
    s
}

/// The bend table as CSV, in the suggested press-brake order.
pub fn bend_table_csv(f: &FlatPattern) -> String {
    let mut out = format!(
        "# {} — {} mm {}, flat blank {} × {} mm\nOrder,Bend,From,To,Direction,Angle deg,Inside radius mm,K-factor,Bend allowance mm,Bend deduction mm,Bend line length mm,Line from x,Line from y,Line to x,Line to y\n",
        f.part,
        fmt_num(f.thickness),
        f.material,
        fmt_num(f.size[0]),
        fmt_num(f.size[1])
    );
    let quoted = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut bends: Vec<&Bend> = f.bends.iter().collect();
    bends.sort_by_key(|b| b.order);
    for b in bends {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            b.order,
            b.id,
            quoted(&b.from),
            quoted(&b.to),
            if b.hem { format!("{} (hem)", b.dir) } else { b.dir.clone() },
            fmt_num(b.angle),
            fmt_num(b.radius),
            fmt_num(b.k),
            fmt_num(b.allowance),
            fmt_num(b.deduction),
            fmt_num(b.length),
            fmt_num(b.line[0][0]),
            fmt_num(b.line[0][1]),
            fmt_num(b.line[1][0]),
            fmt_num(b.line[1][1]),
        );
    }
    out
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ShapeProgram;

    fn program(source: &str) -> ShapeProgram {
        ShapeProgram {
            name: "Test".into(),
            description: String::new(),
            source: source.into(),
            params: Vec::new(),
            display_tolerance: None,
            levels: Vec::new(),
            uses: Default::default(),
            finishes: Vec::new(),
            inputs: String::new(),
        }
    }

    /// Flat length by hand: outside dimensions less one bend deduction per bend.
    fn bd(angle: f64, r: f64, t: f64, k: f64) -> f64 {
        let ossb = if angle <= 90.0 { (r + t) * (angle.to_radians() / 2.0).tan() } else { r + t };
        2.0 * ossb - angle.to_radians() * (r + k * t)
    }

    #[test]
    fn a_channel_unfolds_to_the_hand_calculation() {
        // A U channel, 1 mm 5052 at R1: 60 wide outside, 20 high walls, 100 long.
        let p = program(
            r#"const s = sheet({ width: 60, depth: 100 }, { thickness: 1, material: "Aluminium 5052-H32" });
               s.flange("-x", { length: 20 }); s.flange("+x", { length: 20 });
               s.hole(30, 50, 5);
               return [s.part("Channel")];"#,
        );
        let f = flat_pattern(&p, Some("Channel"), 0.01).unwrap();
        // R/T = 1 on a soft alloy: K 0.40 from the table.
        assert_eq!(f.k, 0.4);
        let hand = 20.0 + 60.0 + 20.0 - 2.0 * bd(90.0, 1.0, 1.0, 0.4);
        assert!((f.size[0] - hand).abs() < 1e-3, "flat {} vs hand {hand}", f.size[0]);
        assert!((f.size[1] - 100.0).abs() < 1e-6);
        assert_eq!(f.bends.len(), 2);
        let b = &f.bends[0];
        assert!((b.allowance - std::f64::consts::FRAC_PI_2 * 1.4).abs() < 1e-3);
        assert!((b.deduction - bd(90.0, 1.0, 1.0, 0.4)).abs() < 1e-3);
        assert_eq!(f.errors(), 0, "{:?}", f.checks);
        // Bend lines run the full 100 mm, BA/2 in from the strip's edge.
        assert!((b.length - 100.0).abs() < 1e-6);
        let dxf = dxf(&f);
        assert!(dxf.contains("BEND_UP") && dxf.contains("CUT"));
        // The hole leaves as a true circle for the laser.
        assert!(dxf.contains("CIRCLE\n8\nCUT\n10\n"), "{dxf}");
        assert!(dxf.contains("\n40\n2.5\n"), "radius 2.5");
    }

    #[test]
    fn an_l_bracket_from_an_open_profile() {
        // 2 mm steel, legs 40 and 30 outside, 25 wide: the profile traces the bottom (outside) face.
        let p = program(
            r#"const s = sheetProfile([[0, 30], [0, 0], [40, 0]], 25, { thickness: 2, material: "Steel DC01 (cold rolled)" });
               return [s.part("Bracket")];"#,
        );
        let f = flat_pattern(&p, None, 0.01).unwrap();
        // R = T = 2 on DC01: R/T 1, medium: K 0.43; the turn at (0, 0) is a left turn → up.
        let hand = 40.0 + 30.0 - bd(90.0, 2.0, 2.0, 0.43);
        assert!((f.size[0] - hand).abs() < 1e-3, "flat {} vs hand {hand}", f.size[0]);
        assert_eq!(f.bends[0].dir, "up");
        // The bent part spans the profile.
        let e = evaluated_at(&p, 0.01).unwrap();
        assert!((e.bounds_max[0] - 40.0).abs() < 1e-3 && (e.bounds_max[2] - 30.0).abs() < 1e-3, "{:?} {:?}", e.bounds_min, e.bounds_max);
    }

    #[test]
    fn checks_catch_short_flanges_holes_at_bends_and_missing_reliefs() {
        let p = program(
            r#"const s = sheet({ width: 50, depth: 40 }, { thickness: 1.5, material: "304" });
               const f = s.flange("-y", { length: 4 });
               s.hole(25, 3, 3);
               s.flange("+y", { length: 15, inset: 10, relief: "none" });
               return [s.part("Plate")];"#,
        );
        let f = flat_pattern(&p, None, 0.01).unwrap();
        let codes: Vec<&str> = f.checks.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"FLANGE_TOO_SHORT"), "{codes:?}");
        assert!(codes.contains(&"HOLE_NEAR_BEND") || codes.contains(&"CUT_IN_BEND"), "{codes:?}");
        assert!(codes.contains(&"NO_BEND_RELIEF"), "{codes:?}");
    }

    #[test]
    fn a_closed_box_corner_clears_folded_and_flat() {
        let p = program(
            r#"const s = sheet({ width: 80, depth: 60 }, { thickness: 1, material: "5052" });
               for (const e of ["-y", "+x", "+y", "-x"]) s.flange(e, { length: 25 });
               return [s.part("Tray")];"#,
        );
        let f = flat_pattern(&p, None, 0.01).unwrap();
        assert_eq!(f.bends.len(), 4);
        assert_eq!(f.errors(), 0, "{:?}", f.checks);
        // Each overall flat dimension: outside sizes less two deductions.
        let d = bd(90.0, 1.0, 1.0, 0.4);
        assert!((f.size[0] - (25.0 + 80.0 + 25.0 - 2.0 * d)).abs() < 1e-3, "{:?}", f.size);
        assert!((f.size[1] - (25.0 + 60.0 + 25.0 - 2.0 * d)).abs() < 1e-3, "{:?}", f.size);
        // Bent, it is 80 × 60 × 25 outside.
        let e = evaluated_at(&p, 0.01).unwrap();
        let size: Vec<f64> = (0..3).map(|k| e.bounds_max[k] - e.bounds_min[k]).collect();
        assert!((size[0] - 80.0).abs() < 1e-3 && (size[1] - 60.0).abs() < 1e-3 && (size[2] - 25.0).abs() < 1e-3, "{size:?}");
    }

    #[test]
    fn hems_jogs_and_louvres() {
        let p = program(
            r#"const s = sheet({ width: 100, depth: 50 }, { thickness: 1, material: "DX51D" });
               s.hem("-y", { length: 6 });
               s.jog("+y", { offset: 5, length: 15 });
               s.louvre(50, 25, { length: 30, width: 5, height: 2.5 });
               return [s.part("Panel")];"#,
        );
        let f = flat_pattern(&p, None, 0.01).unwrap();
        assert_eq!(f.bends.len(), 3);
        assert!(f.bends.iter().any(|b| b.hem && b.angle == 180.0));
        assert_eq!(f.forms.len(), 1);
        // Closed hem with K 0.5: the hem adds its length less 0.43·T (≈ T(2 − π/2)).
        let hem = 6.0 - (2.0 * 1.01 - std::f64::consts::PI * (0.01 + 0.5));
        let riser = 6.0;
        let jog = riser - bd(90.0, 1.0, 1.0, 0.43) + 15.0 - bd(90.0, 1.0, 1.0, 0.43);
        let hand = 50.0 + hem + jog;
        assert!((f.size[1] - hand).abs() < 1e-3, "flat {} vs hand {hand}", f.size[1]);
        assert!(f.loops.len() == 1, "louvres are formed, not cut");
    }

    #[test]
    fn the_exact_kernel_bends_it_the_same_way() {
        // The STEP export runs the program on OpenCascade: same bends, same place.
        let src = r#"const s = sheet({ width: 50, depth: 30 }, { thickness: 1.5, material: "5052" });
               const w = s.flange("-y", { length: 18 }); w.flange("tip", { length: 8, dir: "in" });
               s.flange("+x", { length: 12, angle: 60, dir: "down", inset: 3 });
               s.hole(25, 15, 6);
               return [s.part("Bracket")];"#;
        let p = program(src);
        let mesh = evaluated_at(&p, 0.01).unwrap();
        let exact = crate::exact::evaluate_exact("Test", src, &[], 0.01).unwrap();
        let a = exact.arena;
        let b = a.bounds(exact.parts[0].shape).unwrap();
        for k in 0..3 {
            assert!((b[k] - mesh.bounds_min[k]).abs() < 0.02 && (b[k + 3] - mesh.bounds_max[k]).abs() < 0.02, "axis {k}: exact {b:?} mesh {:?} {:?}", mesh.bounds_min, mesh.bounds_max);
        }
        let v = a.volume(exact.parts[0].shape);
        assert!((v - mesh.volume).abs() / v < 2e-3, "exact {v} mesh {}", mesh.volume);
        assert!(exact.faceted.is_empty(), "{:?}", exact.faceted);
    }
}
