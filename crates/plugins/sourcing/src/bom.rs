//! The bill of materials.
//!
//! Walks what is placed — a shape, an instance, an assembly, or the whole
//! project — in the configuration each thing shows (the shape's active one,
//! an instance's own, parts left out by `parts_off` gone), multiplying
//! quantities down nested assemblies. Every leaf becomes a line:
//!
//! - **Make**: a fabricated part (anything whose process is not "buy"), with
//!   material, process, size, volume, mass, the files a shop needs and an
//!   estimated cost (see [`crate::rates`]). Identical parts — same name,
//!   material, process, size and volume — are one line with a quantity.
//! - **Buy**: a stock item, from `stock()` (the whole shape is bought) or a
//!   part's `stock` (one part is bought), merged by part number across the
//!   whole BOM and rounded up to whole packs. A part whose process is "buy"
//!   but that declares no stock is still listed, and flagged.
//!
//! Other parts of the app add Buy lines through [`BuySource`] (a PCB adds its
//! electronic components): each is asked about every leaf the walk reaches.

use crate::{
    Award, Supplier, price_at,
    rates::{self, Estimate, PartFacts, Rates},
};
use de_assembly::{AssemblyDef, Instance};
use de_core::{
    IdIndex, StableId,
    bevy_ecs::{entity::Entity, world::World},
};
use de_shape::{PartMaterials, ShapeConfigurations, ShapeProgram, ShapeSolid, ShapeStats, fabricate, stock::Stock};
use de_solid::{BoxFeature, Material, MaterialRef, Placement};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::rc::Rc;

/// What a BOM is of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scope {
    /// Everything placed at the top of the project.
    Project,
    /// A shape, box, instance or assembly definition.
    Entity(StableId),
}

#[derive(Clone, Debug)]
pub struct Options {
    /// For a shape: the configuration to list (default: the one it shows).
    pub config: Option<String>,
    /// Finished units to build: line totals and extended prices are for this many.
    pub quantity: u32,
    /// Production quantities to total at (price breaks).
    pub breaks: Vec<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Options { config: None, quantity: 1, breaks: vec![1, 10, 100] }
    }
}

/// A bought item another source contributes, per one of the thing it is on.
#[derive(Clone, Debug)]
pub struct BuyItem {
    /// What the model calls it.
    pub name: String,
    /// As a buyer orders it; None flags it as lacking a part number.
    pub stock: Option<Stock>,
    pub qty: u32,
    /// Mass of one (kg), when known from the model (the stock's catalogue
    /// mass wins over it).
    pub mass_kg: Option<f64>,
}

/// Another source of Buy lines (a PCB's components, a wiring harness's
/// connectors). Asked about every leaf the BOM walk reaches.
pub trait BuySource: Send + Sync {
    /// Shown as the lines' origin ("pcb").
    fn name(&self) -> &'static str;
    /// The bought items one of `entity` brings, shown in `config`; None when
    /// the entity is nothing to this source. An entity only this source knows
    /// (not a shape or assembly) is a leaf of its own.
    fn items(&self, world: &World, entity: StableId, config: Option<&str>) -> Option<Vec<BuyItem>>;
    /// Entities of this source's own kind placed at the top of the project.
    fn roots(&self, _world: &World) -> Vec<StableId> {
        Vec::new()
    }
}

/// An award as the BOM shows it.
#[derive(Clone, Debug, Serialize)]
pub struct AwardView {
    pub id: StableId,
    pub supplier: StableId,
    pub supplier_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<StableId>,
    /// At the order quantity of this BOM.
    pub unit_price: Option<f64>,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_days: Option<u32>,
}

/// A fabricated part.
#[derive(Clone, Debug, Serialize)]
pub struct MakeLine {
    pub key: String,
    pub item: usize,
    pub name: String,
    /// Where its geometry comes from: a shape (in a configuration) and, for
    /// a multi-part program, the part.
    pub shape: StableId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_id: Option<StableId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    /// How many one finished unit needs.
    pub qty: u32,
    pub volume_cm3: f64,
    /// Each.
    pub mass_kg: Option<f64>,
    /// Overall size, largest first (mm).
    pub size_mm: [f64; 3],
    /// Through-cut length (mm), for sheet parts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_mm: Option<f64>,
    /// Files a shop gets for it ("step", "pdf", "stl", "3mf", "dxf").
    pub files: Vec<&'static str>,
    pub estimate: Estimate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub award: Option<AwardView>,
    /// Each, at this BOM's quantity: the award's price, else the estimate.
    pub unit_cost: Option<f64>,
    /// unit_cost × qty × production quantity.
    pub extended: Option<f64>,
    /// "award", "estimate" or "none".
    pub cost_source: &'static str,
    /// Where it is used (assembly paths), the first few.
    pub used_in: Vec<String>,
}

