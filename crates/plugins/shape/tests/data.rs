//! Spreadsheets, tables in and out, and OpenSCAD files: through commands, as
//! people and the AI use them, and through both geometry kernels.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_shape::{ResolvedParams, ShapePlugin, ShapeSolid, exact, runtime, sheet::SheetValues, table};
use de_solid::SolidPlugin;
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

fn shown(p: &Project, sheet: StableId, at: &str) -> String {
    p.get::<SheetValues>(sheet).unwrap().cells[at].shown.clone()
}

/// What a formula's result is in follows what it does with what it reads: a
/// lever's length times the sine of a joint's angle is a plain number, not
/// an angle (the robot arm's torque sheet showed "79.4°" for a length).
#[test]
fn a_formula_result_takes_its_unit_from_what_it_does() {
    let mut p = project();
    run(&mut p, "parameters.create", json!({ "name": "Arm", "params": [
        { "key": "angle", "label": "Angle", "value": 30, "unit": "deg" },
        { "key": "reach", "label": "Reach", "value": 200, "unit": "mm" }
    ] }))
    .unwrap();
    let s = run(&mut p, "sheet.create", json!({ "name": "Check", "cells": {
        "A1": "=Arm.angle", "B1": "=75 * sin(A1)", "C1": "=Arm.reach / 4", "D1": "=Arm.reach / Arm.reach",
        "E1": "=atan2(1, 1)", "F1": "=sqrt(Arm.reach^2 + 75^2 - 2 * Arm.reach * 75 * cos(A1))", "G1": "=Arm.reach * cos(A1)",
        "A2": "=0.25", "B2": "=A1", "C2": "40"
    }, "units": { "A2": "%", "B2": "number", "C2": "%" } }))
    .unwrap();
    assert!(s["errors"].as_object().unwrap().is_empty(), "{s}");
    let sheet = id(&s);
    assert_eq!(shown(&p, sheet, "A1"), "30°");
    assert_eq!(shown(&p, sheet, "B1"), "37.5", "a sine is a plain number");
    assert_eq!(shown(&p, sheet, "C1"), "50 mm", "a length over a number");
    assert_eq!(shown(&p, sheet, "D1"), "1", "a length over a length");
    assert_eq!(shown(&p, sheet, "E1"), "45°");
    assert_eq!(shown(&p, sheet, "F1"), "140.157903", "the law of cosines on numbers");
    assert_eq!(shown(&p, sheet, "G1"), "173.205081 mm", "a length times a cosine is still a length");
    assert_eq!(shown(&p, sheet, "A2"), "25 %");
    assert_eq!(shown(&p, sheet, "B2"), "30", "a cell can say it holds a plain number");
    assert_eq!(shown(&p, sheet, "C2"), "40 %", "a bare number in a % cell is a percentage");
    assert!((p.get::<ResolvedParams>(sheet).unwrap().0["C2"] - 0.4).abs() < 1e-12);
}

