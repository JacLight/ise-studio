//! EasyEDA documents.
//!
//! Standard edition (JSON, `docType` 3 for a PCB, 1 for a schematic, as the
//! editor exports them, optionally wrapped in `{"result": {"dataStr": …}}`):
//! every shape is a string of `~`-separated fields, a part (`LIB`) carrying
//! its sub-shapes after `#@$`. Units are 10 mil (0.254 mm), y down; pad and
//! part rotations are degrees clockwise as seen.
//! - PCB: `TRACK~width~layer~net~points~id`, `PAD~shape~x~y~w~h~layer~net~number~holeR~points~rot~id~holeLength~holePoints~plated`,
//!   `VIA~x~y~diameter~net~holeR~id`, `HOLE~x~y~radius~id`, `COPPERAREA~width~layer~net~path~clearance~fill~id~thermal`,
//!   `ARC~width~layer~net~path~…` and `CIRCLE~cx~cy~r~width~layer~id`; layer 10 is the board outline, 1/2 top/bottom,
//!   21 onwards inner, 3/4 silkscreen, 11 all copper layers.
//! - Schematic: `LIB~x~y~attrs~rot~…` with `P` pins (`^^`-separated parts: the connection point, the pin line,
//!   name and number), body graphics (`R`, `PL`, `PG`, `E`, `A`, `T`), `W` wires, `N` net labels, `F` net flags
//!   (power ports), `J` junctions, `O` no-connect flags.
//!
//! Pro edition (`.epro` zip, or a `.epcb` document): JSON lines, one record
//! per line (`["LINE", id, locked, net, layer, x1, y1, x2, y2, width]`,
//! `["VIA", id, locked, net, rule, x, y, drill, diameter]`,
//! `["PAD", id, locked, net, layer, number, x, y, rot, hole, shape]`,
//! `["COMPONENT", id, locked, layer, x, y, rot, attrs]`, `["ATTR", id, locked,
//! parent, layer, x, y, key, value]`, `["PAD_NET", component, pad, net]`,
//! `["POLY"|"FILL"|"POUR", id, locked, net, layer, width, path]`), mils, y up;
//! a component's pads come from its footprint document (`FOOTPRINT/<uuid>.efoo`)
//! by its `Footprint` attribute. Pro schematics are not read yet.

use super::build::{self, BoardIn, PadIn, PartIn};
use super::sch::{self, Draft, DraftSymbol};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{self, P};
use crate::kicad::{KVia, KZone};
use crate::schematic::LabelKind;
use crate::symbols::{Dir, Fill, Gfx, PinDef, PinType};
use crate::{PadConnection, Side};
use de_core::Value;
use std::collections::BTreeMap;

const U: f64 = 0.254;

/// The document inside an export (the `dataStr` wrapper or the document itself).
pub fn document(v: &Value) -> Option<&Value> {
    if let Some(d) = v.get("result").and_then(|r| r.get("dataStr")) {
        return Some(d);
    }
    if let Some(d) = v.get("dataStr") {
        return Some(d);
    }
    v.get("head").map(|_| v)
}

fn nf(s: &str) -> f64 {
    s.trim().parse().unwrap_or(0.0)
}

/// A point in Forma terms from EasyEDA standard units (y down).
fn sp(x: f64, y: f64) -> P {
    [x * U, -y * U]
}

fn points(s: &str) -> Vec<P> {
    let v: Vec<f64> = s.split([' ', ',']).filter(|x| !x.is_empty()).map(nf).collect();
    v.chunks(2).filter(|c| c.len() == 2).map(|c| sp(c[0], c[1])).collect()
}

