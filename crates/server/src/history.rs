//! Named versions, read-only looks at the past, restoring, and branches.
//!
//! The project's own journal is its history (never git): a version names a
//! state by the changes it holds (see `de_core::versions`), and every copy
//! can rebuild that state by replaying them. Here that becomes:
//!
//! - a read-only look: the project rebuilt as of a version, served to the
//!   web app on a session that refuses every change;
//! - compare: what changed from a version to now, entity by entity;
//! - restore: one new change that sets everything back (undoable, synced);
//! - branch: a new project made of the version's changes, which remembers
//!   where it came from (`<name>.branch.json`) and stamps its own changes as
//!   its own engine, so they can later merge back into the original through
//!   the same field-by-field merge teammates use;
//! - pull: the reverse, the original's newer changes brought into the branch
//!   through that same merge.
//!
//! What has crossed between a branch and its original needs no ledger of its
//! own: every change set keeps the stamp (engine and clock) it was made with
//! wherever it lands, so each side's version vector already says which of the
//! other's changes it holds. A pull sends only what the branch's vector does
//! not cover, a merge back only what the original's does not: pulled changes
//! never go back, and pulling or merging again brings only what is new. The
//! branch file keeps the rest: when each last happened, how much it brought,
//! and the clashes already reported on one side (so the same clash is not
//! reported again on the other when the changes behind it cross back).

use crate::{App, Server, bad_request, info_of, pretty_name, valid_name};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use de_core::{Actor, Conflict, Project, StableId, Value, Vector, Version, WireChangeSet, WireOp, json};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Kinds left alone by restore and left out of compare: the names of versions
/// themselves, and conversations about the design (not the design).
pub const KEEP: &[&str] = &["core.version", "comment.thread", "comment.reply"];

/// Looks kept built (rebuilding a big project takes a moment).
const LOOKS_KEPT: usize = 4;

/// Which past state a request names: a named version, or a revision of this copy.
#[derive(Deserialize, Default, Clone)]
pub struct Which {
    #[serde(default)]
    pub version: Option<StableId>,
    #[serde(default)]
    pub revision: Option<u64>,
}

impl Which {
    pub fn from_query(q: &HashMap<String, String>) -> Option<Which> {
        let version = q.get("version").and_then(|v| serde_json::from_value(json!(v)).ok());
        let revision = q.get("at").or(q.get("revision")).and_then(|r| r.parse().ok());
        (version.is_some() || revision.is_some()).then_some(Which { version, revision })
    }
}

/// The state a request names, and how to call it ("v1.2", "Revision 14").
pub fn resolve(project: &Project, which: &Which) -> Result<(Vector, String), String> {
    if let Some(id) = which.version {
        let v = project.get::<Version>(id).ok_or("that version is not in this project (deleted, or not arrived here yet)")?;
        let missing = v.seen.iter().any(|(origin, clock)| project.known().get(origin).copied().unwrap_or(0) < *clock);
        if missing {
            return Err(format!("some changes in “{}” have not reached this computer yet: try again once your teammates are online", v.name));
        }
        return Ok((v.seen.clone(), v.name.clone()));
    }
    if let Some(r) = which.revision {
        let seen = project.vector_at(de_core::Revision(r)).ok_or_else(|| format!("there is no revision {r} (the project is at {})", project.revision().0))?;
        return Ok((seen, format!("Revision {r}")));
    }
    Err("say which version (version) or revision (revision)".into())
}

/// A past state of the project, rebuilt (or taken from the ones kept).
pub fn look(app: &App, seen: &Vector) -> Result<Arc<Mutex<Project>>, String> {
    if let Some((_, p)) = app.looks.lock().unwrap().iter().find(|(v, _)| v == seen) {
        return Ok(p.clone());
    }
    // Copy what is needed out, and rebuild without holding the project.
    let (id, registry, origin, wires) = {
        let p = app.project.lock().unwrap();
        (p.id().to_string(), p.registry().clone(), p.origin().to_string(), p.log_within(seen))
    };
    let then = Project::rebuild(id, registry, &origin, wires).map_err(|e| format!("could not rebuild that version: {}", e.message))?;
    let then = Arc::new(Mutex::new(then));
    let mut looks = app.looks.lock().unwrap();
    looks.insert(0, (seen.clone(), then.clone()));
    looks.truncate(LOOKS_KEPT);
    Ok(then)
}

fn user_of(q: &HashMap<String, String>) -> Actor {
    Actor { user_id: q.get("user").filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| "local".into()), session_id: StableId::new().to_string() }
}