/// A stock item.
#[derive(Clone, Debug, Serialize)]
pub struct BuyLine {
    pub key: String,
    pub item: usize,
    /// "stock" (declared by a program) or another source's name.
    pub origin: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<StableId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// None: bought without a part number (flagged).
    pub stock: Option<Stock>,
    /// How many one finished unit needs.
    pub qty: u32,
    /// Each: the catalogue mass, else the modelled one.
    pub mass_kg: Option<f64>,
    /// What to order for this BOM's quantity: rounded up to whole packs.
    pub order_qty: u32,
    /// A cut-to-length item (beam, tube, bar): the metres the order comes
    /// to (piece length × order_qty). Each piece is its own line, since its
    /// length is in its part number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length_m: Option<f64>,
    pub unit_price: Option<f64>,
    /// unit_price × order_qty.
    pub extended: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub award: Option<AwardView>,
    /// "award", "list" (the item's own or a source's price) or "none".
    pub cost_source: &'static str,
    pub used_in: Vec<String>,
}

/// One row of the indented BOM.
#[derive(Clone, Debug, Serialize)]
pub struct Node {
    pub name: String,
    /// "assembly", "shape", "part" or "item".
    pub kind: &'static str,
    /// Per one of its parent.
    pub qty: u32,
    /// In one finished unit.
    pub total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// "make" or "buy" for a line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<StableId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// The instances this row stands for, by name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Flag {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// "warning" or "info".
    pub level: &'static str,
    pub text: String,
}

/// Totals at one production quantity.
#[derive(Clone, Debug, Serialize)]
pub struct Break {
    pub qty: u32,
    pub make: f64,
    pub buy: f64,
    /// (make + buy) / qty.
    pub per_unit: f64,
    pub total: f64,
    /// Every line had a price.
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Totals {
    pub currency: String,
    pub quantity: u32,
    pub make: f64,
    pub buy: f64,
    pub per_unit: f64,
    pub total: f64,
    /// Lines with no price at all (left out of the sums).
    pub unpriced: usize,
    /// Mass of one finished unit, None if some line has no mass.
    pub mass_kg: Option<f64>,
    pub breaks: Vec<Break>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Bom {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<StableId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    pub make: Vec<MakeLine>,
    pub buy: Vec<BuyLine>,
    pub tree: Vec<Node>,
    pub flags: Vec<Flag>,
    pub totals: Totals,
}

impl Bom {
    pub fn make_line(&self, key: &str) -> Option<&MakeLine> {
        self.make.iter().find(|l| l.key == key)
    }

    pub fn buy_line(&self, key: &str) -> Option<&BuyLine> {
        self.buy.iter().find(|l| l.key == key)
    }

    /// The line's name and process (None: bought), whichever class it is.
    pub fn line(&self, key: &str) -> Option<(&str, Option<&str>, u32)> {
        if let Some(l) = self.make_line(key) {
            return Some((&l.name, l.process.as_deref(), l.qty));
        }
        self.buy_line(key).map(|l| (l.name.as_str(), None, l.qty))
    }
}

/// Which files a shop gets for a part made by `process`: STEP and a drawing
/// for machined (and anything not printed or cut from sheet), STL and 3MF for
/// prints, DXF for sheet parts; none for bought items.
pub fn files_for(process: Option<&str>) -> Vec<&'static str> {
    match process {
        Some("3d_print") => vec!["stl", "3mf"],
        Some(p) if fabricate::is_sheet(Some(p)) => vec!["dxf"],
        Some("buy") => vec![],
        // Gerbers and drill files come from the PCB workspace (its fab package).
        Some("pcb_fab") => vec![],
        _ => vec!["step", "pdf"],
    }
}

/// Order quantity for `needed` pieces sold in packs of `pack`.
pub fn pack_round(needed: u32, pack: Option<u32>) -> u32 {
    match pack {
        Some(p) if p > 1 => needed.div_ceil(p) * p,
        _ => needed,
    }
}

/// FNV-1a, so keys stay the same across builds.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// A Make line's key: identical parts (name, material, process, size,
/// volume) share it, so an award stays with the part until it changes.
pub fn make_key(name: &str, material: Option<&str>, process: Option<&str>, size: [f64; 3], volume_mm3: f64) -> String {
    let sig = format!("{name}|{}|{}|{:.2}x{:.2}x{:.2}|{:.1}", material.unwrap_or(""), process.unwrap_or(""), size[0], size[1], size[2], volume_mm3);
    format!("make:{:016x}", fnv(&sig))
}

/// A Buy line's key: its part number (or, lacking one, its name).
pub fn buy_key(name: &str, stock: Option<&Stock>) -> String {
    match stock {
        Some(s) => format!("buy:{}", s.part_number.trim()),
        None => format!("buy:?{}", base_name(name)),
    }
}

/// A name without the number a loop gave it ("Glide 3" → "Glide"), so
/// identical parts numbered one by one are still one line.
pub fn base_name(name: &str) -> &str {
    let t = name.trim();
    let digits = t.trim_end_matches(|c: char| c.is_ascii_digit());
    if digits.len() == t.len() || digits.is_empty() {
        return t;
    }
    let stem = digits.trim_end_matches([' ', '#', '-', '_']);
    if stem.len() < digits.len() && !stem.is_empty() { stem } else { t }
}

fn entity(world: &World, id: StableId) -> Option<Entity> {
    world.resource::<IdIndex>().get(id)
}

fn get<'w, T: de_core::bevy_ecs::component::Component>(world: &'w World, id: StableId) -> Option<&'w T> {
    entity(world, id).and_then(|e| world.get::<T>(e))
}

