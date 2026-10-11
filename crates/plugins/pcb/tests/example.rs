//! The worked example, `examples/pcb-esp32/steps.json`, built through the
//! same commands the AI uses: it must route, pass DRC and write fab files
//! that read back. `FORMA_WRITE_EXAMPLE=1` also writes the package to
//! `examples/pcb-esp32/fab/`.

use de_core::{Actor, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value};
use de_pcb::{PcbPlugin, outputs, reader};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::Arc;

fn substitute(v: &Value, outputs: &[Value]) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') => {
            let (n, path) = s[1..].split_once('.').unwrap();
            let n: usize = n.parse().unwrap();
            outputs[n][path].clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, outputs)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), substitute(x, outputs))).collect()),
        other => other.clone(),
    }
}

#[test]
fn the_esp32_example_routes_passes_drc_and_exports() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("steps.json")).unwrap()).unwrap();
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let mut outs = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let params = substitute(&s["parameters"], &outs);
        let env = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: "p".into(), expected_revision: p.revision(), operation: s["operation"].as_str().unwrap().into(), parameters: params, actor: Actor { user_id: "u".into(), session_id: "s".into() } };
        let out = p.commit(env).unwrap_or_else(|e| panic!("step {i} {}: {e}", s["operation"])).output;
        if s["operation"] == "pcb.autoroute" {
            eprintln!("autoroute: {out}");
        }
        outs.push(out);
    }
    let board: StableId = serde_json::from_value(outs[0]["id"].clone()).unwrap();
    let v = de_pcb::view(p.world(), board).expect("worked out");
    for x in &v.violations {
        eprintln!("{} {}: {}", x.severity, x.code, x.message);
    }
    assert!(v.components.len() >= 20, "{}", v.components.len());
    assert_eq!(v.errors(), 0, "the example passes DRC");
    assert!(v.nets.iter().all(|n| n.routed), "every net routed");
    assert!(!v.zones[0].fill.is_empty());
    let b = p.get::<de_pcb::Board>(board).unwrap().clone();
    let files = outputs::package(&v, &b, Some("jlcpcb"));
    for (n, bytes) in &files {
        let text = std::str::from_utf8(bytes).unwrap();
        if n.ends_with(".drl") {
            reader::excellon(text).unwrap_or_else(|e| panic!("{n}: {e}"));
        } else if n.starts_with(&de_pcb::gerber::file_stem(&v.name)) {
            reader::gerber(text).unwrap_or_else(|e| panic!("{n}: {e}"));
        }
    }
    let jlc = de_pcb::fabs::check(de_pcb::fabs::fab("jlcpcb").unwrap(), &v);
    assert!(jlc.can_make, "{:#?}", jlc.checks);
    // The 3D board evaluates with every body.
    let model = b.model.unwrap();
    let solid = p.get::<de_shape::ShapeSolid>(model).unwrap_or_else(|| panic!("{:?}", p.status(model)));
    assert!(solid.stats.parts.len() >= 20);
    if std::env::var("FORMA_WRITE_EXAMPLE").is_ok() {
        let dir = root.join("fab");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (n, bytes) in &files {
            std::fs::write(dir.join(n), bytes).unwrap();
        }
    }
}
