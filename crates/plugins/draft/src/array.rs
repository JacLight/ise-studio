//! Arrays of parts: copies of an instance in a row or grid, round an axis,
//! or along a draft path. Every copy is a real instance of the same
//! definition (so the BOM counts them, exports and checks see them, and an
//! edit to the definition changes them all), placed by the array. The
//! array is also a parameter set — `spacing`, `spacing2`, `angle` — so its
//! spacing links like any parameter (`=Rack.pitch`), and moving the
//! original moves the copies. The count is set on the array (copies are
//! added or removed), not by formula.

use crate::{DraftShape, bad, geom, missing};
use de_assembly::{
    Instance,
    joints::Placed,
    transform::{M3, Transform},
};
use de_core::{
    Command, CommandError, DependsOn, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_shape::{ParameterSet, ResolvedParams};
use geom::{V3, add, cross, dot, mul, norm, sub, unit};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArrayKind {
    /// In a row (and, with count2, a grid).
    Linear,
    /// Round an axis.
    Polar,
    /// Along a draft object (line, wire, arc, B-spline…).
    Path,
}

/// An array's layout. Lives on an entity that is also a parameter set
/// (spacing, spacing2, angle).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Array {
    pub kind: ArrayKind,
    /// The instance copied (copy 1, which stays where it is).
    pub source: StableId,
    /// The copies, in order (2, 3, …).
    #[serde(default)]
    pub members: Vec<StableId>,
    /// How many along the first direction (or round, or along the path), the original included.
    pub count: u32,
    #[serde(default = "one")]
    pub count2: u32,
    #[serde(default = "xdir")]
    pub direction: V3,
    #[serde(default = "ydir")]
    pub direction2: V3,
    #[serde(default)]
    pub center: V3,
    #[serde(default = "zdir")]
    pub axis: V3,
    /// Polar: copies turn with the array (else they only move).
    #[serde(default = "yes")]
    pub rotate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<StableId>,
    /// Path: copies turn to follow the path's direction.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub align: bool,
}

fn one() -> u32 {
    1
}
fn xdir() -> V3 {
    [1.0, 0.0, 0.0]
}
fn ydir() -> V3 {
    [0.0, 1.0, 0.0]
}
fn zdir() -> V3 {
    [0.0, 0.0, 1.0]
}
fn yes() -> bool {
    true
}

impl Persist for Array {
    const KIND: &'static str = "draft.array";
}

impl Array {
    pub fn total(&self) -> u32 {
        self.count * self.count2.max(1)
    }
}

fn rotation(axis: V3, deg: f64) -> M3 {
    let k = unit(axis).unwrap_or([0.0, 0.0, 1.0]);
    let (s, c) = deg.to_radians().sin_cos();
    let t = 1.0 - c;
    let [x, y, z] = k;
    [[t * x * x + c, t * x * y - s * z, t * x * z + s * y], [t * x * y + s * z, t * y * y + c, t * y * z - s * x], [t * x * z - s * y, t * y * z + s * x, t * z * z + c]]
}

