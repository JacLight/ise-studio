//! Footprints: the copper, holes, courtyard, silkscreen and body of a
//! package. The built-in library is generated, not copied: surface-mount
//! lands come from the IPC-7351B land-pattern equations applied to the
//! package's published dimensions (EIA/JEDEC outlines and manufacturers'
//! drawings, each cited in `source`), and through-hole parts follow their
//! makers' recommended PCB layouts.
//!
//! Coordinates: millimetres, y up, the footprint's origin at the package
//! centre (connectors and headers: pin 1, as their drawings dimension them).
//! Zero rotation follows IPC-7351: pin 1 at the left (chips) or top left (ICs).

use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PadKind {
    /// Surface-mount: copper, mask opening and paste on the component's side.
    #[default]
    Smd,
    /// Plated through-hole: copper on every layer, mask on both sides.
    Tht,
    /// A bare hole, no copper.
    Npth,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PadShape {
    #[default]
    Rect,
    /// Rectangle with corners rounded to `corner` × the shorter side.
    RoundRect,
    Circle,
    /// Stadium (slot-shaped).
    Oval,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pad {
    /// Pad number as the datasheet names it ("1", "A1", "EP"); several pads may share one.
    pub number: String,
    #[serde(default)]
    pub kind: PadKind,
    #[serde(default)]
    pub shape: PadShape,
    /// Width (x) and height (y) before `rotation`, mm.
    pub size: [f64; 2],
    pub at: [f64; 2],
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
    /// Finished hole: [width, height] mm (equal for a round hole, unequal for a slot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill: Option<[f64; 2]>,
    /// Round-rect corner radius as a fraction of the shorter side (default 0.25).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner: Option<f64>,
    /// Paste windows instead of the whole pad: [x, y, width, height] each (an exposed pad's windowpane).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paste: Vec<[f64; 4]>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BodyShape {
    #[default]
    Box,
    Cylinder,
}

/// One block of the package body, for the 3D board and height checks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BodyPart {
    #[serde(default)]
    pub shape: BodyShape,
    /// Centre in footprint coordinates.
    pub at: [f64; 2],
    /// x and y size (a cylinder: diameter, diameter).
    pub size: [f64; 2],
    /// Bottom and top above the board surface, mm.
    pub z: [f64; 2],
    /// What it looks like and weighs: Epoxy, Ceramic, Steel, Nylon, Black plastic, Gold, LED, FR4.
    pub material: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Footprint {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Where the dimensions come from.
    #[serde(default)]
    pub source: String,
    pub pads: Vec<Pad>,
    /// Courtyard [min x, min y, max x, max y]; worked out from pads and body when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub courtyard: Option<[f64; 4]>,
    /// Silkscreen polylines; the body outline (kept off the pads) when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub silk: Vec<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<BodyPart>,
    /// resistor, capacitor, led, ic, connector, switch, module, hole.
    #[serde(default)]
    pub kind: String,
}

impl Footprint {
    pub fn height(&self) -> f64 {
        self.body.iter().map(|b| b.z[1]).fold(0.0, f64::max)
    }

    /// Courtyard, given or worked out: pads and body bounds plus `excess`.
    pub fn courtyard_rect(&self) -> [f64; 4] {
        if let Some(c) = self.courtyard {
            return c;
        }
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in &self.pads {
            let prim = crate::geom::Prim::rect(p.at, p.size[0], p.size[1], p.rotation, 0.0);
            let pb = prim.bbox();
            b = [b[0].min(pb[0]), b[1].min(pb[1]), b[2].max(pb[2]), b[3].max(pb[3])];
        }
        for part in &self.body {
            b = [b[0].min(part.at[0] - part.size[0] / 2.0), b[1].min(part.at[1] - part.size[1] / 2.0), b[2].max(part.at[0] + part.size[0] / 2.0), b[3].max(part.at[1] + part.size[1] / 2.0)];
        }
        let e = 0.25;
        [round_out(b[0] - e, false), round_out(b[1] - e, false), round_out(b[2] + e, true), round_out(b[3] + e, true)]
    }

    /// Silkscreen: the given lines, or the body outline 0.1 mm out, cut back 0.2 mm from copper, and a pin-1 mark.
    pub fn silk_lines(&self) -> Vec<Vec<[f64; 2]>> {
        if !self.silk.is_empty() {
            return self.silk.clone();
        }
        let Some(main) = self.body.iter().max_by(|a, b| (a.size[0] * a.size[1]).total_cmp(&(b.size[0] * b.size[1]))) else { return vec![] };
        if self.kind == "hole" {
            return vec![];
        }
        let (hx, hy) = (main.size[0] / 2.0 + 0.1, main.size[1] / 2.0 + 0.1);
        let c = main.at;
        let mut lines = Vec::new();
        if main.shape == BodyShape::Cylinder {
            // A circle of 36 chords.
            let pts: Vec<[f64; 2]> = (0..=36).map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 36.0;
                [c[0] + hx * a.cos(), c[1] + hx * a.sin()]
            }).collect();
            for w in pts.windows(2) {
                lines.push(vec![w[0], w[1]]);
            }
        } else {
            let corners = [[c[0] - hx, c[1] - hy], [c[0] + hx, c[1] - hy], [c[0] + hx, c[1] + hy], [c[0] - hx, c[1] + hy]];
            for i in 0..4 {
                lines.push(vec![corners[i], corners[(i + 1) % 4]]);
            }
        }
        let keep = 0.2;
        let mut out = Vec::new();
        for l in lines {
            out.extend(clip_off_pads(l[0], l[1], &self.pads, keep));
        }
        // Pin 1: a short tick outside the pad, to its left.
        if let Some(p1) = self.pads.iter().find(|p| p.number == "1") {
            if self.pads.len() > 2 || self.kind == "led" {
                let pb = crate::geom::Prim::rect(p1.at, p1.size[0], p1.size[1], p1.rotation, 0.0).bbox();
                let x = pb[0] - 0.35;
                out.push(vec![[x, p1.at[1] - 0.4], [x, p1.at[1] + 0.4]]);
            }
        }
        out
    }
}

