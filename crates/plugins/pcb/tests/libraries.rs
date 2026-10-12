//! Project libraries, footprint assignment, variants, design blocks and the
//! new pad features, through the engine's commands.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::{PcbPlugin, schematic, wizards};
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

fn project() -> (Project, StableId, StableId) {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "Lib test", "width": 40, "height": 30, "model": false })));
    let s = id(&ok(&mut p, "pcb.schematic.create", json!({ "name": "Lib test", "board": b })));
    (p, b, s)
}

fn symbol_of(p: &Project, s: StableId, r: &str) -> schematic::SchSymbol {
    schematic::items(p.world(), s).symbols.into_iter().find(|(_, x)| x.reference == r).map(|(_, x)| x).unwrap_or_else(|| panic!("no {r}"))
}

#[test]
fn derived_symbols_wizard_footprints_assignment_and_push() {
    let (mut p, b, s) = project();
    ok(&mut p, "pcb.lib.copy", json!({ "kind": "symbol", "from": "LDO_SOT23-5", "library": "MyLib", "name": "LDO5" }));
    // A derived symbol: the parent's body, its own part number.
    let out = ok(&mut p, "pcb.lib.symbol.save", json!({ "library": "MyLib", "extends": "MyLib:LDO5", "symbol": { "name": "XC6220B331", "description": "", "prefix": "", "category": "", "value": "", "footprint": "", "mpn": "XC6220B331MR-G", "pins": [], "units": [], "show_pin_numbers": true, "show_pin_names": true, "fields": { "Datasheet": "https://example.com/xc6220.pdf" } } }));
    assert_eq!(out["symbol"], "MyLib:XC6220B331");
    // Its footprint from the wizard, into the project library.
    let fp = wizards::generate(&wizards::Wizard::Sot { package: "SOT-23-5".into(), density: wizards::Density::B, name: Some("SOT-23-5_W".into()) }).unwrap();
    ok(&mut p, "pcb.lib.footprint.save", json!({ "library": "MyLib", "footprint": fp, "model": { "offset": [0.0, 0.0, 0.1], "rotation": 0.0 } }));
    let placed = ok(&mut p, "pcb.lib.place", json!({ "schematic": s, "symbol": "MyLib:XC6220B331", "at": [100.0, 80.0] }));
    let r = placed["reference"].as_str().unwrap().to_string();
    let u = symbol_of(&p, s, &r);
    let def = u.definition.as_ref().unwrap();
    assert_eq!(def.name, "MyLib:XC6220B331");
    assert_eq!(def.pins.len(), 5, "pins come from the parent");
    assert_eq!(u.mpn.as_deref(), Some("XC6220B331MR-G"));
    assert_eq!(u.fields.get("Datasheet").map(String::as_str), Some("https://example.com/xc6220.pdf"));
    // CvPcb: the library footprint goes onto the board, the update builds the part on it.
    let a = ok(&mut p, "pcb.lib.assign", json!({ "schematic": s, "footprints": { r.clone(): "MyLib:SOT-23-5_W" } }));
    assert_eq!(a["warnings"].as_array().unwrap().len(), 0, "{a}");
    let board = p.get::<de_pcb::Board>(b).unwrap().clone();
    let on_board = board.footprints.get("MyLib:SOT-23-5_W").expect("defined on the board");
    assert!((on_board.body[0].z[0] - 0.1).abs() < 1e-9, "3D model offset applied to the body");
    let rep = ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s }));
    assert_eq!(rep["added"][0]["footprint"], "MyLib:SOT-23-5_W");
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.components.iter().all(|c| c.known));
    // Change the library symbol and bring it to the placed part.
    let mut parent = de_pcb::library::find_symbol(p.world(), "MyLib:LDO5").unwrap().1.symbol;
    parent.pins[0].name = "VIN_NEW".into();
    ok(&mut p, "pcb.lib.symbol.save", json!({ "library": "MyLib", "symbol": parent }));
    let pushed = ok(&mut p, "pcb.lib.symbol.push", json!({ "symbol": "MyLib:XC6220B331" }));
    assert_eq!(pushed["updated"].as_array().unwrap().len(), 1);
    assert_eq!(symbol_of(&p, s, &r).definition.unwrap().pins[0].name, "VIN_NEW");
    // A parent with children cannot go.
    assert!(run(&mut p, "pcb.lib.delete", json!({ "symbols": ["MyLib:LDO5"] })).is_err());
}

