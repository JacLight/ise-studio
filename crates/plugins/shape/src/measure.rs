//! Measuring a body: volume, surface area, centre of mass and moments of
//! inertia, integrated over its closed triangle mesh (exact for flat-faced
//! solids, as fine as the display tolerance on curved ones).

use crate::mechanism::{MassProps, mesh_mass};
use crate::{ShapeSolid, masses_of};
use de_core::bevy_ecs::{entity::Entity, world::World};
use de_core::schemars::{self, JsonSchema};
use serde::Serialize;

type V3 = [f64; 3];
type M3 = [[f64; 3]; 3];

/// What a body measures, in millimeters (and kg where a material applies).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Measured {
    pub volume_mm3: f64,
    pub area_mm2: f64,
    pub bounds_min: V3,
    pub bounds_max: V3,
    /// Centre of volume (mm); the centre of mass when densities are uniform.
    pub centroid: V3,
    /// Centre of mass (mm): parts weighted by their own densities.
    pub center_of_mass: V3,
    /// None until every part has a material.
    pub mass_kg: Option<f64>,
    /// Inertia tensor about the centre of mass, mesh axes (kg·mm²).
    pub inertia_kg_mm2: Option<M3>,
    /// Principal moments (kg·mm², smallest first) and their axes.
    pub principal_kg_mm2: Option<V3>,
    pub principal_axes: Option<[V3; 3]>,
    /// Second moments of the volume about the centroid (mm⁵): the inertia at unit density.
    pub inertia_mm5: M3,
}

/// Measures a closed mesh (mm); `density` (kg/m³) gives mass and inertia.
pub fn measure_mesh(positions: &[f32], triangles: &[u32], density: Option<f64>) -> Measured {
    measure_bodies(positions, &[(triangles, density)])
}

