//! Board artwork plotted as KiCad's Plot dialog does: each fabrication layer
//! as its own SVG (black on white, the board edge drawn on every layer,
//! holes left open on copper and mask), every layer together in colour, and
//! a PDF with a page per layer plus the composite. The artwork is drawn from
//! the Gerber files themselves, read back by the independent reader, so the
//! plot shows exactly what the fab gets. Schematic sheets plot to SVG from
//! the same primitives as the editor and the PDF.

use crate::board::BoardView;
use crate::geom::P;
use crate::reader::{self, Aperture, GerberFile, Shape};
use crate::schematic_view::{Draw, SchematicView};
use std::fmt::Write;

#[derive(Clone, Debug)]
pub struct PlotLayer {
    /// "F_Cu", "B_Mask", "Edge_Cuts" (the Gerber file's layer part).
    pub file: String,
    /// "Top copper".
    pub label: String,
    /// Colour in the composite.
    pub color: &'static str,
    pub gerber: String,
}

fn label_of(f: &str) -> (&'static str, &'static str) {
    match f {
        "F_Cu" => ("Top copper", "#c83232"),
        "B_Cu" => ("Bottom copper", "#3c6ec8"),
        "In1_Cu" => ("Inner copper 1", "#c2a000"),
        "In2_Cu" => ("Inner copper 2", "#b44cc2"),
        "F_Mask" => ("Top solder mask openings", "#d27aa6"),
        "B_Mask" => ("Bottom solder mask openings", "#3aa69a"),
        "F_Paste" => ("Top paste", "#8a8a8a"),
        "B_Paste" => ("Bottom paste", "#6a6a6a"),
        "F_Silkscreen" => ("Top silkscreen", "#e0b000"),
        "B_Silkscreen" => ("Bottom silkscreen", "#6e5cc8"),
        "Edge_Cuts" => ("Board outline", "#202020"),
        _ => ("Layer", "#555555"),
    }
}

/// The fabrication layers that plot (not the drill files).
pub fn layers(view: &BoardView) -> Vec<PlotLayer> {
    let stem = crate::gerber::file_stem(&view.name);
    crate::gerber::files(view)
        .into_iter()
        .filter(|(n, _)| !n.ends_with(".drl"))
        .filter_map(|(n, g)| {
            let rest = n.strip_prefix(&format!("{stem}-"))?;
            let file = rest.split('.').next()?.to_string();
            let (label, color) = label_of(&file);
            Some(PlotLayer { file, label: label.into(), color, gerber: g })
        })
        .collect()
}

fn f(v: f64) -> String {
    super::num(v, 4)
}

/// Every object of a Gerber file as (dark, polygons) in board millimetres.
fn shapes(g: &GerberFile) -> Vec<(bool, Vec<Vec<P>>, Option<(f64, Vec<P>)>)> {
    let mut out = Vec::new();
    for o in &g.objects {
        if let Shape::Stroke { d, pts } = &o.shape {
            if let Some(Aperture::Circle(w)) = g.apertures.get(d) {
                out.push((o.dark, vec![], Some((*w, pts.clone()))));
                continue;
            }
        }
        let polys: Vec<Vec<P>> = g.outlines(o, 0.01).into_iter().filter(|(on, p)| *on && p.len() >= 3).map(|(_, p)| p).collect();
        out.push((o.dark, polys, None));
    }
    out
}

fn holes_of(view: &BoardView) -> Vec<(P, f64)> {
    view.holes.iter().map(|h| (h.at, h.size[0].min(h.size[1]))).collect()
}

