//! The drill map (every hole marked with its tool's symbol on the board
//! outline, with a legend of sizes and counts) as Gerber, SVG and PDF, and
//! the drill report (layers, then each drill file's tools, sizes, hole and
//! slot counts), as KiCad's drill generator gives them.

use crate::board::BoardView;
use crate::geom::P;
use std::fmt::Write;

/// One drill tool: diameter, plated, holes, slots.
pub struct Tool {
    pub diameter: f64,
    pub plated: bool,
    pub holes: Vec<P>,
    pub slots: usize,
}

pub fn tools(v: &BoardView) -> Vec<Tool> {
    let mut out: Vec<Tool> = Vec::new();
    for h in &v.holes {
        let d = (h.size[0].min(h.size[1]) * 1000.0).round() / 1000.0;
        let slot = (h.size[0] - h.size[1]).abs() > 1e-9;
        match out.iter_mut().find(|t| (t.diameter - d).abs() < 1e-9 && t.plated == h.plated) {
            Some(t) => {
                t.holes.push(h.at);
                t.slots += slot as usize;
            }
            None => out.push(Tool { diameter: d, plated: h.plated, holes: vec![h.at], slots: slot as usize }),
        }
    }
    out.sort_by(|a, b| b.plated.cmp(&a.plated).then(a.diameter.total_cmp(&b.diameter)));
    out
}

/// The mark for tool `i`, `s` mm across, at `c`: polylines.
fn symbol(i: usize, c: P, s: f64) -> Vec<Vec<P>> {
    let r = s / 2.0;
    let pt = |dx: f64, dy: f64| [c[0] + dx * r, c[1] + dy * r];
    let circle = || super::build::arc(c, r, 0.0, 360.0);
    let plus = || vec![vec![pt(-1.0, 0.0), pt(1.0, 0.0)], vec![pt(0.0, -1.0), pt(0.0, 1.0)]];
    let cross = || vec![vec![pt(-0.7, -0.7), pt(0.7, 0.7)], vec![pt(-0.7, 0.7), pt(0.7, -0.7)]];
    let square = || vec![vec![pt(-0.8, -0.8), pt(0.8, -0.8), pt(0.8, 0.8), pt(-0.8, 0.8), pt(-0.8, -0.8)]];
    let diamond = || vec![vec![pt(0.0, -1.0), pt(1.0, 0.0), pt(0.0, 1.0), pt(-1.0, 0.0), pt(0.0, -1.0)]];
    let tri = || vec![vec![pt(0.0, 1.0), pt(0.87, -0.5), pt(-0.87, -0.5), pt(0.0, 1.0)]];
    let mut v = match i % 8 {
        0 => [vec![circle()], plus()].concat(),
        1 => cross(),
        2 => square(),
        3 => diamond(),
        4 => tri(),
        5 => [vec![circle()], cross()].concat(),
        6 => [square(), cross()].concat(),
        _ => [diamond(), plus()].concat(),
    };
    // Past eight tools the marks repeat with a dot to tell them apart.
    if i >= 8 {
        v.push(super::build::arc(c, r * 0.25, 0.0, 360.0));
    }
    v
}

fn legend_text(t: &Tool) -> String {
    format!(
        "{:.2}mm ({:.3}\") {} — {} hole{}{}",
        t.diameter,
        t.diameter / 25.4,
        if t.plated { "PTH" } else { "NPTH" },
        t.holes.len(),
        if t.holes.len() == 1 { "" } else { "s" },
        if t.slots > 0 { format!(", {} slot{}", t.slots, if t.slots == 1 { "" } else { "s" }) } else { String::new() }
    )
}

/// Every stroke of the map (board outline, marks, legend) as (width, polyline).
fn strokes(v: &BoardView) -> Vec<(f64, Vec<P>)> {
    let mut out: Vec<(f64, Vec<P>)> = Vec::new();
    for c in std::iter::once(&v.outline).chain(v.cutouts.iter()) {
        let mut p = c.clone();
        p.push(c[0]);
        out.push((0.1, p));
    }
    let ts = tools(v);
    let size = 1.2;
    for (i, t) in ts.iter().enumerate() {
        for h in &t.holes {
            for l in symbol(i, *h, size) {
                out.push((0.1, l));
            }
        }
    }
    // Legend under the board.
    let x0 = v.bounds[0];
    let mut y = v.bounds[1] - 4.0;
    let th = 1.2;
    for (i, t) in ts.iter().enumerate() {
        for l in symbol(i, [x0 + 1.0, y], size) {
            out.push((0.1, l));
        }
        let text = legend_text(t).replace('—', "-");
        let w = crate::font::width(&text, th);
        for l in crate::font::strokes(&text, [x0 + 3.0 + w / 2.0, y], th, false) {
            out.push((0.12, l));
        }
        y -= 2.5;
    }
    out
}

