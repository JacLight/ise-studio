//! Fit checks: do the assembled pieces actually go together?
//!
//! Every bought item is recognised by its stock-library family and size
//! (screws, heat-set inserts, nuts, washers, bearings, dowel pins), and the
//! parts around it are measured where it sits, from the true geometry: each
//! neighbour is moved into the item's own frame (its axis on Z, as the
//! library stands every item), sliced square to the axis every quarter
//! millimetre, and 72 rays from the axis find the hole wall, its centre (a
//! least-squares circle), the wall behind it and whether the axis is in
//! solid material. From those:
//!
//! - **Screws**: each hole along the shank — clearance hole (ISO 273 fine /
//!   medium / coarse), the thread it engages (an insert, a nut, a tapped or
//!   self-tapping hole) and how much of it, coaxiality (the shank must clear
//!   every hole wall), bottoming out (solid material before the tip), the
//!   head's seat (what it rests on) and its counterbore, and whether a hex
//!   key reaches the socket in a straight line (ISO 2936 key lengths).
//! - **Heat-set inserts**: the printed hole against the maker's recommended
//!   diameter, its depth (insert length + 1 mm), the wall around it.
//! - **Nuts**: the trap's across-flats against the nut's, hex or round.
//! - **Bearings and pins**: housing bore and shaft against the bearing (or
//!   pin): the ISO 286 class it makes for machined parts, the press or
//!   clearance in mm for printed ones, against the printer profile.
//! - **Joined parts** (a servo in its pocket, a horn on a spline, two links
//!   at a hinge): overlap, and the clearance at the sides once seated.
//! - **Interference at rest** between any two instances not explained by
//!   the items above, and **through the motion** of every moving joint.
//!
//! Every item is pass / warn / fail with what was measured, where, and by
//! how much. The printer profile (`fit.profile.set`) is a project setting:
//! a Bambu X1 printing PLA at 0.2 mm layers by default.

use crate::joints::{self, JointKind};
use crate::motion::{self, Body};
use crate::transform::{Transform, V3};
use crate::{Instance, placement};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use de_shape::ShapeSolid;
use de_shape::library::{self, StockFamily};
use manifold_csg::Manifold;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

// ---- The printer profile ---------------------------------------------------------

/// How the project's printed parts come out: the clearances its fits are
/// judged against. A project setting (one entity), synced and undoable.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FitProfile {
    /// "Bambu Lab X1 Carbon".
    #[serde(default = "default_printer")]
    pub printer: String,
    /// "PLA".
    #[serde(default = "default_material")]
    pub material: String,
    /// Layer height (mm).
    #[serde(default = "d_layer")]
    pub layer: f64,
    /// Gap between moving surfaces printed in place, per side (mm): they
    /// print and turn after a break-in.
    #[serde(default = "d_clearance")]
    pub clearance: f64,
    /// Vertical gap between parts printed in place (mm).
    #[serde(default = "d_layer_gap")]
    pub layer_gap: f64,
    /// Slide (running) fit between separately printed parts, per side (mm): [min, max].
    #[serde(default = "d_slide")]
    pub slide: [f64; 2],
    /// Press fit interference for a bearing or pin in a printed bore (mm on the diameter).
    #[serde(default = "d_press")]
    pub press: f64,
    /// Wall wanted around a heat-set insert (mm); below the maker's minimum fails.
    #[serde(default = "d_wall")]
    pub insert_wall: f64,
}

fn default_printer() -> String {
    "Bambu Lab X1 Carbon".into()
}
fn default_material() -> String {
    "PLA".into()
}
fn d_layer() -> f64 {
    0.2
}
fn d_clearance() -> f64 {
    0.4
}
fn d_layer_gap() -> f64 {
    0.4
}
fn d_slide() -> [f64; 2] {
    [0.2, 0.3]
}
fn d_press() -> f64 {
    0.1
}
fn d_wall() -> f64 {
    2.0
}

impl Default for FitProfile {
    fn default() -> Self {
        FitProfile {
            printer: default_printer(),
            material: default_material(),
            layer: d_layer(),
            clearance: d_clearance(),
            layer_gap: d_layer_gap(),
            slide: d_slide(),
            press: d_press(),
            insert_wall: d_wall(),
        }
    }
}

impl Persist for FitProfile {
    const KIND: &'static str = "assembly.fit_profile";
}

/// The project's printer profile entity, if it has one.
pub fn profile_entity(world: &World) -> Option<StableId> {
    world.resource::<IdIndex>().iter().filter(|(_, e)| world.get::<FitProfile>(*e).is_some()).map(|(id, _)| id).min_by_key(|id| id.to_string())
}

/// The project's printer profile (the defaults when it has none).
pub fn profile(world: &World) -> FitProfile {
    profile_entity(world).and_then(|id| world.resource::<IdIndex>().get(id)).and_then(|e| world.get::<FitProfile>(e).cloned()).unwrap_or_default()
}

/// `fit.profile.set`: the printer profile fits are judged against (fields
/// left out keep their value; the first call creates it from the defaults).
#[derive(Deserialize, JsonSchema)]
pub struct SetFitProfile {
    pub printer: Option<String>,
    pub material: Option<String>,
    pub layer: Option<f64>,
    pub clearance: Option<f64>,
    pub layer_gap: Option<f64>,
    pub slide: Option<[f64; 2]>,
    pub press: Option<f64>,
    pub insert_wall: Option<f64>,
}

impl Command for SetFitProfile {
    const NAME: &'static str = "fit.profile.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let existing = profile_entity(tx.world());
        let mut p = existing.and_then(|id| tx.get::<FitProfile>(id).cloned()).unwrap_or_default();
        let bad = |m: &str| CommandError::new(ErrorCode::InvalidParameters, m.to_string());
        if let Some(v) = self.printer {
            p.printer = v;
        }
        if let Some(v) = self.material {
            p.material = v;
        }
        for (v, slot, what, max) in [
            (self.layer, &mut p.layer, "layer", 1.0),
            (self.clearance, &mut p.clearance, "clearance", 2.0),
            (self.layer_gap, &mut p.layer_gap, "layer_gap", 2.0),
            (self.press, &mut p.press, "press", 1.0),
            (self.insert_wall, &mut p.insert_wall, "insert_wall", 20.0),
        ] {
            if let Some(v) = v {
                if !(v.is_finite() && v >= 0.0 && v <= max) {
                    return Err(bad(&format!("{what} must be 0 to {max} mm")));
                }
                *slot = v;
            }
        }
        if let Some([a, b]) = self.slide {
            if !(a.is_finite() && b.is_finite() && 0.0 <= a && a <= b && b <= 2.0) {
                return Err(bad("slide is [min, max] per side, 0 ≤ min ≤ max ≤ 2 mm"));
            }
            p.slide = [a, b];
        }
        let id = match existing {
            Some(id) => id,
            None => tx.create(),
        };
        tx.set(id, p)?;
        Ok(json!({ "id": id }))
    }
}

// ---- The report ------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FitStatus {
    Pass,
    Warn,
    Fail,
}

/// One thing measured about an item.
#[derive(Clone, Debug, Serialize)]
pub struct FitCheck {
    /// "Clearance hole", "Thread engagement", "Coaxial", "Head seat", "Hex key", …
    pub what: String,
    pub status: FitStatus,
    /// In words, with the numbers: "Ø3.4 in Upper arm: ISO 273 medium".
    pub text: String,
    /// The main measured value and its unit, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// What it should be, in words ("3.2–3.6 mm", "≥ 4.3 mm").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// The part it was measured in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// Where (assembly mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<V3>,
}

