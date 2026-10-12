//! Forma AI: a Claude Code session on this computer, one per project, that
//! the Forma AI panel talks to.
//!
//! The session stays running between messages (streamed JSON in and out), so
//! a reply starts as soon as a message arrives instead of after a cold start;
//! its id is saved, so after a restart the same conversation resumes. It is
//! walled into its own folder in the project directory: it can run `./forma`
//! (which acts on this project through the engine API, as user "forma-ai"),
//! read and write files in its folder, and read the shape vocabulary and the
//! examples. It loads none of this computer's Claude settings, hooks or MCP
//! servers. Every message carries live context (selection, revision, what
//! changed since its last turn), so it always knows what is going on. What it
//! says streams to every open view of the project as it is written.

use crate::App;
use de_core::{Project, StableId, json};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::Mutex,
};

/// How sessions are started, shared by every project.
pub struct AiConfig {
    /// The repository: tools/forma.mjs, the briefing, the prelude, examples.
    pub repo: PathBuf,
    pub port: u16,
    /// The Claude Code CLI, if found.
    pub claude: Option<PathBuf>,
    pub model: Option<String>,
}

impl AiConfig {
    /// Finds the Claude Code CLI: `FORMA_CLAUDE`, else `claude` on the PATH.
    pub fn find_claude() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("FORMA_CLAUDE") {
            return Some(PathBuf::from(p));
        }
        let path = std::env::var_os("PATH")?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        std::env::split_paths(&path)
            .chain(home.map(|h| h.join(".local/bin")))
            .map(|d| d.join("claude"))
            .find(|p| p.is_file())
    }
}

/// One line of the conversation, as every view shows it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatRecord {
    /// "user", "assistant", "tool" or "error".
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub at: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

struct Running {
    stdin: ChildStdin,
    child: Child,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    /// The running session was resumed from a saved one (it may turn out to be gone).
    resumed: bool,
    /// The message in flight, to send again if a resume fails.
    in_flight: Option<String>,
    busy: bool,
    queue: VecDeque<String>,
    /// Revision the session last saw, to tell it what changed since.
    seen_revision: u64,
}

/// The Forma AI of one project.
pub struct Ai {
    config: Arc<AiConfig>,
    project: String,
    folder: PathBuf,
    state: Mutex<State>,
    chat: std::sync::Mutex<Vec<ChatRecord>>,
    /// Who spoke to the AI last: its view commands go to that person's views.
    last_user: std::sync::Mutex<Option<String>>,
}

/// What the panel sends with a message: what the person is looking at.
#[derive(Debug, Default, Deserialize)]
pub struct ViewContext {
    pub selected: Option<StableId>,
    pub picked_part: Option<String>,
    /// What the view is showing: one item isolated, items hidden, the paint.
    #[serde(default)]
    pub isolated: Option<StableId>,
    #[serde(default)]
    pub hidden: Vec<StableId>,
    #[serde(default)]
    pub color_by: Option<String>,
}

impl Ai {
    pub fn new(config: Arc<AiConfig>, project: &str, projects_dir: &Path) -> Ai {
        // Absolute, so the session's permission rules (written as absolute paths) match it
        // even when the engine was started with a relative projects folder.
        let folder = projects_dir.canonicalize().unwrap_or_else(|_| projects_dir.to_path_buf()).join(format!("{project}.ai"));
        let chat = std::fs::read_to_string(folder.join("chat.jsonl"))
            .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
            .unwrap_or_default();
        Ai { config, project: project.to_string(), folder, state: Mutex::new(State::default()), chat: std::sync::Mutex::new(chat), last_user: std::sync::Mutex::new(None) }
    }

    pub fn available(&self) -> bool {
        self.config.claude.is_some()
    }

    /// The conversation so far (most recent last), for a view that just opened.
    pub fn history(&self) -> Vec<ChatRecord> {
        let chat = self.chat.lock().unwrap();
        chat[chat.len().saturating_sub(200)..].to_vec()
    }

    /// Whether a turn is running (for a view that just opened).
    pub fn busy_now(&self) -> bool {
        self.state.try_lock().map(|s| s.busy).unwrap_or(true)
    }

