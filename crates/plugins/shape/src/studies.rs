//! Studies kept with a shape, run as they are saved: a part's material with
//! the person's own figures over the library's, the saved faces found on the
//! part as it is now, and the checks requirements make (largest stress,
//! movement, first frequency, buckling), remembered by their inputs so an
//! unrelated edit never solves the same study twice.

use crate::fea::{self, FacePick, materials::MaterialProps};
use crate::{PartMaterials, ShapeProgram, evaluated_at, resolve_face};
use de_core::{
    IdIndex, StableId,
    bevy_ecs::{entity::Entity, world::World},
    schemars::{self, JsonSchema},
};
use de_solid::{Material, MaterialRef};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};

/// A force on a round face, as a bearing (or a shaft in a bore) gives it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct BearingPick {
    pub face: FacePick,
    /// N.
    pub force: [f64; 3],
}

/// A force and moment at a point (part coordinates, mm) carried to a face.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct RemotePick {
    pub face: FacePick,
    pub point: [f64; 3],
    #[serde(default)]
    pub force: [f64; 3],
    /// N·mm.
    #[serde(default)]
    pub moment: [f64; 3],
}

/// A face moved by set amounts (mm) along some axes; null leaves one free.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct DisplacementPick {
    pub face: FacePick,
    pub by: [Option<f64>; 3],
}

/// A face on springs: N/mm per mm² of face.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ElasticPick {
    pub face: FacePick,
    pub stiffness: f64,
    #[serde(default)]
    pub normal_only: bool,
}

/// A part's material: the library's typical figures for its name, with what
/// the person set on its material entity laid over them, and the entity's
/// own density (which also sets its mass).
pub fn material_props(world: &World, entity: Entity, part: Option<&str>) -> Result<MaterialProps, String> {
    let index = world.resource::<IdIndex>();
    let stats = world.get::<crate::ShapeSolid>(entity).map(|s| &s.stats);
    let named = part.and_then(|n| stats?.parts.iter().find(|p| p.name == n)?.material.clone());
    let fallback: Option<StableId> = world.get::<MaterialRef>(entity).map(|m| m.0);
    let id: Option<StableId> = match &named {
        Some(name) => world.get::<PartMaterials>(entity).and_then(|t| t.0.get(name).copied()),
        None => fallback,
    };
    let material: Option<&Material> = id.and_then(|id| index.get(id)).and_then(|e| world.get::<Material>(e));
    let name = named.or_else(|| material.map(|m| m.name.clone())).ok_or_else(|| match part {
        Some(p) => format!("give “{p}” a material first: its stiffness and strength come from it"),
        None => "give the part a material first: its stiffness and strength come from it".to_string(),
    })?;
    fea::materials::resolve(&name, material.and_then(|m| m.properties.as_ref()), material.map(|m| m.density.canonical())).ok_or_else(|| {
        format!("no engineering figures for “{name}”: name it after a known material (steel, aluminium, PLA…) or set its properties (solid.material.set_properties)")
    })
}

/// A pick given by a point near the face (`near`, part coordinates, mm)
/// turned into one kept by where it sits (fractions of the part's bounds),
/// so it is found again after the part changes. Picks already kept that way pass through.
pub fn pin_face(program: &ShapeProgram, part: Option<&str>, pick: &FacePick) -> Result<FacePick, String> {
    let Some(near) = pick.near else { return Ok(pick.clone()) };
    let face = resolve_face(program, part, pick).ok_or_else(|| format!("no face near {near:?} facing {:?}", pick.normal))?;
    let on = pick.part.as_deref().or(part);
    let e = evaluated_at(program, program.display_tolerance())?;
    let (lo, hi) = match on.and_then(|n| e.parts.iter().find(|p| p.name == n)) {
        Some(p) => (p.bounds_min, p.bounds_max),
        None => (e.bounds_min, e.bounds_max),
    };
    let frac = |q: [f64; 3]| -> [f64; 3] { std::array::from_fn(|k| if hi[k] - lo[k] > 1e-9 { (q[k] - lo[k]) / (hi[k] - lo[k]) } else { 0.5 }) };
    // The probe on one of the face's own triangles (its first), the middle where the face is.
    let t = &face.triangles;
    let probe = [(t[0] + t[3] + t[6]) / 3.0, (t[1] + t[4] + t[7]) / 3.0, (t[2] + t[5] + t[8]) / 3.0];
    Ok(FacePick { at: frac(face.point), probe: frac(probe), normal: face.normal, part: pick.part.clone(), curved: pick.curved, near: None })
}

