//! Saved ways of showing an assembly: named poses (every joint's value, to
//! go back to), exploded views (parts moved apart in steps, with trails from
//! where they sit), and motion studies (a timeline of joint values played
//! together, exported as frames).

use crate::couplings::joint_ref;
use crate::joints::{self, Joint};
use crate::transform::{self as tf, Transform, V3};
use crate::{AssemblyDef, AssemblyStats, Instance, bad, placement};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_shape::params::ParameterSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

fn entity(world: &World, id: StableId) -> Option<Entity> {
    world.resource::<IdIndex>().get(id)
}

fn check_scope(tx: &Tx, scope: Option<StableId>) -> Result<(), CommandError> {
    if let Some(s) = scope
        && tx.get::<AssemblyDef>(s).is_none()
    {
        return Err(bad(ErrorCode::UnresolvedReference, format!("{s} is not an assembly")));
    }
    Ok(())
}

fn name_taken<T: Component>(world: &World, n: &str, name: impl Fn(&T) -> &str, except: Option<StableId>) -> bool {
    world.resource::<IdIndex>().iter().any(|(id, e)| Some(id) != except && world.get::<T>(e).is_some_and(|x| name(x) == n))
}

/// A number or a list of numbers, for a joint's values.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Values {
    One(f64),
    Many(Vec<f64>),
}

/// Joint values given by joint name or id, filled out to each joint's full list.
pub fn joint_values(world: &World, scope: Option<StableId>, given: &BTreeMap<String, Values>) -> Result<BTreeMap<StableId, Vec<f64>>, String> {
    let mut out = BTreeMap::new();
    for (name, v) in given {
        let j = joint_ref(world, scope, name)?;
        let mut base = joints::link(world, j, None)?.values;
        if base.is_empty() {
            return Err(format!("{} has no value to set", joints::joint_name(world, j)));
        }
        let list = match v {
            Values::One(x) => vec![*x],
            Values::Many(x) => x.clone(),
        };
        for (k, x) in list.into_iter().enumerate() {
            if !x.is_finite() {
                return Err(format!("{}: values must be finite", joints::joint_name(world, j)));
            }
            if k < base.len() {
                base[k] = x;
            }
        }
        out.insert(j, base);
    }
    Ok(out)
}

// ---- Named poses ---------------------------------------------------------------------

/// A named pose: joint values to put the assembly back to ("home", "reach").
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pose {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
    /// Joint → its values (in the order of its kind's keys).
    pub values: BTreeMap<StableId, Vec<f64>>,
}

impl Persist for Pose {
    const KIND: &'static str = "assembly.pose";
}

