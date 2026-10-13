//! An assembly moving through a joint's range: every instance placed at each
//! step, and the closest approach of every pair of instances that move
//! relative to each other, with the clashes (as GET /motion does for the
//! parts of one program, here across separately built components).
//!
//! Measuring is the slow part (thousands of closest-approach measurements),
//! so it is kept twice over: a whole sweep by everything it was worked out
//! from (read back when nothing changed: a requirement re-checked, the
//! project opened again), and each pair's measurement by exactly what it
//! depends on — the two parts' geometry and how one moves against the other
//! through the steps. A change elsewhere (a screw renamed, a part far away
//! resized, a joint posed so only the far end of the arm moves differently)
//! measures again only the pairs it changed.
//!
//! The joints' interlocks (see `interlocks`) are respected: a sweep covers
//! only the values they allow with the other joints where they are, and a
//! played motion stops where it would enter a region they forbid.

use crate::joints::{self, Joint, JointKind};
use crate::transform::Transform;
use crate::{Instance, Occurrence, placement};
use de_core::{IdIndex, StableId, bevy_ecs::world::World};
use de_shape::{ShapeSolid, params::ParameterSet};
use manifold_csg::Manifold;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweptInstance {
    pub id: StableId,
    pub name: String,
    /// It moves somewhere in the sweep.
    pub moves: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SweepFrame {
    /// The swept value (degrees or mm).
    pub value: f64,
    /// Each instance's placement in the assembly: position (mm) and rotation
    /// quaternion [x, y, z, w], in the order of `instances`.
    pub poses: Vec<([f64; 3], [f64; 4])>,
    /// Every joint's values at this step.
    pub joints: BTreeMap<StableId, Vec<f64>>,
}

/// A shape whose parameters follow the joints (a spring stretched by the arm
/// it pulls on, a servo's output spline turning with its joint): its values
/// at every frame, so a player can show it changing (evaluate the shape with
/// them, or blend between a few evaluated steps).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FlexShape {
    /// The instance of the swept assembly it is in (what `poses` places).
    pub instance: StableId,
    /// Instances from that one down to the shape (outermost first).
    pub path: Vec<StableId>,
    pub shape: StableId,
    /// The configuration it is evaluated in, if its instance shows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// For each frame: the parameters (key → value) that differ from the
    /// shape as it is (in that configuration).
    pub values: Vec<BTreeMap<String, f64>>,
}

/// Two instances' closest approach through the sweep.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairGap {
    pub a: String,
    pub b: String,
    pub a_id: StableId,
    pub b_id: StableId,
    /// Smallest gap (mm; 0 = touching or overlapping).
    pub gap: f64,
    /// The swept value where it is smallest (the first such step).
    pub at: f64,
    /// For a clash: the first and last swept values where they touch or overlap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// A joint joins them directly (a shaft in its bore, a horn on its spline:
    /// contact there is meant).
    pub joined: bool,
    /// They touch the same way at every step: a part turning in contact about
    /// the joint's axis (a pin in its bearing, a horn screw on its spline), not
    /// a clash the motion brings. Counted as joined.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub turning: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssemblySweep {
    pub assembly: Option<StableId>,
    pub joint: StableId,
    pub name: String,
    pub kind: JointKind,
    /// Which of the joint's values is swept, and its unit ("°" or "mm").
    pub key: String,
    pub unit: String,
    pub from: f64,
    pub to: f64,
    pub instances: Vec<SweptInstance>,
    pub frames: Vec<SweepFrame>,
    /// Where it stopped, if the joints could not follow further.
    pub binds_at: Option<f64>,
    /// Pairs that came within the search distance, closest first.
    pub pairs: Vec<PairGap>,
    /// Pairs that touch or overlap somewhere in the sweep and are not joined.
    pub clashes: Vec<PairGap>,
    /// The distance searched for near misses (mm).
    pub search: f64,
    pub notes: Vec<String>,
    /// Shapes that change as the joints move, with their values at each frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flex: Vec<FlexShape>,
    /// The values of the swept joint the interlocks allow here, when they
    /// leave out some of its range ([from, to] pieces, in order).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<[f64; 2]>,
}

/// One instance's solids, in its own coordinates.
pub(crate) struct Body {
    pub(crate) leaves: Vec<(Arc<Manifold>, Transform)>,
    /// Each leaf's box in the instance's coordinates.
    pub(crate) boxes: Vec<([f64; 3], [f64; 3])>,
    pub(crate) lo: [f64; 3],
    pub(crate) hi: [f64; 3],
}

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

pub(crate) fn body(world: &World, id: StableId) -> Result<Body, String> {
    body_with(world, id, None)
}

/// Joint values to read formulas with (joint → its values, in its kind's order).
type JointValues = BTreeMap<StableId, Vec<f64>>;

