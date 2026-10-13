//! Building information on top of shape programs.
//!
//! A program's parts are its building elements: `part(name, solid, { ifc })`
//! (or the prelude's `wall`, `slab`, `door`, … helpers) gives a part its IFC
//! class, level, property sets, quantities and classification. Besides parts,
//! a program declares what has no material of its own: the site, the
//! building, its levels (storeys with elevations), spaces (rooms with an
//! outline and a height), openings cut in a host, zones, grids and groups.
//! All of it is [`Declared`], carried with the evaluated shape.
//!
//! The engine gathers every placed shape's declarations into one [`Model`]
//! (instances transformed into project coordinates) and from it:
//! - writes IFC4 (`ifc_write`): spatial structure project › site › building ›
//!   storeys, elements with tessellated body geometry, openings voiding their
//!   host and filled by their door or window, spaces, zones, grids, materials,
//!   property sets, base quantities and classification references;
//! - reads IFC (`ifc_read`, IFC2X3 and IFC4 STEP files): the same back, with
//!   geometry from triangulated and polygonal face sets, faceted B-reps,
//!   extruded profiles, mapped items and boolean results;
//! - lists schedules and quantity take-off (`schedule`).
//!
//! Lengths are millimetres, areas square metres, volumes cubic metres.

pub mod geom;
pub mod guid;
pub mod ifc_read;
pub mod ifc_write;
pub mod schedule;
pub mod step;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A property value: a flag, a number or text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PValue {
    Bool(bool),
    Num(f64),
    Text(String),
}

impl PValue {
    pub fn text(&self) -> String {
        match self {
            PValue::Bool(b) => if *b { "yes" } else { "no" }.into(),
            PValue::Num(n) => num_text(*n),
            PValue::Text(t) => t.clone(),
        }
    }
}

/// A number without trailing zeros (at most 4 decimals).
pub fn num_text(n: f64) -> String {
    let s = format!("{n:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

pub type Psets = BTreeMap<String, BTreeMap<String, PValue>>;

/// A classification code: Uniclass, OmniClass, MasterFormat, NRM…
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Classification {
    /// "Uniclass 2015", "OmniClass", "MasterFormat".
    pub system: String,
    /// "EF_25_10", "23-17 11 00".
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A line or arc drawn on plans for an element (a door's swing, a stair's
/// treads and arrow), in the program's coordinates (mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlanMark {
    pub points: Vec<[f64; 2]>,
    /// Drawn dashed (above the cut, or hidden).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dashed: bool,
}

/// What a part is as a building element.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Element {
    /// IFC class: "IfcWall", "IfcSlab", "IfcDoor"… (an unknown one is
    /// written as IfcBuildingElementProxy with this as its object type).
    pub class: String,
    /// The class's predefined type: "STANDARD", "FLOOR", "ROOF", "DOOR"…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predefined: Option<String>,
    /// Its type or preset: "Door 900 × 2100 single swing", "Wall 200 block".
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// The level (storey) it belongs to, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Mark or tag: "D01", "W03", a bar mark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// For a door or window: the part (a wall) whose opening it fills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The group (curtain wall, stair) it is part of, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psets: Psets,
    /// Base quantities by name: lengths in mm, areas in m², volumes in m³,
    /// weights in kg, counts plain ("Length", "NetSideArea", "NetVolume").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quantities: BTreeMap<String, f64>,
    /// Class attributes: OverallWidth/OverallHeight (doors, windows),
    /// NominalDiameter/BarLength (bars), NumberOfRisers… (mm).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, PValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classification: Vec<Classification>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<PlanMark>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Site {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    /// Height of the project's z = 0 above sea level (m).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Angle (degrees, anticlockwise from +Y) of true north.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub north: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psets: Psets,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Building {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psets: Psets,
}

/// A storey: its floor's elevation (mm, project z) and height to the next.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Level {
    pub name: String,
    pub elevation: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
}

/// A room or other bounded space: an outline (outer loop, then holes) on its
/// level, `elevation` (its floor, mm) and `height`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Space {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    pub outline: Vec<Vec<[f64; 2]>>,
    pub elevation: f64,
    pub height: f64,
    /// "INTERNAL", "EXTERNAL", "GFA"…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predefined: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psets: Psets,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classification: Vec<Classification>,
}

impl Space {
    /// Floor area (m²): the outer loop less the holes.
    pub fn area(&self) -> f64 {
        let mut a = 0.0;
        for (i, l) in self.outline.iter().enumerate() {
            let s = geom::area2(l).abs();
            a += if i == 0 { s } else { -s };
        }
        a / 1e6
    }
    pub fn perimeter(&self) -> f64 {
        self.outline.iter().map(|l| geom::perimeter(l)).sum()
    }
}

/// A void cut through a host element: a box `size` = [along, through, up]
/// from `origin` (its lower corner), `along` running in direction `dir`
/// (horizontal). `fills` is the door or window in it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Opening {
    pub name: String,
    pub host: String,
    pub origin: [f64; 3],
    pub dir: [f64; 2],
    pub size: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fills: Option<String>,
}

/// Spaces grouped for a purpose: a flat, a fire compartment, a tenancy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Zone {
    pub name: String,
    pub spaces: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psets: Psets,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GridAxis {
    /// The bubble's label: "A", "1".
    pub tag: String,
    pub from: [f64; 2],
    pub to: [f64; 2],
}

