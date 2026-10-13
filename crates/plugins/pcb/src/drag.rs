//! Copper that follows the person's edits: track ends joined to a part's pads
//! go with it when it moves, turns or flips (a track between two pads of the
//! same part moves whole), keeping 45° corners where it can and flagging each
//! dragged track `reroute` so the person (or the autorouter) can tidy it; and
//! routes drawn by hand that change layer, with a via at each change.

use crate::footprints::PadKind;
use crate::geom::{self, P, Prim};
use crate::{Board, PcbComponent, Side, Track, Via, bad, board_of, check_layer, check_points, clean, net_at};
use de_core::{Command, CommandError, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;

fn place(at: P, rotation: f64, bottom: bool, p: P) -> P {
    let q = if bottom { [-p[0], p[1]] } else { p };
    geom::add(at, geom::rotate(q, rotation))
}

fn unplace(at: P, rotation: f64, bottom: bool, w: P) -> P {
    let q = geom::rotate(geom::sub(w, at), -rotation);
    if bottom { [-q[0], q[1]] } else { q }
}

/// The eight routing directions, unit length.
fn octilinear() -> [P; 8] {
    let d = std::f64::consts::FRAC_1_SQRT_2;
    [[1.0, 0.0], [d, d], [0.0, 1.0], [-d, d], [-1.0, 0.0], [-d, -d], [0.0, -1.0], [d, -d]]
}

fn is_octilinear(v: P) -> bool {
    let l = geom::len(v);
    l < 1e-9 || octilinear().iter().any(|u| (geom::cross(*u, v) / l).abs() < 1e-6 && geom::dot(*u, v) > 0.0)
}

/// Points from `from` to `to` in 45° steps: one corner, the leg next to `to`
/// keeping direction `keep` when it can (how the track left the pad).
pub fn dogleg(from: P, to: P, keep: Option<P>) -> Vec<P> {
    let d = geom::sub(to, from);
    if is_octilinear(d) {
        return vec![from, to];
    }
    let mut best: Option<(f64, P)> = None;
    let consider = |best: &mut Option<(f64, P)>, corner: P| {
        let (a, b) = (geom::sub(corner, from), geom::sub(to, corner));
        if is_octilinear(a) && is_octilinear(b) {
            let cost = geom::len(a) + geom::len(b);
            if best.is_none_or(|(c, _)| cost < c - 1e-9) {
                *best = Some((cost, corner));
            }
        }
    };
    // Keep the pad's exit direction: corner = to − t·keep, from→corner octilinear.
    if let Some(k) = keep.filter(|k| geom::len(*k) > 1e-9) {
        let k = [k[0] / geom::len(k), k[1] / geom::len(k)];
        for u in octilinear() {
            // from + s·u = to − t·k  →  s·u + t·k = d
            let det = geom::cross(u, k);
            if det.abs() < 1e-9 {
                continue;
            }
            let s = geom::cross(d, k) / det;
            let t = geom::cross(u, d) / det;
            if s > 1e-9 && t > 1e-9 {
                consider(&mut best, geom::add(from, [u[0] * s, u[1] * s]));
            }
        }
        if let Some((_, c)) = best {
            return vec![from, c, to];
        }
    }
    // Diagonal then straight, or straight then diagonal: the shorter-looking first.
    let (ax, ay) = (d[0].abs(), d[1].abs());
    let m = ax.min(ay);
    let diag = [m * d[0].signum(), m * d[1].signum()];
    consider(&mut best, geom::add(from, diag));
    consider(&mut best, geom::sub(to, diag));
    match best {
        Some((_, c)) => vec![from, c, to],
        None => vec![from, to],
    }
}

/// Removes repeated and collinear points.
pub fn tidy(points: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for p in points {
        if out.last().is_some_and(|q| geom::dist(*q, *p) < 1e-9) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            if geom::cross(geom::sub(b, a), geom::sub(*p, b)).abs() < 1e-9 && geom::dot(geom::sub(b, a), geom::sub(*p, b)) > 0.0 {
                out.pop();
            }
        }
        out.push(*p);
    }
    out
}

