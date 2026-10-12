//! Assembly analyses over HTTP: degrees of freedom, interference, exploded
//! views, motion studies and robot kinematics. Each reads the project as it
//! is (nothing is changed; the commands that change things go through the
//! websocket like every other edit).

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use de_assembly::robot::{self, Rig, Target, Waypoint};
use de_core::{Project, StableId, bevy_ecs::world::World, json};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

type Response = axum::response::Response;

fn fail(code: StatusCode, message: String) -> Response {
    (code, [(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string())], message).into_response()
}

/// Runs `f` on the project named in the query, off the async threads.
async fn read<T: Serialize + Send + 'static>(server: Arc<Server>, q: &HashMap<String, String>, f: impl FnOnce(&Project, &World) -> Result<T, String> + Send + 'static) -> Response {
    let app = match server.project(q.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let result = tokio::task::spawn_blocking(move || {
        let project = app.project.lock().unwrap();
        f(&project, project.world())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// An assembly (by id or name), or the top of the project ("top" / "project").
fn scope(project: &Project, id: &str) -> Result<Option<StableId>, String> {
    if id == "top" || id == "project" {
        return Ok(None);
    }
    let sid = serde_json::from_value::<StableId>(json!(id)).ok().filter(|s| project.get::<de_assembly::AssemblyDef>(*s).is_some()).or_else(|| project.entity_ids().into_iter().find(|e| project.get::<de_assembly::AssemblyDef>(*e).is_some_and(|d| d.name == id)));
    match sid {
        Some(s) => Ok(Some(s)),
        None => {
            // An instance of an assembly stands for its assembly.
            let inst = serde_json::from_value::<StableId>(json!(id)).ok().and_then(|s| project.get::<de_assembly::Instance>(s).cloned()).or_else(|| project.entity_ids().into_iter().find_map(|e| project.get::<de_assembly::Instance>(e).filter(|i| i.name == id).cloned()));
            match inst {
                Some(i) if project.get::<de_assembly::AssemblyDef>(i.of).is_some() => Ok(Some(i.of)),
                _ => Err(format!("“{id}” is not an assembly (or “top” for the top of the project)")),
            }
        }
    }
}

/// An entity of kind `T` by id or by a name `name` gives it.
fn named<T: de_core::bevy_ecs::component::Component>(project: &Project, id: &str, name: impl Fn(&T) -> &str, what: &str) -> Result<StableId, String> {
    if let Ok(s) = serde_json::from_value::<StableId>(json!(id))
        && project.get::<T>(s).is_some()
    {
        return Ok(s);
    }
    project.entity_ids().into_iter().find(|e| project.get::<T>(*e).is_some_and(|x| name(x) == id)).ok_or_else(|| format!("no {what} “{id}”"))
}

fn json_param<T: serde::de::DeserializeOwned>(q: &HashMap<String, String>, key: &str) -> Result<Option<T>, String> {
    match q.get(key).filter(|s| !s.is_empty()) {
        Some(s) => serde_json::from_str(s).map(Some).map_err(|e| format!("{key}: {e}")),
        None => Ok(None),
    }
}

/// `GET /assembly-dof/<assembly | top>`: what can still move, mobility,
/// redundant and conflicting joints, per group of joined instances.
pub async fn dof(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    read(server, &q, move |p, w| de_assembly::dof::report(w, scope(p, &id)?)).await
}

/// `GET /interference/<assembly | instance | top>?clearance=mm`: every pair
/// of parts that overlap (volume, where), touch, or come within the clearance.
pub async fn interference(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let clearance = q.get("clearance").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    read(server, &q, move |p, w| {
        let direct = serde_json::from_value::<StableId>(json!(id)).ok().filter(|s| p.get::<de_assembly::AssemblyDef>(*s).is_some() || p.get::<de_assembly::Instance>(*s).is_some());
        let of = match direct {
            Some(s) => Some(s),
            None => scope(p, &id)?,
        };
        de_assembly::interference::report(w, of, clearance)
    })
    .await
}

/// `GET /explode/<view id or name>`: a saved exploded view worked out —
/// every instance's placement after each step, and their trails.
pub async fn explode(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    read(server, &q, move |p, w| de_assembly::views::explode(w, named::<de_assembly::views::ExplodedView>(p, &id, |v| &v.name, "exploded view")?)).await
}

/// `GET /motion-study/<study id or name>?fps=24&gaps=off`: the study played:
/// every instance's placement at each frame, the joints' values, clashes.
pub async fn motion_study(State(server): State<Arc<Server>>, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let fps = q.get("fps").and_then(|v| v.parse::<f64>().ok()).unwrap_or(24.0);
    let gaps = q.get("gaps").is_none_or(|g| g != "off");
    read(server, &q, move |p, w| de_assembly::views::play_study(w, named::<de_assembly::views::MotionStudy>(p, &id, |v| &v.name, "motion study")?, fps, gaps)).await
}

fn rig(w: &World, tip: &str, q: &HashMap<String, String>) -> Result<Rig, String> {
    let tip = robot::tip_ref(w, tip)?;
    let point: Option<[f64; 3]> = json_param(q, "point")?;
    Rig::new(w, tip, q.get("tool").filter(|t| !t.is_empty()).map(|t| t.as_str()), point)
}

/// `GET /robot/<tip>?tool=&point=`: the chain from the ground to the tip
/// instance — its moving joints (with limits), their values, where the tool is.
pub async fn chain(State(server): State<Arc<Server>>, Path(tip): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let q2 = q.clone();
    read(server, &q, move |_, w| rig(w, &tip, &q2)?.info()).await
}

/// `GET /robot/<tip>/fk?values={"Shoulder":30}`: where the tool is for these values.
pub async fn fk(State(server): State<Arc<Server>>, Path(tip): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let q2 = q.clone();
    read(server, &q, move |_, w| {
        let r = rig(w, &tip, &q2)?;
        let given: HashMap<String, f64> = json_param(&q2, "values")?.unwrap_or_default();
        let mut v = r.now();
        for (name, x) in &given {
            let k = r.axes.iter().position(|a| &a.name == name).ok_or_else(|| format!("the chain has no joint “{name}” (it has {})", r.axes.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")))?;
            v[k] = *x;
        }
        let t = r.fk(&v)?;
        Ok(json!({ "at": t.at, "axis": t.turn([0.0, 0.0, 1.0]), "quaternion": t.quaternion() }))
    })
    .await
}

/// `GET /robot/<tip>/ik?to=[x,y,z]&axis=[x,y,z]`: joint values that put the tool there (nothing is changed).
pub async fn ik(State(server): State<Arc<Server>>, Path(tip): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let q2 = q.clone();
    read(server, &q, move |_, w| {
        let r = rig(w, &tip, &q2)?;
        let to: [f64; 3] = json_param(&q2, "to")?.ok_or("give `to`: [x, y, z] (mm)")?;
        let axis: Option<[f64; 3]> = json_param(&q2, "axis")?;
        r.ik(&Target { at: to, axis }, &r.now())
    })
    .await
}

/// `GET /robot/<tip>/reach?samples=2000`: where the tool can get to within the joints' limits.
pub async fn reach(State(server): State<Arc<Server>>, Path(tip): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let samples = q.get("samples").and_then(|v| v.parse::<usize>().ok()).unwrap_or(2000);
    let q2 = q.clone();
    read(server, &q, move |_, w| robot::reach(&rig(w, &tip, &q2)?, samples)).await
}

/// `GET /robot/<tip>/trajectory?through=[{"pose":"home"},{"to":[x,y,z],"linear":true}]&fps=12&gaps=off`:
/// the arm moved through poses, joint values and tool points, played with clashes.
pub async fn trajectory(State(server): State<Arc<Server>>, Path(tip): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let fps = q.get("fps").and_then(|v| v.parse::<f64>().ok()).unwrap_or(12.0);
    let gaps = q.get("gaps").is_none_or(|g| g != "off");
    let q2 = q.clone();
    read(server, &q, move |_, w| {
        let r = rig(w, &tip, &q2)?;
        let through: Vec<Waypoint> = json_param(&q2, "through")?.ok_or("give `through`: a list of waypoints ({ pose } | { values } | { to, axis?, linear? })")?;
        robot::trajectory(w, &r, &through, fps, gaps)
    })
    .await
}
