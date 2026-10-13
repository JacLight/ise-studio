//! Scans in the engine: point clouds and meshes added to a project
//! (`POST /scans/<file>`), checked, repaired and reworked into new scans,
//! reconstructed, fitted, sectioned, unwrapped, turned into parts, and
//! inspected against the CAD part they should match (`/scans/<id>/…`).
//!
//! A scan is kept by content in the folder's shared store (as imported
//! files are) and listed per project in `<project>.scans.json`, each entry
//! saying where it came from: a file, or an operation on another scan. That
//! is also the door for photogrammetry: COLMAP's fused point cloud or Apple
//! Object Capture's mesh arrive as a file like any scanner's, with `source`
//! naming what made them.

use crate::{Server, StableId, json};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use de_core::Value;
use de_scan::{Cloud, Mesh};
use de_shape::scans;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

/// One scan in a project's list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// "points" or "mesh".
    pub kind: String,
    /// The stored file's format (xyz, ply, pcd, e57, stl, obj, …).
    pub format: String,
    pub points: usize,
    #[serde(default)]
    pub triangles: usize,
    pub bounds: [f64; 6],
    /// What made it: "file", a scanner or photogrammetry tool's name, or the
    /// operation that derived it ("repair", "decimate", "reconstruct", …).
    pub source: String,
    /// The scan it was derived from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub added: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn project_name(server: &Server, query: &HashMap<String, String>) -> String {
    query.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone())
}

fn index_path(server: &Server, project: &str) -> std::path::PathBuf {
    server.dir.join(format!("{project}.scans.json"))
}

pub fn read_index(server: &Server, project: &str) -> Vec<Entry> {
    std::fs::read_to_string(index_path(server, project)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_index(server: &Server, project: &str, list: &[Entry]) -> Result<(), String> {
    std::fs::write(index_path(server, project), serde_json::to_string_pretty(list).unwrap()).map_err(|e| format!("could not save the list of scans: {e}"))
}

/// Stores scan bytes and lists them in the project; the entry.
fn add(server: &Server, project: &str, bytes: &[u8], ext: &str, name: &str, source: &str, parent: Option<&str>) -> Result<Entry, String> {
    let data = scans::read(bytes, ext)?;
    let id = de_shape::assets::store_scan(bytes, ext)?;
    let (lo, hi) = data.cloud.bounds();
    let entry = Entry {
        id: id.clone(),
        name: name.to_string(),
        kind: if data.mesh.is_some() { "mesh" } else { "points" }.into(),
        format: ext.into(),
        points: data.cloud.len(),
        triangles: data.mesh.as_ref().map(|m| m.triangles.len()).unwrap_or(0),
        bounds: [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]],
        source: source.into(),
        parent: parent.map(str::to_string),
        added: now_ms(),
    };
    let mut list = read_index(server, project);
    list.retain(|e| e.id != id);
    list.push(entry.clone());
    write_index(server, project, &list)?;
    Ok(entry)
}

/// A mesh as a binary PLY (the format derived meshes are kept in).
fn mesh_bytes(m: &Mesh, name: &str) -> Vec<u8> {
    de_shape::formats::mesh::write_ply(&[de_shape::formats::MeshBody { name: name.into(), positions: m.flat_positions(), triangles: m.flat_triangles(), look: None, normals: vec![] }])
}

fn find(server: &Server, project: &str, id: &str) -> Result<Entry, String> {
    read_index(server, project).into_iter().find(|e| e.id == id || e.name == id || (id.len() >= 8 && e.id.starts_with(id))).ok_or_else(|| format!("no scan {id} in this project"))
}

fn fail(code: StatusCode, message: String) -> axum::response::Response {
    (code, message).into_response()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(e.to_string()))
}

