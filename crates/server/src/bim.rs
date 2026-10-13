//! Building information over HTTP: the whole building model gathered from
//! every placed shape (instances turned and moved into project
//! coordinates), and what is made from it. Nothing is sent anywhere.
//!
//! - `GET /bim/<id|project>` — the structure: site, building, levels with
//!   what stands on each, counts by class, spaces, zones, grids, notes
//! - `GET /ifc/<id|project>.ifc` — IFC4 (ReferenceView) of it all
//! - `GET /schedule/<id|project>?class=wall,door&level=…&group=level,class&format=json|csv&psets=1`
//! - `GET /plan/<id|project>[.pdf|.dxf]?view=plan&level=…&cut=1200 | view=section&axis=x&at=…&looking=east
//!   | view=elevation&side=south` and `sheet=A3&scale=100` — architectural drawings
//! - IFC files are imported by `POST /import/<name>.ifc` (see `import_ifc`).

use crate::{Server, bad_request, today};
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_bim::{Model, ModelElement};
use de_core::{Project, StableId, json};
use de_shape::{PartMaterials, ShapeProgram, fabricate};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

type Response = axum::response::Response;

fn fail(code: StatusCode, message: String) -> Response {
    (code, [(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string())], message).into_response()
}

/// One shape where it stands: its program (configured), turned and moved.
struct Placed {
    shape: StableId,
    program: ShapeProgram,
    transform: de_assembly::Transform,
    /// Instance names from the top ("L12"); empty for a shape placed on its own.
    path: Vec<String>,
    /// Material name in the program → material entity's name.
    materials: HashMap<String, String>,
}

fn material_names(project: &Project, id: StableId) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(t) = project.get::<PartMaterials>(id) {
        for (name, mid) in &t.0 {
            if let Some(m) = project.get::<de_solid::Material>(*mid) {
                out.insert(name.clone(), m.name.clone());
            }
        }
    }
    out
}

fn placed_shape(project: &Project, id: StableId, config: Option<&str>, transform: de_assembly::Transform, path: Vec<String>) -> Option<Placed> {
    let e = project.entity(id)?;
    let program = match config {
        Some(c) => de_shape::configs::configured_program(project.world(), e, c).ok()?,
        None => de_shape::resolved_program(project.world(), e)?,
    };
    Some(Placed { shape: id, program, transform, path, materials: material_names(project, id) })
}

/// Every shape inside `scope` ("project": everything placed), where it stands.
fn gather(project: &Project, scope: &str) -> Result<Vec<Placed>, String> {
    let mut out = Vec::new();
    let mut add = |project: &Project, id: StableId| {
        if project.get::<de_assembly::Instance>(id).is_some() || project.get::<de_assembly::AssemblyDef>(id).is_some() {
            for o in de_assembly::occurrences(project.world(), id) {
                let path = o.names[..o.names.len().saturating_sub(1)].to_vec();
                if let Some(p) = placed_shape(project, o.shape, o.config.as_deref(), o.transform, path) {
                    out.push(p);
                }
            }
        } else if project.get::<ShapeProgram>(id).is_some() {
            let at = project.get::<de_solid::Placement>(id).map(|p| [p.x, p.y, p.z].map(|l| l.canonical() * 1000.0)).unwrap_or([0.0; 3]);
            if let Some(p) = placed_shape(project, id, None, de_assembly::Transform::translation(at), vec![]) {
                out.push(p);
            }
        }
    };
    if scope == "project" {
        for id in project.entity_ids() {
            let top_instance = project.get::<de_assembly::Instance>(id).is_some_and(|i| i.parent.is_none());
            let placed_shape = project.get::<ShapeProgram>(id).is_some() && project.get::<de_solid::Placement>(id).is_some();
            if top_instance || placed_shape {
                add(project, id);
            }
        }
    } else {
        let id = serde_json::from_value::<StableId>(json!(scope)).map_err(|_| format!("“{scope}” is not an id (or “project”)"))?;
        if project.entity(id).is_none() {
            return Err(format!("{scope} does not exist"));
        }
        add(project, id);
    }
    if out.is_empty() {
        return Err("there are no shapes to read a building from".into());
    }
    Ok(out)
}

