//! Local engine server for development: projects in a folder (one
//! append-only file each, opened on first use), a WebSocket API for the
//! browser UI, and static hosting of the UI.
//!
//! `cargo run -p de-server -- [--project FILE] [--port 7420] [--web DIR]`
//!
//! `--project` names the default project; its folder holds the others.
//! Clients pick a project with `?project=<name>` on `/ws` and `/export`.

mod ai;
mod machines;
mod peers;

use axum::{
    Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use de_core::{
    Actor, ChangeSet, CommandError, Envelope, ErrorCode, Project, Registry, Revision, SCHEMA_VERSION, StableId,
    Status, Value, WireChangeSet, json,
};
use de_intent::{IntentPlugin, RequirementResult};
use de_shape::{ShapePlugin, ShapeProgram, ShapeSolid, ShapeStats, fabricate};
use de_solid::{DisplayMesh, MassProperties, SolidPlugin};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{broadcast, mpsc};
use tower_http::services::{ServeDir, ServeFile};

/// Every project in the folder, opened on first use and kept open.
struct Server {
    /// This engine's permanent name among peers (stamped on every change made here).
    engine: String,
    /// The port this engine listens on (for the addresses peers are given).
    port: u16,
    ai: Arc<ai::AiConfig>,
    dir: PathBuf,
    default: String,
    registry: Arc<Registry>,
    open: Mutex<HashMap<String, Arc<App>>>,
    /// Drawings already laid out, by what they were made from (exact
    /// evaluation of a big part takes a while; preview and download share one).
    drawings: Mutex<HashMap<u64, Arc<de_shape::drawing::Drawing>>>,
    machines: machines::MachineQueries,
}

/// Project names are file names: lowercase letters, digits and dashes.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !name.starts_with('-')
}

/// The files of a project, by what follows its name.
const PROJECT_FILES: [&str; 5] = ["jsonl", "thumb.png", "conflicts.json", "team.json", "ai"];

impl Server {
    /// Takes a project out of use (before its files move): its tabs are told
    /// where it went, and it stops writing and syncing.
    fn close(&self, name: &str, to: Option<&str>) {
        if let Some(app) = self.open.lock().unwrap().remove(name) {
            app.closed.store(true, Ordering::SeqCst);
            let _ = app.events.send(json!({ "type": "moved", "to": to }).to_string().into());
        }
    }

    fn project(&self, name: Option<&String>) -> Result<Arc<App>, String> {
        let name = name.filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| self.default.clone());
        if !valid_name(&name) {
            return Err(format!("“{name}” is not a project name (use lowercase letters, digits and dashes)"));
        }
        let mut open = self.open.lock().unwrap();
        if let Some(app) = open.get(&name) {
            return Ok(app.clone());
        }
        let path = self.dir.join(format!("{name}.jsonl"));
        let project = load(&name, &path, self.registry.clone(), &self.engine).map_err(|e| e.to_string())?;
        println!("opened project {name} at revision {} ({} entities)", project.revision().0, project.entity_count());
        let log = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| e.to_string())?;
        let (events, _) = broadcast::channel(1024);
        let (shared, _) = broadcast::channel(4096);
        let ai = Arc::new(ai::Ai::new(self.ai.clone(), &name, &self.dir));
        let team = peers::Team::load(&self.dir, &name);
        let app = Arc::new(App {
            project: Mutex::new(project),
            log: Mutex::new(log),
            events,
            sessions: AtomicUsize::new(0),
            ai,
            team: Mutex::new(team),
            links: Mutex::new(peers::Links::default()),
            shared,
            dialing: AtomicBool::new(false),
            conflicts: Mutex::new(std::fs::read_to_string(self.dir.join(format!("{name}.conflicts.json"))).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()),
            conflicts_path: self.dir.join(format!("{name}.conflicts.json")),
            closed: AtomicBool::new(false),
            assets: Mutex::new(peers::Assets::default()),
        });
        open.insert(name, app.clone());
        Ok(app)
    }
}

#[derive(Deserialize, Default)]
struct ProjectTo {
    #[serde(default)]
    to: Option<String>,
}

