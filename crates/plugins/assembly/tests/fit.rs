//! Fit checks with holes made right and seeded wrong: an M3 screw through a
//! printed lid into a heat-set insert in a printed base; a bearing pressed
//! into a printed housing; a nut in a hex trap; a hex key blocked by a cap.

use de_assembly::fit::{self, FitOptions, FitReport, FitStatus};
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

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin])))
}

/// A printed base 30 × 30 × 10 (top at z = 10) with the insert hole, and a
/// printed lid 30 × 30 × 3 on it with the screw's clearance hole.
const BASE: &str = r#"
const body = box(30, 30, 10).translate(-15, -15, 0)
  .subtract(cylinder(p.depth + 0.01, p.hole / 2).translate(0, 0, 10 - p.depth));
return [part("Base", body, { process: "3d_print" })];
"#;
const LID: &str = r#"
const body = box(30, 30, 3).translate(-15, -15, 0)
  .subtract(cylinder(3.2, p.hole / 2).translate(p.off, 0, -0.1));
// The connector is where the hole was meant to be; `off` misplaces the hole.
connector("Hole", { at: [0, 0, 3], axis: [0, 0, 1] });
return [part("Lid", body, { process: "3d_print" })];
"#;

struct Joint {
    p: Project,
    asm: StableId,
    base: StableId,
    lid: StableId,
    screw: StableId,
}

fn param(key: &str, value: f64) -> Value {
    json!({ "key": key, "label": key, "value": value, "unit": "mm" })
}

/// Base hole Ø`insert_hole` × `depth`, lid hole Ø`clear` off-axis by `off`, an M3×`len` screw.
fn screwed(insert_hole: f64, depth: f64, clear: f64, off: f64, len: f64) -> Joint {
    let mut p = project();
    let base = id(&run(&mut p, "shape.create", json!({ "name": "Base", "source": BASE, "params": [param("hole", insert_hole), param("depth", depth)], "definition": true, "display_tolerance": 0.01 })).unwrap());
    let lid = id(&run(&mut p, "shape.create", json!({ "name": "Lid", "source": LID, "params": [param("hole", clear), param("off", off)], "definition": true, "display_tolerance": 0.01 })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Box" })).unwrap());
    let base_i = id(&run(&mut p, "assembly.instance.create", json!({ "of": base, "parent": asm, "name": "Base" })).unwrap());
    let lid_i = id(&run(&mut p, "assembly.instance.create", json!({ "of": lid, "parent": asm, "name": "Lid", "at": [0, 0, 10] })).unwrap());
    // The insert flush with the base's top, the screw's head on the lid.
    run(&mut p, "stock.insert", json!({ "family": "heatset_insert", "size": "M3×5.7", "parent": asm, "name": "Insert", "at": [0, 0, 10] })).unwrap();
    let screw = id(&run(&mut p, "stock.insert", json!({ "family": "iso4762", "size": format!("M3×{len}"), "parent": asm, "name": "Screw", "at": [0, 0, 13] })).unwrap());
    // Seat the screw by a joint on the lid's hole (opposed axes: the shank goes in).
    run(&mut p, "assembly.joint.create", json!({ "assembly": asm, "type": "fixed", "name": "Screw seat", "a": { "instance": lid_i, "connector": "Hole" }, "b": { "instance": screw, "connector": "Head seat" } })).unwrap();
    Joint { p, asm, base: base_i, lid: lid_i, screw }
}

fn report(j: &Joint) -> FitReport {
    fit::report(j.p.world(), Some(j.asm), &FitOptions { motion: false, steps: 12 }).unwrap()
}

fn find<'a>(r: &'a FitReport, kind: &str) -> &'a fit::FitItem {
    r.items.iter().find(|i| i.kind == kind).unwrap_or_else(|| panic!("no {kind} item in {:#?}", r.items))
}

fn check<'a>(item: &'a fit::FitItem, what: &str) -> &'a fit::FitCheck {
    item.checks.iter().find(|c| c.what == what).unwrap_or_else(|| panic!("no {what} check in {:#?}", item.checks))
}

