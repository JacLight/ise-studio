//! An independent reader for Gerber RS-274X (with X2 attributes) and
//! Excellon: a separate code path from the writers in [`crate::gerber`], so
//! what they write can be read back the way a fab's CAM would and compared
//! with the board ([`crate::gerber_check`]). It reads what this crate writes
//! and the common subset other tools write (KiCad, Altium, EasyEDA):
//! standard and macro apertures (circle, vector and centre lines, outlines,
//! regular polygons, with `$n` arithmetic), linear and circular
//! interpolation, regions, dark and clear polarity, and the object
//! attributes that carry net, component and pin.
//!
//! It is strict: an undefined aperture, a missing format or unit, an
//! unterminated region, a missing end-of-file marker, or anything that would
//! change the image and is not understood (step and repeat, aperture
//! mirroring or scaling, aperture holes) is an error rather than a guess.

use std::collections::BTreeMap;

type P = [f64; 2];

#[derive(Clone, Debug, PartialEq)]
pub enum Aperture {
    Circle(f64),
    Rect(f64, f64),
    Obround(f64, f64),
    /// Regular polygon: outer diameter, vertices, rotation (degrees).
    Polygon(f64, u32, f64),
    /// A macro by name with its parameters.
    Macro(String, Vec<f64>),
}

impl Aperture {
    /// Half extents [x, y] of what a flash covers (macros: the RoundRect this crate writes; others 0).
    pub fn half(&self) -> [f64; 2] {
        match self {
            Aperture::Circle(d) => [d / 2.0, d / 2.0],
            Aperture::Rect(w, h) | Aperture::Obround(w, h) => [w / 2.0, h / 2.0],
            Aperture::Polygon(d, _, _) => [d / 2.0, d / 2.0],
            Aperture::Macro(name, p) if name == "RoundRect" && p.len() >= 9 => {
                let r = p[0];
                let xs = [p[1], p[3], p[5], p[7]];
                let ys = [p[2], p[4], p[6], p[8]];
                [xs.iter().fold(0.0f64, |m, v| m.max(v.abs())) + r, ys.iter().fold(0.0f64, |m, v| m.max(v.abs())) + r]
            }
            Aperture::Macro(..) => [0.0, 0.0],
        }
    }
}

/// What one object draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// The aperture stamped at a point.
    Flash { d: u32, at: P },
    /// The aperture swept along a polyline (a straight draw has two points, an arc many).
    Stroke { d: u32, pts: Vec<P> },
    /// A filled contour.
    Region(Vec<P>),
}

/// One graphical object with the attributes in force when it was made.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub shape: Shape,
    /// Dark (adds) or clear (removes).
    pub dark: bool,
    /// `.AperFunction` of its aperture, or of the region.
    pub function: Option<String>,
    /// `.N`: net name.
    pub net: Option<String>,
    /// `.C`: component reference.
    pub component: Option<String>,
    /// `.P`: "reference,pin[,function]".
    pub pin: Option<String>,
}

impl Object {
    /// "R1.2" from the pin attribute.
    pub fn pin_ref(&self) -> Option<String> {
        let p = self.pin.as_ref()?;
        let mut it = p.split(',');
        Some(format!("{}.{}", it.next()?, it.next()?))
    }
}

#[derive(Clone, Debug, Default)]
pub struct GerberFile {
    pub units_mm: bool,
    /// Integer and decimal digits.
    pub format: (u32, u32),
    pub attributes: BTreeMap<String, String>,
    pub apertures: BTreeMap<u32, Aperture>,
    pub aperture_functions: BTreeMap<u32, String>,
    pub macros: Vec<String>,
    /// Macro bodies by name: their primitive and variable statements.
    pub macro_defs: BTreeMap<String, Vec<String>>,
    pub flashes: Vec<(u32, [f64; 2])>,
    pub draws: Vec<(u32, [f64; 2], [f64; 2])>,
    pub regions: Vec<Vec<[f64; 2]>>,
    /// Net names seen in object attributes.
    pub nets: Vec<String>,
    /// Every object in file order, with polarity and attributes.
    pub objects: Vec<Object>,
}

