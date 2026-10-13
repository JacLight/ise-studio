//! Schematic editor parity: buses (vector, group, alias, through hierarchy),
//! shared sheets with per-instance references, moving/turning/aligning/duplicating
//! selections, find and replace, the field table, ERC settings, net classes,
//! graphics, and the netlist / BOM / SVG / PDF outputs.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::schematic::{self, SchematicView};
use de_pcb::{PcbPlugin, schematic_export};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
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

fn net_of<'a>(v: &'a SchematicView, sheet: &str, pin: &str) -> Option<&'a str> {
    let (r, n) = pin.split_once('.').unwrap();
    v.symbols.iter().filter(|s| s.reference == r && s.sheet == sheet).flat_map(|s| s.pins.iter()).find(|p| p.number == n).and_then(|p| p.net.as_deref())
}

fn codes(v: &SchematicView) -> Vec<&'static str> {
    v.violations.iter().filter(|x| !x.excluded).map(|x| x.code).collect()
}

fn sch(p: &mut Project) -> StableId {
    id(&ok(p, "pcb.schematic.create", json!({ "name": "Editor" })))
}

#[test]
fn buses_carry_members_through_the_hierarchy() {
    let mut p = project();
    let s = sch(&mut p);
    ok(&mut p, "pcb.schematic.update", json!({ "target": s, "bus_aliases": { "I2C": ["SDA", "SCL"] } }));
    // Parent: four resistors whose top pins carry DATA0..DATA3, a bus DATA[0..3] into a sheet pin D[0..3].
    for (i, x) in [50.8, 63.5, 76.2, 88.9].iter().enumerate() {
        ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": format!("R{}", i + 1), "at": [x, 101.6] }));
        ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": format!("DATA{i}"), "pins": [format!("R{}.1", i + 1)] }));
        ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": [format!("R{}.2", i + 1)] }));
    }
    let child = ok(&mut p, "pcb.schematic.sheet", json!({ "schematic": s, "parent": "1", "name": "Bank", "at": [152.4, 38.1], "size": [30.48, 30.48], "pins": [{ "name": "D[0..3]", "at": [152.4, 45.72], "side": "left" }] }));
    let c = child["sheet"].as_str().unwrap().to_string();
    let bus = ok(&mut p, "pcb.schematic.bus", json!({ "schematic": s, "points": [[120.65, 45.72], [152.4, 45.72]], "label": "DATA[0..3]" }));
    // Child: a bus named by the hierarchical label D[0..3], resistors on D0..D3.
    ok(&mut p, "pcb.schematic.bus", json!({ "schematic": s, "sheet": c, "points": [[30.48, 38.1], [30.48, 76.2]] }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "sheet": c, "kind": "hierarchical", "text": "D[0..3]", "at": [30.48, 38.1], "rotation": 180 }));
    for i in 0..4 {
        ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "sheet": c, "symbol": "R", "reference": format!("R{}", i + 5), "at": [63.5 + 12.7 * i as f64, 101.6] }));
        ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": format!("D{i}"), "pins": [format!("R{}.1", i + 5)] }));
        ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": [format!("R{}.2", i + 5)] }));
    }
    let v = view(&p, s);
    for i in 0..4 {
        let parent = net_of(&v, "1", &format!("R{}.1", i + 1)).unwrap();
        assert_eq!(parent, format!("DATA{i}"), "the parent's name wins");
        assert_eq!(net_of(&v, &c, &format!("R{}.1", i + 5)), Some(parent), "member {i} joins through the sheet pin");
    }
    let b = v.buses.iter().find(|b| b.id == id(&bus)).unwrap();
    assert_eq!(b.name.as_deref(), Some("DATA[0..3]"));
    assert_eq!(b.members.iter().map(|m| m.net.clone().unwrap_or_default()).collect::<Vec<_>>(), vec!["DATA0", "DATA1", "DATA2", "DATA3"]);
    let bad: Vec<_> = v.violations.iter().filter(|x| x.code.starts_with("bus") || x.code == "label_unmatched" || x.code.starts_with("hier")).collect();
    assert!(bad.is_empty(), "{bad:#?}");
    // Unfolding breaks members out with entries, wires and labels; an unlabelled entry is flagged.
    let u = ok(&mut p, "pcb.schematic.bus.unfold", json!({ "bus": id(&bus), "members": ["DATA0", "DATA1"], "side": "down" }));
    assert_eq!(u["entries"].as_array().unwrap().len(), 2);
    let v = view(&p, s);
    assert_eq!(v.bus_entries.len(), 2);
    assert!(!codes(&v).contains(&"bus_entry_unlabeled"), "{:#?}", v.violations);
    ok(&mut p, "pcb.schematic.bus_entry", json!({ "schematic": s, "at": [140.97, 45.72], "size": [2.54, -2.54] }));
    ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "points": [[143.51, 43.18], [143.51, 35.56], [146.05, 35.56]] }));
    assert!(codes(&view(&p, s)).contains(&"bus_entry_unlabeled"));
    // A wire straight onto a bus is an error.
    ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "points": [[127.0, 30.48], [127.0, 45.72]] }));
    assert!(codes(&view(&p, s)).contains(&"bus_wire_conflict"));
    // Groups and aliases.
    let g = ok(&mut p, "pcb.schematic.bus", json!({ "schematic": s, "points": [[25.4, 25.4], [76.2, 25.4]], "label": "PORT{I2C}" }));
    let v = view(&p, s);
    let gb = v.buses.iter().find(|b| b.id == id(&g)).unwrap();
    assert_eq!(gb.members.iter().map(|m| m.member.as_str()).collect::<Vec<_>>(), vec!["PORT.SDA", "PORT.SCL"]);
    assert!(run(&mut p, "pcb.schematic.bus", json!({ "schematic": s, "points": [[0, 0], [10, 0]], "label": "SDA" })).is_err(), "a plain name is not a bus name");
}

