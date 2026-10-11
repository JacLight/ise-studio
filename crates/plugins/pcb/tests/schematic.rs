//! Schematic capture: net extraction, ERC, schematic → board sync, pin maps,
//! and the ESP32 example rebuilt from a schematic with the same nets.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::schematic::{self, SchematicView};
use de_pcb::{PcbPlugin, symbols};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::collections::BTreeMap;
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])))
}

fn run(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, CommandError> {
    let envelope = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: p.id().into(), expected_revision: p.revision(), operation: operation.into(), parameters, actor: Actor { user_id: "u".into(), session_id: "s".into() } };
    p.commit(envelope).map(|c| c.output)
}

fn ok(p: &mut Project, operation: &str, parameters: Value) -> Value {
    run(p, operation, parameters.clone()).unwrap_or_else(|e| panic!("{operation} {parameters}: {e}"))
}

fn id(v: &Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

fn view(p: &Project, s: StableId) -> Arc<SchematicView> {
    schematic::view(p.world(), s).unwrap_or_else(|| panic!("schematic not worked out: {:?}", p.status(s)))
}

fn net_of<'a>(v: &'a SchematicView, pin: &str) -> Option<&'a str> {
    let (r, n) = pin.split_once('.').unwrap();
    v.symbols.iter().filter(|s| s.reference == r).flat_map(|s| s.pins.iter()).find(|p| p.number == n).and_then(|p| p.net.as_deref())
}

fn codes(v: &SchematicView) -> Vec<&'static str> {
    v.violations.iter().map(|x| x.code).collect()
}

fn sch(p: &mut Project) -> StableId {
    id(&ok(p, "pcb.schematic.create", json!({ "name": "Test" })))
}

#[test]
fn nets_follow_wires_ts_junctions_labels_and_power_ports() {
    let mut p = project();
    let s = sch(&mut p);
    // Three resistors in a row, upright: pins at y ±3.81 around y = 50.
    for (r, x) in [("R1", 50.0), ("R2", 70.0), ("R3", 90.0)] {
        ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": r, "at": [x, 50.8] }));
    }
    // R1.1 to R2.1 by an auto-routed wire.
    let w = ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "from": "R1.1", "to": "R2.1" }));
    let pts: Vec<[f64; 2]> = serde_json::from_value(w["points"].clone()).unwrap();
    assert!(pts.windows(2).all(|q| (q[0][0] - q[1][0]).abs() < 1e-6 || (q[0][1] - q[1][1]).abs() < 1e-6), "orthogonal: {pts:?}");
    // A T onto that wire's middle from R3.1? No: R3.1 joins by a T wire ending on the horizontal run.
    let run_y = pts.iter().map(|q| q[1]).fold(f64::MAX, f64::min);
    ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "points": [[90.0, 46.99], [90.0, run_y], [60.96, run_y]] }));
    // A wire crossing the R1–R2 run without a junction stays separate…
    ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "points": [[66.04, run_y - 5.08], [66.04, run_y + 2.54]] }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "text": "CROSS", "at": [66.04, run_y - 5.08] }));
    // Power ports on R1.2 and R3.2 join by name; a local label names R2.2.
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "power", "text": "GND", "pin": "R1.2" }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "power", "text": "GND", "pin": "R3.2" }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "text": "SIG", "pin": "R2.2" }));
    let v = view(&p, s);
    let top = net_of(&v, "R1.1").unwrap();
    assert_eq!(net_of(&v, "R2.1"), Some(top));
    assert_eq!(net_of(&v, "R3.1"), Some(top), "the T joins");
    assert!(top.starts_with("Net-(R1-Pad1)"), "{top}");
    assert_eq!(net_of(&v, "R1.2"), Some("GND"));
    assert_eq!(net_of(&v, "R3.2"), Some("GND"));
    assert_eq!(net_of(&v, "R2.2"), Some("SIG"));
    let cross = v.wires.iter().find(|w| w.points[0][0] == 66.04 && w.points.len() == 2).unwrap();
    assert_eq!(cross.net, None, "a crossing without a junction does not join (and has no pin)");
    assert!(v.junctions.iter().any(|j| j.id.is_none()), "the T gets a dot");
    // A junction where they cross joins them.
    ok(&mut p, "pcb.schematic.junction", json!({ "schematic": s, "at": [66.04, run_y] }));
    let v = view(&p, s);
    let cross = v.wires.iter().find(|w| w.points[0][0] == 66.04 && w.points.len() == 2).unwrap();
    assert_eq!(cross.net.as_deref(), Some("CROSS"));
    assert_eq!(net_of(&v, "R1.1"), Some("CROSS"), "the label now names the joined net");
    // Same local label on another sheet is a different net; a global label is the same.
    let sh = ok(&mut p, "pcb.schematic.sheet", json!({ "schematic": s, "name": "Two" }));
    let two = sh["sheet"].as_str().unwrap().to_string();
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "sheet": two, "symbol": "C", "reference": "C1", "at": [50.8, 50.8] }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "text": "SIG", "pin": "C1.1" }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "power", "text": "GND", "pin": "C1.2" }));
    let v = view(&p, s);
    assert_eq!(net_of(&v, "C1.2"), Some("GND"), "power ports join across sheets");
    assert_ne!(net_of(&v, "C1.1"), Some("SIG"), "local labels stay on their sheet");
    let gnd = v.nets.iter().find(|n| n.name == "GND").unwrap();
    assert_eq!(gnd.pins, vec!["C1.2", "R1.2", "R3.2"]);
}

