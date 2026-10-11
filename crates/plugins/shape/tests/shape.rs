use de_core::{Actor, CommandError, Envelope, ErrorCode, Project, Registry, SCHEMA_VERSION, StableId, Status, Value, json};
use de_shape::{ShapePlugin, ShapeSolid, runtime};
use de_solid::{DisplayMesh, MassProperties, SolidPlugin};
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

fn id(v: Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

#[test]
fn primitives_and_booleans_measure_correctly() {
    let e = runtime::evaluate("return box(10, 20, 30);", &[], 0.01).unwrap();
    assert!((e.volume - 6000.0).abs() < 1e-6);
    assert_eq!(e.bounds_max, [10.0, 20.0, 30.0]);
    assert!((e.centroid[2] - 15.0).abs() < 1e-6);

    // A 10 mm cube with a 2 mm square hole through it: 1000 − 4 × 10.
    let e = runtime::evaluate("return box(10, 10, 10).subtract(box(2, 2, 12).translate(4, 4, -1));", &[], 0.01).unwrap();
    assert!((e.volume - 960.0).abs() < 1e-6);
    assert_eq!(e.genus, 1, "a hole makes genus 1");
}

#[test]
fn parameters_profiles_and_extrusion() {
    let src = "return regularPolygon(6, p.across / Math.sqrt(3)).extrude(p.height);";
    let e = runtime::evaluate(src, &[("across".into(), 13.0), ("height".into(), 5.0)], 0.01).unwrap();
    // Hexagon across flats s has area (√3/2)·s².
    let expected = 3f64.sqrt() / 2.0 * 13.0 * 13.0 * 5.0;
    assert!((e.volume - expected).abs() < 1e-6, "{} vs {expected}", e.volume);
}

#[test]
fn programs_are_deterministic_and_sandboxed() {
    let src = "const r = rng(7); return box(5 + r(), 5, 5);";
    let a = runtime::evaluate(src, &[], 0.01).unwrap();
    let b = runtime::evaluate(src, &[], 0.01).unwrap();
    assert_eq!(a.volume, b.volume, "seeded randomness repeats exactly");
    assert_eq!(a.positions, b.positions);

    assert!(runtime::evaluate("Math.random(); return box(1,1,1);", &[], 0.01).unwrap_err().contains("deterministic"));
    assert!(runtime::evaluate("return box(Date.now(), 1, 1);", &[], 0.01).is_err(), "no clock");
    assert!(runtime::evaluate("while (true) {}", &[], 0.01).unwrap_err().contains("stopped"));
    assert!(runtime::evaluate("return 42;", &[], 0.01).unwrap_err().contains("must return a Solid"));
    assert!(runtime::evaluate("return box(0, 1, 1);", &[], 0.01).unwrap_err().contains("greater than zero"));
}

#[test]
fn computed_meshes_are_accepted_when_closed() {
    // A tetrahedron from raw math: the escape hatch for any shape.
    let src = "return fromMesh([[0,0,0],[10,0,0],[0,10,0],[0,0,10]], [[0,2,1],[0,1,3],[0,3,2],[1,2,3]]);";
    let e = runtime::evaluate(src, &[], 0.01).unwrap();
    assert!((e.volume - 1000.0 / 6.0).abs() < 1e-6);
    // An open mesh is not a solid.
    assert!(runtime::evaluate("return fromMesh([[0,0,0],[1,0,0],[0,1,0]], [[0,1,2]]);", &[], 0.01).is_err());
}

#[test]
fn shapes_live_in_the_project_with_parameters_mass_and_undo() {
    let mut p = project();
    let steel = id(run(&mut p, "solid.material.create", json!({ "name": "Steel", "density": { "value": 7850, "unit": "kg/m3" } })).unwrap());
    let rod = id(run(&mut p, "shape.create", json!({
        "name": "Rod",
        "source": "return cylinder(p.length, p.diameter / 2, { segments: 256 });",
        "params": [
            { "key": "diameter", "label": "Diameter", "value": 10.0, "min": 1.0 },
            { "key": "length", "label": "Length", "value": 100.0 }
        ],
        "material": steel
    })).unwrap());
    assert_eq!(p.status(rod), Some(&Status::Current));
    assert!(!p.get::<DisplayMesh>(rod).unwrap().normals.is_empty(), "curved shapes carry smooth normals");
    let mass = p.get::<MassProperties>(rod).unwrap().mass_kg.unwrap();
    let exact = std::f64::consts::PI * 0.005f64.powi(2) * 0.1 * 7850.0;
    assert!((mass - exact).abs() / exact < 0.001, "faceted volume within 0.1 % of the true cylinder");

    run(&mut p, "shape.set_params", json!({ "target": rod, "values": { "length": 200.0 } })).unwrap();
    let doubled = p.get::<MassProperties>(rod).unwrap().mass_kg.unwrap();
    assert!((doubled / mass - 2.0).abs() < 1e-9);

    // Out-of-range and unknown parameters are rejected without changing anything.
    assert_eq!(run(&mut p, "shape.set_params", json!({ "target": rod, "values": { "diameter": 0.5 } })).unwrap_err().code, ErrorCode::InvalidParameters);
    assert_eq!(run(&mut p, "shape.set_params", json!({ "target": rod, "values": { "nope": 1.0 } })).unwrap_err().code, ErrorCode::InvalidParameters);

    // A program that fails is rejected before it is stored.
    let err = run(&mut p, "shape.update", json!({ "target": rod, "source": "return cylinder(-1, 2);" })).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidGeometry);

    p.undo(&Actor { user_id: "u".into(), session_id: "s".into() }).unwrap();
    assert!((p.get::<MassProperties>(rod).unwrap().mass_kg.unwrap() - mass).abs() < 1e-12);
    assert!(p.get::<ShapeSolid>(rod).is_some());
}

