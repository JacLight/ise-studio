//! Solid modeling plugin.
//!
//! First slice: parametric box features and materials, with mass properties
//! and a display mesh as derived results. Exact B-rep geometry (OCCT) replaces
//! the analytic box behind these same components and commands.

use de_core::{
    Command, CommandError, Density, Derived, ErrorCode, IdIndex, Length, Persist, Plugin, Registry, StableId, Tx,
    Value, bevy_ecs, json, schemars,
};
use bevy_ecs::{component::Component, entity::Entity, world::World};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub struct SolidPlugin;

impl Plugin for SolidPlugin {
    fn name(&self) -> &'static str {
        "solid"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<BoxFeature>()
            .component::<Material>()
            .component::<MaterialRef>()
            .component::<Placement>()
            .component::<Pivot>()
            .command::<CreateBox>()
            .command::<MovePart>()
            .command::<EditBox>()
            .command::<SetPivot>()
            .command::<SetBoxDimensions>()
            .command::<CreateMaterial>()
            .command::<SetMaterialDensity>()
            .command::<SetMaterialCost>()
            .command::<SetMaterialProperties>()
            .command::<AssignMaterial>()
            .derived(Derived {
                name: "solid.display_mesh",
                inputs: &[BoxFeature::KIND],
                reads_dependencies: false,
                compute: display_mesh,
            })
            .derived(Derived {
                name: "solid.mass_properties",
                inputs: &[BoxFeature::KIND, MaterialRef::KIND],
                reads_dependencies: true,
                compute: mass_properties,
            });
    }
}

// ---- Design data -----------------------------------------------------------

/// Axis-aligned box with one corner at the part origin.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BoxFeature {
    pub width: Length,
    pub depth: Length,
    pub height: Length,
}

impl Persist for BoxFeature {
    const KIND: &'static str = "solid.box";
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Material {
    pub name: String,
    pub density: Density,
    /// What the material costs to buy, for cost estimates in the bill of
    /// materials. Absent (older projects, or not priced yet) means unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<MaterialCost>,
    /// Engineering properties set on this material, overriding the typical
    /// values the simulation library gives its name (absent: all typical).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<EngineeringProps>,
}

/// A material's engineering properties as the person set them; any left out
/// come from the simulation library's typical values for its name. Units:
/// MPa for moduli and strengths, kg/m³ density (the material's own density
/// is used when this has none), W/m·K, J/kg·K, 1/K, °C.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EngineeringProps {
    /// Young's modulus (MPa).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e: Option<f64>,
    /// Poisson's ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nu: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yield_strength: Option<f64>,
    /// Ultimate tensile strength (MPa).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uts: Option<f64>,
    /// Slope of the stress–strain line past yield (MPa), for bilinear plasticity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tangent_modulus: Option<f64>,
    /// Fully reversed fatigue strength (MPa) at `fatigue_cycles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fatigue_strength: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fatigue_cycles: Option<f64>,
    /// Thermal conductivity (W/m·K).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conductivity: Option<f64>,
    /// Specific heat (J/kg·K).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specific_heat: Option<f64>,
    /// Linear thermal expansion (1/K).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<f64>,
    /// Where these figures come from (a datasheet, a test).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// The price of a material, in the forms it is bought: by weight, by the
/// sheet, or by the bar. Any can be given; estimates use the form that suits
/// the part's process (sheet for laser, router and sheet-metal parts, bar for
/// cut extrusions, weight for everything else).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaterialCost {
    /// ISO 4217 code of the prices (default the project's currency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Price per kilogram.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_kg: Option<f64>,
    /// Price of one sheet of a stated size (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<SheetPrice>,
    /// Price of one bar or length of stock (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<BarPrice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SheetPrice {
    pub price: f64,
    /// Sheet size, mm (e.g. 2440 × 1220).
    pub length: f64,
    pub width: f64,
    /// Thickness, mm, when the price is for one thickness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BarPrice {
    pub price: f64,
    /// Length of one bar, mm (e.g. 6000).
    pub length: f64,
}

