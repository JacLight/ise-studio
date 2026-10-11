//! Exact motion of linkages: bodies joined by hinges, sliders, cylinders,
//! balls and fixed joints, driven by one joint, with gear and rack couplings.
//!
//! Each moving body has a position and a rotation; every joint is a set of
//! equations between two bodies (or a body and the ground). For each step of
//! the driving joint the equations are solved by damped Gauss–Newton from the
//! previous step's poses, so a crank turning a connecting rod pushes the
//! piston exactly where the geometry puts it. The degrees of freedom are
//! counted from the equations, and a step that cannot be solved is reported
//! as the place the mechanism binds.

use serde::Serialize;
use serde_json::Value;

pub type V3 = [f64; 3];
type M3 = [[f64; 3]; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Turns about an axis (a pin in a hole).
    Hinge,
    /// Slides along an axis without turning.
    Slider,
    /// Slides along and turns about an axis.
    Cylinder,
    /// Turns any way about a point.
    Ball,
    /// Does not move relative to the other.
    Fixed,
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub kind: Kind,
    /// Bodies (None: the ground).
    pub a: Option<usize>,
    pub b: Option<usize>,
    /// Where and along what, as drawn (mm, unit vector).
    pub at: V3,
    pub axis: V3,
}

#[derive(Clone, Debug)]
pub struct Drive {
    pub joint: usize,
    /// Degrees for a hinge or cylinder (its turn), mm for a slider.
    pub from: f64,
    pub to: f64,
    /// Turns per minute (hinges) or mm per second (sliders), for speeds.
    pub speed: f64,
}

/// One joint's coordinate held at a multiple of another's (gears, racks).
#[derive(Clone, Debug)]
pub struct Coupling {
    pub from: usize,
    pub to: usize,
    pub ratio: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Linkage {
    /// The parts of each moving body.
    pub bodies: Vec<Vec<String>>,
    pub joints: Vec<Joint>,
    pub drive: Option<Drive>,
    pub couplings: Vec<Coupling>,
}

fn v3(v: &Value) -> Option<V3> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

impl Linkage {
    /// The linkage a program declared with joint(), if any.
    pub fn parse(v: &Value) -> Option<Result<Linkage, String>> {
        let joints = v.get("joints")?.as_array()?;
        if joints.is_empty() {
            return None;
        }
        Some(Self::parse_joints(v, joints))
    }

