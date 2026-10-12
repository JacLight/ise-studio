//! Simulation over HTTP: stress (with fatigue), vibration (optionally
//! prestressed), buckling, heat (steady or over time), convergence, beam
//! frames, the material library, reading a solved study back (probes,
//! sections, paths, CSV), CalculiX decks, and a part's saved study run as
//! it is kept (what `./forma stress|modes|buckle|heat …` calls).

use crate::{App, Server};
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_core::{StableId, Value, json};
use de_shape::{
    ShapeProgram,
    fea::{self, materials::MaterialProps},
    studies,
};
use std::collections::HashMap;
use std::sync::Arc;

type Response = axum::response::Response;
type Parts = Vec<(Option<String>, MaterialProps)>;

fn fail(code: StatusCode, message: impl Into<String>) -> Response {
    (code, message.into()).into_response()
}

/// A shape, the parts a study takes (the studied one first) with their
/// materials, and its saved study for `part` (if any).
struct Target {
    program: ShapeProgram,
    parts: Parts,
    saved: Option<fea::StudySpec>,
}

fn target(server: &Server, q: &HashMap<String, String>, id: &str, part: Option<String>, others: &[String]) -> Result<(Arc<App>, Target), Response> {
    let app = server.project(q.get("project")).map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let found = {
        let project = app.project.lock().unwrap();
        let sid: Option<StableId> = serde_json::from_value(json!(id)).ok();
        let entity = sid.and_then(|s| project.entity(s));
        match entity {
            None => Err(fail(StatusCode::NOT_FOUND, format!("{id} is not a shape"))),
            Some(e) => {
                let world = project.world();
                match de_shape::resolved_program(world, e) {
                    None => Err(fail(StatusCode::NOT_FOUND, format!("{id} is not a shape"))),
                    Some(program) => {
                        let saved = world.get::<de_shape::ShapeStudies>(e).and_then(|s| s.0.iter().find(|st| st.part == part).cloned());
                        let shell: fea::StudySpec = serde_json::from_value(json!({ "part": part, "parts": others })).expect("a study of parts");
                        studies::study_parts(world, e, &shell).map(|parts| Target { program, parts, saved }).map_err(|e| fail(StatusCode::UNPROCESSABLE_ENTITY, e))
                    }
                }
            }
        }
    };
    found.map(|t| (app, t))
}

