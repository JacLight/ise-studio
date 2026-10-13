//! Patterns of instances: copies of a seed instance in a row (or a grid),
//! round an axis, or along a path, that stay a pattern — change the count or
//! the spacing and the copies follow; move the seed (or let its joints move
//! it) and they all follow; swap the seed's definition or configuration and
//! they show it too. Copies are ordinary instances of the same definition,
//! so the BOM counts them as one line.
//!
//! A pattern is also a parameter set named for it: `spacing` (mm, rows) and
//! `spacing2` (mm, columns) of a linear pattern, `angle` (degrees between
//! copies) of a circular one. Scrub them or link them by formula
//! ("=Rail.length / 6") like any parameter. The count is the pattern's own.

use crate::geometry::unit;
use crate::transform::{self as tf, Transform, V3};
use crate::{AssemblyDef, Instance, bad};
use de_core::{
    Command, CommandError, DependsOn, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_shape::{Param, params::ParameterSet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// In a row along `direction`, `spacing` apart (and in columns along `direction2`).
    Linear,
    /// Round `axis` through `center`, `angle` apart.
    Circular,
    /// Along a path (a polyline through points), spread evenly.
    Path,
}

/// A pattern (on an entity that is also the parameter set holding its spacing).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pattern {
    pub kind: PatternKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
    /// The instance copied.
    pub seed: StableId,
    /// The copies, in order.
    pub members: Vec<StableId>,
    /// How many along the first direction (the seed counts).
    pub count: usize,
    /// How many along the second direction of a linear grid (1: a single row).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub count2: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<V3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction2: Option<V3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<V3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<V3>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<V3>,
    /// The path closes on itself (copies spread round the loop).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    /// Copies along a path turn with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub align: bool,
}

fn one() -> usize {
    1
}
fn is_one(v: &usize) -> bool {
    *v == 1
}

impl Persist for Pattern {
    const KIND: &'static str = "assembly.pattern";
}

fn entity(world: &World, id: StableId) -> Option<Entity> {
    world.resource::<IdIndex>().get(id)
}

/// The pattern an instance is a copy in, if any.
pub fn pattern_of(world: &World, instance: StableId) -> Option<(StableId, Pattern)> {
    let e = entity(world, instance)?;
    let deps = world.get::<DependsOn>(e)?;
    deps.0.iter().find_map(|d| {
        let p = world.get::<Pattern>(entity(world, *d)?)?;
        p.members.contains(&instance).then(|| (*d, p.clone()))
    })
}

/// Why an instance cannot be moved or joined, if a pattern places it.
pub fn placed_by_pattern(world: &World, instance: StableId) -> Option<String> {
    let (id, _) = pattern_of(world, instance)?;
    let name = entity(world, id).and_then(|e| world.get::<ParameterSet>(e)).map(|s| s.name.clone()).unwrap_or_else(|| "a pattern".into());
    Some(format!("it is a copy in the pattern “{name}”: move or join the pattern's seed (the copies follow), change the pattern, or delete it to free the copies"))
}

