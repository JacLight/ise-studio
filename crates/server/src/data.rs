//! Data in and out: tables (sheets, parameter sets, configuration tables)
//! as CSV and XLSX, OpenSCAD files, and whole projects as one portable
//! bundle file (`.ise`) to hand to someone outside the team.

use super::*;
use de_shape::{ParameterSet, ResolvedParams, ShapeConfigurations, paramtable, scad, sheet, table};

/// A project bundle: a zip with `manifest.json`, `project.jsonl` (the whole
/// log, so history and versions come along), `thumb.png` and the assets the
/// log refers to (imported STEP files, pictures) under `assets/`.
const BUNDLE_FORMAT: &str = "ise-bundle";

fn download(name: String, mime: &str, bytes: Vec<u8>) -> axum::response::Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, mime.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], bytes).into_response()
}

/// `GET /table/<id>.csv|.xlsx?project=…`: a sheet, a parameter set or a
/// shape's configuration table as a file.
pub async fn table_file(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return bad_request(e),
    };
    let Some((stem, ext)) = file.rsplit_once('.') else { return bad_request("expected /table/<id>.csv or .xlsx") };
    let Ok(id) = serde_json::from_value::<StableId>(json!(stem)) else { return bad_request(format!("{stem} is not an id")) };
    let project = app.project.lock().unwrap();
    let world = project.world();
    let Some(e) = project.entity(id) else { return (StatusCode::NOT_FOUND, format!("no entity {id}")).into_response() };
    // (file name, rows for CSV, workbook)
    let (name, rows, workbook) = if let Some(s) = world.get::<sheet::Sheet>(e) {
        let values = world.get::<sheet::SheetValues>(e).cloned().unwrap_or_else(|| sheet::evaluate(world, s));
        (s.name.clone(), sheet::rows(s, &values), sheet::to_xlsx(s, &values))
    } else if let Some(set) = world.get::<ParameterSet>(e) {
        let rows = paramtable::parameter_rows(world, set, world.get::<ResolvedParams>(e));
        let wb = table::write_xlsx(&[table::OutSheet { name: set.name.clone(), cells: table::out_cells(&rows), names: Default::default(), bold_rows: vec![0] }]);
        (set.name.clone(), rows, wb)
    } else if let (Some(t), Some(p)) = (world.get::<ShapeConfigurations>(e), world.get::<ShapeProgram>(e)) {
        let rows = paramtable::config_rows(t);
        let wb = table::write_xlsx(&[table::OutSheet { name: "Configurations".into(), cells: table::out_cells(&rows), names: Default::default(), bold_rows: vec![0] }]);
        (format!("{} configurations", p.name), rows, wb)
    } else {
        return bad_request("only sheets, parameter sets and shapes with a configuration table are tables");
    };
    let base = fabricate::file_name(&name);
    match ext {
        "csv" => download(format!("{base}.csv"), "text/csv; charset=utf-8", table::write_csv(&rows).into_bytes()),
        "xlsx" => download(format!("{base}.xlsx"), "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", workbook),
        _ => bad_request("tables export as .csv or .xlsx"),
    }
}

/// `.scad` for `/export/<id>.scad`: an imported OpenSCAD file with its
/// parameters' values now, else a polyhedron per part.
pub fn scad_file(project: &Project, id: StableId, tolerance: f64) -> Result<(String, Vec<u8>), String> {
    let e = project.entity(id).ok_or_else(|| format!("{id} is not a part"))?;
    if let Some(p) = de_shape::resolved_program(project.world(), e)
        && let Some(src) = scad::export_source(&p.source, &p.params.iter().map(|q| (q.key.clone(), q.value)).collect::<Vec<_>>())
    {
        return Ok((format!("{}.scad", fabricate::file_name(&p.name)), src.into_bytes()));
    }
    let s = source(project, id).ok_or_else(|| format!("{id} is not a part"))?;
    if !s.members.is_empty() {
        return Err("export an assembly's parts one at a time as .scad (or the whole as STEP)".into());
    }
    let items = s.items(tolerance)?;
    let mut out = format!("// {} — exported from ISE Studio as polyhedra (millimetres).\n// ISE Studio keeps the exact model; this is its mesh at {tolerance} mm.\n\n", s.name);
    for it in &items {
        let (props, n, idx) = it.solid.to_mesh_f64();
        let n = n.max(3);
        let pos: Vec<f32> = props.chunks_exact(n).flat_map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]).collect();
        let tri: Vec<u32> = idx.iter().map(|i| *i as u32).collect();
        out.push_str(&scad::polyhedron(&it.name, None, &pos, &tri));
        out.push('\n');
    }
    Ok((format!("{}.scad", fabricate::file_name(&s.name)), out.into_bytes()))
}

