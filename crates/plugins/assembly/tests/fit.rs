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
fn a_nyloc_nut_is_checked_in_its_trap_like_a_plain_nut() {
    for (af, want) in [(5.8, FitStatus::Pass), (5.3, FitStatus::Fail)] {
        let mut p = project();
        // ISO 10511 M3: 5.5 across flats, hex 2.4 high under its collar.
        let src = format!("return [part('Block', box(20, 20, 6).translate(-10, -10, 0).subtract(regularPolygon(6, {af} / Math.sqrt(3), {{ rotate: 30 }}).extrude(5).translate(0, 0, -0.01)).subtract(cylinder(7, 1.7).translate(0, 0, -0.5)), {{ process: '3d_print' }})];");
        let block = id(&run(&mut p, "shape.create", json!({ "name": "Block", "definition": true, "display_tolerance": 0.01, "source": src, "params": [] })).unwrap());
        let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Trap" })).unwrap());
        run(&mut p, "assembly.instance.create", json!({ "of": block, "parent": asm, "name": "Block" })).unwrap();
        run(&mut p, "stock.insert", json!({ "family": "iso10511", "size": "M3", "parent": asm, "name": "Nyloc", "at": [0, 0, 0.3] })).unwrap();
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

/// A caliper's depth rod sliding under its beam (machined steel): a Ø5
/// guide pin pressed into a Ø`hole` bore in the rod runs in a through slot
/// `slot` wide along the beam.
fn caliper(slot: f64, hole: f64) -> FitReport {
    let mut p = project();
    let beam = id(&run(&mut p, "shape.create", json!({
        "name": "Beam", "definition": true, "display_tolerance": 0.005,
        "source": "const b = box(120, 20, 5).translate(0, -10, 0).subtract(box(60, p.w, 7).translate(30, -p.w / 2, -1));\nconnector('Rail', { at: [0, 0, 0], axis: [1, 0, 0] });\nreturn [part('Beam', b, { process: 'cnc_mill' })];",
        "params": [param("w", slot)]
    })).unwrap());
    let rod = id(&run(&mut p, "shape.create", json!({
        "name": "Depth rod", "definition": true, "display_tolerance": 0.005,
        "source": "const r = box(120, 8, 4).translate(0, -4, -4).subtract(cylinder(6, p.hole / 2).translate(60, 0, -5));\nconnector('Rail', { at: [0, 0, 0], axis: [1, 0, 0] });\nreturn [part('Depth rod', r, { process: 'cnc_mill' })];",
        "params": [param("hole", hole)]
    })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Caliper" })).unwrap());
    let b = id(&run(&mut p, "assembly.instance.create", json!({ "of": beam, "parent": asm, "name": "Beam" })).unwrap());
    let r = id(&run(&mut p, "assembly.instance.create", json!({ "of": rod, "parent": asm, "name": "Depth rod" })).unwrap());
    run(&mut p, "stock.insert", json!({ "family": "iso8734", "size": "Ø5×10", "parent": asm, "name": "Guide pin", "at": [60, 0, -4] })).unwrap();
    run(&mut p, "assembly.joint.create", json!({ "assembly": asm, "type": "slider", "name": "Depth slide", "aligned": true, "a": { "instance": b, "connector": "Rail" }, "b": { "instance": r, "connector": "Rail" } })).unwrap();
    fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap()
}

#[test]
fn a_pin_running_in_a_slot_is_a_sliding_fit_not_a_housing() {
    let r = caliper(5.1, 5.0);
    let pin = find(&r, "pin");
    let slot = check(pin, "Slot");
    assert_eq!(slot.status, FitStatus::Pass, "{}", slot.text);
    assert_eq!(slot.part.as_deref(), Some("Beam"));
    assert!((slot.measured.unwrap() - 5.1).abs() < 0.02, "{}", slot.text);
    assert!(slot.text.contains("sliding fit, 0.1 mm clearance") && slot.text.contains("free along it"), "{}", slot.text);
    // The bore in the rod holds it; the slot is not taken for a housing.
    let housings: Vec<&fit::FitCheck> = pin.checks.iter().filter(|c| c.what == "Housing").collect();
    assert_eq!(housings.len(), 1, "{:#?}", pin.checks);
    assert_eq!(housings[0].part.as_deref(), Some("Depth rod"));
    assert_eq!(housings[0].status, FitStatus::Pass, "{}", housings[0].text);
    assert!(r.ok, "{:#?}", r.items);
    assert!(!r.items.iter().any(|i| i.kind == "interference"), "{:#?}", r.items);
}

#[test]
fn a_pin_too_big_for_its_slot_fails_and_a_loose_one_warns() {
    // Ø5 pin in a 4.8 slot: it jams.
    let r = caliper(4.8, 5.0);
    let slot = check(find(&r, "pin"), "Slot");
    assert_eq!(slot.status, FitStatus::Fail, "{}", slot.text);
    assert!(slot.text.contains("0.2 mm too narrow"), "{}", slot.text);
    assert!(!r.ok);
    // 5.6: it rattles.
    let r = caliper(5.6, 5.0);
    let slot = check(find(&r, "pin"), "Slot");
    assert_eq!(slot.status, FitStatus::Warn, "{}", slot.text);
    assert!(slot.text.contains("loose"), "{}", slot.text);
}

#[test]
fn a_bearing_in_a_split_housing_is_housed_by_both_halves() {
    // A pillow block cut in two along its bore: each half alone is open, both close it.
    let mut p = project();
    let half = |p: &mut Project, name: &str, y0: f64| {
        id(&run(p, "shape.create", json!({
            "name": name, "definition": true, "display_tolerance": 0.005,
            "source": format!("return [part('{name}', cylinder(7, 18).subtract(cylinder(7.2, 21.9 / 2).translate(0, 0, -0.1)).intersect(box(40, 20, 9).translate(-20, {y0}, -1)), {{ process: '3d_print' }})];"),
            "params": []
        })).unwrap())
    };
    let (top, bottom) = (half(&mut p, "Cap", 0.0), half(&mut p, "Block", -20.0));
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Pillow block" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": top, "parent": asm, "name": "Cap" })).unwrap();
    run(&mut p, "assembly.instance.create", json!({ "of": bottom, "parent": asm, "name": "Block" })).unwrap();
    run(&mut p, "stock.insert", json!({ "family": "bearing_deep_groove", "size": "608-2RS", "parent": asm, "name": "Bearing" })).unwrap();
    let r = fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap();
    let b = find(&r, "bearing");
    let housings: Vec<&fit::FitCheck> = b.checks.iter().filter(|c| c.what == "Housing").collect();
    assert_eq!(housings.len(), 2, "{:#?}", b.checks);
    for h in housings {
        assert_eq!(h.status, FitStatus::Pass, "{}", h.text);
        assert!(h.text.contains("press fit"), "{}", h.text);
    }
    assert!(!b.checks.iter().any(|c| c.what == "Channel"), "{:#?}", b.checks);
}

/// A Ø5 pin lying along an open, half-round printed channel of radius `r`.
fn channel(r: f64) -> FitReport {
    let mut p = project();
    let block = id(&run(&mut p, "shape.create", json!({
        "name": "Block", "definition": true, "display_tolerance": 0.005,
        "source": "return [part('Block', box(30, 20, 10).translate(-5, -10, 0).subtract(cylinder(40, p.r).rotate(0, 90, 0).translate(-10, 0, 10)), { process: '3d_print' })];",
        "params": [param("r", r)]
    })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Rest" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": block, "parent": asm, "name": "Block" })).unwrap();
    let pin = id(&run(&mut p, "stock.insert", json!({ "family": "iso8734", "size": "Ø5×20", "parent": asm, "name": "Pin" })).unwrap());
    run(&mut p, "assembly.instance.update", json!({ "target": pin, "at": [0, 0, 10], "rotation": [0, 90, 0] })).unwrap();
    fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap()
}