/// A shape as one configuration of it shows.
pub struct ShapeInfo {
    /// “Bolt (M8×30)”.
    pub name: String,
    pub stats: ShapeStats,
    /// Per part (or one for a single solid).
    pub masses: Vec<Option<f64>>,
    /// Ready to evaluate (for files).
    pub program: ShapeProgram,
    /// The configuration it is shown in, if any.
    pub config: Option<String>,
}

/// Shape `id` in configuration `config` (None: as it is shown).
pub fn shape_info(world: &World, id: StableId, config: Option<&str>) -> Result<ShapeInfo, String> {
    let e = entity(world, id).ok_or_else(|| format!("{id} does not exist"))?;
    let active = world.get::<ShapeConfigurations>(e).and_then(|t| t.active.clone());
    match config.filter(|c| Some(*c) != active.as_deref()) {
        Some(c) => {
            let configured = de_shape::configs::evaluate(world, e, c)?;
            let program = de_shape::configs::configured_program(world, e, c)?;
            let masses = if configured.part_masses.is_empty() { vec![configured.mass.mass_kg] } else { configured.part_masses.clone() };
            Ok(ShapeInfo { name: configured.name.clone(), stats: configured.stats, masses, program, config: Some(c.to_string()) })
        }
        None => {
            let program = de_shape::resolved_program(world, e).ok_or("not a shape")?;
            let stats = world.get::<ShapeSolid>(e).map(|s| s.stats.clone()).ok_or_else(|| format!("{} has no valid result", program.name))?;
            let masses = de_shape::part_masses(world, e)?;
            Ok(ShapeInfo { name: program.name.clone(), stats, masses, program, config: active })
        }
    }
}

/// A program standing in for a plain box part (for its files).
pub fn box_program(name: &str, [w, d, h]: [f64; 3]) -> ShapeProgram {
    let mut p: ShapeProgram = serde_json::from_value(serde_json::json!({ "name": name, "source": format!("return box({w}, {d}, {h});"), "params": [] })).expect("a box program");
    p.inputs = r#"{"sketches":{},"finishes":[]}"#.into();
    p
}

fn box_dims(b: &BoxFeature) -> [f64; 3] {
    [b.width, b.depth, b.height].map(|l| l.canonical() * 1000.0)
}

fn sorted(mut s: [f64; 3]) -> [f64; 3] {
    s.sort_by(|a, b| b.total_cmp(a));
    s
}

/// Through-cut outline length (mm) and number of outlines of a part laid flat.
fn cut_of(program: &ShapeProgram, part: Option<&str>) -> Option<(f64, usize)> {
    let e = de_shape::evaluated_at(program, program.display_tolerance()).ok()?;
    let solid = match part {
        Some(n) => &e.part(n).ok()?.solid,
        None => &e.solid,
    };
    let flat = fabricate::lay_flat(solid);
    let t = fabricate::dimensions(&flat)[2].min(fabricate::dimensions(&flat)[1]);
    let rings = flat.slice_at_z(t * 0.02);
    let mut len = 0.0;
    for r in &rings {
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            len += ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        }
    }
    (len > 0.0).then_some((len, rings.len()))
}

struct MakeAcc {
    line: MakeLine,
    cost: Option<de_solid::MaterialCost>,
    density: Option<f64>,
    /// Closed outlines cut through (sheet parts).
    pierces: usize,
}

struct Walker<'w> {
    world: &'w World,
    sources: &'w [&'w dyn BuySource],
    memo: HashMap<(StableId, Option<String>), Rc<Result<ShapeInfo, String>>>,
    make: Vec<MakeAcc>,
    buy: Vec<BuyLine>,
    flags: Vec<Flag>,
}

const USED_IN: usize = 6;

fn note_use(used: &mut Vec<String>, path: &[String]) {
    let p = if path.is_empty() { "(top)".to_string() } else { path.join(" › ") };
    if used.len() < USED_IN && !used.contains(&p) {
        used.push(p);
    }
}

impl<'w> Walker<'w> {
    fn info(&mut self, id: StableId, config: Option<&str>) -> Rc<Result<ShapeInfo, String>> {
        let key = (id, config.map(str::to_string));
        if let Some(i) = self.memo.get(&key) {
            return i.clone();
        }
        let i = Rc::new(shape_info(self.world, id, config));
        self.memo.insert(key, i.clone());
        i
    }

    fn flag(&mut self, line: Option<&str>, level: &'static str, text: String) {
        if !self.flags.iter().any(|f| f.text == text && f.line.as_deref() == line) {
            self.flags.push(Flag { line: line.map(str::to_string), level, text });
        }
    }

