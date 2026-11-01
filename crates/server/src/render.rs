//! Photoreal renders on request: the engine has no GPU of its own, so it asks
//! an open view of the project to path-trace the model (as it is shown, from
//! its camera) and send the PNG back.
//!
//! - `POST /render?project=…&width=2000&height=1125&samples=256`: waits for
//!   the image (409 when no view of the project is open).
//! - `POST /render/<job>?take=1` (from a view): takes the job (204), or 409
//!   when another view took it first. A view takes a picture only while it is
//!   on screen (a background tab is throttled and can stall it for minutes);
//!   when no view takes the job within 10 s the request answers 409, so the
//!   CLI renders in a headless browser of its own. Not every connection is a
//!   view that can render: scripts, the CLI and MCP connect too.
//! - `POST /render/<job>` (from the view that took it): the PNG, or `?error=`.

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

/// Jobs no view has taken yet.
fn untaken() -> &'static Mutex<HashMap<String, oneshot::Sender<()>>> {
    static UNTAKEN: OnceLock<Mutex<HashMap<String, oneshot::Sender<()>>>> = OnceLock::new();
    UNTAKEN.get_or_init(Default::default)
}

/// How long a job waits for a view to take it.
const TAKE_WITHIN: Duration = Duration::from_secs(10);
/// How long the view asked for first has to take a render before any open view may.
const THEIRS_WITHIN: Duration = Duration::from_secs(4);

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
        return fail(StatusCode::CONFLICT, "no view of this project is open to render in: open it in the browser (or let ./ise render start a headless one)".into());
    }
    let job = de_core::StableId::new().to_string();
    let (tx, rx) = oneshot::channel();
    let (take_tx, mut take_rx) = oneshot::channel();
    jobs().lock().unwrap().insert(job.clone(), tx);
    untaken().lock().unwrap().insert(job.clone(), take_tx);
    let value = json!({ "job": job, "width": width, "height": height, "samples": samples, "format": if glb { "glb" } else { "png" } }).to_string();
    // The view of the person asked for (by default the one the AI is talking to) renders it, so the
    // picture is of what they see, not of another open view (a teammate's, cut or turned another way);
    // when theirs does not take it (closed, or in a background tab), any open view.
    let ask = |to: Option<&str>| app.events.send(json!({ "type": "view", "to": to, "action": "render", "target": null, "value": value }).to_string().into());
    let first = query.get("user").cloned().or_else(|| app.ai.audience()).filter(|u| u != "*");
    let mut taken = false;
    if let Some(user) = &first {
        let _ = ask(Some(user));
        taken = matches!(tokio::time::timeout(THEIRS_WITHIN, &mut take_rx).await, Ok(Ok(())));
    }
    if !taken {
        let _ = ask(None);
        taken = matches!(tokio::time::timeout(TAKE_WITHIN, &mut take_rx).await, Ok(Ok(())));
    }
    if !taken {
        untaken().lock().unwrap().remove(&job);
        jobs().lock().unwrap().remove(&job);
        return fail(
            StatusCode::CONFLICT,
            "no open view took the render (its tab is in the background, or the connections are scripts, not views): bring the view to the front, or let ./ise render start a headless one".into(),
        );
    }
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
    if query.contains_key("take") {
        // The first view to take the job renders it; the others leave it.
        let take = untaken().lock().unwrap().remove(&job);
        return match take.map(|t| t.send(())) {
            Some(Ok(())) => StatusCode::NO_CONTENT.into_response(),
            _ => (StatusCode::CONFLICT, "another view took this render, or it gave up").into_response(),
        };
    }
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
