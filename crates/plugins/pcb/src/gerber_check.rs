//! The fabrication files checked the way a fab's CAM sees them: every Gerber
//! and drill file is read back with the independent reader
//! ([`crate::reader`]), painted into a raster (0.02 mm pixels by default) as
//! a photoplotter would expose it, and compared with the board itself:
//!
//! - **copper**, per layer: the exposed area against the board's own copper
//!   (pads, tracks, vias and pour fills painted the same way), the area that
//!   differs, every pad lit where it is, every pin present (X2 `.P`), and the
//!   copper of each net (X2 `.N`) against the board's; copper of two
//!   different nets touching in the files is a short.
//! - **mask**: every pad has its opening; **paste**: every SMD pad gets
//!   paste and no hole does; **silkscreen**: ink over exposed copper (a fab
//!   clips it) is reported.
//! - **drills**: every hole and slot at its place and size, plated and not
//!   plated apart.
//! - **outline**: the profile closes, and its area and extent are the board's.
//!
//! Nothing here shares code with the writers in [`crate::gerber`]: a wrong
//! aperture, a dropped pad, a region with a broken cut-in, a pour lost from
//! the file or a mislabelled net shows up as a difference. The layers are
//! also drawn as SVG from what was read ([`svg`]), for the Fab tab.

use crate::board::BoardView;
use crate::footprints::PadKind;
use crate::geom::{self, P, Prim};
use crate::reader::{self, GerberFile, Object, Shape};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// A painted image: one byte per pixel, 1 where exposed.
#[derive(Clone, Debug)]
pub struct Raster {
    pub x0: f64,
    pub y0: f64,
    pub px: f64,
    pub w: usize,
    pub h: usize,
    pub on: Vec<u8>,
}

impl Raster {
    /// A raster over `b` ([min x, min y, max x, max y]) with `margin` mm around it.
    pub fn new(b: [f64; 4], px: f64, margin: f64) -> Raster {
        let (x0, y0) = (b[0] - margin, b[1] - margin);
        let w = (((b[2] - b[0]) + 2.0 * margin) / px).ceil().max(1.0) as usize;
        let h = (((b[3] - b[1]) + 2.0 * margin) / px).ceil().max(1.0) as usize;
        Raster { x0, y0, px, w, h, on: vec![0; w * h] }
    }

    pub fn area(&self) -> f64 {
        self.on.iter().filter(|v| **v != 0).count() as f64 * self.px * self.px
    }

    pub fn at(&self, p: P) -> bool {
        let i = ((p[0] - self.x0) / self.px).floor();
        let j = ((p[1] - self.y0) / self.px).floor();
        i >= 0.0 && j >= 0.0 && (i as usize) < self.w && (j as usize) < self.h && self.on[j as usize * self.w + i as usize] != 0
    }

    /// Pixels set in one and not the other (same grid), as an area.
    pub fn xor_area(&self, o: &Raster) -> f64 {
        self.on.iter().zip(&o.on).filter(|(a, b)| (**a != 0) != (**b != 0)).count() as f64 * self.px * self.px
    }

    /// Fills polygons together by the non-zero rule, setting pixels whose centres are inside to `value`.
    pub fn fill(&mut self, polys: &[Vec<P>], value: u8) {
        let w = self.w;
        let mut rows: Vec<(usize, usize, usize)> = Vec::new();
        self.spans(polys, |j, i0, i1| rows.push((j, i0, i1)));
        for (j, i0, i1) in rows {
            self.on[j * w + i0..j * w + i1].fill(value);
        }
    }

