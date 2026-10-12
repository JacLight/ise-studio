//! The sketcher's tools end to end, through commands: ready-made shapes,
//! split and join, B-spline editing, transforms with copies, carbon copies
//! that follow their source, the checker, axis release, internal geometry,
//! reference dimensions, and exact conics in a part.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_shape::sketch::{Constraint, Curve, Sketch, Solved, SolvedSketch, loops, polygon, solve};
use de_shape::{ShapePlugin, ShapeSolid};
use de_solid::SolidPlugin;
use std::f64::consts::PI;
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin])))
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

fn empty(p: &mut Project, name: &str) -> StableId {
    id(&run(p, "sketch.create", json!({ "name": name, "plane": { "name": "XY" } })).unwrap())
}

fn state(p: &Project, s: StableId) -> (Sketch, Solved) {
    let sk = p.get::<Sketch>(s).unwrap().clone();
    let dims = de_shape::params::resolve(p.world(), &sk.params).unwrap();
    let solved = solve(&sk.geometry, &dims).unwrap();
    (sk, solved)
}

/// Total signed area of the closed outlines (holes negative).
fn area(p: &Project, s: StableId) -> (f64, usize) {
    let (sk, solved) = state(p, s);
    let (l, _) = loops(&sk.geometry, &solved);
    let a = l
        .iter()
        .map(|l| {
            let poly = polygon(l, 1e-5);
            (0..poly.len()).map(|i| poly[i][0] * poly[(i + 1) % poly.len()][1] - poly[(i + 1) % poly.len()][0] * poly[i][1]).sum::<f64>() / 2.0
        })
        .sum();
    (a, l.len())
}

fn ids(v: &Value, k: &str) -> Vec<u32> {
    v[k].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect()
}

#[test]
fn ready_made_shapes_close_and_hold_together() {
    let mut p = project();
    // A slot: 60 between centres, radius 10.
    let s = empty(&mut p, "Slot");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "slot", "a": [0, 0], "b": [60, 0], "radius": 10 } })).unwrap();
    let (a, n) = area(&p, s);
    assert_eq!(n, 1);
    assert!((a - (60.0 * 20.0 + PI * 100.0)).abs() < 1e-3, "{a}");
    let (sk, solved) = state(&p, s);
    assert!(solved.conflicts.is_empty() && solved.redundant.is_empty(), "{solved:?}");
    // The width is a dimension: change it and the slot follows.
    let key = sk.params.iter().find(|q| q.label.starts_with("Slot width")).unwrap().key.clone();
    run(&mut p, "parameters.set", json!({ "target": s, "values": { key: 30 } })).unwrap();
    let (a, _) = area(&p, s);
    assert!((a - (60.0 * 30.0 + PI * 225.0)).abs() < 0.05, "{a}");

    // A rounded rectangle 80 × 50, r 8.
    let s = empty(&mut p, "Rounded");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "rounded_rectangle", "corners": [[0, 0], [80, 50]], "radius": 8 } })).unwrap();
    let (a, n) = area(&p, s);
    assert_eq!(n, 1);
    assert!((a - (80.0 * 50.0 - (4.0 - PI) * 64.0)).abs() < 1e-3, "{a}");
    assert!(state(&p, s).1.conflicts.is_empty());

    // A hexagon 20 across corners; a centred rectangle; a 3-point circle; a 3-point arc.
    let s = empty(&mut p, "Bits");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "polygon", "center": [0, 0], "corner": [10, 0], "sides": 6 } })).unwrap();
    let (a, _) = area(&p, s);
    assert!((a - 3.0 * 3f64.sqrt() / 2.0 * 100.0).abs() < 1e-6, "{a}");
    let (_, solved) = state(&p, s);
    assert_eq!(solved.free, 4, "centre, size and turn: {solved:?}");
    let s = empty(&mut p, "Centred");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "centered_rectangle", "center": [5, 5], "corner": [25, 15] } })).unwrap();
    assert!((area(&p, s).0 - 40.0 * 20.0).abs() < 1e-6);
    assert_eq!(state(&p, s).1.free, 4, "centre, width and height");
    let s = empty(&mut p, "Round");
    let out = run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "circle3", "points": [[10, 0], [0, 10], [-10, 0]] } })).unwrap();
    assert!((state(&p, s).1.radii[&ids(&out, "curves")[0]] - 10.0).abs() < 1e-9);
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "arc3", "start": [30, 0], "end": [50, 0], "through": [40, -10] } })).unwrap();
    let (sk, solved) = state(&p, s);
    let Some(Curve::Arc { center, start, .. }) = sk.geometry.curves.last() else { panic!() };
    assert!((solved.points[center][1] - 0.0).abs() < 1e-9 && (solved.points[start][0] - 30.0).abs() < 1e-9, "runs anticlockwise from (30, 0) under through (40, −10)");

    // An arc slot: centre (0,0), radius 50 from 0° to 90°, 10 wide.
    let s = empty(&mut p, "Arc slot");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "arc_slot", "center": [0, 0], "start": [50, 0], "end": [0, 50], "width": 10 } })).unwrap();
    let (a, n) = area(&p, s);
    assert_eq!(n, 1);
    let exact = PI / 4.0 * (55.0 * 55.0 - 45.0 * 45.0) + PI * 25.0;
    assert!((a - exact).abs() < 1e-3, "{a} vs {exact}");
    assert!(state(&p, s).1.conflicts.is_empty());
}

