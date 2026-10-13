//! Instances' own parameter values (live, by formula), sweeps that keep their
//! measurements by what they depend on, and joint-space interlocks.

use de_assembly::{AssemblyPlugin, Instance, InstanceConfigured};
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Status, Value, json};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::collections::HashMap;
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

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin])))
}

/// The volume an instance shows (its own values or configuration), else its definition's.
fn shown(p: &Project, inst: StableId) -> f64 {
    match p.get::<InstanceConfigured>(inst).and_then(|c| c.0.clone()) {
        Some(c) => c.stats.volume_mm3,
        None => p.get::<de_shape::ShapeSolid>(p.get::<Instance>(inst).unwrap().of).unwrap().stats.volume_mm3,
    }
}

/// A d × d × len block.
fn bolt(p: &mut Project) -> StableId {
    id(&run(
        p,
        "shape.create",
        json!({
            "name": "Bolt",
            "source": "return box(p.d, p.d, p.len);",
            "params": [
                { "key": "d", "label": "Diameter", "value": 6, "unit": "mm", "min": 1 },
                { "key": "len", "label": "Length", "value": 20, "unit": "mm", "min": 1, "max": 100 }
            ],
            "definition": true
        }),
    )
    .unwrap())
}

#[test]
fn an_instance_gives_its_shape_values_of_its_own_and_still_shares_it() {
    let mut p = project();
    let b = bolt(&mut p);
    let one = id(&run(&mut p, "assembly.instance.create", json!({ "of": b, "name": "One", "at": [0, 0, 0] })).unwrap());
    let two = id(&run(&mut p, "assembly.instance.create", json!({ "of": b, "name": "Two", "at": [50, 0, 0], "params": { "len": 40 } })).unwrap());
    assert!((shown(&p, one) - 720.0).abs() < 1e-6);
    assert!((shown(&p, two) - 6.0 * 6.0 * 40.0).abs() < 1e-6, "its own length");
    // An edit to the definition reaches both; the own value stays.
    run(&mut p, "shape.set_params", json!({ "target": b, "values": { "d": 8 } })).unwrap();
    assert!((shown(&p, one) - 8.0 * 8.0 * 20.0).abs() < 1e-6);
    assert!((shown(&p, two) - 8.0 * 8.0 * 40.0).abs() < 1e-6);
    // A formula reading the definition's own parameter (a bare key), and its bounds.
    run(&mut p, "assembly.instance.update", json!({ "target": two, "params": { "len": "=d * 3" } })).unwrap();
    assert!((shown(&p, two) - 8.0 * 8.0 * 24.0).abs() < 1e-6);
    let err = run(&mut p, "assembly.instance.update", json!({ "target": two, "params": { "len": 500 } })).unwrap_err();
    assert!(err.message.contains("outside"), "{}", err.message);
    let err = run(&mut p, "assembly.instance.update", json!({ "target": two, "params": { "width": 5 } })).unwrap_err();
    assert!(err.message.contains("no parameter “width”"), "{}", err.message);
    // null drops it: the definition's value shows again.
    run(&mut p, "assembly.instance.update", json!({ "target": two, "params": { "len": null } })).unwrap();
    assert!(p.get::<Instance>(two).unwrap().params.is_empty());
    assert!((shown(&p, two) - 8.0 * 8.0 * 20.0).abs() < 1e-6);
}

