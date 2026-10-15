//! The interactive router through its commands: walkaround, shove, drags,
//! optimise, clean-up, unroute, break, fillet and fan-out, each checked by the
//! board's own design-rule check.

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

/// Errors other than unrouted connections.
fn errors(p: &Project, b: StableId) -> Vec<String> {
    let v = de_pcb::view(p.world(), b).unwrap();
    v.violations.iter().filter(|x| x.severity == "error" && x.code != "unrouted").map(|x| format!("{}: {}", x.code, x.message)).collect()
}

/// Two resistors to join on net A, and a B track lying 0.4 mm above the straight way between them.
fn crossing() -> (Project, StableId, StableId) {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "t", "width": 30, "height": 20, "layers": 2 })));
    for (r, at, pins) in [("R1", [5.0, 10.0], json!({ "2": "A" })), ("R2", [25.0, 10.0], json!({ "1": "A" })), ("R3", [10.0, 15.0], json!({ "1": "B" })), ("R4", [20.0, 15.0], json!({ "2": "B" }))] {
        ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": r, "footprint": "R_0603", "value": "1k", "at": at, "pins": pins }));
    }
    let t = id(&ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "points": [[9.175, 15.0], [9.175, 10.4], [20.825, 10.4], [20.825, 15.0]] })));
    (p, b, t)
}

fn track_points(p: &Project, t: StableId) -> Vec<[f64; 2]> {
    p.get::<de_pcb::Track>(t).unwrap().points.clone()
}

#[test]
fn walkaround_goes_round_and_keeps_every_rule() {
    let (mut p, b, t) = crossing();
    let before = errors(&p, b);
    let out = ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R1.2", "to": "R2.1", "mode": "walkaround" }));
    assert_eq!(out["reached"], true, "{out}");
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["net"], "A");
    assert_eq!(track_points(&p, t), vec![[9.175, 15.0], [9.175, 10.4], [20.825, 10.4], [20.825, 15.0]], "walkaround moves nothing");
    assert_eq!(errors(&p, b), before, "no new violations");
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.nets.iter().find(|n| n.name == "A").unwrap().routed);
}

#[test]
fn shove_pushes_the_other_track_and_keeps_every_rule() {
    let (mut p, b, t) = crossing();
    let out = ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R1.2", "to": "R2.1", "mode": "shove" }));
    assert_eq!(out["mode"], "shove", "{out}");
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["pushed_tracks"], 1);
    let pushed = track_points(&p, t);
    assert!(pushed.iter().all(|q| q[1] >= 10.4 - 1e-9), "pushed up, away: {pushed:?}");
    assert_eq!(pushed[0], [9.175, 15.0], "its ends stay on their pads");
    assert!(errors(&p, b).is_empty(), "{:?}", errors(&p, b));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.nets.iter().all(|n| n.routed || n.pins.len() < 2), "both nets joined");
    // The new line is straight: the B track made room.
    let a = v.tracks.iter().find(|x| x.net.as_deref() == Some("A")).unwrap();
    assert_eq!(a.points.len(), 2, "{:?}", a.points);
}

#[test]
fn highlight_reports_what_it_breaks() {
    let (p, b, _) = crossing();
    let board = p.get::<de_pcb::Board>(b).unwrap().clone();
    let v = de_pcb::view(p.world(), b).unwrap();
    let spec: de_pcb::routing::RouteInteractive = serde_json::from_value(json!({ "board": b, "from": "R1.2", "to": "R2.1", "mode": "highlight" })).unwrap();
    let plan = de_pcb::routing::plan_route(&board, &v, &spec).unwrap();
    assert!(!plan.ok);
    assert!(plan.collisions.iter().any(|c| c.with == "track on B"), "{:?}", plan.collisions);
    assert!((plan.clearance - 0.2).abs() < 1e-9);
}

#[test]
fn routes_change_layer_with_a_via() {
    let (mut p, b, _) = crossing();
    let out = ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R1.2", "to": [{ "at": [8.0, 10.0] }, { "at": [22.0, 10.0], "layer": "bottom" }, { "at": "R2.1", "layer": "top" }], "mode": "walkaround" }));
    assert_eq!(out["vias"].as_array().unwrap().len(), 2, "{out}");
    assert_eq!(out["tracks"].as_array().unwrap().len(), 3, "{out}");
    assert!(errors(&p, b).is_empty(), "{:?}", errors(&p, b));
}

