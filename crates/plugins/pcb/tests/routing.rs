//! Routing edits: track ends that follow a moved or turned part, hand routes
//! that change layer through a via, thermal reliefs on pours, and the
//! autorouter on the worked example.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::PcbPlugin;
use de_pcb::geom::{self, P};
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

fn octilinear(a: P, b: P) -> bool {
    let (dx, dy) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
    dx < 1e-9 || dy < 1e-9 || (dx - dy).abs() < 1e-9
}

/// R1 (VCC/SIG), R2 turned (SIG/GND), J1 header (VCC, SIG, GND).
fn small_board(p: &mut Project) -> (StableId, StableId, StableId, StableId) {
    let board = id(&ok(p, "pcb.board.create", json!({ "name": "Test board", "width": 30, "height": 20, "corner_radius": 1 })));
    let r1 = id(&ok(p, "pcb.component.create", json!({ "board": board, "reference": "R1", "footprint": "R_0603", "value": "10k", "at": [8, 10], "pins": { "1": "VCC", "2": "SIG" } })));
    let r2 = id(&ok(p, "pcb.component.create", json!({ "board": board, "reference": "R2", "footprint": "R_0603", "value": "10k", "at": [14, 10], "rotation": 90, "pins": { "1": "SIG", "2": "GND" } })));
    let j1 = id(&ok(p, "pcb.component.create", json!({ "board": board, "reference": "J1", "footprint": "PinHeader_1x03_2.54", "value": "Header", "at": [24, 12.54], "pins": { "1": "VCC", "2": "SIG", "3": "GND" } })));
    (board, r1, r2, j1)
}

fn pad_at(v: &de_pcb::BoardView, r: &str, n: &str) -> P {
    v.pads.iter().find(|p| p.reference == r && p.number == n).unwrap().at
}

