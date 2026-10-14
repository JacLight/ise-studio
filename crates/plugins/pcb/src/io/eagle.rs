//! Eagle (and Fusion Electronics) boards and schematics: the XML format
//! Eagle 6 and later write, read from its published DTD (`eagle.dtd`).
//! Millimetres, y up. An element's or instance's `rot` is `R<deg>`, with `M`
//! for mirrored (an element on the bottom) and `S` for spin (ignored).
//!
//! Boards: elements placed from their library packages (SMDs, pads, holes,
//! silkscreen on tPlace/bPlace), each signal's contact references give the
//! pads their nets, signal wires (arcs flattened) and vias become copper,
//! signal polygons become pours, and the Dimension layer (20) is the edge.
//!
//! Schematics: every gate instance is drawn from its library symbol where it
//! sits; pins land on the pads the device's `connect`s name. Nets are
//! drawn as their segments' wires, junctions and labels; a named net gets a
//! label on each segment (Eagle joins same-named segments without one), and
//! supply symbols become Iṣẹ́ Studio power ports.

use super::build::{self, BoardIn, PadIn, PartIn};
use super::sch::{self, Draft, DraftSymbol};
use super::xml::{self, Element};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{self, P};
use crate::kicad::{KVia, KZone};
use crate::schematic::LabelKind;
use crate::symbols::{Dir, Fill, Gfx, PinDef, PinType};
use crate::{PadConnection, Side};
use std::collections::BTreeMap;

/// `R90`, `MR180`, `SR45` → (degrees, mirrored).
fn rot(s: Option<&str>) -> (f64, bool) {
    let s = s.unwrap_or("R0");
    let mirror = s.contains('M');
    let deg = s.trim_start_matches(|c: char| c.is_ascii_alphabetic()).parse::<f64>().unwrap_or(0.0);
    (deg, mirror)
}

struct Xf {
    at: P,
    deg: f64,
    mirror: bool,
}

impl Xf {
    fn p(&self, q: P) -> P {
        let q = if self.mirror { [-q[0], q[1]] } else { q };
        geom::add(self.at, geom::rotate(q, self.deg))
    }
}

fn pt(e: &Element, x: &str, y: &str) -> P {
    [e.numor(x, 0.0), e.numor(y, 0.0)]
}

/// A wire's points (an arc when it has a `curve`).
fn wire_pts(e: &Element) -> Vec<P> {
    let a = pt(e, "x1", "y1");
    let b = pt(e, "x2", "y2");
    match e.num("curve").filter(|c| c.abs() > 1e-9) {
        Some(c) => build::arc_sweep(a, b, c),
        None => vec![a, b],
    }
}

struct Layers {
    inner: Vec<u32>,
}

