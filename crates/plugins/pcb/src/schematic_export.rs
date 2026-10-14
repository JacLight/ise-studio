//! Files out of a schematic: netlists for other tools (KiCad s-expression,
//! OrCAD PCB2, CADSTAR, SPICE), a bill of materials grouped and with the
//! fields asked for, and each sheet as SVG drawn from the same primitives as
//! the editor and the PDF.

use crate::natural;
use crate::schematic::is_ground;
use crate::schematic_pdf::ink;
use crate::schematic_view::{Draw, SchematicView, SymbolView, W_WIRE};
use std::collections::BTreeMap;
use std::fmt::Write;

/// A part with every unit's pins merged.
struct Part<'a> {
    first: &'a SymbolView,
    /// (pad, pin number, pin name, pin type, net)
    pads: Vec<(String, String, String, String, Option<String>)>,
}

fn parts(v: &SchematicView) -> Vec<Part<'_>> {
    let mut by: BTreeMap<String, Part> = BTreeMap::new();
    for s in &v.symbols {
        if s.virtual_part || s.exclude_from_board || s.reference.starts_with('#') {
            continue;
        }
        let p = by.entry(s.reference.clone()).or_insert(Part { first: s, pads: vec![] });
        for pin in &s.pins {
            for pad in &pin.pads {
                if !p.pads.iter().any(|x| &x.0 == pad) {
                    let t = serde_json::to_value(pin.etype).ok().and_then(|x| x.as_str().map(|s| s.to_string())).unwrap_or_else(|| "passive".into());
                    p.pads.push((pad.clone(), pin.number.clone(), pin.name.clone(), t, pin.net.clone()));
                }
            }
        }
    }
    let mut list: Vec<Part> = by.into_values().collect();
    list.sort_by(|a, b| natural(&a.first.reference, &b.first.reference));
    for p in &mut list {
        p.pads.sort_by(|a, b| natural(&a.0, &b.0));
    }
    list
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let t = secs.rem_euclid(86_400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}/{m:02}/{d:02} {:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

/// Nets with their nodes (reference, pad, pin name, pin type), unconnected pads as their own nets.
fn net_nodes(list: &[Part]) -> Vec<(String, Vec<(String, String, String, String)>)> {
    let mut nets: BTreeMap<String, Vec<(String, String, String, String)>> = BTreeMap::new();
    for p in list {
        for (pad, _, name, t, net) in &p.pads {
            let n = net.clone().unwrap_or_else(|| format!("unconnected-({}-Pad{pad})", p.first.reference));
            nets.entry(n).or_default().push((p.first.reference.clone(), pad.clone(), name.clone(), t.clone()));
        }
    }
    let mut out: Vec<_> = nets.into_iter().collect();
    out.sort_by(|a, b| natural(&a.0, &b.0));
    out
}

pub const FORMATS: [&str; 4] = ["kicad", "orcad", "cadstar", "spice"];

/// A netlist in `format`: kicad (.net), orcad (OrCAD PCB2 .net), cadstar (.frp) or spice (.cir).
pub fn netlist(v: &SchematicView, format: &str) -> Result<String, String> {
    match format.to_ascii_lowercase().as_str() {
        "kicad" | "kicad_net" | "net" => Ok(kicad(v)),
        "orcad" | "orcadpcb2" | "orcad_pcb2" => Ok(orcad(v)),
        "cadstar" => Ok(cadstar(v)),
        "spice" | "cir" => Ok(spice(v)),
        other => Err(format!("netlist formats: {} (not {other})", FORMATS.join(", "))),
    }
}

/// File name extension for a netlist format.
pub fn extension(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "cadstar" => "frp",
        "spice" | "cir" => "cir",
        _ => "net",
    }
}

