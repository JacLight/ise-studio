//! The stock library: real bought-in parts (ISO screws, nuts, washers,
//! bearings, servos, steppers, extrusion…) as shape programs with their true
//! geometry, `stock()` metadata and connectors, sizes as configurations.
//!
//! The library lives in `library/stock` at the repository root and is built
//! into the engine (see `build.rs`). Each family is a manifest
//! (`<id>.json`: name, standard, materials with densities, parameters, the
//! configuration table, test checks) plus the program files it joins
//! (`common.js` + `<id>.js`). `stock.insert` brings a family into a project
//! as a shape definition with its configuration table and places an
//! instance of one size; inserting the same family again reuses that
//! definition.

use crate::{ConfigRow, Param, ShapeProgram};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use de_solid::Material;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

mod files {
    include!(concat!(env!("OUT_DIR"), "/stock_files.rs"));
}

/// Every file of the library, by file name.
pub fn files() -> &'static [(&'static str, &'static str)] {
    files::FILES
}

fn file(name: &str) -> Option<&'static str> {
    files::FILES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// One family of the catalogue: one shape program covering every size.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Family {
    /// Catalogue id, e.g. "iso4762".
    pub id: String,
    /// What people call it: "Socket head cap screw".
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
    pub category: String,
    /// One or two lines: what it is, how it stands in its own coordinates.
    #[serde(default)]
    pub summary: String,
    /// Other names people search by ("SHCS", "DIN 912", "Allen bolt").
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Program files joined in order.
    pub source: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_tolerance: Option<f64>,
    /// Material of a program returning one solid (a key of `materials`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Material name → density (kg/m³), realistic for the item.
    pub materials: BTreeMap<String, f64>,
    pub params: Vec<Param>,
    /// The parameters the sizes set (the configuration table's columns).
    pub columns: Vec<String>,
    /// The sizes: one configuration each.
    pub rows: Vec<ConfigRow>,
    /// The size inserted when none is asked for.
    pub default: String,
    /// What the tests assert per size (overall size, connectors, metrics).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub checks: BTreeMap<String, Value>,
}

impl Family {
    /// The program's source: its files joined.
    pub fn program_source(&self) -> String {
        self.source.iter().map(|f| file(f).unwrap_or_default()).collect::<Vec<_>>().join("\n")
    }

    pub fn row(&self, name: &str) -> Option<&ConfigRow> {
        self.rows.iter().find(|r| r.name == name)
    }

    /// The parameters with a size's values in place.
    pub fn params_for(&self, row: &ConfigRow) -> Vec<Param> {
        let mut params = self.params.clone();
        for p in &mut params {
            if let Some(v) = row.values.get(&p.key) {
                p.value = *v;
            }
        }
        params
    }

    /// Parameter values (key, value) of a size, for evaluating directly.
    pub fn values_for(&self, row: &ConfigRow) -> Vec<(String, f64)> {
        self.params_for(row).into_iter().map(|p| (p.key, p.value)).collect()
    }

    /// The runtime inputs of a size (its name, as the program's `config`).
    pub fn inputs_for(&self, row: &ConfigRow) -> String {
        crate::configs::with_row("{}", row)
    }

    /// The family as a shape program in one size.
    pub fn program(&self, row: &ConfigRow) -> ShapeProgram {
        ShapeProgram {
            name: self.title(),
            description: self.description(),
            source: self.program_source(),
            params: self.params_for(row),
            display_tolerance: self.display_tolerance,
            levels: Vec::new(),
            uses: Default::default(),
            finishes: Vec::new(),
            inputs: self.inputs_for(row),
        }
    }

    /// The shape's name in a project: “Socket head cap screw ISO 4762”.
    pub fn title(&self) -> String {
        match &self.standard {
            Some(s) => format!("{} {s}", self.name),
            None => self.name.clone(),
        }
    }

    fn description(&self) -> String {
        format!("Stock library: {}. {} sizes as configurations.", self.summary.trim_end_matches('.'), self.rows.len())
    }

