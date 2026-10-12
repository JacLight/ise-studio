//! The sheet's furniture: the title block (ISO 7200 fields), the line of
//! general requirements over it (units, general tolerances, the general
//! surface texture), numbered notes, and the revision table.

use super::*;

/// The revision table's width (sheet mm).
pub(super) const REV_W: f64 = 150.0;
const REV_ROW: f64 = 6.0;
const NOTE_SIZE: f64 = 2.5;
const NOTE_PITCH: f64 = 4.4;

/// The size that fits `text` in `room` mm, from `size` down to 1.8 mm (the smallest lettering allowed).
fn fitted(text: &str, size: f64, room: f64, bold: bool) -> f64 {
    let w = text_width(&Text { at: [0.0; 2], size, text: text.into(), anchor: Anchor::Start, angle: 0.0, bold });
    if w <= room || w <= 0.0 { size } else { (size * room / w).max(1.8) }
}

/// Cuts text to `room` mm at `size`, with an ellipsis, when even the smallest lettering is too wide.
fn clipped(text: &str, size: f64, room: f64) -> String {
    let width = |s: &str| text_width(&Text { at: [0.0; 2], size, text: s.into(), anchor: Anchor::Start, angle: 0.0, bold: false });
    if width(text) <= room {
        return text.to_string();
    }
    let mut out: String = text.to_string();
    while !out.is_empty() && width(&format!("{out}…")) > room {
        out.pop();
    }
    format!("{}…", out.trim_end())
}

/// The title block, lower right inside the frame. Rows from the bottom: the
/// title, drawing number and revision; material, finish, mass and scale; who
/// drew, checked and approved it, with the projection symbol; who owns it,
/// the date and the sheet.
pub(super) fn title_block(d: &mut Drawing, at: [f64; 2], size: [f64; 2], t: &Title, options: &Options) {
    let f = &options.title_block;
    let [x, y] = at;
    let [w, h] = size;
    let line = |d: &mut Drawing, a: [f64; 2], b: [f64; 2]| d.strokes.push(Stroke { pen: Pen::Thin, points: vec![a, b] });
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h], [x, y]] });
    let rows = [y, y + 12.0, y + 20.0, y + 28.0, y + h];
    for r in &rows[1..4] {
        line(d, [x, *r], [x + w, *r]);
    }
    // A cell: its label small at the top, its value under it, made to fit.
    let cell = |d: &mut Drawing, x0: f64, x1: f64, row: usize, label: &str, value: &str, size: f64, bold: bool| {
        let (y0, y1) = (rows[row], rows[row + 1]);
        d.texts.push(Text { at: [x0 + 1.2, y1 - 2.4], size: 1.8, text: label.to_uppercase(), anchor: Anchor::Start, angle: 0.0, bold: false });
        let room = x1 - x0 - 2.4;
        let k = fitted(value, size, room, bold);
        let text = if k <= 1.8 + 1e-9 { clipped(value, 1.8, room) } else { value.to_string() };
        d.texts.push(Text { at: [x0 + 1.2, y0 + 1.6], size: k, text, anchor: Anchor::Start, angle: 0.0, bold });
    };
    let or = |o: &Option<String>, v: &str| o.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| v.to_string());
    let col = |k: f64| x + w * k;
    // Row 0: title, drawing number, revision.
    for k in [0.58, 0.87] {
        line(d, [col(k), rows[0]], [col(k), rows[1]]);
    }
    cell(d, col(0.0), col(0.58), 0, "Title", &or(&f.title, &t.title), 5.0, true);
    cell(d, col(0.58), col(0.87), 0, "Drawing no.", &or(&f.number, &t.number), 3.5, false);
    let rev = or(&f.revision, options.revisions.last().map(|r| r.rev.as_str()).unwrap_or("—"));
    cell(d, col(0.87), col(1.0), 0, "Rev", &rev, 3.5, true);
    // Row 1: material, finish, mass, scale.
    for k in [0.40, 0.70, 0.85] {
        line(d, [col(k), rows[1]], [col(k), rows[2]]);
    }
    cell(d, col(0.0), col(0.40), 1, "Material", &or(&f.material, &t.material), 2.5, false);
    cell(d, col(0.40), col(0.70), 1, "Finish", &or(&f.finish, "—"), 2.5, false);
    cell(d, col(0.70), col(0.85), 1, "Mass", &t.mass, 2.5, false);
    let scale = d.scale_text.clone();
    cell(d, col(0.85), col(1.0), 1, "Scale", &scale, 2.5, false);
    // Rows 2 and 3: people and dates; the projection symbol spans both at the right.
    for k in [0.40, 0.55, 0.70, 0.85] {
        line(d, [col(k), rows[2]], [col(k), rows[3]]);
    }
    for k in [0.40, 0.55, 0.85] {
        line(d, [col(k), rows[3]], [col(k), rows[4]]);
    }
    cell(d, col(0.0), col(0.40), 2, "General tolerances", &or(&f.tolerance, "ISO 2768-m"), 2.5, false);
    cell(d, col(0.40), col(0.55), 2, "Drawn", &or(&f.drawn_by, &t.drawn_by), 2.5, false);
    cell(d, col(0.55), col(0.70), 2, "Checked", &or(&f.checked_by, "—"), 2.5, false);
    cell(d, col(0.70), col(0.85), 2, "Approved", &or(&f.approved_by, "—"), 2.5, false);
    cell(d, col(0.0), col(0.40), 3, "Owner", &or(&f.company, "—"), 2.5, false);
    cell(d, col(0.40), col(0.55), 3, "Date", &or(&f.date, &t.date), 2.5, false);
    let sheet = format!("{} · {} of {}", sheet_name(d.width).name(), d.page.max(1), d.pages.max(d.page).max(1));
    cell(d, col(0.55), col(0.85), 3, "Sheet", &sheet, 2.5, false);
    projection_symbol(d, [col(0.925), (rows[2] + rows[4]) / 2.0], options.projection);
}

