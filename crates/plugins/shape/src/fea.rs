//! Linear static stress analysis: where a part bends, how far, and how hard
//! its material is worked.
//!
//! The part is filled with cubes (voxels) cut from its exact outline, each a
//! hexahedral element with Wilson's incompatible modes so bending is not
//! stiffened by shear locking. Every element is the same cube, so one
//! element stiffness serves them all, and the equations are solved by
//! conjugate gradients without ever forming the global matrix. Supports fix
//! the nodes on chosen faces; loads spread a force over the nodes of others.
//! Results: displacement at every node, von Mises stress in every element,
//! and the safety factor against the material's yield strength.
//!
//! Units: millimetres, newtons, megapascals (N/mm²).

mod contact;
mod eigen;
pub mod frame;
mod inp;
pub mod materials;
mod multigrid;
mod post;
mod refine;

pub use eigen::{Buckling, BucklingMode, buckling, modes_study};
pub use post::{Fatigue, FatigueSpec, Probe, Section, csv, cube_near, fatigue, marin, path, principal, probe, section};
pub use inp::{InpKind, calculix_inp};
pub use refine::Peak;

use rayon::prelude::*;
use std::collections::HashMap;
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

/// Elastic modulus (MPa), Poisson's ratio and yield strength (MPa).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Elastic {
    pub e: f64,
    pub nu: f64,
    pub yield_strength: f64,
    /// Thermal expansion coefficient (1/K): strain per degree of warming.
    pub alpha: f64,
    /// kg/m³: what gravity and spinning pull on.
    pub density: f64,
    /// Slope past yield (MPa) when the study lets the material yield
    /// (bilinear plasticity; 0: perfectly plastic).
    pub tangent: f64,
}

/// Typical properties by material name: design guidance, not a datasheet
/// (see `materials`).
pub fn elastic(material: &str) -> Option<Elastic> {
    materials::lookup(material).map(|m| m.elastic())
}

/// Linear thermal expansion coefficient (1/K), typical by material name:
/// how much longer per unit length each degree of warming makes it.
pub fn expansion(material: &str) -> Option<f64> {
    materials::lookup(material).map(|m| m.expansion)
}

/// A face as picked: a point on it, its outward normal there, and its
/// triangles (x y z per vertex, 9 numbers per triangle), in part coordinates.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Face {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub triangles: Vec<f64>,
    /// The part of a several-part study it is on (none: the first part).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// A curved surface (a cylinder's side, a bore, a fillet): each triangle
    /// faces its own way, and loads spread over it by area.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub curved: bool,
}

impl Face {
    fn tris(&self) -> impl Iterator<Item = [[f64; 3]; 3]> + '_ {
        self.triangles.chunks_exact(9).map(|t| [[t[0], t[1], t[2]], [t[3], t[4], t[5]], [t[6], t[7], t[8]]])
    }

    /// Its area (mm²).
    pub fn area(&self) -> f64 {
        self.tris().map(|t| v_norm(v_cross(v_sub(t[1], t[0]), v_sub(t[2], t[0]))) / 2.0).sum()
    }
}

fn v_sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn v_dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn v_cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn v_norm(a: [f64; 3]) -> f64 {
    v_dot(a, a).sqrt()
}

/// A face as kept with a study: where on the part it sits (fractions of the
/// part's bounds: its middle, and a point on one of its triangles) and which
/// way it faces, so it is found again after the part changes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct FacePick {
    #[serde(default)]
    pub at: [f64; 3],
    #[serde(default)]
    pub probe: [f64; 3],
    pub normal: [f64; 3],
    /// The part it is on, when not the study's own part (another part
    /// studied with it); fractions are of that part's bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// The whole smooth curved surface under the probe (a cylinder's side,
    /// a bore, a fillet, a sphere), not a flat face; `normal` is the
    /// surface's where it was picked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub curved: bool,
    /// Instead of `at` and `probe`: a point on or near the face (part
    /// coordinates, mm), as the AI gives it; saving the study pins it to
    /// where it sits (see `studies::pin_face`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<[f64; 3]>,
}

/// How two parts studied together act where they touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContactKind {
    /// They push on each other but never pull, and slide freely.
    #[default]
    Frictionless,
    /// They push, never pull, and grip up to `friction` × the push before sliding.
    Friction,
    /// Glued: they move together where they touch.
    Bonded,
}

/// The contact between two parts studied together (pairs not listed are frictionless).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ContactSpec {
    pub parts: [String; 2],
    #[serde(default)]
    pub kind: ContactKind,
    /// Coefficient of friction (with kind "friction").
    #[serde(default)]
    pub friction: f64,
    /// A press fit: where the parts overlap as drawn, the overlap is an
    /// interference the contact squeezes out, so they start pressed together.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub press_fit: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct LoadPick {
    pub face: FacePick,
    /// Newtons; MPa with direction "pressure".
    pub magnitude: f64,
    /// "into" (the face), "pressure", or an axis: "-z", "+z", "-x", "+x", "-y", "+y".
    ///
    /// "into" on a flat face is a force square to it; on a curved face it is
    /// the newtons spread evenly over its area, each bit pushing straight in
    /// along its own normal (a pressure of magnitude ÷ area), so pushes on
    /// opposite sides of a bore cancel and the net force is less. "pressure"
    /// is that pressure given in MPa (N/mm²), on any face. An axis spreads
    /// the force over the face by area, all one way.
    pub direction: String,
}

/// A study as kept with the shape.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct StudySpec {
    /// The part of an assembly it is on (none: the whole solid).
    #[serde(default)]
    pub part: Option<String>,
    /// Other parts of the assembly studied with it, pressing on it where they
    /// touch (["all"]: every part).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// How each pair of those parts acts where they touch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<ContactSpec>,
    #[serde(default)]
    pub supports: Vec<FacePick>,
    #[serde(default)]
    pub loads: Vec<LoadPick>,
    /// Heat: faces held at temperatures, heat coming in, air cooling.
    #[serde(default)]
    pub holds: Vec<HoldPick>,
    #[serde(default)]
    pub heats: Vec<HeatPick>,
    #[serde(default)]
    pub convection: Option<Convection>,
    #[serde(default = "default_resolution")]
    pub resolution: usize,
    /// The stress study takes in the temperatures the heat study finds
    /// (thermal expansion), against this stress-free temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heat_stress: Option<HeatStress>,
    /// Re-solve the most stressed spot on finer cubes for its true peak.
    #[serde(default = "yes")]
    pub refine: bool,
    /// Faces that slide (held square to themselves only): symmetry planes, frictionless supports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sliding: Vec<FacePick>,
    /// Faces moved by set amounts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub displacements: Vec<crate::studies::DisplacementPick>,
    /// Faces on springs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elastic_supports: Vec<crate::studies::ElasticPick>,
    /// Bearing loads on round faces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bearing_loads: Vec<crate::studies::BearingPick>,
    /// Forces and moments at a point, carried to a face.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_loads: Vec<crate::studies::RemotePick>,
    /// Gravity (m/s²) on the parts' own weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity: Option<[f64; 3]>,
    /// Spinning about an axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spin: Option<Spin>,
    /// Bolts pulled tight first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bolts: Vec<Bolt>,
    /// Let the material yield (bilinear plasticity).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plastic: bool,
    /// Vibration counts the stiffness the loads' stress adds or takes away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prestress: bool,
    /// How the loads come and go, for a fatigue estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fatigue: Option<post::FatigueSpec>,
    /// Heat over time instead of steady.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transient: Option<Transient>,
}

fn yes() -> bool {
    true
}

