//! Joints between instances: how separately built components go together.
//!
//! A joint names a connector on each of two instances in the same assembly
//! (or at the top of the project), or one connector and the ground. It puts
//! the second connector's frame on the first's — axes opposed by default, so
//! a servo's shaft connector meets the horn's bore connector face to face —
//! then turns or slides it by the joint's value:
//!
//!   W_b = W_a · Tz(offset) · M(values) · Flip
//!
//! where W is a connector's frame where its instance is now, M is the joint's
//! motion (a turn about the axis for a hinge, a slide along it for a slider,
//! both for a cylinder, any turn for a ball, nothing for a fixed joint) and
//! Flip turns the frame half round its x (left out for `aligned`).
//!
//! A joint is also a parameter set named for it, whose parameters are its
//! values (`angle`, `distance`, `rx`/`ry`/`rz`), so people scrub them, give
//! them limits (min/max) and link them by formula like any parameter.
//!
//! Joints that touch make a group. A group that reaches the ground hangs from
//! it; one that does not hangs from its *anchor*, the first instance in it
//! that no joint places (moving the anchor moves the group). The others are
//! *held*: they go where the joints put them and cannot be moved by hand.
//! A tree of joints is placed directly, joint by joint; a closed loop (a
//! four-bar, a part bolted at two places) is solved by damped Gauss–Newton
//! for poses where every joint holds, the loop's `free` joints taking
//! whatever value the others leave them.

use crate::geometry::{Elem, Geometry};
use crate::mates::Mate;
use crate::transform::{self as tf, I3, M3, Transform, V3};
use crate::{AssemblyDef, Instance, bad};
use de_core::{
    Command, CommandError, DependsOn, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_shape::{Param, ShapeProgram, ShapeSolid, params::ParameterSet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

// ---- Design data -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    /// Held together, no motion.
    Fixed,
    /// Turns about the connector axis (value `angle`, degrees).
    Hinge,
    /// Slides along it without turning (value `distance`, mm).
    Slider,
    /// Turns about and slides along it (`angle`, `distance`).
    Cylinder,
    /// Turns any way about the connector point (`rx`, `ry`, `rz`: degrees
    /// about X, then Y, then Z of the first connector's frame).
    Ball,
    // Mates: constraints between geometry (faces, edges, axes, points or
    // connectors), each taking away only the freedoms it names.
    /// Points, lines and planes made to coincide (two faces meeting, a point on a face, an axis on a line).
    Coincident,
    /// Two axes made one (a shaft in a bore), or two centres.
    Concentric,
    /// Two directions (axes, face normals) parallel.
    Parallel,
    /// Two directions square.
    Perpendicular,
    /// Two directions at an angle (value `angle`, degrees).
    Angle,
    /// Points, lines, planes a distance apart (value `distance`, mm).
    Distance,
    /// A round face touching a flat or round one (a roller on a rail, a shaft in a V).
    Tangent,
    /// Held exactly where it was against the other end when locked.
    Lock,
}

impl JointKind {
    /// The joint's values: key, label, unit.
    pub fn values(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            JointKind::Fixed => &[],
            JointKind::Hinge => &[("angle", "Angle", "deg")],
            JointKind::Slider => &[("distance", "Distance", "mm")],
            JointKind::Cylinder => &[("angle", "Angle", "deg"), ("distance", "Distance", "mm")],
            JointKind::Ball => &[("rx", "Turn about X", "deg"), ("ry", "Turn about Y", "deg"), ("rz", "Turn about Z", "deg")],
            JointKind::Angle => &[("angle", "Angle", "deg")],
            JointKind::Distance => &[("distance", "Distance", "mm")],
            _ => &[],
        }
    }

    /// The mate it is, for a joint that constrains geometry.
    pub fn mate(self) -> Option<Mate> {
        Some(match self {
            JointKind::Coincident => Mate::Coincident,
            JointKind::Concentric => Mate::Concentric,
            JointKind::Parallel => Mate::Parallel,
            JointKind::Perpendicular => Mate::Perpendicular,
            JointKind::Angle => Mate::Angle,
            JointKind::Distance => Mate::Distance,
            JointKind::Tangent => Mate::Tangent,
            _ => return None,
        })
    }

    /// A joint that places its second end from its first (fixed, hinge,
    /// slider, cylinder, ball); mates and locks only constrain.
    pub fn places(self) -> bool {
        matches!(self, JointKind::Fixed | JointKind::Hinge | JointKind::Slider | JointKind::Cylinder | JointKind::Ball)
    }

    /// No motion left between the two ends (fixed and lock).
    pub fn rigid(self) -> bool {
        matches!(self, JointKind::Fixed | JointKind::Lock)
    }

    pub fn word(self) -> &'static str {
        match self {
            JointKind::Fixed => "fixed",
            JointKind::Hinge => "hinge",
            JointKind::Slider => "slider",
            JointKind::Cylinder => "cylinder",
            JointKind::Ball => "ball",
            JointKind::Coincident => "coincident",
            JointKind::Concentric => "concentric",
            JointKind::Parallel => "parallel",
            JointKind::Perpendicular => "perpendicular",
            JointKind::Angle => "angle",
            JointKind::Distance => "distance",
            JointKind::Tangent => "tangent",
            JointKind::Lock => "lock",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            JointKind::Fixed => "Fixed",
            JointKind::Hinge => "Hinge",
            JointKind::Slider => "Slider",
            JointKind::Cylinder => "Cylinder",
            JointKind::Ball => "Ball",
            JointKind::Coincident => "Coincident",
            JointKind::Concentric => "Concentric",
            JointKind::Parallel => "Parallel",
            JointKind::Perpendicular => "Perpendicular",
            JointKind::Angle => "Angle",
            JointKind::Distance => "Distance",
            JointKind::Tangent => "Tangent",
            JointKind::Lock => "Lock",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GroundWord {
    Ground,
}

fn up() -> V3 {
    [0.0, 0.0, 1.0]
}

/// One end of a joint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum End {
    /// A connector of an instance in the joint's assembly.
    Connector { instance: StableId, connector: String },
    /// Geometry picked on an instance: a face, an edge, a point, a line, a
    /// plane… (see [`Geometry`]), in the instance's own coordinates.
    Geometry { instance: StableId, geometry: Geometry },
    /// "ground": the assembly's own frame (origin, Z up).
    Ground(GroundWord),
    /// A fixed frame in the assembly: `at`, its axis and x (default Z and the
    /// direction nearest +X).
    Frame {
        at: V3,
        #[serde(default = "up")]
        axis: V3,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<V3>,
    },
}

impl End {
    pub fn instance(&self) -> Option<StableId> {
        match self {
            End::Connector { instance, .. } | End::Geometry { instance, .. } => Some(*instance),
            _ => None,
        }
    }
}

/// A joint (on an entity that is also the parameter set holding its values).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Joint {
    pub kind: JointKind,
    /// The assembly whose instances it joins; none at the top of the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
    pub a: End,
    pub b: End,
    /// Gap between the two connector frames along the first one's axis (mm).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f64,
    /// The two axes point the same way (default: opposed, faces meeting).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub aligned: bool,
    /// In a closed loop: its value follows from the others instead of being held.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub free: bool,
    /// A lock: where the second end was against the first when locked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
}

/// A placement kept by a lock: position (mm) and rotation (degrees about X, then Y, then Z).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hold {
    pub at: V3,
    pub rotation: V3,
}

impl Hold {
    pub fn transform(&self) -> Transform {
        Transform::from_euler(self.at, self.rotation)
    }
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl Persist for Joint {
    const KIND: &'static str = "assembly.joint";
}

/// Derived, on every instance: where it is in its parent now. Its own
/// position and rotation when nothing holds it; where its joints put it when
/// they do.
#[derive(Component, Clone, Debug, PartialEq, Serialize)]
pub struct Placed {
    pub at: V3,
    /// Degrees about X, then Y, then Z.
    pub rotation: V3,
    /// The same rotation as a quaternion [x, y, z, w].
    pub quaternion: [f64; 4],
    /// The joint that places it (none: it is free).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<StableId>,
    /// The instance its group hangs from, when no joint reaches the ground.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<StableId>,
    /// Every joint of its group with the values it has now (a loop's free
    /// joints take theirs from the solve).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<StableId, Vec<f64>>,
    #[serde(skip)]
    rot: M3,
}

impl Placed {
    fn new(t: Transform, by: Option<StableId>, anchor: Option<StableId>, values: BTreeMap<StableId, Vec<f64>>) -> Placed {
        Placed { at: t.at, rotation: t.euler(), quaternion: t.quaternion(), by, anchor, values, rot: t.rot }
    }