/// A joint's value `key` among these values.
fn given_in<'a>(world: &'a World, joints: &'a JointValues) -> impl Fn(StableId, &str) -> Option<f64> + 'a {
    move |id: StableId, key: &str| -> Option<f64> {
        let v = joints.get(&id)?;
        let kind = entity(world, id).and_then(|e| world.get::<Joint>(e))?.kind;
        kind.values().iter().position(|(k, _, _)| *k == key).and_then(|i| v.get(i).copied())
    }
}

/// An occurrence's program ready to evaluate with these joint values read by
/// its formulas (a spring whose stretched length follows the shoulder's
/// angle; a servo whose output spline follows its joint).
fn program_given(world: &World, o: &Occurrence, joints: &JointValues) -> Result<de_shape::ShapeProgram, String> {
    let given = given_in(world, joints);
    crate::occurrence_program_given(world, o, &given).map_err(|m| format!("{}: {m}", o.names.join(" › ")))
}

/// Whether an occurrence has a formula (its shape's, or one its instance
/// gives of its own) reading a joint's value: its geometry changes as the
/// joints move.
fn reads_joints(world: &World, o: &Occurrence) -> bool {
    let joint = |r: &StableId| entity(world, *r).is_some_and(|e| world.get::<Joint>(e).is_some());
    let program = entity(world, o.shape).and_then(|e| world.get::<de_shape::ShapeProgram>(e));
    program.is_some_and(|p| de_shape::params::references(&p.params).iter().any(joint)) || de_shape::configs::override_references(&o.params).iter().any(joint)
}

/// Whether some shape the instance shows follows a joint's value: a sweep
/// evaluates it again at each step.
fn follows_joints(world: &World, id: StableId) -> bool {
    crate::occurrences(world, id).iter().any(|o| reads_joints(world, o))
}

/// Whether the instance's shapes come out differently across these joint
/// values (a spring the swept joint stretches), not just read some joint.
fn varies(world: &World, id: StableId, steps: &[&JointValues]) -> bool {
    let Some(first) = steps.first() else { return false };
    crate::occurrences(world, id).iter().filter(|o| reads_joints(world, o)).any(|o| {
        let values = |j: &JointValues| program_given(world, o, j).ok().map(|p| p.params.iter().map(|q| q.value).collect::<Vec<f64>>());
        let base = values(first);
        steps[1..].iter().any(|j| values(j) != base)
    })
}

/// One instance's solids, with its shapes' formulas reading these joint values if given.
fn body_with(world: &World, id: StableId, joints: Option<&JointValues>) -> Result<Body, String> {
    let e = entity(world, id).ok_or("an instance is gone")?;
    let own = placement(world, e).unwrap_or(Transform::IDENTITY).inverse();
    let mut leaves = vec![];
    let mut boxes = vec![];
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for o in crate::occurrences(world, id) {
        let se = entity(world, o.shape).ok_or("a component is gone")?;
        let solid = match joints {
            Some(j) => {
                let p = program_given(world, &o, j)?;
                Arc::new(de_shape::evaluated_at(&p, p.display_tolerance())?.solid.translate(0.0, 0.0, 0.0))
            }
            None if o.varies() => {
                let p = crate::occurrence_program(world, &o)?;
                Arc::new(de_shape::evaluated_at(&p, p.display_tolerance())?.solid.translate(0.0, 0.0, 0.0))
            }
            None => world.get::<ShapeSolid>(se).ok_or_else(|| format!("{} has no valid result", o.names.join(" › ")))?.solid.clone(),
        };
        let inner = own.then(&o.transform);
        let (a, b) = match solid.bounding_box() {
            Some(bb) => inner.bounds(bb.min(), bb.max()),
            None => ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        };
        for k in 0..3 {
            lo[k] = lo[k].min(a[k]);
            hi[k] = hi[k].max(b[k]);
        }
        leaves.push((solid, inner));
        boxes.push((a, b));
    }
    Ok(Body { leaves, boxes, lo, hi })
}

/// What an instance's solids are made from, hashed: each shape's program as
/// the instance shows it (with these joint values, for one that follows
/// them) and where it sits in the instance. Two equal keys are the same solids.
fn body_key(world: &World, id: StableId, joints: Option<&JointValues>) -> Option<u64> {
    let e = entity(world, id)?;
    let own = placement(world, e).unwrap_or(Transform::IDENTITY).inverse();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for o in crate::occurrences(world, id) {
        let program = match joints {
            Some(j) => program_given(world, &o, j).ok()?,
            None => crate::occurrence_program(world, &o).ok()?,
        };
        de_shape::cache::program_key(&program, &mut h);
        hash_transform(&own.then(&o.transform), &mut h);
    }
    Some(h.finish())
}

/// A placement, rounded below what a measurement can tell apart (a solver's
/// last bits differ between runs that land on the same pose).
fn hash_transform(t: &Transform, h: &mut impl Hasher) {
    let mut q = |v: f64, k: f64| ((v * k).round() as i64).hash(h);
    for v in t.at {
        q(v, 1e6);
    }
    for v in t.rot.iter().flatten() {
        q(*v, 1e9);
    }
}