/// Patterns whose seed is this instance.
pub fn patterns_seeded_by(world: &World, seed: StableId) -> Vec<(StableId, Pattern)> {
    let mut out: Vec<(StableId, Pattern)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Pattern>(e).filter(|p| p.seed == seed).map(|p| (id, p.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// Points along a path at arc lengths `s` (and the path's direction there).
fn along(path: &[V3], closed: bool, s: f64) -> (V3, V3) {
    let mut pts = path.to_vec();
    if closed {
        pts.push(path[0]);
    }
    let mut left = s.max(0.0);
    for w in pts.windows(2) {
        let seg = tf::sub(w[1], w[0]);
        let len = tf::norm(seg);
        if len <= 0.0 {
            continue;
        }
        if left <= len {
            return (tf::add(w[0], tf::scale(seg, left / len)), tf::scale(seg, 1.0 / len));
        }
        left -= len;
    }
    let n = pts.len();
    let last = tf::sub(pts[n - 1], pts[n - 2]);
    (pts[n - 1], unit(last))
}

fn length(path: &[V3], closed: bool) -> f64 {
    let mut l: f64 = path.windows(2).map(|w| tf::norm(tf::sub(w[1], w[0]))).sum();
    if closed && path.len() > 2 {
        l += tf::norm(tf::sub(path[0], path[path.len() - 1]));
    }
    l
}

/// The rotation turning unit `a` onto unit `b` the shortest way.
fn turn_onto(a: V3, b: V3) -> tf::M3 {
    let c = tf::cross(a, b);
    let s = tf::norm(c);
    let d = tf::dot(a, b).clamp(-1.0, 1.0);
    if s < 1e-12 {
        if d > 0.0 {
            return tf::I3;
        }
        return tf::rotation_about(crate::geometry::square_x(a), std::f64::consts::PI);
    }
    tf::rotation_about(tf::scale(c, 1.0 / s), s.atan2(d))
}

/// Each copy's move in the assembly (copy k's placement = move · seed's), in member order.
pub fn moves(p: &Pattern, params: &BTreeMap<String, f64>) -> Result<Vec<Transform>, String> {
    let get = |k: &str| params.get(k).copied().unwrap_or(0.0);
    let mut out = vec![];
    match p.kind {
        PatternKind::Linear => {
            let d = unit(p.direction.unwrap_or([1.0, 0.0, 0.0]));
            let d2 = unit(p.direction2.unwrap_or([0.0, 1.0, 0.0]));
            for j in 0..p.count2.max(1) {
                for i in 0..p.count.max(1) {
                    if i == 0 && j == 0 {
                        continue;
                    }
                    out.push(Transform::translation(tf::add(tf::scale(d, get("spacing") * i as f64), tf::scale(d2, get("spacing2") * j as f64))));
                }
            }
        }
        PatternKind::Circular => {
            let axis = unit(p.axis.unwrap_or([0.0, 0.0, 1.0]));
            let c = p.center.unwrap_or([0.0; 3]);
            for i in 1..p.count.max(1) {
                let r = tf::rotation_about(axis, (get("angle") * i as f64).to_radians());
                // About the centre: p → R (p − c) + c.
                out.push(Transform { at: tf::sub(c, tf::mul(&r, c)), rot: r });
            }
        }
        PatternKind::Path => {
            if p.path.len() < 2 {
                return Err("a path pattern needs two or more points".into());
            }
            let len = length(&p.path, p.closed);
            if len <= 0.0 {
                return Err("the path has no length".into());
            }
            let n = p.count.max(1);
            let step = if p.closed { len / n as f64 } else if n > 1 { len / (n - 1) as f64 } else { 0.0 };
            let (q0, t0) = along(&p.path, p.closed, 0.0);
            for i in 1..n {
                let (q, t) = along(&p.path, p.closed, step * i as f64);
                let r = if p.align { turn_onto(t0, t) } else { tf::I3 };
                // The seed's point at the path's start goes to q, turned with the path.
                out.push(Transform { at: tf::sub(q, tf::mul(&r, q0)), rot: r });
            }
        }
    }
    Ok(out)
}

/// Derived placement of a pattern copy (None: not a copy).
pub fn member_placement(world: &World, e: Entity) -> Option<Result<Transform, String>> {
    let id = *world.get::<StableId>(e)?;
    let (pid, p) = pattern_of(world, id)?;
    Some((|| {
        let pe = entity(world, pid).ok_or("its pattern is gone")?;
        let set = world.get::<ParameterSet>(pe).ok_or("its pattern has no values")?;
        let params = de_shape::params::resolve(world, &set.params).map_err(|m| format!("{}: {m}", set.name))?;
        let se = entity(world, p.seed).ok_or("the pattern's seed is gone")?;
        let seed = crate::placement(world, se).ok_or("the pattern's seed is not placed")?;
        let k = p.members.iter().position(|m| *m == id).ok_or("not in its pattern")?;
        let all = moves(&p, &params)?;
        let m = all.get(k).ok_or("the pattern has fewer places than copies")?;
        Ok(m.then(&seed))
    })())
}

/// Where a pattern copy is when the instances are at `posed` (its seed moving
/// in a sweep): None when it is not a copy.
pub fn member_at(world: &World, id: StableId, posed: &std::collections::HashMap<StableId, Transform>) -> Option<Transform> {
    let (pid, p) = pattern_of(world, id)?;
    let seed = posed.get(&p.seed).copied().or_else(|| crate::placement(world, entity(world, p.seed)?))?;
    let set = world.get::<ParameterSet>(entity(world, pid)?)?;
    let params = de_shape::params::resolve(world, &set.params).ok()?;
    let k = p.members.iter().position(|m| *m == id)?;
    Some(moves(&p, &params).ok()?.get(k)?.then(&seed))
}

// ---- Commands ------------------------------------------------------------------------

fn param(key: &str, label: &str, unit: &str, v: Option<ValueIn>) -> Param {
    let (value, expr) = match v {
        Some(ValueIn::Formula(f)) if f.trim().starts_with('=') => (0.0, Some(f.trim().to_string())),
        Some(ValueIn::Formula(f)) => (f.trim().parse().unwrap_or(0.0), None),
        Some(ValueIn::Number(n)) => (n, None),
        None => (0.0, None),
    };
    Param { key: key.into(), label: label.into(), value, unit: unit.into(), min: None, max: None, step: Some(if unit == "deg" { 1.0 } else { 0.5 }), expr }
}

pub use crate::joints::ValueIn;

/// `assembly.pattern.create`: copies of an instance in a row or grid, round an axis, or along a path.
#[derive(Deserialize, JsonSchema)]
pub struct CreatePattern {
    /// The instance copied (its assembly is the pattern's).
    pub seed: StableId,
    #[serde(rename = "type")]
    pub kind: PatternKind,
    pub name: Option<String>,
    /// How many, the seed included (along the first direction).
    pub count: usize,
    /// Linear: mm between copies (number or "=formula"), along `direction` (default X).
    pub spacing: Option<ValueIn>,
    pub direction: Option<V3>,
    /// Linear grid: how many columns, how far apart, which way (default Y).
    pub count2: Option<usize>,
    pub spacing2: Option<ValueIn>,
    pub direction2: Option<V3>,
    /// Circular: degrees between copies; or `span` to spread them (360: all round).
    pub angle: Option<ValueIn>,
    pub span: Option<f64>,
    /// Circular: the axis (default Z) and a point on it (default the origin).
    pub axis: Option<V3>,
    pub center: Option<V3>,
    /// Path: points (assembly coordinates) the copies spread along.
    #[serde(default)]
    pub path: Vec<V3>,
    #[serde(default)]
    pub closed: bool,
    /// Path: copies turn with the path.
    #[serde(default)]
    pub align: bool,
}

const MAX_COPIES: usize = 2000;

fn check_shape(p: &Pattern) -> Result<(), CommandError> {
    let bad_v = |v: &Option<V3>| v.is_some_and(|v| !v.iter().all(|x| x.is_finite()) || tf::norm(v) < 1e-12);
    if p.count < 1 || p.count2 < 1 {
        return Err(bad(ErrorCode::InvalidParameters, "a pattern needs a count of one or more"));
    }
    if p.count * p.count2 > MAX_COPIES {
        return Err(bad(ErrorCode::InvalidParameters, format!("{} copies is too many for one pattern (up to {MAX_COPIES})", p.count * p.count2)));
    }
    if bad_v(&p.direction) || bad_v(&p.direction2) || bad_v(&p.axis) || p.center.is_some_and(|c| !c.iter().all(|x| x.is_finite())) {
        return Err(bad(ErrorCode::InvalidParameters, "directions and axes need finite numbers and a length"));
    }
    if p.kind == PatternKind::Path && (p.path.len() < 2 || p.path.iter().any(|q| !q.iter().all(|x| x.is_finite())) || length(&p.path, p.closed) <= 0.0) {
        return Err(bad(ErrorCode::InvalidParameters, "a path pattern needs a `path` of two or more points, some way apart"));
    }
    Ok(())
}

fn unique_name(world: &World, wanted: Option<String>, seed_name: &str, except: Option<StableId>) -> Result<String, CommandError> {
    let taken = |n: &str| world.resource::<IdIndex>().iter().any(|(id, e)| Some(id) != except && world.get::<ParameterSet>(e).is_some_and(|s| s.name == n));
    match wanted.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        Some(n) if n.contains('.') => Err(bad(ErrorCode::InvalidParameters, "a pattern's name has no dots (formulas read it as Name.spacing)")),
        Some(n) if taken(&n) => Err(bad(ErrorCode::InvalidParameters, format!("there is already a parameter set called “{n}”"))),
        Some(n) => Ok(n),
        None => {
            let base: String = format!("{seed_name} pattern").chars().filter(|c| *c != '.').collect();
            Ok(std::iter::once(base.clone()).chain((2..).map(|i| format!("{base} {i}"))).find(|n| !taken(n)).unwrap())
        }
    }
}

/// Adds or removes copies so the pattern has one per place.
fn reconcile(tx: &mut Tx, id: StableId, mut p: Pattern) -> Result<Pattern, CommandError> {
    let want = p.count * p.count2 - 1;
    let seed = tx.get::<Instance>(p.seed).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, "the pattern's seed is gone"))?;
    while p.members.len() > want {
        let m = p.members.pop().unwrap();
        remove_member(tx, m)?;
    }
    while p.members.len() < want {
        let k = p.members.len() + 2;
        let mut props = seed.properties.clone();
        props.remove("step");
        let m = tx.create();
        tx.set(m, Instance { name: format!("{} {k}", seed.name), of: seed.of, parent: seed.parent, at: seed.at, rotation: seed.rotation, properties: props, config: seed.config.clone(), params: seed.params.clone() })?;
        for r in de_shape::configs::override_references(&seed.params).into_iter().filter(|r| *r != seed.of) {
            tx.add_dependency(m, r)?;
        }
        tx.add_dependency(m, seed.of)?;
        if let Some(parent) = seed.parent {
            tx.add_dependency(parent, m)?;
        }
        tx.add_dependency(m, id)?;
        p.members.push(m);
    }
    Ok(p)
}