#[test]
fn conics_and_splines_make_exact_parts() {
    let mut p = project();
    // An ellipse through a point: a = 40, passing (20, 15·√3) gives b = 30.
    let s = empty(&mut p, "Oval");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "ellipse", "center": [0, 0], "major": [40, 0], "through": [20, 15.0 * 3f64.sqrt()] } })).unwrap();
    let (a, n) = area(&p, s);
    assert_eq!(n, 1);
    assert!((a - PI * 40.0 * 30.0).abs() < 1e-3, "{a}");
    let part = id(&run(&mut p, "shape.create", json!({ "name": "Puck", "source": "return sketch(\"Oval\").extrude(p.t);", "params": [{ "key": "t", "label": "T", "value": 5 }] })).unwrap());
    let v = p.get::<ShapeSolid>(part).unwrap().stats.volume_mm3;
    assert!((v - PI * 1200.0 * 5.0).abs() / (PI * 6000.0) < 1e-3, "{v}");
    // Exact: the kernel takes the ellipse as a rational B-spline.
    let program = de_shape::resolved_program(p.world(), p.entity(part).unwrap()).unwrap();
    let values: Vec<(String, f64)> = program.params.iter().map(|q| (q.key.clone(), q.value)).collect();
    let (arena, e) = de_shape::exact::evaluate_exact_in(de_occt::Arena::new(), "Puck", &program.source, &values, 0.01, &program.inputs).map_err(|(_, e)| e).unwrap();
    let v = arena.volume(e.parts[0].shape);
    assert!((v - PI * 6000.0).abs() / (PI * 6000.0) < 1e-7, "exact {v}");

    // A parabola arc closed by a line: area of the parabolic segment (4/3 · base · height / 2).
    let s = empty(&mut p, "Dish");
    let out = run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "parabola_arc", "vertex": [0, 0], "focus": [0, 5], "start": [-20, 0], "end": [20, 0] } })).unwrap();
    let c = ids(&out, "curves")[0];
    let (sk, _) = state(&p, s);
    let Some(Curve::ParabolaArc { start, end, .. }) = sk.geometry.curves.iter().find(|k| k.id() == c) else { panic!() };
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "line", "a": end, "b": start } })).unwrap();
    let (a, n) = area(&p, s);
    assert_eq!(n, 1);
    // y = x²/20 from −20 to 20 under y = 20: 40·20 − 2·(20³/60).
    assert!((a.abs() - (800.0 - 2.0 * 8000.0 / 60.0)).abs() < 1e-3, "{a}");

    // A closed (periodic) B-spline is an outline of its own; a spline through points passes them.
    let s = empty(&mut p, "Blob");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "bspline", "poles": [[0, 0], [40, -5], [50, 30], [10, 40], [-10, 20]], "periodic": true } })).unwrap();
    let (a, n) = area(&p, s);
    assert!(n == 1 && a > 500.0, "{a} {n}");
    let part = id(&run(&mut p, "shape.create", json!({ "name": "Blob part", "source": "return sketch(\"Blob\").extrude(3);", "params": [] })).unwrap());
    let v = p.get::<ShapeSolid>(part).unwrap().stats.volume_mm3;
    assert!((v - a * 3.0).abs() / (a * 3.0) < 2e-3, "{v} vs {}", a * 3.0);
    let s = empty(&mut p, "Wave");
    let out = run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "interpolate", "points": [[0, 0], [10, 10], [20, 0], [30, 10]] } })).unwrap();
    let (sk, solved) = state(&p, s);
    assert!(solved.conflicts.is_empty() && solved.residual < 1e-8, "{solved:?}");
    let c = ids(&out, "curves")[0];
    let geo = de_shape::sketch::geo_of(sk.geometry.curves.iter().find(|k| k.id() == c).unwrap(), &|q| solved.points.get(&q).copied(), None).unwrap();
    for q in [[10.0, 10.0], [20.0, 0.0]] {
        let t = geo.closest(q, false);
        let r = geo.point(t);
        assert!((r[0] - q[0]).hypot(r[1] - q[1]) < 1e-6, "{q:?} {r:?}");
    }
}