/// Moves the ends of tracks that sit on `before`'s pads to where `after` puts
/// those pads. Returns how many tracks changed.
pub fn follow(tx: &mut Tx, board: &Board, before: &PcbComponent, after: &PcbComponent) -> Result<usize, CommandError> {
    if before.at == after.at && before.rotation == after.rotation && before.side == after.side && before.footprint == after.footprint {
        return Ok(0);
    }
    let Some(fp) = board.footprint(&before.footprint) else { return Ok(0) };
    if after.footprint != before.footprint {
        return Ok(0);
    }
    let layers = board.copper_layers();
    let (b0, b1) = (before.side == Side::Bottom, after.side == Side::Bottom);
    // Old pads: copper, layers, net.
    let pads: Vec<(Prim, Vec<&str>, Option<&String>)> = fp
        .pads
        .iter()
        .filter(|p| p.kind != PadKind::Npth)
        .map(|p| {
            let on: Vec<&str> = match p.kind {
                PadKind::Tht => layers.clone(),
                _ => vec![if b0 { *layers.last().unwrap() } else { layers[0] }],
            };
            (crate::footprints::pad_prim(p, before.at, before.rotation, b0), on, before.pins.get(&p.number))
        })
        .collect();
    let moved = |w: P| place(after.at, after.rotation, b1, unplace(before.at, before.rotation, b0, w));
    let on_pad = |w: P, layer: &str, net: &Option<String>| {
        pads.iter().any(|(prim, on, pnet)| on.contains(&layer) && (net.is_none() || pnet.is_none() || net.as_ref() == *pnet) && geom::gap(prim, &Prim::circle(w, 0.0)) <= 1e-6)
    };
    let items = crate::items(tx.world(), before.board);
    let mut changed = 0;
    for (id, t) in items.tracks {
        let n = t.points.len();
        if n < 2 {
            continue;
        }
        let first = on_pad(t.points[0], &t.layer, &t.net);
        let last = on_pad(t.points[n - 1], &t.layer, &t.net);
        if !first && !last {
            continue;
        }
        let points: Vec<P> = if first && last {
            t.points.iter().map(|p| moved(*p)).collect()
        } else {
            let mut pts = t.points.clone();
            if first {
                pts.reverse();
            }
            // The end on the part moves; the leg to it bends at 45°, keeping the way it left the pad.
            let end = moved(pts[pts.len() - 1]);
            let keep = {
                let (a, b) = (pts[pts.len() - 2], pts[pts.len() - 1]);
                let d = geom::sub(b, a);
                // Turned with the part (a part turned 90° leaves its pads the other way).
                let r = geom::rotate(if b0 { [-d[0], d[1]] } else { d }, after.rotation - before.rotation);
                Some(if b1 { [-r[0], r[1]] } else { r })
            };
            let anchor = pts[pts.len() - 2];
            pts.pop();
            pts.pop();
            pts.extend(dogleg(anchor, end, keep));
            let mut pts = tidy(&pts);
            if first {
                pts.reverse();
            }
            pts
        };
        if points == t.points {
            continue;
        }
        check_points(&points, "track")?;
        tx.set(id, Track { points, reroute: true, ..t })?;
        changed += 1;
    }
    Ok(changed)
}

/// One stretch of a hand-drawn route on one layer.
#[derive(Deserialize, JsonSchema)]
pub struct Run {
    pub layer: String,
    pub points: Vec<[f64; 2]>,
}

/// `pcb.route.create`: a route drawn by hand that may change layer: one track
/// per run, and a via wherever one run ends and the next (on another layer) begins.
#[derive(Deserialize, JsonSchema)]
pub struct CreateRoute {
    pub board: StableId,
    pub net: Option<String>,
    pub width: Option<f64>,
    pub runs: Vec<Run>,
    /// Via drill and diameter; the net class's when left out.
    pub via_drill: Option<f64>,
    pub via_diameter: Option<f64>,
}

impl Command for CreateRoute {
    const NAME: &'static str = "pcb.route.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        if self.runs.is_empty() {
            return Err(bad("a route needs at least one run"));
        }
        for (i, r) in self.runs.iter().enumerate() {
            check_layer(&board, &r.layer)?;
            check_points(&r.points, "route")?;
            if r.points.is_empty() || (self.runs.len() == 1 && r.points.len() < 2) {
                return Err(bad("each run needs points (a single run needs 2 or more)"));
            }
            if i > 0 {
                let prev = &self.runs[i - 1];
                if prev.layer == r.layer {
                    return Err(bad(format!("runs {} and {i} are both on {}: a new run starts on another layer", i - 1, r.layer)));
                }
                if geom::dist(*prev.points.last().unwrap(), r.points[0]) > 1e-6 {
                    return Err(bad(format!("run {i} must start where run {} ends (the via goes there)", i - 1)));
                }
            }
        }
        let first = &self.runs[0];
        let last = self.runs.last().unwrap();
        let net = clean(self.net).or_else(|| net_at(tx.world(), self.board, &first.layer, first.points[0])).or_else(|| net_at(tx.world(), self.board, &last.layer, *last.points.last().unwrap()));
        let class = board.class_of(net.as_deref());
        let width = self.width.unwrap_or(class.track_width);
        if !(width > 0.0 && width <= 20.0) {
            return Err(bad("a track width is above 0 and up to 20 mm"));
        }
        let drill = self.via_drill.unwrap_or(class.via_drill);
        let diameter = self.via_diameter.unwrap_or(class.via_diameter);
        if !(drill > 0.0 && diameter > drill && diameter < 10.0) {
            return Err(bad("a via's diameter is larger than its drill"));
        }
        let mut tracks = Vec::new();
        let mut vias = Vec::new();
        for (i, r) in self.runs.iter().enumerate() {
            if i > 0 {
                let id = tx.create();
                tx.set(id, Via { board: self.board, net: net.clone(), at: r.points[0], drill, diameter, auto: false, spec: Default::default() })?;
                tx.add_dependency(self.board, id)?;
                vias.push(id);
            }
            let points = tidy(&r.points);
            if points.len() >= 2 {
                let id = tx.create();
                tx.set(id, Track { board: self.board, net: net.clone(), layer: r.layer.clone(), width, points, auto: false, reroute: false })?;
                tx.add_dependency(self.board, id)?;
                tracks.push(id);
            }
        }
        Ok(json!({ "tracks": tracks, "vias": vias, "net": net, "width": width }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doglegs_are_45_degrees() {
        let pts = dogleg([0.0, 0.0], [3.0, 1.0], None);
        assert_eq!(pts.len(), 3);
        for w in pts.windows(2) {
            assert!(is_octilinear(geom::sub(w[1], w[0])), "{pts:?}");
        }
        // Keeping the pad exit (arriving along +x).
        let pts = dogleg([0.0, 0.0], [5.0, 2.0], Some([1.0, 0.0]));
        let last = geom::sub(pts[2], pts[1]);
        assert!(last[1].abs() < 1e-9 && last[0] > 0.0, "{pts:?}");
        assert_eq!(dogleg([0.0, 0.0], [2.0, 2.0], None).len(), 2);
    }
}
