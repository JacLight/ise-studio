//! Altium Designer documents: `.PcbDoc` and `.SchDoc`, binary (an OLE
//! compound file of streams) or ASCII (one `|KEY=VALUE|…` record per line).
//! Written from the documented structure of those files as Altium writes
//! them (the same structure KiCad's importer reads), in our own code.
//!
//! PCB: `Board6/Data` (outline vertices), `Components6/Data`, `Nets6/Data`
//! and `Polygons6/Data` are property records (a 32-bit length, then
//! `|KEY=VALUE|…`). `Pads6`, `Vias6`, `Tracks6` and `Arcs6` are binary
//! records: a type byte, then length-prefixed sub-records of little-endian
//! fields. Lengths are in Altium's internal unit (1/10000 mil); y up as
//! Iṣẹ́ Studio. Copper layers: 1 top, 2–31 mid layers, 32 bottom; 74 is multi-layer.
//! Pads, tracks and arcs name their component and net by index (0xFFFF: none).
//!
//! Schematic: the `FileHeader` stream is property records (`RECORD=1`
//! component, 2 pin, 4 label, 6 polyline, 7 polygon, 8 ellipse, 12 arc, 13
//! line, 14 rectangle, 17 power port, 22 no-ERC, 25 net label, 27 wire, 29
//! junction, 34 designator, 41 parameter, 45 implementation (footprint)),
//! children naming their owner by `OWNERINDEX`; coordinates in 10 mil (with
//! `_FRAC` parts), y up.

use super::build::{self, BoardIn, PadIn, PartIn};
use super::sch::{self, Draft, DraftSymbol};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{self, P};
use crate::kicad::{KVia, KZone};
use crate::schematic::LabelKind;
use crate::symbols::{Dir, Fill, Gfx, PinDef, PinType};
use crate::{PadConnection, Side};
use std::collections::BTreeMap;

/// Internal units (1/10000 mil) to mm.
const IU: f64 = 0.0254 / 10000.0;

pub type Props = BTreeMap<String, String>;

/// `|KEY=VALUE|KEY=VALUE` (keys upper-cased).
pub fn props(s: &str) -> Props {
    s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .split('|')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim_end_matches('\0').to_string()))
        .collect()
}

/// Property records of a stream: each a u32 length (high byte: flags) and the text.
pub fn prop_records(b: &[u8]) -> Vec<Props> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= b.len() {
        let len = (u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) & 0x00FF_FFFF) as usize;
        i += 4;
        let Some(body) = b.get(i..i + len) else { break };
        let text: String = body.iter().map(|&c| c as char).collect();
        out.push(props(&text));
        i += len;
    }
    out
}

/// "1000mil", "25.4mm", or plain mils → mm.
fn len_mm(s: &str) -> f64 {
    let t = s.trim();
    if let Some(v) = t.strip_suffix("mil") {
        v.trim().parse::<f64>().unwrap_or(0.0) * 0.0254
    } else if let Some(v) = t.strip_suffix("mm") {
        v.trim().parse().unwrap_or(0.0)
    } else {
        t.parse::<f64>().unwrap_or(0.0) * 0.0254
    }
}

fn pmm(p: &Props, k: &str) -> f64 {
    p.get(k).map(|v| len_mm(v)).unwrap_or(0.0)
}

fn pnum(p: &Props, k: &str) -> f64 {
    p.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0)
}

fn ptrue(p: &Props, k: &str) -> bool {
    p.get(k).is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "T")
}

// ---- Binary records -----------------------------------------------------------------------

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl Rd<'_> {
    fn u8(&mut self) -> u8 {
        let v = self.b.get(self.i).copied().unwrap_or(0);
        self.i += 1;
        v
    }
    fn u16(&mut self) -> u16 {
        let v = self.b.get(self.i..self.i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0);
        self.i += 2;
        v
    }
    fn u32(&mut self) -> u32 {
        let v = self.b.get(self.i..self.i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0);
        self.i += 4;
        v
    }
    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    fn f64(&mut self) -> f64 {
        let v = self.b.get(self.i..self.i + 8).map(|s| f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])).unwrap_or(0.0);
        self.i += 8;
        v
    }
    fn mm(&mut self) -> f64 {
        self.i32() as f64 * IU
    }
    fn pt(&mut self) -> P {
        let x = self.mm();
        let y = self.mm();
        [x, y]
    }
    fn skip(&mut self, n: usize) {
        self.i += n;
    }
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
}

