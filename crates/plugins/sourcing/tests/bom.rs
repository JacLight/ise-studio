use de_assembly::AssemblyPlugin;
use de_core::{Actor, CommandError, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, bevy_ecs::world::World, json};
use de_shape::{ShapePlugin, stock::Stock};
use de_solid::SolidPlugin;
use de_sourcing::{
    BuyItem, BuySource, Rfq, Scope, SourcingPlugin, Supplier,
    bom::{self, Bom, Options},
    package,
};
use std::collections::BTreeSet;
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
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &AssemblyPlugin, &SourcingPlugin])))
}

struct Kit {
    steel: StableId,
    bracket: StableId,
    screw: StableId,
    machine: StableId,
}

/// A bracket (laser-cut plate, turned pin, printed fan only in “Pro”, a
/// bought grommet with no part number), a stock screw sold by the 100, a
/// module of two brackets (Basic and Pro) and three screws, and a machine
/// of two modules and one more Pro bracket; two machines at the top.
fn kit(p: &mut Project) -> Kit {
    let steel = id(&run(p, "solid.material.create", json!({ "name": "Steel", "density": { "value": 7850, "unit": "kg/m3" }, "cost": { "per_kg": 2.0, "sheet": { "price": 120.0, "length": 2500, "width": 1250 } } })).unwrap());
    let pla = id(&run(p, "solid.material.create", json!({ "name": "PLA", "density": { "value": 1240, "unit": "kg/m3" }, "cost": { "per_kg": 25.0 } })).unwrap());
    let bracket = id(&run(
        p,
        "shape.create",
        json!({
            "name": "Bracket",
            "definition": true,
            "source": r#"return [
                part("Plate", box(p.w, 40, 3), { material: "Steel", process: "laser" }),
                part("Pin", cylinder(20, 3).translate(10, 10, 3), { material: "Steel", process: "lathe" }),
                part("Fan", box(20, 20, 10).translate(30, 10, 3), { material: "PLA", process: "3d_print" }),
                part("Grommet", box(4, 4, 2).translate(0, 0, 3), { process: "buy" }),
            ];"#,
            "params": [{ "key": "w", "label": "Width", "value": 60, "unit": "mm", "min": 10 }],
            "materials": { "Steel": steel, "PLA": pla }
        }),
    )
    .unwrap());
    run(p, "shape.config.set_table", json!({ "target": bracket, "params": ["w"], "rows": [
        { "name": "Basic", "values": { "w": 60 }, "parts_off": ["Fan"] },
        { "name": "Pro", "values": { "w": 60 } }
    ] }))
    .unwrap();
    let screw = id(&run(
        p,
        "shape.create",
        json!({
            "name": "Screw",
            "definition": true,
            "source": r#"stock({ standard: "ISO 4762", partNumber: "ISO 4762 M3×12 A2", description: "Socket head cap screw M3×12", unitPrice: 0.05, pack: 100, massG: 1.2,
                       sources: [{ supplier: "McMaster-Carr", sku: "91292A113", unitPrice: 0.09 }] });
                     return cylinder(12, 1.5);"#,
            "params": [],
            "material": steel
        }),
    )
    .unwrap());
    let module = id(&run(p, "assembly.create", json!({ "name": "Module" })).unwrap());
    let place = |p: &mut Project, of: StableId, parent: StableId, name: &str, config: Option<&str>| {
        run(p, "assembly.instance.create", json!({ "of": of, "parent": parent, "name": name, "at": [0, 0, 0], "config": config })).unwrap();
    };
    place(p, bracket, module, "B1", Some("Basic"));
    place(p, bracket, module, "B2", Some("Pro"));
    for n in ["S1", "S2", "S3"] {
        place(p, screw, module, n, None);
    }
    let machine = id(&run(p, "assembly.create", json!({ "name": "Machine" })).unwrap());
    place(p, module, machine, "Left", None);
    place(p, module, machine, "Right", None);
    place(p, bracket, machine, "Spare", Some("Pro"));
    for n in ["M1", "M2"] {
        run(p, "assembly.instance.create", json!({ "of": machine, "name": n, "at": [0, 0, 0] })).unwrap();
    }
    Kit { steel, bracket, screw, machine }
}

