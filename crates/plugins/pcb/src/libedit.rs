//! Symbol editing tools: pin graphic styles drawn on the sheet, derived
//! symbols flattened onto their parent, the De Morgan body-style swap, a box
//! symbol from a pin table, and the symbol checker (KiCad's "Check symbol"
//! behaviour, written here from its documented checks: duplicate pins, pins off
//! the 50 mil grid, stacked pins that are not one signal, hidden power inputs,
//! pins pointing into the body, units without pins, numbering gaps, and every
//! pin landing on a pad of the footprint).

use crate::footprints::{Footprint, PadKind};
use crate::schematic_view::{Draw, P, W_BODY};
use crate::symbols::{Dir, Gfx, PinShape, PinType, SymbolDef};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// How much of a pin's length a style takes at the body end (the bubble's diameter).
pub fn pin_cut(shape: PinShape) -> f64 {
    match shape {
        PinShape::Inverted | PinShape::InvertedClock => 1.27,
        _ => 0.0,
    }
}

/// What a pin style adds on the sheet: `inner` is the pin's body end, `v`
/// the unit vector from the body out along the pin.
pub fn pin_decoration(shape: PinShape, at: P, inner: P, v: P) -> Vec<Draw> {
    let n = [-v[1], v[0]];
    let pt = |along: f64, across: f64| [inner[0] + v[0] * along + n[0] * across, inner[1] + v[1] * along + n[1] * across];
    let line = |p: Vec<P>| Draw::Line { p, w: W_BODY, ink: "body" };
    let bubble = || Draw::Circle { c: pt(0.635, 0.0), r: 0.635, w: W_BODY, ink: "body", fill: "none" };
    let clock_in = || line(vec![pt(0.0, 0.635), pt(-1.016, 0.0), pt(0.0, -0.635)]);
    let low = || line(vec![pt(0.0, 0.0), pt(0.0, 1.016), pt(1.27, 0.0)]);
    match shape {
        PinShape::Line => vec![],
        PinShape::Inverted => vec![bubble()],
        PinShape::Clock => vec![clock_in()],
        PinShape::InvertedClock => vec![bubble(), clock_in()],
        PinShape::InputLow => vec![low()],
        PinShape::ClockLow => vec![low(), clock_in()],
        PinShape::OutputLow => vec![line(vec![pt(0.0, 1.016), pt(1.27, 0.0)])],
        PinShape::EdgeClockHigh => vec![line(vec![pt(0.0, 0.635), pt(1.016, 0.0), pt(0.0, -0.635)])],
        PinShape::NonLogic => {
            let m = [(at[0] + inner[0]) / 2.0, (at[1] + inner[1]) / 2.0];
            let d = 0.5;
            vec![line(vec![[m[0] - d, m[1] - d], [m[0] + d, m[1] + d]]), line(vec![[m[0] - d, m[1] + d], [m[0] + d, m[1] - d]])]
        }
    }
}

/// A derived symbol on its parent: the parent's pins and drawing (both body
/// styles) with the child's name, fields and defaults.
pub fn flatten(child: &SymbolDef, parent: &SymbolDef) -> SymbolDef {
    let mut out = parent.clone();
    out.name = child.name.clone();
    let keep = |c: &str, p: &str| if c.trim().is_empty() { p.to_string() } else { c.to_string() };
    out.description = keep(&child.description, &parent.description);
    out.prefix = keep(&child.prefix, &parent.prefix);
    out.category = keep(&child.category, &parent.category);
    out.value = keep(&child.value, &child.name);
    out.footprint = keep(&child.footprint, &parent.footprint);
    if !child.footprints.is_empty() {
        out.footprints = child.footprints.clone();
    }
    out.mpn = child.mpn.clone().or(parent.mpn.clone());
    out.manufacturer = child.manufacturer.clone().or(parent.manufacturer.clone());
    out.lcsc = child.lcsc.clone().or(parent.lcsc.clone());
    if !child.keywords.is_empty() {
        out.keywords = child.keywords.clone();
    }
    for (k, v) in &child.fields {
        out.fields.insert(k.clone(), v.clone());
    }
    out
}

