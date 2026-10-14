//! IFC4 (ADD2 TC1) out: a STEP physical file of the whole model.
//!
//! Project › site › building › storeys (IfcRelAggregates); elements contained
//! in their storey (IfcRelContainedInSpatialStructure); spaces aggregated to
//! their storey; groups (curtain walls, stairs) aggregating their parts.
//! Bodies are IfcTriangulatedFaceSet in the storey's frame; openings and
//! spaces are extruded profiles (IfcExtrudedAreaSolid), an opening voiding
//! its host (IfcRelVoidsElement) and filled by its door or window
//! (IfcRelFillsElement). Materials, property sets, base quantities
//! (IfcElementQuantity), classification references, zones and grids go with
//! them. Units: millimetre, square metre, cubic metre, kilogram, radian.

use crate::step::{opt_string, r, real, refs, string, Writer};
use crate::{guid, Classification, Model, PValue, Psets};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// How a class's attributes run after Tag.
#[derive(Clone, Copy, PartialEq)]
enum Tail {
    /// PredefinedType
    Predefined,
    /// OverallHeight, OverallWidth, PredefinedType, OperationType, UserDefinedOperationType
    Door,
    /// OverallHeight, OverallWidth, PredefinedType, PartitioningType, UserDefinedPartitioningType
    Window,
    /// NumberOfRisers, NumberOfTreads, RiserHeight, TreadLength, PredefinedType
    StairFlight,
    /// SteelGrade, NominalDiameter, CrossSectionArea, BarLength, PredefinedType, BarSurface
    Bar,
    /// AssemblyPlace, PredefinedType
    Assembly,
    /// Nothing (IfcFurnishingElement)
    None,
}

/// Classes written as themselves, with the predefined types IFC4 allows.
const CLASSES: &[(&str, Tail, &str)] = &[
    ("IfcWall", Tail::Predefined, "MOVABLE PARAPET PARTITIONING PLUMBINGWALL SHEAR SOLIDWALL STANDARD POLYGONAL ELEMENTEDWALL"),
    ("IfcWallStandardCase", Tail::Predefined, "MOVABLE PARAPET PARTITIONING PLUMBINGWALL SHEAR SOLIDWALL STANDARD POLYGONAL ELEMENTEDWALL"),
    ("IfcSlab", Tail::Predefined, "FLOOR ROOF LANDING BASESLAB"),
    ("IfcRoof", Tail::Predefined, "FLAT_ROOF SHED_ROOF GABLE_ROOF HIP_ROOF HIPPED_GABLE_ROOF GAMBREL_ROOF MANSARD_ROOF BARREL_ROOF RAINBOW_ROOF BUTTERFLY_ROOF PAVILION_ROOF DOME_ROOF FREEFORM"),
    ("IfcBeam", Tail::Predefined, "BEAM JOIST HOLLOWCORE LINTEL SPANDREL T_BEAM"),
    ("IfcColumn", Tail::Predefined, "COLUMN PILASTER"),
    ("IfcMember", Tail::Predefined, "BRACE CHORD COLLAR MEMBER MULLION PLATE POST PURLIN RAFTER STRINGER STRUT STUD"),
    ("IfcPlate", Tail::Predefined, "CURTAIN_PANEL SHEET"),
    ("IfcDoor", Tail::Door, "DOOR GATE TRAPDOOR"),
    ("IfcWindow", Tail::Window, "WINDOW SKYLIGHT LIGHTDOME"),
    ("IfcStair", Tail::Predefined, "STRAIGHT_RUN_STAIR TWO_STRAIGHT_RUN_STAIR QUARTER_WINDING_STAIR QUARTER_TURN_STAIR HALF_WINDING_STAIR HALF_TURN_STAIR TWO_QUARTER_WINDING_STAIR TWO_QUARTER_TURN_STAIR THREE_QUARTER_WINDING_STAIR THREE_QUARTER_TURN_STAIR SPIRAL_STAIR DOUBLE_RETURN_STAIR CURVED_RUN_STAIR TWO_CURVED_RUN_STAIR"),
    ("IfcStairFlight", Tail::StairFlight, "STRAIGHT WINDER SPIRAL CURVED FREEFORM"),
    ("IfcRamp", Tail::Predefined, "STRAIGHT_RUN_RAMP TWO_STRAIGHT_RUN_RAMP QUARTER_TURN_RAMP TWO_QUARTER_TURN_RAMP HALF_TURN_RAMP SPIRAL_RAMP"),
    ("IfcRampFlight", Tail::Predefined, "STRAIGHT SPIRAL"),
    ("IfcRailing", Tail::Predefined, "HANDRAIL GUARDRAIL BALUSTRADE"),
    ("IfcCurtainWall", Tail::Predefined, ""),
    ("IfcCovering", Tail::Predefined, "CEILING FLOORING CLADDING ROOFING MOLDING SKIRTINGBOARD INSULATION MEMBRANE SLEEVING WRAPPING"),
    ("IfcFooting", Tail::Predefined, "CAISSON_FOUNDATION FOOTING_BEAM PAD_FOOTING PILE_CAP STRIP_FOOTING"),
    ("IfcChimney", Tail::Predefined, ""),
    ("IfcShadingDevice", Tail::Predefined, "JALOUSIE SHUTTER AWNING"),
    ("IfcBuildingElementProxy", Tail::Predefined, "COMPLEX ELEMENT PARTIAL PROVISIONFORVOID PROVISIONFORSPACE"),
    ("IfcFurniture", Tail::Predefined, "CHAIR TABLE DESK BED FILECABINET SHELF SOFA"),
    ("IfcFurnishingElement", Tail::None, ""),
    ("IfcSanitaryTerminal", Tail::Predefined, "BATH BIDET CISTERN SHOWER SINK SANITARYFOUNTAIN TOILETPAN URINAL WASHHANDBASIN WCSEAT"),
    ("IfcReinforcingBar", Tail::Bar, "ANCHORING EDGE LIGATURE MAIN PUNCHING RING SHEAR STUD"),
    ("IfcElementAssembly", Tail::Assembly, "ACCESSORY_ASSEMBLY ARCH BEAM_GRID BRACED_FRAME GIRDER REINFORCEMENT_UNIT RIGID_FRAME SLAB_FIELD TRUSS"),
];