    fn material(&self, id: Option<StableId>) -> Option<&'w Material> {
        id.and_then(|m| get::<Material>(self.world, m))
    }

    #[allow(clippy::too_many_arguments)]
    fn add_make(&mut self, name: String, shape: StableId, config: Option<String>, part: Option<String>, material_id: Option<StableId>, material_name: Option<String>, process: Option<String>, size: [f64; 3], volume_mm3: f64, mass_kg: Option<f64>, program: Option<&ShapeProgram>, total: u32, path: &[String]) -> String {
        let material = self.material(material_id);
        let material_name = material.map(|m| m.name.clone()).or(material_name);
        let key = make_key(base_name(&name), material_name.as_deref(), process.as_deref(), size, volume_mm3);
        if let Some(acc) = self.make.iter_mut().find(|a| a.line.key == key) {
            if acc.line.name != name {
                acc.line.name = base_name(&name).to_string();
            }
            acc.line.qty += total;
            note_use(&mut acc.line.used_in, path);
            return key;
        }
        let cut_mm = if fabricate::is_sheet(process.as_deref()) {
            match program {
                Some(p) => cut_of(p, part.as_deref()),
                None => Some((2.0 * (size[0] + size[1]), 1)),
            }
        } else {
            None
        };
        let mut used_in = vec![];
        note_use(&mut used_in, path);
        let line = MakeLine {
            key: key.clone(),
            item: 0,
            files: files_for(process.as_deref()),
            name,
            shape,
            config,
            part,
            material: material_name,
            material_id: material_id.filter(|_| material.is_some()),
            process,
            qty: total,
            volume_cm3: volume_mm3 / 1000.0,
            mass_kg,
            size_mm: size,
            cut_mm: cut_mm.map(|c| c.0),
            estimate: Estimate::default(),
            award: None,
            unit_cost: None,
            extended: None,
            cost_source: "none",
            used_in,
        };
        self.make.push(MakeAcc { line, cost: material.and_then(|m| m.cost.clone()), density: material.map(|m| m.density.canonical()), pierces: cut_mm.map(|c| c.1).unwrap_or(0) });
        key
    }

    #[allow(clippy::too_many_arguments)]
    fn add_buy(&mut self, item: BuyItem, origin: &str, shape: Option<StableId>, part: Option<String>, total: u32, path: &[String]) -> String {
        let key = buy_key(&item.name, item.stock.as_ref());
        let qty = total * item.qty.max(1);
        if let Some(l) = self.buy.iter_mut().find(|l| l.key == key) {
            if l.stock.is_none() && l.name != item.name {
                l.name = base_name(&item.name).to_string();
            }
            l.qty += qty;
            note_use(&mut l.used_in, path);
            return key;
        }
        if item.stock.is_none() {
            self.flag(Some(&key), "warning", format!("{}: bought part without a part number — give it stock() or use the stock library", base_name(&item.name)));
        }
        let mut used_in = vec![];
        note_use(&mut used_in, path);
        let mass_kg = item.stock.as_ref().and_then(|s| s.mass_g).map(|g| g / 1000.0).or(item.mass_kg);
        self.buy.push(BuyLine {
            key: key.clone(),
            item: 0,
            origin: origin.to_string(),
            name: item.name,
            shape,
            part,
            stock: item.stock,
            qty,
            mass_kg,
            order_qty: qty,
            length_m: None,
            unit_price: None,
            extended: None,
            award: None,
            cost_source: "none",
            used_in,
        });
        key
    }

    /// Bought items other sources say `id` brings, as child rows.
    fn source_items(&mut self, id: StableId, config: Option<&str>, total: u32, path: &[String]) -> (bool, Vec<Node>) {
        let mut claimed = false;
        let mut nodes = vec![];
        for s in self.sources {
            if let Some(items) = s.items(self.world, id, config) {
                claimed = true;
                for item in items {
                    let (name, per) = (item.name.clone(), item.qty.max(1));
                    let key = self.add_buy(item, s.name(), None, None, total, path);
                    nodes.push(Node { name, kind: "item", qty: per, total: total * per, line: Some(key), class: Some("buy"), id: None, config: None, instances: vec![], children: vec![] });
                }
            }
        }
        (claimed, nodes)
    }

    /// The row for definition `id` (a shape, box, assembly or a source's
    /// own entity) shown in `config`, `per_parent` of them in their parent
    /// and `total` in one finished unit.
    fn visit(&mut self, id: StableId, config: Option<String>, per_parent: u32, total: u32, path: &mut Vec<String>, depth: usize) -> Option<Node> {
        if depth > 32 {
            return None;
        }
        let world = self.world;
        let e = entity(world, id)?;
        let node = |name: String, kind: &'static str| Node { name, kind, qty: per_parent, total, line: None, class: None, id: Some(id), config: None, instances: vec![], children: vec![] };

        if let Some(def) = world.get::<AssemblyDef>(e) {
            let mut n = node(def.name.clone(), "assembly");
            path.push(def.name.clone());
            for (of, cfg, names) in group(de_assembly::children(world, id)) {
                let count = names.len() as u32;
                if let Some(mut child) = self.visit(of, cfg, count, total * count, path, depth + 1) {
                    child.instances = names;
                    n.children.push(child);
                }
            }
            path.pop();
            return Some(n);
        }

        if let Some(inst) = world.get::<Instance>(e) {
            let mut child = self.visit(inst.of, inst.config.clone(), per_parent, total, path, depth + 1)?;
            child.instances = vec![inst.name.clone()];
            return Some(child);
        }

        if world.get::<ShapeProgram>(e).is_some() {
            let info = self.info(id, config.as_deref());
            let info = match &*info {
                Ok(i) => i,
                Err(m) => {
                    let name = world.get::<ShapeProgram>(e).map(|p| p.name.clone()).unwrap_or_default();
                    self.flag(None, "warning", format!("{name}: could not be evaluated ({m}); it is left out of the BOM"));
                    return Some(node(name, "shape"));
                }
            };
            let mut n = node(info.name.clone(), "shape");
            n.config = info.config.clone();
            path.push(info.name.clone());
            let declared = &info.stats.declared;
            if let Some(stock) = &declared.stock {
                let key = self.add_buy(BuyItem { name: info.name.clone(), stock: Some(stock.clone()), qty: 1, mass_kg: info.masses.iter().copied().sum::<Option<f64>>() }, "stock", Some(id), None, total, &path[..path.len() - 1]);
                n.line = Some(key);
                n.class = Some("buy");
            } else if info.stats.parts.is_empty() {
                let s = &info.stats;
                let size = sorted([0, 1, 2].map(|k| s.bounds_max[k] - s.bounds_min[k]));
                let default = world.get::<MaterialRef>(e).map(|m| m.0);
                let key = self.add_make(info.name.clone(), id, info.config.clone(), None, default, None, None, size, s.volume_mm3, info.masses.first().copied().flatten(), Some(&info.program), total, &path[..path.len() - 1]);
                n.line = Some(key);
                n.class = Some("make");
            } else {
                let table = world.get::<PartMaterials>(e);
                let default = world.get::<MaterialRef>(e).map(|m| m.0);
                let mut rows: Vec<Node> = vec![];
                for (i, p) in info.stats.parts.iter().enumerate() {
                    let (key, class) = if p.process.as_deref() == Some("buy") {
                        let stock = declared.parts.get(&p.name).cloned();
                        (self.add_buy(BuyItem { name: p.name.clone(), stock, qty: 1, mass_kg: info.masses.get(i).copied().flatten() }, "stock", Some(id), Some(p.name.clone()), total, path), "buy")
                    } else {
                        let size = sorted([0, 1, 2].map(|k| p.bounds_max[k] - p.bounds_min[k]));
                        let mid = match &p.material {
                            Some(m) => table.and_then(|t| t.0.get(m)).copied(),
                            None => default,
                        };
                        let mass = info.masses.get(i).copied().flatten();
                        (self.add_make(p.name.clone(), id, info.config.clone(), Some(p.name.clone()), mid, p.material.clone(), p.process.clone(), size, p.volume_mm3, mass, Some(&info.program), total, path), "make")
                    };
                    match rows.iter_mut().find(|r| r.line.as_deref() == Some(key.as_str())) {
                        Some(r) => {
                            if r.name != p.name {
                                r.name = base_name(&p.name).to_string();
                            }
                            r.qty += 1;
                            r.total += total;
                        }
                        None => rows.push(Node { name: p.name.clone(), kind: "part", qty: 1, total, line: Some(key), class: Some(class), id: None, config: None, instances: vec![], children: vec![] }),
                    }
                }
                n.children = rows;
            }
            let (_, extra) = self.source_items(id, info.config.as_deref(), total, path);
            n.children.extend(extra);
            path.pop();
            return Some(n);
        }

        if let Some(b) = world.get::<BoxFeature>(e) {
            let dims = box_dims(b);
            let material = world.get::<MaterialRef>(e).map(|m| m.0);
            let mass = self.material(material).map(|m| dims[0] * dims[1] * dims[2] / 1e9 * m.density.canonical());
            let key = self.add_make("Box".into(), id, None, None, material, None, None, sorted(dims), dims[0] * dims[1] * dims[2], mass, None, total, path);
            let mut n = node("Box".into(), "shape");
            n.line = Some(key);
            n.class = Some("make");
            return Some(n);
        }

        let (claimed, items) = self.source_items(id, config.as_deref(), total, path);
        claimed.then(|| {
            let mut n = node(format!("{id}"), "item");
            n.children = items;
            n
        })
    }
}