/// An SVG path (M, L, H, V, A, Z; absolute) as polylines in EasyEDA units, flipped to Forma.
fn svg_path(d: &str) -> Vec<Vec<P>> {
    let mut toks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() {
            if !cur.trim().is_empty() {
                toks.push(cur.trim().to_string());
            }
            toks.push(ch.to_string());
            cur.clear();
        } else if ch == ',' || ch.is_whitespace() {
            if !cur.trim().is_empty() {
                toks.push(cur.trim().to_string());
            }
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        toks.push(cur.trim().to_string());
    }
    let mut out: Vec<Vec<P>> = Vec::new();
    let mut line: Vec<P> = Vec::new();
    let mut pos = [0.0, 0.0];
    let mut cmd = 'M';
    let mut i = 0;
    let num = |i: &mut usize| -> f64 {
        let v = toks.get(*i).map(|t| nf(t)).unwrap_or(0.0);
        *i += 1;
        v
    };
    while i < toks.len() {
        if toks[i].len() == 1 && toks[i].chars().next().unwrap().is_ascii_alphabetic() {
            cmd = toks[i].chars().next().unwrap().to_ascii_uppercase();
            i += 1;
            if cmd == 'Z' {
                if let Some(f) = line.first().copied() {
                    line.push(f);
                }
                if line.len() > 1 {
                    out.push(std::mem::take(&mut line));
                }
                continue;
            }
        }
        match cmd {
            'M' => {
                if line.len() > 1 {
                    out.push(std::mem::take(&mut line));
                }
                line.clear();
                pos = [num(&mut i), num(&mut i)];
                line.push(pos);
                cmd = 'L';
            }
            'L' => {
                pos = [num(&mut i), num(&mut i)];
                line.push(pos);
            }
            'H' => {
                pos[0] = num(&mut i);
                line.push(pos);
            }
            'V' => {
                pos[1] = num(&mut i);
                line.push(pos);
            }
            'A' => {
                let (rx, _ry, _rot, large, sweep, x, y) = (num(&mut i), num(&mut i), num(&mut i), num(&mut i), num(&mut i), num(&mut i), num(&mut i));
                let end = [x, y];
                // Centre from the SVG endpoint form (circular arcs).
                let (a, b) = (pos, end);
                let chord = geom::dist(a, b);
                if chord < 1e-9 || rx <= 0.0 {
                    line.push(end);
                    pos = end;
                    continue;
                }
                let r = rx.max(chord / 2.0);
                let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let h = (r * r - chord * chord / 4.0).max(0.0).sqrt();
                let dir = [(b[0] - a[0]) / chord, (b[1] - a[1]) / chord];
                let n = [-dir[1], dir[0]];
                let sgn = if (large != 0.0) == (sweep != 0.0) { -1.0 } else { 1.0 };
                let c = [mid[0] + n[0] * h * sgn, mid[1] + n[1] * h * sgn];
                // SVG sweep=1 is clockwise on screen (y down) = increasing angle in y-down maths.
                let pts = build::arc_between(c, a, b, Some(sweep != 0.0));
                line.extend(pts.into_iter().skip(1));
                pos = end;
            }
            _ => {
                i += 1;
            }
        }
    }
    if line.len() > 1 {
        out.push(line);
    }
    out.into_iter().map(|l| l.into_iter().map(|q| sp(q[0], q[1])).collect()).collect()
}

fn std_layer(l: &str, b: &mut BoardIn) -> Option<String> {
    match l.trim() {
        "1" => Some("top".into()),
        "2" => Some("bottom".into()),
        "21" => Some("in1".into()),
        "22" => Some("in2".into()),
        x if x.parse::<u32>().is_ok_and(|n| (23..=50).contains(&n)) => {
            b.note("copper on inner layers past the second was left out");
            None
        }
        _ => None,
    }
}

fn shape_std(s: &str) -> PadShape {
    match s {
        "ELLIPSE" => PadShape::Circle,
        "OVAL" => PadShape::Oval,
        "POLYGON" => PadShape::Custom,
        _ => PadShape::Rect,
    }
}

fn std_pad(f: &[&str], b: &mut BoardIn) -> Option<PadIn> {
    if f.len() < 10 {
        return None;
    }
    let at = sp(nf(f[2]), nf(f[3]));
    let size = [nf(f[4]) * U, nf(f[5]) * U];
    let mut p = PadIn::new(f.get(8).copied().unwrap_or("").to_string(), at, size);
    let hole_r = nf(f[9]) * U;
    let multi = f[6].trim() == "11";
    p.shape = build::fix_shape(shape_std(f[1]), size);
    p.rotation = -nf(f.get(11).copied().unwrap_or("0"));
    if p.shape == PadShape::Custom {
        p.polygon = points(f.get(10).copied().unwrap_or(""));
        if p.polygon.len() < 3 {
            p.shape = PadShape::Rect;
        }
    }
    if hole_r > 0.0 {
        let hole_len = nf(f.get(13).copied().unwrap_or("0")) * U;
        let d = 2.0 * hole_r;
        p.drill = Some(if hole_len > d { [hole_len, d] } else { [d, d] });
        let plated = f.get(15).map(|x| x.trim() != "N").unwrap_or(true);
        p.kind = if plated { PadKind::Tht } else { PadKind::Npth };
        if p.kind == PadKind::Tht && p.size[0].min(p.size[1]) <= d {
            p.size = [p.size[0].max(d + 0.3), p.size[1].max(d + 0.3)];
        }
    } else if !multi && f[6].trim() != "1" && f[6].trim() != "2" {
        return None;
    }
    let net = f[7].trim();
    p.net = (!net.is_empty()).then(|| net.to_string());
    let _ = b;
    Some(p)
}

