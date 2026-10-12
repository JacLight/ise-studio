//! examples/robot-arm, built from its steps.json the way `forma.mjs run`
//! does, then verified: every requirement met (each joint clash-free over its
//! limits, everything fits), the fit report with no failing item, and a BOM
//! with no flags (every bought line with a part number and a source, every
//! made line with material, process and estimate).
//!
//! `FORMA_ARM_QUICK=1` skips the motion requirements (minutes) for a quick look;
//! `FORMA_ARM_REPORT=1` prints the fit report's warnings and failures.

use de_assembly::AssemblyPlugin;
use de_assembly::fit::{self, FitOptions, FitStatus};
use de_core::{Actor, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_intent::{IntentPlugin, Requirement, RequirementResult};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use de_sourcing::{SourcingPlugin, bom::{self, Scope}};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/robot-arm")
}

fn resolve(v: &Value, outputs: &[Value], base: &Path) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') && s.contains('.') => {
            let (n, path) = s[1..].split_once('.').unwrap();
            let mut cur = outputs[n.parse::<usize>().unwrap()].clone();
            for k in path.split('.') {
                cur = cur[k].clone();
            }
            assert!(!cur.is_null(), "{s} does not resolve");
            cur
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve(x, outputs, base)).collect()),
        Value::Object(o) if o.contains_key("$file") => {
            let files: Vec<String> = match &o["$file"] {
                Value::Array(a) => a.iter().map(|f| f.as_str().unwrap().to_string()).collect(),
                f => vec![f.as_str().unwrap().to_string()],
            };
            Value::String(files.iter().map(|f| std::fs::read_to_string(base.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"))).collect::<Vec<_>>().join("\n"))
        }
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), resolve(x, outputs, base))).collect()),
        x => x.clone(),
    }
}

pub fn build(quick: bool) -> (Project, StableId) {
    let registry = Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin, &IntentPlugin, &SourcingPlugin]));
    let mut p = Project::new("robot-arm", registry);
    let base = example();
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(base.join("steps.json")).unwrap()).unwrap();
    let mut outputs = vec![];
    let mut asm = None;
    for (i, step) in steps.iter().enumerate() {
        let op = step["operation"].as_str().unwrap();
        let params = resolve(&step["parameters"], &outputs, &base);
        if quick && op == "intent.requirement.create" && params["rule"]["kind"] == "no_clashes_in_motion" {
            outputs.push(json!({}));
            continue;
        }
        let envelope = Envelope {
            schema_version: SCHEMA_VERSION,
            command_id: StableId::new().to_string(),
            project_id: p.id().into(),
            expected_revision: p.revision(),
            operation: op.into(),
            parameters: params,
            actor: Actor { user_id: "u".into(), session_id: "s".into() },
        };
        let out = p.commit(envelope).unwrap_or_else(|e| panic!("step {i} {op}: {e:?}")).output;
        if op == "assembly.create" {
            asm = serde_json::from_value(out["id"].clone()).ok();
        }
        outputs.push(out);
    }
    (p, asm.expect("the steps make an assembly"))
}

fn commit(p: &mut Project, operation: &str, parameters: Value) {
    let envelope = Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().into(),
        expected_revision: p.revision(),
        operation: operation.into(),
        parameters,
        actor: Actor { user_id: "u".into(), session_id: "s".into() },
    };
    p.commit(envelope).unwrap_or_else(|e| panic!("{operation}: {e:?}"));
}

fn joint_named(p: &Project, scope: StableId, name: &str) -> (StableId, de_assembly::joints::Joint) {
    de_assembly::joints::joints_in(p.world(), Some(scope)).into_iter().find(|(id, _)| de_assembly::joints::joint_name(p.world(), *id) == name).unwrap_or_else(|| panic!("no joint {name}"))
}