/// The project's title.
fn title_of(project: &Project, name: &str) -> String {
    project
        .entity_ids()
        .into_iter()
        .find_map(|id| project.get::<de_core::ProjectInfo>(id).map(|p| p.title.clone()))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| crate::pretty_name(name))
}

/// The building model of everything gathered (evaluates each program; slow
/// the first time, cached after).
fn model_of(placed: &[Placed], project_title: &str) -> Result<Model, String> {
    let mut m = Model { project: project_title.to_string(), ..Default::default() };
    let mut used_names: HashSet<String> = HashSet::new();
    let mut level_at: BTreeMap<String, f64> = BTreeMap::new();
    let mut any_levels = false;
    for p in placed {
        let e = de_shape::evaluated_at(&p.program, p.program.display_tolerance()).map_err(|e| format!("{}: {e}", p.program.name))?;
        let bim = e.declared.bim.clone().unwrap_or_default();
        let t = p.transform;
        let dz = t.at[2];
        let prefix = p.path.join(" › ");
        let named = |n: &str| if prefix.is_empty() { n.to_string() } else { format!("{prefix} › {n}") };
        let pt2 = |q: [f64; 2]| {
            let w = t.apply([q[0], q[1], 0.0]);
            [w[0], w[1]]
        };
        let dir2 = |q: [f64; 2]| {
            let w = t.turn([q[0], q[1], 0.0]);
            [w[0], w[1]]
        };
        // Levels: a name means one storey; the same name at another height is another.
        let mut renamed: HashMap<String, String> = HashMap::new();
        for l in &bim.levels {
            any_levels = true;
            let z = l.elevation + dz;
            let name = match level_at.get(&l.name) {
                Some(&h) if (h - z).abs() < 1.0 => l.name.clone(),
                Some(_) => format!("{} ({})", l.name, if prefix.is_empty() { p.program.name.clone() } else { prefix.clone() }),
                None => l.name.clone(),
            };
            if !level_at.contains_key(&name) {
                level_at.insert(name.clone(), z);
                m.levels.push(de_bim::Level { name: name.clone(), elevation: z, height: l.height });
            }
            renamed.insert(l.name.clone(), name);
        }
        let lv = |l: &Option<String>| l.as_ref().map(|n| renamed.get(n).cloned().unwrap_or_else(|| n.clone()));
        if m.site.name.is_empty() {
            if let Some(s) = &bim.site {
                m.site = s.clone();
            }
        }
        if m.building.name.is_empty() {
            if let Some(b) = &bim.building {
                m.building = b.clone();
            }
        }
        let parts: Vec<(&str, &manifold_csg::Manifold, Option<&String>, Option<&String>, f64)> = if e.parts.is_empty() {
            vec![(p.program.name.as_str(), &e.solid, None, None, e.volume)]
        } else {
            e.parts.iter().map(|x| (x.name.as_str(), &x.solid, x.material.as_ref(), x.process.as_ref(), x.volume)).collect()
        };
        for (name, solid, material, process, volume) in parts {
            let solid = t.solid(solid);
            let (pos, n, tri) = solid.to_mesh_f64();
            let n = n.max(3);
            let mesh = de_bim::Mesh {
                positions: pos.chunks_exact(n).map(|v| [v[0], v[1], v[2]]).collect(),
                triangles: tri.chunks_exact(3).map(|c| [c[0] as u32, c[1] as u32, c[2] as u32]).collect(),
            };
            let mut info = bim.elements.get(name).cloned().unwrap_or_default();
            info.level = lv(&info.level);
            info.host = info.host.as_deref().map(named);
            info.parent = info.parent.as_deref().map(named);
            for mark in &mut info.plan {
                for q in &mut mark.points {
                    *q = pt2(*q);
                }
            }
            let mut full = named(name);
            if !used_names.insert(full.clone()) {
                full = format!("{} › {full}", p.program.name);
                used_names.insert(full.clone());
            }
            let material = material.map(|mt| p.materials.get(mt).cloned().unwrap_or_else(|| mt.clone()));
            m.elements.push(ModelElement { name: full, key: format!("{}/{}/{name}", p.shape, prefix), info, material, process: process.cloned(), mesh, volume });
        }
        for s in &bim.spaces {
            let mut s = s.clone();
            s.name = named(&s.name);
            s.level = lv(&s.level);
            s.elevation += dz;
            for l in &mut s.outline {
                for q in l.iter_mut() {
                    *q = pt2(*q);
                }
            }
            m.spaces.push(s);
        }
        for o in &bim.openings {
            let mut o = o.clone();
            o.host = named(&o.host);
            o.fills = o.fills.as_deref().map(named);
            o.name = named(&o.name);
            o.origin = t.apply(o.origin);
            o.dir = dir2(o.dir);
            m.openings.push(o);
        }
        for z in &bim.zones {
            let mut z = z.clone();
            z.name = named(&z.name);
            z.spaces = z.spaces.iter().map(|s| named(s)).collect();
            m.zones.push(z);
        }
        for g in &bim.grids {
            let mut g = g.clone();
            g.name = named(&g.name);
            g.level = lv(&g.level);
            for a in g.u.iter_mut().chain(g.v.iter_mut()) {
                a.from = pt2(a.from);
                a.to = pt2(a.to);
            }
            m.grids.push(g);
        }
        for g in &bim.groups {
            let mut g = g.clone();
            g.name = named(&g.name);
            g.level = lv(&g.level);
            m.groups.push(g);
        }
    }
    // No levels declared anywhere: each top-level instance is a storey (a tower of floor instances).
    if !any_levels {
        let mut seen = HashSet::new();
        for p in placed.iter().filter(|p| !p.path.is_empty()) {
            if seen.insert(p.path[0].clone()) {
                m.levels.push(de_bim::Level { name: p.path[0].clone(), elevation: p.transform.at[2], height: None });
            }
        }
        for e in &mut m.elements {
            if let Some(first) = e.name.split(" › ").next().filter(|_| e.name.contains(" › ")) {
                if seen.contains(first) {
                    e.info.level = Some(first.to_string());
                }
            }
        }
        if !m.levels.is_empty() {
            m.notes.push("no levels are declared: each top-level instance is taken as a storey".into());
        }
    }
    if m.site.name.is_empty() {
        m.site.name = "Site".into();
    }
    m.settle();
    Ok(m)
}

