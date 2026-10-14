//! Read-only sessions, and view-only links anyone can open in a browser.
//!
//! A read-only session shows a project the way the web app expects (a
//! snapshot, then live changes) but refuses every change. It serves two
//! things:
//!
//! - a look at a past version on this computer (`/ws?project=…&version=…`);
//! - a view-only link (`/view/<token>`), served on this engine's network
//!   address (and its public URL, if one is set) to people without Iṣẹ́ Studio.
//!
//! A link is a long random token naming one project (or one version of it).
//! Everything a link opens answers with that project only: there is no
//! project parameter to change, no other route is reachable from the network
//! (the rest of the API stays local-only), and the session takes no
//! commands. Program source code, the AI conversation, history and version
//! notes are left out of what a link shows. Links can expire, and are turned
//! off by deleting them; open viewers are dropped within seconds.

use crate::{App, Server, history, view};
use axum::{
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{Persist, Project, StableId, Value, Version, json};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc};

/// One view-only link.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Share {
    pub id: String,
    /// The secret in the link (kept on this computer only).
    pub token: String,
    pub project: String,
    /// What it is for ("Supplier quote"), shown in the list.
    #[serde(default)]
    pub label: String,
    /// When it was made, and when it stops working (s since 1970).
    pub created: u64,
    #[serde(default)]
    pub expires: Option<u64>,
    /// A named version to show (frozen) instead of the project as it is.
    #[serde(default)]
    pub version: Option<StableId>,
}

impl Share {
    pub fn live(&self, now: u64) -> bool {
        self.expires.is_none_or(|t| now < t)
    }
}

/// Every link this engine serves (`.shares.json` in the projects folder).
#[derive(Default)]
pub struct Shares {
    pub list: Vec<Share>,
    path: std::path::PathBuf,
}

impl Shares {
    pub fn load(dir: &std::path::Path) -> Shares {
        let path = dir.join(".shares.json");
        let list = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Shares { list, path }
    }

    fn save(&self) -> std::io::Result<()> {
        std::fs::write(&self.path, serde_json::to_string_pretty(&self.list).unwrap())
    }

    /// The link with this token, if it is still on and not expired.
    pub fn find(&self, token: &str, now: u64) -> Option<Share> {
        if token.len() < 32 {
            return None;
        }
        self.list.iter().find(|s| constant_eq(&s.token, token) && s.live(now)).cloned()
    }
}

