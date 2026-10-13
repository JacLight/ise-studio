//! Board objects, zones, placement, stackup and the custom rules language:
//! graphics, keepouts, teardrops, via types and their checks, stitching,
//! align/array/flip, multichannel, and what the exports carry.

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

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])))
}

fn codes(p: &Project, board: StableId) -> Vec<(String, String)> {
    de_pcb::view(p.world(), board).unwrap().violations.iter().map(|v| (v.code.to_string(), v.message.clone())).collect()
}

fn has(p: &Project, board: StableId, code: &str, text: &str) -> bool {
    codes(p, board).iter().any(|(c, m)| c == code && m.contains(text))
}

fn board(p: &mut Project, layers: u8) -> StableId {
    id(&ok(p, "pcb.board.create", json!({ "name": "Objects", "width": 40, "height": 30, "layers": layers, "model": false })))
}

#[test]
fn blind_buried_and_micro_vias_are_checked_against_the_stackup() {
    let mut p = project();
    let b = board(&mut p, 6);
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.layers, vec!["top", "in1", "in2", "in3", "in4", "bottom"]);
    assert_eq!(v.stackup.dielectrics.len(), 5);
    // Good ones.
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "A", "at": [5, 5], "kind": "blind", "from": "top", "to": "in2" }));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "B", "at": [10, 5], "kind": "buried", "from": "in1", "to": "in4" }));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "C", "at": [15, 5], "kind": "micro", "from": "top", "to": "in1" }));
    let clean = codes(&p, b);
    assert!(!clean.iter().any(|(c, _)| c.starts_with("via_span") || c.starts_with("microvia")), "{clean:?}");
    let v = de_pcb::view(p.world(), b).unwrap();
    let blind = v.vias.iter().find(|x| x.kind == "blind").unwrap();
    assert_eq!((blind.from.as_str(), blind.to.as_str()), ("top", "in2"));
    // Blind holes get their own drill file; a through hole file does not hold them.
    let files = de_pcb::gerber::files(&v);
    let pair = files.iter().find(|(n, _)| n.ends_with("-1-3.drl")).expect("a top–in2 drill file");
    assert!(pair.1.contains("TF.FileFunction,Plated,1,3,Blind"), "{}", pair.1);
    assert!(files.iter().any(|(n, t)| n.ends_with("-2-5.drl") && t.contains("Buried")));
    // The blind via has no pad on in3 or bottom.
    let bottom = files.iter().find(|(n, _)| n.ends_with("B_Cu.gbl")).unwrap();
    assert!(!bottom.1.contains("X5000000Y5000000D03"), "no pad on bottom for a top–in2 blind via");
    // Bad ones.
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "D", "at": [20, 5], "kind": "blind", "from": "in1", "to": "in2" }));
    assert!(has(&p, b, "via_span", "reaches no outer layer"));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "E", "at": [25, 5], "kind": "buried", "from": "top", "to": "in3" }));
    assert!(has(&p, b, "via_span", "reaches an outer layer"));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "F", "at": [30, 5], "kind": "micro", "from": "in1", "to": "in2" }));
    assert!(has(&p, b, "microvia_crosses_core", "crosses a core"));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "G", "at": [35, 5], "kind": "micro", "from": "top", "to": "in2" }));
    assert!(has(&p, b, "via_span", "crosses 2 dielectrics"));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "H", "at": [35, 15], "kind": "micro", "drill": 0.05, "diameter": 0.2 }));
    assert!(has(&p, b, "microvia_drill", "below the micro via minimum"));
    // Blind vias need 4 layers; a missing layer is refused.
    let two = board(&mut p, 2);
    assert!(run(&mut p, "pcb.via.create", json!({ "board": two, "at": [5, 5], "kind": "blind", "from": "top", "to": "bottom" })).is_err());
    assert!(run(&mut p, "pcb.via.create", json!({ "board": b, "at": [5, 5], "kind": "blind", "from": "top", "to": "in9" })).is_err());
    // A via padstack: no pad on in1.
    let s = id(&ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "S", "at": [5, 25], "stack": { "in1": 0, "top": 0.8 } })));
    let v = de_pcb::view(p.world(), b).unwrap();
    let pieces: Vec<_> = v.cu.iter().filter(|c| matches!(c.owner, de_pcb::board::Owner::Via(x) if x == s)).collect();
    assert!(pieces.iter().all(|c| c.layers & 0b10 == 0), "no copper on in1");
    assert!(pieces.iter().any(|c| c.layers & 1 != 0 && (c.prim.r - 0.4).abs() < 1e-9), "0.8 mm pad on top");
}

