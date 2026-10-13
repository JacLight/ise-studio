//! KiCad interop: `.kicad_pcb` in and out (the documented S-expression
//! format, KiCad 6 to 9; footprints may also be KiCad 5 `module`s).
//!
//! Import is two stages: [`parse_board`] reads the file into a [`KicadBoard`]
//! in Forma's terms (no project needed, so other readers can share it), and
//! `pcb.board.import_kicad` writes that as a board, its footprints,
//! components, tracks, vias and pours in one change. KiCad draws y down;
//! Forma's y is up: a point (x, y) becomes (x, −y), angles stay
//! (counter-clockwise as seen from the top), and the board is moved so its
//! outline's lower-left corner is the origin.
//!
//! Footprints come from the file itself (every footprint instance carries its
//! own copy): pads, silkscreen, courtyard and a body box from the fabrication
//! outline (heights estimated; KiCad's 3D model references are ignored).
//! Identical copies become one board footprint, named after the library id.
//!
//! Export writes KiCad 8 (`version 20240108`): layers, nets, footprints with
//! their pads and silkscreen, tracks, vias, pours (outlines and settings,
//! refilled by KiCad) and the Edge.Cuts outline.

use crate::board::BoardView;
use crate::footprints::{BodyPart, BodyShape, Footprint, Pad, PadKind, PadShape};
use crate::geom::{self, P};
use crate::sexpr::{self, Sexp, atom, list, n, s};
use crate::{Board, NetClass, PadConnection, PcbComponent, Rules, Side, Track, Via, Zone};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use manifold_csg::{CrossSection, JoinType};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

