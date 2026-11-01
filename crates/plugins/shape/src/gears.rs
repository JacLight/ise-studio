//! Gear checks: does a gear train turn cleanly, and are its teeth strong enough?
//!
//! A program declares how its parts move (`hinge` for gears on fixed axes, or
//! joints, a drive and couplings for gears on a turning carrier: a planetary
//! train) and which gears drive which (`mesh`). From that:
//!
//! - the **sweep** turns every hinged body at its gear ratio through one full
//!   turn of the driving gear, in steps fine enough that each tooth passes in
//!   at least 16, and measures the gap between every pair of parts that move
//!   relative to each other: a clash anywhere in the turn is reported with the
//!   angle where it happens, not only in the pose the parts were drawn in;
//! - each **mesh** gets its ratio, its contact ratio (how many tooth pairs
//!   share the load; below 1 the drive jerks) and its backlash, from the
//!   involute geometry and the actual distance between the two axes;
//! - with a load (input torque and speed) each mesh gets tooth **strength**:
//!   bending at the tooth root (Lewis, with the Barth velocity factor) and
//!   surface contact (Hertz), as safety factors against typical allowable
//!   stresses of each part's material. Design guidance, not certification:
//!   the allowable values are typical figures, and the report says so.

use crate::mechanism::{self, Kind, Linkage};
use crate::runtime::Evaluated;
use manifold_csg::Manifold;
use de_core::schemars::{self, JsonSchema};
use serde::Serialize;
use serde_json::Value;
use std::f64::consts::PI;

/// Parts that turn together about an axis.
#[derive(Clone, Debug)]
pub struct Body {
    pub parts: Vec<String>,
    pub at: [f64; 3],
    pub axis: [f64; 3],
}

/// Two gears in mesh, the first driving the second.
#[derive(Clone, Debug)]
pub struct Mesh {
    pub driver: String,
    pub driven: String,
    pub module: f64,
    pub teeth: [u32; 2],
    /// Degrees.
    pub pressure_angle: f64,
    pub width: f64,
    pub backlash: f64,
    pub internal: bool,
}