    fn check(&self) -> Result<(), String> {
        let id = &self.id;
        for f in &self.source {
            if file(f).is_none() {
                return Err(format!("{id}: program file {f} is not in library/stock"));
            }
        }
        for c in &self.columns {
            if !self.params.iter().any(|p| &p.key == c) {
                return Err(format!("{id}: column {c} is not a parameter"));
            }
        }
        if self.row(&self.default).is_none() {
            return Err(format!("{id}: the default size {} is not a row", self.default));
        }
        if let Some(m) = &self.material
            && !self.materials.contains_key(m)
        {
            return Err(format!("{id}: material {m} has no density in materials"));
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.rows {
            if !seen.insert(&r.name) {
                return Err(format!("{id}: two sizes are called {}", r.name));
            }
            if self.columns.iter().any(|c| !r.values.contains_key(c)) || r.values.len() != self.columns.len() {
                return Err(format!("{id}: size {} must give exactly the columns {:?}", r.name, self.columns));
            }
        }
        Ok(())
    }
}

/// The whole catalogue, in display order.
#[derive(Clone, Debug, Serialize)]
pub struct Catalogue {
    pub categories: Vec<String>,
    pub families: Vec<Family>,
}

impl Catalogue {
    pub fn family(&self, id: &str) -> Option<&Family> {
        self.families.iter().find(|f| f.id == id)
    }

    fn ids(&self) -> String {
        self.families.iter().map(|f| f.id.as_str()).collect::<Vec<_>>().join(", ")
    }
}

#[derive(Deserialize)]
struct Index {
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    order: Vec<String>,
}

fn load() -> Result<Catalogue, String> {
    let index: Index = serde_json::from_str(file("catalogue.json").ok_or("library/stock/catalogue.json is missing")?).map_err(|e| format!("catalogue.json: {e}"))?;
    let mut families = Vec::new();
    for (name, text) in files::FILES {
        if !name.ends_with(".json") || *name == "catalogue.json" {
            continue;
        }
        let f: Family = serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        if format!("{}.json", f.id) != *name {
            return Err(format!("{name}: its id is {}", f.id));
        }
        f.check()?;
        families.push(f);
    }
    let rank = |id: &str| index.order.iter().position(|o| o == id).unwrap_or(usize::MAX);
    families.sort_by(|a, b| (rank(&a.id), &a.id).cmp(&(rank(&b.id), &b.id)));
    let mut categories = index.categories;
    for f in &families {
        if !categories.contains(&f.category) {
            categories.push(f.category.clone());
        }
    }
    Ok(Catalogue { categories, families })
}

/// The catalogue built into this engine (parsed once).
pub fn catalogue() -> Result<&'static Catalogue, String> {
    static CATALOGUE: OnceLock<Result<Catalogue, String>> = OnceLock::new();
    CATALOGUE.get_or_init(load).as_ref().map_err(|e| e.clone())
}

/// Marks a shape definition as a family of the stock library, so inserting
/// the family again reuses it.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StockFamily {
    /// Catalogue id, e.g. "iso4762".
    pub family: String,
}

impl Persist for StockFamily {
    const KIND: &'static str = "shape.stock_family";
}

/// The project's definition of a library family, if it has one (the oldest
/// when it somehow has several).
pub fn definition_of(world: &World, family: &str) -> Option<StableId> {
    world
        .resource::<IdIndex>()
        .iter()
        .filter(|(_, e)| world.get::<StockFamily>(*e).is_some_and(|f| f.family == family) && world.get::<ShapeProgram>(*e).is_some())
        .map(|(id, _)| id)
        .min_by_key(|id| id.to_string())
}

fn invalid(m: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, m)
}

/// `stock.insert`: bring a stock-library item into the project in one size.
/// The family becomes (or already is) a shape definition with every size as
/// a configuration and its materials at realistic densities; an instance
/// showing `size` is placed at the top of the project or inside `parent`.
#[derive(Deserialize, JsonSchema)]
pub struct InsertStock {
    /// Catalogue id: "iso4762", "bearing_deep_groove", "servo_hobby"… (GET /stock lists them).
    pub family: String,
    /// The size (a configuration name): "M3×12", "608-2RS", "MG996R". Default: the family's default.
    pub size: Option<String>,
    /// Place an instance (default). `false` only makes sure the definition is in the project.
    #[serde(default = "yes")]
    pub place: bool,
    /// The assembly to place it in; none places it at the top of the project.
    pub parent: Option<StableId>,
    /// The instance's name; default the part number's short form (“M3×12 SHCS”).
    pub name: Option<String>,
    /// Where (mm), in the parent's coordinates.
    #[serde(default)]
    pub at: [f64; 3],
    /// Turn about Z (degrees).
    #[serde(default)]
    pub rotation: f64,
}

