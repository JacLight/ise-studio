//! Symbol and footprint libraries, the editors' previews and checks, the
//! footprint wizards, image-to-logo, footprint assignment (CvPcb), variant
//! outputs and design blocks, over HTTP. Changes go through the `pcb.lib.*`,
//! `pcb.variant.*` and `pcb.block.*` commands; these endpoints read.
//!
//! - `GET  /pcb/libs?project=`                          library table: project, global, KiCad, built-in
//! - `GET  /pcb/libs/symbol/<id>?project=`              a symbol (project `Library:Name` or built-in), drawn and checked
//! - `POST /pcb/libs/symbol/preview?project=`           a symbol being edited, drawn and checked (body: { symbol, extends?, unit?, power? })
//! - `GET  /pcb/libs/symbol/<id>/svg?project=&unit=`    a symbol as SVG
//! - `GET  /pcb/libs/footprint/<id>?project=`           a footprint (project or built-in), outlines and checks
//! - `POST /pcb/libs/footprint/preview`                 a footprint being edited (body: { footprint, model? })
//! - `GET  /pcb/libs/footprint/<id>/svg?project=`       a footprint as SVG
//! - `GET  /pcb/libs/wizards` / `POST /pcb/libs/wizard`  wizard presets / run one (body: a wizard)
//! - `POST /pcb/libs/bitmap`                            an image made into a logo footprint or symbol
//! - `GET  /pcb/libs/global` · `GET|POST|DELETE /pcb/libs/global/<name>`   libraries on this computer
//! - `POST /pcb/libs/global/<name>/from-project?project=&library=`         a project library saved to this computer
//! - `GET  /pcb/libs/assign/<schematic>?project=`       parts and the footprints that fit them (CvPcb)
//! - `GET  /pcb/variants/<schematic>?project=`          assembly variants with their part counts
//! - `GET  /pcb/<board>/variant/<name>/<file>?project=` bom.csv, positions.csv or fab.zip for a variant
//! - `GET  /pcb/blocks?project=`                        design blocks

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{StableId, Value, json};
use de_pcb::footprints::{Footprint, PadKind, pad_polygon, pad_prim};
use de_pcb::library::{self, LibFootprint, LibFootprintIn, LibSymbolIn, Model3d};
use de_pcb::symbols::SymbolDef;
use de_pcb::{bitmap, blocks, fpedit, libedit, schematic, symbols, variants, wizards};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

fn fail(code: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (code, message.into()).into_response()
}

type Q = Query<HashMap<String, String>>;

fn with_world<T>(server: &Server, query: &HashMap<String, String>, f: impl FnOnce(&de_core::bevy_ecs::world::World) -> Result<T, axum::response::Response>) -> Result<T, axum::response::Response> {
    let app = server.project(query.get("project")).map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let project = app.project.lock().unwrap();
    f(project.world())
}

fn respond(r: Result<Value, axum::response::Response>) -> axum::response::Response {
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e,
    }
}

// ---- Global libraries (this computer) ---------------------------------------------

/// Where libraries shared by every project on this computer live: `FORMA_LIBRARIES`, else the user's data folder.
pub fn global_root() -> PathBuf {
    if let Ok(p) = std::env::var("FORMA_LIBRARIES") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        return PathBuf::from(home).join("Library/Application Support/Forma/libraries");
    }
    if cfg!(windows) {
        if let Ok(a) = std::env::var("APPDATA") {
            return PathBuf::from(a).join("Forma/libraries");
        }
    }
    match std::env::var("XDG_DATA_HOME") {
        Ok(x) if !x.is_empty() => PathBuf::from(x).join("forma/libraries"),
        _ => PathBuf::from(home).join(".local/share/forma/libraries"),
    }
}

#[derive(serde::Serialize, Deserialize, Default, Clone)]
pub struct GlobalLib {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub symbols: Vec<LibSymbolIn>,
    #[serde(default)]
    pub footprints: Vec<LibFootprintIn>,
}

fn safe_name(n: &str) -> Result<String, axum::response::Response> {
    let n = n.trim();
    if n.is_empty() || n.len() > 64 || !n.chars().all(|c| c.is_ascii_alphanumeric() || "-_ .".contains(c)) || n.starts_with('.') {
        return Err(fail(StatusCode::BAD_REQUEST, format!("“{n}” is not a library name (letters, digits, - _ . and spaces)")));
    }
    Ok(n.to_string())
}