fn class_entry(class: &str) -> Option<(&'static str, Tail, &'static str)> {
    CLASSES.iter().find(|(c, _, _)| c.eq_ignore_ascii_case(class)).copied()
}

/// Whether Iṣẹ́ Studio writes this class as itself (others go as proxies).
pub fn known_class(class: &str) -> bool {
    class_entry(class).is_some()
}

/// The predefined type to write, and the object type when it is user-defined.
fn predefined(allowed: &str, wanted: Option<&str>) -> (String, Option<String>) {
    match wanted.map(|w| w.trim().to_ascii_uppercase().replace([' ', '-'], "_")) {
        None => (".NOTDEFINED.".into(), None),
        Some(w) if w == "NOTDEFINED" || w == "USERDEFINED" => (format!(".{w}."), None),
        Some(w) if allowed.split(' ').any(|a| a == w) => (format!(".{w}."), None),
        Some(w) => (".USERDEFINED.".into(), Some(w)),
    }
}

/// The kind of a base quantity, from its name.
pub fn quantity_kind(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains("area") {
        "area"
    } else if n.contains("volume") {
        "volume"
    } else if n.contains("weight") || n.contains("mass") {
        "weight"
    } else if n.contains("count") || n.starts_with("number") {
        "count"
    } else {
        "length"
    }
}

fn value(v: &PValue) -> String {
    match v {
        PValue::Bool(b) => format!("IFCBOOLEAN(.{}.)", if *b { "T" } else { "F" }),
        PValue::Num(n) if n.fract() == 0.0 && n.abs() < 1e9 => format!("IFCINTEGER({})", *n as i64),
        PValue::Num(n) => format!("IFCREAL({})", real(*n)),
        PValue::Text(t) => format!("IFCLABEL({})", string(t)),
    }
}

pub struct Options {
    /// ISO 8601 time stamp for the header.
    pub timestamp: String,
    pub author: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { timestamp: "2026-01-01T00:00:00".into(), author: "ISE Studio".into() }
    }
}

struct Ctx {
    w: Writer,
    origin3: u64,
    z: u64,
    x: u64,
    body: u64,
    plan: u64,
    /// Property set relations to write at the end: (set id, objects).
    psets: Vec<(u64, Vec<u64>)>,
    materials: BTreeMap<String, Vec<u64>>,
    classes: BTreeMap<(String, String, Option<String>), Vec<u64>>,
}