#[test]
fn a_pin_in_an_open_channel_is_checked_across_it() {
    let r = channel(2.75);
    let pin = find(&r, "pin");
    let c = check(pin, "Channel");
    assert_eq!(c.status, FitStatus::Pass, "{}", c.text);
    assert!((c.measured.unwrap() - 5.5).abs() < 0.03, "{}", c.text);
    // Nothing holds it there: the channel only guides it.
    assert_eq!(check(pin, "Housing").status, FitStatus::Warn);
    // A channel narrower than the pin: it does not go in.
    let r = channel(2.4);
    assert_eq!(check(find(&r, "pin"), "Channel").status, FitStatus::Fail);
}

/// A case (a printed base with an M3 insert, and its screw) placed at
/// (100, 50, 0) in the project; the screw clamps a board that is not in the
/// case's assembly but placed on its own at `board` (on the base: 100, 50, 10).
fn clamped(board: [f64; 3], len: f64) -> FitReport {
    let mut p = project();
    let base = id(&run(&mut p, "shape.create", json!({ "name": "Base", "source": BASE, "params": [param("hole", 4.0), param("depth", 6.7)], "definition": true, "display_tolerance": 0.01 })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Case" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": base, "parent": asm, "name": "Base" })).unwrap();
    run(&mut p, "stock.insert", json!({ "family": "heatset_insert", "size": "M3×5.7", "parent": asm, "name": "Insert", "at": [0, 0, 10] })).unwrap();
    run(&mut p, "stock.insert", json!({ "family": "iso4762", "size": format!("M3×{len}"), "parent": asm, "name": "Screw", "at": [0, 0, 11.6] })).unwrap();
    run(&mut p, "assembly.instance.create", json!({ "of": asm, "name": "Case", "at": [100, 50, 0] })).unwrap();
    let mm = |v: f64| json!({ "value": v, "unit": "mm" });
    run(&mut p, "shape.create", json!({
        "name": "Board", "display_tolerance": 0.01,
        "source": "return [part('PCB', box(30, 30, 1.6).translate(-15, -15, 0).subtract(cylinder(3, 1.7).translate(0, 0, -0.5)), { process: 'pcb_fab' })];",
        "params": [], "placement": { "x": mm(board[0]), "y": mm(board[1]), "z": mm(board[2]) }
    })).unwrap();
    fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap()
}

