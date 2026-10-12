//! IFC write → read round trips and reading other tools' files.

use de_bim::*;
use std::collections::BTreeMap;

fn block(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Mesh {
    let m = geom::extrude(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], &[], [0.0, 0.0, z1 - z0]);
    let mut m = geom::solid(&m, 1e-9).unwrap();
    for p in &mut m.positions {
        p[2] += z0;
    }
    m
}

fn element(name: &str, class: &str, level: &str, mesh: Mesh) -> ModelElement {
    let volume = geom::volume(&mesh);
    ModelElement { name: name.into(), key: name.into(), info: Element { class: class.into(), level: Some(level.into()), ..Default::default() }, material: Some("Concrete".into()), process: None, mesh, volume }
}

fn house() -> Model {
    let mut wall = element("Wall A", "IfcWall", "Ground floor", block(0.0, 0.0, 0.0, 5000.0, 200.0, 2700.0));
    wall.info.predefined = Some("STANDARD".into());
    wall.info.psets.entry("Pset_WallCommon".into()).or_default().insert("IsExternal".into(), PValue::Bool(true));
    wall.info.psets.entry("Pset_WallCommon".into()).or_default().insert("FireRating".into(), PValue::Text("REI 60".into()));
    wall.info.quantities.insert("Length".into(), 5000.0);
    wall.info.quantities.insert("NetSideArea".into(), 11.61);
    wall.info.classification.push(Classification { system: "Uniclass 2015".into(), code: "EF_25_10".into(), name: Some("Walls".into()) });
    let mut door = element("D01", "IfcDoor", "Ground floor", block(1000.0, 0.0, 0.0, 1900.0, 200.0, 2100.0));
    door.info.host = Some("Wall A".into());
    door.info.tag = Some("D01".into());
    door.info.attributes.insert("OverallWidth".into(), PValue::Num(900.0));
    door.info.attributes.insert("OverallHeight".into(), PValue::Num(2100.0));
    door.info.attributes.insert("OperationType".into(), PValue::Text("SINGLE_SWING_LEFT".into()));
    door.material = Some("Oak — oiled".into());
    let slab = element("Floor 1", "IfcSlab", "First floor", block(0.0, 0.0, 2800.0, 5000.0, 4000.0, 3000.0));
    let mut mullion = element("CW mullions", "IfcMember", "First floor", block(0.0, 3800.0, 3000.0, 100.0, 3900.0, 5800.0));
    mullion.info.parent = Some("Curtain wall".into());
    Model {
        project: "Test house".into(),
        site: Site { name: "Plot 7".into(), latitude: Some(51.5), longitude: Some(-0.12), elevation: Some(12.0), north: Some(15.0), ..Default::default() },
        building: Building { name: "House".into(), address: Some("1 Lane, Town".into()), ..Default::default() },
        levels: vec![Level { name: "Ground floor".into(), elevation: 0.0, height: Some(3000.0) }, Level { name: "First floor".into(), elevation: 3000.0, height: Some(2800.0) }],
        elements: vec![wall, door, slab, mullion],
        spaces: vec![Space {
            name: "Living".into(),
            long_name: Some("Living room".into()),
            level: Some("Ground floor".into()),
            outline: vec![vec![[200.0, 200.0], [4800.0, 200.0], [4800.0, 3800.0], [200.0, 3800.0]], vec![[1000.0, 1000.0], [1000.0, 1500.0], [1500.0, 1500.0], [1500.0, 1000.0]]],
            elevation: 0.0,
            height: 2700.0,
            predefined: Some("INTERNAL".into()),
            ..Default::default()
        }],
        openings: vec![Opening { name: "D01 opening".into(), host: "Wall A".into(), origin: [1000.0, -10.0, 0.0], dir: [1.0, 0.0], size: [900.0, 220.0, 2100.0], fills: Some("D01".into()) }],
        zones: vec![Zone { name: "Flat 1".into(), spaces: vec!["Living".into()], psets: BTreeMap::new() }],
        grids: vec![Grid {
            name: "Grid".into(),
            level: Some("Ground floor".into()),
            u: vec![GridAxis { tag: "A".into(), from: [0.0, -1000.0], to: [0.0, 5000.0] }, GridAxis { tag: "B".into(), from: [5000.0, -1000.0], to: [5000.0, 5000.0] }],
            v: vec![GridAxis { tag: "1".into(), from: [-1000.0, 0.0], to: [6000.0, 0.0] }],
        }],
        groups: vec![Group { name: "Curtain wall".into(), class: "IfcCurtainWall".into(), level: Some("First floor".into()), ..Default::default() }],
        notes: vec![],
    }
}