impl MaterialCost {
    /// Refuses negative, infinite or zero-sized prices.
    pub fn check(&self) -> Result<(), CommandError> {
        let bad = |m: &str| Err(CommandError::new(ErrorCode::InvalidParameters, m.to_string()));
        let price = |v: f64| v.is_finite() && v >= 0.0;
        let size = |v: f64| v.is_finite() && v > 0.0;
        if self.per_kg.is_some_and(|v| !price(v)) {
            return bad("cost per kg must be a price of at least 0");
        }
        if let Some(s) = &self.sheet {
            if !price(s.price) || !size(s.length) || !size(s.width) || s.thickness.is_some_and(|t| !size(t)) {
                return bad("a sheet price needs a price of at least 0 and a size greater than zero");
            }
        }
        if let Some(b) = &self.bar {
            if !price(b.price) || !size(b.length) {
                return bad("a bar price needs a price of at least 0 and a length greater than zero");
            }
        }
        if let Some(c) = &self.currency {
            if c.len() != 3 || !c.chars().all(|c| c.is_ascii_alphabetic()) {
                return bad("currency is a three-letter code such as USD, EUR, GBP");
            }
        }
        Ok(())
    }
}

impl Persist for Material {
    const KIND: &'static str = "solid.material";
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaterialRef(pub StableId);

impl Persist for MaterialRef {
    const KIND: &'static str = "solid.material_ref";
}

/// Position of the part origin in project coordinates (Z up).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Placement {
    pub x: Length,
    pub y: Length,
    pub z: Length,
}

impl Persist for Placement {
    const KIND: &'static str = "solid.placement";
}

/// The point a part moves (and will rotate) about, as fractions of its box:
/// 0 = minimum side, 1 = maximum side on each axis; values outside 0..1 are
/// outside the box. Absent means the center.
/// Fractions keep a chosen anchor (a corner, a face center) meaningful when
/// the part is resized.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pivot {
    pub u: f64,
    pub v: f64,
    pub w: f64,
}

impl Default for Pivot {
    fn default() -> Self {
        Self { u: 0.5, v: 0.5, w: 0.5 }
    }
}

impl Persist for Pivot {
    const KIND: &'static str = "solid.pivot";
}

// ---- Derived results -------------------------------------------------------

#[derive(Component, Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MassProperties {
    pub volume_m3: f64,
    /// `None` until a material is assigned.
    pub mass_kg: Option<f64>,
    pub center_of_mass_m: [f64; 3],
}

/// Triangle mesh for the viewport, in meters relative to the part origin.
#[derive(Component, Clone, Debug, PartialEq, Serialize)]
pub struct DisplayMesh {
    pub positions: Vec<f32>,
    /// Per-vertex normals; empty means flat-shade each triangle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
}

fn mass_properties(world: &World, entity: Entity) -> Option<Result<MassProperties, String>> {
    let b = world.get::<BoxFeature>(entity)?;
    let (w, d, h) = (b.width.canonical(), b.depth.canonical(), b.height.canonical());
    let volume_m3 = w * d * h;
    let mass_kg = match world.get::<MaterialRef>(entity) {
        None => None,
        Some(MaterialRef(id)) => {
            let material = world
                .resource::<IdIndex>()
                .get(*id)
                .and_then(|e| world.get::<Material>(e));
            match material {
                Some(m) => Some(volume_m3 * m.density.canonical()),
                None => return Some(Err(format!("material {id} not found"))),
            }
        }
    };
    Some(Ok(MassProperties { volume_m3, mass_kg, center_of_mass_m: [w / 2.0, d / 2.0, h / 2.0] }))
}

fn display_mesh(world: &World, entity: Entity) -> Option<Result<DisplayMesh, String>> {
    let b = world.get::<BoxFeature>(entity)?;
    let (w, d, h) = (b.width.canonical() as f32, b.depth.canonical() as f32, b.height.canonical() as f32);
    let positions = vec![
        0.0, 0.0, 0.0, w, 0.0, 0.0, w, d, 0.0, 0.0, d, 0.0, //
        0.0, 0.0, h, w, 0.0, h, w, d, h, 0.0, d, h,
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, //
        1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6, 3, 0, 4, 3, 4, 7,
    ];
    Some(Ok(DisplayMesh { positions, normals: Vec::new(), indices }))
}

// ---- Commands --------------------------------------------------------------

fn positive(name: &str, l: Length) -> Result<Length, CommandError> {
    if l.canonical() > 0.0 && l.is_finite() {
        Ok(l)
    } else {
        Err(CommandError::new(ErrorCode::InvalidGeometry, format!("{name} must be greater than zero")))
    }
}

