//! Schematic capture (DE-850): the circuit drawn as symbols and wires, the
//! source of a board's netlist.
//!
//! # Data model
//!
//! Like a board, a schematic is several entities so teammates can draw on
//! different parts of it at once. Every item points at its schematic and
//! names the sheet it is on; the schematic depends on all of them, so its
//! derived result (connectivity, nets, the electrical-rules check, and every
//! item drawn in sheet coordinates) recomputes when any changes.
//!
//! | kind                 | struct          | what                                                         |
//! |----------------------|-----------------|--------------------------------------------------------------|
//! | `pcb.schematic`      | [`Schematic`]   | name, the board it drives, sheets (paper, title block, hierarchy), board-only refs |
//! | `pcb.sch_symbol`     | [`SchSymbol`]   | a symbol instance: library symbol, reference, value, footprint, place, rotation, mirror, unit, MPN/LCSC fields, pin map |
//! | `pcb.sch_wire`       | [`SchWire`]     | a polyline of wire                                           |
//! | `pcb.sch_label`      | [`SchLabel`]    | a net label: local, global, hierarchical, or a power port    |
//! | `pcb.sch_junction`   | [`SchJunction`] | a junction dot (joins wires that cross)                      |
//! | `pcb.sch_no_connect` | [`SchNoConnect`]| a no-connect flag on a pin                                   |
//! | `pcb.sch_text`       | [`SchText`]     | a note                                                       |
//! | `pcb.sch_bus`        | [`SchBus`]      | a bus: a polyline carrying several nets, named by a bus label |
//! | `pcb.sch_bus_entry`  | [`SchBusEntry`] | a 45° entry joining a wire to a bus                          |
//! | `pcb.sch_shape`      | [`SchShape`]    | a graphic polyline, polygon, rectangle, circle, arc or bezier |
//! | `pcb.sch_text_box`   | [`SchTextBox`]  | framed text                                                  |
//! | `pcb.sch_table`      | [`SchTable`]    | a table of text cells                                        |
//! | `pcb.sch_image`      | [`SchImage`]    | a PNG or JPEG picture                                        |
//!
//! Coordinates are millimetres on the sheet, **y down**, origin at the
//! paper's top-left corner (as KiCad's `.kicad_sch`, whose units are also mm).
//! Pins sit on the 1.27 mm (50 mil) grid. A symbol is drawn from its library
//! definition ([`crate::symbols`]) mirrored left-right first when `mirror`,
//! then turned `rotation` degrees counter-clockwise as seen on the sheet.
//! (KiCad's `(mirror y)` is our `mirror`; its `(mirror x)` is `mirror` plus
//! 180°; its angles are the same counter-clockwise degrees.)
//!
//! # Connectivity
//!
//! - Items connect where an end of one lies on the other: a wire end on
//!   another wire's end or anywhere along it (a T), a pin's connection point
//!   on a wire or another pin, a label's or power port's anchor on a wire or
//!   pin.
//! - Wires that merely cross do not connect; a junction where they cross joins them.
//! - Local labels with the same text join within their sheet; global labels
//!   and power ports join across every sheet by text; a hierarchical label
//!   joins the same-named pin of its sheet's box on the parent sheet.
//! - A net is named, in order, by a power port, a global label, a local label
//!   (prefixed `sheet/` on every sheet but the first), a hierarchical label; else
//!   `Net-(R1-Pad2)` after its first pin in natural order. Two names on one net
//!   is a warning; the first by that order wins.
//!
//! Each symbol pin lands on footprint pads through its pin map (the library
//! pin's `pads`, or the instance's `pin_map`), so "Update board from
//! schematic" writes exactly those pads' nets.

use crate::symbols::{self, Dir, PinSpec, SymbolDef};
use crate::{bad, entity, natural};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub use crate::schematic_bus::{SchBus, SchBusEntry};
pub use crate::schematic_graphics::{SchImage, SchShape, SchTable, SchTextBox};
pub use crate::schematic_view::{SchematicView, resolve};

pub const GRID: f64 = 1.27;

// ---- Entities ----------------------------------------------------------------

/// Title block printed on every sheet.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TitleBlock {
    pub title: String,
    pub company: String,
    pub revision: String,
    pub date: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<String>,
}

/// A pin on a hierarchical sheet's box (on the parent sheet), joined to the
/// child's hierarchical label of the same name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SheetPin {
    pub name: String,
    /// Connection point on the parent sheet (on the box edge).
    pub at: [f64; 2],
    #[serde(default)]
    pub side: Dir,
}

/// One sheet of paper.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sheet {
    /// Stable within the schematic: "1", "2", …
    pub id: String,
    pub name: String,
    /// A4, A3, A2, A1, Letter, Tabloid (landscape unless `portrait`).
    #[serde(default = "a4")]
    pub paper: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub portrait: bool,
    /// Hierarchy: the sheet this one is a box on (None: a root sheet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The box on the parent sheet: top-left and size (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<SheetPin>,
    /// Another instance of a sheet's circuit: this sheet shows `instance_of`'s
    /// items (edit one and every instance follows) with its own references and nets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_of: Option<String>,
    /// References of the shared symbols in this instance (symbol id → reference).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub references: BTreeMap<String, String>,
    /// Page number printed in the title block (absent: its place in the list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

fn a4() -> String {
    "A4".into()
}

impl Sheet {
    pub fn paper_size(&self) -> [f64; 2] {
        let s = match self.paper.to_ascii_uppercase().as_str() {
            "A3" => [420.0, 297.0],
            "A2" => [594.0, 420.0],
            "A1" => [841.0, 594.0],
            "LETTER" | "A" => [279.4, 215.9],
            "TABLOID" | "B" => [431.8, 279.4],
            _ => [297.0, 210.0],
        };
        if self.portrait { [s[1], s[0]] } else { s }
    }
}

/// `pcb.schematic`: a schematic.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Schematic {
    pub name: String,
    /// The board "Update board from schematic" writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<StableId>,
    pub sheets: Vec<Sheet>,
    #[serde(default)]
    pub title: TitleBlock,
    /// Board components that belong on the board only (mounting holes,
    /// fiducials, logos), kept by "Update board" though no symbol has them.
    /// Parts without electrical pads are treated so without being listed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub board_only: Vec<String>,
    /// Bus aliases: a name standing for a group of members (`USB` → DP, DM, VBUS).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bus_aliases: BTreeMap<String, Vec<String>>,
    /// Net classes: colour and wire width on the sheet, track rules for the board.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub netclasses: Vec<SchNetClass>,
    /// A colour for one net's wires ("#d03030").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub net_colors: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "ErcSettings::is_default")]
    pub erc: ErcSettings,
    /// Fields every placed symbol gets (with a default value), and which are shown on the sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_templates: Vec<FieldTemplate>,
    /// Text variables `${NAME}` for notes, text boxes, tables and the drawing sheet.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub text_vars: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "DrawingSheet::is_default")]
    pub drawing_sheet: DrawingSheet,
}

/// A schematic net class.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SchNetClass {
    pub name: String,
    /// Wire colour on the sheet ("#2a5bd7").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Wire width on the sheet (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wire_width: Option<f64>,
    /// Board rules the class carries to the board (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clearance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_diameter: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_drill: Option<f64>,
    /// Nets in the class, by name or pattern (`USB_*`, `D?`); directive labels add more.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
}

/// Electrical-rules settings: severities, the pin-conflict matrix, exclusions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ErcSettings {
    /// Rule code → error, warning or ignore.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub severities: BTreeMap<String, String>,
    /// Pin-type pair ("output/open_collector") → error, warning or ok, over the default matrix.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub pin_matrix: BTreeMap<String, String>,
    /// Violations reviewed and accepted, by key.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclusions: Vec<ErcExclusion>,
}

impl ErcSettings {
    pub fn is_default(&self) -> bool {
        self == &ErcSettings::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ErcExclusion {
    pub key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

/// A field every placed symbol gets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FieldTemplate {
    pub name: String,
    /// Default value for new symbols.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// Drawn on the sheet under the value.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub visible: bool,
}

/// The drawing sheet (page layout): which standard parts show and extra items.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DrawingSheet {
    /// Leave out the standard title block (draw your own with `items`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_title_block: bool,
    /// Leave out the zone references round the border.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_zones: bool,
    /// Paper edge to frame (mm, default 8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<SheetItem>,
}

impl DrawingSheet {
    pub fn is_default(&self) -> bool {
        self == &DrawingSheet::default()
    }
}

/// A line, rectangle or text on every sheet, placed from a corner of the inner frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SheetItem {
    /// line, rect or text.
    pub kind: String,
    /// Offset from `corner`, measured inward (mm).
    pub at: [f64; 2],
    /// Other end (line, rect), from the same corner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<[f64; 2]>,
    /// rb (bottom right, default), lb, rt or lt.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub corner: String,
    /// Text; `${TITLE}`, `${REVISION}`, `${DATE}`, `${COMPANY}`, `${SHEETNAME}`, `${#}`, `${##}`, `${COMMENT1}` and text variables expand.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
}

impl Persist for Schematic {
    const KIND: &'static str = "pcb.schematic";
}

impl Schematic {
    pub fn sheet(&self, id: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|s| s.id == id)
    }
    /// How deep a sheet sits in the hierarchy (root sheets 0).
    pub fn depth(&self, id: &str) -> usize {
        let mut d = 0;
        let mut cur = self.sheet(id).and_then(|s| s.parent.clone());
        while let Some(p) = cur {
            d += 1;
            if d > 32 {
                break;
            }
            cur = self.sheet(&p).and_then(|s| s.parent.clone());
        }
        d
    }
    /// The page number of a sheet as printed.
    pub fn page(&self, id: &str) -> String {
        let k = self.sheets.iter().position(|s| s.id == id).unwrap_or(0);
        self.sheets.get(k).and_then(|s| s.page.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| (k + 1).to_string())
    }
    /// `${NAME}` text variables expanded (for a sheet, when given).
    pub fn expand_vars(&self, text: &str, sheet: Option<&str>) -> String {
        if !text.contains("${") {
            return text.to_string();
        }
        let sh = sheet.and_then(|id| self.sheet(id));
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find("${") {
            out.push_str(&rest[..i]);
            let tail = &rest[i + 2..];
            let Some(j) = tail.find('}') else {
                out.push_str(&rest[i..]);
                rest = "";
                break;
            };
            let name = &tail[..j];
            let up = name.to_ascii_uppercase();
            let v = match up.as_str() {
                "TITLE" => Some(if self.title.title.is_empty() { self.name.clone() } else { self.title.title.clone() }),
                "REVISION" | "REV" => Some(self.title.revision.clone()),
                "DATE" | "ISSUE_DATE" => Some(self.title.date.clone()),
                "COMPANY" => Some(self.title.company.clone()),
                "SHEETNAME" => sh.map(|s| s.name.clone()),
                "#" => sh.map(|s| self.page(&s.id)),
                "##" => Some(self.sheets.len().to_string()),
                "PAPER" => sh.map(|s| s.paper.clone()),
                _ if up.starts_with("COMMENT") => Some(up[7..].parse::<usize>().ok().and_then(|n| self.title.comments.get(n.saturating_sub(1)).cloned()).unwrap_or_default()),
                _ => self.text_vars.get(name).or_else(|| self.text_vars.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)).cloned(),
            };
            match v {
                Some(v) => out.push_str(&v),
                None => {
                    out.push_str("${");
                    out.push_str(name);
                    out.push('}');
                }
            }
            rest = &tail[j + 1..];
        }
        out.push_str(rest);
        out
    }
}

