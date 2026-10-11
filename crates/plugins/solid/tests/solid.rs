use de_core::{Actor, Envelope, ErrorCode, Project, Registry, SCHEMA_VERSION, StableId, Status, Value, json};
use de_solid::{BoxFeature, DisplayMesh, MassProperties, SolidPlugin};
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin])))
}

fn run(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, de_core::CommandError> {
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

fn mass(p: &Project, part: StableId) -> f64 {
    p.get::<MassProperties>(part).unwrap().mass_kg.unwrap()
}

#[test]
fn mass_follows_dimensions_and_material() {
    let mut p = project();
    let steel = id(run(&mut p, "solid.material.create", json!({ "name": "steel", "density": { "value": 7850, "unit": "kg/m3" } })).unwrap());
    let part = id(run(&mut p, "solid.box.create", json!({ "width": mm(10.0), "depth": mm(20.0), "height": mm(30.0), "material": steel })).unwrap());

    let props = p.get::<MassProperties>(part).unwrap();
    assert!((props.volume_m3 - 6e-6).abs() < 1e-15);
    assert!((mass(&p, part) - 0.0471).abs() < 1e-12);
    assert_eq!(p.get::<DisplayMesh>(part).unwrap().indices.len(), 36);

    run(&mut p, "solid.box.set_dimensions", json!({ "target": part, "height": mm(60.0) })).unwrap();
    assert!((mass(&p, part) - 0.0942).abs() < 1e-12);

    run(&mut p, "solid.material.set_density", json!({ "target": steel, "density": { "value": 2.7, "unit": "g/cm3" } })).unwrap();
    assert!((mass(&p, part) - 12e-6 * 2700.0).abs() < 1e-12);
    assert_eq!(p.status(part), Some(&Status::Current));
}

#[test]
fn invalid_geometry_is_rejected_without_changes() {
    let mut p = project();
    let part = id(run(&mut p, "solid.box.create", json!({ "width": mm(10.0), "depth": mm(10.0), "height": mm(10.0) })).unwrap());
    let err = run(&mut p, "solid.box.set_dimensions", json!({ "target": part, "width": mm(0.0) })).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidGeometry);
    assert_eq!(p.get::<BoxFeature>(part).unwrap().width.canonical(), 0.01);
    assert_eq!(p.get::<MassProperties>(part).unwrap().mass_kg, None);
}

#[test]
fn deleting_a_material_blocks_its_parts() {
    let mut p = project();
    let alu = id(run(&mut p, "solid.material.create", json!({ "name": "aluminium", "density": { "value": 2700, "unit": "kg/m3" } })).unwrap());
    let part = id(run(&mut p, "solid.box.create", json!({ "width": mm(1.0), "depth": mm(1.0), "height": mm(1.0), "material": alu })).unwrap());

    run(&mut p, "core.entity.delete", json!({ "target": alu })).unwrap();
    assert!(matches!(p.status(part), Some(Status::Blocked(_))));

    run(&mut p, "solid.material.assign", json!({ "target": part, "material": null })).unwrap();
    assert_eq!(p.status(part), Some(&Status::Current));
    assert_eq!(p.get::<MassProperties>(part).unwrap().mass_kg, None);
}

#[test]
fn edit_box_changes_size_and_placement_atomically() {
    let mut p = project();
    let part = id(run(&mut p, "solid.box.create", json!({ "width": mm(40.0), "depth": mm(40.0), "height": mm(20.0) })).unwrap());
    let before = p.revision();
    // Pull the -X face out by 10 mm: wider, and shifted so the +X face stays put.
    run(&mut p, "solid.box.edit", json!({ "target": part, "width": mm(50.0), "placement": { "x": mm(-10.0), "y": mm(0.0), "z": mm(0.0) } })).unwrap();
    assert_eq!(p.revision().0, before.0 + 1, "one change set");
    assert!((p.get::<BoxFeature>(part).unwrap().width.canonical() - 0.05).abs() < 1e-12);
    assert!((p.get::<de_solid::Placement>(part).unwrap().x.canonical() + 0.01).abs() < 1e-12);
    // Invalid size rolls back the placement too.
    let err = run(&mut p, "solid.box.edit", json!({ "target": part, "width": mm(0.0), "placement": { "x": mm(5.0), "y": mm(0.0), "z": mm(0.0) } })).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidGeometry);
    assert!((p.get::<de_solid::Placement>(part).unwrap().x.canonical() + 0.01).abs() < 1e-12);
}

#[test]
fn pivot_is_stored_validated_and_undoable() {
    let mut p = project();
    let part = id(run(&mut p, "solid.box.create", json!({ "width": mm(10.0), "depth": mm(10.0), "height": mm(10.0) })).unwrap());
    assert!(p.get::<de_solid::Pivot>(part).is_none(), "default pivot is the center");
    run(&mut p, "solid.part.set_pivot", json!({ "target": part, "u": 0.0, "v": 0.5, "w": 0.0 })).unwrap();
    assert_eq!(p.get::<de_solid::Pivot>(part), Some(&de_solid::Pivot { u: 0.0, v: 0.5, w: 0.0 }));
    // Outside the box is allowed (a hinge beside the part); absurd values are not.
    run(&mut p, "solid.part.set_pivot", json!({ "target": part, "u": 1.5, "v": 0.5, "w": 0.0 })).unwrap();
    let err = run(&mut p, "solid.part.set_pivot", json!({ "target": part, "u": 1e6, "v": 0.5, "w": 0.0 })).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParameters);
    p.undo(&de_core::Actor { user_id: "u".into(), session_id: "s".into() }).unwrap();
    p.undo(&de_core::Actor { user_id: "u".into(), session_id: "s".into() }).unwrap();
    assert!(p.get::<de_solid::Pivot>(part).is_none());
}