// ---- The board in Forma's terms ------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct KComponent {
    pub reference: String,
    pub value: String,
    /// The board footprint it uses (a key of `KicadBoard::footprints`).
    pub footprint: String,
    pub at: P,
    pub rotation: f64,
    pub side: Side,
    pub pins: BTreeMap<String, String>,
    pub dnp: bool,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KTrack {
    pub net: Option<String>,
    pub layer: String,
    pub width: f64,
    pub points: Vec<P>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KVia {
    pub net: Option<String>,
    pub at: P,
    pub drill: f64,
    pub diameter: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KZone {
    pub net: String,
    pub layer: String,
    pub outline: Vec<P>,
    pub clearance: Option<f64>,
    pub min_width: f64,
    pub priority: i32,
    pub connection: PadConnection,
    pub thermal_gap: Option<f64>,
    pub thermal_spoke: Option<f64>,
}

/// A `.kicad_pcb` read into Forma's terms (mm, y up, outline lower-left at the origin).
#[derive(Clone, Debug)]
pub struct KicadBoard {
    pub title: Option<String>,
    pub revision: Option<String>,
    /// 2 or 4 copper layers.
    pub layers: u8,
    pub thickness: f64,
    pub outline: Vec<P>,
    pub cutouts: Vec<Vec<P>>,
    pub footprints: BTreeMap<String, Footprint>,
    pub components: Vec<KComponent>,
    pub tracks: Vec<KTrack>,
    pub vias: Vec<KVia>,
    pub zones: Vec<KZone>,
    /// A Default class from the most common track width and via (KiCad keeps classes in .kicad_pro).
    pub default_class: NetClass,
    pub rules: Rules,
    /// What was approximated or left out.
    pub notes: Vec<String>,
    /// How far the board was moved to put its lower-left corner at the origin (Forma coordinates).
    pub shift: P,
    /// Keepouts (rule areas).
    pub areas: Vec<KArea>,
    /// Blind, buried and micro vias: (index into `vias`, what it is).
    pub via_specs: Vec<(usize, crate::vias::ViaSpec)>,
    /// The board had teardrops.
    pub teardrops: bool,
}

/// A keepout read from a board file.
#[derive(Clone, Debug)]
pub struct KArea {
    pub name: String,
    pub outline: Vec<P>,
    pub layers: Vec<String>,
    /// tracks, vias, pads, pours, footprints kept out.
    pub no: [bool; 5],
}

// ---- Small geometry --------------------------------------------------------------

fn rot(p: P, deg: f64) -> P {
    geom::rotate(p, deg)
}

/// KiCad file point → Forma (before the shift).
fn kf(x: f64, y: f64) -> P {
    [x, -y]
}

fn r6(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

fn r6p(p: P) -> P {
    [r6(p[0]), r6(p[1])]
}

fn norm_angle(a: f64) -> f64 {
    let a = a.rem_euclid(360.0);
    let r = (a * 1e6).round() / 1e6;
    if (r - 360.0).abs() < 1e-9 { 0.0 } else { r }
}

/// The points of an arc through `a`, `m`, `b` (within `tol` mm), ends included.
pub(crate) fn arc3(a: P, m: P, b: P, tol: f64) -> Vec<P> {
    let d = 2.0 * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
    if d.abs() < 1e-12 {
        return vec![a, b];
    }
    let sq = |p: P| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (m[1] - b[1]) + sq(m) * (b[1] - a[1]) + sq(b) * (a[1] - m[1])) / d;
    let uy = (sq(a) * (b[0] - m[0]) + sq(m) * (a[0] - b[0]) + sq(b) * (m[0] - a[0])) / d;
    let c = [ux, uy];
    let r = geom::dist(a, c);
    let ang = |p: P| (p[1] - c[1]).atan2(p[0] - c[0]);
    let (a0, am, a1) = (ang(a), ang(m), ang(b));
    let tau = std::f64::consts::TAU;
    // Sweep from a0 to a1 through am: counter-clockwise if am lies between going ccw.
    let ccw_span = (a1 - a0).rem_euclid(tau);
    let ccw_mid = (am - a0).rem_euclid(tau);
    let span = if ccw_mid <= ccw_span { ccw_span } else { ccw_span - tau };
    arc_center(c, r, a0, span, tol, a, b)
}

pub(crate) fn arc_center(c: P, r: f64, a0: f64, span: f64, tol: f64, a: P, b: P) -> Vec<P> {
    let n = ((geom::segments_for(r, tol) as f64) * span.abs() / std::f64::consts::TAU).ceil().max(1.0) as usize;
    let mut out = vec![a];
    for k in 1..n {
        let t = a0 + span * k as f64 / n as f64;
        out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
    }
    out.push(b);
    out
}

pub(crate) fn circle_pts(c: P, r: f64, tol: f64) -> Vec<P> {
    let n = geom::segments_for(r, tol);
    (0..n).map(|i| {
        let t = std::f64::consts::TAU * i as f64 / n as f64;
        [c[0] + r * t.cos(), c[1] + r * t.sin()]
    }).collect()
}

fn xy(e: &Sexp, name: &str) -> Option<P> {
    let v = e.nums(name);
    (v.len() >= 2).then(|| [v[0], v[1]])
}

/// `(pts (xy …) (arc …) …)` as KiCad points.
fn pts_of(e: &Sexp) -> Vec<P> {
    let Some(pts) = e.find("pts") else { return vec![] };
    let mut out: Vec<P> = Vec::new();
    for item in pts.args() {
        if item.is("xy") {
            if let (Some(x), Some(y)) = (item.arg_num(0), item.arg_num(1)) {
                out.push([x, y]);
            }
        } else if item.is("arc") {
            if let (Some(a), Some(m), Some(b)) = (xy(item, "start"), xy(item, "mid"), xy(item, "end")) {
                let a_ = [a[0], -a[1]];
                let m_ = [m[0], -m[1]];
                let b_ = [b[0], -b[1]];
                let mut arc: Vec<P> = arc3(a_, m_, b_, 0.01).into_iter().map(|p| [p[0], -p[1]]).collect();
                if out.last().is_some_and(|l| geom::dist(*l, arc[0]) < 1e-6) {
                    arc.remove(0);
                }
                out.extend(arc);
            }
        }
    }
    out
}

/// A graphic item (gr_* or fp_*) as polylines in KiCad coordinates; closed shapes repeat their first point.
fn graphic(e: &Sexp) -> Vec<Vec<P>> {
    let kind = e.head().unwrap_or("");
    let shape = kind.trim_start_matches("gr_").trim_start_matches("fp_");
    match shape {
        "line" => match (xy(e, "start"), xy(e, "end")) {
            (Some(a), Some(b)) => vec![vec![a, b]],
            _ => vec![],
        },
        "rect" => match (xy(e, "start"), xy(e, "end")) {
            (Some(a), Some(b)) => vec![vec![a, [b[0], a[1]], b, [a[0], b[1]], a]],
            _ => vec![],
        },
        "circle" => match (xy(e, "center"), xy(e, "end")) {
            (Some(c), Some(p)) => {
                let mut v = circle_pts(c, geom::dist(c, p), 0.01);
                v.push(v[0]);
                vec![v]
            }
            _ => vec![],
        },
        "arc" => {
            if let (Some(a), Some(m), Some(b)) = (xy(e, "start"), xy(e, "mid"), xy(e, "end")) {
                // Work y-up so the sweep direction is ordinary, then back.
                let f = |p: P| [p[0], -p[1]];
                vec![arc3(f(a), f(m), f(b), 0.01).into_iter().map(f).collect()]
            } else if let (Some(c), Some(p), Some(angle)) = (xy(e, "start"), xy(e, "end"), e.value_num("angle")) {
                // KiCad 5: start is the centre, end the arc's start, angle clockwise on screen.
                let f = |q: P| [q[0], -q[1]];
                let (cf, pf) = (f(c), f(p));
                let r = geom::dist(cf, pf);
                let a0 = (pf[1] - cf[1]).atan2(pf[0] - cf[0]);
                let span = -angle.to_radians();
                let end = [cf[0] + r * (a0 + span).cos(), cf[1] + r * (a0 + span).sin()];
                vec![arc_center(cf, r, a0, span, 0.01, pf, end).into_iter().map(f).collect()]
            } else {
                vec![]
            }
        }
        "poly" => {
            let mut v = pts_of(e);
            if v.len() >= 2 {
                v.push(v[0]);
                vec![v]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

fn layer_of(e: &Sexp) -> Option<&str> {
    e.value("layer")
}

/// Forma's copper layer name for a KiCad one.
fn cu_layer(k: &str) -> Option<&'static str> {
    match k {
        "F.Cu" => Some("top"),
        "B.Cu" => Some("bottom"),
        _ => k.strip_prefix("In").and_then(|r| r.strip_suffix(".Cu")).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=30).contains(n)).map(|n| crate::COPPER_NAMES[n]),
    }
}

/// KiCad's name for a Forma layer ("in3" → "In3.Cu", "silk_top" → "F.SilkS").
fn kicad_layer(f: &str) -> String {
    crate::rules::kicad_layer(f)
}

/// Polylines joined end to end into closed loops (ends within `tol`).
pub(crate) fn chain_loops(mut open: Vec<Vec<P>>, tol: f64) -> (Vec<Vec<P>>, usize) {
    let mut loops = Vec::new();
    let mut dangling = 0;
    while let Some(mut cur) = open.pop() {
        loop {
            if cur.len() > 2 && geom::dist(cur[0], *cur.last().unwrap()) <= tol {
                cur.pop();
                loops.push(cur);
                break;
            }
            let end = *cur.last().unwrap();
            let found = open.iter().position(|p| geom::dist(p[0], end) <= tol || geom::dist(*p.last().unwrap(), end) <= tol);
            match found {
                Some(i) => {
                    let mut next = open.swap_remove(i);
                    if geom::dist(next[0], end) > tol {
                        next.reverse();
                    }
                    cur.extend(next.into_iter().skip(1));
                }
                None => {
                    dangling += 1;
                    break;
                }
            }
        }
    }
    (loops, dangling)
}

// ---- Reading ---------------------------------------------------------------------

struct Nets(HashMap<i64, String>);

impl Nets {
    /// `(net 3 "GND")`, `(net 3)` or `(net "GND")`; empty names are no net.
    fn of(&self, e: &Sexp) -> Option<String> {
        let net = e.find("net")?;
        let a = net.args();
        let name = match a {
            [_, Sexp::Str(name), ..] | [_, Sexp::Atom(name), ..] => Some(name.clone()),
            [Sexp::Str(name)] => Some(name.clone()),
            [Sexp::Atom(code)] => code.parse::<i64>().ok().and_then(|c| self.0.get(&c).cloned()).or_else(|| Some(code.clone())),
            _ => None,
        };
        name.filter(|n| !n.is_empty())
    }
}

/// A footprint instance's placement: KiCad local ↔ board (Forma, before the shift).
#[derive(Clone, Copy)]
struct Place {
    at: P,
    rot: f64,
    bottom: bool,
}

impl Place {
    /// A KiCad footprint-local point on the board.
    fn board(&self, local: P) -> P {
        geom::add(self.at, rot([local[0], -local[1]], self.rot))
    }
    /// A board point in Forma footprint coordinates (bottom mirrored in x).
    fn forma_local(&self, p: P) -> P {
        let q = rot(geom::sub(p, self.at), -self.rot);
        r6p(if self.bottom { [-q[0], q[1]] } else { q })
    }
}

pub(crate) fn kind_for(reference: &str, value: &str) -> &'static str {
    let prefix: String = reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase();
    let v = value.to_ascii_lowercase();
    match prefix.as_str() {
        "R" | "RN" => "resistor",
        "C" => "capacitor",
        "LED" => "led",
        "D" if v.contains("led") => "led",
        "D" => "diode",
        "U" | "IC" | "Q" => "ic",
        "J" | "P" | "CN" | "CON" | "X" => "connector",
        "SW" | "S" | "BTN" => "switch",
        "H" | "MH" | "MK" => "hole",
        "M" | "MOD" => "module",
        _ => "",
    }
}

/// (height, material) of an estimated body by kind.
pub(crate) fn body_guess(kind: &str) -> (f64, &'static str) {
    match kind {
        "resistor" => (0.5, "Epoxy"),
        "capacitor" => (0.9, "Ceramic"),
        "led" => (0.8, "LED"),
        "diode" => (1.0, "Epoxy"),
        "ic" => (1.2, "Epoxy"),
        "connector" => (5.0, "Black plastic"),
        "switch" => (3.5, "Steel"),
        "module" => (3.0, "Steel"),
        _ => (1.0, "Epoxy"),
    }
}

fn bbox_of(points: impl Iterator<Item = P>) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut any = false;
    for p in points {
        any = true;
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    any.then_some(b)
}

/// A custom pad's copper in KiCad pad coordinates (y down): anchor plus primitives, unioned.
fn custom_pad_outline(pad: &Sexp, size: [f64; 2]) -> Vec<P> {
    let mut parts: Vec<CrossSection> = Vec::new();
    let poly = |pts: &[P]| -> CrossSection {
        let mut v: Vec<P> = pts.iter().map(|p| [p[0], -p[1]]).collect();
        if geom::polygon_area(&v) < 0.0 {
            v.reverse();
        }
        CrossSection::from_polygons(&[v])
    };
    let anchor_circle = pad.find("options").and_then(|o| o.value("anchor")) == Some("circle");
    if anchor_circle {
        parts.push(poly(&circle_pts([0.0, 0.0], size[0].min(size[1]) / 2.0, 0.005)));
    } else {
        let (hw, hh) = (size[0] / 2.0, size[1] / 2.0);
        parts.push(poly(&[[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]));
    }
    if let Some(prims) = pad.find("primitives") {
        for g in prims.args() {
            let width = g.find("stroke").and_then(|s| s.value_num("width")).or_else(|| g.value_num("width")).unwrap_or(0.0);
            let filled = !matches!(g.value("fill"), Some("no") | Some("none")) || g.is("gr_poly") && g.find("fill").is_none();
            let lines = graphic(g);
            for l in lines {
                let closed = l.len() > 2 && geom::dist(l[0], *l.last().unwrap()) < 1e-9;
                if closed && filled {
                    let mut cs = poly(&l[..l.len() - 1]);
                    if width > 0.0 {
                        cs = cs.offset(width / 2.0, JoinType::Round, 2.0, 16);
                    }
                    parts.push(cs);
                } else if width > 0.0 {
                    for w in l.windows(2) {
                        let a = [w[0][0], -w[0][1]];
                        let b = [w[1][0], -w[1][1]];
                        parts.push(geom::Prim::capsule(a, b, width / 2.0).section(0.0, 0.005));
                    }
                }
            }
        }
    }
    let union = CrossSection::batch_union(&parts);
    let mut contours = geom::contours(&union);
    contours.sort_by(|a, b| geom::polygon_area(b).total_cmp(&geom::polygon_area(a)));
    // Back to y down (the caller turns pad coordinates into Forma's).
    contours.into_iter().next().map(|c| c.into_iter().map(|p| [p[0], -p[1]]).collect()).unwrap_or_default()
}

/// One footprint instance: its Forma footprint (local geometry), component and notes.
fn read_footprint(e: &Sexp, nets: &Nets, layers: &[String], notes: &mut Vec<String>) -> Option<(Footprint, KComponent)> {
    let lib_id = e.arg_text(0).unwrap_or("Footprint").to_string();
    let bottom = layer_of(e) == Some("B.Cu");
    let at = e.nums("at");
    let place = Place { at: kf(*at.first().unwrap_or(&0.0), *at.get(1).unwrap_or(&0.0)), rot: norm_angle(*at.get(2).unwrap_or(&0.0)), bottom };
    let mut reference = String::new();
    let mut value = String::new();
    let mut props: BTreeMap<String, String> = BTreeMap::new();
    for p in e.find_all("property") {
        if let (Some(k), Some(v)) = (p.arg_text(0), p.arg_text(1)) {
            props.insert(k.to_string(), v.to_string());
        }
    }
    for t in e.find_all("fp_text") {
        match (t.arg_text(0), t.arg_text(1)) {
            (Some("reference"), Some(v)) => reference = v.to_string(),
            (Some("value"), Some(v)) => value = v.to_string(),
            _ => {}
        }
    }
    if let Some(r) = props.get("Reference") {
        reference = r.clone();
    }
    if let Some(v) = props.get("Value") {
        value = v.clone();
    }
    let prop = |keys: &[&str]| keys.iter().find_map(|k| props.iter().find(|(pk, v)| pk.eq_ignore_ascii_case(k) && !v.trim().is_empty()).map(|(_, v)| v.trim().to_string()));
    let side_cu = if bottom { "B.Cu" } else { "F.Cu" };
    let side_layer = if bottom { layers.last().cloned().unwrap_or_else(|| "bottom".into()) } else { "top".to_string() };
    let _ = side_layer;
    let mut pads = Vec::new();
    let mut pins = BTreeMap::new();
    // Pads with no copper (KiCad's paste windows on an exposed pad): (centre, size, rotation) in Forma footprint coordinates.
    let mut windows: Vec<(P, [f64; 2], f64)> = Vec::new();
    for pe in e.find_all("pad") {
        let number = pe.arg_text(0).unwrap_or("").to_string();
        let ptype = pe.arg_text(1).unwrap_or("smd");
        let pshape = pe.arg_text(2).unwrap_or("rect");
        let pat = pe.nums("at");
        let local = [*pat.first().unwrap_or(&0.0), *pat.get(1).unwrap_or(&0.0)];
        let abs_rot = norm_angle(*pat.get(2).unwrap_or(&0.0));
        let size = { let v = pe.nums("size"); [*v.first().unwrap_or(&1.0), *v.get(1).or(v.first()).unwrap_or(&1.0)] };
        let center = place.board(local);
        let layer_names: Vec<&str> = pe.find("layers").map(|l| l.args().iter().filter_map(|a| a.text()).collect()).unwrap_or_default();
        if ptype == "smd" && !layer_names.is_empty() && !layer_names.iter().any(|l| l.ends_with(".Cu")) {
            if layer_names.iter().any(|l| l.ends_with("Paste")) {
                let rotation = if bottom { norm_angle(place.rot - abs_rot) } else { norm_angle(abs_rot - place.rot) };
                windows.push((place.forma_local(center), size, rotation));
            }
            continue;
        }
        let kind = match ptype {
            "thru_hole" => PadKind::Tht,
            "np_thru_hole" => PadKind::Npth,
            _ => PadKind::Smd,
        };
        if ptype == "connect" {
            notes.push(format!("{reference}.{number}: a “connect” pad (no paste in KiCad) came in as SMD"));
        }
        if kind == PadKind::Smd {
            let pl: Vec<&str> = pe.find("layers").map(|l| l.args().iter().filter_map(|a| a.text()).collect()).unwrap_or_default();
            if pl.iter().any(|l| l.ends_with(".Cu") && *l != side_cu && *l != "*.Cu") {
                notes.push(format!("{reference}.{number}: an SMD pad on the other side of its footprint was put on the footprint's side"));
            }
        }
        let mut drill = None;
        let mut drill_offset: Option<[f64; 2]> = None;
        let mut center_shift = [0.0, 0.0];
        if let Some(d) = pe.find("drill") {
            let oval = d.has_atom("oval");
            let v: Vec<f64> = d.args().iter().filter_map(|a| a.num()).collect();
            if let Some(&w) = v.first() {
                drill = Some(if oval { [w, *v.get(1).unwrap_or(&w)] } else { [w, w] });
            }
            if let Some(o) = d.find("offset").map(|o| o.args().iter().filter_map(|a| a.num()).collect::<Vec<f64>>()).filter(|o| o.iter().any(|x| x.abs() > 1e-9)) {
                // KiCad moves the copper by the offset; Forma keeps the copper and moves the hole the other way.
                let (ox, oy) = (o.first().copied().unwrap_or(0.0), o.get(1).copied().unwrap_or(0.0));
                drill_offset = Some(if bottom { [r6(ox), r6(oy)] } else { [r6(-ox), r6(oy)] });
                center_shift = [ox, oy];
            }
        }
        let mut size = size;
        if kind == PadKind::Tht {
            if let Some(d) = drill {
                if d[0].min(d[1]) >= size[0].min(size[1]) {
                    size = [size[0].max(d[0] + 0.2), size[1].max(d[1] + 0.2)];
                    notes.push(format!("{reference}.{number}: a plated hole as big as its pad got a 0.1 mm ring"));
                }
            } else {
                drill = Some([size[0].min(size[1]) * 0.5; 2]);
                notes.push(format!("{reference}.{number}: a through-hole pad without a drill got one half its size"));
            }
        }
        if kind == PadKind::Npth && drill.is_none() {
            drill = Some(size);
        }
        if kind == PadKind::Smd {
            drill = None;
        }
        let (shape, corner) = match pshape {
            "rect" | "trapezoid" => (PadShape::Rect, None),
            "roundrect" => (PadShape::RoundRect, Some(pe.value_num("roundrect_rratio").unwrap_or(0.25).clamp(0.0, 0.5))),
            "circle" => (PadShape::Circle, None),
            "oval" => (PadShape::Oval, None),
            "custom" => (PadShape::Custom, None),
            _ => (PadShape::Rect, None),
        };
        let mut extra = crate::footprints::PadExtra { drill_offset, ..Default::default() };
        // Trapezoids and chamfered rectangles become their exact outline (KiCad pad frame, y down).
        let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
        let mut outline_kicad: Option<Vec<P>> = None;
        if pshape == "trapezoid" {
            let d = pe.find("rect_delta").map(|d| d.args().iter().filter_map(|a| a.num()).collect::<Vec<f64>>()).unwrap_or_default();
            let (ex, ey) = (d.first().copied().unwrap_or(0.0) / 2.0, d.get(1).copied().unwrap_or(0.0) / 2.0);
            outline_kicad = Some(vec![[-hx - ey, hy + ex], [-hx + ey, -hy - ex], [hx - ey, -hy + ex], [hx + ey, hy - ex]]);
            extra.trapezoid = Some([r6(2.0 * ex), r6(2.0 * ey)]);
        }
        let ratio = pe.value_num("chamfer_ratio").unwrap_or(0.0);
        if let Some(ch) = pe.find("chamfer").filter(|_| ratio > 0.0 && matches!(pshape, "rect" | "roundrect")) {
            let corners: Vec<String> = ch.args().iter().filter_map(|a| a.text()).map(str::to_string).collect();
            let k = ratio.clamp(0.0, 0.5) * size[0].min(size[1]);
            let has = |n: &str| corners.iter().any(|c| c == n);
            let mut q: Vec<P> = Vec::new();
            // KiCad frame (y down): top is −y.
            for (name, c, a, b) in [
                ("top_left", [-hx, -hy], [-hx, -hy + k], [-hx + k, -hy]),
                ("top_right", [hx, -hy], [hx - k, -hy], [hx, -hy + k]),
                ("bottom_right", [hx, hy], [hx, hy - k], [hx - k, hy]),
                ("bottom_left", [-hx, hy], [-hx + k, hy], [-hx, hy - k]),
            ] {
                if has(name) {
                    q.push(a);
                    q.push(b);
                } else {
                    q.push(c);
                }
            }
            outline_kicad = Some(q);
            let flip = |c: &String| if bottom { match c.as_str() { "top_left" => "top_right".into(), "top_right" => "top_left".into(), "bottom_left" => "bottom_right".into(), "bottom_right" => "bottom_left".into(), x => x.into() } } else { c.clone() };
            extra.chamfer = Some(crate::footprints::Chamfer { ratio, corners: corners.iter().map(flip).collect() });
            if pshape == "roundrect" {
                notes.push(format!("{reference}.{number}: a rounded rectangle with chamfers came in with sharp corners where not chamfered"));
            }
        }
        let mut polygon = Vec::new();
        let mut psize = size;
        let shape = if outline_kicad.is_some() { PadShape::Custom } else { shape };
        let corner = if outline_kicad.is_some() { None } else { corner };
        if let Some(outline) = outline_kicad {
            let mut q: Vec<P> = outline.into_iter().map(|p| r6p(if bottom { [-p[0], -p[1]] } else { [p[0], -p[1]] })).collect();
            if geom::polygon_area(&q) < 0.0 {
                q.reverse();
            }
            polygon = q;
        } else if shape == PadShape::Custom {
            let outline = custom_pad_outline(pe, size);
            // Pad coordinates (y down) → Forma pad coordinates: y up, mirrored on the bottom.
            let mut q: Vec<P> = outline.into_iter().map(|p| r6p(if bottom { [-p[0], -p[1]] } else { [p[0], -p[1]] })).collect();
            if geom::polygon_area(&q) < 0.0 {
                q.reverse();
            }
            if let Some(b) = bbox_of(q.iter().copied()) {
                psize = [r6(b[2] - b[0]), r6(b[3] - b[1])];
            }
            polygon = q;
        }
        let rotation = if bottom { norm_angle(place.rot - abs_rot) } else { norm_angle(abs_rot - place.rot) };
        let net = nets.of(pe);
        if let Some(net) = &net {
            if !number.is_empty() && kind != PadKind::Npth {
                pins.insert(number.clone(), net.clone());
            }
        }
        // The copper sits at the hole plus the offset turned with the pad (KiCad frame, y down).
        let copper = if center_shift == [0.0, 0.0] {
            center
        } else {
            let (s, c) = (abs_rot - place.rot).to_radians().sin_cos();
            let (ox, oy) = (center_shift[0], center_shift[1]);
            place.board([local[0] + ox * c + oy * s, local[1] - ox * s + oy * c])
        };
        pads.push(Pad { number, kind, shape, size: [r6(psize[0]), r6(psize[1])], at: place.forma_local(copper), rotation, drill: drill.map(|d| [r6(d[0]), r6(d[1])]), corner, paste: vec![], polygon, extra });
    }
    if pads.is_empty() {
        // Logos and graphics: nothing to place.
        return None;
    }
    // Paste windows go to the copper pad under them (sizes in that pad's frame).
    for (c, size, rotation) in windows {
        let host = pads.iter_mut().filter(|p: &&mut Pad| p.kind == PadKind::Smd).find(|p| {
            let b = crate::footprints::pad_prim(p, [0.0, 0.0], 0.0, false).bbox();
            c[0] >= b[0] - 1e-6 && c[0] <= b[2] + 1e-6 && c[1] >= b[1] - 1e-6 && c[1] <= b[3] + 1e-6
        });
        match host {
            Some(p) => {
                let quarter = (((rotation - p.rotation) / 90.0).round() as i64).rem_euclid(2) == 1;
                let (w, h) = if quarter { (size[1], size[0]) } else { (size[0], size[1]) };
                p.paste.push([r6(c[0]), r6(c[1]), r6(w), r6(h)]);
            }
            None => notes.push(format!("{reference}: a paste opening at ({:.2}, {:.2}) with no pad under it was left out", c[0], c[1])),
        }
    }
    // Silkscreen, courtyard and fabrication outline, in Forma footprint coordinates.
    let silk_layer = if bottom { "B.SilkS" } else { "F.SilkS" };
    let crt_layer = if bottom { "B.CrtYd" } else { "F.CrtYd" };
    let fab_layer = if bottom { "B.Fab" } else { "F.Fab" };
    let mut silk: Vec<Vec<P>> = Vec::new();
    let mut court: Vec<P> = Vec::new();
    let mut fab: Vec<P> = Vec::new();
    for g in e.args() {
        let Some(h) = g.head() else { continue };
        if !h.starts_with("fp_") || h == "fp_text" {
            continue;
        }
        let layer = layer_of(g).unwrap_or("");
        let lines = graphic(g);
        let to_local = |l: &Vec<P>| -> Vec<P> { l.iter().map(|p| place.forma_local(place.board(*p))).collect() };
        if layer == silk_layer || layer == "F.SilkS" && !bottom || layer == "F.Silkscreen" || layer == "B.Silkscreen" && bottom {
            for l in &lines {
                silk.push(to_local(l));
            }
        } else if layer == crt_layer || layer == "F.Courtyard" && !bottom || layer == "B.Courtyard" && bottom {
            court.extend(lines.iter().flat_map(to_local));
        } else if layer == fab_layer || layer == "F.Fabrication" && !bottom || layer == "B.Fabrication" && bottom {
            fab.extend(lines.iter().flat_map(to_local));
        }
    }
    let kind = kind_for(&reference, &value).to_string();
    let courtyard = bbox_of(court.into_iter()).map(|b| [r6(b[0]), r6(b[1]), r6(b[2]), r6(b[3])]);
    let mut body = Vec::new();
    if kind != "hole" {
        let b = bbox_of(fab.into_iter()).or_else(|| bbox_of(pads.iter().flat_map(|p| {
            let pr = crate::footprints::pad_prim(p, [0.0, 0.0], 0.0, false).bbox();
            [[pr[0], pr[1]], [pr[2], pr[3]]]
        })));
        if let Some(b) = b {
            let (h, material) = body_guess(&kind);
            body.push(BodyPart { shape: BodyShape::Box, at: r6p([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]), size: [r6(b[2] - b[0]), r6(b[3] - b[1])], z: [0.0, h], material: material.into() });
        }
    }
    let name = lib_id.rsplit(':').next().unwrap_or(&lib_id).to_string();
    let description = e.value("descr").unwrap_or("").to_string();
    let fp = Footprint {
        name: name.clone(),
        description,
        source: format!("Imported from KiCad {lib_id}; body from the fabrication outline, height estimated"),
        pads,
        courtyard,
        silk,
        body,
        kind,
    };
    let dnp = e.find("attr").is_some_and(|a| a.has_atom("dnp"));
    let comp = KComponent {
        reference,
        value,
        footprint: name,
        at: place.at,
        rotation: place.rot,
        side: if bottom { Side::Bottom } else { Side::Top },
        pins,
        dnp,
        mpn: prop(&["MPN", "Manufacturer Part Number", "MFR Part", "PartNumber"]),
        manufacturer: prop(&["Manufacturer", "MFR"]),
        lcsc: prop(&["LCSC", "LCSC Part", "LCSC Part #", "JLCPCB Part"]),
        description: prop(&["Description"]).unwrap_or_default(),
    };
    Some((fp, comp))
}

/// Track segments of one net, layer and width joined into polylines.
pub(crate) fn chain_tracks(segs: Vec<(Option<String>, String, f64, P, P)>) -> Vec<KTrack> {
    let mut groups: BTreeMap<(String, String, i64), Vec<(P, P)>> = BTreeMap::new();
    let mut net_of: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (net, layer, width, a, b) in segs {
        let key = net.clone().unwrap_or_default();
        net_of.insert(key.clone(), net);
        groups.entry((key, layer, (width * 1e6).round() as i64)).or_default().push((a, b));
    }
    let k = |p: P| ((p[0] * 1e4).round() as i64, (p[1] * 1e4).round() as i64);
    let mut out = Vec::new();
    for ((net, layer, w), segs) in groups {
        let mut by_point: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, (a, b)) in segs.iter().enumerate() {
            by_point.entry(k(*a)).or_default().push(i);
            by_point.entry(k(*b)).or_default().push(i);
        }
        let mut used = vec![false; segs.len()];
        // Start at ends (points used once) so chains are as long as possible, then whatever is left.
        let mut starts: Vec<usize> = (0..segs.len()).filter(|i| by_point[&k(segs[*i].0)].len() == 1 || by_point[&k(segs[*i].1)].len() == 1).collect();
        starts.extend(0..segs.len());
        for s0 in starts {
            if used[s0] {
                continue;
            }
            used[s0] = true;
            let (a, b) = segs[s0];
            let mut pts = if by_point[&k(a)].len() == 1 { vec![a, b] } else { vec![b, a] };
            loop {
                let end = *pts.last().unwrap();
                let next = by_point[&k(end)].iter().copied().find(|i| !used[*i]);
                let Some(i) = next else { break };
                used[i] = true;
                let (a, b) = segs[i];
                pts.push(if k(a) == k(end) { b } else { a });
            }
            out.push(KTrack { net: net_of[&net].clone(), layer: layer.clone(), width: w as f64 / 1e6, points: pts });
        }
    }
    out
}

/// Reads a `.kicad_pcb` file into Forma's terms.
/// A library footprint file (`.kicad_mod`: KiCad 6–10, or a KiCad 5 `module`)
/// in Forma's terms: pads exactly as drawn (y flipped up), silkscreen,
/// courtyard and a body box from the fabrication outline, with notes on
/// anything approximated. The kind and body height are guessed from the
/// library name (`Resistor_SMD`, `Package_SO`, `Connector_JST` …) when given.
pub fn read_footprint_file(text: &str, library: Option<&str>) -> Result<(Footprint, Vec<String>), String> {
    let top = sexpr::parse(text)?;
    let e = top.iter().find(|e| e.is("footprint") || e.is("module")).ok_or("not a KiCad footprint: the file does not start with (footprint …)")?;
    let mut notes = Vec::new();
    let (mut fp, _) = read_footprint(e, &Nets(HashMap::new()), &[], &mut notes).ok_or("the footprint has no pads")?;
    let lib = library.unwrap_or("");
    let kind = library_kind(lib, &fp.name);
    if !kind.is_empty() && kind != fp.kind {
        fp.kind = kind.to_string();
        let (h, material) = body_guess(kind);
        for b in &mut fp.body {
            b.z = [0.0, h];
            b.material = material.into();
        }
        if kind == "hole" {
            fp.body.clear();
        }
    }
    if let Some(t) = e.value("tags").filter(|t| !t.is_empty()) {
        fp.description = if fp.description.is_empty() { t.to_string() } else { format!("{} ({t})", fp.description) };
    }
    Ok((fp, notes))
}

/// What a KiCad library holds, by its name.
fn library_kind(lib: &str, name: &str) -> &'static str {
    let l = lib.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    if l.starts_with("resistor") {
        "resistor"
    } else if l.starts_with("capacitor") {
        "capacitor"
    } else if l.starts_with("led") || n.starts_with("led_") {
        "led"
    } else if l.starts_with("diode") {
        "diode"
    } else if l.starts_with("connector") || l.starts_with("terminalblock") {
        "connector"
    } else if l.starts_with("button_switch") {
        "switch"
    } else if l.starts_with("mountinghole") {
        "hole"
    } else if l.starts_with("rf_module") || l.starts_with("module") {
        "module"
    } else if l.starts_with("package") || l.starts_with("crystal") || l.starts_with("inductor") || l.starts_with("oscillator") || l.starts_with("sensor") {
        "ic"
    } else {
        ""
    }
}

pub fn parse_board(text: &str) -> Result<KicadBoard, String> {
    let top = sexpr::parse(text)?;
    let root = top.iter().find(|e| e.is("kicad_pcb")).ok_or("not a KiCad board: the file does not start with (kicad_pcb …)")?;
    let mut notes = Vec::new();
    // Copper layers by name.
    let mut cu: Vec<String> = Vec::new();
    if let Some(ls) = root.find("layers") {
        for l in ls.args() {
            if let Sexp::List(v) = l {
                if let Some(name) = v.get(1).and_then(|x| x.text()) {
                    if name.ends_with(".Cu") {
                        cu.push(name.to_string());
                    }
                }
            }
        }
    }
    if cu.is_empty() {
        cu = vec!["F.Cu".into(), "B.Cu".into()];
    }
    if cu.len() > 32 {
        return Err(format!("the board has {} copper layers; Forma makes 2 to 32", cu.len()));
    }
    let layers: u8 = (cu.len().max(2) + cu.len() % 2) as u8;
    if cu.len() % 2 == 1 {
        notes.push(format!("{} copper layers became a {layers}-layer stack", cu.len()));
    }
    let forma_layers: Vec<String> = std::iter::once("top".to_string()).chain(crate::COPPER_NAMES[1..layers as usize - 1].iter().map(|s| s.to_string())).chain(std::iter::once("bottom".to_string())).collect();
    let thickness = root.find("general").and_then(|g| g.value_num("thickness")).unwrap_or(1.6);
    let mut nets = Nets(HashMap::new());
    for e in root.find_all("net") {
        if let (Some(code), Some(name)) = (e.arg_num(0), e.arg_text(1)) {
            nets.0.insert(code as i64, name.to_string());
        }
    }
    let title = root.find("title_block").and_then(|t| t.value("title")).map(str::to_string).filter(|t| !t.trim().is_empty());
    let revision = root.find("title_block").and_then(|t| t.value("rev")).map(str::to_string).filter(|t| !t.trim().is_empty());
    let mask_expansion = root.find("setup").and_then(|s| s.value_num("pad_to_mask_clearance"));

    // Outline: Edge.Cuts graphics on the board and in footprints, in Forma coordinates.
    let mut open: Vec<Vec<P>> = Vec::new();
    let mut closed: Vec<Vec<P>> = Vec::new();
    let mut add_edge = |lines: Vec<Vec<P>>, map: &dyn Fn(P) -> P| {
        for l in lines {
            let l: Vec<P> = l.into_iter().map(map).collect();
            if l.len() > 2 && geom::dist(l[0], *l.last().unwrap()) < 1e-9 {
                closed.push(l[..l.len() - 1].to_vec());
            } else if l.len() >= 2 {
                open.push(l);
            }
        }
    };
    for g in root.args() {
        if g.head().is_some_and(|h| h.starts_with("gr_")) && layer_of(g) == Some("Edge.Cuts") {
            add_edge(graphic(g), &|p| kf(p[0], p[1]));
        }
    }
    let fps: Vec<&Sexp> = root.args().iter().filter(|e| e.is("footprint") || e.is("module")).collect();
    for f in &fps {
        let at = f.nums("at");
        let place = Place { at: kf(*at.first().unwrap_or(&0.0), *at.get(1).unwrap_or(&0.0)), rot: *at.get(2).unwrap_or(&0.0), bottom: layer_of(f) == Some("B.Cu") };
        for g in f.args() {
            if g.head().is_some_and(|h| h.starts_with("fp_")) && layer_of(g) == Some("Edge.Cuts") {
                add_edge(graphic(g), &|p| place.board(p));
            }
        }
    }
    let (mut loops, dangling) = chain_loops(open, 1e-3);
    if dangling > 0 {
        notes.push(format!("{dangling} Edge.Cuts line{} did not close into an outline and {} left out", if dangling == 1 { "" } else { "s" }, if dangling == 1 { "was" } else { "were" }));
    }
    loops.extend(closed);
    loops.retain(|l| l.len() >= 3 && geom::polygon_area(l).abs() > 1e-6);
    if loops.is_empty() {
        return Err("no board outline: nothing on Edge.Cuts closes into a loop".into());
    }
    loops.sort_by(|a, b| geom::polygon_area(b).abs().total_cmp(&geom::polygon_area(a).abs()));
    let mut outline = loops.remove(0);
    if geom::polygon_area(&outline) < 0.0 {
        outline.reverse();
    }
    let mut cutouts: Vec<Vec<P>> = loops.into_iter().filter(|l| geom::in_polygon(l[0], &outline)).collect();
    for c in &mut cutouts {
        if geom::polygon_area(c) > 0.0 {
            c.reverse();
        }
    }
    let shift = bbox_of(outline.iter().copied()).map(|b| [b[0], b[1]]).unwrap();
    let mv = |p: P| r6p(geom::sub(p, shift));

    // Footprints and components.
    let mut footprints: BTreeMap<String, Footprint> = BTreeMap::new();
    let mut components = Vec::new();
    let mut refs_seen: HashMap<String, usize> = HashMap::new();
    for f in &fps {
        let Some((mut fp, mut c)) = read_footprint(f, &nets, &forma_layers, &mut notes) else { continue };
        // A footprint already defined the same way is reused; a different one under the same name gets a suffix.
        let base = fp.name.clone();
        let mut name = base.clone();
        let mut k = 1;
        loop {
            match footprints.get(&name) {
                None => {
                    fp.name = name.clone();
                    footprints.insert(name.clone(), fp);
                    break;
                }
                Some(existing) if *existing == Footprint { name: name.clone(), ..fp.clone() } => break,
                Some(_) => {
                    k += 1;
                    name = format!("{base}_{k}");
                }
            }
        }
        c.footprint = name;
        c.at = mv(c.at);
        // References: unique, starting with a letter, no spaces, at most 16 characters.
        let mut r: String = c.reference.trim().chars().filter(|ch| !ch.is_whitespace()).collect();
        if !r.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic()) {
            r = format!("X{r}");
        }
        r.truncate(14);
        let count = refs_seen.entry(r.to_ascii_uppercase()).or_insert(0);
        *count += 1;
        if *count > 1 {
            let renamed = format!("{r}_{count}");
            notes.push(format!("{} appears more than once: renamed {renamed}", c.reference));
            r = renamed;
        }
        if r != c.reference && !c.reference.is_empty() && *count == 1 {
            notes.push(format!("reference “{}” became {r}", c.reference));
        }
        c.reference = r;
        components.push(c);
    }

    // Tracks and vias.
    let mut segs = Vec::new();
    let mut vias = Vec::new();
    let mut via_specs = Vec::new();
    let mut skipped_layers = 0;
    for e in root.args() {
        if e.is("segment") || e.is("arc") {
            let Some(layer) = layer_of(e).and_then(cu_layer) else {
                skipped_layers += 1;
                continue;
            };
            if layers == 2 && layer.starts_with("in") {
                skipped_layers += 1;
                continue;
            }
            let width = e.value_num("width").unwrap_or(0.25);
            let net = nets.of(e);
            let (Some(a), Some(b)) = (xy(e, "start"), xy(e, "end")) else { continue };
            let pts: Vec<P> = if e.is("arc") {
                match xy(e, "mid") {
                    Some(m) => arc3(kf(a[0], a[1]), kf(m[0], m[1]), kf(b[0], b[1]), 0.01),
                    None => vec![kf(a[0], a[1]), kf(b[0], b[1])],
                }
            } else {
                vec![kf(a[0], a[1]), kf(b[0], b[1])]
            };
            for w in pts.windows(2) {
                if geom::dist(w[0], w[1]) > 1e-9 {
                    segs.push((net.clone(), layer.to_string(), width, mv(w[0]), mv(w[1])));
                }
            }
        } else if e.is("via") {
            let Some(p) = xy(e, "at") else { continue };
            let diameter = e.value_num("size").unwrap_or(0.6);
            let drill = e.value_num("drill").unwrap_or(diameter / 2.0);
            if e.has_atom("blind") || e.has_atom("micro") {
                let ls: Vec<&'static str> = e.find("layers").map(|l| l.args().iter().filter_map(|a| a.text()).filter_map(cu_layer).collect()).unwrap_or_default();
                if ls.len() == 2 {
                    let outer = |l: &str| l == "top" || l == "bottom";
                    let kind = if e.has_atom("micro") { crate::vias::ViaKind::Micro } else if outer(ls[0]) || outer(ls[1]) { crate::vias::ViaKind::Blind } else { crate::vias::ViaKind::Buried };
                    via_specs.push((vias.len(), crate::vias::ViaSpec { kind, from: Some(ls[0].to_string()), to: Some(ls[1].to_string()), ..Default::default() }));
                } else {
                    notes.push("a blind or micro via with unknown layers came in as a through via".into());
                }
            }
            vias.push(KVia { net: nets.of(e), at: mv(kf(p[0], p[1])), drill, diameter });
        }
    }
    if skipped_layers > 0 {
        notes.push(format!("{skipped_layers} track segment(s) on layers Forma does not have were left out"));
    }
    if root.args().iter().any(|e| e.is("arc")) {
        notes.push("arc tracks came in as short straight segments (within 0.01 mm)".into());
    }
    let tracks = chain_tracks(segs);

    // Pours.
    let mut zones = Vec::new();
    let mut areas = Vec::new();
    let mut had_teardrops = false;
    for z in root.find_all("zone") {
        if let Some(k) = z.find("keepout") {
            let no = |what: &str| k.value(what) == Some("not_allowed");
            let mut ls: Vec<String> = z.value("layer").into_iter().chain(z.find("layers").map(|l| l.args().iter().filter_map(|a| a.text()).collect::<Vec<_>>()).unwrap_or_default()).flat_map(|l| if l.starts_with('*') || l.starts_with("F&B") { forma_layers.clone() } else { cu_layer(l).map(|x| vec![x.to_string()]).unwrap_or_default() }).collect();
            ls.dedup();
            let poly: Vec<P> = z.find("polygon").map(pts_of).unwrap_or_default().into_iter().map(|p| mv(kf(p[0], p[1]))).collect();
            if poly.len() >= 3 {
                areas.push(KArea { name: z.value("name").map(str::to_string).unwrap_or_else(|| format!("Keepout {}", areas.len() + 1)), outline: poly, layers: if ls.len() == forma_layers.len() { vec![] } else { ls }, no: [no("tracks"), no("vias"), no("pads"), no("copperpour"), no("footprints")] });
            }
            continue;
        }
        if z.find("attr").is_some_and(|a| a.find("teardrop").is_some()) {
            had_teardrops = true;
            continue; // teardrops are worked out from the board's settings
        }
        let net = z.value("net_name").map(str::to_string).filter(|s| !s.is_empty()).or_else(|| nets.of(z));
        let Some(net) = net else {
            notes.push("a pour with no net was left out".into());
            continue;
        };
        let mut zl: Vec<&'static str> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        if let Some(l) = z.value("layer") {
            names.push(l.to_string());
        }
        if let Some(ls) = z.find("layers") {
            names.extend(ls.args().iter().filter_map(|a| a.text()).map(str::to_string));
        }
        for n in names {
            match n.as_str() {
                "*.Cu" => zl.extend(forma_layers.iter().map(|l| crate::COPPER_NAMES.iter().find(|x| **x == l.as_str()).copied().unwrap_or("top"))),
                "F&B.Cu" => zl.extend(["top", "bottom"]),
                other => {
                    if let Some(l) = cu_layer(other) {
                        zl.push(l);
                    }
                }
            }
        }
        zl.dedup();
        let poly: Vec<P> = z.find("polygon").map(pts_of).unwrap_or_default().into_iter().map(|p| mv(kf(p[0], p[1]))).collect();
        if poly.len() < 3 {
            continue;
        }
        let cp = z.find("connect_pads");
        let connection = match cp.and_then(|c| c.args().first()).and_then(|a| if let Sexp::Atom(s) = a { Some(s.as_str()) } else { None }) {
            Some("yes") => PadConnection::Solid,
            Some("no") => PadConnection::None,
            Some("thru_hole_only") => PadConnection::ThermalTht,
            _ => PadConnection::Thermal,
        };
        let clearance = cp.and_then(|c| c.value_num("clearance"));
        let fill = z.find("fill");
        for l in zl {
            if layers == 2 && l.starts_with("in") {
                continue;
            }
            zones.push(KZone {
                net: net.clone(),
                layer: l.to_string(),
                outline: poly.clone(),
                clearance,
                min_width: z.value_num("min_thickness").unwrap_or(0.25),
                priority: z.value_num("priority").unwrap_or(0.0) as i32,
                connection,
                thermal_gap: fill.and_then(|f| f.value_num("thermal_gap")),
                thermal_spoke: fill.and_then(|f| f.value_num("thermal_bridge_width")),
            });
        }
    }

    // A Default class from what the board uses most; rules relaxed to admit what it has.
    let mode = |vals: Vec<f64>| -> Option<f64> {
        let mut m: BTreeMap<i64, usize> = BTreeMap::new();
        for v in vals {
            *m.entry((v * 1e4).round() as i64).or_default() += 1;
        }
        m.into_iter().max_by_key(|(_, c)| *c).map(|(v, _)| v as f64 / 1e4)
    };
    let mut class = NetClass::default_class();
    if let Some(w) = mode(tracks.iter().map(|t| t.width).collect()) {
        class.track_width = w;
    }
    if let Some(d) = mode(vias.iter().map(|v| v.diameter).collect()) {
        class.via_diameter = d;
        if let Some(dr) = mode(vias.iter().filter(|v| (v.diameter - d).abs() < 1e-6).map(|v| v.drill).collect()) {
            class.via_drill = dr;
        }
    }
    if class.via_diameter <= class.via_drill {
        class.via_diameter = class.via_drill + 0.2;
    }
    notes.push(format!("net classes live in KiCad's .kicad_pro: a Default class was made from the board ({} mm tracks, {}/{} mm vias, 0.2 mm clearance)", class.track_width, class.via_diameter, class.via_drill));
    let mut rules = Rules::default();
    if let Some(m) = mask_expansion {
        rules.mask_expansion = m.clamp(0.0, 10.0);
    }
    let min_track = tracks.iter().map(|t| t.width).fold(f64::INFINITY, f64::min);
    if min_track < rules.min_track {
        rules.min_track = min_track;
    }
    let min_drill = vias.iter().map(|v| v.drill).chain(footprints.values().flat_map(|f| f.pads.iter().filter(|p| p.kind == PadKind::Tht).filter_map(|p| p.drill.map(|d| d[0].min(d[1]))))).fold(f64::INFINITY, f64::min);
    if min_drill < rules.min_drill {
        rules.min_drill = min_drill;
    }
    let min_ring = vias.iter().map(|v| (v.diameter - v.drill) / 2.0).fold(f64::INFINITY, f64::min);
    if min_ring < rules.min_annular {
        rules.min_annular = min_ring.max(0.0);
    }

    Ok(KicadBoard {
        title,
        revision,
        layers,
        thickness,
        outline: outline.into_iter().map(mv).collect(),
        cutouts: cutouts.into_iter().map(|c| c.into_iter().map(mv).collect()).collect(),
        footprints,
        components,
        tracks,
        vias,
        zones,
        default_class: class,
        rules,
        notes,
        shift,
        areas,
        via_specs,
        teardrops: had_teardrops,
    })
}