/// Every pose in an assembly (or at the top), oldest first.
pub fn poses_in(world: &World, scope: Option<StableId>) -> Vec<(StableId, Pose)> {
    let mut out: Vec<(StableId, Pose)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Pose>(e).filter(|p| p.assembly == scope).map(|p| (id, p.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// A pose by id or name in the scope.
pub fn pose_ref(world: &World, scope: Option<StableId>, r: &str) -> Result<(StableId, Pose), String> {
    let list = poses_in(world, scope);
    list.iter()
        .find(|(id, p)| p.name == r || serde_json::from_value::<StableId>(json!(r)).ok() == Some(*id))
        .cloned()
        .ok_or_else(|| format!("no pose “{r}” (there are {})", if list.is_empty() { "none".into() } else { list.iter().map(|(_, p)| p.name.clone()).collect::<Vec<_>>().join(", ") }))
}

/// Every joint with a value that its own parameter sets (not a formula), now.
fn capture(world: &World, scope: Option<StableId>) -> Result<BTreeMap<StableId, Vec<f64>>, String> {
    let mut out = BTreeMap::new();
    for (j, joint) in joints::joints_in(world, scope) {
        if joint.kind.values().is_empty() {
            continue;
        }
        let set = entity(world, j).and_then(|e| world.get::<ParameterSet>(e)).ok_or("a joint has no values")?;
        if set.params.iter().all(|p| p.expr.is_some()) {
            continue;
        }
        out.insert(j, joints::link(world, j, None)?.values);
    }
    Ok(out)
}

/// `assembly.pose.create`: name the joints' values (now, or as given) to come back to.
#[derive(Deserialize, JsonSchema)]
pub struct CreatePose {
    #[serde(default)]
    pub assembly: Option<StableId>,
    pub name: String,
    /// Joint name or id → value (or values); left out: every joint as it is now.
    pub values: Option<BTreeMap<String, Values>>,
}

impl Command for CreatePose {
    const NAME: &'static str = "assembly.pose.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        check_scope(tx, self.assembly)?;
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(bad(ErrorCode::InvalidParameters, "a pose needs a name"));
        }
        if poses_in(tx.world(), self.assembly).iter().any(|(_, p)| p.name == name) {
            return Err(bad(ErrorCode::InvalidParameters, format!("there is already a pose called “{name}”")));
        }
        let values = match &self.values {
            Some(v) => joint_values(tx.world(), self.assembly, v),
            None => capture(tx.world(), self.assembly),
        }
        .map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
        if values.is_empty() {
            return Err(bad(ErrorCode::InvalidParameters, "no joint here has a value to keep"));
        }
        let id = tx.create();
        let joints: Vec<StableId> = values.keys().copied().collect();
        tx.set(id, Pose { name: name.clone(), assembly: self.assembly, values })?;
        for j in joints {
            tx.add_dependency(id, j)?;
        }
        Ok(json!({ "id": id, "name": name }))
    }
}

/// `assembly.pose.update`: rename, change values, or (`capture`) keep the joints as they are now.
#[derive(Deserialize, JsonSchema)]
pub struct UpdatePose {
    pub target: StableId,
    pub name: Option<String>,
    pub values: Option<BTreeMap<String, Values>>,
    #[serde(default)]
    pub capture: bool,
}

impl Command for UpdatePose {
    const NAME: &'static str = "assembly.pose.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut p = tx.get::<Pose>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a pose", self.target)))?;
        if let Some(n) = self.name {
            let n = n.trim().to_string();
            if n.is_empty() || poses_in(tx.world(), p.assembly).iter().any(|(id, q)| *id != self.target && q.name == n) {
                return Err(bad(ErrorCode::InvalidParameters, format!("“{n}” is empty or taken")));
            }
            p.name = n;
        }
        if self.capture {
            p.values = capture(tx.world(), p.assembly).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
        }
        if let Some(v) = &self.values {
            p.values.extend(joint_values(tx.world(), p.assembly, v).map_err(|m| bad(ErrorCode::InvalidParameters, m))?);
        }
        for j in p.values.keys() {
            tx.add_dependency(self.target, *j)?;
        }
        tx.set(self.target, p)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.pose.apply`: set every joint the pose names to its value (one change).
#[derive(Deserialize, JsonSchema)]
pub struct ApplyPose {
    pub target: StableId,
}

/// Sets joints' values in one change; formulas (and couplings' followers) are left to follow.
pub fn set_joints(tx: &mut Tx, values: &BTreeMap<StableId, Vec<f64>>) -> Result<Vec<String>, CommandError> {
    let mut changed = vec![];
    for (j, v) in values {
        let Some(joint) = tx.get::<Joint>(*j).cloned() else { continue };
        let Some(set) = tx.get::<ParameterSet>(*j).cloned() else { continue };
        let mut patch = serde_json::Map::new();
        for (k, (key, _, _)) in joint.kind.values().iter().enumerate() {
            let (Some(x), Some(p)) = (v.get(k), set.params.iter().find(|p| p.key == *key)) else { continue };
            if p.expr.is_none() && (p.value - x).abs() > 1e-12 {
                patch.insert(key.to_string(), json!((x * 1e9).round() / 1e9));
            }
        }
        if !patch.is_empty() {
            tx.run("parameters.set", json!({ "target": j, "values": patch }))?;
            changed.push(set.name.clone());
        }
    }
    Ok(changed)
}

impl Command for ApplyPose {
    const NAME: &'static str = "assembly.pose.apply";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let p = tx.get::<Pose>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a pose", self.target)))?;
        let changed = set_joints(tx, &p.values)?;
        Ok(json!({ "id": self.target, "changed": changed }))
    }
}

/// `assembly.pose.delete`.
#[derive(Deserialize, JsonSchema)]
pub struct DeletePose {
    pub target: StableId,
}

fn drop(tx: &mut Tx, id: StableId) -> Result<Value, CommandError> {
    let users: Vec<StableId> = tx.world().resource::<IdIndex>().iter().filter(|(_, e)| tx.world().get::<de_core::DependsOn>(*e).is_some_and(|d| d.0.contains(&id))).map(|(u, _)| u).collect();
    for u in users {
        tx.remove_dependency(u, id)?;
    }
    tx.delete(id)?;
    Ok(json!({ "id": id }))
}

impl Command for DeletePose {
    const NAME: &'static str = "assembly.pose.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        tx.get::<Pose>(self.target).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a pose", self.target)))?;
        drop(tx, self.target)
    }
}

// ---- Exploded views ---------------------------------------------------------------------

/// A turn in an explode step: degrees about an axis through a point (default
/// each instance's middle).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Turn {
    pub axis: V3,
    pub angle: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<V3>,
}

/// One step of an exploded view: these instances move (mm, assembly
/// coordinates) and turn, all together, after the steps before.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExplodeStep {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub instances: Vec<StableId>,
    #[serde(default, rename = "move")]
    pub by: V3,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
}

/// A saved exploded view of an assembly (or the top).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExplodedView {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
    pub steps: Vec<ExplodeStep>,
}

impl Persist for ExplodedView {
    const KIND: &'static str = "assembly.exploded_view";
}

fn scope_instances(world: &World, scope: Option<StableId>) -> Vec<(StableId, Instance)> {
    let mut out: Vec<(StableId, Instance)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == scope).map(|i| (id, i.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// An instance's box in the assembly (where it is now).
fn placed_box(world: &World, id: StableId) -> Option<(V3, V3)> {
    let e = entity(world, id)?;
    let s = world.get::<AssemblyStats>(e)?;
    let t = placement(world, e)?;
    if s.shapes == 0 {
        return None;
    }
    // Stats of an instance are in its definition's coordinates.
    Some(t.bounds(s.bounds_min, s.bounds_max))
}

fn mid(b: (V3, V3)) -> V3 {
    tf::scale(tf::add(b.0, b.1), 0.5)
}

/// Steps that pull every instance out from the middle along the line through
/// it (`spread` × the assembly's size), last assembled first when instances
/// carry an assembly "step" property.
pub fn auto_steps(world: &World, scope: Option<StableId>, spread: f64) -> Result<Vec<ExplodeStep>, String> {
    let list: Vec<(StableId, Instance, (V3, V3))> = scope_instances(world, scope).into_iter().filter_map(|(id, i)| Some((id, i, placed_box(world, id)?))).collect();
    if list.len() < 2 {
        return Err("an exploded view needs two or more placed instances".into());
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for (_, _, b) in &list {
        for k in 0..3 {
            lo[k] = lo[k].min(b.0[k]);
            hi[k] = hi[k].max(b.1[k]);
        }
    }
    let centre = mid((lo, hi));
    let size = tf::norm(tf::sub(hi, lo)).max(1.0);
    // By assembly step, last first; otherwise all at once.
    let step_of = |i: &Instance| i.properties.get("step").and_then(|s| s.trim().parse::<f64>().ok());
    let mut groups: BTreeMap<i64, Vec<(StableId, V3)>> = BTreeMap::new();
    for (id, inst, b) in &list {
        let out = tf::sub(mid(*b), centre);
        let len = tf::norm(out);
        let dir = if len > size * 0.01 { tf::scale(out, 1.0 / len) } else { [0.0, 0.0, 1.0] };
        let by = tf::scale(dir, spread * (len + size * 0.12));
        let key = step_of(inst).map(|s| -(s * 1000.0).round() as i64).unwrap_or(0);
        groups.entry(key).or_default().push((*id, by));
    }
    let mut steps = vec![];
    for (k, members) in groups {
        // One step per group; each member's own direction is a step of its own when they differ.
        for (id, by) in members {
            steps.push(ExplodeStep { name: if k == 0 { String::new() } else { format!("Step {}", -k as f64 / 1000.0) }, instances: vec![id], by: by.map(|v| (v * 1000.0).round() / 1000.0), turn: None });
        }
    }
    Ok(steps)
}

fn check_steps(tx: &Tx, scope: Option<StableId>, steps: &[ExplodeStep]) -> Result<(), CommandError> {
    for (n, s) in steps.iter().enumerate() {
        if s.instances.is_empty() {
            return Err(bad(ErrorCode::InvalidParameters, format!("step {}: name the instances it moves", n + 1)));
        }
        for i in &s.instances {
            let inst = tx.get::<Instance>(*i).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("step {}: {i} is not an instance", n + 1)))?;
            if inst.parent != scope {
                return Err(bad(ErrorCode::InvalidParameters, format!("step {}: {} is in another assembly", n + 1, inst.name)));
            }
        }
        let ok = s.by.iter().all(|v| v.is_finite()) && s.turn.as_ref().is_none_or(|t| t.angle.is_finite() && t.axis.iter().all(|v| v.is_finite()) && tf::norm(t.axis) > 1e-12 && t.at.is_none_or(|a| a.iter().all(|v| v.is_finite())));
        if !ok {
            return Err(bad(ErrorCode::InvalidParameters, format!("step {}: moves and turns need finite numbers (and an axis)", n + 1)));
        }
    }
    Ok(())
}

/// `assembly.explode.create`: a saved exploded view, from `steps` or worked out (`auto`).
#[derive(Deserialize, JsonSchema)]
pub struct CreateExplode {
    #[serde(default)]
    pub assembly: Option<StableId>,
    pub name: String,
    #[serde(default)]
    pub steps: Vec<ExplodeStep>,
    /// Work the steps out: every part pulled out from the middle, `spread` × the assembly's size (default 1).
    pub auto: Option<AutoExplode>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AutoExplode {
    #[serde(default = "one")]
    pub spread: f64,
}

fn one() -> f64 {
    1.0
}

impl Command for CreateExplode {
    const NAME: &'static str = "assembly.explode.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        check_scope(tx, self.assembly)?;
        let name = self.name.trim().to_string();
        if name.is_empty() || name_taken::<ExplodedView>(tx.world(), &name, |v| &v.name, None) {
            return Err(bad(ErrorCode::InvalidParameters, format!("an exploded view needs a name not taken (“{name}”)")));
        }
        let mut steps = self.steps;
        if let Some(a) = self.auto {
            if !(a.spread.is_finite() && a.spread > 0.0) {
                return Err(bad(ErrorCode::InvalidParameters, "spread must be a positive number"));
            }
            steps.extend(auto_steps(tx.world(), self.assembly, a.spread).map_err(|m| bad(ErrorCode::InvalidParameters, m))?);
        }
        if steps.is_empty() {
            return Err(bad(ErrorCode::InvalidParameters, "give `steps` (instances, move, turn) or `auto: { spread }`"));
        }
        check_steps(tx, self.assembly, &steps)?;
        let id = tx.create();
        let ids: Vec<StableId> = steps.iter().flat_map(|s| s.instances.clone()).collect();
        tx.set(id, ExplodedView { name: name.clone(), assembly: self.assembly, steps })?;
        for i in ids {
            tx.add_dependency(id, i)?;
        }
        Ok(json!({ "id": id, "name": name }))
    }
}

/// `assembly.explode.update`: rename, or replace the steps.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateExplode {
    pub target: StableId,
    pub name: Option<String>,
    pub steps: Option<Vec<ExplodeStep>>,
}

impl Command for UpdateExplode {
    const NAME: &'static str = "assembly.explode.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut v = tx.get::<ExplodedView>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an exploded view", self.target)))?;
        if let Some(n) = self.name {
            let n = n.trim().to_string();
            if n.is_empty() || name_taken::<ExplodedView>(tx.world(), &n, |v| &v.name, Some(self.target)) {
                return Err(bad(ErrorCode::InvalidParameters, format!("“{n}” is empty or taken")));
            }
            v.name = n;
        }
        if let Some(s) = self.steps {
            check_steps(tx, v.assembly, &s)?;
            v.steps = s;
        }
        for i in v.steps.iter().flat_map(|s| s.instances.clone()).collect::<Vec<_>>() {
            tx.add_dependency(self.target, i)?;
        }
        tx.set(self.target, v)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.explode.delete`.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteExplode {
    pub target: StableId,
}
impl Command for DeleteExplode {
    const NAME: &'static str = "assembly.explode.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        tx.get::<ExplodedView>(self.target).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an exploded view", self.target)))?;
        drop(tx, self.target)
    }
}

/// An exploded view worked out: where every instance is after each step, and
/// the trail each one leaves.
#[derive(Clone, Debug, Serialize)]
pub struct Exploded {
    pub id: StableId,
    pub name: String,
    pub assembly: Option<StableId>,
    pub instances: Vec<crate::motion::SweptInstance>,
    /// Step 0 is the assembly as it is; step k is after k steps. Each:
    /// every instance's position (mm) and rotation quaternion [x, y, z, w].
    pub steps: Vec<ExplodedStep>,
    /// Each moved instance's path from where it sits through every step (its middle).
    pub trails: Vec<Trail>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExplodedStep {
    pub name: String,
    pub poses: Vec<([f64; 3], [f64; 4])>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Trail {
    pub instance: StableId,
    pub points: Vec<V3>,
}

pub fn explode(world: &World, id: StableId) -> Result<Exploded, String> {
    let e = entity(world, id).ok_or("no such exploded view")?;
    let v = world.get::<ExplodedView>(e).ok_or_else(|| format!("{id} is not an exploded view"))?;
    let list = scope_instances(world, v.assembly);
    let mut now: Vec<Transform> = list.iter().map(|(i, _)| entity(world, *i).and_then(|e| placement(world, e)).unwrap_or(Transform::IDENTITY)).collect();
    let middles: Vec<V3> = list.iter().map(|(i, _)| placed_box(world, *i).map(mid).unwrap_or([0.0; 3])).collect();
    let r6 = |x: f64| (x * 1e6).round() / 1e6;
    let pose = |t: &[Transform]| t.iter().map(|t| (t.at.map(r6), t.quaternion())).collect::<Vec<_>>();
    let mut steps = vec![ExplodedStep { name: "Assembled".into(), poses: pose(&now) }];
    let mut trails: Vec<Vec<V3>> = middles.iter().map(|m| vec![*m]).collect();
    let mut moved = vec![false; list.len()];
    for (n, s) in v.steps.iter().enumerate() {
        for i in &s.instances {
            let Some(k) = list.iter().position(|(x, _)| x == i) else { continue };
            let m = *trails[k].last().unwrap(); // its middle, now
            let mut next = now[k];
            let mut middle = m;
            if let Some(turn) = &s.turn {
                let r = tf::rotation_about(crate::geometry::unit(turn.axis), turn.angle.to_radians());
                let c = turn.at.unwrap_or(m);
                let rot = Transform { at: tf::sub(c, tf::mul(&r, c)), rot: r };
                next = rot.then(&next);
                middle = rot.apply(middle);
            }
            next = Transform::translation(s.by).then(&next);
            middle = tf::add(middle, s.by);
            now[k] = next;
            trails[k].push(middle);
            moved[k] = true;
        }
        steps.push(ExplodedStep { name: if s.name.is_empty() { format!("Step {}", n + 1) } else { s.name.clone() }, poses: pose(&now) });
    }
    Ok(Exploded {
        id,
        name: v.name.clone(),
        assembly: v.assembly,
        instances: list.iter().enumerate().map(|(k, (i, inst))| crate::motion::SweptInstance { id: *i, name: inst.name.clone(), moves: moved[k] }).collect(),
        steps,
        trails: list.iter().zip(trails).zip(&moved).filter(|(_, m)| **m).map(|(((i, _), points), _)| Trail { instance: *i, points: points.into_iter().map(|p| p.map(r6)).collect() }).collect(),
    })
}

// ---- Motion studies ---------------------------------------------------------------------

/// One joint's values over time: [seconds, value] keys, eased between.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    pub joint: StableId,
    /// Which of its values (default its main one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub keys: Vec<[f64; 2]>,
}

/// A motion study: several joints moving on one timeline.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MotionStudy {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
    /// Seconds.
    pub duration: f64,
    pub tracks: Vec<Track>,
    /// Straight between keys (default: eased in and out).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub linear: bool,
}

impl Persist for MotionStudy {
    const KIND: &'static str = "assembly.motion_study";
}

/// A move of one joint in a study: to a value, over `duration` seconds from `start`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct StudyStep {
    pub joint: String,
    pub key: Option<String>,
    /// Where it starts (default: where the step before left it, else its value now).
    pub from: Option<f64>,
    pub to: f64,
    /// Seconds from the start of the study (default: when the step before ends).
    pub start: Option<f64>,
    pub duration: f64,
}

