//! Printed circuit boards, designed by the AI and checked by the engine.
//!
//! A *board* (`pcb.board`) holds its outline (a polygon, or a sketch it
//! follows), its layer stack, finish and design rules, and any custom
//! footprints. Everything on it is its own entity, so teammates can work on
//! different parts of a board at once: *components* (`pcb.component`: a
//! reference designator, a footprint, an MPN, a place and side, and which net
//! each pad joins), *tracks*, *vias* and *zones* (copper pours). The board
//! depends on all of them, so its derived result — copper resolved into board
//! coordinates, zone fills, connectivity, the ratsnest and every design-rule
//! violation tied to the objects it is about — recomputes whenever one changes.
//!
//! The netlist is the pads' nets. A schematic (`schematic`), when there is
//! one, owns symbols and wires and writes those same pad nets through
//! "Update board from schematic"; boards without one keep `pcb.netlist.set`.
//!
//! Each board keeps a real 3D part in step (a shape program the engine writes:
//! the FR4 with its holes, exposed copper and every component body at its
//! catalogue height, bought parts declared as stock), so enclosures are
//! designed around it and clash-checked like any other part.

use de_core::{
    Command, CommandError, Derived, ErrorCode, IdIndex, Persist, Plugin, Registry, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub mod bitmap;
pub mod blocks;
pub mod board;
pub mod bodies;
pub mod drag;
pub mod drc;
pub mod fabs;
pub mod font;
pub mod footprints;
pub mod fpedit;
pub mod geom;
pub mod gerber;
pub mod gerber_check;
pub mod idf;
pub mod io;
pub mod kicad;
pub mod kicad_lib;
pub mod libedit;
pub mod library;
pub mod model3d;
pub mod outputs;
pub mod reader;
pub mod render3d;
pub mod route;
pub mod schematic;
pub mod schematic_bus;
pub mod schematic_edit;
pub mod schematic_export;
pub mod schematic_graphics;
pub mod schematic_layout;
pub mod schematic_pdf;
pub mod schematic_sync;
pub mod schematic_view;
pub mod sexpr;
pub mod sourcing;
pub mod spice;
pub mod symbols;
pub mod variants;
pub mod verify;
pub mod wizards;

pub use board::BoardView;
pub use footprints::Footprint;

pub struct PcbPlugin;

impl Plugin for PcbPlugin {
    fn name(&self) -> &'static str {
        "pcb"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<Board>()
            .component::<PcbComponent>()
            .component::<Track>()
            .component::<Via>()
            .component::<Zone>()
            .command::<CreateBoard>()
            .command::<UpdateBoard>()
            .command::<DefineFootprint>()
            .command::<CreateComponent>()
            .command::<UpdateComponent>()
            .command::<SetNetlist>()
            .command::<CreateTrack>()
            .command::<UpdateTrack>()
            .command::<CreateVia>()
            .command::<CreateZone>()
            .command::<UpdateZone>()
            .command::<DeleteItems>()
            .command::<route::Autoroute>()
            .command::<kicad::ImportKicad>()
            .command::<drag::CreateRoute>()
            .command::<OutlineToSketch>()
            .command::<SetSourcing>()
            .derived(Derived { name: "pcb.board", inputs: &[Board::KIND], reads_dependencies: true, compute: board::compute });
        schematic::register(r);
        spice::register(r);
        io::register(r);
        library::register(r);
        variants::register(r);
        blocks::register(r);
    }
}

// ---- Design data -----------------------------------------------------------

/// Copper layers by name: "top", "bottom", and "in1", "in2" on a four-layer board.
pub const LAYERS: [&str; 4] = ["top", "in1", "in2", "bottom"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Top,
    Bottom,
}

/// Limits the board is designed to, checked by DRC and against each fab's capabilities (mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Rules {
    pub min_track: f64,
    /// Copper to copper of different nets.
    pub min_space: f64,
    /// Smallest finished plated hole (pads and vias).
    pub min_drill: f64,
    /// Smallest copper ring around a plated hole.
    pub min_annular: f64,
    /// Copper and holes to the board edge.
    pub edge_clearance: f64,
    /// Hole edge to hole edge.
    pub hole_to_hole: f64,
    pub silk_width: f64,
    pub silk_height: f64,
    /// Solder-mask opening beyond the pad, each side.
    pub mask_expansion: f64,
    /// Courtyards of components on the same side may not overlap.
    pub courtyards: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { min_track: 0.15, min_space: 0.15, min_drill: 0.3, min_annular: 0.13, edge_clearance: 0.3, hole_to_hole: 0.5, silk_width: 0.15, silk_height: 1.0, mask_expansion: 0.05, courtyards: true }
    }
}

/// Widths and clearances for a group of nets (power wider than signals).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NetClass {
    pub name: String,
    #[serde(default)]
    pub nets: Vec<String>,
    pub track_width: f64,
    pub clearance: f64,
    #[serde(default = "default_via_drill")]
    pub via_drill: f64,
    #[serde(default = "default_via_diameter")]
    pub via_diameter: f64,
    /// Narrower width the autorouter may use near the net's pads (fine pitch); default
    /// 60 % of `track_width`, never below the board's `min_track`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neck_width: Option<f64>,
}

fn default_via_drill() -> f64 {
    0.3
}
fn default_via_diameter() -> f64 {
    0.6
}

impl NetClass {
    pub fn default_class() -> NetClass {
        NetClass { name: "Default".into(), nets: vec![], track_width: 0.25, clearance: 0.2, via_drill: 0.3, via_diameter: 0.6, neck_width: None }
    }
}

