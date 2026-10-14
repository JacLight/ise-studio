//! IPC-D-356A netlist for bare-board electrical test: every test point (each
//! pad, through-hole and surface-mount, and every via) with its net, place,
//! size, drill and which side a probe reaches it from, in the standard's
//! fixed 80-column records, metric (`UNITS CUST 1`: 0.001 mm and degrees).
//! Net names longer than 14 characters get an alias (`NNAME1`) declared in a
//! parameter record, as the standard allows.

use crate::board::BoardView;
use crate::footprints::PadKind;
use crate::Side;
use std::collections::BTreeMap;
use std::fmt::Write;

fn coord(v: f64) -> String {
    let k = (v * 1000.0).round() as i64;
    format!("{}{:06}", if k < 0 { '-' } else { '+' }, k.unsigned_abs().min(999_999))
}

fn size4(v: f64) -> String {
    format!("{:04}", ((v * 1000.0).round() as i64).clamp(0, 9999))
}

fn field(s: &str, w: usize) -> String {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).take(w).collect();
    format!("{t:<w$}")
}

/// One test record.
#[allow(clippy::too_many_arguments)]
fn record(op: &str, net: &str, refdes: &str, pin: &str, drill: Option<(f64, bool)>, access: u8, at: [f64; 2], size: [f64; 2], rot: f64, mask: u8) -> String {
    let drill = match drill {
        Some((d, plated)) => format!("D{}{}", size4(d), if plated { 'P' } else { 'U' }),
        None => "      ".to_string(),
    };
    let rot = (rot.rem_euclid(360.0).round() as i64) % 360;
    format!("{op}{}   {}-{} {drill}A{access:02}X{}Y{}X{}Y{}R{rot:03} S{mask}", field(net, 14), field(refdes, 6), field(pin, 4), coord(at[0]), coord(at[1]), size4(size[0]), size4(size[1]))
}

pub fn write(v: &BoardView) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "C  IPC-D-356A netlist written by ISE Studio");
    let _ = writeln!(o, "C  {} rev {}", v.name, v.revision);
    let _ = writeln!(o, "P  JOB   {}", v.name);
    let _ = writeln!(o, "P  UNITS CUST 1");
    let _ = writeln!(o, "P  DIM   N");
    // Aliases for long names.
    let mut alias: BTreeMap<String, String> = BTreeMap::new();
    let mut names: Vec<&String> = v.pads.iter().filter_map(|p| p.net.as_ref()).chain(v.vias.iter().filter_map(|x| x.net.as_ref())).collect();
    names.sort();
    names.dedup();
    for n in names {
        if n.chars().count() > 14 || n.contains(char::is_whitespace) {
            let a = format!("NNAME{}", alias.len() + 1);
            let _ = writeln!(o, "P  {a:<8}{n}");
            alias.insert(n.clone(), a);
        }
    }
    let name_of = |n: &Option<String>| match n {
        Some(n) => alias.get(n).cloned().unwrap_or_else(|| n.clone()),
        None => "N/C".to_string(),
    };
    let o0 = [v.bounds[0], v.bounds[1]];
    let rel = |p: [f64; 2]| [p[0] - o0[0], p[1] - o0[1]];
    for p in &v.pads {
        let bb = p.prim.bbox();
        let size = [bb[2] - bb[0], bb[3] - bb[1]];
        match p.kind {
            PadKind::Tht => {
                let d = p.drill.map(|d| d[0].min(d[1])).unwrap_or(0.0);
                let _ = writeln!(o, "{}", record("317", &name_of(&p.net), &p.reference, &p.number, Some((d, true)), 0, rel(p.at), size, 0.0, 3));
            }
            PadKind::Npth => {
                let d = p.drill.map(|d| d[0].min(d[1])).unwrap_or(0.0);
                let _ = writeln!(o, "{}", record("317", "N/C", &p.reference, &p.number, Some((d, false)), 0, rel(p.at), [d, d], 0.0, 3));
            }
            PadKind::Smd => {
                let top = p.side == Side::Top;
                let _ = writeln!(o, "{}", record("327", &name_of(&p.net), &p.reference, &p.number, None, if top { 1 } else { 2 }, rel(p.at), size, 0.0, if top { 1 } else { 2 }));
            }
        }
    }
    for x in &v.vias {
        let _ = writeln!(o, "{}", record("317", &name_of(&x.net), "VIA", "", Some((x.drill, true)), 0, rel(x.at), [x.diameter, x.diameter], 0.0, 0));
    }
    let _ = writeln!(o, "999");
    o
}

/// Test points read back from a file: (op code, net, refdes, pin, x, y) in mm; aliases resolved.
pub fn read(text: &str) -> Vec<(String, String, String, String, f64, f64)> {
    let mut alias: BTreeMap<String, String> = BTreeMap::new();
    let mut out = Vec::new();
    for l in text.lines() {
        if let Some(rest) = l.strip_prefix("P  NNAME") {
            let (k, name) = rest.split_at(rest.find(' ').unwrap_or(rest.len()));
            alias.insert(format!("NNAME{k}"), name.trim().to_string());
            continue;
        }
        if !(l.starts_with("317") || l.starts_with("327")) || l.len() < 58 {
            continue;
        }
        let net = l[3..17].trim().to_string();
        let net = alias.get(&net).cloned().unwrap_or(net);
        let refdes = l[20..26].trim().to_string();
        let pin = l[27..31].trim().to_string();
        let num = |s: &str| s.parse::<f64>().ok().map(|v| v / 1000.0).unwrap_or(f64::NAN);
        let x = num(&l[42..49]);
        let y = num(&l[50..57]);
        out.push((l[..3].to_string(), net, refdes, pin, x, y));
    }
    out
}
