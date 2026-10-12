//! Design blocks: a piece of circuit kept for reuse (the USB-C power input,
//! an LDO with its capacitors, a crystal with its load caps), its schematic
//! and, when the parts are on a board, their layout and copper. Saved from
//! a selection (symbols by reference, or a whole sheet); placed anywhere on a
//! schematic with fresh references, local labels kept apart from the ones
//! already there, and — with `board_at` — its parts placed on the board as
//! they were laid out, tracks and vias included, on the new nets.
//!
//! Placed parts carry the field `Design block` with the block's name.

use crate::schematic::{self, LabelKind, SchSymbol};
use crate::symbols::{PinSpec, SymbolDef};
use crate::{bad, natural};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

type P = [f64; 2];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockSymbol {
    pub symbol: String,
    pub reference: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub footprint: String,
    /// Relative to the block's origin (sheet mm, y down).
    pub at: P,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub mirror: bool,
    #[serde(default = "one")]
    pub unit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcsc: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub dnp: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pin_map: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<PinSpec>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<SymbolDef>,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockLabel {
    pub kind: LabelKind,
    pub text: String,
    pub at: P,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockText {
    pub at: P,
    pub text: String,
    pub size: f64,
}

/// A part's place on the board, relative to the block's board origin (mm, y up).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockPart {
    pub reference: String,
    pub footprint: String,
    pub at: P,
    pub rotation: f64,
    pub side: crate::Side,
}

/// Copper of the block: its net by a pad it reaches (`pin`, "R1.2") when it has one, else by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockCopper {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    /// Track: layer, width and points; via: at, drill, diameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<P>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<[f64; 4]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlockLayout {
    pub parts: Vec<BlockPart>,
    #[serde(default)]
    pub copper: Vec<BlockCopper>,
    /// Footprints defined on the source board that the parts use.
    #[serde(default)]
    pub footprints: Vec<crate::footprints::Footprint>,
}

/// `pcb.design_block`.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DesignBlock {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    pub symbols: Vec<BlockSymbol>,
    #[serde(default)]
    pub wires: Vec<Vec<P>>,
    #[serde(default)]
    pub labels: Vec<BlockLabel>,
    #[serde(default)]
    pub junctions: Vec<P>,
    #[serde(default)]
    pub no_connects: Vec<P>,
    #[serde(default)]
    pub texts: Vec<BlockText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<BlockLayout>,
}

impl Persist for DesignBlock {
    const KIND: &'static str = "pcb.design_block";
}

pub fn blocks(world: &World) -> Vec<(StableId, DesignBlock)> {
    let mut v: Vec<(StableId, DesignBlock)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<DesignBlock>(e).map(|x| (id, x.clone()))).collect();
    v.sort_by(|a, b| natural(&a.1.name, &b.1.name));
    v
}

fn find(world: &World, name_or_id: &str) -> Option<(StableId, DesignBlock)> {
    blocks(world).into_iter().find(|(id, b)| b.name.eq_ignore_ascii_case(name_or_id) || id.to_string() == name_or_id)
}

fn within(p: P, b: [f64; 4]) -> bool {
    p[0] >= b[0] - 1e-6 && p[0] <= b[2] + 1e-6 && p[1] >= b[1] - 1e-6 && p[1] <= b[3] + 1e-6
}

fn snap(v: f64) -> f64 {
    schematic::snap1(v)
}