#[test]
fn erc_catches_seeded_errors() {
    let mut p = project();
    let s = sch(&mut p);
    // Two LDOs with their outputs shorted, nothing driving their inputs, an unconnected pin,
    // a duplicate reference, a lonely typo label, and a part with no footprint.
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "LDO_SOT223", "reference": "U1", "at": [50.8, 50.8] }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "LDO_SOT223", "reference": "U2", "at": [101.6, 50.8] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "3V3", "pins": ["U1.VO", "U2.VO"] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": ["U1.GND", "U2.GND"] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "VIN", "pins": ["U1.VI"] }));
    // U2.VI left unconnected.
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R1", "at": [50.8, 101.6] }));
    let e = run(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R1", "at": [76.2, 101.6] }));
    assert!(e.is_err(), "placing a duplicate is refused");
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R9", "at": [76.2, 101.6] }));
    let c5 = id(&ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "C", "reference": "C5", "at": [127.0, 101.6] })));
    ok(&mut p, "pcb.schematic.symbol.update", json!({ "target": c5, "footprint": "" }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "SDA", "pins": ["R1.1"] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "SDAA", "pins": ["R1.2"] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "SDA", "pins": ["R9.1"] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": ["R9.2", "C5.1", "C5.2"] }));
    let r9 = view(&p, s).symbols.iter().find(|x| x.reference == "R9").unwrap().id;
    assert!(run(&mut p, "pcb.schematic.symbol.update", json!({ "target": r9, "reference": "R1" })).is_err(), "renaming onto a taken reference is refused");
    let p2 = p;
    let v = view(&p2, s);
    let c = codes(&v);
    for code in ["pin_conflict", "power_not_driven", "unconnected_pin", "label_unmatched", "missing_footprint"] {
        assert!(c.contains(&code), "missing {code}: {:#?}", v.violations);
    }
    let conflict = v.violations.iter().find(|x| x.code == "pin_conflict").unwrap();
    assert!(conflict.message.contains("3V3") && conflict.refs.len() == 2, "{conflict:?}");
    let un = v.violations.iter().find(|x| x.code == "unconnected_pin").unwrap();
    assert_eq!(un.refs, vec!["U2.3"]);
    assert!(un.objects.len() == 1);
    let typo = v.violations.iter().find(|x| x.code == "label_unmatched").unwrap();
    assert!(typo.message.contains("SDAA") && typo.message.contains("did you mean SDA"), "{}", typo.message);
    let pd = v.violations.iter().find(|x| x.code == "power_not_driven" && x.message.starts_with("GND")).expect("GND undriven");
    assert!(pd.refs.contains(&"U1.1".to_string()));
    // A PWR_FLAG on GND and VIN clears the undriven errors for them.
    let mut p = p2;
    let f = ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "PWR_FLAG", "reference": "#FLG1", "at": [25.4, 25.4] }));
    assert_eq!(view(&p, s).symbols.iter().find(|x| x.id == id(&f)).unwrap().virtual_part, true);
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": ["#FLG1.1"], "length": 0 }));
    let v = view(&p, s);
    assert!(!v.violations.iter().any(|x| x.code == "power_not_driven" && x.message.starts_with("GND")), "{:#?}", v.violations);
    // No-connect flag clears the unconnected pin.
    ok(&mut p, "pcb.schematic.no_connect", json!({ "schematic": s, "pin": "U2.3" }));
    assert!(!codes(&view(&p, s)).contains(&"unconnected_pin"));
}