/// The instances that move with a hinge's b side: its component once the hinge is cut.
fn downstream(p: &Project, scope: StableId, hinge: StableId) -> Vec<StableId> {
    let joints = de_assembly::joints::joints_in(p.world(), Some(scope));
    let (_, h) = joints.iter().find(|(id, _)| *id == hinge).unwrap();
    let start = h.b.instance().unwrap();
    let mut seen = vec![start];
    let mut k = 0;
    while k < seen.len() {
        let cur = seen[k];
        for (id, j) in &joints {
            if *id == hinge {
                continue;
            }
            let (a, b) = (j.a.instance(), j.b.instance());
            for (x, y) in [(a, b), (b, a)] {
                if x == Some(cur)
                    && let Some(y) = y
                    && !seen.contains(&y)
                {
                    seen.push(y);
                }
            }
        }
        k += 1;
    }
    seen
}

/// Mass (kg) and centre of mass (mm, assembly coordinates) of an instance as placed.
fn mass_of(p: &Project, inst: StableId) -> (f64, [f64; 3]) {
    let world = p.world();
    let e = p.entity(inst).unwrap();
    let i = world.get::<de_assembly::Instance>(e).unwrap();
    let place = de_assembly::placement(world, e).unwrap();
    let mp = match world.get::<de_assembly::InstanceConfigured>(e).and_then(|c| c.0.clone()) {
        Some(c) => c.mass.clone(),
        None => p.get::<de_solid::MassProperties>(i.of).unwrap().clone(),
    };
    let c = mp.center_of_mass_m.map(|v| v * 1000.0);
    (mp.mass_kg.unwrap_or(0.0), place.apply(c))
}

/// Holding torque (N·m) about a hinge's axis for what hangs from it, plus `payload` kg at `at`.
fn holding_torque(p: &Project, scope: StableId, hinge: StableId, payload: f64, at: [f64; 3]) -> (f64, f64) {
    let world = p.world();
    let link = de_assembly::joints::link(world, hinge, None).unwrap();
    let de_assembly::joints::Side::Inst(a, frame) = link.a else { panic!("hinge on the ground") };
    let pa = de_assembly::placement(world, p.entity(a).unwrap()).unwrap().then(&frame);
    let (o, axis) = (pa.at, pa.turn([0.0, 0.0, 1.0]));
    let mut moment = [0.0; 3];
    let mut mass = 0.0;
    let mut add = |m: f64, c: [f64; 3]| {
        let r = [c[0] - o[0], c[1] - o[1], c[2] - o[2]];
        let f = [0.0, 0.0, -m * 9.81];
        let rf = [r[1] * f[2] - r[2] * f[1], r[2] * f[0] - r[0] * f[2], r[0] * f[1] - r[1] * f[0]];
        for k in 0..3 {
            moment[k] += rf[k] / 1000.0;
        }
        mass += m;
    };
    for i in downstream(p, scope, hinge) {
        let (m, c) = mass_of(p, i);
        if std::env::var("FORMA_ARM_MASSES").is_ok() {
            println!("  {:>7.1} g at x {:>6.1}  {}", m * 1000.0, c[0] - o[0], p.get::<de_assembly::Instance>(i).unwrap().name);
        }
        add(m, c);
    }
    add(payload, at);
    ((moment[0] * axis[0] + moment[1] * axis[1] + moment[2] * axis[2]).abs(), mass)
}

#[test]
fn the_robot_arm_builds_fits_moves_clear_and_has_a_clean_bom() {
    let quick = std::env::var("FORMA_ARM_QUICK").is_ok();
    let (p, asm) = build(quick);
    let world = p.world();
    let r = fit::report(world, Some(asm), &FitOptions { motion: false, steps: 24 }).unwrap();
    if std::env::var("FORMA_ARM_REPORT").is_ok() {
        println!("fit: {} pass, {} warn, {} fail", r.pass, r.warn, r.fail);
        for i in r.items.iter().filter(|i| i.status != FitStatus::Pass) {
            println!("{:?} {}", i.status, i.title);
            for c in i.checks.iter().filter(|c| c.status != FitStatus::Pass) {
                println!("    {:?} {}: {}", c.status, c.what, c.text);
            }
        }
    }
    let failing: Vec<String> = r.items.iter().filter(|i| i.status == FitStatus::Fail).map(|i| i.title.clone()).collect();
    assert!(failing.is_empty(), "fit report fails: {failing:#?}");
    for id in p.entity_ids() {
        if let Some(req) = p.get::<Requirement>(id) {
            let res = p.get::<RequirementResult>(id).unwrap_or_else(|| panic!("{} not evaluated: {:?}", req.name, p.status(id)));
            println!("{}: {}", req.name, res.summary);
            assert!(res.passed, "{} fails: {}", req.name, res.summary);
        }
    }
    let b = bom::compute(world, Scope::Entity(asm), &bom::Options::default(), &[]).unwrap();
    println!("BOM: {} made lines, {} bought lines, flags {:?}", b.make.len(), b.buy.len(), b.flags);
    assert!(b.flags.is_empty(), "BOM flags: {:#?}", b.flags);
    assert!(b.buy.iter().all(|l| l.stock.as_ref().is_some_and(|s| !s.part_number.is_empty() && !s.sources.is_empty())), "every bought line has a part number and a source");
    println!("BOM totals: {}", serde_json::to_string(&b.totals).unwrap());
    let mut p = p;
    reach_and_strength(&mut p, asm);
}

