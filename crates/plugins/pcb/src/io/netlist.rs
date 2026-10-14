//! Netlists for other tools: KiCad's S-expression (`.net`, version "E") and
//! XML netlists, OrcadPCB2, PADS-PCB, Allegro (Telesis third-party) and
//! CADSTAR. From the schematic when the board has one (pins carry their
//! names and electrical types), else from the board's pads.

use crate::board::BoardView;
use crate::schematic_view::SchematicView;
use crate::sexpr::{self, Sexp, list, s};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Clone, Debug, Default)]
pub struct Comp {
    pub reference: String,
    pub value: String,
    pub footprint: String,
    pub symbol: String,
    pub description: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub reference: String,
    pub pad: String,
    pub function: String,
    pub etype: String,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub name: String,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, Default)]
pub struct Netlist {
    pub source: String,
    pub comps: Vec<Comp>,
    pub nets: Vec<Net>,
}

fn split_pad(s: &str) -> Option<(String, String)> {
    s.split_once('.').map(|(a, b)| (a.to_string(), b.to_string()))
}

pub fn from_schematic(v: &SchematicView) -> Netlist {
    let mut comps: BTreeMap<String, Comp> = BTreeMap::new();
    let mut pin_info: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
    for sym in &v.symbols {
        if sym.virtual_part || sym.exclude_from_board {
            continue;
        }
        let c = comps.entry(sym.reference.clone()).or_insert_with(|| Comp { reference: sym.reference.clone(), value: sym.value.clone(), footprint: sym.footprint.clone(), symbol: sym.symbol.clone(), description: sym.description.clone(), fields: sym.fields.clone() });
        if let Some(m) = &sym.mpn {
            c.fields.insert("MPN".into(), m.clone());
        }
        if let Some(m) = &sym.manufacturer {
            c.fields.insert("Manufacturer".into(), m.clone());
        }
        if let Some(m) = &sym.lcsc {
            c.fields.insert("LCSC".into(), m.clone());
        }
        for p in &sym.pins {
            let et = serde_json::to_value(p.etype).ok().and_then(|x| x.as_str().map(str::to_string)).unwrap_or_else(|| "passive".into());
            for pad in &p.pads {
                pin_info.insert((sym.reference.clone(), pad.clone()), (if p.name == "~" { String::new() } else { p.name.clone() }, et.clone()));
            }
        }
    }
    let mut nets = Vec::new();
    for n in &v.nets {
        let mut nodes: Vec<Node> = n
            .pads
            .iter()
            .filter_map(|x| split_pad(x))
            .filter(|(r, _)| comps.contains_key(r))
            .map(|(r, p)| {
                let (function, etype) = pin_info.get(&(r.clone(), p.clone())).cloned().unwrap_or_default();
                Node { reference: r, pad: p, function, etype: if etype.is_empty() { "passive".into() } else { etype } }
            })
            .collect();
        nodes.sort_by(|a, b| crate::natural(&a.reference, &b.reference).then(crate::natural(&a.pad, &b.pad)));
        nodes.dedup_by(|a, b| a.reference == b.reference && a.pad == b.pad);
        if !nodes.is_empty() {
            nets.push(Net { name: n.name.clone(), nodes });
        }
    }
    let mut comps: Vec<Comp> = comps.into_values().collect();
    comps.sort_by(|a, b| crate::natural(&a.reference, &b.reference));
    nets.sort_by(|a, b| a.name.cmp(&b.name));
    Netlist { source: format!("{}.kicad_sch", crate::gerber::file_stem(&v.name)), comps, nets }
}

