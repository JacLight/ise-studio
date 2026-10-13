//! Geometry picked on an instance for a mate: a face, an edge, an axis, a
//! point — not only the connectors its program declares.
//!
//! A pick is kept the way it was made (the point clicked on a face and the
//! face's normal there, or points along an edge) and found again on the part
//! as it is now each time the assembly solves, so a mate follows its part
//! when the part changes: a flat face gives its plane, a round face its axis
//! and radius (a cylinder: shaft or bore) or its centre (a sphere), a straight
//! edge its line, a round edge its circle's axis. Explicit points, lines,
//! planes, cylinders and spheres in the instance's own coordinates (what the
//! AI computes) are taken as given.

use crate::transform::{self as tf, Transform, V3};
use crate::{Instance, placement};
use de_core::{IdIndex, StableId, bevy_ecs::world::World, schemars};
use de_shape::{ShapeProgram, ShapeSolid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};

/// Geometry on an instance, in the instance's own coordinates (mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Geometry {
    /// A face picked in the view: the point clicked on it and the face's
    /// outward normal there. Found on the part each solve: flat → its plane,
    /// round → a cylinder's axis and radius or a sphere's centre.
    Face { at: V3, normal: V3 },
    /// An edge picked in the view: points along it. Straight → its line;
    /// round → its circle's axis (through the centre, square to the circle).
    Edge { points: Vec<V3> },
    /// A point (a vertex, a hole's centre).
    Point { at: V3 },
    /// A straight line or axis through `at` along `dir`.
    Line { at: V3, dir: V3 },
    /// A plane through `at` facing `normal` (outward from the part).
    Plane { at: V3, normal: V3 },
    /// A cylinder: its axis and radius (negative for a bore: the material is outside).
    Cylinder { at: V3, dir: V3, radius: f64 },
    /// A sphere: centre and radius.
    Sphere { at: V3, radius: f64 },
    /// The instance's own origin and axes (for `lock`).
    Origin,
}

/// Geometry resolved to what constraints read, in the instance's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Elem {
    /// A point; `r` > 0 for a sphere's centre.
    Point { p: V3, r: f64 },
    /// A line along unit `d`; `r` ≠ 0 for a cylinder's axis (negative: a bore).
    Line { p: V3, d: V3, r: f64 },
    /// A plane with unit outward normal `n`.
    Plane { p: V3, n: V3 },
    /// The instance's frame.
    Frame,
}

impl Elem {
    pub fn word(&self) -> &'static str {
        match self {
            Elem::Point { r, .. } if *r > 0.0 => "sphere",
            Elem::Point { .. } => "point",
            Elem::Line { r, .. } if *r != 0.0 => "cylinder",
            Elem::Line { .. } => "line",
            Elem::Plane { .. } => "plane",
            Elem::Frame => "origin",
        }
    }

    /// Placed by `t` (an instance's placement).
    pub fn placed(&self, t: &Transform) -> Elem {
        match *self {
            Elem::Point { p, r } => Elem::Point { p: t.apply(p), r },
            Elem::Line { p, d, r } => Elem::Line { p: t.apply(p), d: t.turn(d), r },
            Elem::Plane { p, n } => Elem::Plane { p: t.apply(p), n: t.turn(n) },
            Elem::Frame => Elem::Frame,
        }
    }

    /// A frame for a joint that turns or slides about it: origin on it, z
    /// along its axis or normal (x square to that, nearest +X).
    pub fn frame(&self) -> Transform {
        let (p, z) = match *self {
            Elem::Point { p, .. } => (p, [0.0, 0.0, 1.0]),
            Elem::Line { p, d, .. } => (p, d),
            Elem::Plane { p, n } => (p, n),
            Elem::Frame => return Transform::IDENTITY,
        };
        Transform::frame(p, square_x(z), z)
    }
}

/// A unit x square to unit z, nearest +X (or +Y when z is along X).
pub fn square_x(z: V3) -> V3 {
    let r = if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    unit(tf::sub(r, tf::scale(z, tf::dot(r, z))))
}

