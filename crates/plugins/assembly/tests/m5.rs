//! Mates on picked geometry, degrees of freedom, couplings, patterns,
//! exploded views, motion studies, robot kinematics and interference.

use de_assembly::joints::Placed;
use de_assembly::{AssemblyPlugin, Transform};
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

fn component(p: &mut Project, name: &str, source: &str) -> StableId {
    id(&run(p, "shape.create", json!({ "name": name, "source": source, "params": [], "definition": true })).unwrap())
}

fn place(p: &mut Project, of: StableId, name: &str, at: [f64; 3], rotation: [f64; 3]) -> StableId {
    id(&run(p, "assembly.instance.create", json!({ "of": of, "name": name, "at": at, "rotation": rotation })).unwrap())
}

fn placed(p: &Project, id: StableId) -> Transform {
    assert_eq!(p.status(id), Some(&Status::Current), "{:?}", p.status(id));
    p.get::<Placed>(id).expect("placed").transform()
}

fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < tol)
}

const LINK: &str = r#"
connector("Bottom", { at: [5, 5, 0], axis: [0, 0, -1] });
connector("Tip", { at: [95, 5, 10], axis: [0, 0, 1] });
return box(100, 10, 10);
"#;

#[test]
fn mates_on_picked_faces_take_away_only_what_they_name() {
    let mut p = project();
    let base = component(&mut p, "Base", "return box(100, 100, 10);");
    let block = component(&mut p, "Block", "return box(20, 20, 10);");
    let b = place(&mut p, base, "Base", [0.0; 3], [0.0; 3]);
    let k = place(&mut p, block, "Block", [30.0, 40.0, 25.0], [4.0, -3.0, 20.0]);
    // The block's bottom face (picked at a point on it) on the base's top face.
    run(&mut p, "assembly.joint.create", json!({ "type": "coincident", "name": "On top",
        "a": { "instance": b, "geometry": { "type": "face", "at": [50, 50, 10], "normal": [0, 0, 1] } },
        "b": { "instance": k, "geometry": { "type": "face", "at": [10, 10, 0], "normal": [0, 0, -1] } } })).unwrap();
    let t = placed(&p, k);
    // Flat on the top: every bottom corner at z = 10, its own z up.
    for c in [[0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [0.0, 20.0, 0.0], [20.0, 20.0, 0.0]] {
        assert!((t.apply(c)[2] - 10.0).abs() < 1e-7, "{:?}", t.apply(c));
    }
    assert!(close(t.turn([0.0, 0.0, 1.0]), [0.0, 0.0, 1.0], 1e-9));
    // Still free to slide on the face and turn on it: 3 left.
    let d = de_assembly::dof::report(p.world(), None).unwrap();
    let g = &d.groups[0];
    assert!(g.solved);
    assert_eq!(g.free, 3, "{g:?}");
    let inst = g.instances.iter().find(|i| i.id == k).unwrap();
    assert_eq!(inst.free, 3);
    assert!(inst.motions.iter().any(|m| m.starts_with("turns about (0, 0, 1)")), "{:?}", inst.motions);
    assert_eq!(inst.motions.iter().filter(|m| m.starts_with("slides along")).count(), 2, "{:?}", inst.motions);
    // It can be moved by hand along what is left (a mate does not hold it outright).
    run(&mut p, "assembly.instance.update", json!({ "target": k, "at": [60, 10, 40], "rotation": [0, 0, 0] })).unwrap();
    let t = placed(&p, k);
    assert!(close(t.apply([0.0; 3]), [60.0, 10.0, 10.0], 1e-7), "{:?}", t.apply([0.0; 3]));
    // 15 mm in from the base's left side (a distance between parallel faces) and its front flush (coincident): fully defined.
    run(&mut p, "assembly.joint.create", json!({ "type": "distance", "name": "From the left", "value": 15,
        "a": { "instance": b, "geometry": { "type": "plane", "at": [0, 0, 0], "normal": [1, 0, 0] } },
        "b": { "instance": k, "geometry": { "type": "face", "at": [0, 10, 5], "normal": [-1, 0, 0] } } })).unwrap();
    run(&mut p, "assembly.joint.create", json!({ "type": "coincident", "name": "Front flush", "aligned": true,
        "a": { "instance": b, "geometry": { "type": "face", "at": [50, 0, 5], "normal": [0, -1, 0] } },
        "b": { "instance": k, "geometry": { "type": "face", "at": [10, 0, 5], "normal": [0, -1, 0] } } })).unwrap();
    let t = placed(&p, k);
    assert!(close(t.apply([0.0; 3]), [15.0, 0.0, 10.0], 1e-7), "{:?}", t.apply([0.0; 3]));
    let d = de_assembly::dof::report(p.world(), None).unwrap();
    assert_eq!(d.groups[0].free, 0, "{:?}", d.groups[0]);
    assert_eq!(d.groups[0].redundant, 0);
    // A distance the other mates contradict cannot hold: the joints that conflict are named.
    let dist = de_assembly::joints::joints_in(p.world(), None).iter().find(|(j, _)| de_assembly::joints::joint_name(p.world(), *j) == "From the left").unwrap().0;
    run(&mut p, "parameters.set", json!({ "target": dist, "values": { "distance": 30 } })).unwrap();
    assert!((placed(&p, k).apply([0.0; 3])[0] - 30.0).abs() < 1e-7);
    // Picking an edge works too: the block's front-bottom edge is a line.
    let e = de_assembly::geometry::resolve(p.world(), k, &de_assembly::geometry::Geometry::Edge { points: vec![[0.0, 0.0, 0.0], [20.0, 0.0, 0.0]] }).unwrap();
    assert!(matches!(e, de_assembly::geometry::Elem::Line { .. }));
    // A mate that cannot join what it names is refused with why.
    let err = run(&mut p, "assembly.joint.create", json!({ "type": "tangent",
        "a": { "instance": b, "geometry": { "type": "face", "at": [50, 50, 10], "normal": [0, 0, 1] } },
        "b": { "instance": k, "geometry": { "type": "face", "at": [10, 10, 10], "normal": [0, 0, 1] } } })).unwrap_err();
    assert!(err.message.contains("tangent needs a round face"), "{}", err.message);
}

#[test]
fn a_shaft_goes_concentric_into_a_bore_found_on_the_mesh() {
    let mut p = project();
    let plate = component(&mut p, "Plate", "return box(40, 40, 10).subtract(cylinder(10, 6, { segments: 48 }).translate(20, 20, 0));");
    let shaft = component(&mut p, "Shaft", "return cylinder(30, 5, { segments: 48 });");
    let pl = place(&mut p, plate, "Plate", [0.0; 3], [0.0; 3]);
    let s = place(&mut p, shaft, "Shaft", [70.0, 10.0, 3.0], [10.0, 0.0, 0.0]);
    // The bore's face: a cylinder of radius 6 (hollow), axis through (20, 20).
    let bore = de_assembly::geometry::resolve(p.world(), pl, &de_assembly::geometry::Geometry::Face { at: [26.0, 20.0, 5.0], normal: [-1.0, 0.0, 0.0] }).unwrap();
    match bore {
        de_assembly::geometry::Elem::Line { p: at, d, r } => {
            assert!((r + 6.0).abs() < 1e-6, "{r}");
            assert!((at[0] - 20.0).abs() < 1e-6 && (at[1] - 20.0).abs() < 1e-6, "{at:?}");
            assert!((d[2].abs() - 1.0).abs() < 1e-9);
        }
        e => panic!("{e:?}"),
    }
    run(&mut p, "assembly.joint.create", json!({ "type": "concentric",
        "a": { "instance": pl, "geometry": { "type": "face", "at": [26, 20, 5], "normal": [-1, 0, 0] } },
        "b": { "instance": s, "geometry": { "type": "face", "at": [5, 0, 10], "normal": [1, 0, 0] } } })).unwrap();
    let t = placed(&p, s);
    let axis_pt = t.apply([0.0, 0.0, 0.0]);
    assert!((axis_pt[0] - 20.0).abs() < 1e-6 && (axis_pt[1] - 20.0).abs() < 1e-6, "{axis_pt:?}");
    assert!(t.turn([0.0, 0.0, 1.0])[2].abs() > 1.0 - 1e-9);
    assert!(axis_pt[2].abs() < 15.0, "it moves as little as the mate needs, not along its free axis: {axis_pt:?}");
    // Slides along and turns about the axis: 2 left.
    let d = de_assembly::dof::report(p.world(), None).unwrap();
    assert_eq!(d.groups[0].free, 2, "{:?}", d.groups[0]);
    // A roller resting on a floor: tangent to its top face.
    let floor = component(&mut p, "Floor", "return box(200, 200, 5).translate(0, 0, -5);");
    let f = place(&mut p, floor, "Floor", [0.0, 0.0, -50.0], [0.0; 3]);
    let roller = place(&mut p, shaft, "Roller", [100.0, 100.0, -20.0], [90.0, 0.0, 0.0]);
    run(&mut p, "assembly.joint.create", json!({ "type": "tangent",
        "a": { "instance": f, "geometry": { "type": "face", "at": [100, 100, 0], "normal": [0, 0, 1] } },
        "b": { "instance": roller, "geometry": { "type": "cylinder", "at": [0, 0, 0], "dir": [0, 0, 1], "radius": 5 } } })).unwrap();
    let t = placed(&p, roller);
    assert!((t.apply([0.0; 3])[2] - (-45.0)).abs() < 1e-7, "axis 5 above the floor's top at −50: {:?}", t.apply([0.0; 3]));
}

#[test]
fn redundant_and_conflicting_joints_are_named() {
    let mut p = project();
    let bracket = component(&mut p, "Bracket", r#"connector("A", { at: [10, 10, 5], axis: [0, 0, 1] }); connector("B", { at: [30, 10, 5], axis: [0, 0, 1] }); return box(40, 20, 5);"#);
    let lid = component(&mut p, "Lid", r#"connector("A", { at: [10, 10, 0], axis: [0, 0, -1] }); connector("B", { at: [30, 10, 0], axis: [0, 0, -1] }); return box(40, 20, 3);"#);
    let b = place(&mut p, bracket, "Bracket", [0.0; 3], [0.0; 3]);
    let l = place(&mut p, lid, "Lid", [0.0, 0.0, 50.0], [0.0; 3]);
    run(&mut p, "assembly.joint.create", json!({ "type": "fixed", "name": "Bolt A", "a": { "instance": b, "connector": "A" }, "b": { "instance": l, "connector": "A" } })).unwrap();
    run(&mut p, "assembly.joint.create", json!({ "type": "fixed", "name": "Bolt B", "a": { "instance": b, "connector": "B" }, "b": { "instance": l, "connector": "B" } })).unwrap();
    let d = de_assembly::dof::report(p.world(), None).unwrap();
    let g = &d.groups[0];
    assert_eq!((g.free, g.mobility), (0, 0));
    assert_eq!(g.redundant, 6, "bolted twice: one bolt's six conditions repeat the other's");
    let names: Vec<&str> = g.redundant_joints.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Bolt A", "Bolt B"]);
    // A gap on one bolt only: they cannot both hold.
    let j = de_assembly::joints::joints_in(p.world(), None)[1].0;
    assert!(run(&mut p, "assembly.joint.update", json!({ "target": j, "offset": 2 })).is_err(), "refused with the reason");
}

#[test]
fn a_hinge_chain_moves_one_way_per_hinge() {
    let mut p = project();
    let link = component(&mut p, "Link", LINK);
    let crank = place(&mut p, link, "Crank", [0.0; 3], [0.0; 3]);
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Drive", "a": "ground", "b": { "instance": crank, "connector": "Bottom" }, "value": 30 })).unwrap();
    let forearm = place(&mut p, link, "Rocker", [0.0, 0.0, 0.0], [0.0; 3]);
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Elbow", "a": { "instance": crank, "connector": "Tip" }, "b": { "instance": forearm, "connector": "Bottom" }, "value": 40 })).unwrap();
    let d = de_assembly::dof::report(p.world(), None).unwrap();
    assert_eq!(d.groups[0].mobility, 2, "two hinges in a chain: two ways to move");
    assert_eq!(d.groups[0].free, 0);
    assert_eq!(d.groups[0].redundant, 0);
    let fore = d.groups[0].instances.iter().find(|i| i.id == forearm).unwrap();
    assert_eq!((fore.free, fore.mobility), (0, 2));
}

/// A gripper: a pinion on a hinge drives two fingers on sliders. Fingers geared
/// to the pinion (by formula, as the robot arm's are, or by couplings) move with
/// it: one way to move, not three.
#[test]
fn geared_fingers_count_once() {
    for linked_by in ["none", "formula", "coupling"] {
        let mut p = project();
        let body = component(&mut p, "Body", r#"connector("Pinion", { at: [0, 0, 10], axis: [0, 0, 1] }); connector("A", { at: [0, 10, 5], axis: [1, 0, 0] }); connector("B", { at: [0, -10, 5], axis: [-1, 0, 0] }); return box(60, 40, 10).translate(-30, -20, 0);"#);
        let pinion = component(&mut p, "Pinion", r#"connector("Bore", { at: [0, 0, 0], axis: [0, 0, -1] }); return cylinder(5, 8);"#);
        let finger = component(&mut p, "Finger", r#"connector("Slide", { at: [0, 0, 0], axis: [1, 0, 0] }); return box(20, 4, 4).translate(0, -2, -2);"#);
        let b = place(&mut p, body, "Body", [0.0; 3], [0.0; 3]);
        let pi = place(&mut p, pinion, "Pinion", [0.0, 0.0, 10.0], [0.0; 3]);
        let fa = place(&mut p, finger, "Finger A", [0.0, 10.0, 5.0], [0.0; 3]);
        let fb = place(&mut p, finger, "Finger B", [0.0, -10.0, 5.0], [0.0; 3]);
        run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Gripper", "a": { "instance": b, "connector": "Pinion" }, "b": { "instance": pi, "connector": "Bore" }, "value": 30, "limits": [0, 85] })).unwrap();
        let value = |v: &str| if linked_by == "formula" { json!(v) } else { json!(0) };
        run(&mut p, "assembly.joint.create", json!({ "type": "slider", "name": "Finger A", "aligned": true, "a": { "instance": b, "connector": "A" }, "b": { "instance": fa, "connector": "Slide" }, "value": value("=Gripper.angle * 0.174533"), "limits": [0, 15] })).unwrap();
        run(&mut p, "assembly.joint.create", json!({ "type": "slider", "name": "Finger B", "aligned": true, "a": { "instance": b, "connector": "B" }, "b": { "instance": fb, "connector": "Slide" }, "value": value("=Gripper.angle * 0.174533"), "limits": [0, 15] })).unwrap();
        if linked_by == "coupling" {
            for f in ["Finger A", "Finger B"] {
                run(&mut p, "assembly.coupling.create", json!({ "type": "rack_pinion", "driver": "Gripper", "follower": f, "radius": 10 })).unwrap();
            }
        }
        let d = de_assembly::dof::report(p.world(), None).unwrap();
        let g = &d.groups[0];
        assert_eq!(g.free, 0, "{linked_by}: {}", g.summary);
        if linked_by == "none" {
            assert_eq!(g.mobility, 3, "unlinked, each finger is one more way to move: {}", g.summary);
            assert!(g.following.is_empty());
            continue;
        }
        assert_eq!(g.mobility, 1, "{linked_by}: the fingers follow the pinion: {}", g.summary);
        let names: Vec<&str> = g.following.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Finger A", "Finger B"], "{linked_by}");
        assert!(g.following.iter().all(|f| f.drivers == ["Gripper"]), "{linked_by}: {:?}", g.following);
        assert!(g.summary.contains("Finger A with Gripper"), "{}", g.summary);
        for f in [fa, fb] {
            assert_eq!(g.instances.iter().find(|i| i.id == f).unwrap().mobility, 1, "{linked_by}: a finger moves one way, with the pinion");
        }
        // At the closed end (0 mm, its lower limit) a finger still follows: the slope is taken one-sided.
        let gripper = de_assembly::joints::joints_in(p.world(), None).into_iter().map(|(j, _)| j).find(|j| de_assembly::joints::joint_name(p.world(), *j) == "Gripper").unwrap();
        run(&mut p, "parameters.set", json!({ "target": gripper, "values": { "angle": 0 } })).unwrap();
        let d = de_assembly::dof::report(p.world(), None).unwrap();
        assert_eq!(d.mobility, 1, "{linked_by} closed: {}", d.summary);
    }
}

/// A part whose shape follows a joint (a spring whose stretched length is a
/// formula of an arm's angle) is evaluated again at each step of a sweep, so
/// its clashes are found where they happen, not only where it stands now.
#[test]
fn a_part_that_stretches_with_a_joint_is_swept_as_it_stretches() {
    let mut p = project();
    // The arm's shape shares its name with the joint (as the robot arm's printed
    // Wrist and its Wrist joint do): `Swing.angle` still means the joint's angle.
    let arm = component(&mut p, "Swing", r#"connector("Pivot", { at: [0, 0, 0], axis: [0, 0, -1] }); return box(40, 4, 4).translate(0, -2, 0);"#);
    let a = place(&mut p, arm, "Arm", [0.0; 3], [0.0; 3]);
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Swing", "a": { "at": [0, 0, 0] }, "b": { "instance": a, "connector": "Pivot" }, "value": 0, "limits": [0, 90] })).unwrap();
    // A rod standing apart from the arm, 10 mm long plus a millimetre per degree of the swing.
    let rod = id(&run(&mut p, "shape.create", json!({ "name": "Rod", "source": "return box(p.len, 2, 2);", "params": [{ "key": "len", "label": "Length", "value": 10, "unit": "mm" }], "definition": true })).unwrap());
    run(&mut p, "shape.link_param", json!({ "target": rod, "key": "len", "expr": "=Swing.angle + 10" })).unwrap();
    place(&mut p, rod, "Rod", [0.0, 100.0, 0.0], [0.0; 3]);
    let wall = component(&mut p, "Wall", "return box(5, 5, 5);");
    place(&mut p, wall, "Wall", [50.0, 99.0, -1.0], [0.0; 3]);
    let joint = de_assembly::joints::joints_in(p.world(), None)[0].0;
    let sw = de_assembly::motion::sweep(p.world(), joint, None, None, None, 18, &HashMap::new(), true).unwrap();
    let clash = sw.clashes.iter().find(|c| [c.a.as_str(), c.b.as_str()].contains(&"Rod") && [c.a.as_str(), c.b.as_str()].contains(&"Wall")).unwrap_or_else(|| panic!("the rod grows into the wall: {:?} {:?}", sw.pairs, sw.notes));
    let r = clash.range.unwrap();
    assert!((r[0] - 40.0).abs() < 1e-6 && (r[1] - 90.0).abs() < 1e-6, "it reaches the wall at 40° (10 + 40 = 50 mm): {r:?}");
    assert!(sw.notes.iter().any(|n| n.contains("Rod follows the joints")), "{:?}", sw.notes);
}

#[test]
fn gears_and_racks_turn_together_either_way_round() {
    let mut p = project();
    let wheel = component(&mut p, "Wheel", r#"connector("Bore", { at: [0, 0, 0], axis: [0, 0, -1] }); return cylinder(5, 20);"#);
    let rack = component(&mut p, "Rack", r#"connector("Slide", { at: [0, 0, 0], axis: [1, 0, 0] }); return box(100, 10, 10);"#);
    let g1 = place(&mut p, wheel, "Driver", [0.0; 3], [0.0; 3]);
    let g2 = place(&mut p, wheel, "Driven", [60.0, 0.0, 0.0], [0.0; 3]);
    let r = place(&mut p, rack, "Rack", [0.0, -40.0, 0.0], [0.0; 3]);
    let a = id(&run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Input", "a": { "at": [0, 0, 0] }, "b": { "instance": g1, "connector": "Bore" } })).unwrap());
    let b = id(&run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Output", "a": { "at": [60, 0, 0] }, "b": { "instance": g2, "connector": "Bore" } })).unwrap());
    let s = id(&run(&mut p, "assembly.joint.create", json!({ "type": "slider", "name": "Travel", "a": { "at": [0, -40, 0], "axis": [1, 0, 0] }, "b": { "instance": r, "connector": "Slide" }, "limits": [-500, 500] })).unwrap());
    run(&mut p, "assembly.coupling.create", json!({ "type": "gear", "driver": "Input", "follower": "Output", "teeth": [20, 40] })).unwrap();
    run(&mut p, "assembly.coupling.create", json!({ "type": "rack_pinion", "driver": "Input", "follower": "Travel", "radius": 10 })).unwrap();
    run(&mut p, "parameters.set", json!({ "target": a, "values": { "angle": 90 } })).unwrap();
    let vals = &p.get::<Placed>(g2).unwrap().values;
    assert!((vals[&b][0] + 45.0).abs() < 1e-9, "the driven gear turns back half as far: {:?}", vals[&b]);
    let rv = &p.get::<Placed>(r).unwrap().values;
    assert!((rv[&s][0] - 10.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-9, "a quarter turn of a 10 mm pinion: {:?}", rv[&s]);
    // Driving the output turns the input back through the same ratio.
    let sw = de_assembly::motion::sweep(p.world(), b, None, Some(0.0), Some(-90.0), 4, &HashMap::new(), false).unwrap();
    let last = sw.frames.last().unwrap();
    assert!((last.joints[&a][0] - 180.0).abs() < 1e-6, "{:?}", last.joints[&a]);
    // A follower driven past its limits fails, and says by what.
    run(&mut p, "assembly.joint.update", json!({ "target": s, "limits": [-10, 10] })).unwrap_err();
}

#[test]
fn patterns_keep_their_copies_in_step() {
    let mut p = project();
    let bolt = component(&mut p, "Bolt", "return cylinder(20, 3);");
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Flange" })).unwrap());
    let seed = id(&run(&mut p, "assembly.instance.create", json!({ "of": bolt, "parent": asm, "name": "Bolt", "at": [10, 0, 0] })).unwrap());
    let out = run(&mut p, "assembly.pattern.create", json!({ "seed": seed, "type": "linear", "count": 4, "spacing": 25, "direction": [0, 1, 0] })).unwrap();
    let pat = id(&out);
    let members: Vec<StableId> = serde_json::from_value(out["members"].clone()).unwrap();
    assert_eq!(members.len(), 3);
    assert!(close(placed(&p, members[2]).apply([0.0; 3]), [10.0, 75.0, 0.0], 1e-9));
    // Spacing is a parameter: change it and the copies follow.
    run(&mut p, "parameters.set", json!({ "target": pat, "values": { "spacing": 30 } })).unwrap();
    assert!(close(placed(&p, members[2]).apply([0.0; 3]), [10.0, 90.0, 0.0], 1e-9));
    // Move the seed: they come too.
    run(&mut p, "assembly.instance.update", json!({ "target": seed, "at": [0, 0, 5] })).unwrap();
    assert!(close(placed(&p, members[0]).apply([0.0; 3]), [0.0, 30.0, 5.0], 1e-9));
    // A copy cannot be moved or deleted on its own.
    assert!(run(&mut p, "assembly.instance.update", json!({ "target": members[0], "at": [1, 1, 1] })).unwrap_err().message.contains("pattern"));
    assert!(run(&mut p, "assembly.instance.delete", json!({ "target": members[0] })).is_err());
    // More copies.
    let out = run(&mut p, "assembly.pattern.update", json!({ "target": pat, "count": 6 })).unwrap();
    let members: Vec<StableId> = serde_json::from_value(out["members"].clone()).unwrap();
    assert_eq!(members.len(), 5);
    assert_eq!(de_assembly::children(p.world(), asm).len(), 6);
    let st = p.get::<de_assembly::AssemblyStats>(asm).unwrap();
    assert_eq!(st.shapes, 6);
    // A copy cannot seed another pattern.
    assert!(run(&mut p, "assembly.pattern.create", json!({ "seed": members[4], "type": "circular", "count": 3, "span": 360 })).is_err());
    // Delete keeping the copies: they stay where they are, free.
    let before = placed(&p, members[4]);
    run(&mut p, "assembly.pattern.delete", json!({ "target": pat, "keep": true })).unwrap();
    assert!(close(placed(&p, members[4]).apply([0.0; 3]), before.apply([0.0; 3]), 1e-9));
    run(&mut p, "assembly.instance.update", json!({ "target": members[4], "at": [0, 0, 0] })).unwrap();
}

#[test]
fn a_circular_pattern_goes_round_its_axis() {
    let mut p = project();
    let bolt = component(&mut p, "Bolt", "return cylinder(20, 3);");
    let seed = place(&mut p, bolt, "Bolt", [50.0, 0.0, 0.0], [0.0; 3]);
    let out = run(&mut p, "assembly.pattern.create", json!({ "seed": seed, "type": "circular", "count": 4, "span": 360, "center": [0, 0, 0] })).unwrap();
    let m: Vec<StableId> = serde_json::from_value(out["members"].clone()).unwrap();
    assert!(close(placed(&p, m[0]).apply([0.0; 3]), [0.0, 50.0, 0.0], 1e-9));
    assert!(close(placed(&p, m[1]).apply([0.0; 3]), [-50.0, 0.0, 0.0], 1e-9));
    // Deleting the seed takes the pattern and its copies.
    run(&mut p, "assembly.instance.delete", json!({ "target": seed })).unwrap();
    assert!(p.get::<de_assembly::Instance>(m[0]).is_none());
}

#[test]
fn exploded_views_studies_and_poses() {
    let mut p = project();
    let link = component(&mut p, "Link", LINK);
    let block = component(&mut p, "Block", "return box(10, 10, 10);");
    let l1 = place(&mut p, link, "Arm", [0.0; 3], [0.0; 3]);
    let k = place(&mut p, block, "Block", [200.0, 0.0, 0.0], [0.0; 3]);
    let j = id(&run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Swing", "a": "ground", "b": { "instance": l1, "connector": "Bottom" }, "limits": [-90, 90] })).unwrap());
    // A pose keeps the joints' values; applying it puts them back.
    let home = id(&run(&mut p, "assembly.pose.create", json!({ "name": "Home" })).unwrap());
    run(&mut p, "assembly.pose.create", json!({ "name": "Up", "values": { "Swing": 90 } })).unwrap();
    run(&mut p, "parameters.set", json!({ "target": j, "values": { "angle": 45 } })).unwrap();
    run(&mut p, "assembly.pose.apply", json!({ "target": home })).unwrap();
    assert_eq!(p.get::<Placed>(l1).unwrap().values[&j][0], 0.0);
    // A study: up to the pose over 2 s, then a step back down.
    let st = id(&run(&mut p, "assembly.motion_study.create", json!({ "name": "Wave", "poses": [{ "pose": "Home", "at": 0 }, { "pose": "Up", "at": 2 }], "steps": [{ "joint": "Swing", "to": -30, "start": 2, "duration": 1 }] })).unwrap());
    let played = de_assembly::views::play_study(p.world(), st, 10.0, true).unwrap();
    assert_eq!(played.frames.len(), 31);
    assert!((played.frames[10].value - 1.0).abs() < 1e-9);
    assert!((played.frames[20].joints[&j][0] - 90.0).abs() < 1e-6, "{:?}", played.frames[20].joints[&j]);
    assert!((played.frames[30].joints[&j][0] + 30.0).abs() < 1e-6);
    assert!(played.instances.iter().any(|i| i.id == l1 && i.moves));
    // An exploded view worked out from the middle, with trails.
    let ex = id(&run(&mut p, "assembly.explode.create", json!({ "name": "Apart", "auto": { "spread": 1 } })).unwrap());
    let e = de_assembly::views::explode(p.world(), ex).unwrap();
    assert_eq!(e.steps.len(), 2, "one step: everything but the biggest part (the arm) comes off together");
    assert_eq!(e.trails.len(), 1);
    let kpos = e.instances.iter().position(|i| i.id == k).unwrap();
    assert!(e.steps[1].poses[kpos].0[0] > 200.0, "the block moves outward: {:?}", e.steps[1].poses[kpos]);
    // Steps given by hand: lift the block 50 mm, then turn it a quarter.
    run(&mut p, "assembly.explode.update", json!({ "target": ex, "steps": [{ "instances": [k], "move": [0, 0, 50] }, { "instances": [k], "turn": { "axis": [0, 0, 1], "angle": 90 } }] })).unwrap();
    let e = de_assembly::views::explode(p.world(), ex).unwrap();
    assert_eq!(e.steps[1].poses[kpos].0, [200.0, 0.0, 50.0]);
    assert_eq!(e.trails[0].points.len(), 3);
}

#[test]
fn a_two_link_arm_reaches_where_it_is_told() {
    let mut p = project();
    let link = component(&mut p, "Link", LINK);
    let l1 = place(&mut p, link, "Upper arm", [0.0; 3], [0.0; 3]);
    let l2 = place(&mut p, link, "Forearm", [300.0, 0.0, 0.0], [0.0; 3]);
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Shoulder", "a": "ground", "b": { "instance": l1, "connector": "Bottom" }, "limits": [-150, 150] })).unwrap();
    run(&mut p, "assembly.joint.create", json!({ "type": "hinge", "name": "Elbow", "a": { "instance": l1, "connector": "Tip" }, "b": { "instance": l2, "connector": "Bottom" }, "limits": [-150, 150] })).unwrap();
    let rig = de_assembly::robot::Rig::new(p.world(), l2, Some("Tip"), None).unwrap();
    assert_eq!(rig.axes.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Shoulder", "Elbow"]);
    // Forward: by hand, the tip is at Rz(75°)(90, 0, 10) + Rz(30°)(90, 0, 10) for 30° and 45°.
    let t = rig.fk(&[30.0, 45.0]).unwrap();
    let (r30, r75) = (30f64.to_radians(), 75f64.to_radians());
    let want = [90.0 * r75.cos() + 90.0 * r30.cos(), 90.0 * r75.sin() + 90.0 * r30.sin(), 20.0];
    assert!(close(t.at, want, 1e-9), "{:?} vs {want:?}", t.at);
    // Inverse: back to the same point from straight out.
    let ik = rig.ik(&de_assembly::robot::Target { at: want, axis: None }, &[0.0, 0.0]).unwrap();
    assert!(ik.reached && ik.error_mm < 1e-4, "{ik:?}");
    assert!(close(rig.fk(&ik.q).unwrap().at, want, 1e-4));
    // Out of reach: refused, saying how close it gets.
    let err = run(&mut p, "assembly.robot.move_to", json!({ "tip": l2, "tool": "Tip", "to": [400, 0, 20] })).unwrap_err();
    assert!(err.message.contains("out of reach"), "{}", err.message);
    run(&mut p, "assembly.robot.move_to", json!({ "tip": l2, "tool": "Tip", "to": [0, 150, 20] })).unwrap();
    let tip = placed(&p, l2).apply([95.0, 5.0, 10.0]);
    assert!(close(tip, [0.0, 150.0, 20.0], 1e-3), "{tip:?}");
    // The reach: 180 mm out at most (in the plane, 20 up).
    let rig = de_assembly::robot::Rig::new(p.world(), l2, Some("Tip"), None).unwrap();
    let r = de_assembly::robot::reach(&rig, 600).unwrap();
    assert!((r.max_reach - (180f64.powi(2) + 20f64.powi(2)).sqrt()).abs() < 0.5, "{}", r.max_reach);
    // A trajectory: a straight line, then a pose of values.
    let tr = de_assembly::robot::trajectory(p.world(), &rig, &[
        de_assembly::robot::Waypoint { pose: None, values: None, to: Some([100.0, 50.0, 20.0]), axis: None, linear: true, duration: Some(1.0) },
        de_assembly::robot::Waypoint { pose: None, values: Some([("Shoulder".to_string(), de_assembly::views::Values::One(0.0)), ("Elbow".to_string(), de_assembly::views::Values::One(0.0))].into_iter().collect()), to: None, axis: None, linear: false, duration: Some(1.0) },
    ], 10.0, false).unwrap();
    assert!(tr.waypoints.iter().all(|w| w.reached));
    // Along the straight part every tool point lies on the line from (0, 150) to (100, 50): x + y = 150.
    for q in &tr.tool_path[1..=10] {
        assert!((q[0] + q[1] - 150.0).abs() < 1e-2, "{q:?}");
    }
    assert!(close(*tr.tool_path.last().unwrap(), [180.0, 0.0, 20.0], 1e-6));
}

#[test]
fn interference_finds_overlap_volume_and_touching() {
    let mut p = project();
    let cube = component(&mut p, "Cube", "return box(10, 10, 10);");
    place(&mut p, cube, "A", [0.0; 3], [0.0; 3]);
    place(&mut p, cube, "B", [5.0, 0.0, 0.0], [0.0; 3]);
    place(&mut p, cube, "C", [0.0, 10.0, 0.0], [0.0; 3]);
    place(&mut p, cube, "D", [0.0, 0.0, 10.5], [0.0; 3]);
    let r = de_assembly::interference::report(p.world(), None, 1.0).unwrap();
    assert_eq!(r.overlaps.len(), 1);
    assert!((r.overlaps[0].volume - 500.0).abs() < 1e-6, "{:?}", r.overlaps[0]);
    assert_eq!(r.overlaps[0].size, Some([5.0, 10.0, 10.0]));
    assert!(r.touching.len() >= 2, "C touches A and B: {:?}", r.touching);
    assert_eq!(r.near.len(), 3, "D is 0.5 mm above A, B and C: {:?}", r.near);
    assert!((r.near[0].gap - 0.5).abs() < 1e-6);
}