/// Imports a .scad, .csv or .xlsx dropped on the view (or posted by `./ise import`).
pub async fn import_data(app: Arc<App>, file: &str, query: &HashMap<String, String>, body: &[u8]) -> axum::response::Response {
    let fail = |m: String| (StatusCode::UNPROCESSABLE_ENTITY, m).into_response();
    let lower = file.to_lowercase();
    let stem = file.rsplit_once('.').map(|(n, _)| n).unwrap_or(file).replace(['_', '-'], " ");
    let (operation, parameters) = if lower.ends_with(".scad") {
        let Ok(text) = String::from_utf8(body.to_vec()) else { return fail(format!("{file} is not text")) };
        ("shape.import_scad", json!({ "source": text, "file": file }))
    } else {
        let format = if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") { "xlsx" } else { "csv" };
        let as_ = query.get("as").map(String::as_str).unwrap_or("sheet");
        let payload = if format == "csv" { json!({ "text": String::from_utf8_lossy(body) }) } else { json!({ "base64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body) }) };
        let mut p = json!({ "format": format, "sheet": query.get("sheet") });
        p.as_object_mut().unwrap().extend(payload.as_object().unwrap().clone());
        match (as_, query.get("target")) {
            ("parameters", target) => {
                if let Some(t) = target {
                    p["target"] = json!(t);
                } else {
                    p["name"] = json!(stem.trim());
                }
                ("parameters.import", p)
            }
            ("configurations", Some(t)) => {
                p["target"] = json!(t);
                ("shape.config.import", p)
            }
            ("configurations", None) => return fail("say which shape the configurations are for (&target=<shape id>)".into()),
            (_, target) => {
                if let Some(t) = target {
                    p["target"] = json!(t);
                    p["replace"] = json!(true);
                } else {
                    p["name"] = json!(stem.trim());
                }
                ("sheet.import", p)
            }
        }
    };
    let user = query.get("user").filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| "local".into());
    let actor = Actor { user_id: user, session_id: StableId::new().to_string() };
    let revision = app.project.lock().unwrap().revision();
    let message = json!({ "type": "commit", "request_id": 1, "operation": operation, "expected_revision": revision, "parameters": parameters });
    let handled = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || app.handle(&actor, &message.to_string())).await.unwrap_or_default()
    };
    let reply: Value = serde_json::from_str(&handled).unwrap_or_default();
    if reply["ok"] == true {
        let out = &reply["value"]["output"];
        (StatusCode::OK, axum::Json(json!({ "id": out["id"], "operation": operation, "output": out }))).into_response()
    } else {
        fail(reply["error"]["message"].as_str().unwrap_or("the import failed").to_string())
    }
}

/// The asset files a log refers to (by content hash).
fn assets_of(dir: &std::path::Path, log: &str) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir.join("assets")) else { return vec![] };
    let files: Vec<(String, PathBuf)> = entries.flatten().filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path()))).collect();
    let mut hashes = std::collections::HashSet::new();
    let b = log.as_bytes();
    let mut i = 0;
    while i + 64 <= b.len() {
        if b[i..i + 64].iter().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) && (i == 0 || !b[i - 1].is_ascii_hexdigit()) && b.get(i + 64).is_none_or(|c| !c.is_ascii_hexdigit()) {
            hashes.insert(String::from_utf8_lossy(&b[i..i + 64]).to_string());
            i += 64;
        } else {
            i += 1;
        }
    }
    files.into_iter().filter(|(n, _)| n.len() >= 64 && hashes.contains(&n[..64])).collect()
}