/// Whether any face of a study is given by a point near it.
pub fn has_near(spec: &fea::StudySpec) -> bool {
    let picks = spec
        .supports
        .iter()
        .chain(&spec.sliding)
        .chain(spec.loads.iter().map(|l| &l.face))
        .chain(spec.holds.iter().map(|h| &h.face))
        .chain(spec.heats.iter().map(|h| &h.face))
        .chain(spec.bearing_loads.iter().map(|b| &b.face))
        .chain(spec.remote_loads.iter().map(|r| &r.face))
        .chain(spec.displacements.iter().map(|d| &d.face))
        .chain(spec.elastic_supports.iter().map(|x| &x.face));
    picks.into_iter().any(|p| p.near.is_some())
}

/// Every pick of a saved study pinned (see `pin_face`).
pub fn pin_study(program: &ShapeProgram, spec: &fea::StudySpec) -> Result<fea::StudySpec, String> {
    let part = spec.part.as_deref();
    let pin = |p: &FacePick| pin_face(program, part, p);
    let mut s = spec.clone();
    for f in s.supports.iter_mut().chain(s.sliding.iter_mut()) {
        *f = pin(f)?;
    }
    for l in s.loads.iter_mut() {
        l.face = pin(&l.face)?;
    }
    for h in s.holds.iter_mut() {
        h.face = pin(&h.face)?;
    }
    for h in s.heats.iter_mut() {
        h.face = pin(&h.face)?;
    }
    for b in s.bearing_loads.iter_mut() {
        b.face = pin(&b.face)?;
    }
    for r in s.remote_loads.iter_mut() {
        r.face = pin(&r.face)?;
    }
    for d in s.displacements.iter_mut() {
        d.face = pin(&d.face)?;
    }
    for x in s.elastic_supports.iter_mut() {
        x.face = pin(&x.face)?;
    }
    Ok(s)
}

/// A saved study as the solver takes it: every face found on the part as it is now.
pub fn study_from_spec(program: &ShapeProgram, spec: &fea::StudySpec) -> Result<fea::Study, String> {
    let part = spec.part.as_deref();
    let face = |what: &str, i: usize, p: &FacePick| resolve_face(program, part, p).ok_or_else(|| format!("{what} {}'s face is no longer on the part: pick it again", i + 1));
    let heat = match &spec.heat_stress {
        Some(h) => Some(fea::HeatLoad { reference: h.reference, uniform: None, thermal: Some(thermal_from_spec(program, spec)?), field: None }),
        None => None,
    };
    Ok(fea::Study {
        supports: spec.supports.iter().enumerate().map(|(i, p)| face("support", i, p)).collect::<Result<_, _>>()?,
        loads: spec.loads.iter().enumerate().map(|(i, l)| Ok(l.on(face("load", i, &l.face)?))).collect::<Result<_, String>>()?,
        bore_supports: vec![],
        bore_loads: vec![],
        resolution: spec.resolution,
        contacts: spec.contacts.clone(),
        heat,
        refine: spec.refine && spec.parts.is_empty(),
        gravity: spec.gravity,
        spin: spec.spin,
        bearing_loads: spec.bearing_loads.iter().enumerate().map(|(i, b)| Ok(fea::BearingLoad { face: face("bearing load", i, &b.face)?, force: b.force })).collect::<Result<_, String>>()?,
        remote_loads: spec.remote_loads.iter().enumerate().map(|(i, r)| Ok(fea::RemoteLoad { face: face("remote load", i, &r.face)?, point: r.point, force: r.force, moment: r.moment })).collect::<Result<_, String>>()?,
        sliding: spec.sliding.iter().enumerate().map(|(i, p)| face("sliding face", i, p)).collect::<Result<_, _>>()?,
        displacements: spec.displacements.iter().enumerate().map(|(i, d)| Ok(fea::Displacement { face: face("displacement", i, &d.face)?, by: d.by })).collect::<Result<_, String>>()?,
        elastic_supports: spec.elastic_supports.iter().enumerate().map(|(i, x)| Ok(fea::ElasticSupport { face: face("elastic support", i, &x.face)?, stiffness: x.stiffness, normal_only: x.normal_only })).collect::<Result<_, String>>()?,
        bolts: spec.bolts.clone(),
        plastic: spec.plastic,
    })
}

