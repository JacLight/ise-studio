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
        DimSpec { view: ViewName::Top, a: [50.0, 30.0], b: [100.0, 30.0], kind: DimKind::Horizontal, offset: 12.0, tolerance: None, diameter: false },
        DimSpec { view: ViewName::Front, a: [0.0, 0.0], b: [100.0, 20.0], kind: DimKind::Aligned, offset: 8.0, tolerance: None, diameter: false },
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
fn iso_286_fits_match_the_tables() {
    use de_shape::drawing::fits::deviations;
    // (class, nominal, upper µm, lower µm) from ISO 286-2.
    let known: [(&str, f64, f64, f64); 24] = [
        ("H7", 25.0, 21.0, 0.0),
        ("g6", 25.0, -7.0, -20.0),
        ("H6", 10.0, 9.0, 0.0),
        ("H8", 50.0, 39.0, 0.0),
        ("H9", 100.0, 87.0, 0.0),
        ("H11", 6.0, 75.0, 0.0),
        ("H7", 3.0, 10.0, 0.0),
        ("H7", 500.0, 63.0, 0.0),
        ("f7", 10.0, -13.0, -28.0),
        ("f7", 40.0, -25.0, -50.0),
        ("h6", 100.0, 0.0, -22.0),
        ("h11", 200.0, 0.0, -290.0),
        ("k6", 25.0, 15.0, 2.0),
        ("k6", 40.0, 18.0, 2.0),
        ("n6", 6.0, 16.0, 8.0),
        ("n6", 25.0, 28.0, 15.0),
        ("p6", 25.0, 35.0, 22.0),
        ("p6", 200.0, 79.0, 50.0),
        ("js6", 25.0, 6.5, -6.5),
        ("js7", 25.0, 10.0, -10.0),
        ("G7", 25.0, 28.0, 7.0),
        ("K7", 25.0, 6.0, -15.0),
        ("N7", 25.0, -7.0, -28.0),
        ("P7", 25.0, -14.0, -35.0),
    ];
    for (class, n, up, lo) in known {
        let (u, l) = deviations(class, n).unwrap();
        assert!((u * 1000.0 - up).abs() < 1e-9 && (l * 1000.0 - lo).abs() < 1e-9, "⌀{n} {class}: {:.1}/{:.1} µm, want {up}/{lo}", u * 1000.0, l * 1000.0);
    }
    assert!(deviations("H7", 600.0).is_err(), "beyond the tables");
    assert!(deviations("X7", 25.0).is_err(), "no such class");
    assert!(deviations("H", 25.0).is_err(), "no grade");
}

#[test]
fn a_drawing_has_details_tolerances_and_gdt() {
    use de_shape::drawing::{DatumSpec, DetailSpec, DimKind, DimSpec, FrameSpec, GdtSymbol, HoleFit, MarkKind, Modifier, Options, Pen, Sheet, Title, Tolerance, ViewName};
    // A plate with a ⌀16 hole and a slot across its top.
    let program = de_shape::ShapeProgram {
        name: "Plate".into(),
        description: String::new(),
        source: "return box(100, 60, 20).subtract(cylinder(30, 8).translate([50, 30, -5])).subtract(box(10, 70, 4).translate([10, -5, 16]));".into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let title = Title { title: "Plate".into(), number: "P-2".into(), material: "Steel".into(), mass: "0.9 kg".into(), drawn_by: "F".into(), date: "2026-10-11".into(), parts: vec![] };
    let dim = |view, a, b, kind, offset, tolerance, diameter| DimSpec { view, a, b, kind, offset, tolerance, diameter };
    let options = Options {
        sheet: Sheet::A3,
        section: Some(50.0),
        dimensions: vec![
            dim(ViewName::Top, [42.0, 30.0], [58.0, 30.0], DimKind::Horizontal, 22.0, Some(Tolerance::Fit { fit: "H7".into() }), true),
            dim(ViewName::Front, [0.0, 0.0], [0.0, 16.0], DimKind::Vertical, 14.0, Some(Tolerance::Limits { upper: 0.1, lower: -0.05 }), false),
            dim(ViewName::Top, [10.0, 0.0], [20.0, 0.0], DimKind::Horizontal, -8.0, Some(Tolerance::Symmetric { value: 0.2 }), false),
        ],
        details: vec![
            // The slot's corner in the section, and the hole seen from above.
            DetailSpec { view: ViewName::Front, center: [15.0, 18.0], radius: 6.0, scale: None },
            DetailSpec { view: ViewName::Top, center: [50.0, 30.0], radius: 10.0, scale: Some(2.0) },
        ],
        frames: vec![
            FrameSpec { view: ViewName::Front, at: [70.0, 20.0], symbol: GdtSymbol::Flatness, tolerance: 0.05, diameter_zone: false, datums: vec![], modifier: None },
            FrameSpec { view: ViewName::Top, at: [58.0, 30.0], symbol: GdtSymbol::Position, tolerance: 0.1, diameter_zone: true, datums: vec!["A".into(), "B".into(), "C".into()], modifier: Some(Modifier::Mmc) },
            FrameSpec { view: ViewName::Right, at: [60.0, 10.0], symbol: GdtSymbol::Perpendicularity, tolerance: 0.02, diameter_zone: false, datums: vec!["A".into()], modifier: None },
        ],
        datums: vec![DatumSpec { view: ViewName::Front, at: [85.0, 0.0], letter: "A".into() }, DatumSpec { view: ViewName::Top, at: [0.0, 45.0], letter: "B".into() }, DatumSpec { view: ViewName::Top, at: [80.0, 0.0], letter: "C".into() }],
        hole_fits: vec![HoleFit { diameter: 16.0, fit: "H7".into() }],
        ..Default::default()
    };
    de_shape::drawing::validate(&options.dimensions, &options.details, &options.frames, &options.datums, &options.hole_fits).unwrap();
    let d = de_shape::drawing::drawing(&program, &options, &title).unwrap();
    if let Ok(dir) = std::env::var("DRAWING_OUT") {
        std::fs::write(format!("{dir}/plate-gdt.pdf"), de_shape::drawing::pdf(&d)).unwrap();
        std::fs::write(format!("{dir}/plate-gdt.dxf"), de_shape::drawing::dxf(&d)).unwrap();
    }
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    let has = |s: &str| d.texts.iter().any(|t| t.text == s);
    // Tolerances, ISO 129: ⌀16 H7 (+0.018 / 0); 16 +0.10/−0.05; 10 ±0.2.
    for s in ["⌀16 H7", "+0.018", "0", "+0.10", "−0.05", "10 ±0.2"] {
        assert!(has(s), "shows {s}: {:?}", d.texts.iter().map(|t| &t.text).collect::<Vec<_>>());
    }
    assert!(d.annotations.iter().any(|a| a.kind == MarkKind::Hole && a.diameter == Some(16.0)), "the hole callout is marked");
    // Datums and frames.
    assert_eq!(d.annotations.iter().filter(|a| a.kind == MarkKind::Datum).count(), 3);
    assert_eq!(d.annotations.iter().filter(|a| a.kind == MarkKind::Frame).count(), 3);
    assert!(has("⌀0.1") && has("0.05") && has("0.02") && has("M"), "frame values");
    for l in ["A", "B", "C"] {
        assert!(d.texts.iter().filter(|t| t.text == l && !t.bold).count() >= 2, "datum {l} in a symbol and a frame");
    }
    // Details: lettered after the section (B, C), each one's lines inside its circle.
    let details: Vec<_> = d.annotations.iter().filter(|a| a.kind == MarkKind::Detail).collect();
    assert_eq!(details.len(), 2);
    assert!(has("DETAIL C (2:1)"), "the scale asked for");
    let b_label = d.texts.iter().find(|t| t.text.starts_with("DETAIL B (")).expect("detail B");
    let front = d.views.iter().find(|v| v.view == ViewName::Front).unwrap();
    let k_b: f64 = {
        let r = b_label.text.trim_start_matches("DETAIL B (").trim_end_matches(')');
        let (a, b) = r.split_once(':').unwrap();
        a.parse::<f64>().unwrap() / b.parse::<f64>().unwrap()
    };
    assert_eq!(details[0].index, 0);
    assert!(k_b >= 2.0 * front.scale, "enlarged at least twice: {}", b_label.text);
    // Each detail: its lines inside its circle, over nothing else on the sheet.
    let check = |d: &de_shape::drawing::Drawing, options: &Options| {
        for a in d.annotations.iter().filter(|a| a.kind == MarkKind::Detail) {
            let b = a.bounds;
            let label = d.texts.iter().find(|t| t.text.starts_with("DETAIL") && t.at[0] > b[0] && t.at[0] < b[2] && t.at[1] > b[1] && t.at[1] < b[3]).expect("labelled");
            let ratio = label.text.rsplit('(').next().unwrap().trim_end_matches(')');
            let (x, y) = ratio.split_once(':').unwrap();
            let big = options.details[a.index].radius * x.parse::<f64>().unwrap() / y.parse::<f64>().unwrap();
            let c = [(b[0] + b[2]) / 2.0, b[3] - big];
            let inside: Vec<_> = d.strokes.iter().filter(|s| s.points.iter().all(|p| p[0] >= b[0] - 1e-6 && p[0] <= b[2] + 1e-6 && p[1] >= b[1] - 1e-6 && p[1] <= b[3] + 1e-6)).collect();
            assert!(inside.len() >= 4, "detail has lines: {}", inside.len());
            for s in &inside {
                for p in &s.points {
                    assert!((p[0] - c[0]).hypot(p[1] - c[1]) <= big + 1e-6, "a detail line leaves its circle: {p:?} vs {c:?} r {big}");
                }
            }
            assert!(inside.iter().any(|s| s.pen == Pen::Visible), "visible edges in the detail");
            for v in &d.views {
                let vb = v.bounds;
                assert!(!(b[0] < vb[2] && vb[0] < b[2] && b[1] < vb[3] && vb[1] < b[3]), "detail over the {:?} view", v.view);
            }
            for o in d.annotations.iter().filter(|o| o.kind != MarkKind::Frame && !std::ptr::eq(*o, a)) {
                let ob = o.bounds;
                assert!(!(b[0] < ob[2] && ob[0] < b[2] && b[1] < ob[3] && ob[1] < b[3]), "detail over {:?}", o.kind);
            }
            // Nothing else reaches into it: no line ends there, no lettering but its own label.
            assert!(d.strokes.iter().all(|s| s.points.iter().all(|p| !(p[0] > b[0] && p[0] < b[2] && p[1] > b[1] && p[1] < b[3])) || s.points.iter().all(|p| p[0] >= b[0] - 1e-6 && p[0] <= b[2] + 1e-6 && p[1] >= b[1] - 1e-6 && p[1] <= b[3] + 1e-6)), "a line runs into a detail");
            assert_eq!(d.texts.iter().filter(|t| t.at[0] > b[0] && t.at[0] < b[2] && t.at[1] > b[1] && t.at[1] < b[3]).count(), 1, "only the detail's label in its box");
        }
        // Nothing off the sheet.
        assert!(d.strokes.iter().flat_map(|s| &s.points).chain(d.fills.iter().flatten()).all(|p| p[0] >= 0.0 && p[0] <= d.width && p[1] >= 0.0 && p[1] <= d.height));
        assert!(d.texts.iter().all(|t| t.at[0] >= 0.0 && t.at[0] <= d.width && t.at[1] >= 0.0 && t.at[1] <= d.height));
    };
    check(&d, &options);
    // The section's detail is hatched.
    let b0 = details[0].bounds;
    assert!(d.strokes.iter().filter(|s| s.pen == Pen::Thin && s.points.len() == 2 && s.points.iter().all(|p| p[0] >= b0[0] && p[0] <= b0[2] && p[1] >= b0[1] && p[1] <= b0[3])).count() > 3, "hatching in detail B");
    // A crowded A4 in third angle: whatever finds room is placed, the rest is said.
    let small = Options { sheet: Sheet::A4, projection: de_shape::drawing::Projection::Third, ..options.clone() };
    let d4 = de_shape::drawing::drawing(&program, &small, &title).unwrap();
    check(&d4, &small);
    let placed = d4.annotations.iter().filter(|a| a.kind == MarkKind::Detail).count();
    assert_eq!(placed + d4.warnings.iter().filter(|w| w.contains("detail")).count(), 2, "{:?}", d4.warnings);
    if let Ok(dir) = std::env::var("DRAWING_OUT") {
        std::fs::write(format!("{dir}/plate-gdt-a4.pdf"), de_shape::drawing::pdf(&d4)).unwrap();
    }
    // A fit that is not in the tables is refused when kept.
    let bad = vec![dim(ViewName::Top, [42.0, 30.0], [58.0, 30.0], DimKind::Horizontal, 22.0, Some(Tolerance::Fit { fit: "Z9".into() }), true)];
    assert!(de_shape::drawing::validate(&bad, &[], &[], &[], &[]).is_err());
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
        part: None,
    };
    let study = de_shape::fea::Study { supports: vec![face(0.0, -1.0)], loads: vec![de_shape::fea::Load { face: face(200.0, 1.0), force: [0.0, 0.0, -1000.0] }], bore_supports: vec![], bore_loads: vec![], resolution: 80, contacts: vec![] };
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
    let held = de_shape::fea::Face { point: [0.0, 10.0, 10.0], normal: [-1.0, 0.0, 0.0], triangles: vec![0.0, 0.0, 0.0, 0.0, 20.0, 0.0, 0.0, 20.0, 20.0, 0.0, 0.0, 0.0, 0.0, 20.0, 20.0, 0.0, 0.0, 20.0], part: None };
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
    de_shape::fea::Face { point: [x, s / 2.0, s / 2.0], normal: [n, 0.0, 0.0], triangles: vec![x, 0.0, 0.0, x, s, 0.0, x, s, s, x, 0.0, 0.0, x, s, s, x, 0.0, s], part: None }
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
        part: None,
    };
    let study = de_shape::fea::Study { supports: vec![face(0.0, -1.0)], loads: vec![de_shape::fea::Load { face: face(200.0, 1.0), force: [0.0, 0.0, -50.0] }], bore_supports: vec![], bore_loads: vec![], resolution: 160, contacts: vec![] };
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

