use de_assembly::{AssemblyPlugin, AssemblyStats, Instance, InstanceConfigured};
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Status, Value, json};
use de_shape::{ShapeConfigurations, ShapePlugin, ShapeProgram, ShapeSolid};
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

fn volume(p: &Project, id: StableId) -> f64 {
    p.get::<ShapeSolid>(id).unwrap().stats.volume_mm3
}

fn instance_volume(p: &Project, id: StableId) -> Option<f64> {
    p.get::<InstanceConfigured>(id).unwrap().0.as_ref().map(|c| c.stats.volume_mm3)
}

/// A square "bolt": d × d across, len long, so its volume is d²·len.
fn bolt(p: &mut Project, definition: bool) -> StableId {
    id(&run(
        p,
        "shape.create",
        json!({
            "name": "Bolt",
            "source": "return box(p.d, p.d, p.len);",
            "params": [
                { "key": "d", "label": "Diameter", "value": 6, "unit": "mm", "min": 1 },
                { "key": "len", "label": "Length", "value": 20, "unit": "mm", "min": 1 }
            ],
            "definition": definition
        }),
    )
    .unwrap())
}

fn sizes(target: StableId) -> Value {
    json!({
        "target": target,
        "params": ["d", "len"],
        "rows": [
            { "name": "M6×20", "values": { "d": 6, "len": 20 } },
            { "name": "M8×30", "values": { "d": 8, "len": 30 }, "description": "Longer" },
            { "name": "M10×40", "values": { "d": 10, "len": 40 } }
        ]
    })
}

#[test]
fn activating_a_configuration_resizes_the_shape_and_undo_puts_it_back() {
    let mut p = project();
    let b = bolt(&mut p, false);
    run(&mut p, "shape.config.set_table", sizes(b)).unwrap();
    assert!((volume(&p, b) - 720.0).abs() < 1e-6, "a table alone changes nothing");
    assert_eq!(p.get::<ShapeConfigurations>(b).unwrap().rows.len(), 3);

    run(&mut p, "shape.config.activate", json!({ "target": b, "name": "M8×30" })).unwrap();
    assert!((volume(&p, b) - 8.0 * 8.0 * 30.0).abs() < 1e-6, "M8×30 is 8 × 8 × 30");
    assert_eq!(p.get::<ShapeConfigurations>(b).unwrap().active.as_deref(), Some("M8×30"));
    assert_eq!(de_shape::resolved_program(p.world(), p.entity(b).unwrap()).unwrap().name, "Bolt (M8×30)", "named for its configuration");

    let err = run(&mut p, "shape.config.activate", json!({ "target": b, "name": "M12" })).unwrap_err();
    assert!(err.message.contains("M6×20") && err.message.contains("M10×40"), "says which there are: {}", err.message);

    p.undo(&Actor { user_id: "u".into(), session_id: "s".into() }).unwrap();
    assert!((volume(&p, b) - 720.0).abs() < 1e-6, "undo restores the values");
    assert_eq!(p.get::<ShapeConfigurations>(b).unwrap().active, None, "and the active configuration");

    // Editing the active row's values re-applies them.
    run(&mut p, "shape.config.activate", json!({ "target": b, "name": "M10×40" })).unwrap();
    let mut t = sizes(b);
    t["rows"][2]["values"]["len"] = json!(50);
    run(&mut p, "shape.config.set_table", t).unwrap();
    assert!((volume(&p, b) - 10.0 * 10.0 * 50.0).abs() < 1e-6, "the active configuration follows its row");

    // Bad tables are refused with reasons.
    let mut t = sizes(b);
    t["rows"][1]["values"]["d"] = json!(0.5);
    assert!(run(&mut p, "shape.config.set_table", t).unwrap_err().message.contains("outside"), "values stay in range");
    let mut t = sizes(b);
    t["rows"][1]["name"] = json!("M6×20");
    assert!(run(&mut p, "shape.config.set_table", t).unwrap_err().message.contains("two configurations"));
    let mut t = sizes(b);
    t["rows"][0]["values"].as_object_mut().unwrap().remove("len");
    assert!(run(&mut p, "shape.config.set_table", t).unwrap_err().message.contains("no value for Length"));
    let mut t = sizes(b);
    t["params"] = json!(["d", "len", "pitch"]);
    assert!(run(&mut p, "shape.config.set_table", t).is_err(), "columns must be parameters");

    // An empty table removes it.
    run(&mut p, "shape.config.set_table", json!({ "target": b, "params": [], "rows": [] })).unwrap();
    assert!(p.get::<ShapeConfigurations>(b).is_none());
}

