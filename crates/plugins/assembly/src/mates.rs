//! Mates: constraints between geometry of two instances (or one and the
//! ground) — coincident, concentric, parallel, perpendicular, angle,
//! distance, tangent and lock — as residuals the joint solver drives to zero
//! alongside the joints. Each mate removes only the freedoms it names (a
//! plane on a plane leaves the part sliding and turning on it), so a part
//! can be held by several, and what still moves is reported (see `dof`).
//!
//! The geometry is points, lines (axes; cylinders carry a radius) and planes
//! in assembly coordinates. Lengths are mm; direction terms are scaled by the
//! assembly's size so a degree weighs like a millimetre at its reach.
//!
//! The constraint set follows FreeCAD's Assembly workbench (OndselSolver,
//! LGPL) in what it offers; the formulation here is our own.

use crate::geometry::{Elem, perp};
use crate::transform::{self as tf, V3};

/// What a mate needs from each side: its rank among the kinds (point < line < plane).
fn rank(e: &Elem) -> u8 {
    match e {
        Elem::Point { .. } => 0,
        Elem::Line { .. } => 1,
        Elem::Plane { .. } => 2,
        Elem::Frame => 3,
    }
}

/// The mates (the joint kinds that constrain geometry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mate {
    Coincident,
    Concentric,
    Parallel,
    Perpendicular,
    Angle,
    Distance,
    Tangent,
}

fn dir(e: &Elem) -> Option<V3> {
    match *e {
        Elem::Line { d, .. } => Some(d),
        Elem::Plane { n, .. } => Some(n),
        _ => None,
    }
}

fn need(kind: Mate, a: &Elem, b: &Elem) -> String {
    let what = match kind {
        Mate::Coincident => "coincident joins points, lines (axes) and planes",
        Mate::Concentric => "concentric needs two axes (round faces, round edges, lines) or two centres",
        Mate::Parallel => "parallel needs two directions: lines, axes or flat faces",
        Mate::Perpendicular => "perpendicular needs two directions: lines, axes or flat faces",
        Mate::Angle => "an angle needs two directions: lines, axes or flat faces",
        Mate::Distance => "a distance is between points, lines and planes",
        Mate::Tangent => "tangent needs a round face (a cylinder or a sphere) against a flat or round one",
    };
    format!("{what}, not a {} and a {}", a.word(), b.word())
}

/// Checks that the mate can join these two kinds of geometry.
pub fn check(kind: Mate, a: &Elem, b: &Elem) -> Result<(), String> {
    let mut out = vec![];
    residuals(kind, a, b, Some(0.0), false, 0.0, 1.0, &mut out)
}

