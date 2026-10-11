//! Components and assemblies, the way designers build: the smallest parts
//! first, then assemblies of them, nested as deep as needed.
//!
//! A *definition* is either a shape program (a component) or an assembly.
//! An *instance* places a definition inside an assembly, or at the top of the
//! project, at a position and any rotation (degrees about X, then Y, then Z),
//! or where joints between connectors put it (see `joints`). Every instance of a
//! definition shows the same design: edit the definition and all of them
//! change. `make_unique` gives one instance its own copy to customise (a
//! penthouse floor in a stack of typical floors). An instance of a shape with
//! configurations can show one of them instead (the M8×30 of a bolt whose
//! definition shows M6×20), still sharing the definition. Instances carry
//! free-form properties (use, sale status, owner) for schedules such as a
//! stacking plan.
//!
//! Dependencies run upward (instance → its definition, assembly → its
//! instances), so a change deep inside reaches every assembly and
//! requirement above it.

use de_core::{
    Command, CommandError, Derived, ErrorCode, IdIndex, Persist, Plugin, Registry, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_shape::{PartMaterials, ShapeConfigurations, ShapeProgram, ShapeSolid};
use de_solid::{MassProperties, MaterialRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub mod comments;
pub mod configs;
pub mod fit;
pub mod joints;
pub mod motion;
pub mod transform;
pub use comments::{Anchor, CommentReply, CommentThread, Stroke};

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
            .component::<joints::Joint>()
            .command::<joints::CreateJoint>()
            .command::<joints::UpdateJoint>()
            .command::<joints::DeleteJoint>()
            .command::<configs::SetConfigTable>()
            .component::<fit::FitProfile>()
            .command::<fit::SetFitProfile>()
            .component::<CommentThread>()
            .component::<CommentReply>()
            .command::<comments::CreateComment>()
            .command::<comments::ReplyComment>()
            .command::<comments::MarkupComment>()
            .command::<comments::ResolveComment>()
            .command::<comments::ReopenComment>()
            .command::<comments::EditComment>()
            .command::<comments::DeleteComment>()
            // Where each instance is: its own placement, or where its joints put it.
            .derived(Derived {
                name: "assembly.placed",
                inputs: &[Instance::KIND],
                reads_dependencies: true,
                compute: joints::placed,
            })
            // An instance in a configuration of its shape, before the totals that read it.
            .derived(Derived {
                name: "assembly.configured",
                inputs: &[Instance::KIND],
                reads_dependencies: true,
                compute: configured,
            })
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
    /// Rotation (degrees) about X, then Y, then Z, applied before the
    /// position. Projects saved before full 3D placement stored one number,
    /// the turn about Z; it still loads.
    #[serde(default, deserialize_with = "transform::rotation_compat")]
    pub rotation: [f64; 3],
    /// Free-form facts: "use": "Residence", "status": "Sold", "owner": "…".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
    /// For an instance of a shape with configurations: the one shown here
    /// (`None` shows the definition as it is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
}

impl Persist for Instance {
    const KIND: &'static str = "assembly.instance";
}

pub use transform::{M3, Transform};

impl Instance {
    /// Where it is as stored (its own position and rotation). An instance a
    /// joint holds is drawn where the joint puts it: see [`placement`].
    pub fn transform(&self) -> Transform {
        Transform::from_euler(self.at, self.rotation)
    }
}

/// Where an instance sits in its parent now: where its joints put it when
/// they hold it, else its own position and rotation.
pub fn placement(world: &World, e: Entity) -> Option<Transform> {
    if let Some(p) = world.get::<joints::Placed>(e) {
        return Some(p.transform());
    }
    world.get::<Instance>(e).map(Instance::transform)
}

/// One leaf shape somewhere inside an instance tree.
#[derive(Clone, Debug)]
pub struct Occurrence {
    /// The shape program entity.
    pub shape: StableId,
    /// Instance path from the top (outermost first).
    pub path: Vec<StableId>,
    /// Names along the path, ending with the shape's name (with its
    /// configuration: “Bolt (M8×30)”).
    pub names: Vec<String>,
    /// The configuration of the shape shown here, if its instance names one.
    pub config: Option<String>,
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
            let placed = placement(world, e).unwrap_or_else(|| inst.transform());
            walk(world, def, placed.within(&t), path, names, out, depth + 1);
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
        // The instance that placed this shape may show one of its configurations;
        // a shape on its own shows its active one.
        let config = match path.last().and_then(|i| entity(world, *i)).and_then(|i| world.get::<Instance>(i)) {
            Some(inst) => inst.config.clone(),
            None => None,
        };
        let shown = config.clone().or_else(|| world.get::<ShapeConfigurations>(e).and_then(|t| t.active.clone()));
        let mut n = names.clone();
        n.push(de_shape::configs::display_name(&p.name, shown.as_deref()));
        out.push(Occurrence { shape: id, path: path.clone(), names: n, transform: t, config });
    }
}