fn app_of(server: &Server, q: &HashMap<String, String>) -> Result<(String, Arc<App>), axum::response::Response> {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    match server.project(Some(&name)) {
        Ok(app) => Ok((name, app)),
        Err(e) => Err(bad_request(e)),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(e.to_string()))
}

/// `GET /versions?project=…`: the named versions, newest first, each with
/// the revision of this copy it matches (if one does), and the branch this
/// project is (if it is one).
pub async fn list(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> axum::response::Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let project = app.project.lock().unwrap();
    let mut versions: Vec<Value> = project
        .entity_ids()
        .into_iter()
        .filter_map(|id| project.get::<Version>(id).map(|v| (id, v.clone())))
        .map(|(id, v)| {
            let here = v.seen.iter().all(|(o, c)| project.known().get(o).copied().unwrap_or(0) >= *c);
            json!({ "id": id, "name": v.name, "note": v.note, "author": v.author, "created": v.created, "revision": project.revision_of(&v.seen).map(|r| r.0), "here": here })
        })
        .collect();
    versions.sort_by_key(|v| std::cmp::Reverse(v["created"].as_u64().unwrap_or(0)));
    let revision = project.revision().0;
    drop(project);
    axum::Json(json!({ "revision": revision, "versions": versions, "branch": branch_status(&server, &name) })).into_response()
}

/// `GET /versions/compare?project=…&version=…` (or `&revision=…`): what
/// changed from that state to now.
pub async fn compare(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> axum::response::Response {
    let (_, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Some(which) = Which::from_query(&q) else { return bad_request("say which version (version) or revision (revision)") };
    let result = blocking(move || {
        let (seen, label) = resolve(&app.project.lock().unwrap(), &which)?;
        let then = look(&app, &seen)?;
        let then = then.lock().unwrap();
        let now = app.project.lock().unwrap();
        let changes = de_core::versions::compare(&then, &now, KEEP);
        let count = |k: &str| changes.iter().filter(|c| c.change == k).count();
        Ok(json!({
            "label": label,
            "then_revision": then.revision().0,
            "now_revision": now.revision().0,
            "added": count("added"),
            "removed": count("removed"),
            "changed": count("changed"),
            "changes": changes,
        }))
    })
    .await;
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => bad_request(e),
    }
}

/// `POST /versions/restore?project=…&user=…` `{ version | revision }`: sets
/// the design back to that state as one new change (comments and versions
/// stay as they are). It is undoable, and reaches teammates like any edit.
pub async fn restore(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>, axum::Json(which): axum::Json<Which>) -> axum::response::Response {
    let (_, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let actor = user_of(&q);
    let result = blocking(move || {
        let (seen, label) = resolve(&app.project.lock().unwrap(), &which)?;
        let then = look(&app, &seen)?;
        let then = then.lock().unwrap();
        let mut project = app.project.lock().unwrap();
        let (logged, before) = (project.log().len(), project.revision());
        let changes = de_core::versions::compare(&project, &then, KEEP).len();
        if changes == 0 {
            return Ok(json!({ "restored": label, "changed": 0, "revision": before.0 }));
        }
        let done = project.restore(&then, &actor, KEEP, json!({ "restored": label, "version": which.version, "revision": which.revision })).map_err(|e| e.message)?;
        app.publish(&project, logged, before).map_err(|e| format!("restored but not saved: {e}"))?;
        Ok(json!({ "restored": label, "changed": changes, "revision": done.revision.0 }))
    })
    .await;
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => bad_request(e),
    }
}

// ---- Branches --------------------------------------------------------------

/// What a branch remembers about where it came from (`<name>.branch.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Branch {
    /// The project it was branched from.
    pub from: String,
    /// That project's title then.
    #[serde(default)]
    pub from_title: String,
    /// The version it started from, if it was a named one, and its name.
    #[serde(default)]
    pub version: Option<StableId>,
    pub label: String,
    /// The changes it started with (the version's).
    pub base: Vector,
    /// The engine name its own changes are stamped with.
    pub origin: String,
    #[serde(default)]
    pub created: u64,
    /// The last merge back: when (s), and how many changes it brought.
    #[serde(default)]
    pub merged_at: Option<u64>,
    #[serde(default)]
    pub merged: usize,
    /// The last pull from the original: when (s), and how many changes it brought in all.
    #[serde(default)]
    pub pulled_at: Option<u64>,
    #[serde(default)]
    pub pulled: usize,
    /// Clashes already reported on one side (most recent first, a few hundred kept).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clashed: Vec<Clashed>,
}

