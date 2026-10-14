//! Design intent plugin.
//!
//! A requirement is an entity in the shared project graph that depends on the
//! parts it constrains. The engine's incremental recompute therefore
//! re-verifies exactly the requirements an edit touches, on every change, by
//! exact math. A failing requirement carries a computed fix: a real engine
//! command that would make it pass.

pub mod check;
pub mod geometry;

use de_core::{
    Command, CommandError, Derived, ErrorCode, Length, Mass, Persist, Plugin, Registry, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_solid::BoxFeature;
use geometry::{EPS, PartView, fmt, name, part};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use check::{CheckReport, Finding, Severity, check};

pub struct IntentPlugin;

impl Plugin for IntentPlugin {
    fn name(&self) -> &'static str {
        "intent"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<Requirement>()
            .command::<CreateRequirement>()
            .command::<UpdateRequirement>()
            .derived(Derived {
                name: "intent.evaluate",
                inputs: &[Requirement::KIND],
                reads_dependencies: true,
                compute: evaluate,
            });
    }
}

/// Something the design must satisfy.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Requirement {
    pub name: String,
    pub rule: Rule,
}

impl Persist for Requirement {
    const KIND: &'static str = "intent.requirement";
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rule {
    /// Total mass of the parts must not exceed `limit`.
    MaxMass { parts: Vec<StableId>, limit: Mass },
    /// The parts' combined bounding box must fit inside the envelope.
    FitsWithin { parts: Vec<StableId>, width: Length, depth: Length, height: Length },
    /// Parts `a` and `b` must stay at least `min` apart.
    Clearance { a: StableId, b: StableId, min: Length },
    /// No two of the parts may overlap.
    NoCollisions { parts: Vec<StableId> },
    /// Nothing touches while an assembly joint moves through its range (its
    /// limits, or a full turn): every pair of instances that move relative to
    /// each other is watched. `parts` are what it covers: the joint's
    /// assembly (the default) or some of its instances. Pairs the joint
    /// itself joins may touch.
    NoClashesInMotion {
        joint: StableId,
        #[serde(default)]
        parts: Vec<StableId>,
        /// Positions checked through the range (default 72).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<u32>,
        /// Other joints held at these values (joint id → value) while it
        /// sweeps, instead of where they are now: the worst pose to check
        /// (a shoulder low while the elbow folds).
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        set: BTreeMap<StableId, f64>,
        /// Joints stepped through their own limits too (`across_steps` + 1
        /// values each, default 4 steps): the sweep is repeated at every
        /// combination, covering the joint space as the interlocks allow it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        across: Vec<StableId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        across_steps: Option<u32>,
    },
    /// Everything assembled in `assembly` (an assembly definition, or an
    /// instance of one) actually fits: every stock fastener in its holes
    /// (clearance, thread engagement, length, head seat, hex key reach),
    /// every insert, nut trap and bearing fit, joined parts seated without
    /// overlap, nothing interfering at rest, and (with `motion`, the default)
    /// nothing clashing as each joint moves through its limits. Fails on
    /// any failing item of the fit report; warnings pass.
    AssemblyFits {
        assembly: StableId,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        motion: bool,
        /// Positions per joint sweep (default 36).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<u32>,
    },
    /// The shape's saved stress study (for `part`, or the whole solid) keeps
    /// it at least `min` times from yielding (yield strength over the
    /// largest von Mises stress), re-solved whenever the shape, its study
    /// or its materials change.
    MinSafetyFactor {
        shape: StableId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        min: f64,
    },
    /// Under its saved study the part moves at most `max` anywhere.
    MaxDeflection {
        shape: StableId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        max: Length,
    },
    /// Held by its saved study's supports, the part's first natural
    /// frequency is at least `min` Hz (prestressed when the study says so).
    MinFirstFrequency {
        shape: StableId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        min: f64,
    },
    /// The saved study's loads times `min` do not buckle the part (linear buckling).
    MinBucklingFactor {
        shape: StableId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        min: f64,
    },
    /// The made part matches its CAD model: the scan's points (`scan`, a
    /// stored scan's id) lie within ±`tolerance` of the part's surface — all
    /// of them, or the share `within` (0…1) — after a best-fit alignment when
    /// `align` is set. Re-checked whenever the part changes.
    ScanDeviation {
        part: StableId,
        /// One named part of a multi-part shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part_name: Option<String>,
        scan: String,
        tolerance: Length,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        within: Option<f64>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        align: bool,
    },
    /// The model matches a reference picture: seen square to the canvas, the
    /// part's silhouette overlaps the photo's object by at least `min_iou`
    /// (intersection over union, 0…1) and, when `max_deviation` is given, no
    /// region of extra or missing material is thicker than that. Re-checked
    /// whenever the part or the canvas changes.
    SilhouetteMatch {
        part: StableId,
        canvas: StableId,
        min_iou: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_deviation: Option<Length>,
    },
    /// A circuit board meets its routing constraints: none of its design-rule
    /// violations of these `codes` (default every routing-constraint code:
    /// impedance, diff_pair_gap, diff_pair_uncoupled, length, skew, via_count;
    /// "all" for the whole DRC) — from its net classes, pairs, length groups
    /// and custom rules. Re-checked on every change to the board.
    PcbConstraints {
        board: StableId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        codes: Vec<String>,
    },
    /// Nets of a board (names, patterns like `D?`, or pair names) are each
    /// `target` ± `tolerance` long, or between `min` and `max`, and within
    /// `max_skew` of each other (mm, copper length with vias and pad-to-die).
    /// A parameter (a value of a shape, sketch or parameter set, or a
    /// formula's result: a torque margin worked out from a joint's angle and
    /// a servo's stall torque, a spring's longest stretch) stays at least
    /// `min` and/or at most `max`. Re-checked whenever it, or anything its
    /// formula reads, changes.
    ParameterRange {
        target: StableId,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    PcbLength {
        board: StableId,
        nets: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tolerance: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_skew: Option<f64>,
    },
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

impl Rule {
    pub fn targets(&self) -> Vec<StableId> {
        match self {
            Rule::MaxMass { parts, .. } | Rule::FitsWithin { parts, .. } | Rule::NoCollisions { parts } => parts.clone(),
            Rule::Clearance { a, b, .. } => vec![*a, *b],
            Rule::NoClashesInMotion { joint, parts, set, across, .. } => std::iter::once(*joint).chain(parts.iter().copied()).chain(set.keys().copied()).chain(across.iter().copied()).collect(),
            Rule::AssemblyFits { assembly, .. } => vec![*assembly],
            Rule::MinSafetyFactor { shape, .. } | Rule::MaxDeflection { shape, .. } | Rule::MinFirstFrequency { shape, .. } | Rule::MinBucklingFactor { shape, .. } => vec![*shape],
            Rule::ScanDeviation { part, .. } => vec![*part],
            Rule::SilhouetteMatch { part, canvas, .. } => vec![*part, *canvas],
            Rule::PcbConstraints { board, .. } | Rule::PcbLength { board, .. } => vec![*board],
            Rule::ParameterRange { target, .. } => vec![*target],
        }
    }

    fn default_name(&self) -> String {
        let mm = |l: &Length| fmt(l.canonical() * 1000.0);
        match self {
            Rule::MaxMass { limit, .. } => format!("Mass at most {}", format_mass(limit.canonical())),
            Rule::FitsWithin { width, depth, height, .. } => format!("Fits within {} × {} × {} mm", mm(width), mm(depth), mm(height)),
            Rule::Clearance { min, .. } => format!("Clearance at least {} mm", mm(min)),
            Rule::NoCollisions { .. } => "No collisions".into(),
            Rule::NoClashesInMotion { .. } => "No clashes through the motion".into(),
            Rule::AssemblyFits { .. } => "Everything fits".into(),
            Rule::MinSafetyFactor { min, .. } => format!("Safety factor at least {}", fmt(*min)),
            Rule::MaxDeflection { max, .. } => format!("Deflection at most {} mm", mm(max)),
            Rule::MinFirstFrequency { min, .. } => format!("First frequency at least {} Hz", fmt(*min)),
            Rule::MinBucklingFactor { min, .. } => format!("Buckling factor at least {}", fmt(*min)),
            Rule::ScanDeviation { tolerance, .. } => format!("Matches its scan within ±{} mm", mm(tolerance)),
            Rule::SilhouetteMatch { min_iou, .. } => format!("Matches its reference picture (IoU ≥ {})", fmt(*min_iou)),
            Rule::PcbConstraints { codes, .. } => if codes.is_empty() { "Board meets its routing constraints".into() } else { format!("Board clean of {}", codes.join(", ")) },
            Rule::ParameterRange { key, min, max, .. } => match (min, max) {
                (Some(a), Some(b)) => format!("{key} between {} and {}", fmt(*a), fmt(*b)),
                (Some(a), None) => format!("{key} at least {}", fmt(*a)),
                (None, Some(b)) => format!("{key} at most {}", fmt(*b)),
                (None, None) => key.clone(),
            },
            Rule::PcbLength { nets, target, tolerance, max_skew, .. } => match (target, max_skew) {
                (Some(t), _) => format!("{} {} ± {} mm long", nets.join(", "), fmt(*t), fmt(tolerance.unwrap_or(t * 0.01))),
                (None, Some(s)) => format!("{} matched within {} mm", nets.join(", "), fmt(*s)),
                _ => format!("{} within length limits", nets.join(", ")),
            },
        }
    }
}

/// A single-command fix the UI or agent can apply.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Fix {
    pub label: String,
    pub operation: String,
    pub parameters: Value,
}

/// Where a violation is, for highlighting.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Violation {
    pub parts: Vec<StableId>,
    /// Names of the colliding items (assembly parts are named "Assembly › Part").
    #[serde(default)]
    pub names: Vec<String>,
    /// Overlapping region in project millimeters: [min x, y, z, max x, y, z].
    pub region: Option<[f64; 6]>,
}

/// Engine verdict for a requirement (derived, never edited).
#[derive(Component, Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct RequirementResult {
    pub passed: bool,
    /// Measured value and limit in `unit`, for gauges.
    pub measured: f64,
    pub limit: f64,
    pub unit: String,
    /// `true` when measured must stay at or below limit; `false` for minimums.
    pub upper_bound: bool,
    pub summary: String,
    pub violations: Vec<Violation>,
    pub fix: Option<Fix>,
    /// For checks on AI-made shapes: how closely their formulas were evaluated (mm).
    pub accuracy_mm: Option<f64>,
}