/// The values a joint sweeps between by default: its limits, else a full
/// turn from where it is for an angle.
fn default_range(kind: JointKind, key: &str, set: &ParameterSet, value: f64) -> Result<(f64, f64), String> {
    let p = set.params.iter().find(|p| p.key == key);
    if let Some((lo, hi)) = p.and_then(|p| Some((p.min?, p.max?))) {
        return Ok((lo, hi));
    }
    let turning = kind.values().iter().any(|(k, _, u)| *k == key && *u == "deg");
    if turning {
        return Ok((value, value + 360.0));
    }
    Err(format!("give “{}” limits, or say how far to slide it (from, to)", set.name))
}

/// Sweeps joint `joint`'s value `key` (default its main one) from `from` to
/// `to` (default its limits) in `steps` steps, other joints held at their
/// values or at `set` (joint id → values) when given.
pub fn sweep(world: &World, joint: StableId, key: Option<&str>, from: Option<f64>, to: Option<f64>, steps: usize, set: &HashMap<StableId, Vec<f64>>, gaps_too: bool) -> Result<AssemblySweep, String> {
    use std::sync::{Mutex, OnceLock};
    // Sweeps are kept by what they were worked out from: the same sweep asked
    // again (a requirement re-checked, the motion panel, the project opened
    // again) is read back, not measured again.
    static RECENT: OnceLock<Mutex<Vec<(u64, Arc<AssemblySweep>)>>> = OnceLock::new();
    let fingerprint = sweep_key(world, joint, key, from, to, steps, set, gaps_too);
    let recent = RECENT.get_or_init(|| Mutex::new(Vec::new()));
    if let Some(k) = fingerprint {
        if let Some((_, s)) = recent.lock().unwrap().iter().find(|(x, _)| *x == k) {
            return Ok((**s).clone());
        }
        if let Some(s) = de_shape::cache::load::<AssemblySweep>("sweep", k) {
            remember(recent, k, &s);
            return Ok(s);
        }
    }
    let s = sweep_now(world, joint, key, from, to, steps, set, gaps_too)?;
    if let Some(k) = fingerprint {
        de_shape::cache::store("sweep", k, &s);
        remember(recent, k, &s);
    }
    Ok(s)
}

fn remember(recent: &std::sync::Mutex<Vec<(u64, Arc<AssemblySweep>)>>, key: u64, s: &AssemblySweep) {
    let mut r = recent.lock().unwrap();
    if r.len() >= 32 {
        r.remove(0);
    }
    r.push((key, Arc::new(s.clone())));
}

/// Everything a sweep is worked out from, hashed: its arguments, every joint
/// of the assembly with its values, its couplings and interlocks, and every
/// instance in it with what it shows, where it is and the programs (sizes,
/// finishes) of every shape it holds. None when something is missing (the
/// sweep then reports it).
#[allow(clippy::too_many_arguments)]
fn sweep_key(world: &World, joint: StableId, key: Option<&str>, from: Option<f64>, to: Option<f64>, steps: usize, set: &HashMap<StableId, Vec<f64>>, gaps_too: bool) -> Option<u64> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let bits = |v: Option<f64>| v.map(f64::to_bits);
    (2u8, joint, key, bits(from), bits(to), steps, gaps_too).hash(&mut h);
    let mut set: Vec<_> = set.iter().map(|(k, v)| (*k, v.iter().map(|x| x.to_bits()).collect::<Vec<_>>())).collect();
    set.sort();
    set.hash(&mut h);
    let je = entity(world, joint)?;
    let scope = world.get::<Joint>(je)?.assembly;
    for (id, j) in joints::joints_in(world, scope) {
        let e = entity(world, id)?;
        let ps = world.get::<ParameterSet>(e)?;
        (id, serde_json::to_string(&j).ok()?).hash(&mut h);
        // Values (formulas worked out), limits and names (notes and pairs name them).
        ps.name.hash(&mut h);
        let resolved = de_shape::params::resolve(world, &ps.params).ok()?;
        for p in &ps.params {
            (&p.key, resolved.get(&p.key).map(|v| v.to_bits()), bits(p.min), bits(p.max)).hash(&mut h);
        }
    }
    for (id, c) in crate::couplings::couplings_in(world, scope) {
        (id, serde_json::to_string(&c).ok()?).hash(&mut h);
    }
    for (id, i) in crate::interlocks::interlocks_in(world, scope) {
        (id, serde_json::to_string(&i).ok()?).hash(&mut h);
    }
    let mut ids: Vec<StableId> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == scope).map(|_| id)).collect();
    ids.sort();
    for id in ids {
        let e = entity(world, id)?;
        id.hash(&mut h);
        serde_json::to_string(world.get::<Instance>(e)?).ok()?.hash(&mut h);
        let t = placement(world, e).unwrap_or(Transform::IDENTITY);
        format!("{:?}", (t.at, t.rot)).hash(&mut h);
        for o in crate::occurrences(world, id) {
            let program = crate::occurrence_program(world, &o).ok()?;
            de_shape::cache::program_key(&program, &mut h);
            format!("{:?}", (o.transform.at, o.transform.rot, &o.path)).hash(&mut h);
        }
    }
    Some(h.finish())
}

