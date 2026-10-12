//! Schematic ↔ board: the traceable netlist (DE-850).
//!
//! "Update board from schematic" works out what the board should hold — one
//! component per reference, its footprint, value and sourcing fields, and the
//! net of every pad the symbol pins land on — and compares it with the board:
//! parts to add (placed in a row below the board, to be placed), parts whose
//! fields or pad nets change, and parts no symbol has. Those are removed
//! unless they are board-only (mounting holes and other parts without
//! electrical pads, or references listed in the schematic's `board_only`).
//! Every change is reported; nothing happens silently, and `dry_run` reports
//! without changing anything (GET /pcb/schematic/<id> carries the same
//! pending report). Empty MPN, maker, LCSC and description fields in the
//! schematic leave the board's as they are. Pin names go to new parts.
//!
//! "Back-annotate" goes the other way for what the board owns: reference
//! changes made on the board (renumbering by position), and footprint and
//! value swaps, are written back to the symbols linked to those components.

use crate::schematic::{self, SchSymbol, Schematic};
use crate::schematic_view::SchematicView;
use crate::{PcbComponent, bad, natural};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, bevy_ecs::world::World, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct Added {
    pub reference: String,
    pub footprint: String,
    pub value: String,
    pub at: [f64; 2],
    pub pads: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Changed {
    pub reference: String,
    pub changes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Removed {
    pub reference: String,
    pub footprint: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Kept {
    pub reference: String,
    pub footprint: String,
    pub why: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct NetChange {
    /// "R1.2"
    pub pad: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Skipped {
    pub reference: String,
    pub why: String,
}

/// What "Update board from schematic" does (or would do).
#[derive(Clone, Debug, Serialize)]
pub struct SyncReport {
    pub schematic: StableId,
    pub board: StableId,
    pub dry_run: bool,
    pub added: Vec<Added>,
    pub changed: Vec<Changed>,
    pub nets: Vec<NetChange>,
    pub removed: Vec<Removed>,
    pub board_only: Vec<Kept>,
    pub skipped: Vec<Skipped>,
    /// Nets the board gains or loses as a whole.
    pub new_nets: Vec<String>,
    pub gone_nets: Vec<String>,
    pub unchanged: usize,
    pub erc_errors: usize,
    /// Nothing to do: the board matches the schematic.
    pub in_sync: bool,
}

/// One board component as the schematic wants it.
#[derive(Clone, Debug)]
pub struct Expected {
    pub reference: String,
    pub footprint: String,
    pub value: String,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    pub description: String,
    pub dnp: bool,
    pub pins: BTreeMap<String, String>,
    pub pin_names: BTreeMap<String, String>,
    pub symbols: Vec<StableId>,
    pub link: Option<StableId>,
}

/// The board components a schematic describes, by reference.
pub fn expected(v: &SchematicView, it: &schematic::SchItems) -> (Vec<Expected>, Vec<Skipped>) {
    let mut by: BTreeMap<String, Expected> = BTreeMap::new();
    let mut skipped = Vec::new();
    for sv in &v.symbols {
        if sv.virtual_part || sv.exclude_from_board || sv.reference.starts_with('#') {
            continue;
        }
        if sv.reference.contains('?') {
            skipped.push(Skipped { reference: sv.reference.clone(), why: "not annotated".into() });
            continue;
        }
        if !sv.known {
            skipped.push(Skipped { reference: sv.reference.clone(), why: format!("unknown symbol {}", sv.symbol) });
            continue;
        }
        let e = by.entry(sv.reference.to_ascii_uppercase()).or_insert_with(|| Expected {
            reference: sv.reference.clone(),
            footprint: sv.footprint.clone(),
            value: sv.value.clone(),
            mpn: sv.mpn.clone(),
            manufacturer: sv.manufacturer.clone(),
            lcsc: sv.lcsc.clone(),
            description: sv.description.clone(),
            dnp: sv.dnp,
            pins: BTreeMap::new(),
            pin_names: BTreeMap::new(),
            symbols: vec![],
            link: None,
        });
        e.symbols.push(sv.id);
        if e.link.is_none() {
            e.link = sv.component;
        }
        for p in &sv.pins {
            for pad in &p.pads {
                if let Some(n) = &p.net {
                    e.pins.insert(pad.clone(), n.clone());
                }
                if !p.name.is_empty() && p.name != "~" && !p.name.starts_with("Pin_") {
                    e.pin_names.insert(pad.clone(), p.name.clone());
                }
            }
        }
        // Hidden pins of other units still name their pads.
        if let Some((_, inst)) = it.symbols.iter().find(|(id, _)| *id == sv.id) {
            if let Some(def) = inst.def() {
                for p in def.pins.iter().filter(|p| !p.in_unit(sv.unit)) {
                    for pad in inst.pads_of(p) {
                        if !p.name.is_empty() && p.name != "~" && !p.name.starts_with("Pin_") {
                            e.pin_names.entry(pad).or_insert(p.name.clone());
                        }
                    }
                }
            }
        }
    }
    let mut list: Vec<Expected> = by.into_values().collect();
    list.retain(|e| {
        if e.footprint.is_empty() {
            skipped.push(Skipped { reference: e.reference.clone(), why: "no footprint chosen".into() });
            false
        } else {
            true
        }
    });
    list.sort_by(|a, b| natural(&a.reference, &b.reference));
    (list, skipped)
}

enum Action {
    Create(Expected, [f64; 2]),
    Update(StableId, PcbComponent, Vec<StableId>),
    Delete(StableId),
    Link(Vec<StableId>, StableId),
}

fn mechanical(board: &crate::Board, c: &PcbComponent) -> Option<String> {
    let fp = board.footprint(&c.footprint)?;
    let electrical = fp.pads.iter().any(|p| !p.number.is_empty() && p.kind != crate::footprints::PadKind::Npth);
    if !electrical || fp.kind == "hole" {
        Some(if fp.kind == "hole" || c.footprint.starts_with("MountingHole") { "mounting hole".into() } else { "no electrical pads".into() })
    } else {
        None
    }
}

fn plan(world: &World, id: StableId) -> Result<(SyncReport, Vec<Action>), CommandError> {
    let e = crate::entity(world, id).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a schematic")))?;
    let s: Schematic = world.get::<Schematic>(e).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a schematic")))?;
    let board_id = s.board.ok_or_else(|| bad("the schematic drives no board: link one (pcb.schematic.update { board })"))?;
    let board = crate::entity(world, board_id).and_then(|e| world.get::<crate::Board>(e)).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{board_id} is not a board")))?;
    let it = schematic::items(world, id);
    let v = crate::schematic_view::resolve_items(id, &s, &it, world);
    let (want, mut skipped) = expected(&v, &it);
    let have = crate::items(world, board_id).components;
    let mut actions = Vec::new();
    let mut report = SyncReport {
        schematic: id,
        board: board_id,
        dry_run: true,
        added: vec![],
        changed: vec![],
        nets: vec![],
        removed: vec![],
        board_only: vec![],
        skipped: vec![],
        new_nets: vec![],
        gone_nets: vec![],
        unchanged: 0,
        erc_errors: v.errors(),
        in_sync: false,
    };
    let mut matched: BTreeSet<StableId> = BTreeSet::new();
    // Where new parts wait to be placed: rows below the board.
    let (bx0, by0, bx1) = {
        let o = &board.outline;
        if o.is_empty() { (0.0, 0.0, 50.0) } else { (o.iter().map(|p| p[0]).fold(f64::MAX, f64::min), o.iter().map(|p| p[1]).fold(f64::MAX, f64::min), o.iter().map(|p| p[0]).fold(f64::MIN, f64::max)) }
    };
    let row_w = (bx1 - bx0).max(40.0) * 1.4;
    let mut cursor = 0.0;
    let mut row_top = by0 - 4.0;
    let mut row_h: f64 = 0.0;
    for w in want {
        let found = w.link.and_then(|l| have.iter().find(|(cid, _)| *cid == l)).or_else(|| have.iter().find(|(_, c)| c.reference.eq_ignore_ascii_case(&w.reference)));
        match found {
            Some((cid, c)) => {
                matched.insert(*cid);
                let mut n = c.clone();
                let mut changes = Vec::new();
                if c.reference != w.reference {
                    changes.push(format!("reference {} → {}", c.reference, w.reference));
                    n.reference = w.reference.clone();
                }
                if c.footprint != w.footprint {
                    changes.push(format!("footprint {} → {}", c.footprint, w.footprint));
                    n.footprint = w.footprint.clone();
                }
                if c.value != w.value {
                    changes.push(format!("value {} → {}", show(&c.value), show(&w.value)));
                    n.value = w.value.clone();
                }
                for (what, have_v, want_v, slot) in [("MPN", &c.mpn, &w.mpn, 0), ("maker", &c.manufacturer, &w.manufacturer, 1), ("LCSC", &c.lcsc, &w.lcsc, 2)] {
                    if want_v.is_some() && have_v != want_v {
                        changes.push(format!("{what} {} → {}", have_v.as_deref().unwrap_or("—"), want_v.as_deref().unwrap_or("—")));
                        match slot {
                            0 => n.mpn = want_v.clone(),
                            1 => n.manufacturer = want_v.clone(),
                            _ => n.lcsc = want_v.clone(),
                        }
                    }
                }
                if !w.description.is_empty() && c.description != w.description {
                    changes.push("description".into());
                    n.description = w.description.clone();
                }
                if c.dnp != w.dnp {
                    changes.push(if w.dnp { "do not populate".into() } else { "populate".into() });
                    n.dnp = w.dnp;
                }
                let pads: BTreeSet<&String> = c.pins.keys().chain(w.pins.keys()).collect();
                let mut net_changed = false;
                for pad in pads {
                    let from = c.pins.get(pad);
                    let to = w.pins.get(pad);
                    if from != to {
                        net_changed = true;
                        report.nets.push(NetChange { pad: format!("{}.{pad}", w.reference), from: from.cloned(), to: to.cloned() });
                    }
                }
                if net_changed {
                    n.pins = w.pins.clone();
                    let k = report.nets.iter().filter(|x| x.pad.starts_with(&format!("{}.", w.reference))).count();
                    changes.push(format!("{k} pad net{}", if k == 1 { "" } else { "s" }));
                }
                if changes.is_empty() && n == *c {
                    report.unchanged += 1;
                } else {
                    if !changes.is_empty() {
                        report.changed.push(Changed { reference: w.reference.clone(), changes });
                    }
                    actions.push(Action::Update(*cid, n, w.symbols.clone()));
                }
                if w.link != Some(*cid) {
                    actions.push(Action::Link(w.symbols.clone(), *cid));
                }
            }
            None => {
                let Some(fp) = board.footprint(&w.footprint) else {
                    skipped.push(Skipped { reference: w.reference.clone(), why: format!("no footprint “{}”", w.footprint) });
                    continue;
                };
                let cy = fp.courtyard_rect();
                let (cw, ch) = (cy[2] - cy[0], cy[3] - cy[1]);
                if cursor > 0.0 && cursor + cw > row_w {
                    cursor = 0.0;
                    row_top -= row_h + 2.0;
                    row_h = 0.0;
                }
                let at = [r3(bx0 + cursor - cy[0]), r3(row_top - cy[3])];
                cursor += cw + 2.0;
                row_h = row_h.max(ch);
                report.added.push(Added { reference: w.reference.clone(), footprint: w.footprint.clone(), value: w.value.clone(), at, pads: w.pins.len() });
                actions.push(Action::Create(w, at));
            }
        }
    }
    for (cid, c) in &have {
        if matched.contains(cid) {
            continue;
        }
        let listed = s.board_only.iter().any(|r| r.eq_ignore_ascii_case(&c.reference));
        if let Some(why) = mechanical(&board, c).or(listed.then(|| "listed as board-only".to_string())) {
            report.board_only.push(Kept { reference: c.reference.clone(), footprint: c.footprint.clone(), why });
        } else {
            report.removed.push(Removed { reference: c.reference.clone(), footprint: c.footprint.clone(), value: c.value.clone() });
            actions.push(Action::Delete(*cid));
        }
    }
    // Nets gained and lost as a whole.
    let before: BTreeSet<String> = have.iter().flat_map(|(_, c)| c.pins.values().cloned()).collect();
    let mut after: BTreeSet<String> = BTreeSet::new();
    for a in &actions {
        match a {
            Action::Create(w, _) => after.extend(w.pins.values().cloned()),
            Action::Update(_, n, _) => after.extend(n.pins.values().cloned()),
            _ => {}
        }
    }
    for (cid, c) in &have {
        let touched = actions.iter().any(|a| matches!(a, Action::Update(x, ..) | Action::Delete(x) if x == cid));
        if !touched {
            after.extend(c.pins.values().cloned());
        }
    }
    report.new_nets = after.difference(&before).cloned().collect();
    report.gone_nets = before.difference(&after).cloned().collect();
    report.skipped = skipped;
    report.in_sync = report.added.is_empty() && report.changed.is_empty() && report.nets.is_empty() && report.removed.is_empty() && !actions.iter().any(|a| matches!(a, Action::Update(..)));
    Ok((report, actions))
}

fn show(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The pending "Update board" report, without changing anything.
pub fn pending(world: &World, id: StableId) -> Result<SyncReport, CommandError> {
    plan(world, id).map(|(r, _)| r)
}

/// `pcb.schematic.update_board`: make the board match the schematic (parts,
/// fields, pad nets) and report every change; `dry_run` only reports.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateBoardFromSchematic {
    pub schematic: StableId,
    #[serde(default)]
    pub dry_run: bool,
    /// Where new parts go, by reference (otherwise in rows below the board).
    #[serde(default)]
    pub place: BTreeMap<String, Placement>,
}

/// A new part's place on the board.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct Placement {
    pub at: [f64; 2],
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub side: crate::Side,
}

impl Command for UpdateBoardFromSchematic {
    const NAME: &'static str = "pcb.schematic.update_board";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if !self.dry_run {
            // Footprints from the project libraries go on the board first.
            crate::library::ensure_schematic_footprints(tx, self.schematic)?;
        }
        let (mut report, actions) = plan(tx.world(), self.schematic)?;
        if self.dry_run {
            return Ok(serde_json::to_value(&report).unwrap());
        }
        report.dry_run = false;
        let board = report.board;
        // Deletes first (their references may be reused), then renames through
        // temporary names so swaps never collide, then everything else.
        let deletes: Vec<StableId> = actions.iter().filter_map(|a| if let Action::Delete(id) = a { Some(*id) } else { None }).collect();
        if !deletes.is_empty() {
            tx.run("pcb.delete", json!({ "targets": deletes }))?;
        }
        let mut n = 0;
        for a in &actions {
            if let Action::Update(id, want, _) = a {
                if tx.get::<PcbComponent>(*id).is_some_and(|c| c.reference != want.reference) {
                    n += 1;
                    tx.run("pcb.component.update", json!({ "target": id, "reference": format!("Xsync{n}") }))?;
                }
            }
        }
        for a in &actions {
            match a {
                Action::Update(id, c, symbols) => {
                    tx.run(
                        "pcb.component.update",
                        json!({ "target": id, "reference": c.reference, "footprint": c.footprint, "value": c.value, "mpn": c.mpn.clone().unwrap_or_default(), "manufacturer": c.manufacturer.clone().unwrap_or_default(), "lcsc": c.lcsc.clone().unwrap_or_default(), "description": c.description, "dnp": c.dnp, "pins": c.pins, "pin_names": c.pin_names }),
                    )?;
                    link(tx, symbols, *id)?;
                }
                Action::Create(w, at) => {
                    let pl = self.place.get(&w.reference).cloned().unwrap_or(Placement { at: *at, rotation: 0.0, side: crate::Side::Top });
                    if let Some(a) = report.added.iter_mut().find(|a| a.reference == w.reference) {
                        a.at = pl.at;
                    }
                    let out = tx.run(
                        "pcb.component.create",
                        json!({ "board": board, "reference": w.reference, "footprint": w.footprint, "value": w.value, "at": pl.at, "rotation": pl.rotation, "side": pl.side, "mpn": w.mpn, "manufacturer": w.manufacturer, "lcsc": w.lcsc, "description": w.description, "dnp": w.dnp, "pins": w.pins, "pin_names": w.pin_names }),
                    )?;
                    let cid: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
                    link(tx, &w.symbols, cid)?;
                }
                Action::Link(symbols, cid) => link(tx, symbols, *cid)?,
                Action::Delete(_) => {}
            }
        }
        Ok(serde_json::to_value(&report).unwrap())
    }
}

fn link(tx: &mut Tx, symbols: &[StableId], cid: StableId) -> Result<(), CommandError> {
    for s in symbols {
        if let Some(sym) = tx.get::<SchSymbol>(*s).cloned() {
            if sym.component != Some(cid) {
                tx.set(*s, SchSymbol { component: Some(cid), ..sym })?;
            }
        }
    }
    Ok(())
}

/// `pcb.schematic.back_annotate`: write reference, footprint and value
/// changes made on the board back to the linked symbols.
#[derive(Deserialize, JsonSchema)]
pub struct BackAnnotate {
    pub schematic: StableId,
    #[serde(default)]
    pub dry_run: bool,
}

impl Command for BackAnnotate {
    const NAME: &'static str = "pcb.schematic.back_annotate";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = tx.get::<Schematic>(self.schematic).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a schematic", self.schematic)))?;
        let board = s.board.ok_or_else(|| bad("the schematic drives no board"))?;
        let it = schematic::items(tx.world(), self.schematic);
        let comps = crate::items(tx.world(), board).components;
        let mut renamed = Vec::new();
        let mut changed = Vec::new();
        let mut linked = 0;
        let mut updates: Vec<(StableId, SchSymbol)> = Vec::new();
        for (sid, sym) in &it.symbols {
            if sym.reference.starts_with('#') {
                continue;
            }
            let c = sym.component.and_then(|l| comps.iter().find(|(cid, _)| *cid == l)).or_else(|| comps.iter().find(|(_, c)| c.reference.eq_ignore_ascii_case(&sym.reference)));
            let Some((cid, c)) = c else { continue };
            let mut n = sym.clone();
            if n.component != Some(*cid) {
                n.component = Some(*cid);
                linked += 1;
            }
            if c.reference != sym.reference {
                if !renamed.iter().any(|r: &Value| r["from"] == json!(sym.reference)) {
                    renamed.push(json!({ "from": sym.reference, "to": c.reference }));
                }
                n.reference = c.reference.clone();
            }
            if c.footprint != sym.footprint {
                changed.push(json!({ "reference": c.reference, "change": format!("footprint {} → {}", show(&sym.footprint), c.footprint) }));
                n.footprint = c.footprint.clone();
            }
            if c.value != sym.value {
                changed.push(json!({ "reference": c.reference, "change": format!("value {} → {}", show(&sym.value), show(&c.value)) }));
                n.value = c.value.clone();
            }
            if n != *sym {
                updates.push((*sid, n));
            }
        }
        if !self.dry_run {
            for (sid, n) in updates {
                tx.set(sid, n)?;
            }
        }
        Ok(json!({ "dry_run": self.dry_run, "renamed": renamed, "changed": changed, "linked": linked }))
    }
}
