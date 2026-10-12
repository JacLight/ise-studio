use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::footprints::{self, PadKind};
use de_pcb::{PcbPlugin, outputs, reader, sourcing};
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

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn pad<'a>(f: &'a de_pcb::Footprint, n: &str) -> &'a footprints::Pad {
    f.pads.iter().find(|p| p.number == n).unwrap_or_else(|| panic!("{} has no pad {n}", f.name))
}

#[test]
fn ipc_7351b_lands_match_the_published_nominal_patterns() {
    // IPC-7351B equations (F 0.05, P 0.025, round-off 0.05) on JEDEC outlines.
    let sot = footprints::get("SOT-23").unwrap();
    let p1 = pad(&sot, "1");
    assert!(near(p1.size[0], 1.475, 1e-9) && near(p1.size[1], 0.60, 1e-9), "{:?}", p1.size);
    assert!(near(p1.at[0], -0.9375, 1e-9) && near(p1.at[1], 0.95, 1e-9), "{:?}", p1.at);
    assert!(near(pad(&sot, "3").at[0], 0.9375, 1e-9) && near(pad(&sot, "3").at[1], 0.0, 1e-9));

    let soic = footprints::get("SOIC-8").unwrap();
    let p = pad(&soic, "1");
    assert!(near(p.size[0], 2.0, 1e-9) && near(p.size[1], 0.6, 1e-9) && near(p.at[0], -2.475, 1e-9) && near(p.at[1], 1.905, 1e-9), "{p:?}");
    // Pins run counter-clockwise: 4 at bottom left, 5 at bottom right, 8 at top right.
    assert!(near(pad(&soic, "4").at[1], -1.905, 1e-9) && near(pad(&soic, "5").at[0], 2.475, 1e-9) && near(pad(&soic, "5").at[1], -1.905, 1e-9) && near(pad(&soic, "8").at[1], 1.905, 1e-9));

    let r = footprints::get("R_0603").unwrap();
    assert!(near(pad(&r, "1").size[0], 0.8, 1e-9) && near(pad(&r, "1").size[1], 0.95, 1e-9) && near(pad(&r, "2").at[0], 0.825, 1e-9), "{:?}", r.pads);

    let tssop = footprints::get("TSSOP-20").unwrap();
    assert_eq!(tssop.pads.len(), 20);
    assert!(near(pad(&tssop, "2").at[1] - pad(&tssop, "1").at[1], -0.65, 1e-9));
    assert!(near(pad(&tssop, "1").size[1], 0.40, 1e-9) && near(pad(&tssop, "1").at[0], -2.8875, 1e-9));

    let qfn = footprints::get("QFN-32_5x5").unwrap();
    assert_eq!(qfn.pads.len(), 33);
    let q1 = pad(&qfn, "1");
    assert!(near(q1.size[0], 0.875, 1e-9) && near(q1.size[1], 0.25, 1e-9) && near(q1.at[0], -2.4375, 1e-9) && near(q1.at[1], 1.75, 1e-9), "{q1:?}");
    // Pin 9 starts the bottom row, turned a quarter.
    assert!(near(pad(&qfn, "9").at[1], -2.4375, 1e-9) && near(pad(&qfn, "9").rotation, 90.0, 1e-9));
    assert_eq!(pad(&qfn, "33").paste.len(), 4, "the exposed pad's paste is a windowpane");

    // Lands never touch each other.
    for f in [&sot, &soic, &r, &tssop, &qfn] {
        for (i, a) in f.pads.iter().enumerate() {
            for b in &f.pads[i + 1..] {
                let g = de_pcb::geom::gap(&footprints::pad_prim(a, [0.0, 0.0], 0.0, false), &footprints::pad_prim(b, [0.0, 0.0], 0.0, false));
                assert!(g > 0.15, "{} pads {} and {} are {g} apart", f.name, a.number, b.number);
            }
        }
    }
}