    fn record(&self, app: &App, role: &str, text: &str, user: Option<&str>) {
        let r = ChatRecord { role: role.into(), text: text.into(), user: user.map(str::to_owned), at: now_ms() };
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.folder.join("chat.jsonl")) {
            let _ = writeln!(f, "{}", serde_json::to_string(&r).unwrap());
        }
        let _ = app.events.send(json!({ "type": "ai", "event": "message", "record": r }).to_string().into());
        self.chat.lock().unwrap().push(r);
    }

    fn broadcast(app: &App, event: serde_json::Value) {
        let mut e = event;
        e["type"] = json!("ai");
        let _ = app.events.send(e.to_string().into());
    }

    /// Prepares the session's folder: its `./forma` tool, a place for programs, its memory.
    fn prepare(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.folder.join("programs"))?;
        std::fs::create_dir_all(self.folder.join("memory"))?;
        let tool = self.folder.join("forma");
        let script = format!(
            "#!/bin/sh\n# Acts on project \"{p}\" through the engine API, as Forma AI.\nexec node \"{cli}\" --project \"{p}\" --url \"ws://127.0.0.1:{port}/ws?user=forma-ai\" \"$@\"\n",
            p = self.project,
            cli = self.config.repo.join("tools/forma.mjs").display(),
            port = self.config.port,
        );
        std::fs::write(&tool, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    fn session_file(&self) -> PathBuf {
        self.folder.join("session.json")
    }

    /// The saved session to resume, if it was started with the same briefing.
    /// Claude Code keeps a session's original system prompt when it resumes,
    /// so a changed briefing must start a new session (the conversation is
    /// carried over from the panel's history instead).
    fn resumable(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.session_file()).ok()?;
        let v = serde_json::from_str::<serde_json::Value>(&text).ok()?;
        (v["briefing"].as_str() == Some(&fingerprint(&self.briefing()))).then(|| v["session_id"].as_str().map(str::to_owned)).flatten()
    }

    fn forget_session(&self) {
        let _ = std::fs::remove_file(self.session_file());
    }

    /// The recent conversation, for a session that starts fresh mid-conversation.
    fn continuation(&self) -> String {
        let chat = self.chat.lock().unwrap();
        let lines: Vec<String> = chat
            .iter()
            .filter(|r| r.role == "user" || r.role == "assistant")
            .rev()
            .take(20)
            .map(|r| format!("{}: {}", if r.role == "user" { "Person" } else { "You" }, r.text))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if lines.is_empty() {
            return String::new();
        }
        format!("<earlier-conversation>\nThis conversation continues from the panel; your earlier session ended. The latest exchanges:\n{}\n</earlier-conversation>\n\n", lines.join("\n"))
    }

    fn briefing(&self) -> String {
        let repo = &self.config.repo;
        let brief = std::fs::read_to_string(repo.join("tools/forma-ai.md")).unwrap_or_default();
        format!(
            "{brief}\n\n## This installation\n\n- Project: \"{p}\". Your folder: {folder}.\n- Shape vocabulary (read it before writing programs): {prelude}; sheet metal: {sheet}\n- Worked examples, one folder per design (programs and the steps.json that built them): {examples}\n- Stock library (bought parts as programs, one manifest + program per family; read them to model a new bought part the same way): {library}\n",
            p = self.project,
            folder = self.folder.display(),
            prelude = repo.join("crates/plugins/shape/src/prelude.js").display(),
            sheet = repo.join("crates/plugins/shape/src/sheet.js").display(),
            examples = repo.join("examples").display(),
            library = repo.join("library/stock").display(),
        )
    }

    fn spawn(self: &Arc<Self>, app: Arc<App>, resume: Option<&str>) -> Result<Running, String> {
        let claude = self.config.claude.clone().ok_or("Claude Code is not installed on this computer (set FORMA_CLAUDE to its path)")?;
        self.prepare().map_err(|e| format!("could not prepare the AI folder: {e}"))?;
        let abs = |p: &Path| format!("/{}", p.display()); // a rule path starting with // is absolute
        let folder = abs(&self.folder);
        let repo = &self.config.repo;
        let mut cmd = Command::new(claude);
        cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
            .args(["--tools", "Bash,Read,Write,Edit"])
            .arg("--allowedTools")
            .args([
                "Bash(./forma:*)".to_string(),
                format!("Read({folder}/**)"),
                format!("Write({folder}/**)"),
                format!("Edit({folder}/**)"),
                format!("Read({}/**)", abs(&repo.join("examples"))),
                format!("Read({}/**)", abs(&repo.join("library/stock"))),
                format!("Read({})", abs(&repo.join("crates/plugins/shape/src/prelude.js"))),
                format!("Read({})", abs(&repo.join("crates/plugins/shape/src/sheet.js"))),
            ])
            .args(["--setting-sources", "project", "--strict-mcp-config", "--mcp-config", r#"{"mcpServers":{}}"#])
            .arg("--settings")
            .arg(json!({ "autoMemoryEnabled": true, "autoMemoryDirectory": self.folder.join("memory") }).to_string())
            .arg("--append-system-prompt")
            .arg(self.briefing());
        if let Some(model) = &self.config.model {
            cmd.args(["--model", model]);
        }
        if let Some(session) = resume {
            cmd.args(["--resume", session]);
        }
        cmd.current_dir(&self.folder)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("could not start Claude Code: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;

        let me = Arc::clone(self);
        let app2 = Arc::clone(&app);
        tokio::spawn(async move {
            let mut err = String::new();
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                err.push_str(&l);
                err.push('\n');
            }
            if !err.trim().is_empty() {
                eprintln!("forma ai ({}): {}", me.project, err.trim());
            }
            drop(app2);
        });
        let me = Arc::clone(self);
        tokio::spawn(async move { me.read(app, stdout).await });
        Ok(Running { stdin, child })
    }

    /// Follows the session's output: streams text, notes tool use, ends turns.
    async fn read(self: Arc<Self>, app: Arc<App>, stdout: tokio::process::ChildStdout) {
        let mut lines = BufReader::new(stdout).lines();
        let mut transcript = std::fs::OpenOptions::new().create(true).append(true).open(self.folder.join("transcript.jsonl")).ok();
        let mut resume_failed = false;
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(f) = transcript.as_mut() {
                let _ = writeln!(f, "{line}");
            }
            let Ok(ev) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if let Some(id) = ev["session_id"].as_str()
                && ev["type"] == "system"
                && ev["subtype"] == "init"
            {
                let _ = std::fs::write(self.session_file(), json!({ "session_id": id, "briefing": fingerprint(&self.briefing()) }).to_string());
            }
            match ev["type"].as_str() {
                Some("stream_event") => {
                    let e = &ev["event"];
                    if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                        Self::broadcast(&app, json!({ "event": "delta", "text": e["delta"]["text"] }));
                    } else if e["type"] == "content_block_delta" && e["delta"]["type"] == "thinking_delta" {
                        Self::broadcast(&app, json!({ "event": "thinking" }));
                    }
                }
                Some("assistant") => {
                    for block in ev["message"]["content"].as_array().into_iter().flatten() {
                        match block["type"].as_str() {
                            Some("text") => {
                                if let Some(t) = block["text"].as_str().filter(|t| !t.trim().is_empty()) {
                                    self.record(&app, "assistant", t.trim(), None);
                                }
                            }
                            Some("tool_use") => {
                                let input = &block["input"];
                                let what = input["command"].as_str().map(str::to_owned).or_else(|| input["file_path"].as_str().map(|p| format!("{} {p}", block["name"].as_str().unwrap_or("")))).unwrap_or_else(|| block["name"].as_str().unwrap_or("tool").to_owned());
                                self.record(&app, "tool", &what, None);
                            }
                            _ => {}
                        }
                    }
                }
                Some("result") => {
                    // A saved session that no longer exists fails before its first turn: start fresh below.
                    if ev["is_error"] == true && ev["num_turns"].as_u64() == Some(0) && self.state.lock().await.resumed {
                        resume_failed = true;
                        continue;
                    }
                    if ev["is_error"] == true {
                        let why = ev["result"].as_str().or(ev["subtype"].as_str()).unwrap_or("the turn failed");
                        self.record(&app, "error", why, None);
                    }
                    Self::broadcast(&app, json!({ "event": "done", "cost_usd": ev["total_cost_usd"] }));
                    self.next(&app).await;
                }
                _ => {}
            }
        }
        // The process ended. If its saved session was gone, start a fresh one now
        // and send the message again, with the conversation so far.
        let mut st = self.state.lock().await;
        st.running = None;
        if resume_failed && let Some(line) = st.in_flight.take() {
            eprintln!("forma ai ({}): saved session not found; starting a fresh one", self.project);
            self.forget_session();
            st.resumed = false;
            drop(st);
            match self.spawn(Arc::clone(&app), None) {
                Ok(mut running) => {
                    let line = with_prefix(&line, &self.continuation());
                    let sent = running.stdin.write_all(line.as_bytes()).await.is_ok() && running.stdin.flush().await.is_ok();
                    let mut st = self.state.lock().await;
                    st.running = Some(running);
                    st.in_flight = Some(line);
                    st.busy = sent;
                    if sent {
                        return;
                    }
                }
                Err(e) => self.record(&app, "error", &e, None),
            }
            st = self.state.lock().await;
        }
        if st.busy {
            st.busy = false;
            drop(st);
            self.record(&app, "error", "The AI session stopped. Send the message again to continue.", None);
            Self::broadcast(&app, json!({ "event": "done" }));
        }
    }

    /// Ends the current turn and sends the next queued message, if any.
    async fn next(self: &Arc<Self>, app: &Arc<App>) {
        let mut st = self.state.lock().await;
        st.busy = false;
        st.in_flight = None;
        if let Some(line) = st.queue.pop_front()
            && let Some(r) = st.running.as_mut()
        {
            st.busy = r.stdin.write_all(line.as_bytes()).await.is_ok() && r.stdin.flush().await.is_ok();
            st.in_flight = Some(line);
        }
        let busy = st.busy;
        drop(st);
        Self::broadcast(app, json!({ "event": "status", "busy": busy }));
    }

    /// What the session needs to know with this message.
    fn context(&self, project: &Project, view: &ViewContext, seen: u64) -> String {
        let mut c = format!("<forma-context>\nProject: {} · revision {}\n", self.project, project.revision().0);
        match view.selected.and_then(|id| describe(project, id).map(|d| (id, d))) {
            Some((id, d)) => {
                c += &format!("Selected: {d} ({id})\n");
                if let Some(part) = &view.picked_part {
                    c += &format!("Picked part inside it: {part}\n");
                }
            }
            None => c += "Selected: nothing\n",
        }
        if let Some(d) = view.isolated.and_then(|id| describe(project, id).map(|d| format!("{d} ({id})"))) {
            c += &format!("Their view shows only: {d}\n");
        }
        if !view.hidden.is_empty() {
            let names: Vec<String> = view.hidden.iter().filter_map(|id| describe(project, *id)).collect();
            c += &format!("Hidden in their view: {}\n", names.join("; "));
        }
        if view.color_by.as_deref() == Some("status") {
            c += "Their view is painted by sale status.\n";
        }
        let changes: Vec<String> = project
            .journal()
            .iter()
            .filter(|ch| ch.revision.0 > seen && ch.actor.user_id != "forma-ai")
            .map(|ch| format!("  r{} {} by {}", ch.revision.0, ch.operation, ch.actor.user_id))
            .collect();
        if seen > 0 && !changes.is_empty() {
            let skip = changes.len().saturating_sub(15);
            c += &format!("Changed by people since your last turn:\n{}\n", changes[skip..].join("\n"));
        }
        let report = de_intent::check(project);
        c += &format!("Requirements: {}/{} met\n", report.requirements_passing, report.requirements_total);
        c += "</forma-context>\n\n";
        c
    }

    /// A message from the panel.
    pub async fn ask(self: &Arc<Self>, app: Arc<App>, user: &str, text: &str, view: ViewContext) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("empty message".into());
        }
        *self.last_user.lock().unwrap() = Some(user.to_string());
        let mut st = self.state.lock().await;
        // Starting a session: resume the saved one if it is still valid, else
        // start fresh and bring the conversation along.
        let starting = st.running.is_none();
        let resume = if starting { self.resumable() } else { None };
        let carry = if starting && resume.is_none() { self.continuation() } else { String::new() };
        let prompt = {
            let project = app.project.lock().unwrap();
            let p = format!("{carry}{}{text}", self.context(&project, &view, if starting { 0 } else { st.seen_revision }));
            st.seen_revision = project.revision().0;
            p
        };
        self.record(&app, "user", text, Some(user));
        let line = json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": prompt }] } }).to_string() + "\n";
        if starting {
            drop(st);
            let running = self.spawn(Arc::clone(&app), resume.as_deref())?;
            st = self.state.lock().await;
            st.running = Some(running);
            st.resumed = resume.is_some();
        }
        if st.busy {
            st.queue.push_back(line);
            return Ok(());
        }
        let r = st.running.as_mut().expect("started above");
        r.stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
        r.stdin.flush().await.map_err(|e| e.to_string())?;
        st.busy = true;
        st.in_flight = Some(line);
        drop(st);
        Self::broadcast(&app, json!({ "event": "status", "busy": true }));
        Ok(())
    }

    /// Who the AI is talking to now (its view commands go to them).
    pub fn audience(&self) -> Option<String> {
        self.last_user.lock().unwrap().clone()
    }

    /// Stops the current turn. The conversation resumes with the next message.
    pub async fn stop(self: &Arc<Self>, app: Arc<App>) {
        let mut st = self.state.lock().await;
        st.queue.clear();
        if let Some(mut r) = st.running.take() {
            let _ = r.child.start_kill();
        }
        let was = st.busy;
        st.busy = false;
        drop(st);
        if was {
            self.record(&app, "error", "Stopped.", None);
        }
        Self::broadcast(&app, json!({ "event": "status", "busy": false }));
    }
}

