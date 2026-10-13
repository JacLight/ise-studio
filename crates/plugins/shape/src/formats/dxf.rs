//! Reading DXF drawings (text DXF, any version) into sketch geometry: lines,
//! arcs and circles stay lines, arcs and circles; polylines with bulges
//! become lines and arcs; splines and ellipses become Béziers; blocks are
//! placed where they are inserted. Units follow the file's $INSUNITS
//! (millimetres when it does not say). Paper space, text, dimensions and
//! hatches are left out.

use super::outline::Builder;
use std::collections::HashMap;

/// A 2D affine map: x' = a·x + c·y + e, y' = b·x + d·y + f.
#[derive(Clone, Copy)]
pub(super) struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    pub fn at(&self, p: [f64; 2]) -> [f64; 2] {
        let m = &self.0;
        [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
    }
    /// `self` after `inner` (inner applied first).
    pub fn after(&self, inner: &Affine) -> Affine {
        let (a, b) = (&self.0, &inner.0);
        Affine([
            a[0] * b[0] + a[2] * b[1],
            a[1] * b[0] + a[3] * b[1],
            a[0] * b[2] + a[2] * b[3],
            a[1] * b[2] + a[3] * b[3],
            a[0] * b[4] + a[2] * b[5] + a[4],
            a[1] * b[4] + a[3] * b[5] + a[5],
        ])
    }
    pub fn det(&self) -> f64 {
        self.0[0] * self.0[3] - self.0[1] * self.0[2]
    }
    /// The uniform scale, if this keeps circles circles.
    pub fn similarity(&self) -> Option<f64> {
        let m = &self.0;
        let (sx, sy) = (m[0].hypot(m[1]), m[2].hypot(m[3]));
        let dot = m[0] * m[2] + m[1] * m[3];
        ((sx - sy).abs() <= 1e-9 * sx.max(1.0) && dot.abs() <= 1e-9 * sx * sy.max(1.0)).then_some(sx)
    }
}

/// One entity: its type and group codes in order.
struct Entity<'a> {
    kind: &'a str,
    codes: Vec<(i32, &'a str)>,
}

impl<'a> Entity<'a> {
    fn f(&self, code: i32) -> Option<f64> {
        self.codes.iter().find(|(c, _)| *c == code).and_then(|(_, v)| v.trim().parse().ok())
    }
    fn num(&self, code: i32, default: f64) -> f64 {
        self.f(code).unwrap_or(default)
    }
    fn s(&self, code: i32) -> Option<&'a str> {
        self.codes.iter().find(|(c, _)| *c == code).map(|(_, v)| v.trim())
    }
    /// Every value of a code, in order.
    fn all(&self, code: i32) -> Vec<f64> {
        self.codes.iter().filter(|(c, _)| *c == code).filter_map(|(_, v)| v.trim().parse().ok()).collect()
    }
    /// The object coordinate system is mirrored (extrusion along −Z): x flips.
    fn ocs(&self) -> Affine {
        if self.num(230, 1.0) < 0.0 { Affine([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY }
    }
}

struct Block<'a> {
    base: [f64; 2],
    entities: Vec<Entity<'a>>,
}

/// The arc through a polyline segment with a bulge (tan of a quarter of its angle; positive counter-clockwise).
fn bulge_arc(b: &mut Builder, t: &Affine, p: [f64; 2], q: [f64; 2], bulge: f64) -> Result<(), String> {
    if bulge.abs() < 1e-12 {
        return b.line(t.at(p), t.at(q));
    }
    let chord = (q[0] - p[0]).hypot(q[1] - p[1]);
    if chord < 1e-12 {
        return Ok(());
    }
    let angle = 4.0 * bulge.atan();
    let r = chord / (2.0 * (angle / 2.0).sin()).abs();
    // The centre lies off the chord's midpoint, to the left for a counter-clockwise arc.
    let mid = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
    let d = (r * r - chord * chord / 4.0).max(0.0).sqrt();
    let n = [-(q[1] - p[1]) / chord, (q[0] - p[0]) / chord];
    let side = if (bulge > 0.0) == (angle.abs() < std::f64::consts::PI) { 1.0 } else { -1.0 };
    let c = [mid[0] + side * d * n[0], mid[1] + side * d * n[1]];
    if bulge > 0.0 { arc(b, t, c, r, (p[1] - c[1]).atan2(p[0] - c[0]), (q[1] - c[1]).atan2(q[0] - c[0])) } else { arc(b, t, c, r, (q[1] - c[1]).atan2(q[0] - c[0]), (p[1] - c[1]).atan2(p[0] - c[0])) }
}

