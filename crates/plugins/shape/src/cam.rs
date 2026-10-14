//! Toolpaths for a 3-axis router or mill: a part cut from flat stock, from above.
//!
//! The part is laid flat (see `fabricate::lay_flat`) and sliced at every depth
//! where its section changes. What the tool removes at each depth is the
//! part's footprint minus its section there; that is cleared with concentric
//! offsets of the tool's radius. Holes through the part are cleared or cut
//! round (with tabs when big), and the outline is cut free last, with tabs so
//! the part stays put. The result is G-code for GRBL-compatible controllers
//! (Shapeoko, X-Carve, OneFinity, grblHAL, FluidNC), plus the moves for a
//! preview and the warnings a machinist needs before cutting.
//!
//! Work zero: X0 Y0 at the lower-left corner of the part, Z0 on top of the
//! stock (the part's top face). Cuts go to negative Z.
//!
//! Parked (2026-10-09): tested here, not yet wired to the server or the UI.
//! The user cuts with VCarve (router), LightBurn/EZCAD (fiber laser) and
//! RDWorks (CO2): match those when machine work resumes.

use crate::fabricate::lay_flat;
use manifold_csg::{CrossSection, JoinType, Manifold};
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// How to cut: the tool, its speeds and the cutting strategy (mm, mm/min, rpm).
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct Setup {
    /// Flat end mill diameter.
    pub tool_diameter: f64,
    pub spindle_rpm: f64,
    /// Cutting feed.
    pub feed: f64,
    /// Plunging feed.
    pub plunge: f64,
    /// Depth per pass.
    pub step_down: f64,
    /// Sideways step when clearing, as a fraction of the tool diameter.
    pub step_over: f64,
    /// Height for moves between cuts.
    pub safe_z: f64,
    /// How far the through cuts go below the stock, into the spoilboard.
    pub overcut: f64,
    /// Hold the part (and big hole slugs) with tabs on the last passes.
    pub tabs: bool,
    pub tab_height: f64,
    pub tab_width: f64,
    /// Climb milling (true) or conventional (false, kinder to light machines).
    pub climb: bool,
    /// The machine's working area, to warn when a part will not fit (x, y).
    pub bed: Option<[f64; 2]>,
}

impl Default for Setup {
    /// A 1/8" (3.175 mm) flat end mill in plywood or MDF on a desktop router.
    fn default() -> Self {
        Setup {
            tool_diameter: 3.175,
            spindle_rpm: 18000.0,
            feed: 1200.0,
            plunge: 400.0,
            step_down: 1.5,
            step_over: 0.4,
            safe_z: 5.0,
            overcut: 0.3,
            tabs: true,
            tab_height: 2.0,
            tab_width: 6.0,
            climb: false,
            bed: None,
        }
    }
}

/// One move of the tool, for the preview.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Move {
    /// "rapid", "plunge" or "cut".
    pub kind: &'static str,
    /// Work coordinates (mm), from the previous move's end.
    pub to: [f64; 3],
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Stats {
    /// Estimated machining time (seconds), at the programmed feeds.
    pub seconds: f64,
    /// Length cut (mm).
    pub cut_length: f64,
    /// The stock the part is cut from: x, y and thickness (mm).
    pub stock: [f64; 3],
    pub lines: usize,
}

/// What was cut, in the order the G-code does it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Operation {
    pub name: String,
    /// Depth below the stock's top (mm).
    pub depth: f64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Job {
    pub gcode: String,
    pub moves: Vec<Move>,
    pub operations: Vec<Operation>,
    pub stats: Stats,
    pub warnings: Vec<String>,
    /// The part as cut (laid flat, work coordinates): x, y, z per vertex, and triangles.
    pub positions: Vec<f32>,
    pub indices: Vec<u32>,
}

type Ring = Vec<[f64; 2]>;

fn section(rings: Vec<Ring>) -> CrossSection {
    CrossSection::from_polygons(&rings)
}

