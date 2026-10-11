use de_assembly::{AssemblyPlugin, Group};
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
fn a_group_holds_items_without_changing_them_and_ungrouping_leaves_them() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin])));
    let shape = |p: &mut Project, name: &str| id(&run(p, "shape.create", json!({ "name": name, "source": "return box(10, 10, 10);", "params": [] })).unwrap());
    let (a, b, c) = (shape(&mut p, "A"), shape(&mut p, "B"), shape(&mut p, "C"));
    assert!(run(&mut p, "assembly.group.create", json!({ "name": "One", "members": [a] })).is_err(), "one item is not a group");
    assert!(run(&mut p, "assembly.group.create", json!({ "name": "Ghost", "members": [a, StableId::new()] })).is_err(), "missing items are refused");
    let g = id(&run(&mut p, "assembly.group.create", json!({ "name": "Desk", "members": [a, b, a] })).unwrap());
    assert_eq!(p.get::<Group>(g).unwrap().members, vec![a, b], "duplicates dropped");
    run(&mut p, "assembly.group.update", json!({ "target": g, "name": "Desk set", "members": [a, b, c] })).unwrap();
    let grp = p.get::<Group>(g).unwrap();
    assert_eq!((grp.name.as_str(), grp.members.len()), ("Desk set", 3));
    run(&mut p, "core.entity.delete", json!({ "target": g })).unwrap();
    assert!(p.get::<Group>(g).is_none());
    assert!([a, b, c].iter().all(|x| p.get::<de_shape::ShapeProgram>(*x).is_some()), "items stay after ungrouping");
}
