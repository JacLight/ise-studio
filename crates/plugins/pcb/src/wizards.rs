//! Footprint wizards: parametric IPC-7351B land patterns for any size of
//! the common package families, from the package's toleranced dimensions
//! (as its datasheet or JEDEC outline gives them), at density level A (most
//! material), B (nominal) or C (least). Through-hole parts follow IPC-2222
//! hole and ring sizing. Each wizard returns an ordinary [`Footprint`] that
//! the footprint editor can change further and save to a library.
//!
//! | wizard     | packages                                             |
//! |------------|------------------------------------------------------|
//! | `chip`     | R/C/L/LED/diode chips: 0201 … 2512 or any L × W     |
//! | `gullwing` | SOIC, SOP, SSOP, TSSOP, MSOP (two rows), exposed pad |
//! | `qfp`      | QFP/LQFP/TQFP (four sides), exposed pad              |
//! | `qfn`      | QFN (four sides) and DFN/SON (`rows: 2`), exposed pad |
//! | `sot`      | SOT-23-3/5/6, SOT-223, SOT-89                        |
//! | `bga`      | any grid, pitch and ball; depopulated balls          |
//! | `dip`      | DIP through-hole, 300/400/600 mil rows               |
//! | `header`   | pin headers and connectors, 1–2 rows, any pitch, mounting holes |
//!
//! Coordinates: mm, y up, origin at the package centre (DIP and headers at
//! pin 1, as KiCad's), pin 1 at the top left, numbered counter-clockwise.

use crate::footprints::{BodyPart, Footprint, Goals, Pad, PadKind, PadShape, Range, body, land, r6, smd, tht, with_courtyard};
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

/// A toleranced dimension [min, max] (mm); a single number is exact.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Dim {
    Exact(f64),
    Range([f64; 2]),
}

impl Dim {
    pub fn range(&self) -> Range {
        match *self {
            Dim::Exact(v) => Range(v, v),
            Dim::Range([a, b]) => Range(a.min(b), a.max(b)),
        }
    }
    pub fn nominal(&self) -> f64 {
        let r = self.range();
        (r.0 + r.1) / 2.0
    }
}

/// IPC-7351B density level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Density {
    /// Most material (A): low-density boards, hand soldering, harsh environments.
    A,
    /// Nominal (B): most products.
    #[default]
    B,
    /// Least material (C): dense boards.
    C,
}

/// Solder fillet goals per family and level (IPC-7351B tables 3-2, 3-5, 3-6 and 3-15).
pub fn goals(family: &str, density: Density, fine: bool) -> Goals {
    use Density::*;
    let g = |toe, heel, side, courtyard| Goals { toe, heel, side, courtyard };
    match (family, density) {
        ("chip", A) => g(0.55, 0.0, 0.05, 0.5),
        ("chip", B) => g(0.35, 0.0, 0.0, 0.25),
        ("chip", C) => g(0.15, 0.0, -0.05, 0.1),
        ("chip_small", A) => g(0.20, 0.0, 0.05, 0.2),
        ("chip_small", B) => g(0.10, 0.0, 0.0, 0.15),
        ("chip_small", C) => g(0.05, 0.0, -0.05, 0.1),
        ("gull", A) => g(0.55, 0.45, if fine { 0.01 } else { 0.05 }, 0.5),
        ("gull", B) => g(0.35, 0.35, if fine { -0.02 } else { 0.03 }, 0.25),
        ("gull", C) => g(0.15, 0.25, if fine { -0.04 } else { 0.01 }, 0.1),
        ("nolead", A) => g(0.40, 0.0, -0.04, 0.5),
        ("nolead", B) => g(0.30, 0.0, -0.04, 0.25),
        ("nolead", C) => g(0.20, 0.0, -0.04, 0.1),
        _ => g(0.35, 0.35, 0.03, 0.25),
    }
}

/// An exposed (thermal) pad under the body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExposedPad {
    pub size: [f64; 2],
    /// Paste as an n × n windowpane covering about `paste` of the pad (default 3 × 3 at 50 %… 2 × 2 under 3 mm).
    #[serde(default)]
    pub paste_grid: Option<u32>,
    #[serde(default)]
    pub paste: Option<f64>,
    /// Thermal vias: a grid of n × n plated holes of `via_drill` (0 for none).
    #[serde(default)]
    pub vias: Option<u32>,
    #[serde(default)]
    pub via_drill: Option<f64>,
}

/// What a wizard is asked for. Dimensions follow JEDEC letters where they exist.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "wizard", rename_all = "snake_case")]
pub enum Wizard {
    /// Two-terminal chip: `size` an EIA code (0201 … 2512), or L, W, T given.
    Chip {
        #[serde(default)]
        size: Option<String>,
        /// R, C, L, D or LED (sets the body and name).
        #[serde(default = "prefix_r")]
        prefix: String,
        l: Option<Dim>,
        w: Option<Dim>,
        /// Terminal (end band) length.
        t: Option<Dim>,
        height: Option<f64>,
        #[serde(default)]
        density: Density,
        name: Option<String>,
    },
    /// Two rows of gull-wing leads (SOIC, SSOP, TSSOP, MSOP).
    Gullwing {
        pins: u32,
        pitch: f64,
        /// Lead span tip to tip (JEDEC E).
        span: Dim,
        /// Foot length (JEDEC L).
        lead: Dim,
        /// Lead width (JEDEC b).
        width: Dim,
        /// Body [E1, D].
        body: [f64; 2],
        height: f64,
        #[serde(default)]
        ep: Option<ExposedPad>,
        #[serde(default)]
        density: Density,
        name: Option<String>,
    },
    /// Four sides of gull-wing leads (QFP): `pins` in all, a quarter per side.
    Qfp {
        pins: u32,
        pitch: f64,
        /// Lead span tip to tip (JEDEC D and E; square unless `span_y`).
        span: Dim,
        #[serde(default)]
        span_y: Option<Dim>,
        lead: Dim,
        width: Dim,
        body: [f64; 2],
        height: f64,
        #[serde(default)]
        ep: Option<ExposedPad>,
        #[serde(default)]
        density: Density,
        name: Option<String>,
    },
    /// No-lead: QFN on four sides (`rows: 4`, default) or DFN/SON on two.
    Qfn {
        pins: u32,
        pitch: f64,
        /// Body size [D, E] (the terminals end at its edge).
        body: [Dim; 2],
        /// Terminal length (JEDEC L) and width (b).
        lead: Dim,
        width: Dim,
        height: f64,
        #[serde(default = "four")]
        rows: u32,
        #[serde(default)]
        ep: Option<ExposedPad>,
        #[serde(default)]
        density: Density,
        name: Option<String>,
    },
    /// Small-outline transistors: SOT-23 (3), SOT-23-5, SOT-23-6, SOT-223, SOT-89.
    Sot {
        package: String,
        #[serde(default)]
        density: Density,
        name: Option<String>,
    },
    /// Ball grid array.
    Bga {
        rows: u32,
        cols: u32,
        pitch: f64,
        /// Nominal ball diameter.
        ball: f64,
        body: [f64; 2],
        height: f64,
        /// Balls left out ("A1", "C3"), and a square hole of n × n missing in the middle.
        #[serde(default)]
        depopulate: Vec<String>,
        #[serde(default)]
        center_hole: u32,
        name: Option<String>,
    },
    /// Dual in-line through-hole.
    Dip {
        pins: u32,
        #[serde(default = "pitch_254")]
        pitch: f64,
        /// Row spacing (7.62 = 300 mil, 10.16, 15.24 = 600 mil).
        #[serde(default = "row_300")]
        rows: f64,
        /// Lead across its widest (diagonal of a rectangular lead), mm.
        #[serde(default = "dip_lead")]
        lead: f64,
        name: Option<String>,
    },
    /// Pin headers and through-hole connectors.
    Header {
        pins: u32,
        #[serde(default = "one_row")]
        rows: u32,
        #[serde(default = "pitch_254")]
        pitch: f64,
        /// Lead across its widest (0.64 square pin: 0.9).
        #[serde(default = "header_lead")]
        lead: f64,
        /// Body height above the board, and pin height.
        #[serde(default)]
        height: Option<f64>,
        /// Mounting/peg holes: [x, y, drill] relative to pin 1 (plated when `plated_mounting`).
        #[serde(default)]
        mounting: Vec<[f64; 3]>,
        #[serde(default)]
        plated_mounting: bool,
        name: Option<String>,
    },
}