/// Compares secrets without stopping at the first difference.
fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn secret() -> String {
    let mut b = [0u8; 24];
    getrandom::fill(&mut b).expect("the system's random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---- Read-only sessions ----------------------------------------------------

/// What a read-only session shows.
pub enum Feed {
    /// A past state (a version): never changes.
    Frozen(Arc<Mutex<Project>>),
    /// The project as it is, following every change.
    Live(Arc<App>),
}

pub struct ReadOnly {
    pub feed: Feed,
    /// "version" (a look on this computer) or "share" (a link).
    pub kind: &'static str,
    /// What is being shown ("v1.2", or the project's title).
    pub label: String,
    pub title: String,
    pub version: Option<StableId>,
    /// For a link: its token (re-checked while open, so turning it off drops the viewer).
    pub token: Option<(Arc<Server>, String)>,
}

impl ReadOnly {
    fn sanitize(&self) -> bool {
        self.kind == "share"
    }
}

/// Takes out of an entity what a view-only link does not show: program source
/// code and version notes (the model, names, materials and comments stay).
fn clean_entity(mut e: Value) -> Option<Value> {
    let components = e.get_mut("components")?.as_object_mut()?;
    if components.contains_key(Version::KIND) {
        return None;
    }
    for c in components.values_mut() {
        if let Some(o) = c.as_object_mut()
            && o.get("source").is_some_and(Value::is_string)
        {
            o.insert("source".into(), json!(""));
        }
    }
    if let Some(o) = e.as_object_mut() {
        o.remove("formulas");
    }
    Some(e)
}

fn entities_of(project: &Project, clean: bool) -> Vec<Value> {
    project
        .entity_ids()
        .into_iter()
        .filter_map(|id| view(project, id))
        .filter_map(|v| {
            let v = serde_json::to_value(v).ok()?;
            if clean { clean_entity(v) } else { Some(v) }
        })
        .collect()
}

fn snapshot(ro: &ReadOnly, project: &Project) -> String {
    json!({
        "type": "snapshot",
        "project_id": project.id(),
        "revision": project.revision(),
        "user_id": "viewer",
        "session_id": StableId::new(),
        "sessions": 0,
        "commands": [],
        "entities": entities_of(project, ro.sanitize()),
        "history": [],
        "ai": { "available": false, "busy": false, "chat": [] },
        "read_only": { "kind": ro.kind, "label": ro.label, "title": ro.title, "version": ro.version },
    })
    .to_string()
}

/// The answer to anything a read-only session is sent: a fresh snapshot when
/// asked for one, a refusal for everything that has a request id (commits,
/// previews, undo, AI…), and nothing for the rest.
pub fn answer(ro: &ReadOnly, project: &Project, text: &str) -> Option<String> {
    let m: Value = serde_json::from_str(text).ok()?;
    if m["type"] == "snapshot" {
        return Some(snapshot(ro, project));
    }
    let request_id = m.get("request_id")?.clone();
    let message = if ro.kind == "share" { "This is a view-only link: nothing can be changed here." } else { "This is a past version, shown read-only: restore it or branch from it to work on it." };
    Some(json!({ "type": "result", "request_id": request_id, "ok": false, "error": { "code": "READ_ONLY", "message": message } }).to_string())
}

/// A live change, as a read-only viewer may see it (or nothing).
fn pass_on(ro: &ReadOnly, event: &str) -> Option<String> {
    let mut m: Value = serde_json::from_str(event).ok()?;
    match m["type"].as_str()? {
        "changed" | "refreshed" => {
            if ro.sanitize() {
                let list = m.get_mut("entities")?.as_array_mut()?;
                let ids: Vec<Value> = list.iter().filter(|e| e.get("components").is_some_and(|c| c.get(Version::KIND).is_some())).map(|e| e["id"].clone()).collect();
                let kept: Vec<Value> = std::mem::take(list).into_iter().filter_map(clean_entity).collect();
                *list = kept;
                if let Some(removed) = m.get_mut("removed").and_then(Value::as_array_mut) {
                    removed.extend(ids);
                }
            }
            if let Some(actor) = m.get_mut("actor") {
                *actor = json!({ "user_id": actor["user_id"], "session_id": "" });
            }
            Some(m.to_string())
        }
        "resync" => Some(m.to_string()),
        _ => None,
    }
}

/// Runs a read-only session until the viewer leaves (or the link is turned off).
pub async fn session(socket: WebSocket, ro: ReadOnly) {
    let (mut sink, mut stream) = socket.split();
    let (out, mut queue) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(text) = queue.recv().await {
            let close = text == "\u{0}close";
            let message = if close { Message::Close(None) } else { Message::Text(text.into()) };
            if sink.send(message).await.is_err() || close {
                break;
            }
        }
    });
    let ro = Arc::new(ro);
    // A live view subscribes first, so no change falls between the snapshot and the stream.
    let follower = match &ro.feed {
        Feed::Frozen(_) => None,
        Feed::Live(app) => Some(tokio::spawn(follow(ro.clone(), app.events.subscribe(), out.clone()))),
    };
    let current = |ro: &ReadOnly, f: &dyn Fn(&Project) -> Option<String>| -> Option<String> {
        match &ro.feed {
            Feed::Frozen(p) => f(&p.lock().unwrap()),
            Feed::Live(app) => f(&app.project.lock().unwrap()),
        }
    };
    let _ = out.send(current(&ro, &|p| Some(snapshot(&ro, p))).unwrap_or_default());
    // A link is checked again every few seconds: turned off or expired, the viewer is dropped.
    let watch = ro.token.as_ref().map(|(server, token)| {
        let (server, token, o) = (server.clone(), token.clone(), out.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if server.shares.lock().unwrap().find(&token, crate::peers::now()).is_none() {
                    let _ = o.send(json!({ "type": "error", "message": "This link was turned off or has expired." }).to_string());
                    let _ = o.send("\u{0}close".into());
                    break;
                }
            }
        })
    });
    while let Some(Ok(message)) = stream.next().await {
        let Message::Text(text) = message else { continue };
        if text.len() > 64 * 1024 {
            continue;
        }
        if let Some(reply) = current(&ro, &|p| answer(&ro, p, text.as_str())) {
            if out.send(reply).is_err() {
                break;
            }
        }
    }
    if let Some(w) = watch {
        w.abort();
    }
    if let Some(f) = follower {
        f.abort();
    }
    writer.abort();
}

