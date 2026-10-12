//! The schematic as a PDF: one page per sheet at its paper size, vector
//! lines and Helvetica text (a standard PDF font, nothing embedded), drawn
//! from the same primitives as the editor so print and screen match.

use crate::schematic_view::{Draw, SchematicView};
use crate::symbols::text_width;
use std::fmt::Write;

/// Colour of an ink role on paper (sRGB 0–1). The web editor uses the same values.
pub fn ink(name: &str) -> [f64; 3] {
    let hex = match name {
        "body" | "pin" => 0x8a1f1f,
        "body_fill" => 0xfdf6dc,
        "pin_number" => 0x9b4a3a,
        "pin_name" => 0x0d6b6b,
        "reference" | "value" => 0x0d6b6b,
        "wire" | "junction" => 0x1b7f3d,
        "label" => 0x1d2430,
        "global" => 0x7a3aa8,
        "hierarchical" => 0xa05a00,
        "power" => 0x8a1f1f,
        "no_connect" => 0x2a5bd7,
        "note" => 0x3c4a63,
        "frame" => 0xa04a4a,
        "frame_text" => 0x7e2f2f,
        "frame_label" => 0xb07a7a,
        "sheet" => 0x8a5a00,
        "dnp" | "error" => 0xd03030,
        _ => 0x222222,
    };
    [((hex >> 16) & 255) as f64 / 255.0, ((hex >> 8) & 255) as f64 / 255.0, (hex & 255) as f64 / 255.0]
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '(' | ')' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            '—' | '–' | '−' => o.push('-'),
            'Ω' => o.push_str("Ohm"),
            c if (c as u32) < 0x20 => {}
            c if (c as u32) < 0x7f => o.push(c),
            c if (c as u32) <= 0xff => {
                let _ = write!(o, "\\{:03o}", c as u32);
            }
            _ => o.push('?'),
        }
    }
    o
}

fn rgb(c: [f64; 3], stroke: bool) -> String {
    format!("{:.3} {:.3} {:.3} {}\n", c[0], c[1], c[2], if stroke { "RG" } else { "rg" })
}

fn path(out: &mut String, p: &[[f64; 2]], close: bool) {
    for (i, q) in p.iter().enumerate() {
        let _ = writeln!(out, "{:.3} {:.3} {}", q[0], q[1], if i == 0 { "m" } else { "l" });
    }
    if close {
        out.push_str("h\n");
    }
}

fn circle(out: &mut String, c: [f64; 2], r: f64) {
    let k = 0.552_284_75 * r;
    let (x, y) = (c[0], c[1]);
    let _ = writeln!(out, "{:.3} {:.3} m", x + r, y);
    let _ = writeln!(out, "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c", x + r, y + k, x + k, y + r, x, y + r);
    let _ = writeln!(out, "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c", x - k, y + r, x - r, y + k, x - r, y);
    let _ = writeln!(out, "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c", x - r, y - k, x - k, y - r, x, y - r);
    let _ = writeln!(out, "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c", x + k, y - r, x + r, y - k, x + r, y);
    out.push_str("h\n");
}

