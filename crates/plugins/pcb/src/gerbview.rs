//! The Gerber viewer: any set of Gerber and Excellon files (a fab zip, loose
//! files, a job file) read by the independent reader ([`crate::reader`]) and
//! laid out for looking at: each layer's function (from its X2 attributes,
//! or guessed from the file name the way fabs do), its apertures with how
//! often each is used, its file attributes, the nets, parts and pins its
//! objects carry, and an SVG of it in which every object is tagged with its
//! net, part and D-code so the viewer can highlight any of them. Two
//! revisions can be compared layer by layer (what was added, what removed),
//! and the visible layers exported as a vector PDF.

use crate::gerber_check::{Raster, paint};
use crate::reader::{self, Aperture, GerberFile, Shape};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write;

type P = [f64; 2];

// ---- Files in -----------------------------------------------------------------------

fn u16le(b: &[u8], i: usize) -> usize {
    b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]) as usize).unwrap_or(0)
}
fn u32le(b: &[u8], i: usize) -> usize {
    b.get(i..i + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize).unwrap_or(0)
}

/// The files in a zip archive (stored or deflated), folders and macOS metadata left out.
pub fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let n = bytes.len();
    if n < 22 {
        return Err("not a zip archive".into());
    }
    // End of central directory, searched back from the end (it may carry a comment).
    let eocd = (0..=n - 22).rev().take(65_557).find(|&i| bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or("not a zip archive (no directory)")?;
    let count = u16le(bytes, eocd + 10);
    let mut at = u32le(bytes, eocd + 16);
    let mut out = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        if bytes.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err("the zip directory is damaged".into());
        }
        let method = u16le(bytes, at + 10);
        let csize = u32le(bytes, at + 20);
        let usize_ = u32le(bytes, at + 24);
        let (nlen, xlen, clen) = (u16le(bytes, at + 28), u16le(bytes, at + 30), u16le(bytes, at + 32));
        let local = u32le(bytes, at + 42);
        let name = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + nlen).ok_or("the zip directory is damaged")?).to_string();
        at += 46 + nlen + xlen + clen;
        if name.ends_with('/') || name.starts_with("__MACOSX") || name.rsplit('/').next().is_some_and(|b| b.starts_with("._")) {
            continue;
        }
        if bytes.get(local..local + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
            return Err(format!("{name}: damaged entry"));
        }
        let start = local + 30 + u16le(bytes, local + 26) + u16le(bytes, local + 28);
        let data = bytes.get(start..start + csize).ok_or_else(|| format!("{name}: cut short"))?;
        total += usize_;
        if total > 256 << 20 {
            return Err("the archive unpacks to more than 256 MB".into());
        }
        let content = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, usize_.max(1) + 16).map_err(|e| format!("{name}: {e:?}"))?,
            m => return Err(format!("{name}: compression method {m} is not read (stored and deflate are)")),
        };
        let base = name.rsplit('/').next().unwrap_or(&name).to_string();
        out.push((base, content));
    }
    Ok(out)
}

/// A file as given: a zip is opened, anything else is one file.
pub fn unpack(name: &str, bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    if bytes.starts_with(&[0x50, 0x4b, 0x03, 0x04]) || name.to_ascii_lowercase().ends_with(".zip") {
        unzip(bytes)
    } else {
        Ok(vec![(name.to_string(), bytes.to_vec())])
    }
}

// ---- Layers -------------------------------------------------------------------------

/// The layer a file is, from its name when it carries no X2 file function
/// (Protel extensions, KiCad and Altium names).
pub fn guess_function(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    let has = |s: &str| lower.contains(s);
    let f = match ext {
        "gtl" | "cmp" | "top" => "Copper,L1,Top",
        "gbl" | "sol" | "bot" => "Copper,L2,Bot",
        "gts" | "stc" => "Soldermask,Top",
        "gbs" | "sts" => "Soldermask,Bot",
        "gtp" | "crc" => "Paste,Top",
        "gbp" | "crs" => "Paste,Bot",
        "gto" | "plc" => "Legend,Top",
        "gbo" | "pls" => "Legend,Bot",
        "gko" | "gm1" | "gml" | "gm" | "out" | "oln" => "Profile,NP",
        "drl" | "xln" | "drd" | "exc" => return Some(if has("npth") || has("non") { "NonPlated,NPTH".into() } else { "Plated,PTH".into() }),
        _ => {
            if let Some(n) = ext.strip_prefix('g').and_then(|r| r.trim_end_matches('l').parse::<u32>().ok()) {
                return Some(format!("Copper,L{n},Inr"));
            }
            let key = lower.replace(['-', '.'], "_");
            return [
                ("f_cu", "Copper,L1,Top"),
                ("b_cu", "Copper,L2,Bot"),
                ("f_mask", "Soldermask,Top"),
                ("b_mask", "Soldermask,Bot"),
                ("f_paste", "Paste,Top"),
                ("b_paste", "Paste,Bot"),
                ("f_silk", "Legend,Top"),
                ("b_silk", "Legend,Bot"),
                ("edge_cuts", "Profile,NP"),
                ("v_score", "Vcut"),
                ("vcut", "Vcut"),
            ]
            .iter()
            .find(|(k, _)| key.contains(k))
            .map(|(_, f)| f.to_string())
            .or_else(|| {
                let i = key.find("_cu")?;
                let n: u32 = key[..i].rsplit("in").next()?.parse().ok()?;
                Some(format!("Copper,L{},Inr", n + 1))
            });
        }
    };
    Some(f.into())
}