fn prefix_r() -> String {
    "R".into()
}
fn four() -> u32 {
    4
}
fn one_row() -> u32 {
    1
}
fn pitch_254() -> f64 {
    2.54
}
fn row_300() -> f64 {
    7.62
}
fn dip_lead() -> f64 {
    0.6
}
fn header_lead() -> f64 {
    0.9
}

/// EIA chip sizes: code, metric, L, W, terminal band (Yageo RC / Murata GRM / TDK typical drawings).
pub const CHIPS: [(&str, &str, [f64; 2], [f64; 2], [f64; 2]); 9] = [
    ("0201", "0603", [0.57, 0.63], [0.27, 0.33], [0.10, 0.20]),
    ("0402", "1005", [0.95, 1.05], [0.45, 0.55], [0.10, 0.30]),
    ("0603", "1608", [1.50, 1.70], [0.70, 0.90], [0.10, 0.40]),
    ("0805", "2012", [1.90, 2.10], [1.15, 1.35], [0.15, 0.55]),
    ("1206", "3216", [3.00, 3.20], [1.50, 1.70], [0.25, 0.65]),
    ("1210", "3225", [3.00, 3.20], [2.40, 2.60], [0.25, 0.65]),
    ("1812", "4532", [4.40, 4.60], [3.00, 3.40], [0.25, 0.65]),
    ("2010", "5025", [4.90, 5.10], [2.40, 2.60], [0.35, 0.75]),
    ("2512", "6332", [6.25, 6.45], [3.05, 3.25], [0.35, 0.75]),
];

fn err(m: impl Into<String>) -> String {
    m.into()
}

fn positive(v: f64, what: &str) -> Result<f64, String> {
    if v.is_finite() && v > 0.0 && v < 200.0 { Ok(v) } else { Err(err(format!("{what} must be a positive size in mm"))) }
}

/// Run a wizard.
pub fn generate(w: &Wizard) -> Result<Footprint, String> {
    match w {
        Wizard::Chip { size, prefix, l, w, t, height, density, name } => chip(size.as_deref(), prefix, *l, *w, *t, *height, *density, name.clone()),
        Wizard::Gullwing { pins, pitch, span, lead, width, body, height, ep, density, name } => gullwing(*pins, *pitch, *span, *lead, *width, *body, *height, ep.as_ref(), *density, name.clone(), "gull"),
        Wizard::Qfp { pins, pitch, span, span_y, lead, width, body, height, ep, density, name } => quad(*pins, *pitch, *span, span_y.unwrap_or(*span), *lead, *width, *body, *height, ep.as_ref(), *density, name.clone(), "gull"),
        Wizard::Qfn { pins, pitch, body, lead, width, height, rows, ep, density, name } => {
            let bb = [body[0].nominal(), body[1].nominal()];
            if *rows == 2 {
                gullwing(*pins, *pitch, body[0], *lead, *width, bb, *height, ep.as_ref(), *density, name.clone(), "nolead")
            } else {
                quad(*pins, *pitch, body[0], body[1], *lead, *width, bb, *height, ep.as_ref(), *density, name.clone(), "nolead")
            }
        }
        Wizard::Sot { package, density, name } => sot(package, *density, name.clone()),
        Wizard::Bga { rows, cols, pitch, ball, body, height, depopulate, center_hole, name } => bga(*rows, *cols, *pitch, *ball, *body, *height, depopulate, *center_hole, name.clone()),
        Wizard::Dip { pins, pitch, rows, lead, name } => dip(*pins, *pitch, *rows, *lead, name.clone()),
        Wizard::Header { pins, rows, pitch, lead, height, mounting, plated_mounting, name } => header(*pins, *rows, *pitch, *lead, *height, mounting, *plated_mounting, name.clone()),
    }
}

fn fmt(v: f64) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

fn level(d: Density) -> &'static str {
    match d {
        Density::A => "A (most)",
        Density::B => "B (nominal)",
        Density::C => "C (least)",
    }
}

fn suffix(d: Density) -> &'static str {
    match d {
        Density::A => "_M",
        Density::B => "",
        Density::C => "_L",
    }
}