/// The parts of segment a→b that stay `keep` mm clear of every copper pad.
fn clip_off_pads(a: [f64; 2], b: [f64; 2], pads: &[Pad], keep: f64) -> Vec<Vec<[f64; 2]>> {
    let n = 200;
    let mut out = Vec::new();
    let mut run: Option<[f64; 2]> = None;
    let mut last = a;
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let probe = crate::geom::Prim::circle(p, 0.075);
        let clear = pads.iter().filter(|q| q.kind != PadKind::Npth || q.drill.is_some()).all(|q| {
            let prim = pad_prim(q, [0.0, 0.0], 0.0, false);
            crate::geom::gap(&prim, &probe) >= keep
        });
        match (clear, run) {
            (true, None) => run = Some(p),
            (false, Some(s)) => {
                if crate::geom::dist(s, last) > 0.2 {
                    out.push(vec![s, last]);
                }
                run = None;
            }
            _ => {}
        }
        last = p;
    }
    if let Some(s) = run {
        if crate::geom::dist(s, b) > 0.2 {
            out.push(vec![s, b]);
        }
    }
    out
}

/// A pad's copper as a rounded convex, placed: footprint mirrored for the bottom side, then turned, then moved.
pub fn pad_prim(p: &Pad, at: [f64; 2], rotation: f64, bottom: bool) -> crate::geom::Prim {
    let mirror = |q: [f64; 2]| if bottom { [-q[0], q[1]] } else { q };
    let c = crate::geom::add(at, crate::geom::rotate(mirror(p.at), rotation));
    let rot = if bottom { -p.rotation } else { p.rotation } + rotation;
    let (w, h) = (p.size[0], p.size[1]);
    match p.shape {
        PadShape::Rect => crate::geom::Prim::rect(c, w, h, rot, 0.0),
        PadShape::RoundRect => crate::geom::Prim::rect(c, w, h, rot, p.corner.unwrap_or(0.25) * w.min(h)),
        PadShape::Circle => crate::geom::Prim::circle(c, w.max(h) / 2.0),
        PadShape::Oval => crate::geom::Prim::rect(c, w, h, rot, w.min(h) / 2.0),
    }
}

fn round_out(v: f64, up: bool) -> f64 {
    let g = 0.01;
    if up { (v / g - 1e-9).ceil() * g } else { (v / g + 1e-9).floor() * g }
}

// ---- IPC-7351B land patterns -------------------------------------------------

/// Fabrication and placement tolerances (mm): the IPC-7351B calculator's defaults.
const FAB_TOL: f64 = 0.05;
const PLACE_TOL: f64 = 0.025;

/// Solder fillet goals (mm) at a density level, IPC-7351B tables 3-2 to 3-17.
#[derive(Clone, Copy, Debug)]
pub struct Goals {
    pub toe: f64,
    pub heel: f64,
    pub side: f64,
    pub courtyard: f64,
}

/// Density level B ("nominal"), the level for most products.
pub mod goals {
    use super::Goals;
    /// Rectangular chips 1608 (0603) and larger: table 3-5.
    pub const CHIP: Goals = Goals { toe: 0.35, heel: 0.0, side: 0.0, courtyard: 0.25 };
    /// Rectangular chips smaller than 1608 (0402): table 3-6.
    pub const CHIP_SMALL: Goals = Goals { toe: 0.10, heel: 0.0, side: 0.0, courtyard: 0.15 };
    /// Gull-wing leads, pitch above 0.625 mm: table 3-2.
    pub const GULL: Goals = Goals { toe: 0.35, heel: 0.35, side: 0.03, courtyard: 0.25 };
    /// Gull-wing leads, pitch 0.625 mm or less: table 3-2.
    pub const GULL_FINE: Goals = Goals { toe: 0.35, heel: 0.35, side: -0.02, courtyard: 0.25 };
    /// No-lead QFN/SON: table 3-15.
    pub const QFN: Goals = Goals { toe: 0.30, heel: 0.0, side: -0.04, courtyard: 0.25 };
}

/// A dimension with its tolerance band, mm.
#[derive(Clone, Copy, Debug)]
pub struct Range(pub f64, pub f64);

impl Range {
    fn tol(&self) -> f64 {
        self.1 - self.0
    }
}