/// `pcb.sch_symbol`: a placed symbol (one unit of a part).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchSymbol {
    pub schematic: StableId,
    pub sheet: String,
    /// Library symbol name (`R`, `Conn_01x06`, `ESP32-WROOM-32E`), or `IC` with `pins`.
    pub symbol: String,
    pub reference: String,
    #[serde(default)]
    pub value: String,
    /// Footprint the board component gets ("" when none is chosen yet).
    #[serde(default)]
    pub footprint: String,
    pub at: [f64; 2],
    /// 0, 90, 180 or 270, counter-clockwise on the sheet.
    #[serde(default)]
    pub rotation: f64,
    /// Mirrored left-right (before turning).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    /// Which unit of a multi-unit part (1 = A).
    #[serde(default = "one_u")]
    pub unit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcsc: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dnp: bool,
    /// Symbol pin number → footprint pads, overriding the library's.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pin_map: BTreeMap<String, Vec<String>>,
    /// Pin table of a part the library lacks (symbol `IC`): drawn as a box.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<PinSpec>,
    /// Other fields ("Tolerance", "Voltage", "Datasheet").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    /// Where the reference and value texts sit, when moved by hand (sheet mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_at: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_at: Option<[f64; 2]>,
    /// Kept off the board (a test point drawn for documentation).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_from_board: bool,
    /// The board component it became (written by "Update board").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<StableId>,
    /// A library symbol from outside the built-in library (a KiCad library part, `Library:Name`),
    /// carried by the instance so the schematic stands on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<Box<SymbolDef>>,
}

fn one_u() -> u32 {
    1
}

impl Persist for SchSymbol {
    const KIND: &'static str = "pcb.sch_symbol";
}

impl SchSymbol {
    /// Its library definition (or its own pin table).
    pub fn def(&self) -> Option<SymbolDef> {
        if let Some(d) = &self.definition {
            return Some((**d).clone());
        }
        if !self.pins.is_empty() {
            return Some(symbols::custom(&self.symbol, &self.pins));
        }
        symbols::get(&self.symbol)
    }
    /// Footprint pads a pin lands on.
    pub fn pads_of(&self, pin: &symbols::PinDef) -> Vec<String> {
        self.pin_map.get(&pin.number).cloned().unwrap_or_else(|| pin.pads())
    }
    /// A local point of the symbol on the sheet.
    pub fn place(&self, p: [f64; 2]) -> [f64; 2] {
        xf(self.at, self.rotation, self.mirror, p)
    }
    pub fn dir(&self, d: Dir) -> Dir {
        let v = d.v();
        let o = xf([0.0, 0.0], self.rotation, self.mirror, v);
        Dir::from_v(o)
    }
}

/// A local point mirrored, turned and moved onto the sheet (y down, counter-clockwise).
pub fn xf(at: [f64; 2], rotation: f64, mirror: bool, p: [f64; 2]) -> [f64; 2] {
    let x = if mirror { -p[0] } else { p[0] };
    let y = p[1];
    let (s, c) = rotation.to_radians().sin_cos();
    let (s, c) = (round_unit(s), round_unit(c));
    [at[0] + x * c + y * s, at[1] - x * s + y * c]
}

fn round_unit(v: f64) -> f64 {
    let r = v.round();
    if (v - r).abs() < 1e-12 { r } else { v }
}

/// `pcb.sch_wire`: a wire.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchWire {
    pub schematic: StableId,
    pub sheet: String,
    pub points: Vec<[f64; 2]>,
}

impl Persist for SchWire {
    const KIND: &'static str = "pcb.sch_wire";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    /// Joins same-named local labels on its sheet.
    #[default]
    Local,
    /// Joins same-named global labels and power ports on every sheet.
    Global,
    /// Joins the same-named pin of this sheet's box on the parent sheet.
    Hierarchical,
    /// A power port (GND, 3V3, VBUS): global, drawn as a supply symbol.
    Power,
    /// A net-class directive: puts the net it sits on in the class its text names (names no net).
    Directive,
}

/// `pcb.sch_label`: a net label or power port.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchLabel {
    pub schematic: StableId,
    pub sheet: String,
    #[serde(default)]
    pub kind: LabelKind,
    /// The net name.
    pub text: String,
    /// Anchor (connection point).
    pub at: [f64; 2],
    /// Direction the text runs from the anchor: 0 right, 90 up, 180 left, 270
    /// down. A power port points its symbol this way (ground ports usually 270).
    #[serde(default)]
    pub rotation: f64,
    /// Global/hierarchical flag shape: input, output, bidirectional, tri_state, passive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// Power port drawing: ground, bar, arrow, circle (from the name when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

impl Persist for SchLabel {
    const KIND: &'static str = "pcb.sch_label";
}

/// `pcb.sch_junction`: a junction dot.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchJunction {
    pub schematic: StableId,
    pub sheet: String,
    pub at: [f64; 2],
}

impl Persist for SchJunction {
    const KIND: &'static str = "pcb.sch_junction";
}

/// `pcb.sch_no_connect`: a pin deliberately left unconnected.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchNoConnect {
    pub schematic: StableId,
    pub sheet: String,
    pub at: [f64; 2],
}

impl Persist for SchNoConnect {
    const KIND: &'static str = "pcb.sch_no_connect";
}

/// `pcb.sch_text`: a note on the sheet.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchText {
    pub schematic: StableId,
    pub sheet: String,
    pub at: [f64; 2],
    pub text: String,
    #[serde(default = "note_size")]
    pub size: f64,
    /// left (default), center or right.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub justify: String,
    /// 0, or 90 (reads bottom to top).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub angle: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

pub(crate) fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn note_size() -> f64 {
    1.8
}

impl Persist for SchText {
    const KIND: &'static str = "pcb.sch_text";
}

/// Derived on every schematic.
#[derive(Component, Clone)]
pub struct SchematicResult(pub Arc<SchematicView>);

// ---- Reading -----------------------------------------------------------------

/// Everything on a schematic.
#[derive(Default, Clone)]
pub struct SchItems {
    pub symbols: Vec<(StableId, SchSymbol)>,
    pub wires: Vec<(StableId, SchWire)>,
    pub labels: Vec<(StableId, SchLabel)>,
    pub junctions: Vec<(StableId, SchJunction)>,
    pub no_connects: Vec<(StableId, SchNoConnect)>,
    pub texts: Vec<(StableId, SchText)>,
    pub buses: Vec<(StableId, SchBus)>,
    pub bus_entries: Vec<(StableId, SchBusEntry)>,
    pub shapes: Vec<(StableId, SchShape)>,
    pub text_boxes: Vec<(StableId, SchTextBox)>,
    pub tables: Vec<(StableId, SchTable)>,
    pub images: Vec<(StableId, SchImage)>,
}

pub fn items(world: &World, schematic: StableId) -> SchItems {
    let mut out = SchItems::default();
    let Some(e) = entity(world, schematic) else { return out };
    let deps = world.get::<de_core::DependsOn>(e).map(|d| d.0.clone()).unwrap_or_default();
    for id in deps {
        let Some(x) = entity(world, id) else { continue };
        if let Some(c) = world.get::<SchSymbol>(x).filter(|c| c.schematic == schematic) {
            out.symbols.push((id, c.clone()));
        } else if let Some(w) = world.get::<SchWire>(x).filter(|w| w.schematic == schematic) {
            out.wires.push((id, w.clone()));
        } else if let Some(l) = world.get::<SchLabel>(x).filter(|l| l.schematic == schematic) {
            out.labels.push((id, l.clone()));
        } else if let Some(j) = world.get::<SchJunction>(x).filter(|j| j.schematic == schematic) {
            out.junctions.push((id, j.clone()));
        } else if let Some(n) = world.get::<SchNoConnect>(x).filter(|n| n.schematic == schematic) {
            out.no_connects.push((id, n.clone()));
        } else if let Some(t) = world.get::<SchText>(x).filter(|t| t.schematic == schematic) {
            out.texts.push((id, t.clone()));
        } else if let Some(t) = world.get::<SchBus>(x).filter(|t| t.schematic == schematic) {
            out.buses.push((id, t.clone()));
        } else if let Some(t) = world.get::<SchBusEntry>(x).filter(|t| t.schematic == schematic) {
            out.bus_entries.push((id, t.clone()));
        } else if let Some(t) = world.get::<SchShape>(x).filter(|t| t.schematic == schematic) {
            out.shapes.push((id, t.clone()));
        } else if let Some(t) = world.get::<SchTextBox>(x).filter(|t| t.schematic == schematic) {
            out.text_boxes.push((id, t.clone()));
        } else if let Some(t) = world.get::<SchTable>(x).filter(|t| t.schematic == schematic) {
            out.tables.push((id, t.clone()));
        } else if let Some(t) = world.get::<SchImage>(x).filter(|t| t.schematic == schematic) {
            out.images.push((id, t.clone()));
        }
    }
    out.symbols.sort_by(|a, b| natural(&a.1.reference, &b.1.reference).then(a.1.unit.cmp(&b.1.unit)));
    out
}

/// Every schematic in the project.
pub fn schematics(world: &World) -> Vec<(StableId, Schematic)> {
    let mut v: Vec<(StableId, Schematic)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Schematic>(e).map(|b| (id, b.clone()))).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// The schematic that drives a board, if any.
pub fn of_board(world: &World, board: StableId) -> Option<(StableId, Schematic)> {
    schematics(world).into_iter().find(|(_, s)| s.board == Some(board))
}

/// The derived view of a schematic (when worked out).
pub fn view(world: &World, id: StableId) -> Option<Arc<SchematicView>> {
    entity(world, id).and_then(|e| world.get::<SchematicResult>(e)).map(|r| r.0.clone())
}