/// `GET /projects/<name>/bundle`: the whole project as one `.ise` file.
pub async fn bundle(State(server): State<Arc<Server>>, Path(name): Path<String>) -> axum::response::Response {
    if !valid_name(&name) || !server.dir.join(format!("{name}.jsonl")).exists() {
        return bad_request(format!("no project called {name}"));
    }
    let dir = server.dir.clone();
    let made = tokio::task::spawn_blocking(move || -> Result<(String, Vec<u8>), String> {
        let log = std::fs::read_to_string(dir.join(format!("{name}.jsonl"))).map_err(|e| e.to_string())?;
        let info = info_of(&dir, &name);
        let title = info["title"].as_str().map(str::to_string).unwrap_or_else(|| pretty_name(&name));
        let mut files = vec![
            ("manifest.json".to_string(), serde_json::to_vec_pretty(&json!({ "format": BUNDLE_FORMAT, "version": 1, "name": name, "title": title, "changes": log.lines().filter(|l| !l.trim().is_empty()).count() })).unwrap()),
            ("project.jsonl".to_string(), log.clone().into_bytes()),
        ];
        if let Ok(t) = std::fs::read(dir.join(format!("{name}.thumb.png"))) {
            files.push(("thumb.png".into(), t));
        }
        for (file, path) in assets_of(&dir, &log) {
            files.push((format!("assets/{file}"), std::fs::read(&path).map_err(|e| e.to_string())?));
        }
        Ok((format!("{}.ise", fabricate::file_name(&title)), table::zip_deflated(&files)))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match made {
        Ok((file, bytes)) => download(file, "application/zip", bytes),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// `POST /projects/import?name=<wanted>`: a `.ise` bundle as a new project
/// (named after the bundle, numbered if taken). Its log is checked by
/// loading it before anything is written.
pub async fn import_bundle(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let files = match table::unzip(&body) {
        Ok(f) => f,
        Err(e) => return bad_request(format!("not a ISE Studio bundle: {e}")),
    };
    let manifest: Value = files.get("manifest.json").and_then(|m| serde_json::from_slice(m).ok()).unwrap_or_default();
    if manifest["format"] != BUNDLE_FORMAT {
        return bad_request("not a ISE Studio bundle (no manifest.json saying ise-bundle)");
    }
    let Some(log) = files.get("project.jsonl") else { return bad_request("the bundle has no project.jsonl") };
    let wanted = query.get("name").cloned().or_else(|| manifest["name"].as_str().map(str::to_string)).unwrap_or_else(|| "imported".into());
    let mut stem: String = wanted.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').to_string();
    stem.truncate(56);
    if !valid_name(&stem) {
        stem = "imported".into();
    }
    let mut name = stem.clone();
    let mut n = 1;
    while server.dir.join(format!("{name}.jsonl")).exists() {
        n += 1;
        name = format!("{stem}-{n}");
    }
    // Check the log loads before writing anything.
    let check = {
        let (dir, name, log, registry) = (server.dir.clone(), name.clone(), log.clone(), server.registry.clone());
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let tmp = dir.join(format!(".import-{}.jsonl", StableId::new()));
            std::fs::write(&tmp, &log).map_err(|e| e.to_string())?;
            let loaded = load(&name, &tmp, registry, "import-check");
            let _ = std::fs::remove_file(&tmp);
            loaded.map(|_| ()).map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    };
    if let Err(e) = check {
        return bad_request(format!("the bundle's project does not load: {e}"));
    }
    let write = || -> std::io::Result<usize> {
        let assets = server.dir.join("assets");
        let mut count = 0;
        for (path, bytes) in &files {
            if let Some(file) = path.strip_prefix("assets/") {
                // Content-addressed: a file already here is the same file.
                if file.contains(['/', '\\']) || file.starts_with('.') || file.len() < 64 || !file[..64].bytes().all(|c| c.is_ascii_hexdigit()) {
                    continue;
                }
                std::fs::create_dir_all(&assets)?;
                let to = assets.join(file);
                if !to.exists() {
                    std::fs::write(to, bytes)?;
                    count += 1;
                }
            }
        }
        if let Some(t) = files.get("thumb.png") {
            std::fs::write(server.dir.join(format!("{name}.thumb.png")), t)?;
        }
        std::fs::write(server.dir.join(format!("{name}.jsonl")), log)?;
        Ok(count)
    };
    match write() {
        Ok(assets) => axum::Json(json!({ "name": name, "title": manifest["title"], "assets": assets })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
