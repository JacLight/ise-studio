//! Fabrication files: Gerber RS-274X with X2 attributes (one file per layer:
//! copper, solder mask, paste, silkscreen, board profile) and Excellon drill
//! files (plated and unplated apart), millimetres throughout.
//!
//! Pads flash standard apertures (circle, rectangle, obround) when they sit
//! square, and a `RoundRect` aperture macro otherwise; zone fills are regions
//! with their holes joined to the outline by cut-ins, as the Gerber
//! specification describes. Pads, tracks and vias carry X2 object attributes
//! (net, component, pin), so a viewer can show the netlist from the files.

use crate::board::{BoardView, PadView};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{self, P, Prim};
use std::collections::HashMap;
use std::fmt::Write;

/// Gerber coordinates: 4 integer and 6 decimal digits, mm.
fn c(v: f64) -> i64 {
    (v * 1e6).round() as i64
}

fn n(v: f64) -> String {
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

/// Text safe inside an attribute field.
fn field(s: &str) -> String {
    s.chars().map(|ch| if matches!(ch, ',' | '*' | '%' | '\\') || ch.is_control() { '_' } else { ch }).collect()
}

const ROUNDRECT_MACRO: &str = "%AMRoundRect*\n0 Rectangle with rounded corners: $1 radius, $2..$9 the corner centres*\n4,1,4,$2,$3,$4,$5,$6,$7,$8,$9,$2,$3,0*\n1,1,$1+$1,$2,$3*\n1,1,$1+$1,$4,$5*\n1,1,$1+$1,$6,$7*\n1,1,$1+$1,$8,$9*\n20,1,$1+$1,$2,$3,$4,$5,0*\n20,1,$1+$1,$4,$5,$6,$7,0*\n20,1,$1+$1,$6,$7,$8,$9,0*\n20,1,$1+$1,$8,$9,$2,$3,0*%\n";

pub struct Gerber {
    function: String,
    negative: bool,
    header: String,
    defs: String,
    body: String,
    apertures: HashMap<String, u32>,
    next: u32,
    current: Option<u32>,
    macro_used: bool,
}

impl Gerber {
    pub fn new(view: &BoardView, function: &str, negative: bool) -> Gerber {
        let mut header = String::new();
        let _ = writeln!(header, "%TF.GenerationSoftware,Forma,PCB,{}*%", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(header, "%TF.ProjectId,{},{},{}*%", field(&view.name).replace(' ', "_"), view.id, field(&view.revision));
        let _ = writeln!(header, "%TF.SameCoordinates,Original*%");
        let _ = writeln!(header, "%TF.FileFunction,{function}*%");
        let _ = writeln!(header, "%TF.FilePolarity,{}*%", if negative { "Negative" } else { "Positive" });
        let _ = writeln!(header, "%FSLAX46Y46*%");
        let _ = writeln!(header, "G04 Gerber Fmt 4.6, Leading zero omitted, Abs format (unit mm)*");
        let _ = writeln!(header, "G04 {} rev {}, made by Forma*", field(&view.name), field(&view.revision));
        let _ = writeln!(header, "%MOMM*%");
        let _ = writeln!(header, "%LPD*%");
        let _ = writeln!(header, "G01*");
        Gerber { function: function.into(), negative, header, defs: String::new(), body: String::new(), apertures: HashMap::new(), next: 10, current: None, macro_used: false }
    }

    fn aperture(&mut self, def: String, function: &str) -> u32 {
        let key = format!("{def}|{function}");
        if let Some(d) = self.apertures.get(&key) {
            return *d;
        }
        let d = self.next;
        self.next += 1;
        if def.starts_with("RoundRect") {
            self.macro_used = true;
        }
        if !function.is_empty() {
            let _ = writeln!(self.defs, "%TA.AperFunction,{function}*%");
        }
        let _ = writeln!(self.defs, "%ADD{d}{def}*%");
        if !function.is_empty() {
            let _ = writeln!(self.defs, "%TD*%");
        }
        self.apertures.insert(key, d);
        d
    }

    fn select(&mut self, d: u32) {
        if self.current != Some(d) {
            let _ = writeln!(self.body, "D{d}*");
            self.current = Some(d);
        }
    }

    fn attrs(&mut self, attrs: &[(&str, String)]) {
        for (k, v) in attrs {
            let _ = writeln!(self.body, "%TO.{k},{v}*%");
        }
    }

    fn clear_attrs(&mut self, any: bool) {
        if any {
            let _ = writeln!(self.body, "%TD*%");
        }
    }

    pub fn flash(&mut self, d: u32, p: P) {
        self.select(d);
        let _ = writeln!(self.body, "X{}Y{}D03*", c(p[0]), c(p[1]));
    }

    pub fn polyline(&mut self, width: f64, function: &str, pts: &[P], attrs: &[(&str, String)]) {
        if pts.len() < 2 {
            return;
        }
        let d = self.aperture(format!("C,{}", n(width)), function);
        self.attrs(attrs);
        self.select(d);
        let _ = writeln!(self.body, "X{}Y{}D02*", c(pts[0][0]), c(pts[0][1]));
        for p in &pts[1..] {
            let _ = writeln!(self.body, "X{}Y{}D01*", c(p[0]), c(p[1]));
        }
        self.clear_attrs(!attrs.is_empty());
    }

    /// A filled region: an outer contour (counter-clockwise) with holes, joined by cut-ins.
    pub fn region(&mut self, contours: &[Vec<P>], function: &str, attrs: &[(&str, String)]) {
        let path = cut_in(contours);
        if path.len() < 3 {
            return;
        }
        if !function.is_empty() {
            let _ = writeln!(self.body, "%TA.AperFunction,{function}*%");
        }
        self.attrs(attrs);
        let _ = writeln!(self.body, "G36*");
        let _ = writeln!(self.body, "X{}Y{}D02*", c(path[0][0]), c(path[0][1]));
        for p in path.iter().skip(1).chain(std::iter::once(&path[0])) {
            let _ = writeln!(self.body, "X{}Y{}D01*", c(p[0]), c(p[1]));
        }
        let _ = writeln!(self.body, "G37*");
        self.clear_attrs(!attrs.is_empty() || !function.is_empty());
    }

    /// A pad (or its mask/paste opening, grown by `grow`), flashed or as a region.
    pub fn pad(&mut self, p: &PadView, grow: f64, function: &str, attrs: &[(&str, String)]) {
        if let Some(poly) = &p.polygon {
            // A custom pad: its outline as a region (grown for the mask opening).
            let contours = if grow.abs() > 1e-9 { geom::contours(&manifold_csg::CrossSection::from_polygons(&[poly.clone()]).offset(grow, manifold_csg::JoinType::Round, 2.0, 16)) } else { vec![poly.clone()] };
            for c in contours.iter().filter(|c| geom::polygon_area(c) > 0.0) {
                self.region(&[c.clone()], function, attrs);
            }
            return;
        }
        let (w, h) = (p.size[0] + 2.0 * grow, p.size[1] + 2.0 * grow);
        let quarter = (p.rotation / 90.0).round();
        let square = (p.rotation - quarter * 90.0).abs() < 1e-6;
        let swap = square && (quarter as i64).rem_euclid(2) == 1;
        let (sw, sh) = if swap { (h, w) } else { (w, h) };
        let def = match p.shape {
            PadShape::Circle => Some(format!("C,{}", n(w.max(h)))),
            PadShape::Rect if square => Some(format!("R,{}X{}", n(sw), n(sh))),
            PadShape::Oval if square => Some(if (sw - sh).abs() < 1e-9 { format!("C,{}", n(sw)) } else { format!("O,{}X{}", n(sw), n(sh)) }),
            PadShape::RoundRect | PadShape::Rect => {
                let r = if p.shape == PadShape::Rect { 0.0 } else { p.corner + grow };
                Some(round_rect(w, h, r, p.rotation))
            }
            PadShape::Oval | PadShape::Custom => None,
        };
        match def {
            Some(def) => {
                let d = self.aperture(def, function);
                self.attrs(attrs);
                self.flash(d, p.at);
                self.clear_attrs(!attrs.is_empty());
            }
            None => {
                let prim = Prim { pts: p.prim.pts.clone(), r: p.prim.r + grow };
                self.region(&[prim.outline(0.002)], function, attrs);
            }
        }
    }

    pub fn finish(self) -> String {
        let mut s = self.header;
        if self.macro_used {
            s.push_str(ROUNDRECT_MACRO);
        }
        s.push_str(&self.defs);
        s.push_str(&self.body);
        s.push_str("M02*\n");
        let _ = (self.function, self.negative);
        s
    }
}

/// `RoundRect` macro aperture: radius, then the four corner-circle centres (turned).
fn round_rect(w: f64, h: f64, r: f64, rot: f64) -> String {
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let (hw, hh) = (w / 2.0 - r, h / 2.0 - r);
    let mut s = format!("RoundRect,{}", n(r));
    for q in [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]] {
        let p = geom::rotate(q, rot);
        let _ = write!(s, "X{}X{}", n(p[0]), n(p[1]));
    }
    s
}

/// One contour from an outer contour and its holes: each hole joined to what
/// is already merged by a zero-width bridge, rightmost hole first, along a
/// ray to +x from its rightmost point (which can only meet merged contours).
pub fn cut_in(contours: &[Vec<P>]) -> Vec<P> {
    let mut outers: Vec<&Vec<P>> = contours.iter().filter(|c| geom::polygon_area(c) > 0.0).collect();
    if outers.is_empty() {
        return vec![];
    }
    outers.sort_by(|a, b| geom::polygon_area(b).total_cmp(&geom::polygon_area(a)));
    let mut path = outers[0].clone();
    let mut holes: Vec<Vec<P>> = contours.iter().filter(|c| geom::polygon_area(c) < 0.0).cloned().collect();
    holes.sort_by(|a, b| {
        let ma = a.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let mb = b.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        mb.total_cmp(&ma)
    });
    for hole in holes {
        let (hi, hp) = hole.iter().enumerate().max_by(|a, b| a.1[0].total_cmp(&b.1[0])).map(|(i, p)| (i, *p)).unwrap();
        // Nearest crossing of the ray y = hp.y, x > hp.x with the merged path.
        let mut best: Option<(f64, usize, P)> = None;
        let m = path.len();
        for i in 0..m {
            let (a, b) = (path[i], path[(i + 1) % m]);
            if (a[1] - hp[1]) * (b[1] - hp[1]) > 0.0 || (a[1] - b[1]).abs() < 1e-15 {
                continue;
            }
            let t = (hp[1] - a[1]) / (b[1] - a[1]);
            let x = a[0] + t * (b[0] - a[0]);
            if x > hp[0] - 1e-12 && best.is_none_or(|(bx, _, _)| x < bx) {
                best = Some((x, i, [x, hp[1]]));
            }
        }
        let Some((_, i, q)) = best else { continue };
        // path[..=i], q, hole from hi around back to hi, hp, q, path[i+1..]
        let mut merged = Vec::with_capacity(path.len() + hole.len() + 3);
        merged.extend_from_slice(&path[..=i]);
        merged.push(q);
        for k in 0..=hole.len() {
            merged.push(hole[(hi + k) % hole.len()]);
        }
        merged.push(q);
        merged.extend_from_slice(&path[i + 1..]);
        path = merged;
    }
    path
}

fn pad_function(p: &PadView) -> &'static str {
    match p.kind {
        PadKind::Smd => "SMDPad,CuDef",
        PadKind::Tht => "ComponentPad",
        PadKind::Npth => "ComponentPad",
    }
}

