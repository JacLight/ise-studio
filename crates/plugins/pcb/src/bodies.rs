//! Component bodies as real parts look: chip resistors with their black top
//! and tinned ends, ceramic capacitors, gull-wing leads on SOT/SOIC/TSSOP,
//! QFN terminations, a USB-C receptacle's steel shell and its dark mouth, JST
//! housings with their walls and pins, pin headers (black insulator, gold
//! pins through the board), tactile switches, LEDs with coloured lenses and
//! an ESP32 module (its PCB, steel can and antenna trace). Every body is a
//! few boxes, vertical cylinders, horizontal cylinders or spheres in board
//! coordinates (mm, z up from the board's bottom face), so the 3D board, its
//! STEP file and the browser's renderer draw the same thing.

use crate::footprints::{BodyShape, Footprint, PadKind};
use crate::geom::{self, P};
use crate::{Board, PcbComponent, Side};
use serde::Serialize;

/// One solid of a component body, placed on the board.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Solid {
    /// "box" (centre x, y; z from `z0`), "cyl" (vertical), "hcyl" (along the
    /// body's local y), "sphere" (centre at z0 + size/2).
    pub shape: &'static str,
    /// [x, y, z0]: footprint centre in board coordinates, bottom of the solid.
    pub at: [f64; 3],
    /// [x size, y size, height] before `rot` (cylinders: diameter, length or diameter, height).
    pub size: [f64; 3],
    /// Turn about z, degrees counter-clockwise.
    pub rot: f64,
    /// Look: epoxy, ceramic, res_top, tin, gold, steel, nylon, black, dark, mark,
    /// module_pcb, antenna, led_<colour>, led_body.
    pub mat: String,
}

/// A component's solids with where they come from.
#[derive(Clone, Debug, Serialize)]
pub struct Body {
    pub reference: String,
    pub solids: Vec<Solid>,
}

/// Local solid: footprint coordinates, z above the mounting surface (negative: into and through the board).
struct L {
    shape: &'static str,
    at: P,
    z0: f64,
    size: [f64; 3],
    rot: f64,
    mat: String,
}

fn bx(at: P, sx: f64, sy: f64, z0: f64, z1: f64, mat: &str) -> L {
    L { shape: "box", at, z0, size: [sx, sy, z1 - z0], rot: 0.0, mat: mat.into() }
}
fn cyl(at: P, d: f64, z0: f64, z1: f64, mat: &str) -> L {
    L { shape: "cyl", at, z0, size: [d, d, z1 - z0], rot: 0.0, mat: mat.into() }
}

fn led_colour(value: &str) -> String {
    let v = value.to_ascii_lowercase();
    // Addressable RGB LEDs (WS2812, SK6812, APA102) are a clear lens over three dies.
    if ["rgb", "ws281", "sk681", "apa10"].iter().any(|k| v.contains(k)) {
        return "led_white".into();
    }
    let c = ["red", "green", "blue", "yellow", "orange", "white", "amber"].into_iter().find(|c| v.contains(c)).unwrap_or("red");
    format!("led_{}", if c == "amber" { "orange" } else { c })
}

fn main_body(fp: &Footprint) -> Option<&crate::footprints::BodyPart> {
    fp.body.iter().max_by(|a, b| (a.size[0] * a.size[1]).total_cmp(&(b.size[0] * b.size[1])))
}

fn chip(fp: &Footprint, value: &str, t: f64) -> Vec<L> {
    let Some(b) = fp.body.first() else { return vec![] };
    let (l, w, h) = (b.size[0], b.size[1], b.z[1]);
    let band = (l * 0.2).clamp(0.15, 0.6);
    let mut v = Vec::new();
    match fp.kind.as_str() {
        "resistor" => {
            v.push(bx([0.0, 0.0], l - 0.02, w, 0.02, h - 0.04, "ceramic_white"));
            v.push(bx([0.0, 0.0], l - 2.0 * band, w - 0.02, h - 0.04, h, "res_top"));
        }
        "led" => {
            v.push(bx([0.0, 0.0], l - 0.02, w, 0.02, h * 0.55, "led_body"));
            v.push(bx([0.0, 0.0], l - 2.0 * band, w * 0.86, h * 0.55, h, &led_colour(value)));
        }
        _ => v.push(bx([0.0, 0.0], l - 0.02, w - 0.02, 0.02, h - 0.01, "ceramic")),
    }
    for s in [-1.0, 1.0] {
        v.push(bx([s * (l / 2.0 - band / 2.0), 0.0], band, w + 0.02, 0.0, h + if fp.kind == "resistor" { 0.005 } else { 0.0 }, "tin"));
    }
    let _ = t;
    v
}