#[test]
fn round_trip_keeps_structure_properties_and_geometry() {
    let model = house();
    let text = ifc_write::write(&model, &ifc_write::Options::default());
    assert!(text.starts_with("ISO-10303-21;"));
    assert!(text.contains("FILE_SCHEMA(('IFC4'))"));
    // Every reference points at an entity that exists.
    let f = step::parse(&text).unwrap();
    for (id, e) in &f.entities {
        fn walk(a: &step::Arg, f: &step::File, id: u64) {
            match a {
                step::Arg::Ref(r) => assert!(f.get(*r).is_some(), "#{id} refers to missing #{r}"),
                step::Arg::List(l) => l.iter().for_each(|x| walk(x, f, id)),
                step::Arg::Typed(_, x) => walk(x, f, id),
                _ => {}
            }
        }
        e.args.iter().for_each(|a| walk(a, &f, *id));
    }
    let back = ifc_read::read(&text).unwrap();
    assert_eq!(back.schema, "IFC4");
    assert_eq!(back.project, "Test house");
    assert_eq!(back.site.name, "Plot 7");
    assert!((back.site.latitude.unwrap() - 51.5).abs() < 1e-6, "{:?}", back.site.latitude);
    assert!((back.site.longitude.unwrap() + 0.12).abs() < 1e-6);
    assert!((back.site.elevation.unwrap() - 12.0).abs() < 1e-9);
    assert!((back.site.north.unwrap() - 15.0).abs() < 1e-6);
    assert_eq!(back.building.address.as_deref(), Some("1 Lane, Town"));
    assert_eq!(back.levels, model.levels);
    assert_eq!(back.parts.len(), 4);
    let wall = back.parts.iter().find(|p| p.name == "Wall A").unwrap();
    assert_eq!(wall.info.class, "IfcWall");
    assert_eq!(wall.info.predefined.as_deref(), Some("STANDARD"));
    assert_eq!(wall.info.level.as_deref(), Some("Ground floor"));
    assert_eq!(wall.info.psets, model.elements[0].info.psets);
    assert_eq!(wall.info.quantities, model.elements[0].info.quantities);
    assert_eq!(wall.info.classification, model.elements[0].info.classification);
    assert_eq!(wall.material.as_deref(), Some("Concrete"));
    assert_eq!(wall.solids.len(), 1);
    assert!((geom::volume(&wall.solids[0]) - 5000.0 * 200.0 * 2700.0).abs() < 1.0);
    // The opening comes back as a void in its host, in place.
    assert_eq!(wall.voids.len(), 1);
    let (lo, hi) = geom::bounds(&wall.voids[0]);
    assert!((lo[0] - 1000.0).abs() < 1e-6 && (hi[0] - 1900.0).abs() < 1e-6 && (hi[2] - 2100.0).abs() < 1e-6, "{lo:?} {hi:?}");
    let door = back.parts.iter().find(|p| p.name == "D01").unwrap();
    assert_eq!(door.info.host.as_deref(), Some("Wall A"));
    assert_eq!(door.info.tag.as_deref(), Some("D01"));
    assert_eq!(door.material.as_deref(), Some("Oak — oiled"));
    assert_eq!(door.info.attributes.get("OverallWidth"), Some(&PValue::Num(900.0)));
    assert_eq!(door.info.attributes.get("OperationType"), Some(&PValue::Text("SINGLE_SWING_LEFT".into())));
    // The opening itself comes back as an opening, hosted and filled.
    assert_eq!(back.openings.len(), 1);
    let o = &back.openings[0];
    assert_eq!((o.host.as_str(), o.fills.as_deref(), o.name.as_str()), ("Wall A", Some("D01"), "D01 opening"));
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    assert!(close(&o.origin, &[1000.0, -10.0, 0.0]) && close(&o.size, &[900.0, 220.0, 2100.0]) && close(&o.dir, &[1.0, 0.0]), "{o:?}");
    // A slab on the first floor sits at its true height (the storey's frame is 3 m up).
    let slab = back.parts.iter().find(|p| p.name == "Floor 1").unwrap();
    let (lo, hi) = geom::bounds(&slab.solids[0]);
    assert!((lo[2] - 2800.0).abs() < 1e-6 && (hi[2] - 3000.0).abs() < 1e-6);
    assert_eq!(slab.info.level.as_deref(), Some("First floor"));
    let mullion = back.parts.iter().find(|p| p.name == "CW mullions").unwrap();
    assert_eq!(mullion.info.parent.as_deref(), Some("Curtain wall"));
    assert_eq!(mullion.info.level.as_deref(), Some("First floor"));
    assert_eq!(back.groups.len(), 1);
    assert_eq!(back.groups[0].class, "IfcCurtainWall");
    assert_eq!(back.groups[0].level.as_deref(), Some("First floor"));
    // The space: outline with its hole, height, area.
    assert_eq!(back.spaces.len(), 1);
    let s = &back.spaces[0];
    assert_eq!(s.long_name.as_deref(), Some("Living room"));
    assert_eq!(s.level.as_deref(), Some("Ground floor"));
    assert_eq!(s.predefined.as_deref(), Some("INTERNAL"));
    assert!((s.area() - model.spaces[0].area()).abs() < 1e-9, "{} vs {}", s.area(), model.spaces[0].area());
    assert!((s.height - 2700.0).abs() < 1e-6);
    assert_eq!(back.zones[0].spaces, vec!["Living".to_string()]);
    assert_eq!(back.grids.len(), 1);
    let g = &back.grids[0];
    assert_eq!(g.u.iter().map(|a| a.tag.as_str()).collect::<Vec<_>>(), vec!["A", "B"]);
    assert_eq!(g.v[0].tag, "1");
    assert_eq!(g.v[0].to, [6000.0, 0.0]);
    assert!(back.notes.is_empty(), "{:?}", back.notes);
    // Writing it again gives the same ids (stable GlobalIds).
    assert_eq!(text, ifc_write::write(&model, &ifc_write::Options::default()));
}

