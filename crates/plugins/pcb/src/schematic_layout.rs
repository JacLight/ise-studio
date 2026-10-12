//! Netlist → schematic: a readable sheet drawn from parts and nets, the way
//! a person draws one with net labels.
//!
//! - Each IC, module, regulator and connector is an *anchor*: its connected
//!   pins get a short wire and a net label, or a power port for supply nets
//!   (a global label where a port would collide with the next pin); unused
//!   pins get no-connect flags.
//! - Two-pin parts (resistors, capacitors, LEDs, switches…) hang in a row
//!   under the anchor they share the most signal pins with, upright, supply
//!   on top and ground at the bottom; series pairs (an LED and its resistor)
//!   are stacked and wired directly.
//! - Blocks flow left to right (power input, regulators, processors, other
//!   connectors), wrapping rows; the paper grows A4 → A3 → A2 to fit.
//! - Supply nets that only feed power inputs get a PWR_FLAG so the
//!   electrical-rules check knows where the supply comes in.

use crate::schematic::{self, LabelKind, SchLabel, SchNoConnect, SchSymbol, SchWire, is_ground, is_power_net, snap};
use crate::schematic_view::{label_view, symbol_view};
use crate::symbols::{self, Dir, PinSpec, PinType, SymbolDef};
use crate::{bad, natural};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// A part of the netlist.
#[derive(Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct NetlistPart {
    pub reference: String,
    /// Library symbol; guessed from the footprint and reference when absent.
    pub symbol: Option<String>,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub footprint: String,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dnp: bool,
    #[serde(default)]
    pub pin_map: BTreeMap<String, Vec<String>>,
    /// Pin table for a part the library lacks.
    #[serde(default)]
    pub pins: Vec<PinSpec>,
    /// Pad → pin function, used to name pins of a guessed box symbol.
    #[serde(default)]
    pub pin_names: BTreeMap<String, String>,
}

/// `pcb.schematic.from_netlist`: draw parts and a netlist (net → pads,
/// "U1.3" as in pcb.netlist.set) onto a sheet, laid out for reading.
#[derive(Deserialize, JsonSchema)]
pub struct FromNetlist {
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub parts: Vec<NetlistPart>,
    pub nets: BTreeMap<String, Vec<String>>,
    /// Add PWR_FLAGs on supplies nothing drives (default true).
    pub flags: Option<bool>,
}

#[derive(Clone, Debug)]
struct Part {
    reference: String,
    symbol: String,
    custom: Vec<PinSpec>,
    value: String,
    footprint: String,
    mpn: Option<String>,
    manufacturer: Option<String>,
    lcsc: Option<String>,
    description: String,
    dnp: bool,
    pin_map: BTreeMap<String, Vec<String>>,
    def: SymbolDef,
    /// Symbol pin number → net.
    nets: BTreeMap<String, String>,
}

impl Part {
    fn sym(&self, schematic: StableId, sheet: &str, unit: u32, at: [f64; 2], rotation: f64) -> SchSymbol {
        SchSymbol {
            schematic,
            sheet: sheet.into(),
            symbol: self.symbol.clone(),
            reference: self.reference.clone(),
            value: self.value.clone(),
            footprint: self.footprint.clone(),
            at,
            rotation,
            mirror: false,
            unit,
            mpn: self.mpn.clone(),
            manufacturer: self.manufacturer.clone(),
            lcsc: self.lcsc.clone(),
            description: self.description.clone(),
            dnp: self.dnp,
            pin_map: self.pin_map.clone(),
            pins: self.custom.clone(),
            fields: BTreeMap::new(),
            ref_at: None,
            value_at: None,
            exclude_from_board: false,
            component: None,
        }
    }
    fn two_pin(&self) -> bool {
        self.def.pins.iter().filter(|p| !p.hidden).count() == 2 && matches!(self.def.category.as_str(), "Passive" | "Diode" | "Misc")
    }
}