#[derive(Clone, Debug)]
pub struct APad {
    pub name: String,
    pub layer: u8,
    pub net: u16,
    pub component: u16,
    pub at: P,
    pub size: [f64; 2],
    pub hole: f64,
    pub shape: u8,
    pub rotation: f64,
    pub plated: bool,
    /// Slot length when the hole is a slot (0: round).
    pub slot: f64,
    pub slot_rotation: f64,
    /// Round-rect corner radius, % of the shorter half side (0: none).
    pub corner: u8,
}

#[derive(Clone, Debug)]
pub struct ATrack {
    pub layer: u8,
    pub net: u16,
    pub component: u16,
    pub a: P,
    pub b: P,
    pub width: f64,
}

#[derive(Clone, Debug)]
pub struct AArc {
    pub layer: u8,
    pub net: u16,
    pub component: u16,
    pub center: P,
    pub radius: f64,
    pub start: f64,
    pub end: f64,
    pub width: f64,
}

#[derive(Clone, Debug)]
pub struct AVia {
    pub net: u16,
    pub at: P,
    pub diameter: f64,
    pub hole: f64,
}

/// One sub-record: its length, then a reader over exactly its bytes.
fn sub<'a>(r: &mut Rd<'a>) -> Rd<'a> {
    let len = r.u32() as usize;
    let start = r.i.min(r.b.len());
    let end = (start + len).min(r.b.len());
    r.i = start + len;
    Rd { b: &r.b[start..end], i: 0 }
}

pub fn pads6(b: &[u8]) -> Vec<APad> {
    let mut r = Rd { b, i: 0 };
    let mut out = Vec::new();
    while !r.done() {
        if r.u8() != 2 {
            break;
        }
        let mut s1 = sub(&mut r);
        let n = s1.u8() as usize;
        let name: String = s1.b.get(1..1 + n).unwrap_or(&[]).iter().map(|&c| c as char).collect();
        sub(&mut r);
        sub(&mut r);
        sub(&mut r);
        let mut s = sub(&mut r);
        let layer = s.u8();
        s.skip(2);
        let net = s.u16();
        s.skip(2);
        let component = s.u16();
        s.skip(4);
        let at = s.pt();
        let top = [s.mm(), s.mm()];
        let _mid = [s.mm(), s.mm()];
        let _bot = [s.mm(), s.mm()];
        let hole = s.mm();
        let shape = s.u8();
        let _mid_shape = s.u8();
        let _bot_shape = s.u8();
        let rotation = s.f64();
        let plated = s.u8() != 0;
        let mut p = APad { name, layer, net, component, at, size: top, hole, shape, rotation, plated, slot: 0.0, slot_rotation: 0.0, corner: 0 };
        // Sub-record 6: per-layer sizes and shapes, hole shape, slot, corner radii.
        let mut s6 = sub(&mut r);
        if s6.b.len() >= 596 {
            s6.skip(29 * 4 * 2 + 29 + 1);
            let hole_shape = s6.u8();
            let slot = s6.mm();
            let slot_rot = s6.f64();
            if hole_shape == 2 && slot > hole {
                p.slot = slot;
                p.slot_rotation = slot_rot;
            }
            s6.skip(32 * 4 * 2 + 1 + 32);
            p.corner = s6.u8();
        }
        out.push(p);
    }
    out
}

pub fn tracks6(b: &[u8]) -> Vec<ATrack> {
    let mut r = Rd { b, i: 0 };
    let mut out = Vec::new();
    while !r.done() {
        if r.u8() != 4 {
            break;
        }
        let mut s = sub(&mut r);
        let layer = s.u8();
        s.skip(2);
        let net = s.u16();
        s.skip(2);
        let component = s.u16();
        s.skip(4);
        let a = s.pt();
        let b2 = s.pt();
        let width = s.mm();
        out.push(ATrack { layer, net, component, a, b: b2, width });
    }
    out
}

pub fn arcs6(b: &[u8]) -> Vec<AArc> {
    let mut r = Rd { b, i: 0 };
    let mut out = Vec::new();
    while !r.done() {
        if r.u8() != 1 {
            break;
        }
        let mut s = sub(&mut r);
        let layer = s.u8();
        s.skip(2);
        let net = s.u16();
        s.skip(2);
        let component = s.u16();
        s.skip(4);
        let center = s.pt();
        let radius = s.mm();
        let start = s.f64();
        let end = s.f64();
        let width = s.mm();
        out.push(AArc { layer, net, component, center, radius, start, end, width });
    }
    out
}