/// How a name reads when no title was given ("miami-tower" → "Miami tower").
fn pretty_name(name: &str) -> String {
    let s = name.replace('-', " ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn bad_request(text: impl Into<String>) -> axum::response::Response {
    (StatusCode::UNPROCESSABLE_ENTITY, text.into()).into_response()
}

/// A project's files that exist: (suffix, path).
fn project_files(dir: &std::path::Path, name: &str) -> Vec<(&'static str, PathBuf)> {
    PROJECT_FILES.iter().map(|s| (*s, dir.join(format!("{name}.{s}")))).filter(|(_, p)| p.exists()).collect()
}


/// `POST /projects/{name}/info` `{ title?, description?, tags? }`. The name
/// (the project's files, links, and how teammates' computers know it) never
/// changes; these are kept in the project, so they sync and undo.
async fn project_info(State(server): State<Arc<Server>>, Path(name): Path<String>, axum::Json(body): axum::Json<Value>) -> axum::response::Response {
    if !valid_name(&name) || !server.dir.join(format!("{name}.jsonl")).exists() {
        return bad_request(format!("no project called {name}"));
    }
    let app = match server.project(Some(&name)) {
        Ok(app) => app,
        Err(e) => return bad_request(e),
    };
    match tokio::task::spawn_blocking(move || app.commit_here("project.set_info", body)).await {
        Ok(Ok(_)) => axum::Json(json!({ "name": name })).into_response(),
        Ok(Err(e)) => bad_request(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// A project's title, description and tags, read from its log without opening it.
fn info_of(dir: &std::path::Path, name: &str) -> Value {
    let mut info = json!({ "title": null, "description": "", "tags": [] });
    let Ok(file) = File::open(dir.join(format!("{name}.jsonl"))) else { return info };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.contains("\"core.project\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        for op in v["ops"].as_array().into_iter().flatten() {
            if op["kind"] == "core.project" && op["after"].is_object() {
                info = json!({ "title": op["after"]["title"], "description": op["after"]["description"], "tags": op["after"]["tags"] });
            }
        }
    }
    info
}

/// `POST /projects/{name}/duplicate` `{ to? }`: a private copy (no team, no chat).
async fn project_duplicate(State(server): State<Arc<Server>>, Path(name): Path<String>, body: Option<axum::Json<ProjectTo>>) -> axum::response::Response {
    if !valid_name(&name) || !server.dir.join(format!("{name}.jsonl")).exists() {
        return bad_request(format!("no project called {name}"));
    }
    let to = match body.and_then(|b| b.0.to) {
        Some(to) if valid_name(&to) => to,
        Some(_) => return bad_request("use lowercase letters, digits and dashes"),
        None => {
            let stem = format!("{}-copy", name.trim_end_matches("-copy"));
            let mut n = 1;
            let mut to = stem.clone();
            while server.dir.join(format!("{to}.jsonl")).exists() {
                n += 1;
                to = format!("{stem}-{n}");
            }
            to
        }
    };
    if server.dir.join(format!("{to}.jsonl")).exists() {
        return bad_request("there is already a project called that");
    }
    // The log is append-only and synced line by line: a copy taken now is whole.
    for suffix in ["jsonl", "thumb.png"] {
        let from = server.dir.join(format!("{name}.{suffix}"));
        if from.exists()
            && let Err(e) = std::fs::copy(&from, server.dir.join(format!("{to}.{suffix}")))
        {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    // The copy says it is one.
    let title = info_of(&server.dir, &name)["title"].as_str().map(str::to_string).unwrap_or_else(|| pretty_name(&name));
    if let Ok(app) = server.project(Some(&to)) {
        let _ = tokio::task::spawn_blocking(move || app.commit_here("project.set_info", json!({ "title": format!("{title} copy") }))).await;
    }
    axum::Json(json!({ "name": to })).into_response()
}

/// `POST /projects/{name}/delete`: into `.trash`, from where it can be restored.
async fn project_delete(State(server): State<Arc<Server>>, Path(name): Path<String>) -> axum::response::Response {
    if !valid_name(&name) || !server.dir.join(format!("{name}.jsonl")).exists() {
        return bad_request(format!("no project called {name}"));
    }
    if name == server.default {
        return bad_request("the starting project cannot be deleted");
    }
    let bin = server.dir.join(".trash").join(format!("{name}~{}", peers::now()));
    if let Err(e) = std::fs::create_dir_all(&bin) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    server.close(&name, None);
    for (suffix, path) in project_files(&server.dir, &name) {
        if let Err(e) = std::fs::rename(&path, bin.join(format!("{name}.{suffix}"))) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    axum::Json(json!({ "trashed": bin.file_name().and_then(|f| f.to_str()) })).into_response()
}

/// `GET /projects/trash`: deleted projects, newest first.
async fn trash_list(State(server): State<Arc<Server>>) -> impl IntoResponse {
    let mut list: Vec<Value> = std::fs::read_dir(server.dir.join(".trash"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let entry = e.file_name().to_str()?.to_string();
            let (name, at) = entry.rsplit_once('~')?;
            let at: u64 = at.parse().ok()?;
            let revisions = std::fs::read(e.path().join(format!("{name}.jsonl"))).map(|b| b.iter().filter(|c| **c == b'\n').count()).unwrap_or(0);
            let title = info_of(&e.path(), name)["title"].as_str().map(str::to_string).unwrap_or_else(|| pretty_name(name));
            Some(json!({ "entry": entry, "name": name, "title": title, "deleted": at * 1000, "revisions": revisions, "taken": server.dir.join(format!("{name}.jsonl")).exists() }))
        })
        .collect();
    list.sort_by_key(|v| std::cmp::Reverse(v["deleted"].as_u64().unwrap_or(0)));
    axum::Json(list)
}

/// `POST /projects/trash/{entry}/restore` `{ to? }`: back where it was (or under a new name).
async fn trash_restore(State(server): State<Arc<Server>>, Path(entry): Path<String>, body: Option<axum::Json<ProjectTo>>) -> axum::response::Response {
    let Some((name, _)) = entry.rsplit_once('~') else { return bad_request("not a deleted project") };
    let bin = server.dir.join(".trash").join(&entry);
    if !valid_name(name) || entry.contains('/') || !bin.join(format!("{name}.jsonl")).exists() {
        return bad_request("not a deleted project");
    }
    let to = body.and_then(|b| b.0.to).unwrap_or_else(|| name.to_string());
    if !valid_name(&to) {
        return bad_request("use lowercase letters, digits and dashes");
    }
    if server.dir.join(format!("{to}.jsonl")).exists() {
        return bad_request("a project with that name exists: restore it under another name");
    }
    for suffix in PROJECT_FILES {
        let from = bin.join(format!("{name}.{suffix}"));
        if from.exists()
            && let Err(e) = std::fs::rename(&from, server.dir.join(format!("{to}.{suffix}")))
        {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    let _ = std::fs::remove_dir(&bin);
    axum::Json(json!({ "name": to })).into_response()
}

/// `GET /projects`: every project in the folder, most recently changed first.
async fn projects(State(server): State<Arc<Server>>) -> impl IntoResponse {
    let mut list = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&server.dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else { continue };
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") || !valid_name(&name) {
                continue;
            }
            let meta = entry.metadata().ok();
            let millis = |t: std::io::Result<std::time::SystemTime>| t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
            let modified = meta.as_ref().and_then(|m| millis(m.modified()));
            let created = meta.as_ref().and_then(|m| millis(m.created()));
            let revisions = std::fs::read(&path).map(|b| b.iter().filter(|c| **c == b'\n').count()).unwrap_or(0);
            // Team: who is in it, and whether this engine joined someone else's project.
            let team = std::fs::read_to_string(server.dir.join(format!("{name}.team.json"))).ok().and_then(|t| serde_json::from_str::<peers::Team>(&t).ok()).filter(|t| !t.members.is_empty() || t.joining.is_some());
            let team = team.map(|t| {
                let mine = t.members.iter().find(|m| m.engine.as_deref() == Some(server.engine.as_str()));
                let joined = t.joining.is_some() || mine.is_some_and(|m| !m.invited_by.is_empty());
                json!({
                    "members": t.members.iter().map(|m| json!({ "name": m.name, "role": m.role })).collect::<Vec<_>>(),
                    "joined": joined,
                    "by": mine.map(|m| m.invited_by.clone()).unwrap_or_default(),
                })
            });
            let thumbnail = server.dir.join(format!("{name}.thumb.png")).exists();
            list.push(json!({ "name": name, "modified": modified, "created": created, "revisions": revisions, "default": name == server.default, "team": team, "thumbnail": thumbnail, "info": info_of(&server.dir, &name) }));
        }
    }
    list.sort_by_key(|p| std::cmp::Reverse(p["modified"].as_u64().unwrap_or(0)));
    axum::Json(list)
}

/// A project's picture for the projects page (PNG, taken from its view).
async fn thumbnail_get(State(server): State<Arc<Server>>, Path(name): Path<String>) -> axum::response::Response {
    if !valid_name(&name) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match std::fs::read(server.dir.join(format!("{name}.thumb.png"))) {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/png"), (axum::http::header::CACHE_CONTROL, "no-cache")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn thumbnail_put(State(server): State<Arc<Server>>, Path(name): Path<String>, body: axum::body::Bytes) -> axum::response::Response {
    // Only PNGs, for projects that exist.
    if !valid_name(&name) || !body.starts_with(b"\x89PNG") || !server.dir.join(format!("{name}.jsonl")).exists() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match std::fs::write(server.dir.join(format!("{name}.thumb.png")), &body) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

struct App {
    /// This project's Forma AI session.
    ai: Arc<ai::Ai>,
    project: Mutex<Project>,
    log: Mutex<File>,
    events: broadcast::Sender<Arc<str>>,
    sessions: AtomicUsize,
    /// Who else works on this project, and how to reach their engines.
    team: Mutex<peers::Team>,
    /// Live connections to other members' engines.
    links: Mutex<peers::Links>,
    /// Every new log entry, for the peer connections to pass on.
    shared: broadcast::Sender<Arc<WireChangeSet>>,
    /// Whether this project's peer dialing loop is running.
    dialing: AtomicBool,
    /// Fields teammates changed differently at the same time (latest first),
    /// kept in `<project>.conflicts.json` until someone settles them.
    conflicts: Mutex<Vec<de_core::Conflict>>,
    conflicts_path: PathBuf,
    /// Renamed or deleted while open: nothing more is written or synced.
    closed: AtomicBool,
    /// Imported files arriving from teammates.
    assets: Mutex<peers::Assets>,
}

/// Messages for this project's Forma AI.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AiMessage {
    AiAsk {
        request_id: u64,
        text: String,
        #[serde(default)]
        context: ai::ViewContext,
    },
    AiStop {
        request_id: u64,
    },
    /// Show something in a person's view (select, frame, isolate, hide, paint,
    /// camera). Not design data: nothing is saved, and only that person's views
    /// follow (by default whoever spoke to the AI last).
    View {
        request_id: u64,
        action: String,
        #[serde(default)]
        target: Option<StableId>,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        to: Option<String>,
    },
}

/// Where this person is pointing and what they have selected, for teammates
/// to see live. Never saved; passed straight to the other members' engines.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LiveMessage {
    Cursor {
        #[serde(default)]
        selected: Option<String>,
        #[serde(default)]
        part: Option<String>,
        #[serde(default)]
        pointer: Option<[f64; 3]>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Commit(Request),
    Preview(Request),
    Undo { request_id: u64 },
    Redo { request_id: u64 },
    Check { request_id: u64 },
    /// Evaluate a shape with changed parameter values, without committing.
    /// Values for keys the shape does not have yet are added (a new fillet's
    /// radius), and `finishes`, when given, replaces its edge finishes.
    ShapePreview {
        request_id: u64,
        target: StableId,
        values: BTreeMap<String, f64>,
        #[serde(default)]
        finishes: Option<Vec<de_shape::EdgeFinish>>,
    },
    /// The edges of a shape as it is now, for picking (program coordinates, mm).
    ShapeEdges { request_id: u64, target: StableId },
    /// Which parameter drives the flat face at a point (program coordinates).
    ShapeFace {
        request_id: u64,
        target: StableId,
        #[serde(default)]
        part: Option<String>,
        point: [f64; 3],
        normal: [f64; 3],
    },
    Snapshot,
}

#[derive(Deserialize)]
struct Request {
    request_id: u64,
    command_id: Option<String>,
    operation: String,
    #[serde(default)]
    parameters: Value,
    expected_revision: Revision,
}

/// What the UI needs to show one entity.
#[derive(Serialize)]
struct EntityView<'a> {
    id: StableId,
    components: BTreeMap<&'static str, Value>,
    status: Option<&'a Status>,
    mass: Option<&'a MassProperties>,
    mesh: Option<&'a DisplayMesh>,
    result: Option<&'a RequirementResult>,
    shape: Option<&'a ShapeStats>,
    /// Parameter values after formulas, and the formulas in readable names.
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a de_shape::ResolvedParams>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    formulas: BTreeMap<String, String>,
    /// A sketch where its constraints put it: points, radii, loops, freedom.
    #[serde(skip_serializing_if = "Option::is_none")]
    sketch: Option<&'a de_shape::sketch::SolvedSketch>,
    /// Totals for an assembly definition or instance.
    assembly: Option<&'a de_assembly::AssemblyStats>,
    /// Mass (kg) of each part of an assembly, in the order of `shape.parts`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    part_masses: Vec<Option<f64>>,
}

fn view(project: &Project, id: StableId) -> Option<EntityView<'_>> {
    Some(EntityView {
        id,
        components: project.entity_state(id)?,
        status: project.get::<Status>(id),
        mass: project.get::<MassProperties>(id),
        mesh: project.get::<DisplayMesh>(id),
        result: project.get::<RequirementResult>(id),
        shape: project.get::<ShapeSolid>(id).map(|s| &s.stats),
        assembly: project.get::<de_assembly::AssemblyStats>(id),
        sketch: project.get::<de_shape::sketch::SolvedSketch>(id),
        params: project.get::<de_shape::ResolvedParams>(id),
        formulas: project
            .entity(id)
            .and_then(|e| de_shape::params::params_of(project.world(), e))
            .map(|(_, ps)| ps.iter().filter_map(|p| p.expr.as_ref().map(|f| (p.key.clone(), de_shape::params::display(project.world(), f)))).collect())
            .unwrap_or_default(),
        part_masses: match (project.entity(id), project.get::<ShapeSolid>(id)) {
            (Some(e), Some(s)) if !s.stats.parts.is_empty() => de_shape::part_masses(project.world(), e).unwrap_or_default(),
            _ => Vec::new(),
        },
    })
}

fn snapshot(project: &Project, actor: &Actor, sessions: usize, ai: &ai::Ai) -> String {
    let commands: Vec<Value> = project
        .registry()
        .commands()
        .map(|c| json!({ "name": c.name, "plugin": c.plugin, "schema": c.schema }))
        .collect();
    let entities: Vec<EntityView> = project.entity_ids().into_iter().filter_map(|id| view(project, id)).collect();
    let history: Vec<Value> = project
        .journal()
        .iter()
        .rev()
        .take(200)
        .map(|c| json!({ "revision": c.revision, "operation": c.operation, "user_id": c.actor.user_id }))
        .collect();
    json!({
        "type": "snapshot",
        "project_id": project.id(),
        "revision": project.revision(),
        "user_id": actor.user_id,
        "session_id": actor.session_id,
        "sessions": sessions,
        "commands": commands,
        "entities": entities,
        "history": history,
        "ai": { "available": ai.available(), "busy": ai.busy_now(), "chat": ai.history() },
    })
    .to_string()
}

fn changed(project: &Project, change: &ChangeSet) -> String {
    let mut entities = Vec::new();
    let mut removed = Vec::new();
    for id in project.affected_by(change) {
        match view(project, id) {
            Some(v) => entities.push(v),
            None => removed.push(id),
        }
    }
    json!({
        "type": "changed",
        "revision": change.revision,
        "operation": change.operation,
        "actor": change.actor,
        "entities": entities,
        "removed": removed,
    })
    .to_string()
}

impl App {
    fn handle(&self, actor: &Actor, text: &str) -> String {
        if self.closed.load(Ordering::SeqCst) {
            return json!({ "type": "error", "message": "this project was renamed or deleted: open it again from the projects page" }).to_string();
        }
        let message: ClientMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => return json!({ "type": "error", "message": format!("bad message: {e}") }).to_string(),
        };
        let mut project = self.project.lock().unwrap();
        let before = project.revision();
        let logged = project.log().len();
        let (request_id, result) = match message {
            ClientMessage::Snapshot => return snapshot(&project, actor, self.sessions.load(Ordering::Relaxed), &self.ai),
            ClientMessage::ShapeFace { request_id, target, part, point, normal } => {
                let Some(program) = project.entity(target).and_then(|e| de_shape::resolved_program(project.world(), e)) else {
                    return reply(request_id, Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a shape"))));
                };
                drop(project);
                let driver = de_shape::face_driver(&program, part.as_deref(), point, normal);
                return reply(request_id, Ok(json!({ "target": target, "driver": driver })));
            }
            ClientMessage::ShapeEdges { request_id, target } => {
                let Some(program) = project.entity(target).and_then(|e| de_shape::resolved_program(project.world(), e)) else {
                    return reply(request_id, Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a shape"))));
                };
                drop(project);
                let values: Vec<(String, f64)> = program.params.iter().map(|p| (p.key.clone(), p.value)).collect();
                let result = de_shape::exact::edges_of(&program.source, &values, program.display_tolerance(), &program.inputs)
                    .map(|parts| json!({ "target": target, "parts": parts }))
                    .map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e));
                return reply(request_id, result);
            }
            ClientMessage::ShapePreview { request_id, target, values, finishes } => {
                let Some(mut program) = project.entity(target).and_then(|e| de_shape::resolved_program(project.world(), e)) else {
                    return reply(request_id, Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a shape"))));
                };
                drop(project); // evaluate without holding the project
                for param in &mut program.params {
                    if let Some(v) = values.get(&param.key) {
                        param.value = *v;
                    }
                }
                for (key, value) in &values {
                    if !program.params.iter().any(|p| &p.key == key) {
                        program.params.push(de_shape::Param { key: key.clone(), label: key.clone(), value: *value, unit: "mm".into(), min: None, max: None, step: None, expr: None });
                    }
                }
                if let Some(f) = finishes {
                    program.finishes = f;
                    program.inputs = de_shape::with_finishes(&program.inputs, &program.finishes);
                }
                let result = de_shape::preview(&program)
                    .map(|(stats, mesh)| json!({ "target": target, "stats": stats, "mesh": mesh }))
                    .map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e));
                return reply(request_id, result);
            }
            ClientMessage::Check { request_id } => {
                return reply(request_id, Ok(serde_json::to_value(de_intent::check(&project)).unwrap()));
            }
            ClientMessage::Preview(r) => {
                let envelope = envelope(&project, actor, r.command_id, r.operation, r.parameters, r.expected_revision);
                let result = project.preview(&envelope).map(|p| serde_json::to_value(p).unwrap());
                return reply(r.request_id, result);
            }
            ClientMessage::Commit(r) => {
                let envelope = envelope(&project, actor, r.command_id, r.operation, r.parameters, r.expected_revision);
                (r.request_id, project.commit(envelope))
            }
            ClientMessage::Undo { request_id } => (request_id, project.undo(actor)),
            ClientMessage::Redo { request_id } => (request_id, project.redo(actor)),
        };
        let result = result.and_then(|committed| {
            self.publish(&project, logged, before).map_err(|e| CommandError::new(ErrorCode::Internal, format!("applied but not saved: {e}")))?;
            Ok(serde_json::to_value(committed).unwrap())
        });
        reply(request_id, result)
    }

    /// After any change: saves the new log entries, passes them to the peer
    /// connections, and tells every open view what changed.
    fn publish(&self, project: &Project, logged: usize, before: Revision) -> std::io::Result<()> {
        for wire in &project.log()[logged..] {
            self.append(wire)?;
            let _ = self.shared.send(Arc::new(wire.clone()));
        }
        for change in project.journal().iter().filter(|c| c.revision > before) {
            let _ = self.events.send(changed(project, change).into());
        }
        Ok(())
    }

    /// Takes change sets from another member's engine (merged in causal order).
    fn receive(&self, changes: Vec<WireChangeSet>) -> Result<(), String> {
        let mut project = self.project.lock().unwrap();
        let (logged, before) = (project.log().len(), project.revision());
        let mut clashes = Vec::new();
        let mut result = Ok(());
        for wire in changes {
            match project.receive(wire) {
                Ok(merges) => clashes.extend(merges.into_iter().flat_map(|m| m.conflicts)),
                Err(e) => {
                    result = Err(e.to_string());
                    break;
                }
            }
        }
        self.publish(&project, logged, before).map_err(|e| e.to_string())?;
        drop(project);
        if !clashes.is_empty() {
            let mut list = self.conflicts.lock().unwrap();
            for c in clashes.iter().rev() {
                list.insert(0, c.clone());
            }
            list.truncate(200);
            self.save_conflicts(&list);
            let _ = self.events.send(json!({ "type": "conflicts", "conflicts": clashes }).to_string().into());
        }
        if project_changed(logged, &self.project) {
            self.team_changed();
        }
        result
    }

    /// An imported file arrived from a teammate: the parts reading it are
    /// evaluated again, and every open view shows them.
    fn asset_arrived(&self, sha: &str) {
        let mut project = self.project.lock().unwrap();
        let readers: Vec<StableId> = project
            .entity_ids()
            .into_iter()
            .filter(|id| project.get::<ShapeProgram>(*id).is_some_and(|p| de_shape::assets::referenced(&p.source).iter().any(|s| s == sha)))
            .collect();
        project.refresh(&readers);
        let entities: Vec<EntityView> = project.downstream(&readers).into_iter().filter_map(|id| view(&project, id)).collect();
        let _ = self.events.send(json!({ "type": "refreshed", "entities": entities }).to_string().into());
    }

    /// Tells open views the team or its connections changed.
    /// Runs one command on behalf of this engine (not from a browser).
    fn commit_here(&self, operation: &str, parameters: Value) -> Result<Value, String> {
        let mut project = self.project.lock().unwrap();
        let (logged, before) = (project.log().len(), project.revision());
        let actor = Actor { user_id: "local".into(), session_id: StableId::new().to_string() };
        let e = envelope(&project, &actor, None, operation.into(), parameters, project.revision());
        let out = project.commit(e).map_err(|e| e.message.clone())?;
        self.publish(&project, logged, before).map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(out).unwrap_or_default())
    }

    fn save_conflicts(&self, list: &[de_core::Conflict]) {
        let _ = if list.is_empty() { std::fs::remove_file(&self.conflicts_path).or(Ok(())) } else { std::fs::write(&self.conflicts_path, serde_json::to_string_pretty(list).unwrap()) };
    }

    fn team_changed(&self) {
        let _ = self.events.send(json!({ "type": "team" }).to_string().into());
    }

    fn append(&self, change: &WireChangeSet) -> std::io::Result<()> {
        let mut log = self.log.lock().unwrap();
        serde_json::to_writer(&mut *log, change)?;
        log.write_all(b"\n")?;
        log.sync_data()
    }

    fn presence(&self) {
        let sessions = self.sessions.load(Ordering::Relaxed);
        let _ = self.events.send(json!({ "type": "presence", "sessions": sessions }).to_string().into());
    }
}

fn project_changed(logged: usize, project: &Mutex<Project>) -> bool {
    project.lock().unwrap().log().len() > logged
}

fn envelope(
    project: &Project,
    actor: &Actor,
    command_id: Option<String>,
    operation: String,
    parameters: Value,
    expected_revision: Revision,
) -> Envelope {
    Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: command_id.unwrap_or_else(|| StableId::new().to_string()),
        project_id: project.id().to_owned(),
        expected_revision,
        operation,
        parameters,
        actor: actor.clone(),
    }
}