fn kicad(v: &SchematicView) -> String {
    let list = parts(v);
    let mut o = String::new();
    let _ = writeln!(o, "(export (version \"E\")");
    let _ = writeln!(o, "  (design\n    (source {})\n    (date {})\n    (tool \"ISE Studio\")", q(&v.name), q(&stamp()));
    for sh in &v.sheets {
        let path = if sh.parent.is_some() { format!("/{}/", sh.name) } else { "/".to_string() };
        let _ = writeln!(o, "    (sheet (number {}) (name {}) (tstamps {})", q(&sh.page), q(&path), q(&path));
        let _ = writeln!(o, "      (title_block\n        (title {})\n        (company {})\n        (rev {})\n        (date {})\n        (source {})))", q(&v.title.title), q(&v.title.company), q(&v.title.revision), q(&v.title.date), q(&v.name));
    }
    let _ = writeln!(o, "  )\n  (components");
    for p in &list {
        let s = p.first;
        let _ = writeln!(o, "    (comp (ref {})\n      (value {})", q(&s.reference), q(&s.value));
        if !s.footprint.is_empty() {
            let _ = writeln!(o, "      (footprint {})", q(&s.footprint));
        }
        let mut fields: Vec<(String, String)> = Vec::new();
        for (k, val) in [("MPN", &s.mpn), ("Manufacturer", &s.manufacturer), ("LCSC", &s.lcsc)] {
            if let Some(x) = val {
                fields.push((k.into(), x.clone()));
            }
        }
        fields.extend(s.fields.iter().map(|(k, x)| (k.clone(), x.clone())));
        if !fields.is_empty() {
            let _ = writeln!(o, "      (fields");
            for (k, x) in fields {
                let _ = writeln!(o, "        (field (name {}) {})", q(&k), q(&x));
            }
            let _ = writeln!(o, "      )");
        }
        let _ = writeln!(o, "      (libsource (lib \"ise\") (part {}) (description {}))", q(&s.symbol), q(&s.description));
        if s.dnp {
            let _ = writeln!(o, "      (property (name \"dnp\"))");
        }
        let sheet = v.sheets.iter().find(|x| x.id == s.sheet);
        let path = sheet.filter(|x| x.parent.is_some()).map(|x| format!("/{}/", x.name)).unwrap_or_else(|| "/".into());
        let _ = writeln!(o, "      (sheetpath (names {}) (tstamps {}))\n      (tstamps {}))", q(&path), q(&path), q(&s.id.to_string()));
    }
    let _ = writeln!(o, "  )\n  (nets");
    for (code, (name, nodes)) in net_nodes(&list).iter().enumerate() {
        let _ = writeln!(o, "    (net (code {}) (name {})", q(&(code + 1).to_string()), q(name));
        for (r, pad, func, t) in nodes {
            let _ = writeln!(o, "      (node (ref {}) (pin {}) (pinfunction {}) (pintype {}))", q(r), q(pad), q(func), q(t));
        }
        let _ = writeln!(o, "    )");
    }
    let _ = writeln!(o, "  )\n)");
    o
}

fn plain(n: &str) -> String {
    n.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect()
}

fn orcad(v: &SchematicView) -> String {
    let list = parts(v);
    let mut o = String::new();
    let _ = writeln!(o, "( {{ ISE Studio netlist: {} {} }}", v.name, stamp());
    for (i, p) in list.iter().enumerate() {
        let s = p.first;
        let fp = if s.footprint.is_empty() { "$noname".to_string() } else { plain(&s.footprint) };
        let _ = writeln!(o, " ( {:08X} {} {} {}", i + 1, fp, s.reference, if s.value.is_empty() { "~".to_string() } else { plain(&s.value) });
        for (pad, _, _, _, net) in &p.pads {
            let _ = writeln!(o, "  ( {:>4} {} )", pad, net.as_deref().map(plain).unwrap_or_else(|| "?".into()));
        }
        let _ = writeln!(o, " )");
    }
    let _ = writeln!(o, ")\n*");
    o
}

fn cadstar(v: &SchematicView) -> String {
    let list = parts(v);
    let mut o = String::new();
    let _ = writeln!(o, ".HEA\n.TIM {}\n.APP \"ISE Studio\"\n\n.TYP FULL\n", stamp());
    for p in &list {
        let s = p.first;
        let _ = writeln!(o, ".ADD_COM {} {}", s.reference, q(if s.footprint.is_empty() { &s.value } else { &s.footprint }));
    }
    let _ = writeln!(o);
    for (name, nodes) in net_nodes(&list).iter().filter(|(n, _)| !n.starts_with("unconnected-")) {
        for (k, (r, pad, _, _)) in nodes.iter().enumerate() {
            if k == 0 {
                let _ = writeln!(o, ".ADD_TER {} {} {}", r, pad, q(name));
            } else {
                let _ = writeln!(o, ".TER     {} {}", r, pad);
            }
        }
    }
    let _ = writeln!(o, "\n.END");
    o
}

fn spice_node(n: &Option<String>, k: &mut usize) -> String {
    match n {
        Some(n) if is_ground(n) => "0".into(),
        Some(n) => n.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect(),
        None => {
            *k += 1;
            format!("NC_{}", k)
        }
    }
}