/// One standard-edition shape string onto the board (`part` when inside a LIB).
fn std_shape(s: &str, b: &mut BoardIn, part: Option<&mut PartIn>) {
    let f: Vec<&str> = s.split('~').collect();
    match f[0] {
        "TRACK" if f.len() >= 5 => {
            let w = nf(f[1]) * U;
            let pts = points(f[4]);
            match f[2].trim() {
                "10" => b.edges.push(pts),
                "3" | "4" => {
                    if let Some(p) = part {
                        p.silk.push(pts);
                    }
                }
                l => {
                    if let Some(layer) = std_layer(l, b) {
                        let net = f[3].trim();
                        b.polyline((!net.is_empty()).then(|| net.to_string()), &layer, w, &pts);
                    }
                }
            }
        }
        "ARC" if f.len() >= 5 => {
            let w = nf(f[1]) * U;
            for pts in svg_path(f[4]) {
                match f[2].trim() {
                    "10" => b.edges.push(pts),
                    "3" | "4" => {}
                    l => {
                        if let Some(layer) = std_layer(l, b) {
                            let net = f[3].trim();
                            b.polyline((!net.is_empty()).then(|| net.to_string()), &layer, w, &pts);
                        }
                    }
                }
            }
        }
        "CIRCLE" if f.len() >= 6 && f[5].trim() == "10" => {
            let mut c = build::circle(sp(nf(f[1]), nf(f[2])), nf(f[3]) * U);
            c.push(c[0]);
            b.edges.push(c);
        }
        "PAD" => {
            if let Some(p) = std_pad(&f, b) {
                match part {
                    Some(pt) => pt.pads.push(p),
                    None => {
                        let at = p.at;
                        b.parts.push(PartIn { reference: format!("PAD{}", b.parts.len() + 1), value: String::new(), footprint: "free_pad".into(), at, pads: vec![p], ..Default::default() });
                    }
                }
            }
        }
        "VIA" if f.len() >= 6 => {
            let net = f[4].trim();
            b.vias.push(KVia { net: (!net.is_empty()).then(|| net.to_string()), at: sp(nf(f[1]), nf(f[2])), drill: 2.0 * nf(f[5]) * U, diameter: nf(f[3]) * U });
        }
        "HOLE" if f.len() >= 4 => {
            let at = sp(nf(f[1]), nf(f[2]));
            let d = 2.0 * nf(f[3]) * U;
            let mut p = PadIn::new("", at, [d, d]);
            p.kind = PadKind::Npth;
            p.shape = PadShape::Circle;
            p.drill = Some([d, d]);
            match part {
                Some(pt) => pt.pads.push(p),
                None => {
                    let n = b.parts.iter().filter(|x| x.reference.starts_with('H')).count() + 1;
                    b.parts.push(PartIn { reference: format!("H{n}"), value: "hole".into(), footprint: "hole".into(), at, pads: vec![p], ..Default::default() });
                }
            }
        }
        "COPPERAREA" if f.len() >= 5 => {
            let Some(layer) = std_layer(f[2], b) else { return };
            let net = f[3].trim().to_string();
            for pts in svg_path(f[4]) {
                let mut o = pts;
                if o.len() > 1 && geom::dist(o[0], *o.last().unwrap()) < 1e-9 {
                    o.pop();
                }
                if o.len() >= 3 && !net.is_empty() {
                    b.zones.push(KZone { net: net.clone(), layer: layer.clone(), outline: o, clearance: f.get(5).map(|x| nf(x) * U).filter(|v| *v > 0.0), min_width: 0.25, priority: 0, connection: if f.get(8).is_some_and(|t| t.trim() == "direct") { PadConnection::Solid } else { PadConnection::Thermal }, thermal_gap: None, thermal_spoke: None });
                }
            }
        }
        "TEXT" if f.len() >= 11 => {
            if let Some(p) = part {
                match f[1] {
                    "P" => p.reference = f[10].trim().to_string(),
                    "N" => p.value = f[10].trim().to_string(),
                    _ => {}
                }
            }
        }
        "SOLIDREGION" | "SVGNODE" | "DIMENSION" | "RECT" => {}
        _ => {}
    }
}

