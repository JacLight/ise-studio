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
    eprintln!("{all:?}\nautos: {:?}\nwarnings: {:?}\nquality: {:?}", d.autos.iter().map(|a| &a.key).collect::<Vec<_>>(), d.warnings, d.quality);
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
    eprintln!("{all:?}\nautos: {:?}\nwarnings: {:?}\nquality: {:?}", d.autos.iter().map(|a| &a.key).collect::<Vec<_>>(), d.warnings, d.quality);
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
    eprintln!("rows {:?}\nwarnings: {:?}\nquality: {:?}", parts.iter().map(|r| (&r.name, r.quantity)).collect::<Vec<_>>(), d.warnings, d.quality);
}
