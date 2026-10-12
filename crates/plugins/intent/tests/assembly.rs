use de_assembly::{AssemblyPlugin, AssemblyStats};
use de_core::{Actor, CommandError, Envelope, ErrorCode, Project, Registry, SCHEMA_VERSION, StableId, Status, Value, json};
use de_intent::{IntentPlugin, RequirementResult};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin, &IntentPlugin])))
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

fn id(v: &Value, key: &str) -> StableId {
    serde_json::from_value(v[key].clone()).unwrap()
}

/// A 10 m × 10 m floor plate, 300 mm thick, reporting its area; `width` changes it.
fn floor(p: &mut Project, material: StableId) -> StableId {
    let out = run(p, "shape.create", json!({
        "name": "Typical floor",
        "source": "metric('Net area', p.width * 10000 / 1e6, 'm²'); return box(p.width, 10000, 300);",
        "params": [{ "key": "width", "label": "Width", "value": 10000 }],
        "definition": true,
        "display_tolerance": 1,
        "material": material
    })).unwrap();
    id(&out, "id")
}

#[test]
fn instances_share_a_definition_until_made_unique() {
    let mut p = project();
    let concrete = id(&run(&mut p, "solid.material.create", json!({ "name": "Concrete", "density": { "value": 2400, "unit": "kg/m3" } })).unwrap(), "id");
    let typical = floor(&mut p, concrete);
    let tower = id(&run(&mut p, "assembly.create", json!({ "name": "Tower" })).unwrap(), "id");
    let mut floors = Vec::new();
    for n in 0..3 {
        let out = run(&mut p, "assembly.instance.create", json!({
            "of": typical, "parent": tower, "name": format!("L{}", n + 1), "at": [0, 0, n as f64 * 3500.0],
            "properties": { "status": "Available" }
        })).unwrap();
        floors.push(id(&out, "id"));
    }
    let site = id(&run(&mut p, "assembly.instance.create", json!({ "of": tower, "name": "Tower", "at": [1000, 0, 0] })).unwrap(), "id");

    assert_eq!(p.status(site), Some(&Status::Current), "tower {:?}", p.status(tower));
    let stats = p.get::<AssemblyStats>(site).unwrap().clone();
    assert_eq!(stats.shapes, 3);
    assert_eq!(stats.bounds_min, [1000.0, 0.0, 0.0]);
    assert_eq!(stats.bounds_max, [11000.0, 10000.0, 7300.0]);
    assert!((stats.metrics["Net area"] - 300.0).abs() < 1e-9, "3 × 100 m²");
    let mass = 3.0 * 10.0 * 10.0 * 0.3 * 2400.0;
    assert!((stats.mass_kg.unwrap() - mass).abs() < 1e-6);

    // Editing the definition changes every floor, all the way up.
    run(&mut p, "shape.set_params", json!({ "target": typical, "values": { "width": 12000 } })).unwrap();
    assert_eq!(p.get::<AssemblyStats>(site).unwrap().bounds_max[0], 13000.0);

    // A sold floor gets its own copy: changing it leaves the others alone.
    let out = run(&mut p, "assembly.instance.make_unique", json!({ "target": floors[1], "name": "L2 penthouse" })).unwrap();
    let custom = id(&out, "definition");
    run(&mut p, "shape.set_params", json!({ "target": custom, "values": { "width": 20000 } })).unwrap();
    run(&mut p, "assembly.instance.update", json!({ "target": floors[1], "properties": { "status": "Sold", "owner": "Buyer A" } })).unwrap();
    let s = p.get::<AssemblyStats>(site).unwrap();
    assert_eq!(s.bounds_max[0], 21000.0);
    assert!((s.metrics["Net area"] - (120.0 + 200.0 + 120.0)).abs() < 1e-9);
    assert_eq!(p.status(site), Some(&Status::Current));

    // An assembly cannot contain itself.
    let err = run(&mut p, "assembly.instance.create", json!({ "of": tower, "parent": tower, "name": "loop" })).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParameters);

    // Removing a floor keeps the tower valid.
    run(&mut p, "assembly.instance.delete", json!({ "target": floors[2] })).unwrap();
    assert_eq!(p.get::<AssemblyStats>(site).unwrap().shapes, 2);
    assert_eq!(p.status(tower), Some(&Status::Current));
}

