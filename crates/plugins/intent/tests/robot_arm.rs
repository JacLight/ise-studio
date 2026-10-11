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
}
