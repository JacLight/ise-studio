//! Building sketch geometry from a 2D file: lines, arcs, circles and Béziers
//! kept as such, ends that meet sharing one point (so outlines close into
//! profiles), and nothing constrained (the person adds what they mean).

use crate::sketch::{Curve, Geometry, Point};
use std::collections::HashMap;

/// Ends closer than this (mm) are one point.
const MERGE: f64 = 1e-4;

/// The most curves a 2D import makes (a sketch of more is no longer editable).
pub const MAX_CURVES: usize = 20_000;

#[derive(Default)]
pub struct Builder {
    pub g: Geometry,
    next: u32,
    /// Ends by grid cell (MERGE wide), for finding the one a new end meets.
    at: HashMap<(i64, i64), (u32, [f64; 2])>,
    pub counts: Counts,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Counts {
    pub lines: usize,
    pub arcs: usize,
    pub circles: usize,
    pub curves: usize,
}

impl Builder {
    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    fn full(&self) -> Result<(), String> {
        if self.g.curves.len() >= MAX_CURVES {
            return Err(format!("the drawing has more than {MAX_CURVES} curves: too many to edit as a sketch (simplify it, or split it into several files)"));
        }
        Ok(())
    }

    /// A point, shared with any already within `MERGE`.
    pub fn point(&mut self, p: [f64; 2]) -> u32 {
        let cell = |v: f64| (v / MERGE).round() as i64;
        let (cx, cy) = (cell(p[0]), cell(p[1]));
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(&(id, q)) = self.at.get(&(cx + dx, cy + dy)) {
                    if (q[0] - p[0]).hypot(q[1] - p[1]) <= MERGE {
                        return id;
                    }
                }
            }
        }
        let id = self.id();
        self.g.points.push(Point { id, x: p[0], y: p[1] });
        self.at.insert((cx, cy), (id, p));
        id
    }

    /// A point of its own (a centre or a handle: never merged with an end).
    fn free_point(&mut self, p: [f64; 2]) -> u32 {
        let id = self.id();
        self.g.points.push(Point { id, x: p[0], y: p[1] });
        id
    }

    pub fn line(&mut self, a: [f64; 2], b: [f64; 2]) -> Result<(), String> {
        if (a[0] - b[0]).hypot(a[1] - b[1]) <= MERGE {
            return Ok(());
        }
        self.full()?;
        let (a, b) = (self.point(a), self.point(b));
        let id = self.id();
        self.g.curves.push(Curve::Line { id, a, b, construction: false });
        self.counts.lines += 1;
        Ok(())
    }

    /// A circular arc counter-clockwise from `start` to `end` about `center`.
    pub fn arc(&mut self, center: [f64; 2], start: [f64; 2], end: [f64; 2]) -> Result<(), String> {
        if (start[0] - end[0]).hypot(start[1] - end[1]) <= MERGE {
            // A closed arc is a circle.
            return self.circle(center, (start[0] - center[0]).hypot(start[1] - center[1]));
        }
        self.full()?;
        let (s, e) = (self.point(start), self.point(end));
        let c = self.free_point(center);
        let id = self.id();
        self.g.curves.push(Curve::Arc { id, center: c, start: s, end: e, construction: false });
        self.counts.arcs += 1;
        Ok(())
    }

    pub fn circle(&mut self, center: [f64; 2], radius: f64) -> Result<(), String> {
        if !(radius > MERGE) {
            return Ok(());
        }
        self.full()?;
        let c = self.free_point(center);
        let id = self.id();
        self.g.curves.push(Curve::Circle { id, center: c, radius, construction: false });
        self.counts.circles += 1;
        Ok(())
    }

    pub fn bezier(&mut self, a: [f64; 2], c1: [f64; 2], c2: [f64; 2], b: [f64; 2]) -> Result<(), String> {
        let near = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) <= MERGE;
        if near(a, b) && near(a, c1) && near(a, c2) {
            return Ok(());
        }
        // A straight Bézier is a line.
        let cross = |p: [f64; 2]| ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() / (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-12);
        if !near(a, b) && cross(c1) < MERGE && cross(c2) < MERGE {
            return self.line(a, b);
        }
        self.full()?;
        let (pa, pb) = (self.point(a), self.point(b));
        let (h1, h2) = (self.free_point(c1), self.free_point(c2));
        let id = self.id();
        self.g.curves.push(Curve::Bezier { id, a: pa, b: pb, c1: h1, c2: h2, construction: false });
        self.counts.curves += 1;
        Ok(())
    }

    /// A polyline through points (closing back to the first when asked).
    pub fn polyline(&mut self, pts: &[[f64; 2]], closed: bool) -> Result<(), String> {
        for w in pts.windows(2) {
            self.line(w[0], w[1])?;
        }
        if closed && pts.len() > 2 {
            self.line(pts[pts.len() - 1], pts[0])?;
        }
        Ok(())
    }

    /// The drawing moved so its bounds start at the origin (files often sit
    /// far from it), and its size.
    pub fn finish(mut self) -> Result<(Geometry, Counts, [f64; 2]), String> {
        if self.g.curves.is_empty() {
            return Err("the drawing has no lines, arcs, circles or curves".into());
        }
        // Bounds of what is drawn: ends, an arc's outermost points, points along Béziers.
        let at: HashMap<u32, [f64; 2]> = self.g.points.iter().map(|p| (p.id, [p.x, p.y])).collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        let mut take = |p: [f64; 2]| {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        };
        for c in &self.g.curves {
            match *c {
                Curve::Line { a, b, .. } => {
                    take(at[&a]);
                    take(at[&b]);
                }
                Curve::Circle { center, radius, .. } => {
                    let o = at[&center];
                    take([o[0] - radius, o[1] - radius]);
                    take([o[0] + radius, o[1] + radius]);
                }
                Curve::Arc { center, start, end, .. } => {
                    let (o, s, e) = (at[&center], at[&start], at[&end]);
                    take(s);
                    take(e);
                    let r = (s[0] - o[0]).hypot(s[1] - o[1]);
                    let (a0, mut a1) = ((s[1] - o[1]).atan2(s[0] - o[0]), (e[1] - o[1]).atan2(e[0] - o[0]));
                    while a1 <= a0 {
                        a1 += std::f64::consts::TAU;
                    }
                    // The axis directions the arc passes through.
                    for k in -4..8 {
                        let q = k as f64 * std::f64::consts::FRAC_PI_2;
                        if q > a0 && q < a1 {
                            take([o[0] + r * q.cos(), o[1] + r * q.sin()]);
                        }
                    }
                }
                Curve::Bezier { a, b, c1, c2, .. } => {
                    for i in 0..=16 {
                        take(crate::sketch::bezier_at(at[&a], at[&c1], at[&c2], at[&b], i as f64 / 16.0));
                    }
                }
                // Conics and splines are not made by a file import; their points bound them.
                ref other => {
                    for p in other.points() {
                        take(at[&p]);
                    }
                }
            }
        }
        for p in &mut self.g.points {
            p.x -= lo[0];
            p.y -= lo[1];
        }
        if self.g.points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Err("the drawing has points that are not numbers".into());
        }
        Ok((self.g, self.counts, [hi[0] - lo[0], hi[1] - lo[1]]))
    }
}