    /// Calls `f(row, first, end)` for each run of pixel centres inside the polygons (non-zero rule).
    fn spans(&self, polys: &[Vec<P>], mut f: impl FnMut(usize, usize, usize)) {
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for p in polys {
            for q in p {
                ymin = ymin.min(q[1]);
                ymax = ymax.max(q[1]);
            }
        }
        if !ymin.is_finite() {
            return;
        }
        let j0 = (((ymin - self.y0) / self.px) - 0.5).ceil().max(0.0) as usize;
        let j1 = ((((ymax - self.y0) / self.px) - 0.5).floor().max(-1.0) + 1.0).min(self.h as f64) as usize;
        let mut xs: Vec<(f64, i32)> = Vec::new();
        for j in j0..j1 {
            let y = self.y0 + (j as f64 + 0.5) * self.px;
            xs.clear();
            for p in polys {
                let n = p.len();
                for k in 0..n {
                    let (a, b) = (p[k], p[(k + 1) % n]);
                    if (a[1] <= y) != (b[1] <= y) {
                        let x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                        xs.push((x, if b[1] > a[1] { 1 } else { -1 }));
                    }
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for k in 0..xs.len() {
                wind += xs[k].1;
                if wind != 0 && k + 1 < xs.len() {
                    let (lo, hi) = (xs[k].0, xs[k + 1].0);
                    let i0 = (((lo - self.x0) / self.px) - 0.5).ceil().max(0.0) as usize;
                    let i1 = ((((hi - self.x0) / self.px) - 0.5).ceil().max(0.0) as usize).min(self.w);
                    if i1 > i0 {
                        f(j, i0, i1);
                    }
                }
            }
        }
    }

    /// Share of the pixels inside `poly` that are set, counting only pixels whose four neighbours
    /// are inside too (an edge falling exactly on pixel centres is ambiguous for any two
    /// tessellations of the same shape). 1.0 when it covers no such pixel.
    pub fn coverage(&self, poly: &[P]) -> f64 {
        let mut rows: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
        self.spans(&[poly.to_vec()], |j, i0, i1| rows.entry(j).or_default().push((i0, i1)));
        let inside = |j: usize, i: usize| rows.get(&j).is_some_and(|s| s.iter().any(|(a, b)| i >= *a && i < *b));
        let (mut count, mut lit) = (0usize, 0usize);
        for (j, spans) in &rows {
            for (a, b) in spans {
                for i in a + 1..b.saturating_sub(1) {
                    if *j == 0 || !inside(j - 1, i) || !inside(j + 1, i) {
                        continue;
                    }
                    count += 1;
                    lit += (self.on[j * self.w + i] != 0) as usize;
                }
            }
        }
        if count == 0 { 1.0 } else { lit as f64 / count as f64 }
    }

    fn clone_empty(&self) -> Raster {
        Raster { x0: self.x0, y0: self.y0, px: self.px, w: self.w, h: self.h, on: vec![] }
    }
}

const TOL: f64 = 0.002;

/// Paints what a Gerber file exposes, objects in order (clear objects remove).
pub fn paint(r: &mut Raster, g: &GerberFile, keep: impl Fn(&Object) -> bool) {
    for o in g.objects.iter().filter(|o| keep(o)) {
        for (exposed, poly) in g.outlines(o, TOL) {
            r.fill(&[poly], (o.dark && exposed) as u8);
        }
    }
}

fn prim_poly(p: &Prim) -> Vec<P> {
    p.outline(TOL)
}

/// A pour island with its outer boundary anticlockwise and holes clockwise, for non-zero filling.
fn island_polys(island: &[Vec<P>]) -> Vec<Vec<P>> {
    let Some(outer) = island.iter().enumerate().max_by(|a, b| geom::polygon_area(a.1).abs().total_cmp(&geom::polygon_area(b.1).abs())).map(|(i, _)| i) else { return vec![] };
    island
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut c = c.clone();
            let ccw = geom::polygon_area(&c) > 0.0;
            if (i == outer) != ccw {
                c.reverse();
            }
            c
        })
        .collect()
}

