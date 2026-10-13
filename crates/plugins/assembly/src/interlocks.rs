//! Joint-space interlocks: where some joints stand, others are held to a
//! narrower range — the way an industrial arm's controller keeps it from
//! folding into itself (a shoulder low forbids the elbow's last 20°),
//! instead of a redesign that gives up the reach everywhere.
//!
//! An interlock is kept in the model, next to the joints it ties. It has
//! *zones* (when every joint in `when` stands within its range, every joint
//! in `then` is held within its own) and *limit tables* (one joint's
//! range as a function of another's value, linear between rows
//! `[by, min, max]`: a cross-joint limit curve). The motion tools respect
//! it: a joint's sweep (and its `no_clashes_in_motion` requirement) covers
//! only the values the interlocks allow with the other joints where they
//! are; a motion study or a robot path stops where it would enter a
//! forbidden region; IK and the reach envelope keep out of them; setting a
//! joint into one is refused; the view's joint sliders stop at the
//! boundary. `/robot/<tip>?controller=json|arduino` writes them, with the
//! joints' limits, for the arm's controller firmware.

use crate::joints::{self, Joint};
use crate::{AssemblyDef, bad};
use de_core::{
    Command, CommandError, DependsOn, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_shape::params::ParameterSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A range of one joint's value (its main one unless `key` names another).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JointRange {
    pub joint: StableId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// When every `when` range holds, every `then` range must too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Zone {
    pub when: Vec<JointRange>,
    pub then: Vec<JointRange>,
}

/// `joint`'s range as a function of `by`'s value: rows `[by, min, max]`,
/// in increasing `by`, linear between them and held past the ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitTable {
    pub by: StableId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_key: Option<String>,
    pub joint: StableId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub rows: Vec<[f64; 3]>,
}

/// A joint-space interlock in an assembly (or at the top of the project).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Interlock {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<Zone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<LimitTable>,
}

impl Persist for Interlock {
    const KIND: &'static str = "assembly.interlock";
}

fn entity(world: &World, id: StableId) -> Option<Entity> {
    world.resource::<IdIndex>().get(id)
}

/// Every joint's values, by joint, in its kind's order.
pub type Values = HashMap<StableId, Vec<f64>>;

impl Interlock {
    /// The joints it names.
    pub fn joints(&self) -> Vec<StableId> {
        let mut out: Vec<StableId> = vec![];
        for z in &self.zones {
            out.extend(z.when.iter().chain(&z.then).map(|r| r.joint));
        }
        for t in &self.tables {
            out.push(t.by);
            out.push(t.joint);
        }
        out.sort();
        out.dedup();
        out
    }
}

/// The index of `key` (default the main value) among a joint's values.
pub fn key_index(world: &World, joint: StableId, key: Option<&str>) -> Result<usize, String> {
    let j = entity(world, joint).and_then(|e| world.get::<Joint>(e)).ok_or_else(|| format!("{joint} is not a joint"))?;
    let keys = j.kind.values();
    if keys.is_empty() {
        return Err(format!("{} is a {} joint: it has no value to limit", joints::joint_name(world, joint), j.kind.word()));
    }
    match key {
        None => Ok(0),
        Some(k) => keys.iter().position(|(x, _, _)| *x == k).ok_or_else(|| format!("{} has no value “{k}”", joints::joint_name(world, joint))),
    }
}

