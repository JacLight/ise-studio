//! Bills of materials and requests for quote over HTTP. Everything is made
//! here and handed to the person; nothing is sent anywhere.
//!
//! - `GET /bom/<id|project>?config=…&qty=…&breaks=1,10,100&format=json|csv&view=flat|indented`
//! - `GET /bom/<id|project>/file?line=<key>&format=step|stl|3mf|dxf|pdf` — one line's file
//! - `GET /rfq/<rfq id>.zip` — every supplier's package, a folder each;
//!   `?supplier=<id>` — one supplier's package; `/rfq/<rfq id>` — the plan as
//!   JSON (each supplier's lines and a mailto: draft)
//! - `GET /rates` — process rates in force (the project's or the defaults)

use crate::{App, Server, bad_request, today};
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{Project, StableId, Value, json};
use de_shape::fabricate;
use de_sourcing::{
    Rfq, Scope, Supplier,
    bom::{self, Bom},
    package,
};
use std::collections::HashMap;
use std::sync::Arc;

type Response = axum::response::Response;

fn app_of(server: &Server, q: &HashMap<String, String>) -> Result<(String, Arc<App>), Response> {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    server.project(Some(&name)).map(|a| (name, a)).map_err(bad_request)
}

fn fail(code: StatusCode, message: String) -> Response {
    (code, [(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string())], message).into_response()
}

fn file(name: &str, mime: &str, bytes: Vec<u8>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, mime.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], bytes).into_response()
}

fn scope_of(id: &str) -> Result<Scope, String> {
    if id == "project" {
        return Ok(Scope::Project);
    }
    serde_json::from_value::<StableId>(json!(id)).map(Scope::Entity).map_err(|_| format!("“{id}” is not an id (or “project”)"))
}

fn options(q: &HashMap<String, String>) -> Result<bom::Options, String> {
    let mut o = bom::Options { config: q.get("config").filter(|c| !c.is_empty()).cloned(), ..Default::default() };
    if let Some(n) = q.get("qty").filter(|n| !n.is_empty()) {
        o.quantity = n.parse().ok().filter(|n| *n >= 1 && *n <= 1_000_000).ok_or_else(|| format!("qty is a whole number of finished units from 1, not “{n}”"))?;
    }
    if let Some(b) = q.get("breaks").filter(|b| !b.is_empty()) {
        o.breaks = b.split(',').map(|x| x.trim().parse::<u32>().ok().filter(|n| *n >= 1)).collect::<Option<Vec<_>>>().ok_or("breaks are whole numbers, comma separated (1,10,100)")?;
    }
    Ok(o)
}

/// The project's title, for documents.
fn title(project: &Project, name: &str) -> String {
    project
        .entity_ids()
        .into_iter()
        .find_map(|id| project.get::<de_core::ProjectInfo>(id).map(|p| p.title.clone()))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| crate::pretty_name(name))
}

fn compute(project: &Project, scope: Scope, options: &bom::Options) -> Result<Bom, String> {
    bom::compute(project.world(), scope, options, &[])
}

