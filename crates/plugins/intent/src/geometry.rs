//! Geometry queries for requirements, in millimeters.
//!
//! Boxes use exact closed-form math. AI-made shapes use the evaluated solid:
//! interference is the exact boolean intersection volume and clearance is
//! Manifold's minimum gap, so a nut threaded on a bolt is judged by its
//! actual threads, not its bounding box.

use de_core::{IdIndex, StableId, bevy_ecs::world::World};
use de_shape::{ShapeProgram, ShapeSolid};
use de_solid::{BoxFeature, MassProperties, Placement};
use manifold_csg::Manifold;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    pub fn size(&self) -> [f64; 3] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1], self.max[2] - self.min[2]]
    }

    pub fn center(&self, axis: usize) -> f64 {
        (self.min[axis] + self.max[axis]) / 2.0
    }

    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: [0, 1, 2].map(|k| self.min[k].min(other.min[k])),
            max: [0, 1, 2].map(|k| self.max[k].max(other.max[k])),
        }
    }

    /// Per-axis separation (positive gap) or penetration (negative).
    fn separation(&self, other: &Aabb) -> [f64; 3] {
        [0, 1, 2].map(|k| (other.min[k] - self.max[k]).max(self.min[k] - other.max[k]))
    }

    /// Shortest distance between the boxes; 0 when touching or overlapping.
    pub fn distance(&self, other: &Aabb) -> f64 {
        self.separation(other).iter().map(|s| s.max(0.0).powi(2)).sum::<f64>().sqrt()
    }

    /// The shared volume, if the boxes overlap with positive volume.
    pub fn overlap(&self, other: &Aabb) -> Option<Aabb> {
        let min = [0, 1, 2].map(|k| self.min[k].max(other.min[k]));
        let max = [0, 1, 2].map(|k| self.max[k].min(other.max[k]));
        (0..3).all(|k| max[k] - min[k] > EPS).then_some(Aabb { min, max })
    }

    /// Smallest single-axis move of `other` that leaves at least `gap` between
    /// the boxes. Returns (axis, signed distance).
    pub fn push_apart(&self, other: &Aabb, gap: f64) -> (usize, f64) {
        let sep = self.separation(other);
        let direction = |k: usize| if other.center(k) >= self.center(k) { 1.0 } else { -1.0 };
        if sep.iter().all(|s| *s <= EPS) {
            // Overlapping or touching: escape along the axis of least penetration.
            let k = (0..3).max_by(|a, b| sep[*a].total_cmp(&sep[*b])).unwrap();
            (k, direction(k) * (gap - sep[k]))
        } else {
            // Separated: widen the largest existing gap until it alone suffices.
            let k = (0..3).max_by(|a, b| sep[*a].total_cmp(&sep[*b])).unwrap();
            (k, direction(k) * (gap - sep[k]).max(0.0))
        }
    }
}

pub const EPS: f64 = 1e-9;

/// A part as the intent checks see it.
pub struct PartView {
    pub id: StableId,
    /// World-space bounds.
    pub aabb: Aabb,
    pub height_mm: f64,
    pub mass_kg: Option<f64>,
    pub placement_mm: [f64; 3],
    /// For AI-made shapes: the display solid (program coordinates) and the program,
    /// so checks can re-derive it as finely as they need.
    pub shape: Option<(Arc<Manifold>, ShapeProgram)>,
    /// For one part of an assembly: its name (the solid is that part alone).
    pub part: Option<String>,
    /// Rotation, applied before `placement_mm`.
    pub rotation: de_assembly::M3,
    /// For an instance or assembly: every shape inside it, placed.
    pub members: Vec<PartView>,
    /// Instance path names, outermost first (for messages).
    pub path: Vec<String>,
}

impl PartView {
    pub fn is_box(&self) -> bool {
        self.shape.is_none() && self.members.is_empty()
    }

