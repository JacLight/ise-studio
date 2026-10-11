//! Gear checks: does a gear train turn cleanly, and are its teeth strong enough?
//!
//! A program declares how its parts move (`hinge`) and which gears drive which
//! (`mesh`). From that:
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
            return Err("nothing is hinged: declare hinge(…) for the parts that turn".into());
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
                let a = self.body_of(&g.driver).ok_or_else(|| format!("“{}” is in a mesh but not hinged", g.driver))?;
                let b = self.body_of(&g.driven).ok_or_else(|| format!("“{}” is in a mesh but not hinged", g.driven))?;
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

/// Turns the mechanism through one full turn of its driving body.
pub fn sweep(e: &Evaluated, m: &Mechanism, search: f64) -> Result<Sweep, String> {
    let (driver, speed) = m.speeds()?;
    for b in &m.bodies {
        for p in &b.parts {
            if !e.parts.iter().any(|q| &q.name == p) {
                return Err(format!("“{p}” is hinged but the program has no part of that name"));
            }
        }
    }
    let body_of: Vec<Option<usize>> = e.parts.iter().map(|p| m.body_of(&p.name)).collect();
    // A tooth passes 16 steps or more, for every mesh.
    let passes = m
        .meshes
        .iter()
        .filter_map(|g| m.body_of(&g.driver).map(|b| speed[b].abs() * g.teeth[0] as f64))
        .fold(0.0_f64, f64::max);
    let steps = ((passes * 16.0).ceil() as usize).clamp(72, 1440);
    // Pairs that move relative to each other and can come close (boxes within `search`).
    let mut pairs = Vec::new();
    for i in 0..e.parts.len() {
        for j in i + 1..e.parts.len() {
            let (bi, bj) = (body_of[i], body_of[j]);
            let si = bi.map(|b| speed[b]).unwrap_or(0.0);
            let sj = bj.map(|b| speed[b]).unwrap_or(0.0);
            if bi == bj || (si == 0.0 && sj == 0.0) {
                continue; // they move together, or neither moves
            }
            // A body turning about its own axis keeps the same swept shape: use its swept box.
            let reach = |k: usize| {
                let p = &e.parts[k];
                let r = body_of[k]
                    .map(|b| {
                        let at = m.bodies[b].at;
                        (0..3).map(|d| (p.bounds_min[d] - at[d]).abs().max((p.bounds_max[d] - at[d]).abs())).fold(0.0, f64::max)
                    })
                    .unwrap_or(0.0);
                (p.bounds_min, p.bounds_max, r, body_of[k].map(|b| m.bodies[b].at))
            };
            let (ai, bi_, ri, ci) = reach(i);
            let (aj, bj_, rj, cj) = reach(j);
            let lo = |a: [f64; 3], r: f64, c: Option<[f64; 3]>| c.map(|c| c.map(|v| v - r * 1.75)).unwrap_or(a);
            let hi = |b: [f64; 3], r: f64, c: Option<[f64; 3]>| c.map(|c| c.map(|v| v + r * 1.75)).unwrap_or(b);
            let (li, hi_i, lj, hj) = (lo(ai, ri, ci), hi(bi_, ri, ci), lo(aj, rj, cj), hi(bj_, rj, cj));
            if (0..3).all(|d| li[d] - search <= hj[d] && lj[d] - search <= hi_i[d]) {
                pairs.push((i, j));
            }
        }
    }
    let mut closest: Vec<(f64, f64)> = vec![(f64::INFINITY, 0.0); pairs.len()];
    for step in 0..steps {
        let angle = 360.0 * step as f64 / steps as f64;
        let posed: Vec<Option<Manifold>> = (0..e.parts.len())
            .map(|k| {
                if !pairs.iter().any(|(i, j)| *i == k || *j == k) {
                    return None;
                }
                let solid = &e.parts[k].solid;
                Some(match body_of[k] {
                    Some(b) if speed[b] != 0.0 => solid.transform(&about(m.bodies[b].at, m.bodies[b].axis, angle * speed[b])),
                    _ => solid.translate(0.0, 0.0, 0.0),
                })
            })
            .collect();
        for (n, (i, j)) in pairs.iter().enumerate() {
            let (Some(a), Some(b)) = (&posed[*i], &posed[*j]) else { continue };
            let g = a.min_gap(b, search);
            if g < closest[n].0 {
                closest[n] = (g, angle);
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
    Ok(Sweep { driver: m.bodies[driver].parts.clone(), steps, pairs: out, clashes, search, resolution: 0.0 })
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

/// The geometry and (with a load) strength of every mesh. `materials` gives a
/// part's material name; `torque`/`rpm` are at the driving body.
pub fn meshes(m: &Mechanism, load: Option<(f64, f64)>, material: &dyn Fn(&str) -> Option<String>) -> Result<Vec<MeshReport>, String> {
    let (_, speed) = m.speeds()?;
    let mut out = Vec::new();
    for g in &m.meshes {
        let a = m.body_of(&g.driver).ok_or_else(|| format!("“{}” is not hinged", g.driver))?;
        let b = m.body_of(&g.driven).ok_or_else(|| format!("“{}” is not hinged", g.driven))?;
        let (pa, pb) = (m.bodies[a].at, m.bodies[b].at);
        // Distance between the two (parallel) axes.
        let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let n = m.bodies[a].axis;
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
                // Torque and speed on this mesh's driving gear (no losses assumed).
                let s = speed[a].abs().max(1e-12);
                let torque = torque_in / s;
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

/// The gap between two parts with the mechanism turned to `angle` (degrees of the driving body).
fn gap_at(e: &Evaluated, m: &Mechanism, a: &str, b: &str, angle: f64, search: f64) -> Option<f64> {
    let (_, speed) = m.speeds().ok()?;
    let pose = |name: &str| {
        let p = e.parts.iter().find(|p| p.name == name)?;
        Some(match m.body_of(name) {
            Some(k) if speed[k] != 0.0 => p.solid.transform(&about(m.bodies[k].at, m.bodies[k].axis, angle * speed[k])),
            _ => p.solid.translate(0.0, 0.0, 0.0),
        })
    };
    Some(pose(a)?.min_gap(&pose(b)?, search))
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
/// given, evaluates it again much more finely: a pair that seems to touch is
/// measured there at the angles it touched, so a running fit tighter than the
/// display facets (a shaft in its bearing) is not mistaken for a clash.
pub fn check(e: &Evaluated, load: Option<(f64, f64)>, finer: Option<&dyn Fn() -> Result<Evaluated, String>>) -> Result<Check, String> {
    check_at(e, load, finer, (0.0, 0.0))
}

/// As [`check`], knowing the accuracy of the two evaluations (mm), to report the check's resolution.
pub fn check_at(e: &Evaluated, load: Option<(f64, f64)>, finer: Option<&dyn Fn() -> Result<Evaluated, String>>, tolerances: (f64, f64)) -> Result<Check, String> {
    let m = Mechanism::parse(e.mechanism.as_ref().ok_or("the program declares no mechanism: use hinge(…) and mesh(…)")?)?;
    let material = |part: &str| e.parts.iter().find(|p| p.name == part).and_then(|p| p.material.clone());
    let mut sweep = sweep(e, &m, 1.0)?;
    sweep.resolution = 2.0 * tolerances.0;
    if !sweep.clashes.is_empty()
        && let Some(finer) = finer
    {
        sweep.resolution = 2.0 * tolerances.1;
        let fine = finer()?;
        let step = 360.0 / sweep.steps as f64;
        for pair in sweep.pairs.iter_mut().filter(|p| p.gap < 1e-4) {
            // The worst of the touching angle and its neighbours, measured finely.
            let g = [-1.0, 0.0, 1.0].iter().filter_map(|k| gap_at(&fine, &m, &pair.a, &pair.b, pair.at + k * step, 1.0)).fold(f64::INFINITY, f64::min);
            if g.is_finite() {
                pair.gap = (g * 1e4).round() / 1e4;
            }
        }
        sweep.pairs.sort_by(|a, b| a.gap.total_cmp(&b.gap));
        sweep.clashes = sweep.pairs.iter().filter(|p| p.gap < 1e-4).cloned().collect();
    }
    let meshes = meshes(&m, load, &material)?;
    let (_, speed) = m.speeds()?;
    let speeds = m.bodies.iter().zip(speed).map(|(b, s)| (b.parts.clone(), (s * 1e6).round() / 1e6)).collect();
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