/// One item of the report: a fastener, insert, nut, bearing, joined pair,
/// overlap or joint motion.
#[derive(Clone, Debug, Serialize)]
pub struct FitItem {
    /// "screw", "insert", "nut", "washer", "bearing", "pin", "joined", "interference", "motion".
    pub kind: &'static str,
    pub title: String,
    pub status: FitStatus,
    /// The instances it concerns (the item first), in the assembly.
    pub instances: Vec<StableId>,
    pub names: Vec<String>,
    /// The item's place (assembly mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<V3>,
    /// The joint that seats or moves it, by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joint: Option<String>,
    pub checks: Vec<FitCheck>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FitReport {
    /// The assembly checked (none: the top of the project).
    pub assembly: Option<StableId>,
    pub name: String,
    pub profile: FitProfile,
    pub items: Vec<FitItem>,
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
    /// No item fails.
    pub ok: bool,
    pub notes: Vec<String>,
}

/// What to check.
#[derive(Clone, Debug)]
pub struct FitOptions {
    /// Sweep every moving joint through its limits (slower).
    pub motion: bool,
    /// Steps per sweep.
    pub steps: usize,
}

impl Default for FitOptions {
    fn default() -> Self {
        FitOptions { motion: true, steps: 36 }
    }
}

// ---- What each instance is -----------------------------------------------------------

#[derive(Clone, Debug)]
enum Kind {
    Screw { d: f64, len: f64, dk: f64, k: f64, key: f64, thread: f64, countersunk: bool, family: String },
    Insert { d: f64, len: f64, hole: f64, depth: f64, wall: f64, od: f64 },
    Nut { d: f64, s: f64, m: f64 },
    Washer { d: f64 },
    Bearing { d: f64, outer: f64, width: f64, number: String },
    Pin { d: f64, len: f64 },
    /// Some other bought item (a servo, a stepper, a horn).
    Stock,
    /// A part we make: printed or not, with its process.
    Made { printed: bool },
}

impl Kind {
    fn bought(&self) -> bool {
        !matches!(self, Kind::Made { .. })
    }
    fn printed(&self) -> bool {
        matches!(self, Kind::Made { printed: true })
    }
    /// Holds a thread a screw engages.
    fn threaded(&self) -> bool {
        matches!(self, Kind::Insert { .. } | Kind::Nut { .. })
    }
    /// Covered by its own item (and its neighbours' overlaps explained there).
    fn has_item(&self) -> bool {
        matches!(self, Kind::Screw { .. } | Kind::Insert { .. } | Kind::Nut { .. } | Kind::Washer { .. } | Kind::Bearing { .. } | Kind::Pin { .. })
    }
}

struct Inst {
    id: StableId,
    name: String,
    kind: Kind,
    place: Transform,
    body: Body,
    /// Its leaves in assembly coordinates, with their bounds.
    world: Vec<(Manifold, V3, V3)>,
    lo: V3,
    hi: V3,
}

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

fn kind_of(world: &World, id: StableId, inst: &Instance) -> Kind {
    let Some(def) = entity(world, inst.of) else { return Kind::Stock };
    let configured = entity(world, id).and_then(|e| world.get::<crate::InstanceConfigured>(e)).and_then(|c| c.0.clone());
    let stats = match &configured {
        Some(c) => Some(c.stats.clone()),
        None => world.get::<ShapeSolid>(def).map(|s| s.stats.clone()),
    };
    let metric = |k: &str| stats.as_ref().and_then(|s| s.metrics.get(k)).map(|m| m.value);
    let family = world.get::<StockFamily>(def).map(|f| f.family.clone());
    // Size values: the configuration's row, else the definition's parameters.
    let mut values: BTreeMap<String, f64> = world.get::<de_shape::ShapeProgram>(def).map(|p| p.params.iter().map(|q| (q.key.clone(), q.value)).collect()).unwrap_or_default();
    if let (Some(f), Some(c)) = (&family, &inst.config)
        && let Some(row) = library::catalogue().ok().and_then(|cat| cat.family(f)).and_then(|fam| fam.row(c))
    {
        values.extend(row.values.clone());
    } else if let (Some(c), Some(t)) = (&inst.config, world.get::<de_shape::ShapeConfigurations>(def))
        && let Some(row) = t.rows.iter().find(|r| &r.name == c)
    {
        values.extend(row.values.clone());
    }
    let v = |k: &str| values.get(k).copied().unwrap_or(0.0);
    let declared_stock = stats.as_ref().is_some_and(|s| s.declared.stock.is_some());
    match family.as_deref() {
        Some(f @ ("iso4762" | "iso7380" | "iso10642")) => {
            let (dk, k) = stats.as_ref().map(|s| (s.bounds_max[0] - s.bounds_min[0], s.bounds_max[2].max(0.0))).unwrap_or((1.6 * v("d"), v("d")));
            Kind::Screw {
                d: v("d"),
                len: v("length"),
                dk,
                k,
                key: metric("Hex key").unwrap_or(0.0),
                thread: metric("Thread length").unwrap_or(v("length")),
                countersunk: f == "iso10642",
                family: f.to_string(),
            }
        }
        Some("heatset_insert") => Kind::Insert {
            d: v("d"),
            len: v("length"),
            hole: metric("Printed hole Ø").unwrap_or(0.0),
            depth: metric("Hole depth").unwrap_or(v("length") + 1.0),
            wall: metric("Minimum wall").unwrap_or(1.0),
            od: metric("Outside Ø").unwrap_or(0.0),
        },
        Some("iso4032") => Kind::Nut { d: v("d"), s: metric("Across flats").unwrap_or(0.0), m: metric("Height").unwrap_or(0.0) },
        Some("iso7089") => Kind::Washer { d: v("d") },
        Some("bearing_deep_groove") => Kind::Bearing {
            d: v("d"),
            outer: v("D"),
            width: v("B"),
            number: stats.as_ref().and_then(|s| s.declared.stock.as_ref()).map(|s| s.part_number.clone()).unwrap_or_else(|| "bearing".into()),
        },
        Some("iso8734") => Kind::Pin { d: v("d"), len: v("length") },
        Some(_) => Kind::Stock,
        None if declared_stock => Kind::Stock,
        None => {
            let parts = stats.as_ref().map(|s| s.parts.clone()).unwrap_or_default();
            let printed = parts.iter().any(|p| p.process.as_deref() == Some("3d_print"));
            if !parts.is_empty() && parts.iter().all(|p| p.process.as_deref() == Some("buy")) {
                Kind::Stock
            } else {
                Kind::Made { printed }
            }
        }
    }
}

// ---- Slicing ---------------------------------------------------------------------------

const RAYS: usize = 72;

/// One slice square to an item's axis, seen from the axis: for each ray, the
/// first surface met and the one after it (∞ when none).
#[derive(Clone, Debug)]
struct Ring {
    /// The axis point is inside solid material.
    inside: bool,
    first: [f64; RAYS],
    second: [f64; RAYS],
    /// Vertices of the contour most rays meet first (the wall's true outline).
    contour: Vec<[f64; 2]>,
}

impl Ring {
    fn hits(&self, within: f64) -> usize {
        self.first.iter().filter(|r| **r < within).count()
    }
    fn min(&self) -> f64 {
        self.first.iter().copied().fold(f64::INFINITY, f64::min)
    }
    fn dir(i: usize) -> [f64; 2] {
        let a = (i as f64 + 0.37) * std::f64::consts::TAU / RAYS as f64;
        [a.cos(), a.sin()]
    }
    /// Least-squares circle through the first hits within `within`: centre and radius.
    fn circle(&self, within: f64) -> Option<([f64; 2], f64)> {
        // A tessellated circle's vertices lie on it: fit those when the wall is round.
        let near: Vec<[f64; 2]> = self.contour.iter().copied().filter(|p| (p[0] * p[0] + p[1] * p[1]).sqrt() < within).collect();
        if let Some((c, r)) = circle_fit(&near) {
            let spread = near.iter().map(|p| (((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt() - r).abs()).fold(0.0, f64::max);
            if spread < 0.02 * r.max(1.0) {
                return Some((c, r));
            }
        }
        let pts: Vec<[f64; 2]> = (0..RAYS).filter(|i| self.first[*i] < within).map(|i| {
            let u = Ring::dir(i);
            [u[0] * self.first[i], u[1] * self.first[i]]
        }).collect();
        circle_fit(&pts)
    }
}

/// Kåsa fit: x² + y² + a·x + b·y + c = 0.
fn circle_fit(pts: &[[f64; 2]]) -> Option<([f64; 2], f64)> {
    if pts.len() < 6 {
        return None;
    }
    let (mut sxx, mut sxy, mut syy, mut sx, mut sy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut sxz, mut syz, mut sz) = (0.0, 0.0, 0.0);
    for p in pts {
        let (x, y) = (p[0], p[1]);
        let z = x * x + y * y;
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sx += x;
        sy += y;
        n += 1.0;
        sxz += x * z;
        syz += y * z;
        sz += z;
    }
    // [sxx sxy sx; sxy syy sy; sx sy n] [a b c]ᵀ = −[sxz syz sz]ᵀ
    let m = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]];
    let r = [-sxz, -syz, -sz];
    let det = |m: &[[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = det(&m);
    if d.abs() < 1e-18 {
        return None;
    }
    let solve = |k: usize| {
        let mut q = m;
        for i in 0..3 {
            q[i][k] = r[i];
        }
        det(&q) / d
    };
    let (a, b, c) = (solve(0), solve(1), solve(2));
    let center = [-a / 2.0, -b / 2.0];
    let rr = center[0] * center[0] + center[1] * center[1] - c;
    (rr > 0.0).then(|| (center, rr.sqrt()))
}

/// Crossings of the ray from the origin along `u` with closed contours,
/// sorted: distance, contour, and whether it is on the clipping square of
/// half-size `clip` (an edge of the region looked at, not of the part).
fn crossings(polys: &[Vec<[f64; 2]>], u: [f64; 2], clip: f64) -> Vec<(f64, usize, bool)> {
    let mut ts = vec![];
    for (k, poly) in polys.iter().enumerate() {
        let n = poly.len();
        for i in 0..n {
            let p = poly[i];
            let q = poly[(i + 1) % n];
            let e = [q[0] - p[0], q[1] - p[1]];
            let denom = u[0] * e[1] - u[1] * e[0];
            if denom.abs() < 1e-14 {
                continue;
            }
            let t = (p[0] * e[1] - p[1] * e[0]) / denom;
            let s = (p[0] * u[1] - p[1] * u[0]) / denom;
            if t > 1e-9 && (0.0..1.0).contains(&s) {
                let on_clip = p[0].abs().min(q[0].abs()) > clip - 1e-3 || p[1].abs().min(q[1].abs()) > clip - 1e-3;
                ts.push((t, k, on_clip));
            }
        }
    }
    ts.sort_by(|a, b| a.0.total_cmp(&b.0));
    ts
}

/// The solids of one neighbour in an item's frame, cut down to the region looked at.
struct Local {
    leaves: Vec<Manifold>,
    /// Half-size of the square they were cut to, across the axis.
    clip: f64,
}

impl Local {
    fn ring(&self, z: f64) -> Ring {
        let mut ring = Ring { inside: false, first: [f64::INFINITY; RAYS], second: [f64::INFINITY; RAYS], contour: vec![] };
        let mut best: Option<(usize, Vec<[f64; 2]>)> = None;
        for leaf in &self.leaves {
            let polys = leaf.slice_at_z(z);
            if polys.is_empty() {
                continue;
            }
            let inside = crossings(&polys, Ring::dir(0), self.clip).len() % 2 == 1;
            ring.inside |= inside;
            let mut count = vec![0usize; polys.len()];
            for i in 0..RAYS {
                let ts: Vec<(f64, usize, bool)> = crossings(&polys, Ring::dir(i), self.clip);
                let Some(&(t0, k0, c0)) = ts.first() else { continue };
                if c0 {
                    continue;
                }
                if t0 < ring.first[i] {
                    ring.first[i] = t0;
                    ring.second[i] = ts.get(1).filter(|x| !x.2).map(|x| x.0).unwrap_or(f64::INFINITY);
                    count[k0] += 1;
                }
            }
            if let Some((k, n)) = count.iter().enumerate().max_by_key(|(_, n)| **n)
                && *n > 0
                && best.as_ref().is_none_or(|b| *n > b.0)
            {
                // The contour most rays meet first: the hole (or shaft) wall, its true vertices.
                let pts: Vec<[f64; 2]> = polys[k].iter().copied().filter(|p| p[0].abs() < self.clip - 1e-3 && p[1].abs() < self.clip - 1e-3).collect();
                best = Some((*n, pts));
            }
        }
        if let Some((_, pts)) = best {
            ring.contour = pts;
        }
        ring
    }

    fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    fn volume_with(&self, other: &[Manifold]) -> f64 {
        let mut v = 0.0;
        for a in &self.leaves {
            for b in other {
                v += a.intersection(b).volume();
            }
        }
        v
    }
}

/// `inst` in the frame `frame` (assembly coordinates), cut to the box lo–hi of that frame.
fn local(inst: &Inst, frame: &Transform, lo: V3, hi: V3) -> Local {
    let inv = frame.inverse();
    let mut leaves = vec![];
    for (solid, inner) in &inst.body.leaves {
        let t = inv.then(&inst.place.then(inner));
        let Some(bb) = solid.bounding_box() else { continue };
        let (a, b) = t.bounds(bb.min(), bb.max());
        if (0..3).any(|k| b[k] < lo[k] || a[k] > hi[k]) {
            continue;
        }
        let m = t.solid(solid);
        let clip = Manifold::cube(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2], false).translate(lo[0], lo[1], lo[2]);
        let c = m.intersection(&clip);
        if !c.is_empty() {
            leaves.push(c);
        }
    }
    Local { leaves, clip: hi[0].min(-lo[0]).min(hi[1]).min(-lo[1]) }
}

/// Instances whose placed bounds reach the box lo–hi of `frame`.
fn near(insts: &[Inst], frame: &Transform, lo: V3, hi: V3, except: usize) -> Vec<usize> {
    let inv = frame.inverse();
    (0..insts.len())
        .filter(|i| *i != except && !insts[*i].world.is_empty())
        .filter(|i| {
            let (a, b) = inv.bounds(insts[*i].lo, insts[*i].hi);
            (0..3).all(|k| b[k] >= lo[k] && a[k] <= hi[k])
        })
        .collect()
}

// ---- Numbers --------------------------------------------------------------------------

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn mm(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    let s = format!("{r:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

/// ISO 273 clearance holes: fine, medium, coarse (mm).
fn iso273(d: f64) -> Option<[f64; 3]> {
    let t: [(f64, [f64; 3]); 10] = [
        (1.6, [1.7, 1.8, 2.0]),
        (2.0, [2.2, 2.4, 2.6]),
        (2.5, [2.7, 2.9, 3.1]),
        (3.0, [3.2, 3.4, 3.6]),
        (4.0, [4.3, 4.5, 4.8]),
        (5.0, [5.3, 5.5, 5.8]),
        (6.0, [6.4, 6.6, 7.0]),
        (8.0, [8.4, 9.0, 10.0]),
        (10.0, [10.5, 11.0, 12.0]),
        (12.0, [13.0, 13.5, 14.5]),
    ];
    t.iter().find(|(n, _)| (n - d).abs() < 1e-6).map(|(_, r)| *r)
}

/// ISO 2936 hex key arm lengths (long, short) for a key size.
fn key_arms(s: f64) -> (f64, f64) {
    let t = [(1.5, 46.5, 15.5), (2.0, 50.0, 16.0), (2.5, 56.0, 18.0), (3.0, 63.0, 20.0), (4.0, 70.0, 25.0), (5.0, 80.0, 28.0), (6.0, 90.0, 32.0), (8.0, 100.0, 36.0)];
    t.iter().find(|(k, _, _)| (k - s).abs() < 1e-6).map(|(_, a, b)| (*a, *b)).unwrap_or((10.0 * s + 30.0, 3.5 * s + 9.0))
}

/// The ISO 286 class (from the common ones) whose limits hold a measured
/// deviation `dev` (mm) at `nominal`.
fn iso286(nominal: f64, dev: f64, hole: bool) -> Option<String> {
    let classes: &[&str] = if hole { &["H7", "JS7", "K7", "N7", "P7", "G7", "F8", "H8", "H9", "H11"] } else { &["h6", "js6", "k6", "m6", "n6", "p6", "g6", "f7", "h9", "h11"] };
    classes.iter().find(|c| de_shape::drawing::fits::deviations(c, nominal).is_ok_and(|(up, lo)| dev <= up + 1e-9 && dev >= lo - 1e-9)).map(|c| c.to_string())
}

fn check(what: &str, status: FitStatus, text: String) -> FitCheck {
    FitCheck { what: what.into(), status, text, measured: None, unit: String::new(), target: String::new(), part: None, at: None }
}

impl FitCheck {
    fn value(mut self, v: f64, unit: &str) -> Self {
        self.measured = Some(r2(v * 50.0) / 50.0);
        self.unit = unit.into();
        self
    }
    fn target(mut self, t: impl Into<String>) -> Self {
        self.target = t.into();
        self
    }
    fn part(mut self, p: &str) -> Self {
        self.part = Some(p.into());
        self
    }
    fn at(mut self, p: V3) -> Self {
        self.at = Some(p.map(|c| (c * 1000.0).round() / 1000.0));
        self
    }
}

fn item(kind: &'static str, title: String, insts: &[&Inst], at: Option<V3>, joint: Option<String>, checks: Vec<FitCheck>) -> FitItem {
    let status = checks.iter().map(|c| c.status).max().unwrap_or(FitStatus::Pass);
    FitItem {
        kind,
        title,
        status,
        instances: insts.iter().map(|i| i.id).collect(),
        names: insts.iter().map(|i| i.name.clone()).collect(),
        at: at.map(|p| p.map(|c| (c * 1000.0).round() / 1000.0)),
        joint,
        checks,
    }
}

// ---- The analysis ------------------------------------------------------------------------

/// One sample down an item's axis inside one neighbour.
#[derive(Clone, Debug)]
struct Sample {
    z: f64,
    inside: bool,
    /// Smallest distance to the wall (mm).
    rmin: f64,
    /// Fitted wall circle: centre offset from the axis and radius.
    center: [f64; 2],
    radius: f64,
    /// Thinnest wall behind the hole (mm, ∞ when it does not end in reach).
    wall: f64,
    /// Share of rays that met a wall.
    cover: f64,
    max: f64,
}

fn sample(l: &Local, z: f64, reach: f64) -> Option<Sample> {
    let ring = l.ring(z);
    let hits = ring.hits(reach);
    if !ring.inside && (hits as f64) < 0.25 * RAYS as f64 {
        return None;
    }
    let (center, radius) = if ring.inside { ([0.0, 0.0], 0.0) } else { ring.circle(reach).unwrap_or(([0.0, 0.0], ring.min())) };
    let wall = (0..RAYS).filter(|i| ring.first[*i] < reach).map(|i| ring.second[i] - ring.first[i]).fold(f64::INFINITY, f64::min);
    let max = (0..RAYS).filter(|i| ring.first[*i] < reach).map(|i| ring.first[i]).fold(0.0, f64::max);
    Some(Sample { z, inside: ring.inside, rmin: ring.min(), center, radius, wall, cover: hits as f64 / RAYS as f64, max })
}

/// Consecutive samples down the axis in one neighbour.
#[derive(Clone, Debug)]
struct Segment {
    host: usize,
    samples: Vec<Sample>,
}

impl Segment {
    fn top(&self) -> f64 {
        self.samples.first().map(|s| s.z).unwrap_or(0.0)
    }
    fn bottom(&self) -> f64 {
        self.samples.last().map(|s| s.z).unwrap_or(0.0)
    }
    fn hole(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter().filter(|s| !s.inside)
    }
    /// Smallest hole diameter (inscribed), fitted diameter (median) and the largest centre offset.
    fn measure(&self) -> Option<(f64, f64, f64)> {
        let hs: Vec<&Sample> = self.hole().collect();
        if hs.is_empty() {
            return None;
        }
        let dmin = hs.iter().map(|s| 2.0 * s.rmin).fold(f64::INFINITY, f64::min);
        let mut ds: Vec<f64> = hs.iter().map(|s| 2.0 * s.radius).collect();
        ds.sort_by(f64::total_cmp);
        let dfit = ds[ds.len() / 2];
        let off = hs.iter().map(|s| (s.center[0].powi(2) + s.center[1].powi(2)).sqrt()).fold(0.0, f64::max);
        Some((dmin, dfit, off))
    }
}

fn segments(host: usize, samples: Vec<Option<Sample>>) -> Vec<Segment> {
    let mut out = vec![];
    let mut cur: Vec<Sample> = vec![];
    for s in samples {
        match s {
            Some(s) => cur.push(s),
            None if !cur.is_empty() => out.push(Segment { host, samples: std::mem::take(&mut cur) }),
            None => {}
        }
    }
    if !cur.is_empty() {
        out.push(Segment { host, samples: cur });
    }
    out
}

/// Depths from `top` down to `bottom` (both > bottom), every `step`.
fn depths(top: f64, bottom: f64, step: f64) -> Vec<f64> {
    let n = ((top - bottom) / step).floor().max(0.0) as usize;
    (0..=n).map(|i| top - i as f64 * step + 0.0137).filter(|z| *z > bottom).collect()
}

struct Ctx<'a> {
    world: &'a World,
    insts: Vec<Inst>,
    profile: FitProfile,
    /// Rigid group of each instance (instances held together by fixed joints).
    rigid: Vec<usize>,
    /// The joint seating each instance, by name.
    seated_by: HashMap<StableId, String>,
    /// Pairs whose contact an item explains (no interference item for them).
    explained: std::cell::RefCell<HashSet<(usize, usize)>>,
}

impl Ctx<'_> {
    fn explain(&self, a: usize, b: usize) {
        self.explained.borrow_mut().insert((a.min(b), a.max(b)));
    }
    fn at(&self, i: usize, p: V3) -> V3 {
        self.insts[i].place.apply(p)
    }
}

fn screw_item(cx: &mut Ctx, i: usize) -> FitItem {
    let Kind::Screw { d, len, dk, k, key, thread, countersunk, ref family } = cx.insts[i].kind.clone() else { unreachable!() };
    let frame = cx.insts[i].place;
    let name = cx.insts[i].name.clone();
    let reach = (dk.max(d) / 2.0 + 4.0).max(6.0);
    let mut checks = vec![];
    let lo = [-reach, -reach, -len - 4.0];
    let hi = [reach, reach, k + 1.0];
    let hosts: Vec<usize> = near(&cx.insts, &frame, lo, hi, i).into_iter().filter(|h| !matches!(cx.insts[*h].kind, Kind::Screw { .. })).collect();
    let locals: Vec<(usize, Local)> = hosts.iter().map(|h| (*h, local(&cx.insts[*h], &frame, lo, hi))).filter(|(_, l)| !l.is_empty()).collect();
    let step = 0.25;
    let zs = depths(-0.2, -len + 0.15, step);
    let mut segs: Vec<Segment> = vec![];
    for (h, l) in &locals {
        let samples: Vec<Option<Sample>> = zs.iter().map(|z| sample(l, *z, reach)).collect();
        segs.extend(segments(*h, samples));
    }
    segs.sort_by(|a, b| b.top().total_cmp(&a.top()));
    for s in &segs {
        cx.explain(i, s.host);
    }
    // Thread engagement: inserts and nuts along the threaded length.
    let thread_top = -len + thread;
    let mut engaged = 0.0;
    let mut providers: Vec<(usize, f64, f64)> = vec![]; // host, top, bottom
    let mut required: f64 = 0.0;
    for s in segs.iter().filter(|s| cx.insts[s.host].kind.threaded()) {
        let (top, bottom) = (s.top() + step / 2.0, s.bottom() - step / 2.0);
        let e = (top.min(thread_top) - bottom.max(-len)).max(0.0);
        engaged += e;
        providers.push((s.host, top, bottom));
        let host = &cx.insts[s.host];
        let need = match host.kind {
            Kind::Insert { len: li, .. } => 0.75 * li.min(2.0 * d),
            Kind::Nut { m, .. } => 0.9 * m,
            _ => 1.5 * d,
        };
        required = required.max(need);
        if let Some((_, _, off)) = s.measure() {
            let st = if off <= 0.1 { FitStatus::Pass } else if off <= 0.2 { FitStatus::Warn } else { FitStatus::Fail };
            checks.push(check("Coaxial", st, format!("{} {} mm off the screw's axis", host.name, mm(off))).value(off, "mm").target("≤ 0.1 mm").part(&host.name));
        }
    }
    // Holes in the parts it passes through.
    let mut tapped: Vec<(usize, f64)> = vec![];
    let mut bottomed: Option<(usize, f64)> = None;
    for s in segs.iter().filter(|s| !cx.insts[s.host].kind.threaded()) {
        let host = &cx.insts[s.host];
        // The pocket an insert or nut sits in (and the relief under an insert) is theirs, not a clearance hole.
        let in_pocket = |z: f64| providers.iter().any(|(p, top, bottom)| z <= *top + 0.3 && z >= *bottom - if matches!(cx.insts[*p].kind, Kind::Insert { .. }) { 1.6 } else { 0.3 });
        if let Some(solid) = s.samples.iter().find(|x| x.inside) {
            bottomed = Some(match bottomed {
                Some((h, z)) if z >= solid.z => (h, z),
                _ => (s.host, solid.z),
            });
        }
        let kept: Vec<Sample> = s.samples.iter().filter(|x| !x.inside && !in_pocket(x.z)).cloned().collect();
        if kept.is_empty() {
            continue;
        }
        let seg = Segment { host: s.host, samples: kept };
        let Some((dmin, dfit, off)) = seg.measure() else { continue };
        let span = seg.top() - seg.bottom() + step;
        let at = cx.at(i, [0.0, 0.0, (seg.top() + seg.bottom()) / 2.0]);
        if dfit < d - 0.05 {
            // Smaller than the screw: a tapped or self-tapping hole, or too small.
            if dfit >= 0.72 * d && host.kind.printed() {
                tapped.push((s.host, span));
                checks.push(check("Self-tapping hole", FitStatus::Pass, format!("Ø{} pilot in {} (for M{} into plastic: {}–{} mm), {} mm long", mm(dfit), host.name, mm(d), mm(0.8 * d), mm(0.92 * d), mm(span))).value(dfit, "mm").target(format!("{}–{} mm", mm(0.8 * d), mm(0.92 * d))).part(&host.name).at(at));
            } else {
                checks.push(check("Hole", FitStatus::Fail, format!("Ø{} in {} is smaller than the M{} screw: it will not go in (clearance Ø{} or more, or an insert)", mm(dmin), host.name, mm(d), mm(iso273(d).map(|t| t[0]).unwrap_or(d + 0.2)))).value(dmin, "mm").target(format!("≥ {} mm", mm(iso273(d).map(|t| t[0]).unwrap_or(d + 0.2)))).part(&host.name).at(at));
            }
            continue;
        }
        let room = dmin / 2.0 - off - d / 2.0;
        let [fine, medium, coarse] = iso273(d).unwrap_or([d + 0.2, d + 0.4, d + 0.6]);
        let (st, class) = if dfit + 0.02 < fine {
            (if host.kind.bought() { FitStatus::Pass } else { FitStatus::Warn }, format!("close fit, under ISO 273 fine (Ø{}){}", mm(fine), if host.kind.bought() { ": the maker's hole for this screw" } else { "" }))
        } else if dfit <= fine + 0.1 {
            (FitStatus::Pass, "ISO 273 fine".to_string())
        } else if dfit <= medium + 0.1 {
            (FitStatus::Pass, "ISO 273 medium".to_string())
        } else if dfit <= coarse + 0.1 {
            (FitStatus::Pass, "ISO 273 coarse".to_string())
        } else if dk > 0.0 && dfit < dk - 0.6 {
            (if host.kind.bought() { FitStatus::Pass } else { FitStatus::Warn }, format!("loose, over ISO 273 coarse (Ø{}); the head still covers it", mm(coarse)))
        } else {
            (FitStatus::Fail, format!("so big the Ø{} head can pull through", mm(dk)))
        };
        checks.push(check("Clearance hole", st, format!("Ø{} in {}: {class}", mm(dfit), host.name)).value(dfit, "mm").target(format!("{}–{} mm", mm(fine), mm(coarse))).part(&host.name).at(at));
        let st = if room >= 0.0 { FitStatus::Pass } else { FitStatus::Fail };
        let text = if room >= 0.0 {
            format!("{} mm off-axis in {}; the shank clears the wall by {} mm", mm(off), host.name, mm(room))
        } else {
            format!("{} mm off-axis in {}: the shank hits the hole wall ({} mm)", mm(off), host.name, mm(-room))
        };
        checks.push(check("Coaxial", st, text).value(off, "mm").target(format!("≤ {} mm", mm((dmin - d) / 2.0))).part(&host.name).at(at));
    }
    for (h, span) in &tapped {
        engaged += span;
        required = required.max(match cx.insts[*h].kind {
            _ => 1.5 * d,
        });
        providers.push((*h, 0.0, 0.0));
    }
    // Engagement and length.
    let lengths = screw_lengths(family, d);
    if providers.is_empty() {
        checks.push(check("Thread engagement", FitStatus::Fail, format!("Nothing to thread into along its {} mm: no insert, nut or tapped hole", mm(len))).value(0.0, "mm"));
    } else {
        let st = if engaged + 1e-6 >= required { FitStatus::Pass } else { FitStatus::Fail };
        let mut text = format!("{} mm of thread engaged in {} (needs {} mm)", mm(engaged), providers.iter().map(|(h, _, _)| cx.insts[*h].name.clone()).collect::<Vec<_>>().join(", "), mm(required));
        if st == FitStatus::Fail {
            let want = len + (required - engaged);
            if let Some(l) = lengths.iter().find(|l| **l + 1e-6 >= want) {
                text.push_str(&format!(": too short, use M{}×{}", mm(d), mm(*l)));
            } else {
                text.push_str(": too short");
            }
        }
        checks.push(check("Thread engagement", st, text).value(engaged, "mm").target(format!("≥ {} mm", mm(required))));
    }
    match bottomed {
        Some((h, z)) => {
            let depth = -z;
            let fits: Option<&f64> = lengths.iter().rev().find(|l| **l <= depth - 0.5);
            checks.push(
                check(
                    "Bottoming out",
                    FitStatus::Fail,
                    format!(
                        "The tip runs into solid {} {} mm under the head (the screw is {} mm): {}",
                        cx.insts[h].name,
                        mm(depth),
                        mm(len),
                        match fits {
                            Some(l) => format!("use M{}×{} or deepen the hole", mm(d), mm(*l)),
                            None => "deepen the hole".to_string(),
                        }
                    ),
                )
                .value(depth, "mm")
                .target(format!("> {} mm", mm(len)))
                .part(&cx.insts[h].name)
                .at(cx.at(i, [0.0, 0.0, z])),
            );
        }
        None => checks.push(check("Bottoming out", FitStatus::Pass, format!("The tip runs free to its full {} mm", mm(len)))),
    }
    // The head: its seat and the room around it.
    if countersunk {
        checks.push(check("Head seat", FitStatus::Pass, "Countersunk head: seat checked as an overlap only".into()));
    } else if dk > 0.0 {
        let mut clash: Option<(usize, f64)> = None;
        let mut bore: Option<(usize, f64)> = None;
        for (h, l) in &locals {
            for z in [0.08, k / 2.0, (k - 0.08).max(0.1)] {
                let ring = l.ring(z);
                if ring.hits(reach) == 0 && !ring.inside {
                    continue;
                }
                let r = if ring.inside { 0.0 } else { ring.min() };
                if r < dk / 2.0 - 0.02 {
                    if clash.is_none_or(|(_, c)| r < c) {
                        clash = Some((*h, r));
                    }
                } else if ring.hits(reach) as f64 > 0.5 * RAYS as f64 && bore.is_none_or(|(_, b)| r < b) {
                    bore = Some((*h, r));
                }
            }
        }
        match (clash, bore) {
            (Some((h, r)), _) => checks.push(check("Head clearance", FitStatus::Fail, format!("The Ø{} head hits {}: the counterbore is Ø{}", mm(dk), cx.insts[h].name, mm(2.0 * r))).value(2.0 * r, "mm").target(format!("≥ {} mm", mm(dk + 2.0 * cx.profile.slide[0]))).part(&cx.insts[h].name)),
            (None, Some((h, r))) => {
                let side = r - dk / 2.0;
                let st = if side + 1e-6 >= cx.profile.slide[0] { FitStatus::Pass } else { FitStatus::Warn };
                checks.push(check("Counterbore", st, format!("Ø{} in {}, {} mm round the Ø{} head", mm(2.0 * r), cx.insts[h].name, mm(side), mm(dk))).value(2.0 * r, "mm").target(format!("≥ {} mm", mm(dk + 2.0 * cx.profile.slide[0]))).part(&cx.insts[h].name));
            }
            _ => {}
        }
        // What the head bears on, just under it.
        let mut covered = [false; RAYS];
        let mut under: Vec<String> = vec![];
        for (h, l) in &locals {
            let ring = l.ring(-0.08);
            let mut any = false;
            for (n, c) in covered.iter_mut().enumerate() {
                if ring.inside || ring.first[n] < dk / 2.0 - 0.1 {
                    *c = true;
                    any = true;
                }
            }
            if any {
                under.push(cx.insts[*h].name.clone());
            }
        }
        let share = covered.iter().filter(|c| **c).count() as f64 / RAYS as f64;
        if share >= 0.7 {
            checks.push(check("Head seat", FitStatus::Pass, format!("Seats flat on {} ({}% of the bearing ring)", under.join(", "), (share * 100.0).round())).value(share * 100.0, "%"));
        } else {
            // How far below is the first face?
            let gap = locals.iter().filter_map(|(_, l)| (1..=40).map(|n| -0.08 - 0.1 * n as f64).find(|z| l.ring(*z).hits(dk / 2.0 - 0.1) as f64 > 0.5 * RAYS as f64)).fold(f64::NEG_INFINITY, f64::max);
            let text = if gap.is_finite() { format!("The head stands {} mm above the face under it: it does not seat", mm(-gap - 0.08)) } else { format!("Nothing under the head ({}% of its ring bears)", (share * 100.0).round()) };
            checks.push(check("Head seat", if share >= 0.4 { FitStatus::Warn } else { FitStatus::Fail }, text).value(share * 100.0, "%"));
        }
    }
    // Hex key: a straight path from the socket for the key's long arm.
    if key > 0.0 && !countersunk {
        checks.push(key_reach(cx, i, key, k));
    } else if key > 0.0 {
        checks.push(key_reach(cx, i, key, 0.0));
    }
    let at = cx.at(i, [0.0; 3]);
    let mut with: Vec<usize> = segs.iter().map(|s| s.host).collect();
    with.dedup();
    let mut refs: Vec<&Inst> = vec![&cx.insts[i]];
    let mut seen = HashSet::new();
    for h in &with {
        if seen.insert(*h) {
            refs.push(&cx.insts[*h]);
        }
    }
    let joint = cx.seated_by.get(&cx.insts[i].id).cloned();
    item("screw", format!("{name}: M{}×{}", mm(d), mm(len)), &refs, Some(at), joint, checks)
}

/// Lengths the library holds for a screw family and diameter.
fn screw_lengths(family: &str, d: f64) -> Vec<f64> {
    let mut out: Vec<f64> = library::catalogue()
        .ok()
        .and_then(|c| c.family(family))
        .map(|f| f.rows.iter().filter(|r| r.values.get("d").is_some_and(|x| (x - d).abs() < 1e-6)).filter_map(|r| r.values.get("length").copied()).collect())
        .unwrap_or_default();
    out.sort_by(f64::total_cmp);
    out
}

fn key_reach(cx: &Ctx, i: usize, key: f64, top: f64) -> FitCheck {
    let (long, short) = key_arms(key);
    let frame = cx.insts[i].place;
    let r = key / 3f64.sqrt() + 0.25;
    let lo = [-r, -r, top + 0.05];
    let hi = [r, r, top + long];
    let rod = Manifold::cylinder(long - 0.05, r, r, 24, false).translate(0.0, 0.0, top + 0.05);
    let mut first: Option<(usize, f64)> = None;
    for h in near(&cx.insts, &frame, lo, hi, i) {
        let l = local(&cx.insts[h], &frame, lo, hi);
        for leaf in &l.leaves {
            let x = leaf.intersection(&rod);
            if x.is_empty() || x.volume() < 1e-4 {
                continue;
            }
            let z = x.bounding_box().map(|b| b.min()[2]).unwrap_or(top);
            if first.is_none_or(|(_, f)| z < f) {
                first = Some((h, z));
            }
        }
    }
    match first {
        None => check("Hex key", FitStatus::Pass, format!("A {} mm key reaches straight in: {} mm clear above the socket (its long arm)", mm(key), mm(long))).value(long, "mm").target(format!("{} mm", mm(long))),
        Some((h, z)) => {
            let clear = z - top;
            let moves = cx.rigid[h] != cx.rigid[i];
            let st = if moves { FitStatus::Warn } else { FitStatus::Fail };
            let how = if moves { "it moves on a joint: turn it out of the way to reach the screw" } else { "tighten it before that part goes on, or open an access hole" };
            check("Hex key", st, format!("A {} mm key is blocked by {} {} mm above the socket: {how}", mm(key), cx.insts[h].name, mm(clear))).value(clear, "mm").target(format!("{} mm (long arm) or {} mm (short)", mm(long), mm(short))).part(&cx.insts[h].name).at(cx.at(i, [0.0, 0.0, z]))
        }
    }
}

fn insert_item(cx: &mut Ctx, i: usize) -> FitItem {
    let Kind::Insert { d, len, hole, depth, wall, od } = cx.insts[i].kind.clone() else { unreachable!() };
    let frame = cx.insts[i].place;
    let name = cx.insts[i].name.clone();
    let reach = od.max(hole) / 2.0 + 8.0;
    let lo = [-reach, -reach, -len - 4.0];
    let hi = [reach, reach, 0.5];
    let mut best: Option<(usize, Vec<Sample>, Local)> = None;
    for h in near(&cx.insts, &frame, lo, hi, i) {
        if matches!(cx.insts[h].kind, Kind::Screw { .. } | Kind::Insert { .. }) {
            continue;
        }
        let l = local(&cx.insts[h], &frame, lo, hi);
        if l.is_empty() {
            continue;
        }
        let s: Vec<Sample> = depths(-0.25, -len + 0.2, 0.25).into_iter().filter_map(|z| sample(&l, z, reach)).collect();
        if s.len() > best.as_ref().map(|b| b.1.len()).unwrap_or(0) {
            best = Some((h, s, l));
        }
    }
    let at = cx.at(i, [0.0, 0.0, -len / 2.0]);
    let Some((h, samples, l)) = best else {
        return item("insert", format!("{name}: M{}×{} insert", mm(d), mm(len)), &[&cx.insts[i]], Some(at), None, vec![check("Host", FitStatus::Fail, "No printed part holds it: it sits in the air".into())]);
    };
    cx.explain(i, h);
    let host = cx.insts[h].name.clone();
    let mut checks = vec![];
    if !cx.insts[h].kind.printed() {
        checks.push(check("Host", FitStatus::Warn, format!("{host} is not printed: heat-set inserts go into thermoplastic prints")));
    }
    let open: Vec<Sample> = samples.iter().filter(|s| !s.inside).cloned().collect();
    let seg = Segment { host: h, samples: open.clone() };
    let first_solid = samples.iter().find(|s| s.inside).map(|s| -s.z);
    if open.is_empty() {
        checks.push(check("Hole", FitStatus::Fail, format!("No hole in {host} where the insert goes (print a Ø{} hole {} deep)", mm(hole), mm(depth))).part(&host));
    } else if let Some((_, dfit, off)) = seg.measure() {
        let e = dfit - hole;
        let st = if e.abs() <= 0.1 { FitStatus::Pass } else if e.abs() <= 0.25 { FitStatus::Warn } else { FitStatus::Fail };
        let why = if e < -0.1 { ": too small, the insert will split the print or not go in" } else if e > 0.1 { ": too big, the insert will pull out" } else { "" };
        checks.push(check("Insert hole", st, format!("Ø{} in {host} for the maker's Ø{}{why}", mm(dfit), mm(hole))).value(dfit, "mm").target(format!("{} ± 0.1 mm", mm(hole))).part(&host).at(at));
        let st = if off <= 0.1 { FitStatus::Pass } else if off <= 0.2 { FitStatus::Warn } else { FitStatus::Fail };
        checks.push(check("Coaxial", st, format!("The hole is {} mm off the insert's axis", mm(off))).value(off, "mm").target("≤ 0.1 mm").part(&host));
    }
    if let Some(b) = first_solid {
        checks.push(check("Hole depth", FitStatus::Fail, format!("Only {} mm deep: the {} mm insert cannot seat flush", mm(b), mm(len))).value(b, "mm").target(format!("≥ {} mm", mm(depth))).part(&host));
    } else if !open.is_empty() {
        // Depth: the hole must run on below the tip.
        let mut bottom = None;
        let mut through = true;
        for z in depths(-len - 0.1, -len - 3.5, 0.1) {
            match sample(&l, z, reach) {
                Some(s) if s.inside => {
                    bottom = Some(-z);
                    through = false;
                    break;
                }
                Some(_) => {}
                None => {
                    through = true;
                    break;
                }
            }
        }
        let (st, text) = match bottom {
            Some(b) if b + 0.05 >= depth - 0.2 => (FitStatus::Pass, format!("{} mm deep for the {} mm insert (maker: {} mm)", mm(b), mm(len), mm(depth))),
            Some(b) if b >= len => (FitStatus::Warn, format!("{} mm deep: under the maker's {} mm, the displaced plastic has little room", mm(b), mm(depth))),
            Some(b) => (FitStatus::Fail, format!("Only {} mm deep: the {} mm insert cannot seat flush", mm(b), mm(len))),
            None if through => (FitStatus::Pass, "Through hole: room under the insert".to_string()),
            None => (FitStatus::Pass, format!("More than {} mm deep", mm(len + 3.5))),
        };
        checks.push(check("Hole depth", st, text).target(format!("≥ {} mm", mm(depth))).part(&host));
        // Wall round it, from the knurl's outside.
        let thin = open.iter().map(|s| s.wall + s.radius - od / 2.0).fold(f64::INFINITY, f64::min);
        if thin.is_finite() {
            let st = if thin + 1e-6 >= cx.profile.insert_wall { FitStatus::Pass } else if thin + 1e-6 >= wall { FitStatus::Warn } else { FitStatus::Fail };
            checks.push(check("Wall", st, format!("{} mm of {host} round the Ø{} knurl (maker's minimum {} mm, profile {} mm)", mm(thin), mm(od), mm(wall), mm(cx.profile.insert_wall))).value(thin, "mm").target(format!("≥ {} mm", mm(cx.profile.insert_wall))).part(&host));
        } else {
            checks.push(check("Wall", FitStatus::Pass, format!("More than {} mm of {host} round it", mm(reach - od / 2.0))).part(&host));
        }
    }
    let joint = cx.seated_by.get(&cx.insts[i].id).cloned();
    item("insert", format!("{name}: M{}×{} insert in {host}", mm(d), mm(len)), &[&cx.insts[i], &cx.insts[h]], Some(at), joint, checks)
}

fn nut_item(cx: &mut Ctx, i: usize) -> FitItem {
    let Kind::Nut { d, s, m } = cx.insts[i].kind.clone() else { unreachable!() };
    let frame = cx.insts[i].place;
    let name = cx.insts[i].name.clone();
    let e = s / 3f64.sqrt() * 2.0;
    let reach = e / 2.0 + 5.0;
    let lo = [-reach, -reach, -1.0];
    let hi = [reach, reach, m + 0.5];
    let at = cx.at(i, [0.0, 0.0, m / 2.0]);
    let mut best: Option<(usize, Vec<Sample>, Local)> = None;
    for h in near(&cx.insts, &frame, lo, hi, i) {
        if matches!(cx.insts[h].kind, Kind::Screw { .. }) {
            continue;
        }
        let l = local(&cx.insts[h], &frame, lo, hi);
        if l.is_empty() {
            continue;
        }
        let smp: Vec<Sample> = depths(m - 0.15, 0.1, 0.25).into_iter().filter_map(|z| sample(&l, z, reach)).filter(|x| x.cover > 0.5).collect();
        if smp.len() > best.as_ref().map(|b| b.1.len()).unwrap_or(0) {
            best = Some((h, smp, l));
        }
    }
    let mut checks = vec![];
    let mut refs = vec![&cx.insts[i]];
    let joint = cx.seated_by.get(&cx.insts[i].id).cloned();
    match best {
        Some((h, smp, l)) => {
            let host = cx.insts[h].name.clone();
            let nut_solids: Vec<Manifold> = cx.insts[i].body.leaves.iter().map(|(sld, inner)| inner.solid(sld)).collect();
            let over = l.volume_with(&nut_solids);
            let rmin = smp.iter().map(|x| x.rmin).fold(f64::INFINITY, f64::min);
            let rmax = smp.iter().map(|x| x.max).fold(0.0, f64::max);
            let hex = rmax / rmin.max(1e-9) > 1.07;
            let side = rmin - s / 2.0;
            if over > 0.01 {
                checks.push(check("Nut trap", FitStatus::Fail, format!("The nut overlaps {host} by {} mm³: the pocket is too small or turned against the nut's flats", mm(over))).value(over, "mm³").part(&host).at(at));
            } else if !hex {
                checks.push(check("Nut trap", if rmin < e / 2.0 { FitStatus::Pass } else { FitStatus::Warn }, format!("Round pocket Ø{} in {host}: {}", mm(2.0 * rmin), if rmin < e / 2.0 { "holds it" } else { "the nut can turn in it — make it a hex pocket" })).value(2.0 * rmin, "mm").part(&host));
            } else {
                let st = if (0.05..=0.4).contains(&side) { FitStatus::Pass } else if side >= 0.0 { FitStatus::Warn } else { FitStatus::Fail };
                checks.push(check("Nut trap", st, format!("Hex pocket {} mm across flats in {host} for the {} mm nut: {} mm a side", mm(2.0 * rmin), mm(s), mm(side))).value(2.0 * rmin, "mm").target(format!("{}–{} mm", mm(s + 0.1), mm(s + 0.8))).part(&host).at(at));
            }
            let cover = smp.len() as f64 * 0.25 / (m - 0.25).max(0.25);
            checks.push(check("Pocket depth", if cover >= 0.8 { FitStatus::Pass } else { FitStatus::Warn }, format!("{}% of the nut's {} mm height is in the pocket", (cover.min(1.0) * 100.0).round(), mm(m))).part(&host));
            cx.explain(i, h);
            refs.push(&cx.insts[h]);
            let _ = d;
        }
        None => checks.push(check("Nut", FitStatus::Pass, "Free nut (no pocket round it): held with a spanner".into())),
    }
    item("nut", format!("{name}: M{} nut", mm(d)), &refs, Some(at), joint, checks)
}

/// A bearing's housing and shaft, or a pin's.
fn round_fit(cx: &mut Ctx, i: usize) -> FitItem {
    let (bore, outer, width, label, kind) = match cx.insts[i].kind.clone() {
        Kind::Bearing { d, outer, width, number } => (d, outer, width, number, "bearing"),
        Kind::Pin { d, len } => (0.0, d, len, format!("Ø{} pin", mm(d)), "pin"),
        _ => unreachable!(),
    };
    let frame = cx.insts[i].place;
    let name = cx.insts[i].name.clone();
    let reach = outer / 2.0 + 6.0;
    let lo = [-reach, -reach, -0.5];
    let hi = [reach, reach, width + 0.5];
    let at = cx.at(i, [0.0, 0.0, width / 2.0]);
    let mut checks = vec![];
    let mut refs = vec![&cx.insts[i]];
    let zs = depths(width - 0.25, 0.25, (width / 8.0).clamp(0.25, 1.0));
    let mut housed = false;
    let mut shafted = false;
    let others: Vec<usize> = near(&cx.insts, &frame, lo, hi, i).into_iter().filter(|h| !matches!(cx.insts[*h].kind, Kind::Screw { .. } | Kind::Insert { .. } | Kind::Washer { .. })).collect();
    for h in others {
        let l = local(&cx.insts[h], &frame, lo, hi);
        if l.is_empty() {
            continue;
        }
        let host = cx.insts[h].name.clone();
        let printed = cx.insts[h].kind.printed();
        let machined = !printed && !cx.insts[h].kind.bought();
        let rings: Vec<Ring> = zs.iter().map(|z| l.ring(*z)).collect();
        // A housing: the axis outside it, its wall round the outer ring.
        let housing: Vec<&Ring> = rings.iter().filter(|r| !r.inside && r.hits(reach) as f64 > 0.6 * RAYS as f64 && r.min() > outer / 2.0 - 0.6).collect();
        if housing.len() * 2 >= rings.len() && !housing.is_empty() {
            let fits: Vec<([f64; 2], f64)> = housing.iter().filter_map(|r| r.circle(reach)).collect();
            if fits.is_empty() {
                continue;
            }
            let dh = 2.0 * fits.iter().map(|f| f.1).sum::<f64>() / fits.len() as f64;
            let off = fits.iter().map(|f| (f.0[0].powi(2) + f.0[1].powi(2)).sqrt()).fold(0.0, f64::max);
            let dev = dh - outer;
            let (st, text) = fit_words(dev, printed, machined, outer, true, &cx.profile, &host, &label);
            checks.push(check("Housing", st, text).value(dh, "mm").target(if printed { format!("{} (press {} mm)", mm(outer - cx.profile.press), mm(cx.profile.press)) } else { format!("Ø{} H7/K7", mm(outer)) }).part(&host).at(at));
            let st = if off <= 0.05 { FitStatus::Pass } else if off <= 0.15 { FitStatus::Warn } else { FitStatus::Fail };
            checks.push(check("Coaxial", st, format!("The housing in {host} is {} mm off the {label}'s axis", mm(off))).value(off, "mm").target("≤ 0.05 mm").part(&host));
            if dev < 0.05 {
                cx.explain(i, h);
            }
            housed = true;
            refs.push(&cx.insts[h]);
            continue;
        }
        // A shaft: material at (or round) the axis inside the bore.
        if bore > 0.0 {
            let shaft: Vec<f64> = rings
                .iter()
                .filter_map(|r| {
                    let exits: Vec<f64> = (0..RAYS)
                        .filter_map(|n| {
                            let e = if r.inside { r.first[n] } else { r.second[n] };
                            (e < bore / 2.0 + 0.6 && (r.inside || r.first[n] < bore / 2.0)).then_some(e)
                        })
                        .collect();
                    (exits.len() as f64 > 0.6 * RAYS as f64).then(|| exits.iter().sum::<f64>() / exits.len() as f64)
                })
                .collect();
            if shaft.len() * 2 >= rings.len() && !shaft.is_empty() {
                let ds = 2.0 * shaft.iter().sum::<f64>() / shaft.len() as f64;
                let dev = ds - bore;
                let (st, text) = fit_words(dev, printed, machined, bore, false, &cx.profile, &host, &label);
                checks.push(check("Shaft", st, text).value(ds, "mm").target(if printed { format!("Ø{}–{}", mm(bore - 0.05), mm(bore + cx.profile.press)) } else { format!("Ø{} k6/h6", mm(bore)) }).part(&host).at(at));
                if dev > -0.05 {
                    cx.explain(i, h);
                }
                shafted = true;
                refs.push(&cx.insts[h]);
            }
        }
    }
    if !housed {
        checks.push(check("Housing", FitStatus::Fail, format!("Nothing holds the {label}'s outside: no bore round it")));
    }
    if bore > 0.0 && !shafted {
        checks.push(check("Shaft", FitStatus::Warn, format!("No shaft through the {label}'s Ø{} bore", mm(bore))));
    }
    let joint = cx.seated_by.get(&cx.insts[i].id).cloned();
    item(kind, format!("{name}: {label}"), &refs, Some(at), joint, checks)
}

/// Words for a round fit: deviation `dev` (mm) of the mating diameter from the item's.
#[allow(clippy::too_many_arguments)]
fn fit_words(dev: f64, printed: bool, machined: bool, nominal: f64, housing: bool, profile: &FitProfile, host: &str, label: &str) -> (FitStatus, String) {
    // Interference is positive for a shaft over the bore, negative for a housing under the outer ring.
    let interference = if housing { -dev } else { dev };
    let what = if housing { "housing" } else { "shaft" };
    let size = nominal + dev;
    if machined {
        return match iso286(nominal, dev, housing) {
            Some(c) => (FitStatus::Pass, format!("Ø{} {what} in {host}: ISO 286 {c} for the {label}", mm(size))),
            None => (FitStatus::Warn, format!("Ø{} {what} in {host} ({}{} mm) is outside the usual ISO 286 classes for a {label}", mm(size), if dev >= 0.0 { "+" } else { "" }, mm(dev))),
        };
    }
    if !printed {
        return (FitStatus::Pass, format!("Ø{} {what} in {host} ({} mm {})", mm(size), mm(interference.abs()), if interference > 0.0 { "interference" } else { "clearance" }));
    }
    let press = profile.press;
    let (st, words) = if interference > press + 0.1 {
        (FitStatus::Fail, format!("{} mm interference: too tight, it will crack the print (profile press {} mm)", mm(interference), mm(press)))
    } else if interference >= press * 0.4 {
        (FitStatus::Pass, format!("press fit, {} mm interference (profile {} mm)", mm(interference), mm(press)))
    } else if interference >= -0.05 {
        (FitStatus::Pass, format!("snug, {} mm {}", mm(interference.abs()), if interference >= 0.0 { "interference" } else { "clearance" }))
    } else if -interference <= 2.0 * profile.slide[1] {
        (FitStatus::Warn, format!("loose, {} mm clearance: it can creep — press it ({} mm interference) or glue it", mm(-interference), mm(press)))
    } else {
        (FitStatus::Fail, format!("{} mm clearance: far too loose", mm(-interference)))
    };
    (st, format!("Ø{} printed {what} in {host}: {words}", mm(size)))
}

fn washer_item(cx: &mut Ctx, i: usize) -> FitItem {
    let Kind::Washer { d } = cx.insts[i].kind.clone() else { unreachable!() };
    let name = cx.insts[i].name.clone();
    let at = cx.at(i, [0.0; 3]);
    // Explained with whatever it is clamped between; overlap is still looked for.
    let mut checks = vec![];
    let mine: Vec<Manifold> = cx.insts[i].world.iter().map(|w| w.0.clone()).collect();
    let mut over = vec![];
    for j in 0..cx.insts.len() {
        if j == i || matches!(cx.insts[j].kind, Kind::Screw { .. }) || !boxes_touch(&cx.insts[i], &cx.insts[j], 0.0) {
            continue;
        }
        let v: f64 = mine.iter().map(|a| cx.insts[j].world.iter().map(|b| a.intersection(&b.0).volume()).sum::<f64>()).sum();
        if v > 0.01 {
            over.push(format!("{} ({} mm³)", cx.insts[j].name, mm(v)));
        }
    }
    if over.is_empty() {
        checks.push(check("Washer", FitStatus::Pass, format!("M{} washer clear of everything round it", mm(d))));
    } else {
        checks.push(check("Washer", FitStatus::Fail, format!("Overlaps {}", over.join(", "))));
    }
    item("washer", format!("{name}: M{} washer", mm(d)), &[&cx.insts[i]], Some(at), cx.seated_by.get(&cx.insts[i].id).cloned(), checks)
}

fn boxes_touch(a: &Inst, b: &Inst, pad: f64) -> bool {
    (0..3).all(|k| a.lo[k] <= b.hi[k] + pad && b.lo[k] <= a.hi[k] + pad)
}

fn overlap(a: &Inst, b: &Inst) -> Option<(f64, V3, V3)> {
    let mut v = 0.0;
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for (x, xl, xh) in &a.world {
        for (y, yl, yh) in &b.world {
            if (0..3).any(|k| xh[k] < yl[k] || yh[k] < xl[k]) {
                continue;
            }
            let m = x.intersection(y);
            if m.is_empty() {
                continue;
            }
            let vol = m.volume();
            if vol <= 0.0 {
                continue;
            }
            v += vol;
            if let Some(bb) = m.bounding_box() {
                for k in 0..3 {
                    lo[k] = lo[k].min(bb.min()[k]);
                    hi[k] = hi[k].max(bb.max()[k]);
                }
            }
        }
    }
    (v > 0.0).then_some((v, lo, hi))
}

fn min_gap(a: &Inst, b: &Inst, shift: V3, search: f64) -> f64 {
    let mut g = f64::INFINITY;
    for (x, xl, xh) in &a.world {
        for (y, yl, yh) in &b.world {
            let apart = (0..3).map(|k| ((yl[k] + shift[k]) - xh[k]).max(xl[k] - (yh[k] + shift[k])).max(0.0).powi(2)).sum::<f64>().sqrt();
            if apart >= search {
                g = g.min(search);
                continue;
            }
            let yy = if shift == [0.0; 3] { y.clone() } else { y.translate(shift[0], shift[1], shift[2]) };
            g = g.min(x.min_gap(&yy, search));
        }
    }
    g
}

/// Two instances a joint joins (and no item covers): overlap, and the side clearance once seated.
fn joined_item(cx: &mut Ctx, joint: StableId, j: &joints::Joint, a: usize, b: usize) -> Option<FitItem> {
    let jname = joints::joint_name(cx.world, joint);
    let (ia, ib) = (&cx.insts[a], &cx.insts[b]);
    if ia.world.is_empty() || ib.world.is_empty() {
        return None;
    }
    let printed = ia.kind.printed() || ib.kind.printed();
    let mut checks = vec![];
    let moving = j.kind != JointKind::Fixed;
    if let Some((v, lo, hi)) = overlap(ia, ib) {
        if v > 0.01 {
            let at = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
            checks.push(check("Overlap", FitStatus::Fail, format!("{} and {} overlap by {} mm³ ({} × {} × {} mm)", ia.name, ib.name, mm(v), mm(hi[0] - lo[0]), mm(hi[1] - lo[1]), mm(hi[2] - lo[2]))).value(v, "mm³").at(at));
        }
    }
    cx.explain(a, b);
    if checks.is_empty() {
        let search = 3.0;
        let g0 = min_gap(ia, ib, [0.0; 3], search);
        if g0 > 1e-3 {
            let need = if moving { cx.profile.slide[0] } else { 0.0 };
            let st = if !printed || g0 + 1e-6 >= need { FitStatus::Pass } else { FitStatus::Warn };
            let words = if g0 >= search { format!("more than {} mm apart", mm(search)) } else { format!("{} mm apart", mm(g0)) };
            checks.push(check("Clearance", st, format!("{} and {}: {words}{}", ia.name, ib.name, if moving && printed && st == FitStatus::Warn { format!(" (moving printed parts want {} mm)", mm(need)) } else { String::new() })).value(g0.min(search), "mm"));
        } else {
            // Seated face to face: lift it off its seat to see the clearance round the sides.
            let ca = match &j.a {
                joints::End::Connector { instance, connector } => joints::connectors(cx.world, *instance).ok().and_then(|cs| cs.into_iter().find(|(n, _)| n == connector)).map(|(_, f)| f),
                _ => None,
            };
            let axis = match (&j.a, ca) {
                (joints::End::Connector { instance, .. }, Some(f)) if *instance == ia.id => ia.place.turn(f.turn([0.0, 0.0, 1.0])),
                _ => [0.0, 0.0, 1.0],
            };
            let lift = 0.5;
            let shift = axis.map(|c| c * lift);
            let g1 = min_gap(ia, ib, shift, search).min(min_gap(ia, ib, shift.map(|c| -c), search));
            let side_words = if g1 >= lift - 0.01 { format!("≥ {} mm", mm(lift)) } else { format!("{} mm", mm(g1)) };
            let want = if moving { cx.profile.slide[0] } else { 0.1 };
            let st = if g1 + 1e-6 >= want || !printed { FitStatus::Pass } else if g1 > 1e-3 { FitStatus::Warn } else { FitStatus::Warn };
            checks.push(check("Seated", FitStatus::Pass, format!("{} seats on {} face to face, no overlap", ib.name, ia.name)));
            checks.push(check("Side clearance", st, format!("{side_words} round the sides once seated{}", if st == FitStatus::Warn { format!(" (printed fits want {} mm)", mm(want)) } else { String::new() })).value(g1.min(lift), "mm").target(format!("≥ {} mm", mm(want))));
        }
    }
    let kind_word = j.kind.word();
    Some(item("joined", format!("{} ↔ {} ({kind_word} “{jname}”)", ia.name, ib.name), &[ia, ib], None, Some(jname), checks))
}

fn motion_item(cx: &Ctx, joint: StableId, steps: usize) -> FitItem {
    let jname = joints::joint_name(cx.world, joint);
    match motion::sweep(cx.world, joint, None, None, None, steps, &HashMap::new(), true) {
        Ok(s) => {
            let unit = |v: f64| format!("{}{}", mm(v), if s.unit == "°" { "°" } else { " mm" });
            let mut checks = vec![];
            for c in &s.clashes {
                let [x, y] = c.range.unwrap_or([c.at, c.at]);
                checks.push(check("Clash", FitStatus::Fail, format!("{} hits {} from {} to {}", c.a, c.b, unit(x), unit(y))));
            }
            if let Some(v) = s.binds_at {
                checks.push(check("Binds", FitStatus::Fail, format!("The joints cannot follow past {}", unit(v))));
            }
            if checks.is_empty() {
                let near = s.pairs.iter().find(|p| !p.joined);
                let text = match near {
                    Some(p) => format!("Clear through {}–{}; closest {} mm ({} – {} at {})", unit(s.from), unit(s.to), mm(p.gap), p.a, p.b, unit(p.at)),
                    None => format!("Clear through {}–{}; nothing within {} mm", unit(s.from), unit(s.to), mm(s.search)),
                };
                checks.push(check("Motion", FitStatus::Pass, text).value(near.map(|p| p.gap).unwrap_or(s.search), "mm"));
            }
            let ids: Vec<&Inst> = s.clashes.iter().flat_map(|c| [c.a_id, c.b_id]).filter_map(|id| cx.insts.iter().find(|i| i.id == id)).collect();
            item("motion", format!("{jname}: {}–{}", unit(s.from), unit(s.to)), &ids, None, Some(jname), checks)
        }
        Err(e) => item("motion", format!("{jname}: not swept"), &[], None, Some(jname), vec![check("Motion", FitStatus::Warn, e)]),
    }
}

/// The fit report for an assembly definition (`Some`) or the top of the project.
pub fn report(world: &World, scope: Option<StableId>, options: &FitOptions) -> Result<FitReport, String> {
    let name = match scope {
        Some(a) => entity(world, a).and_then(|e| world.get::<crate::AssemblyDef>(e)).map(|d| d.name.clone()).ok_or_else(|| format!("{a} is not an assembly"))?,
        None => "Project".to_string(),
    };
    let mut ids: Vec<(StableId, Instance)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.parent == scope).map(|i| (id, i.clone()))).collect();
    ids.sort_by_key(|(id, _)| *id);
    let mut insts = vec![];
    let mut notes = vec![];
    for (id, inst) in &ids {
        let e = entity(world, *id).ok_or("an instance is gone")?;
        let place = placement(world, e).unwrap_or_else(|| inst.transform());
        let body = match motion::body(world, *id) {
            Ok(b) => b,
            Err(m) => {
                notes.push(format!("{}: {m}", inst.name));
                continue;
            }
        };
        let mut w = vec![];
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for (s, inner) in &body.leaves {
            let t = place.then(inner);
            let m = t.solid(s);
            if let Some(bb) = m.bounding_box() {
                let (a, b) = (bb.min(), bb.max());
                for k in 0..3 {
                    lo[k] = lo[k].min(a[k]);
                    hi[k] = hi[k].max(b[k]);
                }
                w.push((m, a, b));
            }
        }
        insts.push(Inst { id: *id, name: inst.name.clone(), kind: kind_of(world, *id, inst), place, body, world: w, lo, hi });
    }
    // Rigid groups: union of instances held by fixed joints (the ground is its own member).
    let index: HashMap<StableId, usize> = insts.iter().enumerate().map(|(n, i)| (i.id, n)).collect();
    let mut parent: Vec<usize> = (0..=insts.len()).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut y = x;
        while p[y] != r {
            let n = p[y];
            p[y] = r;
            y = n;
        }
        r
    }
    let ground = insts.len();
    let all_joints = joints::joints_in(world, scope);
    let mut seated_by = HashMap::new();
    for (jid, j) in &all_joints {
        let end = |e: &joints::End| e.instance().and_then(|i| index.get(&i).copied()).unwrap_or(ground);
        if let Some(b) = j.b.instance() {
            seated_by.entry(b).or_insert_with(|| joints::joint_name(world, *jid));
        }
        if j.kind == JointKind::Fixed {
            let (x, y) = (find(&mut parent, end(&j.a)), find(&mut parent, end(&j.b)));
            parent[x] = y;
        }
    }
    // Instances no joint holds stay where they are: with the ground.
    let held: HashSet<StableId> = all_joints.iter().filter_map(|(_, j)| j.b.instance()).collect();
    for (n, i) in insts.iter().enumerate() {
        if !held.contains(&i.id) {
            let (x, y) = (find(&mut parent, n), find(&mut parent, ground));
            parent[x] = y;
        }
    }
    let rigid: Vec<usize> = (0..insts.len()).map(|n| find(&mut parent, n)).collect();
    let mut cx = Ctx { world, insts, profile: profile(world), rigid, seated_by, explained: Default::default() };
    let mut items = vec![];
    for n in 0..cx.insts.len() {
        let it = match cx.insts[n].kind {
            Kind::Screw { .. } => screw_item(&mut cx, n),
            Kind::Insert { .. } => insert_item(&mut cx, n),
            Kind::Nut { .. } => nut_item(&mut cx, n),
            Kind::Bearing { .. } | Kind::Pin { .. } => round_fit(&mut cx, n),
            Kind::Washer { .. } => washer_item(&mut cx, n),
            _ => continue,
        };
        items.push(it);
    }
    // Pairs a joint joins directly, where no item above speaks for them.
    for (jid, j) in &all_joints {
        let (Some(a), Some(b)) = (j.a.instance().and_then(|i| index.get(&i).copied()), j.b.instance().and_then(|i| index.get(&i).copied())) else { continue };
        if cx.insts[a].kind.has_item() || cx.insts[b].kind.has_item() {
            continue;
        }
        if let Some(it) = joined_item(&mut cx, *jid, j, a, b) {
            items.push(it);
        }
    }
    // Interference at rest between anything else.
    for a in 0..cx.insts.len() {
        for b in a + 1..cx.insts.len() {
            if cx.explained.borrow().contains(&(a, b)) || !boxes_touch(&cx.insts[a], &cx.insts[b], 0.0) {
                continue;
            }
            // A screw is judged by its own item against everything it passes.
            if matches!(cx.insts[a].kind, Kind::Screw { .. }) || matches!(cx.insts[b].kind, Kind::Screw { .. }) {
                continue;
            }
            if let Some((v, lo, hi)) = overlap(&cx.insts[a], &cx.insts[b])
                && v > 0.01
            {
                let at = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
                let (ia, ib) = (&cx.insts[a], &cx.insts[b]);
                items.push(item(
                    "interference",
                    format!("{} overlaps {}", ia.name, ib.name),
                    &[ia, ib],
                    Some(at),
                    None,
                    vec![check("Interference", FitStatus::Fail, format!("{} mm³ of {} is inside {} ({} × {} × {} mm)", mm(v), ia.name, ib.name, mm(hi[0] - lo[0]), mm(hi[1] - lo[1]), mm(hi[2] - lo[2]))).value(v, "mm³").at(at)],
                ));
            }
        }
    }
    if options.motion {
        for (jid, j) in &all_joints {
            if j.kind != JointKind::Fixed {
                items.push(motion_item(&cx, *jid, options.steps));
            }
        }
    }
    let count = |s: FitStatus| items.iter().filter(|i| i.status == s).count();
    let (pass, warn, fail) = (count(FitStatus::Pass), count(FitStatus::Warn), count(FitStatus::Fail));
    // Worst first, then in the order found.
    items.sort_by_key(|i| std::cmp::Reverse(i.status));
    if items.is_empty() {
        notes.push("Nothing to check: no stock fasteners, bearings or joints here.".into());
    }
    Ok(FitReport { assembly: scope, name, profile: cx.profile.clone(), items, pass, warn, fail, ok: fail == 0, notes })
}