/// The block a selection makes (no entity written): symbols by reference (or the whole sheet), and the
/// wires, labels, junctions, flags and notes inside them.
pub fn capture(world: &World, sid: StableId, references: &[String], sheet: Option<&str>, include_layout: bool) -> Result<DesignBlock, String> {
    let it = schematic::items(world, sid);
    let view = schematic::resolve(world, sid)?;
    let picked: Vec<&(StableId, SchSymbol)> = it
        .symbols
        .iter()
        .filter(|(_, s)| match sheet {
            Some(sh) => s.sheet == sh,
            None => references.iter().any(|r| r.eq_ignore_ascii_case(&s.reference)),
        })
        .collect();
    if picked.is_empty() {
        return Err("nothing selected: give references or a sheet".into());
    }
    let sheet_id = picked[0].1.sheet.clone();
    if picked.iter().any(|(_, s)| s.sheet != sheet_id) {
        return Err("a block comes from one sheet".into());
    }
    // The selection's extent: symbol bodies and pins, plus a grid step.
    let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (id, _) in &picked {
        if let Some(v) = view.symbols.iter().find(|v| v.id == *id) {
            bb = [bb[0].min(v.bbox[0]), bb[1].min(v.bbox[1]), bb[2].max(v.bbox[2]), bb[3].max(v.bbox[3])];
        }
    }
    let m = if sheet.is_some() { 1e6 } else { 2.54 };
    let area = [bb[0] - m, bb[1] - m, bb[2] + m, bb[3] + m];
    let origin = [snap((bb[0] + bb[2]) / 2.0), snap((bb[1] + bb[3]) / 2.0)];
    let rel = |p: P| [((p[0] - origin[0]) * 1e4).round() / 1e4, ((p[1] - origin[1]) * 1e4).round() / 1e4];
    let on = |s: &str| s == sheet_id;
    let symbols = picked
        .iter()
        .map(|(_, s)| BlockSymbol {
            symbol: s.symbol.clone(),
            reference: s.reference.clone(),
            value: s.value.clone(),
            footprint: s.footprint.clone(),
            at: rel(s.at),
            rotation: s.rotation,
            mirror: s.mirror,
            unit: s.unit,
            mpn: s.mpn.clone(),
            manufacturer: s.manufacturer.clone(),
            lcsc: s.lcsc.clone(),
            description: s.description.clone(),
            dnp: s.dnp,
            pin_map: s.pin_map.clone(),
            pins: s.pins.clone(),
            fields: s.fields.iter().filter(|(k, _)| k.as_str() != "Design block").map(|(k, v)| (k.clone(), v.clone())).collect(),
            definition: s.definition.as_deref().cloned(),
        })
        .collect();
    let wires = it.wires.iter().filter(|(_, w)| on(&w.sheet) && w.points.iter().all(|p| within(*p, area))).map(|(_, w)| w.points.iter().map(|p| rel(*p)).collect()).collect();
    let labels = it.labels.iter().filter(|(_, l)| on(&l.sheet) && within(l.at, area)).map(|(_, l)| BlockLabel { kind: l.kind, text: l.text.clone(), at: rel(l.at), rotation: l.rotation, shape: l.shape.clone(), style: l.style.clone() }).collect();
    let junctions = it.junctions.iter().filter(|(_, j)| on(&j.sheet) && within(j.at, area)).map(|(_, j)| rel(j.at)).collect();
    let no_connects = it.no_connects.iter().filter(|(_, n)| on(&n.sheet) && within(n.at, area)).map(|(_, n)| rel(n.at)).collect();
    let texts = it.texts.iter().filter(|(_, t)| on(&t.sheet) && within(t.at, area)).map(|(_, t)| BlockText { at: rel(t.at), text: t.text.clone(), size: t.size }).collect();
    let mut layout = None;
    if include_layout {
        if let Some(board_id) = view.board {
            layout = capture_layout(world, board_id, &picked.iter().map(|(_, s)| s.reference.clone()).collect::<Vec<_>>());
        }
    }
    Ok(DesignBlock { name: String::new(), description: String::new(), keywords: vec![], symbols, wires, labels, junctions, no_connects, texts, layout })
}

