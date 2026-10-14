//! Schematic endpoints: the symbol library, a project's schematics, one
//! schematic resolved (every item drawn, nets, ERC, and the pending
//! "Update board" report), and its PDF.
//!
//! - `GET /pcb/symbols`                              every built-in symbol, drawn
//! - `GET /pcb/schematics?project=`                  schematics (id, name, board, errors)
//! - `GET /pcb/schematic/<id>?project=`              view + pending sync report
//! - `GET /pcb/schematic/<id>/schematic.pdf?project=` every sheet as a PDF page
//! - `GET /pcb/schematic/<id>/sheet/<sheet>.svg?project=` one sheet as SVG
//! - `GET /pcb/schematic/<id>/netlist?format=kicad|orcad|cadstar|spice` a netlist file
//! - `GET /pcb/schematic/<id>/bom?group=Value,Footprint&fields=Reference,Qty,Value&dnp=1&format=csv|json`
//! - `POST /pcb/schematic/<id>/simulate?project=`    ngspice on the schematic (body: a SimRequest)
//! - `GET /pcb/simulations?project=`                 saved simulations (requirements)

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{StableId, Value, json};
use de_pcb::{schematic, schematic_pdf, schematic_sync, symbols};
use std::collections::HashMap;
use std::sync::Arc;

fn fail(code: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (code, message.into()).into_response()
}

pub async fn library() -> impl IntoResponse {
    let list: Vec<Value> = symbols::names()
        .into_iter()
        .filter_map(|n| symbols::get(&n))
        .map(|d| {
            let body = d.body_bbox(1);
            json!({ "name": d.name, "description": d.description, "prefix": d.prefix, "category": d.category, "value": d.value, "footprint": d.footprint, "footprints": d.footprints, "mpn": d.mpn, "lcsc": d.lcsc, "units": d.unit_count(), "keywords": d.keywords, "pins": d.pins.iter().filter(|p| !p.hidden).count(), "body": body, "virtual": d.virtual_part })
        })
        .collect();
    axum::Json(json!({ "symbols": list }))
}

/// A symbol drawn on its own (for the picker's previews).
pub async fn preview(Path(name): Path<String>) -> axum::response::Response {
    let Some(d) = symbols::get(&name) else { return fail(StatusCode::NOT_FOUND, format!("no symbol {name}")) };
    let s = schematic::SchSymbol {
        schematic: StableId::new(),
        sheet: "1".into(),
        symbol: d.name.clone(),
        reference: format!("{}?", d.prefix),
        value: d.value.clone(),
        footprint: d.footprint.clone(),
        at: [0.0, 0.0],
        rotation: 0.0,
        mirror: false,
        unit: 1,
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
        definition: None,
    };
    let v = de_pcb::schematic_view::symbol_preview(&s);
    axum::Json(json!({ "draw": v.draw, "bbox": v.bbox, "pins": v.pins })).into_response()
}

pub async fn list(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let project = app.project.lock().unwrap();
    let list: Vec<Value> = schematic::schematics(project.world())
        .into_iter()
        .map(|(id, s)| {
            let v = schematic::view(project.world(), id);
            json!({ "id": id, "name": s.name, "board": s.board, "sheets": s.sheets.len(), "errors": v.as_ref().map(|v| v.errors()), "symbols": v.as_ref().map(|v| v.symbols.len()) })
        })
        .collect();
    axum::Json(list).into_response()
}

fn view(server: &Server, query: &HashMap<String, String>, id: &str) -> Result<(Arc<schematic::SchematicView>, Value), axum::response::Response> {
    let app = server.project(query.get("project")).map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let project = app.project.lock().unwrap();
    let id: StableId = serde_json::from_value(json!(id)).map_err(|_| fail(StatusCode::NOT_FOUND, format!("{id} is not a schematic")))?;
    if project.get::<schematic::Schematic>(id).is_none() {
        return Err(fail(StatusCode::NOT_FOUND, format!("{id} is not a schematic")));
    }
    let v = match schematic::view(project.world(), id) {
        Some(v) => v,
        None => Arc::new(schematic::resolve(project.world(), id).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?),
    };
    let sync = match schematic_sync::pending(project.world(), id) {
        Ok(r) => serde_json::to_value(r).unwrap(),
        Err(e) => json!({ "error": e.message }),
    };
    Ok((v, sync))
}

pub async fn show(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    match view(&server, &query, &id) {
        Ok((v, sync)) => axum::Json(json!({ "schematic": &*v, "errors": v.errors(), "sync": sync })).into_response(),
        Err(r) => r,
    }
}

