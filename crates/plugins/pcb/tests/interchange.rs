//! EDA interchange, checked without the other tools: every export of the
//! ESP32 example is read back (KiCad schematic and project round trips,
//! netlists, IPC-D-356, IPC-2581, ODB++, Gerber job, drill map and report,
//! plots, mesh models, position files), and every importer reads a small
//! board (and, where the format has one, a schematic) written here from the
//! format's documented structure, landing where it should in Forma terms.
//!
//! The board every importer fixture describes (Forma coordinates, mm, y up):
//! a 30 × 20 mm outline; R1 (0603: pads 0.9 × 1.0 at ±0.8 mm) at (10, 10)
//! turned 90°; R2 (the same) at (20, 10) on the bottom; J1 (two through-hole
//! pins, 1.0 mm drill, 1.7 mm pads) at (5, 5) and (5, 7.54); nets VCC
//! (R1.1, J1.1, joined by a top track), GND (R1.2, R2.1, J1.2, a via at
//! (15, 15)) and SIG (R2.2).

use base64::Engine;
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::io::{self, altium, archive};
use de_pcb::{PcbPlugin, schematic};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])))
}

fn run(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, CommandError> {
    let envelope = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: p.id().into(), expected_revision: p.revision(), operation: operation.into(), parameters, actor: Actor { user_id: "u".into(), session_id: "s".into() } };
    p.commit(envelope).map(|c| c.output)
}

fn ok(p: &mut Project, operation: &str, parameters: Value) -> Value {
    run(p, operation, parameters).unwrap_or_else(|e| panic!("{operation}: {e}"))
}

fn sid(v: &Value) -> StableId {
    serde_json::from_value(v.clone()).unwrap()
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

/// The routed ESP32 example with a schematic drawn from it.
fn example() -> (Project, StableId, StableId) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("steps.json")).unwrap()).unwrap();
    let mut p = project();
    let mut outs = Vec::new();
    for s in &steps {
        let params = substitute(&s["parameters"], &outs);
        outs.push(ok(&mut p, s["operation"].as_str().unwrap(), params));
    }
    let board = sid(&outs[0]["id"]);
    let s = ok(&mut p, "pcb.schematic.from_board", json!({ "board": board }));
    (p, board, sid(&s["id"]))
}

fn file(p: &Project, board: StableId, name: &str) -> Vec<u8> {
    match io::board_file(p.world(), board, name) {
        Some(Ok(f)) => f.bytes,
        Some(Err(e)) => panic!("{name}: {e}"),
        None => panic!("no export {name}"),
    }
}

fn text(p: &Project, board: StableId, name: &str) -> String {
    String::from_utf8(file(p, board, name)).unwrap()
}

fn sch_nets(v: &schematic::SchematicView) -> BTreeSet<BTreeSet<String>> {
    v.nets.iter().filter(|n| n.pads.len() > 1).map(|n| n.pads.iter().cloned().collect()).collect()
}

#[test]
fn every_board_export_is_written_and_reads_back() {
    let (p, board, sch) = example();
    let v = de_pcb::view(p.world(), board).unwrap();
    let s = de_pcb::gerber::file_stem(&v.name);
    let names = io::board_exports(&v, true);
    assert!(names.len() > 25);
    for (n, _) in &names {
        let bytes = file(&p, board, n);
        assert!(bytes.len() > 50, "{n} is empty");
    }
    // KiCad project: the classes come back.
    let pro = text(&p, board, &format!("{s}.kicad_pro"));
    let (classes, _) = io::kicad_pro::read(&pro).unwrap();
    let b = p.get::<de_pcb::Board>(board).unwrap();
    for c in &b.net_classes {
        let back = classes.iter().find(|x| x.name.eq_ignore_ascii_case(&c.name)).unwrap_or_else(|| panic!("class {}", c.name));
        assert_eq!(back.track_width, c.track_width);
        if !c.name.eq_ignore_ascii_case("default") {
            assert_eq!(back.nets.iter().collect::<BTreeSet<_>>(), c.nets.iter().collect::<BTreeSet<_>>());
        }
    }
    // Netlists: every net and node of the schematic.
    let sv = schematic::view(p.world(), sch).unwrap();
    let net_count = sv.nets.iter().filter(|n| !n.pads.is_empty()).count();
    let kn = text(&p, board, &format!("{s}.net"));
    let e = de_pcb::sexpr::parse(&kn).unwrap();
    let nets = e[0].find("nets").unwrap();
    assert_eq!(nets.find_all("net").count(), net_count);
    let xml = io::xml::parse(&text(&p, board, &format!("{s}-netlist.xml"))).unwrap();
    assert_eq!(xml.child("nets").unwrap().all("net").count(), net_count);
    assert_eq!(xml.child("components").unwrap().all("comp").count(), e[0].find("components").unwrap().find_all("comp").count());
    for n in ["-orcad.net", "-pads.net", "-allegro.net", "-cadstar.frp"] {
        let t = text(&p, board, &format!("{s}{n}"));
        assert!(t.contains("GND"), "{n}");
    }
    // IPC-D-356: a test point per pad and via, in the right place.
    let d356 = io::ipc356::read(&text(&p, board, &format!("{s}.d356")));
    assert_eq!(d356.len(), v.pads.len() + v.vias.len());
    let first = &v.pads[0];
    let tp = d356.iter().find(|t| t.2 == first.reference && t.3 == first.number).unwrap();
    assert!((tp.4 - (first.at[0] - v.bounds[0])).abs() < 0.002 && (tp.5 - (first.at[1] - v.bounds[1])).abs() < 0.002);
    assert_eq!(tp.1, first.net.clone().unwrap_or_else(|| "N/C".into()));
    // IPC-2581: components, nets, layers, holes.
    let ipc = io::xml::parse(&text(&p, board, &format!("{s}-ipc2581.xml"))).unwrap();
    assert_eq!(ipc.attr("revision"), Some("C"));
    let step = ipc.path("Ecad/CadData/Step").unwrap();
    assert_eq!(step.all("Component").count(), v.components.len());
    assert_eq!(step.all("LogicalNet").count(), v.nets.len());
    let mut holes = Vec::new();
    step.descendants("Hole", &mut holes);
    let mut slots = Vec::new();
    step.descendants("SlotCavity", &mut slots);
    assert_eq!(holes.len() + slots.len(), v.holes.len());
    // ODB++: the tree, its nets and components.
    let odb = archive::unzip(&file(&p, board, &format!("{s}-odb.zip"))).unwrap();
    let job = s.to_ascii_lowercase();
    let eda = String::from_utf8(odb[&format!("{job}/steps/pcb/eda/data")].clone()).unwrap();
    for n in &v.nets {
        assert!(eda.lines().any(|l| l == format!("NET {}", n.name)), "ODB++ net {}", n.name);
    }
    let comps = String::from_utf8(odb[&format!("{job}/steps/pcb/layers/comp_+_top/components")].clone()).unwrap() + &String::from_utf8(odb[&format!("{job}/steps/pcb/layers/comp_+_bot/components")].clone()).unwrap();
    assert_eq!(comps.lines().filter(|l| l.starts_with("CMP ")).count(), v.components.len());
    assert!(odb.contains_key(&format!("{job}/matrix/matrix")) && odb.contains_key(&format!("{job}/steps/pcb/layers/top/features")));
    // Gerber job: every Gerber and drill file listed with its function.
    let job: Value = serde_json::from_str(&text(&p, board, &format!("{s}-job.gbrjob"))).unwrap();
    assert_eq!(job["FilesAttributes"].as_array().unwrap().len(), de_pcb::gerber::files(&v).len());
    assert_eq!(job["GeneralSpecs"]["LayerNumber"], json!(v.layers.len()));
    // Drill map (Gerber reads back), report totals.
    let map = de_pcb::reader::gerber(&text(&p, board, &format!("{s}-drill-map.gbr"))).unwrap();
    assert!(!map.objects.is_empty());
    let report = text(&p, board, &format!("{s}-drill-report.txt"));
    let plated = v.holes.iter().filter(|h| h.plated).count();
    assert!(report.contains(&format!("Total plated holes count {plated}")), "{report}");
    // Plots: SVGs are XML, PDFs are PDFs.
    for n in names.iter().filter(|n| n.0.ends_with(".svg")) {
        io::xml::parse(&text(&p, board, &n.0)).unwrap_or_else(|e| panic!("{}: {e}", n.0));
    }
    for n in ["-board.pdf", "-drill-map.pdf"] {
        assert!(file(&p, board, &format!("{s}{n}")).starts_with(b"%PDF-1.4"));
    }
    // Models.
    let glb = file(&p, board, &format!("{s}.glb"));
    assert_eq!(&glb[..4], b"glTF");
    assert_eq!(u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize, glb.len());
    assert!(text(&p, board, &format!("{s}.wrl")).starts_with("#VRML V2.0 utf8"));
    // Positions: one line per placed part.
    let placed = de_pcb::outputs::placed(&v).len();
    assert_eq!(text(&p, board, &format!("{s}-both.pos")).lines().filter(|l| !l.starts_with('#')).count(), placed);
    assert_eq!(text(&p, board, &format!("{s}-pos.csv")).lines().count(), placed + 1);
    let x3 = de_pcb::reader::gerber(&text(&p, board, &format!("{s}-top-pnp.gbr"))).unwrap();
    assert!(x3.objects.iter().any(|o| o.component.is_some()));
}