#[test]
fn a_sheet_used_twice_gets_its_own_references_and_nets() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "B", "width": 40, "height": 30 })));
    let s = id(&ok(&mut p, "pcb.schematic.create", json!({ "board": b })));
    let c = ok(&mut p, "pcb.schematic.sheet", json!({ "schematic": s, "parent": "1", "name": "Channel", "at": [101.6, 25.4], "size": [25.4, 15.24] }))["sheet"].as_str().unwrap().to_string();
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "sheet": c, "symbol": "R", "reference": "R1", "value": "10k", "at": [50.8, 50.8] }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "hierarchical", "text": "IN", "pin": "R1.1" }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": ["R1.2"] }));
    ok(&mut p, "pcb.schematic.sheet", json!({ "schematic": s, "id": c, "sync_pins": true }));
    let c2 = ok(&mut p, "pcb.schematic.sheet", json!({ "schematic": s, "parent": "1", "name": "Channel B", "instance_of": c, "at": [101.6, 63.5], "size": [25.4, 15.24], "sync_pins": true }))["sheet"].as_str().unwrap().to_string();
    let sch: schematic::Schematic = p.get::<schematic::Schematic>(s).unwrap().clone();
    let pin_a = sch.sheet(&c).unwrap().pins[0].at;
    let pin_b = sch.sheet(&c2).unwrap().pins[0].at;
    assert_eq!(sch.sheet(&c2).unwrap().pins[0].name, "IN");
    // Two drivers on the parent, one into each instance.
    for (r, x, pin) in [("R10", 50.8, pin_a), ("R11", 76.2, pin_b)] {
        ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": r, "at": [x, 101.6] }));
        ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "from": format!("{r}.1"), "to": pin }));
        ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": [format!("{r}.2")] }));
    }
    let v = view(&p, s);
    assert!(v.symbols.iter().any(|x| x.sheet == c2 && x.reference == "R?" && x.instance.as_deref() == Some(c.as_str())), "the copy waits to be numbered");
    assert!(codes(&v).contains(&"not_annotated"));
    let a = ok(&mut p, "pcb.schematic.annotate", json!({ "schematic": s }));
    assert_eq!(a["renamed"].as_array().unwrap().len(), 1, "{a}");
    let v = view(&p, s);
    let copy = v.symbols.iter().find(|x| x.sheet == c2).unwrap();
    assert_eq!(copy.reference, "R2");
    let n1 = net_of(&v, &c, "R1.1").unwrap();
    let n2 = net_of(&v, &c2, "R2.1").unwrap();
    assert_ne!(n1, n2, "each instance has its own net");
    assert_eq!(net_of(&v, "1", "R10.1"), Some(n1));
    assert_eq!(net_of(&v, "1", "R11.1"), Some(n2));
    // The board gets both resistors; renaming the copy works per instance.
    let dry = ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s, "dry_run": true }));
    let added: Vec<&str> = dry["added"].as_array().unwrap().iter().map(|x| x["reference"].as_str().unwrap()).collect();
    assert!(added.contains(&"R1") && added.contains(&"R2"), "{added:?}");
    ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s }));
    let again = ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s, "dry_run": true }));
    assert_eq!(again["in_sync"], true, "{again}");
    ok(&mut p, "pcb.schematic.symbol.update", json!({ "target": copy.id, "instance": c2, "reference": "R21" }));
    let v = view(&p, s);
    assert!(v.symbols.iter().any(|x| x.sheet == c2 && x.reference == "R21"));
    assert!(v.symbols.iter().any(|x| x.sheet == c && x.reference == "R1"), "the first instance keeps R1");
    // Increment annotation by 100 on the instance sheet only.
    ok(&mut p, "pcb.schematic.annotate", json!({ "schematic": s, "sheet": c2, "increment": 100 }));
    assert!(view(&p, s).symbols.iter().any(|x| x.sheet == c2 && x.reference == "R121"));
    // Per-sheet numbering from the page number.
    ok(&mut p, "pcb.schematic.annotate", json!({ "schematic": s, "reset": true, "numbering": "sheet" }));
    let v = view(&p, s);
    let page_c = v.sheets.iter().find(|x| x.id == c).unwrap().page.clone();
    assert!(v.symbols.iter().any(|x| x.sheet == c && x.reference == format!("R{}01", page_c)), "{:?}", v.symbols.iter().map(|x| (&x.sheet, &x.reference)).collect::<Vec<_>>());
}