#[test]
fn meshes_carry_unit_normals_for_lighting() {
    let e = runtime::evaluate("return cylinder(10, 5, { segments: 64 });", &[], 0.01).unwrap();
    assert_eq!(e.normals.len(), e.positions.len());
    let bad = e.normals.chunks_exact(3).filter(|n| ((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() > 1e-3).count();
    assert_eq!(bad, 0, "every vertex normal is unit length");
}

#[test]
fn tolerance_bounds_the_error_from_the_true_formula() {
    for tol in [0.01, 0.001] {
        let e = runtime::evaluate("return cylinder(10, 5);", &[], tol).unwrap();
        // Every vertex lies on the true circle, and every chord stays within `tol` of it.
        let pts: Vec<[f64; 2]> = e.positions.chunks_exact(3).filter(|p| p[2].abs() < 1e-6).map(|p| [p[0] as f64, p[1] as f64]).collect();
        let worst = pts.iter().map(|p| (p[0].hypot(p[1]) - 5.0).abs()).fold(0.0, f64::max);
        assert!(worst < 1e-5, "vertices on the circle");
        let n = de_shape::runtime::evaluate("return cylinder(10, 5);", &[], tol).unwrap().triangles;
        let exact = std::f64::consts::PI * 25.0 * 10.0;
        assert!((exact - e.volume) / exact < tol, "volume error shrinks with tolerance (±{tol}: {} vs {exact})", e.volume);
        assert!(n > 0);
    }
    let coarse = runtime::evaluate("return cylinder(10, 5);", &[], 0.01).unwrap().triangles;
    let fine = runtime::evaluate("return cylinder(10, 5);", &[], 0.001).unwrap().triangles;
    assert!(fine > coarse, "finer accuracy uses more facets");
    assert!(runtime::evaluate("return box(1,1,1);", &[], 0.0).is_err(), "tolerance must be positive");
}

#[test]
fn assemblies_keep_every_part_separate_with_its_own_material() {
    let mut p = project();
    let ply = id(run(&mut p, "solid.material.create", json!({ "name": "Birch ply", "density": { "value": 680, "unit": "kg/m3" } })).unwrap());
    let steel = id(run(&mut p, "solid.material.create", json!({ "name": "Steel", "density": { "value": 7850, "unit": "kg/m3" } })).unwrap());
    let src = r#"
        return [
          part("Top", box(1000, 500, 18).translate(0, 0, 700), { material: "Ply", process: "cnc_router" }),
          part("Leg", box(40, 40, 700), { material: "Steel", process: "weld" }),
        ];"#;
    let desk = id(run(&mut p, "shape.create", json!({
        "name": "Desk", "source": src, "params": [], "materials": { "Ply": ply, "Steel": steel }
    })).unwrap());
    let parts = &p.get::<ShapeSolid>(desk).unwrap().stats.parts;
    assert_eq!(parts.iter().map(|q| q.name.as_str()).collect::<Vec<_>>(), ["Top", "Leg"]);
    assert_eq!(parts[0].process.as_deref(), Some("cnc_router"));
    let mesh = p.get::<DisplayMesh>(desk).unwrap();
    assert_eq!(parts[1].index_start + parts[1].index_count, mesh.indices.len(), "parts tile the index buffer");

    let mass = p.get::<MassProperties>(desk).unwrap().mass_kg.unwrap();
    let expected = 1.0 * 0.5 * 0.018 * 680.0 + 0.04 * 0.04 * 0.7 * 7850.0;
    assert!((mass - expected).abs() < 1e-9, "{mass} vs {expected}");

    // A part made of a material the table does not define is rejected.
    let bad = run(&mut p, "shape.update", json!({ "target": desk, "source": "return [part('A', box(1,1,1), { material: 'Oak' })];" }));
    assert_eq!(bad.unwrap_err().code, ErrorCode::UnresolvedReference);
    // So are unknown processes and duplicate names.
    assert!(runtime::evaluate("return [part('A', box(1,1,1), { process: 'magic' })];", &[], 0.01).unwrap_err().contains("unknown process"));
    assert!(runtime::evaluate("return [part('A', box(1,1,1)), part('A', box(1,1,1))];", &[], 0.01).unwrap_err().contains("unique"));
}

#[test]
fn sheet_parts_export_a_layered_cut_file_and_a_package() {
    use de_shape::fabricate;
    // An 18 mm panel standing on edge, with a 6 mm deep pocket and a through hole.
    let src = "return [part('Side', box(18, 400, 700).subtract(box(10, 100, 100).translate(12, 50, 50), box(30, 20, 20).translate(-5, 300, 300)), { process: 'cnc_router' })];";
    let e = runtime::evaluate(src, &[], 0.01).unwrap();
    let side = &e.parts[0].solid;
    assert_eq!(fabricate::dimensions(side), [700.0, 400.0, 18.0], "length × width × thickness");
    let dxf = fabricate::dxf("Side", side);
    assert!(dxf.contains("DEPTH_18.00"), "through cut at full thickness");
    assert!(dxf.contains("DEPTH_6.00"), "pocket layer 6 mm deep:\n{dxf}");
    assert_eq!(dxf.matches("POLYLINE").count(), 2 + 3, "outline+hole at depth 18; outline+hole+pocket at 6");

    let items = vec![
        fabricate::Item { name: "Shelf 1".into(), material: None, process: Some("cnc_router".into()), solid: manifold_csg::Manifold::cube(500.0, 300.0, 18.0, false), volume_mm3: 2.7e6, key: String::new() },
        fabricate::Item { name: "Shelf 2".into(), material: None, process: Some("cnc_router".into()), solid: manifold_csg::Manifold::cube(500.0, 300.0, 18.0, false).translate(0.0, 0.0, 400.0), volume_mm3: 2.7e6, key: String::new() },
    ];
    let (files, lines) = fabricate::package("Cabinet", &items, &|_| ("Birch ply".into(), Some(680.0)), &|_| None);
    assert_eq!(lines.len(), 1, "identical parts share one cut list line");
    assert_eq!(lines[0].names.len(), 2);
    let names: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Cabinet/Shelf_1.stl", "Cabinet/Shelf_1.dxf", "Cabinet/cut-list.csv"]);
    let zip = fabricate::zip(&files);
    assert_eq!(&zip[..4], b"PK\x03\x04");
}

#[test]
fn linked_parameters_drive_everything_that_reads_them() {
    use de_shape::ResolvedParams;
    let mut p = project();
    let size = id(run(&mut p, "parameters.create", json!({
        "name": "Desk size",
        "params": [{ "key": "width", "label": "Width", "value": 1600 }, { "key": "depth", "label": "Depth", "value": 800 }]
    })).unwrap());
    // The top follows the size; the plug strip sits 100 mm in from the top's right end.
    let top = id(run(&mut p, "shape.create", json!({
        "name": "Table top",
        "source": "return box(p.width, p.depth, 24);",
        "params": [
            { "key": "width", "label": "Width", "value": 1, "expr": "=Desk size.width" },
            { "key": "depth", "label": "Depth", "value": 1, "expr": "= Desk size.depth - 2 * 10" }
        ]
    })).unwrap());
    let plug = id(run(&mut p, "shape.create", json!({
        "name": "Plug strip",
        "source": "return box(300, 60, 40).translate(p.x, 0, 0);",
        "params": [{ "key": "x", "label": "Position", "value": 0, "expr": "=Table top.width - 100mm - 300" }]
    })).unwrap());
    let bounds = |p: &Project, id| p.get::<ShapeSolid>(id).unwrap().stats.bounds_max;
    assert_eq!(bounds(&p, top), [1600.0, 780.0, 24.0]);
    assert_eq!(bounds(&p, plug)[0], 1600.0 - 100.0);

    // One change flows through: the set → the top → the plug.
    run(&mut p, "parameters.set", json!({ "target": size, "values": { "width": 1800 } })).unwrap();
    assert_eq!(bounds(&p, top)[0], 1800.0);
    assert_eq!(bounds(&p, plug)[0], 1700.0);
    assert_eq!(p.get::<ResolvedParams>(plug).unwrap().0["x"], 1400.0);

    // A linked value cannot be set by hand; renaming the source keeps the link.
    let err = run(&mut p, "shape.set_params", json!({ "target": top, "values": { "width": 1000 } })).unwrap_err();
    assert!(err.message.contains("follows"), "{}", err.message);
    run(&mut p, "shape.update", json!({ "target": top, "name": "Desktop" })).unwrap();
    let stored = p.get::<de_shape::ShapeProgram>(plug).unwrap().params[0].expr.clone().unwrap();
    assert!(stored.contains('#'), "stored by id: {stored}");
    assert_eq!(de_shape::params::display(p.world(), &stored), "=Desktop.width - 100 - 300");
    run(&mut p, "parameters.set", json!({ "target": size, "values": { "width": 1800 } })).unwrap();
    assert_eq!(bounds(&p, plug)[0], 1700.0, "still linked after the rename");
    // Loops are refused.
    let err = run(&mut p, "shape.link_param", json!({ "target": top, "key": "width", "expr": "=Plug strip.x" })).unwrap_err();
    assert!(err.message.contains("loop"), "{}", err.message);

    // Unlinking keeps the value the formula gave; the plug no longer follows the set through it.
    run(&mut p, "shape.link_param", json!({ "target": top, "key": "width", "expr": null })).unwrap();
    run(&mut p, "parameters.set", json!({ "target": size, "values": { "width": 2000 } })).unwrap();
    assert_eq!(bounds(&p, top)[0], 1800.0);
    assert_eq!(bounds(&p, plug)[0], 1700.0);

    // Brackets that matter survive the round trip.
    run(&mut p, "shape.link_param", json!({ "target": plug, "key": "x", "expr": "=Desktop.width - (100 + 300) * 2" })).unwrap();
    let stored = p.get::<de_shape::ShapeProgram>(plug).unwrap().params[0].expr.clone().unwrap();
    assert_eq!(de_shape::params::display(p.world(), &stored), "=Desktop.width - (100 + 300) * 2");
    assert_eq!(bounds(&p, plug)[0], 1800.0 - 800.0 + 300.0);

    // Bad references are explained.
    let err = run(&mut p, "shape.link_param", json!({ "target": plug, "key": "x", "expr": "=Nothing.width" })).unwrap_err();
    assert!(err.message.contains("not a parameter"), "{}", err.message);
}

#[test]
fn sketches_drive_shapes_and_their_dimensions_link() {
    use de_shape::sketch::SolvedSketch;
    let mut p = project();
    // A plate: a rectangle (corner pinned, width and height dimensioned) with a dimensioned hole.
    let geometry = json!({
        "points": [
            { "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 95, "y": 2 }, { "id": 3, "x": 103, "y": 58 }, { "id": 4, "x": -2, "y": 61 },
            { "id": 5, "x": 50, "y": 30 }
        ],
        "curves": [
            { "kind": "line", "id": 10, "a": 1, "b": 2 }, { "kind": "line", "id": 11, "a": 2, "b": 3 },
            { "kind": "line", "id": 12, "a": 3, "b": 4 }, { "kind": "line", "id": 13, "a": 4, "b": 1 },
            { "kind": "circle", "id": 20, "center": 5, "radius": 6 }
        ],
        "constraints": [
            { "kind": "fixed", "point": 1, "x": 0, "y": 0 },
            { "kind": "horizontal", "line": 10 }, { "kind": "horizontal", "line": 12 },
            { "kind": "vertical", "line": 11 }, { "kind": "vertical", "line": 13 },
            { "kind": "length", "line": 10, "key": "width" }, { "kind": "length", "line": 11, "key": "height" },
            { "kind": "diameter", "curve": 20, "key": "hole" },
            { "kind": "horizontal_distance", "a": 1, "b": 5, "key": "holeX" }, { "kind": "vertical_distance", "a": 1, "b": 5, "key": "holeY" }
        ]
    });
    let dims = json!([
        { "key": "width", "label": "Width", "value": 100 }, { "key": "height", "label": "Height", "value": 60 },
        { "key": "hole", "label": "Hole", "value": 20 }, { "key": "holeX", "label": "Hole X", "value": 50 }, { "key": "holeY", "label": "Hole Y", "value": 30 }
    ]);
    let plate = id(run(&mut p, "sketch.create", json!({ "name": "Plate outline", "plane": { "name": "XY" }, "geometry": geometry, "params": dims })).unwrap());
    let solved = p.get::<SolvedSketch>(plate).unwrap();
    assert_eq!(solved.solved.free, 0, "fully constrained");
    assert_eq!(solved.loops.len(), 2);

    let part = id(run(&mut p, "shape.create", json!({
        "name": "Plate", "source": "return sketch(\"Plate outline\").extrude(p.t);",
        "params": [{ "key": "t", "label": "Thickness", "value": 10 }]
    })).unwrap());
    let volume = |p: &Project| p.get::<ShapeSolid>(part).unwrap().stats.volume_mm3;
    let exact = |w: f64, h: f64, d: f64| (w * h - std::f64::consts::PI * d * d / 4.0) * 10.0;
    assert!((volume(&p) - exact(100.0, 60.0, 20.0)).abs() / exact(100.0, 60.0, 20.0) < 1e-3, "{}", volume(&p));

    // A sketch dimension changes; the part follows.
    run(&mut p, "parameters.set", json!({ "target": plate, "values": { "width": 140 } })).unwrap();
    assert!((volume(&p) - exact(140.0, 60.0, 20.0)).abs() / exact(140.0, 60.0, 20.0) < 1e-3);

    // A sketch dimension links to a parameter set like any other parameter.
    let size = id(run(&mut p, "parameters.create", json!({ "name": "Enclosure", "params": [{ "key": "width", "label": "Width", "value": 180 }] })).unwrap());
    run(&mut p, "shape.link_param", json!({ "target": plate, "key": "width", "expr": "=Enclosure.width - 20" })).unwrap();
    assert_eq!(p.get::<ShapeSolid>(part).unwrap().stats.bounds_max[0], 160.0);
    run(&mut p, "parameters.set", json!({ "target": size, "values": { "width": 220 } })).unwrap();
    assert_eq!(p.get::<ShapeSolid>(part).unwrap().stats.bounds_max[0], 200.0, "set → sketch → part");

    // On another plane: XZ stands the plate up (its y becomes Z).
    run(&mut p, "sketch.update", json!({ "target": plate, "plane": { "name": "XZ", "offset": 5 } })).unwrap();
    let st = p.get::<ShapeSolid>(part).unwrap().stats.clone();
    assert!((st.bounds_max[2] - 60.0).abs() < 1e-6, "height along Z: {:?}", st.bounds_max);
    assert!((st.bounds_min[1] - -15.0).abs() < 1e-6 && (st.bounds_max[1] - -5.0).abs() < 1e-6, "extruded along the plane's normal (−Y), from y = −5: {:?} {:?}", st.bounds_min, st.bounds_max);

    // Exact: the hole is a true circle.
    let program = de_shape::resolved_program(p.world(), p.entity(part).unwrap()).unwrap();
    let values: Vec<(String, f64)> = program.params.iter().map(|q| (q.key.clone(), q.value)).collect();
    let (arena, e) = de_shape::exact::evaluate_exact_in(de_occt::Arena::new(), "Plate", &program.source, &values, 0.01, &program.inputs).map_err(|(_, e)| e).unwrap();
    let v = arena.volume(e.parts[0].shape);
    assert!((v - exact(200.0, 60.0, 20.0)).abs() < 1e-6, "exact {v} vs {}", exact(200.0, 60.0, 20.0));
    assert!(e.faceted.is_empty());

    // A program cannot read a sketch that is not there.
    let err = run(&mut p, "shape.create", json!({ "name": "X", "source": "return sketch(\"Nope\").extrude(1);", "params": [] })).unwrap_err();
    assert!(err.message.contains("no sketch called"), "{}", err.message);
}

#[test]
fn fillets_and_chamfers_are_true_surfaces_in_the_view_and_the_file() {
    use std::f64::consts::PI;
    // In a program: the four upright edges of a 60 × 40 × 10 block, r = 5.
    let e = runtime::evaluate("return box(60, 40, 10).fillet(5, { parallel: \"z\" });", &[], 0.01).unwrap();
    let exact = 24000.0 - 4.0 * (25.0 - PI * 25.0 / 4.0) * 10.0;
    assert!((e.volume - exact).abs() / exact < 1e-4, "{} vs {exact}", e.volume);
    assert_eq!(e.genus, 0);
    // Smooth shading round the fillet: some normals are diagonal in XY.
    assert!(e.normals.chunks_exact(3).any(|n| n[0].abs() > 0.3 && n[1].abs() > 0.3));
    // Chamfer by points: the edge nearest each point.
    let e = runtime::evaluate("return box(10, 10, 10).chamfer(2, [[5, 0, 10]]);", &[], 0.01).unwrap();
    assert!((e.volume - (1000.0 - 2.0 * 10.0)).abs() < 1e-6, "{}", e.volume);
    // Too large is a clear error.
    let err = runtime::evaluate("return box(10, 10, 10).fillet(8);", &[], 0.01).unwrap_err();
    assert!(err.contains("fillet"), "{err}");

    // Picked in the view: stored as a finish sized by a parameter, found again after a resize.
    let mut p = project();
    let part = id(run(&mut p, "shape.create", json!({
        "name": "Block", "source": "return box(p.w, 40, 10);",
        "params": [{ "key": "w", "label": "Width", "value": 60 }]
    })).unwrap());
    // The upright edge at the far corner (x = max, y = max).
    let pick = json!({ "at": [1, 1, 0.5], "kind": "line", "dir": [0, 0, 1] });
    run(&mut p, "shape.update", json!({
        "target": part,
        "params": [{ "key": "w", "label": "Width", "value": 60 }, { "key": "fillet1", "label": "Fillet 1", "value": 4 }],
        "finishes": [{ "kind": "fillet", "key": "fillet1", "edges": [pick] }]
    })).unwrap();
    let corner = |r: f64| (r * r - PI * r * r / 4.0) * 10.0;
    let volume = |p: &Project| p.get::<ShapeSolid>(part).unwrap().stats.volume_mm3;
    assert!((volume(&p) - (24000.0 - corner(4.0))).abs() < 0.5, "{}", volume(&p));
    run(&mut p, "shape.set_params", json!({ "target": part, "values": { "w": 100 } })).unwrap();
    assert!((volume(&p) - (40000.0 - corner(4.0))).abs() < 0.5, "the pick follows the resized edge: {}", volume(&p));
    // The radius links like any parameter.
    let set = id(run(&mut p, "parameters.create", json!({ "name": "Style", "params": [{ "key": "r", "label": "Radius", "value": 6 }] })).unwrap());
    run(&mut p, "shape.link_param", json!({ "target": part, "key": "fillet1", "expr": "=Style.r" })).unwrap();
    assert!((volume(&p) - (40000.0 - corner(6.0))).abs() < 0.5);
    run(&mut p, "parameters.set", json!({ "target": set, "values": { "r": 2 } })).unwrap();
    assert!((volume(&p) - (40000.0 - corner(2.0))).abs() < 0.5);
    // Exact export: a true cylindrical face.
    let program = de_shape::resolved_program(p.world(), p.entity(part).unwrap()).unwrap();
    let values: Vec<(String, f64)> = program.params.iter().map(|q| (q.key.clone(), q.value)).collect();
    let (arena, e) = de_shape::exact::evaluate_exact_in(de_occt::Arena::new(), "Block", &program.source, &values, 0.01, &program.inputs).map_err(|(_, e)| e).unwrap();
    assert!((arena.volume(e.parts[0].shape) - (40000.0 - corner(2.0))).abs() < 1e-6);
    // Edges for picking: the rounded block shows its arcs.
    let edges = de_shape::exact::edges_of(&program.source, &values, 0.01, &program.inputs).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].edges.iter().filter(|e| e.kind == "circle").count(), 2);
    // A finish without a size parameter is refused.
    let err = run(&mut p, "shape.update", json!({ "target": part, "finishes": [{ "kind": "chamfer", "key": "nope", "edges": [pick] }] })).unwrap_err();
    assert!(err.message.contains("nope"), "{}", err.message);
}

#[test]
fn faces_push_and_pull_in_programs_and_as_picked_finishes() {
    // In a program: the top of a 60 × 40 × 10 block pushed up 5.
    let e = runtime::evaluate("return box(60, 40, 10).push(5, { normal: [0, 0, 1] });", &[], 0.01).unwrap();
    assert!((e.volume - 60.0 * 40.0 * 15.0).abs() < 1e-6, "{}", e.volume);
    // A pull cuts: the +x end pulled in 10.
    let e = runtime::evaluate("return box(60, 40, 10).push(-10, { normal: [1, 0, 0] });", &[], 0.01).unwrap();
    assert!((e.volume - 50.0 * 40.0 * 10.0).abs() < 1e-6);

    // Picked in the view: a push stored on the part, sized by a parameter that may be negative.
    let mut p = project();
    let part = id(run(&mut p, "shape.create", json!({
        "name": "Block", "source": "return box(p.w, 40, 10);",
        "params": [{ "key": "w", "label": "Width", "value": 60 }]
    })).unwrap());
    let pick = json!({ "at": [0.5, 0.5, 1.0], "kind": "face", "dir": [0, 0, 1] });
    run(&mut p, "shape.update", json!({
        "target": part,
        "params": [{ "key": "w", "label": "Width", "value": 60 }, { "key": "push1", "label": "Push 1 distance", "value": -4 }],
        "finishes": [{ "kind": "push", "key": "push1", "edges": [pick] }]
    })).unwrap();
    let volume = |p: &Project| p.get::<ShapeSolid>(part).unwrap().stats.volume_mm3;
    assert!((volume(&p) - 60.0 * 40.0 * 6.0).abs() < 1e-3, "{}", volume(&p));
    // It follows a resize.
    run(&mut p, "shape.set_params", json!({ "target": part, "values": { "w": 100 } })).unwrap();
    assert!((volume(&p) - 100.0 * 40.0 * 6.0).abs() < 1e-3);
}

#[test]
fn a_face_finds_the_parameter_that_drives_it() {
    let program = de_shape::ShapeProgram {
        name: "Plate".into(),
        description: String::new(),
        source: "return box(p.w, p.d, p.t);".into(),
        params: serde_json::from_value(json!([
            { "key": "w", "label": "Width", "value": 100, "unit": "mm", "min": null, "max": null, "step": null },
            { "key": "d", "label": "Depth", "value": 60, "unit": "mm", "min": null, "max": null, "step": null },
            { "key": "t", "label": "Thickness", "value": 10, "unit": "mm", "min": null, "max": null, "step": null }
        ])).unwrap(),
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let top = de_shape::face_driver(&program, None, [50.0, 30.0, 10.0], [0.0, 0.0, 1.0]).unwrap();
    assert_eq!(top.key, "t");
    assert!((top.rate - 1.0).abs() < 1e-6, "{}", top.rate);
    let end = de_shape::face_driver(&program, None, [100.0, 30.0, 5.0], [1.0, 0.0, 0.0]).unwrap();
    assert_eq!(end.key, "w");
    // The bottom sits at z = 0 whatever the parameters: nothing drives it.
    assert!(de_shape::face_driver(&program, None, [50.0, 30.0, 0.0], [0.0, 0.0, -1.0]).is_none());
}

/// The gearbox examples, evaluated (involute helpers + the program).
fn example(dir: &str, file: &str) -> (String, Vec<(String, f64)>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples");
    let src = std::fs::read_to_string(root.join("gearbox/involute.js")).unwrap() + "\n" + &std::fs::read_to_string(root.join(dir).join(file)).unwrap();
    let steps: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join(dir).join("steps.json")).unwrap()).unwrap();
    let shape = steps.as_array().unwrap().iter().find(|s| s["operation"] == "shape.create").unwrap();
    let params = shape["parameters"]["params"].as_array().unwrap().iter().map(|p| (p["key"].as_str().unwrap().to_string(), p["value"].as_f64().unwrap())).collect();
    (src, params)
}