/// What the person chose to buy a line from, recorded on the board.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Choice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A printed circuit board.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Board {
    pub name: String,
    /// Outline polygon in board coordinates (mm, y up), any winding.
    #[serde(default)]
    pub outline: Vec<[f64; 2]>,
    /// Corners of the outline rounded to this radius (mm).
    #[serde(default)]
    pub corner_radius: f64,
    /// Holes and slots routed out of the board.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<Vec<[f64; 2]>>,
    /// A sketch whose largest closed loop is the outline (others are cutouts); overrides `outline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<StableId>,
    /// 2 or 4 copper layers.
    pub layers: u8,
    /// Finished thickness (mm).
    pub thickness: f64,
    /// "HASL lead-free", "HASL", "ENIG", "OSP", "Immersion silver".
    pub finish: String,
    /// Outer copper weight (oz/ft²).
    pub copper_oz: f64,
    #[serde(default = "one")]
    pub inner_copper_oz: f64,
    /// Solder-mask colour: green, black, blue, red, white, purple, yellow.
    pub mask: String,
    pub silk: String,
    #[serde(default)]
    pub rules: Rules,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub net_classes: Vec<NetClass>,
    /// Footprints made for this board (a part the library does not have), by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub footprints: BTreeMap<String, Footprint>,
    /// The 3D part kept in step with the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<StableId>,
    /// Revision printed on the fab files ("A", "1.1").
    #[serde(default = "rev_a")]
    pub revision: String,
    /// Boards to order (for estimates and parts totals).
    #[serde(default = "five")]
    pub quantity: u32,
    /// The fab chosen, by id ("jlcpcb", "pcbway", …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fab: Option<String>,
    /// Supplier and price chosen per BOM line (key: the MPN, or "value|footprint").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sourcing: BTreeMap<String, Choice>,
}

fn one() -> f64 {
    1.0
}
fn rev_a() -> String {
    "A".into()
}
fn five() -> u32 {
    5
}

impl Persist for Board {
    const KIND: &'static str = "pcb.board";
}

impl Board {
    pub fn copper_layers(&self) -> Vec<&'static str> {
        if self.layers >= 4 { LAYERS.to_vec() } else { vec!["top", "bottom"] }
    }

    /// The net class a net belongs to (the default from the first class named "Default", else built in).
    pub fn class_of(&self, net: Option<&str>) -> NetClass {
        if let Some(n) = net {
            if let Some(c) = self.net_classes.iter().find(|c| c.nets.iter().any(|x| x == n)) {
                return c.clone();
            }
        }
        self.net_classes.iter().find(|c| c.name.eq_ignore_ascii_case("default")).cloned().unwrap_or_else(NetClass::default_class)
    }

    pub fn footprint(&self, name: &str) -> Option<Footprint> {
        self.footprints.get(name).cloned().or_else(|| footprints::get(name))
    }
}

/// A component placed on a board.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PcbComponent {
    pub board: StableId,
    /// Reference designator: R1, C3, U1, J2.
    pub reference: String,
    /// "10k", "100nF 50V X7R", "ESP32-WROOM-32E".
    #[serde(default)]
    pub value: String,
    /// A library footprint name or one defined on the board.
    pub footprint: String,
    /// Footprint origin on the board (mm).
    pub at: [f64; 2],
    /// Counter-clockwise degrees, seen from the top.
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub side: Side,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// LCSC part number ("C25804"), for JLCPCB assembly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcsc: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Pad number → net name. Pads left out are unconnected.
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
    /// Pad number → pin function from the datasheet ("EN", "IO0", "VBUS").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pin_names: BTreeMap<String, String>,
    /// Do not populate (kept on the board and in the BOM as DNP, not assembled).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dnp: bool,
}

impl Persist for PcbComponent {
    const KIND: &'static str = "pcb.component";
}

/// A copper track: a polyline of `width` on one layer.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    pub board: StableId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub layer: String,
    pub width: f64,
    pub points: Vec<[f64; 2]>,
    /// Made by the autorouter (it may rip it up again).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
    /// An end was dragged along with a part it joins: it may need re-routing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reroute: bool,
}

impl Persist for Track {
    const KIND: &'static str = "pcb.track";
}

/// A plated through via.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Via {
    pub board: StableId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub at: [f64; 2],
    pub drill: f64,
    pub diameter: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
}

impl Persist for Via {
    const KIND: &'static str = "pcb.via";
}

/// A copper pour: fills its outline on one layer, keeping clear of other nets.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Zone {
    pub board: StableId,
    pub net: String,
    pub layer: String,
    /// Outline (mm); empty: the whole board.
    #[serde(default)]
    pub outline: Vec<[f64; 2]>,
    /// Gap to other nets; the net's class clearance when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<f64>,
    /// Copper narrower than this is removed (mm).
    #[serde(default = "min_width")]
    pub min_width: f64,
    /// Higher fills first; lower ones keep clear of them.
    #[serde(default)]
    pub priority: i32,
    /// Islands that join nothing of the net are removed.
    #[serde(default = "yes")]
    pub remove_islands: bool,
    /// How pads of the zone's net join it: thermal reliefs (default), thermal for through-hole
    /// pads only, solid, or not at all. Vias always join solid.
    #[serde(default)]
    pub connection: PadConnection,
    /// Gap around a relieved pad (mm); default 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_gap: Option<f64>,
    /// Width of each relief spoke (mm); default 0.5 (at least the zone's `min_width`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_spoke: Option<f64>,
}

/// How pads join a pour of their net.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PadConnection {
    /// Four spokes across a gap: solders well, the pour does not sink the heat.
    #[default]
    Thermal,
    /// Thermal reliefs on through-hole pads, SMD pads joined solid.
    ThermalTht,
    /// Joined solid (best current and heat transfer, hardest to solder by hand).
    Solid,
    /// Not joined: the pour keeps clear of the net's pads (they need tracks).
    None,
}

impl Zone {
    pub fn gap_thermal(&self) -> f64 {
        self.thermal_gap.unwrap_or(0.5)
    }
    pub fn spoke_width(&self) -> f64 {
        self.thermal_spoke.unwrap_or(0.5).max(self.min_width)
    }
}