fn attrs(s: &str) -> BTreeMap<String, String> {
    let v: Vec<&str> = s.split('`').collect();
    v.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].to_string(), c[1].to_string())).collect()
}

pub fn read_std_board(text: &str) -> Result<BoardIn, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not EasyEDA JSON: {e}"))?;
    let doc = document(&v).ok_or("not an EasyEDA document (no head)")?;
    let shapes = doc.get("shape").and_then(|s| s.as_array()).ok_or("the EasyEDA document has no shapes")?;
    let mut b = BoardIn { title: doc.get("head").and_then(|h| h.get("c_para")).and_then(|c| c.get("name")).and_then(|x| x.as_str()).map(str::to_string), ..Default::default() };
    let layers = doc.get("layers").and_then(|l| l.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).filter(|x| { let id = x.split('~').next().unwrap_or(""); (id == "1" || id == "2" || id.parse::<u32>().is_ok_and(|n| (21..=50).contains(&n))) && x.split('~').nth(3).is_some_and(|v| v == "true") }).count()).unwrap_or(2);
    b.source_copper_layers = layers.max(2);
    for s in shapes.iter().filter_map(|s| s.as_str()) {
        let mut parts = s.split("#@$");
        let head = parts.next().unwrap_or("");
        let f: Vec<&str> = head.split('~').collect();
        if f[0] == "LIB" && f.len() >= 5 {
            let a = attrs(f[3]);
            let rot = -nf(f[4]);
            let mut part = PartIn { footprint: a.get("package").cloned().unwrap_or_default(), at: sp(nf(f[1]), nf(f[2])), rotation: rot, ..Default::default() };
            let mut bottom = false;
            for sub in parts {
                let sf: Vec<&str> = sub.split('~').collect();
                if sf[0] == "PAD" && sf.get(6).is_some_and(|l| l.trim() == "2") {
                    bottom = true;
                }
                std_shape(sub, &mut b, Some(&mut part));
            }
            if part.pads.iter().all(|p| p.kind != PadKind::Smd) {
                bottom = false;
            }
            part.side = if bottom { Side::Bottom } else { Side::Top };
            if part.reference.is_empty() {
                part.reference = format!("U{}", b.parts.len() + 1);
            }
            for k in ["Manufacturer Part", "MPN", "Manufacturer Part Number"] {
                if let Some(m) = a.get(k) {
                    part.mpn = Some(m.clone());
                }
            }
            if let Some(m) = a.get("Manufacturer") {
                part.manufacturer = Some(m.clone());
            }
            b.parts.push(part);
        } else {
            std_shape(head, &mut b, None);
        }
    }
    Ok(b)
}

// ---- Standard schematics -------------------------------------------------------------------

fn sym_gfx(f: &[&str]) -> Option<Gfx> {
    match f[0] {
        "R" if f.len() >= 7 => {
            let (x, y, w, h) = (nf(f[1]), nf(f[2]), nf(f[5]), nf(f[6]));
            Some(Gfx::Poly { pts: vec![sp(x, y), sp(x + w, y), sp(x + w, y + h), sp(x, y + h)], fill: Fill::Body })
        }
        "PL" if f.len() >= 2 => Some(Gfx::Line { pts: points(f[1]), width: 0.0 }),
        "PG" if f.len() >= 2 => Some(Gfx::Poly { pts: points(f[1]), fill: Fill::Outline }),
        "E" if f.len() >= 4 => Some(Gfx::Circle { c: sp(nf(f[1]), nf(f[2])), r: nf(f[3]) * U, fill: Fill::None }),
        "A" if f.len() >= 2 => svg_path(f[1]).into_iter().next().map(|pts| Gfx::Line { pts, width: 0.0 }),
        "T" if f.len() >= 13 && f[1] == "L" => Some(Gfx::Text { at: sp(nf(f[2]), nf(f[3])), text: f[12].to_string(), size: 1.27 }),
        _ => None,
    }
}

