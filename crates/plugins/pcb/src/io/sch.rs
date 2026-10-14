//! Schematics from other tools put into Iṣẹ́ Studio's terms. A reader fills a
//! [`Draft`]: symbols drawn where they sit (their body graphics and pins in
//! the source's sheet coordinates), wires, labels, power ports, junctions,
//! no-connect flags and notes, per sheet. [`Draft::finish`] moves each
//! sheet onto paper (y down, a margin from the top-left corner, the
//! smallest A-size sheet that holds it) and gives every symbol its own
//! definition, drawn relative to its anchor, so the import shows exactly
//! what the source drew and connects exactly where its pins are.
//!
//! [`create`] writes a [`SchIn`] (from a draft or the KiCad reader) into the
//! project as one change.

use crate::schematic::{LabelKind, SchJunction, SchLabel, SchNoConnect, SchSymbol, SchText, SchWire, Schematic, Sheet, TitleBlock};
use crate::symbols::{Dir, Fill, Gfx, PinDef, PinType, SymbolDef};
use de_core::{CommandError, ErrorCode, StableId, Tx, Value, json};
use std::collections::BTreeMap;

type P = [f64; 2];

/// A schematic ready to write (items carry a placeholder schematic id).
#[derive(Clone, Debug, Default)]
pub struct SchIn {
    pub name: Option<String>,
    pub title: TitleBlock,
    pub sheets: Vec<Sheet>,
    pub symbols: Vec<SchSymbol>,
    pub wires: Vec<SchWire>,
    pub labels: Vec<SchLabel>,
    pub junctions: Vec<SchJunction>,
    pub no_connects: Vec<SchNoConnect>,
    pub texts: Vec<SchText>,
    pub notes: Vec<String>,
}

/// The schematic written into the project: the schematic and every item, in one change.
pub fn create(tx: &mut Tx, s: &SchIn, name: Option<String>, board: Option<StableId>) -> Result<Value, CommandError> {
    let name = name.filter(|n| !n.trim().is_empty()).or(s.name.clone()).unwrap_or_else(|| "Imported schematic".into()).trim().to_string();
    if let Some(b) = board {
        if tx.get::<crate::Board>(b).is_none() {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{b} is not a board")));
        }
        if let Some((other, _)) = crate::schematic::of_board(tx.world(), b) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, format!("the board already has a schematic ({other})")));
        }
    }
    let mut sheets = s.sheets.clone();
    if sheets.is_empty() {
        sheets.push(Sheet { id: "1".into(), name: "Root".into(), paper: "A4".into(), portrait: false, parent: None, at: None, size: None, pins: vec![], ..Default::default() });
    }
    let mut title = s.title.clone();
    if title.title.is_empty() {
        title.title = name.clone();
    }
    let sch = Schematic { name: name.clone(), board, sheets, title, board_only: vec![], ..Default::default() };
    let id = tx.create();
    tx.set(id, sch)?;
    macro_rules! put {
        ($items:expr) => {
            for it in $items.iter() {
                let mut it = it.clone();
                it.schematic = id;
                let x = tx.create();
                tx.set(x, it)?;
                tx.add_dependency(id, x)?;
            }
        };
    }
    put!(s.symbols);
    put!(s.wires);
    put!(s.labels);
    put!(s.junctions);
    put!(s.no_connects);
    put!(s.texts);
    Ok(json!({
        "id": id,
        "name": name,
        "sheets": s.sheets.len().max(1),
        "symbols": s.symbols.len(),
        "wires": s.wires.len(),
        "labels": s.labels.len(),
        "notes": s.notes,
    }))
}

// ---- Drafts --------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct DraftSymbol {
    pub sheet: String,
    pub reference: String,
    pub value: String,
    pub footprint: String,
    /// The source library's name for the symbol.
    pub lib: String,
    /// Anchor in source coordinates.
    pub at: P,
    /// Body graphics in source coordinates.
    pub gfx: Vec<Gfx>,
    /// Pins: `at` the connection point in source coordinates, `side` the way it leaves the body
    /// in the source's sense of up.
    pub pins: Vec<PinDef>,
    pub unit: u32,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub description: String,
    pub fields: BTreeMap<String, String>,
    pub dnp: bool,
    /// A power flag or similar: never on the board.
    pub virtual_part: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub name: Option<String>,
    pub title: TitleBlock,
    /// (id, name) of each sheet, in order.
    pub sheets: Vec<(String, String)>,
    /// The source's y axis points up (Eagle, Altium); else down (EasyEDA, KiCad).
    pub y_up: bool,
    pub symbols: Vec<DraftSymbol>,
    pub wires: Vec<(String, Vec<P>)>,
    /// (sheet, kind, text, anchor, rotation as seen: 0 the text runs right, 90 up, 180 left, 270 down).
    pub labels: Vec<(String, LabelKind, String, P, f64)>,
    pub junctions: Vec<(String, P)>,
    pub no_connects: Vec<(String, P)>,
    pub texts: Vec<(String, P, String, f64)>,
    pub notes: Vec<String>,
}