#[test]
fn gear_trains_turn_cleanly_through_a_full_turn() {
    // The machined gearbox: 18:54 then 18:45, so 7.5 : 1 overall.
    let (src, params) = example("gearbox", "gearbox.js");
    let e = runtime::evaluate(&src, &params, 0.05).unwrap();
    let finer = || runtime::evaluate(&src, &params, 0.0025);
    let c = de_shape::gears::check(&e, Some((1.0, 600.0)), Some(&finer)).unwrap();
    assert!(c.sweep.clashes.is_empty(), "clashes: {:?}", c.sweep.clashes);
    // A shaft runs in its bearing with the designed 0.01 mm: found finely, not called a clash.
    let fit = c.sweep.pairs.iter().find(|p| p.a == "Input shaft" && p.b.contains("bearing")).expect("the running fit is reported");
    assert!(fit.gap > 0.0 && fit.gap < 0.02, "{fit:?}");
    let out = c.speeds.iter().find(|(parts, _)| parts.contains(&"Output wheel".to_string())).unwrap().1;
    assert!((out - 1.0 / 7.5).abs() < 1e-6, "{out}");
    assert!(c.meshes.iter().all(|m| m.contact_ratio > 1.4), "{:?}", c.meshes.iter().map(|m| m.contact_ratio).collect::<Vec<_>>());
    assert!(c.sweep.steps >= 16 * 18, "each tooth passes in 16 steps or more");
    // Steel pinions on acetal wheels at 1 N·m: comfortably strong.
    assert!(c.meshes[0].strength.as_ref().unwrap().bending_safety.iter().all(|s| s.unwrap() > 2.0));

    // The same train with its axes pushed 2 mm too close: the sweep finds the teeth clashing.
    let tight: Vec<(String, f64)> = params.iter().map(|(k, v)| (k.clone(), if k == "backlash" { -2.0 } else { *v })).collect();
    if let Ok(e) = runtime::evaluate(&src, &tight, 0.05) {
        let finer = || runtime::evaluate(&src, &tight, 0.0025);
        let c = de_shape::gears::check(&e, None, Some(&finer)).unwrap();
        assert!(!c.sweep.clashes.is_empty(), "teeth thicker than their gaps must clash somewhere in the turn");
    }
}

#[test]
fn the_print_in_place_gearbox_turns_without_touching_its_housing() {
    let (src, params) = example("gearbox-print", "print-in-place.js");
    let e = runtime::evaluate(&src, &params, 0.05).unwrap();
    let c = de_shape::gears::check(&e, Some((0.3, 60.0)), None).unwrap();
    assert!(c.sweep.clashes.is_empty(), "clashes: {:?}", c.sweep.clashes);
    let smallest = c.sweep.pairs.first().map(|p| p.gap).unwrap_or(f64::INFINITY);
    assert!(smallest >= 0.25, "the printed gaps hold all the way round: {smallest} mm ({:?})", c.sweep.pairs.first());
    // PLA at 0.3 N·m on the knob: report the factors (design guidance).
    let s = c.meshes[0].strength.as_ref().unwrap();
    assert!(s.bending_safety[0].is_some() && s.materials[0].contains("PLA"));
}