fn min_width() -> f64 {
    0.25
}
fn yes() -> bool {
    true
}

impl Persist for Zone {
    const KIND: &'static str = "pcb.zone";
}

/// Derived on every board: everything resolved and checked.
#[derive(Component, Clone)]
pub struct BoardResult(pub Arc<BoardView>);

// ---- Helpers ---------------------------------------------------------------

fn bad(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, message)
}

fn entity(world: &World, id: StableId) -> Option<Entity> {
    world.resource::<IdIndex>().get(id)
}

pub(crate) fn board_of(tx: &Tx, id: StableId) -> Result<Board, CommandError> {
    tx.get::<Board>(id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a board")))
}

/// Everything on a board: its dependencies that point back at it.
pub struct Items {
    pub components: Vec<(StableId, PcbComponent)>,
    pub tracks: Vec<(StableId, Track)>,
    pub vias: Vec<(StableId, Via)>,
    pub zones: Vec<(StableId, Zone)>,
}

pub fn items(world: &World, board: StableId) -> Items {
    let mut out = Items { components: vec![], tracks: vec![], vias: vec![], zones: vec![] };
    let Some(e) = entity(world, board) else { return out };
    let deps = world.get::<de_core::DependsOn>(e).map(|d| d.0.clone()).unwrap_or_default();
    for id in deps {
        let Some(x) = entity(world, id) else { continue };
        if let Some(c) = world.get::<PcbComponent>(x).filter(|c| c.board == board) {
            out.components.push((id, c.clone()));
        } else if let Some(t) = world.get::<Track>(x).filter(|t| t.board == board) {
            out.tracks.push((id, t.clone()));
        } else if let Some(v) = world.get::<Via>(x).filter(|v| v.board == board) {
            out.vias.push((id, v.clone()));
        } else if let Some(z) = world.get::<Zone>(x).filter(|z| z.board == board) {
            out.zones.push((id, z.clone()));
        }
    }
    out.components.sort_by(|a, b| natural(&a.1.reference, &b.1.reference));
    out
}

/// "R2" before "R10".
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| {
        let i = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
        let (p, n) = s.split_at(i);
        (p.to_string(), n.parse::<u64>().unwrap_or(0), n.to_string())
    };
    split(a).cmp(&split(b))
}

/// The board result of a board entity (when worked out).
pub fn view(world: &World, board: StableId) -> Option<Arc<BoardView>> {
    entity(world, board).and_then(|e| world.get::<BoardResult>(e)).map(|r| r.0.clone())
}

/// Every board in the project.
pub fn boards(world: &World) -> Vec<(StableId, Board)> {
    let mut v: Vec<(StableId, Board)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Board>(e).map(|b| (id, b.clone()))).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn check_layer(board: &Board, layer: &str) -> Result<(), CommandError> {
    if board.copper_layers().contains(&layer) {
        Ok(())
    } else {
        Err(bad(format!("layer “{layer}” is not on this board (use {})", board.copper_layers().join(", "))))
    }
}

fn finite(v: f64, what: &str) -> Result<f64, CommandError> {
    if v.is_finite() { Ok(v) } else { Err(bad(format!("{what} must be a number"))) }
}

fn check_points(points: &[[f64; 2]], what: &str) -> Result<(), CommandError> {
    if points.iter().flatten().any(|v| !v.is_finite() || v.abs() > 2000.0) {
        return Err(bad(format!("{what}: coordinates are numbers within ±2000 mm")));
    }
    Ok(())
}

fn check_board(b: &Board) -> Result<(), CommandError> {
    if b.name.trim().is_empty() {
        return Err(bad("a board needs a name"));
    }
    if b.layers != 2 && b.layers != 4 {
        return Err(bad("a board has 2 or 4 copper layers"));
    }
    if !(0.2..=3.2).contains(&b.thickness) {
        return Err(bad("thickness is 0.2 to 3.2 mm (1.6 is standard)"));
    }
    if b.sketch.is_none() && b.outline.len() < 3 {
        return Err(bad("an outline needs 3 or more points (or width and height, or a sketch)"));
    }
    check_points(&b.outline, "outline")?;
    for c in &b.cutouts {
        check_points(c, "cutout")?;
        if c.len() < 3 {
            return Err(bad("a cutout needs 3 or more points"));
        }
    }
    if !(0.0..=50.0).contains(&b.corner_radius) {
        return Err(bad("corner radius is 0 to 50 mm"));
    }
    if !(0.25..=6.0).contains(&b.copper_oz) {
        return Err(bad("copper weight is 0.25 to 6 oz"));
    }
    let r = &b.rules;
    for (v, what) in [(r.min_track, "min_track"), (r.min_space, "min_space"), (r.min_drill, "min_drill"), (r.min_annular, "min_annular"), (r.edge_clearance, "edge_clearance"), (r.hole_to_hole, "hole_to_hole"), (r.silk_width, "silk_width"), (r.silk_height, "silk_height"), (r.mask_expansion, "mask_expansion")] {
        if !v.is_finite() || !(0.0..=10.0).contains(&v) {
            return Err(bad(format!("rule {what} is 0 to 10 mm")));
        }
    }
    for c in &b.net_classes {
        if c.neck_width.is_some_and(|n| !(n > 0.0 && n <= c.track_width)) {
            return Err(bad(format!("net class {}: neck_width is above 0 and at most the track width", c.name)));
        }
        if !(c.track_width > 0.0 && c.track_width <= 10.0 && c.clearance >= 0.0 && c.clearance <= 10.0 && c.via_drill > 0.0 && c.via_diameter > c.via_drill) {
            return Err(bad(format!("net class {}: a positive track width and clearance, and a via larger than its drill", c.name)));
        }
    }
    for (name, f) in &b.footprints {
        check_footprint(name, f)?;
    }
    Ok(())
}