fn capture_layout(world: &World, board_id: StableId, refs: &[String]) -> Option<BlockLayout> {
    let board = world.resource::<IdIndex>().get(board_id).and_then(|e| world.get::<crate::Board>(e)).cloned()?;
    let view = crate::view(world, board_id)?;
    let parts: Vec<&crate::board::ComponentView> = view.components.iter().filter(|c| refs.iter().any(|r| r.eq_ignore_ascii_case(&c.reference))).collect();
    if parts.is_empty() {
        return None;
    }
    let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for c in &parts {
        for p in &c.courtyard {
            bb = [bb[0].min(p[0]), bb[1].min(p[1]), bb[2].max(p[0]), bb[3].max(p[1])];
        }
        bb = [bb[0].min(c.at[0]), bb[1].min(c.at[1]), bb[2].max(c.at[0]), bb[3].max(c.at[1])];
    }
    let origin = [((bb[0] + bb[2]) / 2.0 * 100.0).round() / 100.0, ((bb[1] + bb[3]) / 2.0 * 100.0).round() / 100.0];
    let rel = |p: P| [((p[0] - origin[0]) * 1e4).round() / 1e4, ((p[1] - origin[1]) * 1e4).round() / 1e4];
    let area = [bb[0] - 1.0, bb[1] - 1.0, bb[2] + 1.0, bb[3] + 1.0];
    // A pad of the block on each net, to find the net again after placing.
    let mut anchor: BTreeMap<String, String> = BTreeMap::new();
    for c in &parts {
        for (pad, net) in &c.pins {
            anchor.entry(net.clone()).or_insert(format!("{}.{pad}", c.reference));
        }
    }
    let mut copper = Vec::new();
    for t in &view.tracks {
        if t.points.iter().all(|p| within(*p, area)) {
            copper.push(BlockCopper { net: t.net.clone(), pin: t.net.as_ref().and_then(|n| anchor.get(n).cloned()), layer: Some(t.layer.clone()), width: Some(t.width), points: t.points.iter().map(|p| rel(*p)).collect(), via: None });
        }
    }
    for v in &view.vias {
        if within(v.at, area) {
            let at = rel(v.at);
            copper.push(BlockCopper { net: v.net.clone(), pin: v.net.as_ref().and_then(|n| anchor.get(n).cloned()), layer: None, width: None, points: vec![], via: Some([at[0], at[1], v.drill, v.diameter]) });
        }
    }
    let used: BTreeSet<&str> = parts.iter().map(|c| c.footprint.as_str()).collect();
    let footprints = board.footprints.values().filter(|f| used.contains(f.name.as_str())).cloned().collect();
    Some(BlockLayout { parts: parts.iter().map(|c| BlockPart { reference: c.reference.clone(), footprint: c.footprint.clone(), at: rel(c.at), rotation: c.rotation, side: c.side }).collect(), copper, footprints })
}

/// `pcb.block.save`: keep a selection as a design block (replacing the block of that name with `replace`).
#[derive(Deserialize, JsonSchema)]
pub struct SaveBlock {
    pub name: String,
    pub schematic: StableId,
    /// The symbols to keep, by reference (wires, labels and notes among them come along).
    #[serde(default)]
    pub references: Vec<String>,
    /// Or a whole sheet (its id or name).
    pub sheet: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Keep the parts' board layout and copper too (default true when they are on a board).
    #[serde(default = "yes")]
    pub layout: bool,
    #[serde(default)]
    pub replace: bool,
}

fn yes() -> bool {
    true
}

impl Command for SaveBlock {
    const NAME: &'static str = "pcb.block.save";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let name = self.name.trim().to_string();
        if name.is_empty() || name.len() > 64 {
            return Err(bad("a design block needs a name (1 to 64 characters)"));
        }
        let sch = tx.get::<schematic::Schematic>(self.schematic).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a schematic", self.schematic)))?;
        let sheet = match &self.sheet {
            Some(s) => Some(sch.sheets.iter().find(|x| &x.id == s || x.name.eq_ignore_ascii_case(s)).map(|x| x.id.clone()).ok_or_else(|| bad(format!("no sheet {s}")))?),
            None => None,
        };
        let mut b = capture(tx.world(), self.schematic, &self.references, sheet.as_deref(), self.layout).map_err(bad)?;
        b.name = name.clone();
        b.description = self.description;
        b.keywords = self.keywords;
        let id = match find(tx.world(), &name) {
            Some((id, _)) if self.replace => id,
            Some(_) => return Err(bad(format!("there is a design block “{name}” already (replace: true to update it)"))),
            None => tx.create(),
        };
        let summary = json!({ "id": id, "name": name, "symbols": b.symbols.len(), "wires": b.wires.len(), "labels": b.labels.len(), "layout": b.layout.as_ref().map(|l| json!({ "parts": l.parts.len(), "copper": l.copper.len() })) });
        tx.set(id, b)?;
        Ok(summary)
    }
}

/// `pcb.block.place`: put a design block on a schematic at `at` (sheet mm),
/// with new references; with `board_at` its parts go on the board as laid out.
#[derive(Deserialize, JsonSchema)]
pub struct PlaceBlock {
    /// Block name or id.
    pub block: String,
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub at: P,
    /// Where the block's layout goes on the board (mm, y up).
    pub board_at: Option<P>,
}