fn flip_dir(d: Dir) -> Dir {
    match d {
        Dir::Up => Dir::Down,
        Dir::Down => Dir::Up,
        x => x,
    }
}

fn gfx_points(g: &Gfx) -> Vec<P> {
    match g {
        Gfx::Line { pts, .. } | Gfx::Poly { pts, .. } => pts.clone(),
        Gfx::Rect { a, b, .. } => vec![*a, *b],
        Gfx::Circle { c, r, .. } | Gfx::Arc { c, r, .. } => vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] + r]],
        Gfx::Text { at, .. } => vec![*at],
    }
}

fn map_gfx(g: &Gfx, f: &dyn Fn(P) -> P, flip: bool) -> Gfx {
    match g {
        Gfx::Line { pts, width } => Gfx::Line { pts: pts.iter().map(|p| f(*p)).collect(), width: *width },
        Gfx::Poly { pts, fill } => Gfx::Poly { pts: pts.iter().map(|p| f(*p)).collect(), fill: *fill },
        Gfx::Rect { a, b, fill } => Gfx::Rect { a: f(*a), b: f(*b), fill: *fill },
        Gfx::Circle { c, r, fill } => Gfx::Circle { c: f(*c), r: *r, fill: *fill },
        Gfx::Arc { c, r, start, end } => {
            if flip {
                Gfx::Arc { c: f(*c), r: *r, start: -end, end: -start }
            } else {
                Gfx::Arc { c: f(*c), r: *r, start: *start, end: *end }
            }
        }
        Gfx::Text { at, text, size } => Gfx::Text { at: f(*at), text: text.clone(), size: *size },
    }
}

fn paper_for(w: f64, h: f64) -> (&'static str, [f64; 2]) {
    for (name, size) in [("A4", [297.0, 210.0]), ("A3", [420.0, 297.0]), ("A2", [594.0, 420.0]), ("A1", [841.0, 594.0])] {
        if w + 40.0 <= size[0] && h + 40.0 <= size[1] {
            return (name, size);
        }
    }
    ("A1", [841.0, 594.0])
}

fn prefix_of(reference: &str) -> String {
    let p: String = reference.chars().take_while(|c| c.is_ascii_alphabetic() || *c == '#').collect();
    if p.is_empty() { "U".into() } else { p }
}

