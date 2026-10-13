//! Robot kinematics on an assembly's joints: the serial chain from the
//! ground (or the anchor) out to a tip instance, forward kinematics (where
//! the tool is for given joint values), inverse kinematics (joint values
//! that put the tool at a point, its axis along a direction), the reach
//! envelope (where the tool can get to within the joints' limits) and
//! trajectories through poses and points (joint-space moves, or straight
//! lines in space), played with clash checks like any motion.
//!
//! Every value goes through the same joint solver as the rest of the
//! assembly, so closed loops, couplings and formula-linked joints (a geared
//! gripper finger) move as they do when scrubbed by hand.

use crate::joints::{self, Link};
use crate::transform::{self as tf, Transform, V3};
use crate::views::{self, Values};
use crate::{AssemblyStats, Instance, bad};
use de_core::{Command, CommandError, ErrorCode, IdIndex, StableId, Tx, Value, bevy_ecs::world::World, json, schemars};
use de_shape::params::ParameterSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

/// One driven value of a chain joint.
#[derive(Clone, Debug, Serialize)]
pub struct Axis {
    pub joint: StableId,
    pub name: String,
    pub key: String,
    /// "deg" or "mm".
    pub unit: String,
    pub min: f64,
    pub max: f64,
    #[serde(skip)]
    link: usize,
    #[serde(skip)]
    index: usize,
}

/// A serial chain ready to move.
pub struct Rig {
    pub scope: Option<StableId>,
    pub tip: StableId,
    pub tip_name: String,
    /// The tool frame in the tip's own coordinates.
    pub tool: Transform,
    pub axes: Vec<Axis>,
    /// Joints along the chain, base first (with fixed ones).
    pub joints: Vec<StableId>,
    links: Vec<Link>,
    fixed: HashMap<StableId, Transform>,
    guess: HashMap<StableId, Transform>,
    pub size: f64,
    /// The scope's interlocks, the joints off the chain where they are, and
    /// each value the interlocks name's index (judged without the world).
    interlocks: Vec<(StableId, crate::interlocks::Interlock)>,
    held: crate::interlocks::Values,
    index: HashMap<(StableId, Option<String>), usize>,
}