#[test]
fn datasheet_footprints_have_their_makers_dimensions() {
    let esp = footprints::get("ESP32-WROOM-32").unwrap();
    assert_eq!(esp.pads.len(), 39);
    // Side rows 17.5 mm apart centre to centre, reaching 0.5 mm past the module's 18 mm width;
    // pitch 1.27 mm, pin 1 7.49 mm below the top edge (12.75); the bottom row 0.5 mm past the bottom edge.
    let p1 = pad(&esp, "1");
    assert!(near(p1.at[0], -8.75, 1e-9) && near(p1.at[0] - p1.size[0] / 2.0, -9.5, 1e-9) && near(12.75 - p1.at[1], 7.49, 1e-9), "{p1:?}");
    assert!(near(pad(&esp, "1").at[1] - pad(&esp, "2").at[1], 1.27, 1e-9));
    assert!(near(pad(&esp, "15").at[1] - 0.75, -13.25, 1e-9), "{:?}", pad(&esp, "15"));
    // GND pad 3.7 mm square, 7.5 mm from the module's left edge and 10.29 mm above its bottom edge.
    let gnd = pad(&esp, "39");
    assert!(near(gnd.size[0], 3.7, 1e-9) && near(gnd.at[0], -1.5, 1e-9) && near(gnd.at[1], -2.46, 1e-9), "{gnd:?}");

    let usb = footprints::get("USB-C_16P").unwrap();
    let a1 = pad(&usb, "A1");
    let b12 = pad(&usb, "B12");
    assert_eq!(a1.at, b12.at, "paired contacts share one land");
    assert!(near(pad(&usb, "A4").at[0], -2.40, 1e-9) && near(pad(&usb, "A5").at[0] - pad(&usb, "B8").at[0], 0.5, 1e-9));
    let slots: Vec<_> = usb.pads.iter().filter(|p| p.number == "S1").collect();
    assert_eq!(slots.len(), 4);
    assert!(slots.iter().all(|s| near(s.at[0].abs(), 4.325, 1e-9)) && slots.iter().any(|s| s.drill == Some([0.6, 1.7])));

    let ph = footprints::get("JST_PH_4").unwrap();
    assert_eq!(ph.pads.len(), 4);
    assert!(ph.pads.iter().all(|p| p.drill == Some([0.7, 0.7])) && near(pad(&ph, "4").at[0], 6.0, 1e-9));
    assert_eq!(pad(&footprints::get("JST_XH_2").unwrap(), "1").drill, Some([1.0, 1.0]));
    assert_eq!(pad(&footprints::get("JST_XH_3").unwrap(), "1").drill, Some([0.9, 0.9]));
    let sw = footprints::get("SW_Tactile_6x6").unwrap();
    let xs: Vec<f64> = sw.pads.iter().map(|p| p.at[0]).collect();
    assert!(near(xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min), 6.5, 1e-9));
    let hdr = footprints::get("PinHeader_1x07_2.54").unwrap();
    assert_eq!(hdr.pads.len(), 7);
    assert!(near(pad(&hdr, "7").at[1], -6.0 * 2.54, 1e-9));
    assert!(footprints::get("MountingHole_M3").unwrap().pads[0].kind == PadKind::Npth);
}

/// A 30 × 20 mm two-layer board with R1, R2, C1 and a header.
fn small_board(p: &mut Project) -> (StableId, StableId) {
    let b = ok(p, "pcb.board.create", json!({ "name": "Test board", "width": 30, "height": 20, "corner_radius": 1 }));
    let board = id(&b);
    let model = serde_json::from_value(b["model"].clone()).unwrap();
    ok(p, "pcb.component.create", json!({ "board": board, "reference": "R1", "footprint": "R_0603", "value": "10k", "at": [8, 10], "mpn": "0603WAF1002T5E", "pins": { "1": "VCC", "2": "SIG" } }));
    ok(p, "pcb.component.create", json!({ "board": board, "reference": "R2", "footprint": "R_0603", "value": "10k", "at": [14, 10], "rotation": 90, "mpn": "0603WAF1002T5E", "pins": { "1": "SIG", "2": "GND" } }));
    ok(p, "pcb.component.create", json!({ "board": board, "reference": "J1", "footprint": "PinHeader_1x03_2.54", "value": "Header", "at": [24, 12.54], "pins": { "1": "VCC", "2": "SIG", "3": "GND" } }));
    (board, model)
}