/// Gull-wing leads: a foot on each pad, an arm out of the body's side.
fn gull(fp: &Footprint) -> Vec<L> {
    let Some(b) = main_body(fp) else { return vec![] };
    let (bw, bl, h) = (b.size[0], b.size[1], b.z[1]);
    let mut v = vec![bx([0.0, 0.0], bw, bl, 0.1, h, "epoxy")];
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Smd) {
        // Outward direction: the side of the body the pad is on.
        let horizontal = p.at[0].abs() / bw >= p.at[1].abs() / bl;
        let (dir, across) = if horizontal { ([p.at[0].signum(), 0.0], if (p.rotation / 90.0).round() as i64 % 2 == 0 { p.size[1] } else { p.size[0] }) } else { ([0.0, p.at[1].signum()], if (p.rotation / 90.0).round() as i64 % 2 == 0 { p.size[0] } else { p.size[1] }) };
        let lw = (across * 0.62).clamp(0.15, 0.5);
        let edge = if horizontal { bw / 2.0 } else { bl / 2.0 };
        let reach = if horizontal { p.at[0].abs() } else { p.at[1].abs() };
        let tip = reach + (if horizontal { p.size[0] } else { p.size[1] }).min(1.0) * 0.25;
        let foot = (tip - edge).min(0.6).max(0.25);
        let foot_c = tip - foot / 2.0;
        let at_on = |d: f64| -> P { if horizontal { [dir[0] * d, p.at[1]] } else { [p.at[0], dir[1] * d] } };
        let size = |len: f64| -> (f64, f64) { if horizontal { (len, lw) } else { (lw, len) } };
        let (fx, fy) = size(foot);
        v.push(bx(at_on(foot_c), fx, fy, 0.0, 0.13, "tin"));
        let arm = (tip - foot - edge).max(0.05) + 0.1;
        let (ax, ay) = size(arm);
        let zarm = (h * 0.45).max(0.3);
        v.push(bx(at_on(edge + arm / 2.0 - 0.05), ax, ay, zarm - 0.13, zarm, "tin"));
        let (dx, dy) = size(0.13);
        v.push(bx(at_on(tip - foot + 0.065), dx, dy, 0.0, zarm, "tin"));
    }
    // Pin 1 dimple.
    if let Some(p1) = fp.pads.iter().find(|p| p.number == "1") {
        let at = [p1.at[0].signum() * (bw / 2.0 - 0.45).max(0.2), p1.at[1].signum() * (bl / 2.0 - 0.45).max(0.2)];
        v.push(cyl(at, 0.35_f64.min(bw * 0.25), h, h + 0.005, "mark"));
    }
    v
}

fn qfn(fp: &Footprint) -> Vec<L> {
    let Some(b) = main_body(fp) else { return vec![] };
    let (bw, bl, h) = (b.size[0], b.size[1], b.z[1]);
    let mut v = vec![bx([0.0, 0.0], bw, bl, 0.02, h, "epoxy")];
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Smd) {
        let (sx, sy) = if (p.rotation / 90.0).round() as i64 % 2 == 0 { (p.size[0], p.size[1]) } else { (p.size[1], p.size[0]) };
        // Only the terminations on the edge show.
        if p.at[0].abs() + sx / 2.0 < bw / 2.0 - 0.1 && p.at[1].abs() + sy / 2.0 < bl / 2.0 - 0.1 {
            continue;
        }
        let horizontal = sx > sy;
        let (w, l) = if horizontal { (0.4, sy * 0.8) } else { (sx * 0.8, 0.4) };
        let at = if horizontal { [p.at[0].signum() * (bw / 2.0 - 0.2), p.at[1]] } else { [p.at[0], p.at[1].signum() * (bl / 2.0 - 0.2)] };
        v.push(bx(at, w + 0.02, l, 0.0, 0.2, "tin"));
    }
    v.push(cyl([-bw / 2.0 + 0.6, bl / 2.0 - 0.6], 0.4, h, h + 0.005, "mark"));
    v
}

