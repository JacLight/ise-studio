//! Named versions: evaluating the project at a version, comparing, restoring
//! (as a new change), and branching off and merging back.

use bevy_ecs::component::Component;
use de_core::{Actor, Command, CommandError, Envelope, Persist, Plugin, Project, Registry, Revision, SCHEMA_VERSION, StableId, Tx, Value, Version, bevy_ecs, json, schemars};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Doc {
    name: String,
    w: i64,
    h: i64,
    params: Vec<Param>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Param {
    key: String,
    value: f64,
}

impl Persist for Doc {
    const KIND: &'static str = "test.doc";
}

#[derive(Deserialize, schemars::JsonSchema)]
struct EditDoc {
    target: Option<StableId>,
    name: Option<String>,
    w: Option<i64>,
    h: Option<i64>,
    params: Option<Vec<Param>>,
    #[serde(default)]
    delete: bool,
}

impl Command for EditDoc {
    const NAME: &'static str = "test.doc";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let id = match self.target {
            Some(id) => id,
            None => tx.create(),
        };
        if self.delete {
            tx.delete(id)?;
            return Ok(json!({ "id": id }));
        }
        let mut d = tx.get::<Doc>(id).cloned().unwrap_or_default();
        if let Some(n) = self.name {
            d.name = n;
        }
        if let Some(w) = self.w {
            d.w = w;
        }
        if let Some(h) = self.h {
            d.h = h;
        }
        if let Some(p) = self.params {
            d.params = p;
        }
        tx.set(id, d)?;
        Ok(json!({ "id": id }))
    }
}

struct DocPlugin;

impl Plugin for DocPlugin {
    fn name(&self) -> &'static str {
        "test"
    }
    fn register(&self, r: &mut Registry) {
        r.component::<Doc>().command::<EditDoc>();
    }
}

fn registry() -> Arc<Registry> {
    Arc::new(Registry::with_plugins(&[&DocPlugin]))
}

fn copy(origin: &str) -> Project {
    let mut p = Project::new("shared", registry());
    p.set_origin(origin);
    p
}

fn run(p: &mut Project, user: &str, operation: &str, params: Value) -> Value {
    let e = Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().into(),
        expected_revision: p.revision(),
        operation: operation.into(),
        parameters: params,
        actor: Actor { user_id: user.into(), session_id: user.into() },
    };
    p.commit(e).unwrap_or_else(|e| panic!("{operation}: {e}")).output
}

fn new_doc(p: &mut Project, w: i64) -> StableId {
    let out = run(p, "ada", "test.doc", json!({ "name": "Bracket", "w": w, "h": 10 }));
    serde_json::from_value(out["id"].clone()).unwrap()
}

fn doc(p: &Project, id: StableId) -> Option<Doc> {
    p.get::<Doc>(id).cloned()
}

fn version(p: &Project, id: &Value) -> Version {
    p.get::<Version>(serde_json::from_value(id.clone()).unwrap()).cloned().unwrap()
}

/// Sends every log entry of `from` that `to` lacks (what peers do).
fn sync(from: &Project, to: &mut Project) {
    for w in from.log().to_vec() {
        to.receive(w).unwrap();
    }
}