impl GerberFile {
    /// Bounds of everything drawn: [min x, min y, max x, max y].
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for o in &self.objects {
            for poly in self.outlines(o, 0.01) {
                for p in poly.1 {
                    b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
                }
            }
        }
        b
    }

    /// An object as filled polygons, each with its exposure (true adds): flashes and strokes
    /// expanded from their aperture, arcs and round ends to within `tol` mm.
    pub fn outlines(&self, o: &Object, tol: f64) -> Vec<(bool, Vec<P>)> {
        match &o.shape {
            Shape::Region(c) => vec![(true, c.clone())],
            Shape::Flash { d, at } => self.aperture_polys(*d, tol).into_iter().map(|(on, poly)| (on, poly.into_iter().map(|q| [q[0] + at[0], q[1] + at[1]]).collect())).collect(),
            Shape::Stroke { d, pts } => {
                let mut out = Vec::new();
                match self.apertures.get(d) {
                    Some(Aperture::Circle(w)) => {
                        for s in pts.windows(2) {
                            out.push((true, capsule(s[0], s[1], w / 2.0, tol)));
                        }
                        if pts.len() == 1 {
                            out.push((true, circle([0.0, 0.0], w / 2.0, tol).into_iter().map(|q| [q[0] + pts[0][0], q[1] + pts[0][1]]).collect()));
                        }
                    }
                    Some(Aperture::Rect(w, h)) => {
                        // A rectangle swept along a straight segment: the hull of its two ends.
                        for s in pts.windows(2) {
                            let mut c = Vec::new();
                            for e in [s[0], s[1]] {
                                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                                    c.push([e[0] + sx * w / 2.0, e[1] + sy * h / 2.0]);
                                }
                            }
                            out.push((true, hull(&c)));
                        }
                    }
                    _ => {}
                }
                out
            }
        }
    }

    fn aperture_polys(&self, d: u32, tol: f64) -> Vec<(bool, Vec<P>)> {
        match self.apertures.get(&d) {
            Some(Aperture::Circle(dia)) => vec![(true, circle([0.0, 0.0], dia / 2.0, tol))],
            Some(Aperture::Rect(w, h)) => vec![(true, vec![[-w / 2.0, -h / 2.0], [w / 2.0, -h / 2.0], [w / 2.0, h / 2.0], [-w / 2.0, h / 2.0]])],
            Some(Aperture::Obround(w, h)) => {
                let r = w.min(*h) / 2.0;
                let (a, b) = if w >= h { ([-(w / 2.0 - r), 0.0], [w / 2.0 - r, 0.0]) } else { ([0.0, -(h / 2.0 - r)], [0.0, h / 2.0 - r]) };
                vec![(true, capsule(a, b, r, tol))]
            }
            Some(Aperture::Polygon(dia, n, rot)) => vec![(true, regular(*n, [0.0, 0.0], dia / 2.0, *rot))],
            Some(Aperture::Macro(name, params)) => self.macro_defs.get(name).map(|body| eval_macro(body, params, tol).unwrap_or_default()).unwrap_or_default(),
            None => vec![],
        }
    }
}

fn rot(p: P, deg: f64) -> P {
    let (s, c) = deg.to_radians().sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}

fn segments(r: f64, tol: f64) -> usize {
    if r <= tol {
        return 8;
    }
    let a = 2.0 * (1.0 - tol / r).clamp(-1.0, 1.0).acos();
    ((std::f64::consts::TAU / a).ceil() as usize).clamp(8, 256)
}

fn circle(c: P, r: f64, tol: f64) -> Vec<P> {
    let n = segments(r, tol);
    (0..n).map(|i| {
        let a = std::f64::consts::TAU * i as f64 / n as f64;
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    }).collect()
}

fn regular(n: u32, c: P, r: f64, deg: f64) -> Vec<P> {
    let n = n.max(3);
    (0..n).map(|i| {
        let a = (deg + 360.0 * i as f64 / n as f64).to_radians();
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    }).collect()
}

