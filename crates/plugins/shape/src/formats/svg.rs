//! Reading SVG drawings into sketch geometry: paths, rectangles, circles,
//! ellipses, lines and polylines, through their transforms, in millimetres
//! (the drawing's own size when it gives one, else 96 px to the inch), Y up.
//! Straight segments stay lines, circles and circular arcs stay circles and
//! arcs, curves become Béziers. Text, images, `<defs>` and hidden elements
//! are left out.

use super::dxf::{Affine, ellipse};
use super::outline::Builder;
use super::xml::{Event, attr, events};

/// Numbers (and arc flags) in a path or list, in order.
struct Numbers<'a> {
    s: &'a [u8],
    at: usize,
}

impl Numbers<'_> {
    fn skip(&mut self) {
        while self.at < self.s.len() && (self.s[self.at].is_ascii_whitespace() || self.s[self.at] == b',') {
            self.at += 1;
        }
    }
    fn more(&mut self) -> bool {
        self.skip();
        self.at < self.s.len() && matches!(self.s[self.at], b'0'..=b'9' | b'.' | b'-' | b'+')
    }
    fn number(&mut self) -> Result<f64, String> {
        self.skip();
        let start = self.at;
        let s = self.s;
        let mut i = self.at;
        if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
            i += 1;
        }
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        if i < s.len() && s[i] == b'.' {
            i += 1;
            while i < s.len() && s[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < s.len() && (s[i] == b'e' || s[i] == b'E') && i + 1 < s.len() && (s[i + 1].is_ascii_digit() || s[i + 1] == b'-' || s[i + 1] == b'+') {
            i += 2;
            while i < s.len() && s[i].is_ascii_digit() {
                i += 1;
            }
        }
        self.at = i;
        std::str::from_utf8(&s[start..i]).ok().and_then(|t| t.parse().ok()).ok_or_else(|| "a number in the drawing does not read".to_string())
    }
    /// An arc flag: one character, 0 or 1 (often written with no separator).
    fn flag(&mut self) -> Result<bool, String> {
        self.skip();
        let c = self.s.get(self.at).copied();
        self.at += 1;
        match c {
            Some(b'0') => Ok(false),
            Some(b'1') => Ok(true),
            _ => Err("an arc flag in the drawing is not 0 or 1".into()),
        }
    }
}

fn numbers(s: &str) -> Vec<f64> {
    let mut n = Numbers { s: s.as_bytes(), at: 0 };
    let mut out = Vec::new();
    while n.more() {
        match n.number() {
            Ok(v) => out.push(v),
            Err(_) => break,
        }
    }
    out
}