/// A land pattern for two facing rows of terminals, by the IPC-7351B equations:
///   Zmax = Lmin + 2·Jt + √(CL² + F² + P²)
///   Gmin = Smax − 2·Jh − √(CS² + F² + P²)
///   Xmax = Wmin + 2·Js + √(CW² + F² + P²)
/// with S's tolerance taken root-sum-square (√(CL² + 2·CT²)) as the standard
/// does, and Z, G, X rounded to 0.05 mm (Z and X up, G down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Land {
    /// Outside, inside and pad width extents, mm.
    pub z: f64,
    pub g: f64,
    pub x: f64,
    /// Pad length (along the terminal), width, and the centre of each pad from the package centre.
    pub length: f64,
    pub width: f64,
    pub center: f64,
}

pub fn land(l: Range, t: Range, w: Range, goals: Goals) -> Land {
    let ft = FAB_TOL * FAB_TOL + PLACE_TOL * PLACE_TOL;
    let (cl, ct, cw) = (l.tol(), t.tol(), w.tol());
    let smin = l.0 - 2.0 * t.1;
    let smax = l.1 - 2.0 * t.0;
    let stol = smax - smin;
    let stol_rms = (cl * cl + 2.0 * ct * ct).sqrt();
    let smax_rms = smax - (stol - stol_rms) / 2.0;
    let z = l.0 + 2.0 * goals.toe + (cl * cl + ft).sqrt();
    let g = smax_rms - 2.0 * goals.heel - (stol_rms * stol_rms + ft).sqrt();
    let x = w.0 + 2.0 * goals.side + (cw * cw + ft).sqrt();
    let up = |v: f64| (v / 0.05 - 1e-9).ceil() * 0.05;
    let down = |v: f64| (v / 0.05 + 1e-9).floor() * 0.05;
    let (z, g, x) = (r6(up(z)), r6(down(g).max(0.0)), r6(up(x)));
    Land { z, g, x, length: r6((z - g) / 2.0), width: x, center: r6((z + g) / 4.0) }
}

fn r6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn smd(number: impl Into<String>, at: [f64; 2], size: [f64; 2]) -> Pad {
    Pad { number: number.into(), kind: PadKind::Smd, shape: PadShape::RoundRect, size, at, rotation: 0.0, drill: None, corner: Some(0.25), paste: vec![] }
}

fn tht(number: impl Into<String>, at: [f64; 2], size: [f64; 2], drill: f64, shape: PadShape) -> Pad {
    Pad { number: number.into(), kind: PadKind::Tht, shape, size, at, rotation: 0.0, drill: Some([drill, drill]), corner: None, paste: vec![] }
}

fn body(at: [f64; 2], size: [f64; 2], z: [f64; 2], material: &str) -> BodyPart {
    BodyPart { shape: BodyShape::Box, at, size, z, material: material.into() }
}

fn cyl(at: [f64; 2], d: f64, z: [f64; 2], material: &str) -> BodyPart {
    BodyPart { shape: BodyShape::Cylinder, at, size: [d, d], z, material: material.into() }
}

fn with_courtyard(mut f: Footprint, excess: f64) -> Footprint {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in &f.pads {
        let pb = pad_prim(p, [0.0, 0.0], 0.0, false).bbox();
        b = [b[0].min(pb[0]), b[1].min(pb[1]), b[2].max(pb[2]), b[3].max(pb[3])];
    }
    for part in &f.body {
        b = [b[0].min(part.at[0] - part.size[0] / 2.0), b[1].min(part.at[1] - part.size[1] / 2.0), b[2].max(part.at[0] + part.size[0] / 2.0), b[3].max(part.at[1] + part.size[1] / 2.0)];
    }
    f.courtyard = Some([round_out(b[0] - excess, false), round_out(b[1] - excess, false), round_out(b[2] + excess, true), round_out(b[3] + excess, true)]);
    f
}

/// EIA chip sizes: (imperial code, metric code, L, W, terminal band, body height per kind).
struct ChipSize {
    code: &'static str,
    metric: &'static str,
    l: Range,
    w: Range,
    t: Range,
    /// Resistor, capacitor, LED heights (mm).
    h: [f64; 3],
}

/// Chip resistor dimensions: Yageo RC series datasheet (RC0402/0603/0805/1206,
/// "Dimensions" table); capacitor heights: Murata GRM typical thickness for
/// common values; chip LED heights: typical 0402/0603/0805/1206 LED datasheets.
const CHIPS: [ChipSize; 4] = [
    ChipSize { code: "0402", metric: "1005", l: Range(0.95, 1.05), w: Range(0.45, 0.55), t: Range(0.10, 0.30), h: [0.35, 0.5, 0.45] },
    ChipSize { code: "0603", metric: "1608", l: Range(1.50, 1.70), w: Range(0.70, 0.90), t: Range(0.10, 0.40), h: [0.45, 0.8, 0.6] },
    ChipSize { code: "0805", metric: "2012", l: Range(1.90, 2.10), w: Range(1.15, 1.35), t: Range(0.15, 0.55), h: [0.5, 1.25, 0.8] },
    ChipSize { code: "1206", metric: "3216", l: Range(3.00, 3.20), w: Range(1.50, 1.70), t: Range(0.25, 0.65), h: [0.55, 1.6, 1.1] },
];