/// Heat as a load in the stress study.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct HeatStress {
    /// °C at which the part is free of stress (as made or assembled).
    pub reference: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct HoldPick {
    pub face: FacePick,
    /// °C.
    pub temperature: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct HeatPick {
    pub face: FacePick,
    pub watts: f64,
}

impl StudySpec {
    /// Nothing set: the study can go. Any choice made in it (parts studied
    /// with it, how they touch, heat as a load, resolution, sharpening) keeps
    /// it, even with every face removed.
    pub fn is_empty(&self) -> bool {
        self.supports.is_empty()
            && self.loads.is_empty()
            && self.holds.is_empty()
            && self.heats.is_empty()
            && self.convection.is_none()
            && self.parts.is_empty()
            && self.contacts.is_empty()
            && self.heat_stress.is_none()
            && self.resolution == default_resolution()
            && self.refine
            && self.sliding.is_empty()
            && self.displacements.is_empty()
            && self.elastic_supports.is_empty()
            && self.bearing_loads.is_empty()
            && self.remote_loads.is_empty()
            && self.gravity.is_none()
            && self.spin.is_none()
            && self.bolts.is_empty()
            && !self.plastic
            && !self.prestress
            && self.fatigue.is_none()
            && self.transient.is_none()
    }
}

impl LoadPick {
    /// The force (N) on a face facing `normal`.
    pub fn force(&self, normal: [f64; 3]) -> [f64; 3] {
        let unit = match self.direction.as_str() {
            "-z" => [0.0, 0.0, -1.0],
            "+z" => [0.0, 0.0, 1.0],
            "-x" => [-1.0, 0.0, 0.0],
            "+x" => [1.0, 0.0, 0.0],
            "-y" => [0.0, -1.0, 0.0],
            "+y" => [0.0, 1.0, 0.0],
            _ => normal.map(|c| -c),
        };
        unit.map(|c| c * self.magnitude)
    }

    /// The load it puts on the face as found now.
    pub fn on(&self, face: Face) -> Load {
        let pressure = match self.direction.as_str() {
            "pressure" => self.magnitude,
            "into" if face.curved => self.magnitude / face.area().max(1e-12),
            _ => 0.0,
        };
        let force = if pressure != 0.0 { [0.0; 3] } else { self.force(face.normal) };
        Load { face, force, pressure }
    }
}

/// A joint's seat on a part: the material nearest an axis (the skin of a
/// bore, or the core of a shaft along it).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Bore {
    pub at: [f64; 3],
    pub axis: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BoreLoad {
    pub bore: Bore,
    pub force: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Load {
    pub face: Face,
    /// Total force (N), spread evenly over the face.
    pub force: [f64; 3],
    /// Pressure (MPa) besides: every bit of the face pushed straight in
    /// along its own normal.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pressure: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Study {
    pub supports: Vec<Face>,
    pub loads: Vec<Load>,
    /// Bores (a joint's seat: a pin's hole or a shaft) held still.
    #[serde(default)]
    pub bore_supports: Vec<Bore>,
    /// Forces spread over bores.
    #[serde(default)]
    pub bore_loads: Vec<BoreLoad>,
    /// Cubes along the part's longest side.
    #[serde(default = "default_resolution")]
    pub resolution: usize,
    /// With several parts: how each pair acts where they touch (pairs not
    /// listed are frictionless).
    #[serde(default)]
    pub contacts: Vec<ContactSpec>,
    /// Temperatures as a load: the part expands where it is warmer than
    /// the reference, and is stressed where it cannot.
    #[serde(default)]
    pub heat: Option<HeatLoad>,
    /// Re-solve the most stressed spot on cubes several times finer, with
    /// the coarse solution's movement on its boundary, and read the peak at
    /// the surface (see `refine`).
    #[serde(default)]
    pub refine: bool,
    /// Gravity (m/s², e.g. [0, 0, −9.81]) pulling on every part's own mass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity: Option<[f64; 3]>,
    /// Spinning about an axis: every bit pulled outward by ρ·ω²·r.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spin: Option<Spin>,
    /// A shaft pressing in a bore (or a bearing on a shaft): the force spread
    /// over the half of the round face it pushes into, most where it pushes squarely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bearing_loads: Vec<BearingLoad>,
    /// A force and moment at a point off the part, carried to a face as by a
    /// rigid bracket (a motor's torque, a lever's pull).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_loads: Vec<RemoteLoad>,
    /// Faces that slide: held square to themselves only (a symmetry plane, a
    /// frictionless support, a bearing seat that holds radially).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sliding: Vec<Face>,
    /// Faces moved by set amounts along some axes (free along the others).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub displacements: Vec<Displacement>,
    /// Faces resting on springs (a rubber foot, a foundation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elastic_supports: Vec<ElasticSupport>,
    /// Bolts pulled tight before the loads come on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bolts: Vec<Bolt>,
    /// Let the material yield: bilinear plasticity (von Mises, hardening at
    /// the material's tangent modulus), the loads applied in steps.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plastic: bool,
}

impl Default for Study {
    fn default() -> Study {
        Study {
            supports: vec![],
            loads: vec![],
            bore_supports: vec![],
            bore_loads: vec![],
            resolution: default_resolution(),
            contacts: vec![],
            heat: None,
            refine: false,
            gravity: None,
            spin: None,
            bearing_loads: vec![],
            remote_loads: vec![],
            sliding: vec![],
            displacements: vec![],
            elastic_supports: vec![],
            bolts: vec![],
            plastic: false,
        }
    }
}

impl Study {
    /// Whether anything holds the parts (a support of any kind).
    pub fn held(&self) -> bool {
        !self.supports.is_empty() || !self.bore_supports.is_empty() || !self.sliding.is_empty() || !self.displacements.is_empty() || !self.elastic_supports.is_empty()
    }

    /// Whether anything loads them besides heat and press fits.
    pub fn loaded(&self) -> bool {
        !self.loads.is_empty()
            || !self.bore_loads.is_empty()
            || self.gravity.is_some()
            || self.spin.is_some()
            || !self.bearing_loads.is_empty()
            || !self.remote_loads.is_empty()
            || !self.bolts.is_empty()
            || self.displacements.iter().any(|d| d.by.iter().any(|v| v.is_some_and(|v| v != 0.0)))
    }
}

/// Spinning about an axis.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Spin {
    /// A point on the axis (mm) and its direction.
    pub at: [f64; 3],
    pub axis: [f64; 3],
    pub rpm: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BearingLoad {
    /// A round face (a bore or a shaft's side).
    pub face: Face,
    /// N.
    pub force: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RemoteLoad {
    pub face: Face,
    /// Where the force acts (mm), and the force (N) and moment (N·mm) there.
    pub point: [f64; 3],
    #[serde(default)]
    pub force: [f64; 3],
    #[serde(default)]
    pub moment: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Displacement {
    pub face: Face,
    /// mm along x, y, z; null leaves that direction free.
    pub by: [Option<f64>; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ElasticSupport {
    pub face: Face,
    /// Stiffness per area (N/mm per mm², i.e. N/mm³: a foundation modulus).
    pub stiffness: f64,
    /// Only square to the face (it still slides along it).
    #[serde(default)]
    pub normal_only: bool,
}

/// A bolt pretensioned: the bolt part is shortened across its section at
/// `at` until it pulls with `preload`, then held at that length while the loads come on.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Bolt {
    /// The bolt's part (none: the first part).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// A point on the shank (mm) and the bolt's axis.
    pub at: [f64; 3],
    pub axis: [f64; 3],
    /// N.
    pub preload: f64,
}

/// Temperatures loading a stress study.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct HeatLoad {
    /// °C at which the parts are free of stress (as made or assembled).
    pub reference: f64,
    /// One temperature throughout every part (°C), when there is no field.
    #[serde(default)]
    pub uniform: Option<f64>,
    /// A heat study to run first on the studied part (the first): its
    /// temperatures load it (see `stress_study`).
    #[serde(default)]
    pub thermal: Option<ThermalStudy>,
    /// The temperatures of a heat study already run (on the first part).
    #[serde(skip)]
    pub field: Option<Temperatures>,
}

fn default_resolution() -> usize {
    48
}

#[derive(Clone, Debug, Serialize)]
pub struct Result_ {
    /// The cube size (mm) and the grid's corner and counts.
    pub cell: f64,
    pub origin: [f64; 3],
    pub counts: [usize; 3],
    /// Filled cubes: their (i, j, k) and von Mises stress (MPa).
    pub cells: Vec<([u32; 3], f32)>,
    /// Displacement (mm) at each corner of the grid that the part uses: (i, j, k) and (dx, dy, dz).
    pub displacements: Vec<([u32; 3], [f32; 3])>,
    pub max_stress: f64,
    pub max_displacement: f64,
    pub yield_strength: f64,
    /// Yield strength over the largest stress.
    pub safety: f64,
    pub elements: usize,
    pub iterations: usize,
    pub notes: Vec<String>,
    /// Several parts: each one's figures, which part each cube is of (index
    /// into `parts`), and which part each displacement is of.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cell_parts: Vec<u8>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub node_parts: Vec<u8>,
    /// Where the parts touch: pressure, what opened, the force through each pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<contact::Contacts>,
    /// Heat as a load: the temperatures and what they alone do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heat: Option<HeatSummary>,
    /// The most stressed spot re-solved on finer cubes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak: Option<Peak>,
    /// Where the largest stress and the largest movement are (mm).
    pub max_stress_at: [f64; 3],
    pub max_displacement_at: [f64; 3],
    /// What each hold pushes back with.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    /// Pretensioned bolts: their preload and their pull under the loads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bolts: Vec<BoltResult>,
    /// Plasticity: what yielded, and each cube's equivalent plastic strain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plastic: Option<PlasticSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plastic_strain: Vec<f32>,
    /// Each cube's stress (xx yy zz xy yz zx, MPa), for probes, sections and fatigue.
    #[serde(skip)]
    pub tensors: Vec<[f32; 6]>,
    /// Each part's yield strength (MPa), for the safety map.
    #[serde(skip)]
    pub yields: Vec<f64>,
}

/// What heat did in a stress study.
#[derive(Clone, Debug, Serialize)]
pub struct HeatSummary {
    /// °C: stress-free, and the coolest and hottest the parts get.
    pub reference: f64,
    pub min: f64,
    pub max: f64,
    /// The largest stress from the heat alone, and from the loads alone
    /// (MPa); the map shows the two together. None when contact makes them
    /// inseparable (it is not linear).
    pub thermal_stress: Option<f64>,
    pub mechanical_stress: Option<f64>,
}

/// One part's figures in a several-part study.
#[derive(Clone, Debug, Serialize)]
pub struct PartResult {
    pub name: String,
    pub max_stress: f64,
    pub max_displacement: f64,
    pub yield_strength: f64,
    pub safety: f64,
    /// Not held by a support, and free to move where its contacts let it.
    pub floating: bool,
}

// ---- The element ---------------------------------------------------------------

const CORNERS: [[f64; 3]; 8] = [[-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]];

fn d_matrix(m: &Elastic) -> [[f64; 6]; 6] {
    let (e, nu) = (m.e, m.nu);
    let l = e * nu / ((1.0 + nu) * (1.0 - 2.0 * nu));
    let g = e / (2.0 * (1.0 + nu));
    let mut d = [[0.0; 6]; 6];
    for i in 0..3 {
        for j in 0..3 {
            d[i][j] = l;
        }
        d[i][i] = l + 2.0 * g;
        d[i + 3][i + 3] = g;
    }
    d
}

/// Strain from nodal displacements (6×24) at natural coordinates, cube side h.
fn b_matrix(xi: [f64; 3], h: f64) -> [[f64; 24]; 6] {
    let mut b = [[0.0; 24]; 6];
    for (n, c) in CORNERS.iter().enumerate() {
        // dN/dξ for N = ⅛(1+ξξn)(1+ηηn)(1+ζζn), times 2/h for x.
        let f = |k: usize| 1.0 + xi[k] * c[k];
        let d = [c[0] * f(1) * f(2) / 8.0 * 2.0 / h, c[1] * f(0) * f(2) / 8.0 * 2.0 / h, c[2] * f(0) * f(1) / 8.0 * 2.0 / h];
        let k = 3 * n;
        b[0][k] = d[0];
        b[1][k + 1] = d[1];
        b[2][k + 2] = d[2];
        b[3][k] = d[1];
        b[3][k + 1] = d[0];
        b[4][k + 1] = d[2];
        b[4][k + 2] = d[1];
        b[5][k] = d[2];
        b[5][k + 2] = d[0];
    }
    b
}

/// Strain from the nine incompatible modes (1−ξ², 1−η², 1−ζ² per component).
fn g_matrix(xi: [f64; 3], h: f64) -> [[f64; 9]; 6] {
    let mut g = [[0.0; 9]; 6];
    for m in 0..3 {
        // Mode m varies along axis m: d(1−ξm²)/dxm = −2ξm·2/h.
        let dm = -2.0 * xi[m] * 2.0 / h;
        for comp in 0..3 {
            let col = 3 * m + comp;
            let mut grad = [0.0; 3];
            grad[m] = dm;
            match comp {
                0 => {
                    g[0][col] = grad[0];
                    g[3][col] = grad[1];
                    g[5][col] = grad[2];
                }
                1 => {
                    g[1][col] = grad[1];
                    g[3][col] = grad[0];
                    g[4][col] = grad[2];
                }
                _ => {
                    g[2][col] = grad[2];
                    g[4][col] = grad[1];
                    g[5][col] = grad[0];
                }
            }
        }
    }
    g
}

/// The condensed 24×24 stiffness of a cube of side h.
fn element_stiffness(m: &Elastic, h: f64) -> Vec<f64> {
    element_stiffness_d(&d_matrix(m), h)
}

/// The same with any material matrix (an elastoplastic tangent).
fn element_stiffness_d(d: &[[f64; 6]; 6], h: f64) -> Vec<f64> {
    let gp = 1.0 / 3f64.sqrt();
    let w = (h / 2.0).powi(3); // Jacobian determinant × unit weights
    let mut kuu = vec![0.0; 24 * 24];
    let mut kua = vec![0.0; 24 * 9];
    let mut kaa = vec![0.0; 81];
    for c in CORNERS {
        let xi = [c[0] * gp, c[1] * gp, c[2] * gp];
        let b = b_matrix(xi, h);
        let g = g_matrix(xi, h);
        // D·B and D·G.
        let db: Vec<[f64; 24]> = (0..6).map(|r| std::array::from_fn(|j| (0..6).map(|k| d[r][k] * b[k][j]).sum())).collect();
        let dg: Vec<[f64; 9]> = (0..6).map(|r| std::array::from_fn(|j| (0..6).map(|k| d[r][k] * g[k][j]).sum())).collect();
        for i in 0..24 {
            for j in 0..24 {
                kuu[i * 24 + j] += w * (0..6).map(|r| b[r][i] * db[r][j]).sum::<f64>();
            }
            for j in 0..9 {
                kua[i * 9 + j] += w * (0..6).map(|r| b[r][i] * dg[r][j]).sum::<f64>();
            }
        }
        for i in 0..9 {
            for j in 0..9 {
                kaa[i * 9 + j] += w * (0..6).map(|r| g[r][i] * dg[r][j]).sum::<f64>();
            }
        }
    }
    // K = Kuu − Kua Kaa⁻¹ Kau.
    let inv = invert(&kaa, 9);
    let mut k = kuu;
    for i in 0..24 {
        for j in 0..24 {
            let mut s = 0.0;
            for a in 0..9 {
                for bb in 0..9 {
                    s += kua[i * 9 + a] * inv[a * 9 + bb] * kua[j * 9 + bb];
                }
            }
            k[i * 24 + j] -= s;
        }
    }
    k
}

fn invert(a: &[f64], n: usize) -> Vec<f64> {
    let mut m = a.to_vec();
    let mut inv: Vec<f64> = (0..n * n).map(|i| if i / n == i % n { 1.0 } else { 0.0 }).collect();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| m[i * n + c].abs().total_cmp(&m[j * n + c].abs())).unwrap();
        for k in 0..n {
            m.swap(c * n + k, p * n + k);
            inv.swap(c * n + k, p * n + k);
        }
        let d = m[c * n + c];
        for k in 0..n {
            m[c * n + k] /= d;
            inv[c * n + k] /= d;
        }
        for r in 0..n {
            if r != c {
                let f = m[r * n + c];
                for k in 0..n {
                    m[r * n + k] -= f * m[c * n + k];
                    inv[r * n + k] -= f * inv[c * n + k];
                }
            }
        }
    }
    inv
}

// ---- Voxels --------------------------------------------------------------------

struct Grid {
    h: f64,
    origin: [f64; 3],
    n: [usize; 3],
    /// How much of each cube is material (0–1).
    fill: Vec<f32>,
}

impl Grid {
    fn cell(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.n[0] * (j + self.n[1] * k)
    }
    fn node(&self, i: usize, j: usize, k: usize) -> usize {
        i + (self.n[0] + 1) * (j + (self.n[1] + 1) * k)
    }
    fn node_at(&self, id: usize) -> [usize; 3] {
        let nx = self.n[0] + 1;
        let ny = self.n[1] + 1;
        [id % nx, (id / nx) % ny, id / (nx * ny)]
    }
    fn position(&self, id: usize) -> [f64; 3] {
        let [i, j, k] = self.node_at(id);
        [self.origin[0] + i as f64 * self.h, self.origin[1] + j as f64 * self.h, self.origin[2] + k as f64 * self.h]
    }
    /// The 8 corner nodes of a cell, in the element's corner order.
    fn corners(&self, i: usize, j: usize, k: usize) -> [usize; 8] {
        CORNERS.map(|c| self.node(i + (c[0] > 0.0) as usize, j + (c[1] > 0.0) as usize, k + (c[2] > 0.0) as usize))
    }
}

/// Cubes less full than this are left out (slivers add nothing but trouble).
const MIN_FILL: f32 = 0.12;

/// The bounds of a mesh's triangles.
fn bounds(positions: &[f32], triangles: &[u32]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for &t in triangles {
        let i = t as usize * 3;
        for k in 0..3 {
            lo[k] = lo[k].min(positions[i + k] as f64);
            hi[k] = hi[k].max(positions[i + k] as f64);
        }
    }
    (lo, hi)
}

/// An empty grid over a box: `resolution` cubes along its longest side, centred on it.
fn grid_around(lo: [f64; 3], hi: [f64; 3], resolution: usize) -> Grid {
    let longest = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    let h = longest / resolution.max(4) as f64;
    let n = [0, 1, 2].map(|k| (((hi[k] - lo[k]) / h) - 1e-9).ceil().max(1.0) as usize);
    let origin = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0 - n[k] as f64 * h / 2.0);
    Grid { h, origin, n, fill: vec![0.0; n[0] * n[1] * n[2]] }
}

/// How full of material each cube is (rays along x, by parity).
fn voxelize(positions: &[f32], triangles: &[u32], resolution: usize) -> Grid {
    let (lo, hi) = bounds(positions, triangles);
    let mut grid = grid_around(lo, hi, resolution);
    grid.fill = fill_of(&grid, positions, triangles);
    grid
}

/// How full of one mesh each cube of a grid is.
fn fill_of(grid: &Grid, positions: &[f32], triangles: &[u32]) -> Vec<f32> {
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    let (h, origin, n) = (grid.h, grid.origin, grid.n);
    let mut fill = vec![0.0f32; n[0] * n[1] * n[2]];
    // Triangles binned by the (j, k) columns their y–z boxes cover.
    let mut bins: Vec<Vec<usize>> = vec![vec![]; n[1] * n[2]];
    let tris: Vec<[[f64; 3]; 3]> = triangles.chunks_exact(3).map(|t| [p(t[0]), p(t[1]), p(t[2])]).collect();
    for (ti, t) in tris.iter().enumerate() {
        let col = |v: f64, k: usize| ((v - origin[k]) / h - 0.5).floor();
        let (y0, y1) = (t.iter().map(|q| q[1]).fold(f64::INFINITY, f64::min), t.iter().map(|q| q[1]).fold(f64::NEG_INFINITY, f64::max));
        let (z0, z1) = (t.iter().map(|q| q[2]).fold(f64::INFINITY, f64::min), t.iter().map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max));
        let (ja, jb) = ((col(y0, 1) as i64).max(0), (col(y1, 1) as i64 + 1).min(n[1] as i64 - 1));
        let (ka, kb) = ((col(z0, 2) as i64).max(0), (col(z1, 2) as i64 + 1).min(n[2] as i64 - 1));
        for k in ka..=kb {
            for j in ja..=jb {
                bins[j as usize + n[1] * k as usize].push(ti);
            }
        }
    }
    // Each column: 4 × 4 rays along x; each cube's fill is the share of its
    // length inside the part, averaged over the rays (exact along x).
    const RAYS: usize = 4;
    for k in 0..n[2] {
        for j in 0..n[1] {
            for (ry, rz) in (0..RAYS).flat_map(|a| (0..RAYS).map(move |b| (a, b))) {
                // Nudged off any edge it might graze.
                let y = origin[1] + (j as f64 + (ry as f64 + 0.5) / RAYS as f64) * h + h * 1.234e-7;
                let z = origin[2] + (k as f64 + (rz as f64 + 0.5) / RAYS as f64) * h + h * 2.345e-7;
                let mut xs: Vec<f64> = vec![];
                for &ti in &bins[j + n[1] * k] {
                    let [a, b, c] = tris[ti];
                    let det = (b[1] - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (b[2] - a[2]);
                    if det.abs() < 1e-18 {
                        continue;
                    }
                    let u = ((y - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (z - a[2])) / det;
                    let v = ((b[1] - a[1]) * (z - a[2]) - (y - a[1]) * (b[2] - a[2])) / det;
                    if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                        xs.push(a[0] + u * (b[0] - a[0]) + v * (c[0] - a[0]));
                    }
                }
                xs.sort_by(f64::total_cmp);
                for pair in xs.chunks_exact(2) {
                    let (i0, i1) = (((pair[0] - origin[0]) / h).floor().max(0.0) as usize, (((pair[1] - origin[0]) / h).ceil() as usize).min(n[0]));
                    for i in i0..i1 {
                        let (x0, x1) = (origin[0] + i as f64 * h, origin[0] + (i + 1) as f64 * h);
                        let inside = (pair[1].min(x1) - pair[0].max(x0)).max(0.0) / h;
                        fill[grid.cell(i, j, k)] += (inside / (RAYS * RAYS) as f64) as f32;
                    }
                }
            }
        }
    }
    fill
}

/// How much of the part each node serves (the cubes around it, by fill): a
/// face load shared by these weights spreads evenly over the face's area,
/// instead of piling onto its corners.
fn node_shares(nodes: usize, corners: &[[usize; 8]], weight: &[f64]) -> Vec<f64> {
    let mut share = vec![0.0; nodes];
    for (c, w) in corners.iter().zip(weight) {
        for &n in c {
            share[n] += w;
        }
    }
    share
}

/// Spreads `total` over `on` by their shares.
fn spread(on: &[usize], share: &[f64], total: f64) -> Vec<(usize, f64)> {
    let sum: f64 = on.iter().map(|&n| share[n]).sum();
    on.iter().map(|&n| (n, if sum > 0.0 { total * share[n] / sum } else { total / on.len() as f64 })).collect()
}

/// Nodes among `ids` nearest an axis: within a cube and a half of the closest.
fn nodes_near_axis(ids: impl Iterator<Item = usize>, position: impl Fn(usize) -> [f64; 3], h: f64, bore: &Bore) -> Vec<usize> {
    let l = (bore.axis[0] * bore.axis[0] + bore.axis[1] * bore.axis[1] + bore.axis[2] * bore.axis[2]).sqrt().max(1e-12);
    let u = bore.axis.map(|c| c / l);
    let off = |id: usize| {
        let q = position(id);
        let d = [q[0] - bore.at[0], q[1] - bore.at[1], q[2] - bore.at[2]];
        let t = d[0] * u[0] + d[1] * u[1] + d[2] * u[2];
        ((d[0] - t * u[0]).powi(2) + (d[1] - t * u[1]).powi(2) + (d[2] - t * u[2]).powi(2)).sqrt()
    };
    let near: Vec<(usize, f64)> = ids.map(|id| (id, off(id))).collect();
    let closest = near.iter().map(|(_, d)| *d).fold(f64::INFINITY, f64::min);
    near.into_iter().filter(|(_, d)| *d <= closest + 1.5 * h).map(|(id, _)| id).collect()
}

/// Nodes among `ids` that lie on a picked face.
fn nodes_on_face(ids: impl Iterator<Item = usize>, position: impl Fn(usize) -> [f64; 3], h: f64, face: &Face) -> Vec<usize> {
    if face.curved {
        return nodes_on_surface(ids, position, h, face);
    }
    let n = face.normal;
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
    let n = n.map(|c| c / len);
    let tris: Vec<[[f64; 3]; 3]> = face.triangles.chunks_exact(9).map(|t| [[t[0], t[1], t[2]], [t[3], t[4], t[5]], [t[6], t[7], t[8]]]).collect();
    let margin = h * 0.55;
    let inside = |q: [f64; 3]| {
        tris.iter().any(|t| {
            // Project onto the triangle's plane, then test with a margin of half a cube.
            let e = |a: [f64; 3], b: [f64; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
            // Edges' outward sides, whichever way the triangle winds.
            let wind = dot(cross(e(t[0], t[1]), e(t[0], t[2])), n).signum();
            (0..3).all(|k| {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let edge = e(a, b);
                let out = cross(edge, n).map(|c| c * wind);
                let l = dot(out, out).sqrt().max(1e-12);
                dot(e(a, q), out) / l <= margin
            })
        })
    };
    ids.filter(|&id| {
        let q = position(id);
        let d = (q[0] - face.point[0]) * n[0] + (q[1] - face.point[1]) * n[1] + (q[2] - face.point[2]) * n[2];
        d.abs() <= margin && inside(q)
    })
    .collect()
}

/// Nodes among `ids` that lie on a curved face: within half a cube of one
/// of its triangles (square to it, and past its edges), each triangle in its
/// own plane. Triangles are binned in boxes so each node meets only those near it.
fn nodes_on_surface(ids: impl Iterator<Item = usize>, position: impl Fn(usize) -> [f64; 3], h: f64, face: &Face) -> Vec<usize> {
    let margin = h * 0.55;
    let tris: Vec<([[f64; 3]; 3], [f64; 3])> = face
        .tris()
        .filter_map(|t| {
            let c = v_cross(v_sub(t[1], t[0]), v_sub(t[2], t[0]));
            let l = v_norm(c);
            (l > 1e-12).then(|| (t, c.map(|v| v / l)))
        })
        .collect();
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for (t, _) in &tris {
        for p in t {
            for d in 0..3 {
                lo[d] = lo[d].min(p[d] - margin);
                hi[d] = hi[d].max(p[d] + margin);
            }
        }
    }
    let size = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]);
    let s = (2.0 * h).max(size / 48.0);
    let bin = |q: [f64; 3]| [0, 1, 2].map(|d| ((q[d] - lo[d]) / s).floor() as i64);
    let mut bins: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (i, (t, _)) in tris.iter().enumerate() {
        let a = bin([0, 1, 2].map(|d| t[0][d].min(t[1][d]).min(t[2][d]) - margin));
        let b = bin([0, 1, 2].map(|d| t[0][d].max(t[1][d]).max(t[2][d]) + margin));
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    bins.entry([x, y, z]).or_default().push(i);
                }
            }
        }
    }
    let on = |q: [f64; 3]| {
        bins.get(&bin(q)).is_some_and(|list| {
            list.iter().any(|&i| {
                let (t, n) = tris[i];
                v_dot(v_sub(q, t[0]), n).abs() <= margin
                    && (0..3).all(|k| {
                        let (a, b) = (t[k], t[(k + 1) % 3]);
                        let out = v_cross(v_sub(b, a), n);
                        v_dot(v_sub(q, a), out) / v_norm(out).max(1e-12) <= margin
                    })
            })
        })
    };
    ids.filter(|&id| {
        let q = position(id);
        (0..3).all(|d| q[d] >= lo[d] && q[d] <= hi[d]) && on(q)
    })
    .collect()
}