/// A library symbol for a part, from its footprint, value and reference.
pub fn guess_symbol(p: &NetlistPart) -> Option<String> {
    let f = p.footprint.as_str();
    let r = p.reference.trim_end_matches(|c: char| c.is_ascii_digit()).to_ascii_uppercase();
    let v = format!("{} {}", p.value, p.mpn.clone().unwrap_or_default()).to_ascii_uppercase();
    if f.starts_with("MountingHole") || f.starts_with("Fiducial") {
        return None;
    }
    let s = if f == "ESP32-WROOM-32" {
        "ESP32-WROOM-32E".to_string()
    } else if f == "USB-C_16P" {
        "USB_C_16P".into()
    } else if f == "SOT-23-5" && (v.contains("6211") || v.contains("LDO") || v.contains("6220") || v.contains("9013") || v.contains("2112") || v.contains("3.3V")) {
        "LDO_SOT23-5".into()
    } else if f == "SOT-223" && (v.contains("1117") || v.contains("LDO") || r == "U") {
        "LDO_SOT223".into()
    } else if let Some(rest) = f.strip_prefix("PinHeader_1x").and_then(|x| x.strip_suffix("_2.54")) {
        format!("Conn_01x{rest}")
    } else if let Some(rest) = f.strip_prefix("PinHeader_2x").and_then(|x| x.strip_suffix("_2.54")) {
        format!("Conn_02x{rest}")
    } else if f.starts_with("JST_PH_") || f.starts_with("JST_XH_") {
        f.to_string()
    } else if f.starts_with("SW_") || r == "SW" || r == "S" {
        "SW_Push".into()
    } else if f.starts_with("LED_") || r == "LED" {
        "LED".into()
    } else if f == "SOT-23" && r == "Q" {
        if v.contains("3906") || v.contains("PNP") {
            "Q_PNP".into()
        } else if v.contains("3904") || v.contains("NPN") || v.contains("BC8") {
            "Q_NPN".into()
        } else if v.contains("3401") || v.contains("PMOS") {
            "Q_PMOS".into()
        } else {
            "Q_NMOS".into()
        }
    } else {
        match r.as_str() {
            "R" => "R".into(),
            "C" => "C".into(),
            "L" => "L".into(),
            "F" => "Fuse".into(),
            "Y" | "X" => "Crystal".into(),
            "D" => "D".into(),
            _ => return Some("IC".into()),
        }
    };
    Some(s)
}