pub fn vias6(b: &[u8]) -> Vec<AVia> {
    let mut r = Rd { b, i: 0 };
    let mut out = Vec::new();
    while !r.done() {
        if r.u8() != 3 {
            break;
        }
        let mut s = sub(&mut r);
        s.skip(3);
        let net = s.u16();
        s.skip(8);
        let at = s.pt();
        let diameter = s.mm();
        let hole = s.mm();
        out.push(AVia { net, at, diameter, hole });
    }
    out
}

// ---- Writers for the binary records (fixtures and round trips) -------------------------------

fn put_i(v: &mut Vec<u8>, x: f64) {
    v.extend_from_slice(&((x / IU).round() as i32).to_le_bytes());
}

fn rec(kind: u8, subs: &[Vec<u8>]) -> Vec<u8> {
    let mut v = vec![kind];
    for s in subs {
        v.extend_from_slice(&(s.len() as u32).to_le_bytes());
        v.extend_from_slice(s);
    }
    v
}

pub fn write_pad(p: &APad) -> Vec<u8> {
    let mut s1 = vec![p.name.len() as u8];
    s1.extend(p.name.bytes());
    let mut s5 = vec![p.layer, 0, 0];
    s5.extend_from_slice(&p.net.to_le_bytes());
    s5.extend_from_slice(&[0, 0]);
    s5.extend_from_slice(&p.component.to_le_bytes());
    s5.extend_from_slice(&[0; 4]);
    for v in [p.at[0], p.at[1], p.size[0], p.size[1], p.size[0], p.size[1], p.size[0], p.size[1], p.hole] {
        put_i(&mut s5, v);
    }
    s5.extend_from_slice(&[p.shape, p.shape, p.shape]);
    s5.extend_from_slice(&p.rotation.to_le_bytes());
    s5.push(p.plated as u8);
    s5.resize(120, 0);
    let mut s6 = Vec::new();
    if p.slot > 0.0 || p.corner > 0 {
        s6.resize(29 * 4 * 2 + 29 + 1, 0);
        s6.push(if p.slot > 0.0 { 2 } else { 0 });
        put_i(&mut s6, p.slot);
        s6.extend_from_slice(&p.slot_rotation.to_le_bytes());
        s6.resize(s6.len() + 32 * 4 * 2 + 1 + 32, 0);
        s6.push(p.corner);
        s6.resize(651, 0);
    }
    rec(2, &[s1, vec![], vec![], vec![], s5, s6])
}

pub fn write_track(t: &ATrack) -> Vec<u8> {
    let mut s = vec![t.layer, 0, 0];
    s.extend_from_slice(&t.net.to_le_bytes());
    s.extend_from_slice(&[0, 0]);
    s.extend_from_slice(&t.component.to_le_bytes());
    s.extend_from_slice(&[0; 4]);
    for v in [t.a[0], t.a[1], t.b[0], t.b[1], t.width] {
        put_i(&mut s, v);
    }
    s.resize(49, 0);
    rec(4, &[s])
}

pub fn write_arc(a: &AArc) -> Vec<u8> {
    let mut s = vec![a.layer, 0, 0];
    s.extend_from_slice(&a.net.to_le_bytes());
    s.extend_from_slice(&[0, 0]);
    s.extend_from_slice(&a.component.to_le_bytes());
    s.extend_from_slice(&[0; 4]);
    for v in [a.center[0], a.center[1], a.radius] {
        put_i(&mut s, v);
    }
    s.extend_from_slice(&a.start.to_le_bytes());
    s.extend_from_slice(&a.end.to_le_bytes());
    put_i(&mut s, a.width);
    s.resize(56, 0);
    rec(1, &[s])
}

pub fn write_via(v: &AVia) -> Vec<u8> {
    let mut s = vec![0, 0, 0];
    s.extend_from_slice(&v.net.to_le_bytes());
    s.extend_from_slice(&[0; 8]);
    for x in [v.at[0], v.at[1], v.diameter, v.hole] {
        put_i(&mut s, x);
    }
    s.extend_from_slice(&[1, 32]);
    s.resize(31, 0);
    rec(3, &[s])
}

/// Property records as a stream.
pub fn write_props(records: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    for r in records {
        let mut body: Vec<u8> = r.bytes().collect();
        body.push(0);
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(&body);
    }
    v
}

// ---- PCB -----------------------------------------------------------------------------------

fn layer_of(l: u8, b: &mut BoardIn, mids: &[u8]) -> Option<String> {
    match l {
        1 => Some("top".into()),
        32 => Some("bottom".into()),
        2..=31 | 39..=54 => match mids.iter().position(|x| *x == l) {
            Some(0) => Some("in1".into()),
            Some(1) => Some("in2".into()),
            _ => {
                b.note("copper on inner layers past the second was left out");
                None
            }
        },
        _ => None,
    }
}

