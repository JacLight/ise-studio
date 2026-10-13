//! Items brought in from another project (`link.*`), through a minimal
//! plugin whose parts name other entities in fields, in map keys and inside
//! text: copies get new ids everywhere, links follow their source.

use de_core::{Actor, Command, CommandError, Envelope, ErrorCode, Persist, Plugin, Project, Registry, SCHEMA_VERSION, StableId, Tx, Value, bevy_ecs, json, schemars};
use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Part {
    name: String,
    #[serde(default)]
    size: i64,
    /// Other parts it uses.
    #[serde(default)]
    uses: Vec<StableId>,
    /// Material by role (ids as map values) and notes keyed by id.
    #[serde(default)]
    materials: BTreeMap<String, StableId>,
    #[serde(default)]
    notes: BTreeMap<StableId, String>,
    /// A formula naming another entity's value inside text.
    #[serde(default)]
    formula: String,
}

impl Persist for Part {
    const KIND: &'static str = "test.part";
}

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct Mat {
    name: String,
    density: i64,
}

impl Persist for Mat {
    const KIND: &'static str = "test.mat";
}

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
struct At {
    x: i64,
}

impl Persist for At {
    const KIND: &'static str = "test.at";
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Put {
    target: Option<StableId>,
    #[serde(default)]
    part: Option<Part>,
    #[serde(default)]
    mat: Option<Mat>,
    #[serde(default)]
    at: Option<At>,
    #[serde(default)]
    delete: bool,
}

impl Command for Put {
    const NAME: &'static str = "test.put";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let id = match self.target {
            Some(id) => id,
            None => tx.create(),
        };
        if self.delete {
            tx.delete(id)?;
            return Ok(json!({ "id": id }));
        }
        if let Some(p) = self.part {
            tx.set(id, p)?;
        }
        if let Some(m) = self.mat {
            tx.set(id, m)?;
        }
        if let Some(a) = self.at {
            tx.set(id, a)?;
        }
        Ok(json!({ "id": id }))
    }
}

struct TestPlugin;

impl Plugin for TestPlugin {
    fn name(&self) -> &'static str {
        "test"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<Part>().component::<Mat>().component::<At>().command::<Put>();
    }
}

fn registry() -> Arc<Registry> {
    Arc::new(Registry::with_plugins(&[&TestPlugin]))
}

fn actor() -> Actor {
    Actor { user_id: "ada".into(), session_id: "s".into() }
}

fn run(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, CommandError> {
    let e = Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().to_string(),
        expected_revision: p.revision(),
        operation: operation.into(),
        parameters,
        actor: actor(),
    };
    p.commit(e).map(|c| c.output)
}

fn put(p: &mut Project, parameters: Value) -> StableId {
    serde_json::from_value(run(p, "test.put", parameters).unwrap()["id"].clone()).unwrap()
}

/// What the engine hands the commands: entities with every component as JSON.
fn snapshot(p: &Project, ids: &[StableId]) -> Value {
    let mut out = serde_json::Map::new();
    for id in ids {
        let comps: serde_json::Map<String, Value> = p.entity_state(*id).unwrap().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        out.insert(id.to_string(), Value::Object(comps));
    }
    Value::Object(out)
}

struct Source {
    p: Project,
    steel: StableId,
    bolt: StableId,
    bracket: StableId,
}

/// A bracket using a bolt, both of steel; the bracket's size follows the bolt's.
fn source() -> Source {
    let mut p = Project::new("src", registry());
    let steel = put(&mut p, json!({ "mat": { "name": "Steel", "density": 7850 } }));
    let bolt = put(&mut p, json!({ "part": { "name": "Bolt", "size": 8, "materials": { "body": steel } } }));
    let bracket = put(
        &mut p,
        json!({ "part": { "name": "Bracket", "size": 40, "uses": [bolt], "materials": { "body": steel }, "notes": { bolt.to_string(): "M8" }, "formula": format!("=#{bolt}.size * 5") }, "at": { "x": 100 } }),
    );
    Source { p, steel, bolt, bracket }
}

fn part(p: &Project, id: StableId) -> Part {
    p.get::<Part>(id).cloned().unwrap()
}

fn sid(v: &Value) -> StableId {
    serde_json::from_value(v.clone()).unwrap()
}