    /// The part as a solid in world coordinates, with curves within `tolerance` mm
    /// of the true formula (boxes are exact at any tolerance).
    pub fn solid(&self, tolerance: f64) -> Manifold {
        if !self.members.is_empty() {
            return Manifold::batch_union(&self.members.iter().map(|m| m.solid(tolerance)).collect::<Vec<_>>());
        }
        match &self.shape {
            Some((display, program)) => {
                let fine = de_shape::evaluated_at(program, tolerance).ok();
                let solid = match (&self.part, fine) {
                    (Some(name), Some(e)) => e.parts.iter().find(|p| &p.name == name).map(|p| p.solid.translate(0.0, 0.0, 0.0)),
                    (None, Some(e)) => Some(e.solid.translate(0.0, 0.0, 0.0)),
                    _ => None,
                }
                .unwrap_or_else(|| display.translate(0.0, 0.0, 0.0));
                de_assembly::Transform { at: self.placement_mm, rot: self.rotation }.solid(&solid)
            }
            None => {
                let s = self.aabb.size();
                Manifold::cube(s[0], s[1], s[2], false).translate(self.aabb.min[0], self.aabb.min[1], self.aabb.min[2])
            }
        }
    }
}

pub fn part(world: &World, id: StableId) -> Result<PartView, String> {
    let entity = world.resource::<IdIndex>().get(id).ok_or_else(|| format!("part {id} does not exist"))?;
    let p = world.get::<Placement>(entity).copied().unwrap_or_default();
    let mm = |l: de_core::Length| l.canonical() * 1000.0;
    let at = [mm(p.x), mm(p.y), mm(p.z)];
    let mass_kg = world.get::<MassProperties>(entity).and_then(|m| m.mass_kg);
    if let Some(b) = world.get::<BoxFeature>(entity) {
        let size = [mm(b.width), mm(b.depth), mm(b.height)];
        return Ok(PartView {
            id,
            aabb: Aabb { min: at, max: [at[0] + size[0], at[1] + size[1], at[2] + size[2]] },
            height_mm: size[2],
            mass_kg,
            placement_mm: at,
            shape: None,
            part: None,
            rotation: de_assembly::Transform::IDENTITY.rot,
            members: Vec::new(),
            path: Vec::new(),
        });
    }
    if world.get::<de_assembly::Instance>(entity).is_some() || world.get::<de_assembly::AssemblyDef>(entity).is_some() {
        return compound(world, id, entity);
    }
    let program = de_shape::resolved_program(world, entity).ok_or_else(|| format!("{id} is not a part"))?;
    let program = &program;
    let solid = world.get::<ShapeSolid>(entity).ok_or_else(|| format!("{} has not been evaluated", program.name))?;
    let (lo, hi) = (solid.stats.bounds_min, solid.stats.bounds_max);
    Ok(PartView {
        id,
        aabb: Aabb { min: [0, 1, 2].map(|k| at[k] + lo[k]), max: [0, 1, 2].map(|k| at[k] + hi[k]) },
        height_mm: hi[2] - lo[2],
        mass_kg,
        placement_mm: at,
        shape: Some((solid.solid.clone(), program.clone())),
        part: None,
        rotation: de_assembly::Transform::IDENTITY.rot,
        members: Vec::new(),
        path: Vec::new(),
    })
}