pub(crate) fn compute(world: &World, e: Entity) -> Option<Result<SchematicResult, String>> {
    world.get::<Schematic>(e)?;
    let id = world.resource::<IdIndex>().iter().find(|(_, x)| *x == e).map(|(id, _)| id)?;
    Some(resolve(world, id).map(|v| SchematicResult(Arc::new(v))))
}

pub(crate) fn schematic_of(tx: &Tx, id: StableId) -> Result<Schematic, CommandError> {
    tx.get::<Schematic>(id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a schematic")))
}

pub(crate) fn sheet_or_first(s: &Schematic, sheet: Option<String>) -> Result<String, CommandError> {
    match sheet {
        Some(id) => s.sheet(&id).map(|x| x.id.clone()).or_else(|| s.sheets.iter().find(|x| x.name.eq_ignore_ascii_case(&id)).map(|x| x.id.clone())).ok_or_else(|| bad(format!("no sheet “{id}” (sheets: {})", s.sheets.iter().map(|x| format!("{} {}", x.id, x.name)).collect::<Vec<_>>().join(", ")))),
        None => Ok(s.sheets.first().map(|x| x.id.clone()).unwrap_or_else(|| "1".into())),
    }
}

pub fn snap(p: [f64; 2]) -> [f64; 2] {
    [snap1(p[0]), snap1(p[1])]
}

pub fn snap1(v: f64) -> f64 {
    let r = (v / GRID).round() * GRID;
    (r * 1e4).round() / 1e4
}

pub(crate) fn check_xy(p: [f64; 2], what: &str) -> Result<(), CommandError> {
    if p.iter().any(|v| !v.is_finite() || v.abs() > 5000.0) {
        return Err(bad(format!("{what}: coordinates are numbers within ±5000 mm")));
    }
    Ok(())
}

pub(crate) fn add_item<T: Persist>(tx: &mut Tx, schematic: StableId, value: T) -> Result<StableId, CommandError> {
    let id = tx.create();
    tx.set(id, value)?;
    tx.add_dependency(schematic, id)?;
    Ok(id)
}

/// A pin of a placed symbol, found by "U1.3" or "U1.EN".
#[derive(Clone, Debug)]
pub struct PinRef {
    pub symbol: StableId,
    pub sheet: String,
    pub at: [f64; 2],
    /// Direction a wire leaves it.
    pub dir: Dir,
    pub number: String,
    pub name: String,
}

pub fn find_pin(it: &SchItems, text: &str) -> Result<PinRef, CommandError> {
    let (r, p) = text.rsplit_once('.').ok_or_else(|| bad(format!("“{text}”: write pins as REF.PIN (U1.3 or U1.EN)")))?;
    let mut found: Vec<PinRef> = Vec::new();
    let mut any_ref = false;
    for (id, s) in &it.symbols {
        if !s.reference.eq_ignore_ascii_case(r.trim()) {
            continue;
        }
        any_ref = true;
        let Some(def) = s.def() else { continue };
        for pin in def.unit_pins(s.unit).filter(|x| !x.hidden) {
            if pin.number == p.trim() {
                return Ok(pin_ref(*id, s, pin));
            }
        }
        for pin in def.unit_pins(s.unit).filter(|x| !x.hidden) {
            if pin.name.eq_ignore_ascii_case(p.trim()) {
                found.push(pin_ref(*id, s, pin));
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 if !any_ref => Err(bad(format!("{text}: no symbol {r} on the schematic"))),
        0 => Err(bad(format!("{text}: {r} has no pin “{p}” (in its placed units)"))),
        _ => Err(bad(format!("{text}: {r} has several pins named {p}; use the pin number"))),
    }
}

fn pin_ref(id: StableId, s: &SchSymbol, pin: &symbols::PinDef) -> PinRef {
    PinRef { symbol: id, sheet: s.sheet.clone(), at: s.place(pin.at), dir: s.dir(pin.side), number: pin.number.clone(), name: pin.name.clone() }
}

/// Next free number for a reference prefix ("R" → "R8").
pub fn next_reference(it: &SchItems, prefix: &str, taken_extra: &[String]) -> String {
    let mut n = 1;
    loop {
        let r = format!("{prefix}{n}");
        if !it.symbols.iter().any(|(_, s)| s.reference.eq_ignore_ascii_case(&r)) && !taken_extra.iter().any(|t| t.eq_ignore_ascii_case(&r)) {
            return r;
        }
        n += 1;
    }
}

/// A net name that is a supply (drawn as a power port).
pub fn is_power_net(name: &str) -> bool {
    let n = name.trim().trim_start_matches('+').to_ascii_uppercase();
    n.starts_with("GND") || n.ends_with("GND") || n == "VSS" || n == "VBUS" || n == "VCC" || n == "VDD" || n == "VIN" || n == "VBAT" || n == "VSYS" || n == "V+" || n == "V-" || n == "VEE" || n.starts_with("VCC_") || n.starts_with("VDD_") || {
        // 3V3, 5V, 12V, 1V8, 3.3V
        let t = n.trim_start_matches('V');
        !n.is_empty() && n.chars().next().unwrap().is_ascii_digit() && (n.contains('V')) && t.chars().all(|c| c.is_ascii_digit() || c == 'V' || c == '.')
    }
}

pub fn is_ground(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.contains("GND") || n == "VSS" || n == "0V"
}

// ---- Commands ------------------------------------------------------------------

/// `pcb.schematic.create`: a schematic (one sheet), optionally driving a board.
#[derive(Deserialize, JsonSchema)]
pub struct CreateSchematic {
    pub name: Option<String>,
    pub board: Option<StableId>,
    /// A4 (default), A3, A2, Letter, Tabloid.
    pub paper: Option<String>,
    pub title: Option<String>,
    pub company: Option<String>,
    pub revision: Option<String>,
    pub date: Option<String>,
}

impl Command for CreateSchematic {
    const NAME: &'static str = "pcb.schematic.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut name = self.name.unwrap_or_default().trim().to_string();
        if let Some(b) = self.board {
            let board = tx.get::<crate::Board>(b).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{b} is not a board")))?;
            if name.is_empty() {
                name = board.name.clone();
            }
            if let Some((other, _)) = of_board(tx.world(), b) {
                return Err(bad(format!("the board already has a schematic ({other})")));
            }
        }
        if name.is_empty() {
            name = "Schematic".into();
        }
        let s = Schematic {
            name: name.clone(),
            board: self.board,
            sheets: vec![Sheet { id: "1".into(), name: "Root".into(), paper: self.paper.unwrap_or_else(a4), ..Default::default() }],
            title: TitleBlock { title: self.title.unwrap_or(name), company: self.company.unwrap_or_default(), revision: self.revision.unwrap_or_else(|| "A".into()), date: self.date.unwrap_or_default(), comments: vec![] },
            ..Default::default()
        };
        let id = tx.create();
        tx.set(id, s)?;
        Ok(json!({ "id": id, "sheet": "1" }))
    }
}

/// `pcb.schematic.update`: rename, relink to a board, change the title block or board-only list.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateSchematic {
    pub target: StableId,
    pub name: Option<String>,
    /// A board to drive; null unlinks.
    #[serde(default, deserialize_with = "double_option")]
    #[schemars(with = "Option<StableId>")]
    pub board: Option<Option<StableId>>,
    pub title: Option<TitleBlock>,
    pub board_only: Option<Vec<String>>,
    /// Bus aliases, replacing the list: { "USB": ["DP", "DM", "VBUS"] }.
    pub bus_aliases: Option<BTreeMap<String, Vec<String>>>,
    /// Field templates, replacing the list.
    pub field_templates: Option<Vec<FieldTemplate>>,
    /// Text variables, replacing the list.
    pub text_vars: Option<BTreeMap<String, String>>,
    pub drawing_sheet: Option<DrawingSheet>,
}

fn double_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

impl Command for UpdateSchematic {
    const NAME: &'static str = "pcb.schematic.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = schematic_of(tx, self.target)?;
        if let Some(n) = self.name {
            if n.trim().is_empty() {
                return Err(bad("a schematic needs a name"));
            }
            s.name = n.trim().into();
        }
        if let Some(b) = self.board {
            if let Some(b) = b {
                if tx.get::<crate::Board>(b).is_none() {
                    return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{b} is not a board")));
                }
                if let Some((other, _)) = of_board(tx.world(), b).filter(|(o, _)| *o != self.target) {
                    return Err(bad(format!("the board already has a schematic ({other})")));
                }
            }
            s.board = b;
        }
        if let Some(t) = self.title {
            s.title = t;
        }
        if let Some(b) = self.board_only {
            s.board_only = b.into_iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
        }
        if let Some(a) = self.bus_aliases {
            let mut out = BTreeMap::new();
            for (k, v) in a {
                let k = k.trim().to_string();
                if k.is_empty() || k.contains(|c: char| c.is_whitespace() || "{}[]".contains(c)) {
                    return Err(bad(format!("“{k}” is not a bus alias name (letters, digits, _ and -)")));
                }
                let members: Vec<String> = v.iter().map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect();
                if members.is_empty() {
                    return Err(bad(format!("bus alias {k} has no members")));
                }
                out.insert(k, members);
            }
            s.bus_aliases = out;
        }
        if let Some(f) = self.field_templates {
            let mut seen = std::collections::BTreeSet::new();
            s.field_templates = Vec::new();
            for t in f {
                let name = t.name.trim().to_string();
                if name.is_empty() || !seen.insert(name.to_ascii_lowercase()) {
                    return Err(bad(format!("field template names are distinct and not empty (“{name}”)")));
                }
                s.field_templates.push(FieldTemplate { name, ..t });
            }
        }
        if let Some(v) = self.text_vars {
            s.text_vars = v.into_iter().map(|(k, v)| (k.trim().to_string(), v)).filter(|(k, _)| !k.is_empty()).collect();
        }
        if let Some(d) = self.drawing_sheet {
            for it in &d.items {
                if !["line", "rect", "text"].contains(&it.kind.as_str()) {
                    return Err(bad("drawing-sheet items are line, rect or text"));
                }
                if !["", "rb", "lb", "rt", "lt"].contains(&it.corner.as_str()) {
                    return Err(bad("a drawing-sheet item's corner is rb, lb, rt or lt"));
                }
            }
            s.drawing_sheet = d;
        }
        tx.set(self.target, s)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.schematic.sheet`: add a sheet (with `parent`, a hierarchical sheet
/// drawn as a box on the parent), or change one (`id`); `delete` removes an empty sheet.
#[derive(Deserialize, JsonSchema)]
pub struct SheetCommand {
    pub schematic: StableId,
    pub id: Option<String>,
    pub name: Option<String>,
    pub paper: Option<String>,
    pub portrait: Option<bool>,
    pub parent: Option<String>,
    /// The box on the parent: top-left and size.
    pub at: Option<[f64; 2]>,
    pub size: Option<[f64; 2]>,
    pub pins: Option<Vec<SheetPin>>,
    /// Show another sheet's circuit (another instance of it): its items, with this sheet's own references and nets.
    pub instance_of: Option<String>,
    /// Page number for the title block ("" clears).
    pub page: Option<String>,
    /// Make the box's pins match the hierarchical labels inside: add the missing ones, drop those with no label.
    #[serde(default)]
    pub sync_pins: bool,
    #[serde(default)]
    pub delete: bool,
}

impl Command for SheetCommand {
    const NAME: &'static str = "pcb.schematic.sheet";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = schematic_of(tx, self.schematic)?;
        if let Some(p) = &self.paper {
            if !["A4", "A3", "A2", "A1", "LETTER", "TABLOID", "A", "B"].contains(&p.to_ascii_uppercase().as_str()) {
                return Err(bad("paper is A4, A3, A2, A1, Letter or Tabloid"));
            }
        }
        if let Some(parent) = &self.parent {
            if s.sheet(parent).is_none() {
                return Err(bad(format!("no parent sheet {parent}")));
            }
        }
        let id = match &self.id {
            Some(id) => {
                let i = s.sheets.iter().position(|x| &x.id == id).ok_or_else(|| bad(format!("no sheet {id}")))?;
                if self.delete {
                    let it = items(tx.world(), self.schematic);
                    let used = it.symbols.iter().any(|(_, x)| &x.sheet == id) || it.wires.iter().any(|(_, x)| &x.sheet == id) || it.labels.iter().any(|(_, x)| &x.sheet == id);
                    if used || s.sheets.len() == 1 || s.sheets.iter().any(|x| x.parent.as_deref() == Some(id)) {
                        return Err(bad("only an empty sheet with no sub-sheets can be deleted (and not the last one)"));
                    }
                    s.sheets.remove(i);
                    tx.set(self.schematic, s)?;
                    return Ok(json!({ "deleted": id }));
                }
                let sh = &mut s.sheets[i];
                if let Some(v) = self.name {
                    sh.name = v;
                }
                if let Some(v) = self.paper {
                    sh.paper = v;
                }
                if let Some(v) = self.portrait {
                    sh.portrait = v;
                }
                if self.parent.is_some() {
                    if self.parent.as_deref() == Some(id.as_str()) {
                        return Err(bad("a sheet cannot be its own parent"));
                    }
                    sh.parent = self.parent;
                }
                if self.at.is_some() {
                    sh.at = self.at;
                }
                if self.size.is_some() {
                    sh.size = self.size;
                }
                if let Some(p) = self.pins {
                    sh.pins = p.into_iter().map(|p| SheetPin { at: snap(p.at), ..p }).collect();
                }
                if let Some(p) = self.page {
                    sh.page = Some(p.trim().to_string()).filter(|p| !p.is_empty());
                }
                if let Some(src) = &self.instance_of {
                    sh.instance_of = if src.is_empty() { None } else { Some(src.clone()) };
                }
                id.clone()
            }
            None => {
                let n = s.sheets.iter().filter_map(|x| x.id.parse::<u32>().ok()).max().unwrap_or(0) + 1;
                let id = n.to_string();
                s.sheets.push(Sheet {
                    id: id.clone(),
                    name: self.name.unwrap_or_else(|| format!("Sheet {n}")),
                    paper: self.paper.unwrap_or_else(a4),
                    portrait: self.portrait.unwrap_or(false),
                    parent: self.parent.clone(),
                    at: self.at.or(self.parent.as_ref().map(|_| [25.4, 25.4])),
                    size: self.size.or(self.parent.as_ref().map(|_| [30.48, 20.32])),
                    pins: self.pins.unwrap_or_default().into_iter().map(|p| SheetPin { at: snap(p.at), ..p }).collect(),
                    instance_of: self.instance_of.clone().filter(|x| !x.is_empty()),
                    references: BTreeMap::new(),
                    page: self.page.clone().filter(|p| !p.trim().is_empty()),
                });
                id
            }
        };
        // An instance shows a real sheet (not itself, not another instance) that holds no sub-sheets.
        if let Some(src) = s.sheet(&id).and_then(|x| x.instance_of.clone()) {
            let Some(source) = s.sheet(&src) else { return Err(bad(format!("no sheet {src} to show another instance of"))) };
            if src == id || source.instance_of.is_some() {
                return Err(bad("an instance shows a sheet that is not itself an instance"));
            }
            if s.sheets.iter().any(|x| x.parent.as_deref() == Some(src.as_str())) {
                return Err(bad(format!("sheet {} holds sub-sheets: only a sheet without sub-sheets can have several instances", source.name)));
            }
            if s.sheets.iter().any(|x| x.parent.as_deref() == Some(id.as_str())) {
                return Err(bad("an instance sheet cannot hold sub-sheets of its own"));
            }
            let it = items(tx.world(), self.schematic);
            if it.symbols.iter().any(|(_, x)| x.sheet == id) || it.wires.iter().any(|(_, x)| x.sheet == id) || it.labels.iter().any(|(_, x)| x.sheet == id) {
                return Err(bad("only an empty sheet can become an instance of another"));
            }
        }
        if self.sync_pins {
            let it = items(tx.world(), self.schematic);
            sync_sheet_pins(&mut s, &id, &it)?;
        }
        tx.set(self.schematic, s)?;
        Ok(json!({ "schematic": self.schematic, "sheet": id }))
    }
}

/// Makes a sheet box's pins match the hierarchical labels on the sheet it shows:
/// new pins go down the left edge (inputs, bidirectional, passive) or the right
/// (outputs), below those already there; pins with no label go.
pub fn sync_sheet_pins(s: &mut Schematic, id: &str, it: &SchItems) -> Result<Vec<String>, CommandError> {
    let src = s.sheet(id).and_then(|x| x.instance_of.clone()).unwrap_or_else(|| id.to_string());
    let sh = s.sheets.iter_mut().find(|x| x.id == id).ok_or_else(|| bad(format!("no sheet {id}")))?;
    if sh.parent.is_none() {
        return Err(bad("a root sheet has no box to put pins on"));
    }
    let at = sh.at.unwrap_or([25.4, 25.4]);
    let size = sh.size.unwrap_or([30.48, 20.32]);
    let mut labels: Vec<(String, Option<String>, f64)> = Vec::new();
    for (_, l) in it.labels.iter().filter(|(_, l)| l.sheet == src && l.kind == LabelKind::Hierarchical) {
        if !labels.iter().any(|(n, _, _)| n == &l.text) {
            labels.push((l.text.clone(), l.shape.clone(), l.at[1]));
        }
    }
    labels.sort_by(|a, b| a.2.total_cmp(&b.2));
    let mut added = Vec::new();
    sh.pins.retain(|p| labels.iter().any(|(n, _, _)| n == &p.name));
    for (name, shape, _) in &labels {
        if sh.pins.iter().any(|p| &p.name == name) {
            continue;
        }
        let right = shape.as_deref() == Some("output");
        let x = if right { at[0] + size[0] } else { at[0] };
        let side = if right { Dir::Right } else { Dir::Left };
        let mut y = snap1(at[1] + 2.54);
        while sh.pins.iter().any(|p| (p.at[1] - y).abs() < 1e-3 && (p.at[0] - x).abs() < 1e-3) {
            y = snap1(y + 2.54);
        }
        sh.pins.push(SheetPin { name: name.clone(), at: snap([x, y]), side });
        added.push(name.clone());
    }
    // Grow the box to hold its pins.
    let lowest = sh.pins.iter().map(|p| p.at[1]).fold(at[1], f64::max);
    if lowest + 2.54 > at[1] + size[1] {
        sh.size = Some([size[0], snap1(lowest + 2.54 - at[1])]);
    }
    Ok(added)
}

/// `pcb.schematic.place`: place a symbol. The reference defaults to the
/// symbol's prefix and the next free number; value, footprint, MPN, maker and
/// LCSC default to the library symbol's.
#[derive(Deserialize, JsonSchema)]
pub struct PlaceSymbol {
    pub schematic: StableId,
    pub sheet: Option<String>,
    /// Library name (GET /pcb/symbols), or "IC" with `pins`.
    pub symbol: String,
    pub reference: Option<String>,
    pub value: Option<String>,
    pub footprint: Option<String>,
    /// Symbol centre on the sheet (mm, y down), snapped to the 1.27 mm grid.
    pub at: [f64; 2],
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub mirror: bool,
    pub unit: Option<u32>,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dnp: bool,
    #[serde(default)]
    pub pin_map: BTreeMap<String, Vec<String>>,
    /// Pin table for a part the library lacks: [{ number, name, type, side?, pads? }].
    #[serde(default)]
    pub pins: Vec<PinSpec>,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub exclude_from_board: bool,
    /// The symbol itself when it is not in the built-in library: a KiCad library symbol as
    /// GET /pcb/kicad/symbol/<Library:Name> returns it (`./forma pcb add` does all of this).
    pub definition: Option<SymbolDef>,
    /// Its footprint, defined on the schematic's board at the same time (as GET
    /// /pcb/kicad/footprint/<Library:Name> returns it; kept if the board has it already).
    pub footprint_definition: Option<crate::footprints::Footprint>,
}

pub(crate) fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub(crate) fn check_rotation(r: f64) -> Result<f64, CommandError> {
    let r = r.rem_euclid(360.0);
    let q = (r / 90.0).round() * 90.0;
    if (r - q).abs() > 1e-6 {
        return Err(bad("symbols turn in quarter turns: 0, 90, 180 or 270"));
    }
    Ok(q.rem_euclid(360.0))
}

pub(crate) fn check_reference(r: &str) -> Result<(), CommandError> {
    if r.is_empty() || !r.starts_with(|c: char| c.is_ascii_alphabetic() || c == '#') || r.len() > 16 || r.contains(char::is_whitespace) || r.contains('.') {
        return Err(bad(format!("“{r}” is not a reference designator (R1, C3, U1; R? to annotate later)")));
    }
    Ok(())
}

impl Command for PlaceSymbol {
    const NAME: &'static str = "pcb.schematic.place";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let sheet = sheet_or_first(&s, self.sheet)?;
        check_xy(self.at, "at")?;
        if let Some(d) = &self.definition {
            if d.pins.is_empty() && !d.virtual_part {
                return Err(bad(format!("the definition of {} has no pins", d.name)));
            }
            if d.name != self.symbol.trim() {
                return Err(bad(format!("`symbol` is “{}” but the definition is {}", self.symbol.trim(), d.name)));
            }
        }
        let fp_name = self.footprint_definition.as_ref().map(|f| f.name.clone());
        if let Some(fp) = self.footprint_definition {
            let board = s.board.ok_or_else(|| bad("the schematic drives no board, so there is nowhere to define the footprint (pcb.footprint.define on a board)"))?;
            let mut b = crate::board_of(tx, board)?;
            if !b.footprints.contains_key(&fp.name) {
                crate::check_footprint(&fp.name, &fp)?;
                b.footprints.insert(fp.name.clone(), fp);
                tx.set(board, b)?;
                crate::model3d::sync(tx, board)?;
            }
        }
        let mut sym = SchSymbol {
            schematic: self.schematic,
            sheet,
            symbol: self.symbol.trim().to_string(),
            reference: String::new(),
            value: String::new(),
            footprint: String::new(),
            at: snap(self.at),
            rotation: check_rotation(self.rotation)?,
            mirror: self.mirror,
            unit: self.unit.unwrap_or(1).max(1),
            mpn: None,
            manufacturer: None,
            lcsc: None,
            description: self.description.trim().to_string(),
            dnp: self.dnp,
            pin_map: self.pin_map,
            pins: self.pins,
            fields: self.fields,
            ref_at: None,
            value_at: None,
            exclude_from_board: self.exclude_from_board,
            component: None,
            definition: self.definition.map(Box::new),
        };
        if sym.definition.is_none() && sym.pins.is_empty() && symbols::get(&sym.symbol).is_none() {
            // Another of the same library part already on the schematic lends its definition.
            let it = items(tx.world(), self.schematic);
            sym.definition = it.symbols.iter().find(|(_, o)| o.symbol == sym.symbol && o.definition.is_some()).and_then(|(_, o)| o.definition.clone());
        }
        let def = sym.def().ok_or_else(|| bad(format!("no symbol “{}” (GET /pcb/symbols lists them; a KiCad library part needs its `definition`, which `./forma pcb add` fetches; for another part give `pins`, a pin table)", sym.symbol)))?;
        if sym.unit > def.unit_count() {
            return Err(bad(format!("{} has {} unit(s)", def.name, def.unit_count())));
        }
        let it = items(tx.world(), self.schematic);
        sym.reference = match clean(self.reference) {
            Some(r) => r,
            None => next_reference(&it, &def.prefix, &[]),
        };
        check_reference(&sym.reference)?;
        if !sym.reference.contains('?') {
            if let Some((other, _)) = it.symbols.iter().find(|(_, o)| o.reference.eq_ignore_ascii_case(&sym.reference) && (o.symbol != sym.symbol || o.unit == sym.unit)) {
                return Err(bad(format!("{} is already on the schematic ({other}); a part's other units share its reference", sym.reference)));
            }
        }
        sym.value = clean(self.value).unwrap_or(def.value.clone());
        sym.footprint = clean(self.footprint).or(fp_name).unwrap_or(def.footprint.clone());
        sym.mpn = clean(self.mpn).or(def.mpn.clone());
        sym.manufacturer = clean(self.manufacturer).or(def.manufacturer.clone());
        sym.lcsc = clean(self.lcsc).or(def.lcsc.clone());
        for t in &s.field_templates {
            sym.fields.entry(t.name.clone()).or_insert_with(|| t.value.clone());
        }
        let reference = sym.reference.clone();
        let id = add_item(tx, self.schematic, sym)?;
        Ok(json!({ "id": id, "reference": reference }))
    }
}

/// `pcb.schematic.symbol.update`: move, turn, mirror, re-reference or
/// re-value a symbol (what is left out stays). Moving drags the wires
/// attached to its pins along (`drag: false` leaves them).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateSymbol {
    pub target: StableId,
    pub reference: Option<String>,
    pub value: Option<String>,
    pub footprint: Option<String>,
    pub symbol: Option<String>,
    pub at: Option<[f64; 2]>,
    pub rotation: Option<f64>,
    pub mirror: Option<bool>,
    pub unit: Option<u32>,
    pub sheet: Option<String>,
    /// "" clears.
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    pub description: Option<String>,
    pub dnp: Option<bool>,
    pub pin_map: Option<BTreeMap<String, Vec<String>>>,
    pub pins: Option<Vec<PinSpec>>,
    pub fields: Option<BTreeMap<String, String>>,
    pub ref_at: Option<[f64; 2]>,
    pub value_at: Option<[f64; 2]>,
    pub exclude_from_board: Option<bool>,
    /// The instance sheet whose reference `reference` sets (a symbol on a sheet shown more than once).
    pub instance: Option<String>,
    #[serde(default = "yes")]
    pub drag: bool,
}

fn yes() -> bool {
    true
}

/// Pin connection points of a symbol on its sheet.
pub fn pin_points(s: &SchSymbol) -> Vec<[f64; 2]> {
    s.def().map(|d| d.unit_pins(s.unit).filter(|p| !p.hidden).map(|p| s.place(p.at)).collect()).unwrap_or_default()
}

pub fn same(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

/// Wires and labels attached at `old` points follow them to `new` points, staying orthogonal.
pub fn drag_attached(tx: &mut Tx, schematic: StableId, sheet: &str, moves: &[([f64; 2], [f64; 2])], skip: &[StableId]) -> Result<(), CommandError> {
    if moves.iter().all(|(a, b)| same(*a, *b)) {
        return Ok(());
    }
    let it = items(tx.world(), schematic);
    for (id, w) in it.wires.iter().filter(|(id, w)| w.sheet == sheet && !skip.contains(id)) {
        let n = w.points.len();
        let first = moves.iter().find(|(a, _)| same(*a, w.points[0])).map(|(a, b)| [b[0] - a[0], b[1] - a[1]]);
        let last = moves.iter().find(|(a, _)| same(*a, w.points[n - 1])).map(|(a, b)| [b[0] - a[0], b[1] - a[1]]);
        if first.is_none() && last.is_none() {
            continue;
        }
        let mut pts = w.points.clone();
        if let (Some(d0), Some(d1)) = (first, last) {
            if same(d0, d1) {
                for p in &mut pts {
                    *p = [p[0] + d0[0], p[1] + d0[1]];
                }
                tx.set(*id, SchWire { points: pts, ..w.clone() })?;
                continue;
            }
        }
        if let Some(d) = first {
            pts.reverse();
            drag_end(&mut pts, d);
            pts.reverse();
        }
        if let Some(d) = last {
            drag_end(&mut pts, d);
        }
        let pts = simplify(&pts);
        if pts.len() >= 2 {
            tx.set(*id, SchWire { points: pts, ..w.clone() })?;
        }
    }
    for (id, l) in it.labels.iter().filter(|(id, l)| l.sheet == sheet && !skip.contains(id)) {
        if let Some((_, b)) = moves.iter().find(|(a, _)| same(*a, l.at)) {
            tx.set(*id, SchLabel { at: *b, ..l.clone() })?;
        }
    }
    for (id, n) in it.no_connects.iter().filter(|(id, n)| n.sheet == sheet && !skip.contains(id)) {
        if let Some((_, b)) = moves.iter().find(|(a, _)| same(*a, n.at)) {
            tx.set(*id, SchNoConnect { at: *b, ..n.clone() })?;
        }
    }
    Ok(())
}

/// Moves the last point by `d`, bending the wire so it stays orthogonal.
fn drag_end(pts: &mut Vec<[f64; 2]>, d: [f64; 2]) {
    let n = pts.len();
    let e = pts[n - 1];
    let ne = [e[0] + d[0], e[1] + d[1]];
    let prev = pts[n - 2];
    let horizontal = (prev[1] - e[1]).abs() < 1e-6;
    if n == 2 {
        pts[1] = ne;
        if (prev[0] - ne[0]).abs() > 1e-6 && (prev[1] - ne[1]).abs() > 1e-6 {
            let corner = if horizontal { [prev[0], ne[1]] } else { [ne[0], prev[1]] };
            pts.insert(1, corner);
        }
        return;
    }
    // Slide the neighbouring corner with the end, then mend the segment before it.
    let mut np = prev;
    if horizontal {
        np[1] = ne[1];
    } else {
        np[0] = ne[0];
    }
    pts[n - 1] = ne;
    pts[n - 2] = np;
    let before = pts[n - 3];
    if (before[0] - np[0]).abs() > 1e-6 && (before[1] - np[1]).abs() > 1e-6 {
        let corner = if horizontal { [before[0], np[1]] } else { [np[0], before[1]] };
        // Keep the old corner as the joint: before → old prev → new prev.
        pts.insert(n - 2, if (before[1] - prev[1]).abs() < 1e-6 || (before[0] - prev[0]).abs() < 1e-6 { prev } else { corner });
    }
}

/// Drops repeated and collinear middle points.
pub fn simplify(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    for p in pts {
        if out.last().is_some_and(|q| same(*q, *p)) {
            continue;
        }
        while out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
            let dot = (b[0] - a[0]) * (p[0] - b[0]) + (b[1] - a[1]) * (p[1] - b[1]);
            if cross.abs() < 1e-6 && dot >= -1e-9 {
                out.pop();
            } else {
                break;
            }
        }
        out.push(*p);
    }
    out
}

impl Command for UpdateSymbol {
    const NAME: &'static str = "pcb.schematic.symbol.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let old = tx.get::<SchSymbol>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a schematic symbol", self.target)))?;
        let mut s = schematic_of(tx, old.schematic)?;
        let mut c = old.clone();
        if let (Some(inst), Some(v)) = (self.instance.as_ref().filter(|i| **i != old.sheet), &self.reference) {
            // The reference in another instance of the sheet.
            let r = v.trim().to_string();
            check_reference(&r)?;
            let sh = s.sheets.iter_mut().find(|x| &x.id == inst && x.instance_of.as_deref() == Some(old.sheet.as_str())).ok_or_else(|| bad(format!("sheet {inst} is not an instance of sheet {}", old.sheet)))?;
            sh.references.insert(self.target.to_string(), r.clone());
            tx.set(old.schematic, s)?;
            return Ok(json!({ "id": self.target, "reference": r, "instance": inst }));
        }
        if let Some(v) = self.reference {
            c.reference = v.trim().to_string();
            check_reference(&c.reference)?;
        }
        if let Some(v) = self.value {
            c.value = v.trim().to_string();
        }
        if let Some(v) = self.footprint {
            c.footprint = v.trim().to_string();
        }
        if let Some(v) = self.symbol {
            c.symbol = v.trim().to_string();
        }
        if let Some(v) = self.at {
            check_xy(v, "at")?;
            let d = [v[0] - old.at[0], v[1] - old.at[1]];
            c.at = snap(v);
            // Hand-placed field texts travel with it.
            c.ref_at = c.ref_at.map(|p| [p[0] + d[0], p[1] + d[1]]);
            c.value_at = c.value_at.map(|p| [p[0] + d[0], p[1] + d[1]]);
        }
        if let Some(v) = self.rotation {
            c.rotation = check_rotation(v)?;
        }
        if let Some(v) = self.mirror {
            c.mirror = v;
        }
        if let Some(v) = self.unit {
            c.unit = v.max(1);
        }
        if let Some(v) = self.sheet {
            c.sheet = sheet_or_first(&s, Some(v))?;
        }
        if self.mpn.is_some() {
            c.mpn = clean(self.mpn);
        }
        if self.manufacturer.is_some() {
            c.manufacturer = clean(self.manufacturer);
        }
        if self.lcsc.is_some() {
            c.lcsc = clean(self.lcsc);
        }
        if let Some(v) = self.description {
            c.description = v.trim().into();
        }
        if let Some(v) = self.dnp {
            c.dnp = v;
        }
        if let Some(v) = self.pin_map {
            c.pin_map = v;
        }
        if let Some(v) = self.pins {
            c.pins = v;
        }
        if let Some(v) = self.fields {
            c.fields = v;
        }
        if self.ref_at.is_some() {
            c.ref_at = self.ref_at;
        }
        if self.value_at.is_some() {
            c.value_at = self.value_at;
        }
        if let Some(v) = self.exclude_from_board {
            c.exclude_from_board = v;
        }
        let def = c.def().ok_or_else(|| bad(format!("no symbol “{}”", c.symbol)))?;
        if c.unit > def.unit_count() {
            return Err(bad(format!("{} has {} unit(s)", def.name, def.unit_count())));
        }
        let it = items(tx.world(), old.schematic);
        if !c.reference.contains('?') {
            if let Some((other, _)) = it.symbols.iter().find(|(oid, o)| *oid != self.target && o.reference.eq_ignore_ascii_case(&c.reference) && (o.symbol != c.symbol || o.unit == c.unit)) {
                return Err(bad(format!("{} is already on the schematic ({other})", c.reference)));
            }
        }
        // Other units of the part follow a new reference.
        if c.reference != old.reference && !old.reference.contains('?') {
            for (oid, o) in it.symbols.iter().filter(|(oid, o)| *oid != self.target && o.reference == old.reference && o.symbol == old.symbol) {
                tx.set(*oid, SchSymbol { reference: c.reference.clone(), ..o.clone() })?;
            }
        }
        if self.drag && c.sheet == old.sheet {
            let before = pin_points(&old);
            let after = pin_points(&c);
            if before.len() == after.len() {
                let moves: Vec<_> = before.into_iter().zip(after).collect();
                drag_attached(tx, old.schematic, &old.sheet, &moves, &[])?;
            }
        }
        tx.set(self.target, c)?;
        Ok(json!({ "id": self.target }))
    }
}

/// An end of a wire to draw: a pin ("U1.3", "U1.EN") or a point.
#[derive(Deserialize, JsonSchema, Clone)]
#[serde(untagged)]
pub enum End {
    Pin(String),
    Point([f64; 2]),
}

/// `pcb.schematic.wire`: a wire through `points`, or routed orthogonally
/// from `from` to `to` (each a pin "U1.3" or a point), around symbols and
/// without touching other pins.
#[derive(Deserialize, JsonSchema)]
pub struct CreateWire {
    pub schematic: StableId,
    pub sheet: Option<String>,
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
    pub from: Option<End>,
    pub to: Option<End>,
}

impl Command for CreateWire {
    const NAME: &'static str = "pcb.schematic.wire";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let mut sheet = sheet_or_first(&s, self.sheet.clone())?;
        let points = if let (Some(a), Some(b)) = (self.from.clone(), self.to.clone()) {
            let it = items(tx.world(), self.schematic);
            let end = |e: End| -> Result<([f64; 2], Option<Dir>, Option<String>), CommandError> {
                match e {
                    End::Pin(p) => {
                        let r = find_pin(&it, &p)?;
                        Ok((r.at, Some(r.dir), Some(r.sheet)))
                    }
                    End::Point(p) => Ok((snap(p), None, None)),
                }
            };
            let (pa, da, sa) = end(a)?;
            let (pb, db, sb) = end(b)?;
            if let (Some(x), Some(y)) = (&sa, &sb) {
                if x != y {
                    return Err(bad("those pins are on different sheets: join them with global or hierarchical labels"));
                }
            }
            if let Some(x) = sa.or(sb) {
                sheet = x;
            }
            let v = resolve(tx.world(), self.schematic).map_err(bad)?;
            crate::schematic_view::route(&v, &sheet, pa, da, pb, db)
        } else {
            self.points.iter().map(|p| snap(*p)).collect()
        };
        for p in &points {
            check_xy(*p, "wire")?;
        }
        let points = simplify(&points);
        if points.len() < 2 {
            return Err(bad("a wire needs two different points (or from and to)"));
        }
        let id = add_item(tx, self.schematic, SchWire { schematic: self.schematic, sheet, points: points.clone() })?;
        Ok(json!({ "id": id, "points": points }))
    }
}

/// `pcb.schematic.label`: a net label or power port at `at`, or on a short
/// wire out of `pin` ("U1.3"). Power ports (`kind: "power"`) are GND, 3V3,
/// VBUS… and join by name everywhere.
#[derive(Deserialize, JsonSchema)]
pub struct CreateLabel {
    pub schematic: StableId,
    pub sheet: Option<String>,
    #[serde(default)]
    pub kind: Option<LabelKind>,
    pub text: String,
    pub at: Option<[f64; 2]>,
    pub pin: Option<String>,
    /// Stub wire length out of the pin (mm, default 2.54; 0 puts it on the pin).
    pub length: Option<f64>,
    pub rotation: Option<f64>,
    pub shape: Option<String>,
    pub style: Option<String>,
}

/// The label rotation that runs text away from a pin leaving in `d`.
pub fn rotation_for(d: Dir) -> f64 {
    match d {
        Dir::Right => 0.0,
        Dir::Up => 90.0,
        Dir::Left => 180.0,
        Dir::Down => 270.0,
    }
}

/// A label (with its stub wire) on a pin.
pub fn label_on_pin(tx: &mut Tx, schematic: StableId, pin: &PinRef, kind: LabelKind, text: &str, length: f64) -> Result<(StableId, Option<StableId>), CommandError> {
    let v = pin.dir.v();
    let end = snap([pin.at[0] + v[0] * length, pin.at[1] + v[1] * length]);
    let mut wire = None;
    if length > 1e-6 {
        wire = Some(add_item(tx, schematic, SchWire { schematic, sheet: pin.sheet.clone(), points: vec![pin.at, end] })?);
    }
    let rotation = match kind {
        // A port points away from a vertical pin; off a side pin, ground hangs down and supplies stand up.
        LabelKind::Power if pin.dir == Dir::Up => 90.0,
        LabelKind::Power if pin.dir == Dir::Down => 270.0,
        LabelKind::Power if is_ground(text) => 270.0,
        LabelKind::Power => 90.0,
        _ => rotation_for(pin.dir),
    };
    let id = add_item(tx, schematic, SchLabel { schematic, sheet: pin.sheet.clone(), kind, text: text.trim().into(), at: end, rotation, shape: None, style: None })?;
    Ok((id, wire))
}

impl Command for CreateLabel {
    const NAME: &'static str = "pcb.schematic.label";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let text = self.text.trim().to_string();
        if text.is_empty() || text.len() > 64 {
            return Err(bad("a label's text (the net name) is 1 to 64 characters"));
        }
        let kind = self.kind.unwrap_or(LabelKind::Local);
        if kind == LabelKind::Directive && !s.netclasses.iter().any(|c| c.name.eq_ignore_ascii_case(&text)) {
            return Err(bad(format!("no net class “{text}” (pcb.schematic.netclass adds one; classes: {})", s.netclasses.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(", "))));
        }
        if let Some(p) = &self.pin {
            let it = items(tx.world(), self.schematic);
            let pin = find_pin(&it, p)?;
            let (id, wire) = label_on_pin(tx, self.schematic, &pin, kind, &text, self.length.unwrap_or(2.54).max(0.0))?;
            if self.rotation.is_some() || self.shape.is_some() || self.style.is_some() {
                let mut l = tx.get::<SchLabel>(id).cloned().unwrap();
                if let Some(r) = self.rotation {
                    l.rotation = check_rotation(r)?;
                }
                l.shape = clean(self.shape);
                l.style = clean(self.style);
                tx.set(id, l)?;
            }
            return Ok(json!({ "id": id, "wire": wire }));
        }
        let at = self.at.ok_or_else(|| bad("give `at` or `pin`"))?;
        check_xy(at, "at")?;
        let sheet = sheet_or_first(&s, self.sheet)?;
        let rotation = check_rotation(self.rotation.unwrap_or(if kind == LabelKind::Power && is_ground(&text) { 270.0 } else if kind == LabelKind::Power { 90.0 } else { 0.0 }))?;
        let id = add_item(tx, self.schematic, SchLabel { schematic: self.schematic, sheet, kind, text, at: snap(at), rotation, shape: clean(self.shape), style: clean(self.style) })?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.schematic.junction`: a junction dot (joins wires crossing there).
#[derive(Deserialize, JsonSchema)]
pub struct CreateJunction {
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub at: [f64; 2],
}

impl Command for CreateJunction {
    const NAME: &'static str = "pcb.schematic.junction";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        check_xy(self.at, "at")?;
        let sheet = sheet_or_first(&s, self.sheet)?;
        let id = add_item(tx, self.schematic, SchJunction { schematic: self.schematic, sheet, at: snap(self.at) })?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.schematic.no_connect`: mark a pin ("U1.17", or a point) as deliberately unconnected.
#[derive(Deserialize, JsonSchema)]
pub struct CreateNoConnect {
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub at: Option<[f64; 2]>,
    pub pin: Option<String>,
    /// Several pins at once.
    #[serde(default)]
    pub pins: Vec<String>,
}

impl Command for CreateNoConnect {
    const NAME: &'static str = "pcb.schematic.no_connect";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let mut ids = Vec::new();
        let mut pins = self.pins.clone();
        pins.extend(self.pin.clone());
        if !pins.is_empty() {
            let it = items(tx.world(), self.schematic);
            for p in pins {
                let r = find_pin(&it, &p)?;
                ids.push(add_item(tx, self.schematic, SchNoConnect { schematic: self.schematic, sheet: r.sheet, at: r.at })?);
            }
        } else {
            let at = self.at.ok_or_else(|| bad("give `pin`, `pins` or `at`"))?;
            check_xy(at, "at")?;
            let sheet = sheet_or_first(&s, self.sheet)?;
            ids.push(add_item(tx, self.schematic, SchNoConnect { schematic: self.schematic, sheet, at: snap(at) })?);
        }
        Ok(json!({ "ids": ids }))
    }
}

/// `pcb.schematic.text`: a note on the sheet.
#[derive(Deserialize, JsonSchema)]
pub struct CreateText {
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub at: [f64; 2],
    pub text: String,
    pub size: Option<f64>,
    /// left (default), center, right.
    pub justify: Option<String>,
    /// 0 or 90.
    pub angle: Option<f64>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    pub color: Option<String>,
}

pub(crate) fn check_justify(j: &str) -> Result<String, CommandError> {
    match j.trim().to_ascii_lowercase().as_str() {
        "" | "left" | "start" => Ok(String::new()),
        "center" | "centre" | "middle" => Ok("center".into()),
        "right" | "end" => Ok("right".into()),
        _ => Err(bad("justify is left, center or right")),
    }
}

pub(crate) fn check_color(c: Option<String>) -> Result<Option<String>, CommandError> {
    match clean(c) {
        None => Ok(None),
        Some(c) => {
            let hex = c.strip_prefix('#').unwrap_or("");
            if (hex.len() == 6 || hex.len() == 3) && hex.chars().all(|x| x.is_ascii_hexdigit()) {
                Ok(Some(c.to_ascii_lowercase()))
            } else {
                Err(bad(format!("“{c}” is not a colour (#rrggbb)")))
            }
        }
    }
}

impl Command for CreateText {
    const NAME: &'static str = "pcb.schematic.text";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        check_xy(self.at, "at")?;
        if self.text.trim().is_empty() {
            return Err(bad("a note needs text"));
        }
        let size = self.size.unwrap_or(1.8);
        if !(0.5..=20.0).contains(&size) {
            return Err(bad("text size is 0.5 to 20 mm"));
        }
        let sheet = sheet_or_first(&s, self.sheet)?;
        let angle = check_rotation(self.angle.unwrap_or(0.0))?;
        let justify = check_justify(self.justify.as_deref().unwrap_or(""))?;
        let color = check_color(self.color)?;
        let id = add_item(tx, self.schematic, SchText { schematic: self.schematic, sheet, at: self.at, text: self.text, size, justify, angle: if angle == 90.0 || angle == 270.0 { 90.0 } else { 0.0 }, bold: self.bold, italic: self.italic, color })?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.schematic.item.update`: move or change a wire, label, junction, no-connect flag or note.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateItem {
    pub target: StableId,
    pub at: Option<[f64; 2]>,
    pub points: Option<Vec<[f64; 2]>>,
    pub text: Option<String>,
    pub kind: Option<LabelKind>,
    pub rotation: Option<f64>,
    pub size: Option<f64>,
    pub shape: Option<String>,
    pub style: Option<String>,
    /// Move by this much (any item; wires keep their shape).
    pub delta: Option<[f64; 2]>,
    /// Notes: left, center, right.
    pub justify: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// Notes: 0 or 90.
    pub angle: Option<f64>,
    /// "" clears.
    pub color: Option<String>,
    /// Bus entries: where the wire end sits from the bus end ([±2.54, ±2.54]).
    pub entry: Option<[f64; 2]>,
    /// Graphics (shapes, text boxes, tables, images): fields to change, named as in their create command.
    pub set: Option<Value>,
}

impl Command for UpdateItem {
    const NAME: &'static str = "pcb.schematic.item.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let id = self.target;
        let shift = |p: [f64; 2], d: Option<[f64; 2]>| d.map(|d| snap([p[0] + d[0], p[1] + d[1]])).unwrap_or(p);
        if let Some(mut w) = tx.get::<SchWire>(id).cloned() {
            if let Some(p) = self.points {
                for q in &p {
                    check_xy(*q, "wire")?;
                }
                w.points = simplify(&p.iter().map(|q| snap(*q)).collect::<Vec<_>>());
            }
            w.points = w.points.iter().map(|p| shift(*p, self.delta)).collect();
            if w.points.len() < 2 {
                return Err(bad("a wire needs two different points"));
            }
            tx.set(id, w)?;
        } else if let Some(mut l) = tx.get::<SchLabel>(id).cloned() {
            if let Some(a) = self.at {
                check_xy(a, "at")?;
                l.at = snap(a);
            }
            l.at = shift(l.at, self.delta);
            if let Some(t) = self.text {
                if t.trim().is_empty() {
                    return Err(bad("a label needs text"));
                }
                l.text = t.trim().into();
            }
            if let Some(k) = self.kind {
                l.kind = k;
            }
            if let Some(r) = self.rotation {
                l.rotation = check_rotation(r)?;
            }
            if self.shape.is_some() {
                l.shape = clean(self.shape);
            }
            if self.style.is_some() {
                l.style = clean(self.style);
            }
            tx.set(id, l)?;
        } else if let Some(mut j) = tx.get::<SchJunction>(id).cloned() {
            if let Some(a) = self.at {
                j.at = snap(a);
            }
            j.at = shift(j.at, self.delta);
            tx.set(id, j)?;
        } else if let Some(mut n) = tx.get::<SchNoConnect>(id).cloned() {
            if let Some(a) = self.at {
                n.at = snap(a);
            }
            n.at = shift(n.at, self.delta);
            tx.set(id, n)?;
        } else if let Some(mut t) = tx.get::<SchText>(id).cloned() {
            if let Some(a) = self.at {
                t.at = a;
            }
            if let Some(d) = self.delta {
                t.at = [t.at[0] + d[0], t.at[1] + d[1]];
            }
            if let Some(x) = self.text {
                t.text = x;
            }
            if let Some(s) = self.size {
                t.size = s.clamp(0.5, 20.0);
            }
            if let Some(j) = &self.justify {
                t.justify = check_justify(j)?;
            }
            if let Some(b) = self.bold {
                t.bold = b;
            }
            if let Some(b) = self.italic {
                t.italic = b;
            }
            if let Some(a) = self.angle {
                let a = check_rotation(a)?;
                t.angle = if a == 90.0 || a == 270.0 { 90.0 } else { 0.0 };
            }
            if self.color.is_some() {
                t.color = check_color(self.color)?;
            }
            tx.set(id, t)?;
        } else if let Some(s) = tx.get::<SchSymbol>(id).cloned() {
            let d = self.delta.unwrap_or([0.0, 0.0]);
            return UpdateSymbol { target: id, at: Some(self.at.unwrap_or([s.at[0] + d[0], s.at[1] + d[1]])), rotation: self.rotation, ..UpdateSymbol::empty(id) }.apply(tx);
        } else if crate::schematic_bus::update(tx, id, self.at, self.points.clone(), self.delta, self.entry)? || crate::schematic_graphics::update(tx, id, self.at, self.points, self.delta, self.set)? {
            return Ok(json!({ "id": id }));
        } else {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a schematic item")));
        }
        Ok(json!({ "id": id }))
    }
}

impl UpdateSymbol {
    pub fn empty(target: StableId) -> UpdateSymbol {
        UpdateSymbol {
            target,
            reference: None,
            value: None,
            footprint: None,
            symbol: None,
            at: None,
            rotation: None,
            mirror: None,
            unit: None,
            sheet: None,
            mpn: None,
            manufacturer: None,
            lcsc: None,
            description: None,
            dnp: None,
            pin_map: None,
            pins: None,
            fields: None,
            ref_at: None,
            value_at: None,
            exclude_from_board: None,
            instance: None,
            drag: true,
        }
    }
}

/// The schematic an item is on.
pub fn owner(world: &World, id: StableId) -> Option<StableId> {
    let e = entity(world, id)?;
    world
        .get::<SchSymbol>(e)
        .map(|c| c.schematic)
        .or_else(|| world.get::<SchWire>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchLabel>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchJunction>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchNoConnect>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchText>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchBus>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchBusEntry>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchShape>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchTextBox>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchTable>(e).map(|t| t.schematic))
        .or_else(|| world.get::<SchImage>(e).map(|t| t.schematic))
}

/// `pcb.schematic.delete`: remove schematic items (symbols, wires, labels, junctions, flags, notes).
#[derive(Deserialize, JsonSchema)]
pub struct DeleteSchItems {
    pub targets: Vec<StableId>,
}

impl Command for DeleteSchItems {
    const NAME: &'static str = "pcb.schematic.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        for id in &self.targets {
            let s = owner(tx.world(), *id).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not on a schematic")))?;
            tx.remove_dependency(s, *id)?;
            tx.delete(*id)?;
        }
        Ok(json!({ "deleted": self.targets.len() }))
    }
}

