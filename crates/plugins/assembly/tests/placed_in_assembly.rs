//! A shape placed on its own and then put into an assembly stands in the
//! project once, through its instance: not also where its own placement puts
//! it (where it would clash with itself).

use de_assembly::{AssemblyPlugin, instanced, interference};
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
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

fn id(v: &Value) -> StableId {
    serde_json::from_value(v["id"].clone()).unwrap()
}

#[test]
fn a_placed_shape_put_into_an_assembly_stands_only_through_its_instance() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin])));
    // A placed shape (as the PCB workspace keeps a board's model), and a lid as a definition.
    let board = id(&run(&mut p, "shape.create", json!({ "name": "Board", "source": "return box(40, 20, 1.6);", "params": [] })).unwrap());
    assert!(p.get::<de_solid::Placement>(board).is_some(), "the board is placed on its own");
    let lid = id(&run(&mut p, "shape.create", json!({ "name": "Lid", "source": "return box(40, 20, 2);", "params": [], "definition": true })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Sensor" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": board, "parent": asm, "name": "Board" })).unwrap();
    run(&mut p, "assembly.instance.create", json!({ "of": lid, "parent": asm, "name": "Lid", "at": [0, 0, 5] })).unwrap();
    run(&mut p, "assembly.instance.create", json!({ "of": asm, "name": "Sensor" })).unwrap();

    let used = instanced(p.world());
    assert!(used.contains(&board) && used.contains(&lid) && used.contains(&asm));

    // The whole project: the sensor, once; the board does not meet a copy of itself.
    let r = interference::report(p.world(), None, 0.0).unwrap();
    assert!(r.overlaps.is_empty(), "{:?}", r.overlaps.iter().map(|c| (&c.a, &c.b)).collect::<Vec<_>>());
    assert_eq!(r.parts, 2, "the board and the lid, each once");
}