fn spice(v: &SchematicView) -> String {
    let list = parts(v);
    let mut o = String::new();
    let _ = writeln!(o, "* {} — netlist from ISE Studio ({})", v.name, stamp());
    let mut nc = 0;
    for p in &list {
        let s = p.first;
        if s.dnp {
            let _ = writeln!(o, "* {} not populated", s.reference);
            continue;
        }
        // Pins by number; a pad per pin (the first).
        let mut pins: Vec<&(String, String, String, String, Option<String>)> = Vec::new();
        for x in &p.pads {
            if !pins.iter().any(|y| y.1 == x.1) {
                pins.push(x);
            }
        }
        pins.sort_by(|a, b| natural(&a.1, &b.1));
        let by_name = |names: &[&str]| -> Option<Vec<&Option<String>>> { names.iter().map(|n| pins.iter().find(|x| x.2.eq_ignore_ascii_case(n)).map(|x| &x.4)).collect() };
        let letter = s.reference.chars().next().unwrap_or('X').to_ascii_uppercase();
        let (prefix, nodes, model): (String, Vec<&Option<String>>, String) = match letter {
            'R' | 'C' | 'L' if pins.len() == 2 => (String::new(), pins.iter().map(|x| &x.4).collect(), s.value.clone()),
            'D' if pins.len() == 2 => {
                let n = by_name(&["A", "K"]).unwrap_or_else(|| if s.symbol.starts_with("LED") { vec![&pins[1].4, &pins[0].4] } else { vec![&pins[1].4, &pins[0].4] });
                (String::new(), n, if s.symbol.starts_with("LED") { "LED".into() } else { "D".into() })
            }
            'Q' if pins.len() == 3 => {
                if let Some(n) = by_name(&["D", "G", "S"]) {
                    let mut n = n;
                    n.push(n[2]);
                    ("M".into(), n, if s.symbol.contains("PMOS") { "PMOS".into() } else { "NMOS".into() })
                } else {
                    let n = by_name(&["C", "B", "E"]).unwrap_or_else(|| pins.iter().map(|x| &x.4).collect());
                    (String::new(), n, if s.symbol.contains("PNP") { "PNP".into() } else { "NPN".into() })
                }
            }
            _ => ("X".into(), pins.iter().map(|x| &x.4).collect(), s.symbol.replace([':', ' ', '-'], "_")),
        };
        let name = if prefix.is_empty() { s.reference.clone() } else { format!("{prefix}{}", s.reference) };
        let ns: Vec<String> = nodes.into_iter().map(|n| spice_node(n, &mut nc)).collect();
        let _ = writeln!(o, "{} {} {}", name, ns.join(" "), if model.is_empty() { "?".into() } else { model });
    }
    let _ = writeln!(o, ".end");
    o
}

// ---- Bill of materials ---------------------------------------------------------------

pub const BOM_FIELDS: [&str; 9] = ["Reference", "Qty", "Value", "Footprint", "MPN", "Manufacturer", "LCSC", "Description", "DNP"];

fn field_of(s: &SymbolView, f: &str) -> String {
    match f.to_ascii_lowercase().as_str() {
        "reference" | "ref" => s.reference.clone(),
        "value" => s.value.clone(),
        "footprint" => s.footprint.clone(),
        "mpn" => s.mpn.clone().unwrap_or_default(),
        "manufacturer" | "maker" => s.manufacturer.clone().unwrap_or_default(),
        "lcsc" => s.lcsc.clone().unwrap_or_default(),
        "description" => s.description.clone(),
        "dnp" => if s.dnp { "DNP".into() } else { String::new() },
        "symbol" => s.symbol.clone(),
        _ => s.fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(f)).map(|(_, v)| v.clone()).unwrap_or_default(),
    }
}

/// BOM rows: parts grouped by the values of `group` (empty: one row per part),
/// each with the `fields` asked for (Reference lists the group's references, Qty counts them).
pub fn bom_rows(v: &SchematicView, group: &[String], fields: &[String], include_dnp: bool) -> Vec<Vec<String>> {
    let list = parts(v);
    let mut groups: Vec<(Vec<String>, Vec<&SymbolView>)> = Vec::new();
    for p in &list {
        let s = p.first;
        if s.dnp && !include_dnp {
            continue;
        }
        let key: Vec<String> = if group.is_empty() { vec![s.reference.clone()] } else { group.iter().map(|g| field_of(s, g)).collect() };
        let key = if s.dnp { key.into_iter().chain(["DNP".to_string()]).collect() } else { key };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, l)) => l.push(s),
            None => groups.push((key, vec![s])),
        }
    }
    groups.sort_by(|a, b| natural(&a.1[0].reference, &b.1[0].reference));
    groups
        .into_iter()
        .map(|(_, members)| {
            fields
                .iter()
                .map(|f| match f.to_ascii_lowercase().as_str() {
                    "reference" | "ref" | "references" => members.iter().map(|m| m.reference.clone()).collect::<Vec<_>>().join(", "),
                    "qty" | "quantity" => members.len().to_string(),
                    _ => {
                        let mut vals: Vec<String> = members.iter().map(|m| field_of(m, f)).collect();
                        vals.dedup();
                        vals.join(", ")
                    }
                })
                .collect()
        })
        .collect()
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