#[test]
fn duplicate_references_and_pin_maps_are_checked() {
    let mut p = project();
    let s = sch(&mut p);
    let a = id(&ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R1", "at": [50.8, 50.8] })));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R2", "at": [76.2, 50.8] }));
    // A pin map onto a pad the footprint lacks, and a footprint with unmapped pads.
    ok(&mut p, "pcb.schematic.symbol.update", json!({ "target": a, "pin_map": { "2": ["3"] } }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "Conn_01x02", "reference": "J1", "at": [101.6, 50.8], "footprint": "PinHeader_1x03_2.54" }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "C", "reference": "C?", "at": [127.0, 50.8] }));
    let v = view(&p, s);
    let maps: Vec<_> = v.violations.iter().filter(|x| x.code == "pin_map").collect();
    assert!(maps.iter().any(|m| m.severity == "error" && m.refs == vec!["R1"] && m.message.contains("2 → 3")), "{maps:#?}");
    assert!(maps.iter().any(|m| m.severity == "warning" && m.refs == vec!["J1"] && m.message.contains("pads 3")), "{maps:#?}");
    assert!(codes(&v).contains(&"not_annotated"));
    // Library symbols map onto their default footprints.
    for n in symbols::names() {
        let d = symbols::get(&n).unwrap();
        assert!(d.virtual_part || d.footprint.is_empty() || de_pcb::footprints::get(&d.footprint).is_some(), "{n}");
    }
    // Annotation numbers C? as C1.
    let out = ok(&mut p, "pcb.schematic.annotate", json!({ "schematic": s }));
    assert_eq!(out["renamed"][0]["to"], "C1");
    // Multi-unit parts share a reference; a second unit A is refused.
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "OpAmp_Dual", "reference": "U1", "unit": 1, "at": [50.8, 101.6] }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "OpAmp_Dual", "reference": "U1", "unit": 2, "at": [76.2, 101.6] }));
    assert!(run(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "OpAmp_Dual", "reference": "U1", "unit": 2, "at": [101.6, 101.6] })).is_err());
    let v = view(&p, s);
    assert!(v.violations.iter().any(|x| x.code == "missing_unit" && x.message.contains("C")), "{:#?}", v.violations);
}

fn board_nets(p: &Project, board: StableId) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for (_, c) in de_pcb::items(p.world(), board).components {
        for (pad, net) in c.pins {
            m.insert(format!("{}.{pad}", c.reference), net);
        }
    }
    m
}