#[test]
fn dragging_a_smooth_handle_swings_the_other_one() {
    // An arch: a base line and two Béziers meeting smoothly at the top point (5).
    let mut p = project();
    let geometry = json!({
        "points": [
            { "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 100, "y": 0 }, { "id": 5, "x": 50, "y": 80 },
            { "id": 6, "x": 100, "y": 40 }, { "id": 7, "x": 80, "y": 80 }, { "id": 8, "x": 20, "y": 80 }, { "id": 9, "x": 0, "y": 40 }
        ],
        "curves": [
            { "kind": "line", "id": 3, "a": 1, "b": 2 },
            { "kind": "bezier", "id": 10, "a": 2, "b": 5, "c1": 6, "c2": 7 },
            { "kind": "bezier", "id": 11, "a": 5, "b": 1, "c1": 8, "c2": 9 }
        ],
        "constraints": [{ "kind": "tangent", "a": 10, "b": 11, "at": 5 }]
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "Arch", "plane": { "name": "XY" }, "geometry": geometry.clone() })).unwrap());
    // Drag the right-hand top handle (7) up and out: it stays there, handle 8 swings to stay in line.
    run(&mut p, "sketch.update", json!({ "target": s, "geometry": geometry, "drag": { "point": 7, "x": 85, "y": 100 } })).unwrap();
    let sk = p.get::<de_shape::sketch::Sketch>(s).unwrap();
    let at = |id: u32| sk.geometry.points.iter().find(|q| q.id == id).map(|q| [q.x, q.y]).unwrap();
    let (h7, top, h8) = (at(7), at(5), at(8));
    assert!((h7[0] - 85.0).abs() < 1e-6 && (h7[1] - 100.0).abs() < 1e-6, "the dragged handle stays where it was put: {h7:?}");
    let cross = (h7[0] - top[0]) * (h8[1] - top[1]) - (h7[1] - top[1]) * (h8[0] - top[0]);
    assert!(cross.abs() < 1e-3, "the other handle swung round to keep the join smooth: {h8:?}");
    assert!((h8[1] - 80.0).abs() > 1.0, "it moved");
    assert!((top[0] - 50.0).abs() < 1e-6 && (top[1] - 80.0).abs() < 1e-6, "the curve turns about its anchor, which stays put: {top:?}");
}

#[test]
fn a_face_shared_with_another_outline_is_pushed_on_its_own() {
    // A rectangle and a circle in one sketch on the front plane, extruded together (one part).
    let mut p = project();
    let geometry = json!({
        "points": [{ "id": 1, "x": 60, "y": 60 }, { "id": 2, "x": 130, "y": 60 }, { "id": 3, "x": 130, "y": 30 }, { "id": 4, "x": 60, "y": 30 }, { "id": 9, "x": -80, "y": 60 }],
        "curves": [
            { "kind": "line", "id": 5, "a": 1, "b": 2 }, { "kind": "line", "id": 6, "a": 2, "b": 3 },
            { "kind": "line", "id": 7, "a": 3, "b": 4 }, { "kind": "line", "id": 8, "a": 4, "b": 1 },
            { "kind": "circle", "id": 10, "center": 9, "radius": 27.5 }
        ],
        "constraints": []
    });
    let plane = json!({ "origin": [0, 0, 0], "x": [1, 0, 0], "y": [0, 0, 1] });
    run(&mut p, "sketch.create", json!({ "name": "S", "plane": plane, "geometry": geometry })).unwrap();
    let part = id(run(&mut p, "shape.create", json!({
        "name": "S extrusion", "source": "return sketch(\"S\").extrude(p.depth);",
        "params": [{ "key": "depth", "label": "Depth", "value": 57 }]
    })).unwrap());
    let program = de_shape::resolved_program(p.world(), p.entity(part).unwrap()).unwrap();
    // The rectangle-only case is covered above (a_face_finds_the_parameter_that_drives_it).
    // The circle's far face: depth moves it, but it moves the rectangle too, so it is not this face's own.
    assert!(de_shape::face_driver(&program, None, [-80.0, -57.0, 60.0], [0.0, -1.0, 0.0]).is_none(), "depth also moves the rectangle");
    let volume = |p: &Project| p.get::<ShapeSolid>(part).unwrap().stats.volume_mm3;
    let before = volume(&p);
    let disc = std::f64::consts::PI * 27.5 * 27.5;
    // Bounds: x −107.5…130, y −57…0, z 30…87.5.
    let frac = |v: f64, lo: f64, hi: f64| (v - lo) / (hi - lo);
    let push = |p: &mut Project, at: [f64; 3], dir: [f64; 3], d: f64| {
        run(p, "shape.update", json!({
            "target": part,
            "params": [{ "key": "depth", "label": "Depth", "value": 57 }, { "key": "push1", "label": "Push 1 distance", "value": d }],
            "finishes": [{ "kind": "push", "key": "push1", "edges": [{ "at": at, "kind": "face", "dir": dir }] }]
        }))
    };
    // The circle's far face pushed out 10: only the cylinder grows.
    push(&mut p, [frac(-80.0, -107.5, 130.0), 0.0, frac(60.0, 30.0, 87.5)], [0.0, -1.0, 0.0], 10.0).unwrap();
    assert!((volume(&p) - before - disc * 10.0).abs() < disc * 0.02, "far face: {} vs {}", volume(&p) - before, disc * 10.0);
    // The circle's face on the sketch plane pushed out 10 the other way.
    push(&mut p, [frac(-80.0, -107.5, 130.0), 1.0, frac(60.0, 30.0, 87.5)], [0.0, 1.0, 0.0], 10.0).unwrap();
    assert!((volume(&p) - before - disc * 10.0).abs() < disc * 0.02, "near face: {} vs {}", volume(&p) - before, disc * 10.0);
}

/// The area inside a sketch's outlines (holes taken off), from its solved loops.
fn sketch_area(p: &Project, s: StableId) -> (f64, usize, Vec<de_shape::sketch::Loop>) {
    let sk = p.get::<de_shape::sketch::Sketch>(s).unwrap();
    let solved = de_shape::sketch::solve(&sk.geometry, &Default::default()).unwrap();
    let (loops, _) = de_shape::sketch::loops(&sk.geometry, &solved);
    let area = loops
        .iter()
        .map(|l| {
            let poly = de_shape::sketch::polygon(l, 1e-6);
            (0..poly.len()).map(|i| poly[i][0] * poly[(i + 1) % poly.len()][1] - poly[(i + 1) % poly.len()][0] * poly[i][1]).sum::<f64>() / 2.0
        })
        .sum();
    (area, loops.len(), loops)
}

fn rect(id: u32, x0: f64, y0: f64, x1: f64, y1: f64) -> (Vec<Value>, Vec<Value>) {
    let pts = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let points = pts.iter().enumerate().map(|(i, p)| json!({ "id": id + i as u32, "x": p[0], "y": p[1] })).collect();
    let lines = (0..4).map(|i| json!({ "kind": "line", "id": id + 4 + i, "a": id + i, "b": id + (i + 1) % 4 })).collect();
    (points, lines)
}

#[test]
fn pathfinder_unites_subtracts_and_intersects_outlines_exactly() {
    let pi = std::f64::consts::PI;
    let make = |p: &mut Project, extra: Vec<(Vec<Value>, Vec<Value>)>| {
        let (mut points, mut curves) = (vec![], vec![]);
        for (ps, cs) in extra {
            points.extend(ps);
            curves.extend(cs);
        }
        id(run(p, "sketch.create", json!({ "name": "P", "plane": { "name": "XY" }, "geometry": { "points": points, "curves": curves, "constraints": [] } })).unwrap())
    };
    // Two 40 × 40 squares overlapping by 20 × 20.
    let squares = || vec![rect(1, 0.0, 0.0, 40.0, 40.0), rect(20, 20.0, 20.0, 60.0, 60.0)];
    let mut p = project();
    let s = make(&mut p, squares());
    let r = run(&mut p, "sketch.combine", json!({ "target": s, "op": "unite", "curves": [5, 24] })).unwrap();
    let (area, n, loops) = sketch_area(&p, s);
    assert_eq!(n, 1, "one outline");
    assert!((area - (1600.0 + 1600.0 - 400.0)).abs() < 1e-6, "unite: {area}");
    assert_eq!(loops[0].len(), 8, "eight straight sides");
    let made = r.get("curves").or_else(|| r.pointer("/output/curves")).and_then(Value::as_array).map(Vec::len);
    assert_eq!(made, Some(8), "the new curves are returned (to select them): {r}");

    let mut p = project();
    let s = make(&mut p, squares());
    run(&mut p, "sketch.combine", json!({ "target": s, "op": "intersect", "curves": [5, 24] })).unwrap();
    let (area, n, _) = sketch_area(&p, s);
    assert!(n == 1 && (area - 400.0).abs() < 1e-6, "intersect: {n} {area}");

    // A 60 × 40 plate minus a circle of radius 10 inside it: a hole; minus one over its edge: a bite.
    let mut p = project();
    let circle = |id: u32, x: f64, y: f64, r: f64| (vec![json!({ "id": id, "x": x, "y": y })], vec![json!({ "kind": "circle", "id": id + 1, "center": id, "radius": r })]);
    let s = make(&mut p, vec![rect(1, 0.0, 0.0, 60.0, 40.0), circle(30, 20.0, 20.0, 10.0)]);
    run(&mut p, "sketch.combine", json!({ "target": s, "op": "subtract", "curves": [5, 31] })).unwrap();
    let (area, n, _) = sketch_area(&p, s);
    assert!(n == 2 && (area - (2400.0 - pi * 100.0)).abs() < 1e-3, "subtract a hole: {n} {area}");
    let mut p = project();
    let s = make(&mut p, vec![rect(1, 0.0, 0.0, 60.0, 40.0), circle(30, 60.0, 20.0, 10.0)]);
    run(&mut p, "sketch.combine", json!({ "target": s, "op": "subtract", "curves": [5, 31] })).unwrap();
    let (area, n, loops) = sketch_area(&p, s);
    assert!(n == 1 && (area - (2400.0 - pi * 50.0)).abs() < 1e-3, "subtract a bite: {n} {area}");
    assert!(loops[0].iter().any(|seg| matches!(seg, de_shape::sketch::Segment::Arc { .. })), "the bite is a true arc");
}

