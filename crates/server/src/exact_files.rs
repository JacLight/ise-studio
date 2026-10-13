//! Exact files besides STEP (compressed STEP, OpenCascade BREP) and the
//! geometry check (`GET /check-geometry/<id>`).

use crate::{Server, Source, source};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use de_core::StableId;
use de_shape::fabricate;
use serde_json::{Value, json};
use std::{collections::HashMap, io::Read, io::Write, sync::Arc};

/// The exact shapes to write: one part of a shape (at the origin) or the whole design, placed.
fn shapes(s: &Source, part: Option<&str>, maker: &mut fabricate::StepMaker) -> Result<(String, Vec<(String, de_occt::Shape, Option<[f64; 3]>)>), String> {
    if let Some(name) = part {
        let program = s.program.as_ref().ok_or("pick a part of a shape")?;
        let (parts, _) = maker.parts(program)?;
        let shape = parts.iter().find(|(n, _)| n == name).map(|(_, s)| *s).ok_or_else(|| format!("no part {name}"))?;
        let b = maker.arena().bounds(shape)?;
        let at_origin = maker.arena().translate(shape, [-b[0], -b[1], -b[2]])?;
        return Ok((name.to_string(), vec![(name.to_string(), at_origin, None)]));
    }
    let mut out = Vec::new();
    let mut faceted = Vec::new();
    s.exact(maker, &mut out, &mut faceted)?;
    if out.is_empty() {
        return Err("nothing to export".into());
    }
    Ok((s.name.clone(), out))
}

/// `.stpz` (gzip-compressed STEP, as FreeCAD writes it) and `.brep`
/// (OpenCascade's own exact format: one compound, no names): (file, mime, bytes).
pub fn export(s: &Source, part: Option<&str>, ext: &str, tolerance: f64) -> Result<(String, &'static str, Vec<u8>), String> {
    let mut maker = fabricate::StepMaker::new(tolerance);
    let (name, list) = shapes(s, part, &mut maker)?;
    let base = fabricate::file_name(&name);
    if ext == "brep" || ext == "brp" {
        let path = temp("brep");
        let only: Vec<de_occt::Shape> = list.iter().map(|(_, s, _)| *s).collect();
        maker.arena().write_brep(&only, &path)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&path);
        return Ok((format!("{base}.brep"), "application/octet-stream", bytes?));
    }
    let step = maker.file(&list, de_occt::CadFormat::Step)?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&step).map_err(|e| e.to_string())?;
    Ok((format!("{base}.stpz"), "application/gzip", gz.finish().map_err(|e| e.to_string())?))
}