fn reply(request_id: u64, result: Result<Value, CommandError>) -> String {
    match result {
        Ok(value) => json!({ "type": "result", "request_id": request_id, "ok": true, "value": value }),
        Err(error) => json!({ "type": "result", "request_id": request_id, "ok": false, "error": error }),
    }
    .to_string()
}

async fn websocket(
    State(server): State<Arc<Server>>,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    let user_id = query.get("user").filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| "local".into());
    match server.project(query.get("project")) {
        Ok(app) => upgrade.on_upgrade(move |socket| session(app, socket, user_id)),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn session(app: Arc<App>, socket: WebSocket, user_id: String) {
    let actor = Actor { user_id, session_id: StableId::new().to_string() };
    let (mut sink, mut stream) = socket.split();
    let (outgoing, mut queue) = mpsc::unbounded_channel::<Arc<str>>();

    let writer = tokio::spawn(async move {
        while let Some(text) = queue.recv().await {
            if sink.send(Message::Text(text.as_ref().into())).await.is_err() {
                break;
            }
        }
    });

    // Subscribe before taking the snapshot so no change falls between them.
    let mut events = app.events.subscribe();
    app.sessions.fetch_add(1, Ordering::Relaxed);
    let first = snapshot(&app.project.lock().unwrap(), &actor, app.sessions.load(Ordering::Relaxed), &app.ai);
    let _ = outgoing.send(first.into());
    app.presence();

    let forward_to = outgoing.clone();
    let forward = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(text) => {
                    if forward_to.send(text).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let _ = forward_to.send(json!({ "type": "resync" }).to_string().into());
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        if let Message::Text(text) = message {
            if let Ok(LiveMessage::Cursor { selected, part, pointer }) = serde_json::from_str::<LiveMessage>(text.as_str()) {
                peers::cursor(&app, selected, part, pointer);
                continue;
            }
            // Messages for Forma AI run alongside the engine, not on its lock.
            if let Ok(m) = serde_json::from_str::<AiMessage>(text.as_str()) {
                let reply = match m {
                    AiMessage::AiAsk { request_id, text, context } => {
                        let result = app.ai.ask(Arc::clone(&app), &actor.user_id, &text, context).await;
                        reply(request_id, result.map(|_| json!({ "queued": true })).map_err(|e| CommandError::new(ErrorCode::Internal, e)))
                    }
                    AiMessage::AiStop { request_id } => {
                        app.ai.stop(Arc::clone(&app)).await;
                        reply(request_id, Ok(json!({ "stopped": true })))
                    }
                    AiMessage::View { request_id, action, target, value, to } => {
                        const ACTIONS: &[&str] = &["select", "frame", "isolate", "show_all", "hide", "show", "color", "camera", "explode"];
                        let known = target.is_none_or(|id| app.project.lock().unwrap().entity(id).is_some());
                        if !ACTIONS.contains(&action.as_str()) {
                            reply(request_id, Err(CommandError::new(ErrorCode::InvalidParameters, format!("unknown view action {action} (use {})", ACTIONS.join(", ")))))
                        } else if !known {
                            reply(request_id, Err(CommandError::new(ErrorCode::UnresolvedReference, "nothing with that id in this project")))
                        } else {
                            let to = to.or_else(|| app.ai.audience()).filter(|_| actor.user_id == "forma-ai").or(Some(actor.user_id.clone()));
                            let _ = app.events.send(json!({ "type": "view", "to": to, "action": action, "target": target, "value": value }).to_string().into());
                            reply(request_id, Ok(json!({ "shown_to": to })))
                        }
                    }
                };
                if outgoing.send(reply.into()).is_err() {
                    break;
                }
                continue;
            }
            let app = Arc::clone(&app);
            let actor = actor.clone();
            let response = tokio::task::spawn_blocking(move || app.handle(&actor, text.as_str()))
                .await
                .unwrap_or_else(|e| json!({ "type": "error", "message": e.to_string() }).to_string());
            if outgoing.send(response.into()).is_err() {
                break;
            }
        }
    }

    forward.abort();
    writer.abort();
    app.sessions.fetch_sub(1, Ordering::Relaxed);
    app.presence();
}

/// Everything an export needs, copied out of the project so the (possibly
/// slow) fine evaluation runs without holding the lock.
struct Source {
    name: String,
    at: [f64; 3],
    /// Rotation about Z (degrees) of an instance's shape, applied before `at`.
    rotation: f64,
    program: Option<ShapeProgram>,
    boxed: Option<[f64; 3]>,
    /// Material name → (display name, density kg/m³) for parts; "" is the default.
    materials: HashMap<String, (String, f64)>,
    /// Where this sits in an assembly ("Tower › L12"), prefixed to part names.
    prefix: String,
    /// For an instance or assembly: the shapes inside it.
    members: Vec<Source>,
}

/// What gets printed: one part of a shape, or the whole shape as one solid
/// (separate bodies stay separate shells, so print-in-place parts print whole).
fn whole_solid(s: &Source, part: Option<&str>, tolerance: f64) -> Result<(String, manifold_csg::Manifold), String> {
    if !s.members.is_empty() {
        return Err("send a shape, or one part of it: whole assemblies are not printed as one object yet".into());
    }
    if let Some(name) = part {
        let items = s.items(tolerance)?;
        let item = items.iter().find(|i| i.name == name).ok_or_else(|| format!("{} has no part named {name}", s.name))?;
        return Ok((item.name.clone(), item.solid.translate(0.0, 0.0, 0.0)));
    }
    let solid = match (&s.program, s.boxed) {
        (Some(p), _) => de_shape::solid_at(p, tolerance)?,
        (None, Some([w, d, h])) => manifold_csg::Manifold::cube(w, d, h, false),
        _ => return Err("not a part".into()),
    };
    Ok((s.name.clone(), solid))
}

/// `POST /slicer/<id>?project=…[&part=…]`: the part as a 3MF, saved in
/// ~/Downloads/Forma prints and opened in Bambu Studio (the engine runs on
/// this computer, so it can hand the file to the slicer directly).
async fn send_to_slicer(State(server): State<Arc<Server>>, Path(stem): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let source = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(stem)).ok().and_then(|id| source(&project, id))
    };
    let Some(source) = source else { return fail(StatusCode::NOT_FOUND, format!("{stem} is not a part")) };
    let part = query.get("part").cloned();
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        // Fine enough that no facet shows in a print (a 0.4 mm nozzle draws far coarser).
        let (name, solid) = whole_solid(&source, part.as_deref(), 0.005)?;
        let dir = std::env::var_os("HOME").map(PathBuf::from).ok_or("no home folder")?.join("Downloads").join("Forma prints");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{}.3mf", fabricate::file_name(&name)));
        std::fs::write(&path, fabricate::three_mf(&name, &solid)).map_err(|e| format!("could not save {}: {e}", path.display()))?;
        let opened = ["BambuStudio", "Bambu Studio"]
            .iter()
            .any(|app| std::process::Command::new("open").arg("-a").arg(app).arg(&path).status().is_ok_and(|s| s.success()));
        Ok(json!({ "opened": opened, "app": "Bambu Studio", "path": path, "name": name }))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