/// The board's own copper on a layer as polygons, by net ("N/C" for none).
fn board_copper(v: &BoardView, layer: &str) -> BTreeMap<String, Vec<Vec<Vec<P>>>> {
    let mut by: BTreeMap<String, Vec<Vec<Vec<P>>>> = BTreeMap::new();
    let key = |n: &Option<String>| n.clone().unwrap_or_else(|| "N/C".into());
    for p in v.pads.iter().filter(|p| p.layers.iter().any(|l| l == layer)) {
        let poly = p.polygon.clone().unwrap_or_else(|| prim_poly(&p.prim));
        by.entry(key(&p.net)).or_default().push(vec![poly]);
    }
    for t in v.tracks.iter().filter(|t| t.layer == layer) {
        for s in t.points.windows(2) {
            by.entry(key(&t.net)).or_default().push(vec![prim_poly(&Prim::capsule(s[0], s[1], t.width / 2.0))]);
        }
    }
    for via in &v.vias {
        by.entry(key(&via.net)).or_default().push(vec![prim_poly(&Prim::circle(via.at, via.diameter / 2.0))]);
    }
    for z in v.zones.iter().filter(|z| z.layer == layer) {
        for island in &z.fill {
            by.entry(z.net.clone()).or_default().push(island_polys(island));
        }
    }
    by
}

fn bbox_of(polys: impl Iterator<Item = P>) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut any = false;
    for p in polys {
        any = true;
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    any.then_some(b)
}