    fn parse_joints(v: &Value, joints: &[Value]) -> Result<Linkage, String> {
        let mut l = Linkage::default();
        // Bodies: parts listed together move together; "ground" is everything still.
        let body_of = |l: &mut Linkage, parts: &Value| -> Result<Option<usize>, String> {
            let names: Vec<String> = match parts {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect(),
                _ => return Err("a joint names its parts, or \"ground\"".into()),
            };
            if names.iter().any(|n| n == "ground") {
                return Ok(None);
            }
            if let Some(i) = l.bodies.iter().position(|b| b.iter().any(|p| names.contains(p))) {
                for n in names {
                    if !l.bodies[i].contains(&n) {
                        l.bodies[i].push(n);
                    }
                }
                return Ok(Some(i));
            }
            l.bodies.push(names);
            Ok(Some(l.bodies.len() - 1))
        };
        for (i, j) in joints.iter().enumerate() {
            let kind = match j["type"].as_str().unwrap_or("") {
                "hinge" => Kind::Hinge,
                "slider" => Kind::Slider,
                "cylinder" => Kind::Cylinder,
                "ball" => Kind::Ball,
                "fixed" => Kind::Fixed,
                t => return Err(format!("unknown joint “{t}” (hinge, slider, cylinder, ball or fixed)")),
            };
            let a = body_of(&mut l, &j["a"])?;
            let b = body_of(&mut l, &j["b"])?;
            if a == b {
                return Err(format!("joint {} joins a body to itself", i + 1));
            }
            let axis = v3(&j["axis"]).unwrap_or([0.0, 0.0, 1.0]);
            let n = norm(axis);
            if n < 1e-12 {
                return Err("a joint axis needs a direction".into());
            }
            l.joints.push(Joint { name: j["name"].as_str().map(str::to_string).unwrap_or_else(|| format!("Joint {}", i + 1)), kind, a, b, at: v3(&j["at"]).unwrap_or([0.0; 3]), axis: scale(axis, 1.0 / n) });
        }
        let find = |l: &Linkage, name: &Value| -> Result<usize, String> {
            let n = name.as_str().ok_or("name the joint")?;
            l.joints.iter().position(|j| j.name == n).ok_or_else(|| format!("no joint called “{n}”"))
        };
        if let Some(d) = v.get("drive").filter(|d| !d.is_null()) {
            let joint = find(&l, &d["joint"])?;
            let turning = matches!(l.joints[joint].kind, Kind::Hinge | Kind::Cylinder);
            let from = d["from"].as_f64().unwrap_or(0.0);
            let to = d["to"].as_f64().unwrap_or(if turning { 360.0 } else { f64::NAN });
            if !to.is_finite() {
                return Err("a slider drive needs `to` (how far it moves, mm)".into());
            }
            l.drive = Some(Drive { joint, from, to, speed: d["speed"].as_f64().unwrap_or(60.0) });
        }
        for c in v.get("couplings").and_then(Value::as_array).into_iter().flatten() {
            l.couplings.push(Coupling { from: find(&l, &c["from"])?, to: find(&l, &c["to"])?, ratio: c["ratio"].as_f64().ok_or("a coupling needs a ratio")? });
        }
        Ok(l)
    }
}

// ---- Small vector maths --------------------------------------------------------

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn mul(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
fn matmul(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}
const I3: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The rotation by the vector `w` (its length the angle, radians).
fn exp(w: V3) -> M3 {
    let th = norm(w);
    if th < 1e-14 {
        return [[1.0, -w[2], w[1]], [w[2], 1.0, -w[0]], [-w[1], w[0], 1.0]];
    }
    let k = scale(w, 1.0 / th);
    let (s, c) = th.sin_cos();
    let t = 1.0 - c;
    [
        [c + k[0] * k[0] * t, k[0] * k[1] * t - k[2] * s, k[0] * k[2] * t + k[1] * s],
        [k[1] * k[0] * t + k[2] * s, c + k[1] * k[1] * t, k[1] * k[2] * t - k[0] * s],
        [k[2] * k[0] * t - k[1] * s, k[2] * k[1] * t + k[0] * s, c + k[2] * k[2] * t],
    ]
}

/// A rotation as a quaternion [x, y, z, w].
pub fn quaternion(m: &M3) -> [f64; 4] {
    let tr = m[0][0] + m[1][1] + m[2][2];
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [(m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s, 0.25 * s]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s, (m[2][1] - m[1][2]) / s]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[0][2] - m[2][0]) / s]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [(m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[1][0] - m[0][1]) / s]
    };
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    q.map(|c| c / n)
}

/// Two unit vectors square to `u` and to each other.
fn perpendiculars(u: V3) -> (V3, V3) {
    let pick = if u[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let a = cross(u, pick);
    let a = scale(a, 1.0 / norm(a));
    (a, cross(u, a))
}

fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    a - t * ((a + std::f64::consts::PI) / t).floor()
}

// ---- Poses and equations -------------------------------------------------------

/// Where every moving body is: a rotation and a translation from as drawn
/// (a point p drawn on the body is now at R p + t).
#[derive(Clone, Debug)]
pub struct Poses {
    pub rot: Vec<M3>,
    pub pos: Vec<V3>,
}

impl Poses {
    pub fn drawn(n: usize) -> Poses {
        Poses { rot: vec![I3; n], pos: vec![[0.0; 3]; n] }
    }
    fn r(&self, b: Option<usize>) -> M3 {
        b.map(|i| self.rot[i]).unwrap_or(I3)
    }
    fn place(&self, b: Option<usize>, p: V3) -> V3 {
        match b {
            Some(i) => add(mul(&self.rot[i], p), self.pos[i]),
            None => p,
        }
    }
    fn turn(&self, b: Option<usize>, v: V3) -> V3 {
        mul(&self.r(b), v)
    }
    /// Moved by small steps: 3 translation then 3 rotation per body.
    fn stepped(&self, d: &[f64]) -> Poses {
        let mut out = self.clone();
        for i in 0..self.rot.len() {
            out.pos[i] = add(self.pos[i], [d[6 * i], d[6 * i + 1], d[6 * i + 2]]);
            out.rot[i] = matmul(&exp([d[6 * i + 3], d[6 * i + 4], d[6 * i + 5]]), &self.rot[i]);
        }
        out
    }
}