/// Instances grouped by what they show (definition and configuration), in
/// the order first placed: (definition, configuration, instance names).
fn group(children: Vec<(StableId, Instance)>) -> Vec<(StableId, Option<String>, Vec<String>)> {
    let mut out: Vec<(StableId, Option<String>, Vec<String>)> = vec![];
    for (_, inst) in children {
        match out.iter_mut().find(|(of, c, _)| *of == inst.of && *c == inst.config) {
            Some(g) => g.2.push(inst.name),
            None => out.push((inst.of, inst.config, vec![inst.name])),
        }
    }
    out
}

/// The BOM of `scope`, costed with the project's rates, materials and awards.
pub fn compute(world: &World, scope: Scope, options: &Options, sources: &[&dyn BuySource]) -> Result<Bom, String> {
    let mut w = Walker { world, sources, memo: HashMap::new(), make: vec![], buy: vec![], flags: vec![] };
    let mut tree = vec![];
    let (name, root, config) = match scope {
        Scope::Entity(id) => {
            let e = entity(world, id).ok_or_else(|| format!("{id} does not exist"))?;
            let config = options.config.clone().filter(|c| !c.is_empty());
            if config.is_some() && world.get::<ShapeProgram>(e).is_none() {
                return Err("a configuration can be chosen for a shape; instances show their own".into());
            }
            let node = w.visit(id, config.clone(), 1, 1, &mut vec![], 0).ok_or_else(|| format!("{id} has nothing to list: pick a shape, an instance or an assembly"))?;
            let name = node.name.clone();
            tree.push(node);
            (name, Some(id), config)
        }
        Scope::Project => {
            let index = world.resource::<IdIndex>();
            let mut ids: Vec<StableId> = index.iter().map(|(id, _)| id).collect();
            ids.sort();
            let mut top_instances = vec![];
            for id in &ids {
                let Some(e) = entity(world, *id) else { continue };
                if let Some(inst) = world.get::<Instance>(e) {
                    if inst.parent.is_none() {
                        top_instances.push((*id, inst.clone()));
                    }
                } else if (world.get::<ShapeProgram>(e).is_some() && world.get::<Placement>(e).is_some()) || world.get::<BoxFeature>(e).is_some() {
                    if let Some(n) = w.visit(*id, None, 1, 1, &mut vec![], 0) {
                        tree.push(n);
                    }
                }
            }
            for (of, cfg, names) in group(top_instances) {
                let count = names.len() as u32;
                if let Some(mut n) = w.visit(of, cfg, count, count, &mut vec![], 0) {
                    n.instances = names;
                    tree.push(n);
                }
            }
            for s in sources {
                for id in s.roots(world) {
                    if let Some(n) = w.visit(id, None, 1, 1, &mut vec![], 0) {
                        tree.push(n);
                    }
                }
            }
            let title = world
                .resource::<IdIndex>()
                .iter()
                .find_map(|(_, e)| world.get::<de_core::ProjectInfo>(e).map(|p| p.title.clone()))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "Project".into());
            (title, None, None)
        }
    };
    let Walker { make, buy, mut flags, .. } = w;
    let rates = rates::rates_of(world);
    let awards = crate::all::<Award>(world);
    let suppliers: HashMap<StableId, Supplier> = crate::all::<Supplier>(world).into_iter().collect();
    let mut bom = Bom { name, root, config, make: vec![], buy, tree, flags: vec![], totals: empty_totals(&rates, options.quantity) };

    // Estimates.
    for acc in make {
        let mut line = acc.line;
        let pierces = acc.pierces;
        let facts = PartFacts { process: line.process.clone(), size: line.size_mm, volume_mm3: line.volume_cm3 * 1000.0, mass_kg: line.mass_kg, density: acc.density, cut: line.cut_mm.map(|c| (c, pierces)) };
        let rate = line.process.as_deref().and_then(|p| rates.rate(p));
        line.estimate = rates::estimate(&facts, acc.cost.as_ref(), rate.as_ref());
        let key = line.key.clone();
        match (&line.process, &line.material) {
            (None, _) => flags.push(Flag { line: Some(key.clone()), level: "info", text: format!("{}: no process — say how it is made (part process) to estimate it", line.name) }),
            (Some(p), _) if rate.is_none() => flags.push(Flag { line: Some(key.clone()), level: "info", text: format!("{}: no rate for {p}", line.name) }),
            _ => {}
        }
        if line.material.is_none() {
            flags.push(Flag { line: Some(key.clone()), level: "warning", text: format!("{}: no material — no mass and no material cost", line.name) });
        } else if acc.cost.is_none() {
            let m = line.material.clone().unwrap_or_default();
            let text = format!("Material {m} has no price — set its cost per kg, sheet or bar");
            if !flags.iter().any(|f| f.text == text) {
                flags.push(Flag { line: None, level: "info", text });
            }
        } else if let Some(c) = acc.cost.as_ref().and_then(|c| c.currency.clone()).filter(|c| *c != rates.currency()) {
            flags.push(Flag { line: Some(key.clone()), level: "warning", text: format!("{}: its material is priced in {c}, costs are in {} (not converted)", line.name, rates.currency()) });
        }
        bom.make.push(line);
    }
    bom.flags = flags;

    // Item numbers: made parts first, then bought items.
    for (i, l) in bom.make.iter_mut().enumerate() {
        l.item = i + 1;
    }
    let n = bom.make.len();
    for (i, l) in bom.buy.iter_mut().enumerate() {
        l.item = n + i + 1;
    }

    // Awards.
    let award_view = |a: &(StableId, Award), qty: u32| AwardView {
        id: a.0,
        supplier: a.1.supplier,
        supplier_name: suppliers.get(&a.1.supplier).map(|s| s.name.clone()).unwrap_or_else(|| "(deleted supplier)".into()),
        quote: a.1.quote,
        unit_price: price_at(&a.1.prices, qty),
        currency: a.1.currency.clone(),
        lead_days: a.1.lead_days,
    };
    let find_award = |key: &str| awards.iter().find(|a| a.1.line == key);
    let q = options.quantity.max(1);
    for l in &mut bom.make {
        if let Some(a) = find_award(&l.key) {
            l.award = Some(award_view(a, l.qty * q));
            if a.1.currency != rates.currency() {
                bom.flags.push(Flag { line: Some(l.key.clone()), level: "warning", text: format!("{}: awarded in {}, costs are in {} (not converted)", l.name, a.1.currency, rates.currency()) });
            }
        }
    }
    for l in &mut bom.buy {
        l.order_qty = pack_round(l.qty * q, l.stock.as_ref().and_then(|s| s.pack));
        l.length_m = l.stock.as_ref().and_then(|s| s.cut_length_mm).map(|mm| mm * l.order_qty as f64 / 1000.0);
        if let Some(a) = find_award(&l.key) {
            l.award = Some(award_view(a, l.order_qty));
        }
        if let Some(c) = l.stock.as_ref().and_then(|s| s.currency.clone()).filter(|c| *c != rates.currency()) {
            if l.award.is_none() {
                bom.flags.push(Flag { line: Some(l.key.clone()), level: "warning", text: format!("{}: listed in {c}, costs are in {} (not converted)", l.name, rates.currency()) });
            }
        }
    }
    if scope == Scope::Project {
        for (_, a) in &awards {
            if bom.make_line(&a.line).is_none() && bom.buy_line(&a.line).is_none() {
                let name = if a.name.is_empty() { a.line.clone() } else { a.name.clone() };
                bom.flags.push(Flag { line: Some(a.line.clone()), level: "info", text: format!("The award for {name} matches no line now (the part changed or is gone): re-quote it or clear the award") });
            }
        }
    }

    // Costs at this BOM's quantity, then at each break.
    let mut unpriced = 0;
    for l in &mut bom.make {
        let (unit, source) = make_unit(l, &awards, l.qty * q);
        l.unit_cost = unit;
        l.cost_source = source;
        l.extended = unit.map(|u| u * (l.qty * q) as f64);
        if unit.is_none() {
            unpriced += 1;
        }
    }
    for l in &mut bom.buy {
        let (unit, source) = buy_unit(l, &awards, l.order_qty);
        l.unit_price = unit;
        l.cost_source = source;
        l.extended = unit.map(|u| u * l.order_qty as f64);
        if unit.is_none() {
            unpriced += 1;
            if l.stock.is_some() {
                bom.flags.push(Flag { line: Some(l.key.clone()), level: "info", text: format!("{}: no price — give stock() a unitPrice, or get a quote", l.name) });
            }
        }
    }
    let mut breaks: Vec<u32> = options.breaks.iter().copied().chain([q]).filter(|b| *b > 0).collect();
    breaks.sort_unstable();
    breaks.dedup();
    let at = |qty: u32| -> Break {
        let mut make = 0.0;
        let mut buy = 0.0;
        let mut complete = true;
        for l in &bom.make {
            match make_unit(l, &awards, l.qty * qty).0 {
                Some(u) => make += u * (l.qty * qty) as f64,
                None => complete = false,
            }
        }
        for l in &bom.buy {
            let order = pack_round(l.qty * qty, l.stock.as_ref().and_then(|s| s.pack));
            match buy_unit(l, &awards, order).0 {
                Some(u) => buy += u * order as f64,
                None => complete = false,
            }
        }
        Break { qty, make, buy, per_unit: (make + buy) / qty as f64, total: make + buy, complete }
    };
    let breaks: Vec<Break> = breaks.into_iter().map(at).collect();
    let here = breaks.iter().find(|b| b.qty == q).cloned().unwrap();
    let mass = bom.make.iter().map(|l| l.mass_kg.map(|m| m * l.qty as f64)).chain(bom.buy.iter().map(|l| l.mass_kg.map(|m| m * l.qty as f64))).sum::<Option<f64>>();
    bom.totals = Totals { currency: rates.currency(), quantity: q, make: here.make, buy: here.buy, per_unit: here.per_unit, total: here.total, unpriced, mass_kg: mass, breaks };
    Ok(bom)
}