#[test]
fn kicad_schematic_and_bundle_round_trip() {
    let (mut p, board, sch) = example();
    let before = sch_nets(&schematic::view(p.world(), sch).unwrap());
    let stem = "rt";
    let files = io::kicad_sch::write_world(p.world(), sch, stem).unwrap();
    let (root_name, root) = files[0].clone();
    let companions: BTreeMap<String, String> = files.iter().skip(1).cloned().collect();
    let out = ok(&mut p, "pcb.import", json!({ "file": root_name, "text": root, "files": companions }));
    assert_eq!(out["format"], "kicad_sch");
    let s2 = sid(&out["schematic"]["id"]);
    let v2 = schematic::view(p.world(), s2).unwrap();
    assert_eq!(sch_nets(&v2), before, "same nets after .kicad_sch out and in");
    let v1 = schematic::view(p.world(), sch).unwrap();
    assert_eq!(v2.symbols.len(), v1.symbols.len());
    // The whole project as a bundle: board, schematic, classes; the schematic drives the new board.
    let v = de_pcb::view(p.world(), board).unwrap();
    let s = de_pcb::gerber::file_stem(&v.name);
    let zip = file(&p, board, &format!("{s}-kicad.zip"));
    let data = base64::engine::general_purpose::STANDARD.encode(&zip);
    let out = ok(&mut p, "pcb.import", json!({ "file": "bundle.zip", "data": data }));
    assert_eq!(out["format"], "kicad_project");
    let b2 = sid(&out["board"]["id"]);
    let s3 = sid(&out["schematic"]["id"]);
    assert_eq!(p.get::<de_pcb::schematic::Schematic>(s3).unwrap().board, Some(b2));
    let nb = p.get::<de_pcb::Board>(b2).unwrap();
    let ob = p.get::<de_pcb::Board>(board).unwrap();
    assert!(nb.net_classes.len() >= ob.net_classes.len());
    let pending = de_pcb::schematic_sync::pending(p.world(), s3).unwrap();
    assert!(pending.in_sync, "imported schematic agrees with the imported board: {pending:#?}");
}

// ---- Importer fixtures ------------------------------------------------------------------

/// (reference, pad, x, y, net)
const PADS: [(&str, &str, f64, f64, &str); 6] = [("R1", "1", 10.0, 9.2, "VCC"), ("R1", "2", 10.0, 10.8, "GND"), ("R2", "1", 20.8, 10.0, "GND"), ("R2", "2", 19.2, 10.0, "SIG"), ("J1", "1", 5.0, 5.0, "VCC"), ("J1", "2", 5.0, 7.54, "GND")];

fn check_board(p: &Project, out: &Value, via_net: bool, label: &str) {
    let id = sid(&out["board"]["id"]);
    let v = de_pcb::view(p.world(), id).unwrap_or_else(|| panic!("{label}: board not worked out: {:?}", p.status(id)));
    let size = v.size();
    assert!((size[0] - 30.0).abs() < 0.02 && (size[1] - 20.0).abs() < 0.02, "{label}: size {size:?} notes {}", out["notes"]);
    let o = [v.bounds[0], v.bounds[1]];
    for (r, n, x, y, net) in PADS {
        let pad = v.pads.iter().find(|p| p.reference == r && p.number == n).unwrap_or_else(|| panic!("{label}: no pad {r}.{n}; pads {:?}", v.pads.iter().map(|p| format!("{}.{}", p.reference, p.number)).collect::<Vec<_>>()));
        assert!((pad.at[0] - o[0] - x).abs() < 0.02 && (pad.at[1] - o[1] - y).abs() < 0.02, "{label}: {r}.{n} at {:?}, expected [{x}, {y}]", [pad.at[0] - o[0], pad.at[1] - o[1]]);
        assert_eq!(pad.net.as_deref(), Some(net), "{label}: {r}.{n} net");
    }
    let r2 = v.components.iter().find(|c| c.reference == "R2").unwrap();
    assert_eq!(r2.side, de_pcb::Side::Bottom, "{label}: R2 on the bottom");
    let j1 = v.pads.iter().find(|p| p.reference == "J1" && p.number == "1").unwrap();
    assert!(j1.drill.is_some_and(|d| (d[0] - 1.0).abs() < 0.02), "{label}: J1 drill {:?}", j1.drill);
    assert_eq!(v.vias.len(), 1, "{label}: one via");
    assert!((v.vias[0].at[0] - o[0] - 15.0).abs() < 0.02 && (v.vias[0].at[1] - o[1] - 15.0).abs() < 0.02, "{label}: via at {:?}", v.vias[0].at);
    if via_net {
        assert_eq!(v.vias[0].net.as_deref(), Some("GND"), "{label}: via net");
    }
    let t = v.tracks.iter().find(|t| t.net.as_deref() == Some("VCC")).unwrap_or_else(|| panic!("{label}: no VCC track"));
    assert_eq!(t.layer, "top");
    let r1 = v.pads.iter().find(|p| p.reference == "R1" && p.number == "1").unwrap();
    let bb = r1.prim.bbox();
    assert!(((bb[2] - bb[0]) - 1.0).abs() < 0.03 && ((bb[3] - bb[1]) - 0.9).abs() < 0.03, "{label}: R1.1 turned with its part: {bb:?}");
}