/// An M2 board screw through a Ø2.2 hole in a board drawn coarsely (±0.05 mm:
/// an 11-sided hole) into an insert in the case under it.
#[test]
fn a_coarsely_drawn_hole_is_measured_at_its_true_size() {
    let mut p = project();
    let base = id(&run(&mut p, "shape.create", json!({ "name": "Base", "source": "return [part('Base', box(10, 10, 6).translate(-5, -5, -6).subtract(cylinder(5.01, 1.6).translate(0, 0, -5)), { process: '3d_print' })];", "params": [], "definition": true, "display_tolerance": 0.01 })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Case" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": base, "parent": asm, "name": "Base" })).unwrap();
    run(&mut p, "stock.insert", json!({ "family": "heatset_insert", "size": "M2×4", "parent": asm, "name": "Insert", "at": [0, 0, 0] })).unwrap();
    run(&mut p, "stock.insert", json!({ "family": "iso4762", "size": "M2×6", "parent": asm, "name": "Screw", "at": [0, 0, 1.6] })).unwrap();
    run(&mut p, "assembly.instance.create", json!({ "of": asm, "name": "Case" })).unwrap();
    run(&mut p, "shape.create", json!({
        "name": "Board", "display_tolerance": 0.05,
        "source": "return [part('PCB', box(20, 20, 1.6).translate(-10, -10, 0).subtract(cylinder(3.6, 1.1).translate(0, 0, -1)), { process: 'pcb_fab' })];",
        "params": []
    })).unwrap();
    let r = fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap();
    let hole = check(find(&r, "screw"), "Clearance hole");
    assert_eq!(hole.status, FitStatus::Pass, "{}", hole.text);
    assert!(hole.text.contains("ISO 273 fine"), "{}", hole.text);
    assert!((hole.measured.unwrap() - 2.2).abs() < 0.011, "{}", hole.text);
    assert!(r.ok, "{:#?}", r.items);
}