#[test]
fn a_version_at_a_revision_evaluates_to_that_state() {
    let mut p = copy("engine-a");
    let id = new_doc(&mut p, 1); // r1
    run(&mut p, "ada", "test.doc", json!({ "target": id, "w": 2 })); // r2
    let marked = run(&mut p, "ada", "version.create", json!({ "name": "First cut", "note": "Sent to supplier", "author": "Ada", "revision": 1 }));
    run(&mut p, "ada", "test.doc", json!({ "target": id, "w": 3 }));
    let v = version(&p, &marked["id"]);
    assert_eq!((v.name.as_str(), v.note.as_str(), v.author.as_str()), ("First cut", "Sent to supplier", "Ada"));

    let then = p.as_of(&v.seen).unwrap();
    assert_eq!(doc(&then, id).unwrap().w, 1, "the version holds the state at revision 1");
    assert_eq!(then.revision(), Revision(1));
    assert_eq!(p.revision_of(&v.seen), Some(Revision(1)));
    let at2 = p.as_of(&p.vector_at(Revision(2)).unwrap()).unwrap();
    assert_eq!(doc(&at2, id).unwrap().w, 2);
    assert_eq!(doc(&p, id).unwrap().w, 3, "looking at a version leaves the project as it is");
    assert_eq!(p.as_of(&p.known().clone()).unwrap().export_state(), p.export_state(), "now evaluates to now");

    // Names are unique, and a revision that does not exist yet is refused.
    let e = Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().into(),
        expected_revision: p.revision(),
        operation: "version.create".into(),
        parameters: json!({ "name": "first CUT" }),
        actor: Actor { user_id: "ada".into(), session_id: "s".into() },
    };
    assert!(p.commit(e.clone()).is_err());
    assert!(p.commit(Envelope { command_id: "x".into(), parameters: json!({ "name": "Later", "revision": 99 }), ..e }).is_err());
}

#[test]
fn a_teammates_version_means_the_same_state_on_every_copy() {
    // Ada and Bo edit at the same time; their revision numbers differ.
    let (mut a, mut b) = (copy("engine-a"), copy("engine-b"));
    let id = new_doc(&mut a, 100);
    sync(&a, &mut b);
    run(&mut a, "ada", "test.doc", json!({ "target": id, "w": 120 }));
    run(&mut b, "bo", "test.doc", json!({ "target": id, "h": 60 }));
    run(&mut b, "bo", "test.doc", json!({ "target": id, "name": "Bracket B" }));
    sync(&a, &mut b);
    // Bo marks the version on his copy, then keeps working.
    let marked = run(&mut b, "bo", "version.create", json!({ "name": "Review" }));
    run(&mut b, "bo", "test.doc", json!({ "target": id, "w": 1 }));
    sync(&b, &mut a);
    run(&mut a, "ada", "test.doc", json!({ "target": id, "h": 2 }));

    let v = version(&a, &marked["id"]);
    let at_a = a.as_of(&v.seen).unwrap();
    let at_b = b.as_of(&v.seen).unwrap();
    assert_eq!(doc(&at_a, id), doc(&at_b, id));
    assert_eq!(doc(&at_a, id).unwrap(), Doc { name: "Bracket B".into(), w: 120, h: 60, params: vec![] });
}

