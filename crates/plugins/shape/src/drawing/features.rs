//! Reading a part's features off its exact solid, the way a machinist reads
//! a model: holes (drilled, counterbored, countersunk; through or blind, how
//! deep), the diameters, shoulders, grooves and chamfers of a turned part,
//! the steps of a prismatic one (flat faces square to the axes), and the
//! radii a cutter leaves. The drawing dimensions what this finds.

use de_occt::{FaceInfo, SurfaceKind};
use std::f64::consts::{PI, TAU};

const EPS: f64 = 1e-4;

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add_scaled(a: [f64; 3], b: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}

/// The principal axis a direction lies along (0 x, 1 y, 2 z), if it does.
pub fn principal(d: [f64; 3]) -> Option<usize> {
    (0..3).find(|&k| d[k].abs() > 1.0 - 1e-6)
}

/// A line in space, written the same way whichever point and sense it was
/// given by: its direction with the first non-zero component positive, and
/// its point nearest the origin.
#[derive(Clone, Copy, Debug)]
pub struct Line {
    pub dir: [f64; 3],
    pub point: [f64; 3],
}

impl Line {
    pub fn new(point: [f64; 3], dir: [f64; 3]) -> Line {
        let l = dot(dir, dir).sqrt().max(1e-12);
        let mut d = [dir[0] / l, dir[1] / l, dir[2] / l];
        let first = d.iter().copied().find(|v| v.abs() > 1e-9).unwrap_or(1.0);
        if first < 0.0 {
            d = [-d[0], -d[1], -d[2]];
        }
        let t = dot(point, d);
        Line { dir: d, point: add_scaled(point, d, -t) }
    }
    pub fn same(&self, o: &Line) -> bool {
        dot(self.dir, o.dir) > 1.0 - 1e-7 && dot(sub(self.point, o.point), sub(self.point, o.point)).sqrt() < 1e-3
    }
    /// How far along the line a point is.
    pub fn along(&self, p: [f64; 3]) -> f64 {
        dot(p, self.dir)
    }
    pub fn at(&self, t: f64) -> [f64; 3] {
        add_scaled(self.point, self.dir, t)
    }
}

/// One stretch of a hole or a turned profile along its axis: a cylinder (r0 = r1) or a cone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stretch {
    pub from: f64,
    pub to: f64,
    /// Radius at `from` and at `to`.
    pub r0: f64,
    pub r1: f64,
}

impl Stretch {
    pub fn is_cylinder(&self) -> bool {
        (self.r0 - self.r1).abs() < EPS
    }
    pub fn length(&self) -> f64 {
        self.to - self.from
    }
}

/// A round hole: its stretches from the entry inward.
#[derive(Clone, Debug)]
pub struct Hole {
    pub line: Line,
    /// Where it starts (on the surface it is drilled from) and the way it goes in.
    pub entry: [f64; 3],
    pub inward: [f64; 3],
    /// Its stretches in the order the drill meets them, as distances from the entry.
    pub stretches: Vec<Stretch>,
    pub through: bool,
    /// The drilled (smallest full) diameter, and how deep it goes from the entry (to its bottom or the exit).
    pub diameter: f64,
    pub depth: f64,
    /// A counterbore: its diameter and depth.
    pub cbore: Option<(f64, f64)>,
    /// A countersink: its diameter at the surface and included angle (°).
    pub csk: Option<(f64, f64)>,
}

impl Hole {
    /// Holes that read the same in a callout are one size.
    pub fn signature(&self) -> String {
        let r = |v: f64| format!("{:.3}", v);
        format!("{}|{}|{}|{:?}|{:?}", r(self.diameter), self.through, if self.through { String::new() } else { r(self.depth) }, self.cbore.map(|c| (r(c.0), r(c.1))), self.csk.map(|c| (r(c.0), r(c.1))))
    }
    /// The axis it runs along, when that is x, y or z.
    pub fn axis(&self) -> Option<usize> {
        principal(self.line.dir)
    }
}

/// An arc a cutter or a round leaves: a part of a cylinder about an axis along x, y or z.
#[derive(Clone, Debug)]
pub struct Round {
    pub axis: usize,
    pub centre: [f64; 3],
    pub radius: f64,
    /// An inside corner (a pocket's corner, a slot's end) rather than a rounded edge.
    pub concave: bool,
    pub sweep: f64,
    /// Its middle, on the arc (for a leader).
    pub mid: [f64; 3],
    /// Its extent along its axis.
    pub span: [f64; 2],
}