#[allow(clippy::too_many_arguments)]
fn chip(size: Option<&str>, prefix: &str, l: Option<Dim>, w: Option<Dim>, t: Option<Dim>, height: Option<f64>, density: Density, name: Option<String>) -> Result<Footprint, String> {
    let row = size.map(|s| CHIPS.iter().find(|c| c.0 == s).or_else(|| CHIPS.iter().find(|c| c.1 == s)).ok_or_else(|| err(format!("no EIA chip size {s} (sizes: {})", CHIPS.iter().map(|c| c.0).collect::<Vec<_>>().join(", "))))).transpose()?;
    let l = l.map(|d| d.range()).or(row.map(|r| Range(r.2[0], r.2[1]))).ok_or("give `size` (0603) or l, w and t")?;
    let w = w.map(|d| d.range()).or(row.map(|r| Range(r.3[0], r.3[1]))).ok_or("give `size` or w")?;
    let t = t.map(|d| d.range()).or(row.map(|r| Range(r.4[0], r.4[1]))).ok_or("give `size` or t")?;
    positive(l.0, "l")?;
    positive(w.0, "w")?;
    positive(t.0, "t")?;
    if t.1 * 2.0 >= l.0 {
        return Err(err("the terminals (t) overlap: 2·t must be shorter than l"));
    }
    let small = l.1 < 1.5;
    let g = goals(if small { "chip_small" } else { "chip" }, density, false);
    let ld = land(l, t, w, g);
    let (lnom, wnom) = ((l.0 + l.1) / 2.0, (w.0 + w.1) / 2.0);
    let (kind, material, h) = match prefix {
        "C" => ("capacitor", "Ceramic", height.unwrap_or(wnom.min(1.6))),
        "LED" => ("led", "LED", height.unwrap_or(0.6)),
        "D" => ("diode", "Epoxy", height.unwrap_or(1.0)),
        "L" => ("inductor", "Ceramic", height.unwrap_or(wnom.min(1.6))),
        _ => ("resistor", "Epoxy", height.unwrap_or(0.45)),
    };
    let code = row.map(|r| r.0.to_string()).unwrap_or_else(|| format!("{}x{}", fmt(lnom), fmt(wnom)));
    let name = name.unwrap_or_else(|| format!("{prefix}_{code}{}", suffix(density)));
    let f = Footprint {
        name,
        description: format!("{prefix} chip {code}, IPC-7351B density {}: pads {} × {} mm at ±{} mm", level(density), fmt(ld.length), fmt(ld.width), ld.center),
        source: format!("IPC-7351B land equations (wizard): L {}–{}, W {}–{}, T {}–{} mm", l.0, l.1, w.0, w.1, t.0, t.1),
        pads: vec![smd("1", [-ld.center, 0.0], [ld.length, ld.width]), smd("2", [ld.center, 0.0], [ld.length, ld.width])],
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], [lnom, wnom], [0.0, h], material)],
        kind: kind.into(),
    };
    Ok(with_courtyard(f, g.courtyard))
}

fn exposed(number: String, ep: &ExposedPad) -> Result<Vec<Pad>, String> {
    positive(ep.size[0], "ep size")?;
    positive(ep.size[1], "ep size")?;
    let n = ep.paste_grid.unwrap_or(if ep.size[0].min(ep.size[1]) < 3.0 { 2 } else { 3 }).clamp(1, 8);
    let cover = ep.paste.unwrap_or(0.5).clamp(0.2, 1.0);
    let mut pad = smd(number.clone(), [0.0, 0.0], ep.size);
    pad.shape = PadShape::Rect;
    pad.corner = None;
    if n > 1 || cover < 0.99 {
        // n × n windows covering `cover` of the pad, evenly spread.
        let k = cover.sqrt();
        let (cw, ch) = (ep.size[0] / n as f64, ep.size[1] / n as f64);
        for i in 0..n {
            for j in 0..n {
                let x = -ep.size[0] / 2.0 + cw * (i as f64 + 0.5);
                let y = -ep.size[1] / 2.0 + ch * (j as f64 + 0.5);
                pad.paste.push([r6(x), r6(y), r6(cw * k), r6(ch * k)]);
            }
        }
    }
    let mut out = vec![pad];
    if let (Some(v), Some(d)) = (ep.vias, ep.via_drill) {
        if v > 0 && d > 0.0 {
            let ring = d + 0.3;
            let (sx, sy) = (ep.size[0] / v as f64, ep.size[1] / v as f64);
            for i in 0..v {
                for j in 0..v {
                    let at = [r6(-ep.size[0] / 2.0 + sx * (i as f64 + 0.5)), r6(-ep.size[1] / 2.0 + sy * (j as f64 + 0.5))];
                    out.push(tht(number.clone(), at, [ring, ring], d, PadShape::Circle));
                }
            }
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn gullwing(pins: u32, pitch: f64, span: Dim, lead: Dim, width: Dim, bodyd: [f64; 2], height: f64, ep: Option<&ExposedPad>, density: Density, name: Option<String>, family: &str) -> Result<Footprint, String> {
    if pins < 2 || pins % 2 == 1 || pins > 200 {
        return Err(err("two rows need an even number of pins (2 to 200)"));
    }
    positive(pitch, "pitch")?;
    let g = goals(family, density, pitch <= 0.625);
    let ld = land(span.range(), lead.range(), width.range(), g);
    if ld.width >= pitch {
        return Err(err(format!("pads {} mm wide do not fit a {pitch} mm pitch: check the lead width", ld.width)));
    }
    let per = pins / 2;
    let y0 = pitch * (per as f64 - 1.0) / 2.0;
    let mut pads = Vec::new();
    for i in 0..per {
        pads.push(smd((i + 1).to_string(), [-ld.center, r6(y0 - pitch * i as f64)], [ld.length, ld.width]));
    }
    for i in 0..per {
        pads.push(smd((per + i + 1).to_string(), [ld.center, r6(-y0 + pitch * i as f64)], [ld.length, ld.width]));
    }
    if let Some(e) = ep {
        pads.extend(exposed((pins + 1).to_string(), e)?);
    }
    let fam = if family == "nolead" { "DFN" } else if pitch >= 1.27 { "SOIC" } else if pitch >= 0.65 && bodyd[0] >= 5.0 { "SSOP" } else { "TSSOP" };
    let name = name.unwrap_or_else(|| format!("{fam}-{pins}{}_{}x{}mm_P{}mm{}", if ep.is_some() { "-1EP" } else { "" }, fmt(bodyd[0]), fmt(bodyd[1]), fmt(pitch), suffix(density)));
    let f = Footprint {
        name,
        description: format!("{fam}-{pins}, {} × {} mm body, pitch {pitch} mm{}, IPC-7351B density {}: pads {} × {} mm at ±{} mm", fmt(bodyd[0]), fmt(bodyd[1]), if ep.is_some() { ", exposed pad" } else { "" }, level(density), fmt(ld.length), fmt(ld.width), ld.center),
        source: format!("IPC-7351B land equations (wizard): span {}–{}, lead {}–{}, width {}–{} mm", span.range().0, span.range().1, lead.range().0, lead.range().1, width.range().0, width.range().1),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], bodyd, [0.0, height], "Epoxy")],
        kind: "ic".into(),
    };
    Ok(finish_ic(with_courtyard(f, g.courtyard), bodyd, family == "nolead"))
}