fn material_exists(tx: &Tx, id: StableId) -> Result<(), CommandError> {
    match tx.get::<Material>(id) {
        Some(_) => Ok(()),
        None => Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a material"))),
    }
}

fn assign(tx: &mut Tx, target: StableId, material: Option<StableId>) -> Result<(), CommandError> {
    if let Some(MaterialRef(old)) = tx.get::<MaterialRef>(target).copied() {
        tx.remove_dependency(target, old)?;
        tx.remove::<MaterialRef>(target)?;
    }
    if let Some(material) = material {
        material_exists(tx, material)?;
        tx.set(target, MaterialRef(material))?;
        tx.add_dependency(target, material)?;
    }
    Ok(())
}

/// `solid.box.create`: creates a box part, optionally with a material.
#[derive(Deserialize, JsonSchema)]
pub struct CreateBox {
    pub width: Length,
    pub depth: Length,
    pub height: Length,
    pub material: Option<StableId>,
    pub placement: Option<Placement>,
}

impl Command for CreateBox {
    const NAME: &'static str = "solid.box.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let feature = BoxFeature {
            width: positive("width", self.width)?,
            depth: positive("depth", self.depth)?,
            height: positive("height", self.height)?,
        };
        let id = tx.create();
        tx.set(id, feature)?;
        tx.set(id, self.placement.unwrap_or_default())?;
        assign(tx, id, self.material)?;
        Ok(json!({ "id": id }))
    }
}

/// `solid.box.set_dimensions`: changes any of a box's dimensions.
#[derive(Deserialize, JsonSchema)]
pub struct SetBoxDimensions {
    pub target: StableId,
    pub width: Option<Length>,
    pub depth: Option<Length>,
    pub height: Option<Length>,
}

impl Command for SetBoxDimensions {
    const NAME: &'static str = "solid.box.set_dimensions";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut feature = tx
            .get::<BoxFeature>(self.target)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a box", self.target)))?;
        if let Some(w) = self.width {
            feature.width = positive("width", w)?;
        }
        if let Some(d) = self.depth {
            feature.depth = positive("depth", d)?;
        }
        if let Some(h) = self.height {
            feature.height = positive("height", h)?;
        }
        tx.set(self.target, feature)?;
        Ok(json!({ "id": self.target }))
    }
}

/// Any placed solid is a part: boxes here, and shapes from other plugins.
fn require_part(tx: &Tx, id: StableId) -> Result<(), CommandError> {
    if tx.get::<BoxFeature>(id).is_some() || tx.get::<Placement>(id).is_some() {
        Ok(())
    } else {
        Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a part")))
    }
}

/// `solid.part.move`: sets a part's placement.
#[derive(Deserialize, JsonSchema)]
pub struct MovePart {
    pub target: StableId,
    pub placement: Placement,
}

impl Command for MovePart {
    const NAME: &'static str = "solid.part.move";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        require_part(tx, self.target)?;
        let p = self.placement;
        if !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "placement must be finite"));
        }
        tx.set(self.target, p)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `solid.box.edit`: size and placement in one atomic change (one undo step),
/// e.g. pulling a box's minus-side face grows it while its far face stays put.
#[derive(Deserialize, JsonSchema)]
pub struct EditBox {
    pub target: StableId,
    pub width: Option<Length>,
    pub depth: Option<Length>,
    pub height: Option<Length>,
    pub placement: Option<Placement>,
}

impl Command for EditBox {
    const NAME: &'static str = "solid.box.edit";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        SetBoxDimensions { target: self.target, width: self.width, depth: self.depth, height: self.height }.apply(tx)?;
        if let Some(placement) = self.placement {
            MovePart { target: self.target, placement }.apply(tx)?;
        }
        Ok(json!({ "id": self.target }))
    }
}

/// `solid.part.set_pivot`: where the part moves and rotates from.
#[derive(Deserialize, JsonSchema)]
pub struct SetPivot {
    pub target: StableId,
    pub u: f64,
    pub v: f64,
    pub w: f64,
}