/// A turned part: everything round about one axis.
#[derive(Clone, Debug)]
pub struct Turned {
    pub line: Line,
    /// The outside profile's cylinders and cones, in order along the axis.
    pub outside: Vec<Stretch>,
    /// Where flat faces square to the axis are (shoulders, ends, groove walls).
    pub shoulders: Vec<f64>,
    /// Rounds on the profile: (where along the axis, radius, inside corner).
    pub fillets: Vec<(f64, f64, bool)>,
}

/// A flat face square to x, y or z.
#[derive(Clone, Debug)]
pub struct Flat {
    pub axis: usize,
    pub at: f64,
    pub bounds: [f64; 6],
    pub area: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Features {
    pub holes: Vec<Hole>,
    /// Round bosses and pins (whole outside cylinders not part of a turned profile).
    pub bosses: Vec<(Line, f64, [f64; 2])>,
    pub rounds: Vec<Round>,
    pub flats: Vec<Flat>,
    pub turned: Option<Turned>,
    /// 45° bevels on edges: their setback (mm), how many.
    pub chamfers: Vec<(f64, usize)>,
}

/// Reads the features off the faces of one solid.
pub fn read(faces: &[FaceInfo]) -> Features {
    let mut f = Features::default();
    let size = {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for x in faces {
            for k in 0..3 {
                b[k] = b[k].min(x.bounds[k]);
                b[k + 3] = b[k + 3].max(x.bounds[k + 3]);
            }
        }
        b
    };
    // Round faces grouped by their axis line.
    let round = |x: &&FaceInfo| matches!(x.kind, SurfaceKind::Cylinder | SurfaceKind::Cone);
    let mut lines: Vec<(Line, Vec<usize>)> = vec![];
    for (i, x) in faces.iter().enumerate().filter(|(_, x)| round(x)) {
        let line = Line::new(x.point, x.dir);
        match lines.iter_mut().find(|(l, _)| l.same(&line)) {
            Some((_, v)) => v.push(i),
            None => lines.push((line, vec![i])),
        }
    }
    // A face's extent along a line (its own axis may point the other way).
    let span = |l: &Line, x: &FaceInfo| -> Stretch {
        let s = if dot(x.dir, l.dir) >= 0.0 { 1.0 } else { -1.0 };
        let base = l.along(x.point);
        let (a, b) = (base + s * x.along[0], base + s * x.along[1]);
        let (ra, rb) = (x.radius[0], x.radius[1]);
        if a <= b { Stretch { from: a, to: b, r0: ra, r1: rb } } else { Stretch { from: b, to: a, r0: rb, r1: ra } }
    };
    let planes: Vec<&FaceInfo> = faces.iter().filter(|x| x.kind == SurfaceKind::Plane).collect();
    // Whether the hole on `l` is closed at `t` (its end): a flat bottom the
    // hole's own size, or a drill point.
    let closed = |l: &Line, t: f64, r: f64, cones: &[Stretch]| {
        let p = l.at(t);
        let flat = planes.iter().any(|q| {
            if dot(q.dir, l.dir).abs() < 1.0 - 1e-6 || (l.along(q.point) - t).abs() > 1e-3 {
                return false;
            }
            let b = q.bounds;
            let inside = (0..3).all(|k| p[k] >= b[k] - 1e-3 && p[k] <= b[k + 3] + 1e-3);
            let reach = (0..3).map(|k| (b[k + 3] - b[k]).max(0.0)).fold(0.0, f64::max);
            inside && reach <= 2.0 * r + 1e-2
        });
        flat || cones.iter().any(|c| (c.from - t).abs() < 1e-3 && c.r0 < 1e-3 || (c.to - t).abs() < 1e-3 && c.r1 < 1e-3)
    };
    let mut hole_planes: Vec<usize> = vec![];
    for (line, members) in &lines {
        // Concave whole rounds make holes; partial ones are corners or slot ends.
        let mut cyl: Vec<(Stretch, f64)> = vec![];
        let mut cones: Vec<Stretch> = vec![];
        for &i in members {
            let x = &faces[i];
            if !x.concave {
                continue;
            }
            let s = span(line, x);
            if x.kind == SurfaceKind::Cylinder {
                // Two halves of one wall (a split seam) add up.
                match cyl.iter_mut().find(|(c, _)| (c.r0 - s.r0).abs() < EPS && (c.from - s.from).abs() < 1e-3 && (c.to - s.to).abs() < 1e-3) {
                    Some(c) => c.1 += x.sweep,
                    None => cyl.push((s, x.sweep)),
                }
            } else if x.sweep > TAU - 1e-3 {
                cones.push(s);
            }
        }
        let mut whole: Vec<Stretch> = cyl.iter().filter(|c| c.1 > TAU - 1e-3).map(|c| c.0).chain(cones.iter().copied()).collect();
        whole.sort_by(|a, b| a.from.total_cmp(&b.from));
        // Stretches that meet end to end are one hole.
        let mut chains: Vec<Vec<Stretch>> = vec![];
        for s in whole {
            match chains.last_mut() {
                Some(c) if (c.last().unwrap().to - s.from).abs() < 1e-3 => c.push(s),
                _ => chains.push(vec![s]),
            }
        }
        for chain in chains {
            if !chain.iter().any(Stretch::is_cylinder) {
                continue; // a lone cone is a chamfer or a seat, not a hole
            }
            let (t0, t1) = (chain[0].from, chain.last().unwrap().to);
            let (r_lo, r_hi) = (chain[0].r0, chain.last().unwrap().r1);
            let shut_lo = closed(line, t0, r_lo.max(chain[0].r1), &cones);
            let shut_hi = closed(line, t1, r_hi.max(chain.last().unwrap().r0), &cones);
            if shut_lo && shut_hi {
                continue; // a closed cavity
            }
            // Drilled from the open end; when both are open, from the wider end
            // (a counterbore or countersink), else from the top, front or right.
            let from_hi = if shut_lo != shut_hi {
                shut_lo
            } else {
                let (wide_lo, wide_hi) = (chain[0].r0.max(chain[0].r1), chain.last().unwrap().r0.max(chain.last().unwrap().r1));
                if (wide_lo - wide_hi).abs() > EPS { wide_hi > wide_lo } else { true }
            };
            let through = !shut_lo && !shut_hi;
            let (entry_t, sign) = if from_hi { (t1, -1.0) } else { (t0, 1.0) };
            let mut stretches: Vec<Stretch> = chain
                .iter()
                .map(|s| {
                    let (a, b) = ((s.from - entry_t) * sign, (s.to - entry_t) * sign);
                    if a <= b { Stretch { from: a, to: b, r0: s.r0, r1: s.r1 } } else { Stretch { from: b, to: a, r0: s.r1, r1: s.r0 } }
                })
                .collect();
            stretches.sort_by(|a, b| a.from.total_cmp(&b.from));
            let cylinders: Vec<&Stretch> = stretches.iter().filter(|s| s.is_cylinder()).collect();
            let drill = cylinders.iter().copied().min_by(|a, b| a.r0.total_cmp(&b.r0)).copied().unwrap();
            let depth = stretches.iter().filter(|s| s.is_cylinder() && (s.r0 - drill.r0).abs() < EPS).map(|s| s.to).fold(0.0, f64::max);
            let first = stretches[0];
            let csk = (!first.is_cylinder() && first.r0 > first.r1).then(|| (2.0 * first.r0, 2.0 * ((first.r0 - first.r1) / first.length().max(1e-9)).atan().to_degrees()));
            let cbore = stretches.iter().find(|s| s.is_cylinder()).filter(|s| s.r0 > drill.r0 + EPS).map(|s| (2.0 * s.r0, s.to));
            let inward = [line.dir[0] * sign, line.dir[1] * sign, line.dir[2] * sign];
            let hole = Hole { line: *line, entry: line.at(entry_t), inward, stretches, through, diameter: 2.0 * drill.r0, depth, cbore, csk };
            // The flats inside the hole (its bottom, the counterbore's floor) are the hole's, not steps.
            for (j, q) in faces.iter().enumerate() {
                if q.kind != SurfaceKind::Plane || dot(q.dir, line.dir).abs() < 1.0 - 1e-6 {
                    continue;
                }
                let t = line.along(q.point);
                let (lo, hi) = (t0.min(t1) - 1e-3, t0.max(t1) + 1e-3);
                let rmax = hole.stretches.iter().map(|s| s.r0.max(s.r1)).fold(0.0, f64::max);
                let c = line.at(t);
                let within = (0..3).all(|k| q.bounds[k] >= c[k] - rmax - 1e-3 && q.bounds[k + 3] <= c[k] + rmax + 1e-3);
                if t >= lo && t <= hi && within {
                    hole_planes.push(j);
                }
            }
            f.holes.push(hole);
        }
    }

    // A turned part: every round face on one axis, every flat face square to it.
    let any_round = faces.iter().any(|x| matches!(x.kind, SurfaceKind::Cylinder | SurfaceKind::Cone | SurfaceKind::Torus));
    let turned_line = lines.first().map(|l| l.0).filter(|l| {
        any_round
            && faces.iter().all(|x| match x.kind {
                SurfaceKind::Cylinder | SurfaceKind::Cone | SurfaceKind::Torus => Line::new(x.point, x.dir).same(l),
                SurfaceKind::Plane => dot(x.dir, l.dir).abs() > 1.0 - 1e-6,
                SurfaceKind::Sphere => (dot(sub(x.point, l.point), sub(x.point, l.point)) - l.along(x.point).powi(2)).abs() < 1e-3,
                SurfaceKind::Other => false,
            })
            && principal(l.dir).is_some()
    });
    if let Some(line) = turned_line {
        let mut outside: Vec<Stretch> = vec![];
        for x in faces.iter().filter(|x| matches!(x.kind, SurfaceKind::Cylinder | SurfaceKind::Cone) && !x.concave) {
            let s = span(&line, x);
            if !outside.iter().any(|o| (o.from - s.from).abs() < 1e-3 && (o.to - s.to).abs() < 1e-3 && (o.r0 - s.r0).abs() < EPS) {
                outside.push(s);
            }
        }
        // The bore of a hollow turned part is a hole; anything concave and
        // not a hole (an inside cone at a step) is part of the profile too.
        outside.sort_by(|a, b| a.from.total_cmp(&b.from));
        let mut shoulders: Vec<f64> = faces.iter().enumerate().filter(|(j, q)| q.kind == SurfaceKind::Plane && !hole_planes.contains(j)).map(|(_, q)| line.along(q.point)).collect();
        shoulders.sort_by(f64::total_cmp);
        shoulders.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        let fillets = faces
            .iter()
            .filter(|x| x.kind == SurfaceKind::Torus)
            .map(|x| (line.along(x.point), x.radius[1], x.concave))
            .collect();
        f.turned = Some(Turned { line, outside, shoulders, fillets });
    } else {
        // Whole outside cylinders are bosses and pins.
        for (line, members) in &lines {
            let mut seen: Vec<(f64, Stretch, f64)> = vec![];
            for &i in members {
                let x = &faces[i];
                if x.kind != SurfaceKind::Cylinder || x.concave {
                    continue;
                }
                let s = span(line, x);
                match seen.iter_mut().find(|(r, c, _)| (r - x.radius[0]).abs() < EPS && (c.from - s.from).abs() < 1e-3) {
                    Some(c) => c.2 += x.sweep,
                    None => seen.push((x.radius[0], s, x.sweep)),
                }
            }
            for (r, s, sweep) in seen {
                if sweep > TAU - 1e-3 {
                    f.bosses.push((*line, 2.0 * r, [s.from, s.to]));
                }
            }
        }
    }

    // Partial cylinders about x, y or z: a cutter's corners, slot ends, rounded edges.
    for x in faces.iter().filter(|x| x.kind == SurfaceKind::Cylinder && x.sweep < TAU - 1e-3) {
        let Some(axis) = principal(x.dir) else { continue };
        let line = Line::new(x.point, x.dir);
        let s = span(&line, x);
        // Its middle: the box's corner-most point on the arc is the face's centre of area direction.
        let b = x.bounds;
        let middle = [(b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0, (b[2] + b[5]) / 2.0];
        let t = line.along(middle);
        let c = line.at(t);
        let out = sub(middle, c);
        let l = dot(out, out).sqrt();
        let mid = if l > 1e-9 { add_scaled(c, out, x.radius[0] / l) } else { middle };
        if x.sweep < PI / 18.0 {
            continue; // a sliver
        }
        f.rounds.push(Round { axis, centre: c, radius: x.radius[0], concave: x.concave, sweep: x.sweep, mid, span: [s.from, s.to] });
    }

    // Flat faces square to the axes: the steps a drawing dimensions.
    for (j, q) in faces.iter().enumerate() {
        if q.kind != SurfaceKind::Plane || hole_planes.contains(&j) {
            continue;
        }
        let Some(axis) = principal(q.dir) else { continue };
        f.flats.push(Flat { axis, at: q.point[axis], bounds: q.bounds, area: q.area });
    }

    // 45° bevels between two axis-square flats, read off flat faces whose normal is half way.
    let mut chamfers: Vec<(f64, usize)> = vec![];
    let diagonal = 0.5f64.sqrt();
    for q in faces.iter().filter(|q| q.kind == SurfaceKind::Plane && principal(q.dir).is_none()) {
        let comps: Vec<usize> = (0..3).filter(|&k| q.dir[k].abs() > 1e-6).collect();
        if comps.len() != 2 || !comps.iter().all(|&k| (q.dir[k].abs() - diagonal).abs() < 1e-4) {
            continue;
        }
        // The setback: the face's extent along either of its two axes.
        let setback = (q.bounds[comps[0] + 3] - q.bounds[comps[0]]).min(q.bounds[comps[1] + 3] - q.bounds[comps[1]]);
        let setback = (setback * 100.0).round() / 100.0;
        match chamfers.iter_mut().find(|c| (c.0 - setback).abs() < 1e-3) {
            Some(c) => c.1 += 1,
            None => chamfers.push((setback, 1)),
        }
    }
    // A turned part's chamfers are 45° cones at the ends of its stretches.
    if let Some(t) = &f.turned {
        for s in t.outside.iter().filter(|s| !s.is_cylinder()) {
            let dr = (s.r1 - s.r0).abs();
            if (dr - s.length()).abs() < 1e-3 {
                let setback = (s.length() * 100.0).round() / 100.0;
                match chamfers.iter_mut().find(|c| (c.0 - setback).abs() < 1e-3) {
                    Some(c) => c.1 += 1,
                    None => chamfers.push((setback, 1)),
                }
            }
        }
    }
    f.chamfers = chamfers;
    let _ = size;
    f
}

/// ISO 261 coarse pitches by nominal diameter (mm).
const COARSE: [(f64, f64); 17] = [
    (1.6, 0.35),
    (2.0, 0.4),
    (2.5, 0.45),
    (3.0, 0.5),
    (4.0, 0.7),
    (5.0, 0.8),
    (6.0, 1.0),
    (8.0, 1.25),
    (10.0, 1.5),
    (12.0, 1.75),
    (14.0, 2.0),
    (16.0, 2.0),
    (20.0, 2.5),
    (24.0, 3.0),
    (30.0, 3.5),
    (36.0, 4.0),
    (42.0, 4.5),
];

/// The coarse pitch of an ISO metric thread, when the size has one.
pub fn coarse_pitch(nominal: f64) -> Option<f64> {
    COARSE.iter().find(|(d, _)| (d - nominal).abs() < 1e-6).map(|c| c.1)
}

/// Reads "M6", "M16x1.5", "M16×1.5": the nominal diameter and pitch (coarse when not given).
pub fn parse_thread(s: &str) -> Result<(f64, f64), String> {
    let t = s.trim();
    let body = t.strip_prefix('M').or_else(|| t.strip_prefix('m')).ok_or_else(|| format!("“{s}” is not an ISO metric thread (like M6 or M16×1.5)"))?;
    let (d, p) = match body.split_once(['x', '×', 'X']) {
        Some((d, p)) => (d.trim(), Some(p.trim())),
        None => (body, None),
    };
    let d: f64 = d.parse().map_err(|_| format!("“{s}” is not an ISO metric thread (like M6 or M16×1.5)"))?;
    let pitch = match p {
        Some(p) => p.parse::<f64>().map_err(|_| format!("“{s}”: the pitch is a number of mm"))?,
        None => coarse_pitch(d).ok_or_else(|| format!("M{d} has no coarse pitch in ISO 261 here: give it, like M{d}×1.5"))?,
    };
    if !(d > 0.0 && pitch > 0.0 && pitch < d) {
        return Err(format!("“{s}” is not a thread size"));
    }
    Ok((d, pitch))
}

/// An external thread's minor diameter (ISO 68-1: d − 1.22687 P) and an
/// internal one's (D − 1.08253 P), where the cosmetic thread lines go.
pub fn minor_diameter(nominal: f64, pitch: f64, internal: bool) -> f64 {
    nominal - pitch * if internal { 1.082532 } else { 1.226869 }
}
