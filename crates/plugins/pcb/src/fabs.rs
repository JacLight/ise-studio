//! Board fabs: what each needs, its published limits checked against the
//! board as designed, and a price estimate only where the fab publishes its
//! formula (OSH Park, Aisler). Every figure cites the fab's own page as read
//! on 2026-10-11. No fab documents prefilled quote links, so the quote link
//! opens their quote page and the board's specs are shown to enter there.

use crate::board::BoardView;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Caps {
    pub layers: &'static [u8],
    pub min_track: f64,
    pub min_space: f64,
    /// Smallest finished plated component hole.
    pub min_drill: f64,
    pub min_via_drill: f64,
    /// Annular ring on component pads and on vias.
    pub min_annular: f64,
    pub min_annular_via: f64,
    /// Copper to the routed edge; None when not published.
    pub edge: Option<f64>,
    pub silk_width: f64,
    pub silk_height: f64,
    /// Thicknesses offered (mm); empty when not published.
    pub thicknesses: &'static [f64],
    pub finishes: &'static [&'static str],
    pub masks: &'static [&'static str],
    pub copper_oz: &'static [f64],
    /// Largest board (mm), when published.
    pub max_size: Option<[f64; 2]>,
    pub min_size: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Fab {
    pub id: &'static str,
    pub name: &'static str,
    pub country: &'static str,
    pub site: &'static str,
    pub quote: &'static str,
    pub capabilities_source: &'static str,
    /// What to upload.
    pub files: &'static str,
    pub assembly: &'static str,
    pub notes: &'static str,
    /// Two-layer and four-layer limits.
    pub two: Caps,
    pub four: Option<Caps>,
}

const STD_THICK: &[f64] = &[0.4, 0.6, 0.8, 1.0, 1.2, 1.6, 2.0];