pub fn read_std_schematic(text: &str) -> Result<Draft, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not EasyEDA JSON: {e}"))?;
    let doc = document(&v).ok_or("not an EasyEDA document (no head)")?;
    // Several sheets come as `schematics: [{dataStr: …}]`; one sheet is the document itself.
    let sheets: Vec<&Value> = match doc.get("schematics").and_then(|s| s.as_array()) {
        Some(a) => a.iter().filter_map(|s| s.get("dataStr")).collect(),
        None => vec![doc],
    };
    // Forma coordinates are y up here (sp flips); the draft is told so.
    let mut d = Draft { y_up: true, ..Default::default() };
    for (si, sh) in sheets.iter().enumerate() {
        let sid = (si + 1).to_string();
        d.sheets.push((sid.clone(), sh.get("title").and_then(|t| t.as_str()).map(str::to_string).unwrap_or_else(|| format!("Sheet {}", si + 1))));
        for s in sh.get("shape").and_then(|s| s.as_array()).into_iter().flatten().filter_map(|s| s.as_str()) {
            let mut parts = s.split("#@$");
            let head = parts.next().unwrap_or("");
            let f: Vec<&str> = head.split('~').collect();
            match f[0] {
                "LIB" if f.len() >= 4 => {
                    let a = attrs(f[3]);
                    let at = sp(nf(f[1]), nf(f[2]));
                    let mut sym = DraftSymbol { sheet: sid.clone(), at, footprint: a.get("package").cloned().unwrap_or_default(), lib: format!("easyeda:{}", a.get("spicePre").cloned().unwrap_or_default()), unit: 1, ..Default::default() };
                    sym.value = a.get("Value").or(a.get("name")).cloned().unwrap_or_default();
                    for sub in parts {
                        let segs: Vec<&str> = sub.split("^^").collect();
                        let sf: Vec<&str> = segs[0].split('~').collect();
                        match sf[0] {
                            "P" if sf.len() >= 7 => {
                                let tip = sp(nf(sf[4]), nf(sf[5]));
                                // The pin line "M x y h len" (or v): its far end is the body.
                                let body = segs.get(2).and_then(|p| svg_path(p.split('~').next().unwrap_or("")).into_iter().next()).and_then(|l| l.into_iter().max_by(|a, b| geom::dist(*a, tip).total_cmp(&geom::dist(*b, tip)))).unwrap_or(tip);
                                let name = segs.get(3).and_then(|x| x.split('~').nth(4)).unwrap_or("").to_string();
                                let number = segs.get(4).and_then(|x| x.split('~').nth(4)).map(str::to_string).filter(|s| !s.is_empty()).unwrap_or_else(|| sf[3].to_string());
                                let len = geom::dist(body, tip);
                                d_pin(&mut sym, number, name, tip, body, len);
                            }
                            "T" if sf.len() >= 13 => match sf[1] {
                                "P" => sym.reference = sf[12].to_string(),
                                "N" => {
                                    if sym.value.is_empty() {
                                        sym.value = sf[12].to_string()
                                    }
                                }
                                _ => {
                                    if let Some(g) = sym_gfx(&sf) {
                                        sym.gfx.push(g);
                                    }
                                }
                            },
                            _ => {
                                if let Some(g) = sym_gfx(&sf) {
                                    sym.gfx.push(g);
                                }
                            }
                        }
                    }
                    if sym.reference.is_empty() {
                        sym.reference = format!("U{}", d.symbols.len() + 1);
                    }
                    d.symbols.push(sym);
                }
                "W" if f.len() >= 2 => {
                    let pts = points(f[1]);
                    if pts.len() >= 2 {
                        d.wires.push((sid.clone(), pts));
                    }
                }
                "N" if f.len() >= 6 => d.labels.push((sid.clone(), LabelKind::Local, f[5].to_string(), sp(nf(f[1]), nf(f[2])), (-nf(f[3])).rem_euclid(360.0))),
                "F" if f.len() >= 4 => {
                    let segs: Vec<&str> = s.split("^^").collect();
                    let dot = segs.get(1).map(|x| x.split('~').map(nf).collect::<Vec<_>>()).filter(|v| v.len() >= 2).map(|v| sp(v[0], v[1])).unwrap_or(sp(nf(f[2]), nf(f[3])));
                    let name = segs.get(2).and_then(|x| x.split('~').next()).unwrap_or("").to_string();
                    let kind_s = f[1].to_ascii_lowercase();
                    let kind = if kind_s.contains("gnd") || kind_s.contains("vcc") || kind_s.contains("netlabel") && crate::schematic::is_power_net(&name) { LabelKind::Power } else { LabelKind::Global };
                    let ang = if crate::schematic::is_ground(&name) { 270.0 } else { 90.0 };
                    if !name.is_empty() {
                        d.labels.push((sid.clone(), kind, name, dot, ang));
                    }
                }
                "J" if f.len() >= 3 => d.junctions.push((sid.clone(), sp(nf(f[1]), nf(f[2])))),
                "O" if f.len() >= 4 => {
                    // O~id~x~y or O~x~y~id: the first two numbers.
                    let nums: Vec<f64> = f[1..].iter().filter_map(|x| x.trim().parse::<f64>().ok()).collect();
                    if nums.len() >= 2 {
                        d.no_connects.push((sid.clone(), sp(nums[0], nums[1])));
                    }
                }
                "T" if f.len() >= 13 && f[1] == "L" => d.texts.push((sid.clone(), sp(nf(f[2]), nf(f[3])), f[12].to_string(), 1.8)),
                _ => {}
            }
        }
    }
    Ok(d)
}