fn source(project: &Project, id: StableId) -> Option<Source> {
    let material = |mid: StableId| project.get::<de_solid::Material>(mid).map(|m| (m.name.clone(), m.density.canonical()));
    // Instances and assemblies export every shape inside them; shapes with no
    // placement are definitions, made only through their instances.
    let instance = project.get::<de_assembly::Instance>(id);
    if instance.is_some() || project.get::<de_assembly::AssemblyDef>(id).is_some() {
        let name = instance.map(|i| i.name.clone()).or_else(|| project.get::<de_assembly::AssemblyDef>(id).map(|a| a.name.clone())).unwrap_or_default();
        let mut members = Vec::new();
        for o in de_assembly::occurrences(project.world(), id) {
            let mut s = shape_source(project, o.shape, &material)?;
            s.prefix = o.names[..o.names.len() - 1].join(" › ");
            s.at = o.transform.at;
            s.rotation = o.transform.rotation;
            members.push(s);
        }
        return Some(Source { name, at: [0.0; 3], rotation: 0.0, program: None, boxed: None, materials: HashMap::new(), prefix: String::new(), members });
    }
    let program = project.get::<ShapeProgram>(id);
    if program.is_some() && project.get::<de_solid::Placement>(id).is_none() {
        return None;
    }
    shape_source(project, id, &material)
}

fn shape_source(project: &Project, id: StableId, material: &dyn Fn(StableId) -> Option<(String, f64)>) -> Option<Source> {
    let p = project.get::<de_solid::Placement>(id).copied().unwrap_or_default();
    // Evaluated with linked parameters worked out.
    let program = project.entity(id).and_then(|e| de_shape::resolved_program(project.world(), e));
    let boxed = project.get::<de_solid::BoxFeature>(id).map(|b| [b.width, b.depth, b.height].map(|l| l.canonical() * 1000.0));
    if program.is_none() && boxed.is_none() {
        return None;
    }
    let mut materials = HashMap::new();
    if let Some(m) = project.get::<de_solid::MaterialRef>(id).and_then(|m| material(m.0)) {
        materials.insert(String::new(), m);
    }
    if let Some(table) = project.get::<de_shape::PartMaterials>(id) {
        for (name, mid) in &table.0 {
            if let Some(m) = material(*mid) {
                materials.insert(name.clone(), m);
            }
        }
    }
    let name = program.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Box".into());
    Some(Source { name, at: [p.x, p.y, p.z].map(|l| l.canonical() * 1000.0), rotation: 0.0, program, boxed, materials, prefix: String::new(), members: Vec::new() })
}

impl Source {
    fn items(&self, tolerance: f64) -> Result<Vec<fabricate::Item>, String> {
        match (&self.program, self.boxed) {
            (Some(program), _) => fabricate::items(program, tolerance),
            (None, Some([w, d, h])) => {
                Ok(vec![fabricate::Item { name: self.name.clone(), material: None, process: None, solid: manifold_csg::Manifold::cube(w, d, h, false), volume_mm3: w * d * h, key: String::new() }])
            }
            _ => Err("not a part".into()),
        }
    }

    /// Every program in this source (itself, or its members), by program key.
    fn programs(&self, out: &mut HashMap<String, ShapeProgram>) {
        if let Some(p) = &self.program {
            out.insert(fabricate::program_key(p), p.clone());
        }
        for m in &self.members {
            m.programs(out);
        }
    }

    /// The exact named shapes of this source, placed: for a whole-design STEP.
    fn exact(&self, maker: &mut fabricate::StepMaker, out: &mut Vec<(String, de_occt::Shape)>, faceted: &mut Vec<String>) -> Result<(), String> {
        for m in &self.members {
            m.exact(maker, out, faceted)?;
        }
        let place = |maker: &mut fabricate::StepMaker, s: de_occt::Shape| -> Result<de_occt::Shape, String> {
            let a = maker.arena();
            let s = if self.rotation.abs() > 1e-12 { a.rotate(s, [0.0, 0.0, self.rotation])? } else { s };
            if self.at.iter().any(|v| v.abs() > 1e-12) { a.translate(s, self.at) } else { Ok(s) }
        };
        let label = |part: &str| {
            let mut names: Vec<&str> = Vec::new();
            if !self.prefix.is_empty() {
                names.push(&self.prefix);
            }
            names.push(part);
            names.join(" › ")
        };
        match (&self.program, self.boxed) {
            (Some(p), _) => {
                let (parts, f) = maker.parts(p)?;
                faceted.extend(f.into_iter().map(|f| format!("{}: {f}", self.name)));
                for (name, s) in parts {
                    let placed = place(maker, s)?;
                    out.push((label(&name), placed));
                }
            }
            (None, Some([w, d, h])) => {
                let b = maker.arena().cube(w, d, h, false)?;
                let placed = place(maker, b)?;
                out.push((label(&self.name), placed));
            }
            _ => {}
        }
        Ok(())
    }

    fn material(&self, name: Option<&str>) -> (String, Option<f64>) {
        match self.materials.get(name.unwrap_or("")) {
            Some((n, d)) => (n.clone(), Some(*d)),
            None => (name.unwrap_or("").to_string(), None),
        }
    }
}

/// One package for everything inside an assembly: identical parts across all
/// its shapes merge into one cut list line (34 floors' slabs → one line × 34).
fn assembly_package(folder: &str, members: &[Source], tolerance: f64, step_of: &dyn Fn(&fabricate::Item) -> Option<Result<Vec<u8>, String>>) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut items = Vec::new();
    let mut materials: Vec<(String, String, Option<f64>)> = Vec::new(); // (key, name, density)
    for (i, m) in members.iter().enumerate() {
        for mut item in m.items(tolerance)? {
            let (name, density) = m.material(item.material.as_deref());
            let key = format!("{i}:{}", item.material.clone().unwrap_or_default());
            materials.push((key.clone(), name, density));
            item.material = Some(key);
            item.name = if m.prefix.is_empty() { item.name } else { format!("{} › {}", m.prefix, item.name) };
            items.push(item);
        }
    }
    let lookup = |key: Option<&str>| {
        materials.iter().find(|(k, _, _)| Some(k.as_str()) == key).map(|(_, n, d)| (n.clone(), *d)).unwrap_or_default()
    };
    Ok(fabricate::package(folder, &items, &lookup, step_of).0)
}