/// The rotation turning unit vector a onto b (the shortest way).
fn turn_onto(a: V3, b: V3) -> M3 {
    let c = cross(a, b);
    let s = norm(c);
    let d = dot(a, b).clamp(-1.0, 1.0);
    if s < 1e-12 {
        if d > 0.0 {
            return rotation([0.0, 0.0, 1.0], 0.0);
        }
        // Opposite: half a turn about any axis square to a.
        let w = if a[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        return rotation(cross(a, w), 180.0);
    }
    rotation(c, s.atan2(d).to_degrees())
}

/// Where copy `i` (0 = the original) goes, as a transform applied after the original's placement.
pub fn offset(a: &Array, values: &BTreeMap<String, f64>, path: Option<&DraftShape>, i: u32) -> Result<Transform, String> {
    let v = |k: &str, d: f64| values.get(k).copied().unwrap_or(d);
    Ok(match a.kind {
        ArrayKind::Linear => {
            let (u, w) = (i % a.count, i / a.count);
            let d1 = unit(a.direction).ok_or("the array's direction is zero")?;
            let d2 = unit(a.direction2).ok_or("the array's second direction is zero")?;
            Transform::translation(add(mul(d1, v("spacing", 10.0) * u as f64), mul(d2, v("spacing2", 10.0) * w as f64)))
        }
        ArrayKind::Polar => {
            let sweep = v("angle", 360.0);
            let full = (sweep.abs() - 360.0).abs() < 1e-9;
            let step = if full { sweep / a.count as f64 } else { sweep / (a.count.max(2) - 1) as f64 };
            let r = rotation(a.axis, step * i as f64);
            let c = a.center;
            // About the axis through the centre.
            Transform { at: sub(c, de_assembly::transform::mul(&r, c)), rot: r }
        }
        ArrayKind::Path => {
            let shape = path.ok_or("the array's path is gone")?;
            let line = shape.lines.first().ok_or("the path has no line to follow")?;
            let total = geom::length(line);
            if total <= 0.0 {
                return Err("the path has no length".into());
            }
            let gaps = if shape.closed { a.count } else { (a.count.max(2) - 1) as u32 };
            let s = total * i as f64 / gaps as f64;
            let (p0, t0) = geom::along(line, 0.0).ok_or("the path has no length")?;
            let (p, t) = geom::along(line, s).ok_or("the path has no length")?;
            if a.align {
                let r = turn_onto(t0, t);
                Transform { at: sub(p, de_assembly::transform::mul(&r, p0)), rot: r }
            } else {
                Transform::translation(sub(p, p0))
            }
        }
    })
}

/// Where copy `i` sits: the original's placement, then the array's offset.
fn copy_transform(original: Transform, a: &Array, values: &BTreeMap<String, f64>, path: Option<&DraftShape>, i: u32) -> Result<Transform, String> {
    let o = offset(a, values, path, i)?;
    Ok(if a.kind == ArrayKind::Polar && !a.rotate {
        // Only carried round: the position turns, the part does not.
        Transform { at: o.apply(original.at), rot: original.rot }
    } else {
        o.then(&original)
    })
}

/// Derived on instances: a copy in an array is placed by it.
pub(crate) fn member_placed(world: &World, e: Entity) -> Option<Result<Placed, String>> {
    world.get::<Instance>(e)?;
    let id = *world.get::<StableId>(e)?;
    let deps = world.get::<DependsOn>(e)?;
    let index = world.resource::<IdIndex>();
    let (ae, a) = deps.0.iter().find_map(|d| index.get(*d).and_then(|x| world.get::<Array>(x).map(|a| (x, a))))?;
    let i = a.members.iter().position(|m| *m == id)? as u32 + 1;
    Some((|| {
        let se = index.get(a.source).ok_or("the array's original is gone")?;
        let original = de_assembly::placement(world, se).ok_or("the array's original is gone")?;
        let values = world.get::<ResolvedParams>(ae).map(|r| r.0.clone()).unwrap_or_default();
        let path = a.path.and_then(|p| index.get(p)).and_then(|p| world.get::<DraftShape>(p));
        Ok(Placed::at_transform(copy_transform(original, a, &values, path, i)?))
    })())
}

// ---- Commands ----------------------------------------------------------------------------------------

/// Copies to match the array's count, each placed where it goes now.
fn sync(tx: &mut Tx, id: StableId) -> Result<Vec<StableId>, CommandError> {
    let mut a = tx.get::<Array>(id).cloned().ok_or_else(|| missing(id, "array"))?;
    let source = tx.get::<Instance>(a.source).cloned().ok_or_else(|| missing(a.source, "instance"))?;
    let se = tx.entity(a.source)?;
    let original = de_assembly::placement(tx.world(), se).unwrap_or_else(|| source.transform());
    let set = tx.get::<ParameterSet>(id).cloned().ok_or_else(|| missing(id, "array"))?;
    let values = de_shape::params::resolve(tx.world(), &set.params).map_err(bad)?;
    let path_shape = match a.path {
        Some(p) => {
            let o = tx.get::<crate::DraftObject>(p).cloned().ok_or_else(|| missing(p, "draft object (the path)"))?;
            Some(crate::shape_of(tx.world(), &o).map_err(bad)?)
        }
        None => None,
    };
    let want = a.total() as usize - 1;
    while a.members.len() > want {
        let m = a.members.pop().unwrap();
        tx.remove_dependency(m, id)?;
        tx.run("assembly.instance.delete", json!({ "target": m }))?;
    }
    for (k, m) in a.members.clone().iter().enumerate() {
        let t = copy_transform(original, &a, &values, path_shape.as_ref(), k as u32 + 1).map_err(bad)?;
        if let Some(mut inst) = tx.get::<Instance>(*m).cloned() {
            inst.at = t.at.map(|v| (v * 1e9).round() / 1e9);
            inst.rotation = t.euler().map(|v| (v * 1e9).round() / 1e9);
            inst.of = source.of;
            inst.config = source.config.clone();
            tx.set(*m, inst)?;
        }
    }
    while a.members.len() < want {
        let i = a.members.len() as u32 + 1;
        let t = copy_transform(original, &a, &values, path_shape.as_ref(), i).map_err(bad)?;
        let out = tx.run(
            "assembly.instance.create",
            json!({ "of": source.of, "parent": source.parent, "name": format!("{} {}", source.name, i + 1), "at": t.at, "rotation": t.euler(), "properties": source.properties, "config": source.config }),
        )?;
        let m: StableId = serde_json::from_value(out["id"].clone()).map_err(|_| bad("the copy was not made"))?;
        tx.add_dependency(m, id)?;
        a.members.push(m);
    }
    tx.set(id, a.clone())?;
    Ok(a.members)
}

fn check(a: &Array) -> Result<(), CommandError> {
    if a.count < 2 && a.count2 < 2 {
        return Err(bad("an array needs a count of 2 or more"));
    }
    if a.count < 1 || a.total() > 5000 {
        return Err(bad("an array holds up to 5000 copies"));
    }
    if a.kind == ArrayKind::Path && a.path.is_none() {
        return Err(bad("a path array needs `path`: a draft object to follow"));
    }
    if a.kind != ArrayKind::Linear && a.count2 > 1 {
        return Err(bad("count2 (a grid) is for linear arrays"));
    }
    for v in [a.direction, a.direction2, a.center, a.axis] {
        if v.iter().any(|x| !x.is_finite()) {
            return Err(bad("an array's vectors must be finite"));
        }
    }
    Ok(())
}

/// `draft.array.create`: copies of an instance in a row or grid (linear:
/// `direction`, `spacing`, and `count2`/`direction2`/`spacing2` for a grid),
/// round an axis (polar: `center`, `axis`, `angle` — 360 spreads them evenly
/// round, less spreads them from the original to the last — and `rotate`),
/// or along a draft object (path: `path`, `align`). Returns the array (a
/// parameter set named `name`) and its copies.
#[derive(Deserialize, JsonSchema)]
pub struct CreateArray {
    pub source: StableId,
    pub kind: ArrayKind,
    pub count: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub spacing: Option<f64>,
    #[serde(default)]
    pub direction: Option<V3>,
    #[serde(default)]
    pub count2: Option<u32>,
    #[serde(default)]
    pub spacing2: Option<f64>,
    #[serde(default)]
    pub direction2: Option<V3>,
    #[serde(default)]
    pub center: Option<V3>,
    #[serde(default)]
    pub axis: Option<V3>,
    #[serde(default)]
    pub angle: Option<f64>,
    #[serde(default)]
    pub rotate: Option<bool>,
    #[serde(default)]
    pub path: Option<StableId>,
    #[serde(default)]
    pub align: Option<bool>,
}

impl Command for CreateArray {
    const NAME: &'static str = "draft.array.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let source = tx.get::<Instance>(self.source).cloned().ok_or_else(|| missing(self.source, "placed instance (arrays copy instances)"))?;
        if tx.get::<DependsOn>(self.source).is_some_and(|d| d.0.iter().any(|x| tx.get::<Array>(*x).is_some())) {
            return Err(bad(format!("{} is already a copy in an array: array the original", source.name)));
        }
        let a = Array {
            kind: self.kind,
            source: self.source,
            members: vec![],
            count: self.count,
            count2: self.count2.unwrap_or(1).max(1),
            direction: self.direction.unwrap_or([1.0, 0.0, 0.0]),
            direction2: self.direction2.unwrap_or([0.0, 1.0, 0.0]),
            center: self.center.unwrap_or([0.0; 3]),
            axis: self.axis.unwrap_or([0.0, 0.0, 1.0]),
            rotate: self.rotate.unwrap_or(true),
            path: self.path,
            align: self.align.unwrap_or(false),
        };
        check(&a)?;
        if let Some(p) = a.path {
            tx.get::<crate::DraftObject>(p).ok_or_else(|| missing(p, "draft object (the path)"))?;
        }
        let name = self.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} array", source.name));
        let params = json!([
            { "key": "spacing", "label": "Spacing", "value": self.spacing.unwrap_or(20.0), "unit": "mm", "min": null, "max": null, "step": null },
            { "key": "spacing2", "label": "Row spacing", "value": self.spacing2.unwrap_or(20.0), "unit": "mm", "min": null, "max": null, "step": null },
            { "key": "angle", "label": "Angle", "value": self.angle.unwrap_or(360.0), "unit": "deg", "min": -360.0, "max": 360.0, "step": null }
        ]);
        let out = tx.run("parameters.create", json!({ "name": name, "description": "Array spacing and angle (link them like any parameter).", "params": params }))?;
        let id: StableId = serde_json::from_value(out["id"].clone()).map_err(|_| bad("the array was not made"))?;
        tx.set(id, a.clone())?;
        tx.add_dependency(id, self.source)?;
        if let Some(p) = a.path {
            tx.add_dependency(id, p)?;
        }
        let members = sync(tx, id)?;
        Ok(json!({ "id": id, "name": name, "copies": members }))
    }
}

