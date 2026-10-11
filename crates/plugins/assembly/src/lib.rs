//! Components and assemblies, the way designers build: the smallest parts
//! first, then assemblies of them, nested as deep as needed.
//!
//! A *definition* is either a shape program (a component) or an assembly.
//! An *instance* places a definition inside an assembly, or at the top of the
//! project, at a position and a rotation about Z. Every instance of a
//! definition shows the same design: edit the definition and all of them
//! change. `make_unique` gives one instance its own copy to customise (a
//! penthouse floor in a stack of typical floors). Instances carry free-form
//! properties (use, sale status, owner) for schedules such as a stacking plan.
//!
//! Dependencies run upward (instance → its definition, assembly → its
//! instances), so a change deep inside reaches every assembly and
//! requirement above it.

use de_core::{
    Command, CommandError, Derived, ErrorCode, IdIndex, Persist, Plugin, Registry, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_shape::{PartMaterials, ShapeProgram, ShapeSolid};
use de_solid::{MassProperties, MaterialRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct AssemblyPlugin;

impl Plugin for AssemblyPlugin {
    fn name(&self) -> &'static str {
        "assembly"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<AssemblyDef>()
            .component::<Instance>()
            .component::<Group>()
            .command::<CreateGroup>()
            .command::<UpdateGroup>()
            .command::<CreateAssembly>()
            .command::<UpdateAssembly>()
            .command::<CreateInstance>()
            .command::<UpdateInstance>()
            .command::<MakeUnique>()
            .command::<DeleteInstance>()
            .derived(Derived {
                name: "assembly.stats",
                inputs: &[AssemblyDef::KIND, Instance::KIND],
                reads_dependencies: true,
                compute: stats,
            });
    }
}

// ---- Design data -----------------------------------------------------------

/// An assembly definition: what its instances (children) make together.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssemblyDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl Persist for AssemblyDef {
    const KIND: &'static str = "assembly.definition";
}

/// A placed copy of a definition.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Instance {
    pub name: String,
    /// The definition shown here: a shape program or an assembly.
    pub of: StableId,
    /// The assembly this sits in; `None` at the top of the project.
    #[serde(default)]
    pub parent: Option<StableId>,
    /// Position of the definition's origin in the parent (mm).
    pub at: [f64; 3],
    /// Rotation about Z (degrees), applied before the position.
    #[serde(default)]
    pub rotation: f64,
    /// Free-form facts: "use": "Residence", "status": "Sold", "owner": "…".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

impl Persist for Instance {
    const KIND: &'static str = "assembly.instance";
}

/// A rigid placement: rotation about Z, then translation (mm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub at: [f64; 3],
    pub rotation: f64,
}

impl Transform {
    pub const IDENTITY: Transform = Transform { at: [0.0; 3], rotation: 0.0 };

    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let (s, c) = self.rotation.to_radians().sin_cos();
        [c * p[0] - s * p[1] + self.at[0], s * p[0] + c * p[1] + self.at[1], p[2] + self.at[2]]
    }

    /// `self` placed inside `outer`.
    pub fn within(&self, outer: &Transform) -> Transform {
        Transform { at: outer.apply(self.at), rotation: outer.rotation + self.rotation }
    }

    pub fn solid(&self, m: &manifold_csg::Manifold) -> manifold_csg::Manifold {
        let r = if self.rotation.rem_euclid(360.0).abs() < 1e-12 { m.translate(0.0, 0.0, 0.0) } else { m.rotate(0.0, 0.0, self.rotation) };
        r.translate(self.at[0], self.at[1], self.at[2])
    }

    /// Bounds of a box after this transform.
    pub fn bounds(&self, lo: [f64; 3], hi: [f64; 3]) -> ([f64; 3], [f64; 3]) {
        let mut a = [f64::INFINITY; 3];
        let mut b = [f64::NEG_INFINITY; 3];
        for x in [lo[0], hi[0]] {
            for y in [lo[1], hi[1]] {
                for z in [lo[2], hi[2]] {
                    let p = self.apply([x, y, z]);
                    for k in 0..3 {
                        a[k] = a[k].min(p[k]);
                        b[k] = b[k].max(p[k]);
                    }
                }
            }
        }
        (a, b)
    }
}

