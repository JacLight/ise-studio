//! Routing constraints through their commands: impedance-targeted net
//! classes, a USB pair routed together on a 4-layer stackup, a bus tuned to
//! one length, and the checks (class targets, pairs, groups and the custom
//! rules' length, skew, diff-pair, via-count and impedance constraints).

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::PcbPlugin;
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::Arc;

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

fn ok(p: &mut Project, operation: &str, parameters: Value) -> Value {
    run(p, operation, parameters.clone()).unwrap_or_else(|e| panic!("{operation} {parameters}: {e}"))
}

fn id(v: &Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

/// Errors other than connections still to route.
fn codes(p: &Project, b: StableId) -> Vec<String> {
    let v = de_pcb::view(p.world(), b).unwrap();
    v.violations.iter().filter(|x| x.severity == "error" && x.code != "unrouted").map(|x| format!("{}: {}", x.code, x.message)).collect()
}

/// The proof board: a USB-C connector, a QFN MCU and a SOIC flash on JLCPCB's 4-layer 7628 stackup.
fn board() -> (Project, StableId) {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "usb", "width": 40, "height": 26, "layers": 4, "model": false })));
    ok(
        &mut p,
        "pcb.stackup.set",
        json!({ "board": b, "dielectrics": [
            { "kind": "prepreg", "thickness": 0.2104, "material": "7628", "epsilon_r": 4.4 },
            { "kind": "core", "thickness": 1.065, "material": "FR4", "epsilon_r": 4.6 },
            { "kind": "prepreg", "thickness": 0.2104, "material": "7628", "epsilon_r": 4.4 }], "copper_oz": [1, 0.5, 0.5, 1] }),
    );
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "J1", "footprint": "USB-C_16P", "at": [12, 3.2], "pins": { "A6": "USB_D+", "A7": "USB_D-" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "U1", "footprint": "QFN-32_5x5", "at": [17, 18], "pins": { "12": "USB_D+", "13": "USB_D-", "20": "D0", "19": "D1", "18": "D2", "17": "D3" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "U2", "footprint": "SOIC-8", "at": [34, 10], "pins": { "1": "D0", "2": "D1", "3": "D2", "4": "D3" } }));
    (p, b)
}

#[test]
fn class_targets_come_from_the_stackup() {
    let (mut p, b) = board();
    let out = ok(&mut p, "pcb.netclass.set", json!({ "board": b, "name": "USB", "patterns": ["USB_D*"], "diff_impedance": 90, "diff_gap": 0.15 }));
    let layers = out["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 4);
    for l in layers {
        let z = l["zdiff"].as_f64().unwrap();
        assert!((z - 90.0).abs() < 0.5, "{l}");
        assert_eq!(l["gap"].as_f64().unwrap(), 0.15);
    }
    // Outer microstrip is wider than inner stripline for the same impedance.
    assert!(layers[0]["width"].as_f64().unwrap() > layers[1]["width"].as_f64().unwrap());
    // The class takes the nets by pattern; the pair is found from the names.
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.nets.iter().find(|n| n.name == "USB_D+").unwrap().class, "USB");
    assert_eq!(v.signal.pairs.len(), 1);
    assert_eq!((v.signal.pairs[0].p.as_str(), v.signal.pairs[0].n.as_str()), ("USB_D+", "USB_D-"));
    // The router lays a USB track at the class's top-layer width.
    let r = ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "J1.A6", "to": [12, 9] }));
    assert!((r["width"].as_f64().unwrap() - layers[0]["width"].as_f64().unwrap()).abs() < 1e-9, "{r}");
}