fn empty_totals(rates: &Rates, q: u32) -> Totals {
    Totals { currency: rates.currency(), quantity: q, make: 0.0, buy: 0.0, per_unit: 0.0, total: 0.0, unpriced: 0, mass_kg: None, breaks: vec![] }
}

/// Unit cost of a Make line in a batch: the award's price, else the estimate.
fn make_unit(l: &MakeLine, awards: &[(StableId, Award)], batch: u32) -> (Option<f64>, &'static str) {
    if let Some(p) = awards.iter().find(|a| a.1.line == l.key).and_then(|a| price_at(&a.1.prices, batch)) {
        return (Some(p), "award");
    }
    match l.estimate.unit(batch) {
        Some(u) => (Some(u), "estimate"),
        None => (None, "none"),
    }
}

/// Unit price of a Buy line at an order quantity: the award's, else the
/// item's own list price, else the cheapest source's.
fn buy_unit(l: &BuyLine, awards: &[(StableId, Award)], order: u32) -> (Option<f64>, &'static str) {
    if let Some(p) = awards.iter().find(|a| a.1.line == l.key).and_then(|a| price_at(&a.1.prices, order)) {
        return (Some(p), "award");
    }
    let list = l.stock.as_ref().and_then(|s| s.unit_price.or_else(|| s.sources.iter().filter_map(|x| x.unit_price).min_by(f64::total_cmp)));
    match list {
        Some(p) => (Some(p), "list"),
        None => (None, "none"),
    }
}