#[test]
fn moving_or_turning_a_part_drags_its_track_ends() {
    let mut p = project();
    let (board, r1, r2, _) = small_board(&mut p);
    // SIG: R1.2 → R2.1, leaving R1 along +x.
    let v = de_pcb::view(p.world(), board).unwrap();
    let (a, b) = (pad_at(&v, "R1", "2"), pad_at(&v, "R2", "1"));
    let t = id(&ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "top", "points": [a, [11.0, a[1]], [11.0 + (a[1] - b[1]).abs(), b[1]], b] })));
    assert!(de_pcb::view(p.world(), board).unwrap().nets.iter().find(|n| n.name == "SIG").unwrap().islands == 2);

    // Move R1 up 1.5 mm and left 1 mm: the track's first point goes with pad 2, corners stay at 45°.
    let out = ok(&mut p, "pcb.component.update", json!({ "target": r1, "at": [7, 11.5] }));
    assert_eq!(out["tracks_dragged"], 1, "{out}");
    let v = de_pcb::view(p.world(), board).unwrap();
    let tr = v.tracks.iter().find(|x| x.id == t).unwrap();
    let a2 = pad_at(&v, "R1", "2");
    assert!(geom::dist(tr.points[0], a2) < 1e-9, "{:?} vs {a2:?}", tr.points);
    assert!(geom::dist(*tr.points.last().unwrap(), b) < 1e-9, "the other end stays on R2");
    assert!(tr.points.windows(2).all(|w| octilinear(w[0], w[1])), "45° corners: {:?}", tr.points);
    assert!(tr.reroute, "a dragged track is marked for re-routing");
    let sig = v.nets.iter().find(|n| n.name == "SIG").unwrap();
    assert_eq!(sig.islands, 2, "R1.2 and R2.1 still joined (J1.2 left)");
    assert!(!v.violations.iter().any(|x| x.code == "short" || x.code == "clearance"), "{:#?}", v.violations);

    // Turn R2 back to 0°: its pad 1 is now to the left of its centre and the end follows.
    ok(&mut p, "pcb.component.update", json!({ "target": r2, "rotation": 0 }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let tr = v.tracks.iter().find(|x| x.id == t).unwrap();
    let b2 = pad_at(&v, "R2", "1");
    assert!(geom::dist(*tr.points.last().unwrap(), b2) < 1e-9, "{:?} vs {b2:?}", tr.points);
    assert!(tr.points.windows(2).all(|w| octilinear(w[0], w[1])), "{:?}", tr.points);

    // Moves without dragging leave tracks alone.
    let before = tr.points.clone();
    ok(&mut p, "pcb.component.update", json!({ "target": r1, "at": [6, 11.5], "drag_tracks": false }));
    let v = de_pcb::view(p.world(), board).unwrap();
    assert_eq!(v.tracks.iter().find(|x| x.id == t).unwrap().points, before);

    // Editing the track by hand clears the mark.
    ok(&mut p, "pcb.track.update", json!({ "target": t, "width": 0.3 }));
    let v = de_pcb::view(p.world(), board).unwrap();
    assert!(!v.tracks.iter().find(|x| x.id == t).unwrap().reroute);
}

#[test]
fn a_track_between_two_pads_of_one_part_moves_whole() {
    let mut p = project();
    let board = id(&ok(&mut p, "pcb.board.create", json!({ "name": "B", "width": 30, "height": 20 })));
    let u = id(&ok(&mut p, "pcb.component.create", json!({ "board": board, "reference": "U1", "footprint": "SOIC-8", "at": [10, 10], "pins": { "1": "A", "2": "A" } })));
    let v = de_pcb::view(p.world(), board).unwrap();
    let (a, b) = (pad_at(&v, "U1", "1"), pad_at(&v, "U1", "2"));
    let t = id(&ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "top", "net": "A", "width": 0.2, "points": [a, b] })));
    ok(&mut p, "pcb.component.update", json!({ "target": u, "at": [15, 8] }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let tr = v.tracks.iter().find(|x| x.id == t).unwrap();
    assert!(geom::dist(tr.points[0], pad_at(&v, "U1", "1")) < 1e-9 && geom::dist(tr.points[1], pad_at(&v, "U1", "2")) < 1e-9, "{:?}", tr.points);
}

#[test]
fn a_hand_route_changes_layer_through_a_via() {
    let mut p = project();
    let (board, _, _, _) = small_board(&mut p);
    let v = de_pcb::view(p.world(), board).unwrap();
    let (a, j) = (pad_at(&v, "R1", "2"), pad_at(&v, "J1", "2"));
    // Top from R1.2 to x = 11, a via, then the bottom to the header's (through-hole) pin 2.
    let out = ok(&mut p, "pcb.route.create", json!({ "board": board, "runs": [
        { "layer": "top", "points": [a, [11.0, a[1]]] },
        { "layer": "bottom", "points": [[11.0, a[1]], [11.0, 6.0], [20.0, 6.0], [20.0, j[1]], j] },
    ] }));
    assert_eq!(out["net"], "SIG");
    assert_eq!(out["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(out["vias"].as_array().unwrap().len(), 1);
    let v = de_pcb::view(p.world(), board).unwrap();
    assert_eq!(v.vias.len(), 1);
    assert!(geom::dist(v.vias[0].at, [11.0, a[1]]) < 1e-9);
    assert_eq!(v.vias[0].net.as_deref(), Some("SIG"));
    // The via uses the net class's size; both tracks are on SIG and on their layers.
    assert!((v.vias[0].diameter - 0.6).abs() < 1e-9 && (v.vias[0].drill - 0.3).abs() < 1e-9);
    let layers: Vec<&str> = v.tracks.iter().map(|t| t.layer.as_str()).collect();
    assert!(layers.contains(&"top") && layers.contains(&"bottom"));
    // R1.2 and J1.2 are now one piece of copper through the via.
    let sig = v.nets.iter().find(|n| n.name == "SIG").unwrap();
    assert_eq!(sig.islands, 2, "only R2.1 left: {sig:?}");
    assert!(!v.violations.iter().any(|x| x.severity == "error" && x.code != "unrouted"), "{:#?}", v.violations);
    // Two runs on one layer, or runs that do not meet, are refused.
    assert!(run(&mut p, "pcb.route.create", json!({ "board": board, "runs": [{ "layer": "top", "points": [[1, 1], [2, 1]] }, { "layer": "top", "points": [[2, 1], [3, 1]] }] })).is_err());
    assert!(run(&mut p, "pcb.route.create", json!({ "board": board, "runs": [{ "layer": "top", "points": [[1, 1], [2, 1]] }, { "layer": "bottom", "points": [[2, 2], [3, 1]] }] })).is_err());
}

fn in_fill(z: &de_pcb::board::ZoneView, p: P) -> bool {
    z.fill.iter().any(|island| geom::inside_area(p, island))
}

#[test]
fn pours_join_their_pads_through_thermal_reliefs() {
    let mut p = project();
    let (board, _, _, _) = small_board(&mut p);
    // GND on the bottom: J1.3 (through-hole, 1.7 mm oval) joins it.
    let z = id(&ok(&mut p, "pcb.zone.create", json!({ "board": board, "net": "GND", "layer": "bottom", "thermal_gap": 0.4, "thermal_spoke": 0.45 })));
    let v = de_pcb::view(p.world(), board).unwrap();
    let zone = &v.zones[0];
    let c = pad_at(&v, "J1", "3");
    let edge = 1.7 / 2.0;
    // In the gap ring between spokes (45°) there is no copper; on a spoke axis there is.
    let diag = geom::add(c, [(edge + 0.2) * std::f64::consts::FRAC_1_SQRT_2, (edge + 0.2) * std::f64::consts::FRAC_1_SQRT_2]);
    assert!(!in_fill(zone, diag), "the relief gap is clear at 45°");
    for dir in [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]] {
        let q = geom::add(c, [dir[0] * (edge + 0.2), dir[1] * (edge + 0.2)]);
        assert!(in_fill(zone, q), "a spoke crosses the gap toward {dir:?}");
    }
    // Copper just past the gap is solid pour again.
    assert!(in_fill(zone, geom::add(c, [edge + 0.4 + 0.3, edge + 0.4 + 0.3])));
    let relief = zone.reliefs.iter().find(|r| r.pad == "J1.3").expect("J1.3 relieved");
    assert_eq!(relief.spokes, 4);
    // The spoke is the asked width: across a spoke, copper spans ~0.45 mm.
    let mid = edge + 0.2;
    let across: Vec<f64> = (0..200).map(|k| -0.5 + k as f64 * 0.005).filter(|y| in_fill(zone, [c[0] + mid, c[1] + y])).collect();
    let span = across.last().unwrap() - across.first().unwrap();
    assert!((span - 0.45).abs() < 0.03, "spoke width {span}");
    // J1.3 is in the pour's copper (the GND net's pieces: R2.2 on top, J1.3 + pour).
    assert!(!v.violations.iter().any(|x| x.severity == "error" && x.code != "unrouted"), "{:#?}", v.violations);
    let gnd_islands = v.nets.iter().find(|n| n.name == "GND").unwrap().islands;
    assert_eq!(gnd_islands, 2);

    // Solid: the ring is copper.
    ok(&mut p, "pcb.zone.update", json!({ "target": z, "connection": "solid" }));
    let v = de_pcb::view(p.world(), board).unwrap();
    assert!(in_fill(&v.zones[0], diag), "solid joins all round");
    assert!(v.zones[0].reliefs.is_empty());

    // None: the pad keeps clear and is no longer joined.
    ok(&mut p, "pcb.zone.update", json!({ "target": z, "connection": "none" }));
    let v = de_pcb::view(p.world(), board).unwrap();
    for dir in [[1.0, 0.0], [0.0, 1.0]] {
        assert!(!in_fill(&v.zones[0], geom::add(c, [dir[0] * (edge + 0.2), dir[1] * (edge + 0.2)])));
    }

    // A pad hemmed in by other nets gets fewer spokes, and says so.
    ok(&mut p, "pcb.zone.update", json!({ "target": z, "connection": "thermal" }));
    for (k, y) in [(0, 12.54), (1, 12.54 - 5.08 - 1.5)] {
        let _ = k;
        ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "bottom", "net": "VCC", "width": 0.6, "points": [[20.0, y], [28.0, y]] }));
    }
    ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "bottom", "net": "VCC", "width": 0.6, "points": [[22.0, 5.0], [22.0, 12.54]] }));
    ok(&mut p, "pcb.track.create", json!({ "board": board, "layer": "bottom", "net": "VCC", "width": 0.6, "points": [[26.0, 5.0], [26.0, 12.54]] }));
    let v = de_pcb::view(p.world(), board).unwrap();
    let relief = v.zones[0].reliefs.iter().find(|r| r.pad == "J1.3").unwrap();
    assert!(relief.spokes < 4, "{relief:?}");
    if relief.spokes < 2 {
        assert!(v.violations.iter().any(|x| x.code == "thermal_relief" && x.refs.contains(&"J1.3".to_string())), "{:#?}", v.violations);
    }
    // Nothing the pour does breaks clearance.
    assert!(!v.violations.iter().any(|x| x.code == "clearance" && x.refs.iter().any(|r| r.contains("GND"))), "{:#?}", v.violations);
}