pub(crate) fn check_footprint(name: &str, f: &Footprint) -> Result<(), CommandError> {
    if f.pads.is_empty() && f.silk.is_empty() {
        return Err(bad(format!("footprint {name} has no pads (a graphic-only footprint has silkscreen)")));
    }
    for p in &f.pads {
        if !(p.size[0] > 0.0 && p.size[1] > 0.0 && p.size[0] < 100.0 && p.size[1] < 100.0) {
            return Err(bad(format!("footprint {name}: pad {} needs a positive size", p.number)));
        }
        match (p.kind, p.drill) {
            (footprints::PadKind::Smd, Some(_)) => return Err(bad(format!("footprint {name}: SMD pad {} has a drill", p.number))),
            (footprints::PadKind::Tht | footprints::PadKind::Npth, None) => return Err(bad(format!("footprint {name}: pad {} needs a drill", p.number))),
            (footprints::PadKind::Tht, Some(d)) if d[0].min(d[1]) >= p.size[0].min(p.size[1]) => return Err(bad(format!("footprint {name}: pad {}'s hole is as big as the pad", p.number))),
            _ => {}
        }
    }
    Ok(())
}

// ---- Commands --------------------------------------------------------------

/// `pcb.board.create`: a new board, with its 3D part.
#[derive(Deserialize, JsonSchema)]
pub struct CreateBoard {
    pub name: String,
    /// Outline polygon (mm); or give `width` and `height` for a rectangle from the origin.
    #[serde(default)]
    pub outline: Vec<[f64; 2]>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    #[serde(default)]
    pub corner_radius: f64,
    #[serde(default)]
    pub cutouts: Vec<Vec<[f64; 2]>>,
    /// A sketch to follow: its largest loop is the outline.
    pub sketch: Option<StableId>,
    pub layers: Option<u8>,
    pub thickness: Option<f64>,
    pub finish: Option<String>,
    pub copper_oz: Option<f64>,
    pub mask: Option<String>,
    pub silk: Option<String>,
    pub rules: Option<Rules>,
    #[serde(default)]
    pub net_classes: Vec<NetClass>,
    pub revision: Option<String>,
    pub quantity: Option<u32>,
    /// Where the 3D board sits in the project (mm); default the origin.
    pub at: Option<[f64; 3]>,
    /// Make the 3D part (default true).
    pub model: Option<bool>,
}

impl Command for CreateBoard {
    const NAME: &'static str = "pcb.board.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let outline = match (self.outline.is_empty(), self.width, self.height) {
            (false, _, _) => self.outline,
            (true, Some(w), Some(h)) if w > 0.0 && h > 0.0 => vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]],
            _ if self.sketch.is_some() => vec![],
            _ => return Err(bad("give an outline, width and height, or a sketch")),
        };
        if let Some(s) = self.sketch {
            if tx.get::<de_shape::sketch::Sketch>(s).is_none() {
                return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{s} is not a sketch")));
            }
        }
        let board = Board {
            name: self.name.trim().to_string(),
            outline,
            corner_radius: self.corner_radius,
            cutouts: self.cutouts,
            sketch: self.sketch,
            layers: self.layers.unwrap_or(2),
            thickness: self.thickness.unwrap_or(1.6),
            finish: self.finish.unwrap_or_else(|| "HASL lead-free".into()),
            copper_oz: self.copper_oz.unwrap_or(1.0),
            inner_copper_oz: 0.5,
            mask: self.mask.unwrap_or_else(|| "green".into()),
            silk: self.silk.unwrap_or_else(|| "white".into()),
            rules: self.rules.unwrap_or_default(),
            net_classes: self.net_classes,
            footprints: BTreeMap::new(),
            model: None,
            revision: self.revision.unwrap_or_else(rev_a),
            quantity: self.quantity.unwrap_or(5),
            fab: None,
            sourcing: BTreeMap::new(),
        };
        check_board(&board)?;
        let id = tx.create();
        tx.set(id, board)?;
        if let Some(s) = self.sketch {
            tx.add_dependency(id, s)?;
        }
        let mut model = None;
        if self.model.unwrap_or(true) {
            model = Some(model3d::create(tx, id, self.at.unwrap_or([0.0; 3]))?);
        }
        Ok(json!({ "id": id, "model": model }))
    }
}

/// `pcb.board.update`: change any of a board's settings (what is left out stays).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateBoard {
    pub target: StableId,
    pub name: Option<String>,
    pub outline: Option<Vec<[f64; 2]>>,
    pub corner_radius: Option<f64>,
    pub cutouts: Option<Vec<Vec<[f64; 2]>>>,
    /// A sketch to follow; `null` stops following (the outline stays as given).
    #[serde(default, deserialize_with = "double_option")]
    #[schemars(with = "Option<StableId>")]
    pub sketch: Option<Option<StableId>>,
    pub layers: Option<u8>,
    pub thickness: Option<f64>,
    pub finish: Option<String>,
    pub copper_oz: Option<f64>,
    pub mask: Option<String>,
    pub silk: Option<String>,
    pub rules: Option<Rules>,
    pub net_classes: Option<Vec<NetClass>>,
    pub revision: Option<String>,
    pub quantity: Option<u32>,
    /// The fab chosen ("jlcpcb", "pcbway", "oshpark", "eurocircuits", "aisler"); "" for none.
    pub fab: Option<String>,
}

fn double_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