/// Measures several closed bodies together (each its triangles and density, kg/m³).
pub fn measure_bodies(positions: &[f32], bodies: &[(&[u32], Option<f64>)]) -> Measured {
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut area = 0.0;
    let mut geometric = vec![];
    let mut weighted = vec![];
    let mut all_dense = true;
    for (tris, density) in bodies {
        for t in tris.chunks_exact(3) {
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            let n = cross(sub(b, a), sub(c, a));
            area += 0.5 * dot(n, n).sqrt();
            for q in [a, b, c] {
                for k in 0..3 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
        }
        let unit = mesh_mass(positions, tris, 1.0);
        match density {
            // kg/m³ → kg/mm³.
            Some(d) => weighted.push(mesh_mass(positions, tris, d * 1e-9)),
            None => all_dense = false,
        }
        geometric.push(unit);
    }
    let unit = MassProps::combine(&geometric);
    let mass = (all_dense && !weighted.is_empty()).then(|| MassProps::combine(&weighted));
    let principal = mass.as_ref().map(|m| eigen(m.inertia));
    if lo[0] > hi[0] {
        lo = [0.0; 3];
        hi = [0.0; 3];
    }
    Measured {
        volume_mm3: unit.mass,
        area_mm2: area,
        bounds_min: lo,
        bounds_max: hi,
        centroid: unit.center,
        center_of_mass: mass.as_ref().map_or(unit.center, |m| m.center),
        mass_kg: mass.as_ref().map(|m| m.mass),
        inertia_kg_mm2: mass.as_ref().map(|m| m.inertia),
        principal_kg_mm2: principal.map(|(v, _)| v),
        principal_axes: principal.map(|(_, a)| a),
        inertia_mm5: unit.inertia,
    }
}

/// Measures a shape entity, or one part of an assembly shape by name.
pub fn measure(world: &World, entity: Entity, part: Option<&str>) -> Option<Result<Measured, String>> {
    let s = world.get::<ShapeSolid>(entity)?;
    Some((|| {
        let masses = masses_of(world, entity, &s.stats)?;
        let density = |k: usize, volume: f64| masses.get(k).copied().flatten().filter(|_| volume > 0.0).map(|m| m / volume * 1e9);
        if s.stats.parts.is_empty() {
            if let Some(name) = part {
                return Err(format!("this shape has no parts (asked for “{name}”)"));
            }
            return Ok(measure_mesh(&s.positions, &s.indices, density(0, s.stats.volume_mm3)));
        }
        let picked: Vec<usize> = match part {
            Some(name) => vec![s.stats.parts.iter().position(|p| p.name == name).ok_or_else(|| format!("no part called “{name}” (has {})", s.stats.parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))?],
            None => (0..s.stats.parts.len()).collect(),
        };
        let bodies: Vec<(&[u32], Option<f64>)> = picked
            .iter()
            .map(|&k| {
                let p = &s.stats.parts[k];
                (&s.indices[p.index_start..p.index_start + p.index_count], density(k, p.volume_mm3))
            })
            .collect();
        Ok(measure_bodies(&s.positions, &bodies))
    })())
}

/// A measured body turned by `rot` (rows: the matrix) and moved by `at`.
pub fn placed(m: &Measured, rot: M3, at: V3) -> Measured {
    let apply = |p: V3| {
        let r = [dot(rot[0], p), dot(rot[1], p), dot(rot[2], p)];
        [r[0] + at[0], r[1] + at[1], r[2] + at[2]]
    };
    // R I Rᵀ.
    let turn = |i: M3| {
        let mut out = [[0.0; 3]; 3];
        for a in 0..3 {
            for b in 0..3 {
                out[a][b] = (0..3).map(|k| (0..3).map(|l| rot[a][k] * i[k][l] * rot[b][l]).sum::<f64>()).sum();
            }
        }
        out
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for c in 0..8 {
        let q = apply([0, 1, 2].map(|k| if c & (1 << k) != 0 { m.bounds_max[k] } else { m.bounds_min[k] }));
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let inertia = m.inertia_kg_mm2.map(turn);
    let principal = inertia.map(eigen);
    Measured {
        volume_mm3: m.volume_mm3,
        area_mm2: m.area_mm2,
        bounds_min: lo,
        bounds_max: hi,
        centroid: apply(m.centroid),
        center_of_mass: apply(m.center_of_mass),
        mass_kg: m.mass_kg,
        inertia_kg_mm2: inertia,
        principal_kg_mm2: principal.map(|(v, _)| v),
        principal_axes: principal.map(|(_, a)| a),
        inertia_mm5: turn(m.inertia_mm5),
    }
}

/// Several measured bodies (in one frame) taken together.
pub fn combine(list: &[Measured]) -> Measured {
    if list.len() == 1 {
        return list[0].clone();
    }
    let unit: Vec<MassProps> = list.iter().map(|m| MassProps { mass: m.volume_mm3, center: m.centroid, inertia: m.inertia_mm5 }).collect();
    let unit = MassProps::combine(&unit);
    let massive: Option<Vec<MassProps>> = list.iter().map(|m| Some(MassProps { mass: m.mass_kg?, center: m.center_of_mass, inertia: m.inertia_kg_mm2? })).collect();
    let mass = massive.filter(|v| !v.is_empty()).map(|v| MassProps::combine(&v));
    let principal = mass.as_ref().map(|m| eigen(m.inertia));
    let lo = [0, 1, 2].map(|k| list.iter().map(|m| m.bounds_min[k]).fold(f64::INFINITY, f64::min));
    let hi = [0, 1, 2].map(|k| list.iter().map(|m| m.bounds_max[k]).fold(f64::NEG_INFINITY, f64::max));
    Measured {
        volume_mm3: unit.mass,
        area_mm2: list.iter().map(|m| m.area_mm2).sum(),
        bounds_min: if list.is_empty() { [0.0; 3] } else { lo },
        bounds_max: if list.is_empty() { [0.0; 3] } else { hi },
        centroid: unit.center,
        center_of_mass: mass.as_ref().map_or(unit.center, |m| m.center),
        mass_kg: mass.as_ref().map(|m| m.mass),
        inertia_kg_mm2: mass.as_ref().map(|m| m.inertia),
        principal_kg_mm2: principal.map(|(v, _)| v),
        principal_axes: principal.map(|(_, a)| a),
        inertia_mm5: unit.inertia,
    }
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric 3×3 matrix (Jacobi rotations).
pub fn eigen(m: M3) -> (V3, [V3; 3]) {
    let mut a = m;
    let mut v: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        let scale = a[0][0].abs() + a[1][1].abs() + a[2][2].abs();
        if off <= 1e-15 * scale.max(1e-300) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            // A ← Jᵀ A J, V ← V J.
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let values = order.map(|i| a[i][i]);
    let axes = order.map(|i| [v[0][i], v[1][i], v[2][i]]);
    (values, axes)
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A w×d×h box from `at` as 12 outward triangles.
    fn block(w: f32, d: f32, h: f32, at: [f32; 3]) -> (Vec<f32>, Vec<u32>) {
        let mut p = vec![];
        for k in 0..8 {
            p.extend([at[0] + if k & 1 != 0 { w } else { 0.0 }, at[1] + if k & 2 != 0 { d } else { 0.0 }, at[2] + if k & 4 != 0 { h } else { 0.0 }]);
        }
        let t = vec![0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6, 1, 3, 5, 3, 7, 5];
        (p, t)
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs().max(1.0)
    }

    #[test]
    fn a_60_40_20_box_measures_as_the_formulas_say() {
        let (p, t) = block(60.0, 40.0, 20.0, [10.0, -5.0, 0.0]);
        // PLA, 1240 kg/m³.
        let m = measure_mesh(&p, &t, Some(1240.0));
        assert!(near(m.volume_mm3, 48000.0, 1e-12), "volume {}", m.volume_mm3);
        assert!(near(m.area_mm2, 8800.0, 1e-12), "area {}", m.area_mm2);
        assert_eq!(m.bounds_min, [10.0, -5.0, 0.0]);
        assert_eq!(m.bounds_max, [70.0, 35.0, 20.0]);
        for (got, want) in m.centroid.iter().zip([40.0, 15.0, 10.0]) {
            assert!(near(*got, want, 1e-12), "centroid {:?}", m.centroid);
        }
        let mass = 48000.0 * 1240.0e-9;
        assert!(near(m.mass_kg.unwrap(), mass, 1e-12), "mass {:?}", m.mass_kg);
        // Box about its centre: m(b² + c²)/12 etc.
        let want = [mass * (40.0f64.powi(2) + 20.0f64.powi(2)) / 12.0, mass * (60.0f64.powi(2) + 20.0f64.powi(2)) / 12.0, mass * (60.0f64.powi(2) + 40.0f64.powi(2)) / 12.0];
        let i = m.inertia_kg_mm2.unwrap();
        for k in 0..3 {
            assert!(near(i[k][k], want[k], 1e-9), "I{k}{k} {} vs {}", i[k][k], want[k]);
            for j in 0..3 {
                if j != k {
                    assert!(i[k][j].abs() < 1e-9 * want[0], "product of inertia {k}{j}: {}", i[k][j]);
                }
            }
        }
        // 39.68, 79.36, 103.168 kg·mm².
        let pr = m.principal_kg_mm2.unwrap();
        for k in 0..3 {
            assert!(near(pr[k], want[k], 1e-9), "principal {pr:?}");
        }
        // Unit density: 8e6, 16e6, 20.8e6 mm⁵.
        assert!(near(m.inertia_mm5[0][0], 8.0e6, 1e-12) && near(m.inertia_mm5[1][1], 16.0e6, 1e-12) && near(m.inertia_mm5[2][2], 20.8e6, 1e-12));
    }

    #[test]
    fn two_bodies_combine_by_parallel_axes() {
        let (mut p, mut t) = block(10.0, 10.0, 10.0, [0.0; 3]);
        let (p2, t2) = block(10.0, 10.0, 10.0, [20.0, 0.0, 0.0]);
        t.extend(t2.iter().map(|k| k + 8));
        p.extend(p2);
        let m = measure_bodies(&p, &[(&t[..36], Some(1000.0)), (&t[36..], Some(1000.0))]);
        assert!(near(m.volume_mm3, 2000.0, 1e-12));
        assert!(near(m.center_of_mass[0], 15.0, 1e-12));
        // Each cube: m a²/6 about its centre; plus m·10² each about the y and z axes through the middle.
        let mc = 1000.0 * 1e-9 * 1000.0;
        let i = m.inertia_kg_mm2.unwrap();
        assert!(near(i[0][0], 2.0 * mc * 100.0 / 6.0, 1e-9));
        assert!(near(i[1][1], 2.0 * (mc * 100.0 / 6.0 + mc * 100.0), 1e-9));
        // One body without a material: no mass, still a volume and centroid.
        let n = measure_bodies(&p, &[(&t[..36], Some(1000.0)), (&t[36..], None)]);
        assert!(n.mass_kg.is_none() && near(n.centroid[0], 15.0, 1e-12));
    }

    #[test]
    fn placing_and_combining_keep_the_numbers() {
        let (p, t) = block(60.0, 40.0, 20.0, [0.0; 3]);
        let m = measure_mesh(&p, &t, Some(1240.0));
        // Turned 90° about z: Ixx and Iyy swap; moved by (100, 0, 0).
        let r = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let q = placed(&m, r, [100.0, 0.0, 0.0]);
        let (i, j) = (m.inertia_kg_mm2.unwrap(), q.inertia_kg_mm2.unwrap());
        assert!(near(j[0][0], i[1][1], 1e-9) && near(j[1][1], i[0][0], 1e-9));
        assert!(near(q.centroid[0], 100.0 - 20.0, 1e-12) && near(q.centroid[1], 30.0, 1e-12));
        assert!(near(q.bounds_min[0], 60.0, 1e-12) && near(q.bounds_max[1], 60.0, 1e-12));
        let two = combine(&[m.clone(), placed(&m, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], [0.0, 0.0, 20.0])]);
        assert!(near(two.volume_mm3, 96000.0, 1e-12) && near(two.center_of_mass[2], 20.0, 1e-12));
        // A 60×40×40 block: m(40² + 40²)/12 about x.
        let mass = 96000.0 * 1240.0e-9;
        assert!(near(two.inertia_kg_mm2.unwrap()[0][0], mass * 3200.0 / 12.0, 1e-9));
    }

    #[test]
    fn eigen_finds_a_turned_tensor_principal_moments() {
        // diag(1, 2, 3) turned 30° about z.
        let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        let m = [[c * c * 1.0 + s * s * 2.0, c * s * (2.0 - 1.0), 0.0], [c * s * (2.0 - 1.0), s * s * 1.0 + c * c * 2.0, 0.0], [0.0, 0.0, 3.0]];
        let (v, axes) = eigen(m);
        assert!(near(v[0], 1.0, 1e-12) && near(v[1], 2.0, 1e-12) && near(v[2], 3.0, 1e-12), "{v:?}");
        assert!(near(axes[2][2].abs(), 1.0, 1e-12));
    }
}
