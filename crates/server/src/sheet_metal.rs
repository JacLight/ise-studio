//! Sheet-metal flat patterns over HTTP.
//!
//! - `GET /sheet-metal/project`: every sheet-metal part in the project (shape or
//!   definition id, part, material, thickness, flat size, bends, findings).
//! - `GET /sheet-metal/<shape or instance id>?part=<name>&format=json|dxf|svg|csv`:
//!   one part's flat pattern: the bend table, flat size, cut length and checks
//!   (JSON; `loops=1` adds the blank's outlines), the laser's DXF with bend
//!   lines, an SVG to look at, or the bend table as CSV.

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{StableId, json};
use de_shape::{ShapeProgram, ShapeSolid, sheet_metal as sheet};
use std::collections::HashMap;
use std::sync::Arc;

pub async fn pattern(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    if id == "project" {
        return axum::Json(list(&app.project.lock().unwrap())).into_response();
    }
    // An instance shows its definition in its configuration; a shape shows itself.
    let program = {
        let project = app.project.lock().unwrap();
        let Some(sid) = serde_json::from_value::<StableId>(json!(id)).ok() else { return fail(StatusCode::NOT_FOUND, format!("{id} is not an id")) };
        let found = match project.get::<de_assembly::Instance>(sid) {
            Some(i) => project.entity(i.of).and_then(|e| match (&i.config, i.params.is_empty()) {
                (Some(c), true) => de_shape::configs::configured_program(project.world(), e, c).ok(),
                (None, true) => de_shape::resolved_program(project.world(), e),
                (c, false) => de_shape::configs::variant_program(project.world(), e, c.as_deref(), &i.params, &|_, _| None).ok(),
            }),
            None => project.entity(sid).and_then(|e| de_shape::resolved_program(project.world(), e)),
        };
        match found {
            Some(p) => p,
            None => return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape or an instance of one")),
        }
    };
    let part = query.get("part").cloned();
    let tolerance = query.get("tolerance").and_then(|t| t.parse::<f64>().ok()).filter(|t| (0.0001..=1.0).contains(t)).unwrap_or(0.005);
    let result = tokio::task::spawn_blocking(move || sheet::flat_pattern(&program, part.as_deref(), tolerance)).await.unwrap_or_else(|e| Err(e.to_string()));
    let f = match result {
        Ok(f) => f,
        Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    let file = |ext: &str| format!("attachment; filename=\"{}.{ext}\"", de_shape::fabricate::file_name(&f.part));
    match query.get("format").map(String::as_str).unwrap_or("json") {
        "dxf" => (StatusCode::OK, [(header::CONTENT_TYPE, "image/vnd.dxf".to_string()), (header::CONTENT_DISPOSITION, file("dxf"))], sheet::dxf(&f)).into_response(),
        "svg" => (StatusCode::OK, [(header::CONTENT_TYPE, "image/svg+xml".to_string())], sheet::svg(&f)).into_response(),
        "csv" => (StatusCode::OK, [(header::CONTENT_TYPE, "text/csv".to_string()), (header::CONTENT_DISPOSITION, file("csv"))], sheet::bend_table_csv(&f)).into_response(),
        "json" => {
            let mut v = serde_json::to_value(&f).unwrap_or_default();
            if query.get("loops").is_none_or(|l| l != "1") {
                v.as_object_mut().map(|o| o.remove("loops"));
            }
            axum::Json(v).into_response()
        }
        other => fail(StatusCode::BAD_REQUEST, format!("format is json, dxf, svg or csv, not {other}")),
    }
}

/// Every sheet-metal part the project holds, from the shapes as evaluated.
fn list(project: &de_core::Project) -> serde_json::Value {
    let mut out = Vec::new();
    for id in project.entity_ids() {
        let (Some(program), Some(solid)) = (project.get::<ShapeProgram>(id), project.get::<ShapeSolid>(id)) else { continue };
        for (part, data) in &solid.stats.sheet_metal {
            let count = |sev: &str| data["checks"].as_array().map(|c| c.iter().filter(|x| x["severity"] == sev).count()).unwrap_or(0);
            out.push(json!({
                "id": id,
                "shape": program.name,
                "part": if part.is_empty() { &program.name } else { part },
                "material": data["material"],
                "thickness": data["thickness"],
                "flat": data["flat"]["size"],
                "bends": data["bends"].as_array().map(Vec::len).unwrap_or(0),
                "errors": count("error"),
                "warnings": count("warning"),
            }));
        }
    }
    json!(out)
}