impl Command for SetPivot {
    const NAME: &'static str = "solid.part.set_pivot";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        require_part(tx, self.target)?;
        // Fractions beyond 0..1 place the pivot outside the box (e.g. a hinge
        // line beside the part); the bound only rejects nonsense.
        let sane = |f: f64| f.is_finite() && (-1000.0..=1000.0).contains(&f);
        if !(sane(self.u) && sane(self.v) && sane(self.w)) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "pivot must be within 1000 part-lengths of the part"));
        }
        tx.set(self.target, Pivot { u: self.u, v: self.v, w: self.w })?;
        Ok(json!({ "id": self.target }))
    }
}

/// `solid.material.create`
#[derive(Deserialize, JsonSchema)]
pub struct CreateMaterial {
    pub name: String,
    pub density: Density,
    /// What it costs (per kg, per sheet, per bar), for BOM estimates.
    #[serde(default)]
    pub cost: Option<MaterialCost>,
    /// Engineering properties (E, ν, strengths, thermal) beyond the typical ones for its name.
    #[serde(default)]
    pub properties: Option<EngineeringProps>,
}

impl Command for CreateMaterial {
    const NAME: &'static str = "solid.material.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.density.canonical() <= 0.0 {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "density must be greater than zero"));
        }
        if let Some(c) = &self.cost {
            c.check()?;
        }
        let id = tx.create();
        tx.set(id, Material { name: self.name, density: self.density, cost: self.cost, properties: self.properties })?;
        Ok(json!({ "id": id }))
    }
}

/// `solid.material.set_density`: every part using the material recomputes.
#[derive(Deserialize, JsonSchema)]
pub struct SetMaterialDensity {
    pub target: StableId,
    pub density: Density,
}

impl Command for SetMaterialDensity {
    const NAME: &'static str = "solid.material.set_density";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.density.canonical() <= 0.0 {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "density must be greater than zero"));
        }
        let mut material = tx
            .get::<Material>(self.target)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a material", self.target)))?;
        material.density = self.density;
        tx.set(self.target, material)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `solid.material.set_cost`: sets or clears (`null`) what a material costs.
#[derive(Deserialize, JsonSchema)]
pub struct SetMaterialCost {
    pub target: StableId,
    pub cost: Option<MaterialCost>,
}

impl Command for SetMaterialCost {
    const NAME: &'static str = "solid.material.set_cost";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut material = tx
            .get::<Material>(self.target)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a material", self.target)))?;
        if let Some(c) = &self.cost {
            c.check()?;
        }
        material.cost = self.cost.filter(|c| c != &MaterialCost::default());
        tx.set(self.target, material)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `solid.material.set_properties`: sets or clears (`null`) a material's
/// engineering properties; every study and check using it re-runs.
#[derive(Deserialize, JsonSchema)]
pub struct SetMaterialProperties {
    pub target: StableId,
    pub properties: Option<EngineeringProps>,
}

impl Command for SetMaterialProperties {
    const NAME: &'static str = "solid.material.set_properties";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut material = tx
            .get::<Material>(self.target)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a material", self.target)))?;
        if let Some(p) = &self.properties {
            let positive = [("E", p.e), ("yield strength", p.yield_strength), ("tensile strength", p.uts), ("fatigue strength", p.fatigue_strength), ("fatigue cycles", p.fatigue_cycles), ("conductivity", p.conductivity), ("specific heat", p.specific_heat)];
            if let Some((what, _)) = positive.iter().find(|(_, v)| v.is_some_and(|v| !(v > 0.0 && v.is_finite()))) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, format!("{what} must be greater than zero")));
            }
            if p.nu.is_some_and(|v| !(-1.0 < v && v < 0.5)) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "Poisson's ratio must be between −1 and 0.5"));
            }
            if p.tangent_modulus.is_some_and(|v| v < 0.0) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "the tangent modulus cannot be negative"));
            }
        }
        material.properties = self.properties.filter(|p| p != &EngineeringProps::default());
        tx.set(self.target, material)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `solid.material.assign`: sets or clears (`null`) a part's material.
#[derive(Deserialize, JsonSchema)]
pub struct AssignMaterial {
    pub target: StableId,
    pub material: Option<StableId>,
}

impl Command for AssignMaterial {
    const NAME: &'static str = "solid.material.assign";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        // Component definitions (solids with no placement of their own) take materials too.
        if tx.get::<DisplayMesh>(self.target).is_none() {
            require_part(tx, self.target)?;
        }
        assign(tx, self.target, self.material)?;
        Ok(json!({ "id": self.target }))
    }
}