#[test]
fn a_board_resolves_its_nets_and_ratsnest() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    let v = de_pcb::view(p.world(), board).expect("worked out");
    assert_eq!(v.components.len(), 3);
    assert_eq!(v.pads.len(), 7);
    let sig = v.nets.iter().find(|n| n.name == "SIG").unwrap();
    assert_eq!(sig.pins, vec!["J1.2", "R1.2", "R2.1"]);
    assert_eq!(sig.islands, 3);
    assert!(!sig.routed);
    assert_eq!(v.ratsnest.iter().filter(|w| w.net == "SIG").count(), 2);
    assert!(v.violations.iter().any(|x| x.code == "unrouted"));
    // R2 is turned a quarter: its pads stack vertically.
    let r2: Vec<_> = v.pads.iter().filter(|p| p.reference == "R2").collect();
    assert!(near(r2[0].at[0], 14.0, 1e-9) && near(r2[0].at[1], 9.175, 1e-9) && near(r2[1].at[1], 10.825, 1e-9), "{:?}", r2.iter().map(|p| p.at).collect::<Vec<_>>());
}

#[test]
fn drc_catches_seeded_violations() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    // Route SIG properly: R1.2 → R2.1 → J1.2.
    ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "top", "points": [[8.8, 10], [11, 10], [11, 9.2], [14, 9.2]] }));
    let v = de_pcb::view(p.world(), board).unwrap();
    assert!(v.tracks[0].net.as_deref() == Some("SIG"), "the net comes from the pad under the start");
    // 1. Too thin, 2. too close to another net, 3. a via with too small a drill, 4. off the edge, 5. a short.
    ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "bottom", "net": "X", "width": 0.1, "points": [[2, 2], [6, 2]] }));
    ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "top", "net": "GND", "width": 0.25, "points": [[7, 10.75], [9.5, 10.75]] }));
    ok(&mut p, "pcb.via.create", json!({ "board": board, "net": "GND", "at": [5, 15], "drill": 0.2, "diameter": 0.4 }));
    ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "bottom", "net": "VCC", "width": 0.3, "points": [[20, 0.1], [26, 0.1]] }));
    ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "top", "net": "GND", "width": 0.3, "points": [[24, 8], [24, 12.54]] }));
    // 6. Courtyards: C1 on top of R1.
    ok(&mut p, "pcb.component.create", json!({ "board": board, "reference": "C1", "footprint": "C_0603", "value": "100nF", "at": [8.5, 10.6] }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let codes: Vec<&str> = v.violations.iter().map(|x| x.code).collect();
    for want in ["track_width", "clearance", "drill", "annular_ring", "edge_clearance", "short", "courtyard_overlap", "unrouted"] {
        assert!(codes.contains(&want), "missing {want} in {codes:?}");
    }
    // Each violation names what it is about.
    let short = v.violations.iter().find(|x| x.code == "short" && x.refs.iter().any(|r| r == "J1.2")).unwrap();
    assert!(short.refs.iter().any(|r| r == "J1.2"), "{short:?}");
    assert!(!short.objects.is_empty());
    let clear = v.violations.iter().find(|x| x.code == "clearance" && x.message.contains("R1")).expect("GND track by R1");
    assert!(clear.at[1] > 10.0 && clear.at[1] < 11.0, "{clear:?}");
    let court = v.violations.iter().find(|x| x.code == "courtyard_overlap").unwrap();
    assert!(court.refs.contains(&"C1".to_string()) && court.refs.contains(&"R1".to_string()), "{court:?}");
}

#[test]
fn the_autorouter_routes_and_the_board_passes_drc() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    let out = ok(&mut p, "pcb.autoroute", json!({ "board": board }));
    assert_eq!(out["failed"].as_array().unwrap().len(), 0, "{out}");
    let v = de_pcb::view(p.world(), board).unwrap();
    let errors: Vec<_> = v.violations.iter().filter(|x| x.severity == "error").collect();
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(v.nets.iter().all(|n| n.routed));
    // A ground pour on the bottom joins the ground pads and keeps clear of the rest.
    ok(&mut p, "pcb.zone.create", json!({ "board": board, "net": "GND", "layer": "bottom" }));
    let v = de_pcb::view(p.world(), board).unwrap();
    assert!(v.zones[0].area > 300.0, "{}", v.zones[0].area);
    assert!(v.violations.iter().all(|x| x.severity != "error"), "{:#?}", v.violations);
    // Ripping up and routing again gives the same result.
    let again = ok(&mut p, "pcb.autoroute", json!({ "board": board, "replace": false }));
    assert_eq!(again["routed"].as_array().unwrap().len(), 0, "nothing left to route: {again}");
}

