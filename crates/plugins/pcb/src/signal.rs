//! Routing constraints: impedance-controlled net classes, differential pairs,
//! lengths and skew.
//!
//! A net class may ask for a single-ended impedance (`impedance`), a
//! differential one for its pairs (`diff_impedance`, with an optional fixed
//! `diff_gap`), a tolerance, the longest uncoupled stretch of a pair and the
//! largest skew. The width (and gap) that meets the target on each copper
//! layer is worked out from the stackup: outer layers as microstrip over the
//! next layer (Hammerstad–Jensen with thickness correction), inner layers as
//! stripline between their neighbours (Cohn's offset stripline, Wheeler's
//! thickness correction), pairs edge-coupled (Cohn's exact odd mode for
//! stripline, IPC-2141's coupling term for microstrip). Neighbouring copper
//! layers are taken as the reference planes.
//!
//! Differential pairs are found from net names (`D+`/`D-`, `_P`/`_N`,
//! `DP`/`DM`, `P`/`N`) or set explicitly (`pcb.pair.set`). Length-matching
//! groups (`pcb.length.group`) hold nets (or pairs) to a target length, to
//! the longest member, or within a skew. A net's length is its copper: track
//! (arcs exact), every via's barrel between the layers its tracks use
//! (stackup-aware) and any pad-to-die length given for its pads.
//!
//! The checks (`check`, run by DRC) report `impedance`, `diff_pair_gap`,
//! `diff_pair_uncoupled`, `length`, `skew` and `via_count`, from the net
//! classes, pairs and groups and from the custom rules' constraints of the
//! same names (plus Iṣẹ́ Studio's `impedance` constraint, in Ω).

use crate::board::{BoardView, TrackView, Violation};
use crate::calc;
use crate::geom::{self, P};
use crate::pns;
use crate::rules::{self, Item};
use crate::stackup::{OZ_MM, StackupView};
use crate::{Board, NetClass, bad, board_of};
use de_core::{Command, CommandError, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

// ---- Stored settings --------------------------------------------------------

/// A differential pair: two nets routed together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiffPair {
    pub name: String,
    /// The positive net.
    pub p: String,
    /// The negative net.
    pub n: String,
    /// Largest length difference between the two tracks (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_skew: Option<f64>,
    /// Longest stretch the two may run apart (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uncoupled: Option<f64>,
}

/// Nets held to one length.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LengthGroup {
    pub name: String,
    /// Net names, patterns (`D?`, `DDR_DQ*`) or pair names (both of its nets).
    pub nets: Vec<String>,
    /// Every member this long (mm), within `tolerance`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<f64>,
    /// ± around the target (mm); without a target, how far below the longest member each may be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Longest minus shortest member (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_skew: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Signal {
    /// Pairs set by hand (besides the ones found from net names).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<DiffPair>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<LengthGroup>,
    /// Bond-wire / package length inside a part, by pad ("U1.5" → mm), counted into its net's length.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub pad_to_die: BTreeMap<String, f64>,
    /// Find pairs from net names (default on).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detect_pairs: Option<bool>,
}

fn pos(v: Option<f64>, what: &str, max: f64) -> Result<(), String> {
    match v {
        Some(x) if !(x.is_finite() && x > 0.0 && x <= max) => Err(format!("{what} is above 0 and at most {max}")),
        _ => Ok(()),
    }
}

impl Signal {
    pub fn is_empty(&self) -> bool {
        *self == Signal::default()
    }

    pub fn check(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for p in &self.pairs {
            if p.name.trim().is_empty() || p.p.trim().is_empty() || p.n.trim().is_empty() {
                return Err("a pair needs a name and two nets".into());
            }
            if p.p == p.n {
                return Err(format!("pair {}: its two nets must differ", p.name));
            }
            if !names.insert(p.name.to_lowercase()) {
                return Err(format!("there are two pairs named {}", p.name));
            }
            pos(p.max_skew, "max_skew (mm)", 1000.0)?;
            pos(p.max_uncoupled, "max_uncoupled (mm)", 10000.0)?;
        }
        let mut names = BTreeSet::new();
        for g in &self.groups {
            if g.name.trim().is_empty() || g.nets.is_empty() {
                return Err("a length group needs a name and nets".into());
            }
            if !names.insert(g.name.to_lowercase()) {
                return Err(format!("there are two length groups named {}", g.name));
            }
            for (v, w) in [(g.target, "target"), (g.tolerance, "tolerance"), (g.min, "min"), (g.max, "max"), (g.max_skew, "max_skew")] {
                pos(v, &format!("group {}: {w} (mm)", g.name), 10000.0)?;
            }
        }
        for (k, v) in &self.pad_to_die {
            if !(v.is_finite() && (0.0..=200.0).contains(v)) {
                return Err(format!("pad-to-die length of {k} is 0 to 200 mm"));
            }
        }
        Ok(())
    }
}

pub fn check_class(c: &NetClass) -> Result<(), String> {
    let pre = |w: &str| format!("net class {}: {w}", c.name);
    pos(c.impedance, &pre("impedance (Ω)"), 500.0)?;
    pos(c.diff_impedance, &pre("diff_impedance (Ω)"), 1000.0)?;
    pos(c.diff_gap, &pre("diff_gap (mm)"), 10.0)?;
    pos(c.impedance_tolerance, &pre("impedance_tolerance (%)"), 50.0)?;
    pos(c.max_uncoupled, &pre("max_uncoupled (mm)"), 10000.0)?;
    pos(c.max_skew, &pre("max_skew (mm)"), 1000.0)?;
    pos(c.pair_via_pitch, &pre("pair_via_pitch (mm)"), 20.0)?;
    Ok(())
}

// ---- Pairs from names ----------------------------------------------------------

const SUFFIXES: [(&str, &str); 5] = [("+", "-"), ("_P", "_N"), ("_DP", "_DM"), ("DP", "DM"), ("P", "N")];

/// The pair `name` is a member of, when its partner is among `nets`: (pair name, p, n).
pub fn split_pair(name: &str, nets: &BTreeSet<String>) -> Option<(String, String, String)> {
    for (sp, sn) in SUFFIXES {
        for (mine, other, positive) in [(sp, sn, true), (sn, sp, false)] {
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(&mine.to_ascii_lowercase()) || name.len() <= mine.len() {
                continue;
            }
            let base = &name[..name.len() - mine.len()];
            let found = &name[name.len() - mine.len()..];
            let other = if found.chars().any(|c| c.is_ascii_lowercase()) { other.to_ascii_lowercase() } else { other.to_string() };
            let partner = format!("{base}{other}");
            if !nets.contains(&partner) {
                continue;
            }
            let pair = base.trim_end_matches(['_', '-', '/', '.']);
            let pair = if pair.is_empty() { base } else { pair };
            let (p, n) = if positive { (name.to_string(), partner) } else { (partner, name.to_string()) };
            return Some((pair.to_string(), p, n));
        }
    }
    None
}

/// Pairs found from the net names.
pub fn detect(nets: &BTreeSet<String>) -> Vec<DiffPair> {
    let mut out: Vec<DiffPair> = Vec::new();
    let mut used = BTreeSet::new();
    for n in nets {
        if used.contains(n) {
            continue;
        }
        if let Some((name, p, nn)) = split_pair(n, nets) {
            if used.contains(&p) || used.contains(&nn) {
                continue;
            }
            used.insert(p.clone());
            used.insert(nn.clone());
            out.push(DiffPair { name, p, n: nn, max_skew: None, max_uncoupled: None });
        }
    }
    out
}

/// Every pair on the board: the ones set by hand, then the ones found (that do not clash with them).
pub fn pairs(board: &Board, nets: &BTreeSet<String>) -> Vec<(DiffPair, bool)> {
    let mut out: Vec<(DiffPair, bool)> = board.signal.pairs.iter().map(|p| (p.clone(), false)).collect();
    if board.signal.detect_pairs.unwrap_or(true) {
        for d in detect(nets) {
            if !out.iter().any(|(p, _)| p.p == d.p || p.n == d.p || p.p == d.n || p.n == d.n || p.name.eq_ignore_ascii_case(&d.name)) {
                out.push((d, true));
            }
        }
    }
    out
}

// ---- Lines on the stackup ---------------------------------------------------------

/// A track's transmission line on one layer of the stackup.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum Line {
    /// Outer layer over the plane `h` below (mm).
    Microstrip { h: f64, er: f64, t: f64 },
    /// Inner layer, `h1` to the plane above and `h2` to the one below.
    Stripline { h1: f64, er1: f64, h2: f64, er2: f64, t: f64 },
}

/// Light in vacuum, mm per ps.
const C_MM_PS: f64 = 0.299_792_458;

pub fn line_on(sv: &StackupView, k: usize) -> Line {
    let n = sv.copper_oz.len().max(2);
    let t = sv.copper_oz.get(k).copied().unwrap_or(1.0) * OZ_MM;
    let er = |i: usize| sv.dielectrics.get(i).and_then(|d| d.epsilon_r).unwrap_or(4.5);
    let h = |i: usize| sv.dielectrics.get(i).map(|d| d.thickness).unwrap_or(0.2);
    if k == 0 {
        Line::Microstrip { h: h(0), er: er(0), t }
    } else if k + 1 >= n {
        Line::Microstrip { h: h(n - 2), er: er(n - 2), t }
    } else {
        Line::Stripline { h1: h(k - 1), er1: er(k - 1), h2: h(k), er2: er(k), t }
    }
}