/// Runs blocking work off the async threads; an error is the person's to fix (422).
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, Response> {
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(fail(StatusCode::UNPROCESSABLE_ENTITY, e)),
        Err(e) => Err(fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[derive(serde::Deserialize)]
pub struct StudyRequest {
    #[serde(flatten)]
    study: fea::Study,
    /// One part of an assembly.
    #[serde(default)]
    part: Option<String>,
    /// Other parts studied with it (["all"]: every other part).
    #[serde(default)]
    parts: Vec<String>,
    /// With a stress study: how the loads come and go, for a fatigue estimate.
    #[serde(default)]
    fatigue: Option<fea::FatigueSpec>,
    /// Modes or buckling shapes wanted.
    #[serde(default)]
    count: Option<usize>,
    /// Vibration with the loads' stress counted.
    #[serde(default)]
    prestress: bool,
    /// A CalculiX deck: static, frequency or buckle.
    #[serde(default)]
    kind: Option<fea::InpKind>,
    /// A convergence report's resolutions.
    #[serde(default)]
    resolutions: Vec<usize>,
}

/// A stress study run and kept for reading back: the result, its material and the key to read it by.
fn stress_answer(r: fea::Result_, material: MaterialProps, fatigue: Option<&fea::FatigueSpec>) -> Result<Value, String> {
    let fat = fatigue.map(|f| fea::fatigue(&r, &material, f)).transpose()?;
    let summary = json!({ "result": r, "material": material.name, "properties": material, "fatigue": fat });
    let key = studies::keep(r, material);
    let mut out = summary;
    out["key"] = json!(key.to_string());
    Ok(out)
}

/// POST /stress/{id}: a stress study (the body is the study); `fatigue` adds a fatigue estimate.
pub async fn stress(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, axum::Json(req): axum::Json<StudyRequest>) -> Response {
    let (_, t) = match target(&server, &q, &id, req.part.clone(), &req.parts) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match blocking(move || {
        let r = studies::solve_study(&t.program, &t.parts, &req.study)?;
        stress_answer(r, t.parts[0].1.clone(), req.fatigue.as_ref())
    })
    .await
    {
        Ok(v) => axum::Json(v).into_response(),
        Err(r) => r,
    }
}

/// POST /modes/{id}: natural frequencies held by the study's supports (`prestress`: with its loads' stress).
pub async fn modes(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, axum::Json(req): axum::Json<StudyRequest>) -> Response {
    let (_, t) = match target(&server, &q, &id, req.part.clone(), &[]) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match blocking(move || studies::modes_of(&t.program, &t.parts[0], &req.study, req.count.unwrap_or(4), req.prestress)).await {
        Ok(m) => axum::Json(m).into_response(),
        Err(r) => r,
    }
}

/// POST /buckle/{id}: the factors on the study's loads that buckle the part.
pub async fn buckle(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, axum::Json(req): axum::Json<StudyRequest>) -> Response {
    let (_, t) = match target(&server, &q, &id, req.part.clone(), &[]) {
        Ok(x) => x,
        Err(r) => return r,
    };
    match blocking(move || studies::buckling_of(&t.program, &t.parts[0], &req.study, req.count.unwrap_or(3))).await {
        Ok(b) => axum::Json(b).into_response(),
        Err(r) => r,
    }
}

#[derive(serde::Deserialize)]
pub struct ThermalRequest {
    #[serde(flatten)]
    study: fea::ThermalStudy,
    #[serde(default)]
    part: Option<String>,
    /// Over time instead of steady.
    #[serde(default)]
    transient: Option<fea::Transient>,
    /// Points (mm) whose temperature through time is reported.
    #[serde(default)]
    probes: Vec<[f64; 3]>,
}

/// POST /thermal/{id}: steady temperatures, or (with `transient`) temperatures through time.
pub async fn thermal(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, axum::Json(req): axum::Json<ThermalRequest>) -> Response {
    let (_, t) = match target(&server, &q, &id, req.part.clone(), &[]) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let material = t.parts[0].1.name.clone();
    match blocking(move || match &req.transient {
        Some(run) => studies::transient_of(&t.program, &t.parts[0], &req.study, run, &req.probes).map(|r| json!({ "transient": r, "material": material })),
        None => studies::thermal_of(&t.program, &t.parts[0], &req.study).map(|r| json!({ "result": r, "material": material })),
    })
    .await
    {
        Ok(v) => axum::Json(v).into_response(),
        Err(r) => r,
    }
}

/// POST /converge/{id}: the study on ever finer cubes (`resolutions`, default three steps up from its own).
pub async fn converge(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, axum::Json(req): axum::Json<StudyRequest>) -> Response {
    let (_, t) = match target(&server, &q, &id, req.part.clone(), &req.parts) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let resolutions = if req.resolutions.is_empty() { default_ladder(req.study.resolution) } else { req.resolutions.clone() };
    match blocking(move || studies::convergence(&t.program, &t.parts, &req.study, &resolutions)).await {
        Ok(c) => axum::Json(c).into_response(),
        Err(r) => r,
    }
}

fn default_ladder(r: usize) -> Vec<usize> {
    let r = r.clamp(16, 96);
    vec![(r * 2 / 3).max(12), r, (r * 3 / 2).min(160)]
}

/// POST /inp/{id}: the study as a CalculiX deck (`kind`: static, frequency, buckle).
pub async fn inp(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, axum::Json(req): axum::Json<StudyRequest>) -> Response {
    let (_, t) = match target(&server, &q, &id, req.part.clone(), &req.parts) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let kind = req.kind.unwrap_or(fea::InpKind::Static);
    match blocking(move || studies::inp_of(&t.program, &t.parts, &req.study, kind, req.count.unwrap_or(5))).await {
        Ok(text) => deck(text, &id),
        Err(r) => r,
    }
}

fn deck(text: String, id: &str) -> Response {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"study-{id}.inp\""))], text).into_response()
}

