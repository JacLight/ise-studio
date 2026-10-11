//! Electronics endpoints: the board resolved and checked, its fabrication
//! files and package, the fab checks, BOM lines with distributor links, the
//! components as Buy lines for a project-wide BOM, the footprint library, and
//! a local slot for distributor API keys (kept on this computer, never in the
//! project, never sent anywhere by the engine on its own).
//!
//! - `GET /pcb/library`                      every built-in footprint
//! - `GET /pcb/boards?project=`              boards in the project (id, name, errors)
//! - `GET /pcb/buy?project=`                 every board's Buy lines, merged by MPN
//! - `GET /pcb/<id>?project=`                board view + fab reports + BOM lines + totals
//! - `GET /pcb/<id>/bom.json?project=`       that board's Buy lines
//! - `GET /pcb/<id>/file/<name>?project=`    one fabrication file (Gerber, drill, CSV, readme)
//! - `GET /pcb/<id>/fab.zip?project=&fab=`   the package for a fab (jlcpcb, pcbway, …) or generic
//! - `GET|POST /pcb/settings`                which API keys are set / set them

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{StableId, Value, json};
use de_pcb::{Board, BoardView, fabs, outputs, sourcing};
use std::collections::HashMap;
use std::sync::Arc;

fn fail(code: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (code, message.into()).into_response()
}

/// The board and its resolved view, or why not.
fn board(server: &Server, query: &HashMap<String, String>, id: &str) -> Result<(Board, Arc<BoardView>), axum::response::Response> {
    let app = server.project(query.get("project")).map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let project = app.project.lock().unwrap();
    let id: StableId = serde_json::from_value(json!(id)).map_err(|_| fail(StatusCode::NOT_FOUND, format!("{id} is not a board")))?;
    let b = project.get::<Board>(id).cloned().ok_or_else(|| fail(StatusCode::NOT_FOUND, format!("{id} is not a board")))?;
    let view = de_pcb::view(project.world(), id).ok_or_else(|| {
        let why = match project.status(id) {
            Some(de_core::Status::Failed(r)) | Some(de_core::Status::Blocked(r)) => r.clone(),
            _ => "the board is not worked out yet".into(),
        };
        fail(StatusCode::UNPROCESSABLE_ENTITY, why)
    })?;
    Ok((b, view))
}

pub async fn library() -> impl IntoResponse {
    let list: Vec<Value> = de_pcb::footprints::names()
        .into_iter()
        .filter_map(|n| de_pcb::footprints::get(&n))
        .map(|f| json!({ "name": f.name, "description": f.description, "source": f.source, "kind": f.kind, "height": f.height(), "courtyard": f.courtyard_rect(), "pads": f.pads, "body": f.body, "silk": f.silk_lines() }))
        .collect();
    axum::Json(json!({ "footprints": list, "parts": sourcing::KNOWN, "fabs": fabs::FABS }))
}

pub async fn boards(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let project = app.project.lock().unwrap();
    let list: Vec<Value> = de_pcb::boards(project.world())
        .into_iter()
        .map(|(id, b)| {
            let v = de_pcb::view(project.world(), id);
            json!({ "id": id, "name": b.name, "layers": b.layers, "model": b.model, "errors": v.as_ref().map(|v| v.errors()), "components": v.as_ref().map(|v| v.components.len()) })
        })
        .collect();
    axum::Json(list).into_response()
}

pub async fn buy(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let project = app.project.lock().unwrap();
    let mut lines = Vec::new();
    for (id, b) in de_pcb::boards(project.world()) {
        if let Some(v) = de_pcb::view(project.world(), id) {
            lines.extend(sourcing::buy_lines(&v, &b));
        }
    }
    axum::Json(json!({ "lines": sourcing::merge(&lines) })).into_response()
}