fn chip(prefix: &str, size: &ChipSize) -> Footprint {
    let goals = if size.code == "0402" { goals::CHIP_SMALL } else { goals::CHIP };
    let land = land(size.l, size.t, size.w, goals);
    let (kind, h, material, what) = match prefix {
        "R" => ("resistor", size.h[0], "Epoxy", "Chip resistor"),
        "C" => ("capacitor", size.h[1], "Ceramic", "Multilayer ceramic capacitor"),
        _ => ("led", size.h[2], "LED", "Chip LED (pin 1 cathode)"),
    };
    let (l, w) = ((size.l.0 + size.l.1) / 2.0, (size.w.0 + size.w.1) / 2.0);
    let f = Footprint {
        name: format!("{prefix}_{}", size.code),
        description: format!("{what}, EIA {} ({} metric), IPC-7351B density B: pads {:.2} × {:.2} mm at ±{:.3} mm", size.code, size.metric, land.length, land.width, land.center),
        source: "IPC-7351B land equations; body: Yageo RC-series resistor datasheet dimensions (L, W, terminal)".into(),
        pads: vec![smd("1", [-land.center, 0.0], [land.length, land.width]), smd("2", [land.center, 0.0], [land.length, land.width])],
        courtyard: None,
        silk: if size.code == "0402" || size.code == "0603" { vec![] } else { vec![] },
        body: vec![body([0.0, 0.0], [l, w], [0.0, h], material)],
        kind: kind.into(),
    };
    let f = with_courtyard(f, goals.courtyard);
    if prefix == "LED" { f } else { Footprint { silk: chip_silk(&f), ..f } }
}

/// Small chips get silk lines along the long sides only (there is no room at the ends).
fn chip_silk(f: &Footprint) -> Vec<Vec<[f64; 2]>> {
    let b = &f.body[0];
    let pads_y = f.pads[0].size[1] / 2.0;
    let y = (b.size[1] / 2.0).max(pads_y) + 0.2;
    let x = (b.size[0] / 2.0 - 0.1).max(0.1);
    let inner = f.pads[1].at[0] - f.pads[1].size[0] / 2.0 - 0.2;
    let x = x.min(inner.max(0.15));
    vec![vec![[-x, y], [x, y]], vec![[-x, -y], [x, -y]]]
}

/// Two rows of gull-wing or no-lead terminals: `per_side` pins on the left
/// (pin 1 at the top) and as many on the right, numbered counter-clockwise.
fn dual_row(name: &str, n_left: usize, n_right_pins: &[usize], pitch: f64, land: Land, body_wh: [f64; 2], height: f64, description: String, source: &str, goals: Goals) -> Footprint {
    let mut pads = Vec::new();
    let y0 = pitch * (n_left as f64 - 1.0) / 2.0;
    for i in 0..n_left {
        pads.push(smd((i + 1).to_string(), [-land.center, r6(y0 - pitch * i as f64)], [land.length, land.width]));
    }
    // Right side, bottom to top: the positions given (in left-row slots, 0 = top).
    let mut number = n_left + 1;
    for slot in n_right_pins.iter() {
        pads.push(smd(number.to_string(), [land.center, r6(y0 - pitch * *slot as f64)], [land.length, land.width]));
        number += 1;
    }
    let f = Footprint { name: name.into(), description, source: source.into(), pads, courtyard: None, silk: vec![], body: vec![body([0.0, 0.0], body_wh, [0.0, height], "Epoxy")], kind: "ic".into() };
    with_courtyard(f, goals.courtyard)
}

fn sot23() -> Footprint {
    // JEDEC TO-236AB: HE 2.10–2.64, D 2.80–3.04, E 1.20–1.40, e 0.95, L 0.30–0.60 (taken as the foot), b 0.30–0.50, A 0.89–1.12.
    let land = land(Range(2.10, 2.64), Range(0.30, 0.60), Range(0.30, 0.50), goals::GULL);
    let mut f = dual_row(
        "SOT-23",
        2,
        &[],
        1.90,
        land,
        [1.30, 2.92],
        1.12,
        format!("SOT-23 (TO-236AB), 3 pins, pitch 0.95 mm; pads {:.3} × {:.2} mm at ±{:.4} mm", land.length, land.width, land.center),
        "IPC-7351B land equations; package: JEDEC TO-236AB outline",
        goals::GULL,
    );
    f.pads.push(smd("3", [land.center, 0.0], [land.length, land.width]));
    with_courtyard(f, goals::GULL.courtyard)
}

fn sot23_5() -> Footprint {
    // JEDEC MO-178 AA: HE 2.60–3.00, D 2.80–3.00, E1 1.50–1.70, e 0.95, L 0.30–0.60, b 0.30–0.50, A ≤ 1.45.
    let land = land(Range(2.60, 3.00), Range(0.30, 0.60), Range(0.30, 0.50), goals::GULL);
    dual_row(
        "SOT-23-5",
        3,
        &[2, 0],
        0.95,
        land,
        [1.60, 2.90],
        1.45,
        format!("SOT-23-5 (MO-178 AA), pitch 0.95 mm; pads {:.3} × {:.2} mm at ±{:.4} mm", land.length, land.width, land.center),
        "IPC-7351B land equations; package: JEDEC MO-178 variation AA",
        goals::GULL,
    )
}

fn soic8() -> Footprint {
    // JEDEC MS-012 AA: E 5.80–6.20, D 4.80–5.00, E1 3.80–4.00, e 1.27, L 0.40–1.27, b 0.31–0.51, A 1.35–1.75.
    let land = land(Range(5.80, 6.20), Range(0.40, 1.27), Range(0.31, 0.51), goals::GULL);
    dual_row(
        "SOIC-8",
        4,
        &[3, 2, 1, 0],
        1.27,
        land,
        [3.90, 4.90],
        1.75,
        format!("SOIC-8, 3.9 × 4.9 mm body (MS-012 AA), pitch 1.27 mm; pads {:.2} × {:.2} mm at ±{:.3} mm", land.length, land.width, land.center),
        "IPC-7351B land equations; package: JEDEC MS-012 variation AA",
        goals::GULL,
    )
}