#[test]
fn layer_counts_up_to_32_and_the_stackup() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "Many", "width": 50, "height": 50, "layers": 32, "thickness": 3.2, "model": false })));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.layers.len(), 32);
    assert_eq!(v.layers[30], "in30");
    assert!((v.stackup.total - 3.2).abs() < 0.02, "{}", v.stackup.total);
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "in17", "net": "X", "points": [[5, 5], [20, 5]] }));
    assert!(run(&mut p, "pcb.board.create", json!({ "name": "Odd", "width": 10, "height": 10, "layers": 5 })).is_err());
    // A custom stackup, then fewer layers drops it.
    let four = board(&mut p, 4);
    let r = ok(&mut p, "pcb.stackup.set", json!({ "board": four, "dielectrics": [
        { "kind": "prepreg", "thickness": 0.2, "material": "FR408HR", "epsilon_r": 3.7 },
        { "kind": "core", "thickness": 1.0, "material": "FR408HR" },
        { "kind": "prepreg", "thickness": 0.2, "material": "FR408HR" } ], "copper_oz": [1, 0.5, 0.5, 1], "fit_thickness": true }));
    assert_eq!(r["custom"], true);
    let v = de_pcb::view(p.world(), four).unwrap();
    assert_eq!(v.stackup.dielectrics[0].material, "FR408HR");
    assert!((v.thickness - 1.53).abs() < 0.011, "fitted: {}", v.thickness);
    assert!(run(&mut p, "pcb.stackup.set", json!({ "board": four, "dielectrics": [{ "thickness": 1.0 }] })).is_err(), "3 dielectrics for 4 layers");
    // Copper on in17 keeps a 32-layer board from shrinking.
    assert!(run(&mut p, "pcb.board.update", json!({ "target": b, "layers": 8 })).is_err());
    let k = de_pcb::kicad::export_world(p.world(), four).unwrap();
    assert!(k.contains("(stackup") && k.contains("FR408HR") && k.contains("\"In2.Cu\""));
}

#[test]
fn graphics_on_any_layer_join_copper_and_the_exports() {
    let mut p = project();
    let b = board(&mut p, 2);
    ok(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "silk_top", "kind": "text", "text": "FORMA REV A", "at": [20, 25], "size": 1.5 }));
    ok(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "F.SilkS", "kind": "text", "text": "Inter", "at": [20, 20], "size": 2, "font": "Inter", "knockout": true }));
    ok(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "silk_bottom", "kind": "rect", "from": [2, 2], "to": [8, 6], "corner": 0.5 }));
    ok(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "fab_top", "kind": "dimension", "style": "aligned", "points": [[0, 0], [40, 0]], "offset": -3 }));
    ok(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "user", "kind": "arc", "through": [[10, 10], [12, 12], [14, 10]] }));
    ok(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "comments", "kind": "dimension", "style": "leader", "points": [[30, 10], [33, 14]], "text": "NO PARTS HERE" }));
    // A copper heat spreader joined to GND, and a pour round it.
    let pad = id(&ok(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "top", "kind": "polygon", "points": [[10, 10], [16, 10], [16, 14], [10, 14]], "filled": true, "net": "GND" })));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "SIG", "points": [[13, 2], [13, 12]] }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.graphics.len(), 7);
    let dim = v.graphics.iter().find(|g| g.kind == "dimension" && g.value.is_some_and(|x| x > 39.0)).unwrap();
    assert_eq!(dim.text.as_deref(), Some("40.00 mm"));
    assert!(has(&p, b, "short", "copper graphic on GND"), "{:?}", codes(&p, b));
    ok(&mut p, "pcb.delete", json!({ "targets": [pad] }));
    // Exports carry them.
    let v = de_pcb::view(p.world(), b).unwrap();
    let files = de_pcb::gerber::files(&v);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.iter().any(|n| n.ends_with("F_Fab.gbr")) && names.iter().any(|n| n.ends_with("Dwgs_User.gbr")) && names.iter().any(|n| n.ends_with("Cmts_User.gbr")), "{names:?}");
    let silk = &files.iter().find(|(n, _)| n.ends_with(".gto")).unwrap().1;
    assert!(silk.contains("G36*"), "the knocked-out text is a region");
    let bsilk = &files.iter().find(|(n, _)| n.ends_with(".gbo")).unwrap().1;
    assert!(bsilk.lines().filter(|l| l.ends_with("D01*")).count() >= 4);
    for (n, t) in &files {
        if !n.ends_with(".drl") {
            de_pcb::reader::gerber(t).unwrap_or_else(|e| panic!("{n} does not read back: {e}"));
        }
    }
    let ipc = de_pcb::io::ipc2581::write(&v, &de_pcb::boards(p.world())[0].1);
    assert!(ipc.matches("Polyline").count() > 10);
    let k = de_pcb::kicad::export_world(p.world(), b).unwrap();
    assert!(k.contains("gr_poly") && k.contains("\"F.Fab\"") && k.contains("\"Cmts.User\""));
    // Global edit: every silk line 0.2 wide.
    let r = ok(&mut p, "pcb.graphics.edit", json!({ "board": b, "layer": "silk_bottom", "width": 0.2 }));
    assert_eq!(r["changed"], 1);
    // Bad layer.
    assert!(run(&mut p, "pcb.graphic.create", json!({ "board": b, "layer": "in1", "kind": "line", "points": [[0, 0], [1, 1]] })).is_err());
}