/// `GET /bom/<id>`: the bill of materials, as JSON (with the process rates
/// in force) or CSV (`view=indented` for the tree).
pub async fn bom(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (_, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (scope, options) = match (scope_of(&id), options(&q)) {
        (Ok(s), Ok(o)) => (s, o),
        (Err(e), _) | (_, Err(e)) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let format = q.get("format").cloned().unwrap_or_else(|| "json".into());
    let view = q.get("view").cloned().unwrap_or_else(|| "flat".into());
    let result = tokio::task::spawn_blocking(move || {
        let project = app.project.lock().unwrap();
        let b = compute(&project, scope, &options)?;
        let rates = de_sourcing::rates::rates_of(project.world());
        Ok::<_, String>((b, rates))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    let (b, rates) = match result {
        Ok(x) => x,
        Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    match format.as_str() {
        "csv" => {
            let (text, suffix) = if view == "indented" { (bom::indented_csv(&b), "indented BOM") } else { (bom::flat_csv(&b), "BOM") };
            file(&format!("{} {suffix}.csv", fabricate::file_name(&b.name)), "text/csv; charset=utf-8", text.into_bytes())
        }
        "json" => {
            let table: Vec<Value> = rates.table().into_iter().map(|(p, r, default)| json!({ "process": p, "rate": r, "default": default })).collect();
            let mut v = serde_json::to_value(&b).unwrap();
            v["rates"] = json!(table);
            axum::Json(v).into_response()
        }
        other => fail(StatusCode::BAD_REQUEST, format!("format is json or csv, not {other}")),
    }
}

/// `GET /bom/<id>/file?line=…&format=…`: one Make line's file.
pub async fn line_file(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (scope, options) = match (scope_of(&id), options(&q)) {
        (Ok(s), Ok(o)) => (s, o),
        (Err(e), _) | (_, Err(e)) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let Some(key) = q.get("line").cloned() else { return fail(StatusCode::BAD_REQUEST, "say which line (line=make:…)".into()) };
    let ext: &'static str = match q.get("format").map(String::as_str) {
        Some("step") => "step",
        Some("stl") => "stl",
        Some("3mf") => "3mf",
        Some("dxf") => "dxf",
        Some("pdf") => "pdf",
        other => return fail(StatusCode::BAD_REQUEST, format!("format is step, stl, 3mf, dxf or pdf, not {}", other.unwrap_or("nothing"))),
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(String, &'static str, Vec<u8>), String> {
        let (b, programs, context) = {
            let project = app.project.lock().unwrap();
            let b = compute(&project, scope, &options)?;
            let programs = bom::line_programs(project.world(), &b);
            (b, programs, package::Context { project: title(&project, &name), date: today(), from: String::new() })
        };
        let line = b.make_line(&key).ok_or_else(|| format!("no made part has line {key} (bought items have no files)"))?;
        let number = format!("{}-{:02}", fabricate::file_name(&context.project).to_uppercase(), line.item);
        let mut files = package::Files::new(&programs, 0.005, context);
        let bytes = files.file(&key, ext, &line.name, line.material.as_deref(), &number)?;
        let mime = match ext {
            "step" => "model/step",
            "stl" => "model/stl",
            "3mf" => "model/3mf",
            "dxf" => "image/vnd.dxf",
            _ => "application/pdf",
        };
        Ok((format!("{}.{ext}", fabricate::file_name(&line.name)), mime, bytes))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok((n, mime, bytes)) => file(&n, mime, bytes),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// An RFQ, its BOM and its suppliers, read under the lock.
fn rfq_parts(project: &Project, rfq: StableId) -> Result<(Rfq, Bom, Vec<(StableId, Supplier)>), String> {
    let r = project.get::<Rfq>(rfq).cloned().ok_or_else(|| format!("{rfq} is not an RFQ"))?;
    let scope = r.scope.map(Scope::Entity).unwrap_or(Scope::Project);
    let b = compute(project, scope, &bom::Options { config: r.config.clone(), ..Default::default() })?;
    let suppliers = r.suppliers.iter().filter_map(|s| project.get::<Supplier>(*s).map(|x| (*s, x.clone()))).collect();
    Ok((r, b, suppliers))
}

/// `GET /rfq/<id>[.zip]`: the RFQ package (zip) or its plan (JSON).
pub async fn rfq(State(server): State<Arc<Server>>, Path(stem): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (id, zip) = match stem.strip_suffix(".zip") {
        Some(s) => (s.to_string(), true),
        None => (stem.clone(), false),
    };
    let Ok(rfq_id) = serde_json::from_value::<StableId>(json!(id)) else { return fail(StatusCode::BAD_REQUEST, format!("“{id}” is not an RFQ id")) };
    let only: Option<StableId> = match q.get("supplier").filter(|s| !s.is_empty()) {
        Some(s) => match serde_json::from_value(json!(s)) {
            Ok(s) => Some(s),
            Err(_) => return fail(StatusCode::BAD_REQUEST, format!("“{s}” is not a supplier id")),
        },
        None => None,
    };
    let from = q.get("from").cloned().unwrap_or_default();
    let result = tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (r, b, suppliers, programs, context) = {
            let project = app.project.lock().unwrap();
            let (r, b, suppliers) = rfq_parts(&project, rfq_id)?;
            let programs = bom::line_programs(project.world(), &b);
            (r, b, suppliers, programs, package::Context { project: title(&project, &name), date: today(), from })
        };
        let suppliers: Vec<(StableId, Supplier)> = match only {
            Some(s) => suppliers.into_iter().filter(|(id, _)| *id == s).collect(),
            None => suppliers,
        };
        if suppliers.is_empty() {
            return Err(if only.is_some() { "that supplier is not on this RFQ".into() } else { "the RFQ has no suppliers yet: add some, then make the package".into() });
        }
        if !zip {
            let plan: Vec<Value> = suppliers
                .iter()
                .map(|(id, s)| {
                    let lines = package::plan(&r, &b, s);
                    let files: Vec<String> = lines.iter().flat_map(|l| l.files.iter().map(|e| package::file_name(l, e)).collect::<Vec<_>>()).collect();
                    json!({
                        "supplier": id,
                        "name": s.name,
                        "lines": lines.iter().map(|l| json!({ "line": l.key, "item": l.item, "name": l.name, "qty": l.qty, "missing": l.missing })).collect::<Vec<_>>(),
                        "files": files,
                        "mailto": package::mailto(&r, s, lines.len(), &context),
                        "letter": package::cover_letter(&r, s, &lines, &context),
                    })
                })
                .collect();
            return Ok(axum::Json(json!({ "rfq": rfq_id, "suppliers": plan })).into_response());
        }
        let mut files = package::Files::new(&programs, 0.005, context);
        let (bytes, file_name) = if let [(_, s)] = &suppliers[..]
            && only.is_some()
        {
            let (all, _) = package::supplier_package(&r, &b, s, &mut files, "");
            (fabricate::zip(&all), format!("{} - {}.zip", fabricate::file_name(&r.name), fabricate::file_name(&s.name)))
        } else {
            (fabricate::zip(&package::rfq_package(&r, &b, &suppliers, &mut files)), format!("{} RFQ.zip", fabricate::file_name(&r.name)))
        };
        Ok(file(&file_name, "application/zip", bytes))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(r) => r,
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// `GET /rates`: the process rates in force.
pub async fn rates(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (_, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let project = app.project.lock().unwrap();
    let r = de_sourcing::rates::rates_of(project.world());
    let table: Vec<Value> = r.table().into_iter().map(|(p, rate, default)| json!({ "process": p, "rate": rate, "default": default })).collect();
    axum::Json(json!({ "currency": r.currency(), "processes": table })).into_response()
}
