//! Full 3D placement and joints between instances.

use de_assembly::joints::Placed;
use de_assembly::{AssemblyPlugin, AssemblyStats, Instance, Transform};
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

fn registry() -> Arc<Registry> {
    Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin]))
}

fn project() -> Project {
    Project::new("p", registry())
}

fn component(p: &mut Project, name: &str, source: &str) -> StableId {
    id(&run(p, "shape.create", json!({ "name": name, "source": source, "params": [], "definition": true })).unwrap())
}

fn place(p: &mut Project, of: StableId, parent: Option<StableId>, name: &str, at: [f64; 3], rotation: [f64; 3]) -> StableId {
    id(&run(p, "assembly.instance.create", json!({ "of": of, "parent": parent, "name": name, "at": at, "rotation": rotation })).unwrap())
}

fn placed(p: &Project, id: StableId) -> Transform {
    assert_eq!(p.status(id), Some(&Status::Current), "{:?}", p.status(id));
    p.get::<Placed>(id).expect("placed").transform()
}

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < tol)
}

/// 100 × 10 × 10 link with a pivot under its near end and a tip on top of its far end.
const LINK: &str = r#"
connector("Bottom", { at: [5, 5, 0], axis: [0, 0, -1] });
connector("Tip", { at: [95, 5, 10], axis: [0, 0, 1] });
return box(100, 10, 10);
"#;

#[test]
fn instances_turn_any_way_and_nest() {
    let mut p = project();
    let brick = component(&mut p, "Brick", "return box(40, 20, 10);");
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Stack" })).unwrap());
    // On its side: 90° about X stands the 20 mm depth up along Z.
    let inner = place(&mut p, brick, Some(asm), "Brick", [0.0, 0.0, 0.0], [90.0, 0.0, 0.0]);
    let outer = place(&mut p, asm, None, "Stack 1", [100.0, 0.0, 0.0], [0.0, 0.0, 90.0]);
    let s = p.get::<AssemblyStats>(asm).unwrap();
    assert!(close(s.bounds_min, [0.0, -10.0, 0.0], 1e-9) && close(s.bounds_max, [40.0, 0.0, 20.0], 1e-9), "{s:?}");
    // The instance of the assembly turns that a quarter about Z and moves it 100 along X.
    let s = p.get::<AssemblyStats>(outer).unwrap();
    assert!(close(s.bounds_min, [100.0, 0.0, 0.0], 1e-9) && close(s.bounds_max, [110.0, 40.0, 20.0], 1e-9), "{s:?}");
    let occ = de_assembly::occurrences(p.world(), outer);
    let t = occ[0].transform;
    let expect = Transform::from_euler([100.0, 0.0, 0.0], [0.0, 0.0, 90.0]).then(&Transform::from_euler([0.0; 3], [90.0, 0.0, 0.0]));
    assert!(close(t.apply([1.0, 2.0, 3.0]), expect.apply([1.0, 2.0, 3.0]), 1e-12));
    assert_eq!(p.get::<Instance>(inner).unwrap().rotation, [90.0, 0.0, 0.0]);
}

