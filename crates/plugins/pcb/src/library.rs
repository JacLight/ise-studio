//! Project libraries: symbols and footprints made or copied for this
//! project, kept in the project log like everything else (so teammates see
//! the same parts, and versions and branches carry them), grouped in named
//! libraries, and a library table saying which libraries the project uses.
//!
//! | kind              | struct           | what                                               |
//! |-------------------|------------------|----------------------------------------------------|
//! | `pcb.lib_symbol`  | [`LibSymbol`]    | a symbol (`Library:Name`): pins, drawing per unit and body style, fields; derived from another (`extends`), or a power symbol |
//! | `pcb.lib_footprint` | [`LibFootprint`] | a footprint (`Library:Name`) with its 3D model placement |
//! | `pcb.library_table` | [`LibraryTable`] | the libraries in use: project, global (this computer), KiCad; enabled, pinned |
//!
//! Placed symbols carry a copy of their definition (as KiCad's schematics
//! do), so a schematic stands on its own; `pcb.lib.symbol.push` brings
//! library changes to the parts already placed, and `pcb.lib.footprint.push`
//! to the boards using a footprint. A footprint from a project library goes
//! onto a board under its full name (`MyParts:SOT-583`) the first time a
//! part needs it.

use crate::footprints::Footprint;
use crate::schematic::{self, LabelKind, SchSymbol};
use crate::symbols::{self, SymbolDef};
use crate::{bad, board_of, libedit};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_LIBRARY: &str = "Project";

/// A power symbol: placing it puts a power port on the sheet (a global net by name).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PowerSymbol {
    /// The net it names: GND, 3V3, VBUS, +12V.
    pub net: String,
    /// ground, bar, arrow or circle (from the name when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// `pcb.lib_symbol`: a symbol in a project library.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LibSymbol {
    pub library: String,
    /// The symbol; `symbol.name` is its name in the library (no `Library:`).
    pub symbol: SymbolDef,
    /// Derived from this symbol (`Library:Name`, or a built-in name): its pins and drawing come from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerSymbol>,
    /// Where it was copied from ("built-in R", "KiCad Device:R").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl Persist for LibSymbol {
    const KIND: &'static str = "pcb.lib_symbol";
}

/// Where a footprint's 3D model sits: moved and turned from the footprint origin.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Model3d {
    /// A model file the part has elsewhere (STEP/WRL path or URL), kept for exports and the person;
    /// the board's 3D part draws the footprint's body blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// mm: x, y on the board plane, z up.
    #[serde(default)]
    pub offset: [f64; 3],
    /// Degrees about z (counter-clockwise from the top).
    #[serde(default)]
    pub rotation: f64,
}

/// `pcb.lib_footprint`: a footprint in a project library.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LibFootprint {
    pub library: String,
    /// The footprint; `footprint.name` is its name in the library (no `Library:`).
    pub footprint: Footprint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model3d>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl Persist for LibFootprint {
    const KIND: &'static str = "pcb.lib_footprint";
}

/// One row of the library table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LibraryEntry {
    pub name: String,
    /// project (in this project), global (a library on this computer), kicad (the official KiCad libraries), builtin.
    pub kind: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

fn yes() -> bool {
    true
}

/// `pcb.library_table`: which libraries this project uses (one per project).
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LibraryTable {
    pub libraries: Vec<LibraryEntry>,
}

impl Persist for LibraryTable {
    const KIND: &'static str = "pcb.library_table";
}

// ---- Reading -------------------------------------------------------------------

pub fn lib_symbols(world: &World) -> Vec<(StableId, LibSymbol)> {
    let mut v: Vec<(StableId, LibSymbol)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<LibSymbol>(e).map(|x| (id, x.clone()))).collect();
    v.sort_by(|a, b| (a.1.library.as_str(), a.1.symbol.name.as_str()).cmp(&(b.1.library.as_str(), b.1.symbol.name.as_str())));
    v
}

pub fn lib_footprints(world: &World) -> Vec<(StableId, LibFootprint)> {
    let mut v: Vec<(StableId, LibFootprint)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<LibFootprint>(e).map(|x| (id, x.clone()))).collect();
    v.sort_by(|a, b| (a.1.library.as_str(), a.1.footprint.name.as_str()).cmp(&(b.1.library.as_str(), b.1.footprint.name.as_str())));
    v
}