/// A structural grid: lines with bubbles, shown on plans and sections.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Grid {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// One family of lines (U), usually along X…
    pub u: Vec<GridAxis>,
    /// …and the other (V).
    pub v: Vec<GridAxis>,
}

/// An element made of parts (a curtain wall of mullions and panels, a stair
/// of flights and landings): it has no geometry of its own; parts name it as
/// their `parent`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Group {
    pub name: String,
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predefined: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psets: Psets,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quantities: BTreeMap<String, f64>,
}

/// Everything a program declares about building information.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Declared {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<Site>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub building: Option<Building>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<Level>,
    /// Parts as building elements, by part name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub elements: BTreeMap<String, Element>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<Space>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openings: Vec<Opening>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<Zone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grids: Vec<Grid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}

impl Declared {
    pub fn is_empty(&self) -> bool {
        self.site.is_none()
            && self.building.is_none()
            && self.levels.is_empty()
            && self.elements.is_empty()
            && self.spaces.is_empty()
            && self.openings.is_empty()
            && self.zones.is_empty()
            && self.grids.is_empty()
            && self.groups.is_empty()
    }
}

/// A triangle mesh (mm): shared vertices, outward-facing triangles.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// One building element of the whole model, in project coordinates.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelElement {
    /// Unique in the model ("Ground floor › Wall A" for an instance's part).
    pub name: String,
    /// Stable seed for its GlobalId (shape id and part name).
    pub key: String,
    /// What it is; an untagged part is an IfcBuildingElementProxy.
    pub info: Element,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Where the part is made: kept for the BOM ("cast", "glazing", "buy").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    pub mesh: Mesh,
    /// Its true volume (mm³).
    pub volume: f64,
}

/// Every building element, space and declaration of a project (or of one
/// assembly), in project coordinates.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Model {
    pub project: String,
    pub site: Site,
    pub building: Building,
    /// Bottom to top.
    pub levels: Vec<Level>,
    pub elements: Vec<ModelElement>,
    pub spaces: Vec<Space>,
    pub openings: Vec<Opening>,
    pub zones: Vec<Zone>,
    pub grids: Vec<Grid>,
    pub groups: Vec<Group>,
    /// What could not be carried over exactly.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Model {
    /// The level an element belongs to: its own, else the highest level at or
    /// below its bottom (a little tolerance), else none.
    pub fn level_of(&self, e: &ModelElement) -> Option<String> {
        if let Some(l) = &e.info.level {
            if self.levels.iter().any(|x| &x.name == l) {
                return Some(l.clone());
            }
        }
        let bottom = e.mesh.positions.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min);
        level_at(&self.levels, bottom)
    }

    /// Fill in every element's level and sort the levels bottom to top.
    pub fn settle(&mut self) {
        self.levels.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
        let levels: Vec<Option<String>> = self.elements.iter().map(|e| self.level_of(e)).collect();
        for (e, l) in self.elements.iter_mut().zip(levels) {
            e.info.level = l;
        }
        for s in &mut self.spaces {
            if s.level.as_ref().is_none_or(|l| !self.levels.iter().any(|x| &x.name == l)) {
                s.level = level_at(&self.levels, s.elevation);
            }
        }
        for g in &mut self.groups {
            if g.level.as_ref().is_some_and(|l| !self.levels.iter().any(|x| &x.name == l)) {
                g.level = None;
            }
            if g.level.is_none() {
                g.level = self.elements.iter().find(|e| e.info.parent.as_ref() == Some(&g.name)).and_then(|e| e.info.level.clone());
            }
        }
    }
}

/// The highest level at or below `z` (10 mm tolerance), else the lowest.
pub fn level_at(levels: &[Level], z: f64) -> Option<String> {
    if !z.is_finite() {
        return levels.first().map(|l| l.name.clone());
    }
    levels.iter().filter(|l| l.elevation <= z + 10.0).max_by(|a, b| a.elevation.total_cmp(&b.elevation)).or(levels.first()).map(|l| l.name.clone())
}

/// "wall" → "IfcWall", "IfcWall" → "IfcWall", "curtain wall" → "IfcCurtainWall".
pub fn class_name(s: &str) -> String {
    let t = s.trim();
    if t.len() > 3 && t[..3].eq_ignore_ascii_case("ifc") {
        return format!("Ifc{}", &t[3..]);
    }
    let lower = t.to_ascii_lowercase();
    match lower.as_str() {
        "rebar" | "bar" | "bars" | "reinforcement" => return "IfcReinforcingBar".into(),
        "room" | "rooms" => return "IfcSpace".into(),
        "proxy" | "proxies" => return "IfcBuildingElementProxy".into(),
        _ => {}
    }
    // Plurals: "walls" → wall.
    let t = if lower.ends_with('s') && !lower.ends_with("ss") && t.len() > 3 { &t[..t.len() - 1] } else { t };
    let mut out = String::from("Ifc");
    for w in t.split(|c: char| c.is_whitespace() || c == '_' || c == '-') {
        let mut c = w.chars();
        if let Some(f) = c.next() {
            out.extend(f.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    out
}

/// "IfcWall" → "Wall"; "IfcBuildingElementProxy" → "Building element proxy".
pub fn class_words(class: &str) -> String {
    let base = class.strip_prefix("Ifc").unwrap_or(class);
    let mut out = String::new();
    for (i, c) in base.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