// ---- The command -------------------------------------------------------------------

/// `pcb.board.import_kicad`: a `.kicad_pcb` file's text becomes a Forma board.
#[derive(Deserialize, JsonSchema)]
pub struct ImportKicad {
    /// The whole `.kicad_pcb` file.
    pub text: String,
    /// Board name (default: the file's title, else "Imported board").
    pub name: Option<String>,
    /// Where the 3D board sits in the project (mm).
    pub at: Option<[f64; 3]>,
}

impl Command for ImportKicad {
    const NAME: &'static str = "pcb.board.import_kicad";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let k = parse_board(&self.text).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?;
        create_board(tx, &k, self.name, self.at)
    }
}

/// A board read from any file (KiCad or another tool's, in Forma's terms) written into the
/// project in one change: the board, its footprints, components, tracks, vias, pours and 3D part.
pub fn create_board(tx: &mut Tx, k: &KicadBoard, name: Option<String>, at: Option<[f64; 3]>) -> Result<Value, CommandError> {
    let name = name.filter(|n| !n.trim().is_empty()).or(k.title.clone()).unwrap_or_else(|| "Imported board".into()).trim().to_string();
    for (n, f) in &k.footprints {
        for p in &f.pads {
            if !(p.size[0] > 0.0 && p.size[1] > 0.0 && p.size[0] < 100.0 && p.size[1] < 100.0) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, format!("footprint {n}: pad {} has no size", p.number)));
            }
        }
    }
    let board = Board {
        name: name.clone(),
        outline: k.outline.clone(),
        corner_radius: 0.0,
        cutouts: k.cutouts.clone(),
        sketch: None,
        layers: k.layers,
        thickness: k.thickness.clamp(0.2, 8.0),
        finish: "HASL lead-free".into(),
        copper_oz: 1.0,
        inner_copper_oz: 0.5,
        mask: "green".into(),
        silk: "white".into(),
        rules: k.rules.clone(),
        net_classes: vec![k.default_class.clone()],
        footprints: k.footprints.clone(),
        model: None,
        revision: k.revision.clone().unwrap_or_else(|| "A".into()),
        quantity: 5,
        fab: None,
        sourcing: BTreeMap::new(),
        stackup: None,
        drc_rules: String::new(),
        teardrops: k.teardrops.then(crate::teardrops::Teardrops::default),
        signal: Default::default(),
    };
    let id = tx.create();
    tx.set(id, board)?;
    for c in &k.components {
        let cid = tx.create();
        tx.set(
            cid,
            PcbComponent {
                board: id,
                reference: c.reference.clone(),
                value: c.value.clone(),
                footprint: c.footprint.clone(),
                at: c.at,
                rotation: c.rotation,
                side: c.side,
                mpn: c.mpn.clone(),
                manufacturer: c.manufacturer.clone(),
                lcsc: c.lcsc.clone(),
                description: c.description.clone(),
                pins: c.pins.clone(),
                pin_names: BTreeMap::new(),
                dnp: c.dnp,
            },
        )?;
        tx.add_dependency(id, cid)?;
    }
    for t in &k.tracks {
        let tid = tx.create();
        tx.set(tid, Track { board: id, net: t.net.clone(), layer: t.layer.clone(), width: t.width, points: t.points.clone(), auto: false, reroute: false, bulges: vec![] })?;
        tx.add_dependency(id, tid)?;
    }
    for (i, v) in k.vias.iter().enumerate() {
        let vid = tx.create();
        let spec = k.via_specs.iter().find(|(j, _)| *j == i).map(|(_, s)| s.clone()).unwrap_or_default();
        tx.set(vid, Via { board: id, net: v.net.clone(), at: v.at, drill: v.drill, diameter: v.diameter, auto: false, spec })?;
        tx.add_dependency(id, vid)?;
    }
    for a in &k.areas {
        let aid = tx.create();
        tx.set(aid, crate::areas::RuleArea { board: id, name: a.name.clone(), outline: a.outline.clone(), layers: a.layers.clone(), no_tracks: a.no[0], no_vias: a.no[1], no_pads: a.no[2], no_pours: a.no[3], no_footprints: a.no[4], placement: vec![] })?;
        tx.add_dependency(id, aid)?;
    }
    for z in &k.zones {
        let zid = tx.create();
        tx.set(
            zid,
            Zone {
                board: id,
                net: z.net.clone(),
                layer: z.layer.clone(),
                outline: z.outline.clone(),
                clearance: z.clearance,
                min_width: z.min_width,
                priority: z.priority,
                remove_islands: true,
                connection: z.connection,
                thermal_gap: z.thermal_gap,
                thermal_spoke: z.thermal_spoke,
                hatch: None,
                cutouts: vec![],
                min_island_area: None,
                name: None,
            },
        )?;
        tx.add_dependency(id, zid)?;
    }
    let model = crate::model3d::create(tx, id, at.unwrap_or([0.0; 3]))?;
    Ok(json!({
        "id": id,
        "model": model,
        "name": name,
        "layers": k.layers,
        "components": k.components.len(),
        "footprints": k.footprints.len(),
        "tracks": k.tracks.len(),
        "vias": k.vias.len(),
        "zones": k.zones.len(),
        "notes": k.notes,
    }))
}