fn tssop20() -> Footprint {
    // JEDEC MO-153 AC: E 6.20–6.60 (6.40 BSC), D 6.40–6.60, E1 4.30–4.50, e 0.65, L 0.45–0.75, b 0.19–0.30, A ≤ 1.20.
    let land = land(Range(6.20, 6.60), Range(0.45, 0.75), Range(0.19, 0.30), goals::GULL);
    dual_row(
        "TSSOP-20",
        10,
        &(0..10).rev().collect::<Vec<_>>(),
        0.65,
        land,
        [4.40, 6.50],
        1.20,
        format!("TSSOP-20, 4.4 × 6.5 mm body (MO-153 AC), pitch 0.65 mm; pads {:.3} × {:.2} mm at ±{:.4} mm", land.length, land.width, land.center),
        "IPC-7351B land equations; package: JEDEC MO-153 variation AC",
        goals::GULL,
    )
}

fn qfn32() -> Footprint {
    // JEDEC MO-220 (VHHD): D = E 4.90–5.10, e 0.50, b 0.18–0.30, L 0.30–0.50, exposed pad 3.30–3.60, A 0.80–1.00.
    let land = land(Range(4.90, 5.10), Range(0.30, 0.50), Range(0.18, 0.30), goals::QFN);
    let pitch = 0.5;
    let n = 8;
    let y0 = pitch * (n as f64 - 1.0) / 2.0;
    let mut pads = Vec::new();
    let size = [land.length, land.width];
    for side in 0..4 {
        for i in 0..n {
            let num = side * n + i + 1;
            let along = r6(y0 - pitch * i as f64);
            let (at, rot) = match side {
                0 => ([-land.center, along], 0.0),   // left, top → bottom
                1 => ([-along, -land.center], 90.0), // bottom, left → right
                2 => ([land.center, -along], 0.0),   // right, bottom → top
                _ => ([along, land.center], 90.0),   // top, right → left
            };
            let mut p = smd(num.to_string(), at, size);
            p.rotation = rot;
            pads.push(p);
        }
    }
    let ep = 3.45;
    // Paste on the exposed pad as a 2 × 2 windowpane, about 55 % of its area.
    let w = 1.25;
    let o = 0.8;
    let mut epad = Pad { number: "33".into(), kind: PadKind::Smd, shape: PadShape::Rect, size: [ep, ep], at: [0.0, 0.0], rotation: 0.0, drill: None, corner: None, paste: vec![] };
    epad.paste = vec![[-o, -o, w, w], [o, -o, w, w], [-o, o, w, w], [o, o, w, w]];
    pads.push(epad);
    let f = Footprint {
        name: "QFN-32_5x5".into(),
        description: format!("QFN-32, 5 × 5 mm, pitch 0.5 mm, 3.45 mm exposed pad (MO-220); pads {:.3} × {:.2} mm at ±{:.4} mm", land.length, land.width, land.center),
        source: "IPC-7351B land equations; package: JEDEC MO-220 (5 × 5 mm, 0.5 mm pitch, exposed pad 3.30–3.60)".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], [5.0, 5.0], [0.0, 1.0], "Epoxy")],
        kind: "ic".into(),
    };
    let mut f = with_courtyard(f, goals::QFN.courtyard);
    // Corner marks only (the body edge runs through the pads), pin 1 corner closed.
    let h = 2.6;
    let k = 0.45;
    f.silk = vec![
        vec![[-h + k, h], [-h, h], [-h, h - k]],
        vec![[h - k, h], [h, h], [h, h - k]],
        vec![[-h + k, -h], [-h, -h], [-h, -h + k]],
        vec![[h - k, -h], [h, -h], [h, -h + k]],
        vec![[-h - 0.35, h + 0.0], [-h - 0.35, h - 0.6]],
    ];
    f
}

/// Pin headers, 2.54 mm pitch: pin 1 at the origin, rows going −y (and +x for the second row).
fn pin_header(rows: usize, n: usize) -> Footprint {
    let pitch = 2.54;
    let mut pads = Vec::new();
    for i in 0..n {
        for r in 0..rows {
            let number = if rows == 1 { i + 1 } else { i * 2 + r + 1 };
            let shape = if number == 1 { PadShape::Rect } else { PadShape::Oval };
            pads.push(tht(number.to_string(), [r6(r as f64 * pitch), r6(-(i as f64) * pitch)], [1.7, 1.7], 1.0, shape));
        }
    }
    let w = rows as f64 * pitch;
    let l = n as f64 * pitch;
    let cx = (rows as f64 - 1.0) * pitch / 2.0;
    let cy = -(n as f64 - 1.0) * pitch / 2.0;
    let f = Footprint {
        name: format!("PinHeader_{rows}x{n:02}_2.54"),
        description: format!("Pin header {rows}×{n}, 2.54 mm pitch, straight, through-hole (0.64 mm square pins)"),
        source: "Generic 2.54 mm header: 0.64 mm square pin (0.9 mm diagonal), 1.0 mm finished hole, 1.7 mm pad; 2.5 mm insulator".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([cx, cy], [w, l], [0.0, 2.5], "Black plastic"), body([cx, cy], [0.64 + (rows as f64 - 1.0) * pitch, l - pitch + 0.64], [2.5, 8.5], "Gold")],
        kind: "connector".into(),
    };
    with_courtyard(f, 0.5)
}