#[test]
fn instances_of_one_definition_show_different_configurations() {
    let mut p = project();
    let b = bolt(&mut p, true);
    run(&mut p, "shape.config.set_table", sizes(b)).unwrap();
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Kit" })).unwrap());
    let place = |p: &mut Project, name: &str, x: f64, config: Option<&str>| id(&run(p, "assembly.instance.create", json!({ "of": b, "parent": asm, "name": name, "at": [x, 0, 0], "config": config })).unwrap());
    let plain = place(&mut p, "A", 0.0, None);
    let m8 = place(&mut p, "B", 50.0, Some("M8×30"));
    let m10 = place(&mut p, "C", 100.0, Some("M10×40"));
    assert!(run(&mut p, "assembly.instance.create", json!({ "of": b, "parent": asm, "name": "D", "config": "M3" })).is_err(), "only its own configurations");

    assert_eq!(instance_volume(&p, plain), None, "no configuration: the definition as it is");
    assert!((instance_volume(&p, m8).unwrap() - 1920.0).abs() < 1e-6);
    assert!((instance_volume(&p, m10).unwrap() - 4000.0).abs() < 1e-6);
    assert!((volume(&p, b) - 720.0).abs() < 1e-6, "the definition is unchanged");

    // The assembly counts each instance as it shows: bounds reach the M10's 40 mm.
    let stats = p.get::<AssemblyStats>(asm).unwrap();
    assert!((stats.bounds_max[2] - 40.0).abs() < 1e-6, "{:?}", stats.bounds_max);
    assert!((stats.bounds_max[0] - 110.0).abs() < 1e-6, "{:?}", stats.bounds_max);
    let names: Vec<String> = de_assembly::occurrences(p.world(), asm).into_iter().map(|o| o.names.last().unwrap().clone()).collect();
    assert!(names.contains(&"Bolt (M8×30)".to_string()) && names.contains(&"Bolt".to_string()), "parts lists name the configuration: {names:?}");

    // Switching an instance's configuration (journaled, undoable).
    run(&mut p, "assembly.instance.update", json!({ "target": plain, "config": "M10×40" })).unwrap();
    assert!((instance_volume(&p, plain).unwrap() - 4000.0).abs() < 1e-6);
    p.undo(&Actor { user_id: "u".into(), session_id: "s".into() }).unwrap();
    assert_eq!(instance_volume(&p, plain), None, "undo puts it back");

    // Changing the definition reaches every configuration it is shown in.
    run(&mut p, "shape.update", json!({ "target": b, "source": "return box(p.d, p.d, p.len + 10);" })).unwrap();
    assert!((instance_volume(&p, m8).unwrap() - 8.0 * 8.0 * 40.0).abs() < 1e-6);

    // Renaming a configuration carries its instances; deleting one in use is refused.
    let mut t = sizes(b);
    t["rows"][1]["name"] = json!("M8×30 zinc");
    t["renamed"] = json!({ "M8×30": "M8×30 zinc" });
    run(&mut p, "shape.config.set_table", t).unwrap();
    assert_eq!(p.get::<Instance>(m8).unwrap().config.as_deref(), Some("M8×30 zinc"));
    assert_eq!(p.get::<Status>(m8), Some(&Status::Current));
    let mut t = sizes(b);
    t["rows"].as_array_mut().unwrap().remove(2);
    let err = run(&mut p, "shape.config.set_table", t).unwrap_err();
    assert!(err.message.contains("M10×40") && err.message.contains('C'), "says who shows it: {}", err.message);

    // Making one unique keeps what it showed, as its own definition's active configuration.
    let out = run(&mut p, "assembly.instance.make_unique", json!({ "target": m10 })).unwrap();
    let copy: StableId = serde_json::from_value(out["definition"].clone()).unwrap();
    assert_eq!(p.get::<Instance>(m10).unwrap().config, None);
    assert_eq!(p.get::<ShapeConfigurations>(copy).unwrap().active.as_deref(), Some("M10×40"));
    assert!((volume(&p, copy) - 10.0 * 10.0 * 50.0).abs() < 1e-6);
}