/// A pose reached at a time.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct PoseKey {
    pub pose: String,
    pub at: f64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct TrackIn {
    pub joint: String,
    pub key: Option<String>,
    pub keys: Vec<[f64; 2]>,
}

/// `assembly.motion_study.create`: a timeline of joint motion: `steps`
/// (one joint to a value over a time), `poses` (named poses reached at
/// times), or `tracks` of [seconds, value] keys — or several of these.
#[derive(Deserialize, JsonSchema)]
pub struct CreateStudy {
    #[serde(default)]
    pub assembly: Option<StableId>,
    pub name: String,
    #[serde(default)]
    pub steps: Vec<StudyStep>,
    #[serde(default)]
    pub poses: Vec<PoseKey>,
    #[serde(default)]
    pub tracks: Vec<TrackIn>,
    /// Seconds (default: when the last key is).
    pub duration: Option<f64>,
    #[serde(default)]
    pub linear: bool,
}

fn key_of(world: &World, joint: StableId, key: Option<&str>) -> Result<usize, String> {
    let kind = entity(world, joint).and_then(|e| world.get::<Joint>(e)).map(|j| j.kind).ok_or("not a joint")?;
    let keys = kind.values();
    if keys.is_empty() {
        return Err(format!("{} has no value to move", joints::joint_name(world, joint)));
    }
    match key {
        None => Ok(0),
        Some(k) => keys.iter().position(|(x, _, _)| *x == k).ok_or_else(|| format!("{} has no value “{k}”", joints::joint_name(world, joint))),
    }
}