/// Every interlock of a scope (an assembly, or the top), oldest first.
pub fn interlocks_in(world: &World, scope: Option<StableId>) -> Vec<(StableId, Interlock)> {
    let mut out: Vec<(StableId, Interlock)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Interlock>(e).filter(|i| i.assembly == scope).map(|i| (id, i.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// Every joint's values in the scope now (formulas worked out).
pub fn values_now(world: &World, scope: Option<StableId>) -> Values {
    joints::joints_in(world, scope)
        .into_iter()
        .filter_map(|(id, j)| {
            let set = entity(world, id).and_then(|e| world.get::<ParameterSet>(e))?;
            let r = de_shape::params::resolve(world, &set.params).ok()?;
            Some((id, j.kind.values().iter().map(|(k, _, _)| r.get(*k).copied().unwrap_or(0.0)).collect()))
        })
        .collect()
}

const EPS: f64 = 1e-9;

fn within(v: f64, min: Option<f64>, max: Option<f64>) -> bool {
    min.is_none_or(|m| v >= m - EPS) && max.is_none_or(|m| v <= m + EPS)
}

/// A table's [min, max] at `x` (linear between rows, held past the ends).
pub fn table_at(rows: &[[f64; 3]], x: f64) -> (f64, f64) {
    let Some(first) = rows.first() else { return (f64::NEG_INFINITY, f64::INFINITY) };
    if x <= first[0] {
        return (first[1], first[2]);
    }
    for w in rows.windows(2) {
        let (a, b) = (w[0], w[1]);
        if x <= b[0] {
            let t = if b[0] > a[0] { (x - a[0]) / (b[0] - a[0]) } else { 1.0 };
            return (a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t);
        }
    }
    let last = rows[rows.len() - 1];
    (last[1], last[2])
}

fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

/// How a value reads in a message: “Elbow 110°”.
fn value_text(world: &World, joint: StableId, index: usize, v: f64) -> String {
    let unit = entity(world, joint).and_then(|e| world.get::<Joint>(e)).and_then(|j| j.kind.values().get(index).map(|(_, _, u)| *u)).unwrap_or("");
    format!("{} {}{}", joints::joint_name(world, joint), fmt(v), if unit == "deg" { "°" } else if unit == "mm" { " mm" } else { "" })
}

fn range_text(world: &World, r: &JointRange) -> String {
    let n = joints::joint_name(world, r.joint);
    let unit = entity(world, r.joint).and_then(|e| world.get::<Joint>(e)).and_then(|j| j.kind.values().first().map(|(_, _, u)| *u)).unwrap_or("");
    let u = if unit == "deg" { "°" } else if unit == "mm" { " mm" } else { "" };
    match (r.min, r.max) {
        (Some(a), Some(b)) => format!("{n} {}–{}{u}", fmt(a), fmt(b)),
        (Some(a), None) => format!("{n} at least {}{u}", fmt(a)),
        (None, Some(b)) => format!("{n} at most {}{u}", fmt(b)),
        (None, None) => n,
    }
}

/// Why these joint values break an interlock of the scope, if one does.
/// Joints not given read `values` as missing and are not judged.
pub fn violation(world: &World, scope: Option<StableId>, values: &Values) -> Option<String> {
    violation_in(world, &interlocks_in(world, scope), values)
}

pub fn violation_in(world: &World, list: &[(StableId, Interlock)], values: &Values) -> Option<String> {
    let index = |joint: StableId, key: &Option<String>| key_index(world, joint, key.as_deref()).ok();
    let broken = broken(list, values, &index)?;
    let il = &list[broken.interlock].1;
    let get = |joint: StableId, key: &Option<String>| -> Option<(usize, f64)> {
        let i = index(joint, key)?;
        values.get(&joint).and_then(|v| v.get(i)).map(|v| (i, *v))
    };
    Some(match broken.rule {
        Rule::Zone(z, r) => {
            let (z, r) = (&il.zones[z], &il.zones[z].then[r]);
            let when: Vec<String> = z.when.iter().map(|w| range_text(world, w)).collect();
            let (i, v) = get(r.joint, &r.key).unwrap_or((0, f64::NAN));
            format!("the interlock “{}” holds {} while {}: {} is outside it", il.name, range_text(world, r), when.join(" and "), value_text(world, r.joint, i, v))
        }
        Rule::Table(t) => {
            let t = &il.tables[t];
            let ((bi, b), (i, v)) = (get(t.by, &t.by_key).unwrap_or((0, f64::NAN)), get(t.joint, &t.key).unwrap_or((0, f64::NAN)));
            let (lo, hi) = table_at(&t.rows, b);
            format!(
                "the interlock “{}” holds {} within {}–{} at {}: {} is outside it",
                il.name,
                joints::joint_name(world, t.joint),
                fmt(lo),
                fmt(hi),
                value_text(world, t.by, bi, b),
                value_text(world, t.joint, i, v)
            )
        }
    })
}

/// Which rule of which interlock some values break.
#[derive(Clone, Copy, Debug)]
pub struct Broken {
    pub interlock: usize,
    pub rule: Rule,
}

#[derive(Clone, Copy, Debug)]
pub enum Rule {
    /// Zone index, and the index of the `then` range broken.
    Zone(usize, usize),
    Table(usize),
}

/// The first rule these values break, with each named value's index among
/// its joint's values from `index` (so it can be judged without the world:
/// a robot's IK, many times a second). Joints without values are not judged.
pub fn broken(list: &[(StableId, Interlock)], values: &Values, index: &dyn Fn(StableId, &Option<String>) -> Option<usize>) -> Option<Broken> {
    let get = |joint: StableId, key: &Option<String>| -> Option<f64> { values.get(&joint).and_then(|v| v.get(index(joint, key)?)).copied() };
    for (n, (_, il)) in list.iter().enumerate() {
        for (zi, z) in il.zones.iter().enumerate() {
            if !z.when.iter().all(|r| get(r.joint, &r.key).is_some_and(|v| within(v, r.min, r.max))) {
                continue;
            }
            if let Some(ri) = z.then.iter().position(|r| get(r.joint, &r.key).is_some_and(|v| !within(v, r.min, r.max))) {
                return Some(Broken { interlock: n, rule: Rule::Zone(zi, ri) });
            }
        }
        for (ti, t) in il.tables.iter().enumerate() {
            let (Some(b), Some(v)) = (get(t.by, &t.by_key), get(t.joint, &t.key)) else { continue };
            let (lo, hi) = table_at(&t.rows, b);
            if v < lo - EPS || v > hi + EPS {
                return Some(Broken { interlock: n, rule: Rule::Table(ti) });
            }
        }
    }
    None
}

/// Each (joint, key) the interlocks name → its index among the joint's values.
pub fn indices(world: &World, list: &[(StableId, Interlock)]) -> HashMap<(StableId, Option<String>), usize> {
    let mut out = HashMap::new();
    for (_, il) in list {
        let mut add = |j: StableId, k: &Option<String>| {
            if let Ok(i) = key_index(world, j, k.as_deref()) {
                out.insert((j, k.clone()), i);
            }
        };
        for z in &il.zones {
            for r in z.when.iter().chain(&z.then) {
                add(r.joint, &r.key);
            }
        }
        for t in &il.tables {
            add(t.by, &t.by_key);
            add(t.joint, &t.key);
        }
    }
    out
}

/// The values of `joint`'s value `index` in [lo, hi] the interlocks allow
/// with every other joint at `values`: the allowed intervals, in order
/// (exact: worked out between every value where some rule could change).
pub fn allowed(world: &World, scope: Option<StableId>, joint: StableId, index: usize, values: &Values, lo: f64, hi: f64) -> Vec<(f64, f64)> {
    let list = interlocks_in(world, scope);
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    if list.is_empty() || !list.iter().any(|(_, il)| il.joints().contains(&joint)) {
        return vec![(lo, hi)];
    }
    let base = values.get(&joint).cloned().unwrap_or_else(|| vec![0.0; index + 1]);
    let ok = |v: f64| {
        let mut vs = values.clone();
        let mut mine = base.clone();
        if mine.len() <= index {
            mine.resize(index + 1, 0.0);
        }
        mine[index] = v;
        vs.insert(joint, mine);
        violation_in(world, &list, &vs).is_none()
    };
    // Where a rule on this value could start or stop holding.
    let mut cuts = vec![lo, hi];
    let mine = |r: &JointRange| r.joint == joint && key_index(world, r.joint, r.key.as_deref()).ok() == Some(index);
    for (_, il) in &list {
        for z in &il.zones {
            for r in z.when.iter().chain(&z.then).filter(|r| mine(r)) {
                cuts.extend(r.min);
                cuts.extend(r.max);
            }
        }
        for t in &il.tables {
            let by_mine = t.by == joint && key_index(world, t.by, t.by_key.as_deref()).ok() == Some(index);
            let j_mine = t.joint == joint && key_index(world, t.joint, t.key.as_deref()).ok() == Some(index);
            if j_mine && let Some(b) = key_index(world, t.by, t.by_key.as_deref()).ok().and_then(|bi| values.get(&t.by).and_then(|v| v.get(bi)).copied()) {
                let (a, z) = table_at(&t.rows, b);
                cuts.extend([a, z]);
            }
            if by_mine && let Some(v) = key_index(world, t.joint, t.key.as_deref()).ok().and_then(|i| values.get(&t.joint).and_then(|x| x.get(i)).copied()) {
                // Where the table's min or max line crosses the other joint's value.
                for w in t.rows.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    cuts.push(a[0]);
                    cuts.push(b[0]);
                    for k in [1, 2] {
                        let (ya, yb) = (a[k] - v, b[k] - v);
                        if ya * yb < 0.0 {
                            cuts.push(a[0] + (b[0] - a[0]) * ya / (ya - yb));
                        }
                    }
                }
            }
        }
    }
    cuts.retain(|c| c.is_finite() && *c >= lo && *c <= hi);
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    let mut out: Vec<(f64, f64)> = vec![];
    let mut push = |a: f64, b: f64| match out.last_mut() {
        Some(last) if (last.1 - a).abs() < 1e-12 => last.1 = b,
        _ => out.push((a, b)),
    };
    for (k, c) in cuts.iter().enumerate() {
        if ok(*c) {
            push(*c, *c);
        }
        if let Some(n) = cuts.get(k + 1)
            && ok((c + n) / 2.0)
        {
            push(*c, *n);
        }
    }
    // A boundary the rule itself forbids (Elbow at least 78° starts a zone)
    // is left out: the piece stops just short of it.
    let inside = |v: f64, toward: f64| v + (toward - v).signum() * 1e-6 * v.abs().max(1.0);
    out.into_iter()
        .filter_map(|(a, b)| {
            let a = if ok(a) { a } else { inside(a, b) };
            let b = if ok(b) { b } else { inside(b, a) };
            (a <= b).then_some((a, b))
        })
        .collect()
}

/// The allowed interval holding `v` (or the nearest one), from [`allowed`].
pub fn interval_around(intervals: &[(f64, f64)], v: f64) -> Option<(f64, f64)> {
    intervals
        .iter()
        .copied()
        .min_by(|a, b| {
            let d = |(lo, hi): (f64, f64)| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
            d(*a).total_cmp(&d(*b))
        })
}

// ---- Commands -------------------------------------------------------------------

/// A range as written: `{ "min": -60, "max": 100 }` or `[null, 100]`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RangeInput {
    Bounds {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    Pair([Option<f64>; 2]),
}

impl RangeInput {
    fn bounds(&self) -> (Option<f64>, Option<f64>) {
        match self {
            RangeInput::Bounds { min, max } => (*min, *max),
            RangeInput::Pair([a, b]) => (*a, *b),
        }
    }
}

/// A zone as written: joints by name (or id; "Name.key" for another of its
/// values) → their ranges.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct ZoneInput {
    pub when: BTreeMap<String, RangeInput>,
    pub then: BTreeMap<String, RangeInput>,
}

/// A limit table as written: `joint`'s [min, max] for values of `by`, rows `[by, min, max]`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct TableInput {
    pub by: String,
    pub joint: String,
    pub rows: Vec<[f64; 3]>,
}