/// `pcb.schematic.connect`: put pins on a net: each gets a short wire and a
/// power port (supply names: GND, 3V3, VBUS…) or a net label. Pins already
/// on the net are left alone.
#[derive(Deserialize, JsonSchema)]
pub struct Connect {
    pub schematic: StableId,
    pub net: String,
    pub pins: Vec<String>,
    /// local (default for signals), global, or power (default for supply names).
    pub kind: Option<LabelKind>,
    pub length: Option<f64>,
}

impl Command for Connect {
    const NAME: &'static str = "pcb.schematic.connect";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        schematic_of(tx, self.schematic)?;
        let net = self.net.trim().to_string();
        if net.is_empty() {
            return Err(bad("a net needs a name"));
        }
        let kind = self.kind.unwrap_or(if is_power_net(&net) { LabelKind::Power } else { LabelKind::Local });
        let v = resolve(tx.world(), self.schematic).map_err(bad)?;
        let it = items(tx.world(), self.schematic);
        let mut added = 0;
        for p in &self.pins {
            let pin = find_pin(&it, p)?;
            let on = v.symbols.iter().find(|s| s.id == pin.symbol).and_then(|s| s.pins.iter().find(|x| x.number == pin.number)).and_then(|x| x.net.clone());
            if on.as_deref() == Some(net.as_str()) {
                continue;
            }
            label_on_pin(tx, self.schematic, &pin, kind, &net, self.length.unwrap_or(2.54))?;
            added += 1;
        }
        Ok(json!({ "net": net, "added": added }))
    }
}