#[test]
fn update_board_reports_every_change() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "B", "width": 40, "height": 30 })));
    let s = id(&ok(&mut p, "pcb.schematic.create", json!({ "board": b })));
    // A board-only mounting hole and a stray part already on the board.
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "H1", "footprint": "MountingHole_M3", "at": [4, 4] }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R9", "footprint": "R_0603", "value": "1k", "at": [20, 20], "pins": { "1": "X" } }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R1", "value": "10k", "at": [50.8, 50.8] }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "LED", "reference": "D1", "at": [76.2, 50.8] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "3V3", "pins": ["R1.1"] }));
    ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "from": "R1.2", "to": "D1.2" }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": ["D1.1"] }));
    let dry = ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s, "dry_run": true }));
    assert_eq!(dry["added"].as_array().unwrap().len(), 2);
    assert_eq!(dry["removed"][0]["reference"], "R9");
    assert_eq!(dry["board_only"][0]["reference"], "H1");
    assert_eq!(dry["in_sync"], false);
    assert_eq!(de_pcb::items(p.world(), b).components.len(), 2, "dry run changes nothing");
    let r = ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s, "place": { "R1": { "at": [10, 10], "rotation": 90 } } }));
    assert_eq!(r["dry_run"], false);
    let nets = board_nets(&p, b);
    assert_eq!(nets.get("R1.1").map(String::as_str), Some("3V3"));
    assert_eq!(nets.get("D1.1").map(String::as_str), Some("GND"));
    assert_eq!(nets.get("R1.2"), nets.get("D1.2"));
    assert!(nets.get("R1.2").unwrap().starts_with("Net-(D1-Pad2)"));
    let comps = de_pcb::items(p.world(), b).components;
    let r1 = comps.iter().find(|c| c.1.reference == "R1").unwrap();
    assert_eq!(r1.1.at, [10.0, 10.0]);
    assert!(comps.iter().any(|c| c.1.reference == "H1") && !comps.iter().any(|c| c.1.reference == "R9"));
    assert_eq!(schematic::items(p.world(), s).symbols.iter().filter(|x| x.1.component.is_some()).count(), 2, "symbols linked");
    // In sync now.
    assert_eq!(de_pcb::schematic_sync::pending(p.world(), s).unwrap().in_sync, true);
    // Change a value and a net: both reported.
    let r1s = schematic::items(p.world(), s).symbols.iter().find(|x| x.1.reference == "R1").unwrap().0;
    ok(&mut p, "pcb.schematic.symbol.update", json!({ "target": r1s, "value": "4k7" }));
    let lbl = view(&p, s).labels.iter().find(|l| l.text == "3V3").unwrap().id;
    ok(&mut p, "pcb.schematic.item.update", json!({ "target": lbl, "text": "5V" }));
    let r = ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s }));
    assert_eq!(r["changed"][0]["reference"], "R1");
    let ch: Vec<String> = serde_json::from_value(r["changed"][0]["changes"].clone()).unwrap();
    assert!(ch.iter().any(|c| c == "value 10k → 4k7") && ch.iter().any(|c| c.contains("pad net")), "{ch:?}");
    assert_eq!(r["nets"][0], json!({ "pad": "R1.1", "from": "3V3", "to": "5V" }));
    assert_eq!(r["new_nets"], json!(["5V"]));
    assert_eq!(r["gone_nets"], json!(["3V3"]));
    // Back-annotation: a reference changed on the board comes back.
    ok(&mut p, "pcb.component.update", json!({ "target": r1.0, "reference": "R5" }));
    let back = ok(&mut p, "pcb.schematic.back_annotate", json!({ "schematic": s }));
    assert_eq!(back["renamed"][0], json!({ "from": "R1", "to": "R5" }));
    assert!(view(&p, s).symbols.iter().any(|x| x.reference == "R5"));
    assert_eq!(de_pcb::schematic_sync::pending(p.world(), s).unwrap().in_sync, true);
    // Boards without a schematic keep the netlist command.
    let b2 = id(&ok(&mut p, "pcb.board.create", json!({ "name": "B2", "width": 20, "height": 20 })));
    ok(&mut p, "pcb.component.create", json!({ "board": b2, "reference": "R1", "footprint": "R_0603", "at": [5, 5] }));
    ok(&mut p, "pcb.netlist.set", json!({ "board": b2, "nets": { "A": ["R1.1"], "B": ["R1.2"] } }));
    assert_eq!(board_nets(&p, b2).len(), 2);
}