fn remove_member(tx: &mut Tx, m: StableId) -> Result<(), CommandError> {
    let users: Vec<StableId> = tx.world().resource::<IdIndex>().iter().filter(|(_, e)| tx.world().get::<DependsOn>(*e).is_some_and(|d| d.0.contains(&m))).map(|(u, _)| u).collect();
    for u in users {
        tx.remove_dependency(u, m)?;
    }
    tx.delete(m)
}

impl Command for CreatePattern {
    const NAME: &'static str = "assembly.pattern.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let seed = tx.get::<Instance>(self.seed).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an instance", self.seed)))?;
        if let Some(why) = placed_by_pattern(tx.world(), self.seed) {
            return Err(bad(ErrorCode::InvalidParameters, format!("{} {why}", seed.name)));
        }
        if let Some(parent) = seed.parent
            && tx.get::<AssemblyDef>(parent).is_none()
        {
            return Err(bad(ErrorCode::UnresolvedReference, "the seed's assembly is gone"));
        }
        let span_angle = |count: usize| -> Option<ValueIn> {
            let s = self.span?;
            let full = (s.abs() - 360.0).abs() < 1e-9;
            let gaps = if full { count } else { count.saturating_sub(1).max(1) };
            Some(ValueIn::Number(s / gaps as f64))
        };
        let params = match self.kind {
            PatternKind::Linear => {
                let mut v = vec![param("spacing", "Spacing", "mm", Some(self.spacing.ok_or_else(|| bad(ErrorCode::InvalidParameters, "a linear pattern needs its `spacing` (mm)"))?))];
                if self.count2.unwrap_or(1) > 1 {
                    v.push(param("spacing2", "Column spacing", "mm", Some(self.spacing2.ok_or_else(|| bad(ErrorCode::InvalidParameters, "a grid needs `spacing2` between columns"))?)));
                }
                v
            }
            PatternKind::Circular => vec![param("angle", "Angle", "deg", Some(self.angle.or_else(|| span_angle(self.count)).unwrap_or(ValueIn::Number(360.0 / self.count.max(1) as f64))))],
            PatternKind::Path => vec![],
        };
        let p = Pattern {
            kind: self.kind,
            assembly: seed.parent,
            seed: self.seed,
            members: vec![],
            count: self.count,
            count2: if self.kind == PatternKind::Linear { self.count2.unwrap_or(1) } else { 1 },
            direction: self.direction,
            direction2: self.direction2,
            axis: self.axis,
            center: self.center,
            path: self.path,
            closed: self.closed,
            align: self.align,
        };
        check_shape(&p)?;
        let name = unique_name(tx.world(), self.name, &seed.name, None)?;
        let out = tx.run("parameters.create", json!({ "name": name, "description": format!("{} pattern of {}", match p.kind { PatternKind::Linear => "Linear", PatternKind::Circular => "Circular", PatternKind::Path => "Path" }, seed.name), "params": params }))?;
        let id: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| bad(ErrorCode::InvalidParameters, e.to_string()))?;
        tx.add_dependency(id, self.seed)?;
        let p = reconcile(tx, id, p)?;
        let members = p.members.clone();
        tx.set(id, p)?;
        Ok(json!({ "id": id, "name": name, "members": members }))
    }
}