/// The general requirements over the title block (and parts list): units,
/// "do not scale", and the general surface texture symbol at the right.
pub(super) fn general_line(d: &mut Drawing, at: [f64; 2], width: f64, options: &Options) {
    let text = "Dimensions in mm · Do not scale the drawing · Break sharp edges";
    let mut room = width;
    if let Some(r) = &options.roughness {
        // ISO 1302: the general requirement, then the basic symbol in
        // parentheses (others are marked on the drawing), in 2.5 mm lettering.
        let h = 2.5;
        let t60 = 60f64.to_radians().tan();
        let (h1, h2) = (h * 10.0 / 7.0, h * 3.0);
        let right = at[0] + width;
        let label = roughness_text(r);
        let lw = text_width(&Text { at: [0.0; 2], size: h, text: label.clone(), anchor: Anchor::Start, angle: 0.0, bold: false });
        let close = right - 1.0;
        d.texts.push(Text { at: [close, at[1]], size: h, text: ")".into(), anchor: Anchor::End, angle: 0.0, bold: false });
        let basic = close - 1.5 - h2 / t60;
        symbol_shape(d, [basic, at[1]], h, false, None);
        let open = basic - h1 / t60 - 0.6;
        d.texts.push(Text { at: [open, at[1]], size: h, text: "(".into(), anchor: Anchor::End, angle: 0.0, bold: false });
        let sym_x = open - 3.0 - (h2 / t60 + lw + 1.5);
        symbol_shape(d, [sym_x, at[1]], h, !r.no_removal, Some(lw + 1.5));
        d.texts.push(Text { at: [sym_x + h2 / t60 + 0.3, at[1] + h2 - h - 1.0], size: h, text: label, anchor: Anchor::Start, angle: 0.0, bold: false });
        room = sym_x - h1 / t60 - at[0] - 3.0;
    }
    let size = fitted(text, 2.5, room, false);
    d.texts.push(Text { at, size, text: text.into(), anchor: Anchor::Start, angle: 0.0, bold: false });
}

/// What a surface texture symbol says: "Ra 1.6", with the lay after it.
pub(super) fn roughness_text(r: &Roughness) -> String {
    format!("Ra {}", length_text(r.ra))
}

