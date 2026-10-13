//! PADS ASCII boards (`.asc`, "PowerPCB"/"PADS Layout" database files): a
//! header line naming the units (`!PADS-POWERPCB-V9.0-MILS!`, `-METRIC!`,
//! `-INCHES!`), then `*SECTION*` blocks. Read:
//! - `*PCB*`: the copper layer count (`NUMBER OF LAYERS` / `LAYERS`)
//! - `*LINES*`: drawing items; `BOARD` items are the outline
//! - `*PARTDECAL*`: footprints: `name units x y pieces terminals stacks texts labels`, then the
//!   pieces (`type corners width layer` + corners), terminals (`T x y nx ny pin`) and pad stacks
//!   (`PAD pin lines [P|N drill]` + `layer size shape …`; layer −2 top, −1 inner, 0 bottom; shapes
//!   R round, S square, OF/RF oval or rectangular fingers, RT/ST rounded/chamfered)
//! - `*PART*`: `ref type@decal x y rotation glue mirror …` (mirror `M`: bottom)
//! - `*SIGNAL*` blocks in `*NET*`: pin references `REF.PIN`
//! - `*ROUTE*`: per signal, runs of `x y layer width …` corner lines (a via named at a corner)
//! - `*VIA*`: via stacks.

use super::build::{self, BoardIn, PadIn, PartIn};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{self, P};
use crate::kicad::KVia;
use crate::Side;
use std::collections::BTreeMap;

struct Decal {
    units: f64,
    terminals: Vec<(P, String)>,
    /// pin ("0": every pin) → (shape, size, drill, rotation)
    stacks: BTreeMap<String, (PadShape, [f64; 2], f64, f64)>,
    silk: Vec<Vec<P>>,
}

fn num(s: &str) -> f64 {
    s.trim().parse().unwrap_or(0.0)
}

/// A pad stack line: (shape, [w, h], rotation) from `layer size shape [orient length offset]`.
fn stack_shape(t: &[&str], k: f64) -> Option<(PadShape, [f64; 2], f64)> {
    let size = num(t.get(1)?) * k;
    if size <= 0.0 {
        return None;
    }
    let shape = t.get(2).copied().unwrap_or("R");
    Some(match shape {
        "R" => (PadShape::Circle, [size, size], 0.0),
        "S" => (PadShape::Rect, [size, size], num(t.get(3).unwrap_or(&"0"))),
        "A" => (PadShape::Circle, [size, size], 0.0),
        "OF" => (PadShape::Oval, [num(t.get(4).unwrap_or(&"0")) * k, size], num(t.get(3).unwrap_or(&"0"))),
        "RF" => (PadShape::Rect, [num(t.get(4).unwrap_or(&"0")) * k, size], num(t.get(3).unwrap_or(&"0"))),
        "RT" | "ST" => (PadShape::RoundRect, [size, size], num(t.get(3).unwrap_or(&"0"))),
        _ => (PadShape::Circle, [size, size], 0.0),
    })
}