/// Full horizontal reach with 100 g in the gripper: the holding torques at the
/// shoulder and elbow against the servos' stall torque, and the upper arm's
/// left half (it carries both servos) under that load.
fn reach_and_strength(p: &mut Project, asm: StableId) {
    // Datasheet-typical stall torques (TowerPro): MG996R 9.4 kg·cm at 4.8 V, 11 kg·cm at 6 V; MG90S 1.8 / 2.2 kg·cm.
    const MG996R_STALL: f64 = 0.92;
    const MG996R_STALL_6V: f64 = 1.08;
    const MG90S_STALL: f64 = 0.18;
    const PAYLOAD: f64 = 0.1;
    let (shoulder, _) = joint_named(p, asm, "Shoulder");
    let (elbow, _) = joint_named(p, asm, "Elbow");
    let (wrist, _) = joint_named(p, asm, "Wrist");
    let mut full = None;
    for (pose, [s, e, w]) in [("Full horizontal reach", [90.0, 90.0, 0.0]), ("Home (upper arm up, forearm level)", [0.0, 0.0, 0.0])] {
        commit(p, "parameters.set", json!({ "target": shoulder, "values": { "angle": s } }));
        commit(p, "parameters.set", json!({ "target": elbow, "values": { "angle": e } }));
        commit(p, "parameters.set", json!({ "target": wrist, "values": { "angle": w } }));
        let world = p.world();
        // The payload between the jaw pads: the fingers' far ends.
        let fingers: Vec<StableId> = p.entity_ids().into_iter().filter(|x| p.get::<de_assembly::Instance>(*x).is_some_and(|i| i.name.starts_with("Finger, "))).collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for f in &fingers {
            let st = p.get::<de_assembly::AssemblyStats>(*f).unwrap();
            for k in 0..3 {
                lo[k] = lo[k].min(st.bounds_min[k]);
                hi[k] = hi[k].max(st.bounds_max[k]);
            }
        }
        let at = [hi[0] - 8.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
        let link = de_assembly::joints::link(world, shoulder, None).unwrap();
        let de_assembly::joints::Side::Inst(a, frame) = link.a else { unreachable!() };
        let axis = de_assembly::placement(world, p.entity(a).unwrap()).unwrap().then(&frame).at;
        let (ts, ms) = holding_torque(p, asm, shoulder, PAYLOAD, at);
        let (te, me) = holding_torque(p, asm, elbow, PAYLOAD, at);
        let (tw, _) = holding_torque(p, asm, wrist, PAYLOAD, at);
        let (ts0, _) = holding_torque(p, asm, shoulder, 0.0, at);
        let (te0, _) = holding_torque(p, asm, elbow, 0.0, at);
        let (tw0, _) = holding_torque(p, asm, wrist, 0.0, at);
        let cap = |stall: f64, t0: f64, t1: f64| (stall - t0) / ((t1 - t0) / PAYLOAD) * 1000.0;
        println!("{pose}: payload {:.0} mm out from the shoulder axis", at[0] - axis[0]);
        println!("  shoulder {:.3} N·m with 100 g ({:.3} arm alone, {:.0} g hanging): stall/holding {:.2} at 4.8 V, {:.2} at 6 V; payload to stall {:.0} g (4.8 V) / {:.0} g (6 V), with a 1.5 margin {:.0} g / {:.0} g", ts, ts0, ms * 1000.0, MG996R_STALL / ts, MG996R_STALL_6V / ts, cap(MG996R_STALL, ts0, ts), cap(MG996R_STALL_6V, ts0, ts), cap(MG996R_STALL / 1.5, ts0, ts), cap(MG996R_STALL_6V / 1.5, ts0, ts));
        println!("  elbow {:.3} N·m ({:.3} alone, {:.0} g hanging): stall/holding {:.2}; with a 1.5 margin {:.0} g", te, te0, me * 1000.0, MG996R_STALL / te, cap(MG996R_STALL / 1.5, te0, te));
        println!("  wrist {:.3} N·m ({:.3} alone): MG90S stall/holding {:.2}; with a 1.5 margin {:.0} g", tw, tw0, MG90S_STALL / tw, cap(MG90S_STALL / 1.5, tw0, tw));
        assert!(te * 1.5 < MG996R_STALL && tw < MG90S_STALL, "{pose}: the elbow (with a 1.5 margin) or the wrist cannot hold 100 g");
        if s == 90.0 {
            assert!(ts < MG996R_STALL_6V, "full reach: the shoulder cannot hold 100 g even at 6 V");
            full = Some((te, me));
        } else {
            assert!(ts * 1.5 < MG996R_STALL, "home: the shoulder cannot hold 100 g with a 1.5 margin");
        }
    }
    let (te, me) = full.unwrap();
    commit(p, "parameters.set", json!({ "target": shoulder, "values": { "angle": 90 } }));
    commit(p, "parameters.set", json!({ "target": elbow, "values": { "angle": 90 } }));
    let world = p.world();

    // The upper arm's left half: held at the shoulder servo's four tab inserts,
    // loaded at the elbow servo's four by what hangs from the elbow and its torque.
    let ua = p.entity_ids().into_iter().find(|e| p.get::<de_shape::ShapeProgram>(*e).is_some_and(|s| s.name == "Upper arm, left half")).unwrap();
    let program = de_shape::resolved_program(world, p.entity(ua).unwrap()).unwrap();
    let ev = de_shape::evaluated_at(&program, program.display_tolerance()).unwrap();
    let conn = |n: &str| ev.declared.connectors.iter().find(|c| c.name == n).unwrap_or_else(|| panic!("no connector {n}"));
    // Design → print coordinates (the part lies on its outer face: rotate 90° about X).
    let to_print = |v: [f64; 3]| [v[0], -v[2], v[1]];
    // At full reach the arm runs along world +X; world −Z (down) is the arm's design +X.
    let w = (me - 0.0) * 9.81 * 1000.0 / 1000.0; // N: what hangs from the elbow
    let t = te * 1000.0; // N·mm
    let fb = (t - 14.4 * w) / 49.5; // inboard holes (3, 4), up
    let fa = -w - fb; // outboard holes (1, 2)
    let down = |f: f64| to_print([-f, 0.0, 0.0]); // a world-up force f as a design-x force −f… in print coordinates
    let mut study = de_shape::fea::Study { resolution: 72, refine: true, ..Default::default() };
    for k in 1..=4 {
        let c = conn(&format!("Shoulder servo hole {k}"));
        study.bore_supports.push(de_shape::fea::Bore { at: c.at, axis: c.axis });
        let c = conn(&format!("Elbow servo hole {k}"));
        let f = if k <= 2 { fa } else { fb } / 2.0;
        study.bore_loads.push(de_shape::fea::BoreLoad { bore: de_shape::fea::Bore { at: c.at, axis: c.axis }, force: down(f) });
    }
    let r = de_shape::stress_study(&program, Some("Upper arm, left half"), "PLA", &study).unwrap();
    println!(
        "Upper arm, left half (PLA, typical properties): {:.2} MPa peak von Mises, {:.3} mm deflection, safety factor {:.1} against yield ({} N and {:.2} N·m at the elbow)",
        r.max_stress, r.max_displacement, r.safety, (w * 10.0).round() / 10.0, te
    );
    for n in &r.notes {
        println!("  note: {n}");
    }
    assert!(r.safety >= 2.0, "the upper arm is too weak: safety {:.2}", r.safety);
}
