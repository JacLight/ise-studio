//! Plane geometry for copper: every pad, track segment and via is a convex
//! polygon swept by a disc (a "rounded convex"): a rectangle has radius 0, a
//! round-rect is a smaller rectangle with the corner radius, a circle is one
//! point, an oval or a track segment is two points. Distances between them
//! are exact, which is what clearance checks need.

use manifold_csg::CrossSection;
use serde::{Deserialize, Serialize};

pub type P = [f64; 2];

pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
pub fn len(a: P) -> f64 {
    a[0].hypot(a[1])
}
pub fn dist(a: P, b: P) -> f64 {
    len(sub(a, b))
}

/// `p` turned `deg` degrees counter-clockwise about the origin.
pub fn rotate(p: P, deg: f64) -> P {
    if deg.rem_euclid(360.0) == 0.0 {
        return p;
    }
    let (s, c) = deg.to_radians().sin_cos();
    // Quarter turns exactly (no 1e-17 noise in coordinates written to files).
    let (s, c) = (snap_unit(s), snap_unit(c));
    [c * p[0] - s * p[1], s * p[0] + c * p[1]]
}

fn snap_unit(v: f64) -> f64 {
    for t in [-1.0, 0.0, 1.0] {
        if (v - t).abs() < 1e-12 {
            return t;
        }
    }
    v
}

/// A convex polygon (1 to n points, any winding) swept by a disc of radius `r`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prim {
    pub pts: Vec<P>,
    pub r: f64,
}

impl Prim {
    pub fn circle(c: P, r: f64) -> Prim {
        Prim { pts: vec![c], r }
    }

    pub fn capsule(a: P, b: P, r: f64) -> Prim {
        if dist(a, b) < 1e-9 { Prim::circle(a, r) } else { Prim { pts: vec![a, b], r } }
    }

    /// A `w` × `h` rectangle centred at `c`, turned `rot` degrees, with corners rounded by `corner`.
    pub fn rect(c: P, w: f64, h: f64, rot: f64, corner: f64) -> Prim {
        let corner = corner.clamp(0.0, w.min(h) / 2.0);
        let (hw, hh) = (w / 2.0 - corner, h / 2.0 - corner);
        if hw <= 1e-9 && hh <= 1e-9 {
            return Prim::circle(c, corner);
        }
        if hw <= 1e-9 || hh <= 1e-9 {
            let (a, b) = if hw > hh { ([-hw, 0.0], [hw, 0.0]) } else { ([0.0, -hh], [0.0, hh]) };
            return Prim::capsule(add(c, rotate(a, rot)), add(c, rotate(b, rot)), corner);
        }
        let pts = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].iter().map(|p| add(c, rotate(*p, rot))).collect();
        Prim { pts, r: corner }
    }

    pub fn bbox(&self) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in &self.pts {
            b[0] = b[0].min(p[0] - self.r);
            b[1] = b[1].min(p[1] - self.r);
            b[2] = b[2].max(p[0] + self.r);
            b[3] = b[3].max(p[1] + self.r);
        }
        b
    }

    pub fn center(&self) -> P {
        let n = self.pts.len() as f64;
        let s = self.pts.iter().fold([0.0, 0.0], |a, p| add(a, *p));
        [s[0] / n, s[1] / n]
    }

    /// The outline as a counter-clockwise polygon, arcs within `tol` mm.
    pub fn outline(&self, tol: f64) -> Vec<P> {
        let hull = convex_hull(&self.pts);
        if self.r <= 1e-9 {
            return hull;
        }
        let n = segments_for(self.r, tol);
        if hull.len() == 1 {
            return (0..n).map(|i| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                [hull[0][0] + self.r * a.cos(), hull[0][1] + self.r * a.sin()]
            }).collect();
        }
        // Each corner gets the arc between its two edge normals.
        let mut out = Vec::new();
        let m = hull.len();
        for i in 0..m {
            let prev = hull[(i + m - 1) % m];
            let p = hull[i];
            let next = hull[(i + 1) % m];
            let n0 = outward(prev, p);
            let n1 = outward(p, next);
            let a0 = n0[1].atan2(n0[0]);
            let mut a1 = n1[1].atan2(n1[0]);
            while a1 < a0 {
                a1 += std::f64::consts::TAU;
            }
            let steps = (((a1 - a0) / std::f64::consts::TAU) * n as f64).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let a = a0 + (a1 - a0) * k as f64 / steps as f64;
                out.push([p[0] + self.r * a.cos(), p[1] + self.r * a.sin()]);
            }
        }
        out
    }

    /// The copper area as a cross-section, grown by `grow` mm.
    pub fn section(&self, grow: f64, tol: f64) -> CrossSection {
        let r = self.r + grow;
        Prim { pts: self.pts.clone(), r: r.max(0.0) }.section_exact(tol)
    }

    fn section_exact(&self, tol: f64) -> CrossSection {
        let poly = self.outline(tol);
        if poly.len() < 3 {
            return CrossSection::empty();
        }
        CrossSection::from_polygons(&[poly])
    }

    pub fn area(&self) -> f64 {
        polygon_area(&self.outline(0.002)).abs()
    }
}