fn insert(target: &mut Project, s: &Source, linked: bool) -> Value {
    run(
        target,
        "link.insert",
        json!({
            "project": "src", "title": "Source", "revision": s.p.revision().0,
            "entities": snapshot(&s.p, &[s.steel, s.bolt, s.bracket]),
            "roots": [s.bracket], "linked": linked,
            "keep": { s.bracket.to_string(): { "test.at": ["x"] } },
            "rename": { s.bracket.to_string(): ["test.part"], s.bolt.to_string(): ["test.part"] },
            "reuse": ["test.mat"],
        }),
    )
    .unwrap()
}

#[test]
fn a_copy_gets_new_ids_and_every_reference_follows_them() {
    let s = source();
    let mut t = Project::new("dst", registry());
    let out = insert(&mut t, &s, false);
    assert!(out["link"].is_null());
    assert_eq!(out["created"], 3);
    let ids = &out["ids"];
    let (steel, bolt, bracket) = (sid(&ids[s.steel.to_string()]), sid(&ids[s.bolt.to_string()]), sid(&ids[s.bracket.to_string()]));
    assert!(steel != s.steel && bolt != s.bolt && bracket != s.bracket);
    let b = part(&t, bracket);
    assert_eq!(b.uses, vec![bolt]);
    assert_eq!(b.materials["body"], steel);
    assert_eq!(b.notes.keys().copied().collect::<Vec<_>>(), vec![bolt], "map keys are ids too");
    assert_eq!(b.formula, format!("=#{bolt}.size * 5"), "ids inside text");
    assert_eq!(part(&t, bolt).materials["body"], steel);
    assert_eq!(out["roots"], json!([bracket]));
    // Nothing of the source's ids is left anywhere.
    let all = serde_json::to_string(&t.export_state()).unwrap();
    for old in [s.steel, s.bolt, s.bracket] {
        assert!(!all.contains(&old.to_string()));
    }
    // A copy carries no link.
    assert!(t.entity_ids().iter().all(|id| t.entity_state(*id).unwrap().keys().all(|k| !k.starts_with("core.link"))));
}

#[test]
fn inserting_twice_never_collides_and_reuses_identical_materials() {
    let s = source();
    let mut t = Project::new("dst", registry());
    let a = insert(&mut t, &s, false);
    let b = insert(&mut t, &s, true);
    assert_eq!(a["created"], 3);
    // The second finds the same steel here and uses it.
    assert_eq!(b["created"], 2);
    assert_eq!(b["reused"], json!([a["ids"][s.steel.to_string()]]));
    let bracket2 = sid(&b["ids"][s.bracket.to_string()]);
    assert_eq!(part(&t, bracket2).materials["body"], sid(&a["ids"][s.steel.to_string()]));
    // Names that clash are renamed after the source, and say so.
    assert_eq!(part(&t, bracket2).name, "Bracket (Source)");
    let renamed = b["renamed"].as_array().unwrap();
    assert_eq!(renamed.len(), 2);
    assert!(renamed.iter().any(|r| r["from"] == "Bolt" && r["to"] == "Bolt (Source)"));
    // Five parts and one material, all distinct ids.
    assert_eq!(t.entity_count(), 3 + 2 + 1);

    // A linked material counts as one already here too.
    let mut t = Project::new("dst", registry());
    insert(&mut t, &s, true);
    let c = insert(&mut t, &s, false);
    assert_eq!((c["created"].clone(), c["reused"].as_array().unwrap().len()), (json!(2), 1));
}