/// A symbol as it appears on one sheet: on its own sheet, or in another
/// instance of that sheet (`instance`), where it has that instance's reference.
#[derive(Clone, Debug)]
pub struct Placed {
    pub id: StableId,
    pub instance: Option<String>,
    pub sheet: String,
    pub reference: String,
    pub symbol: SchSymbol,
}

/// Every symbol as placed, instances of shared sheets included. A symbol not yet
/// numbered in an instance reads `R?`.
pub fn placed(s: &Schematic, it: &SchItems) -> Vec<Placed> {
    let mut out: Vec<Placed> = it.symbols.iter().map(|(id, x)| Placed { id: *id, instance: None, sheet: x.sheet.clone(), reference: x.reference.clone(), symbol: x.clone() }).collect();
    for sh in s.sheets.iter().filter(|x| x.instance_of.is_some()) {
        let src = sh.instance_of.as_deref().unwrap_or_default();
        for (id, x) in it.symbols.iter().filter(|(_, x)| x.sheet == src) {
            let reference = sh.references.get(&id.to_string()).cloned().unwrap_or_else(|| unnumbered(x));
            out.push(Placed { id: *id, instance: Some(sh.id.clone()), sheet: sh.id.clone(), reference, symbol: x.clone() });
        }
    }
    out
}