#[test]
fn split_join_and_spline_editing() {
    let mut p = project();
    let s = empty(&mut p, "S");
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "polyline", "points": [[0, 0], [40, 0], [40, 30]] } })).unwrap();
    let (sk, _) = state(&p, s);
    let first = sk.geometry.curves[0].id();
    // Split the level line: two lines, both level.
    let out = run(&mut p, "sketch.split", json!({ "target": s, "curve": first, "at": [10, 3] })).unwrap();
    assert_eq!(ids(&out, "curves").len(), 2);
    let (sk, solved) = state(&p, s);
    assert_eq!(sk.geometry.curves.iter().filter(|c| matches!(c, Curve::Line { .. })).count(), 3);
    assert_eq!(solved.points[&(out["point"].as_u64().unwrap() as u32)], [10.0, 0.0]);
    assert!(solved.conflicts.is_empty());

    // Split a circle: two half arcs; the outline is the same.
    run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "circle", "center": [100, 0], "radius": 10 } })).unwrap();
    let circle = state(&p, s).0.geometry.curves.last().unwrap().id();
    run(&mut p, "sketch.split", json!({ "target": s, "curve": circle, "at": [110, 0] })).unwrap();
    let (a, n) = area(&p, s);
    assert_eq!(n, 1);
    assert!((a - PI * 100.0).abs() < 1e-3, "{a}");

    // Join two lines that meet: one B-spline through the corner.
    let (sk, _) = state(&p, s);
    let lines: Vec<u32> = sk.geometry.curves.iter().filter(|c| matches!(c, Curve::Line { .. })).map(Curve::id).collect();
    let out = run(&mut p, "sketch.join", json!({ "target": s, "curves": [lines[1], lines[2]] })).unwrap();
    let joined = out["curve"].as_u64().unwrap() as u32;
    let (sk, solved) = state(&p, s);
    let spline = sk.geometry.curves.iter().find(|c| c.id() == joined).unwrap().clone();
    assert!(matches!(spline, Curve::BSpline { degree: 1, .. }), "{spline:?}");
    let geo = de_shape::sketch::geo_of(&spline, &|q| solved.points.get(&q).copied(), None).unwrap();
    let t = geo.closest([40.0, 0.0], false);
    assert!((geo.point(t)[0] - 40.0).abs() < 1e-9 && geo.point(t)[1].abs() < 1e-9, "passes the corner");

    // Raise its degree, add a knot, weight a pole: the shape holds until the weight changes it.
    let sample = |p: &Project| {
        let (sk, solved) = state(p, s);
        let c = sk.geometry.curves.iter().find(|c| c.id() == joined).unwrap().clone();
        let g = de_shape::sketch::geo_of(&c, &|q| solved.points.get(&q).copied(), None).unwrap();
        [[12.0, 0.0], [40.0, 15.0]].map(|q| {
            let t = g.closest(q, false);
            let r = g.point(t);
            (r[0] - q[0]).hypot(r[1] - q[1])
        })
    };
    run(&mut p, "sketch.bspline", json!({ "target": s, "curve": joined, "op": "increase_degree" })).unwrap();
    assert!(sample(&p).iter().all(|d| *d < 1e-6), "{:?}", sample(&p));
    run(&mut p, "sketch.bspline", json!({ "target": s, "curve": joined, "op": "insert_knot", "at": [40, 15] })).unwrap();
    assert!(sample(&p).iter().all(|d| *d < 1e-6));
    let (sk, _) = state(&p, s);
    let Some(Curve::BSpline { degree, mults, .. }) = sk.geometry.curves.iter().find(|c| c.id() == joined) else { panic!() };
    assert_eq!(*degree, 2);
    assert_eq!(mults.len(), 4, "ends, the corner, the new knot: {mults:?}");
    run(&mut p, "sketch.bspline", json!({ "target": s, "curve": joined, "op": "set_weight", "pole": 1, "weight": 3.0 })).unwrap();
    let (sk, _) = state(&p, s);
    let Some(Curve::BSpline { weights, .. }) = sk.geometry.curves.iter().find(|c| c.id() == joined) else { panic!() };
    assert_eq!(weights[1], 3.0);
    // Any curve converts to a B-spline of the same shape.
    let (sk, _) = state(&p, s);
    let arc = sk.geometry.curves.iter().find(|c| matches!(c, Curve::Arc { .. })).unwrap().id();
    run(&mut p, "sketch.bspline", json!({ "target": s, "curve": arc, "op": "convert" })).unwrap();
    let (a, _) = area(&p, s);
    assert!((a - PI * 100.0).abs() < 1e-3, "still a circle: {a}");
}