fn yes() -> bool {
    true
}

impl Command for InsertStock {
    const NAME: &'static str = "stock.insert";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let cat = catalogue().map_err(|e| CommandError::new(ErrorCode::Internal, e))?;
        let fam = cat.family(&self.family).ok_or_else(|| invalid(format!("the stock library has no family “{}” (it has {})", self.family, cat.ids())))?;
        let size = self.size.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| fam.default.clone());
        let row = fam.row(&size).ok_or_else(|| {
            let some: Vec<&str> = fam.rows.iter().take(12).map(|r| r.name.as_str()).collect();
            invalid(format!("{} has no size “{size}” (sizes: {}{})", fam.title(), some.join(", "), if fam.rows.len() > 12 { ", …" } else { "" }))
        })?;
        let source = fam.program_source();
        let (definition, created, updated) = match definition_of(tx.world(), &fam.id) {
            Some(id) => {
                // Reused. A definition made from an older library is brought
                // up to date (program and sizes), keeping its id and instances.
                let current = tx.get::<ShapeProgram>(id).cloned();
                let stale = current.as_ref().is_some_and(|p| p.source != source || p.params.iter().map(|q| &q.key).ne(fam.params.iter().map(|q| &q.key)));
                let table_stale = tx.get::<crate::ShapeConfigurations>(id).is_none_or(|t| t.rows != fam.rows || t.params != fam.columns);
                if stale {
                    let materials = ensure_materials(tx, fam)?;
                    tx.run(
                        "shape.update",
                        json!({ "target": id, "source": source, "params": fam.params_for(fam.row(&fam.default).unwrap_or(row)), "materials": materials.0, "display_tolerance": fam.display_tolerance }),
                    )?;
                }
                if stale || table_stale {
                    tx.run("shape.config.set_table", json!({ "target": id, "params": fam.columns, "rows": fam.rows }))?;
                }
                (id, false, stale || table_stale)
            }
            None => {
                let (materials, single) = (ensure_materials(tx, fam)?, fam.material.clone());
                let default = fam.row(&fam.default).unwrap_or(row);
                let mut create = json!({
                    "name": fam.title(),
                    "description": fam.description(),
                    "source": source,
                    "params": fam.params_for(default),
                    "display_tolerance": fam.display_tolerance,
                    "definition": true,
                    "materials": materials.0,
                });
                if let Some(m) = single {
                    create["material"] = json!(materials.0.get(&m));
                }
                let out = tx.run("shape.create", create)?;
                let id: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
                tx.set(id, StockFamily { family: fam.id.clone() })?;
                tx.run("shape.config.set_table", json!({ "target": id, "params": fam.columns, "rows": fam.rows, "active": fam.default }))?;
                (id, true, false)
            }
        };
        let mut instance = Value::Null;
        if self.place {
            let name = self.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} {}", fam.name, row.name));
            let out = tx.run(
                "assembly.instance.create",
                json!({ "of": definition, "parent": self.parent, "name": name, "at": self.at, "rotation": self.rotation, "config": row.name }),
            )?;
            instance = out["id"].clone();
        }
        Ok(json!({ "id": instance, "definition": definition, "family": fam.id, "size": row.name, "created": created, "updated": updated }))
    }
}

/// The family's materials in the project: a material with the same name
/// and density is reused, otherwise made. Returns name → material id.
fn ensure_materials(tx: &mut Tx, fam: &Family) -> Result<crate::PartMaterials, CommandError> {
    let mut table = BTreeMap::new();
    for (name, density) in &fam.materials {
        let world = tx.world();
        let found = world
            .resource::<IdIndex>()
            .iter()
            .filter(|(_, e)| world.get::<Material>(*e).is_some_and(|m| &m.name == name && (m.density.canonical() - density).abs() < 0.5))
            .map(|(id, _)| id)
            .min_by_key(|id| id.to_string());
        let id = match found {
            Some(id) => id,
            None => {
                let out = tx.run("solid.material.create", json!({ "name": name, "density": { "value": density, "unit": "kg/m3" } }))?;
                serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
            }
        };
        table.insert(name.clone(), id);
    }
    Ok(crate::PartMaterials(table))
}