pub fn table(world: &World) -> Option<(StableId, LibraryTable)> {
    world.resource::<IdIndex>().iter().find_map(|(id, e)| world.get::<LibraryTable>(e).map(|x| (id, x.clone())))
}

/// Split `Library:Name` (a name alone matches any project library).
fn split(id: &str) -> (Option<&str>, &str) {
    match id.split_once(':') {
        Some((l, n)) => (Some(l.trim()), n.trim()),
        None => (None, id.trim()),
    }
}

pub fn full_id(library: &str, name: &str) -> String {
    format!("{library}:{name}")
}

/// A project library symbol by `Library:Name` (or name).
pub fn find_symbol(world: &World, id: &str) -> Option<(StableId, LibSymbol)> {
    let (lib, name) = split(id);
    lib_symbols(world).into_iter().find(|(_, s)| s.symbol.name == name && lib.is_none_or(|l| s.library == l))
}

pub fn find_footprint(world: &World, id: &str) -> Option<(StableId, LibFootprint)> {
    let (lib, name) = split(id);
    lib_footprints(world).into_iter().find(|(_, s)| s.footprint.name == name && lib.is_none_or(|l| s.library == l))
}

/// A symbol definition ready to place: derived symbols flattened onto their
/// parent (project library or built-in), named `Library:Name`.
pub fn resolve_symbol(world: &World, id: &str) -> Result<SymbolDef, String> {
    resolve_depth(world, id, 0)
}

fn resolve_depth(world: &World, id: &str, depth: usize) -> Result<SymbolDef, String> {
    if depth > 8 {
        return Err(format!("{id}: symbols derive from each other in a loop"));
    }
    if let Some((_, s)) = find_symbol(world, id) {
        let mut def = match &s.extends {
            Some(parent) => {
                let p = resolve_depth(world, parent, depth + 1)?;
                libedit::flatten(&s.symbol, &p)
            }
            None => s.symbol.clone(),
        };
        def.name = full_id(&s.library, &s.symbol.name);
        if s.power.is_some() {
            def.virtual_part = true;
        }
        return Ok(def);
    }
    symbols::get(id).ok_or_else(|| format!("no symbol “{id}” in the project libraries or the built-in library"))
}

/// A footprint ready for a board: named `Library:Name`, outlines written, the
/// 3D model's offset and turn applied to its body.
pub fn placeable_footprint(f: &LibFootprint) -> Footprint {
    let mut fp = f.footprint.clone();
    fp.name = full_id(&f.library, &f.footprint.name);
    fp.pads.iter_mut().for_each(|p| p.normalize());
    if let Some(m) = &f.model {
        for b in &mut fp.body {
            let at = crate::geom::rotate(b.at, m.rotation);
            b.at = [at[0] + m.offset[0], at[1] + m.offset[1]];
            b.z = [b.z[0] + m.offset[2], b.z[1] + m.offset[2]];
            if (m.rotation / 90.0).round() as i64 % 2 != 0 {
                b.size = [b.size[1], b.size[0]];
            }
        }
    }
    fp
}

/// Put a project-library footprint on a board when the board has none by that name.
pub fn ensure_footprint(tx: &mut Tx, board: StableId, name: &str) -> Result<bool, CommandError> {
    if name.trim().is_empty() {
        return Ok(false);
    }
    let Ok(b) = board_of(tx, board) else { return Ok(false) };
    if b.footprint(name).is_some() {
        return Ok(false);
    }
    let Some((_, lf)) = find_footprint(tx.world(), name) else { return Ok(false) };
    let mut fp = placeable_footprint(&lf);
    fp.name = name.to_string();
    tx.run("pcb.footprint.define", json!({ "board": board, "footprint": fp }))?;
    Ok(true)
}

/// Every symbol's footprint on a schematic's board, from the project libraries where the board lacks it.
pub fn ensure_schematic_footprints(tx: &mut Tx, schematic: StableId) -> Result<(), CommandError> {
    let Some(board) = tx.get::<schematic::Schematic>(schematic).and_then(|s| s.board) else { return Ok(()) };
    let it = schematic::items(tx.world(), schematic);
    let names: std::collections::BTreeSet<String> = it.symbols.iter().map(|(_, s)| s.footprint.clone()).filter(|f| !f.is_empty()).collect();
    for n in names {
        ensure_footprint(tx, board, &n)?;
    }
    Ok(())
}

// ---- Checks ---------------------------------------------------------------------