impl Command for UpdateBoard {
    const NAME: &'static str = "pcb.board.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.target)?;
        if let Some(v) = self.name {
            b.name = v.trim().to_string();
        }
        if let Some(v) = self.outline {
            b.outline = v;
        }
        if let Some(v) = self.corner_radius {
            b.corner_radius = v;
        }
        if let Some(v) = self.cutouts {
            b.cutouts = v;
        }
        if let Some(s) = self.sketch {
            if let Some(old) = b.sketch {
                tx.remove_dependency(self.target, old)?;
            }
            if let Some(new) = s {
                if tx.get::<de_shape::sketch::Sketch>(new).is_none() {
                    return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{new} is not a sketch")));
                }
                tx.add_dependency(self.target, new)?;
            } else if b.outline.len() < 3 {
                // Stop following, keeping the shape the sketch gave.
                if let Some(v) = view(tx.world(), self.target) {
                    b.outline = v.outline.clone();
                    b.cutouts = v.cutouts.clone();
                }
            }
            b.sketch = s;
        }
        if let Some(v) = self.layers {
            b.layers = v;
        }
        if let Some(v) = self.thickness {
            b.thickness = v;
        }
        if let Some(v) = self.finish {
            b.finish = v;
        }
        if let Some(v) = self.copper_oz {
            b.copper_oz = v;
        }
        if let Some(v) = self.mask {
            b.mask = v;
        }
        if let Some(v) = self.silk {
            b.silk = v;
        }
        if let Some(v) = self.rules {
            b.rules = v;
        }
        if let Some(v) = self.net_classes {
            b.net_classes = v;
        }
        if let Some(v) = self.revision {
            b.revision = v;
        }
        if let Some(v) = self.quantity {
            if v == 0 || v > 1_000_000 {
                return Err(bad("quantity is 1 to 1 000 000 boards"));
            }
            b.quantity = v;
        }
        if let Some(v) = self.fab {
            if !v.is_empty() && fabs::fab(&v).is_none() {
                return Err(bad(format!("unknown fab “{v}” (use {})", fabs::FABS.iter().map(|f| f.id).collect::<Vec<_>>().join(", "))));
            }
            b.fab = (!v.is_empty()).then_some(v);
        }
        check_board(&b)?;
        // Tracks and zones on inner layers go with them.
        if b.layers == 2 {
            let it = items(tx.world(), self.target);
            if it.tracks.iter().any(|(_, t)| t.layer.starts_with("in")) || it.zones.iter().any(|(_, z)| z.layer.starts_with("in")) {
                return Err(bad("tracks or zones are on inner layers: delete them before going to 2 layers"));
            }
        }
        tx.set(self.target, b)?;
        model3d::sync(tx, self.target)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.footprint.define`: a footprint for this board (a part the library lacks), from its datasheet.
#[derive(Deserialize, JsonSchema)]
pub struct DefineFootprint {
    pub board: StableId,
    pub footprint: Footprint,
}

impl Command for DefineFootprint {
    const NAME: &'static str = "pcb.footprint.define";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        let name = self.footprint.name.trim().to_string();
        if name.is_empty() {
            return Err(bad("a footprint needs a name"));
        }
        let mut fp = self.footprint;
        fp.pads.iter_mut().for_each(|p| p.normalize());
        check_footprint(&name, &fp)?;
        b.footprints.insert(name.clone(), Footprint { name: name.clone(), ..fp });
        tx.set(self.board, b)?;
        model3d::sync(tx, self.board)?;
        Ok(json!({ "board": self.board, "footprint": name }))
    }
}

/// `pcb.component.create`: place a component.
#[derive(Deserialize, JsonSchema)]
pub struct CreateComponent {
    pub board: StableId,
    pub reference: String,
    pub footprint: String,
    #[serde(default)]
    pub value: String,
    pub at: [f64; 2],
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub side: Side,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    #[serde(default)]
    pub description: String,
    /// Pad number → net name.
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
    #[serde(default)]
    pub pin_names: BTreeMap<String, String>,
    #[serde(default)]
    pub dnp: bool,
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn check_component(tx: &Tx, board: &Board, id: Option<StableId>, c: &PcbComponent) -> Result<(), CommandError> {
    let r = c.reference.trim();
    if r.is_empty() || !r.chars().next().unwrap().is_ascii_alphabetic() || r.len() > 16 || r.contains(char::is_whitespace) {
        return Err(bad(format!("“{r}” is not a reference designator (R1, C3, U1)")));
    }
    let fp = board.footprint(&c.footprint).ok_or_else(|| bad(format!("no footprint “{}” (GET /pcb/library lists them; define one with pcb.footprint.define)", c.footprint)))?;
    finite(c.at[0], "at")?;
    finite(c.at[1], "at")?;
    finite(c.rotation, "rotation")?;
    for (pad, net) in &c.pins {
        if !fp.pads.iter().any(|p| &p.number == pad) {
            let mut nums: Vec<&str> = fp.pads.iter().map(|p| p.number.as_str()).filter(|n| !n.is_empty()).collect();
            nums.dedup();
            return Err(bad(format!("{}: {} has no pad “{pad}” (pads: {})", c.reference, fp.name, nums.join(" "))));
        }
        if net.trim().is_empty() || net.len() > 64 {
            return Err(bad(format!("{}.{pad}: a net name is 1 to 64 characters", c.reference)));
        }
    }
    let it = items(tx.world(), c.board);
    if let Some((other, _)) = it.components.iter().find(|(oid, o)| Some(*oid) != id && o.reference.eq_ignore_ascii_case(r)) {
        return Err(bad(format!("{r} is already on the board ({other})")));
    }
    Ok(())
}

impl Command for CreateComponent {
    const NAME: &'static str = "pcb.component.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        library::ensure_footprint(tx, self.board, self.footprint.trim())?;
        let board = board_of(tx, self.board)?;
        let c = PcbComponent {
            board: self.board,
            reference: self.reference.trim().to_string(),
            value: self.value.trim().to_string(),
            footprint: self.footprint.trim().to_string(),
            at: self.at,
            rotation: self.rotation.rem_euclid(360.0),
            side: self.side,
            mpn: clean(self.mpn),
            manufacturer: clean(self.manufacturer),
            lcsc: clean(self.lcsc),
            description: self.description.trim().to_string(),
            pins: self.pins.into_iter().map(|(k, v)| (k, v.trim().to_string())).collect(),
            pin_names: self.pin_names,
            dnp: self.dnp,
        };
        check_component(tx, &board, None, &c)?;
        let id = tx.create();
        tx.set(id, c)?;
        tx.add_dependency(self.board, id)?;
        model3d::sync(tx, self.board)?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.component.update`: move, turn, flip, re-value or re-net a component (what is left out stays).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateComponent {
    pub target: StableId,
    pub reference: Option<String>,
    pub footprint: Option<String>,
    pub value: Option<String>,
    pub at: Option<[f64; 2]>,
    pub rotation: Option<f64>,
    pub side: Option<Side>,
    /// "" clears.
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    pub description: Option<String>,
    /// Replaces the pad nets.
    pub pins: Option<BTreeMap<String, String>>,
    pub pin_names: Option<BTreeMap<String, String>>,
    pub dnp: Option<bool>,
    /// Track ends on its pads follow a move, turn or flip (default true); dragged tracks are marked `reroute`.
    pub drag_tracks: Option<bool>,
}

impl Command for UpdateComponent {
    const NAME: &'static str = "pcb.component.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut c = tx.get::<PcbComponent>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a component", self.target)))?;
        if let Some(f) = &self.footprint {
            library::ensure_footprint(tx, c.board, f.trim())?;
        }
        let before = c.clone();
        let board = board_of(tx, c.board)?;
        if let Some(v) = self.reference {
            c.reference = v.trim().to_string();
        }
        if let Some(v) = self.footprint {
            c.footprint = v.trim().to_string();
        }
        if let Some(v) = self.value {
            c.value = v.trim().to_string();
        }
        if let Some(v) = self.at {
            c.at = v;
        }
        if let Some(v) = self.rotation {
            c.rotation = v.rem_euclid(360.0);
        }
        if let Some(v) = self.side {
            c.side = v;
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
            c.description = v.trim().to_string();
        }
        if let Some(v) = self.pins {
            c.pins = v.into_iter().map(|(k, v)| (k, v.trim().to_string())).collect();
        }
        if let Some(v) = self.pin_names {
            c.pin_names = v;
        }
        if let Some(v) = self.dnp {
            c.dnp = v;
        }
        check_component(tx, &board, Some(self.target), &c)?;
        let board_id = c.board;
        let dragged = if self.drag_tracks.unwrap_or(true) { drag::follow(tx, &board, &before, &c)? } else { 0 };
        tx.set(self.target, c)?;
        model3d::sync(tx, board_id)?;
        Ok(json!({ "id": self.target, "tracks_dragged": dragged }))
    }
}