#[allow(clippy::too_many_arguments)]
fn quad(pins: u32, pitch: f64, span_x: Dim, span_y: Dim, lead: Dim, width: Dim, bodyd: [f64; 2], height: f64, ep: Option<&ExposedPad>, density: Density, name: Option<String>, family: &str) -> Result<Footprint, String> {
    if pins < 8 || pins % 4 != 0 || pins > 400 {
        return Err(err("four sides need a multiple of 4 pins (8 to 400)"));
    }
    positive(pitch, "pitch")?;
    let g = goals(family, density, pitch <= 0.625);
    let lx = land(span_x.range(), lead.range(), width.range(), g);
    let ly = land(span_y.range(), lead.range(), width.range(), g);
    if lx.width >= pitch {
        return Err(err(format!("pads {} mm wide do not fit a {pitch} mm pitch", lx.width)));
    }
    let n = pins / 4;
    let a0 = pitch * (n as f64 - 1.0) / 2.0;
    let mut pads = Vec::new();
    for side in 0..4 {
        for i in 0..n {
            let num = side * n + i + 1;
            let along = r6(a0 - pitch * i as f64);
            let (at, size, rot) = match side {
                0 => ([-lx.center, along], [lx.length, lx.width], 0.0),  // left, top → bottom
                1 => ([-along, -ly.center], [ly.length, ly.width], 90.0), // bottom, left → right
                2 => ([lx.center, -along], [lx.length, lx.width], 0.0),  // right, bottom → top
                _ => ([along, ly.center], [ly.length, ly.width], 90.0),  // top, right → left
            };
            let mut p = smd(num.to_string(), at, size);
            p.rotation = rot;
            pads.push(p);
        }
    }
    if let Some(e) = ep {
        pads.extend(exposed((pins + 1).to_string(), e)?);
    }
    let fam = if family == "nolead" { "QFN" } else if height <= 1.2 { "TQFP" } else { "LQFP" };
    let name = name.unwrap_or_else(|| format!("{fam}-{pins}{}_{}x{}mm_P{}mm{}", if ep.is_some() { "-1EP" } else { "" }, fmt(bodyd[0]), fmt(bodyd[1]), fmt(pitch), suffix(density)));
    let f = Footprint {
        name,
        description: format!("{fam}-{pins}, {} × {} mm body, pitch {pitch} mm{}, IPC-7351B density {}: pads {} × {} mm at ±{} mm", fmt(bodyd[0]), fmt(bodyd[1]), if ep.is_some() { ", exposed pad" } else { "" }, level(density), fmt(lx.length), fmt(lx.width), lx.center),
        source: "IPC-7351B land equations (wizard)".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], bodyd, [0.0, height], "Epoxy")],
        kind: "ic".into(),
    };
    Ok(finish_ic(with_courtyard(f, g.courtyard), bodyd, family == "nolead"))
}

/// IC silkscreen: corner marks just outside the body, clear of the pads, and a pin-1 tick.
fn finish_ic(mut f: Footprint, bodyd: [f64; 2], nolead: bool) -> Footprint {
    let (hx, hy) = (bodyd[0] / 2.0 + 0.1, bodyd[1] / 2.0 + 0.1);
    let keep = 0.2;
    // The body outline cut back from copper (as the built-in footprints do).
    let mut lines = Vec::new();
    let corners = [[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]];
    for i in 0..4 {
        lines.extend(crate::footprints::clip_off_pads(corners[i], corners[(i + 1) % 4], &f.pads, keep));
    }
    if nolead || lines.is_empty() {
        let k = (bodyd[0].min(bodyd[1]) * 0.12).clamp(0.3, 1.0);
        lines = vec![
            vec![[-hx + k, hy], [-hx, hy], [-hx, hy - k]],
            vec![[hx - k, hy], [hx, hy], [hx, hy - k]],
            vec![[-hx + k, -hy], [-hx, -hy], [-hx, -hy + k]],
            vec![[hx - k, -hy], [hx, -hy], [hx, -hy + k]],
        ];
    }
    if let Some(p1) = f.pads.iter().find(|p| p.number == "1") {
        let pb = crate::geom::Prim::rect(p1.at, p1.size[0], p1.size[1], p1.rotation, 0.0).bbox();
        let x = pb[0] - 0.35;
        lines.push(vec![[r6(x), r6(p1.at[1] - 0.4)], [r6(x), r6(p1.at[1] + 0.4)]]);
    }
    f.silk = lines;
    f
}

/// JEDEC outlines of the small-outline transistors.
fn sot(package: &str, density: Density, name: Option<String>) -> Result<Footprint, String> {
    let g = goals("gull", density, false);
    let p = package.to_ascii_uppercase().replace(' ', "");
    let (pkg, f) = match p.as_str() {
        "SOT-23" | "SOT-23-3" | "SOT23" | "TO-236" => {
            let ld = land(Range(2.10, 2.64), Range(0.30, 0.60), Range(0.30, 0.50), g);
            let pads = vec![smd("1", [-ld.center, 0.95], [ld.length, ld.width]), smd("2", [-ld.center, -0.95], [ld.length, ld.width]), smd("3", [ld.center, 0.0], [ld.length, ld.width])];
            ("SOT-23", (pads, [1.30, 2.92], 1.12, "JEDEC TO-236AB"))
        }
        "SOT-23-5" | "SOT23-5" | "SOT-25" => {
            let ld = land(Range(2.60, 3.00), Range(0.30, 0.60), Range(0.30, 0.50), g);
            let s = [ld.length, ld.width];
            let pads = vec![smd("1", [-ld.center, 0.95], s), smd("2", [-ld.center, 0.0], s), smd("3", [-ld.center, -0.95], s), smd("4", [ld.center, -0.95], s), smd("5", [ld.center, 0.95], s)];
            ("SOT-23-5", (pads, [1.60, 2.90], 1.45, "JEDEC MO-178 AA"))
        }
        "SOT-23-6" | "SOT23-6" | "SOT-26" => {
            let ld = land(Range(2.60, 3.00), Range(0.30, 0.60), Range(0.30, 0.50), g);
            let s = [ld.length, ld.width];
            let pads = vec![smd("1", [-ld.center, 0.95], s), smd("2", [-ld.center, 0.0], s), smd("3", [-ld.center, -0.95], s), smd("4", [ld.center, -0.95], s), smd("5", [ld.center, 0.0], s), smd("6", [ld.center, 0.95], s)];
            ("SOT-23-6", (pads, [1.60, 2.90], 1.45, "JEDEC MO-178 AB"))
        }
        "SOT-223" | "SOT223" | "TO-261" => {
            let pl = land(Range(6.70, 7.30), Range(0.75, 1.10), Range(0.66, 0.84), g);
            let tab = land(Range(6.70, 7.30), Range(0.75, 1.10), Range(2.90, 3.10), g);
            let pads = vec![smd("1", [-pl.center, 2.3], [pl.length, pl.width]), smd("2", [-pl.center, 0.0], [pl.length, pl.width]), smd("3", [-pl.center, -2.3], [pl.length, pl.width]), smd("4", [tab.center, 0.0], [tab.length, tab.width])];
            ("SOT-223", (pads, [3.50, 6.50], 1.80, "JEDEC TO-261AA"))
        }
        "SOT-89" | "SOT89" | "TO-243" => {
            // JEDEC TO-243AA: E 3.94–4.25 (span), L 0.89–1.20, b 0.36–0.48 (pin 2: 0.44–0.56), tab 1.40–1.75 wide.
            let ld = land(Range(3.94, 4.25), Range(0.89, 1.20), Range(0.36, 0.48), g);
            let s = [ld.length, ld.width];
            let mut pads = vec![smd("1", [-ld.center, 1.5], s), smd("2", [-ld.center, 0.0], s), smd("3", [-ld.center, -1.5], s)];
            // The tab (pin 2) under the body.
            let mut t = smd("2", [r6(ld.center - 1.6), 0.0], [r6(ld.length + 2.4), 1.8]);
            t.shape = PadShape::Rect;
            t.corner = None;
            pads.push(t);
            ("SOT-89", (pads, [2.50, 4.50], 1.60, "JEDEC TO-243AA"))
        }
        _ => return Err(err(format!("no SOT package {package} (SOT-23, SOT-23-5, SOT-23-6, SOT-223, SOT-89)"))),
    };
    let (pads, bodyd, h, outline) = f;
    let fp = Footprint {
        name: name.unwrap_or_else(|| format!("{pkg}{}", suffix(density))),
        description: format!("{pkg}, IPC-7351B density {}", level(density)),
        source: format!("IPC-7351B land equations (wizard); package: {outline} outline"),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], bodyd, [0.0, h], "Epoxy")],
        kind: "ic".into(),
    };
    Ok(finish_ic(with_courtyard(fp, g.courtyard), bodyd, false))
}