/// Stack order for a layer function, top of the board first.
fn rank(function: &str) -> i32 {
    let p: Vec<&str> = function.split(',').collect();
    let top = p.iter().any(|x| *x == "Top");
    let bot = p.iter().any(|x| *x == "Bot");
    match p.first().copied().unwrap_or("") {
        "Profile" | "Vcut" => 0,
        "Plated" | "NonPlated" => 1,
        "Legend" if top => 10,
        "Paste" if top => 11,
        "Soldermask" if top => 12,
        "Copper" => 20 + p.get(1).and_then(|l| l.trim_start_matches('L').parse::<i32>().ok()).unwrap_or(50) + if bot { 100 } else { 0 },
        "Soldermask" if bot => 300,
        "Paste" if bot => 301,
        "Legend" if bot => 302,
        _ => 400,
    }
}

/// A colour for a layer function, the way viewers usually show them.
pub fn colour(function: &str) -> &'static str {
    let p: Vec<&str> = function.split(',').collect();
    let bot = p.iter().any(|x| *x == "Bot");
    match p.first().copied().unwrap_or("") {
        "Copper" if bot => "#4d7fc4",
        "Copper" if p.iter().any(|x| *x == "Inr") => "#c2c200",
        "Copper" => "#c83434",
        "Soldermask" if bot => "#02ffee",
        "Soldermask" => "#d864ff",
        "Paste" if bot => "#00c2c2",
        "Paste" => "#b4b4b4",
        "Legend" if bot => "#e8b2a7",
        "Legend" => "#f2eda1",
        "Profile" => "#d0d2cd",
        "Vcut" => "#ff8c00",
        "Plated" => "#e0e0e0",
        "NonPlated" => "#a0a0a0",
        _ => "#9ad",
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ApertureInfo {
    pub d: u32,
    pub shape: String,
    /// Size text: "ø0.6", "1.2 × 0.8", the macro's parameters.
    pub size: String,
    pub function: Option<String>,
    pub uses: usize,
}

/// One object as the viewer lists it: indices into the response's net/part tables.
#[derive(Clone, Debug, Serialize)]
pub struct ObjectInfo {
    /// "flash", "draw", "arc" or "region".
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub clear: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// Where it is: a flash's centre, a draw's first point, a region's first vertex.
    pub at: P,
    /// Length of a draw (mm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Layer {
    pub file: String,
    /// "gerber" or "drill".
    pub kind: &'static str,
    pub function: String,
    /// The function came from the file name, not its X2 attributes.
    pub guessed: bool,
    pub colour: &'static str,
    pub negative: bool,
    pub units: String,
    pub format: Option<(u32, u32)>,
    pub bounds: Option<[f64; 4]>,
    pub attributes: BTreeMap<String, String>,
    pub apertures: Vec<ApertureInfo>,
    pub objects: Vec<ObjectInfo>,
    /// Drill tools: diameter, holes, slots.
    pub tools: Vec<(u32, f64, usize, usize)>,
    /// SVG elements in file coordinates (y up), every object tagged `class="o<i> n<net> c<part> d<code>"`.
    pub svg: String,
    pub source_lines: usize,
}

/// The whole set: layers in stack order, the shared net and part tables, a job file if any.
#[derive(Clone, Debug, Serialize)]
pub struct Set {
    pub layers: Vec<Layer>,
    pub nets: Vec<String>,
    pub parts: Vec<String>,
    pub bounds: [f64; 4],
    pub job: Option<Value>,
    /// Files that could not be read, with why.
    pub unreadable: Vec<(String, String)>,
    /// Files that are not Gerber or drill files (readme, BOM…), by name.
    pub other: Vec<String>,
}

fn num(v: f64) -> String {
    let t = format!("{v:.4}");
    let t = t.trim_end_matches('0').trim_end_matches('.').to_string();
    if t == "-0" { "0".into() } else { t }
}

fn aperture_text(a: &Aperture) -> (String, String) {
    match a {
        Aperture::Circle(d) => ("circle".into(), format!("ø{}", num(*d))),
        Aperture::Rect(w, h) => ("rectangle".into(), format!("{} × {}", num(*w), num(*h))),
        Aperture::Obround(w, h) => ("obround".into(), format!("{} × {}", num(*w), num(*h))),
        Aperture::Polygon(d, n, r) => ("polygon".into(), format!("ø{} {n} sides{}", num(*d), if *r != 0.0 { format!(" turned {}°", num(*r)) } else { String::new() })),
        Aperture::Macro(name, p) => (format!("macro {name}"), p.iter().map(|x| num(*x)).collect::<Vec<_>>().join(", ")),
    }
}

fn is_drill(name: &str, text: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    text.trim_start().starts_with("M48") || ((lower.ends_with(".drl") || lower.ends_with(".xln")) && !text.trim_start().starts_with('%'))
}

fn is_gerber(text: &str) -> bool {
    let t = text.trim_start();
    (t.starts_with('%') || t.starts_with("G04") || t.starts_with("G75") || t.starts_with("G01")) && (t.contains("%FS") || t.contains("%MO"))
}

struct Tables {
    nets: Vec<String>,
    parts: Vec<String>,
}

impl Tables {
    fn index(list: &mut Vec<String>, s: &str) -> usize {
        match list.iter().position(|x| x == s) {
            Some(i) => i,
            None => {
                list.push(s.to_string());
                list.len() - 1
            }
        }
    }
}

/// Draws one Gerber file: dark objects in `currentColor`; each run of clear objects cuts
/// everything drawn before it (an SVG mask), so the layers under it still show through.
fn layer_svg(g: &GerberFile, infos: &[ObjectInfo], id: &str, b: [f64; 4]) -> String {
    let mut s = String::new();
    let mut mask_n = 0;
    let (mx, my, mw, mh) = (b[0] - 1.0, b[1] - 1.0, b[2] - b[0] + 2.0, b[3] - b[1] + 2.0);
    let mut i = 0;
    while i < g.objects.len() {
        let dark = g.objects[i].dark;
        let mut j = i;
        let mut run = String::new();
        while j < g.objects.len() && g.objects[j].dark == dark {
            object_svg(g, j, &infos[j], &mut run);
            j += 1;
        }
        if dark {
            s.push_str(&run);
        } else {
            // Shapes inside a mask paint black to cut.
            let cut = run.replace("currentColor", "#000");
            let m = format!("{id}-m{mask_n}");
            mask_n += 1;
            s = format!(r##"<mask id="{m}" maskUnits="userSpaceOnUse" x="{}" y="{}" width="{}" height="{}"><rect x="{}" y="{}" width="{}" height="{}" fill="#fff"/>{cut}</mask><g mask="url(#{m})">{s}</g>"##, num(mx), num(my), num(mw), num(mh), num(mx), num(my), num(mw), num(mh));
        }
        i = j;
    }
    s
}

fn object_svg(g: &GerberFile, i: usize, info: &ObjectInfo, s: &mut String) {
    let o = &g.objects[i];
    let mut class = format!("o{i}");
    if let Some(n) = info.net {
        let _ = write!(class, " n{n}");
    }
    if let Some(c) = info.part {
        let _ = write!(class, " c{c}");
    }
    if let Some(d) = info.d {
        let _ = write!(class, " d{d}");
    }
    match &o.shape {
        Shape::Stroke { d, pts } if matches!(g.apertures.get(d), Some(Aperture::Circle(_))) => {
            let Some(Aperture::Circle(w)) = g.apertures.get(d) else { return };
            let p: Vec<String> = pts.iter().map(|q| format!("{},{}", num(q[0]), num(q[1]))).collect();
            // A zero-width draw still shows (a hairline).
            let _ = write!(s, r#"<polyline class="{class}" points="{}" fill="none" stroke="currentColor" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#, p.join(" "), num(w.max(0.01)));
        }
        Shape::Flash { d, at } if matches!(g.apertures.get(d), Some(Aperture::Circle(_))) => {
            let Some(Aperture::Circle(w)) = g.apertures.get(d) else { return };
            let _ = write!(s, r#"<circle class="{class}" cx="{}" cy="{}" r="{}" fill="currentColor"/>"#, num(at[0]), num(at[1]), num(w / 2.0));
        }
        _ => {
            let mut d = String::new();
            for (exposed, poly) in g.outlines(o, 0.01) {
                if poly.len() < 3 {
                    continue;
                }
                // Holes in an aperture (exposure off) draw as the opposite winding with even-odd fill.
                let _ = exposed;
                let _ = write!(d, "M{}Z", poly.iter().map(|q| format!("{} {}", num(q[0]), num(q[1]))).collect::<Vec<_>>().join("L"));
            }
            if !d.is_empty() {
                let _ = write!(s, r#"<path class="{class}" d="{d}" fill="currentColor" fill-rule="evenodd"/>"#);
            }
        }
    }
}

fn read_gerber(name: &str, text: &str, t: &mut Tables, index: usize) -> Result<Layer, String> {
    let g = reader::gerber(text)?;
    let (function, guessed) = match g.attributes.get("FileFunction") {
        Some(f) => (f.clone(), false),
        None => (guess_function(name).unwrap_or_else(|| "Other".into()), true),
    };
    let mut uses: BTreeMap<u32, usize> = BTreeMap::new();
    let mut infos = Vec::with_capacity(g.objects.len());
    for o in &g.objects {
        let (kind, d, at, length) = match &o.shape {
            Shape::Flash { d, at } => ("flash", Some(*d), *at, None),
            Shape::Stroke { d, pts } => (if pts.len() > 2 { "arc" } else { "draw" }, Some(*d), pts[0], Some(pts.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum::<f64>())),
            Shape::Region(c) => ("region", None, c.first().copied().unwrap_or([0.0, 0.0]), None),
        };
        if let Some(d) = d {
            *uses.entry(d).or_default() += 1;
        }
        let function = o.function.clone().or_else(|| d.and_then(|d| g.aperture_functions.get(&d).cloned()));
        infos.push(ObjectInfo {
            kind,
            d,
            clear: !o.dark,
            net: o.net.as_deref().filter(|n| !n.is_empty()).map(|n| Tables::index(&mut t.nets, n)),
            part: o.component.as_deref().or_else(|| o.pin.as_deref().and_then(|p| p.split(',').next())).filter(|c| !c.is_empty()).map(|c| Tables::index(&mut t.parts, c)),
            pin: o.pin.clone(),
            function,
            at: [(at[0] * 1e4).round() / 1e4, (at[1] * 1e4).round() / 1e4],
            length: length.map(|l| (l * 1e4).round() / 1e4),
        });
    }
    let apertures = g
        .apertures
        .iter()
        .map(|(d, a)| {
            let (shape, size) = aperture_text(a);
            ApertureInfo { d: *d, shape, size, function: g.aperture_functions.get(d).cloned(), uses: uses.get(d).copied().unwrap_or(0) }
        })
        .collect();
    let b = g.bounds();
    let bounds = if b[0].is_finite() { Some(b) } else { None };
    let svg = layer_svg(&g, &infos, &format!("gv{index}"), bounds.unwrap_or([0.0, 0.0, 1.0, 1.0]));
    Ok(Layer {
        file: name.into(),
        kind: "gerber",
        colour: colour(&function),
        negative: g.attributes.get("FilePolarity").is_some_and(|p| p == "Negative"),
        function,
        guessed,
        units: if g.units_mm { "mm".into() } else { "inch".into() },
        format: Some(g.format),
        bounds,
        attributes: g.attributes.clone(),
        apertures,
        objects: infos,
        tools: vec![],
        svg,
        source_lines: text.lines().count(),
    })
}

fn read_drill(name: &str, text: &str) -> Result<Layer, String> {
    let d = reader::excellon(text)?;
    let function = match d.plated {
        Some(true) => "Plated,PTH".to_string(),
        Some(false) => "NonPlated,NPTH".to_string(),
        None => guess_function(name).unwrap_or_else(|| "Plated,PTH".into()),
    };
    let mut svg = String::new();
    let mut objects = Vec::new();
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut grow = |p: P, r: f64| b = [b[0].min(p[0] - r), b[1].min(p[1] - r), b[2].max(p[0] + r), b[3].max(p[1] + r)];
    for (t, at) in &d.hits {
        let r = d.tools[t] / 2.0;
        let i = objects.len();
        let _ = write!(svg, r#"<circle class="o{i} d{t}" cx="{}" cy="{}" r="{}" fill="currentColor"/>"#, num(at[0]), num(at[1]), num(r));
        grow(*at, r);
        objects.push(ObjectInfo { kind: "flash", d: Some(*t), clear: false, net: None, part: None, pin: None, function: Some(format!("ø{} hole", num(d.tools[t]))), at: *at, length: None });
    }
    for (t, a, c) in &d.slots {
        let w = d.tools[t];
        let i = objects.len();
        let _ = write!(svg, r#"<line class="o{i} d{t}" x1="{}" y1="{}" x2="{}" y2="{}" stroke="currentColor" stroke-width="{}" stroke-linecap="round"/>"#, num(a[0]), num(a[1]), num(c[0]), num(c[1]), num(w));
        grow(*a, w / 2.0);
        grow(*c, w / 2.0);
        objects.push(ObjectInfo { kind: "draw", d: Some(*t), clear: false, net: None, part: None, pin: None, function: Some(format!("ø{} slot", num(w))), at: *a, length: Some(((c[0] - a[0]).powi(2) + (c[1] - a[1]).powi(2)).sqrt()) });
    }
    let tools = d.tools.iter().map(|(t, dia)| (*t, *dia, d.hits.iter().filter(|h| h.0 == *t).count(), d.slots.iter().filter(|s| s.0 == *t).count())).collect();
    Ok(Layer {
        file: name.into(),
        kind: "drill",
        colour: colour(&function),
        guessed: d.plated.is_none(),
        function,
        negative: false,
        units: if d.metric { "mm".into() } else { "inch".into() },
        format: None,
        bounds: if b[0].is_finite() { Some(b) } else { None },
        attributes: BTreeMap::new(),
        apertures: vec![],
        objects,
        tools,
        svg,
        source_lines: text.lines().count(),
    })
}

/// Reads a set of files into layers, in stack order.
pub fn read(files: &[(String, Vec<u8>)]) -> Set {
    let mut t = Tables { nets: vec![], parts: vec![] };
    let mut layers = Vec::new();
    let mut unreadable = Vec::new();
    let mut other = Vec::new();
    let mut job = None;
    for (i, (name, bytes)) in files.iter().enumerate() {
        let text = String::from_utf8_lossy(bytes);
        if name.to_ascii_lowercase().ends_with(".gbrjob") {
            match serde_json::from_str::<Value>(&text) {
                Ok(v) => job = Some(v),
                Err(e) => unreadable.push((name.clone(), format!("job file: {e}"))),
            }
            continue;
        }
        let r = if is_drill(name, &text) {
            read_drill(name, &text)
        } else if is_gerber(&text) {
            read_gerber(name, &text, &mut t, i)
        } else {
            other.push(name.clone());
            continue;
        };
        match r {
            Ok(l) => layers.push(l),
            Err(e) => unreadable.push((name.clone(), e)),
        }
    }
    layers.sort_by_key(|l| (rank(&l.function), l.file.clone()));
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for l in &layers {
        if let Some(lb) = l.bounds {
            b = [b[0].min(lb[0]), b[1].min(lb[1]), b[2].max(lb[2]), b[3].max(lb[3])];
        }
    }
    if !b[0].is_finite() {
        b = [0.0, 0.0, 10.0, 10.0];
    }
    Set { layers, nets: t.nets, parts: t.parts, bounds: b, job, unreadable, other }
}

// ---- Comparing two revisions --------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct LayerDiff {
    pub function: String,
    pub before: Option<String>,
    pub after: Option<String>,
    /// mm² present after but not before, and before but not after.
    pub added: f64,
    pub removed: f64,
    /// Holes (drill layers): added and removed, as "ø0.3 at (x, y)".
    pub holes_added: Vec<String>,
    pub holes_removed: Vec<String>,
    /// SVG rectangles (file coordinates) of what was added and removed.
    pub svg_added: String,
    pub svg_removed: String,
    pub same: bool,
}

fn raster_of(text: &str, b: [f64; 4], px: f64) -> Option<Raster> {
    let g = reader::gerber(text).ok()?;
    let mut r = Raster::new(b, px, 0.5);
    paint(&mut r, &g, |_| true);
    Some(r)
}

/// Pixels on in `a` and off in `b`, merged into row runs and drawn as rectangles.
fn runs_svg(a: &Raster, b: &Raster, limit: usize) -> (f64, String) {
    let mut s = String::new();
    let mut count = 0usize;
    let mut on = 0usize;
    for y in 0..a.h {
        let mut x = 0;
        while x < a.w {
            let k = y * a.w + x;
            if a.on[k] != 0 && b.on[k] == 0 {
                let x1 = x;
                while x < a.w && a.on[y * a.w + x] != 0 && b.on[y * a.w + x] == 0 {
                    x += 1;
                }
                on += x - x1;
                if count < limit {
                    let _ = write!(s, r#"<rect x="{}" y="{}" width="{}" height="{}"/>"#, num(a.x0 + x1 as f64 * a.px), num(a.y0 + y as f64 * a.px), num((x - x1) as f64 * a.px), num(a.px));
                    count += 1;
                }
            } else {
                x += 1;
            }
        }
    }
    (on as f64 * a.px * a.px, s)
}

/// Compares two sets layer by layer (matched by function), at `px` mm per pixel.
pub fn compare(before: &[(String, Vec<u8>)], after: &[(String, Vec<u8>)], px: f64) -> Vec<LayerDiff> {
    let sa = read(before);
    let sb = read(after);
    let b = [sa.bounds[0].min(sb.bounds[0]), sa.bounds[1].min(sb.bounds[1]), sa.bounds[2].max(sb.bounds[2]), sa.bounds[3].max(sb.bounds[3])];
    // Keep the raster under ~16 M pixels.
    let px = px.max(((b[2] - b[0] + 1.0) * (b[3] - b[1] + 1.0) / 16e6).sqrt());
    let text_of = |set: &[(String, Vec<u8>)], file: &str| set.iter().find(|f| f.0 == file).map(|f| String::from_utf8_lossy(&f.1).to_string());
    let mut functions: Vec<String> = sa.layers.iter().chain(&sb.layers).map(|l| l.function.clone()).collect();
    functions.sort_by_key(|f| (rank(f), f.clone()));
    functions.dedup();
    let mut out = Vec::new();
    for f in functions {
        let la = sa.layers.iter().find(|l| l.function == f);
        let lb = sb.layers.iter().find(|l| l.function == f);
        let mut d = LayerDiff { function: f.clone(), before: la.map(|l| l.file.clone()), after: lb.map(|l| l.file.clone()), added: 0.0, removed: 0.0, holes_added: vec![], holes_removed: vec![], svg_added: String::new(), svg_removed: String::new(), same: false };
        let kind = la.or(lb).map(|l| l.kind).unwrap_or("gerber");
        if kind == "drill" {
            let holes = |l: Option<&Layer>| -> Vec<String> { l.map(|l| l.objects.iter().map(|o| format!("{} at ({}, {})", o.function.clone().unwrap_or_default(), num(o.at[0]), num(o.at[1]))).collect()).unwrap_or_default() };
            let (ha, hb) = (holes(la), holes(lb));
            d.holes_added = hb.iter().filter(|h| !ha.contains(h)).cloned().collect();
            d.holes_removed = ha.iter().filter(|h| !hb.contains(h)).cloned().collect();
            for o in lb.map(|l| l.objects.as_slice()).unwrap_or(&[]) {
                let key = format!("{} at ({}, {})", o.function.clone().unwrap_or_default(), num(o.at[0]), num(o.at[1]));
                if d.holes_added.contains(&key) {
                    let _ = write!(d.svg_added, r#"<circle cx="{}" cy="{}" r="0.6"/>"#, num(o.at[0]), num(o.at[1]));
                }
            }
            for o in la.map(|l| l.objects.as_slice()).unwrap_or(&[]) {
                let key = format!("{} at ({}, {})", o.function.clone().unwrap_or_default(), num(o.at[0]), num(o.at[1]));
                if d.holes_removed.contains(&key) {
                    let _ = write!(d.svg_removed, r#"<circle cx="{}" cy="{}" r="0.6"/>"#, num(o.at[0]), num(o.at[1]));
                }
            }
            d.same = d.holes_added.is_empty() && d.holes_removed.is_empty() && la.is_some() && lb.is_some();
        } else {
            let empty = Raster::new(b, px, 0.5);
            let ra = la.and_then(|l| text_of(before, &l.file)).and_then(|t| raster_of(&t, b, px)).unwrap_or_else(|| Raster::new(b, px, 0.5));
            let rb = lb.and_then(|l| text_of(after, &l.file)).and_then(|t| raster_of(&t, b, px)).unwrap_or_else(|| Raster::new(b, px, 0.5));
            let _ = empty;
            let (added, svg_added) = runs_svg(&rb, &ra, 40_000);
            let (removed, svg_removed) = runs_svg(&ra, &rb, 40_000);
            d.added = (added * 1e4).round() / 1e4;
            d.removed = (removed * 1e4).round() / 1e4;
            d.svg_added = svg_added;
            d.svg_removed = svg_removed;
            // A pixel or two of difference at edges is rounding.
            d.same = la.is_some() && lb.is_some() && added + removed <= 4.0 * px * px;
        }
        out.push(d);
    }
    out
}

// ---- PDF ----------------------------------------------------------------------------

/// How one layer is drawn in an export.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Look {
    pub file: String,
    #[serde(default)]
    pub colour: Option<String>,
}

fn hex(c: &str) -> [f64; 3] {
    let h = c.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0x888888);
    [((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0]
}

/// The given layers (bottom of the list drawn first) as a one-page vector PDF, scaled to fit A4
/// landscape (or 1:1 when the board fits), on `background`; `mirror` looks from below.
pub fn pdf(files: &[(String, Vec<u8>)], looks: &[Look], background: &str, mirror: bool) -> Result<Vec<u8>, String> {
    let set = read(files);
    let b = set.bounds;
    let (bw, bh) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
    let pt = 72.0 / 25.4;
    let (pw, ph) = (297.0 * pt, 210.0 * pt);
    let margin = 10.0 * pt;
    let scale = ((pw - 2.0 * margin) / (bw * pt)).min((ph - 2.0 * margin) / (bh * pt)).min(1.0) * pt;
    let ox = (pw - bw * scale) / 2.0;
    let oy = (ph - bh * scale) / 2.0;
    let tx = |p: P| -> (f64, f64) {
        let x = if mirror { b[2] - p[0] } else { p[0] - b[0] };
        (ox + x * scale, oy + (p[1] - b[1]) * scale)
    };
    let mut c = String::new();
    let bg = hex(background);
    let _ = writeln!(c, "{:.3} {:.3} {:.3} rg 0 0 {:.2} {:.2} re f", bg[0], bg[1], bg[2], pw, ph);
    for look in looks.iter().rev() {
        let Some(layer) = set.layers.iter().find(|l| l.file == look.file) else { continue };
        let col = hex(look.colour.as_deref().unwrap_or(layer.colour));
        let text = files.iter().find(|f| f.0 == look.file).map(|f| String::from_utf8_lossy(&f.1).to_string()).unwrap_or_default();
        if layer.kind == "drill" {
            let _ = writeln!(c, "{:.3} {:.3} {:.3} rg", col[0], col[1], col[2]);
            if let Ok(d) = reader::excellon(&text) {
                for (t, at) in &d.hits {
                    circle(&mut c, tx(*at), d.tools[t] / 2.0 * scale);
                }
                for (t, a, e) in &d.slots {
                    let poly = capsule(*a, *e, d.tools[t] / 2.0);
                    path(&mut c, &poly, &tx);
                    c.push_str("f\n");
                }
            }
            continue;
        }
        let Ok(g) = reader::gerber(&text) else { continue };
        for o in &g.objects {
            let fill = if o.dark { col } else { bg };
            let _ = writeln!(c, "{:.3} {:.3} {:.3} rg", fill[0], fill[1], fill[2]);
            let mut any = false;
            for (_, poly) in g.outlines(o, 0.01) {
                if poly.len() >= 3 {
                    path(&mut c, &poly, &tx);
                    any = true;
                }
            }
            if any {
                c.push_str("f*\n");
            }
        }
    }
    // A scale note.
    let _ = writeln!(c, "0.4 0.4 0.4 rg BT /F1 8 Tf {:.2} {:.2} Td ({}) Tj ET", margin, margin / 2.0, pdf_text(&format!("{} layers, {:.2} x {:.2} mm, {}{}", looks.len(), bw, bh, if scale >= pt - 1e-9 { "1:1".to_string() } else { format!("scale 1:{:.2}", pt / scale) }, if mirror { ", seen from below" } else { "" })));
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"),
        format!("<< /Length {} >>\nstream\n{c}endstream", c.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        let _ = writeln!(out, "{} 0 obj\n{o}\nendobj", i + 1);
    }
    let xref = out.len();
    let _ = writeln!(out, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(out, "{o:010} 00000 n ");
    }
    let _ = write!(out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
    Ok(out.into_bytes())
}

fn pdf_text(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii()).flat_map(|c| if matches!(c, '(' | ')' | '\\') { vec!['\\', c] } else { vec![c] }).collect()
}

fn path(c: &mut String, poly: &[P], tx: &dyn Fn(P) -> (f64, f64)) {
    for (i, p) in poly.iter().enumerate() {
        let (x, y) = tx(*p);
        let _ = writeln!(c, "{x:.3} {y:.3} {}", if i == 0 { "m" } else { "l" });
    }
    c.push_str("h\n");
}

fn circle(c: &mut String, (x, y): (f64, f64), r: f64) {
    // Four Bézier quarters.
    let k = 0.552_284_75 * r;
    let _ = writeln!(c, "{:.3} {:.3} m {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c f", x + r, y, x + r, y + k, x + k, y + r, x, y + r, x - k, y + r, x - r, y + k, x - r, y, x - r, y - k, x - k, y - r, x, y - r, x + k, y - r, x + r, y - k, x + r, y);
}

fn capsule(a: P, b: P, r: f64) -> Vec<P> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let ang = dy.atan2(dx);
    let mut out = Vec::new();
    for k in 0..=16 {
        let t = ang - std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / 16.0;
        out.push([b[0] + r * t.cos(), b[1] + r * t.sin()]);
    }
    for k in 0..=16 {
        let t = ang + std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / 16.0;
        out.push([a[0] + r * t.cos(), a[1] + r * t.sin()]);
    }
    out
}

// ---- Gerbers to a board -------------------------------------------------------------

/// Commands that rebuild a board from fab files (what GerbView's "export to PCB" does): the
/// outline and cutouts from the profile, every round-aperture draw on a copper layer that is
/// not a pad as a track (with its X2 net), every via pad with its drill as a via. Pads, pours
/// and silkscreen are not turned into footprints or zones (files do not say which part a land
/// belongs to well enough); they are counted in `notes`. Steps use `$0.id` for the board.
pub fn board_steps(files: &[(String, Vec<u8>)], name: &str) -> Result<(Vec<Value>, Vec<String>), String> {
    let text = |f: &str| files.iter().find(|x| x.0 == f).map(|x| String::from_utf8_lossy(&x.1).to_string()).unwrap_or_default();
    let set = read(files);
    let profile = set.layers.iter().find(|l| l.function.starts_with("Profile")).ok_or("no board profile (Edge_Cuts) among the files")?;
    let g = reader::gerber(&text(&profile.file))?;
    let (mut loops, open) = crate::gerber_check::loops(&g);
    if loops.is_empty() {
        return Err("the profile has no closed outline".into());
    }
    let area = |p: &Vec<P>| crate::geom::polygon_area(p).abs();
    loops.sort_by(|a, b| area(b).total_cmp(&area(a)));
    let mut outline = loops.remove(0);
    if crate::geom::polygon_area(&outline) < 0.0 {
        outline.reverse();
    }
    let r4 = |v: f64| (v * 1e4).round() / 1e4;
    let pts = |p: &[P]| Value::Array(p.iter().map(|q| json!([r4(q[0]), r4(q[1])])).collect());
    let copper: Vec<&Layer> = set.layers.iter().filter(|l| l.function.starts_with("Copper")).collect();
    let n_layers = if copper.len() > 2 { 4 } else { 2 };
    let mut notes = Vec::new();
    if open > 0 {
        notes.push(format!("{open} open chains in the profile were left out"));
    }
    let mut steps = vec![json!({ "operation": "pcb.board.create", "parameters": { "name": name, "outline": pts(&outline), "cutouts": loops.iter().map(|c| pts(c)).collect::<Vec<_>>(), "layers": n_layers } })];
    let drills: Vec<(P, f64)> = set.layers.iter().filter(|l| l.function.starts_with("Plated")).flat_map(|l| {
        let d = reader::excellon(&text(&l.file)).ok();
        d.map(|d| d.hits.iter().map(|(t, at)| (*at, d.tools[t])).collect::<Vec<_>>()).unwrap_or_default()
    }).collect();
    let (mut tracks, mut vias, mut pads, mut regions) = (0, 0, 0, 0);
    for l in &copper {
        let p: Vec<&str> = l.function.split(',').collect();
        let n: usize = p.get(1).and_then(|x| x.trim_start_matches('L').parse().ok()).unwrap_or(1);
        let layer = if p.contains(&"Top") { "top".to_string() } else if p.contains(&"Bot") { "bottom".to_string() } else { format!("in{}", n - 1) };
        let g = reader::gerber(&text(&l.file))?;
        for o in &g.objects {
            let function = o.function.clone().unwrap_or_default();
            match &o.shape {
                Shape::Stroke { d, pts: line } if !function.contains("Pad") => {
                    let Some(Aperture::Circle(w)) = g.apertures.get(d) else { continue };
                    let mut s = json!({ "board": "$0.id", "layer": layer, "points": pts(line), "width": r4(*w) });
                    if let Some(net) = o.net.as_deref().filter(|n| !n.is_empty() && *n != "N/C") {
                        s["net"] = json!(net);
                    }
                    steps.push(json!({ "operation": "pcb.track.create", "parameters": s }));
                    tracks += 1;
                }
                Shape::Flash { d, at } if function.starts_with("ViaPad") => {
                    // Each via once: from the top layer.
                    if layer != "top" {
                        continue;
                    }
                    let dia = match g.apertures.get(d) {
                        Some(Aperture::Circle(w)) => *w,
                        _ => continue,
                    };
                    let drill = drills.iter().filter(|(q, _)| (q[0] - at[0]).hypot(q[1] - at[1]) < 0.01).map(|x| x.1).next().unwrap_or(dia / 2.0);
                    let mut s = json!({ "board": "$0.id", "at": [r4(at[0]), r4(at[1])], "diameter": r4(dia), "drill": r4(drill) });
                    if let Some(net) = o.net.as_deref().filter(|n| !n.is_empty()) {
                        s["net"] = json!(net);
                    }
                    steps.push(json!({ "operation": "pcb.via.create", "parameters": s }));
                    vias += 1;
                }
                Shape::Region(_) => regions += 1,
                _ => pads += 1,
            }
        }
    }
    notes.push(format!("{tracks} tracks and {vias} vias made; {pads} pads and {regions} copper regions (pours, custom pads) are not turned into footprints or zones: place the parts and pour again"));
    Ok((steps, notes))
}

/// A short summary of a set (for the CLI and the AI).
pub fn summary(set: &Set) -> Value {
    json!({
        "bounds": set.bounds,
        "size": [((set.bounds[2] - set.bounds[0]) * 1e3).round() / 1e3, ((set.bounds[3] - set.bounds[1]) * 1e3).round() / 1e3],
        "layers": set.layers.iter().map(|l| json!({
            "file": l.file, "kind": l.kind, "function": l.function, "guessed": l.guessed, "units": l.units,
            "objects": l.objects.len(), "apertures": l.apertures.len(), "tools": l.tools,
            "nets": l.objects.iter().filter_map(|o| o.net).collect::<std::collections::BTreeSet<usize>>().len(),
        })).collect::<Vec<_>>(),
        "nets": set.nets.len(),
        "parts": set.parts.len(),
        "unreadable": set.unreadable,
        "other": set.other,
        "job": set.job.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_guess_layers() {
        assert_eq!(guess_function("board.GTL").unwrap(), "Copper,L1,Top");
        assert_eq!(guess_function("board-B_Mask.gbr").unwrap(), "Soldermask,Bot");
        assert_eq!(guess_function("board-In1_Cu.gbr").unwrap(), "Copper,L2,Inr");
        assert_eq!(guess_function("board.G2L").unwrap(), "Copper,L2,Inr");
        assert_eq!(guess_function("board-Edge_Cuts.gm1").unwrap(), "Profile,NP");
        assert_eq!(guess_function("board-NPTH.drl").unwrap(), "NonPlated,NPTH");
    }

    #[test]
    fn zips_open_stored_and_deflated() {
        let files = vec![("a.gtl".to_string(), b"hello".to_vec()), ("b.drl".to_string(), vec![b'x'; 5000])];
        let z = de_shape::fabricate::zip(&files);
        let back = unzip(&z).unwrap();
        assert_eq!(back, files);
        // A deflated entry, made by hand: "abcabcabc" deflated with fixed codes.
        let raw = miniz_oxide::deflate::compress_to_vec(b"abcabcabcabc", 6);
        let mut z = Vec::new();
        let name = b"x.gbr";
        let crc = 0u32;
        z.extend([0x50, 0x4b, 3, 4, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
        z.extend(crc.to_le_bytes());
        z.extend((raw.len() as u32).to_le_bytes());
        z.extend(12u32.to_le_bytes());
        z.extend((name.len() as u16).to_le_bytes());
        z.extend(0u16.to_le_bytes());
        z.extend(name);
        z.extend(&raw);
        let cd = z.len();
        z.extend([0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
        z.extend(crc.to_le_bytes());
        z.extend((raw.len() as u32).to_le_bytes());
        z.extend(12u32.to_le_bytes());
        z.extend((name.len() as u16).to_le_bytes());
        z.extend([0u8; 12]);
        z.extend(0u32.to_le_bytes());
        z.extend(name);
        let cd_len = z.len() - cd;
        z.extend([0x50, 0x4b, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend((cd_len as u32).to_le_bytes());
        z.extend((cd as u32).to_le_bytes());
        z.extend([0, 0]);
        assert_eq!(unzip(&z).unwrap(), vec![("x.gbr".to_string(), b"abcabcabcabc".to_vec())]);
    }

    const ONE: &str = "%TF.FileFunction,Copper,L1,Top*%\n%FSLAX46Y46*%\n%MOMM*%\n%TA.AperFunction,SMDPad,CuDef*%\n%ADD10R,1X2*%\n%TD*%\n%ADD11C,0.25*%\n%TO.N,GND*%\n%TO.C,R1*%\nD10*\nX0Y0D03*\n%TD*%\n%TO.N,VCC*%\nD11*\nX0Y0D02*\nX5000000Y0D01*\n%TD*%\n%LPC*%\nD11*\nX2000000Y0D03*\nM02*\n";

    #[test]
    fn layers_read_with_attributes() {
        let set = read(&[("x.gtl".into(), ONE.as_bytes().to_vec())]);
        assert!(set.unreadable.is_empty(), "{:?}", set.unreadable);
        let l = &set.layers[0];
        assert_eq!(l.function, "Copper,L1,Top");
        assert!(!l.guessed);
        assert_eq!(l.objects.len(), 3);
        assert_eq!(set.nets, vec!["GND", "VCC"]);
        assert_eq!(set.parts, vec!["R1"]);
        assert_eq!(l.apertures.iter().find(|a| a.d == 10).unwrap().uses, 1);
        assert_eq!(l.apertures.iter().find(|a| a.d == 11).unwrap().uses, 2);
        assert_eq!(l.objects[1].length, Some(5.0));
        // The clear flash cuts what came before it.
        assert!(l.svg.contains("<mask") && l.svg.contains("class=\"o0 n0 c0 d10\""), "{}", l.svg);
    }

    #[test]
    fn revisions_compare() {
        let after = ONE.replace("X5000000Y0D01*", "X6000000Y0D01*");
        let d = compare(&[("x.gtl".into(), ONE.as_bytes().to_vec())], &[("x.gtl".into(), after.into_bytes())], 0.01);
        assert_eq!(d.len(), 1);
        // 1 mm more of a 0.25 mm track: 0.25 mm² added (a little more for the round end moving).
        assert!((d[0].added - 0.25).abs() < 0.03, "{:?}", d[0].added);
        assert!(d[0].removed < 0.03);
        assert!(!d[0].same);
        let same = compare(&[("x.gtl".into(), ONE.as_bytes().to_vec())], &[("y.gtl".into(), ONE.as_bytes().to_vec())], 0.01);
        assert!(same[0].same);
    }

    #[test]
    fn pdf_is_made() {
        let files = vec![("x.gtl".to_string(), ONE.as_bytes().to_vec())];
        let p = pdf(&files, &[Look { file: "x.gtl".into(), colour: None }], "#000000", false).unwrap();
        let s = String::from_utf8_lossy(&p);
        assert!(s.starts_with("%PDF-1.4") && s.trim_end().ends_with("%%EOF"));
        assert!(s.contains(" re f") && s.contains("f*"));
    }
}
