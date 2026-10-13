//! gEDA/pcb boards (`.pcb`): the bracketed text format of the gEDA `pcb`
//! program (and pcb-rnd's compatible output). Numbers in `[ ]` are 1/100 mil
//! and in `( )` mils unless they carry a unit (`1.6mm`, `10mil`); y down.
//! Read: `PCB[name width height]`, `Groups(…)` (the group marked `c` is the
//! top, `s` the bottom), `Element[flags desc refdes value x y …]` with its
//! `Pin`s, `Pad`s (a line of copper: its ends and thickness) and silkscreen
//! `ElementLine`s, `Via`s, each copper `Layer`'s `Line`s, `Arc`s and
//! `Polygon`s, the `outline` layer as the board edge, and the `NetList`.

use super::build::{self, BoardIn, PadIn, PartIn};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{self, P};
use crate::kicad::{KVia, KZone};
use crate::{PadConnection, Side};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
enum Tok {
    Word(String),
    Str(String),
    Open(char),
    Close,
}

fn lex(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(&c) = it.peek() {
        if c == '#' {
            for c in it.by_ref() {
                if c == '\n' {
                    break;
                }
            }
        } else if c.is_whitespace() {
            it.next();
        } else if c == '"' {
            it.next();
            let mut s = String::new();
            while let Some(c) = it.next() {
                match c {
                    '\\' => {
                        if let Some(n) = it.next() {
                            s.push(n)
                        }
                    }
                    '"' => break,
                    c => s.push(c),
                }
            }
            out.push(Tok::Str(s));
        } else if c == '(' || c == '[' {
            it.next();
            out.push(Tok::Open(c));
        } else if c == ')' || c == ']' {
            it.next();
            out.push(Tok::Close);
        } else {
            let mut s = String::new();
            while let Some(&c) = it.peek() {
                if c.is_whitespace() || "()[]\"#".contains(c) {
                    break;
                }
                s.push(c);
                it.next();
            }
            out.push(Tok::Word(s));
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
struct Node {
    name: String,
    square: bool,
    args: Vec<Tok>,
    body: Vec<Node>,
}

fn parse(toks: &[Tok], i: &mut usize) -> Vec<Node> {
    let mut out = Vec::new();
    while *i < toks.len() {
        match &toks[*i] {
            Tok::Close => {
                *i += 1;
                return out;
            }
            Tok::Word(w) => {
                let name = w.clone();
                *i += 1;
                let mut n = Node { name, ..Default::default() };
                if let Some(Tok::Open(c)) = toks.get(*i) {
                    n.square = *c == '[';
                    *i += 1;
                    let mut depth = 0;
                    while *i < toks.len() {
                        match &toks[*i] {
                            Tok::Close if depth == 0 => {
                                *i += 1;
                                break;
                            }
                            Tok::Open(_) => depth += 1,
                            Tok::Close => depth -= 1,
                            _ => {}
                        }
                        n.args.push(toks[*i].clone());
                        *i += 1;
                    }
                    // A body in parentheses follows for elements, layers, polygons, nets.
                    if let Some(Tok::Open('(')) = toks.get(*i) {
                        *i += 1;
                        n.body = parse(toks, i);
                    }
                }
                out.push(n);
            }
            Tok::Open(_) => {
                // A bare list (polygon points: [x y]).
                *i += 1;
                let mut n = Node { name: String::new(), square: true, ..Default::default() };
                while *i < toks.len() {
                    if let Tok::Close = toks[*i] {
                        *i += 1;
                        break;
                    }
                    n.args.push(toks[*i].clone());
                    *i += 1;
                }
                out.push(n);
            }
            Tok::Str(_) => *i += 1,
        }
    }
    out
}

impl Node {
    /// Argument `k` as a length in mm.
    fn len(&self, k: usize) -> f64 {
        let Some(Tok::Word(w)) = self.args.get(k) else { return 0.0 };
        let default = if self.square { 0.0254 / 100.0 } else { 0.0254 };
        build::length(w, default).unwrap_or(0.0)
    }
    fn num(&self, k: usize) -> f64 {
        match self.args.get(k) {
            Some(Tok::Word(w)) => w.parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }
    fn text(&self, k: usize) -> String {
        match self.args.get(k) {
            Some(Tok::Str(s)) | Some(Tok::Word(s)) => s.clone(),
            _ => String::new(),
        }
    }
    /// The arguments that are not strings, by position among numbers.
    fn strs(&self) -> Vec<String> {
        self.args.iter().filter_map(|t| if let Tok::Str(s) = t { Some(s.clone()) } else { None }).collect()
    }
    fn nums(&self) -> Vec<usize> {
        self.args.iter().enumerate().filter_map(|(i, t)| if let Tok::Word(w) = t { (w.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '.')).then_some(i) } else { None }).collect()
    }
    fn flags(&self) -> String {
        self.strs().last().cloned().unwrap_or_default().to_ascii_lowercase()
    }
}

fn fl(p: P) -> P {
    [p[0], -p[1]]
}

pub fn read(text: &str) -> Result<BoardIn, String> {
    let toks = lex(text);
    let mut i = 0;
    let nodes = parse(&toks, &mut i);
    let pcb = nodes.iter().find(|n| n.name == "PCB").ok_or("not a gEDA/pcb file (no PCB[…])")?;
    let mut b = BoardIn { title: Some(pcb.text(0)).filter(|t| !t.is_empty()), ..Default::default() };
    let size = [pcb.len(1), pcb.len(2)];
    // Layer groups: which layer numbers are top, bottom, inner.
    let mut group_of: BTreeMap<u32, String> = BTreeMap::new();
    let mut inner_groups = 0;
    if let Some(g) = nodes.iter().find(|n| n.name == "Groups") {
        for grp in g.text(0).split(':') {
            let items: Vec<&str> = grp.split(',').collect();
            let side = if items.contains(&"c") {
                Some("top".to_string())
            } else if items.contains(&"s") {
                Some("bottom".to_string())
            } else {
                inner_groups += 1;
                match inner_groups {
                    1 => Some("in1".into()),
                    2 => Some("in2".into()),
                    _ => None,
                }
            };
            for it in items {
                if let (Ok(n), Some(s)) = (it.parse::<u32>(), side.clone()) {
                    group_of.insert(n, s);
                }
            }
        }
    }
    b.source_copper_layers = 2 + inner_groups;
    // Nets: "R1-1" → net.
    let mut pin_net: BTreeMap<(String, String), String> = BTreeMap::new();
    for nl in nodes.iter().filter(|n| n.name == "NetList") {
        for net in nl.body.iter().filter(|n| n.name == "Net") {
            let name = net.text(0);
            for c in net.body.iter().filter(|n| n.name == "Connect") {
                if let Some((r, p)) = c.text(0).rsplit_once('-') {
                    pin_net.insert((r.to_string(), p.to_string()), name.clone());
                }
            }
        }
    }
    for n in &nodes {
        match n.name.as_str() {
            "Element" => {
                let strs = n.strs();
                let nums = n.nums();
                // New format: [flags "desc" "name" "value" mx my tx ty dir scale tflags].
                let (desc, refdes, value) = (strs.get(1).cloned().unwrap_or_default(), strs.get(2).cloned().unwrap_or_default(), strs.get(3).cloned().unwrap_or_default());
                let mx = nums.first().map(|&k| n.len(k)).unwrap_or(0.0);
                let my = nums.get(1).map(|&k| n.len(k)).unwrap_or(0.0);
                let bottom = strs.first().is_some_and(|f| f.contains("onsolder"));
                let origin = [mx, my];
                let mut part = PartIn { reference: refdes.clone(), value, footprint: if desc.is_empty() { format!("{refdes}_fp") } else { desc }, at: fl(origin), side: if bottom { Side::Bottom } else { Side::Top }, ..Default::default() };
                for e in &n.body {
                    let en = e.nums();
                    let at = |k: usize| en.get(k).map(|&j| e.len(j)).unwrap_or(0.0);
                    match e.name.as_str() {
                        "Pin" => {
                            let c = fl(geom::add(origin, [at(0), at(1)]));
                            let thick = at(2);
                            let drill = at(5);
                            let es = e.strs();
                            let number = es.get(1).cloned().unwrap_or_default();
                            let flags = e.flags();
                            let mut p = PadIn::new(number.clone(), c, [thick, thick]);
                            p.shape = if flags.contains("square") { PadShape::Rect } else { PadShape::Circle };
                            p.drill = Some([drill, drill]);
                            p.kind = if flags.contains("hole") { PadKind::Npth } else { PadKind::Tht };
                            if p.kind == PadKind::Npth {
                                p.size = [drill, drill];
                            } else if thick <= drill {
                                p.size = [drill + 0.3, drill + 0.3];
                            }
                            p.net = pin_net.get(&(refdes.clone(), number)).cloned();
                            part.pads.push(p);
                        }
                        "Pad" => {
                            let a = fl(geom::add(origin, [at(0), at(1)]));
                            let c = fl(geom::add(origin, [at(2), at(3)]));
                            let t = at(4);
                            let es = e.strs();
                            let number = es.get(1).cloned().unwrap_or_default();
                            let flags = e.flags();
                            let len = geom::dist(a, c);
                            let mid = [(a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0];
                            let mut p = PadIn::new(number.clone(), mid, [len + t, t]);
                            p.rotation = (c[1] - a[1]).atan2(c[0] - a[0]).to_degrees();
                            p.shape = if flags.contains("square") { PadShape::Rect } else if len < 1e-9 { PadShape::Circle } else { PadShape::Oval };
                            p.net = pin_net.get(&(refdes.clone(), number)).cloned();
                            part.pads.push(p);
                        }
                        "ElementLine" => part.silk.push(vec![fl(geom::add(origin, [at(0), at(1)])), fl(geom::add(origin, [at(2), at(3)]))]),
                        _ => {}
                    }
                }
                b.parts.push(part);
            }
            "Via" => {
                let en = n.nums();
                let at = |k: usize| en.get(k).map(|&j| n.len(j)).unwrap_or(0.0);
                b.vias.push(KVia { net: None, at: fl([at(0), at(1)]), drill: at(5), diameter: at(2) });
            }
            "Layer" => {
                let num = n.num(0) as u32;
                let lname = n.text(1).to_ascii_lowercase();
                let edge = lname == "outline" || lname == "route" || n.text(2).eq_ignore_ascii_case("outline");
                let copper = group_of.get(&num).cloned();
                if lname.contains("silk") {
                    continue;
                }
                for e in &n.body {
                    let en = e.nums();
                    let at = |k: usize| en.get(k).map(|&j| e.len(j)).unwrap_or(0.0);
                    match e.name.as_str() {
                        "Line" => {
                            let (a, c) = (fl([at(0), at(1)]), fl([at(2), at(3)]));
                            if edge {
                                b.edges.push(vec![a, c]);
                            } else if let Some(l) = &copper {
                                b.seg(None, l, at(4), a, c);
                            }
                        }
                        "Arc" => {
                            let (cx, cy, r) = (at(0), at(1), at(2));
                            let (sa, da) = (en.get(6).map(|&j| e.num(j)).unwrap_or(0.0), en.get(7).map(|&j| e.num(j)).unwrap_or(90.0));
                            // gEDA: 0° points to −x, angles grow towards +y (down on screen).
                            let n_ = ((da.abs() / 7.5).ceil() as usize).max(2);
                            let pts: Vec<P> = (0..=n_)
                                .map(|k| {
                                    let t = (sa + da * k as f64 / n_ as f64).to_radians();
                                    fl([cx - r * t.cos(), cy + r * t.sin()])
                                })
                                .collect();
                            if edge {
                                b.edges.push(pts);
                            } else if let Some(l) = &copper {
                                b.polyline(None, l, at(4), &pts);
                            }
                        }
                        "Polygon" => {
                            let Some(l) = &copper else { continue };
                            let pts: Vec<P> = e
                                .body
                                .iter()
                                .filter(|x| x.name.is_empty())
                                .map(|x| {
                                    let xn = x.nums();
                                    fl([xn.first().map(|&j| x.len(j)).unwrap_or(0.0), xn.get(1).map(|&j| x.len(j)).unwrap_or(0.0)])
                                })
                                .collect();
                            if pts.len() >= 3 {
                                b.zones.push(KZone { net: String::new(), layer: l.clone(), outline: pts, clearance: None, min_width: 0.25, priority: 0, connection: PadConnection::Thermal, thermal_gap: None, thermal_spoke: None });
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if b.edges.is_empty() && size[0] > 0.0 && size[1] > 0.0 {
        b.edges.push(vec![[0.0, 0.0], [size[0], 0.0], [size[0], -size[1]], [0.0, -size[1]], [0.0, 0.0]]);
        b.note("no outline layer: the board is the PCB[] size");
    }
    // gEDA copper carries no net: give each piece the net of a pad it touches.
    let pads: Vec<(P, f64, String)> = b.parts.iter().flat_map(|p| p.pads.iter().filter_map(|q| q.net.clone().map(|n| (q.at, q.size[0].min(q.size[1]) / 2.0, n)))).collect();
    let mut changed = true;
    let mut guard = 0;
    while changed && guard < 50 {
        changed = false;
        guard += 1;
        let known: Vec<(P, P, f64, String)> = b.segments.iter().filter_map(|s| s.net.clone().map(|n| (s.a, s.b, s.width / 2.0, n))).collect();
        for s in b.segments.iter_mut().filter(|s| s.net.is_none()) {
            for e in [s.a, s.b] {
                let hit = pads.iter().find(|(c, r, _)| geom::dist(*c, e) <= r + 1e-6).map(|x| x.2.clone()).or_else(|| known.iter().find(|(a, c, r, _)| geom::point_seg_dist(e, *a, *c) <= r + 1e-6).map(|x| x.3.clone()));
                if hit.is_some() {
                    s.net = hit;
                    changed = true;
                    break;
                }
            }
        }
    }
    let segs = b.segments.clone();
    for v in b.vias.iter_mut() {
        v.net = segs.iter().find(|s| s.net.is_some() && (geom::dist(s.a, v.at) < 1e-6 || geom::dist(s.b, v.at) < 1e-6)).and_then(|s| s.net.clone());
    }
    let unnamed = b.zones.iter().filter(|z| z.net.is_empty()).count();
    if unnamed > 0 {
        b.note(format!("{unnamed} polygons carry no net in gEDA and were left out"));
    }
    Ok(b)
}
