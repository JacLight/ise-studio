//! The KiCad libraries and the footprint check, over HTTP (the parts library
//! in the Electronics workspace and `./ise pcb …` use these):
//!
//! - `GET  /pcb/kicad`                              what is downloaded, where, and the licence
//! - `POST /pcb/kicad/fetch?repo=footprints|symbols|all`   download (again)
//! - `GET  /pcb/kicad/search?q=&kind=symbols|footprints&limit=`   parts matching every word
//! - `GET  /pcb/kicad/footprint/<Library:Name>`     a footprint in Iṣẹ́ Studio's terms (for `pcb.footprint.define`)
//! - `GET  /pcb/kicad/symbol/<Library:Name>`        a symbol (for `pcb.schematic.place`'s `definition`),
//!   its default footprint read too, and a preview drawing
//! - `GET  /pcb/verify-footprints?names=a,b`        built-in footprints against KiCad's, pad by pad
//!
//! The first search or read downloads the library it needs (about 12 MB);
//! everything runs off the async threads.

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
};
use de_core::{StableId, Value, json};
use de_pcb::kicad_lib::{Library, Repo};
use std::collections::HashMap;

fn fail(code: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (code, message.into()).into_response()
}

async fn blocking<F: FnOnce() -> Result<Value, String> + Send + 'static>(f: F) -> axum::response::Response {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => axum::Json(v).into_response(),
        Ok(Err(e)) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub async fn status() -> axum::response::Response {
    axum::Json(json!(Library::default().status())).into_response()
}

pub async fn fetch(Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let which = query.get("repo").cloned().unwrap_or_else(|| "all".into());
    blocking(move || {
        let lib = Library::default();
        let repos: Vec<Repo> = match which.as_str() {
            "footprints" => vec![Repo::Footprints],
            "symbols" => vec![Repo::Symbols],
            _ => vec![Repo::Footprints, Repo::Symbols],
        };
        for r in repos {
            lib.fetch(r)?;
        }
        Ok(json!(lib.status()))
    })
    .await
}

pub async fn search(Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let q = query.get("q").cloned().unwrap_or_default();
    let kind = query.get("kind").cloned().unwrap_or_else(|| "symbols".into());
    let limit = query.get("limit").and_then(|l| l.parse().ok()).unwrap_or(40usize).min(500);
    blocking(move || {
        let lib = Library::default();
        let repo = if kind.starts_with("foot") { Repo::Footprints } else { Repo::Symbols };
        let hits = lib.search(&q, repo, limit)?;
        Ok(json!({ "query": q, "kind": kind, "results": hits, "licence": de_pcb::kicad_lib::LICENCE }))
    })
    .await
}

pub async fn footprint(Path(id): Path<String>) -> axum::response::Response {
    blocking(move || {
        let (fp, notes) = Library::default().footprint(&id)?;
        Ok(json!({ "footprint": fp, "notes": notes, "courtyard": fp.courtyard_rect(), "silk": fp.silk_lines() }))
    })
    .await
}

pub async fn symbol(Path(id): Path<String>) -> axum::response::Response {
    blocking(move || {
        let lib = Library::default();
        let def = lib.symbol(&id)?;
        // Its default footprint, when the library names one.
        let (footprint, notes, footprint_error) = if def.footprint.contains(':') {
            match lib.footprint(&def.footprint) {
                Ok((f, n)) => (Some(f), n, None),
                Err(e) => (None, vec![], Some(e)),
            }
        } else {
            (None, vec![], None)
        };
        let s = de_pcb::schematic::SchSymbol {
            schematic: StableId::new(),
            sheet: "1".into(),
            symbol: def.name.clone(),
            reference: format!("{}?", def.prefix),
            value: def.value.clone(),
            footprint: def.footprint.clone(),
            at: [0.0, 0.0],
            rotation: 0.0,
            mirror: false,
            unit: 1,
            mpn: None,
            manufacturer: None,
            lcsc: None,
            description: String::new(),
            dnp: false,
            pin_map: Default::default(),
            pins: vec![],
            fields: Default::default(),
            ref_at: None,
            value_at: None,
            exclude_from_board: false,
            component: None,
            definition: Some(Box::new(def.clone())),
        };
        let v = de_pcb::schematic_view::symbol_preview(&s);
        // Pins whose number is no pad of the footprint (the pin map to check before using it).
        let unmapped: Vec<String> = match &footprint {
            Some(f) => def.pins.iter().filter(|p| !p.hidden || p.etype != de_pcb::symbols::PinType::NoConnect).filter(|p| !f.pads.iter().any(|q| q.number == p.number)).map(|p| p.number.clone()).collect(),
            None => vec![],
        };
        Ok(json!({ "definition": def, "footprint": footprint, "footprint_notes": notes, "footprint_error": footprint_error, "unmapped_pins": unmapped, "preview": { "draw": v.draw, "bbox": v.bbox, "pins": v.pins } }))
    })
    .await
}

pub async fn verify(Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    let names: Vec<String> = query.get("names").map(|n| n.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    blocking(move || {
        let lib = Library::default();
        lib.ensure(Repo::Footprints)?;
        Ok(json!(de_pcb::verify::verify(&lib, &names)))
    })
    .await
}
