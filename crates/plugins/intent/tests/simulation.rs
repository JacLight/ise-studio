//! Requirements on what a part's saved study finds: re-checked on every
//! change that matters, and only then.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_intent::{IntentPlugin, RequirementResult};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::Arc;

fn run(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, CommandError> {
    let envelope = Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().into(),
        expected_revision: p.revision(),
        operation: operation.into(),
        parameters,
        actor: Actor { user_id: "u".into(), session_id: "s".into() },
    };
    p.commit(envelope).map(|c| c.output)
}

fn id(v: Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

fn result(p: &Project, req: StableId) -> RequirementResult {
    p.get::<RequirementResult>(req).unwrap_or_else(|| panic!("requirement not evaluated: {:?}", p.get::<de_core::Status>(req))).clone()
}

#[test]
fn a_printed_bracket_keeps_its_safety_factor_requirement() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &IntentPlugin])));
    let pla = id(run(&mut p, "solid.material.create", json!({ "name": "PLA", "density": { "value": 1240, "unit": "kg/m3" } })).unwrap());
    // An L bracket: a plate t thick and 60 long off a wall plate, 50 N down on its end.
    let bracket = id(run(&mut p, "shape.create", json!({
        "name": "Bracket",
        "source": "return box(60, 30, p.t).add(box(p.t, 30, 50));",
        "params": [{ "key": "t", "label": "Thickness", "value": 8 }]
    })).unwrap());
    run(&mut p, "solid.material.assign", json!({ "target": bracket, "material": pla })).unwrap();
    // Faces given by a point near them, as the AI gives them: pinned when saved.
    run(&mut p, "shape.set_study", json!({ "target": bracket, "study": {
        "supports": [{ "normal": [-1, 0, 0], "near": [0, 15, 25] }],
        "loads": [{ "face": { "normal": [1, 0, 0], "near": [60, 15, 1] }, "magnitude": 50, "direction": "-z" }],
        "resolution": 48, "refine": false
    } })).unwrap();
    let saved = p.get::<de_shape::ShapeStudies>(bracket).unwrap().0[0].clone();
    assert!(saved.supports[0].near.is_none() && (saved.supports[0].at[0]).abs() < 1e-9, "pinned to where it sits: {:?}", saved.supports[0]);
    let safe = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "min_safety_factor", "shape": bracket, "min": 2.0 } })).unwrap());
    let stiff = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "max_deflection", "shape": bracket, "max": { "value": 1.0, "unit": "mm" } } })).unwrap());
    let r = result(&p, safe);
    // Beam theory at the root of a 60 mm arm, 30 × 8: σ = 6FL/bt² = 9.4 MPa, ×5.3 on PLA's 50 MPa.
    eprintln!("bracket t = 8: {}", r.summary);
    assert!(r.passed && r.measured > 3.0 && r.measured < 8.0, "{}", r.summary);
    assert!(result(&p, stiff).passed, "{}", result(&p, stiff).summary);
    // Thinner: 4 mm reads 37.5 MPa by beam theory, ×1.3: the requirement fails, on the next commit.
    run(&mut p, "shape.set_params", json!({ "target": bracket, "values": { "t": 4 } })).unwrap();
    let r = result(&p, safe);
    eprintln!("bracket t = 4: {}", r.summary);
    assert!(!r.passed && r.measured < 2.0, "{}", r.summary);
    assert_eq!(r.violations.len(), 1);
    // Renaming the requirement changes nothing the study depends on: answered from memory, fast.
    let started = std::time::Instant::now();
    run(&mut p, "intent.requirement.update", json!({ "target": safe, "name": "Bracket strong enough" })).unwrap();
    assert!(started.elapsed().as_millis() < 500, "re-solved for a rename ({} ms)", started.elapsed().as_millis());
    assert_eq!(result(&p, safe).measured, r.measured);
    // A stronger material for its name: set on the material, the check re-runs and passes again.
    run(&mut p, "solid.material.set_properties", json!({ "target": pla, "properties": { "yield_strength": 120, "source": "test" } })).unwrap();
    let r = result(&p, safe);
    eprintln!("bracket t = 4, 120 MPa material: {}", r.summary);
    assert!(r.passed, "{}", r.summary);
}

#[test]
fn a_column_keeps_its_buckling_and_frequency_requirements() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &IntentPlugin])));
    let steel = id(run(&mut p, "solid.material.create", json!({ "name": "Steel", "density": { "value": 7850, "unit": "kg/m3" } })).unwrap());
    let column = id(run(&mut p, "shape.create", json!({ "name": "Column", "source": "return box(10, 10, 200);", "params": [] })).unwrap());
    run(&mut p, "solid.material.assign", json!({ "target": column, "material": steel })).unwrap();
    run(&mut p, "shape.set_study", json!({ "target": column, "study": {
        "supports": [{ "normal": [0, 0, -1], "near": [5, 5, 0] }],
        "loads": [{ "face": { "normal": [0, 0, 1], "near": [5, 5, 200] }, "magnitude": 2000, "direction": "-z" }],
        "resolution": 80, "refine": false
    } })).unwrap();
    // Euler: 10.3 kN, so 2 kN is ×5.1.
    let buckle = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "min_buckling_factor", "shape": column, "min": 4.0 } })).unwrap());
    let r = result(&p, buckle);
    eprintln!("column: {}", r.summary);
    assert!(r.passed && (r.measured - 5.14).abs() < 0.4, "{}", r.summary);
    // Held at its foot: f₁ = (1.875²/2π)·√(EI/ρAL⁴) ≈ 204 Hz.
    let ring = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "min_first_frequency", "shape": column, "min": 250 } })).unwrap());
    let r = result(&p, ring);
    eprintln!("column: {}", r.summary);
    assert!(!r.passed && (r.measured - 204.0).abs() < 12.0, "{}", r.summary);
}