impl Ctx {
    fn point3(&mut self, p: [f64; 3]) -> u64 {
        self.w.add("IFCCARTESIANPOINT", &format!("({},{},{})", real(p[0]), real(p[1]), real(p[2])))
    }
    fn point2(&mut self, p: [f64; 2]) -> u64 {
        self.w.add("IFCCARTESIANPOINT", &format!("({},{})", real(p[0]), real(p[1])))
    }
    fn dir3(&mut self, d: [f64; 3]) -> u64 {
        self.w.add("IFCDIRECTION", &format!("({},{},{})", real(d[0]), real(d[1]), real(d[2])))
    }
    /// A local placement `at` from `rel`, axes as the parent's.
    fn placement(&mut self, rel: Option<u64>, at: [f64; 3]) -> u64 {
        let a = if at == [0.0; 3] {
            self.w.add("IFCAXIS2PLACEMENT3D", &format!("{},$,$", r(self.origin3)))
        } else {
            let p = self.point3(at);
            self.w.add("IFCAXIS2PLACEMENT3D", &format!("{},$,$", r(p)))
        };
        self.w.add("IFCLOCALPLACEMENT", &format!("{},{}", rel.map(r).unwrap_or_else(|| "$".into()), r(a)))
    }
    fn psets(&mut self, owner: u64, key: &str, psets: &Psets) {
        for (name, props) in psets {
            if props.is_empty() {
                continue;
            }
            let ids: Vec<u64> = props.iter().map(|(k, v)| self.w.add("IFCPROPERTYSINGLEVALUE", &format!("{},$,{},$", string(k), value(v)))).collect();
            let set = self.w.add("IFCPROPERTYSET", &format!("{},$,{},$,{}", string(&guid::of(&format!("{key}/pset/{name}"))), string(name), refs(&ids)));
            self.psets.push((set, vec![owner]));
        }
    }
    fn quantities(&mut self, owner: u64, key: &str, class: &str, q: &BTreeMap<String, f64>) {
        if q.is_empty() {
            return;
        }
        let ids: Vec<u64> = q
            .iter()
            .map(|(k, v)| {
                let (ty, v) = match quantity_kind(k) {
                    "area" => ("IFCQUANTITYAREA", real(*v)),
                    "volume" => ("IFCQUANTITYVOLUME", real(*v)),
                    "weight" => ("IFCQUANTITYWEIGHT", real(*v)),
                    "count" => ("IFCQUANTITYCOUNT", real(v.round())),
                    _ => ("IFCQUANTITYLENGTH", real(*v)),
                };
                self.w.add(ty, &format!("{},$,$,{v},$", string(k)))
            })
            .collect();
        let name = format!("Qto_{}BaseQuantities", class.strip_prefix("Ifc").unwrap_or(class));
        let set = self.w.add("IFCELEMENTQUANTITY", &format!("{},$,{},$,$,{}", string(&guid::of(&format!("{key}/qto"))), string(&name), refs(&ids)));
        self.psets.push((set, vec![owner]));
    }
    fn classify(&mut self, owner: u64, c: &[Classification]) {
        for x in c {
            self.classes.entry((x.system.clone(), x.code.clone(), x.name.clone())).or_default().push(owner);
        }
    }
    /// A tessellated body in a frame `elev` mm up.
    fn body(&mut self, m: &crate::Mesh, elev: f64) -> u64 {
        let mut pts = String::from("(");
        for (i, p) in m.positions.iter().enumerate() {
            if i > 0 {
                pts.push(',');
            }
            let _ = write!(pts, "({},{},{})", real(p[0]), real(p[1]), real(p[2] - elev));
        }
        pts.push(')');
        let list = self.w.add("IFCCARTESIANPOINTLIST3D", &pts);
        let mut idx = String::from("(");
        for (i, t) in m.triangles.iter().enumerate() {
            if i > 0 {
                idx.push(',');
            }
            let _ = write!(idx, "({},{},{})", t[0] + 1, t[1] + 1, t[2] + 1);
        }
        idx.push(')');
        let set = self.w.add("IFCTRIANGULATEDFACESET", &format!("{},$,.T.,{idx},$", r(list)));
        let rep = self.w.add("IFCSHAPEREPRESENTATION", &format!("{},'Body','Tessellation',({})", r(self.body), r(set)));
        self.w.add("IFCPRODUCTDEFINITIONSHAPE", &format!("$,$,({})", r(rep)))
    }
    /// A profile's outer loop and holes extruded up `depth` from `at`, axes turned so X runs along `dir`.
    fn extruded(&mut self, outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>], at: [f64; 3], dir: [f64; 2], depth: f64) -> u64 {
        let poly = |c: &mut Ctx, l: &[[f64; 2]]| {
            let mut ids: Vec<u64> = l.iter().map(|p| c.point2(*p)).collect();
            ids.push(ids[0]);
            c.w.add("IFCPOLYLINE", &refs(&ids))
        };
        let o = poly(self, outer);
        let profile = if holes.is_empty() {
            self.w.add("IFCARBITRARYCLOSEDPROFILEDEF", &format!(".AREA.,$,{}", r(o)))
        } else {
            let hs: Vec<u64> = holes.iter().map(|h| poly(self, h)).collect();
            self.w.add("IFCARBITRARYPROFILEDEFWITHVOIDS", &format!(".AREA.,$,{},{}", r(o), refs(&hs)))
        };
        let p = self.point3(at);
        let x = self.dir3([dir[0], dir[1], 0.0]);
        let pos = self.w.add("IFCAXIS2PLACEMENT3D", &format!("{},{},{}", r(p), r(self.z), r(x)));
        let solid = self.w.add("IFCEXTRUDEDAREASOLID", &format!("{},{},{},{}", r(profile), r(pos), r(self.z), real(depth)));
        let rep = self.w.add("IFCSHAPEREPRESENTATION", &format!("{},'Body','SweptSolid',({})", r(self.body), r(solid)));
        self.w.add("IFCPRODUCTDEFINITIONSHAPE", &format!("$,$,({})", r(rep)))
    }
}