#[test]
fn a_pair_routes_together_through_coupled_vias() {
    let (mut p, b) = board();
    ok(&mut p, "pcb.netclass.set", json!({ "board": b, "name": "USB", "patterns": ["USB_D*"], "diff_impedance": 90, "diff_gap": 0.15, "max_uncoupled": 5, "max_skew": 0.1 }));
    let out = ok(&mut p, "pcb.route.pair", json!({ "board": b, "pair": "USB_D", "steps": [{ "at": [12, 10.5] }, { "at": [12, 12.5], "layer": "in1" }, { "at": [15, 14], "layer": "top" }] }));
    assert_eq!(out["ok"], true, "{out}");
    let v = de_pcb::view(p.world(), b).unwrap();
    let pair = &v.signal.pairs[0];
    assert!(pair.routed, "{pair:?}");
    assert_eq!(v.vias.len(), 4, "two coupled vias at each layer change");
    // Both tracks keep the gap wherever they run side by side, on every layer.
    for g in &pair.gaps {
        assert!((g.min - 0.15).abs() < 1e-3 && (g.max - 0.15).abs() < 1e-3, "{g:?}");
    }
    assert!(pair.coupled > 9.0, "{pair:?}");
    // Via barrels count, from the stackup: top to in1 is 0.2104 mm plus half of each copper.
    let l = v.signal.lengths.iter().find(|l| l.net == "USB_D+").unwrap();
    assert!((l.via_length - 2.0 * (0.2104 + (0.035 + 0.0175) / 2.0)).abs() < 1e-6, "{l:?}");
    // The jogs to the vias leave some skew: tuned out on the shorter track.
    let before = pair.skew;
    assert!(before > 0.01, "{before}");
    let t = ok(&mut p, "pcb.tune", json!({ "board": b, "pair": "USB_D", "mode": "skew" }));
    assert_eq!(t["ok"], true, "{t}");
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.signal.pairs[0].skew < 0.001, "{:?}", v.signal.pairs[0]);
    assert!(codes(&p, b).is_empty(), "{:?}", codes(&p, b));
}

#[test]
fn a_bus_is_tuned_to_one_length() {
    let (mut p, b) = board();
    ok(&mut p, "pcb.netclass.set", json!({ "board": b, "name": "BUS", "patterns": ["D?"], "impedance": 60 }));
    for (net, from, x, ys, to) in [("D0", "U1.20", 29.5, 11.905, "U2.1"), ("D1", "U1.19", 27.0, 10.635, "U2.2"), ("D2", "U1.18", 24.5, 9.365, "U2.3"), ("D3", "U1.17", 22.0, 8.095, "U2.4")] {
        let y0 = match net {
            "D0" => 17.75,
            "D1" => 17.25,
            "D2" => 16.75,
            _ => 16.25,
        };
        ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": from, "to": [{ "at": [x, y0] }, { "at": [x, ys] }, { "at": to }] }));
    }
    ok(&mut p, "pcb.length.group", json!({ "board": b, "name": "BUS", "nets": ["D0", "D1", "D2", "D3"], "tolerance": 0.5, "max_skew": 0.5 }));
    let bad = codes(&p, b);
    assert!(bad.iter().any(|c| c.starts_with("skew: Group BUS")), "{bad:?}");
    assert_eq!(bad.iter().filter(|c| c.starts_with("length:")).count(), 3, "{bad:?}");
    let out = ok(&mut p, "pcb.tune", json!({ "board": b, "group": "BUS", "amplitude": 1.0 }));
    assert_eq!(out["ok"], true, "{out}");
    let v = de_pcb::view(p.world(), b).unwrap();
    let g = &v.signal.groups[0];
    assert!(g.skew < 1e-3, "{g:?}");
    // The meanders' arcs are counted exactly: every member lands on the longest's length.
    for m in &g.members {
        assert!((m.length - g.target).abs() < 1e-3, "{m:?} vs {}", g.target);
    }
    assert!(codes(&p, b).is_empty(), "{:?}", codes(&p, b));
}

