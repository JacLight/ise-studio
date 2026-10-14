//! Pictures in and out: pictures (and fonts) a project's shape programs read,
//! pictures pasted into the Iṣẹ́ AI conversation, and the engine's own
//! pictures of the model (`./ise look`).
//!
//! Files programs read are kept by content in the folder's shared store (see
//! `de_shape::assets`), so a part refers to them by hash and teammates fetch
//! them like imported STEP files; each project keeps an index of the ones
//! added to it (`<project>.images.json`: names, sizes, when). Pictures pasted
//! into the conversation stay with it, in the project's `.ai/attachments`
//! folder, and reach the journal only if a part comes to use one.

use crate::{Server, StableId, json};
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

/// One file added to a project for its programs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// The file it came from.
    pub name: String,
    /// "png", "jpg", "ttf" or "otf".
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<usize>,
    pub bytes: usize,
    pub added: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn project_name(server: &Server, query: &HashMap<String, String>) -> String {
    query.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone())
}

fn index_path(server: &Server, project: &str) -> std::path::PathBuf {
    server.dir.join(format!("{project}.images.json"))
}

fn read_index(server: &Server, project: &str) -> Vec<Entry> {
    std::fs::read_to_string(index_path(server, project)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// What a file is and how big, checked by reading it: a picture decodes, a font parses.
fn check(bytes: &[u8]) -> Result<(&'static str, Option<(usize, usize)>), String> {
    match de_shape::assets::sniff(bytes) {
        k @ ("png" | "jpg") => {
            let p = de_shape::image::decode(bytes)?;
            Ok((k, Some((p.width, p.height))))
        }
        k @ ("ttf" | "otf") => {
            de_shape::text::check_font(bytes)?;
            Ok((k, None))
        }
        _ => Err("ISE Studio reads PNG and JPEG pictures, and TrueType or OpenType fonts".into()),
    }
}

/// `POST /images/<file name>?project=…` with the file as the body: stores a
/// picture or font for the project's programs; answers its id.
pub async fn add(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    if let Err(e) = server.project(query.get("project")) {
        return fail(StatusCode::BAD_REQUEST, e);
    }
    let bytes = body.to_vec();
    let checked = tokio::task::spawn_blocking(move || check(&bytes).map(|c| (c, bytes))).await;
    let ((kind, size), bytes) = match checked {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{file}: {e}")),
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let id = match de_shape::assets::store(&bytes) {
        Ok(id) => id,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let project = project_name(&server, &query);
    let mut index = read_index(&server, &project);
    let entry = Entry { id: id.clone(), name: file.clone(), kind: kind.into(), width: size.map(|s| s.0), height: size.map(|s| s.1), bytes: bytes.len(), added: now_ms() };
    index.retain(|e| e.id != id);
    index.push(entry.clone());
    if let Err(e) = std::fs::write(index_path(&server, &project), serde_json::to_string_pretty(&index).unwrap()) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, format!("could not save the list of pictures: {e}"));
    }
    axum::Json(entry).into_response()
}

/// `GET /images?project=…`: the pictures and fonts added to the project.
pub async fn list(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let project = project_name(&server, &query);
    axum::Json(read_index(&server, &project)).into_response()
}

fn content_type(kind: &str) -> &'static str {
    match kind {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "pdf" => "application/pdf",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

/// `GET /images/<id>`: a stored picture's bytes (for showing it).
/// `?silhouette=<rows>` instead measures the object it shows (`./ise image
/// profile`): its outline row by row in pixels, and in mm given the object's
/// real `height` or `width` (`threshold`: colour distance 0–1 from the background).
pub async fn get(Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    if let Some(rows) = query.get("silhouette") {
        let num = |k: &str| query.get(k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0);
        let rows = rows.parse::<usize>().unwrap_or(40);
        let rim = query.get("rim").is_some_and(|v| v != "0" && v != "false");
        return match silhouette(&id, rows, num("threshold").map(|t| t as f32), num("height"), num("width"), rim) {
            Ok(v) => axum::Json(v).into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
        };
    }
    match de_shape::assets::path(&id).and_then(|p| std::fs::read(p).map_err(|e| e.to_string())) {
        Ok(bytes) => {
            let kind = de_shape::assets::sniff(&bytes);
            ([(header::CONTENT_TYPE, content_type(kind)), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], bytes).into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// A picture's object measured (see `de_shape::image::silhouette`): pixels,
/// and millimetres when its real height or width is given, the profile as
/// [radius, z] from the bottom up for revolving a round object.
fn silhouette(id: &str, rows: usize, threshold: Option<f32>, height: Option<f64>, width: Option<f64>, rim: bool) -> Result<serde_json::Value, String> {
    if !(2..=2000).contains(&rows) {
        return Err("silhouette: 2 to 2000 rows".into());
    }
    let p = de_shape::image::load(id)?;
    let s = de_shape::image::silhouette(&p, rows, threshold, rim)?;
    let widest = s.rows.iter().map(|r| r.right - r.left).fold(0.0, f64::max);
    let tall = s.bottom - s.top;
    let k = height.map(|h| h / tall).or(width.map(|w| w / widest));
    let rows_px: Vec<_> = s.rows.iter().map(|r| json!({ "y": r.y, "left": r.left, "right": r.right, "width": r.right - r.left, "middle": (r.left + r.right) / 2.0 })).collect();
    let mut out = json!({
        "picture": { "width": p.width, "height": p.height },
        "pixels": { "top": s.top, "bottom": s.bottom, "height": tall, "widest": widest, "axis": s.axis, "drift": s.drift, "cap_top": s.cap_top, "cap_bottom": s.cap_bottom, "rows": rows_px },
    });
    if let Some(k) = k {
        let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
        let mut points: Vec<[f64; 2]> = s.rows.iter().rev().map(|r| [r3((r.right - r.left) / 2.0 * k), r3((s.bottom - r.y) * k)]).collect();
        if let Some(first) = points.first_mut() {
            first[1] = 0.0;
        }
        if let Some(last) = points.last_mut() {
            last[1] = r3(tall * k);
        }
        out["mm"] = json!({ "per_pixel": k, "height": r3(tall * k), "widest": r3(widest * k), "drift": r3(s.drift * k), "cap_top": r3((s.cap_top - s.top) * k), "cap_bottom": r3((s.bottom - s.cap_bottom) * k), "profile": points });
    }
    Ok(out)
}

// ---- Pictures in the conversation -------------------------------------------------

/// What the panel can attach: pictures, and PDF pages (a drawing).
pub fn attachment_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"%PDF-") {
        return Some("pdf");
    }
    match de_shape::assets::sniff(bytes) {
        k @ ("png" | "jpg") => Some(k),
        _ if bytes.starts_with(b"GIF8") => Some("gif"),
        _ if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" => Some("webp"),
        _ => None,
    }
}

pub fn media_type(kind: &str) -> &'static str {
    match kind {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// `POST /ai/attachments/<file name>?project=…`: keeps a picture (or PDF) for
/// the conversation; answers its id, to send with the message.
pub async fn attach(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let Some(kind) = attachment_kind(&body) else { return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{file}: attach a picture (PNG, JPEG, GIF, WebP) or a PDF")) };
    // Claude reads pictures up to about 5 MB and PDFs up to 32 MB.
    let limit = if kind == "pdf" { 32 } else { 5 } * 1024 * 1024;
    if body.len() > limit {
        return fail(StatusCode::PAYLOAD_TOO_LARGE, format!("{file} is {:.1} MB: the AI reads {} up to {} MB", body.len() as f64 / 1048576.0, if kind == "pdf" { "PDFs" } else { "pictures" }, limit / 1048576));
    }
    let size = if matches!(kind, "png" | "jpg") { de_shape::image::decode(&body).ok().map(|p| (p.width, p.height)) } else { None };
    match app.ai.keep_attachment(&body, kind, &file, size) {
        Ok(a) => axum::Json(a).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// `GET /ai/attachments/<id>?project=…`: an attachment's bytes.
pub async fn attachment(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    match app.ai.attachment(&id) {
        Some((bytes, kind)) => ([(header::CONTENT_TYPE, media_type(&kind)), (header::CACHE_CONTROL, "private, max-age=31536000, immutable")], bytes).into_response(),
        None => (StatusCode::NOT_FOUND, "no such attachment").into_response(),
    }
}

// ---- Looking at the model ---------------------------------------------------------

/// JSON with every character outside ASCII escaped, so it fits in a header.
fn ascii_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            let mut buf = [0u16; 2];
            for u in ch.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{:04x}", u));
            }
        }
    }
    out
}

/// `GET /look?project=…[&id=…][&view=iso][&size=1000x750]`: a picture of the
/// model (or of one placed thing), drawn by the engine. Headers carry the
/// scale (mm per pixel) and the bounds.
pub async fn look(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    // A canvas's outline measured in mm (no model needed): `measure=<canvas>`
    // with trace options (channel, threshold, invert, largest, holes, region
    // l,t,r,b) and `every` (mm between stations).
    if let Some(what) = query.get("measure").filter(|s| !s.is_empty()) {
        let found = {
            let project = app.project.lock().unwrap();
            de_shape::canvas::find(project.world(), what)
        };
        let Some((_, canvas)) = found else { return fail(StatusCode::NOT_FOUND, format!("no canvas called “{what}” (./ise canvas list)")) };
        let mut o = serde_json::Map::new();
        for key in ["channel", "threshold", "invert", "largest", "holes", "region", "smooth", "minArea"] {
            if let Some(v) = query.get(key) {
                let value = match key {
                    "channel" => json!(v),
                    "invert" | "largest" | "holes" => json!(v != "0" && v != "false"),
                    "region" => json!(v.split(',').filter_map(|s| s.trim().parse::<f64>().ok()).collect::<Vec<f64>>()),
                    _ => match v.parse::<f64>() {
                        Ok(n) => json!(n),
                        Err(_) => return fail(StatusCode::BAD_REQUEST, format!("{key} must be a number")),
                    },
                };
                o.insert(key.to_string(), value);
            }
        }
        if !o.contains_key("largest") && !o.contains_key("channel") {
            o.insert("largest".into(), json!(true));
        }
        let options: de_shape::canvas::TraceOptions = match serde_json::from_value(serde_json::Value::Object(o)) {
            Ok(v) => v,
            Err(e) => return fail(StatusCode::BAD_REQUEST, e.to_string()),
        };
        let every = query.get("every").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let measured = tokio::task::spawn_blocking(move || de_shape::canvas::measure(&canvas, &options, every, 0.05)).await.unwrap_or_else(|e| Err(e.to_string()));
        return match measured {
            Ok(v) => axum::Json(v).into_response(),
            Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        };
    }
    let view = match de_shape::render::View::parse(query.get("view").map(String::as_str).unwrap_or("iso")) {
        Ok(v) => v,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let (w, h) = query
        .get("size")
        .and_then(|s| s.split_once('x'))
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .unwrap_or((1000, 750));
    if !(64..=2400).contains(&w) || !(64..=2400).contains(&h) {
        return fail(StatusCode::BAD_REQUEST, "size is WIDTHxHEIGHT, 64 to 2400 pixels each".into());
    }
    // What to draw: one thing, or everything placed.
    let sources = {
        let project = app.project.lock().unwrap();
        let ids: Vec<StableId> = match query.get("id").filter(|s| !s.is_empty()) {
            Some(id) => match serde_json::from_value::<StableId>(json!(id)) {
                Ok(sid) if project.entity(sid).is_some() => vec![sid],
                _ => return fail(StatusCode::NOT_FOUND, format!("nothing with id {id} in this project")),
            },
            None => project
                .entity_ids()
                .into_iter()
                .filter(|id| match project.get::<de_assembly::Instance>(*id) {
                    Some(i) => i.parent.is_none(),
                    None => project.get::<de_solid::Placement>(*id).is_some() && (project.get::<de_shape::ShapeProgram>(*id).is_some() || project.get::<de_solid::BoxFeature>(*id).is_some()),
                })
                .collect(),
        };
        // One thing asked for by id may be a component (a definition placed only through instances): drawn as it is.
        let one = query.get("id").is_some_and(|s| !s.is_empty());
        ids.into_iter().filter_map(|id| if one { crate::exact_files::checked(&project, id) } else { crate::source(&project, id) }).collect::<Vec<_>>()
    };
    if sources.is_empty() {
        return fail(StatusCode::NOT_FOUND, "nothing placed to look at".into());
    }
    // Against a canvas: the model drawn square to it over its photo, and how well the silhouettes match.
    if let Some(what) = query.get("canvas").filter(|s| !s.is_empty()) {
        let found = {
            let project = app.project.lock().unwrap();
            de_shape::canvas::find(project.world(), what)
        };
        let Some((_, canvas)) = found else { return fail(StatusCode::NOT_FOUND, format!("no canvas called “{what}” (./ise canvas list)")) };
        let json_only = query.get("format").is_some_and(|f| f == "json");
        let compared = tokio::task::spawn_blocking(move || -> Result<de_shape::silhouette::Comparison, String> {
            let mut meshes = Vec::new();
            for s in &sources {
                s.meshes(&mut meshes)?;
            }
            let o = de_shape::silhouette::Options { picture: !json_only, max_edge: w.max(h) as usize, ..Default::default() };
            de_shape::silhouette::compare(&canvas, &meshes, &o)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        return match compared {
            Ok(c) => {
                let numbers = serde_json::to_string(&c).unwrap_or_default();
                if json_only {
                    ([(header::CONTENT_TYPE, "application/json")], numbers).into_response()
                } else {
                    ([(header::CONTENT_TYPE, "image/png".to_string()), (header::HeaderName::from_static("x-ise-match"), ascii_json(&numbers))], c.png).into_response()
                }
            }
            Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        };
    }
    // A photo-like picture from an orbiting perspective camera (photogrammetry test sets).
    if let Some(orbit) = query.get("orbit") {
        let v: Vec<f64> = orbit.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if v.len() < 2 {
            return fail(StatusCode::BAD_REQUEST, "orbit is azimuth,elevation[,distance mm] in degrees".into());
        }
        let fov = query.get("fov").and_then(|s| s.parse::<f64>().ok()).unwrap_or(40.0).clamp(5.0, 120.0);
        let on = |k: &str, d: bool| query.get(k).map(|s| s != "0" && s != "false").unwrap_or(d);
        let scene = de_shape::render::PhotoScene {
            speckle: on("speckle", true),
            sheet: on("sheet", false).then(de_scan::markers::Sheet::default),
            light: query.get("light").and_then(|s| s.parse().ok()).unwrap_or(30.0),
        };
        let drawn = tokio::task::spawn_blocking(move || -> Result<de_shape::render::Photo, String> {
            let mut meshes = Vec::new();
            for s in &sources {
                s.meshes(&mut meshes)?;
            }
            let mut b = [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN];
            for m in &meshes {
                for p in m.positions.chunks_exact(3) {
                    for k in 0..3 {
                        b[k] = b[k].min(p[k] as f64);
                        b[k + 3] = b[k + 3].max(p[k] as f64);
                    }
                }
            }
            let centre = [(b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0, b[2] + (b[5] - b[2]) * 0.35];
            let diag = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
            let distance = v.get(2).copied().unwrap_or(diag * 2.6);
            let cam = de_shape::render::PhotoCamera::orbit(centre, v[0], v[1], distance, fov);
            de_shape::render::render_photo(&meshes, cam, w, h, &scene)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        return match drawn {
            Ok(p) => {
                let camera = json!({ "focal": p.focal, "centre": p.centre, "rotation": p.rotation, "translation": p.translation, "width": w, "height": h }).to_string();
                ([(header::CONTENT_TYPE, "image/png".to_string()), (header::HeaderName::from_static("x-ise-camera"), camera)], p.png).into_response()
            }
            Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        };
    }
    // `box=x0,y0,z0,x1,y1,z1`: frame only that region (mm), a close look at a detail.
    let frame = match query.get("box") {
        Some(b) => {
            let v: Vec<f64> = b.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if v.len() != 6 || v.iter().any(|x| !x.is_finite()) {
                return fail(StatusCode::BAD_REQUEST, "box is x0,y0,z0,x1,y1,z1 in mm".into());
            }
            Some([v[0].min(v[3]), v[1].min(v[4]), v[2].min(v[5]), v[0].max(v[3]), v[1].max(v[4]), v[2].max(v[5])])
        }
        None => None,
    };
    let drawn = tokio::task::spawn_blocking(move || -> Result<de_shape::render::Picture, String> {
        let mut meshes = Vec::new();
        for s in &sources {
            s.meshes(&mut meshes)?;
        }
        de_shape::render::render_framed(&meshes, view, w, h, frame)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match drawn {
        Ok(p) => {
            let b = p.bounds;
            let bounds = format!("{:.2},{:.2},{:.2},{:.2},{:.2},{:.2}", b[0], b[1], b[2], b[3], b[4], b[5]);
            (
                [(header::CONTENT_TYPE, "image/png".to_string()), (header::HeaderName::from_static("x-ise-mm-per-px"), format!("{:.5}", p.mm_per_px)), (header::HeaderName::from_static("x-ise-bounds"), bounds), (header::HeaderName::from_static("x-ise-triangles"), p.triangles.to_string())],
                p.png,
            )
                .into_response()
        }
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}