fn signed_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1]).sum::<f64>() / 2.0
}

fn perimeter(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| dist(ring[i], ring[(i + 1) % n])).sum()
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Heights of a flat part's horizontal faces (where its section changes).
fn levels(flat: &Manifold, top: f64) -> Vec<f64> {
    let (positions, n_props, indices) = flat.to_mesh_f64();
    let n = n_props.max(3);
    let p = |i: u64| {
        let i = i as usize * n;
        [positions[i], positions[i + 1], positions[i + 2]]
    };
    let mut zs = vec![0.0, top];
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let nz = u[0] * w[1] - u[1] * w[0];
        let len = ((u[1] * w[2] - u[2] * w[1]).powi(2) + (u[2] * w[0] - u[0] * w[2]).powi(2) + nz * nz).sqrt();
        if len > 0.0 && nz.abs() / len > 0.9999 {
            zs.push((a[2] + b[2] + c[2]) / 3.0);
        }
    }
    zs.sort_by(f64::total_cmp);
    zs.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    zs
}

/// Depths from `from` to `to` (both below the top, `to` deeper), one per pass.
fn passes(from: f64, to: f64, step: f64) -> Vec<f64> {
    let n = ((to - from) / step - 1e-9).ceil().max(1.0) as usize;
    (1..=n).map(|i| from + (to - from) * i as f64 / n as f64).collect()
}

struct Writer<'a> {
    setup: &'a Setup,
    g: String,
    moves: Vec<Move>,
    at: [f64; 3],
    cut: f64,
    seconds: f64,
}

/// A coordinate as G-code writes it: up to 3 decimals, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

const RAPID: f64 = 5000.0; // mm/min, a typical desktop router's rapid rate (for the estimate)

impl Writer<'_> {
    fn go(&mut self, kind: &'static str, to: [f64; 3]) {
        let d = ((to[0] - self.at[0]).powi(2) + (to[1] - self.at[1]).powi(2) + (to[2] - self.at[2]).powi(2)).sqrt();
        if d < 1e-6 {
            return;
        }
        let f = num;
        match kind {
            "rapid" => {
                let _ = writeln!(self.g, "G0 X{} Y{} Z{}", f(to[0]), f(to[1]), f(to[2]));
                self.seconds += d / RAPID * 60.0;
            }
            "plunge" => {
                let _ = writeln!(self.g, "G1 X{} Y{} Z{} F{}", f(to[0]), f(to[1]), f(to[2]), f(self.setup.plunge));
                self.seconds += d / self.setup.plunge * 60.0;
                self.cut += d;
            }
            _ => {
                let _ = writeln!(self.g, "G1 X{} Y{} Z{} F{}", f(to[0]), f(to[1]), f(to[2]), f(self.setup.feed));
                self.seconds += d / self.setup.feed * 60.0;
                self.cut += d;
            }
        }
        self.moves.push(Move { kind, to });
        self.at = to;
    }

    /// Up to the safe height, over to (x, y), then down to `z` at the plunge feed.
    fn enter(&mut self, x: f64, y: f64, z: f64) {
        let safe = self.setup.safe_z;
        if self.at[2] < safe - 1e-9 {
            self.go("rapid", [self.at[0], self.at[1], safe]);
        }
        self.go("rapid", [x, y, safe]);
        self.go("rapid", [x, y, 1.0_f64.min(safe)]);
        self.go("plunge", [x, y, z]);
    }

    fn retract(&mut self) {
        let safe = self.setup.safe_z;
        self.go("rapid", [self.at[0], self.at[1], safe]);
    }

    /// A closed ring at depth z (from its first point and back to it).
    fn ring(&mut self, ring: &[[f64; 2]], z: f64) {
        for p in ring.iter().skip(1).chain(ring.first()) {
            self.go("cut", [p[0], p[1], z]);
        }
    }

    /// A closed ring at depth z, lifted to `tab_z` over each tab.
    fn ring_with_tabs(&mut self, ring: &[[f64; 2]], z: f64, tab_z: f64, tabs: &[(f64, f64)]) {
        let in_tab = |s: f64| tabs.iter().any(|(a, b)| s > *a && s < *b);
        let mut s = 0.0;
        let n = ring.len();
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            let l = dist(a, b);
            // Split the segment at tab edges.
            let mut cuts: Vec<f64> = tabs.iter().flat_map(|(t0, t1)| [*t0, *t1]).filter(|t| *t > s && *t < s + l).map(|t| (t - s) / l).collect();
            cuts.push(1.0);
            cuts.sort_by(f64::total_cmp);
            let mut prev = 0.0;
            for t in cuts {
                let mid = s + l * (prev + t) / 2.0;
                let zz = if in_tab(mid) { tab_z } else { z };
                let p0 = [a[0] + (b[0] - a[0]) * prev, a[1] + (b[1] - a[1]) * prev];
                let p1 = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                if (self.at[2] - zz).abs() > 1e-9 {
                    self.go(if zz < self.at[2] { "plunge" } else { "cut" }, [p0[0], p0[1], zz]);
                }
                self.go("cut", [p1[0], p1[1], zz]);
                prev = t;
            }
            s += l;
        }
    }
}