#[test]
fn a_linked_parameter_stays_driven_and_the_table_cannot_set_it() {
    let mut p = project();
    let b = id(&run(
        &mut p,
        "shape.create",
        json!({
            "name": "Bolt",
            "source": "return box(p.head, p.d, p.len);",
            "params": [
                { "key": "d", "label": "Diameter", "value": 6, "unit": "mm" },
                { "key": "len", "label": "Length", "value": 20, "unit": "mm" },
                { "key": "head", "label": "Head", "value": 12, "unit": "mm", "expr": "=d * 2" }
            ]
        }),
    )
    .unwrap());
    let err = run(&mut p, "shape.config.set_table", json!({ "target": b, "params": ["d", "head"], "rows": [{ "name": "S", "values": { "d": 4, "head": 9 } }] })).unwrap_err();
    assert!(err.message.contains("Head follows") && err.message.contains("Unlink"), "explains why: {}", err.message);
    let err = run(&mut p, "shape.config.set_table", json!({ "target": b, "params": ["d"], "rows": [{ "name": "S", "values": { "d": 4, "head": 9 } }] })).unwrap_err();
    assert!(err.message.contains("Head follows"), "not even as an extra value: {}", err.message);

    // A formula reading a configured parameter follows it.
    run(&mut p, "shape.config.set_table", json!({ "target": b, "params": ["d", "len"], "rows": [{ "name": "S", "values": { "d": 4, "len": 10 } }, { "name": "L", "values": { "d": 10, "len": 30 } }] })).unwrap();
    run(&mut p, "shape.config.activate", json!({ "target": b, "name": "L" })).unwrap();
    assert!((volume(&p, b) - 20.0 * 10.0 * 30.0).abs() < 1e-6, "head = 2 × d in L");
    let s = de_shape::configs::configured_program(p.world(), p.entity(b).unwrap(), "S").unwrap();
    assert_eq!(s.params.iter().find(|q| q.key == "head").unwrap().value, 8.0, "and in S");

    // A column cannot be linked, nor dropped by a rewrite, while it is a column.
    let err = run(&mut p, "shape.link_param", json!({ "target": b, "key": "len", "expr": "=d * 3" })).unwrap_err();
    assert!(err.message.contains("column"), "{}", err.message);
    let err = run(&mut p, "shape.update", json!({ "target": b, "params": [
        { "key": "d", "label": "Diameter", "value": 6, "unit": "mm" },
        { "key": "head", "label": "Head", "value": 12, "unit": "mm", "expr": "=d * 2" }
    ], "source": "return box(p.head, p.d, 5);" })).unwrap_err();
    assert!(err.message.contains("len") && err.message.contains("table"), "{}", err.message);
    assert!(p.get::<ShapeProgram>(b).unwrap().params.iter().any(|q| q.key == "len"));
}