#[test]
fn offset_adds_an_outline_round_or_sharp() {
    let pi = std::f64::consts::PI;
    let (points, curves) = rect(1, 0.0, 0.0, 60.0, 40.0);
    for (corners, want) in [("round", 70.0 * 50.0 - (4.0 - pi) * 25.0), ("sharp", 70.0 * 50.0)] {
        let mut p = project();
        let s = id(run(&mut p, "sketch.create", json!({ "name": "O", "plane": { "name": "XY" }, "geometry": { "points": points, "curves": curves, "constraints": [] } })).unwrap());
        run(&mut p, "sketch.offset", json!({ "target": s, "curves": [5], "distance": 5, "corners": corners })).unwrap();
        // The original stays, inside the new outline.
        let sk = p.get::<de_shape::sketch::Sketch>(s).unwrap();
        let solved = de_shape::sketch::solve(&sk.geometry, &Default::default()).unwrap();
        let (loops, _) = de_shape::sketch::loops(&sk.geometry, &solved);
        assert_eq!(loops.len(), 2, "{corners}: the original and its offset");
        let big = loops
            .iter()
            .map(|l| {
                let poly = de_shape::sketch::polygon(l, 1e-6);
                ((0..poly.len()).map(|i| poly[i][0] * poly[(i + 1) % poly.len()][1] - poly[(i + 1) % poly.len()][0] * poly[i][1]).sum::<f64>() / 2.0).abs()
            })
            .fold(0.0, f64::max);
        assert!((big - want).abs() < 1e-3, "{corners}: {big} vs {want}");
    }
    // A closed Bézier blob offset outwards comes back as Béziers that close.
    let mut p = project();
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 40, "y": 0 }, { "id": 3, "x": 10, "y": -20 }, { "id": 4, "x": 30, "y": -20 }, { "id": 5, "x": 30, "y": 25 }, { "id": 6, "x": 10, "y": 25 }],
        "curves": [{ "kind": "bezier", "id": 7, "a": 1, "b": 2, "c1": 3, "c2": 4 }, { "kind": "bezier", "id": 8, "a": 2, "b": 1, "c1": 5, "c2": 6 }],
        "constraints": []
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "B", "plane": { "name": "XY" }, "geometry": g })).unwrap());
    run(&mut p, "sketch.offset", json!({ "target": s, "curves": [7], "distance": 3 })).unwrap();
    let (_, n, loops) = sketch_area(&p, s);
    assert_eq!(n, 2, "the blob and its offset");
    assert!(loops.iter().any(|l| l.iter().all(|seg| matches!(seg, de_shape::sketch::Segment::Bezier { .. } | de_shape::sketch::Segment::Arc { .. })) && l.len() > 2), "offset of curves is curves");
}

#[test]
fn a_drawing_has_views_hidden_lines_dimensions_and_exports() {
    let program = de_shape::ShapeProgram {
        name: "Plate".into(),
        description: String::new(),
        source: "return box(100, 60, 20).subtract(cylinder(30, 8).translate([50, 30, -5]));".into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let title = de_shape::drawing::Title { title: "Plate".into(), number: "P-001".into(), material: "Aluminium".into(), mass: "300 g".into(), drawn_by: "Forma".into(), date: "2026-10-10".into(), parts: vec![] };
    for projection in [de_shape::drawing::Projection::First, de_shape::drawing::Projection::Third] {
        let d = de_shape::drawing::drawing(&program, &de_shape::drawing::Options { sheet: de_shape::drawing::Sheet::A4, projection, part: None, ..Default::default() }, &title).unwrap();
        assert_eq!(d.scale_text, "1:1", "100 mm fits an A4 at full size");
        // The hole shows as hidden lines in the front and side views.
        let hidden = d.strokes.iter().filter(|s| s.pen == de_shape::drawing::Pen::Hidden).count();
        assert!(hidden >= 4, "{hidden} hidden lines");
        // The three overall sizes are dimensioned.
        for v in ["100", "60", "20"] {
            assert!(d.texts.iter().any(|t| t.text == v), "dimension {v}");
        }
        assert!(d.texts.iter().any(|t| t.text == "Plate" && t.bold), "the title");
        assert!(d.texts.iter().any(|t| t.text == "⌀16"), "the hole is called out");
        assert!(d.strokes.iter().filter(|s| s.pen == de_shape::drawing::Pen::Center).count() >= 2, "and has centre lines");
        // Everything inside the sheet.
        assert!(d.strokes.iter().flat_map(|s| &s.points).all(|p| p[0] >= 0.0 && p[0] <= d.width && p[1] >= 0.0 && p[1] <= d.height));
        let pdf = de_shape::drawing::pdf(&d);
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&pdf);
        let xref: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert_eq!(&pdf[xref..xref + 4], b"xref", "the cross-reference table is where the trailer says");
        let dxf = de_shape::drawing::dxf(&d);
        assert!(dxf.contains("HIDDEN") && dxf.trim_end().ends_with("EOF"));
        // DRAWING_OUT=dir writes the files to look at.
        if let Ok(dir) = std::env::var("DRAWING_OUT") {
            std::fs::write(format!("{dir}/plate-{projection:?}.pdf"), &pdf).unwrap();
            std::fs::write(format!("{dir}/plate-{projection:?}.dxf"), &dxf).unwrap();
        }
    }
}