/// How a curved face's area falls to the nodes `on` it: each triangle cut
/// into pieces under half a cube across, each piece going to the node of
/// `on` nearest it, with its area and its area times its outward normal.
/// Only pieces whose middle is `within` count (a re-solved piece of the part).
fn area_on_nodes(on: &[usize], position: impl Fn(usize) -> [f64; 3], h: f64, face: &Face, within: impl Fn([f64; 3]) -> bool) -> Vec<(usize, f64, [f64; 3])> {
    let Some(&first) = on.first() else { return vec![] };
    // The nodes are on a lattice of spacing h: key them by where they sit on it.
    let base = position(first);
    let key = |q: [f64; 3]| [0, 1, 2].map(|d| ((q[d] - base[d]) / h + 0.5).floor() as i64);
    let mut at: HashMap<[i64; 3], usize> = HashMap::with_capacity(on.len());
    for (k, &id) in on.iter().enumerate() {
        at.insert(key(position(id)), k);
    }
    let places: Vec<[f64; 3]> = on.iter().map(|&id| position(id)).collect();
    let nearest = |c: [f64; 3]| -> usize {
        let g = key(c);
        let mut best: Option<(f64, usize)> = None;
        for r in 1..=3i64 {
            for x in -r..=r {
                for y in -r..=r {
                    for z in -r..=r {
                        if let Some(&k) = at.get(&[g[0] + x, g[1] + y, g[2] + z]) {
                            let d = v_sub(places[k], c);
                            let d = v_dot(d, d);
                            if best.is_none_or(|(b, _)| d < b) {
                                best = Some((d, k));
                            }
                        }
                    }
                }
            }
            if best.is_some() {
                break;
            }
        }
        // Rarely, no node within three cubes: the nearest of all.
        best.map(|(_, k)| k).unwrap_or_else(|| {
            (0..places.len())
                .min_by(|&a, &b| {
                    let (da, db) = (v_sub(places[a], c), v_sub(places[b], c));
                    v_dot(da, da).total_cmp(&v_dot(db, db))
                })
                .unwrap_or(0)
        })
    };
    let mut area = vec![0.0; on.len()];
    let mut vector = vec![[0.0; 3]; on.len()];
    for t in face.tris() {
        let c = v_cross(v_sub(t[1], t[0]), v_sub(t[2], t[0]));
        let whole = v_norm(c) / 2.0;
        if whole <= 0.0 {
            continue;
        }
        let n = c.map(|v| v / (2.0 * whole));
        let longest = (0..3).map(|k| v_norm(v_sub(t[(k + 1) % 3], t[k]))).fold(0.0, f64::max);
        let m = ((longest / (0.5 * h)).ceil() as usize).clamp(1, 64);
        let piece = whole / (m * m) as f64;
        let point = |u: f64, v: f64| [0, 1, 2].map(|d| t[0][d] + u * (t[1][d] - t[0][d]) + v * (t[2][d] - t[0][d]));
        let mut put = |c: [f64; 3]| {
            if within(c) {
                let k = nearest(c);
                area[k] += piece;
                for d in 0..3 {
                    vector[k][d] += piece * n[d];
                }
            }
        };
        // m² equal pieces: m(m+1)/2 upright, m(m−1)/2 upside down.
        let mf = m as f64;
        for i in 0..m {
            for j in 0..m - i {
                put(point((i as f64 + 1.0 / 3.0) / mf, (j as f64 + 1.0 / 3.0) / mf));
                if i + j + 1 < m {
                    put(point((i as f64 + 2.0 / 3.0) / mf, (j as f64 + 2.0 / 3.0) / mf));
                }
            }
        }
    }
    on.iter().enumerate().filter(|(k, _)| area[*k] > 0.0).map(|(k, &id)| (id, area[k], vector[k])).collect()
}

/// The forces a load puts on the nodes `on` its face (free nodes, part
/// coordinates). A flat face shares its force by the nodes' shares, its
/// pressure as a force square to it; a curved face spreads its force by
/// area and pushes its pressure in along each bit's own normal. `within`
/// picks out the part of a curved face that counts.
fn load_forces(on: &[usize], position: impl Fn(usize) -> [f64; 3], h: f64, share: &[f64], load: &Load, within: impl Fn([f64; 3]) -> bool) -> Vec<(usize, [f64; 3])> {
    if !load.face.curved {
        let n = load.face.normal;
        let n = n.map(|c| c / v_norm(n).max(1e-12));
        let push = load.pressure * load.face.area();
        let total: [f64; 3] = std::array::from_fn(|d| load.force[d] - push * n[d]);
        let mut out: Vec<(usize, [f64; 3])> = on.iter().map(|&id| (id, [0.0; 3])).collect();
        for d in 0..3 {
            for (k, (_, v)) in spread(on, share, total[d]).into_iter().enumerate() {
                out[k].1[d] = v;
            }
        }
        return out;
    }
    let whole = load.face.area().max(1e-12);
    area_on_nodes(on, position, h, &load.face, within)
        .into_iter()
        .map(|(id, a, av)| (id, std::array::from_fn(|d| load.force[d] * a / whole - load.pressure * av[d])))
        .collect()
}

/// The pushes in a load, added up regardless of direction (N): the scale it is judged by.
fn load_size(load: &Load) -> f64 {
    v_norm(load.force) + load.pressure.abs() * load.face.area()
}

/// Grid nodes the part uses that lie on a picked face.
fn nodes_on(grid: &Grid, used: &[bool], face: &Face) -> Vec<usize> {
    nodes_on_face((0..used.len()).filter(|&id| used[id]), |id| grid.position(id), grid.h, face)
}

// ---- Solving -------------------------------------------------------------------

/// One part's nodes: the corners of a box of the grid's cubes (`n` cubes from
/// cube `lo` along each axis), numbered from `first`. Parts studied together
/// each have their own nodes, so where they touch they can part or slide.
#[derive(Clone, Debug)]
struct Block {
    lo: [usize; 3],
    n: [usize; 3],
    first: usize,
}

impl Block {
    fn count(&self) -> usize {
        (self.n[0] + 1) * (self.n[1] + 1) * (self.n[2] + 1)
    }
    fn end(&self) -> usize {
        self.first + self.count()
    }
    /// The node at grid corner `g` (which must be in the box).
    fn node(&self, g: [usize; 3]) -> usize {
        self.first + (g[0] - self.lo[0]) + (self.n[0] + 1) * ((g[1] - self.lo[1]) + (self.n[1] + 1) * (g[2] - self.lo[2]))
    }
    /// The grid corner of one of its nodes.
    fn at(&self, id: usize) -> [usize; 3] {
        let l = id - self.first;
        let (nx, ny) = (self.n[0] + 1, self.n[1] + 1);
        [self.lo[0] + l % nx, self.lo[1] + (l / nx) % ny, self.lo[2] + l / (nx * ny)]
    }
    /// The 8 corner nodes of a cube, in the element's corner order.
    fn corners(&self, c: [usize; 3]) -> [usize; 8] {
        CORNERS.map(|o| self.node([c[0] + (o[0] > 0.0) as usize, c[1] + (o[1] > 0.0) as usize, c[2] + (o[2] > 0.0) as usize]))
    }
    /// The same box on a grid of cubes twice the size.
    fn coarser(&self, first: usize) -> Block {
        let lo = self.lo.map(|v| v / 2);
        let n = std::array::from_fn(|d| (self.lo[d] + self.n[d]).div_ceil(2) - lo[d]);
        Block { lo, n, first }
    }
}

/// Numbers blocks one after another.
fn lay_out(boxes: impl Iterator<Item = ([usize; 3], [usize; 3])>) -> Vec<Block> {
    let mut first = 0;
    boxes
        .map(|(lo, n)| {
            let b = Block { lo, n, first };
            first = b.end();
            b
        })
        .collect()
}

/// The block a node is in.
fn block_of(blocks: &[Block], id: usize) -> usize {
    blocks.partition_point(|b| b.end() <= id)
}

/// A stiff link between nodes (a contact or a glued joint): adds k·g·gᵀ to
/// the stiffness, where g is sparse over the unknowns.
#[derive(Clone, Debug)]
struct Spring {
    k: f64,
    g: Vec<(usize, f64)>,
}

/// y += Σ k·g·(g·x).
fn add_springs(springs: &[Spring], x: &[f64], y: &mut [f64]) {
    for s in springs {
        let v = s.k * s.g.iter().map(|&(i, c)| c * x[i]).sum::<f64>();
        if v != 0.0 {
            for &(i, c) in &s.g {
                y[i] += v * c;
            }
        }
    }
}

/// How weak (beside a cube's own stiffness) the springs are that keep a part
/// with no support from drifting off: far too weak to carry load, but enough
/// that a part held only by frictionless contact has a definite answer.
const SOFT: f64 = 1e-9;
/// A part whose weak tie carries more than this share of the load is drifting.
const DRIFT: f64 = 1e-3;

/// The cubes of the parts, their stiffness, and which nodes supports hold.
struct Model {
    grid: Grid,
    blocks: Vec<Block>,
    corners: Vec<[usize; 8]>,
    cells: Vec<[usize; 3]>,
    /// Each cube's part (index into `blocks` and `kes`).
    part: Vec<usize>,
    nodes: usize,
    used: Vec<bool>,
    fixed: Vec<bool>,
    /// Each part's cube stiffness, in its own material.
    kes: Vec<Vec<f64>>,
    /// Each cube's share of material (scales its stiffness and mass).
    weight: Vec<f64>,
    /// The diagonal of the stiffness: of the cubes alone, and with the springs.
    el_diag: Vec<f64>,
    diag: Vec<f64>,
    /// Links between parts where they touch.
    springs: Vec<Spring>,
    /// Parts held by no support, and the weak tie (per cube corner) that keeps them from drifting.
    floating: Vec<Option<f64>>,
    /// Coarse grids that speed up the solve on big or thin parts.
    mg: Option<multigrid::Multigrid>,
    /// Springs that are part of the study itself (elastic supports, sliding
    /// faces at a slant), kept under whatever contact adds.
    base: Vec<Spring>,
    /// Geometric stiffness from a stress state, 8 × 8 per cube (the same for
    /// each direction): a prestressed part, or the buckling eigenproblem.
    geo: Option<Vec<[f64; 64]>>,
    /// Cubes with a stiffness of their own (an elastoplastic tangent), by cube; empty: none.
    own: Vec<Option<Box<[f64]>>>,
}

impl Model {
    /// Several parts on one grid, each cube going to the part that fills
    /// most of it, each part with nodes of its own. Supports and bores name
    /// the part they hold. `whole`: the grid holds the whole part, so it
    /// needs a support and every support must touch it (a piece of a part,
    /// re-solved finer, is held at its cut instead).
    fn assemble(mut grid: Grid, fills: &[Vec<f32>], materials: &[Elastic], names: &[String], supports: &[(usize, &Face)], bores: &[(usize, &Bore)], whole: bool) -> Result<Model, String> {
        let parts = fills.len();
        let count = grid.fill.len();
        // A cube's stiffness counts all the material in it, whichever part has most.
        let mut owner = vec![usize::MAX; count];
        let mut total = vec![0.0f32; count];
        for c in 0..count {
            for (p, fill) in fills.iter().enumerate() {
                total[c] += fill[c];
                if fill[c] >= MIN_FILL && (owner[c] == usize::MAX || fill[c] > fills[owner[c]][c]) {
                    owner[c] = p;
                }
            }
        }
        grid.fill = total;
        let mut cells = vec![];
        let mut part = vec![];
        for k in 0..grid.n[2] {
            for j in 0..grid.n[1] {
                for i in 0..grid.n[0] {
                    let p = owner[grid.cell(i, j, k)];
                    if p != usize::MAX {
                        cells.push([i, j, k]);
                        part.push(p);
                    }
                }
            }
        }
        if cells.is_empty() {
            return Err("the part is too thin for this resolution: raise it".into());
        }
        // Each part's box of cubes.
        let mut lo = vec![[usize::MAX; 3]; parts];
        let mut hi = vec![[0usize; 3]; parts];
        for (c, &p) in cells.iter().zip(&part) {
            for d in 0..3 {
                lo[p][d] = lo[p][d].min(c[d]);
                hi[p][d] = hi[p][d].max(c[d] + 1);
            }
        }
        if let Some(p) = (0..parts).find(|&p| lo[p][0] == usize::MAX) {
            return Err(format!("{} is too thin for this resolution: raise it", if parts > 1 { format!("“{}”", names[p]) } else { "the part".into() }));
        }
        let blocks = lay_out((0..parts).map(|p| (lo[p], std::array::from_fn(|d| hi[p][d] - lo[p][d]))));
        let nodes = blocks.last().map_or(0, |b| b.end());
        let corners: Vec<[usize; 8]> = cells.iter().zip(&part).map(|(&c, &p)| blocks[p].corners(c)).collect();
        let mut used = vec![false; nodes];
        for c in &corners {
            for &n in c {
                used[n] = true;
            }
        }
        let position = |id: usize| {
            let g = blocks[block_of(&blocks, id)].at(id);
            [0, 1, 2].map(|d| grid.origin[d] + g[d] as f64 * grid.h)
        };
        let of_part = |p: usize| (blocks[p].first..blocks[p].end()).filter(|&id| used[id]);
        let mut fixed = vec![false; 3 * nodes];
        for (s, &(p, face)) in supports.iter().enumerate() {
            let on = nodes_on_face(of_part(p), position, grid.h, face);
            if on.is_empty() && whole {
                return Err(format!("support {} touches no part of the part at this resolution", s + 1));
            }
            for n in on {
                fixed[3 * n..3 * n + 3].fill(true);
            }
        }
        for (s, &(p, bore)) in bores.iter().enumerate() {
            let on = nodes_near_axis(of_part(p), position, grid.h, bore);
            if on.is_empty() && whole {
                return Err(format!("bore {} touches no part of the part", s + 1));
            }
            for n in on {
                fixed[3 * n..3 * n + 3].fill(true);
            }
        }
        let kes: Vec<Vec<f64>> = materials.iter().map(|m| element_stiffness(m, grid.h)).collect();
        let weight: Vec<f64> = cells.iter().map(|&[i, j, k]| (grid.fill[grid.cell(i, j, k)] as f64).min(1.0)).collect();
        Ok(Model { grid, blocks, corners, cells, part, nodes, used, fixed, kes, weight, el_diag: vec![], diag: vec![], springs: vec![], floating: vec![None; parts], mg: None, base: vec![], geo: None, own: vec![] })
    }

    /// Whether each part is held by a support of its own.
    fn held(&self) -> Vec<bool> {
        self.blocks.iter().map(|b| (b.first..b.end()).any(|n| self.used[n] && (self.fixed[3 * n] || self.fixed[3 * n + 1] || self.fixed[3 * n + 2])) || self.base.iter().any(|s| s.g.iter().any(|&(i, _)| (b.first..b.end()).contains(&(i / 3))))).collect()
    }

    /// Finishes the stiffness: parts marked `loose` (held by no support,
    /// nor glued to anything that is) are tied very weakly to where they
    /// are; then the diagonal and the coarse grids.
    fn finish(&mut self, loose: &[bool]) {
        for (p, _) in loose.iter().enumerate().filter(|(_, l)| **l) {
            let ke = &mut self.kes[p];
            let s = SOFT * (0..24).map(|r| ke[r * 25]).sum::<f64>() / 24.0;
            (0..24).for_each(|r| ke[r * 25] += s);
            self.floating[p] = Some(s);
        }
        self.rebuild();
    }

    /// Gives cubes stiffnesses of their own (None: elastic), then rebuilds the diagonal and the coarse grids.
    fn set_own(&mut self, own: Vec<Option<Box<[f64]>>>) {
        self.own = own;
        self.rebuild();
    }

    /// The diagonal of the cubes, the springs (the study's own), and the coarse grids.
    fn rebuild(&mut self) {
        let mut el_diag = vec![0.0; 3 * self.nodes];
        for (e, ((c, w), &p)) in self.corners.iter().zip(&self.weight).zip(&self.part).enumerate() {
            let ke: &[f64] = match self.own.get(e) {
                Some(Some(m)) => m,
                _ => &self.kes[p],
            };
            for (r, &n) in c.iter().enumerate() {
                for d in 0..3 {
                    el_diag[3 * n + d] += w * ke[(3 * r + d) * 24 + 3 * r + d];
                }
            }
        }
        self.el_diag = el_diag;
        self.mg = None;
        self.set_springs(vec![]);
        self.mg = multigrid::Multigrid::build(self);
    }

    /// How hard the weak tie of a part with no support pulls on it (N): more
    /// than a sliver of the load, and the part is drifting freely.
    fn tie_pull(&self, p: usize, u: &[f64]) -> f64 {
        let Some(s) = self.floating[p] else { return 0.0 };
        let mut pull = [0.0; 3];
        for ((c, w), _) in self.corners.iter().zip(&self.weight).zip(&self.part).filter(|(_, q)| **q == p) {
            for &n in c {
                for d in 0..3 {
                    pull[d] += w * s * u[3 * n + d];
                }
            }
        }
        (pull[0] * pull[0] + pull[1] * pull[1] + pull[2] * pull[2]).sqrt()
    }