/// Appends how far the mate is from holding. `value` is its angle (°) or
/// distance (mm) when held (None: it follows). `aligned` makes two planes
/// face the same way (default: opposed, faces meeting); `offset` is a gap
/// between coincident planes or a point and a plane (mm).
#[allow(clippy::too_many_arguments)]
pub fn residuals(kind: Mate, a: &Elem, b: &Elem, value: Option<f64>, aligned: bool, offset: f64, size: f64, out: &mut Vec<f64>) -> Result<(), String> {
    // Lower rank first (point, line, plane); planes stay the reference for signed terms.
    let (a, b) = if rank(a) <= rank(b) { (a, b) } else { (b, a) };
    let s = size;
    let fail = || Err(need(kind, a, b));
    let v = value.unwrap_or(0.0);
    let held = value.is_some();
    match kind {
        Mate::Coincident | Mate::Concentric => match (*a, *b) {
            (Elem::Point { p: pa, .. }, Elem::Point { p: pb, .. }) => out.extend(tf::sub(pb, pa)),
            (Elem::Point { p, .. }, Elem::Line { p: lp, d, .. }) => out.extend(perp(tf::sub(p, lp), d)),
            (Elem::Line { p: pa, d: da, .. }, Elem::Line { p: pb, d: db, .. }) => {
                out.extend(tf::scale(tf::cross(da, db), s));
                out.extend(perp(tf::sub(pb, pa), da));
            }
            (Elem::Point { p, .. }, Elem::Plane { p: pp, n }) if kind == Mate::Coincident => out.push(tf::dot(n, tf::sub(p, pp)) - offset),
            (Elem::Line { p, d, .. }, Elem::Plane { p: pp, n }) if kind == Mate::Coincident => {
                out.push(tf::dot(n, d) * s);
                out.push(tf::dot(n, tf::sub(p, pp)) - offset);
            }
            (Elem::Plane { p: pa, n: na }, Elem::Plane { p: pb, n: nb }) if kind == Mate::Coincident => {
                out.extend(tf::scale(if aligned { tf::sub(na, nb) } else { tf::add(na, nb) }, s));
                out.push(tf::dot(na, tf::sub(pb, pa)) - offset);
            }
            _ => return fail(),
        },
        Mate::Parallel => match (dir(a), dir(b)) {
            (Some(da), Some(db)) => {
                if rank(a) != rank(b) {
                    out.push(tf::dot(da, db) * s); // a line along a plane
                } else if aligned {
                    out.extend(tf::scale(tf::sub(da, db), s));
                } else {
                    out.extend(tf::scale(tf::cross(da, db), s));
                }
            }
            _ => return fail(),
        },
        Mate::Perpendicular => match (dir(a), dir(b)) {
            (Some(da), Some(db)) => {
                if rank(a) != rank(b) {
                    out.extend(tf::scale(tf::cross(da, db), s)); // a line along a plane's normal
                } else {
                    out.push(tf::dot(da, db) * s);
                }
            }
            _ => return fail(),
        },
        Mate::Angle => match (dir(a), dir(b)) {
            (Some(da), Some(db)) => {
                if held {
                    let c = tf::dot(da, db);
                    // A line against a plane: the angle to the plane (sin); else between directions (cos).
                    let want = if rank(a) != rank(b) { v.to_radians().sin() } else { v.to_radians().cos() };
                    out.push((c - want) * s);
                }
            }
            _ => return fail(),
        },
        Mate::Distance => match (*a, *b) {
            (Elem::Point { p: pa, .. }, Elem::Point { p: pb, .. }) => {
                if held {
                    out.push(tf::norm(tf::sub(pb, pa)) - v);
                }
            }
            (Elem::Point { p, .. }, Elem::Line { p: lp, d, .. }) => {
                if held {
                    out.push(tf::norm(perp(tf::sub(p, lp), d)) - v);
                }
            }
            (Elem::Point { p, .. }, Elem::Plane { p: pp, n }) => {
                if held {
                    out.push(tf::dot(n, tf::sub(p, pp)) - v);
                }
            }
            (Elem::Line { p: pa, d: da, .. }, Elem::Line { p: pb, d: db, .. }) => {
                out.extend(tf::scale(tf::cross(da, db), s));
                if held {
                    out.push(tf::norm(perp(tf::sub(pb, pa), da)) - v);
                }
            }
            (Elem::Line { p, d, .. }, Elem::Plane { p: pp, n }) => {
                out.push(tf::dot(n, d) * s);
                if held {
                    out.push(tf::dot(n, tf::sub(p, pp)) - v);
                }
            }
            (Elem::Plane { p: pa, n: na }, Elem::Plane { p: pb, n: nb }) => {
                out.extend(tf::scale(if aligned { tf::sub(na, nb) } else { tf::add(na, nb) }, s));
                if held {
                    out.push(tf::dot(na, tf::sub(pb, pa)) - v);
                }
            }
            _ => return fail(),
        },
        Mate::Tangent => match (*a, *b) {
            (Elem::Line { p, d, r }, Elem::Plane { p: pp, n }) if r != 0.0 => {
                out.push(tf::dot(n, d) * s);
                out.push(tf::dot(n, tf::sub(p, pp)) - r.abs());
            }
            (Elem::Point { p, r }, Elem::Plane { p: pp, n }) if r > 0.0 => out.push(tf::dot(n, tf::sub(p, pp)) - r),
            (Elem::Line { p: pa, d: da, r: ra }, Elem::Line { p: pb, r: rb, d: db }) if ra != 0.0 && rb != 0.0 => {
                out.extend(tf::scale(tf::cross(da, db), s));
                out.push(tf::norm(perp(tf::sub(pb, pa), da)) - (ra + rb).abs());
            }
            (Elem::Point { p: pa, r: ra }, Elem::Point { p: pb, r: rb }) if ra > 0.0 && rb > 0.0 => out.push(tf::norm(tf::sub(pb, pa)) - (ra + rb)),
            (Elem::Point { p, r: rp }, Elem::Line { p: lp, d, r: rl }) if rp > 0.0 && rl != 0.0 => out.push(tf::norm(perp(tf::sub(p, lp), d)) - (rl + rp).abs()),
            _ => return fail(),
        },
    }
    Ok(())
}