pub fn bom_csv(v: &SchematicView, group: &[String], fields: &[String], include_dnp: bool) -> String {
    let mut o = fields.iter().map(|f| csv_cell(f)).collect::<Vec<_>>().join(",");
    o.push('\n');
    for row in bom_rows(v, group, fields, include_dnp) {
        o.push_str(&row.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        o.push('\n');
    }
    o
}

// ---- SVG plot ---------------------------------------------------------------------------

fn hex(name: &str) -> String {
    let c = ink(name);
    format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn pts(p: &[[f64; 2]]) -> String {
    p.iter().map(|q| format!("{:.3},{:.3}", q[0], q[1])).collect::<Vec<_>>().join(" ")
}

pub fn dash_array(d: &str, w: f64) -> Option<String> {
    let w = w.max(0.15);
    match d {
        "dash" => Some(format!("{:.2} {:.2}", w * 6.0, w * 3.0)),
        "dot" => Some(format!("{:.2} {:.2}", w, w * 2.5)),
        "dash_dot" => Some(format!("{:.2} {:.2} {:.2} {:.2}", w * 6.0, w * 2.5, w, w * 2.5)),
        _ => None,
    }
}

fn svg_draw(o: &mut String, d: &Draw) {
    match d {
        Draw::Line { p, w, ink } => {
            let _ = writeln!(o, "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{w:.3}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>", pts(p), hex(ink));
        }
        Draw::Poly { p, w, ink, fill } => {
            let f = match *fill {
                "ink" => hex(ink),
                "body" => hex("body_fill"),
                _ => "none".into(),
            };
            let _ = writeln!(o, "<polygon points=\"{}\" fill=\"{f}\" stroke=\"{}\" stroke-width=\"{w:.3}\" stroke-linejoin=\"round\"/>", pts(p), hex(ink));
        }
        Draw::Circle { c, r, w, ink, fill } => {
            let f = match *fill {
                "ink" => hex(ink),
                "body" => hex("body_fill"),
                _ => "none".into(),
            };
            let _ = writeln!(o, "<circle cx=\"{:.3}\" cy=\"{:.3}\" r=\"{r:.3}\" fill=\"{f}\" stroke=\"{}\" stroke-width=\"{w:.3}\"/>", c[0], c[1], hex(ink));
        }
        Draw::Text { at, s, size, angle, anchor, ink } => {
            if s.is_empty() {
                return;
            }
            let rot = if *angle != 0.0 { format!(" transform=\"rotate(-90 {:.3} {:.3})\"", at[0], at[1]) } else { String::new() };
            let weight = if *ink == "reference" || *ink == "frame_text" { " font-weight=\"600\"" } else { "" };
            let _ = writeln!(o, "<text x=\"{:.3}\" y=\"{:.3}\" dy=\"{:.3}\" font-size=\"{size:.3}\" text-anchor=\"{anchor}\" fill=\"{}\"{weight}{rot}>{}</text>", at[0], at[1], size * 0.35, hex(ink), esc(s));
        }
        Draw::Rich { at, s, size, angle, anchor, color, bold, italic } => {
            if s.is_empty() {
                return;
            }
            let rot = if *angle != 0.0 { format!(" transform=\"rotate(-90 {:.3} {:.3})\"", at[0], at[1]) } else { String::new() };
            let _ = writeln!(o, "<text x=\"{:.3}\" y=\"{:.3}\" dy=\"{:.3}\" font-size=\"{size:.3}\" text-anchor=\"{anchor}\" fill=\"{}\"{}{}{rot}>{}</text>", at[0], at[1], size * 0.35, esc(color), if *bold { " font-weight=\"700\"" } else { "" }, if *italic { " font-style=\"italic\"" } else { "" }, esc(s));
        }
        Draw::Styled { p, w, color, fill, dash, closed } => {
            let tag = if *closed { "polygon" } else { "polyline" };
            let da = dash_array(dash, *w).map(|d| format!(" stroke-dasharray=\"{d}\"")).unwrap_or_default();
            let stroke = if *w > 0.0 { esc(color) } else { "none".into() };
            let _ = writeln!(o, "<{tag} points=\"{}\" fill=\"{}\" stroke=\"{stroke}\" stroke-width=\"{w:.3}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{da}/>", pts(p), fill.as_deref().map(esc).unwrap_or_else(|| "none".into()));
        }
        Draw::Image { at, size, mime, data } => {
            let _ = writeln!(o, "<image x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" preserveAspectRatio=\"none\" href=\"data:{mime};base64,{data}\"/>", at[0], at[1], size[0], size[1]);
        }
    }
}

/// Everything drawn on a sheet, in paint order (shared by the SVG and PDF writers).
pub fn sheet_draws(v: &SchematicView, sheet: &str) -> Vec<Draw> {
    let mut out: Vec<Draw> = Vec::new();
    let Some(sh) = v.sheets.iter().find(|s| s.id == sheet) else { return out };
    out.extend(sh.frame.iter().cloned());
    out.extend(sh.boxes_draw.iter().cloned());
    for g in v.graphics.iter().filter(|g| g.sheet == sheet) {
        out.extend(g.draw.iter().cloned());
    }
    for t in v.texts.iter().filter(|t| t.sheet == sheet) {
        let anchor = match t.justify.as_str() {
            "center" => "middle",
            "right" => "end",
            _ => "start",
        };
        for (i, line) in t.text.lines().enumerate() {
            let off = i as f64 * t.size * 1.35;
            let at = if t.angle != 0.0 { [t.at[0] + off, t.at[1]] } else { [t.at[0], t.at[1] + off] };
            out.push(Draw::Rich { at, s: line.to_string(), size: t.size, angle: t.angle, anchor, color: t.color.clone().unwrap_or_else(|| hex("note")), bold: t.bold, italic: t.italic });
        }
    }
    for s in v.symbols.iter().filter(|s| s.sheet == sheet) {
        out.extend(s.draw.iter().cloned());
    }
    for b in v.buses.iter().filter(|b| b.sheet == sheet) {
        out.push(Draw::Line { p: b.points.clone(), w: 0.5, ink: "bus" });
    }
    for e in v.bus_entries.iter().filter(|b| b.sheet == sheet) {
        out.push(Draw::Line { p: vec![e.at, e.end], w: W_WIRE * 1.2, ink: "bus" });
    }
    for w in v.wires.iter().filter(|x| x.sheet == sheet) {
        match &w.color {
            Some(c) => out.push(Draw::Styled { p: w.points.clone(), w: w.width.unwrap_or(W_WIRE), color: c.clone(), fill: None, dash: "", closed: false }),
            None => out.push(Draw::Line { p: w.points.clone(), w: w.width.unwrap_or(W_WIRE), ink: "wire" }),
        }
    }
    for j in v.junctions.iter().filter(|x| x.sheet == sheet) {
        out.push(Draw::Circle { c: j.at, r: 0.45, w: 0.05, ink: "junction", fill: "ink" });
    }
    for n in v.no_connects.iter().filter(|x| x.sheet == sheet) {
        let a = 0.7;
        out.push(Draw::Line { p: vec![[n.at[0] - a, n.at[1] - a], [n.at[0] + a, n.at[1] + a]], w: 0.2, ink: "no_connect" });
        out.push(Draw::Line { p: vec![[n.at[0] - a, n.at[1] + a], [n.at[0] + a, n.at[1] - a]], w: 0.2, ink: "no_connect" });
    }
    for l in v.labels.iter().filter(|x| x.sheet == sheet) {
        out.extend(l.draw.iter().cloned());
    }
    out
}

/// One sheet as an SVG file (millimetres).
pub fn svg(v: &SchematicView, sheet: &str) -> Option<String> {
    let sh = v.sheets.iter().find(|s| s.id == sheet)?;
    let [w, h] = sh.size;
    let mut o = String::new();
    let _ = writeln!(o, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(o, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\" font-family=\"Helvetica, Arial, sans-serif\">");
    let _ = writeln!(o, "<title>{} — {}</title>", esc(&v.name), esc(&sh.name));
    let _ = writeln!(o, "<rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>");
    for d in sheet_draws(v, sheet) {
        svg_draw(&mut o, &d);
    }
    let _ = writeln!(o, "</svg>");
    Some(o)
}