    /// The grid corner a node sits at.
    fn node_at(&self, id: usize) -> [usize; 3] {
        self.blocks[block_of(&self.blocks, id)].at(id)
    }

    fn position(&self, id: usize) -> [f64; 3] {
        let g = self.node_at(id);
        [0, 1, 2].map(|d| self.grid.origin[d] + g[d] as f64 * self.grid.h)
    }

    /// The nodes a part uses.
    fn part_nodes(&self, p: usize) -> impl Iterator<Item = usize> + '_ {
        (self.blocks[p].first..self.blocks[p].end()).filter(|&id| self.used[id])
    }

    fn free(&self, i: usize) -> bool {
        self.used[i / 3] && !self.fixed[i]
    }

    fn mask(&self, v: &mut [f64]) {
        v.iter_mut().enumerate().for_each(|(i, x)| {
            if !self.free(i) {
                *x = 0.0
            }
        });
    }

    /// Puts links between parts in the stiffness (replacing any before),
    /// dropping their held ends, and brings the coarse grids up to date.
    fn set_springs(&mut self, springs: Vec<Spring>) {
        let mut springs: Vec<Spring> = self.base.iter().cloned().chain(springs).collect();
        for s in springs.iter_mut() {
            s.g.retain(|&(i, c)| c != 0.0 && self.free(i));
        }
        springs.retain(|s| !s.g.is_empty() && s.k > 0.0);
        self.diag = self.el_diag.clone();
        for s in &springs {
            for &(i, c) in &s.g {
                self.diag[i] += s.k * c * c;
            }
        }
        self.springs = springs;
        if let Some(mut mg) = self.mg.take() {
            mg.set_springs(self);
            self.mg = Some(mg);
        }
    }

    /// K·x, element by element, in parallel (held and unused entries zero).
    fn apply(&self, x: &[f64]) -> Vec<f64> {
        let mut y = self.apply_raw(x);
        self.mask(&mut y);
        y
    }

    /// K·x with every entry (a held node's is the force holding it there).
    fn apply_raw(&self, x: &[f64]) -> Vec<f64> {
        self.apply_with(x, true)
    }

    /// K·x; `own`: with each cube's own matrix where it has one (an
    /// elastoplastic tangent), else every cube elastic.
    fn apply_with(&self, x: &[f64], own: bool) -> Vec<f64> {
        let n3 = 3 * self.nodes;
        let mut y = self
            .corners
            .par_chunks(4096)
            .enumerate()
            .zip(self.weight.par_chunks(4096))
            .zip(self.part.par_chunks(4096))
            .fold(
                || vec![0.0; n3],
                |mut y, (((chunk_at, chunk), weights), parts)| {
                    for (i, ((c, w), &p)) in chunk.iter().zip(weights).zip(parts).enumerate() {
                        let e = chunk_at * 4096 + i;
                        let ke: &[f64] = match self.own.get(e) {
                            Some(Some(m)) if own => m,
                            _ => &self.kes[p],
                        };
                        let mut xe = [0.0; 24];
                        for (a, &n) in c.iter().enumerate() {
                            xe[3 * a..3 * a + 3].copy_from_slice(&x[3 * n..3 * n + 3]);
                        }
                        for (r, &n) in c.iter().enumerate() {
                            for d in 0..3 {
                                let row = &ke[(3 * r + d) * 24..(3 * r + d + 1) * 24];
                                y[3 * n + d] += w * row.iter().zip(&xe).map(|(a, b)| a * b).sum::<f64>();
                            }
                        }
                    }
                    y
                },
            )
            .reduce(|| vec![0.0; n3], |mut a, b| {
                a.iter_mut().zip(&b).for_each(|(x, y)| *x += y);
                a
            });
        if let Some(geo) = &self.geo {
            for (c, g) in self.corners.iter().zip(geo) {
                for (a, &na) in c.iter().enumerate() {
                    for (b, &nb) in c.iter().enumerate() {
                        let k = g[8 * a + b];
                        if k != 0.0 {
                            for d in 0..3 {
                                y[3 * na + d] += k * x[3 * nb + d];
                            }
                        }
                    }
                }
            }
        }
        add_springs(&self.springs, x, &mut y);
        y
    }

    /// Solves K·u = f by conjugate gradients (multigrid-preconditioned when built, else Jacobi) from `u`;
    /// returns the iterations and whether it settled.
    fn cg(&self, f: &[f64], u: &mut [f64], tol: f64) -> (usize, bool) {
        let mut r: Vec<f64> = f.to_vec();
        self.mask(&mut r);
        let fnorm = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        let ku = self.apply(u);
        r.iter_mut().zip(&ku).for_each(|(a, b)| *a -= b);
        // Driven only by held nodes moved (a re-solved piece): measure against what that moving asks.
        let fnorm = fnorm.max(r.iter().map(|v| v * v).sum::<f64>().sqrt()).max(1e-300);
        let precondition = |r: &[f64]| -> Vec<f64> {
            match &self.mg {
                Some(mg) => mg.precondition(self, r),
                None => r.iter().zip(&self.diag).enumerate().map(|(i, (v, d))| if self.free(i) { v / d } else { 0.0 }).collect(),
            }
        };
        let mut z = precondition(&r);
        let mut p = z.clone();
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let limit = (20 * 3 * self.cells.len().max(100)).min(40_000);
        let mut it = 0;
        while it < limit {
            if r.iter().map(|v| v * v).sum::<f64>().sqrt() < tol * fnorm {
                return (it, true);
            }
            let q = self.apply(&p);
            let pq: f64 = p.iter().zip(&q).map(|(a, b)| a * b).sum();
            if pq.abs() < 1e-300 {
                break;
            }
            let alpha = rz / pq;
            u.iter_mut().zip(&p).for_each(|(x, y)| *x += alpha * y);
            r.iter_mut().zip(&q).for_each(|(x, y)| *x -= alpha * y);
            it += 1;
            z = precondition(&r);
            let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            let beta = rz2 / rz;
            rz = rz2;
            p.iter_mut().zip(&z).for_each(|(x, y)| *x = y + beta * *x);
        }
        (it, r.iter().map(|v| v * v).sum::<f64>().sqrt() < tol * 1000.0 * fnorm)
    }
}

/// One part of a several-part study: its closed mesh (all parts in the same
/// coordinates, mm) and its material.
pub struct Body<'a> {
    pub name: String,
    pub positions: &'a [f32],
    pub triangles: &'a [u32],
    pub material: Elastic,
}

/// Runs a study on a closed mesh (part coordinates, mm).
pub fn solve(positions: &[f32], triangles: &[u32], material: &Elastic, study: &Study) -> Result<Result_, String> {
    solve_parts(&[Body { name: String::new(), positions, triangles, material: *material }], study)
}

/// Runs a study on several parts together, each in its own material, filled
/// with cubes on one grid and pressing on one another where they touch (see
/// `contact`). Faces name the part they are on (none: the first part); bores
/// are on the first part.
pub fn solve_parts(bodies: &[Body], study: &Study) -> Result<Result_, String> {
    let mut s = Setup::build(bodies, study, true)?;
    let several = bodies.len() > 1;
    let count = bodies.len();
    let nodes = s.model.nodes;
    let h = s.model.grid.h;
    let mut notes = std::mem::take(&mut s.notes);
    let ds: Vec<[[f64; 6]; 6]> = s.materials.iter().map(d_matrix).collect();
    let mut u = s.u0.clone();
    let mut slides = vec![false; count];
    // Bolts: each shortened across its section until it pulls with its preload (no loads yet), then held at that length.
    let mut bolt_scale = vec![0.0; study.bolts.len()];
    let mut bolt_results = vec![];
    if !study.bolts.is_empty() {
        let slabs = bolt_slabs(&s.model, study, &s.part_of_name, &s.names)?;
        // A first guess: the strain that pulls a bar of the slab's section with the preload.
        for (b, bolt) in study.bolts.iter().enumerate() {
            let (cubes, area, e) = (&slabs[b].0, slabs[b].1, s.materials[slabs[b].2].e);
            if cubes.is_empty() || area <= 0.0 {
                return Err(format!("bolt {} crosses no part of its bolt at its point", b + 1));
            }
            bolt_scale[b] = bolt.preload / (e * area);
        }
        let mut pre = vec![0.0; 3 * nodes];
        let mut ok = false;
        for _ in 0..8 {
            let eps = bolt_strains(&s.model, study, &slabs, &bolt_scale);
            let rhs = eigen_forces(&s.model, &ds, &eps);
            solve_rhs(&mut s.model, s.pairs.as_ref(), &rhs, &mut pre);
            let tensors = strain_stresses(&s.model, &ds, &pre, &eps);
            let forces: Vec<f64> = (0..study.bolts.len()).map(|b| bolt_force(&s.model, &tensors, study, &slabs, b)).collect();
            ok = forces.iter().zip(&study.bolts).all(|(f, bolt)| (f - bolt.preload).abs() <= 0.005 * bolt.preload.abs());
            if ok {
                break;
            }
            for (b, f) in forces.iter().enumerate() {
                if f.abs() > 1e-9 {
                    bolt_scale[b] *= study.bolts[b].preload / f;
                }
            }
        }
        if !ok {
            notes.push("The bolts' preload did not settle to within 0.5 %: the clamp force is approximate.".into());
        }
        s.bolt_slabs = Some(slabs);
    }
    let bolt_eps = match &s.bolt_slabs {
        Some(slabs) => bolt_strains(&s.model, study, slabs, &bolt_scale),
        None => vec![[0.0; 6]; s.model.cells.len()],
    };
    let mechanical = s.applied > 0.0;
    // Heat and loads apart (the map shows their sum), when nothing ties them together.
    let mut split: Option<(f64, f64)> = None;
    let mut plastic_out: Option<(Vec<[f64; 6]>, Vec<f64>, PlasticSummary)> = None;
    let mut contact_out = None;
    let heat_on = s.eps_heat.iter().any(|e| e[0] != 0.0);
    let base_eps: Vec<[f64; 6]> = s.eps_heat.iter().zip(&bolt_eps).map(|(a, b)| std::array::from_fn(|r| a[r] + b[r])).collect();
    let (iterations, converged, tensors) = if study.plastic {
        let (tensors, its, ok, summary, ep) = plastic_solve(&mut s, &ds, &base_eps, &mut u, &mut notes)?;
        plastic_out = Some((ep, vec![], summary));
        (its, ok, tensors)
    } else if heat_on && mechanical && s.pairs.is_none() && study.bolts.is_empty() {
        let mut ut = vec![0.0; 3 * nodes];
        let (a, oka) = s.model.cg(&s.f, &mut u, 1e-9);
        let fh = eigen_forces(&s.model, &ds, &s.eps_heat);
        let (b, okb) = s.model.cg(&fh, &mut ut, 1e-9);
        let zero = vec![[0.0; 6]; s.eps_heat.len()];
        let sm = strain_stresses(&s.model, &ds, &u, &zero);
        let st = strain_stresses(&s.model, &ds, &ut, &s.eps_heat);
        let peak = |t: &[[f64; 6]]| t.iter().map(von_mises).fold(0.0, f64::max);
        split = Some((peak(&st), peak(&sm)));
        u.iter_mut().zip(&ut).for_each(|(a, b)| *a += b);
        (a + b, oka && okb, sm.iter().zip(&st).map(|(a, b)| std::array::from_fn(|r| a[r] + b[r])).collect())
    } else {
        let fe = eigen_forces(&s.model, &ds, &base_eps);
        let rhs: Vec<f64> = s.f.iter().zip(&fe).map(|(a, b)| a + b).collect();
        let (it, ok, out) = solve_rhs(&mut s.model, s.pairs.as_ref(), &rhs, &mut u);
        contact_out = out;
        (it, ok, strain_stresses(&s.model, &ds, &u, &base_eps))
    };
    let mut applied = s.applied;
    if heat_on {
        applied += eigen_forces(&s.model, &ds, &s.eps_heat).chunks_exact(3).map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).sum::<f64>() / 2.0;
    }
    applied += study.bolts.iter().map(|b| b.preload.abs()).sum::<f64>();
    let contact = match (&s.pairs, &contact_out) {
        (Some(pairs), Some(out)) => {
            slides = contact::slipping(pairs, out, count);
            if !out.settled {
                notes.push(format!("The contacts did not fully settle in {} rounds: pressures are approximate.", out.rounds));
            }
            let report = contact::report(&s.model, pairs, out, &s.names);
            applied = applied.max(report.pairs.iter().map(|p| p.normal_force.abs()).sum::<f64>());
            Some(report)
        }
        _ => None,
    };
    if s.pairs.is_none() && several {
        notes.push("The parts do not touch anywhere: each carries only its own loads.".to_string());
    }
    if !converged {
        notes.push("The solve did not fully settle: is the part held so it cannot spin or slide? Results are approximate.".into());
    }
    if let Some(slabs) = &s.bolt_slabs {
        for (b, bolt) in study.bolts.iter().enumerate() {
            let force = bolt_force(&s.model, &tensors, study, slabs, b);
            bolt_results.push(BoltResult { part: bolt.part.clone().unwrap_or_else(|| s.names[0].clone()), preload: bolt.preload, force, shortening: bolt_scale[b] * 2.0 * h });
        }
    }
    let stresses: Vec<f64> = tensors.iter().map(von_mises).collect();
    let model = &s.model;
    let free: Vec<bool> = (0..count).map(|p| model.floating[p].is_some() && (slides[p] || model.tie_pull(p, &u) > DRIFT * applied.max(1e-12))).collect();
    for p in (0..count).filter(|&p| free[p]) {
        notes.push(format!("“{}” is held by no support and its contacts do not hold it: it slides or lifts away freely, so how far it moves means nothing. Hold it, or glue it with a bonded contact.", s.names[p]));
    }
    let mut part_stress = vec![0.0f64; count];
    for (st, &p) in stresses.iter().zip(&model.part) {
        part_stress[p] = part_stress[p].max(*st);
    }
    let mut part_move = vec![0.0f64; count];
    let mut displacements = vec![];
    let mut node_parts = vec![];
    let mut farthest = (0.0, [0.0; 3]);
    for n in (0..nodes).filter(|&n| model.used[n]) {
        let p = block_of(&model.blocks, n);
        let v = [u[3 * n], u[3 * n + 1], u[3 * n + 2]];
        let size = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        part_move[p] = part_move[p].max(size);
        if free[p] {
            continue; // drifting: drawn where it is
        }
        if size > farthest.0 {
            farthest = (size, model.position(n));
        }
        displacements.push((model.node_at(n).map(|c| c as u32), v.map(|c| c as f32)));
        if several {
            node_parts.push(p as u8);
        }
    }
    let mut max_stress = stresses.iter().cloned().fold(0.0, f64::max);
    let hottest = (0..stresses.len()).max_by(|&a, &b| stresses[a].total_cmp(&stresses[b])).unwrap_or(0);
    let mut max_stress_at = cell_centre(&model.grid, model.cells[hottest]);
    let max_displacement = (0..count).filter(|&p| !free[p]).map(|p| part_move[p]).fold(0.0, f64::max);
    let heat = s.warmth.as_ref().map(|w| {
        let (lo, hi) = s.dt.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &t| (a.min(t), b.max(t)));
        notes.extend(w.notes(several));
        let alone = (!mechanical && contact.is_none()).then_some(max_stress);
        HeatSummary { reference: w.reference, min: w.reference + lo, max: w.reference + hi, thermal_stress: split.map(|x| x.0).or(alone), mechanical_stress: split.map(|x| x.1) }
    });
    // Reactions: what each hold pushes back with (K·u − f at its held directions, or its springs).
    let fe = eigen_forces(model, &ds, &tensors_eps(&base_eps, plastic_out.as_ref().map(|p| &p.0)));
    let ku = model.apply_raw(&u);
    let reactions = s
        .holds
        .iter()
        .map(|g| {
            let mut force = [0.0; 3];
            let mut moment = [0.0; 3];
            let c = g.centre;
            let mut add = |n: usize, v: [f64; 3]| {
                let r = v_sub(model.position(n), c);
                let m = v_cross(r, v);
                for d in 0..3 {
                    force[d] += v[d];
                    moment[d] += m[d];
                }
            };
            for &n in &g.nodes {
                let v: [f64; 3] = std::array::from_fn(|d| if model.fixed[3 * n + d] { ku[3 * n + d] - s.f[3 * n + d] - fe[3 * n + d] } else { 0.0 });
                add(n, v);
            }
            for sp in &g.springs {
                let stretch: f64 = sp.g.iter().map(|&(i, c)| c * u[i]).sum();
                // A spring along one node's direction: its pull back on that node.
                if let Some(&(i, _)) = sp.g.first() {
                    let mut v = [0.0; 3];
                    for &(j, cj) in &sp.g {
                        v[j % 3] -= sp.k * stretch * cj;
                    }
                    add(i / 3, v);
                }
            }
            Reaction { name: g.name.clone(), force: force.map(round6), moment: moment.map(round6), at: c }
        })
        .collect::<Vec<_>>();
    // The most stressed spot, re-solved finer (one part, plain supports and face loads: its cut is held by the coarse solution).
    let mut peak = None;
    let simple = study.bolts.is_empty() && !study.plastic && study.sliding.is_empty() && study.displacements.is_empty() && study.elastic_supports.is_empty() && study.bearing_loads.is_empty() && study.remote_loads.is_empty() && study.gravity.is_none() && study.spin.is_none();
    if study.refine && !several && !simple {
        notes.push("Peak sharpening is for studies held by plain supports and loaded on faces: this one reads its stress at the cube centres.".into());
    }
    if study.refine && !several && simple {
        let warm = |x: [f64; 3]| s.warmth.as_ref().map_or(0.0, |w| w.at(0, x));
        let ctx = refine::Context { positions: bodies[0].positions, triangles: bodies[0].triangles, material: s.materials[0], study, warm: &warm };
        match refine::sharpen(model, &u, &s.pointed, &stresses, &ctx) {
            Ok((p, more)) => {
                notes.extend(more);
                // A higher peak always counts. A lower one replaces the largest only when it is the
                // same spot re-read (the coarse cube there was the largest) and it has settled: the
                // staircase cube at a curved edge reads high, the surface re-solve does not.
                let same_spot_settled = !p.at_support && p.change <= 0.1 && p.coarse >= max_stress - 1e-9;
                if !p.singular && (p.stress > max_stress || same_spot_settled) {
                    part_stress[0] = p.stress;
                    max_stress = p.stress;
                    max_stress_at = p.at;
                }
                peak = Some(p);
            }
            Err(why) => notes.push(why),
        }
    }
    let materials = &s.materials;
    let safety_of = |p: usize| if part_stress[p] > 0.0 { materials[p].yield_strength / part_stress[p] } else { f64::INFINITY };
    // The part nearest to yielding sets the study's safety.
    let worst = (0..count).min_by(|&a, &b| safety_of(a).total_cmp(&safety_of(b))).unwrap_or(0);
    if peak.is_none() {
        notes.push(format!("{} cubes of {} mm; stress is read at each cube's centre, so a sharp inside corner shows lower than its true peak.", model.cells.len(), crate::drawing::length_text(h)));
    } else {
        notes.insert(0, format!("{} cubes of {} mm.", model.cells.len(), crate::drawing::length_text(h)));
    }
    let (plastic, plastic_strain) = match plastic_out {
        Some((ep, _, summary)) => (Some(summary), ep.iter().map(|e| equivalent_strain(e) as f32).collect()),
        None => (None, vec![]),
    };
    Ok(Result_ {
        cell: h,
        origin: model.grid.origin,
        counts: model.grid.n,
        cells: model.cells.iter().zip(&stresses).map(|(c, st)| (c.map(|v| v as u32), *st as f32)).collect(),
        displacements,
        max_stress,
        max_displacement,
        yield_strength: materials[worst].yield_strength,
        safety: safety_of(worst),
        elements: model.cells.len(),
        iterations,
        notes,
        parts: if several {
            (0..count).map(|p| PartResult { name: s.names[p].clone(), max_stress: part_stress[p], max_displacement: part_move[p], yield_strength: materials[p].yield_strength, safety: safety_of(p), floating: free[p] }).collect()
        } else {
            vec![]
        },
        cell_parts: if several { model.part.iter().map(|&p| p as u8).collect() } else { vec![] },
        node_parts,
        contact,
        heat,
        peak,
        max_stress_at,
        max_displacement_at: farthest.1,
        reactions,
        bolts: bolt_results,
        plastic,
        plastic_strain,
        tensors: tensors.iter().map(|t| t.map(|v| v as f32)).collect(),
        yields: materials.iter().map(|m| m.yield_strength).collect(),
    })
}

