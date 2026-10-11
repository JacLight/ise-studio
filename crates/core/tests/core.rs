//! Core behavior through a minimal plugin: `test.num` values, and a derived
//! `Total` = own value + totals of dependencies (negative values fail).

use de_core::{
    Actor, Command, CommandError, DependsOn, Derived, Envelope, ErrorCode, IdIndex, Persist, Plugin, Project,
    Registry, Revision, SCHEMA_VERSION, StableId, Status, Tx, Value, bevy_ecs, json, schemars,
};
use bevy_ecs::{component::Component, entity::Entity, world::World};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Num(i64);

impl Persist for Num {
    const KIND: &'static str = "test.num";
}

#[derive(Component, Clone, Debug, PartialEq)]
struct Total(i64);

#[derive(Deserialize, schemars::JsonSchema)]
struct SetNum {
    target: Option<StableId>,
    value: i64,
    #[serde(default)]
    depends_on: Vec<StableId>,
}

impl Command for SetNum {
    const NAME: &'static str = "test.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let id = match self.target {
            Some(id) => id,
            None => tx.create(),
        };
        tx.set(id, Num(self.value))?;
        for dep in self.depends_on {
            tx.add_dependency(id, dep)?;
        }
        if self.value == 999 {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "999 is rejected after writing"));
        }
        Ok(json!({ "id": id }))
    }
}

fn total(world: &World, entity: Entity) -> Option<Result<Total, String>> {
    let own = world.get::<Num>(entity)?.0;
    if own < 0 {
        return Some(Err("negative".into()));
    }
    let index = world.resource::<IdIndex>();
    let upstream: i64 = world
        .get::<DependsOn>(entity)
        .map(|d| d.0.iter().filter_map(|id| world.get::<Total>(index.get(*id)?)).map(|t| t.0).sum())
        .unwrap_or(0);
    Some(Ok(Total(own + upstream)))
}

struct TestPlugin;

impl Plugin for TestPlugin {
    fn name(&self) -> &'static str {
        "test"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<Num>().command::<SetNum>().derived(Derived {
            name: "total",
            inputs: &[Num::KIND],
            reads_dependencies: true,
            compute: total,
        });
    }
}

fn registry() -> Arc<Registry> {
    Arc::new(Registry::with_plugins(&[&TestPlugin]))
}

fn alice() -> Actor {
    Actor { user_id: "alice".into(), session_id: "s1".into() }
}

fn env(p: &Project, operation: &str, parameters: Value) -> Envelope {
    Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().into(),
        expected_revision: p.revision(),
        operation: operation.into(),
        parameters,
        actor: alice(),
    }
}

fn set(p: &mut Project, parameters: Value) -> StableId {
    let e = env(p, "test.set", parameters);
    let out = p.commit(e).unwrap().output;
    serde_json::from_value(out["id"].clone()).unwrap()
}

fn total_of(p: &Project, id: StableId) -> Option<i64> {
    p.get::<Total>(id).map(|t| t.0)
}

#[test]
fn derived_results_follow_dependencies() {
    let mut p = Project::new("p", registry());
    let a = set(&mut p, json!({ "value": 1 }));
    let b = set(&mut p, json!({ "value": 10, "depends_on": [a] }));
    let c = set(&mut p, json!({ "value": 100, "depends_on": [b] }));
    assert_eq!(total_of(&p, c), Some(111));

    set(&mut p, json!({ "target": a, "value": 5 }));
    assert_eq!((total_of(&p, b), total_of(&p, c)), (Some(15), Some(115)));
    assert_eq!(p.status(c), Some(&Status::Current));
}

#[test]
fn failure_blocks_dependents_and_recovers() {
    let mut p = Project::new("p", registry());
    let a = set(&mut p, json!({ "value": 1 }));
    let b = set(&mut p, json!({ "value": 2, "depends_on": [a] }));

    set(&mut p, json!({ "target": a, "value": -1 }));
    assert!(matches!(p.status(a), Some(Status::Failed(_))));
    assert_eq!(total_of(&p, a), None, "a failed result must not look current");
    assert!(matches!(p.status(b), Some(Status::Blocked(_))));

    set(&mut p, json!({ "target": a, "value": 3 }));
    assert_eq!(p.status(b), Some(&Status::Current));
    assert_eq!(total_of(&p, b), Some(5));
}

#[test]
fn deleting_a_dependency_blocks_and_undo_restores() {
    let mut p = Project::new("p", registry());
    let a = set(&mut p, json!({ "value": 1 }));
    let b = set(&mut p, json!({ "value": 2, "depends_on": [a] }));

    let e = env(&p, "core.entity.delete", json!({ "target": a }));
    p.commit(e).unwrap();
    assert!(p.entity(a).is_none());
    assert!(matches!(p.status(b), Some(Status::Blocked(_))));

    p.undo(&alice()).unwrap();
    assert_eq!(p.get::<Num>(a), Some(&Num(1)));
    assert_eq!(total_of(&p, b), Some(3));
}