/// Where the tool is: a connector on the tip, a point in its coordinates,
/// else a connector called TCP / Tool / Tip, else the middle of the tip.
fn tool_frame(world: &World, tip: StableId, tool: Option<&str>, point: Option<V3>) -> Result<Transform, String> {
    if let Some(p) = point {
        return Ok(Transform::translation(p));
    }
    let cs = joints::connectors(world, tip).unwrap_or_default();
    if let Some(name) = tool {
        return cs.iter().find(|(n, _)| n == name).map(|(_, f)| *f).ok_or_else(|| format!("the tip has no connector “{name}” (it has {})", cs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")));
    }
    for want in ["tcp", "tool", "tip", "tool point"] {
        if let Some((_, f)) = cs.iter().find(|(n, _)| n.to_lowercase() == want) {
            return Ok(*f);
        }
    }
    let e = entity(world, tip).ok_or("the tip is gone")?;
    let s = world.get::<AssemblyStats>(e).ok_or("the tip has no shape yet")?;
    // Its totals are where it is placed (its parent's coordinates): into its own.
    let place = crate::placement(world, e).ok_or("the tip is not placed")?;
    Ok(Transform::translation(place.inverse().apply(tf::scale(tf::add(s.bounds_min, s.bounds_max), 0.5))))
}

impl Rig {
    /// The chain from the ground (or anchor) to `tip`.
    pub fn new(world: &World, tip: StableId, tool: Option<&str>, point: Option<V3>) -> Result<Rig, String> {
        let e = entity(world, tip).ok_or_else(|| format!("{tip} does not exist"))?;
        let inst = world.get::<Instance>(e).ok_or_else(|| format!("{tip} is not an instance"))?;
        let scope = inst.parent;
        let all = joints::joints_in(world, scope);
        let group = joints::groups(&all).into_iter().find(|g| g.members.contains(&tip) && g.anchor != Some(tip)).ok_or_else(|| format!("{} is not moved by any joint: pick the part at the end of the arm", inst.name))?;
        let coupled = crate::couplings::coupled_values(world, scope, &HashMap::new())?;
        let links: Vec<Link> = group.joints.iter().map(|j| joints::link(world, *j, coupled.as_ref().and_then(|c| c.get(j)).map(|v| v.as_slice()))).collect::<Result<_, _>>()?;
        let stored = |m: StableId| entity(world, m).and_then(|x| world.get::<Instance>(x)).map(Instance::transform);
        let mut fixed = HashMap::new();
        if let Some(a) = group.anchor {
            fixed.insert(a, stored(a).ok_or("the anchor is gone")?);
        }
        let mut guess = HashMap::new();
        for m in group.held() {
            if let Some(t) = entity(world, m).and_then(|x| crate::placement(world, x)) {
                guess.insert(m, t);
            }
        }
        let sol = joints::solve(&links, &fixed, &guess)?;
        // Walk from the tip back along the joints that placed each instance.
        let mut chain = vec![];
        let mut cur = tip;
        let mut seen = vec![tip];
        while let Some(j) = sol.by.get(&cur) {
            let i = links.iter().position(|l| l.id == *j).ok_or("a joint went missing")?;
            chain.push(i);
            let l = &links[i];
            let other = [l.a.id(), l.b.id()].into_iter().flatten().find(|x| *x != cur);
            match other {
                Some(o) if !seen.contains(&o) && Some(o) != group.anchor => {
                    seen.push(o);
                    cur = o;
                }
                _ => break,
            }
        }
        chain.reverse();
        let followers: Vec<StableId> = crate::couplings::couplings_in(world, scope).into_iter().map(|(_, c)| c.follower).collect();
        let size = joints::size_of(&links);
        let mut axes = vec![];
        for &i in &chain {
            let l = &links[i];
            if followers.contains(&l.id) || l.free {
                continue;
            }
            let set = entity(world, l.id).and_then(|x| world.get::<ParameterSet>(x)).ok_or("a joint has no values")?;
            for (k, (key, _, unit)) in l.kind.values().iter().enumerate() {
                let Some(p) = set.params.iter().find(|p| p.key == *key) else { continue };
                if p.expr.is_some() {
                    continue; // follows a formula
                }
                let (dlo, dhi) = if *unit == "deg" { (-180.0, 180.0) } else { (-size, size) };
                axes.push(Axis { joint: l.id, name: if l.kind.values().len() > 1 { format!("{} {key}", l.name) } else { l.name.clone() }, key: key.to_string(), unit: unit.to_string(), min: p.min.unwrap_or(dlo), max: p.max.unwrap_or(dhi), link: i, index: k });
            }
        }
        if axes.is_empty() {
            return Err(format!("nothing between the ground and {} moves: its joints are fixed or follow others", inst.name));
        }
        let tool = tool_frame(world, tip, tool, point)?;
        let interlocks = crate::interlocks::interlocks_in(world, scope);
        let index = crate::interlocks::indices(world, &interlocks);
        let held = if interlocks.is_empty() { HashMap::new() } else { crate::interlocks::values_now(world, scope) };
        Ok(Rig { scope, tip, tip_name: inst.name.clone(), tool, axes, joints: chain.iter().map(|i| links[*i].id).collect(), links, fixed, guess, size, interlocks, held, index })
    }

    /// The chain's values as they are now.
    pub fn now(&self) -> Vec<f64> {
        self.axes.iter().map(|a| self.links[a.link].values[a.index]).collect()
    }

    /// Whether the interlocks allow the chain at `q` (the other joints where they are).
    pub fn allows(&self, q: &[f64]) -> bool {
        if self.interlocks.is_empty() {
            return true;
        }
        let mut values = self.held.clone();
        for (a, v) in self.axes.iter().zip(q) {
            let e = values.entry(a.joint).or_insert_with(|| self.links[a.link].values.clone());
            if e.len() > a.index {
                e[a.index] = *v;
            }
        }
        crate::interlocks::broken(&self.interlocks, &values, &|j, k| self.index.get(&(j, k.clone())).copied()).is_none()
    }

    fn with(&self, q: &[f64]) -> Vec<Link> {
        let mut links = self.links.clone();
        for (a, v) in self.axes.iter().zip(q) {
            links[a.link].values[a.index] = *v;
        }
        links
    }

    /// Where the tool is for these values (assembly coordinates).
    pub fn fk(&self, q: &[f64]) -> Result<Transform, String> {
        let sol = joints::solve(&self.with(q), &self.fixed, &self.guess)?;
        let p = sol.poses.get(&self.tip).ok_or("the tip is not placed")?;
        Ok(p.then(&self.tool))
    }

    /// Every chain joint's values for `q` (to set or to play).
    pub fn joint_values(&self, q: &[f64]) -> HashMap<StableId, Vec<f64>> {
        let links = self.with(q);
        self.axes.iter().map(|a| (a.joint, links[a.link].values.clone())).collect()
    }

    fn clamp(&self, q: &mut [f64]) {
        for (v, a) in q.iter_mut().zip(&self.axes) {
            *v = v.clamp(a.min, a.max);
        }
    }

    fn error(&self, q: &[f64], target: &Target) -> Result<Vec<f64>, String> {
        let t = self.fk(q)?;
        let mut e = tf::sub(t.at, target.at).to_vec();
        if let Some(axis) = target.axis {
            let z = t.turn([0.0, 0.0, 1.0]);
            e.extend(tf::scale(tf::sub(z, crate::geometry::unit(axis)), self.size));
        }
        Ok(e)
    }

    /// Joint values putting the tool at `target`, from `start` (damped least squares, within limits).
    pub fn ik(&self, target: &Target, start: &[f64]) -> Result<IkResult, String> {
        let norm = |e: &[f64]| e.iter().map(|x| x * x).sum::<f64>().sqrt();
        let mut best: Option<(f64, Vec<f64>)> = None;
        let n = self.axes.len();
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut starts = vec![start.to_vec()];
        for _ in 0..12 {
            starts.push(self.axes.iter().map(|a| a.min + (a.max - a.min) * rand()).collect());
        }
        for s in starts {
            let mut q = s;
            self.clamp(&mut q);
            // Only from, and through, poses the interlocks allow.
            if !self.allows(&q) {
                continue;
            }
            let Ok(mut e) = self.error(&q, target) else { continue };
            let mut lambda = 1.0;
            for _ in 0..150 {
                if norm(&e) < 1e-6 {
                    break;
                }
                // Jacobian by central differences (per degree or mm).
                let mut cols = Vec::with_capacity(n);
                for k in 0..n {
                    let h = 1e-4;
                    let (mut a, mut b) = (q.clone(), q.clone());
                    a[k] += h;
                    b[k] -= h;
                    let (ea, eb) = (self.error(&a, target)?, self.error(&b, target)?);
                    cols.push(ea.iter().zip(&eb).map(|(x, y)| (x - y) / (2.0 * h)).collect::<Vec<f64>>());
                }
                let mut improved = false;
                for _ in 0..12 {
                    // (JᵀJ + λ²I) Δq = −Jᵀe
                    let mut m = vec![vec![0.0; n]; n];
                    let mut g = vec![0.0; n];
                    for i in 0..n {
                        g[i] = -cols[i].iter().zip(&e).map(|(x, y)| x * y).sum::<f64>();
                        for j in 0..n {
                            m[i][j] = cols[i].iter().zip(&cols[j]).map(|(x, y)| x * y).sum::<f64>();
                        }
                        m[i][i] += lambda * lambda;
                    }
                    let Some(d) = de_shape::mechanism::solve_dense(m, g) else {
                        lambda *= 4.0;
                        continue;
                    };
                    let mut q2: Vec<f64> = q.iter().zip(&d).map(|(a, b)| a + b).collect();
                    self.clamp(&mut q2);
                    match self.error(&q2, target) {
                        Ok(e2) if norm(&e2) < norm(&e) && self.allows(&q2) => {
                            q = q2;
                            e = e2;
                            lambda = (lambda / 3.0).max(1e-6);
                            improved = true;
                            break;
                        }
                        _ => lambda *= 4.0,
                    }
                }
                if !improved {
                    break;
                }
            }
            let err = norm(&e);
            if best.as_ref().is_none_or(|(b, _)| err < *b) {
                best = Some((err, q.clone()));
            }
            if err < 1e-4 {
                break;
            }
        }
        let (_, q) = best.ok_or(if self.interlocks.is_empty() { "the chain could not be solved from any start" } else { "the chain could not be solved from any start the interlocks allow" })?;
        let t = self.fk(&q)?;
        let pos = tf::norm(tf::sub(t.at, target.at));
        let ang = target.axis.map(|a| tf::dot(t.turn([0.0, 0.0, 1.0]), crate::geometry::unit(a)).clamp(-1.0, 1.0).acos().to_degrees()).unwrap_or(0.0);
        Ok(IkResult { reached: pos < 0.01 && ang < 0.01, error_mm: round(pos, 1e6), error_deg: round(ang, 1e6), values: self.named(&q), q: q.iter().map(|v| round(*v, 1e9)).collect(), tool: tool_pose(&t) })
    }

    fn named(&self, q: &[f64]) -> BTreeMap<String, f64> {
        self.axes.iter().zip(q).map(|(a, v)| (a.name.clone(), round(*v, 1e6))).collect()
    }

    /// The chain's description: its joints, the tool, where it is now.
    pub fn info(&self) -> Result<ChainInfo, String> {
        let q = self.now();
        Ok(ChainInfo { tip: self.tip, tip_name: self.tip_name.clone(), axes: self.axes.clone(), joints: self.joints.clone(), values: self.named(&q), tool: tool_pose(&self.fk(&q)?), tool_local: self.tool.at })
    }
}

fn round(v: f64, k: f64) -> f64 {
    (v * k).round() / k
}

fn tool_pose(t: &Transform) -> ToolPose {
    ToolPose { at: t.at.map(|v| round(v, 1e6)), axis: t.turn([0.0, 0.0, 1.0]).map(|v| round(v, 1e9)), quaternion: t.quaternion() }
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolPose {
    pub at: V3,
    /// The tool's z axis (its approach direction).
    pub axis: V3,
    pub quaternion: [f64; 4],
}

#[derive(Clone, Debug, Serialize)]
pub struct ChainInfo {
    pub tip: StableId,
    pub tip_name: String,
    pub axes: Vec<Axis>,
    pub joints: Vec<StableId>,
    pub values: BTreeMap<String, f64>,
    pub tool: ToolPose,
    /// The tool point in the tip's own coordinates.
    pub tool_local: V3,
}

/// Where the tool should be: a point, and optionally which way its axis points.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct Target {
    pub at: V3,
    #[serde(default)]
    pub axis: Option<V3>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IkResult {
    pub reached: bool,
    pub error_mm: f64,
    pub error_deg: f64,
    /// By axis name.
    pub values: BTreeMap<String, f64>,
    /// In axis order.
    pub q: Vec<f64>,
    pub tool: ToolPose,
}

/// Where the tool can get to: sampled within the joints' limits.
#[derive(Clone, Debug, Serialize)]
pub struct Reach {
    pub samples: usize,
    pub points: Vec<V3>,
    pub bounds_min: V3,
    pub bounds_max: V3,
    /// The first chain joint's centre, and the farthest and nearest the tool gets from it (mm).
    pub base: V3,
    pub max_reach: f64,
    pub min_reach: f64,
}

pub fn reach(rig: &Rig, samples: usize) -> Result<Reach, String> {
    let samples = samples.clamp(10, 20000);
    let mut seed = 0x2545f4914f6cdd1du64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    // The base: where the first moving joint's frame is now.
    let first = &rig.links[rig.axes[0].link];
    let sol = joints::solve(&rig.links, &rig.fixed, &rig.guess)?;
    let base = match &first.a {
        joints::Side::Ground(g) => g.at,
        joints::Side::Inst(id, f) => sol.poses.get(id).map(|p| p.then(f).at).unwrap_or([0.0; 3]),
        joints::Side::Elem(id, _) => sol.poses.get(id).map(|p| p.at).unwrap_or([0.0; 3]),
    };
    let mut points = vec![];
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let (mut far, mut near) = (0.0f64, f64::INFINITY);
    // The corners of the joint space first (the extremes), then random.
    let n = rig.axes.len();
    let corners = if n <= 8 { 1usize << n } else { 0 };
    for s in 0..samples {
        let q: Vec<f64> = if s < corners { rig.axes.iter().enumerate().map(|(k, a)| if s >> k & 1 == 1 { a.max } else { a.min }).collect() } else { rig.axes.iter().map(|a| a.min + (a.max - a.min) * rand()).collect() };
        // Where the interlocks forbid the arm to be is not reach.
        if !rig.allows(&q) {
            continue;
        }
        let Ok(t) = rig.fk(&q) else { continue };
        let p = t.at;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        let d = tf::norm(tf::sub(p, base));
        far = far.max(d);
        near = near.min(d);
        points.push(p.map(|v| round(v, 10.0)));
    }
    if points.is_empty() {
        return Err("no pose of the chain solves".into());
    }
    Ok(Reach { samples: points.len(), points, bounds_min: lo.map(|v| round(v, 1e3)), bounds_max: hi.map(|v| round(v, 1e3)), base: base.map(|v| round(v, 1e6)), max_reach: round(far, 1e3), min_reach: round(near, 1e3) })
}

/// A point a trajectory passes: a named pose, joint values, or a tool target.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct Waypoint {
    pub pose: Option<String>,
    pub values: Option<BTreeMap<String, Values>>,
    /// The tool's point (assembly coordinates) …
    pub to: Option<V3>,
    /// … and which way its axis points.
    pub axis: Option<V3>,
    /// Reach a `to` point along a straight line (default: joints move together).
    #[serde(default)]
    pub linear: bool,
    /// Seconds for this move (default: by the largest joint move at 60 °/s or 50 mm/s).
    pub duration: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WaypointResult {
    pub reached: bool,
    pub error_mm: f64,
    pub values: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Trajectory {
    pub axes: Vec<Axis>,
    pub waypoints: Vec<WaypointResult>,
    /// The tool's point at every frame.
    pub tool_path: Vec<V3>,
    #[serde(flatten)]
    pub played: crate::motion::Played,
}

/// The chain moved through `waypoints` from where it is: joint moves eased
/// between waypoints (or straight tool lines), `fps` frames a second, played
/// with every instance and (with `gaps`) its clashes.
pub fn trajectory(world: &World, rig: &Rig, waypoints: &[Waypoint], fps: f64, gaps: bool) -> Result<Trajectory, String> {
    let fps = if fps.is_finite() && fps > 0.0 { fps.min(60.0) } else { 12.0 };
    let mut q = rig.now();
    // Other joints a pose names (a gripper), eased alongside.
    let mut extra: HashMap<StableId, Vec<f64>> = HashMap::new();
    let mut frames: Vec<(f64, Vec<f64>, HashMap<StableId, Vec<f64>>)> = vec![(0.0, q.clone(), extra.clone())];
    let mut results = vec![];
    let mut clock = 0.0;
    for (n, w) in waypoints.iter().enumerate() {
        let mut goal = q.clone();
        let mut goal_extra = extra.clone();
        let mut path: Option<(V3, V3)> = None;
        let mut reached = (true, 0.0);
        let given: Option<BTreeMap<StableId, Vec<f64>>> = match (&w.pose, &w.values) {
            (Some(p), _) => Some(views::pose_ref(world, rig.scope, p)?.1.values),
            (None, Some(v)) => Some(views::joint_values(world, rig.scope, v)?),
            _ => None,
        };
        if let Some(values) = given {
            for (j, v) in values {
                let mut on_chain = false;
                for (k, a) in rig.axes.iter().enumerate() {
                    if a.joint == j
                        && let Some(x) = v.get(a.index)
                    {
                        goal[k] = *x;
                        on_chain = true;
                    }
                }
                if !on_chain {
                    goal_extra.insert(j, v);
                }
            }
        } else if let Some(to) = w.to {
            let r = rig.ik(&Target { at: to, axis: w.axis }, &q)?;
            reached = (r.reached, r.error_mm);
            goal = r.q.clone();
            if w.linear {
                path = Some((rig.fk(&q)?.at, to));
            }
        } else {
            return Err(format!("waypoint {}: give a `pose`, `values` or a point `to`", n + 1));
        }
        for (v, a) in goal.iter_mut().zip(&rig.axes) {
            if *v < a.min - 1e-6 || *v > a.max + 1e-6 {
                return Err(format!("waypoint {}: {} {:.2} is past its limits {}…{}", n + 1, a.name, v, a.min, a.max));
            }
        }
        if !rig.allows(&goal) {
            let mut values = crate::interlocks::values_now(world, rig.scope);
            values.extend(rig.joint_values(&goal));
            values.extend(goal_extra.clone());
            let why = crate::interlocks::violation(world, rig.scope, &values).unwrap_or_else(|| "an interlock forbids it".into());
            return Err(format!("waypoint {}: {why}", n + 1));
        }
        let biggest = goal.iter().zip(&q).zip(&rig.axes).map(|((g, c), a)| (g - c).abs() / if a.unit == "deg" { 60.0 } else { 50.0 }).fold(0.0, f64::max);
        let duration = w.duration.unwrap_or(biggest.max(0.2)).max(1e-3);
        let steps = ((duration * fps).ceil() as usize).max(1);
        let ease = |x: f64| x * x * (3.0 - 2.0 * x);
        let mut cur = q.clone();
        for s in 1..=steps {
            let x = s as f64 / steps as f64;
            let qs: Vec<f64> = match path {
                Some((from, to)) => {
                    // Along the straight line, each point reached from the last.
                    let p = tf::add(from, tf::scale(tf::sub(to, from), ease(x)));
                    let r = rig.ik(&Target { at: p, axis: w.axis }, &cur)?;
                    if !r.reached {
                        return Err(format!("waypoint {}: the straight line leaves the reach {:.1} mm from it, {:.0}% along", n + 1, r.error_mm, x * 100.0));
                    }
                    r.q
                }
                None => q.iter().zip(&goal).map(|(a, b)| a + (b - a) * ease(x)).collect(),
            };
            let ex: HashMap<StableId, Vec<f64>> = goal_extra
                .iter()
                .map(|(j, g)| {
                    let from = extra.get(j).cloned().or_else(|| joints::link(world, *j, None).ok().map(|l| l.values)).unwrap_or_else(|| g.clone());
                    (*j, from.iter().zip(g).map(|(a, b)| a + (b - a) * ease(x)).collect())
                })
                .collect();
            cur = qs.clone();
            frames.push((clock + duration * x, qs, ex));
        }
        clock += duration;
        q = goal.clone();
        if path.is_some() {
            q = cur.clone();
        }
        extra = goal_extra;
        results.push(WaypointResult { reached: reached.0, error_mm: round(reached.1, 1e6), values: rig.named(&q) });
    }
    let tool_path = frames.iter().map(|(_, qs, _)| rig.fk(qs).map(|t| t.at.map(|v| round(v, 1e4)))).collect::<Result<Vec<_>, _>>()?;
    let labelled: Vec<(f64, HashMap<StableId, Vec<f64>>)> = frames
        .iter()
        .map(|(t, qs, ex)| {
            let mut v = rig.joint_values(qs);
            v.extend(ex.clone());
            (*t, v)
        })
        .collect();
    let played = crate::motion::play(world, rig.scope, &labelled, gaps, "s", "the trajectory")?;
    Ok(Trajectory { axes: rig.axes.clone(), waypoints: results, tool_path, played })
}

// ---- The controller's configuration ------------------------------------------------------

/// One joint as the arm's controller drives it: a channel, its limits, where home is.
#[derive(Clone, Debug, Serialize)]
pub struct ControllerJoint {
    /// Its index in the controller's arrays (base first).
    pub channel: usize,
    pub joint: StableId,
    pub name: String,
    /// A C identifier for it: `J_SHOULDER`.
    pub symbol: String,
    pub key: String,
    /// "deg" or "mm".
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub home: f64,
    /// On the chain from the base to the tip (else named by an interlock only).
    pub chain: bool,
}

/// A range of one channel, as the controller tests it.
#[derive(Clone, Debug, Serialize)]
pub struct ChannelRange {
    pub channel: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControllerZone {
    pub when: Vec<ChannelRange>,
    pub then: Vec<ChannelRange>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControllerTable {
    pub by: usize,
    pub channel: usize,
    /// [by, min, max], linear between rows and held past the ends.
    pub rows: Vec<[f64; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControllerInterlock {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// In words, as the model states it.
    pub text: Vec<String>,
    pub zones: Vec<ControllerZone>,
    pub tables: Vec<ControllerTable>,
}

/// What the arm's controller firmware needs to keep it where the model says
/// it may go: every joint's channel, limits and home, and the interlocks
/// between them, by channel.
#[derive(Clone, Debug, Serialize)]
pub struct ControllerConfig {
    pub arm: String,
    pub tip: String,
    pub joints: Vec<ControllerJoint>,
    pub interlocks: Vec<ControllerInterlock>,
}

fn symbol(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    format!("J_{}", s.trim_matches('_'))
}

/// The controller's configuration for the chain `rig` moves.
pub fn controller(world: &World, rig: &Rig) -> Result<ControllerConfig, String> {
    let home = views::poses_in(world, rig.scope).into_iter().find(|(_, p)| p.name.eq_ignore_ascii_case("home")).map(|(_, p)| p.values);
    let now = rig.now();
    let mut joints: Vec<ControllerJoint> = rig
        .axes
        .iter()
        .enumerate()
        .map(|(k, a)| ControllerJoint {
            channel: k,
            joint: a.joint,
            name: a.name.clone(),
            symbol: symbol(&a.name),
            key: a.key.clone(),
            unit: a.unit.clone(),
            min: a.min,
            max: a.max,
            home: home.as_ref().and_then(|h| h.get(&a.joint)).and_then(|v| v.get(a.index)).copied().unwrap_or(now[k]),
            chain: true,
        })
        .collect();
    // Joints an interlock names that are off the chain (a gripper) get channels too.
    let index = crate::interlocks::indices(world, &rig.interlocks);
    let mut keyed: Vec<(StableId, Option<String>)> = index.keys().cloned().collect();
    keyed.sort();
    for (j, key) in &keyed {
        let i = index[&(*j, key.clone())];
        if joints.iter().any(|c| c.joint == *j && rig.axes.iter().any(|a| a.joint == *j && a.index == i)) {
            continue;
        }
        let e = entity(world, *j).ok_or("an interlock names a joint that is gone")?;
        let (kind, set) = (world.get::<joints::Joint>(e).ok_or("not a joint")?.kind, world.get::<ParameterSet>(e).ok_or("a joint has no values")?);
        let (k, _, unit) = kind.values()[i];
        let p = set.params.iter().find(|p| p.key == k).ok_or("a joint has no value")?;
        let v = de_shape::params::resolve(world, &set.params)?.get(k).copied().unwrap_or(p.value);
        let name = if kind.values().len() > 1 { format!("{} {k}", set.name) } else { set.name.clone() };
        joints.push(ControllerJoint {
            channel: joints.len(),
            joint: *j,
            symbol: symbol(&name),
            name,
            key: k.to_string(),
            unit: unit.to_string(),
            min: p.min.unwrap_or(f64::NEG_INFINITY),
            max: p.max.unwrap_or(f64::INFINITY),
            home: home.as_ref().and_then(|h| h.get(j)).and_then(|x| x.get(i)).copied().unwrap_or(v),
            chain: false,
        });
    }
    let channel = |j: StableId, key: &Option<String>| -> Result<usize, String> {
        let i = index.get(&(j, key.clone())).copied().ok_or("an interlock names a joint that is gone")?;
        let name_key = entity(world, j).and_then(|e| world.get::<joints::Joint>(e)).map(|x| x.kind.values()[i].0).unwrap_or("");
        joints.iter().find(|c| c.joint == j && c.key == name_key).map(|c| c.channel).ok_or_else(|| "an interlock names a joint with no channel".into())
    };
    let mut interlocks = vec![];
    for (_, il) in &rig.interlocks {
        let range = |r: &crate::interlocks::JointRange| -> Result<ChannelRange, String> { Ok(ChannelRange { channel: channel(r.joint, &r.key)?, min: r.min, max: r.max }) };
        let mut zones = vec![];
        let mut text = vec![];
        let words = |r: &ChannelRange| {
            let c = &joints[r.channel];
            match (r.min, r.max) {
                (Some(a), Some(b)) => format!("{} {a}…{b} {}", c.name, c.unit),
                (Some(a), None) => format!("{} ≥ {a} {}", c.name, c.unit),
                (None, Some(b)) => format!("{} ≤ {b} {}", c.name, c.unit),
                (None, None) => c.name.clone(),
            }
        };
        for z in &il.zones {
            let when = z.when.iter().map(range).collect::<Result<Vec<_>, _>>()?;
            let then = z.then.iter().map(range).collect::<Result<Vec<_>, _>>()?;
            text.push(format!("when {}: {}", when.iter().map(words).collect::<Vec<_>>().join(" and "), then.iter().map(words).collect::<Vec<_>>().join(", ")));
            zones.push(ControllerZone { when, then });
        }
        let mut tables = vec![];
        for t in &il.tables {
            let (by, ch) = (channel(t.by, &t.by_key)?, channel(t.joint, &t.key)?);
            text.push(format!("{} follows {} through {} rows", joints[ch].name, joints[by].name, t.rows.len()));
            tables.push(ControllerTable { by, channel: ch, rows: t.rows.clone() });
        }
        interlocks.push(ControllerInterlock { name: il.name.clone(), description: il.description.clone(), text, zones, tables });
    }
    let arm = match rig.scope {
        Some(a) => entity(world, a).and_then(|e| world.get::<crate::AssemblyDef>(e)).map(|d| d.name.clone()).unwrap_or_default(),
        None => "Project".into(),
    };
    Ok(ControllerConfig { arm, tip: rig.tip_name.clone(), joints, interlocks })
}

/// The configuration as a C header for an Arduino (or any C/C++) controller:
/// the channels, limits and home pose as arrays, `arm_allowed(q)` (whether a
/// pose is inside every limit and interlock) and `arm_clamp(q)` (a commanded
/// pose pulled back inside them: limits first, then each interlock's held
/// joints into their allowed range).
pub fn arduino_header(cfg: &ControllerConfig) -> String {
    use std::fmt::Write as _;
    let n = cfg.joints.len();
    let num = |v: f64| if v.is_finite() { format!("{v:.3}f") } else if v > 0.0 { "INFINITY".into() } else { "-INFINITY".into() };
    let list = |f: &dyn Fn(&ControllerJoint) -> f64| cfg.joints.iter().map(|j| num(f(j))).collect::<Vec<_>>().join(", ");
    let mut h = String::new();
    let _ = writeln!(h, "// {} — joint limits and interlocks for the arm's controller.", cfg.arm);
    let _ = writeln!(h, "// Generated by Forma from the model (GET /robot/{}?controller=arduino); regenerate when", cfg.tip);
    let _ = writeln!(h, "// the joints or interlocks change. Angles in degrees, slides in mm, as the model's joints.");
    let _ = writeln!(h, "#pragma once\n#include <math.h>\n#include <stdbool.h>\n");
    let _ = writeln!(h, "#define ARM_JOINTS {n}");
    let _ = writeln!(h, "enum ArmJoint {{ {} }};", cfg.joints.iter().map(|j| format!("{} = {}", j.symbol, j.channel)).collect::<Vec<_>>().join(", "));
    let _ = writeln!(h, "static const char *const ARM_NAMES[ARM_JOINTS] = {{ {} }};", cfg.joints.iter().map(|j| format!("{:?}", j.name)).collect::<Vec<_>>().join(", "));
    let _ = writeln!(h, "static const float ARM_MIN[ARM_JOINTS] = {{ {} }};", list(&|j| j.min));
    let _ = writeln!(h, "static const float ARM_MAX[ARM_JOINTS] = {{ {} }};", list(&|j| j.max));
    let _ = writeln!(h, "static const float ARM_HOME[ARM_JOINTS] = {{ {} }};\n", list(&|j| j.home));
    // Tables as arrays.
    let mut t_id = 0;
    for il in &cfg.interlocks {
        for t in &il.tables {
            let _ = writeln!(h, "// {}: {} limits by {}.", il.name, cfg.joints[t.channel].name, cfg.joints[t.by].name);
            let _ = writeln!(h, "static const float ARM_TABLE_{t_id}[{}][3] = {{ {} }};", t.rows.len(), t.rows.iter().map(|r| format!("{{ {}, {}, {} }}", num(r[0]), num(r[1]), num(r[2]))).collect::<Vec<_>>().join(", "));
            t_id += 1;
        }
    }
    let _ = writeln!(h, "\n/* A table's [min, max] at x: linear between rows, held past the ends. */");
    let _ = writeln!(h, "static inline void arm_table_at(const float (*rows)[3], int n, float x, float *lo, float *hi) {{");
    let _ = writeln!(h, "  if (x <= rows[0][0]) {{ *lo = rows[0][1]; *hi = rows[0][2]; return; }}");
    let _ = writeln!(h, "  for (int i = 1; i < n; i++) {{");
    let _ = writeln!(h, "    if (x <= rows[i][0]) {{");
    let _ = writeln!(h, "      float t = rows[i][0] > rows[i - 1][0] ? (x - rows[i - 1][0]) / (rows[i][0] - rows[i - 1][0]) : 1.0f;");
    let _ = writeln!(h, "      *lo = rows[i - 1][1] + (rows[i][1] - rows[i - 1][1]) * t; *hi = rows[i - 1][2] + (rows[i][2] - rows[i - 1][2]) * t; return;");
    let _ = writeln!(h, "    }}\n  }}\n  *lo = rows[n - 1][1]; *hi = rows[n - 1][2];\n}}\n");
    let cond = |r: &ChannelRange| -> String {
        let s = &cfg.joints[r.channel].symbol;
        match (r.min, r.max) {
            (Some(a), Some(b)) => format!("(q[{s}] >= {} && q[{s}] <= {})", num(a), num(b)),
            (Some(a), None) => format!("q[{s}] >= {}", num(a)),
            (None, Some(b)) => format!("q[{s}] <= {}", num(b)),
            (None, None) => "true".into(),
        }
    };
    // arm_allowed
    let _ = writeln!(h, "/* Whether pose q is inside every joint's limits and every interlock. */");
    let _ = writeln!(h, "static inline bool arm_allowed(const float q[ARM_JOINTS]) {{");
    let _ = writeln!(h, "  for (int i = 0; i < ARM_JOINTS; i++) if (q[i] < ARM_MIN[i] || q[i] > ARM_MAX[i]) return false;");
    let mut t_id = 0;
    for il in &cfg.interlocks {
        let _ = writeln!(h, "  /* {}: {} */", il.name, il.text.join("; "));
        for z in &il.zones {
            let when = z.when.iter().map(cond).collect::<Vec<_>>().join(" && ");
            let then = z.then.iter().map(cond).collect::<Vec<_>>().join(" && ");
            let _ = writeln!(h, "  if ({when} && !({then})) return false;");
        }
        for t in &il.tables {
            let (by, ch) = (&cfg.joints[t.by].symbol, &cfg.joints[t.channel].symbol);
            let _ = writeln!(h, "  {{ float lo, hi; arm_table_at(ARM_TABLE_{t_id}, {}, q[{by}], &lo, &hi); if (q[{ch}] < lo || q[{ch}] > hi) return false; }}", t.rows.len());
            t_id += 1;
        }
    }
    let _ = writeln!(h, "  return true;\n}}\n");
    // arm_clamp
    let _ = writeln!(h, "/* Pulls a commanded pose back inside: each joint into its limits, then each");
    let _ = writeln!(h, "   interlock's held joints into the range it allows where the others stand. */");
    let _ = writeln!(h, "static inline void arm_clamp(float q[ARM_JOINTS]) {{");
    let _ = writeln!(h, "  for (int i = 0; i < ARM_JOINTS; i++) q[i] = fminf(fmaxf(q[i], ARM_MIN[i]), ARM_MAX[i]);");
    let mut t_id = 0;
    for il in &cfg.interlocks {
        let _ = writeln!(h, "  /* {} */", il.name);
        for z in &il.zones {
            let when = z.when.iter().map(cond).collect::<Vec<_>>().join(" && ");
            let _ = writeln!(h, "  if ({when}) {{");
            for r in &z.then {
                let s = &cfg.joints[r.channel].symbol;
                if let Some(a) = r.min {
                    let _ = writeln!(h, "    q[{s}] = fmaxf(q[{s}], {});", num(a));
                }
                if let Some(b) = r.max {
                    let _ = writeln!(h, "    q[{s}] = fminf(q[{s}], {});", num(b));
                }
            }
            let _ = writeln!(h, "  }}");
        }
        for t in &il.tables {
            let (by, ch) = (&cfg.joints[t.by].symbol, &cfg.joints[t.channel].symbol);
            let _ = writeln!(h, "  {{ float lo, hi; arm_table_at(ARM_TABLE_{t_id}, {}, q[{by}], &lo, &hi); q[{ch}] = fminf(fmaxf(q[{ch}], lo), hi); }}", t.rows.len());
            t_id += 1;
        }
    }
    let _ = writeln!(h, "}}");
    h
}

// ---- Command -------------------------------------------------------------------------------

/// `assembly.robot.move_to`: set the chain's joints so the tool reaches a point
/// (and points its axis a way), within the joints' limits.
#[derive(Deserialize, JsonSchema)]
pub struct MoveTo {
    /// The instance at the end of the chain (the gripper, the tool).
    pub tip: StableId,
    /// A connector on the tip for the tool point (default TCP / Tool / Tip, else its middle).
    pub tool: Option<String>,
    /// Or a point in the tip's own coordinates.
    pub point: Option<V3>,
    /// The tool's point, assembly coordinates (mm).
    pub to: V3,
    pub axis: Option<V3>,
}

impl Command for MoveTo {
    const NAME: &'static str = "assembly.robot.move_to";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let rig = Rig::new(tx.world(), self.tip, self.tool.as_deref(), self.point).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
        let r = rig.ik(&Target { at: self.to, axis: self.axis }, &rig.now()).map_err(|m| bad(ErrorCode::InvalidGeometry, m))?;
        if !r.reached {
            return Err(bad(ErrorCode::InvalidGeometry, format!("out of reach: the closest the tool gets is {:.2} mm away{} (joints {})", r.error_mm, if r.error_deg > 0.01 { format!(", its axis {:.1}° off", r.error_deg) } else { String::new() }, r.values.iter().map(|(k, v)| format!("{k} {v:.1}")).collect::<Vec<_>>().join(", "))));
        }
        let values: BTreeMap<StableId, Vec<f64>> = rig.joint_values(&r.q).into_iter().collect();
        views::set_joints(tx, &values)?;
        Ok(json!({ "values": r.values, "tool": r.tool }))
    }
}

/// A tip given by id or instance name in an assembly (or the top).
pub fn tip_ref(world: &World, r: &str) -> Result<StableId, String> {
    if let Ok(id) = serde_json::from_value::<StableId>(json!(r))
        && entity(world, id).and_then(|e| world.get::<Instance>(e)).is_some()
    {
        return Ok(id);
    }
    world.resource::<IdIndex>().iter().filter(|(_, e)| world.get::<Instance>(*e).is_some_and(|i| i.name == r)).map(|(id, _)| id).min().ok_or_else(|| format!("no instance “{r}”"))
}