fn read_global(name: &str) -> Option<GlobalLib> {
    let t = std::fs::read_to_string(global_root().join(format!("{name}.json"))).ok()?;
    serde_json::from_str(&t).ok()
}

fn write_global(lib: &GlobalLib) -> Result<(), String> {
    let root = global_root();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let tmp = root.join(format!(".{}.json.tmp", lib.name));
    std::fs::write(&tmp, serde_json::to_string_pretty(lib).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, root.join(format!("{}.json", lib.name))).map_err(|e| e.to_string())
}

fn global_list() -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(global_root()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if name.starts_with('.') {
                    continue;
                }
                if let Some(l) = read_global(&name) {
                    out.push(json!({ "name": l.name, "description": l.description, "symbols": l.symbols.iter().map(|s| s.symbol.name.clone()).collect::<Vec<_>>(), "footprints": l.footprints.iter().map(|f| f.footprint.name.clone()).collect::<Vec<_>>() }));
                }
            }
        }
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

pub async fn global_index() -> axum::response::Response {
    axum::Json(json!({ "root": global_root(), "libraries": global_list() })).into_response()
}

pub async fn global_get(Path(name): Path<String>) -> axum::response::Response {
    let name = match safe_name(&name) {
        Ok(n) => n,
        Err(e) => return e,
    };
    match read_global(&name) {
        Some(l) => axum::Json(l).into_response(),
        None => fail(StatusCode::NOT_FOUND, format!("no library {name} on this computer")),
    }
}

#[derive(Deserialize)]
pub struct GlobalSave {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub symbols: Vec<LibSymbolIn>,
    #[serde(default)]
    pub footprints: Vec<LibFootprintIn>,
    /// Add to (replacing same-named parts) instead of replacing the library.
    #[serde(default)]
    pub merge: bool,
}

fn merge_into(lib: &mut GlobalLib, symbols: Vec<LibSymbolIn>, footprints: Vec<LibFootprintIn>, merge: bool) {
    if !merge {
        lib.symbols.clear();
        lib.footprints.clear();
    }
    for s in symbols {
        lib.symbols.retain(|x| x.symbol.name != s.symbol.name);
        lib.symbols.push(s);
    }
    for f in footprints {
        lib.footprints.retain(|x| x.footprint.name != f.footprint.name);
        lib.footprints.push(f);
    }
    lib.symbols.sort_by(|a, b| a.symbol.name.cmp(&b.symbol.name));
    lib.footprints.sort_by(|a, b| a.footprint.name.cmp(&b.footprint.name));
}