/// Rings with the cutting direction chosen: outlines are cut with the part on
/// one side, holes and pockets with it on the other.
fn directed(mut ring: Ring, outside: bool, climb: bool) -> Ring {
    // Conventional milling with a clockwise spindle: outside counter-clockwise, inside clockwise.
    let want_ccw = outside != climb;
    if (signed_area(&ring) > 0.0) != want_ccw {
        ring.reverse();
    }
    ring
}

/// Concentric clearing rings of a region, innermost first, each the tool's
/// centre path (the region shrunk by the tool radius, then by the step-over).
fn clearing(region: &CrossSection, r: f64, step: f64, segments: i32) -> Vec<Vec<Ring>> {
    let mut out = Vec::new();
    let mut cur = region.offset(-r, JoinType::Round, 2.0, segments);
    while !cur.is_empty() && cur.area() > 1e-6 {
        out.push(cur.to_polygons());
        cur = cur.offset(-step, JoinType::Round, 2.0, segments);
    }
    out.reverse();
    out
}

/// Plans and writes the G-code for one part.
pub fn job(name: &str, solid: &Manifold, setup: &Setup) -> Result<Job, String> {
    let s = setup;
    if !(s.tool_diameter > 0.0 && s.feed > 0.0 && s.plunge > 0.0 && s.step_down > 0.0 && s.step_over > 0.0 && s.step_over <= 1.0) {
        return Err("the tool diameter, feeds and step-down must be positive, and the step-over between 0 and 1".into());
    }
    let flat = lay_flat(solid);
    let bb = flat.bounding_box().ok_or("the part is empty")?;
    let (lo, hi) = (bb.min(), bb.max());
    let top = hi[2] - lo[2];
    let r = s.tool_diameter / 2.0;
    let segments = 48;
    let mut warnings = Vec::new();

    let zs = levels(&flat, top);
    let slice = |z: f64| section(flat.slice_at_z(z));
    // The footprint: what sits on the spoilboard. Outer rings and holes.
    let foot = slice(zs[1].min(top) * 0.5);
    let rings = foot.to_polygons();
    let outers: Vec<Ring> = rings.iter().filter(|g| signed_area(g) > 0.0).cloned().collect();
    let holes: Vec<Ring> = rings.iter().filter(|g| signed_area(g) < 0.0).cloned().collect();
    let filled = section(outers.clone());

    // Bands between levels, top down: what to clear in each.
    let mut bands = Vec::new();
    for w in zs.windows(2).rev() {
        let (b0, b1) = (w[0], w[1]);
        if b0 < 1e-6 {
            continue; // the bottom band is the footprint itself
        }
        let here = slice((b0 + b1) / 2.0);
        bands.push((top - b1, top - b0, foot.difference(&here)));
    }
    // A section that grows higher up can only be cut from the other side.
    let mut above: Option<CrossSection> = None;
    for w in zs.windows(2) {
        let here = slice((w[0] + w[1]) / 2.0);
        if let Some(below) = &above
            && here.difference(below).area() > 1.0
        {
            warnings.push(format!("part of {name} overhangs at {:.1} mm from the bottom: a 3-axis cut from above cannot reach under it", w[0]));
            break;
        }
        above = Some(here);
    }

    let mut w = Writer { setup: s, g: String::new(), moves: Vec::new(), at: [0.0, 0.0, s.safe_z], cut: 0.0, seconds: 0.0 };
    let [sx, sy, _] = [hi[0] - lo[0], hi[1] - lo[1], top];
    let header = format!(
        "(ISE Studio: {name})\n(Stock {:.1} x {:.1} x {:.2} mm. Zero: X0 Y0 at the part's lower-left corner, Z0 on top of the stock.)\n(Tool: {:.3} mm flat end mill at {:.0} rpm, feed {:.0}, plunge {:.0} mm/min, {:.2} mm per pass.)\n",
        sx, sy, top, s.tool_diameter, s.spindle_rpm, s.feed, s.plunge, s.step_down
    );
    w.g.push_str(&header);
    w.g.push_str("G21 G90 G17 G94\n");
    let _ = writeln!(w.g, "G0 Z{:.3}", s.safe_z);
    let _ = writeln!(w.g, "M3 S{:.0}", s.spindle_rpm);
    w.g.push_str("G4 P3\n");
    let mut operations = Vec::new();
    // Work coordinates: the part's lower-left corner at the origin.
    let shift = |ring: &Ring| -> Ring { ring.iter().map(|p| [p[0] - lo[0], p[1] - lo[1]]).collect() };
    let through = top + s.overcut;
    let step = s.step_over * s.tool_diameter;

    // 1. Pockets, band by band from the top.
    for (k, (from, to, region)) in bands.iter().enumerate() {
        if region.area() < 1e-3 {
            continue;
        }
        let paths = clearing(region, r, step, segments);
        let reached = region.offset(-r, JoinType::Round, 2.0, segments).offset(r, JoinType::Round, 2.0, segments);
        let missed = region.difference(&reached).area();
        if paths.is_empty() {
            warnings.push(format!("a pocket {:.1} mm deep is narrower than the {:.2} mm tool and is skipped", to, s.tool_diameter));
            continue;
        }
        if missed > 0.5 {
            warnings.push(format!("{missed:.1} mm² of the pocket {to:.1} mm deep is narrower than the tool (inside corners stay round, radius {r:.2} mm)"));
        }
        operations.push(Operation { name: format!("Pocket {}", k + 1), depth: *to });
        for z in passes(*from, *to, s.step_down) {
            let mut first = true;
            for level in &paths {
                for ring in level {
                    let ring = directed(shift(ring), false, s.climb);
                    if first {
                        w.enter(ring[0][0], ring[0][1], -z);
                        first = false;
                    } else {
                        w.go("cut", [ring[0][0], ring[0][1], -z]);
                    }
                    w.ring(&ring, -z);
                }
            }
            w.retract();
        }
    }

    // 2. Holes through the part: small ones cleared, big ones cut round with tabs.
    for (i, hole) in holes.iter().enumerate() {
        let region = section(vec![hole.iter().rev().copied().collect()]);
        let inner = region.offset(-r, JoinType::Round, 2.0, segments);
        if inner.is_empty() {
            warnings.push(format!("hole {} is narrower than the {:.2} mm tool and is skipped", i + 1, s.tool_diameter));
            continue;
        }
        let big = perimeter(hole) > 30.0 * s.tool_diameter;
        operations.push(Operation { name: format!("Hole {}{}", i + 1, if big { " (cut round, tabs)" } else { " (cleared)" }), depth: through });
        if big {
            for ring in inner.to_polygons() {
                let ring = directed(shift(&ring), false, s.climb);
                let tabs = tab_spans(&ring, s);
                for z in passes(0.0, through, s.step_down) {
                    w.enter(ring[0][0], ring[0][1], -z);
                    if s.tabs && z > top - s.tab_height { w.ring_with_tabs(&ring, -z, -(top - s.tab_height), &tabs) } else { w.ring(&ring, -z) }
                    w.retract();
                }
            }
        } else {
            let paths = clearing(&region, r, step, segments);
            for z in passes(0.0, through, s.step_down) {
                let mut first = true;
                for level in &paths {
                    for ring in level {
                        let ring = directed(shift(ring), false, s.climb);
                        if first {
                            w.enter(ring[0][0], ring[0][1], -z);
                            first = false;
                        } else {
                            w.go("cut", [ring[0][0], ring[0][1], -z]);
                        }
                        w.ring(&ring, -z);
                    }
                }
                w.retract();
            }
        }
    }

    // 3. The outline, cut free last, with tabs on the final passes.
    let outline = filled.offset(r, JoinType::Round, 2.0, segments);
    operations.push(Operation { name: format!("Outline{}", if s.tabs { " (tabs)" } else { "" }), depth: through });
    for ring in outline.to_polygons().into_iter().filter(|g| signed_area(g) > 0.0) {
        let ring = directed(shift(&ring), true, s.climb);
        let tabs = tab_spans(&ring, s);
        for z in passes(0.0, through, s.step_down) {
            w.enter(ring[0][0], ring[0][1], -z);
            if s.tabs && z > top - s.tab_height { w.ring_with_tabs(&ring, -z, -(top - s.tab_height), &tabs) } else { w.ring(&ring, -z) }
            w.retract();
        }
    }

    w.g.push_str("M5\n");
    let _ = writeln!(w.g, "G0 Z{:.3}", s.safe_z);
    w.g.push_str("G0 X0 Y0\nM30\n");
    w.moves.push(Move { kind: "rapid", to: [0.0, 0.0, s.safe_z] });

    if let Some([bx, by]) = s.bed
        && (sx + s.tool_diameter > bx || sy + s.tool_diameter > by)
    {
        warnings.push(format!("the part ({sx:.0} x {sy:.0} mm, plus the tool) is larger than the machine's {bx:.0} x {by:.0} mm bed"));
    }
    if top > 30.0 * s.tool_diameter / 3.175 {
        warnings.push(format!("the part is {top:.1} mm thick: check your {:.2} mm tool's cutting length reaches", s.tool_diameter));
    }
    if s.tabs {
        warnings.push(format!("tabs {:.1} mm high hold the part: cut them after it comes off the machine", s.tab_height));
    }

    // The part in work coordinates (top at Z0), for the preview.
    let shown = flat.translate(-lo[0], -lo[1], -hi[2]);
    let (props, n, idx) = shown.to_mesh_f32();
    let n = n.max(3);
    let positions = props.chunks_exact(n).flat_map(|v| [v[0], v[1], v[2]]).collect();
    let lines = w.g.lines().count();
    let stats = Stats { seconds: w.seconds, cut_length: w.cut, stock: [sx, sy, top], lines };
    Ok(Job { gcode: w.g, moves: w.moves, operations, stats, warnings, positions, indices: idx })
}