fn round6(v: f64) -> f64 {
    if v.abs() < 1e-9 { 0.0 } else { v }
}

/// The strains a solution's stresses are taken from (heat and bolts, and
/// the plastic strain when the material yielded).
fn tensors_eps(base: &[[f64; 6]], plastic: Option<&Vec<[f64; 6]>>) -> Vec<[f64; 6]> {
    match plastic {
        Some(ep) => base.iter().zip(ep).map(|(a, b)| std::array::from_fn(|r| a[r] + b[r])).collect(),
        None => base.to_vec(),
    }
}

/// What a hold pushes back with.
#[derive(Clone, Debug, Serialize)]
pub struct Reaction {
    /// "support 1", "sliding 1", "displacement 1", "elastic 1", "bore 1".
    pub name: String,
    /// N, on the part; and N·mm about `at` (the middle of the held face).
    pub force: [f64; 3],
    pub moment: [f64; 3],
    pub at: [f64; 3],
}

/// A pretensioned bolt after the loads came on.
#[derive(Clone, Debug, Serialize)]
pub struct BoltResult {
    pub part: String,
    /// N: what it was pulled tight with, and what it pulls with now.
    pub preload: f64,
    pub force: f64,
    /// How much its section was shortened to get the preload (mm).
    pub shortening: f64,
}

/// What yielding did.
#[derive(Clone, Debug, Serialize)]
pub struct PlasticSummary {
    /// Largest equivalent plastic strain (mm/mm) and where.
    pub max_strain: f64,
    pub at: [f64; 3],
    /// Cubes that yielded, of all.
    pub yielded: usize,
    pub steps: usize,
    pub settled: bool,
    /// The material's tangent modulus used (MPa).
    pub tangent: f64,
}

/// A hold's nodes, for its reaction.
struct HoldGroup {
    name: String,
    nodes: Vec<usize>,
    springs: Vec<Spring>,
    centre: [f64; 3],
}

/// A study set up on its cubes: the model with every hold, the loads on its
/// nodes, and the strains heat puts in each cube.
struct Setup {
    model: Model,
    pairs: Option<contact::Found>,
    materials: Vec<Elastic>,
    names: Vec<String>,
    part_of_name: Vec<Option<String>>,
    /// External forces on the nodes, and the prescribed movements (on held directions).
    f: Vec<f64>,
    u0: Vec<f64>,
    pointed: Vec<bool>,
    applied: f64,
    holds: Vec<HoldGroup>,
    warmth: Option<Warmth>,
    dt: Vec<f64>,
    eps_heat: Vec<[f64; 6]>,
    bolt_slabs: Option<Vec<(Vec<usize>, f64, usize)>>,
    notes: Vec<String>,
}

impl Setup {
    /// `need_load`: a static study needs something loading it; vibration does not.
    fn build(bodies: &[Body], study: &Study, need_load: bool) -> Result<Setup, String> {
        if bodies.is_empty() {
            return Err("nothing to study".into());
        }
        let press_fit = bodies.len() > 1 && study.contacts.iter().any(|c| c.press_fit);
        if need_load && !study.loaded() && study.heat.is_none() && !press_fit {
            return Err("add a load: a face and the force on it".into());
        }
        if !study.held() {
            return Err("add a support: a face the part is held by".into());
        }
        let several = bodies.len() > 1;
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for b in bodies {
            let (l, u) = bounds(b.positions, b.triangles);
            for d in 0..3 {
                lo[d] = lo[d].min(l[d]);
                hi[d] = hi[d].max(u[d]);
            }
        }
        let grid = grid_around(lo, hi, study.resolution.clamp(8, 160));
        let fills: Vec<Vec<f32>> = bodies.par_iter().map(|b| fill_of(&grid, b.positions, b.triangles)).collect();
        let names: Vec<String> = bodies.iter().map(|b| b.name.clone()).collect();
        let materials: Vec<Elastic> = bodies.iter().map(|b| b.material).collect();
        let part_of = |what: String, name: &Option<String>| -> Result<usize, String> {
            match name {
                None => Ok(0),
                Some(n) => names.iter().position(|m| m == n).ok_or_else(|| format!("{what} is on “{n}”, which is not in the study")),
            }
        };
        let supports = study.supports.iter().enumerate().map(|(i, f)| Ok((part_of(format!("support {}", i + 1), &f.part)?, f))).collect::<Result<Vec<_>, String>>()?;
        let bores: Vec<(usize, &Bore)> = study.bore_supports.iter().map(|b| (0, b)).collect();
        let shown: Vec<String> = if several { names.clone() } else { vec!["the part".into()] };
        let mut model = Model::assemble(grid, &fills, &materials, &shown, &supports, &bores, true)?;
        let h = model.grid.h;
        let mut notes = vec![];
        let mut holds = vec![];
        let centre = |m: &Model, on: &[usize]| -> [f64; 3] {
            let mut c = [0.0; 3];
            for &n in on {
                let p = m.position(n);
                (0..3).for_each(|d| c[d] += p[d] / on.len().max(1) as f64);
            }
            c
        };
        for (i, &(p, face)) in supports.iter().enumerate() {
            let on = nodes_on_face(model.part_nodes(p), |id| model.position(id), h, face);
            holds.push(HoldGroup { name: format!("support {}", i + 1), centre: centre(&model, &on), nodes: on, springs: vec![] });
        }
        for (i, &(p, bore)) in bores.iter().enumerate() {
            let on = nodes_near_axis(model.part_nodes(p), |id| model.position(id), h, bore);
            holds.push(HoldGroup { name: format!("bore {}", i + 1), centre: centre(&model, &on), nodes: on, springs: vec![] });
        }
        // Stiff enough to hold, soft enough to solve: a thousand times a cube's own stiffness per node.
        let stiff = |m: &Model, p: usize| 1e3 * (0..24).map(|r| m.kes[p][r * 25]).sum::<f64>() / 24.0;
        let mut slanted = false;
        for (i, face) in study.sliding.iter().enumerate() {
            let p = part_of(format!("sliding face {}", i + 1), &face.part)?;
            let on = nodes_on_face(model.part_nodes(p), |id| model.position(id), h, face);
            if on.is_empty() {
                return Err(format!("sliding face {} touches no part of the part at this resolution", i + 1));
            }
            let mut springs = vec![];
            let normals: Vec<[f64; 3]> = if face.curved {
                // Each node square to the bit of surface nearest it.
                let per: HashMap<usize, [f64; 3]> = area_on_nodes(&on, |id| model.position(id), h, face, |_| true).into_iter().map(|(n, _, av)| (n, av.map(|c| c / v_norm(av).max(1e-300)))).collect();
                on.iter().map(|n| per.get(n).copied().unwrap_or(face.normal)).collect()
            } else {
                let n = face.normal;
                vec![n.map(|c| c / v_norm(n).max(1e-12)); on.len()]
            };
            let k = stiff(&model, p);
            for (&node, nrm) in on.iter().zip(&normals) {
                match (0..3).find(|&d| nrm[d].abs() > 0.9999) {
                    Some(d) => model.fixed[3 * node + d] = true,
                    None => {
                        slanted = true;
                        springs.push(Spring { k, g: (0..3).map(|d| (3 * node + d, nrm[d])).collect() });
                    }
                }
            }
            model.base.extend(springs.iter().cloned());
            holds.push(HoldGroup { name: format!("sliding {}", i + 1), centre: centre(&model, &on), nodes: on, springs });
        }
        if slanted {
            notes.push("Sliding faces at a slant (or round) are held by stiff springs square to them: they give about 0.1 % of a cube's movement.".into());
        }
        let mut u0 = vec![0.0; 3 * model.nodes];
        for (i, disp) in study.displacements.iter().enumerate() {
            let p = part_of(format!("displacement {}", i + 1), &disp.face.part)?;
            let on = nodes_on_face(model.part_nodes(p), |id| model.position(id), h, &disp.face);
            if on.is_empty() {
                return Err(format!("displaced face {} touches no part of the part at this resolution", i + 1));
            }
            for &n in &on {
                for d in 0..3 {
                    if let Some(v) = disp.by[d] {
                        model.fixed[3 * n + d] = true;
                        u0[3 * n + d] = v;
                    }
                }
            }
            holds.push(HoldGroup { name: format!("displacement {}", i + 1), centre: centre(&model, &on), nodes: on, springs: vec![] });
        }
        let share = node_shares(model.nodes, &model.corners, &model.weight);
        for (i, es) in study.elastic_supports.iter().enumerate() {
            let p = part_of(format!("elastic support {}", i + 1), &es.face.part)?;
            let on = nodes_on_face(model.part_nodes(p), |id| model.position(id), h, &es.face);
            if on.is_empty() {
                return Err(format!("elastic support {} touches no part of the part at this resolution", i + 1));
            }
            // Each node's share of the face's area and the way it faces.
            let pieces: Vec<(usize, f64, [f64; 3])> = if es.face.curved {
                area_on_nodes(&on, |id| model.position(id), h, &es.face, |_| true).into_iter().map(|(n, a, av)| (n, a, av.map(|c| c / v_norm(av).max(1e-300)))).collect()
            } else {
                let n = es.face.normal;
                let n = n.map(|c| c / v_norm(n).max(1e-12));
                spread(&on, &share, es.face.area()).into_iter().map(|(node, a)| (node, a, n)).collect()
            };
            let mut springs = vec![];
            for (node, a, n) in pieces {
                let k = es.stiffness * a;
                if es.normal_only {
                    springs.push(Spring { k, g: (0..3).map(|d| (3 * node + d, n[d])).collect() });
                } else {
                    springs.extend((0..3).map(|d| Spring { k, g: vec![(3 * node + d, 1.0)] }));
                }
            }
            model.base.extend(springs.iter().cloned());
            holds.push(HoldGroup { name: format!("elastic {}", i + 1), centre: centre(&model, &on), nodes: on, springs });
        }
        let meshes: Vec<(&[f32], &[u32])> = bodies.iter().map(|b| (b.positions, b.triangles)).collect();
        let pairs = several.then(|| contact::find(&model, &fills, &materials, &names, &study.contacts, &meshes));
        let anchored = match &pairs {
            Some(found) => found.anchored(model.held()),
            None => model.held(),
        };
        model.finish(&anchored.iter().map(|a| !a).collect::<Vec<_>>());
        let nodes = model.nodes;
        let mut f = vec![0.0; 3 * nodes];
        // Where loads make a sharp peak of their own (every load but a pressure
        // all over a curved face, which ends nowhere): the re-solve keeps off them.
        let mut pointed = vec![false; nodes];
        let share = node_shares(nodes, &model.corners, &model.weight);
        let mut applied = 0.0;
        let free_on = |m: &Model, p: usize, face: &Face| -> Vec<usize> { nodes_on_face(m.part_nodes(p), |id| m.position(id), h, face).into_iter().filter(|&n| !(0..3).all(|d| m.fixed[3 * n + d])).collect() };
        for (l, load) in study.loads.iter().enumerate() {
            let p = part_of(format!("load {}", l + 1), &load.face.part)?;
            let on = free_on(&model, p, &load.face);
            if on.is_empty() {
                return Err(format!("load {} touches no free part of the part", l + 1));
            }
            let smooth = load.face.curved && load.force == [0.0; 3];
            for (n, v) in load_forces(&on, |id| model.position(id), h, &share, load, |_| true) {
                for d in 0..3 {
                    f[3 * n + d] += v[d];
                }
                pointed[n] |= !smooth;
            }
            applied += load_size(load);
        }
        for (l, load) in study.bore_loads.iter().enumerate() {
            let on: Vec<usize> = nodes_near_axis(model.part_nodes(0), |id| model.position(id), h, &load.bore).into_iter().filter(|&n| !model.fixed[3 * n]).collect();
            if on.is_empty() {
                return Err(format!("bore load {} touches no free part of the part", l + 1));
            }
            for d in 0..3 {
                for (n, v) in spread(&on, &share, load.force[d]) {
                    f[3 * n + d] += v;
                }
            }
            on.iter().for_each(|&n| pointed[n] = true);
            applied += v_norm(load.force);
        }
        for (l, load) in study.bearing_loads.iter().enumerate() {
            let p = part_of(format!("bearing load {}", l + 1), &load.face.part)?;
            let on = free_on(&model, p, &load.face);
            let size = v_norm(load.force);
            if on.is_empty() || size <= 0.0 {
                return Err(format!("bearing load {} touches no free part of the part", l + 1));
            }
            for (n, v) in bearing_forces(&on, |id| model.position(id), h, &load.face, load.force)? {
                (0..3).for_each(|d| f[3 * n + d] += v[d]);
                pointed[n] = true;
            }
            applied += size;
        }
        for (l, load) in study.remote_loads.iter().enumerate() {
            let p = part_of(format!("remote load {}", l + 1), &load.face.part)?;
            let on = free_on(&model, p, &load.face);
            if on.is_empty() {
                return Err(format!("remote load {} touches no free part of the part", l + 1));
            }
            for (n, v) in remote_forces(&on, |id| model.position(id), &share, load) {
                (0..3).for_each(|d| f[3 * n + d] += v[d]);
                pointed[n] = true;
            }
            applied += v_norm(load.force) + v_norm(load.moment) / (h * 10.0);
        }
        // Gravity and spinning: each cube's mass pulled, shared by its corners.
        if study.gravity.is_some() || study.spin.is_some() {
            let g = study.gravity.unwrap_or([0.0; 3]);
            let spin = study.spin.map(|sp| {
                let a = sp.axis.map(|c| c / v_norm(sp.axis).max(1e-12));
                let w = sp.rpm * std::f64::consts::TAU / 60.0;
                (sp.at, a, w * w)
            });
            for (e, (c, &p)) in model.cells.iter().zip(&model.part).enumerate() {
                let mass = materials[p].density * 1e-9 * h * h * h * model.weight[e];
                let x = cell_centre(&model.grid, *c);
                let mut acc = g;
                if let Some((at, a, w2)) = spin {
                    let r = v_sub(x, at);
                    let along = v_dot(r, a);
                    // ω²·r outward, r in metres.
                    (0..3).for_each(|d| acc[d] += w2 * (r[d] - along * a[d]) * 1e-3);
                }
                for &n in &model.corners[e] {
                    (0..3).for_each(|d| f[3 * n + d] += mass * acc[d] / 8.0);
                }
                applied += mass * v_norm(acc);
            }
            if materials.iter().any(|m| m.density <= 0.0) {
                notes.push("A part has no density: gravity and spinning do not pull on it.".into());
            }
        }
        // Prescribed movements load the part too.
        if u0.iter().any(|v| *v != 0.0) {
            applied += model.apply_raw(&u0).iter().enumerate().filter(|(i, _)| model.free(*i)).map(|(_, v)| v.abs()).sum::<f64>() / 2.0;
        }
        if pairs.as_ref().is_some_and(|p| p.overlap) {
            notes.push("Some parts overlap rather than touch: they are taken as just touching (no squeeze). Mark the contact a press fit to squeeze the overlap out.".to_string());
        }
        // Heat: each cube's warming above the stress-free temperature, as a strain.
        let warmth = study.heat.as_ref().map(Warmth::new);
        let dt: Vec<f64> = match &warmth {
            Some(w) => model.cells.iter().zip(&model.part).map(|(&c, &p)| w.at(p, cell_centre(&model.grid, c))).collect(),
            None => vec![0.0; model.cells.len()],
        };
        let eps_heat: Vec<[f64; 6]> = dt.iter().zip(&model.part).map(|(&t, &p)| {
            let e = materials[p].alpha * t;
            [e, e, e, 0.0, 0.0, 0.0]
        }).collect();
        let part_of_name = study.bolts.iter().map(|b| b.part.clone()).collect();
        let pairs = pairs.filter(|p| !p.is_empty());
        Ok(Setup { model, pairs, materials, names, part_of_name, f, u0, pointed, applied, holds, warmth, dt, eps_heat, bolt_slabs: None, notes })
    }
}