#[test]
fn a_link_follows_its_source_and_keeps_local_placement_and_edits() {
    let mut s = source();
    let mut t = Project::new("dst", registry());
    let out = insert(&mut t, &s, true);
    let link = sid(&out["link"]);
    let bracket = sid(&out["ids"][s.bracket.to_string()]);
    let bolt = sid(&out["ids"][s.bolt.to_string()]);
    // Moved here; the bolt's size changed here (edited locally).
    run(&mut t, "test.put", json!({ "target": bracket, "at": { "x": 5 } })).unwrap();
    run(&mut t, "link.settings", json!({ "link": link, "local_edits": true })).unwrap();
    let mut b = part(&t, bolt);
    b.size = 10;
    run(&mut t, "test.put", json!({ "target": bolt, "part": b })).unwrap();

    // The source moves the bracket, resizes it, and adds a washer it uses.
    let washer = put(&mut s.p, json!({ "part": { "name": "Washer", "size": 9 } }));
    let mut sb = part(&s.p, s.bracket);
    sb.size = 60;
    sb.uses.push(washer);
    run(&mut s.p, "test.put", json!({ "target": s.bracket, "part": sb, "at": { "x": 300 } })).unwrap();

    let update = |t: &mut Project, s: &Source, ids: &[StableId]| {
        run(
            t,
            "link.update",
            json!({ "link": link, "revision": s.p.revision().0, "entities": snapshot(&s.p, ids), "keep": { s.bracket.to_string(): { "test.at": ["x"] } }, "reuse": ["test.mat"] }),
        )
        .unwrap()
    };
    let up = update(&mut t, &s, &[s.steel, s.bolt, s.bracket, washer]);
    assert_eq!(up["up_to_date"], false);
    assert_eq!(up["created"].as_array().unwrap().len(), 1);
    assert_eq!(up["conflicts"], json!([]));
    let b = part(&t, bracket);
    assert_eq!(b.size, 60, "the source's change arrived");
    let washer_here = sid(&up["created"][0]);
    assert_eq!(b.uses, vec![bolt, washer_here], "new references point at the new entity here");
    assert_eq!(t.get::<At>(bracket).unwrap().x, 5, "placement stays local");
    assert_eq!(part(&t, bolt).size, 10, "a local edit the source did not touch stays");

    // Nothing new: up to date, and no change is made.
    let before = t.revision();
    let again = run(&mut t, "link.update", json!({ "link": link, "revision": s.p.revision().0, "entities": snapshot(&s.p, &[s.steel, s.bolt, s.bracket, washer]), "keep": { s.bracket.to_string(): { "test.at": ["x"] } } })).unwrap();
    assert_eq!(again["up_to_date"], true);
    assert_eq!(t.revision(), before, "nothing to record");

    // Both change the bolt: the source wins, and it is reported.
    let mut sbolt = part(&s.p, s.bolt);
    sbolt.size = 12;
    run(&mut s.p, "test.put", json!({ "target": s.bolt, "part": sbolt })).unwrap();
    let up = update(&mut t, &s, &[s.steel, s.bolt, s.bracket, washer]);
    assert_eq!(up["conflicts"], json!([{ "id": bolt, "kind": "test.part" }]));
    assert_eq!(part(&t, bolt).size, 12);

    // The source drops the washer: it goes here too.
    let up = update(&mut t, &s, &[s.steel, s.bolt, s.bracket]);
    assert_eq!(up["removed"], json!([washer_here]));
    assert!(t.entity(washer_here).is_none());
}

#[test]
fn updates_on_two_copies_make_the_same_new_entity() {
    let mut s = source();
    let mut a = Project::new("dst", registry());
    let out = insert(&mut a, &s, true);
    let link = sid(&out["link"]);
    // A teammate's copy of the target, made from the same log.
    let mut b = Project::rebuild("dst".into(), registry(), "other", a.log().to_vec()).unwrap();
    let washer = put(&mut s.p, json!({ "part": { "name": "Washer", "size": 9 } }));
    let params = json!({ "link": link, "revision": s.p.revision().0, "entities": snapshot(&s.p, &[s.steel, s.bolt, s.bracket, washer]) });
    let ua = run(&mut a, "link.update", params.clone()).unwrap();
    let ub = run(&mut b, "link.update", params).unwrap();
    assert_eq!(ua["created"], ub["created"], "the same id on both copies");
}

#[test]
fn breaking_a_link_leaves_a_local_copy_and_undo_takes_an_insert_back() {
    let s = source();
    let mut t = Project::new("dst", registry());
    let out = insert(&mut t, &s, true);
    let link = sid(&out["link"]);
    let bracket = sid(&out["ids"][s.bracket.to_string()]);
    assert!(t.entity_state(bracket).unwrap().contains_key("core.linked"));
    // Broken through one of its items.
    let broke = run(&mut t, "link.break", json!({ "link": bracket })).unwrap();
    assert_eq!(broke["items"], 3);
    assert!(t.entity(link).is_none());
    assert!(!t.entity_state(bracket).unwrap().contains_key("core.linked"));
    assert_eq!(part(&t, bracket).name, "Bracket");
    assert!(run(&mut t, "link.update", json!({ "link": link, "entities": {} })).is_err());

    // Undo the break, then the insert: nothing is left.
    t.undo(&actor()).unwrap();
    assert!(t.entity(link).is_some());
    t.undo(&actor()).unwrap();
    assert_eq!(t.entity_count(), 0);
}

#[test]
fn derived_ids_are_stable_and_distinct() {
    let (a, b) = (StableId::new(), StableId::new());
    let x = StableId::new();
    assert_eq!(de_core::links::derived_id(a, x), de_core::links::derived_id(a, x));
    assert_ne!(de_core::links::derived_id(a, x), de_core::links::derived_id(b, x));
    assert_eq!(de_core::links::derived_id(a, x).0.get_version_num(), 7);
}