#[test]
fn a_project_saved_with_rotation_about_z_still_loads() {
    let mut p = project();
    let brick = component(&mut p, "Brick", "return box(40, 20, 10);");
    let inst = place(&mut p, brick, None, "Brick 1", [10.0, 0.0, 0.0], [0.0, 0.0, 90.0]);
    // Write the journal the way projects stored instances before full 3D placement.
    let mut text = serde_json::to_string(p.log()).unwrap();
    assert!(text.contains(r#""rotation":[0.0,0.0,90.0]"#));
    text = text.replace(r#""rotation":[0.0,0.0,90.0]"#, r#""rotation":90.0"#);
    let wires: Vec<de_core::WireChangeSet> = serde_json::from_str(&text).unwrap();
    let old = Project::rebuild("p".into(), registry(), "e", wires).unwrap();
    assert_eq!(old.get::<Instance>(inst).unwrap().rotation, [0.0, 0.0, 90.0]);
    let s = old.get::<AssemblyStats>(inst).unwrap();
    assert!(close(s.bounds_min, [-10.0, 0.0, 0.0], 1e-9) && close(s.bounds_max, [10.0, 40.0, 10.0], 1e-9), "{s:?}");
}

#[test]
fn a_hinge_chain_of_two_links_poses_as_computed_by_hand() {
    let mut p = project();
    let link = component(&mut p, "Link", LINK);
    let l1 = place(&mut p, link, None, "Upper arm", [0.0; 3], [0.0; 3]);
    let l2 = place(&mut p, link, None, "Forearm", [300.0, 0.0, 0.0], [0.0; 3]);
    let shoulder = id(&run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Shoulder", "a": "ground", "b": { "instance": l1, "connector": "Bottom" }, "value": 30 })).unwrap());
    let elbow = id(&run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Elbow", "a": { "instance": l1, "connector": "Tip" }, "b": { "instance": l2, "connector": "Bottom" }, "value": 45, "limits": [-120, 120] })).unwrap());
    // By hand: the upper arm turns 30° about its pivot (at the origin); the
    // forearm stands on its tip and turns 45° more: its own tip is at
    // Rz(75°)(90, 0, 10) + Rz(30°)(90, 0, 10).
    let (c75, s75) = 75f64.to_radians().sin_cos();
    let (c30, s30) = 30f64.to_radians().sin_cos();
    let (s75, c75, s30, c30) = (c75, s75, c30, s30);
    let tip = placed(&p, l2).apply([95.0, 5.0, 10.0]);
    let want = [90.0 * c75 + 90.0 * c30, 90.0 * s75 + 90.0 * s30, 20.0];
    assert!(close(tip, want, 1e-9), "{tip:?} vs {want:?}");
    assert!(close(placed(&p, l1).apply([5.0, 5.0, 0.0]), [0.0; 3], 1e-12), "the pivot stays on the ground's origin");
    assert_eq!(p.get::<Placed>(l2).unwrap().by, Some(elbow));
    // Values are parameters: scrub one, and formulas follow.
    run(&mut p, "parameters.set", json!({ "target": shoulder, "values": { "angle": 0 } })).unwrap();
    run(&mut p, "shape.link_param", json!({ "target": elbow, "key": "angle", "expr": "=Shoulder.angle + 90" })).unwrap();
    let tip = placed(&p, l2).apply([95.0, 5.0, 10.0]);
    assert!(close(tip, [90.0, 90.0, 20.0], 1e-9), "{tip:?}");
    // Limits hold: driven past one, the joint says so and what it holds waits.
    run(&mut p, "parameters.set", json!({ "target": shoulder, "values": { "angle": 40 } })).unwrap();
    assert!(matches!(p.status(elbow), Some(Status::Failed(m)) if m.contains("outside -120–120")), "{:?}", p.status(elbow));
    assert!(matches!(p.status(l2), Some(Status::Blocked(_))), "{:?}", p.status(l2));
    run(&mut p, "parameters.set", json!({ "target": shoulder, "values": { "angle": 0 } })).unwrap();
    let err = run(&mut p, "parameters.set", json!({ "target": elbow, "values": { "angle": 10 } })).unwrap_err();
    assert!(err.message.contains("formula"), "a linked value is not set by hand: {}", err.message);
    // A held instance cannot be moved by hand, and says why.
    let err = run(&mut p, "assembly.instance.update", json!({ "target": l2, "at": [0, 0, 50] })).unwrap_err();
    assert!(err.message.contains("Forearm is held by the hinge joint “Elbow” to Upper arm"), "{}", err.message);
    // Deleting a joint leaves what it held where it was, free.
    let before = placed(&p, l2);
    run(&mut p, "assembly.joint.delete", json!({ "target": elbow })).unwrap();
    assert!(close(placed(&p, l2).apply([1.0, 2.0, 3.0]), before.apply([1.0, 2.0, 3.0]), 1e-6));
    assert_eq!(p.get::<Placed>(l2).unwrap().by, None);
    run(&mut p, "assembly.instance.update", json!({ "target": l2, "at": [0, 0, 50] })).unwrap();
}

#[test]
fn a_fixed_mate_seats_a_part_exactly() {
    let mut p = project();
    let bracket = component(&mut p, "Bracket", r#"connector("Mount", { at: [20, 20, 5], axis: [0, 0, 1] }); return box(40, 40, 5);"#);
    let servo = component(&mut p, "Servo", r#"connector("Base", { at: [10, 5, 0], axis: [0, 0, -1] }); return box(20, 10, 30);"#);
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Pan unit" })).unwrap());
    let b = place(&mut p, bracket, Some(asm), "Bracket", [100.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    let s = place(&mut p, servo, Some(asm), "Servo", [0.0, 300.0, 0.0], [0.0; 3]);
    run(&mut p, "assembly.joint.create", json!({ "assembly": asm, "type": "fixed", "a": { "instance": b, "connector": "Mount" }, "b": { "instance": s, "connector": "Base" } })).unwrap();
    let t = placed(&p, s);
    // Centred on the bracket's top: x 110–130, y 15–25, z 5–35.
    assert!(close(t.apply([0.0; 3]), [110.0, 15.0, 5.0], 1e-12) && close(t.apply([20.0, 10.0, 30.0]), [130.0, 25.0, 35.0], 1e-12), "{t:?}");
    let st = p.get::<AssemblyStats>(asm).unwrap();
    assert!(close(st.bounds_min, [100.0, 0.0, 0.0], 1e-9) && close(st.bounds_max, [140.0, 40.0, 35.0], 1e-9), "{st:?}");
    // Tilt the bracket on its side (the anchor moves freely): the servo goes with it.
    run(&mut p, "assembly.instance.update", json!({ "target": b, "rotation": [90, 0, 0] })).unwrap();
    let (tb, ts) = (placed(&p, b), placed(&p, s));
    assert!(close(ts.apply([10.0, 5.0, 0.0]), tb.apply([20.0, 20.0, 5.0]), 1e-12));
    assert!(close(ts.turn([0.0, 0.0, -1.0]), tb.turn([0.0, 0.0, -1.0]), 1e-12), "the servo's base faces the bracket's top");
    // With a gap of 2 mm.
    let j = de_assembly::joints::joints_in(p.world(), Some(asm))[0].0;
    run(&mut p, "assembly.joint.update", json!({ "target": j, "offset": 2 })).unwrap();
    let (tb, ts) = (placed(&p, b), placed(&p, s));
    assert!(close(ts.apply([10.0, 5.0, 0.0]), tb.apply([20.0, 20.0, 7.0]), 1e-12));
    // Making a unique copy of the assembly copies the joint, joining the copies.
    let top = place(&mut p, asm, None, "Pan 1", [0.0; 3], [0.0; 3]);
    let out = run(&mut p, "assembly.instance.make_unique", json!({ "target": top })).unwrap();
    let copy: StableId = serde_json::from_value(out["definition"].clone()).unwrap();
    assert_eq!(de_assembly::joints::joints_in(p.world(), Some(copy)).len(), 1);
    let kids = de_assembly::children(p.world(), copy);
    let sc = kids.iter().find(|(_, i)| i.name == "Servo").unwrap().0;
    assert!(p.get::<Placed>(sc).unwrap().by.is_some());
    // Deleting the bracket takes its joint; the servo stays put, free.
    run(&mut p, "assembly.instance.delete", json!({ "target": b })).unwrap();
    assert_eq!(p.status(s), Some(&Status::Current));
    assert!(close(placed(&p, s).apply([10.0, 5.0, 0.0]), tb.apply([20.0, 20.0, 7.0]), 1e-9));
}

#[test]
fn a_missing_connector_is_refused_with_the_ones_there_are() {
    let mut p = project();
    let link = component(&mut p, "Link", LINK);
    let l1 = place(&mut p, link, None, "A", [0.0; 3], [0.0; 3]);
    let err = run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "a": "ground", "b": { "instance": l1, "connector": "Shaft" } })).unwrap_err();
    assert!(err.message.contains("no connector “Shaft” (it has Bottom, Tip)"), "{}", err.message);
}

/// An arm hinged on the ground sweeps into a post standing in its way.
#[test]
fn the_clash_sweep_finds_the_post_where_the_arm_hits_it() {
    let mut p = project();
    let arm = component(&mut p, "Arm", r#"connector("Pivot", { at: [0, 0, 0], axis: [0, 0, -1] }); return box(85, 10, 10).translate([-5, -5, 0]);"#);
    let post = component(&mut p, "Post", "return box(10, 10, 10);");
    let a = place(&mut p, arm, None, "Arm", [0.0; 3], [0.0; 3]);
    place(&mut p, post, None, "Post", [-5.0, 40.0, 0.0], [0.0; 3]);
    let j = id(&run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Swing", "a": "ground", "b": { "instance": a, "connector": "Pivot" } })).unwrap());
    let s = de_assembly::motion::sweep(p.world(), j, None, Some(0.0), Some(180.0), 180, &Default::default(), true).unwrap();
    assert_eq!(s.frames.len(), 181);
    // The arm's leading face (5 mm off its centre line) meets the post's near
    // corner (5, 40) where 40 cos θ − 5 sin θ = 5: θ = 75.75°; its trailing
    // face leaves the corner (−5, 40) where 40 cos θ + 5 sin θ = −5: θ = 104.25°.
    let c = s.clashes.first().expect("a clash");
    assert_eq!((c.a.as_str(), c.b.as_str()), ("Arm", "Post"));
    let [first, last] = c.range.unwrap();
    assert_eq!((first, last), (76.0, 104.0), "{c:?}");
    assert!(s.instances.iter().any(|i| i.name == "Arm" && i.moves) && s.instances.iter().any(|i| i.name == "Post" && !i.moves));
    // Through 0–70° it clears; the closest gap is at 70°.
    let s = de_assembly::motion::sweep(p.world(), j, None, Some(0.0), Some(70.0), 70, &Default::default(), true).unwrap();
    assert!(s.clashes.is_empty());
    let g = &s.pairs[0];
    let want = 40.0 * 70f64.to_radians().cos() - 5.0 * 70f64.to_radians().sin() - 5.0;
    assert!((g.gap - want).abs() < 0.05 && g.at == 70.0, "{g:?} vs {want}");
}

/// A parallelogram four-bar made of separate links: the crank is driven, the
/// other joints are free, and the coupler stays level.
#[test]
fn a_closed_loop_of_instances_solves() {
    let mut p = project();
    let link = component(&mut p, "Link", LINK);
    let crank = place(&mut p, link, None, "Crank", [0.0; 3], [0.0, 0.0, 90.0]);
    let rocker = place(&mut p, link, None, "Rocker", [200.0, 0.0, 0.0], [0.0, 0.0, 90.0]);
    let coupler = place(&mut p, link, None, "Coupler", [0.0, 90.0, 10.0], [0.0; 3]);
    // Crank and rocker pivot on the ground 200 apart; the coupler spans their tips.
    let drive = id(&run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Drive", "a": "ground", "b": { "instance": crank, "connector": "Bottom" }, "value": 90 })).unwrap());
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Rocker pivot", "a": { "at": [200, 0, 0] }, "b": { "instance": rocker, "connector": "Bottom" }, "value": 90, "free": true })).unwrap();
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Crank pin", "a": { "instance": crank, "connector": "Tip" }, "b": { "instance": coupler, "connector": "Bottom" }, "value": -90, "free": true })).unwrap();
    // The coupler's far end on the rocker's tip; this is the joint that closes the loop.
    let long = component(&mut p, "Coupler bar", r#"connector("Bottom", { at: [5, 5, 0], axis: [0, 0, -1] }); connector("Far", { at: [205, 5, 0], axis: [0, 0, -1] }); return box(210, 10, 10);"#);
    run(&mut p, "assembly.instance.update", json!({ "target": coupler, "of": long })).unwrap();
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Rocker pin", "a": { "instance": rocker, "connector": "Tip" }, "b": { "instance": coupler, "connector": "Far" }, "free": true })).unwrap();
    for angle in [90.0, 60.0, 120.0] {
        run(&mut p, "parameters.set", json!({ "target": drive, "values": { "angle": angle } })).unwrap();
        let c = placed(&p, coupler);
        let (s, co) = (angle as f64).to_radians().sin_cos();
        // Parallelogram: the coupler's near pin is on the crank tip, and it stays parallel to the ground line.
        assert!(close(c.apply([5.0, 5.0, 0.0]), [90.0 * co, 90.0 * s, 10.0], 1e-6), "{angle}: {:?}", c.apply([5.0, 5.0, 0.0]));
        assert!(close(c.turn([1.0, 0.0, 0.0]), [1.0, 0.0, 0.0], 1e-6), "{angle}: the coupler stays level");
        let r = placed(&p, rocker);
        assert!(close(r.apply([95.0, 5.0, 10.0]), [200.0 + 90.0 * co, 90.0 * s, 10.0], 1e-6));
    }
}