/// Solves K·u = rhs from `u`, with contact when there is any: the
/// iterations, whether it settled, and how the contacts ended up.
fn solve_rhs(model: &mut Model, pairs: Option<&contact::Found>, rhs: &[f64], u: &mut Vec<f64>) -> (usize, bool, Option<contact::Outcome>) {
    match pairs {
        Some(pairs) => {
            let out = contact::solve(model, rhs, u, pairs);
            (out.iterations, out.converged, Some(out))
        }
        None => {
            let (it, ok) = model.cg(rhs, u, 1e-9);
            (it, ok, None)
        }
    }
}

/// A bearing's push on a round face: over the half it pushes into, each
/// bit pressed straight in, as much as it faces the force (a cosine), all
/// adding up to the force.
fn bearing_forces(on: &[usize], position: impl Fn(usize) -> [f64; 3], h: f64, face: &Face, force: [f64; 3]) -> Result<Vec<(usize, [f64; 3])>, String> {
    let size = v_norm(force);
    let dir = force.map(|c| c / size);
    let pieces: Vec<(usize, f64, [f64; 3])> = if face.curved {
        area_on_nodes(on, &position, h, face, |_| true).into_iter().map(|(n, a, av)| (n, a, av.map(|c| c / v_norm(av).max(1e-300)))).collect()
    } else {
        let n = face.normal.map(|c| c / v_norm(face.normal).max(1e-12));
        on.iter().map(|&id| (id, 1.0, n)).collect()
    };
    // Pressed where the face looks against the force; pushed along its inward normal.
    let weighted: Vec<(usize, f64, [f64; 3])> = pieces.into_iter().map(|(n, a, nrm)| (n, a * (-v_dot(nrm, dir)).max(0.0), nrm.map(|c| -c))).filter(|(_, w, _)| *w > 0.0).collect();
    let along: f64 = weighted.iter().map(|(_, w, inward)| w * v_dot(*inward, dir)).sum();
    if along <= 0.0 {
        return Err("a bearing load needs a round face that faces against its force".into());
    }
    let s = size / along;
    Ok(weighted.into_iter().map(|(n, w, inward)| (n, inward.map(|c| c * s * w))).collect())
}

/// A force and moment at a point, carried to a face's nodes as by a rigid
/// link: the force shared by the nodes' shares, the moment (about their
/// middle, with the force's own lever) as forces growing with distance.
fn remote_forces(on: &[usize], position: impl Fn(usize) -> [f64; 3], share: &[f64], load: &RemoteLoad) -> Vec<(usize, [f64; 3])> {
    let w: Vec<f64> = on.iter().map(|&n| share[n].max(1e-12)).collect();
    let total: f64 = w.iter().sum();
    let mut c = [0.0; 3];
    for (&n, wi) in on.iter().zip(&w) {
        let p = position(n);
        (0..3).for_each(|d| c[d] += wi * p[d] / total);
    }
    let lever = v_cross(v_sub(load.point, c), load.force);
    let m: [f64; 3] = std::array::from_fn(|d| load.moment[d] + lever[d]);
    // J = Σ w (|r|² I − r rᵀ); the nodes' forces w·(J⁻¹M × r) add up to M.
    let mut j = [0.0; 9];
    for (&n, wi) in on.iter().zip(&w) {
        let r = v_sub(position(n), c);
        let rr = v_dot(r, r);
        for a in 0..3 {
            for b in 0..3 {
                j[3 * a + b] += wi * (if a == b { rr } else { 0.0 } - r[a] * r[b]);
            }
        }
    }
    // Nodes in a line cannot take a moment about it: a little stiffness there keeps J invertible.
    let trace = j[0] + j[4] + j[8];
    (0..3).for_each(|a| j[4 * a] += 1e-9 * trace.max(1e-300));
    let inv = invert(&j, 3);
    let alpha: [f64; 3] = std::array::from_fn(|a| (0..3).map(|b| inv[3 * a + b] * m[b]).sum());
    on.iter()
        .zip(&w)
        .map(|(&n, wi)| {
            let r = v_sub(position(n), c);
            let turn = v_cross(alpha, r);
            (n, std::array::from_fn(|d| load.force[d] * wi / total + wi * turn[d]))
        })
        .collect()
}

/// Each bolt's slab: the cubes of its part whose centres lie within a cube of
/// its point along its axis, the slab's section area (mm²), and the part.
fn bolt_slabs(model: &Model, study: &Study, parts: &[Option<String>], names: &[String]) -> Result<Vec<(Vec<usize>, f64, usize)>, String> {
    let h = model.grid.h;
    study
        .bolts
        .iter()
        .zip(parts)
        .enumerate()
        .map(|(b, (bolt, part))| {
            let p = match part {
                None => 0,
                Some(n) => names.iter().position(|m| m == n).ok_or_else(|| format!("bolt {} is on “{n}”, which is not in the study", b + 1))?,
            };
            let a = bolt.axis.map(|c| c / v_norm(bolt.axis).max(1e-12));
            let cubes: Vec<usize> = (0..model.cells.len()).filter(|&e| model.part[e] == p && v_dot(v_sub(cell_centre(&model.grid, model.cells[e]), bolt.at), a).abs() < h).collect();
            let volume: f64 = cubes.iter().map(|&e| model.weight[e] * h * h * h).sum();
            Ok((cubes, volume / (2.0 * h), p))
        })
        .collect()
}

/// The shortening strain of each bolt's slab, `scale` along its axis.
fn bolt_strains(model: &Model, study: &Study, slabs: &[(Vec<usize>, f64, usize)], scale: &[f64]) -> Vec<[f64; 6]> {
    let mut eps = vec![[0.0; 6]; model.cells.len()];
    for ((bolt, (cubes, _, _)), &s) in study.bolts.iter().zip(slabs).zip(scale) {
        let a = bolt.axis.map(|c| c / v_norm(bolt.axis).max(1e-12));
        // Shorten along the axis: −s·a aᵀ (engineering shears twice the tensor's).
        let e = [-s * a[0] * a[0], -s * a[1] * a[1], -s * a[2] * a[2], -2.0 * s * a[0] * a[1], -2.0 * s * a[1] * a[2], -2.0 * s * a[2] * a[0]];
        for &c in cubes {
            (0..6).for_each(|r| eps[c][r] += e[r]);
        }
    }
    eps
}

/// The pull through a bolt's section (N): the axial stress over its slab's volume, per its thickness.
fn bolt_force(model: &Model, tensors: &[[f64; 6]], study: &Study, slabs: &[(Vec<usize>, f64, usize)], b: usize) -> f64 {
    let h = model.grid.h;
    let a = study.bolts[b].axis.map(|c| c / v_norm(study.bolts[b].axis).max(1e-12));
    slabs[b]
        .0
        .iter()
        .map(|&e| {
            let s = &tensors[e];
            let axial = s[0] * a[0] * a[0] + s[1] * a[1] * a[1] + s[2] * a[2] * a[2] + 2.0 * (s[3] * a[0] * a[1] + s[4] * a[1] * a[2] + s[5] * a[2] * a[0]);
            axial * model.weight[e] * h * h * h
        })
        .sum::<f64>()
        / (2.0 * h)
}

/// The stress at each cube's centre from the movement, less a strain the
/// cube takes freely (heat, a bolt's shortening, plastic flow): σ = D·(B(0)·uₑ − ε₀).
fn strain_stresses(model: &Model, ds: &[[[f64; 6]; 6]], u: &[f64], eps0: &[[f64; 6]]) -> Vec<[f64; 6]> {
    let b0 = b_matrix([0.0; 3], model.grid.h);
    model
        .corners
        .par_iter()
        .zip(&model.part)
        .zip(eps0)
        .map(|((c, &p), e0)| {
            let d = &ds[p];
            let mut ue = [0.0; 24];
            for (a, &n) in c.iter().enumerate() {
                ue[3 * a..3 * a + 3].copy_from_slice(&u[3 * n..3 * n + 3]);
            }
            let eps: [f64; 6] = std::array::from_fn(|r| (0..24).map(|j| b0[r][j] * ue[j]).sum::<f64>() - e0[r]);
            std::array::from_fn(|r| (0..6).map(|k| d[r][k] * eps[k]).sum())
        })
        .collect()
}

/// The forces a strain each cube would take freely puts on the nodes when
/// it cannot: w·h³·B(0)ᵀ·D·ε₀ (exact for a strain constant over the cube).
fn eigen_forces(model: &Model, ds: &[[[f64; 6]; 6]], eps0: &[[f64; 6]]) -> Vec<f64> {
    let h = model.grid.h;
    let b0 = b_matrix([0.0; 3], h);
    let mut f = vec![0.0; 3 * model.nodes];
    for (((c, &w), &p), e0) in model.corners.iter().zip(&model.weight).zip(&model.part).zip(eps0) {
        if e0.iter().all(|v| *v == 0.0) {
            continue;
        }
        let d = &ds[p];
        let sig: [f64; 6] = std::array::from_fn(|r| (0..6).map(|k| d[r][k] * e0[k]).sum::<f64>() * w * h * h * h);
        for (a, &n) in c.iter().enumerate() {
            for dd in 0..3 {
                f[3 * n + dd] += (0..6).map(|r| b0[r][3 * a + dd] * sig[r]).sum::<f64>();
            }
        }
    }
    f
}

/// Equivalent (von Mises) strain of a plastic strain (engineering shears).
fn equivalent_strain(e: &[f64; 6]) -> f64 {
    let (a, b, c) = (e[0], e[1], e[2]);
    let (x, y, z) = (e[3] / 2.0, e[4] / 2.0, e[5] / 2.0);
    (2.0 / 3.0 * (a * a + b * b + c * c + 2.0 * (x * x + y * y + z * z))).sqrt()
}

/// Radial return for von Mises with linear hardening: the plastic strain
/// a trial stress makes (engineering shears) and the hardening it adds, or
/// None when it stays elastic.
fn return_map(trial: &[f64; 6], m: &Elastic, hardened: f64) -> Option<([f64; 6], f64)> {
    let p = (trial[0] + trial[1] + trial[2]) / 3.0;
    let s = [trial[0] - p, trial[1] - p, trial[2] - p, trial[3], trial[4], trial[5]];
    let ss = s[0] * s[0] + s[1] * s[1] + s[2] * s[2] + 2.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5]);
    let q = (1.5 * ss).sqrt();
    let hmod = plastic_modulus(m);
    let over = q - (m.yield_strength + hmod * hardened);
    if over <= 0.0 || q <= 0.0 {
        return None;
    }
    let g = m.e / (2.0 * (1.0 + m.nu));
    let dgamma = over / (3.0 * g + hmod);
    let k = 1.5 * dgamma / q;
    Some(([k * s[0], k * s[1], k * s[2], 2.0 * k * s[3], 2.0 * k * s[4], 2.0 * k * s[5]], dgamma))
}

/// Hardening per unit plastic strain from the tangent modulus: H = E·Et/(E − Et).
fn plastic_modulus(m: &Elastic) -> f64 {
    if m.tangent <= 0.0 { 0.0 } else { m.e * m.tangent / (m.e - m.tangent).max(1e-9 * m.e) }
}

/// The elastoplastic tangent of von Mises with linear hardening at a
/// stress on the yield surface: D − (D·n)(n·D)/(n:D:n + H), with
/// n = 3s/2q, which is D − 4G²·n nᵀ/(3G + H) (engineering shears).
fn tangent_d(m: &Elastic, stress: &[f64; 6]) -> [[f64; 6]; 6] {
    let mut d = d_matrix(m);
    let p = (stress[0] + stress[1] + stress[2]) / 3.0;
    let s = [stress[0] - p, stress[1] - p, stress[2] - p, stress[3], stress[4], stress[5]];
    let q = (1.5 * (s[0] * s[0] + s[1] * s[1] + s[2] * s[2] + 2.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5]))).sqrt();
    if q <= 0.0 {
        return d;
    }
    let g = m.e / (2.0 * (1.0 + m.nu));
    let a: [f64; 6] = std::array::from_fn(|r| 2.0 * g * 1.5 * s[r] / q);
    let c = 1.0 / (3.0 * g + plastic_modulus(m));
    for i in 0..6 {
        for j in 0..6 {
            d[i][j] -= c * a[i] * a[j];
        }
    }
    d
}

/// A yielding cube's tangent stiffness, consistent with how its plastic
/// strain acts (one strain over the cube, read at its centre): the elastic
/// cube with the centre's stiffness changed, Kₑ + h³·B(0)ᵀ·(D_ep − D)·B(0).
/// (The cube's bending modes stay elastic, as they do in the out-of-balance force.)
fn plastic_stiffness(ke: &[f64], d: &[[f64; 6]; 6], dep: &[[f64; 6]; 6], h: f64) -> Box<[f64]> {
    let b0 = b_matrix([0.0; 3], h);
    let dd: [[f64; 6]; 6] = std::array::from_fn(|i| std::array::from_fn(|j| dep[i][j] - d[i][j]));
    let db: Vec<[f64; 24]> = (0..6).map(|r| std::array::from_fn(|j| (0..6).map(|k| dd[r][k] * b0[k][j]).sum())).collect();
    let v = h * h * h;
    let mut out = ke.to_vec();
    for i in 0..24 {
        for j in 0..24 {
            out[i * 24 + j] += v * (0..6).map(|r| b0[r][i] * db[r][j]).sum::<f64>();
        }
    }
    out.into_boxed_slice()
}