fn build_parts(parts: &[NetlistPart], nets: &BTreeMap<String, Vec<String>>) -> Result<(Vec<Part>, Vec<String>), CommandError> {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    // Pad nets per part.
    let mut pad_net: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (net, pins) in nets {
        for pin in pins {
            let (r, pad) = pin.rsplit_once('.').ok_or_else(|| bad(format!("“{pin}”: write pins as REF.PAD (U1.3)")))?;
            if !parts.iter().any(|p| p.reference.eq_ignore_ascii_case(r)) {
                return Err(bad(format!("{pin}: no part {r} in `parts`")));
            }
            pad_net.entry(r.to_ascii_uppercase()).or_default().insert(pad.to_string(), net.trim().to_string());
        }
    }
    for p in parts {
        let pads = pad_net.remove(&p.reference.to_ascii_uppercase()).unwrap_or_default();
        let Some(mut symbol) = p.symbol.clone().or_else(|| guess_symbol(p)) else { continue };
        let mut custom = p.pins.clone();
        let mut def = if !custom.is_empty() { Some(symbols::custom(&symbol, &custom)) } else { symbols::get(&symbol) };
        // A symbol whose pins miss a connected pad: draw a box from the footprint instead.
        let covers = |d: &SymbolDef| pads.keys().all(|pad| d.pins.iter().any(|x| p.pin_map.get(&x.number).cloned().unwrap_or_else(|| x.pads()).contains(pad)));
        if def.as_ref().is_none_or(|d| !covers(d)) && p.symbol.is_none() {
            let fp = crate::footprints::get(&p.footprint);
            let mut numbers: Vec<String> = fp.map(|f| f.pads.iter().map(|x| x.number.clone()).filter(|n| !n.is_empty()).collect()).unwrap_or_else(|| pads.keys().cloned().collect());
            numbers.sort_by(|a, b| natural(a, b));
            numbers.dedup();
            custom = numbers
                .iter()
                .map(|n| {
                    let name = p.pin_names.get(n).cloned().unwrap_or_else(|| n.clone());
                    let up = name.to_ascii_uppercase();
                    let etype = if is_ground(&up) || ["VCC", "VDD", "3V3", "5V", "VIN", "VBUS"].contains(&up.as_str()) { PinType::PowerIn } else { PinType::Passive };
                    PinSpec { number: n.clone(), name, etype, side: None, pads: vec![], hidden: false }
                })
                .collect();
            symbol = if p.value.trim().is_empty() { p.reference.clone() } else { p.value.trim().to_string() };
            def = Some(symbols::custom(&symbol, &custom));
            if p.symbol.is_none() && !matches!(guess_symbol(p).as_deref(), Some("IC")) {
                warnings.push(format!("{}: drew a box from its footprint's pads (no library symbol fits)", p.reference));
            }
        }
        let def = def.ok_or_else(|| bad(format!("{}: no symbol “{symbol}”", p.reference)))?;
        let mut pin_nets = BTreeMap::new();
        for (pad, net) in &pads {
            let pin = def.pins.iter().find(|x| p.pin_map.get(&x.number).cloned().unwrap_or_else(|| x.pads()).contains(pad));
            match pin {
                Some(x) => {
                    if let Some(other) = pin_nets.insert(x.number.clone(), net.clone()) {
                        if &other != net {
                            return Err(bad(format!("{}: pads of pin {} are on {other} and {net}", p.reference, x.number)));
                        }
                    }
                }
                None => warnings.push(format!("{}.{pad} ({net}) has no pin on symbol {}", p.reference, def.name)),
            }
        }
        out.push(Part {
            reference: p.reference.trim().to_string(),
            symbol,
            custom,
            value: if p.value.trim().is_empty() { def.value.clone() } else { p.value.trim().to_string() },
            footprint: if p.footprint.trim().is_empty() { def.footprint.clone() } else { p.footprint.trim().to_string() },
            mpn: p.mpn.clone().filter(|s| !s.trim().is_empty()).or(def.mpn.clone()),
            manufacturer: p.manufacturer.clone().filter(|s| !s.trim().is_empty()).or(def.manufacturer.clone()),
            lcsc: p.lcsc.clone().filter(|s| !s.trim().is_empty()).or(def.lcsc.clone()),
            description: p.description.clone(),
            dnp: p.dnp,
            pin_map: p.pin_map.clone(),
            def,
            nets: pin_nets,
        });
    }
    out.sort_by(|a, b| natural(&a.reference, &b.reference));
    Ok((out, warnings))
}

/// Items drawn in local coordinates, then moved as a block.
#[derive(Default, Clone)]
struct Items {
    symbols: Vec<SchSymbol>,
    wires: Vec<Vec<[f64; 2]>>,
    labels: Vec<(LabelKind, String, [f64; 2], f64)>,
    ncs: Vec<[f64; 2]>,
}

impl Items {
    fn extend(&mut self, o: Items, d: [f64; 2]) {
        let m = |p: [f64; 2]| [p[0] + d[0], p[1] + d[1]];
        self.symbols.extend(o.symbols.into_iter().map(|mut s| {
            s.at = m(s.at);
            s
        }));
        self.wires.extend(o.wires.into_iter().map(|w| w.into_iter().map(m).collect()));
        self.labels.extend(o.labels.into_iter().map(|(k, t, a, r)| (k, t, m(a), r)));
        self.ncs.extend(o.ncs.into_iter().map(m));
    }
    fn bbox(&self, schematic: StableId) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut add = |r: [f64; 4]| {
            b[0] = b[0].min(r[0]);
            b[1] = b[1].min(r[1]);
            b[2] = b[2].max(r[2]);
            b[3] = b[3].max(r[3]);
        };
        for s in &self.symbols {
            add(symbol_view(StableId::new(), s).bbox);
        }
        for w in &self.wires {
            for p in w {
                add([p[0], p[1], p[0], p[1]]);
            }
        }
        for (k, t, a, r) in &self.labels {
            let l = SchLabel { schematic, sheet: "1".into(), kind: *k, text: t.clone(), at: *a, rotation: *r, shape: None, style: None };
            add(label_view(StableId::new(), &l).bbox);
        }
        if !b[0].is_finite() {
            return [0.0, 0.0, 0.0, 0.0];
        }
        b
    }
}

fn label_kind(net: &str) -> LabelKind {
    if is_power_net(net) { LabelKind::Power } else { LabelKind::Local }
}