fn legend_bottom(v: &BoardView) -> f64 {
    v.bounds[1] - 4.0 - 2.5 * tools(v).len() as f64
}

pub fn map_gerber(v: &BoardView) -> String {
    let mut g = crate::gerber::Gerber::new(v, "Drillmap", false);
    for (w, l) in strokes(v) {
        g.polyline(w, "", &l, &[]);
    }
    g.finish()
}

pub fn map_svg(v: &BoardView) -> String {
    let m = 3.0;
    let b = [v.bounds[0] - m, legend_bottom(v) - m, v.bounds[2].max(v.bounds[0] + 60.0) + m, v.bounds[3] + m];
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let mut s = String::new();
    let _ = write!(
        s,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"{} {} {} {}\">\n<title>{} drill map</title>\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n<g transform=\"scale(1 -1)\" fill=\"none\" stroke=\"#000000\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n",
        super::num(w, 3),
        super::num(h, 3),
        super::num(b[0], 3),
        super::num(-b[3], 3),
        super::num(w, 3),
        super::num(h, 3),
        super::xml::escape(&v.name),
        super::num(b[0], 3),
        super::num(-b[3], 3),
        super::num(w, 3),
        super::num(h, 3)
    );
    for (sw, l) in strokes(v) {
        let p: Vec<String> = l.iter().map(|q| format!("{},{}", super::num(q[0], 4), super::num(q[1], 4))).collect();
        let _ = write!(s, "<polyline points=\"{}\" stroke-width=\"{}\"/>", p.join(" "), super::num(sw, 3));
    }
    s.push_str("\n</g>\n</svg>\n");
    s
}

pub fn map_pdf(v: &BoardView) -> Vec<u8> {
    let bottom = legend_bottom(v);
    let b = [v.bounds[0], bottom, v.bounds[2].max(v.bounds[0] + 60.0), v.bounds[3]];
    let mut cv = super::plot::PdfCanvas::new(b, 5.0, 0.0);
    for (w, l) in strokes(v) {
        cv.stroke([0.0, 0.0, 0.0], w);
        cv.path(&l, false);
        cv.c.push_str("S\n");
    }
    cv.fill([0.0, 0.0, 0.0]);
    cv.text([v.bounds[0], v.bounds[3] + 2.0], 2.5, &format!("{} rev {} - drill map", v.name, v.revision));
    super::plot::pdf_pages(&format!("{} drill map", v.name), &[(cv.w, cv.h, cv.c)])
}

pub fn report(v: &BoardView) -> String {
    let stem = crate::gerber::file_stem(&v.name);
    let mut s = String::new();
    let _ = writeln!(s, "Drill report for {}", v.name);
    let _ = writeln!(s, "Created on {} by Forma", super::now_iso());
    let _ = writeln!(s, "\nCopper Layer Stackup:\n    =============================================================");
    for (i, l) in v.layers.iter().enumerate() {
        let k = match l.as_str() {
            "top" => "F.Cu",
            "bottom" => "B.Cu",
            "in1" => "In1.Cu",
            "in2" => "In2.Cu",
            x => x,
        };
        let _ = writeln!(s, "    L{} :  {k:<20} {}", i + 1, if i == 0 { "front" } else if i + 1 == v.layers.len() { "back" } else { "in" });
    }
    let ts = tools(v);
    for plated in [true, false] {
        let mine: Vec<(usize, &Tool)> = ts.iter().enumerate().filter(|(_, t)| t.plated == plated).collect();
        let _ = writeln!(s, "\n\nDrill file '{stem}-{}.drl' contains", if plated { "PTH" } else { "NPTH" });
        let _ = writeln!(s, "    {} through holes:\n    =============================================================", if plated { "plated" } else { "unplated" });
        let mut total = 0;
        let mut slots = 0;
        for (k, (_, t)) in mine.iter().enumerate() {
            let _ = writeln!(s, "    T{}  {:.3}mm  {:.4}\"  ({} hole{}){}", k + 1, t.diameter, t.diameter / 25.4, t.holes.len(), if t.holes.len() == 1 { "" } else { "s" }, if t.slots > 0 { format!("  (with {} slot{})", t.slots, if t.slots == 1 { "" } else { "s" }) } else { String::new() });
            total += t.holes.len();
            slots += t.slots;
        }
        let _ = writeln!(s, "\n    Total {} holes count {total}{}", if plated { "plated" } else { "unplated" }, if slots > 0 { format!(" ({slots} slots)") } else { String::new() });
    }
    s
}