#[test]
fn transforms_copy_and_carbon_copies_follow_their_source() {
    let mut p = project();
    let s = empty(&mut p, "Source");
    let out = run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "slot", "a": [0, 0], "b": [30, 0], "radius": 5 } })).unwrap();
    let curves = ids(&out, "curves");
    // Three copies, each turned 90° about (50, 0): four slots in all.
    let out = run(&mut p, "sketch.transform", json!({ "target": s, "curves": curves, "rotate": 90, "center": [50, 0], "copies": 3 })).unwrap();
    assert_eq!(ids(&out, "curves").len(), 12);
    let (a, n) = area(&p, s);
    assert_eq!(n, 4);
    assert!((a - 4.0 * (300.0 + PI * 25.0)).abs() < 1e-3, "{a}");
    let (_, solved) = state(&p, s);
    assert!(solved.conflicts.is_empty(), "{solved:?}");

    // A carbon copy on a parallel plane, linked: the source's slot width drives the copy's.
    let t = id(&run(&mut p, "sketch.create", json!({ "name": "Copy", "plane": { "name": "XY", "offset": 20 } })).unwrap());
    run(&mut p, "sketch.carbon_copy", json!({ "target": t, "from": s })).unwrap();
    let (a, n) = area(&p, t);
    assert_eq!(n, 4);
    assert!((a - 4.0 * (300.0 + PI * 25.0)).abs() < 1e-3);
    let (src, _) = state(&p, s);
    let key = src.params.iter().find(|q| q.label.starts_with("Slot width")).unwrap().key.clone();
    run(&mut p, "parameters.set", json!({ "target": s, "values": { key: 20 } })).unwrap();
    let (a, _) = area(&p, t);
    let first = 30.0 * 20.0 + PI * 100.0;
    assert!(a > 4.0 * (300.0 + PI * 25.0) + (first - 300.0 - PI * 25.0) - 0.05, "the copy followed: {a}");
    // A plane that is not parallel is refused.
    let side = id(&run(&mut p, "sketch.create", json!({ "name": "Side", "plane": { "name": "YZ" } })).unwrap());
    assert!(run(&mut p, "sketch.carbon_copy", json!({ "target": side, "from": s })).is_err());

    // Moving and scaling a dimensioned shape scales its dimensions.
    let m = empty(&mut p, "Scaled");
    let out = run(&mut p, "sketch.add", json!({ "target": m, "shape": { "kind": "rounded_rectangle", "corners": [[0, 0], [40, 20]], "radius": 4 } })).unwrap();
    run(&mut p, "sketch.transform", json!({ "target": m, "curves": ids(&out, "curves"), "scale": 2, "move": [100, 0] })).unwrap();
    let (a, _) = area(&p, m);
    assert!((a - (80.0 * 40.0 - (4.0 - PI) * 64.0)).abs() < 1e-3, "{a}");
}