/// An anchor part (every unit side by side), its pins labelled or flagged.
fn anchor(part: &Part, schematic: StableId) -> Items {
    let mut it = Items::default();
    let mut x = 0.0;
    for unit in 1..=part.def.unit_count() {
        let mut s = part.sym(schematic, "1", unit, [0.0, 0.0], 0.0);
        let body = part.def.body_bbox(unit);
        s.at = snap([x - body[0], 0.0]);
        let pins: Vec<&symbols::PinDef> = part.def.unit_pins(unit).filter(|p| !p.hidden).collect();
        for p in &pins {
            let at = s.place(p.at);
            let dir = s.dir(p.side);
            let Some(net) = part.nets.get(&p.number) else {
                if p.etype != PinType::NoConnect {
                    it.ncs.push(at);
                }
                continue;
            };
            let v = dir.v();
            let end = snap([at[0] + v[0] * 2.54, at[1] + v[1] * 2.54]);
            it.wires.push(vec![at, end]);
            let mut kind = label_kind(net);
            let mut rot = schematic::rotation_for(dir);
            if kind == LabelKind::Power {
                let ground = is_ground(net);
                rot = if ground { 270.0 } else { 90.0 };
                let points_into_pin = (ground && dir == Dir::Up) || (!ground && dir == Dir::Down);
                // A port hanging over the neighbouring pin's wire would read as joined: label it instead.
                let step = if ground { 2.54 } else { -2.54 };
                let neighbour = matches!(dir, Dir::Left | Dir::Right)
                    && pins.iter().any(|q| {
                        let qa = s.place(q.at);
                        s.dir(q.side) == dir && (qa[0] - at[0]).abs() < 1e-6 && (qa[1] - (at[1] + step)).abs() < 1e-6 && part.nets.contains_key(&q.number)
                    });
                if points_into_pin && dir == Dir::Down {
                    rot = 270.0;
                } else if points_into_pin || neighbour {
                    kind = LabelKind::Global;
                    rot = schematic::rotation_for(dir);
                }
            }
            it.labels.push((kind, net.clone(), end, rot));
        }
        let vb = symbol_view(StableId::new(), &s).bbox;
        x = vb[2] + 15.24 + 10.0;
        it.symbols.push(s);
    }
    it
}

/// The rotation that puts pin `top` up and the other pin down.
fn upright(def: &SymbolDef, top: &str) -> f64 {
    for rot in [0.0, 90.0, 180.0, 270.0] {
        let s = SchSymbol { pins: vec![], ..dummy(def, rot) };
        if def.pins.iter().all(|p| {
            let d = s.dir(p.side);
            if p.number == top { d == Dir::Up } else { d == Dir::Down }
        }) {
            return rot;
        }
    }
    0.0
}

fn dummy(def: &SymbolDef, rotation: f64) -> SchSymbol {
    SchSymbol {
        schematic: StableId::new(),
        sheet: "1".into(),
        symbol: def.name.clone(),
        reference: "X".into(),
        value: String::new(),
        footprint: String::new(),
        at: [0.0, 0.0],
        rotation,
        mirror: false,
        unit: 1,
        mpn: None,
        manufacturer: None,
        lcsc: None,
        description: String::new(),
        dnp: false,
        pin_map: BTreeMap::new(),
        pins: vec![],
        fields: BTreeMap::new(),
        ref_at: None,
        value_at: None,
        exclude_from_board: false,
        component: None,
    }
}