fn pad_shape(s: u8, corner: u8, size: [f64; 2]) -> (PadShape, Option<f64>) {
    match s {
        1 if (size[0] - size[1]).abs() < 1e-9 => (PadShape::Circle, None),
        1 => (PadShape::Oval, None),
        2 => (PadShape::Rect, None),
        3 => (PadShape::Rect, None),
        9 => (PadShape::RoundRect, Some((corner.max(1) as f64 / 200.0).min(0.5))),
        _ => (PadShape::Rect, None),
    }
}

struct Pcb {
    board: Vec<Props>,
    components: Vec<Props>,
    nets: Vec<String>,
    polygons: Vec<Props>,
    pads: Vec<APad>,
    tracks: Vec<ATrack>,
    arcs: Vec<AArc>,
    vias: Vec<AVia>,
}

fn net_of(nets: &[String], i: u16) -> Option<String> {
    if i == u16::MAX { None } else { nets.get(i as usize).cloned() }
}

fn from_ascii(text: &str) -> Pcb {
    let mut p = Pcb { board: vec![], components: vec![], nets: vec![], polygons: vec![], pads: vec![], tracks: vec![], arcs: vec![], vias: vec![] };
    let idx = |r: &Props, k: &str| r.get(k).and_then(|v| v.trim().parse::<u16>().ok()).unwrap_or(u16::MAX);
    let layer = |r: &Props| -> u8 {
        match r.get("LAYER").map(|s| s.to_ascii_uppercase()).as_deref() {
            Some("TOP") => 1,
            Some("BOTTOM") => 32,
            Some("MULTILAYER") => 74,
            Some(x) if x.starts_with("MID") => x[3..].parse::<u8>().map(|n| n + 1).unwrap_or(0),
            Some(x) if x.starts_with("PLANE") => x[5..].parse::<u8>().map(|n| n + 38).unwrap_or(0),
            Some("KEEPOUT") => 56,
            Some("TOPOVERLAY") => 33,
            Some("BOTTOMOVERLAY") => 34,
            Some(x) if x.starts_with("MECHANICAL") => x[10..].parse::<u8>().map(|n| n + 56).unwrap_or(57),
            _ => 0,
        }
    };
    for line in text.lines() {
        let r = props(line);
        match r.get("RECORD").map(|s| s.to_ascii_uppercase()).as_deref() {
            Some("BOARD") => p.board.push(r),
            Some("COMPONENT") => p.components.push(r),
            Some("NET") => p.nets.push(r.get("NAME").cloned().unwrap_or_default()),
            Some("POLYGON") => p.polygons.push(r),
            Some("PAD") => {
                let size = [pmm(&r, "TOPXSIZE").max(pmm(&r, "XSIZE")), pmm(&r, "TOPYSIZE").max(pmm(&r, "YSIZE"))];
                let shape = match r.get("TOPSHAPE").or(r.get("SHAPE")).map(|s| s.to_ascii_uppercase()).as_deref() {
                    Some("ROUND") => 1,
                    Some("RECTANGLE") => 2,
                    Some("OCTAGONAL") => 3,
                    Some("ROUNDEDRECTANGLE") => 9,
                    _ => 1,
                };
                p.pads.push(APad { name: r.get("NAME").cloned().unwrap_or_default(), layer: layer(&r), net: idx(&r, "NET"), component: idx(&r, "COMPONENT"), at: [pmm(&r, "X"), pmm(&r, "Y")], size, hole: pmm(&r, "HOLESIZE"), shape, rotation: pnum(&r, "ROTATION"), plated: !r.get("PLATED").is_some_and(|v| v.eq_ignore_ascii_case("false")), slot: pmm(&r, "SLOTSIZE"), slot_rotation: pnum(&r, "SLOTROTATION"), corner: pnum(&r, "CORNERRADIUS") as u8 });
            }
            Some("TRACK") => p.tracks.push(ATrack { layer: layer(&r), net: idx(&r, "NET"), component: idx(&r, "COMPONENT"), a: [pmm(&r, "X1"), pmm(&r, "Y1")], b: [pmm(&r, "X2"), pmm(&r, "Y2")], width: pmm(&r, "WIDTH") }),
            Some("ARC") => p.arcs.push(AArc { layer: layer(&r), net: idx(&r, "NET"), component: idx(&r, "COMPONENT"), center: [pmm(&r, "LOCATION.X"), pmm(&r, "LOCATION.Y")], radius: pmm(&r, "RADIUS"), start: pnum(&r, "STARTANGLE"), end: pnum(&r, "ENDANGLE"), width: pmm(&r, "WIDTH") }),
            Some("VIA") => p.vias.push(AVia { net: idx(&r, "NET"), at: [pmm(&r, "X"), pmm(&r, "Y")], diameter: pmm(&r, "DIAMETER"), hole: pmm(&r, "HOLESIZE") }),
            _ => {}
        }
    }
    p
}

