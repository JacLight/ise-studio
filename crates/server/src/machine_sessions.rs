//! Explicit, computer-wide machine ownership. No child exists before Connect.
//! The host process is independent of document edits; it does not survive server exit.
use axum::{extract::{Path, State, rejection::JsonRejection}, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, Json};
use serde_json::{json, Value};
use std::{path::{Path as FsPath, PathBuf}, sync::Arc, time::Duration};
use tokio::{io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader}, process::{Child, ChildStdin, ChildStdout, Command}, sync::{Mutex, OwnedMutexGuard}, task::JoinHandle};

const INPUT_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const OPS: [&str; 9] = ["connect", "prepare", "start", "status", "pause", "resume", "abort", "close", "capabilities"];

#[derive(Clone)]
pub(crate) struct SessionBridge {
    script: PathBuf,
    python: PathBuf,
    state: Arc<Mutex<Slot>>,
    machine_owner: Arc<Mutex<()>>,
    timeout: Duration,
}
#[derive(Default)]
struct Slot { process: Option<Host>, sequence: u64 }
struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    partial_line: Vec<u8>,
    stderr: JoinHandle<()>,
    _owner: OwnedMutexGuard<()>,
    physical: bool,
    unknown: bool,
}
impl Drop for Host { fn drop(&mut self) { self.stderr.abort(); } }
impl SessionBridge {
    pub(crate) fn new(repo: &FsPath, machine_owner: Arc<Mutex<()>>) -> Self {
        let sibling = repo.join("../cad_software");
        Self {
            script: std::env::var_os("CAD_MACHINE_HOST_SCRIPT").map(PathBuf::from)
                .unwrap_or_else(|| sibling.join("study/machine-runtime/machine_host.py")),
            python: std::env::var_os("CAD_MACHINE_PYTHON").map(PathBuf::from)
                .unwrap_or_else(|| sibling.join("study/toolchain/meerk40t-env/bin/python")),
            state: Arc::new(Mutex::new(Slot::default())), machine_owner,
            timeout: Duration::from_secs(60),
        }
    }
    fn spawn(&self, owner: OwnedMutexGuard<()>, physical: bool) -> Result<Host, String> {
        if !self.script.is_file() { return Err("Machine host unavailable. Configure CAD_MACHINE_HOST_SCRIPT on this computer.".into()); }
        let mut child = Command::new(&self.python).arg("-B").arg("-u").arg(&self.script)
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()).kill_on_drop(true).spawn()
            .map_err(|_| "Machine host could not start. Check CAD_MACHINE_PYTHON.")?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut stderr = child.stderr.take().unwrap();
        // Drain without collecting machine paths, serials or unbounded diagnostics.
        let stderr = tokio::spawn(async move {
            let mut bytes = [0u8; 4096];
            loop { match stderr.read(&mut bytes).await { Ok(0) | Err(_) => break, Ok(_) => {} } }
        });
        Ok(Host { child, stdin, stdout, partial_line: Vec::new(), stderr, _owner: owner, physical, unknown: false })
    }
    async fn request(&self, operation: &str, body: Value) -> Response {
        if let Err(message) = validate(operation, &body) { return failure(StatusCode::BAD_REQUEST, message, false); }
        let Ok(mut slot) = self.state.try_lock() else {
            return failure(StatusCode::CONFLICT, "Another machine request is in progress; this request was not sent.", false);
        };
        if operation == "connect" {
            if slot.process.is_some() { return failure(StatusCode::CONFLICT, "A machine host already owns this computer. Disconnect it explicitly before connecting again.", false); }
            let Ok(owner) = self.machine_owner.clone().try_lock_owned() else {
                return failure(StatusCode::CONFLICT, "Another controller operation owns this computer.", false);
            };
            match self.spawn(owner, body["backend"] == "physical") {
                Ok(host) => slot.process = Some(host),
                Err(message) => return failure(StatusCode::SERVICE_UNAVAILABLE, message, false),
            }
        }
        if slot.process.is_none() {
            if matches!(operation, "status" | "close") {
                return Json(json!({"ok":true,"result":{"state":"disconnected","connected":false,"worker_active":false,"job_prepared":false,"outcome_unknown":false}})).into_response();
            }
            return failure(StatusCode::CONFLICT, "Connect explicitly first. No machine host was started.", false);
        }
        slot.sequence += 1;
        let id = format!("request-{}", slot.sequence);
        let host = slot.process.as_mut().unwrap();
        if host.unknown && operation != "close" {
            return failure(StatusCode::CONFLICT, "The previous operation's outcome is unknown. No command was sent. Disconnect explicitly; there is no automatic replay or reconnect.", true);
        }
        if operation == "start" && host.physical && body["authorized"] != true {
            return failure(StatusCode::BAD_REQUEST, "Physical Start requires explicit authorized:true.", false);
        }
        let mut request = body;
        request["id"] = json!(id);
        request["op"] = json!(operation);
        let closing = operation == "close";
        let previous_unknown = host.unknown;
        let duration = if closing { self.timeout.min(Duration::from_secs(10)) } else { self.timeout };
        let result = tokio::time::timeout(duration, exchange(host, &request, &id, closing && previous_unknown)).await;
        let mut reply = match result {
            Ok(Ok(reply)) => reply,
            result => {
                host.unknown = true;
                if !closing {
                    return failure(if result.is_err() { StatusCode::GATEWAY_TIMEOUT } else { StatusCode::BAD_GATEWAY },
                        "Machine host timed out, closed its stream, or returned an invalid reply. Outcome unknown; no operation was replayed. Disconnect explicitly.", true);
                }
                json!({"ok":false,"error":"Graceful disconnect could not be confirmed. Host termination was required; machine outcome remains unknown.","outcome_unknown":true})
            }
        };
        if reply["outcome_unknown"] == true || reply["result"]["outcome_unknown"] == true {
            host.unknown = true;
        }
        if closing {
            // Closing stdin asks the standalone host to exit after its explicit close.
            drop(host.stdin.take());
            let exited = matches!(tokio::time::timeout(Duration::from_secs(2), host.child.wait()).await, Ok(Ok(_)));
            let unknown = previous_unknown || host.unknown || reply["ok"] != true || !exited;
            if !exited {
                let _ = host.child.start_kill();
                if !matches!(tokio::time::timeout(Duration::from_secs(1), host.child.wait()).await, Ok(Ok(_))) {
                    host.unknown = true;
                    return failure(StatusCode::BAD_GATEWAY, "Host termination could not be confirmed. Computer ownership is retained; outcome unknown. Retry explicit Disconnect only.", true);
                }
            }
            if unknown {
                reply["outcome_unknown"] = json!(true);
                if reply["result"].is_object() { reply["result"]["outcome_unknown"] = json!(true); }
                reply["warning"] = json!("The host is disconnected, but machine outcome was not confirmed. No job will be replayed on reconnect.");
            }
            slot.process.take(); // Release computer-wide ownership only after shutdown.
            reply["host_disconnected"] = json!(true);
        }
        let status = if reply["ok"] == true { StatusCode::OK } else { StatusCode::CONFLICT };
        (status, Json(reply)).into_response()
    }
}
fn validate(operation: &str, body: &Value) -> Result<(), &'static str> {
    if !OPS.contains(&operation) { return Err("Unsupported machine operation."); }
    let object = body.as_object().ok_or("Machine requests must be JSON objects.")?;
    let allowed: &[&str] = match operation {
        "connect" => &["family", "backend", "configuration", "authorized"],
        "prepare" => &["job"], "start" => &["authorized"], _ => &[],
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) { return Err("Unexpected machine request field."); }
    if let Some(value) = object.get("authorized") { if !value.is_boolean() { return Err("authorized must be a boolean."); } }
    if operation == "connect" {
        if !matches!(body["family"].as_str(), Some("jcz" | "ruida" | "grbl")) { return Err("Choose JCZ, Ruida or GRBL for a session. Delta live SDK is unavailable."); }
        if !matches!(body["backend"].as_str(), Some("simulation" | "physical")) || !body["configuration"].is_object() { return Err("Provide an explicit backend and configuration object."); }
        if body["backend"] == "physical" && body["authorized"] != true { return Err("Physical Connect requires explicit authorized:true."); }
    }
    if operation == "start" && body["authorized"] != true { return Err("Start requires explicit authorized:true."); }
    if operation == "prepare" && !body["job"].is_object() { return Err("Provide a job object."); }
    if serde_json::to_vec(body).map_err(|_| "Invalid JSON.")?.len() > INPUT_LIMIT - 128 { return Err("Machine request exceeds the 1 MiB limit."); }
    Ok(())
}
async fn exchange(host: &mut Host, request: &Value, id: &str, discard_old: bool) -> Result<Value, String> {
    let mut bytes = serde_json::to_vec(request).map_err(|_| "Invalid host request")?;
    bytes.push(b'\n');
    let stdin = host.stdin.as_mut().ok_or("Host input is closed")?;
    stdin.write_all(&bytes).await.map_err(|_| "Host write failed")?;
    stdin.flush().await.map_err(|_| "Host flush failed")?;
    // Only explicit cleanup may consume a late reply to the one timed-out request.
    for _ in 0..if discard_old { 2 } else { 1 } {
        let reply = read_reply(host).await?;
        if reply["id"].as_str() != Some(id) {
            if discard_old { continue; }
            return Err("Host response ID mismatch".into());
        }
        if !reply["ok"].is_boolean() || (reply["ok"] == true && !reply["result"].is_object())
            || (reply["ok"] == false && !reply["error"].is_string()) { return Err("Invalid host response".into()); }
        return Ok(reply);
    }
    Err("No matching close reply".into())
}
async fn read_reply(host: &mut Host) -> Result<Value, String> {
    loop {
        let buffer = host.stdout.fill_buf().await.map_err(|_| "Host read failed")?;
        if buffer.is_empty() { return Err("Host exited before replying".into()); }
        let count = buffer.iter().position(|b| *b == b'\n').map(|n| n + 1).unwrap_or(buffer.len());
        if host.partial_line.len() + count > OUTPUT_LIMIT { return Err("Host response exceeded 4 MiB".into()); }
        let ended = buffer[count - 1] == b'\n';
        host.partial_line.extend_from_slice(&buffer[..count]);
        host.stdout.consume(count);
        if ended {
            let bytes = std::mem::take(&mut host.partial_line);
            return serde_json::from_slice(&bytes).map_err(|_| "Invalid host JSON".into());
        }
    }
}
fn failure(status: StatusCode, message: impl Into<String>, unknown: bool) -> Response {
    (status, Json(json!({"ok":false,"error":message.into(),"outcome_unknown":unknown}))).into_response()
}
pub(crate) async fn action(State(server): State<Arc<super::Server>>, Path(operation): Path<String>, headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>) -> Response {
    if !super::machines::same_origin(&headers) { return failure(StatusCode::FORBIDDEN, "Machine commands must come from this engine's own page.", false); }
    let body = match body { Ok(Json(body)) => body, Err(_) => return failure(StatusCode::BAD_REQUEST, "Provide a valid JSON machine request.", false) };
    let bridge = server.machines.session.clone();
    // A lost HTTP connection must not cancel a partially written machine request.
    // The task runs to its bounded conclusion and persists any unknown state.
    match tokio::spawn(async move { bridge.request(&operation, body).await }).await {
        Ok(response) => response,
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "Machine request task failed. Outcome unknown.", true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture { bridge: SessionBridge, path: PathBuf }
    impl Fixture {
        fn new(behavior: &str) -> Self {
            let path = std::env::temp_dir().join(format!("de-machine-host-{}-{}.py", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
            let script = format!(r#"import json, sys, time
behavior = {behavior:?}
for line in sys.stdin:
    req = json.loads(line)
    with open(__file__ + '.calls', 'a') as calls: calls.write(req['op'] + '\n')
    if behavior == 'slow' and req['op'] == 'start': time.sleep(0.2)
    if behavior == 'overflow' and req['op'] == 'status':
        sys.stdout.write('x' * (4 * 1024 * 1024 + 1)); sys.stdout.flush(); continue
    if behavior == 'exit' and req['op'] == 'status': sys.exit(0)
    result = {{'state': {{'connect':'connected','prepare':'prepared','start':'running','abort':'aborted','close':'closed'}}.get(req['op'], 'connected'), 'connected': req['op'] != 'close', 'backend': 'simulation', 'outcome_unknown': False}}
    reply = {{'id': 'wrong' if behavior == 'wrong-id' and req['op'] == 'status' else req['id'], 'ok': True, 'result': result}}
    if behavior == 'driver-fault' and req['op'] == 'pause':
        reply.update(ok=False, error='driver control failed')
        result.update(state='faulted', outcome_unknown=True)
    print(json.dumps(reply), flush=True)
    if req['op'] == 'close': break
"#);
            std::fs::write(&path, script).unwrap();
            let mut bridge = SessionBridge::new(FsPath::new("/unused"), Arc::new(Mutex::new(())));
            bridge.script = path.clone(); bridge.python = PathBuf::from("python3"); bridge.timeout = Duration::from_secs(2);
            Self {bridge, path}
        }
        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(format!("{}.calls", self.path.display())).unwrap_or_default().lines().map(str::to_owned).collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(format!("{}.calls", self.path.display()));
        }
    }
    fn connect() -> Value { json!({"family":"jcz","backend":"simulation","configuration":{}}) }
    async fn body(response: Response) -> Value {
        serde_json::from_slice(&axum::body::to_bytes(response.into_body(), OUTPUT_LIMIT + 4096).await.unwrap()).unwrap()
    }
    #[test]
    fn validates_fixed_operations_fields_and_physical_consent() {
        assert!(validate("connect", &connect()).is_ok());
        for (op, request) in [("shell", json!({})), ("status", json!({"op":"start"})),
            ("prepare", json!({"job":[]})), ("connect", json!({"family":"delta_nc5","backend":"simulation","configuration":{}})),
            ("connect", json!({"family":"grbl","backend":"physical","configuration":{}})),
            ("connect", json!({"family":"grbl","backend":"simulation","configuration":{},"executable":"evil"})),
            ("start", json!({"authorized":"true"}))] { assert!(validate(op, &request).is_err(), "{op}: {request}"); }
        assert!(validate("prepare", &json!({"job":{"data":"x".repeat(INPUT_LIMIT)}})).is_err());
    }
    #[tokio::test]
    async fn status_and_invalid_requests_never_start_child() {
        let fixture = Fixture::new("normal");
        assert_eq!(body(fixture.bridge.request("status",json!({})).await).await["result"]["state"], "disconnected");
        assert_eq!(fixture.bridge.request("start",json!({"authorized":true})).await.status(), StatusCode::CONFLICT);
        assert_eq!(fixture.bridge.request("connect",json!({"family":"jcz","backend":"physical","configuration":{}})).await.status(), StatusCode::BAD_REQUEST);
        assert!(fixture.calls().is_empty());
        assert!(fixture.bridge.state.lock().await.process.is_none());
    }
    #[tokio::test]
    async fn lifecycle_uses_one_process_and_releases_global_owner_only_on_close() {
        let fixture = Fixture::new("normal");
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(), StatusCode::OK);
        assert!(fixture.bridge.machine_owner.try_lock().is_err());
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(), StatusCode::CONFLICT);
        for (op, request) in [("prepare",json!({"job":{}})),("start",json!({"authorized":true})),("status",json!({})),("pause",json!({})),("resume",json!({})),("abort",json!({})),("close",json!({}))] {
            assert_eq!(fixture.bridge.request(op,request).await.status(),StatusCode::OK, "{op}");
        }
        assert!(fixture.bridge.machine_owner.try_lock().is_ok());
        assert_eq!(fixture.calls(), ["connect","prepare","start","status","pause","resume","abort","close"]);
    }
    #[tokio::test]
    async fn competing_request_rejected_without_queuing_or_replay() {
        let fixture=Fixture::new("normal");
        let _held=fixture.bridge.state.lock().await;
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(),StatusCode::CONFLICT);
        assert!(fixture.calls().is_empty());
    }
    #[tokio::test]
    async fn protocol_failure_poisoned_until_explicit_disconnect() {
        let fixture=Fixture::new("wrong-id");
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(),StatusCode::OK);
        let failure=body(fixture.bridge.request("status",json!({})).await).await;
        assert_eq!(failure["outcome_unknown"],true);
        assert_eq!(fixture.bridge.request("start",json!({"authorized":true})).await.status(),StatusCode::CONFLICT);
        assert_eq!(fixture.calls(),["connect","status"]);
        let closed=body(fixture.bridge.request("close",json!({})).await).await;
        assert_eq!(closed["outcome_unknown"],true);
        assert!(fixture.bridge.state.lock().await.process.is_none());
    }
    #[tokio::test]
    async fn timeout_does_not_replay_or_spawn_a_replacement() {
        let mut fixture=Fixture::new("slow");
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(),StatusCode::OK);
        fixture.bridge.timeout=Duration::from_millis(30);
        assert_eq!(fixture.bridge.request("start",json!({"authorized":true})).await.status(),StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(),StatusCode::CONFLICT);
        assert_eq!(fixture.bridge.request("start",json!({"authorized":true})).await.status(),StatusCode::CONFLICT);
        fixture.bridge.timeout=Duration::from_secs(2);
        let closed=body(fixture.bridge.request("close",json!({})).await).await;
        assert_eq!(closed["outcome_unknown"],true);
        assert_eq!(fixture.calls(),["connect","start","close"]);
    }
    #[tokio::test]
    async fn bounded_reply_and_eof_are_unknown_not_completion() {
        for behavior in ["overflow","exit"] {
            let fixture=Fixture::new(behavior);
            assert_eq!(fixture.bridge.request("connect",connect()).await.status(),StatusCode::OK);
            let failure=body(fixture.bridge.request("status",json!({})).await).await;
            assert_eq!(failure["outcome_unknown"],true);
            fixture.bridge.request("close",json!({})).await;
        }
    }
    #[tokio::test]
    async fn physical_start_requires_its_own_explicit_consent() {
        let fixture=Fixture::new("normal");
        let mut request=connect();request["backend"]=json!("physical");request["authorized"]=json!(true);
        assert_eq!(fixture.bridge.request("connect",request).await.status(),StatusCode::OK);
        assert_eq!(fixture.bridge.request("start",json!({})).await.status(),StatusCode::BAD_REQUEST);
        assert_eq!(fixture.calls(),["connect"]);
        fixture.bridge.request("close",json!({})).await;
    }
    #[tokio::test]
    async fn driver_reported_uncertainty_blocks_commands_and_survives_close() {
        let fixture=Fixture::new("driver-fault");
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(),StatusCode::OK);
        let failed=body(fixture.bridge.request("pause",json!({})).await).await;
        assert_eq!(failed["result"]["outcome_unknown"],true);
        assert_eq!(fixture.bridge.request("start",json!({"authorized":true})).await.status(),StatusCode::CONFLICT);
        assert_eq!(fixture.calls(),["connect","pause"]);
        let closed=body(fixture.bridge.request("close",json!({})).await).await;
        assert_eq!(closed["outcome_unknown"],true);
        assert_eq!(closed["host_disconnected"],true);
    }
    #[tokio::test]
    async fn dropped_request_waiter_does_not_cancel_or_replay_host_operation() {
        let fixture=Fixture::new("slow");
        assert_eq!(fixture.bridge.request("connect",connect()).await.status(),StatusCode::OK);
        let bridge=fixture.bridge.clone();
        let handle=tokio::spawn(async move {bridge.request("start",json!({"authorized":true})).await});
        // Same detached ownership as the HTTP handler: dropping its waiter must
        // not discard a half exchange or release the machine request lock.
        drop(handle);
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(fixture.bridge.request("status",json!({})).await.status(),StatusCode::OK);
        assert_eq!(fixture.calls(),["connect","start","status"]);
        fixture.bridge.request("close",json!({})).await;
    }
    #[tokio::test]
    #[ignore = "requires the local cad_software machine host and pinned offline environment; simulation only"]
    async fn actual_simulation_host_lifecycle() {
        let repo=FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bridge=SessionBridge::new(&repo,Arc::new(Mutex::new(())));
        let preparation=repo.join("../cad_software/study/machine-runtime/prepare_job.py");
        let output=Command::new(&bridge.python).arg("-B").arg(preparation).arg("--examples").output().await.unwrap();
        assert!(output.status.success());
        let examples:Value=serde_json::from_slice(&output.stdout).unwrap();
        for family in ["jcz","ruida","grbl"] {
            let connected=body(bridge.request("connect",json!({"family":family,"backend":"simulation","configuration":{}})).await).await;
            assert_eq!(connected["ok"],true,"{connected}");
            let prepared=body(bridge.request("prepare",json!({"job":examples["examples"][family]})).await).await;
            assert_eq!(prepared["ok"],true,"{prepared}");
            let started=body(bridge.request("start",json!({"authorized":true})).await).await;
            assert_eq!(started["ok"],true,"{started}");
            let snapshot=body(bridge.request("status",json!({})).await).await;
            assert_eq!(snapshot["ok"],true,"{snapshot}");
            let stopped=body(bridge.request("abort",json!({})).await).await;
            assert_eq!(stopped["ok"],true,"{stopped}");
            let closed=body(bridge.request("close",json!({})).await).await;
            assert_eq!(closed["ok"],true,"{closed}");
        }
    }
}
