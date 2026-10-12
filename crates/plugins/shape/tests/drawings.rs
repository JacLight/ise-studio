//! Shop drawings of the example parts (examples/drawings, examples/gearbox),
//! checked the way a checker reads them: every feature dimensioned once,
//! nothing said twice, every word legible and clear of every other.
//! DRAWING_OUT=dir writes the PDFs and DXFs to look at.

use de_shape::drawing::{self, Drawing, Options, Sheet, Title};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples")
}

/// A shape program as an example's steps.json creates it (its `$file` sources joined, its parameters).
fn program(dir: &str, name: &str) -> de_shape::ShapeProgram {
    let dir = examples().join(dir);
    let steps: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("steps.json")).unwrap()).unwrap();
    let step = steps.as_array().unwrap().iter().find(|s| s["operation"] == "shape.create" && s["parameters"]["name"] == name).unwrap_or_else(|| panic!("no shape {name}"));
    let p = &step["parameters"];
    let source = match &p["source"] {
        Value::String(s) => s.clone(),
        v => {
            let files: Vec<String> = match &v["$file"] {
                Value::String(f) => vec![f.clone()],
                Value::Array(a) => a.iter().map(|f| f.as_str().unwrap().to_string()).collect(),
                _ => panic!("source"),
            };
            files.iter().map(|f| std::fs::read_to_string(dir.join(f)).unwrap()).collect::<Vec<_>>().join("\n")
        }
    };
    let params: Vec<de_shape::Param> = serde_json::from_value(p["params"].clone()).unwrap_or_default();
    de_shape::ShapeProgram { name: name.into(), description: String::new(), source, params, display_tolerance: None, levels: vec![], uses: Default::default(), finishes: vec![], inputs: String::new() }
}

fn title(name: &str, material: &str) -> Title {
    Title { title: name.into(), number: "FD-001".into(), material: material.into(), mass: "—".into(), drawn_by: "Forma".into(), date: "2026-10-12".into(), ..Default::default() }
}

fn write(d: &[Drawing], name: &str) {
    if let Ok(dir) = std::env::var("DRAWING_OUT") {
        std::fs::write(format!("{dir}/{name}.pdf"), drawing::pdf_pages(d)).unwrap();
        for (i, s) in d.iter().enumerate() {
            std::fs::write(format!("{dir}/{name}-{}.dxf", i + 1), drawing::dxf(s)).unwrap();
            std::fs::write(format!("{dir}/{name}-{}.json", i + 1), serde_json::to_string_pretty(s).unwrap()).unwrap();
        }
    }
}

fn texts(d: &Drawing) -> Vec<String> {
    d.texts.iter().map(|t| t.text.clone()).collect()
}