/// `assembly.pattern.update`: change the count, directions, axis, path, name;
/// spacing and angle too (or through `parameters.set` on the pattern).
#[derive(Deserialize, JsonSchema)]
pub struct UpdatePattern {
    pub target: StableId,
    pub name: Option<String>,
    pub count: Option<usize>,
    pub count2: Option<usize>,
    pub spacing: Option<ValueIn>,
    pub spacing2: Option<ValueIn>,
    pub angle: Option<ValueIn>,
    pub direction: Option<V3>,
    pub direction2: Option<V3>,
    pub axis: Option<V3>,
    pub center: Option<V3>,
    pub path: Option<Vec<V3>>,
    pub closed: Option<bool>,
    pub align: Option<bool>,
}

impl Command for UpdatePattern {
    const NAME: &'static str = "assembly.pattern.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut p = tx.get::<Pattern>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a pattern", self.target)))?;
        let mut set = tx.get::<ParameterSet>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, "the pattern has no values"))?;
        if let Some(c) = self.count {
            p.count = c;
        }
        if let Some(c) = self.count2 {
            p.count2 = c;
        }
        p.direction = self.direction.or(p.direction);
        p.direction2 = self.direction2.or(p.direction2);
        p.axis = self.axis.or(p.axis);
        p.center = self.center.or(p.center);
        if let Some(path) = self.path {
            p.path = path;
        }
        p.closed = self.closed.unwrap_or(p.closed);
        p.align = self.align.unwrap_or(p.align);
        check_shape(&p)?;
        if let Some(n) = self.name {
            let seed_name = tx.get::<Instance>(p.seed).map(|i| i.name.clone()).unwrap_or_default();
            set.name = unique_name(tx.world(), Some(n), &seed_name, Some(self.target))?;
            tx.set(self.target, set.clone())?;
        }
        for (key, label, unit, v) in [("spacing", "Spacing", "mm", self.spacing), ("spacing2", "Column spacing", "mm", self.spacing2), ("angle", "Angle", "deg", self.angle)] {
            let Some(v) = v else { continue };
            let new = param(key, label, unit, Some(v));
            let mut list = set.params.clone();
            match list.iter_mut().find(|q| q.key == key) {
                Some(q) => {
                    q.value = new.value;
                    q.expr = None;
                }
                None => list.push(Param { expr: None, ..new.clone() }),
            }
            if list != set.params {
                if set.params.iter().any(|q| q.key == key && q.expr.is_some()) {
                    tx.run("shape.link_param", json!({ "target": self.target, "key": key, "expr": null }))?;
                    set = tx.get::<ParameterSet>(self.target).cloned().unwrap();
                    list = set.params.clone();
                    if let Some(q) = list.iter_mut().find(|q| q.key == key) {
                        q.value = new.value;
                    }
                }
                set.params = list;
                tx.set(self.target, set.clone())?;
            }
            if let Some(e) = new.expr {
                tx.run("shape.link_param", json!({ "target": self.target, "key": key, "expr": e }))?;
                set = tx.get::<ParameterSet>(self.target).cloned().unwrap();
            }
        }
        if p.kind == PatternKind::Linear && p.count2 > 1 && !set.params.iter().any(|q| q.key == "spacing2") {
            return Err(bad(ErrorCode::InvalidParameters, "a grid needs `spacing2` between columns"));
        }
        let p = reconcile(tx, self.target, p)?;
        let members = p.members.clone();
        tx.set(self.target, p)?;
        Ok(json!({ "id": self.target, "members": members }))
    }
}