#[test]
fn a_curved_face_pushes_and_pulls_exactly() {
    // A 40 × 40 × 10 plate with a ⌀10 hole; the hole's wall pushed 1 mm (into the hole: ⌀8),
    // then a cylinder's side pulled 2 mm (r 10 → 8). Picked by a point on the face, as the view does.
    let program = |source: &str| de_shape::ShapeProgram {
        name: "P".into(),
        description: String::new(),
        source: source.into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let volume_with = |p: &de_shape::ShapeProgram, values: &[(String, f64)]| {
        let (arena, exact) = de_shape::exact::evaluate_exact_in(de_occt::Arena::new(), &p.name, &p.source, values, 0.01, &p.inputs).map_err(|(_, e)| e).unwrap();
        exact.parts.iter().map(|q| arena.volume(q.shape)).sum::<f64>()
    };
    let volume = |p: &de_shape::ShapeProgram| volume_with(p, &[]);
    let pi = std::f64::consts::PI;
    let plate = program("return box(40, 40, 10).subtract(cylinder(20, 5).translate([20, 20, -5])).push(1, { curved: true, near: [25, 20, 5] });");
    let want = 16000.0 - pi * 16.0 * 10.0;
    assert!((volume(&plate) - want).abs() < want * 1e-5, "plate {} vs {want}", volume(&plate));
    let rod = program("return cylinder(20, 10).push(-2, { curved: true, near: [10, 0, 10] });");
    let want = pi * 64.0 * 20.0;
    assert!((volume(&rod) - want).abs() < want * 1e-5, "rod {} vs {want}", volume(&rod));
    // As a finish the view adds: where on the bounds, and kind "curved_face".
    let mut finished = program("return cylinder(20, 10);");
    finished.finishes = vec![serde_json::from_value(json!({ "kind": "push", "key": "push1", "edges": [{ "at": [1.0, 0.5, 0.5], "kind": "curved_face", "dir": [1, 0, 0] }] })).unwrap()];
    finished.inputs = format!(r#"{{"sketches":{{}},"finishes":{}}}"#, serde_json::to_string(&finished.finishes).unwrap());
    let want = pi * 169.0 * 20.0;
    let got = volume_with(&finished, &[("push1".into(), 3.0)]);
    assert!((got - want).abs() < want * 1e-5, "finished {got} vs {want}");
}

#[test]
fn an_assembly_drawing_balloons_every_listed_part() {
    use de_shape::drawing::{iso_point, segments_cross, Options, PartRow, Pen, Sheet, Title};
    // A base, a housing on it, two bolts at opposite corners and a pin on top.
    let source = r#"return [
        part("Base", box(120, 80, 10)),
        part("Housing", box(40, 40, 30).translate([40, 20, 10])),
        part("Bolt 1", cylinder(20, 4).translate([12, 12, 10])),
        part("Bolt 2", cylinder(20, 4).translate([108, 68, 10])),
        part("Pin", cylinder(40, 3).translate([60, 40, 40])),
    ];"#;
    let program = de_shape::ShapeProgram {
        name: "Bracket".into(),
        description: String::new(),
        source: source.into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    // The isometric projection the balloons use is the one the view is drawn
    // in: the base's eight corners land exactly on the view's outline.
    let (mut arena, exact) = de_shape::exact::evaluate_exact_in(de_occt::Arena::new(), "Bracket", source, &[], 0.01, "").map_err(|(_, e)| e).unwrap();
    let k = 1.0 / 3f64.sqrt();
    let lines = arena.hlr_view(exact.parts[0].shape, [-k, k, -k], [0.0, 0.0, 1.0], 0.01).unwrap();
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in lines.iter().flat_map(|l| &l.points) {
        (lo, hi) = ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]);
    }
    let (mut plo, mut phi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for c in 0..8 {
        let q = iso_point([if c & 1 > 0 { 120.0 } else { 0.0 }, if c & 2 > 0 { 80.0 } else { 0.0 }, if c & 4 > 0 { 10.0 } else { 0.0 }]);
        (plo, phi) = ([plo[0].min(q[0]), plo[1].min(q[1])], [phi[0].max(q[0]), phi[1].max(q[1])]);
    }
    for i in 0..2 {
        assert!((lo[i] - plo[i]).abs() < 1e-6 && (hi[i] - phi[i]).abs() < 1e-6, "iso outline {lo:?}–{hi:?} vs projected {plo:?}–{phi:?}");
    }
    // Isometric foreshortening: each axis shows at √(2/3) of its length, 30°
    // off the horizontal, so the 120 × 80 base spans (120 + 80)·√(2/3)·cos 30° across.
    assert!(((hi[0] - lo[0]) - 200.0 * (2.0f64 / 3.0).sqrt() * 30f64.to_radians().cos()).abs() < 1e-6, "{} across", hi[0] - lo[0]);

    let row = |name: &str, quantity| PartRow { name: name.into(), quantity, material: "Steel".into(), mass: "—".into() };
    let rows = vec![row("Base", 1), row("Housing", 1), row("Bolt 1", 1), row("Bolt 2", 1), row("Pin", 1)];
    let title = Title { title: "Bracket".into(), number: "B-1".into(), material: "Steel".into(), mass: "1 kg".into(), drawn_by: "F".into(), date: "2026-10-11".into(), parts: rows.clone() };
    for sheet in [Sheet::A3, Sheet::A4] {
        let d = de_shape::drawing::drawing(&program, &Options { sheet, ..Default::default() }, &title).unwrap();
        let iso = d.iso.clone().expect("an isometric view");
        // One balloon per row of the parts list, numbered as it is.
        let mut items: Vec<usize> = d.balloons.iter().map(|b| b.item).collect();
        items.sort();
        assert_eq!(items, vec![1, 2, 3, 4, 5], "{sheet:?}: balloons {items:?}");
        for b in &d.balloons {
            assert!(d.texts.iter().any(|t| t.text == b.item.to_string() && (t.at[0] - b.centre[0]).abs() < 1e-9), "balloon {} is numbered", b.item);
            // Inside the frame (20 mm binding margin, 10 elsewhere).
            assert!(b.centre[0] - b.radius >= 20.0 && b.centre[0] + b.radius <= d.width - 10.0 && b.centre[1] - b.radius >= 10.0 && b.centre[1] + b.radius <= d.height - 10.0, "{sheet:?}: balloon {} off the sheet at {:?}", b.item, b.centre);
            // Clear of every view: no visible edge comes within its circle.
            for s in d.strokes.iter().filter(|s| s.pen == Pen::Visible) {
                for w in s.points.windows(2) {
                    let (a, q, c) = (w[0], w[1], b.centre);
                    let (dx, dy) = (q[0] - a[0], q[1] - a[1]);
                    let t = (((c[0] - a[0]) * dx + (c[1] - a[1]) * dy) / (dx * dx + dy * dy).max(1e-12)).clamp(0.0, 1.0);
                    let gap = (a[0] + t * dx - c[0]).hypot(a[1] + t * dy - c[1]);
                    assert!(gap > b.radius, "{sheet:?}: balloon {} overlaps a view edge ({gap:.2} mm from its centre)", b.item);
                }
            }
            // Its leader ends on its own part: inside that part's isometric outline.
            let name = &rows[b.item - 1].name;
            let p = [(b.anchor[0] - iso.at[0]) / iso.scale, (b.anchor[1] - iso.at[1]) / iso.scale];
            let on_part = exact.parts.iter().filter(|q| &q.name == name).any(|q| {
                let bb = arena.bounds(q.shape).unwrap();
                let corners: Vec<[f64; 2]> = (0..8).map(|c| iso_point([bb[if c & 1 > 0 { 3 } else { 0 }], bb[if c & 2 > 0 { 4 } else { 1 }], bb[if c & 4 > 0 { 5 } else { 2 }]])).collect();
                let (x0, x1) = (corners.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min), corners.iter().map(|c| c[0]).fold(f64::NEG_INFINITY, f64::max));
                let (y0, y1) = (corners.iter().map(|c| c[1]).fold(f64::INFINITY, f64::min), corners.iter().map(|c| c[1]).fold(f64::NEG_INFINITY, f64::max));
                p[0] > x0 && p[0] < x1 && p[1] > y0 && p[1] < y1
            });
            assert!(on_part, "{sheet:?}: balloon {}'s leader ends on the {name}", b.item);
            // The anchor is on the view; the balloon outside it.
            let o = iso.bounds;
            assert!(b.anchor[0] > o[0] && b.anchor[0] < o[2] && b.anchor[1] > o[1] && b.anchor[1] < o[3]);
            assert!(b.centre[0] + b.radius < o[0] || b.centre[0] - b.radius > o[2] || b.centre[1] + b.radius < o[1] || b.centre[1] - b.radius > o[3], "{sheet:?}: balloon {} sits on the view", b.item);
        }
        for (i, a) in d.balloons.iter().enumerate() {
            for b in &d.balloons[i + 1..] {
                let apart = (a.centre[0] - b.centre[0]).hypot(a.centre[1] - b.centre[1]);
                assert!(apart >= a.radius + b.radius, "{sheet:?}: balloons {} and {} overlap ({apart:.1} mm apart)", a.item, b.item);
                assert!(!segments_cross(a.anchor, a.centre, b.anchor, b.centre), "{sheet:?}: the leaders of {} and {} cross", a.item, b.item);
            }
        }
        // The isometric's caption stays clear of the balloons.
        let caption = d.texts.iter().find(|t| t.text.starts_with("ISOMETRIC")).unwrap();
        assert!(d.balloons.iter().all(|b| caption.at[1] + caption.size * 1.2 < b.centre[1] - b.radius), "{sheet:?}: caption under the balloons");
        if let Ok(dir) = std::env::var("DRAWING_OUT") {
            std::fs::write(format!("{dir}/bracket-{sheet:?}.pdf"), de_shape::drawing::pdf(&d)).unwrap();
            std::fs::write(format!("{dir}/bracket-{sheet:?}.dxf"), de_shape::drawing::dxf(&d)).unwrap();
        }
    }

    // The ring on its own: a dozen anchors bunched in one corner still get
    // balloons that neither overlap nor cross.
    let anchors: Vec<[f64; 2]> = (0..12).map(|i| [10.0 + (i % 4) as f64 * 3.0, 10.0 + (i / 4) as f64 * 3.0]).collect();
    let ring = de_shape::drawing::balloon_ring(&anchors, [0.0, 0.0, 100.0, 60.0], 4.0, 10.0);
    for i in 0..12 {
        for j in i + 1..12 {
            assert!((ring[i][0] - ring[j][0]).hypot(ring[i][1] - ring[j][1]) >= 8.0, "ring balloons {i} and {j} overlap");
            assert!(!segments_cross(anchors[i], ring[i], anchors[j], ring[j]), "ring leaders {i} and {j} cross");
        }
    }
}