pub const FABS: &[Fab] = &[
    Fab {
        id: "jlcpcb",
        name: "JLCPCB",
        country: "China",
        site: "https://jlcpcb.com",
        quote: "https://cart.jlcpcb.com/quote",
        capabilities_source: "https://jlcpcb.com/capabilities/pcb-capabilities",
        files: "Zip of Gerber RS-274X (Protel extensions) and Excellon drill in mm; for assembly, bom.csv (Comment, Designator, Footprint, LCSC Part #) and positions.csv (Designator, Mid X, Mid Y, Layer, Rotation)",
        assembly: "PCBA from their LCSC stock: Basic parts carry no setup fee, Extended parts do",
        notes: "Annular ring 0.25 mm recommended (0.18 absolute) on 2 layers; holes below 0.3 mm cost more",
        two: Caps { layers: &[1, 2], min_track: 0.10, min_space: 0.10, min_drill: 0.15, min_via_drill: 0.15, min_annular: 0.18, min_annular_via: 0.05, edge: Some(0.2), silk_width: 0.15, silk_height: 1.0, thicknesses: STD_THICK, finishes: &["HASL", "HASL lead-free", "ENIG", "OSP"], masks: &["green", "purple", "red", "yellow", "blue", "white", "black"], copper_oz: &[1.0, 2.0, 2.5, 3.5, 4.5], max_size: Some([670.0, 600.0]), min_size: Some(3.0) },
        four: Some(Caps { layers: &[4], min_track: 0.09, min_space: 0.09, min_drill: 0.15, min_via_drill: 0.15, min_annular: 0.15, min_annular_via: 0.05, edge: Some(0.2), silk_width: 0.15, silk_height: 1.0, thicknesses: STD_THICK, finishes: &["HASL", "HASL lead-free", "ENIG", "OSP"], masks: &["green", "purple", "red", "yellow", "blue", "white", "black"], copper_oz: &[1.0, 2.0], max_size: Some([663.0, 593.0]), min_size: Some(3.0) }),
    },
    Fab {
        id: "pcbway",
        name: "PCBWay",
        country: "China",
        site: "https://www.pcbway.com",
        quote: "https://www.pcbway.com/orderonline.aspx",
        capabilities_source: "https://www.pcbway.com/capabilities.html",
        files: "Gerber RS-274X and Excellon (zip); for assembly, bom.csv (their BOM columns) and positions.csv",
        assembly: "Turnkey or kitted assembly; the BOM needs manufacturer, MPN and type (SMD/THT) per line",
        notes: "Green, red, yellow and blue mask at no extra cost; edge clearance is not published for the standard tier (0.25 mm is their advanced tier's)",
        two: Caps { layers: &[1, 2], min_track: 0.10, min_space: 0.10, min_drill: 0.15, min_via_drill: 0.15, min_annular: 0.15, min_annular_via: 0.15, edge: None, silk_width: 0.15, silk_height: 0.8, thicknesses: &[0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 1.6, 2.0, 2.4, 2.6, 2.8, 3.0, 3.2], finishes: &["HASL", "HASL lead-free", "ENIG", "OSP", "Immersion silver", "Immersion tin", "Hard gold", "ENEPIG"], masks: &["green", "red", "yellow", "blue", "white", "black", "purple", "matt green", "matt black"], copper_oz: &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], max_size: Some([600.0, 1200.0]), min_size: Some(3.0) },
        four: Some(Caps { layers: &[4], min_track: 0.10, min_space: 0.10, min_drill: 0.15, min_via_drill: 0.15, min_annular: 0.15, min_annular_via: 0.15, edge: None, silk_width: 0.15, silk_height: 0.8, thicknesses: &[0.4, 0.6, 0.8, 1.0, 1.2, 1.6, 2.0, 2.4, 2.6, 2.8, 3.0, 3.2], finishes: &["HASL", "HASL lead-free", "ENIG", "OSP", "Immersion silver", "Immersion tin", "Hard gold", "ENEPIG"], masks: &["green", "red", "yellow", "blue", "white", "black", "purple", "matt green", "matt black"], copper_oz: &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], max_size: Some([560.0, 1150.0]), min_size: Some(3.0) }),
    },
    Fab {
        id: "oshpark",
        name: "OSH Park",
        country: "USA",
        site: "https://oshpark.com",
        quote: "https://oshpark.com/",
        capabilities_source: "https://docs.oshpark.com/services/",
        files: "Gerber and Excellon zip (Protel extensions are recognised), or a KiCad board file",
        assembly: "Boards only",
        notes: "Purple mask over ENIG, sold in sets of three; 2-layer FR4 1.6 mm (0.8 mm with 2 oz), 4-layer FR408 1.6 mm",
        two: Caps { layers: &[2], min_track: 0.1524, min_space: 0.1524, min_drill: 0.254, min_via_drill: 0.254, min_annular: 0.127, min_annular_via: 0.127, edge: None, silk_width: 0.0, silk_height: 0.0, thicknesses: &[1.6, 0.8], finishes: &["ENIG"], masks: &["purple"], copper_oz: &[1.0, 2.0], max_size: None, min_size: None },
        four: Some(Caps { layers: &[4], min_track: 0.127, min_space: 0.127, min_drill: 0.254, min_via_drill: 0.254, min_annular: 0.1016, min_annular_via: 0.1016, edge: None, silk_width: 0.0, silk_height: 0.0, thicknesses: &[1.6], finishes: &["ENIG"], masks: &["purple"], copper_oz: &[1.0], max_size: None, min_size: None }),
    },
    Fab {
        id: "eurocircuits",
        name: "Eurocircuits (PCB proto)",
        country: "Belgium / Hungary",
        site: "https://www.eurocircuits.com",
        quote: "https://www.eurocircuits.com/",
        capabilities_source: "https://www.eurocircuits.com/pcb-design-guidelines-classification/",
        files: "Gerber X2 or RS-274X with Excellon (their visualizer checks them on upload)",
        assembly: "PCB assembly service available",
        notes: "PCB proto allows pattern class 6 (0.15 mm) and drill class B (0.35 mm PTH); their STANDARD pool goes to class 8 (0.10 mm) and drill class D (0.15 mm). Finishes, colours and sizes are not checked here (not published in the classification)",
        two: Caps { layers: &[1, 2], min_track: 0.15, min_space: 0.15, min_drill: 0.35, min_via_drill: 0.35, min_annular: 0.125, min_annular_via: 0.125, edge: None, silk_width: 0.0, silk_height: 0.0, thicknesses: &[], finishes: &[], masks: &[], copper_oz: &[], max_size: None, min_size: None },
        four: Some(Caps { layers: &[4], min_track: 0.15, min_space: 0.15, min_drill: 0.35, min_via_drill: 0.35, min_annular: 0.125, min_annular_via: 0.125, edge: None, silk_width: 0.0, silk_height: 0.0, thicknesses: &[], finishes: &[], masks: &[], copper_oz: &[], max_size: None, min_size: None }),
    },
    Fab {
        id: "aisler",
        name: "Aisler",
        country: "Germany",
        site: "https://aisler.net",
        quote: "https://aisler.net/",
        capabilities_source: "https://community.aisler.net/t/2-layer-1-6mm-35-m-enig-design-rules/3732",
        files: "Gerber and Excellon (or the KiCad board directly)",
        assembly: "Assembly available",
        notes: "ENIG rules: component holes from 0.5 mm with a 0.30 mm ring, vias 0.25–0.45 mm with a 0.10 mm ring; boards 10 mm to 500 × 500 mm",
        two: Caps { layers: &[2], min_track: 0.125, min_space: 0.125, min_drill: 0.5, min_via_drill: 0.25, min_annular: 0.30, min_annular_via: 0.10, edge: Some(0.30), silk_width: 0.15, silk_height: 0.8, thicknesses: &[1.6], finishes: &["HASL", "HASL lead-free", "ENIG"], masks: &["green", "white", "black", "red", "blue"], copper_oz: &[1.0, 2.0], max_size: Some([500.0, 500.0]), min_size: Some(10.0) },
        four: Some(Caps { layers: &[4], min_track: 0.125, min_space: 0.125, min_drill: 0.5, min_via_drill: 0.25, min_annular: 0.30, min_annular_via: 0.10, edge: Some(0.30), silk_width: 0.15, silk_height: 0.8, thicknesses: &[1.6, 0.8], finishes: &["ENIG"], masks: &["green", "white", "black", "red", "blue"], copper_oz: &[1.0, 2.0], max_size: Some([500.0, 500.0]), min_size: Some(10.0) }),
    },
];

