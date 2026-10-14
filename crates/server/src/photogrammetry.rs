//! Photogrammetry: photos of an object from many angles turned into a scan.
//!
//! - `POST /photos/<set>/<file>?project=…` with a JPG, PNG or HEIC photo (or
//!   a .zip of them) as the body: kept by content in the folder's store and
//!   listed in `<project>.photos.json` under the set's name.
//! - `GET /photos?project=…`: the photo sets.
//! - `GET /photogrammetry/engines`: which engines can run here, and what is
//!   missing for the others.
//! - `POST /photogrammetry?project=…&set=…[&engine=auto|object-capture|colmap]
//!   [&detail=preview|reduced|medium|full][&mask=1][&clean=0][&name=…]`:
//!   starts a background job; its progress goes to the project's views as
//!   `{ "type": "photogrammetry", "job": … }` events.
//! - `GET /photogrammetry?project=…`: the jobs; `POST /photogrammetry/<job>/cancel`.
//! - `GET /photogrammetry/markers.pdf[?paper=letter]`: the marker sheet to print.
//!
//! Engines (no commercial SDKs): Apple Object Capture through a small Swift
//! helper built on first use (`tools/photogrammetry/ObjectCapture.swift`),
//! and COLMAP (BSD) through its command line: structure from motion, then
//! multi-view stereo where COLMAP has CUDA, else Iṣẹ́ Studio's own on the CPU
//! (`de_scan::mvs`). The result is sized and levelled from the marker sheet
//! when the photos show it (see `de_scan::photo`), cleaned (floor cut away,
//! what stands in the field kept whole, holes filled; scenes walked through
//! are kept whole), and kept as scans whose entry carries the provenance: engine,
//! photos used and rejected, camera poses, scale method and its uncertainty,
//! reprojection error and coverage gaps.

use crate::{Server, json};
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::Value;
use de_scan::photo::{self, Camera, Scaling, Similarity};
use de_scan::{Cloud, Mesh, V3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Read};
use std::path::{Path as FsPath, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn project_name(server: &Server, query: &HashMap<String, String>) -> String {
    query.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone())
}

fn fail(code: StatusCode, message: String) -> axum::response::Response {
    (code, message).into_response()
}

// ---- Photo sets -----------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Photo {
    pub name: String,
    /// Its id in the folder's store.
    pub id: String,
    /// "jpg", "png" or "heic".
    pub kind: String,
    pub bytes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PhotoSet {
    pub name: String,
    pub photos: Vec<Photo>,
    pub added: u64,
}

fn sets_path(server: &Server, project: &str) -> PathBuf {
    server.dir.join(format!("{project}.photos.json"))
}

pub fn read_sets(server: &Server, project: &str) -> Vec<PhotoSet> {
    std::fs::read_to_string(sets_path(server, project)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_sets(server: &Server, project: &str, sets: &[PhotoSet]) -> Result<(), String> {
    std::fs::write(sets_path(server, project), serde_json::to_string_pretty(sets).unwrap()).map_err(|e| format!("could not save the photo sets: {e}"))
}

/// What a photo's bytes are: "jpg", "png" or "heic".
pub fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("jpg");
    }
    if bytes.starts_with(b"\x89PNG") {
        return Some("png");
    }
    if bytes.len() > 12 && &bytes[4..8] == b"ftyp" && [&b"heic"[..], b"heix", b"hevc", b"heim", b"heis", b"mif1", b"msf1"].contains(&&bytes[8..12]) {
        return Some("heic");
    }
    None
}

/// The files in a zip archive (stored or deflated entries), by the central directory.
pub fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16le = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or("the zip is cut short");
    let u32le = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or("the zip is cut short");
    let end = (0..bytes.len().saturating_sub(21)).rev().find(|&i| bytes[i..].starts_with(b"PK\x05\x06")).ok_or("not a zip file")?;
    let count = u16le(end + 10)?;
    let mut at = u32le(end + 16)?;
    let mut out = Vec::new();
    for _ in 0..count {
        if !bytes.get(at..).is_some_and(|b| b.starts_with(b"PK\x01\x02")) {
            return Err("the zip's directory is damaged".into());
        }
        let method = u16le(at + 10)?;
        let size = u32le(at + 20)?;
        let real = u32le(at + 24)?;
        let (nlen, xlen, clen) = (u16le(at + 28)?, u16le(at + 30)?, u16le(at + 32)?);
        let local = u32le(at + 42)?;
        let name = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + nlen).ok_or("the zip is cut short")?).to_string();
        at += 46 + nlen + xlen + clen;
        if name.ends_with('/') {
            continue;
        }
        let start = local + 30 + u16le(local + 26)? + u16le(local + 28)?;
        let data = bytes.get(start..start + size).ok_or("the zip is cut short")?;
        let content = match method {
            0 => data.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(real);
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut v).map_err(|e| format!("{name}: {e}"))?;
                v
            }
            m => return Err(format!("{name}: zip compression method {m} is not read (save the zip with plain deflate)")),
        };
        out.push((name, content));
    }
    Ok(out)
}

fn set_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 80 && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_ .".contains(c))
}

/// `POST /photos/<set>/<file>`: a photo (or a zip of them) added to a set.
pub async fn add_photo(State(server): State<Arc<Server>>, Path((set, file)): Path<(String, String)>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    if let Err(e) = server.project(query.get("project")) {
        return fail(StatusCode::BAD_REQUEST, e);
    }
    if !set_name_ok(&set) {
        return fail(StatusCode::BAD_REQUEST, "a photo set's name is letters, digits, spaces, dots, dashes and underscores".into());
    }
    let project = project_name(&server, &query);
    let s = server.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let files = if body.starts_with(b"PK\x03\x04") { unzip(&body)? } else { vec![(file.clone(), body.to_vec())] };
        let mut added = Vec::new();
        let mut skipped = Vec::new();
        let mut sets = read_sets(&s, &project);
        let i = match sets.iter().position(|x| x.name == set) {
            Some(i) => i,
            None => {
                sets.push(PhotoSet { name: set.clone(), photos: vec![], added: now_ms() });
                sets.len() - 1
            }
        };
        for (name, bytes) in files {
            let base = name.rsplit('/').next().unwrap_or(&name).to_string();
            if base.starts_with('.') {
                continue;
            }
            let Some(kind) = image_kind(&bytes) else {
                skipped.push(base);
                continue;
            };
            let id = de_shape::assets::store(&bytes)?;
            sets[i].photos.retain(|p| p.name != base);
            sets[i].photos.push(Photo { name: base.clone(), id, kind: kind.into(), bytes: bytes.len() });
            added.push(base);
        }
        if added.is_empty() {
            return Err(format!("no photos in {file} (ISE Studio takes JPG, PNG and HEIC){}", if skipped.is_empty() { String::new() } else { format!("; skipped {}", skipped.join(", ")) }));
        }
        sets[i].photos.sort_by(|a, b| a.name.cmp(&b.name));
        write_sets(&s, &project, &sets)?;
        Ok(json!({ "set": sets[i].name, "photos": sets[i].photos.len(), "added": added, "skipped": skipped }))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// `GET /photos`: the project's photo sets.
pub async fn list_photos(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    axum::Json(read_sets(&server, &project_name(&server, &query))).into_response()
}

/// `GET /photogrammetry/markers.pdf[?paper=letter]`: the marker sheet.
pub async fn markers_pdf(Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let letter = query.get("paper").is_some_and(|p| p.eq_ignore_ascii_case("letter"));
    let pdf = de_scan::markers::Sheet::default().pdf(letter);
    ([(header::CONTENT_TYPE, "application/pdf"), (header::CONTENT_DISPOSITION, "inline; filename=\"ise-scale-markers.pdf\"")], pdf).into_response()
}

// ---- Engines --------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct EngineInfo {
    pub name: String,
    pub available: bool,
    /// What it does here, or what is missing.
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Dense multi-view stereo (COLMAP needs CUDA for it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense: Option<bool>,
}

const HELPER_SOURCE: &str = include_str!("../../../tools/photogrammetry/ObjectCapture.swift");

fn find_program(name: &str, env: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(env).map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// The Object Capture helper, built from its Swift source on first use.
fn object_capture_helper(server: &Server) -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("Object Capture is Apple's, on macOS only".into());
    }
    let hash = &de_shape::assets::sha256(HELPER_SOURCE.as_bytes())[..12];
    let dir = server.dir.join(".tools");
    let bin = dir.join(format!("ise-object-capture-{hash}"));
    if bin.is_file() {
        return Ok(bin);
    }
    static BUILD: Mutex<()> = Mutex::new(());
    let _g = BUILD.lock().unwrap();
    if bin.is_file() {
        return Ok(bin);
    }
    let swiftc = find_program("swiftc", "ISE_SWIFTC").ok_or("Object Capture needs the Swift compiler to build ISE Studio's helper: install the Xcode command line tools (xcode-select --install)")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = dir.join(format!("ObjectCapture-{hash}.swift"));
    std::fs::write(&src, HELPER_SOURCE).map_err(|e| e.to_string())?;
    let out = Command::new(swiftc).arg("-O").arg(&src).arg("-o").arg(&bin).output().map_err(|e| format!("could not run swiftc: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&bin);
        return Err(format!("building the Object Capture helper failed (RealityKit's PhotogrammetrySession needs macOS 12 or later): {}", String::from_utf8_lossy(&out.stderr).lines().take(6).collect::<Vec<_>>().join(" / ")));
    }
    Ok(bin)
}

fn object_capture_info(server: &Server) -> EngineInfo {
    static CHECK: OnceLock<EngineInfo> = OnceLock::new();
    if let Some(i) = CHECK.get() {
        return i.clone();
    }
    let info = match object_capture_helper(server) {
        Err(e) => EngineInfo { name: "object-capture".into(), available: false, detail: e, version: None, dense: None },
        Ok(bin) => match Command::new(&bin).arg("--check").output() {
            Ok(o) => {
                let v: Value = String::from_utf8_lossy(&o.stdout).lines().find_map(|l| serde_json::from_str(l).ok()).unwrap_or(Value::Null);
                let os = v["os"].as_str().map(str::to_string);
                if v["supported"] == json!(true) {
                    EngineInfo { name: "object-capture".into(), available: true, detail: "Apple Object Capture: a dense, textured mesh; uses depth in iPhone photos for true size".into(), version: os, dense: Some(true) }
                } else {
                    EngineInfo { name: "object-capture".into(), available: false, detail: "this Mac cannot run Object Capture (it needs Apple silicon, or a GPU with 4 GB and ray tracing)".into(), version: os, dense: None }
                }
            }
            Err(e) => EngineInfo { name: "object-capture".into(), available: false, detail: format!("the helper would not run: {e}"), version: None, dense: None },
        },
    };
    let _ = CHECK.set(info.clone());
    info
}

fn colmap_info() -> EngineInfo {
    let Some(bin) = find_program("colmap", "ISE_COLMAP") else {
        return EngineInfo { name: "colmap".into(), available: false, detail: "COLMAP is not installed: brew install colmap (macOS), apt install colmap (Linux), or the Windows release from colmap.github.io".into(), version: None, dense: None };
    };
    let text = Command::new(&bin).arg("help").output().map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))).unwrap_or_default();
    let line = text.lines().find(|l| l.contains("COLMAP ")).map(|l| l[l.find("COLMAP ").unwrap()..].trim().to_string());
    let cuda = text.contains("with CUDA");
    EngineInfo {
        name: "colmap".into(),
        available: true,
        detail: if cuda { "COLMAP: structure from motion, then dense multi-view stereo (CUDA) and a Poisson mesh".into() } else { "COLMAP: structure from motion; this build has no CUDA, so ISE Studio's own dense matching runs on the CPU (slower than CUDA), then a Poisson mesh".into() },
        version: line,
        dense: Some(cuda),
    }
}