#[test]
fn a_configuration_can_leave_a_finish_off() {
    use std::f64::consts::PI;
    let mut p = project();
    let block = id(&run(
        &mut p,
        "shape.create",
        json!({
            "name": "Block", "source": "return box(p.w, 40, 10);",
            "params": [{ "key": "w", "label": "Width", "value": 60 }, { "key": "fillet1", "label": "Fillet 1", "value": 4 }],
        }),
    )
    .unwrap());
    let pick = json!({ "at": [1, 1, 0.5], "kind": "line", "dir": [0, 0, 1] });
    run(&mut p, "shape.update", json!({ "target": block, "finishes": [{ "kind": "fillet", "key": "fillet1", "edges": [pick] }] })).unwrap();
    let corner = (16.0 - PI * 16.0 / 4.0) * 10.0;
    assert!((volume(&p, block) - (24000.0 - corner)).abs() < 0.5);
    let table = |suppress: &str| {
        json!({ "target": block, "params": ["w"], "rows": [
            { "name": "Rounded", "values": { "w": 60 } },
            { "name": "Sharp", "values": { "w": 60 }, "suppress": [suppress] }
        ] })
    };
    assert!(run(&mut p, "shape.config.set_table", table("chamfer9")).unwrap_err().message.contains("chamfer9"), "only finishes it has");
    run(&mut p, "shape.config.set_table", table("fillet1")).unwrap();
    run(&mut p, "shape.config.activate", json!({ "target": block, "name": "Sharp" })).unwrap();
    assert!((volume(&p, block) - 24000.0).abs() < 1e-6, "Sharp has no fillet");
    run(&mut p, "shape.config.activate", json!({ "target": block, "name": "Rounded" })).unwrap();
    assert!((volume(&p, block) - (24000.0 - corner)).abs() < 0.5, "Rounded has it back");
    // Removing the finish drops it from the rows that left it off.
    run(&mut p, "shape.update", json!({ "target": block, "finishes": [] })).unwrap();
    assert!(p.get::<ShapeConfigurations>(block).unwrap().rows[1].suppress.is_empty());
}

/// An enclosure: a 100 × 60 × 10 base, a 100 × 60 × 4 lid hinged to it, a
/// 20 × 20 × 5 fan spun by a motor and a 20 × 20 × 1 guard over it, all
/// steel (7850 kg/m³). "Basic" leaves the fan and its guard out; "Pro" has them.
fn enclosure(p: &mut Project, definition: bool) -> StableId {
    let steel = id(&run(p, "solid.material.create", json!({ "name": "Steel", "density": { "value": 7850, "unit": "kg/m3" } })).unwrap());
    let e = id(&run(
        p,
        "shape.create",
        json!({
            "name": "Enclosure",
            "source": r#"
                joint("hinge", "Lid", "Base", { at: [0, 0, 10], axis: [1, 0, 0], name: "Lid hinge" });
                joint("hinge", "Fan", "ground", { at: [50, 30, 10], axis: [0, 0, 1], name: "Fan spin" });
                drive("Fan spin", { speed: 1200 });
                return [
                    part("Base", box(100, 60, p.h)),
                    part("Lid", box(100, 60, 4).translate(0, 0, p.h)),
                    part("Fan", box(20, 20, 5).translate(40, 20, p.h + 4)),
                    part("Fan guard", box(20, 20, 1).translate(40, 20, p.h + 9)),
                ];
            "#,
            "params": [{ "key": "h", "label": "Height", "value": 10, "unit": "mm", "min": 5 }],
            "material": steel,
            "definition": definition
        }),
    )
    .unwrap());
    run(
        p,
        "shape.config.set_table",
        json!({ "target": e, "params": ["h"], "rows": [
            { "name": "Basic", "values": { "h": 10 }, "parts_off": ["Fan", "Fan guard"] },
            { "name": "Pro", "values": { "h": 10 } }
        ] }),
    )
    .unwrap();
    e
}

const BASE: f64 = 100.0 * 60.0 * 10.0;
const LID: f64 = 100.0 * 60.0 * 4.0;
const FAN: f64 = 20.0 * 20.0 * 5.0;
const GUARD: f64 = 20.0 * 20.0 * 1.0;

fn part_names(stats: &de_shape::ShapeStats) -> Vec<&str> {
    stats.parts.iter().map(|q| q.name.as_str()).collect()
}