/// Exports, in project millimeters with curves within `tolerance` of the formula:
///
/// - `/export/<id>.stl`: the whole part or assembly, where it sits in the project.
/// - `/export/<id>.stl?part=<name>` and `.dxf?part=<name>`: one part of an
///   assembly, at the origin; the DXF is laid flat with one layer per cut depth.
/// - `/export/<id>.step`: exact B-rep (OpenCascade), every part named; with
///   `?part=<name>` one part at the origin.
/// - `/export/<id>.zip`: the fabrication package (every distinct part's STL,
///   STEP and, for sheet parts, DXF, plus a cut list); `/export/project.zip`
///   does the same for every part.
async fn export(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(query): Query<HashMap<String, String>>) -> impl IntoResponse {
    let fail = |code: StatusCode, message: String| (code, [(header::CONTENT_TYPE, "text/plain".to_string())], message.into_bytes()).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let project_name = query.get("project").cloned().unwrap_or_else(|| server.default.clone());
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return fail(StatusCode::BAD_REQUEST, "expected /export/<part id>.<stl|dxf|zip>".into());
    };
    let ext = ext.to_string();
    let tolerance = query.get("tolerance").and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.005);
    if !(tolerance.is_finite() && (0.0001..=10.0).contains(&tolerance)) {
        return fail(StatusCode::BAD_REQUEST, "tolerance must be between 0.0001 and 10 mm".into());
    }
    let part = query.get("part").cloned();
    let sources: Vec<Source> = {
        let project = app.project.lock().unwrap();
        if stem == "project" && ext == "zip" {
            // Top-level items only: nested instances and assembly definitions come with their parents.
            project
                .entity_ids()
                .into_iter()
                .filter(|id| project.get::<de_assembly::AssemblyDef>(*id).is_none() && project.get::<de_assembly::Instance>(*id).is_none_or(|i| i.parent.is_none()))
                .filter_map(|id| source(&project, id))
                .collect()
        } else {
            match serde_json::from_value::<StableId>(json!(stem)).ok().and_then(|id| source(&project, id)) {
                Some(s) => vec![s],
                None => return fail(StatusCode::NOT_FOUND, format!("{stem} is not a part")),
            }
        }
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(String, &'static str, Vec<u8>), String> {
        match ext.as_str() {
            "zip" => {
                // Every distinct part also gets an exact STEP, made from its program.
                let mut programs = HashMap::new();
                for s in &sources {
                    s.programs(&mut programs);
                }
                let maker = std::cell::RefCell::new(fabricate::StepMaker::new(tolerance));
                let step_of = |item: &fabricate::Item| -> Option<Result<Vec<u8>, String>> {
                    let (key, part) = item.key.split_once('#')?;
                    let program = programs.get(key)?;
                    Some(maker.borrow_mut().part_step(program, part))
                };
                let mut files = Vec::new();
                let mut seen = HashMap::<String, usize>::new();
                for s in &sources {
                    // Two assemblies with the same name get separate folders.
                    let n = seen.entry(s.name.clone()).or_default();
                    *n += 1;
                    let folder = if *n == 1 { s.name.clone() } else { format!("{} {n}", s.name) };
                    if s.members.is_empty() {
                        let items = s.items(tolerance)?;
                        files.extend(fabricate::package(&folder, &items, &|m| s.material(m), &step_of).0);
                    } else {
                        files.extend(assembly_package(&folder, &s.members, tolerance, &step_of)?);
                    }
                }
                let name = if sources.len() == 1 { sources[0].name.clone() } else { project_name.clone() };
                Ok((format!("{} fabrication.zip", fabricate::file_name(&name)), "application/zip", fabricate::zip(&files)))
            }
            "step" | "stp" => {
                // Exact B-rep, every part named. One part (at the origin) or the whole design, placed.
                let s = &sources[0];
                let mut maker = fabricate::StepMaker::new(tolerance);
                if let Some(name) = &part {
                    let program = s.program.as_ref().ok_or("pick a part of a shape")?;
                    let bytes = maker.part_step(program, name)?;
                    return Ok((format!("{}.step", fabricate::file_name(name)), "model/step", bytes));
                }
                let mut shapes = Vec::new();
                let mut faceted = Vec::new();
                s.exact(&mut maker, &mut shapes, &mut faceted)?;
                if shapes.is_empty() {
                    return Err("nothing to export".into());
                }
                if !faceted.is_empty() {
                    eprintln!("step export of {}: faceted where the program built geometry itself: {}", s.name, faceted.join("; "));
                }
                Ok((format!("{}.step", fabricate::file_name(&s.name)), "model/step", maker.step(&shapes)?))
            }
            "stl" | "dxf" => {
                let s = &sources[0];
                if !s.members.is_empty() {
                    return Err("export an assembly as a fabrication package (.zip), or pick one of its parts".into());
                }
                match part {
                    None if ext == "stl" => {
                        let solid = match (&s.program, s.boxed) {
                            (Some(p), _) => de_shape::solid_at(p, tolerance)?,
                            (None, Some([w, d, h])) => manifold_csg::Manifold::cube(w, d, h, false),
                            _ => return Err("not a part".into()),
                        };
                        Ok((format!("{}.stl", fabricate::file_name(&s.name)), "model/stl", de_shape::stl_of(&solid, &format!("Forma {}", s.name), s.at)))
                    }
                    _ => {
                        let items = s.items(tolerance)?;
                        let item = match &part {
                            Some(name) => items.iter().find(|i| &i.name == name).ok_or_else(|| format!("{} has no part named {name}", s.name))?,
                            None => &items[0],
                        };
                        let base = fabricate::file_name(&item.name);
                        if ext == "dxf" {
                            Ok((format!("{base}.dxf"), "image/vnd.dxf", fabricate::dxf(&item.name, &item.solid).into_bytes()))
                        } else {
                            Ok((format!("{base}.stl"), "model/stl", de_shape::stl_of(&fabricate::at_origin(&item.solid), &format!("Forma {}", item.name), [0.0; 3])))
                        }
                    }
                }
            }
            "3mf" => {
                let s = &sources[0];
                let (name, solid) = whole_solid(s, part.as_deref(), tolerance)?;
                Ok((format!("{}.3mf", fabricate::file_name(&name)), "model/3mf", fabricate::three_mf(&name, &solid)))
            }
            _ => Err(format!("unknown export format .{ext}")),
        }
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok((name, mime, bytes)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))],
            bytes,
        )
            .into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// `POST /gears/<id>?project=…&torque=<N·m>&rpm=<rpm>`: turns the shape's gear