/// Gathers the model for a request: (title, placed shapes) under the lock,
/// the evaluation outside it.
async fn model(server: &Server, q: &HashMap<String, String>, scope: &str) -> Result<(String, Model), Response> {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    let app = server.project(Some(&name)).map_err(bad_request)?;
    let (title, placed) = {
        let project = app.project.lock().unwrap();
        let placed = gather(&project, scope).map_err(|e| fail(StatusCode::NOT_FOUND, e))?;
        (title_of(&project, &name), placed)
    };
    let t = title.clone();
    let m = tokio::task::spawn_blocking(move || model_of(&placed, &t)).await.map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?.map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    Ok((title, m))
}

/// `GET /bim/<id|project>`: the building's structure.
pub async fn structure(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (_, m) = match model(&server, &q, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    for e in &m.elements {
        *classes.entry(if e.info.class.is_empty() { "IfcBuildingElementProxy".into() } else { e.info.class.clone() }).or_default() += 1;
    }
    let levels: Vec<_> = m
        .levels
        .iter()
        .map(|l| {
            let mut by: BTreeMap<String, usize> = BTreeMap::new();
            for e in m.elements.iter().filter(|e| e.info.level.as_ref() == Some(&l.name)) {
                *by.entry(if e.info.class.is_empty() { "IfcBuildingElementProxy".into() } else { e.info.class.clone() }).or_default() += 1;
            }
            let spaces: Vec<_> = m.spaces.iter().filter(|s| s.level.as_ref() == Some(&l.name)).map(|s| json!({ "name": s.name, "long_name": s.long_name, "area_m2": (s.area() * 100.0).round() / 100.0, "height": s.height })).collect();
            json!({ "name": l.name, "elevation": l.elevation, "height": l.height, "elements": by, "spaces": spaces })
        })
        .collect();
    let untagged: Vec<&str> = m.elements.iter().filter(|e| e.info.class.is_empty()).map(|e| e.name.as_str()).take(20).collect();
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for e in &m.elements {
        let (lo, hi) = de_bim::geom::bounds(&e.mesh);
        for k in 0..3 {
            bounds[k] = bounds[k].min(lo[k]);
            bounds[k + 3] = bounds[k + 3].max(hi[k]);
        }
    }
    axum::Json(json!({
        "project": m.project,
        "site": m.site,
        "building": m.building,
        "levels": levels,
        "classes": classes,
        "elements": m.elements.len(),
        "openings": m.openings.len(),
        "bounds": bounds.map(|v| (v * 10.0).round() / 10.0),
        "zones": m.zones.iter().map(|z| json!({ "name": z.name, "spaces": z.spaces })).collect::<Vec<_>>(),
        "grids": m.grids.iter().map(|g| json!({ "name": g.name, "level": g.level, "axes": g.u.iter().chain(g.v.iter()).map(|a| &a.tag).collect::<Vec<_>>() })).collect::<Vec<_>>(),
        "groups": m.groups.iter().map(|g| json!({ "name": g.name, "class": g.class, "level": g.level })).collect::<Vec<_>>(),
        "untagged": untagged,
        "notes": m.notes,
    }))
    .into_response()
}

/// `GET /ifc/<id|project>.ifc`: IFC4 of the building.
pub async fn ifc(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let scope = file.strip_suffix(".ifc").unwrap_or(&file).to_string();
    let (title, m) = match model(&server, &q, &scope).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let author = std::env::var("USER").unwrap_or_else(|_| "Forma".into());
    let text = de_bim::ifc_write::write(&m, &de_bim::ifc_write::Options { timestamp: format!("{}T00:00:00", today()), author });
    let name = if m.building.name.is_empty() { title } else { m.building.name.clone() };
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/x-step".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}.ifc\"", fabricate::file_name(&name)))], text).into_response()
}