pub fn from_board(v: &BoardView) -> Netlist {
    let comps: Vec<Comp> = v
        .components
        .iter()
        .map(|c| {
            let mut fields = BTreeMap::new();
            for (k, x) in [("MPN", &c.mpn), ("Manufacturer", &c.manufacturer), ("LCSC", &c.lcsc)] {
                if let Some(x) = x {
                    fields.insert(k.to_string(), x.clone());
                }
            }
            Comp { reference: c.reference.clone(), value: c.value.clone(), footprint: c.footprint.clone(), symbol: String::new(), description: c.description.clone(), fields }
        })
        .collect();
    let mut by_net: BTreeMap<String, Vec<Node>> = BTreeMap::new();
    for p in &v.pads {
        if let Some(n) = &p.net {
            let n2 = Node { reference: p.reference.clone(), pad: p.number.clone(), function: p.pin_name.clone().unwrap_or_default(), etype: "passive".into() };
            let list = by_net.entry(n.clone()).or_default();
            if !list.iter().any(|x| x.reference == n2.reference && x.pad == n2.pad) {
                list.push(n2);
            }
        }
    }
    let nets = by_net
        .into_iter()
        .map(|(name, mut nodes)| {
            nodes.sort_by(|a, b| crate::natural(&a.reference, &b.reference).then(crate::natural(&a.pad, &b.pad)));
            Net { name, nodes }
        })
        .collect();
    Netlist { source: format!("{}.kicad_pcb", crate::gerber::file_stem(&v.name)), comps, nets }
}

fn today() -> String {
    super::today()
}

/// KiCad's S-expression netlist (version "E").
pub fn kicad(n: &Netlist, title: &str) -> String {
    let comps: Vec<Sexp> = n
        .comps
        .iter()
        .map(|c| {
            let (lib, part) = c.symbol.split_once(':').unwrap_or(("ise", &c.symbol));
            let mut v = vec![list("ref", vec![s(c.reference.clone())]), list("value", vec![s(c.value.clone())])];
            if !c.footprint.is_empty() {
                v.push(list("footprint", vec![s(c.footprint.clone())]));
            }
            v.push(list("libsource", vec![list("lib", vec![s(lib)]), list("part", vec![s(part)]), list("description", vec![s(c.description.clone())])]));
            for (k, x) in &c.fields {
                v.push(list("property", vec![list("name", vec![s(k.clone())]), list("value", vec![s(x.clone())])]));
            }
            v.push(list("sheetpath", vec![list("names", vec![s("/")]), list("tstamps", vec![s("/")])]));
            v.push(list("tstamps", vec![s(crate::kicad::uuid(&format!("{title}/{}", c.reference)))]));
            list("comp", v)
        })
        .collect();
    let nets: Vec<Sexp> = n
        .nets
        .iter()
        .enumerate()
        .map(|(i, net)| {
            let mut v = vec![list("code", vec![s((i + 1).to_string())]), list("name", vec![s(net.name.clone())])];
            for nd in &net.nodes {
                let mut x = vec![list("ref", vec![s(nd.reference.clone())]), list("pin", vec![s(nd.pad.clone())])];
                if !nd.function.is_empty() {
                    x.push(list("pinfunction", vec![s(nd.function.clone())]));
                }
                x.push(list("pintype", vec![s(nd.etype.clone())]));
                v.push(list("node", x));
            }
            list("net", v)
        })
        .collect();
    let e = list(
        "export",
        vec![
            list("version", vec![s("E")]),
            list("design", vec![list("source", vec![s(n.source.clone())]), list("date", vec![s(today())]), list("tool", vec![s("ISE Studio")])]),
            list("components", comps),
            list("nets", nets),
        ],
    );
    sexpr::write(&e)
}

/// KiCad's XML netlist.
pub fn kicad_xml(n: &Netlist, _title: &str) -> String {
    let mut w = super::xml::Writer::new();
    w.open("export", &[("version", "E".into())]);
    w.open("design", &[]);
    w.text("source", &[], &n.source);
    w.text("date", &[], &today());
    w.text("tool", &[], "ISE Studio");
    w.close();
    w.open("components", &[]);
    for c in &n.comps {
        let (lib, part) = c.symbol.split_once(':').unwrap_or(("ise", &c.symbol));
        w.open("comp", &[("ref", c.reference.clone())]);
        w.text("value", &[], &c.value);
        if !c.footprint.is_empty() {
            w.text("footprint", &[], &c.footprint);
        }
        w.empty("libsource", &[("lib", lib.to_string()), ("part", part.to_string()), ("description", c.description.clone())]);
        for (k, x) in &c.fields {
            w.empty("property", &[("name", k.clone()), ("value", x.clone())]);
        }
        w.close();
    }
    w.close();
    w.open("nets", &[]);
    for (i, net) in n.nets.iter().enumerate() {
        w.open("net", &[("code", (i + 1).to_string()), ("name", net.name.clone())]);
        for nd in &net.nodes {
            w.empty("node", &[("ref", nd.reference.clone()), ("pin", nd.pad.clone()), ("pinfunction", nd.function.clone()), ("pintype", nd.etype.clone())]);
        }
        w.close();
    }
    w.finish()
}