/// The same part drawn in its other body style (units ↔ demorgan, pin shapes swapped).
pub fn swap_body_style(def: &SymbolDef) -> Option<SymbolDef> {
    if def.demorgan.is_empty() {
        return None;
    }
    let mut out = def.clone();
    std::mem::swap(&mut out.units, &mut out.demorgan);
    let mut shapes = BTreeMap::new();
    for p in &mut out.pins {
        let other = def.demorgan_pins.get(&p.number).copied().unwrap_or(PinShape::Line);
        shapes.insert(p.number.clone(), p.shape);
        p.shape = other;
    }
    out.demorgan_pins = shapes;
    out.body_style = if def.body_style <= 1 { 2 } else { 1 };
    Some(out)
}

/// One finding of a library check.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Finding {
    /// error, warning or note.
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
    /// Pins or pads it is about.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<P>,
}

pub fn finding(severity: &'static str, code: &'static str, message: impl Into<String>, items: Vec<String>, at: Option<P>) -> Finding {
    Finding { severity, code, message: message.into(), items, at }
}

fn on_grid(v: f64) -> bool {
    ((v / 1.27).round() * 1.27 - v).abs() < 1e-4
}

/// Graphics bounding box of a unit (pins left out), or None when it draws nothing.
fn gfx_box(gfx: &[Gfx]) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut add = |p: [f64; 2]| {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    };
    for g in gfx {
        match g {
            Gfx::Line { pts, .. } | Gfx::Poly { pts, .. } => pts.iter().for_each(|p| add(*p)),
            Gfx::Rect { a, b, .. } => {
                add(*a);
                add(*b);
            }
            Gfx::Circle { c, r, .. } | Gfx::Arc { c, r, .. } => {
                add([c[0] - r, c[1] - r]);
                add([c[0] + r, c[1] + r]);
            }
            Gfx::Text { .. } => {}
        }
    }
    b[0].is_finite().then_some(b)
}

/// Natural sort of pin numbers ("2" before "10", "A2" before "A10").
fn natural_numbers(v: &mut [String]) {
    v.sort_by(|a, b| crate::natural(a, b));
}

