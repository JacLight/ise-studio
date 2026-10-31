//! The automatic exploded view of a stacked product (a housing with gears in
//! it and a lid on top, a case round a board) lifts each layer straight up
//! clear of the ones under it, top first; a product spread out sideways still
//! comes apart from its middle.

use de_assembly::views::auto_steps;
use de_shape::stack::{PartMesh, lift_layers};
use de_assembly::AssemblyPlugin;
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

/// A box as a part: its top and bottom faces are all the stack reads.
fn block(min: [f64; 3], max: [f64; 3]) -> PartMesh {
    let face = |z: f64| [[[min[0], min[1], z], [max[0], min[1], z], [max[0], max[1], z]], [[min[0], min[1], z], [max[0], max[1], z], [min[0], max[1], z]]];
    PartMesh { triangles: [face(min[2]), face(max[2])].concat(), min, max }
}

#[test]
fn layers_lift_clear_of_the_ones_under_them() {
    // A housing 0–20, two planets 2–14 inside it, a carrier 12–26 over them, a knob 0–40 through the middle.
    let parts = [
        block([-50.0, -50.0, 0.0], [50.0, 50.0, 20.0]),
        block([10.0, -6.0, 2.0], [22.0, 6.0, 14.0]),
        block([-22.0, -6.0, 2.0], [-10.0, 6.0, 14.0]),
        block([-30.0, -30.0, 12.0], [30.0, 30.0, 26.0]),
        block([-8.0, -8.0, 0.0], [8.0, 8.0, 40.0]),
    ];
    let layers = lift_layers(&parts, 5.0).expect("stacked");
    let order: Vec<Vec<usize>> = layers.iter().map(|(m, _)| m.clone()).collect();
    assert_eq!(order, vec![vec![1, 2], vec![3], vec![4]], "planets, then the carrier, then the knob");
    // Planets clear the housing top (20) by the gap; the carrier clears the risen planets; the knob the carrier.
    assert_eq!(layers[0].1, 20.0 - 2.0 + 5.0);
    assert_eq!(layers[1].1, (14.0 + 23.0) - 12.0 + 5.0);
    assert_eq!(layers[2].1, (26.0 + 30.0) - 0.0 + 5.0);
    // Side by side is not a stack.
    assert!(lift_layers(&[block([0.0; 3], [10.0; 3]), block([40.0, 0.0, 0.0], [50.0, 10.0, 10.0])], 5.0).is_none());
}

#[test]
fn an_assembly_of_a_case_and_its_lid_comes_apart_upwards_lid_first() {
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin])));
    let base = id(&run(&mut p, "shape.create", json!({ "name": "Base", "source": "return box(60, 40, 20);", "params": [], "definition": true })).unwrap());
    let board = id(&run(&mut p, "shape.create", json!({ "name": "Board", "source": "return box(50, 30, 1.6);", "params": [], "definition": true })).unwrap());
    let lid = id(&run(&mut p, "shape.create", json!({ "name": "Lid", "source": "return box(60, 40, 3);", "params": [], "definition": true })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Case" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": base, "parent": asm, "name": "Base" })).unwrap();
    let b = id(&run(&mut p, "assembly.instance.create", json!({ "of": board, "parent": asm, "name": "Board", "at": [0, 0, 4] })).unwrap());
    let l = id(&run(&mut p, "assembly.instance.create", json!({ "of": lid, "parent": asm, "name": "Lid", "at": [0, 0, 20] })).unwrap());

    let steps = auto_steps(p.world(), Some(asm), 1.0).unwrap();
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert_eq!((steps[0].name.as_str(), &steps[0].instances), ("Lid", &vec![l]), "the lid comes off first");
    assert_eq!((steps[1].name.as_str(), &steps[1].instances), ("Board", &vec![b]));
    for s in &steps {
        assert!(s.by[0] == 0.0 && s.by[1] == 0.0 && s.by[2] > 0.0, "straight up: {:?}", s.by);
    }
    // The board rises above the base; the lid above the risen board.
    let (board_rise, lid_rise) = (steps[1].by[2], steps[0].by[2]);
    assert!(4.0 + board_rise > 20.0, "board clear of the base: {board_rise}");
    assert!(20.0 + lid_rise > 4.0 + 1.6 + board_rise, "lid clear of the board: {lid_rise}");
}