/// Bilinear plasticity: the loads in steps, each found by Newton's method.
/// The cubes' plastic strain is a strain they take freely (as heat is): the
/// out-of-balance force is f + F(ε₀ + εₚ) − K·u, with εₚ the radial return
/// of each cube's trial stress from the strain at the start of the step,
/// and each correction is solved with the cubes that yield on their
/// elastoplastic tangent. One part, without contact.
fn plastic_solve(s: &mut Setup, ds: &[[[f64; 6]; 6]], base: &[[f64; 6]], u: &mut Vec<f64>, notes: &mut Vec<String>) -> Result<(Vec<[f64; 6]>, usize, bool, PlasticSummary, Vec<[f64; 6]>), String> {
    const STEPS: usize = 10;
    const ROUNDS: usize = 40;
    if s.pairs.is_some() {
        return Err("plasticity is for one part on its own: leave out the touching parts".into());
    }
    let cells = s.model.cells.len();
    let h = s.model.grid.h;
    let mut ep = vec![[0.0; 6]; cells];
    let mut hardened = vec![0.0; cells];
    let mut iterations = 0;
    let mut settled = true;
    let mut tensors = vec![[0.0; 6]; cells];
    let fixed_dofs: Vec<usize> = (0..u.len()).filter(|&i| s.model.used[i / 3] && s.model.fixed[i]).collect();
    let scale = s.f.iter().map(|v| v.abs()).sum::<f64>() + eigen_forces(&s.model, ds, base).iter().map(|v| v.abs()).sum::<f64>() + s.model.apply_with(&s.u0, false).iter().map(|v| v.abs()).sum::<f64>();
    for step in 1..=STEPS {
        let frac = step as f64 / STEPS as f64;
        for &i in &fixed_dofs {
            u[i] = s.u0[i] * frac;
        }
        let mut trial_ep = ep.clone();
        let mut new_hard = hardened.clone();
        let mut yielding = vec![false; cells];
        let mut done = false;
        for _ in 0..ROUNDS {
            // Each cube's stress from the movement, returned to the yield surface.
            let committed: Vec<[f64; 6]> = (0..cells).map(|e| std::array::from_fn(|r| base[e][r] * frac + ep[e][r])).collect();
            let trial = strain_stresses(&s.model, ds, u, &committed);
            for e in 0..cells {
                let m = &s.materials[s.model.part[e]];
                match return_map(&trial[e], m, hardened[e]) {
                    Some((dep, dh)) => {
                        trial_ep[e] = std::array::from_fn(|r| ep[e][r] + dep[r]);
                        new_hard[e] = hardened[e] + dh;
                        yielding[e] = true;
                    }
                    None => {
                        trial_ep[e] = ep[e];
                        new_hard[e] = hardened[e];
                        yielding[e] = false;
                    }
                }
            }
            let eps: Vec<[f64; 6]> = (0..cells).map(|e| std::array::from_fn(|r| base[e][r] * frac + trial_ep[e][r])).collect();
            tensors = strain_stresses(&s.model, ds, u, &eps);
            // Out of balance: f + F(ε) − K·u on the free directions.
            let fe = eigen_forces(&s.model, ds, &eps);
            let ku = s.model.apply_with(u, false);
            let mut r: Vec<f64> = (0..u.len()).map(|i| s.f[i] * frac + fe[i] - ku[i]).collect();
            s.model.mask(&mut r);
            let size: f64 = r.iter().map(|v| v.abs()).sum();
            if size <= 1e-6 * scale.max(1e-300) {
                done = true;
                break;
            }
            // The correction on the tangent stiffness (rebuilt when what yields changed).
            let own: Vec<Option<Box<[f64]>>> = (0..cells)
                .map(|e| yielding[e].then(|| plastic_stiffness(&s.model.kes[s.model.part[e]], &ds[s.model.part[e]], &tangent_d(&s.materials[s.model.part[e]], &tensors[e]), h)))
                .collect();
            s.model.set_own(own);
            let mut du = vec![0.0; u.len()];
            let (it, _) = s.model.cg(&r, &mut du, 1e-8);
            iterations += it;
            u.iter_mut().zip(&du).for_each(|(a, b)| *a += b);
            // A correction too small to matter: settled (to the solver's own precision).
            let (step_size, whole) = (du.iter().map(|v| v.abs()).fold(0.0, f64::max), u.iter().map(|v| v.abs()).fold(0.0, f64::max));
            if step_size <= 1e-7 * whole.max(1e-300) {
                done = true;
                break;
            }
        }
        settled &= done;
        ep = trial_ep;
        hardened = new_hard;
    }
    s.model.set_own(vec![]);
    let yielded = hardened.iter().filter(|&&a| a > 0.0).count();
    let top = (0..cells).max_by(|&a, &b| hardened[a].total_cmp(&hardened[b])).unwrap_or(0);
    let tangent = s.materials[0].tangent;
    if !settled {
        notes.push("The plastic flow did not settle in every load step: the part may be near collapse (it cannot carry much more), and the figures are approximate.".into());
    }
    notes.push(format!("Plasticity: von Mises with linear hardening ({} MPa past yield), the loads applied in {STEPS} steps; {} of {} cubes yielded.", trim(tangent), yielded, cells));
    let summary = PlasticSummary { max_strain: equivalent_strain(&ep[top]), at: cell_centre(&s.model.grid, s.model.cells[top]), yielded, steps: STEPS, settled, tangent };
    Ok((tensors, iterations, settled, summary, ep))
}

/// The centre of a grid cube.
fn cell_centre(grid: &Grid, c: [usize; 3]) -> [f64; 3] {
    [0, 1, 2].map(|d| grid.origin[d] + (c[d] as f64 + 0.5) * grid.h)
}

/// Von Mises stress of a stress (xx yy zz xy yz zx).
fn von_mises(s: &[f64; 6]) -> f64 {
    (0.5 * ((s[0] - s[1]).powi(2) + (s[1] - s[2]).powi(2) + (s[2] - s[0]).powi(2)) + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5])).sqrt()
}

/// The stress at each cube's centre: σ = D·(B(0)·uₑ − α·ΔT·I) (the extra
/// modes add no strain there; ΔT the cube's warming).
fn cube_stresses(model: &Model, ds: &[[[f64; 6]; 6]], materials: &[Elastic], u: &[f64], dt: &[f64]) -> Vec<[f64; 6]> {
    let b0 = b_matrix([0.0; 3], model.grid.h);
    model
        .corners
        .par_iter()
        .zip(&model.part)
        .zip(dt)
        .map(|((c, &p), &t)| {
            let d = &ds[p];
            let mut ue = [0.0; 24];
            for (a, &n) in c.iter().enumerate() {
                ue[3 * a..3 * a + 3].copy_from_slice(&u[3 * n..3 * n + 3]);
            }
            let mut eps: [f64; 6] = std::array::from_fn(|r| (0..24).map(|j| b0[r][j] * ue[j]).sum());
            let free = materials[p].alpha * t;
            (0..3).for_each(|r| eps[r] -= free);
            std::array::from_fn(|r| (0..6).map(|k| d[r][k] * eps[k]).sum())
        })
        .collect()
}

/// The forces thermal expansion puts on the nodes: each cube, warmed ΔT and
/// held to its size, would push out with the pressure E·α·ΔT/(1 − 2ν), which
/// reaches its corners as w·∫Bᵀ dV · that = w·h³·B(0)ᵀ · that (the
/// incompatible modes take none of a uniform strain).
fn thermal_forces(model: &Model, materials: &[Elastic], dt: &[f64]) -> Vec<f64> {
    let h = model.grid.h;
    let b0 = b_matrix([0.0; 3], h);
    let mut f = vec![0.0; 3 * model.nodes];
    for (((c, &w), &p), &t) in model.corners.iter().zip(&model.weight).zip(&model.part).zip(dt) {
        if t == 0.0 {
            continue;
        }
        let m = &materials[p];
        let push = w * h * h * h * m.e * m.alpha * t / (1.0 - 2.0 * m.nu);
        for (a, &n) in c.iter().enumerate() {
            for d in 0..3 {
                f[3 * n + d] += b0[d][3 * a + d] * push;
            }
        }
    }
    f
}

/// Temperatures loading a stress study, read at any point.
struct Warmth {
    reference: f64,
    uniform: Option<f64>,
    /// A heat study's cubes (on the first part): size, corner, index → °C.
    field: Option<(f64, [f64; 3], std::collections::HashMap<[i64; 3], f32>)>,
}

impl Warmth {
    fn new(heat: &HeatLoad) -> Warmth {
        let field = heat.field.as_ref().map(|t| (t.cell, t.origin, t.cells.iter().map(|(c, v)| (c.map(|x| x as i64), *v)).collect()));
        Warmth { reference: heat.reference, uniform: heat.uniform, field }
    }

    /// The warming (K) above the stress-free temperature at a point of a part.
    fn at(&self, part: usize, x: [f64; 3]) -> f64 {
        let fallback = self.uniform.map_or(0.0, |t| t - self.reference);
        let Some((cell, origin, cells)) = self.field.as_ref().filter(|_| part == 0) else { return fallback };
        let idx = [0, 1, 2].map(|d| ((x[d] - origin[d]) / cell).floor() as i64);
        if let Some(t) = cells.get(&idx) {
            return *t as f64 - self.reference;
        }
        // A cube the heat study left out (a sliver, or a different grid): its filled neighbours.
        let near: Vec<f64> = (-1..=1).flat_map(|a| (-1..=1).flat_map(move |b| (-1..=1).map(move |c| [a, b, c]))).filter_map(|o: [i64; 3]| cells.get(&[idx[0] + o[0], idx[1] + o[1], idx[2] + o[2]]).map(|t| *t as f64)).collect();
        if near.is_empty() {
            fallback
        } else {
            near.iter().sum::<f64>() / near.len() as f64 - self.reference
        }
    }