fn check_name(what: &str, s: &str) -> Result<String, CommandError> {
    let s = s.trim();
    if s.is_empty() || s.len() > 96 || s.contains(':') || s.contains('/') || s.chars().any(|c| c.is_control()) {
        return Err(bad(format!("“{s}” is not a {what} name (1 to 96 characters, no ':' or '/')")));
    }
    Ok(s.to_string())
}

fn check_def(def: &SymbolDef, derived: bool, power: bool) -> Result<(), CommandError> {
    if !derived && !power && def.pins.is_empty() {
        return Err(bad(format!("{} has no pins (a derived symbol names its parent in `extends`)", def.name)));
    }
    for p in &def.pins {
        for v in p.at {
            if !v.is_finite() || v.abs() > 1000.0 {
                return Err(bad(format!("pin {}: coordinates are numbers within ±1000 mm", p.number)));
            }
        }
        if !(0.0..=50.0).contains(&p.length) {
            return Err(bad(format!("pin {}: length is 0 to 50 mm", p.number)));
        }
    }
    if !derived && def.units.is_empty() {
        return Err(bad("a symbol has at least one unit (its drawing may be empty)"));
    }
    Ok(())
}

// ---- Commands -------------------------------------------------------------------

/// `pcb.lib.symbol.save`: add a symbol to a project library, or replace the
/// one of that name (`rename_from` renames it at the same time).
#[derive(Deserialize, JsonSchema)]
pub struct SaveSymbol {
    /// Library name (default "Project").
    pub library: Option<String>,
    pub symbol: SymbolDef,
    pub extends: Option<String>,
    pub power: Option<PowerSymbol>,
    pub origin: Option<String>,
    /// The `Library:Name` it had before (a rename or a move to another library).
    pub rename_from: Option<String>,
}

impl Command for SaveSymbol {
    const NAME: &'static str = "pcb.lib.symbol.save";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let library = check_name("library", self.library.as_deref().unwrap_or(DEFAULT_LIBRARY))?;
        let mut def = self.symbol;
        def.name = check_name("symbol", &def.name)?;
        if def.prefix.trim().is_empty() {
            def.prefix = if self.power.is_some() { "#PWR".into() } else { "U".into() };
        }
        if let Some(p) = &self.extends {
            if p.trim() == full_id(&library, &def.name) || p.trim() == def.name {
                return Err(bad("a symbol cannot derive from itself"));
            }
            resolve_symbol(tx.world(), p).map_err(bad)?;
        }
        if let Some(p) = &self.power {
            if p.net.trim().is_empty() {
                return Err(bad("a power symbol names its net (GND, 3V3, +12V)"));
            }
        }
        check_def(&def, self.extends.is_some(), self.power.is_some())?;
        let id_new = full_id(&library, &def.name);
        let existing = find_symbol(tx.world(), &id_new);
        let renamed = self.rename_from.as_deref().and_then(|r| find_symbol(tx.world(), r));
        if let (Some((a, _)), Some((b, _))) = (&existing, &renamed) {
            if a != b {
                return Err(bad(format!("{id_new} is already in the library")));
            }
        }
        let value = LibSymbol { library: library.clone(), symbol: def, extends: self.extends.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()), power: self.power, origin: self.origin };
        let (id, created) = match renamed.or(existing) {
            Some((id, _)) => (id, false),
            None => (tx.create(), true),
        };
        tx.set(id, value)?;
        let def = resolve_symbol(tx.world(), &id_new).map_err(bad)?;
        let fp = crate::footprints::get(&def.footprint).or_else(|| find_footprint(tx.world(), &def.footprint).map(|(_, f)| placeable_footprint(&f)));
        let findings = libedit::check_symbol(&def, fp.as_ref());
        Ok(json!({ "id": id, "symbol": id_new, "created": created, "findings": findings }))
    }
}

/// `pcb.lib.footprint.save`: add a footprint to a project library, or replace the one of that name.
#[derive(Deserialize, JsonSchema)]
pub struct SaveFootprint {
    pub library: Option<String>,
    pub footprint: Footprint,
    pub model: Option<Model3d>,
    pub origin: Option<String>,
    pub rename_from: Option<String>,
}

