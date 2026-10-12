//! Assembly files and the fabrication package: pick-and-place (JLCPCB CPL,
//! PCBWay centroid, generic), BOM (JLCPCB, PCBWay, full), and a zip with the
//! Gerbers, drill files, BOM, placement and a readme of the board's specs.
//!
//! Formats as the fabs publish them (checked 2026-10-11):
//! - JLCPCB CPL: `Designator,Mid X,Mid Y,Layer,Rotation`, values like
//!   `95.0518mm`, layer Top/Bottom, degrees counter-clockwise (their sample CPL).
//! - JLCPCB BOM: `Comment,Designator,Footprint,LCSC Part #` (their sample BOM
//!   and KiCad guide).
//! - PCBWay BOM: `Item #,Designator,Qty,Manufacturer,Mfg Part #,Description / Value,
//!   Package/Footprint,Type,Your Instructions / Notes` (their sample BOM).

use crate::board::{BoardView, ComponentView};
use crate::footprints::PadKind;
use crate::geom::P;
use crate::sourcing::{self, BomLine};
use crate::{Board, Side};
use std::fmt::Write;

fn csv(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Where a pick-and-place machine takes the part: the centre of its body (the pads' centre when it has none).
pub fn centroid(view: &BoardView, c: &ComponentView) -> P {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let pts: Vec<P> = if c.bodies.is_empty() {
        view.pads.iter().filter(|p| p.component == c.id).flat_map(|p| {
            let bb = p.prim.bbox();
            [[bb[0], bb[1]], [bb[2], bb[3]]]
        }).collect()
    } else {
        c.bodies.iter().flat_map(|bd| bd.outline.clone()).collect()
    };
    for p in &pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    if pts.is_empty() { c.at } else { [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0] }
}

/// Parts a machine places: not holes, not DNP.
pub fn placed(view: &BoardView) -> Vec<&ComponentView> {
    view.components.iter().filter(|c| c.kind != "hole" && c.kind != "logo" && !c.dnp && c.known).collect()
}

fn is_tht(view: &BoardView, c: &ComponentView) -> bool {
    view.pads.iter().any(|p| p.component == c.id && p.kind == PadKind::Tht) && !view.pads.iter().any(|p| p.component == c.id && p.kind == PadKind::Smd)
}

fn fmt_mm(v: f64) -> String {
    format!("{:.4}", v + 0.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jlcpcb,
    Pcbway,
    Generic,
}

impl Format {
    pub fn parse(s: &str) -> Format {
        match s {
            "jlcpcb" => Format::Jlcpcb,
            "pcbway" => Format::Pcbway,
            _ => Format::Generic,
        }
    }
}

/// Coordinates are relative to the board's lower-left corner (what fabs expect), mm.
pub fn pick_and_place(view: &BoardView, format: Format) -> String {
    let o = [view.bounds[0], view.bounds[1]];
    let mut s = String::new();
    let mut parts = placed(view);
    parts.sort_by(|a, b| crate::natural(&a.reference, &b.reference));
    match format {
        Format::Jlcpcb => {
            let _ = writeln!(s, "Designator,Mid X,Mid Y,Layer,Rotation");
            for c in parts {
                let p = centroid(view, c);
                let _ = writeln!(s, "{},{}mm,{}mm,{},{}", csv(&c.reference), fmt_mm(p[0] - o[0]), fmt_mm(p[1] - o[1]), if c.side == Side::Top { "Top" } else { "Bottom" }, rot(c.rotation));
            }
        }
        Format::Pcbway => {
            let _ = writeln!(s, "Designator,Footprint,Mid X,Mid Y,Layer,Rotation,Comment,Type");
            for c in parts {
                let p = centroid(view, c);
                let _ = writeln!(s, "{},{},{}mm,{}mm,{},{},{},{}", csv(&c.reference), csv(&c.footprint), fmt_mm(p[0] - o[0]), fmt_mm(p[1] - o[1]), if c.side == Side::Top { "T" } else { "B" }, rot(c.rotation), csv(&c.value), if is_tht(view, c) { "THT" } else { "SMD" });
            }
        }
        Format::Generic => {
            let _ = writeln!(s, "Ref,Value,Footprint,PosX,PosY,Rot,Side");
            for c in parts {
                let p = centroid(view, c);
                let _ = writeln!(s, "{},{},{},{},{},{},{}", csv(&c.reference), csv(&c.value), csv(&c.footprint), fmt_mm(p[0] - o[0]), fmt_mm(p[1] - o[1]), rot(c.rotation), if c.side == Side::Top { "top" } else { "bottom" });
            }
        }
    }
    s
}

fn rot(r: f64) -> String {
    let r = (r.rem_euclid(360.0) * 1000.0).round() / 1000.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

pub fn bom(view: &BoardView, board: &Board, format: Format) -> String {
    let lines = sourcing::bom_lines(view, board);
    let mut s = String::new();
    match format {
        Format::Jlcpcb => {
            let _ = writeln!(s, "Comment,Designator,Footprint,LCSC Part #");
            for l in lines.iter().filter(|l| !l.dnp) {
                let _ = writeln!(s, "{},{},{},{}", csv(&comment(l)), csv(&l.references.join(",")), csv(&l.footprint), csv(l.lcsc.as_deref().unwrap_or("")));
            }
        }
        Format::Pcbway => {
            let _ = writeln!(s, "Item #,Designator,Qty,Manufacturer,Mfg Part #,Description / Value,Package/Footprint,Type,Your Instructions / Notes");
            for (i, l) in lines.iter().enumerate() {
                let kind = if l.dnp { "DNS" } else if l.tht { "THT" } else { "SMD" };
                let _ = writeln!(s, "{},{},{},{},{},{},{},{},{}", i + 1, csv(&l.references.join(",")), l.quantity, csv(l.manufacturer.as_deref().unwrap_or("")), csv(l.mpn.as_deref().unwrap_or("")), csv(&describe(l)), csv(&l.footprint), kind, csv(l.lcsc.as_deref().map(|c| format!("LCSC {c}")).as_deref().unwrap_or("")));
            }
        }
        Format::Generic => {
            let _ = writeln!(s, "Line,Quantity,Designators,Value,Footprint,Manufacturer,MPN,LCSC Part #,LCSC class,Description,DNP,Supplier,Supplier part,Unit price,Currency");
            for (i, l) in lines.iter().enumerate() {
                let c = l.choice.clone().unwrap_or_default();
                let _ = writeln!(
                    s,
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    i + 1,
                    l.quantity,
                    csv(&l.references.join(" ")),
                    csv(&l.value),
                    csv(&l.footprint),
                    csv(l.manufacturer.as_deref().unwrap_or("")),
                    csv(l.mpn.as_deref().unwrap_or("")),
                    csv(l.lcsc.as_deref().unwrap_or("")),
                    csv(l.lcsc_class.unwrap_or("")),
                    csv(&l.description),
                    if l.dnp { "DNP" } else { "" },
                    csv(c.supplier.as_deref().unwrap_or("")),
                    csv(c.sku.as_deref().unwrap_or("")),
                    c.unit_price.map(|p| format!("{p}")).unwrap_or_default(),
                    csv(c.currency.as_deref().unwrap_or("")),
                );
            }
        }
    }
    s
}

fn comment(l: &BomLine) -> String {
    if l.value.is_empty() { l.mpn.clone().unwrap_or_default() } else { l.value.clone() }
}

fn describe(l: &BomLine) -> String {
    match (l.value.is_empty(), l.description.is_empty()) {
        (false, false) => format!("{} {}", l.value, l.description),
        (false, true) => l.value.clone(),
        _ => l.description.clone(),
    }
}

/// A readme for the fab: what the board is, layer by layer.
pub fn readme(view: &BoardView, board: &Board, fab: Option<&crate::fabs::Fab>) -> String {
    let size = view.size();
    let mut s = String::new();
    let _ = writeln!(s, "{} — revision {}", view.name, view.revision);
    let _ = writeln!(s, "Made with Forma. Gerber RS-274X with X2 attributes, Excellon drill, millimetres.\n");
    let _ = writeln!(s, "Board: {:.2} × {:.2} mm, {} copper layers, {} mm FR4, {} oz outer copper", size[0], size[1], view.layers.len(), view.thickness, view.copper_oz);
    let _ = writeln!(s, "Finish: {}   Solder mask: {}   Silkscreen: {}", view.finish, view.mask, view.silk);
    let st = &view.stats;
    let _ = writeln!(s, "Smallest track {:.3} mm, smallest gap {:.3} mm, smallest plated hole {:.3} mm, smallest annular ring {:.3} mm", st.smallest_track.unwrap_or(0.0), st.smallest_gap.unwrap_or(0.0), st.smallest_drill.unwrap_or(0.0), st.smallest_annular.unwrap_or(0.0));
    let _ = writeln!(s, "{} components ({} top, {} bottom), {} holes, {} vias; design-rule check: {} errors", st.components, st.top_parts, st.bottom_parts, st.holes, st.vias, view.errors());
    let _ = writeln!(s, "Quantity: {}\n", board.quantity);
    let stem = crate::gerber::file_stem(&view.name);
    let _ = writeln!(s, "Files:");
    let _ = writeln!(s, "  {stem}-F_Cu.gtl / -B_Cu.gbl     top / bottom copper");
    if view.layers.len() == 4 {
        let _ = writeln!(s, "  {stem}-In1_Cu.g2l / -In2_Cu.g3l  inner copper (layers 2 and 3)");
    }
    let _ = writeln!(s, "  -F_Mask.gts / -B_Mask.gbs        solder mask openings");
    let _ = writeln!(s, "  -F_Paste.gtp / -B_Paste.gbp      paste stencil");
    let _ = writeln!(s, "  -F_Silkscreen.gto / -B_Silkscreen.gbo  legend");
    let _ = writeln!(s, "  -Edge_Cuts.gm1                   board outline (profile)");
    let _ = writeln!(s, "  -PTH.drl / -NPTH.drl             plated / unplated holes (slots as G85)");
    let _ = writeln!(s, "  bom.csv, positions.csv           assembly (coordinates from the board's lower-left corner)");
    if let Some(f) = fab {
        let _ = writeln!(s, "\nPrepared for {}: {}", f.name, f.files);
    }
    let _ = writeln!(s, "\nPlacement rotations are counter-clockwise from the top, from each footprint's IPC-7351 zero orientation; check them in the fab's assembly preview (their libraries' zero orientations differ for some packages).");
    s
}

/// The whole package for a fab (or a generic one): (file name, bytes).
pub fn package(view: &BoardView, board: &Board, fab: Option<&str>) -> Vec<(String, Vec<u8>)> {
    let f = fab.and_then(crate::fabs::fab);
    let format = match f.map(|f| f.id) {
        Some("jlcpcb") => Format::Jlcpcb,
        Some("pcbway") => Format::Pcbway,
        _ => Format::Generic,
    };
    let mut files: Vec<(String, Vec<u8>)> = crate::gerber::files(view).into_iter().map(|(n, s)| (n, s.into_bytes())).collect();
    files.push(("bom.csv".into(), bom(view, board, format).into_bytes()));
    files.push(("positions.csv".into(), pick_and_place(view, format).into_bytes()));
    if format != Format::Generic {
        files.push(("bom-full.csv".into(), bom(view, board, Format::Generic).into_bytes()));
    }
    files.push(("README.txt".into(), readme(view, board, f).into_bytes()));
    files
}

pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    de_shape::fabricate::zip(files)
}