    pub fn transform(&self) -> Transform {
        Transform { at: self.at, rot: self.rot }
    }

    /// Placed by something other than a joint (an array the instance is a copy in).
    pub fn at_transform(t: Transform) -> Placed {
        Placed::new(t, None, None, BTreeMap::new())
    }
}

// ---- Solving -------------------------------------------------------------------

/// A joint end as the solver sees it.
#[derive(Clone, Debug)]
pub enum Side {
    /// A fixed frame in the assembly.
    Ground(Transform),
    /// An instance and its connector's frame in the instance's own coordinates.
    Inst(StableId, Transform),
    /// An instance and geometry on it, in its own coordinates (for a mate).
    Elem(StableId, Elem),
}

impl Side {
    pub fn id(&self) -> Option<StableId> {
        match self {
            Side::Inst(id, _) | Side::Elem(id, _) => Some(*id),
            Side::Ground(_) => None,
        }
    }
}

/// A joint ready to solve.
#[derive(Clone, Debug)]
pub struct Link {
    pub id: StableId,
    pub name: String,
    pub kind: JointKind,
    pub a: Side,
    pub b: Side,
    pub offset: f64,
    pub aligned: bool,
    pub free: bool,
    /// Degrees or mm, in the order of `JointKind::values`.
    pub values: Vec<f64>,
    /// A lock's kept placement of `b`'s frame in `a`'s.
    pub hold: Option<Transform>,
}

/// The joint's motion for these values.
pub fn motion(kind: JointKind, v: &[f64]) -> Transform {
    let g = |i: usize| v.get(i).copied().unwrap_or(0.0);
    match kind {
        JointKind::Fixed => Transform::IDENTITY,
        JointKind::Hinge => Transform::rotation(tf::axis_rotation(2, g(0))),
        JointKind::Slider => Transform::translation([0.0, 0.0, g(0)]),
        JointKind::Cylinder => Transform { at: [0.0, 0.0, g(1)], rot: tf::axis_rotation(2, g(0)) },
        JointKind::Ball => Transform::from_euler([0.0; 3], [g(0), g(1), g(2)]),
        _ => Transform::IDENTITY,
    }
}

const FLIP: M3 = [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]];

impl Link {
    /// The transform from the first connector's frame to the second's.
    pub fn relation(&self, values: &[f64]) -> Transform {
        let flip = if self.aligned { Transform::IDENTITY } else { Transform::rotation(FLIP) };
        Transform::translation([0.0, 0.0, self.offset]).then(&motion(self.kind, values)).then(&flip)
    }

    fn world(side: &Side, poses: &HashMap<StableId, Transform>) -> Option<Transform> {
        match side {
            Side::Ground(g) => Some(*g),
            Side::Inst(id, f) => poses.get(id).map(|p| p.then(f)),
            Side::Elem(id, _) => poses.get(id).copied(),
        }
    }

    /// A mate's two ends as geometry where they are now.
    fn elems(&self, poses: &HashMap<StableId, Transform>) -> Option<(Elem, Elem)> {
        let mate = self.kind.mate()?;
        let concrete = |s: &Side| match s {
            Side::Elem(id, e) => poses.get(id).map(|p| e.placed(p)),
            _ => None,
        };
        let (ca, cb) = (concrete(&self.a), concrete(&self.b));
        let adapt = |s: &Side, c: Option<Elem>, other: Option<Elem>| -> Option<Elem> {
            match s {
                Side::Elem(..) => c,
                _ => Some(crate::mates::adapt(mate, &Self::world(s, poses)?, other.as_ref())),
            }
        };
        let a = adapt(&self.a, ca, cb)?;
        let b = adapt(&self.b, cb, Some(a))?;
        Some((a, b))
    }

    /// Whether the mate can join its two ends' kinds of geometry.
    pub fn check_mate(&self, poses: &HashMap<StableId, Transform>) -> Result<(), String> {
        let (Some(mate), Some((a, b))) = (self.kind.mate(), self.elems(poses)) else { return Ok(()) };
        crate::mates::check(mate, &a, &b).map_err(|m| format!("{}: {m}", self.name))
    }

    /// The second connector's frame relative to the first's, with the offset
    /// and flip taken out: what the motion should be.
    fn error_frame(&self, poses: &HashMap<StableId, Transform>) -> Option<Transform> {
        let wa = Self::world(&self.a, poses)?.then(&Transform::translation([0.0, 0.0, self.offset]));
        let wb = Self::world(&self.b, poses)?;
        let flip = if self.aligned { Transform::IDENTITY } else { Transform::rotation(FLIP) };
        Some(wa.inverse().then(&wb).then(&flip.inverse()))
    }

    /// The values the joint has at these poses.
    pub fn measure(&self, poses: &HashMap<StableId, Transform>) -> Option<Vec<f64>> {
        let e = self.error_frame(poses)?;
        let angle = e.rot[1][0].atan2(e.rot[0][0]).to_degrees();
        let near = |a: f64, v: f64| v + wrap_deg(a - v);
        if let Some(mate) = self.kind.mate() {
            let (a, b) = self.elems(poses)?;
            return Some(crate::mates::measure(mate, &a, &b).into_iter().collect());
        }
        Some(match self.kind {
            JointKind::Fixed | JointKind::Lock => vec![],
            JointKind::Hinge => vec![near(angle, self.values[0])],
            JointKind::Slider => vec![e.at[2]],
            JointKind::Cylinder => vec![near(angle, self.values[0]), e.at[2]],
            JointKind::Ball => e.euler().to_vec(),
            _ => vec![],
        })
    }

    fn residuals(&self, poses: &HashMap<StableId, Transform>, size: f64, out: &mut Vec<f64>) {
        self.residuals_with(poses, size, false, out)
    }

    /// How far the joint is from holding (lengths in mm, turns scaled by
    /// `size`). `release` leaves its values out (what the joint lets move).
    pub(crate) fn residuals_with(&self, poses: &HashMap<StableId, Transform>, size: f64, release: bool, out: &mut Vec<f64>) {
        if let Some(mate) = self.kind.mate() {
            let Some((a, b)) = self.elems(poses) else { return };
            let value = (!self.free && !release).then(|| self.values.first().copied().unwrap_or(0.0));
            let _ = crate::mates::residuals(mate, &a, &b, value, self.aligned, self.offset, size, out);
            return;
        }
        if self.kind == JointKind::Lock {
            let (Some(wa), Some(wb)) = (Self::world(&self.a, poses), Self::world(&self.b, poses)) else { return };
            let e = wa.then(&self.hold.unwrap_or(Transform::IDENTITY)).inverse().then(&wb);
            out.extend(e.at);
            out.extend(tf::scale(tf::log(&e.rot), size));
            return;
        }
        let Some(e) = self.error_frame(poses) else { return };
        let (p, r) = (e.at, e.rot);
        let z = [r[0][2], r[1][2], r[2][2]];
        let held = !self.free && !release;
        let angle = || wrap_deg(r[1][0].atan2(r[0][0]).to_degrees() - self.values[0]).to_radians() * size;
        match self.kind {
            JointKind::Fixed => {
                out.extend(p);
                out.extend(tf::scale(tf::log(&r), size));
            }
            JointKind::Hinge => {
                out.extend(p);
                out.extend([z[0] * size, z[1] * size]);
                if held {
                    out.push(angle());
                }
            }
            JointKind::Slider => {
                out.extend([p[0], p[1]]);
                out.extend(tf::scale(tf::log(&r), size));
                if held {
                    out.push(p[2] - self.values[0]);
                }
            }
            JointKind::Cylinder => {
                out.extend([p[0], p[1]]);
                out.extend([z[0] * size, z[1] * size]);
                if held {
                    out.push(angle());
                    out.push(p[2] - self.values[1]);
                }
            }
            JointKind::Ball => {
                out.extend(p);
                if held {
                    let m = motion(JointKind::Ball, &self.values);
                    out.extend(tf::scale(tf::log(&tf::matmul(&r, &tf::transpose(&m.rot))), size));
                }
            }
            _ => {}
        }
    }
}

fn wrap_deg(a: f64) -> f64 {
    a - 360.0 * ((a + 180.0) / 360.0).floor()
}

/// The reach turns are weighed at (mm): how far the joints' frames are from
/// their instances' origins.
pub(crate) fn size_of(links: &[Link]) -> f64 {
    links
        .iter()
        .flat_map(|l| [&l.a, &l.b])
        .map(|s| match s {
            Side::Inst(_, f) => tf::norm(f.at),
            Side::Ground(g) => tf::norm(g.at),
            Side::Elem(_, e) => match e {
                Elem::Point { p, .. } | Elem::Line { p, .. } | Elem::Plane { p, .. } => tf::norm(*p),
                Elem::Frame => 0.0,
            },
        })
        .fold(10.0, f64::max)
        .min(1e5)
}