#[test]
fn a_machined_bracket_is_dimensioned_for_the_shop() {
    let program = program("drawings", "Bearing bracket");
    let options: Options = serde_json::from_value(serde_json::json!({
        "sheet": "A3",
        "threads": [{ "diameter": 5, "size": "M6", "class": "6H", "depth": 10 }],
        "hole_fits": [{ "diameter": 8, "fit": "H7" }, { "diameter": 22, "fit": "H7" }],
        "roughness": { "ra": 3.2 },
        "finishes": [{ "view": "right", "at": [40, 61], "ra": 0.8, "process": "reamed" }],
        "notes": ["Remove all burrs and break sharp edges 0.2–0.5.", "Anodise clear to MIL-A-8625 Type II after machining; mask the ⌀22 H7 bore."],
        "revisions": [{ "rev": "A", "description": "First issue", "date": "2026-10-12", "by": "Forma" }],
        "title_block": { "company": "Forma examples", "finish": "Anodise clear" },
    }))
    .unwrap();
    let d = drawing::drawing(&program, &options, &title("Bearing bracket", "Aluminium 6061-T6")).unwrap();
    write(std::slice::from_ref(&d), "bracket");
    let all = texts(&d);
    let keys: Vec<&str> = d.autos.iter().map(|a| a.key.as_str()).collect();
    eprintln!("{all:?}\nautos: {keys:?}\nwarnings: {:?}\nquality: {:?}", d.warnings, d.quality);
    // Every step, wall and hole centre placed once: the flange (15), tapped holes (25, 110 · 10, 70),
    // pocket walls (35, 95 · 20, 60), dowels (65), counterbore (40), pocket floor (9), base (15), seat (50).
    let want = ["x:15", "x:25", "x:35", "x:65", "x:95", "x:110", "x:120", "y:10", "y:20", "y:40", "y:60", "y:70", "y:80", "z:9", "z:15", "z:50", "z:75", "hole:22", "hole:8", "hole:6.6", "hole:5", "r:8"];
    let mut sorted = keys.clone();
    sorted.sort();
    let mut wanted = want.to_vec();
    wanted.sort();
    assert_eq!(sorted, wanted, "every feature dimensioned, nothing else");
    for t in ["4× M6-6H ", "⌀5 ", "2× ⌀8 H7", "⌀6.6 THRU", "⌀11 ", "4× R8", "⌀22 H7", "Ra 0.8", "Ra 3.2", "NOTES", "First issue", "Anodise clear"] {
        assert!(all.iter().any(|x| x == t), "{t}");
    }
    assert!(d.quality.is_clean(), "{:?}", d.quality);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.scale_text, "1:1", "a 120 mm bracket at full size on A3");
}

#[test]
fn a_turned_shaft_is_dimensioned_for_the_shop() {
    let program = program("drawings", "Output shaft");
    let options: Options = serde_json::from_value(serde_json::json!({
        "sheet": "A3",
        "threads": [{ "diameter": 16, "size": "M16×1.5", "class": "6g" }],
        "hole_fits": [{ "diameter": 20, "fit": "g6" }, { "diameter": 22, "fit": "k6" }],
    }))
    .unwrap();
    let d = drawing::drawing(&program, &options, &title("Output shaft", "Steel C45")).unwrap();
    write(std::slice::from_ref(&d), "shaft");
    let all = texts(&d);
    let keys: Vec<&str> = d.autos.iter().map(|a| a.key.as_str()).collect();
    eprintln!("{all:?}\nautos: {keys:?}\nwarnings: {:?}\nquality: {:?}", d.warnings, d.quality);
    for k in ["d:16", "d:13.5", "d:20", "d:19", "d:26", "d:22", "d:18", "x:20", "x:24", "x:40", "x:41.3", "x:44", "x:54", "x:94", "x:110", "c:1@0", "c:1@53", "c:1@109"] {
        assert!(keys.contains(&k), "{k}");
    }
    assert_eq!(keys.len(), 18, "nothing else");
    for t in ["M16×1.5-6g", "⌀20 g6", "−0.007", "−0.020", "⌀22 k6", "+0.015", "+0.002", "1×45°"] {
        assert!(all.iter().any(|x| x == t), "{t}");
    }
    // A turned part shows its profile once (and the isometric).
    assert_eq!(d.views.len(), 1);
    assert!(d.quality.is_clean() && d.warnings.is_empty(), "{:?} {:?}", d.quality, d.warnings);
    // The thread drawn ISO 6410: thin lines at the minor diameter (13.16 for M16×1.5) along it.
    let s = d.scale;
    assert!(d.strokes.iter().filter(|st| st.pen == drawing::Pen::Thin && st.points.len() == 2 && (st.points[0][1] - st.points[1][1]).abs() < 1e-9 && (st.points[1][0] - st.points[0][0]).abs() > 15.0 * s).count() >= 2);
    let _ = Sheet::A3;
}