fn from_streams(s: &BTreeMap<String, Vec<u8>>) -> Pcb {
    let get = |k: &str| s.get(k).map(Vec::as_slice).unwrap_or(&[]);
    Pcb {
        board: prop_records(get("Board6/Data")),
        components: prop_records(get("Components6/Data")),
        nets: prop_records(get("Nets6/Data")).into_iter().map(|r| r.get("NAME").cloned().unwrap_or_default()).collect(),
        polygons: prop_records(get("Polygons6/Data")),
        pads: pads6(get("Pads6/Data")),
        tracks: tracks6(get("Tracks6/Data")),
        arcs: arcs6(get("Arcs6/Data")),
        vias: vias6(get("Vias6/Data")),
    }
}

/// Outline vertices of `Board6`: VX<n>, VY<n>, KIND<n> (1: an arc to the next vertex, about CX/CY).
fn outline(board: &[Props]) -> Vec<P> {
    let Some(r) = board.iter().find(|r| r.contains_key("VX0")) else { return vec![] };
    let mut out = Vec::new();
    let mut i = 0;
    while let (Some(x), Some(y)) = (r.get(&format!("VX{i}")), r.get(&format!("VY{i}"))) {
        let a = [len_mm(x), len_mm(y)];
        let kind = r.get(&format!("KIND{i}")).map(|k| k.trim() == "1").unwrap_or(false);
        if kind {
            let c = [pmm(r, &format!("CX{i}")), pmm(r, &format!("CY{i}"))];
            let rad = pmm(r, &format!("R{i}"));
            let (sa, ea) = (pnum(r, &format!("SA{i}")), pnum(r, &format!("EA{i}")));
            let mut arc = build::arc(c, rad, sa, (ea - sa).rem_euclid(360.0));
            // Run the arc from this vertex's end.
            if geom::dist(arc[0], a) > geom::dist(*arc.last().unwrap(), a) {
                arc.reverse();
            }
            out.extend(arc);
        } else {
            out.push(a);
        }
        i += 1;
    }
    out
}