/// Every instance's placement, and every joint's values, once the joints hold.
#[derive(Clone, Debug, Default)]
pub struct Solution {
    pub poses: HashMap<StableId, Transform>,
    /// The link (joint) that placed each instance.
    pub by: HashMap<StableId, StableId>,
    pub values: HashMap<StableId, Vec<f64>>,
}

/// Places every instance the links reach: `fixed` are placed already (an
/// anchor); `guess` is where held instances were, to start a loop's solve
/// near (so it stays on the same branch).
pub fn solve(links: &[Link], fixed: &HashMap<StableId, Transform>, guess: &HashMap<StableId, Transform>) -> Result<Solution, String> {
    let mut poses: HashMap<StableId, Transform> = fixed.clone();
    let mut by = HashMap::new();
    let mut tree = vec![false; links.len()];
    // Joint by joint outward from what is placed.
    loop {
        let mut progress = false;
        for (i, l) in links.iter().enumerate() {
            if tree[i] || !l.kind.places() {
                continue;
            }
            let (wa, wb) = (Link::world(&l.a, &poses), Link::world(&l.b, &poses));
            let placed = match (wa, wb, &l.a, &l.b) {
                (Some(wa), None, _, Side::Inst(id, f)) => Some((*id, wa.then(&l.relation(&l.values)).then(&f.inverse()))),
                (None, Some(wb), Side::Inst(id, f), _) => Some((*id, wb.then(&l.relation(&l.values).inverse()).then(&f.inverse()))),
                _ => None,
            };
            if let Some((id, p)) = placed {
                poses.insert(id, p);
                by.insert(id, l.id);
                tree[i] = true;
                progress = true;
            }
        }
        if progress {
            continue;
        }
        // What only mates (or a lock) hold starts where it was, and the
        // solve moves it as little as the mates need.
        let constrained = links.iter().filter(|l| !l.kind.places()).flat_map(|l| [(&l.a, l.id), (&l.b, l.id)]);
        let Some((id, by_link)) = constrained.filter_map(|(s, j)| s.id().map(|i| (i, j))).find(|(i, _)| !poses.contains_key(i)) else { break };
        match guess.get(&id) {
            Some(t) => {
                poses.insert(id, *t);
                by.insert(id, by_link);
            }
            None => break,
        }
    }
    for l in links {
        for s in [&l.a, &l.b] {
            if let Some(id) = s.id()
                && !poses.contains_key(&id)
            {
                return Err(format!("nothing places the instance {id} that “{}” joins: join it to the ground or to a placed instance", l.name));
            }
        }
    }
    let size = size_of(links);
    // Joints left over close loops: fine if they already hold.
    let closing: Vec<&Link> = links.iter().zip(&tree).filter(|(_, t)| !**t).map(|(l, _)| l).collect();
    let worst = |p: &HashMap<StableId, Transform>, ls: &[&Link]| {
        let mut r = vec![];
        for l in ls {
            l.residuals(p, size, &mut r);
        }
        r.iter().fold(0.0f64, |m, v| m.max(v.abs()))
    };
    let tol = 1e-7 * size;
    if !closing.is_empty() && worst(&poses, &closing) > tol {
        let moving: Vec<StableId> = {
            let mut ids: Vec<StableId> = poses.keys().filter(|id| !fixed.contains_key(id)).copied().collect();
            ids.sort();
            ids
        };
        let all: Vec<&Link> = links.iter().collect();
        let from_guess = moving.iter().all(|id| guess.contains_key(id)).then(|| {
            let mut p = poses.clone();
            for id in &moving {
                p.insert(*id, guess[id]);
            }
            p
        });
        let mut solved = None;
        let mut best = f64::INFINITY;
        for start in from_guess.into_iter().chain([poses.clone()]) {
            match settle(&all, &moving, start, size) {
                Ok(p) => {
                    solved = Some(p);
                    break;
                }
                Err(e) => best = best.min(e),
            }
        }
        match solved {
            Some(p) => poses = p,
            None => {
                let names: Vec<&str> = closing.iter().map(|l| l.name.as_str()).collect();
                return Err(format!(
                    "the joints cannot all hold: the loop closed by {} misses by {:.3} mm (check the connectors and values, or make one of its joints free)",
                    names.join(", "),
                    best
                ));
            }
        }
    }
    let values = links.iter().filter_map(|l| Some((l.id, l.measure(&poses)?))).collect();
    Ok(Solution { poses, by, values })
}

