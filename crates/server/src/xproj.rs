//! Insert from another project: pick items of a project in the folder (all
//! of it, or any parts, assemblies, sub-assemblies, boards…) and bring them
//! into this one, copied or linked (`de_core::links`).
//!
//! Here is what knows the design: which entities an item needs (everything
//! it names, followed transitively, plus what belongs to it: an assembly's
//! joints, a board's tracks, the requirements about it), where things are
//! placed, which names must not clash and which materials are the same. The
//! core commands do the generic part (new ids, every reference rewritten,
//! one undoable change).
//!
//! Routes (all with `?project=<this project>`):
//! - `GET  /link/tree?from=<source>`   the source's items as a tree
//! - `POST /link/preview`              what an insert would bring: count, kinds, renames
//! - `POST /link/insert`               insert (`{from, items | whole, copy, place | at}`)
//! - `GET  /links`                     this project's links: up to date or not, and what would change
//! - `POST /links/update`              take the sources' changes (`{link}` or every link)
//! - `POST /links/break`               make a link's items a local copy
//! - `POST /links/settings`            update automatically or not
//!
//! A link updates by itself (when set to) as soon as its source changes,
//! while this engine runs: a watcher notices the source's file change. When
//! the engine starts ([`start`]), every project with links is checked against
//! its sources at once: links set to update by themselves catch up with what
//! changed while it was off, and the others say an update is available.
//!
//! Linked items are read-only here (the engine refuses edits to them, see
//! `de_core::links::guard`) unless their link allows local edits
//! (`/links/settings {link, local_edits}`); where they are placed stays
//! editable.

use crate::{App, Server, bad_request, info_of, pretty_name, valid_name};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use de_assembly::transform::Transform;
use de_core::links::{Link, references};
use de_core::{Actor, Project, StableId, Value, json};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io::BufRead,
    sync::{Arc, Mutex, OnceLock},
};

/// Every entity of a project, components as JSON by kind.
pub type Entities = BTreeMap<StableId, BTreeMap<String, Value>>;

/// A project as read to insert from.
pub struct Source {
    pub name: String,
    pub title: String,
    pub revision: u64,
    pub state: Entities,
}

// ---- Policy -----------------------------------------------------------------------------

/// Entities never brought: the project's own name, versions, comments, and
/// the source's own links (their items come as plain items).
const LEFT_OUT: &[&str] = &["core.project", "core.version", "comment.thread", "comment.reply", "core.link"];
/// Components never brought (a source's link marks).
const STRIPPED: &[&str] = &["core.linked"];
/// Things in their own right: brought only when something needs them or
/// they are picked, never just because they name what is brought.
const DEFINITIONS: &[&str] = &[
    "shape.program", "solid.box", "solid.material", "solid.placement", "assembly.definition", "pcb.board", "shape.sheet", "shape.sketch",
    "draft.object", "draft.layer", "draft.plane", "pcb.lib_symbol", "pcb.lib_footprint", "pcb.library_table", "pcb.design_block",
    "sourcing.supplier", "sourcing.rates", "sourcing.rfq", "sourcing.quote", "sourcing.award",
];
/// Kinds whose name must not repeat one already in the project.
const NAMED: &[&str] = &["shape.program", "assembly.definition", "pcb.board", "pcb.schematic", "shape.parameter_set", "shape.sheet", "shape.sketch"];
/// Kinds that, identical to one already here, are used rather than copied.
const REUSED: &[&str] = &["solid.material", "pcb.lib_symbol", "pcb.lib_footprint"];

fn left_out(c: &BTreeMap<String, Value>) -> bool {
    c.keys().any(|k| LEFT_OUT.contains(&k.as_str()))
}

fn is_joint(c: &BTreeMap<String, Value>) -> bool {
    c.contains_key("assembly.joint")
}

/// An instance at the top of the project (placed in no assembly).
fn top_instance(c: &BTreeMap<String, Value>) -> bool {
    c.get("assembly.instance").is_some_and(|i| i.get("parent").is_none_or(Value::is_null))
}

/// A part placed in the project (a box, or a shape with a position).
fn placed_part(c: &BTreeMap<String, Value>) -> bool {
    c.contains_key("solid.box") || (c.contains_key("shape.program") && c.contains_key("solid.placement"))
}

fn standalone(c: &BTreeMap<String, Value>) -> bool {
    top_instance(c) || c.keys().any(|k| DEFINITIONS.contains(&k.as_str())) || (c.contains_key("shape.parameter_set") && !is_joint(c))
}

/// Who an entity is, for people.
fn name_of(c: &BTreeMap<String, Value>) -> String {
    for k in ["assembly.instance", "pcb.board", "pcb.schematic", "assembly.definition", "shape.program", "shape.parameter_set", "shape.sheet", "shape.sketch", "intent.requirement", "solid.material", "draft.object"] {
        if let Some(Value::String(n)) = c.get(k).and_then(|v| v.get("name")) {
            return n.clone();
        }
    }
    if c.contains_key("solid.box") {
        return "Box".into();
    }
    "Item".into()
}

/// The ids every entity names (that exist and are brought at all).
fn reference_map(state: &Entities) -> HashMap<StableId, Vec<StableId>> {
    let mut out = HashMap::with_capacity(state.len());
    for (id, comps) in state {
        let mut refs = Vec::new();
        for (k, v) in comps {
            if !STRIPPED.contains(&k.as_str()) {
                references(v, &mut refs);
            }
        }
        refs.sort();
        refs.dedup();
        refs.retain(|r| r != id && state.get(r).is_some_and(|c| !left_out(c)));
        out.insert(*id, refs);
    }
    out
}

/// What picking `roots` brings: everything they name (and what that names),
/// and everything that belongs to what is brought: entities that are not
/// things in their own right and name only what is brought (an assembly's
/// joints, a board's tracks, a requirement on the picked parts). Library
/// parts a board names by name come too.
pub fn closure(state: &Entities, roots: &[StableId]) -> BTreeSet<StableId> {
    let refs = reference_map(state);
    let mut set: BTreeSet<StableId> = BTreeSet::new();
    let mut queue: Vec<StableId> = roots.iter().copied().filter(|r| state.contains_key(r)).collect();
    let forward = |set: &mut BTreeSet<StableId>, queue: &mut Vec<StableId>| {
        while let Some(id) = queue.pop() {
            if set.insert(id) {
                queue.extend(refs.get(&id).into_iter().flatten().filter(|r| !set.contains(r)));
            }
        }
    };
    forward(&mut set, &mut queue);
    loop {
        let owned: Vec<StableId> = state
            .iter()
            .filter(|(id, c)| !set.contains(id) && !left_out(c) && !standalone(c))
            .filter(|(id, _)| refs.get(id).is_some_and(|r| !r.is_empty() && r.iter().all(|x| set.contains(x))))
            .map(|(id, _)| *id)
            .collect();
        // Library symbols and footprints a board or schematic names by name.
        let mut libs = Vec::new();
        if set.iter().any(|id| state[id].keys().any(|k| k.starts_with("pcb.") && !k.starts_with("pcb.lib_"))) {
            let mut words: HashSet<String> = HashSet::new();
            for id in &set {
                for v in state[id].values() {
                    strings(v, &mut words);
                }
            }
            for (id, c) in state {
                if set.contains(id) {
                    continue;
                }
                let lib = c.get("pcb.lib_symbol").map(|v| (v, "symbol")).or_else(|| c.get("pcb.lib_footprint").map(|v| (v, "footprint")));
                if let Some((v, what)) = lib
                    && let Some(name) = v[what]["name"].as_str()
                {
                    let library = v["library"].as_str().unwrap_or("");
                    if words.contains(name) || words.contains(&format!("{library}:{name}")) {
                        libs.push(*id);
                    }
                }
            }
        }
        if owned.is_empty() && libs.is_empty() {
            break;
        }
        queue.extend(owned);
        queue.extend(libs);
        forward(&mut set, &mut queue);
    }
    set
}

fn strings(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::String(s) if s.len() <= 200 => {
            out.insert(s.clone());
        }
        Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        Value::Object(m) => m.values().for_each(|x| strings(x, out)),
        _ => {}
    }
}