impl Instance {
    pub fn transform(&self) -> Transform {
        Transform { at: self.at, rotation: self.rotation }
    }
}

/// One leaf shape somewhere inside an instance tree.
#[derive(Clone, Debug)]
pub struct Occurrence {
    /// The shape program entity.
    pub shape: StableId,
    /// Instance path from the top (outermost first).
    pub path: Vec<StableId>,
    /// Names along the path, ending with the shape's name.
    pub names: Vec<String>,
    /// Placement of the shape's own coordinates in the root's coordinates.
    pub transform: Transform,
}

fn entity(world: &World, id: StableId) -> Option<Entity> {
    world.resource::<IdIndex>().get(id)
}

/// The stable id of an entity (a scan: assemblies are walked rarely and are small).
fn id_of(world: &World, e: Entity) -> Option<StableId> {
    world.resource::<IdIndex>().iter().find(|(_, x)| *x == e).map(|(id, _)| id)
}

/// Children of an assembly definition (its dependencies that are instances in it).
pub fn children(world: &World, assembly: StableId) -> Vec<(StableId, Instance)> {
    match entity(world, assembly) {
        Some(e) => children_of(world, e, assembly),
        None => Vec::new(),
    }
}

fn children_of(world: &World, e: Entity, assembly: StableId) -> Vec<(StableId, Instance)> {
    let deps = world.get::<de_core::DependsOn>(e).map(|d| d.0.clone()).unwrap_or_default();
    deps.into_iter()
        .filter_map(|id| {
            let inst = entity(world, id).and_then(|e| world.get::<Instance>(e))?;
            (inst.parent == Some(assembly)).then(|| (id, inst.clone()))
        })
        .collect()
}

/// Every leaf shape under `id` (an instance, an assembly definition or a
/// shape), in `id`'s coordinates.
pub fn occurrences(world: &World, id: StableId) -> Vec<Occurrence> {
    let mut out = Vec::new();
    if let Some(e) = entity(world, id) {
        walk(world, e, Transform::IDENTITY, &mut Vec::new(), &mut Vec::new(), &mut out, 0);
    }
    out
}

fn walk(world: &World, e: Entity, t: Transform, path: &mut Vec<StableId>, names: &mut Vec<String>, out: &mut Vec<Occurrence>, depth: usize) {
    if depth > 32 {
        return; // guarded against cycles at commit; this is belt and braces
    }
    let next = |id: StableId| entity(world, id);
    if let Some(inst) = world.get::<Instance>(e) {
        let Some(id) = id_of(world, e) else { return };
        path.push(id);
        names.push(inst.name.clone());
        if let Some(def) = next(inst.of) {
            walk(world, def, inst.transform().within(&t), path, names, out, depth + 1);
        }
        path.pop();
        names.pop();
    } else if world.get::<AssemblyDef>(e).is_some() {
        let Some(id) = id_of(world, e) else { return };
        for (child, _) in children_of(world, e, id) {
            if let Some(c) = next(child) {
                walk(world, c, t, path, names, out, depth + 1);
            }
        }
    } else if let Some(p) = world.get::<ShapeProgram>(e) {
        let Some(id) = id_of(world, e) else { return };
        let mut n = names.clone();
        n.push(p.name.clone());
        out.push(Occurrence { shape: id, path: path.clone(), names: n, transform: t });
    }
}

// ---- Derived: what an assembly or instance adds up to ----------------------

