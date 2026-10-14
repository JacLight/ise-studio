//! The ESP32 example simulated with ngspice: regulator start-up, EN reset
//! timing (10 k / 1 µF), and the 3V3 rail under a 500 mA Wi-Fi burst.
//! Skipped (with a note) where ngspice is not installed.

use de_core::{Actor, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value};
use de_pcb::PcbPlugin;
use de_pcb::spice::{self, Analysis, SimRequest};
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

fn schematic() -> Arc<de_pcb::schematic_view::SchematicView> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("schematic-steps.json")).unwrap()).unwrap();
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let mut outs = Vec::new();
    // The schematic is all that is simulated: stop before routing.
    for s in steps.iter().take(3) {
        let env = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: "p".into(), expected_revision: p.revision(), operation: s["operation"].as_str().unwrap().into(), parameters: subst(&s["parameters"], &outs), actor: Actor { user_id: "u".into(), session_id: "s".into() } };
        outs.push(p.commit(env).unwrap().output);
    }
    let id: StableId = serde_json::from_value(outs[1]["id"].clone()).unwrap();
    de_pcb::schematic::view(p.world(), id).unwrap()
}

#[test]
fn the_netlist_models_every_power_part() {
    let v = schematic();
    let nl = spice::netlist(&v, &SimRequest::default());
    assert!(nl.text.contains("ise_ldo") && nl.text.contains("X_U2 n_vbus 0 n_vbus n_3v3"), "{}", nl.text);
    assert!(nl.text.contains("X_U1 n_3v3 0 n_en ise_load"));
    assert!(nl.text.contains("R_R3 n_3v3 n_en 10000") || nl.text.contains("R_R3 n_en n_3v3 10000"), "{}", nl.text);
    assert!(nl.text.contains("V_J1 n_vbus_src 0"));
    assert!(nl.parts.iter().any(|p| p.reference == "U2" && p.approximate && p.source.contains("ME6211")));
    assert!(nl.skipped.iter().any(|s| s.starts_with("J2")), "connectors have no model: {:?}", nl.skipped);
}

#[test]
fn the_esp32_board_starts_resets_and_rides_the_wifi_burst() {
    if spice::ngspice().is_none() {
        eprintln!("ngspice is not installed: skipped");
        return;
    }
    let v = schematic();
    let req = SimRequest { analysis: Analysis::Tran { stop: 0.03, step: Some(5e-6) }, expect: vec!["min(V(3V3), burst) > 3.0".into(), "cross(V(EN), 2.475) > 10ms".into(), "rise(V(3V3)) < 1ms".into()], ..Default::default() };
    let r = spice::simulate(&v, &req).unwrap_or_else(|e| panic!("{e}"));
    for c in r.checks.iter().chain(&r.expectations).chain(&r.measurements) {
        eprintln!("{:5} {}: {} {}", c.status, c.name, c.detail, c.limit);
    }
    for w in &r.windows {
        eprintln!("window {} {:.6}…{:.6}", w.name, w.from, w.to);
    }
    let burst = r.windows.iter().find(|w| w.name == "burst").expect("the module ran and burst");
    assert!(burst.from > 0.013 && burst.from < 0.025, "EN releases about 14 ms in (10 k × 1 µF to 0.75·VDD), the burst 5 ms later: {}", burst.from);
    assert!(r.expectations.iter().all(|e| e.status == "pass"), "{:#?}", r.expectations);
    assert!(r.checks.iter().any(|c| c.name.contains("released from reset") && c.status == "pass"));
    // The rail dips but stays in Espressif's range.
    let v3 = r.signals.iter().find(|s| s.name == "V(3V3)").unwrap();
    let fin = *v3.values.last().unwrap();
    assert!((fin - 3.3).abs() < 0.05, "{fin}");
    // DC sweep and AC run too.
    let dc = spice::simulate(&v, &SimRequest { analysis: Analysis::Dc { net: "VBUS".into(), from: 0.0, to: 6.0, step: 0.1 }, ..Default::default() }).unwrap();
    assert!(dc.x.len() > 50);
    let ac = spice::simulate(&v, &SimRequest { analysis: Analysis::Ac { net: "VBUS".into(), from: 10.0, to: 1e6, points: Some(10) }, ..Default::default() }).unwrap();
    assert!(ac.signals.iter().any(|s| s.unit == "dB"));
}
