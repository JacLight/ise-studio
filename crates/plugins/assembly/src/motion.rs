//! An assembly moving through a joint's range: every instance placed at each
//! step, and the closest approach of every pair of instances that move
//! relative to each other, with the clashes (as GET /motion does for the
//! parts of one program, here across separately built components).

use crate::joints::{self, Joint, JointKind};
use crate::transform::Transform;
use crate::{Instance, placement};
use de_core::{IdIndex, StableId, bevy_ecs::world::World};
use de_shape::{ShapeSolid, params::ParameterSet};
use manifold_csg::Manifold;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
pub struct SweptInstance {
    pub id: StableId,
    pub name: String,
    /// It moves somewhere in the sweep.
    pub moves: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SweepFrame {
    /// The swept value (degrees or mm).
    pub value: f64,
    /// Each instance's placement in the assembly: position (mm) and rotation
    /// quaternion [x, y, z, w], in the order of `instances`.
    pub poses: Vec<([f64; 3], [f64; 4])>,
    /// Every joint's values at this step.
    pub joints: BTreeMap<StableId, Vec<f64>>,
}

/// Two instances' closest approach through the sweep.
#[derive(Clone, Debug, Serialize)]
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
}

#[derive(Clone, Debug, Serialize)]
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
}

/// One instance's solids, in its own coordinates.
struct Body {
    leaves: Vec<(Arc<Manifold>, Transform)>,
    lo: [f64; 3],
    hi: [f64; 3],
}

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

fn body(world: &World, id: StableId) -> Result<Body, String> {
    let e = entity(world, id).ok_or("an instance is gone")?;
    let own = placement(world, e).unwrap_or(Transform::IDENTITY).inverse();
    let mut leaves = vec![];
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for o in crate::occurrences(world, id) {
        let se = entity(world, o.shape).ok_or("a component is gone")?;
        let solid = match &o.config {
            Some(c) => {
                let p = de_shape::configs::configured_program(world, se, c)?;
                Arc::new(de_shape::evaluated_at(&p, p.display_tolerance())?.solid.translate(0.0, 0.0, 0.0))
            }
            None => world.get::<ShapeSolid>(se).ok_or_else(|| format!("{} has no valid result", o.names.join(" › ")))?.solid.clone(),
        };
        let inner = own.then(&o.transform);
        if let Some(bb) = solid.bounding_box() {
            let (a, b) = inner.bounds(bb.min(), bb.max());
            for k in 0..3 {
                lo[k] = lo[k].min(a[k]);
                hi[k] = hi[k].max(b[k]);
            }
        }
        leaves.push((solid, inner));
    }
    Ok(Body { leaves, lo, hi })
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
    let je = entity(world, joint).ok_or_else(|| format!("joint {joint} does not exist"))?;
    let j: Joint = world.get::<Joint>(je).cloned().ok_or_else(|| format!("{joint} is not a joint"))?;
    let params = world.get::<ParameterSet>(je).cloned().ok_or("the joint has no values")?;
    let keys: Vec<&str> = j.kind.values().iter().map(|(k, _, _)| *k).collect();
    if keys.is_empty() {
        return Err(format!("{} is a fixed joint: nothing moves", params.name));
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
    // Every instance of the assembly (or the top), in a stable order.
    let mut ids: Vec<(StableId, String)> = world
        .resource::<IdIndex>()
        .iter()
        .filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == j.assembly).map(|i| (id, i.name.clone())))
        .collect();
    ids.sort_by_key(|(id, _)| *id);
    let mut out = AssemblySweep {
        assembly: j.assembly,
        joint,
        name: params.name.clone(),
        kind: j.kind,
        key: key.to_string(),
        unit: unit.into(),
        from,
        to,
        instances: vec![],
        frames: vec![],
        binds_at: None,
        pairs: vec![],
        clashes: vec![],
        search: 0.0,
        notes: vec![],
    };
    let mut guess: HashMap<StableId, Transform> = HashMap::new();
    let mut poses_per_frame: Vec<Vec<Transform>> = vec![];
    for k in 0..=steps {
        let v = from + (to - from) * k as f64 / steps as f64;
        let mut values = set.clone();
        let mut mine = base.clone();
        mine[index] = v;
        values.insert(joint, mine);
        match joints::solve_scope(world, j.assembly, &values, &guess) {
            Ok(sol) => {
                let poses: Vec<Transform> = ids
                    .iter()
                    .map(|(id, _)| sol.poses.get(id).copied().or_else(|| entity(world, *id).and_then(|e| placement(world, e))).unwrap_or(Transform::IDENTITY))
                    .collect();
                guess = sol.poses.clone();
                out.frames.push(SweepFrame {
                    value: round(v, 1e6),
                    poses: poses.iter().map(|t| (t.at.map(|c| round(c, 1e6)), t.quaternion())).collect(),
                    joints: sol.values.iter().map(|(k, v)| (*k, v.iter().map(|x| round(*x, 1e6)).collect())).collect(),
                });
                poses_per_frame.push(poses);
            }
            Err(e) => {
                if out.frames.is_empty() {
                    return Err(e);
                }
                out.binds_at = Some(round(v, 1e6));
                out.notes.push(format!("It binds at {} {unit}: {e}", round(v, 100.0)));
                break;
            }
        }
    }
    let moved = |i: usize| poses_per_frame.iter().any(|f| f[i].distance(&poses_per_frame[0][i], 100.0) > 1e-7);
    out.instances = ids.iter().enumerate().map(|(i, (id, name))| SweptInstance { id: *id, name: name.clone(), moves: moved(i) }).collect();
    if !out.instances.iter().any(|i| i.moves) {
        out.notes.push(format!("Nothing moves when {} turns or slides: no instance hangs from it.", params.name));
        return Ok(out);
    }
    if gaps_too {
        gaps(world, &j, &ids, &poses_per_frame, &mut out)?;
    }
    Ok(out)
}