impl Command for PlaceBlock {
    const NAME: &'static str = "pcb.block.place";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (_, b) = find(tx.world(), &self.block).ok_or_else(|| bad(format!("no design block “{}”", self.block)))?;
        let sch = tx.get::<schematic::Schematic>(self.schematic).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a schematic", self.schematic)))?;
        let sheet = match &self.sheet {
            Some(s) => sch.sheets.iter().find(|x| &x.id == s || x.name.eq_ignore_ascii_case(s)).map(|x| x.id.clone()).ok_or_else(|| bad(format!("no sheet {s}")))?,
            None => sch.sheets.first().map(|x| x.id.clone()).unwrap_or_else(|| "1".into()),
        };
        let at = [snap(self.at[0]), snap(self.at[1])];
        let abs = |p: P| [p[0] + at[0], p[1] + at[1]];
        let it = schematic::items(tx.world(), self.schematic);
        // Fresh references: one per old reference (units of a part share theirs).
        let mut taken: Vec<String> = it.symbols.iter().map(|(_, s)| s.reference.clone()).collect();
        let mut renamed: BTreeMap<String, String> = BTreeMap::new();
        let empty = schematic::SchItems::default();
        for s in &b.symbols {
            if renamed.contains_key(&s.reference) || s.reference.starts_with('#') {
                continue;
            }
            let prefix: String = s.reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '?').to_string();
            let r = schematic::next_reference(&empty, &prefix, &taken);
            taken.push(r.clone());
            renamed.insert(s.reference.clone(), r);
        }
        // Local labels already on that sheet get a suffix so the block's nets stay its own.
        let existing: BTreeSet<String> = it.labels.iter().filter(|(_, l)| l.sheet == sheet && l.kind == LabelKind::Local).map(|(_, l)| l.text.clone()).collect();
        let n = blocks_placed(&it, &b.name) / renamed.len().max(1) + 1;
        let local = |t: &str| if existing.contains(t) { format!("{t}_{n}") } else { t.to_string() };
        let mut placed = Vec::new();
        for s in &b.symbols {
            let reference = renamed.get(&s.reference).cloned().unwrap_or(s.reference.clone());
            let mut fields = s.fields.clone();
            fields.insert("Design block".into(), b.name.clone());
            let out = tx.run(
                "pcb.schematic.place",
                json!({ "schematic": self.schematic, "sheet": sheet, "symbol": s.symbol, "definition": s.definition, "reference": reference, "value": s.value, "footprint": if s.footprint.is_empty() { None } else { Some(&s.footprint) }, "at": abs(s.at), "rotation": s.rotation, "mirror": s.mirror, "unit": s.unit, "mpn": s.mpn, "manufacturer": s.manufacturer, "lcsc": s.lcsc, "description": s.description, "dnp": s.dnp, "pin_map": s.pin_map, "pins": s.pins, "fields": fields }),
            )?;
            placed.push(out["reference"].as_str().unwrap_or("").to_string());
        }
        for w in &b.wires {
            tx.run("pcb.schematic.wire", json!({ "schematic": self.schematic, "sheet": sheet, "points": w.iter().map(|p| abs(*p)).collect::<Vec<_>>() }))?;
        }
        for l in &b.labels {
            let text = if l.kind == LabelKind::Local { local(&l.text) } else { l.text.clone() };
            tx.run("pcb.schematic.label", json!({ "schematic": self.schematic, "sheet": sheet, "kind": l.kind, "text": text, "at": abs(l.at), "rotation": l.rotation, "shape": l.shape, "style": l.style }))?;
        }
        for j in &b.junctions {
            tx.run("pcb.schematic.junction", json!({ "schematic": self.schematic, "sheet": sheet, "at": abs(*j) }))?;
        }
        for q in &b.no_connects {
            tx.run("pcb.schematic.no_connect", json!({ "schematic": self.schematic, "sheet": sheet, "at": abs(*q) }))?;
        }
        for t in &b.texts {
            tx.run("pcb.schematic.text", json!({ "schematic": self.schematic, "sheet": sheet, "at": abs(t.at), "text": t.text, "size": t.size }))?;
        }
        let mut on_board = 0;
        if let (Some(board_at), Some(layout), Some(board)) = (self.board_at, &b.layout, sch.board) {
            on_board = place_layout(tx, self.schematic, board, layout, &renamed, board_at)?;
        }
        Ok(json!({ "placed": placed, "renamed": renamed, "on_board": on_board }))
    }
}

fn blocks_placed(it: &schematic::SchItems, name: &str) -> usize {
    let refs: BTreeSet<&str> = it.symbols.iter().filter(|(_, s)| s.fields.get("Design block").is_some_and(|b| b == name)).map(|(_, s)| s.reference.as_str()).collect();
    refs.len()
}

