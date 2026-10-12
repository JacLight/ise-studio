use de_assembly::{AssemblyPlugin, Instance, joints::Placed};
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_draft::{DraftPlugin, DraftShape, Layer};
use de_shape::{ShapePlugin, sketch::{SolvedSketch, Sketch}};
use de_solid::SolidPlugin;
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin, &DraftPlugin])))
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

fn id(v: &Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn draft_geometry_in_model_space() {
    let mut p = project();
    let wire = id(&run(&mut p, "draft.create", json!({ "kind": "wire", "points": [[0, 0, 0], [100, 0, 0], [100, 50, 0], [0, 50, 0]], "closed": true, "hatch": { "pattern": "lines", "angle": 0, "spacing": 5 } })).unwrap());
    let s = p.get::<DraftShape>(wire).unwrap();
    assert!(near(s.length, 300.0) && near(s.area.unwrap(), 5000.0));
    assert_eq!(s.hatch.len(), 10);
    let arc = run(&mut p, "draft.create", json!({ "kind": "arc", "points": [[10, 0, 0], [0, 10, 0], [-10, 0, 0]] })).unwrap();
    assert!((arc["length"].as_f64().unwrap() - std::f64::consts::PI * 10.0).abs() < 0.01, "{arc}");
    let circle = run(&mut p, "draft.create", json!({ "kind": "circle", "center": [0, 0, 5], "radius": 10, "normal": [0, 1, 0] })).unwrap();
    assert!((circle["area"].as_f64().unwrap() - std::f64::consts::PI * 100.0).abs() < 0.5);
    let poly = run(&mut p, "draft.create", json!({ "kind": "polygon", "center": [0, 0, 0], "radius": 10, "sides": 6 })).unwrap();
    assert!(near(poly["length"].as_f64().unwrap(), 60.0));
    let rect = run(&mut p, "draft.create", json!({ "kind": "rectangle", "points": [[0, 0, 0], [30, 20, 0]] })).unwrap();
    assert!(near(rect["area"].as_f64().unwrap(), 600.0));
    let el = run(&mut p, "draft.create", json!({ "kind": "ellipse", "center": [0, 0, 0], "radius": 20, "radius2": 10 })).unwrap();
    assert!((el["area"].as_f64().unwrap() - std::f64::consts::PI * 200.0).abs() < 0.5);
    let spline = run(&mut p, "draft.create", json!({ "kind": "bspline", "points": [[0, 0, 0], [20, 10, 0], [40, -5, 0], [60, 5, 0]] })).unwrap();
    assert!(spline["length"].as_f64().unwrap() > 60.0);
    // On a working plane: points are [u, v] in it.
    let wp = id(&run(&mut p, "draft.plane.create", json!({ "name": "Side", "origin": [0, 0, 100], "normal": [1, 0, 0] })).unwrap());
    let l = id(&run(&mut p, "draft.create", json!({ "kind": "line", "plane": wp, "points": [[0, 0], [10, 0]] })).unwrap());
    let ls = p.get::<DraftShape>(l).unwrap();
    assert!(ls.points.iter().all(|q| near(q[0], 0.0) && near(q[2] - 100.0, if q == &ls.points[0] { 0.0 } else { q[2] - 100.0 })));
    assert!(near(ls.length, 10.0));
    // Refusals say what is missing.
    assert!(run(&mut p, "draft.create", json!({ "kind": "circle", "center": [0, 0, 0] })).unwrap_err().message.contains("radius"));
    assert!(run(&mut p, "draft.create", json!({ "kind": "arc", "points": [[0, 0, 0], [1, 0, 0], [2, 0, 0]] })).unwrap_err().message.contains("line"));
}

#[test]
fn dimensions_follow_the_parts_they_measure() {
    let mut p = project();
    let block = id(&run(&mut p, "shape.create", json!({ "name": "Block", "source": "return box(40, 20, 10);", "params": [], "definition": true })).unwrap());
    let a = id(&run(&mut p, "assembly.instance.create", json!({ "of": block, "name": "A", "at": [0, 0, 0] })).unwrap());
    let b = id(&run(&mut p, "assembly.instance.create", json!({ "of": block, "name": "B", "at": [100, 0, 0] })).unwrap());
    // From A's right end to B's left end, each point on its part.
    let d = run(&mut p, "draft.create", json!({ "kind": "dimension", "measure": "x", "points": [[40, 0, 10], [0, 0, 10]], "anchors": [a, b], "offset": [0, 0, 15] })).unwrap();
    assert!(near(d["value"].as_f64().unwrap(), 60.0), "{d}");
    let dim = id(&d);
    run(&mut p, "assembly.instance.update", json!({ "target": b, "at": [150, 0, 0] })).unwrap();
    let s = p.get::<DraftShape>(dim).unwrap();
    assert!(near(s.value.unwrap(), 110.0));
    assert_eq!(s.label.as_ref().unwrap().1, "110 mm");
    let ang = run(&mut p, "draft.create", json!({ "kind": "dimension", "measure": "angle", "points": [[0, 0, 0], [10, 0, 0], [10, 10, 0]] })).unwrap();
    assert!(near(ang["value"].as_f64().unwrap(), 45.0));
    let r = run(&mut p, "draft.create", json!({ "kind": "dimension", "measure": "diameter", "points": [[0, 0, 0], [12.5, 0, 0]], "text": "<> bore" })).unwrap();
    assert_eq!(p.get::<DraftShape>(id(&r)).unwrap().label.as_ref().unwrap().1, "⌀25 bore");
    let lab = id(&run(&mut p, "draft.create", json!({ "kind": "label", "points": [[50, 50, 30], [20, 10, 10]], "text": "Datum face", "on": a })).unwrap());
    run(&mut p, "assembly.instance.update", json!({ "target": a, "at": [0, 0, 100] })).unwrap();
    assert!(near(p.get::<DraftShape>(lab).unwrap().points[1][2], 110.0), "the label's leader follows its part");
}

#[test]
fn draft_to_sketch_layers_and_arrays() {
    let mut p = project();
    let a = id(&run(&mut p, "draft.create", json!({ "kind": "wire", "points": [[0, 0, 5], [50, 0, 5], [50, 30, 5]] })).unwrap());
    let b = id(&run(&mut p, "draft.create", json!({ "kind": "arc", "points": [[50, 30, 5], [25, 40, 5], [0, 30, 5]] })).unwrap());
    let c = id(&run(&mut p, "draft.create", json!({ "kind": "line", "points": [[0, 30, 5], [0, 0, 5]] })).unwrap());
    let sp = id(&run(&mut p, "draft.create", json!({ "kind": "bspline", "points": [[10, 10, 5], [20, 20, 5], [30, 12, 5], [40, 18, 5]] })).unwrap());
    let txt = id(&run(&mut p, "draft.create", json!({ "kind": "text", "points": [[0, 0, 5]], "text": "Note" })).unwrap());
    let out = run(&mut p, "draft.to_sketch", json!({ "targets": [a, b, c, sp, txt], "name": "Outline" })).unwrap();
    assert_eq!(out["skipped"], json!(["Text 1"]));
    let sk = id(&out);
    let sketch = p.get::<Sketch>(sk).unwrap();
    assert!(near(sketch.plane.origin[2], 5.0));
    let solved = p.get::<SolvedSketch>(sk).unwrap();
    assert_eq!(solved.loops.len(), 1, "the wire, arc and line close one loop");
    assert!(sketch.geometry.curves.iter().any(|c| matches!(c, de_shape::sketch::Curve::Bezier { .. })));
    assert!(run(&mut p, "draft.to_sketch", json!({ "targets": [txt] })).is_err());

    // Layers hold anything; something is in one layer at a time.
    let l1 = id(&run(&mut p, "draft.layer.create", json!({ "name": "Construction", "color": "#888888", "members": [a, b] })).unwrap());
    let l2 = id(&run(&mut p, "draft.layer.create", json!({ "name": "Notes", "members": [txt, b] })).unwrap());
    assert_eq!(p.get::<Layer>(l1).unwrap().members, vec![a]);
    run(&mut p, "draft.layer.update", json!({ "target": l2, "visible": false })).unwrap();
    assert!(!p.get::<Layer>(l2).unwrap().visible);

    // Arrays: real instances placed by the array, spacing linkable.
    let bolt = id(&run(&mut p, "shape.create", json!({ "name": "Peg", "source": "return cylinder(10, 2);", "params": [], "definition": true })).unwrap());
    let peg = id(&run(&mut p, "assembly.instance.create", json!({ "of": bolt, "name": "Peg", "at": [10, 0, 0] })).unwrap());
    let arr = run(&mut p, "draft.array.create", json!({ "source": peg, "kind": "linear", "count": 4, "spacing": 25, "count2": 2, "spacing2": 40 })).unwrap();
    let copies: Vec<StableId> = serde_json::from_value(arr["copies"].clone()).unwrap();
    assert_eq!(copies.len(), 7);
    let at = |p: &Project, i: StableId| p.get::<Placed>(i).unwrap().at;
    assert_eq!(at(&p, copies[2]), [85.0, 0.0, 0.0]);
    assert_eq!(at(&p, copies[3]), [10.0, 40.0, 0.0]);
    let array = id(&arr);
    run(&mut p, "parameters.create", json!({ "name": "Rack", "params": [{ "key": "pitch", "label": "Pitch", "value": 30 }] })).unwrap();
    run(&mut p, "shape.link_param", json!({ "target": array, "key": "spacing", "expr": "=Rack.pitch" })).unwrap();
    assert_eq!(at(&p, copies[2]), [100.0, 0.0, 0.0]);
    run(&mut p, "assembly.instance.update", json!({ "target": peg, "at": [0, 0, 0] })).unwrap();
    assert_eq!(at(&p, copies[0]), [30.0, 0.0, 0.0], "copies follow the original");
    let r = run(&mut p, "draft.array.update", json!({ "target": array, "count": 2, "count2": 1 })).unwrap();
    assert_eq!(r["copies"].as_array().unwrap().len(), 1);
    assert!(p.get::<Instance>(copies[3]).is_none(), "extra copies are removed");

    // Polar: evenly round; path: along a wire.
    let ring = run(&mut p, "draft.array.create", json!({ "source": copies[0], "kind": "polar", "count": 4 })).unwrap_err();
    assert!(ring.message.contains("copy"), "{}", ring.message);
    let hub = id(&run(&mut p, "assembly.instance.create", json!({ "of": bolt, "name": "Spoke", "at": [50, 0, 0] })).unwrap());
    let pol = run(&mut p, "draft.array.create", json!({ "source": hub, "kind": "polar", "count": 4 })).unwrap();
    let pc: Vec<StableId> = serde_json::from_value(pol["copies"].clone()).unwrap();
    let q = at(&p, pc[0]);
    assert!(near(q[0], 0.0) && near(q[1], 50.0), "{q:?}");
    let path = id(&run(&mut p, "draft.create", json!({ "kind": "wire", "points": [[0, 0, 0], [0, 100, 0], [100, 100, 0]] })).unwrap());
    let rail = id(&run(&mut p, "assembly.instance.create", json!({ "of": bolt, "name": "Post", "at": [0, 0, 0] })).unwrap());
    let pa = run(&mut p, "draft.array.create", json!({ "source": rail, "kind": "path", "count": 5, "path": path, "align": true })).unwrap();
    let pc: Vec<StableId> = serde_json::from_value(pa["copies"].clone()).unwrap();
    let q = at(&p, pc[1]);
    assert!(near(q[0], 0.0) && near(q[1], 100.0), "{q:?}");
    let q = at(&p, *pc.last().unwrap());
    assert!(near(q[0], 100.0) && near(q[1], 100.0), "{q:?}");
    run(&mut p, "draft.array.delete", json!({ "target": id(&pa) })).unwrap();
    assert!(p.get::<Instance>(pc[0]).is_some(), "copies stay as instances");
}