/// The values a sweep of `joint`'s value `index` steps through: `steps`
/// even steps from `from` to `to`, keeping only those the interlocks allow
/// with the other joints at `base`, plus the ends of each allowed piece (so
/// the sweep reaches exactly to an interlock's boundary). Also returns the
/// allowed pieces when the interlocks leave some of the range out.
fn steps_allowed(world: &World, scope: Option<StableId>, joint: StableId, index: usize, base: &HashMap<StableId, Vec<f64>>, from: f64, to: f64, steps: usize) -> (Vec<f64>, Vec<[f64; 2]>) {
    let grid: Vec<f64> = (0..=steps).map(|k| from + (to - from) * k as f64 / steps as f64).collect();
    let pieces = crate::interlocks::allowed(world, scope, joint, index, base, from, to);
    let (lo, hi) = (from.min(to), from.max(to));
    if pieces.len() == 1 && (pieces[0].0 - lo).abs() < 1e-9 && (pieces[0].1 - hi).abs() < 1e-9 {
        return (grid, vec![]);
    }
    let inside = |v: f64| pieces.iter().any(|(a, b)| v >= a - 1e-9 && v <= b + 1e-9);
    let mut values: Vec<f64> = grid.into_iter().filter(|v| inside(*v)).collect();
    for (a, b) in &pieces {
        for v in [*a, *b] {
            if !values.iter().any(|x| (x - v).abs() < 1e-9) {
                values.push(v);
            }
        }
    }
    if to >= from {
        values.sort_by(f64::total_cmp);
    } else {
        values.sort_by(|a, b| b.total_cmp(a));
    }
    (values, pieces.iter().map(|(a, b)| [*a, *b]).collect())
}

#[allow(clippy::too_many_arguments)]
fn sweep_now(world: &World, joint: StableId, key: Option<&str>, from: Option<f64>, to: Option<f64>, steps: usize, set: &HashMap<StableId, Vec<f64>>, gaps_too: bool) -> Result<AssemblySweep, String> {
    let je = entity(world, joint).ok_or_else(|| format!("joint {joint} does not exist"))?;
    let j: Joint = world.get::<Joint>(je).cloned().ok_or_else(|| format!("{joint} is not a joint"))?;
    let params = world.get::<ParameterSet>(je).cloned().ok_or("the joint has no values")?;
    let keys: Vec<&str> = j.kind.values().iter().map(|(k, _, _)| *k).collect();
    if keys.is_empty() {
        return Err(format!("{} is a {} joint: nothing moves", params.name, j.kind.word()));
    }
    let key = key.unwrap_or(keys[0]);
    let index = keys.iter().position(|k| *k == key).ok_or_else(|| format!("{} has no value “{key}” (it has {})", params.name, keys.join(", ")))?;
    let base = match set.get(&joint) {
        Some(v) => v.clone(),
        None => joints::link(world, joint, None)?.values,
    };
    let (lo, hi) = default_range(j.kind, key, &params, base[index])?;
    let (from, to) = (from.unwrap_or(lo), to.unwrap_or(hi));
    if !(from.is_finite() && to.is_finite()) || from == to {
        return Err("the sweep needs two different finite ends".into());
    }
    let steps = steps.clamp(2, 3600);
    let unit = if j.kind.values()[index].2 == "deg" { "°" } else { "mm" };
    // The other joints where they are (or as set), for the interlocks.
    let mut held = crate::interlocks::values_now(world, j.assembly);
    for (k, v) in set {
        held.insert(*k, v.clone());
    }
    held.insert(joint, base.clone());
    let (values, allowed) = steps_allowed(world, j.assembly, joint, index, &held, from, to, steps);
    if values.is_empty() {
        let why = crate::interlocks::violation(world, j.assembly, &held).unwrap_or_else(|| "the interlocks allow none of it with the other joints where they are".into());
        return Err(format!("{} cannot move here: {why}", params.name));
    }
    let frames: Vec<(f64, HashMap<StableId, Vec<f64>>)> = values
        .iter()
        .map(|v| {
            let mut values = set.clone();
            let mut mine = base.clone();
            mine[index] = *v;
            values.insert(joint, mine);
            (*v, values)
        })
        .collect();
    let mut p = play(world, j.assembly, &frames, gaps_too, unit, &params.name)?;
    if !allowed.is_empty() {
        let r = |v: f64| format!("{}{}", (v * 100.0).round() / 100.0, if unit == "°" { "°" } else { " mm" });
        let pieces: Vec<String> = allowed.iter().map(|[a, b]| if a == b { r(*a) } else { format!("{}…{}", r(*a), r(*b)) }).collect();
        p.notes.insert(0, format!("Interlocks keep {} within {} here (of {}…{}), with the other joints where they are: only that is swept.", params.name, pieces.join(" and "), r(from), r(to)));
    }
    Ok(AssemblySweep {
        assembly: j.assembly,
        joint,
        name: params.name.clone(),
        kind: j.kind,
        key: key.to_string(),
        unit: unit.into(),
        from,
        to,
        instances: p.instances,
        frames: p.frames,
        binds_at: p.binds_at,
        pairs: p.pairs,
        clashes: p.clashes,
        search: p.search,
        notes: p.notes,
        flex: p.flex,
        allowed,
    })
}