fn list(q: &HashMap<String, String>, key: &str) -> Vec<String> {
    q.get(key).map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

/// `GET /schedule/<id|project>`: rows of elements, spaces and zones with their quantities.
pub async fn schedule(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (title, m) = match model(&server, &q, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let group_by = list(&q, "group");
    if let Some(bad) = group_by.iter().find(|g| !["level", "class", "type", "material", "predefined"].contains(&g.as_str())) {
        return fail(StatusCode::BAD_REQUEST, format!("group by level, class, type, material or predefined, not “{bad}”"));
    }
    let query = de_bim::schedule::Query { classes: list(&q, "class"), levels: list(&q, "level"), group_by };
    let s = de_bim::schedule::schedule(&m, &query);
    match q.get("format").map(String::as_str).unwrap_or("json") {
        "csv" => {
            let what = if query.classes.is_empty() { "schedule".to_string() } else { format!("{} schedule", query.classes.join(" ")) };
            let text = de_bim::schedule::csv(&s, q.get("psets").is_some_and(|v| v == "1" || v == "true"));
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{} {what}.csv\"", fabricate::file_name(&title)))], text).into_response()
        }
        "json" => axum::Json(json!({ "project": title, "levels": m.levels, "schedule": s })).into_response(),
        other => fail(StatusCode::BAD_REQUEST, format!("format is json or csv, not {other}")),
    }
}