/// A counter-clockwise arc (radians) under a transform: still an arc when the
/// transform keeps circles round, else Béziers.
fn arc(b: &mut Builder, t: &Affine, c: [f64; 2], r: f64, a0: f64, a1: f64) -> Result<(), String> {
    let mut a1 = a1;
    while a1 <= a0 + 1e-12 {
        a1 += std::f64::consts::TAU;
    }
    let p = |a: f64| [c[0] + r * a.cos(), c[1] + r * a.sin()];
    if t.similarity().is_some() {
        let (s, e) = (t.at(p(a0)), t.at(p(a1)));
        // A mirror turns counter-clockwise to clockwise: swap the ends.
        return if t.det() < 0.0 { b.arc(t.at(c), e, s) } else { b.arc(t.at(c), s, e) };
    }
    ellipse(b, t, c, [r, 0.0], 1.0, a0, a1)
}

/// An elliptical arc (centre, major axis vector, minor/major ratio, from a0 to a1 counter-clockwise) as Béziers.
pub(super) fn ellipse(b: &mut Builder, t: &Affine, c: [f64; 2], major: [f64; 2], ratio: f64, a0: f64, a1: f64) -> Result<(), String> {
    let minor = [-major[1] * ratio, major[0] * ratio];
    let at = |a: f64| [c[0] + major[0] * a.cos() + minor[0] * a.sin(), c[1] + major[1] * a.cos() + minor[1] * a.sin()];
    let d = |a: f64| [-major[0] * a.sin() + minor[0] * a.cos(), -major[1] * a.sin() + minor[1] * a.cos()];
    let n = ((a1 - a0) / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = (a1 - a0) / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    for i in 0..n {
        let (s, e) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
        let (p0, p3, d0, d3) = (at(s), at(e), d(s), d(e));
        let p1 = [p0[0] + k * d0[0], p0[1] + k * d0[1]];
        let p2 = [p3[0] - k * d3[0], p3[1] - k * d3[1]];
        b.bezier(t.at(p0), t.at(p1), t.at(p2), t.at(p3))?;
    }
    Ok(())
}

/// A B-spline as Béziers (degree 3 or less, not rational), else as a fine polyline.
fn spline(b: &mut Builder, t: &Affine, e: &Entity) -> Result<(), String> {
    let p = e.num(71, 3.0) as usize;
    let mut knots = e.all(40);
    let (xs, ys) = (e.all(10), e.all(20));
    let mut ctrl: Vec<[f64; 2]> = xs.iter().zip(&ys).map(|(x, y)| [*x, *y]).collect();
    let weights = e.all(41);
    if ctrl.len() < 2 || knots.len() != ctrl.len() + p + 1 || p == 0 {
        // Fit points only: a polyline through them.
        let fit: Vec<[f64; 2]> = e.all(11).iter().zip(&e.all(21)).map(|(x, y)| [*x, *y]).collect();
        let pts: Vec<[f64; 2]> = if fit.len() >= 2 { fit } else { ctrl };
        let pts: Vec<[f64; 2]> = pts.iter().map(|q| t.at(*q)).collect();
        return b.polyline(&pts, e.num(70, 0.0) as i32 & 1 == 1);
    }
    let rational = weights.len() == ctrl.len() && weights.iter().any(|w| (w - weights[0]).abs() > 1e-12);
    if p > 3 || rational {
        // Sample it (de Boor), 16 points per knot span.
        let w = |i: usize| if rational { weights[i] } else { 1.0 };
        let eval = |u: f64| {
            let mut k = p;
            while k + 1 < ctrl.len() && knots[k + 1] <= u {
                k += 1;
            }
            let mut d: Vec<[f64; 3]> = (0..=p).map(|j| [ctrl[j + k - p][0] * w(j + k - p), ctrl[j + k - p][1] * w(j + k - p), w(j + k - p)]).collect();
            for r in 1..=p {
                for j in (r..=p).rev() {
                    let i = j + k - p;
                    let den = knots[i + p + 1 - r] - knots[i];
                    let a = if den.abs() < 1e-15 { 0.0 } else { (u - knots[i]) / den };
                    for c in 0..3 {
                        d[j][c] = (1.0 - a) * d[j - 1][c] + a * d[j][c];
                    }
                }
            }
            [d[p][0] / d[p][2], d[p][1] / d[p][2]]
        };
        let (u0, u1) = (knots[p], knots[ctrl.len()]);
        let n = 16 * (ctrl.len() - p).max(1);
        let pts: Vec<[f64; 2]> = (0..=n).map(|i| t.at(eval(u0 + (u1 - u0) * i as f64 / n as f64))).collect();
        return b.polyline(&pts, false);
    }
    // Insert knots until every one in the domain has multiplicity p: the
    // control polygon is then a chain of Bézier segments.
    let insert = |knots: &mut Vec<f64>, ctrl: &mut Vec<[f64; 2]>, u: f64| {
        let mut k = p;
        while k + 1 < knots.len() - p - 1 && knots[k + 1] <= u {
            k += 1;
        }
        let mut next = Vec::with_capacity(ctrl.len() + 1);
        for i in 0..=ctrl.len() {
            if i + p <= k {
                next.push(ctrl[i]);
            } else if i > k {
                next.push(ctrl[i - 1]);
            } else {
                let a = (u - knots[i]) / (knots[i + p] - knots[i]);
                next.push([(1.0 - a) * ctrl[i - 1][0] + a * ctrl[i][0], (1.0 - a) * ctrl[i - 1][1] + a * ctrl[i][1]]);
            }
        }
        *ctrl = next;
        knots.insert(k + 1, u);
    };
    let (u0, u1) = (knots[p], knots[ctrl.len()]);
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 10_000 {
            return Err("a spline in the drawing could not be converted".into());
        }
        let n = ctrl.len();
        let pick = (p..=n).map(|i| knots[i]).find(|&u| u >= u0 && u <= u1 && knots.iter().filter(|&&k| (k - u).abs() < 1e-12).count() < p);
        let Some(u) = pick else { break };
        insert(&mut knots, &mut ctrl, u);
    }
    for j in p..ctrl.len() {
        if knots[j + 1] - knots[j] < 1e-12 {
            continue;
        }
        let q: Vec<[f64; 2]> = ctrl[j - p..=j].iter().map(|c| t.at(*c)).collect();
        match p {
            1 => b.line(q[0], q[1])?,
            2 => {
                let c1 = [q[0][0] + 2.0 / 3.0 * (q[1][0] - q[0][0]), q[0][1] + 2.0 / 3.0 * (q[1][1] - q[0][1])];
                let c2 = [q[2][0] + 2.0 / 3.0 * (q[1][0] - q[2][0]), q[2][1] + 2.0 / 3.0 * (q[1][1] - q[2][1])];
                b.bezier(q[0], c1, c2, q[2])?
            }
            _ => b.bezier(q[0], q[1], q[2], q[3])?,
        }
    }
    Ok(())
}