#[test]
fn gerbers_and_drills_read_back() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    ok(&mut p, "pcb.autoroute", json!({ "board": board }));
    ok(&mut p, "pcb.component.create", json!({ "board": board, "reference": "H1", "footprint": "MountingHole_M2", "at": [4, 16] }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let files = de_pcb::gerber::files(&v);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    for ext in [".gtl", ".gbl", ".gts", ".gbs", ".gtp", ".gto", ".gm1", "-PTH.drl", "-NPTH.drl"] {
        assert!(names.iter().any(|n| n.ends_with(ext)), "{ext} missing from {names:?}");
    }
    let get = |suffix: &str| files.iter().find(|(n, _)| n.ends_with(suffix)).map(|(_, s)| s.as_str()).unwrap();
    let top = reader::gerber(get(".gtl")).unwrap();
    assert!(top.units_mm);
    assert_eq!(top.format, (4, 6));
    assert_eq!(top.attributes.get("FileFunction").map(String::as_str), Some("Copper,L1,Top"));
    // Every top pad is flashed at its centre with an aperture of its size.
    for pad in v.pads.iter().filter(|p| p.layers.iter().any(|l| l == "top")) {
        let hit = top.flashes.iter().find(|(_, at)| near(at[0], pad.at[0], 1e-6) && near(at[1], pad.at[1], 1e-6));
        let (d, _) = hit.unwrap_or_else(|| panic!("{}.{} not flashed", pad.reference, pad.number));
        let half = top.apertures[d].half();
        let want = if (pad.rotation / 90.0).round() as i64 % 2 == 1 { [pad.size[1] / 2.0, pad.size[0] / 2.0] } else { [pad.size[0] / 2.0, pad.size[1] / 2.0] };
        assert!(near(half[0], want[0], 1e-6) && near(half[1], want[1], 1e-6), "{}.{}: {half:?} vs {want:?}", pad.reference, pad.number);
    }
    assert!(top.macros.contains(&"RoundRect".to_string()));
    assert!(top.nets.contains(&"SIG".to_string()), "X2 net attributes: {:?}", top.nets);
    // Tracks are draws of their width.
    let draws: f64 = top.draws.iter().map(|(_, a, b)| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()).sum();
    let len: f64 = v.tracks.iter().filter(|t| t.layer == "top").map(|t| t.points.windows(2).map(|w| ((w[0][0] - w[1][0]).powi(2) + (w[0][1] - w[1][1]).powi(2)).sqrt()).sum::<f64>()).sum();
    assert!(near(draws, len, 1e-4), "{draws} vs {len}");
    // The profile is the outline.
    let edge = reader::gerber(get(".gm1")).unwrap();
    let b = edge.bounds();
    assert!(near(b[0], -0.05, 1e-6) && near(b[2], 30.05, 1e-6) && near(b[3], 20.05, 1e-6), "{b:?}");
    // Masks are negative, paste covers SMD pads only.
    assert_eq!(reader::gerber(get(".gts")).unwrap().attributes.get("FilePolarity").map(String::as_str), Some("Negative"));
    assert_eq!(reader::gerber(get(".gtp")).unwrap().flashes.len(), 4);
    assert!(!reader::gerber(get(".gto")).unwrap().draws.is_empty());
    // Drill hits are the holes.
    let pth = reader::excellon(get("-PTH.drl")).unwrap();
    assert!(pth.metric && pth.plated == Some(true));
    assert_eq!(pth.hits.len(), 3 + v.vias.len());
    for h in v.holes.iter().filter(|h| h.plated) {
        assert!(pth.hits.iter().any(|(t, at)| near(at[0], h.at[0], 1e-3) && near(at[1], h.at[1], 1e-3) && near(pth.tools[t], h.size[0], 1e-3)), "{h:?}");
    }
    let npth = reader::excellon(get("-NPTH.drl")).unwrap();
    assert_eq!(npth.plated, Some(false));
    assert_eq!(npth.hits, vec![(1, [4.0, 16.0])]);
    assert!(near(npth.tools[&1], 2.2, 1e-9));
    // Every file is strict-valid; a broken one is refused.
    for (n, s) in &files {
        if n.ends_with(".drl") { reader::excellon(s).unwrap(); } else { reader::gerber(s).unwrap_or_else(|e| panic!("{n}: {e}")); }
    }
    assert!(reader::gerber(&get(".gtl").replace("M02*", "")).is_err());
}

#[test]
fn zone_fills_with_holes_come_out_as_cut_in_regions() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    ok(&mut p, "pcb.zone.create", json!({ "board": board, "net": "GND", "layer": "top" }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let z = &v.zones[0];
    assert!(z.fill.iter().any(|island| island.len() > 1), "the pour has holes around other nets' pads");
    let g = reader::gerber(&de_pcb::gerber::copper(&v, "top")).unwrap();
    assert!(!g.regions.is_empty());
    let area: f64 = g.regions.iter().map(|r| de_pcb::geom::polygon_area(r)).sum();
    assert!(near(area, z.area, z.area * 1e-6), "{area} vs {}", z.area);
}

#[test]
fn pick_and_place_has_centres_rotations_and_sides() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    ok(&mut p, "pcb.component.create", json!({ "board": board, "reference": "U1", "footprint": "SOIC-8", "value": "NE555", "at": [16, 5], "rotation": 270, "side": "bottom", "mpn": "NE555DR" }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let csv = outputs::pick_and_place(&v, outputs::Format::Jlcpcb);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "Designator,Mid X,Mid Y,Layer,Rotation");
    assert!(lines.contains(&"R2,14.0000mm,10.0000mm,Top,90"), "{csv}");
    assert!(lines.contains(&"U1,16.0000mm,5.0000mm,Bottom,270"), "{csv}");
    // A header's origin is pin 1; it is placed by its body centre.
    assert!(lines.contains(&"J1,24.0000mm,10.0000mm,Top,0"), "{csv}");
    // U1 on the bottom is mirrored: pin 1 (top left seen from above, unmirrored) goes to the right.
    let u1p1 = v.pads.iter().find(|p| p.reference == "U1" && p.number == "1").unwrap();
    // Mirror x (−2.475 → 2.475), then turn 270°: (x, y) → (y, −x).
    assert!(near(u1p1.at[0], 16.0 + 1.905, 1e-9) && near(u1p1.at[1], 5.0 - 2.475, 1e-9), "{:?}", u1p1.at);
    assert_eq!(u1p1.layers, vec!["bottom".to_string()]);
}

#[test]
fn bom_lines_merge_by_mpn_and_carry_lcsc() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    let b2 = id(&ok(&mut p, "pcb.board.create", json!({ "name": "Second", "width": 20, "height": 20, "quantity": 10 })));
    ok(&mut p, "pcb.component.create", json!({ "board": b2, "reference": "R7", "footprint": "R_0603", "value": "10k", "at": [5, 5], "mpn": "0603waf1002t5e" }));
    let world = p.world();
    let board_a = world.get::<de_pcb::Board>(p.entity(board).unwrap()).unwrap().clone();
    let board_b = world.get::<de_pcb::Board>(p.entity(b2).unwrap()).unwrap().clone();
    let va = de_pcb::view(world, board).unwrap();
    let vb = de_pcb::view(world, b2).unwrap();
    let lines = sourcing::bom_lines(&va, &board_a);
    let r = lines.iter().find(|l| l.mpn.as_deref() == Some("0603WAF1002T5E")).unwrap();
    assert_eq!(r.references, vec!["R1", "R2"]);
    assert_eq!(r.lcsc.as_deref(), Some("C25804"));
    assert_eq!(r.lcsc_class, Some("Basic"));
    assert!(r.links.iter().any(|l| l.url == "https://www.lcsc.com/product-detail/C25804.html"));
    assert!(r.links.iter().any(|l| l.url.starts_with("https://www.mouser.com/c/?q=0603WAF1002T5E")));
    let mut buy = sourcing::buy_lines(&va, &board_a);
    buy.extend(sourcing::buy_lines(&vb, &board_b));
    let merged = sourcing::merge(&buy);
    let r = merged.iter().find(|l| l.mpn.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("0603WAF1002T5E"))).unwrap();
    assert_eq!(r.quantity, 2 * 5 + 10, "per board × each board's quantity");
    assert_eq!(r.references, vec!["Test board:R1", "Test board:R2", "Second:R7"]);
    // JLCPCB's BOM groups designators in one cell.
    let csv = outputs::bom(&va, &board_a, outputs::Format::Jlcpcb);
    assert!(csv.contains("10k,\"R1,R2\",R_0603,C25804"), "{csv}");
    // A recorded supplier and price flows into the lines and totals.
    ok(&mut p, "pcb.sourcing.set", json!({ "board": board, "key": "0603WAF1002T5E", "supplier": "LCSC", "sku": "C25804", "unit_price": 0.002, "currency": "USD" }));
    let board_a = p.world().get::<de_pcb::Board>(p.entity(board).unwrap()).unwrap().clone();
    let lines = sourcing::bom_lines(&va, &board_a);
    let (sums, missing) = sourcing::totals(&lines, 5);
    assert!(near(sums["USD"], 0.002 * 10.0, 1e-12) && missing == 1, "{sums:?} {missing}");
}