fn snap(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

impl Draft {
    pub fn note(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    pub fn finish(self) -> SchIn {
        let placeholder = StableId::new();
        let mut sheet_ids: Vec<(String, String)> = self.sheets.clone();
        if sheet_ids.is_empty() {
            sheet_ids.push(("1".into(), "Root".into()));
        }
        let mut out = SchIn { name: self.name.clone(), title: self.title.clone(), notes: self.notes.clone(), ..Default::default() };
        let known: Vec<String> = sheet_ids.iter().map(|s| s.0.clone()).collect();
        let sheet_of = |s: &str| if known.iter().any(|k| k == s) { s.to_string() } else { known[0].clone() };
        for (sid, sname) in &sheet_ids {
            // Bounds of everything on the sheet in source coordinates.
            let mut pts: Vec<P> = Vec::new();
            for s in self.symbols.iter().filter(|s| sheet_of(&s.sheet) == *sid) {
                pts.push(s.at);
                pts.extend(s.pins.iter().map(|p| p.at));
                pts.extend(s.gfx.iter().flat_map(gfx_points));
            }
            pts.extend(self.wires.iter().filter(|w| sheet_of(&w.0) == *sid).flat_map(|w| w.1.clone()));
            pts.extend(self.labels.iter().filter(|l| sheet_of(&l.0) == *sid).map(|l| l.3));
            pts.extend(self.texts.iter().filter(|t| sheet_of(&t.0) == *sid).map(|t| t.1));
            let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for p in &pts {
                b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
            }
            if pts.is_empty() {
                b = [0.0, 0.0, 0.0, 0.0];
            }
            let (paper, _) = paper_for(b[2] - b[0], b[3] - b[1]);
            // On the 1.27 mm grid, 20 mm in from the corner.
            let g = crate::schematic::GRID;
            let ox = (20.0 / g).round() * g - (b[0] / g).round() * g;
            let y_up = self.y_up;
            let oy = if y_up { (20.0 / g).round() * g + (b[3] / g).round() * g } else { (20.0 / g).round() * g - (b[1] / g).round() * g };
            let f = move |p: P| -> P { if y_up { [snap(p[0] + ox), snap(oy - p[1])] } else { [snap(p[0] + ox), snap(p[1] + oy)] } };
            out.sheets.push(Sheet { id: sid.clone(), name: sname.clone(), paper: paper.into(), portrait: false, parent: None, at: None, size: None, pins: vec![], ..Default::default() });
            for s in self.symbols.iter().filter(|s| sheet_of(&s.sheet) == *sid) {
                let at = f(s.at);
                let rel = |p: P| -> P {
                    let q = f(p);
                    [snap(q[0] - at[0]), snap(q[1] - at[1])]
                };
                let pins: Vec<PinDef> = s.pins.iter().map(|p| PinDef { at: rel(p.at), side: if y_up { flip_dir(p.side) } else { p.side }, unit: 0, ..p.clone() }).collect();
                let gfx: Vec<Gfx> = s.gfx.iter().map(|g| map_gfx(g, &rel, y_up)).collect();
                let def = SymbolDef {
                    name: if s.lib.is_empty() { format!("imported:{}", s.reference) } else { s.lib.clone() },
                    description: s.description.clone(),
                    prefix: prefix_of(&s.reference),
                    category: if s.virtual_part { "Power".into() } else { "IC".into() },
                    value: s.value.clone(),
                    footprint: s.footprint.clone(),
                    footprints: vec![],
                    mpn: s.mpn.clone(),
                    manufacturer: s.manufacturer.clone(),
                    lcsc: None,
                    pins,
                    // A gate of a multi-gate part draws as its own unit number.
                    units: (1..s.unit.max(1)).map(|_| vec![]).chain(std::iter::once(gfx)).collect(),
                    show_pin_numbers: !s.virtual_part,
                    show_pin_names: !s.virtual_part,
                    virtual_part: s.virtual_part,
                    keywords: vec![],
                    demorgan: vec![],
                    demorgan_pins: BTreeMap::new(),
                    body_style: 1,
                    fields: BTreeMap::new(),
                };
                out.symbols.push(SchSymbol {
                    schematic: placeholder,
                    sheet: sid.clone(),
                    symbol: def.name.clone(),
                    reference: s.reference.clone(),
                    value: s.value.clone(),
                    footprint: s.footprint.clone(),
                    at,
                    rotation: 0.0,
                    mirror: false,
                    unit: s.unit.max(1),
                    mpn: s.mpn.clone(),
                    manufacturer: s.manufacturer.clone(),
                    lcsc: None,
                    description: s.description.clone(),
                    dnp: s.dnp,
                    pin_map: BTreeMap::new(),
                    pins: vec![],
                    fields: s.fields.clone(),
                    ref_at: None,
                    value_at: None,
                    exclude_from_board: false,
                    component: None,
                    definition: Some(Box::new(def)),
                });
            }
            for (_, w) in self.wires.iter().filter(|w| sheet_of(&w.0) == *sid) {
                let pts: Vec<P> = w.iter().map(|p| f(*p)).collect();
                if pts.len() >= 2 {
                    out.wires.push(SchWire { schematic: placeholder, sheet: sid.clone(), points: pts });
                }
            }
            for (_, kind, text, at, rot) in self.labels.iter().filter(|l| sheet_of(&l.0) == *sid) {
                let rotation = rot.rem_euclid(360.0);
                out.labels.push(SchLabel { schematic: placeholder, sheet: sid.clone(), kind: *kind, text: text.clone(), at: f(*at), rotation, shape: None, style: None });
            }
            for (_, at) in self.junctions.iter().filter(|j| sheet_of(&j.0) == *sid) {
                out.junctions.push(SchJunction { schematic: placeholder, sheet: sid.clone(), at: f(*at) });
            }
            for (_, at) in self.no_connects.iter().filter(|j| sheet_of(&j.0) == *sid) {
                out.no_connects.push(SchNoConnect { schematic: placeholder, sheet: sid.clone(), at: f(*at) });
            }
            for (_, at, text, size) in self.texts.iter().filter(|t| sheet_of(&t.0) == *sid) {
                if !text.trim().is_empty() {
                    out.texts.push(SchText { schematic: placeholder, sheet: sid.clone(), at: f(*at), text: text.clone(), size: if *size > 0.3 { *size } else { 1.8 }, justify: String::new(), angle: 0.0, bold: false, italic: false, color: None });
                }
            }
        }
        out
    }
}

/// A rectangle body graphic.
pub fn rect(a: P, b: P) -> Gfx {
    Gfx::Rect { a, b, fill: Fill::Body }
}

pub fn line(pts: Vec<P>) -> Gfx {
    Gfx::Line { pts, width: 0.0 }
}

/// A pin from its connection point and the way it leaves the body.
pub fn pin(number: &str, name: &str, at: P, side: Dir, length: f64, etype: PinType) -> PinDef {
    PinDef { number: number.to_string(), name: if name.is_empty() { "~".into() } else { name.to_string() }, at, side, length, etype, hidden: false, unit: 0, shape: Default::default(), pads: vec![] }
}

/// The direction a pin leaves the body when its line runs from `body` to `tip` (source coordinates).
pub fn side_from(body: P, tip: P) -> Dir {
    Dir::from_v([tip[0] - body[0], tip[1] - body[1]])
}