#[test]
fn selections_move_turn_align_duplicate_and_replace() {
    let mut p = project();
    let s = sch(&mut p);
    let r1 = id(&ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R1", "value": "10k", "at": [50.8, 50.8] })));
    let r2 = id(&ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R2", "value": "10k", "at": [76.2, 55.88] })));
    let w = id(&ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "from": "R1.1", "to": "R2.1" })));
    let l = id(&ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "text": "D0", "pin": "R1.2" })));
    // Moving R2 alone drags the wire end; the net holds.
    ok(&mut p, "pcb.schematic.move", json!({ "targets": [r2], "delta": [12.7, 0] }));
    let v = view(&p, s);
    assert_eq!(net_of(&v, "1", "R1.1"), net_of(&v, "1", "R2.1"));
    assert!(net_of(&v, "1", "R1.1").is_some());
    // Turning R1 a quarter: the wire and label follow its pins.
    ok(&mut p, "pcb.schematic.move", json!({ "targets": [r1], "rotate": 90 }));
    let v = view(&p, s);
    assert_eq!(v.symbols.iter().find(|x| x.id == r1).unwrap().rotation, 90.0);
    assert_eq!(net_of(&v, "1", "R1.1"), net_of(&v, "1", "R2.1"), "{:#?}", v.wires);
    assert_eq!(net_of(&v, "1", "R1.2"), Some("D0"));
    // Mirror up-down = mirror + half turn.
    ok(&mut p, "pcb.schematic.move", json!({ "targets": [r2], "mirror": "y" }));
    let sym = view(&p, s).symbols.iter().find(|x| x.id == r2).unwrap().clone();
    assert!(sym.mirror && sym.rotation == 180.0, "{} {}", sym.mirror, sym.rotation);
    // Align tops.
    ok(&mut p, "pcb.schematic.align", json!({ "targets": [r1, r2], "how": "top" }));
    let v = view(&p, s);
    let tops: Vec<f64> = [r1, r2].iter().map(|i| v.symbols.iter().find(|x| x.id == *i).unwrap().body[1]).collect();
    assert!((tops[0] - tops[1]).abs() < 1.3, "{tops:?}");
    // Duplicate with label increment: new reference and D1.
    let d = ok(&mut p, "pcb.schematic.duplicate", json!({ "targets": [r1, l, w], "delta": [0, 25.4], "increment": 1 }));
    assert_eq!(d["ids"].as_array().unwrap().len(), 3);
    let v = view(&p, s);
    assert!(v.symbols.iter().any(|x| x.reference == "R3"), "next free reference");
    assert!(v.labels.iter().any(|x| x.text == "D1"));
    // Find and replace values and labels.
    let dry = ok(&mut p, "pcb.schematic.replace", json!({ "schematic": s, "find": "10k", "replace": "4k7", "in": ["values"], "dry_run": true }));
    assert_eq!(dry["count"], 3);
    assert_eq!(view(&p, s).symbols.iter().filter(|x| x.value == "10k").count(), 3, "dry run changes nothing");
    ok(&mut p, "pcb.schematic.replace", json!({ "schematic": s, "find": "10k", "replace": "4k7", "in": ["values"] }));
    ok(&mut p, "pcb.schematic.replace", json!({ "schematic": s, "find": "D?", "replace": "DATA", "in": ["labels"], "wildcards": true }));
    let v = view(&p, s);
    assert_eq!(v.symbols.iter().filter(|x| x.value == "4k7").count(), 3);
    assert_eq!(v.labels.iter().filter(|x| x.text == "DATA").count(), 2);
}

