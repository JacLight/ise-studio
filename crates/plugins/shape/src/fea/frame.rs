//! Frames of beams: members along lines between joints, each with a cross
//! section, for structures that are mostly slender bars (an extrusion
//! frame, a truss, a shelf of tubes), where filling every bar with cubes
//! would take far more cubes than the answer needs.
//!
//! Each member is a 3D Euler–Bernoulli beam (12 unknowns: three movements
//! and three turns at each end) carrying axial force, bending both ways and
//! torsion; the joints are rigid. The equations are solved directly
//! (Cholesky), so the answer is exact for the beam theory. Results: each
//! joint's movement, each member's end forces, its largest stress (axial
//! plus bending, at the extreme fibre), its safety against yield and, when
//! it is pushed, against Euler buckling between its joints.
//!
//! Units: mm, N, MPa.

use super::materials::{self, MaterialProps};
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

/// A member's cross section. Local axes: x along the member, y and z across
/// it (y towards `up` crossed over, see `Member::up`): `w` along y, `h` along z.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum SectionSpec {
    /// Solid rectangle.
    Rect { w: f64, h: f64 },
    /// Solid round bar.
    Round { d: f64 },
    /// Round tube: outside diameter and wall.
    Tube { d: f64, t: f64 },
    /// Rectangular tube: outside sizes and wall.
    Box { w: f64, h: f64, t: f64 },
    /// A T-slot extrusion by name ("2020"): typical catalogue figures.
    Extrusion { name: String },
    /// Given outright: area (mm²), second moments about local y and z (mm⁴),
    /// torsion constant (mm⁴), and the farthest fibre from each axis (mm).
    Custom { a: f64, iy: f64, iz: f64, j: f64, cy: f64, cz: f64 },
}

/// A section's figures.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SectionProps {
    pub a: f64,
    /// Second moments (mm⁴) about local y (bending in z) and z (bending in y).
    pub iy: f64,
    pub iz: f64,
    pub j: f64,
    /// Farthest fibre along z (for bending about y) and along y (mm).
    pub cz: f64,
    pub cy: f64,
}