#[test]
fn the_checker_finds_and_mends() {
    let mut p = project();
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 10, "y": 0 }, { "id": 3, "x": 10.004, "y": 0 }, { "id": 4, "x": 10, "y": 10 }, { "id": 5, "x": 0, "y": 0.003 }, { "id": 6, "x": 5, "y": 5 }],
        "curves": [
            { "kind": "line", "id": 10, "a": 1, "b": 2 }, { "kind": "line", "id": 11, "a": 3, "b": 4 }, { "kind": "line", "id": 12, "a": 4, "b": 5 },
            { "kind": "line", "id": 13, "a": 1, "b": 2 }, { "kind": "line", "id": 14, "a": 6, "b": 6 }
        ],
        "constraints": [{ "kind": "horizontal", "line": 10 }, { "kind": "horizontal", "line": 10 }]
    });
    let s = id(&run(&mut p, "sketch.create", json!({ "name": "Messy", "geometry": g })).unwrap());
    let r = run(&mut p, "sketch.validate", json!({ "target": s })).unwrap();
    assert_eq!(r["gaps"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(ids(&r, "duplicates"), vec![13]);
    assert_eq!(ids(&r, "degenerate"), vec![14]);
    assert_eq!(r["duplicate_constraints"], json!([1]));
    assert!(state(&p, s).1.redundant.contains(&1));
    let r = run(&mut p, "sketch.validate", json!({ "target": s, "fix": true })).unwrap();
    assert_eq!(r["fixed"]["joined"], 2);
    let (a, n) = area(&p, s);
    assert_eq!(n, 1, "the triangle closes");
    assert!((a - 50.0).abs() < 0.1, "{a}");
    let r = run(&mut p, "sketch.validate", json!({ "target": s })).unwrap();
    assert!(r["gaps"].as_array().unwrap().is_empty() && r["duplicates"].as_array().unwrap().is_empty() && r["open_ends"].as_array().unwrap().is_empty(), "{r}");
}

#[test]
fn axes_internal_geometry_references_and_faces() {
    let mut p = project();
    let s = empty(&mut p, "Box");
    let out = run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "rectangle", "corners": [[0, 0], [40, 20]] } })).unwrap();
    let lines = ids(&out, "curves");
    let (_, solved) = state(&p, s);
    assert_eq!(solved.free, 4);
    run(&mut p, "sketch.unalign", json!({ "target": s, "curves": lines })).unwrap();
    let (sk, solved) = state(&p, s);
    assert_eq!(solved.free, 5, "now it can turn as well: {solved:?}");
    assert!(!sk.geometry.constraints.iter().any(|c| matches!(c, Constraint::Horizontal { .. } | Constraint::Vertical { .. })));

    // An ellipse's axes and foci as construction, held to it.
    let e = empty(&mut p, "E");
    let out = run(&mut p, "sketch.add", json!({ "target": e, "shape": { "kind": "ellipse", "center": [0, 0], "major": [50, 0], "minor": 30 } })).unwrap();
    let c = ids(&out, "curves")[0];
    let out = run(&mut p, "sketch.internal", json!({ "target": e, "curve": c })).unwrap();
    assert_eq!(ids(&out, "curves").len(), 2);
    let (_, solved) = state(&p, e);
    assert!(solved.conflicts.is_empty() && solved.residual < 1e-9, "{solved:?}");
    let foci: Vec<[f64; 2]> = ids(&out, "points").iter().map(|q| solved.points[q]).filter(|q| q[1].abs() < 1e-9 && q[0].abs() > 1e-9 && q[0].abs() < 49.0).collect();
    assert!(foci.iter().any(|f| (f[0] - 40.0).abs() < 1e-9), "{foci:?}");
    assert_eq!(solved.free, 5, "the axes and foci add no freedom");

    // A dimension made a reference measures; driving again keeps its value.
    let d = empty(&mut p, "D");
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 30, "y": 0 }],
        "curves": [{ "kind": "line", "id": 3, "a": 1, "b": 2 }],
        "constraints": [{ "kind": "fixed", "point": 1, "x": 0, "y": 0 }, { "kind": "horizontal", "line": 3 }, { "kind": "length", "line": 3, "key": "d1" }, { "kind": "distance", "a": 1, "b": 2, "key": "d2" }]
    });
    run(&mut p, "sketch.update", json!({ "target": d, "geometry": g, "params": [{ "key": "d1", "label": "Length 1", "value": 30 }, { "key": "d2", "label": "Distance 2", "value": 30 }] })).unwrap();
    assert!(state(&p, d).1.redundant.contains(&3), "the second dimension repeats the first");
    run(&mut p, "sketch.constraint", json!({ "target": d, "index": 3, "driving": false })).unwrap();
    let (sk, solved) = state(&p, d);
    assert!(solved.redundant.is_empty());
    assert!(matches!(sk.geometry.constraints[3], Constraint::Reference { .. }));
    run(&mut p, "parameters.set", json!({ "target": d, "values": { "d1": 45 } })).unwrap();
    run(&mut p, "sketch.update", json!({ "target": d })).unwrap();
    let (sk, _) = state(&p, d);
    assert_eq!(sk.params.iter().find(|q| q.key == "d2").unwrap().value, 45.0, "the reference measures");
    run(&mut p, "sketch.constraint", json!({ "target": d, "index": 0, "active": false })).unwrap();
    assert_eq!(state(&p, d).1.free, 2, "the pin is off: it can slide");

    // A sketch on a face: a plane through a point, square to a normal.
    let f = id(&run(&mut p, "sketch.create", json!({ "name": "On top", "plane": { "origin": [5, 5, 12], "normal": [0, 0, 1] } })).unwrap());
    let sk = p.get::<Sketch>(f).unwrap();
    assert_eq!(sk.plane.origin, [5.0, 5.0, 12.0]);
    assert_eq!(sk.plane.x, [1.0, 0.0, 0.0]);
    assert_eq!(sk.plane.normal(), [0.0, 0.0, 1.0]);
    let tilted = id(&run(&mut p, "sketch.create", json!({ "name": "Tilted", "plane": { "origin": [0, 0, 0], "normal": [1, 0, 1] } })).unwrap());
    let n = p.get::<Sketch>(tilted).unwrap().plane.normal();
    assert!((n[0] - 0.5f64.sqrt()).abs() < 1e-12 && (n[2] - 0.5f64.sqrt()).abs() < 1e-12, "{n:?}");
    // Conflicts no longer fail the sketch: it solves as near as it can and names them.
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 30, "y": 0 }],
        "curves": [{ "kind": "line", "id": 3, "a": 1, "b": 2 }],
        "constraints": [{ "kind": "length", "line": 3, "key": "a" }, { "kind": "length", "line": 3, "key": "b" }]
    });
    let c = id(&run(&mut p, "sketch.create", json!({ "name": "Clash", "geometry": g, "params": [{ "key": "a", "label": "A", "value": 30 }, { "key": "b", "label": "B", "value": 40 }] })).unwrap());
    let solved = p.get::<SolvedSketch>(c).unwrap();
    assert_eq!(solved.solved.conflicts, vec![0, 1]);
}