// ---- Writing ------------------------------------------------------------------------

/// A stable RFC 4122-shaped id from a seed (same board, same ids).
pub(crate) fn uuid(seed: &str) -> String {
    let h = |salt: u64| {
        let mut x = std::collections::hash_map::DefaultHasher::new();
        salt.hash(&mut x);
        seed.hash(&mut x);
        x.finish()
    };
    let (a, b) = (h(0x9e37), h(0x7f4a));
    let b = (b & 0x3fff_ffff_ffff_ffff) | 0x8000_0000_0000_0000;
    let a = (a & 0xffff_ffff_ffff_0fff) | 0x0000_0000_0000_4000;
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", a >> 32, (a >> 16) & 0xffff, a & 0xffff, b >> 48, b & 0xffff_ffff_ffff)
}

fn u(seed: &str) -> Sexp {
    list("uuid", vec![s(uuid(seed))])
}

/// Forma board point → KiCad page point.
struct Page {
    ox: f64,
    oy: f64,
}

impl Page {
    fn k(&self, p: P) -> P {
        [p[0] + self.ox, self.oy - p[1]]
    }
    fn xy(&self, head: &str, p: P) -> Sexp {
        let q = self.k(p);
        list(head, vec![n(q[0]), n(q[1])])
    }
}

/// A board point in KiCad footprint-local coordinates (y down) for a footprint at `at`, turned `rot`.
fn k_local(at: P, rot_deg: f64, p: P) -> P {
    let q = rot(geom::sub(p, at), -rot_deg);
    [q[0], -q[1]]
}