fn pad_attrs(p: &PadView) -> Vec<(&'static str, String)> {
    let mut a = vec![("C", field(&p.reference))];
    if !p.number.is_empty() {
        a.push(("P", match &p.pin_name {
            Some(f) => format!("{},{},{}", field(&p.reference), field(&p.number), field(f)),
            None => format!("{},{}", field(&p.reference), field(&p.number)),
        }));
    }
    a.push(("N", field(p.net.as_deref().unwrap_or("N/C"))));
    a
}

/// One copper layer.
pub fn copper(view: &BoardView, layer: &str) -> String {
    let idx = view.layers.iter().position(|l| l == layer).unwrap_or(0);
    let total = view.layers.len();
    let pos = if idx == 0 { "Top" } else if idx == total - 1 { "Bot" } else { "Inr" };
    let mut g = Gerber::new(view, &format!("Copper,L{},{pos}", idx + 1), false);
    for z in view.zones.iter().filter(|z| z.layer == layer) {
        for island in &z.fill {
            g.region(island, "Conductor", &[("N", field(&z.net))]);
        }
    }
    for t in view.tracks.iter().filter(|t| t.layer == layer) {
        g.polyline(t.width, "Conductor", &t.points, &[("N", field(t.net.as_deref().unwrap_or("N/C")))]);
    }
    for v in &view.vias {
        let d = g.aperture(format!("C,{}", n(v.diameter)), "ViaPad");
        g.attrs(&[("N", field(v.net.as_deref().unwrap_or("N/C")))]);
        g.flash(d, v.at);
        g.clear_attrs(true);
    }
    for p in view.pads.iter().filter(|p| p.layers.iter().any(|l| l == layer)) {
        g.pad(p, 0.0, pad_function(p), &pad_attrs(p));
    }
    g.finish()
}