/// BGA row letters: A…Z without I, O, Q, S, X, Z (JEDEC JESD95-1), then AA, AB…
pub fn bga_row(i: u32) -> String {
    const L: [char; 20] = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'L', 'M', 'N', 'P', 'R', 'T', 'U', 'V', 'W', 'Y'];
    if i < 20 { L[i as usize].to_string() } else { format!("{}{}", L[(i / 20 - 1) as usize % 20], L[(i % 20) as usize]) }
}

/// BGA land diameter for a ball: IPC-7351B reduces the land below the ball
/// (collapsing balls): 25 % for balls of 0.45 mm and up, 20 % below.
pub fn bga_land(ball: f64) -> f64 {
    let k = if ball >= 0.45 { 0.75 } else { 0.80 };
    r6((ball * k / 0.01).round() * 0.01)
}

#[allow(clippy::too_many_arguments)]
fn bga(rows: u32, cols: u32, pitch: f64, ball: f64, bodyd: [f64; 2], height: f64, depop: &[String], hole: u32, name: Option<String>) -> Result<Footprint, String> {
    if rows == 0 || cols == 0 || rows > 60 || cols > 60 {
        return Err(err("a BGA has 1 to 60 rows and columns"));
    }
    positive(pitch, "pitch")?;
    positive(ball, "ball")?;
    if ball >= pitch {
        return Err(err("the ball is as big as the pitch"));
    }
    let d = bga_land(ball);
    let (x0, y0) = (-(cols as f64 - 1.0) * pitch / 2.0, (rows as f64 - 1.0) * pitch / 2.0);
    let (hr0, hc0) = ((rows.saturating_sub(hole)) / 2, (cols.saturating_sub(hole)) / 2);
    let mut pads = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let id = format!("{}{}", bga_row(r), c + 1);
            let in_hole = hole > 0 && (hr0..hr0 + hole).contains(&r) && (hc0..hc0 + hole).contains(&c);
            if in_hole || depop.iter().any(|x| x.eq_ignore_ascii_case(&id)) {
                continue;
            }
            let mut p = smd(id, [r6(x0 + pitch * c as f64), r6(y0 - pitch * r as f64)], [d, d]);
            p.shape = PadShape::Circle;
            p.corner = None;
            pads.push(p);
        }
    }
    let n = pads.len();
    let name = name.unwrap_or_else(|| format!("BGA-{n}_{rows}x{cols}_{}x{}mm_P{}mm", fmt(bodyd[0]), fmt(bodyd[1]), fmt(pitch)));
    let mut f = Footprint {
        name,
        description: format!("BGA, {n} balls on a {rows} × {cols} grid, pitch {pitch} mm, ball ø{ball} mm, lands ø{d} mm (IPC-7351B: 25 %/20 % smaller than the ball)"),
        source: "IPC-7351B BGA land reduction (wizard); rows A… without I, O, Q, S, X, Z (JESD95-1)".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], bodyd, [0.0, height], "Epoxy")],
        kind: "ic".into(),
    };
    f = with_courtyard(f, 1.0);
    let (hx, hy) = (bodyd[0] / 2.0 + 0.1, bodyd[1] / 2.0 + 0.1);
    let k = 1.0f64.min(hx);
    f.silk = vec![vec![[-hx, hy - k], [-hx, hy], [-hx + k, hy]], vec![[hx - k, hy], [hx, hy], [hx, hy - k]], vec![[-hx, -hy + k], [-hx, -hy], [-hx + k, -hy]], vec![[hx - k, -hy], [hx, -hy], [hx, -hy + k]], vec![[-hx - 0.3, hy + 0.3], [-hx + k * 0.6, hy + 0.3]]];
    Ok(f)
}

/// IPC-2222 level B: finished hole = lead + 0.25 mm (rounded up to 0.05), pad = hole + 0.6 mm (0.3 mm ring).
pub fn tht_hole(lead: f64) -> (f64, f64) {
    let hole = ((lead + 0.25) / 0.05 - 1e-9).ceil() * 0.05;
    (r6(hole), r6(hole + 0.6))
}

fn dip(pins: u32, pitch: f64, rows: f64, lead: f64, name: Option<String>) -> Result<Footprint, String> {
    if pins < 2 || pins % 2 == 1 || pins > 80 {
        return Err(err("a DIP has an even number of pins (2 to 80)"));
    }
    positive(pitch, "pitch")?;
    positive(rows, "row spacing")?;
    let (hole, pad) = tht_hole(lead);
    if pad >= pitch {
        return Err(err("pads as big as the pitch: check the lead size"));
    }
    let per = pins / 2;
    let mut pads = Vec::new();
    for i in 0..per {
        let shape = if i == 0 { PadShape::Rect } else { PadShape::Oval };
        pads.push(tht((i + 1).to_string(), [0.0, r6(-pitch * i as f64)], [r6(pad * 1.5), pad], hole, shape));
    }
    for i in 0..per {
        pads.push(tht((per + i + 1).to_string(), [rows, r6(-pitch * (per - 1 - i) as f64)], [r6(pad * 1.5), pad], hole, PadShape::Oval));
    }
    let len = pitch * per as f64 + 0.6;
    let bw = rows - 1.27;
    let name = name.unwrap_or_else(|| format!("DIP-{pins}_W{}mm", fmt(rows)));
    let cy = -pitch * (per as f64 - 1.0) / 2.0;
    let mut f = Footprint {
        name,
        description: format!("DIP-{pins}, {} mm rows, pitch {pitch} mm, through-hole: holes ø{hole}, pads {} × {pad} mm (IPC-2222 level B)", fmt(rows), fmt(pad * 1.5)),
        source: "IPC-2222 hole and ring sizing (wizard); JEDEC MS-001 body".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([rows / 2.0, cy], [bw, len], [0.4, 4.6], "Epoxy")],
        kind: "ic".into(),
    };
    f = with_courtyard(f, 0.5);
    // Outline with a notch at pin 1's end.
    let (x0, x1, y0, y1) = (rows / 2.0 - bw / 2.0, rows / 2.0 + bw / 2.0, cy + len / 2.0, cy - len / 2.0);
    let m = rows / 2.0;
    f.silk = vec![vec![[m - 0.8, y0], [x0, y0], [x0, y1], [x1, y1], [x1, y0], [m + 0.8, y0]], (0..=8).map(|i| {
        let a = std::f64::consts::PI * i as f64 / 8.0;
        [r6(m - 0.8 * a.cos()), r6(y0 - 0.8 * a.sin())]
    }).collect()];
    Ok(f)
}