    fn notes(&self, several: bool) -> Vec<String> {
        let mut out = vec![];
        if self.field.is_some() {
            out.push(format!("Heat: the heat study's temperatures load it, stress-free at {} °C.", trim(self.reference)));
            if several {
                out.push(match self.uniform {
                    Some(t) => format!("The heat study covers the studied part; the others are at {} °C.", trim(t)),
                    None => "The heat study covers the studied part; the others stay at the stress-free temperature.".into(),
                });
            }
        } else if let Some(t) = self.uniform {
            out.push(format!("Heat: {} °C throughout, stress-free at {} °C.", trim(t), trim(self.reference)));
        }
        out
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(String::from).unwrap_or(s)
}

/// Whether two closed meshes (positions, triangles, bounds) touch or nearly
/// do: within about a cube of a normal study (56 cubes) of the two together.
pub fn touching(a: (&[f32], &[u32], [f64; 3], [f64; 3]), b: (&[f32], &[u32], [f64; 3], [f64; 3])) -> bool {
    let size = (0..3).map(|d| a.3[d].max(b.3[d]) - a.2[d].min(b.2[d])).fold(0.0, f64::max);
    let reach = size / 56.0;
    let lo: [f64; 3] = std::array::from_fn(|d| a.2[d].max(b.2[d]) - reach);
    let hi: [f64; 3] = std::array::from_fn(|d| a.3[d].min(b.3[d]) + reach);
    if (0..3).any(|d| hi[d] <= lo[d]) {
        return false;
    }
    // Where their boxes meet, in cubes half that reach.
    let longest = (0..3).map(|d| hi[d] - lo[d]).fold(0.0, f64::max);
    let grid = grid_around(lo, hi, ((longest / (reach / 2.0)).ceil() as usize).clamp(4, 96));
    let (fa, fb) = (fill_of(&grid, a.0, a.1), fill_of(&grid, b.0, b.1));
    let n = grid.n;
    (0..n[2]).any(|k| {
        (0..n[1]).any(|j| {
            (0..n[0]).any(|i| {
                fa[grid.cell(i, j, k)] > 0.02
                    && [[0, 0, 0], [-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]].iter().any(|o: &[i64; 3]| {
                        let c = [i as i64 + o[0], j as i64 + o[1], k as i64 + o[2]];
                        (0..3).all(|d| c[d] >= 0 && (c[d] as usize) < n[d]) && fb[grid.cell(c[0] as usize, c[1] as usize, c[2] as usize)] > 0.02
                    })
            })
        })
    })
}

/// The stress at the cube nearest a point (MPa), for checking results.
pub fn stress_at(r: &Result_, point: [f64; 3]) -> Option<f64> {
    let idx = [0, 1, 2].map(|k| ((point[k] - r.origin[k]) / r.cell - 0.5).round() as i64);
    r.cells.iter().find(|(c, _)| (0..3).all(|k| c[k] as i64 == idx[k])).map(|(_, s)| *s as f64)
}

/// The displacement at the grid node nearest a point (mm).
pub fn displacement_at(r: &Result_, point: [f64; 3]) -> Option<[f64; 3]> {
    let idx = [0, 1, 2].map(|k| ((point[k] - r.origin[k]) / r.cell).round() as i64);
    r.displacements.iter().find(|(c, _)| (0..3).all(|k| c[k] as i64 == idx[k])).map(|(_, d)| d.map(|v| v as f64))
}

/// The displacement of one part (index in a several-part study) at the grid node nearest a point (mm).
pub fn displacement_of(r: &Result_, part: usize, point: [f64; 3]) -> Option<[f64; 3]> {
    let idx = [0, 1, 2].map(|k| ((point[k] - r.origin[k]) / r.cell).round() as i64);
    r.displacements.iter().zip(0..).find(|((c, _), i)| r.node_parts.get(*i).map_or(part == 0, |&p| p as usize == part) && (0..3).all(|k| c[k] as i64 == idx[k])).map(|((_, d), _)| d.map(|v| v as f64))
}

// ---- Natural frequencies -------------------------------------------------------

/// One way the part vibrates.
#[derive(Clone, Debug, Serialize)]
pub struct Mode {
    /// Hz.
    pub frequency: f64,
    /// Which way it mostly moves: "x", "y" or "z".
    pub direction: String,
    /// Per filled cube (in `Modes::cells` order): how far it moves, 0–1.
    pub amount: Vec<f32>,
    /// Per node (in `Modes::nodes` order): the shape, largest movement 1.
    pub shape: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Modes {
    pub cell: f64,
    pub origin: [f64; 3],
    pub cells: Vec<[u32; 3]>,
    pub nodes: Vec<[u32; 3]>,
    pub modes: Vec<Mode>,
    pub mass: f64,
    pub notes: Vec<String>,
}

/// Eigenvalues and vectors of a small symmetric matrix (Jacobi rotations), ascending.
fn symmetric_eigen(mut a: Vec<Vec<f64>>) -> (Vec<f64>, Vec<Vec<f64>>) {
    let n = a.len();
    let mut v: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    for _ in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| a[i][j] * a[i][j]).sum();
        if off < 1e-24 * (0..n).map(|i| a[i][i] * a[i][i]).sum::<f64>().max(1e-300) {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let th = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = th.signum() / (th.abs() + (th * th + 1.0).sqrt());
                let t = if th == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vkp, vkq) = (row[p], row[q]);
                    row[p] = c * vkp - s * vkq;
                    row[q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    (order.iter().map(|&i| a[i][i]).collect(), order.iter().map(|&i| (0..n).map(|k| v[k][i]).collect()).collect())
}

/// The lowest natural frequencies of a part held at its supports, with the
/// mode shapes: subspace iteration on K·φ = ω²·M·φ (lumped masses), each
/// step solving K·z = M·x by conjugate gradients from the last answer.
/// `density` in kg/m³.
pub fn modes(positions: &[f32], triangles: &[u32], material: &Elastic, density: f64, supports: &[Face], count: usize, resolution: usize) -> Result<Modes, String> {
    let body = Body { name: String::new(), positions, triangles, material: Elastic { density, ..*material } };
    let study = Study { supports: supports.to_vec(), resolution, ..Default::default() };
    modes_study(&body, &study, count, false)
}

// ---- Heat ------------------------------------------------------------------------

/// Thermal conductivity (W/m·K), typical by material name.
pub fn conductivity(material: &str) -> Option<f64> {
    materials::lookup(material).map(|m| m.conductivity)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Hold {
    pub face: Face,
    /// °C.
    pub temperature: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Heat {
    pub face: Face,
    /// Watts going in (negative: taken out).
    pub watts: f64,
}

/// Air carrying heat away from every outside face.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Convection {
    /// W/m²·K: about 5–10 still air, 25–100 a fan.
    pub h: f64,
    /// °C.
    pub ambient: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ThermalStudy {
    #[serde(default)]
    pub holds: Vec<Hold>,
    #[serde(default)]
    pub heats: Vec<Heat>,
    #[serde(default)]
    pub convection: Option<Convection>,
    #[serde(default = "default_resolution")]
    pub resolution: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Temperatures {
    pub cell: f64,
    pub origin: [f64; 3],
    /// Filled cubes and their temperature (°C).
    pub cells: Vec<([u32; 3], f32)>,
    pub max: f64,
    pub min: f64,
    /// Watts flowing in through the heated faces, and out through holds and air.
    pub heat_in: f64,
    pub to_holds: f64,
    pub to_air: f64,
    pub notes: Vec<String>,
}

/// Conductance of a cube of side h (mm) per unit conductivity (W/mm·K): 8×8.
fn cube_conductance(h: f64) -> [[f64; 8]; 8] {
    let gp = 1.0 / 3f64.sqrt();
    let w = (h / 2.0).powi(3);
    let mut k = [[0.0; 8]; 8];
    for c in CORNERS {
        let xi = [c[0] * gp, c[1] * gp, c[2] * gp];
        let grad: Vec<[f64; 3]> = CORNERS
            .iter()
            .map(|n| {
                let f = |q: usize| 1.0 + xi[q] * n[q];
                [n[0] * f(1) * f(2) / 8.0 * 2.0 / h, n[1] * f(0) * f(2) / 8.0 * 2.0 / h, n[2] * f(0) * f(1) / 8.0 * 2.0 / h]
            })
            .collect();
        for a in 0..8 {
            for b in 0..8 {
                k[a][b] += w * (grad[a][0] * grad[b][0] + grad[a][1] * grad[b][1] + grad[a][2] * grad[b][2]);
            }
        }
    }
    k
}

/// Steady temperatures through a part: conduction inside, faces held at
/// temperatures, heat in through faces, and air cooling on the outside.
/// `k` in W/m·K.
pub fn thermal(positions: &[f32], triangles: &[u32], k: f64, study: &ThermalStudy) -> Result<Temperatures, String> {
    if study.holds.is_empty() && study.convection.is_none() {
        return Err("hold a face at a temperature, or cool it with air: heat needs somewhere to go".into());
    }
    let hm = HeatModel::build(positions, triangles, k, study)?;
    let mut t: Vec<f64> = (0..hm.nodes).map(|n| hm.fixed[n].unwrap_or(0.0)).collect();
    hm.solve(&hm.q, &vec![0.0; hm.nodes], &mut t);
    Ok(hm.temperatures(&t, study))
}

/// A part's cubes set up for heat: conduction, air on the outside, held
/// faces and heat coming in.
struct HeatModel {
    grid: Grid,
    cells: Vec<[usize; 3]>,
    corners: Vec<[usize; 8]>,
    weight: Vec<f64>,
    used: Vec<bool>,
    nodes: usize,
    ke: [[f64; 8]; 8],
    /// W/mm·K.
    k_mm: f64,
    /// W/K to the air at each node.
    air: Vec<f64>,
    air_area: f64,
    fixed: Vec<Option<f64>>,
    /// W in at each node (heat in, and the air's pull towards ambient).
    q: Vec<f64>,
    heat_in: f64,
    k: f64,
}

impl HeatModel {
    fn build(positions: &[f32], triangles: &[u32], k: f64, study: &ThermalStudy) -> Result<HeatModel, String> {
        let grid = voxelize(positions, triangles, study.resolution.clamp(8, 160));
        let h = grid.h;
        let cells: Vec<[usize; 3]> = (0..grid.n[2]).flat_map(|z| (0..grid.n[1]).flat_map(move |y| (0..grid.n[0]).map(move |x| [x, y, z]))).filter(|&[i, j, kk]| grid.fill[grid.cell(i, j, kk)] >= MIN_FILL).collect();
        if cells.is_empty() {
            return Err("the part is too thin for this resolution: raise it".into());
        }
        let nodes = (grid.n[0] + 1) * (grid.n[1] + 1) * (grid.n[2] + 1);
        let corners: Vec<[usize; 8]> = cells.iter().map(|&[i, j, kk]| grid.corners(i, j, kk)).collect();
        let weight: Vec<f64> = cells.iter().map(|&[i, j, kk]| (grid.fill[grid.cell(i, j, kk)] as f64).min(1.0)).collect();
        let mut used = vec![false; nodes];
        for c in &corners {
            for &n in c {
                used[n] = true;
            }
        }
        // Air: each outside face of a cube (no filled neighbour) sheds h·A·(T − ambient), shared by its 4 corners.
        let mut air = vec![0.0; nodes];
        let filled = |i: i64, j: i64, kk: i64| i >= 0 && j >= 0 && kk >= 0 && (i as usize) < grid.n[0] && (j as usize) < grid.n[1] && (kk as usize) < grid.n[2] && grid.fill[grid.cell(i as usize, j as usize, kk as usize)] >= MIN_FILL;
        let mut air_area = 0.0;
        if let Some(cv) = study.convection {
            let per_face = cv.h * 1e-6 * h * h; // W/K for one cube face (W/m²K × mm² → ×1e-6)
            for &[i, j, kk] in &cells {
                let (i, j, kk) = (i as i64, j as i64, kk as i64);
                for (d, quad) in [([-1, 0, 0], [0usize, 3, 7, 4]), ([1, 0, 0], [1, 2, 6, 5]), ([0, -1, 0], [0, 1, 5, 4]), ([0, 1, 0], [3, 2, 6, 7]), ([0, 0, -1], [0, 1, 2, 3]), ([0, 0, 1], [4, 5, 6, 7])] {
                    if filled(i + d[0], j + d[1], kk + d[2]) {
                        continue;
                    }
                    let c = grid.corners(i as usize, j as usize, kk as usize);
                    for q in quad {
                        air[c[q]] += per_face / 4.0;
                    }
                    air_area += h * h;
                }
            }
        }
        // Held nodes and their temperatures; heat in at the nodes of heated faces.
        let mut fixed: Vec<Option<f64>> = vec![None; nodes];
        for (s, hold) in study.holds.iter().enumerate() {
            let on = nodes_on(&grid, &used, &hold.face);
            if on.is_empty() {
                return Err(format!("held face {} touches no part of the part", s + 1));
            }
            for n in on {
                fixed[n] = Some(hold.temperature);
            }
        }
        let mut q = vec![0.0; nodes];
        let mut heat_in = 0.0;
        let share = node_shares(nodes, &corners, &weight);
        for (s, heat) in study.heats.iter().enumerate() {
            let on: Vec<usize> = nodes_on(&grid, &used, &heat.face).into_iter().filter(|&n| fixed[n].is_none()).collect();
            if on.is_empty() {
                return Err(format!("heated face {} touches no free part of the part", s + 1));
            }
            if heat.face.curved {
                // By area: the same heat through every square millimetre.
                let whole = heat.face.area().max(1e-12);
                for (n, a, _) in area_on_nodes(&on, |id| grid.position(id), h, &heat.face, |_| true) {
                    q[n] += heat.watts * a / whole;
                }
            } else {
                for (n, v) in spread(&on, &share, heat.watts) {
                    q[n] += v;
                }
            }
            heat_in += heat.watts;
        }
        if let Some(cv) = study.convection {
            for n in 0..nodes {
                q[n] += air[n] * cv.ambient;
            }
        }
        Ok(HeatModel { ke: cube_conductance(h), k_mm: k / 1000.0, grid, cells, corners, weight, used, nodes, air, air_area, fixed, q, heat_in, k })
    }

    /// (K + C)·T: conduction, air, and `c` (W/K per node) besides.
    fn apply(&self, t: &[f64], c: &[f64]) -> Vec<f64> {
        let nodes = self.nodes;
        let mut y = self
            .corners
            .par_chunks(4096)
            .zip(self.weight.par_chunks(4096))
            .fold(
                || vec![0.0; nodes],
                |mut y, (chunk, ws)| {
                    for (cn, w) in chunk.iter().zip(ws) {
                        for a in 0..8 {
                            let mut s = 0.0;
                            for b in 0..8 {
                                s += self.ke[a][b] * t[cn[b]];
                            }
                            y[cn[a]] += w * self.k_mm * s;
                        }
                    }
                    y
                },
            )
            .reduce(|| vec![0.0; nodes], |mut a, b| {
                a.iter_mut().zip(&b).for_each(|(x, y)| *x += y);
                a
            });
        y.iter_mut().zip(&self.air).zip(t).zip(c).for_each(|(((y, a), t), c)| *y += (a + c) * t);
        y
    }

    fn free(&self, n: usize) -> bool {
        self.used[n] && self.fixed[n].is_none()
    }

    /// Solves (K + C)·T = q on the free nodes, from `t` (held nodes keep their values).
    fn solve(&self, q: &[f64], c: &[f64], t: &mut [f64]) -> usize {
        let nodes = self.nodes;
        let mut diag: Vec<f64> = self.air.iter().zip(c).map(|(a, c)| a + c).collect();
        for (cn, w) in self.corners.iter().zip(&self.weight) {
            for a in 0..8 {
                diag[cn[a]] += w * self.k_mm * self.ke[a][a];
            }
        }
        let mask = |v: &mut Vec<f64>| v.iter_mut().enumerate().for_each(|(n, x)| if !self.free(n) { *x = 0.0 });
        let kt = self.apply(t, c);
        let mut r: Vec<f64> = q.iter().zip(&kt).map(|(a, b)| a - b).collect();
        mask(&mut r);
        let rnorm0 = r.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
        let pre = |r: &[f64]| -> Vec<f64> { r.iter().zip(&diag).enumerate().map(|(n, (v, d))| if self.free(n) && *d > 0.0 { v / d } else { 0.0 }).collect() };
        let mut z = pre(&r);
        let mut p = z.clone();
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let mut it = 0;
        let _ = nodes;
        while it < 20_000 && r.iter().map(|v| v * v).sum::<f64>().sqrt() > 1e-10 * rnorm0 {
            let mut ap = self.apply(&p, c);
            mask(&mut ap);
            let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
            if pap.abs() < 1e-300 {
                break;
            }
            let alpha = rz / pap;
            t.iter_mut().zip(&p).for_each(|(x, y)| *x += alpha * y);
            r.iter_mut().zip(&ap).for_each(|(x, y)| *x -= alpha * y);
            z = pre(&r);
            let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            p.iter_mut().zip(&z).for_each(|(x, y)| *x = y + rz2 / rz * *x);
            rz = rz2;
            it += 1;
        }
        it
    }

    /// Each cube's temperature (the mean of its corners').
    fn cell_temps(&self, t: &[f64]) -> Vec<f64> {
        self.corners.iter().map(|c| c.iter().map(|&n| t[n]).sum::<f64>() / 8.0).collect()
    }

    /// The steady result: temperatures, and where the heat went.
    fn temperatures(&self, t: &[f64], study: &ThermalStudy) -> Temperatures {
        let nodes = self.nodes;
        let zero = vec![0.0; nodes];
        let kt = self.apply(t, &zero);
        // A held node takes out what conduction brings it beyond its own input: q − K·T there.
        let to_holds: f64 = (0..nodes).filter(|&n| self.used[n] && self.fixed[n].is_some()).map(|n| self.q[n] - kt[n]).sum();
        let to_air: f64 = study.convection.map_or(0.0, |cv| (0..nodes).filter(|&n| self.used[n]).map(|n| self.air[n] * (t[n] - cv.ambient)).sum());
        let cell_t = self.cell_temps(t);
        let (max, min) = (0..nodes).filter(|&n| self.used[n]).fold((f64::NEG_INFINITY, f64::INFINITY), |(hi, lo), n| (hi.max(t[n]), lo.min(t[n])));
        let mut notes = vec![format!("{} cubes of {} mm; conductivity {} W/m·K.", self.cells.len(), crate::drawing::length_text(self.grid.h), self.k)];
        if study.convection.is_some() {
            notes.push(format!("Air cools {} cm² of outside surface (as the cubes' faces).", crate::drawing::length_text(self.air_area / 100.0)));
        }
        Temperatures {
            cell: self.grid.h,
            origin: self.grid.origin,
            cells: self.cells.iter().zip(&cell_t).map(|(c, v)| (c.map(|x| x as u32), *v as f32)).collect(),
            max,
            min,
            heat_in: self.heat_in,
            to_holds,
            to_air,
            notes,
        }
    }
}

/// How a transient heat study runs.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Transient {
    /// Seconds simulated, in `steps` equal steps (backward Euler: steady for any step size).
    pub duration: f64,
    #[serde(default = "default_steps")]
    pub steps: usize,
    /// °C everywhere at the start (held faces jump to theirs at once).
    pub initial: f64,
}

fn default_steps() -> usize {
    50
}

/// Temperatures through time.
#[derive(Clone, Debug, Serialize)]
pub struct TransientTemperatures {
    /// Seconds, and the hottest and coolest °C then.
    pub times: Vec<f64>,
    pub max: Vec<f64>,
    pub min: Vec<f64>,
    /// Each cube's °C at some of the times (`frames` index into `times`), the last always among them.
    pub frames: Vec<usize>,
    pub cells: Vec<[u32; 3]>,
    pub temperatures: Vec<Vec<f32>>,
    pub cell: f64,
    pub origin: [f64; 3],
    /// The state at the end, as a steady study reports it.
    pub last: Temperatures,
    /// Each probe's °C at every time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<Vec<f64>>,
    pub notes: Vec<String>,
}

/// Temperatures through time from a uniform start: C·dT/dt + K·T = q,
/// stepped by backward Euler with masses (heat capacity) lumped at the
/// cube corners. `k` W/m·K, `density` kg/m³, `specific_heat` J/kg·K.
pub fn thermal_transient(positions: &[f32], triangles: &[u32], k: f64, density: f64, specific_heat: f64, study: &ThermalStudy, run: &Transient, probes: &[[f64; 3]]) -> Result<TransientTemperatures, String> {
    if !(run.duration > 0.0) || run.steps == 0 {
        return Err("a transient study needs a duration and steps".into());
    }
    if density <= 0.0 || specific_heat <= 0.0 {
        return Err("the material needs a density and a specific heat for heat over time".into());
    }
    let hm = HeatModel::build(positions, triangles, k, study)?;
    let steps = run.steps.min(2000);
    let dt = run.duration / steps as f64;
    let h = hm.grid.h;
    // Heat capacity per node (J/K): ρ·c·V shared by the corners.
    let mut cap = vec![0.0; hm.nodes];
    for (c, w) in hm.corners.iter().zip(&hm.weight) {
        for &n in c {
            cap[n] += density * specific_heat * 1e-9 * h * h * h * w / 8.0;
        }
    }
    let c_dt: Vec<f64> = cap.iter().map(|c| c / dt).collect();
    let mut t: Vec<f64> = (0..hm.nodes).map(|n| hm.fixed[n].unwrap_or(run.initial)).collect();
    let probe_cells: Vec<Option<usize>> = probes
        .iter()
        .map(|p| {
            let idx = [0, 1, 2].map(|k| ((p[k] - hm.grid.origin[k]) / h - 0.5).round() as i64);
            hm.cells.iter().position(|c| (0..3).all(|k| c[k] as i64 == idx[k]))
        })
        .collect();
    let every = (steps / 20).max(1);
    let (mut times, mut max, mut min, mut frames, mut temperatures) = (vec![0.0], vec![], vec![], vec![], vec![]);
    let mut probe_out: Vec<Vec<f64>> = vec![vec![]; probes.len()];
    let mut record = |t: &[f64], step: usize, times: &Vec<f64>, max: &mut Vec<f64>, min: &mut Vec<f64>, frames: &mut Vec<usize>, temperatures: &mut Vec<Vec<f32>>| {
        let cells = hm.cell_temps(t);
        let (hi, lo) = (0..hm.nodes).filter(|&n| hm.used[n]).fold((f64::NEG_INFINITY, f64::INFINITY), |(a, b), n| (a.max(t[n]), b.min(t[n])));
        max.push(hi);
        min.push(lo);
        for (out, c) in probe_out.iter_mut().zip(&probe_cells) {
            out.push(c.map_or(f64::NAN, |c| cells[c]));
        }
        if step % every == 0 || step == steps {
            frames.push(times.len() - 1);
            temperatures.push(cells.iter().map(|v| *v as f32).collect());
        }
    };
    record(&t, 0, &times, &mut max, &mut min, &mut frames, &mut temperatures);
    for step in 1..=steps {
        // (K + C/Δt)·Tₙ₊₁ = q + C/Δt·Tₙ.
        let rhs: Vec<f64> = hm.q.iter().zip(&c_dt).zip(&t).map(|((q, c), t)| q + c * t).collect();
        hm.solve(&rhs, &c_dt, &mut t);
        times.push(step as f64 * dt);
        record(&t, step, &times, &mut max, &mut min, &mut frames, &mut temperatures);
    }
    let mut last = hm.temperatures(&t, study);
    let mut notes = std::mem::take(&mut last.notes);
    notes.push(format!("{} s in {} steps of {} s (backward Euler), from {} °C; heat capacity {} J/kg·K at {} kg/m³.", trim(run.duration), steps, trim(dt), trim(run.initial), trim(specific_heat), trim(density)));
    Ok(TransientTemperatures { times, max, min, frames, cells: hm.cells.iter().map(|c| c.map(|x| x as u32)).collect(), temperatures, cell: h, origin: hm.grid.origin, last, probes: probe_out, notes })
}
/// The temperature at the cube nearest a point (°C).
pub fn temperature_at(r: &Temperatures, point: [f64; 3]) -> Option<f64> {
    let idx = [0, 1, 2].map(|k| ((point[k] - r.origin[k]) / r.cell - 0.5).round() as i64);
    r.cells.iter().find(|(c, _)| (0..3).all(|k| c[k] as i64 == idx[k])).map(|(_, s)| *s as f64)
}

// ---- Distance to a surface -----------------------------------------------------

/// The point of triangle (a, b, c) nearest p (Ericson, Real-Time Collision Detection 5.1.5).
pub(crate) fn nearest_on_triangle(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let sub = |u: [f64; 3], v: [f64; 3]| [u[0] - v[0], u[1] - v[1], u[2] - v[2]];
    let dot = |u: [f64; 3], v: [f64; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let along = |o: [f64; 3], d: [f64; 3], t: f64| [o[0] + t * d[0], o[1] + t * d[1], o[2] + t * d[2]];
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return along(a, ab, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return along(a, ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    [a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w]
}

/// Where a closed mesh's surface is nearest each point: the signed distance
/// (negative inside), the nearest point, and the outward direction there.
/// Only triangles within `reach` of the points' box are looked at (a point
/// farther than that from the surface gets an infinite distance).
fn surface_near(points: &[[f64; 3]], positions: &[f32], triangles: &[u32], reach: f64) -> Vec<(f64, [f64; 3], [f64; 3])> {
    if points.is_empty() {
        return vec![];
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in points {
        for d in 0..3 {
            lo[d] = lo[d].min(p[d] - reach);
            hi[d] = hi[d].max(p[d] + reach);
        }
    }
    let at = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    // The triangles near, each with its unit normal (counter-clockwise seen from outside).
    let tris: Vec<([[f64; 3]; 3], [f64; 3])> = triangles
        .chunks_exact(3)
        .map(|t| [at(t[0]), at(t[1]), at(t[2])])
        .filter(|t| (0..3).all(|d| t.iter().any(|v| v[d] >= lo[d]) && t.iter().any(|v| v[d] <= hi[d])))
        .filter_map(|t| {
            let (u, v) = ([t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]]);
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            (l > 1e-18).then(|| (t, n.map(|c| c / l)))
        })
        .collect();
    points
        .par_iter()
        .map(|&p| {
            // Nearest, and among (near) ties the triangle the point sits most squarely off.
            let mut best = (f64::INFINITY, f64::NEG_INFINITY, p, [0.0, 0.0, 1.0], 0.0);
            for (t, n) in &tris {
                let q = nearest_on_triangle(p, t[0], t[1], t[2]);
                let e = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                let d2 = e[0] * e[0] + e[1] * e[1] + e[2] * e[2];
                let side = e[0] * n[0] + e[1] * n[1] + e[2] * n[2];
                let squarely = side.abs() / d2.sqrt().max(1e-300);
                if d2 < best.0 * (1.0 - 1e-9) - 1e-24 || (d2 <= best.0 * (1.0 + 1e-9) + 1e-24 && squarely > best.1) {
                    best = (d2, squarely, q, *n, side);
                }
            }
            let (d2, _, q, n, side) = best;
            if !d2.is_finite() {
                return (f64::INFINITY, p, n);
            }
            let d = d2.sqrt();
            let sign = if side < 0.0 { -1.0 } else { 1.0 };
            // Off the surface, the way out is from the nearest point to it (or back, inside).
            let out = if d > 1e-9 { [0, 1, 2].map(|k| sign * (p[k] - q[k]) / d) } else { n };
            (sign * d, q, out)
        })
        .collect()
}
