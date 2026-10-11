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
    },
}

impl Rule {
    pub fn targets(&self) -> Vec<StableId> {
        match self {
            Rule::MaxMass { parts, .. } | Rule::FitsWithin { parts, .. } | Rule::NoCollisions { parts } => parts.clone(),
            Rule::Clearance { a, b, .. } => vec![*a, *b],
            Rule::NoClashesInMotion { joint, parts, .. } => std::iter::once(*joint).chain(parts.iter().copied()).collect(),
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
        Rule::NoCollisions { parts: ids } => parts(world, ids).map(|ps| no_collisions(&geometry::expand(ps))),
        Rule::NoClashesInMotion { joint, parts: ids, steps } => clear_in_motion(world, *joint, ids, steps.unwrap_or(72) as usize),
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

fn no_collisions(ps: &[PartView]) -> RequirementResult {
    let mut violations = Vec::new();
    let mut fix = None;
    for (i, a) in ps.iter().enumerate() {
        for b in &ps[i + 1..] {
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
        summary: match n {
            0 => format!("{} parts, none overlapping", ps.len()),
            1 => "1 pair of parts overlaps".into(),
            n => format!("{n} pairs of parts overlap"),
        },
        violations,
        fix,
        accuracy_mm: ps.iter().any(|p| !p.is_box()).then(|| ps.iter().map(|p| geometry::collision_tolerance(p, p)).fold(0.0, f64::max)),
    }
}

fn clear_in_motion(world: &World, joint: StableId, ids: &[StableId], steps: usize) -> Result<RequirementResult, String> {
    let s = de_assembly::motion::sweep(world, joint, None, None, None, steps, &Default::default())?;
    let covered = |id: StableId| ids.is_empty() || ids.contains(&id) || s.assembly.is_some_and(|a| ids.contains(&a));
    let unit = |v: f64| format!("{}{}", fmt(v), if s.unit == "°" { "°" } else { " mm" });
    let watched: Vec<&de_assembly::motion::PairGap> = s.pairs.iter().filter(|p| !p.joined && (covered(p.a_id) || covered(p.b_id))).collect();
    let clashes: Vec<&&de_assembly::motion::PairGap> = watched.iter().filter(|p| p.gap < 1e-4).collect();
    let range = format!("{}–{}", unit(s.from), unit(s.to));
    let summary = match (clashes.first(), watched.first()) {
        (Some(c), _) => {
            let [a, b] = c.range.unwrap_or([c.at, c.at]);
            let more = if clashes.len() > 1 { format!(" (+{} more)", clashes.len() - 1) } else { String::new() };
            format!("{} hits {} as {} moves, from {} to {}{more}", c.a, c.b, s.name, unit(a), unit(b))
        }
        (None, Some(p)) => format!("Clear through {range} of {}; closest {} mm ({} – {} at {})", s.name, fmt_gap(p.gap), p.a, p.b, unit(p.at)),
        (None, None) => format!("Clear through {range} of {}; nothing comes within {} mm", s.name, fmt(s.search)),
    };
    let summary = match s.binds_at {
        Some(v) => format!("{summary}; binds at {}", unit(v)),
        None => summary,
    };
    Ok(RequirementResult {
        passed: clashes.is_empty() && s.binds_at.is_none(),
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

// ---- Commands --------------------------------------------------------------

fn validate(tx: &Tx, rule: &Rule) -> Result<(), CommandError> {
    if let Rule::NoClashesInMotion { joint, steps, .. } = rule {
        if tx.get::<de_assembly::joints::Joint>(*joint).is_none() {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{joint} is not an assembly joint")));
        }
        if steps.is_some_and(|n| !(2..=720).contains(&n)) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "steps must be 2 to 720"));
        }
    }
    let joint_of = |id: StableId| matches!(rule, Rule::NoClashesInMotion { joint, .. } if *joint == id);
    for id in rule.targets().into_iter().filter(|id| !joint_of(*id)) {
        if tx.get::<BoxFeature>(id).is_none()
            && tx.get::<de_shape::ShapeProgram>(id).is_none()
            && tx.get::<de_assembly::Instance>(id).is_none()
            && tx.get::<de_assembly::AssemblyDef>(id).is_none()
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
        Rule::NoClashesInMotion { .. } => {}
    }
    Ok(())
}

/// A motion check covers the joint's assembly unless told otherwise (top
/// level: every top-level instance), so it re-runs when any of it changes.
fn with_default_parts(tx: &Tx, rule: Rule) -> Rule {
    match rule {
        Rule::NoClashesInMotion { joint, parts, steps } if parts.is_empty() => {
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
            Rule::NoClashesInMotion { joint, parts, steps }
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
