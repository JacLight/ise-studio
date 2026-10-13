//! Teardrops: copper fillets where a track meets a pad or via, so the joint
//! survives drilling wander and thermal stress. They are worked out with the
//! board (not stored copper): change a track and its teardrops follow.
//!
//! A teardrop spans `width_ratio` of the pad's size at the pad (at most
//! `max_width`), narrowing to the track `length_ratio` of the pad's size
//! beyond the pad's edge (at most `max_length`), with concave (curved) or
//! straight sides. One is left out where the track is nearly as wide as the
//! pad, where the track turns too soon, or where it would come closer to other
//! copper than the clearance allows.

use crate::board::{Cu, Owner};
use crate::geom::{self, P, Prim};
use crate::{bad, board_of};
use de_core::{Command, CommandError, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}
fn d_len() -> f64 {
    0.5
}
fn d_maxlen() -> f64 {
    1.0
}
fn d_wid() -> f64 {
    1.0
}
fn d_maxwid() -> f64 {
    2.0
}
fn d_ratio() -> f64 {
    0.9
}

/// Teardrop settings for a board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Teardrops {
    /// On vias.
    #[serde(default = "yes")]
    pub vias: bool,
    /// On through-hole pads.
    #[serde(default = "yes")]
    pub pads: bool,
    /// On SMD pads too.
    #[serde(default)]
    pub smd: bool,
    /// Length beyond the pad edge, as a fraction of the pad size.
    #[serde(default = "d_len")]
    pub length_ratio: f64,
    #[serde(default = "d_maxlen")]
    pub max_length: f64,
    /// Width at the pad, as a fraction of the pad size.
    #[serde(default = "d_wid")]
    pub width_ratio: f64,
    #[serde(default = "d_maxwid")]
    pub max_width: f64,
    /// Concave sides (otherwise straight).
    #[serde(default = "yes")]
    pub curved: bool,
    /// No teardrop when the track is at least this fraction of the pad size.
    #[serde(default = "d_ratio")]
    pub max_track_ratio: f64,
}

impl Default for Teardrops {
    fn default() -> Self {
        Teardrops { vias: true, pads: true, smd: false, length_ratio: 0.5, max_length: 1.0, width_ratio: 1.0, max_width: 2.0, curved: true, max_track_ratio: 0.9 }
    }
}