/// `GET /plan/<id|project>[.pdf|.dxf]`: a level plan, section or elevation.
pub async fn plan(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (scope, format) = match file.rsplit_once('.') {
        Some((s, f)) if f == "pdf" || f == "dxf" => (s.to_string(), f),
        _ => (file.clone(), "json"),
    };
    let num = |k: &str| q.get(k).filter(|v| !v.is_empty()).map(|v| v.parse::<f64>().map_err(|_| format!("{k} is a number, not “{v}”"))).transpose();
    let view = match q.get("view").map(String::as_str).unwrap_or("plan") {
        "plan" => match (q.get("level"), num("cut")) {
            (_, Err(e)) => return fail(StatusCode::BAD_REQUEST, e),
            (Some(l), Ok(cut)) => de_shape::plan::View::Plan { level: l.clone(), cut },
            (None, Ok(cut)) => de_shape::plan::View::Plan { level: String::new(), cut },
        },
        "section" => match num("at") {
            Ok(Some(at)) => de_shape::plan::View::Section { axis: q.get("axis").cloned().unwrap_or_else(|| "y".into()), at, looking: q.get("looking").cloned() },
            Ok(None) => return fail(StatusCode::BAD_REQUEST, "a section needs at (mm along its axis)".into()),
            Err(e) => return fail(StatusCode::BAD_REQUEST, e),
        },
        "elevation" => de_shape::plan::View::Elevation { side: q.get("side").cloned().unwrap_or_else(|| "south".into()) },
        other => return fail(StatusCode::BAD_REQUEST, format!("view is plan, section or elevation, not {other}")),
    };
    let scale = match num("scale") {
        Ok(s) => s,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let (title, m) = match model(&server, &q, &scope).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    // A plan of no named level: the lowest.
    let view = match view {
        de_shape::plan::View::Plan { level, cut } if level.is_empty() => match m.levels.first() {
            Some(l) => de_shape::plan::View::Plan { level: l.name.clone(), cut },
            None => return fail(StatusCode::UNPROCESSABLE_ENTITY, "the model has no levels: declare them with level(name, elevation)".into()),
        },
        v => v,
    };
    let sheet = de_shape::plan::Sheet::named(q.get("sheet").map(String::as_str).unwrap_or("A3"));
    let t = de_shape::plan::Title {
        project: title.clone(),
        number: format!("{}-{}", q.get("project").cloned().unwrap_or_else(|| server.default.clone()).to_uppercase(), view.title().chars().filter(|c| c.is_alphanumeric()).take(8).collect::<String>().to_uppercase()),
        drawn_by: std::env::var("USER").map(|u| u[..1].to_uppercase() + &u[1..]).unwrap_or_else(|_| "Forma".into()),
        date: today(),
    };
    let v2 = view.clone();
    let made = tokio::task::spawn_blocking(move || de_shape::plan::draw(&m, &v2, sheet, &t, scale)).await;
    let d = match made {
        Ok(Ok(d)) => d,
        Ok(Err(e)) => return fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let stem = fabricate::file_name(&format!("{title} {}", view.title()));
    match format {
        "pdf" => (StatusCode::OK, [(header::CONTENT_TYPE, "application/pdf".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{stem}.pdf\""))], de_shape::drawing::pdf(&d)).into_response(),
        "dxf" => (StatusCode::OK, [(header::CONTENT_TYPE, "application/dxf".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{stem}.dxf\""))], de_shape::drawing::dxf(&d)).into_response(),
        _ => axum::Json(&d).into_response(),
    }
}

/// `POST /import/<name>.ifc`: stores the file by content (an IFC file is a
/// STEP physical file), reads it, makes its materials and adds the shape.
pub async fn import_ifc(server: &Server, query: &HashMap<String, String>, file: &str, body: &[u8]) -> Response {
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let checked = {
        let (file, body) = (file.to_string(), body.to_vec());
        tokio::task::spawn_blocking(move || ifc_source(&file, &body)).await.unwrap_or_else(|e| Err(e.to_string()))
    };
    let (name, source, shown, count, mats) = match checked {
        Ok(x) => x,
        Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{file} could not be read: {e}")),
    };
    let sha = match de_shape::assets::store(body) {
        Ok(s) => s,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let source = source.replace("{SHA}", &sha);
    let user = query.get("user").filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| "local".into());
    let actor = de_core::Actor { user_id: user, session_id: StableId::new().to_string() };
    let materials = materials(&app, &actor, &mats);
    let revision = app.project.lock().unwrap().revision();
    let message = json!({
        "type": "commit", "request_id": 1, "operation": "shape.create", "expected_revision": revision,
        "parameters": { "name": name, "description": format!("Imported from {file}."), "source": source, "params": [], "display_tolerance": shown, "materials": materials }
    });
    let reply: serde_json::Value = serde_json::from_str(&app.handle(&actor, &message.to_string())).unwrap_or_default();
    if reply["ok"] == true {
        (StatusCode::OK, axum::Json(json!({ "id": reply["value"]["output"]["id"], "bodies": count, "ifc": true, "sha": sha }))).into_response()
    } else {
        fail(StatusCode::UNPROCESSABLE_ENTITY, reply["error"]["message"].as_str().unwrap_or("the import failed").to_string())
    }
}