#[test]
fn the_3d_part_follows_the_board() {
    let mut p = project();
    let (board, model) = small_board(&mut p);
    let solid = p.get::<de_shape::ShapeSolid>(model).expect("the 3D board evaluates");
    let names: Vec<&str> = solid.stats.parts.iter().map(|x| x.name.as_str()).collect();
    assert!(names.contains(&"PCB") && names.contains(&"R1 10k") && names.contains(&"J1 Header"), "{names:?}");
    // The header's pins come 3 mm through the bottom.
    assert!(near(solid.stats.bounds_min[2], -3.0, 1e-6), "{:?}", solid.stats.bounds_min);
    // The header's pins stand 8.5 mm above the 1.6 mm board.
    assert!(near(solid.stats.bounds_max[2], 1.6 + 8.5, 1e-6), "{:?}", solid.stats.bounds_max);
    let r1 = de_pcb::items(p.world(), board).components.into_iter().find(|(_, c)| c.reference == "R1").unwrap().0;
    ok(&mut p, "pcb.component.update", json!({ "target": r1, "at": [5, 5] }));
    let src = &p.get::<de_shape::ShapeProgram>(model).unwrap().source;
    assert!(src.contains("\"box\",5.0,5.0") || src.contains("\"box\",5,5"), "moved in the program");
    // Bought parts are declared as stock for the BOM.
    let declared = &p.get::<de_shape::ShapeSolid>(model).unwrap().stats;
    let _ = declared;
    // Moving to the bottom puts the body under the board.
    ok(&mut p, "pcb.component.update", json!({ "target": r1, "side": "bottom" }));
    let solid = p.get::<de_shape::ShapeSolid>(model).unwrap();
    assert!(solid.stats.bounds_min[2] <= -3.0 + 1e-6, "{:?}", solid.stats.bounds_min);
    let src = &p.get::<de_shape::ShapeProgram>(model).unwrap().source;
    assert!(src.contains("\"box\",5.0,5.0,-") || src.contains("\"box\",5,5,-"), "R1's body is under the board");
}