pub fn read_pcb(bytes: &[u8]) -> Result<BoardIn, String> {
    let pcb = if super::archive::is_cfb(bytes) { from_streams(&super::archive::cfb_streams(bytes)?) } else { from_ascii(&String::from_utf8_lossy(bytes)) };
    let mut b = BoardIn::default();
    let mut mids: Vec<u8> = pcb.tracks.iter().map(|t| t.layer).chain(pcb.arcs.iter().map(|a| a.layer)).filter(|l| (2..=31).contains(l) || (39..=54).contains(l)).collect();
    mids.sort();
    mids.dedup();
    b.source_copper_layers = 2 + mids.len();
    let mut o = outline(&pcb.board);
    if o.len() >= 3 {
        o.push(o[0]);
        b.edges.push(o);
    }
    // Components (pads, silk, copper grouped by component index).
    let mut parts: Vec<PartIn> = pcb
        .components
        .iter()
        .map(|c| {
            let bottom = c.get("LAYER").is_some_and(|l| l.eq_ignore_ascii_case("BOTTOM"));
            PartIn {
                reference: c.get("SOURCEDESIGNATOR").cloned().unwrap_or_default(),
                value: c.get("COMMENT").cloned().or_else(|| c.get("SOURCEDESCRIPTION").cloned()).unwrap_or_default(),
                footprint: c.get("PATTERN").cloned().unwrap_or_default(),
                at: [pmm(c, "X"), pmm(c, "Y")],
                rotation: pnum(c, "ROTATION"),
                side: if bottom { Side::Bottom } else { Side::Top },
                description: c.get("SOURCEDESCRIPTION").cloned().unwrap_or_default(),
                ..Default::default()
            }
        })
        .collect();
    let mut free = 0;
    for p in &pcb.pads {
        let (shape, corner) = pad_shape(p.shape, p.corner, p.size);
        let mut q = PadIn::new(p.name.clone(), p.at, p.size);
        q.shape = shape;
        q.corner = corner;
        q.rotation = p.rotation;
        q.net = net_of(&pcb.nets, p.net);
        if p.layer == 74 || p.hole > 0.0 {
            if p.hole <= 0.0 {
                continue;
            }
            q.kind = if p.plated { PadKind::Tht } else { PadKind::Npth };
            q.drill = Some(if p.slot > p.hole { if (p.slot_rotation.rem_euclid(180.0) - 90.0).abs() < 1.0 { [p.hole, p.slot] } else { [p.slot, p.hole] } } else { [p.hole, p.hole] });
            if q.kind == PadKind::Tht && q.size[0].min(q.size[1]) <= p.hole {
                q.size = [q.size[0].max(p.hole + 0.2), q.size[1].max(p.hole + 0.2)];
            }
        } else if p.layer != 1 && p.layer != 32 {
            continue;
        }
        if p.shape == 3 {
            b.note("octagonal pads became rectangles");
        }
        match parts.get_mut(p.component as usize).filter(|_| p.component != u16::MAX) {
            Some(part) => part.pads.push(q),
            None => {
                free += 1;
                b.parts.push(PartIn { reference: format!("FP{free}"), footprint: "free_pad".into(), at: p.at, pads: vec![q], ..Default::default() });
            }
        }
    }
    for t in &pcb.tracks {
        match t.layer {
            33 | 34 => {
                if let Some(part) = parts.get_mut(t.component as usize).filter(|_| t.component != u16::MAX) {
                    part.silk.push(vec![t.a, t.b]);
                }
            }
            l if (57..=72).contains(&l) && t.component == u16::MAX => {
                // A mechanical layer: board outlines are usually drawn here when Board6 has none.
                if pcb.board.iter().all(|r| !r.contains_key("VX0")) {
                    b.edges.push(vec![t.a, t.b]);
                }
            }
            l => {
                if let Some(layer) = layer_of(l, &mut b, &mids) {
                    b.seg(net_of(&pcb.nets, t.net), &layer, t.width, t.a, t.b);
                }
            }
        }
    }
    for a in &pcb.arcs {
        let span = (a.end - a.start).rem_euclid(360.0);
        let pts = build::arc(a.center, a.radius, a.start, if span == 0.0 { 360.0 } else { span });
        match a.layer {
            33 | 34 => {
                if let Some(part) = parts.get_mut(a.component as usize).filter(|_| a.component != u16::MAX) {
                    part.silk.push(pts);
                }
            }
            l if (57..=72).contains(&l) && a.component == u16::MAX => {
                if pcb.board.iter().all(|r| !r.contains_key("VX0")) {
                    b.edges.push(pts);
                }
            }
            l => {
                if let Some(layer) = layer_of(l, &mut b, &mids) {
                    b.polyline(net_of(&pcb.nets, a.net), &layer, a.width, &pts);
                }
            }
        }
    }
    for v in &pcb.vias {
        b.vias.push(KVia { net: net_of(&pcb.nets, v.net), at: v.at, drill: v.hole, diameter: v.diameter });
    }
    for poly in &pcb.polygons {
        let net = poly.get("NET").and_then(|n| n.trim().parse::<u16>().ok()).and_then(|i| net_of(&pcb.nets, i));
        let layer = match poly.get("LAYER").map(|l| l.to_ascii_uppercase()).as_deref() {
            Some("TOP") => Some("top"),
            Some("BOTTOM") => Some("bottom"),
            _ => None,
        };
        let mut pts = Vec::new();
        let mut i = 0;
        while let (Some(x), Some(y)) = (poly.get(&format!("VX{i}")), poly.get(&format!("VY{i}"))) {
            pts.push([len_mm(x), len_mm(y)]);
            i += 1;
        }
        if let (Some(net), Some(layer), true) = (net, layer, pts.len() >= 3) {
            b.zones.push(KZone { net, layer: layer.into(), outline: pts, clearance: None, min_width: 0.25, priority: pnum(poly, "POURINDEX") as i32, connection: PadConnection::Thermal, thermal_gap: None, thermal_spoke: None });
        }
    }
    b.parts.extend(parts);
    b.note("Altium component values come from the Comment parameter when present");
    Ok(b)
}

// ---- Schematic -----------------------------------------------------------------------------

/// A coordinate with its fraction (10 mil units + 1/100000 of them) in mm.
fn sc(r: &Props, k: &str) -> f64 {
    let whole = pnum(r, k);
    let frac = pnum(r, &format!("{k}_FRAC"));
    (whole + frac / 100000.0) * 0.254
}

fn spt(r: &Props, x: &str, y: &str) -> P {
    [sc(r, x), sc(r, y)]
}