fn import_text(name: &str, text: &str) -> (Project, Value) {
    let mut p = project();
    let out = ok(&mut p, "pcb.import", json!({ "file": name, "text": text }));
    (p, out)
}

fn import_bytes(name: &str, bytes: &[u8]) -> (Project, Value) {
    let mut p = project();
    let out = ok(&mut p, "pcb.import", json!({ "file": name, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }));
    (p, out)
}

const EAGLE_BRD: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE eagle SYSTEM "eagle.dtd">
<eagle version="9.6.2"><drawing><layers><layer number="1" name="Top"/><layer number="16" name="Bottom"/><layer number="20" name="Dimension"/></layers>
<board><plain>
<wire x1="0" y1="0" x2="30" y2="0" width="0" layer="20"/><wire x1="30" y1="0" x2="30" y2="20" width="0" layer="20"/>
<wire x1="30" y1="20" x2="0" y2="20" width="0" layer="20"/><wire x1="0" y1="20" x2="0" y2="0" width="0" layer="20"/>
</plain>
<libraries><library name="rcl"><packages><package name="R0603">
<smd name="1" x="-0.8" y="0" dx="0.9" dy="1" layer="1"/><smd name="2" x="0.8" y="0" dx="0.9" dy="1" layer="1"/>
<wire x1="-0.2" y1="0.6" x2="0.2" y2="0.6" width="0.12" layer="21"/></package></packages></library>
<library name="con"><packages><package name="PIN2"><pad name="1" x="0" y="0" drill="1" diameter="1.7" shape="square"/><pad name="2" x="0" y="2.54" drill="1" diameter="1.7"/></package></packages></library></libraries>
<elements>
<element name="R1" library="rcl" package="R0603" value="10k" x="10" y="10" rot="R90"/>
<element name="R2" library="rcl" package="R0603" value="1k" x="20" y="10" rot="MR0"/>
<element name="J1" library="con" package="PIN2" value="HDR" x="5" y="5"/>
</elements>
<signals>
<signal name="VCC"><contactref element="R1" pad="1"/><contactref element="J1" pad="1"/><wire x1="5" y1="5" x2="10" y2="9.2" width="0.25" layer="1"/></signal>
<signal name="GND"><contactref element="R1" pad="2"/><contactref element="R2" pad="1"/><contactref element="J1" pad="2"/><via x="15" y="15" extent="1-16" drill="0.3" diameter="0.6"/></signal>
<signal name="SIG"><contactref element="R2" pad="2"/></signal>
</signals></board></drawing></eagle>"#;

#[test]
fn eagle_board() {
    let (p, out) = import_text("demo.brd", EAGLE_BRD);
    assert_eq!(out["format"], "eagle_brd");
    check_board(&p, &out, true, "eagle");
}

/// Two resistors and a ground symbol: R1.2–R2.1 wired with a label MID, R2.2 on GND.
fn check_schematic(p: &Project, out: &Value, label: &str) {
    let id = sid(&out["schematic"]["id"]);
    let v = schematic::view(p.world(), id).unwrap_or_else(|| panic!("{label}: not worked out {:?}", p.status(id)));
    let net = |pin: &str| -> Option<String> {
        let (r, n) = pin.split_once('.').unwrap();
        v.symbols.iter().filter(|s| s.reference == r).flat_map(|s| s.pins.iter()).find(|p| p.number == n).and_then(|p| p.net.clone())
    };
    assert!(v.symbols.iter().any(|s| s.reference == "R1") && v.symbols.iter().any(|s| s.reference == "R2"), "{label}: symbols {:?}", v.symbols.iter().map(|s| s.reference.clone()).collect::<Vec<_>>());
    assert_eq!(net("R1.2"), Some("MID".to_string()), "{label}: R1.2; nets {:?}", v.nets.iter().map(|n| (n.name.clone(), n.pins.clone())).collect::<Vec<_>>());
    assert_eq!(net("R2.1"), Some("MID".to_string()), "{label}: R2.1");
    assert_eq!(net("R2.2"), Some("GND".to_string()), "{label}: R2.2");
    assert_ne!(net("R1.1").as_deref(), Some("MID"), "{label}: R1.1 is not on MID");
}