fn make<'a>(b: &'a Bom, name: &str) -> &'a bom::MakeLine {
    b.make.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("no make line {name}: {:?}", b.make.iter().map(|l| &l.name).collect::<Vec<_>>()))
}

fn compute(p: &Project, scope: Scope, quantity: u32) -> Bom {
    bom::compute(p.world(), scope, &Options { quantity, ..Default::default() }, &[]).unwrap()
}

#[test]
fn quantities_roll_up_through_nested_assemblies_configurations_and_parts_left_out() {
    let mut p = project();
    let k = kit(&mut p);
    let m = compute(&p, Scope::Entity(k.machine), 1);
    // Per machine: two modules of two brackets, plus a spare; the fan only where Pro.
    assert_eq!(make(&m, "Plate").qty, 5, "Basic and Pro plates are identical: one line");
    assert_eq!(make(&m, "Pin").qty, 5);
    assert_eq!(make(&m, "Fan").qty, 3, "Basic leaves the fan out");
    assert_eq!(m.make.len(), 3);
    let screws = m.buy.iter().find(|l| l.key == "buy:ISO 4762 M3×12 A2").unwrap();
    assert_eq!((screws.qty, screws.order_qty), (6, 100), "six screws, ordered as a pack of 100");
    let grommet = m.buy.iter().find(|l| l.name == "Grommet").unwrap();
    assert_eq!(grommet.qty, 5);
    assert!(grommet.stock.is_none());
    assert!(m.flags.iter().any(|f| f.text.contains("Grommet: bought part without a part number")), "{:?}", m.flags);

    // The indented view: Machine › Module ×2 › Bracket (Basic) ×1 …
    let root = &m.tree[0];
    assert_eq!((root.name.as_str(), root.kind), ("Machine", "assembly"));
    let module = root.children.iter().find(|n| n.name == "Module").unwrap();
    assert_eq!((module.qty, module.total, module.instances.len()), (2, 2, 2));
    let basic = module.children.iter().find(|n| n.name == "Bracket (Basic)").unwrap();
    assert_eq!((basic.qty, basic.total), (1, 2));
    assert!(!basic.children.iter().any(|c| c.name == "Fan"), "the fan is left out of Basic");
    let screw_row = module.children.iter().find(|n| n.class == Some("buy")).unwrap();
    assert_eq!((screw_row.qty, screw_row.total), (3, 6));

    // The whole project: two machines.
    let all = compute(&p, Scope::Project, 1);
    assert_eq!(make(&all, "Plate").qty, 10);
    assert_eq!(make(&all, "Fan").qty, 6);
    assert_eq!(all.buy.iter().find(|l| l.name.starts_with("Screw")).unwrap().qty, 12);
    assert_eq!(all.tree.len(), 1, "two instances of one machine are one row");
    assert_eq!((all.tree[0].qty, all.tree[0].instances.len()), (2, 2));

    // A shape on its own, in a chosen configuration.
    let pro = bom::compute(p.world(), Scope::Entity(k.bracket), &Options { config: Some("Pro".into()), ..Default::default() }, &[]).unwrap();
    assert_eq!(pro.make.len(), 3);
    let basic = bom::compute(p.world(), Scope::Entity(k.bracket), &Options { config: Some("Basic".into()), ..Default::default() }, &[]).unwrap();
    assert_eq!(basic.make.len(), 2);
    let _ = k.screw;
}