/// `pcb.netlist.set`: the netlist as net → pins ("U1.3", "R4.1"). With
/// `replace` (default) pads not listed lose their nets; without it only the
/// listed pins change.
#[derive(Deserialize, JsonSchema)]
pub struct SetNetlist {
    pub board: StableId,
    pub nets: BTreeMap<String, Vec<String>>,
    #[serde(default = "yes")]
    pub replace: bool,
}

impl Command for SetNetlist {
    const NAME: &'static str = "pcb.netlist.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        let it = items(tx.world(), self.board);
        let mut by_ref: BTreeMap<String, (StableId, PcbComponent)> = it.components.into_iter().map(|(id, c)| (c.reference.to_ascii_uppercase(), (id, c))).collect();
        if self.replace {
            for (_, c) in by_ref.values_mut() {
                c.pins.clear();
            }
        }
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for (net, pins) in &self.nets {
            let net = net.trim();
            if net.is_empty() {
                return Err(bad("a net needs a name"));
            }
            for pin in pins {
                let (r, pad) = pin.rsplit_once('.').ok_or_else(|| bad(format!("“{pin}”: write pins as REF.PAD (U1.3)")))?;
                let (_, c) = by_ref.get_mut(&r.trim().to_ascii_uppercase()).ok_or_else(|| bad(format!("{pin}: no component {r} on the board")))?;
                let pad = pad.trim().to_string();
                if let Some(other) = seen.insert(format!("{}.{pad}", c.reference), net.to_string()) {
                    if other != net {
                        return Err(bad(format!("{pin} is in both {other} and {net}")));
                    }
                }
                c.pins.insert(pad, net.to_string());
            }
        }
        let mut changed = 0;
        for (id, c) in by_ref.into_values() {
            check_component(tx, &board, Some(id), &c)?;
            if tx.get::<PcbComponent>(id) != Some(&c) {
                changed += 1;
                tx.set(id, c)?;
            }
        }
        Ok(json!({ "board": self.board, "nets": self.nets.len(), "components_changed": changed }))
    }
}

/// The net of whatever copper is under `p` on `layer` (a pad, via or track end).
fn net_at(world: &World, board: StableId, layer: &str, p: [f64; 2]) -> Option<String> {
    let v = view(world, board)?;
    let probe = geom::Prim::circle(p, 0.0);
    for pad in &v.pads {
        if pad.layers.iter().any(|l| l == layer) && geom::gap(&pad.prim, &probe) <= 1e-6 {
            if let Some(n) = &pad.net {
                return Some(n.clone());
            }
        }
    }
    for via in &v.vias {
        if geom::dist(via.at, p) <= via.diameter / 2.0 + 1e-6 {
            if let Some(n) = &via.net {
                return Some(n.clone());
            }
        }
    }
    for t in &v.tracks {
        if t.layer == layer && t.net.is_some() && t.points.windows(2).any(|w| geom::point_seg_dist(p, w[0], w[1]) <= t.width / 2.0 + 1e-6) {
            return t.net.clone();
        }
    }
    None
}

/// `pcb.track.create`: a track through `points` on `layer`. The net, when
/// left out, is the net of the copper under its first (or last) point; the
/// width, the net class's.
#[derive(Deserialize, JsonSchema)]
pub struct CreateTrack {
    pub board: StableId,
    pub net: Option<String>,
    pub layer: String,
    pub width: Option<f64>,
    pub points: Vec<[f64; 2]>,
}