fn d_pin(sym: &mut DraftSymbol, number: String, name: String, tip: P, body: P, len: f64) {
    sym.pins.push(PinDef { number, name: if name.is_empty() { "~".into() } else { name }, at: tip, side: if len > 1e-9 { sch::side_from(body, tip) } else { Dir::Left }, length: len, etype: PinType::Passive, hidden: false, unit: 0, shape: Default::default(), pads: vec![] });
}

// ---- Pro -----------------------------------------------------------------------------------

const MIL: f64 = 0.0254;

fn pro_lines(text: &str) -> Vec<Vec<Value>> {
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).filter_map(|v| v.as_array().cloned()).collect()
}

fn jf(v: &[Value], i: usize) -> f64 {
    v.get(i).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0)
}

fn js(v: &[Value], i: usize) -> String {
    v.get(i).and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string()))).unwrap_or_default()
}

fn pro_layer(id: f64) -> Option<&'static str> {
    match id as i64 {
        1 => Some("top"),
        2 => Some("bottom"),
        15 => Some("in1"),
        16 => Some("in2"),
        _ => None,
    }
}

/// A pad's shape record (["RECT", w, h, round%], ["ELLIPSE", w, h], ["OVAL", w, h], ["POLY", [x, y, …]]).
fn pro_pad(rec: &[Value], at: P, rot: f64) -> PadIn {
    let shape = rec.get(10).and_then(|s| s.as_array()).cloned().unwrap_or_default();
    let kind = shape.first().and_then(|x| x.as_str()).unwrap_or("RECT");
    let (w, h) = (jf(&shape, 1) * MIL, jf(&shape, 2) * MIL);
    let mut p = PadIn::new(js(rec, 5), at, [w.max(0.01), h.max(0.01)]);
    p.rotation = rot;
    p.shape = match kind {
        "ELLIPSE" => build::fix_shape(PadShape::Circle, [w, h]),
        "OVAL" => PadShape::Oval,
        "POLY" | "POLYGON" => PadShape::Custom,
        _ if jf(&shape, 3) > 0.0 => PadShape::RoundRect,
        _ => PadShape::Rect,
    };
    if p.shape == PadShape::RoundRect {
        p.corner = Some((jf(&shape, 3) / 200.0).clamp(0.0, 0.5));
    }
    if p.shape == PadShape::Custom {
        let pts: Vec<f64> = shape.get(1).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default();
        let poly: Vec<P> = pts.chunks(2).filter(|c| c.len() == 2).map(|c| geom::add(at, geom::rotate([c[0] * MIL, c[1] * MIL], rot))).collect();
        if poly.len() >= 3 {
            let bb = poly.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]);
            p.size = [bb[2] - bb[0], bb[3] - bb[1]];
            p.polygon = poly;
        } else {
            p.shape = PadShape::Rect;
        }
    }
    if let Some(hole) = rec.get(9).and_then(|h| h.as_array()) {
        let (hw, hh) = (jf(hole, 1) * MIL, jf(hole, 2) * MIL);
        if hw > 0.0 {
            p.drill = Some([hw, if hh > 0.0 { hh } else { hw }]);
            p.kind = PadKind::Tht;
        }
    }
    p
}