/// OrcadPCB2: each part with its pins' nets.
pub fn orcad(n: &Netlist) -> String {
    let mut pad_net: BTreeMap<(String, String), String> = BTreeMap::new();
    for net in &n.nets {
        for nd in &net.nodes {
            pad_net.insert((nd.reference.clone(), nd.pad.clone()), net.name.clone());
        }
    }
    let mut o = String::new();
    let _ = writeln!(o, "( {{ EESchema Netlist Version 1.1 created by ISE Studio {} }}", today());
    for c in &n.comps {
        let fp = if c.footprint.is_empty() { "$noname".to_string() } else { c.footprint.replace(' ', "_") };
        let val = if c.value.is_empty() { "~".to_string() } else { c.value.replace(' ', "_") };
        let _ = writeln!(o, " ( {} {fp} {} {val}", crate::kicad::uuid(&c.reference)[..8].to_uppercase(), c.reference);
        let mut pins: Vec<(&String, &String)> = pad_net.iter().filter(|((r, _), _)| *r == c.reference).map(|((_, p), net)| (p, net)).collect();
        pins.sort_by(|a, b| crate::natural(a.0, b.0));
        for (p, net) in pins {
            let _ = writeln!(o, "  ( {p:>4} {} )", net.replace(' ', "_"));
        }
        let _ = writeln!(o, " )");
    }
    let _ = writeln!(o, ")\n*");
    o
}

/// PADS-PCB netlist.
pub fn pads(n: &Netlist) -> String {
    let clean = |s: &str| s.replace(' ', "_");
    let mut o = String::from("*PADS-PCB*\n*PART*\n");
    for c in &n.comps {
        let _ = writeln!(o, "{} {}", clean(&c.reference), if c.footprint.is_empty() { "unknown".into() } else { clean(&c.footprint) });
    }
    o.push_str("\n*NET*\n");
    for net in &n.nets {
        let _ = writeln!(o, "*SIGNAL* {}", clean(&net.name));
        let nodes: Vec<String> = net.nodes.iter().map(|x| format!("{}.{}", clean(&x.reference), clean(&x.pad))).collect();
        for chunk in nodes.chunks(8) {
            let _ = writeln!(o, " {}", chunk.join(" "));
        }
    }
    o.push_str("*END*\n");
    o
}

/// Allegro third-party (Telesis) netlist.
pub fn allegro(n: &Netlist) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "_"));
    let mut o = String::from("(Source: ISE Studio)\n$PACKAGES\n");
    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for c in &n.comps {
        groups.entry((c.footprint.clone(), c.value.clone())).or_default().push(c.reference.clone());
    }
    for ((fp, val), refs) in groups {
        let _ = writeln!(o, "{} ! {} ; {}", q(if fp.is_empty() { "unknown" } else { &fp }), q(&val), refs.join(" "));
    }
    o.push_str("$NETS\n");
    for net in &n.nets {
        let nodes: Vec<String> = net.nodes.iter().map(|x| format!("{}.{}", x.reference, x.pad)).collect();
        let _ = writeln!(o, "{} ; {}", q(&net.name), nodes.join(" "));
    }
    o.push_str("$END\n");
    o
}

/// CADSTAR netlist (.frp).
pub fn cadstar(n: &Netlist, title: &str) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', "'"));
    let mut o = String::from(".HEA\n");
    let _ = writeln!(o, ".TIM {}", today().replace('-', " "));
    let _ = writeln!(o, ".APP \"ISE Studio\"\n.TYP FULL\n.JOB {}\n", q(title));
    for c in &n.comps {
        let _ = writeln!(o, ".ADD_COM {} {} {}", c.reference, q(&c.footprint), q(&c.value));
    }
    o.push('\n');
    for net in &n.nets {
        for (i, nd) in net.nodes.iter().enumerate() {
            if i == 0 {
                let _ = writeln!(o, ".ADD_TER {}.{} {}", nd.reference, nd.pad, q(&net.name));
            } else {
                let _ = writeln!(o, ".TER     {}.{}", nd.reference, nd.pad);
            }
        }
    }
    o.push_str("\n.END\n");
    o
}