#[test]
fn reads_ifc2x3_in_metres_with_extrusions_mapped_items_and_breps() {
    let text = r#"ISO-10303-21;
HEADER;FILE_DESCRIPTION((''),'2;1');FILE_NAME('a.ifc','',(''),(''),'','','');FILE_SCHEMA(('IFC2X3'));ENDSEC;
DATA;
#1=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#2=IFCUNITASSIGNMENT((#1));
#3=IFCPROJECT('0YvctVUKr0kugbFTf53O9L',$,'Other tool',$,$,$,$,$,#2);
#10=IFCCARTESIANPOINT((0.,0.,0.));
#11=IFCAXIS2PLACEMENT3D(#10,$,$);
#12=IFCLOCALPLACEMENT($,#11);
#13=IFCBUILDINGSTOREY('1YvctVUKr0kugbFTf53O9L',$,'Level 1',$,$,#12,$,$,.ELEMENT.,0.);
#20=IFCCARTESIANPOINT((2.,0.,0.));
#21=IFCAXIS2PLACEMENT3D(#20,$,$);
#22=IFCLOCALPLACEMENT(#12,#21);
#23=IFCAXIS2PLACEMENT2D(#10,$);
#24=IFCRECTANGLEPROFILEDEF(.AREA.,$,#23,0.4,0.4);
#25=IFCDIRECTION((0.,0.,1.));
#26=IFCEXTRUDEDAREASOLID(#24,#11,#25,3.);
#27=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#26));
#28=IFCPRODUCTDEFINITIONSHAPE($,$,(#27));
#29=IFCCOLUMN('2YvctVUKr0kugbFTf53O9L',$,'C1',$,$,#22,#28,'C1');
#30=IFCREPRESENTATIONMAP(#11,#27);
#31=IFCCARTESIANPOINT((5.,1.,0.));
#32=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#31,$,$);
#33=IFCMAPPEDITEM(#30,#32);
#34=IFCSHAPEREPRESENTATION($,'Body','MappedRepresentation',(#33));
#35=IFCPRODUCTDEFINITIONSHAPE($,$,(#34));
#36=IFCCOLUMN('3YvctVUKr0kugbFTf53O9L',$,'C2',$,$,#12,#35,$);
#40=IFCCARTESIANPOINT((0.,0.,0.));
#41=IFCCARTESIANPOINT((1.,0.,0.));
#42=IFCCARTESIANPOINT((0.,1.,0.));
#43=IFCCARTESIANPOINT((0.,0.,1.));
#44=IFCPOLYLOOP((#40,#42,#41));
#45=IFCPOLYLOOP((#40,#41,#43));
#46=IFCPOLYLOOP((#40,#43,#42));
#47=IFCPOLYLOOP((#41,#42,#43));
#48=IFCFACEOUTERBOUND(#44,.T.);
#49=IFCFACEOUTERBOUND(#45,.T.);
#50=IFCFACEOUTERBOUND(#46,.T.);
#51=IFCFACEOUTERBOUND(#47,.T.);
#52=IFCFACE((#48));
#53=IFCFACE((#49));
#54=IFCFACE((#50));
#55=IFCFACE((#51));
#56=IFCCLOSEDSHELL((#52,#53,#54,#55));
#57=IFCFACETEDBREP(#56);
#58=IFCSHAPEREPRESENTATION($,'Body','Brep',(#57));
#59=IFCPRODUCTDEFINITIONSHAPE($,$,(#58));
#60=IFCBUILDINGELEMENTPROXY('4YvctVUKr0kugbFTf53O9L',$,'Wedge',$,$,#12,#59,$,$);
#61=IFCMATERIAL('Steel');
#62=IFCMATERIALLAYER(#61,0.2,$);
#63=IFCMATERIALLAYERSET((#62),$);
#64=IFCMATERIALLAYERSETUSAGE(#63,.AXIS2.,.POSITIVE.,0.);
#65=IFCRELASSOCIATESMATERIAL('5YvctVUKr0kugbFTf53O9L',$,$,$,(#29,#36),#64);
#66=IFCRELCONTAINEDINSPATIALSTRUCTURE('6YvctVUKr0kugbFTf53O9L',$,$,$,(#29,#36,#60),#13);
#67=IFCQUANTITYLENGTH('Length',$,$,3.);
#68=IFCELEMENTQUANTITY('7YvctVUKr0kugbFTf53O9L',$,'Qto_ColumnBaseQuantities',$,$,(#67));
#69=IFCRELDEFINESBYPROPERTIES('8YvctVUKr0kugbFTf53O9L',$,$,$,(#29),#68);
ENDSEC;
END-ISO-10303-21;
"#;
    let r = ifc_read::read(text).unwrap();
    assert_eq!(r.schema, "IFC2X3");
    assert_eq!(r.levels.len(), 1);
    assert_eq!(r.parts.len(), 3, "{:?}", r.notes);
    let c1 = r.parts.iter().find(|p| p.name == "C1").unwrap();
    let (lo, hi) = geom::bounds(&c1.solids[0]);
    // 0.4 m square, 3 m tall, centred 2 m along X: millimetres.
    assert!((lo[0] - 1800.0).abs() < 1e-6 && (hi[0] - 2200.0).abs() < 1e-6 && (hi[2] - 3000.0).abs() < 1e-6, "{lo:?} {hi:?}");
    assert!((geom::volume(&c1.solids[0]) - 400.0 * 400.0 * 3000.0).abs() < 1e-3);
    assert_eq!(c1.material.as_deref(), Some("Steel"));
    assert_eq!(c1.info.quantities.get("Length"), Some(&3000.0));
    assert_eq!(c1.info.tag.as_deref(), Some("C1"));
    let c2 = r.parts.iter().find(|p| p.name == "C2").unwrap();
    let (lo, _) = geom::bounds(&c2.solids[0]);
    assert!((lo[0] - 4800.0).abs() < 1e-6 && (lo[1] - 800.0).abs() < 1e-6, "{lo:?}");
    let w = r.parts.iter().find(|p| p.name == "Wedge").unwrap();
    assert!((geom::volume(&w.solids[0]) - 1e9 / 6.0).abs() < 1e-3);
    assert_eq!(w.info.class, "IfcBuildingElementProxy");
    assert_eq!(w.info.level.as_deref(), Some("Level 1"));
}

