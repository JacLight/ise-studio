//! The building vocabulary (bim.js) on a real model: the example house.

use de_shape::runtime;

fn house_params() -> Vec<(String, f64)> {
    [("width", 10000.0), ("depth", 8000.0), ("wall", 300.0), ("partition", 100.0), ("groundHeight", 3000.0), ("firstHeight", 2800.0), ("floorSlab", 200.0), ("hallDepth", 3500.0), ("roofSlope", 35.0)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect()
}

#[test]
fn structure_and_envelope_elements() {
    let src = r#"
        const L0 = level("Ground", 0, { height: 4000 });
        const G = grid("Grid", { x: [0, 6000], y: [0, 5000], level: L0 });
        const parts = [];
        for (let i = 0; i < 2; i++) for (let j = 0; j < 2; j++) parts.push(...column(`C${i}${j}`, { at: G.at(i, j), section: "HEB 200", level: L0 }));
        parts.push(...beam("B1", { from: [0, 0, 4000], to: [6000, 0, 4000], section: "IPE 300", level: L0 }));
        parts.push(...beam("B2", { from: [0, 5000, 4000], to: [6000, 5000, 3800], size: [200, 400], level: L0 }));
        parts.push(...curtainWall("Glazing", { from: [0, 5000], to: [6000, 5000], height: 3500, panel: [1500, 1750], level: L0 }));
        parts.push(...ramp("Ramp", { at: [0, -2000], dir: 0, length: 6000, width: 1200, rise: 500, level: L0 }));
        parts.push(...roof("Roof", { x: 0, y: 0, width: 6000, depth: 5000, kind: "hip", slope: 30, level: level("Eaves", 4000) }));
        return parts;
    "#;
    let e = runtime::evaluate(src, &[], 2.0).unwrap_or_else(|e| panic!("{e}"));
    let bim = e.declared.bim.as_ref().unwrap();
    let class = |n: &str| bim.elements[n].class.as_str();
    assert_eq!(class("C00"), "IfcColumn");
    assert_eq!(bim.elements["C00"].type_name.as_deref(), Some("HEB 200"));
    assert_eq!(class("B1"), "IfcBeam");
    assert_eq!(class("Glazing mullions"), "IfcMember");
    assert_eq!(bim.elements["Glazing panels"].parent.as_deref(), Some("Glazing"));
    assert_eq!(bim.groups[0].class, "IfcCurtainWall");
    assert_eq!(class("Ramp"), "IfcRamp");
    assert_eq!(bim.elements["Roof"].predefined.as_deref(), Some("HIP_ROOF"));
    // An HEB 200 is 200 deep; the beam hangs below its line: its top at 4000.
    let b1 = e.part("B1").unwrap();
    assert!((b1.bounds_max[2] - 4000.0).abs() < 1e-6 && (b1.bounds_min[2] - 3700.0).abs() < 1e-6, "{:?}", (b1.bounds_min, b1.bounds_max));
    let c = e.part("C00").unwrap();
    assert!((c.volume - 7808.0 * 4000.0).abs() / (7808.0 * 4000.0) < 0.05, "HEB 200 area {}", c.volume / 4000.0);
}

#[test]
fn house_declares_its_building() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../examples/house/house.js")).unwrap();
    let e = runtime::evaluate(&src, &house_params(), 5.0).unwrap_or_else(|e| panic!("{e}"));
    let bim = e.declared.bim.as_ref().expect("declared");
    assert_eq!(bim.levels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Ground floor", "First floor", "Roof"]);
    assert_eq!(bim.site.as_ref().unwrap().name, "Plot 12, Orchard Lane");
    assert_eq!(bim.spaces.len(), 7);
    assert_eq!(bim.zones.len(), 2);
    assert_eq!(bim.grids[0].u.len(), 3);
    // Every part is an element; doors and windows fill openings in their wall.
    for p in &e.parts {
        assert!(bim.elements.contains_key(&p.name), "{} has no IFC class", p.name);
    }
    let count = |c: &str| bim.elements.values().filter(|x| x.class == c).count();
    assert_eq!(count("IfcDoor"), 7, "doors");
    assert_eq!(count("IfcWindow"), 10, "windows");
    assert_eq!(count("IfcStair"), 1);
    assert_eq!(count("IfcRoof"), 1);
    assert_eq!(count("IfcReinforcingBar"), 2);
    assert_eq!(bim.openings.len(), 17);
    for o in &bim.openings {
        let host = &bim.elements[&o.host];
        assert_eq!(host.class, "IfcWall", "{} is in {}", o.name, o.host);
        // The door or window stands in its wall's opening.
        let (fill, wall) = (e.part(o.fills.as_deref().unwrap()).unwrap(), e.part(&o.host).unwrap());
        assert!(fill.bounds_min[2] >= wall.bounds_min[2] - 1e-6 && fill.bounds_max[2] <= wall.bounds_max[2] + 1e-6, "{} is not in {}", o.name, o.host);
        assert!((fill.bounds_min[2] - o.origin[2]).abs() < 11.0, "{} stands at {} not {}", o.name, fill.bounds_min[2], o.origin[2]);
    }
    // Wall quantities: the south wall's net side area is its gross less its two openings.
    let south = &bim.elements["GF south wall"];
    let q = &south.quantities;
    assert!((q["Length"] - 10000.0).abs() < 1e-9);
    assert!((q["GrossSideArea"] - 10.0 * 2.8).abs() < 1e-9);
    assert!((q["NetSideArea"] - (28.0 - 1.5 * 1.2 - 1.0 * 2.2)).abs() < 1e-9, "{q:?}");
    // The stair: 17 risers of 176.5 mm.
    let stair = &bim.elements["Stair"];
    assert_eq!(stair.quantities["NumberOfRisers"], 17.0);
    // The living room: 4.85 × 7.7 m.
    let living = bim.spaces.iter().find(|s| s.name == "Living").unwrap();
    assert!((living.area() - 4.8 * 7.7).abs() < 1e-9, "{}", living.area());
    // Nothing overlaps: the walls, slabs and roof are separate solids that only touch.
    let names = ["GF south wall", "GF east wall", "FF south wall", "FF east gable", "Roof", "First floor slab"];
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            let (pa, pb) = (e.part(a).unwrap(), e.part(b).unwrap());
            let v = pa.solid.intersection(&pb.solid).volume();
            assert!(v < 1e3, "{a} and {b} overlap by {v} mm³");
        }
    }
}