/// train through a full turn (clashes, smallest gaps), and reports each
/// mesh's ratio, contact ratio, backlash and, with a load, tooth strength.
async fn gear_check(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let program = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|id| project.entity(id)).and_then(|e| de_shape::resolved_program(project.world(), e))
    };
    let Some(program) = program else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    let num = |k: &str| query.get(k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0);
    let load = match (num("torque"), num("rpm")) {
        (Some(t), Some(r)) if t > 0.0 => Some((t, r)),
        _ => None,
    };
    match tokio::task::spawn_blocking(move || de_shape::gear_check(&program, load)).await {
        Ok(Ok(c)) => axum::Json(c).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// A linkage's motion: every body's pose at each step of its drive.
async fn motion(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let found = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|id| project.entity(id)).and_then(|e| {
            let program = de_shape::resolved_program(project.world(), e)?;
            // Each part's mass (kg), for the forces.
            let masses = de_shape::part_masses(project.world(), e).unwrap_or_default();
            Some((program, masses))
        })
    };
    let Some((program, masses)) = found else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    let steps = query.get("steps").and_then(|v| v.parse::<usize>().ok()).unwrap_or(360);
    let gravity = query.get("gravity").is_none_or(|g| g != "off");
    match tokio::task::spawn_blocking(move || de_shape::motion_with_forces(&program, steps, Some(&masses), gravity)).await {
        Ok(Ok(s)) => axum::Json(s).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct StressRequest {
    #[serde(flatten)]
    study: de_shape::fea::Study,
    /// One part of an assembly.
    #[serde(default)]
    part: Option<String>,
    /// Other parts studied with it, pressing on it where they touch (["all"]: every other part).
    #[serde(default)]
    parts: Vec<String>,
}

/// A part's material by name: its own, else the shape's.
fn part_material(project: &Project, sid: StableId, part: Option<&str>) -> Option<String> {
    let own = part.and_then(|n| project.get::<ShapeSolid>(sid)?.stats.parts.iter().find(|p| p.name == n)?.material.clone());
    own.or_else(|| {
        let m = project.get::<de_solid::MaterialRef>(sid)?;
        project.get::<de_solid::Material>(m.0).map(|m| m.name.clone())
    })
}

/// A stress study: supports, loads and resolution in the body; the part's
/// own material sets its stiffness and yield strength. With `parts`, those
/// parts are studied with it, each in its own material, in contact where
/// they touch (`contacts` says how each pair acts; frictionless if unsaid).
async fn stress(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, axum::Json(req): axum::Json<StressRequest>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let found = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|sid| {
            let e = project.entity(sid)?;
            let program = de_shape::resolved_program(project.world(), e)?;
            let all: Vec<String> = project.get::<ShapeSolid>(sid).map(|s| s.stats.parts.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
            // The studied part first, then the others (each once).
            let mut names: Vec<String> = req.part.iter().cloned().collect();
            for n in &req.parts {
                let more: Vec<String> = if n == "all" { all.clone() } else { vec![n.clone()] };
                for m in more {
                    if !names.contains(&m) {
                        names.push(m);
                    }
                }
            }
            let materials: Vec<(String, Option<String>)> = names.iter().map(|n| (n.clone(), part_material(&project, sid, Some(n)))).collect();
            Some((program, part_material(&project, sid, req.part.as_deref()), materials))
        })
    };
    let Some((program, material, materials)) = found else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    if materials.len() > 1 {
        if let Some((name, _)) = materials.iter().find(|(_, m)| m.is_none()) {
            return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("give “{name}” a material first: its stiffness comes from it"));
        }
        let parts: Vec<(String, String)> = materials.into_iter().map(|(n, m)| (n, m.unwrap_or_default())).collect();
        let material = parts[0].1.clone();
        return match tokio::task::spawn_blocking(move || de_shape::assembly_stress(&program, &parts, &req.study).map(|r| (r, material))).await {
            Ok(Ok((r, material))) => axum::Json(json!({ "result": r, "material": material })).into_response(),
            Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
            Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }
    let Some(material) = material else { return fail(StatusCode::UNPROCESSABLE_ENTITY, "give the part a material first: its stiffness and strength come from it".into()) };
    match tokio::task::spawn_blocking(move || de_shape::stress_study(&program, req.part.as_deref(), &material, &req.study).map(|r| (r, material))).await {
        Ok(Ok((r, material))) => axum::Json(json!({ "result": r, "material": material })).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// The other parts of an assembly touching a part: the ones a stress study can take with it.
async fn touching(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let Some(part) = query.get("part").cloned() else { return fail(StatusCode::BAD_REQUEST, "name the part".into()) };
    let program = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|sid| de_shape::resolved_program(project.world(), project.entity(sid)?))
    };
    let Some(program) = program else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    match tokio::task::spawn_blocking(move || de_shape::touching_parts(&program, &part)).await {
        Ok(Ok(parts)) => axum::Json(json!({ "parts": parts })).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct ThermalRequest {
    #[serde(flatten)]
    study: de_shape::fea::ThermalStudy,
    #[serde(default)]
    part: Option<String>,
}

/// Steady temperatures through a part, in its own material.
async fn thermal(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, axum::Json(req): axum::Json<ThermalRequest>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let found = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|sid| {
            let e = project.entity(sid)?;
            let program = de_shape::resolved_program(project.world(), e)?;
            let part_material = req.part.as_ref().and_then(|n| project.get::<ShapeSolid>(sid)?.stats.parts.iter().find(|p| &p.name == n)?.material.clone());
            let material = part_material.or_else(|| {
                let m = project.get::<de_solid::MaterialRef>(sid)?;
                project.get::<de_solid::Material>(m.0).map(|m| m.name.clone())
            });
            Some((program, material))
        })
    };
    let Some((program, material)) = found else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    let Some(material) = material else { return fail(StatusCode::UNPROCESSABLE_ENTITY, "give the part a material first: how well it conducts heat comes from it".into()) };
    match tokio::task::spawn_blocking(move || de_shape::thermal_study(&program, req.part.as_deref(), &material, &req.study).map(|r| (r, material))).await {
        Ok(Ok((r, material))) => axum::Json(json!({ "result": r, "material": material })).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// A moving part checked for strength at the worst moment of its motion.
async fn motion_stress(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let Some(part) = query.get("part").cloned() else { return fail(StatusCode::BAD_REQUEST, "name the part to check".into()) };
    let found = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|sid| {
            let e = project.entity(sid)?;
            let program = de_shape::resolved_program(project.world(), e)?;
            let masses = de_shape::part_masses(project.world(), e).unwrap_or_default();
            let material = project.get::<ShapeSolid>(sid)?.stats.parts.iter().find(|p| p.name == part).and_then(|p| p.material.clone()).or_else(|| {
                let m = project.get::<de_solid::MaterialRef>(sid)?;
                project.get::<de_solid::Material>(m.0).map(|m| m.name.clone())
            });
            Some((program, masses, material))
        })
    };
    let Some((program, masses, material)) = found else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    let Some(material) = material else { return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("give “{part}” a material first")) };
    match tokio::task::spawn_blocking(move || de_shape::motion_stress(&program, &part, &masses, &material).map(|r| (r, material))).await {
        Ok(Ok((r, material))) => axum::Json(json!({ "check": r, "material": material })).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// A part's saved study with its faces found on the shape as it is now
/// (a face that no longer exists comes back null).
async fn study(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let part = query.get("part").filter(|p| !p.is_empty()).cloned();
    let found = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|sid| {
            let e = project.entity(sid)?;
            let program = de_shape::resolved_program(project.world(), e)?;
            let spec = project.get::<de_shape::ShapeStudies>(sid).and_then(|s| s.0.iter().find(|st| st.part == part).cloned());
            Some((program, spec))
        })
    };
    let Some((program, spec)) = found else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    let Some(spec) = spec else { return axum::Json(json!({ "spec": null })).into_response() };
    let out = tokio::task::spawn_blocking(move || {
        let supports: Vec<Option<de_shape::fea::Face>> = spec.supports.iter().map(|f| de_shape::resolve_face(&program, spec.part.as_deref(), f)).collect();
        let loads: Vec<Value> = spec
            .loads
            .iter()
            .map(|l| {
                let face = de_shape::resolve_face(&program, spec.part.as_deref(), &l.face);
                let force = face.as_ref().map(|f| l.force(f.normal));
                json!({ "face": face, "force": force })
            })
            .collect();
        let holds: Vec<Value> = spec.holds.iter().map(|h| json!({ "face": de_shape::resolve_face(&program, spec.part.as_deref(), &h.face) })).collect();
        let heats: Vec<Value> = spec.heats.iter().map(|h| json!({ "face": de_shape::resolve_face(&program, spec.part.as_deref(), &h.face) })).collect();
        json!({ "spec": spec, "supports": supports, "loads": loads, "holds": holds, "heats": heats })
    })
    .await;
    match out {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct ModesRequest {
    supports: Vec<de_shape::fea::Face>,
    #[serde(default)]
    part: Option<String>,
    #[serde(default = "default_mode_count")]
    count: usize,
    #[serde(default = "default_mode_resolution")]
    resolution: usize,
}
fn default_mode_count() -> usize {
    4
}
fn default_mode_resolution() -> usize {
    48
}

/// Natural frequencies of a part held at its supports, in its own material.
async fn modes(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(query): Query<HashMap<String, String>>, axum::Json(req): axum::Json<ModesRequest>) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let found = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|sid| {
            let e = project.entity(sid)?;
            let program = de_shape::resolved_program(project.world(), e)?;
            let stats = &project.get::<ShapeSolid>(sid)?.stats;
            // Material name, and density from the part's mass and volume.
            let (material, mass, volume) = match &req.part {
                Some(n) => {
                    let k = stats.parts.iter().position(|p| &p.name == n)?;
                    let mass = de_shape::part_masses(project.world(), e).ok()?.get(k).copied().flatten();
                    (stats.parts[k].material.clone(), mass, stats.parts[k].volume_mm3)
                }
                None => {
                    let m = project.get::<de_solid::MaterialRef>(sid).and_then(|m| project.get::<de_solid::Material>(m.0)).map(|m| m.name.clone());
                    (m, project.get::<MassProperties>(sid).and_then(|m| m.mass_kg), stats.volume_mm3)
                }
            };
            Some((program, material, mass.map(|kg| kg / (volume * 1e-9))))
        })
    };
    let Some((program, material, density)) = found else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    let (Some(material), Some(density)) = (material, density) else { return fail(StatusCode::UNPROCESSABLE_ENTITY, "give the part a material first: its stiffness and mass come from it".into()) };
    match tokio::task::spawn_blocking(move || de_shape::vibration_study(&program, req.part.as_deref(), &material, density, &req.supports, req.count, req.resolution)).await {
        Ok(Ok(m)) => axum::Json(m).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Today as YYYY-MM-DD (UTC), for title blocks.
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A part's engineering drawing: `{id}` gives a sheet as JSON (the app draws
/// it), `{id}.pdf` every sheet as pages, `{id}.dxf` a sheet and `{id}.zip`
/// every sheet's DXF. Query: sheet (A4–A0), projection (first, third), part
/// (one part of an assembly), page (which sheet, from 1).
async fn drawing(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    use axum::http::header;
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let (id, format) = match file.rsplit_once('.') {
        Some((id, f)) if f == "pdf" || f == "dxf" || f == "zip" => (id.to_string(), f),
        _ => (file.clone(), "json"),
    };
    let page: Option<usize> = match query.get("page").filter(|p| !p.is_empty()) {
        None => None,
        Some(p) => match p.parse::<usize>() {
            Ok(n) if n >= 1 => Some(n),
            _ => return fail(StatusCode::BAD_REQUEST, format!("page is a sheet number from 1, not “{p}”")),
        },
    };
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let base = de_shape::drawing::Options {
        sheet: match query.get("sheet").map(String::as_str) {
            Some("A4") => de_shape::drawing::Sheet::A4,
            Some("A2") => de_shape::drawing::Sheet::A2,
            Some("A1") => de_shape::drawing::Sheet::A1,
            Some("A0") => de_shape::drawing::Sheet::A0,
            _ => de_shape::drawing::Sheet::A3,
        },
        projection: if query.get("projection").is_some_and(|p| p == "third") { de_shape::drawing::Projection::Third } else { de_shape::drawing::Projection::First },
        part: query.get("part").filter(|p| !p.is_empty()).cloned(),
        ..Default::default()
    };
    let kg_text = |kg: f64| if kg < 1.0 { format!("{} g", de_shape::drawing::length_text(kg * 1000.0)) } else { format!("{} kg", de_shape::drawing::length_text(kg)) };
    let found = {
        let project = app.project.lock().unwrap();
        serde_json::from_value::<StableId>(json!(id)).ok().and_then(|sid| {
            let e = project.entity(sid)?;
            let program = de_shape::resolved_program(project.world(), e)?;
            let stats = &project.get::<ShapeSolid>(sid)?.stats.parts;
            let masses = de_shape::part_masses(project.world(), e).unwrap_or_default();
            // The saved setup: its sheets, sections, dimensions and annotations.
            let setup = project.get::<de_shape::ShapeDrawings>(sid).and_then(|all| all.0.iter().find(|d| d.part == base.part).cloned()).unwrap_or_default();
            // What a sheet's title block says of the part it draws.
            let about = |part: &Option<String>| {
                let part_material = part.as_ref().and_then(|n| stats.iter().find(|p| &p.name == n)?.material.clone());
                // The part's own material; else the whole thing's; else the one all parts share.
                let shared = || {
                    let first = stats.first()?.material.clone()?;
                    stats.iter().all(|p| p.material.as_ref() == Some(&first)).then_some(first)
                };
                let material = part_material
                    .or_else(|| {
                        let m = project.get::<de_solid::MaterialRef>(sid)?;
                        project.get::<de_solid::Material>(m.0).map(|m| m.name.clone())
                    })
                    .or_else(|| if part.is_none() { shared() } else { None });
                let mass = match part {
                    None => project.get::<MassProperties>(sid).and_then(|m| m.mass_kg),
                    Some(n) => stats.iter().position(|p| &p.name == n).and_then(|i| masses.get(i).copied().flatten()),
                };
                // The parts list: one row per distinct part, with how many there are.
                let mut parts: Vec<de_shape::drawing::PartRow> = vec![];
                if part.is_none() {
                    for (i, p) in stats.iter().enumerate() {
                        if let Some(row) = parts.iter_mut().find(|r| r.name == p.name) {
                            row.quantity += 1;
                            continue;
                        }
                        let kg = masses.get(i).copied().flatten();
                        parts.push(de_shape::drawing::PartRow { name: p.name.clone(), quantity: 1, material: p.material.clone().unwrap_or_else(|| "—".into()), mass: kg.map(kg_text).unwrap_or_else(|| "—".into()) });
                    }
                }
                (material, mass, parts)
            };
            let sheets = setup.sheet_options(&base);
            let pages = sheets.len();
            let number = format!("{}-{}", query.get("project").unwrap_or(&server.default).to_uppercase(), &sid.to_string().replace('-', "")[..6].to_uppercase());
            let drawn_by = std::env::var("USER").map(|u| u[..1].to_uppercase() + &u[1..]).unwrap_or_else(|_| "Forma".into());
            let sheets: Vec<(de_shape::drawing::Options, de_shape::drawing::Title)> = sheets
                .into_iter()
                .enumerate()
                .map(|(i, options)| {
                    let (material, mass, parts) = about(&options.part);
                    let title = de_shape::drawing::Title {
                        title: options.part.clone().unwrap_or_else(|| program.name.clone()),
                        number: number.clone(),
                        material: material.unwrap_or_else(|| "—".into()),
                        mass: mass.map(kg_text).unwrap_or_else(|| "—".into()),
                        parts,
                        drawn_by: drawn_by.clone(),
                        date: today(),
                        page: i + 1,
                        pages,
                    };
                    (options, title)
                })
                .collect();
            Some((program, sheets))
        })
    };
    let Some((program, sheets)) = found else { return fail(StatusCode::NOT_FOUND, format!("{id} is not a shape")) };
    let pages = sheets.len();
    if page.is_some_and(|p| p > pages) {
        return fail(StatusCode::NOT_FOUND, format!("the drawing has {pages} sheet{}", if pages == 1 { "" } else { "s" }));
    }
    // One sheet for the app and a DXF; every sheet for the PDF and the zip, unless one is asked for.
    let wanted: Vec<usize> = match (format, page) {
        ("json" | "dxf", p) => vec![p.unwrap_or(1) - 1],
        (_, Some(p)) => vec![p - 1],
        _ => (0..pages).collect(),
    };
    let key = |(options, title): &(de_shape::drawing::Options, de_shape::drawing::Title)| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&program.source, &program.inputs, format!("{:?}", program.params), format!("{options:?}"), format!("{title:?}")).hash(&mut h);
        h.finish()
    };
    let keys: Vec<u64> = wanted.iter().map(|&i| key(&sheets[i])).collect();
    let mut have: Vec<Option<Arc<de_shape::drawing::Drawing>>> = {
        let cache = server.drawings.lock().unwrap();
        keys.iter().map(|k| cache.get(k).cloned()).collect()
    };
    let missing: Vec<usize> = (0..wanted.len()).filter(|&j| have[j].is_none()).collect();
    if !missing.is_empty() {
        let todo: Vec<(de_shape::drawing::Options, de_shape::drawing::Title)> = missing.iter().map(|&j| sheets[wanted[j]].clone()).collect();
        match tokio::task::spawn_blocking(move || de_shape::drawing::drawings(&program, &todo)).await {
            Ok(Ok(made)) => {
                let mut cache = server.drawings.lock().unwrap();
                if cache.len() + made.len() > 48 {
                    cache.clear();
                }
                for (&j, d) in missing.iter().zip(made) {
                    let d = Arc::new(d);
                    cache.insert(keys[j], d.clone());
                    have[j] = Some(d);
                }
            }
            Ok(Err(e)) => return fail(StatusCode::UNPROCESSABLE_ENTITY, e),
            Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
    let made: Vec<Arc<de_shape::drawing::Drawing>> = have.into_iter().flatten().collect();
    let name = sheets[wanted[0]].1.title.clone();
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let suffix = |i: usize| if pages > 1 { format!("-sheet{}", i + 1) } else { String::new() };
    match format {
        "pdf" => {
            let all: Vec<de_shape::drawing::Drawing> = made.iter().map(|d| (**d).clone()).collect();
            let file = if wanted.len() == 1 && pages > 1 { format!("{safe}{}", suffix(wanted[0])) } else { safe.clone() };
            ([(header::CONTENT_TYPE, "application/pdf".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}.pdf\""))], de_shape::drawing::pdf_pages(&all)).into_response()
        }
        "dxf" => ([(header::CONTENT_TYPE, "application/dxf".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{safe}{}.dxf\"", suffix(wanted[0])))], de_shape::drawing::dxf(&made[0])).into_response(),
        "zip" => {
            let files: Vec<(String, Vec<u8>)> = wanted.iter().zip(&made).map(|(&i, d)| (format!("{safe}{}.dxf", suffix(i)), de_shape::drawing::dxf(d).into_bytes())).collect();
            ([(header::CONTENT_TYPE, "application/zip".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{safe}-dxf.zip\""))], de_shape::drawing::zip(&files)).into_response()
        }
        _ => axum::Json(&*made[0]).into_response(),
    }
}

