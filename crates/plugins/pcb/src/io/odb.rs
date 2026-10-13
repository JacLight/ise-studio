//! ODB++ (the published ODB++ Design format, v8), as a zip of its directory
//! tree, as KiCad writes it: the matrix of layers, job info, one step with
//! its profile (outline and cutouts), a features file per layer (pads
//! flashed with standard symbols, tracks, pours as surfaces with holes, mask
//! and paste openings, silkscreen, drill hits), the component layers (each
//! part with its pins' nets), and the EDA data tying copper features to
//! nets and packages. Metric: coordinates in mm, symbol sizes in microns,
//! rotations clockwise (ODB++'s sense).

use crate::board::{BoardView, ComponentView, PadView};
use crate::footprints::{PadKind, PadShape};
use crate::geom::P;
use crate::{Board, Side};
use std::collections::BTreeMap;
use std::fmt::Write;

fn f(v: f64) -> String {
    super::num(v, 6)
}

fn um(v: f64) -> String {
    super::num(v * 1000.0, 3)
}

/// Clockwise degrees from counter-clockwise.
fn cw(r: f64) -> String {
    super::num((360.0 - r.rem_euclid(360.0)).rem_euclid(360.0), 4)
}

#[derive(Default)]
struct Features {
    symbols: Vec<String>,
    body: String,
    count: usize,
}

impl Features {
    fn sym(&mut self, s: String) -> usize {
        if let Some(i) = self.symbols.iter().position(|x| *x == s) {
            return i;
        }
        self.symbols.push(s);
        self.symbols.len() - 1
    }
    fn pad(&mut self, at: P, sym: String, rot: f64, attrs: &str) -> usize {
        let i = self.sym(sym);
        let _ = writeln!(self.body, "P {} {} {i} P 0 8 {}{attrs}", f(at[0]), f(at[1]), cw(rot));
        self.count += 1;
        self.count - 1
    }
    fn line(&mut self, a: P, b: P, w: f64) -> usize {
        let i = self.sym(format!("r{}", um(w)));
        let _ = writeln!(self.body, "L {} {} {} {} {i} P 0", f(a[0]), f(a[1]), f(b[0]), f(b[1]));
        self.count += 1;
        self.count - 1
    }
    fn surface(&mut self, contours: &[Vec<P>]) -> usize {
        let _ = writeln!(self.body, "S P 0");
        for (k, c) in contours.iter().enumerate() {
            if c.len() < 3 {
                continue;
            }
            // Islands clockwise, holes counter-clockwise (ODB++'s rule).
            let mut c = c.clone();
            let ccw = crate::geom::polygon_area(&c) > 0.0;
            let island = k == 0;
            if island == ccw {
                c.reverse();
            }
            let _ = writeln!(self.body, "OB {} {} {}", f(c[0][0]), f(c[0][1]), if island { "I" } else { "H" });
            for q in c.iter().skip(1).chain(std::iter::once(&c[0])) {
                let _ = writeln!(self.body, "OS {} {}", f(q[0]), f(q[1]));
            }
            let _ = writeln!(self.body, "OE");
        }
        let _ = writeln!(self.body, "SE");
        self.count += 1;
        self.count - 1
    }
    fn text(&self) -> String {
        let mut s = String::from("UNITS=MM\n#\n#Feature symbol names\n#\n");
        for (i, x) in self.symbols.iter().enumerate() {
            let _ = writeln!(s, "${i} {x}");
        }
        s.push_str("#\n#Layer features\n#\n");
        s.push_str(&self.body);
        s
    }
}

/// A pad's standard symbol (microns) and rotation.
fn pad_symbol(p: &PadView, grow: f64) -> String {
    let (w, h) = (p.size[0] + 2.0 * grow, p.size[1] + 2.0 * grow);
    match p.shape {
        PadShape::Circle => format!("r{}", um(w)),
        PadShape::Oval => format!("oval{}x{}", um(w), um(h)),
        PadShape::RoundRect if p.corner > 0.0 => format!("rect{}x{}xr{}", um(w), um(h), um(p.corner + grow)),
        _ => format!("rect{}x{}", um(w), um(h)),
    }
}

struct Layer {
    name: &'static str,
    kind: &'static str,
    feats: Features,
}