const EAGLE_SCH: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<eagle version="9.6.2"><drawing><schematic>
<libraries><library name="rcl"><symbols><symbol name="R">
<wire x1="-2.54" y1="-0.889" x2="2.54" y2="-0.889" width="0.254" layer="94"/><wire x1="2.54" y1="0.889" x2="-2.54" y2="0.889" width="0.254" layer="94"/>
<pin name="1" x="-5.08" y="0" visible="off" length="short" direction="pas" rot="R0"/>
<pin name="2" x="5.08" y="0" visible="off" length="short" direction="pas" rot="R180"/>
<text x="-3.81" y="1.4" size="1.778" layer="95">&gt;NAME</text></symbol></symbols>
<devicesets><deviceset name="R" prefix="R"><gates><gate name="G$1" symbol="R" x="0" y="0"/></gates>
<devices><device name="0603" package="R0603"><connects><connect gate="G$1" pin="1" pad="1"/><connect gate="G$1" pin="2" pad="2"/></connects></device></devices></deviceset></devicesets></library>
<library name="supply"><symbols><symbol name="GND"><wire x1="-1.905" y1="0" x2="1.905" y2="0" width="0.254" layer="94"/>
<pin name="GND" x="0" y="2.54" visible="off" length="short" direction="sup" rot="R270"/></symbol></symbols>
<devicesets><deviceset name="GND" prefix="SUPPLY"><gates><gate name="1" symbol="GND" x="0" y="0"/></gates><devices><device name=""/></devices></deviceset></devicesets></library></libraries>
<parts><part name="R1" library="rcl" deviceset="R" device="0603" value="10k"/><part name="R2" library="rcl" deviceset="R" device="0603" value="1k"/><part name="SUPPLY1" library="supply" deviceset="GND" device=""/></parts>
<sheets><sheet><instances>
<instance part="R1" gate="G$1" x="20.32" y="20.32"/><instance part="R2" gate="G$1" x="40.64" y="20.32"/><instance part="SUPPLY1" gate="1" x="45.72" y="17.78"/>
</instances><nets>
<net name="MID" class="0"><segment><pinref part="R1" gate="G$1" pin="2"/><pinref part="R2" gate="G$1" pin="1"/><wire x1="25.4" y1="20.32" x2="35.56" y2="20.32" width="0.1524" layer="91"/><label x="30.48" y="20.32" size="1.778" layer="95"/></segment></net>
<net name="GND" class="0"><segment><pinref part="R2" gate="G$1" pin="2"/><pinref part="SUPPLY1" gate="1" pin="GND"/></segment></net>
</nets></sheet></sheets></schematic></drawing></eagle>"#;

#[test]
fn eagle_schematic() {
    let (p, out) = import_text("demo.sch", EAGLE_SCH);
    assert_eq!(out["format"], "eagle_sch");
    check_schematic(&p, &out, "eagle");
}

// EasyEDA Standard: 10 mil units, y down, rotations clockwise.
fn eu(v: f64) -> String {
    format!("{:.4}", v / 0.254)
}
fn ey(v: f64) -> String {
    format!("{:.4}", -v / 0.254)
}

fn easyeda_pcb() -> String {
    let pad = |shape: &str, x: f64, y: f64, w: f64, h: f64, layer: &str, net: &str, num: &str, hole: f64, rot: f64| format!("PAD~{shape}~{}~{}~{}~{}~{layer}~{net}~{num}~{}~~{}~gge{num}~0~~Y~0", eu(x), ey(y), eu(w), eu(h), eu(hole / 2.0), -rot);
    let shapes = vec![
        format!("TRACK~1~10~~{} {} {} {} {} {} {} {} {} {}~gge1~0", eu(0.0), ey(0.0), eu(30.0), ey(0.0), eu(30.0), ey(20.0), eu(0.0), ey(20.0), eu(0.0), ey(0.0)),
        format!("LIB~{}~{}~package`R0603`~270~~gge2~0#@${}#@${}#@$TEXT~P~{}~{}~0.8~0~0~3~~4.5~R1~~~gge9~0", eu(10.0), ey(10.0), pad("RECT", 10.0, 9.2, 0.9, 1.0, "1", "VCC", "1", 0.0, 90.0), pad("RECT", 10.0, 10.8, 0.9, 1.0, "1", "GND", "2", 0.0, 90.0), eu(10.0), ey(12.0)),
        format!("LIB~{}~{}~package`R0603`~0~~gge3~0#@${}#@${}#@$TEXT~P~{}~{}~0.8~0~0~4~~4.5~R2~~~gge10~0", eu(20.0), ey(10.0), pad("RECT", 20.8, 10.0, 0.9, 1.0, "2", "GND", "1", 0.0, 0.0), pad("RECT", 19.2, 10.0, 0.9, 1.0, "2", "SIG", "2", 0.0, 0.0), eu(20.0), ey(12.0)),
        format!("LIB~{}~{}~package`PIN2`~0~~gge4~0#@${}#@${}#@$TEXT~P~{}~{}~0.8~0~0~3~~4.5~J1~~~gge11~0", eu(5.0), ey(5.0), pad("RECT", 5.0, 5.0, 1.7, 1.7, "11", "VCC", "1", 1.0, 0.0), pad("ELLIPSE", 5.0, 7.54, 1.7, 1.7, "11", "GND", "2", 1.0, 0.0), eu(5.0), ey(3.0)),
        format!("TRACK~{}~1~VCC~{} {} {} {}~gge5~0", eu(0.25), eu(5.0), ey(5.0), eu(10.0), ey(9.2)),
        format!("VIA~{}~{}~{}~GND~{}~gge6~0", eu(15.0), ey(15.0), eu(0.6), eu(0.15)),
    ];
    serde_json::to_string(&json!({ "head": { "docType": "3", "editorVersion": "6.5" }, "canvas": "CA~1000~1000~#000000~yes", "shape": shapes, "layers": ["1~TopLayer~#FF0000~true~true~true~", "2~BottomLayer~#0000FF~true~false~true~"] })).unwrap()
}

#[test]
fn easyeda_standard_board() {
    let (p, out) = import_text("demo.json", &easyeda_pcb());
    assert_eq!(out["format"], "easyeda_pcb");
    check_board(&p, &out, true, "easyeda");
}

fn easyeda_sch() -> String {
    // A resistor with pins whose tips are at ±5 units (1.27 mm × 10) from its centre.
    let lib = |r: &str, x: f64| -> String {
        let pin = |n: &str, tip: f64, dir: f64| format!("P~show~0~{n}~{tip}~200~0~gge{r}{n}~0^^{tip}~200^^M {tip} 200 h {}~#880000^^1~{tip}~200~0~{n}~start~~~#0000FF^^1~{tip}~200~0~{n}~end~~~#0000FF^^0~{tip}~200^^0~M {tip} 200 L {tip} 200", 10.0 * dir);
        format!("LIB~{x}~200~package`R0603`Value`1k`~0~0~gge{r}~0#@$T~P~{x}~190~0~#000080~Arial~~~~~comment~{r}~1~start~gge{r}t~0#@$R~{}~196~~~20~8~#A00000~1~0~none~gge{r}b~0#@${}#@${}", x - 10.0, pin("1", x - 20.0, 1.0), pin("2", x + 20.0, -1.0))
    };
    let shapes = vec![lib("R1", 100.0), lib("R2", 200.0), "W~120 200 180 200~#008800~1~0~none~gge20~0".to_string(), "N~150~200~0~#0000ff~MID~gge21~start~150~200~Times New Roman~~".to_string(), "F~part_netLabel_gnD~220~200~270~gge22~~0^^220~200^^GND~#000000~220~215~0~start~1~Times New Roman~9pt~flag_gge23^^PL~210 210 230 210~#000000~1~0~transparent~gge24~0".to_string()];
    serde_json::to_string(&json!({ "head": { "docType": "1", "editorVersion": "6.5" }, "canvas": "CA~1000~1000~#FFFFFF~yes", "shape": shapes })).unwrap()
}