/// The parts list as the engine makes it: numbered copies of one part (same
/// material, process and volume) are one row.
fn rows(program: &de_shape::ShapeProgram) -> Vec<drawing::PartRow> {
    let values: Vec<(String, f64)> = program.params.iter().map(|p| (p.key.clone(), p.value)).collect();
    let (arena, exact) = de_shape::exact::evaluate_exact_in(de_occt::Arena::new(), &program.name, &program.source, &values, 0.05, "").map_err(|(_, e)| e).unwrap();
    let base = |n: &str| n.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end().to_string();
    let mut out: Vec<(drawing::PartRow, f64)> = vec![];
    for p in &exact.parts {
        let v = arena.volume(p.shape);
        let material = p.material.clone().unwrap_or_default();
        match out.iter_mut().find(|(r, rv)| (r.name == p.name || base(&r.name) == base(&p.name)) && r.material == material && (*rv - v).abs() < 1e-6 * v.max(1.0)) {
            Some((r, _)) => {
                r.quantity += 1;
                r.members.push(p.name.clone());
                r.name = base(&p.name);
            }
            None => out.push((drawing::PartRow { name: p.name.clone(), quantity: 1, material, mass: "—".into(), process: p.process.clone().unwrap_or_default(), members: vec![p.name.clone()] }, v)),
        }
    }
    out.into_iter().map(|r| r.0).collect()
}

#[test]
fn an_assembly_drawing_explodes_and_balloons_every_part() {
    let program = program("gearbox", "Two-stage spur reducer");
    let options: Options = serde_json::from_value(serde_json::json!({ "sheet": "A3", "explode": { "axis": "z" } })).unwrap();
    let parts = rows(&program);
    let t = Title { parts: parts.clone(), ..title("Two-stage spur reducer", "—") };
    let d = drawing::drawing(&program, &options, &t).unwrap();
    write(std::slice::from_ref(&d), "gearbox");
    assert!(parts.iter().any(|r| r.name == "Spacer" && r.quantity == 4), "four spacers are one row");
    assert_eq!(d.balloons.len(), parts.len(), "a balloon for every row");
    assert!(texts(&d).iter().any(|t| t.starts_with("EXPLODED VIEW")));
    assert!(d.quality.is_clean() && d.warnings.is_empty(), "{:?} {:?}", d.quality, d.warnings);
    eprintln!("rows {:?}\nwarnings: {:?}\nquality: {:?}", parts.iter().map(|r| (&r.name, r.quantity)).collect::<Vec<_>>(), d.warnings, d.quality);
}

#[test]
fn threads_read_and_minor_diameters() {
    use drawing::features::{coarse_pitch, minor_diameter, parse_thread};
    assert_eq!(parse_thread("M6").unwrap(), (6.0, 1.0));
    assert_eq!(parse_thread("M16×1.5").unwrap(), (16.0, 1.5));
    assert_eq!(parse_thread("m10x1.25").unwrap(), (10.0, 1.25));
    assert!(parse_thread("UNC 1/4").is_err() && parse_thread("M7").is_err(), "no coarse pitch for M7");
    assert_eq!(coarse_pitch(12.0), Some(1.75));
    assert!((minor_diameter(16.0, 2.0, false) - 13.546).abs() < 1e-3, "ISO 68-1 d3");
}

#[test]
fn an_exploded_stack_slides_parts_clear() {
    // A long shaft through the middle stays; a gear on it and a plate beyond slide out, clear of each other.
    let boxes = [[-5.0, -5.0, 0.0, 5.0, 5.0, 100.0], [-20.0, -20.0, 60.0, 20.0, 20.0, 70.0], [-30.0, -30.0, 90.0, 30.0, 30.0, 95.0], [-30.0, -30.0, 5.0, 30.0, 30.0, 10.0]];
    let s = drawing::explode_shifts(&boxes, 2, 10.0);
    assert_eq!(s[0], 0.0, "the shaft stays");
    assert_eq!(s[1], 0.0, "the gear nearest the middle has nothing to clear");
    assert!(boxes[2][2] + s[2] >= boxes[1][5] + s[1] + 10.0 - 1e-9, "the plate clears the gear by the gap");
    assert!(s[3] <= 0.0, "the near plate slides the other way");
}