/// Totals for an assembly definition or an instance, in its own coordinates (mm).
#[derive(Component, Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AssemblyStats {
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    /// `None` when some shape has no material.
    pub mass_kg: Option<f64>,
    /// Leaf shapes placed, and fabricable parts (assembly parts count each).
    pub shapes: usize,
    pub parts: usize,
    /// Metrics the shape programs report (e.g. "Net area" m², "Keys"), summed.
    pub metrics: BTreeMap<String, f64>,
}

fn stats(world: &World, e: Entity) -> Option<Result<AssemblyStats, String>> {
    if world.get::<AssemblyDef>(e).is_none() && world.get::<Instance>(e).is_none() {
        return None;
    }
    let mut occ = Vec::new();
    walk(world, e, Transform::IDENTITY, &mut Vec::new(), &mut Vec::new(), &mut occ, 0);
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut mass = Some(0.0);
    let mut parts = 0;
    let mut metrics = BTreeMap::new();
    for o in &occ {
        let Some(se) = entity(world, o.shape) else { continue };
        let Some(solid) = world.get::<ShapeSolid>(se) else {
            return Some(Err(format!("{} has no valid result", o.names.join(" › "))));
        };
        let (a, b) = o.transform.bounds(solid.stats.bounds_min, solid.stats.bounds_max);
        for k in 0..3 {
            lo[k] = lo[k].min(a[k]);
            hi[k] = hi[k].max(b[k]);
        }
        mass = match (mass, world.get::<MassProperties>(se).and_then(|m| m.mass_kg)) {
            (Some(total), Some(m)) => Some(total + m),
            _ => None,
        };
        parts += solid.stats.parts.len().max(1);
        for (k, v) in &solid.stats.metrics {
            *metrics.entry(k.clone()).or_insert(0.0) += v.value;
        }
    }
    if occ.is_empty() {
        lo = [0.0; 3];
        hi = [0.0; 3];
    }
    Some(Ok(AssemblyStats { bounds_min: lo, bounds_max: hi, mass_kg: mass, shapes: occ.len(), parts, metrics }))
}

// ---- Commands --------------------------------------------------------------

fn bad(code: ErrorCode, m: impl Into<String>) -> CommandError {
    CommandError::new(code, m)
}

fn is_definition(tx: &Tx, id: StableId) -> bool {
    tx.get::<ShapeProgram>(id).is_some() || tx.get::<AssemblyDef>(id).is_some()
}

/// Whether placing `of` inside `parent` would make an assembly contain itself.
fn contains(tx: &Tx, of: StableId, parent: StableId, depth: usize) -> bool {
    if of == parent {
        return true;
    }
    if depth > 64 || tx.get::<AssemblyDef>(of).is_none() {
        return false;
    }
    children(tx.world(), of).iter().any(|(_, inst)| contains(tx, inst.of, parent, depth + 1))
}

/// `assembly.create`: a new, empty assembly definition.
#[derive(Deserialize, JsonSchema)]
pub struct CreateAssembly {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

impl Command for CreateAssembly {
    const NAME: &'static str = "assembly.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.name.trim().is_empty() {
            return Err(bad(ErrorCode::InvalidParameters, "an assembly needs a name"));
        }
        let id = tx.create();
        tx.set(id, AssemblyDef { name: self.name, description: self.description })?;
        Ok(json!({ "id": id }))
    }
}

/// A named group of items that are picked and moved together (a desk and its
/// drawers, a motor and its bracket). Unlike an assembly it changes nothing
/// about the items: ungrouping (deleting the group) leaves them as they are.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Group {
    pub name: String,
    pub members: Vec<StableId>,
}

impl Persist for Group {
    const KIND: &'static str = "assembly.group";
}

/// `assembly.group.create`: group two or more items.
#[derive(Deserialize, JsonSchema)]
pub struct CreateGroup {
    pub name: String,
    pub members: Vec<StableId>,
}