/// An assembly played through a list of joint settings: what [`sweep`]
/// does for one joint, for any number at once (a motion study's timeline,
/// a robot's trajectory).
#[derive(Clone, Debug, Serialize)]
pub struct Played {
    pub assembly: Option<StableId>,
    pub instances: Vec<SweptInstance>,
    /// Each step; `value` is the step's label (a time in seconds, a swept value).
    pub frames: Vec<SweepFrame>,
    /// The label where it stopped, if the joints could not follow further.
    pub binds_at: Option<f64>,
    pub pairs: Vec<PairGap>,
    pub clashes: Vec<PairGap>,
    pub search: f64,
    pub notes: Vec<String>,
    /// Shapes that change as the joints move, with their values at each frame.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flex: Vec<FlexShape>,
}

/// Plays the assembly (or the top) through `frames`: each a label and the
/// joint values to hold (others keep theirs). `unit` words where it binds;
/// `what` names what drives it in notes. It stops where the joints cannot
/// follow, or where an interlock forbids the joints' values.
pub fn play(world: &World, scope: Option<StableId>, frames: &[(f64, HashMap<StableId, Vec<f64>>)], gaps_too: bool, unit: &str, what: &str) -> Result<Played, String> {
    // Every instance of the assembly (or the top), in a stable order.
    let mut ids: Vec<(StableId, String)> = world
        .resource::<IdIndex>()
        .iter()
        .filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == scope).map(|i| (id, i.name.clone())))
        .collect();
    ids.sort_by_key(|(id, _)| *id);
    let mut out = Played { assembly: scope, instances: vec![], frames: vec![], binds_at: None, pairs: vec![], clashes: vec![], search: 0.0, notes: vec![], flex: vec![] };
    let interlocks = crate::interlocks::interlocks_in(world, scope);
    let mut guess: HashMap<StableId, Transform> = HashMap::new();
    let mut poses_per_frame: Vec<Vec<Transform>> = vec![];
    for (v, values) in frames {
        let solved = joints::solve_scope(world, scope, values, &guess).and_then(|sol| {
            // A pose an interlock forbids is as far as the motion may go.
            match crate::interlocks::violation_in(world, &interlocks, &sol.values) {
                Some(why) => Err(why),
                None => Ok(sol),
            }
        });
        match solved {
            Ok(sol) => {
                // Pattern copies follow their seeds.
                let mut posed = sol.poses.clone();
                for (id, _) in &ids {
                    if let Some(t) = crate::patterns::member_at(world, *id, &posed) {
                        posed.insert(*id, t);
                    }
                }
                let poses: Vec<Transform> = ids
                    .iter()
                    .map(|(id, _)| posed.get(id).copied().or_else(|| entity(world, *id).and_then(|e| placement(world, e))).unwrap_or(Transform::IDENTITY))
                    .collect();
                guess = sol.poses.clone();
                out.frames.push(SweepFrame {
                    value: round(*v, 1e6),
                    poses: poses.iter().map(|t| (t.at.map(|c| round(c, 1e6)), t.quaternion())).collect(),
                    joints: sol.values.iter().map(|(k, v)| (*k, v.iter().map(|x| round(*x, 1e6)).collect())).collect(),
                });
                poses_per_frame.push(poses);
            }
            Err(e) => {
                if out.frames.is_empty() {
                    return Err(e);
                }
                out.binds_at = Some(round(*v, 1e6));
                out.notes.push(format!("It binds at {} {unit}: {e}", round(*v, 100.0)));
                break;
            }
        }
    }
    let moved = |i: usize| poses_per_frame.iter().any(|f| f[i].distance(&poses_per_frame[0][i], 100.0) > 1e-7);
    out.instances = ids.iter().enumerate().map(|(i, (id, name))| SweptInstance { id: *id, name: name.clone(), moves: moved(i) }).collect();
    out.flex = flexing(world, &ids, &out.frames);
    if !out.instances.iter().any(|i| i.moves) && out.flex.is_empty() {
        out.notes.push(format!("Nothing moves when {what} turns or slides: no instance hangs from it."));
        return Ok(out);
    }
    if gaps_too {
        gaps(world, &ids, &poses_per_frame, &mut out)?;
    }
    Ok(out)
}