// ---- CSV ---------------------------------------------------------------------

pub(crate) fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n']) { format!("\"{}\"", field.replace('"', "\"\"")) } else { field.to_string() }
}

fn num(v: Option<f64>, digits: usize) -> String {
    v.map(|v| format!("{v:.digits$}")).unwrap_or_default()
}

/// The flat BOM as CSV: every line, Make then Buy.
pub fn flat_csv(bom: &Bom) -> String {
    let mut out = String::from("Item,Class,Part number,Name,Description,Standard,Manufacturer,MPN,Material,Process,Qty per unit,Order qty,Length mm,Width mm,Thickness mm,Volume cm3,Mass each kg,Unit cost,Extended,Currency,Cost source,Supplier,Lead days,Used in\n");
    let cur = &bom.totals.currency;
    let q = bom.totals.quantity;
    for l in &bom.make {
        let _ = writeln!(
            out,
            "{},Make,,{},,,,,{},{},{},{},{:.2},{:.2},{:.2},{:.2},{},{},{},{},{},{},{},{}",
            l.item,
            csv(&l.name),
            csv(l.material.as_deref().unwrap_or("")),
            csv(l.process.as_deref().unwrap_or("")),
            l.qty,
            l.qty * q,
            l.size_mm[0],
            l.size_mm[1],
            l.size_mm[2],
            l.volume_cm3,
            num(l.mass_kg, 4),
            num(l.unit_cost, 2),
            num(l.extended, 2),
            cur,
            l.cost_source,
            csv(l.award.as_ref().map(|a| a.supplier_name.as_str()).unwrap_or("")),
            l.award.as_ref().and_then(|a| a.lead_days).map(|d| d.to_string()).unwrap_or_default(),
            csv(&l.used_in.join("; ")),
        );
    }
    for l in &bom.buy {
        let s = l.stock.as_ref();
        let t = |f: fn(&Stock) -> Option<&String>| csv(s.and_then(f).map(String::as_str).unwrap_or(""));
        let _ = writeln!(
            out,
            "{},Buy,{},{},{},{},{},{},,,{},{},{},,,,{},{},{},{},{},{},{},{}",
            l.item,
            csv(s.map(|s| s.part_number.as_str()).unwrap_or("")),
            csv(&l.name),
            t(|s| s.description.as_ref()),
            t(|s| s.standard.as_ref()),
            t(|s| s.manufacturer.as_ref()),
            t(|s| s.mpn.as_ref()),
            l.qty,
            l.order_qty,
            num(s.and_then(|s| s.cut_length_mm), 1),
            num(l.mass_kg, 4),
            num(l.unit_price, 4),
            num(l.extended, 2),
            cur,
            l.cost_source,
            csv(l.award.as_ref().map(|a| a.supplier_name.clone()).or_else(|| s.and_then(|s| s.sources.first()).map(|x| x.supplier.clone())).as_deref().unwrap_or("")),
            l.award.as_ref().and_then(|a| a.lead_days).map(|d| d.to_string()).unwrap_or_default(),
            csv(&l.used_in.join("; ")),
        );
    }
    out
}