#[test]
fn shells_and_drafts_are_exact_in_programs_and_as_finishes() {
    let exact_volume = |source: &str, values: &[(String, f64)], inputs: &str| {
        let (arena, exact) = de_shape::exact::evaluate_exact_in(de_occt::Arena::new(), "P", source, values, 0.01, inputs).map_err(|(_, e)| e).unwrap();
        exact.parts.iter().map(|q| arena.volume(q.shape)).sum::<f64>()
    };
    // A 40 × 40 × 20 box, open at the top, 2 mm walls.
    let tray = 40.0 * 40.0 * 20.0 - 36.0 * 36.0 * 18.0;
    let v = exact_volume("return box(40, 40, 20).shell(2, { normal: [0, 0, 1] });", &[], "");
    assert!((v - tray).abs() < 1e-6, "tray {v} vs {tray}");
    // Closed: a sealed 36 × 36 × 16 void.
    let sealed = 40.0 * 40.0 * 20.0 - 36.0 * 36.0 * 16.0;
    let v = exact_volume("return box(40, 40, 20).shell(2);", &[], "");
    assert!((v - sealed).abs() < 1e-6, "sealed {v} vs {sealed}");
    // A list of open faces: top and front.
    let v = exact_volume("return box(40, 40, 20).shell(2, [{ normal: [0, 0, 1] }, { normal: [0, -1, 0] }]);", &[], "");
    let want = 40.0 * 40.0 * 20.0 - 36.0 * 38.0 * 18.0;
    assert!((v - want).abs() < 1e-6, "channel {v} vs {want}");
    // The mesh path hands shells to the exact kernel too.
    let e = runtime::evaluate("return box(40, 40, 20).shell(2, { normal: [0, 0, 1] });", &[], 0.01).unwrap();
    assert!((e.volume - tray).abs() < 1e-6, "meshed tray {}", e.volume);

    // Draft: the four sides of the box by 5°, pulled up from the bottom → a frustum.
    let t = 5f64.to_radians().tan();
    let top = 40.0 - 2.0 * 20.0 * t;
    let frustum = 20.0 / 3.0 * (1600.0 + top * top + 40.0 * top);
    let v = exact_volume("return box(40, 40, 20).draft(5);", &[], "");
    assert!((v - frustum).abs() < 1e-6, "draft {v} vs {frustum}");
    let v = exact_volume(
        "return box(40, 40, 20).draft(5, { pull: [0, 0, 1], neutral: 0, faces: [{ normal: [1, 0, 0] }, { normal: [-1, 0, 0] }, { normal: [0, 1, 0] }, { normal: [0, -1, 0] }] });",
        &[],
        "",
    );
    assert!((v - frustum).abs() < 1e-6, "named faces {v} vs {frustum}");
    // Pulled down (the mould opens below, neutral at the top): the bottom shrinks instead, the same volume.
    let v = exact_volume("return box(40, 40, 20).draft(5, { pull: [0, 0, -1] });", &[], "");
    assert!((v - frustum).abs() < 1e-6, "pulled down {v} vs {frustum}");

    // As finishes the view adds, each sized by a parameter: draft 5° (every wall), then shell 2 open at the top.
    let finishes = json!([
        { "kind": "draft", "key": "draft1", "edges": [], "pull": [0, 0, 1], "neutral": 0 },
        { "kind": "shell", "key": "shell1", "edges": [{ "at": [0.5, 0.5, 1.0], "kind": "face", "dir": [0, 0, 1] }] }
    ]);
    let inputs = format!(r#"{{"sketches":{{}},"finishes":{finishes}}}"#);
    let v = exact_volume("return box(40, 40, 20);", &[("shell1".into(), 2.0), ("draft1".into(), 5.0)], &inputs);
    // The cavity: walls 2 mm square to each drafted face sit 2/cos 5° in horizontally,
    // the floor 2 mm up; so a frustum 18 tall from (40 − 2·2·tan5 − 2·2/cos5) wide.
    let c = 5f64.to_radians().cos();
    let ib = 40.0 - 4.0 * t - 4.0 / c;
    let it = ib - 2.0 * 18.0 * t;
    let inner = 18.0 / 3.0 * (ib * ib + it * it + ib * it);
    let want = frustum - inner;
    assert!((v - want).abs() < 1e-6, "drafted tray {v} vs {want}");

    // Through the project: stored on the part, editable by its parameter.
    let mut p = project();
    let part = id(run(&mut p, "shape.create", json!({ "name": "Tray", "source": "return box(40, 40, 20);", "params": [] })).unwrap());
    run(&mut p, "shape.update", json!({
        "target": part,
        "params": [{ "key": "shell1", "label": "Shell 1 wall", "value": 2 }],
        "finishes": [{ "kind": "shell", "key": "shell1", "edges": [{ "at": [0.5, 0.5, 1.0], "kind": "face", "dir": [0, 0, 1] }] }]
    }))
    .unwrap();
    let volume = |p: &Project| p.get::<ShapeSolid>(part).unwrap().stats.volume_mm3;
    assert!((volume(&p) - tray).abs() < 1e-3, "{}", volume(&p));
    run(&mut p, "shape.set_params", json!({ "target": part, "values": { "shell1": 3 } })).unwrap();
    let want = 40.0 * 40.0 * 20.0 - 34.0 * 34.0 * 17.0;
    assert!((volume(&p) - want).abs() < 1e-3, "{}", volume(&p));
    // A zero wall is refused.
    assert!(run(&mut p, "shape.set_params", json!({ "target": part, "values": { "shell1": 0 } })).is_err());
}

// ---- Contact between parts -----------------------------------------------------

/// An assembly's parts as bodies for a several-part study, in the materials given.
fn bodies<'a>(e: &'a runtime::Evaluated, materials: &[&str]) -> Vec<de_shape::fea::Body<'a>> {
    e.parts
        .iter()
        .zip(materials)
        .map(|(p, m)| de_shape::fea::Body { name: p.name.clone(), positions: &e.positions, triangles: &e.indices[p.index_start..p.index_start + p.index_count], material: de_shape::fea::elastic(m).unwrap() })
        .collect()
}