#[test]
fn a_configuration_leaves_parts_out_of_mass_parts_list_and_exports() {
    let mut p = project();
    let e = enclosure(&mut p, false);
    let all = BASE + LID + FAN + GUARD;
    assert!((volume(&p, e) - all).abs() < 1e-6, "no configuration active: every part");

    run(&mut p, "shape.config.activate", json!({ "target": e, "name": "Basic" })).unwrap();
    let stats = &p.get::<ShapeSolid>(e).unwrap().stats;
    assert_eq!(part_names(stats), ["Base", "Lid"], "Basic has no fan and no guard");
    assert!((stats.volume_mm3 - (BASE + LID)).abs() < 1e-6);
    assert_eq!(stats.left_out.parts, ["Fan", "Fan guard"], "and says so");
    assert_eq!(stats.left_out.all, ["Base", "Lid", "Fan", "Fan guard"], "naming every part, for the picker");
    let kg = p.get::<de_solid::MassProperties>(e).unwrap().mass_kg.unwrap();
    assert!((kg - (BASE + LID) / 1e9 * 7850.0).abs() < 1e-9, "mass of base and lid only: {kg}");
    assert_eq!(de_shape::part_masses(p.world(), p.entity(e).unwrap()).unwrap().len(), 2, "two lines in the parts list");

    // The fan's joint, and the drive on it, go with it: said plainly, and the
    // motion still solves with the lid's hinge.
    assert!(stats.left_out.dropped.iter().any(|d| d.contains("Fan spin")), "{:?}", stats.left_out.dropped);
    assert!(stats.left_out.dropped.iter().any(|d| d.contains("drive")), "{:?}", stats.left_out.dropped);
    assert!(stats.linkage, "the lid hinge stays");
    let program = de_shape::resolved_program(p.world(), p.entity(e).unwrap()).unwrap();
    assert_eq!(program.name, "Enclosure (Basic)");
    de_shape::motion_sweep(&program, 12).expect("motion without the fan's joint does not fail on it");

    // Exports are made from the same program: the fabrication items and the STL hold base and lid only.
    let items = de_shape::fabricate::items(&program, 0.05).unwrap();
    assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Base", "Lid"]);
    let stl = de_shape::stl(&program, 0.05, [0.0; 3]).unwrap();
    let top = stl[84..].chunks_exact(50).flat_map(|t| (0..3).map(move |v| f32::from_le_bytes(t[12 + v * 12 + 8..12 + v * 12 + 12].try_into().unwrap()))).fold(f32::MIN, f32::max);
    assert!((top - 14.0).abs() < 1e-4, "the STL stops at the lid's top (14 mm), no guard at 20: {top}");

    // Pro puts them back.
    run(&mut p, "shape.config.activate", json!({ "target": e, "name": "Pro" })).unwrap();
    let stats = &p.get::<ShapeSolid>(e).unwrap().stats;
    assert_eq!(part_names(stats), ["Base", "Lid", "Fan", "Fan guard"]);
    assert!(stats.left_out.is_empty());
}

#[test]
fn an_instance_in_its_own_configuration_leaves_its_parts_out() {
    let mut p = project();
    let e = enclosure(&mut p, true);
    run(&mut p, "shape.config.activate", json!({ "target": e, "name": "Pro" })).unwrap();
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Rack" })).unwrap());
    let place = |p: &mut Project, name: &str, x: f64, config: Option<&str>| id(&run(p, "assembly.instance.create", json!({ "of": e, "parent": asm, "name": name, "at": [x, 0, 0], "config": config })).unwrap());
    let pro = place(&mut p, "Unit 1", 0.0, None);
    let basic = place(&mut p, "Unit 2", 150.0, Some("Basic"));

    assert_eq!(instance_volume(&p, pro), None, "as defined: the definition's Pro");
    let c = p.get::<InstanceConfigured>(basic).unwrap().0.clone().unwrap();
    assert_eq!(part_names(&c.stats), ["Base", "Lid"], "Unit 2 is Basic");
    assert_eq!(c.part_masses.len(), 2);
    assert!((c.stats.volume_mm3 - (BASE + LID)).abs() < 1e-6);

    let stats = p.get::<AssemblyStats>(asm).unwrap();
    assert_eq!(stats.parts, 4 + 2, "the rack's parts list: four from Pro, two from Basic");
    let kg = stats.mass_kg.unwrap();
    assert!((kg - (2.0 * (BASE + LID) + FAN + GUARD) / 1e9 * 7850.0).abs() < 1e-9, "{kg}");
}