/// `draft.array.update`: change an array's layout or count (copies are added
/// or removed); `spacing`, `spacing2` and `angle` are its parameters (a
/// linked one follows its formula: unlink it to set it here).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateArray {
    pub target: StableId,
    #[serde(default)]
    pub kind: Option<ArrayKind>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub count2: Option<u32>,
    #[serde(default)]
    pub spacing: Option<f64>,
    #[serde(default)]
    pub spacing2: Option<f64>,
    #[serde(default)]
    pub angle: Option<f64>,
    #[serde(default)]
    pub direction: Option<V3>,
    #[serde(default)]
    pub direction2: Option<V3>,
    #[serde(default)]
    pub center: Option<V3>,
    #[serde(default)]
    pub axis: Option<V3>,
    #[serde(default)]
    pub rotate: Option<bool>,
    #[serde(default)]
    pub path: Option<StableId>,
    #[serde(default)]
    pub align: Option<bool>,
}

impl Command for UpdateArray {
    const NAME: &'static str = "draft.array.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut a = tx.get::<Array>(self.target).cloned().ok_or_else(|| missing(self.target, "array"))?;
        macro_rules! set {
            ($($f:ident),*) => { $( if let Some(v) = self.$f { a.$f = v; } )* };
        }
        set!(kind, count, count2, direction, direction2, center, axis, rotate, align);
        if let Some(p) = self.path {
            tx.get::<crate::DraftObject>(p).ok_or_else(|| missing(p, "draft object (the path)"))?;
            if let Some(old) = a.path {
                tx.remove_dependency(self.target, old)?;
            }
            a.path = Some(p);
            tx.add_dependency(self.target, p)?;
        }
        check(&a)?;
        tx.set(self.target, a)?;
        let mut values = BTreeMap::new();
        for (k, v) in [("spacing", self.spacing), ("spacing2", self.spacing2), ("angle", self.angle)] {
            if let Some(v) = v {
                values.insert(k, v);
            }
        }
        if !values.is_empty() {
            tx.run("parameters.set", json!({ "target": self.target, "values": values }))?;
        }
        let members = sync(tx, self.target)?;
        Ok(json!({ "id": self.target, "copies": members }))
    }
}

/// `draft.array.delete`: ends an array. Its copies stay as ordinary
/// instances where they are (`keep: false` deletes them).
#[derive(Deserialize, JsonSchema)]
pub struct DeleteArray {
    pub target: StableId,
    #[serde(default = "yes")]
    pub keep: bool,
}

impl Command for DeleteArray {
    const NAME: &'static str = "draft.array.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        sync(tx, self.target)?;
        let a = tx.get::<Array>(self.target).cloned().ok_or_else(|| missing(self.target, "array"))?;
        for m in &a.members {
            tx.remove_dependency(*m, self.target)?;
            if !self.keep {
                tx.run("assembly.instance.delete", json!({ "target": m }))?;
            }
        }
        tx.delete(self.target)?;
        Ok(json!({ "deleted": self.target, "kept": if self.keep { a.members.len() } else { 0 } }))
    }
}