/// A stadium around segment a→b (a disc when they coincide), counter-clockwise.
fn capsule(a: P, b: P, r: f64, tol: f64) -> Vec<P> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if l < 1e-12 {
        return circle(a, r, tol);
    }
    let base = d[1].atan2(d[0]);
    let n = segments(r, tol) / 2 + 1;
    let mut out = Vec::with_capacity(2 * n + 2);
    for (c, from) in [(b, base - std::f64::consts::FRAC_PI_2), (a, base + std::f64::consts::FRAC_PI_2)] {
        for i in 0..=n {
            let t = from + std::f64::consts::PI * i as f64 / n as f64;
            out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
        }
    }
    out
}

fn hull(points: &[P]) -> Vec<P> {
    let mut p: Vec<P> = points.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: P, a: P, b: P| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<P> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<P> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

// ---- Aperture macros --------------------------------------------------------------

/// Arithmetic of macro parameters: numbers, `$n`, + − x / and brackets.
fn eval_expr(s: &str, vars: &BTreeMap<u32, f64>) -> Result<f64, String> {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut pos = 0;
    fn sum(c: &[char], p: &mut usize, v: &BTreeMap<u32, f64>) -> Result<f64, String> {
        let mut x = product(c, p, v)?;
        while *p < c.len() && (c[*p] == '+' || c[*p] == '-') {
            let op = c[*p];
            *p += 1;
            let y = product(c, p, v)?;
            x = if op == '+' { x + y } else { x - y };
        }
        Ok(x)
    }
    fn product(c: &[char], p: &mut usize, v: &BTreeMap<u32, f64>) -> Result<f64, String> {
        let mut x = unary(c, p, v)?;
        while *p < c.len() && (c[*p] == 'x' || c[*p] == 'X' || c[*p] == '/') {
            let op = c[*p];
            *p += 1;
            let y = unary(c, p, v)?;
            x = if op == '/' { x / y } else { x * y };
        }
        Ok(x)
    }
    fn unary(c: &[char], p: &mut usize, v: &BTreeMap<u32, f64>) -> Result<f64, String> {
        match c.get(*p) {
            Some('-') => {
                *p += 1;
                Ok(-unary(c, p, v)?)
            }
            Some('+') => {
                *p += 1;
                unary(c, p, v)
            }
            Some('(') => {
                *p += 1;
                let x = sum(c, p, v)?;
                if c.get(*p) != Some(&')') {
                    return Err("unbalanced bracket in a macro".into());
                }
                *p += 1;
                Ok(x)
            }
            Some('$') => {
                *p += 1;
                let start = *p;
                while *p < c.len() && c[*p].is_ascii_digit() {
                    *p += 1;
                }
                let n: u32 = c[start..*p].iter().collect::<String>().parse().map_err(|_| "bad macro variable")?;
                Ok(*v.get(&n).unwrap_or(&0.0))
            }
            Some(_) => {
                let start = *p;
                while *p < c.len() && (c[*p].is_ascii_digit() || c[*p] == '.') {
                    *p += 1;
                }
                let t: String = c[start..*p].iter().collect();
                t.parse().map_err(|_| format!("bad number “{t}” in a macro"))
            }
            None => Err("a macro expression ends early".into()),
        }
    }
    let x = sum(&chars, &mut pos, vars)?;
    if pos != chars.len() {
        return Err(format!("cannot read “{s}” in a macro"));
    }
    Ok(x)
}

/// A macro's primitives as polygons (aperture coordinates) with their exposure.
pub fn eval_macro(body: &[String], params: &[f64], tol: f64) -> Result<Vec<(bool, Vec<P>)>, String> {
    let mut vars: BTreeMap<u32, f64> = params.iter().enumerate().map(|(i, v)| (i as u32 + 1, *v)).collect();
    let mut out = Vec::new();
    for stmt in body {
        let stmt = stmt.trim();
        if stmt.is_empty() || stmt.starts_with('0') && (stmt.len() == 1 || stmt.as_bytes()[1] == b' ' || stmt.as_bytes()[1] == b',') {
            continue;
        }
        if let Some(rest) = stmt.strip_prefix('$') {
            let (n, e) = rest.split_once('=').ok_or("bad macro assignment")?;
            let n: u32 = n.trim().parse().map_err(|_| "bad macro variable")?;
            let v = eval_expr(e, &vars)?;
            vars.insert(n, v);
            continue;
        }
        let f: Vec<f64> = stmt.split(',').map(|t| eval_expr(t, &vars)).collect::<Result<_, _>>()?;
        let code = f[0] as i64;
        let a = &f[1..];
        let on = |x: f64| x != 0.0;
        let need = |n: usize| if a.len() < n { Err(format!("macro primitive {code} needs {n} values")) } else { Ok(()) };
        match code {
            1 => {
                need(4)?;
                let rotation = a.get(4).copied().unwrap_or(0.0);
                let c = rot([a[2], a[3]], rotation);
                out.push((on(a[0]), circle(c, a[1] / 2.0, tol)));
            }
            20 => {
                need(7)?;
                let (w, s, e, r) = (a[1], [a[2], a[3]], [a[4], a[5]], a[6]);
                let d = [e[0] - s[0], e[1] - s[1]];
                let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-12);
                let n = [-d[1] / l * w / 2.0, d[0] / l * w / 2.0];
                let poly = [[s[0] + n[0], s[1] + n[1]], [e[0] + n[0], e[1] + n[1]], [e[0] - n[0], e[1] - n[1]], [s[0] - n[0], s[1] - n[1]]];
                out.push((on(a[0]), poly.iter().map(|p| rot(*p, r)).collect()));
            }
            21 => {
                need(6)?;
                let (w, h, c, r) = (a[1], a[2], [a[3], a[4]], a[5]);
                let poly = [[c[0] - w / 2.0, c[1] - h / 2.0], [c[0] + w / 2.0, c[1] - h / 2.0], [c[0] + w / 2.0, c[1] + h / 2.0], [c[0] - w / 2.0, c[1] + h / 2.0]];
                out.push((on(a[0]), poly.iter().map(|p| rot(*p, r)).collect()));
            }
            4 => {
                need(2)?;
                let n = a[1] as usize;
                need(2 + 2 * (n + 1) + 1)?;
                let r = a[2 + 2 * (n + 1)];
                let pts: Vec<P> = (0..n).map(|i| rot([a[2 + 2 * i], a[3 + 2 * i]], r)).collect();
                out.push((on(a[0]), pts));
            }
            5 => {
                need(5)?;
                let r = a.get(5).copied().unwrap_or(0.0);
                let c = rot([a[2], a[3]], r);
                out.push((on(a[0]), regular(a[1] as u32, c, a[4] / 2.0, r)));
            }
            other => return Err(format!("macro primitive {other} is not read (thermal and moiré primitives are not supported)")),
        }
    }
    Ok(out)
}