/// The value an angle (°) or distance (mm) mate has between these.
pub fn measure(kind: Mate, a: &Elem, b: &Elem) -> Option<f64> {
    let (a, b) = if rank(a) <= rank(b) { (a, b) } else { (b, a) };
    match kind {
        Mate::Angle => {
            let (da, db) = (dir(a)?, dir(b)?);
            let c = tf::dot(da, db).clamp(-1.0, 1.0);
            Some(if rank(a) != rank(b) { c.asin().to_degrees() } else { c.acos().to_degrees() })
        }
        Mate::Distance => Some(match (*a, *b) {
            (Elem::Point { p: pa, .. }, Elem::Point { p: pb, .. }) => tf::norm(tf::sub(pb, pa)),
            (Elem::Point { p, .. }, Elem::Line { p: lp, d, .. }) => tf::norm(perp(tf::sub(p, lp), d)),
            (Elem::Point { p, .. }, Elem::Plane { p: pp, n }) => tf::dot(n, tf::sub(p, pp)),
            (Elem::Line { p: pa, d: da, .. }, Elem::Line { p: pb, .. }) => tf::norm(perp(tf::sub(pb, pa), da)),
            (Elem::Line { p, .. }, Elem::Plane { p: pp, n }) => tf::dot(n, tf::sub(p, pp)),
            (Elem::Plane { p: pa, n: na }, Elem::Plane { p: pb, .. }) => tf::dot(na, tf::sub(pb, pa)),
            _ => return None,
        }),
        _ => None,
    }
}

/// The geometry a connector or ground frame stands for in a mate with
/// `other` (None: the other end is a frame too): the same kind as the other
/// end, else a plane (faces meet) for coincident, distance and tangent, an
/// axis for the rest.
pub fn adapt(kind: Mate, frame: &crate::Transform, other: Option<&Elem>) -> Elem {
    let (p, z) = (frame.at, frame.turn([0.0, 0.0, 1.0]));
    match other {
        Some(Elem::Point { .. }) => Elem::Point { p, r: 0.0 },
        Some(Elem::Line { .. }) => Elem::Line { p, d: z, r: 0.0 },
        Some(Elem::Plane { .. }) => Elem::Plane { p, n: z },
        _ => match kind {
            Mate::Coincident | Mate::Distance | Mate::Tangent => Elem::Plane { p, n: z },
            _ => Elem::Line { p, d: z, r: 0.0 },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(p: V3, n: V3) -> Elem {
        Elem::Plane { p, n }
    }

    #[test]
    fn residuals_vanish_when_the_mate_holds() {
        let mut r = vec![];
        // Two faces meeting.
        residuals(Mate::Coincident, &plane([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]), &plane([3.0, 9.0, 5.0], [0.0, 0.0, -1.0]), None, false, 0.0, 10.0, &mut r).unwrap();
        assert!(r.iter().all(|v| v.abs() < 1e-12), "{r:?}");
        // A shaft resting on a floor.
        r.clear();
        residuals(Mate::Tangent, &plane([0.0; 3], [0.0, 0.0, 1.0]), &Elem::Line { p: [4.0, 0.0, 6.0], d: [1.0, 0.0, 0.0], r: 6.0 }, None, false, 0.0, 10.0, &mut r).unwrap();
        assert!(r.iter().all(|v| v.abs() < 1e-12), "{r:?}");
        // A shaft in a bore, touching one side: |r_bore + r_shaft| apart.
        r.clear();
        residuals(Mate::Tangent, &Elem::Line { p: [0.0; 3], d: [0.0, 0.0, 1.0], r: -10.0 }, &Elem::Line { p: [4.0, 0.0, 0.0], d: [0.0, 0.0, 1.0], r: 6.0 }, None, false, 0.0, 10.0, &mut r).unwrap();
        assert!(r.iter().all(|v| v.abs() < 1e-12), "{r:?}");
        // 30° between two faces' normals.
        r.clear();
        let t = 30f64.to_radians();
        residuals(Mate::Angle, &plane([0.0; 3], [0.0, 0.0, 1.0]), &plane([0.0; 3], [t.sin(), 0.0, t.cos()]), Some(30.0), false, 0.0, 10.0, &mut r).unwrap();
        assert!(r[0].abs() < 1e-12);
        assert!((measure(Mate::Angle, &plane([0.0; 3], [0.0, 0.0, 1.0]), &plane([0.0; 3], [t.sin(), 0.0, t.cos()])).unwrap() - 30.0).abs() < 1e-9);
        assert!(check(Mate::Tangent, &plane([0.0; 3], [0.0, 0.0, 1.0]), &plane([0.0; 3], [0.0, 0.0, 1.0])).is_err());
        assert!(check(Mate::Concentric, &plane([0.0; 3], [0.0, 0.0, 1.0]), &Elem::Line { p: [0.0; 3], d: [0.0, 0.0, 1.0], r: 0.0 }).is_err());
    }
}