pub fn fab(id: &str) -> Option<&'static Fab> {
    FABS.iter().find(|f| f.id == id)
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub what: String,
    pub ok: bool,
    pub board: String,
    pub fab: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Estimate {
    pub amount: f64,
    pub currency: &'static str,
    /// The formula with the board's numbers in it.
    pub working: String,
    pub source: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct FabReport {
    pub fab: &'static Fab,
    pub can_make: bool,
    pub checks: Vec<Check>,
    pub estimate: Option<Estimate>,
}

fn same_finish(a: &str, b: &str) -> bool {
    let n = |s: &str| s.to_ascii_lowercase().replace(['-', ' '], "");
    n(a) == n(b)
}

/// The board as designed against one fab's published limits.
pub fn check(f: &'static Fab, v: &BoardView) -> FabReport {
    let layers = v.layers.len() as u8;
    let caps = if layers >= 4 { f.four.as_ref() } else { Some(&f.two) };
    let mut checks = Vec::new();
    let mm = |x: f64| format!("{x:.3} mm");
    let Some(c) = caps else {
        return FabReport { fab: f, can_make: false, checks: vec![Check { what: "Layers".into(), ok: false, board: layers.to_string(), fab: "not offered".into() }], estimate: None };
    };
    let st = &v.stats;
    let mut add = |what: &str, ok: bool, board: String, fab: String| checks.push(Check { what: what.into(), ok, board, fab });
    add("Layers", c.layers.contains(&layers), layers.to_string(), c.layers.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(", "));
    let track = st.smallest_track.unwrap_or(v.rules.min_track).min(f64::INFINITY);
    add("Track width", track + 1e-9 >= c.min_track, mm(track), format!("≥ {}", mm(c.min_track)));
    let gap = st.smallest_gap.unwrap_or(v.rules.min_space);
    add("Copper gap", gap + 1e-9 >= c.min_space, mm(gap), format!("≥ {}", mm(c.min_space)));
    let comp_holes: Vec<f64> = v.pads.iter().filter(|p| p.kind == crate::footprints::PadKind::Tht).filter_map(|p| p.drill.map(|d| d[0].min(d[1]))).collect();
    if let Some(h) = comp_holes.iter().cloned().reduce(f64::min) {
        add("Component holes", h + 1e-9 >= c.min_drill, mm(h), format!("≥ {}", mm(c.min_drill)));
    }
    if let Some(h) = v.vias.iter().map(|x| x.drill).reduce(f64::min) {
        add("Via drill", h + 1e-9 >= c.min_via_drill, mm(h), format!("≥ {}", mm(c.min_via_drill)));
    }
    let ring = v.pads.iter().filter(|p| p.kind == crate::footprints::PadKind::Tht).filter_map(|p| p.drill.map(|d| ((p.size[0] - d[0]).min(p.size[1] - d[1])) / 2.0)).reduce(f64::min);
    if let Some(r) = ring {
        add("Pad annular ring", r + 1e-9 >= c.min_annular, mm(r), format!("≥ {}", mm(c.min_annular)));
    }
    if let Some(r) = v.vias.iter().map(|x| (x.diameter - x.drill) / 2.0).reduce(f64::min) {
        add("Via annular ring", r + 1e-9 >= c.min_annular_via, mm(r), format!("≥ {}", mm(c.min_annular_via)));
    }
    if let Some(e) = c.edge {
        add("Copper to edge (rule)", v.rules.edge_clearance + 1e-9 >= e, mm(v.rules.edge_clearance), format!("≥ {}", mm(e)));
    }
    if c.silk_height > 0.0 {
        add("Silkscreen text", v.rules.silk_height + 1e-9 >= c.silk_height && v.rules.silk_width + 1e-9 >= c.silk_width, format!("{} tall, {} lines", mm(v.rules.silk_height), mm(v.rules.silk_width)), format!("≥ {} tall, {} lines", mm(c.silk_height), mm(c.silk_width)));
    }
    if !c.thicknesses.is_empty() {
        add("Thickness", c.thicknesses.iter().any(|t| (t - v.thickness).abs() < 1e-6), format!("{} mm", v.thickness), c.thicknesses.iter().map(|t| format!("{t}")).collect::<Vec<_>>().join(", ") + " mm");
    }
    if !c.finishes.is_empty() {
        add("Finish", c.finishes.iter().any(|x| same_finish(x, &v.finish)), v.finish.clone(), c.finishes.join(", "));
    }
    if !c.masks.is_empty() {
        add("Mask colour", c.masks.iter().any(|m| m.eq_ignore_ascii_case(&v.mask)), v.mask.clone(), c.masks.join(", "));
    }
    if !c.copper_oz.is_empty() {
        add("Copper weight", c.copper_oz.iter().any(|o| (o - v.copper_oz).abs() < 1e-6), format!("{} oz", v.copper_oz), c.copper_oz.iter().map(|o| format!("{o}")).collect::<Vec<_>>().join(", ") + " oz");
    }
    let size = v.size();
    if let Some(m) = c.max_size {
        let (a, b) = (size[0].max(size[1]), size[0].min(size[1]));
        add("Size", a <= m[0].max(m[1]) + 1e-6 && b <= m[0].min(m[1]) + 1e-6, format!("{:.1} × {:.1} mm", size[0], size[1]), format!("up to {:.0} × {:.0} mm", m[0], m[1]));
    }
    if let Some(m) = c.min_size {
        add("Smallest side", size[0].min(size[1]) + 1e-6 >= m, format!("{:.1} mm", size[0].min(size[1])), format!("≥ {m} mm"));
    }
    let can_make = checks.iter().all(|c| c.ok);
    FabReport { fab: f, can_make, checks, estimate: estimate(f, v) }
}

/// Only where the fab publishes the formula.
pub fn estimate(f: &Fab, v: &BoardView) -> Option<Estimate> {
    let area_mm2 = crate::geom::polygon_area(&v.outline).abs();
    let four = v.layers.len() >= 4;
    match f.id {
        "oshpark" => {
            // $5 per square inch for three boards (2 layers), $10 for 4 layers.
            let sq_in = area_mm2 / 645.16;
            let rate = if four { 10.0 } else { 5.0 };
            let sets = (v.quantity as f64 / 3.0).ceil().max(1.0);
            Some(Estimate { amount: sq_in * rate * sets, currency: "USD", working: format!("{sq_in:.2} in² × ${rate}/in² × {sets} set{} of 3", if sets == 1.0 { "" } else { "s" }), source: "https://docs.oshpark.com/services/" })
        }
        "aisler" => {
            // Job fee + area × area fee × quantity (sets of 3), net of VAT; Budget service.
            let cm2 = area_mm2 / 100.0;
            let enig = v.finish.to_ascii_lowercase().contains("enig");
            let (fee, per) = match (four, enig) {
                (true, _) => (14.0, 0.13),
                (false, true) => (12.0, 0.097),
                (false, false) => (12.0, 0.067),
            };
            let sets = (v.quantity as f64 / 3.0).ceil().max(1.0);
            Some(Estimate { amount: fee + cm2 * per * sets, currency: "EUR", working: format!("€{fee} job fee + {cm2:.1} cm² × €{per}/cm² × {sets} set{} of 3 (Budget, net of VAT)", if sets == 1.0 { "" } else { "s" }), source: "https://community.aisler.net/t/our-simple-pricing" })
        }
        _ => None,
    }
}

pub fn reports(v: &BoardView) -> Vec<FabReport> {
    FABS.iter().map(|f| check(f, v)).collect()
}