#[test]
fn easyeda_standard_schematic() {
    let (p, out) = import_text("demo-sch.json", &easyeda_sch());
    assert_eq!(out["format"], "easyeda_sch");
    check_schematic(&p, &out, "easyeda");
}

fn mil(v: f64) -> f64 {
    v / 0.0254
}

#[test]
fn easyeda_pro_project() {
    let line = |v: Value| serde_json::to_string(&v).unwrap();
    let fp_r = [line(json!(["DOCTYPE", "FOOTPRINT", "1.8"])), line(json!(["PAD", "p1", 0, "", 1, "1", mil(-0.8), 0, 0, null, ["RECT", mil(0.9), mil(1.0), 0]])), line(json!(["PAD", "p2", 0, "", 1, "2", mil(0.8), 0, 0, null, ["RECT", mil(0.9), mil(1.0), 0]]))].join("\n");
    let fp_j = [line(json!(["DOCTYPE", "FOOTPRINT", "1.8"])), line(json!(["PAD", "p1", 0, "", 12, "1", 0, 0, 0, ["ROUND", mil(1.0), mil(1.0)], ["RECT", mil(1.7), mil(1.7), 0]])), line(json!(["PAD", "p2", 0, "", 12, "2", 0, mil(2.54), 0, ["ROUND", mil(1.0), mil(1.0)], ["ELLIPSE", mil(1.7), mil(1.7)]]))].join("\n");
    let mut pcb = vec![line(json!(["DOCTYPE", "PCB", "1.8"])), line(json!(["POLY", "o1", 0, "", 11, 0, [0, 0, "L", mil(30.0), 0, mil(30.0), mil(20.0), 0, mil(20.0)]]))];
    for (id, layer, x, y, rot, des, fp) in [("c1", 1, 10.0, 10.0, 90.0, "R1", "fp-r0603"), ("c2", 2, 20.0, 10.0, 0.0, "R2", "fp-r0603"), ("c3", 1, 5.0, 5.0, 0.0, "J1", "fp-pin2")] {
        pcb.push(line(json!(["COMPONENT", id, 0, layer, mil(x), mil(y), rot, {}, 0])));
        pcb.push(line(json!(["ATTR", format!("{id}a"), 0, id, 3, 0, 0, "Designator", des])));
        pcb.push(line(json!(["ATTR", format!("{id}f"), 0, id, 3, 0, 0, "Footprint", fp])));
    }
    for (c, pad, net) in [("c1", "1", "VCC"), ("c1", "2", "GND"), ("c2", "1", "GND"), ("c2", "2", "SIG"), ("c3", "1", "VCC"), ("c3", "2", "GND")] {
        pcb.push(line(json!(["PAD_NET", c, pad, net])));
    }
    pcb.push(line(json!(["LINE", "t1", 0, "VCC", 1, mil(5.0), mil(5.0), mil(10.0), mil(9.2), mil(0.25)])));
    pcb.push(line(json!(["VIA", "v1", 0, "GND", "", mil(15.0), mil(15.0), mil(0.3), mil(0.6)])));
    let zip = de_shape::fabricate::zip(&[("project.json".into(), b"{}".to_vec()), ("PCB/board.epcb".into(), pcb.join("\n").into_bytes()), ("FOOTPRINT/fp-r0603.efoo".into(), fp_r.into_bytes()), ("FOOTPRINT/fp-pin2.efoo".into(), fp_j.into_bytes())]);
    let (p, out) = import_bytes("demo.epro", &zip);
    assert_eq!(out["format"], "easyeda_pro");
    check_board(&p, &out, true, "easyeda pro");
}

// Altium: internal units are 1/10000 mil.
fn altium_streams() -> Vec<(&'static str, Vec<u8>)> {
    let ml = |v: f64| format!("{:.4}mil", mil(v));
    let board = format!("|RECORD=Board|VX0={}|VY0={}|KIND0=0|VX1={}|VY1={}|KIND1=0|VX2={}|VY2={}|KIND2=0|VX3={}|VY3={}|KIND3=0", ml(0.0), ml(0.0), ml(30.0), ml(0.0), ml(30.0), ml(20.0), ml(0.0), ml(20.0));
    let comps = [format!("|SOURCEDESIGNATOR=R1|PATTERN=R0603|X={}|Y={}|ROTATION=90.000|LAYER=TOP|COMMENT=10k", ml(10.0), ml(10.0)), format!("|SOURCEDESIGNATOR=R2|PATTERN=R0603|X={}|Y={}|ROTATION=0.000|LAYER=BOTTOM|COMMENT=1k", ml(20.0), ml(10.0)), format!("|SOURCEDESIGNATOR=J1|PATTERN=PIN2|X={}|Y={}|ROTATION=0.000|LAYER=TOP", ml(5.0), ml(5.0))];
    let nets = ["|NAME=VCC", "|NAME=GND", "|NAME=SIG"];
    let pad = |name: &str, layer: u8, net: u16, comp: u16, at: [f64; 2], size: [f64; 2], hole: f64, shape: u8, rot: f64| altium::APad { name: name.into(), layer, net, component: comp, at, size, hole, shape, rotation: rot, plated: true, slot: 0.0, slot_rotation: 0.0, corner: 0 };
    let pads = [pad("1", 1, 0, 0, [10.0, 9.2], [0.9, 1.0], 0.0, 2, 90.0), pad("2", 1, 1, 0, [10.0, 10.8], [0.9, 1.0], 0.0, 2, 90.0), pad("1", 32, 1, 1, [20.8, 10.0], [0.9, 1.0], 0.0, 2, 0.0), pad("2", 32, 2, 1, [19.2, 10.0], [0.9, 1.0], 0.0, 2, 0.0), pad("1", 74, 0, 2, [5.0, 5.0], [1.7, 1.7], 1.0, 2, 0.0), pad("2", 74, 1, 2, [5.0, 7.54], [1.7, 1.7], 1.0, 1, 0.0)];
    let mut pads6 = Vec::new();
    for p in &pads {
        pads6.extend(altium::write_pad(p));
    }
    let tracks6 = altium::write_track(&altium::ATrack { layer: 1, net: 0, component: u16::MAX, a: [5.0, 5.0], b: [10.0, 9.2], width: 0.25 });
    let vias6 = altium::write_via(&altium::AVia { net: 1, at: [15.0, 15.0], diameter: 0.6, hole: 0.3 });
    let comp_refs: Vec<&str> = comps.iter().map(String::as_str).collect();
    vec![("FileHeader", b"PCB 6.0 Binary File".to_vec()), ("Board6/Data", altium::write_props(&[&board])), ("Components6/Data", altium::write_props(&comp_refs)), ("Nets6/Data", altium::write_props(&nets)), ("Pads6/Data", pads6), ("Tracks6/Data", tracks6), ("Vias6/Data", vias6)]
}