/// `GET /photogrammetry/engines`
pub async fn engines(State(server): State<Arc<Server>>) -> axum::response::Response {
    let s = server.clone();
    let list = tokio::task::spawn_blocking(move || vec![object_capture_info(&s), colmap_info()]).await.unwrap_or_default();
    axum::Json(json!({ "engines": list })).into_response()
}

// ---- Jobs -----------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub project: String,
    pub set: String,
    pub name: String,
    pub engine: String,
    pub detail: String,
    pub mask: bool,
    pub clean: bool,
    /// "queued", "running", "done", "failed" or "cancelled".
    pub state: String,
    pub stage: String,
    /// 0…1 over the whole job.
    pub progress: f64,
    pub log: Vec<String>,
    pub images: usize,
    pub started: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

impl Job {
    /// A stage reached: `within` is the stage's own progress (0…1), placed in [lo, hi] of the job.
    pub fn advance(&mut self, stage: &str, lo: f64, hi: f64, within: f64) {
        if self.stage != stage {
            self.stage = stage.to_string();
            self.note(stage);
        }
        let p = lo + (hi - lo) * within.clamp(0.0, 1.0);
        // Never backwards within a job.
        self.progress = self.progress.max(p).min(1.0);
    }

    pub fn note(&mut self, line: &str) {
        self.log.push(line.to_string());
        if self.log.len() > 60 {
            self.log.remove(0);
        }
    }

    pub fn end(&mut self, outcome: Result<Value, String>, cancelled: bool) {
        self.finished = Some(now_ms());
        match outcome {
            Ok(v) => {
                self.state = "done".into();
                self.progress = 1.0;
                self.stage = "done".into();
                self.result = Some(v);
            }
            Err(e) => {
                self.state = if cancelled { "cancelled" } else { "failed" }.into();
                self.error = Some(if cancelled { "cancelled".into() } else { e });
            }
        }
    }

    pub fn active(&self) -> bool {
        self.state == "queued" || self.state == "running"
    }
}

fn jobs() -> &'static Mutex<Vec<Job>> {
    static JOBS: OnceLock<Mutex<Vec<Job>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

fn cancels() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static C: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// One reconstruction at a time (they take the whole machine).
static RUNNING: Mutex<()> = Mutex::new(());

/// Changes a job and tells the project's views.
fn update(server: &Server, id: &str, f: impl FnOnce(&mut Job)) {
    let job = {
        let mut list = jobs().lock().unwrap();
        let Some(j) = list.iter_mut().find(|j| j.id == id) else { return };
        f(j);
        j.clone()
    };
    if let Ok(app) = server.project(Some(&job.project)) {
        let _ = app.events.send(json!({ "type": "photogrammetry", "job": job }).to_string().into());
    }
}

/// `GET /photogrammetry?project=…`: this project's jobs, newest first.
pub async fn list_jobs(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let project = project_name(&server, &query);
    let mut list: Vec<Job> = jobs().lock().unwrap().iter().filter(|j| j.project == project).cloned().collect();
    list.reverse();
    axum::Json(list).into_response()
}

/// `POST /photogrammetry/<job>/cancel`
pub async fn cancel(Path(id): Path<String>) -> axum::response::Response {
    if !jobs().lock().unwrap().iter().any(|j| j.id == id && j.active()) {
        return fail(StatusCode::NOT_FOUND, format!("no running job {id}"));
    }
    match cancels().lock().unwrap().get(&id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            axum::Json(json!({ "cancelling": id })).into_response()
        }
        None => fail(StatusCode::NOT_FOUND, format!("no running job {id}")),
    }
}

/// `POST /photogrammetry?project=…&set=…`: starts a reconstruction.
pub async fn start(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    if let Err(e) = server.project(query.get("project")) {
        return fail(StatusCode::BAD_REQUEST, e);
    }
    let project = project_name(&server, &query);
    let Some(set) = query.get("set").cloned() else {
        return fail(StatusCode::BAD_REQUEST, "say which photo set (set=…); add photos with POST /photos/<set>/<file>".into());
    };
    let Some(photos) = read_sets(&server, &project).into_iter().find(|s| s.name == set) else {
        return fail(StatusCode::NOT_FOUND, format!("no photo set {set} in this project"));
    };
    if photos.photos.len() < 3 {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{set} has {} photo(s): photogrammetry needs many (30–80 round the object)", photos.photos.len()));
    }
    let detail = query.get("detail").cloned().unwrap_or_else(|| "medium".into());
    if !["preview", "reduced", "medium", "full"].contains(&detail.as_str()) {
        return fail(StatusCode::BAD_REQUEST, "detail is preview, reduced, medium or full".into());
    }
    let want = query.get("engine").cloned().unwrap_or_else(|| "auto".into());
    let s = server.clone();
    let infos = tokio::task::spawn_blocking(move || vec![object_capture_info(&s), colmap_info()]).await.unwrap_or_default();
    let engine = match want.as_str() {
        "auto" => match infos.iter().find(|i| i.available) {
            Some(i) => i.name.clone(),
            None => return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("no photogrammetry engine can run here: {}", infos.iter().map(|i| format!("{}: {}", i.name, i.detail)).collect::<Vec<_>>().join("; "))),
        },
        name => match infos.iter().find(|i| i.name == name) {
            Some(i) if i.available => i.name.clone(),
            Some(i) => return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{name} cannot run here: {}", i.detail)),
            None => return fail(StatusCode::BAD_REQUEST, "engine is auto, object-capture or colmap".into()),
        },
    };
    let on = |k: &str, d: bool| query.get(k).map(|v| v != "0" && v != "false").unwrap_or(d);
    let job = Job {
        id: de_core::StableId::new().to_string(),
        project: project.clone(),
        set: set.clone(),
        name: query.get("name").cloned().unwrap_or_else(|| set.clone()),
        engine,
        detail,
        mask: on("mask", false),
        clean: on("clean", true),
        state: "queued".into(),
        stage: "queued".into(),
        progress: 0.0,
        log: vec![],
        images: photos.photos.len(),
        started: now_ms(),
        finished: None,
        error: None,
        result: None,
    };
    let flag = Arc::new(AtomicBool::new(false));
    cancels().lock().unwrap().insert(job.id.clone(), flag.clone());
    jobs().lock().unwrap().push(job.clone());
    let s = server.clone();
    let id = job.id.clone();
    std::thread::spawn(move || {
        let _turn = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
        update(&s, &id, |j| {
            j.state = "running".into();
            j.started = now_ms();
        });
        let outcome = if flag.load(Ordering::Relaxed) { Err("cancelled".into()) } else { run(&s, &id, &photos, &flag) };
        let cancelled = flag.load(Ordering::Relaxed);
        update(&s, &id, |j| j.end(outcome, cancelled));
        cancels().lock().unwrap().remove(&id);
    });
    axum::Json(job).into_response()
}

// ---- Running a job ----------------------------------------------------------------

/// Runs a program, passing each output line (stdout and stderr) to `line`,
/// stopping it when the job is cancelled.
fn run_program(cmd: Command, flag: &AtomicBool, mut line: impl FnMut(&str)) -> Result<(), String> {
    run_program_talk(cmd, flag, false, |l| {
        line(l);
        None
    })
}