// ---- Gerber -------------------------------------------------------------------------

fn num(s: &str) -> Result<f64, String> {
    s.trim().parse::<f64>().map_err(|_| format!("bad number “{s}”"))
}

/// Points along an arc from `a` to `b` about `c`, clockwise or not (multi-quadrant).
fn arc_points(a: P, b: P, c: P, clockwise: bool, tol: f64) -> Vec<P> {
    let r = ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2)).sqrt();
    let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
    let a1 = (b[1] - c[1]).atan2(b[0] - c[0]);
    let tau = std::f64::consts::TAU;
    let mut span = if clockwise { -(a0 - a1).rem_euclid(tau) } else { (a1 - a0).rem_euclid(tau) };
    if span.abs() < 1e-9 && (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9 {
        // Same start and end in multi-quadrant mode: a full circle.
        span = if clockwise { -tau } else { tau };
    }
    let n = ((span.abs() / tau * segments(r.max(1e-6), tol) as f64).ceil() as usize).max(2);
    let mut out: Vec<P> = (0..=n).map(|i| {
        let t = a0 + span * i as f64 / n as f64;
        [c[0] + r * t.cos(), c[1] + r * t.sin()]
    }).collect();
    *out.last_mut().unwrap() = b;
    out
}

pub fn gerber(text: &str) -> Result<GerberFile, String> {
    let mut g = GerberFile::default();
    let mut have_fs = false;
    let mut have_mo = false;
    let mut current: Option<u32> = None;
    let mut pos = [0.0f64, 0.0];
    let mut in_region = false;
    let mut contour: Vec<[f64; 2]> = Vec::new();
    let mut pending_function: Option<String> = None;
    let mut object_attrs: BTreeMap<String, String> = BTreeMap::new();
    let mut dark = true;
    // 1 linear, 2 clockwise, 3 counter-clockwise.
    let mut mode = 1;
    let mut stroke: Vec<P> = Vec::new();
    let mut ended = false;
    let mut i = 0;
    let bytes = text.as_bytes();
    let tol = 0.002;

    macro_rules! flush_stroke {
        () => {
            if stroke.len() >= 2 {
                let d = current.ok_or("draw with no aperture")?;
                g.objects.push(Object { shape: Shape::Stroke { d, pts: std::mem::take(&mut stroke) }, dark, function: g.aperture_functions.get(&d).cloned(), net: object_attrs.get("N").cloned(), component: object_attrs.get("C").cloned(), pin: object_attrs.get("P").cloned() });
            }
            stroke.clear();
        };
    }

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '%' {
            let end = text[i + 1..].find('%').ok_or("unterminated extended command")? + i + 1;
            let body = &text[i + 1..end];
            i = end + 1;
            let cmd = body.trim().trim_end_matches(['*', '\n', '\r', ' ']);
            // A stroke ends where polarity or object attributes change (each draw is its own object).
            if !cmd.starts_with("AM") && !cmd.starts_with("TA") && !cmd.starts_with("TF") {
                flush_stroke!();
            }
            if let Some(rest) = cmd.strip_prefix("FS") {
                let x = rest.find('X').ok_or("FS without X")?;
                let digits = &rest[x + 1..x + 3];
                g.format = (digits[0..1].parse().map_err(|_| "FS digits")?, digits[1..2].parse().map_err(|_| "FS digits")?);
                if !rest.starts_with("LA") {
                    return Err("only leading-zero-omitted absolute coordinates are read".into());
                }
                have_fs = true;
            } else if let Some(u) = cmd.strip_prefix("MO") {
                g.units_mm = u == "MM";
                if u != "MM" && u != "IN" {
                    return Err(format!("unknown unit {u}"));
                }
                have_mo = true;
            } else if let Some(rest) = cmd.strip_prefix("TF.") {
                let (k, v) = rest.split_once(',').unwrap_or((rest, ""));
                g.attributes.insert(k.to_string(), v.to_string());
            } else if let Some(rest) = cmd.strip_prefix("TA.AperFunction,") {
                pending_function = Some(rest.to_string());
            } else if cmd.starts_with("TA") {
                // Other aperture attributes: kept by nobody here.
            } else if let Some(rest) = cmd.strip_prefix("TO.") {
                let (k, v) = rest.split_once(',').unwrap_or((rest, ""));
                if k == "N" && !g.nets.contains(&v.to_string()) {
                    g.nets.push(v.to_string());
                }
                object_attrs.insert(k.to_string(), v.to_string());
            } else if let Some(rest) = cmd.strip_prefix("TD") {
                let rest = rest.trim_start_matches('.');
                if rest.is_empty() {
                    pending_function = None;
                    object_attrs.clear();
                } else if rest == "AperFunction" {
                    pending_function = None;
                } else {
                    object_attrs.remove(rest);
                }
            } else if let Some(rest) = cmd.strip_prefix("AM") {
                let mut parts = rest.split('*');
                let name = parts.next().unwrap_or("").trim().to_string();
                let stmts: Vec<String> = parts.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                g.macros.push(name.clone());
                g.macro_defs.insert(name, stmts);
            } else if let Some(rest) = cmd.strip_prefix("ADD") {
                let k = rest.find(|c: char| !c.is_ascii_digit()).ok_or("bad AD")?;
                let d: u32 = rest[..k].parse().map_err(|_| "bad D-code")?;
                if d < 10 {
                    return Err(format!("aperture D{d}: D-codes start at 10"));
                }
                let def = &rest[k..];
                let (shape, params) = def.split_once(',').unwrap_or((def, ""));
                let p: Vec<f64> = if params.is_empty() { vec![] } else { params.split('X').map(num).collect::<Result<_, _>>()? };
                let hole = |n: usize| if p.len() > n { Err(format!("aperture D{d} has a hole; aperture holes are not read")) } else { Ok(()) };
                let ap = match shape {
                    "C" => {
                        hole(1)?;
                        Aperture::Circle(*p.first().ok_or("C needs a diameter")?)
                    }
                    "R" => {
                        hole(2)?;
                        Aperture::Rect(p[0], *p.get(1).ok_or("R needs w and h")?)
                    }
                    "O" => {
                        hole(2)?;
                        Aperture::Obround(p[0], *p.get(1).ok_or("O needs w and h")?)
                    }
                    "P" => {
                        hole(3)?;
                        Aperture::Polygon(*p.first().ok_or("P needs a diameter")?, *p.get(1).ok_or("P needs vertices")? as u32, p.get(2).copied().unwrap_or(0.0))
                    }
                    other => {
                        if !g.macros.iter().any(|m| m == other) {
                            return Err(format!("aperture D{d} uses undefined macro {other}"));
                        }
                        eval_macro(&g.macro_defs[other], &p, tol).map_err(|e| format!("aperture D{d}: {e}"))?;
                        Aperture::Macro(other.to_string(), p)
                    }
                };
                if g.apertures.insert(d, ap).is_some() {
                    return Err(format!("aperture D{d} defined twice"));
                }
                if let Some(f) = &pending_function {
                    g.aperture_functions.insert(d, f.clone());
                }
            } else if let Some(p) = cmd.strip_prefix("LP") {
                dark = match p {
                    "D" => true,
                    "C" => false,
                    _ => return Err(format!("unknown polarity {p}")),
                };
            } else if cmd == "IPPOS" || cmd == "OFA0B0" || cmd == "SFA1B1" || cmd == "MIA0B0" || cmd == "LMN" || cmd == "LR0" || cmd == "LS1" || cmd.starts_with("IN") || cmd.starts_with("LN") || cmd == "ASAXBY" {
                // Defaults spelled out (and names): nothing changes.
            } else {
                return Err(format!("unsupported extended command %{cmd}%"));
            }
            continue;
        }
        let end = text[i..].find('*').ok_or("word without *")? + i;
        let mut word = text[i..end].trim();
        i = end + 1;
        if word.starts_with("G04") || word.starts_with("G4 ") {
            continue;
        }
        if ended {
            return Err("content after M02".into());
        }
        // A leading G code may share the word with coordinates (G01X…Y…D01).
        for (code, m) in [("G01", 1), ("G02", 2), ("G03", 3), ("G1", 1), ("G2", 2), ("G3", 3)] {
            if let Some(rest) = word.strip_prefix(code) {
                if rest.is_empty() || rest.starts_with(['X', 'Y', 'I', 'J', 'D']) {
                    mode = m;
                    word = rest;
                    break;
                }
            }
        }
        match word {
            "" | "G75" | "G74" | "G70" | "G71" | "G90" | "G91" | "M00" | "M01" => {
                if word == "G70" || word == "G71" {
                    return Err("G70/G71 units are not read (use %MO%)".into());
                }
                if word == "G91" {
                    return Err("incremental coordinates are not read".into());
                }
                continue;
            }
            "G36" => {
                flush_stroke!();
                in_region = true;
                contour.clear();
                continue;
            }
            "G37" => {
                if !in_region {
                    return Err("G37 without G36".into());
                }
                if contour.len() >= 3 {
                    let c = std::mem::take(&mut contour);
                    g.regions.push(c.clone());
                    g.objects.push(Object { shape: Shape::Region(c), dark, function: pending_function.clone(), net: object_attrs.get("N").cloned(), component: object_attrs.get("C").cloned(), pin: object_attrs.get("P").cloned() });
                }
                in_region = false;
                continue;
            }
            "M02" => {
                flush_stroke!();
                ended = true;
                continue;
            }
            _ => {}
        }
        let dsel = word.strip_prefix('D').or_else(|| word.strip_prefix("G54D")).and_then(|s| s.parse::<u32>().ok());
        if let Some(d) = dsel {
            if d >= 10 {
                if !g.apertures.contains_key(&d) {
                    return Err(format!("D{d} selected but not defined"));
                }
                flush_stroke!();
                current = Some(d);
                continue;
            }
        }
        if !(have_fs && have_mo) {
            return Err("coordinates before the format (FS) and unit (MO)".into());
        }
        // X…Y…I…J…Dnn
        let scale = 10f64.powi(g.format.1 as i32) * if g.units_mm { 1.0 } else { 1.0 / 25.4 };
        let mut next = pos;
        let mut ij = [0.0f64, 0.0];
        let mut op = None;
        let mut rest = word;
        while !rest.is_empty() {
            let key = rest.chars().next().unwrap();
            let k = rest[1..].find(|c: char| c.is_ascii_alphabetic()).map(|k| k + 1).unwrap_or(rest.len());
            let val = &rest[1..k];
            let int = || val.parse::<i64>().map_err(|_| format!("bad {key} in {word}"));
            match key {
                'X' => next[0] = int()? as f64 / scale,
                'Y' => next[1] = int()? as f64 / scale,
                'I' => ij[0] = int()? as f64 / scale,
                'J' => ij[1] = int()? as f64 / scale,
                'D' => op = Some(val.parse::<u32>().map_err(|_| format!("bad D in {word}"))?),
                other => return Err(format!("unexpected {other} in {word}")),
            }
            rest = &rest[k..];
        }
        match op {
            Some(1) => {
                let path: Vec<P> = if mode == 1 { vec![pos, next] } else { arc_points(pos, next, [pos[0] + ij[0], pos[1] + ij[1]], mode == 2, tol) };
                if in_region {
                    if contour.is_empty() {
                        contour.push(pos);
                    }
                    contour.extend(path.into_iter().skip(1));
                } else {
                    let d = current.ok_or("draw with no aperture")?;
                    if mode == 1 {
                        g.draws.push((d, pos, next));
                    }
                    if stroke.is_empty() {
                        stroke.push(pos);
                    } else if *stroke.last().unwrap() != pos {
                        flush_stroke!();
                        stroke.push(pos);
                    }
                    stroke.extend(path.into_iter().skip(1));
                }
            }
            Some(2) => {
                flush_stroke!();
                if in_region && contour.len() >= 3 {
                    let c = std::mem::take(&mut contour);
                    g.regions.push(c.clone());
                    g.objects.push(Object { shape: Shape::Region(c), dark, function: pending_function.clone(), net: object_attrs.get("N").cloned(), component: object_attrs.get("C").cloned(), pin: object_attrs.get("P").cloned() });
                }
                contour.clear();
            }
            Some(3) => {
                if in_region {
                    return Err("flash inside a region".into());
                }
                flush_stroke!();
                let d = current.ok_or("flash with no aperture")?;
                g.flashes.push((d, next));
                g.objects.push(Object { shape: Shape::Flash { d, at: next }, dark, function: g.aperture_functions.get(&d).cloned(), net: object_attrs.get("N").cloned(), component: object_attrs.get("C").cloned(), pin: object_attrs.get("P").cloned() });
            }
            _ => return Err(format!("coordinate word without operation: {word}")),
        }
        pos = next;
    }
    if !ended {
        return Err("no M02 at the end".into());
    }
    if in_region {
        return Err("region not closed".into());
    }
    Ok(g)
}