/// Small gaps keep three decimals: 0.0499 mm → "0.05", 0.004 → "0.004".
fn fmt_gap(mm: f64) -> String {
    if mm < 0.1 { format!("{}", (mm * 1000.0).round() / 1000.0) } else { fmt(mm) }
}

fn fmt_volume(mm3: f64) -> String {
    if mm3 < 1.0 { format!("{}", (mm3 * 1000.0).round() / 1000.0) } else { fmt(mm3) }
}

fn format_mass(kg: f64) -> String {
    if kg >= 1.0 { format!("{} kg", fmt(kg)) } else { format!("{} g", fmt(kg * 1000.0)) }
}

fn mm_q(v: f64) -> Value {
    json!({ "value": (v * 100.0).round() / 100.0, "unit": "mm" })
}

fn move_fix(p: &PartView, axis: usize, delta: f64, why: &str) -> Fix {
    let mut to = p.placement_mm;
    to[axis] += delta;
    let dir = ["X", "Y", "Z"][axis];
    Fix {
        label: format!("Move {} {}{} mm along {dir} {why}", name(p), if delta >= 0.0 { "+" } else { "" }, fmt(delta)),
        operation: "solid.part.move".into(),
        parameters: json!({ "target": p.id, "placement": { "x": mm_q(to[0]), "y": mm_q(to[1]), "z": mm_q(to[2]) } }),
    }
}

fn parts(world: &World, ids: &[StableId]) -> Result<Vec<PartView>, String> {
    ids.iter().map(|id| part(world, *id)).collect()
}

fn evaluate(world: &World, entity: Entity) -> Option<Result<RequirementResult, String>> {
    let req = world.get::<Requirement>(entity)?;
    Some(match &req.rule {
        Rule::MaxMass { parts: ids, limit } => parts(world, ids).and_then(|ps| max_mass(&ps, limit.canonical())),
        Rule::FitsWithin { parts: ids, width, depth, height } => parts(world, ids).map(|ps| {
            fits_within(&ps, [width, depth, height].map(|l| l.canonical() * 1000.0))
        }),
        Rule::Clearance { a, b, min } => {
            part(world, *a).and_then(|a| part(world, *b).map(|b| clearance(&a, &b, min.canonical() * 1000.0)))
        }
        // Assemblies are checked part against part, inside and out.
        Rule::NoCollisions { parts: ids } => parts(world, ids).map(|ps| no_collisions(world, &geometry::expand(ps))),
        Rule::NoClashesInMotion { joint, parts: ids, steps, set, across, across_steps } => clear_in_motion(world, *joint, ids, steps.unwrap_or(72) as usize, set, across, across_steps.unwrap_or(4) as usize),
        Rule::AssemblyFits { assembly, motion, steps } => assembly_fits(world, *assembly, *motion, steps.unwrap_or(36) as usize),
        Rule::MinSafetyFactor { shape, part, min } => study_check(world, *shape, part.as_deref(), de_shape::studies::Check::Safety, *min, false, ""),
        Rule::MaxDeflection { shape, part, max } => study_check(world, *shape, part.as_deref(), de_shape::studies::Check::Deflection, max.canonical() * 1000.0, true, "mm"),
        Rule::MinFirstFrequency { shape, part, min } => study_check(world, *shape, part.as_deref(), de_shape::studies::Check::Frequency, *min, false, "Hz"),
        Rule::ScanDeviation { part: id, part_name, scan, tolerance, within, align } => scan_check(world, *id, part_name.as_deref(), scan, tolerance.canonical() * 1000.0, within.unwrap_or(1.0), *align),
        Rule::SilhouetteMatch { part: id, canvas, min_iou, max_deviation } => silhouette_check(world, *id, *canvas, *min_iou, max_deviation.as_ref().map(|l| l.canonical() * 1000.0)),
        Rule::MinBucklingFactor { shape, part, min } => study_check(world, *shape, part.as_deref(), de_shape::studies::Check::Buckling, *min, false, ""),
        Rule::PcbConstraints { board, codes } => pcb_constraints(world, *board, codes),
        Rule::PcbLength { board, nets, target, tolerance, min, max, max_skew } => pcb_length(world, *board, nets, *target, *tolerance, *min, *max, *max_skew),
        Rule::ParameterRange { target, key, min, max } => parameter_range(world, *target, key, *min, *max),
    })
}