fn kept(key: &str) -> Result<Arc<studies::Kept>, Response> {
    key.parse::<u64>().ok().and_then(studies::kept_result).ok_or_else(|| fail(StatusCode::NOT_FOUND, "that result is no longer kept: run the study again"))
}

fn point(q: &HashMap<String, String>, name: &str) -> Result<[f64; 3], Response> {
    let raw = q.get(name).ok_or_else(|| fail(StatusCode::BAD_REQUEST, format!("give {name}=x,y,z")))?;
    let v: Vec<f64> = raw.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if v.len() != 3 {
        return Err(fail(StatusCode::BAD_REQUEST, format!("{name} must be x,y,z in mm")));
    }
    Ok([v[0], v[1], v[2]])
}

/// GET /results/{key}/{what}: a kept stress result read back. `probe?at=x,y,z`,
/// `section?point=x,y,z&normal=x,y,z`, `path?a=…&b=…&points=50`, `csv`.
pub async fn results(Path((key, what)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
    let k = match kept(&key) {
        Ok(k) => k,
        Err(r) => return r,
    };
    let r = &k.result;
    let out = match what.as_str() {
        "probe" => point(&q, "at").map(|at| match fea::probe(r, at) {
            Some(p) => axum::Json(p).into_response(),
            None => fail(StatusCode::UNPROCESSABLE_ENTITY, "that point is not in the part"),
        }),
        "section" => point(&q, "point").and_then(|p| point(&q, "normal").map(|n| axum::Json(fea::section(r, p, n)).into_response())),
        "path" => point(&q, "a").and_then(|a| {
            point(&q, "b").map(|b| {
                let n = q.get("points").and_then(|v| v.parse().ok()).unwrap_or(50);
                let samples: Vec<Value> = fea::path(r, a, b, n).into_iter().map(|(s, p)| json!({ "distance": s, "probe": p })).collect();
                axum::Json(json!({ "a": a, "b": b, "samples": samples })).into_response()
            })
        }),
        "csv" => Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"study-{key}.csv\""))], fea::csv(r)).into_response()),
        other => Err(fail(StatusCode::NOT_FOUND, format!("no “{other}”: probe, section, path or csv"))),
    };
    out.unwrap_or_else(|r| r)
}

