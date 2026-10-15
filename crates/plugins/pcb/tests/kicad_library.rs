//! The KiCad library, read from a small committed copy of a few of its files
//! (`tests/fixtures/kicad-lib`): footprints checked pad by pad against the
//! built-in ones, symbols read with their pins and drawing, the search index,
//! and a KiCad part placed on a schematic and synced onto a board.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::footprints::{self, PadKind};
use de_pcb::kicad_lib::{Library, Repo};
use de_pcb::symbols::{Dir, PinType};
use de_pcb::{PcbPlugin, verify};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::Arc;

fn lib() -> Library {
    Library::at(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kicad-lib"))
}

fn run(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, CommandError> {
    let envelope = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: p.id().into(), expected_revision: p.revision(), operation: operation.into(), parameters, actor: Actor { user_id: "u".into(), session_id: "s".into() } };
    p.commit(envelope).map(|c| c.output)
}

fn ok(p: &mut Project, operation: &str, parameters: Value) -> Value {
    run(p, operation, parameters.clone()).unwrap_or_else(|e| panic!("{operation}: {e}"))
}

#[test]
fn built_in_footprints_match_kicad_pad_for_pad() {
    let names: Vec<String> = ["ESP32-WROOM-32", "R_0603", "SOT-23-5", "SOT-223", "USB-C_16P", "SW_Tactile_6x6", "PinHeader_1x06_2.54", "MountingHole_M2", "QFN-32_5x5"].map(String::from).to_vec();
    let r = verify::verify(&lib(), &names);
    for f in &r.footprints {
        for x in &f.findings {
            eprintln!("{} {} {} {}", f.footprint, x.severity, x.pad, x.message);
        }
    }
    assert!(r.missing.is_empty(), "{:?}", r.missing);
    assert_eq!(r.checked, names.len());
    assert_eq!(r.errors, 0, "no footprint is off from KiCad's by more than the tolerances");
    let by = |n: &str| r.footprints.iter().find(|f| f.footprint == n).unwrap();
    // The module's pads reach 0.5 mm past its edges as Espressif draws them (they once sat 0.75 mm inside).
    let esp = by("ESP32-WROOM-32");
    assert!(esp.worst_position < 0.005, "{}", esp.worst_position);
    assert!(esp.findings.iter().any(|x| x.pad == "39" && x.severity == "note"), "the thermal pad is compared as one land");
    // KiCad numbers SOT-223's tab "2"; Iṣẹ́ Studio "4": same copper, flagged.
    assert!(by("SOT-223").findings.iter().any(|x| x.pad == "4" && x.message.contains("numbered")));
    // Headers line up exactly, origin at pin 1 in both.
    let h = by("PinHeader_1x06_2.54");
    assert!(h.findings.is_empty(), "{:?}", h.findings);
    // The QFN's exposed pad paste windows are not copper.
    assert_eq!(by("QFN-32_5x5").kicad_pads, 33);
    // The tactile switch's origin: KiCad's at pin 1, Iṣẹ́ Studio's at the centre.
    let sw = by("SW_Tactile_6x6");
    assert!((sw.offset[0] + 3.25).abs() < 1e-6 && (sw.offset[1] - 2.25).abs() < 1e-6, "{:?}", sw.offset);
}

#[test]
fn a_moved_pad_is_an_error() {
    let (kicad, _) = lib().footprint("RF_Module:ESP32-WROOM-32E").unwrap();
    let mut ours = footprints::get("ESP32-WROOM-32").unwrap();
    let p = ours.pads.iter_mut().find(|p| p.number == "3").unwrap();
    p.at[0] += 0.8;
    let r = verify::compare(&ours, &kicad, "RF_Module:ESP32-WROOM-32E");
    assert_eq!(r.errors, 1, "{:#?}", r.findings);
    assert!(r.findings.iter().any(|x| x.severity == "error" && x.pad == "3"));
    let mut ours = footprints::get("SW_Tactile_6x6").unwrap();
    ours.pads.pop();
    let (kicad, _) = lib().footprint("Button_Switch_THT:SW_PUSH_6mm").unwrap();
    let r = verify::compare(&ours, &kicad, "Button_Switch_THT:SW_PUSH_6mm");
    assert!(r.findings.iter().any(|x| x.severity == "error" && x.message.contains("lacks")), "{:#?}", r.findings);
}

#[test]
fn kicad_footprints_read_exactly() {
    let (q, notes) = lib().footprint("Package_DFN_QFN:QFN-32-1EP_5x5mm_P0.5mm_EP3.45x3.45mm").unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(q.name, "Package_DFN_QFN:QFN-32-1EP_5x5mm_P0.5mm_EP3.45x3.45mm");
    assert_eq!(q.kind, "ic");
    assert_eq!(q.pads.len(), 33);
    let ep = q.pads.iter().find(|p| p.number == "33").unwrap();
    assert_eq!(ep.paste.len(), 9, "KiCad's paste windows go onto the exposed pad");
    // y is flipped up: KiCad's pin 1 is at the top left (y −1.75 there), here at +1.75.
    let p1 = q.pads.iter().find(|p| p.number == "1").unwrap();
    assert!(p1.at[0] < 0.0 && (p1.at[1] - 1.75).abs() < 1e-9, "{:?}", p1.at);
    let (usb, _) = lib().footprint("Connector_USB:USB_C_Receptacle_HRO_TYPE-C-31-M-12").unwrap();
    assert_eq!(usb.pads.iter().filter(|p| p.kind == PadKind::Npth).count(), 2);
    assert_eq!(usb.kind, "connector");
    assert!(usb.source.contains("CC-BY-SA"));
    let (hole, _) = lib().footprint("MountingHole:MountingHole_2.2mm_M2").unwrap();
    assert_eq!(hole.kind, "hole");
    assert!(lib().footprint("Nope:Nothing").is_err());
    assert!(lib().footprint("../etc:passwd").is_err());
}

#[test]
fn kicad_symbols_read_with_pins_and_parents() {
    let me = lib().symbol("Regulator_Linear:ME6211C33M5").unwrap();
    assert_eq!(me.name, "Regulator_Linear:ME6211C33M5");
    assert_eq!(me.prefix, "U");
    assert_eq!(me.footprint, "Package_TO_SOT_SMD:SOT-23-5");
    assert_eq!(me.category, "Power IC");
    assert_eq!(me.lcsc.as_deref(), Some("C82942"), "the MPN is known: its LCSC number comes along");
    let pin = |n: &str| me.pins.iter().find(|p| p.number == n).unwrap();
    assert_eq!(pin("1").name, "VIN");
    assert_eq!(pin("1").etype, PinType::PowerIn);
    assert_eq!(pin("1").side, Dir::Left);
    // KiCad's y is up: pin 1 at (−7.62, 2.54) there is above the centre, at y −2.54 on the sheet.
    assert_eq!(pin("1").at, [-7.62, -2.54]);
    assert_eq!(pin("2").side, Dir::Down);
    assert_eq!(pin("5").etype, PinType::PowerOut);
    assert!(pin("4").hidden);
    assert_eq!(me.units.len(), 1);
    // A derived symbol: its own name and footprint, its parent's pins.
    let ap = lib().symbol("Regulator_Linear:AP2112K-3.3").unwrap();
    assert_eq!(ap.value, "AP2112K-3.3");
    assert_eq!(ap.pins.len(), 5);
    assert!(ap.description.contains("600mA"));
    let r = lib().symbol("Device:R").unwrap();
    assert_eq!(r.prefix, "R");
    assert_eq!(r.mpn, None, "a generic part has no part number");
}

#[test]
fn stacked_kicad_pins_become_one_pin_with_every_pad() {
    // KiCad's USB-C (USB 2.0) receptacle: VBUS on A4, A9, B4, B9 and GND on A1, A12, B1, B12,
    // one visible pin each with the rest hidden under it; KiCad joins them.
    let usb = lib().symbol("Connector:USB_C_Receptacle_USB2.0_16P").unwrap();
    let pin = |n: &str| usb.pins.iter().find(|p| p.number == n).unwrap_or_else(|| panic!("no pin {n}: {:?}", usb.pins.iter().map(|p| &p.number).collect::<Vec<_>>()));
    assert_eq!(pin("A4").pads(), vec!["A4", "A9", "B4", "B9"]);
    assert_eq!(pin("A1").pads(), vec!["A1", "A12", "B1", "B12"]);
    assert!(!pin("A4").hidden && !pin("A1").hidden);
    assert!(usb.pins.iter().all(|p| p.number != "A9" && p.number != "B12"), "the hidden ones are folded in");
    // Pins on their own spots stay as they are: D+ is two pins (A6, B6), each its own pad.
    assert_eq!(pin("A6").pads(), vec!["A6"]);
    assert_eq!(pin("B6").pads(), vec!["B6"]);
    // Placed and synced, every contact gets its net.
    let mut p = Project::new("p", Arc::new(de_core::Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let board = serde_json::from_value::<StableId>(ok(&mut p, "pcb.board.create", json!({ "name": "b", "width": 30, "height": 20 }))["id"].clone()).unwrap();
    let sch = serde_json::from_value::<StableId>(ok(&mut p, "pcb.schematic.create", json!({ "board": board }))["id"].clone()).unwrap();
    ok(&mut p, "pcb.schematic.from_netlist", json!({ "schematic": sch, "parts": [{ "reference": "J1", "value": "USB-C", "footprint": "USB-C_16P", "definition": usb }], "nets": { "VBUS": ["J1.A4", "J1.A9", "J1.B4", "J1.B9"], "GND": ["J1.A1", "J1.A12", "J1.B1", "J1.B12"] } }));
    ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": sch, "place": { "J1": { "at": [15, 5] } } }));
    let v = de_pcb::view(p.world(), board).unwrap();
    for pad in ["A4", "A9", "B4", "B9"] {
        assert!(v.pads.iter().filter(|x| x.reference == "J1" && x.number == pad).all(|x| x.net.as_deref() == Some("VBUS")), "J1.{pad}");
    }
    for pad in ["A1", "A12", "B1", "B12"] {
        assert!(v.pads.iter().filter(|x| x.reference == "J1" && x.number == pad).all(|x| x.net.as_deref() == Some("GND")), "J1.{pad}");
    }
}

#[test]
fn the_index_finds_parts_by_name_and_words() {
    let l = lib();
    let hits = l.search("esp32 wroom", Repo::Footprints, 5).unwrap();
    assert_eq!(hits[0].id, "RF_Module:ESP32-WROOM-32E");
    let hits = l.search("ldo 3.3V", Repo::Symbols, 5).unwrap();
    assert!(hits.iter().any(|h| h.name == "ME6211C33M5"), "{hits:?}");
    assert_eq!(hits.iter().find(|h| h.name == "AP2112K-3.3").map(|h| h.count), Some(5), "a derived symbol counts its parent's pins");
}

#[test]
fn a_kicad_part_goes_on_the_schematic_and_onto_the_board() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let b = ok(&mut p, "pcb.board.create", json!({ "name": "B", "width": 30, "height": 20 }))["id"].clone();
    let s = ok(&mut p, "pcb.schematic.create", json!({ "board": b }))["id"].clone();
    let def = lib().symbol("Regulator_Linear:ME6211C33M5").unwrap();
    let (fp, _) = lib().footprint(&def.footprint).unwrap();
    let out = ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": def.name, "definition": def, "footprint_definition": fp, "at": [100, 80] }));
    assert_eq!(out["reference"], "U1");
    // A second one needs no definition: the first lends it.
    ok(&mut p, "pcb.schematic.place", json!({ "schematic": s, "symbol": "Regulator_Linear:ME6211C33M5", "at": [140, 80] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "VBUS", "pins": ["U1.1", "U1.3", "U2.1", "U2.3"] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "GND", "pins": ["U1.2", "U2.2"] }));
    ok(&mut p, "pcb.schematic.connect", json!({ "schematic": s, "net": "3V3", "pins": ["U1.5", "U2.5"] }));
    let sid: StableId = serde_json::from_value(s.clone()).unwrap();
    let v = de_pcb::schematic::view(p.world(), sid).unwrap();
    assert!(!v.violations.iter().any(|x| x.code == "unknown_footprint" || x.code == "pin_map"), "{:?}", v.violations);
    ok(&mut p, "pcb.schematic.update_board", json!({ "schematic": s }));
    let bid: StableId = serde_json::from_value(b).unwrap();
    let bv = de_pcb::view(p.world(), bid).unwrap();
    let u1 = bv.components.iter().find(|c| c.reference == "U1").unwrap();
    assert_eq!(u1.footprint, "Package_TO_SOT_SMD:SOT-23-5");
    assert_eq!(u1.lcsc.as_deref(), Some("C82942"));
    let vout = bv.pads.iter().find(|q| q.reference == "U1" && q.number == "5").unwrap();
    assert_eq!(vout.net.as_deref(), Some("3V3"));
    // The project replays from its log alone: the definitions travel in it.
    let text = serde_json::to_string(&p.get::<de_pcb::schematic::SchSymbol>(v.symbols[0].id).unwrap()).unwrap();
    assert!(text.contains("\"definition\""));
}
