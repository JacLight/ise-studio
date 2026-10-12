//! Couplings: one joint's motion driving another's — gears, belts and
//! chains, rack and pinion, lead screws, cams, or any ratio.
//!
//! A coupling names a driver joint and a follower joint in the same assembly
//! (or at the top). Before the joints are solved the follower's value is
//! worked out from the driver's: `follower = ratio × driver + offset`, or for
//! a cam the lift its profile gives at the driver's angle. Driving the
//! follower instead (sweeping it, scrubbing it in a motion study) turns the
//! driver back through the same ratio, so a gear train can be driven from
//! either end; a cam only runs forward. Gear trains chain: a coupling's
//! follower can drive the next.

use crate::bad;
use crate::joints::{self, Joint};
use de_core::{
    Command, CommandError, DependsOn, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use de_shape::params::ParameterSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CouplingKind {
    /// Meshing gears: the follower turns the other way, by teeth ratio.
    Gear,
    /// A belt or chain: same way round, by pulley teeth or diameters.
    Belt,
    /// A pinion turning (hinge) and a rack sliding (slider), by the pinion's pitch radius.
    RackPinion,
    /// A lead screw turning (hinge) and a nut sliding (slider), by the lead per turn.
    Screw,
    /// A cam turning and a follower lifting (or swinging) by a profile.
    Cam,
    /// Any linear ratio between two values.
    Ratio,
}

impl CouplingKind {
    pub fn word(self) -> &'static str {
        match self {
            CouplingKind::Gear => "gear",
            CouplingKind::Belt => "belt",
            CouplingKind::RackPinion => "rack and pinion",
            CouplingKind::Screw => "screw",
            CouplingKind::Cam => "cam",
            CouplingKind::Ratio => "ratio",
        }
    }
}

/// A coupling between two joints.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Coupling {
    pub name: String,
    pub kind: CouplingKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly: Option<StableId>,
    pub driver: StableId,
    pub follower: StableId,
    /// Which value of each joint (default: its main one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follower_key: Option<String>,
    /// Follower units per driver unit (degrees per degree, mm per degree…).
    pub ratio: f64,
    /// Added to the follower (its value when the driver is at zero).
    #[serde(default)]
    pub offset: f64,
    /// A cam's profile: [driver angle °, follower value], over a turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile: Vec<[f64; 2]>,
}

impl Persist for Coupling {
    const KIND: &'static str = "assembly.coupling";
}

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