#[test]
fn a_drawing_sections_dimensions_and_lists_parts() {
    use de_shape::drawing::{DimKind, DimSpec, Options, PartRow, Pen, Sheet, Title, ViewName};
    // Plate 100 × 60 × 20 with a ⌀16 hole through it at (50, 30).
    let program = de_shape::ShapeProgram {
        name: "Plate".into(),
        description: String::new(),
        source: "return box(100, 60, 20).subtract(cylinder(30, 8).translate([50, 30, -5]));".into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let title = |parts: Vec<PartRow>| Title { title: "Plate".into(), number: "P-1".into(), material: "Steel".into(), mass: "1 kg".into(), drawn_by: "F".into(), date: "2026-10-10".into(), parts };
    let plain = de_shape::drawing::drawing(&program, &Options { sheet: Sheet::A4, ..Default::default() }, &title(vec![])).unwrap();
    // The view frames say where the model lands on the sheet: the front view's
    // outline is the part's x and z extent.
    let front = plain.views.iter().find(|v| v.view == ViewName::Front).unwrap();
    let want = [front.at[0], front.at[1] + 0.0, front.at[0] + 100.0 * front.scale, front.at[1] + 20.0 * front.scale];
    for k in 0..4 {
        assert!((front.bounds[k] - want[k]).abs() < 0.05, "front view bounds {:?} vs {want:?}", front.bounds);
    }
    let top = plain.views.iter().find(|v| v.view == ViewName::Top).unwrap();
    assert!((top.bounds[3] - top.bounds[1] - 60.0 * top.scale).abs() < 0.05, "top view is 60 deep");

    // Section through the hole's centre (half way back): hatching on the
    // material either side of the hole, none across the hole.
    let dims = vec![
        DimSpec { view: ViewName::Top, a: [50.0, 30.0], b: [100.0, 30.0], kind: DimKind::Horizontal, offset: 12.0 },
        DimSpec { view: ViewName::Front, a: [0.0, 0.0], b: [100.0, 20.0], kind: DimKind::Aligned, offset: 8.0 },
    ];
    let cut = de_shape::drawing::drawing(&program, &Options { sheet: Sheet::A4, section: Some(50.0), dimensions: dims, ..Default::default() }, &title(vec![])).unwrap();
    let f = cut.views.iter().find(|v| v.view == ViewName::Front).unwrap();
    let hatch: Vec<_> = cut.strokes.iter().filter(|s| s.pen == Pen::Thin && s.points.len() == 2).filter(|s| s.points.iter().all(|p| p[0] >= f.bounds[0] - 0.01 && p[0] <= f.bounds[2] + 0.01 && p[1] >= f.bounds[1] - 0.01 && p[1] <= f.bounds[3] + 0.01)).filter(|s| ((s.points[1][1] - s.points[0][1]) - (s.points[1][0] - s.points[0][0])).abs() < 1e-6).collect();
    assert!(hatch.len() > 20, "{} hatch lines", hatch.len());
    let hole = (f.at[0] + 42.0 * f.scale + 0.1, f.at[0] + 58.0 * f.scale - 0.1);
    for s in &hatch {
        let (x0, x1) = (s.points[0][0].min(s.points[1][0]), s.points[0][0].max(s.points[1][0]));
        assert!(x1 <= hole.0 || x0 >= hole.1, "a hatch line crosses the hole: {:?}", s.points);
    }
    assert!(cut.strokes.iter().all(|s| s.pen != Pen::Hidden || !s.points.iter().all(|p| p[1] >= f.bounds[1] && p[1] <= f.bounds[3] && p[0] >= f.bounds[0] && p[0] <= f.bounds[2])), "no hidden lines in the section");
    assert!(cut.texts.iter().any(|t| t.text == "SECTION A–A"));
    assert_eq!(cut.texts.iter().filter(|t| t.text == "A").count(), 2, "the cutting plane is lettered at both ends");
    // The person's dimensions: 50 across the top view, the front diagonal √(100² + 20²).
    assert!(cut.texts.iter().any(|t| t.text == "50"), "added 50");
    let diag = de_shape::drawing::length_text(100f64.hypot(20.0));
    let t = cut.texts.iter().find(|t| t.text == diag).expect("aligned dimension");
    assert!((t.angle - 20f64.atan2(100.0).to_degrees()).abs() < 0.01, "it runs along the diagonal: {}°", t.angle);

    // A parts list for an assembly.
    let rows = vec![PartRow { name: "Base".into(), quantity: 1, material: "Steel".into(), mass: "1.2 kg".into() }, PartRow { name: "Bolt".into(), quantity: 4, material: "Steel".into(), mass: "12 g".into() }];
    let listed = de_shape::drawing::drawing(&program, &Options { sheet: Sheet::A4, ..Default::default() }, &title(rows)).unwrap();
    for v in ["ITEM", "QTY", "Base", "Bolt", "4", "12 g"] {
        assert!(listed.texts.iter().any(|t| t.text == v), "parts list shows {v}");
    }
    assert!(listed.strokes.iter().flat_map(|s| &s.points).all(|p| p[0] >= 0.0 && p[0] <= listed.width && p[1] >= 0.0 && p[1] <= listed.height));
    if let Ok(dir) = std::env::var("DRAWING_OUT") {
        std::fs::write(format!("{dir}/plate-section.pdf"), de_shape::drawing::pdf(&cut)).unwrap();
        std::fs::write(format!("{dir}/plate-list.pdf"), de_shape::drawing::pdf(&listed)).unwrap();
    }
}

#[test]
fn a_slider_crank_moves_exactly_as_the_textbook_says() {
    // Crank 30 mm about y at the origin, rod 100 mm, piston sliding along x.
    let (r, l) = (30.0, 100.0);
    let source = r#"
        joint("hinge", "Crank", "ground", { at: [0, 0, 0], axis: [0, 1, 0], name: "Main bearing" });
        joint("hinge", "Crank", "Rod", { at: [30, 0, 0], axis: [0, 1, 0], name: "Crank pin" });
        joint("hinge", "Rod", "Piston", { at: [130, 0, 0], axis: [0, 1, 0], name: "Gudgeon pin" });
        joint("slider", "Piston", "ground", { at: [130, 0, 0], axis: [1, 0, 0], name: "Bore" });
        drive("Main bearing", { speed: 3000 });
        return [
            part("Crank", box(40, 10, 10).translate([-5, -5, -5])),
            part("Rod", box(100, 6, 6).translate([30, 6, -3])),
            part("Piston", cylinder(20, 12).rotate([0, 90, 0]).translate([120, 0, 0])),
            part("Block", box(10, 10, 10).translate([-40, 30, -5])),
        ];
    "#;
    let program = de_shape::ShapeProgram {
        name: "Slider crank".into(),
        description: String::new(),
        source: source.into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let sweep = de_shape::motion_sweep(&program, 72).unwrap();
    assert_eq!(sweep.freedom, 1, "one way to move: the crank's turn ({:?})", sweep.notes);
    assert!(sweep.binds_at.is_none(), "{:?}", sweep.notes);
    assert_eq!(sweep.frames.len(), 73, "every 5° of a full turn");
    let bore = sweep.joints.iter().position(|j| j.name == "Bore").unwrap();
    for f in &sweep.frames {
        let th = f.drive.to_radians();
        let x = r * th.cos() + (l * l - (r * th.sin()).powi(2)).sqrt();
        assert!((f.joints[bore] - (x - (r + l))).abs() < 1e-6, "at {}°: {} vs {}", f.drive, f.joints[bore], x - (r + l));
    }
    // Top and bottom dead centre: the stroke is twice the crank.
    let travel = sweep.frames.iter().map(|f| f.joints[bore]).fold(f64::INFINITY, f64::min);
    assert!((travel + 2.0 * r).abs() < 1e-6, "stroke {}", -travel);
}

#[test]
fn a_four_bar_that_cannot_turn_round_is_reported_as_binding() {
    // Ground 100, crank 80, coupler 30, rocker 40: the crank cannot make a full turn (not Grashof).
    let source = r#"
        joint("hinge", "Crank", "ground", { at: [0, 0, 0], axis: [0, 0, 1], name: "A" });
        joint("hinge", "Crank", "Coupler", { at: [80, 0, 0], axis: [0, 0, 1] });
        // The coupler (30) meets the rocker (40) whose pivot is at (100, 0).
        joint("hinge", "Coupler", "Rocker", { at: [80 + 30 * Math.cos(1.0), 30 * Math.sin(1.0), 0], axis: [0, 0, 1] });
        joint("hinge", "Rocker", "ground", { at: [100, 0, 0], axis: [0, 0, 1] });
        drive("A");
        return [part("Crank", box(80, 4, 4)), part("Coupler", box(30, 4, 4).translate([80, 0, 5])), part("Rocker", box(4, 40, 4).translate([100, 0, 10]))];
    "#;
    let program = de_shape::ShapeProgram {
        name: "Four bar".into(),
        description: String::new(),
        source: source.into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let sweep = de_shape::motion_sweep(&program, 72).unwrap();
    assert!(sweep.binds_at.is_some() || sweep.notes.iter().any(|n| n.contains("cannot")), "{:?}", sweep.notes);
}

fn linkage_program(source: &str) -> de_shape::ShapeProgram {
    de_shape::ShapeProgram {
        name: "Linkage".into(),
        description: String::new(),
        source: source.into(),
        params: vec![],
        display_tolerance: Some(0.01),
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    }
}

#[test]
fn an_unbalanced_crank_loads_its_bearing_by_m_e_omega_squared() {
    // A disc whose centre is 30 mm off the axis, spun at 600 rpm.
    let program = linkage_program(
        r#"
        joint("hinge", "Crank", "ground", { at: [0, 0, 0], axis: [0, 1, 0], name: "Bearing" });
        drive("Bearing", { speed: 600 });
        return [part("Crank", cylinder(10, 20).rotate(-90, 0, 0).translate(30, -5, 0)), part("Stand", box(10, 10, 10).translate(-5, 30, -60))];
    "#,
    );
    let m = 0.2; // kg
    let w = 600.0 * std::f64::consts::TAU / 60.0;
    let still = de_shape::motion_with_forces(&program, 72, Some(&[Some(m), Some(1.0)]), false).unwrap();
    let f = still.forces.as_ref().expect("forces with masses");
    let want = m * 0.030 * w * w;
    for (k, load) in f.joints[0].iter().enumerate() {
        assert!((load - want).abs() < want * 1e-3, "frame {k}: {load} N vs m·e·ω² = {want} N");
        assert!(f.drive[k].abs() < want * 1e-3 * 0.03, "no torque at steady speed without gravity: {}", f.drive[k]);
    }
    // With gravity the motor lifts and lowers the offset weight: ±m·g·e.
    let heavy = de_shape::motion_with_forces(&program, 72, Some(&[Some(m), Some(1.0)]), true).unwrap();
    let t = &heavy.forces.as_ref().unwrap().drive;
    let peak = t.iter().fold(0.0f64, |a, v| a.max(v.abs()));
    assert!((peak - m * 9.81 * 0.030).abs() < 1e-4, "peak torque {peak} N·m vs m·g·e = {}", m * 9.81 * 0.030);
    assert!(heavy.forces.as_ref().unwrap().mean_power.abs() < 1e-3, "no net work over a turn");
}

#[test]
fn a_piston_takes_the_torque_the_energy_balance_says() {
    // Slider-crank (r 30, rod 100) with all the mass in a 0.5 kg piston, 3000 rpm, no gravity.
    let program = linkage_program(
        r#"
        joint("hinge", "Crank", "ground", { at: [0, 0, 0], axis: [0, 1, 0], name: "Main bearing" });
        joint("hinge", "Crank", "Rod", { at: [30, 0, 0], axis: [0, 1, 0] });
        joint("hinge", "Rod", "Piston", { at: [130, 0, 0], axis: [0, 1, 0] });
        joint("slider", "Piston", "ground", { at: [130, 0, 0], axis: [1, 0, 0], name: "Bore" });
        drive("Main bearing", { speed: 3000 });
        return [
            part("Crank", box(40, 10, 10).translate(-5, -5, -5)),
            part("Rod", box(100, 6, 6).translate(30, 6, -3)),
            part("Piston", cylinder(20, 12).rotate(0, 90, 0).translate(120, 0, 0)),
            part("Block", box(10, 10, 10).translate(-40, 30, -5)),
        ];
    "#,
    );
    let m = 0.5;
    let sweep = de_shape::motion_with_forces(&program, 360, Some(&[Some(1e-9), Some(1e-9), Some(m), Some(1.0)]), false).unwrap();
    let f = sweep.forces.as_ref().unwrap();
    let w = 3000.0 * std::f64::consts::TAU / 60.0;
    let bore = sweep.joints.iter().position(|j| j.name == "Bore").unwrap();
    let x: Vec<f64> = sweep.frames.iter().map(|fr| fr.joints[bore] / 1000.0).collect(); // m
    let h = 1f64.to_radians();
    let n = x.len() - 1; // the last frame repeats the first
    let mut worst: f64 = 0.0;
    let mut peak: f64 = 0.0;
    for k in 0..n {
        let (a, b) = (x[(k + n - 1) % n], x[(k + 1) % n]);
        let dx = (b - a) / (2.0 * h);
        let acc = (a - 2.0 * x[k] + b) / (h * h) * w * w;
        // Power balance: T·ω = m·a·v  →  T = m·a·dx/dθ.
        let t = m * acc * dx;
        worst = worst.max((f.drive[k] - t).abs());
        peak = peak.max(t.abs());
    }
    assert!(worst < peak * 2e-3, "torque off by {worst} N·m (peak {peak} N·m)");
    assert!(f.mean_power.abs() < peak * w * 1e-3, "mean power {} W", f.mean_power);
}

#[test]
fn a_cantilever_bends_and_is_stressed_as_beam_theory_says() {
    // Steel, 200 × 20 × 20 mm, held at x = 0, 1000 N down on the free end.
    let e = runtime::evaluate("return box(200, 20, 20);", &[], 0.01).unwrap();
    let face = |x: f64, n: f64| de_shape::fea::Face {
        point: [x, 10.0, 10.0],
        normal: [n, 0.0, 0.0],
        triangles: vec![x, 0.0, 0.0, x, 20.0, 0.0, x, 20.0, 20.0, x, 0.0, 0.0, x, 20.0, 20.0, x, 0.0, 20.0],
    };
    let study = de_shape::fea::Study { supports: vec![face(0.0, -1.0)], loads: vec![de_shape::fea::Load { face: face(200.0, 1.0), force: [0.0, 0.0, -1000.0] }], bore_supports: vec![], bore_loads: vec![], resolution: 80 };
    let steel = de_shape::fea::elastic("Steel").unwrap();
    let r = de_shape::fea::solve(&e.positions, &e.indices, &steel, &study).unwrap();
    // Tip deflection: PL³/3EI (1.000 mm) plus shear PL/κGA (0.008 mm).
    let i = 20.0f64.powi(4) / 12.0;
    let bending = 1000.0 * 200f64.powi(3) / (3.0 * 200_000.0 * i);
    let g = 200_000.0 / (2.0 * 1.29);
    let want = bending + 1000.0 * 200.0 / (5.0 / 6.0 * g * 400.0);
    let tip = de_shape::fea::displacement_at(&r, [200.0, 10.0, 10.0]).unwrap();
    assert!((-tip[2] - want).abs() < want * 0.03, "tip {} mm vs {want:.4} mm ({} iterations)", -tip[2], r.iterations);
    // Bending stress at a cube centre near mid-span, top fibre: M·y/I.
    let cell = r.cell;
    let (x, z) = (100.0 + cell / 2.0, 20.0 - cell / 2.0);
    let sigma = 1000.0 * (200.0 - x) * (z - 10.0) / i;
    let got = de_shape::fea::stress_at(&r, [x, 10.0 - cell / 2.0, z]).unwrap();
    assert!((got - sigma).abs() < sigma * 0.04, "stress {got} MPa vs {sigma:.1} MPa");
    assert!(r.safety > 1.0 && r.safety < 3.0, "safety {}", r.safety);
    eprintln!("cantilever: tip {:.4} mm (theory {want:.4}), stress {got:.2} MPa (theory {sigma:.2}), {} cubes, {} iterations", -tip[2], r.elements, r.iterations);
}

#[test]
fn a_cantilever_rings_at_the_frequency_beam_theory_says() {
    // Steel 200 × 20 × 20 mm held at one end: f₁ = (1.875²/2π)·√(EI/ρAL⁴) ≈ 407.7 Hz, twice (square section).
    let e = runtime::evaluate("return box(200, 20, 20);", &[], 0.01).unwrap();
    let held = de_shape::fea::Face { point: [0.0, 10.0, 10.0], normal: [-1.0, 0.0, 0.0], triangles: vec![0.0, 0.0, 0.0, 0.0, 20.0, 0.0, 0.0, 20.0, 20.0, 0.0, 0.0, 0.0, 0.0, 20.0, 20.0, 0.0, 0.0, 20.0] };
    let steel = de_shape::fea::elastic("Steel").unwrap();
    let m = de_shape::fea::modes(&e.positions, &e.indices, &steel, 7850.0, &[held], 3, 60).unwrap();
    let (ei, rho_a, l) = (200e9 * 0.02f64.powi(4) / 12.0, 7850.0 * 0.0004, 0.2f64);
    let f1 = 1.875f64.powi(2) / std::f64::consts::TAU * (ei / (rho_a * l.powi(4))).sqrt();
    eprintln!("cantilever modes: {:?} Hz (theory {f1:.1})", m.modes.iter().map(|x| (x.frequency * 10.0).round() / 10.0).collect::<Vec<_>>());
    for k in 0..2 {
        assert!((m.modes[k].frequency - f1).abs() < f1 * 0.03, "mode {}: {} Hz vs {f1:.1} Hz", k + 1, m.modes[k].frequency);
    }
    assert_ne!(m.modes[0].direction, "x", "the first modes sway sideways, not along the beam");
    assert!(m.modes[2].frequency > f1 * 1.5, "the next mode is well above");
}

#[test]
fn a_saved_study_finds_its_faces_again_after_the_part_changes() {
    let mut p = project();
    let part = id(run(&mut p, "shape.create", json!({
        "name": "Beam", "source": "return box(p.l, 20, 20);",
        "params": [{ "key": "l", "label": "Length", "value": 200 }]
    })).unwrap());
    // Held at the x = 0 end, loaded on the far end (fractions of the bounds; probe on the face).
    let study = json!({
        "part": null,
        "supports": [{ "at": [0.0, 0.5, 0.5], "probe": [0.0, 0.3, 0.3], "normal": [-1.0, 0.0, 0.0] }],
        "loads": [{ "face": { "at": [1.0, 0.5, 0.5], "probe": [1.0, 0.7, 0.3], "normal": [1.0, 0.0, 0.0] }, "magnitude": 1000.0, "direction": "-z" }],
        "resolution": 48
    });
    run(&mut p, "shape.set_study", json!({ "target": part, "study": study })).unwrap();
    let saved = p.get::<de_shape::ShapeStudies>(part).unwrap().0[0].clone();
    assert_eq!(saved.loads[0].magnitude, 1000.0);
    // Longer beam: the far face moves to x = 300 and is still found.
    run(&mut p, "shape.set_params", json!({ "target": part, "values": { "l": 300 } })).unwrap();
    let program = de_shape::resolved_program(p.world(), p.entity(part).unwrap()).unwrap();
    let far = de_shape::resolve_face(&program, None, &saved.loads[0].face).expect("the far end is found");
    assert!((far.point[0] - 300.0).abs() < 1e-6, "{:?}", far.point);
    assert!(far.triangles.chunks_exact(3).all(|v| (v[0] - 300.0).abs() < 1e-6), "every triangle on the end face");
    let area: f64 = far.triangles.chunks_exact(9).map(|t| {
        let (a, b, c) = ([t[0], t[1], t[2]], [t[3], t[4], t[5]], [t[6], t[7], t[8]]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
    }).sum();
    assert!((area - 400.0).abs() < 1e-6, "the whole 20 × 20 face: {area}");
    assert_eq!(saved.loads[0].force(far.normal), [0.0, 0.0, -1000.0]);
    // Clearing both lists removes the study.
    run(&mut p, "shape.set_study", json!({ "target": part, "study": { "part": null, "supports": [], "loads": [] } })).unwrap();
    assert!(p.get::<de_shape::ShapeStudies>(part).is_none());
}

#[test]
fn a_connecting_rod_is_checked_at_its_hardest_moment() {
    // Slider-crank, r 30, rod 100 (10 × 8 shank between two eyes), all the mass in a 0.5 kg piston, 3000 rpm.
    let program = linkage_program(
        r#"
        joint("hinge", "Crank", "ground", { at: [0, 0, 0], axis: [0, 1, 0], name: "Main bearing" });
        joint("hinge", "Rod", "Crank", { at: [30, 0, 0], axis: [0, 1, 0], name: "Big end" });
        joint("hinge", "Rod", "Piston", { at: [130, 0, 0], axis: [0, 1, 0], name: "Small end" });
        joint("slider", "Piston", "ground", { at: [130, 0, 0], axis: [1, 0, 0], name: "Bore" });
        drive("Main bearing", { speed: 3000 });
        const eye = (x) => cylinder(8, 9).rotate(-90, 0, 0).translate(x, -4, 0).subtract(cylinder(10, 5).rotate(-90, 0, 0).translate(x, -5, 0));
        const rod = box(80, 8, 10).translate(40, -4, -5).add(eye(30), eye(130));
        return [
            part("Crank", box(40, 10, 10).translate(-5, 10, -5)),
            part("Rod", rod, { material: "Steel" }),
            part("Piston", cylinder(20, 12).rotate(0, 90, 0).translate(120, 20, 0)),
            part("Block", box(10, 10, 10).translate(-40, 40, -5)),
        ];
    "#,
    );
    let r = de_shape::motion_stress(&program, "Rod", &[Some(1e-9), Some(1e-9), Some(0.5), Some(1.0)], "Steel").unwrap();
    let w = 3000.0 * std::f64::consts::TAU / 60.0;
    let peak = 0.5 * 0.030 * w * w * (1.0 + 0.3);
    eprintln!("rod: at {}°, held by {}, loads {:?}, max {:.1} MPa, safety {:.2}", r.at, r.held_by, r.loads, r.result.max_stress, r.result.safety);
    assert!(r.at.rem_euclid(360.0) < 1.0 || r.at.rem_euclid(360.0) > 359.0, "top dead centre: {}", r.at);
    assert_eq!(r.held_by, "Big end");
    assert_eq!(r.loads[0].0, "Small end");
    assert!((r.loads[0].1 - peak).abs() < peak * 0.01, "{} N vs m·r·ω²(1 + r/L) = {peak:.0} N", r.loads[0].1);
    // Mid-shank: the rod is pulled straight, so σ = F/A.
    let mid = de_shape::fea::stress_at(&r.result, [80.0, 0.0, 0.0]).unwrap();
    let want = peak / 80.0;
    eprintln!("rod mid-shank {mid:.2} MPa vs F/A {want:.2} MPa");
    assert!((mid - want).abs() < want * 0.1, "mid-shank {mid:.1} MPa vs F/A = {want:.1} MPa");
}

fn end_face(x: f64, n: f64, s: f64) -> de_shape::fea::Face {
    de_shape::fea::Face { point: [x, s / 2.0, s / 2.0], normal: [n, 0.0, 0.0], triangles: vec![x, 0.0, 0.0, x, s, 0.0, x, s, s, x, 0.0, 0.0, x, s, s, x, 0.0, s] }
}

#[test]
fn heat_flows_along_a_bar_as_fouriers_law_says() {
    // Aluminium 100 × 10 × 10 mm, held at 20 °C at x = 0, 10 W in at x = 100: T(x) = 20 + Q·x/(k·A).
    let e = runtime::evaluate("return box(100, 10, 10);", &[], 0.01).unwrap();
    let k = de_shape::fea::conductivity("Aluminium").unwrap();
    let study = de_shape::fea::ThermalStudy {
        holds: vec![de_shape::fea::Hold { face: end_face(0.0, -1.0, 10.0), temperature: 20.0 }],
        heats: vec![de_shape::fea::Heat { face: end_face(100.0, 1.0, 10.0), watts: 10.0 }],
        convection: None,
        resolution: 50,
    };
    let r = de_shape::fea::thermal(&e.positions, &e.indices, k, &study).unwrap();
    let at = |x: f64| 20.0 + 10.0 * (x / 1000.0) / (k * 1e-4);
    for x in [25.0, 51.0, 77.0] {
        let x = (x / r.cell).floor() * r.cell + r.cell / 2.0;
        let got = de_shape::fea::temperature_at(&r, [x, 5.0, 5.0]).unwrap();
        assert!((got - at(x)).abs() < (at(x) - 20.0) * 0.02 + 0.05, "at x = {x}: {got:.2} °C vs {:.2} °C", at(x));
    }
    assert!((r.max - at(100.0)).abs() < (at(100.0) - 20.0) * 0.02, "hottest {:.2} vs {:.2}", r.max, at(100.0));
    assert!((r.to_holds - 10.0).abs() < 1e-3, "all 10 W leave through the held end: {}", r.to_holds);
}

#[test]
fn a_block_in_still_air_settles_where_its_surface_sheds_the_heat() {
    // Copper 50 mm cube, 5 W in on one face, still air (h = 10 W/m²K) at 20 °C on all six faces:
    // conduction is quick, so T ≈ 20 + Q/(h·A), A = 6 × 2500 mm².
    let e = runtime::evaluate("return box(50, 50, 50);", &[], 0.01).unwrap();
    let k = de_shape::fea::conductivity("Copper").unwrap();
    let study = de_shape::fea::ThermalStudy {
        holds: vec![],
        heats: vec![de_shape::fea::Heat { face: end_face(50.0, 1.0, 50.0), watts: 5.0 }],
        convection: Some(de_shape::fea::Convection { h: 10.0, ambient: 20.0 }),
        resolution: 24,
    };
    let r = de_shape::fea::thermal(&e.positions, &e.indices, k, &study).unwrap();
    let want = 20.0 + 5.0 / (10.0 * 6.0 * 0.05 * 0.05);
    let mean = r.cells.iter().map(|(_, t)| *t as f64).sum::<f64>() / r.cells.len() as f64;
    eprintln!("block: mean {mean:.2} °C (want {want:.2}), spread {:.3}, to air {:.4} W", r.max - r.min, r.to_air);
    assert!((mean - want).abs() < 0.2, "mean {mean:.2} vs {want:.2}");
    assert!(r.max - r.min < 0.5, "copper evens it out");
    assert!((r.to_air - 5.0).abs() < 1e-3, "all 5 W go to the air: {}", r.to_air);
}

#[test]
fn a_batch_is_one_change_and_one_undo_or_nothing_at_all() {
    let mut p = project();
    let shape = |name: &str| json!({ "operation": "shape.create", "parameters": { "name": name, "source": "return box(10, 10, 10);", "params": [] } });
    let out = run(&mut p, "core.batch", json!({ "steps": [shape("A"), shape("B")] })).unwrap();
    let ids: Vec<StableId> = out["outputs"].as_array().or_else(|| out.pointer("/output/outputs").and_then(Value::as_array)).unwrap().iter().map(|o| serde_json::from_value(o["id"].clone()).unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| p.get::<de_shape::ShapeProgram>(*id).is_some()));
    // One undo takes both back.
    p.undo(&Actor { user_id: "u".into(), session_id: "s".into() }).unwrap();
    assert!(ids.iter().all(|id| p.get::<de_shape::ShapeProgram>(*id).is_none()));
    // A failing step: none of the batch happens.
    let before = p.revision();
    let bad = json!({ "operation": "shape.set_params", "parameters": { "target": StableId::new(), "values": {} } });
    assert!(run(&mut p, "core.batch", json!({ "steps": [shape("C"), bad] })).is_err());
    assert_eq!(p.revision(), before, "nothing was recorded");
    assert!(p.entity_ids().iter().all(|id| p.get::<de_shape::ShapeProgram>(*id).map_or(true, |s| s.name != "C")), "C was rolled back");
}

#[test]
fn a_thin_plate_bends_as_plate_theory_says_and_solves_quickly() {
    // Steel plate 200 × 100 × 4 mm held along x = 0, 50 N down along the free edge.
    // Two cubes through the thickness: the case plain Jacobi-CG crawls through.
    let e = runtime::evaluate("return box(200, 100, 4);", &[], 0.01).unwrap();
    let face = |x: f64, n: f64| de_shape::fea::Face {
        point: [x, 50.0, 2.0],
        normal: [n, 0.0, 0.0],
        triangles: vec![x, 0.0, 0.0, x, 100.0, 0.0, x, 100.0, 4.0, x, 0.0, 0.0, x, 100.0, 4.0, x, 0.0, 4.0],
    };
    let study = de_shape::fea::Study { supports: vec![face(0.0, -1.0)], loads: vec![de_shape::fea::Load { face: face(200.0, 1.0), force: [0.0, 0.0, -50.0] }], bore_supports: vec![], bore_loads: vec![], resolution: 160 };
    let steel = de_shape::fea::elastic("Steel").unwrap();
    let t0 = std::time::Instant::now();
    let r = de_shape::fea::solve(&e.positions, &e.indices, &steel, &study).unwrap();
    let took = t0.elapsed().as_secs_f64();
    // Between a free beam (PL³/3EI) and a plate held flat across its width (× (1 − ν²)).
    let beam = 50.0 * 200f64.powi(3) / (3.0 * 200_000.0 * (100.0 * 4f64.powi(3) / 12.0));
    let plate = beam * (1.0 - 0.29f64.powi(2));
    let tip = -de_shape::fea::displacement_at(&r, [200.0, 50.0, 2.0]).unwrap()[2];
    eprintln!("thin plate: tip {tip:.4} mm (plate {plate:.4} – beam {beam:.4}), {} cubes, {} iterations, {took:.2} s", r.elements, r.iterations);
    assert!(tip > plate * 0.95 && tip < beam * 1.05, "tip {tip} mm, expected {plate:.4}–{beam:.4}");
    assert!(r.notes.iter().all(|n| !n.contains("did not fully settle")), "{:?}", r.notes);
    // Plain Jacobi-CG took over 3000 steps here; multigrid takes a few dozen.
    assert!(r.iterations < 150, "{} iterations", r.iterations);
}

#[test]
fn trim_cuts_back_to_the_nearest_crossings_and_extend_reaches_the_next_curve() {
    use de_shape::sketch::{Constraint, Curve, Sketch};
    let solved_of = |p: &Project, s: StableId| {
        let sk = p.get::<Sketch>(s).unwrap().clone();
        let solved = de_shape::sketch::solve(&sk.geometry, &Default::default()).unwrap();
        (sk, solved)
    };
    let mut p = project();
    // A plus sign (two lines crossing at the origin) and a circle round it, radius 30.
    let g = json!({
        "points": [{ "id": 1, "x": -50, "y": 0 }, { "id": 2, "x": 50, "y": 0 }, { "id": 3, "x": 0, "y": -50 }, { "id": 4, "x": 0, "y": 50 }, { "id": 5, "x": 0, "y": 0 }],
        "curves": [{ "kind": "line", "id": 10, "a": 1, "b": 2 }, { "kind": "line", "id": 11, "a": 3, "b": 4 }, { "kind": "circle", "id": 12, "center": 5, "radius": 30 }],
        "constraints": [{ "kind": "horizontal", "line": 10 }]
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "T", "plane": { "name": "XY" }, "geometry": g })).unwrap());
    // Trim the horizontal line between the crossings at x = 0 and x = 30: two pieces stay.
    run(&mut p, "sketch.trim", json!({ "target": s, "curve": 10, "at": [15, 0] })).unwrap();
    let (sk, sol) = solved_of(&p, s);
    let lines: Vec<(f64, f64)> = sk.geometry.curves.iter().filter_map(|c| match c {
        Curve::Line { a, b, .. } if sol.points[a][1].abs() < 1e-6 && sol.points[b][1].abs() < 1e-6 => Some((sol.points[a][0].min(sol.points[b][0]), sol.points[a][0].max(sol.points[b][0]))),
        _ => None,
    }).collect();
    let mut lines = lines;
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!((lines[0].0 + 50.0).abs() < 1e-6 && lines[0].1.abs() < 1e-6, "left piece ends at the crossing: {lines:?}");
    assert!((lines[1].0 - 30.0).abs() < 1e-6 && (lines[1].1 - 50.0).abs() < 1e-6, "right piece starts at the circle: {lines:?}");
    assert_eq!(sk.geometry.constraints.iter().filter(|c| matches!(c, Constraint::Horizontal { .. })).count(), 2, "both pieces stay level");
    assert!(sk.geometry.constraints.iter().any(|c| matches!(c, Constraint::OnCurve { curve: 12, .. })), "the cut end is held on the circle");

    // Trim the top of the circle (between the vertical line and the right piece): an arc stays.
    run(&mut p, "sketch.trim", json!({ "target": s, "curve": 12, "at": [21.2, 21.2] })).unwrap();
    let (sk, sol) = solved_of(&p, s);
    let arc = sk.geometry.curves.iter().find(|c| c.id() == 12).unwrap();
    let Curve::Arc { start, end, .. } = arc else { panic!("the circle became {arc:?}") };
    let (a, b) = (sol.points[start], sol.points[end]);
    assert!((a[0] - 0.0).abs() < 1e-6 && (a[1] - 30.0).abs() < 1e-6, "arc starts at the top crossing: {a:?}");
    assert!((b[0] - 30.0).abs() < 1e-6 && b[1].abs() < 1e-6, "and runs round to the right crossing: {b:?}");

    // A lone line with no crossings goes entirely.
    let lone = json!({
        "points": [{ "id": 1, "x": 100, "y": 100 }, { "id": 2, "x": 120, "y": 100 }],
        "curves": [{ "kind": "line", "id": 3, "a": 1, "b": 2 }],
        "constraints": [{ "kind": "length", "line": 3, "key": "d1" }]
    });
    let s2 = id(run(&mut p, "sketch.create", json!({ "name": "L", "plane": { "name": "XY" }, "geometry": lone, "params": [{ "key": "d1", "label": "Length", "value": 20 }] })).unwrap());
    run(&mut p, "sketch.trim", json!({ "target": s2, "curve": 3, "at": [110, 100] })).unwrap();
    let sk2 = p.get::<Sketch>(s2).unwrap();
    assert!(sk2.geometry.curves.is_empty() && sk2.geometry.constraints.is_empty() && sk2.params.is_empty(), "line, its length and the dimension all go");

    // Extend: a line stopping short of an upright reaches it; another reaches the upright's end and joins it.
    let e = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 20, "y": 0 }, { "id": 3, "x": 50, "y": -10 }, { "id": 4, "x": 50, "y": 40 }, { "id": 5, "x": 0, "y": 40 }, { "id": 6, "x": 10, "y": 40 }],
        "curves": [{ "kind": "line", "id": 10, "a": 1, "b": 2 }, { "kind": "line", "id": 11, "a": 3, "b": 4 }, { "kind": "line", "id": 12, "a": 5, "b": 6 }],
        "constraints": [{ "kind": "length", "line": 10, "key": "d1" }]
    });
    let s3 = id(run(&mut p, "sketch.create", json!({ "name": "E", "plane": { "name": "XY" }, "geometry": e, "params": [{ "key": "d1", "label": "Length", "value": 20 }] })).unwrap());
    run(&mut p, "sketch.extend", json!({ "target": s3, "curve": 10, "at": [19, 0] })).unwrap();
    run(&mut p, "sketch.extend", json!({ "target": s3, "curve": 12, "at": [9, 40] })).unwrap();
    let (sk3, sol3) = solved_of(&p, s3);
    assert!((sol3.points[&2][0] - 50.0).abs() < 1e-6, "reached the upright: {:?}", sol3.points[&2]);
    assert!(sk3.geometry.constraints.iter().any(|c| matches!(c, Constraint::OnCurve { point: 2, curve: 11 })), "and is held on it");
    assert!(!sk3.geometry.constraints.iter().any(|c| matches!(c, Constraint::Length { .. })), "its old length no longer holds it back");
    let Some(Curve::Line { b, .. }) = sk3.geometry.curves.iter().find(|c| c.id() == 12) else { panic!() };
    assert_eq!(*b, 4, "the second line now shares the upright's top end");
    assert!(run(&mut p, "sketch.extend", json!({ "target": s3, "curve": 11, "at": [50, 39] })).is_err(), "a joined end does not extend");

    // A Bézier crossed by a line: trimming keeps the part outside, still a Bézier ending on the line.
    let bz = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 60, "y": 0 }, { "id": 3, "x": 15, "y": 30 }, { "id": 4, "x": 45, "y": 30 }, { "id": 5, "x": 30, "y": -10 }, { "id": 6, "x": 30, "y": 40 }],
        "curves": [{ "kind": "bezier", "id": 10, "a": 1, "b": 2, "c1": 3, "c2": 4 }, { "kind": "line", "id": 11, "a": 5, "b": 6 }],
        "constraints": []
    });
    let s4 = id(run(&mut p, "sketch.create", json!({ "name": "B", "plane": { "name": "XY" }, "geometry": bz })).unwrap());
    run(&mut p, "sketch.trim", json!({ "target": s4, "curve": 10, "at": [50, 15] })).unwrap();
    let (sk4, sol4) = solved_of(&p, s4);
    let Some(Curve::Bezier { a, b, c1, c2, .. }) = sk4.geometry.curves.iter().find(|c| c.id() == 10) else { panic!() };
    assert_eq!(*a, 1, "keeps its start");
    let end = sol4.points[b];
    assert!((end[0] - 30.0).abs() < 0.05 && (end[1] - 22.5).abs() < 0.05, "ends on the line at the crossing: {end:?}");
    // The kept half follows the original curve exactly (de Casteljau split at t = ½).
    let mid = de_shape::sketch::bezier_at(sol4.points[a], sol4.points[c1], sol4.points[c2], end, 0.5);
    let orig = de_shape::sketch::bezier_at([0.0, 0.0], [15.0, 30.0], [45.0, 30.0], [60.0, 0.0], 0.25);
    assert!((mid[0] - orig[0]).abs() < 0.05 && (mid[1] - orig[1]).abs() < 0.05, "{mid:?} vs {orig:?}");
}