#[test]
fn requirements_see_through_assemblies_and_rotations() {
    let mut p = project();
    let slab = id(&run(&mut p, "shape.create", json!({
        "name": "Slab", "source": "return box(4000, 1000, 200);", "params": [], "definition": true, "display_tolerance": 1
    })).unwrap(), "id");
    let pair = id(&run(&mut p, "assembly.create", json!({ "name": "Pair" })).unwrap(), "id");
    run(&mut p, "assembly.instance.create", json!({ "of": slab, "parent": pair, "name": "A", "at": [0, 0, 0] })).unwrap();
    // Rotated 90° about Z at x = 2000, y = 2000: it crosses A.
    let b = id(&run(&mut p, "assembly.instance.create", json!({ "of": slab, "parent": pair, "name": "B", "at": [2500, -1000, 0], "rotation": [0, 0, 90] })).unwrap(), "id");
    let top = id(&run(&mut p, "assembly.instance.create", json!({ "of": pair, "name": "Pair 1" })).unwrap(), "id");
    let req = id(&run(&mut p, "intent.requirement.create", json!({ "name": "No clashes", "rule": { "kind": "no_collisions", "parts": [top] } })).unwrap(), "id");
    let r: RequirementResult = p.get::<RequirementResult>(req).unwrap().clone();
    assert!(!r.passed, "{}", r.summary);
    assert_eq!(r.violations[0].names, ["Pair 1 › A › Slab", "Pair 1 › B › Slab"]);

    // Move B clear (it now spans x 2500..3500 → beyond A's 4000? no: lift it onto A instead).
    run(&mut p, "assembly.instance.update", json!({ "target": b, "at": [2500, -1000, 200] })).unwrap();
    let r: RequirementResult = p.get::<RequirementResult>(req).unwrap().clone();
    assert!(r.passed, "stacked, touching only: {}", r.summary);
}

#[test]
fn collisions_are_checked_in_each_instance_s_configuration() {
    let mut p = project();
    // A pin whose definition shows Short (10 long); the Long size reaches up into the block.
    let pin = id(&run(&mut p, "shape.create", json!({
        "name": "Pin", "source": "return box(5, 5, p.len);",
        "params": [{ "key": "len", "label": "Length", "value": 10 }], "definition": true, "display_tolerance": 0.1
    })).unwrap(), "id");
    run(&mut p, "shape.config.set_table", json!({
        "target": pin, "params": ["len"],
        "rows": [{ "name": "Short", "values": { "len": 10 } }, { "name": "Long", "values": { "len": 40 } }]
    })).unwrap();
    let block = id(&run(&mut p, "shape.create", json!({
        "name": "Block", "source": "return box(20, 20, 10);", "params": [], "definition": true, "display_tolerance": 0.1
    })).unwrap(), "id");
    let kit = id(&run(&mut p, "assembly.create", json!({ "name": "Kit" })).unwrap(), "id");
    let pinned = id(&run(&mut p, "assembly.instance.create", json!({ "of": pin, "parent": kit, "name": "P", "at": [5, 5, 0], "config": "Long" })).unwrap(), "id");
    run(&mut p, "assembly.instance.create", json!({ "of": block, "parent": kit, "name": "B", "at": [0, 0, 20] })).unwrap();
    let top = id(&run(&mut p, "assembly.instance.create", json!({ "of": kit, "name": "Kit 1" })).unwrap(), "id");
    let req = id(&run(&mut p, "intent.requirement.create", json!({ "name": "No clashes", "rule": { "kind": "no_collisions", "parts": [top] } })).unwrap(), "id");
    let r: RequirementResult = p.get::<RequirementResult>(req).unwrap().clone();
    assert!(!r.passed, "the Long pin reaches into the block: {}", r.summary);

    run(&mut p, "assembly.instance.update", json!({ "target": pinned, "config": "Short" })).unwrap();
    let r: RequirementResult = p.get::<RequirementResult>(req).unwrap().clone();
    assert!(r.passed, "the Short pin stops below it: {}", r.summary);
}