#[test]
fn keepouts_rule_areas_and_zone_features() {
    let mut p = project();
    let b = board(&mut p, 2);
    let area = id(&ok(&mut p, "pcb.area.create", json!({ "board": b, "name": "Antenna", "outline": [[25, 0], [40, 0], [40, 30], [25, 30]] })));
    let z = id(&ok(&mut p, "pcb.zone.create", json!({ "board": b, "net": "GND", "layer": "bottom", "name": "Ground" })));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "RF", "points": [[10, 15], [30, 15]] }));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "GND", "at": [30, 25] }));
    assert!(has(&p, b, "keepout", "track on RF is inside keepout “Antenna”"));
    assert!(has(&p, b, "keepout", "via on GND"));
    let v = de_pcb::view(p.world(), b).unwrap();
    let pour = v.zones.iter().find(|x| x.id == z).unwrap();
    assert!(pour.area < 40.0 * 30.0 * 0.7, "no pour in the keepout: {}", pour.area);
    // Hatched pour with a cutout.
    ok(&mut p, "pcb.area.update", json!({ "target": area, "no_pours": false }));
    let solid = de_pcb::view(p.world(), b).unwrap().zones[0].area;
    ok(&mut p, "pcb.zone.update", json!({ "target": z, "hatch": { "width": 0.4, "gap": 1.0 }, "cutouts": [[[5, 5], [10, 5], [10, 10], [5, 10]]] }));
    let hatched = de_pcb::view(p.world(), b).unwrap().zones[0].area;
    assert!(hatched < solid * 0.7, "hatched {hatched} vs solid {solid}");
    // Priorities and duplication.
    let top = id(&ok(&mut p, "pcb.zone.duplicate", json!({ "target": z, "layer": "top" })));
    let r = ok(&mut p, "pcb.zone.priority", json!({ "target": top, "to": "raise" }));
    assert_eq!(r["priority"], 0, "nothing else on top to rise above");
    let other = id(&ok(&mut p, "pcb.zone.create", json!({ "board": b, "net": "VCC", "layer": "top", "priority": 3 })));
    let r = ok(&mut p, "pcb.zone.priority", json!({ "target": top, "to": "top" }));
    assert_eq!(r["priority"], 4);
    let _ = other;
    // KiCad round trip keeps the keepout.
    let k = de_pcb::kicad::export_world(p.world(), b).unwrap();
    assert!(k.contains("(keepout") && k.contains("(mode hatch)"));
    let kb = de_pcb::kicad::parse_board(&k).unwrap();
    assert_eq!(kb.areas.len(), 1);
    assert_eq!(kb.areas[0].name, "Antenna");
}