impl Linkage {
    /// A joint's coordinate: how far its first body has turned (radians,
    /// unwrapped near `near`, anticlockwise about the axis) or slid (mm,
    /// along the axis) relative to its second.
    pub fn coordinate(&self, p: &Poses, j: usize, near: f64) -> f64 {
        let jt = &self.joints[j];
        match jt.kind {
            Kind::Slider => dot(sub(p.place(jt.a, jt.at), p.place(jt.b, jt.at)), p.turn(jt.b, jt.axis)),
            _ => {
                let (n, _) = perpendiculars(jt.axis);
                let (nb, na, u) = (p.turn(jt.b, n), p.turn(jt.a, n), p.turn(jt.b, jt.axis));
                let raw = dot(cross(nb, na), u).atan2(dot(nb, na));
                near + wrap(raw - near)
            }
        }
    }

    /// Every equation's error at these poses (lengths in mm; directions
    /// scaled by `size` so they weigh the same). `held` pins the driving
    /// joint's coordinate; `last` keeps turning coordinates continuous.
    fn residuals(&self, p: &Poses, size: f64, held: Option<f64>, last: &[f64]) -> Vec<f64> {
        let mut r = Vec::with_capacity(6 * self.bodies.len() + 2);
        for jt in &self.joints {
            let (pa, pb) = (p.place(jt.a, jt.at), p.place(jt.b, jt.at));
            let ua = p.turn(jt.a, jt.axis);
            let (n1, n2) = perpendiculars(jt.axis);
            let (na1, na2) = (p.turn(jt.a, n1), p.turn(jt.a, n2));
            let (nb1, nb2) = (p.turn(jt.b, n1), p.turn(jt.b, n2));
            let d = sub(pb, pa);
            // The shared point (hinge, ball, fixed) or the shared line (slider, cylinder).
            match jt.kind {
                Kind::Hinge | Kind::Ball | Kind::Fixed => r.extend(d),
                Kind::Slider | Kind::Cylinder => r.extend([dot(d, na1), dot(d, na2)]),
            }
            // Axes kept parallel (all but the ball).
            if jt.kind != Kind::Ball {
                r.extend([dot(ua, nb1) * size, dot(ua, nb2) * size]);
            }
            // No turning about the axis (slider, fixed).
            if matches!(jt.kind, Kind::Slider | Kind::Fixed) {
                r.push(dot(na1, nb2) * size);
            }
        }
        for c in &self.couplings {
            let unit = |j: usize| if self.joints[j].kind == Kind::Slider { 1.0 } else { size };
            let (a, b) = (self.coordinate(p, c.from, last[c.from]), self.coordinate(p, c.to, last[c.to]));
            r.push((b - c.ratio * a) * unit(c.to));
        }
        if let (Some(d), Some(v)) = (&self.drive, held) {
            let unit = if self.joints[d.joint].kind == Kind::Slider { 1.0 } else { size };
            r.push((self.coordinate(p, d.joint, last[d.joint]) - v) * unit);
        }
        r
    }

    fn jacobian(&self, p: &Poses, size: f64, held: Option<f64>, last: &[f64]) -> (Vec<f64>, Vec<Vec<f64>>) {
        let r0 = self.residuals(p, size, held, last);
        let n = 6 * self.bodies.len();
        let mut cols = vec![];
        for k in 0..n {
            // Translations step by a millionth of the size, rotations by a millionth of a radian.
            let h = if k % 6 < 3 { 1e-6 * size } else { 1e-6 };
            let mut d = vec![0.0; n];
            d[k] = h;
            let r1 = self.residuals(&p.stepped(&d), size, held, last);
            d[k] = -h;
            let r2 = self.residuals(&p.stepped(&d), size, held, last);
            cols.push(r1.iter().zip(&r2).map(|(a, b)| (a - b) / (2.0 * h)).collect::<Vec<f64>>());
        }
        (r0, cols)
    }