/// "R?" for a symbol's reference prefix (power and flag symbols keep their `#` names).
fn unnumbered(x: &SchSymbol) -> String {
    if x.reference.starts_with('#') {
        return x.reference.clone();
    }
    let prefix: String = x.reference.trim_end_matches('?').trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
    let prefix = if prefix.is_empty() { x.def().map(|d| d.prefix).unwrap_or("U".into()) } else { prefix };
    format!("{prefix}?")
}

/// The items of a schematic with every instance sheet filled in: the items of the
/// sheet it shows, copied onto it (same ids), symbols carrying the instance's references.
pub fn expanded(s: &Schematic, it: &SchItems) -> SchItems {
    if !s.sheets.iter().any(|x| x.instance_of.is_some()) {
        return it.clone();
    }
    let mut out = it.clone();
    for p in placed(s, it).into_iter().filter(|p| p.instance.is_some()) {
        out.symbols.push((p.id, SchSymbol { sheet: p.sheet, reference: p.reference, ..p.symbol }));
    }
    for sh in s.sheets.iter().filter(|x| x.instance_of.is_some()) {
        let src = sh.instance_of.clone().unwrap_or_default();
        let on = |x: &str| x == src;
        macro_rules! copy {
            ($f:ident) => {
                for (id, x) in it.$f.iter().filter(|(_, x)| on(&x.sheet)) {
                    let mut c = x.clone();
                    c.sheet = sh.id.clone();
                    out.$f.push((*id, c));
                }
            };
        }
        copy!(wires);
        copy!(labels);
        copy!(junctions);
        copy!(no_connects);
        copy!(texts);
        copy!(buses);
        copy!(bus_entries);
        copy!(shapes);
        copy!(text_boxes);
        copy!(tables);
        copy!(images);
    }
    out.symbols.sort_by(|a, b| natural(&a.1.reference, &b.1.reference).then(a.1.unit.cmp(&b.1.unit)));
    out
}