pub fn read(text: &str) -> Result<BoardIn, String> {
    let mut lines = text.lines().map(|l| l.trim_end()).peekable();
    let head = lines.next().unwrap_or("");
    if !head.starts_with("!PADS-") {
        return Err("not a PADS ASCII file".into());
    }
    let up = head.to_ascii_uppercase();
    let k = if up.contains("-METRIC") {
        1.0
    } else if up.contains("-INCHES") {
        25.4
    } else if up.contains("-BASIC") {
        return Err("PADS files in basic units are not read; export in mils or metric".into());
    } else {
        0.0254
    };
    let all: Vec<&str> = lines.collect();
    let mut b = BoardIn::default();
    let mut layers = 2usize;
    let mut decals: BTreeMap<String, Decal> = BTreeMap::new();
    let mut parts: Vec<PartIn> = Vec::new();
    let mut part_decal: Vec<String> = Vec::new();
    let mut pin_net: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut vias: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut section = String::new();
    let mut i = 0;
    let toks = |l: &str| -> Vec<String> { l.split_whitespace().map(str::to_string).collect() };
    let mut route_net: Option<String> = None;
    let mut route_prev: Option<(P, usize, f64)> = None;
    let mut pending_routes: Vec<(Option<String>, usize, f64, P, P)> = Vec::new();
    let mut pending_vias: Vec<(Option<String>, P, String)> = Vec::new();
    while i < all.len() {
        let l = all[i].trim();
        i += 1;
        if l.is_empty() || l.starts_with("*REMARK*") {
            continue;
        }
        if l.starts_with('*') && l[1..].contains('*') && !l.starts_with("*SIGNAL*") {
            section = l[1..].split('*').next().unwrap_or("").to_string();
            continue;
        }
        let t = toks(l);
        match section.as_str() {
            "PCB" => {
                if (t.first().map(String::as_str) == Some("LAYERS") || l.starts_with("NUMBER OF LAYERS")) && t.last().is_some_and(|x| x.parse::<usize>().is_ok()) {
                    layers = num(t.last().unwrap()) as usize;
                }
            }
            "LINES" => {
                // name type x y pieces …
                if t.len() >= 5 && t[0] != "CLOSED" && t[0] != "OPEN" && t[0] != "CIRCLE" {
                    let kind = t[1].clone();
                    let origin = [num(&t[2]) * k, num(&t[3]) * k];
                    let pieces = num(&t[4]) as usize;
                    for _ in 0..pieces {
                        let Some(ph) = all.get(i) else { break };
                        i += 1;
                        let pt = toks(ph);
                        let corners = pt.get(1).map(|x| num(x) as usize).unwrap_or(0);
                        let mut pts: Vec<P> = Vec::new();
                        for _ in 0..corners {
                            let Some(cl) = all.get(i) else { break };
                            i += 1;
                            let c = toks(cl);
                            if c.len() >= 2 {
                                pts.push([origin[0] + num(&c[0]) * k, origin[1] + num(&c[1]) * k]);
                            }
                        }
                        if pt.first().map(String::as_str) == Some("CIRCLE") && pts.len() >= 2 {
                            let c = [(pts[0][0] + pts[1][0]) / 2.0, (pts[0][1] + pts[1][1]) / 2.0];
                            let mut v = build::circle(c, geom::dist(pts[0], pts[1]) / 2.0);
                            v.push(v[0]);
                            pts = v;
                        } else if pt.first().map(String::as_str) == Some("CLOSED") && pts.first() != pts.last() {
                            if let Some(f) = pts.first().copied() {
                                pts.push(f);
                            }
                        }
                        if kind == "BOARD" {
                            b.edges.push(pts);
                        }
                    }
                }
            }
            "PARTDECAL" => {
                if t.len() >= 7 && t[1].len() == 1 {
                    let name = t[0].clone();
                    let units = if t[1] == "M" { 1.0 } else { 0.0254 };
                    let units = if up.contains("-METRIC") && t[1] != "I" { 1.0 } else { units };
                    let pieces = num(&t[4]) as usize;
                    let terms = num(&t[5]) as usize;
                    let stacks = num(&t[6]) as usize;
                    let texts = t.get(7).map(|x| num(x) as usize).unwrap_or(0);
                    let labels = t.get(8).map(|x| num(x) as usize).unwrap_or(0);
                    let mut d = Decal { units, terminals: vec![], stacks: BTreeMap::new(), silk: vec![] };
                    // Labels and texts come first in newer files: skip lines until the first piece header.
                    for _ in 0..(texts + labels) * 2 {
                        if all.get(i).is_some_and(|x| { let f = x.split_whitespace().next().unwrap_or(""); matches!(f, "OPEN" | "CLOSED" | "CIRCLE" | "COPOPN" | "COPCLS" | "COPCIR" | "KPTCLS" | "KPTCIR" | "TAG") }) {
                            break;
                        }
                        i += 1;
                    }
                    for _ in 0..pieces {
                        let Some(ph) = all.get(i) else { break };
                        i += 1;
                        let pt = toks(ph);
                        let corners = pt.get(1).map(|x| num(x) as usize).unwrap_or(0);
                        let mut pts = Vec::new();
                        for _ in 0..corners {
                            let Some(cl) = all.get(i) else { break };
                            i += 1;
                            let c = toks(cl);
                            if c.len() >= 2 {
                                pts.push([num(&c[0]) * units, num(&c[1]) * units]);
                            }
                        }
                        if matches!(pt.first().map(String::as_str), Some("OPEN") | Some("CLOSED")) && pts.len() >= 2 {
                            d.silk.push(pts);
                        }
                    }
                    for _ in 0..terms {
                        let Some(tl) = all.get(i) else { break };
                        i += 1;
                        let c = toks(tl.trim_start_matches('T'));
                        if c.len() >= 5 {
                            d.terminals.push(([num(&c[0]) * units, num(&c[1]) * units], c[4].clone()));
                        } else if c.len() >= 2 {
                            d.terminals.push(([num(&c[0]) * units, num(&c[1]) * units], (d.terminals.len() + 1).to_string()));
                        }
                    }
                    for _ in 0..stacks {
                        let Some(sl) = all.get(i) else { break };
                        i += 1;
                        let s = toks(sl);
                        if s.first().map(String::as_str) != Some("PAD") {
                            continue;
                        }
                        let pin = s.get(1).cloned().unwrap_or_else(|| "0".into());
                        let n = s.get(2).map(|x| num(x) as usize).unwrap_or(0);
                        let drill = s.iter().position(|x| x == "P" || x == "N").and_then(|j| s.get(j + 1)).map(|x| num(x) * units).unwrap_or(0.0);
                        let mut best: Option<(PadShape, [f64; 2], f64)> = None;
                        for _ in 0..n {
                            let Some(ll) = all.get(i) else { break };
                            i += 1;
                            let lt: Vec<&str> = ll.split_whitespace().collect();
                            if lt.first() == Some(&"-2") || (best.is_none() && lt.first() == Some(&"0")) {
                                if let Some(sh) = stack_shape(&lt, units) {
                                    if lt.first() == Some(&"-2") || best.is_none() {
                                        best = Some(sh);
                                    }
                                }
                            }
                        }
                        if let Some((shape, size, rot)) = best {
                            d.stacks.insert(pin, (build::fix_shape(shape, size), size, drill, rot));
                        }
                    }
                    decals.insert(name, d);
                }
            }
            "PART" => {
                if t.len() >= 7 {
                    let decal = t[1].split('@').nth(1).unwrap_or(&t[1]).to_string();
                    let ptype = t[1].split('@').next().unwrap_or("").to_string();
                    let mirror = t[6] == "M";
                    parts.push(PartIn { reference: t[0].clone(), value: ptype, footprint: decal.clone(), at: [num(&t[2]) * k, num(&t[3]) * k], rotation: num(&t[4]), side: if mirror { Side::Bottom } else { Side::Top }, ..Default::default() });
                    part_decal.push(decal);
                }
            }
            "NET" => {
                if l.starts_with("*SIGNAL*") {
                    route_net = t.get(1).cloned();
                } else if let Some(net) = &route_net {
                    for p in &t {
                        if let Some((r, pin)) = p.rsplit_once('.') {
                            pin_net.insert((r.to_string(), pin.to_string()), net.clone());
                        }
                    }
                }
            }
            "ROUTE" => {
                if l.starts_with("*SIGNAL*") {
                    route_net = t.get(1).cloned();
                    route_prev = None;
                } else if t.len() >= 2 && t[0].contains('.') && t[1].contains('.') && t[0].parse::<f64>().is_err() {
                    // A pin pair starts a new run.
                    for p in &t {
                        if let (Some((r, pin)), Some(net)) = (p.rsplit_once('.'), &route_net) {
                            pin_net.entry((r.to_string(), pin.to_string())).or_insert(net.clone());
                        }
                    }
                    route_prev = None;
                } else if t.len() >= 4 && t[0].parse::<f64>().is_ok() {
                    let at = [num(&t[0]) * k, num(&t[1]) * k];
                    let layer = num(&t[2]) as usize;
                    let width = num(&t[3]) * k;
                    if let Some((prev, pl, pw)) = route_prev {
                        if pl > 0 && pw > 0.0 {
                            pending_routes.push((route_net.clone(), pl, pw, prev, at));
                        }
                    }
                    if let Some(v) = t.iter().skip(5).find(|x| x.parse::<f64>().is_err()) {
                        pending_vias.push((route_net.clone(), at, v.clone()));
                    }
                    route_prev = Some((at, layer, width));
                }
            }
            "VIA" => {
                if t.len() >= 3 && t[1].parse::<f64>().is_ok() {
                    let drill = num(&t[1]) * k;
                    let n = num(&t[2]) as usize;
                    let mut dia: f64 = 0.0;
                    for _ in 0..n {
                        let Some(ll) = all.get(i) else { break };
                        i += 1;
                        let lt: Vec<&str> = ll.split_whitespace().collect();
                        if let Some(sz) = lt.get(1) {
                            dia = dia.max(num(sz) * k);
                        }
                    }
                    vias.insert(t[0].clone(), (drill, dia));
                }
            }
            _ => {}
        }
    }
    b.source_copper_layers = layers.max(2);
    let layer_name = |n: usize, b: &mut BoardIn| -> Option<String> {
        if n == 1 {
            Some("top".into())
        } else if n == layers.max(2) {
            Some("bottom".into())
        } else if n == 2 {
            Some("in1".into())
        } else if n == 3 {
            Some("in2".into())
        } else {
            b.note("copper on inner layers past the second was left out");
            None
        }
    };
    for (net, l, w, a, c) in pending_routes {
        if let Some(name) = layer_name(l, &mut b) {
            b.seg(net, &name, w, a, c);
        }
    }
    for (net, at, name) in pending_vias {
        let (drill, dia) = vias.get(&name).copied().unwrap_or((0.3, 0.6));
        b.vias.push(KVia { net, at, drill, diameter: dia.max(drill + 0.1) });
    }
    for (mut p, decal) in parts.into_iter().zip(part_decal) {
        let Some(d) = decals.get(&decal) else {
            b.note(format!("{}: decal {decal} is not in the file", p.reference));
            continue;
        };
        let bottom = p.side == Side::Bottom;
        let place = |q: P| geom::add(p.at, geom::rotate(if bottom { [-q[0], q[1]] } else { q }, p.rotation));
        for (at, pin) in &d.terminals {
            let st = d.stacks.get(pin).or_else(|| d.stacks.get("0")).cloned().unwrap_or((PadShape::Circle, [1.0, 1.0], 0.0, 0.0));
            let mut q = PadIn::new(pin.clone(), place(*at), st.1);
            q.shape = st.0;
            q.rotation = if bottom { p.rotation - st.3 } else { p.rotation + st.3 };
            if st.2 > 0.0 {
                q.kind = PadKind::Tht;
                q.drill = Some([st.2, st.2]);
                if q.size[0].min(q.size[1]) <= st.2 {
                    q.size = [st.2 + 0.3, st.2 + 0.3];
                }
            }
            q.net = pin_net.get(&(p.reference.clone(), pin.clone())).cloned();
            p.pads.push(q);
        }
        p.silk = d.silk.iter().map(|l| l.iter().map(|q| place(*q)).collect()).collect();
        let _ = d.units;
        b.parts.push(p);
    }
    Ok(b)
}