impl Line {
    pub fn z0(&self, w: f64) -> f64 {
        match *self {
            Line::Microstrip { h, er, t } => calc::microstrip(w, h, t, er).0,
            Line::Stripline { h1, er1, h2, er2, t } => calc::stripline_offset(w, h1, er1, h2, er2, t),
        }
    }
    pub fn zdiff(&self, w: f64, s: f64) -> f64 {
        match *self {
            Line::Microstrip { h, er, t } => calc::zdiff_microstrip(w, s, h, t, er),
            Line::Stripline { h1, er1, h2, er2, t } => calc::zdiff_stripline(w, s, h1, er1, h2, er2, t),
        }
    }
    pub fn eeff(&self, w: f64) -> f64 {
        match *self {
            Line::Microstrip { h, er, t } => calc::microstrip(w, h, t, er).1,
            Line::Stripline { h1, er1, h2, er2, .. } => (h1 * er1 + h2 * er2) / (h1 + h2),
        }
    }
    /// Propagation delay, ps per mm.
    pub fn delay(&self, w: f64) -> f64 {
        self.eeff(w).sqrt() / C_MM_PS
    }
    pub fn width_for(&self, z: f64) -> Option<f64> {
        calc::solve(&|w| self.z0(w), z, 0.01, 10.0)
    }
    pub fn width_for_diff(&self, zd: f64, s: f64) -> Option<f64> {
        calc::solve(&|w| self.zdiff(w, s), zd, 0.01, 10.0)
    }
    pub fn gap_for_diff(&self, zd: f64, w: f64) -> Option<f64> {
        calc::solve(&|s| self.zdiff(w, s), zd, 0.01, 10.0)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Line::Microstrip { .. } => "microstrip",
            Line::Stripline { .. } => "stripline",
        }
    }
}

/// What a class's targets come to on one layer.
#[derive(Clone, Debug, Serialize)]
pub struct LayerTarget {
    pub layer: String,
    pub line: Line,
    /// Track width that meets the target (mm, rounded to 1 µm).
    pub width: f64,
    /// Single-ended impedance at that width (Ω).
    pub z0: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zdiff: Option<f64>,
    /// ps per mm.
    pub delay: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

fn um(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// The width (and pair gap) that meets the class's impedance targets on each copper layer.
pub fn targets(board: &Board, sv: &StackupView, class: &NetClass) -> Vec<LayerTarget> {
    let names = board.copper_layers();
    names
        .iter()
        .enumerate()
        .map(|(k, layer)| {
            let line = line_on(sv, k);
            let mut notes = Vec::new();
            let mut w = class.track_width;
            if let Some(z) = class.impedance {
                match line.width_for(z) {
                    Some(x) => w = um(x),
                    None => notes.push(format!("{z} Ω is out of reach on {layer} (widths 0.01–10 mm)")),
                }
            }
            let mut gap = None;
            if let Some(zt) = class.diff_impedance {
                if class.impedance.is_some() {
                    match class.diff_gap.or_else(|| line.gap_for_diff(zt, w).map(um)) {
                        Some(s) => gap = Some(s),
                        None => notes.push(format!("{zt} Ω differential is out of reach at {w} mm wide on {layer}")),
                    }
                } else if let Some(s) = class.diff_gap {
                    match line.width_for_diff(zt, s) {
                        Some(x) => {
                            w = um(x);
                            gap = Some(s);
                        }
                        None => notes.push(format!("{zt} Ω differential is out of reach with a {s} mm gap on {layer}")),
                    }
                } else {
                    // Gap about the width (the usual loosely coupled pair), never below the board's minimum space.
                    let floor = board.rules.min_space.max(0.05);
                    let mut s = floor.max(0.1);
                    let mut found = None;
                    for _ in 0..40 {
                        let Some(x) = line.width_for_diff(zt, s) else { break };
                        found = Some(x);
                        let next = x.max(floor);
                        if (next - s).abs() < 1e-7 {
                            break;
                        }
                        s = next;
                    }
                    match found {
                        Some(x) => {
                            w = um(x);
                            gap = Some(um(s));
                        }
                        None => notes.push(format!("{zt} Ω differential is out of reach on {layer}")),
                    }
                }
            }
            if w + 1e-9 < board.rules.min_track {
                notes.push(format!("{w} mm is below the board's smallest track ({} mm): change the stackup", board.rules.min_track));
            }
            if gap.is_some_and(|s| s + 1e-9 < board.rules.min_space) {
                notes.push(format!("the {} mm gap is below the board's smallest space ({} mm)", gap.unwrap(), board.rules.min_space));
            }
            LayerTarget { layer: layer.to_string(), line, width: w, z0: line.z0(w), gap, zdiff: gap.map(|s| line.zdiff(w, s)), delay: line.delay(w), notes }
        })
        .collect()
}

fn has_targets(c: &NetClass) -> bool {
    c.impedance.is_some() || c.diff_impedance.is_some()
}

/// The pair a net is in.
pub fn pair_of<'a>(v: &'a BoardView, net: &str) -> Option<&'a PairView> {
    v.signal.pairs.iter().find(|p| p.p == net || p.n == net)
}

fn controlled(class: &NetClass, in_pair: bool) -> bool {
    class.impedance.is_some() || (in_pair && class.diff_impedance.is_some())
}

/// The layer target that governs a net on `layer`, when its class has one.
pub fn target_on<'a>(board: &Board, v: &'a BoardView, net: Option<&str>, layer: &str) -> Option<&'a LayerTarget> {
    let class = board.class_of(net);
    let in_pair = net.is_some_and(|n| pair_of(v, n).is_some());
    if !controlled(&class, in_pair) {
        return None;
    }
    v.signal.classes.iter().find(|c| c.name == class.name)?.layers.iter().find(|l| l.layer == layer)
}

/// The track width for a net on a layer: the impedance target's when its class has one, else the class width.
pub fn width_on(board: &Board, v: &BoardView, net: Option<&str>, layer: &str) -> f64 {
    target_on(board, v, net, layer).map(|t| t.width).unwrap_or_else(|| board.class_of(net).track_width)
}

/// The gap between a pair's tracks on a layer.
pub fn gap_on(board: &Board, v: &BoardView, net: &str, layer: &str) -> Option<f64> {
    target_on(board, v, Some(net), layer).and_then(|t| t.gap).or(board.class_of(Some(net)).diff_gap)
}

// ---- Lengths -----------------------------------------------------------------------

/// Length of one segment, an arc when it has a bulge.
pub fn seg_length(a: P, b: P, bulge: f64) -> f64 {
    if bulge.abs() < 1e-12 || geom::dist(a, b) < 1e-12 {
        return geom::dist(a, b);
    }
    let (_, r, _, sweep) = pns::arc_of(a, b, bulge);
    r * sweep.abs()
}

pub fn path_length(points: &[P], bulges: &[f64]) -> f64 {
    points.windows(2).enumerate().map(|(i, w)| seg_length(w[0], w[1], bulges.get(i).copied().unwrap_or(0.0))).sum()
}

/// A track's corners and bulges (its own, not the flattened line).
pub fn geometry(t: &TrackView) -> (&[P], &[f64]) {
    if t.vertices.is_empty() { (&t.points, &[]) } else { (&t.vertices, &t.bulges) }
}