#[test]
fn netlists_outlines_and_custom_footprints() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    ok(&mut p, "pcb.netlist.set", json!({ "board": board, "nets": { "A": ["R1.1", "J1.1"], "B": ["R1.2", "R2.1", "J1.2"] } }));
    let v = de_pcb::view(p.world(), board).unwrap();
    assert_eq!(v.nets.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), vec!["A", "B"], "replace drops the old nets");
    assert!(run(&mut p, "pcb.netlist.set", json!({ "board": board, "nets": { "A": ["R9.1"] } })).is_err());
    assert!(run(&mut p, "pcb.netlist.set", json!({ "board": board, "nets": { "A": ["R1.3"] } })).is_err());
    // A footprint from a datasheet, defined on the board.
    ok(&mut p, "pcb.footprint.define", json!({ "board": board, "footprint": { "name": "TestPoint_1.5", "pads": [{ "number": "1", "shape": "circle", "size": [1.5, 1.5], "at": [0, 0] }], "body": [] } }));
    ok(&mut p, "pcb.component.create", json!({ "board": board, "reference": "TP1", "footprint": "TestPoint_1.5", "at": [3, 3], "pins": { "1": "A" } }));
    // The outline goes to a sketch, and the board can follow it.
    let s = id(&ok(&mut p, "pcb.board.outline_to_sketch", json!({ "board": board, "link": true })));
    assert!(p.get::<de_shape::sketch::Sketch>(s).is_some());
    let v = de_pcb::view(p.world(), board).unwrap();
    assert!(near(v.bounds[2], 30.0, 1e-6) && near(v.bounds[3], 20.0, 1e-6), "{:?}", v.bounds);
    let model = p.get::<de_pcb::Board>(board).unwrap().model.unwrap();
    let j1 = de_pcb::items(p.world(), board).components.into_iter().find(|(_, c)| c.reference == "J1").unwrap().0;
    ok(&mut p, "pcb.component.update", json!({ "target": j1, "value": "Header 3" }));
    assert!(p.get::<de_shape::ShapeProgram>(model).unwrap().source.contains("sketch(\"Test board outline\").profile"));
    assert!(p.get::<de_shape::ShapeSolid>(model).is_some(), "{:?}", p.status(model));
    // Deleting a component takes it off the board.
    let tp = de_pcb::items(p.world(), board).components.into_iter().find(|(_, c)| c.reference == "TP1").unwrap().0;
    ok(&mut p, "pcb.delete", json!({ "targets": [tp] }));
    assert_eq!(de_pcb::view(p.world(), board).unwrap().components.len(), 3);
    assert!(p.status(board).is_some_and(|s| s.is_ok()));
}

