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
        "bus" => 0x1f4fbf,
        "directive" => 0x2f7d8a,
        "field" => 0x4f7f7f,
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

/// "#rrggbb" or "#rgb" as sRGB 0–1 (dark grey when unreadable).
pub fn parse_color(c: &str) -> [f64; 3] {
    let h = c.trim().trim_start_matches('#');
    let full: String = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
    match u32::from_str_radix(&full, 16) {
        Ok(v) if full.len() == 6 => [((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0],
        _ => [0.13, 0.13, 0.13],
    }
}

fn dash(out: &mut String, d: &str, w: f64) {
    match crate::schematic_export::dash_array(d, w) {
        Some(a) => {
            let _ = writeln!(out, "[{a}] 0 d");
        }
        None => out.push_str("[] 0 d\n"),
    }
}

#[allow(clippy::too_many_arguments)]
fn text_at(out: &mut String, at: [f64; 2], s: &str, size: f64, angle: f64, anchor: &str, font: &str, color: [f64; 3]) {
    let w = text_width(s, size);
    let shift = match anchor {
        "end" => -w,
        "middle" => -w / 2.0,
        _ => 0.0,
    };
    let base = 0.35 * size;
    // Text runs along `u`; glyph up is `n` (on the sheet, y down).
    let (u, n) = if angle.abs() < 1.0 { ([1.0, 0.0], [0.0, -1.0]) } else { ([0.0, -1.0], [-1.0, 0.0]) };
    let x = at[0] + u[0] * shift - n[0] * base;
    let y = at[1] + u[1] * shift - n[1] * base;
    out.push_str(&rgb(color, false));
    let _ = writeln!(out, "BT /{font} {size:.3} Tf {} {} {} {} {x:.3} {y:.3} Tm ({}) Tj ET", u[0], u[1], n[0], n[1], esc(s));
}

/// A picture ready for the PDF: an image XObject's dictionary entries and data.
pub struct PdfImage {
    pub dict: String,
    pub data: Vec<u8>,
}

/// A PNG (8-bit grey, RGB or palette, not interlaced, no alpha) or a JPEG as a PDF image.
pub fn pdf_image(mime: &str, b64: &str) -> Option<PdfImage> {
    let b = crate::schematic_graphics::base64_decode(b64)?;
    if mime == "image/jpeg" {
        let (_, px) = crate::schematic_graphics::image_info(&b)?;
        // Components from the frame header.
        let mut i = 2;
        let mut comps = 3;
        while i + 9 < b.len() {
            if b[i] == 0xff && (0xc0..=0xcf).contains(&b[i + 1]) && ![0xc4, 0xc8, 0xcc].contains(&b[i + 1]) {
                comps = b[i + 9];
                break;
            }
            if b[i] == 0xff && b[i + 1] != 0xd8 && b[i + 1] != 0x01 && !(0xd0..=0xd7).contains(&b[i + 1]) {
                i += 2 + u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            } else {
                i += 1;
            }
        }
        let cs = match comps {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        return Some(PdfImage { dict: format!("/Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /DCTDecode{}", px[0], px[1], if comps == 4 { " /Decode [1 0 1 0 1 0 1 0]" } else { "" }), data: b });
    }
    // PNG chunks.
    if b.len() < 33 || &b[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    let (depth, ctype, interlace) = (b[24], b[25], b[28]);
    if depth != 8 || interlace != 0 || ![0, 2, 3].contains(&ctype) {
        return None;
    }
    let mut i = 8;
    let mut idat = Vec::new();
    let mut plte = Vec::new();
    while i + 8 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let kind = &b[i + 4..i + 8];
        let body = b.get(i + 8..i + 8 + len)?;
        match kind {
            b"IDAT" => idat.extend_from_slice(body),
            b"PLTE" => plte = body.to_vec(),
            b"IEND" => break,
            _ => {}
        }
        i += 12 + len;
    }
    let (cs, colors) = match ctype {
        0 => ("/DeviceGray".to_string(), 1),
        2 => ("/DeviceRGB".to_string(), 3),
        _ => {
            if plte.len() < 3 {
                return None;
            }
            let hexs: String = plte.iter().map(|x| format!("{x:02x}")).collect();
            (format!("[/Indexed /DeviceRGB {} <{hexs}>]", plte.len() / 3 - 1), 1)
        }
    };
    let _ = h;
    Some(PdfImage { dict: format!("/Width {w} /Height {h} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors {colors} /BitsPerComponent 8 /Columns {w} >>"), data: idat })
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
        Draw::Rich { at, s, size, angle, anchor, color, bold, italic } => {
            if s.is_empty() {
                return;
            }
            let font = match (*bold, *italic) {
                (true, true) => "F4",
                (true, false) => "F2",
                (false, true) => "F3",
                _ => "F1",
            };
            text_at(out, *at, s, *size, *angle, anchor, font, parse_color(color));
        }
        Draw::Styled { p, w, color, fill, dash: d, closed } => {
            let stroke = *w > 0.0;
            if stroke {
                out.push_str(&rgb(parse_color(color), true));
                let _ = writeln!(out, "{w:.3} w");
                dash(out, d, *w);
            }
            if let Some(f) = fill {
                out.push_str(&rgb(parse_color(f), false));
            }
            path(out, p, *closed);
            out.push_str(match (stroke, fill.is_some() && *closed) {
                (true, true) => "B\n",
                (false, true) => "f\n",
                (true, false) => "S\n",
                (false, false) => "n\n",
            });
            if stroke && !d.is_empty() {
                out.push_str("[] 0 d\n");
            }
        }
        Draw::Image { .. } => {}
    }
}

/// Every sheet as a page.
pub fn pdf(v: &SchematicView) -> Vec<u8> {
    let k = 72.0 / 25.4;
    let mut pages: Vec<(f64, f64, String)> = Vec::new();
    // Pictures: one XObject each (those that cannot be embedded draw as a framed box).
    let mut images: Vec<PdfImage> = Vec::new();
    let mut page_images: Vec<Vec<usize>> = Vec::new();
    for sh in &v.sheets {
        let mut c = String::new();
        let [w, h] = sh.size;
        let mut used = Vec::new();
        let _ = writeln!(c, "{k:.6} 0 0 {:.6} 0 {:.3} cm", -k, h * k);
        c.push_str("1 J 1 j\n");
        for d in crate::schematic_export::sheet_draws(v, &sh.id) {
            if let Draw::Image { at, size, mime, data } = &d {
                match pdf_image(mime, data) {
                    Some(img) => {
                        images.push(img);
                        let n = images.len() - 1;
                        used.push(n);
                        let _ = writeln!(c, "q {:.3} 0 0 {:.3} {:.3} {:.3} cm /Im{n} Do Q", size[0], -size[1], at[0], at[1] + size[1]);
                    }
                    None => {
                        let r = vec![*at, [at[0] + size[0], at[1]], [at[0] + size[0], at[1] + size[1]], [at[0], at[1] + size[1]]];
                        draw(&mut c, &Draw::Poly { p: r, w: 0.2, ink: "note", fill: "none" });
                        draw(&mut c, &Draw::Text { at: [at[0] + size[0] / 2.0, at[1] + size[1] / 2.0], s: "picture".into(), size: 2.0, angle: 0.0, anchor: "middle", ink: "note" });
                    }
                }
                continue;
            }
            draw(&mut c, &d);
        }
        pages.push((w * k, h * k, c));
        page_images.push(used);
    }
    // Objects: 1 catalog, 2 pages, 3 font, 4 info, then a page and its content per sheet.
    let mut objs: Vec<Vec<u8>> = Vec::new();
    objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 5 + i * 2)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes());
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    objs.push(format!("<< /Title ({}) /Producer (ISE Studio) >>", esc(&v.name)).into_bytes());
    // Fonts F2–F4 and the pictures come after the pages.
    let after = 5 + pages.len() * 2;
    let fonts = format!("/F1 3 0 R /F2 {} 0 R /F3 {} 0 R /F4 {} 0 R", after, after + 1, after + 2);
    for (i, (w, h, c)) in pages.iter().enumerate() {
        let xo: String = page_images[i].iter().map(|n| format!("/Im{n} {} 0 R ", after + 3 + n)).collect();
        let xobj = if xo.is_empty() { String::new() } else { format!(" /XObject << {xo}>>") };
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /Font << {fonts} >>{xobj} >> /Contents {} 0 R >>", 6 + i * 2).into_bytes());
        let mut s = format!("<< /Length {} >>\nstream\n", c.len()).into_bytes();
        s.extend_from_slice(c.as_bytes());
        s.extend_from_slice(b"\nendstream");
        objs.push(s);
    }
    for f in ["Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique"] {
        objs.push(format!("<< /Type /Font /Subtype /Type1 /BaseFont /{f} /Encoding /WinAnsiEncoding >>").into_bytes());
    }
    for img in &images {
        let mut s = format!("<< /Type /XObject /Subtype /Image {} /Length {} >>\nstream\n", img.dict, img.data.len()).into_bytes();
        s.extend_from_slice(&img.data);
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