/// Outward normal of edge a→b of a counter-clockwise polygon.
fn outward(a: P, b: P) -> P {
    let d = sub(b, a);
    let l = len(d).max(1e-12);
    [d[1] / l, -d[0] / l]
}

pub fn segments_for(r: f64, tol: f64) -> usize {
    let r = r.abs();
    if r <= tol {
        return 8;
    }
    ((std::f64::consts::PI / (1.0 - tol / r).acos()).ceil() as usize).clamp(8, 256)
}

/// Convex hull, counter-clockwise (monotone chain). One or two points come back as they are.
pub fn convex_hull(points: &[P]) -> Vec<P> {
    let mut pts: Vec<P> = points.to_vec();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup_by(|a, b| dist(*a, *b) < 1e-12);
    if pts.len() < 3 {
        return pts;
    }
    let mut lower: Vec<P> = Vec::new();
    for p in &pts {
        while lower.len() >= 2 && cross(sub(lower[lower.len() - 1], lower[lower.len() - 2]), sub(*p, lower[lower.len() - 2])) <= 0.0 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<P> = Vec::new();
    for p in pts.iter().rev() {
        while upper.len() >= 2 && cross(sub(upper[upper.len() - 1], upper[upper.len() - 2]), sub(*p, upper[upper.len() - 2])) <= 0.0 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    if lower.len() < 3 {
        // All on a line: its two ends.
        return vec![pts[0], pts[pts.len() - 1]];
    }
    lower
}

pub fn polygon_area(poly: &[P]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| cross(poly[i], poly[(i + 1) % n])).sum::<f64>() / 2.0
}

pub fn point_seg_dist(p: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 < 1e-24 {
        return dist(p, a);
    }
    let t = (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0);
    dist(p, add(a, [ab[0] * t, ab[1] * t]))
}

fn segments_cross(a0: P, a1: P, b0: P, b1: P) -> bool {
    let d1 = cross(sub(a1, a0), sub(b0, a0));
    let d2 = cross(sub(a1, a0), sub(b1, a0));
    let d3 = cross(sub(b1, b0), sub(a0, b0));
    let d4 = cross(sub(b1, b0), sub(a1, b0));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

pub fn seg_seg_dist(a0: P, a1: P, b0: P, b1: P) -> f64 {
    if segments_cross(a0, a1, b0, b1) {
        return 0.0;
    }
    point_seg_dist(a0, b0, b1).min(point_seg_dist(a1, b0, b1)).min(point_seg_dist(b0, a0, a1)).min(point_seg_dist(b1, a0, a1))
}

/// Inside (or on) a convex polygon of 3 or more points, either winding.
pub fn in_convex(p: P, poly: &[P]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut sign = 0.0;
    for i in 0..poly.len() {
        let c = cross(sub(poly[(i + 1) % poly.len()], poly[i]), sub(p, poly[i]));
        if c.abs() < 1e-12 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Inside a simple polygon (even-odd).
pub fn in_polygon(p: P, poly: &[P]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

fn edges(pts: &[P]) -> Vec<(P, P)> {
    match pts.len() {
        0 => vec![],
        1 => vec![(pts[0], pts[0])],
        2 => vec![(pts[0], pts[1])],
        n => (0..n).map(|i| (pts[i], pts[(i + 1) % n])).collect(),
    }
}

/// Gap between two rounded convexes: negative or zero when they touch or overlap.
pub fn gap(a: &Prim, b: &Prim) -> f64 {
    let (ha, hb) = (convex_hull(&a.pts), convex_hull(&b.pts));
    if ha.iter().any(|p| in_convex(*p, &hb)) || hb.iter().any(|p| in_convex(*p, &ha)) {
        return -(a.r + b.r) - 1e-6;
    }
    let mut d = f64::INFINITY;
    for (a0, a1) in edges(&ha) {
        for (b0, b1) in edges(&hb) {
            d = d.min(seg_seg_dist(a0, a1, b0, b1));
        }
    }
    d - a.r - b.r
}

/// Gap from a rounded convex to the area of polygons (outer contours
/// counter-clockwise and holes clockwise, as cross-sections give them):
/// negative when it overlaps the area.
pub fn gap_to_area(a: &Prim, contours: &[Vec<P>]) -> f64 {
    let probe = a.center();
    if inside_area(probe, contours) {
        return -a.r.max(1e-6);
    }
    let mut d = f64::INFINITY;
    let hull = convex_hull(&a.pts);
    for c in contours {
        for (b0, b1) in edges(c) {
            for (a0, a1) in edges(&hull) {
                d = d.min(seg_seg_dist(a0, a1, b0, b1));
            }
        }
    }
    d - a.r
}

/// Inside the area of contours (non-zero winding over signed contours).
pub fn inside_area(p: P, contours: &[Vec<P>]) -> bool {
    let mut w = 0i32;
    for c in contours {
        let n = c.len();
        for i in 0..n {
            let (a, b) = (c[i], c[(i + 1) % n]);
            if a[1] <= p[1] {
                if b[1] > p[1] && cross(sub(b, a), sub(p, a)) > 0.0 {
                    w += 1;
                }
            } else if b[1] <= p[1] && cross(sub(b, a), sub(p, a)) < 0.0 {
                w -= 1;
            }
        }
    }
    w != 0
}

/// Distance from a point to a polyline loop (closed).
pub fn dist_to_loop(p: P, poly: &[P]) -> f64 {
    let n = poly.len();
    (0..n).map(|i| point_seg_dist(p, poly[i], poly[(i + 1) % n])).fold(f64::INFINITY, f64::min)
}

/// Gap from a rounded convex to a closed outline's line (a board edge).
pub fn gap_to_loop(a: &Prim, poly: &[P]) -> f64 {
    let hull = convex_hull(&a.pts);
    let mut d = f64::INFINITY;
    let n = poly.len();
    for i in 0..n {
        for (a0, a1) in edges(&hull) {
            d = d.min(seg_seg_dist(a0, a1, poly[i], poly[(i + 1) % n]));
        }
    }
    d - a.r
}

/// Overlap of two convex polygons (separating axis), for courtyards.
pub fn convex_overlap(a: &[P], b: &[P]) -> bool {
    for poly in [a, b] {
        let n = poly.len();
        for i in 0..n {
            let e = sub(poly[(i + 1) % n], poly[i]);
            let axis = [-e[1], e[0]];
            let (mut amin, mut amax) = (f64::INFINITY, f64::NEG_INFINITY);
            for p in a {
                let d = dot(*p, axis);
                amin = amin.min(d);
                amax = amax.max(d);
            }
            let (mut bmin, mut bmax) = (f64::INFINITY, f64::NEG_INFINITY);
            for p in b {
                let d = dot(*p, axis);
                bmin = bmin.min(d);
                bmax = bmax.max(d);
            }
            // Touching edges do not overlap.
            let eps = 1e-9 * len(axis).max(1.0);
            if amax <= bmin + eps || bmax <= amin + eps {
                return false;
            }
        }
    }
    true
}

pub fn bbox_overlap(a: &[f64; 4], b: &[f64; 4], margin: f64) -> bool {
    a[0] - margin <= b[2] && b[0] - margin <= a[2] && a[1] - margin <= b[3] && b[1] - margin <= a[3]
}

/// The contours of a cross-section (outer contours counter-clockwise, holes clockwise).
pub fn contours(cs: &CrossSection) -> Vec<Vec<P>> {
    cs.to_polygons()
}

/// Corners of a polygon rounded to radius `r` (arcs within `tol`).
pub fn round_corners(poly: &[P], r: f64, tol: f64) -> Vec<P> {
    if r <= 0.0 || poly.len() < 3 {
        return poly.to_vec();
    }
    let cs = CrossSection::from_polygons(&[poly.to_vec()]);
    let n = segments_for(r, tol) as i32;
    let rounded = cs.offset(-r, manifold_csg::JoinType::Round, 2.0, n).offset(r, manifold_csg::JoinType::Round, 2.0, n);
    let mut out = rounded.to_polygons();
    out.sort_by(|a, b| polygon_area(b).abs().total_cmp(&polygon_area(a).abs()));
    out.into_iter().next().unwrap_or_else(|| poly.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_are_exact() {
        let a = Prim::rect([0.0, 0.0], 2.0, 1.0, 0.0, 0.0);
        let b = Prim::circle([3.0, 0.0], 0.5);
        assert!((gap(&a, &b) - 1.5).abs() < 1e-12);
        let t = Prim::capsule([0.0, 2.0], [10.0, 2.0], 0.125);
        assert!((gap(&a, &t) - (1.5 - 0.125)).abs() < 1e-12);
        let c = Prim::rect([0.5, 0.0], 1.0, 1.0, 45.0, 0.0);
        assert!(gap(&a, &c) < 0.0);
        // A rounded rect: a corner reads its radius.
        let rr = Prim::rect([0.0, 0.0], 2.0, 2.0, 0.0, 0.5);
        let p = Prim::circle([2.0, 2.0], 0.0);
        let want = (0.5f64.hypot(0.5) + 1.0f64.hypot(1.0)) - 0.5 - 0.5f64.hypot(0.5);
        assert!((gap(&rr, &p) - (2.0f64.hypot(2.0) - 0.5f64.hypot(0.5) - 0.5)).abs() < 1e-9, "{} {want}", gap(&rr, &p));
    }

    #[test]
    fn outlines_close_round() {
        let c = Prim::circle([1.0, 1.0], 1.0).outline(0.001);
        assert!((polygon_area(&c) - std::f64::consts::PI).abs() < 0.01);
        let r = Prim::rect([0.0, 0.0], 2.0, 1.0, 30.0, 0.0).outline(0.01);
        assert!((polygon_area(&r) - 2.0).abs() < 1e-9);
    }
}