/// The symbol checker. `footprint`, when known, is the default footprint the pins must land on.
pub fn check_symbol(def: &SymbolDef, footprint: Option<&Footprint>) -> Vec<Finding> {
    let mut out = Vec::new();
    if def.name.trim().is_empty() {
        out.push(finding("error", "no_name", "the symbol needs a name", vec![], None));
    }
    if def.prefix.trim().is_empty() && !def.virtual_part {
        out.push(finding("error", "no_prefix", "the symbol needs a reference prefix (R, C, U, J…)", vec![], None));
    }
    if def.pins.is_empty() && !def.virtual_part {
        out.push(finding("error", "no_pins", "the symbol has no pins", vec![], None));
        return out;
    }
    let units = def.unit_count();
    // Duplicate numbers: one pin number is one signal (stacking several pads on a pin uses `pads`).
    let mut by_number: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, p) in def.pins.iter().enumerate() {
        if p.number.trim().is_empty() {
            out.push(finding("error", "unnumbered_pin", format!("a pin named “{}” has no number", p.name), vec![p.name.clone()], Some(p.at)));
            continue;
        }
        by_number.entry(p.number.as_str()).or_default().push(i);
    }
    for (n, list) in &by_number {
        if list.len() > 1 {
            let units: Vec<String> = list.iter().map(|i| crate::schematic_view::unit_letter(def.pins[*i].unit.max(1))).collect();
            out.push(finding("error", "duplicate_pin", format!("pin {n} appears {} times (units {}): a number is one signal; give the extra pads in its `pads`", list.len(), units.join(", ")), vec![n.to_string()], Some(def.pins[list[0]].at)));
        }
    }
    for u in 1..=units {
        let pins: Vec<&crate::symbols::PinDef> = def.unit_pins(u).collect();
        if pins.is_empty() {
            out.push(finding("warning", "empty_unit", format!("unit {} has no pins", crate::schematic_view::unit_letter(u)), vec![], None));
        }
        // Stacked pins: same connection point in one unit.
        for (i, a) in pins.iter().enumerate() {
            for b in pins.iter().skip(i + 1) {
                if !a.hidden && !b.hidden && (a.at[0] - b.at[0]).abs() < 1e-6 && (a.at[1] - b.at[1]).abs() < 1e-6 {
                    let same_signal = a.name == b.name && a.name != "~" && !a.name.is_empty();
                    if !same_signal {
                        out.push(finding("error", "overlapping_pins", format!("pins {} ({}) and {} ({}) sit on one point: they would join; stack only pins of one signal", a.number, a.name, b.number, b.name), vec![a.number.clone(), b.number.clone()], Some(a.at)));
                    }
                }
            }
        }
        let gfx = def.units.get(u as usize - 1).map(|v| v.as_slice()).unwrap_or(&[]);
        if let Some(bx) = gfx_box(gfx) {
            for p in &pins {
                if p.hidden || p.length < 1e-6 {
                    continue;
                }
                let inside = p.at[0] > bx[0] + 1e-6 && p.at[0] < bx[2] - 1e-6 && p.at[1] > bx[1] + 1e-6 && p.at[1] < bx[3] - 1e-6;
                if inside {
                    out.push(finding("warning", "pin_inside_body", format!("pin {}'s connection end is inside the body: it points the wrong way (its side is the direction it leaves the body)", p.number), vec![p.number.clone()], Some(p.at)));
                }
            }
        }
    }
    for p in &def.pins {
        if p.hidden {
            if matches!(p.etype, PinType::PowerIn) {
                out.push(finding("warning", "hidden_power_pin", format!("pin {} ({}) is a hidden power input: ISE Studio joins no net to hidden pins, so it stays unconnected; show it and wire it", p.number, p.name), vec![p.number.clone()], Some(p.at)));
            }
            continue;
        }
        if !on_grid(p.at[0]) || !on_grid(p.at[1]) {
            out.push(finding("error", "off_grid", format!("pin {} at ({:.3}, {:.3}) is off the 1.27 mm (50 mil) grid: wires will not meet it", p.number, p.at[0], p.at[1]), vec![p.number.clone()], Some(p.at)));
        }
        if p.length < 1e-6 && !def.virtual_part {
            out.push(finding("warning", "zero_length", format!("pin {} has no length", p.number), vec![p.number.clone()], Some(p.at)));
        }
        if p.unit > units {
            out.push(finding("error", "bad_unit", format!("pin {} is in unit {} but the symbol has {units}", p.number, p.unit), vec![p.number.clone()], Some(p.at)));
        }
    }
    if !def.demorgan.is_empty() && def.demorgan.len() != def.units.len() {
        out.push(finding("error", "demorgan_units", format!("{} units but {} De Morgan bodies", def.units.len(), def.demorgan.len()), vec![], None));
    }
    // Numbering: gaps in a run of plain numbers (1…N).
    let nums: BTreeSet<u32> = def.pins.iter().filter_map(|p| p.number.parse().ok()).collect();
    if nums.len() >= 3 && nums.len() * 2 >= def.pins.len() {
        let max = *nums.iter().max().unwrap();
        let missing: Vec<String> = (1..=max).filter(|n| !nums.contains(n)).map(|n| n.to_string()).collect();
        if !missing.is_empty() && missing.len() <= 32 {
            out.push(finding("note", "numbering_gap", format!("pins {} are not on the symbol (1…{max}): missing, or left out on purpose (not connected)?", missing.join(", ")), missing, None));
        }
    }
    if let Some(fp) = footprint {
        let pads: BTreeSet<&str> = fp.pads.iter().filter(|p| p.kind != PadKind::Npth && !p.number.is_empty()).map(|p| p.number.as_str()).collect();
        let mut reached: BTreeSet<String> = BTreeSet::new();
        for p in &def.pins {
            for pad in p.pads() {
                if !pads.contains(pad.as_str()) {
                    out.push(finding("error", "pin_without_pad", format!("pin {} lands on pad {pad}, which {} does not have", p.number, fp.name), vec![p.number.clone()], Some(p.at)));
                }
                reached.insert(pad);
            }
        }
        let mut free: Vec<String> = pads.iter().filter(|n| !reached.contains(**n)).map(|n| n.to_string()).collect();
        natural_numbers(&mut free);
        if !free.is_empty() {
            out.push(finding("warning", "pad_without_pin", format!("pads {} of {} have no pin: they will have no net", free.join(", "), fp.name), free, None));
        }
    }
    out
}

