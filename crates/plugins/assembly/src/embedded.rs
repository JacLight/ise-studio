//! Parts embedded in others by design: rebar cast in a slab, an insert
//! moulded or pressed into a boss, a threaded insert in its hole, an
//! overmould on its core. Their overlap with what holds them is how they are
//! made, so the interference, fit and collision checks count it as meant —
//! as long as the part stays inside its host: where it pokes out (rebar with
//! no cover, an insert standing proud) it is still reported.
//!
//! Declared two ways:
//! - in a program, on a part: `part("Mesh", bars, { embedded: "Slab" })`
//!   (hosts are other parts of the same program; `rebar(…, { embedded })`
//!   passes it on);
//! - on an instance, the property `embedded`: "Slab" (or "Slab, Beam"):
//!   every shape under it is embedded in whatever those names name — an
//!   instance, a shape or a part, anywhere in what is checked.

use de_core::{IdIndex, StableId, bevy_ecs::world::World};
use manifold_csg::Manifold;

/// The instance property that declares what an instance is embedded in.
pub const PROPERTY: &str = "embedded";

/// Names in a declaration: "Slab, Beam" → ["Slab", "Beam"].
pub fn split(v: &str) -> Vec<String> {
    v.split([',', ';']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// What the instances along `path` declare their shapes embedded in.
pub fn on_path(world: &World, path: &[StableId]) -> Vec<String> {
    let index = world.resource::<IdIndex>();
    let mut out = vec![];
    for id in path {
        if let Some(v) = index.get(*id).and_then(|e| world.get::<crate::Instance>(e)).and_then(|i| i.properties.get(PROPERTY)) {
            out.extend(split(v));
        }
    }
    out
}

/// What a program declares one of its parts embedded in.
pub fn of_part(declared: &de_shape::stock::Declared, part: Option<&str>) -> Vec<String> {
    part.and_then(|p| declared.embedded.get(p)).cloned().unwrap_or_default()
}

/// Whether the host name `host` names this piece: its part, its shape, or
/// an instance on its path.
pub fn names(host: &str, part: Option<&str>, shape: Option<&str>, path_names: &[String]) -> bool {
    let same = |n: &str| n.trim().eq_ignore_ascii_case(host.trim());
    part.is_some_and(same) || shape.is_some_and(same) || path_names.iter().any(|n| same(n))
}

/// How much of `part` is outside its hosts (mm³), and where (the box of
/// what sticks out), when that is more than numerical noise.
pub fn outside(part: &Manifold, hosts: &[&Manifold]) -> Option<(f64, [f64; 3], [f64; 3])> {
    if hosts.is_empty() {
        return None;
    }
    let all = Manifold::batch_union(&hosts.iter().map(|h| (*h).clone()).collect::<Vec<_>>());
    let out = part.difference(&all);
    if out.is_empty() {
        return None;
    }
    let v = out.volume();
    // Faces flush with the host's leave slivers: noise is a millionth of the part.
    if v <= 0.01 + 1e-6 * part.volume().abs() {
        return None;
    }
    let bb = out.bounding_box()?;
    Some((v, bb.min(), bb.max()))
}
