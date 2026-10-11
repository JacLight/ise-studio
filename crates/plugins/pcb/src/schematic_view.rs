//! A schematic resolved: every item drawn in sheet coordinates (the web
//! editor and the PDF draw exactly these primitives), connectivity worked out
//! into named nets, each pin's net and footprint pads, and the
//! electrical-rules check with every violation tied to its objects.

use crate::schematic::{self, LabelKind, SchItems, Schematic, is_ground, same};
use crate::symbols::{self, Dir, Fill, Gfx, PinType, text_width};
use crate::{entity, natural};
use de_core::{StableId, bevy_ecs::world::World};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type P = [f64; 2];

/// One drawing primitive on a sheet. `ink` names a colour role (body, pin,
/// pin_number, pin_name, reference, value, wire, junction, label, global,
/// hierarchical, power, no_connect, note, frame, frame_text, sheet); `fill`
/// is none, ink or body.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Draw {
    Line { p: Vec<P>, w: f64, ink: &'static str },
    Poly { p: Vec<P>, w: f64, ink: &'static str, fill: &'static str },
    Circle { c: P, r: f64, w: f64, ink: &'static str, fill: &'static str },
    /// `at` is on the text's middle line; `angle` 0 reads left to right, 90 bottom to top; `anchor` start, middle or end.
    Text { at: P, s: String, size: f64, angle: f64, anchor: &'static str, ink: &'static str },
}