fn usb_c(fp: &Footprint) -> Vec<L> {
    let Some(b) = main_body(fp) else { return vec![] };
    let (w, l, h) = (b.size[0], b.size[1], b.z[1]);
    let c = b.at;
    let r = h / 2.0;
    let mut v = vec![
        // The shell: a stadium in x–z, running along y.
        bx(c, w - h, l, 0.0, h, "steel"),
        L { shape: "hcyl", at: [c[0] - (w / 2.0 - r), c[1]], z0: 0.0, size: [h, l, h], rot: 0.0, mat: "steel".into() },
        L { shape: "hcyl", at: [c[0] + (w / 2.0 - r), c[1]], z0: 0.0, size: [h, l, h], rot: 0.0, mat: "steel".into() },
        // The mouth (toward −y) and its tongue.
        bx([c[0], c[1] - l / 2.0 - 0.005], w - 0.7, 0.02, 0.35, h - 0.35, "dark"),
        bx([c[0], c[1] - l / 2.0 + 0.4], w - 3.2, 0.9, h / 2.0 - 0.3, h / 2.0 + 0.3, "black"),
    ];
    // Tabs through the shell slots.
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Tht) {
        if let Some(d) = p.drill {
            v.push(bx(p.at, d[0] * 0.8, d[1] * 0.85, -1.6 - 0.8, 0.6, "steel"));
        }
    }
    // Signal legs at the back.
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Smd) {
        v.push(bx([p.at[0], p.at[1] - 0.2], p.size[0] * 0.6, p.size[1] * 0.7, 0.0, 0.12, "tin"));
    }
    v
}

fn jst(fp: &Footprint) -> Vec<L> {
    let Some(b) = main_body(fp) else { return vec![] };
    let (w, d, h) = (b.size[0], b.size[1], b.z[1]);
    let c = b.at;
    let wall = 0.6;
    let mut v = vec![
        bx(c, w, d, 0.0, 1.2, "nylon"),
        bx([c[0], c[1] + d / 2.0 - wall / 2.0], w, wall, 1.2, h, "nylon"),
        bx([c[0], c[1] - d / 2.0 + wall / 2.0], w, wall, 1.2, h * 0.72, "nylon"),
        bx([c[0] - w / 2.0 + wall / 2.0, c[1]], wall, d - 2.0 * wall, 1.2, h, "nylon"),
        bx([c[0] + w / 2.0 - wall / 2.0, c[1]], wall, d - 2.0 * wall, 1.2, h, "nylon"),
    ];
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Tht) {
        v.push(bx(p.at, 0.64, 0.64, -1.6 - 2.5, h - 1.5, "gold"));
    }
    v
}

fn header(fp: &Footprint) -> Vec<L> {
    let mut v = Vec::new();
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Tht) {
        // Each pin's own insulator block (they read as one bar), chamfered look from a slightly smaller top.
        v.push(bx(p.at, 2.5, 2.5, 0.0, 2.3, "black"));
        v.push(bx(p.at, 2.2, 2.2, 2.3, 2.5, "black"));
        v.push(bx(p.at, 0.64, 0.64, -1.6 - 3.0, 8.5, "gold"));
    }
    v
}

fn tactile(fp: &Footprint) -> Vec<L> {
    let mut v = vec![
        bx([0.0, 0.0], 6.0, 6.0, 0.0, 3.1, "black"),
        bx([0.0, 0.0], 6.0, 6.0, 3.1, 3.4, "steel"),
        cyl([0.0, 0.0], 3.5, 3.4, 5.0, "black"),
    ];
    for k in [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]] {
        v.push(cyl([k[0] * 1.9, k[1] * 1.9], 0.7, 3.4, 3.45, "steel_dark"));
    }
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Tht) {
        v.push(bx(p.at, 0.7, 0.35, -1.6 - 1.2, 0.8, "steel"));
        v.push(bx([p.at[0] * 0.9, p.at[1]], 0.8, 0.35, 0.5, 0.8, "steel"));
    }
    v
}

/// A surface-mount tactile switch from its body blocks: dark base, steel cover,
/// the plunger (the footprint's cylinder, else a round button) and tinned legs.
fn tactile_smd(fp: &Footprint) -> Vec<L> {
    let Some(b) = main_body(fp) else { return vec![] };
    let (w, l, h) = (b.size[0], b.size[1], b.z[1]);
    let mut v = vec![bx(b.at, w, l, 0.0, h * 0.72, "black"), bx(b.at, w, l, h * 0.72, h, "steel")];
    // The plunger starts a little inside the cover: a face shared with the cover's top flickers.
    match fp.body.iter().find(|p| p.shape == BodyShape::Cylinder) {
        Some(p) => v.push(cyl(p.at, p.size[0], p.z[0].max(h) - 0.05, p.z[1], "black")),
        None => v.push(cyl(b.at, w.min(l) * 0.55, h - 0.05, h + 0.3 * h.min(1.5), "black")),
    }
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Smd) {
        let (sx, sy) = if (p.rotation / 90.0).round() as i64 % 2 == 0 { (p.size[0], p.size[1]) } else { (p.size[1], p.size[0]) };
        v.push(bx(p.at, sx * 0.7, sy * 0.45, 0.0, 0.15, "tin"));
    }
    v
}