/// An axis-aligned rectangular face: `axis` (0 x, 1 y, 2 z) at `at`, facing `sign`, spanning [a0, a1] × [b0, b1] on the next two axes (cyclic).
fn rect_face(axis: usize, at: f64, sign: f64, a: [f64; 2], b: [f64; 2], part: Option<&str>) -> de_shape::fea::Face {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let corner = |s: f64, t: f64| {
        let mut p = [0.0; 3];
        p[axis] = at;
        p[u] = s;
        p[v] = t;
        p
    };
    let q = [corner(a[0], b[0]), corner(a[1], b[0]), corner(a[1], b[1]), corner(a[0], b[1])];
    let mut normal = [0.0; 3];
    normal[axis] = sign;
    de_shape::fea::Face {
        point: corner((a[0] + a[1]) / 2.0, (b[0] + b[1]) / 2.0),
        normal,
        triangles: [q[0], q[1], q[2], q[0], q[2], q[3]].iter().flatten().copied().collect(),
        part: part.map(String::from),
    }
}

fn load(face: de_shape::fea::Face, force: [f64; 3]) -> de_shape::fea::Load {
    de_shape::fea::Load { face, force }
}

fn study(supports: Vec<de_shape::fea::Face>, loads: Vec<de_shape::fea::Load>, resolution: usize, contacts: Vec<de_shape::fea::ContactSpec>) -> de_shape::fea::Study {
    de_shape::fea::Study { supports, loads, bore_supports: vec![], bore_loads: vec![], resolution, contacts }
}

fn contact(a: &str, b: &str, kind: de_shape::fea::ContactKind, friction: f64) -> de_shape::fea::ContactSpec {
    de_shape::fea::ContactSpec { parts: [a.into(), b.into()], kind, friction }
}