/// A saved study's heat part as the heat solver takes it.
pub fn thermal_from_spec(program: &ShapeProgram, spec: &fea::StudySpec) -> Result<fea::ThermalStudy, String> {
    let part = spec.part.as_deref();
    let face = |what: &str, i: usize, p: &FacePick| resolve_face(program, part, p).ok_or_else(|| format!("{what} {}'s face is no longer on the part: pick it again", i + 1));
    Ok(fea::ThermalStudy {
        holds: spec.holds.iter().enumerate().map(|(i, h)| Ok(fea::Hold { face: face("held face", i, &h.face)?, temperature: h.temperature })).collect::<Result<_, String>>()?,
        heats: spec.heats.iter().enumerate().map(|(i, h)| Ok(fea::Heat { face: face("heated face", i, &h.face)?, watts: h.watts })).collect::<Result<_, String>>()?,
        convection: spec.convection.filter(|c| c.h > 0.0),
        resolution: spec.resolution.min(96),
    })
}

/// The parts a saved study covers, each with its material: the studied part first.
pub fn study_parts(world: &World, entity: Entity, spec: &fea::StudySpec) -> Result<Vec<(Option<String>, MaterialProps)>, String> {
    let mut names: Vec<Option<String>> = vec![spec.part.clone()];
    let all: Vec<String> = world.get::<crate::ShapeSolid>(entity).map(|s| s.stats.parts.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
    for n in &spec.parts {
        let more: Vec<String> = if n == "all" { all.clone() } else { vec![n.clone()] };
        for m in more {
            if !names.iter().any(|x| x.as_deref() == Some(m.as_str())) {
                names.push(Some(m));
            }
        }
    }
    names.into_iter().map(|n| material_props(world, entity, n.as_deref()).map(|m| (n, m))).collect()
}

/// What a requirement reads from a part's saved study.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// Yield strength over the largest stress (the lowest of the parts studied).
    Safety,
    /// The largest movement (mm).
    Deflection,
    /// The first natural frequency (Hz), held by the study's supports (prestressed when the study says so).
    Frequency,
    /// The lowest factor on the loads that buckles it.
    Buckling,
}

/// A check's answer: the figure, and a sentence about it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Checked {
    pub value: f64,
    pub summary: String,
    /// Where (mm, part coordinates), when it is a spot.
    pub at: Option<[f64; 3]>,
}

fn cache() -> &'static Mutex<Vec<(u64, Result<Checked, String>)>> {
    static CACHE: OnceLock<Mutex<Vec<(u64, Result<Checked, String>)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn key_of(parts: &[impl Serialize]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        serde_json::to_string(p).unwrap_or_default().hash(&mut h);
    }
    h.finish()
}

/// Runs a check on a shape's saved study for `part` (None: the whole solid),
/// remembering the answer by everything it depends on (the program and its
/// values, the study, the materials), so a requirement re-checked after an
/// edit that changes none of them costs nothing.
pub fn run_check(world: &World, entity: Entity, part: Option<&str>, check: Check) -> Result<Checked, String> {
    let program = crate::resolved_program(world, entity).ok_or("not a shape")?;
    let spec = world
        .get::<crate::ShapeStudies>(entity)
        .and_then(|s| s.0.iter().find(|st| st.part.as_deref() == part).cloned())
        .ok_or_else(|| format!("{} has no saved study: set one (supports and loads) in its Stress section, or with shape.set_study", part.map_or("the part".to_string(), |p| format!("“{p}”"))))?;
    let parts = study_parts(world, entity, &spec)?;
    let key = key_of(&[serde_json::to_value(&program).unwrap_or_default(), serde_json::to_value(&spec).unwrap_or_default(), serde_json::to_value(&parts).unwrap_or_default(), serde_json::to_value(check).unwrap_or_default()]);
    if let Some((_, hit)) = cache().lock().unwrap().iter().find(|(k, _)| *k == key) {
        return hit.clone();
    }
    let out = check_now(&program, &spec, &parts, check);
    let mut c = cache().lock().unwrap();
    c.retain(|(k, _)| *k != key);
    c.push((key, out.clone()));
    if c.len() > 64 {
        c.drain(..1);
    }
    out
}