#[test]
fn a_rectangular_array_is_rows_of_a_row() {
    let mut p = project();
    let s = empty(&mut p, "Holes");
    let out = run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "circle", "center": [0, 0], "radius": 2 } })).unwrap();
    let out = run(&mut p, "sketch.pattern", json!({ "target": s, "curves": ids(&out, "curves"), "kind": "linear", "count": 4, "spacing": 10, "rows": 3, "row_spacing": 8 })).unwrap();
    let (sk, solved) = state(&p, s);
    assert_eq!(ids(&out, "curves").iter().filter(|c| matches!(sk.geometry.curves.iter().find(|k| k.id() == **c), Some(Curve::Circle { .. }))).count(), 11);
    let (a, n) = area(&p, s);
    assert_eq!(n, 12);
    assert!((a - 12.0 * PI * 4.0).abs() < 1e-2, "{a}");
    assert!(solved.conflicts.is_empty(), "{solved:?}");
    // The far corner hole sits at (30, 16).
    assert!(solved.points.values().any(|q| (q[0] - 30.0).abs() < 1e-6 && (q[1] - 16.0).abs() < 1e-6));
}

#[test]
fn splitting_a_slot_side_keeps_it_clean() {
    let mut p = project();
    let s = empty(&mut p, "Slot");
    let out = run(&mut p, "sketch.add", json!({ "target": s, "shape": { "kind": "slot", "a": [0, 0], "b": [40, 0], "radius": 6 } })).unwrap();
    for (c, at) in [(ids(&out, "curves")[2], [20.0, 6.0]), (ids(&out, "curves")[0], [20.0, -6.0])] {
        run(&mut p, "sketch.split", json!({ "target": s, "curve": c, "at": at })).unwrap();
        let (_, solved) = state(&p, s);
        assert!(solved.redundant.is_empty() && solved.partially_redundant.is_empty() && solved.conflicts.is_empty(), "{solved:?}");
    }
}