#[test]
fn stacked_blocks_press_with_f_over_a_and_act_as_one_block() {
    use de_shape::fea::ContactKind::*;
    // A 10 × 10 column 40 tall in two blocks, held at the bottom, 1000 N down on top: σ = F/A = 10 MPa.
    let e = runtime::evaluate("return [part('Base', box(10, 10, 20)), part('Top', box(10, 10, 20).translate(0, 0, 20))];", &[], 0.01).unwrap();
    let held = rect_face(2, 0.0, -1.0, [0.0, 10.0], [0.0, 10.0], Some("Base"));
    let pushed = load(rect_face(2, 40.0, 1.0, [0.0, 10.0], [0.0, 10.0], Some("Top")), [0.0, 0.0, -1000.0]);
    let steel = bodies(&e, &["Steel", "Steel"]);
    let t0 = std::time::Instant::now();
    let r = de_shape::fea::solve_parts(&steel, &study(vec![held.clone()], vec![pushed.clone()], 40, vec![])).unwrap();
    let took = t0.elapsed().as_secs_f64();
    let c = r.contact.as_ref().expect("the blocks touch");
    let pair = &c.pairs[0];
    eprintln!("stack: push {:.2} N over {:.1} mm², pressure {:.3}–{:.3} MPa, {} rounds, {} iterations, {took:.2} s", pair.normal_force, pair.pressing, c.faces.iter().map(|f| f.pressure).fold(f32::INFINITY, f32::min), c.max_pressure, c.rounds, r.iterations);
    assert!(c.settled, "contact settles");
    assert!((pair.normal_force - 1000.0).abs() < 1000.0 * 2e-3, "the whole load passes: {} N", pair.normal_force);
    assert!((pair.pressing - 100.0).abs() < 1e-6, "the whole 10 × 10 face presses: {} mm²", pair.pressing);
    assert!(c.faces.len() == 100 && c.faces.iter().all(|f| (f.pressure as f64 - 10.0).abs() < 10.0 * 0.03), "every face at F/A = 10 MPa within 3 %");
    assert!(c.opened.is_empty(), "nothing opens under pure compression");
    // The contact springs ride down the multigrid: a part held only through its contact solves as fast as one block.
    assert!(r.iterations < 80, "{} iterations", r.iterations);
    // The same as one solid block 40 tall.
    let solid = runtime::evaluate("return box(10, 10, 40);", &[], 0.01).unwrap();
    let mut whole_study = study(vec![held.clone()], vec![pushed.clone()], 40, vec![]);
    whole_study.supports[0].part = None;
    whole_study.loads[0].face.part = None;
    let whole = de_shape::fea::solve(&solid.positions, &solid.indices, &de_shape::fea::elastic("Steel").unwrap(), &whole_study).unwrap();
    let top = |r: &de_shape::fea::Result_, part: usize| -de_shape::fea::displacement_of(r, part, [5.0, 5.0, 40.0]).unwrap()[2];
    let (two, one) = (top(&r, 1), top(&whole, 0));
    // Plain compression: FL/EA = 0.002 mm (the held foot, kept from spreading, stiffens it slightly).
    let fl_ea = 1000.0 * 40.0 / (200_000.0 * 100.0);
    eprintln!("stack: top moves {two:.6} mm, one block {one:.6} mm, FL/EA {fl_ea:.6} mm");
    assert!((two - one).abs() < one * 0.01, "two blocks {two} mm vs one {one} mm");
    assert!((one - fl_ea).abs() < fl_ea * 0.03, "one block {one} mm vs FL/EA {fl_ea} mm");
    assert!((r.max_stress - whole.max_stress).abs() < whole.max_stress * 0.02, "stress {} vs {} MPa", r.max_stress, whole.max_stress);

    // Aluminium on steel: two springs in series, FL/E₁A + FL/E₂A; bonded or not, under compression it is the same.
    let mixed = bodies(&e, &["Steel", "Aluminium"]);
    let pressed = de_shape::fea::solve_parts(&mixed, &study(vec![held.clone()], vec![pushed.clone()], 40, vec![])).unwrap();
    let glued = de_shape::fea::solve_parts(&mixed, &study(vec![held], vec![pushed], 40, vec![contact("Base", "Top", Bonded, 0.0)])).unwrap();
    let series = 1000.0 * 20.0 / 100.0 * (1.0 / 200_000.0 + 1.0 / 69_000.0);
    let (p, g) = (top(&pressed, 1), top(&glued, 1));
    eprintln!("aluminium on steel: pressed {p:.6} mm, bonded {g:.6} mm, series springs {series:.6} mm ({} and {} iterations)", pressed.iterations, glued.iterations);
    assert!(pressed.iterations < 120 && glued.iterations < 120, "{} / {} iterations", pressed.iterations, glued.iterations);
    assert!((p - series).abs() < series * 0.03, "pressed {p} mm vs {series} mm");
    assert!((p - g).abs() < g * 0.01, "pressed {p} mm vs bonded {g} mm");
}

#[test]
fn a_block_lifted_off_its_support_opens_the_contact_and_passes_no_force() {
    // Top block held at its own side face, pulled up 500 N: it lifts off the base, which feels nothing.
    let e = runtime::evaluate("return [part('Base', box(10, 10, 20)), part('Top', box(10, 10, 20).translate(0, 0, 20))];", &[], 0.01).unwrap();
    let base_held = rect_face(2, 0.0, -1.0, [0.0, 10.0], [0.0, 10.0], Some("Base"));
    let top_held = rect_face(0, 0.0, -1.0, [0.0, 10.0], [20.0, 40.0], Some("Top"));
    let pull = load(rect_face(2, 40.0, 1.0, [0.0, 10.0], [0.0, 10.0], Some("Top")), [0.0, 0.0, 500.0]);
    let r = de_shape::fea::solve_parts(&bodies(&e, &["Steel", "Steel"]), &study(vec![base_held, top_held.clone()], vec![pull.clone()], 40, vec![])).unwrap();
    let c = r.contact.as_ref().unwrap();
    eprintln!("lift: push {:.2e} N, pressing {:.1} mm², {} opened markers, base stress {:.2e} MPa, top {:.2} MPa", c.pairs[0].normal_force, c.pairs[0].pressing, c.opened.len(), r.parts[0].max_stress, r.parts[1].max_stress);
    assert!(c.pairs[0].normal_force.abs() < 1e-6 * 500.0, "no force passes: {} N", c.pairs[0].normal_force);
    assert!(c.opened.len() > 50 && c.opened.iter().all(|o| o[3] > 0.0), "the contact opens: {} markers", c.opened.len());
    assert!(r.parts[0].max_stress < 1e-6 * r.parts[1].max_stress, "the base is not stressed: {} MPa", r.parts[0].max_stress);
    // The top block behaves exactly as if studied alone (same cubes: 1 mm on both grids).
    let alone = runtime::evaluate("return box(10, 10, 20).translate(0, 0, 20);", &[], 0.01).unwrap();
    let (mut held, mut pulled) = (top_held, pull);
    held.part = None;
    pulled.face.part = None;
    let solo = de_shape::fea::solve(&alone.positions, &alone.indices, &de_shape::fea::elastic("Steel").unwrap(), &study(vec![held], vec![pulled], 20, vec![])).unwrap();
    let corner = [10.0, 5.0, 40.0];
    let (with, without) = (de_shape::fea::displacement_of(&r, 1, corner).unwrap()[2], de_shape::fea::displacement_at(&solo, corner).unwrap()[2]);
    eprintln!("lift: free corner rises {with:.6} mm (alone {without:.6} mm)");
    assert!(with > 0.0 && (with - without).abs() < without * 1e-3, "{with} mm vs {without} mm alone");
}

#[test]
fn a_bonded_lap_joint_carries_shear_and_a_frictionless_one_slides() {
    use de_shape::fea::ContactKind::*;
    // Two 40 × 10 × 2 strips overlapping 20 mm; A held at x = 0, B pulled 200 N along x at x = 60.
    let e = runtime::evaluate("return [part('A', box(40, 10, 2)), part('B', box(40, 10, 2).translate(20, 0, 2))];", &[], 0.01).unwrap();
    let held = rect_face(0, 0.0, -1.0, [0.0, 10.0], [0.0, 2.0], Some("A"));
    let pulled = load(rect_face(0, 60.0, 1.0, [0.0, 10.0], [2.0, 4.0], Some("B")), [200.0, 0.0, 0.0]);
    let steel = bodies(&e, &["Steel", "Steel"]);
    let glued = de_shape::fea::solve_parts(&steel, &study(vec![held.clone()], vec![pulled.clone()], 60, vec![contact("A", "B", Bonded, 0.0)])).unwrap();
    let loose = de_shape::fea::solve_parts(&steel, &study(vec![held], vec![pulled], 60, vec![contact("A", "B", Frictionless, 0.0)])).unwrap();
    let (gc, lc) = (glued.contact.as_ref().unwrap(), loose.contact.as_ref().unwrap());
    let tip = |r: &de_shape::fea::Result_| r.parts[1].max_displacement;
    eprintln!(
        "lap joint: bonded shear {:.2} N, A {:.2} MPa, B moves {:.5} mm; frictionless shear {:.2e} N, A {:.2e} MPa, B moves {:.3e} mm",
        gc.pairs[0].shear_force, glued.parts[0].max_stress, tip(&glued), lc.pairs[0].shear_force, loose.parts[0].max_stress, tip(&loose)
    );
    // Bonded: all 200 N passes through the overlap. A's shank is pulled (F/A = 10 MPa) and, the
    // pull acting 2 mm above its middle, bent by M = 400 N·mm: at a cube centre 0.5 mm above the
    // middle, σ = F/A + M·y/I = 10 + 400 · 0.5 / (10 · 2³/12) = 40 MPa.
    assert!((gc.pairs[0].shear_force - 200.0).abs() < 200.0 * 0.01, "bonded shear {} N", gc.pairs[0].shear_force);
    assert!(glued.iterations < 150, "{} iterations", glued.iterations);
    assert!(!glued.parts[1].floating && tip(&glued) < 1.0, "bonded: B stays put ({} mm)", tip(&glued));
    let shank = de_shape::fea::stress_at(&glued, [10.5, 5.5, 1.5]).unwrap();
    eprintln!("lap joint: A's shank {shank:.2} MPa (F/A + My/I = 40 MPa)");
    assert!((shank - 40.0).abs() < 40.0 * 0.05, "A's shank {shank} MPa vs 40 MPa");
    // Frictionless: nothing grips sideways, so B slides off and A feels nothing.
    assert!(lc.pairs[0].shear_force < 200.0 * 1e-3, "frictionless shear {} N", lc.pairs[0].shear_force);
    assert!(loose.parts[1].floating && loose.notes.iter().any(|n| n.contains("“B”") && n.contains("slides")), "{:?}", loose.notes);
    assert!(loose.parts[0].max_stress < glued.parts[0].max_stress * 1e-2, "A carries nothing: {} MPa", loose.parts[0].max_stress);
    assert!(tip(&loose) > 100.0 * tip(&glued), "B slides away");
}