/// The block's parts onto the board with the nets the schematic now gives them, then its copper.
fn place_layout(tx: &mut Tx, sid: StableId, board: StableId, layout: &BlockLayout, renamed: &BTreeMap<String, String>, origin: P) -> Result<usize, CommandError> {
    let mut b = crate::board_of(tx, board)?;
    let mut defined = false;
    for f in &layout.footprints {
        if !b.footprints.contains_key(&f.name) {
            b.footprints.insert(f.name.clone(), f.clone());
            defined = true;
        }
    }
    if defined {
        tx.set(board, b)?;
    }
    crate::library::ensure_schematic_footprints(tx, sid)?;
    let view = schematic::resolve(tx.world(), sid).map_err(bad)?;
    let it = schematic::items(tx.world(), sid);
    let abs = |p: P| [p[0] + origin[0], p[1] + origin[1]];
    let mut nets_of: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut n = 0;
    for part in &layout.parts {
        let Some(new_ref) = renamed.get(&part.reference) else { continue };
        let syms: Vec<&(StableId, SchSymbol)> = it.symbols.iter().filter(|(_, s)| &s.reference == new_ref).collect();
        let Some((_, first)) = syms.first() else { continue };
        let mut pins: BTreeMap<String, String> = BTreeMap::new();
        let mut pin_names: BTreeMap<String, String> = BTreeMap::new();
        for (id, _) in &syms {
            if let Some(v) = view.symbols.iter().find(|v| v.id == *id) {
                for p in &v.pins {
                    for pad in &p.pads {
                        if let Some(net) = &p.net {
                            pins.insert(pad.clone(), net.clone());
                        }
                        if !p.name.is_empty() && p.name != "~" {
                            pin_names.insert(pad.clone(), p.name.clone());
                        }
                    }
                }
            }
        }
        nets_of.insert(part.reference.clone(), pins.clone());
        let out = tx.run(
            "pcb.component.create",
            json!({ "board": board, "reference": new_ref, "footprint": part.footprint, "value": first.value, "at": abs(part.at), "rotation": part.rotation, "side": part.side, "mpn": first.mpn, "manufacturer": first.manufacturer, "lcsc": first.lcsc, "description": first.description, "dnp": first.dnp, "pins": pins, "pin_names": pin_names }),
        )?;
        let cid: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        for (id, s) in syms {
            tx.set(*id, SchSymbol { component: Some(cid), ..s.clone() })?;
        }
        n += 1;
    }
    let net_for = |c: &BlockCopper| -> Option<String> {
        if let Some(pin) = &c.pin {
            if let Some((r, pad)) = pin.rsplit_once('.') {
                if let Some(net) = nets_of.get(r).and_then(|m| m.get(pad)) {
                    return Some(net.clone());
                }
            }
        }
        c.net.clone().filter(|n| !n.starts_with("Net-("))
    };
    for c in &layout.copper {
        let net = net_for(c);
        if let Some(v) = c.via {
            tx.run("pcb.via.create", json!({ "board": board, "at": abs([v[0], v[1]]), "drill": v[2], "diameter": v[3], "net": net }))?;
        } else if let Some(layer) = &c.layer {
            tx.run("pcb.track.create", json!({ "board": board, "layer": layer, "width": c.width, "points": c.points.iter().map(|p| abs(*p)).collect::<Vec<_>>(), "net": net }))?;
        }
    }
    Ok(n)
}

/// `pcb.block.update`: rename or describe a block.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateBlock {
    pub target: StableId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
}

impl Command for UpdateBlock {
    const NAME: &'static str = "pcb.block.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = tx.get::<DesignBlock>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a design block", self.target)))?;
        if let Some(n) = self.name {
            let n = n.trim().to_string();
            if n.is_empty() {
                return Err(bad("a design block needs a name"));
            }
            if find(tx.world(), &n).is_some_and(|(id, _)| id != self.target) {
                return Err(bad(format!("there is a design block “{n}” already")));
            }
            b.name = n;
        }
        if let Some(d) = self.description {
            b.description = d;
        }
        if let Some(k) = self.keywords {
            b.keywords = k;
        }
        tx.set(self.target, b)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.block.delete`.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteBlock {
    pub target: StableId,
}

impl Command for DeleteBlock {
    const NAME: &'static str = "pcb.block.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        tx.get::<DesignBlock>(self.target).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a design block", self.target)))?;
        tx.delete(self.target)?;
        Ok(json!({ "deleted": self.target }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.component::<DesignBlock>().command::<SaveBlock>().command::<PlaceBlock>().command::<UpdateBlock>().command::<DeleteBlock>();
}