/// A column of two-pin parts in series, top net up; local y 0 is the top label.
fn column(chain: &[(usize, String)], parts: &[Part], schematic: StableId) -> Items {
    let mut it = Items::default();
    let mut y = 2.54; // top pin
    let mut prev_bottom: Option<[f64; 2]> = None;
    for (k, (pi, top)) in chain.iter().enumerate() {
        let part = &parts[*pi];
        let rot = upright(&part.def, top);
        let mut s = part.sym(schematic, "1", 1, [0.0, 0.0], rot);
        let pins: Vec<_> = part.def.pins.iter().filter(|p| !p.hidden).collect();
        let tp = pins.iter().find(|p| &p.number == top).unwrap();
        let bp = pins.iter().find(|p| &p.number != top).unwrap();
        let t_local = s.place(tp.at);
        s.at = snap([-t_local[0], y - t_local[1]]);
        let t = s.place(tp.at);
        let b = s.place(bp.at);
        if k == 0 {
            if let Some(net) = part.nets.get(top) {
                it.wires.push(vec![[t[0], 0.0], t]);
                let kind = label_kind(net);
                let rot = if kind == LabelKind::Power { if is_ground(net) { 270.0 } else { 90.0 } } else { 0.0 };
                let kind = if kind == LabelKind::Power && is_ground(net) { LabelKind::Global } else { kind };
                let rot = if kind == LabelKind::Global { 90.0 } else { rot };
                it.labels.push((kind, net.clone(), [t[0], 0.0], rot));
            } else {
                it.ncs.push(t);
            }
        }
        if let Some(pb) = prev_bottom {
            it.wires.push(vec![pb, t]);
            // Keep the series net's name where it has one.
            if let Some(net) = part.nets.get(top).filter(|n| !n.starts_with("Net-(")) {
                it.labels.push((LabelKind::Local, net.clone(), [t[0], (pb[1] + t[1]) / 2.0], 0.0));
            }
        }
        prev_bottom = Some(b);
        y = b[1] + 2.54;
        if k == chain.len() - 1 {
            if let Some(net) = part.nets.get(&bp.number) {
                let end = [b[0], b[1] + 2.54];
                it.wires.push(vec![b, end]);
                let kind = label_kind(net);
                let (kind, rot) = match kind {
                    LabelKind::Power if is_ground(net) => (LabelKind::Power, 270.0),
                    LabelKind::Power => (LabelKind::Power, 270.0),
                    _ => (LabelKind::Local, 0.0),
                };
                it.labels.push((kind, net.clone(), end, rot));
            } else {
                it.ncs.push(b);
            }
        }
        it.symbols.push(s);
    }
    it
}

fn r(v: f64) -> f64 {
    (v / 2.54).round() * 2.54
}

struct Layout {
    items: Items,
    paper: &'static str,
}