/// JST PH (2.0 mm) and XH (2.5 mm) top-entry shrouded headers.
fn jst(series: &str, n: usize) -> Footprint {
    // (pitch, hole, pad, body depth, height, body width = (n-1)·pitch + extra, body centre offset from the pin row (+y = back))
    let (pitch, hole, pad, depth, height, extra, back, part) = match series {
        "PH" => (2.0, 0.7, [1.2, 1.75], 4.5, 6.0, 3.9, 0.6, format!("B{n}B-PH-K-S")),
        _ => (2.5, if n == 2 { 1.0 } else { 0.9 }, [1.7, 2.0], 5.75, 7.0, 4.9, 0.5, format!("B{n}B-XH-A")),
    };
    let mut pads = Vec::new();
    for i in 0..n {
        let shape = if i == 0 { PadShape::RoundRect } else { PadShape::Oval };
        let mut p = tht((i + 1).to_string(), [r6(i as f64 * pitch), 0.0], pad, hole, shape);
        if i == 0 {
            p.corner = Some(0.2);
        }
        pads.push(p);
    }
    let w = (n as f64 - 1.0) * pitch + extra;
    let cx = (n as f64 - 1.0) * pitch / 2.0;
    let f = Footprint {
        name: format!("JST_{series}_{n}"),
        description: format!("JST {series} {part}, {n} pins, {pitch} mm pitch, top entry, through-hole"),
        source: format!("JST {series} connector datasheet: PCB layout (pitch {pitch} mm, hole ø{hole} mm), body {w:.1} × {depth} × {height} mm"),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([cx, back], [w, depth], [0.0, height], "Nylon")],
        kind: "connector".into(),
    };
    with_courtyard(f, 0.5)
}

fn led_tht(d: f64) -> Footprint {
    let (h, flange, hole) = if d < 4.0 { (5.3, 3.8, 0.8) } else { (8.6, 5.8, 0.9) };
    let pads = vec![tht("1", [0.0, 0.0], [1.8, 1.8], hole, PadShape::Rect), tht("2", [2.54, 0.0], [1.8, 1.8], hole, PadShape::Circle)];
    let f = Footprint {
        name: format!("LED_{}mm", d as i32),
        description: format!("Through-hole LED, ø{d} mm, 2.54 mm lead spacing; pin 1 cathode (square pad, flat side)"),
        source: format!("Typical T-{} LED drawing: ø{d} mm lens, ø{flange} mm flange, {h} mm tall, 0.5 mm leads at 2.54 mm", if d < 4.0 { "1" } else { "1¾" }),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![cyl([1.27, 0.0], flange, [0.0, 1.0], "LED"), cyl([1.27, 0.0], d, [1.0, h], "LED")],
        kind: "led".into(),
    };
    with_courtyard(f, 0.5)
}

fn tactile_6x6() -> Footprint {
    // Omron B3F-1000 style: 4 legs on a 6.5 × 4.5 mm grid, ø1.0 holes; legs 1–2 and 3–4 are joined inside.
    let pads = vec![
        tht("1", [-3.25, 2.25], [1.7, 1.7], 1.0, PadShape::Circle),
        tht("1", [3.25, 2.25], [1.7, 1.7], 1.0, PadShape::Circle),
        tht("2", [-3.25, -2.25], [1.7, 1.7], 1.0, PadShape::Circle),
        tht("2", [3.25, -2.25], [1.7, 1.7], 1.0, PadShape::Circle),
    ];
    let f = Footprint {
        name: "SW_Tactile_6x6".into(),
        description: "Tactile switch 6 × 6 mm, through-hole, 4 legs (pairs joined: 1–1, 2–2); 5 mm tall".into(),
        source: "Omron B3F datasheet: mounting holes 4 × ø1.0 on 6.5 × 4.5 mm; body 6 × 6 × 3.5 mm with ø3.5 actuator".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], [6.0, 6.0], [0.0, 3.5], "Steel"), cyl([0.0, 0.0], 3.5, [3.5, 5.0], "Black plastic")],
        kind: "switch".into(),
    };
    with_courtyard(f, 0.5)
}