#[test]
fn a_screw_clamps_a_board_outside_its_assembly() {
    let r = clamped([100.0, 50.0, 10.0], 8.0);
    let screw = find(&r, "screw");
    let seat = check(screw, "Head seat");
    assert_eq!(seat.status, FitStatus::Pass, "{}", seat.text);
    assert!(seat.text.contains("Board"), "{}", seat.text);
    let hole = check(screw, "Clearance hole");
    assert_eq!(hole.part.as_deref(), Some("Board"));
    assert!(hole.text.contains("ISO 273 medium"), "{}", hole.text);
    assert_eq!(check(screw, "Thread engagement").status, FitStatus::Pass);
    assert!(r.ok, "{:#?}", r.items);
    // Reported for the case alone: the board is no item of it.
    assert!(r.items.iter().all(|i| i.kind != "interference"), "{:#?}", r.items);
    assert!(r.notes.iter().any(|n| n.contains("outside Case")), "{:?}", r.notes);
}

#[test]
fn a_screw_with_nothing_under_its_head_or_too_short_still_fails() {
    // The board is elsewhere: the head stands in the air.
    let r = clamped([300.0, 50.0, 10.0], 8.0);
    let seat = check(find(&r, "screw"), "Head seat");
    assert_eq!(seat.status, FitStatus::Fail, "{}", seat.text);
    // An M3×4 through the board does not reach the insert.
    let r = clamped([100.0, 50.0, 10.0], 4.0);
    let e = check(find(&r, "screw"), "Thread engagement");
    assert_eq!(e.status, FitStatus::Fail, "{}", e.text);
    assert!(!r.ok);
}

/// A printed block with a sleeve inside it, `long` mm long from z = 2 (the block is 10 high).
fn sleeved(long: f64, embedded: bool) -> FitReport {
    let mut p = project();
    let block = id(&run(&mut p, "shape.create", json!({ "name": "Block", "definition": true, "source": "return [part('Block', box(20, 20, 10).translate(-10, -10, 0), { process: '3d_print' })];", "params": [] })).unwrap());
    let sleeve = id(&run(&mut p, "shape.create", json!({ "name": "Sleeve", "definition": true, "source": "return [part('Sleeve', cylinder(p.l, 3), { process: 'lathe' })];", "params": [param("l", long)] })).unwrap());
    let asm = id(&run(&mut p, "assembly.create", json!({ "name": "Moulding" })).unwrap());
    run(&mut p, "assembly.instance.create", json!({ "of": block, "parent": asm, "name": "Block" })).unwrap();
    let props = if embedded { json!({ "embedded": "Block" }) } else { json!({}) };
    run(&mut p, "assembly.instance.create", json!({ "of": sleeve, "parent": asm, "name": "Sleeve", "at": [0, 0, 2], "properties": props })).unwrap();
    fit::report(p.world(), Some(asm), &FitOptions { motion: false, steps: 12 }).unwrap()
}

#[test]
fn a_part_declared_embedded_is_meant_to_overlap_unless_it_pokes_out() {
    let r = sleeved(6.0, true);
    let e = find(&r, "embedded");
    assert_eq!(e.status, FitStatus::Pass, "{:#?}", e.checks);
    assert!(!r.items.iter().any(|i| i.kind == "interference"), "{:#?}", r.items);
    assert!(r.ok);
    // 12 mm long from z = 2: 4 mm stands out of the block's top.
    let r = sleeved(12.0, true);
    let c = check(find(&r, "embedded"), "Embedded");
    assert_eq!(c.status, FitStatus::Fail, "{}", c.text);
    assert!(c.text.contains("pokes out"), "{}", c.text);
    let out = std::f64::consts::PI * 9.0 * 4.0;
    assert!((c.measured.unwrap() - out).abs() < 0.03 * out, "{}", c.text);
    // Not declared: an overlap like any other.
    let r = sleeved(6.0, false);
    assert_eq!(find(&r, "interference").status, FitStatus::Fail);
}