/// Every coupling in an assembly (or at the top), oldest first.
pub fn couplings_in(world: &World, scope: Option<StableId>) -> Vec<(StableId, Coupling)> {
    let mut out: Vec<(StableId, Coupling)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Coupling>(e).filter(|c| c.assembly == scope).map(|c| (id, c.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// The cam profile's value at `angle` (degrees, periodic over a turn), by straight lines between points.
pub fn cam_at(profile: &[[f64; 2]], angle: f64) -> f64 {
    if profile.is_empty() {
        return 0.0;
    }
    if profile.len() == 1 {
        return profile[0][1];
    }
    let a = angle.rem_euclid(360.0);
    let n = profile.len();
    for i in 0..n {
        let (p, q) = (profile[i], profile[(i + 1) % n]);
        let q0 = if i + 1 == n { q[0] + 360.0 } else { q[0] };
        let a = if a < p[0] { a + 360.0 } else { a };
        if a >= p[0] && a <= q0 {
            let t = if q0 > p[0] { (a - p[0]) / (q0 - p[0]) } else { 0.0 };
            return p[1] + t * (q[1] - p[1]);
        }
    }
    profile[0][1]
}

fn key_index(world: &World, joint: StableId, key: Option<&str>) -> Result<(usize, &'static str), String> {
    let j = entity(world, joint).and_then(|e| world.get::<Joint>(e)).ok_or_else(|| format!("{joint} is not a joint"))?;
    let keys = j.kind.values();
    if keys.is_empty() {
        return Err(format!("{} is a {} joint: it has no value to couple", joints::joint_name(world, joint), j.kind.word()));
    }
    match key {
        None => Ok((0, keys[0].2)),
        Some(k) => keys.iter().position(|(x, _, _)| *x == k).map(|i| (i, keys[i].2)).ok_or_else(|| format!("{} has no value “{k}”", joints::joint_name(world, joint))),
    }
}

/// Every joint's values in the scope with the couplings applied: `given`
/// values (a sweep's, a study's) are kept and drive what they couple to;
/// the rest are the joints' own (formulas worked out). None when the scope
/// has no couplings.
pub fn coupled_values(world: &World, scope: Option<StableId>, given: &HashMap<StableId, Vec<f64>>) -> Result<Option<HashMap<StableId, Vec<f64>>>, String> {
    let list = couplings_in(world, scope);
    if list.is_empty() {
        return Ok(None);
    }
    let mut values: HashMap<StableId, Vec<f64>> = HashMap::new();
    for (j, joint) in joints::joints_in(world, scope) {
        if let Some(v) = given.get(&j) {
            values.insert(j, v.clone());
            continue;
        }
        let e = entity(world, j).ok_or("a joint is gone")?;
        let set = world.get::<ParameterSet>(e).ok_or("a joint has no values")?;
        let given_value = |id: StableId, key: &str| -> Option<f64> {
            let v = given.get(&id)?;
            let kind = entity(world, id).and_then(|e| world.get::<Joint>(e))?.kind;
            kind.values().iter().position(|(k, _, _)| *k == key).and_then(|i| v.get(i).copied())
        };
        let resolved = de_shape::params::resolve_with(world, &set.params, &given_value).map_err(|m| format!("{}: {m}", set.name))?;
        values.insert(j, joint.kind.values().iter().map(|(k, _, _)| resolved.get(*k).copied().unwrap_or(0.0)).collect());
    }
    let mut driven: HashSet<StableId> = given.keys().copied().collect();
    // Chains of couplings settle in a few passes.
    for _ in 0..=list.len() {
        let mut changed = false;
        for (_, c) in &list {
            let (di, _) = key_index(world, c.driver, c.driver_key.as_deref())?;
            let (fi, _) = key_index(world, c.follower, c.follower_key.as_deref())?;
            let backwards = driven.contains(&c.follower) && !driven.contains(&c.driver);
            if backwards {
                if c.kind == CouplingKind::Cam {
                    return Err(format!("{}: a cam only runs forward: drive {} instead", c.name, joints::joint_name(world, c.driver)));
                }
                let f = values.get(&c.follower).and_then(|v| v.get(fi)).copied().unwrap_or(0.0);
                let d = (f - c.offset) / c.ratio;
                if let Some(v) = values.get_mut(&c.driver)
                    && (v[di] - d).abs() > 1e-12
                {
                    v[di] = d;
                    changed = true;
                }
                driven.insert(c.driver);
            } else {
                let d = values.get(&c.driver).and_then(|v| v.get(di)).copied().unwrap_or(0.0);
                let f = if c.kind == CouplingKind::Cam { cam_at(&c.profile, d) + c.offset } else { c.ratio * d + c.offset };
                if let Some(v) = values.get_mut(&c.follower)
                    && (v[fi] - f).abs() > 1e-12
                {
                    v[fi] = f;
                    changed = true;
                }
                if driven.contains(&c.driver) {
                    driven.insert(c.follower);
                }
            }
        }
        if !changed {
            break;
        }
    }
    // A follower driven past its limits fails, as a formula would.
    for (_, c) in &list {
        for (j, key) in [(c.follower, c.follower_key.as_deref()), (c.driver, c.driver_key.as_deref())] {
            let Some(e) = entity(world, j) else { continue };
            let Some(set) = world.get::<ParameterSet>(e) else { continue };
            let (i, _) = key_index(world, j, key)?;
            let k = world.get::<Joint>(e).map(|x| x.kind.values()[i].0).unwrap_or("");
            let v = values.get(&j).and_then(|v| v.get(i)).copied().unwrap_or(0.0);
            if let Some(p) = set.params.iter().find(|p| p.key == k)
                && (p.min.is_some_and(|m| v < m - 1e-6) || p.max.is_some_and(|m| v > m + 1e-6))
            {
                return Err(format!("{} drives {} to {:.3} {}, past its limits {}…{}", c.name, set.name, v, p.unit, p.min.unwrap_or(f64::NEG_INFINITY), p.max.unwrap_or(f64::INFINITY)));
            }
        }
    }
    Ok(Some(values))
}

// ---- Commands ------------------------------------------------------------------------

/// `assembly.coupling.create`: drive one joint's value from another's.
#[derive(Deserialize, JsonSchema)]
pub struct CreateCoupling {
    #[serde(default)]
    pub assembly: Option<StableId>,
    #[serde(rename = "type")]
    pub kind: CouplingKind,
    /// The joint that drives (id or name).
    pub driver: String,
    /// The joint that follows (id or name).
    pub follower: String,
    pub name: Option<String>,
    pub driver_key: Option<String>,
    pub follower_key: Option<String>,
    /// Follower units per driver unit. Gears and belts may give `teeth` instead.
    pub ratio: Option<f64>,
    /// [driver teeth, follower teeth] (or pulley diameters for a belt).
    pub teeth: Option<[f64; 2]>,
    /// Rack and pinion: the pinion's pitch radius (mm).
    pub radius: Option<f64>,
    /// Screw: the lead, mm per turn.
    pub lead: Option<f64>,
    /// Cam: [driver angle °, follower value] points over a turn.
    #[serde(default)]
    pub profile: Vec<[f64; 2]>,
    #[serde(default)]
    pub offset: f64,
}

/// A joint by id or name, in the scope.
pub fn joint_ref(world: &World, scope: Option<StableId>, r: &str) -> Result<StableId, String> {
    let list = joints::joints_in(world, scope);
    if let Ok(id) = serde_json::from_value::<StableId>(json!(r))
        && list.iter().any(|(j, _)| *j == id)
    {
        return Ok(id);
    }
    list.iter().map(|(j, _)| *j).find(|j| joints::joint_name(world, *j) == r).ok_or_else(|| {
        let names: Vec<String> = list.iter().map(|(j, _)| joints::joint_name(world, *j)).collect();
        format!("no joint “{r}” here (there are {})", if names.is_empty() { "none".into() } else { names.join(", ") })
    })
}

#[allow(clippy::too_many_arguments)]
fn build(world: &World, kind: CouplingKind, scope: Option<StableId>, driver: StableId, follower: StableId, driver_key: Option<String>, follower_key: Option<String>, ratio: Option<f64>, teeth: Option<[f64; 2]>, radius: Option<f64>, lead: Option<f64>, profile: Vec<[f64; 2]>, offset: f64, name: String) -> Result<Coupling, String> {
    if driver == follower {
        return Err("a coupling joins two different joints".into());
    }
    let (_, du) = key_index(world, driver, driver_key.as_deref())?;
    let (_, fu) = key_index(world, follower, follower_key.as_deref())?;
    let (dn, fname) = (joints::joint_name(world, driver), joints::joint_name(world, follower));
    let turn = |u: &str| u == "deg";
    let ratio = match kind {
        CouplingKind::Gear | CouplingKind::Belt => {
            if !(turn(du) && turn(fu)) {
                return Err(format!("a {} couples two turning joints (hinges)", kind.word()));
            }
            let sign = if kind == CouplingKind::Gear { -1.0 } else { 1.0 };
            match (ratio, teeth) {
                (Some(r), _) => r,
                (None, Some([zd, zf])) if zd > 0.0 && zf > 0.0 => sign * zd / zf,
                _ => return Err(format!("a {} needs `teeth: [driver, follower]` or a `ratio`", kind.word())),
            }
        }
        CouplingKind::RackPinion | CouplingKind::Screw => {
            let per_degree = match kind {
                CouplingKind::RackPinion => radius.filter(|r| *r > 0.0 && r.is_finite()).map(|r| r * std::f64::consts::PI / 180.0).ok_or("a rack and pinion needs the pinion's pitch `radius` (mm)")?,
                _ => lead.filter(|l| *l != 0.0 && l.is_finite()).map(|l| l / 360.0).ok_or("a screw needs its `lead` (mm per turn)")?,
            };
            match (turn(du), turn(fu)) {
                (true, false) => ratio.unwrap_or(per_degree),
                (false, true) => ratio.unwrap_or(1.0 / per_degree),
                _ => return Err(format!("a {} couples a turning joint and a sliding one ({dn} and {fname})", kind.word())),
            }
        }
        CouplingKind::Cam => {
            if !turn(du) {
                return Err(format!("a cam turns: {dn} must be a hinge"));
            }
            if profile.len() < 2 || profile.windows(2).any(|w| w[1][0] <= w[0][0]) || profile.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) || profile.last().unwrap()[0] - profile[0][0] >= 360.0 {
                return Err("a cam needs a `profile` of two or more [angle, value] points, angles rising within one turn".into());
            }
            1.0
        }
        CouplingKind::Ratio => ratio.ok_or("give the `ratio` (follower units per driver unit)")?,
    };
    if !ratio.is_finite() || ratio == 0.0 {
        return Err("the ratio must be a finite number other than zero".into());
    }
    if !offset.is_finite() {
        return Err("offset must be finite".into());
    }
    Ok(Coupling { name, kind, assembly: scope, driver, follower, driver_key, follower_key, ratio, offset, profile })
}

fn unique_name(world: &World, kind: CouplingKind, wanted: Option<String>, except: Option<StableId>) -> Result<String, CommandError> {
    let taken = |n: &str| world.resource::<IdIndex>().iter().any(|(id, e)| Some(id) != except && world.get::<Coupling>(e).is_some_and(|c| c.name == n));
    match wanted.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        Some(n) if taken(&n) => Err(bad(ErrorCode::InvalidParameters, format!("there is already a coupling called “{n}”"))),
        Some(n) => Ok(n),
        None => {
            let base = match kind {
                CouplingKind::RackPinion => "Rack and pinion".to_string(),
                k => {
                    let w = k.word();
                    format!("{}{}", w[..1].to_uppercase(), &w[1..])
                }
            };
            Ok((1..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap())
        }
    }
}

fn refresh(tx: &mut Tx, scope: Option<StableId>) -> Result<(), CommandError> {
    joints::rewire(tx, scope)?;
    joints::remark(tx, scope)?;
    joints::solve_scope(tx.world(), scope, &HashMap::new(), &HashMap::new()).map(|_| ()).map_err(|m| bad(ErrorCode::InvalidGeometry, m))
}

impl Command for CreateCoupling {
    const NAME: &'static str = "assembly.coupling.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let w = tx.world();
        let driver = joint_ref(w, self.assembly, &self.driver).map_err(|m| bad(ErrorCode::UnresolvedReference, format!("driver: {m}")))?;
        let follower = joint_ref(w, self.assembly, &self.follower).map_err(|m| bad(ErrorCode::UnresolvedReference, format!("follower: {m}")))?;
        let name = unique_name(w, self.kind, self.name, None)?;
        let c = build(w, self.kind, self.assembly, driver, follower, self.driver_key, self.follower_key, self.ratio, self.teeth, self.radius, self.lead, self.profile, self.offset, name.clone()).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
        if couplings_in(w, self.assembly).iter().any(|(_, o)| o.follower == follower && o.follower_key == c.follower_key) {
            return Err(bad(ErrorCode::InvalidParameters, format!("{} already follows another coupling", joints::joint_name(w, follower))));
        }
        let id = tx.create();
        tx.set(id, c)?;
        tx.add_dependency(id, driver)?;
        tx.add_dependency(id, follower)?;
        refresh(tx, self.assembly)?;
        Ok(json!({ "id": id, "name": name }))
    }
}

/// `assembly.coupling.update`: change a coupling's ratio, teeth, offset, profile or name.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateCoupling {
    pub target: StableId,
    pub name: Option<String>,
    pub ratio: Option<f64>,
    pub teeth: Option<[f64; 2]>,
    pub radius: Option<f64>,
    pub lead: Option<f64>,
    pub profile: Option<Vec<[f64; 2]>>,
    pub offset: Option<f64>,
}