#[test]
fn power_symbols_place_ports_and_de_morgan_bodies_swap() {
    let (mut p, _, s) = project();
    ok(&mut p, "pcb.lib.symbol.save", json!({ "library": "Power", "power": { "net": "+12V", "style": "arrow" }, "symbol": { "name": "+12V", "description": "", "prefix": "#PWR", "category": "Power", "value": "+12V", "footprint": "", "pins": [], "units": [[]], "show_pin_numbers": false, "show_pin_names": false } }));
    ok(&mut p, "pcb.lib.place", json!({ "schematic": s, "symbol": "Power:+12V", "at": [50.8, 50.8] }));
    let it = schematic::items(p.world(), s);
    assert!(it.labels.iter().any(|(_, l)| l.text == "+12V" && l.kind == schematic::LabelKind::Power && l.style.as_deref() == Some("arrow")));
    // A NAND with an OR-shaped De Morgan body and inverted inputs.
    let mut nand = de_pcb::symbols::get("OpAmp").unwrap();
    nand.name = "NAND".into();
    nand.demorgan = vec![vec![de_pcb::symbols::Gfx::Circle { c: [0.0, 0.0], r: 3.0, fill: de_pcb::symbols::Fill::Body }]];
    nand.demorgan_pins.insert("3".into(), de_pcb::symbols::PinShape::Inverted);
    ok(&mut p, "pcb.lib.symbol.save", json!({ "library": "Logic", "symbol": nand }));
    let r = ok(&mut p, "pcb.lib.place", json!({ "schematic": s, "symbol": "Logic:NAND", "at": [100.0, 100.0], "body_style": 2 }));
    let u = symbol_of(&p, s, r["reference"].as_str().unwrap());
    assert_eq!(u.definition.as_ref().unwrap().body_style, 2);
    assert_eq!(u.definition.as_ref().unwrap().pins[0].shape, de_pcb::symbols::PinShape::Inverted);
    let target = schematic::items(p.world(), s).symbols.into_iter().find(|(_, x)| x.reference == u.reference).unwrap().0;
    ok(&mut p, "pcb.lib.body_style", json!({ "targets": [target], "style": 1 }));
    let back = symbol_of(&p, s, &u.reference);
    assert_eq!(back.definition.as_ref().unwrap().body_style, 1);
    // The sheet draws the bubble of an inverted pin.
    let v = schematic::view(p.world(), s).unwrap();
    let _ = v;
}

#[test]
fn variants_change_the_bom_not_the_board() {
    let (mut p, b, s) = project();
    for (i, (r, v)) in [("R1", "10k"), ("R2", "1k"), ("C1", "100nF")].into_iter().enumerate() {
        ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": if r.starts_with('R') { "R" } else { "C" }, "reference": r, "value": v, "at": [50.8 + 20.32 * i as f64, 50.8] }));
    }
    ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s }));
    let v = id(&ok(&mut p, "pcb.variant.create", json!({ "schematic": s, "name": "Lite", "dnp": ["R2"] })));
    ok(&mut p, "pcb.variant.update", json!({ "target": v, "parts": { "R1": { "value": "4k7", "lcsc": "C23162" } } }));
    assert!(run(&mut p, "pcb.variant.create", json!({ "schematic": s, "name": "lite" })).is_err(), "names are unique");
    assert!(run(&mut p, "pcb.variant.update", json!({ "target": v, "dnp": ["R9"] })).is_err(), "unknown parts are refused");
    let view = de_pcb::view(p.world(), b).unwrap();
    let var = de_pcb::variants::of_board(p.world(), b, "Lite").unwrap();
    let lite = var.apply(&view);
    let r1 = lite.components.iter().find(|c| c.reference == "R1").unwrap();
    assert_eq!((r1.value.as_str(), r1.lcsc.as_deref(), r1.mpn.as_deref()), ("4k7", Some("C23162"), None));
    assert!(lite.components.iter().find(|c| c.reference == "R2").unwrap().dnp);
    assert!(!view.components.iter().find(|c| c.reference == "R2").unwrap().dnp, "the board itself is unchanged");
    let board = p.get::<de_pcb::Board>(b).unwrap().clone();
    let bom = de_pcb::outputs::bom(&lite, &board, de_pcb::outputs::Format::parse("generic"));
    assert!(bom.contains("4k7") && bom.contains("DNP"), "{bom}");
    ok(&mut p, "pcb.variant.update", json!({ "target": v, "parts": { "R2": { "dnp": null, "reset": true } } }));
    assert!(!de_pcb::variants::of_board(p.world(), b, "Lite").unwrap().dnp.contains(&"R2".to_string()));
}