#[test]
fn a_screw_through_a_clearance_hole_into_an_insert_fits() {
    let j = screwed(4.0, 6.7, 3.4, 0.0, 8.0);
    let r = report(&j);
    assert!(r.ok, "{:#?}", r.items);
    let screw = find(&r, "screw");
    assert_eq!(screw.instances[0], j.screw);
    assert_eq!(screw.joint.as_deref(), Some("Screw seat"));
    let hole = check(screw, "Clearance hole");
    assert_eq!(hole.status, FitStatus::Pass);
    assert!(hole.text.contains("ISO 273 medium"), "{}", hole.text);
    assert!((hole.measured.unwrap() - 3.4).abs() < 0.03, "{:?}", hole.measured);
    let e = check(screw, "Thread engagement");
    assert_eq!(e.status, FitStatus::Pass, "{}", e.text);
    // Head on the lid at 13, tip at 5; the insert runs 10 → 4.3: 5 mm engaged.
    assert!((e.measured.unwrap() - 5.0).abs() < 0.3, "{}", e.text);
    assert_eq!(check(screw, "Head seat").status, FitStatus::Pass);
    assert_eq!(check(screw, "Bottoming out").status, FitStatus::Pass);
    assert_eq!(check(screw, "Hex key").status, FitStatus::Pass);
    let insert = find(&r, "insert");
    assert_eq!(insert.status, FitStatus::Pass, "{:#?}", insert.checks);
    assert!((check(insert, "Insert hole").measured.unwrap() - 4.0).abs() < 0.03);
    let wall = check(insert, "Wall");
    assert_eq!(wall.status, FitStatus::Pass, "{}", wall.text);
    assert!(insert.instances.contains(&j.base));
    assert!(screw.instances.contains(&j.lid));
}

#[test]
fn an_insert_hole_too_small_fails() {
    let r = report(&screwed(3.0, 6.7, 3.4, 0.0, 8.0));
    assert!(!r.ok);
    let c = check(find(&r, "insert"), "Insert hole");
    assert_eq!(c.status, FitStatus::Fail);
    assert!(c.text.contains("too small"), "{}", c.text);
    // And too shallow: 4 mm for a 5.7 mm insert.
    let r = report(&screwed(4.0, 4.0, 3.4, 0.0, 6.0));
    assert_eq!(check(find(&r, "insert"), "Hole depth").status, FitStatus::Fail);
}

#[test]
fn an_off_axis_clearance_hole_fails() {
    // 0.6 mm off in a Ø3.4 hole: the M3 shank hits the wall by 0.4 mm.
    let r = report(&screwed(4.0, 6.7, 3.4, 0.6, 8.0));
    let screw = find(&r, "screw");
    let c = screw.checks.iter().find(|c| c.what == "Coaxial" && c.part.as_deref() == Some("Lid")).unwrap();
    assert_eq!(c.status, FitStatus::Fail, "{}", c.text);
    assert!((c.measured.unwrap() - 0.6).abs() < 0.05, "{}", c.text);
    assert!(c.text.contains("hits the hole wall"));
}

#[test]
fn a_short_screw_and_a_long_one_fail() {
    // M3×5: only 2 mm into the insert; the report says which length to use.
    let r = report(&screwed(4.0, 6.7, 3.4, 0.0, 5.0));
    let e = check(find(&r, "screw"), "Thread engagement");
    assert_eq!(e.status, FitStatus::Fail, "{}", e.text);
    assert!(e.text.contains("use M3×8"), "{}", e.text);
    // M3×12: the tip runs past the 6.7 mm hole into the base.
    let r = report(&screwed(4.0, 6.7, 3.4, 0.0, 12.0));
    let b = check(find(&r, "screw"), "Bottoming out");
    assert_eq!(b.status, FitStatus::Fail, "{}", b.text);
    assert_eq!(b.part.as_deref(), Some("Base"));
    // The hole bottom is 9.7 mm under the head.
    assert!((b.measured.unwrap() - 9.7).abs() < 0.3, "{}", b.text);
    assert!(b.text.contains("M3×8"), "{}", b.text);
}

#[test]
fn a_clearance_hole_smaller_than_the_screw_fails() {
    let r = report(&screwed(4.0, 6.7, 2.0, 0.0, 8.0));
    let c = check(find(&r, "screw"), "Hole");
    assert_eq!(c.status, FitStatus::Fail, "{}", c.text);
}

#[test]
fn a_cap_over_the_head_blocks_the_key() {
    let mut j = screwed(4.0, 6.7, 3.4, 0.0, 8.0);
    let cap = id(&run(&mut j.p, "shape.create", json!({ "name": "Cap", "source": "return [part('Cap', box(20, 20, 2).translate(-10, -10, 0), { process: '3d_print' })];", "params": [], "definition": true })).unwrap());
    run(&mut j.p, "assembly.instance.create", json!({ "of": cap, "parent": j.asm, "name": "Cap", "at": [0, 0, 30] })).unwrap();
    let r = report(&j);
    let c = check(find(&r, "screw"), "Hex key");
    assert_eq!(c.status, FitStatus::Fail, "{}", c.text);
    assert_eq!(c.part.as_deref(), Some("Cap"));
    // Socket top at 13 + 3 = 16; the cap's underside at 30.
    assert!((c.measured.unwrap() - 14.0).abs() < 0.1, "{}", c.text);
    // The cap goes on at a later assembly step: the screw is tightened before it is there.
    let caps: Vec<StableId> = j.p.entity_ids().into_iter().filter(|e| j.p.get::<de_assembly::Instance>(*e).is_some_and(|i| i.name == "Cap")).collect();
    run(&mut j.p, "assembly.instance.update", json!({ "target": caps[0], "properties": { "step": "5" } })).unwrap();
    run(&mut j.p, "assembly.instance.update", json!({ "target": j.screw, "properties": { "step": "2" } })).unwrap();
    let r = report(&j);
    assert_eq!(check(find(&r, "screw"), "Hex key").status, FitStatus::Pass);
}

