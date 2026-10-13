//! Degrees of freedom: what in an assembly can still move, and what is held
//! more than once or cannot all hold.
//!
//! For every group of joined instances the constraints (joints and mates)
//! are linearised where they hold now. The rank of that system says how many
//! of the moving instances' 6 freedoms each it takes away; what is left is
//! found per instance and put in words (turns about this axis through this
//! point, slides along this direction). Two counts:
//!
//! - `free`: freedoms nothing determines with every joint at its value — a
//!   part held only by a face-to-face mate still slides and turns on it.
//!   Zero for a fully defined assembly.
//! - `mobility`: freedoms with the joints' values let go — how the mechanism
//!   can move (6 for a six-axis arm, 1 for a four-bar). A joint whose value
//!   follows others (a coupling's follower, or a formula reading other
//!   joints' values: two gripper fingers geared to one pinion) is not let
//!   go on its own: it moves with what drives it, so it adds no freedom.
//!
//! Redundant constraints are those the others already imply (a part bolted
//! at two places, a four-bar's hinges, which ideal parts allow but real parts
//! must be made to fit); each joint that shares in it is named. When the
//! joints cannot all hold, the joints whose removal lets the rest hold are
//! named as conflicting.

use crate::joints::{self, Link};
use crate::transform::{self as tf, Transform, V3};
use crate::{Instance, placement};
use de_core::{IdIndex, StableId, bevy_ecs::world::World};
use de_shape::params::ParameterSet;
use serde::Serialize;
use std::collections::HashMap;

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

