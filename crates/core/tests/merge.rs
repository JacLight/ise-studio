//! Two members' copies of one project, editing at the same time with no host:
//! change sets cross in any order, and both copies end up identical.

use de_core::{Actor, Command, CommandError, Envelope, Persist, Plugin, Project, Registry, SCHEMA_VERSION, StableId, Tx, Value, WireChangeSet, bevy_ecs, json, schemars};
use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Doc {
    w: i64,
    h: i64,
    tags: Vec<String>,
}

impl Persist for Doc {
    const KIND: &'static str = "test.doc";
}

#[derive(Deserialize, schemars::JsonSchema)]
struct EditDoc {
    target: Option<StableId>,
    w: Option<i64>,
    h: Option<i64>,
    tags: Option<Vec<String>>,
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
        if let Some(w) = self.w {
            d.w = w;
        }
        if let Some(h) = self.h {
            d.h = h;
        }
        if let Some(t) = self.tags {
            d.tags = t;
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

fn copy(origin: &str) -> Project {
    let mut p = Project::new("shared", Arc::new(Registry::with_plugins(&[&DocPlugin])));
    p.set_origin(origin);
    p
}

/// Edits a copy; returns the change set to send to the others.
fn edit(p: &mut Project, user: &str, params: Value) -> WireChangeSet {
    let e = Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().into(),
        expected_revision: p.revision(),
        operation: "test.doc".into(),
        parameters: params,
        actor: Actor { user_id: user.into(), session_id: user.into() },
    };
    let before = p.log().len();
    p.commit(e).unwrap();
    // What a peer receives: the shared log's new entry (an edit that changed nothing sends nothing).
    match p.log().get(before) {
        Some(w) => w.clone(),
        None => WireChangeSet { ops: vec![], ..p.log().last().unwrap().clone() },
    }
}

fn doc(p: &Project, id: StableId) -> Option<Doc> {
    p.get::<Doc>(id).cloned()
}

fn id_of(c: &WireChangeSet) -> StableId {
    match &c.ops[0] {
        de_core::WireOp::Create { id } => *id,
        other => panic!("{other:?}"),
    }
}

/// Alice and Bob share a document at w 100, h 50.
fn shared() -> (Project, Project, StableId) {
    let (mut a, mut b) = (copy("engine-a"), copy("engine-b"));
    let start = edit(&mut a, "alice", json!({ "w": 100, "h": 50 }));
    b.merge_remote(start.clone()).unwrap();
    (a, b, id_of(&start))
}

#[test]
fn different_fields_merge_on_both_copies() {
    let (mut a, mut b, id) = shared();
    let from_a = edit(&mut a, "alice", json!({ "target": id, "w": 120 }));
    let from_b = edit(&mut b, "bob", json!({ "target": id, "h": 60 }));
    assert!(a.merge_remote(from_b).unwrap().conflicts.is_empty());
    assert!(b.merge_remote(from_a).unwrap().conflicts.is_empty());
    let want = Doc { w: 120, h: 60, tags: vec![] };
    assert_eq!((doc(&a, id), doc(&b, id)), (Some(want.clone()), Some(want)));
}

#[test]
fn the_same_field_converges_and_keeps_the_other_value() {
    let (mut a, mut b, id) = shared();
    let from_a = edit(&mut a, "alice", json!({ "target": id, "w": 1 }));
    let from_b = edit(&mut b, "bob", json!({ "target": id, "w": 2 }));
    let at_a = a.merge_remote(from_b).unwrap().conflicts;
    let at_b = b.merge_remote(from_a).unwrap().conflicts;
    assert_eq!(doc(&a, id), doc(&b, id), "both copies hold the same value");
    // Same clock: the engine name breaks the tie, the same way on both copies.
    assert_eq!(doc(&a, id).unwrap().w, 2);
    assert_eq!((at_a.len(), at_b.len()), (1, 1), "each copy reports the clash");
    assert_eq!((at_a[0].path.as_str(), at_a[0].kept.as_str()), ("/w", "theirs"));
    assert_eq!((at_a[0].ours.clone(), at_a[0].theirs.clone()), (json!(1), json!(2)), "Alice's 1 is not lost");
    assert_eq!(at_b[0].kept, "ours");
}

#[test]
fn a_later_edit_to_another_field_does_not_flip_the_winner() {
    // Bob edits w then h; Alice edits w once. Both copies must agree on w.
    let (mut a, mut b, id) = shared();
    let b1 = edit(&mut b, "bob", json!({ "target": id, "w": 7 }));
    let a1 = edit(&mut a, "alice", json!({ "target": id, "w": 9 }));
    let a2 = edit(&mut a, "alice", json!({ "target": id, "w": 9, "tags": ["x"] }));
    let b2 = edit(&mut b, "bob", json!({ "target": id, "h": 77 }));
    for c in [b1, b2] {
        a.merge_remote(c).unwrap();
    }
    for c in [a2, a1] {
        b.merge_remote(c).unwrap(); // out of order, too
    }
    assert_eq!(doc(&a, id), doc(&b, id), "{:?} vs {:?}", doc(&a, id), doc(&b, id));
    assert_eq!(doc(&a, id).unwrap().h, 77);
}

#[test]
fn lists_are_won_whole_and_duplicates_change_nothing() {
    let (mut a, mut b, id) = shared();
    let from_a = edit(&mut a, "alice", json!({ "target": id, "tags": ["steel"] }));
    let from_b = edit(&mut b, "bob", json!({ "target": id, "tags": ["painted"] }));
    let clash = a.merge_remote(from_b.clone()).unwrap().conflicts;
    b.merge_remote(from_a).unwrap();
    assert_eq!(doc(&a, id), doc(&b, id), "one list wins on both copies");
    assert_eq!(clash.len(), 1, "the other list is reported, not lost");
    let rev = a.revision();
    a.merge_remote(from_b).unwrap();
    assert_eq!(a.revision(), rev, "the same change set twice is applied once");
}

#[test]
fn deleting_what_someone_else_changed_keeps_it() {
    let (mut a, mut b, id) = shared();
    let gone = edit(&mut a, "alice", json!({ "target": id, "delete": true }));
    let changed = edit(&mut b, "bob", json!({ "target": id, "w": 300 }));
    let at_b = b.merge_remote(gone).unwrap().conflicts;
    assert_eq!(doc(&b, id).map(|d| d.w), Some(300), "Bob's edit survives the delete");
    assert_eq!(at_b[0].theirs, json!("deleted"));
    let at_a = a.merge_remote(changed).unwrap().conflicts;
    assert_eq!(doc(&a, id), doc(&b, id), "a change beats a delete on every copy");
    assert_eq!(at_a[0].ours, json!("deleted here"), "Alice is told Bob changed what she deleted");
    // Had Alice deleted after seeing Bob's change, the delete would win everywhere.
    let gone = edit(&mut a, "alice", json!({ "target": id, "delete": true }));
    assert!(b.merge_remote(gone).unwrap().conflicts.is_empty());
    assert!(doc(&a, id).is_none() && doc(&b, id).is_none());
}

fn trace(round: usize) -> bool {
    std::env::var("TRACE_ROUND").ok().and_then(|r| r.parse::<usize>().ok()) == Some(round)
}

/// Three copies make random edits (fields, tags, deletes) while apart, then
/// receive each other's change sets in fully random orders (held back until
/// what they build on arrives): every copy ends up identical, 300 runs over.
#[test]
fn three_copies_converge_whatever_the_order() {
    let mut seed = 0x9e3779b97f4a7c15u64;
    let mut rand = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let rounds: usize = std::env::var("FUZZ_ROUNDS").ok().and_then(|r| r.parse().ok()).unwrap_or(300);
    for round in 0..rounds {
        let mut copies = vec![copy("engine-a"), copy("engine-b"), copy("engine-c")];
        // Two shared documents to start.
        let mut ids = Vec::new();
        for _ in 0..2 {
            let c = edit(&mut copies[0], "alice", json!({ "w": 1, "h": 1 }));
            ids.push(id_of(&c));
            for other in copies.iter_mut().skip(1) {
                other.merge_remote(c.clone()).unwrap();
            }
        }
        // Everyone edits on their own, with partial syncs in between.
        let mut sent: Vec<Vec<WireChangeSet>> = vec![Vec::new(); 3];
        for _ in 0..12 {
            let who = rand(3);
            let id = ids[rand(2)];
            if copies[who].get::<Doc>(id).is_none() {
                continue;
            }
            let params = match rand(5) {
                0 => json!({ "target": id, "w": rand(100) }),
                1 => json!({ "target": id, "h": rand(100) }),
                2 => json!({ "target": id, "tags": [format!("t{}", rand(4))] }),
                3 if rand(4) == 0 => json!({ "target": id, "delete": true }),
                _ => json!({ "target": id, "w": rand(100), "h": rand(100) }),
            };
            let c = edit(&mut copies[who], ["alice", "bob", "cara"][who], params.clone());
            if c.ops.is_empty() {
                continue; // changed nothing: nothing to send
            }
            if trace(round) {
                eprintln!("EDIT by {who}: {params} {:?} seen {:?}", c.stamp.as_ref().map(|s| s.clock), c.seen);
            }
            sent[who].push(c);
            // Sometimes one copy hears from another early.
            if rand(3) == 0 {
                let (from, to) = (rand(3), rand(3));
                if from != to {
                    for c in sent[from].clone() {
                        if trace(round) {
                            eprintln!("EARLY {from}->{to} {:?}", c.stamp);
                        }
                        copies[to].receive(c).unwrap();
                    }
                }
            }
        }
        // Finally everyone receives everything, each in its own shuffled order.
        let all: Vec<WireChangeSet> = sent.iter().flatten().cloned().collect();
        for (i, copy) in copies.iter_mut().enumerate() {
            let mut order = all.clone();
            for k in (1..order.len()).rev() {
                order.swap(k, rand(k + 1));
            }
            for c in order {
                if trace(round) && i == 0 {
                    eprintln!("  wire {:?}: {:?}", c.stamp.as_ref().map(|s| (&s.origin, s.clock)), c.ops.iter().map(|o| serde_json::to_string(o).unwrap().chars().take(140).collect::<String>()).collect::<Vec<_>>());
                }
                let st = c.stamp.clone();
                let ms = copy.receive(c).unwrap_or_else(|e| panic!("round {round}, copy {i}: {e}"));
                if trace(round) {
                    eprintln!("copy {i} <- {:?}: {} merges, conflicts {:?}; now {:?}", st, ms.len(), ms.iter().flat_map(|m| m.conflicts.iter().map(|c| (c.path.clone(), c.kept.clone(), c.ours.clone(), c.theirs.clone()))).collect::<Vec<_>>(), ids.iter().map(|id| doc(copy, *id)).collect::<Vec<_>>());
                }
            }
            assert_eq!(copy.waiting(), 0, "round {round}: everything it needed arrived");
        }
        let state = |p: &Project| ids.iter().map(|id| doc(p, *id).map(|mut d| {
            d.tags.sort();
            d
        })).collect::<Vec<_>>();
        assert_eq!(state(&copies[0]), state(&copies[1]), "round {round}");
        // A restart rebuilds each copy from its saved log: exactly the same state.
        for c in &copies {
            let mut again = copy(c.origin());
            for w in c.log().to_vec() {
                again.replay(w).unwrap_or_else(|e| panic!("round {round}: replay: {e}"));
            }
            assert_eq!(state(&again), state(c), "round {round}: rebuilt from the log");
            assert_eq!(again.known(), c.known(), "round {round}: same version vector");
        }
        assert_eq!(state(&copies[1]), state(&copies[2]), "round {round}");
    }
}