fn reply(r: Result<Value, String>) -> axum::response::Response {
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// `POST /scans/<file name>?project=…[&name=…][&source=colmap]` with the
/// file as the body: a point cloud or mesh added to the project.
pub async fn upload(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    if let Err(e) = server.project(query.get("project")) {
        return fail(StatusCode::BAD_REQUEST, e);
    }
    let Some(ext) = scans::ext_of(&body, Some(&file)) else {
        return fail(StatusCode::BAD_REQUEST, format!("Forma reads scans as {}", scans::FORMATS));
    };
    let project = project_name(&server, &query);
    let name = query.get("name").cloned().unwrap_or_else(|| file.rsplit_once('.').map(|(n, _)| n.to_string()).unwrap_or(file.clone()));
    let source = query.get("source").cloned().unwrap_or_else(|| "file".into());
    let s = server.clone();
    reply(blocking(move || add(&s, &project, &body, ext, &name, &source, None).map(|e| json!(e))).await)
}

/// `GET /scans?project=…`: the project's scans.
pub async fn list(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    axum::Json(read_index(&server, &project_name(&server, &query))).into_response()
}

/// `GET /scans/<id>/view?max=200000`: points (and triangles, for a mesh of
/// up to `maxTriangles`) for drawing: flat arrays, every `step`-th point.
pub async fn view(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let project = project_name(&server, &query);
    let max = query.get("max").and_then(|v| v.parse::<usize>().ok()).unwrap_or(200_000).max(1);
    let max_tris = query.get("maxTriangles").and_then(|v| v.parse::<usize>().ok()).unwrap_or(600_000);
    let s = server.clone();
    reply(
        blocking(move || {
            let e = find(&s, &project, &id)?;
            let d = scans::load(&e.id)?;
            let step = d.cloud.len().div_ceil(max).max(1);
            let pts: Vec<f32> = d.cloud.positions.iter().step_by(step).flat_map(|p| p.map(|v| v as f32)).collect();
            let colors: Vec<u8> = if d.cloud.colors.is_empty() { vec![] } else { d.cloud.colors.iter().step_by(step).flat_map(|c| *c).collect() };
            let mut v = json!({ "id": e.id, "kind": e.kind, "step": step, "count": d.cloud.len(), "positions": pts, "colors": colors });
            if let Some(m) = d.mesh.as_ref().filter(|m| m.triangles.len() <= max_tris) {
                v["mesh"] = json!({ "positions": m.positions.iter().flat_map(|p| p.map(|x| x as f32)).collect::<Vec<f32>>(), "triangles": m.flat_triangles() });
            }
            Ok(v)
        })
        .await,
    )
}

/// `POST /scans/<id>/query/<op>` with JSON options: info, check, fit,
/// detect, section, curvature, segment, boundaries, fitCurve, surface (see
/// `de_shape::scans::query`).
pub async fn query_op(State(server): State<Arc<Server>>, Path((id, op)): Path<(String, String)>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let project = project_name(&server, &query);
    let o: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let s = server.clone();
    reply(blocking(move || scans::query(&op, &find(&s, &project, &id)?.id, &o)).await)
}

/// `POST /scans/<id>/mesh/<op>` (repair, decimate, smooth, remesh,
/// fillHoles, harmonize, flip, largest, removeComponents, split) or
/// `/points/<op>` (downsample, outliers, normals) or `/reconstruct`: the
/// result kept as a new scan; the reply is its entry and the report.
pub async fn derive(State(server): State<Arc<Server>>, Path((id, what, op)): Path<(String, String, String)>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let project = project_name(&server, &query);
    let o: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let s = server.clone();
    reply(
        blocking(move || {
            let e = find(&s, &project, &id)?;
            let d = scans::load(&e.id)?;
            let label = |op: &str| o.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{} ({op})", e.name));
            match what.as_str() {
                "mesh" if op == "split" => {
                    let m = d.mesh.as_ref().ok_or("this scan is points only")?;
                    let pieces = m.split_components();
                    let mut out = Vec::new();
                    for (i, p) in pieces.iter().enumerate().take(50) {
                        let name = format!("{} (piece {})", e.name, i + 1);
                        out.push(add(&s, &project, &mesh_bytes(p, &name), "ply", &name, "split", Some(&e.id))?);
                    }
                    Ok(json!({ "scans": out, "pieces": pieces.len() }))
                }
                "mesh" if op == "merge" => {
                    let mut m = d.mesh.clone().ok_or("this scan is points only")?;
                    for other in o.get("with").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                        let oe = find(&s, &project, other)?;
                        m.merge(&scans::mesh_of(&oe.id)?);
                    }
                    let name = label("merged");
                    let entry = add(&s, &project, &mesh_bytes(&m, &name), "ply", &name, "merge", Some(&e.id))?;
                    Ok(json!({ "scan": entry }))
                }
                "mesh" => {
                    let m = d.mesh.as_ref().ok_or("this scan is points only: reconstruct a surface from it first")?;
                    let (out, report) = scans::mesh_op(m, &op, &o)?;
                    let name = label(&op);
                    let entry = add(&s, &project, &mesh_bytes(&out, &name), "ply", &name, &op, Some(&e.id))?;
                    Ok(json!({ "scan": entry, "report": report }))
                }
                "points" => {
                    let (out, report) = scans::cloud_op(&d.cloud, &op, &o)?;
                    let name = label(&op);
                    let entry = add(&s, &project, &de_scan::points::write_ply(&out), "ply", &name, &op, Some(&e.id))?;
                    Ok(json!({ "scan": entry, "report": report }))
                }
                "surface" if op == "reconstruct" => {
                    let c = scans::region(&d.cloud, &o);
                    let (m, report) = scans::reconstruct(&c, &o)?;
                    let name = label("surface");
                    let entry = add(&s, &project, &mesh_bytes(&m, &name), "ply", &name, "reconstruct", Some(&e.id))?;
                    Ok(json!({ "scan": entry, "report": report }))
                }
                "surface" if op == "sample" => {
                    // Mesh to points (FreeCAD's Points_Convert).
                    let m = d.mesh.as_ref().ok_or("this scan is already points")?;
                    let c = de_scan::points::sample_mesh(m, o.get("count").and_then(Value::as_u64).unwrap_or(50_000) as usize);
                    let name = label("points");
                    let entry = add(&s, &project, &de_scan::points::write_ply(&c), "ply", &name, "sample", Some(&e.id))?;
                    Ok(json!({ "scan": entry }))
                }
                _ => Err(format!("unknown scan operation {what}/{op}")),
            }
        })
        .await,
    )
}