/// The board whose 3D model a placed shape is.
fn board_of_model(state: &Entities, id: StableId) -> Option<StableId> {
    let key = id.to_string();
    state.iter().find(|(_, c)| c.get("pcb.board").and_then(|b| b.get("model")).and_then(Value::as_str) == Some(key.as_str())).map(|(b, _)| *b)
}

fn instance_of(c: &BTreeMap<String, Value>) -> Option<de_assembly::Instance> {
    c.get("assembly.instance").and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Where a nested instance sits in the whole project (through the first
/// instance of each assembly above it).
fn world_of(state: &Entities, id: StableId) -> Transform {
    let Some(inst) = state.get(&id).and_then(instance_of) else { return Transform::IDENTITY };
    let mut t = Transform::from_euler(inst.at, inst.rotation);
    let mut parent = inst.parent;
    for _ in 0..32 {
        let Some(p) = parent else { break };
        let key = p.to_string();
        let owner = state.values().filter_map(instance_of).find(|i| i.of.to_string() == key);
        let Some(owner) = owner else { break };
        t = Transform::from_euler(owner.at, owner.rotation).then(&t);
        parent = owner.parent;
    }
    t
}

/// Position (mm) of something placed at the top of the project.
fn position(c: &BTreeMap<String, Value>) -> Option<[f64; 3]> {
    if top_instance(c) {
        return instance_of(c).map(|i| i.at);
    }
    let p: de_solid::Placement = serde_json::from_value(c.get("solid.placement")?.clone()).ok()?;
    Some([p.x.canonical() * 1000.0, p.y.canonical() * 1000.0, p.z.canonical() * 1000.0])
}

/// Moves something placed at the top of the project by `d` (mm).
fn shift(c: &mut BTreeMap<String, Value>, d: [f64; 3]) {
    if top_instance(c) {
        if let Some(at) = c.get_mut("assembly.instance").and_then(|i| i.get_mut("at")).and_then(Value::as_array_mut) {
            for (k, v) in at.iter_mut().enumerate().take(3) {
                *v = json!(round(v.as_f64().unwrap_or(0.0) + d[k]));
            }
        }
    } else if let Some(p) = c.get_mut("solid.placement") {
        for (k, axis) in ["x", "y", "z"].iter().enumerate() {
            let mm = p[axis]["value"].as_f64().unwrap_or(0.0) * unit_mm(p[axis]["unit"].as_str().unwrap_or("m"));
            p[axis] = json!({ "value": round(mm + d[k]), "unit": "mm" });
        }
    }
}

fn unit_mm(u: &str) -> f64 {
    match u {
        "mm" => 1.0,
        "cm" => 10.0,
        "in" => 25.4,
        "ft" => 304.8,
        _ => 1000.0,
    }
}

fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// What the commands are told about the source's entities (see `de_core::links`).
fn policy(entities: &Entities) -> Value {
    let mut keep = serde_json::Map::new();
    let mut rename = serde_json::Map::new();
    for (id, c) in entities {
        if top_instance(c) {
            keep.insert(id.to_string(), json!({ "assembly.instance": ["at", "rotation", "name", "properties", "parent"] }));
        } else if c.contains_key("solid.placement") {
            keep.insert(id.to_string(), json!({ "solid.placement": ["x", "y", "z"] }));
        }
        // Stock items keep their catalogue names (the stock library matches them).
        let kinds: Vec<&str> = NAMED.iter().copied().filter(|k| c.contains_key(*k)).filter(|k| !(*k == "shape.program" && c.contains_key("shape.stock_family")) && !(*k == "shape.parameter_set" && is_joint(c))).collect();
        if !kinds.is_empty() {
            rename.insert(id.to_string(), json!(kinds));
        }
    }
    json!({ "keep": keep, "rename": rename, "reuse": REUSED })
}

/// The entities to send for these ids, without what is never brought.
fn snapshot(state: &Entities, ids: &BTreeSet<StableId>) -> Entities {
    ids.iter()
        .filter_map(|id| {
            let c = state.get(id)?;
            if left_out(c) {
                return None;
            }
            Some((*id, c.iter().filter(|(k, _)| !STRIPPED.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()))
        })
        .collect()
}

fn whole_roots(state: &Entities) -> Vec<StableId> {
    state.iter().filter(|(_, c)| !left_out(c)).map(|(id, _)| *id).collect()
}

/// What a link covers in its source now.
fn link_entities(src: &Source, link: &Link) -> Entities {
    let roots = if link.whole { whole_roots(&src.state) } else { link.roots.clone() };
    snapshot(&src.state, &closure(&src.state, &roots))
}

// ---- Reading a source -----------------------------------------------------------------

fn signature(server: &Server, name: &str) -> Option<(u64, u128)> {
    let m = std::fs::metadata(server.dir.join(format!("{name}.jsonl"))).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some((m.len(), t))
}

fn export(p: &Project) -> Entities {
    p.export_state().into_iter().map(|(id, m)| (id, m.into_iter().map(|(k, v)| (k.to_string(), v)).collect())).collect()
}

fn title_of(server: &Server, name: &str) -> String {
    info_of(&server.dir, name)["title"].as_str().filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| pretty_name(name))
}

/// A project of the folder as it is now: the open copy when it is open,
/// else read from its file (its design only: nothing is evaluated), kept
/// until the file changes.
pub fn read_source(server: &Server, name: &str) -> Result<Arc<Source>, String> {
    if !valid_name(name) {
        return Err(format!("“{name}” is not a project name"));
    }
    let path = server.dir.join(format!("{name}.jsonl"));
    if !path.exists() {
        return Err(format!("there is no project called {name}"));
    }
    let title = title_of(server, name);
    let open = server.open.lock().unwrap().get(name).cloned();
    if let Some(app) = open {
        let p = app.project.lock().unwrap();
        return Ok(Arc::new(Source { name: name.into(), title, revision: p.revision().0, state: export(&p) }));
    }
    type Cache = Mutex<HashMap<String, ((u64, u128), Arc<Source>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let sig = signature(server, name).unwrap_or_default();
    if let Some((s, src)) = cache.lock().unwrap().get(name)
        && *s == sig
        && src.title == title
    {
        return Ok(src.clone());
    }
    let mut p = Project::new(name, server.registry.clone());
    p.set_origin(crate::history::origin_for(&server.dir, name, &server.engine));
    // Replayed for its design only: derived results are never computed.
    p.begin_replay();
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    for (n, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let wire: de_core::WireChangeSet = serde_json::from_str(&line).map_err(|e| format!("{name}:{}: {e}", n + 1))?;
        p.replay(wire).map_err(|e| format!("{name}:{}: {e}", n + 1))?;
    }
    let src = Arc::new(Source { name: name.into(), title, revision: p.revision().0, state: export(&p) });
    cache.lock().unwrap().insert(name.into(), (sig, src.clone()));
    Ok(src)
}

// ---- The tree ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct Item {
    /// What to pick: the item's id in the source.
    pub id: StableId,
    pub name: String,
    /// "assembly", "component", "part", "board", "schematic", "parameters", "sheet", "sketch", "definition".
    pub kind: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Entities picking it brings (top-level items only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brings: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Item>,
}

fn is_assembly(state: &Entities, def: StableId) -> bool {
    state.get(&def).is_some_and(|c| c.contains_key("assembly.definition"))
}

fn nested(state: &Entities, def: StableId, depth: usize) -> Vec<Item> {
    if depth > 32 {
        return Vec::new();
    }
    let key = def.to_string();
    let mut kids: Vec<(StableId, de_assembly::Instance)> = state.iter().filter_map(|(id, c)| instance_of(c).map(|i| (*id, i))).filter(|(_, i)| i.parent.is_some_and(|p| p.to_string() == key)).collect();
    kids.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    kids.into_iter()
        .map(|(id, i)| {
            let asm = is_assembly(state, i.of);
            let def_name = state.get(&i.of).map(name_of).unwrap_or_default();
            Item {
                id,
                name: i.name.clone(),
                kind: if asm { "assembly" } else { "component" },
                detail: if def_name != i.name { def_name } else { String::new() },
                brings: None,
                children: if asm { nested(state, i.of, depth + 1) } else { Vec::new() },
            }
        })
        .collect()
}

/// The source's items: what is placed (assemblies with what is inside),
/// boards, schematics, parameter sets, sheets, sketches, definitions placed nowhere.
pub fn tree(state: &Entities) -> Vec<Item> {
    let mut used: HashSet<String> = HashSet::new();
    for c in state.values() {
        if let Some(i) = instance_of(c) {
            used.insert(i.of.to_string());
        }
    }
    let models: HashMap<String, StableId> = state.iter().filter_map(|(b, c)| Some((c.get("pcb.board")?.get("model")?.as_str()?.to_string(), *b))).collect();
    let count = |roots: &[StableId]| Some(closure(state, roots).len());
    let mut items = Vec::new();
    let mut boards_done = HashSet::new();
    for (id, c) in state {
        if left_out(c) {
            continue;
        }
        let item = if top_instance(c) {
            let i = instance_of(c).unwrap();
            let asm = is_assembly(state, i.of);
            let children = if asm { nested(state, i.of, 0) } else { Vec::new() };
            let def_name = state.get(&i.of).map(name_of).unwrap_or_default();
            let detail = if asm { format!("Assembly · {} inside", children.len()) } else if def_name != i.name { format!("Component · {def_name}") } else { "Component".into() };
            Item { id: *id, name: i.name, kind: if asm { "assembly" } else { "component" }, detail, brings: count(&[*id]), children }
        } else if let Some(board) = c.get("pcb.board") {
            boards_done.insert(*id);
            let parts = state.values().filter(|x| x.get("pcb.component").and_then(|p| p.get("board")).and_then(Value::as_str) == Some(id.to_string().as_str())).count();
            Item { id: *id, name: board["name"].as_str().unwrap_or("Board").to_string(), kind: "board", detail: format!("Board · {parts} components, with its 3D model"), brings: count(&[*id]), children: Vec::new() }
        } else if placed_part(c) {
            if models.contains_key(&id.to_string()) {
                continue; // shown as its board
            }
            Item { id: *id, name: name_of(c), kind: "part", detail: if c.contains_key("solid.box") { "Box part".into() } else { "Part".into() }, brings: count(&[*id]), children: Vec::new() }
        } else if c.contains_key("pcb.schematic") {
            Item { id: *id, name: name_of(c), kind: "schematic", detail: "Schematic".into(), brings: count(&[*id]), children: Vec::new() }
        } else if c.contains_key("shape.parameter_set") && !is_joint(c) {
            let n = c["shape.parameter_set"]["params"].as_array().map(Vec::len).unwrap_or(0);
            Item { id: *id, name: name_of(c), kind: "parameters", detail: format!("Parameters · {n}"), brings: count(&[*id]), children: Vec::new() }
        } else if c.contains_key("shape.sheet") {
            Item { id: *id, name: name_of(c), kind: "sheet", detail: "Sheet".into(), brings: count(&[*id]), children: Vec::new() }
        } else if c.contains_key("shape.sketch") {
            Item { id: *id, name: name_of(c), kind: "sketch", detail: "Sketch".into(), brings: count(&[*id]), children: Vec::new() }
        } else if (c.contains_key("assembly.definition") || (c.contains_key("shape.program") && !c.contains_key("solid.placement"))) && !used.contains(&id.to_string()) && !models.contains_key(&id.to_string()) {
            let asm = c.contains_key("assembly.definition");
            Item { id: *id, name: name_of(c), kind: "definition", detail: if asm { "Assembly, placed nowhere".into() } else { "Component, placed nowhere".into() }, brings: count(&[*id]), children: if asm { nested(state, *id, 0) } else { Vec::new() } }
        } else {
            continue;
        };
        items.push(item);
    }
    let order = |k: &str| ["assembly", "component", "part", "board", "schematic", "definition", "parameters", "sheet", "sketch"].iter().position(|x| *x == k).unwrap_or(99);
    items.sort_by(|a, b| order(a.kind).cmp(&order(b.kind)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    items
}

// ---- Picking ---------------------------------------------------------------------------

/// What an insert brings: the source ids picked, and placements to make here
/// for things picked from inside an assembly (or placed nowhere).
#[derive(Default, Debug)]
pub struct Pick {
    pub roots: Vec<StableId>,
    pub extra: Vec<BTreeMap<String, Value>>,
    /// Where the first picked thing sits in the source (mm), if it is placed.
    pub anchor: Option<[f64; 3]>,
}

/// Finds an item by id or by name (instances, parts, boards, definitions…).
fn find(state: &Entities, what: &str) -> Result<StableId, String> {
    if let Ok(id) = serde_json::from_value::<StableId>(json!(what.trim())) {
        return if state.get(&id).is_some_and(|c| !left_out(c)) { Ok(id) } else { Err(format!("{what} is not in that project")) };
    }
    let want = what.trim().to_lowercase();
    let named: Vec<(StableId, bool)> = state
        .iter()
        .filter(|(_, c)| !left_out(c) && (standalone(c) || c.contains_key("assembly.instance") || c.contains_key("pcb.schematic")))
        .filter(|(_, c)| name_of(c).to_lowercase() == want)
        .map(|(id, c)| (*id, top_instance(c) || placed_part(c) || c.contains_key("pcb.board")))
        .collect();
    // An instance and the definition it shows, both called "Forearm": the instance (it says where).
    let shown: HashSet<StableId> = named.iter().filter_map(|(id, _)| instance_of(&state[id]).map(|i| i.of)).collect();
    let named: Vec<(StableId, bool)> = named.into_iter().filter(|(id, _)| !shown.contains(id)).collect();
    // Placed things first: "Robot arm" is the arm as placed, not its definition.
    let top: Vec<StableId> = named.iter().filter(|(_, t)| *t).map(|(id, _)| *id).collect();
    match (top.as_slice(), named.as_slice()) {
        ([one], _) => Ok(*one),
        ([], [(one, _)]) => Ok(*one),
        ([], []) => Err(format!("nothing called “{what}” in that project (see `./ise insert --from <project> --list`)")),
        _ => Err(format!(
            "“{what}” names {} things there: {}; pick one by id",
            named.len(),
            named.iter().map(|(id, _)| format!("{id} ({})", kind_word(&state[id]))).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn kind_word(c: &BTreeMap<String, Value>) -> &'static str {
    if top_instance(c) {
        "placed"
    } else if c.contains_key("assembly.instance") {
        "inside an assembly"
    } else if c.contains_key("pcb.board") {
        "board"
    } else if c.contains_key("assembly.definition") {
        "assembly definition"
    } else if c.contains_key("shape.program") {
        "shape"
    } else {
        "item"
    }
}

/// A placement here for a definition picked on its own.
fn placement_of(def: StableId, name: &str, t: Transform, config: Option<String>) -> BTreeMap<String, Value> {
    let rot = t.euler();
    let mut inst = json!({ "name": name, "of": def, "parent": null, "at": t.at.map(round), "rotation": rot });
    if let Some(c) = config {
        inst["config"] = json!(c);
    }
    BTreeMap::from([("assembly.instance".to_string(), inst), ("core.depends_on".to_string(), json!([def]))])
}

pub fn pick(state: &Entities, items: &[String]) -> Result<Pick, String> {
    let mut p = Pick::default();
    for what in items {
        let id = find(state, what)?;
        let c = &state[&id];
        let (root, extra) = if let Some(board) = board_of_model(state, id) {
            (board, None)
        } else if c.contains_key("assembly.instance") && !top_instance(c) {
            // From inside an assembly: its definition, placed where it sits in the whole.
            let i = instance_of(c).unwrap();
            (i.of, Some(placement_of(i.of, &i.name, world_of(state, id), i.config)))
        } else if c.contains_key("assembly.definition") || (c.contains_key("shape.program") && !c.contains_key("solid.placement")) {
            (id, Some(placement_of(id, &name_of(c), Transform::IDENTITY, None)))
        } else {
            (id, None)
        };
        if p.anchor.is_none() {
            p.anchor = match &extra {
                Some(e) => position(e),
                None => {
                    let rc = &state[&root];
                    position(rc).or_else(|| rc.get("pcb.board").and_then(|b| b["model"].as_str()).and_then(|m| serde_json::from_value::<StableId>(json!(m)).ok()).and_then(|m| state.get(&m)).and_then(position))
                }
            };
        }
        if !p.roots.contains(&root) {
            p.roots.push(root);
        }
        if let Some(e) = extra {
            p.extra.push(e);
        }
    }
    if p.roots.is_empty() {
        return Err("pick something to insert (ids or names), or the whole project".into());
    }
    Ok(p)
}

// ---- Committing --------------------------------------------------------------------------

fn actor(user: Option<&String>) -> Actor {
    Actor { user_id: user.filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| "local".into()), session_id: StableId::new().to_string() }
}

/// Runs one command on a project as `actor` (their undo takes it back), and
/// tells every view and teammate, like a command from the browser.
fn commit_as(app: &App, actor: &Actor, operation: &str, parameters: Value) -> Result<Value, String> {
    if app.closed.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("this project was renamed or deleted".into());
    }
    let mut project = app.project.lock().unwrap();
    let (logged, before) = (project.log().len(), project.revision());
    let e = crate::envelope(&project, actor, None, operation.into(), parameters, project.revision());
    let out = project.commit(e).map_err(|e| e.message.clone())?;
    app.publish(&project, logged, before).map_err(|e| e.to_string())?;
    Ok(out.output)
}

/// What a command would do, without doing it.
fn preview(app: &App, operation: &str, parameters: Value) -> Result<Value, String> {
    let mut project = app.project.lock().unwrap();
    let e = crate::envelope(&project, &actor(None), None, operation.into(), parameters, project.revision());
    project.preview(&e).map(|p| p.output).map_err(|e| e.message)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The parameters of `link.insert` for these picks.
pub fn insert_parameters(src: &Source, items: &[String], whole: bool, linked: bool, auto_update: bool, place: Place, engine: &str) -> Result<Value, String> {
    let mut p = if whole { Pick { roots: whole_roots(&src.state), ..Default::default() } } else { pick(&src.state, items)? };
    let ids = closure(&src.state, &p.roots);
    let mut entities = snapshot(&src.state, &ids);
    if entities.is_empty() {
        return Err(format!("{} has nothing to insert", src.title));
    }
    let anchor = if whole { Some([0.0; 3]) } else { p.anchor.or_else(|| entities.values().find_map(position)) };
    let delta = match (place, anchor) {
        (Place::Origin, Some(a)) => Some([-a[0], -a[1], -a[2]]),
        (Place::At(to), Some(a)) => Some([to[0] - a[0], to[1] - a[1], to[2] - a[2]]),
        (Place::At(to), None) => Some(to),
        _ => None,
    };
    if let Some(d) = delta.filter(|d| d.iter().any(|v| v.abs() > 1e-9)) {
        for c in entities.values_mut().filter(|c| top_instance(c) || c.contains_key("solid.placement")) {
            shift(c, d);
        }
        for c in &mut p.extra {
            shift(c, d);
        }
    }
    let mut params = json!({
        "project": src.name, "title": src.title, "revision": src.revision,
        "roots": if whole { json!([]) } else { json!(p.roots) }, "whole": whole,
        "linked": linked, "auto_update": auto_update && linked,
        "extra": p.extra, "engine": engine, "time": now_ms(),
    });
    let pol = policy(&entities);
    for (k, v) in pol.as_object().unwrap() {
        params[k] = v.clone();
    }
    params["entities"] = serde_json::to_value(&entities).unwrap();
    Ok(params)
}

/// The parameters of `link.update` for one link.
pub fn update_parameters(src: &Source, link_id: StableId, link: &Link) -> Value {
    let entities = link_entities(src, link);
    let mut params = json!({ "link": link_id, "title": src.title, "revision": src.revision, "time": now_ms() });
    for (k, v) in policy(&entities).as_object().unwrap() {
        params[k] = v.clone();
    }
    params["entities"] = serde_json::to_value(&entities).unwrap();
    params
}

#[derive(Clone, Copy, Debug)]
pub enum Place {
    /// Where it is in the source.
    Same,
    /// The first picked thing's origin at the project's origin.
    Origin,
    /// The first picked thing's origin at this point (mm).
    At([f64; 3]),
}

/// Pictures and fonts the inserted items read: listed in this project's index too.
fn copy_picture_index(server: &Server, from: &str, to: &str, params: &Value) {
    let path = |n: &str| server.dir.join(format!("{n}.images.json"));
    let Ok(text) = std::fs::read_to_string(path(from)) else { return };
    let Ok(theirs) = serde_json::from_str::<Vec<crate::pictures::Entry>>(&text) else { return };
    let body = params["entities"].to_string();
    let mut mine: Vec<crate::pictures::Entry> = std::fs::read_to_string(path(to)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let before = mine.len();
    for e in theirs {
        if body.contains(&e.id) && !mine.iter().any(|m| m.id == e.id) {
            mine.push(e);
        }
    }
    if mine.len() > before {
        let _ = std::fs::write(path(to), serde_json::to_string_pretty(&mine).unwrap());
    }
}

// ---- Links -------------------------------------------------------------------------------

fn links_in(project: &Project) -> Vec<(StableId, Link)> {
    let mut out: Vec<(StableId, Link)> = project.entity_ids().into_iter().filter_map(|id| project.get::<Link>(id).map(|l| (id, l.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// One link's state: where from, whether the source has changed since, and
/// what an update would do.
fn status(server: &Server, app: &App, id: StableId, link: &Link) -> Value {
    let names: Vec<String> = {
        let p = app.project.lock().unwrap();
        let roots: Vec<StableId> = if link.whole { Vec::new() } else { link.roots.iter().filter_map(|r| link.ids.get(r)).copied().collect() };
        roots.iter().filter_map(|r| p.entity_state(*r)).map(|c| name_of(&c.into_iter().map(|(k, v)| (k.to_string(), v)).collect())).collect()
    };
    let items = link.ids.len();
    let mut out = json!({
        "id": id, "project": link.project, "title": link.title, "whole": link.whole, "names": names, "items": items,
        "revision": link.revision, "auto_update": link.auto_update, "local_edits": link.local_edits, "updates_here": link.engine.is_empty() || link.engine == server.engine,
        "inserted": link.inserted, "synced": link.synced, "placed": link.placed,
    });
    match read_source(server, &link.project) {
        Err(e) => {
            out["state"] = json!("missing");
            out["message"] = json!(e);
        }
        Ok(src) => {
            out["source_revision"] = json!(src.revision);
            out["title"] = json!(src.title);
            match preview(app, "link.update", update_parameters(&src, id, link)) {
                Ok(p) => {
                    let n = |k: &str| p[k].as_array().map(Vec::len).unwrap_or(0);
                    out["state"] = json!(if p["up_to_date"] == true { "up_to_date" } else { "update_available" });
                    out["changes"] = json!({ "created": n("created"), "changed": n("changed"), "removed": n("removed"), "conflicts": n("conflicts"), "deleted_here": p["deleted_here"] });
                }
                Err(e) => {
                    out["state"] = json!("error");
                    out["message"] = json!(e);
                }
            }
        }
    }
    out
}

fn update_link(server: &Server, app: &App, actor: &Actor, id: StableId, link: &Link) -> Result<Value, String> {
    let src = read_source(server, &link.project)?;
    let params = update_parameters(&src, id, link);
    let out = commit_as(app, actor, "link.update", params.clone())?;
    if out["up_to_date"] != true {
        copy_picture_index(server, &link.project, app.project.lock().unwrap().id(), &params);
    }
    Ok(out)
}

/// Watches the sources of every open project's links: each change tells the
/// project's views (their "update available" shows), and links set to update
/// by themselves take it at once.
fn watch(server: &Arc<Server>) {
    watch_from(server, false);
}

/// When the engine starts: opens every project of the folder that has links
/// and checks them against their sources right away (links set to update by
/// themselves take what changed while the engine was off; the others are
/// reported as having an update), then keeps watching.
pub fn start(server: &Arc<Server>) {
    watch_from(server, true);
}

fn watch_from(server: &Arc<Server>, open_all: bool) {
    // One watcher per engine (tests run several in one process).
    static STARTED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    if !STARTED.get_or_init(Default::default).lock().unwrap().insert(Arc::as_ptr(server) as usize) {
        return;
    }
    let server = server.clone();
    tokio::spawn(async move {
        let mut seen: HashMap<(String, StableId), Option<(u64, u128)>> = HashMap::new();
        if open_all {
            let s = server.clone();
            seen = tokio::task::spawn_blocking(move || {
                let opened = open_linking(&s);
                if !opened.is_empty() {
                    println!("checking the links of {} against their sources", opened.join(", "));
                }
                tick(&s, &mut seen);
                seen
            })
            .await
            .unwrap_or_default();
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            let s = server.clone();
            seen = tokio::task::spawn_blocking(move || {
                tick(&s, &mut seen);
                seen
            })
            .await
            .unwrap_or_default();
        }
    });
}

/// Opens every project of the folder whose log holds a link (read as text
/// first: projects without links stay closed). Returns their names.
fn open_linking(server: &Server) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&server.dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|f| f.strip_suffix(".jsonl")).map(str::to_owned))
        .filter(|n| valid_name(n))
        .collect();
    names.sort();
    let mut opened = Vec::new();
    for name in names {
        let has = std::fs::File::open(server.dir.join(format!("{name}.jsonl")))
            .map(|f| std::io::BufReader::new(f).lines().map_while(Result::ok).any(|l| l.contains("\"kind\":\"core.link\"")))
            .unwrap_or(false);
        if !has {
            continue;
        }
        match server.project(Some(&name)) {
            Ok(app) if !links_in(&app.project.lock().unwrap()).is_empty() => opened.push(name),
            Ok(_) => {}
            Err(e) => eprintln!("{name}: could not open it to check its links: {e}"),
        }
    }
    opened
}

fn tick(server: &Server, seen: &mut HashMap<(String, StableId), Option<(u64, u128)>>) {
    let apps: Vec<(String, Arc<App>)> = server.open.lock().unwrap().iter().map(|(n, a)| (n.clone(), a.clone())).collect();
    for (name, app) in apps {
        let links = links_in(&app.project.lock().unwrap());
        let mut told = false;
        for (id, link) in links {
            let sig = signature(server, &link.project);
            let before = seen.insert((name.clone(), id), sig);
            let changed = before.is_some_and(|b| b != sig);
            if changed && !told {
                let _ = app.events.send(json!({ "type": "links" }).to_string().into());
                told = true;
            }
            // A link that does not update by itself, first seen (the engine just
            // started, or the project just opened): say whether its source changed.
            let automatic = link.auto_update && (link.engine.is_empty() || link.engine == server.engine);
            if before.is_none() && !automatic {
                let st = status(server, &app, id, &link);
                if st["state"] == "update_available" {
                    println!("{name}: the link to {} has an update available (ise links update)", link.project);
                    if !told {
                        let _ = app.events.send(json!({ "type": "links" }).to_string().into());
                        told = true;
                    }
                } else if st["state"] == "missing" {
                    eprintln!("{name}: the link to {}: {}", link.project, st["message"].as_str().unwrap_or("its source is missing"));
                }
            }
            // Automatic links catch up when first seen (the engine was off) and on every change.
            if automatic && (changed || before.is_none()) {
                let who = Actor { user_id: "ise-links".into(), session_id: StableId::new().to_string() };
                match update_link(server, &app, &who, id, &link) {
                    Ok(out) if out["up_to_date"] != true => {
                        println!("{name}: link to {} updated automatically", link.project);
                        let _ = app.events.send(json!({ "type": "links", "updated": id, "from": link.project, "output": out }).to_string().into());
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("{name}: updating the link to {} failed: {e}", link.project),
                }
            }
        }
    }
}

// ---- Routes -------------------------------------------------------------------------------

type Q = Query<HashMap<String, String>>;

fn fail(code: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (code, message.into()).into_response()
}

fn target(server: &Server, query: &HashMap<String, String>) -> Result<(String, Arc<App>), axum::response::Response> {
    let name = query.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    server.project(Some(&name)).map(|a| (name, a)).map_err(bad_request)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, axum::response::Response> + Send + 'static) -> axum::response::Response
where
    T: serde::Serialize,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => axum::Json(v).into_response(),
        Ok(Err(r)) => r,
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `GET /link/tree?from=<source>`: the source's items, and what picking each brings.
pub async fn source_tree(State(server): State<Arc<Server>>, Query(query): Q) -> axum::response::Response {
    watch(&server);
    let Some(from) = query.get("from").cloned() else { return bad_request("say which project to insert from (&from=<name>)") };
    let here = query.get("project").cloned().unwrap_or_else(|| server.default.clone());
    blocking(move || {
        if from == here {
            return Err(bad_request("that is this project: insert from another one"));
        }
        let src = read_source(&server, &from).map_err(|e| fail(StatusCode::NOT_FOUND, e))?;
        let total = src.state.values().filter(|c| !left_out(c)).count();
        Ok(json!({ "project": src.name, "title": src.title, "revision": src.revision, "entities": total, "items": tree(&src.state) }))
    })
    .await
}

#[derive(Deserialize)]
pub struct InsertBody {
    from: String,
    /// Ids or names of what to bring (see the tree).
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    whole: bool,
    /// An independent copy instead of a link.
    #[serde(default)]
    copy: bool,
    #[serde(default)]
    auto_update: bool,
    /// "same" (where it is in the source, the default) or "origin".
    #[serde(default)]
    place: Option<String>,
    /// Put the first picked item's origin here (mm).
    #[serde(default)]
    at: Option<[f64; 3]>,
}

fn place_of(body: &InsertBody) -> Result<Place, String> {
    Ok(match (body.at, body.place.as_deref()) {
        (Some(at), _) if at.iter().all(|v| v.is_finite()) => Place::At(at),
        (Some(_), _) => return Err("the point must be three numbers (mm)".into()),
        (None, None | Some("same")) => Place::Same,
        (None, Some("origin")) => Place::Origin,
        (None, Some(other)) => return Err(format!("place is “same” or “origin” (or give at: [x, y, z]), not “{other}”")),
    })
}

/// `POST /link/preview`: what an insert would bring, without inserting.
pub async fn insert_preview(State(server): State<Arc<Server>>, Query(query): Q, axum::Json(body): axum::Json<InsertBody>) -> axum::response::Response {
    blocking(move || {
        let (_, app) = target(&server, &query)?;
        let src = read_source(&server, &body.from).map_err(|e| fail(StatusCode::NOT_FOUND, e))?;
        let params = insert_parameters(&src, &body.items, body.whole, !body.copy, false, Place::Same, &server.engine).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?;
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for c in params["entities"].as_object().unwrap().values() {
            let c: BTreeMap<String, Value> = serde_json::from_value(c.clone()).unwrap_or_default();
            let k = if top_instance(&c) || c.contains_key("assembly.instance") {
                "placements"
            } else if c.contains_key("assembly.definition") {
                "assemblies"
            } else if c.contains_key("pcb.board") {
                "boards"
            } else if c.keys().any(|k| k.starts_with("pcb.")) {
                "board items"
            } else if is_joint(&c) {
                "joints"
            } else if c.contains_key("shape.program") || c.contains_key("solid.box") {
                "parts and components"
            } else if c.contains_key("solid.material") {
                "materials"
            } else if c.contains_key("intent.requirement") {
                "requirements"
            } else if c.contains_key("shape.parameter_set") {
                "parameter sets"
            } else {
                "other"
            };
            *kinds.entry(k).or_default() += 1;
        }
        let out = preview(&app, "link.insert", params).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?;
        Ok(json!({ "entities": out["created"], "reused": out["reused"].as_array().map(Vec::len), "renamed": out["renamed"], "kinds": kinds, "placed": out["placed"].as_array().map(Vec::len) }))
    })
    .await
}

/// `POST /link/insert`: brings the picked items in (one undoable change).
pub async fn insert(State(server): State<Arc<Server>>, Query(query): Q, axum::Json(body): axum::Json<InsertBody>) -> axum::response::Response {
    watch(&server);
    blocking(move || {
        let (name, app) = target(&server, &query)?;
        if body.from == name {
            return Err(bad_request("that is this project: insert from another one"));
        }
        let place = place_of(&body).map_err(bad_request)?;
        let src = read_source(&server, &body.from).map_err(|e| fail(StatusCode::NOT_FOUND, e))?;
        let params = insert_parameters(&src, &body.items, body.whole, !body.copy, body.auto_update, place, &server.engine).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?;
        let out = commit_as(&app, &actor(query.get("user")), "link.insert", params.clone()).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?;
        copy_picture_index(&server, &body.from, &name, &params);
        // What to show: what was placed (a board by its 3D model).
        let here = |s: &str| out["ids"].get(s).cloned();
        let mut show: Vec<Value> = out["placed"].as_array().cloned().unwrap_or_default();
        let roots: Vec<StableId> = serde_json::from_value(params["roots"].clone()).unwrap_or_default();
        for r in &roots {
            let c = &src.state[r];
            let shown = match c.get("pcb.board").and_then(|b| b["model"].as_str()) {
                Some(model) => here(model),
                None if top_instance(c) || placed_part(c) => here(&r.to_string()),
                None => None,
            };
            show.extend(shown);
        }
        if body.whole {
            show.extend(src.state.iter().filter(|(_, c)| top_instance(c) || placed_part(c)).filter_map(|(id, _)| here(&id.to_string())));
        }
        Ok(json!({ "from": src.name, "title": src.title, "revision": src.revision, "output": out, "show": show }))
    })
    .await
}

/// `GET /links`: this project's links and their state.
pub async fn list(State(server): State<Arc<Server>>, Query(query): Q) -> axum::response::Response {
    watch(&server);
    blocking(move || {
        let (_, app) = target(&server, &query)?;
        let links = links_in(&app.project.lock().unwrap());
        Ok(links.iter().map(|(id, l)| status(&server, &app, *id, l)).collect::<Vec<_>>())
    })
    .await
}

#[derive(Deserialize)]
pub struct LinkBody {
    /// The link, or any item it brought (every link when left out, for update).
    #[serde(default)]
    link: Option<StableId>,
    #[serde(default)]
    auto_update: Option<bool>,
    /// Allow editing the linked items here ("Edit locally").
    #[serde(default)]
    local_edits: Option<bool>,
}

/// The link an id names: the link itself, or an item it brought.
fn link_named(app: &App, id: StableId) -> Result<(StableId, Link), String> {
    let p = app.project.lock().unwrap();
    let link = if p.get::<Link>(id).is_some() { id } else { p.get::<de_core::links::Linked>(id).map(|m| m.link).ok_or_else(|| format!("{id} is not a link, nor an item linked from another project"))? };
    p.get::<Link>(link).cloned().map(|l| (link, l)).ok_or_else(|| format!("{id} is not a link, nor an item linked from another project"))
}

/// `POST /links/update`: takes the source's changes (one link, or all).
pub async fn update(State(server): State<Arc<Server>>, Query(query): Q, axum::Json(body): axum::Json<LinkBody>) -> axum::response::Response {
    blocking(move || {
        let (_, app) = target(&server, &query)?;
        let who = actor(query.get("user"));
        let links = match body.link {
            Some(id) => vec![link_named(&app, id).map_err(bad_request)?],
            None => links_in(&app.project.lock().unwrap()),
        };
        let mut out = Vec::new();
        for (id, link) in links {
            match update_link(&server, &app, &who, id, &link) {
                Ok(o) => out.push(json!({ "link": id, "project": link.project, "output": o })),
                Err(e) => out.push(json!({ "link": id, "project": link.project, "error": e })),
            }
        }
        Ok(out)
    })
    .await
}

/// `POST /links/break`: the link's items become an ordinary local copy.
pub async fn unlink(State(server): State<Arc<Server>>, Query(query): Q, axum::Json(body): axum::Json<LinkBody>) -> axum::response::Response {
    blocking(move || {
        let (_, app) = target(&server, &query)?;
        let Some(id) = body.link else { return Err(bad_request("say which link (or an item it brought)")) };
        let (link, _) = link_named(&app, id).map_err(bad_request)?;
        commit_as(&app, &actor(query.get("user")), "link.break", json!({ "link": link })).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))
    })
    .await
}

/// `POST /links/settings`: `{link, auto_update?, local_edits?}`.
pub async fn settings(State(server): State<Arc<Server>>, Query(query): Q, axum::Json(body): axum::Json<LinkBody>) -> axum::response::Response {
    watch(&server);
    blocking(move || {
        let (_, app) = target(&server, &query)?;
        let Some(id) = body.link else { return Err(bad_request("say which link (or an item it brought)")) };
        if body.auto_update.is_none() && body.local_edits.is_none() {
            return Err(bad_request("give auto_update and/or local_edits (true or false)"));
        }
        let (link, _) = link_named(&app, id).map_err(bad_request)?;
        let mut params = json!({ "link": link, "engine": server.engine });
        if let Some(a) = body.auto_update {
            params["auto_update"] = json!(a);
        }
        if let Some(l) = body.local_edits {
            params["local_edits"] = json!(l);
        }
        commit_as(&app, &actor(query.get("user")), "link.settings", params).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{serve, server_in};
    use std::net::SocketAddr;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ise-xproj-{}", StableId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ok(app: &App, operation: &str, parameters: Value) -> Value {
        app.commit_here(operation, parameters).unwrap_or_else(|e| panic!("{operation}: {e}"))["output"].clone()
    }

    fn id(v: &Value) -> StableId {
        serde_json::from_value(v["id"].clone()).unwrap()
    }

    fn sid(v: &Value) -> StableId {
        serde_json::from_value(v.clone()).unwrap()
    }

    const LINK: &str = r#"
connector("Bottom", { at: [5, 5, 0], axis: [0, 0, -1] });
connector("Tip", { at: [95, 5, 10], axis: [0, 0, 1] });
return box(100, 10, 10);
"#;

    /// The source project: an arm (two links on a hinge, in PLA) placed at
    /// x 50, a plate of the same PLA beside it, and a small board.
    struct Arm {
        app: Arc<App>,
        top: StableId,
        asm: StableId,
        upper: StableId,
        fore: StableId,
        joint: StableId,
        link: StableId,
        pla: StableId,
        plate: StableId,
        board: StableId,
        model: StableId,
        r1: StableId,
    }

    fn arm(server: &Server) -> Arm {
        let app = server.project(Some(&"arm".to_string())).unwrap();
        ok(&app, "project.set_info", json!({ "title": "Arm project" }));
        let pla = id(&ok(&app, "solid.material.create", json!({ "name": "PLA", "density": { "value": 1240, "unit": "kg/m3" } })));
        let link = id(&ok(&app, "shape.create", json!({ "name": "Link", "source": LINK, "params": [], "definition": true, "material": pla })));
        let asm = id(&ok(&app, "assembly.create", json!({ "name": "Arm" })));
        let upper = id(&ok(&app, "assembly.instance.create", json!({ "of": link, "parent": asm, "name": "Upper arm", "at": [0, 0, 0] })));
        let fore = id(&ok(&app, "assembly.instance.create", json!({ "of": link, "parent": asm, "name": "Forearm", "at": [100, 0, 10], "rotation": [0, 0, 90] })));
        let joint = id(&ok(&app, "assembly.joint.create", json!({ "assembly": asm, "type": "hinge", "name": "Elbow", "a": { "instance": upper, "connector": "Tip" }, "b": { "instance": fore, "connector": "Bottom" }, "value": 0 })));
        let top = id(&ok(&app, "assembly.instance.create", json!({ "of": asm, "name": "Arm 1", "at": [50, 0, 0] })));
        let plate = id(&ok(&app, "shape.create", json!({ "name": "Plate", "source": "return box(40, 40, 3);", "params": [], "material": pla, "placement": { "x": { "value": 300, "unit": "mm" }, "y": { "value": 0, "unit": "mm" }, "z": { "value": 0, "unit": "mm" } } })));
        let b = ok(&app, "pcb.board.create", json!({ "name": "Driver board", "width": 30, "height": 20 }));
        let (board, model) = (id(&b), sid(&b["model"]));
        let r1 = id(&ok(&app, "pcb.component.create", json!({ "board": board, "reference": "R1", "footprint": "R_0603", "value": "10k", "at": [8, 10], "pins": { "1": "VCC", "2": "SIG" } })));
        ok(&app, "pcb.component.create", json!({ "board": board, "reference": "R2", "footprint": "R_0603", "value": "10k", "at": [14, 10], "pins": { "1": "SIG", "2": "GND" } }));
        ok(&app, "pcb.track.create", json!({ "board": board, "layer": "top", "net": "SIG", "points": [[8.8, 10], [13.2, 10]] }));
        Arm { app, top, asm, upper, fore, joint, link, pla, plate, board, model, r1 }
    }

    fn state(app: &App) -> Entities {
        export(&app.project.lock().unwrap())
    }

    #[test]
    fn picking_an_assembly_brings_what_it_needs_and_what_belongs_to_it_only() {
        let server = server_in(&temp_dir());
        let a = arm(&server);
        let s = state(&a.app);
        let got = closure(&s, &[a.top]);
        for need in [a.top, a.asm, a.upper, a.fore, a.joint, a.link, a.pla] {
            assert!(got.contains(&need), "{need} missing");
        }
        assert!(!got.contains(&a.plate), "a part that only shares the material stays");
        assert!(!got.contains(&a.board));
        // A board brings its components, tracks and 3D model, nothing of the arm.
        let got = closure(&s, &[a.board]);
        assert!(got.contains(&a.model) && got.contains(&a.r1));
        assert!(s.iter().filter(|(_, c)| c.contains_key("pcb.track")).all(|(t, _)| got.contains(t)));
        assert!(!got.contains(&a.asm) && !got.contains(&a.link));
        // The tree: the arm with what is inside, the plate, the board (not its model as a part).
        let items = tree(&s);
        let names: Vec<(&str, &str)> = items.iter().map(|i| (i.name.as_str(), i.kind)).collect();
        assert_eq!(names, vec![("Arm 1", "assembly"), ("Plate", "part"), ("Driver board", "board")]);
        let kids: Vec<&str> = items[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(kids, vec!["Forearm", "Upper arm"]);
        assert!(items[0].brings.unwrap() >= 7);
    }

    #[test]
    fn a_part_picked_from_inside_an_assembly_is_placed_where_it_was() {
        let server = server_in(&temp_dir());
        let a = arm(&server);
        let s = state(&a.app);
        let p = pick(&s, &["Forearm".to_string()]).unwrap();
        assert_eq!(p.roots, vec![a.link]);
        let inst = instance_of(&p.extra[0]).unwrap();
        assert_eq!((inst.name.as_str(), inst.of, inst.parent), ("Forearm", a.link, None));
        // Inside "Arm 1" (x 50): 100 + 50 along x, 10 up, turned 90° about Z.
        assert_eq!(inst.at, [150.0, 0.0, 10.0]);
        assert_eq!(inst.rotation, [0.0, 0.0, 90.0]);
        // The board's 3D model stands for the board.
        assert_eq!(pick(&s, &[a.model.to_string()]).unwrap().roots, vec![a.board]);
        assert!(pick(&s, &["Nothing like it".to_string()]).unwrap_err().contains("nothing called"));
        // An instance named as its definition is that instance; two things truly sharing a name are refused.
        let mut s2 = s.clone();
        s2.get_mut(&a.link).unwrap().get_mut("shape.program").unwrap()["name"] = json!("Forearm");
        assert_eq!(find(&s2, "Forearm").unwrap(), a.fore);
        s2.get_mut(&a.upper).unwrap().get_mut("assembly.instance").unwrap()["name"] = json!("Forearm");
        assert!(find(&s2, "forearm").unwrap_err().contains("names 2 things"));
    }

    #[test]
    fn inserting_twice_from_one_project_never_collides_and_names_stay_distinct() {
        let server = server_in(&temp_dir());
        let a = arm(&server);
        let src = read_source(&server, "arm").unwrap();
        let dst = server.project(Some(&"dst".to_string())).unwrap();
        let params = insert_parameters(&src, &["Arm 1".into()], false, false, false, Place::Same, "e").unwrap();
        let first = ok(&dst, "link.insert", params.clone());
        let second = ok(&dst, "link.insert", insert_parameters(&src, &["Arm 1".into()], false, true, false, Place::At([0.0, 200.0, 0.0]), "e").unwrap());
        let s = state(&dst);
        // Two arms, two definitions each, one PLA shared by all.
        assert_eq!(s.values().filter(|c| c.contains_key("assembly.definition")).count(), 2);
        assert_eq!(s.values().filter(|c| c.contains_key("solid.material")).count(), 1);
        assert_eq!(second["renamed"].as_array().unwrap().len(), 2, "{second}");
        assert!(s.values().any(|c| c.get("assembly.definition").is_some_and(|d| d["name"] == "Arm (Arm project)")));
        // The joint joins this copy's instances, and nothing names the source's ids.
        let map = &second["ids"];
        let joint = sid(&map[a.joint.to_string()]);
        let j = &s[&joint]["assembly.joint"];
        assert_eq!(j["a"]["instance"], map[a.upper.to_string()]);
        assert_eq!(j["assembly"], map[a.asm.to_string()]);
        // (The link itself remembers the source's ids; the design never names them.)
        let design: Vec<Value> = s.values().map(|c| json!(c.iter().filter(|(k, _)| !k.starts_with("core.link")).collect::<BTreeMap<_, _>>())).collect();
        let text = serde_json::to_string(&design).unwrap();
        for old in [a.asm, a.upper, a.fore, a.joint, a.link, a.top] {
            assert!(!text.contains(&old.to_string()), "{old} left");
        }
        // Placed at the point asked: the arm's origin (x 50 there) at (0, 200, 0).
        let top2 = sid(&map[a.top.to_string()]);
        assert_eq!(s[&top2]["assembly.instance"]["at"], json!([0.0, 200.0, 0.0]));
        let top1 = sid(&first["ids"][a.top.to_string()]);
        assert_eq!(s[&top1]["assembly.instance"]["at"], json!([50.0, 0.0, 0.0]));
        // Every placed instance evaluates here.
        let p = dst.project.lock().unwrap();
        assert!(p.get::<de_assembly::AssemblyStats>(top1).is_some() && p.get::<de_assembly::AssemblyStats>(top2).is_some());
    }

    #[test]
    fn a_link_updates_breaks_and_undoes() {
        let server = server_in(&temp_dir());
        let a = arm(&server);
        let dst = server.project(Some(&"dst".to_string())).unwrap();
        let src = read_source(&server, "arm").unwrap();
        let out = ok(&dst, "link.insert", insert_parameters(&src, &[a.board.to_string()], false, true, false, Place::Same, &server.engine).unwrap());
        let link = sid(&out["link"]);
        let r1 = sid(&out["ids"][a.r1.to_string()]);
        let (_, rec) = link_named(&dst, r1).unwrap();
        assert_eq!(status(&server, &dst, link, &rec)["state"], "up_to_date");
        // The source moves R1 and gains a capacitor.
        ok(&a.app, "pcb.component.update", json!({ "target": a.r1, "at": [6, 12] }));
        ok(&a.app, "pcb.component.create", json!({ "board": a.board, "reference": "C1", "footprint": "C_0603", "value": "100n", "at": [20, 15] }));
        let st = status(&server, &dst, link, &rec);
        assert_eq!(st["state"], "update_available", "{st}");
        assert_eq!(st["changes"]["created"], 1);
        let who = actor(None);
        let up = update_link(&server, &dst, &who, link, &rec).unwrap();
        assert!(up["conflicts"].as_array().unwrap().is_empty(), "{up}");
        let s = state(&dst);
        assert_eq!(s[&r1]["pcb.component"]["at"], json!([6.0, 12.0]));
        assert_eq!(s.values().filter(|c| c.contains_key("pcb.component")).count(), 3);
        let (_, rec) = link_named(&dst, link).unwrap();
        assert_eq!(status(&server, &dst, link, &rec)["state"], "up_to_date");
        // Undo the update; break the link; the board stays as a copy.
        dst.project.lock().unwrap().undo(&who).unwrap();
        assert_eq!(state(&dst)[&r1]["pcb.component"]["at"], json!([8.0, 10.0]));
        ok(&dst, "link.break", json!({ "link": r1 }));
        let s = state(&dst);
        assert!(s.values().all(|c| !c.contains_key("core.link") && !c.contains_key("core.linked")));
        assert_eq!(s.values().filter(|c| c.contains_key("pcb.board")).count(), 1);
    }

    async fn http(at: SocketAddr, method: &str, path: &str, body: Option<Value>) -> Value {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(at).await.unwrap();
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {at}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf).to_string();
        let reply = &text[text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0)..];
        serde_json::from_str(reply).unwrap_or_else(|_| json!({ "error": reply }))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn over_http_a_whole_project_copies_and_a_linked_board_updates_by_itself() {
        let server = server_in(&temp_dir());
        let a = arm(&server);
        let at = serve(server.clone()).await;
        let tree = http(at, "GET", "/link/tree?project=dst&from=arm", None).await;
        assert_eq!(tree["title"], "Arm project");
        assert_eq!(tree["items"].as_array().unwrap().len(), 3);
        assert!(http(at, "GET", "/link/tree?project=arm&from=arm", None).await["error"].as_str().unwrap().contains("this project"));

        // The whole project, copied.
        let whole = http(at, "POST", "/link/insert?project=copy", Some(json!({ "from": "arm", "whole": true, "copy": true }))).await;
        assert!(whole["output"]["link"].is_null(), "{whole}");
        let copy = server.project(Some(&"copy".to_string())).unwrap();
        let s = state(&copy);
        assert_eq!(s.values().filter(|c| c.contains_key("assembly.definition")).count(), 1);
        assert_eq!(s.values().filter(|c| c.contains_key("pcb.board")).count(), 1);
        assert!(s.values().any(|c| c.get("shape.program").is_some_and(|p| p["name"] == "Plate")));
        assert!(s.values().all(|c| !c.contains_key("core.project")), "the project's own name stays its own");

        // The board, linked and set to update by itself.
        let ins = http(at, "POST", "/link/insert?project=dst", Some(json!({ "from": "arm", "items": ["Driver board"], "auto_update": true }))).await;
        let r1 = sid(&ins["output"]["ids"][a.r1.to_string()]);
        let links = http(at, "GET", "/links?project=dst", None).await;
        assert_eq!(links[0]["state"], "up_to_date", "{links}");
        assert_eq!(links[0]["auto_update"], true);
        assert_eq!(links[0]["local_edits"], false);
        // Read-only here: an edit is refused with what to do; edited locally it goes through.
        let dst = server.project(Some(&"dst".to_string())).unwrap();
        let refused = dst.commit_here("pcb.component.update", json!({ "target": r1, "value": "22k" })).unwrap_err();
        assert!(refused.contains("is linked from Arm project (arm)") && refused.contains("links break") && refused.contains("links local"), "{refused}");
        let set = http(at, "POST", "/links/settings?project=dst", Some(json!({ "link": r1, "local_edits": true }))).await;
        assert_eq!((set["local_edits"].clone(), set["auto_update"].clone()), (json!(true), json!(true)), "{set}");
        assert_eq!(http(at, "GET", "/links?project=dst", None).await[0]["local_edits"], true);
        dst.commit_here("pcb.component.update", json!({ "target": r1, "value": "22k" })).unwrap();
        http(at, "POST", "/links/settings?project=dst", Some(json!({ "link": r1, "local_edits": false }))).await;
        assert!(http(at, "POST", "/links/settings?project=dst", Some(json!({ "link": r1 }))).await["error"].as_str().unwrap().contains("local_edits"));
        ok(&a.app, "pcb.component.update", json!({ "target": a.r1, "at": [5, 5] }));
        let mut moved = false;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if state(&dst)[&r1]["pcb.component"]["at"] == json!([5.0, 5.0]) {
                moved = true;
                break;
            }
        }
        assert!(moved, "the change arrived by itself");
        // Then by hand: off, change, update.
        http(at, "POST", "/links/settings?project=dst", Some(json!({ "link": r1, "auto_update": false }))).await;
        ok(&a.app, "pcb.component.update", json!({ "target": a.r1, "at": [7, 7] }));
        assert_eq!(http(at, "GET", "/links?project=dst", None).await[0]["state"], "update_available");
        let up = http(at, "POST", "/links/update?project=dst", Some(json!({}))).await;
        assert!(!up[0]["output"]["changed"].as_array().unwrap().is_empty(), "{up}");
        assert_eq!(state(&dst)[&r1]["pcb.component"]["at"], json!([7.0, 7.0]));
        let broke = http(at, "POST", "/links/break?project=dst", Some(json!({ "link": r1 }))).await;
        assert_eq!(broke["project"], "arm");
        assert_eq!(http(at, "GET", "/links?project=dst", None).await, json!([]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn when_the_engine_starts_links_catch_up_with_what_changed_while_it_was_off() {
        let dir = temp_dir();
        // One engine: the arm's board linked twice, updating by itself and by hand; then it stops.
        let (r1_auto, r1_hand) = {
            let server = server_in(&dir);
            let a = arm(&server);
            let src = read_source(&server, "arm").unwrap();
            let auto = server.project(Some(&"auto".to_string())).unwrap();
            let out = ok(&auto, "link.insert", insert_parameters(&src, &[a.board.to_string()], false, true, true, Place::Same, &server.engine).unwrap());
            let r1_auto = sid(&out["ids"][a.r1.to_string()]);
            let hand = server.project(Some(&"hand".to_string())).unwrap();
            let out = ok(&hand, "link.insert", insert_parameters(&src, &[a.board.to_string()], false, true, false, Place::Same, &server.engine).unwrap());
            let r1_hand = sid(&out["ids"][a.r1.to_string()]);
            let plain = server.project(Some(&"plain".to_string())).unwrap();
            ok(&plain, "solid.material.create", json!({ "name": "Oak", "density": { "value": 700, "unit": "kg/m3" } }));
            // The source changes while no engine watches the links (only its file changes).
            ok(&a.app, "pcb.component.update", json!({ "target": a.r1, "at": [3, 4] }));
            (r1_auto, r1_hand)
        };
        // The engine starts again: nothing is open until the links are checked.
        let server = server_in(&dir);
        assert!(server.open.lock().unwrap().is_empty());
        start(&server);
        let mut caught_up = false;
        for _ in 0..80 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let open = server.open.lock().unwrap().get("auto").cloned();
            if let Some(app) = open
                && state(&app)[&r1_auto]["pcb.component"]["at"] == json!([3.0, 4.0])
            {
                caught_up = true;
                break;
            }
        }
        assert!(caught_up, "the automatic link took the change at start, with no view open");
        let mut open: Vec<String> = server.open.lock().unwrap().keys().cloned().collect();
        open.sort();
        assert_eq!(open, vec!["auto".to_string(), "hand".to_string()], "only projects with links are opened");
        let hand = server.project(Some(&"hand".to_string())).unwrap();
        assert_eq!(state(&hand)[&r1_hand]["pcb.component"]["at"], json!([8.0, 10.0]), "a link updated by hand waits");
        let (link, rec) = link_named(&hand, r1_hand).unwrap();
        assert_eq!(status(&server, &hand, link, &rec)["state"], "update_available");
    }
}