fn layout(parts: &[Part], schematic: StableId, flags: bool) -> Layout {
    let two: Vec<usize> = (0..parts.len()).filter(|i| parts[*i].two_pin()).collect();
    let anchors: Vec<usize> = (0..parts.len()).filter(|i| !parts[*i].two_pin()).collect();
    // Net → (part, pin) list.
    let mut net_pins: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for (i, p) in parts.iter().enumerate() {
        for (pin, net) in &p.nets {
            net_pins.entry(net.clone()).or_default().push((i, pin.clone()));
        }
    }
    // Series chains: a signal net joining exactly two two-pin parts.
    let mut link: BTreeMap<usize, Vec<(usize, String, String)>> = BTreeMap::new(); // part → (other, my pin, net)
    for (net, list) in &net_pins {
        if list.len() == 2 && !is_power_net(net) && two.contains(&list[0].0) && two.contains(&list[1].0) && list[0].0 != list[1].0 {
            link.entry(list[0].0).or_default().push((list[1].0, list[0].1.clone(), net.clone()));
            link.entry(list[1].0).or_default().push((list[0].0, list[1].1.clone(), net.clone()));
        }
    }
    let mut done: BTreeSet<usize> = BTreeSet::new();
    let mut chains: Vec<Vec<(usize, String)>> = Vec::new();
    let other_pin = |i: usize, pin: &str| parts[i].def.pins.iter().filter(|p| !p.hidden).find(|p| p.number != pin).map(|p| p.number.clone()).unwrap_or_default();
    let net_of = |i: usize, pin: &str| parts[i].nets.get(pin).cloned();
    let top_score = |net: Option<&String>| match net {
        Some(n) if is_ground(n) => 0,
        Some(n) if is_power_net(n) => 3,
        Some(_) => 2,
        None => 1,
    };
    for &i in &two {
        if done.contains(&i) || link.get(&i).is_some_and(|l| l.len() > 1) {
            continue;
        }
        // Walk from an end of a chain (or a lone part).
        let mut seq = vec![i];
        done.insert(i);
        let mut cur = i;
        while let Some((next, _, _)) = link.get(&cur).and_then(|l| l.iter().find(|(o, _, _)| !done.contains(o))).cloned() {
            seq.push(next);
            done.insert(next);
            cur = next;
        }
        // Pins facing up through the chain.
        let ends = |seq: &[usize]| -> Vec<(usize, String)> {
            let mut out = Vec::new();
            let first = seq[0];
            let inner_first = if seq.len() > 1 { link[&first].iter().find(|(o, _, _)| *o == seq[1]).map(|x| x.1.clone()).unwrap() } else { String::new() };
            let mut top = if seq.len() > 1 { other_pin(first, &inner_first) } else { parts[first].def.pins.iter().find(|p| !p.hidden).map(|p| p.number.clone()).unwrap_or_default() };
            for (k, &pi) in seq.iter().enumerate() {
                out.push((pi, top.clone()));
                if k + 1 < seq.len() {
                    let bottom = other_pin(pi, &top);
                    let net = net_of(pi, &bottom);
                    let nxt = seq[k + 1];
                    top = parts[nxt].nets.iter().find(|(_, n)| Some(*n) == net.as_ref()).map(|(p, _)| p.clone()).unwrap_or_else(|| other_pin(nxt, ""));
                }
            }
            out
        };
        let mut c = ends(&seq);
        let last = c.last().unwrap().clone();
        let top_net = net_of(c[0].0, &c[0].1);
        let bottom_net = net_of(last.0, &other_pin(last.0, &last.1));
        if top_score(bottom_net.as_ref()) > top_score(top_net.as_ref()) {
            seq.reverse();
            c = ends(&seq);
        }
        chains.push(c);
    }
    for &i in &two {
        if !done.contains(&i) {
            let first = parts[i].def.pins.iter().find(|p| !p.hidden).map(|p| p.number.clone()).unwrap_or_default();
            chains.push(vec![(i, first)]);
        }
    }
    // Anchor order: supply inputs, regulators, processors, other connectors.
    let rank = |i: usize| {
        let p = &parts[i];
        let supply_conn = p.def.category == "Connector" && p.nets.values().any(|n| is_power_net(n) && !is_ground(n)) && p.nets.values().all(|n| is_power_net(n));
        let powered_conn = p.def.category == "Connector" && p.nets.values().any(|n| is_power_net(n) && !is_ground(n));
        match p.def.category.as_str() {
            _ if powered_conn && p.def.name.starts_with("USB") => 0,
            "Power IC" => 2,
            "Module" | "IC" => 3,
            _ if supply_conn => 1,
            "Connector" => 4,
            _ => 5,
        }
    };
    let mut anchors = anchors;
    anchors.sort_by(|a, b| rank(*a).cmp(&rank(*b)).then(natural(&parts[*a].reference, &parts[*b].reference)));
    // Each chain joins the anchor it shares the most pins with (signals count most).
    let mut groups: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (ci, c) in chains.iter().enumerate() {
        let mut nets: BTreeSet<String> = BTreeSet::new();
        for (pi, _) in c {
            nets.extend(parts[*pi].nets.values().cloned());
        }
        let mut best: Option<(usize, usize)> = None;
        for &a in &anchors {
            let mut score = 0;
            for n in &nets {
                let w = if is_ground(n) { 0 } else if is_power_net(n) { 2 } else { 10 };
                score += 4 * w * parts[a].nets.values().filter(|x| *x == n).count();
            }
            // Ties go to processors and regulators before connectors.
            if score > 0 && matches!(parts[a].def.category.as_str(), "IC" | "Module" | "Power IC") {
                score += 1;
            }
            if score > 0 && best.is_none_or(|(s, _)| score > s) {
                best = Some((score, a));
            }
        }
        groups.entry(best.map(|b| b.1)).or_default().push(ci);
    }
    // Blocks.
    let mut blocks: Vec<Items> = Vec::new();
    let mut make_block = |a: Option<usize>, cols: &[usize]| {
        let mut b = Items::default();
        let mut ab = [0.0, 0.0, 0.0, 0.0];
        if let Some(a) = a {
            let it = anchor(&parts[a], schematic);
            ab = it.bbox(schematic);
            b.extend(it, [0.0, 0.0]);
        }
        let mut x = ab[0];
        let top = if a.is_some() { ab[3] + 3.81 } else { 0.0 };
        for &ci in cols {
            let col = column(&chains[ci], parts, schematic);
            let cb = col.bbox(schematic);
            let d = [r(x - cb[0]) + 2.54, r(top - cb[1]) + 2.54];
            b.extend(col, d);
            x = d[0] + cb[2] + 2.54;
        }
        blocks.push(b);
    };
    for &a in &anchors {
        let cols = groups.get(&Some(a)).cloned().unwrap_or_default();
        make_block(Some(a), &cols);
    }
    if let Some(cols) = groups.get(&None) {
        for chunk in cols.chunks(8) {
            make_block(None, chunk);
        }
    }
    // Power flags on supplies that only feed inputs.
    if flags {
        let mut flagged = Vec::new();
        for (net, list) in &net_pins {
            let types: Vec<PinType> = list.iter().filter_map(|(pi, pin)| parts[*pi].def.pins.iter().find(|p| &p.number == pin).map(|p| p.etype)).collect();
            if types.contains(&PinType::PowerIn) && !types.contains(&PinType::PowerOut) {
                flagged.push(net.clone());
            }
        }
        if !flagged.is_empty() {
            let flag = symbols::get("PWR_FLAG").unwrap();
            let mut b = Items::default();
            for (k, net) in flagged.iter().enumerate() {
                let x = k as f64 * 15.24;
                let p = [x, 7.62];
                let ground = is_ground(net);
                b.labels.push((if is_power_net(net) { LabelKind::Power } else { LabelKind::Global }, net.clone(), p, if ground { 270.0 } else if is_power_net(net) { 90.0 } else { 180.0 }));
                b.wires.push(vec![p, [x + 5.08, 7.62]]);
                let mut s = dummy(&flag, 0.0);
                s.schematic = schematic;
                s.symbol = "PWR_FLAG".into();
                s.reference = format!("#FLG{:02}", k + 1);
                s.value = "PWR_FLAG".into();
                s.at = [x + 5.08, 7.62];
                b.symbols.push(s);
            }
            blocks.push(b);
        }
    }
    // Shelf packing on the paper's drawing area, growing the paper to fit.
    for (paper, w, h) in [("A4", 297.0, 210.0), ("A3", 420.0, 297.0), ("A2", 594.0, 420.0), ("A1", 841.0, 594.0)] {
        let (x0, y0, x1, y1) = (20.32, 20.32, w - 17.0, h - 17.0);
        let title = [w - 12.0 - 110.0 - 3.0, h - 12.0 - 32.0 - 3.0];
        let mut out = Items::default();
        let mut x = x0;
        let mut y = y0;
        let mut row_h: f64 = 0.0;
        let mut fits = true;
        for b in &blocks {
            let bb = b.bbox(schematic);
            let (bw, bh) = (bb[2] - bb[0], bb[3] - bb[1]);
            if x > x0 && x + bw > x1 {
                x = x0;
                y += row_h + 10.16;
                row_h = 0.0;
            }
            // Keep clear of the title block.
            if x + bw > title[0] && y + bh > title[1] {
                x = x0;
                y += row_h + 10.16;
                row_h = 0.0;
            }
            if y + bh > y1 || x + bw > x1 {
                fits = false;
                break;
            }
            out.extend(b.clone(), [r(x - bb[0]), r(y - bb[1])]);
            x += bw + 12.7;
            row_h = row_h.max(bh);
        }
        if fits || paper == "A1" {
            return Layout { items: out, paper };
        }
    }
    unreachable!()
}