/// `assembly.pattern.delete`: remove the pattern and its copies, or (`keep`)
/// leave the copies where they are as free instances.
#[derive(Deserialize, JsonSchema)]
pub struct DeletePattern {
    pub target: StableId,
    #[serde(default)]
    pub keep: bool,
}

impl Command for DeletePattern {
    const NAME: &'static str = "assembly.pattern.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let p = tx.get::<Pattern>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a pattern", self.target)))?;
        delete(tx, self.target, &p, self.keep)?;
        Ok(json!({ "id": self.target }))
    }
}

pub(crate) fn delete(tx: &mut Tx, id: StableId, p: &Pattern, keep: bool) -> Result<(), CommandError> {
    for m in &p.members {
        if keep {
            // Freed where it shows now.
            let placed = tx.get::<crate::joints::Placed>(*m).map(|x| x.transform());
            if let (Some(t), Some(mut inst)) = (placed, tx.get::<Instance>(*m).cloned()) {
                inst.at = t.at.map(|v| (v * 1e9).round() / 1e9);
                inst.rotation = t.euler();
                tx.set(*m, inst)?;
            }
            tx.remove_dependency(*m, id)?;
        } else {
            remove_member(tx, *m)?;
        }
    }
    let users: Vec<StableId> = tx.world().resource::<IdIndex>().iter().filter(|(_, e)| tx.world().get::<DependsOn>(*e).is_some_and(|d| d.0.contains(&id))).map(|(u, _)| u).collect();
    for u in users {
        tx.remove_dependency(u, id)?;
    }
    tx.delete(id)
}