impl Command for UpdateCoupling {
    const NAME: &'static str = "assembly.coupling.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let c = tx.get::<Coupling>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a coupling", self.target)))?;
        let w = tx.world();
        let name = match self.name {
            Some(n) => unique_name(w, c.kind, Some(n), Some(self.target))?,
            None => c.name.clone(),
        };
        let keep_ratio = self.ratio.is_none() && self.teeth.is_none() && self.radius.is_none() && self.lead.is_none();
        let ratio = if keep_ratio { Some(c.ratio) } else { self.ratio };
        let n = build(w, c.kind, c.assembly, c.driver, c.follower, c.driver_key.clone(), c.follower_key.clone(), ratio, self.teeth, self.radius, self.lead, self.profile.unwrap_or(c.profile.clone()), self.offset.unwrap_or(c.offset), name).map_err(|m| bad(ErrorCode::InvalidParameters, m))?;
        tx.set(self.target, n)?;
        refresh(tx, c.assembly)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `assembly.coupling.delete`: the follower keeps its own value again.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteCoupling {
    pub target: StableId,
}

impl Command for DeleteCoupling {
    const NAME: &'static str = "assembly.coupling.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let c = tx.get::<Coupling>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a coupling", self.target)))?;
        delete(tx, self.target)?;
        refresh(tx, c.assembly)?;
        Ok(json!({ "id": self.target }))
    }
}