/// A parameter's value now (a formula worked out) and its label and unit.
fn parameter_now(world: &World, id: StableId, key: &str) -> Result<(f64, String, String, String), String> {
    let e = world.resource::<de_core::IdIndex>().get(id).ok_or_else(|| format!("{id} does not exist"))?;
    // A spreadsheet: a cell by address or alias.
    if let Some(sheet) = world.get::<de_shape::sheet::Sheet>(e) {
        let found = sheet.cells.iter().find(|(a, c)| a.as_str() == key || c.alias.as_deref() == Some(key));
        let Some((address, cell)) = found.filter(|(_, c)| !c.input.is_empty()) else {
            return Err(format!("{} has no cell or alias “{key}”", sheet.name));
        };
        let value = match world.get::<de_shape::ResolvedParams>(e).and_then(|r| r.0.get(key).copied()) {
            Some(v) => v,
            None => *de_shape::sheet::evaluate(world, sheet).numbers().get(key).ok_or_else(|| format!("{} › {key} is not a number", sheet.name))?,
        };
        let label = if address == key { key.to_string() } else { format!("{key} ({address})") };
        return Ok((value, sheet.name.clone(), label, cell.unit.clone().unwrap_or_default()));
    }
    let (name, params) = de_shape::params::params_of(world, e).ok_or_else(|| format!("{id} has no parameters"))?;
    let p = params.iter().find(|p| p.key == key).ok_or_else(|| {
        let keys: Vec<&str> = params.iter().map(|p| p.key.as_str()).collect();
        format!("{name} has no parameter “{key}” (it has {})", keys.join(", "))
    })?;
    let value = match world.get::<de_shape::ResolvedParams>(e).and_then(|r| r.0.get(key).copied()) {
        Some(v) => v,
        None => *de_shape::params::resolve(world, params)?.get(key).ok_or_else(|| format!("{name}: “{key}” has no value"))?,
    };
    Ok((value, name.to_string(), if p.label.is_empty() { key.to_string() } else { p.label.clone() }, p.unit.clone()))
}

fn parameter_range(world: &World, id: StableId, key: &str, min: Option<f64>, max: Option<f64>) -> Result<RequirementResult, String> {
    let (v, name, label, unit) = parameter_now(world, id, key)?;
    let low = min.is_some_and(|m| v < m - 1e-9);
    let high = max.is_some_and(|m| v > m + 1e-9);
    let passed = !low && !high;
    let u = if unit.is_empty() { String::new() } else { format!(" {unit}") };
    let bound = match (min, max) {
        (Some(a), Some(b)) => format!("between {}{u} and {}{u}", fmt(a), fmt(b)),
        (Some(a), None) => format!("at least {}{u}", fmt(a)),
        (None, Some(b)) => format!("at most {}{u}", fmt(b)),
        (None, None) => String::new(),
    };
    Ok(RequirementResult {
        passed,
        measured: v,
        limit: if high || min.is_none() { max.unwrap_or(v) } else { min.unwrap_or(v) },
        unit,
        upper_bound: high || min.is_none(),
        summary: format!("{name} › {label} is {}{u}, {bound}", fmt(v)),
        violations: if passed { vec![] } else { vec![Violation { parts: vec![id], names: vec![name], region: None }] },
        fix: None,
        accuracy_mm: None,
    })
}

/// The board as worked out now (its derived result, else resolved on the spot).
fn board_view(world: &World, id: StableId) -> Result<std::sync::Arc<de_pcb::BoardView>, String> {
    if let Some(v) = de_pcb::view(world, id) {
        return Ok(v);
    }
    let e = world.resource::<de_core::IdIndex>().get(id).ok_or_else(|| format!("{id} does not exist"))?;
    let b = world.get::<de_pcb::Board>(e).ok_or_else(|| format!("{id} is not a circuit board"))?;
    de_pcb::board::resolve(world, id, b).map(std::sync::Arc::new)
}

pub const ROUTING_CODES: [&str; 6] = ["impedance", "diff_pair_gap", "diff_pair_uncoupled", "length", "skew", "via_count"];