// ---- Browsing ------------------------------------------------------------------

/// The catalogue as the stock browser shows it: every family with every size
/// worked out (part number, description, typical price, catalogue mass,
/// overall size, connectors, metrics, where to buy it). Evaluated once per
/// engine, in parallel; sizes that fail to evaluate say why.
pub fn summary() -> Result<&'static Value, String> {
    static SUMMARY: OnceLock<Result<Value, String>> = OnceLock::new();
    SUMMARY.get_or_init(|| catalogue().map(summarize)).as_ref().map_err(|e| e.clone())
}

fn summarize(cat: &Catalogue) -> Value {
    use rayon::prelude::*;
    let families: Vec<Value> = cat
        .families
        .par_iter()
        .map(|f| {
            let source = f.program_source();
            // Metadata does not depend on accuracy: evaluate coarsely.
            let tol = f.display_tolerance.unwrap_or(crate::runtime::DISPLAY_TOLERANCE).max(0.02);
            let sizes: Vec<Value> = f
                .rows
                .par_iter()
                .map(|row| match crate::runtime::evaluate_in(&source, &f.values_for(row), tol, &f.inputs_for(row)) {
                    Ok(e) => {
                        let s = e.declared.stock.clone().unwrap_or_default();
                        json!({
                            "name": row.name,
                            "values": row.values,
                            "partNumber": s.part_number,
                            "description": s.description,
                            "manufacturer": s.manufacturer,
                            "mpn": s.mpn,
                            "unitPrice": s.unit_price,
                            "currency": s.currency,
                            "pack": s.pack,
                            "massG": s.mass_g,
                            "sources": s.sources,
                            "min": e.bounds_min.map(|v| (v * 100.0).round() / 100.0),
                            "size": ([0, 1, 2].map(|k| ((e.bounds_max[k] - e.bounds_min[k]) * 100.0).round() / 100.0)),
                            "connectors": e.declared.connectors.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                            "metrics": e.metrics.iter().map(|(k, m)| (k.clone(), json!({ "value": m.value, "unit": m.unit }))).collect::<serde_json::Map<_, _>>(),
                            "parts": e.parts.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                        })
                    }
                    Err(err) => json!({ "name": row.name, "values": row.values, "error": err }),
                })
                .collect();
            let columns: Vec<Value> = f
                .columns
                .iter()
                .filter_map(|c| f.params.iter().find(|p| &p.key == c))
                .map(|p| json!({ "key": p.key, "label": p.label, "unit": p.unit }))
                .collect();
            json!({
                "id": f.id,
                "name": f.name,
                "title": f.title(),
                "standard": f.standard,
                "category": f.category,
                "summary": f.summary,
                "keywords": f.keywords,
                "default": f.default,
                "columns": columns,
                "materials": f.materials,
                "sizes": sizes,
            })
        })
        .collect();
    json!({ "categories": cat.categories, "families": families })
}

/// One size's geometry for a preview: positions (mm), normals, indices and
/// the parts' index ranges with their materials.
pub fn preview(family: &str, size: Option<&str>) -> Result<Value, String> {
    let cat = catalogue()?;
    let f = cat.family(family).ok_or_else(|| format!("the stock library has no family “{family}”"))?;
    let name = size.unwrap_or(&f.default);
    let row = f.row(name).ok_or_else(|| format!("{} has no size “{name}”", f.title()))?;
    let tol = f.display_tolerance.unwrap_or(crate::runtime::DISPLAY_TOLERANCE).max(0.02);
    let e = crate::runtime::evaluate_in(&f.program_source(), &f.values_for(row), tol, &f.inputs_for(row))?;
    let round = |v: &f32| (v * 1000.0).round() / 1000.0;
    Ok(json!({
        "family": f.id,
        "size": row.name,
        "material": f.material,
        "positions": e.positions.iter().map(round).collect::<Vec<_>>(),
        "normals": e.normals.iter().map(|v| (v * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
        "indices": e.indices,
        "bounds": [e.bounds_min, e.bounds_max],
        "parts": e.parts.iter().map(|p| json!({ "name": p.name, "material": p.material, "start": p.index_start, "count": p.index_count })).collect::<Vec<_>>(),
        "connectors": e.declared.connectors,
    }))
}