#[derive(Clone, Debug, Serialize)]
pub struct JointRef {
    pub id: StableId,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstanceDof {
    pub id: StableId,
    pub name: String,
    /// Freedoms nothing determines (joints at their values).
    pub free: usize,
    /// What still moves, in words.
    pub motions: Vec<String>,
    /// Freedoms with the joints' values let go.
    pub mobility: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Redundant {
    pub id: StableId,
    pub name: String,
    /// How many of its conditions the other joints already impose.
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupDof {
    pub joints: Vec<JointRef>,
    /// Hangs from the ground, or from this anchor instance.
    pub grounded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<StableId>,
    pub solved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub free: usize,
    pub mobility: usize,
    pub instances: Vec<InstanceDof>,
    /// Conditions imposed more than once, and the joints that share them.
    pub redundant: usize,
    pub redundant_joints: Vec<Redundant>,
    /// When the joints cannot all hold: those whose removal lets the rest hold.
    pub conflicting: Vec<JointRef>,
    /// Joints whose values follow other joints (couplings, formulas): they move with them, adding no freedom.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub following: Vec<Following>,
    pub summary: String,
}

/// A joint whose value follows others', and the joints it follows.
#[derive(Clone, Debug, Serialize)]
pub struct Following {
    pub id: StableId,
    pub name: String,
    pub drivers: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DofReport {
    pub assembly: Option<StableId>,
    pub groups: Vec<GroupDof>,
    /// Instances no joint touches: free in all six ways (where they are put).
    pub unjoined: Vec<JointRef>,
    pub free: usize,
    pub mobility: usize,
    pub summary: String,
}

/// Reduced row echelon form in place; the pivot columns.
pub fn rref(m: &mut [Vec<f64>], tol: f64) -> Vec<usize> {
    let rows = m.len();
    let cols = m.first().map_or(0, |r| r.len());
    let mut pivots = vec![];
    let mut r = 0;
    for c in 0..cols {
        if r >= rows {
            break;
        }
        let Some(p) = (r..rows).max_by(|&a, &b| m[a][c].abs().total_cmp(&m[b][c].abs())) else { break };
        if m[p][c].abs() <= tol {
            continue;
        }
        m.swap(r, p);
        let d = m[r][c];
        for v in m[r].iter_mut() {
            *v /= d;
        }
        for i in 0..rows {
            if i != r && m[i][c] != 0.0 {
                let f = m[i][c];
                let (src, dst) = if i < r { let (a, b) = m.split_at_mut(r); (&b[0], &mut a[i]) } else { let (a, b) = m.split_at_mut(i); (&a[r], &mut b[0]) };
                for (x, y) in dst.iter_mut().zip(src.iter()) {
                    *x -= f * y;
                }
            }
        }
        pivots.push(c);
        r += 1;
    }
    pivots
}

pub fn rank(rows: &[Vec<f64>], tol: f64) -> usize {
    let mut m = rows.to_vec();
    rref(&mut m, tol).len()
}

/// A basis of the null space of the matrix with these rows (n columns).
pub fn null_space(rows: &[Vec<f64>], n: usize, tol: f64) -> Vec<Vec<f64>> {
    let mut m = rows.to_vec();
    let pivots = rref(&mut m, tol);
    (0..n)
        .filter(|c| !pivots.contains(c))
        .map(|f| {
            let mut v = vec![0.0; n];
            v[f] = 1.0;
            for (r, &p) in pivots.iter().enumerate() {
                v[p] = -m[r][f];
            }
            v
        })
        .collect()
}

/// The poses with moving instance `k / 6` nudged by `h` along freedom `k % 6`
/// (x, y, z in mm; turns about x, y, z in radians).
fn nudge(p: &HashMap<StableId, Transform>, moving: &[StableId], k: usize, h: f64) -> HashMap<StableId, Transform> {
    let mut q = p.clone();
    let (i, c) = (k / 6, k % 6);
    let t = p[&moving[i]];
    let moved = if c < 3 {
        let mut d = [0.0; 3];
        d[c] = h;
        Transform { at: tf::add(t.at, d), rot: t.rot }
    } else {
        let mut w = [0.0; 3];
        w[c - 3] = 1.0;
        Transform { at: t.at, rot: tf::matmul(&tf::rotation_about(w, h), &t.rot) }
    };
    q.insert(moving[i], moved);
    q
}

fn step_size(k: usize, size: f64) -> f64 {
    if k % 6 < 3 { 1e-6 * size } else { 1e-6 }
}

/// The constraint rows of each link (and the moving instances' columns) at `poses`.
fn jacobian(links: &[Link], moving: &[StableId], poses: &HashMap<StableId, Transform>, size: f64, release: &dyn Fn(&Link) -> bool) -> Vec<Vec<Vec<f64>>> {
    let n = 6 * moving.len();
    links
        .iter()
        .map(|l| {
            let mut base = vec![];
            let release = release(l);
            l.residuals_with(poses, size, release, &mut base);
            let mut cols = vec![];
            for k in 0..n {
                let h = step_size(k, size);
                let (mut a, mut b) = (vec![], vec![]);
                l.residuals_with(&nudge(poses, moving, k, h), size, release, &mut a);
                l.residuals_with(&nudge(poses, moving, k, -h), size, release, &mut b);
                cols.push(a.iter().zip(&b).map(|(x, y)| (x - y) / (2.0 * h)).collect::<Vec<f64>>());
            }
            (0..base.len()).map(|r| cols.iter().map(|c| c.get(r).copied().unwrap_or(0.0)).collect()).collect()
        })
        .collect()
}

/// One joint value that others determine: moving its drivers by dv moves it by Σ c·dv.
struct Follow {
    link: usize,
    index: usize,
    /// (link, value index, follower units per driver unit), for drivers in this group.
    drivers: Vec<(usize, usize, f64)>,
    /// Every joint it follows, by name (in this group or not).
    names: Vec<String>,
}

/// The joint values in `links` that follow other joints: couplings' followers,
/// and values whose formulas read other joints' values (the slope worked out
/// from the formula at the drivers' values now). A value following joints
/// outside the group has no drivers here: it is held.
fn follows(world: &World, scope: Option<StableId>, links: &[Link]) -> Vec<Follow> {
    use crate::couplings::{self, CouplingKind};
    let at = |id: StableId| links.iter().position(|l| l.id == id);
    let list = couplings::couplings_in(world, scope);
    let mut out: Vec<Follow> = vec![];
    for (li, l) in links.iter().enumerate() {
        if l.free || l.kind.mate().is_some() {
            continue;
        }
        for (index, (key, _, _)) in l.kind.values().iter().enumerate() {
            let mut f = Follow { link: li, index, drivers: vec![], names: vec![] };
            // A coupling sets the value whatever its own formula says.
            for (_, c) in list.iter().filter(|(_, c)| c.follower == l.id) {
                if couplings::key_index(world, c.follower, c.follower_key.as_deref()).map(|k| k.0) != Ok(index) {
                    continue;
                }
                f.names.push(joints::joint_name(world, c.driver));
                let (Some(d), Ok((di, _))) = (at(c.driver), couplings::key_index(world, c.driver, c.driver_key.as_deref())) else { continue };
                let slope = if c.kind == CouplingKind::Cam {
                    let v = links[d].values.get(di).copied().unwrap_or(0.0);
                    (couplings::cam_at(&c.profile, v + 0.01) - couplings::cam_at(&c.profile, v - 0.01)) / 0.02
                } else {
                    c.ratio
                };
                f.drivers.push((d, di, slope));
            }
            if f.names.is_empty()
                && let Some(set) = entity(world, l.id).and_then(|e| world.get::<ParameterSet>(e))
                && let Some(p) = set.params.iter().find(|p| p.key == *key && p.expr.is_some())
            {
                for r in de_shape::params::references(std::slice::from_ref(p)) {
                    let Some(rj) = entity(world, r).and_then(|e| world.get::<joints::Joint>(e)) else { continue };
                    if r == l.id {
                        continue;
                    }
                    f.names.push(joints::joint_name(world, r));
                    let Some(d) = at(r) else { continue };
                    for (di, (dk, _, _)) in rj.kind.values().iter().enumerate() {
                        let base = links[d].values.get(di).copied().unwrap_or(0.0);
                        let value = |dv: f64| {
                            let given = |id: StableId, k: &str| (id == r && k == *dk).then_some(base + dv);
                            de_shape::params::resolve_with(world, &set.params, &given).ok().and_then(|m| m.get(*key).copied())
                        };
                        let h = 1e-3;
                        // One-sided at a limit (a finger closed at 0 mm cannot go below).
                        let slope = match (value(h), value(-h), value(0.0)) {
                            (Some(a), Some(b), _) => (a - b) / (2.0 * h),
                            (Some(a), None, Some(c)) => (a - c) / h,
                            (None, Some(b), Some(c)) => (c - b) / h,
                            _ => 0.0,
                        };
                        if slope.abs() > 1e-12 {
                            f.drivers.push((d, di, slope));
                        }
                    }
                }
            }
            if !f.names.is_empty() {
                f.names.dedup();
                out.push(f);
            }
        }
    }
    out
}

/// Rows saying each following value moves only with its drivers:
/// d(value) − Σ c·d(driver) = 0, turns scaled by `size` like the joints' own rows.
fn follow_rows(links: &[Link], follows: &[Follow], moving: &[StableId], poses: &HashMap<StableId, Transform>, size: f64) -> Vec<Vec<f64>> {
    let n = 6 * moving.len();
    let scale = |l: &Link, i: usize| if l.kind.values().get(i).is_some_and(|v| v.2 == "deg") { size.to_radians() } else { 1.0 };
    let g = |f: &Follow, p: &HashMap<StableId, Transform>| -> Option<f64> {
        let l = &links[f.link];
        let s = scale(l, f.index);
        let mut v = l.measure(p)?.get(f.index)? * s;
        for &(d, di, c) in &f.drivers {
            v -= c * links[d].measure(p)?.get(di)? * s;
        }
        Some(v)
    };
    follows
        .iter()
        .filter_map(|f| {
            g(f, poses)?;
            (0..n)
                .map(|k| {
                    let h = step_size(k, size);
                    Some((g(f, &nudge(poses, moving, k, h))? - g(f, &nudge(poses, moving, k, -h))?) / (2.0 * h))
                })
                .collect::<Option<Vec<f64>>>()
        })
        .collect()
}

fn fmt_v(v: V3) -> String {
    let r = |x: f64| {
        let y = (x * 1000.0).round() / 1000.0;
        if y == 0.0 { 0.0 } else { y }
    };
    format!("({}, {}, {})", r(v[0]), r(v[1]), r(v[2]))
}

/// What one instance can still do, from twists [v (mm), ω (rad)] about its origin `at`.
pub fn describe(twists: &[[f64; 6]], at: V3, tol: f64) -> Vec<String> {
    // Rotation parts first, so pure slides come last.
    let mut m: Vec<Vec<f64>> = twists.iter().map(|t| vec![t[3], t[4], t[5], t[0], t[1], t[2]]).collect();
    let piv = rref(&mut m, tol);
    let mut out = vec![];
    for (r, &p) in piv.iter().enumerate() {
        let row = &m[r];
        let (w, v) = ([row[0], row[1], row[2]], [row[3], row[4], row[5]]);
        if p < 3 {
            let wn = tf::norm(w);
            let axis = tf::scale(w, 1.0 / wn);
            // A point on the screw axis: at + ω × v / |ω|²; pitch ω·v / |ω|².
            let point = tf::add(at, tf::scale(tf::cross(w, v), 1.0 / (wn * wn)));
            let pitch = tf::dot(w, v) / (wn * wn);
            if pitch.abs() < 1e-6 {
                out.push(format!("turns about {} through {}", fmt_v(axis), fmt_v(point)));
            } else {
                out.push(format!("screws about {} through {} ({:.3} mm per radian)", fmt_v(axis), fmt_v(point), pitch));
            }
        } else {
            out.push(format!("slides along {}", fmt_v(tf::scale(v, 1.0 / tf::norm(v)))));
        }
    }
    out
}

fn name_of(world: &World, id: StableId) -> String {
    entity(world, id).and_then(|e| world.get::<Instance>(e)).map(|i| i.name.clone()).unwrap_or_else(|| id.to_string())
}

/// The degrees of freedom of every group of joined instances in an assembly (or the top).
pub fn report(world: &World, scope: Option<StableId>) -> Result<DofReport, String> {
    let all = joints::joints_in(world, scope);
    let coupled = crate::couplings::coupled_values(world, scope, &HashMap::new()).unwrap_or(None);
    let mut groups = vec![];
    let mut joined = vec![];
    for g in joints::groups(&all) {
        joined.extend(g.members.iter().copied());
        let links: Vec<Link> = g.joints.iter().map(|j| joints::link(world, *j, coupled.as_ref().and_then(|c| c.get(j)).map(|v| v.as_slice()))).collect::<Result<_, _>>()?;
        let jref = |l: &Link| JointRef { id: l.id, name: l.name.clone(), kind: l.kind.word().into() };
        let stored = |m: StableId| entity(world, m).and_then(|x| world.get::<Instance>(x)).map(Instance::transform);
        let mut fixed = HashMap::new();
        if let Some(a) = g.anchor
            && let Some(t) = stored(a)
        {
            fixed.insert(a, t);
        }
        let mut guess = HashMap::new();
        for m in g.held() {
            if let Some(t) = entity(world, m).and_then(|x| placement(world, x)) {
                guess.insert(m, t);
            }
        }
        let (poses, solved, error) = match joints::solve(&links, &fixed, &guess) {
            Ok(s) => (s.poses, true, None),
            Err(e) => {
                let mut p = guess.clone();
                p.extend(fixed.clone());
                (p, false, Some(e))
            }
        };
        let mut conflicting = vec![];
        if !solved && links.len() > 1 {
            for i in 0..links.len() {
                let rest: Vec<Link> = links.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, l)| l.clone()).collect();
                if joints::solve(&rest, &fixed, &guess).is_ok() {
                    conflicting.push(jref(&links[i]));
                }
            }
        }
        let mut moving: Vec<StableId> = g.members.iter().copied().filter(|m| !fixed.contains_key(m) && poses.contains_key(m)).collect();
        moving.sort();
        let n = 6 * moving.len();
        let size = joints::size_of(&links);
        let held_rows = jacobian(&links, &moving, &poses, size, &|_| false);
        let rel_rows = jacobian(&links, &moving, &poses, size, &|_| true);
        // For repeated conditions a mate's distance or angle is a dimension, held; a joint's value is its motion.
        let red_rows = jacobian(&links, &moving, &poses, size, &|l| l.kind.mate().is_none());
        let flat = |blocks: &[Vec<Vec<f64>>]| blocks.iter().flatten().cloned().collect::<Vec<Vec<f64>>>();
        let scale = flat(&rel_rows).iter().flatten().fold(1.0f64, |m, v| m.max(v.abs()));
        let tol = 1e-6 * scale;
        // Values that follow other joints move only with them.
        let follow = follows(world, scope, &links);
        let (held_all, mut rel_all) = (flat(&held_rows), flat(&rel_rows));
        rel_all.extend(follow_rows(&links, &follow, &moving, &poses, size));
        let mut following: Vec<Following> = vec![];
        for f in &follow {
            let l = &links[f.link];
            if !following.iter().any(|x| x.id == l.id) {
                following.push(Following { id: l.id, name: l.name.clone(), drivers: f.names.clone() });
            }
        }
        let null_held = null_space(&held_all, n, tol);
        let null_rel = null_space(&rel_all, n, tol);
        let rank_red = rank(&flat(&red_rows), tol);
        let own: Vec<usize> = red_rows.iter().map(|b| rank(b, tol)).collect();
        let repeated = own.iter().sum::<usize>().saturating_sub(rank_red);
        let mut redundant_joints = vec![];
        if repeated > 0 {
            for (i, l) in links.iter().enumerate() {
                let without: Vec<Vec<f64>> = red_rows.iter().enumerate().filter(|(k, _)| *k != i).flat_map(|(_, b)| b.clone()).collect();
                let shared = own[i].saturating_sub(rank_red - rank(&without, tol));
                // Mates overlap in part as a matter of course (three faces
                // mated square each fix two turns): a mate counts only when
                // the others imply all of it; a joint (or lock) whenever they
                // imply any of it (bolted twice, a closed loop).
                let counts = if l.kind.mate().is_some() { shared > 0 && shared == own[i] } else { shared > 0 };
                if counts {
                    redundant_joints.push(Redundant { id: l.id, name: l.name.clone(), count: shared });
                }
            }
        }
        let redundant = repeated.min(redundant_joints.iter().map(|r| r.count).sum());
        let per = |basis: &[Vec<f64>], k: usize| -> Vec<[f64; 6]> {
            basis.iter().map(|v| [v[6 * k], v[6 * k + 1], v[6 * k + 2], v[6 * k + 3], v[6 * k + 4], v[6 * k + 5]]).filter(|t| t.iter().any(|x| x.abs() > 1e-9)).collect()
        };
        let instances: Vec<InstanceDof> = moving
            .iter()
            .enumerate()
            .map(|(k, id)| {
                let (h, r) = (per(&null_held, k), per(&null_rel, k));
                let rows = |t: &[[f64; 6]]| t.iter().map(|x| x.to_vec()).collect::<Vec<_>>();
                let at = poses.get(id).map(|p| p.at).unwrap_or([0.0; 3]);
                InstanceDof { id: *id, name: name_of(world, *id), free: rank(&rows(&h), 1e-9), motions: describe(&h, at, 1e-9), mobility: rank(&rows(&r), 1e-9) }
            })
            .collect();
        let free = null_held.len();
        let mobility = null_rel.len();
        let summary = if !solved {
            format!(
                "the joints cannot all hold{}",
                if conflicting.is_empty() { String::new() } else { format!(": {} conflict (any one removed, the rest hold)", conflicting.iter().map(|j| j.name.as_str()).collect::<Vec<_>>().join(", ")) }
            )
        } else {
            let mut s = if free == 0 { "fully defined".to_string() } else { format!("{free} freedom{} left undetermined", if free == 1 { "" } else { "s" }) };
            s.push_str(&format!("; moves with {mobility} degree{} of freedom", if mobility == 1 { "" } else { "s" }));
            if !following.is_empty() {
                let each: Vec<String> = following.iter().map(|f| format!("{} with {}", f.name, f.drivers.join(", "))).collect();
                s.push_str(&format!(" ({} follow{} other joints)", each.join("; "), if following.len() == 1 { "s" } else { "" }));
            }
            if redundant > 0 {
                s.push_str(&format!("; {redundant} redundant condition{} ({})", if redundant == 1 { "" } else { "s" }, redundant_joints.iter().map(|j| j.name.as_str()).collect::<Vec<_>>().join(", ")));
            }
            s
        };
        groups.push(GroupDof {
            joints: links.iter().map(jref).collect(),
            grounded: g.anchor.is_none(),
            anchor: g.anchor,
            solved,
            error,
            free,
            mobility,
            instances,
            redundant,
            redundant_joints,
            conflicting,
            following,
            summary,
        });
    }
    let mut unjoined: Vec<JointRef> = world
        .resource::<IdIndex>()
        .iter()
        .filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == scope && !joined.contains(&id) && crate::patterns::pattern_of(world, id).is_none()).map(|i| JointRef { id, name: i.name.clone(), kind: "free".into() }))
        .collect();
    unjoined.sort_by_key(|j| j.id);
    let free = groups.iter().map(|g| g.free).sum::<usize>();
    let mobility = groups.iter().map(|g| g.mobility).sum::<usize>();
    let bad = groups.iter().filter(|g| !g.solved).count();
    let summary = format!(
        "{} group{} of joined instances: {free} freedom{} undetermined, mobility {mobility}{}{}",
        groups.len(),
        if groups.len() == 1 { "" } else { "s" },
        if free == 1 { "" } else { "s" },
        if bad > 0 { format!("; {bad} cannot all hold") } else { String::new() },
        if unjoined.is_empty() { String::new() } else { format!("; {} instance{} joined to nothing (placed by hand)", unjoined.len(), if unjoined.len() == 1 { "" } else { "s" }) }
    );
    Ok(DofReport { assembly: scope, groups, unjoined, free, mobility, summary })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_space_and_words() {
        // x + y = 0 in three unknowns: a plane of solutions.
        let ns = null_space(&[vec![1.0, 1.0, 0.0]], 3, 1e-12);
        assert_eq!(ns.len(), 2);
        // A turn about Z through (10, 0, 0) of a part whose origin is at the origin: v = −ω × r.
        let w = [0.0, 0.0, 1.0];
        let v = tf::scale(tf::cross(w, [10.0, 0.0, 0.0]), -1.0);
        let d = describe(&[[v[0], v[1], v[2], w[0], w[1], w[2]]], [0.0; 3], 1e-9);
        assert_eq!(d, vec!["turns about (0, 0, 1) through (10, 0, 0)".to_string()]);
        let d = describe(&[[0.0, 2.0, 0.0, 0.0, 0.0, 0.0]], [0.0; 3], 1e-9);
        assert_eq!(d, vec!["slides along (0, 1, 0)".to_string()]);
    }
}