#[test]
fn altium_binary_and_ascii_boards() {
    let cfb = archive::cfb_write(&altium_streams());
    let (p, out) = import_bytes("demo.PcbDoc", &cfb);
    assert_eq!(out["format"], "altium_pcb");
    check_board(&p, &out, true, "altium binary");
    // ASCII: the same board as records, one per line.
    let ml = |v: f64| format!("{:.4}mil", mil(v));
    let mut lines = vec![format!("|RECORD=Board|VX0={}|VY0={}|KIND0=0|VX1={}|VY1={}|KIND1=0|VX2={}|VY2={}|KIND2=0|VX3={}|VY3={}|KIND3=0", ml(0.0), ml(0.0), ml(30.0), ml(0.0), ml(30.0), ml(20.0), ml(0.0), ml(20.0))];
    for n in ["VCC", "GND", "SIG"] {
        lines.push(format!("|RECORD=Net|NAME={n}"));
    }
    lines.push(format!("|RECORD=Component|SOURCEDESIGNATOR=R1|PATTERN=R0603|X={}|Y={}|ROTATION=90|LAYER=TOP", ml(10.0), ml(10.0)));
    lines.push(format!("|RECORD=Component|SOURCEDESIGNATOR=R2|PATTERN=R0603|X={}|Y={}|ROTATION=0|LAYER=BOTTOM", ml(20.0), ml(10.0)));
    lines.push(format!("|RECORD=Component|SOURCEDESIGNATOR=J1|PATTERN=PIN2|X={}|Y={}|ROTATION=0|LAYER=TOP", ml(5.0), ml(5.0)));
    for (name, layer, net, comp, x, y, w, h, hole, rot, shape) in [("1", "TOP", 0, 0, 10.0, 9.2, 0.9, 1.0, 0.0, 90, "RECTANGLE"), ("2", "TOP", 1, 0, 10.0, 10.8, 0.9, 1.0, 0.0, 90, "RECTANGLE"), ("1", "BOTTOM", 1, 1, 20.8, 10.0, 0.9, 1.0, 0.0, 0, "RECTANGLE"), ("2", "BOTTOM", 2, 1, 19.2, 10.0, 0.9, 1.0, 0.0, 0, "RECTANGLE"), ("1", "MULTILAYER", 0, 2, 5.0, 5.0, 1.7, 1.7, 1.0, 0, "RECTANGLE"), ("2", "MULTILAYER", 1, 2, 5.0, 7.54, 1.7, 1.7, 1.0, 0, "ROUND")] {
        lines.push(format!("|RECORD=Pad|NAME={name}|LAYER={layer}|NET={net}|COMPONENT={comp}|X={}|Y={}|XSIZE={}|YSIZE={}|HOLESIZE={}|ROTATION={rot}|SHAPE={shape}", ml(x), ml(y), ml(w), ml(h), ml(hole)));
    }
    lines.push(format!("|RECORD=Track|LAYER=TOP|NET=0|X1={}|Y1={}|X2={}|Y2={}|WIDTH={}", ml(5.0), ml(5.0), ml(10.0), ml(9.2), ml(0.25)));
    lines.push(format!("|RECORD=Via|NET=1|X={}|Y={}|DIAMETER={}|HOLESIZE={}", ml(15.0), ml(15.0), ml(0.6), ml(0.3)));
    let (p, out) = import_text("demo-ascii.PcbDoc", &lines.join("\n"));
    assert_eq!(out["format"], "altium_pcb");
    check_board(&p, &out, true, "altium ascii");
}

#[test]
fn altium_schematic() {
    // Coordinates in 10 mil: R1 centred at (200, 300), pins 30 long.
    let mut recs: Vec<String> = vec!["|HEADER=Protel for Windows - Schematic Capture Binary File Version 5.0|WEIGHT=20".into()];
    let mut idx = 0;
    let mut part = |recs: &mut Vec<String>, r: &str, x: i64| {
        let me = idx;
        recs.push(format!("|RECORD=1|LIBREFERENCE=Res|LOCATION.X={x}|LOCATION.Y=300|CURRENTPARTID=1|PARTCOUNT=2"));
        recs.push(format!("|RECORD=14|OWNERINDEX={me}|OWNERPARTID=1|LOCATION.X={}|LOCATION.Y=296|CORNER.X={}|CORNER.Y=304", x - 10, x + 10));
        // Pin 1: body end at x−10, pointing left (orientation 2); pin 2 at x+10 pointing right (0).
        recs.push(format!("|RECORD=2|OWNERINDEX={me}|OWNERPARTID=1|LOCATION.X={}|LOCATION.Y=300|PINLENGTH=20|PINCONGLOMERATE=2|NAME=1|DESIGNATOR=1|ELECTRICAL=4", x - 10));
        recs.push(format!("|RECORD=2|OWNERINDEX={me}|OWNERPARTID=1|LOCATION.X={}|LOCATION.Y=300|PINLENGTH=20|PINCONGLOMERATE=0|NAME=2|DESIGNATOR=2|ELECTRICAL=4", x + 10));
        recs.push(format!("|RECORD=34|OWNERINDEX={me}|TEXT={r}"));
        recs.push(format!("|RECORD=41|OWNERINDEX={me}|NAME=Comment|TEXT=1k"));
        recs.push(format!("|RECORD=45|OWNERINDEX={me}|MODELNAME=R0603|MODELTYPE=PCBLIB|ISCURRENT=T"));
        idx += 7;
    };
    part(&mut recs, "R1", 200);
    part(&mut recs, "R2", 300);
    // R1.2 tip at 230, R2.1 tip at 270: a wire, a label on it, GND on R2.2 (tip 330).
    recs.push("|RECORD=27|LOCATIONCOUNT=2|X1=230|Y1=300|X2=270|Y2=300".into());
    recs.push("|RECORD=25|LOCATION.X=250|LOCATION.Y=300|TEXT=MID".into());
    recs.push("|RECORD=17|LOCATION.X=330|LOCATION.Y=300|TEXT=GND|STYLE=4|ORIENTATION=3".into());
    let refs: Vec<&str> = recs.iter().map(String::as_str).collect();
    let cfb = archive::cfb_write(&[("FileHeader", altium::write_props(&refs))]);
    let (p, out) = import_bytes("demo.SchDoc", &cfb);
    assert_eq!(out["format"], "altium_sch");
    check_schematic(&p, &out, "altium");
}