/// The CAD part a scan is compared with, as a mesh in project coordinates.
fn target_mesh(app: &crate::App, target: &str, part: Option<&str>, tolerance: f64) -> Result<(StableId, String, Mesh), String> {
    let project = app.project.lock().unwrap();
    let world = project.world();
    let id = serde_json::from_value::<StableId>(json!(target)).ok().filter(|s| project.entity(*s).is_some()).or_else(|| {
        project.entity_ids().into_iter().find(|e| {
            project.get::<de_assembly::Instance>(*e).is_some_and(|i| i.name == target) || project.get::<de_shape::ShapeProgram>(*e).is_some_and(|p| p.name == target)
        })
    });
    let id = id.ok_or_else(|| format!("nothing called {target}"))?;
    let mut view = de_intent::geometry::part(world, id)?;
    if let Some(p) = part {
        view.part = Some(p.to_string());
    }
    let name = project.get::<de_shape::ShapeProgram>(id).map(|p| p.name.clone()).or_else(|| project.get::<de_assembly::Instance>(id).map(|i| i.name.clone())).unwrap_or_else(|| target.to_string());
    let solid = view.solid(tolerance);
    Ok((id, name, scans::from_manifold(&solid)))
}

/// Inspection options.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectOptions {
    /// The part (id or name) the scan should match.
    target: String,
    #[serde(default)]
    part: Option<String>,
    /// ± mm.
    tolerance: f64,
    /// Share of points that must be within tolerance (0…1, default all).
    #[serde(default)]
    required: Option<f64>,
    /// "none" (as placed), "icp" (fine best fit), "best" (coarse then fine).
    #[serde(default)]
    align: Option<String>,
    /// Values for at most this many points (every step-th, as /view sends them).
    #[serde(default)]
    max: Option<usize>,
}