/// `pcb.schematic.annotate`: number the symbols still called R?, C?, U?…
/// (all of them with `reset`), each instance of a shared sheet on its own.
/// Order: by rows (top to bottom, then left to right; default) or columns;
/// numbering continuous from `start`, or per sheet from the page number × 100
/// (`sheet`) or × 1000 (`sheet1000`). `targets` or `sheet` limit it;
/// `increment` instead adds to the numbers (R3 → R13 with 10).
#[derive(Deserialize, JsonSchema)]
pub struct Annotate {
    pub schematic: StableId,
    #[serde(default)]
    pub reset: bool,
    /// rows (default) or columns.
    pub order: Option<String>,
    /// continuous (default), sheet (page × 100 + 1) or sheet1000.
    pub numbering: Option<String>,
    /// First number for continuous numbering (default 1).
    pub start: Option<u32>,
    /// Only these symbols.
    pub targets: Option<Vec<StableId>>,
    /// Only this sheet (an instance sheet numbers its own copies).
    pub sheet: Option<String>,
    /// Add this to the numbers instead of renumbering.
    pub increment: Option<i64>,
}

fn split_reference(r: &str) -> (String, Option<i64>) {
    let digits = r.len() - r.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (p, n) = r.split_at(r.len() - digits);
    (p.to_string(), n.parse().ok())
}