/// Solder mask openings for one side (negative: the image is where mask is absent).
pub fn mask(view: &BoardView, top: bool) -> String {
    let mut g = Gerber::new(view, if top { "Soldermask,Top" } else { "Soldermask,Bot" }, true);
    let side_layer = if top { view.layers[0].clone() } else { view.layers[view.layers.len() - 1].clone() };
    let e = view.rules.mask_expansion;
    for p in &view.pads {
        let on_side = p.layers.contains(&side_layer) || p.kind == PadKind::Npth;
        if on_side {
            g.pad(p, if p.kind == PadKind::Npth { 0.0 } else { e }, pad_function(p), &[]);
        }
    }
    g.finish()
}

/// Paste stencil openings for one side.
pub fn paste(view: &BoardView, top: bool) -> String {
    let mut g = Gerber::new(view, if top { "Paste,Top" } else { "Paste,Bot" }, false);
    let side_layer = if top { view.layers[0].clone() } else { view.layers[view.layers.len() - 1].clone() };
    for p in view.pads.iter().filter(|p| p.kind == PadKind::Smd && p.layers.contains(&side_layer)) {
        if p.paste.len() == 1 && p.paste[0] == p.prim {
            g.pad(p, 0.0, "SMDPad,CuDef", &[]);
        } else {
            for w in &p.paste {
                g.region(&[w.outline(0.002)], "SMDPad,CuDef", &[]);
            }
        }
    }
    g.finish()
}