fn check_members(tx: &Tx, members: &[StableId]) -> Result<Vec<StableId>, CommandError> {
    let mut out: Vec<StableId> = vec![];
    for m in members {
        if !tx.exists(*m) {
            return Err(bad(ErrorCode::UnresolvedReference, format!("{m} does not exist")));
        }
        if !out.contains(m) {
            out.push(*m);
        }
    }
    if out.len() < 2 {
        return Err(bad(ErrorCode::InvalidParameters, "a group needs two or more items"));
    }
    Ok(out)
}

impl Command for CreateGroup {
    const NAME: &'static str = "assembly.group.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let members = check_members(tx, &self.members)?;
        let id = tx.create();
        let name = if self.name.trim().is_empty() { "Group".to_string() } else { self.name };
        tx.set(id, Group { name, members })?;
        Ok(json!({ "id": id }))
    }
}

/// `assembly.group.update`: rename a group, or change who is in it.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateGroup {
    pub target: StableId,
    pub name: Option<String>,
    pub members: Option<Vec<StableId>>,
}

impl Command for UpdateGroup {
    const NAME: &'static str = "assembly.group.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut g = tx.get::<Group>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a group", self.target)))?;
        if let Some(n) = self.name {
            g.name = n;
        }
        if let Some(m) = self.members {
            g.members = check_members(tx, &m)?;
        }
        tx.set(self.target, g)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.update`: rename or re-describe an assembly definition.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateAssembly {
    pub target: StableId,
    pub name: Option<String>,
    pub description: Option<String>,
}

impl Command for UpdateAssembly {
    const NAME: &'static str = "assembly.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut def = tx.get::<AssemblyDef>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an assembly", self.target)))?;
        if let Some(n) = self.name {
            def.name = n;
        }
        if let Some(d) = self.description {
            def.description = d;
        }
        tx.set(self.target, def)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.instance.create`: place a definition in an assembly (or at the top).
#[derive(Deserialize, JsonSchema)]
pub struct CreateInstance {
    pub of: StableId,
    pub parent: Option<StableId>,
    pub name: String,
    #[serde(default)]
    pub at: [f64; 3],
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

impl Command for CreateInstance {
    const NAME: &'static str = "assembly.instance.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if !is_definition(tx, self.of) {
            return Err(bad(ErrorCode::UnresolvedReference, format!("{} is not a component or an assembly", self.of)));
        }
        if let Some(parent) = self.parent {
            if tx.get::<AssemblyDef>(parent).is_none() {
                return Err(bad(ErrorCode::UnresolvedReference, format!("{parent} is not an assembly")));
            }
            if contains(tx, self.of, parent, 0) {
                return Err(bad(ErrorCode::InvalidParameters, "an assembly cannot contain itself"));
            }
        }
        if !(self.at.iter().all(|v| v.is_finite()) && self.rotation.is_finite()) {
            return Err(bad(ErrorCode::InvalidParameters, "position and rotation must be finite numbers"));
        }
        let id = tx.create();
        tx.set(id, Instance { name: self.name, of: self.of, parent: self.parent, at: self.at, rotation: self.rotation, properties: self.properties })?;
        tx.add_dependency(id, self.of)?;
        if let Some(parent) = self.parent {
            tx.add_dependency(parent, id)?;
        }
        Ok(json!({ "id": id }))
    }
}

/// `assembly.instance.update`: move, rotate, rename, swap the definition, or set properties
/// (a property set to "" is removed).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateInstance {
    pub target: StableId,
    pub name: Option<String>,
    pub at: Option<[f64; 3]>,
    pub rotation: Option<f64>,
    pub of: Option<StableId>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