#[test]
fn custom_rules_raise_their_own_violations() {
    let mut p = project();
    let b = board(&mut p, 2);
    ok(&mut p, "pcb.board.update", json!({ "target": b, "net_classes": [{ "name": "HV", "nets": ["MAINS"], "track_width": 0.5, "clearance": 0.2 }] }));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "MAINS", "width": 0.5, "points": [[5, 10], [35, 10]] }));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "SIG", "width": 0.2, "points": [[5, 11], [35, 11]] }));
    ok(&mut p, "pcb.area.create", json!({ "board": b, "name": "NoVia", "outline": [[0, 20], [40, 20], [40, 30], [0, 30]], "no_tracks": false, "no_vias": false, "no_pours": false }));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "SIG", "at": [20, 25] }));
    assert!(!has(&p, b, "clearance", "MAINS"), "0.65 mm passes the class clearance");
    let rules = r#"(version 1)
        (rule "HV" (condition "A.NetClass == 'HV' && B.NetClass != 'HV'") (constraint clearance (min 1.0mm)))
        (rule "no vias" (condition "A.insideArea('NoVia')") (constraint disallow via))
        (rule "thin" (condition "A.NetName == 'SIG'") (constraint track_width (min 0.25mm)) (severity warning))"#;
    let r = ok(&mut p, "pcb.rules.set", json!({ "board": b, "text": rules }));
    assert_eq!(r["rules"].as_array().unwrap().len(), 3);
    assert!(has(&p, b, "clearance", "rule “HV”"), "{:?}", codes(&p, b));
    assert!(has(&p, b, "disallow", "vias are not allowed here (rule “no vias”)"));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.violations.iter().any(|x| x.code == "track_width" && x.severity == "warning" && x.message.contains("thin")));
    assert!(run(&mut p, "pcb.rules.set", json!({ "board": b, "text": "(rule x (constraint nonsense (min 1mm)))" })).is_err());
    let files = de_pcb::io::board_exports(&v, false);
    let dru = files.iter().find(|(n, _)| n.ends_with(".kicad_dru")).expect("a rules file").0.clone();
    let out = de_pcb::io::board_file(p.world(), b, &dru).unwrap().unwrap();
    let text = String::from_utf8(out.bytes).unwrap();
    let back = de_pcb::rules::parse(&text).unwrap();
    assert!(back.rules.len() >= 10 && back.rules.iter().any(|r| r.name == "HV"), "{text}");
}

#[test]
fn teardrops_follow_the_tracks() {
    let mut p = project();
    let b = board(&mut p, 2);
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "A", "at": [10, 10], "drill": 0.4, "diameter": 0.9 }));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "A", "width": 0.2, "points": [[10, 10], [20, 10]] }));
    ok(&mut p, "pcb.teardrops.set", json!({ "board": b }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.teardrops.len(), 1);
    assert_eq!(v.teardrops[0].on, "via");
    let tip = v.teardrops[0].polygon.iter().map(|q| q[0]).fold(f64::MIN, f64::max);
    assert!((tip - (10.0 + 0.45 + 0.45)).abs() < 0.01, "tip at {tip}");
    let cu = de_pcb::gerber::copper(&v, "top");
    assert!(cu.matches("G36*").count() >= 1, "the teardrop is a region");
    assert!(de_pcb::kicad::export_world(p.world(), b).unwrap().contains("(teardrop"));
    ok(&mut p, "pcb.teardrops.set", json!({ "board": b, "enabled": false }));
    assert!(de_pcb::view(p.world(), b).unwrap().teardrops.is_empty());
}

#[test]
fn stitching_and_fences_place_vias_clear_of_everything() {
    let mut p = project();
    let b = board(&mut p, 2);
    let z = id(&ok(&mut p, "pcb.zone.create", json!({ "board": b, "net": "GND", "layer": "top" })));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "bottom", "net": "RF", "width": 0.5, "points": [[5, 15], [35, 15]] }));
    let r = ok(&mut p, "pcb.via.stitch", json!({ "board": b, "zone": z, "pitch": 3 }));
    let n = r["vias"].as_u64().unwrap();
    assert!(n > 40, "{n} stitching vias");
    // Again: replaced, not doubled.
    let r2 = ok(&mut p, "pcb.via.stitch", json!({ "board": b, "zone": z, "pitch": 3 }));
    assert_eq!(r2["replaced"].as_u64(), Some(n));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.vias.len() as u64, r2["vias"].as_u64().unwrap());
    assert!(!v.violations.iter().any(|x| x.code == "clearance" || x.code == "short" || x.code == "hole_to_hole"), "{:?}", codes(&p, b));
    let track = v.tracks[0].id;
    let f = ok(&mut p, "pcb.via.fence", json!({ "board": b, "net": "GND", "track": track, "pitch": 2 }));
    assert!(f["vias"].as_u64().unwrap() > 20);
}