impl Command for CreateTrack {
    const NAME: &'static str = "pcb.track.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        check_layer(&board, &self.layer)?;
        check_points(&self.points, "track")?;
        if self.points.len() < 2 {
            return Err(bad("a track needs 2 or more points"));
        }
        let net = clean(self.net).or_else(|| net_at(tx.world(), self.board, &self.layer, self.points[0])).or_else(|| net_at(tx.world(), self.board, &self.layer, *self.points.last().unwrap()));
        let width = self.width.unwrap_or_else(|| board.class_of(net.as_deref()).track_width);
        if !(width > 0.0 && width <= 20.0) {
            return Err(bad("a track width is above 0 and up to 20 mm"));
        }
        let id = tx.create();
        tx.set(id, Track { board: self.board, net: net.clone(), layer: self.layer, width, points: self.points, auto: false, reroute: false })?;
        tx.add_dependency(self.board, id)?;
        Ok(json!({ "id": id, "net": net, "width": width }))
    }
}

/// `pcb.track.update`: change a track's points, width, layer or net.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateTrack {
    pub target: StableId,
    pub points: Option<Vec<[f64; 2]>>,
    pub width: Option<f64>,
    pub layer: Option<String>,
    pub net: Option<String>,
}

impl Command for UpdateTrack {
    const NAME: &'static str = "pcb.track.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut t = tx.get::<Track>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a track", self.target)))?;
        let board = board_of(tx, t.board)?;
        if let Some(p) = self.points {
            check_points(&p, "track")?;
            if p.len() < 2 {
                return Err(bad("a track needs 2 or more points"));
            }
            t.points = p;
        }
        if let Some(w) = self.width {
            if !(w > 0.0 && w <= 20.0) {
                return Err(bad("a track width is above 0 and up to 20 mm"));
            }
            t.width = w;
        }
        if let Some(l) = self.layer {
            check_layer(&board, &l)?;
            t.layer = l;
        }
        if self.net.is_some() {
            t.net = clean(self.net);
        }
        t.auto = false;
        t.reroute = false;
        tx.set(self.target, t)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.via.create`: a through via; drill and diameter default to the net class's.
#[derive(Deserialize, JsonSchema)]
pub struct CreateVia {
    pub board: StableId,
    pub net: Option<String>,
    pub at: [f64; 2],
    pub drill: Option<f64>,
    pub diameter: Option<f64>,
}

impl Command for CreateVia {
    const NAME: &'static str = "pcb.via.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        check_points(&[self.at], "via")?;
        let net = clean(self.net).or_else(|| board.copper_layers().iter().find_map(|l| net_at(tx.world(), self.board, l, self.at)));
        let class = board.class_of(net.as_deref());
        let drill = self.drill.unwrap_or(class.via_drill);
        let diameter = self.diameter.unwrap_or(class.via_diameter);
        if !(drill > 0.0 && diameter > drill && diameter < 10.0) {
            return Err(bad("a via's diameter is larger than its drill"));
        }
        let id = tx.create();
        tx.set(id, Via { board: self.board, net: net.clone(), at: self.at, drill, diameter, auto: false })?;
        tx.add_dependency(self.board, id)?;
        Ok(json!({ "id": id, "net": net }))
    }
}

/// `pcb.zone.create`: a copper pour of `net` on `layer` (whole board unless an outline is given).
#[derive(Deserialize, JsonSchema)]
pub struct CreateZone {
    pub board: StableId,
    pub net: String,
    pub layer: String,
    #[serde(default)]
    pub outline: Vec<[f64; 2]>,
    pub clearance: Option<f64>,
    pub min_width: Option<f64>,
    #[serde(default)]
    pub priority: i32,
    pub remove_islands: Option<bool>,
    /// "thermal" (default), "thermal_tht", "solid" or "none".
    pub connection: Option<PadConnection>,
    pub thermal_gap: Option<f64>,
    pub thermal_spoke: Option<f64>,
}

fn check_thermal(gap: Option<f64>, spoke: Option<f64>) -> Result<(), CommandError> {
    if gap.is_some_and(|g| !(g.is_finite() && (0.0..=5.0).contains(&g))) {
        return Err(bad("thermal_gap is 0 to 5 mm"));
    }
    if spoke.is_some_and(|w| !(w.is_finite() && w > 0.0 && w <= 5.0)) {
        return Err(bad("thermal_spoke is above 0 and up to 5 mm"));
    }
    Ok(())
}

impl Command for CreateZone {
    const NAME: &'static str = "pcb.zone.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        check_layer(&board, &self.layer)?;
        check_points(&self.outline, "zone")?;
        if !self.outline.is_empty() && self.outline.len() < 3 {
            return Err(bad("a zone outline needs 3 or more points (or none for the whole board)"));
        }
        if self.net.trim().is_empty() {
            return Err(bad("a zone needs a net"));
        }
        let z = Zone {
            board: self.board,
            net: self.net.trim().to_string(),
            layer: self.layer,
            outline: self.outline,
            clearance: self.clearance,
            min_width: self.min_width.unwrap_or(0.25),
            priority: self.priority,
            remove_islands: self.remove_islands.unwrap_or(true),
            connection: self.connection.unwrap_or_default(),
            thermal_gap: self.thermal_gap,
            thermal_spoke: self.thermal_spoke,
        };
        check_thermal(z.thermal_gap, z.thermal_spoke)?;
        let id = tx.create();
        tx.set(id, z)?;
        tx.add_dependency(self.board, id)?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.zone.update`: change a pour.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateZone {
    pub target: StableId,
    pub net: Option<String>,
    pub layer: Option<String>,
    pub outline: Option<Vec<[f64; 2]>>,
    pub clearance: Option<f64>,
    pub min_width: Option<f64>,
    pub priority: Option<i32>,
    pub remove_islands: Option<bool>,
    pub connection: Option<PadConnection>,
    pub thermal_gap: Option<f64>,
    pub thermal_spoke: Option<f64>,
}

impl Command for UpdateZone {
    const NAME: &'static str = "pcb.zone.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut z = tx.get::<Zone>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a zone", self.target)))?;
        let board = board_of(tx, z.board)?;
        if let Some(v) = self.net {
            z.net = v.trim().to_string();
        }
        if let Some(v) = self.layer {
            check_layer(&board, &v)?;
            z.layer = v;
        }
        if let Some(v) = self.outline {
            check_points(&v, "zone")?;
            z.outline = v;
        }
        if self.clearance.is_some() {
            z.clearance = self.clearance;
        }
        if let Some(v) = self.min_width {
            z.min_width = v;
        }
        if let Some(v) = self.priority {
            z.priority = v;
        }
        if let Some(v) = self.remove_islands {
            z.remove_islands = v;
        }
        if let Some(v) = self.connection {
            z.connection = v;
        }
        if self.thermal_gap.is_some() {
            z.thermal_gap = self.thermal_gap;
        }
        if self.thermal_spoke.is_some() {
            z.thermal_spoke = self.thermal_spoke;
        }
        check_thermal(z.thermal_gap, z.thermal_spoke)?;
        tx.set(self.target, z)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.delete`: remove components, tracks, vias or zones from their board.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteItems {
    pub targets: Vec<StableId>,
}

/// The board an item is on.
fn owner(world: &World, id: StableId) -> Option<StableId> {
    let e = entity(world, id)?;
    world.get::<PcbComponent>(e).map(|c| c.board).or_else(|| world.get::<Track>(e).map(|t| t.board)).or_else(|| world.get::<Via>(e).map(|v| v.board)).or_else(|| world.get::<Zone>(e).map(|z| z.board))
}

impl Command for DeleteItems {
    const NAME: &'static str = "pcb.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut boards = Vec::new();
        for id in &self.targets {
            let b = owner(tx.world(), *id).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not on a board")))?;
            let was_component = tx.get::<PcbComponent>(*id).is_some();
            tx.remove_dependency(b, *id)?;
            tx.delete(*id)?;
            if was_component && !boards.contains(&b) {
                boards.push(b);
            }
        }
        for b in boards {
            model3d::sync(tx, b)?;
        }
        Ok(json!({ "deleted": self.targets.len() }))
    }
}

/// `pcb.board.outline_to_sketch`: the board's outline as a sketch on XY (for
/// an enclosure to follow). With `link` the board then follows the sketch.
#[derive(Deserialize, JsonSchema)]
pub struct OutlineToSketch {
    pub board: StableId,
    pub name: Option<String>,
    #[serde(default)]
    pub link: bool,
}

impl Command for OutlineToSketch {
    const NAME: &'static str = "pcb.board.outline_to_sketch";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let b = board_of(tx, self.board)?;
        let v = view(tx.world(), self.board).ok_or_else(|| CommandError::new(ErrorCode::StaleDerivedOutput, "the board is not worked out yet"))?;
        let (outline, cutouts) = if b.corner_radius > 0.0 || b.sketch.is_some() { (v.outline.clone(), v.cutouts.clone()) } else { (b.outline.clone(), b.cutouts.clone()) };
        let mut points = Vec::new();
        let mut curves = Vec::new();
        let mut next = 1u32;
        for poly in std::iter::once(&outline).chain(cutouts.iter()) {
            let first = next;
            for p in poly {
                points.push(json!({ "id": next, "x": p[0], "y": p[1] }));
                next += 1;
            }
            let n = poly.len() as u32;
            for i in 0..n {
                curves.push(json!({ "kind": "line", "id": 1000 + first + i, "a": first + i, "b": first + (i + 1) % n }));
            }
        }
        let name = self.name.unwrap_or_else(|| format!("{} outline", b.name)).replace('.', " ");
        let out = tx.run("sketch.create", json!({ "name": name, "plane": { "name": "XY", "offset": 0 }, "geometry": { "points": points, "curves": curves, "constraints": [] }, "params": [] }))?;
        let sketch: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        if self.link {
            // The sketch is solved when this change commits, so the 3D part
            // keeps the (identical) polygon until the board next changes.
            let mut b = board_of(tx, self.board)?;
            b.sketch = Some(sketch);
            tx.set(self.board, b)?;
            tx.add_dependency(self.board, sketch)?;
        }
        Ok(json!({ "id": sketch, "linked": self.link }))
    }
}

/// `pcb.sourcing.set`: record where one BOM line is bought and at what price
/// (key: its MPN, or "value|footprint"); leave everything out to clear it.
#[derive(Deserialize, JsonSchema)]
pub struct SetSourcing {
    pub board: StableId,
    pub key: String,
    #[serde(flatten)]
    pub choice: Choice,
}

impl Command for SetSourcing {
    const NAME: &'static str = "pcb.sourcing.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        if self.key.trim().is_empty() {
            return Err(bad("a sourcing line needs its key (MPN)"));
        }
        if self.choice.unit_price.is_some_and(|p| !p.is_finite() || p < 0.0) {
            return Err(bad("a unit price is 0 or more"));
        }
        let c = Choice { supplier: clean(self.choice.supplier), sku: clean(self.choice.sku), unit_price: self.choice.unit_price, currency: clean(self.choice.currency), url: clean(self.choice.url), note: clean(self.choice.note) };
        if c == Choice::default() {
            b.sourcing.remove(self.key.trim());
        } else {
            b.sourcing.insert(self.key.trim().to_string(), c);
        }
        tx.set(self.board, b)?;
        Ok(json!({ "board": self.board, "key": self.key }))
    }
}
