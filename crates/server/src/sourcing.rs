//! Bills of materials and requests for quote over HTTP. Everything is made
//! here and handed to the person; nothing is sent anywhere.
//!
//! - `GET /bom/<id|project>?config=…&qty=…&breaks=1,10,100&format=json|csv&view=flat|indented`
//! - `GET /bom/<id|project>/file?line=<key>&format=step|stl|3mf|dxf|pdf` — one line's file
//! - `GET /rfq/<rfq id>.zip` — every supplier's package, a folder each;
//!   `?supplier=<id>` — one supplier's package; `/rfq/<rfq id>` — the plan as
//!   JSON (each supplier's lines and a mailto: draft)
//! - `GET /rates` — process rates in force (the project's or the defaults),
//!   and the exchange rates with when, by whom and from where each was set

use crate::{App, Server, bad_request, today};
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{Project, StableId, Value, json};
use de_shape::fabricate;
use de_sourcing::{
    Rfq, Scope, Supplier,
    bom::{self, Bom},
    package,
};
use std::collections::HashMap;
use std::sync::Arc;

type Response = axum::response::Response;

fn app_of(server: &Server, q: &HashMap<String, String>) -> Result<(String, Arc<App>), Response> {
    let name = q.get("project").filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| server.default.clone());
    server.project(Some(&name)).map(|a| (name, a)).map_err(bad_request)
}

fn fail(code: StatusCode, message: String) -> Response {
    (code, [(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string())], message).into_response()
}

fn file(name: &str, mime: &str, bytes: Vec<u8>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, mime.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))], bytes).into_response()
}

fn scope_of(id: &str) -> Result<Scope, String> {
    if id == "project" {
        return Ok(Scope::Project);
    }
    serde_json::from_value::<StableId>(json!(id)).map(Scope::Entity).map_err(|_| format!("“{id}” is not an id (or “project”)"))
}

/// What a BOM is of, by name when not by id: an assembly, else an instance,
/// else a shape called that (`./ise bom "Robot arm"`).
fn named_scope(project: &Project, name: &str) -> Result<Scope, String> {
    let ids = project.entity_ids();
    let kinds: [&dyn Fn(StableId) -> bool; 3] = [
        &|e| project.get::<de_assembly::AssemblyDef>(e).is_some_and(|d| d.name == name),
        &|e| project.get::<de_assembly::Instance>(e).is_some_and(|i| i.name == name),
        &|e| project.get::<de_shape::ShapeProgram>(e).is_some_and(|p| p.name == name),
    ];
    for is in kinds {
        let found: Vec<StableId> = ids.iter().copied().filter(|e| is(*e)).collect();
        match found.as_slice() {
            [] => continue,
            [one] => return Ok(Scope::Entity(*one)),
            many => return Err(format!("{} things are called “{name}”: use an id ({})", many.len(), many.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", "))),
        }
    }
    Err(format!("“{name}” is not an id, “project”, or the name of an assembly, instance or shape"))
}

fn options(q: &HashMap<String, String>) -> Result<bom::Options, String> {
    let mut o = bom::Options { config: q.get("config").filter(|c| !c.is_empty()).cloned(), ..Default::default() };
    if let Some(n) = q.get("qty").filter(|n| !n.is_empty()) {
        o.quantity = n.parse().ok().filter(|n| *n >= 1 && *n <= 1_000_000).ok_or_else(|| format!("qty is a whole number of finished units from 1, not “{n}”"))?;
    }
    if let Some(b) = q.get("breaks").filter(|b| !b.is_empty()) {
        o.breaks = b.split(',').map(|x| x.trim().parse::<u32>().ok().filter(|n| *n >= 1)).collect::<Option<Vec<_>>>().ok_or("breaks are whole numbers, comma separated (1,10,100)")?;
    }
    Ok(o)
}