#[test]
fn placement_tools_and_multichannel() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "Place", "width": 80, "height": 60, "model": false })));
    let mut ids = vec![];
    for (i, (r, x, y)) in [("R1", 10.0, 10.0), ("R2", 17.0, 13.0), ("R3", 30.0, 8.0)].iter().enumerate() {
        ids.push(id(&ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": r, "footprint": "R_0603", "at": [x, y], "pins": { "1": format!("IN{i}"), "2": "GND" } }))));
    }
    ok(&mut p, "pcb.align", json!({ "targets": ids, "to": "center_y", "reference": ids[0] }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.components.iter().all(|c| (c.at[1] - 10.0).abs() < 1e-6));
    ok(&mut p, "pcb.distribute", json!({ "targets": ids, "axis": "x" }));
    let v = de_pcb::view(p.world(), b).unwrap();
    let r2 = v.components.iter().find(|c| c.reference == "R2").unwrap();
    assert!((r2.at[0] - 20.0).abs() < 1e-6, "{}", r2.at[0]);
    ok(&mut p, "pcb.move", json!({ "targets": [ids[2]], "relative_to": ids[0], "offset": [0, 10] }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.components.iter().find(|c| c.reference == "R3").unwrap().at, [10.0, 20.0]);
    ok(&mut p, "pcb.flip", json!({ "targets": [ids[2]] }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.components.iter().find(|c| c.reference == "R3").unwrap().side, de_pcb::Side::Bottom);
    ok(&mut p, "pcb.swap", json!({ "a": ids[0], "b": ids[1] }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!((v.components.iter().find(|c| c.reference == "R1").unwrap().at[0] - 20.0).abs() < 1e-6);
    let a = ok(&mut p, "pcb.array", json!({ "targets": [ids[0]], "grid": { "nx": 3, "ny": 2, "dx": 5, "dy": 5 } }));
    assert_eq!(a["made"], 5);
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.components.iter().any(|c| c.reference == "R8"), "{:?}", v.components.iter().map(|c| &c.reference).collect::<Vec<_>>());

    // Multichannel: channel 1 (U1 + R10) routed, repeated onto channel 2 (U2 + R11).
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "Channels", "width": 80, "height": 60, "model": false })));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "U1", "footprint": "SOT-23", "at": [10, 10], "pins": { "1": "A_IN", "2": "GND", "3": "A_OUT" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R10", "footprint": "R_0603", "at": [15, 10], "pins": { "1": "A_OUT", "2": "GND" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "U2", "footprint": "SOT-23", "at": [40, 30], "rotation": 90, "pins": { "1": "B_IN", "2": "GND", "3": "B_OUT" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R11", "footprint": "R_0603", "at": [60, 50], "pins": { "1": "B_OUT", "2": "GND" } }));
    let v = de_pcb::view(p.world(), b).unwrap();
    let u1_3 = v.pads.iter().find(|q| q.reference == "U1" && q.number == "3").unwrap().at;
    let r10_1 = v.pads.iter().find(|q| q.reference == "R10" && q.number == "1").unwrap().at;
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "A_OUT", "points": [u1_3, [r10_1[0], u1_3[1]], r10_1] }));
    let r = ok(&mut p, "pcb.channel.repeat", json!({ "board": b, "source": ["U1", "R10"], "channels": [["U2", "R11"]], "areas": true }));
    assert_eq!(r["channels"][0]["tracks"], 1);
    let v = de_pcb::view(p.world(), b).unwrap();
    let r11 = v.components.iter().find(|c| c.reference == "R11").unwrap();
    assert!((r11.at[0] - 40.0).abs() < 1e-6 && (r11.at[1] - 35.0).abs() < 1e-6, "turned with U2: {:?}", r11.at);
    assert!(v.nets.iter().find(|n| n.name == "B_OUT").unwrap().routed, "the copied track joins U2.3 to R11.1");
    assert_eq!(v.areas.len(), 2);
    assert!(!v.violations.iter().any(|x| x.code == "placement_area"));
}
