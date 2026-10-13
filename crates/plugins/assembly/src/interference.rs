//! Interference across a whole assembly, at every depth: every pair of
//! placed parts that overlap (with how much volume, and where), that touch,
//! and (asked for) that come closer than a clearance. Pairs a joint joins
//! directly are marked: contact there is usually meant (a pin in its bore),
//! overlap rarely is. Parts declared embedded in others (rebar in its slab:
//! see `embedded`) are marked too, unless they poke out of what holds them.

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
    /// One is declared embedded in the other (rebar in its slab) and stays inside it: overlap there is meant.
    pub embedded: bool,
    /// mm³ of a part declared embedded in the other that pokes out of its hosts
    /// (0 when it stays inside), and the middle of what sticks out.
    #[serde(skip_serializing_if = "is_zero")]
    pub outside: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside_at: Option<V3>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
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
    /// Its shape's name, its part's (in a shape made of several) and the instances above it, by name.
    shape: String,
    part: Option<String>,
    path_names: Vec<String>,
    /// What it is declared embedded in (by name).
    hosts: Vec<String>,
}

impl Leaf {
    fn named(&self, host: &str) -> bool {
        crate::embedded::names(host, self.part.as_deref(), Some(&self.shape), &self.path_names)
    }
}

fn shape_name(world: &World, o: &Occurrence) -> String {
    entity(world, o.shape).and_then(|e| world.get::<de_shape::ShapeProgram>(e)).map(|p| p.name.clone()).unwrap_or_default()
}

fn path_names(o: &Occurrence) -> Vec<String> {
    o.names[..o.names.len().saturating_sub(1)].to_vec()
}

/// The leaves of one occurrence: each part of a shape made of several (a
/// building's walls, slabs, doors; a gearbox's housing and shafts), else the
/// shape whole.
fn leaves_of(world: &World, o: &Occurrence) -> Result<Vec<Leaf>, String> {
    let se = entity(world, o.shape).ok_or("a component is gone")?;
    let several = world.get::<ShapeSolid>(se).is_some_and(|s| s.stats.parts.len() >= 2);
    if !several {
        return Ok(leaf(world, o)?.into_iter().collect());
    }
    let program = match &o.config {
        Some(c) => de_shape::configs::configured_program(world, se, c)?,
        None => de_shape::resolved_program(world, se).ok_or("not a shape")?,
    };
    let ev = de_shape::evaluated_at(&program, program.display_tolerance())?;
    let mut out = vec![];
    for p in &ev.parts {
        let placed = o.transform.solid(&p.solid);
        let Some(bb) = placed.bounding_box() else { continue };
        if placed.is_empty() {
            continue;
        }
        let mut hosts = crate::embedded::on_path(world, &o.path);
        hosts.extend(crate::embedded::of_part(&ev.declared, Some(&p.name)));
        out.push(Leaf {
            names: format!("{} › {}", o.names.join(" › "), p.name),
            path: o.path.clone(),
            solid: placed,
            lo: bb.min(),
            hi: bb.max(),
            shape: shape_name(world, o),
            part: Some(p.name.clone()),
            path_names: path_names(o),
            hosts,
        });
    }
    Ok(out)
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
    Ok(Some(Leaf {
        names: o.names.join(" › "),
        path: o.path.clone(),
        solid: placed,
        lo: bb.min(),
        hi: bb.max(),
        shape: shape_name(world, o),
        part: None,
        path_names: path_names(o),
        hosts: crate::embedded::on_path(world, &o.path),
    }))
}

/// Every shape in an instance or assembly; a shape placed on its own where its placement puts it.
pub(crate) fn placed_occurrences(world: &World, id: StableId) -> Vec<Occurrence> {
    let mut occ = crate::occurrences(world, id);
    if let Some(p) = entity(world, id).filter(|e| world.get::<Instance>(*e).is_none()).and_then(|e| world.get::<de_solid::Placement>(e)) {
        let mm = |l: de_core::Length| l.canonical() * 1000.0;
        for o in &mut occ {
            o.transform.at = [o.transform.at[0] + mm(p.x), o.transform.at[1] + mm(p.y), o.transform.at[2] + mm(p.z)];
        }
    }
    occ
}

