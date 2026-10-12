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
    /// They touch the same way at every step: a part turning in contact about
    /// the joint's axis (a pin in its bearing, a horn screw on its spline), not
    /// a clash the motion brings. Counted as joined.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub turning: bool,
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
    let e = entity(world, id).ok_or("an instance is gone")?;
    let own = placement(world, e).unwrap_or(Transform::IDENTITY).inverse();
    let mut leaves = vec![];
    let mut boxes = vec![];
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
/// Sweeps joint `joint`'s value `key` (default its main one) from `from` to
/// `to` (default its limits) in `steps` steps, other joints held at their
/// values or at `set` (joint id → values) when given.
pub fn sweep(world: &World, joint: StableId, key: Option<&str>, from: Option<f64>, to: Option<f64>, steps: usize, set: &HashMap<StableId, Vec<f64>>, gaps_too: bool) -> Result<AssemblySweep, String> {
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
    let frames: Vec<(f64, HashMap<StableId, Vec<f64>>)> = (0..=steps)
        .map(|k| {
            let v = from + (to - from) * k as f64 / steps as f64;
            let mut values = set.clone();
            let mut mine = base.clone();
            mine[index] = v;
            values.insert(joint, mine);
            (v, values)
        })
        .collect();
    let p = play(world, j.assembly, &frames, gaps_too, unit, &params.name)?;
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
}

/// Plays the assembly (or the top) through `frames`: each a label and the
/// joint values to hold (others keep theirs). `unit` words where it binds;
/// `what` names what drives it in notes.
pub fn play(world: &World, scope: Option<StableId>, frames: &[(f64, HashMap<StableId, Vec<f64>>)], gaps_too: bool, unit: &str, what: &str) -> Result<Played, String> {
    // Every instance of the assembly (or the top), in a stable order.
    let mut ids: Vec<(StableId, String)> = world
        .resource::<IdIndex>()
        .iter()
        .filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == scope).map(|i| (id, i.name.clone())))
        .collect();
    ids.sort_by_key(|(id, _)| *id);
    let mut out = Played { assembly: scope, instances: vec![], frames: vec![], binds_at: None, pairs: vec![], clashes: vec![], search: 0.0, notes: vec![] };
    let mut guess: HashMap<StableId, Transform> = HashMap::new();
    let mut poses_per_frame: Vec<Vec<Transform>> = vec![];
    for (v, values) in frames {
        match joints::solve_scope(world, scope, values, &guess) {
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
    if !out.instances.iter().any(|i| i.moves) {
        out.notes.push(format!("Nothing moves when {what} turns or slides: no instance hangs from it."));
        return Ok(out);
    }
    if gaps_too {
        gaps(world, &ids, &poses_per_frame, &mut out)?;
    }
    Ok(out)
}

fn round(v: f64, k: f64) -> f64 {
    (v * k).round() / k
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
    let mut most = vec![0.0f64; pairs.len()];
    // Leaves are placed only when a pair comes near enough to measure: an
    // instance that holds still is placed once for the whole sweep, a moving
    // one once per step (placing a mesh rebuilds its collision tree).
    let still: Vec<bool> = (0..bodies.len()).map(|i| frames.iter().all(|f| f[i].distance(&frames[0][i], size.max(1.0)) <= 1e-9)).collect();
    let mut placed_still: HashMap<(usize, usize), Manifold> = HashMap::new();
    let apart = |(la, ha): ([f64; 3], [f64; 3]), (lb, hb): ([f64; 3], [f64; 3])| (0..3).map(|k| (lb[k] - ha[k]).max(la[k] - hb[k]).max(0.0).powi(2)).sum::<f64>().sqrt();
    for (n, f) in frames.iter().enumerate().step_by(every) {
        let value = out.frames[n].value;
        let mut placed_now: HashMap<(usize, usize), Manifold> = HashMap::new();
        for (p, (a, b)) in pairs.iter().enumerate() {
            // Each step only asks whether the pair comes closer than it has so
            // far: a pair in contact is then checked within a micron, not the
            // whole search (measuring is per triangle pair within reach).
            let reach = best[p].0.clamp(1e-3, search);
            // Placed boxes farther apart than that: no closer look needed.
            let g = if apart(f[*a].bounds(bodies[*a].lo, bodies[*a].hi), f[*b].bounds(bodies[*b].lo, bodies[*b].hi)) >= reach {
                reach
            } else {
                let mut g = reach;
                for (x, bx) in bodies[*a].boxes.iter().enumerate() {
                    let wx = f[*a].bounds(bx.0, bx.1);
                    for (y, by) in bodies[*b].boxes.iter().enumerate() {
                        if apart(wx, f[*b].bounds(by.0, by.1)) >= g {
                            continue;
                        }
                        for (i, l) in [(*a, x), (*b, y)] {
                            let cache = if still[i] { &mut placed_still } else { &mut placed_now };
                            cache.entry((i, l)).or_insert_with(|| {
                                let (s, inner) = &bodies[i].leaves[l];
                                f[i].then(inner).solid(s)
                            });
                        }
                        let get = |i: usize, l: usize| if still[i] { &placed_still[&(i, l)] } else { &placed_now[&(i, l)] };
                        g = g.min(get(*a, x).min_gap(get(*b, y), g));
                    }
                }
                g
            };
            most[p] = most[p].max(g);
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
        .zip(&most)
        .filter(|((_, (g, _, _)), _)| *g < search)
        .map(|(((a, b), (g, at, range)), most)| {
            let (ia, ib) = (ids[*a].0, ids[*b].0);
            let turning = *most < 1e-4;
            let joined = turning || joined.iter().any(|(x, y)| (*x == ia && *y == ib) || (*x == ib && *y == ia));
            PairGap {
                a: ids[*a].1.clone(),
                b: ids[*b].1.clone(),
                a_id: ia,
                b_id: ib,
                gap: round(g.max(0.0), 1e4),
                at: *at,
                range: *range,
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