impl Command for Annotate {
    const NAME: &'static str = "pcb.schematic.annotate";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = schematic_of(tx, self.schematic)?;
        let it = items(tx.world(), self.schematic);
        let all = placed(&s, &it);
        let sheet = match &self.sheet {
            Some(x) => Some(sheet_or_first(&s, Some(x.clone()))?),
            None => None,
        };
        let chosen = |p: &Placed| self.targets.as_ref().is_none_or(|t| t.contains(&p.id)) && sheet.as_ref().is_none_or(|x| &p.sheet == x) && !p.reference.starts_with('#');
        let order = |sh: &str| s.sheets.iter().position(|x| x.id == sh).unwrap_or(0);
        let mut renamed = Vec::new();
        let mut set: Vec<(usize, String)> = Vec::new();
        if let Some(inc) = self.increment {
            for (k, p) in all.iter().enumerate().filter(|(_, p)| chosen(p)) {
                let (prefix, n) = split_reference(&p.reference);
                if let Some(n) = n {
                    let r = format!("{prefix}{}", (n + inc).max(1));
                    set.push((k, r));
                }
            }
        } else {
            let columns = self.order.as_deref().is_some_and(|o| o.starts_with("col"));
            let per_sheet = match self.numbering.as_deref().unwrap_or("continuous") {
                "continuous" | "" => None,
                "sheet" | "sheet100" => Some(100),
                "sheet1000" => Some(1000),
                other => return Err(bad(format!("numbering is continuous, sheet or sheet1000 (not {other})"))),
            };
            let mut todo: Vec<usize> = (0..all.len()).filter(|k| chosen(&all[*k]) && (self.reset || all[*k].reference.contains('?'))).collect();
            todo.sort_by(|a, b| {
                let (a, b) = (&all[*a], &all[*b]);
                let (pa, pb) = (a.symbol.at, b.symbol.at);
                let first = order(&a.sheet).cmp(&order(&b.sheet));
                if columns { first.then(snap1(pa[0]).total_cmp(&snap1(pb[0]))).then(pa[1].total_cmp(&pb[1])) } else { first.then(snap1(pa[1]).total_cmp(&snap1(pb[1]))).then(pa[0].total_cmp(&pb[0])) }
            });
            let mut taken: Vec<String> = (0..all.len()).filter(|k| !todo.contains(k)).map(|k| all[k].reference.clone()).collect();
            // Units of one part (same old reference, symbol and sheet instance) get one new reference.
            let mut given: BTreeMap<(String, String, Option<String>), String> = BTreeMap::new();
            for k in todo {
                let x = &all[k];
                let (prefix, _) = split_reference(x.reference.trim_end_matches('?'));
                let prefix = if prefix.is_empty() { x.symbol.def().map(|d| d.prefix).unwrap_or("U".into()) } else { prefix };
                let key = (x.reference.clone(), x.symbol.symbol.clone(), x.instance.clone());
                let r = match given.get(&key).filter(|_| !x.reference.contains('?')) {
                    Some(r) => r.clone(),
                    None => {
                        let start = match per_sheet {
                            Some(m) => s.page(&x.sheet).parse::<u32>().unwrap_or(order(&x.sheet) as u32 + 1) * m + 1,
                            None => self.start.unwrap_or(1).max(1),
                        };
                        let mut n = start;
                        let r = loop {
                            let r = format!("{prefix}{n}");
                            if !taken.iter().any(|t| t.eq_ignore_ascii_case(&r)) {
                                break r;
                            }
                            n += 1;
                        };
                        taken.push(r.clone());
                        given.insert(key, r.clone());
                        r
                    }
                };
                set.push((k, r));
            }
        }
        let mut changed_sheets = false;
        for (k, r) in set {
            let x = &all[k];
            if r == x.reference {
                continue;
            }
            renamed.push(json!({ "from": x.reference, "to": r, "sheet": x.sheet }));
            match &x.instance {
                None => {
                    let cur = tx.get::<SchSymbol>(x.id).cloned().unwrap_or_else(|| x.symbol.clone());
                    tx.set(x.id, SchSymbol { reference: r, ..cur })?;
                }
                Some(inst) => {
                    if let Some(sh) = s.sheets.iter_mut().find(|y| &y.id == inst) {
                        sh.references.insert(x.id.to_string(), r);
                        changed_sheets = true;
                    }
                }
            }
        }
        if changed_sheets {
            tx.set(self.schematic, s)?;
        }
        Ok(json!({ "renamed": renamed }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.component::<Schematic>()
        .component::<SchSymbol>()
        .component::<SchWire>()
        .component::<SchLabel>()
        .component::<SchJunction>()
        .component::<SchNoConnect>()
        .component::<SchText>()
        .component::<SchBus>()
        .component::<SchBusEntry>()
        .component::<SchShape>()
        .component::<SchTextBox>()
        .component::<SchTable>()
        .component::<SchImage>()
        .command::<CreateSchematic>()
        .command::<UpdateSchematic>()
        .command::<SheetCommand>()
        .command::<PlaceSymbol>()
        .command::<UpdateSymbol>()
        .command::<CreateWire>()
        .command::<CreateLabel>()
        .command::<CreateJunction>()
        .command::<CreateNoConnect>()
        .command::<CreateText>()
        .command::<UpdateItem>()
        .command::<DeleteSchItems>()
        .command::<Connect>()
        .command::<Annotate>()
        .command::<crate::schematic_sync::UpdateBoardFromSchematic>()
        .command::<crate::schematic_sync::BackAnnotate>()
        .command::<crate::schematic_layout::FromNetlist>()
        .command::<crate::schematic_layout::FromBoard>()
        .command::<crate::schematic_bus::CreateBus>()
        .command::<crate::schematic_bus::CreateBusEntry>()
        .command::<crate::schematic_bus::Unfold>()
        .command::<crate::schematic_graphics::CreateShape>()
        .command::<crate::schematic_graphics::CreateTextBox>()
        .command::<crate::schematic_graphics::CreateTable>()
        .command::<crate::schematic_graphics::CreateImage>()
        .command::<crate::schematic_edit::Move>()
        .command::<crate::schematic_edit::Align>()
        .command::<crate::schematic_edit::Duplicate>()
        .command::<crate::schematic_edit::Replace>()
        .command::<crate::schematic_edit::Fields>()
        .command::<crate::schematic_edit::ErcConfig>()
        .command::<crate::schematic_edit::NetClassCommand>()
        .command::<crate::schematic_edit::NetColor>()
        .derived(de_core::Derived { name: "pcb.schematic", inputs: &[Schematic::KIND], reads_dependencies: true, compute });
}