/// A `transform` attribute as one affine map.
fn transform(s: &str) -> Affine {
    let mut m = Affine::IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let v = numbers(&rest[open + 1..open + close]);
        let g = |i: usize, d: f64| v.get(i).copied().unwrap_or(d);
        let t = match name {
            "matrix" if v.len() == 6 => Affine([v[0], v[1], v[2], v[3], v[4], v[5]]),
            "translate" => Affine([1.0, 0.0, 0.0, 1.0, g(0, 0.0), g(1, 0.0)]),
            "scale" => Affine([g(0, 1.0), 0.0, 0.0, g(1, g(0, 1.0)), 0.0, 0.0]),
            "rotate" => {
                let a = g(0, 0.0).to_radians();
                let (c, s) = (a.cos(), a.sin());
                let (cx, cy) = (g(1, 0.0), g(2, 0.0));
                Affine([1.0, 0.0, 0.0, 1.0, cx, cy]).after(&Affine([c, s, -s, c, 0.0, 0.0])).after(&Affine([1.0, 0.0, 0.0, 1.0, -cx, -cy]))
            }
            "skewX" => Affine([1.0, 0.0, g(0, 0.0).to_radians().tan(), 1.0, 0.0, 0.0]),
            "skewY" => Affine([1.0, g(0, 0.0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => Affine::IDENTITY,
        };
        m = m.after(&t);
        rest = &rest[open + close + 1..];
    }
    m
}

/// A length with its unit, in millimetres (`px` and plain numbers at 96 to the inch).
fn length_mm(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E')).unwrap_or(s.len());
    let v: f64 = s[..end].parse().ok()?;
    let mm = match s[end..].trim() {
        "mm" => 1.0,
        "cm" => 10.0,
        "in" => 25.4,
        "pt" => 25.4 / 72.0,
        "pc" => 25.4 / 6.0,
        "" | "px" => 25.4 / 96.0,
        _ => return None, // a percentage says nothing about size
    };
    Some(v * mm)
}

/// A circular arc (or, under a transform that is not a similarity, an
/// elliptical one) from `a` to `b` around `c`, counter-clockwise in the
/// drawing's own coordinates when `ccw`.
fn circular(b: &mut Builder, t: &Affine, c: [f64; 2], r: f64, a0: f64, a1: f64, ccw: bool) -> Result<(), String> {
    let (lo, hi) = if ccw { (a0, a1) } else { (a1, a0) };
    let mut hi = hi;
    while hi <= lo + 1e-12 {
        hi += std::f64::consts::TAU;
    }
    if t.similarity().is_some() {
        let p = |a: f64| t.at([c[0] + r * a.cos(), c[1] + r * a.sin()]);
        let (s, e) = (p(lo), p(hi));
        return if t.det() < 0.0 { b.arc(t.at(c), e, s) } else { b.arc(t.at(c), s, e) };
    }
    ellipse(b, t, c, [r, 0.0], 1.0, lo, hi)
}

/// The SVG elliptical arc from `p` to `q` (spec, appendix B.2.4).
#[allow(clippy::too_many_arguments)]
fn svg_arc(b: &mut Builder, t: &Affine, p: [f64; 2], mut rx: f64, mut ry: f64, phi: f64, large: bool, sweep: bool, q: [f64; 2]) -> Result<(), String> {
    if (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-12 {
        return Ok(());
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-12 || ry < 1e-12 {
        return b.line(t.at(p), t.at(q));
    }
    let (cos, sin) = (phi.cos(), phi.sin());
    let dx = (p[0] - q[0]) / 2.0;
    let dy = (p[1] - q[1]) / 2.0;
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let l = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if l > 1.0 {
        rx *= l.sqrt();
        ry *= l.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let k = if large == sweep { -1.0 } else { 1.0 } * (num / den).sqrt();
    let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let c = [cos * cx1 - sin * cy1 + (p[0] + q[0]) / 2.0, sin * cx1 + cos * cy1 + (p[1] + q[1]) / 2.0];
    let angle = |ux: f64, uy: f64| uy.atan2(ux);
    let a0 = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let a1 = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    if (rx - ry).abs() <= 1e-9 * rx {
        return circular(b, t, c, rx, a0 + phi, a1 + phi, sweep);
    }
    // Sweep 1 runs the angle up (counter-clockwise in the file's own numbers).
    let (lo, mut hi) = if sweep { (a0, a1) } else { (a1, a0) };
    while hi <= lo + 1e-12 {
        hi += std::f64::consts::TAU;
    }
    ellipse(b, t, c, [rx * cos, rx * sin], ry / rx, lo, hi)
}

fn path(b: &mut Builder, t: &Affine, d: &str) -> Result<(), String> {
    let mut n = Numbers { s: d.as_bytes(), at: 0 };
    let (mut cur, mut start) = ([0.0; 2], [0.0; 2]);
    // The last control point, for smooth curves (S and T).
    let mut last_cubic: Option<[f64; 2]> = None;
    let mut last_quad: Option<[f64; 2]> = None;
    let mut cmd = b'M';
    loop {
        n.skip();
        if n.at >= n.s.len() {
            break;
        }
        let c = n.s[n.at];
        if c.is_ascii_alphabetic() {
            cmd = c;
            n.at += 1;
        } else if !n.more() {
            return Err(format!("the path “{}…” does not read", &d[..d.len().min(40)]));
        }
        let rel = cmd.is_ascii_lowercase();
        let off = |p: [f64; 2], cur: [f64; 2]| if rel { [p[0] + cur[0], p[1] + cur[1]] } else { p };
        let pt = |n: &mut Numbers| -> Result<[f64; 2], String> { Ok([n.number()?, n.number()?]) };
        let (mut cubic, mut quad) = (None, None);
        match cmd.to_ascii_uppercase() {
            b'M' => {
                cur = off(pt(&mut n)?, cur);
                start = cur;
                // Further pairs after a move are lines.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let p = off(pt(&mut n)?, cur);
                b.line(t.at(cur), t.at(p))?;
                cur = p;
            }
            b'H' => {
                let x = n.number()? + if rel { cur[0] } else { 0.0 };
                b.line(t.at(cur), t.at([x, cur[1]]))?;
                cur[0] = x;
            }
            b'V' => {
                let y = n.number()? + if rel { cur[1] } else { 0.0 };
                b.line(t.at(cur), t.at([cur[0], y]))?;
                cur[1] = y;
            }
            b'C' | b'S' => {
                let c1 = if cmd.eq_ignore_ascii_case(&b'C') {
                    off(pt(&mut n)?, cur)
                } else {
                    last_cubic.map(|l| [2.0 * cur[0] - l[0], 2.0 * cur[1] - l[1]]).unwrap_or(cur)
                };
                let c2 = off(pt(&mut n)?, cur);
                let p = off(pt(&mut n)?, cur);
                b.bezier(t.at(cur), t.at(c1), t.at(c2), t.at(p))?;
                cubic = Some(c2);
                cur = p;
            }
            b'Q' | b'T' => {
                let q = if cmd.eq_ignore_ascii_case(&b'Q') {
                    off(pt(&mut n)?, cur)
                } else {
                    last_quad.map(|l| [2.0 * cur[0] - l[0], 2.0 * cur[1] - l[1]]).unwrap_or(cur)
                };
                let p = off(pt(&mut n)?, cur);
                let c1 = [cur[0] + 2.0 / 3.0 * (q[0] - cur[0]), cur[1] + 2.0 / 3.0 * (q[1] - cur[1])];
                let c2 = [p[0] + 2.0 / 3.0 * (q[0] - p[0]), p[1] + 2.0 / 3.0 * (q[1] - p[1])];
                b.bezier(t.at(cur), t.at(c1), t.at(c2), t.at(p))?;
                quad = Some(q);
                cur = p;
            }
            b'A' => {
                let (rx, ry, phi) = (n.number()?, n.number()?, n.number()?.to_radians());
                let (large, sweep) = (n.flag()?, n.flag()?);
                let p = off(pt(&mut n)?, cur);
                svg_arc(b, t, cur, rx, ry, phi, large, sweep, p)?;
                cur = p;
            }
            b'Z' => {
                b.line(t.at(cur), t.at(start))?;
                cur = start;
            }
            other => return Err(format!("unknown path command “{}”", other as char)),
        }
        last_cubic = cubic;
        last_quad = quad;
    }
    Ok(())
}

fn hidden(attrs: &[(&str, String)]) -> bool {
    let style = attr(attrs, "style").unwrap_or("").replace(' ', "");
    attr(attrs, "display") == Some("none") || style.contains("display:none") || attr(attrs, "visibility") == Some("hidden")
}

/// An SVG file's shapes, in millimetres, Y up.
pub fn read(data: &[u8]) -> Result<Builder, String> {
    let text = String::from_utf8_lossy(data);
    let mut b = Builder::default();
    // Transforms by depth; elements whose contents are skipped (defs, text…).
    let mut stack: Vec<Affine> = Vec::new();
    let mut skip_depth: Option<usize> = None;
    let mut seen_root = false;
    for e in events(&text)? {
        match e {
            Event::End { .. } => {
                stack.pop();
                if skip_depth.is_some_and(|d| stack.len() < d) {
                    skip_depth = None;
                }
            }
            Event::Start { name, attrs } => {
                let parent = stack.last().copied().unwrap_or(Affine::IDENTITY);
                let mut t = parent.after(&transform(attr(&attrs, "transform").unwrap_or("")));
                if name == "svg" && !seen_root {
                    seen_root = true;
                    // User units to millimetres, Y flipped up.
                    let vb = numbers(attr(&attrs, "viewBox").unwrap_or(""));
                    let (w, h) = (attr(&attrs, "width").and_then(length_mm), attr(&attrs, "height").and_then(length_mm));
                    let (sx, sy, ox, oy) = if vb.len() == 4 && vb[2] > 0.0 && vb[3] > 0.0 {
                        let sx = w.map(|w| w / vb[2]).unwrap_or(25.4 / 96.0);
                        let sy = h.map(|h| h / vb[3]).unwrap_or(sx);
                        (sx, sy, vb[0], vb[1])
                    } else {
                        (25.4 / 96.0, 25.4 / 96.0, 0.0, 0.0)
                    };
                    t = Affine([sx, 0.0, 0.0, -sy, -ox * sx, oy * sy]);
                }
                stack.push(t);
                if skip_depth.is_some() {
                    continue;
                }
                if matches!(name, "defs" | "clipPath" | "mask" | "symbol" | "pattern" | "marker" | "text" | "style" | "metadata" | "title" | "desc" | "image" | "foreignObject") || hidden(&attrs) {
                    skip_depth = Some(stack.len());
                    continue;
                }
                let f = |k: &str| attr(&attrs, k).and_then(|v| numbers(v).first().copied()).unwrap_or(0.0);
                match name {
                    "path" => path(&mut b, &t, attr(&attrs, "d").unwrap_or(""))?,
                    "line" => b.line(t.at([f("x1"), f("y1")]), t.at([f("x2"), f("y2")]))?,
                    "polyline" | "polygon" => {
                        let v = numbers(attr(&attrs, "points").unwrap_or(""));
                        let pts: Vec<[f64; 2]> = v.chunks_exact(2).map(|p| t.at([p[0], p[1]])).collect();
                        b.polyline(&pts, name == "polygon")?;
                    }
                    "circle" => {
                        let (c, r) = ([f("cx"), f("cy")], f("r"));
                        match t.similarity() {
                            Some(s) => b.circle(t.at(c), r * s)?,
                            None => ellipse(&mut b, &t, c, [r, 0.0], 1.0, 0.0, std::f64::consts::TAU)?,
                        }
                    }
                    "ellipse" => {
                        let (c, rx, ry) = ([f("cx"), f("cy")], f("rx"), f("ry"));
                        match t.similarity() {
                            Some(s) if (rx - ry).abs() < 1e-12 => b.circle(t.at(c), rx * s)?,
                            _ if rx > 0.0 && ry > 0.0 => ellipse(&mut b, &t, c, [rx, 0.0], ry / rx, 0.0, std::f64::consts::TAU)?,
                            _ => {}
                        }
                    }
                    "rect" => {
                        let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
                        if w <= 0.0 || h <= 0.0 {
                            continue;
                        }
                        let (rx, ry) = (attr(&attrs, "rx").map(|_| f("rx")), attr(&attrs, "ry").map(|_| f("ry")));
                        let rx = rx.or(ry).unwrap_or(0.0).min(w / 2.0);
                        let ry = ry.or(Some(rx)).unwrap_or(0.0).min(h / 2.0);
                        if rx <= 0.0 || ry <= 0.0 {
                            let pts = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]].map(|p| t.at(p));
                            b.polyline(&pts, true)?;
                        } else {
                            // Sides, then rounded corners (arcs when round, else Béziers).
                            let corner = |b: &mut Builder, c: [f64; 2], a: f64| -> Result<(), String> {
                                if (rx - ry).abs() < 1e-12 {
                                    circular(b, &t, c, rx, a, a + std::f64::consts::FRAC_PI_2, true)
                                } else {
                                    ellipse(b, &t, c, [rx, 0.0], ry / rx, a, a + std::f64::consts::FRAC_PI_2)
                                }
                            };
                            use std::f64::consts::PI;
                            b.line(t.at([x + rx, y]), t.at([x + w - rx, y]))?;
                            b.line(t.at([x + w, y + ry]), t.at([x + w, y + h - ry]))?;
                            b.line(t.at([x + w - rx, y + h]), t.at([x + rx, y + h]))?;
                            b.line(t.at([x, y + h - ry]), t.at([x, y + ry]))?;
                            corner(&mut b, [x + w - rx, y + ry], -PI / 2.0)?;
                            corner(&mut b, [x + w - rx, y + h - ry], 0.0)?;
                            corner(&mut b, [x + rx, y + h - ry], PI / 2.0)?;
                            corner(&mut b, [x + rx, y + ry], PI)?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if !seen_root {
        return Err("not an SVG drawing (no <svg> element)".into());
    }
    Ok(b)
}