impl Layers {
    fn copper(&mut self, n: u32) -> Option<String> {
        match n {
            1 => Some("top".into()),
            16 => Some("bottom".into()),
            2..=15 => {
                if !self.inner.contains(&n) {
                    self.inner.push(n);
                    self.inner.sort();
                }
                match self.inner.iter().position(|x| *x == n) {
                    Some(0) => Some("in1".into()),
                    Some(1) => Some("in2".into()),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

fn library_packages(board: &Element) -> BTreeMap<(String, String), Element> {
    let mut m = BTreeMap::new();
    if let Some(libs) = board.child("libraries") {
        for lib in libs.all("library") {
            let ln = lib.attr("name").unwrap_or("").to_string();
            for pk in lib.child("packages").into_iter().flat_map(|p| p.all("package")) {
                m.insert((ln.clone(), pk.attr("name").unwrap_or("").to_string()), pk.clone());
            }
        }
    }
    m
}

pub fn read_board(text: &str) -> Result<BoardIn, String> {
    let root = xml::parse(text)?;
    let board = root.path("drawing/board").ok_or("not an Eagle board (no <board>)")?;
    let mut b = BoardIn::default();
    let mut layers = Layers { inner: vec![] };
    // Copper layers in use (signals) for the layer count.
    let mut used: Vec<u32> = Vec::new();
    let mut ws = Vec::new();
    board.descendants("wire", &mut ws);
    for w in &ws {
        if let Some(l) = w.num("layer").map(|v| v as u32).filter(|l| (1..=16).contains(l)) {
            if !used.contains(&l) {
                used.push(l);
            }
        }
    }
    used.extend([1, 16]);
    used.sort();
    used.dedup();
    b.source_copper_layers = used.len();
    let packages = library_packages(board);

    // Plain: board edge, holes.
    let mut holes = 0;
    if let Some(plain) = board.child("plain") {
        for e in &plain.children {
            match e.name.as_str() {
                "wire" if e.num("layer") == Some(20.0) => b.edges.push(wire_pts(e)),
                "circle" if e.num("layer") == Some(20.0) => {
                    let mut c = build::circle(pt(e, "x", "y"), e.numor("radius", 0.0));
                    c.push(c[0]);
                    b.edges.push(c);
                }
                "rectangle" if e.num("layer") == Some(20.0) => {
                    let (a, c) = (pt(e, "x1", "y1"), pt(e, "x2", "y2"));
                    b.edges.push(vec![a, [c[0], a[1]], c, [a[0], c[1]], a]);
                }
                "hole" => {
                    holes += 1;
                    let d = e.numor("drill", 1.0);
                    let at = pt(e, "x", "y");
                    let mut p = PadIn::new("", at, [d, d]);
                    p.kind = PadKind::Npth;
                    p.shape = PadShape::Circle;
                    p.drill = Some([d, d]);
                    b.parts.push(PartIn { reference: format!("H{holes}"), value: "hole".into(), footprint: format!("Hole_{d}mm"), at, pads: vec![p], ..Default::default() });
                }
                "polygon" | "text" | "circle" | "rectangle" | "wire" | "dimension" | "frame" => {}
                _ => {}
            }
        }
    }

    // Nets of pads from the signals' contact references.
    let mut pad_net: BTreeMap<(String, String), String> = BTreeMap::new();
    if let Some(sigs) = board.child("signals") {
        for s in sigs.all("signal") {
            let net = s.attr("name").unwrap_or("").to_string();
            for c in s.all("contactref") {
                pad_net.insert((c.attr("element").unwrap_or("").to_string(), c.attr("pad").unwrap_or("").to_string()), net.clone());
            }
        }
    }

    // Elements.
    if let Some(els) = board.child("elements") {
        for el in els.all("element") {
            let name = el.attr("name").unwrap_or("").to_string();
            let key = (el.attr("library").unwrap_or("").to_string(), el.attr("package").unwrap_or("").to_string());
            let Some(pkg) = packages.get(&key) else {
                b.note(format!("{name}: package {} is not in the file's libraries", key.1));
                continue;
            };
            let (deg, mirror) = rot(el.attr("rot"));
            let at = pt(el, "x", "y");
            let xf = Xf { at, deg, mirror };
            let side = if mirror { Side::Bottom } else { Side::Top };
            let mut part = PartIn { reference: name.clone(), value: el.attr("value").unwrap_or("").to_string(), footprint: key.1.clone(), at, rotation: deg, side, ..Default::default() };
            for a in el.all("attribute") {
                let v = a.attr("value").unwrap_or("").to_string();
                match a.attr("name").unwrap_or("").to_ascii_uppercase().as_str() {
                    "MPN" | "MFR_PN" | "PARTNO" => part.mpn = Some(v),
                    "MANUFACTURER" | "MFR" => part.manufacturer = Some(v),
                    "DESCRIPTION" => part.description = v,
                    _ => {}
                }
            }
            let pad_rot = |r: f64| if mirror { deg - r } else { deg + r };
            for e in &pkg.children {
                match e.name.as_str() {
                    "smd" => {
                        let (r, _) = rot(e.attr("rot"));
                        let (dx, dy) = (e.numor("dx", 0.5), e.numor("dy", 0.5));
                        let round = e.numor("roundness", 0.0).clamp(0.0, 100.0);
                        let shape = if round >= 99.9 {
                            if (dx - dy).abs() < 1e-9 { PadShape::Circle } else { PadShape::Oval }
                        } else if round > 0.0 {
                            PadShape::RoundRect
                        } else {
                            PadShape::Rect
                        };
                        let num = e.attr("name").unwrap_or("").to_string();
                        let mut p = PadIn::new(num.clone(), xf.p(pt(e, "x", "y")), [dx, dy]);
                        p.shape = shape;
                        p.rotation = pad_rot(r);
                        if shape == PadShape::RoundRect {
                            p.corner = Some(round / 200.0);
                        }
                        p.net = pad_net.get(&(name.clone(), num)).cloned();
                        part.pads.push(p);
                    }
                    "pad" => {
                        let (r, _) = rot(e.attr("rot"));
                        let drill = e.numor("drill", 0.8);
                        let dia = e.num("diameter").filter(|d| *d > drill).unwrap_or(drill + 0.5);
                        let (shape, size) = match e.attr("shape").unwrap_or("round") {
                            "square" => (PadShape::Rect, [dia, dia]),
                            "long" => (PadShape::Oval, [2.0 * dia, dia]),
                            "offset" => (PadShape::Oval, [2.0 * dia, dia]),
                            "octagon" => {
                                b.note("octagonal pads became round");
                                (PadShape::Circle, [dia, dia])
                            }
                            _ => (PadShape::Circle, [dia, dia]),
                        };
                        if e.attr("shape") == Some("offset") {
                            b.note("offset pads were centred on their hole");
                        }
                        let num = e.attr("name").unwrap_or("").to_string();
                        let mut p = PadIn::new(num.clone(), xf.p(pt(e, "x", "y")), size);
                        p.kind = PadKind::Tht;
                        p.shape = shape;
                        p.rotation = pad_rot(r);
                        p.drill = Some([drill, drill]);
                        p.net = pad_net.get(&(name.clone(), num)).cloned();
                        part.pads.push(p);
                    }
                    "hole" => {
                        let d = e.numor("drill", 1.0);
                        let mut p = PadIn::new("", xf.p(pt(e, "x", "y")), [d, d]);
                        p.kind = PadKind::Npth;
                        p.shape = PadShape::Circle;
                        p.drill = Some([d, d]);
                        part.pads.push(p);
                    }
                    "wire" => match e.num("layer").map(|v| v as u32) {
                        Some(21) | Some(22) => part.silk.push(wire_pts(e).into_iter().map(|q| xf.p(q)).collect()),
                        Some(20) => b.edges.push(wire_pts(e).into_iter().map(|q| xf.p(q)).collect()),
                        _ => {}
                    },
                    "circle" if matches!(e.num("layer").map(|v| v as u32), Some(21) | Some(22)) => {
                        let mut c = build::circle(pt(e, "x", "y"), e.numor("radius", 0.5));
                        c.push(c[0]);
                        part.silk.push(c.into_iter().map(|q| xf.p(q)).collect());
                    }
                    _ => {}
                }
            }
            b.parts.push(part);
        }
    }

    // Signals: copper.
    if let Some(sigs) = board.child("signals") {
        for s in sigs.all("signal") {
            let net = s.attr("name").map(str::to_string);
            for e in &s.children {
                match e.name.as_str() {
                    "wire" => {
                        let ln = e.num("layer").unwrap_or(0.0) as u32;
                        match layers.copper(ln) {
                            Some(l) => {
                                let pts = wire_pts(e);
                                b.polyline(net.clone(), &l, e.numor("width", 0.25), &pts);
                            }
                            None if (2..=15).contains(&ln) => b.note("copper on inner layers past the second was left out"),
                            None => {}
                        }
                    }
                    "via" => {
                        let drill = e.numor("drill", 0.3);
                        let dia = e.num("diameter").filter(|d| *d > drill).unwrap_or(drill + 0.3);
                        if e.attr("extent").is_some_and(|x| x != "1-16") {
                            b.note("blind and buried vias became through vias");
                        }
                        b.vias.push(KVia { net: net.clone(), at: pt(e, "x", "y"), drill, diameter: dia });
                    }
                    "polygon" => {
                        let ln = e.num("layer").unwrap_or(0.0) as u32;
                        let Some(l) = layers.copper(ln) else { continue };
                        let mut outline: Vec<P> = Vec::new();
                        let vs: Vec<&Element> = e.all("vertex").collect();
                        for (i, v) in vs.iter().enumerate() {
                            let a = pt(v, "x", "y");
                            let nxt = pt(vs[(i + 1) % vs.len()], "x", "y");
                            match v.num("curve").filter(|c| c.abs() > 1e-9) {
                                Some(c) => {
                                    let arc = build::arc_sweep(a, nxt, c);
                                    outline.extend(&arc[..arc.len() - 1]);
                                }
                                None => outline.push(a),
                            }
                        }
                        if outline.len() < 3 || net.is_none() {
                            continue;
                        }
                        let rank = e.numor("rank", 1.0);
                        b.zones.push(KZone {
                            net: net.clone().unwrap_or_default(),
                            layer: l,
                            outline,
                            clearance: e.num("isolate").filter(|v| *v > 0.0),
                            min_width: e.numor("width", 0.25).max(0.1),
                            priority: (7.0 - rank) as i32,
                            connection: if e.attr("thermals") == Some("no") { PadConnection::Solid } else { PadConnection::Thermal },
                            thermal_gap: None,
                            thermal_spoke: None,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    b.title = root.path("drawing/board/description").map(|d| d.text.trim().to_string()).filter(|t| !t.is_empty() && t.len() < 80);
    Ok(b)
}

// ---- Schematics ------------------------------------------------------------------------

fn pin_len(s: Option<&str>) -> f64 {
    match s.unwrap_or("long") {
        "point" => 0.0,
        "short" => 2.54,
        "middle" => 5.08,
        _ => 7.62,
    }
}

fn pin_type(s: Option<&str>) -> PinType {
    match s.unwrap_or("io") {
        "in" => PinType::Input,
        "out" => PinType::Output,
        "io" => PinType::Bidirectional,
        "oc" => PinType::OpenCollector,
        "hiz" => PinType::TriState,
        "pwr" => PinType::PowerIn,
        "sup" => PinType::PowerOut,
        "nc" => PinType::NoConnect,
        _ => PinType::Passive,
    }
}

fn base_name(s: &str) -> String {
    s.split('@').next().unwrap_or(s).to_string()
}

struct Lib {
    symbols: BTreeMap<String, Element>,
    /// deviceset → (prefix, gates (name, symbol), devices (name → (package, pin→pads)))
    devicesets: BTreeMap<String, (String, Vec<(String, String)>, BTreeMap<String, (Option<String>, BTreeMap<(String, String), Vec<String>>)>)>,
}

fn libraries(sch: &Element) -> BTreeMap<String, Lib> {
    let mut out = BTreeMap::new();
    for lib in sch.child("libraries").into_iter().flat_map(|l| l.all("library")) {
        let mut l = Lib { symbols: BTreeMap::new(), devicesets: BTreeMap::new() };
        for s in lib.child("symbols").into_iter().flat_map(|s| s.all("symbol")) {
            l.symbols.insert(s.attr("name").unwrap_or("").to_string(), s.clone());
        }
        for ds in lib.child("devicesets").into_iter().flat_map(|d| d.all("deviceset")) {
            let gates: Vec<(String, String)> = ds.child("gates").into_iter().flat_map(|g| g.all("gate")).map(|g| (g.attr("name").unwrap_or("").to_string(), g.attr("symbol").unwrap_or("").to_string())).collect();
            let mut devices = BTreeMap::new();
            for d in ds.child("devices").into_iter().flat_map(|d| d.all("device")) {
                let mut connects = BTreeMap::new();
                for c in d.child("connects").into_iter().flat_map(|c| c.all("connect")) {
                    connects.insert((c.attr("gate").unwrap_or("").to_string(), c.attr("pin").unwrap_or("").to_string()), c.attr("pad").unwrap_or("").split_whitespace().map(str::to_string).collect());
                }
                devices.insert(d.attr("name").unwrap_or("").to_string(), (d.attr("package").map(str::to_string), connects));
            }
            l.devicesets.insert(ds.attr("name").unwrap_or("").to_string(), (ds.attr("prefix").unwrap_or("U").to_string(), gates, devices));
        }
        out.insert(lib.attr("name").unwrap_or("").to_string(), l);
    }
    out
}

fn symbol_gfx(sym: &Element, xf: &Xf) -> Vec<Gfx> {
    let mut g = Vec::new();
    for e in &sym.children {
        match e.name.as_str() {
            "wire" => g.push(Gfx::Line { pts: wire_pts(e).into_iter().map(|q| xf.p(q)).collect(), width: 0.0 }),
            "rectangle" => {
                let (a, c) = (pt(e, "x1", "y1"), pt(e, "x2", "y2"));
                g.push(Gfx::Poly { pts: [a, [c[0], a[1]], c, [a[0], c[1]]].into_iter().map(|q| xf.p(q)).collect(), fill: Fill::Outline });
            }
            "circle" => g.push(Gfx::Circle { c: xf.p(pt(e, "x", "y")), r: e.numor("radius", 1.0), fill: if e.num("width") == Some(0.0) { Fill::Outline } else { Fill::None } }),
            "polygon" => {
                let pts: Vec<P> = e.all("vertex").map(|v| xf.p(pt(v, "x", "y"))).collect();
                if pts.len() >= 3 {
                    g.push(Gfx::Poly { pts, fill: Fill::Outline });
                }
            }
            "text" => {
                let t = e.text.trim();
                if !t.is_empty() && !t.starts_with('>') {
                    g.push(Gfx::Text { at: xf.p(pt(e, "x", "y")), text: t.to_string(), size: e.numor("size", 1.778) });
                }
            }
            _ => {}
        }
    }
    g
}

pub fn read_schematic(text: &str) -> Result<Draft, String> {
    let root = xml::parse(text)?;
    let sch = root.path("drawing/schematic").ok_or("not an Eagle schematic (no <schematic>)")?;
    let libs = libraries(sch);
    let mut d = Draft { y_up: true, ..Default::default() };
    // Parts: reference → (library, deviceset, device, value, attributes).
    let mut parts: BTreeMap<String, (String, String, String, String, BTreeMap<String, String>)> = BTreeMap::new();
    for p in sch.child("parts").into_iter().flat_map(|p| p.all("part")) {
        let attrs = p.all("attribute").map(|a| (a.attr("name").unwrap_or("").to_string(), a.attr("value").unwrap_or("").to_string())).collect();
        parts.insert(p.attr("name").unwrap_or("").to_string(), (p.attr("library").unwrap_or("").to_string(), p.attr("deviceset").unwrap_or("").to_string(), p.attr("device").unwrap_or("").to_string(), p.attr("value").unwrap_or("").to_string(), attrs));
    }
    let sheets: Vec<&Element> = sch.child("sheets").into_iter().flat_map(|s| s.all("sheet")).collect();
    // Which sheets each named net appears on (labels go global when several).
    let mut net_sheets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (si, sh) in sheets.iter().enumerate() {
        for n in sh.child("nets").into_iter().flat_map(|n| n.all("net")) {
            let e = net_sheets.entry(n.attr("name").unwrap_or("").to_string()).or_default();
            if !e.contains(&si) {
                e.push(si);
            }
        }
    }
    for (si, sh) in sheets.iter().enumerate() {
        let sid = (si + 1).to_string();
        d.sheets.push((sid.clone(), sh.child("description").map(|x| x.text.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("Sheet {}", si + 1))));
        // Where each part's pins sit on this sheet.
        let mut pin_at: BTreeMap<(String, String, String), P> = BTreeMap::new();
        for inst in sh.child("instances").into_iter().flat_map(|i| i.all("instance")) {
            let pname = inst.attr("part").unwrap_or("").to_string();
            let gate = inst.attr("gate").unwrap_or("").to_string();
            let Some((lib, ds, dev, value, attrs)) = parts.get(&pname) else { continue };
            let Some(l) = libs.get(lib) else { continue };
            let Some((_, gates, devices)) = l.devicesets.get(ds) else { continue };
            let Some(sym_name) = gates.iter().find(|g| g.0 == gate).map(|g| g.1.clone()) else { continue };
            let Some(sym) = l.symbols.get(&sym_name) else { continue };
            let unit = gates.iter().position(|g| g.0 == gate).unwrap_or(0) as u32 + 1;
            let (deg, mirror) = rot(inst.attr("rot"));
            let at = pt(inst, "x", "y");
            let xf = Xf { at, deg, mirror };
            let (package, connects) = devices.get(dev).cloned().unwrap_or((None, BTreeMap::new()));
            let mut pins: Vec<PinDef> = Vec::new();
            for p in sym.all("pin") {
                let pn = p.attr("name").unwrap_or("").to_string();
                let local = pt(p, "x", "y");
                let len = pin_len(p.attr("length"));
                let (pr, _) = rot(p.attr("rot"));
                // The pin runs from its connection point into the body along its rotation.
                let body_local = geom::add(local, geom::rotate([len, 0.0], pr));
                let tip = xf.p(local);
                let body = xf.p(body_local);
                let side = if len > 0.0 { sch::side_from(body, tip) } else { Dir::Left };
                pin_at.insert((pname.clone(), gate.clone(), pn.clone()), tip);
                let pads = connects.get(&(gate.clone(), pn.clone())).cloned().unwrap_or_default();
                let number = pads.first().cloned().unwrap_or_else(|| base_name(&pn));
                pins.push(PinDef { number, name: base_name(&pn), at: tip, side, length: len.max(0.0), etype: pin_type(p.attr("direction")), hidden: false, unit: 0, shape: Default::default(), pads });
            }
            // A supply symbol (no package, supply pins): a power port per pin.
            let supply = package.is_none() && !pins.is_empty() && sym.all("pin").all(|p| p.attr("direction") == Some("sup"));
            if supply {
                for p in &pins {
                    let v = p.side.v();
                    // `side` holds the signs of (tip − body) in source (y-up) coordinates; the port points back into the body.
                    let ang = (-v[1]).atan2(-v[0]).to_degrees();
                    let net = if value.is_empty() || value == ds { p.name.clone() } else { value.clone() };
                    d.labels.push((sid.clone(), LabelKind::Power, net, p.at, ang.rem_euclid(360.0)));
                }
                continue;
            }
            if package.is_none() {
                d.note(format!("{pname} has no package: kept on the schematic only"));
            }
            let mut fields = BTreeMap::new();
            let mut mpn = None;
            let mut manufacturer = None;
            let mut description = String::new();
            for (k, v) in attrs {
                match k.to_ascii_uppercase().as_str() {
                    "MPN" | "MFR_PN" | "PARTNO" => mpn = Some(v.clone()),
                    "MANUFACTURER" | "MFR" => manufacturer = Some(v.clone()),
                    "DESCRIPTION" => description = v.clone(),
                    _ => {
                        fields.insert(k.clone(), v.clone());
                    }
                }
            }
            d.symbols.push(DraftSymbol {
                sheet: sid.clone(),
                reference: pname.clone(),
                value: if value.is_empty() { ds.clone() } else { value.clone() },
                footprint: package.unwrap_or_default(),
                lib: format!("eagle:{lib}:{sym_name}"),
                at,
                gfx: symbol_gfx(sym, &xf),
                pins,
                unit,
                mpn,
                manufacturer,
                description,
                fields,
                dnp: false,
                virtual_part: false,
            });
        }
        for n in sh.child("nets").into_iter().flat_map(|n| n.all("net")) {
            let name = n.attr("name").unwrap_or("").to_string();
            let auto = name.starts_with("N$");
            let multi_sheet = net_sheets.get(&name).is_some_and(|v| v.len() > 1);
            let segs: Vec<&Element> = n.all("segment").collect();
            for seg in &segs {
                let mut first: Option<P> = None;
                let mut labelled = false;
                for e in &seg.children {
                    match e.name.as_str() {
                        "wire" => {
                            let pts = wire_pts(e);
                            first.get_or_insert(pts[0]);
                            d.wires.push((sid.clone(), pts));
                        }
                        "junction" => d.junctions.push((sid.clone(), pt(e, "x", "y"))),
                        "label" => labelled = true,
                        "pinref" => {
                            let k = (e.attr("part").unwrap_or("").to_string(), e.attr("gate").unwrap_or("").to_string(), e.attr("pin").unwrap_or("").to_string());
                            if let Some(p) = pin_at.get(&k) {
                                first.get_or_insert(*p);
                            }
                        }
                        _ => {}
                    }
                }
                // Same-named segments join by name in Eagle: a label on each carries that.
                if !auto && (segs.len() > 1 || multi_sheet || labelled) {
                    if let Some(at) = first {
                        d.labels.push((sid.clone(), if multi_sheet { LabelKind::Global } else { LabelKind::Local }, name.clone(), at, 0.0));
                    }
                } else if auto && segs.len() > 1 {
                    if let Some(at) = first {
                        d.labels.push((sid.clone(), LabelKind::Local, name.replace('$', "_"), at, 0.0));
                    }
                }
            }
        }
        for t in sh.child("plain").into_iter().flat_map(|p| p.all("text")) {
            d.texts.push((sid.clone(), pt(t, "x", "y"), t.text.trim().to_string(), t.numor("size", 1.778)));
        }
    }
    let _ = sch::rect;
    Ok(d)
}