/// What stands at the top of the project: instances at the top, and shapes
/// placed on their own (a building, a part).
pub(crate) fn tops(world: &World) -> Vec<StableId> {
    let mut tops: Vec<StableId> = world
        .resource::<IdIndex>()
        .iter()
        .filter(|(_, e)| world.get::<Instance>(*e).is_some_and(|i| i.parent.is_none()) || (world.get::<de_shape::ShapeProgram>(*e).is_some() && world.get::<de_solid::Placement>(*e).is_some()))
        .map(|(id, _)| id)
        .collect();
    tops.sort();
    tops
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
            name = world
                .get::<AssemblyDef>(e)
                .map(|a| a.name.clone())
                .or_else(|| world.get::<Instance>(e).map(|i| i.name.clone()))
                .or_else(|| world.get::<de_shape::ShapeProgram>(e).map(|p| p.name.clone()))
                .ok_or_else(|| format!("{id} is not an assembly, an instance or a shape"))?;
            occ = placed_occurrences(world, id);
        }
        None => {
            name = "the project".into();
            for t in tops(world) {
                occ.extend(placed_occurrences(world, t));
            }
        }
    }
    let leaves: Vec<Leaf> = occ.iter().map(|o| leaves_of(world, o)).collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect();
    let clearance = if clearance.is_finite() { clearance.max(0.0) } else { 0.0 };
    let mut out = Interference { of, name, parts: leaves.len(), checked: 0, overlaps: vec![], touching: vec![], near: vec![], clearance, notes: vec![] };
    let reach = clearance.max(1e-3);
    // Each embedded part's hosts, and how much of it pokes out of them.
    let hosts: Vec<Vec<usize>> = leaves
        .iter()
        .enumerate()
        .map(|(i, l)| (0..leaves.len()).filter(|k| *k != i && l.hosts.iter().any(|h| leaves[*k].named(h))).collect())
        .collect();
    let poking: Vec<Option<(f64, V3, V3)>> = hosts
        .iter()
        .enumerate()
        .map(|(i, hs)| if hs.is_empty() { None } else { crate::embedded::outside(&leaves[i].solid, &hs.iter().map(|k| &leaves[*k].solid).collect::<Vec<_>>()) })
        .collect();
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
            // Declared embedded one in the other: meant, unless it pokes out of its hosts.
            let embeds: Vec<usize> = [(i, k), (k, i)].into_iter().filter(|(e, h)| hosts[*e].contains(h)).map(|(e, _)| e).collect();
            let poke = embeds.iter().find_map(|e| poking[*e]);
            let embedded = !embeds.is_empty() && poke.is_none();
            let mid = |lo: V3, hi: V3| [0, 1, 2].map(|d| ((lo[d] + hi[d]) / 2.0 * 1000.0).round() / 1000.0);
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
                embedded,
                outside: poke.map(|p| (p.0 * 1000.0).round() / 1000.0).unwrap_or(0.0),
                outside_at: poke.map(|p| mid(p.1, p.2)),
            };
            let both = a.solid.intersection(&b.solid);
            let v = if both.is_empty() { 0.0 } else { both.volume() };
            if v > 1e-3 {
                let bb = both.bounding_box();
                let (at, size) = match bb {
                    Some(bb) => {
                        let (lo, hi) = (bb.min(), bb.max());
                        (Some(mid(lo, hi)), Some([0, 1, 2].map(|d| ((hi[d] - lo[d]) * 1000.0).round() / 1000.0)))
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
    let unmeant = out.overlaps.iter().filter(|c| !c.joined && !c.fastener && !c.embedded).count();
    out.notes.push(if out.overlaps.is_empty() {
        format!("No parts overlap among {} parts.", out.parts)
    } else {
        format!("{} overlapping pair{}: {} not meant (not joined by a joint, a fastener threading in, or a part embedded in its host).", out.overlaps.len(), if out.overlaps.len() == 1 { "" } else { "s" }, unmeant)
    });
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    for (i, p) in poking.iter().enumerate() {
        if let Some((v, lo, hi)) = p {
            let host: Vec<&str> = hosts[i].iter().map(|k| leaves[*k].names.as_str()).collect();
            out.notes.push(format!(
                "{} is embedded in {} but {} mm³ of it pokes out ({} × {} × {} mm round {}, {}, {})",
                leaves[i].names,
                host.join(", "),
                r2(*v),
                r2(hi[0] - lo[0]),
                r2(hi[1] - lo[1]),
                r2(hi[2] - lo[2]),
                r2((lo[0] + hi[0]) / 2.0),
                r2((lo[1] + hi[1]) / 2.0),
                r2((lo[2] + hi[2]) / 2.0),
            ));
        }
    }
    Ok(out)
}