/// A joint by name or id, with an optional ".key" for one of its values.
fn joint_key(world: &World, scope: Option<StableId>, r: &str) -> Result<(StableId, Option<String>), String> {
    if let Ok(id) = crate::couplings::joint_ref(world, scope, r) {
        return Ok((id, None));
    }
    if let Some((name, key)) = r.rsplit_once('.')
        && let Ok(id) = crate::couplings::joint_ref(world, scope, name)
    {
        key_index(world, id, Some(key))?;
        return Ok((id, Some(key.to_string())));
    }
    crate::couplings::joint_ref(world, scope, r).map(|id| (id, None))
}

fn build_zones(world: &World, scope: Option<StableId>, zones: Vec<ZoneInput>) -> Result<Vec<Zone>, String> {
    let range = |name: &str, r: &RangeInput| -> Result<JointRange, String> {
        let (joint, key) = joint_key(world, scope, name)?;
        key_index(world, joint, key.as_deref())?;
        let (min, max) = r.bounds();
        if min.is_none() && max.is_none() {
            return Err(format!("{name}: give a min, a max or both"));
        }
        if let (Some(a), Some(b)) = (min, max)
            && a > b
        {
            return Err(format!("{name}: min {a} is above max {b}"));
        }
        Ok(JointRange { joint, key, min, max })
    };
    let mut out_zones = vec![];
    for (i, z) in zones.iter().enumerate() {
        if z.when.is_empty() || z.then.is_empty() {
            return Err(format!("zone {}: say when it holds (when) and what it holds (then)", i + 1));
        }
        let when = z.when.iter().map(|(n, r)| range(n, r)).collect::<Result<Vec<_>, _>>()?;
        let then = z.then.iter().map(|(n, r)| range(n, r)).collect::<Result<Vec<_>, _>>()?;
        if then.iter().any(|t| when.iter().any(|w| w.joint == t.joint && w.key == t.key)) {
            return Err(format!("zone {}: a joint cannot both set the condition and be held by it", i + 1));
        }
        out_zones.push(Zone { when, then });
    }
    Ok(out_zones)
}