/// The indented BOM as CSV: one row per tree row, level by level.
pub fn indented_csv(bom: &Bom) -> String {
    fn rows(out: &mut String, bom: &Bom, nodes: &[Node], level: usize) {
        for n in nodes {
            let item = n.line.as_deref().and_then(|k| bom.make_line(k).map(|l| l.item).or_else(|| bom.buy_line(k).map(|l| l.item)));
            let _ = writeln!(
                out,
                "{level},{},{},{},{},{},{}",
                csv(&format!("{}{}", "  ".repeat(level), n.name)),
                n.qty,
                n.total,
                n.kind,
                n.class.unwrap_or(""),
                item.map(|i| i.to_string()).unwrap_or_default()
            );
            rows(out, bom, &n.children, level + 1);
        }
    }
    let mut out = String::from("Level,Name,Qty per parent,Qty per unit,Kind,Class,Item\n");
    rows(&mut out, bom, &bom.tree, 0);
    out
}

/// Programs (and parts) behind each Make line, for making its files.
pub fn line_programs(world: &World, bom: &Bom) -> BTreeMap<String, (ShapeProgram, Option<String>)> {
    let mut out = BTreeMap::new();
    for l in &bom.make {
        let program = if let Some(b) = get::<BoxFeature>(world, l.shape) {
            Some(box_program(&l.name, box_dims(b)))
        } else {
            shape_info(world, l.shape, l.config.as_deref()).ok().map(|i| i.program)
        };
        if let Some(p) = program {
            out.insert(l.key.clone(), (p, l.part.clone()));
        }
    }
    out
}