pub fn track_length(t: &TrackView) -> f64 {
    let (p, b) = geometry(t);
    path_length(p, b)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NetLength {
    pub net: String,
    /// Track + vias + pad-to-die (mm).
    pub length: f64,
    pub tracks: f64,
    pub vias: usize,
    pub via_length: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub pad_to_die: f64,
    /// Propagation delay along it (ps).
    pub delay: f64,
    /// Every pad joined.
    pub routed: bool,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// Barrel length of a via between the outermost layers its tracks use (centre of copper to centre).
pub fn via_length(sv: &StackupView, lo: usize, hi: usize) -> f64 {
    if hi <= lo {
        return 0.0;
    }
    let cu = |i: usize| sv.copper_oz.get(i).copied().unwrap_or(1.0) * OZ_MM;
    sv.depth(lo, hi) - (cu(lo) + cu(hi)) / 2.0
}

fn mean_er(sv: &StackupView, lo: usize, hi: usize) -> f64 {
    let ds = sv.dielectrics.get(lo..hi).unwrap_or(&[]);
    let t: f64 = ds.iter().map(|d| d.thickness).sum();
    if t <= 0.0 { 4.5 } else { ds.iter().map(|d| d.thickness * d.epsilon_r.unwrap_or(4.5)).sum::<f64>() / t }
}

/// The copper length of every net with tracks or vias.
pub fn net_lengths(board: &Board, v: &BoardView) -> Vec<NetLength> {
    let sv = &v.stackup;
    let mut out: BTreeMap<String, NetLength> = BTreeMap::new();
    for t in &v.tracks {
        let Some(net) = &t.net else { continue };
        let e = out.entry(net.clone()).or_insert_with(|| NetLength { net: net.clone(), ..Default::default() });
        let l = track_length(t);
        e.tracks += l;
        if let Some(k) = crate::board::layer_index(&v.layers, &t.layer) {
            e.delay += l * line_on(sv, k).delay(t.width);
        }
    }
    for via in &v.vias {
        let Some(net) = &via.net else { continue };
        let reach = via.diameter / 2.0 + 1e-3;
        let ks: BTreeSet<usize> = v
            .tracks
            .iter()
            .filter(|t| t.net.as_ref() == Some(net))
            .filter(|t| {
                let (p, _) = geometry(t);
                p.first().is_some_and(|a| geom::dist(*a, via.at) <= reach) || p.last().is_some_and(|a| geom::dist(*a, via.at) <= reach)
            })
            .filter_map(|t| crate::board::layer_index(&v.layers, &t.layer))
            .collect();
        let e = out.entry(net.clone()).or_insert_with(|| NetLength { net: net.clone(), ..Default::default() });
        e.vias += 1;
        if let (Some(lo), Some(hi)) = (ks.first(), ks.last()) {
            let l = via_length(sv, *lo, *hi);
            e.via_length += l;
            e.delay += l * mean_er(sv, *lo, *hi).sqrt() / C_MM_PS;
        }
    }
    for p in &v.pads {
        let Some(net) = &p.net else { continue };
        if let Some(d) = board.signal.pad_to_die.get(&format!("{}.{}", p.reference, p.number)) {
            let e = out.entry(net.clone()).or_insert_with(|| NetLength { net: net.clone(), ..Default::default() });
            e.pad_to_die += d;
            e.delay += d * 4.5f64.sqrt() / C_MM_PS;
        }
    }
    let routed: BTreeMap<&str, bool> = v.nets.iter().map(|n| (n.name.as_str(), n.routed)).collect();
    out.into_values()
        .map(|mut e| {
            e.length = e.tracks + e.via_length + e.pad_to_die;
            e.routed = routed.get(e.net.as_str()).copied().unwrap_or(false);
            for x in [&mut e.length, &mut e.tracks, &mut e.via_length, &mut e.pad_to_die] {
                *x = (*x * 1e6).round() / 1e6;
            }
            e.delay = (e.delay * 1000.0).round() / 1000.0;
            e
        })
        .collect()
}

/// A net's length with new runs (and the vias between them) added, and what it is tuned to:
/// its length group's target, else its pair partner's length.
pub fn readout(board: &Board, v: &BoardView, net: &str, runs: &[crate::routing::PlanRun], vias: &[P]) -> (f64, Option<f64>) {
    let _ = board;
    let base = v.signal.lengths.iter().find(|l| l.net == net).map(|l| l.length).unwrap_or(0.0);
    let mut len = base + runs.iter().map(|r| path_length(&r.points, &r.bulges)).sum::<f64>();
    for (i, w) in runs.windows(2).enumerate() {
        if i < vias.len() {
            let (a, b) = (crate::board::layer_index(&v.layers, &w[0].layer), crate::board::layer_index(&v.layers, &w[1].layer));
            if let (Some(a), Some(b)) = (a, b) {
                len += via_length(&v.stackup, a.min(b), a.max(b));
            }
        }
    }
    let target = v.signal.groups.iter().find(|g| g.members.iter().any(|m| m.net == net)).map(|g| g.target).or_else(|| {
        let p = pair_of(v, net)?;
        let other = if p.p == net { &p.n } else { &p.p };
        v.signal.lengths.iter().find(|l| &l.net == other).map(|l| l.length).filter(|l| *l > 0.0)
    });
    ((len * 1e4).round() / 1e4, target)
}

// ---- Coupling ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Shape {
    Line { a: P, b: P },
    Arc { c: P, r: f64, lo: f64, hi: f64 },
}

#[derive(Clone, Debug)]
struct Piece {
    layer: String,
    w: f64,
    s: Shape,
    len: f64,
}

fn pieces(tracks: &[&TrackView]) -> Vec<Piece> {
    let mut out = Vec::new();
    for t in tracks {
        let (pts, bulges) = geometry(t);
        for (i, w) in pts.windows(2).enumerate() {
            let b = bulges.get(i).copied().unwrap_or(0.0);
            let len = seg_length(w[0], w[1], b);
            if len < 1e-9 {
                continue;
            }
            let s = if b.abs() < 1e-12 {
                Shape::Line { a: w[0], b: w[1] }
            } else {
                let (c, r, a0, sweep) = pns::arc_of(w[0], w[1], b);
                Shape::Arc { c, r, lo: a0.min(a0 + sweep), hi: a0.max(a0 + sweep) }
            };
            out.push(Piece { layer: t.layer.clone(), w: t.width, s, len });
        }
    }
    out
}

/// Where piece `b` runs alongside piece `a`: the stretch of `a` (from, to, in mm along it) and the edge gap.
fn alongside(a: &Piece, b: &Piece, max_gap: f64) -> Option<(f64, f64, f64)> {
    if a.layer != b.layer {
        return None;
    }
    let half = (a.w + b.w) / 2.0;
    match (a.s, b.s) {
        (Shape::Line { a: a0, b: a1 }, Shape::Line { a: b0, b: b1 }) => {
            let u = geom::sub(a1, a0);
            let l = geom::len(u);
            let u = [u[0] / l, u[1] / l];
            let v = geom::sub(b1, b0);
            let lv = geom::len(v);
            if geom::cross(u, [v[0] / lv, v[1] / lv]).abs() > 0.03 {
                return None;
            }
            let d = geom::cross(u, geom::sub(b0, a0)).abs().min(geom::cross(u, geom::sub(b1, a0)).abs());
            let gap = d - half;
            if gap > max_gap || gap < -1e-6 {
                return None;
            }
            let (t0, t1) = (geom::dot(geom::sub(b0, a0), u), geom::dot(geom::sub(b1, a0), u));
            let (lo, hi) = (t0.min(t1).max(0.0), t0.max(t1).min(l));
            (hi - lo > 1e-6).then_some((lo, hi, gap))
        }
        (Shape::Arc { c: c1, r: r1, lo: l1, hi: h1 }, Shape::Arc { c: c2, r: r2, lo: l2, hi: h2 }) => {
            if geom::dist(c1, c2) > 0.01 {
                return None;
            }
            let gap = (r1 - r2).abs() - half;
            if gap > max_gap || gap < -1e-6 {
                return None;
            }
            let mut best = (0.0, 0.0, 0.0);
            for k in -1..=1 {
                let sh = 2.0 * PI * k as f64;
                let (lo, hi) = (l1.max(l2 + sh), h1.min(h2 + sh));
                if hi - lo > best.1 - best.0 {
                    best = (lo, hi, gap);
                }
            }
            (best.1 - best.0 > 1e-9).then(|| ((best.0 - l1) * r1, (best.1 - l1) * r1, gap))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
struct Coupled {
    /// Length of `a` running alongside `b`.
    length: f64,
    /// (edge gap, length, layer, width of a).
    runs: Vec<(f64, f64, String, f64)>,
}

fn coupled(a: &[Piece], b: &[Piece], max_gap: &dyn Fn(&str) -> f64) -> Coupled {
    let mut out = Coupled::default();
    for pa in a {
        let mut spans: Vec<(f64, f64, f64)> = b.iter().filter_map(|pb| alongside(pa, pb, max_gap(&pa.layer))).collect();
        spans.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut cur: Option<(f64, f64)> = None;
        for (lo, hi, gap) in &spans {
            out.runs.push((*gap, hi - lo, pa.layer.clone(), pa.w));
            cur = match cur {
                Some((s, e)) if *lo <= e => Some((s, e.max(*hi))),
                Some((s, e)) => {
                    out.length += e - s;
                    Some((*lo, *hi))
                }
                None => Some((*lo, *hi)),
            };
        }
        if let Some((s, e)) = cur {
            out.length += (e - s).min(pa.len);
        }
    }
    out
}

// ---- The view ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct ClassTargets {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impedance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_impedance: Option<f64>,
    /// %.
    pub tolerance: f64,
    pub layers: Vec<LayerTarget>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PairView {
    pub name: String,
    pub p: String,
    pub n: String,
    /// Found from the net names (not set by hand).
    pub detected: bool,
    pub class: String,
    pub length_p: f64,
    pub length_n: f64,
    pub skew: f64,
    /// Length the two run side by side (on the positive net).
    pub coupled: f64,
    /// The longer of the two tracks' stretches apart.
    pub uncoupled: f64,
    /// The gap the class asks for (on the top layer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_max: Option<f64>,
    /// The coupled gap per layer, against that layer's own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<LayerGap>,
    /// Differential impedance of the coupled stretches as routed (length-weighted, Ω).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zdiff: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_skew: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uncoupled: Option<f64>,
    pub routed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LayerGap {
    pub layer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nominal: Option<f64>,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemberLength {
    pub net: String,
    pub length: f64,
    pub routed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupView {
    pub name: String,
    pub members: Vec<MemberLength>,
    /// What each member is tuned to: the target, else the longest member.
    pub target: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_skew: Option<f64>,
    pub skew: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SignalView {
    /// Classes with impedance targets, worked out per layer.
    pub classes: Vec<ClassTargets>,
    pub pairs: Vec<PairView>,
    pub lengths: Vec<NetLength>,
    pub groups: Vec<GroupView>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub pad_to_die: BTreeMap<String, f64>,
    pub detect_pairs: bool,
    /// Every net class as set (the editor's).
    pub net_classes: Vec<NetClass>,
}

/// The nets of a group: names, patterns and pair names expanded.
pub fn members(g: &LengthGroup, nets: &[String], pairs: &[PairView]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in &g.nets {
        if let Some(p) = pairs.iter().find(|p| p.name.eq_ignore_ascii_case(e)) {
            for n in [&p.p, &p.n] {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
            continue;
        }
        for n in nets.iter().filter(|n| *n == e || rules::wild(e, n)) {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
    }
    out
}

fn tol_of(c: &NetClass) -> f64 {
    c.impedance_tolerance.unwrap_or(10.0)
}

/// Lengths, pairs, groups and class targets for the board as it stands.
pub fn view(board: &Board, v: &BoardView) -> SignalView {
    let sv = &v.stackup;
    let classes: Vec<ClassTargets> = board
        .net_classes
        .iter()
        .filter(|c| has_targets(c))
        .map(|c| ClassTargets { name: c.name.clone(), impedance: c.impedance, diff_impedance: c.diff_impedance, tolerance: tol_of(c), layers: targets(board, sv, c) })
        .collect();
    let lengths = net_lengths(board, v);
    let len_of = |n: &str| lengths.iter().find(|l| l.net == n).map(|l| l.length).unwrap_or(0.0);
    let routed_of = |n: &str| v.nets.iter().find(|x| x.name == n).is_some_and(|x| x.routed && x.pins.len() > 1);
    let names: BTreeSet<String> = v.nets.iter().map(|n| n.name.clone()).collect();
    let mut pv = Vec::new();
    for (p, detected) in pairs(board, &names) {
        let class = board.class_of(Some(&p.p));
        let ct = classes.iter().find(|c| c.name == class.name);
        let top = ct.and_then(|c| c.layers.first());
        let nominal = top.and_then(|t| t.gap).or(class.diff_gap);
        let nominal_on = |layer: &str| ct.and_then(|c| c.layers.iter().find(|l| l.layer == layer)).and_then(|l| l.gap).or(class.diff_gap);
        let width_on_layer = |layer: &str| ct.and_then(|c| c.layers.iter().find(|l| l.layer == layer)).map(|l| l.width).unwrap_or(class.track_width);
        // Side by side within a quarter of the layer's gap (plus 20 µm); further apart is uncoupled (skew bumps, via jogs).
        let max_gap = |layer: &str| match nominal_on(layer) {
            Some(g) => g * 1.25 + 0.02,
            None => (2.0 * width_on_layer(layer)).max(0.5),
        };
        let tp: Vec<&TrackView> = v.tracks.iter().filter(|t| t.net.as_deref() == Some(p.p.as_str())).collect();
        let tn: Vec<&TrackView> = v.tracks.iter().filter(|t| t.net.as_deref() == Some(p.n.as_str())).collect();
        let (pp, pn) = (pieces(&tp), pieces(&tn));
        let cp = coupled(&pp, &pn, &max_gap);
        let cn = coupled(&pn, &pp, &max_gap);
        let (lp, ln) = (len_of(&p.p), len_of(&p.n));
        let tracks_p: f64 = pp.iter().map(|x| x.len).sum();
        let tracks_n: f64 = pn.iter().map(|x| x.len).sum();
        let gaps: Vec<f64> = cp.runs.iter().map(|r| r.0).collect();
        let mut by_layer: Vec<LayerGap> = Vec::new();
        for (g, _, layer, _) in &cp.runs {
            match by_layer.iter_mut().find(|x| &x.layer == layer) {
                Some(x) => {
                    x.min = x.min.min(*g);
                    x.max = x.max.max(*g);
                }
                None => by_layer.push(LayerGap { layer: layer.clone(), nominal: nominal_on(layer), min: *g, max: *g }),
            }
        }
        let total: f64 = cp.runs.iter().map(|r| r.1).sum();
        let zdiff = (total > 0.0).then(|| {
            cp.runs
                .iter()
                .map(|(g, l, layer, w)| {
                    let k = crate::board::layer_index(&v.layers, layer).unwrap_or(0);
                    line_on(sv, k).zdiff(*w, g.max(0.001)) * l
                })
                .sum::<f64>()
                / total
        });
        let r3 = |x: f64| (x * 1000.0).round() / 1000.0;
        pv.push(PairView {
            name: p.name.clone(),
            p: p.p.clone(),
            n: p.n.clone(),
            detected,
            class: class.name.clone(),
            length_p: lp,
            length_n: ln,
            skew: r3((lp - ln).abs() * 1000.0) / 1000.0,
            coupled: r3(cp.length),
            uncoupled: r3((tracks_p - cp.length).max(tracks_n - cn.length).max(0.0)),
            gap: nominal,
            gap_min: gaps.iter().copied().reduce(f64::min).map(r3),
            gap_max: gaps.iter().copied().reduce(f64::max).map(r3),
            gaps: by_layer.into_iter().map(|x| LayerGap { min: r3(x.min), max: r3(x.max), ..x }).collect(),
            zdiff: zdiff.map(|z| (z * 10.0).round() / 10.0),
            max_skew: p.max_skew.or(class.max_skew),
            max_uncoupled: p.max_uncoupled.or(class.max_uncoupled),
            routed: routed_of(&p.p) && routed_of(&p.n),
        });
    }
    let all: Vec<String> = v.nets.iter().map(|n| n.name.clone()).collect();
    let groups = board
        .signal
        .groups
        .iter()
        .map(|g| {
            let m: Vec<MemberLength> = members(g, &all, &pv).into_iter().map(|n| MemberLength { length: len_of(&n), routed: routed_of(&n), net: n }).collect();
            let longest = m.iter().map(|x| x.length).fold(0.0, f64::max);
            let shortest = m.iter().map(|x| x.length).fold(f64::INFINITY, f64::min);
            GroupView { name: g.name.clone(), target: g.target.unwrap_or(longest), tolerance: g.tolerance, min: g.min, max: g.max, max_skew: g.max_skew, skew: if m.is_empty() { 0.0 } else { ((longest - shortest) * 1e6).round() / 1e6 }, members: m }
        })
        .collect();
    let mut net_classes = board.net_classes.clone();
    if !net_classes.iter().any(|c| c.name.eq_ignore_ascii_case("default")) {
        net_classes.insert(0, NetClass::default_class());
    }
    SignalView { classes, pairs: pv, lengths, groups, pad_to_die: board.signal.pad_to_die.clone(), detect_pairs: board.signal.detect_pairs.unwrap_or(true), net_classes }
}

// ---- Checks --------------------------------------------------------------------------------

fn sev(s: &str) -> Option<&'static str> {
    match s {
        "ignore" => None,
        "warning" => Some("warning"),
        _ => Some("error"),
    }
}

fn net_spot(v: &BoardView, net: &str) -> (P, Vec<StableId>) {
    let ts: Vec<&TrackView> = v.tracks.iter().filter(|t| t.net.as_deref() == Some(net)).collect();
    let at = ts.iter().max_by(|a, b| track_length(a).total_cmp(&track_length(b))).map(|t| {
        let n = t.points.len();
        if n >= 2 { let (a, b) = (t.points[n / 2 - 1], t.points[n / 2]); [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0] } else { t.points[0] }
    });
    let at = at.or_else(|| v.pads.iter().find(|p| p.net.as_deref() == Some(net)).map(|p| p.at)).unwrap_or([v.bounds[0], v.bounds[3]]);
    (at, ts.iter().map(|t| t.id).collect())
}

fn net_item(board: &Board, v: &BoardView, net: &str) -> Item {
    let layers: BTreeSet<String> = v.tracks.iter().filter(|t| t.net.as_deref() == Some(net)).map(|t| t.layer.clone()).collect();
    Item { kind: "track", net: Some(net.into()), netclass: Some(board.class_of(Some(net)).name), layers: layers.into_iter().collect(), pair: pair_of(v, net).map(|p| p.name.clone()), ..Default::default() }
}

fn f3(x: f64) -> String {
    format!("{:.3}", x).trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The routing-constraint checks, added to the board's violations.
pub fn check(board: &Board, v: &BoardView, out: &mut Vec<Violation>) {
    let set = rules::parse(&board.drc_rules).ok().filter(|s| !s.rules.is_empty());
    let s = &v.signal;
    let mut push = |code: &'static str, severity: &'static str, message: String, at: P, layer: Option<String>, objects: Vec<StableId>, refs: Vec<String>| {
        out.push(Violation { code, severity, message, at, layer, objects, refs });
    };

    // Impedance: every controlled track's width on its layer.
    let mut seen = BTreeSet::new();
    for t in &v.tracks {
        let Some(net) = t.net.as_deref() else { continue };
        let pair = pair_of(v, net);
        let class = board.class_of(Some(net));
        let k = crate::board::layer_index(&v.layers, &t.layer).unwrap_or(0);
        let line = line_on(&v.stackup, k);
        let custom = set.as_ref().and_then(|rs| rs.single("impedance", &Item { kind: "track", net: Some(net.into()), netclass: Some(class.name.clone()), layers: vec![t.layer.clone()], width: Some(t.width), pair: pair.map(|p| p.name.clone()), ..Default::default() }, Some(&t.layer)));
        let lt = target_on(board, v, Some(net), &t.layer);
        if lt.is_none() && custom.is_none() {
            continue;
        }
        // A short neck into a pad is allowed to be narrower.
        let (pts, _) = geometry(t);
        let touches_pad = |q: P| v.pads.iter().any(|p| p.net.as_deref() == Some(net) && geom::gap(&p.prim, &geom::Prim::circle(q, 0.0)) <= 1e-6);
        if track_length(t) < 1.5 && (pts.first().is_some_and(|q| touches_pad(*q)) || pts.last().is_some_and(|q| touches_pad(*q))) {
            continue;
        }
        let diff = pair.is_some() && class.diff_impedance.is_some();
        let gap = lt.and_then(|l| l.gap).or(class.diff_gap);
        let (z, kind) = match (diff, gap) {
            (true, Some(g)) => (line.zdiff(t.width, g), "differential"),
            _ => (line.z0(t.width), "single-ended"),
        };
        let (lo, hi, label, sv) = match &custom {
            Some(h) => {
                let Some(sv) = sev(&h.rule.severity) else { continue };
                let c = h.constraint;
                let (lo, hi) = match (c.min, c.max, c.opt) {
                    (None, None, Some(o)) => (o * 0.9, o * 1.1),
                    (mn, mx, _) => (mn.unwrap_or(0.0), mx.unwrap_or(f64::INFINITY)),
                };
                (lo, hi, format!("rule “{}”", h.rule.name), sv)
            }
            None => {
                let target = if diff { class.diff_impedance } else { class.impedance };
                let Some(target) = target else { continue };
                let tol = tol_of(&class) / 100.0;
                (target * (1.0 - tol), target * (1.0 + tol), format!("{target} Ω ±{} %", f3(tol_of(&class))), "error")
            }
        };
        if z + 1e-9 < lo || z > hi + 1e-9 {
            if !seen.insert((net.to_string(), t.layer.clone(), (t.width * 1e6) as i64)) {
                continue;
            }
            let want = lt.map(|l| format!("; {} mm meets it on {}", f3(l.width), t.layer)).unwrap_or_default();
            push("impedance", sv, format!("Track on {net} ({}) is {} mm wide: {:.1} Ω {kind}, outside {label}{want}", t.layer, f3(t.width), z), t.points[t.points.len() / 2], Some(t.layer.clone()), vec![t.id], vec![format!("track on {net}")]);
        }
    }

    // Pairs: skew, uncoupled length, gap.
    for p in &s.pairs {
        if p.coupled == 0.0 && !p.routed {
            continue;
        }
        let item = net_item(board, v, &p.p);
        let (at, mut objs) = net_spot(v, &p.p);
        objs.extend(net_spot(v, &p.n).1);
        let refs = vec![p.p.clone(), p.n.clone()];
        let custom = |kind: &str| set.as_ref().and_then(|rs| rs.single(kind, &item, None)).map(|h| (h.rule.name.clone(), h.rule.severity.clone(), h.constraint.clone()));
        if p.routed {
            let (limit, by, sv) = match custom("skew") {
                Some((name, sv, c)) => (c.max, format!(" (rule “{name}”)"), sev(&sv)),
                None => (p.max_skew, String::new(), Some("error")),
            };
            if let (Some(max), Some(sv)) = (limit, sv) {
                if p.skew > max + 1e-6 {
                    push("skew", sv, format!("Pair {}: {} is {} mm and {} {} mm, a skew of {} mm; at most {} mm{by}", p.name, p.p, f3(p.length_p), p.n, f3(p.length_n), f3(p.skew), f3(max)), at, None, objs.clone(), refs.clone());
                }
            }
        }
        let (limit, by, sv) = match custom("diff_pair_uncoupled") {
            Some((name, sv, c)) => (c.max, format!(" (rule “{name}”)"), sev(&sv)),
            None => (p.max_uncoupled, String::new(), Some("error")),
        };
        if let (Some(max), Some(sv)) = (limit, sv) {
            if p.uncoupled > max + 1e-6 {
                push("diff_pair_uncoupled", sv, format!("Pair {}: {} mm runs uncoupled; at most {} mm{by}", p.name, f3(p.uncoupled), f3(max)), at, None, objs.clone(), refs.clone());
            }
        }
        // The gap: a custom rule's range everywhere, else each layer's own gap ± 10 %.
        let rule = custom("diff_pair_gap");
        for lg in &p.gaps {
            let (lo, hi, by, sv) = match &rule {
                Some((name, sv, c)) => (c.min.or(c.opt.map(|o| o * 0.9)), c.max.or(c.opt.map(|o| o * 1.1)), format!(" (rule “{name}”)"), sev(sv)),
                None => match lg.nominal {
                    Some(g) => {
                        let tol = (g * 0.1).max(0.005);
                        (Some(g - tol), Some(g + tol), format!(" ({} mm ± {} mm for {} on {})", f3(g), f3(tol), p.class, lg.layer), Some("error"))
                    }
                    None => (None, None, String::new(), None),
                },
            };
            let Some(sv) = sv else { continue };
            if lo.is_some_and(|lo| lg.min + 1e-6 < lo) {
                push("diff_pair_gap", sv, format!("Pair {}: the gap narrows to {} mm on {}; at least {} mm{by}", p.name, f3(lg.min), lg.layer, f3(lo.unwrap()), ), at, Some(lg.layer.clone()), objs.clone(), refs.clone());
            }
            if hi.is_some_and(|hi| lg.max > hi + 1e-6) {
                push("diff_pair_gap", sv, format!("Pair {}: the gap widens to {} mm on {}; at most {} mm{by}", p.name, f3(lg.max), lg.layer, f3(hi.unwrap())), at, Some(lg.layer.clone()), objs.clone(), refs.clone());
            }
        }
    }

    // Length-matching groups.
    for g in &s.groups {
        let routed: Vec<&MemberLength> = g.members.iter().filter(|m| m.routed).collect();
        let def = board.signal.groups.iter().find(|x| x.name == g.name);
        let explicit_target = def.and_then(|d| d.target);
        for m in &routed {
            let (at, objs) = net_spot(v, &m.net);
            let mut bad_len = |msg: String| push("length", "error", msg, at, None, objs.clone(), vec![m.net.clone()]);
            if let Some(t) = explicit_target {
                let tol = g.tolerance.unwrap_or(t * 0.01);
                if (m.length - t).abs() > tol + 1e-6 {
                    bad_len(format!("{} is {} mm long; group {} wants {} ± {} mm", m.net, f3(m.length), g.name, f3(t), f3(tol)));
                }
            } else if let Some(tol) = g.tolerance {
                if g.target - m.length > tol + 1e-6 && routed.len() == g.members.len() {
                    bad_len(format!("{} is {} mm long, {} mm short of the longest in group {} (± {} mm)", m.net, f3(m.length), f3(g.target - m.length), g.name, f3(tol)));
                }
            }
            if let Some(mn) = g.min {
                if m.length + 1e-6 < mn {
                    bad_len(format!("{} is {} mm long; group {} wants at least {} mm", m.net, f3(m.length), g.name, f3(mn)));
                }
            }
            if let Some(mx) = g.max {
                if m.length > mx + 1e-6 {
                    bad_len(format!("{} is {} mm long; group {} wants at most {} mm", m.net, f3(m.length), g.name, f3(mx)));
                }
            }
        }
        if let Some(mx) = g.max_skew {
            if routed.len() == g.members.len() && g.skew > mx + 1e-6 {
                let (at, _) = net_spot(v, &routed[0].net);
                let objs = routed.iter().flat_map(|m| net_spot(v, &m.net).1).collect();
                push("skew", "error", format!("Group {}: lengths span {} mm (longest − shortest); at most {} mm", g.name, f3(g.skew), f3(mx)), at, None, objs, routed.iter().map(|m| m.net.clone()).collect());
            }
        }
    }

    // Custom rules on whole nets: length, via_count, skew across the nets a rule matches.
    let Some(rs) = set else { return };
    let mut skew_groups: BTreeMap<String, (String, f64, Vec<&NetLength>)> = BTreeMap::new();
    for l in &s.lengths {
        let item = net_item(board, v, &l.net);
        let (at, objs) = net_spot(v, &l.net);
        if l.routed {
            if let Some(h) = rs.single("length", &item, None) {
                if let Some(sv) = sev(&h.rule.severity) {
                    let c = h.constraint;
                    if c.min.is_some_and(|m| l.length + 1e-6 < m) || c.max.is_some_and(|m| l.length > m + 1e-6) {
                        let range = match (c.min, c.max) {
                            (Some(a), Some(b)) => format!("{} to {} mm", f3(a), f3(b)),
                            (Some(a), None) => format!("at least {} mm", f3(a)),
                            (None, Some(b)) => format!("at most {} mm", f3(b)),
                            _ => String::new(),
                        };
                        push("length", sv, format!("{} is {} mm long; {range} (rule “{}”)", l.net, f3(l.length), h.rule.name), at, None, objs.clone(), vec![l.net.clone()]);
                    }
                }
            }
        }
        if let Some(h) = rs.single("via_count", &item, None) {
            if let (Some(sv), Some(mx)) = (sev(&h.rule.severity), h.constraint.max) {
                if l.vias as f64 > mx + 1e-9 {
                    push("via_count", sv, format!("{} has {} vias; at most {} (rule “{}”)", l.net, l.vias, mx, h.rule.name), at, None, objs.clone(), vec![l.net.clone()]);
                }
            }
        }
        // Pairs' skew was checked with the pair; a rule over plain nets holds them together.
        if item.pair.is_none() && l.routed {
            if let Some(h) = rs.single("skew", &item, None) {
                if let (Some(_), Some(mx)) = (sev(&h.rule.severity), h.constraint.max) {
                    skew_groups.entry(h.rule.name.clone()).or_insert_with(|| (h.rule.severity.clone(), mx, vec![])).2.push(l);
                }
            }
        }
    }
    for (name, (severity, mx, ls)) in skew_groups {
        if ls.len() < 2 {
            continue;
        }
        let hi = ls.iter().map(|l| l.length).fold(f64::MIN, f64::max);
        let lo = ls.iter().map(|l| l.length).fold(f64::MAX, f64::min);
        if hi - lo > mx + 1e-6 {
            let (at, _) = net_spot(v, &ls[0].net);
            push("skew", sev(&severity).unwrap_or("error"), format!("Nets of rule “{name}” span {} mm ({} to {} mm); at most {} mm", f3(hi - lo), f3(lo), f3(hi), f3(mx)), at, None, ls.iter().flat_map(|l| net_spot(v, &l.net).1).collect(), ls.iter().map(|l| l.net.clone()).collect());
        }
    }
}

/// The clearance the DRC asks between two nets of the same pair: no more than the pair's own gap allows.
pub fn pair_clearance(board: &Board, v: &BoardView, a: Option<&str>, b: Option<&str>, need: f64) -> f64 {
    let (Some(a), Some(b)) = (a, b) else { return need };
    match pair_of(v, a) {
        Some(p) if (p.p == a && p.n == b) || (p.n == a && p.p == b) => {
            let g = v.signal.classes.iter().find(|c| c.name == p.class).and_then(|c| c.layers.iter().filter_map(|l| l.gap).reduce(f64::min)).or(board.class_of(Some(a)).diff_gap);
            match g {
                Some(g) => need.min((g * 0.9).max(board.rules.min_space)),
                None => need,
            }
        }
        _ => need,
    }
}

// ---- For the fab and other tools ---------------------------------------------------------

/// One controlled-impedance line the fab must build: a class's target on one layer.
#[derive(Clone, Debug, Serialize)]
pub struct ImpedanceRow {
    pub class: String,
    pub layer: String,
    /// "single" or "differential".
    pub kind: &'static str,
    pub target: f64,
    /// ± %.
    pub tolerance: f64,
    pub width: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
    /// What the closed forms give at that width (and gap).
    pub computed: f64,
    pub model: &'static str,
    /// The reference planes (copper layers above and below).
    pub refs: Vec<String>,
    /// The nets it applies to.
    pub nets: Vec<String>,
}

/// Every impedance the board asks for, per class and layer (layers its nets are routed on, or all when unrouted).
pub fn impedance_table(board: &Board, v: &BoardView) -> Vec<ImpedanceRow> {
    let mut out = Vec::new();
    for c in &v.signal.classes {
        let class = board.net_classes.iter().find(|x| x.name == c.name).cloned().unwrap_or_else(NetClass::default_class);
        let nets: Vec<String> = v.nets.iter().filter(|n| n.class == c.name).map(|n| n.name.clone()).collect();
        let used: BTreeSet<&str> = v.tracks.iter().filter(|t| t.net.as_ref().is_some_and(|n| nets.contains(n))).map(|t| t.layer.as_str()).collect();
        for (k, lt) in c.layers.iter().enumerate() {
            if !used.is_empty() && !used.contains(lt.layer.as_str()) {
                continue;
            }
            let refs: Vec<String> = [k.checked_sub(1), Some(k + 1)].into_iter().flatten().filter_map(|i| v.layers.get(i).cloned()).collect();
            let pair_nets: Vec<String> = nets.iter().filter(|n| pair_of(v, n).is_some()).cloned().collect();
            let single_nets: Vec<String> = nets.iter().filter(|n| pair_of(v, n).is_none()).cloned().collect();
            if let Some(z) = c.impedance {
                out.push(ImpedanceRow { class: c.name.clone(), layer: lt.layer.clone(), kind: "single", target: z, tolerance: c.tolerance, width: lt.width, gap: None, computed: (lt.z0 * 10.0).round() / 10.0, model: lt.line.name(), refs: refs.clone(), nets: if class.diff_impedance.is_some() { single_nets.clone() } else { nets.clone() } });
            }
            if let (Some(z), Some(g), Some(zd)) = (c.diff_impedance, lt.gap, lt.zdiff) {
                out.push(ImpedanceRow { class: c.name.clone(), layer: lt.layer.clone(), kind: "differential", target: z, tolerance: c.tolerance, width: lt.width, gap: Some(g), computed: (zd * 10.0).round() / 10.0, model: lt.line.name(), refs, nets: pair_nets });
            }
        }
    }
    out
}

/// The fab's "impedance control" note (none when nothing is controlled).
pub fn fab_note(board: &Board, v: &BoardView) -> Option<String> {
    let rows = impedance_table(board, v);
    if rows.is_empty() {
        return None;
    }
    let mut s = String::from("IMPEDANCE CONTROL\n\nBuild to the stackup below and hold these impedances; adjust track widths (and pair gaps) by up to ±10 % to meet them, and report the as-built values. Test coupons per IPC-2141 / IPC-6012.\n\n");
    s.push_str("Stackup (top down):\n");
    for r in &v.stackup.rows {
        let mut line = format!("  {:<18} {:>7.4} mm", r.name, r.thickness);
        if let Some(m) = &r.material {
            line.push_str(&format!("  {m}"));
        }
        if let Some(e) = r.epsilon_r.filter(|_| r.kind == "core" || r.kind == "prepreg") {
            line.push_str(&format!("  εr {e}"));
        }
        if let Some(oz) = r.copper_oz {
            line.push_str(&format!("  {oz} oz"));
        }
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str("\nControlled lines:\n");
    for r in &rows {
        s.push_str(&format!(
            "  {:<10} {:<7} {:<12} {:>5} Ω ±{} %  width {:.3} mm{}  ({} over {}; computed {} Ω)  nets: {}\n",
            r.class,
            r.layer,
            r.kind,
            r.target,
            r.tolerance,
            r.width,
            r.gap.map(|g| format!(", gap {g:.3} mm")).unwrap_or_default(),
            r.model,
            r.refs.join(" / "),
            r.computed,
            if r.nets.is_empty() { "—".into() } else { r.nets.join(", ") }
        ));
    }
    s.push_str("\nWidths from closed-form models (Hammerstad–Jensen microstrip, Cohn/Wheeler stripline, IPC-2141 coupling), solder mask not counted; the fab's field solver is final.\n");
    Some(s)
}

/// `.kicad_dru` rules for the board's routing constraints (class widths per layer, pair gap,
/// uncoupled length, skew, length groups), as KiCad reads them.
pub fn dru_rules(board: &Board, v: &BoardView) -> String {
    let mut out = String::new();
    let q = |s: &str| s.replace('\'', "");
    for c in &v.signal.classes {
        let class = board.net_classes.iter().find(|x| x.name == c.name).cloned().unwrap_or_else(NetClass::default_class);
        let tol = c.tolerance / 100.0;
        for lt in &c.layers {
            let kl = rules::kicad_layer(&lt.layer);
            let w = lt.width;
            // Necks into fine-pitch pads may be narrower (the class neck width).
            let neck = class.neck_width.unwrap_or(w * 0.6).max(board.rules.min_track).min(w * (1.0 - tol));
            out.push_str(&format!("(rule \"{} width {}\"\n  (layer \"{kl}\")\n  (condition \"A.NetClass == '{}'\")\n  (constraint track_width (min {:.4}mm) (opt {:.4}mm) (max {:.4}mm)))\n", c.name, lt.layer, q(&c.name), neck, w, w * (1.0 + tol)));
            if let Some(g) = lt.gap {
                out.push_str(&format!("(rule \"{} pair gap {}\"\n  (layer \"{kl}\")\n  (condition \"A.NetClass == '{}' && A.inDiffPair('*')\")\n  (constraint diff_pair_gap (min {:.4}mm) (opt {:.4}mm) (max {:.4}mm)))\n", c.name, lt.layer, q(&c.name), g * 0.9, g, g * 1.1));
            }
        }
        if let Some(u) = class.max_uncoupled {
            out.push_str(&format!("(rule \"{} uncoupled\"\n  (condition \"A.NetClass == '{}' && A.inDiffPair('*')\")\n  (constraint diff_pair_uncoupled (max {u}mm)))\n", c.name, q(&c.name)));
        }
        if let Some(sk) = class.max_skew {
            out.push_str(&format!("(rule \"{} skew\"\n  (condition \"A.NetClass == '{}' && A.isCoupledDiffPair()\")\n  (constraint skew (max {sk}mm)))\n", c.name, q(&c.name)));
        }
    }
    for p in &v.signal.pairs {
        let def = board.signal.pairs.iter().find(|x| x.name == p.name);
        if let Some(sk) = def.and_then(|d| d.max_skew) {
            out.push_str(&format!("(rule \"pair {} skew\"\n  (condition \"A.NetName == '{}' || A.NetName == '{}'\")\n  (constraint skew (max {sk}mm)))\n", p.name, q(&p.p), q(&p.n)));
        }
        if let Some(u) = def.and_then(|d| d.max_uncoupled) {
            out.push_str(&format!("(rule \"pair {} uncoupled\"\n  (condition \"A.NetName == '{}' || A.NetName == '{}'\")\n  (constraint diff_pair_uncoupled (max {u}mm)))\n", p.name, q(&p.p), q(&p.n)));
        }
    }
    for g in &v.signal.groups {
        if g.members.is_empty() {
            continue;
        }
        let cond = g.members.iter().map(|m| format!("A.NetName == '{}'", q(&m.net))).collect::<Vec<_>>().join(" || ");
        let def = board.signal.groups.iter().find(|x| x.name == g.name);
        let mut cs = String::new();
        if let Some(t) = def.and_then(|d| d.target) {
            let tol = g.tolerance.unwrap_or(t * 0.01);
            cs.push_str(&format!("\n  (constraint length (min {:.4}mm) (opt {:.4}mm) (max {:.4}mm))", t - tol, t, t + tol));
        } else if g.min.is_some() || g.max.is_some() {
            cs.push_str(&format!("\n  (constraint length{}{})", g.min.map(|x| format!(" (min {x}mm)")).unwrap_or_default(), g.max.map(|x| format!(" (max {x}mm)")).unwrap_or_default()));
        }
        if let Some(sk) = g.max_skew.or(if def.and_then(|d| d.target).is_none() { g.tolerance } else { None }) {
            cs.push_str(&format!("\n  (constraint skew (max {sk}mm))"));
        }
        if !cs.is_empty() {
            out.push_str(&format!("(rule \"length group {}\"\n  (condition \"{cond}\"){cs})\n", g.name));
        }
    }
    out
}

// ---- Commands -----------------------------------------------------------------------

fn opt_set(slot: &mut Option<f64>, v: Option<f64>) {
    if let Some(x) = v {
        *slot = (x > 0.0).then_some(x);
    }
}

/// `pcb.netclass.set`: add or change one of the board's net classes —
/// widths, clearance, vias, the nets and patterns it takes, and its
/// electrical targets (`impedance`, `diff_impedance`, `diff_gap`,
/// `impedance_tolerance` %, `max_uncoupled`, `max_skew`, `pair_via_pitch`; 0
/// clears one). Answers with the width and gap that meet the targets on
/// every copper layer, from the stackup.
#[derive(Deserialize, JsonSchema)]
pub struct SetNetClass {
    pub board: StableId,
    pub name: String,
    pub rename: Option<String>,
    #[serde(default)]
    pub delete: bool,
    pub track_width: Option<f64>,
    pub clearance: Option<f64>,
    pub via_drill: Option<f64>,
    pub via_diameter: Option<f64>,
    pub neck_width: Option<f64>,
    /// The nets in the class (replacing the list); each leaves any other class.
    pub nets: Option<Vec<String>>,
    /// Nets added to the class.
    #[serde(default)]
    pub add_nets: Vec<String>,
    pub patterns: Option<Vec<String>>,
    pub impedance: Option<f64>,
    pub diff_impedance: Option<f64>,
    pub diff_gap: Option<f64>,
    pub impedance_tolerance: Option<f64>,
    pub max_uncoupled: Option<f64>,
    pub max_skew: Option<f64>,
    pub pair_via_pitch: Option<f64>,
}

impl Command for SetNetClass {
    const NAME: &'static str = "pcb.netclass.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(bad("a net class needs a name"));
        }
        let at = b.net_classes.iter().position(|c| c.name.eq_ignore_ascii_case(&name));
        if self.delete {
            let i = at.ok_or_else(|| bad(format!("no net class {name}")))?;
            b.net_classes.remove(i);
            crate::check_board(&b)?;
            tx.set(self.board, b)?;
            return Ok(json!({ "deleted": name }));
        }
        let mut c = match at {
            Some(i) => b.net_classes[i].clone(),
            None => {
                let d = b.class_of(None);
                NetClass { name: name.clone(), nets: vec![], patterns: vec![], impedance: None, diff_impedance: None, diff_gap: None, impedance_tolerance: None, max_uncoupled: None, max_skew: None, pair_via_pitch: None, neck_width: None, ..d }
            }
        };
        if let Some(r) = self.rename.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()) {
            if b.net_classes.iter().any(|x| x.name.eq_ignore_ascii_case(&r)) && !r.eq_ignore_ascii_case(&name) {
                return Err(bad(format!("there is a net class {r} already")));
            }
            c.name = r;
        }
        if let Some(v) = self.track_width {
            c.track_width = v;
        }
        if let Some(v) = self.clearance {
            c.clearance = v;
        }
        if let Some(v) = self.via_drill {
            c.via_drill = v;
        }
        if let Some(v) = self.via_diameter {
            c.via_diameter = v;
        }
        opt_set(&mut c.neck_width, self.neck_width);
        if let Some(n) = self.nets {
            c.nets = n;
        }
        for n in self.add_nets {
            if !c.nets.contains(&n) {
                c.nets.push(n);
            }
        }
        c.nets = c.nets.iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
        if let Some(p) = self.patterns {
            c.patterns = p.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        }
        opt_set(&mut c.impedance, self.impedance);
        opt_set(&mut c.diff_impedance, self.diff_impedance);
        opt_set(&mut c.diff_gap, self.diff_gap);
        opt_set(&mut c.impedance_tolerance, self.impedance_tolerance);
        opt_set(&mut c.max_uncoupled, self.max_uncoupled);
        opt_set(&mut c.max_skew, self.max_skew);
        opt_set(&mut c.pair_via_pitch, self.pair_via_pitch);
        let mine = c.nets.clone();
        for other in b.net_classes.iter_mut().filter(|x| !x.name.eq_ignore_ascii_case(&name)) {
            other.nets.retain(|n| !mine.contains(n));
        }
        match at {
            Some(i) => b.net_classes[i] = c.clone(),
            None => b.net_classes.push(c.clone()),
        }
        crate::check_board(&b)?;
        let sv = crate::stackup::resolve(&b);
        let layers = if has_targets(&c) { targets(&b, &sv, &c) } else { vec![] };
        tx.set(self.board, b)?;
        Ok(json!({ "class": c, "layers": layers }))
    }
}

/// `pcb.pair.set`: name two nets a differential pair (pairs are also found
/// from net names: `D+`/`D-`, `_P`/`_N`, `DP`/`DM`), with its own skew and
/// uncoupled-length limits; `delete` removes a pair set by hand.
#[derive(Deserialize, JsonSchema)]
pub struct SetPair {
    pub board: StableId,
    pub name: String,
    pub p: Option<String>,
    pub n: Option<String>,
    pub max_skew: Option<f64>,
    pub max_uncoupled: Option<f64>,
    #[serde(default)]
    pub delete: bool,
}

impl Command for SetPair {
    const NAME: &'static str = "pcb.pair.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        let name = self.name.trim().to_string();
        let at = b.signal.pairs.iter().position(|p| p.name.eq_ignore_ascii_case(&name));
        if self.delete {
            let i = at.ok_or_else(|| bad(format!("no pair {name} set by hand")))?;
            b.signal.pairs.remove(i);
            tx.set(self.board, b)?;
            return Ok(json!({ "deleted": name }));
        }
        let detected = {
            let v = crate::view(tx.world(), self.board);
            v.and_then(|v| v.signal.pairs.iter().find(|p| p.name.eq_ignore_ascii_case(&name)).map(|p| DiffPair { name: p.name.clone(), p: p.p.clone(), n: p.n.clone(), max_skew: None, max_uncoupled: None }))
        };
        let mut p = at.map(|i| b.signal.pairs[i].clone()).or(detected).unwrap_or(DiffPair { name: name.clone(), p: String::new(), n: String::new(), max_skew: None, max_uncoupled: None });
        if let Some(x) = self.p {
            p.p = x.trim().to_string();
        }
        if let Some(x) = self.n {
            p.n = x.trim().to_string();
        }
        opt_set(&mut p.max_skew, self.max_skew);
        opt_set(&mut p.max_uncoupled, self.max_uncoupled);
        if b.signal.pairs.iter().enumerate().any(|(i, q)| Some(i) != at && (q.p == p.p || q.n == p.p || q.p == p.n || q.n == p.n)) {
            return Err(bad(format!("{} or {} is in another pair already", p.p, p.n)));
        }
        match at {
            Some(i) => b.signal.pairs[i] = p.clone(),
            None => b.signal.pairs.push(p.clone()),
        }
        crate::check_board(&b)?;
        tx.set(self.board, b)?;
        Ok(json!({ "pair": p }))
    }
}

/// `pcb.length.group`: nets held to one length — a `target` (± `tolerance`),
/// or each within `tolerance` of the longest, `min`/`max`, and the largest
/// spread `max_skew`. Members are nets, patterns (`D?`) or pair names.
#[derive(Deserialize, JsonSchema)]
pub struct SetGroup {
    pub board: StableId,
    pub name: String,
    pub nets: Option<Vec<String>>,
    pub target: Option<f64>,
    pub tolerance: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub max_skew: Option<f64>,
    #[serde(default)]
    pub delete: bool,
}

impl Command for SetGroup {
    const NAME: &'static str = "pcb.length.group";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        let name = self.name.trim().to_string();
        let at = b.signal.groups.iter().position(|g| g.name.eq_ignore_ascii_case(&name));
        if self.delete {
            let i = at.ok_or_else(|| bad(format!("no length group {name}")))?;
            b.signal.groups.remove(i);
            tx.set(self.board, b)?;
            return Ok(json!({ "deleted": name }));
        }
        let mut g = at.map(|i| b.signal.groups[i].clone()).unwrap_or(LengthGroup { name: name.clone(), nets: vec![], target: None, tolerance: None, min: None, max: None, max_skew: None });
        if let Some(n) = self.nets {
            g.nets = n.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        }
        opt_set(&mut g.target, self.target);
        opt_set(&mut g.tolerance, self.tolerance);
        opt_set(&mut g.min, self.min);
        opt_set(&mut g.max, self.max);
        opt_set(&mut g.max_skew, self.max_skew);
        match at {
            Some(i) => b.signal.groups[i] = g.clone(),
            None => b.signal.groups.push(g.clone()),
        }
        crate::check_board(&b)?;
        tx.set(self.board, b)?;
        Ok(json!({ "group": g }))
    }
}

/// `pcb.signal.set`: whether pairs are found from net names, and pad-to-die
/// lengths by pad (`{ "U1.5": 1.2 }`, 0 removes one).
#[derive(Deserialize, JsonSchema)]
pub struct SetSignal {
    pub board: StableId,
    pub detect_pairs: Option<bool>,
    #[serde(default)]
    pub pad_to_die: BTreeMap<String, f64>,
}

impl Command for SetSignal {
    const NAME: &'static str = "pcb.signal.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        if let Some(d) = self.detect_pairs {
            b.signal.detect_pairs = (!d).then_some(false);
        }
        for (k, v) in self.pad_to_die {
            if v <= 0.0 {
                b.signal.pad_to_die.remove(&k);
            } else {
                b.signal.pad_to_die.insert(k, v);
            }
        }
        crate::check_board(&b)?;
        tx.set(self.board, b.clone())?;
        Ok(json!({ "signal": b.signal }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<SetNetClass>().command::<SetPair>().command::<SetGroup>().command::<SetSignal>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv4() -> StackupView {
        // JLCPCB's JLC04161H-7628: 0.2104 mm prepreg (εr 4.4) to the outer layers, 1 oz outer, 0.5 oz inner.
        let d = |kind, t: f64, er: f64| crate::stackup::Dielectric { kind, thickness: t, material: "FR4".into(), epsilon_r: Some(er), loss_tangent: None };
        use crate::stackup::DielectricKind::*;
        StackupView { rows: vec![], dielectrics: vec![d(Prepreg, 0.2104, 4.4), d(Core, 1.065, 4.6), d(Prepreg, 0.2104, 4.4)], copper_oz: vec![1.0, 0.5, 0.5, 1.0], total: 1.6, custom: true, finish: String::new() }
    }

    #[test]
    fn widths_invert_the_calculators() {
        let sv = sv4();
        for k in 0..4 {
            let line = line_on(&sv, k);
            for z in [40.0, 50.0, 75.0] {
                let w = line.width_for(z).unwrap();
                assert!((line.z0(w) - z).abs() < 1e-6, "{z} Ω on layer {k}: {w} mm gives {}", line.z0(w));
            }
            let s = 0.15;
            let w = line.width_for_diff(90.0, s).unwrap();
            assert!((line.zdiff(w, s) - 90.0).abs() < 1e-6);
            let g = line.gap_for_diff(100.0, w).unwrap();
            assert!((line.zdiff(w, g) - 100.0).abs() < 1e-6);
        }
        // Top layer: Hammerstad–Jensen, as the calculator panel gives it.
        let w = line_on(&sv, 0).width_for(50.0).unwrap();
        let (z, _) = calc::microstrip(w, 0.2104, 0.035, 4.4);
        assert!((z - 50.0).abs() < 1e-6);
        // A known point: 50 Ω microstrip on 0.2104 mm 4.4 prepreg, 35 µm copper is about 0.36 mm (JLCPCB gives 0.3438–0.36).
        assert!((0.32..0.40).contains(&w), "{w}");
    }

    #[test]
    fn offset_stripline_meets_the_centred_one() {
        // Centred: the offset form is the symmetric stripline.
        let z = calc::stripline_offset(0.2, 0.3, 4.3, 0.3, 4.3, 0.017);
        let c = calc::stripline(0.2, 0.617, 0.017, 4.3);
        assert!((z - c).abs() < 1e-9);
        // Cohn's coupled stripline: far apart the odd mode tends to the single line.
        let (ze, zo) = calc::coupled_stripline_thin(0.2, 5.0, 0.6, 4.3);
        let z1 = calc::stripline(0.2, 0.6, 0.0, 4.3);
        assert!((ze - z1).abs() / z1 < 1e-3 && (zo - z1).abs() / z1 < 1e-3, "{ze} {zo} {z1}");
        // Close together the odd mode drops (the pair couples).
        let (_, zo) = calc::coupled_stripline_thin(0.2, 0.1, 0.6, 4.3);
        assert!(zo < z1 * 0.9);
    }

    #[test]
    fn pairs_are_found_from_names() {
        let nets: BTreeSet<String> = ["USB_D+", "USB_D-", "ETH_TX_P", "ETH_TX_N", "LVDS_DP", "LVDS_DM", "CLKP", "CLKN", "GND", "VBUS", "SDA"].iter().map(|s| s.to_string()).collect();
        let found = detect(&nets);
        let names: Vec<(&str, &str, &str)> = found.iter().map(|p| (p.name.as_str(), p.p.as_str(), p.n.as_str())).collect();
        assert!(names.contains(&("USB_D", "USB_D+", "USB_D-")), "{names:?}");
        assert!(names.contains(&("ETH_TX", "ETH_TX_P", "ETH_TX_N")));
        assert!(names.contains(&("LVDS", "LVDS_DP", "LVDS_DM")));
        assert!(names.contains(&("CLK", "CLKP", "CLKN")));
        assert_eq!(found.len(), 4);
    }

    #[test]
    fn arcs_count_exactly() {
        // A half circle of radius 2 as one bulge-1 segment: π·2.
        let l = path_length(&[[0.0, 0.0], [4.0, 0.0]], &[1.0]);
        assert!((l - 2.0 * PI).abs() < 1e-9, "{l}");
        // A quarter: bulge tan(π/8).
        let l = path_length(&[[1.0, 0.0], [0.0, 1.0]], &[(PI / 8.0).tan()]);
        assert!((l - PI / 2.0).abs() < 1e-9, "{l}");
    }

    #[test]
    fn via_length_reads_the_stackup() {
        let sv = sv4();
        // Top to bottom: every dielectric plus the inner copper, from mid copper to mid copper.
        let l = via_length(&sv, 0, 3);
        let want = 0.2104 + 1.065 + 0.2104 + 2.0 * 0.5 * OZ_MM + 0.035;
        assert!((l - want).abs() < 1e-9, "{l} vs {want}");
        assert!((via_length(&sv, 0, 1) - (0.2104 + (0.035 + 0.0175) / 2.0)).abs() < 1e-9);
    }

    #[test]
    fn coupling_measures_parallel_runs() {
        let t = |pts: Vec<P>, b: Vec<f64>| TrackView { id: StableId::default(), net: None, layer: "top".into(), width: 0.2, points: pts.clone(), auto: false, reroute: false, vertices: if b.is_empty() { vec![] } else { pts }, bulges: b };
        // Two tracks 0.35 mm apart centre to centre (0.15 mm gap) for 10 mm, then apart.
        let a = t(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0]], vec![]);
        let b = t(vec![[0.0, 0.35], [10.0, 0.35]], vec![]);
        let c = coupled(&pieces(&[&a]), &pieces(&[&b]), &|_| 0.3);
        assert!((c.length - 10.0).abs() < 1e-9, "{}", c.length);
        assert!((c.runs[0].0 - 0.15).abs() < 1e-9);
        // Concentric arcs couple along their shared sweep.
        let r1 = 2.0;
        let r2 = 2.35;
        let q = (PI / 8.0).tan();
        let a = t(vec![[r1, 0.0], [0.0, r1]], vec![q]);
        let b = t(vec![[r2, 0.0], [0.0, r2]], vec![q]);
        let c = coupled(&pieces(&[&a]), &pieces(&[&b]), &|_| 0.3);
        assert!((c.length - r1 * PI / 2.0).abs() < 1e-6, "{}", c.length);
    }
}
