//! Real library parts joined at their connectors.

use de_assembly::joints::Placed;
use de_assembly::{AssemblyPlugin, Transform};
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Status, Value, json};
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

fn id(v: &Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

fn placed(p: &Project, id: StableId) -> Transform {
    assert_eq!(p.status(id), Some(&Status::Current), "{:?}", p.status(id));
    p.get::<Placed>(id).expect("placed").transform()
}

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < tol)
}

/// Distance from point `p` to the line through `o` along unit `u`.
fn off_axis(p: [f64; 3], o: [f64; 3], u: [f64; 3]) -> f64 {
    let d = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
    let t = d[0] * u[0] + d[1] * u[1] + d[2] * u[2];
    ((d[0] - t * u[0]).powi(2) + (d[1] - t * u[1]).powi(2) + (d[2] - t * u[2]).powi(2)).sqrt()
}

/// An MG996R lying on its side drives a printed arm by its output shaft, and
/// an M3 × 10 cap screw is seated in the arm's hole by a fixed mate.
#[test]
fn a_servo_drives_an_arm_and_a_screw_seats_in_it() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin])));
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Arm unit" })).unwrap());
    let servo = id(&run(&mut p, "stock.insert", json!({ "family": "servo_hobby", "size": "MG996R", "parent": asm, "at": [0, 0, 0], "rotation": [90, 0, 0] })).unwrap());
    let screw = id(&run(&mut p, "stock.insert", json!({ "family": "iso4762", "size": "M3×10", "parent": asm, "at": [200, 0, 0] })).unwrap());
    let arm = id(&run(&mut p, "shape.create", json!({ "name": "Arm", "definition": true, "params": [], "source": r#"
connector("Bore", { at: [0, 0, 0], axis: [0, 0, -1] });
connector("Hole", { at: [40, 0, 4], axis: [0, 0, 1] });
return box(48, 10, 4).translate([-5, -5, 0]).subtract(cylinder(4, 1.7).translate([40, 0, 0]));"# })).unwrap());
    let a = id(&run(&mut p, "assembly.instance.create", json!({ "of": arm, "parent": asm, "name": "Arm", "at": [0, 100, 0] })).unwrap());
    let horn = id(&run(&mut p, "assembly.joint.create", json!({ "assembly": asm, "type": "hinge", "name": "Horn", "a": { "instance": servo, "connector": "Output shaft" }, "b": { "instance": a, "connector": "Bore" }, "limits": [-90, 90] })).unwrap());
    run(&mut p, "assembly.joint.create", json!({ "assembly": asm, "type": "fixed", "name": "Screw seat", "a": { "instance": a, "connector": "Hole" }, "b": { "instance": screw, "connector": "Head seat" } })).unwrap();
    let f = de_assembly::joints::connectors(p.world(), servo).unwrap().into_iter().find(|(n, _)| n == "Output shaft").unwrap().1;
    let shaft = placed(&p, servo).then(&f);
    // Lying on its side (90° about X) the shaft points along −Y; the arm's bore sits on its tip.
    assert!(close(shaft.turn([0.0, 0.0, 1.0]), [0.0, -1.0, 0.0], 1e-9));
    for angle in [0.0, 45.0, 90.0] {
        run(&mut p, "parameters.set", json!({ "target": horn, "values": { "angle": angle } })).unwrap();
        let ta = placed(&p, a);
        assert!(close(ta.apply([0.0; 3]), shaft.at, 1e-9), "{angle}: the bore is on the shaft");
        let hole = ta.apply([40.0, 0.0, 4.0]);
        assert!((off_axis(hole, shaft.at, shaft.turn([0.0, 0.0, 1.0])) - 40.0).abs() < 1e-9, "{angle}: the hole turns 40 mm round the shaft");
        // The screw's head sits on the hole and its tip is 10 mm down, 6 mm past the arm's 4 mm.
        let ts = placed(&p, screw);
        assert!(close(ts.apply([0.0; 3]), hole, 1e-9));
        assert!(close(ts.apply([0.0, 0.0, -10.0]), ta.apply([40.0, 0.0, -6.0]), 1e-9));
    }
    // Swept through its limits, the screw is joined to the arm, so its contact there is no clash.
    let sw = de_assembly::motion::sweep(p.world(), horn, None, None, None, 36, &Default::default(), true).unwrap();
    assert_eq!(sw.frames.len(), 37);
    assert!(sw.pairs.iter().any(|g| g.joined), "{:?}", sw.pairs);
    assert!(sw.clashes.iter().all(|c| !c.joined), "{:?}", sw.clashes);
}
