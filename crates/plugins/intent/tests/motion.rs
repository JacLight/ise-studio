//! "No clashes through the motion": an assembly joint swept through its range.

use de_assembly::AssemblyPlugin;
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_intent::{IntentPlugin, RequirementResult};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::Arc;

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin, &IntentPlugin])))
}

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

fn id(v: &Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

#[test]
fn an_arm_swinging_into_a_post_fails_until_its_limits_keep_it_clear() {
    let mut p = project();
    let arm = id(&run(&mut p, "shape.create", json!({ "name": "Arm", "definition": true, "params": [],
        "source": r#"connector("Pivot", { at: [0, 0, 0], axis: [0, 0, -1] }); return box(85, 10, 10).translate([-5, -5, 0]);"# })).unwrap());
    let post = id(&run(&mut p, "shape.create", json!({ "name": "Post", "definition": true, "params": [], "source": "return box(10, 10, 10);" })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Swing test" })).unwrap());
    let a = id(&run(&mut p, "assembly.instance.create", json!({ "of": arm, "parent": asm, "name": "Arm", "at": [0, 0, 0] })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": post, "parent": asm, "name": "Post", "at": [-5, 40, 0] })).unwrap();
    let j = id(&run(&mut p, "assembly.joint.create", json!({ "assembly": asm, "type": "hinge", "name": "Swing", "a": "ground", "b": { "instance": a, "connector": "Pivot" }, "limits": [0, 180] })).unwrap());
    let r = id(&run(&mut p, "intent.requirement.create", json!({ "rule": { "kind": "no_clashes_in_motion", "joint": j, "steps": 180 } })).unwrap());
    let result = p.get::<RequirementResult>(r).unwrap();
    assert!(!result.passed);
    assert_eq!(result.summary, "Arm hits Post as Swing moves, from 76° to 104°");
    // Limit the swing to 70°: clear, and it says how close it comes.
    run(&mut p, "assembly.joint.update", json!({ "target": j, "limits": [0, 70] })).unwrap();
    let result = p.get::<RequirementResult>(r).unwrap();
    assert!(result.passed, "{}", result.summary);
    assert!(result.summary.starts_with("Clear through 0°–70° of Swing; closest 3.98 mm (Arm – Post at 70°)"), "{}", result.summary);
    // Moving the post into the swept path is caught (the requirement covers the assembly).
    let post_inst = de_assembly::children(p.world(), asm).into_iter().find(|(_, i)| i.name == "Post").unwrap().0;
    run(&mut p, "assembly.instance.update", json!({ "target": post_inst, "at": [30, 20, 0] })).unwrap();
    assert!(!p.get::<RequirementResult>(r).unwrap().passed);
}