fn draw(b: &mut Builder, blocks: &HashMap<String, Block>, list: &[Entity], t: &Affine, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("the drawing's blocks nest too deep (a block inside itself?)".into());
    }
    let mut i = 0;
    while i < list.len() {
        let e = &list[i];
        i += 1;
        if e.s(67) == Some("1") {
            continue; // paper space
        }
        let o = t.after(&e.ocs());
        let pt = |x: i32| [e.num(x, 0.0), e.num(x + 10, 0.0)];
        match e.kind {
            "LINE" => b.line(o.at(pt(10)), o.at(pt(11)))?,
            "CIRCLE" => {
                let r = e.num(40, 0.0);
                match o.similarity() {
                    Some(s) => b.circle(o.at(pt(10)), r * s)?,
                    None => ellipse(b, &o, pt(10), [r, 0.0], 1.0, 0.0, std::f64::consts::TAU)?,
                }
            }
            "ARC" => arc(b, &o, pt(10), e.num(40, 0.0), e.num(50, 0.0).to_radians(), e.num(51, 360.0).to_radians())?,
            "ELLIPSE" => {
                let (a0, a1) = (e.num(41, 0.0), e.num(42, std::f64::consts::TAU));
                ellipse(b, &o, pt(10), pt(11), e.num(40, 1.0), a0, if a1 <= a0 { a1 + std::f64::consts::TAU } else { a1 })?
            }
            "LWPOLYLINE" => {
                // Vertices in order, each with the bulge of the segment leaving it.
                let mut verts: Vec<([f64; 2], f64)> = Vec::new();
                for (c, v) in &e.codes {
                    let v: f64 = v.trim().parse().unwrap_or(0.0);
                    match c {
                        10 => verts.push(([v, 0.0], 0.0)),
                        20 => {
                            if let Some(last) = verts.last_mut() {
                                last.0[1] = v
                            }
                        }
                        42 => {
                            if let Some(last) = verts.last_mut() {
                                last.1 = v
                            }
                        }
                        _ => {}
                    }
                }
                polyline(b, &o, &verts, e.num(70, 0.0) as i32 & 1 == 1)?;
            }
            "POLYLINE" => {
                let mut verts = Vec::new();
                while i < list.len() && list[i].kind == "VERTEX" {
                    let v = &list[i];
                    verts.push(([v.num(10, 0.0), v.num(20, 0.0)], v.num(42, 0.0)));
                    i += 1;
                }
                if i < list.len() && list[i].kind == "SEQEND" {
                    i += 1;
                }
                // 3D polylines and meshes (flags 8, 16, 64) are not outlines.
                let flags = e.num(70, 0.0) as i32;
                if flags & (8 | 16 | 64) == 0 {
                    polyline(b, &o, &verts, flags & 1 == 1)?;
                }
            }
            "SPLINE" => spline(b, &o, e)?,
            "INSERT" => {
                let name = e.s(2).unwrap_or("");
                let Some(block) = blocks.get(name) else { continue };
                let (sx, sy) = (e.num(41, 1.0), e.num(42, 1.0));
                let a = e.num(50, 0.0).to_radians();
                let (cos, sin) = (a.cos(), a.sin());
                let at = pt(10);
                // Insert point + rotation · scale · (p − block base).
                let place = Affine([cos * sx, sin * sx, -sin * sy, cos * sy, at[0], at[1]]).after(&Affine([1.0, 0.0, 0.0, 1.0, -block.base[0], -block.base[1]]));
                // Rectangular arrays of the block (columns × rows).
                let (cols, rows) = (e.num(70, 1.0).max(1.0) as usize, e.num(71, 1.0).max(1.0) as usize);
                let (dx, dy) = (e.num(44, 0.0), e.num(45, 0.0));
                for r in 0..rows.min(1000) {
                    for c in 0..cols.min(1000) {
                        let off = [c as f64 * dx, r as f64 * dy];
                        let shift = Affine([1.0, 0.0, 0.0, 1.0, cos * off[0] - sin * off[1], sin * off[0] + cos * off[1]]);
                        draw(b, blocks, &block.entities, &o.after(&shift.after(&place)), depth + 1)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn polyline(b: &mut Builder, t: &Affine, verts: &[([f64; 2], f64)], closed: bool) -> Result<(), String> {
    let n = verts.len();
    let segments = if closed { n } else { n.saturating_sub(1) };
    for k in 0..segments {
        let (p, bulge) = verts[k];
        let q = verts[(k + 1) % n].0;
        bulge_arc(b, t, p, q, bulge)?;
    }
    Ok(())
}

/// A DXF file's model-space geometry, in millimetres.
pub fn read(data: &[u8]) -> Result<Builder, String> {
    if data.starts_with(b"AutoCAD Binary DXF") {
        return Err("binary DXF is not read: save it as text (ASCII) DXF".into());
    }
    let text = String::from_utf8_lossy(data);
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs: Vec<(i32, &str)> = Vec::with_capacity(lines.len() / 2);
    for w in lines.chunks(2) {
        let [code, value] = w else { break };
        let code: i32 = code.trim().parse().map_err(|_| "not a DXF file (group codes are not numbers)".to_string())?;
        pairs.push((code, value.trim_end_matches('\r')));
    }
    // Sections, and entities within them (each starts at a code 0).
    let mut units = 0;
    let mut blocks: HashMap<String, Block> = HashMap::new();
    let mut model: Vec<Entity> = Vec::new();
    let mut section = "";
    let mut current_block: Option<(String, Block)> = None;
    let mut i = 0;
    while i < pairs.len() {
        let (code, value) = pairs[i];
        i += 1;
        if code != 0 {
            continue;
        }
        let kind = value.trim();
        let start = i;
        while i < pairs.len() && pairs[i].0 != 0 {
            i += 1;
        }
        let e = Entity { kind, codes: pairs[start..i].to_vec() };
        match kind {
            "SECTION" => {
                section = e.s(2).unwrap_or("");
                // Header variables run on as one entity: `9 $INSUNITS 70 <units>`.
                if section == "HEADER"
                    && let Some(k) = e.codes.iter().position(|(c, v)| *c == 9 && v.trim() == "$INSUNITS")
                    && let Some((70, u)) = e.codes.get(k + 1)
                {
                    units = u.trim().parse().unwrap_or(0);
                }
            }
            "ENDSEC" => section = "",
            "EOF" => break,
            "BLOCK" if section == "BLOCKS" => current_block = Some((e.s(2).unwrap_or("").to_string(), Block { base: [e.num(10, 0.0), e.num(20, 0.0)], entities: Vec::new() })),
            "ENDBLK" => {
                if let Some((name, block)) = current_block.take() {
                    blocks.insert(name, block);
                }
            }
            _ if section == "BLOCKS" => {
                if let Some((_, block)) = &mut current_block {
                    block.entities.push(e);
                }
            }
            _ if section == "ENTITIES" => model.push(e),
            _ => {}
        }
    }
    let mm = match units {
        1 => 25.4,
        2 => 304.8,
        5 => 10.0,
        6 => 1000.0,
        8 => 25.4e-6,
        9 => 0.0254,
        10 => 914.4,
        14 => 100.0,
        _ => 1.0,
    };
    let mut b = Builder::default();
    draw(&mut b, &blocks, &model, &Affine([mm, 0.0, 0.0, mm, 0.0, 0.0]), 0)?;
    Ok(b)
}