#[test]
fn drag_a_segment_at_45_degrees_pushing_its_neighbour() {
    let (mut p, b, t) = crossing();
    // Route A first (walking round), then drag the B track's long segment down onto it: A is pushed.
    ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R1.2", "to": "R2.1", "mode": "walkaround" }));
    let out = ok(&mut p, "pcb.route.drag", json!({ "board": b, "target": t, "kind": "segment", "index": 1, "from": [15.0, 10.4], "to": [15.0, 12.0], "mode": "shove" }));
    assert_eq!(out["ok"], true, "{out}");
    let pts = track_points(&p, t);
    assert!(pts.iter().any(|q| (q[1] - 12.0).abs() < 1e-9), "segment moved to y 12: {pts:?}");
    for w in pts.windows(2) {
        let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
        assert!(d[0].abs() < 1e-9 || d[1].abs() < 1e-9 || (d[0].abs() - d[1].abs()).abs() < 1e-9, "45°: {pts:?}");
    }
    assert!(errors(&p, b).is_empty(), "{:?}", errors(&p, b));
    // Corner and free drags.
    let out = ok(&mut p, "pcb.route.drag", json!({ "board": b, "target": t, "kind": "corner", "index": 1, "to": [9.5, 12.5], "mode": "highlight" }));
    let now = track_points(&p, t);
    assert!(now.windows(2).any(|w| de_pcb::geom::point_seg_dist([9.5, 12.5], w[0], w[1]) < 1e-9), "{out} {now:?} was {pts:?}");
}

#[test]
fn drag_a_via_and_its_tracks_follow() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "t", "width": 30, "height": 20, "layers": 2 })));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R1", "footprint": "R_0603", "at": [5.0, 10.0], "pins": { "2": "A" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R2", "footprint": "R_0603", "at": [25.0, 10.0], "pins": { "1": "A" } }));
    let out = ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R1.2", "to": [{ "at": [15.0, 10.0] }, { "at": [20.0, 10.0], "layer": "bottom" }, { "at": "R2.1", "layer": "top" }] }));
    let via: StableId = serde_json::from_value(out["vias"][0].clone()).unwrap();
    ok(&mut p, "pcb.route.drag", json!({ "board": b, "target": via, "to": [15.0, 13.0] }));
    assert_eq!(p.get::<de_pcb::Via>(via).unwrap().at, [15.0, 13.0]);
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.nets.iter().find(|n| n.name == "A").unwrap().routed, "still joined through the moved via");
    assert!(errors(&p, b).is_empty(), "{:?}", errors(&p, b));
}

#[test]
fn arcs_fillet_drag_and_break() {
    let (mut p, b, t) = crossing();
    let out = ok(&mut p, "pcb.track.fillet", json!({ "targets": [t], "radius": 1.0 }));
    assert_eq!(out["tracks"][0]["radius"], 1.0, "{out}");
    let tr = p.get::<de_pcb::Track>(t).unwrap().clone();
    assert!(tr.bulges.iter().filter(|x| x.abs() > 1e-9).count() == 2, "{:?}", tr.bulges);
    let v = de_pcb::view(p.world(), b).unwrap();
    let tv = v.tracks.iter().find(|x| x.id == t).unwrap();
    assert!(tv.points.len() > tr.points.len(), "flattened for copper");
    assert!(v.nets.iter().find(|n| n.name == "B").unwrap().routed);
    // Drag the first arc wider.
    let k = tr.bulges.iter().position(|x| x.abs() > 1e-9).unwrap();
    let mid = de_pcb::pns::arc_mid(tr.points[k], tr.points[k + 1], tr.bulges[k]);
    ok(&mut p, "pcb.route.drag", json!({ "board": b, "target": t, "kind": "arc", "index": k, "to": [mid[0] - 0.1, mid[1] - 0.1], "mode": "highlight" }));
    assert!(p.get::<de_pcb::Track>(t).unwrap().bulges[k].abs() > tr.bulges[k].abs());
    // Break the straight long run in the middle.
    let out = ok(&mut p, "pcb.track.break", json!({ "target": t, "at": [15.0, 10.4] }));
    assert_eq!(out["tracks"].as_array().unwrap().len(), 2);
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.nets.iter().find(|n| n.name == "B").unwrap().routed, "two tracks still join");
}