fn attachment(query: &HashMap<String, String>, name: &str) -> String {
    if query.contains_key("download") { format!("attachment; filename=\"{name}\"") } else { format!("inline; filename=\"{name}\"") }
}

/// One sheet as SVG.
pub async fn svg(State(server): State<Arc<Server>>, Path((id, sheet)): Path<(String, String)>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (v, _) = match view(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let sheet = sheet.trim_end_matches(".svg").to_string();
    let Some(body) = de_pcb::schematic_export::svg(&v, &sheet) else { return fail(StatusCode::NOT_FOUND, format!("no sheet {sheet}")) };
    let name = format!("{}-sheet{sheet}.svg", de_pcb::gerber::file_stem(&v.name));
    ([(header::CONTENT_TYPE, "image/svg+xml".to_string()), (header::CONTENT_DISPOSITION, attachment(&query, &name))], body).into_response()
}

/// A netlist file.
pub async fn netlist(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (v, _) = match view(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let format = query.get("format").map(String::as_str).unwrap_or("kicad");
    match de_pcb::schematic_export::netlist(&v, format) {
        Ok(body) => {
            let name = format!("{}.{}", de_pcb::gerber::file_stem(&v.name), de_pcb::schematic_export::extension(format));
            ([(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, attachment(&query, &name))], body).into_response()
        }
        Err(e) => fail(StatusCode::BAD_REQUEST, e),
    }
}

/// The bill of materials straight from the schematic, grouped and with the fields asked for.
pub async fn bom(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (v, _) = match view(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let list = |k: &str, d: &[&str]| -> Vec<String> {
        match query.get(k) {
            Some(x) => x.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            None => d.iter().map(|s| s.to_string()).collect(),
        }
    };
    let group = list("group", &["Value", "Footprint", "MPN"]);
    let fields = list("fields", &de_pcb::schematic_export::BOM_FIELDS[..8]);
    let dnp = query.get("dnp").is_some_and(|x| x == "1" || x == "true");
    if query.get("format").map(String::as_str) == Some("json") {
        let rows = de_pcb::schematic_export::bom_rows(&v, &group, &fields, dnp);
        return axum::Json(json!({ "fields": fields, "group": group, "rows": rows })).into_response();
    }
    let body = de_pcb::schematic_export::bom_csv(&v, &group, &fields, dnp);
    let name = format!("{}-bom.csv", de_pcb::gerber::file_stem(&v.name));
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, attachment(&query, &name))], body).into_response()
}

pub async fn pdf(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (v, _) = match view(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let bytes = schematic_pdf::pdf(&v);
    let name = format!("{}-schematic.pdf", de_pcb::gerber::file_stem(&v.name));
    let disposition = if query.contains_key("download") { format!("attachment; filename=\"{name}\"") } else { format!("inline; filename=\"{name}\"") };
    ([(header::CONTENT_TYPE, "application/pdf".to_string()), (header::CONTENT_DISPOSITION, disposition)], bytes).into_response()
}

/// `POST /pcb/schematic/<id>/simulate`: run ngspice on the schematic (body: a `SimRequest`).
pub async fn simulate(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, axum::Json(req): axum::Json<de_pcb::spice::SimRequest>) -> axum::response::Response {
    let (v, _) = match view(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match tokio::task::spawn_blocking(move || de_pcb::spice::simulate(&v, &req)).await {
        Ok(Ok(r)) => axum::Json(&*r).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `GET /pcb/simulations?project=`: the saved simulations.
pub async fn simulations(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let project = app.project.lock().unwrap();
    let list: Vec<Value> = de_pcb::spice::saved(project.world()).into_iter().map(|(id, s)| json!({ "id": id, "schematic": s.schematic, "name": s.name, "request": s.request })).collect();
    axum::Json(list).into_response()
}

/// Saved simulations that fail a datasheet check or an expectation, for `./ise check`.
pub fn findings(project: &de_core::Project) -> Vec<Value> {
    let mut out = Vec::new();
    for (id, s) in de_pcb::spice::saved(project.world()) {
        let Some(v) = schematic::view(project.world(), s.schematic) else { continue };
        match de_pcb::spice::simulate(&v, &s.request) {
            Ok(r) => {
                for c in r.checks.iter().chain(&r.expectations).filter(|c| c.status == "fail") {
                    out.push(json!({ "code": "PCB_SIM", "severity": "error", "entities": [id, s.schematic], "message": format!("simulation “{}”: {} — {}", s.name, c.name, c.detail), "fix": null }));
                }
            }
            Err(e) => out.push(json!({ "code": "PCB_SIM", "severity": "error", "entities": [id], "message": format!("simulation “{}” did not run: {e}", s.name), "fix": null })),
        }
    }
    out
}