/// A Pro path ([x, y, "L", x, y, …, "ARC", angle, x, y, …]) as a polyline (mm).
fn pro_path(v: &Value) -> Vec<P> {
    let a = v.as_array().cloned().unwrap_or_default();
    let mut out: Vec<P> = Vec::new();
    let mut i = 0;
    let mut mode = "L";
    while i < a.len() {
        if let Some(s) = a[i].as_str() {
            mode = if s == "ARC" || s == "CARC" { "ARC" } else { "L" };
            i += 1;
            continue;
        }
        if mode == "ARC" {
            let ang = jf(&a, i);
            let end = [jf(&a, i + 1) * MIL, jf(&a, i + 2) * MIL];
            if let Some(start) = out.last().copied() {
                out.extend(build::arc_sweep(start, end, ang).into_iter().skip(1));
            }
            i += 3;
        } else {
            out.push([jf(&a, i) * MIL, jf(&a, i + 1) * MIL]);
            i += 2;
        }
    }
    out
}

fn pro_board(text: &str, footprints: &BTreeMap<String, String>) -> BoardIn {
    let mut b = BoardIn { source_copper_layers: 2, ..Default::default() };
    let recs = pro_lines(text);
    let mut comps: BTreeMap<String, PartIn> = BTreeMap::new();
    let mut comp_fp: BTreeMap<String, String> = BTreeMap::new();
    let mut pad_net: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut inner = 0;
    for r in &recs {
        let kind = r.first().and_then(|x| x.as_str()).unwrap_or("");
        match kind {
            "LAYER" if pro_layer(jf(r, 1)).is_some_and(|l| l.starts_with("in")) => inner += 1,
            "COMPONENT" => {
                let id = js(r, 1);
                let side = if jf(r, 3) as i64 == 2 { Side::Bottom } else { Side::Top };
                comps.insert(id, PartIn { at: [jf(r, 4) * MIL, jf(r, 5) * MIL], rotation: jf(r, 6), side, ..Default::default() });
            }
            "PAD_NET" => {
                pad_net.insert((js(r, 1), js(r, 2)), js(r, 3));
            }
            _ => {}
        }
    }
    b.source_copper_layers = 2 + inner;
    for r in &recs {
        let kind = r.first().and_then(|x| x.as_str()).unwrap_or("");
        match kind {
            "ATTR" => {
                let parent = js(r, 3);
                let (k, v) = (js(r, 7), js(r, 8));
                if let Some(c) = comps.get_mut(&parent) {
                    match k.as_str() {
                        "Designator" => c.reference = v,
                        "Value" | "Name" => {
                            if c.value.is_empty() || k == "Value" {
                                c.value = v
                            }
                        }
                        "Footprint" => {
                            comp_fp.insert(parent.clone(), v);
                        }
                        "Manufacturer Part" | "MPN" => c.mpn = Some(v),
                        "Manufacturer" => c.manufacturer = Some(v),
                        _ => {}
                    }
                }
            }
            "LINE" => {
                let net = js(r, 3);
                let (a, c) = ([jf(r, 5) * MIL, jf(r, 6) * MIL], [jf(r, 7) * MIL, jf(r, 8) * MIL]);
                match jf(r, 4) as i64 {
                    11 => b.edges.push(vec![a, c]),
                    l => {
                        if let Some(layer) = pro_layer(l as f64) {
                            b.seg((!net.is_empty()).then_some(net), layer, jf(r, 9) * MIL, a, c);
                        }
                    }
                }
            }
            "ARC" => {
                let net = js(r, 3);
                let pts = build::arc_sweep([jf(r, 5) * MIL, jf(r, 6) * MIL], [jf(r, 7) * MIL, jf(r, 8) * MIL], jf(r, 9));
                match jf(r, 4) as i64 {
                    11 => b.edges.push(pts),
                    l => {
                        if let Some(layer) = pro_layer(l as f64) {
                            b.polyline((!net.is_empty()).then_some(net), layer, jf(r, 10) * MIL, &pts);
                        }
                    }
                }
            }
            "VIA" => {
                let net = js(r, 3);
                b.vias.push(KVia { net: (!net.is_empty()).then_some(net), at: [jf(r, 5) * MIL, jf(r, 6) * MIL], drill: jf(r, 7) * MIL, diameter: jf(r, 8) * MIL });
            }
            "PAD" => {
                let at = [jf(r, 6) * MIL, jf(r, 7) * MIL];
                let mut p = pro_pad(r, at, jf(r, 8));
                let net = js(r, 3);
                p.net = (!net.is_empty()).then_some(net);
                if p.kind == PadKind::Smd && !matches!(jf(r, 4) as i64, 1 | 2) {
                    continue;
                }
                b.parts.push(PartIn { reference: format!("PAD{}", b.parts.len() + 1), footprint: "free_pad".into(), at, pads: vec![p], ..Default::default() });
            }
            "POLY" | "FILL" | "POUR" => {
                let net = js(r, 3);
                let layer = jf(r, 4) as i64;
                let path = r.get(6).cloned().unwrap_or(Value::Null);
                let mut pts = pro_path(&path);
                if layer == 11 {
                    if let Some(f) = pts.first().copied() {
                        pts.push(f);
                    }
                    b.edges.push(pts);
                } else if let Some(l) = pro_layer(layer as f64) {
                    if pts.len() >= 3 && !net.is_empty() && kind == "POUR" {
                        b.zones.push(KZone { net, layer: l.into(), outline: pts, clearance: None, min_width: 0.25, priority: 0, connection: PadConnection::Thermal, thermal_gap: None, thermal_spoke: None });
                    }
                }
            }
            _ => {}
        }
    }
    // Component pads from their footprint documents.
    for (id, mut c) in comps {
        let Some(fp_id) = comp_fp.get(&id) else {
            b.note(format!("{} names no footprint and was left out", c.reference));
            continue;
        };
        let Some(fp_text) = footprints.iter().find(|(k, _)| k.contains(fp_id.as_str())).map(|(_, v)| v) else {
            b.note(format!("{}: footprint {fp_id} is not in the project", c.reference));
            continue;
        };
        let bottom = c.side == Side::Bottom;
        let place = |q: P| -> P {
            let q = if bottom { [-q[0], q[1]] } else { q };
            geom::add(c.at, geom::rotate(q, c.rotation))
        };
        for r in pro_lines(fp_text) {
            match r.first().and_then(|x| x.as_str()) {
                Some("PAD") => {
                    let local = [jf(&r, 6) * MIL, jf(&r, 7) * MIL];
                    let lrot = jf(&r, 8);
                    let mut p = pro_pad(&r, [0.0, 0.0], 0.0);
                    p.at = place(local);
                    p.rotation = if bottom { c.rotation - lrot } else { c.rotation + lrot };
                    if p.shape == PadShape::Custom {
                        p.polygon = p.polygon.iter().map(|q| geom::add(p.at, geom::rotate(if bottom { [-q[0], q[1]] } else { *q }, if bottom { -lrot } else { lrot } + c.rotation))).collect();
                    }
                    p.net = pad_net.get(&(id.clone(), p.number.clone())).cloned();
                    c.pads.push(p);
                }
                Some("LINE") if matches!(jf(&r, 4) as i64, 3 | 4) => {
                    c.silk.push(vec![place([jf(&r, 5) * MIL, jf(&r, 6) * MIL]), place([jf(&r, 7) * MIL, jf(&r, 8) * MIL])]);
                }
                _ => {}
            }
        }
        c.footprint = footprints.keys().find(|k| k.contains(fp_id.as_str())).map(|k| k.rsplit('/').next().unwrap_or(k).trim_end_matches(".efoo").to_string()).unwrap_or_default();
        if c.reference.is_empty() {
            c.reference = format!("U{}", b.parts.len() + 1);
        }
        b.parts.push(c);
    }
    b
}