#[allow(clippy::too_many_arguments)]
fn header(pins: u32, rows: u32, pitch: f64, lead: f64, height: Option<f64>, mounting: &[[f64; 3]], plated: bool, name: Option<String>) -> Result<Footprint, String> {
    if !(rows == 1 || rows == 2) || pins == 0 || pins > 100 || pins % rows != 0 {
        return Err(err("1 or 2 rows, and a pin count that fills them (up to 100)"));
    }
    positive(pitch, "pitch")?;
    let (hole, pad) = tht_hole(lead);
    let pad = pad.min(pitch - 0.25);
    if pad <= hole + 0.1 {
        return Err(err("no room for a ring at that pitch: check the lead size"));
    }
    let n = pins / rows;
    let mut pads = Vec::new();
    for i in 0..n {
        for r in 0..rows {
            let number = if rows == 1 { i + 1 } else { i * rows + r + 1 };
            let shape = if number == 1 { PadShape::Rect } else { PadShape::Oval };
            pads.push(tht(number.to_string(), [r6(r as f64 * pitch), r6(-(i as f64) * pitch)], [pad, pad], hole, shape));
        }
    }
    for m in mounting {
        let d = m[2];
        positive(d, "mounting drill")?;
        if plated {
            pads.push(tht("MP", [m[0], m[1]], [r6(d + 0.6), r6(d + 0.6)], d, PadShape::Circle));
        } else {
            pads.push(Pad { number: String::new(), kind: PadKind::Npth, shape: PadShape::Circle, size: [d, d], at: [m[0], m[1]], rotation: 0.0, drill: Some([d, d]), corner: None, paste: vec![], polygon: vec![], extra: Default::default() });
        }
    }
    let w = rows as f64 * pitch;
    let l = n as f64 * pitch;
    let cx = (rows as f64 - 1.0) * pitch / 2.0;
    let cy = -(n as f64 - 1.0) * pitch / 2.0;
    let h = height.unwrap_or(if pitch >= 2.5 { 2.5 } else { 1.5 * pitch });
    let pin_w = (lead / std::f64::consts::SQRT_2).max(0.3);
    let parts: Vec<BodyPart> = vec![body([cx, cy], [w, l], [0.0, h], "Black plastic"), body([cx, cy], [pin_w + (rows as f64 - 1.0) * pitch, l - pitch + pin_w], [h, h + 6.0 * pitch / 2.54], "Gold")];
    let name = name.unwrap_or_else(|| format!("PinHeader_{rows}x{n:02}_P{}mm", fmt(pitch)));
    let f = Footprint {
        name,
        description: format!("Header {rows} × {n}, {pitch} mm pitch, through-hole: holes ø{hole}, pads ø{pad} mm (IPC-2222 level B){}", if mounting.is_empty() { String::new() } else { format!(", {} mounting holes", mounting.len()) }),
        source: "IPC-2222 hole and ring sizing (wizard)".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: parts,
        kind: "connector".into(),
    };
    Ok(with_courtyard(f, 0.5))
}