/// Deletes a coupling (nothing may keep depending on it).
pub(crate) fn delete(tx: &mut Tx, id: StableId) -> Result<(), CommandError> {
    let users: Vec<StableId> = tx.world().resource::<IdIndex>().iter().filter(|(_, e)| tx.world().get::<DependsOn>(*e).is_some_and(|d| d.0.contains(&id))).map(|(u, _)| u).collect();
    for u in users {
        tx.remove_dependency(u, id)?;
    }
    tx.delete(id)
}

/// Deletes the couplings that name a joint (it is being deleted).
pub(crate) fn delete_couplings_of(tx: &mut Tx, joint: StableId) -> Result<(), CommandError> {
    let ids: Vec<StableId> = tx.world().resource::<IdIndex>().iter().filter(|(_, e)| tx.world().get::<Coupling>(*e).is_some_and(|c| c.driver == joint || c.follower == joint)).map(|(id, _)| id).collect();
    for id in ids {
        delete(tx, id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cam_profiles_wrap_round() {
        let p = [[0.0, 0.0], [90.0, 5.0], [180.0, 5.0], [270.0, 0.0]];
        assert_eq!(cam_at(&p, 45.0), 2.5);
        assert_eq!(cam_at(&p, 135.0), 5.0);
        assert_eq!(cam_at(&p, 315.0), 0.0);
        assert_eq!(cam_at(&p, 405.0), 2.5);
        assert_eq!(cam_at(&p, -45.0), 0.0);
    }
}
