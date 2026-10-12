//! Allegro "Fabmaster" extracts (`.fab`, `.txt`): the `!`-separated report
//! Allegro's extract command writes for third-party tools. Each section is
//! a header line `A!FIELD!FIELD!…!` naming its columns, then data lines
//! `S!value!value!…!`; columns are found by name, so any extract that
//! carries them reads. Used: components (`REFDES`, `SYM_X`, `SYM_Y`,
//! `SYM_ROTATE`, `SYM_MIRROR`, `SYM_NAME`, `COMP_VALUE`/`COMP_DEVICE_TYPE`),
//! pins (`REFDES`, `PIN_NUMBER`, `PIN_X`, `PIN_Y`, `PAD_STACK_NAME`,
//! `NET_NAME`), pad stacks (`PAD_NAME`, `LAYER`, `PADSHAPE1`, `PADWIDTH`,
//! `PADHGHT`, a drill column), vias (`VIA_X`, `VIA_Y`, `PAD_STACK_NAME`,
//! `NET_NAME`) and graphics (`CLASS`, `SUBCLASS`, `GRAPHIC_DATA_NAME`,
//! `GRAPHIC_DATA_1…10`, `NET_NAME`): ETCH lines and arcs are copper, BOARD
//! GEOMETRY outlines the edge. Units from the `J!` line (mils by default).

use super::build::{self, BoardIn, PadIn, PartIn};
use crate::footprints::{PadKind, PadShape};
use crate::geom::P;
use crate::kicad::KVia;
use crate::Side;
use std::collections::BTreeMap;

struct Section {
    cols: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Section {
    fn col(&self, name: &str) -> Option<usize> {
        self.cols.iter().position(|c| c == name)
    }
    fn has(&self, names: &[&str]) -> bool {
        names.iter().all(|n| self.col(n).is_some())
    }
}

fn get<'a>(s: &Section, row: &'a [String], name: &str) -> &'a str {
    s.col(name).and_then(|i| row.get(i)).map(|x| x.trim()).unwrap_or("")
}