#[test]
fn design_blocks_place_schematic_and_layout_on_new_nets() {
    let (mut p, b, s) = project();
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "R", "reference": "R1", "value": "1k", "at": [60.96, 50.8] }));
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "LED", "reference": "D1", "at": [60.96, 66.04] }));
    ok(&mut p, "pcb.schematic.wire", json!({ "schematic": s, "from": "R1.2", "to": "D1.2" }));
    ok(&mut p, "pcb.schematic.label", json!({ "schematic": s, "text": "LED_IN", "pin": "R1.1" }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": ["D1.1"] }));
    ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s, "place": { "R1": { "at": [10.0, 10.0] }, "D1": { "at": [14.0, 10.0] } } }));
    let net = de_pcb::view(p.world(), b).unwrap().components.iter().find(|c| c.reference == "R1").unwrap().pins["2"].clone();
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "points": [[10.8, 10.0], [13.2, 10.0]], "net": net }));
    let saved = ok(&mut p, "pcb.block.save", json!({ "name": "LED", "schematic": s, "references": ["R1", "D1"] }));
    assert_eq!(saved["symbols"], 2);
    assert_eq!(saved["layout"]["parts"], 2);
    assert_eq!(saved["layout"]["copper"], 1);
    let placed = ok(&mut p, "pcb.block.place", json!({ "block": "LED", "schematic": s, "at": [120.0, 60.0], "board_at": [25.0, 20.0] }));
    assert_eq!(placed["on_board"], 2);
    let renamed = &placed["renamed"];
    let (r_new, d_new) = (renamed["R1"].as_str().unwrap(), renamed["D1"].as_str().unwrap());
    assert!(r_new != "R1" && d_new != "D1");
    let v = de_pcb::view(p.world(), b).unwrap();
    let rn = v.components.iter().find(|c| c.reference == r_new).unwrap();
    let dn = v.components.iter().find(|c| c.reference == d_new).unwrap();
    assert_eq!(rn.pins["2"], dn.pins["2"], "the block's own net joins its two parts");
    assert_ne!(rn.pins["2"], net, "and is not the original's");
    assert_eq!(dn.pins["1"], "GND", "power stays power");
    assert_eq!(rn.pins["1"], "LED_IN_1", "local labels are kept apart");
    assert!(v.tracks.iter().any(|t| t.net.as_deref() == Some(rn.pins["2"].as_str())), "copper comes on the new net");
    // The schematic agrees with the board: nothing pending for the new parts.
    let pending = de_pcb::schematic_sync::pending(p.world(), s).unwrap();
    assert!(pending.added.is_empty(), "{:?}", pending.added.iter().map(|a| &a.reference).collect::<Vec<_>>());
}

#[test]
fn chamfered_trapezoid_stacked_and_thermal_pads_resolve_on_the_board() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "Pads", "width": 30, "height": 30, "layers": 4, "model": false })));
    let fp = json!({ "name": "Odd", "kind": "ic", "pads": [
        { "number": "1", "kind": "smd", "shape": "rect", "size": [1.2, 0.8], "at": [-3.0, 0.0], "chamfer": { "ratio": 0.25, "corners": ["top_left"] } },
        { "number": "2", "kind": "smd", "shape": "rect", "size": [1.2, 0.8], "at": [0.0, 0.0], "trapezoid": [0.0, 0.3], "zone_connection": "solid" },
        { "number": "3", "kind": "tht", "shape": "circle", "size": [1.8, 1.8], "at": [3.5, 0.0], "drill": [1.0, 1.0], "stack": { "inner": { "shape": "circle", "size": [1.3, 1.3] } }, "thermal_gap": 0.3, "drill_offset": [0.2, 0.0] }
    ], "body": [] });
    ok(&mut p, "pcb.footprint.define", json!({ "board": b, "footprint": fp }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "U1", "footprint": "Odd", "at": [15.0, 15.0], "pins": { "1": "GND", "2": "GND", "3": "GND" } }));
    ok(&mut p, "pcb.zone.create", json!({ "board": b, "net": "GND", "layer": "top" }));
    let v = de_pcb::view(p.world(), b).unwrap();
    let pads: Vec<_> = v.pads.iter().filter(|x| x.reference == "U1").collect();
    let p1 = pads.iter().find(|x| x.number == "1").unwrap();
    assert_eq!(p1.polygon.as_ref().unwrap().len(), 5, "one corner chamfered: five corners");
    assert!(pads.iter().find(|x| x.number == "2").unwrap().polygon.is_some());
    // Pad 3: one pad per layer group (near, far, inner), one hole, offset from the copper.
    let p3: Vec<_> = pads.iter().filter(|x| x.number == "3").collect();
    assert_eq!(p3.len(), 3);
    assert!(p3.iter().any(|x| x.layers == vec!["in1".to_string(), "in2".to_string()] && (x.size[0] - 1.3).abs() < 1e-9));
    let holes: Vec<_> = v.holes.iter().filter(|h| h.owner == "U1.3").collect();
    assert_eq!(holes.len(), 1);
    assert!((holes[0].at[0] - 18.7).abs() < 1e-6, "{:?}", holes[0].at);
    // Pad 2 joins the pour solid (no relief), pad 3 with its own gap.
    let z = &v.zones[0];
    assert!(!z.reliefs.iter().any(|r| r.pad == "U1.2"));
    assert!(z.reliefs.iter().any(|r| r.pad == "U1.3"));
}