fn pcb_constraints(world: &World, id: StableId, codes: &[String]) -> Result<RequirementResult, String> {
    let v = board_view(world, id)?;
    let all = codes.iter().any(|c| c == "all");
    let hits: Vec<&de_pcb::board::Violation> = v
        .violations
        .iter()
        .filter(|x| x.severity == "error")
        .filter(|x| if all { true } else if codes.is_empty() { ROUTING_CODES.contains(&x.code) } else { codes.iter().any(|c| c == x.code) })
        .collect();
    let passed = hits.is_empty();
    let s = &v.signal;
    let mut facts = Vec::new();
    for p in &s.pairs {
        facts.push(format!("{} skew {:.3} mm, uncoupled {:.2} mm{}", p.name, p.skew, p.uncoupled, p.zdiff.map(|z| format!(", {z} Ω")).unwrap_or_default()));
    }
    for g in &s.groups {
        facts.push(format!("{} spread {:.3} mm", g.name, g.skew));
    }
    let summary = if passed {
        format!("no violations{}", if facts.is_empty() { String::new() } else { format!(" ({})", facts.join("; ")) })
    } else {
        format!("{} violation{}: {}", hits.len(), if hits.len() == 1 { "" } else { "s" }, hits.iter().take(4).map(|x| x.message.clone()).collect::<Vec<_>>().join(" · "))
    };
    Ok(RequirementResult {
        passed,
        measured: hits.len() as f64,
        limit: 0.0,
        unit: "violations".into(),
        upper_bound: true,
        summary,
        violations: if passed { vec![] } else { vec![Violation { parts: vec![id], names: hits.iter().flat_map(|x| x.refs.clone()).take(12).collect(), region: None }] },
        fix: None,
        accuracy_mm: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn pcb_length(world: &World, id: StableId, nets: &[String], target: Option<f64>, tolerance: Option<f64>, min: Option<f64>, max: Option<f64>, max_skew: Option<f64>) -> Result<RequirementResult, String> {
    let v = board_view(world, id)?;
    let all: Vec<String> = v.nets.iter().map(|n| n.name.clone()).collect();
    let group = de_pcb::signal::LengthGroup { name: String::new(), nets: nets.to_vec(), target: None, tolerance: None, min: None, max: None, max_skew: None };
    let members = de_pcb::signal::members(&group, &all, &v.signal.pairs);
    if members.is_empty() {
        return Err(format!("no net on the board matches {}", nets.join(", ")));
    }
    let len = |n: &str| v.signal.lengths.iter().find(|l| l.net == n).map(|l| l.length).unwrap_or(0.0);
    let ls: Vec<(String, f64)> = members.iter().map(|n| (n.clone(), len(n))).collect();
    let mut worst = 0.0f64;
    let mut limit = 0.0;
    let mut bad = Vec::new();
    if let Some(t) = target {
        let tol = tolerance.unwrap_or(t * 0.01);
        limit = tol;
        for (n, l) in &ls {
            let e = (l - t).abs();
            worst = worst.max(e);
            if e > tol + 1e-6 {
                bad.push(format!("{n} {l:.3} mm"));
            }
        }
    }
    for (n, l) in &ls {
        if min.is_some_and(|m| *l + 1e-6 < m) || max.is_some_and(|m| *l > m + 1e-6) {
            bad.push(format!("{n} {l:.3} mm"));
        }
    }
    let hi = ls.iter().map(|x| x.1).fold(f64::MIN, f64::max);
    let lo = ls.iter().map(|x| x.1).fold(f64::MAX, f64::min);
    if let Some(s) = max_skew {
        if target.is_none() {
            worst = hi - lo;
            limit = s;
        }
        if hi - lo > s + 1e-6 {
            bad.push(format!("spread {:.3} mm", hi - lo));
        }
    }
    let passed = bad.is_empty();
    Ok(RequirementResult {
        passed,
        measured: worst,
        limit,
        unit: "mm".into(),
        upper_bound: true,
        summary: format!("{} (spread {:.3} mm){}", ls.iter().map(|(n, l)| format!("{n} {l:.3}")).collect::<Vec<_>>().join(", "), hi - lo, if passed { String::new() } else { format!("; off: {}", bad.join(", ")) }),
        violations: if passed { vec![] } else { vec![Violation { parts: vec![id], names: members, region: None }] },
        fix: None,
        accuracy_mm: None,
    })
}

fn max_mass(ps: &[PartView], limit: f64) -> Result<RequirementResult, String> {
    if let Some(p) = ps.iter().find(|p| p.mass_kg.is_none()) {
        return Err(format!("{} has no material, so its mass is unknown", name(p)));
    }
    let total: f64 = ps.iter().filter_map(|p| p.mass_kg).sum();
    let passed = total <= limit * (1.0 + 1e-12);
    let fix = (!passed).then(|| {
        // Thin the heaviest part: mass is linear in height for a box.
        let heavy = ps.iter().filter(|p| p.is_box()).max_by(|a, b| a.mass_kg.unwrap_or(0.0).total_cmp(&b.mass_kg.unwrap_or(0.0)))?;
        let budget = limit - (total - heavy.mass_kg?);
        (budget > 0.0).then(|| {
            let height = (heavy.height_mm * budget / heavy.mass_kg.unwrap() * 10.0).floor() / 10.0;
            Fix {
                label: format!("Reduce {} height to {} mm", name(heavy), fmt(height)),
                operation: "solid.box.set_dimensions".into(),
                parameters: json!({ "target": heavy.id, "height": mm_q(height) }),
            }
        })
    });
    Ok(RequirementResult {
        passed,
        measured: total,
        limit,
        unit: "kg".into(),
        upper_bound: true,
        summary: format!("{} of {} max", format_mass(total), format_mass(limit)),
        violations: if passed { vec![] } else { vec![Violation { parts: ps.iter().map(|p| p.id).collect(), names: ps.iter().map(geometry::name).collect(), region: None }] },
        fix: fix.flatten(),
        accuracy_mm: None,
    })
}

fn fits_within(ps: &[PartView], envelope: [f64; 3]) -> RequirementResult {
    let bounds = ps.iter().skip(1).fold(ps[0].aabb, |acc, p| acc.union(&p.aabb));
    let size = bounds.size();
    let over: Vec<usize> = (0..3).filter(|k| size[*k] > envelope[*k] + EPS).collect();
    let worst = (0..3).map(|k| size[k] / envelope[k]).fold(0.0, f64::max);
    let fix = (!over.is_empty() && ps.len() == 1 && ps[0].is_box()).then(|| {
        let names = ["width", "depth", "height"];
        let mut parameters = json!({ "target": ps[0].id });
        for k in &over {
            parameters[names[*k]] = mm_q(envelope[*k]);
        }
        Fix {
            label: format!("Shrink {} to fit the envelope", name(&ps[0])),
            operation: "solid.box.set_dimensions".into(),
            parameters,
        }
    });
    RequirementResult {
        passed: over.is_empty(),
        measured: worst * 100.0,
        limit: 100.0,
        unit: "%".into(),
        upper_bound: true,
        summary: format!(
            "{} × {} × {} in {} × {} × {} mm",
            fmt(size[0]), fmt(size[1]), fmt(size[2]), fmt(envelope[0]), fmt(envelope[1]), fmt(envelope[2])
        ),
        violations: if over.is_empty() {
            vec![]
        } else {
            vec![Violation { parts: ps.iter().map(|p| p.id).collect(), names: ps.iter().map(geometry::name).collect(), region: Some([bounds.min, bounds.max].concat().try_into().unwrap()) }]
        },
        fix,
        accuracy_mm: None,
    }
}

fn clearance(a: &PartView, b: &PartView, min: f64) -> RequirementResult {
    // Search just past the requirement: enough to prove it, and cheap even for
    // dense meshes like threads (a wide search compares every nearby triangle pair).
    let search = (min * 2.0).max(min + 0.25).min(min + 5.0);
    let (overlap, distance) = geometry::contact(a, b, search, geometry::check_tolerance(min));
    let exact_shapes = !(a.is_box() && b.is_box());
    let passed = overlap.is_none() && distance + EPS >= min;
    // Exact single-axis fixes exist for boxes; shapes get none rather than a guess.
    let fix = (!passed && a.is_box() && b.is_box()).then(|| {
        let (axis, delta) = a.aabb.push_apart(&b.aabb, min);
        move_fix(b, axis, delta, "to restore clearance")
    });
    RequirementResult {
        passed,
        measured: if overlap.is_some() { 0.0 } else { distance },
        limit: min,
        unit: "mm".into(),
        upper_bound: false,
        summary: {
            let text = match overlap {
                Some((v, _)) => format!("Parts overlap by {} mm³", fmt_volume(v)),
                None if distance >= search => format!("More than {} mm apart, {} mm required", fmt(search), fmt(min)),
                None => format!("{} mm apart, {} mm required", fmt_gap(distance), fmt(min)),
            };
            if exact_shapes { format!("{text} · checked to ±{} mm", geometry::check_tolerance(min)) } else { text }
        },
        violations: if passed {
            vec![]
        } else {
            vec![Violation { parts: vec![a.id, b.id], names: vec![geometry::name(a), geometry::name(b)], region: overlap.map(|(_, o)| [o.min, o.max].concat().try_into().unwrap()) }]
        },
        fix,
        accuracy_mm: exact_shapes.then(|| geometry::check_tolerance(min)),
    }
}

fn no_collisions(world: &World, ps: &[PartView]) -> RequirementResult {
    let mut violations = Vec::new();
    let mut fix = None;
    // A screw in its thread, an insert in its boss, a PEM standoff pressed
    // into its sheet: overlap is how fasteners go in (as the interference check counts them).
    let fastener = |p: &PartView| p.instance.is_some_and(|i| de_assembly::fit::is_fastener(world, i));
    // A part declared embedded in another (rebar in its slab, an insert moulded
    // into its boss) may overlap it, as long as it stays inside its hosts.
    let hosts: Vec<Vec<usize>> = ps.iter().enumerate().map(|(i, p)| (0..ps.len()).filter(|k| *k != i && p.hosts.iter().any(|h| ps[*k].named(h))).collect()).collect();
    let tolerance = |i: usize| hosts[i].iter().map(|k| geometry::collision_tolerance(&ps[i], &ps[*k])).fold(0.0, f64::max);
    let poking: Vec<Option<(f64, [f64; 3], [f64; 3])>> = hosts
        .iter()
        .enumerate()
        .map(|(i, hs)| {
            if hs.is_empty() {
                return None;
            }
            let t = tolerance(i);
            let host_solids: Vec<manifold_csg::Manifold> = hs.iter().map(|k| ps[*k].solid(t)).collect();
            de_assembly::embedded::outside(&ps[i].solid(t), &host_solids.iter().collect::<Vec<_>>())
        })
        .collect();
    let mut poked = vec![false; ps.len()];
    for (i, a) in ps.iter().enumerate() {
        for (k, b) in ps.iter().enumerate().skip(i + 1) {
            if fastener(a) || fastener(b) {
                continue;
            }
            let embeds: Vec<usize> = [(i, k), (k, i)].into_iter().filter(|(e, h)| hosts[*e].contains(h)).map(|(e, _)| e).collect();
            if !embeds.is_empty() {
                // Inside its host as declared; where it pokes out, that is the violation (once per part).
                for e in embeds {
                    if let Some((_, lo, hi)) = poking[e]
                        && !poked[e]
                    {
                        poked[e] = true;
                        let host = if e == i { b } else { a };
                        violations.push(Violation { parts: vec![ps[e].id, host.id], names: vec![geometry::name(&ps[e]), geometry::name(host)], region: Some([lo, hi].concat().try_into().unwrap()) });
                    }
                }
                continue;
            }
            if let (Some((_, o)), _) = geometry::contact(a, b, 0.0, geometry::collision_tolerance(a, b)) {
                if fix.is_none() && a.is_box() && b.is_box() {
                    let (axis, delta) = a.aabb.push_apart(&b.aabb, 0.0);
                    fix = Some(move_fix(b, axis, delta, "to clear the collision"));
                }
                violations.push(Violation { parts: vec![a.id, b.id], names: vec![geometry::name(a), geometry::name(b)], region: Some([o.min, o.max].concat().try_into().unwrap()) });
            }
        }
    }
    let n = violations.len();
    RequirementResult {
        passed: n == 0,
        measured: n as f64,
        limit: 0.0,
        unit: "collisions".into(),
        upper_bound: true,
        summary: match (n, poked.iter().filter(|p| **p).count()) {
            (0, _) => {
                let inside = hosts.iter().filter(|h| !h.is_empty()).count();
                if inside > 0 { format!("{} parts, none overlapping ({inside} embedded in their hosts as declared)", ps.len()) } else { format!("{} parts, none overlapping", ps.len()) }
            }
            (n, k) if k == n => format!("{n} embedded part{} poke{} out of {} host", if n == 1 { "" } else { "s" }, if n == 1 { "s" } else { "" }, if n == 1 { "its" } else { "their" }),
            (1, _) => "1 pair of parts overlaps".into(),
            (n, _) => format!("{n} pairs of parts overlap"),
        },
        violations,
        fix,
        accuracy_mm: ps.iter().any(|p| !p.is_box()).then(|| ps.iter().map(|p| geometry::collision_tolerance(p, p)).fold(0.0, f64::max)),
    }
}

/// The joint values a motion check holds the other joints at: `set` (by
/// id), and each combination of the `across` joints stepped through their
/// limits. One empty map when there is nothing to hold.
fn motion_poses(world: &World, set: &BTreeMap<StableId, f64>, across: &[StableId], across_steps: usize) -> Result<Vec<Vec<(StableId, f64)>>, String> {
    let mut poses: Vec<Vec<(StableId, f64)>> = vec![set.iter().map(|(k, v)| (*k, *v)).collect()];
    for j in across {
        let e = world.resource::<de_core::IdIndex>().get(*j).ok_or_else(|| format!("joint {j} does not exist"))?;
        let ps = world.get::<de_shape::params::ParameterSet>(e).ok_or_else(|| format!("{j} is not a joint"))?;
        let p = ps.params.first().ok_or_else(|| format!("{}: nothing moves", ps.name))?;
        let (Some(lo), Some(hi)) = (p.min, p.max) else { return Err(format!("{} needs limits to be stepped through", ps.name)) };
        let n = across_steps.clamp(1, 36);
        let values: Vec<f64> = (0..=n).map(|k| lo + (hi - lo) * k as f64 / n as f64).collect();
        poses = poses.into_iter().flat_map(|pose| values.iter().map(move |v| pose.iter().copied().chain(std::iter::once((*j, *v))).collect::<Vec<_>>())).collect();
    }
    Ok(poses)
}

fn clear_in_motion(world: &World, joint: StableId, ids: &[StableId], steps: usize, set: &BTreeMap<StableId, f64>, across: &[StableId], across_steps: usize) -> Result<RequirementResult, String> {
    let poses = motion_poses(world, set, across, across_steps)?;
    let name_of = |j: StableId| de_assembly::joints::joint_name(world, j);
    let mut found: Vec<(de_assembly::motion::AssemblySweep, Vec<(StableId, f64)>)> = vec![];
    let mut skipped: Vec<String> = vec![];
    for pose in &poses {
        let mut held: std::collections::HashMap<StableId, Vec<f64>> = Default::default();
        for (j, v) in pose {
            let mut now = de_assembly::joints::link(world, *j, None)?.values;
            if now.is_empty() {
                return Err(format!("{} has no value to hold", name_of(*j)));
            }
            now[0] = *v;
            held.insert(*j, now);
        }
        match de_assembly::motion::sweep(world, joint, None, None, None, steps, &held, true) {
            Ok(s) => found.push((s, pose.clone())),
            // A pose the interlocks forbid outright is not one the arm can be in.
            Err(m) if poses.len() > 1 && m.contains("interlock") => skipped.push(m),
            Err(m) => return Err(m),
        }
    }
    let Some((first, _)) = found.first() else { return Err(skipped.first().cloned().unwrap_or_else(|| "nothing to sweep".into())) };
    let unit = |v: f64| format!("{}{}", fmt(v), if first.unit == "°" { "°" } else { " mm" });
    let pose_text = |pose: &[(StableId, f64)]| -> String {
        if pose.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = pose.iter().map(|(j, v)| format!("{} at {}", name_of(*j), unit(*v))).collect();
        format!(" with {}", parts.join(", "))
    };
    let mut clashes: Vec<(&de_assembly::motion::PairGap, &de_assembly::motion::AssemblySweep, &[(StableId, f64)])> = vec![];
    let mut closest: Option<(&de_assembly::motion::PairGap, &de_assembly::motion::AssemblySweep, &[(StableId, f64)])> = None;
    let mut binds: Option<(f64, &[(StableId, f64)])> = None;
    for (s, pose) in &found {
        let covered = |id: StableId| ids.is_empty() || ids.contains(&id) || s.assembly.is_some_and(|a| ids.contains(&a));
        for p in s.pairs.iter().filter(|p| !p.joined && (covered(p.a_id) || covered(p.b_id))) {
            if p.gap < 1e-4 {
                clashes.push((p, s, pose));
            } else if closest.is_none_or(|(c, _, _)| p.gap < c.gap) {
                closest = Some((p, s, pose));
            }
        }
        if let Some(v) = s.binds_at
            && binds.is_none()
        {
            binds = Some((v, pose));
        }
    }
    let range = format!("{}–{}", unit(first.from), unit(first.to));
    let over = if poses.len() > 1 { format!(" at {} poses of {}", found.len(), across.iter().map(|j| name_of(*j)).collect::<Vec<_>>().join(" × ")) } else { pose_text(&poses[0]) };
    let summary = match (clashes.first(), closest) {
        (Some((c, s, pose)), _) => {
            let [a, b] = c.range.unwrap_or([c.at, c.at]);
            let more = if clashes.len() > 1 { format!(" (+{} more)", clashes.len() - 1) } else { String::new() };
            format!("{} hits {} as {} moves, from {} to {}{}{more}", c.a, c.b, s.name, unit(a), unit(b), pose_text(pose))
        }
        (None, Some((p, s, pose))) => format!("Clear through {range} of {}{over}; closest {} mm ({} – {} at {}{})", s.name, fmt_gap(p.gap), p.a, p.b, unit(p.at), pose_text(pose)),
        (None, None) => format!("Clear through {range} of {}{over}; nothing comes within {} mm", first.name, fmt(first.search)),
    };
    let summary = match binds {
        Some((v, pose)) => format!("{summary}; binds at {}{}", unit(v), pose_text(pose)),
        None => summary,
    };
    // Where the interlocks narrowed the sweep, say so.
    let narrowed = found.iter().find(|(s, _)| !s.allowed.is_empty());
    let summary = match narrowed {
        Some((s, pose)) => {
            let pieces: Vec<String> = s.allowed.iter().map(|[a, b]| format!("{}…{}", unit(*a), unit(*b))).collect();
            format!("{summary} (interlocks hold {} to {}{})", s.name, pieces.join(" and "), pose_text(pose))
        }
        None => summary,
    };
    let clashes: Vec<&de_assembly::motion::PairGap> = clashes.into_iter().map(|(p, _, _)| p).collect();
    Ok(RequirementResult {
        passed: clashes.is_empty() && binds.is_none(),
        measured: clashes.len() as f64,
        limit: 0.0,
        unit: "clashes".into(),
        upper_bound: true,
        summary,
        violations: clashes.iter().map(|c| Violation { parts: vec![c.a_id, c.b_id], names: vec![c.a.clone(), c.b.clone()], region: None }).collect(),
        fix: None,
        accuracy_mm: None,
    })
}

/// The assembly a fit rule covers: an assembly definition, or the one an instance shows.
fn fit_scope(world: &World, id: StableId) -> Option<StableId> {
    let e = world.resource::<de_core::IdIndex>().get(id)?;
    if world.get::<de_assembly::AssemblyDef>(e).is_some() {
        return Some(id);
    }
    let inst = world.get::<de_assembly::Instance>(e)?;
    let d = world.resource::<de_core::IdIndex>().get(inst.of)?;
    world.get::<de_assembly::AssemblyDef>(d).is_some().then_some(inst.of)
}

fn assembly_fits(world: &World, assembly: StableId, motion: bool, steps: usize) -> Result<RequirementResult, String> {
    let scope = fit_scope(world, assembly).ok_or_else(|| format!("{assembly} is not an assembly"))?;
    let r = de_assembly::fit::report(world, Some(scope), &de_assembly::fit::FitOptions { motion, steps })?;
    let failing: Vec<&de_assembly::fit::FitItem> = r.items.iter().filter(|i| i.status == de_assembly::fit::FitStatus::Fail).collect();
    let summary = match failing.first() {
        Some(f) => {
            let why = f.checks.iter().find(|c| c.status == de_assembly::fit::FitStatus::Fail).map(|c| c.text.clone()).unwrap_or_default();
            let more = if failing.len() > 1 { format!(" (+{} more)", failing.len() - 1) } else { String::new() };
            format!("{}: {why}{more}", f.title)
        }
        None => format!("{} items fit ({} with warnings)", r.pass + r.warn, r.warn),
    };
    Ok(RequirementResult {
        passed: r.ok,
        measured: r.fail as f64,
        limit: 0.0,
        unit: "misfits".into(),
        upper_bound: true,
        summary,
        violations: failing.iter().map(|f| Violation { parts: f.instances.clone(), names: f.names.clone(), region: None }).collect(),
        fix: None,
        accuracy_mm: None,
    })
}

/// A requirement on what a shape's saved study finds: its figure against the
/// limit (`upper`: must stay at or below it). The study is solved only when
/// something it depends on changed (see `de_shape::studies::run_check`).
fn study_check(world: &World, shape: StableId, part: Option<&str>, check: de_shape::studies::Check, limit: f64, upper: bool, unit: &str) -> Result<RequirementResult, String> {
    let e = world.resource::<de_core::IdIndex>().get(shape).ok_or_else(|| format!("{shape} does not exist"))?;
    let c = de_shape::studies::run_check(world, e, part, check)?;
    let passed = if upper { c.value <= limit * (1.0 + 1e-9) } else { c.value >= limit * (1.0 - 1e-9) };
    let need = match (check, upper) {
        (de_shape::studies::Check::Safety, _) => format!("at least ×{}", fmt(limit)),
        (_, true) => format!("at most {} {unit}", fmt(limit)),
        (_, false) => format!("at least {}{}", fmt(limit), if unit.is_empty() { String::new() } else { format!(" {unit}") }),
    };
    let name = world.get::<de_shape::ShapeProgram>(e).map(|p| p.name.clone()).unwrap_or_default();
    let label = part.map(|p| format!("{name} › {p}")).unwrap_or(name);
    Ok(RequirementResult {
        passed,
        measured: if c.value.is_finite() { c.value } else { 1e9 },
        limit,
        unit: if unit.is_empty() { "×".into() } else { unit.into() },
        upper_bound: upper,
        summary: format!("{}; {need}", c.summary),
        violations: if passed { vec![] } else { vec![Violation { parts: vec![shape], names: vec![label], region: c.at.map(|a| [a[0] - 2.0, a[1] - 2.0, a[2] - 2.0, a[0] + 2.0, a[1] + 2.0, a[2] + 2.0]) }] },
        fix: None,
        accuracy_mm: None,
    })
}

/// A part against its scan: each point's distance from the part's surface.
fn scan_check(world: &World, id: StableId, part_name: Option<&str>, scan: &str, tolerance: f64, within: f64, align: bool) -> Result<RequirementResult, String> {
    use de_scan::inspect;
    let mut view = part(world, id)?;
    if let Some(n) = part_name {
        view.part = Some(n.to_string());
    }
    let accuracy = (tolerance / 10.0).clamp(0.0005, 0.05);
    let mesh = de_shape::scans::from_manifold(&view.solid(accuracy));
    let bvh = de_scan::spatial::TriBvh::new(&mesh);
    let data = de_shape::scans::load(scan)?;
    let points: Vec<[f64; 3]> = if align {
        let a = inspect::align(&data.cloud.positions, &bvh, true, 60);
        data.cloud.positions.iter().map(|p| inspect::apply(&a.transform, *p)).collect()
    } else {
        data.cloud.positions.clone()
    };
    let d = inspect::deviation(&points, &bvh, tolerance, within);
    // The gauge: the |deviation| that the required share of points stays under.
    let mut abs: Vec<f64> = d.values.iter().map(|v| v.abs()).collect();
    abs.sort_by(|a, b| a.total_cmp(b));
    let at = ((abs.len() as f64 * within).ceil() as usize).clamp(1, abs.len().max(1)) - 1;
    let measured = abs.get(at).copied().unwrap_or(0.0);
    let w = d.stats.worst;
    let share = if within < 1.0 { format!("{}% of points", fmt(within * 100.0)) } else { "every point".into() };
    Ok(RequirementResult {
        passed: d.passed,
        measured,
        limit: tolerance,
        unit: "mm".into(),
        upper_bound: true,
        summary: format!(
            "{}% of {} points within ±{} mm ({share} required); worst {:+.3} mm, mean {:+.3}, RMS {:.3}{}",
            fmt(d.within * 100.0),
            d.stats.count,
            fmt(tolerance),
            d.stats.worst_deviation,
            d.stats.mean,
            d.stats.rms,
            if align { ", best-fit aligned" } else { "" }
        ),
        violations: if d.passed { vec![] } else { vec![Violation { parts: vec![id], names: vec![name(&view)], region: Some([w[0] - 2.0, w[1] - 2.0, w[2] - 2.0, w[0] + 2.0, w[1] + 2.0, w[2] + 2.0]) }] },
        fix: None,
        accuracy_mm: Some(accuracy),
    })
}

fn silhouette_check(world: &World, id: StableId, canvas: StableId, min_iou: f64, max_deviation: Option<f64>) -> Result<RequirementResult, String> {
    let e = world.resource::<de_core::IdIndex>().get(canvas).ok_or_else(|| format!("canvas {canvas} does not exist"))?;
    let c = world.get::<de_shape::canvas::Canvas>(e).ok_or_else(|| format!("{canvas} is not a canvas"))?;
    let view = part(world, id)?;
    let accuracy = (c.mm_per_px_at([c.pixels[0] as f64 / 2.0, c.pixels[1] as f64 / 2.0]) / 4.0).clamp(0.01, 0.5);
    let mesh = de_shape::silhouette::mesh_of(&view.solid(accuracy), [0.7, 0.7, 0.7]);
    let o = de_shape::silhouette::Options { picture: false, ..Default::default() };
    let m = de_shape::silhouette::compare(c, &[mesh], &o)?;
    let deviation_ok = max_deviation.is_none_or(|d| m.worst_mm <= d);
    let passed = m.iou >= min_iou && deviation_ok;
    let worst = m.regions.first();
    Ok(RequirementResult {
        passed,
        measured: m.iou,
        limit: min_iou,
        unit: "IoU".into(),
        upper_bound: false,
        summary: format!(
            "“{}”: silhouette IoU {:.3} (needs {}); outlines {:.2} mm apart on average, worst {:.1} mm{}{}",
            c.name,
            m.iou,
            fmt(min_iou),
            m.mean_mm,
            m.worst_mm,
            max_deviation.map(|d| format!(" (limit {} mm)", fmt(d))).unwrap_or_default(),
            worst.map(|r| format!("; largest difference: {} {:.0} mm² near ({:.0}, {:.0}, {:.0})", if r.kind == "extra" { "extra material" } else { "missing material" }, r.area_mm2, r.at[0], r.at[1], r.at[2])).unwrap_or_default()
        ),
        violations: if passed {
            vec![]
        } else {
            worst.map(|r| {
                let (a, b) = (c.plane.to_world([r.bounds[0], r.bounds[1]]), c.plane.to_world([r.bounds[2], r.bounds[3]]));
                vec![Violation { parts: vec![id], names: vec![name(&view)], region: Some([a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2]), a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]) }]
            }).unwrap_or_default()
        },
        fix: None,
        accuracy_mm: Some(accuracy),
    })
}

// ---- Commands --------------------------------------------------------------

fn validate(tx: &Tx, rule: &Rule) -> Result<(), CommandError> {
    if let Rule::NoClashesInMotion { joint, steps, set, across, across_steps, .. } = rule {
        for j in std::iter::once(joint).chain(set.keys()).chain(across) {
            if tx.get::<de_assembly::joints::Joint>(*j).is_none() {
                return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{j} is not an assembly joint")));
            }
        }
        if set.contains_key(joint) || across.contains(joint) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "the swept joint cannot also be held (set) or stepped (across)"));
        }
        if steps.is_some_and(|n| !(2..=720).contains(&n)) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "steps must be 2 to 720"));
        }
        if across_steps.is_some_and(|n| !(1..=36).contains(&n)) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "across_steps must be 1 to 36"));
        }
        motion_poses(tx.world(), set, across, across_steps.unwrap_or(4) as usize).map_err(|m| CommandError::new(ErrorCode::InvalidParameters, m))?;
    }
    if let Rule::AssemblyFits { assembly, steps, .. } = rule {
        if fit_scope(tx.world(), *assembly).is_none() {
            return Err(CommandError::new(ErrorCode::InvalidParameters, format!("{assembly} is not an assembly (or an instance of one)")));
        }
        if steps.is_some_and(|n| !(2..=720).contains(&n)) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "steps must be 2 to 720"));
        }
    }
    // A motion check's joint, a value check's target and a photo match's canvas are checked below, not as parts.
    let joint_of = |id: StableId| {
        matches!(rule, Rule::NoClashesInMotion { joint, set, across, .. } if *joint == id || set.contains_key(&id) || across.contains(&id))
            || matches!(rule, Rule::ParameterRange { .. })
            || matches!(rule, Rule::SilhouetteMatch { canvas, .. } if *canvas == id)
    };
    for id in rule.targets().into_iter().filter(|id| !joint_of(*id)) {
        if tx.get::<BoxFeature>(id).is_none()
            && tx.get::<de_shape::ShapeProgram>(id).is_none()
            && tx.get::<de_assembly::Instance>(id).is_none()
            && tx.get::<de_assembly::AssemblyDef>(id).is_none()
            && tx.get::<de_pcb::Board>(id).is_none()
        {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a part")));
        }
    }
    let positive = |l: &Length, what: &str| {
        if l.canonical() > 0.0 { Ok(()) } else { Err(CommandError::new(ErrorCode::InvalidParameters, format!("{what} must be greater than zero"))) }
    };
    match rule {
        Rule::MaxMass { parts, limit } => {
            if parts.is_empty() || limit.canonical() <= 0.0 {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "a mass limit needs parts and a positive limit"));
            }
        }
        Rule::FitsWithin { parts, width, depth, height } => {
            if parts.is_empty() {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "an envelope needs at least one part"));
            }
            positive(width, "width")?;
            positive(depth, "depth")?;
            positive(height, "height")?;
        }
        Rule::Clearance { a, b, min } => {
            if a == b {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "clearance needs two different parts"));
            }
            if min.canonical() < 0.0 {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "clearance cannot be negative"));
            }
        }
        Rule::NoCollisions { parts } => {
            let compound = |id: StableId| {
                tx.get::<de_shape::ShapeProgram>(id).is_some() || tx.get::<de_assembly::Instance>(id).is_some() || tx.get::<de_assembly::AssemblyDef>(id).is_some()
            };
            if parts.is_empty() || (parts.len() == 1 && !compound(parts[0])) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "collision checks need two parts, or one assembly"));
            }
        }
        Rule::NoClashesInMotion { .. } | Rule::AssemblyFits { .. } => {}
        Rule::MinSafetyFactor { shape, min, .. } | Rule::MinFirstFrequency { shape, min, .. } | Rule::MinBucklingFactor { shape, min, .. } => {
            if tx.get::<de_shape::ShapeProgram>(*shape).is_none() {
                return Err(CommandError::new(ErrorCode::InvalidParameters, format!("{shape} is not a shape: studies are on shapes")));
            }
            if !(*min > 0.0) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "the minimum must be greater than zero"));
            }
        }
        Rule::MaxDeflection { shape, max, .. } => {
            if tx.get::<de_shape::ShapeProgram>(*shape).is_none() {
                return Err(CommandError::new(ErrorCode::InvalidParameters, format!("{shape} is not a shape: studies are on shapes")));
            }
            positive(max, "the deflection limit")?;
        }
        Rule::ParameterRange { target, key, min, max } => {
            if min.is_none() && max.is_none() {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "give a min, a max or both"));
            }
            if let (Some(a), Some(b)) = (min, max)
                && a > b
            {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "min is above max"));
            }
            parameter_now(tx.world(), *target, key).map_err(|m| CommandError::new(ErrorCode::UnresolvedReference, m))?;
        }
        Rule::PcbConstraints { board, .. } | Rule::PcbLength { board, .. } => {
            if tx.get::<de_pcb::Board>(*board).is_none() {
                return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{board} is not a circuit board")));
            }
        }
        Rule::SilhouetteMatch { canvas, min_iou, max_deviation, .. } => {
            if tx.get::<de_shape::canvas::Canvas>(*canvas).is_none() {
                return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{canvas} is not a canvas (ise canvas list)")));
            }
            if !(*min_iou > 0.0 && *min_iou <= 1.0) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "min_iou is the overlap needed, above 0 and at most 1"));
            }
            if let Some(d) = max_deviation {
                positive(d, "the largest deviation")?;
            }
        }
        Rule::ScanDeviation { scan, tolerance, within, .. } => {
            positive(tolerance, "the tolerance")?;
            if within.is_some_and(|w| !(w > 0.0 && w <= 1.0)) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "within is the share of points that must pass, 0 to 1"));
            }
            if de_shape::assets::path(scan).is_err() {
                return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{scan} is not a stored scan (add it with POST /scans or ise scan add)")));
            }
        }
    }
    Ok(())
}