#[test]
fn fields_erc_settings_netclasses_and_graphics() {
    let mut p = project();
    let s = sch(&mut p);
    ok(&mut p, "pcb.schematic.update", json!({ "target": s, "field_templates": [{ "name": "Tolerance", "value": "1%", "visible": true }], "text_vars": { "PROJECT": "Probe" } }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R1", "at": [50.8, 50.8] }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R2", "at": [76.2, 50.8] }));
    let v = view(&p, s);
    assert_eq!(v.symbols[0].fields.get("Tolerance").map(String::as_str), Some("1%"), "templates fill new symbols");
    assert!(v.symbols[0].draw.iter().any(|d| matches!(d, de_pcb::schematic_view::Draw::Text { s, .. } if s == "1%")), "visible template fields are drawn");
    // The field table: several symbols at once.
    ok(&mut p, "pcb.schematic.fields", json!({ "schematic": s, "set": [{ "reference": "R1", "field": "MPN", "value": "RC0603" }, { "reference": "R2", "field": "Tolerance", "value": "5%" }, { "reference": "R2", "field": "DNP", "value": "yes" }] }));
    ok(&mut p, "pcb.schematic.fields", json!({ "schematic": s, "rename": ["Tolerance", "Tol"] }));
    let v = view(&p, s);
    let r2 = v.symbols.iter().find(|x| x.reference == "R2").unwrap();
    assert!(r2.dnp && r2.fields.get("Tol").map(String::as_str) == Some("5%"));
    assert_eq!(v.symbols.iter().find(|x| x.reference == "R1").unwrap().mpn.as_deref(), Some("RC0603"));
    // ERC: two outputs (custom ICs) conflict; the matrix can relax it, an exclusion hides it.
    for (r, x) in [("U1", 101.6), ("U2", 139.7)] {
        ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "IC", "reference": r, "footprint": "R_0603", "at": [x, 101.6], "pins": [{ "number": "1", "name": "OUT", "type": "open_collector" }, { "number": "2", "name": "GND", "type": "passive" }] }));
    }
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "BUSY", "pins": ["U1.1", "U2.1", "R1.1"] }));
    let v = view(&p, s);
    assert!(!v.violations.iter().any(|x| x.code == "pin_conflict" && x.message.contains("BUSY")), "open collectors may share a net by default");
    ok(&mut p, "pcb.schematic.erc", json!({ "schematic": s, "pin_matrix": { "open_collector/open_collector": "error" } }));
    let v = view(&p, s);
    let conflict = v.violations.iter().find(|x| x.code == "pin_conflict" && x.message.contains("BUSY")).expect("override makes it an error").clone();
    assert_eq!(conflict.severity, "error");
    let before = v.stats.errors;
    ok(&mut p, "pcb.schematic.erc", json!({ "schematic": s, "exclude": [{ "key": conflict.key, "comment": "wired-OR on purpose" }] }));
    let v = view(&p, s);
    assert!(v.violations.iter().any(|x| x.key == conflict.key && x.excluded));
    assert_eq!(v.stats.errors, before - 1);
    ok(&mut p, "pcb.schematic.erc", json!({ "schematic": s, "severities": { "unconnected_pin": "ignore" } }));
    assert!(!codes(&view(&p, s)).contains(&"unconnected_pin"));
    // Net classes: by pattern and by directive label; net colours.
    ok(&mut p, "pcb.schematic.netclass", json!({ "schematic": s, "name": "Power", "color": "#d03030", "wire_width": 0.4, "track_width": 0.5, "patterns": ["VCC*"] }));
    ok(&mut p, "pcb.schematic.netclass", json!({ "schematic": s, "name": "Fast", "color": "#2a5bd7" }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "VCC_IO", "pins": ["R1.2", "R2.2"], "kind": "local" }));
    let wire_on_busy = view(&p, s).wires.iter().find(|w| w.net.as_deref() == Some("BUSY")).unwrap().points[1];
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "directive", "text": "Fast", "at": wire_on_busy }));
    assert!(run(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "directive", "text": "Nope", "at": [0, 0] })).is_err());
    let v = view(&p, s);
    let net = |n: &str| v.nets.iter().find(|x| x.name == n).unwrap().clone();
    assert_eq!(net("VCC_IO").class.as_deref(), Some("Power"));
    assert_eq!(net("BUSY").class.as_deref(), Some("Fast"));
    assert!(v.wires.iter().any(|w| w.net.as_deref() == Some("VCC_IO") && w.color.as_deref() == Some("#d03030") && w.width == Some(0.4)));
    ok(&mut p, "pcb.schematic.net_color", json!({ "schematic": s, "net": "BUSY", "color": "#00aa00" }));
    assert_eq!(view(&p, s).nets.iter().find(|x| x.name == "BUSY").unwrap().color.as_deref(), Some("#00aa00"));
    // Graphics and text.
    ok(&mut p, "pcb.schematic.shape", json!({ "schematic": s, "shape": "rect", "points": [[20, 20], [60, 40]], "dash": "dash", "fill": "#fff3c4" }));
    ok(&mut p, "pcb.schematic.shape", json!({ "schematic": s, "shape": "arc", "points": [[70, 30], [80, 20], [90, 30]] }));
    ok(&mut p, "pcb.schematic.text_box", json!({ "schematic": s, "at": [150, 20], "size": [40, 15], "text": "Board for ${PROJECT}, rev ${REVISION}" }));
    ok(&mut p, "pcb.schematic.table", json!({ "schematic": s, "at": [150, 40], "cells": [["Rail", "Volts"], ["3V3", "3.3"]] }));
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4zwAE/xkgFAAb8gP91pbyKwAAAABJRU5ErkJggg==";
    let img = ok(&mut p, "pcb.schematic.image", json!({ "schematic": s, "at": [200, 20], "width": 20, "data": png }));
    assert_eq!(img["px"], json!([2, 2]));
    assert!(run(&mut p, "pcb.schematic.shape", json!({ "schematic": s, "shape": "arc", "points": [[0, 0], [1, 1]] })).is_err());
    ok(&mut p, "pcb.schematic.text", json!({ "schematic": s, "at": [20, 60], "text": "${PROJECT} notes", "justify": "center", "bold": true }));
    let v = view(&p, s);
    assert_eq!(v.graphics.len(), 5);
    assert!(v.graphics.iter().any(|g| g.kind == "text_box" && g.draw.iter().any(|d| matches!(d, de_pcb::schematic_view::Draw::Rich { s, .. } if s.contains("Probe")))));
    assert!(v.texts.iter().any(|t| t.text == "Probe notes" && t.bold && t.justify == "center"));
    // Graphics move with a selection.
    let shape = v.graphics.iter().find(|g| g.kind == "shape").unwrap().id;
    ok(&mut p, "pcb.schematic.item.update", json!({ "target": shape, "delta": [5, 0], "set": { "color": "#123456" } }));
    let sh: schematic::SchShape = p.get::<schematic::SchShape>(shape).unwrap().clone();
    assert_eq!(sh.points[0], [25.0, 20.0]);
    assert_eq!(sh.color.as_deref(), Some("#123456"));

    // Outputs.
    let v = view(&p, s);
    let kicad = schematic_export::netlist(&v, "kicad").unwrap();
    assert!(kicad.starts_with("(export (version \"E\")") && kicad.contains("(name \"BUSY\")") && kicad.contains("(node (ref \"U1\") (pin \"1\")"), "{kicad}");
    let orcad = schematic_export::netlist(&v, "orcad").unwrap();
    assert!(orcad.contains(" R1 ") && orcad.trim_end().ends_with('*'));
    let cadstar = schematic_export::netlist(&v, "cadstar").unwrap();
    assert!(cadstar.contains(".ADD_TER") && cadstar.contains(".END"));
    let spice = schematic_export::netlist(&v, "spice").unwrap();
    assert!(spice.contains("XU1 BUSY") && spice.trim_end().ends_with(".end"), "{spice}");
    assert!(schematic_export::netlist(&v, "allegro-ish").is_err());
    let bom = schematic_export::bom_csv(&v, &["Value".into(), "Footprint".into()], &["Reference".into(), "Qty".into(), "Value".into(), "Tol".into()], false);
    assert!(bom.lines().next().unwrap() == "Reference,Qty,Value,Tol" && bom.contains("\"U1, U2\",2"), "{bom}");
    assert!(!bom.contains("R2"), "DNP parts are left out: {bom}");
    let svg = schematic_export::svg(&v, "1").unwrap();
    assert!(svg.contains("<svg") && svg.contains("data:image/png;base64") && svg.contains("stroke-dasharray"));
    let pdf = de_pcb::schematic_pdf::pdf(&v);
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.starts_with("%PDF-1.4") && text.contains("/Subtype /Image") && text.contains("Helvetica-Bold"));
}