pub fn unit(v: V3) -> V3 {
    tf::scale(v, 1.0 / tf::norm(v).max(1e-300))
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// The instance's geometry `g`, as it is now.
pub fn resolve(world: &World, instance: StableId, g: &Geometry) -> Result<Elem, String> {
    let bad_dir = |v: V3| !finite(&v) || tf::norm(v) < 1e-12;
    match g {
        Geometry::Point { at } if finite(at) => Ok(Elem::Point { p: *at, r: 0.0 }),
        Geometry::Line { at, dir } if finite(at) && !bad_dir(*dir) => Ok(Elem::Line { p: *at, d: unit(*dir), r: 0.0 }),
        Geometry::Plane { at, normal } if finite(at) && !bad_dir(*normal) => Ok(Elem::Plane { p: *at, n: unit(*normal) }),
        Geometry::Cylinder { at, dir, radius } if finite(at) && !bad_dir(*dir) && radius.is_finite() && *radius != 0.0 => Ok(Elem::Line { p: *at, d: unit(*dir), r: *radius }),
        Geometry::Sphere { at, radius } if finite(at) && radius.is_finite() && *radius > 0.0 => Ok(Elem::Point { p: *at, r: *radius }),
        Geometry::Origin => Ok(Elem::Frame),
        Geometry::Edge { points } => edge(points),
        Geometry::Face { at, normal } if finite(at) && !bad_dir(*normal) => face(world, instance, *at, unit(*normal)),
        _ => Err("the geometry needs finite numbers (and a direction that is not zero, a radius that is not zero)".into()),
    }
}

// ---- Edges ---------------------------------------------------------------------------

/// A straight edge's line, or a round edge's circle (its axis, through the centre).
pub fn edge(points: &[V3]) -> Result<Elem, String> {
    if points.len() < 2 || !points.iter().all(|p| finite(p)) {
        return Err("an edge needs two or more points along it".into());
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    // The two points farthest apart give the line to test against.
    let (mut i0, mut i1, mut far) = (0, points.len() - 1, tf::norm(tf::sub(b, a)));
    for i in 0..points.len() {
        for j in i + 1..points.len() {
            let d = tf::norm(tf::sub(points[j], points[i]));
            if d > far {
                (i0, i1, far) = (i, j, d);
            }
        }
    }
    if far < 1e-9 {
        return Err("the edge's points are all in one place".into());
    }
    let d = unit(tf::sub(points[i1], points[i0]));
    let off = points.iter().map(|p| tf::norm(perp(tf::sub(*p, points[i0]), d))).fold(0.0, f64::max);
    if off <= 1e-4 * far.max(1.0) || points.len() < 3 {
        return Ok(Elem::Line { p: points[i0], d, r: 0.0 });
    }
    // A circle (or an arc): its plane by Newell's method, then its centre.
    let c0 = centroid(points);
    let mut n = [0.0; 3];
    for i in 0..points.len() {
        let (p, q) = (tf::sub(points[i], c0), tf::sub(points[(i + 1) % points.len()], c0));
        n = tf::add(n, tf::cross(p, q));
    }
    if tf::norm(n) < 1e-12 {
        return Err("the edge is neither straight nor round".into());
    }
    let n = unit(n);
    let (u, v) = (square_x(n), tf::cross(n, square_x(n)));
    let flat: Vec<[f64; 2]> = points.iter().map(|p| {
        let q = tf::sub(*p, c0);
        [tf::dot(q, u), tf::dot(q, v)]
    }).collect();
    let (cx, cy, r) = circle_fit(&flat).ok_or("the edge is neither straight nor round")?;
    let worst = flat.iter().map(|p| ((p[0] - cx).hypot(p[1] - cy) - r).abs()).fold(0.0, f64::max);
    if worst > 1e-2 * r.max(1.0) {
        return Err("the edge is neither straight nor a circle: give a point or a line instead".into());
    }
    let centre = tf::add(c0, tf::add(tf::scale(u, cx), tf::scale(v, cy)));
    Ok(Elem::Line { p: centre, d: n, r })
}

fn centroid(points: &[V3]) -> V3 {
    let s = points.iter().fold([0.0; 3], |a, p| tf::add(a, *p));
    tf::scale(s, 1.0 / points.len() as f64)
}

/// The part of `v` square to unit `d`.
pub fn perp(v: V3, d: V3) -> V3 {
    tf::sub(v, tf::scale(d, tf::dot(v, d)))
}

/// Least-squares circle through 2D points (Kåsa): centre and radius.
pub fn circle_fit(p: &[[f64; 2]]) -> Option<(f64, f64, f64)> {
    // x² + y² = 2a x + 2b y + c
    let mut m = vec![vec![0.0; 3]; 3];
    let mut rhs = vec![0.0; 3];
    for q in p {
        let row = [2.0 * q[0], 2.0 * q[1], 1.0];
        let z = q[0] * q[0] + q[1] * q[1];
        for i in 0..3 {
            rhs[i] += row[i] * z;
            for j in 0..3 {
                m[i][j] += row[i] * row[j];
            }
        }
    }
    let s = de_shape::mechanism::solve_dense(m, rhs)?;
    let r2 = s[2] + s[0] * s[0] + s[1] * s[1];
    (r2 > 0.0).then(|| (s[0], s[1], r2.sqrt()))
}

/// Least-squares sphere through 3D points: centre and radius.
pub fn sphere_fit(p: &[V3]) -> Option<(V3, f64)> {
    let mut m = vec![vec![0.0; 4]; 4];
    let mut rhs = vec![0.0; 4];
    for q in p {
        let row = [2.0 * q[0], 2.0 * q[1], 2.0 * q[2], 1.0];
        let z = tf::dot(*q, *q);
        for i in 0..4 {
            rhs[i] += row[i] * z;
            for j in 0..4 {
                m[i][j] += row[i] * row[j];
            }
        }
    }
    let s = de_shape::mechanism::solve_dense(m, rhs)?;
    let c = [s[0], s[1], s[2]];
    let r2 = s[3] + tf::dot(c, c);
    (r2 > 0.0).then(|| (c, r2.sqrt()))
}

/// Eigenvalues (ascending) and unit eigenvectors of a symmetric 3×3 matrix (Jacobi).
pub fn eigen3(m: [[f64; 3]; 3]) -> ([f64; 3], [V3; 3]) {
    let mut a = m;
    let mut v = tf::I3;
    for _ in 0..50 {
        let off = a[0][1].powi(2) + a[0][2].powi(2) + a[1][2].powi(2);
        if off < 1e-30 {
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
            let mut j = tf::I3;
            j[p][p] = c;
            j[q][q] = c;
            j[p][q] = s;
            j[q][p] = -s;
            a = tf::matmul(&tf::transpose(&j), &tf::matmul(&a, &j));
            v = tf::matmul(&v, &j);
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|x, y| a[*x][*x].total_cmp(&a[*y][*y]));
    let vals = order.map(|i| a[i][i]);
    let vecs = order.map(|i| [v[0][i], v[1][i], v[2][i]]);
    (vals, vecs)
}

// ---- Faces ------------------------------------------------------------------------------

/// A face found on a part from triangles: flat → its plane; round → its axis
/// and radius (from the vertices, which lie on the true surface) or its
/// sphere. `at` is where it was picked, `n` its normal there.
pub fn surface(tris: &[[[f64; 3]; 3]], at: V3, n: V3, flat: bool) -> Result<Elem, String> {
    if flat {
        // The plane through the face (its triangles' vertices), facing n.
        let off = tris.iter().flat_map(|t| t.iter()).map(|p| tf::dot(*p, n)).sum::<f64>() / (3 * tris.len().max(1)) as f64;
        let p = tf::add(at, tf::scale(n, off - tf::dot(at, n)));
        return Ok(Elem::Plane { p, n });
    }
    let mut m = [[0.0; 3]; 3];
    let mut verts: Vec<V3> = vec![];
    let mut normals: Vec<(V3, V3)> = vec![];
    for t in tris {
        let c = tf::cross(tf::sub(t[1], t[0]), tf::sub(t[2], t[0]));
        let area = tf::norm(c) / 2.0;
        if area <= 0.0 {
            continue;
        }
        let nn = unit(c);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += area * nn[i] * nn[j];
            }
        }
        normals.push((centroid(t), nn));
        verts.extend(t.iter().copied());
    }
    verts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    verts.dedup_by(|a, b| tf::norm(tf::sub(*a, *b)) < 1e-9);
    if verts.len() < 4 {
        return Err("too small a face to tell its shape: pick a bigger one, or give the geometry".into());
    }
    let (vals, vecs) = eigen3(m);
    let trace = vals.iter().sum::<f64>().max(1e-300);
    if vals[0] < 0.02 * trace && vals[1] > 0.15 * trace {
        // A cylinder: every normal square to the axis.
        let d = vecs[0];
        let (u, v) = (square_x(d), tf::cross(d, square_x(d)));
        let flat: Vec<[f64; 2]> = verts.iter().map(|p| [tf::dot(*p, u), tf::dot(*p, v)]).collect();
        let (cx, cy, r) = circle_fit(&flat).ok_or("could not fit a cylinder to the face")?;
        let axis_at = tf::add(tf::scale(u, cx), tf::scale(v, cy));
        // Put the axis point level with the pick.
        let p = tf::add(axis_at, tf::scale(d, tf::dot(at, d)));
        let worst = flat.iter().map(|q| ((q[0] - cx).hypot(q[1] - cy) - r).abs()).fold(0.0, f64::max);
        if worst > 0.02 * r.max(0.5) {
            return Err("the round face is not a cylinder (a cone or a fillet?): pick a flat face, a round edge, or give an axis".into());
        }
        // Outward normals pointing away from the axis: a shaft; towards it: a bore.
        let away = normals.iter().map(|(c, nn)| tf::dot(perp(tf::sub(*c, p), d), *nn)).sum::<f64>();
        let _ = n;
        return Ok(Elem::Line { p, d, r: if away >= 0.0 { r } else { -r } });
    }
    if vals[0] > 0.15 * trace {
        let (c, r) = sphere_fit(&verts).ok_or("could not fit a sphere to the face")?;
        let worst = verts.iter().map(|q| (tf::norm(tf::sub(*q, c)) - r).abs()).fold(0.0, f64::max);
        if worst > 0.02 * r.max(0.5) {
            return Err("the round face is neither a cylinder nor a sphere: pick a flat face, a round edge, or give an axis".into());
        }
        return Ok(Elem::Point { p: c, r });
    }
    Err("the round face is neither a cylinder nor a sphere (a cone, a torus, a fillet?): pick a flat face, a round edge, or give an axis or point".into())
}

/// Resolved faces by what they were found on (the part's mesh) and the pick.
fn cache() -> &'static Mutex<Vec<(u64, Result<Elem, String>)>> {
    static CACHE: OnceLock<Mutex<Vec<(u64, Result<Elem, String>)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn entity(world: &World, id: StableId) -> Option<de_core::bevy_ecs::entity::Entity> {
    world.resource::<IdIndex>().get(id)
}

/// The face of `instance` under `at` facing `n` (instance coordinates).
fn face(world: &World, instance: StableId, at: V3, n: V3) -> Result<Elem, String> {
    use std::hash::{Hash, Hasher};
    let e = entity(world, instance).ok_or("the instance is gone")?;
    world.get::<Instance>(e).ok_or("not an instance")?;
    let own = placement(world, e).unwrap_or(Transform::IDENTITY).inverse();
    // Each leaf shape of the instance: its mesh identity and where it sits in the instance.
    let mut leaves = vec![];
    for o in crate::occurrences(world, instance) {
        let se = entity(world, o.shape).ok_or("a component is gone")?;
        let program = world.get::<ShapeProgram>(se).ok_or("not a component")?;
        let key = match o.varies() {
            true => {
                let p = crate::occurrence_program(world, &o)?;
                let ev = de_shape::evaluated_at(&p, p.display_tolerance())?;
                (Arc::as_ptr(&ev) as usize, Some(ev))
            }
            false => {
                let solid = world.get::<ShapeSolid>(se).ok_or_else(|| format!("{} has no valid result", program.name))?;
                (Arc::as_ptr(&solid.solid) as usize, None)
            }
        };
        leaves.push((se, key, own.then(&o.transform)));
    }
    if leaves.is_empty() {
        return Err("the instance has no shape to pick a face on".into());
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (_, (ptr, _), t) in &leaves {
        ptr.hash(&mut h);
        for v in t.at.iter().chain(t.rot.iter().flatten()) {
            v.to_bits().hash(&mut h);
        }
    }
    for v in at.iter().chain(n.iter()) {
        v.to_bits().hash(&mut h);
    }
    let key = h.finish();
    if let Some((_, r)) = cache().lock().unwrap().iter().find(|(k, _)| *k == key) {
        return r.clone();
    }
    let mut best: Option<(f64, Result<Elem, String>)> = None;
    for (se, (_, ev), t) in leaves {
        let ev = match ev {
            Some(ev) => ev,
            None => {
                let program = world.get::<ShapeProgram>(se).ok_or("not a component")?;
                de_shape::evaluated_at(program, program.display_tolerance())?
            }
        };
        let (pl, nl) = (t.to_local(at), t.dir_to_local(n));
        // The flat face facing that way under the pick, or the round one: whichever the pick is on.
        let dist = |tris: &[[[f64; 3]; 3]]| tris.iter().map(|tri| tf::norm(tf::sub(nearest_on_triangle(pl, tri[0], tri[1], tri[2]), pl))).fold(f64::INFINITY, f64::min);
        let mut found = vec![];
        if let Some(tris) = de_shape::flat_face(&ev, None, pl, nl).filter(|t| !t.is_empty()) {
            found.push((dist(&tris), tris, true));
        }
        if found.first().is_none_or(|(d, _, _)| *d > 1e-3)
            && let Some((tris, _, _)) = de_shape::smooth_face(&ev, None, pl, nl)
        {
            found.push((dist(&tris), tris, false));
        }
        let Some((d, tris, flat)) = found.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)) else { continue };
        if best.as_ref().is_some_and(|(b, _)| *b <= d) {
            continue;
        }
        let r = surface(&tris, pl, nl, flat).map(|el| el.placed(&t));
        best = Some((d, r));
    }
    let r = match best {
        Some((d, r)) if d < 1.0 => r,
        Some((d, _)) => Err(format!("no face of the part is under the pick (the nearest is {d:.2} mm away): pick it again")),
        None => Err("no face of the part faces that way under the pick: pick it again".into()),
    };
    let mut c = cache().lock().unwrap();
    c.push((key, r.clone()));
    if c.len() > 256 {
        c.drain(..1);
    }
    r
}