fn led_tht(fp: &Footprint, value: &str) -> Vec<L> {
    let lens = led_colour(value);
    let flange = fp.body.first();
    let dome = fp.body.get(1);
    let (Some(f), Some(d)) = (flange, dome) else { return vec![] };
    let dia = d.size[0];
    let mut v = vec![
        cyl(f.at, f.size[0], 0.6, 1.6, &lens),
        cyl(d.at, dia, 1.6, d.z[1] - dia / 2.0, &lens),
        L { shape: "sphere", at: d.at, z0: d.z[1] - dia, size: [dia, dia, dia], rot: 0.0, mat: lens.clone() },
    ];
    for p in fp.pads.iter().filter(|p| p.kind == PadKind::Tht) {
        v.push(bx(p.at, 0.5, 0.5, -1.6 - 1.5, 0.6, "tin"));
    }
    v
}

fn module(fp: &Footprint) -> Vec<L> {
    let pcb = fp.body.iter().find(|b| b.material == "FR4");
    let can = fp.body.iter().find(|b| b.material == "Steel");
    let mut v = Vec::new();
    if let Some(p) = pcb {
        v.push(bx(p.at, p.size[0], p.size[1], 0.0, p.z[1], "module_pcb"));
        // The antenna: a meandered trace across the top of the module, past the can.
        if let Some(c) = can {
            let top = p.at[1] + p.size[1] / 2.0;
            let start = c.at[1] + c.size[1] / 2.0 + 0.8;
            let x0 = p.at[0] - p.size[0] / 2.0 + 1.2;
            let x1 = p.at[0] + p.size[0] / 2.0 - 2.5;
            let z = p.z[1];
            let tw = 0.5;
            let rows = 4;
            let span = (top - 0.8 - start).max(1.0);
            for k in 0..rows {
                let y = start + span * k as f64 / (rows - 1) as f64;
                v.push(bx([(x0 + x1) / 2.0, y], x1 - x0, tw, z, z + 0.03, "antenna"));
                let x = if k % 2 == 0 { x1 } else { x0 };
                if k + 1 < rows {
                    v.push(bx([x, y + span / (rows - 1) as f64 / 2.0], tw, span / (rows - 1) as f64, z, z + 0.03, "antenna"));
                }
            }
            v.push(bx([x1 + 1.2, start + span / 2.0], tw, span + tw, z, z + 0.03, "antenna"));
        }
        // Castellations: tinned half-holes at each edge pad.
        for q in fp.pads.iter().filter(|q| q.kind == PadKind::Smd && q.size[0] < 2.0) {
            let (sx, sy) = if (q.rotation / 90.0).round() as i64 % 2 == 0 { (q.size[0], q.size[1]) } else { (q.size[1], q.size[0]) };
            let horizontal = sx > sy;
            let at = if horizontal { [q.at[0].signum() * (p.size[0] / 2.0 - 0.35), q.at[1]] } else { [q.at[0], q.at[1].signum() * (p.size[1] / 2.0 - 0.35) + p.at[1] * 0.0] };
            let (w, l) = if horizontal { (0.72, sy * 0.85) } else { (sx * 0.85, 0.72) };
            v.push(bx(at, w, l, 0.0, p.z[1] + 0.02, "tin"));
        }
    }
    if let Some(c) = can {
        v.push(bx(c.at, c.size[0], c.size[1], c.z[0], c.z[1] - 0.15, "steel"));
        v.push(bx(c.at, c.size[0] - 0.3, c.size[1] - 0.3, c.z[1] - 0.15, c.z[1], "steel"));
    }
    v
}

