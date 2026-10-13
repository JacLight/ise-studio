//! Requirements on circuit boards: routing constraints and net lengths,
//! re-checked as the copper changes and reported by the check gate.

use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_intent::{IntentPlugin, RequirementResult, check};
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

fn result(p: &Project, req: StableId) -> RequirementResult {
    p.get::<RequirementResult>(req).cloned().unwrap_or_else(|| panic!("no result: {:?}", p.status(req)))
}

#[test]
fn board_requirements_follow_the_copper() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&de_solid::SolidPlugin, &de_shape::ShapePlugin, &de_pcb::PcbPlugin, &IntentPlugin])));
    let b = id(&ok(&mut p, "pcb.board.create", json!({ "name": "t", "width": 40, "height": 20, "model": false })));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R1", "footprint": "R_0603", "at": [5, 10], "pins": { "2": "D0" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R2", "footprint": "R_0603", "at": [35, 10], "pins": { "1": "D0" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R3", "footprint": "R_0603", "at": [5, 14], "pins": { "2": "D1" } }));
    ok(&mut p, "pcb.component.create", json!({ "board": b, "reference": "R4", "footprint": "R_0603", "at": [35, 14], "pins": { "1": "D1" } }));
    ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R1.2", "to": "R2.1" }));
    ok(&mut p, "pcb.route.interactive", json!({ "board": b, "from": "R3.2", "to": [{ "at": [20, 17] }, { "at": "R4.1" }] }));
    let skew = id(&ok(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "pcb_length", "board": b, "nets": ["D?"], "max_skew": 0.1 } })));
    ok(&mut p, "pcb.length.group", json!({ "board": b, "name": "D", "nets": ["D?"], "max_skew": 0.1 }));
    let clean = id(&ok(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "pcb_constraints", "board": b } })));
    assert!(!result(&p, skew).passed, "{:?}", result(&p, skew));
    assert!(!result(&p, clean).passed, "the group's spread is a skew violation");
    assert!(!check(&p).ok);
    // Meandering D0 to D1's length satisfies both.
    let t = ok(&mut p, "pcb.tune", json!({ "board": b, "group": "D" }));
    assert_eq!(t["ok"], true, "{t}");
    let r = result(&p, skew);
    assert!(r.passed && r.measured < 0.01, "{r:?}");
    assert!(result(&p, clean).passed, "{:?}", result(&p, clean));
    let report = check(&p);
    assert_eq!((report.requirements_passing, report.requirements_total), (2, 2));
}
