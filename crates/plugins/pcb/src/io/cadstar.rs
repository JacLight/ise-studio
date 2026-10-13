//! CADSTAR PCB archives (`.cpa`): the bracketed text format CADSTAR writes
//! for exchange. Read: the header's resolution (coordinates are integers in
//! that unit, y up), the electrical layers by physical order, pad, via and
//! route codes, the library's footprints (pads with their codes and sides,
//! silkscreen figures), the board outline, placed components (position,
//! orientation in thousandths of a degree, mirrored when on the bottom), and
//! each net's pins, routes, vias and copper pour templates.

use super::build::{self, BoardIn, PadIn, PartIn};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{self, P};
use crate::kicad::{KVia, KZone};
use crate::sexpr::{self, Sexp};
use crate::{PadConnection, Side};
use std::collections::BTreeMap;

fn all<'a>(e: &'a Sexp, head: &str, out: &mut Vec<&'a Sexp>) {
    for c in e.args() {
        if c.is(head) {
            out.push(c);
        }
        all(c, head, out);
    }
}

fn first<'a>(e: &'a Sexp, head: &str) -> Option<&'a Sexp> {
    let mut v = Vec::new();
    all(e, head, &mut v);
    v.into_iter().next()
}

struct Code {
    shape: PadShape,
    size: [f64; 2],
    drill: Option<f64>,
}