// ---- Derived: what an assembly or instance adds up to ----------------------

/// Derived, on every instance: its shape evaluated in the configuration it
/// names (`None` when it shows its definition as it is).
#[derive(Component, Clone, Default)]
pub struct InstanceConfigured(pub Option<Arc<de_shape::configs::Configured>>);

fn configured(world: &World, e: Entity) -> Option<Result<InstanceConfigured, String>> {
    let inst = world.get::<Instance>(e)?;
    let (Some(name), Some(def)) = (&inst.config, entity(world, inst.of)) else { return Some(Ok(InstanceConfigured(None))) };
    if world.get::<ShapeProgram>(def).is_none() {
        return Some(Ok(InstanceConfigured(None)));
    }
    Some(de_shape::configs::evaluate(world, def, name).map(|c| InstanceConfigured(Some(Arc::new(c)))))
}

/// The configured result an occurrence shows, when its instance names a configuration.
pub fn configured_of<'w>(world: &'w World, o: &Occurrence) -> Option<&'w de_shape::configs::Configured> {
    o.config.as_ref()?;
    let e = entity(world, *o.path.last()?)?;
    world.get::<InstanceConfigured>(e)?.0.as_deref()
}

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
        // An instance in a configuration counts as that configuration.
        let (stats, kg) = match (configured_of(world, o), &o.config) {
            (Some(c), _) => (&c.stats, c.mass.mass_kg),
            (None, Some(_)) => return Some(Err(format!("{} has no valid result", o.names.join(" › ")))),
            (None, None) => match world.get::<ShapeSolid>(se) {
                Some(solid) => (&solid.stats, world.get::<MassProperties>(se).and_then(|m| m.mass_kg)),
                None => return Some(Err(format!("{} has no valid result", o.names.join(" › ")))),
            },
        };
        let (a, b) = o.transform.bounds(stats.bounds_min, stats.bounds_max);
        for k in 0..3 {
            lo[k] = lo[k].min(a[k]);
            hi[k] = hi[k].max(b[k]);
        }
        mass = match (mass, kg) {
            (Some(total), Some(m)) => Some(total + m),
            _ => None,
        };
        parts += stats.parts.len().max(1);
        for (k, v) in &stats.metrics {
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
    /// Degrees about X, then Y, then Z.
    #[serde(default)]
    pub rotation: [f64; 3],
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
    /// A configuration of the shape to show (see `shape.config.set_table`).
    pub config: Option<String>,
}

/// Checks that `config` is a configuration of definition `of`.
fn check_config(tx: &Tx, of: StableId, config: &str) -> Result<(), CommandError> {
    let Some(program) = tx.get::<ShapeProgram>(of) else {
        return Err(bad(ErrorCode::InvalidParameters, "only a shape's instances can show a configuration"));
    };
    match tx.get::<ShapeConfigurations>(of) {
        Some(t) if t.row(config).is_some() => Ok(()),
        Some(t) => Err(bad(ErrorCode::InvalidParameters, format!("{} has no configuration “{config}” (it has {})", program.name, t.names()))),
        None => Err(bad(ErrorCode::InvalidParameters, format!("{} has no configurations: make a table first (shape.config.set_table)", program.name))),
    }
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
        if !(self.at.iter().all(|v| v.is_finite()) && self.rotation.iter().all(|v| v.is_finite())) {
            return Err(bad(ErrorCode::InvalidParameters, "position and rotation must be finite numbers"));
        }
        let config = self.config.filter(|c| !c.is_empty());
        if let Some(c) = &config {
            check_config(tx, self.of, c)?;
        }
        let id = tx.create();
        tx.set(id, Instance { name: self.name, of: self.of, parent: self.parent, at: self.at, rotation: self.rotation, properties: self.properties, config })?;
        tx.add_dependency(id, self.of)?;
        if let Some(parent) = self.parent {
            tx.add_dependency(parent, id)?;
        }
        Ok(json!({ "id": id }))
    }
}