pub async fn show(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (b, v) = match board(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let lines = sourcing::bom_lines(&v, &b);
    let (totals, unpriced) = sourcing::totals(&lines, b.quantity);
    let files: Vec<String> = de_pcb::gerber::files(&v).into_iter().map(|(n, _)| n).chain(["bom.csv".to_string(), "positions.csv".to_string(), "README.txt".to_string()]).chain(interop_names(&v)).collect();
    axum::Json(json!({
        "board": &*v,
        "size": v.size(),
        "errors": v.errors(),
        "fabs": fabs::reports(&v),
        "bom": lines,
        "totals": totals,
        "unpriced": unpriced,
        "files": files,
        "sourcing": b.sourcing,
    }))
    .into_response()
}

pub async fn bom_json(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    match board(&server, &query, &id) {
        Ok((b, v)) => axum::Json(json!({ "lines": sourcing::buy_lines(&v, &b) })).into_response(),
        Err(r) => r,
    }
}

pub async fn file(State(server): State<Arc<Server>>, Path((id, name)): Path<(String, String)>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (b, v) = match board(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let format = outputs::Format::parse(query.get("format").map(String::as_str).unwrap_or(""));
    let stem = de_pcb::gerber::file_stem(&v.name);
    if name == format!("{stem}.step") {
        // The board's 3D part, exact (OpenCascade): the shape export does it.
        let Some(model) = b.model else { return fail(StatusCode::NOT_FOUND, "the board has no 3D part") };
        let project = query.get("project").map(|p| format!("&project={p}")).unwrap_or_default();
        return axum::response::Redirect::temporary(&format!("/export/{model}.step?download=1{project}")).into_response();
    }
    let interop = if name == format!("{stem}.kicad_pcb") { Some(de_pcb::kicad::export(&v, &b)) } else { de_pcb::idf::files(&v, &b).into_iter().find(|(n, _)| *n == name).map(|(_, s)| s) };
    if let Some(body) = interop {
        let disposition = if query.contains_key("download") { format!("attachment; filename=\"{name}\"") } else { "inline".into() };
        return ([(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, disposition)], body).into_response();
    }
    let body = match name.as_str() {
        "bom.csv" => outputs::bom(&v, &b, format),
        "positions.csv" => outputs::pick_and_place(&v, format),
        "README.txt" => outputs::readme(&v, &b, query.get("fab").and_then(|f| fabs::fab(f))),
        _ => match de_pcb::gerber::files(&v).into_iter().find(|(n, _)| *n == name) {
            Some((_, s)) => s,
            None => return fail(StatusCode::NOT_FOUND, format!("no file {name}")),
        },
    };
    let kind = if name.ends_with(".csv") { "text/csv; charset=utf-8" } else { "text/plain; charset=utf-8" };
    let disposition = if query.contains_key("download") { format!("attachment; filename=\"{name}\"") } else { "inline".into() };
    ([(header::CONTENT_TYPE, kind.to_string()), (header::CONTENT_DISPOSITION, disposition)], body).into_response()
}

pub async fn zip(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (b, v) = match board(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let fab = query.get("fab").filter(|f| !f.is_empty()).cloned();
    let result = tokio::task::spawn_blocking(move || {
        let files = outputs::package(&v, &b, fab.as_deref());
        let name = format!("{}-rev{}{}.zip", de_pcb::gerber::file_stem(&v.name), de_pcb::gerber::file_stem(&v.revision), fab.map(|f| format!("-{f}")).unwrap_or_default());
        (name, outputs::zip(&files))
    })
    .await;
    match result {
        Ok((name, bytes)) => ([(header::CONTENT_TYPE, "application/zip".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], bytes).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Files for other EDA and MCAD tools: KiCad board, IDF 3.0 board and library, the 3D board as STEP.
fn interop_names(v: &BoardView) -> Vec<String> {
    let stem = de_pcb::gerber::file_stem(&v.name);
    vec![format!("{stem}.kicad_pcb"), format!("{stem}.emn"), format!("{stem}.emp"), format!("{stem}.step")]
}

/// API keys a person may add for live distributor prices. Only which are set is ever returned.
const KEYS: [(&str, &str); 6] = [
    ("digikey", "Digi-Key client ID and secret (developer.digikey.com)"),
    ("mouser", "Mouser search API key (mouser.com/api-search)"),
    ("nexar", "Nexar / Octopart client ID and secret (nexar.com)"),
    ("lcsc", "LCSC open API key and secret (lcsc.com/docs/openapi)"),
    ("element14", "element14 / Farnell API key (partner.element14.com)"),
    ("tme", "TME API token (developers.tme.eu)"),
];

fn settings_path(server: &Server) -> std::path::PathBuf {
    server.dir.join("pcb-settings.json")
}

fn read_settings(server: &Server) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(settings_path(server)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn settings_view(server: &Server) -> Value {
    let s = read_settings(server);
    let keys: Vec<Value> = KEYS.iter().map(|(k, label)| json!({ "id": k, "label": label, "set": s.get(*k).and_then(|v| v.as_str()).is_some_and(|v| !v.is_empty()) })).collect();
    json!({ "keys": keys, "live_prices": false, "note": "Keys stay in this computer's projects folder (pcb-settings.json), never in a project or its sync. Live price lookup with them is not built yet; the distributor links work without any key." })
}

pub async fn settings_get(State(server): State<Arc<Server>>) -> impl IntoResponse {
    axum::Json(settings_view(&server))
}

pub async fn settings_post(State(server): State<Arc<Server>>, axum::Json(body): axum::Json<HashMap<String, String>>) -> axum::response::Response {
    let mut s = read_settings(&server);
    for (k, v) in body {
        if !KEYS.iter().any(|(id, _)| *id == k) {
            return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("unknown key {k}"));
        }
        if v.trim().is_empty() {
            s.remove(&k);
        } else {
            s.insert(k, json!(v.trim()));
        }
    }
    if let Err(e) = std::fs::write(settings_path(&server), serde_json::to_string_pretty(&s).unwrap_or_default()) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    axum::Json(settings_view(&server)).into_response()
}

/// DRC errors as check findings (`./forma check` reports boards with everything else).
pub fn findings(project: &de_core::Project) -> Vec<Value> {
    let mut out = Vec::new();
    for (id, b) in de_pcb::boards(project.world()) {
        let Some(v) = de_pcb::view(project.world(), id) else { continue };
        for x in v.violations.iter().filter(|x| x.severity == "error") {
            out.push(json!({ "code": "PCB_DRC", "severity": "error", "entities": [id], "message": format!("{}: {} ({})", b.name, x.message, x.code), "fix": null }));
        }
    }
    out
}