fn build_tables(world: &World, scope: Option<StableId>, tables: Vec<TableInput>) -> Result<Vec<LimitTable>, String> {
    let mut out_tables = vec![];
    for t in tables {
        let (by, by_key) = joint_key(world, scope, &t.by)?;
        let (joint, key) = joint_key(world, scope, &t.joint)?;
        if by == joint && by_key == key {
            return Err("a limit table ties two different joints".into());
        }
        if t.rows.is_empty() {
            return Err(format!("{} by {}: give rows [{}, min, max]", t.joint, t.by, t.by));
        }
        if t.rows.iter().any(|r| r.iter().any(|v| !v.is_finite()) || r[1] > r[2]) || t.rows.windows(2).any(|w| w[1][0] < w[0][0]) {
            return Err(format!("{} by {}: rows are [{}, min, max] with min ≤ max, in increasing {}", t.joint, t.by, t.by, t.by));
        }
        out_tables.push(LimitTable { by, by_key, joint, key, rows: t.rows });
    }
    Ok(out_tables)
}

/// Keeps an interlock's dependencies on its joints, and the scope's joint
/// hub's on it (what the hub holds re-solves, and the requirements above
/// it are checked again).
fn link(tx: &mut Tx, id: StableId, il: &Interlock) -> Result<(), CommandError> {
    let want = il.joints();
    let current = tx.get::<DependsOn>(id).map(|d| d.0.clone()).unwrap_or_default();
    for d in current.iter().filter(|d| !want.contains(d)) {
        tx.remove_dependency(id, *d)?;
    }
    for d in want.iter().filter(|d| !current.contains(d)) {
        tx.add_dependency(id, *d)?;
    }
    joints::rewire(tx, il.assembly)?;
    joints::remark(tx, il.assembly)
}