fn substitute(v: &Value, outputs: &[Value]) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') => {
            let (n, path) = s[1..].split_once('.').unwrap();
            outputs[n.parse::<usize>().unwrap()][path].clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, outputs)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), substitute(x, outputs))).collect()),
        other => other.clone(),
    }
}

fn build(file: &str, skip_route: bool) -> (Project, Vec<Value>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join(file)).unwrap()).unwrap();
    let mut p = project();
    let mut outs = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        if skip_route && s["operation"] == "pcb.autoroute" {
            outs.push(Value::Null);
            continue;
        }
        let params = substitute(&s["parameters"], &outs);
        let out = run(&mut p, s["operation"].as_str().unwrap(), params).unwrap_or_else(|e| panic!("{file} step {i} {}: {e}", s["operation"]));
        outs.push(out);
    }
    (p, outs)
}

#[test]
fn the_esp32_example_gets_a_clean_schematic_and_rebuilds_from_one() {
    // The board as it is today, without routing (nets only).
    let (mut p, outs) = build("steps.json", true);
    let board = id(&outs[0]);
    let before = board_nets(&p, board);
    // Draw its schematic: the board is then already in step with it, and it passes ERC.
    let out = ok(&mut p, "pcb.schematic.from_board", json!({ "board": board }));
    let s = id(&out);
    let v = view(&p, s);
    for x in &v.violations {
        eprintln!("{} {}: {}", x.severity, x.code, x.message);
    }
    assert_eq!(v.errors(), 0, "ERC passes");
    assert_eq!(v.stats.parts, 21, "21 parts drawn (mounting holes stay on the board only)");
    let pending = de_pcb::schematic_sync::pending(p.world(), s).unwrap();
    assert!(pending.in_sync, "{pending:#?}");
    assert_eq!(pending.board_only.len(), 4);
    // Every symbol sits on the paper, none overlapping another.
    let sheet = &v.sheets[0];
    for a in &v.symbols {
        assert!(a.bbox[0] > 10.0 && a.bbox[1] > 10.0 && a.bbox[2] < sheet.size[0] - 10.0 && a.bbox[3] < sheet.size[1] - 10.0, "{} off the paper: {:?} on {}", a.reference, a.bbox, sheet.paper);
        for b in &v.symbols {
            if a.id != b.id {
                let o = a.body[0] < b.body[2] && b.body[0] < a.body[2] && a.body[1] < b.body[3] && b.body[1] < a.body[3];
                assert!(!o, "{} overlaps {}", a.reference, b.reference);
            }
        }
    }
    // The PDF has one page per sheet.
    let pdf = de_pcb::schematic_pdf::pdf(&v);
    assert!(pdf.starts_with(b"%PDF-1.4") && String::from_utf8_lossy(&pdf).contains("/Count 1"));
    // Rebuilt schematic-first: the same nets, and it still routes and passes DRC.
    let (q, outs2) = build("schematic-steps.json", false);
    let board2 = id(&outs2[0]);
    let after = board_nets(&q, board2);
    assert_eq!(before, after, "identical pad nets");
    let s2 = id(&outs2[1]);
    assert_eq!(view(&q, s2).errors(), 0);
    let bv = de_pcb::view(q.world(), board2).unwrap();
    for x in bv.violations.iter().filter(|x| x.severity == "error") {
        eprintln!("DRC {}: {}", x.code, x.message);
    }
    assert_eq!(bv.errors(), 0, "DRC passes");
    assert!(bv.nets.iter().all(|n| n.routed));
}