#[test]
fn stale_revision_wrong_project_and_unknown_operation_are_rejected() {
    let mut p = Project::new("p", registry());
    set(&mut p, json!({ "value": 1 }));

    let mut stale = env(&p, "test.set", json!({ "value": 2 }));
    stale.expected_revision = Revision(0);
    assert_eq!(p.commit(stale).unwrap_err().code, ErrorCode::StaleRevision);

    let mut other = env(&p, "test.set", json!({ "value": 2 }));
    other.project_id = "q".into();
    assert_eq!(p.commit(other).unwrap_err().code, ErrorCode::WrongProject);

    assert_eq!(p.commit(env(&p, "nope", json!({}))).unwrap_err().code, ErrorCode::UnknownOperation);
    assert_eq!(p.commit(env(&p, "test.set", json!({ "value": "x" }))).unwrap_err().code, ErrorCode::InvalidParameters);
}

#[test]
fn repeated_command_id_is_idempotent() {
    let mut p = Project::new("p", registry());
    let e = env(&p, "test.set", json!({ "value": 1 }));
    let first = p.commit(e.clone()).unwrap();
    let again = p.commit(e).unwrap();
    assert_eq!(first, again);
    assert_eq!(p.entity_count(), 1);
    assert_eq!(p.revision(), Revision(1));
}

#[test]
fn failed_command_leaves_no_trace() {
    let mut p = Project::new("p", registry());
    let a = set(&mut p, json!({ "value": 1 }));
    let before = p.export_state();
    let err = p.commit(env(&p, "test.set", json!({ "value": 999, "depends_on": [a] }))).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParameters);
    assert_eq!(p.export_state(), before);
    assert_eq!(p.revision(), Revision(1));
}

#[test]
fn preview_reports_ops_without_changing_anything() {
    let mut p = Project::new("p", registry());
    let a = set(&mut p, json!({ "value": 1 }));
    let before = p.export_state();
    let preview = p.preview(&env(&p, "test.set", json!({ "target": a, "value": 7 }))).unwrap();
    assert_eq!(preview.ops.len(), 1);
    assert_eq!(p.export_state(), before);
    assert_eq!(total_of(&p, a), Some(1));
}

#[test]
fn undo_redo_and_undo_conflict() {
    let mut p = Project::new("p", registry());
    let a = set(&mut p, json!({ "value": 1 }));
    set(&mut p, json!({ "target": a, "value": 2 }));

    p.undo(&alice()).unwrap();
    assert_eq!(total_of(&p, a), Some(1));
    p.redo(&alice()).unwrap();
    assert_eq!(total_of(&p, a), Some(2));

    // Bob changes the same value; Alice's undo must not silently overwrite it.
    let mut bob = env(&p, "test.set", json!({ "target": a, "value": 50 }));
    bob.actor = Actor { user_id: "bob".into(), session_id: "s2".into() };
    p.commit(bob).unwrap();
    assert_eq!(p.undo(&alice()).unwrap_err().code, ErrorCode::UndoConflict);
    assert_eq!(total_of(&p, a), Some(50));
}

#[test]
fn peers_converge_through_change_sets() {
    let registry = registry();
    let mut host = Project::new("shared", Arc::clone(&registry));
    let mut peer = Project::new("shared", registry);

    let a = set(&mut host, json!({ "value": 1 }));
    set(&mut host, json!({ "value": 2, "depends_on": [a] }));
    for change in host.changes_since(peer.revision()) {
        let wire: de_core::WireChangeSet = serde_json::from_value(serde_json::to_value(change.to_wire()).unwrap()).unwrap();
        peer.apply_remote(wire).unwrap();
    }
    assert_eq!(peer.export_state(), host.export_state());
    assert_eq!(peer.revision(), host.revision());

    // A change set made on an old base is rejected rather than merged blindly.
    let old = host.journal()[0].to_wire();
    let mut replay = old.clone();
    replay.command_id = "different".into();
    assert_eq!(peer.apply_remote(replay).unwrap_err().code, ErrorCode::StaleRevision);
}

#[test]
fn command_schemas_are_published() {
    let r = registry();
    let names: Vec<_> = r.commands().map(|c| (c.plugin, c.name)).collect();
    assert!(names.contains(&("test", "test.set")));
    assert!(names.contains(&("core", "core.entity.delete")));
    let schema = serde_json::to_value(&r.commands().find(|c| c.name == "test.set").unwrap().schema).unwrap();
    assert!(schema["properties"]["value"].is_object());
}
