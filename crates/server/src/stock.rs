//! The stock library over HTTP: `GET /stock` lists every family and size
//! (part numbers, prices, masses, sizes, connectors), with the families the
//! project already holds; `GET /stock/<family>/preview?size=` is one size's
//! mesh for a preview. Inserting is the `stock.insert` command, so it is
//! journaled, undoable and synced like any change.

use crate::Server;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use de_core::json;
use std::collections::HashMap;
use std::sync::Arc;

pub async fn list(State(server): State<Arc<Server>>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let summary = match tokio::task::spawn_blocking(de_shape::library::summary).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    // Which families this project already has a definition of.
    let mut held = serde_json::Map::new();
    if let Ok(app) = server.project(query.get("project"))
        && let Ok(cat) = de_shape::library::catalogue()
    {
        let project = app.project.lock().unwrap();
        for f in &cat.families {
            if let Some(id) = de_shape::library::definition_of(project.world(), &f.id) {
                held.insert(f.id.clone(), json!(id));
            }
        }
    }
    let mut out = summary.clone();
    out["inProject"] = serde_json::Value::Object(held);
    axum::Json(out).into_response()
}

pub async fn preview(Path(family): Path<String>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let size = query.get("size").cloned();
    match tokio::task::spawn_blocking(move || de_shape::library::preview(&family, size.as_deref())).await {
        Ok(Ok(v)) => axum::Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use de_core::{Actor, Project, Registry, StableId, Value, json};
    use std::sync::Arc;

    fn project() -> Project {
        Project::new(
            "stock-test",
            Arc::new(Registry::with_plugins(&[&de_solid::SolidPlugin, &de_shape::ShapePlugin, &de_assembly::AssemblyPlugin, &de_intent::IntentPlugin])),
        )
    }

    fn commit(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, String> {
        let actor = Actor { user_id: "local".into(), session_id: StableId::new().to_string() };
        let e = crate::envelope(p, &actor, None, operation.into(), parameters, p.revision());
        p.commit(e).map(|c| c.output).map_err(|e| e.message)
    }

    fn id(v: &Value) -> StableId {
        serde_json::from_value(v.clone()).unwrap()
    }

    #[test]
    fn inserting_stock_makes_one_definition_per_family_and_places_sizes() {
        let mut p = project();
        let a = commit(&mut p, "stock.insert", json!({ "family": "iso4762", "size": "M3×12", "at": [10, 0, 0] })).unwrap();
        assert_eq!(a["created"], true);
        let def = id(&a["definition"]);
        let b = commit(&mut p, "stock.insert", json!({ "family": "iso4762", "size": "M8×40" })).unwrap();
        assert_eq!(b["created"], false, "the second insert reuses the definition");
        assert_eq!(b["updated"], false);
        assert_eq!(id(&b["definition"]), def);
        // One material, at the stainless density, not one per insert.
        let materials = p.entity_ids().into_iter().filter(|i| p.entity(*i).and_then(|e| p.world().get::<de_solid::Material>(e)).is_some()).count();
        assert_eq!(materials, 1);
        // The instances show their sizes; the configured mass is the catalogue's within 10 %.
        let w = p.world();
        let inst = w.get::<de_assembly::Instance>(p.entity(id(&b["id"])).unwrap()).unwrap();
        assert_eq!(inst.config.as_deref(), Some("M8×40"));
        let c = de_shape::configs::evaluate(w, p.entity(def).unwrap(), "M8×40").unwrap();
        let catalogue = c.stats.declared.stock.as_ref().unwrap().mass_g.unwrap();
        let modelled = c.mass.mass_kg.unwrap() * 1000.0;
        assert!((modelled / catalogue - 1.0).abs() < 0.1, "modelled {modelled} g vs catalogue {catalogue} g");
        assert_eq!(c.stats.declared.stock.as_ref().unwrap().part_number, "ISO 4762 M8×40 A2-70");
        let table = w.get::<de_shape::ShapeConfigurations>(p.entity(def).unwrap()).unwrap();
        assert!(table.row("M2×4").is_some() && table.row("M8×80").is_some());
        // Only the definition, no instance, with place: false.
        let d = commit(&mut p, "stock.insert", json!({ "family": "iso4762", "place": false })).unwrap();
        assert!(d["id"].is_null());
    }

    #[test]
    fn beams_cut_to_many_lengths_share_one_definition_and_cost_by_length() {
        let mut p = project();
        let a = commit(&mut p, "stock.insert", json!({ "family": "ipe", "size": "IPE 200", "length": 2350 })).unwrap();
        assert_eq!(a["size"], "IPE 200 × 2350");
        let b = commit(&mut p, "stock.insert", json!({ "family": "ipe", "size": "IPE 200", "length": 4000 })).unwrap();
        assert_eq!(id(&a["definition"]), id(&b["definition"]));
        // A plain size later keeps the cut lengths already in the table.
        commit(&mut p, "stock.insert", json!({ "family": "ipe", "size": "IPE 100" })).unwrap();
        let w = p.world();
        let def = p.entity(id(&a["definition"])).unwrap();
        let table = w.get::<de_shape::ShapeConfigurations>(def).unwrap();
        assert!(table.params.contains(&"length".to_string()));
        assert!(table.row("IPE 200 × 2350").is_some() && table.row("IPE 200 × 4000").is_some() && table.row("IPE 100").is_some());
        let short = de_shape::configs::evaluate(w, def, "IPE 200 × 2350").unwrap();
        let long = de_shape::configs::evaluate(w, def, "IPE 200 × 4000").unwrap();
        let (s, l) = (short.stats.declared.stock.clone().unwrap(), long.stats.declared.stock.clone().unwrap());
        assert_ne!(s.part_number, l.part_number);
        let ratio = l.unit_price.unwrap() / s.unit_price.unwrap();
        assert!((ratio - 4000.0 / 2350.0).abs() < 0.01, "price follows the length: {ratio}");
        // Something sold by the piece has no length to cut.
        let e = commit(&mut p, "stock.insert", json!({ "family": "iso4762", "size": "M3×12", "length": 50 })).unwrap_err();
        assert!(e.contains("not cut to length"), "{e}");
    }

    #[test]
    fn inserting_an_unknown_family_or_size_says_what_there_is() {
        let mut p = project();
        let e = commit(&mut p, "stock.insert", json!({ "family": "iso9999" })).unwrap_err();
        assert!(e.contains("no family") && e.contains("iso4762"), "{e}");
        let e = commit(&mut p, "stock.insert", json!({ "family": "iso4762", "size": "M3×13" })).unwrap_err();
        assert!(e.contains("no size") && e.contains("M2×4"), "{e}");
    }
}