/// An instance or assembly: every shape inside it, placed in project coordinates.
fn compound(world: &World, id: StableId, entity: de_core::bevy_ecs::entity::Entity) -> Result<PartView, String> {
    let name = world
        .get::<de_assembly::Instance>(entity)
        .map(|i| i.name.clone())
        .or_else(|| world.get::<de_assembly::AssemblyDef>(entity).map(|a| a.name.clone()))
        .unwrap_or_default();
    let mut members = Vec::new();
    for o in de_assembly::occurrences(world, id) {
        let se = world.resource::<IdIndex>().get(o.shape).ok_or_else(|| format!("{} does not exist", o.names.join(" › ")))?;
        let program = de_shape::resolved_program(world, se).ok_or("not a shape")?;
        let program = &program;
        let solid = world.get::<ShapeSolid>(se).ok_or_else(|| format!("{} has not been evaluated", o.names.join(" › ")))?;
        let (lo, hi) = o.transform.bounds(solid.stats.bounds_min, solid.stats.bounds_max);
        let mut path = o.names.clone();
        path.pop();
        members.push(PartView {
            id,
            aabb: Aabb { min: lo, max: hi },
            height_mm: hi[2] - lo[2],
            mass_kg: world.get::<MassProperties>(se).and_then(|m| m.mass_kg),
            placement_mm: o.transform.at,
            shape: Some((solid.solid.clone(), program.clone())),
            part: None,
            rotation: o.transform.rot,
            members: Vec::new(),
            path,
        });
    }
    if members.is_empty() {
        return Err(format!("{name} is empty"));
    }
    let aabb = members.iter().skip(1).fold(members[0].aabb, |a, m| a.union(&m.aabb));
    let mass_kg = members.iter().map(|m| m.mass_kg).sum::<Option<f64>>();
    Ok(PartView {
        id,
        aabb,
        height_mm: aabb.max[2] - aabb.min[2],
        mass_kg,
        placement_mm: [0.0; 3],
        shape: None,
        part: None,
        rotation: de_assembly::Transform::IDENTITY.rot,
        members,
        path: vec![name],
    })
}

/// Splits instances and assemblies into their shapes, and multi-part shapes
/// into their parts, so each can be checked against every other.
pub fn expand(ps: Vec<PartView>) -> Vec<PartView> {
    let mut out = Vec::new();
    for p in ps {
        if !p.members.is_empty() {
            out.extend(expand(p.members));
            continue;
        }
        let Some((_, program)) = &p.shape else {
            out.push(p);
            continue;
        };
        let parts = match de_shape::evaluated_at(program, program.display_tolerance()) {
            Ok(e) if !e.parts.is_empty() => e,
            _ => {
                out.push(p);
                continue;
            }
        };
        let t = de_assembly::Transform { at: p.placement_mm, rot: p.rotation };
        for q in &parts.parts {
            let (lo, hi) = t.bounds(q.bounds_min, q.bounds_max);
            out.push(PartView {
                id: p.id,
                aabb: Aabb { min: lo, max: hi },
                height_mm: hi[2] - lo[2],
                mass_kg: None,
                placement_mm: p.placement_mm,
                shape: Some((Arc::new(q.solid.translate(0.0, 0.0, 0.0)), program.clone())),
                part: Some(q.name.clone()),
                rotation: p.rotation,
                members: Vec::new(),
                path: p.path.clone(),
            });
        }
    }
    out
}

/// Accuracy for checking shapes against each other: at least 20× finer than
/// the limit being proven (0.02 mm clearance → ±0.001 mm), between 0.5 and 5 µm.
/// Interference checks (limit 0) use 2 µm.
pub fn check_tolerance(limit: f64) -> f64 {
    if limit > 0.0 { (limit / 20.0).clamp(0.0005, 0.005) } else { 0.002 }
}

/// Accuracy for an interference check between two parts: 2 µm, or a fifth
/// of how finely the coarser part is drawn when that is larger. A bolt
/// (shown at ±0.01 mm) is checked to ±2 µm, a tower (±5 mm) to ±1 mm.
pub fn collision_tolerance(a: &PartView, b: &PartView) -> f64 {
    let shown = |p: &PartView| p.shape.as_ref().map(|(_, program)| program.display_tolerance()).unwrap_or(0.0);
    check_tolerance(0.0).max(shown(a).max(shown(b)) / 5.0)
}