/// The pose now must already be one the interlock allows.
fn allows_now(tx: &Tx, scope: Option<StableId>, name: &str) -> Result<(), CommandError> {
    let values = values_now(tx.world(), scope);
    match violation(tx.world(), scope, &values) {
        Some(why) => Err(bad(ErrorCode::InvalidParameters, format!("{why}; move the joints out of it first (it would hold “{name}” in a pose it forbids)"))),
        None => Ok(()),
    }
}

/// `assembly.interlock.create`: a joint-space interlock — zones (when these
/// joints stand within these ranges, those are held within theirs) and/or
/// limit tables (one joint's range following another's value).
#[derive(Deserialize, JsonSchema)]
pub struct CreateInterlock {
    pub assembly: Option<StableId>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub zones: Vec<ZoneInput>,
    #[serde(default)]
    pub tables: Vec<TableInput>,
}

impl Command for CreateInterlock {
    const NAME: &'static str = "assembly.interlock.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if let Some(a) = self.assembly
            && tx.get::<AssemblyDef>(a).is_none()
        {
            return Err(bad(ErrorCode::UnresolvedReference, format!("{a} is not an assembly")));
        }
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(bad(ErrorCode::InvalidParameters, "an interlock needs a name"));
        }
        if interlocks_in(tx.world(), self.assembly).iter().any(|(_, i)| i.name == name) {
            return Err(bad(ErrorCode::InvalidParameters, format!("there is already an interlock called “{name}” here")));
        }
        let invalid = |m: String| bad(ErrorCode::InvalidParameters, m);
        let zones = build_zones(tx.world(), self.assembly, self.zones).map_err(invalid)?;
        let tables = build_tables(tx.world(), self.assembly, self.tables).map_err(invalid)?;
        if zones.is_empty() && tables.is_empty() {
            return Err(invalid("an interlock needs a zone or a limit table".into()));
        }
        let il =Interlock { name: name.clone(), assembly: self.assembly, description: self.description, zones, tables };
        let id = tx.create();
        tx.set(id, il.clone())?;
        allows_now(tx, il.assembly, &name)?;
        link(tx, id, &il)?;
        Ok(json!({ "id": id, "name": name }))
    }
}

/// `assembly.interlock.update`: rename it, or replace its zones or tables.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateInterlock {
    pub target: StableId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub zones: Option<Vec<ZoneInput>>,
    pub tables: Option<Vec<TableInput>>,
}