#[test]
fn custom_rules_check_lengths_vias_skew_and_impedance() {
    let (mut p, b) = board();
    ok(&mut p, "pcb.netclass.set", json!({ "board": b, "name": "USB", "patterns": ["USB_D*"], "diff_impedance": 90, "diff_gap": 0.15 }));
    ok(&mut p, "pcb.route.pair", json!({ "board": b, "pair": "USB_D", "steps": [{ "at": [12, 10.5] }, { "at": [12, 12.5], "layer": "in1" }, { "at": [15, 14], "layer": "top" }] }));
    ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "U1.20", "to": [{ "at": [29.5, 17.75] }, { "at": [29.5, 11.905] }, { "at": "U2.1" }] }));
    ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "U1.19", "to": [{ "at": [27.0, 17.25] }, { "at": [27.0, 10.635] }, { "at": "U2.2" }] }));
    let rules = r#"(version 1)
        (rule "short USB" (condition "A.inDiffPair('USB_D')") (constraint length (max 10mm)))
        (rule "no vias on USB" (condition "A.NetClass == 'USB'") (constraint via_count (max 0)))
        (rule "pair gap" (condition "A.isCoupledDiffPair()") (constraint diff_pair_gap (min 0.2mm)))
        (rule "bus skew" (condition "A.NetName == 'D?'") (constraint skew (max 0.1mm)))
        (rule "bus 50 ohm" (condition "A.NetName == 'D?'") (constraint impedance (min 45) (max 55)))"#;
    let out = ok(&mut p, "pcb.rules.set", json!({ "board": b, "text": rules }));
    assert!(out["unchecked"].as_array().unwrap().is_empty(), "{out}");
    let bad = codes(&p, b);
    let has = |code: &str, rule: &str| bad.iter().any(|c| c.starts_with(code) && c.contains(rule));
    assert!(has("length:", "short USB"), "{bad:?}");
    assert!(has("via_count:", "no vias on USB"), "{bad:?}");
    assert!(has("diff_pair_gap:", "pair gap"), "{bad:?}");
    assert!(has("skew:", "bus skew"), "{bad:?}");
    assert!(has("impedance:", "bus 50 ohm"), "{bad:?}");
    // A USB track at the wrong width breaks the class's impedance.
    ok(&mut p, "pcb.rules.set", json!({ "board": b, "text": "" }));
    ok(&mut p, "pcb.route.unroute", json!({ "board": b, "nets": ["USB_D+", "USB_D-"] }));
    ok(&mut p, "pcb.route.pair", json!({ "board": b, "pair": "USB_D", "width": 0.5, "gap": 0.2, "mode": "highlight" }));
    let bad = codes(&p, b);
    assert!(bad.iter().any(|c| c.starts_with("impedance:") && c.contains("USB_D")), "{bad:?}");
}

#[test]
fn exports_carry_the_controlled_impedances() {
    let (mut p, b) = board();
    ok(&mut p, "pcb.netclass.set", json!({ "board": b, "name": "USB", "patterns": ["USB_D*"], "diff_impedance": 90, "diff_gap": 0.15, "max_skew": 0.1 }));
    ok(&mut p, "pcb.length.group", json!({ "board": b, "name": "BUS", "nets": ["D?"], "tolerance": 0.5 }));
    let v = de_pcb::view(p.world(), b).unwrap();
    let board = p.get::<de_pcb::Board>(b).unwrap().clone();
    let note = de_pcb::signal::fab_note(&board, &v).expect("a fab note");
    assert!(note.contains("IMPEDANCE CONTROL") && note.contains("90 Ω") && note.contains("differential"), "{note}");
    let readme = de_pcb::outputs::readme(&v, &board, None);
    assert!(readme.contains("IMPEDANCE CONTROL"));
    let ipc = de_pcb::io::ipc2581::write(&v, &board);
    assert!(ipc.contains("EDGE_COUPLED") && ipc.contains("SpecRef"), "IPC-2581 specs");
    let dru = de_pcb::signal::dru_rules(&board, &v);
    assert!(dru.contains("diff_pair_gap") && dru.contains("(layer \"F.Cu\")") && dru.contains("length group BUS"), "{dru}");
    let rules = de_pcb::rules::parse(&format!("(version 1)\n{dru}")).expect("KiCad-readable rules");
    assert!(rules.rules.len() >= 4);
    let pro = de_pcb::io::kicad_pro::write(&v, &board);
    assert!(pro.contains("\"diff_pair_gap\": 0.15") && pro.contains("USB_D*"), "{pro}");
    let job = de_pcb::io::gbrjob::write(&v, &board);
    assert!(job.contains("\"ImpedanceControlled\": true"));
    let odb = de_pcb::io::odb::files(&v, &board);
    assert!(odb.iter().any(|(n, x)| n.ends_with("matrix/stackup.xml") && String::from_utf8_lossy(x).contains("DIFFERENTIAL")));
}