/// The shapes that change through these frames, with their values at each.
fn flexing(world: &World, ids: &[(StableId, String)], frames: &[SweepFrame]) -> Vec<FlexShape> {
    let mut out = vec![];
    if frames.len() < 2 {
        return out;
    }
    for (id, _) in ids {
        for o in crate::occurrences(world, *id).iter().filter(|o| reads_joints(world, o)) {
            let Some(se) = entity(world, o.shape) else { continue };
            // What a player starts from: the shape as it is, in the configuration shown.
            let base = match &o.config {
                Some(c) => de_shape::configs::configured_program(world, se, c).ok(),
                None => de_shape::resolved_program(world, se),
            };
            let Some(base) = base else { continue };
            let mut values = vec![];
            for f in frames {
                let Ok(p) = program_given(world, o, &f.joints) else { break };
                let diff: BTreeMap<String, f64> = p
                    .params
                    .iter()
                    .filter(|q| base.params.iter().find(|b| b.key == q.key).is_none_or(|b| (b.value - q.value).abs() > 1e-12))
                    .map(|q| (q.key.clone(), round(q.value, 1e9)))
                    .collect();
                values.push(diff);
            }
            if values.len() == frames.len() && values.iter().any(|v| v != &values[0]) {
                out.push(FlexShape { instance: *id, path: o.path.clone(), shape: o.shape, config: o.config.clone(), values });
            }
        }
    }
    out
}

fn round(v: f64, k: f64) -> f64 {
    (v * k).round() / k
}

/// What one pair's measurement through a sweep came to, by step index.
#[derive(Clone, Copy, Debug)]
struct PairOutcome {
    /// Smallest gap, and the step where it is.
    best: f64,
    best_at: usize,
    /// The first and last steps where they touch or overlap.
    range: Option<[usize; 2]>,
    /// The largest gap measured (within the search): a pair touching at
    /// every step turns in contact.
    most: f64,
}

/// Pairs measured before, by what their measurement depends on (see [`gaps`]).
fn pair_cache() -> &'static std::sync::Mutex<HashMap<u64, PairOutcome>> {
    static PAIRS: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, PairOutcome>>> = std::sync::OnceLock::new();
    PAIRS.get_or_init(Default::default)
}

/// Pairs read back from the cache and pairs measured by sweeps on this
/// thread so far (for tests and timing).
pub fn pair_counts() -> (usize, usize) {
    PAIR_COUNTS.with(|c| c.get())
}