/// Ready-made wizard inputs for the common JEDEC packages (the person picks one, then edits it).
pub fn presets() -> Vec<(&'static str, Wizard)> {
    let d = |a: f64, b: f64| Dim::Range([a, b]);
    let gw = |pins: u32, pitch: f64, span: Dim, lead: Dim, width: Dim, body: [f64; 2], height: f64| Wizard::Gullwing { pins, pitch, span, lead, width, body, height, ep: None, density: Density::B, name: None };
    let qfp = |pins: u32, pitch: f64, span: f64, bodyd: f64, width: Dim| Wizard::Qfp { pins, pitch, span: d(span - 0.25, span + 0.25), span_y: None, lead: d(0.45, 0.75), width, body: [bodyd, bodyd], height: 1.6, ep: None, density: Density::B, name: None };
    let qfn = |pins: u32, pitch: f64, size: f64, ep: f64| Wizard::Qfn { pins, pitch, body: [d(size - 0.1, size + 0.1), d(size - 0.1, size + 0.1)], lead: d(0.30, 0.50), width: if pitch <= 0.4 { d(0.15, 0.25) } else { d(0.18, 0.30) }, height: 0.9, rows: 4, ep: Some(ExposedPad { size: [ep, ep], paste_grid: None, paste: None, vias: None, via_drill: None }), density: Density::B, name: None };
    vec![
        ("Chip 0402", Wizard::Chip { size: Some("0402".into()), prefix: "R".into(), l: None, w: None, t: None, height: None, density: Density::B, name: None }),
        ("Chip 0603", Wizard::Chip { size: Some("0603".into()), prefix: "R".into(), l: None, w: None, t: None, height: None, density: Density::B, name: None }),
        ("Chip 0805", Wizard::Chip { size: Some("0805".into()), prefix: "C".into(), l: None, w: None, t: None, height: None, density: Density::B, name: None }),
        ("Chip 2512", Wizard::Chip { size: Some("2512".into()), prefix: "R".into(), l: None, w: None, t: None, height: None, density: Density::B, name: None }),
        ("SOIC-8 (MS-012 AA)", gw(8, 1.27, d(5.80, 6.20), d(0.40, 1.27), d(0.31, 0.51), [3.90, 4.90], 1.75)),
        ("SOIC-14 (MS-012 AB)", gw(14, 1.27, d(5.80, 6.20), d(0.40, 1.27), d(0.31, 0.51), [3.90, 8.65], 1.75)),
        ("SOIC-16 (MS-012 AC)", gw(16, 1.27, d(5.80, 6.20), d(0.40, 1.27), d(0.31, 0.51), [3.90, 9.90], 1.75)),
        ("SOIC-16W (MS-013 AA)", gw(16, 1.27, d(10.00, 10.65), d(0.40, 1.27), d(0.31, 0.51), [7.50, 10.30], 2.65)),
        ("MSOP-8 (MO-187 AA)", gw(8, 0.65, d(4.75, 5.05), d(0.40, 0.70), d(0.22, 0.38), [3.00, 3.00], 1.10)),
        ("MSOP-10 (MO-187 BA)", gw(10, 0.50, d(4.75, 5.05), d(0.40, 0.70), d(0.17, 0.27), [3.00, 3.00], 1.10)),
        ("TSSOP-14 (MO-153 AB-1)", gw(14, 0.65, d(6.20, 6.60), d(0.45, 0.75), d(0.19, 0.30), [4.40, 5.00], 1.20)),
        ("TSSOP-20 (MO-153 AC)", gw(20, 0.65, d(6.20, 6.60), d(0.45, 0.75), d(0.19, 0.30), [4.40, 6.50], 1.20)),
        ("SSOP-28 (MO-150 AH)", gw(28, 0.65, d(7.40, 8.20), d(0.55, 0.95), d(0.22, 0.38), [5.30, 10.20], 2.00)),
        ("LQFP-32 7×7", qfp(32, 0.8, 9.0, 7.0, d(0.30, 0.45))),
        ("LQFP-48 7×7", qfp(48, 0.5, 9.0, 7.0, d(0.17, 0.27))),
        ("LQFP-64 10×10", qfp(64, 0.5, 12.0, 10.0, d(0.17, 0.27))),
        ("LQFP-100 14×14", qfp(100, 0.5, 16.0, 14.0, d(0.17, 0.27))),
        ("QFN-16 3×3", qfn(16, 0.5, 3.0, 1.7)),
        ("QFN-20 4×4", qfn(20, 0.5, 4.0, 2.6)),
        ("QFN-32 5×5", qfn(32, 0.5, 5.0, 3.45)),
        ("QFN-48 7×7", qfn(48, 0.5, 7.0, 5.6)),
        ("DFN-8 3×3", Wizard::Qfn { pins: 8, pitch: 0.65, body: [d(2.9, 3.1), d(2.9, 3.1)], lead: d(0.30, 0.50), width: d(0.25, 0.35), height: 0.8, rows: 2, ep: Some(ExposedPad { size: [1.6, 2.4], paste_grid: Some(2), paste: None, vias: None, via_drill: None }), density: Density::B, name: None }),
        ("SOT-23", Wizard::Sot { package: "SOT-23".into(), density: Density::B, name: None }),
        ("SOT-23-5", Wizard::Sot { package: "SOT-23-5".into(), density: Density::B, name: None }),
        ("SOT-23-6", Wizard::Sot { package: "SOT-23-6".into(), density: Density::B, name: None }),
        ("SOT-223", Wizard::Sot { package: "SOT-223".into(), density: Density::B, name: None }),
        ("SOT-89", Wizard::Sot { package: "SOT-89".into(), density: Density::B, name: None }),
        ("BGA-64 8×8 0.8", Wizard::Bga { rows: 8, cols: 8, pitch: 0.8, ball: 0.4, body: [8.0, 8.0], height: 1.2, depopulate: vec![], center_hole: 0, name: None }),
        ("BGA-256 16×16 1.0", Wizard::Bga { rows: 16, cols: 16, pitch: 1.0, ball: 0.5, body: [17.0, 17.0], height: 1.6, depopulate: vec![], center_hole: 0, name: None }),
        ("DIP-8 300 mil", Wizard::Dip { pins: 8, pitch: 2.54, rows: 7.62, lead: 0.6, name: None }),
        ("DIP-28 600 mil", Wizard::Dip { pins: 28, pitch: 2.54, rows: 15.24, lead: 0.6, name: None }),
        ("Header 1×06 2.54", Wizard::Header { pins: 6, rows: 1, pitch: 2.54, lead: 0.9, height: None, mounting: vec![], plated_mounting: false, name: None }),
        ("Header 2×05 1.27", Wizard::Header { pins: 10, rows: 2, pitch: 1.27, lead: 0.4, height: None, mounting: vec![], plated_mounting: false, name: None }),
        ("Connector 1×04 3.96 + pegs", Wizard::Header { pins: 4, rows: 1, pitch: 3.96, lead: 1.4, height: Some(9.0), mounting: vec![[-3.0, 2.0, 1.6], [14.88, 2.0, 1.6]], plated_mounting: false, name: Some("Conn_1x04_P3.96mm_Pegs".into()) }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad<'a>(f: &'a Footprint, n: &str) -> &'a Pad {
        f.pads.iter().find(|p| p.number == n).unwrap_or_else(|| panic!("{}: no pad {n}", f.name))
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn chip_0603_by_hand_from_the_ipc_equations() {
        // L 1.50–1.70, W 0.70–0.90, T 0.10–0.40, density B (toe 0.35, heel 0, side 0), F 0.05, P 0.025:
        // Z = 1.50 + 0.70 + √(0.2² + 0.05² + 0.025²) = 2.4077 → 2.45
        // S: 0.70…1.50, RMS tolerance √(0.2² + 2·0.3²) = 0.4690, Smax' = 1.5 − (0.8 − 0.469)/2 = 1.3345
        // G = 1.3345 − √(0.469² + 0.05² + 0.025²) = 0.862 → 0.85;  X = 0.70 + √(0.2² + …) = 0.9077 → 0.95
        let f = generate(&Wizard::Chip { size: Some("0603".into()), prefix: "R".into(), l: None, w: None, t: None, height: None, density: Density::B, name: None }).unwrap();
        let p1 = pad(&f, "1");
        assert!(close(p1.at[0], -0.825) && close(p1.at[1], 0.0), "{:?}", p1.at);
        assert!(close(p1.size[0], 0.8) && close(p1.size[1], 0.95), "{:?}", p1.size);
        assert!(close(pad(&f, "2").at[0], 0.825));
        // Same as the built-in R_0603 (one land algorithm).
        assert_eq!(crate::footprints::get("R_0603").unwrap().pads[0].at, p1.at);
        // Density A gives bigger pads, C smaller.
        let a = generate(&Wizard::Chip { size: Some("0603".into()), prefix: "R".into(), l: None, w: None, t: None, height: None, density: Density::A, name: None }).unwrap();
        let c = generate(&Wizard::Chip { size: Some("0603".into()), prefix: "R".into(), l: None, w: None, t: None, height: None, density: Density::C, name: None }).unwrap();
        assert!(a.pads[0].size[0] > p1.size[0] && c.pads[0].size[0] < p1.size[0]);
        assert_eq!(a.name, "R_0603_M");
    }

    #[test]
    fn soic8_matches_the_built_in_and_numbers_counter_clockwise() {
        let w = presets().into_iter().find(|(n, _)| n.starts_with("SOIC-8")).unwrap().1;
        let f = generate(&w).unwrap();
        let b = crate::footprints::get("SOIC-8").unwrap();
        for n in ["1", "4", "5", "8"] {
            assert_eq!(pad(&f, n).at, pad(&b, n).at, "pad {n}");
            assert_eq!(pad(&f, n).size, pad(&b, n).size, "pad {n}");
        }
        // Pin 1 top left, 4 bottom left, 5 bottom right, 8 top right, 1.27 apart.
        let (p1, p4, p5, p8) = (pad(&f, "1").at, pad(&f, "4").at, pad(&f, "5").at, pad(&f, "8").at);
        assert!(p1[0] < 0.0 && close(p1[1], 1.905) && close(p4[1], -1.905) && p5[0] > 0.0 && close(p5[1], -1.905) && close(p8[1], 1.905));
        assert!(close(pad(&f, "2").at[1] - pad(&f, "3").at[1], 1.27));
    }

    #[test]
    fn qfp_sides_and_pitch() {
        let w = presets().into_iter().find(|(n, _)| n.starts_with("LQFP-48")).unwrap().1;
        let f = generate(&w).unwrap();
        assert_eq!(f.pads.len(), 48);
        let (p1, p12, p13, p24, p25, p37) = (pad(&f, "1"), pad(&f, "12"), pad(&f, "13"), pad(&f, "24"), pad(&f, "25"), pad(&f, "37"));
        // Left column top→bottom, bottom row left→right, right column bottom→top, top row right→left.
        assert!(p1.at[0] < -4.0 && close(p1.at[1], 2.75), "{:?}", p1.at);
        assert!(close(p12.at[1], -2.75) && close(p12.at[0], p1.at[0]));
        assert!(close(p13.at[0], -2.75) && p13.at[1] < -4.0 && close(p13.rotation, 90.0));
        assert!(close(p24.at[0], 2.75));
        assert!(p25.at[0] > 4.0 && close(p25.at[1], -2.75));
        assert!(close(p37.at[0], 2.75) && p37.at[1] > 4.0);
        // Fine pitch: the pad is narrower than the 0.5 mm pitch with room for mask.
        assert!(p1.size[1] <= 0.3 + 1e-9, "{:?}", p1.size);
    }

    #[test]
    fn qfn_and_dfn_exposed_pad_with_paste_windows() {
        let w = presets().into_iter().find(|(n, _)| n.starts_with("QFN-32")).unwrap().1;
        let f = generate(&w).unwrap();
        let ep = pad(&f, "33");
        assert_eq!(ep.size, [3.45, 3.45]);
        assert_eq!(ep.paste.len(), 9);
        let area: f64 = ep.paste.iter().map(|w| w[2] * w[3]).sum();
        assert!((area / (3.45 * 3.45) - 0.5).abs() < 0.01, "{area}");
        // Same land as the built-in QFN-32.
        let b = crate::footprints::get("QFN-32_5x5").unwrap();
        assert_eq!(pad(&f, "1").at, pad(&b, "1").at);
        assert_eq!(pad(&f, "1").size, pad(&b, "1").size);
        let dfn = generate(&presets().into_iter().find(|(n, _)| n.starts_with("DFN-8")).unwrap().1).unwrap();
        assert_eq!(dfn.pads.len(), 9);
        assert!(dfn.name.starts_with("DFN-8-1EP"));
    }

    #[test]
    fn bga_names_skip_letters_and_lands_shrink() {
        assert_eq!((0..22).map(bga_row).collect::<Vec<_>>().join(""), "ABCDEFGHJKLMNPRTUVWYAAAB");
        let f = generate(&Wizard::Bga { rows: 10, cols: 10, pitch: 0.8, ball: 0.4, body: [9.0, 9.0], height: 1.0, depopulate: vec!["A1".into()], center_hole: 2, name: None }).unwrap();
        assert_eq!(f.pads.len(), 100 - 1 - 4);
        assert!(f.pads.iter().all(|p| p.number != "I1" && p.number != "A1"));
        let a2 = pad(&f, "A2");
        assert!(close(a2.at[0], -3.6 + 0.8) && close(a2.at[1], 3.6));
        assert!(close(a2.size[0], 0.32), "{:?}", a2.size);
        assert!(f.pads.iter().all(|p| p.number != "E5" && p.number != "F6"));
        assert!(close(bga_land(0.5), 0.38) || close(bga_land(0.5), 0.37));
    }

    #[test]
    fn dip_and_header_follow_ipc_2222() {
        assert_eq!(tht_hole(0.6), (0.85, 1.45));
        let f = generate(&Wizard::Dip { pins: 8, pitch: 2.54, rows: 7.62, lead: 0.6, name: None }).unwrap();
        assert_eq!(pad(&f, "1").at, [0.0, 0.0]);
        assert_eq!(pad(&f, "4").at, [0.0, -7.62]);
        assert_eq!(pad(&f, "5").at, [7.62, -7.62]);
        assert_eq!(pad(&f, "8").at, [7.62, 0.0]);
        assert_eq!(pad(&f, "1").shape, PadShape::Rect);
        let h = generate(&Wizard::Header { pins: 10, rows: 2, pitch: 2.54, lead: 0.9, height: None, mounting: vec![[1.27, 3.0, 2.0]], plated_mounting: false, name: None }).unwrap();
        assert_eq!(pad(&h, "2").at, [2.54, 0.0]);
        assert_eq!(pad(&h, "3").at, [0.0, -2.54]);
        assert_eq!(h.name, "PinHeader_2x05_P2.54mm");
        assert!(h.pads.iter().any(|p| p.kind == PadKind::Npth && p.drill == Some([2.0, 2.0])));
        // The 2.54 header lands on the same holes as the built-in one (1.0 mm hole for 0.64 square pins... 1.15 by IPC level B).
        assert!(pad(&h, "1").drill.unwrap()[0] >= 1.0);
    }

    #[test]
    fn every_preset_builds_and_passes_the_footprint_checker() {
        for (n, w) in presets() {
            let f = generate(&w).unwrap_or_else(|e| panic!("{n}: {e}"));
            let errs: Vec<_> = crate::fpedit::check_footprint(&f, &crate::Rules::default()).into_iter().filter(|x| x.severity == "error").collect();
            assert!(errs.is_empty(), "{n}: {errs:?}");
        }
    }

    #[test]
    fn sot_packages() {
        let f = generate(&Wizard::Sot { package: "SOT-23".into(), density: Density::B, name: None }).unwrap();
        let b = crate::footprints::get("SOT-23").unwrap();
        for n in ["1", "2", "3"] {
            assert_eq!(pad(&f, n).at, pad(&b, n).at);
        }
        assert!(generate(&Wizard::Sot { package: "SOT-99".into(), density: Density::B, name: None }).is_err());
    }
}
