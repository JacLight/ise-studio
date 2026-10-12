//! IDF 3.0 board (.emn) and library (.emp) files for MCAD tools: the board
//! outline with its thickness and cutouts, every drilled hole, and each
//! populated component placed with its package (an outline and a height).
//! Millimetres; the outline loop is counter-clockwise, cutouts clockwise;
//! package outlines are drawn from the top, as IDF expects (MCAD mirrors
//! bottom-side parts itself).

use crate::board::BoardView;
use crate::footprints::BodyShape;
use crate::geom::{self, P};
use crate::{Board, Side};
use std::collections::BTreeMap;
use std::fmt::Write;

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "'"))
}

fn f(v: f64) -> String {
    let s = format!("{:.4}", if v.abs() < 5e-5 { 0.0 } else { v });
    s
}

/// Today as IDF wants it: yyyy/mm/dd.hh:mm:ss (UTC).
fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}/{m:02}/{d:02}.{:02}:{:02}:{:02}", rem / 3600, (rem / 60) % 60, rem % 60)
}

fn loop_lines(out: &mut String, label: u32, poly: &[P]) {
    for p in poly.iter().chain(poly.first()) {
        let _ = writeln!(out, "{label} {} {} 0", f(p[0]), f(p[1]));
    }
}

/// The package name and part number of a component.
fn package(c: &crate::board::ComponentView) -> (String, String) {
    let part = c.mpn.clone().filter(|m| !m.is_empty()).unwrap_or_else(|| if c.value.is_empty() { c.footprint.clone() } else { c.value.clone() });
    (c.footprint.clone(), part)
}

/// `<stem>.emn` and `<stem>.emp`.
pub fn files(view: &BoardView, board: &Board) -> Vec<(String, String)> {
    let stem = crate::gerber::file_stem(&view.name);
    let date = stamp();
    let mut emn = String::new();
    let _ = writeln!(emn, ".HEADER\nBOARD_FILE 3.0 \"Forma\" {date} 1\n{} MM\n.END_HEADER", q(&view.name));
    let _ = writeln!(emn, ".BOARD_OUTLINE ECAD\n{}", f(view.thickness));
    let mut outline = view.outline.clone();
    if geom::polygon_area(&outline) < 0.0 {
        outline.reverse();
    }
    loop_lines(&mut emn, 0, &outline);
    for (i, c) in view.cutouts.iter().enumerate() {
        let mut c = c.clone();
        if geom::polygon_area(&c) > 0.0 {
            c.reverse();
        }
        loop_lines(&mut emn, i as u32 + 1, &c);
    }
    let _ = writeln!(emn, ".END_BOARD_OUTLINE\n.DRILLED_HOLES");
    for h in &view.holes {
        // IDF holes are round: a slot is written as its width at its centre.
        let d = h.size[0].min(h.size[1]);
        let plated = if h.plated { "PTH" } else { "NPTH" };
        let (owner, kind) = if h.owner == "via" {
            ("BOARD".to_string(), "VIA")
        } else {
            let reference = h.owner.split('.').next().unwrap_or(&h.owner).to_string();
            let comp = view.components.iter().find(|c| c.reference == reference);
            let kind = if comp.is_some_and(|c| c.kind == "hole") { "MTG" } else if h.plated { "PIN" } else { "TOOL" };
            (reference, kind)
        };
        let _ = writeln!(emn, "{} {} {} {plated} {} {kind} ECAD", f(d), f(h.at[0]), f(h.at[1]), q(&owner));
    }
    let _ = writeln!(emn, ".END_DRILLED_HOLES\n.PLACEMENT");
    let placed: Vec<&crate::board::ComponentView> = view.components.iter().filter(|c| !c.dnp && c.kind != "hole" && c.kind != "logo" && c.known).collect();
    for c in &placed {
        let (pkg, part) = package(c);
        let _ = writeln!(emn, "{} {} {}", q(&pkg), q(&part), c.reference);
        let _ = writeln!(emn, "{} {} 0.0 {} {} PLACED", f(c.at[0]), f(c.at[1]), f(c.rotation), if c.side == Side::Bottom { "BOTTOM" } else { "TOP" });
    }
    let _ = writeln!(emn, ".END_PLACEMENT");

    // The library: one entry per package and part number, drawn in footprint coordinates.
    let mut emp = String::new();
    let _ = writeln!(emp, ".HEADER\nLIBRARY_FILE 3.0 \"Forma\" {date} 1\n.END_HEADER");
    let mut done: BTreeMap<(String, String), ()> = BTreeMap::new();
    for c in &placed {
        let key = package(c);
        if done.insert(key.clone(), ()).is_some() {
            continue;
        }
        let Some(fp) = board.footprint(&c.footprint) else { continue };
        let height = fp.height().max(0.1);
        // The bodies' extent (a single cylinder stays round), else the courtyard.
        let outline: Vec<P> = if fp.body.len() == 1 && fp.body[0].shape == BodyShape::Cylinder {
            let b = &fp.body[0];
            (0..24).map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 24.0;
                [b.at[0] + b.size[0] / 2.0 * a.cos(), b.at[1] + b.size[0] / 2.0 * a.sin()]
            }).collect()
        } else {
            let r = if fp.body.is_empty() {
                fp.courtyard_rect()
            } else {
                fp.body.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |m, b| [m[0].min(b.at[0] - b.size[0] / 2.0), m[1].min(b.at[1] - b.size[1] / 2.0), m[2].max(b.at[0] + b.size[0] / 2.0), m[3].max(b.at[1] + b.size[1] / 2.0)])
            };
            vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
        };
        let _ = writeln!(emp, ".ELECTRICAL\n{} {} MM {}", q(&key.0), q(&key.1), f(height));
        loop_lines(&mut emp, 0, &outline);
        let _ = writeln!(emp, ".END_ELECTRICAL");
    }
    vec![(format!("{stem}.emn"), emn), (format!("{stem}.emp"), emp)]
}