/// Builds the study's tracks from what was given.
#[allow(clippy::too_many_arguments)]
fn build_tracks(world: &World, scope: Option<StableId>, steps: &[StudyStep], poses: &[PoseKey], tracks: &[TrackIn]) -> Result<Vec<Track>, String> {
    let mut out: Vec<Track> = vec![];
    let track = |out: &mut Vec<Track>, joint: StableId, key: Option<String>| -> usize {
        match out.iter().position(|t| t.joint == joint && t.key == key) {
            Some(i) => i,
            None => {
                out.push(Track { joint, key, keys: vec![] });
                out.len() - 1
            }
        }
    };
    for t in tracks {
        let j = joint_ref(world, scope, &t.joint)?;
        key_of(world, j, t.key.as_deref())?;
        let i = track(&mut out, j, t.key.clone());
        out[i].keys.extend(t.keys.iter().copied());
    }
    for pk in poses {
        let (_, p) = pose_ref(world, scope, &pk.pose)?;
        for (j, v) in &p.values {
            let kind = entity(world, *j).and_then(|e| world.get::<Joint>(e)).map(|x| x.kind).ok_or("a pose's joint is gone")?;
            for (k, (key, _, _)) in kind.values().iter().enumerate() {
                let Some(x) = v.get(k) else { continue };
                let i = track(&mut out, *j, if k == 0 { None } else { Some(key.to_string()) });
                out[i].keys.push([pk.at, *x]);
            }
        }
    }
    let mut clock = 0.0f64;
    for s in steps {
        let j = joint_ref(world, scope, &s.joint)?;
        let k = key_of(world, j, s.key.as_deref())?;
        if !(s.duration.is_finite() && s.duration >= 0.0 && s.to.is_finite()) {
            return Err(format!("{}: a step needs a finite `to` and a duration of 0 s or more", s.joint));
        }
        let start = s.start.unwrap_or(clock);
        let i = track(&mut out, j, s.key.clone());
        let before = out[i].keys.iter().filter(|kk| kk[0] <= start).max_by(|a, b| a[0].total_cmp(&b[0])).map(|kk| kk[1]);
        let from = match s.from.or(before) {
            Some(f) => f,
            None => joints::link(world, j, None)?.values.get(k).copied().unwrap_or(0.0),
        };
        out[i].keys.push([start, from]);
        out[i].keys.push([start + s.duration, s.to]);
        clock = start + s.duration;
    }
    for t in &mut out {
        if t.keys.iter().any(|k| !k[0].is_finite() || !k[1].is_finite() || k[0] < 0.0) {
            return Err("keys need finite times (0 s or later) and values".into());
        }
        t.keys.sort_by(|a, b| a[0].total_cmp(&b[0]));
        t.keys.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12);
    }
    out.retain(|t| !t.keys.is_empty());
    Ok(out)
}