/// A clash reported when changes crossed one way. The changes the receiving
/// side held then (`held`) are the ones it clashed with: when one of them
/// crosses the other way and clashes on the same field, it is the same clash
/// met from the other side (already listed where it first came up, and
/// settled there), not a new one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clashed {
    /// Where it was reported: "branch" (on a pull) or "original" (on a merge back).
    pub on: String,
    pub id: StableId,
    pub kind: String,
    pub path: String,
    pub held: Vector,
}

const CLASHES_KEPT: usize = 400;

impl Clashed {
    /// Whether `c`, reported on the other side for the change set stamped
    /// `stamp`, is this clash met again.
    fn again(&self, c: &Conflict, stamp: Option<&de_core::Stamp>) -> bool {
        let overlaps = |a: &str, b: &str| a.is_empty() || b.is_empty() || a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"));
        self.id == c.id && self.kind == c.kind && overlaps(&self.path, &c.path) && stamp.is_some_and(|s| de_core::versions::covers(&self.held, &s.origin, s.clock))
    }
}

impl Branch {
    pub fn path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        dir.join(format!("{name}.branch.json"))
    }

    pub fn load(dir: &std::path::Path, name: &str) -> Option<Branch> {
        std::fs::read_to_string(Self::path(dir, name)).ok().and_then(|t| serde_json::from_str(&t).ok())
    }

    fn save(&self, dir: &std::path::Path, name: &str) -> std::io::Result<()> {
        std::fs::write(Self::path(dir, name), serde_json::to_string_pretty(self).unwrap())
    }
}

/// The engine name a project's changes are stamped with: this engine's, or a
/// branch's own (so they never pass for the original's when merged back).
pub fn origin_for(dir: &std::path::Path, name: &str, engine: &str) -> String {
    Branch::load(dir, name).map(|b| b.origin).unwrap_or_else(|| engine.to_string())
}

/// A branch's changes made since it started (what merging back brings).
fn fresh(branch: &Project, base: &Vector) -> Vec<WireChangeSet> {
    branch.log_beyond(base)
}

/// A change set that only names the project (its title and description):
/// each side keeps its own name, so it is not something to bring over.
fn names_only(w: &WireChangeSet) -> bool {
    w.ops.iter().all(|op| match op {
        WireOp::Set { kind, .. } => kind == "core.project",
        WireOp::Create { id } | WireOp::Delete { id } => w.ops.iter().any(|o| matches!(o, WireOp::Set { id: i, kind, .. } if i == id && kind == "core.project")),
    })
}

/// The change set without the project's name in it. (It still crosses, empty
/// if need be: later ones may build on it.)
fn without_name(mut w: WireChangeSet) -> WireChangeSet {
    w.ops.retain(|op| !matches!(op, WireOp::Set { kind, .. } if kind == "core.project"));
    w
}

/// Of `changes`, the ones not covered by `known` (what the side holding it lacks).
fn beyond(changes: Vec<WireChangeSet>, known: &Vector) -> Vec<WireChangeSet> {
    changes.into_iter().filter(|w| w.stamp.as_ref().is_some_and(|s| !de_core::versions::covers(known, &s.origin, s.clock))).collect()
}

/// Of `changes`, the ones worth counting: not empty, not just a name, and not
/// the naming of the version the branch started from (made after the state it
/// names, so never in the branch at first; it crosses with the first pull).
fn counted(changes: &[WireChangeSet], start: Option<StableId>) -> usize {
    let only_start = |w: &WireChangeSet| start.is_some_and(|v| w.ops.iter().all(|op| matches!(op, WireOp::Set { id, .. } | WireOp::Create { id } | WireOp::Delete { id } if *id == v)));
    changes.iter().filter(|w| !w.ops.is_empty() && !names_only(w) && !only_start(w)).count()
}