fn at3(head: &str, p: P, angle: f64) -> Sexp {
    let a = norm_angle(angle);
    if a == 0.0 { list(head, vec![n(p[0]), n(p[1])]) } else { list(head, vec![n(p[0]), n(p[1]), n(a)]) }
}

fn effects(mirror: bool, hide: bool) -> Sexp {
    let mut v = vec![list("font", vec![list("size", vec![n(1.0), n(1.0)]), list("thickness", vec![n(0.15)])])];
    if mirror {
        v.push(list("justify", vec![atom("mirror")]));
    }
    let _ = hide;
    list("effects", v)
}

fn stroke(w: f64) -> Sexp {
    list("stroke", vec![list("width", vec![n(w)]), list("type", vec![atom("solid")])])
}

/// The board as a KiCad 8 `.kicad_pcb` file (pours with their default settings).
pub fn export(view: &BoardView, board: &Board) -> String {
    export_with(view, board, &BTreeMap::new())
}

/// The board as a KiCad 8 file, pours with the settings of `zones` (by id) where given.
pub fn export_world(world: &de_core::bevy_ecs::world::World, id: StableId) -> Option<String> {
    let e = world.resource::<de_core::IdIndex>().get(id)?;
    let board = world.get::<Board>(e)?.clone();
    let view = crate::view(world, id)?;
    let zones: BTreeMap<StableId, Zone> = crate::items(world, id).zones.into_iter().collect();
    Some(export_with(&view, &board, &zones))
}