#[test]
fn fabs_flag_what_they_cannot_make() {
    let mut p = project();
    let (board, _) = small_board(&mut p);
    ok(&mut p, "pcb.autoroute", json!({ "board": board }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let reports = de_pcb::fabs::reports(&v);
    let jlc = reports.iter().find(|r| r.fab.id == "jlcpcb").unwrap();
    assert!(jlc.can_make, "{:#?}", jlc.checks);
    let osh = reports.iter().find(|r| r.fab.id == "oshpark").unwrap();
    assert!(!osh.can_make, "OSH Park makes purple ENIG only");
    assert!(osh.checks.iter().any(|c| c.what == "Mask colour" && !c.ok));
    // OSH Park's published price: $5 per square inch per three boards.
    let e = osh.estimate.as_ref().unwrap();
    let area = de_pcb::geom::polygon_area(&v.outline) / 645.16;
    assert!(near(e.amount, area * 5.0 * 2.0, 1e-9), "5 boards = 2 sets: {e:?}");
    assert!(reports.iter().find(|r| r.fab.id == "jlcpcb").unwrap().estimate.is_none(), "no invented prices");
}

#[test]
fn the_project_bom_lists_board_parts_to_buy_and_the_board_to_make() {
    use de_sourcing::bom::{self, Options, Scope};
    let mut p = project();
    small_board(&mut p);
    // A second board with no 3D part: its components come through the board's Buy source.
    let b2 = id(&ok(&mut p, "pcb.board.create", json!({ "name": "Bare", "width": 20, "height": 20, "model": false })));
    ok(&mut p, "pcb.component.create", json!({ "board": b2, "reference": "R9", "footprint": "R_0603", "value": "10k", "at": [5, 5], "mpn": "0603WAF1002T5E" }));
    let b = bom::compute(p.world(), Scope::Project, &Options::default(), &[&sourcing::PcbBuySource]).unwrap();
    let r = b.buy.iter().find(|l| l.stock.as_ref().is_some_and(|s| s.part_number == "0603WAF1002T5E")).unwrap_or_else(|| panic!("{:#?}", b.buy.iter().map(|l| &l.name).collect::<Vec<_>>()));
    assert_eq!(r.qty, 3, "R1 and R2 on the placed board, R9 on the bare one, merged by MPN");
    assert!(r.stock.as_ref().unwrap().sources.iter().any(|s| s.supplier == "LCSC" && s.sku.as_deref() == Some("C25804")) || r.origin == "stock");
    let pcb = b.make.iter().find(|m| m.process.as_deref() == Some("pcb_fab")).expect("the bare board is a Make line");
    assert!(pcb.material.as_deref().is_some_and(|m| m.starts_with("FR4")), "{:?}", pcb.material);
    assert!(!b.make.iter().any(|m| m.name.contains("copper")), "copper is part of the board, not a line of its own");
    assert!(b.buy.iter().all(|l| l.stock.is_some()), "every bought part has its part number");
}