/// An `.epro` zip or a single Pro document: (board, schematic).
pub fn read_pro(bytes: &[u8]) -> Result<(Option<BoardIn>, Option<Draft>), String> {
    let text_of = |b: &[u8]| String::from_utf8_lossy(b).to_string();
    let files: BTreeMap<String, String> = if super::archive::is_zip(bytes) { super::archive::unzip(bytes)?.into_iter().map(|(k, v)| (k, text_of(&v))).collect() } else { [("document.epcb".to_string(), text_of(bytes))].into() };
    let footprints: BTreeMap<String, String> = files.iter().filter(|(k, _)| k.ends_with(".efoo")).map(|(k, v)| (k.clone(), v.clone())).collect();
    let pcb = files.iter().find(|(k, v)| k.ends_with(".epcb") || v.trim_start().starts_with("[\"DOCTYPE\",\"PCB\""));
    let board = pcb.map(|(_, t)| pro_board(t, &footprints));
    let has_sch = files.iter().any(|(k, v)| k.ends_with(".esch") || v.trim_start().starts_with("[\"DOCTYPE\",\"SCH\""));
    let mut board = board;
    if has_sch {
        if let Some(b) = board.as_mut() {
            b.note("EasyEDA Pro schematics are not read yet; the board was");
        } else {
            return Err("EasyEDA Pro schematics are not read yet (Pro boards and Standard schematics are)".into());
        }
    }
    let _ = sch::line;
    Ok((board, None))
}