/// An IFC file dropped on the view (or `./forma ifc import`): checked to
/// read, stored by content like a STEP file (it is one: ISO 10303-21), and
/// added as a shape whose program is `return importIfc("<sha>")`.
pub fn ifc_source(file: &str, bytes: &[u8]) -> Result<(String, String, f64, usize, Vec<String>), String> {
    let text = String::from_utf8_lossy(bytes);
    let read = de_bim::ifc_read::read(&text)?;
    if read.parts.is_empty() {
        return Err("the file has no building elements with geometry".into());
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &read.parts {
        for m in &p.solids {
            let (a, b) = de_bim::geom::bounds(m);
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(b[k]);
            }
        }
    }
    let size = (0..3).map(|k| hi[k] - lo[k]).filter(|v| v.is_finite()).fold(0.0, f64::max);
    let shown = (size / 4000.0).clamp(0.05, 5.0);
    let notes = if read.notes.is_empty() { String::new() } else { format!("\n// Read with notes: {}", read.notes.join("; ")) };
    let summary = format!(
        "// Imported from “{file}” ({}, {} elements on {} level{}, {} spaces).{notes}\nreturn importIfc(\"{{SHA}}\");",
        read.schema,
        read.parts.len(),
        read.levels.len(),
        if read.levels.len() == 1 { "" } else { "s" },
        read.spaces.len()
    );
    let name = if read.building.name.is_empty() || read.building.name == "Building" { file.rsplit_once('.').map(|(n, _)| n).unwrap_or(file).replace(['_', '-'], " ") } else { read.building.name.clone() };
    let mut materials: Vec<String> = read.parts.iter().filter_map(|p| p.material.clone()).collect();
    materials.sort();
    materials.dedup();
    Ok((name, summary, shown, read.parts.len(), materials))
}

/// A typical density (kg/m³) for a building material's name.
fn typical_density(name: &str) -> f64 {
    let n = name.to_lowercase();
    let table: &[(&str, f64)] = &[
        ("concrete", 2400.0), ("brick", 1900.0), ("block", 1400.0), ("masonry", 1900.0), ("stone", 2600.0), ("steel", 7850.0), ("iron", 7200.0), ("alumin", 2700.0), ("copper", 8900.0),
        ("glass", 2500.0), ("gypsum", 700.0), ("plaster", 1200.0), ("oak", 750.0), ("timber", 500.0), ("wood", 500.0), ("plywood", 600.0), ("insulation", 30.0), ("mineral wool", 30.0), ("tile", 2000.0),
    ];
    table.iter().find(|(k, _)| n.contains(k)).map(|(_, d)| *d).unwrap_or(1000.0)
}

/// Material entities for these names: the project's own by name, else made at typical densities.
pub fn materials(app: &crate::App, actor: &de_core::Actor, names: &[String]) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for name in names {
        let found = {
            let project = app.project.lock().unwrap();
            project.entity_ids().into_iter().find(|id| project.get::<de_solid::Material>(*id).is_some_and(|m| m.name.eq_ignore_ascii_case(name)))
        };
        let id = match found {
            Some(id) => serde_json::to_value(id).ok(),
            None => {
                let revision = app.project.lock().unwrap().revision();
                let message = json!({ "type": "commit", "request_id": 1, "operation": "solid.material.create", "expected_revision": revision, "parameters": { "name": name, "density": { "value": typical_density(name), "unit": "kg/m3" } } });
                let reply: serde_json::Value = serde_json::from_str(&app.handle(actor, &message.to_string())).unwrap_or_default();
                (reply["ok"] == true).then(|| reply["value"]["output"]["id"].clone())
            }
        };
        if let Some(id) = id {
            out.insert(name.clone(), id);
        }
    }
    out
}