/// What the footprint's own body blocks look like, for anything without a model above.
fn generic(fp: &Footprint, value: &str) -> Vec<L> {
    fp.body
        .iter()
        .map(|b| {
            let mat = match b.material.as_str() {
                "Epoxy" => "epoxy".to_string(),
                "Ceramic" => "ceramic".to_string(),
                "Steel" => "steel".to_string(),
                "Nylon" => "nylon".to_string(),
                "Black plastic" => "black".to_string(),
                "Gold" => "gold".to_string(),
                "FR4" => "module_pcb".to_string(),
                "LED" => led_colour(value),
                _ => "epoxy".to_string(),
            };
            if b.shape == BodyShape::Cylinder { cyl(b.at, b.size[0], b.z[0], b.z[1], &mat) } else { bx(b.at, b.size[0], b.size[1], b.z[0], b.z[1], &mat) }
        })
        .collect()
}

fn local(fp: &Footprint, c: &PcbComponent, t: f64) -> Vec<L> {
    // KiCad library footprints are named "Library:Name" (R_0402_1005Metric, SOT-23-6,
    // USB_C_Receptacle_…): the package is the name after the library.
    let (lib, n) = fp.name.split_once(':').unwrap_or(("", fp.name.as_str()));
    let chip_code = |p: &str| n.strip_prefix(p).is_some_and(|r| r.len() >= 4 && r.as_bytes()[..4].iter().all(|ch| ch.is_ascii_digit()) && (r.len() == 4 || r[4..].starts_with('_')));
    let no_leads = n.starts_with("QFN") || n.starts_with("DFN") || ["Package_DFN_QFN", "Package_SON", "Package_LGA", "Package_CSP"].contains(&lib);
    if chip_code("R_") || chip_code("C_") || chip_code("LED_") || chip_code("L_") || chip_code("Fuse_") {
        chip(fp, &c.value, t)
    } else if ["SOT-", "SOIC", "TSSOP", "SSOP", "MSOP", "TSOT-", "SC-70", "SOD-123", "SOD-323"].iter().any(|k| n.starts_with(k)) {
        gull(fp)
    } else if no_leads {
        qfn(fp)
    } else if n.starts_with("USB-C") || n.starts_with("USB_C_Receptacle") {
        usb_c(fp)
    } else if n.starts_with("JST_") {
        jst(fp)
    } else if n.starts_with("PinHeader_") {
        header(fp)
    } else if n.starts_with("SW_Tactile") {
        tactile(fp)
    } else if fp.kind == "switch" && lib == "Button_Switch_SMD" && (n.starts_with("SW_SPST") || n.starts_with("SW_Push")) {
        tactile_smd(fp)
    } else if n == "LED_3mm" || n == "LED_5mm" {
        led_tht(fp, &c.value)
    } else if fp.kind == "module" {
        module(fp)
    } else {
        generic(fp, &c.value)
    }
}