impl Command for SaveFootprint {
    const NAME: &'static str = "pcb.lib.footprint.save";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let library = check_name("library", self.library.as_deref().unwrap_or(DEFAULT_LIBRARY))?;
        let mut fp = self.footprint;
        fp.name = check_name("footprint", &fp.name)?;
        fp.pads.iter_mut().for_each(|p| p.normalize());
        crate::check_footprint(&fp.name, &fp)?;
        if let Some(m) = &self.model {
            if m.offset.iter().any(|v| !v.is_finite() || v.abs() > 100.0) || !m.rotation.is_finite() {
                return Err(bad("the 3D model's offset is within ±100 mm"));
            }
        }
        let id_new = full_id(&library, &fp.name);
        let existing = find_footprint(tx.world(), &id_new);
        let renamed = self.rename_from.as_deref().and_then(|r| find_footprint(tx.world(), r));
        if let (Some((a, _)), Some((b, _))) = (&existing, &renamed) {
            if a != b {
                return Err(bad(format!("{id_new} is already in the library")));
            }
        }
        let value = LibFootprint { library, footprint: fp, model: self.model, origin: self.origin };
        let findings = crate::fpedit::check_footprint(&placeable_footprint(&value), &crate::Rules::default());
        let (id, created) = match renamed.or(existing) {
            Some((id, _)) => (id, false),
            None => (tx.create(), true),
        };
        tx.set(id, value)?;
        Ok(json!({ "id": id, "footprint": id_new, "created": created, "findings": findings }))
    }
}

/// `pcb.lib.delete`: remove symbols or footprints from the project libraries (by `Library:Name`).
#[derive(Deserialize, JsonSchema)]
pub struct DeleteLibItems {
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub footprints: Vec<String>,
}

impl Command for DeleteLibItems {
    const NAME: &'static str = "pcb.lib.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut n = 0;
        for s in &self.symbols {
            let (id, _) = find_symbol(tx.world(), s).ok_or_else(|| bad(format!("no project symbol {s}")))?;
            let users: Vec<String> = lib_symbols(tx.world()).into_iter().filter(|(_, x)| x.extends.as_deref().is_some_and(|e| e == s || find_symbol(tx.world(), e).is_some_and(|(i, _)| i == id))).map(|(_, x)| full_id(&x.library, &x.symbol.name)).collect();
            if !users.is_empty() {
                return Err(bad(format!("{s} is the parent of {}: delete or flatten them first", users.join(", "))));
            }
            tx.delete(id)?;
            n += 1;
        }
        for f in &self.footprints {
            let (id, _) = find_footprint(tx.world(), f).ok_or_else(|| bad(format!("no project footprint {f}")))?;
            tx.delete(id)?;
            n += 1;
        }
        Ok(json!({ "deleted": n }))
    }
}

/// `pcb.lib.copy`: duplicate a symbol or footprint (built-in or project) into a project library to edit it.
#[derive(Deserialize, JsonSchema)]
pub struct CopyLibItem {
    /// symbol or footprint
    pub kind: String,
    /// Built-in name (`R`, `SOIC-8`) or `Library:Name` of a project part.
    pub from: String,
    pub library: Option<String>,
    /// New name (default: the source's name).
    pub name: Option<String>,
    /// A symbol: copy the parent's body into it (flatten) instead of deriving from it.
    #[serde(default = "yes")]
    pub flatten: bool,
}

impl Command for CopyLibItem {
    const NAME: &'static str = "pcb.lib.copy";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let library = self.library.unwrap_or_else(|| DEFAULT_LIBRARY.into());
        if self.kind.starts_with("sym") {
            let (def, power, extends) = match find_symbol(tx.world(), &self.from) {
                Some((_, s)) if !self.flatten => (s.symbol.clone(), s.power.clone(), s.extends.clone()),
                Some((_, s)) => {
                    let mut d = resolve_symbol(tx.world(), &self.from).map_err(bad)?;
                    d.name = s.symbol.name.clone();
                    (d, s.power.clone(), None)
                }
                None => (symbols::get(&self.from).ok_or_else(|| bad(format!("no symbol {}", self.from)))?, None, None),
            };
            let mut def = def;
            def.name = self.name.clone().unwrap_or_else(|| split(&def.name).1.to_string());
            let out = tx.run("pcb.lib.symbol.save", json!({ "library": library, "symbol": def, "power": power, "extends": extends, "origin": format!("copy of {}", self.from) }))?;
            Ok(out)
        } else {
            let (fp, model) = match find_footprint(tx.world(), &self.from) {
                Some((_, f)) => (f.footprint.clone(), f.model.clone()),
                None => (crate::footprints::get(&self.from).ok_or_else(|| bad(format!("no footprint {}", self.from)))?, None),
            };
            let mut fp = fp;
            fp.name = self.name.clone().unwrap_or_else(|| split(&fp.name).1.to_string());
            tx.run("pcb.lib.footprint.save", json!({ "library": library, "footprint": fp, "model": model, "origin": format!("copy of {}", self.from) }))
        }
    }
}