/// As `run_program`; with `talk`, what `line` answers is written to the program's stdin as a line.
fn run_program_talk(mut cmd: Command, flag: &AtomicBool, talk: bool, mut line: impl FnMut(&str) -> Option<String>) -> Result<(), String> {
    use std::io::Write;
    let name = cmd.get_program().to_string_lossy().rsplit('/').next().unwrap_or("program").to_string();
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if talk { Stdio::piped() } else { Stdio::null() }).spawn().map_err(|e| format!("could not run {name}: {e}"))?;
    let mut stdin = child.stdin.take();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for l in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        }));
    }
    drop(tx);
    let mut tail: Vec<String> = Vec::new();
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(l) => {
                if let (Some(reply), Some(input)) = (line(&l), stdin.as_mut()) {
                    let _ = writeln!(input, "{reply}");
                    let _ = input.flush();
                }
                tail.push(l);
                if tail.len() > 8 {
                    tail.remove(0);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if flag.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("cancelled".into());
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{name} failed: {}", tail.join(" / ")));
    }
    Ok(())
}

/// COLMAP's progress in a log line: "[i/n]" counters, or a registered image.
pub fn colmap_progress(line: &str) -> Option<(usize, usize)> {
    let open = line.rfind('[')?;
    let close = open + line[open..].find(']')?;
    let inner = &line[open + 1..close];
    let first = inner.split(',').next()?;
    let (a, b) = first.split_once('/')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// A photo as the engines read it: HEIC turned into JPEG where an engine
/// (or marker detection) cannot read it (macOS `sips`).
fn decode_gray(path: &FsPath) -> Option<(Vec<f32>, usize, usize)> {
    let bytes = std::fs::read(path).ok()?;
    let bytes = if image_kind(&bytes) == Some("heic") {
        let tmp = path.with_extension("ise-gray.jpg");
        Command::new("sips").args(["-s", "format", "jpeg"]).arg(path).arg("--out").arg(&tmp).output().ok()?;
        let b = std::fs::read(&tmp).ok()?;
        let _ = std::fs::remove_file(&tmp);
        b
    } else {
        bytes
    };
    let p = de_shape::image::decode(&bytes).ok()?;
    Some((p.channel(de_shape::image::Channel::Brightness), p.width, p.height))
}

/// What an engine made, in its own frame and units.
struct Raw {
    mesh: Option<Mesh>,
    cloud: Cloud,
    cameras: Vec<Camera>,
    rejected: Vec<Value>,
    reprojection: Option<f64>,
    /// Photos carrying depth (the size is then the engine's own, metric).
    depth: usize,
    version: String,
    notes: Vec<String>,
    /// Markers already found in each camera's photo (index into `cameras`).
    detections: Option<Vec<(usize, Vec<de_scan::markers::Detection>)>>,
}

fn run(server: &Server, id: &str, set: &PhotoSet, flag: &AtomicBool) -> Result<Value, String> {
    let t0 = std::time::Instant::now();
    let job = jobs().lock().unwrap().iter().find(|j| j.id == id).cloned().ok_or("the job vanished")?;
    let work = server.dir.join(".photogrammetry").join(id);
    let images = work.join("images");
    std::fs::create_dir_all(&images).map_err(|e| e.to_string())?;
    let result = (|| {
        update(server, id, |j| j.advance("copying photos", 0.0, 0.02, 0.0));
        for p in &set.photos {
            let src = de_shape::assets::path(&p.id)?;
            let mut name = p.name.clone();
            if job.engine == "colmap" && p.kind == "heic" {
                // COLMAP reads JPEG and PNG; HEIC through macOS's sips.
                name = format!("{}.jpg", name.rsplit_once('.').map(|x| x.0).unwrap_or(&name));
                let out = Command::new("sips").args(["-s", "format", "jpeg"]).arg(&src).arg("--out").arg(images.join(&name)).output();
                if !out.is_ok_and(|o| o.status.success()) {
                    return Err(format!("{}: COLMAP cannot read HEIC and it could not be converted (convert the photos to JPEG)", p.name));
                }
            } else {
                std::fs::copy(&src, images.join(&name)).map_err(|e| e.to_string())?;
            }
        }
        let raw = match job.engine.as_str() {
            "object-capture" => run_object_capture(server, id, &job, &images, &work, flag)?,
            _ => run_colmap(server, id, &job, &images, &work, flag)?,
        };
        finish(server, id, &job, set, &images, raw, t0)
    })();
    // Photos and engine workspaces are big: only the scans are kept.
    let _ = std::fs::remove_dir_all(&work);
    result.map_err(|e| explain(&e, set.photos.len()))
}

/// Turns an engine's alignment failure (its raw log) into what went wrong and
/// what to shoot instead; other errors pass through.
fn explain(error: &str, photos: usize) -> String {
    let unaligned = ["No images with matches", "Failed to create any sparse model", "PhotogrammetrySession.Error error 6", "imageAlignment"]
        .iter()
        .any(|k| error.contains(k));
    if !unaligned {
        return error.to_string();
    }
    let few = if photos < 20 { format!("only {photos} photos; ") } else { String::new() };
    format!(
        "the photos could not be placed relative to each other ({few}each photo must share about two thirds of what it sees with the next). \
         Product shots from separate sides (front, top, side), cut-out or retouched backgrounds, and compressed copies (chat apps) do not align. \
         Shoot 30–80 original photos in two or three rings round the object, moving 10–15° between shots, on the marker sheet (`scan markers`), in soft light. \
         From a few separate views, model the part from them as reference pictures instead. (Engine: {})",
        error.lines().last().unwrap_or(error).chars().take(160).collect::<String>()
    )
}

fn run_object_capture(server: &Server, id: &str, job: &Job, images: &FsPath, work: &FsPath, flag: &AtomicBool) -> Result<Raw, String> {
    let bin = object_capture_helper(server)?;
    let out = work.join("out");
    let mut cmd = Command::new(bin);
    // Cameras first; then the model of only the box over the marker sheet (when the photos show it), so the
    // model's triangles go to the object rather than the table round it.
    cmd.arg(images).arg(&out).args(["--detail", &job.detail, "--ask-bounds"]);
    if job.mask {
        cmd.arg("--mask");
    }
    let (mut model, mut poses, mut rejected, mut depth, mut error, mut notes) = (None::<String>, Value::Null, Vec::new(), 0usize, None::<String>, Vec::new());
    let mut detections = None;
    update(server, id, |j| j.advance("Object Capture", 0.02, 0.03, 0.0));
    run_program_talk(cmd, flag, true, |l| {
        let Ok(v) = serde_json::from_str::<Value>(l) else { return None };
        if v["awaiting"] == "bounds" {
            update(server, id, |j| j.advance("finding the scale markers", 0.5, 0.55, 0.0));
            let cameras = oc_cameras(&poses, images);
            let found = detect_markers(images, &cameras);
            let sheet = de_scan::markers::Sheet::default();
            let reply = match photo::marker_scaling(&cameras, &found, &sheet) {
                Ok((s, _)) => {
                    notes.push("modelled only the box over the marker sheet".into());
                    sheet_box(&s.transform, &sheet)
                }
                Err(_) => json!({}),
            };
            detections = Some(found);
            return Some(reply.to_string());
        }
        if let Some(p) = v["progress"].as_f64() {
            update(server, id, |j| j.advance(&j.stage.clone(), 0.03, 0.85, p));
        }
        if let Some(s) = v["stage"].as_str() {
            update(server, id, |j| j.advance(&format!("Object Capture: {s}"), 0.03, 0.85, 0.0));
        }
        if let Some(n) = v["invalid"].as_u64() {
            rejected.push(json!({ "sample": n, "reason": v["reason"] }));
        }
        if let Some(n) = v["skipped"].as_u64() {
            rejected.push(json!({ "sample": n, "reason": "skipped" }));
        }
        if let Some(n) = v["depth"].as_u64() {
            depth = n as usize;
        }
        if let Some(m) = v["model"].as_str() {
            model = Some(m.to_string());
        }
        if v["poses"].is_array() {
            poses = v["poses"].clone();
        }
        for k in ["warning", "note"] {
            if let Some(w) = v[k].as_str() {
                notes.push(w.to_string());
            }
        }
        if let Some(e) = v["error"].as_str() {
            error = Some(e.to_string());
        }
        None
    })
    .map_err(|e| error.clone().unwrap_or(e))?;
    if let Some(e) = error {
        return Err(e);
    }
    let model = model.ok_or("Object Capture finished without a model")?;
    update(server, id, |j| j.advance("reading the model", 0.85, 0.88, 0.0));
    let bytes = std::fs::read(&model).map_err(|e| e.to_string())?;
    let data = de_shape::scans::read(&bytes, "obj")?;
    // Object Capture works in metres; Iṣẹ́ Studio in millimetres.
    let mut mesh = data.mesh.ok_or("Object Capture's model has no triangles")?;
    for p in mesh.positions.iter_mut() {
        *p = p.map(|v| v * 1000.0);
    }
    let cloud = Cloud { positions: mesh.positions.clone(), normals: vec![], colors: vec![] };
    let cameras = oc_cameras(&poses, images);
    if cameras.is_empty() {
        notes.push("Object Capture gave no camera poses (macOS 14+ does): marker scaling is not possible; scale with a known distance".into());
    }
    Ok(Raw { mesh: Some(mesh), cloud, cameras, rejected, reprojection: None, depth, version: object_capture_info(server).version.unwrap_or_default(), notes, detections })
}

/// RealityKit's camera poses (column-major camera-to-world, metres; its camera
/// x right, y up, looking −z) as Iṣẹ́ Studio's cameras, millimetres.
fn oc_cameras(poses: &Value, images: &FsPath) -> Vec<Camera> {
    let mut cameras = Vec::new();
    for p in poses.as_array().into_iter().flatten() {
        let m: Vec<f64> = p["matrix"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        let k: Vec<f64> = p["intrinsics"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        if m.len() != 16 || k.len() != 9 {
            continue;
        }
        let name = p["image"].as_str().unwrap_or_default().to_string();
        let col = |c: usize| [m[c * 4], m[c * 4 + 1], m[c * 4 + 2]];
        let (x, y, z, c) = (col(0), col(1), col(2), col(3).map(|v| v * 1000.0));
        let rotation = [x, [-y[0], -y[1], -y[2]], [-z[0], -z[1], -z[2]]];
        let translation = [0, 1, 2].map(|i| -de_scan::linalg::dot(rotation[i], c));
        let (w, h) = image_size(&images.join(&name)).unwrap_or(((k[6] * 2.0) as u32, (k[7] * 2.0) as u32));
        cameras.push(Camera { name, width: w, height: h, fx: k[0], fy: k[4], cx: k[6], cy: k[7], dist: [0.0; 4], rotation, translation });
    }
    cameras
}

/// The marker sheet's markers found in each camera's photo.
fn detect_markers(images: &FsPath, cameras: &[Camera]) -> Vec<(usize, Vec<de_scan::markers::Detection>)> {
    let mut out = Vec::new();
    for (ci, cam) in cameras.iter().enumerate() {
        if let Some((g, w, h)) = decode_gray(&images.join(&cam.name)) {
            out.push((ci, de_scan::markers::detect(&g, w, h)));
        }
    }
    out
}

/// The box an object on the marker sheet stands in, as the engine's frame
/// sees it (axis-aligned there, metres): the sheet's field and half its size
/// again round it (as the cleaning keeps), from just under the sheet to
/// 500 mm over it. `s` takes the engine's frame (mm) to the sheet's.
fn sheet_box(s: &Similarity, sheet: &de_scan::markers::Sheet) -> Value {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for (_, c) in &sheet.markers {
        lo = [lo[0].min(c[0]), lo[1].min(c[1])];
        hi = [hi[0].max(c[0]), hi[1].max(c[1])];
    }
    let h = sheet.marker / 2.0;
    let (lo, hi) = ([lo[0] + h, lo[1] + h], [hi[0] - h, hi[1] - h]);
    let grow = (hi[0] - lo[0]).max(hi[1] - lo[1]) * 0.5;
    let r = &s.rotation;
    let (box_lo, box_hi) = ([lo[0] - grow, lo[1] - grow, -10.0], [hi[0] + grow, hi[1] + grow, 500.0]);
    let (mut a, mut b) = ([f64::MAX; 3], [f64::MIN; 3]);
    for corner in 0..8 {
        let p = [if corner & 1 == 0 { box_lo[0] } else { box_hi[0] }, if corner & 2 == 0 { box_lo[1] } else { box_hi[1] }, if corner & 4 == 0 { box_lo[2] } else { box_hi[2] }];
        // x = Rᵀ (p − t) / s, then metres.
        let d = [p[0] - s.translation[0], p[1] - s.translation[1], p[2] - s.translation[2]];
        let q = [0, 1, 2].map(|i| (r[0][i] * d[0] + r[1][i] * d[1] + r[2][i] * d[2]) / s.scale / 1000.0);
        for k in 0..3 {
            a[k] = a[k].min(q[k]);
            b[k] = b[k].max(q[k]);
        }
    }
    // The same box turned with the sheet (macOS 13+). RealityKit turns an oriented box about its own centre,
    // which (min + max) / 2 places in the session's frame: so the centre there, ± the half-sizes along the sheet.
    let centre = [0, 1, 2].map(|k| (box_lo[k] + box_hi[k]) / 2.0);
    let d = [centre[0] - s.translation[0], centre[1] - s.translation[1], centre[2] - s.translation[2]];
    let c = [0, 1, 2].map(|i| (r[0][i] * d[0] + r[1][i] * d[1] + r[2][i] * d[2]) / s.scale / 1000.0);
    let half = [0, 1, 2].map(|k| (box_hi[k] - box_lo[k]) / 2.0 / s.scale / 1000.0);
    let (o_lo, o_hi) = ([0, 1, 2].map(|k| c[k] - half[k]), [0, 1, 2].map(|k| c[k] + half[k]));
    let rt = [[r[0][0], r[1][0], r[2][0]], [r[0][1], r[1][1], r[2][1]], [r[0][2], r[1][2], r[2][2]]];
    json!({ "min": a, "max": b, "oriented": { "rotation": rotation_quaternion(&rt), "min": o_lo, "max": o_hi } })
}

/// A rotation matrix (rows) as a unit quaternion [w, x, y, z].
fn rotation_quaternion(m: &[[f64; 3]; 3]) -> [f64; 4] {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [0.25 * s, (m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [(m[2][1] - m[1][2]) / s, 0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [(m[0][2] - m[2][0]) / s, (m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [(m[1][0] - m[0][1]) / s, (m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s]
    };
    let n = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    q.map(|v| v / n)
}

fn image_size(path: &FsPath) -> Option<(u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    match image_kind(&bytes)? {
        "png" => Some((u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?), u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?))),
        "jpg" => jpeg_size(&bytes),
        _ => None,
    }
}

/// A photo's name as the person gave it (without the size folder COLMAP may see it in).
fn shown(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// A JPEG's pixel size from its frame header (no decoding), as stored
/// (before any EXIF turn, which COLMAP and Iṣẹ́ Studio both ignore).
pub fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        // Start of frame: baseline, progressive and the rest (not DHT C4, JPG C8, DAC CC).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// Photos of more than one size (two phones, portrait and landscape shots)
/// moved into a folder per size, so COLMAP gives each size its own lens
/// instead of dropping the odd ones ("single camera, but images have
/// different dimensions"). Returns whether it did.
fn group_by_size(images: &FsPath) -> Result<bool, String> {
    let mut sizes: Vec<(String, (u32, u32))> = Vec::new();
    for e in std::fs::read_dir(images).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(s) = image_size(&e.path()) {
            sizes.push((name, s));
        }
    }
    let first = sizes.first().map(|s| s.1);
    if sizes.iter().all(|s| Some(s.1) == first) {
        return Ok(false);
    }
    for (name, (w, h)) in &sizes {
        let dir = images.join(format!("{w}x{h}"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::rename(images.join(name), dir.join(name)).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// The photos in the images folder (and its size folders), as COLMAP names them.
fn photo_names(images: &FsPath) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(images).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() {
            for f in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
                out.push(format!("{name}/{}", f.file_name().to_string_lossy()));
            }
        } else {
            out.push(name);
        }
    }
    out.sort();
    out
}

fn run_colmap(server: &Server, id: &str, job: &Job, images: &FsPath, work: &FsPath, flag: &AtomicBool) -> Result<Raw, String> {
    let info = colmap_info();
    let bin = find_program("colmap", "ISE_COLMAP").ok_or(info.detail.clone())?;
    let v4 = info.version.as_deref().is_some_and(|v| v.trim_start_matches("COLMAP ").split('.').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(3) >= 4);
    let (fe, fm) = if v4 { ("FeatureExtraction", "FeatureMatching") } else { ("SiftExtraction", "SiftMatching") };
    let gpu = if info.dense == Some(true) { "1" } else { "0" };
    let per_size = group_by_size(images)?;
    let threads = (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) / 2).clamp(2, 8).to_string();
    let db = work.join("database.db");
    let sparse = work.join("sparse");
    std::fs::create_dir_all(&sparse).map_err(|e| e.to_string())?;
    let n = job.images;
    let max_size = match job.detail.as_str() {
        "preview" => "1000",
        "reduced" => "1600",
        "medium" => "2400",
        _ => "3200",
    };
    let step = |stage: &'static str, lo: f64, hi: f64, args: Vec<String>| -> Result<(), String> {
        update(server, id, |j| j.advance(stage, lo, hi, 0.0));
        let mut cmd = Command::new(&bin);
        cmd.args(&args);
        let mut registered = 0usize;
        run_program(cmd, flag, |l| {
            if l.contains("Registering image") {
                registered += 1;
                update(server, id, |j| j.advance(stage, lo, hi, registered as f64 / n as f64));
            } else if let Some((a, b)) = colmap_progress(l).filter(|x| x.1 > 0) {
                update(server, id, |j| j.advance(stage, lo, hi, a as f64 / b as f64));
            }
        })
    };
    let s = |v: &str| v.to_string();
    let p = |v: &FsPath| v.to_string_lossy().to_string();
    // Without CUDA, Iṣẹ́ Studio's own dense matching follows the camera solve.
    let cpu_dense = info.dense != Some(true);
    let (matching, solving, solved) = if cpu_dense { (0.12, 0.3, 0.5) } else { (0.2, 0.45, 0.8) };
    step(
        "COLMAP: finding features",
        0.02,
        matching,
        // One lens for all (or per size of photo); OPENCV's two radial and two tangential terms, which wide phone lenses need
        // (on ETH3D's wide-angle photos the cameras came out 4–7× closer to the truth than with one radial term).
        vec![s("feature_extractor"), s("--database_path"), p(&db), s("--image_path"), p(images), s(if per_size { "--ImageReader.single_camera_per_folder" } else { "--ImageReader.single_camera" }), s("1"), s("--ImageReader.camera_model"), s("OPENCV"), format!("--{fe}.use_gpu"), s(gpu), format!("--{fe}.num_threads"), threads.clone(), format!("--{fe}.max_image_size"), s(max_size)],
    )?;
    step("COLMAP: matching photos", matching, solving, vec![s("exhaustive_matcher"), s("--database_path"), p(&db), format!("--{fm}.use_gpu"), s(gpu), format!("--{fm}.num_threads"), threads.clone()])?;
    step("COLMAP: solving cameras", solving, solved, vec![s("mapper"), s("--database_path"), p(&db), s("--image_path"), p(images), s("--output_path"), p(&sparse), s("--Mapper.num_threads"), threads.clone()])?;
    // The largest model it built.
    let mut best: Option<photo::ColmapModel> = None;
    let mut best_dir = sparse.join("0");
    for k in 0..10 {
        let m = sparse.join(k.to_string());
        if !m.is_dir() {
            continue;
        }
        let txt = work.join(format!("txt{k}"));
        std::fs::create_dir_all(&txt).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&bin);
        cmd.args(["model_converter", "--input_path"]).arg(&m).arg("--output_path").arg(&txt).args(["--output_type", "TXT"]);
        run_program(cmd, flag, |_| {})?;
        let read = |f: &str| std::fs::read_to_string(txt.join(f)).unwrap_or_default();
        let model = photo::read_colmap(&read("cameras.txt"), &read("images.txt"), &read("points3D.txt"))?;
        if best.as_ref().is_none_or(|b| model.cameras.len() > b.cameras.len()) {
            best = Some(model);
            best_dir = m.clone();
        }
    }
    let model = best.ok_or("COLMAP could not join the photos into a model: too little overlap or texture (see the capture guide)")?;
    let mean_error = model.errors.iter().sum::<f64>() / model.errors.len().max(1) as f64;
    let used: std::collections::HashSet<&str> = model.cameras.iter().map(|c| c.name.as_str()).collect();
    let mut rejected = Vec::new();
    for name in photo_names(images) {
        if !used.contains(name.as_str()) {
            rejected.push(json!({ "image": shown(&name), "reason": "not registered (too few matches with the others)" }));
        }
    }
    let mut notes = Vec::new();
    if per_size {
        notes.push("photos of different sizes: each size solved with its own lens".into());
    }
    let mesh: Option<Mesh> = None;
    let mut cloud = Cloud::default();
    if info.dense == Some(true) {
        update(server, id, |j| j.advance("COLMAP: dense stereo", 0.8, 0.85, 0.0));
        let dense = work.join("dense");
        let best_dir = (0..10).map(|k| sparse.join(k.to_string())).filter(|d| d.is_dir()).max_by_key(|d| std::fs::read_dir(d).map(|r| r.count()).unwrap_or(0)).unwrap_or(sparse.join("0"));
        let mut ok = true;
        for args in [
            vec![s("image_undistorter"), s("--image_path"), p(images), s("--input_path"), p(&best_dir), s("--output_path"), p(&dense)],
            vec![s("patch_match_stereo"), s("--workspace_path"), p(&dense)],
            vec![s("stereo_fusion"), s("--workspace_path"), p(&dense), s("--output_path"), p(&dense.join("fused.ply"))],
        ] {
            let mut cmd = Command::new(&bin);
            cmd.args(&args);
            if let Err(e) = run_program(cmd, flag, |_| {}) {
                if flag.load(Ordering::Relaxed) {
                    return Err(e);
                }
                notes.push(format!("dense stereo failed ({e}); the sparse points are used"));
                ok = false;
                break;
            }
        }
        if ok {
            if let Ok(bytes) = std::fs::read(dense.join("fused.ply")) {
                let (c, _) = de_scan::points::read_ply(&bytes)?;
                cloud = c;
            }
        }
    }
    if cloud.is_empty() && cpu_dense {
        let aspect = model.cameras.first().map(|c| c.width.max(c.height) as f64 / c.width.min(c.height).max(1) as f64).unwrap_or(1.5);
        match cpu_dense_cloud(server, id, job, (&bin, images, &best_dir, work), (model.cameras.len(), aspect), flag) {
            Ok((c, r)) => {
                notes.push(format!("dense matching (ISE Studio, on the CPU): {} points from {} of {} photos in {:.0} s", r.points, r.matched, r.photos, r.seconds));
                if !r.skipped.is_empty() {
                    notes.push(format!("not matched: {}", r.skipped.join("; ")));
                }
                cloud = c;
            }
            Err(e) if flag.load(Ordering::Relaxed) => return Err(e),
            Err(e) => notes.push(format!("dense matching failed ({e}); the sparse points are used")),
        }
    }
    if cloud.is_empty() {
        // The sparse points seen well: three or more photos, under 2 px.
        for i in 0..model.points.len() {
            if model.track[i] >= 3 && model.errors[i] < 2.0 {
                cloud.positions.push(model.points[i]);
                cloud.colors.push(model.colors[i]);
            }
        }
        notes.push(format!("{} sparse points kept (seen in 3+ photos, under 2 px) of {}", cloud.len(), model.points.len()));
    }
    if cloud.len() < 50 {
        return Err(format!("COLMAP solved {} cameras but only {} good points: the surface needs more texture (see the capture guide)", model.cameras.len(), cloud.len()));
    }
    Ok(Raw { mesh, cloud, cameras: model.cameras, rejected, reprojection: Some(mean_error), depth: 0, version: info.version.unwrap_or_default(), notes, detections: None })
}

/// The longer side Iṣẹ́ Studio's dense matching works at: by detail, made smaller
/// for many photos so they all fit in memory together (about 11 bytes a
/// pixel: grey, colour, depth and normal; 100 million pixels, ~1.1 GB).
fn dense_size(detail: &str, photos: usize, aspect: f64) -> usize {
    let wanted = match detail {
        "preview" => 640,
        "reduced" => 900,
        "medium" => 1200,
        _ => 1600,
    };
    let per_photo = 100e6 / photos.max(1) as f64;
    let fits = (per_photo * aspect.max(1.0)).sqrt() as usize;
    wanted.min(fits).max(320)
}

/// Iṣẹ́ Studio's dense matching (`de_scan::mvs`) on COLMAP's solved cameras, for
/// COLMAP builds without CUDA: COLMAP undistorts the photos (on the CPU),
/// Iṣẹ́ Studio matches them; an oriented, coloured cloud.
fn cpu_dense_cloud(server: &Server, id: &str, job: &Job, (bin, images, model_dir, work): (&FsPath, &FsPath, &FsPath, &FsPath), (photos, aspect): (usize, f64), flag: &AtomicBool) -> Result<(Cloud, de_scan::mvs::Report), String> {
    let size = dense_size(&job.detail, photos, aspect);
    let dense = work.join("dense");
    update(server, id, |j| j.advance("dense matching: undistorting the photos", 0.5, 0.52, 0.0));
    let mut cmd = Command::new(bin);
    cmd.arg("image_undistorter").arg("--image_path").arg(images).arg("--input_path").arg(model_dir).arg("--output_path").arg(&dense).args(["--output_type", "COLMAP", "--max_image_size", &size.to_string()]);
    run_program(cmd, flag, |_| {})?;
    let txt = dense.join("txt");
    std::fs::create_dir_all(&txt).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(bin);
    cmd.args(["model_converter", "--input_path"]).arg(dense.join("sparse")).arg("--output_path").arg(&txt).args(["--output_type", "TXT"]);
    run_program(cmd, flag, |_| {})?;
    let read = |f: &str| std::fs::read_to_string(txt.join(f)).unwrap_or_default();
    let model = photo::read_colmap(&read("cameras.txt"), &read("images.txt"), &read("points3D.txt"))?;
    let (mut cameras, mut photos) = (Vec::new(), Vec::new());
    for (k, c) in model.cameras.iter().enumerate() {
        update(server, id, |j| j.advance("dense matching: reading the photos", 0.52, 0.54, k as f64 / model.cameras.len().max(1) as f64));
        let bytes = std::fs::read(dense.join("images").join(&c.name)).map_err(|e| format!("{}: {e}", c.name))?;
        let pic = de_shape::image::decode(&bytes).map_err(|e| format!("{}: {e}", c.name))?;
        let [r, g, b] = [de_shape::image::Channel::Red, de_shape::image::Channel::Green, de_shape::image::Channel::Blue].map(|ch| pic.channel(ch));
        let rgb = (0..pic.width * pic.height).map(|i| [r[i], g[i], b[i]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        photos.push(de_scan::mvs::Photo::new(pic.width, pic.height, rgb, size));
        cameras.push(c.clone());
    }
    let sparse: Vec<V3> = (0..model.points.len()).filter(|&i| model.track[i] >= 3 && model.errors[i] < 2.0).map(|i| model.points[i]).collect();
    de_scan::mvs::densify(&cameras, &photos, &sparse, &de_scan::mvs::Options::default(), &|p| update(server, id, |j| j.advance("dense matching (ISE Studio, on the CPU)", 0.54, 0.88, p)), &|| flag.load(Ordering::Relaxed))
}

fn transform_camera(c: &Camera, s: &Similarity) -> Camera {
    // x' = s·R·x + t: the camera keeps its view, its frame scaled by s.
    let r = &s.rotation;
    let rt = [[r[0][0], r[1][0], r[2][0]], [r[0][1], r[1][1], r[2][1]], [r[0][2], r[1][2], r[2][2]]];
    let rotation = de_scan::linalg::mat_mul(&c.rotation, &rt);
    let rt_t = [0, 1, 2].map(|i| de_scan::linalg::dot(rotation[i], s.translation));
    let translation = [0, 1, 2].map(|i| s.scale * c.translation[i] - rt_t[i]);
    Camera { rotation, translation, ..c.clone() }
}

fn finish(server: &Server, id: &str, job: &Job, set: &PhotoSet, images: &FsPath, mut raw: Raw, t0: std::time::Instant) -> Result<Value, String> {
    // Size and floor: the marker sheet when the photos show it.
    update(server, id, |j| j.advance("finding the scale markers", 0.88, 0.93, 0.0));
    // (Object Capture's run may have found them already, to model only the box over the sheet.)
    let detections = match raw.detections.take() {
        Some(d) => d,
        None => {
            let mut d = Vec::new();
            for (ci, cam) in raw.cameras.iter().enumerate() {
                if let Some((g, w, h)) = decode_gray(&images.join(&cam.name)) {
                    d.push((ci, de_scan::markers::detect(&g, w, h)));
                }
                update(server, id, |j| j.advance("finding the scale markers", 0.88, 0.93, (ci + 1) as f64 / raw.cameras.len().max(1) as f64));
            }
            d
        }
    };
    let photos_with_markers = detections.iter().filter(|(_, f)| !f.is_empty()).count();
    let sheet = de_scan::markers::Sheet::default();
    let size = {
        let (lo, hi) = raw.cloud.bounds();
        de_scan::linalg::dist(lo, hi)
    };
    let mut marker_report = Value::Null;
    let mut floor_found = false;
    let scaling: Scaling = match photo::marker_scaling(&raw.cameras, &detections, &sheet) {
        Ok((s, report)) => {
            marker_report = report;
            s
        }
        Err(why) => {
            // The floor is under the cameras; a plane they stand on both sides of (a wall, ground walked across) is not one.
            let eyes: Vec<V3> = raw.cameras.iter().map(Camera::centre).collect();
            let floor = photo::floor(&raw.cloud.positions, size * 0.004).and_then(|f| photo::face_cameras(f, &eyes));
            floor_found = floor.is_some();
            let levelled = floor.as_ref().map(|f| photo::level(&raw.cloud.positions, f, size * 0.004)).unwrap_or_else(Similarity::identity);
            let on_floor = if floor.is_some() { "; levelled on the floor plane" } else { "" };
            if raw.depth > 0 {
                Scaling { method: "depth".into(), transform: levelled, uncertainty: 0.01, detail: format!("{} photos carried depth: Object Capture's size is metric (about ±1 %){on_floor}", raw.depth) }
            } else {
                // Marker-like patterns turn up in ordinary photos; say so only when they did.
                let note = if photos_with_markers > 0 { format!(" ({photos_with_markers} photo(s) showed marker-like patterns, but {why})") } else { String::new() };
                Scaling {
                    method: "none".into(),
                    transform: levelled,
                    uncertainty: f64::NAN,
                    detail: format!("no scale: no marker sheet was found{note} and the photos carry no depth. Give it one: ./ise scan scale <scan> --from x,y,z --to x,y,z --distance mm (two points whose distance you measured){on_floor}"),
                }
            }
        }
    };
    let t = scaling.transform;
    for p in raw.cloud.positions.iter_mut() {
        *p = t.apply(*p);
    }
    // Normals (from dense matching) turn with the cloud.
    for n in raw.cloud.normals.iter_mut() {
        *n = de_scan::linalg::mat_vec(&t.rotation, *n);
    }
    if let Some(m) = raw.mesh.as_mut() {
        for p in m.positions.iter_mut() {
            *p = t.apply(*p);
        }
    }
    let cameras: Vec<Camera> = raw.cameras.iter().map(|c| transform_camera(c, &t)).collect();
    // Quality.
    update(server, id, |j| j.advance("checking the result", 0.93, 0.95, 0.0));
    let levelled = scaling.method == "markers" || floor_found;
    let above: Vec<V3> = raw.cloud.positions.iter().filter(|p| !levelled || p[2] > 0.5).copied().collect();
    let target = if above.is_empty() { [0.0; 3] } else { de_scan::linalg::scale(above.iter().fold([0.0; 3], |s, p| de_scan::linalg::add(s, *p)), 1.0 / above.len() as f64) };
    let centres: Vec<V3> = cameras.iter().map(Camera::centre).collect();
    let coverage = photo::coverage(&centres, target, [0.0, 0.0, 1.0]);
    // A room, a street, a façade: the photos walk through it rather than round one object.
    let scene = scaling.method != "markers" && photo::is_scene(&cameras);
    let mut advice: Vec<String> = if scene {
        vec!["the photos walk through a scene (a room, a street, a façade) rather than round one object: the scan is kept whole, not cut down to an object; cut out what you want with ./ise scan crop <scan> --box x0,y0,z0,x1,y1,z1".into()]
    } else {
        coverage.gaps.clone()
    };
    let used = cameras.len();
    if used < 20 {
        advice.push(format!("only {used} photos were used: take 30–80, overlapping by two thirds"));
    }
    if let Some(e) = raw.reprojection.filter(|e| *e > 1.5) {
        advice.push(format!("reprojection error {e:.2} px is high: blurred photos, a moving object or changing light"));
    }
    if !raw.rejected.is_empty() {
        advice.push(format!("{} photo(s) could not be used (too little overlap with the others, or blurred)", raw.rejected.len()));
    }
    if scaling.method == "none" {
        advice.push(if scene { "for true size, measure one distance in the scene (a door, a tile, a table) and give it with ./ise scan scale" } else { "print the marker sheet (./ise scan markers) and stand the object on it for true size" }.into());
    }
    let mut provenance = json!({
        "engine": job.engine,
        "version": raw.version,
        "detail": job.detail,
        "mask": job.mask,
        "photoSet": set.name,
        "images": set.photos.len(),
        "used": used,
        "rejected": raw.rejected,
        "reprojectionPx": raw.reprojection.or(marker_report.get("reprojectionPx").and_then(Value::as_f64)),
        "scale": { "method": scaling.method, "factor": t.scale, "uncertainty": if scaling.uncertainty.is_finite() { json!(scaling.uncertainty) } else { Value::Null }, "detail": scaling.detail, "markers": marker_report, "transform": t.matrix() },
        "coverage": coverage,
        "advice": advice,
        "notes": raw.notes,
        "seconds": t0.elapsed().as_secs_f64().round(),
        "cameras": cameras.iter().map(|c| json!({ "image": shown(&c.name), "centre": c.centre(), "rotation": c.rotation, "focal": c.fx, "size": [c.width, c.height] })).collect::<Vec<_>>(),
    });
    // Kept as a scan, then cleaned.
    update(server, id, |j| j.advance("saving the scan", 0.95, 0.97, 0.0));
    let name = job.name.clone();
    let raw_entry = match &raw.mesh {
        Some(m) => {
            let r = de_scan::analyze::analyze(m);
            provenance["holes"] = json!(r.holes);
            crate::scans::add_with(server, &job.project, &crate::scans::mesh_bytes(m, &name), "ply", &name, &job.engine, None, Some(provenance.clone()))?
        }
        None => crate::scans::add_with(server, &job.project, &de_scan::points::write_ply(&raw.cloud), "ply", &name, &job.engine, None, Some(provenance.clone()))?,
    };
    let mut out = json!({ "scan": raw_entry, "quality": { "images": set.photos.len(), "used": used, "rejected": provenance["rejected"], "reprojectionPx": provenance["reprojectionPx"], "coverage": provenance["coverage"], "holes": provenance["holes"], "advice": provenance["advice"] }, "scale": provenance["scale"] });
    if job.clean && scene {
        out["cleanSkipped"] = json!("a scene, not one object: kept whole");
    } else if job.clean {
        update(server, id, |j| j.advance("cleaning: floor, pieces, holes", 0.97, 0.995, 0.0));
        // With the marker sheet the object stands inside its field (the markers' inner edges).
        let marker_field = (scaling.method == "markers").then(|| {
            let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
            for (_, c) in &sheet.markers {
                lo = [lo[0].min(c[0]), lo[1].min(c[1])];
                hi = [hi[0].max(c[0]), hi[1].max(c[1])];
            }
            let h = sheet.marker / 2.0;
            [lo[0] + h, lo[1] + h, hi[0] - h, hi[1] - h]
        });
        // Without it, the object is where the cameras look: round the point nearest their axes.
        let aim_field = if marker_field.is_none() && cameras.len() >= 3 {
            let rays: Vec<(V3, V3)> = cameras.iter().map(|c| (c.centre(), c.forward())).collect();
            photo::triangulate(&rays).map(|(c, _)| {
                let mut d: Vec<f64> = rays.iter().map(|r| de_scan::linalg::dist(r.0, c)).collect();
                d.sort_by(|a, b| a.total_cmp(b));
                let r = d[d.len() / 2] * 0.3;
                [c[0] - r, c[1] - r, c[0] + r, c[1] + r]
            })
        } else {
            None
        };
        let field = marker_field.or(aim_field);
        let object_size = match field {
            Some(f) => (f[2] - f[0]).max(f[3] - f[1]) / 2.0,
            None => size * t.scale.abs(),
        };
        // Unscaled results are cleaned at a nominal 100 mm (the meshing's defaults are in millimetres) and put back.
        let metric = scaling.method != "none";
        let k = if metric || object_size <= 0.0 { 1.0 } else { 100.0 / object_size };
        let nominal = |raw: &Raw| Raw {
            mesh: raw.mesh.as_ref().map(|m| Mesh::new(m.positions.iter().map(|p| p.map(|v| v * k)).collect(), m.triangles.clone())),
            cloud: Cloud { positions: raw.cloud.positions.iter().map(|p| p.map(|v| v * k)).collect(), normals: raw.cloud.normals.clone(), colors: raw.cloud.colors.clone() },
            cameras: vec![],
            rejected: vec![],
            reprojection: None,
            depth: 0,
            version: String::new(),
            notes: vec![],
            detections: None,
        };
        // The markers' field is small (190 × 110 mm): the crop round it reaches half its size further, so a longer object is not cut at its edge.
        let grow = if marker_field.is_some() { 0.5 } else { 0.0 };
        let cleaned = if k == 1.0 { clean(&raw, levelled, field, grow, metric, object_size) } else { clean(&nominal(&raw), levelled, field.map(|f| f.map(|v| v * k)), grow, metric, object_size * k) };
        match cleaned.map(|(mut m, r)| {
            for p in m.positions.iter_mut() {
                *p = p.map(|v| v / k);
            }
            (m, r)
        }) {
            Ok((m, report)) => {
                let clean_name = format!("{name} (clean)");
                let mut p = json!({ "from": raw_entry.id, "steps": report, "scale": provenance["scale"], "engine": job.engine });
                p["holes"] = json!(de_scan::analyze::analyze(&m).holes);
                let e = crate::scans::add_with(server, &job.project, &crate::scans::mesh_bytes(&m, &clean_name), "ply", &clean_name, "photogrammetry-clean", Some(&raw_entry.id), Some(p))?;
                out["clean"] = json!(e);
            }
            Err(e) => out["cleanError"] = json!(e),
        }
    }
    Ok(out)
}

/// The object alone, closed: the floor cut away (when levelled), only what
/// stands in the field kept (the marker sheet's, or where the cameras look)
/// but whole, even where it reaches past the field; holes filled, lightened
/// to at most 200 000 triangles. Points are thinned of outliers, the things
/// standing in the field picked out, and meshed (Poisson) first.
fn clean(raw: &Raw, levelled: bool, field: Option<[f64; 4]>, grow: f64, metric: bool, size: f64) -> Result<(Mesh, Vec<Value>), String> {
    use de_shape::scans::{cloud_op, mesh_op, reconstruct};
    let mut steps = Vec::new();
    let within = |f: &[f64; 4], p: &V3| p[0] > f[0] && p[1] > f[1] && p[0] < f[2] && p[1] < f[3];
    // The field only seeds the object; the crop round it reaches `grow` × its size further each way.
    let region = field.map(|f| {
        let m = (f[2] - f[0]).max(f[3] - f[1]) * grow;
        [f[0] - m, f[1] - m, f[2] + m, f[3] + m]
    });
    let inside = |p: &V3| field.is_none_or(|f| within(&f, p));
    let in_region = |p: &V3| region.is_none_or(|f| within(&f, p));
    // Cut just above the floor: a few times its own roughness (median, so the object's foot does not count).
    let cut = if levelled {
        let mut near: Vec<f64> = raw.cloud.positions.iter().filter(|p| inside(p) && p[2].abs() < (size * 0.01).max(if metric { 1.0 } else { 0.0 })).map(|p| p[2].abs()).collect();
        near.sort_by(|a, b| a.total_cmp(b));
        let median = near.get(near.len() / 2).copied().unwrap_or(0.1);
        (4.0 * median).clamp(if metric { 0.3 } else { size * 0.003 }, size * 0.02)
    } else {
        0.0
    };
    let mesh = match &raw.mesh {
        Some(m) => {
            let mut m = m.clone();
            if region.is_some() {
                m.triangles.retain(|t| t.iter().all(|&i| in_region(&m.positions[i as usize])));
                m.compact();
                steps.push(json!({ "operation": if metric { "keep round the marker sheet" } else { "keep round where the cameras look" }, "triangles": m.triangles.len() }));
            }
            m
        }
        None => {
            let keep: Vec<u32> = (0..raw.cloud.len() as u32).filter(|&i| in_region(&raw.cloud.positions[i as usize]) && (!levelled || raw.cloud.positions[i as usize][2] > cut)).collect();
            let c = raw.cloud.select(&keep);
            steps.push(json!({ "operation": "cut the floor and background", "points": c.len() }));
            if c.len() < 30 {
                return Err(format!("only {} points stand above the floor: too few to mesh", c.len()));
            }
            let (mut c, r) = cloud_op(&c, "outliers", &json!({ "neighbours": 12, "sigma": 2.0 }))?;
            steps.push(r);
            // Separate things (the object, things beside it): those standing in the field.
            let groups = de_scan::points::clusters(&c, c.spacing() * 4.0);
            let stands = |g: &Vec<u32>| g.iter().any(|&i| inside(&c.positions[i as usize]));
            let biggest = groups.iter().filter(|g| stands(g)).map(Vec::len).max().unwrap_or(0);
            let chosen: Vec<&Vec<u32>> = groups.iter().filter(|g| stands(g) && g.len() * 50 >= biggest).collect();
            if !chosen.is_empty() && chosen.len() < groups.len() {
                let keep: Vec<u32> = chosen.iter().flat_map(|g| g.iter().copied()).collect();
                steps.push(json!({ "operation": "keep what stands in the field", "clusters": [chosen.len(), groups.len()], "points": keep.len() }));
                c = c.select(&keep);
            }
            // Dense points (with their normals) carry finer detail than sparse ones.
            let depth = if c.len() > 100_000 {
                8
            } else if c.len() > 3000 {
                7
            } else {
                6
            };
            let (m, r) = reconstruct(&c, &json!({ "depth": depth, "trim": 6 }))?;
            steps.push(r);
            m
        }
    };
    let mut m = mesh;
    if levelled {
        let (t, r) = mesh_op(&m, "trim", &json!({ "origin": [0.0, 0.0, cut], "normal": [0.0, 0.0, -1.0] }))?;
        if !t.triangles.is_empty() {
            m = t;
            steps.push(r);
        }
    }
    let (t, r) = standing(m, |p| inside(p) && (!levelled || p[2] > cut), levelled);
    m = t;
    steps.push(r);
    let (t, r) = mesh_op(&m, "fillHoles", &json!({ "fair": false }))?;
    m = t;
    steps.push(r);
    if m.triangles.len() > 200_000 {
        let (t, r) = mesh_op(&m, "decimate", &json!({ "triangles": 200_000 }))?;
        m = t;
        steps.push(r);
    }
    steps.push(json!({ "operation": "floor cut", "at": cut }));
    Ok((m, steps))
}

/// The pieces of a mesh standing in the field (`seed` true at one of their
/// vertices): the biggest, and any other at least 2 % its size that (when
/// `levelled`) reaches a quarter of its height — the object, whole, and its
/// loose parts, without the things round it or scraps of the floor. The
/// largest piece when none stands there.
fn standing(m: Mesh, seed: impl Fn(&V3) -> bool, levelled: bool) -> (Mesh, Value) {
    let pieces = m.split_components();
    let total = pieces.len();
    let stands: Vec<bool> = pieces.iter().map(|p| p.positions.iter().any(&seed)).collect();
    let top = |p: &Mesh| p.positions.iter().map(|v| v[2]).fold(f64::MIN, f64::max);
    // Pieces come biggest first: the first standing one is the object.
    let main = stands.iter().position(|s| *s);
    let (biggest, main_top) = main.map(|i| (pieces[i].triangles.len(), top(&pieces[i]))).unwrap_or((0, 0.0));
    let mut out = Mesh::default();
    let mut kept = 0;
    for (i, (p, s)) in pieces.iter().zip(&stands).enumerate() {
        if *s && (Some(i) == main || (p.triangles.len() * 50 >= biggest && (!levelled || top(p) >= 0.25 * main_top))) {
            out.merge(p);
            kept += 1;
        }
    }
    if out.triangles.is_empty() {
        let largest = pieces.into_iter().next().unwrap_or_default();
        let n = largest.triangles.len();
        return (largest, json!({ "operation": "largest", "pieces": [1.min(total), total], "triangles": n }));
    }
    let n = out.triangles.len();
    (out, json!({ "operation": "keep what stands in the field", "pieces": [kept, total], "triangles": n }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_failures_say_what_to_shoot() {
        let e = explain("colmap failed: W incremental_pipeline.cc:390] No images with matches / E sfm.cc:310] Failed to create any sparse model", 8);
        assert!(e.contains("only 8 photos") && e.contains("two or three rings") && e.contains("reference pictures"), "{e}");
        let e = explain("Object Capture could not build the model: The operation couldn’t be completed. (CoreOC.PhotogrammetrySession.Error error 6.)", 40);
        assert!(e.contains("could not be placed") && !e.contains("only 40"), "{e}");
        assert_eq!(explain("no photos in x.zip", 0), "no photos in x.zip");
    }

    #[test]
    fn object_capture_models_the_box_over_the_sheet() {
        // The engine's frame: metres-ish units turned and shifted from the sheet's (mm).
        let s = Similarity { scale: 1000.0 / 0.8, rotation: photo::quat_rotation([0.8, 0.3, -0.2, 0.4]), translation: [30.0, -20.0, 5.0] };
        let b = sheet_box(&s, &de_scan::markers::Sheet::default());
        let (lo, hi): (Vec<f64>, Vec<f64>) = (b["min"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect(), b["max"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect());
        // Where a sheet point is in the engine's frame, metres: s⁻¹(p) / 1000.
        let engine = |p: V3| {
            let r = &s.rotation;
            let d = [p[0] - s.translation[0], p[1] - s.translation[1], p[2] - s.translation[2]];
            [0, 1, 2].map(|i| (r[0][i] * d[0] + r[1][i] * d[1] + r[2][i] * d[2]) / s.scale / 1000.0)
        };
        let inside = |q: V3| (0..3).all(|k| q[k] >= lo[k] && q[k] <= hi[k]);
        // The sheet's centre, an object's top, the end of a 360 mm long one: in. Far off the sheet, or 1 m up: out.
        for p in [[0.0, 0.0, 0.0], [0.0, 0.0, 300.0], [180.0, 0.0, 20.0]] {
            assert!(inside(engine(p)), "{p:?}");
        }
        for p in [[900.0, 0.0, 0.0], [0.0, 0.0, 1500.0]] {
            assert!(!inside(engine(p)), "{p:?}");
        }
        // The turned box: centred where the sheet's box is, as big as it, its rotation the sheet's.
        let o = &b["oriented"];
        let (olo, ohi): (Vec<f64>, Vec<f64>) = (o["min"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect(), o["max"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect());
        let centre = engine([0.0, 0.0, 245.0]);
        for k in 0..3 {
            assert!(((olo[k] + ohi[k]) / 2.0 - centre[k]).abs() < 1e-9);
        }
        assert!(((ohi[0] - olo[0]) * s.scale * 1000.0 - 380.0).abs() < 1e-6 && ((ohi[2] - olo[2]) * s.scale * 1000.0 - 510.0).abs() < 1e-6);
        let q: Vec<f64> = o["rotation"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        let m = photo::quat_rotation([q[0], q[1], q[2], q[3]]);
        for i in 0..3 {
            for j in 0..3 {
                assert!((m[i][j] - s.rotation[j][i]).abs() < 1e-9, "{m:?}");
            }
        }
    }

    #[test]
    fn dense_matching_shrinks_photos_to_fit_many() {
        // 48 photos at 3:2 fit at the detail's size; 300 do not, and are matched smaller.
        assert_eq!(dense_size("medium", 48, 1.5), 1200);
        assert_eq!(dense_size("full", 48, 1.5), 1600);
        let many = dense_size("full", 300, 1.5);
        assert!(many < 800 && (many * many) as f64 / 1.5 * 300.0 <= 100e6, "{many}");
        assert_eq!(dense_size("preview", 100_000, 1.5), 320);
    }

    #[test]
    fn a_job_moves_forward_and_ends() {
        let mut j = Job {
            id: "j".into(),
            project: "p".into(),
            set: "s".into(),
            name: "n".into(),
            engine: "colmap".into(),
            detail: "medium".into(),
            mask: false,
            clean: true,
            state: "running".into(),
            stage: String::new(),
            progress: 0.0,
            log: vec![],
            images: 40,
            started: 0,
            finished: None,
            error: None,
            result: None,
        };
        j.advance("matching", 0.2, 0.45, 0.5);
        assert!((j.progress - 0.325).abs() < 1e-12 && j.stage == "matching" && j.log == ["matching"]);
        // A stage's counter restarting never moves the bar back.
        j.advance("matching", 0.2, 0.45, 0.1);
        assert!((j.progress - 0.325).abs() < 1e-12);
        assert!(j.active());
        j.end(Err("killed".into()), true);
        assert_eq!((j.state.as_str(), j.error.as_deref()), ("cancelled", Some("cancelled")));
        assert!(!j.active());
    }

    #[test]
    fn colmap_progress_is_read_from_its_log() {
        assert_eq!(colmap_progress("I20261013 01:49:12 feature_extraction.cc:250] Processed file [12/40]"), Some((12, 40)));
        assert_eq!(colmap_progress("Matching block [3/7, 1/7]"), Some((3, 7)));
        assert_eq!(colmap_progress("Bundle adjustment report"), None);
    }

    #[test]
    fn photos_are_known_by_their_bytes_and_zips_are_read() {
        assert_eq!(image_kind(b"\xff\xd8\xff\xe0rest"), Some("jpg"));
        assert_eq!(image_kind(b"\0\0\0\x18ftypheic\0\0\0\0"), Some("heic"));
        assert_eq!(image_kind(b"GIF89a"), None);
        // A zip with one stored file and one deflated one.
        let files: Vec<(&str, Vec<u8>, bool)> = vec![("a.jpg", b"\xff\xd8\xffjpeg".to_vec(), false), ("dir/b.png", b"\x89PNGpngpngpngpngpng".to_vec(), true)];
        let mut zip = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in &files {
            let body = if *deflate {
                use std::io::Write;
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.clone()
            };
            let offset = zip.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            zip.extend_from_slice(b"PK\x03\x04");
            zip.extend_from_slice(&[20, 0, 0, 0]);
            zip.extend_from_slice(&method.to_le_bytes());
            zip.extend_from_slice(&[0; 8]);
            zip.extend_from_slice(&(body.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&[0, 0]);
            zip.extend_from_slice(name.as_bytes());
            zip.extend_from_slice(&body);
            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd = zip.len() as u32;
        zip.extend_from_slice(&central);
        zip.extend_from_slice(b"PK\x05\x06\0\0\0\0");
        zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(central.len() as u32).to_le_bytes());
        zip.extend_from_slice(&cd.to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        let out = unzip(&zip).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!((out[1].0.as_str(), out[1].1.as_slice()), ("dir/b.png", files[1].1.as_slice()));
    }

    /// A JPEG's first bytes: SOI, an APP0 segment, then a baseline frame header of w × h.
    fn jpeg_header(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        b.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
        b
    }

    #[test]
    fn a_jpegs_size_is_read_from_its_header() {
        assert_eq!(jpeg_size(&jpeg_header(6048, 4032)), Some((6048, 4032)));
        // Progressive (SOF2) too; a DHT segment (C4) before it is skipped.
        let mut p = jpeg_header(1200, 1600);
        p.splice(20..20, [0xFF, 0xC4, 0x00, 0x04, 0x00, 0x00]);
        let at = p.iter().position(|&b| b == 0xC0).unwrap();
        p[at] = 0xC2;
        assert_eq!(jpeg_size(&p), Some((1200, 1600)));
        assert_eq!(jpeg_size(b"\xff\xd8\xff"), None);
    }

    #[test]
    fn photos_of_two_sizes_get_a_lens_each() {
        let dir = std::env::temp_dir().join(format!("ise-sizes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, w, h) in [("a.jpg", 2400, 1600), ("b.jpg", 2400, 1600)] {
            std::fs::write(dir.join(name), jpeg_header(w, h)).unwrap();
        }
        // One size: left as it is.
        assert!(!group_by_size(&dir).unwrap());
        assert_eq!(photo_names(&dir), ["a.jpg", "b.jpg"]);
        // A portrait shot from another phone: a folder per size.
        std::fs::write(dir.join("c.jpg"), jpeg_header(1200, 1600)).unwrap();
        assert!(group_by_size(&dir).unwrap());
        assert_eq!(photo_names(&dir), ["1200x1600/c.jpg", "2400x1600/a.jpg", "2400x1600/b.jpg"]);
        assert_eq!(shown("1200x1600/c.jpg"), "c.jpg");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_object_is_kept_whole_and_what_stands_beside_it_dropped() {
        // A block `len` long and `s` wide and high, centred on (x, y), its bottom at z.
        let cube = |x: f64, y: f64, z: f64, s: f64, len: f64| {
            let mut m = de_scan::mesh::cube(len, s, s);
            for p in m.positions.iter_mut() {
                *p = [p[0] + x - len / 2.0, p[1] + y - s / 2.0, p[2] + z];
            }
            m
        };
        // An object 200 mm long (a stretched ball) standing in a 190 mm field, a loose part over it, and a box beside it.
        let mut m = de_scan::mesh::uv_sphere(1.0, 12, 24);
        for p in m.positions.iter_mut() {
            *p = [p[0] * 100.0, p[1] * 20.0, p[2] * 20.0 + 20.0];
        }
        m.merge(&cube(0.0, 0.0, 60.0, 10.0, 10.0));
        m.merge(&cube(260.0, 0.0, 0.0, 50.0, 50.0));
        // A scrap of the floor left just over the cut, beside the object's foot.
        let mut scrap = de_scan::mesh::cube(60.0, 30.0, 1.0);
        for p in scrap.positions.iter_mut() {
            *p = [p[0] - 80.0, p[1] + 22.0, p[2] + 0.5];
        }
        m.merge(&scrap);
        let field = [-95.0, -55.0, 95.0, 55.0];
        let (kept, r) = standing(m, |p| p[0] > field[0] && p[1] > field[1] && p[0] < field[2] && p[1] < field[3], true);
        let (lo, hi) = kept.bounds();
        assert_eq!(r["pieces"], json!([2, 4]), "{r}");
        assert!(hi[0] - lo[0] > 199.0 && hi[0] < 150.0 && hi[1] < 25.0, "{lo:?} {hi:?}");
        // Nothing standing in the field: the largest piece.
        let (largest, r) = standing(cube(400.0, 0.0, 0.0, 10.0, 10.0), |_| false, true);
        assert_eq!((largest.triangles.len(), r["operation"].as_str()), (12, Some("largest")));
    }

    #[test]
    fn a_camera_moved_with_the_model_sees_the_same_pixels() {
        let c = Camera { name: "a".into(), width: 800, height: 600, fx: 700.0, fy: 700.0, cx: 400.0, cy: 300.0, dist: [0.0; 4], rotation: photo::quat_rotation([0.9, 0.1, 0.3, -0.2]), translation: [1.0, -2.0, 30.0] };
        let s = Similarity { scale: 25.0, rotation: photo::quat_rotation([0.7, -0.2, 0.5, 0.1]), translation: [10.0, 4.0, -3.0] };
        let p = [0.5, -0.3, 1.2];
        let moved = transform_camera(&c, &s);
        let (a, b) = (c.project(p).unwrap(), moved.project(s.apply(p)).unwrap());
        assert!((a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "{a:?} {b:?}");
    }
}