/// What the branch panel shows: where it came from, what is not merged back
/// yet (`ahead`), and what the original has that the branch does not
/// (`behind`, what a pull brings).
pub fn branch_status(server: &Server, name: &str) -> Value {
    let Some(b) = Branch::load(&server.dir, name) else { return Value::Null };
    let exists = server.dir.join(format!("{}.jsonl", b.from)).exists();
    let (ahead, behind) = match (server.project(Some(&name.to_string())), exists.then(|| server.project(Some(&b.from))).and_then(Result::ok)) {
        (Ok(app), Some(original)) => {
            // One project locked at a time (another request may lock them the other way round).
            let (mine, known_here) = {
                let p = app.project.lock().unwrap();
                (fresh(&p, &b.base), p.known().clone())
            };
            let (theirs, known_there) = {
                let p = original.project.lock().unwrap();
                (beyond(p.log_beyond(&known_here), &known_here), p.known().clone())
            };
            (counted(&beyond(mine, &known_there), None), counted(&theirs, b.version))
        }
        _ => (0, 0),
    };
    let from_title = if exists { info_of(&server.dir, &b.from)["title"].as_str().map(str::to_string).unwrap_or_else(|| pretty_name(&b.from)) } else { b.from_title.clone() };
    json!({
        "from": b.from,
        "from_title": from_title,
        "from_exists": exists,
        "version": b.version,
        "label": b.label,
        "created": b.created * 1000,
        "merged_at": b.merged_at.map(|t| t * 1000),
        "merged": b.merged,
        "pulled_at": b.pulled_at.map(|t| t * 1000),
        "pulled": b.pulled,
        "ahead": ahead,
        "behind": behind,
    })
}

#[derive(Deserialize)]
pub struct BranchRequest {
    #[serde(flatten)]
    which: Which,
    /// The new project's name (default: made from the original's and the version's).
    #[serde(default)]
    to: Option<String>,
}

fn slug(text: &str) -> String {
    let s: String = text.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// `POST /versions/branch?project=…` `{ version | revision, to? }`: a new
/// project holding that version's changes, titled “X — branch from v1.2”,
/// which remembers where it came from.
pub async fn branch(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>, axum::Json(req): axum::Json<BranchRequest>) -> axum::response::Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let server2 = server.clone();
    let result = blocking(move || {
        let server = server2;
        let (seen, label, wires) = {
            let p = app.project.lock().unwrap();
            let (seen, label) = resolve(&p, &req.which)?;
            let wires = p.log_within(&seen);
            (seen, label, wires)
        };
        let to = match req.to.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            Some(t) if valid_name(&t) => t,
            Some(_) => return Err("use lowercase letters, digits and dashes".to_string()),
            None => {
                let stem: String = format!("{}-{}", name, slug(&label)).chars().take(56).collect::<String>().trim_end_matches('-').to_string();
                let mut to = stem.clone();
                let mut n = 1;
                while server.dir.join(format!("{to}.jsonl")).exists() {
                    n += 1;
                    to = format!("{stem}-{n}");
                }
                to
            }
        };
        if server.dir.join(format!("{to}.jsonl")).exists() {
            return Err("there is already a project called that".into());
        }
        let title = info_of(&server.dir, &name)["title"].as_str().map(str::to_string).unwrap_or_else(|| pretty_name(&name));
        let record = Branch {
            from: name.clone(),
            from_title: title.clone(),
            version: req.which.version,
            label: label.clone(),
            base: seen,
            origin: format!("{}~{}", server.engine, StableId::new()),
            created: crate::peers::now(),
            merged_at: None,
            merged: 0,
            pulled_at: None,
            pulled: 0,
            clashed: Vec::new(),
        };
        // The branch file first (it says how the project's changes are stamped), then its log.
        record.save(&server.dir, &to).map_err(|e| e.to_string())?;
        let mut text = String::new();
        for w in &wires {
            text.push_str(&serde_json::to_string(w).map_err(|e| e.to_string())?);
            text.push('\n');
        }
        if let Err(e) = std::fs::write(server.dir.join(format!("{to}.jsonl")), text) {
            let _ = std::fs::remove_file(Branch::path(&server.dir, &to));
            return Err(e.to_string());
        }
        let _ = std::fs::copy(server.dir.join(format!("{name}.thumb.png")), server.dir.join(format!("{to}.thumb.png")));
        let branch = server.project(Some(&to))?;
        branch.commit_here("project.set_info", json!({ "title": format!("{title} — branch from {label}").chars().take(120).collect::<String>() }))?;
        Ok(json!({ "name": to, "label": label }))
    })
    .await;
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => bad_request(e),
    }
}