fn layer_dir(l: &str) -> &'static str {
    crate::COPPER_NAMES.iter().find(|x| **x == l).copied().unwrap_or("bottom")
}

/// The ODB++ tree: (path inside the zip, bytes).
pub fn files(v: &BoardView, board: &Board) -> Vec<(String, Vec<u8>)> {
    let job = crate::gerber::file_stem(&v.name).to_ascii_lowercase();
    let root = format!("{job}/");
    let n = v.layers.len();
    let mut lay: Vec<Layer> = vec![
        Layer { name: "comp_+_top", kind: "COMPONENT", feats: Features::default() },
        Layer { name: "silk_top", kind: "SILK_SCREEN", feats: Features::default() },
        Layer { name: "paste_top", kind: "SOLDER_PASTE", feats: Features::default() },
        Layer { name: "mask_top", kind: "SOLDER_MASK", feats: Features::default() },
    ];
    for l in &v.layers {
        lay.push(Layer { name: layer_dir(l), kind: if l == "top" || l == "bottom" { "SIGNAL" } else { "POWER_GROUND" }, feats: Features::default() });
    }
    lay.push(Layer { name: "mask_bot", kind: "SOLDER_MASK", feats: Features::default() });
    lay.push(Layer { name: "paste_bot", kind: "SOLDER_PASTE", feats: Features::default() });
    lay.push(Layer { name: "silk_bot", kind: "SILK_SCREEN", feats: Features::default() });
    lay.push(Layer { name: "comp_+_bot", kind: "COMPONENT", feats: Features::default() });
    lay.push(Layer { name: "drill", kind: "DRILL", feats: Features::default() });
    let idx = |name: &str, lay: &Vec<Layer>| lay.iter().position(|l| l.name == name).unwrap();
    // Copper layers in EDA order (LYR record) and their feature ids per net.
    let cu_names: Vec<&'static str> = v.layers.iter().map(|l| layer_dir(l)).collect();
    let mut net_fids: BTreeMap<String, Vec<(usize, usize, &'static str)>> = BTreeMap::new();
    let mut pad_fids: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    for (pi, p) in v.pads.iter().enumerate() {
        for l in &p.layers {
            let li = idx(layer_dir(l), &lay);
            let fid = match &p.polygon {
                Some(pg) => lay[li].feats.surface(std::slice::from_ref(pg)),
                None => lay[li].feats.pad(p.at, pad_symbol(p, 0.0), p.rotation, ";.pad_usage=toeprint"),
            };
            let ci = cu_names.iter().position(|x| *x == layer_dir(l)).unwrap();
            pad_fids.entry(pi).or_default().push((ci, fid));
        }
    }
    for t in &v.tracks {
        let li = idx(layer_dir(&t.layer), &lay);
        let ci = cu_names.iter().position(|x| *x == layer_dir(&t.layer)).unwrap();
        for s in t.points.windows(2) {
            let fid = lay[li].feats.line(s[0], s[1], t.width);
            if let Some(nn) = &t.net {
                net_fids.entry(nn.clone()).or_default().push((ci, fid, "TRC"));
            }
        }
    }
    for x in &v.vias {
        for (ci, l) in cu_names.iter().enumerate() {
            let Some(dia) = crate::gerber::via_pad(v, x, &v.layers[ci]) else { continue };
            let li = idx(l, &lay);
            let fid = lay[li].feats.pad(x.at, format!("r{}", um(dia)), 0.0, ";.pad_usage=via");
            if let Some(nn) = &x.net {
                net_fids.entry(nn.clone()).or_default().push((ci, fid, "VIA"));
            }
        }
    }
    for z in &v.zones {
        let li = idx(layer_dir(&z.layer), &lay);
        let ci = cu_names.iter().position(|x| *x == layer_dir(&z.layer)).unwrap();
        for island in &z.fill {
            if island.is_empty() {
                continue;
            }
            let fid = lay[li].feats.surface(island);
            net_fids.entry(z.net.clone()).or_default().push((ci, fid, "PLN"));
        }
    }
    for t in &v.teardrops {
        let li = idx(layer_dir(&t.layer), &lay);
        let ci = cu_names.iter().position(|x| *x == layer_dir(&t.layer)).unwrap();
        let fid = lay[li].feats.surface(std::slice::from_ref(&t.polygon));
        if let Some(nn) = &t.net {
            net_fids.entry(nn.clone()).or_default().push((ci, fid, "PLN"));
        }
    }
    // Board graphics: copper (with their net), silkscreen.
    for g in &v.graphics {
        let target = match g.layer.as_str() {
            "silk_top" => "silk_top",
            "silk_bottom" => "silk_bot",
            "mask_top" => "mask_top",
            "mask_bottom" => "mask_bot",
            "paste_top" => "paste_top",
            "paste_bottom" => "paste_bot",
            l if v.layers.iter().any(|x| x == l) => layer_dir(l),
            _ => continue,
        };
        let li = idx(target, &lay);
        let ci = cu_names.iter().position(|x| *x == target);
        let mut fids = Vec::new();
        for island in g.fills.iter().filter(|i| !i.is_empty()) {
            fids.push((lay[li].feats.surface(island), "PLN"));
        }
        for st in &g.strokes {
            for seg in st.windows(2) {
                fids.push((lay[li].feats.line(seg[0], seg[1], g.width), "TRC"));
            }
        }
        if let (Some(ci), Some(nn)) = (ci, &g.net) {
            for (fid, k) in fids {
                net_fids.entry(nn.clone()).or_default().push((ci, fid, k));
            }
        }
    }
    let e = v.rules.mask_expansion;
    for side in [Side::Top, Side::Bottom] {
        let l = &v.layers[if side == Side::Top { 0 } else { n - 1 }];
        let (mask, paste, silk) = if side == Side::Top { ("mask_top", "paste_top", "silk_top") } else { ("mask_bot", "paste_bot", "silk_bot") };
        let (mi, pi, si) = (idx(mask, &lay), idx(paste, &lay), idx(silk, &lay));
        for p in v.pads.iter().filter(|p| p.layers.contains(l) || p.kind == PadKind::Npth) {
            match &p.polygon {
                Some(pg) => {
                    lay[mi].feats.surface(std::slice::from_ref(pg));
                }
                None => {
                    lay[mi].feats.pad(p.at, pad_symbol(p, if p.kind == PadKind::Npth { 0.0 } else { e }), p.rotation, "");
                }
            }
        }
        for p in v.pads.iter().filter(|p| p.kind == PadKind::Smd && p.side == side) {
            for q in &p.paste {
                lay[pi].feats.surface(&[q.outline(0.01)]);
            }
        }
        for c in v.components.iter().filter(|c| c.side == side) {
            for line in &c.silk {
                for s in line.windows(2) {
                    lay[si].feats.line(s[0], s[1], v.rules.silk_width);
                }
            }
        }
    }
    let di = idx("drill", &lay);
    for h in &v.holes {
        let d = h.size[0].min(h.size[1]);
        let usage = if h.owner == "via" { "via" } else if h.plated { "plated" } else { "non_plated" };
        if (h.size[0] - h.size[1]).abs() > 1e-9 {
            let half = (h.size[0].max(h.size[1]) - d) / 2.0;
            let dir = if h.size[0] > h.size[1] { [half, 0.0] } else { [0.0, half] };
            let q = crate::geom::rotate(dir, h.rotation);
            let i = lay[di].feats.sym(format!("r{}", um(d)));
            let _ = writeln!(lay[di].feats.body, "L {} {} {} {} {i} P 0;.drill={usage}", f(h.at[0] - q[0]), f(h.at[1] - q[1]), f(h.at[0] + q[0]), f(h.at[1] + q[1]));
            lay[di].feats.count += 1;
        } else {
            lay[di].feats.pad(h.at, format!("r{}", um(d)), 0.0, &format!(";.drill={usage}"));
        }
    }

    // EDA data: layers, nets with their pins and copper, packages.
    let mut pkgs: Vec<String> = Vec::new();
    let mut pkg_of: BTreeMap<String, usize> = BTreeMap::new();
    for c in &v.components {
        if !pkg_of.contains_key(&c.footprint) {
            pkg_of.insert(c.footprint.clone(), pkgs.len());
            pkgs.push(c.footprint.clone());
        }
    }
    let comp_index: BTreeMap<String, (Side, usize)> = {
        let mut m = BTreeMap::new();
        let (mut t, mut b) = (0, 0);
        for c in &v.components {
            let k = if c.side == Side::Top { &mut t } else { &mut b };
            m.insert(c.reference.clone(), (c.side, *k));
            *k += 1;
        }
        m
    };
    let mut nets: Vec<String> = v.nets.iter().map(|x| x.name.clone()).collect();
    for nn in net_fids.keys() {
        if !nets.contains(nn) {
            nets.push(nn.clone());
        }
    }
    nets.push("$NONE$".into());
    let mut eda = String::new();
    let _ = writeln!(eda, "HDR Forma\nUNITS=MM\nLYR {}\n#", cu_names.join(" "));
    // Pin numbers within each component, and the (net, subnet) each pin is on.
    let mut pin_subnet: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    for (ni, name) in nets.iter().enumerate() {
        let _ = writeln!(eda, "NET {name}");
        let mut sub = 0usize;
        for (pi, p) in v.pads.iter().enumerate().filter(|(_, p)| p.net.as_deref() == Some(name.as_str()) || (name == "$NONE$" && p.net.is_none())) {
            let Some((side, cn)) = comp_index.get(&p.reference) else { continue };
            let pin_no = v.pads.iter().filter(|q| q.component == p.component).position(|q| std::ptr::eq(q, p)).unwrap_or(0);
            let _ = writeln!(eda, "SNT TOP {} {cn} {pin_no}", if *side == Side::Top { "T" } else { "B" });
            for (ci, fid) in pad_fids.get(&pi).into_iter().flatten() {
                let _ = writeln!(eda, "FID C {ci} {fid}");
            }
            pin_subnet.insert((p.reference.clone(), format!("{pi}")), (ni, sub));
            sub += 1;
        }
        let mut kinds: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
        for (ci, fid, k) in net_fids.get(name).into_iter().flatten() {
            kinds.entry(k).or_default().push((*ci, *fid));
        }
        for (k, fids) in kinds {
            let _ = writeln!(eda, "SNT {k}{}", if k == "PLN" { " S" } else { "" });
            for (ci, fid) in fids {
                let _ = writeln!(eda, "FID C {ci} {fid}");
            }
        }
        eda.push_str("#\n");
    }
    for name in &pkgs {
        let Some(c) = v.components.iter().find(|c| c.footprint == *name) else { continue };
        let pads: Vec<&PadView> = v.pads.iter().filter(|p| p.component == c.id).collect();
        let local = |q: P| to_local(c, q);
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in &pads {
            let bb = p.prim.bbox();
            for q in [[bb[0], bb[1]], [bb[2], bb[3]], [bb[0], bb[3]], [bb[2], bb[1]]] {
                let l = local(q);
                b = [b[0].min(l[0]), b[1].min(l[1]), b[2].max(l[0]), b[3].max(l[1])];
            }
        }
        if pads.is_empty() {
            b = [-0.5, -0.5, 0.5, 0.5];
        }
        let pitch = pads.windows(2).map(|w| crate::geom::dist(w[0].at, w[1].at)).fold(f64::INFINITY, f64::min);
        let _ = writeln!(eda, "PKG {} {} {} {} {} {}", name.replace(' ', "_"), f(if pitch.is_finite() { pitch } else { 0.0 }), f(b[0]), f(b[1]), f(b[2]), f(b[3]));
        let _ = writeln!(eda, "RC {} {} {} {}", f(b[0]), f(b[1]), f(b[2] - b[0]), f(b[3] - b[1]));
        for p in &pads {
            let l = local(p.at);
            let t = match p.kind {
                PadKind::Smd => "S",
                _ => "T",
            };
            let _ = writeln!(eda, "PIN {} {t} {} {} 0 {} {}", if p.number.is_empty() { "NP".to_string() } else { p.number.replace(' ', "_") }, f(l[0]), f(l[1]), if p.kind == PadKind::Npth { "M" } else { "E" }, if p.kind == PadKind::Smd { "S" } else { "T" });
            let bb = p.prim.bbox();
            let (w, h) = (bb[2] - bb[0], bb[3] - bb[1]);
            let _ = writeln!(eda, "RC {} {} {} {}", f(l[0] - w / 2.0), f(l[1] - h / 2.0), f(w), f(h));
        }
        eda.push_str("#\n");
    }

    // Components per side.
    let comps = |side: Side| -> String {
        let mut s = String::from("UNITS=MM\n#\n");
        for c in v.components.iter().filter(|c| c.side == side) {
            let _ = writeln!(s, "CMP {} {} {} {} {} {} {}", pkg_of[&c.footprint], f(c.at[0]), f(c.at[1]), cw(c.rotation), if side == Side::Bottom { "M" } else { "N" }, c.reference, if c.value.is_empty() { c.footprint.replace(' ', "_") } else { c.value.replace(' ', "_") });
            let _ = writeln!(s, "PRP VALUE '{}'", c.value.replace('\'', ""));
            if let Some(m) = &c.mpn {
                let _ = writeln!(s, "PRP MPN '{}'", m.replace('\'', ""));
            }
            for (k, (pi, p)) in v.pads.iter().enumerate().filter(|(_, p)| p.component == c.id).enumerate() {
                let (ni, sub) = pin_subnet.get(&(c.reference.clone(), format!("{pi}"))).copied().unwrap_or((nets.len() - 1, 0));
                let _ = writeln!(s, "TOP {k} {} {} {} {} {ni} {sub} {}", f(p.at[0]), f(p.at[1]), cw(p.rotation), if side == Side::Bottom { "M" } else { "N" }, if p.number.is_empty() { "NP".to_string() } else { p.number.replace(' ', "_") });
            }
            s.push_str("#\n");
        }
        s
    };

    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    // Matrix.
    let mut m = String::from("STEP {\n   COL=1\n   NAME=PCB\n}\n\n");
    for (row, l) in lay.iter().enumerate() {
        let (start, end) = if l.kind == "DRILL" { (cu_names[0].to_ascii_uppercase(), cu_names[n - 1].to_ascii_uppercase()) } else { (String::new(), String::new()) };
        let _ = writeln!(m, "LAYER {{\n   ROW={}\n   CONTEXT=BOARD\n   TYPE={}\n   NAME={}\n   OLD_NAME=\n   POLARITY=POSITIVE\n   START_NAME={start}\n   END_NAME={end}\n   ADD_TYPE=\n   COLOR=0\n}}\n", row + 1, l.kind, l.name.to_ascii_uppercase());
    }
    out.push((format!("{root}matrix/matrix"), m.into_bytes()));
    let (y, mo, d, h, mi, s) = super::now();
    let stamp = format!("{y:04}{mo:02}{d:02}.{h:02}{mi:02}{s:02}");
    out.push((format!("{root}misc/info"), format!("UNITS=MM\nJOB_NAME={job}\nODB_VERSION_MAJOR=8\nODB_VERSION_MINOR=1\nODB_SOURCE=Forma\nCREATION_DATE={stamp}\nSAVE_DATE={stamp}\nSAVE_APP=Forma {}\nSAVE_USER=forma\n", env!("CARGO_PKG_VERSION")).into_bytes()));
    out.push((format!("{root}steps/pcb/stephdr"), b"UNITS=MM\nX_DATUM=0\nY_DATUM=0\nX_ORIGIN=0\nY_ORIGIN=0\nTOP_ACTIVE=0\nBOTTOM_ACTIVE=0\nRIGHT_ACTIVE=0\nLEFT_ACTIVE=0\nAFFECTING_BOM=\nAFFECTING_BOM_CHANGED=0\n".to_vec()));
    let mut prof = Features::default();
    let mut contours = vec![v.outline.clone()];
    contours.extend(v.cutouts.iter().cloned());
    prof.surface(&contours);
    out.push((format!("{root}steps/pcb/profile"), prof.text().into_bytes()));
    out.push((format!("{root}steps/pcb/eda/data"), eda.into_bytes()));
    for l in &lay {
        let dir = format!("{root}steps/pcb/layers/{}/", l.name);
        if l.kind == "COMPONENT" {
            out.push((format!("{dir}components"), comps(if l.name.ends_with("top") { Side::Top } else { Side::Bottom }).into_bytes()));
        } else {
            out.push((format!("{dir}features"), l.feats.text().into_bytes()));
        }
        out.push((format!("{dir}attrlist"), b"".to_vec()));
    }
    let _ = board;
    out
}

fn to_local(c: &ComponentView, p: P) -> P {
    let q = crate::geom::rotate(crate::geom::sub(p, c.at), -c.rotation);
    if c.side == Side::Bottom { [-q[0], q[1]] } else { q }
}