#[test]
fn friction_grips_below_mu_n_and_slips_at_mu_n() {
    use de_shape::fea::ContactKind::*;
    // A 10 mm cube pressed onto a base with 1000 N and pushed sideways; μ = 0.3 holds up to 300 N.
    let e = runtime::evaluate("return [part('Base', box(10, 10, 20)), part('Top', box(10, 10, 10).translate(0, 0, 20))];", &[], 0.01).unwrap();
    let held = rect_face(2, 0.0, -1.0, [0.0, 10.0], [0.0, 10.0], Some("Base"));
    let press = load(rect_face(2, 30.0, 1.0, [0.0, 10.0], [0.0, 10.0], Some("Top")), [0.0, 0.0, -1000.0]);
    let push = |t: f64| load(rect_face(0, 10.0, 1.0, [0.0, 10.0], [20.0, 30.0], Some("Top")), [-t, 0.0, 0.0]);
    let steel = bodies(&e, &["Steel", "Steel"]);
    let mu = vec![contact("Base", "Top", Friction, 0.3)];
    let grip = de_shape::fea::solve_parts(&steel, &study(vec![held.clone()], vec![press.clone(), push(200.0)], 30, mu.clone())).unwrap();
    let slip = de_shape::fea::solve_parts(&steel, &study(vec![held], vec![press, push(450.0)], 30, mu)).unwrap();
    let (g, s) = (&grip.contact.as_ref().unwrap().pairs[0], &slip.contact.as_ref().unwrap().pairs[0]);
    eprintln!("friction: 200 N push → shear {:.2} N, slipping {:.0} of {:.0} mm²; 450 N push → shear {:.2} N (μN = 300), slipping {:.0} mm², {} rounds", g.shear_force, g.slipping, g.pressing, s.shear_force, s.slipping, slip.contact.as_ref().unwrap().rounds);
    assert!((g.normal_force - 1000.0).abs() < 2.0 && (s.normal_force - 1000.0).abs() < 2.0, "push {} / {} N", g.normal_force, s.normal_force);
    assert!((g.shear_force - 200.0).abs() < 200.0 * 0.01 && !grip.parts[1].floating, "grips the whole 200 N: {} N", g.shear_force);
    assert!((s.shear_force - 300.0).abs() < 300.0 * 0.01, "slides at μN = 300 N: {} N", s.shear_force);
    assert!(slip.parts[1].floating && (s.slipping - s.pressing).abs() < 1e-6, "slipping everywhere it presses");
}

/// A sketch and where its constraints put everything (every constraint met).
fn solved_sketch(p: &Project, s: StableId) -> (de_shape::sketch::Sketch, de_shape::sketch::Solved) {
    let sk = p.get::<de_shape::sketch::Sketch>(s).unwrap().clone();
    let dims = de_shape::params::resolve(p.world(), &sk.params).unwrap();
    let solved = de_shape::sketch::solve(&sk.geometry, &dims).unwrap();
    assert!(solved.conflicts.is_empty(), "every constraint holds: {:?}", solved.conflicts);
    (sk, solved)
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

/// A 40 × 30 rectangle from the origin, level and upright, its width and height dimensioned.
fn rectangle(p: &mut Project) -> StableId {
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 40, "y": 0 }, { "id": 3, "x": 40, "y": 30 }, { "id": 4, "x": 0, "y": 30 }],
        "curves": [{ "kind": "line", "id": 5, "a": 1, "b": 2 }, { "kind": "line", "id": 6, "a": 2, "b": 3 }, { "kind": "line", "id": 7, "a": 3, "b": 4 }, { "kind": "line", "id": 8, "a": 4, "b": 1 }],
        "constraints": [
            { "kind": "horizontal", "line": 5 }, { "kind": "vertical", "line": 6 }, { "kind": "horizontal", "line": 7 }, { "kind": "vertical", "line": 8 },
            { "kind": "fixed", "point": 1, "x": 0, "y": 0 },
            { "kind": "length", "line": 5, "key": "d1" }, { "kind": "length", "line": 8, "key": "d2" }
        ]
    });
    let params = json!([{ "key": "d1", "label": "Width", "value": 40, "unit": "mm" }, { "key": "d2", "label": "Height", "value": 30, "unit": "mm" }]);
    id(run(p, "sketch.create", json!({ "name": "R", "plane": { "name": "XY" }, "geometry": g, "params": params })).unwrap())
}

#[test]
fn a_fillet_rounds_a_corner_tangent_with_its_radius_a_dimension() {
    use de_shape::sketch::{Constraint, Curve};
    let mut p = project();
    let s = rectangle(&mut p);
    // Round the corner at (40, 0) with r = 5: the arc runs from 5 mm along each side.
    let out = run(&mut p, "sketch.fillet", json!({ "target": s, "point": 2, "radius": 5 })).unwrap();
    let key = out["key"].as_str().unwrap().to_string();
    let arc_id = out["curves"][0].as_u64().unwrap() as u32;
    let arc_ends = |p: &Project| {
        let (sk, sol) = solved_sketch(p, s);
        let Some(Curve::Arc { center, start, end, .. }) = sk.geometry.curves.iter().find(|c| c.id() == arc_id).cloned() else { panic!("no arc") };
        (sol.points[&center], sol.points[&start], sol.points[&end], sk, sol)
    };
    let (c, a, b, sk, sol) = arc_ends(&p);
    assert!(near(c, [35.0, 5.0]), "centre {c:?}");
    assert!(near(a, [35.0, 0.0]) && near(b, [40.0, 5.0]), "anticlockwise from the bottom side to the right side: {a:?} {b:?}");
    assert_eq!(sk.geometry.constraints.iter().filter(|c| matches!(c, Constraint::Tangent { b, .. } if *b == arc_id)).count(), 2, "tangent to both sides");
    assert!(sk.geometry.constraints.iter().any(|c| matches!(c, Constraint::Radius { curve, key: k } if *curve == arc_id && *k == key)));
    assert_eq!(sk.params.iter().find(|q| q.key == key).unwrap().value, 5.0);
    // The bottom side now ends at the round; the width still measures to the virtual sharp.
    let Some(Curve::Line { a: l0, b: l1, .. }) = sk.geometry.curves.iter().find(|c| c.id() == 5).cloned() else { panic!() };
    assert!(near(sol.points[&l0], [0.0, 0.0]) && near(sol.points[&l1], [35.0, 0.0]));
    assert!(near(sol.points[&2], [40.0, 0.0]), "the old corner stays as a virtual sharp");
    assert_eq!(sol.free, 0, "still fully constrained");

    // The radius is a dimension: r = 8 moves the tangent points 8 mm from the corner.
    run(&mut p, "parameters.set", json!({ "target": s, "values": { key.clone(): 8.0 } })).unwrap();
    let (_, a, b, ..) = arc_ends(&p);
    assert!(near(a, [32.0, 0.0]) && near(b, [40.0, 8.0]), "{a:?} {b:?}");
    // And the round follows the rectangle when it grows.
    run(&mut p, "parameters.set", json!({ "target": s, "values": { "d1": 50.0 } })).unwrap();
    let (c, a, b, ..) = arc_ends(&p);
    assert!(near(c, [42.0, 8.0]) && near(a, [42.0, 0.0]) && near(b, [50.0, 8.0]), "{c:?} {a:?} {b:?}");

    // Too large a round for the sides is refused.
    assert!(run(&mut p, "sketch.fillet", json!({ "target": s, "point": 4, "radius": 60 })).is_err());
}