// ---- Excellon ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct DrillFile {
    pub metric: bool,
    pub plated: Option<bool>,
    pub tools: BTreeMap<u32, f64>,
    pub hits: Vec<(u32, [f64; 2])>,
    pub slots: Vec<(u32, [f64; 2], [f64; 2])>,
}

pub fn excellon(text: &str) -> Result<DrillFile, String> {
    let mut f = DrillFile::default();
    let mut header = false;
    let mut body = false;
    let mut tool = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(c) = line.strip_prefix(';') {
            if c.contains("TF.FileFunction,Plated") {
                f.plated = Some(true);
            } else if c.contains("TF.FileFunction,NonPlated") {
                f.plated = Some(false);
            }
            continue;
        }
        match line {
            "M48" => {
                header = true;
                continue;
            }
            "%" | "M95" => {
                body = true;
                continue;
            }
            "METRIC" | "METRIC,TZ" | "METRIC,LZ" => {
                f.metric = true;
                continue;
            }
            "INCH" | "INCH,TZ" | "INCH,LZ" => return Err("inch drill files are not read".into()),
            "G90" | "G05" | "FMAT,2" => continue,
            "M30" => return if header { Ok(f) } else { Err("no M48 header".into()) },
            _ => {}
        }
        if let Some(rest) = line.strip_prefix('T') {
            let k = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            let t: u32 = rest[..k].parse().map_err(|_| format!("bad tool {line}"))?;
            if let Some(d) = rest[k..].strip_prefix('C') {
                if body {
                    return Err("tool defined after the header".into());
                }
                f.tools.insert(t, num(d)?);
            } else {
                tool = if t == 0 { None } else { Some(t) };
                if t != 0 && !f.tools.contains_key(&t) {
                    return Err(format!("T{t} used but not defined"));
                }
            }
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            let t = tool.ok_or("hole before a tool")?;
            if !line.contains('.') {
                return Err(format!("“{line}”: only decimal coordinates are read"));
            }
            let parse = |s: &str| -> Result<[f64; 2], String> {
                let x = s.find('X').ok_or("no X")?;
                let y = s.find('Y').ok_or("no Y")?;
                Ok([num(&s[x + 1..y])?, num(&s[y + 1..])?])
            };
            if let Some((a, b)) = line.split_once("G85") {
                f.slots.push((t, parse(a)?, parse(b)?));
            } else {
                f.hits.push((t, parse(line)?));
            }
            continue;
        }
        return Err(format!("unsupported line {line}"));
    }
    Err("no M30 at the end".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(p: &[P]) -> f64 {
        let n = p.len();
        (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f64>() / 2.0
    }

    #[test]
    fn macros_evaluate_with_variables() {
        let body = vec!["0 a comment".to_string(), "$3=$1x2".to_string(), "21,1,$3,$2,0,0,45".to_string()];
        let polys = eval_macro(&body, &[1.0, 0.5], 0.001).unwrap();
        assert_eq!(polys.len(), 1);
        assert!((area(&polys[0].1).abs() - 1.0).abs() < 1e-9);
        assert!(eval_expr("(1+2)x3-$1/2", &BTreeMap::from([(1, 4.0)])).unwrap() == 7.0);
    }

    #[test]
    fn arcs_and_polarity_read() {
        let text = "%FSLAX46Y46*%\n%MOMM*%\n%ADD10C,0.100000*%\nD10*\nX1000000Y0D02*\nG75*\nG03X-1000000Y0I-1000000J0D01*\n%LPC*%\nX0Y0D03*\nM02*\n";
        let g = gerber(text).unwrap();
        assert_eq!(g.objects.len(), 2);
        let Shape::Stroke { pts, .. } = &g.objects[0].shape else { panic!() };
        // Half a circle of radius 1 anticlockwise from (1, 0): it passes the top.
        assert!(pts.iter().any(|p| p[1] > 0.99));
        assert!(!g.objects[1].dark);
        assert!(gerber("%FSLAX46Y46*%\n%MOMM*%\n%SRX2Y2I1J1*%\nM02*\n").is_err(), "step and repeat is refused");
    }
}