/// `pcb.lib.place`: place a project-library symbol on a schematic (its
/// footprint goes on the board too). A power symbol puts a power port.
#[derive(Deserialize, JsonSchema)]
pub struct PlaceLibSymbol {
    pub schematic: StableId,
    /// `Library:Name`.
    pub symbol: String,
    pub at: [f64; 2],
    pub sheet: Option<String>,
    pub reference: Option<String>,
    pub value: Option<String>,
    pub footprint: Option<String>,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub mirror: bool,
    pub unit: Option<u32>,
    /// 2: the De Morgan body.
    pub body_style: Option<u8>,
}

impl Command for PlaceLibSymbol {
    const NAME: &'static str = "pcb.lib.place";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if let Some((_, s)) = find_symbol(tx.world(), &self.symbol) {
            if let Some(p) = &s.power {
                let style = p.style.clone();
                return tx.run("pcb.schematic.label", json!({ "schematic": self.schematic, "sheet": self.sheet, "kind": LabelKind::Power, "text": p.net, "at": self.at, "rotation": self.rotation, "style": style }));
            }
        }
        let mut def = resolve_symbol(tx.world(), &self.symbol).map_err(bad)?;
        if self.body_style == Some(2) {
            def = libedit::swap_body_style(&def).ok_or_else(|| bad(format!("{} has no De Morgan body", def.name)))?;
        }
        let footprint = self.footprint.clone().unwrap_or(def.footprint.clone());
        if let Some(board) = tx.get::<schematic::Schematic>(self.schematic).and_then(|s| s.board) {
            ensure_footprint(tx, board, &footprint)?;
        }
        let fields = def.fields.clone();
        tx.run(
            "pcb.schematic.place",
            json!({ "schematic": self.schematic, "sheet": self.sheet, "symbol": def.name, "definition": def, "at": self.at, "rotation": self.rotation, "mirror": self.mirror, "unit": self.unit, "reference": self.reference, "value": self.value, "footprint": if footprint.is_empty() { None } else { Some(footprint) }, "fields": fields }),
        )
    }
}

/// `pcb.lib.body_style`: draw placed parts in their other body style (De Morgan), or back.
#[derive(Deserialize, JsonSchema)]
pub struct BodyStyle {
    pub targets: Vec<StableId>,
    /// 1 normal, 2 De Morgan.
    pub style: u8,
}

impl Command for BodyStyle {
    const NAME: &'static str = "pcb.lib.body_style";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut changed = 0;
        for t in &self.targets {
            let s = tx.get::<SchSymbol>(*t).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{t} is not a placed symbol")))?;
            let def = s.def().ok_or_else(|| bad(format!("{} has no definition", s.reference)))?;
            if def.body_style.max(1) == self.style.max(1) {
                continue;
            }
            let swapped = libedit::swap_body_style(&def).ok_or_else(|| bad(format!("{} ({}) has no De Morgan body", s.reference, def.name)))?;
            tx.set(*t, SchSymbol { definition: Some(Box::new(swapped)), ..s })?;
            changed += 1;
        }
        Ok(json!({ "changed": changed }))
    }
}

/// `pcb.lib.symbol.push`: bring a library symbol's current definition to the
/// parts placed from it (body style, pin map and fields of each part kept).
#[derive(Deserialize, JsonSchema)]
pub struct PushSymbol {
    /// `Library:Name`.
    pub symbol: String,
    /// Only this schematic (default every one).
    pub schematic: Option<StableId>,
}