/// An arm hinged on the ground with a dial on it whose own `turn` follows
/// the hinge, and a post the arm reaches between 76° and 104° (see joints.rs).
fn arm(p: &mut Project) -> (StableId, StableId, StableId) {
    let arm = id(&run(p, "shape.create", json!({ "name": "Arm", "source": r#"connector("Pivot", { at: [0, 0, 0], axis: [0, 0, -1] }); connector("Dial", { at: [60, 0, 10], axis: [0, 0, 1] }); return box(85, 10, 10).translate([-5, -5, 0]);"#, "params": [], "definition": true })).unwrap());
    let post = id(&run(p, "shape.create", json!({ "name": "Post", "source": "return box(10, 10, 10);", "params": [], "definition": true })).unwrap());
    // A pointer whose length shows its `turn` (so its volume tells the value apart).
    let dial = id(&run(p, "shape.create", json!({ "name": "Dial", "source": r#"connector("Base", { at: [0, 0, 0], axis: [0, 0, -1] }); return box(2 + p.turn / 10, 2, 2);"#, "params": [{ "key": "turn", "label": "Turn", "value": 0, "unit": "deg", "min": -360, "max": 360 }], "definition": true })).unwrap());
    let a = id(&run(p, "assembly.instance.create", json!({ "of": arm, "name": "Arm", "at": [0, 0, 0] })).unwrap());
    id(&run(p, "assembly.instance.create", json!({ "of": post, "name": "Post", "at": [-5, 40, 0] })).unwrap());
    let d = id(&run(p, "assembly.instance.create", json!({ "of": dial, "name": "Dial", "at": [0, 0, 0] })).unwrap());
    let j = id(&run(p, "assembly.joint.create", json!({ "type": "hinge", "name": "Swing", "a": "ground", "b": { "instance": a, "connector": "Pivot" }, "limits": [0, 180] })).unwrap());
    run(p, "assembly.joint.create", json!({ "type": "fixed", "a": { "instance": a, "connector": "Dial" }, "b": { "instance": d, "connector": "Base" } })).unwrap();
    (j, a, d)
}

#[test]
fn an_own_value_follows_its_joint_live_and_through_a_sweep() {
    let mut p = project();
    let (j, _, d) = arm(&mut p);
    run(&mut p, "assembly.instance.update", json!({ "target": d, "params": { "turn": "=Swing.angle" } })).unwrap();
    assert_eq!(p.status(d), Some(&Status::Current), "{:?}", p.status(d));
    let vol = |p: &Project| shown(p, d);
    assert!((vol(&p) - 2.0 * 2.0 * 2.0).abs() < 1e-6, "at 0°: 2 mm long");
    run(&mut p, "parameters.set", json!({ "target": j, "values": { "angle": 50 } })).unwrap();
    assert!((vol(&p) - 7.0 * 2.0 * 2.0).abs() < 1e-6, "at 50°: 7 mm long, {}", vol(&p));
    // Joints changing around it keep its link to the joint (the hub rewires the instances).
    run(&mut p, "assembly.joint.update", json!({ "target": j, "limits": [0, 170] })).unwrap();
    run(&mut p, "parameters.set", json!({ "target": j, "values": { "angle": 30 } })).unwrap();
    assert!((vol(&p) - 5.0 * 2.0 * 2.0).abs() < 1e-6, "at 30°: 5 mm long, {}", vol(&p));
    // A sweep lists it with its value at each frame (for players), and evaluates it at each step.
    let s = de_assembly::motion::sweep(p.world(), j, None, Some(0.0), Some(90.0), 9, &HashMap::new(), true).unwrap();
    let f = s.flex.iter().find(|f| f.instance == d).expect("the dial flexes");
    assert_eq!(f.values.len(), s.frames.len());
    assert_eq!(f.values[3].get("turn"), Some(&30.0), "{:?}", f.values[3]);
    assert!(s.notes.iter().any(|n| n.contains("Dial follows the joints")), "{:?}", s.notes);
}

#[test]
fn sweeps_measure_again_only_the_pairs_a_change_moves() {
    let mut p = project();
    let (j, a, _) = arm(&mut p);
    let s1 = de_assembly::motion::sweep(p.world(), j, None, Some(0.0), Some(180.0), 36, &HashMap::new(), true).unwrap();
    let (hits0, measured0) = de_assembly::motion::pair_counts();
    // Renamed and labelled: the same measurement, read back pair by pair.
    run(&mut p, "assembly.instance.update", json!({ "target": a, "name": "Long arm", "properties": { "note": "checked" } })).unwrap();
    let s2 = de_assembly::motion::sweep(p.world(), j, None, Some(0.0), Some(180.0), 36, &HashMap::new(), true).unwrap();
    let (hits1, measured1) = de_assembly::motion::pair_counts();
    assert!(hits1 > hits0, "pairs read back");
    assert_eq!(measured1 - measured0, 0, "nothing measured again");
    assert_eq!(s1.clashes.len(), s2.clashes.len());
    assert_eq!(s2.clashes[0].a, "Long arm", "named as it is now");
    assert_eq!(s1.clashes[0].range, s2.clashes[0].range);
}

#[test]
fn an_interlock_narrows_sweeps_refuses_poses_and_writes_the_controller_config() {
    let mut p = project();
    let (j, a, _) = arm(&mut p);
    // A second joint the interlock reads: a gate sliding beside the post.
    let gate = id(&run(&mut p, "shape.create", json!({ "name": "Gate", "source": r#"connector("Rail", { at: [0, 0, 0], axis: [0, 0, -1] }); return box(4, 4, 4);"#, "params": [], "definition": true })).unwrap());
    let g = id(&run(&mut p, "assembly.instance.create", json!({ "of": gate, "name": "Gate", "at": [200, 0, 0] })).unwrap());
    let slide = id(&run(&mut p, "assembly.joint.create", json!({ "type": "slider", "name": "Gate slide", "a": { "at": [200, 0, 0], "axis": [1, 0, 0] }, "b": { "instance": g, "connector": "Rail" }, "limits": [0, 50] })).unwrap());
    // Without an interlock the swing hits the post.
    let free = de_assembly::motion::sweep(p.world(), j, None, None, None, 36, &HashMap::new(), true).unwrap();
    assert!(!free.clashes.is_empty() && free.allowed.is_empty());
    // While the gate is shut (below 10 mm), the swing stops at 70°.
    let il = id(&run(&mut p, "assembly.interlock.create", json!({ "name": "Post guard", "zones": [{ "when": { "Gate slide": { "max": 10 } }, "then": { "Swing": { "max": 70 } } }] })).unwrap());
    let held = de_assembly::motion::sweep(p.world(), j, None, None, None, 36, &HashMap::new(), true).unwrap();
    assert!(held.clashes.is_empty(), "{:?}", held.clashes);
    assert_eq!(held.allowed, vec![[0.0, 70.0]]);
    assert_eq!(held.frames.last().unwrap().value, 70.0, "swept exactly to the boundary");
    assert!(held.notes[0].contains("Interlocks keep Swing within 0°…70°"), "{}", held.notes[0]);
    // With the gate open the whole range is allowed (and hits the post).
    let open = de_assembly::motion::sweep(p.world(), j, None, None, None, 36, &HashMap::from([(slide, vec![30.0])]), true).unwrap();
    assert!(open.allowed.is_empty() && !open.clashes.is_empty());
    // Setting the arm into the forbidden region is refused, with the reason.
    let err = run(&mut p, "parameters.set", json!({ "target": j, "values": { "angle": 90 } })).unwrap_err();
    assert!(err.message.contains("Post guard") && err.message.contains("Swing 90°"), "{}", err.message);
    run(&mut p, "parameters.set", json!({ "target": j, "values": { "angle": 60 } })).unwrap();
    // An interlock forbidding where the arm stands now is refused too.
    let err = run(&mut p, "assembly.interlock.create", json!({ "name": "Too tight", "zones": [{ "when": { "Gate slide": { "max": 10 } }, "then": { "Swing": { "max": 20 } } }] })).unwrap_err();
    assert!(err.message.contains("move the joints out of it first"), "{}", err.message);
    // A limit table: the swing's top follows the gate, linearly.
    run(&mut p, "assembly.interlock.update", json!({ "target": il, "zones": [], "tables": [{ "by": "Gate slide", "joint": "Swing", "rows": [[0, 0, 70], [10, 0, 70], [30, 0, 180]] }] })).unwrap();
    let mid = de_assembly::motion::sweep(p.world(), j, None, None, None, 36, &HashMap::from([(slide, vec![20.0])]), true).unwrap();
    assert_eq!(mid.allowed, vec![[0.0, 125.0]], "half way between 70 and 180");
    // The controller's configuration and its C header.
    let rig = de_assembly::robot::Rig::new(p.world(), a, None, None).unwrap();
    let cfg = de_assembly::robot::controller(p.world(), &rig).unwrap();
    assert_eq!(cfg.joints.iter().map(|j| j.symbol.as_str()).collect::<Vec<_>>(), ["J_SWING", "J_GATE_SLIDE"]);
    assert_eq!(cfg.interlocks[0].tables[0].rows.len(), 3);
    let h = de_assembly::robot::arduino_header(&cfg);
    assert!(h.contains("#define ARM_JOINTS 2") && h.contains("arm_table_at(ARM_TABLE_0, 3, q[J_GATE_SLIDE]") && h.contains("static inline void arm_clamp"), "{h}");
    // IK and the reach keep out of what it forbids.
    assert!(rig.allows(&[60.0]) && !rig.allows(&[120.0]));
    // Deleting the gate's joint drops the rules that name it, and the interlock with them.
    run(&mut p, "assembly.joint.delete", json!({ "target": slide })).unwrap();
    assert!(p.get::<de_assembly::interlocks::Interlock>(il).is_none());
}