#[test]
fn a_sheet_calculates_and_drives_shapes_through_links() {
    let mut p = project();
    let desk = id(&run(&mut p, "parameters.create", json!({ "name": "Desk", "params": [{ "key": "width", "label": "Width", "value": 1600 }] })).unwrap());
    let s = run(
        &mut p,
        "sheet.create",
        json!({
            "name": "Sizes",
            "cells": {
                "A1": "Leg", "B1": "2in",
                "A2": "Gap", "B2": "=Desk.width / 4",
                "A3": "Rail", "B3": "=B2 - 2*leg + 5mm",
                "A4": "Total", "B4": "=SUM(B1:B3)",
                "A5": "Mean", "B5": "=average(B1:B3)",
                "B6": "1.2 m", "B7": "3", "B8": "=b7*2"
            },
            "aliases": { "B1": "leg", "B3": "rail" },
            "units": { "B7": "in" }
        }),
    )
    .unwrap();
    let sheet = id(&s);
    assert!(s["errors"].as_object().unwrap().is_empty(), "{s}");
    assert_eq!(shown(&p, sheet, "B1"), "2 in");
    assert_eq!(shown(&p, sheet, "B2"), "400 mm", "a length from the desk's width");
    assert_eq!(shown(&p, sheet, "B3"), "303.4 mm", "a length, shown in mm");
    assert_eq!(shown(&p, sheet, "A1"), "Leg");
    assert_eq!(shown(&p, sheet, "B6"), "1.2 m");
    assert_eq!(shown(&p, sheet, "B7"), "3 in", "a bare number in an inch cell is inches");
    assert_eq!(shown(&p, sheet, "B8"), "6 in");
    let values = p.get::<ResolvedParams>(sheet).unwrap().0.clone();
    assert!((values["B4"] - (50.8 + 400.0 + 303.4)).abs() < 1e-9);
    assert!((values["B5"] - (50.8 + 400.0 + 303.4) / 3.0).abs() < 1e-9);
    assert!((values["rail"] - 303.4).abs() < 1e-9);

    // A shape follows the sheet's alias; the sheet follows the desk.
    let rail = id(&run(&mut p, "shape.create", json!({ "name": "Rail", "source": "return box(p.len, 20, 20);", "params": [{ "key": "len", "label": "Length", "value": 1, "expr": "=Sizes.rail" }] })).unwrap());
    assert_eq!(p.get::<ShapeSolid>(rail).unwrap().stats.bounds_max[0], 303.4);
    run(&mut p, "parameters.set", json!({ "target": desk, "values": { "width": 2000 } })).unwrap();
    assert_eq!(p.get::<ShapeSolid>(rail).unwrap().stats.bounds_max[0], 403.4);
    run(&mut p, "sheet.set", json!({ "target": sheet, "cells": { "B1": "10" } })).unwrap();
    assert_eq!(p.get::<ShapeSolid>(rail).unwrap().stats.bounds_max[0], 485.0);
    // References by cell address work too, in any case.
    run(&mut p, "shape.link_param", json!({ "target": rail, "key": "len", "expr": "=Sizes.b2 + 1" })).unwrap();
    assert_eq!(p.get::<ShapeSolid>(rail).unwrap().stats.bounds_max[0], 501.0);

    // Formulas show names; renaming the desk keeps the link.
    run(&mut p, "parameters.create", json!({ "name": "Other", "params": [] })).unwrap();
    assert_eq!(p.get::<SheetValues>(sheet).unwrap().cells["B2"].formula.as_deref(), Some("=Desk.width / 4"));

    // Errors stay in their cell; unknown names and loops are refused.
    let r = run(&mut p, "sheet.set", json!({ "target": sheet, "cells": { "C1": "=A1 * 2", "C2": "=C3", "C3": "=C2" } })).unwrap();
    assert_eq!(r["errors"].as_object().unwrap().len(), 3, "{r}");
    assert!(p.get::<SheetValues>(sheet).unwrap().cells["C1"].error.as_ref().unwrap().contains("text"));
    let e = run(&mut p, "sheet.set", json!({ "target": sheet, "cells": { "C4": "=Nowhere.x" } })).unwrap_err();
    assert!(e.message.contains("C4"), "{}", e.message);
    run(&mut p, "parameters.create", json!({ "name": "Back", "params": [{ "key": "x", "label": "X", "value": 0, "expr": "=Sizes.B4" }] })).unwrap();
    let e = run(&mut p, "sheet.set", json!({ "target": sheet, "cells": { "C5": "=Back.x" } })).unwrap_err();
    assert!(e.message.contains("loop"), "{}", e.message);
    let e = run(&mut p, "sheet.set", json!({ "target": sheet, "aliases": { "C9": "AB1" } })).unwrap_err();
    assert!(e.message.contains("address"), "{}", e.message);

    // Export: the grid as shown, and a workbook whose formulas still work.
    let sh = p.get::<de_shape::sheet::Sheet>(sheet).unwrap().clone();
    let v = p.get::<SheetValues>(sheet).unwrap().clone();
    let rows = de_shape::sheet::rows(&sh, &v);
    assert_eq!(rows[0][..2], ["Leg".to_string(), "10".to_string()]);
    let x = table::read_xlsx(&de_shape::sheet::to_xlsx(&sh, &v), None).unwrap();
    assert_eq!(x.cells[&(1, 3)].input, "=SUM(B1:B3)");
    assert_eq!(x.cells[&(1, 1)].input, "500", "a formula reading the desk goes as its value");
    assert_eq!(x.names["rail"], (1, 2));
}

