//! Photoreal renders on request: the engine has no GPU of its own, so it asks
//! an open view of the project to path-trace the model (as it is shown, from
//! its camera) and send the PNG back.
//!
//! - `POST /render?project=…&width=2000&height=1125&samples=256`: waits for
//!   the image (409 when no view of the project is open).
//! - `POST /render/<job>` (from the view): the PNG, or `?error=` if it failed.

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::json;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::oneshot;

type Reply = Result<Vec<u8>, String>;

fn jobs() -> &'static Mutex<HashMap<String, oneshot::Sender<Reply>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, oneshot::Sender<Reply>>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

pub async fn request(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let num = |k: &str, d: u32, lo: u32, hi: u32| query.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(d).clamp(lo, hi);
    let width = num("width", 1920, 64, 8192);
    let height = num("height", width * 9 / 16, 64, 8192);
    let samples = num("samples", 256, 1, 4096);
    // `format=glb`: the view sends the shown model as a .glb for a website instead of a picture.
    let glb = query.get("format").is_some_and(|f| f == "glb");
    if app.sessions.load(Ordering::Relaxed) == 0 {
        return fail(StatusCode::CONFLICT, "no view of this project is open to render in: open it in the browser (or let ./forma render start a headless one)".into());
    }
    let job = de_core::StableId::new().to_string();
    let (tx, rx) = oneshot::channel();
    jobs().lock().unwrap().insert(job.clone(), tx);
    let value = json!({ "job": job, "width": width, "height": height, "samples": samples, "format": if glb { "glb" } else { "png" } }).to_string();
    let _ = app.events.send(json!({ "type": "view", "to": query.get("user"), "action": "render", "target": null, "value": value }).to_string().into());
    let minutes = query.get("timeout").and_then(|v| v.parse::<u64>().ok()).unwrap_or(15).clamp(1, 120);
    let result = tokio::time::timeout(Duration::from_secs(minutes * 60), rx).await;
    jobs().lock().unwrap().remove(&job);
    match result {
        Ok(Ok(Ok(bytes))) => (StatusCode::OK, [(header::CONTENT_TYPE, if glb { "model/gltf-binary" } else { "image/png" })], bytes).into_response(),
        Ok(Ok(Err(e))) => fail(StatusCode::UNPROCESSABLE_ENTITY, format!("the view could not render: {e}")),
        Ok(Err(_)) => fail(StatusCode::INTERNAL_SERVER_ERROR, "the render was dropped".into()),
        Err(_) => fail(StatusCode::GATEWAY_TIMEOUT, format!("no image after {minutes} minutes (fewer samples, or a smaller size?)")),
    }
}

pub async fn deliver(Path(job): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let Some(tx) = jobs().lock().unwrap().remove(&job) else {
        // Another view answered first, or the request gave up.
        return (StatusCode::NOT_FOUND, "no such render waiting").into_response();
    };
    let reply = match query.get("error") {
        Some(e) => Err(e.clone()),
        // A picture, or a model for the web (binary glTF).
        None if body.starts_with(b"\x89PNG") || body.starts_with(b"glTF") => Ok(body.to_vec()),
        None => Err("the view sent something that is neither a PNG nor a .glb".into()),
    };
    let _ = tx.send(reply);
    StatusCode::NO_CONTENT.into_response()
}