#[test]
fn fillets_between_lines_arcs_and_circles_touch_both_exactly() {
    use de_shape::sketch::Curve;
    let mut p = project();
    // Two lines crossing at the origin, rounded between their right and upper pieces.
    let g = json!({
        "points": [{ "id": 1, "x": -20, "y": 0 }, { "id": 2, "x": 20, "y": 0 }, { "id": 3, "x": 0, "y": -20 }, { "id": 4, "x": 0, "y": 20 }],
        "curves": [{ "kind": "line", "id": 5, "a": 1, "b": 2 }, { "kind": "line", "id": 6, "a": 3, "b": 4 }],
        "constraints": []
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "X", "geometry": g })).unwrap());
    let out = run(&mut p, "sketch.fillet", json!({ "target": s, "curves": [5, 6], "picks": [[10, 0], [0, 10]], "radius": 5 })).unwrap();
    let arc_id = out["curves"][0].as_u64().unwrap() as u32;
    let (sk, sol) = solved_sketch(&p, s);
    let Some(Curve::Arc { center, start, end, .. }) = sk.geometry.curves.iter().find(|c| c.id() == arc_id).cloned() else { panic!() };
    // Anticlockwise round the centre (5, 5) from the upper line to the right one, facing the crossing.
    assert!(near(sol.points[&center], [5.0, 5.0]) && near(sol.points[&start], [0.0, 5.0]) && near(sol.points[&end], [5.0, 0.0]));
    let Some(Curve::Line { a, b, .. }) = sk.geometry.curves.iter().find(|c| c.id() == 5).cloned() else { panic!() };
    assert!(near(sol.points[&a], [5.0, 0.0]) && near(sol.points[&b], [20.0, 0.0]), "the left piece is trimmed away");

    // An arc (r 20 round the origin, from (20, 0) to the top) meeting a line straight up from its top.
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 20, "y": 0 }, { "id": 3, "x": 0, "y": 20 }, { "id": 4, "x": 0, "y": 50 }],
        "curves": [{ "kind": "arc", "id": 5, "center": 1, "start": 2, "end": 3 }, { "kind": "line", "id": 6, "a": 3, "b": 4 }],
        "constraints": []
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "A", "geometry": g })).unwrap());
    let out = run(&mut p, "sketch.fillet", json!({ "target": s, "point": 3, "radius": 5 })).unwrap();
    let arc_id = out["curves"][0].as_u64().unwrap() as u32;
    let (sk, sol) = solved_sketch(&p, s);
    // Centre 5 from the line and 25 from the arc's centre: (5, √600); it touches the arc on the line to its centre.
    let h = 600f64.sqrt();
    let Some(Curve::Arc { center, start, end, .. }) = sk.geometry.curves.iter().find(|c| c.id() == arc_id).cloned() else { panic!() };
    assert!(near(sol.points[&center], [5.0, h]), "{:?}", sol.points[&center]);
    let touches = [sol.points[&start], sol.points[&end]];
    assert!(touches.iter().any(|t| near(*t, [0.0, h])), "touches the line at (0, √600): {touches:?}");
    assert!(touches.iter().any(|t| near(*t, [4.0, h * 0.8])), "touches the arc 20/25 of the way out: {touches:?}");
    let Some(Curve::Arc { end: arc_end, .. }) = sk.geometry.curves.iter().find(|c| c.id() == 5).cloned() else { panic!() };
    assert!(near(sol.points[&arc_end], [4.0, h * 0.8]), "the arc now ends at the round");

    // A line and a whole circle: the circle stays whole, the round outside it above the line.
    let g = json!({
        "points": [{ "id": 1, "x": -40, "y": 0 }, { "id": 2, "x": 40, "y": 0 }, { "id": 3, "x": 0, "y": 0 }],
        "curves": [{ "kind": "line", "id": 5, "a": 1, "b": 2 }, { "kind": "circle", "id": 6, "center": 3, "radius": 20 }],
        "constraints": []
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "C", "geometry": g })).unwrap());
    let out = run(&mut p, "sketch.fillet", json!({ "target": s, "curves": [5, 6], "picks": [[30, 0], [0, 20]], "radius": 5 })).unwrap();
    let arc_id = out["curves"][0].as_u64().unwrap() as u32;
    let (sk, sol) = solved_sketch(&p, s);
    let Some(Curve::Arc { center, .. }) = sk.geometry.curves.iter().find(|c| c.id() == arc_id).cloned() else { panic!() };
    assert!(near(sol.points[&center], [h, 5.0]), "{:?}", sol.points[&center]);
    assert!(sk.geometry.curves.iter().any(|c| matches!(c, Curve::Circle { id: 6, .. })), "the circle stays whole");
}

#[test]
fn a_chamfer_cuts_both_sides_back_by_its_distance() {
    use de_shape::sketch::Curve;
    let mut p = project();
    let s = rectangle(&mut p);
    let out = run(&mut p, "sketch.chamfer", json!({ "target": s, "point": 3, "distance": 5 })).unwrap();
    let cut = out["curves"][0].as_u64().unwrap() as u32;
    let key = out["key"].as_str().unwrap().to_string();
    let (sk, sol) = solved_sketch(&p, s);
    let Some(Curve::Line { a, b, .. }) = sk.geometry.curves.iter().find(|c| c.id() == cut).cloned() else { panic!() };
    let ends = [sol.points[&a], sol.points[&b]];
    assert!(ends.iter().any(|e| near(*e, [40.0, 25.0])) && ends.iter().any(|e| near(*e, [35.0, 30.0])), "{ends:?}");
    assert_eq!(sol.free, 0);
    run(&mut p, "parameters.set", json!({ "target": s, "values": { key: 10.0 } })).unwrap();
    let (_, sol) = solved_sketch(&p, s);
    let ends = [sol.points[&a], sol.points[&b]];
    assert!(ends.iter().any(|e| near(*e, [40.0, 20.0])) && ends.iter().any(|e| near(*e, [30.0, 30.0])), "{ends:?}");
}