impl Teardrops {
    pub fn check(&self) -> Result<(), String> {
        let ok = |v: f64, lo: f64, hi: f64| v.is_finite() && v >= lo && v <= hi;
        if !(ok(self.length_ratio, 0.05, 5.0) && ok(self.width_ratio, 0.1, 2.0) && ok(self.max_length, 0.05, 20.0) && ok(self.max_width, 0.05, 20.0) && ok(self.max_track_ratio, 0.1, 1.0)) {
            return Err("teardrops: length_ratio 0.05–5, width_ratio 0.1–2, max_length and max_width 0.05–20 mm, max_track_ratio 0.1–1".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TeardropView {
    pub track: StableId,
    pub net: Option<String>,
    pub layer: String,
    /// Counter-clockwise outline.
    pub polygon: Vec<P>,
    /// "R1.2" or "via".
    pub on: String,
}

/// Distance from `c` to the edge of `prim` along unit `dir`.
fn reach(prim: &Prim, c: P, dir: P) -> f64 {
    let (mut lo, mut hi) = (0.0, 50.0);
    for _ in 0..40 {
        let m = (lo + hi) / 2.0;
        let p = [c[0] + dir[0] * m, c[1] + dir[1] * m];
        if geom::gap(prim, &Prim::circle(p, 0.0)) <= 0.0 {
            lo = m;
        } else {
            hi = m;
        }
    }
    lo
}

/// The teardrop outline for a track of width `tw` leaving pad `pad` (centre `c`,
/// size `size` across) through `end` towards `next`; None when it does not fit.
pub fn shape(pad: &Prim, c: P, size: f64, end: P, next: P, tw: f64, s: &Teardrops) -> Option<Vec<P>> {
    if tw >= s.max_track_ratio * size {
        return None;
    }
    let seg = geom::sub(next, c);
    let l = geom::len(seg);
    if l < 1e-6 {
        return None;
    }
    let dir = [seg[0] / l, seg[1] / l];
    let n = [-dir[1], dir[0]];
    let edge = reach(pad, c, dir);
    let len = (s.length_ratio * size).min(s.max_length);
    let tip_d = edge + len;
    // The track must run straight that far (from the pad centre, along the segment).
    let along_end = geom::dot(geom::sub(end, c), dir);
    if tip_d > l + along_end.min(0.0) - 1e-9 || len < 0.02 {
        return None;
    }
    let w = (s.width_ratio * size).min(s.max_width).max(tw);
    let at = |d: f64, o: f64| [c[0] + dir[0] * d + n[0] * o, c[1] + dir[1] * d + n[1] * o];
    // From the base (inside the pad, at its widest), along one side to the tip, back the other.
    let base = (edge - w.min(size) / 2.0).max(0.0) * 0.5;
    let side = |sign: f64| -> Vec<P> {
        let b = at(base, sign * w / 2.0);
        let t = at(tip_d, sign * tw / 2.0);
        if !s.curved {
            return vec![b, t];
        }
        // A quadratic curve bowed in towards the track.
        let ctrl = at(edge + len * 0.15, sign * tw / 2.0);
        (0..=12).map(|i| {
            let k = i as f64 / 12.0;
            let u = 1.0 - k;
            [u * u * b[0] + 2.0 * u * k * ctrl[0] + k * k * t[0], u * u * b[1] + 2.0 * u * k * ctrl[1] + k * k * t[1]]
        }).collect()
    };
    let mut poly = side(-1.0);
    let mut other = side(1.0);
    other.reverse();
    poly.extend(other);
    if geom::polygon_area(&poly) < 0.0 {
        poly.reverse();
    }
    Some(poly)
}

/// Teardrops for every track end on a pad or via of its net, as views and as
/// copper (convex pieces, owned by the track so they join it).
pub fn make(s: &Teardrops, view_layers: &[String], tracks: &[(StableId, crate::Track, Vec<P>)], cu: &[Cu], pads: &[crate::board::PadView], board: &crate::Board) -> (Vec<TeardropView>, Vec<Cu>) {
    let mut views = Vec::new();
    let mut copper = Vec::new();
    for (tid, t, line) in tracks {
        let Some(li) = crate::board::layer_index(view_layers, &t.layer) else { continue };
        if line.len() < 2 || t.net.is_none() {
            continue;
        }
        for (end, next) in [(line[0], line[1]), (line[line.len() - 1], line[line.len() - 2])] {
            // What the end sits on: a pad or a via of the track's net on its layer.
            let probe = Prim::circle(end, 0.0);
            let host = cu.iter().find(|c| {
                c.layers & (1 << li) != 0
                    && c.net == t.net
                    && match &c.owner {
                        Owner::Pad(i) => {
                            let p = &pads[*i];
                            (s.pads && p.kind == crate::footprints::PadKind::Tht || s.smd && p.kind == crate::footprints::PadKind::Smd) && geom::gap(&c.prim, &probe) <= 1e-6
                        }
                        Owner::Via(_) => s.vias && geom::gap(&c.prim, &probe) <= 1e-6,
                        _ => false,
                    }
            });
            let Some(host) = host else { continue };
            let (c, size, on) = match &host.owner {
                Owner::Pad(i) => (pads[*i].at, pads[*i].size[0].min(pads[*i].size[1]), format!("{}.{}", pads[*i].reference, pads[*i].number)),
                _ => (host.prim.center(), 2.0 * host.prim.r, "via".to_string()),
            };
            let Some(poly) = shape(&host.prim, c, size, end, next, t.width, s) else { continue };
            let pieces = crate::graphics::triangles(&[poly.clone()]);
            // Kept only where it stays clear of other nets' copper.
            let gap = board.class_of(t.net.as_deref()).clearance.max(board.rules.min_space);
            let clash = cu.iter().any(|o| o.layers & (1 << li) != 0 && o.net != t.net && pieces.iter().any(|p| geom::bbox_overlap(&p.bbox(), &o.bbox, gap) && geom::gap(p, &o.prim) < gap - 1e-6));
            if clash {
                continue;
            }
            for p in pieces {
                copper.push(Cu { bbox: p.bbox(), prim: p, layers: 1 << li, net: t.net.clone(), owner: Owner::Track(*tid, usize::MAX) });
            }
            views.push(TeardropView { track: *tid, net: t.net.clone(), layer: t.layer.clone(), polygon: poly, on });
        }
    }
    (views, copper)
}

/// `pcb.teardrops.set`: teardrops on a board — `enabled` false removes them;
/// the rest are the settings (what is left out keeps its value or default).
#[derive(Deserialize, JsonSchema)]
pub struct SetTeardrops {
    pub board: StableId,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub vias: Option<bool>,
    pub pads: Option<bool>,
    pub smd: Option<bool>,
    pub length_ratio: Option<f64>,
    pub max_length: Option<f64>,
    pub width_ratio: Option<f64>,
    pub max_width: Option<f64>,
    pub curved: Option<bool>,
    pub max_track_ratio: Option<f64>,
}

impl Command for SetTeardrops {
    const NAME: &'static str = "pcb.teardrops.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        if !self.enabled {
            b.teardrops = None;
        } else {
            let mut s = b.teardrops.clone().unwrap_or_default();
            if let Some(v) = self.vias {
                s.vias = v;
            }
            if let Some(v) = self.pads {
                s.pads = v;
            }
            if let Some(v) = self.smd {
                s.smd = v;
            }
            if let Some(v) = self.length_ratio {
                s.length_ratio = v;
            }
            if let Some(v) = self.max_length {
                s.max_length = v;
            }
            if let Some(v) = self.width_ratio {
                s.width_ratio = v;
            }
            if let Some(v) = self.max_width {
                s.max_width = v;
            }
            if let Some(v) = self.curved {
                s.curved = v;
            }
            if let Some(v) = self.max_track_ratio {
                s.max_track_ratio = v;
            }
            s.check().map_err(bad)?;
            b.teardrops = Some(s);
        }
        let t = b.teardrops.clone();
        tx.set(self.board, b)?;
        Ok(json!({ "board": self.board, "teardrops": t }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<SetTeardrops>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_via_teardrop_narrows_to_the_track() {
        let via = Prim::circle([0.0, 0.0], 0.4);
        let s = Teardrops::default();
        let poly = shape(&via, [0.0, 0.0], 0.8, [0.0, 0.0], [5.0, 0.0], 0.2, &s).expect("fits");
        let xs: Vec<f64> = poly.iter().map(|p| p[0]).collect();
        let tip = xs.iter().cloned().fold(f64::MIN, f64::max);
        // Pad edge 0.4 + 0.5 × 0.8 = 0.8.
        assert!((tip - 0.8).abs() < 1e-3, "tip at {tip}");
        let widest = poly.iter().map(|p| p[1].abs()).fold(0.0, f64::max);
        assert!((widest - 0.4).abs() < 1e-6, "full pad width at the base: {widest}");
        let at_tip = poly.iter().filter(|p| (p[0] - tip).abs() < 1e-6).map(|p| p[1].abs()).fold(0.0, f64::max);
        assert!((at_tip - 0.1).abs() < 1e-6, "track width at the tip");
        assert!(geom::polygon_area(&poly) > 0.0);
    }

    #[test]
    fn no_teardrop_for_a_wide_track_or_a_short_stub() {
        let via = Prim::circle([0.0, 0.0], 0.4);
        let s = Teardrops::default();
        assert!(shape(&via, [0.0, 0.0], 0.8, [0.0, 0.0], [5.0, 0.0], 0.75, &s).is_none(), "track nearly as wide as the via");
        assert!(shape(&via, [0.0, 0.0], 0.8, [0.0, 0.0], [0.5, 0.0], 0.2, &s).is_none(), "the track turns before the tip");
    }

    #[test]
    fn max_length_caps_a_big_pad() {
        let pad = Prim::circle([0.0, 0.0], 2.0);
        let s = Teardrops { curved: false, ..Teardrops::default() };
        let poly = shape(&pad, [0.0, 0.0], 4.0, [0.0, 0.0], [0.0, 10.0], 0.3, &s).unwrap();
        let tip = poly.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
        assert!((tip - 3.0).abs() < 1e-3, "edge 2 + max 1: {tip}");
        let widest = poly.iter().map(|p| p[0].abs()).fold(0.0, f64::max);
        assert!((widest - 1.0).abs() < 1e-6, "max width 2 mm");
    }
}