/// A pin's side from where it sits on a box (for box symbols drawn by hand).
pub fn side_for(at: P, body: [f64; 4]) -> Dir {
    let d = [(at[0] - body[0]).abs(), (body[2] - at[0]).abs(), (at[1] - body[1]).abs(), (body[3] - at[1]).abs()];
    let m = d.iter().cloned().fold(f64::INFINITY, f64::min);
    if m == d[0] {
        Dir::Left
    } else if m == d[1] {
        Dir::Right
    } else if m == d[2] {
        Dir::Up
    } else {
        Dir::Down
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols;

    fn codes(f: &[Finding]) -> Vec<&'static str> {
        f.iter().map(|x| x.code).collect()
    }

    #[test]
    fn built_in_symbols_pass_the_checker() {
        for n in symbols::names() {
            let d = symbols::get(&n).unwrap();
            let fp = crate::footprints::get(&d.footprint);
            let errs: Vec<Finding> = check_symbol(&d, fp.as_ref()).into_iter().filter(|f| f.severity == "error").collect();
            assert!(errs.is_empty(), "{n}: {errs:?}");
        }
    }

    #[test]
    fn checker_finds_duplicates_off_grid_and_overlaps() {
        let mut d = symbols::get("OpAmp").unwrap();
        d.pins[1].number = "3".into();
        d.pins[2].at = [7.5, 0.1];
        d.pins[4].at = d.pins[3].at;
        let c = codes(&check_symbol(&d, None));
        assert!(c.contains(&"duplicate_pin"), "{c:?}");
        assert!(c.contains(&"off_grid"), "{c:?}");
        assert!(c.contains(&"overlapping_pins"), "{c:?}");
    }

    #[test]
    fn checker_maps_pins_to_pads() {
        let d = symbols::get("OpAmp").unwrap();
        let fp = crate::footprints::get("SOIC-8").unwrap();
        let f = check_symbol(&d, Some(&fp));
        let free = f.iter().find(|x| x.code == "pad_without_pin").expect("SOIC-8 has 3 spare pads");
        assert_eq!(free.items, vec!["6", "7", "8"]);
        let fp3 = crate::footprints::get("SOT-23").unwrap();
        assert!(check_symbol(&d, Some(&fp3)).iter().any(|x| x.code == "pin_without_pad"));
    }

    #[test]
    fn hidden_power_and_inward_pins_are_flagged() {
        let mut d = symbols::get("R").unwrap();
        d.pins[0].hidden = true;
        d.pins[0].etype = PinType::PowerIn;
        d.pins[1].at = [0.0, 0.0];
        let c = codes(&check_symbol(&d, None));
        assert!(c.contains(&"hidden_power_pin"));
        assert!(c.contains(&"pin_inside_body"));
    }

    #[test]
    fn derived_symbols_take_the_parents_body_and_their_own_fields() {
        let parent = symbols::get("LDO_SOT23-5").unwrap();
        let mut child = parent.clone();
        child.name = "XC6220B331".into();
        child.value = String::new();
        child.pins.clear();
        child.units = vec![];
        child.mpn = Some("XC6220B331MR-G".into());
        let f = flatten(&child, &parent);
        assert_eq!(f.name, "XC6220B331");
        assert_eq!(f.value, "XC6220B331");
        assert_eq!(f.pins, parent.pins);
        assert_eq!(f.mpn.as_deref(), Some("XC6220B331MR-G"));
    }

    #[test]
    fn body_style_swaps_and_back() {
        let mut d = symbols::get("R").unwrap();
        d.demorgan = vec![vec![Gfx::Circle { c: [0.0, 0.0], r: 2.0, fill: crate::symbols::Fill::None }]];
        d.demorgan_pins.insert("1".into(), PinShape::Inverted);
        let s = swap_body_style(&d).unwrap();
        assert_eq!(s.body_style, 2);
        assert_eq!(s.pins[0].shape, PinShape::Inverted);
        let back = swap_body_style(&s).unwrap();
        assert_eq!(back.units, d.units);
        assert_eq!(back.pins[0].shape, PinShape::Line);
        assert_eq!(back.body_style, 1);
    }
}