/// `GET /branch?project=…`: where this branch came from, and what is not merged back.
pub async fn status(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> axum::response::Response {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    if !valid_name(&name) {
        return bad_request("not a project name");
    }
    axum::Json(branch_status(&server, &name)).into_response()
}

/// Merges a branch's changes since it started into the project it came
/// from, through the same merge teammates' changes go through: what does not
/// clash lands, and fields both changed differently are reported as
/// conflicts (in the original's team panel, like teammates' clashes).
/// Merging again brings only what is new, and changes pulled from the
/// original do not go back. The branch's own title and description stay on
/// the branch.
pub fn merge_back(server: &Server, name: &str) -> Result<(String, usize, Vec<Conflict>), String> {
    cross(server, name, Way::Merge)
}

/// Brings the original's changes the branch does not have yet into the
/// branch, through the same merge: what does not clash lands, and fields both
/// changed differently are reported as conflicts in the branch's team panel.
/// Pulling again brings only what is new, and pulled changes are never merged
/// back. The original's title and description stay on the original.
pub fn pull(server: &Server, name: &str) -> Result<(String, usize, Vec<Conflict>), String> {
    cross(server, name, Way::Pull)
}

#[derive(Clone, Copy, PartialEq)]
enum Way {
    /// Branch → original.
    Merge,
    /// Original → branch.
    Pull,
}

impl Way {
    /// The side receiving, and the side sending (as `Clashed::on` names them).
    fn sides(self) -> (&'static str, &'static str) {
        match self {
            Way::Merge => ("original", "branch"),
            Way::Pull => ("branch", "original"),
        }
    }
}

/// Sends what one side has and the other lacks across (see `merge_back`, `pull`).
fn cross(server: &Server, name: &str, way: Way) -> Result<(String, usize, Vec<Conflict>), String> {
    let mut record = Branch::load(&server.dir, name).ok_or("this project is not a branch")?;
    if !server.dir.join(format!("{}.jsonl", record.from)).exists() {
        return Err(format!("the project it was branched from ({}) is not here any more", record.from));
    }
    let branch = server.project(Some(&name.to_string()))?;
    let original = server.project(Some(&record.from))?;
    let (from, to) = match way {
        Way::Merge => (&branch, &original),
        Way::Pull => (&original, &branch),
    };
    // One project locked at a time (another request may lock them the other way round).
    let held = to.project.lock().unwrap().known().clone();
    let changes: Vec<WireChangeSet> = {
        let p = from.project.lock().unwrap();
        let candidates = match way {
            Way::Merge => fresh(&p, &record.base),
            Way::Pull => p.log_beyond(&held),
        };
        beyond(candidates, &held).into_iter().map(without_name).collect()
    };
    let stamps: HashMap<String, de_core::Stamp> = changes.iter().filter_map(|w| Some((w.command_id.clone(), w.stamp.clone()?))).collect();
    // Counted as the branch panel counts them (so "3 newer" pulls 3).
    let landed = counted(&changes, if way == Way::Pull { record.version } else { None });
    let (here, there) = way.sides();
    // A clash first reported on the other side, met again as its changes cross back, is not listed twice.
    let conflicts = to.receive_except(changes, |c| record.clashed.iter().any(|k| k.on == there && k.again(c, stamps.get(&c.command_id))))?;
    let waiting = to.project.lock().unwrap().waiting();
    if waiting > 0 {
        return Err(match way {
            Way::Merge => format!("{waiting} of the branch's changes build on changes the original does not have (made by teammates on the branch who are not on the original): they wait until those arrive"),
            Way::Pull => format!("{waiting} of the original's changes build on changes this branch does not have: they wait until those arrive"),
        });
    }
    for c in conflicts.iter().rev() {
        record.clashed.insert(0, Clashed { on: here.into(), id: c.id, kind: c.kind.clone(), path: c.path.clone(), held: held.clone() });
    }
    record.clashed.truncate(CLASHES_KEPT);
    let now = Some(crate::peers::now());
    match way {
        Way::Merge => (record.merged_at, record.merged) = (now, record.merged + landed),
        Way::Pull => (record.pulled_at, record.pulled) = (now, record.pulled + landed),
    }
    record.save(&server.dir, name).map_err(|e| e.to_string())?;
    branch.team_changed();
    Ok((record.from, landed, conflicts))
}

/// `POST /branch/merge?project=…`: merges this branch back into its original.
pub async fn merge(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> axum::response::Response {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    if !valid_name(&name) {
        return bad_request("not a project name");
    }
    let s = server.clone();
    let n = name.clone();
    match blocking(move || merge_back(&s, &n)).await {
        Ok((into, landed, conflicts)) => axum::Json(json!({ "into": into, "changes": landed, "conflicts": conflicts, "branch": branch_status(&server, &name) })).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

/// `POST /branch/pull?project=…`: brings the original's newer changes into this branch.
pub async fn pull_in(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> axum::response::Response {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    if !valid_name(&name) {
        return bad_request("not a project name");
    }
    let s = server.clone();
    let n = name.clone();
    match blocking(move || pull(&s, &n)).await {
        Ok((from, landed, conflicts)) => axum::Json(json!({ "from": from, "changes": landed, "conflicts": conflicts, "branch": branch_status(&server, &name) })).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}