/// The point of triangle abc nearest p (Ericson, Real-Time Collision Detection 5.1.5).
pub fn nearest_on_triangle(p: V3, a: V3, b: V3, c: V3) -> V3 {
    let (ab, ac, ap) = (tf::sub(b, a), tf::sub(c, a), tf::sub(p, a));
    let (d1, d2) = (tf::dot(ab, ap), tf::dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = tf::sub(p, b);
    let (d3, d4) = (tf::dot(ab, bp), tf::dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return tf::add(a, tf::scale(ab, d1 / (d1 - d3)));
    }
    let cp = tf::sub(p, c);
    let (d5, d6) = (tf::dot(ab, cp), tf::dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return tf::add(a, tf::scale(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return tf::add(b, tf::scale(tf::sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6))));
    }
    let denom = 1.0 / (va + vb + vc);
    tf::add(a, tf::add(tf::scale(ab, vb * denom), tf::scale(ac, vc * denom)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_give_lines_and_circles() {
        let line = edge(&[[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [10.0, 0.0, 0.0]]).unwrap();
        assert!(matches!(line, Elem::Line { d, r, .. } if (d[0] - 1.0).abs() < 1e-12 && r == 0.0));
        let pts: Vec<V3> = (0..24).map(|i| {
            let a = i as f64 / 24.0 * std::f64::consts::TAU;
            [3.0 + 4.0 * a.cos(), -2.0, 1.0 + 4.0 * a.sin()]
        }).collect();
        match edge(&pts).unwrap() {
            Elem::Line { p, d, r } => {
                assert!((r - 4.0).abs() < 1e-9, "{r}");
                assert!(tf::norm(tf::sub(p, [3.0, -2.0, 1.0])) < 1e-9, "{p:?}");
                assert!((d[1].abs() - 1.0).abs() < 1e-9);
            }
            e => panic!("{e:?}"),
        }
    }

    fn cylinder_tris(r: f64, h: f64, inward: bool) -> Vec<[[f64; 3]; 3]> {
        let n = 32;
        let mut out = vec![];
        for i in 0..n {
            let (a0, a1) = (i as f64 / n as f64 * std::f64::consts::TAU, (i + 1) as f64 / n as f64 * std::f64::consts::TAU);
            let p = |a: f64, z: f64| [10.0 + r * a.cos(), 5.0 + r * a.sin(), z];
            let (q0, q1, q2, q3) = (p(a0, 0.0), p(a1, 0.0), p(a1, h), p(a0, h));
            if inward {
                out.push([q0, q2, q1]);
                out.push([q0, q3, q2]);
            } else {
                out.push([q0, q1, q2]);
                out.push([q0, q2, q3]);
            }
        }
        out
    }

    #[test]
    fn round_faces_give_axes_exactly() {
        let shaft = surface(&cylinder_tris(6.0, 20.0, false), [16.0, 5.0, 7.0], [1.0, 0.0, 0.0], false).unwrap();
        match shaft {
            Elem::Line { p, d, r } => {
                assert!((r - 6.0).abs() < 1e-9, "{r}");
                assert!((p[0] - 10.0).abs() < 1e-9 && (p[1] - 5.0).abs() < 1e-9 && (p[2] - 7.0).abs() < 1e-9, "{p:?}");
                assert!((d[2].abs() - 1.0).abs() < 1e-9);
            }
            e => panic!("{e:?}"),
        }
        let bore = surface(&cylinder_tris(6.0, 20.0, true), [16.0, 5.0, 7.0], [-1.0, 0.0, 0.0], false).unwrap();
        assert!(matches!(bore, Elem::Line { r, .. } if (r + 6.0).abs() < 1e-9));
    }
}