/// The project's title, for documents.
fn title(project: &Project, name: &str) -> String {
    project
        .entity_ids()
        .into_iter()
        .find_map(|id| project.get::<de_core::ProjectInfo>(id).map(|p| p.title.clone()))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| crate::pretty_name(name))
}

fn compute(project: &Project, scope: Scope, options: &bom::Options) -> Result<Bom, String> {
    let mut options = options.clone();
    options.rate_dates = rate_history(project).into_iter().map(|(code, set)| (code, set.date)).collect();
    bom::compute(project.world(), scope, &options, &[&de_pcb::sourcing::PcbBuySource])
}

/// When an exchange rate was last set, and by whom.
#[derive(Clone, Debug, PartialEq)]
pub struct RateSet {
    pub date: String,
    pub by: String,
    pub revision: u64,
}

/// The change that last set each exchange rate in force, from the
/// project's history (a change's id, or its session's, tells when it was
/// made). Rates set before the history began have none.
pub fn rate_history(project: &Project) -> std::collections::BTreeMap<String, RateSet> {
    let mut out = std::collections::BTreeMap::new();
    for change in project.journal() {
        for op in &change.ops {
            let de_core::Op::Set { kind, before, after, .. } = op else { continue };
            if *kind != "sourcing.rates" {
                continue;
            }
            let rates = |v: &Option<de_core::value::CompValue>| v.as_ref().map(|v| v.to_json()["exchange"].clone()).unwrap_or(Value::Null);
            let (was, now) = (rates(before), rates(after));
            let codes: std::collections::BTreeSet<String> = [&was, &now].iter().filter_map(|m| m.as_object()).flat_map(|m| m.keys().cloned()).collect();
            for code in codes {
                if was.get(&code) == now.get(&code) {
                    continue;
                }
                if now.get(&code).is_none() {
                    out.remove(&code);
                    continue;
                }
                let when = time_of(&change.command_id).or_else(|| time_of(&change.actor.session_id));
                match when {
                    Some(ms) => out.insert(code, RateSet { date: date_of(ms), by: change.actor.user_id.clone(), revision: change.revision.0 }),
                    None => out.remove(&code),
                };
            }
        }
    }
    out
}

/// The time (ms since 1970) a time-ordered id (UUID version 7) was made.
fn time_of(id: &str) -> Option<u64> {
    let hex: String = id.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) || hex.as_bytes()[12] != b'7' {
        return None;
    }
    u64::from_str_radix(&hex[..12], 16).ok()
}