fn place(tx: &mut Tx, schematic: StableId, sheet: &str, items: Items) -> Result<(usize, usize, usize, usize), CommandError> {
    let mut counts = (0, 0, 0, 0);
    for mut s in items.symbols {
        s.schematic = schematic;
        s.sheet = sheet.into();
        let id = tx.create();
        tx.set(id, s)?;
        tx.add_dependency(schematic, id)?;
        counts.0 += 1;
    }
    for w in items.wires {
        let pts = schematic::simplify(&w);
        if pts.len() < 2 {
            continue;
        }
        let id = tx.create();
        tx.set(id, SchWire { schematic, sheet: sheet.into(), points: pts })?;
        tx.add_dependency(schematic, id)?;
        counts.1 += 1;
    }
    for (kind, text, at, rotation) in items.labels {
        let id = tx.create();
        tx.set(id, SchLabel { schematic, sheet: sheet.into(), kind, text, at, rotation, shape: None, style: None })?;
        tx.add_dependency(schematic, id)?;
        counts.2 += 1;
    }
    for at in items.ncs {
        let id = tx.create();
        tx.set(id, SchNoConnect { schematic, sheet: sheet.into(), at })?;
        tx.add_dependency(schematic, id)?;
        counts.3 += 1;
    }
    Ok(counts)
}

