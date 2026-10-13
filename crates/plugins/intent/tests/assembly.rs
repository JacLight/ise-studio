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

/// Two storeys, each a 4 × 3 m slab 200 thick with a bottom mesh of Ø12 bars
/// (40 cover) cast in it; the ground floor's X bars run to `end` (the slab
/// stops at 4000). `embedded` declares the bars embedded in their slab.
fn storeys(end: f64, embedded: bool) -> (Project, StableId, StableId) {
    let mut p = project();
    let source = format!(
        r#"const parts = [];
        for (const [n, lv] of [["Ground slab", level("Ground", 0)], ["First slab", level("First", 3000)]]) {{
          const z = lv.elevation - 200 + 40;
          const host = {embedded} ? n : undefined;
          parts.push(...slab(n, {{ outline: [[0, 0], [4000, 0], [4000, 3000], [0, 3000]], thickness: 200, level: lv }}));
          const x1 = n === "Ground slab" ? {end} : 3960;
          parts.push(...rebar(`${{n}} X`, {{ path: [[40, 40, z], [x1, 40, z]], diameter: 12, count: 15, spacing: 200, direction: [0, 1, 0], level: lv, embedded: host }}));
          parts.push(...rebar(`${{n}} Y`, {{ path: [[40, 40, z + 12], [40, 2960, z + 12]], diameter: 12, count: 20, spacing: 200, direction: [1, 0, 0], level: lv, embedded: host }}));
        }}
        return parts;"#
    );
    let concrete = id(&run(&mut p, "solid.material.create", json!({ "name": "Concrete", "density": { "value": 2400, "unit": "kg/m3" } })).unwrap(), "id");
    let steel = id(&run(&mut p, "solid.material.create", json!({ "name": "Steel", "density": { "value": 7850, "unit": "kg/m3" } })).unwrap(), "id");
    let building = id(&run(&mut p, "shape.create", json!({ "name": "Building", "source": source, "params": [], "display_tolerance": 1, "materials": { "Concrete": concrete, "Steel": steel } })).unwrap(), "id");
    let req = id(&run(&mut p, "intent.requirement.create", json!({ "name": "Nothing collides", "rule": { "kind": "no_collisions", "parts": [building] } })).unwrap(), "id");
    (p, building, req)
}

#[test]
fn rebar_cast_in_its_slab_is_no_collision_unless_it_pokes_out() {
    // Not declared: every mesh overlaps its slab.
    let (p, _, req) = storeys(3960.0, false);
    let r: RequirementResult = p.get::<RequirementResult>(req).unwrap().clone();
    assert!(!r.passed);
    assert_eq!(r.violations.len(), 4, "{}", r.summary);
    let i = de_assembly::interference::report(p.world(), None, 0.0).unwrap();
    assert_eq!(i.overlaps.iter().filter(|c| !c.embedded && !c.fastener && !c.joined).count(), 4);

    // Declared embedded in their slabs: meant.
    let (p, _, req) = storeys(3960.0, true);
    let r: RequirementResult = p.get::<RequirementResult>(req).unwrap().clone();
    assert!(r.passed, "{}: {:?}", r.summary, r.violations);
    assert!(r.summary.contains("4 embedded in their hosts"), "{}", r.summary);
    let i = de_assembly::interference::report(p.world(), None, 0.0).unwrap();
    assert_eq!(i.overlaps.len(), 4, "{:#?}", i.overlaps);
    assert!(i.overlaps.iter().all(|c| c.embedded && c.outside == 0.0), "{:#?}", i.overlaps);
    assert!(i.notes[0].contains("0 not meant"), "{:?}", i.notes);

    // The ground floor's X bars run 100 mm past the slab's edge: no cover, they poke out.
    let (p, _, req) = storeys(4100.0, true);
    let r: RequirementResult = p.get::<RequirementResult>(req).unwrap().clone();
    assert!(!r.passed);
    assert_eq!(r.violations.len(), 1, "{:?}", r.violations);
    assert_eq!(r.violations[0].names, ["Building › Ground slab X", "Building › Ground slab"]);
    assert_eq!(r.summary, "1 embedded part pokes out of its host");
    // Where: from the slab's edge at x = 4000 to the bars' ends at 4100.
    let region = r.violations[0].region.unwrap();
    assert!((region[0] - 4000.0).abs() < 1.0 && (region[3] - 4100.0).abs() < 1.0, "{region:?}");
    let i = de_assembly::interference::report(p.world(), None, 0.0).unwrap();
    let bad: Vec<_> = i.overlaps.iter().filter(|c| !c.embedded).collect();
    assert_eq!(bad.len(), 1, "{:#?}", i.overlaps);
    // 15 bars × 100 mm of octagonal Ø12 (2√2·6² mm²) out of the slab.
    let out = 15.0 * 100.0 * 2.0 * 2f64.sqrt() * 36.0;
    assert!((bad[0].outside - out).abs() < 0.05 * out, "{}", bad[0].outside);
    assert!(i.notes.iter().any(|n| n.contains("Ground slab X is embedded in Building › Ground slab but")), "{:?}", i.notes);
}
