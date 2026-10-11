//! Rigid placements in full 3D: a rotation then a translation (mm).
//!
//! Instances store their rotation as degrees about X, then Y, then Z (fixed
//! axes), the same order as `Solid.rotate`, Manifold and OpenCascade use, so
//! R = Rz · Ry · Rx. Composition and joints work on the matrix.

use serde::{Deserialize, Deserializer};

pub type V3 = [f64; 3];
/// A rotation matrix, rows first.
pub type M3 = [[f64; 3]; 3];

pub const I3: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A rigid placement: a point p goes to `rot · p + at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub at: V3,
    pub rot: M3,
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn mul(m: &M3, v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
pub fn matmul(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}
pub fn transpose(m: &M3) -> M3 {
    [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]]
}

/// Rotation by `deg` degrees about one of the axes (0 = X, 1 = Y, 2 = Z).
pub fn axis_rotation(axis: usize, deg: f64) -> M3 {
    let (s, c) = deg.to_radians().sin_cos();
    // Exact quarter turns, so 90° placements stay exact.
    let (s, c) = (snap(s), snap(c));
    match axis {
        0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

fn snap(v: f64) -> f64 {
    for t in [-1.0, 0.0, 1.0] {
        if (v - t).abs() < 1e-15 {
            return t;
        }
    }
    v
}

/// Rotation by `angle` radians about the unit vector `k`.
pub fn rotation_about(k: V3, angle: f64) -> M3 {
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    [
        [c + k[0] * k[0] * t, k[0] * k[1] * t - k[2] * s, k[0] * k[2] * t + k[1] * s],
        [k[1] * k[0] * t + k[2] * s, c + k[1] * k[1] * t, k[1] * k[2] * t - k[0] * s],
        [k[2] * k[0] * t - k[1] * s, k[2] * k[1] * t + k[0] * s, c + k[2] * k[2] * t],
    ]
}

/// The rotation vector of a rotation (axis × angle in radians).
pub fn log(r: &M3) -> V3 {
    let c = ((r[0][0] + r[1][1] + r[2][2] - 1.0) / 2.0).clamp(-1.0, 1.0);
    let angle = c.acos();
    let v = [r[2][1] - r[1][2], r[0][2] - r[2][0], r[1][0] - r[0][1]];
    if angle < 1e-9 {
        return scale(v, 0.5);
    }
    if std::f64::consts::PI - angle < 1e-6 {
        // Half a turn: the axis is the column of R + I with the largest length.
        let m = [[r[0][0] + 1.0, r[0][1], r[0][2]], [r[1][0], r[1][1] + 1.0, r[1][2]], [r[2][0], r[2][1], r[2][2] + 1.0]];
        let cols = [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]];
        let best = cols.iter().copied().max_by(|a, b| norm(*a).total_cmp(&norm(*b))).unwrap();
        let k = scale(best, 1.0 / norm(best));
        // Keep the sign that agrees with the small antisymmetric part, if any.
        let k = if dot(k, v) < 0.0 { scale(k, -1.0) } else { k };
        return scale(k, angle);
    }
    scale(v, angle / (2.0 * angle.sin()))
}

/// Accepts the rotation as stored now ([x, y, z] degrees) or as projects
/// stored it before full 3D placement (one number: degrees about Z).
pub fn rotation_compat<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; 3], D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Turn(f64),
        Xyz([f64; 3]),
    }
    Ok(match Either::deserialize(d)? {
        Either::Turn(z) => [0.0, 0.0, z],
        Either::Xyz(r) => r,
    })
}

impl Transform {
    pub const IDENTITY: Transform = Transform { at: [0.0; 3], rot: I3 };

    /// A position and a rotation in degrees about X, then Y, then Z.
    pub fn from_euler(at: V3, deg: V3) -> Transform {
        let r = matmul(&axis_rotation(2, deg[2]), &matmul(&axis_rotation(1, deg[1]), &axis_rotation(0, deg[0])));
        Transform { at, rot: r }
    }

    /// A frame: origin `at`, its z along `z`, its x along `x` (both unit, square).
    pub fn frame(at: V3, x: V3, z: V3) -> Transform {
        let y = cross(z, x);
        Transform { at, rot: [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]] }
    }

    pub fn translation(at: V3) -> Transform {
        Transform { at, rot: I3 }
    }

    pub fn rotation(rot: M3) -> Transform {
        Transform { at: [0.0; 3], rot }
    }

    /// The rotation as degrees about X, then Y, then Z (Y within ±90°).
    pub fn euler(&self) -> V3 {
        let r = &self.rot;
        let sy = (-r[2][0]).clamp(-1.0, 1.0);
        let (x, y, z) = if sy.abs() < 1.0 - 1e-12 {
            (r[2][1].atan2(r[2][2]), sy.asin(), r[1][0].atan2(r[0][0]))
        } else {
            // Looking straight along Y: only X − Z (or X + Z) is defined; keep X at 0.
            (0.0, sy.signum() * std::f64::consts::FRAC_PI_2, (-r[0][1]).atan2(r[1][1]))
        };
        let clean = |a: f64| {
            let d = a.to_degrees();
            let d = if d.abs() < 1e-10 { 0.0 } else { d };
            (d * 1e9).round() / 1e9
        };
        [clean(x), clean(y), clean(z)]
    }

    /// The rotation as a unit quaternion [x, y, z, w].
    pub fn quaternion(&self) -> [f64; 4] {
        let m = &self.rot;
        let tr = m[0][0] + m[1][1] + m[2][2];
        let q = if tr > 0.0 {
            let s = (tr + 1.0).sqrt() * 2.0;
            [(m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s, 0.25 * s]
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            [0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s, (m[2][1] - m[1][2]) / s]
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            [(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[0][2] - m[2][0]) / s]
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            [(m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[1][0] - m[0][1]) / s]
        };
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        q.map(|c| c / n)
    }

    pub fn apply(&self, p: V3) -> V3 {
        add(mul(&self.rot, p), self.at)
    }

    /// A direction (rotation only).
    pub fn turn(&self, v: V3) -> V3 {
        mul(&self.rot, v)
    }

    /// `self` after `inner`: a point goes through `inner`, then `self`.
    pub fn then(&self, inner: &Transform) -> Transform {
        Transform { at: self.apply(inner.at), rot: matmul(&self.rot, &inner.rot) }
    }

    /// `self` placed inside `outer`.
    pub fn within(&self, outer: &Transform) -> Transform {
        outer.then(self)
    }

    pub fn inverse(&self) -> Transform {
        let rt = transpose(&self.rot);
        Transform { at: scale(mul(&rt, self.at), -1.0), rot: rt }
    }

    pub fn has_rotation(&self) -> bool {
        (0..3).any(|i| (0..3).any(|j| (self.rot[i][j] - I3[i][j]).abs() > 1e-12))
    }

    /// As Manifold's column-major 3 × 4 matrix.
    pub fn matrix12(&self) -> [f64; 12] {
        let r = &self.rot;
        [r[0][0], r[1][0], r[2][0], r[0][1], r[1][1], r[2][1], r[0][2], r[1][2], r[2][2], self.at[0], self.at[1], self.at[2]]
    }

    pub fn solid(&self, m: &manifold_csg::Manifold) -> manifold_csg::Manifold {
        if self.has_rotation() { m.transform(&self.matrix12()) } else { m.translate(self.at[0], self.at[1], self.at[2]) }
    }

    /// Bounds of a box after this transform.
    pub fn bounds(&self, lo: V3, hi: V3) -> (V3, V3) {
        let mut a = [f64::INFINITY; 3];
        let mut b = [f64::NEG_INFINITY; 3];
        for x in [lo[0], hi[0]] {
            for y in [lo[1], hi[1]] {
                for z in [lo[2], hi[2]] {
                    let p = self.apply([x, y, z]);
                    for k in 0..3 {
                        a[k] = a[k].min(p[k]);
                        b[k] = b[k].max(p[k]);
                    }
                }
            }
        }
        (a, b)
    }

    // The words comments use: a part's own coordinates and the project's.

    /// A point in the part's coordinates, in project coordinates.
    pub fn to_world(&self, p: V3) -> V3 {
        self.apply(p)
    }

    /// A project point in the part's coordinates.
    pub fn to_local(&self, p: V3) -> V3 {
        mul(&transpose(&self.rot), sub(p, self.at))
    }

    pub fn dir_to_world(&self, d: V3) -> V3 {
        self.turn(d)
    }

    pub fn dir_to_local(&self, d: V3) -> V3 {
        mul(&transpose(&self.rot), d)
    }

    /// How far apart two placements are: the largest move of a point within
    /// `size` of the origin (mm).
    pub fn distance(&self, other: &Transform, size: f64) -> f64 {
        let d = norm(sub(self.at, other.at));
        let r = norm(log(&matmul(&transpose(&self.rot), &other.rot)));
        d + r * size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9)
    }

    #[test]
    fn euler_order_is_x_then_y_then_z() {
        // 90° about X takes +Y to +Z; then 90° about Z takes +Z to +Z and +X to +Y.
        let t = Transform::from_euler([0.0; 3], [90.0, 0.0, 90.0]);
        assert!(close(t.apply([0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]));
        assert!(close(t.apply([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        for deg in [[10.0, 20.0, 30.0], [-45.0, 60.0, 170.0], [0.0, 0.0, -90.0], [33.0, -12.5, 0.0]] {
            let back = Transform::from_euler([0.0; 3], deg).euler();
            assert!(close(back, deg), "{deg:?} → {back:?}");
        }
    }

    #[test]
    fn composition_and_inverse() {
        let outer = Transform::from_euler([100.0, 0.0, 0.0], [0.0, 0.0, 90.0]);
        let inner = Transform::from_euler([10.0, 0.0, 0.0], [90.0, 0.0, 0.0]);
        let both = inner.within(&outer);
        let p = [1.0, 2.0, 3.0];
        assert!(close(both.apply(p), outer.apply(inner.apply(p))));
        // inner: (1,2,3) → X-turn (1,−3,2) → +10 x → (11,−3,2); outer: Z-turn (3,11,2) → +100 x → (103, 11, 2).
        assert!(close(both.apply(p), [103.0, 11.0, 2.0]));
        assert!(close(both.inverse().apply(both.apply(p)), p));
        let q = both.quaternion();
        assert!(((q.iter().map(|c| c * c).sum::<f64>()) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn old_rotation_reads_as_a_turn_about_z() {
        #[derive(Deserialize)]
        struct R {
            #[serde(deserialize_with = "rotation_compat")]
            rotation: [f64; 3],
        }
        let r: R = serde_json::from_str(r#"{"rotation": 90}"#).unwrap();
        assert_eq!(r.rotation, [0.0, 0.0, 90.0]);
        let r: R = serde_json::from_str(r#"{"rotation": [0, 90, 0]}"#).unwrap();
        assert_eq!(r.rotation, [0.0, 90.0, 0.0]);
    }

    #[test]
    fn log_of_rotations() {
        let r = rotation_about([0.0, 0.0, 1.0], 0.5);
        assert!(close(log(&r), [0.0, 0.0, 0.5]));
        let r = rotation_about([1.0, 0.0, 0.0], std::f64::consts::PI);
        assert!((norm(log(&r)) - std::f64::consts::PI).abs() < 1e-9);
    }
}