#[test]
fn cadstar_archive() {
    let u = |v: f64| format!("{}", (v * 1e5).round() as i64);
    let pt = |x: f64, y: f64| format!("(PT {} {})", u(x), u(y));
    let t = format!(
        r#"(CADSTARPCB
 (HEADER (FORMAT LAYOUT 21 0) (JOBFILE "demo.pcb") (JOBTITLE "Demo") (GENERATOR "test") (RESOLUTION HUNDREDTH MICRON))
 (ASSIGNMENTS
  (LAYERDEFS
   (LAYER L1 "Top" (ELEC (PHYSICAL 1)))
   (LAYER L2 "Bottom" (ELEC (PHYSICAL 2)))
   (LAYER L3 "Top silk" (NONELEC (LAYERSUBTYPE_SILKSCREEN))))
  (CODEDEFS
   (PADCODE PC1 "smd" (RECTANGLE {} {}))
   (PADCODE PC2 "tht" (CIRCLE {}) (DRILLDIAMETER {}))
   (VIACODE VC1 "via" (CIRCLE {}) (DRILLDIAMETER {}))))
 (LIBRARY
  (SYMBOLDEF FP1 "R0603" "" (ORIGIN 0 0)
   (PAD 1 PC1 {} (SIDE TOP))
   (PAD 2 PC1 {} (SIDE TOP)))
  (SYMBOLDEF FP2 "PIN2" "" (ORIGIN 0 0)
   (PAD 1 PC2 {} (SIDE THROUGH_HOLE))
   (PAD 2 PC2 {} (SIDE THROUGH_HOLE))))
 (PARTS (PART PA1 "10k") (PART PA2 "HDR"))
 (LAYOUT
  (BOARD BD1 L1 (OUTLINE {} {} {} {}))
  (COMPONENT CO1 "R1" FP1 PA1 {} (ORIENT 90000))
  (COMPONENT CO2 "R2" FP1 PA1 {} (ORIENT 0) (MIRROR))
  (COMPONENT CO3 "J1" FP2 PA2 {} (ORIENT 0))
  (NET NET1 (SIGNAME "VCC") (PIN PN1 CO1 1) (PIN PN2 CO3 1) (CONNECTION PN2 PN1 (ROUTE L1 (ROUTEWIDTH {}) {} {})))
  (NET NET2 (SIGNAME "GND") (PIN PN3 CO1 2) (PIN PN4 CO2 1) (PIN PN5 CO3 2) (VIA VI1 VC1 {}))
  (NET NET3 (SIGNAME "SIG") (PIN PN6 CO2 2))))"#,
        u(0.9),
        u(1.0),
        u(1.7),
        u(1.0),
        u(0.6),
        u(0.3),
        pt(-0.8, 0.0),
        pt(0.8, 0.0),
        pt(0.0, 0.0),
        pt(0.0, 2.54),
        pt(0.0, 0.0),
        pt(30.0, 0.0),
        pt(30.0, 20.0),
        pt(0.0, 20.0),
        pt(10.0, 10.0),
        pt(20.0, 10.0),
        pt(5.0, 5.0),
        u(0.25),
        pt(5.0, 5.0),
        pt(10.0, 9.2),
        pt(15.0, 15.0)
    );
    let (p, out) = import_text("demo.cpa", &t);
    assert_eq!(out["format"], "cadstar_pcb");
    check_board(&p, &out, true, "cadstar");
}

#[test]
fn fabmaster_extract() {
    let t = "J!demo.brd!Mon Oct 12 2026!MILLIMETERS!\n\
A!REFDES!COMP_CLASS!COMP_PART_NUMBER!COMP_HEIGHT!COMP_DEVICE_LABEL!COMP_INSERTION_CODE!SYM_TYPE!SYM_NAME!SYM_MIRROR!SYM_ROTATE!SYM_X!SYM_Y!COMP_VALUE!COMP_TOL!COMP_VOLTAGE!\n\
S!R1!DISCRETE!RC0603!!!!PACKAGE!R0603!NO!90!10!10!10k!!!\n\
S!R2!DISCRETE!RC0603!!!!PACKAGE!R0603!YES!0!20!10!1k!!!\n\
S!J1!IO!HDR2!!!!PACKAGE!PIN2!NO!0!5!5!HDR!!!\n\
A!NET_NAME!REFDES!PIN_NUMBER!PIN_NAME!PIN_GROUND!PIN_POWER!\n\
S!VCC!R1!1!!!!\nS!GND!R1!2!!!!\nS!GND!R2!1!!!!\nS!SIG!R2!2!!!!\nS!VCC!J1!1!!!!\nS!GND!J1!2!!!!\n\
A!REFDES!PIN_NUMBER!PIN_X!PIN_Y!PAD_STACK_NAME!\n\
S!R1!1!10!9.2!SMD!\nS!R1!2!10!10.8!SMD!\nS!R2!1!20.8!10!SMD!\nS!R2!2!19.2!10!SMD!\nS!J1!1!5!5!THT!\nS!J1!2!5!7.54!THT!\n\
A!PAD_NAME!REC_NUMBER!LAYER!FIXFLAG!VIAFLAG!PADSHAPE1!PADWIDTH!PADHGHT!PADXOFF!PADYOFF!PADFLASH!PADSHAPENAME!\n\
S!SMD!1!TOP!!!RECTANGLE!0.9!1!0!0!!!\nS!THT!1!TOP!!!CIRCLE!1.7!1.7!0!0!!!\nS!THT!2!~DRILL!!!CIRCLE!1!1!0!0!!!\nS!VIA06!1!TOP!!!CIRCLE!0.6!0.6!0!0!!!\nS!VIA06!2!~DRILL!!!CIRCLE!0.3!0.3!0!0!!!\n\
A!VIA_X!VIA_Y!PAD_STACK_NAME!NET_NAME!TEST_POINT!\n\
S!15!15!VIA06!GND!!\n\
A!CLASS!SUBCLASS!GRAPHIC_DATA_NAME!GRAPHIC_DATA_NUMBER!RECORD_TAG!GRAPHIC_DATA_1!GRAPHIC_DATA_2!GRAPHIC_DATA_3!GRAPHIC_DATA_4!GRAPHIC_DATA_5!GRAPHIC_DATA_6!GRAPHIC_DATA_7!GRAPHIC_DATA_8!GRAPHIC_DATA_9!GRAPHIC_DATA_10!NET_NAME!\n\
S!BOARD GEOMETRY!OUTLINE!LINE!1!1 1 0!0!0!30!0!0!!!!!!!\n\
S!BOARD GEOMETRY!OUTLINE!LINE!2!1 2 0!30!0!30!20!0!!!!!!!\n\
S!BOARD GEOMETRY!OUTLINE!LINE!3!1 3 0!30!20!0!20!0!!!!!!!\n\
S!BOARD GEOMETRY!OUTLINE!LINE!4!1 4 0!0!20!0!0!0!!!!!!!\n\
S!ETCH!TOP!LINE!5!2 1 0!5!5!10!9.2!0.25!!!!!!VCC!\n";
    let (p, out) = import_text("demo.fab", t);
    assert_eq!(out["format"], "fabmaster");
    check_board(&p, &out, true, "fabmaster");
}