impl Command for CreateStudy {
    const NAME: &'static str = "assembly.motion_study.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        check_scope(tx, self.assembly)?;
        let name = self.name.trim().to_string();
        if name.is_empty() || name_taken::<MotionStudy>(tx.world(), &name, |v| &v.name, None) {
            return Err(bad(ErrorCode::InvalidParameters, format!("a motion study needs a name not taken (“{name}”)")));
        }
        let tracks = build_tracks(tx.world(), self.assembly, &self.steps, &self.poses, &self.tracks).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
        if tracks.is_empty() {
            return Err(bad(ErrorCode::InvalidParameters, "give `steps`, `poses` or `tracks`: what moves, and when"));
        }
        let end = tracks.iter().flat_map(|t| t.keys.iter().map(|k| k[0])).fold(0.0, f64::max);
        let duration = self.duration.unwrap_or(end);
        if !(duration.is_finite() && duration > 0.0) {
            return Err(bad(ErrorCode::InvalidParameters, "a study lasts some time: give a duration (s) or keys later than 0 s"));
        }
        let id = tx.create();
        let joints: Vec<StableId> = tracks.iter().map(|t| t.joint).collect();
        tx.set(id, MotionStudy { name: name.clone(), assembly: self.assembly, duration, tracks, linear: self.linear })?;
        for j in joints {
            tx.add_dependency(id, j)?;
        }
        Ok(json!({ "id": id, "name": name, "duration": duration }))
    }
}