/// The web app's page is checked with the engine on every load, so a new
/// build shows at once; its bundles are named by content and kept for good.
async fn cache_policy(request: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let fixed = request.uri().path().starts_with("/assets/");
    let mut response = next.run(request).await;
    let html = response.headers().get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/html"));
    let policy = if fixed && response.status().is_success() {
        "public, max-age=31536000, immutable"
    } else if html {
        "no-cache"
    } else {
        return response;
    };
    response.headers_mut().insert(axum::http::header::CACHE_CONTROL, axum::http::HeaderValue::from_static(policy));
    response
}

/// Refuses requests that do not come from this computer, or that a browser
/// sends on behalf of another website.
///
/// - Proxies and tunnels deliver outside requests from this computer's own
///   address; the headers they add give them away.
/// - Any web page can make the browser open a WebSocket to localhost (there is
///   no CORS for WebSockets) or post a form to it; the browser always names
///   the page's origin, which must be this engine's own.
async fn local_only(axum::extract::ConnectInfo(from): axum::extract::ConnectInfo<SocketAddr>, request: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let refuse = |why: &str| (StatusCode::FORBIDDEN, why.to_string()).into_response();
    if !from.ip().is_loopback() {
        return refuse("this engine only answers its own computer here; peers connect at /peer");
    }
    let h = request.headers();
    let proxied = ["x-forwarded-for", "forwarded", "x-real-ip", "cf-connecting-ip", "x-forwarded-host", "true-client-ip"].iter().any(|k| h.contains_key(*k));
    if proxied {
        return refuse("requests through a proxy reach only /peer");
    }
    if let Some(origin) = h.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        let host = origin.strip_prefix("http://").unwrap_or("");
        let local = host.split(':').next().is_some_and(|name| name == "127.0.0.1" || name == "localhost" || name == "[::1]");
        if !local {
            return refuse("only this engine's own pages may use it");
        }
    }
    next.run(request).await
}

/// `GET /peer`: another member's engine connecting (WebSocket).
async fn peer_socket(State(server): State<Arc<Server>>, axum::extract::ConnectInfo(from): axum::extract::ConnectInfo<SocketAddr>, upgrade: WebSocketUpgrade) -> axum::response::Response {
    upgrade.on_upgrade(move |socket| peers::accept(server, socket, from.to_string()))
}