#[test]
fn a_drawing_has_every_kind_of_view_and_annotation() {
    // A long plate with a hole and a slanted end face.
    let program = de_shape::ShapeProgram {
        name: "Plate".into(),
        description: String::new(),
        source: "const s = box(200, 40, 20).subtract(cylinder(30, 6).translate([20, 20, -5]));\nconst cut = box(40, 60, 40).rotate(0, 30, 0).translate([170, -10, 12]);\nreturn s.subtract(cut);".into(),
        params: vec![],
        display_tolerance: None,
        levels: vec![],
        uses: Default::default(),
        finishes: vec![],
        inputs: String::new(),
    };
    let options: Options = serde_json::from_value(serde_json::json!({
        "sheet": "ANSI_B",
        "section_view": "top", "section_at": 10, "hatch": { "angle": 30, "spacing": 3, "cross": true },
        "breaks": [{ "axis": "x", "from": 60, "to": 140 }],
        "aux": [{ "dir": [-0.5, 0, -0.866] }],
        "leaders": [{ "view": "front", "at": [100, 20], "text": "Deburr" }],
        "texts": [{ "at": [30, 262], "text": "PROTOTYPE", "size": 5, "bold": true }],
        "tables": [{ "title": "Hole table", "rows": [["TAG", "X", "Y", "SIZE"], ["A1", "20", "20", "⌀12 THRU"]] }],
        "lines": [{ "view": "front", "points": [[0, 10], [200, 10]], "style": "center" }],
        "angles": [{ "view": "front", "vertex": [180, 20], "a": [160, 20], "b": [190, 2.7] }],
        "arcs": [{ "view": "top", "centre": [20, 20], "radius": 6, "from": 0, "to": 90, "length": true }],
        "areas": [{ "view": "top", "at": [100, 20] }],
        "welds": [{ "view": "front", "at": [0, 10], "kind": "fillet", "size": 4, "all_around": true }],
    }))
    .unwrap();
    let d = drawing::drawing(&program, &options, &title("Plate", "Steel")).unwrap();
    write(std::slice::from_ref(&d), "plate-everything");
    let all = texts(&d);
    eprintln!("{all:?}\n{:?}\n{:?}\n{:?}", d.warnings, d.quality, d.autos.iter().map(|a| &a.key).collect::<Vec<_>>());
    assert_eq!((d.width, d.height), (431.8, 279.4), "ANSI B");
    for want in ["SECTION A–A", "PROTOTYPE", "Hole table", "Deburr", "a4", "VIEW B"] {
        assert!(all.iter().any(|t| t.contains(want)), "{want} is on the sheet");
    }
    assert!(all.iter().any(|t| t.starts_with("A = ") && t.ends_with("mm²")), "the area callout");
    assert!(all.iter().any(|t| t.ends_with('°')), "the angle");
    // The break leaves 80 mm out but every position is stated true: the same as the whole part's.
    let whole = drawing::drawing(&program, &Options { extra: drawing::Extras { breaks: vec![], ..options.extra.clone() }, ..options.clone() }, &title("Plate", "Steel")).unwrap();
    let xs = |d: &Drawing| {
        let mut v: Vec<String> = d.autos.iter().filter(|a| a.key.starts_with("x:")).map(|a| a.key.clone()).collect();
        v.sort();
        v
    };
    let (broken, full) = (xs(&d), xs(&whole));
    assert!(!broken.is_empty() && broken.iter().all(|k| full.contains(k)), "{broken:?} vs {full:?}");
    assert!(d.quality.is_clean(), "{:?}", d.quality);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    let svg = drawing::svg(&d);
    assert!(svg.starts_with("<svg") && svg.contains("PROTOTYPE") && svg.trim_end().ends_with("</svg>"));
}