/// POST /results/{key}/fatigue: a fatigue estimate over a kept stress result.
pub async fn results_fatigue(Path(key): Path<String>, axum::Json(spec): axum::Json<fea::FatigueSpec>) -> Response {
    let k = match kept(&key) {
        Ok(k) => k,
        Err(r) => return r,
    };
    match fea::fatigue(&k.result, &k.material, &spec) {
        Ok(f) => axum::Json(f).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// POST /frame: a frame of beams (joints, members with sections, supports, loads).
pub async fn frame(axum::Json(frame): axum::Json<fea::frame::Frame>) -> Response {
    match blocking(move || fea::frame::solve(&frame)).await {
        Ok(r) => axum::Json(r).into_response(),
        Err(r) => r,
    }
}

/// GET /materials: the library (typical figures with sources), and the
/// project's materials as studies see them (with what was set on them).
pub async fn materials(State(server): State<Arc<Server>>, Query(q): Query<HashMap<String, String>>) -> Response {
    let app = match server.project(q.get("project")) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let project = app.project.lock().unwrap();
    let mut own = vec![];
    for id in project.entity_ids() {
        if let Some(m) = project.get::<de_solid::Material>(id) {
            let resolved = fea::materials::resolve(&m.name, m.properties.as_ref(), Some(m.density.canonical()));
            own.push(json!({ "id": id, "name": m.name, "set": m.properties, "properties": resolved }));
        }
    }
    axum::Json(json!({ "library": fea::materials::library(), "project": own })).into_response()
}

/// POST /simulate/{id}?part=&kind=stress|fatigue|modes|buckle|heat|converge|inp: the part's saved
/// study run as it is kept (what the AI's `./forma stress` and friends call).
pub async fn simulate(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let part = q.get("part").filter(|p| !p.is_empty()).cloned();
    let kind = q.get("kind").cloned().unwrap_or_else(|| "stress".into());
    let (_, t0) = match target(&server, &q, &id, part.clone(), &[]) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Some(spec) = t0.saved.clone() else {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{} has no saved study: save one with shape.set_study (supports and loads, faces given by a point near them and their normal)", part.as_deref().map_or("the shape".to_string(), |p| format!("“{p}”"))));
    };
    // The saved study's other parts, with their materials.
    let (_, t) = match target(&server, &q, &id, part.clone(), &spec.parts) {
        Ok(x) => x,
        Err(r) => return r,
    };
    let count: Option<usize> = q.get("count").and_then(|c| c.parse().ok());
    let resolutions: Vec<usize> = q.get("resolutions").map(|r| r.split(',').filter_map(|v| v.trim().parse().ok()).collect()).unwrap_or_default();
    let is_inp = kind == "inp";
    let out = blocking(move || {
        let program = &t.program;
        match kind.as_str() {
            "stress" | "fatigue" => {
                let study = studies::study_from_spec(program, &spec)?;
                let r = studies::solve_study(program, &t.parts, &study)?;
                let fatigue = if kind == "fatigue" { Some(spec.fatigue.clone().unwrap_or_default()) } else { spec.fatigue.clone() };
                stress_answer(r, t.parts[0].1.clone(), fatigue.as_ref()).map(|v| v.to_string())
            }
            "modes" => {
                let study = studies::study_from_spec(program, &spec)?;
                let m = studies::modes_of(program, &t.parts[0], &study, count.unwrap_or(4), spec.prestress)?;
                Ok(json!({ "modes": m.modes.iter().map(|m| json!({ "frequency": m.frequency, "direction": m.direction })).collect::<Vec<_>>(), "mass": m.mass, "notes": m.notes, "prestress": spec.prestress }).to_string())
            }
            "buckle" => {
                let study = studies::study_from_spec(program, &spec)?;
                let b = studies::buckling_of(program, &t.parts[0], &study, count.unwrap_or(3))?;
                Ok(json!({ "modes": b.modes.iter().map(|m| json!({ "factor": m.factor, "direction": m.direction })).collect::<Vec<_>>(), "max_stress": b.max_stress, "yield_factor": b.yield_factor, "notes": b.notes }).to_string())
            }
            "heat" => {
                let thermal = studies::thermal_from_spec(program, &spec)?;
                match &spec.transient {
                    Some(run) => {
                        let r = studies::transient_of(program, &t.parts[0], &thermal, run, &[])?;
                        Ok(json!({ "times": r.times, "max": r.max, "min": r.min, "last": { "max": r.last.max, "min": r.last.min, "heat_in": r.last.heat_in, "to_holds": r.last.to_holds, "to_air": r.last.to_air }, "notes": r.notes }).to_string())
                    }
                    None => {
                        let r = studies::thermal_of(program, &t.parts[0], &thermal)?;
                        Ok(json!({ "max": r.max, "min": r.min, "heat_in": r.heat_in, "to_holds": r.to_holds, "to_air": r.to_air, "notes": r.notes }).to_string())
                    }
                }
            }
            "converge" => {
                let study = studies::study_from_spec(program, &spec)?;
                let ladder = if resolutions.is_empty() { default_ladder(study.resolution) } else { resolutions };
                Ok(serde_json::to_string(&studies::convergence(program, &t.parts, &study, &ladder)?).unwrap_or_default())
            }
            "inp" => {
                let study = studies::study_from_spec(program, &spec)?;
                studies::inp_of(program, &t.parts, &study, fea::InpKind::Static, 5)
            }
            other => Err(format!("no study kind “{other}”: stress, fatigue, modes, buckle, heat, converge or inp")),
        }
    })
    .await;
    match out {
        Ok(text) if is_inp => deck(text, &id),
        Ok(text) => ([(header::CONTENT_TYPE, "application/json")], text).into_response(),
        Err(r) => r,
    }
}