    /// Solves for poses near `start`; Err when no pose satisfies the joints.
    fn solve(&self, start: &Poses, size: f64, held: Option<f64>, last: &[f64]) -> Result<Poses, f64> {
        let mut p = start.clone();
        let n = 6 * self.bodies.len();
        let tol = 1e-9 * size;
        let mut lambda = 1e-6;
        let worst = |r: &[f64]| r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let mut err = worst(&self.residuals(&p, size, held, last));
        for _ in 0..80 {
            if err < tol {
                return Ok(p);
            }
            let (r, cols) = self.jacobian(&p, size, held, last);
            // Damped normal equations: (JᵀJ + λ·diag) d = −Jᵀr, scaled so steps are comparable.
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
            for _ in 0..12 {
                let mut m = a.clone();
                for (i, row) in m.iter_mut().enumerate() {
                    row[i] += lambda * (a[i][i] + 1e-12);
                }
                let Some(d) = solve_dense(m, g.clone()) else {
                    lambda *= 10.0;
                    continue;
                };
                let q = p.stepped(&d);
                let e = worst(&self.residuals(&q, size, held, last));
                if e < err {
                    p = q;
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

    /// How many ways the bodies can still move (6 per body less independent equations).
    pub fn freedom(&self, size: f64) -> usize {
        let p = Poses::drawn(self.bodies.len());
        let last = vec![0.0; self.joints.len()];
        let (_, cols) = self.jacobian(&p, size, None, &last);
        6 * self.bodies.len() - rank(&cols, 1e-7 * size.max(1.0))
    }
}

/// Gaussian elimination with partial pivoting; None when singular.
fn solve_dense(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-300 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            if f == 0.0 {
                continue;
            }
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for c in (0..n).rev() {
        x[c] = (b[c] - (c + 1..n).map(|k| a[c][k] * x[k]).sum::<f64>()) / a[c][c];
    }
    Some(x)
}

/// The rank of a matrix given by columns.
fn rank(cols: &[Vec<f64>], tol: f64) -> usize {
    if cols.is_empty() {
        return 0;
    }
    let rows = cols[0].len();
    let mut m: Vec<Vec<f64>> = (0..rows).map(|i| cols.iter().map(|c| c[i]).collect()).collect();
    let n = cols.len();
    let mut rank = 0;
    for c in 0..n {
        let Some(p) = (rank..rows).max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs())) else { break };
        if m[p][c].abs() <= tol {
            continue;
        }
        m.swap(rank, p);
        for r in 0..rows {
            if r != rank {
                let f = m[r][c] / m[rank][c];
                for k in c..n {
                    m[r][k] -= f * m[rank][k];
                }
            }
        }
        rank += 1;
        if rank == rows {
            break;
        }
    }
    rank
}

// ---- Sweeps --------------------------------------------------------------------

/// One position of the mechanism.
#[derive(Clone, Debug, Serialize)]
pub struct Frame {
    /// The driving joint's value (degrees or mm).
    pub drive: f64,
    /// Each body's pose: position (mm) and rotation quaternion [x, y, z, w],
    /// taking a point as drawn to where it is now.
    pub bodies: Vec<([f64; 3], [f64; 4])>,
    /// Every joint's value (degrees for turns, mm for slides).
    pub joints: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct JointInfo {
    pub name: String,
    pub kind: Kind,
    /// The value's unit: "°" or "mm".
    pub unit: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sweep {
    pub bodies: Vec<Vec<String>>,
    pub joints: Vec<JointInfo>,
    /// Which joint drives, its range and speed.
    pub drive: Option<usize>,
    pub speed: f64,
    /// Ways it can move before the drive holds one (1 for a working mechanism).
    pub freedom: usize,
    pub frames: Vec<Frame>,
    /// Where it stopped, if it bound before the end of the range.
    pub binds_at: Option<f64>,
    pub notes: Vec<String>,
    /// Every pair of parts that came close, closest first (at = drive value).
    pub pairs: Vec<PairGap>,
    /// Pairs that touch or overlap somewhere in the motion.
    pub clashes: Vec<PairGap>,
    /// Joint forces and drive effort at the drive's speed (when masses are known).
    pub forces: Option<Forces>,
}

/// Two parts' closest approach through the motion.
#[derive(Clone, Debug, Serialize)]
pub struct PairGap {
    pub a: String,
    pub b: String,
    /// Smallest gap (mm; 0 = touching or overlapping).
    pub gap: f64,
    /// The drive's value where it happens (degrees or mm).
    pub at: f64,
    /// The two are joined (a bearing's running clearance, not a near miss).
    pub joined: bool,
}

/// A pose as Manifold's column-major 3×4 matrix.
pub fn pose_matrix(t: [f64; 3], q: [f64; 4]) -> [f64; 12] {
    let [x, y, z, w] = q;
    let r = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    [r[0][0], r[1][0], r[2][0], r[0][1], r[1][1], r[2][1], r[0][2], r[1][2], r[2][2], t[0], t[1], t[2]]
}

impl Linkage {
    /// Whether two bodies (None: the ground) share a joint.
    pub fn joined(&self, a: Option<usize>, b: Option<usize>) -> bool {
        self.joints.iter().any(|j| (j.a == a && j.b == b) || (j.a == b && j.b == a))
    }
    /// The body a part belongs to (None: the ground).
    pub fn body_of(&self, part: &str) -> Option<usize> {
        self.bodies.iter().position(|b| b.iter().any(|p| p == part))
    }
}

impl Linkage {
    /// The mechanism through its drive's range in `steps` steps.
    pub fn sweep(&self, steps: usize, size: f64) -> Sweep {
        let size = size.max(1.0);
        let freedom = self.freedom(size);
        let joints = self.joints.iter().map(|j| JointInfo { name: j.name.clone(), kind: j.kind, unit: if j.kind == Kind::Slider { "mm".into() } else { "°".into() } }).collect();
        let mut out = Sweep {
            bodies: self.bodies.clone(),
            joints,
            drive: self.drive.as_ref().map(|d| d.joint),
            speed: self.drive.as_ref().map_or(0.0, |d| d.speed),
            freedom,
            frames: vec![],
            binds_at: None,
            notes: vec![],
            pairs: vec![],
            clashes: vec![],
            forces: None,
        };
        let mut notes = vec![];
        if freedom == 0 {
            notes.push("Nothing can move: the joints lock every part.".to_string());
        } else if self.drive.is_some() && freedom > 1 {
            notes.push(format!("{freedom} ways to move but one drive: the rest are loose (add joints or couplings to hold them)."));
        }
        let mut last = vec![0.0; self.joints.len()];
        let mut p = Poses::drawn(self.bodies.len());
        // As drawn, the joints should already hold; if not, settle them first.
        match self.solve(&p, size, None, &last) {
            Ok(q) => {
                let moved = q.pos.iter().map(|t| norm(*t)).fold(0.0, f64::max);
                if moved > 1e-6 * size {
                    notes.push(format!("The parts as drawn do not quite meet their joints; they were moved up to {moved:.3} mm to assemble."));
                }
                p = q;
            }
            Err(e) => {
                notes.push(format!("The joints cannot all hold as drawn (off by {e:.3} mm): check where each joint is."));
                out.notes = notes;
                return out;
            }
        }
        let Some(drive) = &self.drive else {
            out.frames.push(self.frame(&p, 0.0, &mut last));
            out.notes = notes;
            return out;
        };
        let turning = self.joints[drive.joint].kind != Kind::Slider;
        let to_unit = |v: f64| if turning { v.to_radians() } else { v };
        let start = self.coordinate(&p, drive.joint, 0.0);
        for k in 0..=steps {
            let value = drive.from + (drive.to - drive.from) * k as f64 / steps as f64;
            // Small sub-steps when a step is hard, so the solve follows the motion.
            let target = start + to_unit(value - drive.from);
            match self.solve(&p, size, Some(target), &last) {
                Ok(q) => {
                    p = q;
                    out.frames.push(self.frame(&p, value, &mut last));
                }
                Err(_) => {
                    let mut ok = false;
                    let prev = out.frames.last().map_or(drive.from, |f| f.drive);
                    for sub in 1..=8 {
                        let v = prev + (value - prev) * sub as f64 / 8.0;
                        match self.solve(&p, size, Some(start + to_unit(v - drive.from)), &last) {
                            Ok(q) => {
                                p = q;
                                for (i, l) in last.iter_mut().enumerate() {
                                    *l = self.coordinate(&p, i, *l);
                                }
                                ok = sub == 8;
                            }
                            Err(_) => break,
                        }
                    }
                    if ok {
                        out.frames.push(self.frame(&p, value, &mut last));
                    } else {
                        out.binds_at = Some(value);
                        notes.push(format!("It binds at {} {}: the joints cannot follow the drive further.", crate::drawing::length_text(value), if turning { "°" } else { "mm" }));
                        break;
                    }
                }
            }
        }
        out.notes = notes;
        out
    }

    fn frame(&self, p: &Poses, drive: f64, last: &mut [f64]) -> Frame {
        for (i, l) in last.iter_mut().enumerate() {
            *l = self.coordinate(p, i, *l);
        }
        Frame {
            drive,
            bodies: (0..self.bodies.len()).map(|i| (p.pos[i], quaternion(&p.rot[i]))).collect(),
            joints: last.iter().enumerate().map(|(i, v)| if self.joints[i].kind == Kind::Slider { *v } else { v.to_degrees() }).collect(),
        }
    }
}

// ---- Forces --------------------------------------------------------------------

/// A body's mass properties as drawn: mass (kg), centre of mass (mm) and
/// inertia about the centre of mass (kg·mm², drawn axes).
#[derive(Clone, Debug)]
pub struct MassProps {
    pub mass: f64,
    pub center: V3,
    pub inertia: M3,
}

impl MassProps {
    /// The sum of several bodies' mass properties.
    pub fn combine(parts: &[MassProps]) -> MassProps {
        let mass: f64 = parts.iter().map(|p| p.mass).sum();
        if mass <= 0.0 {
            return MassProps { mass: 0.0, center: [0.0; 3], inertia: [[0.0; 3]; 3] };
        }
        let center = scale(parts.iter().fold([0.0; 3], |c, p| add(c, scale(p.center, p.mass))), 1.0 / mass);
        let mut inertia = [[0.0; 3]; 3];
        for p in parts {
            // Parallel axes: I_c + m (|d|² E − d dᵀ).
            let d = sub(p.center, center);
            let dd = dot(d, d);
            for i in 0..3 {
                for j in 0..3 {
                    inertia[i][j] += p.inertia[i][j] + p.mass * (if i == j { dd } else { 0.0 } - d[i] * d[j]);
                }
            }
        }
        MassProps { mass, center, inertia }
    }
}

/// Mass properties of a closed triangle mesh (mm) of uniform density (kg/mm³).
pub fn mesh_mass(positions: &[f32], triangles: &[u32], density: f64) -> MassProps {
    let v = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    // Integrals over tetrahedra from the origin to each triangle.
    let (mut vol, mut first, mut s) = (0.0, [0.0; 3], [[0.0; 3]; 3]);
    for t in triangles.chunks_exact(3) {
        let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
        let dv = dot(a, cross(b, c)) / 6.0;
        vol += dv;
        for k in 0..3 {
            first[k] += dv * (a[k] + b[k] + c[k]) / 4.0;
        }
        for i in 0..3 {
            for j in 0..3 {
                s[i][j] += dv / 20.0 * (2.0 * (a[i] * a[j] + b[i] * b[j] + c[i] * c[j]) + a[i] * b[j] + a[j] * b[i] + a[i] * c[j] + a[j] * c[i] + b[i] * c[j] + b[j] * c[i]);
            }
        }
    }
    if vol.abs() < 1e-12 {
        return MassProps { mass: 0.0, center: [0.0; 3], inertia: [[0.0; 3]; 3] };
    }
    let mass = vol * density;
    let center = scale(first, 1.0 / vol);
    let tr = s[0][0] + s[1][1] + s[2][2];
    let mut inertia = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            // About the origin, then moved to the centre of mass.
            let about_origin = density * (if i == j { tr } else { 0.0 } - s[i][j]);
            inertia[i][j] = about_origin - mass * (if i == j { dot(center, center) } else { 0.0 } - center[i] * center[j]);
        }
    }
    MassProps { mass, center, inertia }
}

fn from_quaternion(q: [f64; 4]) -> M3 {
    let [x, y, z, w] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

fn transpose(m: &M3) -> M3 {
    [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]]
}

/// The forces through the motion at the drive's speed.
#[derive(Clone, Debug, Serialize)]
pub struct Forces {
    /// Per frame: the torque the drive needs (N·m), or its force (N) for a slider.
    pub drive: Vec<f64>,
    /// Per joint, per frame: the force it carries (N).
    pub joints: Vec<Vec<f64>>,
    /// Per frame: the force the moving parts shake the ground with (N, inertia only).
    pub shaking: Vec<f64>,
    /// The drive's average power over the range (W): about zero for a
    /// frictionless full cycle; what it takes on average otherwise.
    pub mean_power: f64,
    /// Each body's mass (kg).
    pub masses: Vec<f64>,
    pub gravity: bool,
}

impl Linkage {
    /// Inverse dynamics: the joint forces and drive effort that make the
    /// bodies follow the solved motion at the drive's speed, with gravity
    /// (−z, 9.81 m/s²) if asked. Reactions solve Jᵀλ = M·a − F through the
    /// same joint equations the motion is solved with.
    pub fn forces(&self, sweep: &Sweep, bodies: &[MassProps], size: f64, gravity: bool) -> Option<Forces> {
        let drive = self.drive.as_ref()?;
        let frames = &sweep.frames;
        let n = frames.len();
        if n < 3 || bodies.len() != self.bodies.len() {
            return None;
        }
        let size = size.max(1.0);
        let turning = self.joints[drive.joint].kind != Kind::Slider;
        let step = frames[1].drive - frames[0].drive;
        // Seconds per frame at the drive's speed (rpm → °/s is ×6).
        let dt = if turning { step / (drive.speed * 6.0) } else { step / drive.speed };
        let full = turning && sweep.binds_at.is_none() && ((frames[n - 1].drive - frames[0].drive).abs() - 360.0).abs() < 1e-6;
        // Central differences over a frame and its neighbours: round the cycle
        // when it closes, else the nearest centred triple at the ends.
        let triple = |k: usize| -> (usize, usize, usize) {
            if full {
                let m = n - 1; // the last frame repeats the first
                let k = k % m;
                ((k + m - 1) % m, k, (k + 1) % m)
            } else {
                let mid = k.clamp(1, n - 2);
                (mid - 1, mid, mid + 1)
            }
        };
        let rot: Vec<Vec<M3>> = frames.iter().map(|f| f.bodies.iter().map(|b| from_quaternion(b.1)).collect()).collect();
        let com = |k: usize, b: usize| add(mul(&rot[k][b], bodies[b].center), frames[k].bodies[b].0);
        // Angular velocity (rad/s, world axes): vee(Ṙ Rᵀ).
        let omega = |k: usize, b: usize| {
            let (i, mid, j) = triple(k);
            let mut d = [[0.0; 3]; 3];
            for r in 0..3 {
                for c in 0..3 {
                    d[r][c] = (rot[j][b][r][c] - rot[i][b][r][c]) / (2.0 * dt);
                }
            }
            let w = matmul(&d, &transpose(&rot[mid][b]));
            [(w[2][1] - w[1][2]) / 2.0, (w[0][2] - w[2][0]) / 2.0, (w[1][0] - w[0][1]) / 2.0]
        };
        let mut out = Forces { drive: vec![], joints: vec![vec![]; self.joints.len()], shaking: vec![], mean_power: 0.0, masses: bodies.iter().map(|b| b.mass).collect(), gravity };
        let g = if gravity { [0.0, 0.0, -9.81] } else { [0.0; 3] };
        for k in 0..n {
            let (i, mid, j) = triple(k);
            let p = Poses { rot: rot[k].clone(), pos: frames[k].bodies.iter().map(|b| b.0).collect() };
            let last: Vec<f64> = frames[k].joints.iter().enumerate().map(|(q, v)| if self.joints[q].kind == Kind::Slider { *v } else { v.to_radians() }).collect();
            let target = self.coordinate(&p, drive.joint, last[drive.joint]);
            let (_, cols) = self.jacobian(&p, size, Some(target), &last);
            // Generalised inertial load per body: force (N) and moment about its origin (N·mm).
            let mut qv = vec![0.0; 6 * bodies.len()];
            let mut shake = [0.0; 3];
            for (b, mp) in bodies.iter().enumerate() {
                let (ci, cm, cj, c) = (com(i, b), com(mid, b), com(j, b), com(k, b));
                let acc_mm = scale(add(sub(ci, scale(cm, 2.0)), cj), 1.0 / (dt * dt)); // mm/s²
                let acc = scale(acc_mm, 1e-3); // m/s²
                let w = omega(k, b);
                let alpha = scale(sub(omega(j, b), omega(i, b)), 1.0 / (2.0 * dt));
                let r = &rot[k][b];
                let iw = matmul(&matmul(r, &mp.inertia), &transpose(r)); // kg·mm²
                let force = scale(sub(acc, g), mp.mass); // N, what the joints must supply
                shake = add(shake, scale(acc, mp.mass));
                // Moment about the body's origin point t (N·mm): I·α + ω×Iω (kg·mm²/s² = 1e-3 N·mm) + (c − t)×F.
                let spin = add(mul(&iw, alpha), cross(w, mul(&iw, w)));
                let arm = sub(c, frames[k].bodies[b].0);
                let moment = add(scale(spin, 1e-3), cross(arm, force));
                qv[6 * b..6 * b + 3].copy_from_slice(&force);
                qv[6 * b + 3..6 * b + 6].copy_from_slice(&moment);
            }
            // Jᵀλ = q, least squares: (J Jᵀ + ε) λ = J q. Rows are the joint equations.
            let rows = cols[0].len();
            let mut a = vec![vec![0.0; rows]; rows];
            let mut rhs = vec![0.0; rows];
            for r1 in 0..rows {
                rhs[r1] = cols.iter().zip(&qv).map(|(c, q)| c[r1] * q).sum();
                for r2 in r1..rows {
                    let v: f64 = cols.iter().map(|c| c[r1] * c[r2]).sum();
                    a[r1][r2] = v;
                    a[r2][r1] = v;
                }
            }
            let scale_eps = (0..rows).map(|r| a[r][r]).fold(0.0, f64::max) * 1e-12 + 1e-18;
            for (r, row) in a.iter_mut().enumerate() {
                row[r] += scale_eps;
            }
            let lambda = solve_dense(a, rhs).unwrap_or_else(|| vec![0.0; rows]);
            // Which rows are whose: joints in order, then couplings, then the drive.
            let mut row = 0;
            for (q, jt) in self.joints.iter().enumerate() {
                let count = match jt.kind {
                    Kind::Hinge => 5,
                    Kind::Ball => 3,
                    Kind::Fixed => 6,
                    Kind::Slider => 5,
                    Kind::Cylinder => 4,
                };
                // The force this joint puts on its first body (or on the second when the first is the ground).
                let body = jt.a.or(jt.b);
                let mut f = [0.0; 3];
                if let Some(b) = body {
                    for rr in row..row + count {
                        for d in 0..3 {
                            f[d] += cols[6 * b + d][rr] * lambda[rr];
                        }
                    }
                }
                out.joints[q].push(norm(f));
                row += count;
            }
            row += self.couplings.len();
            // The drive's row: residual (coordinate − target)·size, so effort = λ·size (N·mm → N·m), or λ (N) for a slider.
            let effort = if turning { lambda[row] * size / 1000.0 } else { lambda[row] };
            out.drive.push(effort);
            out.shaking.push(norm(shake));
        }
        // Average power: torque × angular speed (or force × speed), over the range.
        let rate = if turning { drive.speed * std::f64::consts::TAU / 60.0 } else { drive.speed / 1000.0 };
        let m = if full { n - 1 } else { n };
        out.mean_power = out.drive[..m].iter().sum::<f64>() / m as f64 * rate;
        Some(out)
    }
}