impl Command for FromNetlist {
    const NAME: &'static str = "pcb.schematic.from_netlist";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = tx.get::<schematic::Schematic>(self.schematic).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a schematic", self.schematic)))?;
        let sheet = match &self.sheet {
            Some(x) => s.sheet(x).map(|x| x.id.clone()).ok_or_else(|| bad(format!("no sheet {x}")))?,
            None => s.sheets[0].id.clone(),
        };
        let it = schematic::items(tx.world(), self.schematic);
        if it.symbols.iter().any(|(_, x)| x.sheet == sheet) {
            return Err(bad("the sheet already has symbols: draw the netlist on an empty sheet (add one with pcb.schematic.sheet)"));
        }
        let (parts, warnings) = build_parts(&self.parts, &self.nets)?;
        if parts.is_empty() {
            return Err(bad("no parts to draw"));
        }
        let l = layout(&parts, self.schematic, self.flags.unwrap_or(true));
        if let Some(sh) = s.sheets.iter_mut().find(|x| x.id == sheet) {
            sh.paper = l.paper.into();
            sh.portrait = false;
        }
        tx.set(self.schematic, s)?;
        let (symbols, wires, labels, ncs) = place(tx, self.schematic, &sheet, l.items)?;
        Ok(json!({ "schematic": self.schematic, "sheet": sheet, "paper": l.paper, "symbols": symbols, "wires": wires, "labels": labels, "no_connects": ncs, "warnings": warnings }))
    }
}

/// `pcb.schematic.from_board`: draw a schematic for an existing board from
/// its components and pad nets (mounting holes stay board-only), linked to
/// it so the board can then be updated from the schematic.
#[derive(Deserialize, JsonSchema)]
pub struct FromBoard {
    pub board: StableId,
    pub name: Option<String>,
}

impl Command for FromBoard {
    const NAME: &'static str = "pcb.schematic.from_board";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let b = tx.get::<crate::Board>(self.board).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a board", self.board)))?;
        if let Some((other, _)) = schematic::of_board(tx.world(), self.board) {
            return Err(bad(format!("the board already has a schematic ({other})")));
        }
        let out = tx.run("pcb.schematic.create", json!({ "name": self.name.unwrap_or_else(|| b.name.clone()), "board": self.board, "revision": b.revision }))?;
        let sid: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        let comps = crate::items(tx.world(), self.board).components;
        let mut parts = Vec::new();
        let mut nets: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (_, c) in &comps {
            let p = NetlistPart {
                reference: c.reference.clone(),
                symbol: None,
                value: c.value.clone(),
                footprint: c.footprint.clone(),
                mpn: c.mpn.clone(),
                manufacturer: c.manufacturer.clone(),
                lcsc: c.lcsc.clone(),
                description: c.description.clone(),
                dnp: c.dnp,
                pin_map: BTreeMap::new(),
                pins: vec![],
                pin_names: c.pin_names.clone(),
            };
            if guess_symbol(&p).is_none() {
                continue;
            }
            for (pad, net) in &c.pins {
                nets.entry(net.clone()).or_default().push(format!("{}.{pad}", c.reference));
            }
            parts.push(p);
        }
        let drawn = FromNetlist { schematic: sid, sheet: None, parts, nets, flags: Some(true) }.apply(tx)?;
        // Link symbols to the components they came from.
        let it = schematic::items(tx.world(), sid);
        for (id, sym) in it.symbols {
            if let Some((cid, _)) = comps.iter().find(|(_, c)| c.reference == sym.reference) {
                tx.set(id, SchSymbol { component: Some(*cid), ..sym })?;
            }
        }
        Ok(json!({ "id": sid, "drawn": drawn }))
    }
}