pub async fn global_post(Path(name): Path<String>, axum::Json(body): axum::Json<GlobalSave>) -> axum::response::Response {
    let name = match safe_name(&name) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let mut lib = read_global(&name).unwrap_or(GlobalLib { name: name.clone(), ..Default::default() });
    if let Some(d) = body.description {
        lib.description = d;
    }
    merge_into(&mut lib, body.symbols, body.footprints, body.merge);
    match write_global(&lib) {
        Ok(()) => axum::Json(json!({ "name": name, "symbols": lib.symbols.len(), "footprints": lib.footprints.len() })).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub async fn global_delete(Path(name): Path<String>) -> axum::response::Response {
    let name = match safe_name(&name) {
        Ok(n) => n,
        Err(e) => return e,
    };
    match std::fs::remove_file(global_root().join(format!("{name}.json"))) {
        Ok(()) => axum::Json(json!({ "deleted": name })).into_response(),
        Err(e) => fail(StatusCode::NOT_FOUND, e.to_string()),
    }
}

/// A project library (or some of its parts) saved into a library on this computer.
pub async fn global_from_project(State(server): State<Arc<Server>>, Path(name): Path<String>, Query(query): Q) -> axum::response::Response {
    let name = match safe_name(&name) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let from = query.get("library").cloned().unwrap_or_else(|| library::DEFAULT_LIBRARY.into());
    let only: Option<Vec<String>> = query.get("parts").map(|p| p.split(',').map(|s| s.trim().to_string()).collect());
    let picked = with_world(&server, &query, |w| {
        let keep = |n: &str| only.as_ref().is_none_or(|o| o.iter().any(|x| x == n));
        let s: Vec<LibSymbolIn> = library::lib_symbols(w).into_iter().filter(|(_, s)| s.library == from && keep(&s.symbol.name)).map(|(_, s)| LibSymbolIn { symbol: s.symbol, extends: s.extends, power: s.power }).collect();
        let f: Vec<LibFootprintIn> = library::lib_footprints(w).into_iter().filter(|(_, f)| f.library == from && keep(&f.footprint.name)).map(|(_, f)| LibFootprintIn { footprint: f.footprint, model: f.model }).collect();
        Ok((s, f))
    });
    let (s, f) = match picked {
        Ok(x) => x,
        Err(e) => return e,
    };
    if s.is_empty() && f.is_empty() {
        return fail(StatusCode::NOT_FOUND, format!("nothing in the project library {from}"));
    }
    let mut lib = read_global(&name).unwrap_or(GlobalLib { name: name.clone(), ..Default::default() });
    let (ns, nf) = (s.len(), f.len());
    merge_into(&mut lib, s, f, true);
    match write_global(&lib) {
        Ok(()) => axum::Json(json!({ "name": name, "added_symbols": ns, "added_footprints": nf, "path": global_root().join(format!("{name}.json")) })).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// ---- Library table ---------------------------------------------------------------

pub async fn index(State(server): State<Arc<Server>>, Query(query): Q) -> axum::response::Response {
    respond(with_world(&server, &query, |w| {
        let project = library::listing(w);
        let table = library::table(w).map(|(_, t)| t.libraries).unwrap_or_default();
        let blocks: Vec<Value> = blocks::blocks(w).into_iter().map(|(id, b)| json!({ "id": id, "name": b.name, "description": b.description, "symbols": b.symbols.len(), "layout": b.layout.is_some() })).collect();
        Ok(json!({
            "table": table,
            "project": project,
            "global": { "root": global_root(), "libraries": global_list() },
            "kicad": de_pcb::kicad_lib::Library::default().status(),
            "builtin": { "symbols": symbols::names(), "footprints": de_pcb::footprints::names() },
            "blocks": blocks,
        }))
    }))
}

// ---- Symbols -----------------------------------------------------------------------

fn preview_symbol(def: &SymbolDef, unit: u32) -> Value {
    let s = schematic::SchSymbol {
        schematic: StableId::new(),
        sheet: "1".into(),
        symbol: def.name.clone(),
        reference: format!("{}?", def.prefix),
        value: def.value.clone(),
        footprint: def.footprint.clone(),
        at: [0.0, 0.0],
        rotation: 0.0,
        mirror: false,
        unit: unit.clamp(1, def.unit_count()),
        mpn: None,
        manufacturer: None,
        lcsc: None,
        description: String::new(),
        dnp: false,
        pin_map: Default::default(),
        pins: vec![],
        fields: Default::default(),
        ref_at: None,
        value_at: None,
        exclude_from_board: false,
        component: None,
        definition: Some(Box::new(def.clone())),
    };
    let v = de_pcb::schematic_view::symbol_preview(&s);
    json!({ "draw": v.draw, "bbox": v.bbox, "body": v.body, "pins": v.pins, "unit": s.unit, "units": def.unit_count() })
}

fn footprint_for(w: &de_core::bevy_ecs::world::World, name: &str) -> Option<Footprint> {
    de_pcb::footprints::get(name).or_else(|| library::find_footprint(w, name).map(|(_, f)| library::placeable_footprint(&f))).or_else(|| de_pcb::boards(w).into_iter().find_map(|(_, b)| b.footprints.get(name).cloned()))
}

pub async fn symbol(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Q) -> axum::response::Response {
    let unit: u32 = query.get("unit").and_then(|u| u.parse().ok()).unwrap_or(1);
    respond(with_world(&server, &query, |w| {
        let entry = library::find_symbol(w, &id);
        let def = library::resolve_symbol(w, &id).map_err(|e| fail(StatusCode::NOT_FOUND, e))?;
        let fp = footprint_for(w, &def.footprint);
        let findings = libedit::check_symbol(&def, fp.as_ref());
        let style2 = libedit::swap_body_style(&def).map(|d| preview_symbol(&d, unit));
        Ok(json!({ "kind": if entry.is_some() { "project" } else { "builtin" }, "entry": entry.map(|(eid, e)| json!({ "id": eid, "library": e.library, "symbol": e.symbol, "extends": e.extends, "power": e.power, "origin": e.origin })), "definition": def, "preview": preview_symbol(&def, unit), "demorgan_preview": style2, "findings": findings }))
    }))
}

#[derive(Deserialize)]
pub struct SymbolPreview {
    pub symbol: SymbolDef,
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub unit: Option<u32>,
    #[serde(default)]
    pub body_style: Option<u8>,
}

pub async fn symbol_preview(State(server): State<Arc<Server>>, Query(query): Q, axum::Json(body): axum::Json<SymbolPreview>) -> axum::response::Response {
    respond(with_world(&server, &query, |w| {
        let mut def = match &body.extends {
            Some(p) if !p.trim().is_empty() => {
                let parent = library::resolve_symbol(w, p).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?;
                libedit::flatten(&body.symbol, &parent)
            }
            _ => body.symbol.clone(),
        };
        if def.units.is_empty() {
            def.units = vec![vec![]];
        }
        let fp = footprint_for(w, &def.footprint);
        let findings = libedit::check_symbol(&def, fp.as_ref());
        let shown = if body.body_style == Some(2) { libedit::swap_body_style(&def).unwrap_or(def.clone()) } else { def.clone() };
        Ok(json!({ "definition": def, "preview": preview_symbol(&shown, body.unit.unwrap_or(1)), "findings": findings, "footprint_pads": fp.map(|f| f.pads.iter().filter(|p| p.kind != PadKind::Npth && !p.number.is_empty()).map(|p| p.number.clone()).collect::<Vec<_>>()) }))
    }))
}

pub async fn symbol_svg(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Q) -> axum::response::Response {
    let unit: u32 = query.get("unit").and_then(|u| u.parse().ok()).unwrap_or(1);
    let r = with_world(&server, &query, |w| library::resolve_symbol(w, &id).map_err(|e| fail(StatusCode::NOT_FOUND, e)));
    match r {
        Ok(def) => {
            let p = preview_symbol(&def, unit);
            let draw: Vec<de_pcb::schematic_view::Draw> = {
                let s = schematic::SchSymbol { schematic: StableId::new(), sheet: "1".into(), symbol: def.name.clone(), reference: format!("{}?", def.prefix), value: def.value.clone(), footprint: String::new(), at: [0.0, 0.0], rotation: 0.0, mirror: false, unit: unit.clamp(1, def.unit_count()), mpn: None, manufacturer: None, lcsc: None, description: String::new(), dnp: false, pin_map: Default::default(), pins: vec![], fields: Default::default(), ref_at: None, value_at: None, exclude_from_board: false, component: None, definition: Some(Box::new(def.clone())) };
                de_pcb::schematic_view::symbol_preview(&s).draw
            };
            let bbox: [f64; 4] = serde_json::from_value(p["bbox"].clone()).unwrap_or([-5.0, -5.0, 5.0, 5.0]);
            let svg = fpedit::symbol_svg(&def.name, &draw, bbox);
            svg_response(svg, &format!("{}.svg", def.name.replace(':', "_")), query.contains_key("download"))
        }
        Err(e) => e,
    }
}

fn svg_response(svg: String, name: &str, download: bool) -> axum::response::Response {
    let disposition = if download { format!("attachment; filename=\"{name}\"") } else { "inline".into() };
    ([(header::CONTENT_TYPE, "image/svg+xml".to_string()), (header::CONTENT_DISPOSITION, disposition)], svg).into_response()
}

// ---- Footprints --------------------------------------------------------------------

/// What the footprint editor draws: each pad's outline (footprint coordinates, y up), holes, courtyard, silk, body.
pub fn footprint_view(f: &Footprint, rules: &de_pcb::Rules) -> Value {
    let pads: Vec<Value> = f
        .pads
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let poly = pad_polygon(p, [0.0, 0.0], 0.0, false);
            let outline = if poly.is_empty() { pad_prim(p, [0.0, 0.0], 0.0, false).outline(0.01) } else { poly };
            let hole = p.drill.map(|d| {
                let off = p.extra.drill_offset.map(|o| de_pcb::geom::rotate(o, p.rotation)).unwrap_or([0.0, 0.0]);
                json!({ "at": [p.at[0] + off[0], p.at[1] + off[1]], "size": d, "rotation": p.rotation })
            });
            json!({ "index": i, "number": p.number, "kind": p.kind, "at": p.at, "outline": outline, "hole": hole })
        })
        .collect();
    json!({ "footprint": f, "pads": pads, "courtyard": f.courtyard_rect(), "silk": f.silk_lines(), "findings": fpedit::check_footprint(f, rules), "height": f.height() })
}

pub async fn footprint(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Q) -> axum::response::Response {
    respond(with_world(&server, &query, |w| {
        if let Some((eid, e)) = library::find_footprint(w, &id) {
            let placed = library::placeable_footprint(&e);
            let mut v = footprint_view(&placed, &de_pcb::Rules::default());
            v["kind"] = json!("project");
            v["entry"] = json!({ "id": eid, "library": e.library, "footprint": e.footprint, "model": e.model, "origin": e.origin });
            return Ok(v);
        }
        let f = de_pcb::footprints::get(&id).or_else(|| de_pcb::boards(w).into_iter().find_map(|(_, b)| b.footprints.get(&id).cloned())).ok_or_else(|| fail(StatusCode::NOT_FOUND, format!("no footprint {id}")))?;
        let mut v = footprint_view(&f, &de_pcb::Rules::default());
        v["kind"] = json!(if de_pcb::footprints::get(&id).is_some() { "builtin" } else { "board" });
        Ok(v)
    }))
}

#[derive(Deserialize)]
pub struct FootprintPreview {
    pub footprint: Footprint,
    #[serde(default)]
    pub model: Option<Model3d>,
    #[serde(default)]
    pub library: Option<String>,
    /// Renumber: { start, prefix, reading_order }.
    #[serde(default)]
    pub enumerate: Option<(u32, String, bool)>,
}

pub async fn footprint_preview(axum::Json(body): axum::Json<FootprintPreview>) -> axum::response::Response {
    let mut fp = body.footprint;
    fp.pads.iter_mut().for_each(|p| p.normalize());
    if let Some((start, prefix, order)) = &body.enumerate {
        fpedit::enumerate_pads(&mut fp, *start, prefix, *order);
    }
    let lf = LibFootprint { library: body.library.unwrap_or_else(|| library::DEFAULT_LIBRARY.into()), footprint: fp.clone(), model: body.model, origin: None };
    let placed = library::placeable_footprint(&lf);
    let mut v = footprint_view(&placed, &de_pcb::Rules::default());
    // The editor keeps editing the library copy (body before the model offset).
    v["edited"] = json!(fp);
    axum::Json(v).into_response()
}

pub async fn footprint_svg(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Q) -> axum::response::Response {
    let r = with_world(&server, &query, |w| footprint_for(w, &id).ok_or_else(|| fail(StatusCode::NOT_FOUND, format!("no footprint {id}"))));
    match r {
        Ok(f) => svg_response(fpedit::footprint_svg(&f), &format!("{}.svg", f.name.replace(':', "_")), query.contains_key("download")),
        Err(e) => e,
    }
}

pub async fn wizard_presets() -> axum::response::Response {
    axum::Json(json!({ "presets": wizards::presets().into_iter().map(|(n, w)| json!({ "name": n, "wizard": w })).collect::<Vec<_>>() })).into_response()
}

pub async fn wizard(axum::Json(w): axum::Json<wizards::Wizard>) -> axum::response::Response {
    match wizards::generate(&w) {
        Ok(f) => axum::Json(footprint_view(&f, &de_pcb::Rules::default())).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

pub async fn bitmap(axum::Json(req): axum::Json<bitmap::BitmapRequest>) -> axum::response::Response {
    match tokio::task::spawn_blocking(move || bitmap::make(&req)).await {
        Ok(Ok(bitmap::Made::Footprint(f))) => {
            let mut v = footprint_view(&f, &de_pcb::Rules::default());
            v["kind"] = json!("footprint");
            axum::Json(v).into_response()
        }
        Ok(Ok(bitmap::Made::Symbol(s))) => axum::Json(json!({ "kind": "symbol", "symbol": s, "preview": preview_symbol(&s, 1) })).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- Footprint assignment (CvPcb) ---------------------------------------------------

pub async fn assign(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Q) -> axum::response::Response {
    respond(with_world(&server, &query, |w| {
        let sid: StableId = serde_json::from_value(json!(id)).map_err(|_| fail(StatusCode::NOT_FOUND, "not a schematic"))?;
        let sch = w.resource::<de_core::IdIndex>().get(sid).and_then(|e| w.get::<schematic::Schematic>(e)).cloned().ok_or_else(|| fail(StatusCode::NOT_FOUND, "not a schematic"))?;
        let board = sch.board.and_then(|b| w.resource::<de_core::IdIndex>().get(b).and_then(|e| w.get::<de_pcb::Board>(e)).cloned());
        // Every footprint Forma has to hand: built-in, project libraries, the board's own.
        let mut fps: Vec<Value> = Vec::new();
        let mut push = |f: &Footprint, source: &str| {
            let pads: std::collections::BTreeSet<&str> = f.pads.iter().filter(|p| p.kind != PadKind::Npth && !p.number.is_empty()).map(|p| p.number.as_str()).collect();
            fps.push(json!({ "name": f.name, "source": source, "pads": pads.len(), "numbers": pads, "description": f.description, "kind": f.kind }));
        };
        for n in de_pcb::footprints::names() {
            if let Some(f) = de_pcb::footprints::get(&n) {
                push(&f, "built-in");
            }
        }
        // Project libraries switched off in the library table are not offered.
        let off: Vec<String> = library::table(w).map(|(_, t)| t.libraries.into_iter().filter(|l| l.kind == "project" && !l.enabled).map(|l| l.name).collect()).unwrap_or_default();
        for (_, f) in library::lib_footprints(w) {
            if !off.contains(&f.library) {
                push(&library::placeable_footprint(&f), "project");
            }
        }
        if let Some(b) = &board {
            for f in b.footprints.values() {
                if de_pcb::footprints::get(&f.name).is_none() && library::find_footprint(w, &f.name).is_none() {
                    push(f, "board");
                }
            }
        }
        let mut parts: Vec<Value> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (_, s) in schematic::items(w, sid).symbols {
            if !seen.insert(s.reference.clone()) || s.reference.starts_with('#') {
                continue;
            }
            let Some(def) = s.def() else { continue };
            if def.virtual_part || s.exclude_from_board {
                continue;
            }
            let mut pads: Vec<String> = def.pins.iter().filter(|p| !(p.hidden && p.etype == symbols::PinType::NoConnect)).flat_map(|p| s.pads_of(p)).collect();
            pads.sort_by(|a, b| de_pcb::natural(a, b));
            pads.dedup();
            // Every pad the part may have, unconnected ones too (a footprint fits when it has the
            // connected pads and no more than these).
            let mut all: Vec<String> = def.pins.iter().flat_map(|p| s.pads_of(p)).collect();
            all.sort();
            all.dedup();
            parts.push(json!({ "reference": s.reference, "value": s.value, "symbol": s.symbol, "footprint": s.footprint, "pads": pads, "pads_all": all.len(), "candidates": def.footprints, "default": def.footprint, "description": def.description }));
        }
        parts.sort_by(|a, b| de_pcb::natural(a["reference"].as_str().unwrap_or(""), b["reference"].as_str().unwrap_or("")));
        Ok(json!({ "schematic": sid, "board": sch.board, "parts": parts, "footprints": fps }))
    }))
}

// ---- Variants ------------------------------------------------------------------------

pub async fn variant_list(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Q) -> axum::response::Response {
    respond(with_world(&server, &query, |w| {
        let sid: StableId = serde_json::from_value(json!(id)).map_err(|_| fail(StatusCode::NOT_FOUND, "not a schematic"))?;
        let sch = w.resource::<de_core::IdIndex>().get(sid).and_then(|e| w.get::<schematic::Schematic>(e)).cloned().ok_or_else(|| fail(StatusCode::NOT_FOUND, "not a schematic"))?;
        let view = sch.board.and_then(|b| de_pcb::view(w, b));
        let list: Vec<Value> = variants::variants(w, Some(sid))
            .into_iter()
            .map(|(vid, v)| {
                let fitted = view.as_ref().map(|bv| v.apply(bv).components.iter().filter(|c| !c.dnp && c.kind != "hole").count());
                json!({ "id": vid, "variant": v, "fitted": fitted })
            })
            .collect();
        let parts: Vec<Value> = {
            let mut seen = std::collections::BTreeSet::new();
            schematic::items(w, sid).symbols.into_iter().filter(|(_, s)| !s.reference.starts_with('#') && seen.insert(s.reference.clone()) && s.def().is_none_or(|d| !d.virtual_part)).map(|(_, s)| json!({ "reference": s.reference, "value": s.value, "mpn": s.mpn, "lcsc": s.lcsc, "manufacturer": s.manufacturer, "dnp": s.dnp, "footprint": s.footprint })).collect()
        };
        Ok(json!({ "schematic": sid, "board": sch.board, "variants": list, "parts": parts }))
    }))
}

pub async fn variant_file(State(server): State<Arc<Server>>, Path((id, name, file)): Path<(String, String, String)>, Query(query): Q) -> axum::response::Response {
    let got = with_world(&server, &query, |w| {
        let bid: StableId = serde_json::from_value(json!(id)).map_err(|_| fail(StatusCode::NOT_FOUND, "not a board"))?;
        let b = w.resource::<de_core::IdIndex>().get(bid).and_then(|e| w.get::<de_pcb::Board>(e)).cloned().ok_or_else(|| fail(StatusCode::NOT_FOUND, "not a board"))?;
        let v = de_pcb::view(w, bid).ok_or_else(|| fail(StatusCode::UNPROCESSABLE_ENTITY, "the board is not worked out yet"))?;
        let var = variants::of_board(w, bid, &name).ok_or_else(|| fail(StatusCode::NOT_FOUND, format!("no variant {name} (variants belong to the schematic that drives the board)")))?;
        Ok((b, var.apply(&v), var))
    });
    let (b, v, var) = match got {
        Ok(x) => x,
        Err(e) => return e,
    };
    let format = de_pcb::outputs::Format::parse(query.get("format").map(String::as_str).unwrap_or(""));
    let stem = format!("{}-{}", de_pcb::gerber::file_stem(&v.name), de_pcb::gerber::file_stem(&var.name));
    match file.as_str() {
        "bom.csv" | "positions.csv" => {
            let body = if file == "bom.csv" { de_pcb::outputs::bom(&v, &b, format) } else { de_pcb::outputs::pick_and_place(&v, format) };
            ([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{stem}-{file}\""))], body).into_response()
        }
        "bom.json" => axum::Json(json!({ "variant": var.name, "lines": de_pcb::sourcing::bom_lines(&v, &b) })).into_response(),
        "fab.zip" => {
            let fab = query.get("fab").filter(|f| !f.is_empty()).cloned();
            match tokio::task::spawn_blocking(move || de_pcb::outputs::zip(&de_pcb::outputs::package(&v, &b, fab.as_deref()))).await {
                Ok(bytes) => ([(header::CONTENT_TYPE, "application/zip".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{stem}.zip\""))], bytes).into_response(),
                Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        _ => fail(StatusCode::NOT_FOUND, format!("no variant file {file} (bom.csv, bom.json, positions.csv, fab.zip)")),
    }
}

// ---- Design blocks ---------------------------------------------------------------------

pub async fn blocks_list(State(server): State<Arc<Server>>, Query(query): Q) -> axum::response::Response {
    respond(with_world(&server, &query, |w| Ok(json!({ "blocks": blocks::blocks(w).into_iter().map(|(id, b)| json!({ "id": id, "block": b })).collect::<Vec<_>>() }))))
}

/// Library findings for `./forma check`: errors in project library parts.
pub fn findings(project: &de_core::Project) -> Vec<Value> {
    let w = project.world();
    let mut out = Vec::new();
    for (id, s) in library::lib_symbols(w) {
        let full = library::full_id(&s.library, &s.symbol.name);
        match library::resolve_symbol(w, &full) {
            Ok(def) => {
                let fp = footprint_for(w, &def.footprint);
                for f in libedit::check_symbol(&def, fp.as_ref()).into_iter().filter(|f| f.severity == "error") {
                    out.push(json!({ "code": "PCB_LIB", "severity": "error", "entities": [id], "message": format!("symbol {full}: {} ({})", f.message, f.code), "fix": null }));
                }
            }
            Err(e) => out.push(json!({ "code": "PCB_LIB", "severity": "error", "entities": [id], "message": format!("symbol {full}: {e}"), "fix": null })),
        }
    }
    for (id, f) in library::lib_footprints(w) {
        let full = library::full_id(&f.library, &f.footprint.name);
        for x in fpedit::check_footprint(&library::placeable_footprint(&f), &de_pcb::Rules::default()).into_iter().filter(|x| x.severity == "error") {
            out.push(json!({ "code": "PCB_LIB", "severity": "error", "entities": [id], "message": format!("footprint {full}: {} ({})", x.message, x.code), "fix": null }));
        }
    }
    out
}
