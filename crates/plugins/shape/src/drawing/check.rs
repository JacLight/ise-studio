//! A checker's pass over a finished sheet, by measurement: lettering that
//! overlaps other lettering, lettering laid over the part's lines, anything
//! off the frame, lettering too small to read, and dimensions said twice.

use super::*;

/// What the checker found (empty lists: a clean sheet).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Quality {
    /// Pairs of texts whose letters overlap.
    pub overlaps: Vec<[String; 2]>,
    /// Texts laid over a visible or hidden line of the part.
    pub on_lines: Vec<String>,
    /// Texts or lines outside the sheet's frame.
    pub off_sheet: Vec<String>,
    /// Automatic dimensions given more than once.
    pub duplicates: Vec<String>,
    /// The smallest lettering on the sheet (cap height, mm).
    pub smallest_text: f64,
}

impl Quality {
    pub fn is_clean(&self) -> bool {
        self.overlaps.is_empty() && self.on_lines.is_empty() && self.off_sheet.is_empty() && self.duplicates.is_empty()
    }
}

/// A text's letters: the box from just under the baseline to the cap
/// height, along its run, turned with it (sheet mm, axis aligned).
pub fn letters(t: &Text) -> [f64; 4] {
    let w = text_width(t);
    let x0 = match t.anchor {
        Anchor::Start => 0.0,
        Anchor::Middle => -w / 2.0,
        Anchor::End => -w,
    };
    let (sn, cs) = t.angle.to_radians().sin_cos();
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (x, y) in [(x0, -0.2 * t.size), (x0 + w, -0.2 * t.size), (x0, t.size), (x0 + w, t.size)] {
        let p = [t.at[0] + x * cs - y * sn, t.at[1] + x * sn + y * cs];
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

/// Whether the segment pq passes through the box (Liang–Barsky).
fn crosses(p: [f64; 2], q: [f64; 2], b: [f64; 4]) -> bool {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (den, num) in [(-dx, p[0] - b[0]), (dx, b[2] - p[0]), (-dy, p[1] - b[1]), (dy, b[3] - p[1])] {
        if den.abs() < 1e-12 {
            if num < 0.0 {
                return false;
            }
        } else {
            let r = num / den;
            if den < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Measures a sheet. `frame` is the frame's inside.
pub fn inspect(d: &Drawing, frame: [f64; 4]) -> Quality {
    let mut q = Quality { smallest_text: d.texts.iter().map(|t| t.size).fold(f64::INFINITY, f64::min), ..Default::default() };
    if !q.smallest_text.is_finite() {
        q.smallest_text = 0.0;
    }
    let shrink = |b: [f64; 4], m: f64| [b[0] + m, b[1] + m, b[2] - m, b[3] - m];
    let boxes: Vec<[f64; 4]> = d.texts.iter().map(|t| shrink(letters(t), 0.15)).collect();
    for i in 0..boxes.len() {
        if d.texts[i].text.trim().is_empty() {
            continue;
        }
        for j in i + 1..boxes.len() {
            let (a, b) = (boxes[i], boxes[j]);
            if a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3] && !d.texts[j].text.trim().is_empty() {
                q.overlaps.push([d.texts[i].text.clone(), d.texts[j].text.clone()]);
            }
        }
    }
    // The part's own lines: what the views and the isometric draw.
    let part: Vec<&Stroke> = d.strokes.iter().filter(|s| matches!(s.pen, Pen::Visible | Pen::Hidden)).collect();
    for (t, b) in d.texts.iter().zip(&boxes) {
        let b = shrink(*b, 0.25);
        if part.iter().any(|s| s.points.windows(2).any(|w| crosses(w[0], w[1], b))) {
            q.on_lines.push(t.text.clone());
        }
        if b[0] < frame[0] || b[1] < frame[1] || b[2] > frame[2] || b[3] > frame[3] {
            q.off_sheet.push(t.text.clone());
        }
    }
    if d.strokes.iter().flat_map(|s| &s.points).any(|p| p[0] < frame[0] - 1e-6 || p[1] < frame[1] - 1e-6 || p[0] > frame[2] + 1e-6 || p[1] > frame[3] + 1e-6) {
        q.off_sheet.push("lines past the frame".into());
    }
    let mut keys: Vec<&str> = d.autos.iter().map(|a| a.key.as_str()).collect();
    keys.sort();
    for w in keys.windows(2) {
        if w[0] == w[1] && !q.duplicates.iter().any(|k| k == w[0]) {
            q.duplicates.push(w[0].to_string());
        }
    }
    q
}