pub fn export_with(view: &BoardView, board: &Board, zones: &BTreeMap<StableId, Zone>) -> String {
    let page = Page { ox: 50.0 - view.bounds[0], oy: 50.0 + view.bounds[3] };
    let four = view.layers.len() >= 4;
    let mut items: Vec<Sexp> = vec![
        list("version", vec![atom("20240108")]),
        list("generator", vec![s("forma")]),
        list("generator_version", vec![s("8.0")]),
        list("general", vec![list("thickness", vec![n(view.thickness)]), list("legacy_teardrops", vec![atom("no")])]),
        list("paper", vec![s("A4")]),
        list("title_block", vec![list("title", vec![s(view.name.clone())]), list("rev", vec![s(view.revision.clone())]), list("company", vec![s("Made with Forma")])]),
    ];
    let mut layer_table: Vec<Sexp> = vec![Sexp::List(vec![atom("0"), s("F.Cu"), atom("signal")])];
    let _ = four;
    for k in 1..view.layers.len().saturating_sub(1) {
        layer_table.push(Sexp::List(vec![atom(k.to_string()), s(format!("In{k}.Cu")), atom("signal")]));
    }
    layer_table.push(Sexp::List(vec![atom("31"), s("B.Cu"), atom("signal")]));
    for (k, name, user) in [
        (32, "B.Adhes", Some("B.Adhesive")),
        (33, "F.Adhes", Some("F.Adhesive")),
        (34, "B.Paste", None),
        (35, "F.Paste", None),
        (36, "B.SilkS", Some("B.Silkscreen")),
        (37, "F.SilkS", Some("F.Silkscreen")),
        (38, "B.Mask", None),
        (39, "F.Mask", None),
        (40, "Dwgs.User", Some("User.Drawings")),
        (41, "Cmts.User", Some("User.Comments")),
        (42, "Eco1.User", Some("User.Eco1")),
        (43, "Eco2.User", Some("User.Eco2")),
        (44, "Edge.Cuts", None),
        (45, "Margin", None),
        (46, "B.CrtYd", Some("B.Courtyard")),
        (47, "F.CrtYd", Some("F.Courtyard")),
        (48, "B.Fab", None),
        (49, "F.Fab", None),
    ] {
        let mut v = vec![atom(k.to_string()), s(name), atom("user")];
        if let Some(u) = user {
            v.push(s(u));
        }
        layer_table.push(Sexp::List(v));
    }
    items.push(list("layers", layer_table));
    // The physical stackup.
    let mut stack: Vec<Sexp> = vec![
        list("layer", vec![s("F.SilkS"), list("type", vec![s("Top Silk Screen")])]),
        list("layer", vec![s("F.Paste"), list("type", vec![s("Top Solder Paste")])]),
        list("layer", vec![s("F.Mask"), list("type", vec![s("Top Solder Mask")]), list("thickness", vec![n(crate::stackup::MASK_MM)])]),
    ];
    for (i, l) in view.layers.iter().enumerate() {
        let oz = view.stackup.copper_oz.get(i).copied().unwrap_or(1.0);
        stack.push(list("layer", vec![s(kicad_layer(l)), list("type", vec![s("copper")]), list("thickness", vec![n(oz * crate::stackup::OZ_MM)])]));
        if let Some(d) = view.stackup.dielectrics.get(i) {
            stack.push(list(
                "layer",
                vec![
                    s(format!("dielectric {}", i + 1)),
                    list("type", vec![s(if d.kind == crate::stackup::DielectricKind::Core { "core" } else { "prepreg" })]),
                    list("thickness", vec![n(d.thickness)]),
                    list("material", vec![s(d.material.clone())]),
                    list("epsilon_r", vec![n(d.epsilon_r.unwrap_or(4.5))]),
                    list("loss_tangent", vec![n(d.loss_tangent.unwrap_or(0.02))]),
                ],
            ));
        }
    }
    stack.push(list("layer", vec![s("B.Mask"), list("type", vec![s("Bottom Solder Mask")]), list("thickness", vec![n(crate::stackup::MASK_MM)])]));
    stack.push(list("layer", vec![s("B.Paste"), list("type", vec![s("Bottom Solder Paste")])]));
    stack.push(list("layer", vec![s("B.SilkS"), list("type", vec![s("Bottom Silk Screen")])]));
    stack.push(list("copper_finish", vec![s(view.finish.clone())]));
    stack.push(list("dielectric_constraints", vec![atom("no")]));
    items.push(list("setup", vec![list("stackup", stack), list("pad_to_mask_clearance", vec![n(view.rules.mask_expansion)]), list("allow_soldermask_bridges_in_footprints", vec![atom("no")])]));
    // Nets, numbered in name order.
    let mut names: Vec<String> = view.nets.iter().map(|x| x.name.clone()).collect();
    for p in &view.pads {
        if let Some(x) = &p.net {
            names.push(x.clone());
        }
    }
    names.sort_by(|a, b| crate::natural(a, b));
    names.dedup();
    let code: BTreeMap<String, usize> = names.iter().enumerate().map(|(i, x)| (x.clone(), i + 1)).collect();
    items.push(Sexp::List(vec![atom("net"), atom("0"), s("")]));
    for (i, x) in names.iter().enumerate() {
        items.push(Sexp::List(vec![atom("net"), atom((i + 1).to_string()), s(x.clone())]));
    }
    let net_ref = |x: &Option<String>| -> Sexp {
        match x.as_ref().and_then(|x| code.get(x)) {
            Some(c) => Sexp::List(vec![atom("net"), atom(c.to_string()), s(x.clone().unwrap())]),
            None => Sexp::List(vec![atom("net"), atom("0"), s("")]),
        }
    };
    let net_num = |x: &Option<String>| -> Sexp { list("net", vec![atom(x.as_ref().and_then(|x| code.get(x)).copied().unwrap_or(0).to_string())]) };

    // Footprints.
    for c in &view.components {
        let bottom = c.side == Side::Bottom;
        let fp = board.footprint(&c.footprint);
        let at_k = page.k(c.at);
        let side = |f: &str, b: &str| if bottom { b.to_string() } else { f.to_string() };
        let mut fv: Vec<Sexp> = vec![s(format!("Forma:{}", c.footprint)), list("layer", vec![s(side("F.Cu", "B.Cu"))]), u(&format!("fp {}", c.id)), at3("at", at_k, c.rotation)];
        if let Some(fp) = &fp {
            if !fp.description.is_empty() {
                fv.push(list("descr", vec![s(fp.description.clone())]));
            }
        }
        let lab = k_local(c.at, c.rotation, c.label);
        fv.push(list("property", vec![s("Reference"), s(c.reference.clone()), at3("at", lab, c.rotation), list("layer", vec![s(side("F.SilkS", "B.SilkS"))]), u(&format!("ref {}", c.id)), effects(bottom, false)]));
        let below = [lab[0], lab[1] + 2.0 * (k_local(c.at, c.rotation, c.at)[1] - lab[1]).max(1.0).min(3.0)];
        fv.push(list("property", vec![s("Value"), s(c.value.clone()), at3("at", below, c.rotation), list("layer", vec![s(side("F.Fab", "B.Fab"))]), u(&format!("val {}", c.id)), effects(bottom, false)]));
        for (key, val) in [("MPN", &c.mpn), ("Manufacturer", &c.manufacturer), ("LCSC", &c.lcsc)] {
            if let Some(v) = val {
                fv.push(list("property", vec![s(key), s(v.clone()), at3("at", [0.0, 0.0], c.rotation), list("layer", vec![s(side("F.Fab", "B.Fab"))]), list("hide", vec![atom("yes")]), u(&format!("{key} {}", c.id)), effects(bottom, false)]));
            }
        }
        if !c.description.is_empty() {
            fv.push(list("property", vec![s("Description"), s(c.description.clone()), at3("at", [0.0, 0.0], c.rotation), list("layer", vec![s(side("F.Fab", "B.Fab"))]), list("hide", vec![atom("yes")]), u(&format!("desc {}", c.id)), effects(bottom, false)]));
        }
        let pads: Vec<&crate::board::PadView> = view.pads.iter().filter(|p| p.component == c.id).collect();
        let tht = pads.iter().any(|p| p.kind == PadKind::Tht);
        let mut attr = vec![atom(if tht { "through_hole" } else { "smd" })];
        if c.kind == "hole" {
            attr = vec![atom("exclude_from_pos_files"), atom("exclude_from_bom")];
        }
        if c.dnp {
            attr.push(atom("dnp"));
        }
        fv.push(list("attr", attr));
        let line = |a: P, b: P, layer: &str, w: f64, seed: String| -> Sexp {
            list("fp_line", vec![list("start", vec![n(a[0]), n(a[1])]), list("end", vec![n(b[0]), n(b[1])]), stroke(w), list("layer", vec![s(layer)]), u(&seed)])
        };
        let mut li = 0;
        for l in &c.silk {
            for w in l.windows(2) {
                li += 1;
                fv.push(line(k_local(c.at, c.rotation, w[0]), k_local(c.at, c.rotation, w[1]), &side("F.SilkS", "B.SilkS"), view.rules.silk_width, format!("silk {} {li}", c.id)));
            }
        }
        if c.courtyard.len() >= 3 {
            let n_ = c.courtyard.len();
            for i in 0..n_ {
                li += 1;
                fv.push(line(k_local(c.at, c.rotation, c.courtyard[i]), k_local(c.at, c.rotation, c.courtyard[(i + 1) % n_]), &side("F.CrtYd", "B.CrtYd"), 0.05, format!("crt {} {li}", c.id)));
            }
        }
        if let Some(b) = c.bodies.iter().max_by(|a, b| (a.size[0] * a.size[1]).total_cmp(&(b.size[0] * b.size[1]))) {
            let n_ = b.outline.len();
            for i in 0..n_ {
                li += 1;
                fv.push(line(k_local(c.at, c.rotation, b.outline[i]), k_local(c.at, c.rotation, b.outline[(i + 1) % n_]), &side("F.Fab", "B.Fab"), 0.1, format!("fab {} {li}", c.id)));
            }
        }
        for (pi, p) in pads.iter().enumerate() {
            let local = k_local(c.at, c.rotation, p.at);
            let (ptype, cu_layers): (&str, Vec<&str>) = match p.kind {
                PadKind::Smd => ("smd", if bottom { vec!["B.Cu", "B.Paste", "B.Mask"] } else { vec!["F.Cu", "F.Paste", "F.Mask"] }),
                PadKind::Tht => ("thru_hole", vec!["*.Cu", "*.Mask"]),
                PadKind::Npth => ("np_thru_hole", vec!["*.Cu", "*.Mask"]),
            };
            let shape = match p.shape {
                PadShape::Rect => "rect",
                PadShape::RoundRect => "roundrect",
                PadShape::Circle => "circle",
                PadShape::Oval => "oval",
                PadShape::Custom => "custom",
            };
            let size = if p.shape == PadShape::Circle { [p.size[0].max(p.size[1]); 2] } else { p.size };
            let mut pv = vec![s(p.number.clone()), atom(ptype), atom(shape), at3("at", local, p.rotation)];
            if p.shape == PadShape::Custom {
                let a = 0.1f64.min(size[0].min(size[1]) * 0.5).max(0.01);
                pv.push(list("size", vec![n(a), n(a)]));
            } else {
                pv.push(list("size", vec![n(size[0]), n(size[1])]));
            }
            if let Some(d) = p.drill {
                if (d[0] - d[1]).abs() < 1e-9 {
                    pv.push(list("drill", vec![n(d[0])]));
                } else {
                    pv.push(list("drill", vec![atom("oval"), n(d[0]), n(d[1])]));
                }
            }
            pv.push(list("layers", cu_layers.into_iter().map(s).collect()));
            if p.shape == PadShape::RoundRect {
                let ratio = if size[0].min(size[1]) > 0.0 { p.corner / size[0].min(size[1]) } else { 0.25 };
                pv.push(list("roundrect_rratio", vec![n(ratio.clamp(0.0, 0.5))]));
            }
            if p.kind != PadKind::Npth && p.net.is_some() {
                pv.push(net_ref(&p.net));
            }
            if let (PadShape::Custom, Some(poly)) = (p.shape, &p.polygon) {
                // Primitives in pad coordinates: unrotated by the pad's angle, y down.
                let pts: Vec<Sexp> = poly.iter().map(|q| {
                    let r = rot(geom::sub(*q, p.at), -p.rotation);
                    list("xy", vec![n(r[0]), n(-r[1])])
                }).collect();
                pv.push(list("options", vec![list("clearance", vec![atom("outline")]), list("anchor", vec![atom("rect")])]));
                pv.push(list("primitives", vec![list("gr_poly", vec![list("pts", pts), list("width", vec![n(0.0)]), list("fill", vec![atom("yes")])])]));
            }
            pv.push(u(&format!("pad {} {pi}", c.id)));
            fv.push(list("pad", pv));
        }
        items.push(list("footprint", fv));
    }

    // Board outline.
    let mut ei = 0;
    for poly in std::iter::once(&view.outline).chain(view.cutouts.iter()) {
        let m = poly.len();
        for i in 0..m {
            ei += 1;
            items.push(list("gr_line", vec![page.xy("start", poly[i]), page.xy("end", poly[(i + 1) % m]), stroke(0.05), list("layer", vec![s("Edge.Cuts")]), u(&format!("edge {ei}"))]));
        }
    }
    // Copper.
    for t in &view.tracks {
        for (i, w) in t.points.windows(2).enumerate() {
            items.push(list("segment", vec![page.xy("start", w[0]), page.xy("end", w[1]), list("width", vec![n(t.width)]), list("layer", vec![s(kicad_layer(&t.layer))]), net_num(&t.net), u(&format!("seg {} {i}", t.id))]));
        }
    }
    for v in &view.vias {
        let mut vv = vec![];
        match v.kind {
            "blind" | "buried" => vv.push(atom("blind")),
            "micro" => vv.push(atom("micro")),
            _ => {}
        }
        vv.extend([page.xy("at", v.at), list("size", vec![n(v.diameter)]), list("drill", vec![n(v.drill)]), list("layers", vec![s(kicad_layer(&v.from)), s(kicad_layer(&v.to))])]);
        if !v.stack.is_empty() {
            // A per-layer padstack (KiCad 9); pads of 0 are removed on their layers.
            if v.stack.values().any(|d| *d == 0.0) {
                vv.push(list("remove_unused_layers", vec![atom("yes")]));
            }
            let mut ps = vec![list("mode", vec![atom("custom")])];
            for (l, d) in v.stack.iter().filter(|(_, d)| **d > 0.0) {
                ps.push(list("layer", vec![s(kicad_layer(l)), list("size", vec![n(*d)])]));
            }
            vv.push(list("padstack", ps));
        }
        if let Some(td) = &board.teardrops {
            vv.push(list(
                "teardrops",
                vec![
                    list("best_length_ratio", vec![n(td.length_ratio)]),
                    list("max_length", vec![n(td.max_length)]),
                    list("best_width_ratio", vec![n(td.width_ratio)]),
                    list("max_width", vec![n(td.max_width)]),
                    list("curve_points", vec![atom(if td.curved { "5" } else { "0" })]),
                    list("filter_ratio", vec![n(td.max_track_ratio)]),
                    list("enabled", vec![atom(if td.vias { "yes" } else { "no" })]),
                    list("allow_two_segments", vec![atom("yes")]),
                    list("prefer_zone_connections", vec![atom("yes")]),
                ],
            ));
        }
        vv.extend([net_num(&v.net), u(&format!("via {}", v.id))]);
        items.push(list("via", vv));
    }
    // Teardrops as KiCad stores them: filled zones marked as teardrops.
    for (i, t) in view.teardrops.iter().enumerate() {
        let pts: Vec<Sexp> = t.polygon.iter().map(|p| page.xy("xy", *p)).collect();
        items.push(list(
            "zone",
            vec![
                net_num(&t.net),
                list("net_name", vec![s(t.net.clone().unwrap_or_default())]),
                list("layer", vec![s(kicad_layer(&t.layer))]),
                u(&format!("teardrop {} {i}", t.track)),
                list("hatch", vec![atom("full"), n(0.1)]),
                list("priority", vec![atom("30000")]),
                list("attr", vec![list("teardrop", vec![list("type", vec![atom("padvia")])])]),
                list("connect_pads", vec![atom("yes"), list("clearance", vec![n(0.0)])]),
                list("min_thickness", vec![n(0.0254)]),
                list("filled_areas_thickness", vec![atom("no")]),
                list("fill", vec![atom("yes"), list("thermal_gap", vec![n(0.5)]), list("thermal_bridge_width", vec![n(0.5)])]),
                list("polygon", vec![list("pts", pts.clone())]),
                list("filled_polygon", vec![list("layer", vec![s(kicad_layer(&t.layer))]), list("pts", pts)]),
            ],
        ));
    }
    // Board graphics, as drawn (strokes as lines, fills as filled polygons).
    for gv in &view.graphics {
        let layer = kicad_layer(&gv.layer);
        for (si, st) in gv.strokes.iter().enumerate() {
            for (k, w) in st.windows(2).enumerate() {
                items.push(list("gr_line", vec![page.xy("start", w[0]), page.xy("end", w[1]), stroke(gv.width), list("layer", vec![s(layer.clone())]), u(&format!("gfx {} {si} {k}", gv.id))]));
            }
        }
        for (fi, island) in gv.fills.iter().enumerate() {
            // KiCad polygons have no holes: join each hole to the outline by a cut-in.
            let pts: Vec<Sexp> = crate::gerber::cut_in(island).iter().map(|p| page.xy("xy", *p)).collect();
            let mut pv = vec![list("pts", pts), stroke(0.0), list("fill", vec![atom("solid")]), list("layer", vec![s(layer.clone())])];
            if gv.net.is_some() {
                pv.push(net_num(&gv.net));
            }
            pv.push(u(&format!("gfx fill {} {fi}", gv.id)));
            items.push(list("gr_poly", pv));
        }
    }
    // Rule areas (keepouts).
    for a in &view.areas {
        let pts: Vec<Sexp> = a.outline.iter().map(|p| page.xy("xy", *p)).collect();
        let allow = |no: bool| atom(if no { "not_allowed" } else { "allowed" });
        let mut layers_s = vec![];
        for l in &a.layers {
            layers_s.push(s(kicad_layer(l)));
        }
        items.push(list(
            "zone",
            vec![
                list("net", vec![atom("0")]),
                list("net_name", vec![s("")]),
                list("layers", layers_s),
                u(&format!("area {}", a.id)),
                list("name", vec![s(a.name.clone())]),
                list("hatch", vec![atom("edge"), n(0.5)]),
                list("connect_pads", vec![list("clearance", vec![n(0.0)])]),
                list("min_thickness", vec![n(0.25)]),
                list("filled_areas_thickness", vec![atom("no")]),
                list("keepout", vec![list("tracks", vec![allow(a.no_tracks)]), list("vias", vec![allow(a.no_vias)]), list("pads", vec![allow(a.no_pads)]), list("copperpour", vec![allow(a.no_pours)]), list("footprints", vec![allow(a.no_footprints)])]),
                list("fill", vec![list("thermal_gap", vec![n(0.5)]), list("thermal_bridge_width", vec![n(0.5)])]),
                list("polygon", vec![list("pts", pts)]),
            ],
        ));
    }
    for z in &view.zones {
        let set = zones.get(&z.id);
        let clearance = set.and_then(|x| x.clearance).unwrap_or_else(|| board.class_of(Some(&z.net)).clearance);
        let mut cp = vec![];
        match set.map(|x| x.connection).unwrap_or_default() {
            PadConnection::Solid => cp.push(atom("yes")),
            PadConnection::None => cp.push(atom("no")),
            PadConnection::ThermalTht => cp.push(atom("thru_hole_only")),
            PadConnection::Thermal => {}
        }
        cp.push(list("clearance", vec![n(clearance)]));
        let gap = set.map(|x| x.gap_thermal()).unwrap_or(0.5);
        let spoke = set.map(|x| x.spoke_width()).unwrap_or(0.5);
        let mut zv = vec![net_num(&Some(z.net.clone())), list("net_name", vec![s(z.net.clone())]), list("layer", vec![s(kicad_layer(&z.layer))]), u(&format!("zone {}", z.id)), list("hatch", vec![atom("edge"), n(0.5)])];
        if z.priority > 0 {
            zv.push(list("priority", vec![atom(z.priority.max(0).to_string())]));
        }
        zv.push(list("connect_pads", cp));
        zv.push(list("min_thickness", vec![n(set.map(|x| x.min_width).unwrap_or(0.25))]));
        zv.push(list("filled_areas_thickness", vec![atom("no")]));
        if let Some(name) = set.and_then(|x| x.name.clone()) {
            zv.push(list("name", vec![s(name)]));
        }
        if let Some(limit) = set.and_then(|x| x.min_island_area) {
            zv.push(list("island_removal_mode", vec![atom("2")]));
            zv.push(list("island_area_min", vec![n(limit)]));
        } else if set.is_some_and(|x| !x.remove_islands) {
            zv.push(list("island_removal_mode", vec![atom("1")]));
        }
        let mut fill = vec![list("thermal_gap", vec![n(gap)]), list("thermal_bridge_width", vec![n(spoke)])];
        if let Some(h) = set.and_then(|x| x.hatch.clone()) {
            fill.insert(0, list("mode", vec![atom("hatch")]));
            fill.push(list("hatch_thickness", vec![n(h.width)]));
            fill.push(list("hatch_gap", vec![n(h.gap)]));
            fill.push(list("hatch_orientation", vec![n(h.angle)]));
            fill.push(list("hatch_border_algorithm", vec![atom("hatch_thickness")]));
        }
        zv.push(list("fill", fill));
        zv.push(list("polygon", vec![list("pts", z.outline.iter().map(|p| page.xy("xy", *p)).collect())]));
        items.push(list("zone", zv));
    }
    sexpr::write(&list("kicad_pcb", items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_bend_the_right_way() {
        // A quarter circle from (1,0) through (√½,√½) to (0,1), counter-clockwise.
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let a = arc3([1.0, 0.0], [h, h], [0.0, 1.0], 0.001);
        assert!(a.iter().all(|p| (p[0].hypot(p[1]) - 1.0).abs() < 1e-9 && p[0] >= -1e-9 && p[1] >= -1e-9));
        // The long way round, clockwise from (1,0) through (0,-1) to (0,1).
        let b = arc3([1.0, 0.0], [0.0, -1.0], [0.0, 1.0], 0.001);
        assert!(b.iter().any(|p| p[0] < -0.9));
    }

    #[test]
    fn uuids_are_stable_and_well_formed() {
        let a = uuid("x");
        assert_eq!(a, uuid("x"));
        assert_ne!(a, uuid("y"));
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
    }
}