fn round(v: f64, k: f64) -> f64 {
    (v * k).round() / k
}

/// Closest approach of every pair whose relative placement changes.
fn gaps(world: &World, _j: &Joint, ids: &[(StableId, String)], frames: &[Vec<Transform>], out: &mut AssemblySweep) -> Result<(), String> {
    let bodies: Vec<Body> = ids.iter().map(|(id, _)| body(world, *id)).collect::<Result<_, _>>()?;
    let size = bodies.iter().filter(|b| b.lo[0].is_finite()).map(|b| (0..3).map(|k| b.hi[k] - b.lo[k]).fold(0.0, f64::max)).fold(0.0, f64::max);
    let search = (size * 0.05).clamp(2.0, 20.0);
    out.search = search;
    let joined: Vec<(StableId, StableId)> = joints::joints_in(world, out.assembly)
        .iter()
        .filter_map(|(_, j)| Some((j.a.instance()?, j.b.instance()?)))
        .collect();
    let mut pairs = vec![];
    for i in 0..bodies.len() {
        for k in i + 1..bodies.len() {
            let rel = |f: &Vec<Transform>| f[i].inverse().then(&f[k]);
            let r0 = rel(&frames[0]);
            if frames.iter().any(|f| rel(f).distance(&r0, size.max(1.0)) > 1e-6) && !bodies[i].leaves.is_empty() && !bodies[k].leaves.is_empty() {
                pairs.push((i, k));
            }
        }
    }
    let every = frames.len().div_ceil(180).max(1);
    let mut best = vec![(f64::INFINITY, 0.0, None::<[f64; 2]>); pairs.len()];
    for (n, f) in frames.iter().enumerate().step_by(every) {
        let value = out.frames[n].value;
        let posed: HashMap<usize, Vec<Manifold>> = pairs
            .iter()
            .flat_map(|(a, b)| [*a, *b])
            .map(|i| (i, bodies[i].leaves.iter().map(|(s, inner)| f[i].then(inner).solid(s)).collect()))
            .collect();
        for (p, (a, b)) in pairs.iter().enumerate() {
            // Placed boxes farther apart than the search: no closer look needed.
            let (la, ha) = f[*a].bounds(bodies[*a].lo, bodies[*a].hi);
            let (lb, hb) = f[*b].bounds(bodies[*b].lo, bodies[*b].hi);
            let apart = (0..3).map(|k| (lb[k] - ha[k]).max(la[k] - hb[k]).max(0.0).powi(2)).sum::<f64>().sqrt();
            let g = if apart >= search {
                search
            } else {
                let mut g = f64::INFINITY;
                for x in &posed[a] {
                    for y in &posed[b] {
                        g = g.min(x.min_gap(y, search));
                    }
                }
                g
            };
            if g < best[p].0 {
                best[p].0 = g;
                best[p].1 = value;
            }
            if g < 1e-4 {
                let r = best[p].2.get_or_insert([value, value]);
                r[0] = r[0].min(value);
                r[1] = r[1].max(value);
            }
        }
    }
    let mut list: Vec<PairGap> = pairs
        .iter()
        .zip(&best)
        .filter(|(_, (g, _, _))| *g < search)
        .map(|((a, b), (g, at, range))| {
            let (ia, ib) = (ids[*a].0, ids[*b].0);
            PairGap {
                a: ids[*a].1.clone(),
                b: ids[*b].1.clone(),
                a_id: ia,
                b_id: ib,
                gap: round(g.max(0.0), 1e4),
                at: *at,
                range: *range,
                joined: joined.iter().any(|(x, y)| (*x == ia && *y == ib) || (*x == ib && *y == ia)),
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