fn etype(e: f64) -> PinType {
    match e as i64 {
        0 => PinType::Input,
        1 => PinType::Bidirectional,
        2 => PinType::Output,
        3 => PinType::OpenCollector,
        4 => PinType::Passive,
        5 => PinType::TriState,
        6 => PinType::OpenEmitter,
        7 => PinType::PowerIn,
        _ => PinType::Unspecified,
    }
}

fn sch_records(bytes: &[u8]) -> Result<Vec<Props>, String> {
    if super::archive::is_cfb(bytes) {
        let s = super::archive::cfb_streams(bytes)?;
        let fh = s.get("FileHeader").ok_or("the SchDoc has no FileHeader stream")?;
        Ok(prop_records(fh))
    } else {
        Ok(String::from_utf8_lossy(bytes).lines().map(props).filter(|r| !r.is_empty()).collect())
    }
}

pub fn read_sch(bytes: &[u8]) -> Result<Draft, String> {
    let recs = sch_records(bytes)?;
    // Records after the header are numbered from 0; OWNERINDEX refers to that number.
    let start = recs.iter().position(|r| r.contains_key("HEADER")).map(|i| i + 1).unwrap_or(0);
    let items: Vec<&Props> = recs[start..].iter().collect();
    let mut d = Draft { y_up: true, sheets: vec![("1".into(), "Sheet 1".into())], ..Default::default() };
    let sid = "1".to_string();
    // Components by record index.
    let mut comps: BTreeMap<usize, DraftSymbol> = BTreeMap::new();
    let mut parts: BTreeMap<usize, i64> = BTreeMap::new();
    for (i, r) in items.iter().enumerate() {
        if r.get("RECORD").map(String::as_str) == Some("1") {
            let at = spt(r, "LOCATION.X", "LOCATION.Y");
            comps.insert(i, DraftSymbol { sheet: sid.clone(), lib: format!("altium:{}", r.get("LIBREFERENCE").cloned().unwrap_or_default()), at, unit: pnum(r, "CURRENTPARTID").max(1.0) as u32, description: r.get("COMPONENTDESCRIPTION").cloned().unwrap_or_default(), ..Default::default() });
            parts.insert(i, pnum(r, "CURRENTPARTID").max(1.0) as i64);
        }
    }
    // Children: only those drawn for the component's current part.
    let owner = |r: &Props| -> Option<usize> { r.get("OWNERINDEX").and_then(|v| v.trim().parse::<usize>().ok()) };
    let mine = |r: &Props, o: usize| -> bool { r.get("OWNERPARTID").and_then(|v| v.trim().parse::<i64>().ok()).is_none_or(|p| p < 1 || parts.get(&o) == Some(&p)) };
    for r in &items {
        let kind = r.get("RECORD").map(String::as_str).unwrap_or("");
        let Some(o) = owner(r).filter(|o| comps.contains_key(o)) else { continue };
        if !mine(r, o) {
            continue;
        }
        let c = comps.get_mut(&o).unwrap();
        match kind {
            "2" => {
                let body = spt(r, "LOCATION.X", "LOCATION.Y");
                let len = sc(r, "PINLENGTH");
                let cong = pnum(r, "PINCONGLOMERATE") as i64;
                let dir = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]][(cong & 3) as usize];
                let tip = [body[0] + dir[0] * len, body[1] + dir[1] * len];
                let number = r.get("DESIGNATOR").cloned().unwrap_or_default();
                c.pins.push(PinDef { number, name: r.get("NAME").cloned().map(|n| crate::kicad_lib::plain(&n.replace('\\', ""))).filter(|n| !n.is_empty()).unwrap_or_else(|| "~".into()), at: tip, side: if len > 0.0 { sch::side_from(body, tip) } else { Dir::Left }, length: len, etype: etype(pnum(r, "ELECTRICAL")), hidden: cong & 4 != 0, unit: 0, shape: Default::default(), pads: vec![] });
            }
            "34" => c.reference = r.get("TEXT").cloned().unwrap_or_default(),
            "41" => {
                let (name, text) = (r.get("NAME").cloned().unwrap_or_default(), r.get("TEXT").cloned().unwrap_or_default());
                match name.to_ascii_uppercase().as_str() {
                    "COMMENT" | "VALUE" => {
                        if !text.starts_with('=') {
                            c.value = text
                        }
                    }
                    "MANUFACTURER PART NUMBER" | "MPN" | "PART NUMBER" => c.mpn = Some(text),
                    "MANUFACTURER" => c.manufacturer = Some(text),
                    _ => {
                        if !text.is_empty() && !text.starts_with('=') && !text.starts_with('*') {
                            c.fields.insert(name, text);
                        }
                    }
                }
            }
            "45" if r.get("MODELTYPE").is_some_and(|m| m.eq_ignore_ascii_case("PCBLIB")) && (c.footprint.is_empty() || ptrue(r, "ISCURRENT")) => c.footprint = r.get("MODELNAME").cloned().unwrap_or_default(),
            "13" => c.gfx.push(Gfx::Line { pts: vec![spt(r, "LOCATION.X", "LOCATION.Y"), spt(r, "CORNER.X", "CORNER.Y")], width: 0.0 }),
            "6" | "7" => {
                let n = pnum(r, "LOCATIONCOUNT") as usize;
                let pts: Vec<P> = (1..=n).map(|k| spt(r, &format!("X{k}"), &format!("Y{k}"))).collect();
                if pts.len() >= 2 {
                    c.gfx.push(if kind == "7" && pts.len() >= 3 { Gfx::Poly { pts, fill: Fill::Outline } } else { Gfx::Line { pts, width: 0.0 } });
                }
            }
            "14" | "10" => {
                let (a, b) = (spt(r, "LOCATION.X", "LOCATION.Y"), spt(r, "CORNER.X", "CORNER.Y"));
                c.gfx.push(Gfx::Poly { pts: vec![a, [b[0], a[1]], b, [a[0], b[1]]], fill: if ptrue(r, "ISSOLID") { Fill::Body } else { Fill::None } });
            }
            "8" => c.gfx.push(Gfx::Circle { c: spt(r, "LOCATION.X", "LOCATION.Y"), r: sc(r, "RADIUS"), fill: if ptrue(r, "ISSOLID") { Fill::Outline } else { Fill::None } }),
            "12" | "11" => {
                let (sa, ea) = (pnum(r, "STARTANGLE"), pnum(r, "ENDANGLE"));
                let pts = build::arc(spt(r, "LOCATION.X", "LOCATION.Y"), sc(r, "RADIUS"), sa, (ea - sa).rem_euclid(360.0).max(1e-3));
                c.gfx.push(Gfx::Line { pts, width: 0.0 });
            }
            "4" => {
                if let Some(t) = r.get("TEXT") {
                    c.gfx.push(Gfx::Text { at: spt(r, "LOCATION.X", "LOCATION.Y"), text: t.clone(), size: 1.27 });
                }
            }
            _ => {}
        }
    }
    for r in &items {
        let kind = r.get("RECORD").map(String::as_str).unwrap_or("");
        if owner(r).is_some_and(|o| comps.contains_key(&o)) {
            continue;
        }
        match kind {
            "27" => {
                let n = pnum(r, "LOCATIONCOUNT") as usize;
                let pts: Vec<P> = (1..=n).map(|k| spt(r, &format!("X{k}"), &format!("Y{k}"))).collect();
                if pts.len() >= 2 {
                    d.wires.push((sid.clone(), pts));
                }
            }
            "29" => d.junctions.push((sid.clone(), spt(r, "LOCATION.X", "LOCATION.Y"))),
            "22" => d.no_connects.push((sid.clone(), spt(r, "LOCATION.X", "LOCATION.Y"))),
            "25" => d.labels.push((sid.clone(), LabelKind::Local, r.get("TEXT").cloned().unwrap_or_default(), spt(r, "LOCATION.X", "LOCATION.Y"), pnum(r, "ORIENTATION") * 90.0)),
            "17" => {
                let text = r.get("TEXT").cloned().unwrap_or_default();
                // Orientation 0..3: the port's symbol points right, up, left, down from its anchor.
                let ang = pnum(r, "ORIENTATION").rem_euclid(4.0) * 90.0;
                d.labels.push((sid.clone(), LabelKind::Power, text, spt(r, "LOCATION.X", "LOCATION.Y"), ang));
            }
            "18" => d.labels.push((sid.clone(), LabelKind::Global, r.get("NAME").cloned().unwrap_or_default(), spt(r, "LOCATION.X", "LOCATION.Y"), 0.0)),
            "4" => {
                if let Some(t) = r.get("TEXT").filter(|t| !t.starts_with('=')) {
                    d.texts.push((sid.clone(), spt(r, "LOCATION.X", "LOCATION.Y"), t.clone(), 1.8));
                }
            }
            _ => {}
        }
    }
    for (_, mut c) in comps {
        if c.reference.is_empty() {
            c.reference = format!("U{}", d.symbols.len() + 1);
        }
        if c.pins.is_empty() && c.gfx.is_empty() {
            continue;
        }
        d.symbols.push(c);
    }
    d.note("Altium schematics are read as one sheet; sheet symbols and harnesses are not followed");
    Ok(d)
}