/// The whole model as an IFC4 file.
pub fn write(model: &Model, options: &Options) -> String {
    let mut c = Ctx { w: Writer::default(), origin3: 0, z: 0, x: 0, body: 0, plan: 0, psets: vec![], materials: BTreeMap::new(), classes: BTreeMap::new() };
    let key = |k: &str| format!("{}/{k}", model.project);
    // Shared geometry and contexts.
    c.origin3 = c.w.add("IFCCARTESIANPOINT", "(0.,0.,0.)");
    c.z = c.dir3([0.0, 0.0, 1.0]);
    c.x = c.dir3([1.0, 0.0, 0.0]);
    let world = c.w.add("IFCAXIS2PLACEMENT3D", &format!("{},{},{}", r(c.origin3), r(c.z), r(c.x)));
    let north = model.site.north.map(|a| {
        let a = a.to_radians();
        c.w.add("IFCDIRECTION", &format!("({},{})", real(-a.sin()), real(a.cos())))
    });
    let ctx = c.w.add("IFCGEOMETRICREPRESENTATIONCONTEXT", &format!("$,'Model',3,1.E-05,{},{}", r(world), north.map(r).unwrap_or_else(|| "$".into())));
    c.body = c.w.add("IFCGEOMETRICREPRESENTATIONSUBCONTEXT", &format!("'Body','Model',*,*,*,*,{},$,.MODEL_VIEW.,$", r(ctx)));
    c.plan = c.w.add("IFCGEOMETRICREPRESENTATIONSUBCONTEXT", &format!("'FootPrint','Model',*,*,*,*,{},$,.PLAN_VIEW.,$", r(ctx)));
    let units = [
        c.w.add("IFCSIUNIT", "*,.LENGTHUNIT.,.MILLI.,.METRE."),
        c.w.add("IFCSIUNIT", "*,.AREAUNIT.,$,.SQUARE_METRE."),
        c.w.add("IFCSIUNIT", "*,.VOLUMEUNIT.,$,.CUBIC_METRE."),
        c.w.add("IFCSIUNIT", "*,.MASSUNIT.,.KILO.,.GRAM."),
        c.w.add("IFCSIUNIT", "*,.PLANEANGLEUNIT.,$,.RADIAN."),
    ];
    let units = c.w.add("IFCUNITASSIGNMENT", &refs(&units));
    let project = c.w.add("IFCPROJECT", &format!("{},$,{},$,$,$,$,({}),{}", string(&guid::of(&key("project"))), string(&model.project), r(ctx), r(units)));

    // Site and building.
    let site_place = c.placement(None, [0.0; 3]);
    let angle = |deg: f64| {
        let neg = deg < 0.0;
        let a = deg.abs();
        let d = a.trunc();
        let m = ((a - d) * 60.0).trunc();
        let s = ((a - d) * 3600.0 - m * 60.0).trunc();
        let u = (((a - d) * 3600.0 - m * 60.0 - s) * 1e6).round();
        let sign = if neg { -1.0 } else { 1.0 };
        format!("({},{},{},{})", (sign * d) as i64, (sign * m) as i64, (sign * s) as i64, (sign * u) as i64)
    };
    let site_name = if model.site.name.is_empty() { "Site".to_string() } else { model.site.name.clone() };
    let site_addr = model.site.address.as_ref().map(|a| c.w.add("IFCPOSTALADDRESS", &format!("$,$,$,$,({}),$,$,$,$,$", string(a))));
    let site = c.w.add(
        "IFCSITE",
        &format!(
            "{},$,{},$,$,{},$,$,.ELEMENT.,{},{},{},$,{}",
            string(&guid::of(&key("site"))),
            string(&site_name),
            r(site_place),
            model.site.latitude.map(angle).unwrap_or_else(|| "$".into()),
            model.site.longitude.map(angle).unwrap_or_else(|| "$".into()),
            model.site.elevation.map(|e| real(e * 1000.0)).unwrap_or_else(|| "$".into()),
            site_addr.map(r).unwrap_or_else(|| "$".into()),
        ),
    );
    c.psets(site, &key("site"), &model.site.psets);
    let building_place = c.placement(Some(site_place), [0.0; 3]);
    let building_name = if model.building.name.is_empty() { model.project.clone() } else { model.building.name.clone() };
    let addr = model.building.address.as_ref().map(|a| c.w.add("IFCPOSTALADDRESS", &format!("$,$,$,$,({}),$,$,$,$,$", string(a))));
    let building = c.w.add(
        "IFCBUILDING",
        &format!(
            "{},$,{},{},$,{},$,$,.ELEMENT.,$,$,{}",
            string(&guid::of(&key("building"))),
            string(&building_name),
            opt_string(model.building.description.as_deref()),
            r(building_place),
            addr.map(r).unwrap_or_else(|| "$".into())
        ),
    );
    c.psets(building, &key("building"), &model.building.psets);
    c.w.add("IFCRELAGGREGATES", &format!("{},$,$,$,{},({})", string(&guid::of(&key("project/site"))), r(project), r(site)));
    c.w.add("IFCRELAGGREGATES", &format!("{},$,$,$,{},({})", string(&guid::of(&key("site/building"))), r(site), r(building)));

    // Storeys.
    let mut storeys: Vec<(String, u64, u64, f64)> = vec![]; // name, storey, placement, elevation
    for l in &model.levels {
        let place = c.placement(Some(building_place), [0.0, 0.0, l.elevation]);
        let id = c.w.add("IFCBUILDINGSTOREY", &format!("{},$,{},$,$,{},$,$,.ELEMENT.,{}", string(&guid::of(&key(&format!("level/{}", l.name)))), string(&l.name), r(place), real(l.elevation)));
        if let Some(h) = l.height {
            let mut q = BTreeMap::new();
            q.insert("GrossHeight".to_string(), h);
            c.quantities(id, &key(&format!("level/{}", l.name)), "IfcBuildingStorey", &q);
        }
        storeys.push((l.name.clone(), id, place, l.elevation));
    }
    if !storeys.is_empty() {
        let ids: Vec<u64> = storeys.iter().map(|s| s.1).collect();
        c.w.add("IFCRELAGGREGATES", &format!("{},$,$,$,{},{}", string(&guid::of(&key("building/storeys"))), r(building), refs(&ids)));
    }
    let frame = |level: Option<&String>| -> (u64, f64) {
        level.and_then(|l| storeys.iter().find(|s| &s.0 == l)).map(|s| (s.2, s.3)).unwrap_or((building_place, 0.0))
    };
    // What each storey (or the building) holds.
    let mut contained: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let structure_of = |level: Option<&String>| level.and_then(|l| storeys.iter().find(|s| &s.0 == l)).map(|s| s.1).unwrap_or(building);

    // Groups first, so parts can be aggregated into them.
    let mut groups: BTreeMap<String, (u64, u64, Vec<u64>)> = BTreeMap::new(); // name → (id, placement, children)
    for g in &model.groups {
        let (rel, _) = frame(g.level.as_ref());
        let place = c.placement(Some(rel), [0.0; 3]);
        let gkey = key(&format!("group/{}", g.name));
        let (entry, tail, allowed) = class_entry(&g.class).unwrap_or(("IfcElementAssembly", Tail::Assembly, ""));
        let (pre, user) = predefined(allowed, g.predefined.as_deref());
        let object_type = g.type_name.clone().or(user).or_else(|| (!known_class(&g.class)).then(|| g.class.clone()));
        let head = format!("{},$,{},$,{},{},$,{}", string(&guid::of(&gkey)), string(&g.name), opt_string(object_type.as_deref()), r(place), opt_string(g.tag.as_deref()));
        let args = match tail {
            Tail::Assembly => format!("{head},.NOTDEFINED.,{pre}"),
            Tail::None => head,
            Tail::Door | Tail::Window => format!("{head},$,$,{pre},.NOTDEFINED.,$"),
            Tail::StairFlight => format!("{head},$,$,$,$,{pre}"),
            Tail::Bar => format!("{head},$,$,$,$,{pre},$"),
            Tail::Predefined => format!("{head},{pre}"),
        };
        let id = c.w.add(&entry.to_ascii_uppercase(), &args);
        c.psets(id, &gkey, &g.psets);
        c.quantities(id, &gkey, entry, &g.quantities);
        contained.entry(structure_of(g.level.as_ref())).or_default().push(id);
        groups.insert(g.name.clone(), (id, place, vec![]));
    }

    // Elements.
    let mut by_name: BTreeMap<&str, (u64, u64, f64)> = BTreeMap::new(); // name → (id, placement, elevation)
    for e in &model.elements {
        let info = &e.info;
        let (rel, elev) = frame(info.level.as_ref());
        let parent = info.parent.as_ref().and_then(|p| groups.get(p).map(|g| g.1));
        let place = c.placement(Some(parent.unwrap_or(rel)), [0.0; 3]);
        let shape = c.body(&e.mesh, elev);
        let class = if info.class.is_empty() { "IfcBuildingElementProxy" } else { info.class.as_str() };
        let (entry, tail, allowed) = class_entry(class).unwrap_or(("IfcBuildingElementProxy", Tail::Predefined, "COMPLEX ELEMENT PARTIAL PROVISIONFORVOID PROVISIONFORSPACE"));
        let (pre, user) = predefined(allowed, info.predefined.as_deref());
        let object_type = info.type_name.clone().or(user).or_else(|| (!known_class(class) && !info.class.is_empty()).then(|| class.to_string()));
        let ekey = key(&format!("element/{}", e.key));
        let head = format!(
            "{},$,{},{},{},{},{},{}",
            string(&guid::of(&ekey)),
            string(&e.name),
            opt_string(info.description.as_deref()),
            opt_string(object_type.as_deref()),
            r(place),
            r(shape),
            opt_string(info.tag.as_deref())
        );
        let attr = |k: &str| info.attributes.get(k).and_then(|v| if let PValue::Num(n) = v { Some(*n) } else { None });
        let attr_text = |k: &str| info.attributes.get(k).and_then(|v| if let PValue::Text(t) = v { Some(t.clone()) } else { None });
        let num = |v: Option<f64>| v.map(real).unwrap_or_else(|| "$".into());
        let args = match tail {
            Tail::Predefined => format!("{head},{pre}"),
            Tail::None => head,
            Tail::Door => {
                let op = attr_text("OperationType").map(|o| o.to_ascii_uppercase()).filter(|o| {
                    "SINGLE_SWING_LEFT SINGLE_SWING_RIGHT DOUBLE_DOOR_SINGLE_SWING DOUBLE_DOOR_SINGLE_SWING_OPPOSITE_LEFT DOUBLE_DOOR_SINGLE_SWING_OPPOSITE_RIGHT DOUBLE_SWING_LEFT DOUBLE_SWING_RIGHT DOUBLE_DOOR_DOUBLE_SWING SLIDING_TO_LEFT SLIDING_TO_RIGHT DOUBLE_DOOR_SLIDING FOLDING_TO_LEFT FOLDING_TO_RIGHT DOUBLE_DOOR_FOLDING REVOLVING ROLLINGUP SWING_FIXED_LEFT SWING_FIXED_RIGHT"
                        .split(' ')
                        .any(|a| a == o)
                });
                format!("{head},{},{},{pre},.{}.,$", num(attr("OverallHeight")), num(attr("OverallWidth")), op.unwrap_or_else(|| "NOTDEFINED".into()))
            }
            Tail::Window => {
                let pt = attr_text("PartitioningType").map(|o| o.to_ascii_uppercase()).filter(|o| {
                    "SINGLE_PANEL DOUBLE_PANEL_VERTICAL DOUBLE_PANEL_HORIZONTAL TRIPLE_PANEL_VERTICAL TRIPLE_PANEL_BOTTOM TRIPLE_PANEL_TOP TRIPLE_PANEL_LEFT TRIPLE_PANEL_RIGHT TRIPLE_PANEL_HORIZONTAL".split(' ').any(|a| a == o)
                });
                format!("{head},{},{},{pre},.{}.,$", num(attr("OverallHeight")), num(attr("OverallWidth")), pt.unwrap_or_else(|| "NOTDEFINED".into()))
            }
            Tail::StairFlight => {
                let int = |v: Option<f64>| v.map(|n| format!("{}", n.round() as i64)).unwrap_or_else(|| "$".into());
                format!("{head},{},{},{},{},{pre}", int(attr("NumberOfRisers")), int(attr("NumberOfTreads")), num(attr("RiserHeight")), num(attr("TreadLength")))
            }
            Tail::Bar => {
                let d = attr("NominalDiameter");
                format!(
                    "{head},{},{},{},{},{pre},$",
                    opt_string(attr_text("SteelGrade").as_deref()),
                    num(d),
                    num(d.map(|d| std::f64::consts::PI * d * d / 4.0)),
                    num(attr("BarLength")),
                )
            }
            Tail::Assembly => format!("{head},.NOTDEFINED.,{pre}"),
        };
        let id = c.w.add(&entry.to_ascii_uppercase(), &args);
        c.psets(id, &ekey, &info.psets);
        c.quantities(id, &ekey, entry, &info.quantities);
        c.classify(id, &info.classification);
        if let Some(m) = &e.material {
            c.materials.entry(m.clone()).or_default().push(id);
        }
        match info.parent.as_ref().and_then(|p| groups.get_mut(p)) {
            Some(g) => g.2.push(id),
            None => contained.entry(structure_of(info.level.as_ref())).or_default().push(id),
        }
        by_name.insert(&e.name, (id, place, elev));
    }
    for (name, (id, _, kids)) in &groups {
        if !kids.is_empty() {
            c.w.add("IFCRELAGGREGATES", &format!("{},$,$,$,{},{}", string(&guid::of(&key(&format!("group/{name}/parts")))), r(*id), refs(kids)));
        }
    }

    // Openings: a box through the host, voiding it, filled by its door or window.
    for o in &model.openings {
        let Some(&(host, host_place, elev)) = by_name.get(o.host.as_str()) else { continue };
        let place = c.placement(Some(host_place), [0.0; 3]);
        let [l, t, h] = o.size;
        let len = (o.dir[0] * o.dir[0] + o.dir[1] * o.dir[1]).sqrt().max(1e-12);
        let dir = [o.dir[0] / len, o.dir[1] / len];
        let rect = [[0.0, 0.0], [l, 0.0], [l, t], [0.0, t]];
        let shape = c.extruded(&rect, &[], [o.origin[0], o.origin[1], o.origin[2] - elev], dir, h);
        let okey = key(&format!("opening/{}/{}", o.host, o.name));
        let id = c.w.add("IFCOPENINGELEMENT", &format!("{},$,{},$,$,{},{},$,.OPENING.", string(&guid::of(&okey)), string(&o.name), r(place), r(shape)));
        c.w.add("IFCRELVOIDSELEMENT", &format!("{},$,$,$,{},{}", string(&guid::of(&format!("{okey}/voids"))), r(host), r(id)));
        if let Some(&(fill, _, _)) = o.fills.as_deref().and_then(|f| by_name.get(f)) {
            c.w.add("IFCRELFILLSELEMENT", &format!("{},$,$,$,{},{}", string(&guid::of(&format!("{okey}/fills"))), r(id), r(fill)));
        }
    }

    // Spaces, aggregated to their storey.
    let mut space_ids: BTreeMap<&str, u64> = BTreeMap::new();
    let mut spaces_in: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for s in &model.spaces {
        if s.outline.is_empty() || s.outline[0].len() < 3 {
            continue;
        }
        let (rel, elev) = frame(s.level.as_ref());
        let place = c.placement(Some(rel), [0.0; 3]);
        let shape = c.extruded(&s.outline[0], &s.outline[1..], [0.0, 0.0, s.elevation - elev], [1.0, 0.0], s.height);
        let skey = key(&format!("space/{}", s.name));
        let (pre, _) = predefined("SPACE PARKING GFA INTERNAL EXTERNAL", s.predefined.as_deref());
        let id = c.w.add(
            "IFCSPACE",
            &format!("{},$,{},$,$,{},{},{},.ELEMENT.,{pre},$", string(&guid::of(&skey)), string(&s.name), r(place), r(shape), opt_string(s.long_name.as_deref())),
        );
        let mut q = BTreeMap::new();
        q.insert("NetFloorArea".to_string(), s.area());
        q.insert("GrossFloorArea".to_string(), s.area());
        q.insert("GrossPerimeter".to_string(), s.perimeter());
        q.insert("Height".to_string(), s.height);
        q.insert("NetVolume".to_string(), s.area() * s.height / 1000.0);
        c.quantities(id, &skey, "IfcSpace", &q);
        c.psets(id, &skey, &s.psets);
        c.classify(id, &s.classification);
        spaces_in.entry(structure_of(s.level.as_ref())).or_default().push(id);
        space_ids.insert(&s.name, id);
    }
    for (storey, ids) in &spaces_in {
        c.w.add("IFCRELAGGREGATES", &format!("{},$,$,$,{},{}", string(&guid::of(&key(&format!("spaces/{storey}")))), r(*storey), refs(ids)));
    }

    // Zones.
    for z in &model.zones {
        let members: Vec<u64> = z.spaces.iter().filter_map(|s| space_ids.get(s.as_str()).copied()).collect();
        let zkey = key(&format!("zone/{}", z.name));
        let id = c.w.add("IFCZONE", &format!("{},$,{},$,$,$", string(&guid::of(&zkey)), string(&z.name)));
        c.psets(id, &zkey, &z.psets);
        if !members.is_empty() {
            c.w.add("IFCRELASSIGNSTOGROUP", &format!("{},$,$,$,{},$,{}", string(&guid::of(&format!("{zkey}/spaces"))), refs(&members), r(id)));
        }
    }

    // Grids.
    for g in &model.grids {
        let (rel, _) = frame(g.level.as_ref());
        let place = c.placement(Some(rel), [0.0; 3]);
        let (mut u, mut v) = (g.u.clone(), g.v.clone());
        if v.is_empty() && u.len() > 1 {
            v.push(u.pop().unwrap());
        }
        if u.is_empty() && v.len() > 1 {
            u.push(v.remove(0));
        }
        let mut curves = vec![];
        let mut axis = |c: &mut Ctx, a: &crate::GridAxis| {
            let p = [c.point2(a.from), c.point2(a.to)];
            let line = c.w.add("IFCPOLYLINE", &refs(&p));
            curves.push(line);
            c.w.add("IFCGRIDAXIS", &format!("{},{},.T.", string(&a.tag), r(line)))
        };
        let us: Vec<u64> = u.iter().map(|a| axis(&mut c, a)).collect();
        let vs: Vec<u64> = v.iter().map(|a| axis(&mut c, a)).collect();
        let set = c.w.add("IFCGEOMETRICCURVESET", &refs(&curves));
        let rep = c.w.add("IFCSHAPEREPRESENTATION", &format!("{},'FootPrint','GeometricCurveSet',({})", r(c.plan), r(set)));
        let shape = c.w.add("IFCPRODUCTDEFINITIONSHAPE", &format!("$,$,({})", r(rep)));
        let id = c.w.add(
            "IFCGRID",
            &format!("{},$,{},$,$,{},{},{},{},$,.RECTANGULAR.", string(&guid::of(&key(&format!("grid/{}", g.name)))), string(&g.name), r(place), r(shape), refs(&us), refs(&vs)),
        );
        contained.entry(structure_of(g.level.as_ref())).or_default().push(id);
    }

    for (structure, ids) in &contained {
        c.w.add("IFCRELCONTAINEDINSPATIALSTRUCTURE", &format!("{},$,$,$,{},{}", string(&guid::of(&key(&format!("contains/{structure}")))), refs(ids), r(*structure)));
    }
    let psets = std::mem::take(&mut c.psets);
    for (set, owners) in psets {
        c.w.add("IFCRELDEFINESBYPROPERTIES", &format!("{},$,$,$,{},{}", string(&guid::of(&key(&format!("defines/{set}")))), refs(&owners), r(set)));
    }
    let materials = std::mem::take(&mut c.materials);
    for (name, owners) in materials {
        let m = c.w.add("IFCMATERIAL", &format!("{},$,$", string(&name)));
        c.w.add("IFCRELASSOCIATESMATERIAL", &format!("{},$,$,$,{},{}", string(&guid::of(&key(&format!("material/{name}")))), refs(&owners), r(m)));
    }
    let classes = std::mem::take(&mut c.classes);
    let mut systems: BTreeMap<String, u64> = BTreeMap::new();
    for ((system, code, name), owners) in classes {
        let sys = *systems.entry(system.clone()).or_insert_with(|| c.w.add("IFCCLASSIFICATION", &format!("$,$,$,{},$,$,$", string(&system))));
        let refr = c.w.add("IFCCLASSIFICATIONREFERENCE", &format!("$,{},{},{},$,$", string(&code), opt_string(name.as_deref()), r(sys)));
        c.w.add("IFCRELASSOCIATESCLASSIFICATION", &format!("{},$,$,$,{},{}", string(&guid::of(&key(&format!("class/{system}/{code}")))), refs(&owners), r(refr)));
    }

    let mut out = String::new();
    let _ = writeln!(out, "ISO-10303-21;\nHEADER;");
    let _ = writeln!(out, "FILE_DESCRIPTION(('ViewDefinition [ReferenceView_V1.2]'),'2;1');");
    let _ = writeln!(out, "FILE_NAME({},{},({}),('ISE Studio'),'ISE Studio IFC writer','ISE Studio','');", string(&format!("{}.ifc", model.project)), string(&options.timestamp), string(&options.author));
    let _ = writeln!(out, "FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;");
    out.push_str(&c.w.data);
    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    out
}