/// How a mechanism's bodies turn together, for showing it move.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Motion {
    /// The body that drives the rest.
    pub driver: usize,
    pub bodies: Vec<MotionBody>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MotionBody {
    pub parts: Vec<String>,
    /// A point on the axis (mm, program coordinates) and its direction.
    pub at: [f64; 3],
    pub axis: [f64; 3],
    /// Turns per turn of the driving body, signed (negative turns the other way).
    pub speed: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Mechanism {
    pub bodies: Vec<Body>,
    pub meshes: Vec<Mesh>,
}

fn v3(v: &Value) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

impl Mechanism {
    pub fn parse(v: &Value) -> Result<Mechanism, String> {
        let mut m = Mechanism::default();
        for b in v["bodies"].as_array().into_iter().flatten() {
            let parts = b["parts"].as_array().ok_or("a hinge without parts")?.iter().filter_map(|p| p.as_str().map(str::to_string)).collect();
            m.bodies.push(Body { parts, at: v3(&b["at"]).ok_or("a hinge without a point")?, axis: v3(&b["axis"]).ok_or("a hinge without an axis")? });
        }
        for g in v["meshes"].as_array().into_iter().flatten() {
            let teeth = g["teeth"].as_array().ok_or("a mesh without teeth")?;
            m.meshes.push(Mesh {
                driver: g["driver"].as_str().ok_or("a mesh without its driving part")?.to_string(),
                driven: g["driven"].as_str().ok_or("a mesh without its driven part")?.to_string(),
                module: g["module"].as_f64().ok_or("a mesh without a module")?,
                teeth: [teeth.first().and_then(Value::as_u64).unwrap_or(0) as u32, teeth.get(1).and_then(Value::as_u64).unwrap_or(0) as u32],
                pressure_angle: g["pressure_angle"].as_f64().unwrap_or(20.0),
                width: g["width"].as_f64().ok_or("a mesh without a face width")?,
                backlash: g["backlash"].as_f64().unwrap_or(0.0),
                internal: g["internal"].as_bool().unwrap_or(false),
            });
        }
        Ok(m)
    }

    fn body_of(&self, part: &str) -> Option<usize> {
        self.bodies.iter().position(|b| b.parts.iter().any(|p| p == part))
    }

    /// Each body with its axis and speed (see `speeds`).
    pub fn motion(&self) -> Result<Motion, String> {
        let (driver, speeds) = self.speeds()?;
        Ok(Motion {
            driver,
            bodies: self.bodies.iter().zip(speeds).map(|(b, speed)| MotionBody { parts: b.parts.clone(), at: b.at, axis: b.axis, speed }).collect(),
        })
    }

    /// How far each body turns per turn of the driving body (signed), and
    /// which body drives: the first mesh's driver, else the first hinge.
    pub fn speeds(&self) -> Result<(usize, Vec<f64>), String> {
        if self.bodies.is_empty() {
            return Err("nothing is hinged: declare hinge(…) for the parts that turn (or joint(…)s and a drive(…))".into());
        }
        let driver = match self.meshes.first() {
            Some(g) => self.body_of(&g.driver).ok_or_else(|| format!("“{}” drives a gear but is not hinged", g.driver))?,
            None => 0,
        };
        let mut speed = vec![f64::NAN; self.bodies.len()];
        speed[driver] = 1.0;
        // Spread speeds through the meshes until nothing changes.
        for _ in 0..self.meshes.len() + 1 {
            for g in &self.meshes {
                let unhinged = |p: &str| format!("“{p}” is in a mesh but not hinged (gears whose axes are carried round, as a planetary's planets, and a ring that stays put: join the gears with joint(…), drive(…) and couple(…) instead of hinges, and the check follows the joints)");
                let a = self.body_of(&g.driver).ok_or_else(|| unhinged(&g.driver))?;
                let b = self.body_of(&g.driven).ok_or_else(|| unhinged(&g.driven))?;
                let ratio = g.teeth[0] as f64 / g.teeth[1] as f64 * if g.internal { 1.0 } else { -1.0 };
                if speed[a].is_finite() && !speed[b].is_finite() {
                    speed[b] = speed[a] * ratio;
                } else if speed[b].is_finite() && !speed[a].is_finite() {
                    speed[a] = speed[b] / ratio;
                }
            }
        }
        // Hinged but driven by nothing: it stays where it is.
        Ok((driver, speed.into_iter().map(|s| if s.is_finite() { s } else { 0.0 }).collect()))
    }
}

/// A rotation of `deg` degrees about an axis through `at`, as Manifold's 3×4 matrix.
fn about(at: [f64; 3], axis: [f64; 3], deg: f64) -> [f64; 12] {
    let (s, c) = deg.to_radians().sin_cos();
    let [x, y, z] = axis;
    let t = 1.0 - c;
    // Rodrigues' rotation matrix, rows.
    let r = [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ];
    // p' = R (p − at) + at  →  translation = at − R·at
    let tr = [0, 1, 2].map(|i| at[i] - (r[i][0] * at[0] + r[i][1] * at[1] + r[i][2] * at[2]));
    // Column-major: X basis, Y basis, Z basis, translation.
    [r[0][0], r[1][0], r[2][0], r[0][1], r[1][1], r[2][1], r[0][2], r[1][2], r[2][2], tr[0], tr[1], tr[2]]
}

/// Two parts' closest approach through the turn.
#[derive(Clone, Debug, Serialize)]
pub struct PairGap {
    pub a: String,
    pub b: String,
    /// Smallest gap (mm; 0 = touching or overlapping).
    pub gap: f64,
    /// Angle of the driving body where it happens (degrees).
    pub at: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sweep {
    /// The driving body's parts, and the steps of its one full turn.
    pub driver: Vec<String>,
    pub steps: usize,
    /// Every pair of parts that came within the search distance, closest first.
    pub pairs: Vec<PairGap>,
    /// Pairs that touch or overlap somewhere in the turn.
    pub clashes: Vec<PairGap>,
    /// How far the pairs were watched (mm): gaps beyond it count as clear.
    pub search: f64,
    /// The finest gap the check can tell apart from touching (mm): the
    /// accuracy of the facets it measured. Smaller gaps are running fits.
    pub resolution: f64,
}

/// How the check moves the parts: which body each part is on, and every
/// body's pose at each step of one full turn of the driving gear.
struct Train {
    /// The driving body's parts.
    driver: Vec<String>,
    bodies: Vec<Vec<String>>,
    /// The driving joint's value at each step (degrees).
    at: Vec<f64>,
    /// Each step's pose of each body (Manifold's 3×4); None: as drawn.
    poses: Vec<Vec<Option<[f64; 12]>>>,
}

const IDENTITY: [f64; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

impl Train {
    fn body_of(&self, part: &str) -> Option<usize> {
        self.bodies.iter().position(|b| b.iter().any(|p| p == part))
    }

    /// Gears on hinges about fixed axes, each turning at its gear ratio.
    fn of_hinges(m: &Mechanism) -> Result<Train, String> {
        let (driver, speed) = m.speeds()?;
        // A tooth passes 16 steps or more, for every mesh.
        let passes = m.meshes.iter().filter_map(|g| m.body_of(&g.driver).map(|b| speed[b].abs() * g.teeth[0] as f64)).fold(0.0_f64, f64::max);
        let steps = ((passes * 16.0).ceil() as usize).clamp(72, 1440);
        let at: Vec<f64> = (0..steps).map(|k| 360.0 * k as f64 / steps as f64).collect();
        let poses = at.iter().map(|a| m.bodies.iter().zip(&speed).map(|(b, s)| (*s != 0.0).then(|| about(b.at, b.axis, a * s))).collect()).collect();
        Ok(Train { driver: m.bodies[driver].parts.clone(), bodies: m.bodies.iter().map(|b| b.parts.clone()).collect(), at, poses })
    }

    /// Gears that follow the program's joints, from the linkage's frames
    /// through one full turn of its drive (the last frame, back at the
    /// start, left off).
    fn of_joints(l: &Linkage, frames: &[mechanism::Frame]) -> Train {
        let steps = frames.len().saturating_sub(1).max(1);
        let driver = l.drive.as_ref().and_then(|d| driven_side(&l.joints[d.joint])).map(|b| l.bodies[b].clone()).unwrap_or_default();
        Train {
            driver,
            bodies: l.bodies.clone(),
            at: frames[..steps].iter().map(|f| f.drive).collect(),
            poses: frames[..steps].iter().map(|f| f.bodies.iter().map(|(t, q)| Some(mechanism::pose_matrix(*t, *q))).collect()).collect(),
        }
    }

    /// The step whose angle is nearest `at`.
    fn step_at(&self, at: f64) -> usize {
        (0..self.at.len()).min_by(|&i, &j| (self.at[i] - at).abs().total_cmp(&(self.at[j] - at).abs())).unwrap_or(0)
    }

    /// A part posed at a step (wrapping round the turn).
    fn posed(&self, e: &Evaluated, name: &str, step: isize) -> Option<Manifold> {
        let p = e.parts.iter().find(|p| p.name == name)?;
        let k = step.rem_euclid(self.at.len() as isize) as usize;
        Some(match self.body_of(name).and_then(|b| self.poses[k][b]) {
            Some(m) => p.solid.transform(&m),
            None => p.solid.translate(0.0, 0.0, 0.0),
        })
    }
}

/// The moving side of a joint (the other one is the ground, or comes second).
fn driven_side(j: &mechanism::Joint) -> Option<usize> {
    if j.a.is_none() { j.b } else { j.a }
}

/// Turns the mechanism through one full turn of its driving body.
pub fn sweep(e: &Evaluated, m: &Mechanism, search: f64) -> Result<Sweep, String> {
    sweep_train(e, &Train::of_hinges(m)?, search)
}

fn sweep_train(e: &Evaluated, t: &Train, search: f64) -> Result<Sweep, String> {
    for b in &t.bodies {
        for p in b {
            if !e.parts.iter().any(|q| &q.name == p) {
                return Err(format!("“{p}” turns in the mechanism but the program has no part of that name"));
            }
        }
    }
    let body_of: Vec<Option<usize>> = e.parts.iter().map(|p| t.body_of(&p.name)).collect();
    let moves: Vec<bool> = (0..t.bodies.len()).map(|b| t.poses.iter().any(|s| s[b].is_some_and(|m| m.iter().zip(IDENTITY).any(|(x, y)| (x - y).abs() > 1e-9)))).collect();
    // Each part's box over the turn: its corners carried round, with a margin for between the steps.
    let boxes: Vec<([f64; 3], [f64; 3])> = e
        .parts
        .iter()
        .zip(&body_of)
        .map(|(p, b)| {
            let (lo, hi) = (p.bounds_min, p.bounds_max);
            let Some(b) = b.filter(|&b| moves[b]) else { return (lo, hi) };
            let pad = 0.05 * (0..3).map(|d| (hi[d] - lo[d]).powi(2)).sum::<f64>().sqrt();
            let (mut a, mut z) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            for m in t.poses.iter().map(|s| s[b].unwrap_or(IDENTITY)) {
                for c in 0..8 {
                    let v = [if c & 1 == 0 { lo[0] } else { hi[0] }, if c & 2 == 0 { lo[1] } else { hi[1] }, if c & 4 == 0 { lo[2] } else { hi[2] }];
                    for i in 0..3 {
                        let x = m[i] * v[0] + m[3 + i] * v[1] + m[6 + i] * v[2] + m[9 + i];
                        a[i] = a[i].min(x - pad);
                        z[i] = z[i].max(x + pad);
                    }
                }
            }
            (a, z)
        })
        .collect();
    // Pairs that move relative to each other and can come close.
    let mut pairs = Vec::new();
    for i in 0..e.parts.len() {
        for j in i + 1..e.parts.len() {
            let (bi, bj) = (body_of[i], body_of[j]);
            let moving = |b: Option<usize>| b.is_some_and(|b| moves[b]);
            if bi == bj || !(moving(bi) || moving(bj)) {
                continue; // they move together, or neither moves
            }
            let ((li, hi), (lj, hj)) = (boxes[i], boxes[j]);
            if (0..3).all(|d| li[d] - search <= hj[d] && lj[d] - search <= hi[d]) {
                pairs.push((i, j));
            }
        }
    }
    let mut closest: Vec<(f64, f64)> = vec![(f64::INFINITY, 0.0); pairs.len()];
    for (step, pose) in t.poses.iter().enumerate() {
        let posed: Vec<Option<Manifold>> = (0..e.parts.len())
            .map(|k| {
                if !pairs.iter().any(|(i, j)| *i == k || *j == k) {
                    return None;
                }
                let solid = &e.parts[k].solid;
                Some(match body_of[k].and_then(|b| pose[b]) {
                    Some(m) => solid.transform(&m),
                    None => solid.translate(0.0, 0.0, 0.0),
                })
            })
            .collect();
        for (n, (i, j)) in pairs.iter().enumerate() {
            let (Some(a), Some(b)) = (&posed[*i], &posed[*j]) else { continue };
            let g = a.min_gap(b, search);
            if g < closest[n].0 {
                closest[n] = (g, t.at[step]);
            }
        }
    }
    let mut out: Vec<PairGap> = pairs
        .iter()
        .zip(&closest)
        .filter(|(_, (g, _))| *g < search)
        .map(|((i, j), (g, at))| PairGap { a: e.parts[*i].name.clone(), b: e.parts[*j].name.clone(), gap: (g * 1e4).round() / 1e4, at: (at * 100.0).round() / 100.0 })
        .collect();
    out.sort_by(|a, b| a.gap.total_cmp(&b.gap));
    let clashes = out.iter().filter(|p| p.gap < 1e-4).cloned().collect();
    Ok(Sweep { driver: t.driver.clone(), steps: t.at.len(), pairs: out, clashes, search, resolution: 0.0 })
}

/// A mesh's geometry: ratio, contact ratio and backlash.
#[derive(Clone, Debug, Serialize)]
pub struct MeshReport {
    pub driver: String,
    pub driven: String,
    pub teeth: [u32; 2],
    pub module: f64,
    /// Driven turns per driver turn, as "n : 1" reduction.
    pub ratio: f64,
    /// Average tooth pairs in contact; below 1 the teeth lose contact.
    pub contact_ratio: f64,
    /// Distance between the axes: as built, and the standard one for these gears.
    pub center_distance: f64,
    pub standard_center_distance: f64,
    /// Play at the pitch circle (mm), from tooth thinning and the axes' spread.
    pub backlash: f64,
    pub strength: Option<Strength>,
    pub notes: Vec<String>,
}

/// Typical elastic and allowable figures for a material (MPa).
#[derive(Clone, Copy, Debug)]
struct Props {
    e: f64,
    nu: f64,
    bending: f64,
    contact: f64,
}

/// Typical values by material name (design guidance: check the datasheet).
fn props(material: &str) -> Option<(Props, &'static str)> {
    let m = material.to_lowercase();
    let p = |e, nu, bending, contact| Props { e, nu, bending, contact };
    Some(if m.contains("steel") {
        (p(200_000.0, 0.30, 195.0, 645.0), "steel, through-hardened ~200 HB")
    } else if m.contains("brass") || m.contains("bronze") {
        (p(100_000.0, 0.34, 100.0, 400.0), "brass/bronze")
    } else if m.contains("alumin") {
        (p(69_000.0, 0.33, 90.0, 250.0), "aluminium 6061-T6")
    } else if m.contains("acetal") || m.contains("pom") || m.contains("delrin") {
        (p(2_800.0, 0.35, 35.0, 55.0), "acetal (POM)")
    } else if m.contains("nylon") || m.contains("pa6") || m.contains("pa12") {
        (p(2_700.0, 0.39, 30.0, 50.0), "nylon")
    } else if m.contains("pla") {
        (p(3_500.0, 0.36, 20.0, 35.0), "PLA, printed")
    } else if m.contains("petg") {
        (p(2_100.0, 0.38, 18.0, 30.0), "PETG, printed")
    } else if m.contains("abs") {
        (p(2_200.0, 0.35, 16.0, 28.0), "ABS, printed")
    } else {
        return None;
    })
}

/// Lewis form factor for 20° full-depth teeth (Shigley, table 14-2), interpolated.
pub fn lewis_y(teeth: u32) -> f64 {
    const T: [(f64, f64); 26] = [
        (12.0, 0.245), (13.0, 0.261), (14.0, 0.277), (15.0, 0.290), (16.0, 0.296), (17.0, 0.303), (18.0, 0.309), (19.0, 0.314), (20.0, 0.322),
        (21.0, 0.328), (22.0, 0.331), (24.0, 0.337), (26.0, 0.346), (28.0, 0.353), (30.0, 0.359), (34.0, 0.371), (38.0, 0.384), (43.0, 0.397),
        (50.0, 0.409), (60.0, 0.422), (75.0, 0.435), (100.0, 0.447), (150.0, 0.460), (300.0, 0.472), (400.0, 0.480), (100_000.0, 0.485),
    ];
    let z = teeth as f64;
    if z <= T[0].0 {
        return T[0].1;
    }
    for w in T.windows(2) {
        if z <= w[1].0 {
            return w[0].1 + (w[1].1 - w[0].1) * (z - w[0].0) / (w[1].0 - w[0].0);
        }
    }
    0.485
}

/// Tooth stresses and safety factors at a load.
#[derive(Clone, Debug, Serialize)]
pub struct Strength {
    /// Torque on the driving gear of this mesh (N·m) and its speed (rpm).
    pub torque: f64,
    pub rpm: f64,
    /// Force along the pitch line (N), pitch line speed (m/s), velocity factor.
    pub tangential_force: f64,
    pub pitch_speed: f64,
    pub velocity_factor: f64,
    /// Root bending stress in each gear (MPa) and safety factors.
    pub bending: [f64; 2],
    pub bending_safety: [Option<f64>; 2],
    /// Contact stress at the pitch point (MPa) and the weaker gear's safety factor.
    pub contact: Option<f64>,
    pub contact_safety: Option<f64>,
    pub materials: [String; 2],
}

/// Where a gear turns, how fast (turns per turn of the drive, about its
/// axis), and how fast what carries its axis turns (a planet's carrier; 0
/// for an axis that stays put).
#[derive(Clone, Copy, Debug)]
struct Axis {
    at: [f64; 3],
    axis: [f64; 3],
    speed: f64,
    carrier: f64,
}

/// The geometry and (with a load) strength of every mesh. `materials` gives a
/// part's material name; `torque`/`rpm` are at the driving body.
pub fn meshes(m: &Mechanism, load: Option<(f64, f64)>, material: &dyn Fn(&str) -> Option<String>) -> Result<Vec<MeshReport>, String> {
    let (_, speed) = m.speeds()?;
    let axis = |part: &str| -> Result<Axis, String> {
        let b = m.body_of(part).ok_or_else(|| format!("“{part}” is not hinged"))?;
        Ok(Axis { at: m.bodies[b].at, axis: m.bodies[b].axis, speed: speed[b], carrier: 0.0 })
    };
    mesh_reports(&m.meshes, &|g| Ok((axis(&g.driver)?, axis(&g.driven)?, 1.0)), load, material)
}

/// Each mesh's report, given its two gears' axes and the drive's own speed
/// about the first one's axis.
fn mesh_reports(list: &[Mesh], axes: &dyn Fn(&Mesh) -> Result<(Axis, Axis, f64), String>, load: Option<(f64, f64)>, material: &dyn Fn(&str) -> Option<String>) -> Result<Vec<MeshReport>, String> {
    let mut out = Vec::new();
    for g in list {
        let (ga, gb, input) = axes(g)?;
        let (pa, pb) = (ga.at, gb.at);
        // Distance between the two (parallel) axes.
        let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let n = ga.axis;
        let along = d[0] * n[0] + d[1] * n[1] + d[2] * n[2];
        let a_w = ((d[0] - along * n[0]).powi(2) + (d[1] - along * n[1]).powi(2) + (d[2] - along * n[2]).powi(2)).sqrt();
        let alpha = g.pressure_angle.to_radians();
        let (r1, r2) = (g.module * g.teeth[0] as f64 / 2.0, g.module * g.teeth[1] as f64 / 2.0);
        let a0 = if g.internal { r2 - r1 } else { r1 + r2 };
        let mut notes = Vec::new();
        let alpha_w = (a0 * alpha.cos() / a_w).clamp(-1.0, 1.0).acos();
        let (rb1, rb2) = (r1 * alpha.cos(), r2 * alpha.cos());
        let (ra1, ra2) = (r1 + g.module, if g.internal { r2 - g.module } else { r2 + g.module });
        let pb = PI * g.module * alpha.cos();
        let path = if g.internal {
            (ra1 * ra1 - rb1 * rb1).sqrt() - (ra2 * ra2 - rb2 * rb2).max(0.0).sqrt() + a_w * alpha_w.sin()
        } else {
            (ra1 * ra1 - rb1 * rb1).sqrt() + (ra2 * ra2 - rb2 * rb2).sqrt() - a_w * alpha_w.sin()
        };
        let contact_ratio = path / pb;
        if (a_w - a0).abs() > 0.5 * g.module {
            notes.push(format!("the axes are {a_w:.2} mm apart, {:.2} mm from these gears' standard {a0:.2} mm", a_w - a0));
        }
        if contact_ratio < 1.0 {
            notes.push("contact ratio below 1: the teeth lose contact and the drive jerks or slips".into());
        } else if contact_ratio < 1.2 {
            notes.push("contact ratio below 1.2: rough running; more teeth or a smaller spread between the axes helps".into());
        }
        let backlash = g.backlash + 2.0 * (a_w - a0) * alpha.tan();
        let strength = match load {
            None => None,
            Some((torque_in, rpm_in)) => {
                // Seen from what carries the gears' axes (a planetary's carrier), the
                // train is an ordinary one: the power the drive puts in there flows
                // through each mesh, shared by identical meshes side by side (the
                // planets). No losses assumed.
                let frame = if ga.carrier != 0.0 { ga.carrier } else { gb.carrier };
                let shared = list.iter().filter(|h| h.teeth == g.teeth && h.module == g.module && h.internal == g.internal && (h.driver == g.driver || h.driven == g.driven)).count().max(1);
                if shared > 1 {
                    notes.push(format!("the load is taken as shared equally by the {shared} identical meshes"));
                }
                let s = (ga.speed - frame).abs().max(1e-12);
                let torque = torque_in * (input - frame).abs() / (s * shared as f64);
                let rpm = rpm_in * s;
                let wt = torque * 1000.0 / r1; // N (torque in N·m, radius in mm)
                let v = rpm * 2.0 * PI / 60.0 * r1 / 1000.0; // m/s
                let kv = (6.1 + v) / 6.1; // Barth, cut or printed teeth
                let bending = [kv * wt / (g.width * g.module * lewis_y(g.teeth[0])), kv * wt / (g.width * g.module * lewis_y(g.teeth[1]))];
                let mats = [material(&g.driver).unwrap_or_default(), material(&g.driven).unwrap_or_default()];
                let pr = [props(&mats[0]), props(&mats[1])];
                for (k, name) in [&g.driver, &g.driven].iter().enumerate() {
                    if pr[k].is_none() {
                        notes.push(format!("no strength figures for {}'s material ({}): give it steel, aluminium, brass, acetal, nylon, PLA, PETG or ABS", name, if mats[k].is_empty() { "none set" } else { &mats[k] }));
                    }
                }
                let bending_safety = [0, 1].map(|k| pr[k].map(|(p, _)| p.bending / bending[k]));
                let (contact, contact_safety) = match (pr[0], pr[1]) {
                    (Some((p1, _)), Some((p2, _))) => {
                        // Hertz at the pitch point (Shigley 14-13), radii of curvature r·sin α.
                        let (c1, c2) = (r1 * alpha_w.sin(), r2 * alpha_w.sin());
                        let curv = if g.internal { 1.0 / c1 - 1.0 / c2 } else { 1.0 / c1 + 1.0 / c2 };
                        let elastic = PI * ((1.0 - p1.nu * p1.nu) / p1.e + (1.0 - p2.nu * p2.nu) / p2.e);
                        let sc = (kv * wt / (g.width * alpha_w.cos()) * curv / elastic).sqrt();
                        (Some(sc), Some(p1.contact.min(p2.contact) / sc))
                    }
                    _ => (None, None),
                };
                if let Some(sf) = bending_safety.iter().flatten().copied().reduce(f64::min)
                    && sf < 1.5
                {
                    notes.push(format!("tooth bending safety factor {sf:.2}: below 1.5 teeth may break; widen the face or use a bigger module"));
                }
                if let Some(sf) = contact_safety
                    && sf < 1.2
                {
                    notes.push(format!("contact safety factor {sf:.2}: the tooth faces may wear or pit"));
                }
                let names = pr.map(|p| p.map(|(_, n)| n.to_string()).unwrap_or_default());
                Some(Strength {
                    torque: (torque * 1e4).round() / 1e4,
                    rpm: (rpm * 100.0).round() / 100.0,
                    tangential_force: (wt * 100.0).round() / 100.0,
                    pitch_speed: (v * 1000.0).round() / 1000.0,
                    velocity_factor: (kv * 1000.0).round() / 1000.0,
                    bending: bending.map(|s| (s * 100.0).round() / 100.0),
                    bending_safety: bending_safety.map(|s| s.map(|s| (s * 100.0).round() / 100.0)),
                    contact: contact.map(|s| (s * 100.0).round() / 100.0),
                    contact_safety: contact_safety.map(|s| (s * 100.0).round() / 100.0),
                    materials: names,
                })
            }
        };
        out.push(MeshReport {
            driver: g.driver.clone(),
            driven: g.driven.clone(),
            teeth: g.teeth,
            module: g.module,
            ratio: (g.teeth[1] as f64 / g.teeth[0] as f64 * 1000.0).round() / 1000.0,
            contact_ratio: (contact_ratio * 1000.0).round() / 1000.0,
            center_distance: (a_w * 1000.0).round() / 1000.0,
            standard_center_distance: (a0 * 1000.0).round() / 1000.0,
            backlash: (backlash * 1000.0).round() / 1000.0,
            strength,
            notes,
        });
    }
    Ok(out)
}

/// Turns a body makes about `axis` over the frames (None: the ground, which stays).
fn spin(frames: &[mechanism::Frame], body: Option<usize>, axis: [f64; 3]) -> f64 {
    let Some(b) = body else { return 0.0 };
    let mut total = 0.0;
    for w in frames.windows(2) {
        // The turn from one frame to the next: q1 · q0⁻¹.
        let ([x0, y0, z0, w0], [x1, y1, z1, w1]) = (w[0].bodies[b].1, w[1].bodies[b].1);
        let (bx, by, bz, bw) = (-x0, -y0, -z0, w0);
        let x = w1 * bx + x1 * bw + y1 * bz - z1 * by;
        let y = w1 * by - x1 * bz + y1 * bw + z1 * bx;
        let z = w1 * bz + x1 * by - y1 * bx + z1 * bw;
        let s = w1 * bw - x1 * bx - y1 * by - z1 * bz;
        let along = x * axis[0] + y * axis[1] + z * axis[2];
        let (along, s) = if s < 0.0 { (-along, -s) } else { (along, s) };
        total += 2.0 * along.atan2(s);
    }
    total / (2.0 * PI)
}

/// The joint a body turns on, and what is on its other side (None: the
/// ground), preferring the one to the ground (a carrier's bearing, not the
/// pins its planets turn on).
fn pivot(l: &Linkage, b: usize) -> Option<(&mechanism::Joint, Option<usize>)> {
    let turning = l.joints.iter().filter(|j| matches!(j.kind, Kind::Hinge | Kind::Cylinder));
    let sides: Vec<_> = turning.filter_map(|j| if j.a == Some(b) { Some((j, j.b)) } else if j.b == Some(b) { Some((j, j.a)) } else { None }).collect();
    sides.iter().find(|(_, o)| o.is_none()).or(sides.first()).copied()
}

/// A gear's axis and speeds in a linkage. A gear on no joint stays put (a
/// ring gear in its housing) and turns about the axis its partner is carried
/// round.
fn joint_axis(l: &Linkage, frames: &[mechanism::Frame], part: &str, partner: &str) -> Result<Axis, String> {
    match l.body_of(part) {
        Some(b) => {
            let (j, other) = pivot(l, b).ok_or_else(|| format!("“{part}” is in a mesh but turns on no hinge joint"))?;
            Ok(Axis { at: j.at, axis: j.axis, speed: spin(frames, Some(b), j.axis), carrier: spin(frames, other, j.axis) })
        }
        None => {
            let carrier = l.body_of(partner).and_then(|p| pivot(l, p)).and_then(|(_, o)| o).and_then(|c| pivot(l, c));
            let (j, _) = carrier.ok_or_else(|| format!("“{part}” stays put and “{partner}” turns about a fixed axis, so the two cannot turn in mesh"))?;
            Ok(Axis { at: j.at, axis: j.axis, speed: 0.0, carrier: 0.0 })
        }
    }
}

/// The linkage turned through one full turn of its drive (a slider drive
/// through its travel), in `steps` steps.
fn turn_joints(l: &Linkage, size: f64, steps: usize) -> Result<Vec<mechanism::Frame>, String> {
    let mut l = l.clone();
    let joint = l.drive.as_ref().ok_or("the joints have no drive: drive(…) the joint that turns the gears")?.joint;
    if l.joints[joint].kind != Kind::Slider
        && let Some(d) = l.drive.as_mut()
    {
        d.to = d.from + 360.0;
    }
    let s = l.sweep(steps, size);
    if s.frames.len() < steps + 1 {
        return Err(format!("the gears cannot turn on their joints: {}", s.notes.join(" ")));
    }
    Ok(s.frames)
}

/// The whole check: sweep plus every mesh.
#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub sweep: Sweep,
    pub meshes: Vec<MeshReport>,
    /// The overall ratio from the driving body to each other body (turns per turn).
    pub speeds: Vec<(Vec<String>, f64)>,
}

/// Runs the check on a shape evaluated at display accuracy. `finer`, if
/// given, evaluates it again much more finely, and every pair that came
/// close is measured there where it came closest, so the gaps reported are
/// the ones drawn, not the display facets' (a running fit tighter than the
/// facets is not mistaken for a clash either).
pub fn check(e: &Evaluated, load: Option<(f64, f64)>, finer: Option<&dyn Fn() -> Result<Evaluated, String>>) -> Result<Check, String> {
    check_at(e, load, finer, (0.0, 0.0))
}

/// As [`check`], knowing the accuracy of the two evaluations (mm), to report the check's resolution.
///
/// The gears turn as their hinges say (`hinge`: fixed axes at the meshes'
/// ratios) or, when the program joins them with joints instead (`joint`,
/// `drive`, `couple`: planets on a turning carrier, a ring that stays put),
/// exactly as the joints move them.
pub fn check_at(e: &Evaluated, load: Option<(f64, f64)>, finer: Option<&dyn Fn() -> Result<Evaluated, String>>, tolerances: (f64, f64)) -> Result<Check, String> {
    check_with(e, load, finer, tolerances, &|m| m)
}

/// As [`check_at`], each part's material (its name in the program, or None)
/// read through `material`: a shape's material table key to the material it stands for.
pub fn check_with(e: &Evaluated, load: Option<(f64, f64)>, finer: Option<&dyn Fn() -> Result<Evaluated, String>>, tolerances: (f64, f64), material: &dyn Fn(Option<String>) -> Option<String>) -> Result<Check, String> {
    let raw = e.mechanism.as_ref().ok_or("the program declares no mechanism: use hinge(…) and mesh(…), or joint(…), drive(…) and mesh(…)")?;
    let m = Mechanism::parse(raw)?;
    let hinged = |p: &str| m.body_of(p).is_some();
    let by_joints = if m.meshes.is_empty() { m.bodies.is_empty() } else { !m.meshes.iter().any(|g| hinged(&g.driver) || hinged(&g.driven)) };
    let linkage = if by_joints { Linkage::parse(raw).transpose()? } else { None };
    let material = |part: &str| e.parts.iter().find(|p| p.name == part).and_then(|p| material(p.material.clone()));
    let (train, meshes, speeds) = match &linkage {
        None => {
            let train = Train::of_hinges(&m)?;
            let (_, speed) = m.speeds()?;
            let speeds = m.bodies.iter().zip(speed).map(|(b, s)| (b.parts.clone(), (s * 1e6).round() / 1e6)).collect();
            (train, meshes(&m, load, &material)?, speeds)
        }
        Some(l) => {
            let size = (0..3).map(|k| e.bounds_max[k] - e.bounds_min[k]).fold(0.0, f64::max);
            let axes = |frames: &[mechanism::Frame], g: &Mesh| -> Result<(Axis, Axis, f64), String> {
                let a = joint_axis(l, frames, &g.driver, &g.driven)?;
                let b = joint_axis(l, frames, &g.driven, &g.driver)?;
                let input = l.drive.as_ref().map_or(1.0, |d| spin(frames, driven_side(&l.joints[d.joint]), a.axis));
                Ok((a, b, input))
            };
            // A first pass to see how fast the teeth pass, then fine enough that each passes in 16 steps or more.
            let rough = turn_joints(l, size, 72)?;
            let mut passes = 0.0_f64;
            for g in &m.meshes {
                let (a, b, _) = axes(&rough, g)?;
                let frame = if a.carrier != 0.0 { a.carrier } else { b.carrier };
                passes = passes.max((a.speed - frame).abs() * g.teeth[0] as f64);
            }
            let steps = ((passes * 16.0).ceil() as usize).clamp(72, 1440);
            let frames = if steps > 72 { turn_joints(l, size, steps)? } else { rough };
            let reports = mesh_reports(&m.meshes, &|g| axes(&frames, g), load, &material)?;
            let speeds = (0..l.bodies.len())
                .map(|b| {
                    let axis = pivot(l, b).map_or([0.0, 0.0, 1.0], |(j, _)| j.axis);
                    (l.bodies[b].clone(), (spin(&frames, Some(b), axis) * 1e6).round() / 1e6)
                })
                .collect();
            (Train::of_joints(l, &frames), reports, speeds)
        }
    };
    let mut sweep = sweep_train(e, &train, 1.0)?;
    sweep.resolution = 2.0 * tolerances.0;
    if !sweep.pairs.is_empty()
        && let Some(finer) = finer
    {
        sweep.resolution = 2.0 * tolerances.1;
        let fine = finer()?;
        for pair in sweep.pairs.iter_mut() {
            // The worst of the closest step and its neighbours, measured finely.
            let k = train.step_at(pair.at) as isize;
            let g = [-1, 0, 1].iter().filter_map(|d| Some(train.posed(&fine, &pair.a, k + d)?.min_gap(&train.posed(&fine, &pair.b, k + d)?, 1.0))).fold(f64::INFINITY, f64::min);
            if g.is_finite() {
                pair.gap = (g * 1e4).round() / 1e4;
            }
        }
        sweep.pairs.sort_by(|a, b| a.gap.total_cmp(&b.gap));
        sweep.clashes = sweep.pairs.iter().filter(|p| p.gap < 1e-4).cloned().collect();
    }
    Ok(Check { sweep, meshes, speeds })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_ratio_of_standard_gears() {
        // 20 and 40 teeth, module 2, 20°, at the standard centre distance: ε ≈ 1.635.
        let m = Mechanism {
            bodies: vec![Body { parts: vec!["P".into()], at: [0.0; 3], axis: [0.0, 0.0, 1.0] }, Body { parts: vec!["G".into()], at: [60.0, 0.0, 0.0], axis: [0.0, 0.0, 1.0] }],
            meshes: vec![Mesh { driver: "P".into(), driven: "G".into(), module: 2.0, teeth: [20, 40], pressure_angle: 20.0, width: 10.0, backlash: 0.0, internal: false }],
        };
        let r = meshes(&m, None, &|_| None).unwrap();
        assert!((r[0].contact_ratio - 1.635).abs() < 0.002, "{}", r[0].contact_ratio);
        assert_eq!(r[0].ratio, 2.0);
        let (_, speed) = m.speeds().unwrap();
        assert_eq!(speed, vec![1.0, -0.5], "the wheel turns half as fast, the other way");
    }

    #[test]
    fn lewis_bending_and_hertz_contact_by_hand() {
        // Steel pinion 18 teeth, steel wheel 54, module 2, face 20 mm, 50 N·m at 300 rpm.
        let m = Mechanism {
            bodies: vec![Body { parts: vec!["P".into()], at: [0.0; 3], axis: [0.0, 0.0, 1.0] }, Body { parts: vec!["G".into()], at: [72.0, 0.0, 0.0], axis: [0.0, 0.0, 1.0] }],
            meshes: vec![Mesh { driver: "P".into(), driven: "G".into(), module: 2.0, teeth: [18, 54], pressure_angle: 20.0, width: 20.0, backlash: 0.0, internal: false }],
        };
        let r = meshes(&m, Some((50.0, 300.0)), &|_| Some("Steel".into())).unwrap();
        let s = r[0].strength.as_ref().unwrap();
        // By hand: r1 = 18 mm, Wt = 50000/18 = 2777.8 N, V = 300·2π/60·0.018 = 0.5655 m/s,
        // Kv = (6.1 + 0.5655)/6.1 = 1.0927, σ = 1.0927·2777.8/(20·2·0.309) = 245.6 MPa.
        assert!((s.tangential_force - 2777.78).abs() < 0.01, "{}", s.tangential_force);
        assert!((s.velocity_factor - 1.093).abs() < 0.001);
        assert!((s.bending[0] - 245.6).abs() < 0.2, "{}", s.bending[0]);
        // Hertz: ρ1 = 18 sin20° = 6.156, ρ2 = 54 sin20° = 18.469;
        // σc = √(1.0927·2777.8/(20 cos20°) · (1/6.156 + 1/18.469) / (π·2·(1−0.09)/200000)) = 1106.1 MPa.
        let c = s.contact.unwrap();
        assert!((c - 1106.1).abs() < 0.2, "{c}");
        assert!(s.bending_safety[0].unwrap() < 1.0, "an overloaded pinion is flagged");
        assert!(r[0].notes.iter().any(|n| n.contains("bending")));
    }
}
