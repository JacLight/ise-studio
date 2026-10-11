use de_core::{Actor, CommandError, Envelope, ErrorCode, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_intent::{IntentPlugin, RequirementResult, Severity, check};
use de_solid::SolidPlugin;
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &IntentPlugin])))
}

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

fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}

fn boxed(p: &mut Project, size: [f64; 3], at: [f64; 3], material: Option<StableId>) -> StableId {
    id(run(p, "solid.box.create", json!({
        "width": mm(size[0]), "depth": mm(size[1]), "height": mm(size[2]), "material": material,
        "placement": { "x": mm(at[0]), "y": mm(at[1]), "z": mm(at[2]) }
    })).unwrap())
}

fn result(p: &Project, req: StableId) -> RequirementResult {
    p.get::<RequirementResult>(req).expect("requirement evaluated").clone()
}

/// Applies the requirement's computed fix and returns the new result.
fn apply_fix(p: &mut Project, req: StableId) -> RequirementResult {
    let fix = result(p, req).fix.expect("a failing requirement offers a fix");
    run(p, &fix.operation, fix.parameters).unwrap();
    result(p, req)
}

#[test]
fn mass_limit_tracks_edits_and_its_fix_works() {
    let mut p = project();
    let steel = id(run(&mut p, "solid.material.create", json!({ "name": "Steel", "density": { "value": 7850, "unit": "kg/m3" } })).unwrap());
    let part = boxed(&mut p, [100.0, 100.0, 10.0], [0.0; 3], Some(steel)); // 0.785 kg
    let req = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "max_mass", "parts": [part], "limit": { "value": 1, "unit": "kg" } } })).unwrap());
    assert!(result(&p, req).passed);

    run(&mut p, "solid.box.set_dimensions", json!({ "target": part, "height": mm(20.0) })).unwrap(); // 1.57 kg
    let failing = result(&p, req);
    assert!(!failing.passed);
    assert!((failing.measured - 1.57).abs() < 1e-9);
    assert_eq!(failing.fix.as_ref().unwrap().label, "Reduce Box 100 × 100 × 20 height to 12.7 mm");

    let fixed = apply_fix(&mut p, req);
    assert!(fixed.passed, "{}", fixed.summary);
}

#[test]
fn clearance_is_exact_and_fix_restores_it() {
    let mut p = project();
    let a = boxed(&mut p, [10.0; 3], [0.0; 3], None);
    let b = boxed(&mut p, [10.0; 3], [13.0, 0.0, 0.0], None);
    let req = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "clearance", "a": a, "b": b, "min": mm(2.0) } })).unwrap());
    let ok = result(&p, req);
    assert!(ok.passed);
    assert_eq!(ok.summary, "3 mm apart, 2 mm required");

    run(&mut p, "solid.part.move", json!({ "target": b, "placement": { "x": mm(8.0), "y": mm(0.0), "z": mm(0.0) } })).unwrap();
    let bad = result(&p, req);
    assert!(!bad.passed);
    assert_eq!(bad.summary, "Parts overlap by 200 mm³");
    assert_eq!(bad.violations[0].region, Some([8.0, 0.0, 0.0, 10.0, 10.0, 10.0]));

    let fixed = apply_fix(&mut p, req);
    assert!(fixed.passed);
    assert!((fixed.measured - 2.0).abs() < 1e-9, "fix lands exactly on the minimum");
}

#[test]
fn envelope_and_collisions() {
    let mut p = project();
    let a = boxed(&mut p, [50.0, 40.0, 30.0], [0.0; 3], None);
    let b = boxed(&mut p, [20.0; 3], [45.0, 0.0, 0.0], None);
    let fits = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "fits_within", "parts": [a, b], "width": mm(60.0), "depth": mm(40.0), "height": mm(30.0) } })).unwrap());
    let none = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "no_collisions", "parts": [a, b] } })).unwrap());
    assert!(!result(&p, fits).passed, "65 mm wide does not fit 60");
    assert_eq!(result(&p, fits).summary, "65 × 40 × 30 in 60 × 40 × 30 mm");
    assert!(!result(&p, none).passed);

    apply_fix(&mut p, none);
    assert!(result(&p, none).passed);
}

#[test]
fn deleting_a_part_blocks_and_check_reports_everything() {
    let mut p = project();
    let a = boxed(&mut p, [10.0; 3], [0.0; 3], None);
    let b = boxed(&mut p, [10.0; 3], [5.0, 0.0, 0.0], None);
    let c = boxed(&mut p, [10.0; 3], [40.0, 0.0, 0.0], None);
    id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "clearance", "a": a, "b": c, "min": mm(1.0) } })).unwrap());

    let report = check(&p);
    assert!(report.ok, "overlaps and missing materials are not errors");
    assert_eq!(report.requirements_passing, 1);
    assert!(report.findings.iter().any(|f| f.code == "PARTS_OVERLAP" && f.severity == Severity::Warning && f.fix.is_some()));
    assert_eq!(report.findings.iter().filter(|f| f.code == "NO_MATERIAL").count(), 3);
    let _ = b;

    run(&mut p, "core.entity.delete", json!({ "target": c })).unwrap();
    let report = check(&p);
    assert!(!report.ok);
    assert!(report.findings.iter().any(|f| f.code == "REQUIREMENT_BLOCKED"));
}

#[test]
fn invalid_requirements_are_rejected() {
    let mut p = project();
    let a = boxed(&mut p, [10.0; 3], [0.0; 3], None);
    let err = run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "clearance", "a": a, "b": a, "min": mm(1.0) } })).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParameters);
    let err = run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "no_collisions", "parts": [a] } })).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParameters);
}

#[test]
fn shapes_are_judged_by_exact_geometry_not_bounding_boxes() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &de_shape::ShapePlugin, &IntentPlugin])));
    let shape = |p: &mut Project, name: &str, source: &str| -> StableId {
        id(run(p, "shape.create", json!({ "name": name, "source": source, "params": [] })).unwrap())
    };
    // A rod inside a tube: their bounding boxes overlap completely, but the solids never touch.
    let rod = shape(&mut p, "Rod", "return cylinder(40, 4, { segments: 256 });");
    let tube = shape(&mut p, "Tube", "return cylinder(20, 8, { segments: 256 }).subtract(cylinder(22, 4.5, { segments: 256 }).translate(0, 0, -1));");
    let gap = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "clearance", "a": rod, "b": tube, "min": mm(0.4) } })).unwrap());
    let none = id(run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "no_collisions", "parts": [rod, tube] } })).unwrap());
    let r = result(&p, gap);
    assert!(r.passed, "{}", r.summary);
    assert!((r.measured - 0.5).abs() < 0.01, "radial gap 4.5 − 4 = 0.5 mm, measured {}", r.measured);
    assert!(result(&p, none).passed, "bounding boxes overlap, solids do not");
    assert!(check(&p).findings.iter().all(|f| f.code != "PARTS_OVERLAP"));

    // Shrink the tube's bore below the rod: now the solids really intersect.
    run(&mut p, "shape.update", json!({ "target": tube, "source": "return cylinder(20, 8, { segments: 256 }).subtract(cylinder(22, 3.5, { segments: 256 }).translate(0, 0, -1));" })).unwrap();
    let r = result(&p, gap);
    assert!(!r.passed);
    assert!(r.summary.starts_with("Parts overlap by"), "{}", r.summary);
    assert!(!result(&p, none).passed);
}