#[test]
fn schedule_totals_by_level_and_class() {
    let mut model = house();
    model.settle();
    let s = schedule::schedule(&model, &schedule::Query { group_by: vec!["level".into(), "class".into()], ..Default::default() });
    let walls = s.groups.iter().find(|g| g.key == ["Ground floor", "IfcWall"]).unwrap();
    assert_eq!(walls.count, 1);
    assert!((walls.sums["Volume"] - 2.7).abs() < 1e-9);
    assert_eq!(walls.sums["Openings"], 1.0);
    let spaces = s.groups.iter().find(|g| g.key == ["Ground floor", "IfcSpace"]).unwrap();
    assert!((spaces.sums["NetFloorArea"] - (4.6 * 3.6 - 0.25)).abs() < 1e-9);
    let doors = schedule::schedule(&model, &schedule::Query { classes: vec!["door".into()], ..Default::default() });
    assert_eq!(doors.rows.len(), 1);
    assert_eq!(doors.rows[0].quantities["OverallWidth"], 900.0);
    let csv = schedule::csv(&doors, true);
    assert!(csv.lines().next().unwrap().contains("OverallWidth (mm)"), "{csv}");
    assert!(csv.contains("D01"));
    assert!(csv.lines().last().unwrap().starts_with("Total,"));
}
