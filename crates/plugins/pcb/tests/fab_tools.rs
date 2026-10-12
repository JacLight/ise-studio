//! Fab utilities on the ESP32 example: its panel's files read back by the
//! independent reader (every copy, the panel's own features, a closed
//! profile), the Gerber viewer reading its package, and the board as glTF.

use de_core::{Actor, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value};
use de_pcb::{PcbPlugin, gerber, gerbview, panel, reader};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::Arc;

fn subst(v: &Value, outputs: &[Value]) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') => {
            let (n, path) = s[1..].split_once('.').unwrap();
            outputs[n.parse::<usize>().unwrap()][path].clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| subst(x, outputs)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), subst(x, outputs))).collect()),
        other => other.clone(),
    }
}

fn example() -> (Project, StableId) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("steps.json")).unwrap()).unwrap();
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let mut outs = Vec::new();
    for s in &steps {
        let env = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: "p".into(), expected_revision: p.revision(), operation: s["operation"].as_str().unwrap().into(), parameters: subst(&s["parameters"], &outs), actor: Actor { user_id: "u".into(), session_id: "s".into() } };
        outs.push(p.commit(env).unwrap().output);
    }
    let id: StableId = serde_json::from_value(outs[0]["id"].clone()).unwrap();
    (p, id)
}

#[test]
fn fab_tools_on_the_example() {
    let (p, id) = example();
    let v = de_pcb::view(p.world(), id).unwrap();
    let b = p.get::<de_pcb::Board>(id).cloned().unwrap();

    // ---- Panel: 2 × 3 with mouse bites and rails.
    let spec = panel::Spec { rows: 2, cols: 3, ..panel::Spec::default() };
    let pan = panel::layout(&v.outline, &v.cutouts, &spec).unwrap();
    assert!(pan.problems.is_empty(), "{:?}", pan.problems);
    let files = panel::files(&v, &b, &pan, Some("jlcpcb")).unwrap();
    let board_files = gerber::files(&v);
    let back = panel::readback(&files);
    for r in &back {
        assert!(r["ok"].as_bool().unwrap(), "{r}");
    }
    let text = |name: &str| String::from_utf8(files.iter().find(|f| f.0.ends_with(name)).unwrap().1.clone()).unwrap();
    let board_text = |name: &str| board_files.iter().find(|f| f.0.ends_with(name)).unwrap().1.clone();
    // Top copper: six copies plus three fiducials.
    let one = reader::gerber(&board_text("F_Cu.gtl")).unwrap().objects.len();
    let six = reader::gerber(&text("F_Cu.gtl")).unwrap();
    assert_eq!(six.objects.len(), 6 * one + 3);
    // Every copy's nets are still named.
    assert!(six.objects.iter().filter(|o| o.net.as_deref() == Some("GND")).count() >= 6);
    // NPTH: six copies' holes, the mouse bites and four tooling holes.
    let npth_one = reader::excellon(&board_text("NPTH.drl")).unwrap().hits.len();
    let npth = reader::excellon(&text("NPTH.drl")).unwrap();
    assert_eq!(npth.hits.len(), 6 * npth_one + pan.bites.len() + 4);
    let pth_one = reader::excellon(&board_text("PTH.drl")).unwrap();
    let pth = reader::excellon(&text("-PTH.drl")).unwrap();
    assert_eq!(pth.hits.len(), 6 * pth_one.hits.len());
    assert_eq!(pth.slots.len(), 6 * pth_one.slots.len());
    // The profile is the panel's outline alone, read back as closed strokes.
    let prof = reader::gerber(&text("Edge_Cuts.gm1")).unwrap();
    assert_eq!(prof.objects.len(), pan.outline.len());
    // A part on copy 4 sits where copy 4 is.
    let pos = text("positions.csv");
    let pos_one = de_pcb::outputs::pick_and_place(&v, de_pcb::outputs::Format::Jlcpcb);
    assert_eq!(pos.lines().count(), 1 + 6 * (pos_one.lines().count() - 1));
    assert!(pos.contains("U1_4,"), "{pos}");
    let bom = text("bom.csv");
    assert!(bom.contains("C1_1") && bom.contains("C2_6"), "{bom}");

    // ---- V-score panel.
    let spec = panel::Spec { rows: 2, cols: 2, separation: "vscore".into(), frame: "frame".into(), ..panel::Spec::default() };
    let vs = panel::layout(&v.outline, &v.cutouts, &spec).unwrap();
    let files = panel::files(&v, &b, &vs, None).unwrap();
    let vcut = files.iter().find(|f| f.0.ends_with("V_Score.gbr")).unwrap();
    let g = reader::gerber(&String::from_utf8_lossy(&vcut.1)).unwrap();
    assert_eq!(g.attributes.get("FileFunction").map(String::as_str), Some("Vcut"));
    assert_eq!(g.objects.len(), vs.vcuts.len());

    // ---- The Gerber viewer on the board's own package (as a zip).
    let zip = de_pcb::outputs::zip(&de_pcb::outputs::package(&v, &b, None));
    let set = gerbview::read(&gerbview::unpack("board.zip", &zip).unwrap());
    assert!(set.unreadable.is_empty(), "{:?}", set.unreadable);
    assert_eq!(set.layers.len(), board_files.len());
    assert!(set.layers.iter().all(|l| !l.guessed));
    assert!(set.nets.iter().any(|n| n == "GND"));
    assert!(set.parts.iter().any(|n| n == "U1"));
    // Size matches the board outline (the profile's 0.1 mm stroke adds half its width each side).
    assert!(((set.bounds[2] - set.bounds[0]) - (v.bounds[2] - v.bounds[0]) - 0.1).abs() < 0.02, "{:?} vs {:?}", set.bounds, v.bounds);
    // Compared with itself: nothing differs.
    let same = gerbview::compare(&board_files.iter().map(|(n, s)| (n.clone(), s.clone().into_bytes())).collect::<Vec<_>>(), &gerbview::unpack("board.zip", &zip).unwrap(), 0.05);
    assert!(same.iter().all(|d| d.same), "{:?}", same.iter().filter(|d| !d.same).map(|d| (&d.function, d.added, d.removed)).collect::<Vec<_>>());

    // ---- The board as glTF.
    let glb = de_pcb::glb::glb(&v, &b, &de_pcb::items(p.world(), id));
    assert_eq!(&glb[0..4], b"glTF");
    let jl = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let doc: Value = serde_json::from_slice(&glb[20..20 + jl]).unwrap();
    let names: Vec<&str> = doc["meshes"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"board") && names.contains(&"pads") && names.contains(&"silkscreen"), "{names:?}");
    // The board's extent in metres (x along the board, glTF z is −y).
    let lo = doc["accessors"][0]["min"][0].as_f64().unwrap();
    let hi = doc["accessors"][0]["max"][0].as_f64().unwrap();
    assert!(((hi - lo) * 1000.0 - (v.bounds[2] - v.bounds[0])).abs() < 0.01);
}