fn usb_c_16() -> Footprint {
    // HRO TYPE-C-31-M-12, 16-pin USB 2.0 receptacle: SMT signal pads, plated shell slots. Origin at the shell centre, mouth toward −y.
    let len = 1.15;
    let y = 4.77 - len / 2.0;
    let mut pads = Vec::new();
    let wide = [("A1", "B12", -3.20), ("A4", "B9", -2.40), ("B4", "A9", 2.40), ("B1", "A12", 3.20)];
    for (a, b, x) in wide {
        // Each wide pad serves two contacts (one per plug orientation): two pad numbers, one copper.
        pads.push(smd(a, [x, y], [0.6, len]));
        pads.push(smd(b, [x, y], [0.6, len]));
    }
    for (num, x) in [("B8", -1.75), ("A5", -1.25), ("B7", -0.75), ("A6", -0.25), ("A7", 0.25), ("B6", 0.75), ("A8", 1.25), ("B5", 1.75)] {
        pads.push(smd(num, [x, y], [0.3, len]));
    }
    let slot = |at: [f64; 2], pad: [f64; 2], hole: [f64; 2]| Pad { number: "S1".into(), kind: PadKind::Tht, shape: PadShape::Oval, size: pad, at, rotation: 0.0, drill: Some(hole), corner: None, paste: vec![] };
    for x in [-4.325, 4.325] {
        // The drawing's pads are 0.9 mm wide (a 0.15 mm ring); 1.0 mm gives 0.2 mm, above JLCPCB's 0.18 mm minimum.
        pads.push(slot([x, 3.13], [1.0, 2.1], [0.6, 1.7]));
        pads.push(slot([x, 3.13 - 4.18], [1.0, 1.8], [0.6, 1.4]));
    }
    for x in [-2.89, 2.89] {
        pads.push(Pad { number: "".into(), kind: PadKind::Npth, shape: PadShape::Circle, size: [0.6, 0.6], at: [x, 2.6], rotation: 0.0, drill: Some([0.6, 0.6]), corner: None, paste: vec![] });
    }
    let f = Footprint {
        name: "USB-C_16P".into(),
        description: "USB Type-C receptacle, 16 pins (USB 2.0: VBUS, GND, CC1/CC2, D+/D−, SBU), HRO TYPE-C-31-M-12 (LCSC C165948), horizontal, shell slots plated (S1); the mouth faces −y and should overhang the board edge slightly".into(),
        source: "HRO TYPE-C-31-M-12 drawing, recommended PCB layout: pads 0.6/0.3 mm at 0.5 mm pitch (wide pads at ±2.40, ±3.20), shell slots on 8.65 mm (0.6 × 1.7 / 0.6 × 1.4 holes, 4.18 mm apart; slot pads widened from 0.9 to 1.0 mm for a 0.2 mm ring), locating holes ø0.6 on 5.78 mm".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.15], [8.94, 7.35], [0.0, 3.26], "Steel")],
        kind: "connector".into(),
    };
    let mut f = with_courtyard(f, 0.5);
    f.silk = vec![vec![[-4.47, -1.9], [-4.47, 1.9]], vec![[4.47, -1.9], [4.47, 1.9]]];
    f
}

fn esp32_wroom_32() -> Footprint {
    // Espressif ESP32-WROOM-32E datasheet v2.1, fig. 13: 18 × 25.5 mm, 38 pads 1.5 × 0.9 at 1.27 mm, outer pad span 17.5 mm,
    // pin 1 7.49 mm below the top edge, GND pad 3.7 × 3.7 mm (3 × 3 paste windows) 7.5 mm right of the left pad row and 10.29 mm above the bottom.
    let (w, l) = (18.0, 25.5);
    let pitch = 1.27;
    let pad = [1.5, 0.9];
    let side_x = 17.5 / 2.0 - pad[0] / 2.0;
    let top_y = l / 2.0 - 7.49;
    let mut pads = Vec::new();
    for i in 0..14 {
        pads.push(smd((i + 1).to_string(), [-side_x, r6(top_y - pitch * i as f64)], pad));
    }
    let bottom_y = -(l / 2.0 - 0.25 - pad[0] / 2.0);
    for i in 0..10 {
        let mut p = smd((15 + i).to_string(), [r6(-pitch * 4.5 + pitch * i as f64), bottom_y], pad);
        p.rotation = 90.0;
        pads.push(p);
    }
    for i in 0..14 {
        pads.push(smd((25 + i).to_string(), [side_x, r6(top_y - pitch * (13 - i) as f64)], pad));
    }
    let gc = [r6(-side_x + 7.5), r6(-l / 2.0 + 10.29)];
    let windows = (0..9).map(|k| {
        let (i, j) = ((k % 3) as f64 - 1.0, (k / 3) as f64 - 1.0);
        [gc[0] + i * 1.4, gc[1] + j * 1.4, 0.9, 0.9]
    }).collect();
    pads.push(Pad { number: "39".into(), kind: PadKind::Smd, shape: PadShape::Rect, size: [3.7, 3.7], at: gc, rotation: 0.0, drill: None, corner: None, paste: windows });
    let f = Footprint {
        name: "ESP32-WROOM-32".into(),
        description: "Espressif ESP32-WROOM-32E Wi-Fi/Bluetooth module (LCSC C701341), 18 × 25.5 × 3.1 mm, 38 pads at 1.27 mm + GND pad 39; antenna at +y (top 6.19 mm): keep copper away, ideally overhang the board edge".into(),
        source: "Espressif ESP32-WROOM-32E datasheet v2.1, figure 13 (recommended PCB land pattern)".into(),
        pads,
        courtyard: None,
        silk: vec![],
        body: vec![body([0.0, 0.0], [w, l], [0.0, 0.8], "FR4"), body([0.0, -2.9], [16.6, 17.6], [0.8, 3.1], "Steel")],
        kind: "module".into(),
    };
    let mut f = with_courtyard(f, 0.25);
    let a = l / 2.0 - 6.19;
    f.silk = vec![vec![[-w / 2.0, a], [-w / 2.0, l / 2.0], [w / 2.0, l / 2.0], [w / 2.0, a]], vec![[-w / 2.0, a], [w / 2.0, a]]];
    f
}