/// Interference volume (mm³) with its region, and the minimum gap (mm, 0 when
/// touching or overlapping; `search` when farther than `search`). Shapes are
/// re-derived with curves within `tolerance` mm of their formulas.
pub fn contact(a: &PartView, b: &PartView, search: f64, tolerance: f64) -> (Option<(f64, Aabb)>, f64) {
    if a.is_box() && b.is_box() {
        let o = a.aabb.overlap(&b.aabb).map(|o| (o.size().iter().product(), o));
        return (o, a.aabb.distance(&b.aabb));
    }
    // Bounds apart by at least the search distance: the solids cannot touch.
    let bounds_gap = a.aabb.distance(&b.aabb);
    if bounds_gap > 0.0 && bounds_gap >= search {
        return (None, search.max(bounds_gap.min(search)));
    }
    let (sa, sb) = (a.solid(tolerance), b.solid(tolerance));
    let common = sa.intersection(&sb);
    let volume = common.volume();
    // Parts that meet face to face leave a sliver of numerical noise (worse
    // far from the origin). Interference is an overlap thicker than the check
    // accuracy, measured as mean thickness: 2 × volume / surface area.
    let thickness = if volume > 0.0 { 2.0 * volume / common.surface_area().max(1e-12) } else { 0.0 };
    if volume > 1e-6 && thickness > tolerance {
        let region = common.bounding_box().map(|bb| Aabb { min: bb.min(), max: bb.max() }).unwrap_or(a.aabb);
        return (Some((volume, region)), 0.0);
    }
    (None, sa.min_gap(&sb, search))
}

/// Parts' display name, matching the UI ("Box 40 × 40 × 20", or the shape's name).
pub fn name(p: &PartView) -> String {
    if !p.members.is_empty() {
        return p.path.join(" › ");
    }
    if let Some((_, program)) = &p.shape {
        let mut names = p.path.clone();
        names.push(program.name.clone());
        if let Some(part) = &p.part {
            names.push(part.clone());
        }
        return names.join(" › ");
    }
    let s = p.aabb.size();
    format!("Box {} × {} × {}", fmt(s[0]), fmt(s[1]), fmt(s[2]))
}

/// Up to two decimals, no trailing zeros, thousands grouped: 24150 → "24,150".
pub fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    let text = if r == r.trunc() { format!("{r:.0}") } else { format!("{r}") };
    let (sign, rest) = text.strip_prefix('-').map_or(("", text.as_str()), |t| ("-", t));
    let (int, frac) = rest.split_once('.').map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}{grouped}{}", frac.map(|f| format!(".{f}")).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(min: [f64; 3], size: [f64; 3]) -> Aabb {
        Aabb { min, max: [min[0] + size[0], min[1] + size[1], min[2] + size[2]] }
    }

    #[test]
    fn distance_and_overlap() {
        let a = b([0.0; 3], [10.0; 3]);
        assert_eq!(a.distance(&b([13.0, 0.0, 0.0], [5.0; 3])), 3.0);
        assert_eq!(a.distance(&b([13.0, 14.0, 0.0], [5.0; 3])), 5.0);
        assert_eq!(a.distance(&b([10.0, 0.0, 0.0], [5.0; 3])), 0.0);
        assert!(a.overlap(&b([10.0, 0.0, 0.0], [5.0; 3])).is_none(), "touching is not overlapping");
        let o = a.overlap(&b([8.0, 2.0, 0.0], [5.0; 3])).unwrap();
        assert_eq!(o.size(), [2.0, 5.0, 5.0]);
    }

    #[test]
    fn formats_numbers() {
        assert_eq!(fmt(24150.0), "24,150");
        assert_eq!(fmt(-1234567.891), "-1,234,567.89");
        assert_eq!(fmt(12.5), "12.5");
        assert_eq!(fmt(999.0), "999");
    }

    #[test]
    fn push_apart_resolves_both_cases() {
        let a = b([0.0; 3], [10.0; 3]);
        // Overlapping by 2 mm on X: move +X by 2 + gap.
        assert_eq!(a.push_apart(&b([8.0, 2.0, 0.0], [5.0; 3]), 1.0), (0, 3.0));
        // 0.5 mm apart on Y, need 2: move +Y by 1.5.
        let (k, d) = a.push_apart(&b([0.0, 10.5, 0.0], [5.0; 3]), 2.0);
        assert_eq!((k, d), (1, 1.5));
        // Other side: move negative.
        assert_eq!(a.push_apart(&b([-6.0, 0.0, 0.0], [5.0; 3]), 2.0), (0, -1.0));
    }
}