/// `POST /scans/<id>/inspect` with { target, part?, tolerance, required?,
/// align?, max? }: each point's deviation from the part (signed, mm), the
/// statistics, pass/fail, and the values to colour the points by.
pub async fn inspect(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let project = project_name(&server, &query);
    let o: InspectOptions = match serde_json::from_slice(&body) {
        Ok(o) => o,
        Err(e) => return fail(StatusCode::BAD_REQUEST, format!("inspect: {e} (send {{ \"target\": part, \"tolerance\": mm }})")),
    };
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let s = server.clone();
    reply(
        blocking(move || {
            if !(o.tolerance > 0.0) {
                return Err("inspect: the tolerance must be positive".into());
            }
            let e = find(&s, &project, &id)?;
            let d = scans::load(&e.id)?;
            let (target_id, target_name, mesh) = target_mesh(&app, &o.target, o.part.as_deref(), (o.tolerance / 10.0).clamp(0.0005, 0.05))?;
            let bvh = de_scan::spatial::TriBvh::new(&mesh);
            let r = deviation_report(&d.cloud, &bvh, o.tolerance, o.required.unwrap_or(1.0), o.align.as_deref().unwrap_or("none"), o.max.unwrap_or(200_000))?;
            let mut v = r;
            v["scan"] = json!(e);
            v["target"] = json!({ "id": target_id, "name": target_name, "part": o.part });
            Ok(v)
        })
        .await,
    )
}

/// Deviation of a cloud from a surface, optionally aligned first, as the reply.
pub fn deviation_report(cloud: &Cloud, bvh: &de_scan::spatial::TriBvh, tolerance: f64, required: f64, align: &str, max: usize) -> Result<Value, String> {
    use de_scan::inspect;
    let (points, alignment) = match align {
        "none" | "" => (cloud.positions.clone(), None),
        "icp" | "best" => {
            let a = inspect::align(&cloud.positions, bvh, align == "best", 80);
            (cloud.positions.iter().map(|p| inspect::apply(&a.transform, *p)).collect(), Some(a))
        }
        other => return Err(format!("align is none, icp or best, not {other}")),
    };
    let d = inspect::deviation(&points, bvh, tolerance, required);
    let step = points.len().div_ceil(max.max(1)).max(1);
    let values: Vec<f32> = d.values.iter().step_by(step).map(|v| *v as f32).collect();
    // The worst places, for labels in the view.
    let mut order: Vec<usize> = (0..d.values.len()).collect();
    order.sort_by(|a, b| d.values[*b].abs().total_cmp(&d.values[*a].abs()));
    let worst: Vec<Value> = order.iter().take(5).map(|&i| json!({ "at": points[i], "deviation": d.values[i] })).collect();
    Ok(json!({
        "stats": d.stats,
        "tolerance": d.tolerance,
        "within": d.within,
        "over": d.over,
        "under": d.under,
        "passed": d.passed,
        "required": required,
        "histogram": d.histogram,
        "alignment": alignment,
        "step": step,
        "values": values,
        "worst": worst,
    }))
}