#[test]
fn drawing_sheet_page_numbers_and_sheet_pins() {
    let mut p = project();
    let s = sch(&mut p);
    ok(&mut p, "pcb.schematic.update", json!({ "target": s, "drawing_sheet": { "items": [{ "kind": "text", "at": [4, 40], "text": "Approved: ${CHECKER}" }], "no_zones": true }, "text_vars": { "CHECKER": "A. Ade" } }));
    let c = ok(&mut p, "pcb.schematic.sheet", json!({ "schematic": s, "parent": "1", "name": "Power", "page": "7" }))["sheet"].as_str().unwrap().to_string();
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "sheet": c, "symbol": "R", "reference": "R1", "at": [50.8, 50.8] }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "hierarchical", "text": "VIN", "pin": "R1.1" }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "kind": "hierarchical", "text": "VOUT", "pin": "R1.2", "shape": "output" }));
    let v = view(&p, s);
    assert!(codes(&v).contains(&"hier_label_unmatched"));
    ok(&mut p, "pcb.schematic.sheet", json!({ "schematic": s, "id": c, "sync_pins": true }));
    let sch: schematic::Schematic = p.get::<schematic::Schematic>(s).unwrap().clone();
    let box_ = sch.sheet(&c).unwrap();
    assert_eq!(box_.pins.len(), 2);
    let vout = box_.pins.iter().find(|x| x.name == "VOUT").unwrap();
    assert!((vout.at[0] - (box_.at.unwrap()[0] + box_.size.unwrap()[0])).abs() < 1e-3, "outputs go on the right edge");
    let v = view(&p, s);
    assert!(!codes(&v).contains(&"hier_label_unmatched"));
    let frame = &v.sheets.iter().find(|x| x.id == c).unwrap().frame;
    let texts: Vec<&str> = frame.iter().filter_map(|d| if let de_pcb::schematic_view::Draw::Text { s, .. } = d { Some(s.as_str()) } else { None }).collect();
    assert!(texts.contains(&"Approved: A. Ade"), "{texts:?}");
    assert!(texts.contains(&"7 / 2"), "page number in the title block: {texts:?}");
}