fn check_now(program: &ShapeProgram, spec: &fea::StudySpec, parts: &[(Option<String>, MaterialProps)], check: Check) -> Result<Checked, String> {
    let study = study_from_spec(program, spec)?;
    let fmt = |v: f64, d: usize| format!("{v:.d$}");
    match check {
        Check::Safety | Check::Deflection => {
            let r = solve_study(program, parts, &study)?;
            Ok(match check {
                Check::Safety => {
                    let worst = r.parts.iter().min_by(|a, b| a.safety.total_cmp(&b.safety)).map(|p| format!(" (“{}”)", p.name)).unwrap_or_default();
                    Checked { value: r.safety, summary: format!("Safety ×{} against yield{worst}: {} MPa at most, {} MPa yield", fmt(r.safety, 2), fmt(r.max_stress, 1), fmt(r.yield_strength, 0)), at: Some(r.max_stress_at) }
                }
                _ => Checked { value: r.max_displacement, summary: format!("Moves {} mm at most under the study's loads", fmt(r.max_displacement, 3)), at: Some(r.max_displacement_at) },
            })
        }
        Check::Frequency => {
            let m = modes_of(program, &parts[0], &study, 1, spec.prestress)?;
            let f = m.modes.first().map(|m| m.frequency).ok_or("no vibration mode found")?;
            Ok(Checked { value: f, summary: format!("First natural frequency {} Hz ({} rpm){}", fmt(f, 1), fmt(f * 60.0, 0), if spec.prestress { ", prestressed by the loads" } else { "" }), at: None })
        }
        Check::Buckling => {
            let b = buckling_of(program, &parts[0], &study, 1)?;
            let f = b.modes.first().map(|m| m.factor).unwrap_or(f64::INFINITY);
            Ok(Checked { value: f, summary: if f.is_finite() { format!("Buckles at {} × the study's loads (yields at {} ×)", fmt(f, 2), fmt(b.yield_factor, 2)) } else { "Does not buckle under the study's loads".into() }, at: None })
        }
    }
}

/// The meshes of the parts studied, each with its material.
fn with_bodies<T>(program: &ShapeProgram, parts: &[(Option<String>, MaterialProps)], run: impl FnOnce(&[fea::Body]) -> Result<T, String>) -> Result<T, String> {
    let e = evaluated_at(program, program.display_tolerance())?;
    let bodies = parts
        .iter()
        .map(|(name, m)| {
            let (start, count) = match name {
                Some(n) => {
                    let p = e.part(n)?;
                    (p.index_start, p.index_count)
                }
                None if e.parts.len() > 1 => return Err("pick one part of the assembly to study".to_string()),
                None => (0, e.indices.len()),
            };
            Ok(fea::Body { name: name.clone().unwrap_or_default(), positions: &e.positions, triangles: &e.indices[start..start + count], material: m.elastic() })
        })
        .collect::<Result<Vec<_>, String>>()?;
    run(&bodies)
}

/// A stress study on the parts (the first is the studied part), heat from
/// a heat study run first on it when the study asks.
pub fn solve_study(program: &ShapeProgram, parts: &[(Option<String>, MaterialProps)], study: &fea::Study) -> Result<fea::Result_, String> {
    with_bodies(program, parts, |bodies| {
        let mut study = study.clone();
        if let Some(heat) = study.heat.as_mut() {
            if let Some(mut thermal) = heat.thermal.take() {
                thermal.resolution = study.resolution;
                heat.field = Some(fea::thermal(bodies[0].positions, bodies[0].triangles, parts[0].1.conductivity, &thermal).map_err(|e| format!("the heat study: {e}"))?);
            }
        }
        fea::solve_parts(bodies, &study)
    })
}

/// Natural frequencies of the studied part.
pub fn modes_of(program: &ShapeProgram, part: &(Option<String>, MaterialProps), study: &fea::Study, count: usize, prestress: bool) -> Result<fea::Modes, String> {
    with_bodies(program, std::slice::from_ref(part), |b| fea::modes_study(&b[0], study, count, prestress))
}

/// Buckling of the studied part under the study's loads.
pub fn buckling_of(program: &ShapeProgram, part: &(Option<String>, MaterialProps), study: &fea::Study, count: usize) -> Result<fea::Buckling, String> {
    with_bodies(program, std::slice::from_ref(part), |b| fea::buckling(&b[0], study, count))
}

/// Steady temperatures in the studied part.
pub fn thermal_of(program: &ShapeProgram, part: &(Option<String>, MaterialProps), study: &fea::ThermalStudy) -> Result<fea::Temperatures, String> {
    let k = part.1.conductivity;
    with_bodies(program, std::slice::from_ref(part), |b| fea::thermal(b[0].positions, b[0].triangles, k, study))
}

/// Temperatures through time in the studied part.
pub fn transient_of(program: &ShapeProgram, part: &(Option<String>, MaterialProps), study: &fea::ThermalStudy, run: &fea::Transient, probes: &[[f64; 3]]) -> Result<fea::TransientTemperatures, String> {
    let m = &part.1;
    with_bodies(program, std::slice::from_ref(part), |b| fea::thermal_transient(b[0].positions, b[0].triangles, m.conductivity, m.density, m.specific_heat, study, run, probes))
}