thread_local! {
    static PAIR_COUNTS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Closest approach of every pair whose relative placement changes.
fn gaps(world: &World, ids: &[(StableId, String)], frames: &[Vec<Transform>], out: &mut Played) -> Result<(), String> {
    let bodies: Vec<Body> = ids.iter().map(|(id, _)| body(world, *id)).collect::<Result<_, _>>()?;
    let size = bodies.iter().filter(|b| b.lo[0].is_finite()).map(|b| (0..3).map(|k| b.hi[k] - b.lo[k]).fold(0.0, f64::max)).fold(0.0, f64::max);
    let search = (size * 0.05).clamp(2.0, 20.0);
    out.search = search;
    let joined: Vec<(StableId, StableId)> = joints::joints_in(world, out.assembly)
        .iter()
        .filter_map(|(_, j)| Some((j.a.instance()?, j.b.instance()?)))
        .collect();
    let every = frames.len().div_ceil(180).max(1);
    let sampled: Vec<usize> = (0..frames.len()).step_by(every).collect();
    // Shapes that follow the joints (a spring stretching as an arm swings)
    // change at each step: evaluated again with that step's joint values
    // (only for pairs that are measured).
    let used: Vec<&JointValues> = sampled.iter().map(|n| &out.frames[*n].joints).collect();
    let flex: Vec<bool> = ids.iter().map(|(id, _)| follows_joints(world, *id) && varies(world, *id, &used)).collect();
    if flex.iter().any(|f| *f) {
        let names: Vec<&str> = ids.iter().zip(&flex).filter(|(_, f)| **f).map(|((_, n), _)| n.as_str()).collect();
        out.notes.push(format!("{} follow{} the joints: evaluated again at each step.", names.join(", "), if names.len() == 1 { "s" } else { "" }));
    }
    let mut pairs = vec![];
    for i in 0..bodies.len() {
        for k in i + 1..bodies.len() {
            let rel = |f: &Vec<Transform>| f[i].inverse().then(&f[k]);
            let r0 = rel(&frames[0]);
            if (flex[i] || flex[k] || frames.iter().any(|f| rel(f).distance(&r0, size.max(1.0)) > 1e-6)) && !bodies[i].leaves.is_empty() && !bodies[k].leaves.is_empty() {
                pairs.push((i, k));
            }
        }
    }
    // What each pair's measurement depends on: both bodies (each step's, for
    // one that follows the joints) and how one sits against the other at
    // every step measured, with the search distance and the steps measured.
    let mut keys: HashMap<usize, Vec<Option<u64>>> = HashMap::new();
    let mut key_of = |i: usize| -> Vec<Option<u64>> {
        keys.entry(i)
            .or_insert_with(|| {
                if flex[i] {
                    sampled.iter().map(|n| body_key(world, ids[i].0, Some(&out.frames[*n].joints))).collect()
                } else {
                    vec![body_key(world, ids[i].0, None)]
                }
            })
            .clone()
    };
    let pair_keys: Vec<Option<u64>> = pairs
        .iter()
        .map(|(a, b)| {
            let (ka, kb) = (key_of(*a), key_of(*b));
            if ka.iter().chain(&kb).any(Option::is_none) {
                return None;
            }
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (1u8, search.to_bits(), every, frames.len(), &ka, &kb).hash(&mut h);
            for n in &sampled {
                hash_transform(&frames[*n][*a].inverse().then(&frames[*n][*b]), &mut h);
            }
            Some(h.finish())
        })
        .collect();
    let mut results: Vec<Option<PairOutcome>> = {
        let cache = pair_cache().lock().unwrap();
        pair_keys.iter().map(|k| k.and_then(|k| cache.get(&k).copied())).collect()
    };
    let todo: Vec<usize> = (0..pairs.len()).filter(|p| results[*p].is_none()).collect();
    PAIR_COUNTS.with(|c| {
        let (hit, measured) = c.get();
        c.set((hit + pairs.len() - todo.len(), measured + todo.len()));
    });
    if !todo.is_empty() {
        let measured = measure(world, ids, frames, &bodies, &flex, &keys, &pairs, &todo, &sampled, search, size, out)?;
        let mut cache = pair_cache().lock().unwrap();
        if cache.len() > 400_000 {
            cache.clear();
        }
        for (p, r) in todo.iter().zip(measured) {
            if let Some(k) = pair_keys[*p] {
                cache.insert(k, r);
            }
            results[*p] = Some(r);
        }
    }
    let value = |n: usize| out.frames[n].value;
    let mut list: Vec<PairGap> = pairs
        .iter()
        .zip(&results)
        .filter_map(|((a, b), r)| r.map(|r| (a, b, r)))
        .filter(|(_, _, r)| r.best < search)
        .map(|(a, b, r)| {
            let (ia, ib) = (ids[*a].0, ids[*b].0);
            let turning = r.most < 1e-4;
            let joined = turning || joined.iter().any(|(x, y)| (*x == ia && *y == ib) || (*x == ib && *y == ia));
            PairGap {
                a: ids[*a].1.clone(),
                b: ids[*b].1.clone(),
                a_id: ia,
                b_id: ib,
                gap: round(r.best.max(0.0), 1e4),
                at: value(r.best_at),
                range: r.range.map(|[s, e]| {
                    let (x, y) = (value(s), value(e));
                    [x.min(y), x.max(y)]
                }),
                joined,
                turning,
            }
        })
        .collect();
    list.sort_by(|a, b| a.gap.total_cmp(&b.gap));
    out.clashes = list.iter().filter(|p| p.gap < 1e-4 && !p.joined).cloned().collect();
    out.pairs = list;
    if every > 1 {
        out.notes.push(format!("Gaps checked at {} of the {} steps.", frames.len().div_ceil(every), frames.len()));
    }
    Ok(())
}

/// Bodies of shapes that follow the joints, by what they are made from
/// (see [`body_key`]): a spring or a servo's turning spline at each step of
/// a sweep is evaluated once, not again for every sweep that holds it so.
fn flexed_body(world: &World, id: StableId, key: Option<u64>, joints: &JointValues) -> Result<Arc<Body>, String> {
    use std::sync::{Mutex, OnceLock};
    static KEPT: OnceLock<Mutex<Vec<(u64, Arc<Body>)>>> = OnceLock::new();
    const CAPACITY: usize = 320;
    let kept = KEPT.get_or_init(Default::default);
    if let Some(k) = key {
        let mut list = kept.lock().unwrap();
        if let Some(i) = list.iter().position(|(x, _)| *x == k) {
            let hit = list.remove(i);
            let b = hit.1.clone();
            list.push(hit);
            return Ok(b);
        }
    }
    let b = Arc::new(body_with(world, id, Some(joints))?);
    if let Some(k) = key {
        let mut list = kept.lock().unwrap();
        if list.len() >= CAPACITY {
            list.remove(0);
        }
        list.push((k, b.clone()));
    }
    Ok(b)
}

/// Measures pairs `todo` (indices into `pairs`) through the sampled steps.
#[allow(clippy::too_many_arguments)]
fn measure(world: &World, ids: &[(StableId, String)], frames: &[Vec<Transform>], bodies: &[Body], flex: &[bool], flex_keys: &HashMap<usize, Vec<Option<u64>>>, pairs: &[(usize, usize)], todo: &[usize], sampled: &[usize], search: f64, size: f64, out: &Played) -> Result<Vec<PairOutcome>, String> {
    // Bodies of the shapes that follow the joints, at each step measured
    // (only those some pair to measure needs).
    let mut flexed: HashMap<(usize, usize), Arc<Body>> = HashMap::new();
    let needed: Vec<usize> = (0..ids.len()).filter(|i| flex[*i] && todo.iter().any(|p| pairs[*p].0 == *i || pairs[*p].1 == *i)).collect();
    for (s, n) in sampled.iter().enumerate() {
        for i in &needed {
            let key = flex_keys.get(i).and_then(|k| k.get(s).copied().flatten());
            let b = flexed_body(world, ids[*i].0, key, &out.frames[*n].joints).map_err(|m| format!("{} at {}: {m}", ids[*i].1, out.frames[*n].value))?;
            flexed.insert((*n, *i), b);
        }
    }
    let bod = |n: usize, i: usize| -> &Body { if flex[i] { &flexed[&(n, i)] } else { &bodies[i] } };
    let mut best = vec![(f64::INFINITY, 0usize, None::<[usize; 2]>); todo.len()];
    let mut most = vec![0.0f64; todo.len()];
    // Leaves are placed only when a pair comes near enough to measure: an
    // instance that holds still is placed once for the whole sweep, a moving
    // one once per step (placing a mesh rebuilds its collision tree).
    let still: Vec<bool> = (0..bodies.len()).map(|i| !flex[i] && frames.iter().all(|f| f[i].distance(&frames[0][i], size.max(1.0)) <= 1e-9)).collect();
    let mut placed_still: HashMap<(usize, usize), Manifold> = HashMap::new();
    let apart = |(la, ha): ([f64; 3], [f64; 3]), (lb, hb): ([f64; 3], [f64; 3])| (0..3).map(|k| (lb[k] - ha[k]).max(la[k] - hb[k]).max(0.0).powi(2)).sum::<f64>().sqrt();
    for &n in sampled {
        let f = &frames[n];
        let mut placed_now: HashMap<(usize, usize), Manifold> = HashMap::new();
        for (t, p) in todo.iter().enumerate() {
            let (a, b) = pairs[*p];
            // Each step only asks whether the pair comes closer than it has so
            // far: a pair in contact is then checked within a micron, not the
            // whole search (measuring is per triangle pair within reach).
            let reach = best[t].0.clamp(1e-3, search);
            // Placed boxes farther apart than that: no closer look needed.
            let (ba, bb) = (bod(n, a), bod(n, b));
            let g = if apart(f[a].bounds(ba.lo, ba.hi), f[b].bounds(bb.lo, bb.hi)) >= reach {
                reach
            } else {
                let mut g = reach;
                for (x, bx) in ba.boxes.iter().enumerate() {
                    let wx = f[a].bounds(bx.0, bx.1);
                    for (y, by) in bb.boxes.iter().enumerate() {
                        if apart(wx, f[b].bounds(by.0, by.1)) >= g {
                            continue;
                        }
                        for (i, l) in [(a, x), (b, y)] {
                            let cache = if still[i] { &mut placed_still } else { &mut placed_now };
                            cache.entry((i, l)).or_insert_with(|| {
                                let (s, inner) = &bod(n, i).leaves[l];
                                f[i].then(inner).solid(s)
                            });
                        }
                        let get = |i: usize, l: usize| if still[i] { &placed_still[&(i, l)] } else { &placed_now[&(i, l)] };
                        g = g.min(get(a, x).min_gap(get(b, y), g));
                    }
                }
                g
            };
            most[t] = most[t].max(g);
            if g < best[t].0 {
                best[t].0 = g;
                best[t].1 = n;
            }
            if g < 1e-4 {
                let r = best[t].2.get_or_insert([n, n]);
                r[0] = r[0].min(n);
                r[1] = r[1].max(n);
            }
        }
    }
    Ok(best.into_iter().zip(most).map(|((best, best_at, range), most)| PairOutcome { best, best_at, range, most }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sweeps are kept on disk as JSON: what is written must read back.
    #[test]
    fn a_sweep_reads_back_as_written() {
        let (a, b) = (StableId::new(), StableId::new());
        let gap = PairGap { a: "Arm".into(), b: "Base".into(), a_id: a, b_id: b, gap: 0.4, at: 30.0, range: None, joined: false, turning: false };
        let s = AssemblySweep {
            assembly: None,
            joint: a,
            name: "Shoulder".into(),
            kind: JointKind::Hinge,
            key: "angle".into(),
            unit: "°".into(),
            from: 0.0,
            to: 90.0,
            instances: vec![SweptInstance { id: b, name: "Arm".into(), moves: true }],
            frames: vec![SweepFrame { value: 0.0, poses: vec![([0.0, 1.0, 2.0], [0.0, 0.0, 0.0, 1.0])], joints: BTreeMap::from([(a, vec![0.0])]) }],
            binds_at: None,
            pairs: vec![gap.clone(), PairGap { turning: true, range: Some([0.0, 90.0]), ..gap }],
            clashes: vec![],
            search: 2.0,
            notes: vec![],
            flex: vec![FlexShape { instance: b, path: vec![b], shape: a, config: None, values: vec![BTreeMap::from([("length".to_string(), 42.5)])] }],
            allowed: vec![[0.0, 60.0]],
        };
        let back: AssemblySweep = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(&s).unwrap());
    }
}