pub fn read(text: &str) -> Result<BoardIn, String> {
    let top = sexpr::parse(text)?;
    let root = top.iter().find(|e| e.is("CADSTARPCB")).ok_or("not a CADSTAR PCB archive")?;
    // Units per mm from the header's resolution.
    let res = root.find("HEADER").and_then(|h| h.find("RESOLUTION")).map(|r| r.args().iter().filter_map(|a| a.text()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let k = if res.contains("HUNDREDTH") { 1e-5 } else if res.contains("TENTH") { 1e-4 } else if res.contains("MICRON") { 1e-3 } else { 1e-5 };
    let mm = |v: f64| v * k;
    let pt = |e: &Sexp| -> P { [mm(e.arg_num(0).unwrap_or(0.0)), mm(e.arg_num(1).unwrap_or(0.0))] };
    let pts = |e: &Sexp| -> Vec<P> { e.find_all("PT").map(pt).collect() };
    let mut b = BoardIn::default();
    b.title = root.find("HEADER").and_then(|h| h.find("JOBTITLE")).and_then(|t| t.arg_text(0)).map(str::to_string).filter(|t| !t.is_empty());

    // Electrical layers by physical order.
    let mut phys: Vec<(f64, String)> = Vec::new();
    let mut silk_layers: Vec<String> = Vec::new();
    let mut lys = Vec::new();
    all(root, "LAYER", &mut lys);
    for l in lys {
        let id = l.arg_text(0).unwrap_or("").to_string();
        if let Some(p) = first(l, "PHYSICAL").and_then(|p| p.arg_num(0)) {
            if first(l, "ELEC").is_some() || l.args().iter().any(|a| a.text() == Some("ELEC")) || first(l, "NONELEC").is_none() {
                phys.push((p, id.clone()));
            }
        }
        let name = l.arg_text(1).unwrap_or("").to_ascii_lowercase();
        if name.contains("silk") || name.contains("legend") || first(l, "LAYERSUBTYPE_SILKSCREEN").is_some() {
            silk_layers.push(id);
        }
    }
    phys.sort_by(|a, c| a.0.total_cmp(&c.0));
    b.source_copper_layers = phys.len().max(2);
    let mut layer_of: BTreeMap<String, String> = BTreeMap::new();
    for (i, (_, id)) in phys.iter().enumerate() {
        let name = if i == 0 {
            "top"
        } else if i + 1 == phys.len() {
            "bottom"
        } else if i == 1 {
            "in1"
        } else if i == 2 {
            "in2"
        } else {
            continue;
        };
        layer_of.insert(id.clone(), name.into());
    }
    let top_id = phys.first().map(|x| x.1.clone()).unwrap_or_default();

    // Pad, via and route codes.
    let mut codes: BTreeMap<String, Code> = BTreeMap::new();
    for head in ["PADCODE", "VIACODE"] {
        let mut v = Vec::new();
        all(root, head, &mut v);
        for c in v {
            let id = c.arg_text(0).unwrap_or("").to_string();
            let (shape, size) = if let Some(s) = first(c, "CIRCLE").or_else(|| first(c, "OCTAGON")).or_else(|| first(c, "ANNULUS")) {
                let d = mm(s.arg_num(0).unwrap_or(0.0));
                (PadShape::Circle, [d, d])
            } else if let Some(s) = first(c, "RECTANGLE").or_else(|| first(c, "DIAMOND")) {
                (PadShape::Rect, [mm(s.arg_num(0).unwrap_or(0.0)), mm(s.arg_num(1).unwrap_or(0.0))])
            } else if let Some(s) = first(c, "ROUNDEDRECT").or_else(|| first(c, "ROUNDED")) {
                (PadShape::RoundRect, [mm(s.arg_num(0).unwrap_or(0.0)), mm(s.arg_num(1).unwrap_or(0.0))])
            } else if let Some(s) = first(c, "BULLET").or_else(|| first(c, "FINGER")) {
                (PadShape::Oval, [mm(s.arg_num(0).unwrap_or(0.0)), mm(s.arg_num(1).unwrap_or(0.0))])
            } else {
                (PadShape::Circle, [0.5, 0.5])
            };
            let drill = first(c, "DRILLDIAMETER").and_then(|d| d.arg_num(0)).map(mm).filter(|d| *d > 0.0);
            codes.insert(id, Code { shape: build::fix_shape(shape, size), size, drill });
        }
    }
    let mut route_w: BTreeMap<String, f64> = BTreeMap::new();
    let mut rcs = Vec::new();
    all(root, "ROUTECODE", &mut rcs);
    for c in rcs {
        if let Some(w) = first(c, "OPTWIDTH").and_then(|w| w.arg_num(0)) {
            route_w.insert(c.arg_text(0).unwrap_or("").to_string(), mm(w));
        }
    }

    // Footprints (symbol definitions): pads and silkscreen, local.
    struct FpPad {
        id: String,
        number: String,
        code: String,
        at: P,
        rot: f64,
        through: bool,
    }
    let mut fps: BTreeMap<String, (String, Vec<FpPad>, Vec<Vec<P>>)> = BTreeMap::new();
    let mut defs = Vec::new();
    all(root, "SYMBOLDEF", &mut defs);
    for d in defs {
        let id = d.arg_text(0).unwrap_or("").to_string();
        let name = d.arg_text(1).unwrap_or(&id).to_string();
        let mut pads = Vec::new();
        for p in d.find_all("PAD") {
            let pid = p.arg_text(0).unwrap_or("").to_string();
            let number = first(p, "PADIDENTIFIER").and_then(|x| x.arg_text(0)).map(str::to_string).unwrap_or_else(|| pid.clone());
            let side = p.args().iter().filter_map(|a| a.text()).chain(first(p, "SIDE").and_then(|s| s.arg_text(0))).collect::<Vec<_>>().join(" ");
            pads.push(FpPad { id: pid, number, code: p.arg_text(1).unwrap_or("").to_string(), at: p.find("PT").map(pt).unwrap_or([0.0, 0.0]), rot: p.find("ORIENT").and_then(|o| o.arg_num(0)).unwrap_or(0.0) / 1000.0, through: side.contains("THROUGH") });
        }
        let mut silk = Vec::new();
        for f in d.find_all("FIGURE") {
            let layer = f.arg_text(1).unwrap_or("");
            if silk_layers.iter().any(|s| s == layer) {
                if let Some(o) = first(f, "OUTLINE").or_else(|| first(f, "OPENSHAPE")) {
                    silk.push(pts(o));
                }
            }
        }
        fps.insert(id, (name, pads, silk));
    }
    // Part names (values).
    let mut part_names: BTreeMap<String, String> = BTreeMap::new();
    if let Some(parts) = root.find("PARTS") {
        for p in parts.args().iter().filter(|p| p.is("PART")) {
            part_names.insert(p.arg_text(0).unwrap_or("").to_string(), p.arg_text(1).unwrap_or("").to_string());
        }
    }
    let layout = root.find("LAYOUT").ok_or("the archive has no LAYOUT")?;
    for bd in layout.find_all("BOARD") {
        if let Some(o) = first(bd, "OUTLINE").or_else(|| first(bd, "SHAPE")) {
            let mut v = pts(o);
            if let Some(f) = v.first().copied() {
                v.push(f);
            }
            b.edges.push(v);
        }
    }
    // Pin nets.
    let mut pin_net: BTreeMap<(String, String), String> = BTreeMap::new();
    for n in layout.find_all("NET") {
        let name = n.find("SIGNAME").and_then(|s| s.arg_text(0)).unwrap_or(n.arg_text(0).unwrap_or("")).to_string();
        for p in n.find_all("PIN") {
            pin_net.insert((p.arg_text(1).unwrap_or("").to_string(), p.arg_text(2).unwrap_or("").to_string()), name.clone());
        }
    }
    for c in layout.find_all("COMPONENT") {
        let cid = c.arg_text(0).unwrap_or("").to_string();
        let reference = c.arg_text(1).unwrap_or("").to_string();
        let Some((fp_name, fp_pads, silk)) = fps.get(c.arg_text(2).unwrap_or("")) else {
            b.note(format!("{reference}: its footprint is not in the archive's library"));
            continue;
        };
        let at = c.find("PT").map(pt).unwrap_or([0.0, 0.0]);
        let rot = c.find("ORIENT").and_then(|o| o.arg_num(0)).unwrap_or(0.0) / 1000.0;
        let bottom = c.find("MIRROR").is_some() || c.args().iter().any(|a| a.text() == Some("MIRROR"));
        let place = |q: P| -> P { geom::add(at, geom::rotate(if bottom { [-q[0], q[1]] } else { q }, rot)) };
        let mut part = PartIn { reference: reference.clone(), value: part_names.get(c.arg_text(3).unwrap_or("")).cloned().unwrap_or_default(), footprint: fp_name.clone(), at, rotation: rot, side: if bottom { Side::Bottom } else { Side::Top }, ..Default::default() };
        for fp in fp_pads {
            let code = codes.get(&fp.code);
            let size = code.map(|c| c.size).unwrap_or([1.0, 1.0]);
            let mut p = PadIn::new(fp.number.clone(), place(fp.at), size);
            p.shape = code.map(|c| c.shape).unwrap_or(PadShape::Circle);
            p.rotation = if bottom { rot - fp.rot } else { rot + fp.rot };
            if let Some(d) = code.and_then(|c| c.drill).filter(|_| fp.through || code.is_some_and(|c| c.drill.is_some())) {
                p.kind = PadKind::Tht;
                p.drill = Some([d, d]);
                if p.size[0].min(p.size[1]) <= d {
                    p.size = [d + 0.3, d + 0.3];
                }
            }
            p.net = pin_net.get(&(cid.clone(), fp.id.clone())).or_else(|| pin_net.get(&(cid.clone(), fp.number.clone()))).cloned();
            part.pads.push(p);
        }
        part.silk = silk.iter().map(|l| l.iter().map(|q| place(*q)).collect()).collect();
        b.parts.push(part);
    }
    // Copper.
    for n in layout.find_all("NET") {
        let name = n.find("SIGNAME").and_then(|s| s.arg_text(0)).unwrap_or(n.arg_text(0).unwrap_or("")).to_string();
        let net = (!name.is_empty()).then_some(name.clone());
        let mut routes = Vec::new();
        all(n, "ROUTE", &mut routes);
        for r in routes {
            let layer = r.arg_text(0).unwrap_or("");
            let Some(l) = layer_of.get(layer).cloned() else {
                if !phys.iter().any(|x| x.1 == layer) {
                    continue;
                }
                b.note("copper on inner layers past the second was left out");
                continue;
            };
            let width = r.find("ROUTEWIDTH").and_then(|w| w.arg_num(0)).map(mm).or_else(|| r.args().iter().filter_map(|a| a.text()).find_map(|t| route_w.get(t).copied())).unwrap_or(0.25);
            let mut cur_w = width;
            let mut prev: Option<P> = None;
            for e in r.args() {
                if e.is("ROUTEWIDTH") {
                    cur_w = e.arg_num(0).map(mm).unwrap_or(cur_w);
                }
                if e.is("PT") || e.is("ROUTEPOINT") {
                    let q = if e.is("PT") { pt(e) } else { e.find("PT").map(pt).unwrap_or([0.0, 0.0]) };
                    if let Some(a) = prev {
                        b.seg(net.clone(), &l, cur_w, a, q);
                    }
                    prev = Some(q);
                }
            }
        }
        for v in n.find_all("VIA") {
            let code = codes.get(v.arg_text(1).unwrap_or(""));
            let at = v.find("PT").map(pt).unwrap_or([0.0, 0.0]);
            let dia = code.map(|c| c.size[0]).unwrap_or(0.6);
            let drill = code.and_then(|c| c.drill).unwrap_or(dia / 2.0);
            b.vias.push(KVia { net: net.clone(), at, drill, diameter: dia });
        }
    }
    for t in layout.find_all("TEMPLATE") {
        let net_id = t.args().iter().filter_map(|a| a.text()).find(|x| x.starts_with("NET")).unwrap_or("");
        let net = layout.find_all("NET").find(|n| n.arg_text(0) == Some(net_id)).and_then(|n| n.find("SIGNAME")).and_then(|s| s.arg_text(0)).map(str::to_string);
        let layer = t.args().iter().filter_map(|a| a.text()).find_map(|x| layer_of.get(x).cloned());
        let outline = first(t, "OUTLINE").map(pts).unwrap_or_default();
        if let (Some(net), Some(layer), true) = (net, layer, outline.len() >= 3) {
            b.zones.push(KZone { net, layer, outline, clearance: None, min_width: 0.25, priority: 0, connection: PadConnection::Thermal, thermal_gap: None, thermal_spoke: None });
        }
    }
    let _ = top_id;
    Ok(b)
}