#[test]
fn pads_ascii() {
    let t = "!PADS-POWERPCB-V9.0-METRIC! DESIGN DATABASE ASCII FILE 1.0\n\
*PCB*        GENERAL PARAMETERS OF THE PCB DESIGN\nLAYERS 2\n\
*LINES*      LINES ITEMS\nBOARD BOARD 0 0 1 0\nCLOSED 5 0.1 0\n0 0\n30 0\n30 20\n0 20\n0 0\n\
*PARTDECAL*  PART DECAL ITEMS\n\
R0603 M 0 0 1 2 1 0 0\nOPEN 2 0.12 26\n-0.2 0.6\n0.2 0.6\nT-0.8 0 -0.8 0 1\nT0.8 0 0.8 0 2\nPAD 0 1\n-2 1 RF 0 0.9 0\n\
PIN2 M 0 0 0 2 1 0 0\nT0 0 0 0 1\nT0 2.54 0 2.54 2\nPAD 0 3 P 1\n-2 1.7 R\n-1 1.7 R\n0 1.7 R\n\
*PART*       ITEMS\nR1 RES@R0603 10 10 90 U N 0 -1 0 -1 0\nR2 RES@R0603 20 10 0 U M 0 -1 0 -1 0\nJ1 HDR@PIN2 5 5 0 U N 0 -1 0 -1 0\n\
*VIA*        ITEMS\nSTDVIA 0.3 2\n-2 0.6 R\n0 0.6 R\n\
*NET*        NETLIST DATA\n*SIGNAL* VCC\nR1.1 J1.1\n*SIGNAL* GND\nR1.2 R2.1 J1.2\n*SIGNAL* SIG\nR2.2\n\
*ROUTE*\n*SIGNAL* VCC\nJ1.1 R1.1\n5 5 1 0.25 0\n10 9.2 1 0.25 0\n*SIGNAL* GND\nR1.2 J1.2\n15 15 1 0 0 STDVIA\n\
*END*     OF ASCII OUTPUT FILE\n";
    let (p, out) = import_text("demo.asc", t);
    assert_eq!(out["format"], "pads");
    check_board(&p, &out, true, "pads");
}

#[test]
fn geda_pcb() {
    // [ ] numbers are 1/100 mil; y down.
    let c = |v: f64| format!("{}", (mil(v) * 100.0).round() as i64);
    let y = |v: f64| c(20.0 - v);
    let pad = |x: f64, yy: f64, w: f64, h: f64, n: &str, flags: &str| {
        // A pad is a line along its longer side, as thick as the shorter.
        let (t, half, along_x) = if w >= h { (h, (w - h) / 2.0, true) } else { (w, (h - w) / 2.0, false) };
        let (dx, dy) = if along_x { (half, 0.0) } else { (0.0, half) };
        format!("\tPad[{} {} {} {} {} 2000 3000 \"{n}\" \"{n}\" \"{flags}\"]\n", c(x - dx), c(-(yy - dy)), c(x + dx), c(-(yy + dy)), c(t))
    };
    let mut s = String::from("PCB[\"demo\" ");
    s.push_str(&format!("{} {}]\nGroups(\"1,c:2,s:3\")\n", c(30.0), c(20.0)));
    // R1 at (10, 10) turned: pads stand along y.
    s.push_str(&format!("Element[\"\" \"R0603\" \"R1\" \"10k\" {} {} 0 0 0 100 \"\"]\n(\n", c(10.0), y(10.0)));
    s.push_str(&pad(0.0, -0.8, 1.0, 0.9, "1", "square"));
    s.push_str(&pad(0.0, 0.8, 1.0, 0.9, "2", "square"));
    s.push_str(")\n");
    s.push_str(&format!("Element[\"onsolder\" \"R0603\" \"R2\" \"1k\" {} {} 0 0 0 100 \"\"]\n(\n", c(20.0), y(10.0)));
    s.push_str(&pad(0.8, 0.0, 0.9, 1.0, "1", "square,onsolder"));
    s.push_str(&pad(-0.8, 0.0, 0.9, 1.0, "2", "square,onsolder"));
    s.push_str(")\n");
    s.push_str(&format!("Element[\"\" \"PIN2\" \"J1\" \"HDR\" {} {} 0 0 0 100 \"\"]\n(\n", c(5.0), y(5.0)));
    s.push_str(&format!("\tPin[0 0 {} 2000 3000 {} \"1\" \"1\" \"square\"]\n", c(1.7), c(1.0)));
    s.push_str(&format!("\tPin[0 {} {} 2000 3000 {} \"2\" \"2\" \"\"]\n)\n", c(-2.54), c(1.7), c(1.0)));
    s.push_str(&format!("Via[{} {} {} 2000 0 {} \"\" \"\"]\n", c(15.0), y(15.0), c(0.6), c(0.3)));
    s.push_str(&format!("Layer(1 \"top\")\n(\n\tLine[{} {} {} {} {} 2000 \"clearline\"]\n)\n", c(5.0), y(5.0), c(10.0), y(9.2), c(0.25)));
    s.push_str("Layer(2 \"bottom\")\n(\n)\n");
    s.push_str(&format!("Layer(3 \"outline\")\n(\n\tLine[0 0 {w} 0 1000 2000 \"\"]\n\tLine[{w} 0 {w} {h} 1000 2000 \"\"]\n\tLine[{w} {h} 0 {h} 1000 2000 \"\"]\n\tLine[0 {h} 0 0 1000 2000 \"\"]\n)\n", w = c(30.0), h = c(20.0)));
    s.push_str("NetList()\n(\n\tNet(\"VCC\" \"(unknown)\")\n\t(\n\t\tConnect(\"R1-1\")\n\t\tConnect(\"J1-1\")\n\t)\n\tNet(\"GND\" \"(unknown)\")\n\t(\n\t\tConnect(\"R1-2\")\n\t\tConnect(\"R2-1\")\n\t\tConnect(\"J1-2\")\n\t)\n\tNet(\"SIG\" \"(unknown)\")\n\t(\n\t\tConnect(\"R2-2\")\n\t)\n)\n");
    let (p, out) = import_text("demo.pcb", &s);
    assert_eq!(out["format"], "geda_pcb");
    check_board(&p, &out, false, "geda");
}