impl Command for UpdateInstance {
    const NAME: &'static str = "assembly.instance.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut inst = tx.get::<Instance>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an instance", self.target)))?;
        if let Some(n) = self.name {
            inst.name = n;
        }
        if let Some(at) = self.at {
            if !at.iter().all(|v| v.is_finite()) {
                return Err(bad(ErrorCode::InvalidParameters, "position must be finite"));
            }
            inst.at = at;
        }
        if let Some(r) = self.rotation {
            if !r.is_finite() {
                return Err(bad(ErrorCode::InvalidParameters, "rotation must be finite"));
            }
            inst.rotation = r;
        }
        if let Some(of) = self.of {
            if !is_definition(tx, of) {
                return Err(bad(ErrorCode::UnresolvedReference, format!("{of} is not a component or an assembly")));
            }
            if let Some(parent) = inst.parent
                && contains(tx, of, parent, 0)
            {
                return Err(bad(ErrorCode::InvalidParameters, "an assembly cannot contain itself"));
            }
            tx.remove_dependency(self.target, inst.of)?;
            tx.add_dependency(self.target, of)?;
            inst.of = of;
        }
        for (k, v) in self.properties {
            if v.is_empty() {
                inst.properties.remove(&k);
            } else {
                inst.properties.insert(k, v);
            }
        }
        tx.set(self.target, inst)?;
        Ok(json!({ "id": self.target }))
    }
}

/// Copies a definition: shapes copy their program and materials; assemblies
/// copy their instances (which keep sharing their own definitions until those
/// are made unique too).
fn copy_definition(tx: &mut Tx, def: StableId, name: Option<String>) -> Result<StableId, CommandError> {
    let id = tx.create();
    if let Some(mut program) = tx.get::<ShapeProgram>(def).cloned() {
        if let Some(n) = name {
            program.name = n;
        }
        let materials = tx.get::<PartMaterials>(def).cloned();
        let material = tx.get::<MaterialRef>(def).copied();
        // A copy reads the same linked parameters as the original.
        for r in de_shape::params::references(&program.params) {
            if r != def {
                tx.add_dependency(id, r)?;
            }
        }
        tx.set(id, program)?;
        if let Some(m) = material {
            tx.set(id, m)?;
            tx.add_dependency(id, m.0)?;
        }
        if let Some(table) = materials {
            for m in table.0.values() {
                tx.add_dependency(id, *m)?;
            }
            tx.set(id, table)?;
        }
    } else if let Some(mut a) = tx.get::<AssemblyDef>(def).cloned() {
        if let Some(n) = name {
            a.name = n;
        }
        tx.set(id, a)?;
        for (_, mut child) in children(tx.world(), def) {
            child.parent = Some(id);
            let of = child.of;
            let c = tx.create();
            tx.set(c, child)?;
            tx.add_dependency(c, of)?;
            tx.add_dependency(id, c)?;
        }
    } else {
        return Err(bad(ErrorCode::UnresolvedReference, format!("{def} is not a component or an assembly")));
    }
    Ok(id)
}

/// `assembly.instance.make_unique`: give this instance its own copy of its
/// definition, to customise without changing the others.
#[derive(Deserialize, JsonSchema)]
pub struct MakeUnique {
    pub target: StableId,
    /// Name for the new definition (default: the instance's name).
    pub name: Option<String>,
}

impl Command for MakeUnique {
    const NAME: &'static str = "assembly.instance.make_unique";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut inst = tx.get::<Instance>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an instance", self.target)))?;
        let copy = copy_definition(tx, inst.of, Some(self.name.unwrap_or_else(|| inst.name.clone())))?;
        tx.remove_dependency(self.target, inst.of)?;
        tx.add_dependency(self.target, copy)?;
        inst.of = copy;
        tx.set(self.target, inst)?;
        Ok(json!({ "id": self.target, "definition": copy }))
    }
}

/// `assembly.instance.delete`: remove an instance from its assembly.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteInstance {
    pub target: StableId,
}

impl Command for DeleteInstance {
    const NAME: &'static str = "assembly.instance.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let inst = tx.get::<Instance>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an instance", self.target)))?;
        if let Some(parent) = inst.parent {
            tx.remove_dependency(parent, self.target)?;
        }
        tx.delete(self.target)?;
        Ok(json!({ "id": self.target }))
    }
}