#[test]
fn cleanup_merges_trims_and_drops_redundant_vias() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "t", "width": 30, "height": 20, "layers": 2 })));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R1", "footprint": "R_0603", "at": [5.0, 10.0], "pins": { "2": "A" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R2", "footprint": "R_0603", "at": [25.0, 10.0], "pins": { "1": "A" } }));
    // Two tracks end to end with a straight-through point, plus a stub off to nowhere and a lone via.
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "A", "points": [[5.825, 10.0], [10.0, 10.0], [15.0, 10.0]] }));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "A", "points": [[15.0, 10.0], [24.175, 10.0]] }));
    ok(&mut p, "pcb.track.create", json!({ "board": b, "layer": "top", "net": "A", "points": [[12.0, 10.0], [12.0, 14.0], [14.0, 16.0]] }));
    ok(&mut p, "pcb.via.create", json!({ "board": b, "net": "A", "at": [20.0, 10.0] }));
    let out = ok(&mut p, "pcb.route.cleanup", json!({ "board": b }));
    assert_eq!(out["joined"], 1, "{out}");
    assert_eq!(out["redundant_vias"], 1, "{out}");
    assert!(out["trimmed"].as_u64().unwrap() >= 1, "{out}");
    let v = de_pcb::view(p.world(), b).unwrap();
    assert_eq!(v.tracks.len(), 1, "{:?}", v.tracks.iter().map(|t| &t.points).collect::<Vec<_>>());
    assert_eq!(v.tracks[0].points, vec![[5.825, 10.0], [24.175, 10.0]]);
    assert!(v.vias.is_empty());
    assert!(v.nets.iter().find(|n| n.name == "A").unwrap().routed);
}

/// The example board, autorouted.
fn esp32() -> (Project, StableId, Vec<Value>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("steps.json")).unwrap()).unwrap();
    let mut p = project();
    let mut outs: Vec<Value> = Vec::new();
    fn subst(v: &Value, outs: &[Value]) -> Value {
        match v {
            Value::String(s) if s.starts_with('$') => {
                let (n, path) = s[1..].split_once('.').unwrap();
                outs[n.parse::<usize>().unwrap()][path].clone()
            }
            Value::Array(a) => Value::Array(a.iter().map(|x| subst(x, outs)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), subst(x, outs))).collect()),
            other => other.clone(),
        }
    }
    for s in &steps {
        let params = subst(&s["parameters"], &outs);
        outs.push(ok(&mut p, s["operation"].as_str().unwrap(), params));
    }
    let b = id(&outs[0]);
    (p, b, outs)
}

#[test]
fn on_the_example_board_optimise_cleanup_unroute_and_fanout_keep_it_clean() {
    let (mut p, b, outs) = esp32();
    let before = errors(&p, b);
    let out = ok(&mut p, "pcb.route.optimize", json!({ "board": b }));
    eprintln!("optimize: {out}");
    assert_eq!(errors(&p, b), before, "optimise breaks nothing");
    let out = ok(&mut p, "pcb.route.cleanup", json!({ "board": b }));
    eprintln!("cleanup: {out}");
    assert_eq!(errors(&p, b), before, "clean-up breaks nothing");
    let v = de_pcb::view(p.world(), b).unwrap();
    let routed = v.nets.iter().filter(|n| n.routed).count();
    let out = ok(&mut p, "pcb.route.unroute", json!({ "board": b, "nets": ["IO2"] }));
    assert!(out["tracks"].as_u64().unwrap() >= 1, "{out}");
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(!v.nets.iter().find(|n| n.name == "IO2").unwrap().routed);
    assert_eq!(v.nets.iter().filter(|n| n.routed).count(), routed - 1);
    // Route it again by hand, pad to pad, shoving.
    let out = ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R5.2", "to": "U1.24", "mode": "shove" }));
    eprintln!("route IO2: {out}");
    assert_eq!(out["ok"], true, "{out}");
    if out["reached"] == true {
        let v = de_pcb::view(p.world(), b).unwrap();
        assert!(v.nets.iter().find(|n| n.name == "IO2").unwrap().routed);
    }
    assert_eq!(errors(&p, b), before, "hand routing breaks nothing");
    // Unroute a part: the copper on its pads goes.
    let r5: StableId = serde_json::from_value(outs.iter().find(|o| o.get("reference").is_some_and(|r| r == "R5")).map(|o| o["id"].clone()).unwrap_or_else(|| outs[16]["id"].clone())).unwrap();
    ok(&mut p, "pcb.route.unroute", json!({ "board": b, "components": [r5] }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(!v.nets.iter().find(|n| n.name == "IO2").unwrap().routed);
    // Fan out the regulator's pads that have no copper yet.
    let u2: StableId = serde_json::from_value(outs[6]["id"].clone()).unwrap();
    ok(&mut p, "pcb.route.unroute", json!({ "board": b, "components": [u2] }));
    let out = ok(&mut p, "pcb.fanout", json!({ "component": u2 }));
    eprintln!("fanout: {out}");
    assert!(!out["fanned"].as_array().unwrap().is_empty(), "{out}");
    assert_eq!(errors(&p, b), before, "fan-out breaks nothing");
}

/// A board whose straight way from R1 to R2 crosses a keepout, with a ground via-fan-out candidate beside it.
fn keepout_board() -> (Project, StableId) {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "k", "width": 40, "height": 20, "layers": 2, "rules": { "hole_to_hole": 0.5 } })));
    for (r, at, pins) in [("R1", [5.0, 10.0], json!({ "1": "GND", "2": "A" })), ("R2", [35.0, 10.0], json!({ "1": "A", "2": "GND" }))] {
        ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": r, "footprint": "R_0603", "value": "1k", "at": at, "pins": pins }));
    }
    // A keepout across the middle (tracks, vias and pours), leaving a way round below it.
    ok(&mut p, "pcb.area.create", json!({ "board": b, "name": "Antenna", "outline": [[15, 4], [25, 4], [25, 20], [15, 20]] }));
    (p, b)
}