impl Command for UpdateInterlock {
    const NAME: &'static str = "assembly.interlock.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut il = tx.get::<Interlock>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an interlock", self.target)))?;
        if let Some(n) = self.name {
            let n = n.trim().to_string();
            if n.is_empty() {
                return Err(bad(ErrorCode::InvalidParameters, "an interlock needs a name"));
            }
            il.name = n;
        }
        if let Some(d) = self.description {
            il.description = d;
        }
        let invalid = |m: String| bad(ErrorCode::InvalidParameters, m);
        if let Some(z) = self.zones {
            il.zones = build_zones(tx.world(), il.assembly, z).map_err(invalid)?;
        }
        if let Some(t) = self.tables {
            il.tables = build_tables(tx.world(), il.assembly, t).map_err(invalid)?;
        }
        if il.zones.is_empty() && il.tables.is_empty() {
            return Err(invalid("an interlock needs a zone or a limit table (delete it instead)".into()));
        }
        tx.set(self.target, il.clone())?;
        allows_now(tx, il.assembly, &il.name)?;
        link(tx, self.target, &il)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.interlock.delete`: the joints move freely within their own limits again.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteInterlock {
    pub target: StableId,
}

impl Command for DeleteInterlock {
    const NAME: &'static str = "assembly.interlock.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let il = tx.get::<Interlock>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an interlock", self.target)))?;
        delete(tx, self.target)?;
        joints::rewire(tx, il.assembly)?;
        joints::remark(tx, il.assembly)?;
        Ok(json!({ "id": self.target }))
    }
}

fn delete(tx: &mut Tx, id: StableId) -> Result<(), CommandError> {
    let users: Vec<StableId> = tx.world().resource::<IdIndex>().iter().filter(|(_, e)| tx.world().get::<DependsOn>(*e).is_some_and(|d| d.0.contains(&id))).map(|(u, _)| u).collect();
    for u in users {
        tx.remove_dependency(u, id)?;
    }
    let deps = tx.get::<DependsOn>(id).map(|d| d.0.clone()).unwrap_or_default();
    for d in deps {
        tx.remove_dependency(id, d)?;
    }
    tx.delete(id)
}

/// A joint is being deleted: the interlock rules that name it go (and an
/// interlock left with none goes too).
pub(crate) fn joint_deleted(tx: &mut Tx, joint: StableId) -> Result<(), CommandError> {
    let ids: Vec<(StableId, Interlock)> = tx.world().resource::<IdIndex>().iter().filter_map(|(id, e)| tx.world().get::<Interlock>(e).filter(|i| i.joints().contains(&joint)).map(|i| (id, i.clone()))).collect();
    for (id, mut il) in ids {
        il.zones.retain(|z| !z.when.iter().chain(&z.then).any(|r| r.joint == joint));
        il.tables.retain(|t| t.by != joint && t.joint != joint);
        if il.zones.is_empty() && il.tables.is_empty() {
            delete(tx, id)?;
        } else {
            tx.set(id, il.clone())?;
            tx.remove_dependency(id, joint)?;
        }
    }
    Ok(())
}

/// Refuses a change that sets joints into a region an interlock forbids
/// (a joint scrubbed, a pose applied, a robot moved): run after every
/// command, it looks only when a joint's values changed.
pub fn guard(world: &World, ops: &[de_core::Op]) -> Result<(), CommandError> {
    let mut scopes: Vec<Option<StableId>> = vec![];
    for op in ops {
        let de_core::Op::Set { id, kind, .. } = op else { continue };
        if *kind != ParameterSet::KIND {
            continue;
        }
        if let Some(j) = entity(world, *id).and_then(|e| world.get::<Joint>(e))
            && !scopes.contains(&j.assembly)
        {
            scopes.push(j.assembly);
        }
    }
    for scope in scopes {
        let list = interlocks_in(world, scope);
        if list.is_empty() {
            continue;
        }
        let values = values_now(world, scope);
        if let Some(why) = violation_in(world, &list, &values) {
            return Err(bad(ErrorCode::InvalidParameters, format!("Refused: {why}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_linear_between_rows_and_held_past_the_ends() {
        let rows = [[0.0, -60.0, 100.0], [40.0, -60.0, 100.0], [60.0, -60.0, 120.0]];
        assert_eq!(table_at(&rows, -10.0), (-60.0, 100.0));
        assert_eq!(table_at(&rows, 50.0), (-60.0, 110.0));
        assert_eq!(table_at(&rows, 90.0), (-60.0, 120.0));
    }

    #[test]
    fn the_interval_around_a_value_is_the_one_holding_it() {
        let iv = [(-60.0, 100.0), (110.0, 120.0)];
        assert_eq!(interval_around(&iv, 50.0), Some((-60.0, 100.0)));
        assert_eq!(interval_around(&iv, 105.0), Some((-60.0, 100.0)));
        assert_eq!(interval_around(&iv, 119.0), Some((110.0, 120.0)));
    }
}