/// Damped Gauss–Newton over the moving instances' poses until every link
/// holds; Err with the remaining miss.
fn settle(links: &[&Link], moving: &[StableId], start: HashMap<StableId, Transform>, size: f64) -> Result<HashMap<StableId, Transform>, f64> {
    let n = 6 * moving.len();
    let min_norm = links.iter().any(|l| !l.kind.places());
    // How freely each instance moves in those steps: the farther it is (by
    // joints) from what is fixed, the more it yields — the part mated last
    // moves to its mate, not the part it is mated to.
    let mobility: Vec<f64> = {
        let mut depth: HashMap<StableId, usize> = start.keys().filter(|id| !moving.contains(id)).map(|id| (*id, 0)).collect();
        for l in links {
            if let (None, Some(b)) | (Some(b), None) = (l.a.id(), l.b.id()) {
                depth.entry(b).or_insert(1);
            }
        }
        for _ in 0..links.len() {
            let mut changed = false;
            for l in links {
                if let (Some(a), Some(b)) = (l.a.id(), l.b.id()) {
                    for (x, y) in [(a, b), (b, a)] {
                        if let Some(dx) = depth.get(&x).copied()
                            && depth.get(&y).is_none_or(|dy| *dy > dx + 1)
                        {
                            depth.insert(y, dx + 1);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        moving.iter().map(|id| 8f64.powi(depth.get(id).copied().unwrap_or(1).saturating_sub(1).min(6) as i32)).collect()
    };
    let res = |p: &HashMap<StableId, Transform>| {
        let mut r = vec![];
        for l in links {
            l.residuals(p, size, &mut r);
        }
        r
    };
    let step = |p: &HashMap<StableId, Transform>, d: &[f64]| {
        let mut q = p.clone();
        for (i, id) in moving.iter().enumerate() {
            let t = p[id];
            let w = [d[6 * i + 3], d[6 * i + 4], d[6 * i + 5]];
            let th = tf::norm(w);
            let r = if th < 1e-15 { I3 } else { tf::rotation_about(tf::scale(w, 1.0 / th), th) };
            q.insert(*id, Transform { at: tf::add(t.at, [d[6 * i], d[6 * i + 1], d[6 * i + 2]]), rot: tf::matmul(&r, &t.rot) });
        }
        q
    };
    let worst = |r: &[f64]| r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let tol = 1e-9 * size;
    let mut p = start;
    let mut r = res(&p);
    let mut err = worst(&r);
    let mut lambda = 1e-6;
    for _ in 0..100 {
        if err < tol {
            return Ok(p);
        }
        let mut cols = Vec::with_capacity(n);
        for k in 0..n {
            let h = if k % 6 < 3 { 1e-6 * size } else { 1e-6 };
            let mut d = vec![0.0; n];
            d[k] = h;
            let r1 = res(&step(&p, &d));
            d[k] = -h;
            let r2 = res(&step(&p, &d));
            cols.push(r1.iter().zip(&r2).map(|(a, b)| (a - b) / (2.0 * h)).collect::<Vec<f64>>());
        }
        let mut a = vec![vec![0.0; n]; n];
        let mut g = vec![0.0; n];
        for i in 0..n {
            g[i] = -cols[i].iter().zip(&r).map(|(x, y)| x * y).sum::<f64>();
            for j in i..n {
                let v: f64 = cols[i].iter().zip(&cols[j]).map(|(x, y)| x * y).sum();
                a[i][j] = v;
                a[j][i] = v;
            }
        }
        let mut improved = false;
        // What mates hold is under-determined: each step is then the least
        // motion that closes the miss (turns weighed by the reach), d = −W⁻¹Jᵀ(JW⁻¹Jᵀ + λI)⁻¹r,
        // so free directions stay where they were instead of drifting.
        let dual = |r: &[f64], lambda: f64| -> Option<Vec<f64>> {
            let m = r.len();
            let w = |k: usize| mobility[k / 6] * if k % 6 < 3 { 1.0 } else { 1.0 / (size * size) };
            let mut jj = vec![vec![0.0; m]; m];
            for k in 0..n {
                let c = &cols[k];
                let wk = w(k);
                for i in 0..m {
                    if c[i] == 0.0 {
                        continue;
                    }
                    for j in i..m {
                        jj[i][j] += c[i] * c[j] * wk;
                    }
                }
            }
            let mean = (0..m).map(|i| jj[i][i]).sum::<f64>() / m.max(1) as f64;
            for i in 0..m {
                for j in 0..i {
                    jj[i][j] = jj[j][i];
                }
                jj[i][i] += lambda * mean.max(1e-300) + 1e-14 * mean;
            }
            let y = de_shape::mechanism::solve_dense(jj, r.to_vec())?;
            Some((0..n).map(|k| -w(k) * cols[k].iter().zip(&y).map(|(a, b)| a * b).sum::<f64>()).collect())
        };
        for _ in 0..14 {
            let d = if min_norm {
                dual(&r, lambda)
            } else {
                let mut m = a.clone();
                for (i, row) in m.iter_mut().enumerate() {
                    row[i] += lambda * (a[i][i] + 1e-12);
                }
                de_shape::mechanism::solve_dense(m, g.clone())
            };
            let Some(d) = d else {
                lambda *= 10.0;
                continue;
            };
            let q = step(&p, &d);
            let rq = res(&q);
            let e = worst(&rq);
            if e < err {
                p = q;
                r = rq;
                err = e;
                lambda = (lambda / 10.0).max(1e-12);
                improved = true;
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    if err < tol * 100.0 { Ok(p) } else { Err(err) }
}

/// Solutions by their inputs: every instance of a group asks for the same one.
fn cached_solve(links: &[Link], fixed: &HashMap<StableId, Transform>, guess: &HashMap<StableId, Transform>) -> Result<Arc<Solution>, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let t = |h: &mut std::collections::hash_map::DefaultHasher, t: &Transform| {
        for v in t.at.iter().chain(t.rot.iter().flatten()) {
            v.to_bits().hash(h);
        }
    };
    for l in links {
        l.id.hash(&mut h);
        l.kind.hash(&mut h);
        for s in [&l.a, &l.b] {
            match s {
                Side::Ground(g) => t(&mut h, g),
                Side::Inst(id, f) => {
                    id.hash(&mut h);
                    t(&mut h, f);
                }
                Side::Elem(id, e) => {
                    id.hash(&mut h);
                    format!("{e:?}").hash(&mut h);
                }
            }
        }
        for v in l.values.iter().chain([&l.offset]) {
            v.to_bits().hash(&mut h);
        }
        (l.aligned, l.free).hash(&mut h);
        if let Some(hold) = &l.hold {
            t(&mut h, hold);
        }
    }
    let sorted = |m: &HashMap<StableId, Transform>, h: &mut std::collections::hash_map::DefaultHasher| {
        let mut keys: Vec<&StableId> = m.keys().collect();
        keys.sort();
        for k in keys {
            k.hash(h);
            t(h, &m[k]);
        }
    };
    sorted(fixed, &mut h);
    sorted(guess, &mut h);
    let key = h.finish();
    static CACHE: OnceLock<Mutex<Vec<(u64, Arc<Solution>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, s)) = cache.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return Ok(Arc::clone(s));
    }
    let s = Arc::new(solve(links, fixed, guess)?);
    let mut c = cache.lock().unwrap();
    c.push((key, Arc::clone(&s)));
    if c.len() > 64 {
        c.remove(0);
    }
    Ok(s)
}

// ---- Reading the project ---------------------------------------------------------

fn entity(world: &World, id: StableId) -> Option<Entity> {
    world.resource::<IdIndex>().get(id)
}

fn instance_name(world: &World, id: StableId) -> String {
    entity(world, id).and_then(|e| world.get::<Instance>(e)).map(|i| i.name.clone()).unwrap_or_else(|| id.to_string())
}

/// A joint's name (its parameter set's).
pub fn joint_name(world: &World, id: StableId) -> String {
    entity(world, id).and_then(|e| world.get::<ParameterSet>(e)).map(|s| s.name.clone()).unwrap_or_else(|| "joint".into())
}

/// Every connector an instance offers, in the instance's own coordinates
/// (an assembly's are its instances', named “Instance › connector”).
pub fn connectors(world: &World, instance: StableId) -> Result<Vec<(String, Transform)>, String> {
    connectors_at(world, instance, 0)
}

fn connectors_at(world: &World, instance: StableId, depth: usize) -> Result<Vec<(String, Transform)>, String> {
    if depth > 16 {
        return Ok(vec![]);
    }
    let e = entity(world, instance).ok_or_else(|| format!("{instance} does not exist"))?;
    let inst = world.get::<Instance>(e).ok_or_else(|| format!("{instance} is not an instance"))?;
    let def = entity(world, inst.of).ok_or_else(|| format!("{}: its definition is gone", inst.name))?;
    if world.get::<AssemblyDef>(def).is_some() {
        let mut out = vec![];
        for (child, ci) in crate::children(world, inst.of) {
            let Some(ce) = entity(world, child) else { continue };
            let place = crate::placement(world, ce).unwrap_or_else(|| ci.transform());
            for (n, f) in connectors_at(world, child, depth + 1)? {
                out.push((format!("{} › {n}", ci.name), place.then(&f)));
            }
        }
        return Ok(out);
    }
    let program = world.get::<ShapeProgram>(def).ok_or_else(|| format!("{}: not a component", inst.name))?;
    let declared = match &inst.config {
        Some(c) => {
            let p = de_shape::configs::configured_program(world, def, c)?;
            de_shape::evaluated_at(&p, p.display_tolerance())?.declared.clone()
        }
        None => world.get::<ShapeSolid>(def).ok_or_else(|| format!("{} has no valid result", program.name))?.stats.declared.clone(),
    };
    Ok(declared.connectors.iter().map(|c| (c.name.clone(), Transform::frame(c.at, c.x, c.axis))).collect())
}

fn connector(world: &World, instance: StableId, name: &str) -> Result<Transform, String> {
    let all = connectors(world, instance)?;
    if let Some((_, f)) = all.iter().find(|(n, _)| n == name) {
        return Ok(*f);
    }
    let who = instance_name(world, instance);
    if all.is_empty() {
        return Err(format!("{who} has no connectors: its program declares none (connector(name, {{ at, axis }}))"));
    }
    Err(format!("{who} has no connector “{name}” (it has {})", all.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")))
}

fn side(world: &World, end: &End, kind: JointKind) -> Result<Side, String> {
    Ok(match end {
        End::Connector { instance, connector: c } => Side::Inst(*instance, connector(world, *instance, c)?),
        End::Geometry { instance, geometry } => {
            let e = crate::geometry::resolve(world, *instance, geometry).map_err(|m| format!("{} › {m}", instance_name(world, *instance)))?;
            // A hinge, slider… on picked geometry turns about its axis (or normal).
            if kind.places() { Side::Inst(*instance, e.frame()) } else { Side::Elem(*instance, e) }
        }
        End::Ground(_) => Side::Ground(Transform::IDENTITY),
        End::Frame { at, axis, x } => {
            let z = tf::scale(*axis, 1.0 / tf::norm(*axis).max(1e-300));
            let r = x.unwrap_or(if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] });
            let r = tf::sub(r, tf::scale(z, tf::dot(r, z)));
            Side::Ground(Transform::frame(*at, tf::scale(r, 1.0 / tf::norm(r).max(1e-300)), z))
        }
    })
}

/// A joint's values now (formulas worked out), in the order of its kind's keys.
fn values_of(world: &World, joint: &Joint, set: &ParameterSet) -> Result<Vec<f64>, String> {
    let resolved = de_shape::params::resolve(world, &set.params)?;
    Ok(joint.kind.values().iter().map(|(k, _, _)| resolved.get(*k).copied().unwrap_or(0.0)).collect())
}

/// The joint `id` ready to solve, with `values` in place of its own if given.
pub fn link(world: &World, id: StableId, values: Option<&[f64]>) -> Result<Link, String> {
    let e = entity(world, id).ok_or_else(|| format!("joint {id} does not exist"))?;
    let joint = world.get::<Joint>(e).ok_or_else(|| format!("{id} is not a joint"))?;
    let set = world.get::<ParameterSet>(e).ok_or_else(|| format!("{id} has no values"))?;
    let named = |r: Result<Side, String>| r.map_err(|m| format!("{}: {m}", set.name));
    Ok(Link {
        id,
        name: set.name.clone(),
        kind: joint.kind,
        a: named(side(world, &joint.a, joint.kind))?,
        b: named(side(world, &joint.b, joint.kind))?,
        offset: joint.offset,
        aligned: joint.aligned,
        free: joint.free,
        hold: joint.hold.map(|h| h.transform()),
        values: match values {
            Some(v) => v.to_vec(),
            None => values_of(world, joint, set).map_err(|m| format!("{}: {m}", set.name))?,
        },
    })
}

/// Every joint in an assembly (or at the top), oldest first.
pub fn joints_in(world: &World, scope: Option<StableId>) -> Vec<(StableId, Joint)> {
    let mut out: Vec<(StableId, Joint)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Joint>(e).filter(|j| j.assembly == scope).map(|j| (id, j.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// Joints that touch, and how they hang.
#[derive(Clone, Debug)]
pub struct Group {
    pub joints: Vec<StableId>,
    /// Instances in it, in the order the joints name them.
    pub members: Vec<StableId>,
    /// The instance it hangs from when it does not reach the ground.
    pub anchor: Option<StableId>,
}

impl Group {
    pub fn held(&self) -> impl Iterator<Item = StableId> + '_ {
        self.members.iter().copied().filter(|m| Some(*m) != self.anchor)
    }
}

/// Splits joints (oldest first) into groups that touch.
pub fn groups(joints: &[(StableId, Joint)]) -> Vec<Group> {
    let mut out: Vec<(Group, bool, Vec<StableId>)> = vec![]; // group, grounded, instances some joint places as `b`
    for (id, j) in joints {
        let ends: Vec<StableId> = [j.a.instance(), j.b.instance()].into_iter().flatten().collect();
        let grounded = j.a.instance().is_none() || j.b.instance().is_none();
        let touching: Vec<usize> = (0..out.len()).filter(|&g| ends.iter().any(|e| out[g].0.members.contains(e))).collect();
        let mut merged = (Group { joints: vec![], members: vec![], anchor: None }, false, vec![]);
        for g in touching.iter().rev() {
            let (grp, gr, bs) = out.remove(*g);
            merged.0.joints.splice(0..0, grp.joints);
            merged.0.members.splice(0..0, grp.members);
            merged.1 |= gr;
            merged.2.extend(bs);
        }
        merged.0.joints.push(*id);
        for e in ends {
            if !merged.0.members.contains(&e) {
                merged.0.members.push(e);
            }
        }
        merged.1 |= grounded;
        if let Some(b) = j.b.instance() {
            merged.2.push(b);
        }
        out.push(merged);
    }
    out.into_iter()
        .map(|(mut g, grounded, placed)| {
            g.joints.sort();
            if !grounded {
                g.anchor = g.members.iter().find(|m| !placed.contains(m)).or(g.members.first()).copied();
            }
            g
        })
        .collect()
}

/// The group holding an instance (it is in it and not its anchor), if any.
fn group_holding(world: &World, instance: StableId) -> Option<Group> {
    let scope = entity(world, instance).and_then(|e| world.get::<Instance>(e)).map(|i| i.parent)?;
    groups(&joints_in(world, scope)).into_iter().find(|g| g.members.contains(&instance) && g.anchor != Some(instance))
}

/// The joints holding an instance (its group's), if any hold it.
pub fn holding(world: &World, instance: StableId) -> Vec<StableId> {
    group_holding(world, instance).map(|g| g.joints).unwrap_or_default()
}

/// Derived: an instance's placement (see [`Placed`]). Its group is read from
/// the joints in its assembly; the dependencies (on the assembly's joint hub,
/// see [`rewire`]) only say when to look again.
pub(crate) fn placed(world: &World, e: Entity) -> Option<Result<Placed, String>> {
    let inst = world.get::<Instance>(e)?;
    let id = *world.get::<StableId>(e)?;
    // A pattern's copy goes where its pattern puts it.
    if let Some(r) = crate::patterns::member_placement(world, e) {
        return Some(r.map(|t| Placed::new(t, None, None, BTreeMap::new())));
    }
    let Some(g) = group_holding(world, id) else {
        return Some(Ok(Placed::new(inst.transform(), None, None, BTreeMap::new())));
    };
    Some(place_in_group(world, inst.parent, id, &g.joints, g.anchor))
}

fn place_in_group(world: &World, scope: Option<StableId>, id: StableId, joint_ids: &[StableId], anchor: Option<StableId>) -> Result<Placed, String> {
    // Couplings set followers' values from their drivers' first.
    let coupled = crate::couplings::coupled_values(world, scope, &HashMap::new())?;
    let links = joint_ids.iter().map(|j| link(world, *j, coupled.as_ref().and_then(|c| c.get(j)).map(|v| v.as_slice()))).collect::<Result<Vec<_>, _>>()?;
    let stored = |m: StableId| entity(world, m).and_then(|x| world.get::<Instance>(x)).map(Instance::transform);
    let mut fixed = HashMap::new();
    if let Some(a) = anchor {
        fixed.insert(a, stored(a).ok_or("its anchor is gone")?);
    }
    let mut guess = HashMap::new();
    for l in &links {
        for m in [l.a.id(), l.b.id()].into_iter().flatten() {
            if !fixed.contains_key(&m)
                && let Some(t) = stored(m)
            {
                guess.insert(m, t);
            }
        }
    }
    let sol = cached_solve(&links, &fixed, &guess)?;
    let pose = *sol.poses.get(&id).ok_or("no joint reaches it")?;
    let values = sol.values.iter().map(|(k, v)| (*k, v.iter().map(|x| (x * 1e9).round() / 1e9).collect())).collect();
    Ok(Placed::new(pose, sol.by.get(&id).copied(), anchor, values))
}

/// Every instance of an assembly (or the top) placed with these joint values
/// in place of their own (joint id → values): for sweeps. Free instances
/// keep their own placement; `guess` starts loops near a previous pose.
pub fn solve_scope(world: &World, scope: Option<StableId>, values: &HashMap<StableId, Vec<f64>>, guess: &HashMap<StableId, Transform>) -> Result<Solution, String> {
    let joints = joints_in(world, scope);
    let mut out = Solution::default();
    // Couplings: every joint's value worked out together (a driven follower turns its driver back).
    let coupled = crate::couplings::coupled_values(world, scope, values)?;
    for g in groups(&joints) {
        // Joints linked by formula to a joint given a value here follow it (an idler finger geared to the driven one).
        let given = |id: StableId, key: &str| -> Option<f64> {
            let v = values.get(&id)?;
            let kind = entity(world, id).and_then(|e| world.get::<Joint>(e))?.kind;
            kind.values().iter().position(|(k, _, _)| *k == key).and_then(|i| v.get(i).copied())
        };
        let links = g
            .joints
            .iter()
            .map(|j| match coupled.as_ref().and_then(|c| c.get(j)).or(values.get(j)) {
                Some(v) => link(world, *j, Some(v.as_slice())),
                None if values.is_empty() => link(world, *j, None),
                None => {
                    let e = entity(world, *j).ok_or("a joint is gone")?;
                    let (joint, set) = (world.get::<Joint>(e).ok_or("not a joint")?, world.get::<ParameterSet>(e).ok_or("no values")?);
                    let resolved = de_shape::params::resolve_with(world, &set.params, &given).map_err(|m| format!("{}: {m}", set.name))?;
                    let v: Vec<f64> = joint.kind.values().iter().map(|(k, _, _)| resolved.get(*k).copied().unwrap_or(0.0)).collect();
                    link(world, *j, Some(&v))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let stored = |m: StableId| entity(world, m).and_then(|x| world.get::<Instance>(x)).map(Instance::transform);
        let mut fixed = HashMap::new();
        if let Some(a) = g.anchor {
            fixed.insert(a, stored(a).ok_or("an anchor is gone")?);
        }
        let mut start = HashMap::new();
        for m in g.held() {
            if let Some(t) = guess.get(&m).copied().or_else(|| entity(world, m).and_then(|e| crate::placement(world, e))) {
                start.insert(m, t);
            }
        }
        let s = solve(&links, &fixed, &start)?;
        out.poses.extend(s.poses);
        out.by.extend(s.by);
        out.values.extend(s.values);
    }
    Ok(out)
}

// ---- Keeping dependencies in step --------------------------------------------------

fn scope_instances(world: &World, scope: Option<StableId>) -> Vec<(StableId, Instance)> {
    let mut out: Vec<(StableId, Instance)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == scope).map(|i| (id, i.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// One per assembly (and one for the top of the project) that has joints:
/// it depends on everything that places the assembly's held instances — its
/// joints, the groups' anchors and the definitions of the instances joined
/// (their connectors) — and every held instance depends on it alone. Adding
/// a joint changes the hub's dependencies, not every instance's, so an
/// assembly of hundreds of joined parts stays cheap to change and to store.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JointHub {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
}

impl Persist for JointHub {
    const KIND: &'static str = "assembly.joint_hub";
}

fn hub_of(world: &World, scope: Option<StableId>) -> Option<StableId> {
    world.resource::<IdIndex>().iter().filter(|(_, e)| world.get::<JointHub>(*e).is_some_and(|h| h.assembly == scope)).map(|(id, _)| id).min()
}

/// Keeps the scope's joint hub (see [`JointHub`]) and its held instances'
/// dependencies in step with the joints.
pub(crate) fn rewire(tx: &mut Tx, scope: Option<StableId>) -> Result<(), CommandError> {
    let joints = joints_in(tx.world(), scope);
    let gs = groups(&joints);
    let instances = scope_instances(tx.world(), scope);
    let of: HashMap<StableId, StableId> = instances.iter().map(|(id, i)| (*id, i.of)).collect();
    let hub = match hub_of(tx.world(), scope) {
        Some(h) => Some(h),
        None if !joints.is_empty() => {
            let h = tx.create();
            tx.set(h, JointHub { assembly: scope })?;
            Some(h)
        }
        None => None,
    };
    if let Some(h) = hub {
        let mut want: Vec<StableId> = joints.iter().map(|(j, _)| *j).collect();
        want.extend(crate::couplings::couplings_in(tx.world(), scope).into_iter().map(|(c, _)| c));
        for g in &gs {
            want.extend(g.anchor);
            for m in &g.members {
                if let Some(d) = of.get(m)
                    && !want.contains(d)
                {
                    want.push(*d);
                }
            }
        }
        let current = tx.get::<DependsOn>(h).map(|d| d.0.clone()).unwrap_or_default();
        for d in current.iter().filter(|d| !want.contains(d)) {
            tx.remove_dependency(h, *d)?;
        }
        for d in want.iter().filter(|d| !current.contains(d)) {
            tx.add_dependency(h, *d)?;
        }
    }
    for (id, inst) in &instances {
        let mut want: Vec<StableId> = vec![];
        if gs.iter().any(|g| g.members.contains(id) && g.anchor != Some(*id)) {
            want.extend(hub);
        }
        let world = tx.world();
        let current = tx.get::<DependsOn>(*id).map(|d| d.0.clone()).unwrap_or_default();
        let ours = |d: &StableId| {
            entity(world, *d).is_some_and(|e| world.get::<Joint>(e).is_some() || world.get::<JointHub>(e).is_some() || world.get::<Instance>(e).is_some() || ((world.get::<ShapeProgram>(e).is_some() || world.get::<AssemblyDef>(e).is_some()) && *d != inst.of))
                || entity(world, *d).is_none()
        };
        let stale: Vec<StableId> = current.iter().copied().filter(|d| ours(d) && !want.contains(d)).collect();
        for d in stale {
            tx.remove_dependency(*id, d)?;
        }
        for d in want.into_iter().filter(|d| !current.contains(d)) {
            tx.add_dependency(*id, d)?;
        }
    }
    Ok(())
}

/// Writes where held instances are now into their own position and
/// rotation, so a joint deleted or changed leaves them where they were.
fn freeze(tx: &mut Tx, scope: Option<StableId>) -> Result<(), CommandError> {
    for (id, mut inst) in scope_instances(tx.world(), scope) {
        let Some(p) = tx.get::<Placed>(id).filter(|p| p.by.is_some()).cloned() else { continue };
        let at = p.at.map(|v| (v * 1e9).round() / 1e9);
        if inst.at == at && inst.rotation == p.rotation {
            continue; // unchanged: nothing to journal
        }
        inst.at = at;
        inst.rotation = p.rotation;
        tx.set(id, inst)?;
    }
    Ok(())
}

/// A held instance's group noticed a change in one of its instances that no
/// dependency carries (its configuration): its other instances re-solve.
pub(crate) fn touch(tx: &mut Tx, instance: StableId, scope: Option<StableId>) -> Result<(), CommandError> {
    // With a hub: re-mark it (its dependents, every held instance, re-solve).
    if let Some(h) = hub_of(tx.world(), scope) {
        let of = tx.get::<Instance>(instance).map(|i| i.of);
        if let Some(d) = of.filter(|d| tx.get::<DependsOn>(h).is_some_and(|x| x.0.contains(d))) {
            tx.remove_dependency(h, d)?;
            tx.add_dependency(h, d)?;
            return Ok(());
        }
    }
    let joints = joints_in(tx.world(), scope);
    for g in groups(&joints) {
        if !g.members.contains(&instance) {
            continue;
        }
        for m in g.held().filter(|m| *m != instance).collect::<Vec<_>>() {
            let j = g.joints[0];
            tx.remove_dependency(m, j)?;
            tx.add_dependency(m, j)?;
        }
    }
    Ok(())
}

/// The scope's held instances solve again (one moved by hand along what its
/// mates leave free: the others start from where they are too).
pub(crate) fn remark(tx: &mut Tx, scope: Option<StableId>) -> Result<(), CommandError> {
    if let Some(h) = hub_of(tx.world(), scope)
        && let Some(d) = tx.get::<DependsOn>(h).and_then(|x| x.0.first().copied())
    {
        tx.remove_dependency(h, d)?;
        tx.add_dependency(h, d)?;
    }
    Ok(())
}

/// Why an instance cannot be moved by hand, if a joint holds it.
pub fn held_reason(world: &World, instance: StableId) -> Option<String> {
    let joints = holding(world, instance);
    let first = *joints.first()?;
    let e = entity(world, instance)?;
    let by = world.get::<Placed>(e).and_then(|p| p.by).unwrap_or(first);
    let j = world.get::<Joint>(entity(world, by)?)?;
    // What mates hold is moved by hand along what they leave free (the solve
    // starts from where it is put); only a lock holds it outright.
    if j.kind.mate().is_some() {
        return None;
    }
    let other = [j.a.instance(), j.b.instance()].into_iter().flatten().find(|o| *o != instance).map(|o| instance_name(world, o)).unwrap_or_else(|| "the ground".into());
    let how = match j.kind {
        JointKind::Fixed => "change the joint's offset or connectors".to_string(),
        JointKind::Lock => "delete the lock".to_string(),
        k => format!("change its {}", k.values().iter().map(|(_, l, _)| l.to_lowercase()).collect::<Vec<_>>().join(" or ")),
    };
    Some(format!(
        "{} is held by the {} joint “{}” to {other}: {how}, or delete the joint to move it freely",
        instance_name(world, instance),
        j.kind.word(),
        joint_name(world, by)
    ))
}

/// Deletes the joints that name an instance (it is being deleted).
pub(crate) fn delete_joints_of(tx: &mut Tx, instance: StableId, scope: Option<StableId>) -> Result<(), CommandError> {
    freeze(tx, scope)?;
    for (id, j) in joints_in(tx.world(), scope) {
        if j.a.instance() == Some(instance) || j.b.instance() == Some(instance) {
            drop_joint(tx, id)?;
        }
    }
    Ok(())
}

fn drop_joint(tx: &mut Tx, id: StableId) -> Result<(), CommandError> {
    // Its couplings go with it.
    crate::couplings::delete_couplings_of(tx, id)?;
    // Nothing else may keep depending on it.
    let users: Vec<StableId> = tx.world().resource::<IdIndex>().iter().filter(|(_, e)| tx.world().get::<DependsOn>(*e).is_some_and(|d| d.0.contains(&id))).map(|(u, _)| u).collect();
    for u in users {
        tx.remove_dependency(u, id)?;
    }
    tx.delete(id)
}

/// Copies the joints of one assembly onto its copy (instances mapped old → new).
pub(crate) fn copy_joints(tx: &mut Tx, from: StableId, to: StableId, map: &HashMap<StableId, StableId>) -> Result<(), CommandError> {
    for (id, mut j) in joints_in(tx.world(), Some(from)) {
        let Some(set) = tx.get::<ParameterSet>(id).cloned() else { continue };
        for end in [&mut j.a, &mut j.b] {
            if let End::Connector { instance, .. } = end
                && let Some(n) = map.get(instance)
            {
                *instance = *n;
            }
        }
        j.assembly = Some(to);
        let c = tx.create();
        for r in de_shape::params::references(&set.params) {
            if r != id {
                tx.add_dependency(c, r)?;
            }
        }
        tx.set(c, set)?;
        tx.set(c, j)?;
    }
    rewire(tx, Some(to))
}

// ---- Commands ----------------------------------------------------------------------

/// A joint value as given: a number, or a formula ("=Arm.angle * 2").
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ValueIn {
    Number(f64),
    Formula(String),
}

fn check_end(tx: &Tx, end: &End, scope: Option<StableId>, which: &str) -> Result<(), CommandError> {
    match end {
        End::Connector { instance, connector: c } => {
            let inst = tx.get::<Instance>(*instance).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{which}: {instance} is not an instance")))?;
            if inst.parent != scope {
                let place = |s: Option<StableId>| match s.and_then(|s| tx.get::<AssemblyDef>(s)) {
                    Some(a) => format!("in {}", a.name),
                    None => "at the top of the project".to_string(),
                };
                return Err(bad(ErrorCode::InvalidParameters, format!("{which}: {} is {}, but the joint is {}: join instances of the same assembly", inst.name, place(inst.parent), place(scope))));
            }
            connector(tx.world(), *instance, c).map_err(|m| bad(ErrorCode::InvalidParameters, format!("{which}: {m}")))?;
        }
        End::Frame { at, axis, x } => {
            let ok = at.iter().chain(axis.iter()).chain(x.iter().flatten()).all(|v| v.is_finite());
            if !ok || tf::norm(*axis) < 1e-9 {
                return Err(bad(ErrorCode::InvalidParameters, format!("{which}: a ground frame needs a finite point and an axis")));
            }
            if let Some(x) = x {
                let z = tf::scale(*axis, 1.0 / tf::norm(*axis));
                if tf::norm(tf::sub(*x, tf::scale(z, tf::dot(*x, z)))) < 1e-9 {
                    return Err(bad(ErrorCode::InvalidParameters, format!("{which}: x must not lie along the axis")));
                }
            }
        }
        End::Geometry { instance, geometry } => {
            let inst = tx.get::<Instance>(*instance).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{which}: {instance} is not an instance")))?;
            if inst.parent != scope {
                return Err(bad(ErrorCode::InvalidParameters, format!("{which}: {} is in another assembly than the joint: join instances of the same assembly", inst.name)));
            }
            crate::geometry::resolve(tx.world(), *instance, geometry).map_err(|m| bad(ErrorCode::InvalidParameters, format!("{which}: {} › {m}", inst.name)))?;
        }
        End::Ground(_) => {}
    }
    Ok(())
}

/// Where each end's instance is now (for checks made before the joint exists).
fn poses_now(world: &World, j: &Joint) -> HashMap<StableId, Transform> {
    [j.a.instance(), j.b.instance()].into_iter().flatten().filter_map(|id| Some((id, crate::placement(world, entity(world, id)?)?))).collect()
}

/// A joint as the solver would see it, from its stored form (values zero).
fn trial_link(world: &World, j: &Joint) -> Result<Link, String> {
    Ok(Link {
        id: StableId::new(),
        name: format!("the {} joint", j.kind.word()),
        kind: j.kind,
        a: side(world, &j.a, j.kind)?,
        b: side(world, &j.b, j.kind)?,
        offset: j.offset,
        aligned: j.aligned,
        free: j.free,
        values: vec![0.0; j.kind.values().len()],
        hold: None,
    })
}

/// A lock keeps where its second end is now against its first.
fn lock_hold(world: &World, j: &Joint) -> Result<Option<Hold>, CommandError> {
    if j.kind != JointKind::Lock {
        return Ok(None);
    }
    let l = trial_link(world, j).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
    let poses = poses_now(world, j);
    let (Some(wa), Some(wb)) = (Link::world(&l.a, &poses), Link::world(&l.b, &poses)) else {
        return Err(bad(ErrorCode::InvalidGeometry, "a lock needs both ends placed"));
    };
    let h = wa.inverse().then(&wb);
    let r9 = |v: f64| (v * 1e9).round() / 1e9;
    Ok(Some(Hold { at: h.at.map(r9), rotation: h.euler() }))
}

fn check_joint(tx: &Tx, j: &Joint) -> Result<(), CommandError> {
    if let Some(s) = j.assembly
        && tx.get::<AssemblyDef>(s).is_none()
    {
        return Err(bad(ErrorCode::UnresolvedReference, format!("{s} is not an assembly")));
    }
    check_end(tx, &j.a, j.assembly, "a")?;
    check_end(tx, &j.b, j.assembly, "b")?;
    match (j.a.instance(), j.b.instance()) {
        (None, None) => return Err(bad(ErrorCode::InvalidParameters, "a joint joins an instance to another, or to the ground: both ends are ground")),
        (Some(a), Some(b)) if a == b => return Err(bad(ErrorCode::InvalidParameters, "a joint joins two different instances")),
        _ => {}
    }
    if !j.offset.is_finite() {
        return Err(bad(ErrorCode::InvalidParameters, "offset must be a finite number"));
    }
    if j.kind.mate().is_some() {
        let l = trial_link(tx.world(), j).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
        l.check_mate(&poses_now(tx.world(), j)).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
    }
    // Pattern copies are placed by their pattern.
    for id in [j.a.instance(), j.b.instance()].into_iter().flatten() {
        if let Some(why) = crate::patterns::placed_by_pattern(tx.world(), id) {
            return Err(bad(ErrorCode::InvalidParameters, why));
        }
    }
    Ok(())
}

/// A mate's value where its ends are now (an angle or distance not given takes it).
fn measured_now(world: &World, j: &Joint) -> Option<f64> {
    let l = trial_link(world, j).ok()?;
    l.measure(&poses_now(world, j))?.first().copied()
}

/// The parameters a joint of `kind` holds, keeping what `old` had for the same keys.
fn params_for(kind: JointKind, old: &[Param]) -> Vec<Param> {
    kind.values()
        .iter()
        .map(|(key, label, unit)| {
            old.iter().find(|p| p.key == *key).cloned().unwrap_or(Param {
                key: key.to_string(),
                label: label.to_string(),
                value: 0.0,
                unit: unit.to_string(),
                min: None,
                max: None,
                step: Some(if *unit == "deg" { 1.0 } else { 0.5 }),
                expr: None,
            })
        })
        .collect()
}

/// Applies given values, formulas and ranges to a joint's parameters.
fn apply_values(params: &mut [Param], kind: JointKind, value: Option<ValueIn>, values: BTreeMap<String, ValueIn>, limits: Option<Option<[f64; 2]>>, ranges: BTreeMap<String, Option<[f64; 2]>>) -> Result<(), CommandError> {
    let primary = kind.values().first().map(|(k, _, _)| k.to_string());
    let mut given: Vec<(String, ValueIn)> = values.into_iter().collect();
    if let Some(v) = value {
        let k = primary.clone().ok_or_else(|| bad(ErrorCode::InvalidParameters, format!("a {} joint has no value", kind.word())))?;
        given.push((k, v));
    }
    let mut range: Vec<(String, Option<[f64; 2]>)> = ranges.into_iter().collect();
    if let Some(l) = limits {
        let k = primary.ok_or_else(|| bad(ErrorCode::InvalidParameters, format!("a {} joint has no value to limit", kind.word())))?;
        range.push((k, l));
    }
    let keys = || kind.values().iter().map(|(k, _, _)| *k).collect::<Vec<_>>().join(", ");
    for (k, r) in range {
        let p = params.iter_mut().find(|p| p.key == k).ok_or_else(|| bad(ErrorCode::InvalidParameters, format!("a {} joint has no value “{k}” (it has {})", kind.word(), keys())))?;
        match r {
            Some([lo, hi]) => {
                if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                    return Err(bad(ErrorCode::InvalidParameters, format!("limits of {} must be two finite numbers, the smaller first", p.label.to_lowercase())));
                }
                p.min = Some(lo);
                p.max = Some(hi);
            }
            None => {
                p.min = None;
                p.max = None;
            }
        }
    }
    for (k, v) in given {
        let p = params.iter_mut().find(|p| p.key == k).ok_or_else(|| bad(ErrorCode::InvalidParameters, format!("a {} joint has no value “{k}” (it has {})", kind.word(), keys())))?;
        match v {
            ValueIn::Number(n) => {
                p.value = n;
                p.expr = None;
            }
            ValueIn::Formula(f) => {
                let f = f.trim();
                if !f.starts_with('=') {
                    match f.parse::<f64>() {
                        Ok(n) => {
                            p.value = n;
                            p.expr = None;
                        }
                        Err(_) => return Err(bad(ErrorCode::InvalidParameters, format!("{}: a value is a number or a formula starting with =", p.label))),
                    }
                } else {
                    p.expr = Some(f.to_string());
                }
            }
        }
    }
    for p in params.iter() {
        if p.expr.is_none() && (p.min.is_some_and(|m| p.value < m - 1e-9) || p.max.is_some_and(|m| p.value > m + 1e-9)) {
            return Err(bad(ErrorCode::InvalidParameters, format!("{} {} is outside its limits {}–{}", p.label, p.value, p.min.unwrap_or(f64::NEG_INFINITY), p.max.unwrap_or(f64::INFINITY))));
        }
    }
    Ok(())
}

/// Stores a joint's parameters; formulas go through `shape.link_param`, so
/// they are checked, stored by id and depended on.
fn store_params(tx: &mut Tx, id: StableId, params: Vec<Param>) -> Result<(), CommandError> {
    let old = tx.get::<ParameterSet>(id).map(|s| s.params.clone()).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{id} is not a joint")))?;
    let old_expr = |k: &str| old.iter().find(|p| p.key == k).and_then(|p| p.expr.clone());
    // Formulas that go or change are unlinked first (dropping what they read).
    for p in &old {
        let now = params.iter().find(|n| n.key == p.key).and_then(|n| n.expr.clone());
        if p.expr.is_some() && now != p.expr {
            tx.run("shape.link_param", json!({ "target": id, "key": p.key, "expr": null }))?;
        }
    }
    let mut set = tx.get::<ParameterSet>(id).cloned().unwrap();
    set.params = params.iter().cloned().map(|mut p| {
        p.expr = p.expr.filter(|e| old_expr(&p.key).as_ref() == Some(e));
        p
    }).collect();
    tx.set(id, set)?;
    for p in &params {
        if let Some(e) = &p.expr
            && old_expr(&p.key).as_ref() != Some(e)
        {
            tx.run("shape.link_param", json!({ "target": id, "key": p.key, "expr": e }))?;
        }
    }
    Ok(())
}

fn unique_name(tx: &Tx, kind: JointKind, wanted: Option<String>, except: Option<StableId>) -> Result<String, CommandError> {
    let taken = |n: &str| {
        tx.world().resource::<IdIndex>().iter().any(|(id, e)| Some(id) != except && tx.world().get::<Joint>(e).is_some() && tx.world().get::<ParameterSet>(e).is_some_and(|s| s.name == n))
    };
    match wanted.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        Some(n) => {
            if n.contains('.') {
                return Err(bad(ErrorCode::InvalidParameters, "a joint's name has no dots (formulas read its values as Name.angle)"));
            }
            if taken(&n) {
                return Err(bad(ErrorCode::InvalidParameters, format!("there is already a joint called “{n}”")));
            }
            Ok(n)
        }
        None => Ok((1..).map(|i| format!("{} {i}", kind.title())).find(|n| !taken(n)).unwrap()),
    }
}

/// Solves the group a joint is in now, so a joint that cannot hold is refused
/// with the reason rather than stored broken.
fn check_solves(tx: &Tx, scope: Option<StableId>) -> Result<(), CommandError> {
    solve_scope(tx.world(), scope, &HashMap::new(), &HashMap::new()).map(|_| ()).map_err(|m| bad(ErrorCode::InvalidGeometry, m))
}

/// `assembly.joint.create`: join two instances (or one and the ground) at
/// their connectors.
#[derive(Deserialize, JsonSchema)]
pub struct CreateJoint {
    /// The assembly whose instances it joins (none: top-level instances).
    #[serde(default)]
    pub assembly: Option<StableId>,
    #[serde(rename = "type")]
    pub kind: JointKind,
    /// `{ instance, connector }`, "ground", or a ground frame `{ at, axis?, x? }`.
    pub a: End,
    /// Placed on `a`: `{ instance, connector }`, "ground" or a ground frame.
    pub b: End,
    pub name: Option<String>,
    /// The main value (hinge/cylinder angle °, slider distance mm): a number or "=formula".
    pub value: Option<ValueIn>,
    /// Values by key (angle, distance, rx, ry, rz).
    #[serde(default)]
    pub values: BTreeMap<String, ValueIn>,
    /// [min, max] of the main value.
    pub limits: Option<[f64; 2]>,
    /// [min, max] by key.
    #[serde(default)]
    pub ranges: BTreeMap<String, [f64; 2]>,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub aligned: bool,
    #[serde(default)]
    pub free: bool,
}

impl Command for CreateJoint {
    const NAME: &'static str = "assembly.joint.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut joint = Joint { kind: self.kind, assembly: self.assembly, a: self.a, b: self.b, offset: self.offset, aligned: self.aligned, free: self.free, hold: None };
        check_joint(tx, &joint)?;
        joint.hold = lock_hold(tx.world(), &joint)?;
        let name = unique_name(tx, self.kind, self.name, None)?;
        let mut params = params_for(self.kind, &[]);
        // An angle or distance mate given no value keeps the one it has now.
        let mut value = self.value;
        if value.is_none() && self.values.is_empty() && self.kind.mate().is_some() && !self.kind.values().is_empty() {
            value = measured_now(tx.world(), &joint).map(|v| ValueIn::Number((v * 1e6).round() / 1e6));
        }
        apply_values(&mut params, self.kind, value, self.values, self.limits.map(Some), self.ranges.into_iter().map(|(k, v)| (k, Some(v))).collect())?;
        freeze(tx, joint.assembly)?;
        let out = tx.run("parameters.create", json!({ "name": name, "description": format!("{} joint", self.kind.title()), "params": params }))?;
        let id: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| bad(ErrorCode::InvalidParameters, e.to_string()))?;
        tx.set(id, joint.clone())?;
        rewire(tx, joint.assembly)?;
        check_solves(tx, joint.assembly)?;
        Ok(json!({ "id": id, "name": name }))
    }
}

/// `assembly.joint.update`: change a joint's values, limits, ends, type or
/// name. `limits: null` removes the main value's limits.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateJoint {
    pub target: StableId,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<JointKind>,
    pub a: Option<End>,
    pub b: Option<End>,
    pub value: Option<ValueIn>,
    #[serde(default)]
    pub values: BTreeMap<String, ValueIn>,
    #[serde(default, deserialize_with = "some")]
    pub limits: Option<Option<[f64; 2]>>,
    #[serde(default)]
    pub ranges: BTreeMap<String, Option<[f64; 2]>>,
    pub offset: Option<f64>,
    pub aligned: Option<bool>,
    pub free: Option<bool>,
}

/// `null` given (Some(None)) apart from left out (None).
fn some<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

impl Command for UpdateJoint {
    const NAME: &'static str = "assembly.joint.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut joint = tx.get::<Joint>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a joint", self.target)))?;
        let mut set = tx.get::<ParameterSet>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} has no values", self.target)))?;
        let before = joint.clone();
        if let Some(k) = self.kind {
            joint.kind = k;
        }
        if let Some(a) = self.a {
            joint.a = a;
        }
        if let Some(b) = self.b {
            joint.b = b;
        }
        if let Some(o) = self.offset {
            joint.offset = o;
        }
        if let Some(al) = self.aligned {
            joint.aligned = al;
        }
        if let Some(f) = self.free {
            joint.free = f;
        }
        check_joint(tx, &joint)?;
        if joint.kind != before.kind || joint.a != before.a || joint.b != before.b {
            joint.hold = lock_hold(tx.world(), &joint)?;
        }
        if let Some(n) = self.name {
            let n = unique_name(tx, joint.kind, Some(n), Some(self.target))?;
            if n != set.name {
                set.name = n;
                set.description = format!("{} joint", joint.kind.title());
                tx.set(self.target, set.clone())?;
            }
        }
        let mut params = params_for(joint.kind, &set.params);
        apply_values(&mut params, joint.kind, self.value, self.values, self.limits, self.ranges)?;
        let structural = joint.a != before.a || joint.b != before.b || joint.kind != before.kind;
        if structural {
            freeze(tx, joint.assembly)?;
        }
        if params != set.params || joint.kind != before.kind {
            if joint.kind != before.kind {
                set.description = format!("{} joint", joint.kind.title());
                tx.set(self.target, set)?;
            }
            store_params(tx, self.target, params)?;
        }
        tx.set(self.target, joint.clone())?;
        if structural {
            rewire(tx, joint.assembly)?;
        }
        check_solves(tx, joint.assembly)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.joint.delete`: remove a joint; what it held stays where it is, free.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteJoint {
    pub target: StableId,
}

impl Command for DeleteJoint {
    const NAME: &'static str = "assembly.joint.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let joint = tx.get::<Joint>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a joint", self.target)))?;
        freeze(tx, joint.assembly)?;
        drop_joint(tx, self.target)?;
        rewire(tx, joint.assembly)?;
        Ok(json!({ "id": self.target }))
    }
}