/// A day as YYYY-MM-DD (UTC).
fn date_of(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
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

/// `GET /bom/<id>`: the bill of materials, as JSON (with the process rates
/// in force) or CSV (`view=indented` for the tree).
pub async fn bom(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (_, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let options = match options(&q) {
        Ok(o) => o,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let format = q.get("format").cloned().unwrap_or_else(|| "json".into());
    let view = q.get("view").cloned().unwrap_or_else(|| "flat".into());
    let result = tokio::task::spawn_blocking(move || {
        let project = app.project.lock().unwrap();
        let scope = scope_of(&id).or_else(|_| named_scope(&project, &id))?;
        let b = compute(&project, scope, &options)?;
        let rates = de_sourcing::rates::rates_of(project.world());
        Ok::<_, String>((b, rates))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    let (b, rates) = match result {
        Ok(x) => x,
        Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    match format.as_str() {
        "csv" => {
            let (text, suffix) = if view == "indented" { (bom::indented_csv(&b), "indented BOM") } else { (bom::flat_csv(&b), "BOM") };
            file(&format!("{} {suffix}.csv", fabricate::file_name(&b.name)), "text/csv; charset=utf-8", text.into_bytes())
        }
        "json" => {
            let table: Vec<Value> = rates.table().into_iter().map(|(p, r, default)| json!({ "process": p, "rate": r, "default": default })).collect();
            let mut v = serde_json::to_value(&b).unwrap();
            v["rates"] = json!(table);
            axum::Json(v).into_response()
        }
        other => fail(StatusCode::BAD_REQUEST, format!("format is json or csv, not {other}")),
    }
}

/// `GET /bom/<id>/file?line=…&format=…`: one Make line's file.
pub async fn line_file(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (scope, options) = match (scope_of(&id), options(&q)) {
        (Ok(s), Ok(o)) => (s, o),
        (Err(e), _) | (_, Err(e)) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let Some(key) = q.get("line").cloned() else { return fail(StatusCode::BAD_REQUEST, "say which line (line=make:…)".into()) };
    let ext: &'static str = match q.get("format").map(String::as_str) {
        Some("step") => "step",
        Some("stl") => "stl",
        Some("3mf") => "3mf",
        Some("dxf") => "dxf",
        Some("pdf") => "pdf",
        other => return fail(StatusCode::BAD_REQUEST, format!("format is step, stl, 3mf, dxf or pdf, not {}", other.unwrap_or("nothing"))),
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(String, &'static str, Vec<u8>), String> {
        let (b, programs, context) = {
            let project = app.project.lock().unwrap();
            let b = compute(&project, scope, &options)?;
            let programs = bom::line_programs(project.world(), &b);
            (b, programs, package::Context { project: title(&project, &name), date: today(), from: String::new() })
        };
        let line = b.make_line(&key).ok_or_else(|| format!("no made part has line {key} (bought items have no files)"))?;
        let number = format!("{}-{:02}", fabricate::file_name(&context.project).to_uppercase(), line.item);
        let mut files = package::Files::new(&programs, 0.005, context);
        let bytes = files.file(&key, ext, &line.name, line.material.as_deref(), &number)?;
        let mime = match ext {
            "step" => "model/step",
            "stl" => "model/stl",
            "3mf" => "model/3mf",
            "dxf" => "image/vnd.dxf",
            _ => "application/pdf",
        };
        Ok((format!("{}.{ext}", fabricate::file_name(&line.name)), mime, bytes))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok((n, mime, bytes)) => file(&n, mime, bytes),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// An RFQ, its BOM and its suppliers, read under the lock.
fn rfq_parts(project: &Project, rfq: StableId) -> Result<(Rfq, Bom, Vec<(StableId, Supplier)>), String> {
    let r = project.get::<Rfq>(rfq).cloned().ok_or_else(|| format!("{rfq} is not an RFQ"))?;
    let scope = r.scope.map(Scope::Entity).unwrap_or(Scope::Project);
    let b = compute(project, scope, &bom::Options { config: r.config.clone(), ..Default::default() })?;
    let suppliers = r.suppliers.iter().filter_map(|s| project.get::<Supplier>(*s).map(|x| (*s, x.clone()))).collect();
    Ok((r, b, suppliers))
}

/// `GET /rfq/<id>[.zip]`: the RFQ package (zip) or its plan (JSON).
pub async fn rfq(State(server): State<Arc<Server>>, Path(stem): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (name, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let (id, zip) = match stem.strip_suffix(".zip") {
        Some(s) => (s.to_string(), true),
        None => (stem.clone(), false),
    };
    let Ok(rfq_id) = serde_json::from_value::<StableId>(json!(id)) else { return fail(StatusCode::BAD_REQUEST, format!("“{id}” is not an RFQ id")) };
    let only: Option<StableId> = match q.get("supplier").filter(|s| !s.is_empty()) {
        Some(s) => match serde_json::from_value(json!(s)) {
            Ok(s) => Some(s),
            Err(_) => return fail(StatusCode::BAD_REQUEST, format!("“{s}” is not a supplier id")),
        },
        None => None,
    };
    let from = q.get("from").cloned().unwrap_or_default();
    let result = tokio::task::spawn_blocking(move || -> Result<Response, String> {
        let (r, b, suppliers, programs, context) = {
            let project = app.project.lock().unwrap();
            let (r, b, suppliers) = rfq_parts(&project, rfq_id)?;
            let programs = bom::line_programs(project.world(), &b);
            (r, b, suppliers, programs, package::Context { project: title(&project, &name), date: today(), from })
        };
        let suppliers: Vec<(StableId, Supplier)> = match only {
            Some(s) => suppliers.into_iter().filter(|(id, _)| *id == s).collect(),
            None => suppliers,
        };
        if suppliers.is_empty() {
            return Err(if only.is_some() { "that supplier is not on this RFQ".into() } else { "the RFQ has no suppliers yet: add some, then make the package".into() });
        }
        if !zip {
            let plan: Vec<Value> = suppliers
                .iter()
                .map(|(id, s)| {
                    let lines = package::plan(&r, &b, s);
                    let files: Vec<String> = lines.iter().flat_map(|l| l.files.iter().map(|e| package::file_name(l, e)).collect::<Vec<_>>()).collect();
                    json!({
                        "supplier": id,
                        "name": s.name,
                        "lines": lines.iter().map(|l| json!({ "line": l.key, "item": l.item, "name": l.name, "qty": l.qty, "missing": l.missing })).collect::<Vec<_>>(),
                        "files": files,
                        "mailto": package::mailto(&r, s, lines.len(), &context),
                        "letter": package::cover_letter(&r, s, &lines, &context),
                    })
                })
                .collect();
            return Ok(axum::Json(json!({ "rfq": rfq_id, "suppliers": plan })).into_response());
        }
        let mut files = package::Files::new(&programs, 0.005, context);
        let (bytes, file_name) = if let [(_, s)] = &suppliers[..]
            && only.is_some()
        {
            let (all, _) = package::supplier_package(&r, &b, s, &mut files, "");
            (fabricate::zip(&all), format!("{} - {}.zip", fabricate::file_name(&r.name), fabricate::file_name(&s.name)))
        } else {
            (fabricate::zip(&package::rfq_package(&r, &b, &suppliers, &mut files)), format!("{} RFQ.zip", fabricate::file_name(&r.name)))
        };
        Ok(file(&file_name, "application/zip", bytes))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(r) => r,
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// `GET /rates`: the process rates in force, and the exchange rates: what
/// one unit of each currency is worth in the project's, when and by whom it
/// was set, and where it came from (a reference rate, or by hand).
pub async fn rates(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> Response {
    let (_, app) = match app_of(&server, &q) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let project = app.project.lock().unwrap();
    let r = de_sourcing::rates::rates_of(project.world());
    let table: Vec<Value> = r.table().into_iter().map(|(p, rate, default)| json!({ "process": p, "rate": rate, "default": default })).collect();
    let history = rate_history(&project);
    let options = bom::Options { rate_dates: history.iter().map(|(c, s)| (c.clone(), s.date.clone())).collect(), ..Default::default() };
    let exchange: Vec<Value> = r
        .exchange
        .iter()
        .map(|(code, rate)| {
            let set = history.get(code);
            json!({
                "currency": code, "rate": rate,
                "set_on": set.map(|s| s.date.clone()), "by": set.map(|s| s.by.clone()), "revision": set.map(|s| s.revision),
                "source": r.exchange_source.get(code), "note": bom::rate_note(code, &r, &options),
            })
        })
        .collect();
    axum::Json(json!({ "currency": r.currency(), "exchange": exchange, "processes": table })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body(r: Response) -> String {
        String::from_utf8(axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap()
    }

    #[test]
    fn a_change_id_tells_the_day_it_was_made() {
        assert_eq!(time_of("01a1349b-83c8-7090-b86f-b624be2910b1").map(date_of).as_deref(), Some("2026-10-13"));
        assert_eq!(time_of("run-0123456789abcdef0123456789abcdef"), None);
        assert_eq!(time_of("2b6f0c1e-4f2a-4c3b-9d1e-0123456789ab"), None, "not time-ordered (version 4)");
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400_000), "2000-02-29");
    }

    /// A rate set by hand before rates carried dates (robot-arm's EUR 1.15)
    /// says the day from the history, unchanged; a reference rate says where from.
    #[tokio::test(flavor = "multi_thread")]
    async fn exchange_rates_say_when_and_how_they_were_set() {
        let dir = std::env::temp_dir().join(format!("ise-rates-{}", StableId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let line = json!({ "revision": 1, "base_revision": 0, "command_id": "01a1349b-83c8-7090-b86f-b624be2910b1", "operation": "sourcing.rates.set",
            "actor": { "user_id": "ise-ai", "session_id": "01a1349b-8095-7220-9767-ed8cb646a3d9" },
            "ops": [{ "op": "create", "id": "01a1349b-83c8-7090-b86f-b6257fccf216" }, { "op": "set", "id": "01a1349b-83c8-7090-b86f-b6257fccf216", "kind": "sourcing.rates", "before": null, "after": { "exchange": { "EUR": 1.15 } } }] });
        std::fs::write(dir.join("arm.jsonl"), format!("{line}\n")).unwrap();
        let server = crate::tests::server_in(&dir);
        let app = server.project(Some(&"arm".to_string())).unwrap();
        let spring = r#"stock({ partNumber: "RZ-075NI", manufacturer: "Gutekunst Federn", unitPrice: 6.40, currency: "EUR", pack: 1, massG: 3.5 }); return cylinder(5, 30);"#;
        app.commit_here("shape.create", json!({ "name": "Spring", "params": [], "source": spring, "placement": { "x": { "value": 0, "unit": "mm" }, "y": { "value": 0, "unit": "mm" }, "z": { "value": 0, "unit": "mm" } } })).unwrap();
        let q = |extra: &[(&str, &str)]| Query([("project", "arm")].iter().chain(extra).map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>());

        let r: Value = serde_json::from_str(&body(rates(State(server.clone()), q(&[])).await).await).unwrap();
        assert_eq!(r["currency"], "USD");
        assert_eq!(r["exchange"], json!([{ "currency": "EUR", "rate": 1.15, "set_on": "2026-10-13", "by": "ise-ai", "revision": 1, "source": null, "note": "rate set on 2026-10-13 by hand" }]));
        let csv = body(bom(State(server.clone()), Path("project".into()), q(&[("format", "csv")])).await).await;
        assert!(csv.contains("\"list (EUR at 1.15 to USD, rate set on 2026-10-13 by hand)\""), "{csv}");
        let json: Value = serde_json::from_str(&body(bom(State(server.clone()), Path("project".into()), q(&[])).await).await).unwrap();
        assert!(json["flags"].as_array().unwrap().iter().any(|f| f["text"].as_str().unwrap().ends_with("converted to USD at 1.15 (rate set on 2026-10-13 by hand)")), "{}", json["flags"]);

        // Refreshed from a reference rate: today, and where it came from.
        app.commit_here("sourcing.rates.set", json!({ "exchange": { "EUR": 1.1634 }, "source": "ECB reference rate of 2026-10-12" })).unwrap();
        let r: Value = serde_json::from_str(&body(rates(State(server.clone()), q(&[])).await).await).unwrap();
        let today = crate::today();
        assert_eq!((r["exchange"][0]["rate"].clone(), r["exchange"][0]["set_on"].clone(), r["exchange"][0]["by"].clone()), (json!(1.1634), json!(today), json!("local")));
        assert_eq!(r["exchange"][0]["note"], format!("ECB reference rate of 2026-10-12, set on {today}"));
        // Removed: gone from the list.
        app.commit_here("sourcing.rates.set", json!({ "exchange": { "EUR": 0 } })).unwrap();
        let r: Value = serde_json::from_str(&body(rates(State(server.clone()), q(&[])).await).await).unwrap();
        assert_eq!(r["exchange"], json!([]));
        assert!(rate_history(&app.project.lock().unwrap()).is_empty());
    }
}