#[test]
fn restore_brings_the_old_state_back_as_a_new_revision() {
    let mut p = copy("engine-a");
    let id = new_doc(&mut p, 10);
    run(&mut p, "ada", "test.doc", json!({ "target": id, "params": [{ "key": "width", "value": 5.0 }] }));
    let marked = run(&mut p, "ada", "version.create", json!({ "name": "v1.2" }));
    let v = version(&p, &marked["id"]);
    // After the version: one part changed, one added, and later the original deleted.
    let extra = new_doc(&mut p, 99);
    run(&mut p, "ada", "test.doc", json!({ "target": id, "w": 11, "params": [{ "key": "width", "value": 7.5 }] }));
    let before_restore = p.revision();
    let journal = p.journal().len();

    // Compare: what changed from the version to now.
    let then = p.as_of(&v.seen).unwrap();
    let diff = de_core::versions::compare(&then, &p, &["core.version"]);
    let added: Vec<_> = diff.iter().filter(|c| c.change == "added").map(|c| c.id).collect();
    assert_eq!(added, vec![extra]);
    let changed = diff.iter().find(|c| c.change == "changed").unwrap();
    assert_eq!(changed.name.as_deref(), Some("Bracket"));
    let paths: Vec<(&str, Value, Value)> = changed.fields.iter().map(|f| (f.path.as_str(), f.before.clone(), f.after.clone())).collect();
    assert!(paths.contains(&("/params/width/value", json!(5.0), json!(7.5))), "{paths:?}");
    assert!(paths.contains(&("/w", json!(10), json!(11))));
    assert!(!diff.iter().any(|c| c.id == serde_json::from_value::<StableId>(marked["id"].clone()).unwrap()), "versions are not design changes");

    run(&mut p, "ada", "test.doc", json!({ "target": id, "delete": true }));
    let then = p.as_of(&v.seen).unwrap();
    let actor = Actor { user_id: "ada".into(), session_id: "s".into() };
    let done = p.restore(&then, &actor, &["core.version"], json!({ "restored": "v1.2" })).unwrap();
    assert_eq!(done.revision.0, before_restore.0 + 2, "a new revision: nothing is rewritten");
    assert_eq!(p.journal().len(), journal + 2);
    assert_eq!(p.journal().last().unwrap().operation, "version.restore");
    assert_eq!(doc(&p, id).unwrap(), Doc { name: "Bracket".into(), w: 10, h: 10, params: vec![Param { key: "width".into(), value: 5.0 }] }, "the deleted part is back as it was, same id");
    assert!(doc(&p, extra).is_none(), "what was added since is gone");
    assert!(p.get::<Version>(serde_json::from_value(marked["id"].clone()).unwrap()).is_some(), "versions stay");
    // The state now equals the version's (apart from the version itself).
    assert!(de_core::versions::compare(&then, &p, &["core.version"]).is_empty());

    // And the restore is one undo step.
    p.undo(&actor).unwrap();
    assert!(doc(&p, id).is_none());
    assert_eq!(doc(&p, extra).unwrap().w, 99);
}

#[test]
fn a_branch_works_on_its_own_and_merges_back() {
    let mut main = copy("engine-a");
    let id = new_doc(&mut main, 100);
    let marked = run(&mut main, "ada", "version.create", json!({ "name": "v1" }));
    let v = version(&main, &marked["id"]);

    // Branch: a new project from the version's changes, stamping its own changes
    // as a separate engine so they never pass for the original's.
    let mut branch = Project::new("shared-branch", registry());
    for w in main.log_within(&v.seen) {
        branch.replay(w).unwrap();
    }
    branch.set_origin("engine-a~branch");
    assert_eq!(doc(&branch, id).unwrap().w, 100);
    run(&mut branch, "ada", "test.doc", json!({ "target": id, "h": 77 }));
    let added = new_doc(&mut branch, 5);
    run(&mut branch, "ada", "test.doc", json!({ "target": id, "w": 300 }));
    // Meanwhile the original moves on: one field the branch changed too, one it did not.
    run(&mut main, "ada", "test.doc", json!({ "target": id, "w": 200, "name": "Bracket v2" }));

    // Merge back: the branch's changes since the fork, through the team merge.
    let fresh = branch.log_beyond(&v.seen);
    assert_eq!(fresh.len(), 3, "only what the branch made after the fork");
    let mut conflicts = Vec::new();
    for w in fresh.clone() {
        for m in main.receive(w).unwrap() {
            conflicts.extend(m.conflicts);
        }
    }
    assert_eq!(main.waiting(), 0);
    let d = doc(&main, id).unwrap();
    assert_eq!((d.h, d.name.as_str()), (77, "Bracket v2"), "changes to different fields both land");
    assert_eq!(d.w, 300, "the later write to the same field wins, as between teammates");
    assert!(conflicts.iter().any(|c| c.path == "/w"), "and the clash is reported: {conflicts:?}");
    assert_eq!(doc(&main, added).unwrap().w, 5);

    // Merging again changes nothing; the merged changes survive a reload.
    let revision = main.revision();
    for w in fresh {
        main.receive(w).unwrap();
    }
    assert_eq!(main.revision(), revision);
    let mut reloaded = Project::new("shared", registry());
    reloaded.set_origin("engine-a");
    for w in main.log().to_vec() {
        reloaded.replay(w).unwrap();
    }
    assert_eq!(reloaded.export_state(), main.export_state());
}