#[test]
fn patterns_make_linked_copies_that_follow_the_original() {
    use de_shape::sketch::Curve;
    let mut p = project();
    // A ⌀6 circle at the origin, pinned: four in a row, 10 mm apart.
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }],
        "curves": [{ "kind": "circle", "id": 2, "center": 1, "radius": 3 }],
        "constraints": [{ "kind": "fixed", "point": 1, "x": 0, "y": 0 }, { "kind": "diameter", "curve": 2, "key": "d1" }]
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "Row", "geometry": g, "params": [{ "key": "d1", "label": "Hole", "value": 6, "unit": "mm" }] })).unwrap());
    let out = run(&mut p, "sketch.pattern", json!({ "target": s, "curves": [2], "kind": "linear", "count": 4, "spacing": 10, "direction": [1, 0] })).unwrap();
    let spacing = out["key"].as_str().unwrap().to_string();
    let copies: Vec<u32> = out["curves"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
    assert_eq!(copies.len(), 3);
    let circles = |p: &Project| {
        let (sk, sol) = solved_sketch(p, s);
        copies
            .iter()
            .map(|id| match sk.geometry.curves.iter().find(|c| c.id() == *id).unwrap() {
                Curve::Circle { center, .. } => (sol.points[center], sol.radii[id]),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>()
    };
    for (k, (c, r)) in circles(&p).iter().enumerate() {
        assert!(near(*c, [10.0 * (k + 1) as f64, 0.0]) && (r - 3.0).abs() < 1e-6, "copy {k}: {c:?} r {r}");
    }
    assert_eq!(solved_sketch(&p, s).1.free, 0, "the row is fully defined");
    // The spacing is a dimension, and the copies follow the original's size.
    run(&mut p, "parameters.set", json!({ "target": s, "values": { spacing: 12.0, "d1": 8.0 } })).unwrap();
    for (k, (c, r)) in circles(&p).iter().enumerate() {
        assert!(near(*c, [12.0 * (k + 1) as f64, 0.0]) && (r - 4.0).abs() < 1e-6, "copy {k}: {c:?} r {r}");
    }

    // Circular: a radial line 20–30 mm out (its inner end pinned), four round the origin (90° apart by default).
    let g = json!({
        "points": [{ "id": 1, "x": 20, "y": 0 }, { "id": 2, "x": 30, "y": 0 }],
        "curves": [{ "kind": "line", "id": 3, "a": 1, "b": 2 }],
        "constraints": [{ "kind": "fixed", "point": 1, "x": 20, "y": 0 }, { "kind": "horizontal", "line": 3 }]
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "Ring", "geometry": g })).unwrap());
    let out = run(&mut p, "sketch.pattern", json!({ "target": s, "curves": [3], "kind": "circular", "count": 4, "center": [0, 0] })).unwrap();
    let angle = out["key"].as_str().unwrap().to_string();
    let copies: Vec<u32> = out["curves"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
    let lines = |p: &Project| {
        let (sk, sol) = solved_sketch(p, s);
        copies
            .iter()
            .map(|id| match sk.geometry.curves.iter().find(|c| c.id() == *id).unwrap() {
                Curve::Line { a, b, .. } => (sol.points[a], sol.points[b]),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>()
    };
    let want = [([0.0, 20.0], [0.0, 30.0]), ([-20.0, 0.0], [-30.0, 0.0]), ([0.0, -20.0], [0.0, -30.0])];
    for (got, want) in lines(&p).iter().zip(want) {
        assert!(near(got.0, want.0) && near(got.1, want.1), "{got:?} vs {want:?}");
    }
    // A 60° step: the first copy at 60°, the third at 180°.
    run(&mut p, "parameters.set", json!({ "target": s, "values": { angle: 60.0 } })).unwrap();
    let got = lines(&p);
    let r3 = 3f64.sqrt();
    // (The outer end is free along its level line, so check where the inner one lands and which way the outer one points.)
    let toward = |q: [f64; 2]| q[1].atan2(q[0]).to_degrees();
    assert!(near(got[0].0, [10.0, 10.0 * r3]) && (toward(got[0].1) - 60.0).abs() < 1e-6, "{:?}", got[0]);
    assert!(near(got[2].0, [-20.0, 0.0]) && (toward(got[2].1).abs() - 180.0).abs() < 1e-6, "{:?}", got[2]);
    // Dragging the original's outer end turns its copies with it.
    run(&mut p, "sketch.update", json!({ "target": s, "drag": { "point": 2, "x": 40, "y": 0 } })).unwrap();
    let got = lines(&p);
    assert!(near(got[2].1, [-40.0, 0.0]), "{:?}", got[2]);
}

#[test]
fn a_mirror_copies_curves_reflected_and_keeps_them_symmetric() {
    use de_shape::sketch::Curve;
    let mut p = project();
    // An upright construction line on x = 0, a line from it to the right and an arc beyond.
    let g = json!({
        "points": [{ "id": 1, "x": 0, "y": -50 }, { "id": 2, "x": 0, "y": 50 }, { "id": 3, "x": 0, "y": 10 }, { "id": 4, "x": 20, "y": 10 }, { "id": 5, "x": 20, "y": 0 }, { "id": 6, "x": 30, "y": 0 }],
        "curves": [{ "kind": "line", "id": 7, "a": 1, "b": 2, "construction": true }, { "kind": "line", "id": 8, "a": 3, "b": 4 }, { "kind": "arc", "id": 9, "center": 5, "start": 6, "end": 4 }],
        "constraints": [{ "kind": "vertical", "line": 7 }, { "kind": "fixed", "point": 1, "x": 0, "y": -50 }]
    });
    let s = id(run(&mut p, "sketch.create", json!({ "name": "M", "geometry": g })).unwrap());
    let out = run(&mut p, "sketch.mirror", json!({ "target": s, "curves": [8, 9], "line": 7 })).unwrap();
    let made: Vec<u32> = out["curves"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
    assert_eq!(made.len(), 2);
    let (sk, sol) = solved_sketch(&p, s);
    let Some(Curve::Line { a, b, .. }) = sk.geometry.curves.iter().find(|c| c.id() == made[0]).cloned() else { panic!() };
    assert_eq!(a, 3, "the end on the mirror is shared");
    assert!(near(sol.points[&b], [-20.0, 10.0]));
    let Some(Curve::Arc { center, start, end, .. }) = sk.geometry.curves.iter().find(|c| c.id() == made[1]).cloned() else { panic!() };
    // Reflected, so it runs the other way round: from the mirrored end to the mirrored start.
    assert!(near(sol.points[&center], [-20.0, 0.0]) && near(sol.points[&start], [-20.0, 10.0]) && near(sol.points[&end], [-30.0, 0.0]));
    // Dragging the original moves its reflection.
    run(&mut p, "sketch.update", json!({ "target": s, "drag": { "point": 6, "x": 32, "y": 0 } })).unwrap();
    let (_, sol) = solved_sketch(&p, s);
    assert!(near(sol.points[&6], [32.0, 0.0]) && near(sol.points[&end], [-32.0, 0.0]), "{:?} mirrors {:?}", sol.points[&end], sol.points[&6]);
}

#[test]
fn projecting_a_box_and_a_cylinder_gives_their_outline_on_the_plane() {
    use de_shape::sketch::{Constraint, Curve};
    let mut p = project();
    let block = id(run(&mut p, "shape.create", json!({ "name": "Block", "source": "return box(40, 30, 10);", "params": [] })).unwrap());
    run(&mut p, "solid.part.move", json!({ "target": block, "placement": { "x": { "value": 5, "unit": "mm" }, "y": { "value": 5, "unit": "mm" }, "z": { "value": 0, "unit": "mm" } } })).unwrap();
    let rod = id(run(&mut p, "shape.create", json!({ "name": "Rod", "source": "return cylinder(20, 10).translate(100, 0, 0);", "params": [] })).unwrap());
    let s = id(run(&mut p, "sketch.create", json!({ "name": "Top", "plane": { "name": "XY" } })).unwrap());

    // The block from above: its 40 × 30 rectangle, moved 5 mm in x and y.
    let out = run(&mut p, "sketch.project", json!({ "target": s, "from": block })).unwrap();
    assert_eq!(out["count"], 4, "{out}");
    let (sk, sol) = solved_sketch(&p, s);
    let mut lines: Vec<([f64; 2], [f64; 2])> = sk
        .geometry
        .curves
        .iter()
        .filter_map(|c| match c {
            Curve::Line { a, b, .. } => Some((sol.points[a], sol.points[b])),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 4);
    for c in [[5.0, 5.0], [45.0, 5.0], [45.0, 35.0], [5.0, 35.0]] {
        assert_eq!(lines.iter().filter(|(a, b)| near(*a, c) || near(*b, c)).count(), 2, "two sides meet at {c:?}: {lines:?}");
    }
    lines.retain(|(a, b)| (a[1] - b[1]).abs() < 1e-9);
    assert!(lines.iter().all(|(a, b)| ((a[0] - b[0]).abs() - 40.0).abs() < 1e-6), "the level sides are 40 long");
    assert_eq!(sk.geometry.points.len(), 4, "the corners are shared");
    assert_eq!(sk.geometry.constraints.iter().filter(|c| matches!(c, Constraint::Fixed { .. })).count(), 4, "pinned as reference geometry");
    assert_eq!(sol.free, 0);

    // The rod from above: one circle, r 10 round (100, 0), its size a dimension.
    let out = run(&mut p, "sketch.project", json!({ "target": s, "from": rod })).unwrap();
    let made = out["curves"].as_array().unwrap();
    assert_eq!(made.len(), 1, "{out}");
    let (sk, sol) = solved_sketch(&p, s);
    let circle_id = made[0].as_u64().unwrap() as u32;
    let Some(Curve::Circle { center, .. }) = sk.geometry.curves.iter().find(|c| c.id() == circle_id).cloned() else { panic!("{:?}", sk.geometry.curves) };
    assert!(near(sol.points[&center], [100.0, 0.0]) && (sol.radii[&circle_id] - 10.0).abs() < 1e-6);
    assert_eq!(sol.free, 0);

    // From the front (XZ): the rod's side is a 20 × 20 rectangle; its silhouettes are true lines.
    let front = id(run(&mut p, "sketch.create", json!({ "name": "Front", "plane": { "name": "XZ" } })).unwrap());
    run(&mut p, "sketch.project", json!({ "target": front, "from": rod, "construction": true, "fixed": false })).unwrap();
    let (sk, sol) = solved_sketch(&p, front);
    let xs: Vec<f64> = sol.points.values().map(|q| q[0]).collect();
    let ys: Vec<f64> = sol.points.values().map(|q| q[1]).collect();
    let (x0, x1) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
    let (y0, y1) = (ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max));
    assert!((x0 - 90.0).abs() < 1e-6 && (x1 - 110.0).abs() < 1e-6 && y0.abs() < 1e-6 && (y1 - 20.0).abs() < 1e-6, "{x0} {x1} {y0} {y1}");
    assert!(sk.geometry.curves.iter().all(|c| matches!(c, Curve::Line { construction: true, .. })), "lines only, as construction: {:?}", sk.geometry.curves);

    // A section half-way up the block (and through the rod): the rectangle and the circle.
    let mid = id(run(&mut p, "sketch.create", json!({ "name": "Mid", "plane": { "name": "XY", "offset": 5 } })).unwrap());
    let out = run(&mut p, "sketch.project", json!({ "target": mid, "from": [block, rod], "mode": "section" })).unwrap();
    let (sk, sol) = solved_sketch(&p, mid);
    assert_eq!(sk.geometry.curves.iter().filter(|c| matches!(c, Curve::Line { .. })).count(), 4, "{out}");
    assert_eq!(sk.geometry.curves.iter().filter(|c| matches!(c, Curve::Circle { .. })).count(), 1, "{out}");
    assert!(sol.points.values().any(|q| near(*q, [45.0, 35.0])));
}

#[test]
fn a_loop_of_no_width_is_no_profile_and_does_not_stop_the_sketch() {
    // Two lines drawn over each other between the same points (as overlapping pattern copies make).
    let g: de_shape::sketch::Geometry = serde_json::from_value(json!({
        "points": [{ "id": 1, "x": 0, "y": 0 }, { "id": 2, "x": 40, "y": 0 }],
        "curves": [{ "kind": "line", "id": 3, "a": 1, "b": 2 }, { "kind": "line", "id": 4, "a": 2, "b": 1 }],
        "constraints": []
    }))
    .unwrap();
    let solved = de_shape::sketch::solve(&g, &Default::default()).unwrap();
    let (loops, _) = de_shape::sketch::loops(&g, &solved);
    assert!(loops.is_empty());
}