/// Every populated component's solids, placed on the board (`t` thick, bottom face at z = 0).
pub fn bodies(board: &Board, components: &[(de_core::StableId, PcbComponent)]) -> Vec<Body> {
    let t = board.thickness;
    let mut out = Vec::new();
    for (_, c) in components {
        if c.dnp {
            continue;
        }
        let Some(fp) = board.footprint(&c.footprint) else { continue };
        let bottom = c.side == Side::Bottom;
        let solids = local(&fp, c, t)
            .into_iter()
            .map(|s| {
                let q = if bottom { [-s.at[0], s.at[1]] } else { s.at };
                let at = geom::add(c.at, geom::rotate(q, c.rotation));
                let rot = (if bottom { -s.rot } else { s.rot }) + c.rotation;
                // Through-board parts (z < 0) are measured from the mounting face; scale them to this board.
                let (z0, z1) = (s.z0, s.z0 + s.size[2]);
                let (z0, z1) = (if z0 < -1.6 + 1e-9 { z0 + 1.6 - t } else { z0 }, z1);
                let (a, b) = if bottom { (-z1, -z0) } else { (t + z0, t + z1) };
                let (a, b) = if s.shape == "sphere" { (a, a + s.size[2]) } else { (a, b) };
                Solid { shape: s.shape, at: [at[0], at[1], a], size: [s.size[0], s.size[1], b - a], rot: rot.rem_euclid(360.0), mat: s.mat }
            })
            .filter(|s| s.size.iter().all(|v| *v > 1e-6))
            .collect();
        out.push(Body { reference: c.reference.clone(), solids });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprints::{BodyPart, Pad};

    fn fp(name: &str, kind: &str, pads: Vec<([f64; 2], [f64; 2])>, body: Vec<BodyPart>) -> Footprint {
        Footprint {
            name: name.into(),
            description: String::new(),
            source: String::new(),
            pads: pads.into_iter().enumerate().map(|(i, (at, size))| serde_json::from_value::<Pad>(serde_json::json!({ "number": (i + 1).to_string(), "kind": "smd", "shape": "rect", "size": size, "at": at })).unwrap()).collect(),
            courtyard: None,
            silk: vec![],
            body,
            kind: kind.into(),
        }
    }
    fn part(value: &str) -> PcbComponent {
        serde_json::from_value(serde_json::json!({ "board": de_core::StableId::new(), "reference": "X1", "value": value, "footprint": "x", "at": [0.0, 0.0] })).unwrap()
    }
    fn block(at: [f64; 2], size: [f64; 2], z: [f64; 2], material: &str) -> BodyPart {
        BodyPart { shape: BodyShape::Box, at, size, z, material: material.into() }
    }
    fn mats(v: &[L]) -> Vec<String> {
        v.iter().map(|s| s.mat.clone()).collect()
    }

    #[test]
    fn kicad_library_footprints_get_their_package_models() {
        // A KiCad 0402 resistor: a chip with a black top and tinned ends, not a plain box.
        let r = fp("Resistor_SMD:R_0402_1005Metric", "resistor", vec![([-0.51, 0.0], [0.54, 0.64]), ([0.51, 0.0], [0.54, 0.64])], vec![block([0.0, 0.0], [1.0, 0.5], [0.0, 0.35], "Epoxy")]);
        let m = mats(&local(&r, &part("10k"), 1.6));
        assert!(m.contains(&"res_top".to_string()) && m.iter().filter(|x| *x == "tin").count() == 2, "{m:?}");
        // A KiCad SOT-23-6 has gull-wing leads.
        let pads: Vec<([f64; 2], [f64; 2])> = [-1.0, 1.0].iter().flat_map(|s| [0.95, 0.0, -0.95].map(|y| ([s * 1.14, y], [1.325, 0.6]))).collect();
        let sot = fp("Package_TO_SOT_SMD:SOT-23-6", "ic", pads, vec![block([0.0, 0.0], [1.6, 2.9], [0.0, 1.2], "Epoxy")]);
        let m = mats(&local(&sot, &part("USBLC6"), 1.6));
        assert!(m.iter().filter(|x| *x == "tin").count() >= 12, "{m:?}");
        // A KiCad QFN/SON/LGA package shows its terminations.
        let q = fp("Package_SON:Texas_DRC0010J_ThermalVias", "ic", vec![([-1.4, 1.0], [0.6, 0.24]), ([1.4, 1.0], [0.6, 0.24])], vec![block([0.0, 0.0], [3.0, 3.0], [0.0, 1.0], "Epoxy")]);
        assert!(mats(&local(&q, &part("TPS63001"), 1.6)).contains(&"tin".to_string()));
        // A KiCad USB-C receptacle is the steel shell with its mouth.
        let u = fp("Connector_USB:USB_C_Receptacle_HRO_TYPE-C-31-M-12", "connector", vec![([0.0, 4.0], [0.3, 1.45])], vec![block([0.0, 0.0], [8.94, 7.3], [0.0, 3.16], "Steel")]);
        let s = local(&u, &part("USB-C"), 1.6);
        assert!(s.iter().any(|x| x.shape == "hcyl") && mats(&s).contains(&"dark".to_string()));
        // An SMD tactile switch: base, cover, plunger and legs; an RGB LED is a clear lens.
        let sw = fp("Button_Switch_SMD:SW_SPST_PTS810", "switch", vec![([-2.08, 1.07], [1.05, 0.65])], vec![block([0.0, 0.0], [4.2, 3.2], [0.0, 2.0], "Steel")]);
        let s = local(&sw, &part("BOOT"), 1.6);
        assert!(s.iter().any(|x| x.shape == "cyl") && mats(&s).contains(&"tin".to_string()) && mats(&s).contains(&"steel".to_string()));
        let led = fp("LED_SMD:LED_WS2812B-2020_PLCC4_2.0x2.0mm", "led", vec![], vec![block([0.0, 0.0], [2.0, 2.0], [0.0, 0.84], "LED")]);
        assert_eq!(mats(&local(&led, &part("WS2812B-2020"), 1.6)), vec!["led_white".to_string()]);
        // Built-in names keep their models.
        let b = fp("R_0603", "resistor", vec![([-0.8, 0.0], [0.9, 0.95]), ([0.8, 0.0], [0.9, 0.95])], vec![block([0.0, 0.0], [1.6, 0.8], [0.0, 0.45], "Epoxy")]);
        assert!(mats(&local(&b, &part("1k"), 1.6)).contains(&"res_top".to_string()));
    }
}