#[test]
fn the_autorouter_and_the_router_keep_out_of_keepouts() {
    let (mut p, b) = keepout_board();
    let out = ok(&mut p, "pcb.autoroute", json!({ "board": b, "nets": ["A"] }));
    assert_eq!(out["failed"].as_array().unwrap().len(), 0, "{out}");
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.nets.iter().find(|n| n.name == "A").unwrap().routed);
    let keepouts: Vec<_> = v.violations.iter().filter(|x| x.code == "keepout").map(|x| x.message.clone()).collect();
    assert!(keepouts.is_empty(), "autorouted through the keepout: {keepouts:?}");
    // The router goes round it too.
    ok(&mut p, "pcb.route.unroute", json!({ "board": b, "nets": ["A"] }));
    let r = ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R1.2", "to": "R2.1", "layer": "top", "mode": "walkaround" }));
    assert_eq!(r["reached"], json!(true), "{r}");
    let errs = errors(&p, b);
    assert!(errs.iter().all(|e| !e.starts_with("keepout")), "{errs:?}");
}

#[test]
fn fanout_keeps_out_of_keepouts_and_keeps_hole_spacing() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "f", "width": 20, "height": 20, "layers": 2, "rules": { "hole_to_hole": 0.5 } })));
    // Two ground pads side by side: their fan-out vias must keep 0.5 mm between holes.
    for (r, at) in [("C1", [10.0, 10.0]), ("C2", [10.0, 11.2])] {
        ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": r, "footprint": "C_0603", "value": "100nF", "at": at, "pins": { "1": "GND", "2": "V" } }));
    }
    // A keepout to the left of the ground pads: no via there.
    ok(&mut p, "pcb.area.create", json!({ "board": b, "name": "Keep", "outline": [[0, 0], [8.5, 0], [8.5, 20], [0, 20]], "no_vias": true }));
    let v = de_pcb::view(p.world(), b).unwrap();
    for c in v.components.clone() {
        ok(&mut p, "pcb.fanout", json!({ "component": c.id, "nets": ["GND"] }));
    }
    let errs = errors(&p, b);
    assert!(errs.iter().all(|e| !e.starts_with("keepout") && !e.starts_with("hole_to_hole")), "{errs:?}");
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(!v.vias.is_empty(), "the ground pads are fanned out somewhere");
    assert!(v.vias.iter().all(|x| x.at[0] > 8.5 + x.diameter / 2.0 - 1e-9), "{:?}", v.vias.iter().map(|x| x.at).collect::<Vec<_>>());
}


#[test]
fn fanout_and_stitching_vias_stay_out_of_smd_pads_of_their_own_net() {
    let mut p = project();
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "s", "width": 20, "height": 20, "layers": 2 })));
    // A large ground pad (a connector's mounting tab) right where C1's ground fan-out would go.
    ok(&mut p, "pcb.footprint.define", json!({ "board": b, "footprint": { "name": "Tab", "pads": [{ "number": "MP", "kind": "smd", "shape": "rect", "size": [2.0, 2.0], "at": [0.0, 0.0] }], "source": "test" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "J1", "footprint": "Tab", "value": "tab", "at": [7.6, 10.0], "pins": { "MP": "GND" } }));
    let c1 = id(&ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "C1", "footprint": "C_0603", "value": "100nF", "at": [10.0, 10.0], "pins": { "1": "GND", "2": "V" } })));
    ok(&mut p, "pcb.zone.create", json!({ "board": b, "net": "GND", "layer": "bottom" }));
    ok(&mut p, "pcb.fanout", json!({ "component": c1, "nets": ["GND"] }));
    ok(&mut p, "pcb.via.stitch", json!({ "board": b, "net": "GND", "pitch": 1.0 }));
    let v = de_pcb::view(p.world(), b).unwrap();
    assert!(v.vias.len() > 10, "{} vias", v.vias.len());
    for via in &v.vias {
        for pad in v.pads.iter().filter(|x| x.kind == de_pcb::footprints::PadKind::Smd) {
            let g = de_pcb::geom::gap(&de_pcb::geom::Prim::circle(via.at, via.diameter / 2.0), &pad.prim);
            assert!(g >= 0.049, "via at {:?} is in {}.{} (gap {g})", via.at, pad.reference, pad.number);
        }
    }
}
