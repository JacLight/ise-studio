use de_assembly::comments::{Frame, anchor_world, frame, replies};
use de_assembly::{AssemblyPlugin, CommentReply, CommentThread};
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

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin])))
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

fn mm(v: f64) -> Value {
    json!({ "value": v, "unit": "mm" })
}

#[test]
fn frames_convert_both_ways_with_rotation() {
    let f = Frame::from_euler([100.0, -20.0, 5.0], [0.0, 0.0, 90.0]);
    let local = [10.0, 0.0, 3.0];
    let w = f.to_world(local);
    assert!(near(w, [100.0, -10.0, 8.0]), "x turns onto y, then moves: {w:?}");
    assert!(near(f.to_local(w), local), "and back");
    assert!(near(f.dir_to_world([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
    assert!(near(f.dir_to_local([0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]));
}

#[test]
fn a_thread_takes_replies_and_resolves() {
    let mut p = project();
    let part = id(&run(&mut p, "solid.box.create", json!({ "width": mm(100.0), "depth": mm(50.0), "height": mm(20.0) })).unwrap());
    let t = id(&run(&mut p, "comment.create", json!({ "target": part, "point": [50, 25, 20], "normal": [0, 0, 2], "text": "  Chamfer this edge?  ", "author": "Ada" })).unwrap());
    let th = p.get::<CommentThread>(t).unwrap().clone();
    assert_eq!((th.text.as_str(), th.author.as_str(), th.resolved), ("Chamfer this edge?", "Ada", false), "text trimmed, open");
    assert_eq!(th.anchor.normal, Some([0.0, 0.0, 1.0]), "normal made unit length");
    assert!(th.created > 0);

    let r1 = id(&run(&mut p, "comment.reply", json!({ "thread": t, "text": "1 mm is enough", "author": "Bo" })).unwrap());
    let r2 = id(&run(&mut p, "comment.reply", json!({ "thread": t, "text": "Agreed", "author": "Ada" })).unwrap());
    let thread_replies = replies(p.world(), t);
    assert_eq!(thread_replies.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![r1, r2], "oldest first");

    run(&mut p, "comment.resolve", json!({ "target": t, "author": "Bo" })).unwrap();
    let th = p.get::<CommentThread>(t).unwrap();
    assert!(th.resolved && th.resolved_by.as_deref() == Some("Bo") && th.resolved_at.is_some());
    run(&mut p, "comment.reopen", json!({ "target": t })).unwrap();
    let th = p.get::<CommentThread>(t).unwrap();
    assert!(!th.resolved && th.resolved_by.is_none());

    // Only the author edits or deletes.
    assert!(run(&mut p, "comment.edit", json!({ "target": r1, "text": "2 mm", "author": "Ada" })).is_err());
    run(&mut p, "comment.edit", json!({ "target": r1, "text": "2 mm", "author": "Bo" })).unwrap();
    let r = p.get::<CommentReply>(r1).unwrap();
    assert!(r.text == "2 mm" && r.edited.is_some());
    assert!(run(&mut p, "comment.delete", json!({ "target": t, "author": "Bo" })).is_err());

    // Deleting a thread takes its replies with it.
    let out = run(&mut p, "comment.delete", json!({ "target": t, "author": "Ada" })).unwrap();
    assert_eq!(out["replies"], 2);
    assert!(p.get::<CommentThread>(t).is_none() && p.get::<CommentReply>(r1).is_none() && p.get::<CommentReply>(r2).is_none());
    assert!(p.get::<de_solid::BoxFeature>(part).is_some(), "the part stays");
}

#[test]
fn bad_comments_are_refused() {
    let mut p = project();
    let part = id(&run(&mut p, "solid.box.create", json!({ "width": mm(10.0), "depth": mm(10.0), "height": mm(10.0) })).unwrap());
    let ok = json!({ "target": part, "point": [0, 0, 0], "text": "hi", "author": "Ada" });
    let with = |k: &str, v: Value| {
        let mut o = ok.clone();
        o[k] = v;
        o
    };
    assert!(run(&mut p, "comment.create", with("text", json!("   "))).is_err(), "empty text");
    assert!(run(&mut p, "comment.create", with("text", json!("x".repeat(4001)))).is_err(), "too long");
    assert!(run(&mut p, "comment.create", with("author", json!(""))).is_err(), "no author");
    assert!(run(&mut p, "comment.create", with("target", json!(StableId::new()))).is_err(), "missing part");
    assert!(run(&mut p, "comment.create", with("normal", json!([0, 0, 0]))).is_err(), "zero normal");
    assert!(run(&mut p, "comment.create", with("at", json!([1, 2, 3]))).is_err(), "point and at both");
    assert!(run(&mut p, "comment.create", json!({ "target": part, "text": "hi", "author": "Ada" })).is_err(), "nowhere");
    assert!(run(&mut p, "comment.create", with("markup", json!([{ "points": [[0, 0, 0]] }]))).is_err(), "one-point stroke");
    let component = id(&run(&mut p, "shape.create", json!({ "name": "C", "source": "return box(10, 10, 10);", "params": [], "definition": true })).unwrap());
    assert!(run(&mut p, "comment.create", with("target", json!(component))).is_err(), "a component definition is not placed");
    assert!(run(&mut p, "comment.reply", json!({ "thread": part, "text": "hi", "author": "Ada" })).is_err(), "reply to a non-thread");
    run(&mut p, "comment.create", ok).unwrap();
}

#[test]
fn a_pin_follows_its_part_when_moved() {
    let mut p = project();
    // A box: its own coordinates start at its placement.
    let part = id(&run(&mut p, "solid.box.create", json!({ "width": mm(100.0), "depth": mm(50.0), "height": mm(20.0) })).unwrap());
    let t = id(&run(&mut p, "comment.create", json!({ "target": part, "at": [100, 25, 10], "normal": [1, 0, 0], "text": "End face", "author": "Ada" })).unwrap());
    let pin = |p: &Project| anchor_world(p.world(), &p.get::<CommentThread>(t).unwrap().anchor).unwrap();
    assert!(near(pin(&p), [100.0, 25.0, 10.0]));
    run(&mut p, "solid.part.move", json!({ "target": part, "placement": { "x": mm(500.0), "y": mm(0.0), "z": mm(40.0) } })).unwrap();
    assert!(near(pin(&p), [600.0, 25.0, 50.0]), "moved with the box: {:?}", pin(&p));
    assert!(near(p.get::<CommentThread>(t).unwrap().anchor.point, [100.0, 25.0, 10.0]), "stored in the box's coordinates");

    // A placed shape: its placement is its program's origin (here inside the part).
    let shape = id(&run(&mut p, "shape.create", json!({ "name": "Puck", "source": "return box(40, 40, 10).translate(-20, -20, 0);", "params": [] })).unwrap());
    run(&mut p, "solid.part.move", json!({ "target": shape, "placement": { "x": mm(30.0), "y": mm(0.0), "z": mm(0.0) } })).unwrap();
    assert!(near(frame(p.world(), shape).unwrap().at, [30.0, 0.0, 0.0]));
    let s = id(&run(&mut p, "comment.create", json!({ "target": shape, "at": [30, 0, 10], "text": "Center", "author": "Ada" })).unwrap());
    assert!(near(p.get::<CommentThread>(s).unwrap().anchor.point, [0.0, 0.0, 10.0]), "the program's center");
    run(&mut p, "solid.part.move", json!({ "target": shape, "placement": { "x": mm(-100.0), "y": mm(0.0), "z": mm(0.0) } })).unwrap();
    assert!(near(anchor_world(p.world(), &p.get::<CommentThread>(s).unwrap().anchor).unwrap(), [-100.0, 0.0, 10.0]));

    // An instance: moved and turned, the pin turns with it.
    let bracket = component_of(&mut p);
    let inst = id(&run(&mut p, "assembly.instance.create", json!({ "of": bracket, "name": "Bracket", "at": [0, 0, 0] })).unwrap());
    let i = id(&run(&mut p, "comment.create", json!({ "target": inst, "at": [10, 0, 5], "text": "Hole here", "author": "Bo" })).unwrap());
    run(&mut p, "assembly.instance.update", json!({ "target": inst, "at": [200, 100, 0], "rotation": [0, 0, 90] })).unwrap();
    assert!(near(anchor_world(p.world(), &p.get::<CommentThread>(i).unwrap().anchor).unwrap(), [200.0, 110.0, 5.0]));

    // Its part gone, the pin has nowhere to be (the thread stays, listed as on a deleted part).
    run(&mut p, "core.entity.delete", json!({ "target": part })).unwrap();
    assert!(anchor_world(p.world(), &p.get::<CommentThread>(t).unwrap().anchor).is_none());
}

fn component_of(p: &mut Project) -> StableId {
    id(&run(p, "shape.create", json!({ "name": "Bracket", "source": "return box(20, 10, 10);", "params": [], "definition": true })).unwrap())
}

#[test]
fn markup_is_kept_in_the_parts_coordinates() {
    let mut p = project();
    let part = id(&run(&mut p, "solid.box.create", json!({ "width": mm(100.0), "depth": mm(50.0), "height": mm(20.0), "placement": { "x": mm(10.0), "y": mm(0.0), "z": mm(0.0) } })).unwrap());
    let t = id(&run(
        &mut p,
        "comment.create",
        json!({ "target": part, "at": [60, 25, 20], "text": "This way", "author": "Ada", "markup": [{ "points": [[20, 10, 20], [60, 25, 20]], "arrow": true }] }),
    )
    .unwrap());
    let th = p.get::<CommentThread>(t).unwrap();
    assert!(near(th.anchor.point, [50.0, 25.0, 20.0]));
    assert!(near(th.markup[0].points[0], [10.0, 10.0, 20.0]) && th.markup[0].arrow, "strokes converted like the pin");
}

#[test]
fn replies_carry_markup_in_the_threads_part_coordinates() {
    let mut p = project();
    let bracket = component_of(&mut p);
    let inst = id(&run(&mut p, "assembly.instance.create", json!({ "of": bracket, "name": "Bracket", "at": [200, 100, 0], "rotation": [0, 0, 90] })).unwrap());
    let t = id(&run(&mut p, "comment.create", json!({ "target": inst, "at": [200, 110, 5], "text": "Hole here", "author": "Ada" })).unwrap());

    // Bo answers with an arrow drawn in project coordinates: stored in the instance's own.
    let r = id(&run(&mut p, "comment.reply", json!({ "thread": t, "author": "Bo", "text": "", "markup_at": [{ "points": [[200, 100, 5], [200, 110, 5]], "arrow": true }] })).unwrap());
    let reply = p.get::<CommentReply>(r).unwrap().clone();
    assert_eq!(reply.text, "", "markup alone is a reply");
    assert!(near(reply.markup[0].points[0], [0.0, 0.0, 5.0]) && near(reply.markup[0].points[1], [10.0, 0.0, 5.0]) && reply.markup[0].arrow, "{:?}", reply.markup);

    // When the instance moves, the reply's drawing goes with it (same frame as the pin).
    run(&mut p, "assembly.instance.update", json!({ "target": inst, "at": [0, 0, 0], "rotation": [0, 0, 0] })).unwrap();
    let f = frame(p.world(), inst).unwrap();
    assert!(near(f.to_world(p.get::<CommentReply>(r).unwrap().markup[0].points[1]), [10.0, 0.0, 5.0]));

    // Strokes in part coordinates are kept as given; the original poster adds markup later.
    run(&mut p, "comment.markup", json!({ "target": t, "author": "Ada", "markup": [{ "points": [[1, 2, 3], [4, 5, 6]] }] })).unwrap();
    let th = p.get::<CommentThread>(t).unwrap();
    assert_eq!(th.markup.len(), 1);
    assert!(near(th.markup[0].points[1], [4.0, 5.0, 6.0]) && !th.markup[0].arrow);
    run(&mut p, "comment.markup", json!({ "target": r, "author": "Bo", "markup_at": [{ "points": [[3, 0, 0], [3, 3, 0]] }] })).unwrap();
    assert_eq!(p.get::<CommentReply>(r).unwrap().markup.len(), 2);

    // Only the author draws on a comment; nothing to draw is refused; a reply with no text keeps a stroke.
    assert!(run(&mut p, "comment.markup", json!({ "target": t, "author": "Bo", "markup": [{ "points": [[0, 0, 0], [1, 1, 1]] }] })).is_err());
    assert!(run(&mut p, "comment.markup", json!({ "target": t, "author": "Ada" })).is_err());
    assert!(run(&mut p, "comment.markup", json!({ "target": r, "author": "Bo", "replace": true })).is_err());
    assert!(run(&mut p, "comment.reply", json!({ "thread": t, "author": "Bo", "text": " " })).is_err(), "no text and no markup");
}