/// `assembly.instance.update`: move, rotate, rename, swap the definition, pick
/// a configuration of it ("" shows the definition as it is), or set
/// properties (a property set to "" is removed).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateInstance {
    pub target: StableId,
    pub name: Option<String>,
    pub at: Option<[f64; 3]>,
    /// Degrees about X, then Y, then Z.
    pub rotation: Option<[f64; 3]>,
    pub of: Option<StableId>,
    pub config: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

impl Command for UpdateInstance {
    const NAME: &'static str = "assembly.instance.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut inst = tx.get::<Instance>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an instance", self.target)))?;
        let before = inst.clone();
        if let Some(n) = self.name {
            inst.name = n;
        }
        // An instance a joint holds goes where the joint puts it.
        let moved = self.at.is_some_and(|a| a != inst.at) || self.rotation.is_some_and(|r| r != inst.rotation);
        if moved && let Some(why) = joints::held_reason(tx.world(), self.target) {
            return Err(bad(ErrorCode::InvalidParameters, why));
        }
        if let Some(at) = self.at {
            if !at.iter().all(|v| v.is_finite()) {
                return Err(bad(ErrorCode::InvalidParameters, "position must be finite"));
            }
            inst.at = at;
        }
        if let Some(r) = self.rotation {
            if !r.iter().all(|v| v.is_finite()) {
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
            // The new definition keeps the configuration only if it has one of that name.
            if inst.config.as_ref().is_some_and(|c| check_config(tx, of, c).is_err()) {
                inst.config = None;
            }
        }
        if let Some(c) = self.config {
            if c.is_empty() {
                inst.config = None;
            } else {
                check_config(tx, inst.of, &c)?;
                inst.config = Some(c);
            }
        }
        for (k, v) in self.properties {
            if v.is_empty() {
                inst.properties.remove(&k);
            } else {
                inst.properties.insert(k, v);
            }
        }
        let (of_changed, config_changed) = (inst.of != before.of, inst.config != before.config);
        let parent = inst.parent;
        tx.set(self.target, inst)?;
        // Its joints read its connectors: the instances they hold follow a new definition or configuration.
        if of_changed {
            joints::rewire(tx, parent)?;
        }
        if of_changed || config_changed {
            joints::touch(tx, self.target, parent)?;
        }
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
        let configs = tx.get::<ShapeConfigurations>(def).cloned();
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
        if let Some(table) = configs {
            tx.set(id, table)?;
        }
    } else if let Some(mut a) = tx.get::<AssemblyDef>(def).cloned() {
        if let Some(n) = name {
            a.name = n;
        }
        tx.set(id, a)?;
        let mut map = std::collections::HashMap::new();
        for (old, mut child) in children(tx.world(), def) {
            child.parent = Some(id);
            let of = child.of;
            let c = tx.create();
            tx.set(c, child)?;
            tx.add_dependency(c, of)?;
            tx.add_dependency(id, c)?;
            map.insert(old, c);
        }
        // Its joints come too, joining the copies.
        joints::copy_joints(tx, def, id, &map)?;
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
        // The copy shows the configuration this instance showed, as its active one.
        if let Some(c) = inst.config.take()
            && let Some(mut table) = tx.get::<ShapeConfigurations>(copy).cloned()
        {
            table.active = Some(c);
            de_shape::configs::store_table(tx, copy, table)?;
        }
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
        // Its joints go with it; what they held stays where it is.
        joints::delete_joints_of(tx, self.target, inst.parent)?;
        if let Some(parent) = inst.parent {
            tx.remove_dependency(parent, self.target)?;
        }
        tx.delete(self.target)?;
        joints::rewire(tx, inst.parent)?;
        Ok(json!({ "id": self.target }))
    }
}