/// `POST /scans/<id>/shape` with { name?, repair?, decimate?, smooth?, depth? }:
/// the scan as a part. A mesh becomes `importMesh(sha, {…})`, points a
/// reconstructed surface, both as a program the AI or a person can edit.
pub async fn to_shape(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let project = project_name(&server, &query);
    let o: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let s = server.clone();
    reply(
        blocking(move || {
            let e = find(&s, &project, &id)?;
            let name = o.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| e.name.clone());
            let mut opts = serde_json::Map::new();
            for k in ["repair", "remesh", "decimate", "smooth", "refine", "depth"] {
                if let Some(v) = o.get(k) {
                    opts.insert(k.into(), v.clone());
                }
            }
            let options = if opts.is_empty() { String::new() } else { format!(", {}", Value::Object(opts.clone())) };
            let source = if e.kind == "mesh" {
                // Check it closes now, so the part does not land broken.
                if !opts.contains_key("repair") {
                    let m = scans::mesh_of(&e.id)?;
                    scans::to_manifold(&m).map_err(|err| format!("{err} — ask for repair: true"))?;
                }
                format!("// The scan “{}” ({} triangles) as a part: a mesh, faceted in STEP.\n// Fit primitives to it (fitScan) to rebuild exact geometry.\nreturn importMesh(\"{}\"{options});", e.name, e.triangles, e.id)
            } else {
                format!("// A closed surface reconstructed from the scan “{}” ({} points).\nreturn reconstruct(\"{}\"{options});", e.name, e.points, e.id)
            };
            let out = app.commit_here("shape.create", json!({ "name": name, "description": format!("Made from the scan {}.", e.name), "source": source, "params": [] }))?;
            Ok(json!({ "id": out["output"]["id"], "name": name, "source": source }))
        })
        .await,
    )
}

/// `POST /scans/<id>/unwrap` with { box | near+within | segment, sketch: true, name }:
/// the surface flattened; with `sketch`, its outline becomes a sketch.
pub async fn unwrap(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let project = project_name(&server, &query);
    let o: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let s = server.clone();
    reply(
        blocking(move || {
            let e = find(&s, &project, &id)?;
            let mut v = scans::query("unwrap", &e.id, &o)?;
            let outlines: Vec<Vec<[f64; 2]>> = serde_json::from_value(v["outlines"].clone()).unwrap_or_default();
            v["svg"] = json!(outline_svg(&outlines));
            if o.get("sketch").and_then(Value::as_bool) == Some(true) {
                let dxf = outline_dxf(&outlines);
                let (geometry, _, _) = de_shape::formats::read_sketch(dxf.as_bytes(), de_shape::formats::Kind::Dxf)?;
                let name = o.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{} flat", e.name));
                let out = app.commit_here("sketch.create", json!({ "name": name, "plane": { "name": "XY" }, "geometry": geometry }))?;
                v["sketch"] = json!({ "id": out["output"]["id"], "name": name });
            }
            v.as_object_mut().map(|m| m.remove("uv"));
            v.as_object_mut().map(|m| m.remove("triangles"));
            Ok(v)
        })
        .await,
    )
}

fn outline_svg(outlines: &[Vec<[f64; 2]>]) -> String {
    let (mut w, mut h) = (0.0f64, 0.0f64);
    for p in outlines.iter().flatten() {
        w = w.max(p[0]);
        h = h.max(p[1]);
    }
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.2}mm\" height=\"{h:.2}mm\" viewBox=\"0 0 {w:.3} {h:.3}\">");
    for l in outlines {
        let d: Vec<String> = l.iter().map(|p| format!("{:.3},{:.3}", p[0], h - p[1])).collect();
        s.push_str(&format!("<polygon points=\"{}\" fill=\"none\" stroke=\"black\" stroke-width=\"0.2\"/>", d.join(" ")));
    }
    s.push_str("</svg>");
    s
}

fn outline_dxf(outlines: &[Vec<[f64; 2]>]) -> String {
    let mut s = String::from("0\nSECTION\n2\nENTITIES\n");
    for l in outlines {
        s.push_str(&format!("0\nLWPOLYLINE\n90\n{}\n70\n1\n", l.len()));
        for p in l {
            s.push_str(&format!("10\n{:.4}\n20\n{:.4}\n", p[0], p[1]));
        }
    }
    s.push_str("0\nENDSEC\n0\nEOF\n");
    s
}