/// A 608 in a printed housing of bore `bore`, on a printed shaft of `shaft`.
fn bearing(bore: f64, shaft: f64) -> FitReport {
    let mut p = project();
    let housing = id(&run(&mut p, "shape.create", json!({
        "name": "Housing", "definition": true, "display_tolerance": 0.005,
        "source": "return [part('Housing', cylinder(7, 18).subtract(cylinder(7.2, p.bore / 2).translate(0, 0, -0.1)), { process: '3d_print' })];",
        "params": [param("bore", bore)]
    })).unwrap());
    let axle = id(&run(&mut p, "shape.create", json!({
        "name": "Axle", "definition": true, "display_tolerance": 0.005,
        "source": "return [part('Axle', cylinder(20, p.d / 2).translate(0, 0, -6), { process: '3d_print' })];",
        "params": [param("d", shaft)]
    })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Idler" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": housing, "parent": asm, "name": "Housing" })).unwrap();
    run(&mut p, "assembly.instance.create", json!({ "of": axle, "parent": asm, "name": "Axle" })).unwrap();
    run(&mut p, "stock.insert", json!({ "family": "bearing_deep_groove", "size": "608-2RS", "parent": asm, "name": "Bearing" })).unwrap();
    fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap()
}

#[test]
fn a_bearing_press_fit_in_a_print_passes_and_a_loose_one_does_not() {
    let r = bearing(21.9, 8.0);
    let b = find(&r, "bearing");
    let h = check(b, "Housing");
    assert_eq!(h.status, FitStatus::Pass, "{}", h.text);
    assert!(h.text.contains("press fit, 0.1 mm"), "{}", h.text);
    assert_eq!(check(b, "Shaft").status, FitStatus::Pass, "{}", check(b, "Shaft").text);
    // The press fit is the bearing's item, not an interference.
    assert!(!r.items.iter().any(|i| i.kind == "interference"), "{:#?}", r.items);
    assert!(r.ok, "{:#?}", r.items);
    // 0.5 mm over: loose.
    let r = bearing(22.5, 8.0);
    let h = check(find(&r, "bearing"), "Housing");
    assert_eq!(h.status, FitStatus::Warn, "{}", h.text);
    // 0.4 mm under: it would crack the print.
    let r = bearing(21.6, 8.0);
    assert_eq!(check(find(&r, "bearing"), "Housing").status, FitStatus::Fail);
}

#[test]
fn a_nut_in_a_hex_trap_and_in_one_too_small() {
    for (af, want) in [(5.8, FitStatus::Pass), (5.3, FitStatus::Fail)] {
        let mut p = project();
        // M3 nut: 5.5 across flats, 2.4 high; flats parallel to X as the library stands it.
        let src = format!("return [part('Block', box(20, 20, 6).translate(-10, -10, 0).subtract(regularPolygon(6, {af} / Math.sqrt(3), {{ rotate: 30 }}).extrude(3).translate(0, 0, -0.01)).subtract(cylinder(7, 1.7).translate(0, 0, -0.5)), {{ process: '3d_print' }})];");
        let block = id(&run(&mut p, "shape.create", json!({ "name": "Block", "definition": true, "display_tolerance": 0.01, "source": src, "params": [] })).unwrap());
        let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Trap" })).unwrap());
        run(&mut p, "assembly.instance.create", json!({ "of": block, "parent": asm, "name": "Block" })).unwrap();
        run(&mut p, "stock.insert", json!({ "family": "iso4032", "size": "M3", "parent": asm, "name": "Nut", "at": [0, 0, 0.3] })).unwrap();
        let r = fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap();
        let c = check(find(&r, "nut"), "Nut trap");
        assert_eq!(c.status, want, "{}", c.text);
    }
}

#[test]
fn the_printer_profile_is_a_project_setting() {
    let mut p = project();
    assert_eq!(fit::profile(p.world()).clearance, 0.4);
    assert_eq!(fit::profile(p.world()).slide, [0.2, 0.3]);
    let a = id(&run(&mut p, "fit.profile.set", json!({ "press": 0.15 })).unwrap());
    let b = id(&run(&mut p, "fit.profile.set", json!({ "material": "PETG" })).unwrap());
    assert_eq!(a, b, "one profile per project");
    let pr = fit::profile(p.world());
    assert_eq!((pr.press, pr.material.as_str(), pr.layer), (0.15, "PETG", 0.2));
    assert!(run(&mut p, "fit.profile.set", json!({ "slide": [0.4, 0.2] })).is_err());
}