fn mounting_hole(m: u32, plated: bool) -> Footprint {
    // ISO 273 medium clearance holes: M2 → 2.4, M2.5 → 2.9, M3 → 3.4; fine: M2 2.2, M3 3.2 (used here: snug for a PCB).
    let (hole, head) = match m {
        2 => (2.2, 3.8),
        25 => (2.7, 4.5),
        _ => (3.2, 5.5),
    };
    let keep = head + 0.5;
    let pad = if plated {
        Pad { number: "1".into(), kind: PadKind::Tht, shape: PadShape::Circle, size: [keep, keep], at: [0.0, 0.0], rotation: 0.0, drill: Some([hole, hole]), corner: None, paste: vec![] }
    } else {
        Pad { number: "".into(), kind: PadKind::Npth, shape: PadShape::Circle, size: [hole, hole], at: [0.0, 0.0], rotation: 0.0, drill: Some([hole, hole]), corner: None, paste: vec![] }
    };
    let label = if m == 25 { "M2.5".to_string() } else { format!("M{m}") };
    let mut f = Footprint {
        name: format!("MountingHole_{}{}", label.replace('.', ""), if plated { "_Pad" } else { "" }),
        description: format!("Mounting hole for {label}: ø{hole} mm{}, keep-out ø{keep} mm for the screw head", if plated { ", plated with a ring (can join GND)" } else { ", not plated" }),
        source: format!("ISO 273 fine clearance hole for {label}; head ø{head} mm (ISO 4762 / ISO 7380)"),
        pads: vec![pad],
        courtyard: None,
        silk: vec![],
        body: vec![],
        kind: "hole".into(),
    };
    let r = keep / 2.0 + 0.1;
    f.courtyard = Some([-r, -r, r, r]);
    f.silk = (0..36).map(|i| {
        let a0 = std::f64::consts::TAU * i as f64 / 36.0;
        let a1 = std::f64::consts::TAU * (i + 1) as f64 / 36.0;
        vec![[r6(r * a0.cos()), r6(r * a0.sin())], [r6(r * a1.cos()), r6(r * a1.sin())]]
    }).collect();
    f
}

/// Every built-in footprint name, with the parametric families written as patterns.
pub fn names() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for p in ["R", "C", "LED"] {
        for c in &CHIPS {
            v.push(format!("{p}_{}", c.code));
        }
    }
    v.extend(["SOT-23", "SOT-23-5", "SOIC-8", "TSSOP-20", "QFN-32_5x5", "USB-C_16P", "ESP32-WROOM-32", "SW_Tactile_6x6", "LED_3mm", "LED_5mm"].map(String::from));
    for n in 2..=6 {
        v.push(format!("JST_PH_{n}"));
    }
    for n in 2..=6 {
        v.push(format!("JST_XH_{n}"));
    }
    for n in [1, 2, 3, 4, 5, 6, 8, 10] {
        v.push(format!("PinHeader_1x{n:02}_2.54"));
    }
    for n in [3, 4, 5, 10] {
        v.push(format!("PinHeader_2x{n:02}_2.54"));
    }
    v.extend(["MountingHole_M2", "MountingHole_M25", "MountingHole_M3", "MountingHole_M3_Pad"].map(String::from));
    v
}

/// A built-in footprint by name. Families are parametric: `PinHeader_1x07_2.54`,
/// `JST_PH_9`, `JST_XH_12` work for any count, not only those `names` lists.
pub fn get(name: &str) -> Option<Footprint> {
    for p in ["R", "C", "LED"] {
        for c in &CHIPS {
            if name == format!("{p}_{}", c.code) {
                return Some(chip(p, c));
            }
        }
    }
    Some(match name {
        "SOT-23" => sot23(),
        "SOT-23-5" => sot23_5(),
        "SOIC-8" => soic8(),
        "TSSOP-20" => tssop20(),
        "QFN-32_5x5" => qfn32(),
        "USB-C_16P" => usb_c_16(),
        "ESP32-WROOM-32" => esp32_wroom_32(),
        "SW_Tactile_6x6" => tactile_6x6(),
        "LED_3mm" => led_tht(3.0),
        "LED_5mm" => led_tht(5.0),
        "MountingHole_M2" => mounting_hole(2, false),
        "MountingHole_M25" => mounting_hole(25, false),
        "MountingHole_M3" => mounting_hole(3, false),
        "MountingHole_M3_Pad" => mounting_hole(3, true),
        _ => {
            if let Some(rest) = name.strip_prefix("JST_PH_") {
                let n: usize = rest.parse().ok().filter(|n| (1..=16).contains(n))?;
                return Some(jst("PH", n));
            }
            if let Some(rest) = name.strip_prefix("JST_XH_") {
                let n: usize = rest.parse().ok().filter(|n| (1..=16).contains(n))?;
                return Some(jst("XH", n));
            }
            if let Some(rest) = name.strip_prefix("PinHeader_").and_then(|r| r.strip_suffix("_2.54")) {
                let (rows, n) = rest.split_once('x')?;
                let rows: usize = rows.parse().ok().filter(|r| *r == 1 || *r == 2)?;
                let n: usize = n.parse().ok().filter(|n| (1..=40).contains(n))?;
                return Some(pin_header(rows, n));
            }
            return None;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_footprint_builds() {
        for n in names() {
            let f = get(&n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(f.name, n);
            assert!(!f.pads.is_empty());
        }
    }
}
