//! Boards from other EDA tools put into Forma's terms. Every importer reads
//! its file into a [`BoardIn`]: parts with their pads where they sit on the
//! board (Forma coordinates: mm, y up), copper segments and arcs, vias,
//! pours and the board edge as loose lines. [`finish`] makes that a
//! [`KicadBoard`] (the shape `pcb.board.import_kicad` also produces): each
//! part's pads turned back into a footprint in its own coordinates
//! (identical footprints shared), edges chained into the outline and
//! cutouts, track segments chained into polylines, a Default net class from
//! what the board uses most, and the board moved so its lower-left corner is
//! the origin.

use crate::footprints::{BodyPart, BodyShape, Footprint, Pad, PadKind, PadShape};
use crate::geom::{self, P};
use crate::kicad::{KComponent, KVia, KZone, KicadBoard};
use crate::{NetClass, Rules, Side};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct PadIn {
    pub number: String,
    pub kind: PadKind,
    pub shape: PadShape,
    /// Width and height before `rotation`.
    pub size: [f64; 2],
    /// Centre on the board.
    pub at: P,
    /// Counter-clockwise degrees on the board (as seen from the top).
    pub rotation: f64,
    pub drill: Option<[f64; 2]>,
    /// Round-rect corner as a fraction of the shorter side.
    pub corner: Option<f64>,
    /// A custom pad's outline on the board.
    pub polygon: Vec<P>,
    pub net: Option<String>,
}