pub fn read(text: &str) -> Result<BoardIn, String> {
    let mut k = 0.0254;
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        let fields: Vec<String> = line.split('!').map(str::to_string).collect();
        match fields.first().map(String::as_str) {
            Some("J") => {
                let up = line.to_ascii_uppercase();
                if up.contains("MILLIMETER") || up.contains("!MM!") {
                    k = 1.0;
                } else if up.contains("INCHES") {
                    k = 25.4;
                } else if up.contains("MICRON") {
                    k = 0.001;
                }
            }
            Some("A") => sections.push(Section { cols: fields[1..].iter().map(|c| c.trim().to_ascii_uppercase()).collect(), rows: vec![] }),
            Some("S") => {
                if let Some(s) = sections.last_mut() {
                    s.rows.push(fields[1..].to_vec());
                }
            }
            _ => {}
        }
    }
    if sections.is_empty() {
        return Err("not a Fabmaster extract (no A! sections)".into());
    }
    let num = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
    let mut b = BoardIn::default();
    // Pad stacks: name → (shape, size on top/bottom/inner, drill).
    let mut stacks: BTreeMap<String, (PadShape, [f64; 2], f64, bool)> = BTreeMap::new();
    for s in sections.iter().filter(|s| s.has(&["PAD_NAME", "LAYER"])) {
        let drill_col = s.cols.iter().position(|c| c.contains("DRILL") || c.contains("HOLE"));
        for r in &s.rows {
            let name = get(s, r, "PAD_NAME").to_string();
            let layer = get(s, r, "LAYER").to_ascii_uppercase();
            let e = stacks.entry(name).or_insert((PadShape::Circle, [0.0, 0.0], 0.0, false));
            if let Some(d) = drill_col.and_then(|i| r.get(i)).map(|x| num(x) * k).filter(|d| *d > 0.0) {
                e.2 = d;
            }
            if layer.contains("DRILL") {
                let w = num(get(s, r, "PADWIDTH")) * k;
                if w > 0.0 {
                    e.2 = w;
                }
                continue;
            }
            let (w, h) = (num(get(s, r, "PADWIDTH")) * k, num(get(s, r, "PADHGHT")) * k);
            if w <= 0.0 {
                continue;
            }
            if layer == "TOP" || e.1 == [0.0, 0.0] {
                e.0 = build::fix_shape(build::shape_named(get(s, r, "PADSHAPE1")), [w, h.max(1e-9)]);
                e.1 = [w, if h > 0.0 { h } else { w }];
            }
            if layer != "TOP" && layer != "BOTTOM" && !layer.starts_with('~') {
                e.3 = true;
            }
        }
    }
    // Copper layer names.
    let mut inner: Vec<String> = Vec::new();
    let mut layer = |b: &mut BoardIn, sub: &str| -> Option<String> {
        let s = sub.trim().to_ascii_uppercase();
        match s.as_str() {
            "TOP" => Some("top".into()),
            "BOTTOM" => Some("bottom".into()),
            "" => None,
            _ => {
                if !inner.contains(&s) {
                    inner.push(s.clone());
                }
                match inner.iter().position(|x| *x == s) {
                    Some(0) => Some("in1".into()),
                    Some(1) => Some("in2".into()),
                    _ => {
                        b.note("copper on inner layers past the second was left out");
                        None
                    }
                }
            }
        }
    };
    // Components.
    let mut parts: BTreeMap<String, PartIn> = BTreeMap::new();
    for s in sections.iter().filter(|s| s.has(&["REFDES", "SYM_X", "SYM_Y"])) {
        for r in &s.rows {
            let reference = get(s, r, "REFDES").to_string();
            if reference.is_empty() {
                continue;
            }
            let mirror = get(s, r, "SYM_MIRROR").eq_ignore_ascii_case("YES");
            let value = [get(s, r, "COMP_VALUE"), get(s, r, "COMP_DEVICE_TYPE"), get(s, r, "COMP_PART_NUMBER")].into_iter().find(|v| !v.is_empty()).unwrap_or("").to_string();
            let rot = num(get(s, r, "SYM_ROTATE"));
            parts.insert(reference.clone(), PartIn { reference, value, footprint: get(s, r, "SYM_NAME").to_string(), at: [num(get(s, r, "SYM_X")) * k, num(get(s, r, "SYM_Y")) * k], rotation: if mirror { -rot } else { rot }, side: if mirror { Side::Bottom } else { Side::Top }, mpn: Some(get(s, r, "COMP_PART_NUMBER").to_string()).filter(|x| !x.is_empty()), ..Default::default() });
        }
    }
    // Pins.
    let mut net_of: BTreeMap<(String, String), String> = BTreeMap::new();
    for s in sections.iter().filter(|s| s.has(&["NET_NAME", "REFDES", "PIN_NUMBER"])) {
        for r in &s.rows {
            net_of.insert((get(s, r, "REFDES").to_string(), get(s, r, "PIN_NUMBER").to_string()), get(s, r, "NET_NAME").to_string());
        }
    }
    for s in sections.iter().filter(|s| s.has(&["REFDES", "PIN_NUMBER", "PIN_X", "PIN_Y"])) {
        for r in &s.rows {
            let reference = get(s, r, "REFDES").to_string();
            let number = get(s, r, "PIN_NUMBER").to_string();
            let at = [num(get(s, r, "PIN_X")) * k, num(get(s, r, "PIN_Y")) * k];
            let stack = stacks.get(get(s, r, "PAD_STACK_NAME")).cloned();
            let Some(part) = parts.get_mut(&reference) else { continue };
            if part.pads.iter().any(|p| p.number == number && p.at == at) {
                continue;
            }
            let (shape, size, drill, _) = stack.unwrap_or((PadShape::Circle, [1.0, 1.0], 0.0, false));
            let mut p = PadIn::new(number.clone(), at, size);
            p.shape = shape;
            p.rotation = part.rotation + num(get(s, r, "PIN_ROTATION"));
            if drill > 0.0 {
                p.kind = PadKind::Tht;
                p.drill = Some([drill, drill]);
                if p.size[0].min(p.size[1]) <= drill {
                    p.size = [drill + 0.3, drill + 0.3];
                }
            }
            p.net = net_of.get(&(reference.clone(), number.clone())).cloned().or_else(|| Some(get(s, r, "NET_NAME").to_string())).filter(|n| !n.is_empty());
            part.pads.push(p);
        }
    }
    // Vias.
    for s in sections.iter().filter(|s| s.has(&["VIA_X", "VIA_Y"])) {
        for r in &s.rows {
            let stack = stacks.get(get(s, r, "PAD_STACK_NAME"));
            let dia = stack.map(|x| x.1[0]).filter(|d| *d > 0.0).unwrap_or(0.6);
            let drill = stack.map(|x| x.2).filter(|d| *d > 0.0).unwrap_or(dia / 2.0);
            let net = get(s, r, "NET_NAME");
            b.vias.push(KVia { net: (!net.is_empty()).then(|| net.to_string()), at: [num(get(s, r, "VIA_X")) * k, num(get(s, r, "VIA_Y")) * k], drill, diameter: dia });
        }
    }
    // Graphics.
    for s in sections.iter().filter(|s| s.has(&["CLASS", "SUBCLASS", "GRAPHIC_DATA_NAME", "GRAPHIC_DATA_1"])) {
        for r in &s.rows {
            let class = get(s, r, "CLASS").to_ascii_uppercase();
            let sub = get(s, r, "SUBCLASS").to_ascii_uppercase();
            let g = |i: usize| num(get(s, r, &format!("GRAPHIC_DATA_{i}"))) * k;
            let kind = get(s, r, "GRAPHIC_DATA_NAME").to_ascii_uppercase();
            let (pts, width): (Vec<P>, f64) = match kind.as_str() {
                "LINE" => (vec![[g(1), g(2)], [g(3), g(4)]], g(5)),
                "ARC" => {
                    let (a, e, c) = ([g(1), g(2)], [g(3), g(4)], [g(5), g(6)]);
                    let cw = get(s, r, "GRAPHIC_DATA_10").to_ascii_uppercase().starts_with("CLOCKWISE");
                    let full = crate::geom::dist(a, e) < 1e-9;
                    let pts = if full { build::arc(c, crate::geom::dist(c, a), 0.0, 360.0) } else { build::arc_between(c, a, e, Some(!cw)) };
                    (pts, g(8))
                }
                "RECTANGLE" => {
                    let (a, c) = ([g(1), g(2)], [g(3), g(4)]);
                    (vec![a, [c[0], a[1]], c, [a[0], c[1]], a], 0.0)
                }
                _ => continue,
            };
            if class == "BOARD GEOMETRY" && (sub.contains("OUTLINE") || sub == "DESIGN_OUTLINE") {
                b.edges.push(pts);
            } else if class == "ETCH" {
                if let Some(l) = layer(&mut b, &sub) {
                    let net = get(s, r, "NET_NAME");
                    b.polyline((!net.is_empty()).then(|| net.to_string()), &l, width.max(0.05), &pts);
                }
            } else if class == "PACKAGE GEOMETRY" && sub.starts_with("SILKSCREEN") {
                if let Some(p) = parts.get_mut(get(s, r, "REFDES")) {
                    p.silk.push(pts);
                }
            }
        }
    }
    b.source_copper_layers = 2 + inner.len();
    b.parts.extend(parts.into_values());
    Ok(b)
}