#[test]
fn tables_come_in_from_csv_and_xlsx() {
    let mut p = project();
    let csv = "Part,Length,Qty\nRail,600 mm,2\nPost,=B2/2,4\nTotal,=sum(B2:B3),\n";
    let r = run(&mut p, "sheet.import", json!({ "name": "Cut list", "format": "csv", "text": csv })).unwrap();
    let s = id(&r);
    assert_eq!(shown(&p, s, "B3"), "300 mm");
    assert_eq!(shown(&p, s, "B4"), "900 mm");

    // A workbook as Excel writes it: shared strings, a defined name, a formula Iṣẹ́ Studio cannot read.
    let mut cells = std::collections::BTreeMap::new();
    cells.insert((0, 0), table::OutCell::Text("Span".into()));
    cells.insert((1, 0), table::OutCell::Number(2400.0));
    cells.insert((1, 1), table::OutCell::Formula("B1/3".into(), Some(800.0)));
    cells.insert((1, 2), table::OutCell::Formula("IF(B1>2000,1,0)".into(), Some(1.0)));
    let mut names = std::collections::BTreeMap::new();
    names.insert("span".to_string(), (1, 0));
    let bytes = table::write_xlsx(&[table::OutSheet { name: "Beam".into(), cells, names, bold_rows: vec![] }]);
    let b64 = base64(&bytes);
    let r = run(&mut p, "sheet.import", json!({ "name": "Beam", "format": "xlsx", "base64": b64 })).unwrap();
    let s = id(&r);
    assert_eq!(shown(&p, s, "B2"), "800");
    assert_eq!(shown(&p, s, "B3"), "1", "kept as its value");
    assert_eq!(r["notes"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(p.get::<ResolvedParams>(s).unwrap().0["span"], 2400.0);

    // Parameter sets fill from a table and update in place.
    let r = run(&mut p, "parameters.import", json!({ "name": "Shelf", "format": "csv", "text": "key,value,unit,min,max\nwidth,800,mm,300,1200\nangle,15,deg,,\n" })).unwrap();
    let set = id(&r);
    assert_eq!(r["added"], 2);
    let r = run(&mut p, "parameters.import", json!({ "target": set, "format": "csv", "text": "width,900\nshelves,4\n" })).unwrap();
    assert_eq!((r["added"].as_u64(), r["updated"].as_u64()), (Some(1), Some(1)));
    let v = &p.get::<ResolvedParams>(set).unwrap().0;
    assert_eq!((v["width"], v["shelves"]), (900.0, 4.0));
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { A[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn scad(src: &str) -> runtime::Evaluated {
    let prog = format!("return openscad({});", serde_json::to_string(src).unwrap());
    runtime::evaluate(&prog, &[], 0.01).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn scad_err(src: &str) -> String {
    let prog = format!("return openscad({});", serde_json::to_string(src).unwrap());
    runtime::evaluate(&prog, &[], 0.01).err().expect("should fail")
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

#[test]
fn openscad_language() {
    // Primitives, booleans, transforms.
    let e = scad("difference() { cube([20, 20, 10]); translate([5, 5, -1]) cube([10, 10, 12]); }");
    assert!(close(e.volume, 4000.0 - 1000.0, 1e-9), "{}", e.volume);
    let e = scad("cube(10, center = true);");
    assert_eq!(e.bounds_min, [-5.0, -5.0, -5.0]);
    // Modules with defaults, children(), for over ranges, variables (the last assignment wins).
    let e = scad(
        "n = 2; module row(k = 3, gap = 15) { for (i = [0 : k - 1]) translate([i * gap, 0, 0]) children(); }
         row(n) cube(10); n = 4;",
    );
    assert!(close(e.volume, 4000.0, 1e-9), "four cubes: {}", e.volume);
    // Functions, recursion, list comprehensions, let, ternary, vectors.
    let e = scad(
        "function fact(n) = n <= 1 ? 1 : n * fact(n - 1);
         sizes = [for (i = [1:3]) let(s = i * 2) if (s > 2) s];
         for (s = sizes) translate([s * 10, 0, 0]) cube([s, s, fact(3) / 6]);
         echo(len(sizes), norm([3, 4]) + [1, 2] * [3, 4]);",
    );
    assert!(close(e.volume, 16.0 + 36.0, 1e-9), "{}", e.volume);
    // $fn = 6 is a true hexagon (a nut trap stays six-sided), round otherwise.
    let e = scad("cylinder(h = 5, r = 10, $fn = 6);");
    assert!(close(e.volume, 3.0 * 3f64.sqrt() / 2.0 * 100.0 * 5.0, 1e-9), "{}", e.volume);
    let e = scad("cylinder(h = 10, d = 20);");
    assert!(close(e.volume, std::f64::consts::PI * 100.0 * 10.0, 2e-3), "{}", e.volume);
    // 2D, extrusions (twist, scale), offsets.
    let e = scad("linear_extrude(height = 10, center = true) square([10, 4], center = true);");
    assert!(close(e.volume, 400.0, 1e-9) && e.bounds_min[2] == -5.0);
    let e = scad("rotate_extrude() translate([10, 0]) square([2, 5]);");
    assert!(close(e.volume, std::f64::consts::PI * (144.0 - 100.0) * 5.0, 3e-3), "{}", e.volume);
    let e = scad("linear_extrude(10) offset(delta = 1) square(8);");
    assert!(close(e.volume, 1000.0, 1e-9), "{}", e.volume);
    let e = scad("linear_extrude(height = 10, scale = 0.5) square(10, center = true);");
    assert!(close(e.volume, 10.0 / 3.0 * (100.0 + 25.0 + 50.0), 1e-6), "frustum {}", e.volume);
    // polygon with a hole path, polyhedron.
    let e = scad("linear_extrude(1) polygon(points = [[0,0],[10,0],[10,10],[0,10],[2,2],[8,2],[8,8],[2,8]], paths = [[0,1,2,3],[4,5,6,7]]);");
    assert!(close(e.volume, 100.0 - 36.0, 1e-9), "{}", e.volume);
    let e = scad("polyhedron(points = [[0,0,0],[10,0,0],[0,10,0],[0,0,10]], faces = [[0,1,2],[0,3,1],[0,2,3],[1,3,2]]);");
    assert!(close(e.volume, 1000.0 / 6.0, 1e-9), "{}", e.volume);
    // Axis-angle rotation and multmatrix keep the volume; minkowski grows a box by a sphere.
    let e = scad("rotate(a = 30, v = [1, 1, 0]) cube([10, 20, 5]);");
    assert!(close(e.volume, 1000.0, 1e-9));
    let e = scad("multmatrix([[1, 0.5, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0]]) cube(10);");
    assert!(close(e.volume, 1000.0, 1e-9), "a shear keeps volume: {}", e.volume);
    let e = scad("minkowski() { cube(10); sphere(1); }");
    assert!(e.volume > 1000.0 + 6.0 * 100.0 && e.volume < 12.0 * 12.0 * 12.0, "{}", e.volume);
    let e = scad("hull() { cube(1); translate([10, 0, 0]) cube(1); }");
    assert!(close(e.volume, 11.0, 1e-9));
    let e = scad("intersection() { cube(10); translate([5, 5, 5]) cube(10); }");
    assert!(close(e.volume, 125.0, 1e-9));
    // Modifiers: * disables, % is background (not part of the model).
    let e = scad("cube(1); *cube(100); %cube(50);");
    assert!(close(e.volume, 1.0, 1e-9));
    // resize, mirror, color → parts.
    let e = scad("resize([20, 0, 0], auto = true) cube([10, 5, 5]);");
    assert!(close(e.volume, 20.0 * 10.0 * 10.0, 1e-9), "{}", e.volume);
    let e = scad("color(\"red\") cube(5); color(\"blue\") translate([10, 0, 0]) cube(5);");
    assert_eq!(e.parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Red part", "Blue part"]);

    // Errors say what and where.
    assert!(scad_err("cube(10);\nfoo(1);").contains("line 2: unknown module foo()"));
    assert!(scad_err("square(10);").contains("2D"));
    assert!(scad_err("assert(1 > 2, \"too big\");").contains("too big"));
    assert!(scad_err("import(\"x.stl\");").contains("import()"));
}

#[test]
fn openscad_exact_kernel() {
    // The exact kernel builds the same: true cylinders, hexagon prisms, transforms split into exact steps.
    let src = "difference() { cube([30, 30, 10]); translate([15, 15, -1]) cylinder(h = 12, r = 5); translate([5, 5, -1]) cylinder(h = 12, r = 3, $fn = 6); }
               translate([40, 0, 0]) rotate(a = 90, v = [0, 1, 0]) mirror([0, 0, 1]) scale([1, 2, 1]) cube(5);";
    let prog = format!("return openscad({});", serde_json::to_string(src).unwrap());
    let e = exact::evaluate_exact("Plate", &prog, &[], 0.01).unwrap();
    let volume: f64 = e.parts.iter().map(|p| e.arena.volume(p.shape)).sum();
    let hex = 3.0 * 3f64.sqrt() / 2.0 * 9.0 * 10.0;
    let expected = 9000.0 - std::f64::consts::PI * 25.0 * 10.0 - hex + 250.0;
    assert!(close(volume, expected, 1e-6), "exact {volume} vs {expected}");
    assert!(e.faceted.is_empty(), "{:?}", e.faceted);
    let mesh = runtime::evaluate(&prog, &[], 0.01).unwrap();
    assert!(close(mesh.volume, expected, 2e-3));
    let b = e.arena.bounds(e.parts.last().unwrap().shape).unwrap();
    let mb = mesh.bounds_max;
    assert!((b[3] - mb[0]).abs() < 1e-6 && (b[5] - mb[2]).abs() < 1e-6, "{b:?} vs {mb:?}");
}

#[test]
fn an_imported_scad_file_stays_parametric() {
    let mut p = project();
    let src = "// Plate width\nwidth = 40; // [20:100]\nholes = 3;\nmodule plate() difference() { cube([width, 20, 4]); for (i = [1:holes]) translate([i * width / (holes + 1), 10, -1]) cylinder(h = 6, r = 2); }\nplate();\n";
    let r = run(&mut p, "shape.import_scad", json!({ "source": src, "file": "mount_plate.scad" })).unwrap();
    let shape = id(&r);
    let prog = p.get::<de_shape::ShapeProgram>(shape).unwrap().clone();
    assert_eq!(prog.name, "Mount plate");
    assert_eq!(prog.params.iter().map(|q| (q.key.as_str(), q.unit.as_str(), q.min, q.max)).collect::<Vec<_>>(), [("width", "mm", Some(20.0), Some(100.0)), ("holes", "count", None, None)]);
    assert_eq!(prog.params[0].label, "Plate width");
    let v = |p: &Project| p.get::<ShapeSolid>(shape).unwrap().stats.volume_mm3;
    let hole = std::f64::consts::PI * 4.0 * 4.0;
    assert!(close(v(&p), 40.0 * 80.0 - 3.0 * hole, 2e-3), "{}", v(&p));
    run(&mut p, "shape.set_params", json!({ "target": shape, "values": { "width": 80, "holes": 5 } })).unwrap();
    assert!(close(v(&p), 80.0 * 80.0 - 5.0 * hole, 2e-3), "{}", v(&p));
    let back = de_shape::scad::export_source(&p.get::<de_shape::ShapeProgram>(shape).unwrap().source, &[("width".into(), 80.0), ("holes".into(), 5.0)]).unwrap();
    assert!(back.starts_with("// Plate width") && back.ends_with("width = 80;\nholes = 5;\n"));
    let e = run(&mut p, "shape.import_scad", json!({ "source": "cube(1" })).unwrap_err();
    assert!(e.message.contains("does not read"), "{}", e.message);
}