impl PadIn {
    pub fn new(number: impl Into<String>, at: P, size: [f64; 2]) -> PadIn {
        PadIn { number: number.into(), kind: PadKind::Smd, shape: PadShape::Rect, size, at, rotation: 0.0, drill: None, corner: None, polygon: vec![], net: None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PartIn {
    pub reference: String,
    pub value: String,
    /// Package / footprint name in the source tool.
    pub footprint: String,
    pub at: P,
    pub rotation: f64,
    pub side: Side,
    pub pads: Vec<PadIn>,
    /// Silkscreen polylines on the board.
    pub silk: Vec<Vec<P>>,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub description: String,
    pub dnp: bool,
}

#[derive(Clone, Debug)]
pub struct SegIn {
    pub net: Option<String>,
    /// top, in1, in2, bottom.
    pub layer: String,
    pub width: f64,
    pub a: P,
    pub b: P,
}

#[derive(Clone, Debug, Default)]
pub struct BoardIn {
    pub title: Option<String>,
    pub revision: Option<String>,
    pub thickness: Option<f64>,
    /// Board edge pieces (lines, arcs as polylines, closed shapes) on the board.
    pub edges: Vec<Vec<P>>,
    pub parts: Vec<PartIn>,
    pub segments: Vec<SegIn>,
    pub vias: Vec<KVia>,
    pub zones: Vec<KZone>,
    /// Copper layers the source has (for a note when more than four).
    pub source_copper_layers: usize,
    pub notes: Vec<String>,
}

impl BoardIn {
    pub fn note(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }
    pub fn seg(&mut self, net: Option<String>, layer: &str, width: f64, a: P, b: P) {
        if geom::dist(a, b) > 1e-9 || width > 0.0 {
            self.segments.push(SegIn { net: net.filter(|n| !n.is_empty()), layer: layer.into(), width, a, b });
        }
    }
    /// A polyline of copper (an arc already flattened).
    pub fn polyline(&mut self, net: Option<String>, layer: &str, width: f64, pts: &[P]) {
        for w in pts.windows(2) {
            self.seg(net.clone(), layer, width, w[0], w[1]);
        }
    }
}

// ---- Geometry helpers shared by the importers ---------------------------------------

/// Points of an arc around `c` of radius `r` from `a0` sweeping `span` degrees (counter-clockwise when positive).
pub fn arc(c: P, r: f64, a0: f64, span: f64) -> Vec<P> {
    let n = ((span.abs() / 7.5).ceil() as usize).clamp(2, 96);
    (0..=n)
        .map(|i| {
            let t = (a0 + span * i as f64 / n as f64).to_radians();
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

pub fn circle(c: P, r: f64) -> Vec<P> {
    let mut v = arc(c, r, 0.0, 360.0);
    v.pop();
    v
}

/// The arc from `a` to `b` around `c` the short way round, or counter-clockwise when `ccw` is given.
pub fn arc_between(c: P, a: P, b: P, ccw: Option<bool>) -> Vec<P> {
    let r = geom::dist(c, a);
    let a0 = (a[1] - c[1]).atan2(a[0] - c[0]).to_degrees();
    let a1 = (b[1] - c[1]).atan2(b[0] - c[0]).to_degrees();
    let mut span = (a1 - a0).rem_euclid(360.0);
    match ccw {
        Some(true) => {
            if span < 1e-9 {
                span = 360.0;
            }
        }
        Some(false) => span -= 360.0,
        None => {
            if span > 180.0 {
                span -= 360.0;
            }
        }
    }
    let mut pts = arc(c, r, a0, span);
    if let Some(l) = pts.last_mut() {
        *l = b;
    }
    pts[0] = a;
    pts
}

/// An arc from `a` to `b` turning `sweep` degrees (counter-clockwise positive).
pub fn arc_sweep(a: P, b: P, sweep: f64) -> Vec<P> {
    if sweep.abs() < 1e-9 {
        return vec![a, b];
    }
    let chord = geom::dist(a, b);
    let half = sweep.to_radians() / 2.0;
    let r = chord / (2.0 * half.sin().abs());
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let d = r * half.cos().abs();
    let dir = [(b[0] - a[0]) / chord, (b[1] - a[1]) / chord];
    // Centre to the left of a→b for a counter-clockwise turn under 180°.
    let left = [-dir[1], dir[0]];
    let s = if (sweep > 0.0) == (sweep.abs() < 180.0) { 1.0 } else { -1.0 };
    let c = [mid[0] + left[0] * d * s, mid[1] + left[1] * d * s];
    arc_between(c, a, b, Some(sweep > 0.0))
}

fn r6(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

fn r6p(p: P) -> P {
    [r6(p[0]), r6(p[1])]
}

/// A board point in a part's own coordinates (bottom parts mirrored in x).
fn local(at: P, rot: f64, bottom: bool, p: P) -> P {
    let q = geom::rotate(geom::sub(p, at), -rot);
    if bottom { [-q[0], q[1]] } else { q }
}

fn bbox(points: impl Iterator<Item = P>) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut any = false;
    for p in points {
        any = true;
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    any.then_some(b)
}

/// A part's footprint in its own coordinates.
fn footprint_of(p: &PartIn, name: &str) -> Footprint {
    let bottom = p.side == Side::Bottom;
    let pads: Vec<Pad> = p
        .pads
        .iter()
        .map(|q| {
            let lr = (q.rotation - p.rotation).rem_euclid(360.0);
            let lr = if bottom { (-lr).rem_euclid(360.0) } else { lr };
            let polygon: Vec<P> = if q.shape == PadShape::Custom && q.polygon.len() >= 3 {
                let mut v: Vec<P> = q
                    .polygon
                    .iter()
                    .map(|g| {
                        let d = geom::rotate(geom::sub(*g, q.at), -q.rotation);
                        r6p(if bottom { [-d[0], d[1]] } else { d })
                    })
                    .collect();
                if geom::polygon_area(&v) < 0.0 {
                    v.reverse();
                }
                v
            } else {
                vec![]
            };
            let shape = if q.shape == PadShape::Custom && polygon.is_empty() { PadShape::Rect } else { q.shape };
            let lr = if lr > 359.9999 { 0.0 } else { r6(lr) };
            Pad { number: q.number.clone(), kind: q.kind, shape, size: [r6(q.size[0]), r6(q.size[1])], at: r6p(local(p.at, p.rotation, bottom, q.at)), rotation: lr, drill: q.drill.map(|d| [r6(d[0]), r6(d[1])]), corner: q.corner, paste: vec![], polygon, extra: Default::default() }
        })
        .collect();
    let silk: Vec<Vec<P>> = p.silk.iter().map(|l| l.iter().map(|g| r6p(local(p.at, p.rotation, bottom, *g))).collect()).collect();
    let kind = crate::kicad::kind_for(&p.reference, &p.value);
    let mut body = Vec::new();
    if kind != "hole" {
        let pts = pads.iter().flat_map(|q| {
            let h = [q.size[0].max(q.size[1]) / 2.0; 2];
            [[q.at[0] - h[0], q.at[1] - h[1]], [q.at[0] + h[0], q.at[1] + h[1]]]
        });
        if let Some(b) = bbox(pts) {
            let (h, material) = crate::kicad::body_guess(kind);
            let w = (b[2] - b[0]) * 0.8;
            let d = (b[3] - b[1]) * 0.8;
            if w > 0.05 && d > 0.05 {
                body.push(BodyPart { shape: BodyShape::Box, at: [r6((b[0] + b[2]) / 2.0), r6((b[1] + b[3]) / 2.0)], size: [r6(w), r6(d)], z: [0.0, h], material: material.into() });
            }
        }
    }
    Footprint { name: name.to_string(), description: format!("{} (imported)", p.footprint), source: "imported".into(), pads, courtyard: None, silk, body, kind: kind.into() }
}

fn same_geometry(a: &Footprint, b: &Footprint) -> bool {
    a.pads == b.pads
}

/// The board in Forma's terms.
pub fn finish(mut b: BoardIn) -> Result<KicadBoard, String> {
    let mut notes = std::mem::take(&mut b.notes);
    if b.source_copper_layers > 4 {
        notes.push(format!("the board has {} copper layers; Forma boards have up to 4, so copper on layers past the second inner layer was left out", b.source_copper_layers));
    }
    let uses_inner = b.segments.iter().any(|s| s.layer.starts_with("in")) || b.zones.iter().any(|z| z.layer.starts_with("in"));
    let layers: u8 = if uses_inner || b.source_copper_layers >= 4 { 4 } else { 2 };
    // Outline from the edge pieces.
    let (mut loops, open) = crate::kicad::chain_loops(b.edges.clone(), 0.01);
    if open > 0 {
        notes.push(format!("{open} board-edge pieces did not close into a loop and were left out"));
    }
    loops.retain(|l| l.len() >= 3 && geom::polygon_area(l).abs() > 1e-6);
    loops.sort_by(|x, y| geom::polygon_area(y).abs().total_cmp(&geom::polygon_area(x).abs()));
    let (outline, cutouts) = if loops.is_empty() {
        let pts = b.parts.iter().flat_map(|p| p.pads.iter().map(|q| q.at)).chain(b.segments.iter().flat_map(|s| [s.a, s.b])).chain(b.vias.iter().map(|v| v.at));
        let bb = bbox(pts).ok_or("the file has no board outline and nothing on it")?;
        notes.push("the file has no closed board outline: a rectangle 2 mm around everything was used".into());
        let m = 2.0;
        (vec![[bb[0] - m, bb[1] - m], [bb[2] + m, bb[1] - m], [bb[2] + m, bb[3] + m], [bb[0] - m, bb[3] + m]], vec![])
    } else {
        let mut ls = loops.into_iter();
        let o = ls.next().unwrap();
        let cut: Vec<Vec<P>> = ls.filter(|l| geom::in_polygon(l[0], &o)).collect();
        (o, cut)
    };
    let bb = bbox(outline.iter().copied()).unwrap();
    let shift = [-bb[0], -bb[1]];
    let mv = |p: P| r6p(geom::add(p, shift));

    // Footprints: one per distinct geometry, named after the source's package.
    let mut footprints: BTreeMap<String, Footprint> = BTreeMap::new();
    let mut components = Vec::new();
    let mut refs: BTreeMap<String, usize> = BTreeMap::new();
    for p in &b.parts {
        if p.pads.is_empty() && p.silk.is_empty() {
            notes.push(format!("{} has no pads and was left out", p.reference));
            continue;
        }
        let base = if p.footprint.trim().is_empty() { format!("{}_fp", p.reference) } else { p.footprint.trim().to_string() };
        let mut name = base.clone();
        let mut k = 1;
        let fp = loop {
            let f = footprint_of(p, &name);
            match footprints.get(&name) {
                Some(existing) if same_geometry(existing, &f) => break f,
                Some(_) => {
                    k += 1;
                    name = format!("{base}_{k}");
                }
                None => break f,
            }
        };
        if fp.pads.is_empty() {
            notes.push(format!("{} has no pads (silkscreen only) and was left out", p.reference));
            continue;
        }
        footprints.entry(name.clone()).or_insert(fp);
        let mut reference = if p.reference.trim().is_empty() { "X".to_string() } else { p.reference.trim().to_string() };
        let n = refs.entry(reference.clone()).or_default();
        *n += 1;
        if *n > 1 {
            let r2 = format!("{reference}_{n}");
            notes.push(format!("reference {reference} is used more than once; the copy became {r2}"));
            reference = r2;
        }
        let mut pins = BTreeMap::new();
        for q in &p.pads {
            if let Some(net) = q.net.as_ref().filter(|n| !n.is_empty()) {
                if q.kind != PadKind::Npth && !q.number.is_empty() {
                    pins.insert(q.number.clone(), net.clone());
                }
            }
        }
        components.push(KComponent {
            reference,
            value: p.value.clone(),
            footprint: name,
            at: mv(p.at),
            rotation: r6(p.rotation.rem_euclid(360.0)),
            side: p.side,
            pins,
            dnp: p.dnp,
            mpn: p.mpn.clone(),
            manufacturer: p.manufacturer.clone(),
            lcsc: None,
            description: p.description.clone(),
        });
    }
    let four = layers == 4;
    let segs: Vec<(Option<String>, String, f64, P, P)> = b
        .segments
        .iter()
        .filter(|s| s.width > 0.0 && (four || !s.layer.starts_with("in")))
        .map(|s| (s.net.clone(), s.layer.clone(), r6(s.width), mv(s.a), mv(s.b)))
        .collect();
    let tracks = crate::kicad::chain_tracks(segs);
    let vias: Vec<KVia> = b.vias.iter().filter(|v| v.diameter > v.drill && v.drill > 0.0).map(|v| KVia { net: v.net.clone().filter(|n| !n.is_empty()), at: mv(v.at), drill: r6(v.drill), diameter: r6(v.diameter) }).collect();
    let zones: Vec<KZone> = b.zones.iter().filter(|z| z.outline.len() >= 3 && (four || !z.layer.starts_with("in")) && !z.net.is_empty()).map(|z| KZone { outline: z.outline.iter().map(|p| mv(*p)).collect(), ..z.clone() }).collect();

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
    let mut rules = Rules::default();
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
        title: b.title.clone(),
        revision: b.revision.clone(),
        layers,
        thickness: b.thickness.unwrap_or(1.6),
        outline: outline.iter().map(|p| mv(*p)).collect(),
        cutouts: cutouts.iter().map(|c| c.iter().map(|p| mv(*p)).collect()).collect(),
        footprints,
        components,
        tracks,
        vias,
        zones,
        default_class: class,
        rules,
        notes,
        shift,
    })
}

/// A pad shape from common names (round, circle, ellipse, oval, obround, rect, square, roundrect, octagon).
pub fn shape_named(s: &str) -> PadShape {
    let s = s.to_ascii_lowercase();
    if s.contains("roundrect") || s.contains("rounded") || s.contains("round_rect") {
        PadShape::RoundRect
    } else if s.contains("oval") || s.contains("obround") || s.contains("long") || s.contains("ellipse") || s.contains("finger") {
        PadShape::Oval
    } else if s.contains("round") || s.contains("circ") || s == "r" {
        PadShape::Circle
    } else {
        PadShape::Rect
    }
}

/// A round shape becomes oval when its sides differ.
pub fn fix_shape(shape: PadShape, size: [f64; 2]) -> PadShape {
    if shape == PadShape::Circle && (size[0] - size[1]).abs() > 1e-6 { PadShape::Oval } else { shape }
}

/// Text that is a number with an optional unit (mm, mil, in, um), in mm; `default` mm per unit otherwise.
pub fn length(s: &str, default: f64) -> Option<f64> {
    let t = s.trim();
    let (num, k) = if let Some(v) = t.strip_suffix("mm") {
        (v, 1.0)
    } else if let Some(v) = t.strip_suffix("mil") {
        (v, 0.0254)
    } else if let Some(v) = t.strip_suffix("um") {
        (v, 0.001)
    } else if let Some(v) = t.strip_suffix("in") {
        (v, 25.4)
    } else {
        (t, default)
    };
    num.trim().parse::<f64>().ok().map(|v| v * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_turn_the_right_way() {
        let a = arc_sweep([1.0, 0.0], [0.0, 1.0], 90.0);
        let m = a[a.len() / 2];
        assert!((geom::dist(m, [0.0, 0.0]) - 1.0).abs() < 1e-6, "{m:?}");
        assert!(m[0] > 0.5 && m[1] > 0.5);
        let b = arc_sweep([1.0, 0.0], [0.0, 1.0], -270.0);
        let m = b[b.len() / 2];
        assert!(m[0] < -0.5 && m[1] < -0.5, "{m:?}");
        assert_eq!(length("10mil", 1.0).map(|v| (v * 1e4).round()), Some(2540.0));
    }

    #[test]
    fn parts_become_footprints_in_their_own_coordinates() {
        let mut b = BoardIn::default();
        b.edges.push(vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0], [0.0, 0.0]]);
        for (r, at, rot, side) in [("R1", [5.0, 5.0], 90.0, Side::Top), ("R2", [12.0, 5.0], 0.0, Side::Bottom)] {
            let bottom = side == Side::Bottom;
            let place = |l: P| {
                let l = if bottom { [-l[0], l[1]] } else { l };
                geom::add(at, geom::rotate(l, rot))
            };
            let pads = vec![
                PadIn { net: Some("A".into()), ..PadIn::new("1", place([-0.8, 0.0]), [0.9, 1.0]) },
                PadIn { net: Some("B".into()), ..PadIn::new("2", place([0.8, 0.0]), [0.9, 1.0]) },
            ]
            .into_iter()
            .map(|mut p| {
                p.rotation = rot;
                p
            })
            .collect();
            b.parts.push(PartIn { reference: r.into(), value: "1k".into(), footprint: "R0603".into(), at, rotation: rot, side, pads, ..Default::default() });
        }
        let k = finish(b).unwrap();
        assert_eq!(k.footprints.len(), 1, "same geometry, one footprint");
        let f = &k.footprints["R0603"];
        let mut xs: Vec<f64> = f.pads.iter().map(|p| p.at[0]).collect();
        xs.sort_by(f64::total_cmp);
        assert_eq!(xs, vec![-0.8, 0.8]);
        assert!(f.pads.iter().all(|p| p.rotation == 0.0 && p.at[1] == 0.0));
        assert_eq!(k.components[0].pins["1"], "A");
        assert_eq!(k.outline.len(), 4);
    }
}
