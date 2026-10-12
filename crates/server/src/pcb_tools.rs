//! Fab utilities around a board: calculators, panelization, the Gerber
//! viewer and the board as glTF. Nothing here changes a project; the panel
//! and the viewer are worked out from the request each time.
//!
//! - `GET  /pcb/calc`                         every calculator with its inputs
//! - `GET  /pcb/calc/<id>?key=value…`         one calculator's answer
//! - `GET  /pcb/<id>/panel?rows=&cols=&…`     the panel's layout and checks (settings: `panel::Spec`)
//! - `GET  /pcb/<id>/panel.zip?…&fab=`        its Gerbers, drill, positions, BOM and readme
//! - `GET  /pcb/<id>/gerbview`                the board's own package as the Gerber viewer reads it
//! - `POST /pcb/gerbview`                     `{ files: [{ name, data (base64) }], against?: [...], px? }`: layers (and a comparison)
//! - `POST /pcb/gerbview/pdf`                 `{ files, looks: [{ file, colour }], background?, mirror? }`: the layers as a PDF
//! - `GET  /pcb/<id>/board.glb`               the populated board as binary glTF

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use base64::Engine;
use de_core::{StableId, Value, json};
use de_pcb::{Board, BoardView, gerbview, panel};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

fn fail(code: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (code, message.into()).into_response()
}

/// The board, its view and its items, or why not.
fn board(server: &Server, query: &HashMap<String, String>, id: &str) -> Result<(Board, Arc<BoardView>, de_pcb::Items), axum::response::Response> {
    let app = server.project(query.get("project")).map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let project = app.project.lock().unwrap();
    let id: StableId = serde_json::from_value(json!(id)).map_err(|_| fail(StatusCode::NOT_FOUND, format!("{id} is not a board")))?;
    let b = project.get::<Board>(id).cloned().ok_or_else(|| fail(StatusCode::NOT_FOUND, format!("{id} is not a board")))?;
    let view = de_pcb::view(project.world(), id).ok_or_else(|| fail(StatusCode::UNPROCESSABLE_ENTITY, "the board is not worked out yet"))?;
    Ok((b, view, de_pcb::items(project.world(), id)))
}

pub async fn calc_list() -> impl IntoResponse {
    axum::Json(json!({ "calculators": de_pcb::calc::list() }))
}

pub async fn calc(Path(id): Path<String>, Query(query): Query<BTreeMap<String, String>>) -> axum::response::Response {
    let mut args = query;
    args.remove("project");
    match de_pcb::calc::run(&id, &args) {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

fn spec_of(query: &HashMap<String, String>) -> Result<panel::Spec, String> {
    panel::Spec::from_pairs(&query.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

pub async fn panel_show(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (b, v, _) = match board(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let spec = match spec_of(&query) {
        Ok(s) => s,
        Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    let fab = query.get("fab").cloned();
    let out = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let p = panel::layout(&v.outline, &v.cutouts, &spec)?;
        let files = panel::files(&v, &b, &p, fab.as_deref())?;
        let readback = panel::readback(&files);
        Ok(json!({ "panel": p, "board": { "outline": v.outline, "cutouts": v.cutouts, "bounds": v.bounds, "name": v.name }, "files": files.iter().map(|f| &f.0).collect::<Vec<_>>(), "readback": readback, "defaults": panel::Spec::default() }))
    })
    .await;
    match out {
        Ok(Ok(v)) => axum::Json(v).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn panel_zip(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (b, v, _) = match board(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let spec = match spec_of(&query) {
        Ok(s) => s,
        Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    let fab = query.get("fab").filter(|f| !f.is_empty()).cloned();
    let out = tokio::task::spawn_blocking(move || -> Result<(String, Vec<u8>), String> {
        let p = panel::layout(&v.outline, &v.cutouts, &spec)?;
        let files = panel::files(&v, &b, &p, fab.as_deref())?;
        let name = format!("{}-rev{}-panel-{}x{}{}.zip", de_pcb::gerber::file_stem(&v.name), de_pcb::gerber::file_stem(&v.revision), spec.rows, spec.cols, fab.map(|f| format!("-{f}")).unwrap_or_default());
        Ok((name, de_pcb::outputs::zip(&files)))
    })
    .await;
    match out {
        Ok(Ok((name, bytes))) => ([(header::CONTENT_TYPE, "application/zip".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], bytes).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn board_gerbview(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (_, v, _) = match board(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let out = tokio::task::spawn_blocking(move || {
        let files: Vec<(String, Vec<u8>)> = de_pcb::gerber::files(&v).into_iter().map(|(n, s)| (n, s.into_bytes())).collect();
        gerbview::read(&files)
    })
    .await;
    match out {
        Ok(set) => axum::Json(set).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(serde::Deserialize)]
pub struct Upload {
    name: String,
    /// Base64 bytes.
    data: String,
}

fn files_of(list: &[Upload]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    for f in list {
        let bytes = base64::engine::general_purpose::STANDARD.decode(f.data.trim()).map_err(|e| format!("{}: {e}", f.name))?;
        out.extend(gerbview::unpack(&f.name, &bytes)?);
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
pub struct ViewRequest {
    files: Vec<Upload>,
    #[serde(default)]
    against: Option<Vec<Upload>>,
    #[serde(default)]
    px: Option<f64>,
    /// Only the summary (no drawings): for the CLI.
    #[serde(default)]
    summary: bool,
}

pub async fn gerbview_post(axum::Json(req): axum::Json<ViewRequest>) -> axum::response::Response {
    let out = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let files = files_of(&req.files)?;
        let set = gerbview::read(&files);
        let mut v = if req.summary { gerbview::summary(&set) } else { serde_json::to_value(&set).map_err(|e| e.to_string())? };
        if let Some(against) = &req.against {
            let other = files_of(against)?;
            let diff = gerbview::compare(&other, &files, req.px.unwrap_or(0.05).clamp(0.005, 0.5));
            let diff = if req.summary { diff.into_iter().map(|d| json!({ "function": d.function, "before": d.before, "after": d.after, "added": d.added, "removed": d.removed, "holes_added": d.holes_added, "holes_removed": d.holes_removed, "same": d.same })).collect::<Vec<_>>() } else { diff.into_iter().map(|d| serde_json::to_value(d).unwrap()).collect() };
            v["compare"] = json!(diff);
            if !req.summary {
                v["before"] = serde_json::to_value(gerbview::read(&other)).map_err(|e| e.to_string())?;
            }
        }
        Ok(v)
    })
    .await;
    match out {
        Ok(Ok(v)) => axum::Json(v).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(serde::Deserialize)]
pub struct PdfRequest {
    files: Vec<Upload>,
    looks: Vec<gerbview::Look>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    mirror: bool,
}

pub async fn gerbview_pdf(axum::Json(req): axum::Json<PdfRequest>) -> axum::response::Response {
    let out = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let files = files_of(&req.files)?;
        gerbview::pdf(&files, &req.looks, req.background.as_deref().unwrap_or("#ffffff"), req.mirror)
    })
    .await;
    match out {
        Ok(Ok(bytes)) => ([(header::CONTENT_TYPE, "application/pdf".to_string()), (header::CONTENT_DISPOSITION, "attachment; filename=\"gerbers.pdf\"".to_string())], bytes).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn board_glb(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let (b, v, items) = match board(&server, &query, &id) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let name = format!("{}.glb", de_pcb::gerber::file_stem(&v.name));
    match tokio::task::spawn_blocking(move || de_pcb::glb::glb(&v, &b, &items)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "model/gltf-binary".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], bytes).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