/// A stable fingerprint of a briefing (FNV-1a, 64-bit), to tell whether a
/// saved session was started with the current one.
fn fingerprint(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A user message line with text put in front of its prompt.
fn with_prefix(line: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return line.to_string();
    }
    let mut v: serde_json::Value = serde_json::from_str(line.trim()).unwrap_or_default();
    if let Some(t) = v["message"]["content"][0]["text"].as_str() {
        v["message"]["content"][0]["text"] = json!(format!("{prefix}{t}"));
    }
    v.to_string() + "\n"
}

/// A one-line description of an entity, for context.
fn describe(project: &Project, id: StableId) -> Option<String> {
    if let Some(i) = project.get::<de_assembly::Instance>(id) {
        let def = project
            .get::<de_shape::ShapeProgram>(i.of)
            .map(|p| format!("component “{}”", p.name))
            .or_else(|| project.get::<de_assembly::AssemblyDef>(i.of).map(|a| format!("assembly “{}”", a.name)))
            .unwrap_or_default();
        let users = project.entity_ids().into_iter().filter(|e| project.get::<de_assembly::Instance>(*e).is_some_and(|x| x.of == i.of)).count();
        let props: Vec<String> = i.properties.iter().map(|(k, v)| format!("{k}={v}")).collect();
        return Some(format!(
            "instance “{}” of {def} {}{} ({}){}",
            i.name,
            i.of,
            i.config.as_ref().map(|c| format!(" in configuration “{c}”")).unwrap_or_default(),
            if users > 1 { format!("shared by {users} instances") } else { "unique".into() },
            if props.is_empty() { String::new() } else { format!(" [{}]", props.join(", ")) }
        ));
    }
    if let Some(p) = project.get::<de_shape::ShapeProgram>(id) {
        let configs = project
            .get::<de_shape::ShapeConfigurations>(id)
            .filter(|t| !t.rows.is_empty())
            .map(|t| format!(" with configurations {}{}", t.names(), t.active.as_ref().map(|a| format!(" (showing “{a}”)")).unwrap_or_default()))
            .unwrap_or_default();
        return Some(format!("shape “{}”{configs}", p.name));
    }
    if let Some(a) = project.get::<de_assembly::AssemblyDef>(id) {
        return Some(format!("assembly definition “{}”", a.name));
    }
    project.get::<de_solid::BoxFeature>(id).map(|_| "box part".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_folder_is_absolute_even_from_a_relative_projects_folder() {
        let config = Arc::new(AiConfig { repo: PathBuf::from("."), port: 0, claude: None, model: None });
        let ai = Ai::new(config, "demo", Path::new("."));
        assert!(ai.folder.is_absolute(), "{}", ai.folder.display());
        assert!(ai.folder.ends_with("demo.ai"));
    }
}
