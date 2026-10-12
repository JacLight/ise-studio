//! Interference across a whole assembly, at every depth: every pair of
//! placed parts that overlap (with how much volume, and where), that touch,
//! and (asked for) that come closer than a clearance. Pairs a joint joins
//! directly are marked: contact there is usually meant (a pin in its bore),
//! overlap rarely is.

use crate::joints;
use crate::transform::V3;
use crate::{AssemblyDef, Instance, Occurrence};
use de_core::{IdIndex, StableId, bevy_ecs::world::World};
use de_shape::ShapeSolid;
use manifold_csg::Manifold;
use serde::Serialize;

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

#[derive(Clone, Debug, Serialize)]
pub struct Clash {
    /// The two parts, named down the tree ("Arm › Elbow servo").
    pub a: String,
    pub b: String,
    /// Their instance paths from the top of what was checked.
    pub a_path: Vec<StableId>,
    pub b_path: Vec<StableId>,
    /// mm³ of overlap (0 for touching or near pairs).
    pub volume: f64,
    /// The overlap's (or the closest approach's) box middle and size (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<V3>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<V3>,
    /// mm apart (near pairs).
    pub gap: f64,
    /// A joint joins the two instances directly.
    pub joined: bool,
    /// One of them is a screw, insert or nut threading into the other: overlap there is meant.
    pub fastener: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Interference {
    pub of: Option<StableId>,
    pub name: String,
    pub parts: usize,
    /// Pairs whose boxes met and were checked closely.
    pub checked: usize,
    pub overlaps: Vec<Clash>,
    pub touching: Vec<Clash>,
    /// Closer than `clearance` (and not touching).
    pub near: Vec<Clash>,
    pub clearance: f64,
    pub notes: Vec<String>,
}

struct Leaf {
    names: String,
    path: Vec<StableId>,
    solid: Manifold,
    lo: V3,
    hi: V3,
}

fn leaf(world: &World, o: &Occurrence) -> Result<Option<Leaf>, String> {
    let se = entity(world, o.shape).ok_or("a component is gone")?;
    let solid = match &o.config {
        Some(c) => {
            let p = de_shape::configs::configured_program(world, se, c)?;
            de_shape::evaluated_at(&p, p.display_tolerance())?.solid.translate(0.0, 0.0, 0.0)
        }
        None => match world.get::<ShapeSolid>(se) {
            Some(s) => (*s.solid).clone(),
            None => return Err(format!("{} has no valid result", o.names.join(" › "))),
        },
    };
    let placed = o.transform.solid(&solid);
    let Some(bb) = placed.bounding_box() else { return Ok(None) };
    if placed.is_empty() {
        return Ok(None);
    }
    Ok(Some(Leaf { names: o.names.join(" › "), path: o.path.clone(), solid: placed, lo: bb.min(), hi: bb.max() }))
}

/// Whether a joint joins two sibling instances directly.
fn joined(world: &World, a: StableId, b: StableId) -> bool {
    let Some(scope) = entity(world, a).and_then(|e| world.get::<Instance>(e)).map(|i| i.parent) else { return false };
    joints::joints_in(world, scope).iter().any(|(_, j)| {
        let (x, y) = (j.a.instance(), j.b.instance());
        (x == Some(a) && y == Some(b)) || (x == Some(b) && y == Some(a))
    })
}

/// Every pair of parts in `of` (an assembly, an instance, or the whole top
/// of the project) that overlap, touch, or come within `clearance` mm.
pub fn report(world: &World, of: Option<StableId>, clearance: f64) -> Result<Interference, String> {
    let mut occ: Vec<Occurrence> = vec![];
    let name;
    match of {
        Some(id) => {
            let e = entity(world, id).ok_or_else(|| format!("{id} does not exist"))?;
            name = world.get::<AssemblyDef>(e).map(|a| a.name.clone()).or_else(|| world.get::<Instance>(e).map(|i| i.name.clone())).ok_or_else(|| format!("{id} is not an assembly or an instance"))?;
            occ = crate::occurrences(world, id);
        }
        None => {
            name = "the project".into();
            let mut tops: Vec<StableId> = world.resource::<IdIndex>().iter().filter(|(_, e)| world.get::<Instance>(*e).is_some_and(|i| i.parent.is_none())).map(|(id, _)| id).collect();
            tops.sort();
            for t in tops {
                occ.extend(crate::occurrences(world, t));
            }
        }
    }
    let leaves: Vec<Leaf> = occ.iter().map(|o| leaf(world, o)).collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect();
    let clearance = if clearance.is_finite() { clearance.max(0.0) } else { 0.0 };
    let mut out = Interference { of, name, parts: leaves.len(), checked: 0, overlaps: vec![], touching: vec![], near: vec![], clearance, notes: vec![] };
    let reach = clearance.max(1e-3);
    for i in 0..leaves.len() {
        for k in i + 1..leaves.len() {
            let (a, b) = (&leaves[i], &leaves[k]);
            if (0..3).any(|d| a.hi[d] + reach < b.lo[d] || b.hi[d] + reach < a.lo[d]) {
                continue;
            }
            out.checked += 1;
            // Where their paths part: two sibling instances.
            let split = a.path.iter().zip(&b.path).position(|(x, y)| x != y);
            let is_joined = split.is_some_and(|s| joined(world, a.path[s], b.path[s]));
            let fastener = [a.path.last(), b.path.last()].into_iter().flatten().any(|i| crate::fit::is_fastener(world, *i));
            let clash = |volume: f64, at: Option<V3>, size: Option<V3>, gap: f64| Clash {
                a: a.names.clone(),
                b: b.names.clone(),
                a_path: a.path.clone(),
                b_path: b.path.clone(),
                volume: (volume * 1000.0).round() / 1000.0,
                at,
                size,
                gap: (gap * 1e4).round() / 1e4,
                joined: is_joined,
                fastener,
            };
            let both = a.solid.intersection(&b.solid);
            let v = if both.is_empty() { 0.0 } else { both.volume() };
            if v > 1e-3 {
                let bb = both.bounding_box();
                let (at, size) = match bb {
                    Some(bb) => {
                        let (lo, hi) = (bb.min(), bb.max());
                        (Some([0, 1, 2].map(|d| ((lo[d] + hi[d]) / 2.0 * 1000.0).round() / 1000.0)), Some([0, 1, 2].map(|d| ((hi[d] - lo[d]) * 1000.0).round() / 1000.0)))
                    }
                    None => (None, None),
                };
                out.overlaps.push(clash(v, at, size, 0.0));
                continue;
            }
            let g = a.solid.min_gap(&b.solid, reach);
            if g < 1e-4 {
                out.touching.push(clash(0.0, None, None, 0.0));
            } else if clearance > 0.0 && g < clearance {
                out.near.push(clash(0.0, None, None, g));
            }
        }
    }
    out.overlaps.sort_by(|x, y| y.volume.total_cmp(&x.volume));
    out.near.sort_by(|x, y| x.gap.total_cmp(&y.gap));
    let unmeant = out.overlaps.iter().filter(|c| !c.joined && !c.fastener).count();
    out.notes.push(if out.overlaps.is_empty() {
        format!("No parts overlap among {} parts.", out.parts)
    } else {
        format!("{} overlapping pair{}: {} not meant (neither joined by a joint nor a fastener threading in).", out.overlaps.len(), if out.overlaps.len() == 1 { "" } else { "s" }, unmeant)
    });
    Ok(out)
}