fn svg_layer(out: &mut String, g: &GerberFile, color: &str, clear: &str) {
    for (dark, polys, stroke) in shapes(g) {
        let c = if dark { color } else { clear };
        if let Some((w, pts)) = stroke {
            let p: Vec<String> = pts.iter().map(|q| format!("{},{}", f(q[0]), f(q[1]))).collect();
            let _ = write!(out, r#"<polyline points="{}" fill="none" stroke="{c}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#, p.join(" "), f(w));
            continue;
        }
        let mut d = String::new();
        for p in polys {
            let _ = write!(d, "M{}Z", p.iter().map(|q| format!("{} {}", f(q[0]), f(q[1]))).collect::<Vec<_>>().join("L"));
        }
        if !d.is_empty() {
            let _ = write!(out, r#"<path d="{d}" fill="{c}" fill-rule="nonzero"/>"#);
        }
    }
}

fn outline_path(view: &BoardView) -> String {
    let mut d = String::new();
    for c in std::iter::once(&view.outline).chain(view.cutouts.iter()) {
        if c.len() >= 2 {
            let _ = write!(d, "M{}Z", c.iter().map(|q| format!("{} {}", f(q[0]), f(q[1]))).collect::<Vec<_>>().join("L"));
        }
    }
    d
}

/// One layer (black on white) or, with `None`, every layer in colour.
pub fn svg(view: &BoardView, layer: Option<&PlotLayer>) -> String {
    let m = 2.0;
    let b = view.bounds;
    let (w, h) = (b[2] - b[0] + 2.0 * m, b[3] - b[1] + 2.0 * m);
    let mut s = String::new();
    let _ = write!(s, r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="{} {} {} {}">
<title>{} rev {}{}</title>
<rect x="{}" y="{}" width="{}" height="{}" fill="#ffffff"/>
<g transform="translate(0 {}) scale(1 -1)">
"##,
        f(w), f(h), f(b[0] - m), f(-b[3] - m), f(w), f(h),
        super::xml::escape(&view.name), super::xml::escape(&view.revision), layer.map(|l| format!(" — {}", l.label)).unwrap_or_default(),
        f(b[0] - m), f(-b[3] - m), f(w), f(h),
        f(0.0)
    );
    // The y flip: board y up → SVG y down about y = 0, so the viewBox above spans −top..−bottom.
    match layer {
        Some(l) => {
            if let Ok(g) = reader::gerber(&l.gerber) {
                let _ = write!(s, r#"<g id="{}">"#, l.file);
                svg_layer(&mut s, &g, "#000000", "#ffffff");
                s.push_str("</g>");
            }
            if l.file.ends_with("_Cu") || l.file.ends_with("_Mask") {
                for (at, d) in holes_of(view) {
                    let _ = write!(s, r##"<circle cx="{}" cy="{}" r="{}" fill="#ffffff"/>"##, f(at[0]), f(at[1]), f(d / 2.0));
                }
            }
        }
        None => {
            // Bottom to top, translucent so lower layers show through.
            let order = ["B_Silkscreen", "B_Mask", "B_Cu", "In2_Cu", "In1_Cu", "F_Cu", "F_Mask", "F_Silkscreen"];
            let all = layers(view);
            for name in order {
                if let Some(l) = all.iter().find(|l| l.file == name) {
                    if let Ok(g) = reader::gerber(&l.gerber) {
                        let _ = write!(s, r#"<g id="{}" opacity="0.75">"#, l.file);
                        svg_layer(&mut s, &g, l.color, "#ffffff");
                        s.push_str("</g>");
                    }
                }
            }
            for (at, d) in holes_of(view) {
                let _ = write!(s, r##"<circle cx="{}" cy="{}" r="{}" fill="#ffffff" stroke="#202020" stroke-width="0.05"/>"##, f(at[0]), f(at[1]), f(d / 2.0));
            }
        }
    }
    let _ = write!(s, r##"<path d="{}" fill="none" stroke="#202020" stroke-width="0.1"/>"##, outline_path(view));
    s.push_str("</g>\n</svg>\n");
    s
}

// ---- PDF -----------------------------------------------------------------------------

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            c if (c as u32) < 0x20 => {}
            c if (c as u32) < 0x7f => o.push(c),
            _ => o.push('?'),
        }
    }
    o
}

/// Pages (width and height in points, content stream) as a PDF with Helvetica as /F1.
pub fn pdf_pages(title: &str, pages: &[(f64, f64, String)]) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = Vec::new();
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 5 + i * 2)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    objs.push(format!("<< /Title ({}) /Producer (ISE Studio) >>", esc(title)).into_bytes());
    for (i, (w, h, c)) in pages.iter().enumerate() {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 6 + i * 2).into_bytes());
        let mut st = format!("<< /Length {} >>\nstream\n", c.len()).into_bytes();
        st.extend_from_slice(c.as_bytes());
        st.extend_from_slice(b"\nendstream");
        objs.push(st);
    }
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn hex(c: &str) -> [f64; 3] {
    let v = u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(0);
    [((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0]
}

/// PDF drawing in board millimetres on a page whose lower-left is the board's lower-left minus a margin.
pub struct PdfCanvas {
    pub c: String,
    pub w: f64,
    pub h: f64,
}

impl PdfCanvas {
    pub fn new(bounds: [f64; 4], margin: f64, extra_bottom: f64) -> PdfCanvas {
        let k = 72.0 / 25.4;
        let w = (bounds[2] - bounds[0] + 2.0 * margin) * k;
        let h = (bounds[3] - bounds[1] + 2.0 * margin + extra_bottom) * k;
        let mut c = String::new();
        let _ = writeln!(c, "{k:.6} 0 0 {k:.6} {:.4} {:.4} cm", (margin - bounds[0]) * k, (margin + extra_bottom - bounds[1]) * k);
        c.push_str("1 J 1 j\n");
        PdfCanvas { c, w, h }
    }
    pub fn fill(&mut self, color: [f64; 3]) {
        let _ = writeln!(self.c, "{:.3} {:.3} {:.3} rg", color[0], color[1], color[2]);
    }
    pub fn stroke(&mut self, color: [f64; 3], width: f64) {
        let _ = writeln!(self.c, "{:.3} {:.3} {:.3} RG {:.4} w", color[0], color[1], color[2], width);
    }
    pub fn path(&mut self, p: &[P], close: bool) {
        for (i, q) in p.iter().enumerate() {
            let _ = writeln!(self.c, "{:.4} {:.4} {}", q[0], q[1], if i == 0 { "m" } else { "l" });
        }
        if close {
            self.c.push_str("h\n");
        }
    }
    pub fn circle(&mut self, c: P, r: f64) {
        let k = 0.552_284_75 * r;
        let (x, y) = (c[0], c[1]);
        let _ = writeln!(self.c, "{:.4} {:.4} m", x + r, y);
        let _ = writeln!(self.c, "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} c", x + r, y + k, x + k, y + r, x, y + r);
        let _ = writeln!(self.c, "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} c", x - k, y + r, x - r, y + k, x - r, y);
        let _ = writeln!(self.c, "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} c", x - r, y - k, x - k, y - r, x, y - r);
        let _ = writeln!(self.c, "{:.4} {:.4} {:.4} {:.4} {:.4} {:.4} c", x + k, y - r, x + r, y - k, x + r, y);
        self.c.push_str("h\n");
    }
    /// Text at a board point (mm), `size` mm high.
    pub fn text(&mut self, at: P, size: f64, s: &str) {
        let _ = writeln!(self.c, "BT /F1 {size:.3} Tf 1 0 0 1 {:.4} {:.4} Tm ({}) Tj ET", at[0], at[1], esc(s));
    }
}

fn pdf_layer(cv: &mut PdfCanvas, g: &GerberFile, color: [f64; 3]) {
    for (dark, polys, stroke) in shapes(g) {
        let c = if dark { color } else { [1.0, 1.0, 1.0] };
        if let Some((w, pts)) = stroke {
            cv.stroke(c, w);
            cv.path(&pts, false);
            cv.c.push_str("S\n");
            continue;
        }
        if polys.is_empty() {
            continue;
        }
        cv.fill(c);
        for p in &polys {
            cv.path(p, true);
        }
        cv.c.push_str("f\n");
    }
}

/// A page per layer and a composite page.
pub fn pdf(view: &BoardView) -> Vec<u8> {
    let all = layers(view);
    let mut pages = Vec::new();
    let title_size = 2.5;
    let page = |cv: &mut PdfCanvas, label: &str| {
        cv.stroke([0.12, 0.12, 0.12], 0.1);
        for c in std::iter::once(&view.outline).chain(view.cutouts.iter()) {
            cv.path(c, true);
            cv.c.push_str("S\n");
        }
        cv.fill([0.1, 0.1, 0.1]);
        cv.text([view.bounds[0], view.bounds[1] - 6.0], title_size, &format!("{} rev {} — {label}", view.name, view.revision));
    };
    let mut composite = PdfCanvas::new(view.bounds, 5.0, 6.0);
    for name in ["B_Silkscreen", "B_Mask", "B_Cu", "In2_Cu", "In1_Cu", "F_Cu", "F_Mask", "F_Silkscreen"] {
        if let Some(l) = all.iter().find(|l| l.file == name) {
            if let Ok(g) = reader::gerber(&l.gerber) {
                pdf_layer(&mut composite, &g, hex(l.color));
            }
        }
    }
    composite.fill([1.0, 1.0, 1.0]);
    for (at, d) in holes_of(view) {
        composite.circle(at, d / 2.0);
        composite.c.push_str("f\n");
    }
    page(&mut composite, "all layers");
    pages.push((composite.w, composite.h, composite.c));
    for l in &all {
        let mut cv = PdfCanvas::new(view.bounds, 5.0, 6.0);
        if let Ok(g) = reader::gerber(&l.gerber) {
            pdf_layer(&mut cv, &g, [0.0, 0.0, 0.0]);
        }
        if l.file.ends_with("_Cu") || l.file.ends_with("_Mask") {
            cv.fill([1.0, 1.0, 1.0]);
            for (at, d) in holes_of(view) {
                cv.circle(at, d / 2.0);
                cv.c.push_str("f\n");
            }
        }
        page(&mut cv, &l.label);
        pages.push((cv.w, cv.h, cv.c));
    }
    pdf_pages(&format!("{} board plot", view.name), &pages)
}

// ---- Schematic sheets -------------------------------------------------------------------

fn ink(name: &str) -> String {
    let c = crate::schematic_pdf::ink(name);
    format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

fn sch_draw(s: &mut String, d: &Draw) {
    let pts = |p: &[P]| p.iter().map(|q| format!("{},{}", f(q[0]), f(q[1]))).collect::<Vec<_>>().join(" ");
    let fill_of = |fill: &str, i: &str| match fill {
        "ink" => ink(i),
        "body" => ink("body_fill"),
        _ => "none".into(),
    };
    match d {
        Draw::Line { p, w, ink: i } => {
            let _ = write!(s, r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#, pts(p), ink(i), f(*w));
        }
        Draw::Poly { p, w, ink: i, fill } => {
            let _ = write!(s, r#"<polygon points="{}" fill="{}" stroke="{}" stroke-width="{}" stroke-linejoin="round"/>"#, pts(p), fill_of(fill, i), ink(i), f(*w));
        }
        Draw::Circle { c, r, w, ink: i, fill } => {
            let _ = write!(s, r#"<circle cx="{}" cy="{}" r="{}" fill="{}" stroke="{}" stroke-width="{}"/>"#, f(c[0]), f(c[1]), f(*r), fill_of(fill, i), ink(i), f(*w));
        }
        Draw::Text { at, s: t, size, angle, anchor, ink: i } => {
            if t.is_empty() {
                return;
            }
            let rot = if angle.abs() < 1.0 { String::new() } else { format!(r#" transform="rotate({} {} {})""#, f(-angle), f(at[0]), f(at[1])) };
            let _ = write!(s, r#"<text x="{}" y="{}" font-family="Helvetica, Arial, sans-serif" font-size="{}" text-anchor="{anchor}" dominant-baseline="central" fill="{}"{rot}>{}</text>"#, f(at[0]), f(at[1]), f(*size), ink(i), super::xml::escape(t));
        }
        Draw::Styled { p, w, color, fill, dash, closed } => {
            let tag = if *closed { "polygon" } else { "polyline" };
            let fill = fill.clone().unwrap_or_else(|| "none".into());
            let dash = if dash.is_empty() { String::new() } else { format!(r#" stroke-dasharray="{dash}""#) };
            let _ = write!(s, r#"<{tag} points="{}" fill="{}" stroke="{}" stroke-width="{}" stroke-linejoin="round"{dash}/>"#, pts(p), super::xml::escape(&fill), super::xml::escape(color), f(*w));
        }
        Draw::Rich { at, s: t, size, angle, anchor, color, bold, italic } => {
            if t.is_empty() {
                return;
            }
            let rot = if angle.abs() < 1.0 { String::new() } else { format!(r#" transform="rotate({} {} {})""#, f(-angle), f(at[0]), f(at[1])) };
            let weight = if *bold { r#" font-weight="bold""# } else { "" };
            let style = if *italic { r#" font-style="italic""# } else { "" };
            let _ = write!(s, r#"<text x="{}" y="{}" font-family="Helvetica, Arial, sans-serif" font-size="{}" text-anchor="{anchor}" dominant-baseline="central" fill="{}"{weight}{style}{rot}>{}</text>"#, f(at[0]), f(at[1]), f(*size), super::xml::escape(color), super::xml::escape(t));
        }
        Draw::Image { at, size, mime, data } => {
            let _ = write!(s, r#"<image x="{}" y="{}" width="{}" height="{}" href="data:{};base64,{}"/>"#, f(at[0]), f(at[1]), f(size[0]), f(size[1]), super::xml::escape(mime), data);
        }
    }
}

/// One sheet as SVG (sheet millimetres, y down).
pub fn schematic_svg(v: &SchematicView, sheet: &str) -> String {
    let Some(sh) = v.sheets.iter().find(|s| s.id == sheet) else { return String::new() };
    let [w, h] = sh.size;
    let mut s = String::new();
    let _ = write!(s, r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="0 0 {} {}">
<title>{} — {}</title>
<rect width="{}" height="{}" fill="#ffffff"/>
"##, f(w), f(h), f(w), f(h), super::xml::escape(&v.name), super::xml::escape(&sh.name), f(w), f(h));
    for d in sh.frame.iter().chain(sh.boxes_draw.iter()) {
        sch_draw(&mut s, d);
    }
    for t in v.texts.iter().filter(|t| t.sheet == sh.id) {
        for (i, line) in t.text.lines().enumerate() {
            sch_draw(&mut s, &Draw::Text { at: [t.at[0], t.at[1] + i as f64 * t.size * 1.35], s: line.to_string(), size: t.size, angle: 0.0, anchor: "start", ink: "note" });
        }
    }
    for sym in v.symbols.iter().filter(|x| x.sheet == sh.id) {
        for d in &sym.draw {
            sch_draw(&mut s, d);
        }
    }
    for wv in v.wires.iter().filter(|x| x.sheet == sh.id) {
        sch_draw(&mut s, &Draw::Line { p: wv.points.clone(), w: crate::schematic_view::W_WIRE, ink: "wire" });
    }
    for j in v.junctions.iter().filter(|x| x.sheet == sh.id) {
        sch_draw(&mut s, &Draw::Circle { c: j.at, r: 0.45, w: 0.05, ink: "junction", fill: "ink" });
    }
    for nc in v.no_connects.iter().filter(|x| x.sheet == sh.id) {
        let a = 0.7;
        sch_draw(&mut s, &Draw::Line { p: vec![[nc.at[0] - a, nc.at[1] - a], [nc.at[0] + a, nc.at[1] + a]], w: 0.2, ink: "no_connect" });
        sch_draw(&mut s, &Draw::Line { p: vec![[nc.at[0] - a, nc.at[1] + a], [nc.at[0] + a, nc.at[1] - a]], w: 0.2, ink: "no_connect" });
    }
    for l in v.labels.iter().filter(|x| x.sheet == sh.id) {
        for d in &l.draw {
            sch_draw(&mut s, d);
        }
    }
    s.push_str("</svg>\n");
    s
}