/// `POST /mesh-from/<id>` with { tolerance, angle (degrees), maxEdge, name }:
/// a part tessellated with these controls, kept as a scan (to check, compare
/// or export), as FreeCAD's MeshPart mesher.
pub async fn mesh_from(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let project = project_name(&server, &query);
    let o: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let app = match server.project(query.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let s = server.clone();
    reply(
        blocking(move || {
            let tolerance = o.get("tolerance").and_then(Value::as_f64).unwrap_or(0.01).clamp(0.0001, 10.0);
            let angle = o.get("angle").and_then(Value::as_f64).unwrap_or(20.0).clamp(0.5, 90.0).to_radians();
            let src = {
                let project = app.project.lock().unwrap();
                let sid = serde_json::from_value::<StableId>(json!(id)).ok().filter(|x| project.entity(*x).is_some()).ok_or_else(|| format!("{id} is not a part"))?;
                crate::source(&project, sid).ok_or_else(|| format!("{id} is not a part"))?
            };
            let mut maker = de_shape::fabricate::StepMaker::new(tolerance);
            let mut shapes = Vec::new();
            let mut faceted = Vec::new();
            src.exact(&mut maker, &mut shapes, &mut faceted)?;
            let mut m = Mesh::default();
            for (_, shape, _) in &shapes {
                let part = maker.arena().mesh_angular(*shape, tolerance, angle)?;
                m.merge(&Mesh::from_flat(&part.positions, &part.triangles));
            }
            m.weld(tolerance * 1e-3);
            if let Some(e) = o.get("maxEdge").and_then(Value::as_f64).filter(|e| *e > 0.0) {
                de_scan::remesh::split_long(&mut m, e);
            }
            let name = o.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{} mesh", src.name));
            let report = de_scan::analyze::analyze(&m);
            let entry = add(&s, &project, &mesh_bytes(&m, &name), "ply", &name, "mesh-from-shape", None)?;
            Ok(json!({ "scan": entry, "triangles": m.triangles.len(), "solid": report.solid, "faceted": faceted }))
        })
        .await,
    )
}

/// `GET /scans/<id>/file`: the stored scan file (to download or export).
pub async fn file(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let project = project_name(&server, &query);
    let e = match find(&server, &project, &id) {
        Ok(e) => e,
        Err(err) => return fail(StatusCode::NOT_FOUND, err),
    };
    match de_shape::assets::path(&e.id).and_then(|p| std::fs::read(p).map_err(|x| x.to_string())) {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream".to_string()), (axum::http::header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.{}\"", de_shape::fabricate::file_name(&e.name), e.format))],
            bytes,
        )
            .into_response(),
        Err(err) => fail(StatusCode::NOT_FOUND, err),
    }
}