/// The seed was deleted: its patterns go, copies and all.
pub(crate) fn seed_deleted(tx: &mut Tx, seed: StableId) -> Result<(), CommandError> {
    for (id, p) in patterns_seeded_by(tx.world(), seed) {
        delete(tx, id, &p, false)?;
    }
    Ok(())
}

/// The seed changed its definition or configuration: the copies show it too.
pub(crate) fn follow_seed(tx: &mut Tx, seed: StableId) -> Result<(), CommandError> {
    let Some(s) = tx.get::<Instance>(seed).cloned() else { return Ok(()) };
    for (_, p) in patterns_seeded_by(tx.world(), seed) {
        for m in p.members {
            let Some(mut inst) = tx.get::<Instance>(m).cloned() else { continue };
            if inst.of != s.of {
                tx.remove_dependency(m, inst.of)?;
                tx.add_dependency(m, s.of)?;
                inst.of = s.of;
            }
            inst.config = s.config.clone();
            if inst.params != s.params {
                let (before, after) = (de_shape::configs::override_references(&inst.params), de_shape::configs::override_references(&s.params));
                for r in before.iter().filter(|r| !after.contains(r) && **r != s.of) {
                    tx.remove_dependency(m, *r)?;
                }
                for r in after.iter().filter(|r| !before.contains(r) && **r != s.of) {
                    tx.add_dependency(m, *r)?;
                }
                inst.params = s.params.clone();
            }
            tx.set(m, inst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(kind: PatternKind) -> Pattern {
        Pattern { kind, assembly: None, seed: StableId::new(), members: vec![], count: 4, count2: 1, direction: None, direction2: None, axis: None, center: None, path: vec![], closed: false, align: false }
    }

    #[test]
    fn copies_go_where_the_pattern_says() {
        let mut params = BTreeMap::new();
        params.insert("spacing".to_string(), 25.0);
        let m = moves(&Pattern { direction: Some([0.0, 2.0, 0.0]), ..pat(PatternKind::Linear) }, &params).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[2].at, [0.0, 75.0, 0.0]);
        params.insert("angle".to_string(), 90.0);
        let m = moves(&Pattern { center: Some([10.0, 0.0, 0.0]), ..pat(PatternKind::Circular) }, &params).unwrap();
        // The origin turned a quarter about (10, 0): to (10, −10).
        let p = m[0].apply([0.0; 3]);
        assert!((p[0] - 10.0).abs() < 1e-12 && (p[1] + 10.0).abs() < 1e-12, "{p:?}");
        let m = moves(&Pattern { path: vec![[0.0; 3], [30.0, 0.0, 0.0], [30.0, 30.0, 0.0]], align: true, ..pat(PatternKind::Path) }, &params).unwrap();
        // 60 mm of path, 4 copies: every 20 mm; the third is round the corner, turned a quarter.
        let p = m[1].apply([0.0; 3]);
        assert!((p[0] - 30.0).abs() < 1e-12 && (p[1] - 10.0).abs() < 1e-12, "{p:?}");
        assert!((m[1].turn([1.0, 0.0, 0.0])[1] - 1.0).abs() < 1e-12);
    }
}