/// `GET /team?project=…`: who is on the team, who is online, sync state.
async fn team_status(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    match server.project(query.get("project")) {
        Ok(app) => axum::Json(peers::status(&server, &app)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct InviteRequest {
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    role: String,
}

/// `POST /team/invite?project=…` `{ name, email, role }`: one person's invite link.
async fn team_invite(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>, axum::Json(body): axum::Json<InviteRequest>) -> axum::response::Response {
    let name = query.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    let app = match server.project(Some(&name)) {
        Ok(app) => app,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    match peers::invite(&server, &name, &app, &body.name, &body.email, &body.role) {
        Ok(v) => {
            peers::start(server.clone(), name, app.clone());
            app.team_changed();
            axum::Json(v).into_response()
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

#[derive(Deserialize)]
struct RemoveMember {
    member: String,
}

/// `POST /team/remove?project=…` `{ member }`: takes someone off the team.
/// Their computer is dropped now and refused from then on, by every member as
/// the news spreads; what they made before stays in the project.
async fn team_remove(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>, axum::Json(body): axum::Json<RemoveMember>) -> axum::response::Response {
    let name = query.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    let app = match server.project(Some(&name)) {
        Ok(app) => app,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    {
        let mut team = app.team.lock().unwrap();
        if let Err(e) = team.remove(&body.member, &server.engine) {
            return (StatusCode::UNPROCESSABLE_ENTITY, e).into_response();
        }
        if let Err(e) = team.save(&server.dir, &name) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    app.team_changed();
    axum::Json(peers::status(&server, &app)).into_response()
}

#[derive(Deserialize)]
struct Settle {
    id: StableId,
    path: String,
    command_id: String,
}

/// `POST /team/conflicts/settle?project=…` `{ id, path, command_id }`: a
/// clash someone has looked at (and kept or changed) leaves the list.
async fn team_settle(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>, axum::Json(body): axum::Json<Settle>) -> axum::response::Response {
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    {
        let mut list = app.conflicts.lock().unwrap();
        list.retain(|c| !(c.id == body.id && c.path == body.path && c.command_id == body.command_id));
        app.save_conflicts(&list);
    }
    app.team_changed();
    axum::Json(peers::status(&server, &app)).into_response()
}

#[derive(Deserialize)]
struct TeamSettings {
    my_name: Option<String>,
    public_url: Option<String>,
}

/// `POST /team/settings?project=…` `{ my_name, public_url }`.
async fn team_settings(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>, axum::Json(body): axum::Json<TeamSettings>) -> axum::response::Response {
    let name = query.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    let app = match server.project(Some(&name)) {
        Ok(app) => app,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    {
        let mut team = app.team.lock().unwrap();
        if let Some(n) = body.my_name.filter(|n| !n.trim().is_empty()) {
            team.my_name = n.trim().to_string();
        }
        if let Some(u) = body.public_url {
            team.public_url = (!u.trim().is_empty()).then(|| u.trim().to_string());
        }
        if let Err(e) = team.save(&server.dir, &name) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }
    app.team_changed();
    axum::Json(peers::status(&server, &app)).into_response()
}

#[derive(Deserialize)]
struct JoinRequest {
    invite: String,
}

/// `POST /join` `{ invite }`: join a teammate's project from their invite.
async fn team_join(State(server): State<Arc<Server>>, axum::Json(body): axum::Json<JoinRequest>) -> axum::response::Response {
    match peers::join(&server, &body.invite) {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

/// `POST /import/<file name>?project=…&user=…` with a STEP file as the body:
/// stores it by content, checks it reads, and adds it to the project as a
/// shape whose parts are the file's bodies (named as in the file).
async fn import(
    State(server): State<Arc<Server>>,
    Path(file): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    let lower = file.to_lowercase();
    if !(lower.ends_with(".step") || lower.ends_with(".stp")) {
        return fail(StatusCode::BAD_REQUEST, "Forma imports STEP files (.step, .stp)".into());
    }
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
    let dir = server.dir.join("assets");
    let stored = dir.join(format!("{sha}.step"));
    if !stored.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&stored, &body)) {
            return fail(StatusCode::INTERNAL_SERVER_ERROR, format!("could not store the file: {e}"));
        }
    }
    // Read it once now: a file that does not read is refused, not stored as a broken part.
    let check = {
        let stored = stored.clone();
        tokio::task::spawn_blocking(move || -> Result<(usize, [f64; 6]), String> {
            let mut a = de_occt::Arena::new();
            let bodies = a.read_step(&stored)?;
            if bodies.is_empty() {
                return Err("the file has no solid bodies".into());
            }
            let mut b = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for (_, s) in &bodies {
                let x = a.bounds(*s)?;
                for k in 0..3 {
                    b[k] = b[k].min(x[k]);
                    b[k + 3] = b[k + 3].max(x[k + 3]);
                }
            }
            Ok((bodies.len(), b))
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    };
    let (count, bounds) = match check {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&stored);
            return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{file} could not be read: {e}"));
        }
    };
    // Show it about as finely as its size calls for (a bolt to ±0.01 mm, a building to ±5).
    let size = (0..3).map(|k| bounds[k + 3] - bounds[k]).fold(0.0, f64::max);
    let shown = (size / 4000.0).clamp(0.005, 5.0);
    let name = file.rsplit_once('.').map(|(n, _)| n).unwrap_or(&file).replace(['_', '-'], " ");
    let source = format!(
        "// Imported from “{file}” ({count} {}). The geometry is the file's own exact B-rep:\n// the view meshes it, STEP exports pass it through untouched.\nreturn importStep(\"{sha}\");",
        if count == 1 { "body" } else { "bodies" }
    );
    let user = query.get("user").filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| "local".into());
    let actor = Actor { user_id: user, session_id: StableId::new().to_string() };
    let revision = app.project.lock().unwrap().revision();
    let message = json!({
        "type": "commit", "request_id": 1, "operation": "shape.create", "expected_revision": revision,
        "parameters": { "name": name, "description": format!("Imported from {file}."), "source": source, "params": [], "display_tolerance": shown }
    });
    let reply: Value = serde_json::from_str(&app.handle(&actor, &message.to_string())).unwrap_or_default();
    if reply["ok"] == true {
        (StatusCode::OK, axum::Json(json!({ "id": reply["value"]["output"]["id"], "bodies": count, "sha": sha }))).into_response()
    } else {
        fail(StatusCode::UNPROCESSABLE_ENTITY, reply["error"]["message"].as_str().unwrap_or("the import failed").to_string())
    }
}

/// Rebuilds a project from its log: changes made here, and peers' changes
/// re-merged exactly as they were (so every copy rebuilds what it held).
fn load(name: &str, path: &PathBuf, registry: Arc<Registry>, engine: &str) -> std::io::Result<Project> {
    let mut project = Project::new(name, registry);
    project.set_origin(engine);
    if path.exists() {
        for (n, line) in BufReader::new(File::open(path)?).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let change: WireChangeSet = serde_json::from_str(&line)
                .map_err(|e| std::io::Error::other(format!("{}:{}: {e}", path.display(), n + 1)))?;
            project
                .replay(change)
                .map_err(|e| std::io::Error::other(format!("{}:{}: {e}", path.display(), n + 1)))?;
        }
    }
    Ok(project)
}

/// This engine's permanent name among peers, made once per projects folder.
fn engine_id(dir: &std::path::Path) -> std::io::Result<String> {
    let path = dir.join(".engine-id");
    if let Ok(id) = std::fs::read_to_string(&path)
        && !id.trim().is_empty()
    {
        return Ok(id.trim().to_string());
    }
    let id = format!("engine-{}", StableId::new());
    std::fs::write(&path, &id)?;
    Ok(id)
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project_path = arg(&args, "--project").map(PathBuf::from).unwrap_or_else(|| root.join("projects/default.jsonl"));
    let web = arg(&args, "--web").map(PathBuf::from).unwrap_or_else(|| root.join("apps/web/dist"));
    let port: u16 = arg(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(7420);

    let dir = project_path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir)?;
    let default = project_path.file_stem().and_then(|s| s.to_str()).unwrap_or("default").to_string();
    let registry = Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &de_assembly::AssemblyPlugin, &IntentPlugin]));
    let claude = ai::AiConfig::find_claude();
    match &claude {
        Some(p) => println!("Forma AI: Claude Code at {}", p.display()),
        None => println!("Forma AI: Claude Code not found (install it, or set FORMA_CLAUDE); the panel stays local"),
    }
    let ai = Arc::new(ai::AiConfig { repo: root.canonicalize().unwrap_or(root.clone()), port, claude, model: std::env::var("FORMA_AI_MODEL").ok() });
    // Imported files, stored by content, shared by every project in the folder.
    de_shape::assets::set_dir(dir.join("assets"));
    let engine = engine_id(&dir)?;
    println!("engine {engine}");
    let machines = machines::MachineQueries::new(&root);
    let server = Arc::new(Server { engine, port, ai, dir, default: default.clone(), registry, open: Mutex::new(HashMap::new()), drawings: Mutex::new(HashMap::new()), machines });
    server.project(None).map_err(std::io::Error::other)?;

    let ui = ServeDir::new(&web).fallback(ServeFile::new(web.join("index.html")));
    let router = Router::new()
        .route("/ws", get(websocket))
        .route("/export/{file}", get(export))
        .route("/projects", get(projects))
        .route("/projects/{name}/info", axum::routing::post(project_info))
        .route("/projects/{name}/duplicate", axum::routing::post(project_duplicate))
        .route("/projects/{name}/delete", axum::routing::post(project_delete))
        .route("/projects/trash", get(trash_list))
        .route("/projects/trash/{entry}/restore", axum::routing::post(trash_restore))
        .route("/projects/{name}/thumbnail", get(thumbnail_get).post(thumbnail_put).layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024)))
        .route("/slicer/{file}", axum::routing::post(send_to_slicer))
        .route("/machines/jcz/status", axum::routing::post(machines::status))
        .route("/gears/{id}", axum::routing::post(gear_check))
        .route("/drawing/{file}", get(drawing))
        .route("/motion/{id}", get(motion))
        .route("/stress/{id}", axum::routing::post(stress))
        .route("/modes/{id}", axum::routing::post(modes))
        .route("/study/{id}", get(study))
        .route("/touching/{id}", get(touching))
        .route("/motion-stress/{id}", get(motion_stress))
        .route("/thermal/{id}", axum::routing::post(thermal))
        .route("/team", get(team_status))
        .route("/team/invite", axum::routing::post(team_invite))
        .route("/team/settings", axum::routing::post(team_settings))
        .route("/team/conflicts/settle", axum::routing::post(team_settle))
        .route("/team/remove", axum::routing::post(team_remove))
        // An invite link opens the join screen (the web app); joining itself is a POST.
        .route("/join", axum::routing::get_service(ServeFile::new(web.join("index.html"))).post(team_join))
        .route("/import/{file}", axum::routing::post(import).layer(axum::extract::DefaultBodyLimit::max(512 * 1024 * 1024)))
        .fallback_service(ui)
        .layer(axum::middleware::from_fn(cache_policy))
        // Everything above answers to this computer only.
        .layer(axum::middleware::from_fn(local_only))
        // Other members' engines reach /peer from the network (they prove membership first).
        .route("/peer", get(peer_socket))
        .with_state(server.clone());

    // Peers reach /peer over the network; everything else is checked to be local.
    let address = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    let local = SocketAddr::from(([127, 0, 0, 1], port));
    if web.join("index.html").exists() {
        println!("open http://{local}");
    } else {
        println!("engine API on ws://{local}/ws (UI not built: run `pnpm build` in apps/web, or `pnpm dev` there)");
    }
    // Projects with a team reconnect to their peers.
    if let Ok(entries) = std::fs::read_dir(&server.dir) {
        for e in entries.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            if let Some(name) = file.strip_suffix(".team.json")
                && let Ok(app) = server.project(Some(&name.to_string()))
            {
                let has = { let t = app.team.lock().unwrap(); !t.key.is_empty() || t.joining.is_some() };
                if has {
                    peers::start(server.clone(), name.to_string(), app);
                }
            }
        }
    }
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}