/// `assembly.motion_study.update`: rename, change the duration, or replace what moves.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateStudy {
    pub target: StableId,
    pub name: Option<String>,
    pub duration: Option<f64>,
    pub linear: Option<bool>,
    pub steps: Option<Vec<StudyStep>>,
    pub poses: Option<Vec<PoseKey>>,
    pub tracks: Option<Vec<TrackIn>>,
}

impl Command for UpdateStudy {
    const NAME: &'static str = "assembly.motion_study.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = tx.get::<MotionStudy>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a motion study", self.target)))?;
        if let Some(n) = self.name {
            let n = n.trim().to_string();
            if n.is_empty() || name_taken::<MotionStudy>(tx.world(), &n, |v| &v.name, Some(self.target)) {
                return Err(bad(ErrorCode::InvalidParameters, format!("“{n}” is empty or taken")));
            }
            s.name = n;
        }
        if self.steps.is_some() || self.poses.is_some() || self.tracks.is_some() {
            s.tracks = build_tracks(tx.world(), s.assembly, &self.steps.unwrap_or_default(), &self.poses.unwrap_or_default(), &self.tracks.unwrap_or_default()).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
            if s.tracks.is_empty() {
                return Err(bad(ErrorCode::InvalidParameters, "a study needs something that moves"));
            }
            s.duration = s.tracks.iter().flat_map(|t| t.keys.iter().map(|k| k[0])).fold(0.0, f64::max).max(1e-3);
        }
        if let Some(d) = self.duration {
            if !(d.is_finite() && d > 0.0) {
                return Err(bad(ErrorCode::InvalidParameters, "the duration must be a positive number of seconds"));
            }
            s.duration = d;
        }
        if let Some(l) = self.linear {
            s.linear = l;
        }
        for j in s.tracks.iter().map(|t| t.joint).collect::<Vec<_>>() {
            tx.add_dependency(self.target, j)?;
        }
        tx.set(self.target, s)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.motion_study.delete`.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteStudy {
    pub target: StableId,
}
impl Command for DeleteStudy {
    const NAME: &'static str = "assembly.motion_study.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        tx.get::<MotionStudy>(self.target).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a motion study", self.target)))?;
        drop(tx, self.target)
    }
}

/// A track's value at time `t`.
pub fn track_at(keys: &[[f64; 2]], t: f64, linear: bool) -> f64 {
    let Some(first) = keys.first() else { return 0.0 };
    if t <= first[0] {
        return first[1];
    }
    for w in keys.windows(2) {
        if t <= w[1][0] {
            let span = w[1][0] - w[0][0];
            let x = if span > 0.0 { (t - w[0][0]) / span } else { 1.0 };
            let x = if linear { x } else { x * x * (3.0 - 2.0 * x) };
            return w[0][1] + (w[1][1] - w[0][1]) * x;
        }
    }
    keys[keys.len() - 1][1]
}

/// A study played at `fps` frames a second: every instance's placement at
/// each frame, the joints' values, and (with `gaps`) the clashes.
pub fn play_study(world: &World, id: StableId, fps: f64, gaps: bool) -> Result<crate::motion::Played, String> {
    let e = entity(world, id).ok_or("no such motion study")?;
    let s = world.get::<MotionStudy>(e).ok_or_else(|| format!("{id} is not a motion study"))?.clone();
    let fps = if fps.is_finite() && fps > 0.0 { fps.min(120.0) } else { 24.0 };
    let n = ((s.duration * fps).ceil() as usize).clamp(1, 7200);
    let mut base: HashMap<StableId, Vec<f64>> = HashMap::new();
    for t in &s.tracks {
        if !base.contains_key(&t.joint) {
            base.insert(t.joint, joints::link(world, t.joint, None)?.values);
        }
    }
    let frames: Vec<(f64, HashMap<StableId, Vec<f64>>)> = (0..=n)
        .map(|k| {
            let time = s.duration * k as f64 / n as f64;
            let mut values = base.clone();
            for t in &s.tracks {
                let i = key_of(world, t.joint, t.key.as_deref()).unwrap_or(0);
                if let Some(v) = values.get_mut(&t.joint)
                    && i < v.len()
                {
                    v[i] = track_at(&t.keys, time, s.linear);
                }
            }
            (time, values)
        })
        .collect();
    crate::motion::play(world, s.assembly, &frames, gaps, "s", &s.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_ease_between_keys() {
        let k = [[0.0, 0.0], [2.0, 90.0], [3.0, 90.0]];
        assert_eq!(track_at(&k, -1.0, false), 0.0);
        assert_eq!(track_at(&k, 1.0, false), 45.0);
        assert!(track_at(&k, 0.5, false) < 22.5);
        assert_eq!(track_at(&k, 0.5, true), 22.5);
        assert_eq!(track_at(&k, 9.0, false), 90.0);
    }
}