/// The ISO 1302 symbol with its apex at `apex` (upright): the short leg up
/// left at 60°, the long one up right; closed by a bar for material removal,
/// a circle in its V for none; and the long leg's horizontal over the
/// requirement (`over` mm long) when there is one. Returns its top-right corner.
pub(super) fn symbol_shape(d: &mut Drawing, apex: [f64; 2], h: f64, removal: bool, over: Option<f64>) -> [f64; 2] {
    // ISO 1302 for 3.5 mm lettering: H1 = 5, H2 = 10.5 (in proportion for other sizes).
    let (h1, h2) = (h * 10.0 / 7.0, h * 3.0);
    let t60 = 60f64.to_radians().tan();
    let short = [apex[0] - h1 / t60, apex[1] + h1];
    let long = [apex[0] + h2 / t60, apex[1] + h2];
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![short, apex, long] });
    if removal {
        // The bar across closes the short leg into a triangle.
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![short, [apex[0] + h1 / t60, apex[1] + h1]] });
    }
    if let Some(len) = over {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![long, [long[0] + len, long[1]]] });
        return [long[0] + len, long[1]];
    }
    long
}

/// How tall the notes are (0 when there are none).
pub(super) fn notes_height(notes: &[String], width: f64) -> f64 {
    if notes.is_empty() {
        return 0.0;
    }
    let lines: usize = notes.iter().enumerate().map(|(i, n)| wrap(&format!("{}. {n}", i + 1), width).len()).sum();
    8.0 + lines as f64 * NOTE_PITCH + 3.0
}

fn wrap(text: &str, width: f64) -> Vec<String> {
    let w = |s: &str| text_width(&Text { at: [0.0; 2], size: NOTE_SIZE, text: s.into(), anchor: Anchor::Start, angle: 0.0, bold: false });
    let mut lines = vec![];
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let next = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if w(&next) > width && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur = format!("   {word}");
        } else {
            cur = next;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// The numbered notes, from `top` (their upper left) down.
pub(super) fn notes(d: &mut Drawing, top: [f64; 2], width: f64, notes: &[String]) {
    if notes.is_empty() {
        return;
    }
    let mut y = top[1] - 5.5;
    d.texts.push(Text { at: [top[0], y], size: 3.5, text: "NOTES".into(), anchor: Anchor::Start, angle: 0.0, bold: true });
    y -= 2.5;
    for (i, n) in notes.iter().enumerate() {
        for line in wrap(&format!("{}. {n}", i + 1), width) {
            y -= NOTE_PITCH;
            d.texts.push(Text { at: [top[0], y], size: NOTE_SIZE, text: line, anchor: Anchor::Start, angle: 0.0, bold: false });
        }
    }
}

pub(super) fn revisions_height(revs: &[Revision]) -> f64 {
    if revs.is_empty() { 0.0 } else { REV_ROW * (revs.len() + 1) as f64 }
}

/// The revision table in the top right corner (`corner` is the frame's): REV,
/// DESCRIPTION, DATE, BY, newest at the bottom.
pub(super) fn revisions(d: &mut Drawing, corner: [f64; 2], revs: &[Revision]) {
    if revs.is_empty() {
        return;
    }
    let h = revisions_height(revs);
    let (x, y) = (corner[0] - REV_W, corner[1] - h);
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[x, y], [x + REV_W, y], [x + REV_W, y + h], [x, y + h], [x, y]] });
    let cols = [0.0, 14.0, 98.0, 124.0, REV_W];
    for c in &cols[1..4] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x + c, y], [x + c, y + h]] });
    }
    for r in 1..=revs.len() {
        let ry = y + h - REV_ROW * r as f64;
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, ry], [x + REV_W, ry]] });
    }
    let row = |d: &mut Drawing, r: usize, values: [&str; 4], bold: bool| {
        let ry = y + h - REV_ROW * (r + 1) as f64 + 1.9;
        for (k, v) in values.iter().enumerate() {
            let room = cols[k + 1] - cols[k] - 2.4;
            let size = fitted(v, if bold { 2.0 } else { 2.5 }, room, bold);
            let text = if size <= 1.8 + 1e-9 { clipped(v, 1.8, room) } else { v.to_string() };
            d.texts.push(Text { at: [x + cols[k] + 1.2, ry], size, text, anchor: Anchor::Start, angle: 0.0, bold });
        }
    };
    row(d, 0, ["REV", "DESCRIPTION", "DATE", "BY"], true);
    for (i, r) in revs.iter().enumerate() {
        row(d, i + 1, [&r.rev, &r.description, &r.date, &r.by], false);
    }
}