fn draw(out: &mut String, d: &Draw) {
    match d {
        Draw::Line { p, w, ink: i } => {
            out.push_str(&rgb(ink(i), true));
            let _ = writeln!(out, "{w:.3} w");
            path(out, p, false);
            out.push_str("S\n");
        }
        Draw::Poly { p, w, ink: i, fill } => {
            out.push_str(&rgb(ink(i), true));
            let _ = writeln!(out, "{w:.3} w");
            match *fill {
                "ink" => out.push_str(&rgb(ink(i), false)),
                "body" => out.push_str(&rgb(ink("body_fill"), false)),
                _ => {}
            }
            path(out, p, true);
            out.push_str(if *fill == "none" { "S\n" } else { "B\n" });
        }
        Draw::Circle { c, r, w, ink: i, fill } => {
            out.push_str(&rgb(ink(i), true));
            let _ = writeln!(out, "{w:.3} w");
            match *fill {
                "ink" => out.push_str(&rgb(ink(i), false)),
                "body" => out.push_str(&rgb(ink("body_fill"), false)),
                _ => {}
            }
            circle(out, *c, *r);
            out.push_str(if *fill == "none" { "S\n" } else { "B\n" });
        }
        Draw::Text { at, s, size, angle, anchor, ink: i } => {
            if s.is_empty() {
                return;
            }
            let w = text_width(s, *size);
            let shift = match *anchor {
                "end" => -w,
                "middle" => -w / 2.0,
                _ => 0.0,
            };
            let base = 0.35 * size;
            // Text runs along `u`; glyph up is `n` (on the sheet, y down).
            let (u, n) = if angle.abs() < 1.0 { ([1.0, 0.0], [0.0, -1.0]) } else { ([0.0, -1.0], [-1.0, 0.0]) };
            let x = at[0] + u[0] * shift - n[0] * base;
            let y = at[1] + u[1] * shift - n[1] * base;
            out.push_str(&rgb(ink(i), false));
            let _ = writeln!(out, "BT /F1 {size:.3} Tf {} {} {} {} {x:.3} {y:.3} Tm ({}) Tj ET", u[0], u[1], n[0], n[1], esc(s));
        }
    }
}

/// Every sheet as a page.
pub fn pdf(v: &SchematicView) -> Vec<u8> {
    let k = 72.0 / 25.4;
    let mut pages: Vec<(f64, f64, String)> = Vec::new();
    for sh in &v.sheets {
        let mut c = String::new();
        let [w, h] = sh.size;
        let _ = writeln!(c, "{k:.6} 0 0 {:.6} 0 {:.3} cm", -k, h * k);
        c.push_str("1 J 1 j\n");
        for d in sh.frame.iter().chain(sh.boxes_draw.iter()) {
            draw(&mut c, d);
        }
        for t in v.texts.iter().filter(|t| t.sheet == sh.id) {
            for (i, line) in t.text.lines().enumerate() {
                draw(&mut c, &Draw::Text { at: [t.at[0], t.at[1] + i as f64 * t.size * 1.35], s: line.to_string(), size: t.size, angle: 0.0, anchor: "start", ink: "note" });
            }
        }
        for s in v.symbols.iter().filter(|s| s.sheet == sh.id) {
            for d in &s.draw {
                draw(&mut c, d);
            }
        }
        for wv in v.wires.iter().filter(|x| x.sheet == sh.id) {
            draw(&mut c, &Draw::Line { p: wv.points.clone(), w: crate::schematic_view::W_WIRE, ink: "wire" });
        }
        for j in v.junctions.iter().filter(|x| x.sheet == sh.id) {
            draw(&mut c, &Draw::Circle { c: j.at, r: 0.45, w: 0.05, ink: "junction", fill: "ink" });
        }
        for n in v.no_connects.iter().filter(|x| x.sheet == sh.id) {
            let a = 0.7;
            draw(&mut c, &Draw::Line { p: vec![[n.at[0] - a, n.at[1] - a], [n.at[0] + a, n.at[1] + a]], w: 0.2, ink: "no_connect" });
            draw(&mut c, &Draw::Line { p: vec![[n.at[0] - a, n.at[1] + a], [n.at[0] + a, n.at[1] - a]], w: 0.2, ink: "no_connect" });
        }
        for l in v.labels.iter().filter(|x| x.sheet == sh.id) {
            for d in &l.draw {
                draw(&mut c, d);
            }
        }
        pages.push((w * k, h * k, c));
    }
    // Objects: 1 catalog, 2 pages, 3 font, 4 info, then a page and its content per sheet.
    let mut objs: Vec<Vec<u8>> = Vec::new();
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 5 + i * 2)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    objs.push(format!("<< /Title ({}) /Producer (Forma) >>", esc(&v.name)).into_bytes());
    for (i, (w, h, c)) in pages.iter().enumerate() {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 6 + i * 2).into_bytes());
        let mut s = format!("<< /Length {} >>\nstream\n", c.len()).into_bytes();
        s.extend_from_slice(c.as_bytes());
        s.extend_from_slice(b"\nendstream");
        objs.push(s);
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