#[derive(Clone, Debug, Serialize)]
pub struct NetArea {
    pub net: String,
    /// mm²
    pub board: f64,
    pub files: f64,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LayerCheck {
    pub file: String,
    /// The X2 file function ("Copper,L1,Top", "Soldermask,Bot" …).
    pub function: String,
    pub objects: usize,
    /// Copper: board and file areas (mm²) and the area where they differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_area: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_area: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub differ_area: Option<f64>,
    /// Pads expected here and found exposed.
    pub pads: usize,
    pub pads_ok: usize,
    /// Pads not (fully) exposed, as "U1.3".
    pub missing: Vec<String>,
    /// Pins named in the file that the board does not have here.
    pub extra: Vec<String>,
    pub nets: Vec<NetArea>,
    /// Pairs of nets whose copper touches in the file.
    pub shorts: Vec<String>,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DrillCheck {
    pub file: String,
    pub plated: bool,
    pub board_holes: usize,
    pub file_holes: usize,
    /// Finished size (mm) → (board, file) counts.
    pub sizes: Vec<(f64, usize, usize)>,
    pub problems: Vec<String>,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OutlineCheck {
    pub file: String,
    pub loops: usize,
    pub board_area: f64,
    pub file_area: f64,
    pub board_size: [f64; 2],
    pub file_size: [f64; 2],
    pub problems: Vec<String>,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GerberCheck {
    pub ok: bool,
    /// Pixel size used (mm).
    pub resolution: f64,
    pub layers: Vec<LayerCheck>,
    pub drills: Vec<DrillCheck>,
    pub outline: Option<OutlineCheck>,
    /// Files that could not be read, with why.
    pub unreadable: Vec<String>,
    pub summary: String,
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Checks a set of fabrication files (name, text) against the board they were made for.
pub fn check(v: &BoardView, files: &[(String, String)], px: f64) -> GerberCheck {
    let mut out = GerberCheck { ok: true, resolution: px, layers: vec![], drills: vec![], outline: None, unreadable: vec![], summary: String::new() };
    let bounds = v.bounds;
    let mut masks: BTreeMap<bool, Raster> = BTreeMap::new();
    let mut silks: Vec<(bool, String, Raster)> = Vec::new();
    for (name, text) in files {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".drl") || lower.ends_with(".xln") {
            match reader::excellon(text) {
                Ok(d) => out.drills.push(check_drill(v, name, &d)),
                Err(e) => out.unreadable.push(format!("{name}: {e}")),
            }
            continue;
        }
        // Anything else that is a Gerber file (they open with an extended command or a comment).
        let head = text.trim_start();
        if !(head.starts_with('%') || head.starts_with("G04")) {
            continue;
        }
        let g = match reader::gerber(text) {
            Ok(g) => g,
            Err(e) => {
                out.unreadable.push(format!("{name}: {e}"));
                continue;
            }
        };
        let function = g.attributes.get("FileFunction").cloned().unwrap_or_default();
        let parts: Vec<&str> = function.split(',').collect();
        match parts.first().copied() {
            Some("Copper") => {
                let n: usize = parts.get(1).and_then(|l| l.trim_start_matches('L').parse().ok()).unwrap_or(1);
                let layer = v.layers.get(n.saturating_sub(1)).cloned().unwrap_or_default();
                out.layers.push(check_copper(v, name, &function, &g, &layer, bounds, px));
            }
            Some("Soldermask") => {
                let top = parts.get(1) == Some(&"Top");
                let mut r = Raster::new(bounds, px, 1.0);
                paint(&mut r, &g, |_| true);
                out.layers.push(check_mask(v, name, &function, &g, &r, top));
                masks.insert(top, r);
            }
            Some("Paste") => {
                let top = parts.get(1) == Some(&"Top");
                let mut r = Raster::new(bounds, px, 1.0);
                paint(&mut r, &g, |_| true);
                out.layers.push(check_paste(v, name, &function, &g, &r, top));
            }
            Some("Legend") => {
                let top = parts.get(1) == Some(&"Top");
                let mut r = Raster::new(bounds, px, 1.0);
                paint(&mut r, &g, |_| true);
                silks.push((top, name.clone(), r));
                out.layers.push(LayerCheck { file: name.clone(), function: function.clone(), objects: g.objects.len(), board_area: None, file_area: None, differ_area: None, pads: 0, pads_ok: 0, missing: vec![], extra: vec![], nets: vec![], shorts: vec![], problems: vec![], notes: vec![], ok: true });
            }
            Some("Profile") => out.outline = Some(check_outline(v, name, &g)),
            _ => out.unreadable.push(format!("{name}: no X2 file function, so its layer is unknown")),
        }
    }
    // Silkscreen over mask openings: a fab clips it.
    for (top, name, r) in &silks {
        if let Some(m) = masks.get(top) {
            let over = r.on.iter().zip(&m.on).filter(|(s, o)| **s != 0 && **o != 0).count() as f64 * px * px;
            if let Some(l) = out.layers.iter_mut().find(|l| &l.file == name) {
                if over > 0.01 {
                    l.notes.push(format!("{:.2} mm² of ink over exposed copper (the fab clips it from the pads)", over));
                }
            }
        }
    }
    let bad_layers = out.layers.iter().filter(|l| !l.ok).count();
    let bad_drills = out.drills.iter().filter(|d| !d.ok).count();
    let outline_ok = out.outline.as_ref().is_some_and(|o| o.ok);
    out.ok = bad_layers == 0 && bad_drills == 0 && outline_ok && out.unreadable.is_empty();
    let copper: Vec<&LayerCheck> = out.layers.iter().filter(|l| l.function.starts_with("Copper")).collect();
    let pads: usize = copper.iter().map(|l| l.pads).sum();
    let pads_ok: usize = copper.iter().map(|l| l.pads_ok).sum();
    let holes: usize = out.drills.iter().map(|d| d.file_holes).sum();
    let board_holes: usize = out.drills.iter().map(|d| d.board_holes).sum();
    out.summary = format!(
        "{} files read back: {} copper layers ({}/{} pads exposed, {} nets compared), {}/{} holes, outline {}{}",
        files.len() - out.unreadable.len(),
        copper.len(),
        pads_ok,
        pads,
        copper.iter().map(|l| l.nets.len()).max().unwrap_or(0),
        holes,
        board_holes,
        if outline_ok { "closed and matching" } else { "NOT matching" },
        if out.ok { "; everything matches the board" } else { "; differences found" }
    );
    out
}

fn check_copper(v: &BoardView, name: &str, function: &str, g: &GerberFile, layer: &str, bounds: [f64; 4], px: f64) -> LayerCheck {
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let mut files = Raster::new(bounds, px, 1.0);
    paint(&mut files, g, |_| true);
    let mut board = files.clone_empty();
    board.on = vec![0; board.w * board.h];
    let copper = board_copper(v, layer);
    for shapes in copper.values() {
        for s in shapes {
            board.fill(s, 1);
        }
    }
    let (ba, fa, xa) = (board.area(), files.area(), board.xor_area(&files));
    if ba > 0.0 && ((fa - ba).abs() / ba > 0.01 || xa / ba > 0.02) {
        problems.push(format!("the file exposes {fa:.2} mm² where the board has {ba:.2} mm²; {xa:.2} mm² differ"));
    }
    // Every pad lit where it is.
    let here: Vec<&crate::board::PadView> = v.pads.iter().filter(|p| p.layers.iter().any(|l| l == layer)).collect();
    let mut missing = Vec::new();
    for p in &here {
        let poly = p.polygon.clone().unwrap_or_else(|| prim_poly(&p.prim));
        if files.coverage(&poly) < 0.99 {
            missing.push(format!("{}.{}", p.reference, if p.number.is_empty() { "?" } else { &p.number }));
        }
    }
    if !missing.is_empty() {
        problems.push(format!("{} pads not exposed: {}", missing.len(), missing.iter().take(12).cloned().collect::<Vec<_>>().join(", ")));
    }
    // Pins by their X2 attribute.
    let file_pins: BTreeSet<String> = g.objects.iter().filter(|o| o.function.as_deref().is_some_and(|f| f.contains("Pad"))).filter_map(|o| o.pin_ref()).collect();
    let board_pins: BTreeSet<String> = here.iter().filter(|p| !p.number.is_empty()).map(|p| format!("{}.{}", p.reference, p.number)).collect();
    let extra: Vec<String> = file_pins.difference(&board_pins).cloned().collect();
    let unnamed: Vec<String> = board_pins.difference(&file_pins).cloned().collect();
    if !extra.is_empty() {
        problems.push(format!("pins in the file the board lacks here: {}", extra.join(", ")));
    }
    if !unnamed.is_empty() {
        notes.push(format!("{} pads carry no pin attribute in the file ({}…)", unnamed.len(), unnamed.iter().take(5).cloned().collect::<Vec<_>>().join(", ")));
    }
    // Copper per net: the file's objects of each net against the board's.
    let mut file_nets: BTreeMap<String, Vec<&Object>> = BTreeMap::new();
    for o in &g.objects {
        file_nets.entry(o.net.clone().unwrap_or_else(|| "N/C".into())).or_default().push(o);
    }
    let names: BTreeSet<String> = file_nets.keys().chain(copper.keys()).cloned().collect();
    let mut nets = Vec::new();
    let mut net_rasters: Vec<(String, Raster)> = Vec::new();
    for n in names {
        let objs = file_nets.get(&n).cloned().unwrap_or_default();
        let fpolys: Vec<(bool, bool, Vec<P>)> = objs.iter().flat_map(|o| g.outlines(o, TOL).into_iter().map(move |(e, p)| (o.dark, e, p))).collect();
        let bshapes = copper.get(&n).cloned().unwrap_or_default();
        let Some(b) = bbox_of(fpolys.iter().flat_map(|x| x.2.iter().copied()).chain(bshapes.iter().flatten().flatten().copied())) else { continue };
        let mut fr = Raster::new(b, px, 0.1);
        for (dark, exposed, p) in &fpolys {
            fr.fill(std::slice::from_ref(p), (*dark && *exposed) as u8);
        }
        let mut br = fr.clone_empty();
        br.on = vec![0; br.w * br.h];
        for s in &bshapes {
            br.fill(s, 1);
        }
        let (a, f) = (br.area(), fr.area());
        // Edge pixels make small nets differ by a few per cent; a lost 0402 land is 0.27 mm².
        let ok = (a - f).abs() <= (0.03 * a.max(f)).max(0.05);
        if !ok {
            problems.push(format!("net {n}: {f:.3} mm² of copper in the file, {a:.3} mm² on the board"));
        }
        nets.push(NetArea { net: n.clone(), board: r3(a), files: r3(f), ok });
        if n != "N/C" {
            net_rasters.push((n, fr));
        }
    }
    // Shorts: copper of two nets touching in the file.
    let mut shorts = Vec::new();
    for i in 0..net_rasters.len() {
        for j in i + 1..net_rasters.len() {
            let (a, b) = (&net_rasters[i].1, &net_rasters[j].1);
            let lo = [a.x0.max(b.x0), a.y0.max(b.y0)];
            let hi = [(a.x0 + a.w as f64 * px).min(b.x0 + b.w as f64 * px), (a.y0 + a.h as f64 * px).min(b.y0 + b.h as f64 * px)];
            if lo[0] >= hi[0] || lo[1] >= hi[1] {
                continue;
            }
            let mut touch = 0usize;
            let mut y = lo[1] + px / 2.0;
            while y < hi[1] {
                let mut x = lo[0] + px / 2.0;
                while x < hi[0] {
                    if a.at([x, y]) && b.at([x, y]) {
                        touch += 1;
                    }
                    x += px;
                }
                y += px;
            }
            if touch > 0 {
                shorts.push(format!("{} – {}", net_rasters[i].0, net_rasters[j].0));
            }
        }
    }
    if !shorts.is_empty() {
        problems.push(format!("copper of different nets touches: {}", shorts.join("; ")));
    }
    LayerCheck {
        file: name.into(),
        function: function.into(),
        objects: g.objects.len(),
        board_area: Some(r3(ba)),
        file_area: Some(r3(fa)),
        differ_area: Some(r3(xa)),
        pads: here.len(),
        pads_ok: here.len() - missing.len(),
        missing,
        extra,
        nets,
        shorts,
        ok: problems.is_empty(),
        problems,
        notes,
    }
}

fn side_layer(v: &BoardView, top: bool) -> String {
    if top { v.layers[0].clone() } else { v.layers[v.layers.len() - 1].clone() }
}

fn check_mask(v: &BoardView, name: &str, function: &str, g: &GerberFile, r: &Raster, top: bool) -> LayerCheck {
    let side = side_layer(v, top);
    let pads: Vec<&crate::board::PadView> = v.pads.iter().filter(|p| p.layers.contains(&side) || p.kind == PadKind::Npth).collect();
    let missing: Vec<String> = pads.iter().filter(|p| r.coverage(&p.polygon.clone().unwrap_or_else(|| prim_poly(&p.prim))) < 0.99).map(|p| format!("{}.{}", p.reference, p.number)).collect();
    let problems = if missing.is_empty() { vec![] } else { vec![format!("{} pads without a mask opening: {}", missing.len(), missing.iter().take(12).cloned().collect::<Vec<_>>().join(", "))] };
    LayerCheck { file: name.into(), function: function.into(), objects: g.objects.len(), board_area: None, file_area: Some(r3(r.area())), differ_area: None, pads: pads.len(), pads_ok: pads.len() - missing.len(), missing, extra: vec![], nets: vec![], shorts: vec![], ok: problems.is_empty(), problems, notes: vec![] }
}

fn check_paste(v: &BoardView, name: &str, function: &str, g: &GerberFile, r: &Raster, top: bool) -> LayerCheck {
    let side = side_layer(v, top);
    let smd: Vec<&crate::board::PadView> = v.pads.iter().filter(|p| p.kind == PadKind::Smd && p.layers.contains(&side)).collect();
    let mut missing = Vec::new();
    for p in &smd {
        // A windowpane exposes part of a pad: at least a quarter of it gets paste.
        if r.coverage(&p.polygon.clone().unwrap_or_else(|| prim_poly(&p.prim))) < 0.25 {
            missing.push(format!("{}.{}", p.reference, p.number));
        }
    }
    let holes: Vec<String> = v.holes.iter().filter(|h| r.at(h.at)).map(|h| h.owner.clone()).collect();
    let mut problems = Vec::new();
    if !missing.is_empty() {
        problems.push(format!("{} SMD pads without paste: {}", missing.len(), missing.iter().take(12).cloned().collect::<Vec<_>>().join(", ")));
    }
    if !holes.is_empty() {
        problems.push(format!("paste over holes: {}", holes.join(", ")));
    }
    LayerCheck { file: name.into(), function: function.into(), objects: g.objects.len(), board_area: None, file_area: Some(r3(r.area())), differ_area: None, pads: smd.len(), pads_ok: smd.len() - missing.len(), missing, extra: vec![], nets: vec![], shorts: vec![], ok: problems.is_empty(), problems, notes: vec![] }
}

fn check_drill(v: &BoardView, name: &str, d: &reader::DrillFile) -> DrillCheck {
    let plated = d.plated.unwrap_or(!name.to_ascii_uppercase().contains("NPTH"));
    let board: Vec<&crate::board::Hole> = v.holes.iter().filter(|h| h.plated == plated).collect();
    let mut problems = Vec::new();
    if !d.metric {
        problems.push("not metric".into());
    }
    // File holes as (centre, size, rotation-free long axis).
    let mut file: Vec<(P, [f64; 2])> = d.hits.iter().map(|(t, at)| (*at, [d.tools[t], d.tools[t]])).collect();
    for (t, a, b) in &d.slots {
        let w = d.tools[t];
        let l = geom::dist(*a, *b) + w;
        let c = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        file.push((c, [w, l]));
    }
    let mut used = vec![false; file.len()];
    let mut sizes: BTreeMap<i64, (usize, usize)> = BTreeMap::new();
    let key = |s: [f64; 2]| (s[0].min(s[1]) * 1000.0).round() as i64;
    for (_, s) in &file {
        sizes.entry(key(*s)).or_default().1 += 1;
    }
    let mut missing = Vec::new();
    for h in &board {
        sizes.entry(key(h.size)).or_default().0 += 1;
        let (w, l) = (h.size[0].min(h.size[1]), h.size[0].max(h.size[1]));
        let found = file.iter().enumerate().position(|(k, (c, s))| !used[k] && geom::dist(*c, h.at) < 0.005 && (s[0].min(s[1]) - w).abs() < 0.0015 && (s[0].max(s[1]) - l).abs() < 0.0015);
        match found {
            Some(k) => used[k] = true,
            None => missing.push(format!("{} ø{:.2}{} at ({:.2}, {:.2})", h.owner, w, if l > w + 1e-6 { format!(" × {l:.2} slot") } else { String::new() }, h.at[0], h.at[1])),
        }
    }
    let extra: Vec<String> = file.iter().enumerate().filter(|(k, _)| !used[*k]).map(|(_, (c, s))| format!("ø{:.2} at ({:.2}, {:.2})", s[0].min(s[1]), c[0], c[1])).collect();
    if !missing.is_empty() {
        problems.push(format!("{} holes missing or wrong: {}", missing.len(), missing.iter().take(8).cloned().collect::<Vec<_>>().join("; ")));
    }
    if !extra.is_empty() {
        problems.push(format!("{} holes the board does not have: {}", extra.len(), extra.iter().take(8).cloned().collect::<Vec<_>>().join("; ")));
    }
    DrillCheck { file: name.into(), plated, board_holes: board.len(), file_holes: file.len(), sizes: sizes.into_iter().map(|(k, (b, f))| (k as f64 / 1000.0, b, f)).collect(), ok: problems.is_empty(), problems }
}

/// Chains the profile's strokes into closed loops.
pub fn loops(g: &GerberFile) -> (Vec<Vec<P>>, usize) {
    let mut segs: Vec<(P, P)> = Vec::new();
    for o in &g.objects {
        if let Shape::Stroke { pts, .. } = &o.shape {
            for w in pts.windows(2) {
                if geom::dist(w[0], w[1]) > 1e-9 {
                    segs.push((w[0], w[1]));
                }
            }
        }
    }
    let close = |a: P, b: P| geom::dist(a, b) < 1e-4;
    let mut used = vec![false; segs.len()];
    let mut out = Vec::new();
    let mut open = 0;
    for s in 0..segs.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let mut path = vec![segs[s].0, segs[s].1];
        loop {
            let end = *path.last().unwrap();
            if path.len() > 2 && close(end, path[0]) {
                path.pop();
                out.push(path);
                break;
            }
            let next = (0..segs.len()).find(|k| !used[*k] && (close(segs[*k].0, end) || close(segs[*k].1, end)));
            match next {
                Some(k) => {
                    used[k] = true;
                    path.push(if close(segs[k].0, end) { segs[k].1 } else { segs[k].0 });
                }
                None => {
                    open += 1;
                    break;
                }
            }
        }
    }
    (out, open)
}

fn check_outline(v: &BoardView, name: &str, g: &GerberFile) -> OutlineCheck {
    let (ls, open) = loops(g);
    let mut problems = Vec::new();
    if open > 0 {
        problems.push(format!("{open} open chains in the profile"));
    }
    let outer = ls.iter().max_by(|a, b| geom::polygon_area(a).abs().total_cmp(&geom::polygon_area(b).abs()));
    let file_area = outer.map(|o| geom::polygon_area(o).abs()).unwrap_or(0.0) - ls.iter().filter(|l| Some(*l) != outer).map(|l| geom::polygon_area(l).abs()).sum::<f64>();
    let board_area = geom::polygon_area(&v.outline).abs() - v.cutouts.iter().map(|c| geom::polygon_area(c).abs()).sum::<f64>();
    let size = |pts: &[P]| bbox_of(pts.iter().copied()).map(|b| [r3(b[2] - b[0]), r3(b[3] - b[1])]).unwrap_or([0.0, 0.0]);
    let (bs, fs) = (size(&v.outline), outer.map(|o| size(o)).unwrap_or([0.0, 0.0]));
    if ls.len() != 1 + v.cutouts.len() {
        problems.push(format!("{} closed loops; the board has an outline and {} cutouts", ls.len(), v.cutouts.len()));
    }
    if (file_area - board_area).abs() > 0.001 * board_area.max(1.0) {
        problems.push(format!("the profile encloses {file_area:.2} mm², the board {board_area:.2} mm²"));
    }
    if (bs[0] - fs[0]).abs() > 0.01 || (bs[1] - fs[1]).abs() > 0.01 {
        problems.push(format!("the profile is {} × {} mm, the board {} × {} mm", fs[0], fs[1], bs[0], bs[1]));
    }
    OutlineCheck { file: name.into(), loops: ls.len(), board_area: r3(board_area), file_area: r3(file_area), board_size: bs, file_size: fs, ok: problems.is_empty(), problems }
}

// ---- Drawing what was read ---------------------------------------------------------

/// One file drawn from what the reader understood: SVG elements in board coordinates
/// (y up; the caller flips), dark objects in `currentColor`, clear ones in `var(--pcb-clear)`.
pub fn svg(g: &GerberFile) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let n = |v: f64| {
        let t = format!("{v:.4}");
        let t = t.trim_end_matches('0').trim_end_matches('.').to_string();
        if t == "-0" { "0".to_string() } else { t }
    };
    for o in &g.objects {
        let fill = if o.dark { "currentColor" } else { "var(--pcb-clear, #000)" };
        match &o.shape {
            Shape::Stroke { d, pts } if matches!(g.apertures.get(d), Some(reader::Aperture::Circle(_))) => {
                let Some(reader::Aperture::Circle(w)) = g.apertures.get(d) else { continue };
                let p: Vec<String> = pts.iter().map(|q| format!("{},{}", n(q[0]), n(q[1]))).collect();
                let _ = write!(s, r#"<polyline points="{}" fill="none" stroke="{fill}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#, p.join(" "), n(*w));
            }
            _ => {
                let mut d = String::new();
                for (exposed, poly) in g.outlines(o, 0.01) {
                    if !exposed || poly.len() < 3 {
                        continue;
                    }
                    let _ = write!(d, "M{}", poly.iter().map(|q| format!("{} {}", n(q[0]), n(q[1]))).collect::<Vec<_>>().join("L"));
                    d.push('Z');
                }
                if !d.is_empty() {
                    let _ = write!(s, r#"<path d="{d}" fill="{fill}"/>"#);
                }
            }
        }
    }
    s
}

/// Drill hits and slots drawn as circles and stadiums (board coordinates).
pub fn drill_svg(d: &reader::DrillFile) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for (t, at) in &d.hits {
        let _ = write!(s, r#"<circle cx="{:.4}" cy="{:.4}" r="{:.4}" fill="currentColor"/>"#, at[0], at[1], d.tools[t] / 2.0);
    }
    for (t, a, b) in &d.slots {
        let _ = write!(s, r#"<line x1="{:.4}" y1="{:.4}" x2="{:.4}" y2="{:.4}" stroke="currentColor" stroke-width="{:.4}" stroke-linecap="round"/>"#, a[0], a[1], b[0], b[1], d.tools[t]);
    }
    s
}