/// Silkscreen for one side: outlines and reference designators.
pub fn silk(view: &BoardView, top: bool) -> String {
    let mut g = Gerber::new(view, if top { "Legend,Top" } else { "Legend,Bot" }, false);
    let w = view.rules.silk_width;
    let side = if top { crate::Side::Top } else { crate::Side::Bottom };
    for comp in view.components.iter().filter(|c| c.side == side) {
        for l in &comp.silk {
            g.polyline(w, "Other,Outline", l, &[("C", field(&comp.reference))]);
        }
        if comp.kind != "hole" && comp.kind != "logo" {
            for l in crate::font::strokes(&comp.reference, comp.label, view.rules.silk_height, !top) {
                g.polyline(w, "Other,Reference", &l, &[]);
            }
        }
    }
    g.finish()
}

/// The board outline (and cutouts) as a closed profile.
pub fn profile(view: &BoardView) -> String {
    let mut g = Gerber::new(view, "Profile,NP", false);
    for c in std::iter::once(&view.outline).chain(view.cutouts.iter()) {
        let mut pts = c.clone();
        pts.push(c[0]);
        g.polyline(0.1, "Profile", &pts, &[]);
    }
    g.finish()
}

/// Excellon drill file, plated or not: decimal millimetres, slots routed with G85.
pub fn drill(view: &BoardView, plated: bool) -> String {
    let mut tools: Vec<f64> = Vec::new();
    let holes: Vec<&crate::board::Hole> = view.holes.iter().filter(|h| h.plated == plated).collect();
    for h in &holes {
        let d = (h.size[0].min(h.size[1]) * 1000.0).round() / 1000.0;
        if !tools.iter().any(|t| (t - d).abs() < 1e-9) {
            tools.push(d);
        }
    }
    tools.sort_by(f64::total_cmp);
    let n_layers = view.layers.len();
    let mut s = String::new();
    let _ = writeln!(s, "M48");
    let _ = writeln!(s, "; DRILL file: {} rev {} ({})", view.name, view.revision, if plated { "plated" } else { "not plated" });
    let _ = writeln!(s, "; FORMAT={{-:-/ absolute / metric / decimal}}");
    let _ = writeln!(s, "; #@! TF.GenerationSoftware,Forma,PCB,{}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "; #@! TF.FileFunction,{},1,{n_layers},{}", if plated { "Plated" } else { "NonPlated" }, if plated { "PTH" } else { "NPTH" });
    let _ = writeln!(s, "FMAT,2");
    let _ = writeln!(s, "METRIC");
    for (i, t) in tools.iter().enumerate() {
        let _ = writeln!(s, "; #@! TA.AperFunction,{}", if plated { "Plated,PTH,ComponentDrill" } else { "NonPlated,NPTH,ComponentDrill" });
        let _ = writeln!(s, "T{}C{:.3}", i + 1, t);
    }
    let _ = writeln!(s, "%");
    let _ = writeln!(s, "G90");
    let _ = writeln!(s, "G05");
    let f = |v: f64| format!("{:.3}", v);
    for (i, t) in tools.iter().enumerate() {
        let mine: Vec<&&crate::board::Hole> = holes.iter().filter(|h| ((h.size[0].min(h.size[1]) * 1000.0).round() / 1000.0 - t).abs() < 1e-9).collect();
        if mine.is_empty() {
            continue;
        }
        let _ = writeln!(s, "T{}", i + 1);
        for h in mine {
            if (h.size[0] - h.size[1]).abs() < 1e-9 {
                let _ = writeln!(s, "X{}Y{}", f(h.at[0]), f(h.at[1]));
            } else {
                // A slot: from end to end along its long axis.
                let half = (h.size[0].max(h.size[1]) - h.size[0].min(h.size[1])) / 2.0;
                let dir = if h.size[0] > h.size[1] { [half, 0.0] } else { [0.0, half] };
                let d = geom::rotate(dir, h.rotation);
                let (a, b) = (geom::sub(h.at, d), geom::add(h.at, d));
                let _ = writeln!(s, "X{}Y{}G85X{}Y{}", f(a[0]), f(a[1]), f(b[0]), f(b[1]));
            }
        }
    }
    let _ = writeln!(s, "T0");
    let _ = writeln!(s, "M30");
    s
}

/// Every fabrication file, named with Protel extensions (what JLCPCB, PCBWay
/// and OSH Park read without renaming): (file name, contents).
pub fn files(view: &BoardView) -> Vec<(String, String)> {
    let stem = file_stem(&view.name);
    let mut out = vec![(format!("{stem}-F_Cu.gtl"), copper(view, "top"))];
    for (i, l) in view.layers.iter().enumerate().filter(|(_, l)| l.starts_with("in")) {
        out.push((format!("{stem}-In{}_Cu.g{}l", i, i + 1), copper(view, l)));
    }
    out.push((format!("{stem}-B_Cu.gbl"), copper(view, "bottom")));
    out.push((format!("{stem}-F_Mask.gts"), mask(view, true)));
    out.push((format!("{stem}-B_Mask.gbs"), mask(view, false)));
    out.push((format!("{stem}-F_Paste.gtp"), paste(view, true)));
    out.push((format!("{stem}-B_Paste.gbp"), paste(view, false)));
    out.push((format!("{stem}-F_Silkscreen.gto"), silk(view, true)));
    out.push((format!("{stem}-B_Silkscreen.gbo"), silk(view, false)));
    out.push((format!("{stem}-Edge_Cuts.gm1"), profile(view)));
    out.push((format!("{stem}-PTH.drl"), drill(view, true)));
    out.push((format!("{stem}-NPTH.drl"), drill(view, false)));
    out
}

pub fn file_stem(name: &str) -> String {
    let s: String = name.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "board".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_in_keeps_the_area() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[2.0, 2.0], [2.0, 4.0], [4.0, 4.0], [4.0, 2.0]];
        let hole2 = vec![[6.0, 6.0], [6.0, 8.0], [8.0, 8.0], [8.0, 6.0]];
        let path = cut_in(&[outer, hole, hole2]);
        assert!((geom::polygon_area(&path) - (100.0 - 8.0)).abs() < 1e-9, "{}", geom::polygon_area(&path));
    }
}