impl Command for PushSymbol {
    const NAME: &'static str = "pcb.lib.symbol.push";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let def = resolve_symbol(tx.world(), &self.symbol).map_err(bad)?;
        let mut updated = Vec::new();
        for (sid, _) in schematic::schematics(tx.world()) {
            if self.schematic.is_some_and(|s| s != sid) {
                continue;
            }
            for (id, s) in schematic::items(tx.world(), sid).symbols {
                if s.symbol != def.name {
                    continue;
                }
                let style = s.definition.as_ref().map(|d| d.body_style).unwrap_or(1);
                let mut d = def.clone();
                if style == 2 {
                    d = libedit::swap_body_style(&d).unwrap_or(d);
                }
                if d.unit_count() < s.unit {
                    return Err(bad(format!("{} is unit {} but {} now has {} unit(s)", s.reference, s.unit, def.name, d.unit_count())));
                }
                let mut fields = s.fields.clone();
                for (k, v) in &def.fields {
                    fields.entry(k.clone()).or_insert(v.clone());
                }
                updated.push(s.reference.clone());
                tx.set(id, SchSymbol { definition: Some(Box::new(d)), fields, ..s })?;
            }
        }
        Ok(json!({ "updated": updated }))
    }
}

/// `pcb.lib.footprint.push`: redefine a library footprint on the boards that use it.
#[derive(Deserialize, JsonSchema)]
pub struct PushFootprint {
    pub footprint: String,
    pub board: Option<StableId>,
}

impl Command for PushFootprint {
    const NAME: &'static str = "pcb.lib.footprint.push";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (_, lf) = find_footprint(tx.world(), &self.footprint).ok_or_else(|| bad(format!("no project footprint {}", self.footprint)))?;
        let fp = placeable_footprint(&lf);
        let mut boards = Vec::new();
        for (bid, b) in crate::boards(tx.world()) {
            if self.board.is_some_and(|x| x != bid) {
                continue;
            }
            let used = b.footprints.contains_key(&fp.name) || crate::items(tx.world(), bid).components.iter().any(|(_, c)| c.footprint == fp.name);
            if used {
                tx.run("pcb.footprint.define", json!({ "board": bid, "footprint": fp }))?;
                boards.push(bid);
            }
        }
        Ok(json!({ "boards": boards }))
    }
}

/// `pcb.lib.assign`: choose footprints for placed symbols by reference (CvPcb).
/// Project-library footprints go on the board; each pin's pads are checked
/// against the footprint and the misses reported.
#[derive(Deserialize, JsonSchema)]
pub struct AssignFootprints {
    pub schematic: StableId,
    /// Reference → footprint name ("" clears).
    pub footprints: BTreeMap<String, String>,
}

impl Command for AssignFootprints {
    const NAME: &'static str = "pcb.lib.assign";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = tx.get::<schematic::Schematic>(self.schematic).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a schematic", self.schematic)))?.board;
        let it = schematic::items(tx.world(), self.schematic);
        let mut warnings = Vec::new();
        let mut assigned = 0;
        for (r, fp_name) in &self.footprints {
            let fp_name = fp_name.trim().to_string();
            let parts: Vec<&(StableId, SchSymbol)> = it.symbols.iter().filter(|(_, s)| s.reference.eq_ignore_ascii_case(r)).collect();
            if parts.is_empty() {
                return Err(bad(format!("no {r} on the schematic")));
            }
            if !fp_name.is_empty() {
                if let Some(b) = board {
                    ensure_footprint(tx, b, &fp_name)?;
                }
                let fp = board.and_then(|b| board_of(tx, b).ok()).and_then(|b| b.footprint(&fp_name)).or_else(|| crate::footprints::get(&fp_name)).or_else(|| find_footprint(tx.world(), &fp_name).map(|(_, f)| placeable_footprint(&f)));
                match fp {
                    None => warnings.push(format!("{r}: no footprint “{fp_name}” here yet (a KiCad one goes on the board with `./forma pcb footprint`)")),
                    Some(fp) => {
                        if let Some(def) = parts[0].1.def() {
                            let missing: Vec<String> = def.pins.iter().filter(|p| !(p.hidden && p.etype == symbols::PinType::NoConnect)).flat_map(|p| parts[0].1.pads_of(p)).filter(|pad| !fp.pads.iter().any(|q| &q.number == pad)).collect();
                            if !missing.is_empty() {
                                warnings.push(format!("{r}: pads {} are not on {fp_name} (give a pin map)", missing.join(", ")));
                            }
                        }
                    }
                }
            }
            for (id, _) in parts {
                tx.run("pcb.schematic.symbol.update", json!({ "target": id, "footprint": fp_name }))?;
            }
            assigned += 1;
        }
        Ok(json!({ "assigned": assigned, "warnings": warnings }))
    }
}

/// `pcb.lib.table.set`: the project's library table (replaces it).
#[derive(Deserialize, JsonSchema)]
pub struct SetTable {
    pub libraries: Vec<LibraryEntry>,
}