/// `POST /scans/<id>/remove`: taken off the project's list (the stored file stays for history).
pub async fn remove(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let project = project_name(&server, &query);
    let mut list = read_index(&server, &project);
    let before = list.len();
    list.retain(|e| e.id != id && e.name != id);
    match write_index(&server, &project, &list) {
        Ok(()) => axum::Json(json!({ "removed": before - list.len() })).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{serve, server_in};

    async fn http(at: std::net::SocketAddr, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(at).await.unwrap();
        s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {at}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        s.write_all(body).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let status = String::from_utf8_lossy(&buf[..split]).split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut body = buf[split + 4..].to_vec();
        // Chunked replies: join the chunks.
        if String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase().contains("transfer-encoding: chunked") {
            let mut out = Vec::new();
            let mut i = 0;
            while let Some(end) = body[i..].windows(2).position(|w| w == b"\r\n") {
                let n = usize::from_str_radix(String::from_utf8_lossy(&body[i..i + end]).trim(), 16).unwrap_or(0);
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&body[i + end + 2..i + end + 2 + n]);
                i += end + 2 + n + 2;
            }
            body = out;
        }
        (status, body)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_scan_is_added_repaired_inspected_against_its_part_and_made_a_part() {
        let dir = std::env::temp_dir().join(format!("forma-scans-{}", StableId::new()));
        if de_shape::assets::dir().is_none() {
            de_shape::assets::set_dir(dir.join("assets"));
        }
        let dir = de_shape::assets::dir().unwrap().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let server = server_in(&dir);
        let at = serve(server.clone()).await;
        let app = server.project(None).unwrap();
        // The CAD part: a 40 × 30 × 20 block.
        let part = app.commit_here("shape.create", json!({ "name": "Block", "source": "return box(40, 30, 20);", "params": [] })).unwrap()["output"]["id"].as_str().unwrap().to_string();

        // Its scan: points over the block, the top bulging up to 0.3 mm.
        let mut c = de_scan::points::sample_mesh(&de_scan::mesh::cube(40.0, 30.0, 20.0), 20000);
        for p in c.positions.iter_mut() {
            if (p[2] - 20.0).abs() < 1e-9 {
                p[2] += 0.3 * (std::f64::consts::PI * p[0] / 40.0).sin() * (std::f64::consts::PI * p[1] / 30.0).sin();
            }
        }
        c.normals.clear();
        let (status, reply) = http(at, "POST", "/scans/block_scan.ply?source=photogrammetry", &de_scan::points::write_ply(&c)).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&reply));
        let entry: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!((entry["kind"].as_str(), entry["points"].as_u64(), entry["source"].as_str()), (Some("points"), Some(20000), Some("photogrammetry")));
        let sid = entry["id"].as_str().unwrap().to_string();
        let (_, list) = http(at, "GET", "/scans", b"").await;
        assert_eq!(serde_json::from_slice::<Value>(&list).unwrap().as_array().unwrap().len(), 1);

        // Inspected: fails ±0.1, passes ±0.35, the worst at the top's middle.
        let (status, reply) = http(at, "POST", &format!("/scans/{sid}/inspect"), json!({ "target": part, "tolerance": 0.1 }).to_string().as_bytes()).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&reply));
        let r: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(r["passed"], json!(false));
        assert!((r["stats"]["max"].as_f64().unwrap() - 0.3).abs() < 0.01, "{}", r["stats"]);
        assert_eq!(r["values"].as_array().unwrap().len(), 20000);
        let (_, reply) = http(at, "POST", &format!("/scans/{sid}/inspect"), json!({ "target": "Block", "tolerance": 0.35 }).to_string().as_bytes()).await;
        assert_eq!(serde_json::from_slice::<Value>(&reply).unwrap()["passed"], json!(true));

        // A fit reads the top back as a plane; a reconstruction is a closed mesh scan.
        let (_, reply) = http(at, "POST", &format!("/scans/{sid}/query/fit"), json!({ "kind": "plane", "box": [0, 0, 19.5, 40, 30, 21] }).to_string().as_bytes()).await;
        let f: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(f["kind"], "plane", "{f}");
        let (status, reply) = http(at, "POST", &format!("/scans/{sid}/surface/reconstruct"), json!({ "depth": 6 }).to_string().as_bytes()).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&reply));
        let r: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(r["report"]["closed"], json!(true), "{r}");
        let mesh_id = r["scan"]["id"].as_str().unwrap().to_string();
        // Decimated, then made a part.
        let (_, reply) = http(at, "POST", &format!("/scans/{mesh_id}/mesh/decimate"), json!({ "ratio": 0.3 }).to_string().as_bytes()).await;
        let r: Value = serde_json::from_slice(&reply).unwrap();
        let light = r["scan"]["id"].as_str().unwrap().to_string();
        assert!(r["report"]["triangles"][1].as_u64() < r["report"]["triangles"][0].as_u64());
        let (status, reply) = http(at, "POST", &format!("/scans/{light}/shape"), json!({ "name": "Scanned block" }).to_string().as_bytes()).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&reply));
        let made: Value = serde_json::from_slice(&reply).unwrap();
        assert!(made["source"].as_str().unwrap().contains("importMesh"));

        // A mesh from the part with angle control, kept as a scan.
        let (status, reply) = http(at, "POST", &format!("/mesh-from/{part}"), json!({ "tolerance": 0.01, "maxEdge": 5 }).to_string().as_bytes()).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&reply));
        let r: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(r["solid"], json!(true));
        assert!(r["triangles"].as_u64().unwrap() > 12);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