/// The study as a CalculiX deck.
pub fn inp_of(program: &ShapeProgram, parts: &[(Option<String>, MaterialProps)], study: &fea::Study, kind: fea::InpKind, count: usize) -> Result<String, String> {
    with_bodies(program, parts, |bodies| fea::calculix_inp(bodies, study, kind, count))
}

/// One row of a convergence report.
#[derive(Clone, Debug, Serialize)]
pub struct ConvergenceRow {
    pub resolution: usize,
    pub cell: f64,
    pub elements: usize,
    pub max_stress: f64,
    pub max_displacement: f64,
    /// Change from the row before (fractions).
    pub stress_change: Option<f64>,
    pub displacement_change: Option<f64>,
    pub seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Convergence {
    pub rows: Vec<ConvergenceRow>,
    /// The movement extrapolated to infinitely fine cubes (Richardson, from the last three).
    pub displacement_limit: Option<f64>,
    pub converged: bool,
    pub notes: Vec<String>,
}

/// The study solved on ever finer cubes: how much the answer still moves.
pub fn convergence(program: &ShapeProgram, parts: &[(Option<String>, MaterialProps)], study: &fea::Study, resolutions: &[usize]) -> Result<Convergence, String> {
    let mut rows: Vec<ConvergenceRow> = vec![];
    for &res in resolutions.iter().take(5) {
        let started = std::time::Instant::now();
        let r = solve_study(program, parts, &fea::Study { resolution: res, ..study.clone() })?;
        let change = |a: f64, b: Option<f64>| b.map(|b| (a - b).abs() / a.abs().max(1e-300));
        let last = rows.last();
        rows.push(ConvergenceRow {
            resolution: res,
            cell: r.cell,
            elements: r.elements,
            max_stress: r.max_stress,
            max_displacement: r.max_displacement,
            stress_change: change(r.max_stress, last.map(|l| l.max_stress)),
            displacement_change: change(r.max_displacement, last.map(|l| l.max_displacement)),
            seconds: started.elapsed().as_secs_f64(),
        });
    }
    let n = rows.len();
    // Richardson from the last two, taking the error to fall as the cube size squared.
    let displacement_limit = (n >= 2).then(|| {
        let (b, c) = (rows[n - 2].max_displacement, rows[n - 1].max_displacement);
        let (hb, hc) = (rows[n - 2].cell, rows[n - 1].cell);
        c + (c - b) * hc * hc / (hb * hb - hc * hc).max(1e-300)
    });
    let converged = rows.last().is_some_and(|r| r.displacement_change.is_some_and(|d| d < 0.02) && r.stress_change.is_some_and(|s| s < 0.05));
    let mut notes = vec![];
    notes.push(if converged { "Converged: the last refinement moved the movement under 2 % and the largest stress under 5 %.".to_string() } else { "Not yet converged: the answer still moves with finer cubes; trust it less, or study finer.".to_string() });
    if rows.iter().any(|r| r.stress_change.is_some_and(|s| s > 0.15)) {
        notes.push("The largest stress keeps rising with finer cubes: it may sit at a sharp inside corner (no finite peak) or a point load; a fillet gives a real number.".into());
    }
    Ok(Convergence { rows, displacement_limit, converged, notes })
}

/// A whole study run kept for reading back (probes, sections, CSV, fatigue) without solving again.
pub struct Kept {
    pub result: Arc<fea::Result_>,
    pub material: MaterialProps,
}

fn kept() -> &'static Mutex<Vec<(u64, Arc<Kept>)>> {
    static KEPT: OnceLock<Mutex<Vec<(u64, Arc<Kept>)>>> = OnceLock::new();
    KEPT.get_or_init(|| Mutex::new(Vec::new()))
}

/// Keeps a solved study (the last few) under a key for reading back.
pub fn keep(result: fea::Result_, material: MaterialProps) -> u64 {
    let key = key_of(&[serde_json::to_value(&result.max_stress).unwrap_or_default(), serde_json::to_value(&result.max_displacement_at).unwrap_or_default(), serde_json::to_value(result.elements).unwrap_or_default(), serde_json::to_value(std::time::SystemTime::now()).unwrap_or_default()]);
    let mut k = kept().lock().unwrap();
    k.push((key, Arc::new(Kept { result: Arc::new(result), material })));
    if k.len() > 6 {
        k.remove(0);
    }
    key
}

/// A kept study by its key.
pub fn kept_result(key: u64) -> Option<Arc<Kept>> {
    kept().lock().unwrap().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
}