/// A motion check covers the joint's assembly unless told otherwise (top
/// level: every top-level instance), so it re-runs when any of it changes.
fn with_default_parts(tx: &Tx, rule: Rule) -> Rule {
    match rule {
        Rule::NoClashesInMotion { joint, parts, steps, set, across, across_steps } if parts.is_empty() => {
            let scope = tx.get::<de_assembly::joints::Joint>(joint).and_then(|j| j.assembly);
            let parts = match scope {
                Some(a) => vec![a],
                None => {
                    let world = tx.world();
                    let mut ids: Vec<StableId> = world.resource::<de_core::IdIndex>().iter().filter(|(_, e)| world.get::<de_assembly::Instance>(*e).is_some_and(|i| i.parent.is_none())).map(|(id, _)| id).collect();
                    ids.sort();
                    ids
                }
            };
            Rule::NoClashesInMotion { joint, parts, steps, set, across, across_steps }
        }
        r => r,
    }
}

fn link(tx: &mut Tx, id: StableId, old: &[StableId], new: &[StableId]) -> Result<(), CommandError> {
    for gone in old.iter().filter(|o| !new.contains(o)) {
        tx.remove_dependency(id, *gone)?;
    }
    for added in new {
        tx.add_dependency(id, *added)?;
    }
    Ok(())
}