impl Command for SetTable {
    const NAME: &'static str = "pcb.lib.table.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut seen = std::collections::BTreeSet::new();
        for l in &self.libraries {
            check_name("library", &l.name)?;
            if !["project", "global", "kicad", "builtin"].contains(&l.kind.as_str()) {
                return Err(bad(format!("{}: kind is project, global, kicad or builtin", l.name)));
            }
            if !seen.insert((l.kind.clone(), l.name.clone())) {
                return Err(bad(format!("{} is in the table twice", l.name)));
            }
        }
        let id = match table(tx.world()) {
            Some((id, _)) => id,
            None => tx.create(),
        };
        tx.set(id, LibraryTable { libraries: self.libraries })?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.lib.import`: bring many parts into a project library at once (a global library copied into the project).
#[derive(Deserialize, JsonSchema)]
pub struct ImportLib {
    pub library: String,
    #[serde(default)]
    pub symbols: Vec<LibSymbolIn>,
    #[serde(default)]
    pub footprints: Vec<LibFootprintIn>,
    pub origin: Option<String>,
}

#[derive(Deserialize, Serialize, JsonSchema, Clone)]
pub struct LibSymbolIn {
    pub symbol: SymbolDef,
    pub extends: Option<String>,
    pub power: Option<PowerSymbol>,
}

#[derive(Deserialize, Serialize, JsonSchema, Clone)]
pub struct LibFootprintIn {
    pub footprint: Footprint,
    pub model: Option<Model3d>,
}

impl Command for ImportLib {
    const NAME: &'static str = "pcb.lib.import";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut n = 0;
        // Footprints first (symbols' checks look them up), parents before derived symbols.
        for f in &self.footprints {
            tx.run("pcb.lib.footprint.save", json!({ "library": self.library, "footprint": f.footprint, "model": f.model, "origin": self.origin }))?;
            n += 1;
        }
        let (base, derived): (Vec<&LibSymbolIn>, Vec<&LibSymbolIn>) = self.symbols.iter().partition(|s| s.extends.is_none());
        for s in base.into_iter().chain(derived) {
            tx.run("pcb.lib.symbol.save", json!({ "library": self.library, "symbol": s.symbol, "extends": s.extends, "power": s.power, "origin": self.origin }))?;
            n += 1;
        }
        Ok(json!({ "imported": n }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.component::<LibSymbol>()
        .component::<LibFootprint>()
        .component::<LibraryTable>()
        .command::<SaveSymbol>()
        .command::<SaveFootprint>()
        .command::<DeleteLibItems>()
        .command::<CopyLibItem>()
        .command::<PlaceLibSymbol>()
        .command::<BodyStyle>()
        .command::<PushSymbol>()
        .command::<PushFootprint>()
        .command::<AssignFootprints>()
        .command::<SetTable>()
        .command::<ImportLib>();
}

/// Library entries for listing: every project library with its parts.
pub fn listing(world: &World) -> Value {
    let mut libs: BTreeMap<String, (Vec<Value>, Vec<Value>)> = BTreeMap::new();
    for (id, s) in lib_symbols(world) {
        let def = resolve_symbol(world, &full_id(&s.library, &s.symbol.name));
        let (pins, units, desc) = match &def {
            Ok(d) => (d.pins.iter().filter(|p| !p.hidden).count(), d.unit_count(), d.description.clone()),
            Err(_) => (0, 0, String::new()),
        };
        libs.entry(s.library.clone()).or_default().0.push(json!({ "id": id, "name": s.symbol.name, "full": full_id(&s.library, &s.symbol.name), "description": desc, "extends": s.extends, "power": s.power, "pins": pins, "units": units, "demorgan": def.as_ref().map(|d| !d.demorgan.is_empty()).unwrap_or(false), "footprint": s.symbol.footprint, "origin": s.origin, "error": def.err() }));
    }
    for (id, f) in lib_footprints(world) {
        libs.entry(f.library.clone()).or_default().1.push(json!({ "id": id, "name": f.footprint.name, "full": full_id(&f.library, &f.footprint.name), "description": f.footprint.description, "pads": f.footprint.pads.iter().filter(|p| !p.number.is_empty()).count(), "kind": f.footprint.kind, "model": f.model, "origin": f.origin }));
    }
    json!(libs.into_iter().map(|(name, (s, f))| json!({ "name": name, "symbols": s, "footprints": f })).collect::<Vec<_>>())
}