fn temp(ext: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!("forma-{}-{}.{ext}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))
}

/// The file names Forma imports, and how: STEP as it is, compressed STEP
/// unpacked, BREP translated to STEP (so every copy of a project stores one kind).
pub fn importable(lower: &str) -> bool {
    [".step", ".stp", ".stpz", ".stepz", ".step.gz", ".stp.gz", ".brep", ".brp"].iter().any(|e| lower.ends_with(e))
}

/// The file's name without its (possibly double) extension.
pub fn stem(file: &str) -> &str {
    let lower = file.to_lowercase();
    for e in [".step.gz", ".stp.gz"] {
        if lower.ends_with(e) {
            return &file[..file.len() - e.len()];
        }
    }
    file.rsplit_once('.').map(|(n, _)| n).unwrap_or(file)
}

/// An uploaded file as STEP bytes.
pub fn to_step(lower: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    if lower.ends_with(".step") || lower.ends_with(".stp") {
        return Ok(body.to_vec());
    }
    if lower.ends_with(".brep") || lower.ends_with(".brp") {
        let (from, to) = (temp("brep"), temp("step"));
        std::fs::write(&from, body).map_err(|e| e.to_string())?;
        let mut a = de_occt::Arena::new();
        let r = a.read_brep(&from).and_then(|bodies| {
            let named: Vec<(String, de_occt::Shape)> = bodies.iter().enumerate().map(|(i, (n, s))| (if bodies.len() == 1 { n.clone() } else { format!("{n} {}", i + 1) }, *s)).collect();
            a.write_step(&named, &to)
        });
        let _ = std::fs::remove_file(&from);
        let bytes = r.and_then(|_| std::fs::read(&to).map_err(|e| e.to_string()));
        let _ = std::fs::remove_file(&to);
        return bytes;
    }
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(body).read_to_end(&mut out).map_err(|e| format!("not a compressed STEP file ({e})"))?;
    Ok(out)
}

/// `GET /check-geometry/<id>?project=…&part=…`: the exact kernel's check of
/// every part (as FreeCAD's Check geometry): validity, what booleans would
/// trip on (self-intersections, tiny edges, curves off their surfaces),
/// tolerances, free edges.
pub async fn check(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let s = {
        let project = app.project.lock().unwrap();
        match serde_json::from_value::<StableId>(json!(id)).ok().and_then(|i| source(&project, i)) {
            Some(s) => s,
            None => return fail(StatusCode::NOT_FOUND, format!("{id} is not a part")),
        }
    };
    let part = query.get("part").cloned();
    let r = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let mut maker = fabricate::StepMaker::new(0.01);
        let (name, list) = shapes(&s, part.as_deref(), &mut maker)?;
        let mut parts = Vec::new();
        let mut ok = true;
        for (n, shape, _) in list {
            let report = maker.arena().check_geometry(shape)?;
            ok &= report["valid"] == true && report["boolean_ready"] == true;
            parts.push(json!({ "name": n, "report": report }));
        }
        Ok(json!({ "name": name, "ok": ok, "parts": parts }))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// `GET /surfaces/<id>?project=…&part=…&pull=x,y,z&min=deg`: surface analysis
/// of every part, exactly: what each body is (solid or open sheet), the
/// continuity across every edge two of its faces share (gap, crease angle,
/// curvature jump), the tightest curvature, and draft against a pull
/// direction (faces within `min` degrees of square to it).
pub async fn surfaces(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let s = {
        let project = app.project.lock().unwrap();
        match serde_json::from_value::<StableId>(json!(id)).ok().and_then(|i| source(&project, i)) {
            Some(s) => s,
            None => return fail(StatusCode::NOT_FOUND, format!("{id} is not a part")),
        }
    };
    let part = query.get("part").cloned();
    let pull: Vec<f64> = query.get("pull").map(|p| p.split(',').filter_map(|v| v.trim().parse().ok()).collect()).unwrap_or_else(|| vec![0.0, 0.0, 1.0]);
    let min: f64 = query.get("min").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let r = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        if pull.len() != 3 {
            return Err("pull is x,y,z".into());
        }
        let l = (pull[0] * pull[0] + pull[1] * pull[1] + pull[2] * pull[2]).sqrt().max(1e-12);
        let p = [pull[0] / l, pull[1] / l, pull[2] / l];
        let mut maker = fabricate::StepMaker::new(0.01);
        let (name, list) = shapes(&s, part.as_deref(), &mut maker)?;
        let mut parts = Vec::new();
        for (n, shape, _) in list {
            let a = maker.arena();
            let body = a.body_info(shape)?;
            let cont = a.continuity(shape, None, 24)?;
            let mut edges: Vec<Value> = cont["edges"].as_array().cloned().unwrap_or_default();
            // The worst seams first (by crease, then curvature jump).
            edges.sort_by(|x, y| {
                let k = |e: &Value| (e["g1"].as_f64().unwrap_or(0.0), e["g2"].as_f64().unwrap_or(0.0));
                k(y).partial_cmp(&k(x)).unwrap_or(std::cmp::Ordering::Equal)
            });
            let smooth = edges.iter().filter(|e| e["g1"].as_f64().unwrap_or(0.0) < 0.1).count();
            let samples = a.face_samples(shape, 10)?;
            let tight = samples.iter().map(|q| q.k1.abs().max(q.k2.abs())).fold(0.0f64, f64::max);
            let mut square = std::collections::BTreeMap::<usize, f64>::new();
            for q in &samples {
                let d = (q.n[0] * p[0] + q.n[1] * p[1] + q.n[2] * p[2]).clamp(-1.0, 1.0).asin().to_degrees().abs();
                let e = square.entry(q.face).or_insert(f64::INFINITY);
                *e = e.min(d);
            }
            let under: Vec<usize> = square.iter().filter(|(_, d)| **d < min).map(|(f, _)| *f).collect();
            parts.push(json!({
                "name": n,
                "body": { "solids": body.solids, "faces": body.faces, "edges": body.edges, "free_edges": body.free_edges, "closed": body.closed },
                "continuity": { "max": cont["max"], "edges": edges.len(), "smooth": smooth, "worst": edges.iter().take(8).cloned().collect::<Vec<_>>() },
                "curvature": { "min_radius": if tight > 1e-12 { json!(1.0 / tight) } else { Value::Null } },
                "draft": { "pull": p, "min": min, "under": under },
            }));
        }
        Ok(json!({ "name": name, "parts": parts }))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}