impl SectionSpec {
    pub fn props(&self) -> Result<SectionProps, String> {
        let pi = std::f64::consts::PI;
        let p = match *self {
            SectionSpec::Rect { w, h } => {
                let (a, b) = (w.max(h), w.min(h));
                // Saint-Venant's torsion constant of a rectangle (Roark, table 10.7).
                let j = a * b.powi(3) * (1.0 / 3.0 - 0.21 * b / a * (1.0 - b.powi(4) / (12.0 * a.powi(4))));
                SectionProps { a: w * h, iy: w * h.powi(3) / 12.0, iz: h * w.powi(3) / 12.0, j, cz: h / 2.0, cy: w / 2.0 }
            }
            SectionSpec::Round { d } => {
                let i = pi * d.powi(4) / 64.0;
                SectionProps { a: pi * d * d / 4.0, iy: i, iz: i, j: 2.0 * i, cz: d / 2.0, cy: d / 2.0 }
            }
            SectionSpec::Tube { d, t } => {
                let di = d - 2.0 * t;
                if di < 0.0 || t <= 0.0 {
                    return Err("a tube's wall must be under half its diameter".into());
                }
                let i = pi * (d.powi(4) - di.powi(4)) / 64.0;
                SectionProps { a: pi * (d * d - di * di) / 4.0, iy: i, iz: i, j: 2.0 * i, cz: d / 2.0, cy: d / 2.0 }
            }
            SectionSpec::Box { w, h, t } => {
                let (wi, hi) = (w - 2.0 * t, h - 2.0 * t);
                if wi < 0.0 || hi < 0.0 || t <= 0.0 {
                    return Err("a box's wall must be under half its sizes".into());
                }
                // Bredt: J = 4·Am²·t / perimeter of the mid-wall.
                let (wm, hm) = (w - t, h - t);
                SectionProps { a: w * h - wi * hi, iy: (w * h.powi(3) - wi * hi.powi(3)) / 12.0, iz: (h * w.powi(3) - hi * wi.powi(3)) / 12.0, j: 4.0 * (wm * hm).powi(2) * t / (2.0 * (wm + hm)), cz: h / 2.0, cy: w / 2.0 }
            }
            SectionSpec::Extrusion { ref name } => match name.trim().to_lowercase().as_str() {
                // Typical 20 × 20 slot-6 profile (Misumi HFS5-2020, OpenBuilds V-slot 20×20 within about 10 %).
                "2020" => SectionProps { a: 160.0, iy: 6_900.0, iz: 6_900.0, j: 1_500.0, cz: 10.0, cy: 10.0 },
                // Two of those side by side: the strong way along z.
                "2040" => SectionProps { a: 300.0, iy: 46_000.0, iz: 13_800.0, j: 4_500.0, cz: 20.0, cy: 10.0 },
                other => return Err(format!("no figures for extrusion “{other}”: use 2020 or 2040, or a custom section")),
            },
            SectionSpec::Custom { a, iy, iz, j, cy, cz } => SectionProps { a, iy, iz, j, cz, cy },
        };
        if !(p.a > 0.0 && p.iy > 0.0 && p.iz > 0.0 && p.j > 0.0) {
            return Err("a section needs positive area, second moments and torsion constant".into());
        }
        Ok(p)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Member {
    /// The joints it runs between (indices into `nodes`).
    pub a: usize,
    pub b: usize,
    pub section: SectionSpec,
    /// Its material by name (typical properties).
    pub material: String,
    /// Which way its local z points, roughly (default: up, +z; along x for a vertical member).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<[f64; 3]>,
    /// A load spread along it (N/mm, global axes), besides its weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct FrameSupport {
    pub node: usize,
    /// "fixed" (all six), "pinned" (movement held, free to turn), or six
    /// flags [x, y, z, rx, ry, rz] for the held directions.
    #[serde(default = "fixed")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<[bool; 6]>,
}

fn fixed() -> String {
    "fixed".into()
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct NodeLoad {
    pub node: usize,
    #[serde(default)]
    pub force: [f64; 3],
    /// N·mm.
    #[serde(default)]
    pub moment: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Frame {
    /// Joints (mm).
    pub nodes: Vec<[f64; 3]>,
    pub members: Vec<Member>,
    pub supports: Vec<FrameSupport>,
    #[serde(default)]
    pub loads: Vec<NodeLoad>,
    /// Gravity on the members' own weight (m/s²).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemberResult {
    pub a: usize,
    pub b: usize,
    pub length: f64,
    /// End forces in the member's own axes, on the member at each end:
    /// [N, Vy, Vz, T, My, Mz] (N, N·mm); axial N pulling positive at end b.
    pub end_a: [f64; 6],
    pub end_b: [f64; 6],
    /// Axial force (pulling positive, N), largest stress (MPa), safety against yield.
    pub axial: f64,
    pub max_stress: f64,
    pub safety: f64,
    /// When pushed: the Euler load between its joints (pinned ends) and the factor on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buckling_load: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buckling_factor: Option<f64>,
    pub section: SectionProps,
    pub material: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FrameResult {
    /// Each joint's movement (mm) and turn (rad).
    pub displacements: Vec<[f64; 6]>,
    pub members: Vec<MemberResult>,
    /// Each support's reaction: force (N) and moment (N·mm).
    pub reactions: Vec<(usize, [f64; 6])>,
    pub max_displacement: f64,
    pub max_stress: f64,
    pub safety: f64,
    /// The lowest buckling factor of any pushed member.
    pub buckling: Option<f64>,
    pub mass: f64,
    pub notes: Vec<String>,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = norm(a).max(1e-300);
    a.map(|c| c / l)
}

/// The member's axes as rows: x along it, y and z across (z towards `up`).
fn axes(p: [f64; 3], q: [f64; 3], up: Option<[f64; 3]>) -> [[f64; 3]; 3] {
    let x = unit(sub(q, p));
    let mut up = up.unwrap_or([0.0, 0.0, 1.0]);
    if norm(cross(x, up)) < 1e-6 * norm(up) {
        up = [1.0, 0.0, 0.0];
    }
    let y = unit(cross(up, x));
    let z = cross(x, y);
    [x, y, z]
}

/// The 12 × 12 local stiffness of a 3D Euler–Bernoulli beam (order: u v w θx θy θz at a, then at b).
fn local_stiffness(e: f64, g: f64, s: &SectionProps, l: f64) -> [[f64; 12]; 12] {
    let mut k = [[0.0; 12]; 12];
    let mut put = |i: usize, j: usize, v: f64| {
        k[i][j] += v;
        if i != j {
            k[j][i] += v;
        }
    };
    let ea = e * s.a / l;
    put(0, 0, ea);
    put(6, 6, ea);
    put(0, 6, -ea);
    let gj = g * s.j / l;
    put(3, 3, gj);
    put(9, 9, gj);
    put(3, 9, -gj);
    // Bending in the x–y plane (about z): v and θz.
    let (a, b, c, d) = (12.0 * e * s.iz / l.powi(3), 6.0 * e * s.iz / l.powi(2), 4.0 * e * s.iz / l, 2.0 * e * s.iz / l);
    put(1, 1, a);
    put(7, 7, a);
    put(1, 7, -a);
    put(1, 5, b);
    put(1, 11, b);
    put(5, 7, -b);
    put(7, 11, -b);
    put(5, 5, c);
    put(11, 11, c);
    put(5, 11, d);
    // Bending in the x–z plane (about y): w and θy (signs turn: θy = −dw/dx).
    let (a, b, c, d) = (12.0 * e * s.iy / l.powi(3), 6.0 * e * s.iy / l.powi(2), 4.0 * e * s.iy / l, 2.0 * e * s.iy / l);
    put(2, 2, a);
    put(8, 8, a);
    put(2, 8, -a);
    put(2, 4, -b);
    put(2, 10, -b);
    put(4, 8, b);
    put(8, 10, b);
    put(4, 4, c);
    put(10, 10, c);
    put(4, 10, d);
    k
}

/// Local fixed-end forces of a uniform load (local N/mm) along a member of length l, on the joints.
fn fixed_end(w: [f64; 3], l: f64) -> [f64; 12] {
    let mut f = [0.0; 12];
    f[0] = w[0] * l / 2.0;
    f[6] = w[0] * l / 2.0;
    f[1] = w[1] * l / 2.0;
    f[7] = w[1] * l / 2.0;
    f[5] = w[1] * l * l / 12.0;
    f[11] = -w[1] * l * l / 12.0;
    f[2] = w[2] * l / 2.0;
    f[8] = w[2] * l / 2.0;
    f[4] = -w[2] * l * l / 12.0;
    f[10] = w[2] * l * l / 12.0;
    f
}

/// Solves a frame.
pub fn solve(frame: &Frame) -> Result<FrameResult, String> {
    let n = frame.nodes.len();
    if n < 2 || frame.members.is_empty() {
        return Err("a frame needs at least two joints and a member".into());
    }
    if n > 2000 {
        return Err("a frame of more than 2000 joints is beyond this solver".into());
    }
    if frame.supports.is_empty() {
        return Err("hold the frame somewhere: add a support".into());
    }
    let dofs = 6 * n;
    let mut k = vec![0.0; dofs * dofs];
    let mut f = vec![0.0; dofs];
    let mut notes = vec![];
    let mut mass = 0.0;
    struct Prepared {
        t: [[f64; 3]; 3],
        l: f64,
        s: SectionProps,
        m: MaterialProps,
        kl: [[f64; 12]; 12],
        fe: [f64; 12],
    }
    let mut prepared = vec![];
    for (i, mb) in frame.members.iter().enumerate() {
        if mb.a >= n || mb.b >= n || mb.a == mb.b {
            return Err(format!("member {} joins joints that are not in the frame", i + 1));
        }
        let (p, q) = (frame.nodes[mb.a], frame.nodes[mb.b]);
        let l = norm(sub(q, p));
        if l <= 0.0 {
            return Err(format!("member {} has no length", i + 1));
        }
        let s = mb.section.props().map_err(|e| format!("member {}: {e}", i + 1))?;
        let m = materials::lookup(&mb.material).ok_or_else(|| format!("member {}: no figures for “{}”", i + 1, mb.material))?;
        let g = m.e / (2.0 * (1.0 + m.nu));
        let t = axes(p, q, mb.up);
        let kl = local_stiffness(m.e, g, &s, l);
        // Spread load: given plus weight (ρ·A·g, N/mm), into the member's axes.
        let mut w = mb.load.unwrap_or([0.0; 3]);
        mass += m.density * s.a * l * 1e-9;
        if let Some(gr) = frame.gravity {
            for d in 0..3 {
                w[d] += m.density * 1e-9 * s.a * gr[d];
            }
        }
        let wl: [f64; 3] = std::array::from_fn(|r| (0..3).map(|c| t[r][c] * w[c]).sum());
        let fe = fixed_end(wl, l);
        // Global: K = Tᵀ·k·T, f += Tᵀ·fe.
        let at = |dof: usize| if dof < 6 { 6 * mb.a + dof } else { 6 * mb.b + dof - 6 };
        let tg = |r: usize, c: usize| -> f64 {
            // The 12 × 12 block-diagonal rotation.
            if r / 3 != c / 3 { 0.0 } else { t[r % 3][c % 3] }
        };
        let mut kt = [[0.0; 12]; 12];
        for r in 0..12 {
            for c in 0..12 {
                let base = (c / 3) * 3;
                kt[r][c] = (0..3).map(|q| kl[r][base + q] * tg(base + q, c)).sum();
            }
        }
        for r in 0..12 {
            for c in 0..12 {
                let base = (r / 3) * 3;
                let v: f64 = (0..3).map(|q| tg(base + q, r) * kt[base + q][c]).sum();
                k[at(r) * dofs + at(c)] += v;
            }
            let base = (r / 3) * 3;
            f[at(r)] += (0..3).map(|q| tg(base + q, r) * fe[base + q]).sum::<f64>();
        }
        prepared.push(Prepared { t, l, s, m, kl, fe });
    }
    for ld in &frame.loads {
        if ld.node >= n {
            return Err(format!("a load is on joint {}, which is not in the frame", ld.node));
        }
        for d in 0..3 {
            f[6 * ld.node + d] += ld.force[d];
            f[6 * ld.node + 3 + d] += ld.moment[d];
        }
    }
    let mut held = vec![false; dofs];
    for sp in &frame.supports {
        if sp.node >= n {
            return Err(format!("a support is on joint {}, which is not in the frame", sp.node));
        }
        let flags = sp.hold.unwrap_or(match sp.kind.as_str() {
            "pinned" => [true, true, true, false, false, false],
            _ => [true; 6],
        });
        for d in 0..6 {
            held[6 * sp.node + d] |= flags[d];
        }
    }
    // A joint no member turns (every member meets it pinned in effect) would leave a turn free: none here, all rigid.
    let free: Vec<usize> = (0..dofs).filter(|&d| !held[d]).collect();
    let m = free.len();
    let mut a = vec![0.0; m * m];
    for (i, &r) in free.iter().enumerate() {
        for (j, &c) in free.iter().enumerate() {
            a[i * m + j] = k[r * dofs + c];
        }
    }
    let b: Vec<f64> = free.iter().map(|&r| f[r]).collect();
    let x = cholesky_solve(&mut a, &b, m).ok_or("the frame can move freely somewhere: hold it so it cannot slide or spin (or a joint turns freely)")?;
    let mut u = vec![0.0; dofs];
    for (i, &r) in free.iter().enumerate() {
        u[r] = x[i];
    }
    // Reactions: K·u − f at held directions.
    let mut reactions = vec![];
    for sp in &frame.supports {
        let mut r = [0.0; 6];
        for d in 0..6 {
            let row = 6 * sp.node + d;
            if held[row] {
                r[d] = (0..dofs).map(|c| k[row * dofs + c] * u[c]).sum::<f64>() - f[row];
            }
        }
        reactions.push((sp.node, r));
    }
    let mut members = vec![];
    let (mut max_stress, mut safety, mut buckling): (f64, f64, Option<f64>) = (0.0, f64::INFINITY, None);
    for (mb, p) in frame.members.iter().zip(&prepared) {
        let ug: [f64; 12] = std::array::from_fn(|d| if d < 6 { u[6 * mb.a + d] } else { u[6 * mb.b + d - 6] });
        let ul: [f64; 12] = std::array::from_fn(|r| {
            let base = (r / 3) * 3;
            (0..3).map(|q| p.t[r % 3][q] * ug[base + q]).sum()
        });
        // On the member: k·u − fixed-end loads.
        let end: [f64; 12] = std::array::from_fn(|r| (0..12).map(|c| p.kl[r][c] * ul[c]).sum::<f64>() - p.fe[r]);
        let end_a: [f64; 6] = std::array::from_fn(|d| end[d]);
        let end_b: [f64; 6] = std::array::from_fn(|d| end[6 + d]);
        let axial = end_b[0];
        // The extreme fibre at either end (and, under a spread load, its middle).
        let fibre = |my: f64, mz: f64| axial.abs() / p.s.a + my.abs() * p.s.cz / p.s.iy + mz.abs() * p.s.cy / p.s.iz;
        let mid_my = (end_b[4] - end_a[4]) / 2.0 + p.fe[2] * p.l / 4.0;
        let mid_mz = (end_b[5] - end_a[5]) / 2.0 - p.fe[1] * p.l / 4.0;
        let stress = fibre(end_a[4], end_a[5]).max(fibre(end_b[4], end_b[5])).max(fibre(mid_my, mid_mz));
        let sf = if stress > 0.0 { p.m.yield_strength / stress } else { f64::INFINITY };
        let (bl, bf) = if axial < 0.0 {
            let pcr = std::f64::consts::PI.powi(2) * p.m.e * p.s.iy.min(p.s.iz) / (p.l * p.l);
            (Some(pcr), Some(pcr / -axial))
        } else {
            (None, None)
        };
        if let Some(fct) = bf {
            buckling = Some(buckling.map_or(fct, |b: f64| b.min(fct)));
        }
        max_stress = max_stress.max(stress);
        safety = safety.min(sf);
        members.push(MemberResult { a: mb.a, b: mb.b, length: p.l, end_a, end_b, axial, max_stress: stress, safety: sf, buckling_load: bl, buckling_factor: bf, section: p.s, material: p.m.name.clone() });
    }
    let displacements: Vec<[f64; 6]> = (0..n).map(|i| std::array::from_fn(|d| u[6 * i + d])).collect();
    let max_displacement = displacements.iter().map(|d| norm([d[0], d[1], d[2]])).fold(0.0, f64::max);
    notes.push(format!("{} members, {} joints; Euler–Bernoulli beams with rigid joints; buckling per member between its joints (pinned ends, a conservative length for braced frames).", frame.members.len(), n));
    if buckling.is_some_and(|b| b < 3.0) {
        notes.push("A pushed member buckles at less than 3 × its load: brace it or use a stiffer section.".into());
    }
    Ok(FrameResult { displacements, members, reactions, max_displacement, max_stress, safety, buckling, mass, notes })
}

/// Solves A·x = b (A symmetric positive definite, n × n, overwritten) by Cholesky.
fn cholesky_solve(a: &mut [f64], b: &[f64], n: usize) -> Option<Vec<f64>> {
    let scale = (0..n).map(|i| a[i * n + i].abs()).fold(0.0, f64::max).max(1e-300);
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        if d <= 1e-12 * scale {
            return None;
        }
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        y[i] = (b[i] - (0..i).map(|k| a[i * n + k] * y[k]).sum::<f64>()) / a[i * n + i];
    }
    for i in (0..n).rev() {
        y[i] = (y[i] - (i + 1..n).map(|k| a[k * n + i] * y[k]).sum::<f64>()) / a[i * n + i];
    }
    Some(y)
}