#[test]
fn leaving_parts_out_is_checked_and_reported() {
    let mut p = project();
    let e = enclosure(&mut p, false);
    let table = |off: Value| json!({ "target": e, "params": ["h"], "rows": [{ "name": "Basic", "values": { "h": 10 }, "parts_off": off }, { "name": "Pro", "values": { "h": 10 } }] });

    // Every part out is refused, as is a part the program does not make.
    let err = run(&mut p, "shape.config.set_table", table(json!(["Base", "Lid", "Fan", "Fan guard"]))).unwrap_err();
    assert!(err.message.contains("every part"), "{}", err.message);
    let err = run(&mut p, "shape.config.set_table", table(json!(["Fan", "Blower"]))).unwrap_err();
    assert!(err.message.contains("“Blower”") && err.message.contains("“Fan guard”"), "names what it lacks and what there is: {}", err.message);
    let err = run(&mut p, "shape.config.set_table", table(json!(["Fan", "Fan"]))).unwrap_err();
    assert!(err.message.contains("twice"), "{}", err.message);

    // The program renames the fan: the stale name is reported where the
    // configuration shows, not silently ignored, and nothing breaks.
    run(&mut p, "shape.config.activate", json!({ "target": e, "name": "Basic" })).unwrap();
    let source = p.get::<ShapeProgram>(e).unwrap().source.replace("\"Fan\"", "\"Blower\"");
    run(&mut p, "shape.update", json!({ "target": e, "source": source })).unwrap();
    let stats = &p.get::<ShapeSolid>(e).unwrap().stats;
    assert_eq!(part_names(stats), ["Base", "Lid", "Blower"], "the guard is still out; the renamed fan is back in");
    assert_eq!(stats.left_out.missing, ["Fan"], "and the old name is reported");
    // Other edits of the table still go through with the stale name in it.
    run(&mut p, "shape.config.activate", json!({ "target": e, "name": "Pro" })).unwrap();
    run(&mut p, "shape.config.activate", json!({ "target": e, "name": "Basic" })).unwrap();

    // A one-solid shape has no parts to leave out.
    let b = bolt(&mut p, false);
    let err = run(&mut p, "shape.config.set_table", json!({ "target": b, "params": ["d"], "rows": [{ "name": "S", "values": { "d": 4 }, "parts_off": ["Head"] }] })).unwrap_err();
    assert!(err.message.contains("one solid"), "{}", err.message);
}

#[test]
fn a_program_can_read_its_configuration() {
    let mut p = project();
    let e = id(&run(
        &mut p,
        "shape.create",
        json!({
            "name": "Plate",
            "source": "return box(p.w, 10, config === \"Thick\" ? 8 : 2);",
            "params": [{ "key": "w", "label": "Width", "value": 10, "unit": "mm" }]
        }),
    )
    .unwrap());
    assert!((volume(&p, e) - 200.0).abs() < 1e-6, "no configuration: config is null");
    run(&mut p, "shape.config.set_table", json!({ "target": e, "params": ["w"], "rows": [{ "name": "Thin", "values": { "w": 10 } }, { "name": "Thick", "values": { "w": 10 } }], "active": "Thick" })).unwrap();
    assert!((volume(&p, e) - 800.0).abs() < 1e-6, "Thick reads its name");
    let c = de_shape::configs::evaluate(p.world(), p.entity(e).unwrap(), "Thin").unwrap();
    assert!((c.stats.volume_mm3 - 200.0).abs() < 1e-6, "an instance's configuration reaches the program too");
}