#[derive(Clone, Debug, Serialize)]
pub struct PinView {
    pub number: String,
    pub name: String,
    pub etype: PinType,
    pub at: P,
    pub dir: Dir,
    pub hidden: bool,
    pub net: Option<String>,
    pub pads: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SymbolView {
    pub id: StableId,
    pub sheet: String,
    pub symbol: String,
    pub reference: String,
    pub value: String,
    pub footprint: String,
    pub unit: u32,
    pub units: u32,
    pub at: P,
    pub rotation: f64,
    pub mirror: bool,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    pub description: String,
    pub dnp: bool,
    pub virtual_part: bool,
    pub exclude_from_board: bool,
    pub component: Option<StableId>,
    pub known: bool,
    pub category: String,
    /// Body extents, and everything (pins, texts) [x0, y0, x1, y1].
    pub body: [f64; 4],
    pub bbox: [f64; 4],
    pub draw: Vec<Draw>,
    pub pins: Vec<PinView>,
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WireView {
    pub id: StableId,
    pub sheet: String,
    pub points: Vec<P>,
    pub net: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LabelView {
    pub id: StableId,
    pub sheet: String,
    pub kind: LabelKind,
    pub text: String,
    pub at: P,
    pub rotation: f64,
    pub net: Option<String>,
    pub draw: Vec<Draw>,
    pub bbox: [f64; 4],
}

#[derive(Clone, Debug, Serialize)]
pub struct JunctionView {
    pub id: Option<StableId>,
    pub sheet: String,
    pub at: P,
}

#[derive(Clone, Debug, Serialize)]
pub struct MarkView {
    pub id: StableId,
    pub sheet: String,
    pub at: P,
}

#[derive(Clone, Debug, Serialize)]
pub struct TextView {
    pub id: StableId,
    pub sheet: String,
    pub at: P,
    pub text: String,
    pub size: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SheetBoxView {
    pub sheet: String,
    pub name: String,
    pub at: P,
    pub size: P,
    pub pins: Vec<(String, P)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SheetView {
    pub id: String,
    pub name: String,
    pub paper: String,
    pub size: P,
    pub parent: Option<String>,
    pub frame: Vec<Draw>,
    pub boxes: Vec<SheetBoxView>,
    pub boxes_draw: Vec<Draw>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SchNet {
    pub name: String,
    /// Symbol pins on it ("U1.3", "J1.A4").
    pub pins: Vec<String>,
    /// Footprint pads it reaches ("U1.1", "U1.15", "J1.B9").
    pub pads: Vec<String>,
    pub labels: Vec<String>,
    pub sheets: Vec<String>,
    /// Named by a label or power port (not `Net-(…)`).
    pub named: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ErcViolation {
    pub code: &'static str,
    pub severity: &'static str,
    pub message: String,
    pub sheet: String,
    pub at: P,
    pub objects: Vec<StableId>,
    pub refs: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SchStats {
    pub sheets: usize,
    pub symbols: usize,
    pub parts: usize,
    pub wires: usize,
    pub labels: usize,
    pub nets: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SchematicView {
    pub id: StableId,
    pub name: String,
    pub board: Option<StableId>,
    pub title: schematic::TitleBlock,
    pub board_only: Vec<String>,
    pub sheets: Vec<SheetView>,
    pub symbols: Vec<SymbolView>,
    pub wires: Vec<WireView>,
    pub labels: Vec<LabelView>,
    pub junctions: Vec<JunctionView>,
    pub no_connects: Vec<MarkView>,
    pub texts: Vec<TextView>,
    pub nets: Vec<SchNet>,
    pub violations: Vec<ErcViolation>,
    pub stats: SchStats,
}

impl SchematicView {
    pub fn errors(&self) -> usize {
        self.violations.iter().filter(|v| v.severity == "error").count()
    }
}

// ---- Drawing -------------------------------------------------------------------

pub const W_BODY: f64 = 0.254;
pub const W_PIN: f64 = 0.1524;
pub const W_WIRE: f64 = 0.1778;
pub const FIELD: f64 = 1.6;
pub const PIN_NUM: f64 = 1.15;
pub const LABEL: f64 = 1.5;

fn fill_name(f: Fill) -> &'static str {
    match f {
        Fill::None => "none",
        Fill::Outline => "ink",
        Fill::Body => "body",
    }
}

fn bbox_add(b: &mut [f64; 4], p: P) {
    b[0] = b[0].min(p[0]);
    b[1] = b[1].min(p[1]);
    b[2] = b[2].max(p[0]);
    b[3] = b[3].max(p[1]);
}

const EMPTY: [f64; 4] = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];

/// Extents of a text primitive.
pub fn text_box(at: P, s: &str, size: f64, angle: f64, anchor: &str) -> [f64; 4] {
    let w = text_width(s, size);
    let h = size * 0.75;
    let (a, b) = match anchor {
        "start" => (0.0, w),
        "end" => (-w, 0.0),
        _ => (-w / 2.0, w / 2.0),
    };
    if angle.abs() < 1.0 { [at[0] + a, at[1] - h / 2.0, at[0] + b, at[1] + h / 2.0] } else { [at[0] - h / 2.0, at[1] - b, at[0] + h / 2.0, at[1] - a] }
}

pub fn draw_box(d: &Draw) -> [f64; 4] {
    let mut b = EMPTY;
    match d {
        Draw::Line { p, .. } | Draw::Poly { p, .. } => p.iter().for_each(|q| bbox_add(&mut b, *q)),
        Draw::Circle { c, r, .. } => {
            bbox_add(&mut b, [c[0] - r, c[1] - r]);
            bbox_add(&mut b, [c[0] + r, c[1] + r]);
        }
        Draw::Text { at, s, size, angle, anchor, .. } => {
            let t = text_box(*at, s, *size, *angle, anchor);
            bbox_add(&mut b, [t[0], t[1]]);
            bbox_add(&mut b, [t[2], t[3]]);
        }
    }
    b
}

fn arc_points(c: P, r: f64, start: f64, end: f64) -> Vec<P> {
    let mut e = end;
    while e < start {
        e += 360.0;
    }
    let n = (((e - start) / 15.0).ceil() as usize).max(2);
    (0..=n)
        .map(|i| {
            let a = (start + (e - start) * i as f64 / n as f64).to_radians();
            [c[0] + r * a.cos(), c[1] - r * a.sin()]
        })
        .collect()
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn seg_box(a: P, b: P, pad: f64) -> [f64; 4] {
    [a[0].min(b[0]) - pad, a[1].min(b[1]) - pad, a[0].max(b[0]) + pad, a[1].max(b[1]) + pad]
}

pub(crate) fn symbol_view(id: StableId, s: &schematic::SchSymbol) -> SymbolView {
    let def = s.def();
    let mut draw = Vec::new();
    let mut pins = Vec::new();
    let mut body = EMPTY;
    let known = def.is_some();
    let (units, category, virtual_part) = def.as_ref().map(|d| (d.unit_count(), d.category.clone(), d.virtual_part)).unwrap_or((1, "IC".into(), false));
    if let Some(d) = &def {
        let gfx = d.units.get(s.unit.saturating_sub(1) as usize).or(d.units.first()).cloned().unwrap_or_default();
        // Fills first, so outlines and pins draw over them.
        let mut fills = Vec::new();
        let mut lines = Vec::new();
        for g in &gfx {
            match g {
                Gfx::Line { pts, width } => lines.push(Draw::Line { p: pts.iter().map(|p| s.place(*p)).collect(), w: if *width > 0.0 { *width } else { W_BODY }, ink: "body" }),
                Gfx::Poly { pts, fill } => {
                    let p: Vec<P> = pts.iter().map(|q| s.place(*q)).collect();
                    let d = Draw::Poly { p, w: W_BODY, ink: "body", fill: fill_name(*fill) };
                    if *fill == Fill::Body { fills.push(d) } else { lines.push(d) }
                }
                Gfx::Rect { a, b, fill } => {
                    let p = vec![s.place(*a), s.place([b[0], a[1]]), s.place(*b), s.place([a[0], b[1]])];
                    let d = Draw::Poly { p, w: W_BODY, ink: "body", fill: fill_name(*fill) };
                    if *fill == Fill::Body { fills.push(d) } else { lines.push(d) }
                }
                Gfx::Circle { c, r, fill } => lines.push(Draw::Circle { c: s.place(*c), r: *r, w: W_BODY, ink: "body", fill: fill_name(*fill) }),
                Gfx::Arc { c, r, start, end } => lines.push(Draw::Line { p: arc_points(*c, *r, *start, *end).into_iter().map(|p| s.place(p)).collect(), w: W_BODY, ink: "body" }),
                Gfx::Text { at, text, size } => lines.push(Draw::Text { at: s.place(*at), s: text.clone(), size: *size, angle: 0.0, anchor: "middle", ink: "body" }),
            }
        }
        for d in fills.iter().chain(lines.iter()) {
            let b = draw_box(d);
            bbox_add(&mut body, [b[0], b[1]]);
            bbox_add(&mut body, [b[2], b[3]]);
        }
        draw.extend(fills);
        draw.extend(lines);
        for p in d.unit_pins(s.unit) {
            let at = s.place(p.at);
            let dir = s.dir(p.side);
            let pads = s.pads_of(p);
            pins.push(PinView { number: p.number.clone(), name: p.name.clone(), etype: p.etype, at, dir, hidden: p.hidden, net: None, pads: pads.clone() });
            if p.hidden {
                continue;
            }
            let v = dir.v();
            let inner = [at[0] - v[0] * p.length, at[1] - v[1] * p.length];
            if p.length > 1e-6 {
                draw.push(Draw::Line { p: vec![at, inner], w: W_PIN, ink: "pin" });
            }
            let mid = [(at[0] + inner[0]) / 2.0, (at[1] + inner[1]) / 2.0];
            let horizontal = matches!(dir, Dir::Left | Dir::Right);
            if d.show_pin_numbers && p.length > 1e-6 {
                let shown = if pads.len() > 1 && pads.len() <= 4 && !pads.iter().any(|x| x == &p.number) { pads.join(",") } else { p.number.clone() };
                if horizontal {
                    draw.push(Draw::Text { at: [mid[0], mid[1] - 0.75], s: shown, size: PIN_NUM, angle: 0.0, anchor: "middle", ink: "pin_number" });
                } else {
                    draw.push(Draw::Text { at: [mid[0] - 0.75, mid[1]], s: shown, size: PIN_NUM, angle: 90.0, anchor: "middle", ink: "pin_number" });
                }
            }
            if d.show_pin_names && !p.name.is_empty() && p.name != "~" {
                let g = 0.65;
                let (pos, angle, anchor) = match dir {
                    Dir::Left => ([inner[0] + g, inner[1]], 0.0, "start"),
                    Dir::Right => ([inner[0] - g, inner[1]], 0.0, "end"),
                    Dir::Up => ([inner[0], inner[1] + g], 90.0, "end"),
                    Dir::Down => ([inner[0], inner[1] - g], 90.0, "start"),
                };
                draw.push(Draw::Text { at: pos, s: p.name.clone(), size: symbols::PIN_NAME_SIZE, angle, anchor, ink: "pin_name" });
            }
        }
        if !body[0].is_finite() {
            body = [s.at[0] - 1.27, s.at[1] - 1.27, s.at[0] + 1.27, s.at[1] + 1.27];
        }
    } else {
        // Unknown symbol: a dashed-looking placeholder box so it is still visible and selectable.
        body = [s.at[0] - 5.08, s.at[1] - 2.54, s.at[0] + 5.08, s.at[1] + 2.54];
        draw.push(Draw::Poly { p: vec![[body[0], body[1]], [body[2], body[1]], [body[2], body[3]], [body[0], body[3]]], w: W_BODY, ink: "error", fill: "none" });
        draw.push(Draw::Text { at: s.at, s: format!("? {}", s.symbol), size: 1.27, angle: 0.0, anchor: "middle", ink: "error" });
    }
    // Reference and value: where set by hand, else the first clear spot.
    let reference = if units > 1 { format!("{}{}", s.reference, unit_letter(s.unit)) } else { s.reference.clone() };
    let value = if virtual_part && s.value == "PWR_FLAG" { String::new() } else { s.value.clone() };
    let pin_boxes: Vec<[f64; 4]> = pins
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| {
            let v = p.dir.v();
            seg_box(p.at, [p.at[0] - v[0] * 2.54, p.at[1] - v[1] * 2.54], 0.5)
        })
        .collect();
    let (rp, vp, anchor) = field_spots(&body, &pin_boxes, &reference, &value, &category, &pins);
    let ref_at = s.ref_at.unwrap_or(rp);
    let value_at = s.value_at.unwrap_or(vp);
    let anchor_r = if s.ref_at.is_some() { "start" } else { anchor };
    let anchor_v = if s.value_at.is_some() { "start" } else { anchor };
    if !s.reference.starts_with('#') {
        draw.push(Draw::Text { at: ref_at, s: reference.clone(), size: FIELD, angle: 0.0, anchor: anchor_r, ink: "reference" });
    }
    if !value.is_empty() {
        draw.push(Draw::Text { at: value_at, s: value, size: FIELD, angle: 0.0, anchor: anchor_v, ink: "value" });
    }
    if s.dnp {
        draw.push(Draw::Line { p: vec![[body[0], body[1]], [body[2], body[3]]], w: 0.3, ink: "dnp" });
        draw.push(Draw::Line { p: vec![[body[0], body[3]], [body[2], body[1]]], w: 0.3, ink: "dnp" });
    }
    let mut bbox = EMPTY;
    for d in &draw {
        let b = draw_box(d);
        bbox_add(&mut bbox, [b[0], b[1]]);
        bbox_add(&mut bbox, [b[2], b[3]]);
    }
    for p in &pins {
        if !p.hidden {
            bbox_add(&mut bbox, p.at);
        }
    }
    SymbolView {
        id,
        sheet: s.sheet.clone(),
        symbol: s.symbol.clone(),
        reference: s.reference.clone(),
        value: s.value.clone(),
        footprint: s.footprint.clone(),
        unit: s.unit,
        units,
        at: s.at,
        rotation: s.rotation,
        mirror: s.mirror,
        mpn: s.mpn.clone(),
        manufacturer: s.manufacturer.clone(),
        lcsc: s.lcsc.clone(),
        description: s.description.clone(),
        dnp: s.dnp,
        virtual_part,
        exclude_from_board: s.exclude_from_board,
        component: s.component,
        known,
        category,
        body,
        bbox,
        draw,
        pins,
        fields: s.fields.clone(),
    }
}

pub fn unit_letter(u: u32) -> String {
    let mut n = u.max(1);
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

/// Reference and value positions: the first candidate whose texts clear the pins.
fn field_spots(body: &[f64; 4], pins: &[[f64; 4]], r: &str, v: &str, category: &str, pin_views: &[PinView]) -> (P, P, &'static str) {
    let cx = (body[0] + body[2]) / 2.0;
    let cy = (body[1] + body[3]) / 2.0;
    let gap = 1.0;
    let boxy = matches!(category, "IC" | "Module" | "Connector" | "Power IC") && pin_views.len() >= 3;
    let mut cands: Vec<(P, P, &'static str)> = Vec::new();
    let right = ([body[2] + gap, cy - 1.0], [body[2] + gap, cy + 1.0], "start");
    let left = ([body[0] - gap, cy - 1.0], [body[0] - gap, cy + 1.0], "end");
    let above_below = ([cx, body[1] - 1.3], [cx, body[3] + 1.3], "middle");
    let top_left = ([body[0], body[1] - 1.2], [body[0], body[3] + 1.4], "start");
    let top_stack = ([body[0], body[1] - 3.2], [body[0], body[1] - 1.2], "start");
    let bottom_stack = ([body[0], body[3] + 1.4], [body[0], body[3] + 3.4], "start");
    if boxy {
        cands.extend([top_left, top_stack, bottom_stack, right]);
    } else if pin_views.iter().filter(|p| !p.hidden).all(|p| matches!(p.dir, Dir::Up | Dir::Down)) {
        cands.extend([right, left, above_below]);
    } else {
        cands.extend([above_below, right, left, top_stack]);
    }
    for c in &cands {
        let rb = text_box(c.0, r, FIELD, 0.0, c.2);
        let vb = text_box(c.1, v, FIELD, 0.0, c.2);
        if !pins.iter().any(|p| overlaps(*p, rb) || (!v.is_empty() && overlaps(*p, vb))) {
            return *c;
        }
    }
    cands[0]
}

pub(crate) fn label_view(id: StableId, l: &schematic::SchLabel) -> LabelView {
    let mut draw = Vec::new();
    let a = l.at;
    let rot = l.rotation.rem_euclid(360.0);
    let u: P = match rot.round() as i64 {
        90 => [0.0, -1.0],
        180 => [-1.0, 0.0],
        270 => [0.0, 1.0],
        _ => [1.0, 0.0],
    };
    let n: P = [u[1], -u[0]]; // left of travel (screen)
    let vertical = u[0].abs() < 0.5;
    let at = |d: f64, s: f64| [a[0] + u[0] * d + n[0] * s, a[1] + u[1] * d + n[1] * s];
    match l.kind {
        LabelKind::Local => {
            let (angle, anchor) = if vertical { (90.0, if u[1] < 0.0 { "start" } else { "end" }) } else { (0.0, if u[0] > 0.0 { "start" } else { "end" }) };
            // Text sits just above the wire it names (left of it when vertical).
            let pos = if vertical { [a[0] - 0.85, a[1] + u[1] * 0.3] } else { [a[0] + u[0] * 0.3, a[1] - 0.85] };
            draw.push(Draw::Text { at: pos, s: l.text.clone(), size: LABEL, angle, anchor, ink: "label" });
        }
        LabelKind::Global | LabelKind::Hierarchical => {
            let h = 2.2;
            let w = text_width(&l.text, LABEL) + 1.2;
            let ink = if l.kind == LabelKind::Global { "global" } else { "hierarchical" };
            let tip = h / 2.0;
            let shape = l.shape.as_deref().unwrap_or(if l.kind == LabelKind::Global { "bidirectional" } else { "passive" });
            let mut p = Vec::new();
            match shape {
                "input" => {
                    p.extend([at(0.0, h / 2.0), at(w + tip, h / 2.0), at(w + tip * 2.0, 0.0), at(w + tip, -h / 2.0), at(0.0, -h / 2.0)]);
                }
                "output" => {
                    p.extend([at(0.0, 0.0), at(tip, h / 2.0), at(w + tip * 2.0, h / 2.0), at(w + tip * 2.0, -h / 2.0), at(tip, -h / 2.0)]);
                }
                "passive" => {
                    p.extend([at(0.0, h / 2.0), at(w + tip, h / 2.0), at(w + tip, -h / 2.0), at(0.0, -h / 2.0)]);
                }
                _ => {
                    p.extend([at(0.0, 0.0), at(tip, h / 2.0), at(w + tip, h / 2.0), at(w + tip * 2.0, 0.0), at(w + tip, -h / 2.0), at(tip, -h / 2.0)]);
                }
            }
            draw.push(Draw::Poly { p, w: W_PIN, ink, fill: "none" });
            let mid = at(tip + w / 2.0, 0.0);
            draw.push(Draw::Text { at: mid, s: l.text.clone(), size: LABEL, angle: if vertical { 90.0 } else { 0.0 }, anchor: "middle", ink });
        }
        LabelKind::Power => {
            let style = l.style.clone().unwrap_or_else(|| if is_ground(&l.text) { "ground".into() } else { "bar".into() });
            let stem = 1.27;
            draw.push(Draw::Line { p: vec![a, at(stem, 0.0)], w: W_PIN, ink: "power" });
            let text_d;
            match style.as_str() {
                "ground" => {
                    draw.push(Draw::Line { p: vec![at(stem, 1.27), at(stem, -1.27)], w: W_BODY, ink: "power" });
                    draw.push(Draw::Line { p: vec![at(stem + 0.508, 0.762), at(stem + 0.508, -0.762)], w: W_BODY, ink: "power" });
                    draw.push(Draw::Line { p: vec![at(stem + 1.016, 0.254), at(stem + 1.016, -0.254)], w: W_BODY, ink: "power" });
                    text_d = stem + 1.016 + 1.3;
                }
                "arrow" => {
                    draw.push(Draw::Poly { p: vec![at(stem, 0.762), at(stem + 1.27, 0.0), at(stem, -0.762)], w: W_BODY, ink: "power", fill: "ink" });
                    text_d = stem + 1.27 + 1.2;
                }
                "circle" => {
                    draw.push(Draw::Circle { c: at(stem + 0.508, 0.0), r: 0.508, w: W_BODY, ink: "power", fill: "none" });
                    text_d = stem + 1.016 + 1.2;
                }
                _ => {
                    draw.push(Draw::Line { p: vec![at(stem, 1.016), at(stem, -1.016)], w: W_BODY * 1.4, ink: "power" });
                    text_d = stem + 1.2;
                }
            }
            let (pos, anchor) = if vertical { (at(text_d, 0.0), "middle") } else { (at(text_d - 0.6, 0.0), if u[0] > 0.0 { "start" } else { "end" }) };
            draw.push(Draw::Text { at: pos, s: l.text.clone(), size: LABEL, angle: 0.0, anchor, ink: "power" });
        }
    }
    let mut bbox = EMPTY;
    bbox_add(&mut bbox, a);
    for d in &draw {
        let b = draw_box(d);
        bbox_add(&mut bbox, [b[0], b[1]]);
        bbox_add(&mut bbox, [b[2], b[3]]);
    }
    LabelView { id, sheet: l.sheet.clone(), kind: l.kind, text: l.text.clone(), at: l.at, rotation: rot, net: None, draw, bbox }
}

/// The paper's frame, zone references and title block.
pub fn frame(s: &Schematic, sheet: &schematic::Sheet, index: usize) -> Vec<Draw> {
    let [w, h] = sheet.paper_size();
    let m = 8.0;
    let strip = 4.0;
    let mut d = Vec::new();
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64, wd: f64| Draw::Poly { p: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], w: wd, ink: "frame", fill: "none" };
    d.push(rect(m, m, w - m, h - m, 0.25));
    d.push(rect(m + strip, m + strip, w - m - strip, h - m - strip, 0.18));
    let cols = ((w - 2.0 * m) / 50.0).round().max(1.0) as usize;
    let rows = ((h - 2.0 * m) / 50.0).round().max(1.0) as usize;
    let cw = (w - 2.0 * m) / cols as f64;
    let rh = (h - 2.0 * m) / rows as f64;
    for i in 0..cols {
        let x = m + cw * (i as f64 + 0.5);
        for y in [m + strip / 2.0, h - m - strip / 2.0] {
            d.push(Draw::Text { at: [x, y], s: (i + 1).to_string(), size: 2.0, angle: 0.0, anchor: "middle", ink: "frame_text" });
        }
        if i > 0 {
            let x = m + cw * i as f64;
            d.push(Draw::Line { p: vec![[x, m], [x, m + strip]], w: 0.18, ink: "frame" });
            d.push(Draw::Line { p: vec![[x, h - m - strip], [x, h - m]], w: 0.18, ink: "frame" });
        }
    }
    for j in 0..rows {
        let y = m + rh * (j as f64 + 0.5);
        let letter = ((b'A' + j as u8) as char).to_string();
        for x in [m + strip / 2.0, w - m - strip / 2.0] {
            d.push(Draw::Text { at: [x, y], s: letter.clone(), size: 2.0, angle: 0.0, anchor: "middle", ink: "frame_text" });
        }
        if j > 0 {
            let y = m + rh * j as f64;
            d.push(Draw::Line { p: vec![[m, y], [m + strip, y]], w: 0.18, ink: "frame" });
            d.push(Draw::Line { p: vec![[w - m - strip, y], [w - m, y]], w: 0.18, ink: "frame" });
        }
    }
    // Title block, bottom right of the inner frame.
    let x1 = w - m - strip;
    let y1 = h - m - strip;
    let x0 = x1 - 110.0;
    let y0 = y1 - 32.0;
    d.push(rect(x0, y0, x1, y1, 0.25));
    let line = |a: P, b: P| Draw::Line { p: vec![a, b], w: 0.18, ink: "frame" };
    d.push(line([x0, y0 + 6.0], [x1, y0 + 6.0]));
    d.push(line([x0, y0 + 17.0], [x1, y0 + 17.0]));
    d.push(line([x0, y0 + 24.0], [x1, y0 + 24.0]));
    d.push(line([x0 + 36.0, y0 + 17.0], [x0 + 36.0, y1]));
    d.push(line([x0 + 82.0, y0 + 17.0], [x0 + 82.0, y1]));
    let t = |at: P, s: String, size: f64, anchor: &'static str| Draw::Text { at, s, size, angle: 0.0, anchor, ink: "frame_text" };
    let path = if sheet.parent.is_some() { format!("Sheet: /{}/", sheet.name) } else { format!("Sheet: {}", sheet.name) };
    d.push(t([x0 + 2.0, y0 + 3.0], path, 1.6, "start"));
    d.push(t([x1 - 2.0, y0 + 3.0], format!("{} sheet{}", s.sheets.len(), if s.sheets.len() == 1 { "" } else { "s" }), 1.4, "end"));
    d.push(Draw::Text { at: [x0 + 2.0, y0 + 9.0], s: "Title".into(), size: 1.3, angle: 0.0, anchor: "start", ink: "frame_label" });
    d.push(t([x0 + 2.0, y0 + 13.2], if s.title.title.is_empty() { s.name.clone() } else { s.title.title.clone() }, 3.4, "start"));
    d.push(Draw::Text { at: [x0 + 2.0, y0 + 19.0], s: "Size".into(), size: 1.3, angle: 0.0, anchor: "start", ink: "frame_label" });
    d.push(t([x0 + 2.0, y0 + 21.8], sheet.paper.clone(), 1.8, "start"));
    d.push(Draw::Text { at: [x0 + 14.0, y0 + 19.0], s: "Date".into(), size: 1.3, angle: 0.0, anchor: "start", ink: "frame_label" });
    d.push(t([x0 + 14.0, y0 + 21.8], s.title.date.clone(), 1.8, "start"));
    d.push(Draw::Text { at: [x0 + 38.0, y0 + 19.0], s: "Company".into(), size: 1.3, angle: 0.0, anchor: "start", ink: "frame_label" });
    d.push(t([x0 + 38.0, y0 + 21.8], s.title.company.clone(), 1.8, "start"));
    d.push(Draw::Text { at: [x0 + 84.0, y0 + 19.0], s: "Rev".into(), size: 1.3, angle: 0.0, anchor: "start", ink: "frame_label" });
    d.push(t([x0 + 84.0, y0 + 21.8], s.title.revision.clone(), 1.8, "start"));
    d.push(Draw::Text { at: [x0 + 2.0, y0 + 26.0], s: "Sheet".into(), size: 1.3, angle: 0.0, anchor: "start", ink: "frame_label" });
    d.push(t([x0 + 2.0, y0 + 29.2], format!("{} / {}", index + 1, s.sheets.len()), 1.8, "start"));
    d.push(Draw::Text { at: [x0 + 38.0, y0 + 26.0], s: "Drawn with".into(), size: 1.3, angle: 0.0, anchor: "start", ink: "frame_label" });
    d.push(t([x0 + 38.0, y0 + 29.2], "Forma".into(), 1.8, "start"));
    for (i, c) in s.title.comments.iter().take(3).enumerate() {
        d.push(t([x0 + 84.0, y0 + 26.0 + i as f64 * 2.2], c.clone(), 1.2, "start"));
    }
    d
}

// ---- Connectivity ----------------------------------------------------------------

struct Dsu(Vec<usize>);

impl Dsu {
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut y = x;
        while self.0[y] != r {
            let n = self.0[y];
            self.0[y] = r;
            y = n;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[b] = a;
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Node {
    Pin(usize, usize),
    Wire(usize),
    Label(usize),
    Junction,
    SheetPin(usize, usize),
}

fn key(sheet: &str, p: P) -> (String, i64, i64) {
    (sheet.to_string(), (p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64)
}

pub fn on_segment(p: P, a: P, b: P) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    if l2 < 1e-12 {
        return same(p, a);
    }
    let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2;
    if !(-1e-9..=1.0 + 1e-9).contains(&t) {
        return false;
    }
    let q = [a[0] + dx * t, a[1] + dy * t];
    (q[0] - p[0]).abs() < 1e-3 && (q[1] - p[1]).abs() < 1e-3
}

fn on_wire(p: P, pts: &[P]) -> bool {
    pts.windows(2).any(|w| on_segment(p, w[0], w[1]))
}

/// Works the schematic out from its entities.
pub fn resolve(world: &World, id: StableId) -> Result<SchematicView, String> {
    let e = entity(world, id).ok_or("no such schematic")?;
    let s = world.get::<Schematic>(e).ok_or("not a schematic")?.clone();
    let it = schematic::items(world, id);
    Ok(resolve_items(id, &s, &it, world))
}

pub fn resolve_items(id: StableId, s: &Schematic, it: &SchItems, world: &World) -> SchematicView {
    let mut symbols: Vec<SymbolView> = it.symbols.iter().map(|(sid, x)| symbol_view(*sid, x)).collect();
    let mut wires: Vec<WireView> = it.wires.iter().map(|(wid, w)| WireView { id: *wid, sheet: w.sheet.clone(), points: w.points.clone(), net: None }).collect();
    let mut labels: Vec<LabelView> = it.labels.iter().map(|(lid, l)| label_view(*lid, l)).collect();
    let sheet_name = |sid: &str| s.sheet(sid).map(|x| x.name.clone()).unwrap_or_else(|| sid.to_string());
    // Local names are bare on the first sheet, `sheet/NAME` elsewhere.
    let is_child = |sid: &str| s.sheets.first().is_some_and(|f| f.id != sid);

    // Nodes.
    let mut nodes: Vec<Node> = Vec::new();
    let mut points: HashMap<(String, i64, i64), Vec<usize>> = HashMap::new();
    let add_point = |nodes: &mut Vec<Node>, points: &mut HashMap<(String, i64, i64), Vec<usize>>, n: Node, sheet: &str, p: P| {
        nodes.push(n);
        let i = nodes.len() - 1;
        points.entry(key(sheet, p)).or_default().push(i);
        i
    };
    let mut pin_node: HashMap<(usize, usize), usize> = HashMap::new();
    for (si, sv) in symbols.iter().enumerate() {
        for (pi, p) in sv.pins.iter().enumerate() {
            if p.hidden {
                continue;
            }
            let n = add_point(&mut nodes, &mut points, Node::Pin(si, pi), &sv.sheet, p.at);
            pin_node.insert((si, pi), n);
        }
    }
    let wire0 = nodes.len();
    for (wi, w) in wires.iter().enumerate() {
        nodes.push(Node::Wire(wi));
        let i = nodes.len() - 1;
        points.entry(key(&w.sheet, w.points[0])).or_default().push(i);
        points.entry(key(&w.sheet, *w.points.last().unwrap())).or_default().push(i);
    }
    let mut label_node = Vec::new();
    for (li, l) in labels.iter().enumerate() {
        label_node.push(add_point(&mut nodes, &mut points, Node::Label(li), &l.sheet, l.at));
    }
    for j in it.junctions.iter() {
        add_point(&mut nodes, &mut points, Node::Junction, &j.1.sheet, j.1.at);
    }
    let mut sheet_pin_nodes: Vec<(String, String, usize)> = Vec::new(); // (child sheet, name, node)
    for (shi, sh) in s.sheets.iter().enumerate() {
        if let Some(parent) = &sh.parent {
            for (pi, p) in sh.pins.iter().enumerate() {
                let n = add_point(&mut nodes, &mut points, Node::SheetPin(shi, pi), parent, p.at);
                sheet_pin_nodes.push((sh.id.clone(), p.name.clone(), n));
            }
        }
    }
    let mut dsu = Dsu((0..nodes.len()).collect());
    // Coincident points.
    for list in points.values() {
        for w in list.windows(2) {
            dsu.union(w[0], w[1]);
        }
    }
    // Points on wires (ends on ends, Ts, pins and labels along a wire).
    let mut by_sheet: HashMap<String, Vec<usize>> = HashMap::new();
    for (wi, w) in wires.iter().enumerate() {
        by_sheet.entry(w.sheet.clone()).or_default().push(wi);
    }
    let mut point_list: Vec<(&(String, i64, i64), &Vec<usize>)> = points.iter().collect();
    point_list.sort_by(|a, b| a.0.cmp(b.0));
    for (k, list) in &point_list {
        let p = [k.1 as f64 / 1000.0, k.2 as f64 / 1000.0];
        if let Some(ws) = by_sheet.get(k.0.as_str()) {
            for wi in ws {
                if on_wire(p, &wires[*wi].points) {
                    dsu.union(wire0 + wi, list[0]);
                }
            }
        }
    }
    // Names: local labels per sheet, global labels and power ports everywhere, hierarchical ↔ sheet pins.
    let mut scope: HashMap<(u8, String, String), usize> = HashMap::new();
    for (li, l) in labels.iter().enumerate() {
        let k = match l.kind {
            LabelKind::Local => (0, l.sheet.clone(), l.text.clone()),
            LabelKind::Global | LabelKind::Power => (1, String::new(), l.text.clone()),
            LabelKind::Hierarchical => (2, l.sheet.clone(), l.text.clone()),
        };
        let n = label_node[li];
        match scope.get(&k) {
            Some(first) => dsu.union(*first, n),
            None => {
                scope.insert(k, n);
            }
        }
    }
    for (child, name, n) in &sheet_pin_nodes {
        if let Some(first) = scope.get(&(2, child.clone(), name.clone())) {
            dsu.union(*first, *n);
        }
    }

    // Groups.
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..nodes.len() {
        let r = dsu.find(i);
        groups.entry(r).or_default().push(i);
    }
    let mut nets: Vec<SchNet> = Vec::new();
    let mut net_of_root: HashMap<usize, String> = HashMap::new();
    let mut multi_names: Vec<(usize, Vec<String>)> = Vec::new();
    let mut used_names: BTreeMap<String, usize> = BTreeMap::new();
    let mut roots: Vec<(&usize, &Vec<usize>)> = groups.iter().collect();
    // Stable order: by first pin reference.
    roots.sort_by_key(|(r, _)| **r);
    for (root, members) in roots {
        let mut cands: Vec<(u8, String)> = Vec::new();
        let mut pins: Vec<(usize, usize)> = Vec::new();
        for m in members {
            match nodes[*m] {
                Node::Label(li) => {
                    let l = &labels[li];
                    let (prio, name) = match l.kind {
                        LabelKind::Power => (0, l.text.clone()),
                        LabelKind::Global => (1, l.text.clone()),
                        LabelKind::Local => (2, if is_child(&l.sheet) { format!("{}/{}", sheet_name(&l.sheet), l.text) } else { l.text.clone() }),
                        LabelKind::Hierarchical => (3, format!("{}/{}", sheet_name(&l.sheet), l.text)),
                    };
                    cands.push((prio, name));
                }
                Node::SheetPin(shi, pi) => {
                    let sh = &s.sheets[shi];
                    cands.push((4, format!("{}/{}", sheet_name(&sh.id), sh.pins[pi].name)));
                }
                Node::Pin(si, pi) => pins.push((si, pi)),
                _ => {}
            }
        }
        // A lone pin is not a net.
        if pins.is_empty() || members.len() == 1 {
            continue;
        }
        pins.sort_by(|a, b| natural(&symbols[a.0].reference, &symbols[b.0].reference).then(natural(&symbols[a.0].pins[a.1].number, &symbols[b.0].pins[b.1].number)));
        cands.sort();
        cands.dedup();
        let distinct: BTreeSet<&String> = cands.iter().map(|c| &c.1).collect();
        if distinct.len() > 1 {
            multi_names.push((*root, distinct.iter().map(|s| s.to_string()).collect()));
        }
        let (mut name, named) = match cands.first() {
            Some((_, n)) => (n.clone(), true),
            None => {
                let (si, pi) = pins[0];
                let sv = &symbols[si];
                let pad = sv.pins[pi].pads.first().cloned().unwrap_or_else(|| sv.pins[pi].number.clone());
                (format!("Net-({}-Pad{})", sv.reference, pad), false)
            }
        };
        if let Some(n) = used_names.get(&name) {
            name = format!("{name}_{}", n + 1);
        }
        *used_names.entry(name.clone()).or_default() += 1;
        net_of_root.insert(*root, name.clone());
        let mut pin_names = Vec::new();
        let mut pads = Vec::new();
        let mut sheets = BTreeSet::new();
        for (si, pi) in &pins {
            let sv = &symbols[*si];
            let p = &sv.pins[*pi];
            pin_names.push(format!("{}.{}", sv.reference, p.number));
            sheets.insert(sv.sheet.clone());
            if !sv.virtual_part {
                for pad in &p.pads {
                    pads.push(format!("{}.{}", sv.reference, pad));
                }
            }
        }
        pads.sort_by(|a, b| natural(a, b));
        pads.dedup();
        nets.push(SchNet { name, pins: pin_names, pads, labels: cands.into_iter().map(|c| c.1).collect(), sheets: sheets.into_iter().collect(), named });
    }
    nets.sort_by(|a, b| natural(&a.name, &b.name));
    // Back onto the items.
    for (i, n) in nodes.iter().enumerate() {
        let r = dsu.find(i);
        let Some(name) = net_of_root.get(&r) else { continue };
        match n {
            Node::Pin(si, pi) => symbols[*si].pins[*pi].net = Some(name.clone()),
            Node::Wire(wi) => wires[*wi].net = Some(name.clone()),
            Node::Label(li) => labels[*li].net = Some(name.clone()),
            _ => {}
        }
    }

    // Junction dots: placed ones, and wherever three or more wire ends / pins meet or a wire end lands mid-wire.
    let mut junctions: Vec<JunctionView> = it.junctions.iter().map(|(jid, j)| JunctionView { id: Some(*jid), sheet: j.sheet.clone(), at: j.at }).collect();
    for (k, list) in &point_list {
        let p = [k.1 as f64 / 1000.0, k.2 as f64 / 1000.0];
        let mut count = 0;
        let mut ends = 0;
        for n in list.iter() {
            match nodes[*n] {
                Node::Wire(wi) => {
                    let w = &wires[wi];
                    if same(w.points[0], p) {
                        ends += 1;
                    }
                    if same(*w.points.last().unwrap(), p) {
                        ends += 1;
                    }
                }
                Node::Pin(..) => count += 1,
                _ => {}
            }
        }
        let mids = by_sheet.get(k.0.as_str()).map(|ws| ws.iter().filter(|wi| on_wire(p, &wires[**wi].points) && !same(wires[**wi].points[0], p) && !same(*wires[**wi].points.last().unwrap(), p)).count()).unwrap_or(0);
        let total = count + ends + mids * 2;
        if total >= 3 && ends >= 1 && !junctions.iter().any(|j| j.sheet == k.0 && same(j.at, p)) {
            junctions.push(JunctionView { id: None, sheet: k.0.clone(), at: p });
        }
    }

    // ---- Electrical rules -------------------------------------------------------
    let mut v: Vec<ErcViolation> = Vec::new();
    let mut viol = |code: &'static str, severity: &'static str, message: String, sheet: &str, at: P, objects: Vec<StableId>, refs: Vec<String>| v.push(ErcViolation { code, severity, message, sheet: sheet.to_string(), at, objects, refs });
    let ncs: Vec<(StableId, String, P)> = it.no_connects.iter().map(|(nid, n)| (*nid, n.sheet.clone(), n.at)).collect();
    let group_of = |dsu: &mut Dsu, n: usize| dsu.find(n);
    let mut group_size: HashMap<usize, usize> = HashMap::new();
    for i in 0..nodes.len() {
        *group_size.entry(dsu.find(i)).or_default() += 1;
    }
    // Pins: unconnected, no-connect flags.
    let mut flagged: BTreeSet<usize> = BTreeSet::new();
    for (si, sv) in symbols.iter().enumerate() {
        for (pi, p) in sv.pins.iter().enumerate() {
            if p.hidden {
                continue;
            }
            let n = pin_node[&(si, pi)];
            let g = group_of(&mut dsu, n);
            let alone = group_size[&g] == 1;
            let nc = ncs.iter().position(|(_, sh, at)| sh == &sv.sheet && same(*at, p.at));
            if let Some(k) = nc {
                flagged.insert(k);
                if !alone {
                    viol("no_connect_connected", "warning", format!("{}.{} ({}) has a no-connect flag but is connected", sv.reference, p.number, p.name), &sv.sheet, p.at, vec![sv.id, ncs[k].0], vec![format!("{}.{}", sv.reference, p.number)]);
                }
                continue;
            }
            if p.etype == PinType::NoConnect {
                if !alone {
                    viol("no_connect_pin_connected", "warning", format!("{}.{} ({}) is a no-connect pin but is wired", sv.reference, p.number, p.name), &sv.sheet, p.at, vec![sv.id], vec![format!("{}.{}", sv.reference, p.number)]);
                }
                continue;
            }
            if alone {
                let name = if p.name.is_empty() || p.name == "~" { String::new() } else { format!(" ({})", p.name) };
                viol("unconnected_pin", "error", format!("{} pin {}{} is not connected (wire it, or put a no-connect flag on it)", sv.reference, p.number, name), &sv.sheet, p.at, vec![sv.id], vec![format!("{}.{}", sv.reference, p.number)]);
            }
        }
    }
    for (k, (nid, sh, at)) in ncs.iter().enumerate() {
        if !flagged.contains(&k) {
            viol("no_connect_dangling", "warning", "no-connect flag not on a pin".into(), sh, *at, vec![*nid], vec![]);
        }
    }
    // Pin types on each net.
    let mut net_pins: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (si, sv) in symbols.iter().enumerate() {
        for (pi, p) in sv.pins.iter().enumerate() {
            if let Some(n) = &p.net {
                net_pins.entry(n.clone()).or_default().push((si, pi));
            }
        }
    }
    for (net, list) in &net_pins {
        let of = |t: PinType| list.iter().filter(|(si, pi)| symbols[*si].pins[*pi].etype == t).cloned().collect::<Vec<_>>();
        let outs = of(PinType::Output);
        let pouts = of(PinType::PowerOut);
        let pins_txt = |l: &[(usize, usize)]| l.iter().map(|(si, pi)| format!("{}.{}", symbols[*si].reference, symbols[*si].pins[*pi].number)).collect::<Vec<_>>();
        let objs = |l: &[(usize, usize)]| {
            let mut o: Vec<StableId> = l.iter().map(|(si, _)| symbols[*si].id).collect();
            o.dedup();
            o
        };
        let first_at = |l: &[(usize, usize)]| (symbols[l[0].0].sheet.clone(), symbols[l[0].0].pins[l[0].1].at);
        let conflict = |a: &[(usize, usize)], b: &[(usize, usize)]| -> Vec<(usize, usize)> { a.iter().chain(b.iter()).cloned().collect() };
        if outs.len() >= 2 {
            let (sh, at) = first_at(&outs);
            viol("pin_conflict", "error", format!("{net}: outputs {} drive each other", pins_txt(&outs).join(", ")), &sh, at, objs(&outs), pins_txt(&outs));
        }
        if pouts.len() >= 2 {
            let (sh, at) = first_at(&pouts);
            viol("pin_conflict", "error", format!("{net}: power outputs {} are shorted together", pins_txt(&pouts).join(", ")), &sh, at, objs(&pouts), pins_txt(&pouts));
        }
        if !outs.is_empty() && !pouts.is_empty() {
            let c = conflict(&outs, &pouts);
            let (sh, at) = first_at(&c);
            viol("pin_conflict", "error", format!("{net}: output {} is shorted to power output {}", pins_txt(&outs).join(", "), pins_txt(&pouts).join(", ")), &sh, at, objs(&c), pins_txt(&c));
        }
        let tri = of(PinType::TriState);
        if !tri.is_empty() && !outs.is_empty() {
            let c = conflict(&tri, &outs);
            let (sh, at) = first_at(&c);
            viol("pin_conflict", "warning", format!("{net}: tri-state {} shares the net with output {}", pins_txt(&tri).join(", "), pins_txt(&outs).join(", ")), &sh, at, objs(&c), pins_txt(&c));
        }
        let pins_in = of(PinType::PowerIn);
        if !pins_in.is_empty() && pouts.is_empty() {
            let (sh, at) = first_at(&pins_in);
            viol("power_not_driven", "error", format!("{net} feeds power inputs ({}) but nothing drives it: put a PWR_FLAG where the supply comes in (a connector), or check the regulator's output", pins_txt(&pins_in).join(", ")), &sh, at, objs(&pins_in), pins_txt(&pins_in));
        }
    }
    // Single-pin nets (a wire or label leading nowhere).
    for n in &nets {
        if n.pins.len() == 1 {
            let (si, pi) = net_pins[&n.name][0];
            let sv = &symbols[si];
            let msg = format!("net {} reaches only {} (a label or wire leading nowhere?)", n.name, n.pins[0]);
            viol("single_pin_net", "warning", msg, &sv.sheet, sv.pins[pi].at, vec![sv.id], n.pins.clone());
        }
    }
    for (root, names) in &multi_names {
        let at = groups[root].iter().find_map(|m| if let Node::Label(li) = nodes[*m] { Some((labels[li].sheet.clone(), labels[li].at, labels[li].id)) } else { None });
        if let Some((sh, at, lid)) = at {
            viol("multiple_net_names", "warning", format!("one net carries several names: {} (it is called {})", names.join(", "), net_of_root.get(root).cloned().unwrap_or_default()), &sh, at, vec![lid], vec![]);
        }
    }
    // Labels used once whose net goes nowhere: a typo?
    let mut label_names: BTreeMap<(u8, String), Vec<usize>> = BTreeMap::new();
    for (li, l) in labels.iter().enumerate() {
        let k = match l.kind {
            LabelKind::Local | LabelKind::Hierarchical => (0, format!("{}\u{1}{}", l.sheet, l.text)),
            _ => (1, l.text.clone()),
        };
        label_names.entry(k).or_default().push(li);
    }
    let all_names: Vec<String> = labels.iter().map(|l| l.text.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    for list in label_names.values() {
        if list.len() != 1 {
            continue;
        }
        let l = &labels[list[0]];
        if l.kind == LabelKind::Hierarchical {
            continue;
        }
        let pins_on = l.net.as_ref().and_then(|n| net_pins.get(n)).map(|x| x.len()).unwrap_or(0);
        if pins_on <= 1 {
            let similar = all_names.iter().filter(|o| *o != &l.text && similar(o, &l.text)).cloned().collect::<Vec<_>>();
            let hint = if similar.is_empty() { String::new() } else { format!(" — did you mean {}?", similar.join(" or ")) };
            viol("label_unmatched", "warning", format!("label “{}” appears once and joins nothing else{hint}", l.text), &l.sheet, l.at, vec![l.id], vec![]);
        }
    }
    // Dangling wire ends.
    for w in &wires {
        for end in [w.points[0], *w.points.last().unwrap()] {
            let others = points.get(&key(&w.sheet, end)).map(|l| l.iter().filter(|n| !matches!(nodes[**n], Node::Wire(x) if wires[x].id == w.id)).count()).unwrap_or(0);
            let mid = by_sheet.get(w.sheet.as_str()).map(|ws| ws.iter().any(|wi| wires[*wi].id != w.id && on_wire(end, &wires[*wi].points))).unwrap_or(false);
            let both_ends_same_wire = same(w.points[0], *w.points.last().unwrap());
            if others == 0 && !mid && !both_ends_same_wire {
                viol("dangling_wire", "warning", "wire end connects to nothing".into(), &w.sheet, end, vec![w.id], vec![]);
            }
        }
    }
    // References: duplicates, unannotated, footprints and pin maps.
    let mut by_ref: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (si, sv) in symbols.iter().enumerate() {
        by_ref.entry(sv.reference.to_ascii_uppercase()).or_default().push(si);
    }
    let board_fps = s.board.and_then(|b| entity(world, b)).and_then(|e| world.get::<crate::Board>(e)).map(|b| b.footprints.clone()).unwrap_or_default();
    for list in by_ref.values() {
        let first = &symbols[list[0]];
        if first.reference.contains('?') {
            for si in list {
                let sv = &symbols[*si];
                viol("not_annotated", "error", format!("{} is not numbered yet (annotate)", sv.reference), &sv.sheet, sv.at, vec![sv.id], vec![sv.reference.clone()]);
            }
            continue;
        }
        let mut units_seen = BTreeSet::new();
        for si in list {
            let sv = &symbols[*si];
            if sv.symbol != first.symbol || !units_seen.insert(sv.unit) {
                viol("duplicate_reference", "error", format!("{} is used by more than one symbol", sv.reference), &sv.sheet, sv.at, list.iter().map(|i| symbols[*i].id).collect(), vec![sv.reference.clone()]);
                break;
            }
        }
        if first.virtual_part || first.exclude_from_board || first.reference.starts_with('#') {
            continue;
        }
        if !first.known {
            viol("unknown_symbol", "error", format!("{}: no symbol “{}” in the library", first.reference, first.symbol), &first.sheet, first.at, vec![first.id], vec![first.reference.clone()]);
            continue;
        }
        if first.units > 1 {
            let missing: Vec<String> = (1..=first.units).filter(|u| !units_seen.contains(u)).map(unit_letter).collect();
            if !missing.is_empty() {
                viol("missing_unit", "warning", format!("{} unit{} {} not placed (their pins stay unconnected)", first.reference, if missing.len() > 1 { "s" } else { "" }, missing.join(", ")), &first.sheet, first.at, vec![first.id], vec![first.reference.clone()]);
            }
        }
        if first.footprint.is_empty() {
            viol("missing_footprint", "error", format!("{} ({}) has no footprint: choose one so it can go on the board", first.reference, first.value), &first.sheet, first.at, vec![first.id], vec![first.reference.clone()]);
            continue;
        }
        let fp = board_fps.get(&first.footprint).cloned().or_else(|| crate::footprints::get(&first.footprint));
        let Some(fp) = fp else {
            viol("unknown_footprint", "error", format!("{}: no footprint “{}” (GET /pcb/library; or define it on the board)", first.reference, first.footprint), &first.sheet, first.at, vec![first.id], vec![first.reference.clone()]);
            continue;
        };
        // Pin map: every mapped pad exists; every electrical pad has a pin.
        let def = it.symbols.iter().find(|(sid, _)| *sid == first.id).and_then(|(_, x)| x.def());
        let inst = it.symbols.iter().find(|(sid, _)| *sid == first.id).map(|(_, x)| x.clone());
        if let (Some(def), Some(inst)) = (def, inst) {
            let mut covered = BTreeSet::new();
            let mut bad_pads = Vec::new();
            for p in &def.pins {
                for pad in inst.pads_of(p) {
                    if fp.pads.iter().any(|x| x.number == pad) {
                        covered.insert(pad);
                    } else {
                        bad_pads.push(format!("{} → {pad}", p.number));
                    }
                }
            }
            if !bad_pads.is_empty() {
                viol("pin_map", "error", format!("{}: pins land on pads {} lacks ({})", first.reference, fp.name, bad_pads.join(", ")), &first.sheet, first.at, vec![first.id], vec![first.reference.clone()]);
            }
            let mut loose: Vec<String> = fp.pads.iter().filter(|p| !p.number.is_empty() && p.kind != crate::footprints::PadKind::Npth && !covered.contains(&p.number)).map(|p| p.number.clone()).collect();
            loose.sort_by(|a, b| natural(a, b));
            loose.dedup();
            if !loose.is_empty() {
                viol("pin_map", "warning", format!("{}: pads {} of {} have no symbol pin (they stay unconnected)", first.reference, loose.join(", "), fp.name), &first.sheet, first.at, vec![first.id], vec![first.reference.clone()]);
            }
        }
    }

    let texts: Vec<TextView> = it.texts.iter().map(|(tid, t)| TextView { id: *tid, sheet: t.sheet.clone(), at: t.at, text: t.text.clone(), size: t.size }).collect();
    let no_connects: Vec<MarkView> = it.no_connects.iter().map(|(nid, n)| MarkView { id: *nid, sheet: n.sheet.clone(), at: n.at }).collect();
    let sheets: Vec<SheetView> = s
        .sheets
        .iter()
        .enumerate()
        .map(|(i, sh)| {
            let mut boxes = Vec::new();
            let mut boxes_draw = Vec::new();
            for child in s.sheets.iter().filter(|c| c.parent.as_deref() == Some(sh.id.as_str())) {
                let at = child.at.unwrap_or([25.4, 25.4]);
                let size = child.size.unwrap_or([30.48, 20.32]);
                boxes_draw.push(Draw::Poly { p: vec![at, [at[0] + size[0], at[1]], [at[0] + size[0], at[1] + size[1]], [at[0], at[1] + size[1]]], w: W_BODY, ink: "sheet", fill: "body" });
                boxes_draw.push(Draw::Text { at: [at[0], at[1] - 1.0], s: child.name.clone(), size: FIELD, angle: 0.0, anchor: "start", ink: "sheet" });
                for p in &child.pins {
                    let anchor = if p.side == Dir::Right { "end" } else { "start" };
                    let off = if p.side == Dir::Right { -1.0 } else { 1.0 };
                    boxes_draw.push(Draw::Text { at: [p.at[0] + off, p.at[1]], s: p.name.clone(), size: LABEL, angle: 0.0, anchor, ink: "hierarchical" });
                }
                boxes.push(SheetBoxView { sheet: child.id.clone(), name: child.name.clone(), at, size, pins: child.pins.iter().map(|p| (p.name.clone(), p.at)).collect() });
            }
            SheetView { id: sh.id.clone(), name: sh.name.clone(), paper: sh.paper.clone(), size: sh.paper_size(), parent: sh.parent.clone(), frame: frame(s, sh, i), boxes, boxes_draw }
        })
        .collect();
    let errors = v.iter().filter(|x| x.severity == "error").count();
    let stats = SchStats {
        sheets: s.sheets.len(),
        symbols: symbols.len(),
        parts: symbols.iter().filter(|x| !x.virtual_part).map(|x| x.reference.clone()).collect::<BTreeSet<_>>().len(),
        wires: wires.len(),
        labels: labels.len(),
        nets: nets.len(),
        errors,
        warnings: v.len() - errors,
    };
    SchematicView {
        id,
        name: s.name.clone(),
        board: s.board,
        title: s.title.clone(),
        board_only: s.board_only.clone(),
        sheets,
        symbols,
        wires,
        labels,
        junctions,
        no_connects,
        texts,
        nets,
        violations: v,
        stats,
    }
}

/// Names a typo away from each other (case, one edit, or a swapped pair).
fn similar(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.to_ascii_uppercase().chars().collect(), b.to_ascii_uppercase().chars().collect());
    if a.len().abs_diff(b.len()) > 1 || a.len() < 2 {
        return false;
    }
    // Levenshtein ≤ 1, or one adjacent swap.
    let n = a.len();
    let m = b.len();
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..=n {
        d[i][0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let c = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + c);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m] <= 1
}

// ---- Wire routing --------------------------------------------------------------------

/// An orthogonal path from `a` to `b` on `sheet` that keeps out of symbol
/// bodies and never touches another pin, label or wire end (which would
/// connect it), preferring few bends and short length.
pub fn route(v: &SchematicView, sheet: &str, a: P, da: Option<Dir>, b: P, db: Option<Dir>) -> Vec<P> {
    let g = 2.54;
    let bodies: Vec<[f64; 4]> = v.symbols.iter().filter(|s| s.sheet == sheet).map(|s| [s.body[0] + 0.05, s.body[1] + 0.05, s.body[2] - 0.05, s.body[3] - 0.05]).collect();
    let mut forbidden: Vec<P> = Vec::new();
    for s in v.symbols.iter().filter(|s| s.sheet == sheet) {
        for p in s.pins.iter().filter(|p| !p.hidden) {
            forbidden.push(p.at);
        }
    }
    for l in v.labels.iter().filter(|l| l.sheet == sheet) {
        forbidden.push(l.at);
    }
    let mut segs: Vec<(P, P)> = Vec::new();
    for w in v.wires.iter().filter(|w| w.sheet == sheet) {
        forbidden.push(w.points[0]);
        forbidden.push(*w.points.last().unwrap());
        for s in w.points.windows(2) {
            segs.push((s[0], s[1]));
        }
    }
    forbidden.retain(|p| !same(*p, a) && !same(*p, b));
    let ea = da.map(|d| [a[0] + d.v()[0] * g, a[1] + d.v()[1] * g]);
    let eb = db.map(|d| [b[0] + d.v()[0] * g, b[1] + d.v()[1] * g]);
    let starts: Vec<Vec<P>> = match ea {
        Some(e) => vec![vec![a], vec![a, e]],
        None => vec![vec![a]],
    };
    let ends: Vec<Vec<P>> = match eb {
        Some(e) => vec![vec![b], vec![e, b]],
        None => vec![vec![b]],
    };
    let mut best: Option<(f64, Vec<P>)> = None;
    for st in &starts {
        for en in &ends {
            let p = *st.last().unwrap();
            let q = en[0];
            let mut mids: Vec<Vec<P>> = vec![vec![[q[0], p[1]]], vec![[p[0], q[1]]]];
            for k in -8i32..=8 {
                let xm = schematic::snap1((p[0] + q[0]) / 2.0 + k as f64 * g);
                mids.push(vec![[xm, p[1]], [xm, q[1]]]);
                let ym = schematic::snap1((p[1] + q[1]) / 2.0 + k as f64 * g);
                mids.push(vec![[p[0], ym], [q[0], ym]]);
            }
            for m in mids {
                let mut path = st.clone();
                path.extend(m);
                path.extend(en.iter().cloned());
                let path = schematic::simplify(&path);
                if path.windows(2).any(|w| (w[0][0] - w[1][0]).abs() > 1e-6 && (w[0][1] - w[1][1]).abs() > 1e-6) {
                    continue;
                }
                let score = score_path(&path, &bodies, &forbidden, &segs, da, db);
                if best.as_ref().is_none_or(|(s, _)| score < *s) {
                    best = Some((score, path));
                }
            }
        }
    }
    best.map(|b| b.1).unwrap_or_else(|| vec![a, [b[0], a[1]], b])
}

fn score_path(path: &[P], bodies: &[[f64; 4]], forbidden: &[P], segs: &[(P, P)], da: Option<Dir>, db: Option<Dir>) -> f64 {
    let mut score = 0.0;
    for w in path.windows(2) {
        let (p, q) = (w[0], w[1]);
        score += (p[0] - q[0]).abs() + (p[1] - q[1]).abs();
        let sb = seg_box(p, q, 0.0);
        for b in bodies {
            if sb[0] < b[2] && b[0] < sb[2] && sb[1] < b[3] && b[1] < sb[3] {
                score += 1000.0;
            }
        }
        for f in forbidden {
            if on_segment(*f, p, q) {
                score += 100_000.0;
            }
        }
        for (s0, s1) in segs {
            // Running along an existing wire would join it.
            let h_new = (p[1] - q[1]).abs() < 1e-6;
            let h_old = (s0[1] - s1[1]).abs() < 1e-6;
            if h_new && h_old && (p[1] - s0[1]).abs() < 1e-6 {
                let (a0, a1) = (p[0].min(q[0]), p[0].max(q[0]));
                let (b0, b1) = (s0[0].min(s1[0]), s0[0].max(s1[0]));
                if a0.max(b0) < a1.min(b1) - 1e-6 {
                    score += 100_000.0;
                }
            } else if !h_new && !h_old && (p[0] - s0[0]).abs() < 1e-6 {
                let (a0, a1) = (p[1].min(q[1]), p[1].max(q[1]));
                let (b0, b1) = (s0[1].min(s1[1]), s0[1].max(s1[1]));
                if a0.max(b0) < a1.min(b1) - 1e-6 {
                    score += 100_000.0;
                }
            }
        }
    }
    // A bend landing on an old wire makes a T: it would join it.
    for e in &path[1..path.len() - 1] {
        if segs.iter().any(|(s0, s1)| on_segment(*e, *s0, *s1)) {
            score += 100_000.0;
        }
    }
    score += 4.0 * (path.len() as f64 - 2.0).max(0.0);
    // Leaving a pin backwards (into its own body) is ugly even when it clears.
    let leaves = |from: P, to: P, d: Option<Dir>| d.is_some_and(|d| {
        let v = d.v();
        (to[0] - from[0]) * v[0] + (to[1] - from[1]) * v[1] < -1e-6
    });
    if path.len() >= 2 {
        if leaves(path[0], path[1], da) {
            score += 50.0;
        }
        let n = path.len();
        if leaves(path[n - 1], path[n - 2], db) {
            score += 50.0;
        }
    }
    score
}

/// One symbol drawn on its own (library previews).
pub fn symbol_preview(s: &schematic::SchSymbol) -> SymbolView {
    symbol_view(StableId::new(), s)
}