/// Where the tabs go along a ring: `(start, end)` distances, evenly spread.
fn tab_spans(ring: &[[f64; 2]], s: &Setup) -> Vec<(f64, f64)> {
    let len = perimeter(ring);
    let n = ((len / 250.0).round() as usize).clamp(2, 8);
    let width = s.tab_width + s.tool_diameter;
    (0..n)
        .map(|i| {
            let c = len * (i as f64 + 0.5) / n as f64;
            (c - width / 2.0, c + width / 2.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays G-code back: every cutting move's lowest point, and the XY reach.
    fn simulate(g: &str) -> (f64, [f64; 4]) {
        let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
        let mut deepest = 0.0_f64;
        let mut reach = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for line in g.lines().filter(|l| l.starts_with("G0") || l.starts_with("G1")) {
            for w in line.split_whitespace() {
                let v = |s: &str| s[1..].parse::<f64>().unwrap();
                match w.chars().next() {
                    Some('X') => x = v(w),
                    Some('Y') => y = v(w),
                    Some('Z') => z = v(w),
                    _ => {}
                }
            }
            if line.starts_with("G1") {
                deepest = deepest.min(z);
                reach = [reach[0].min(x), reach[1].min(y), reach[2].max(x), reach[3].max(y)];
            }
        }
        (deepest, reach)
    }

    #[test]
    fn a_plate_with_a_pocket_and_holes() {
        // 100 × 60 × 12 plate: a 40 × 20 pocket 5 deep, a Ø6 hole and a Ø40 hole.
        let plate = Manifold::cube(100.0, 60.0, 12.0, false)
            .difference(&Manifold::cube(40.0, 20.0, 6.0, false).translate(10.0, 10.0, 7.0))
            .difference(&Manifold::cylinder(14.0, 3.0, 3.0, 48, false).translate(30.0, 45.0, -1.0))
            .difference(&Manifold::cylinder(14.0, 20.0, 20.0, 96, false).translate(75.0, 32.0, -1.0));
        let job = job("Plate", &plate, &Setup::default()).unwrap();
        let (deepest, reach) = simulate(&job.gcode);
        assert!((deepest + 12.3).abs() < 1e-6, "through cuts reach the overcut: {deepest}");
        let r = 3.175 / 2.0;
        assert!((reach[0] + r).abs() < 0.05 && (reach[2] - 100.0 - r).abs() < 0.05, "the outline runs a tool radius outside: {reach:?}");
        let names: Vec<&str> = job.operations.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names.len(), 4, "{names:?}");
        assert!(names[0].starts_with("Pocket") && (job.operations[0].depth - 5.0).abs() < 1e-6, "{:?}", job.operations[0]);
        assert!(names.iter().any(|n| n.contains("cleared")) && names.iter().any(|n| n.contains("cut round")), "{names:?}");
        assert!(names.last().unwrap().starts_with("Outline"));
        assert!(job.gcode.contains("M3 S18000") && job.gcode.trim_end().ends_with("M30"));
        assert!(job.stats.seconds > 60.0 && job.stats.stock == [100.0, 60.0, 12.0], "{:?}", job.stats);
        // Tabs: the outline's last passes rise to the tab top somewhere.
        assert!(job.gcode.contains("Z-10 F"), "tab tops at 12 − 2 = 10 mm deep");
        // The pocket (cut first) never goes below its 5 mm floor.
        let pocket_end = job.gcode.find("Z-1.367").expect("the first hole pass");
        let (pocket_deepest, _) = simulate(&job.gcode[..pocket_end]);
        assert!((pocket_deepest + 5.0).abs() < 1e-9, "{pocket_deepest}");
    }

    #[test]
    fn warnings_for_what_cannot_be_cut() {
        // A 2 mm slot cannot take a 3.175 mm tool; a part bigger than the bed is flagged.
        let plate = Manifold::cube(300.0, 50.0, 6.0, false).difference(&Manifold::cube(30.0, 2.0, 4.0, false).translate(10.0, 20.0, 3.0));
        let setup = Setup { bed: Some([200.0, 200.0]), ..Setup::default() };
        let job = job("Plate", &plate, &setup).unwrap();
        assert!(job.warnings.iter().any(|w| w.contains("narrower than")), "{:?}", job.warnings);
        assert!(job.warnings.iter().any(|w| w.contains("larger than the machine")), "{:?}", job.warnings);
    }
}