#[test]
fn identical_items_merge_and_stock_merges_by_part_number() {
    let mut p = project();
    let k = kit(&mut p);
    // A second shape making the same plate (same name, material, process, size) shares its line.
    let other = id(&run(
        &mut p,
        "shape.create",
        json!({ "name": "Other bracket", "source": r#"return [part("Plate", box(60, 40, 3), { material: "Steel", process: "laser" }), part("Tab", box(5, 5, 3), { material: "Steel", process: "laser" })];"#, "params": [], "materials": { "Steel": k.steel }, "placement": { "x": { "value": 0, "unit": "m" }, "y": { "value": 0, "unit": "m" }, "z": { "value": 0, "unit": "m" } } }),
    )
    .unwrap());
    // A screw declared by another program with the same part number merges with the kit's.
    run(&mut p, "shape.create", json!({ "name": "Loose screw", "source": r#"stock({ partNumber: "ISO 4762 M3×12 A2" }); return cylinder(12, 1.5);"#, "params": [], "placement": { "x": { "value": 0, "unit": "m" }, "y": { "value": 0, "unit": "m" }, "z": { "value": 0, "unit": "m" } } })).unwrap();
    let all = compute(&p, Scope::Project, 1);
    assert_eq!(make(&all, "Plate").qty, 11, "10 from the machines and one from the other bracket");
    assert_eq!(make(&all, "Tab").qty, 1);
    let screws: Vec<_> = all.buy.iter().filter(|l| l.key == "buy:ISO 4762 M3×12 A2").collect();
    assert_eq!(screws.len(), 1);
    assert_eq!(screws[0].qty, 13);
    assert!(make(&all, "Plate").used_in.len() >= 2, "used in both places: {:?}", make(&all, "Plate").used_in);
    // A merged line says every definition its parts come from, so a view can find them all.
    let plate_uses = &make(&all, "Plate").uses;
    assert!(plate_uses.iter().any(|u| u.shape == k.bracket && u.part.as_deref() == Some("Plate")), "{plate_uses:?}");
    assert!(plate_uses.iter().any(|u| u.shape == other && u.part.as_deref() == Some("Plate")), "{plate_uses:?}");
    assert!(plate_uses.iter().any(|u| u.config.as_deref() == Some("Basic")) && plate_uses.iter().any(|u| u.config.as_deref() == Some("Pro")), "both configurations: {plate_uses:?}");
    assert_eq!(screws[0].uses.len(), 2, "the kit's screw and the loose one: {:?}", screws[0].uses);
    assert!(screws[0].uses.iter().any(|u| u.shape == k.screw && u.part.is_none()));

    // Parts a loop numbered one by one are one line under their common name.
    run(&mut p, "shape.create", json!({ "name": "Stand", "source": r#"const out = [];
        for (let i = 1; i <= 4; i++) out.push(part(`Foot ${i}`, cylinder(5, 8).translate(i * 30, 0, 0), { process: "buy" }));
        for (let i = 1; i <= 2; i++) out.push(part(`Rib ${i}`, box(50, 3, 20).translate(0, i * 20, 5), { material: "Steel", process: "laser" }));
        out.push(part("Rib 3", box(60, 3, 20).translate(0, 80, 5), { material: "Steel", process: "laser" }));
        return out;"#, "params": [], "materials": { "Steel": k.steel }, "placement": { "x": { "value": 0, "unit": "m" }, "y": { "value": 0, "unit": "m" }, "z": { "value": 0, "unit": "m" } } })).unwrap();
    let all = compute(&p, Scope::Project, 1);
    let feet = all.buy.iter().find(|l| l.key == "buy:?Foot").unwrap();
    assert_eq!((feet.name.as_str(), feet.qty), ("Foot", 4));
    assert_eq!(make(&all, "Rib").qty, 2, "Rib 1 and Rib 2 are the same part");
    assert_eq!(make(&all, "Rib 3").qty, 1, "Rib 3 is longer: its own line");
    assert_eq!(bom::base_name("M3 nut"), "M3 nut");
    assert_eq!(bom::base_name("Glide #12"), "Glide");
    assert_eq!(bom::base_name("608"), "608");
}

#[test]
fn packs_round_orders_up_and_breaks_price_production_runs() {
    assert_eq!(bom::pack_round(6, Some(100)), 100);
    assert_eq!(bom::pack_round(100, Some(100)), 100);
    assert_eq!(bom::pack_round(101, Some(100)), 200);
    assert_eq!(bom::pack_round(7, None), 7);
    assert_eq!(bom::pack_round(7, Some(1)), 7);

    let mut p = project();
    let k = kit(&mut p);
    let one = compute(&p, Scope::Entity(k.machine), 1);
    let screws = one.buy.iter().find(|l| l.name.starts_with("Screw")).unwrap();
    assert_eq!(screws.unit_price, Some(0.05), "the item's own price over its source's");
    assert!((screws.extended.unwrap() - 5.0).abs() < 1e-9, "a whole pack: 100 × 0.05");
    // 20 machines need 120 screws: two packs.
    let twenty = compute(&p, Scope::Entity(k.machine), 20);
    let screws = twenty.buy.iter().find(|l| l.name.starts_with("Screw")).unwrap();
    assert_eq!(screws.order_qty, 200);
    // Setup spreads over the batch: a made part costs less each at 100 than at 1.
    let plate = make(&one, "Plate");
    let b1 = one.totals.breaks.iter().find(|b| b.qty == 1).unwrap();
    let b100 = one.totals.breaks.iter().find(|b| b.qty == 100).unwrap();
    assert!(b100.per_unit < b1.per_unit, "{} vs {}", b100.per_unit, b1.per_unit);
    assert_eq!(plate.cost_source, "estimate");
    assert!(plate.cut_mm.unwrap() > 199.0 && plate.cut_mm.unwrap() < 201.0, "the plate's outline is 2 × (60 + 40): {:?}", plate.cut_mm);
    // The printed fan's material is its mass at the PLA price.
    let fan = make(&one, "Fan");
    let kg = 20.0 * 20.0 * 10.0 / 1e9 * 1240.0;
    assert!((fan.mass_kg.unwrap() - kg).abs() < 1e-9);
    assert!((fan.estimate.material.unwrap() - kg * 25.0).abs() < 1e-9);
    assert!(fan.estimate.hours.unwrap() > 0.0);
}

fn supplier(p: &mut Project, name: &str, kind: &[&str]) -> StableId {
    id(&run(p, "sourcing.supplier.create", json!({ "name": name, "kind": kind, "email": format!("{}@example.com", name.to_lowercase().replace(' ', "")), "country": "US", "currency": "USD" })).unwrap())
}

#[test]
fn an_awarded_price_overrides_the_estimate() {
    let mut p = project();
    let k = kit(&mut p);
    let before = compute(&p, Scope::Entity(k.machine), 1);
    let plate = make(&before, "Plate").clone();
    let laser = supplier(&mut p, "Laser Co", &["laser"]);
    let quote = id(&run(
        &mut p,
        "sourcing.quote.create",
        json!({ "supplier": laser, "reference": "Q-1001", "lines": [{ "line": plate.key, "prices": [{ "qty": 1, "unit_price": 9.0 }, { "qty": 50, "unit_price": 7.5 }] }], "lead_days": 7, "valid_until": "2026-12-31" }),
    )
    .unwrap());
    run(&mut p, "sourcing.quote.award", json!({ "quote": quote, "names": { (plate.key.clone()): "Plate" } })).unwrap();
    let after = compute(&p, Scope::Entity(k.machine), 1);
    let line = make(&after, "Plate");
    assert_eq!(line.cost_source, "award");
    assert_eq!(line.unit_cost, Some(9.0), "5 plates: below the 50 break");
    assert_eq!(line.award.as_ref().unwrap().supplier_name, "Laser Co");
    assert_eq!(line.award.as_ref().unwrap().lead_days, Some(7));
    assert!((line.extended.unwrap() - 45.0).abs() < 1e-9);
    // 10 machines: 50 plates take the lower price.
    let ten = compute(&p, Scope::Entity(k.machine), 10);
    assert_eq!(make(&ten, "Plate").unit_cost, Some(7.5));
    let b100 = after.totals.breaks.iter().find(|b| b.qty == 100).unwrap();
    let est = make(&before, "Plate").estimate.unit(500).unwrap();
    assert!(b100.make > 0.0 && (est - 7.5).abs() > 1e-6);
    // The quote is accepted; clearing the award goes back to the estimate.
    let q: de_sourcing::Quote = serde_json::from_value(p.entity_state(quote).unwrap()["sourcing.quote"].clone()).unwrap();
    assert_eq!(q.status, de_sourcing::QuoteStatus::Accepted);
    run(&mut p, "sourcing.award.clear", json!({ "line": plate.key })).unwrap();
    let cleared = compute(&p, Scope::Entity(k.machine), 1);
    assert_eq!(make(&cleared, "Plate").cost_source, "estimate");
    assert_eq!(make(&cleared, "Plate").unit_cost, plate.unit_cost);

    // Declining a quote withdraws its awards.
    run(&mut p, "sourcing.quote.award", json!({ "quote": quote })).unwrap();
    let v = run(&mut p, "sourcing.quote.update", json!({ "target": quote, "status": "declined" })).unwrap();
    assert_eq!(v["awards_withdrawn"], 1);
    assert_eq!(make(&compute(&p, Scope::Entity(k.machine), 1), "Plate").cost_source, "estimate");

    // A stated award on a bought line replaces its list price.
    let dist = supplier(&mut p, "Bolt Depot", &["distributor"]);
    run(&mut p, "sourcing.award", json!({ "line": "buy:ISO 4762 M3×12 A2", "supplier": dist, "prices": [{ "qty": 100, "unit_price": 0.03 }] })).unwrap();
    let screws = compute(&p, Scope::Entity(k.machine), 1).buy.into_iter().find(|l| l.name.starts_with("Screw")).unwrap();
    assert_eq!((screws.unit_price, screws.cost_source), (Some(0.03), "award"));

    // Bad input is refused with reasons.
    assert!(run(&mut p, "sourcing.supplier.create", json!({ "name": "X", "kind": ["juggling"] })).unwrap_err().message.contains("juggling"));
    assert!(run(&mut p, "sourcing.quote.create", json!({ "supplier": laser, "lines": [{ "line": "make:1", "prices": [{ "qty": 0, "unit_price": 1 }] }] })).is_err());
    assert!(run(&mut p, "sourcing.rfq.create", json!({ "name": "R", "lines": [] })).unwrap_err().message.contains("at least one BOM line"));
}

#[test]
fn an_rfq_package_gives_each_supplier_exactly_their_lines_files() {
    let mut p = project();
    let k = kit(&mut p);
    let b = compute(&p, Scope::Entity(k.machine), 1);
    let laser = supplier(&mut p, "Laser Co", &["laser"]);
    let print = supplier(&mut p, "Print Farm", &["3d_print"]);
    let mill = supplier(&mut p, "Mill Works", &["cnc_mill", "lathe"]);
    let dist = supplier(&mut p, "Bolt Depot", &["distributor"]);
    let lines: Vec<Value> = b.make.iter().map(|l| json!({ "line": l.key, "name": l.name, "qty": l.qty })).chain(b.buy.iter().map(|l| json!({ "line": l.key, "name": l.name, "qty": l.qty }))).collect();
    let rfq_id = id(&run(&mut p, "sourcing.rfq.create", json!({ "name": "Machine pilot run", "scope": k.machine, "lines": lines, "quantities": [10, 1, 100, 10], "due": "2026-11-01", "suppliers": [laser, print, mill, dist], "finish": "Deburred, no paint" })).unwrap());
    let rfq: Rfq = serde_json::from_value(p.entity_state(rfq_id).unwrap()["sourcing.rfq"].clone()).unwrap();
    assert_eq!(rfq.quantities, vec![1, 10, 100], "sorted, without repeats");
    let suppliers: Vec<(StableId, Supplier)> = rfq.suppliers.iter().map(|s| (*s, serde_json::from_value(p.entity_state(*s).unwrap()["sourcing.supplier"].clone()).unwrap())).collect();
    let programs = bom::line_programs(p.world(), &b);
    let mut files = package::Files::new(&programs, 0.05, package::Context { project: "Test".into(), date: "2026-10-11".into(), from: "Ada".into() });
    let all = package::rfq_package(&rfq, &b, &suppliers, &mut files);
    let names: BTreeSet<String> = all.iter().map(|(n, _)| n.clone()).collect();
    let item = |n: &str| b.make.iter().find(|l| l.name == n).map(|l| l.item).or_else(|| b.buy.iter().find(|l| l.name.starts_with(n)).map(|l| l.item)).unwrap();
    let f = |folder: &str, rest: &[String]| -> Vec<String> { rest.iter().map(|r| format!("{folder}/{r}")).chain(["BOM.csv", "RFQ.md"].iter().map(|r| format!("{folder}/{r}"))).collect() };
    let mut want: BTreeSet<String> = BTreeSet::new();
    want.extend(f("Laser_Co", &[format!("{:02}-Plate.dxf", item("Plate"))]));
    want.extend(f("Print_Farm", &[format!("{:02}-Fan.stl", item("Fan")), format!("{:02}-Fan.3mf", item("Fan"))]));
    want.extend(f("Mill_Works", &[format!("{:02}-Pin.step", item("Pin")), format!("{:02}-Pin.pdf", item("Pin"))]));
    want.extend(f("Bolt_Depot", &[]));
    assert_eq!(names, want, "exactly each supplier's files (no NOTES.txt: every file was made)");

    let get = |n: &str| String::from_utf8_lossy(&all.iter().find(|(p, _)| p == n).unwrap().1).to_string();
    assert!(get(&format!("Mill_Works/{:02}-Pin.step", item("Pin"))).starts_with("ISO-10303-21"));
    assert!(all.iter().find(|(p, _)| p.ends_with("Pin.pdf")).unwrap().1.starts_with(b"%PDF"));
    assert!(get(&format!("Laser_Co/{:02}-Plate.dxf", item("Plate"))).contains("DEPTH_"));
    let dist_csv = get("Bolt_Depot/BOM.csv");
    assert!(dist_csv.contains("ISO 4762 M3×12 A2") && dist_csv.contains("Grommet") && !dist_csv.contains("Plate"), "{dist_csv}");
    assert!(dist_csv.lines().next().unwrap().contains("Qty for 100 units"));
    let letter = get("Laser_Co/RFQ.md");
    assert!(letter.contains("Laser Co team") && letter.contains("2026-11-01") && letter.contains("Deburred, no paint") && letter.contains("ISO 2768-m"), "{letter}");
    assert!(letter.contains("| 5 | 50 | 500 |"), "quantities per production run: {letter}");
    assert!(!letter.contains("Pin"));
    let mail = package::mailto(&rfq, &suppliers[0].1, 1, &package::Context::default());
    assert!(mail.starts_with("mailto:laserco@example.com?subject=Request%20for%20quote"));

    // One supplier's package on its own, and the zip opens.
    let (one, problems) = package::supplier_package(&rfq, &b, &suppliers[2].1, &mut files, "");
    assert!(problems.is_empty());
    assert_eq!(one.len(), 4);
    let zip = de_shape::fabricate::zip(&one);
    assert_eq!(&zip[..4], b"PK\x03\x04");
}

#[test]
fn materials_without_cost_fields_load_and_read_as_unpriced() {
    // A material saved before costs existed.
    let old: de_solid::Material = serde_json::from_value(json!({ "name": "Oak", "density": { "value": 750, "unit": "kg/m3" } })).unwrap();
    assert!(old.cost.is_none());
    assert!(!serde_json::to_value(&old).unwrap().as_object().unwrap().contains_key("cost"), "saved as before");

    let mut p = project();
    let oak = id(&run(&mut p, "solid.material.create", json!({ "name": "Oak", "density": { "value": 750, "unit": "kg/m3" } })).unwrap());
    run(&mut p, "shape.create", json!({ "name": "Shelf", "source": r#"return [part("Board", box(800, 250, 20), { material: "Oak", process: "cnc_router" })];"#, "params": [], "materials": { "Oak": oak }, "placement": { "x": { "value": 0, "unit": "m" }, "y": { "value": 0, "unit": "m" }, "z": { "value": 0, "unit": "m" } } })).unwrap();
    let b = compute(&p, Scope::Project, 1);
    let board = make(&b, "Board");
    assert!(board.estimate.material.is_none() && board.estimate.process.is_some(), "router time is known, the oak's price is not");
    assert!(b.flags.iter().any(|f| f.text.contains("Oak has no price")));
    run(&mut p, "solid.material.set_cost", json!({ "target": oak, "cost": { "per_kg": 3.0 } })).unwrap();
    let b = compute(&p, Scope::Project, 1);
    let board = make(&b, "Board");
    let block = 800.0 * 250.0 * 20.0 / 1e9 * 750.0;
    assert!((board.estimate.material.unwrap() - block * 3.0).abs() < 1e-6, "a router part is cut from a block its size");
    assert!(run(&mut p, "solid.material.set_cost", json!({ "target": oak, "cost": { "per_kg": -1 } })).is_err());

    // Rates are project settings: a changed rate changes the estimate.
    let before = board.estimate.process.unwrap();
    run(&mut p, "sourcing.rates.set", json!({ "currency": "EUR", "processes": { "cnc_router": { "per_hour": 150, "setup_hours": 0.5, "basis": { "basis": "cut_length", "mm_per_minute": 2500, "pierce_seconds": 4 } } } })).unwrap();
    let b = compute(&p, Scope::Project, 1);
    assert!((make(&b, "Board").estimate.process.unwrap() - 2.0 * before).abs() < 1e-9);
    assert_eq!(b.totals.currency, "EUR");
}

/// Another source of Buy lines (a stand-in for a PCB's components).
struct Harness;

impl BuySource for Harness {
    fn name(&self) -> &'static str {
        "harness"
    }

    fn items(&self, world: &World, entity: StableId, _config: Option<&str>) -> Option<Vec<BuyItem>> {
        let e = world.resource::<de_core::IdIndex>().get(entity)?;
        let p = world.get::<de_shape::ShapeProgram>(e)?;
        (p.name == "Bracket").then(|| vec![BuyItem { name: "JST-XH 2 pin".into(), stock: Some(Stock { part_number: "B2B-XH-A".into(), unit_price: Some(0.1), pack: Some(10), ..Default::default() }), qty: 2, mass_kg: None }])
    }
}

#[test]
fn other_sources_add_buy_lines_that_roll_up_too() {
    let mut p = project();
    let k = kit(&mut p);
    let b = bom::compute(p.world(), Scope::Entity(k.machine), &Options::default(), &[&Harness]).unwrap();
    let jst = b.buy.iter().find(|l| l.key == "buy:B2B-XH-A").unwrap();
    assert_eq!((jst.qty, jst.order_qty, jst.origin.as_str()), (10, 10, "harness"), "two per bracket, five brackets");
    let csv = bom::flat_csv(&b);
    assert!(csv.lines().count() == 1 + b.make.len() + b.buy.len());
    let indented = bom::indented_csv(&b);
    assert!(indented.contains("JST-XH 2 pin"));
}

#[test]
fn inserted_stock_library_items_are_buy_lines_with_their_catalogue_facts() {
    let mut p = project();
    let rig = id(&run(&mut p, "assembly.create", json!({ "name": "Rig" })).unwrap());
    for i in 0..12 {
        run(&mut p, "stock.insert", json!({ "family": "iso4762", "size": "M3×12", "parent": rig, "name": format!("S{i}"), "at": [i as f64 * 10.0, 0, 0] })).unwrap();
    }
    for i in 0..2 {
        run(&mut p, "stock.insert", json!({ "family": "bearing_deep_groove", "size": "608-2RS", "parent": rig, "name": format!("B{i}") })).unwrap();
    }
    run(&mut p, "assembly.instance.create", json!({ "of": rig, "name": "Rig 1", "at": [0, 0, 0] })).unwrap();
    run(&mut p, "assembly.instance.create", json!({ "of": rig, "name": "Rig 2", "at": [0, 0, 0] })).unwrap();

    let b = compute(&p, Scope::Project, 10);
    assert!(b.make.is_empty(), "stock items are bought, not made: {:?}", b.make.iter().map(|l| &l.name).collect::<Vec<_>>());
    let screws = b.buy.iter().find(|l| l.stock.as_ref().is_some_and(|s| s.part_number.contains("M3×12"))).expect("the screws");
    let s = screws.stock.as_ref().unwrap();
    assert_eq!(screws.qty, 24, "12 per rig, two rigs");
    assert_eq!(s.pack, Some(100));
    assert_eq!(screws.order_qty, 300, "240 for ten units: three packs");
    let price = s.unit_price.expect("a typical price");
    assert_eq!(screws.unit_price, Some(price));
    assert!((screws.extended.unwrap() - 300.0 * price).abs() < 1e-9);
    assert!((screws.mass_kg.unwrap() - s.mass_g.unwrap() / 1000.0).abs() < 1e-12, "mass from massG");
    let bearings = b.buy.iter().find(|l| l.stock.as_ref().is_some_and(|s| s.part_number.contains("608"))).expect("the bearings");
    assert_eq!(bearings.qty, 4);
    let pack = bearings.stock.as_ref().unwrap().pack;
    assert_eq!(bearings.order_qty, bom::pack_round(40, pack));
    assert!(bearings.mass_kg.is_some());
    assert!(!b.flags.iter().any(|f| f.level == "warning"), "{:?}", b.flags);
    let unit_mass = 24.0 * screws.mass_kg.unwrap() + 4.0 * bearings.mass_kg.unwrap();
    assert!((b.totals.mass_kg.unwrap() - unit_mass).abs() < 1e-9);
}

#[test]
fn cut_to_length_stock_is_costed_by_its_length() {
    let mut p = project();
    // A program declaring a piece cut to length with a price per metre and no unitPrice.
    let src = r#"stock({ partNumber: `RHS 40×20×2 × ${p.length} S235JRH`, cutLength: p.length, pricePerM: 4.2, massG: 1.7 * p.length });
        return box(40, 20, p.length);"#;
    run(&mut p, "shape.create", json!({ "name": "Rail", "source": src, "params": [{ "key": "length", "label": "Length", "value": 2350, "unit": "mm", "min": 10 }], "placement": { "x": { "value": 0, "unit": "m" }, "y": { "value": 0, "unit": "m" }, "z": { "value": 0, "unit": "m" } } })).unwrap();
    // A library beam at its default length.
    run(&mut p, "stock.insert", json!({ "family": "ipe", "size": "IPE 200" })).unwrap();

    let b = compute(&p, Scope::Project, 3);
    let rail = b.buy.iter().find(|l| l.stock.as_ref().is_some_and(|s| s.part_number == "RHS 40×20×2 × 2350 S235JRH")).expect("the rail");
    let s = rail.stock.as_ref().unwrap();
    assert_eq!((s.cut_length_mm, s.price_per_m), (Some(2350.0), Some(4.2)));
    assert_eq!(rail.unit_price, Some(9.87), "2.35 m at 4.20 a metre");
    assert_eq!(rail.order_qty, 3);
    assert!((rail.length_m.unwrap() - 7.05).abs() < 1e-9, "three pieces of 2.35 m: {:?}", rail.length_m);
    assert!((rail.extended.unwrap() - 3.0 * 9.87).abs() < 1e-9);
    let csv = bom::flat_csv(&b);
    assert!(csv.lines().any(|l| l.contains("RHS 40×20×2 × 2350 S235JRH") && l.contains(",1,3,2350.0,")), "the CSV gives the cut length:\n{csv}");

    let beam = b.buy.iter().find(|l| l.stock.as_ref().is_some_and(|s| s.part_number.starts_with("IPE 200 × "))).expect("the beam");
    let s = beam.stock.as_ref().unwrap();
    let (len, per_m) = (s.cut_length_mm.expect("a cut length"), s.price_per_m.expect("a price per metre"));
    assert_eq!(s.part_number, format!("IPE 200 × {len} S235JR"));
    assert!((beam.unit_price.unwrap() - per_m * len / 1000.0).abs() < 0.006, "the piece is priced by its length");
    assert!((beam.mass_kg.unwrap() - 22.4 * len / 1000.0).abs() < 1e-6, "22.4 kg/m");
    assert!((beam.length_m.unwrap() - 3.0 * len / 1000.0).abs() < 1e-9);
}