/// `intent.requirement.create`
#[derive(Deserialize, JsonSchema)]
pub struct CreateRequirement {
    pub name: Option<String>,
    pub rule: Rule,
}

impl Command for CreateRequirement {
    const NAME: &'static str = "intent.requirement.create";

    fn apply(mut self, tx: &mut Tx) -> Result<Value, CommandError> {
        validate(tx, &self.rule)?;
        self.rule = with_default_parts(tx, self.rule);
        let id = tx.create();
        let name = self.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| self.rule.default_name());
        link(tx, id, &[], &self.rule.targets())?;
        if matches!(self.rule, Rule::AssemblyFits { .. }) {
            // Judged against the printer profile: make sure there is one to follow.
            let profile = match de_assembly::fit::profile_entity(tx.world()) {
                Some(p) => p,
                None => serde_json::from_value(tx.run("fit.profile.set", json!({}))?["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?,
            };
            tx.add_dependency(id, profile)?;
        }
        tx.set(id, Requirement { name, rule: self.rule })?;
        Ok(json!({ "id": id }))
    }
}

/// `intent.requirement.update`: rename and/or change the rule.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateRequirement {
    pub target: StableId,
    pub name: Option<String>,
    pub rule: Option<Rule>,
}

impl Command for UpdateRequirement {
    const NAME: &'static str = "intent.requirement.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut req = tx
            .get::<Requirement>(self.target)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a requirement", self.target)))?;
        if let Some(rule) = self.rule {
            validate(tx, &rule)?;
            let rule = with_default_parts(tx, rule);
            let renamed = req.name != req.rule.default_name();
            link(tx, self.target, &req.rule.targets(), &rule.targets())?;
            if !renamed {
                req.name = rule.default_name();
            }
            req.rule = rule;
        }
        if let Some(name) = self.name.filter(|n| !n.trim().is_empty()) {
            req.name = name;
        }
        tx.set(self.target, req)?;
        Ok(json!({ "id": self.target }))
    }
}