async fn follow(ro: Arc<ReadOnly>, mut events: broadcast::Receiver<Arc<str>>, out: mpsc::UnboundedSender<String>) {
    loop {
        match events.recv().await {
            Ok(text) => {
                if text.contains("\"moved\"") && serde_json::from_str::<Value>(&text).is_ok_and(|m| m["type"] == "moved") {
                    let _ = out.send(json!({ "type": "error", "message": "This project is no longer shared here." }).to_string());
                    let _ = out.send("\u{0}close".into());
                    break;
                }
                if let Some(m) = pass_on(&ro, &text)
                    && out.send(m).is_err()
                {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                if out.send(json!({ "type": "resync" }).to_string()).is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

// ---- Managing links (this computer only) ------------------------------------

/// Where a link can be opened: this engine's network address(es), and its
/// public URL if the project has one.
fn urls(server: &Server, app: &App, token: &str) -> Vec<String> {
    let mut out: Vec<String> = crate::peers::lan_ips().into_iter().map(|ip| format!("http://{ip}:{}/view/{token}", server.port)).collect();
    if let Some(u) = app.team.lock().unwrap().public_url.as_ref().map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty()) {
        let base = u.trim_end_matches("/peer");
        out.insert(0, format!("{base}/view/{token}"));
    }
    if out.is_empty() {
        out.push(format!("http://127.0.0.1:{}/view/{token}", server.port));
    }
    out
}

fn listing(server: &Server, name: &str, app: &App) -> Value {
    let now = crate::peers::now();
    let project = app.project.lock().unwrap();
    let links: Vec<Value> = server
        .shares
        .lock()
        .unwrap()
        .list
        .iter()
        .filter(|s| s.project == name)
        .map(|s| {
            let version = s.version.and_then(|v| project.get::<Version>(v).map(|x| x.name.clone()));
            json!({
                "id": s.id, "label": s.label, "created": s.created * 1000, "expires": s.expires.map(|t| t * 1000),
                "expired": !s.live(now), "version": s.version, "version_name": version, "urls": urls(server, app, &s.token),
            })
        })
        .collect();
    json!({ "links": links })
}

fn app_of(server: &Server, q: &HashMap<String, String>) -> Result<(String, Arc<App>), axum::response::Response> {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    server.project(Some(&name)).map(|app| (name, app)).map_err(crate::bad_request)
}

/// `GET /shares?project=…`: the project's view-only links.
pub async fn list(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> axum::response::Response {
    match app_of(&server, &q) {
        Ok((name, app)) => axum::Json(listing(&server, &name, &app)).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
pub struct NewShare {
    #[serde(default)]
    label: String,
    /// Days until it stops working (none: until turned off).
    #[serde(default)]
    days: Option<f64>,
    #[serde(default)]
    version: Option<StableId>,
}

/// `POST /shares?project=…` `{ label?, days?, version? }`: a new view-only link.
pub async fn create(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>, axum::Json(body): axum::Json<NewShare>) -> axum::response::Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Some(v) = body.version
        && app.project.lock().unwrap().get::<Version>(v).is_none()
    {
        return crate::bad_request("that version is not in this project");
    }
    let expires = match body.days {
        Some(d) if d.is_finite() && d > 0.0 && d <= 3650.0 => Some(crate::peers::now() + (d * 86400.0) as u64),
        Some(_) => return crate::bad_request("a link lasts from a moment to ten years (days)"),
        None => None,
    };
    let share = Share { id: StableId::new().to_string(), token: secret(), project: name.clone(), label: body.label.trim().chars().take(80).collect(), created: crate::peers::now(), expires, version: body.version };
    {
        let mut shares = server.shares.lock().unwrap();
        shares.list.push(share.clone());
        if let Err(e) = shares.save() {
            shares.list.pop();
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    let mut out = listing(&server, &name, &app);
    out["created"] = json!(share.id);
    axum::Json(out).into_response()
}

#[derive(Deserialize)]
pub struct Revoke {
    id: String,
}

/// `POST /shares/revoke?project=…` `{ id }`: turns a link off for good (open viewers drop).
pub async fn revoke(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>, axum::Json(body): axum::Json<Revoke>) -> axum::response::Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    {
        let mut shares = server.shares.lock().unwrap();
        let before = shares.list.len();
        shares.list.retain(|s| !(s.id == body.id && s.project == name));
        if shares.list.len() == before {
            return crate::bad_request("no such link");
        }
        if let Err(e) = shares.save() {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    axum::Json(listing(&server, &name, &app)).into_response()
}

// ---- What the network reaches ----------------------------------------------

const GONE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Link not available · ISE Studio</title><style>body{margin:0;min-height:100vh;display:grid;place-items:center;font:15px/1.5 system-ui,sans-serif;background:#fafbfc;color:#1d232b}main{max-width:340px;padding:24px;text-align:center}h1{font-size:18px;margin:0 0 6px}p{margin:0;color:#5d6773}@media(prefers-color-scheme:dark){body{background:#14171b;color:#e8ebef}p{color:#9aa4ae}}</style></head><body><main><h1>This link isn't available</h1><p>It was turned off or has expired. Ask whoever shared it for a new one.</p></main></body></html>";

fn private_headers(mut r: axum::response::Response) -> axum::response::Response {
    let h = r.headers_mut();
    // The token is in the address: never send it on to other sites, and keep the page out of search.
    h.insert(header::REFERRER_POLICY, header::HeaderValue::from_static("no-referrer"));
    h.insert("x-robots-tag", header::HeaderValue::from_static("noindex, nofollow"));
    h.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    h.insert("x-frame-options", header::HeaderValue::from_static("DENY"));
    r
}

/// `GET /view/<token>`: the web app, opening as a viewer (or a short page if
/// the link is off).
pub async fn page(State(server): State<Arc<Server>>, Path(token): Path<String>) -> axum::response::Response {
    let ok = server.shares.lock().unwrap().find(&token, crate::peers::now()).is_some();
    let r = match (ok, std::fs::read_to_string(server.web.join("index.html"))) {
        (true, Ok(html)) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response(),
        (true, Err(_)) => (StatusCode::SERVICE_UNAVAILABLE, "the viewer is not built on this engine").into_response(),
        (false, _) => (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], GONE).into_response(),
    };
    private_headers(r)
}

/// `GET /view/<token>/ws`: the link's read-only session, for its project only.
pub async fn socket(State(server): State<Arc<Server>>, Path(token): Path<String>, upgrade: WebSocketUpgrade) -> axum::response::Response {
    let Some(share) = server.shares.lock().unwrap().find(&token, crate::peers::now()) else {
        return (StatusCode::NOT_FOUND, "This link was turned off or has expired.").into_response();
    };
    if !server.dir.join(format!("{}.jsonl", share.project)).exists() {
        return (StatusCode::NOT_FOUND, "This project is no longer shared here.").into_response();
    }
    let app = match server.project(Some(&share.project)) {
        Ok(app) => app,
        Err(_) => return (StatusCode::NOT_FOUND, "This project is no longer shared here.").into_response(),
    };
    let title = crate::info_of(&server.dir, &share.project)["title"].as_str().map(str::to_string).unwrap_or_else(|| crate::pretty_name(&share.project));
    let (feed, label) = match share.version {
        None => (Feed::Live(app), title.clone()),
        Some(v) => {
            let found = {
                let p = app.project.lock().unwrap();
                history::resolve(&p, &history::Which { version: Some(v), revision: None })
            };
            let Ok((seen, name)) = found else { return (StatusCode::NOT_FOUND, "The version this link shows is gone.").into_response() };
            let a = app.clone();
            match tokio::task::spawn_blocking(move || history::look(&a, &seen)).await {
                Ok(Ok(p)) => (Feed::Frozen(p), name),
                _ => return (StatusCode::INTERNAL_SERVER_ERROR, "could not show that version").into_response(),
            }
        }
    };
    let ro = ReadOnly { feed, kind: "share", label, title, version: share.version, token: Some((server.clone(), token)) };
    upgrade.max_message_size(64 * 1024).on_upgrade(move |socket| session(socket, ro))
}
