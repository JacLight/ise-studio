//! Local controller observations, outside project commands and their retries.
use axum::{extract::State, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, Json};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio::{process::Command, sync::Mutex};

pub(crate) struct MachineQueries {
    script: PathBuf,
    python: String,
    /// One USB query owner across every project and browser tab.
    active: Mutex<()>,
}

impl MachineQueries {
    pub(crate) fn new(repo: &std::path::Path) -> Self {
        Self {
            script: std::env::var_os("CAD_JCZ_QUERY_SCRIPT").map(PathBuf::from)
                .unwrap_or_else(|| repo.join("../cad_software/study/device-control/jcz_usb_query.py")),
            python: std::env::var("CAD_JCZ_PYTHON").unwrap_or_else(|_| "python3".into()),
            active: Mutex::new(()),
        }
    }
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"mode": "read_only_query", "framing_available": false, "error": message.into()}))).into_response()
}

fn parse_report(stdout: &[u8]) -> Result<Value, String> {
    let report: Value = serde_json::from_slice(stdout).map_err(|_| "The controller query returned an invalid report.")?;
    if !report.is_object() || !matches!(report["status"].as_str(), Some("queries_completed" | "failed" | "unavailable")) {
        return Err("The controller query returned an invalid report.".into());
    }
    Ok(report)
}

/// Explicit Refresh only. No query on boot, page mount, commit or reconnect.
pub(crate) async fn status(State(server): State<Arc<super::Server>>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return error(StatusCode::FORBIDDEN, "Controller queries must come from this engine's own page.");
    }
    query(&server.machines).await
}

fn same_origin(headers: &HeaderMap) -> bool {
    if headers.get("sec-fetch-site").is_some_and(|v| v == "cross-site") {
        return false;
    }
    match headers.get("origin") {
        None => true, // Local CLI clients; socket locality is checked by local_only.
        Some(origin) => headers.get("host").and_then(|host| host.to_str().ok())
            .is_some_and(|host| origin.to_str().ok() == Some(format!("http://{host}").as_str())),
    }
}

async fn query(bridge: &MachineQueries) -> Response {
    let Ok(_owner) = bridge.active.try_lock() else {
        return error(StatusCode::CONFLICT, "A controller query is already running on this computer.");
    };
    if !bridge.script.is_file() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "The local controller query tool is unavailable. Configure CAD_JCZ_QUERY_SCRIPT on this computer.");
    }
    let mut command = Command::new(&bridge.python);
    // A fixed read-only allowlist, with no arguments provided by HTTP clients.
    command.arg(&bridge.script).args(["--query", "serial", "--query", "version", "--query", "status", "--query", "position", "--timeout-ms", "500"]).kill_on_drop(true);
    let output = match tokio::time::timeout(Duration::from_secs(15), command.output()).await {
        Err(_) => return error(StatusCode::GATEWAY_TIMEOUT, "The controller query timed out; its process was stopped."),
        Ok(Err(_)) => return error(StatusCode::SERVICE_UNAVAILABLE, "The local controller query could not start. Check CAD_JCZ_PYTHON and the USB query tool installation."),
        Ok(Ok(output)) => output,
    };
    let report = match parse_report(&output.stdout) {
        Ok(report) => report,
        Err(message) => return error(StatusCode::BAD_GATEWAY, message),
    };
    let observed_at_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    // Preserve structured failure reports without claiming connection.
    let query_succeeded = output.status.success() && report["status"] == "queries_completed";
    Json(json!({
        "mode": "read_only_query", "query_succeeded": query_succeeded,
        "framing_available": false, "observed_at_ms": observed_at_ms, "report": report,
    })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_must_have_a_recognized_query_status() {
        assert!(parse_report(br#"{"status":"queries_completed"}"#).is_ok());
        assert!(parse_report(br#"{"status":"failed","errors":["No device"]}"#).is_ok());
        for invalid in [&b"not JSON"[..], &b"[]"[..], &b"null"[..], &b"{}"[..]] {
            assert!(parse_report(invalid).is_err());
        }
    }
    #[tokio::test]
    async fn busy_handler_returns_conflict_without_starting_a_process() {
        let bridge = MachineQueries {script: PathBuf::from("unused"), python: "unused".into(), active: Mutex::new(())};
        let _owner = bridge.active.try_lock().unwrap();
        assert_eq!(query(&bridge).await.status(), StatusCode::CONFLICT);
    }
    #[tokio::test]
    async fn absent_query_tool_is_reported_without_a_process() {
        let bridge = MachineQueries {script: PathBuf::from("/not-present/disabled-jcz-query.py"), python: "unused".into(), active: Mutex::new(())};
        assert_eq!(query(&bridge).await.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    #[test]
    fn browser_queries_require_the_same_origin_and_port() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:7420".parse().unwrap());
        headers.insert("origin", "http://127.0.0.1:7420".parse().unwrap());
        assert!(same_origin(&headers));
        headers.insert("origin", "http://127.0.0.1:9999".parse().unwrap());
        assert!(!same_origin(&headers));
        headers.insert("origin", "https://example.com".parse().unwrap());
        assert!(!same_origin(&headers));
        headers.remove("origin");
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!same_origin(&headers));
    }
}
