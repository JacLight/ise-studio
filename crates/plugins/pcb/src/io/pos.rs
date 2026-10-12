//! Position files: KiCad's ASCII `.pos` and its CSV, and Gerber X3
//! component layers (the assembly drawing a placement machine can read:
//! each part's centre flashed with its reference, rotation, footprint,
//! value, manufacturer part, mount type and height, its courtyard outline
//! and every pin). Coordinates from the board's lower-left corner, mm, y up,
//! rotations counter-clockwise; footprint origins as KiCad writes them.

use crate::board::{BoardView, ComponentView};
use crate::footprints::PadKind;
use crate::Side;
use std::fmt::Write;

fn parts(v: &BoardView) -> Vec<&ComponentView> {
    let mut p = crate::outputs::placed(v);
    p.sort_by(|a, b| crate::natural(&a.reference, &b.reference));
    p
}

fn smd(v: &BoardView, c: &ComponentView) -> bool {
    !v.pads.iter().any(|p| p.component == c.id && p.kind == PadKind::Tht)
}

fn f4(x: f64) -> String {
    format!("{:.4}", x + 0.0)
}

pub fn ascii(v: &BoardView) -> String {
    let o = [v.bounds[0], v.bounds[1]];
    let mut s = String::new();
    let _ = writeln!(s, "### Footprint positions - created on {} ###", super::now_iso());
    let _ = writeln!(s, "### Printed by Forma");
    let _ = writeln!(s, "## Unit = mm, Angle = deg.");
    let _ = writeln!(s, "## Side : All");
    let rows: Vec<[String; 7]> = parts(v)
        .into_iter()
        .map(|c| [c.reference.clone(), c.value.replace(' ', "_"), c.footprint.replace(' ', "_"), f4(c.at[0] - o[0]), f4(c.at[1] - o[1]), f4(c.rotation.rem_euclid(360.0)), if c.side == Side::Top { "top".into() } else { "bottom".into() }])
        .collect();
    let head = ["# Ref", "Val", "Package", "PosX", "PosY", "Rot", "Side"];
    let mut w = head.map(|h| h.len());
    for r in &rows {
        for (k, x) in r.iter().enumerate() {
            w[k] = w[k].max(x.len());
        }
    }
    let line = |cells: Vec<String>| -> String {
        cells.iter().enumerate().map(|(k, x)| if (3..6).contains(&k) { format!("{x:>width$}", width = w[k]) } else { format!("{x:<width$}", width = w[k]) }).collect::<Vec<_>>().join("  ").trim_end().to_string()
    };
    let _ = writeln!(s, "{}", line(head.iter().map(|x| x.to_string()).collect()));
    for r in rows {
        let _ = writeln!(s, "{}", line(r.to_vec()));
    }
    let _ = writeln!(s, "## End");
    s
}

pub fn csv(v: &BoardView) -> String {
    let o = [v.bounds[0], v.bounds[1]];
    let q = |x: &str| if x.contains([',', '"']) { format!("\"{}\"", x.replace('"', "\"\"")) } else { x.to_string() };
    let mut s = String::from("Ref,Val,Package,PosX,PosY,Rot,Side\n");
    for c in parts(v) {
        let _ = writeln!(s, "{},{},{},{},{},{},{}", q(&c.reference), q(&c.value), q(&c.footprint), f4(c.at[0] - o[0]), f4(c.at[1] - o[1]), f4(c.rotation.rem_euclid(360.0)), if c.side == Side::Top { "top" } else { "bottom" });
    }
    s
}

fn field(s: &str) -> String {
    s.chars().map(|ch| if matches!(ch, ',' | '*' | '%' | '\\') || ch.is_control() { '_' } else { ch }).collect()
}

fn c6(x: f64) -> i64 {
    (x * 1e6).round() as i64
}

/// A Gerber X3 component layer for one side.
pub fn gerber_x3(v: &BoardView, top: bool) -> String {
    let n = v.layers.len();
    let mut s = String::new();
    let _ = writeln!(s, "%TF.GenerationSoftware,Forma,PCB,{}*%", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "%TF.CreationDate,{}*%", super::now_iso());
    let _ = writeln!(s, "%TF.ProjectId,{},{},{}*%", field(&v.name).replace(' ', "_"), v.id, field(&v.revision));
    let _ = writeln!(s, "%TF.SameCoordinates,Original*%");
    let _ = writeln!(s, "%TF.FileFunction,Component,L{},{}*%", if top { 1 } else { n }, if top { "Top" } else { "Bot" });
    let _ = writeln!(s, "%TF.FilePolarity,Positive*%");
    let _ = writeln!(s, "%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\nG01*");
    let _ = writeln!(s, "%TA.AperFunction,ComponentMain*%\n%ADD10C,0.300000*%\n%TD*%");
    let _ = writeln!(s, "%TA.AperFunction,ComponentOutline,Courtyard*%\n%ADD11C,0.100000*%\n%TD*%");
    let _ = writeln!(s, "%TA.AperFunction,ComponentPin*%\n%ADD12P,0.360000X4X0.000000*%\n%ADD13C,0*%\n%TD*%");
    let side = if top { Side::Top } else { Side::Bottom };
    for c in parts(v).into_iter().filter(|c| c.side == side) {
        let at = crate::outputs::centroid(v, c);
        let _ = writeln!(s, "%TO.C,{}*%", field(&c.reference));
        let _ = writeln!(s, "%TO.CRot,{}*%", super::num(c.rotation.rem_euclid(360.0), 3));
        if let Some(m) = &c.manufacturer {
            let _ = writeln!(s, "%TO.CMfr,{}*%", field(m));
        }
        if let Some(m) = &c.mpn {
            let _ = writeln!(s, "%TO.CMPN,{}*%", field(m));
        }
        if !c.value.is_empty() {
            let _ = writeln!(s, "%TO.CVal,{}*%", field(&c.value));
        }
        let _ = writeln!(s, "%TO.CMnt,{}*%", if smd(v, c) { "SMD" } else { "TH" });
        let _ = writeln!(s, "%TO.CFtp,{}*%", field(&c.footprint));
        if c.height > 0.0 {
            let _ = writeln!(s, "%TO.CHgt,{}*%", super::num(c.height, 3));
        }
        let _ = writeln!(s, "D10*\nX{}Y{}D03*", c6(at[0]), c6(at[1]));
        if c.courtyard.len() >= 3 {
            let _ = writeln!(s, "D11*");
            let _ = writeln!(s, "X{}Y{}D02*", c6(c.courtyard[0][0]), c6(c.courtyard[0][1]));
            for p in c.courtyard.iter().skip(1).chain(std::iter::once(&c.courtyard[0])) {
                let _ = writeln!(s, "X{}Y{}D01*", c6(p[0]), c6(p[1]));
            }
        }
        let mut first = true;
        for p in v.pads.iter().filter(|p| p.component == c.id && !p.number.is_empty()) {
            let _ = writeln!(s, "%TO.P,{},{}{}*%", field(&c.reference), field(&p.number), p.pin_name.as_ref().map(|n| format!(",{}", field(n))).unwrap_or_default());
            let _ = writeln!(s, "{}\nX{}Y{}D03*", if first { "D12*" } else { "D13*" }, c6(p.at[0]), c6(p.at[1]));
            first = false;
        }
        let _ = writeln!(s, "%TD*%");
    }
    let _ = writeln!(s, "M02*");
    s
}