fn preview(p: &mut Project, operation: &str, parameters: Value) -> Result<Value, CommandError> {
    let e = Envelope {
        schema_version: SCHEMA_VERSION,
        command_id: StableId::new().to_string(),
        project_id: p.id().to_string(),
        expected_revision: p.revision(),
        operation: operation.into(),
        parameters,
        actor: actor(),
    };
    p.preview(&e).map(|c| c.output)
}

#[test]
fn linked_items_are_read_only_until_edited_locally_or_the_link_is_broken() {
    let s = source();
    let mut t = Project::new("dst", registry());
    let out = insert(&mut t, &s, true);
    let link = sid(&out["link"]);
    let bracket = sid(&out["ids"][s.bracket.to_string()]);
    let bolt = sid(&out["ids"][s.bolt.to_string()]);
    let mut b = part(&t, bolt);
    b.size = 10;
    let edit = json!({ "target": bolt, "part": b });

    // Refused, with what to do instead; nothing changed, and a preview says the same.
    let before = t.revision();
    let err = run(&mut t, "test.put", edit.clone()).unwrap_err();
    assert_eq!(err.code, ErrorCode::LinkedReadOnly);
    assert!(err.message.contains("“Bolt”") && err.message.contains("break the link") && err.message.contains("edit it locally"), "{}", err.message);
    assert!(err.message.contains(&link.to_string()), "names the link: {}", err.message);
    assert_eq!(part(&t, bolt).size, 8);
    assert_eq!(t.revision(), before);
    assert_eq!(preview(&mut t, "test.put", edit.clone()).unwrap_err().code, ErrorCode::LinkedReadOnly);

    // Where it is placed stays editable; a field beside it that is not kept does not.
    run(&mut t, "test.put", json!({ "target": bracket, "at": { "x": 7 } })).unwrap();
    assert_eq!(t.get::<At>(bracket).unwrap().x, 7);
    let mut br = part(&t, bracket);
    br.name = "Renamed".into();
    let err = run(&mut t, "test.put", json!({ "target": bracket, "part": br })).unwrap_err();
    assert!(err.message.starts_with("“Bracket” is linked from"), "named as it was: {}", err.message);

    // A command touching a linked item and a local one is refused whole.
    let mine = put(&mut t, json!({ "part": { "name": "Mine", "size": 1 } }));
    let err = run(&mut t, "core.batch", json!({ "steps": [
        { "operation": "test.put", "parameters": { "target": mine, "part": { "name": "Mine", "size": 2 } } },
        { "operation": "test.put", "parameters": edit.clone() },
    ] }))
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::LinkedReadOnly);
    assert_eq!(part(&t, mine).size, 1, "nothing of a refused batch is kept");

    // Edit locally: allowed (and updates merge, see above); off again: read-only again.
    let set = run(&mut t, "link.settings", json!({ "link": bolt, "local_edits": true })).unwrap();
    assert_eq!(set["local_edits"], true);
    assert_eq!(set["auto_update"], false, "the other setting stays");
    run(&mut t, "test.put", edit.clone()).unwrap();
    assert_eq!(part(&t, bolt).size, 10);
    run(&mut t, "link.settings", json!({ "link": link, "local_edits": false })).unwrap();
    let mut b = part(&t, bolt);
    b.size = 11;
    assert!(run(&mut t, "test.put", json!({ "target": bolt, "part": b.clone() })).is_err());
    assert!(run(&mut t, "link.settings", json!({ "link": link })).is_err(), "says what to set");

    // Removing a linked item is allowed (an update leaves it removed).
    run(&mut t, "test.put", json!({ "target": bracket, "delete": true })).unwrap();
    assert!(t.entity(bracket).is_none());

    // A broken link leaves an ordinary copy, free to change.
    run(&mut t, "link.break", json!({ "link": link })).unwrap();
    run(&mut t, "test.put", json!({ "target": bolt, "part": b })).unwrap();
    assert_eq!(part(&t, bolt).size, 11);

    // A copy was never locked.
    let mut c = Project::new("dst", registry());
    let out = insert(&mut c, &s, false);
    let bolt = sid(&out["ids"][s.bolt.to_string()]);
    run(&mut c, "test.put", json!({ "target": bolt, "part": { "name": "Bolt", "size": 3 } })).unwrap();
}
