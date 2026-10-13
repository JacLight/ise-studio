//! Graphics on a schematic sheet that carry no connectivity: shapes (polyline,
//! polygon, rectangle, circle, arc, bezier), framed text boxes, tables and
//! pictures (PNG or JPEG). Each is drawn into the same primitives as the rest
//! of the sheet, so the editor, the PDF and the SVG plot show the same thing.

use crate::bad;
use crate::schematic::{Schematic, add_item, check_color, check_justify, check_xy, schematic_of, sheet_or_first};
use crate::schematic_view::{Draw, P};
use crate::symbols::text_width;
use de_core::{
    Command, CommandError, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn line_w() -> f64 {
    0.254
}

fn text_size() -> f64 {
    1.8
}

fn yes() -> bool {
    true
}

/// `pcb.sch_shape`: a graphic line or outline.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchShape {
    pub schematic: StableId,
    pub sheet: String,
    /// polyline, polygon, rect, circle, arc or bezier.
    pub shape: String,
    /// polyline/polygon: the corners; rect: two opposite corners; circle: centre and a point
    /// on it; arc: start, a point on the arc, end; bezier: start, two control points, end.
    pub points: Vec<[f64; 2]>,
    #[serde(default = "line_w")]
    pub width: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Fill colour (closed shapes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// solid (default), dash, dot, dash_dot.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dash: String,
}

impl Persist for SchShape {
    const KIND: &'static str = "pcb.sch_shape";
}

/// `pcb.sch_text_box`: text in a frame, wrapped to its width.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchTextBox {
    pub schematic: StableId,
    pub sheet: String,
    /// Top-left corner.
    pub at: [f64; 2],
    pub size: [f64; 2],
    pub text: String,
    #[serde(default = "text_size")]
    pub text_size: f64,
    /// left (default), center, right.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub justify: String,
    #[serde(default = "yes")]
    pub border: bool,
    #[serde(default = "line_w")]
    pub width: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
}

impl Persist for SchTextBox {
    const KIND: &'static str = "pcb.sch_text_box";
}

/// `pcb.sch_table`: rows of text cells in a ruled grid.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchTable {
    pub schematic: StableId,
    pub sheet: String,
    /// Top-left corner.
    pub at: [f64; 2],
    /// Rows of cells.
    pub cells: Vec<Vec<String>>,
    /// Column widths (mm); missing ones fit their text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub col_widths: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_height: Option<f64>,
    #[serde(default = "text_size")]
    pub text_size: f64,
    /// The first row is a header (bold, shaded).
    #[serde(default = "yes")]
    pub header: bool,
    #[serde(default = "line_w")]
    pub width: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Persist for SchTable {
    const KIND: &'static str = "pcb.sch_table";
}

/// `pcb.sch_image`: a picture.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchImage {
    pub schematic: StableId,
    pub sheet: String,
    /// Top-left corner.
    pub at: [f64; 2],
    /// Width on the sheet (mm); the height follows the picture's proportions.
    pub width: f64,
    /// image/png or image/jpeg.
    pub mime: String,
    /// The file, base64.
    pub data: String,
    /// Its size in pixels.
    pub px: [u32; 2],
}

impl Persist for SchImage {
    const KIND: &'static str = "pcb.sch_image";
}

impl SchImage {
    pub fn size(&self) -> [f64; 2] {
        [self.width, self.width * self.px[1].max(1) as f64 / self.px[0].max(1) as f64]
    }
}

// ---- Checks --------------------------------------------------------------------

const SHAPES: [&str; 6] = ["polyline", "polygon", "rect", "circle", "arc", "bezier"];

fn check_dash(d: &str) -> Result<String, CommandError> {
    match d.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "" | "solid" => Ok(String::new()),
        x @ ("dash" | "dot" | "dash_dot") => Ok(x.to_string()),
        _ => Err(bad("dash is solid, dash, dot or dash_dot")),
    }
}

fn check_shape(s: &mut SchShape) -> Result<(), CommandError> {
    s.shape = s.shape.trim().to_ascii_lowercase();
    if s.shape == "line" {
        s.shape = "polyline".into();
    }
    if s.shape == "rectangle" {
        s.shape = "rect".into();
    }
    if !SHAPES.contains(&s.shape.as_str()) {
        return Err(bad(format!("shape is one of {}", SHAPES.join(", "))));
    }
    for p in &s.points {
        check_xy(*p, "points")?;
    }
    let n = s.points.len();
    let ok = match s.shape.as_str() {
        "polyline" => n >= 2,
        "polygon" => n >= 3,
        "rect" | "circle" => n == 2,
        "arc" => n == 3,
        _ => n == 4,
    };
    if !ok {
        return Err(bad(format!("a {} takes {} points", s.shape, match s.shape.as_str() {
            "polyline" => "two or more",
            "polygon" => "three or more",
            "rect" => "two (opposite corners)",
            "circle" => "two (centre, a point on it)",
            "arc" => "three (start, a point on it, end)",
            _ => "four (start, two control points, end)",
        })));
    }
    s.points = s.points.iter().map(|p| [round(p[0]), round(p[1])]).collect();
    if !(0.0..=5.0).contains(&s.width) {
        return Err(bad("line width is 0 to 5 mm"));
    }
    s.color = check_color(s.color.take())?;
    s.fill = check_color(s.fill.take())?;
    s.dash = check_dash(&s.dash)?;
    Ok(())
}

fn round(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

fn check_box(t: &mut SchTextBox) -> Result<(), CommandError> {
    check_xy(t.at, "at")?;
    if t.size.iter().any(|v| !(1.0..=1000.0).contains(v)) {
        return Err(bad("a text box is 1 to 1000 mm each way"));
    }
    if !(0.5..=20.0).contains(&t.text_size) {
        return Err(bad("text size is 0.5 to 20 mm"));
    }
    t.justify = check_justify(&t.justify)?;
    t.color = check_color(t.color.take())?;
    t.fill = check_color(t.fill.take())?;
    Ok(())
}

fn check_table(t: &mut SchTable) -> Result<(), CommandError> {
    check_xy(t.at, "at")?;
    if t.cells.is_empty() || t.cells.iter().all(|r| r.is_empty()) {
        return Err(bad("a table needs at least one cell: cells: [[\"a\", \"b\"], [\"1\", \"2\"]]"));
    }
    if t.cells.len() > 200 || t.cells.iter().any(|r| r.len() > 50) {
        return Err(bad("a table has at most 200 rows and 50 columns"));
    }
    if !(0.5..=20.0).contains(&t.text_size) {
        return Err(bad("text size is 0.5 to 20 mm"));
    }
    if t.col_widths.iter().any(|w| !(1.0..=500.0).contains(w)) || t.row_height.is_some_and(|h| !(1.0..=100.0).contains(&h)) {
        return Err(bad("column widths are 1 to 500 mm, row height 1 to 100 mm"));
    }
    t.color = check_color(t.color.take())?;
    Ok(())
}

/// Decodes base64 (standard or URL alphabet, padding optional, whitespace ignored).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The type and pixel size of a PNG or JPEG file.
pub fn image_info(b: &[u8]) -> Option<(&'static str, [u32; 2])> {
    if b.len() > 24 && b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) && &b[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
        let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        return Some(("image/png", [w, h]));
    }
    if b.len() > 4 && b[0] == 0xff && b[1] == 0xd8 {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if (0xc0..=0xcf).contains(&m) && m != 0xc4 && m != 0xc8 && m != 0xcc {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
                return Some(("image/jpeg", [w, h]));
            }
            if m == 0xd8 || m == 0x01 || (0xd0..=0xd7).contains(&m) {
                i += 2;
            } else {
                i += 2 + len;
            }
        }
    }
    None
}

// ---- Commands -------------------------------------------------------------------

/// `pcb.schematic.shape`: a graphic line or outline (no connectivity).
#[derive(Deserialize, JsonSchema)]
pub struct CreateShape {
    pub schematic: StableId,
    pub sheet: Option<String>,
    /// polyline, polygon, rect, circle, arc or bezier.
    pub shape: String,
    /// polyline/polygon: corners; rect: two opposite corners; circle: centre and a point on it; arc: start, a point on it, end; bezier: start, two control points, end.
    pub points: Vec<[f64; 2]>,
    /// Line width (mm, default 0.254; 0 for a fill without outline).
    pub width: Option<f64>,
    /// Line colour (#rrggbb; default the note colour).
    pub color: Option<String>,
    /// Fill colour for closed shapes.
    pub fill: Option<String>,
    /// solid, dash, dot or dash_dot.
    pub dash: Option<String>,
}

impl Command for CreateShape {
    const NAME: &'static str = "pcb.schematic.shape";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let sheet = sheet_or_first(&s, self.sheet)?;
        let mut sh = SchShape { schematic: self.schematic, sheet, shape: self.shape, points: self.points, width: self.width.unwrap_or(0.254), color: self.color, fill: self.fill, dash: self.dash.unwrap_or_default() };
        check_shape(&mut sh)?;
        let id = add_item(tx, self.schematic, sh)?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.schematic.text_box`: framed text, wrapped to the box (`${VAR}` text variables expand).
#[derive(Deserialize, JsonSchema)]
pub struct CreateTextBox {
    pub schematic: StableId,
    pub sheet: Option<String>,
    /// Top-left corner.
    pub at: [f64; 2],
    pub size: [f64; 2],
    pub text: String,
    pub text_size: Option<f64>,
    pub justify: Option<String>,
    pub border: Option<bool>,
    pub width: Option<f64>,
    pub color: Option<String>,
    pub fill: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
}

impl Command for CreateTextBox {
    const NAME: &'static str = "pcb.schematic.text_box";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let sheet = sheet_or_first(&s, self.sheet)?;
        let mut t = SchTextBox {
            schematic: self.schematic,
            sheet,
            at: self.at,
            size: self.size,
            text: self.text,
            text_size: self.text_size.unwrap_or(1.8),
            justify: self.justify.unwrap_or_default(),
            border: self.border.unwrap_or(true),
            width: self.width.unwrap_or(0.254),
            color: self.color,
            fill: self.fill,
            bold: self.bold,
            italic: self.italic,
        };
        check_box(&mut t)?;
        let id = add_item(tx, self.schematic, t)?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.schematic.table`: a table of text cells (first row a header unless `header: false`).
#[derive(Deserialize, JsonSchema)]
pub struct CreateTable {
    pub schematic: StableId,
    pub sheet: Option<String>,
    /// Top-left corner.
    pub at: [f64; 2],
    /// Rows of cells: [["Rail", "Volts"], ["3V3", "3.3"]].
    pub cells: Vec<Vec<String>>,
    #[serde(default)]
    pub col_widths: Vec<f64>,
    pub row_height: Option<f64>,
    pub text_size: Option<f64>,
    pub header: Option<bool>,
    pub width: Option<f64>,
    pub color: Option<String>,
}

impl Command for CreateTable {
    const NAME: &'static str = "pcb.schematic.table";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let sheet = sheet_or_first(&s, self.sheet)?;
        let mut t = SchTable {
            schematic: self.schematic,
            sheet,
            at: self.at,
            cells: self.cells,
            col_widths: self.col_widths,
            row_height: self.row_height,
            text_size: self.text_size.unwrap_or(1.8),
            header: self.header.unwrap_or(true),
            width: self.width.unwrap_or(0.254),
            color: self.color,
        };
        check_table(&mut t)?;
        let id = add_item(tx, self.schematic, t)?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.schematic.image`: a PNG or JPEG picture (base64, or a data: URI) at `at` (top-left), `width` mm wide.
#[derive(Deserialize, JsonSchema)]
pub struct CreateImage {
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub at: [f64; 2],
    /// Width on the sheet (mm, default 50).
    pub width: Option<f64>,
    /// The file as base64 or a data: URI.
    pub data: String,
}

fn decode_image(data: &str) -> Result<(String, String, [u32; 2]), CommandError> {
    let raw = data.trim();
    let b64 = match raw.strip_prefix("data:") {
        Some(rest) => rest.split_once(',').map(|(_, d)| d).ok_or_else(|| bad("a data: URI has a comma before its data"))?,
        None => raw,
    };
    if b64.len() > 12_000_000 {
        return Err(bad("pictures are up to about 9 MB"));
    }
    let bytes = base64_decode(b64).ok_or_else(|| bad("the picture is not base64"))?;
    let (mime, px) = image_info(&bytes).ok_or_else(|| bad("the picture is not a PNG or JPEG file"))?;
    if px[0] == 0 || px[1] == 0 {
        return Err(bad("the picture has no pixels"));
    }
    Ok((mime.to_string(), b64.chars().filter(|c| !c.is_whitespace()).collect(), px))
}

impl Command for CreateImage {
    const NAME: &'static str = "pcb.schematic.image";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let sheet = sheet_or_first(&s, self.sheet)?;
        check_xy(self.at, "at")?;
        let width = self.width.unwrap_or(50.0);
        if !(1.0..=1000.0).contains(&width) {
            return Err(bad("a picture is 1 to 1000 mm wide"));
        }
        let (mime, data, px) = decode_image(&self.data)?;
        let id = add_item(tx, self.schematic, SchImage { schematic: self.schematic, sheet, at: self.at, width, mime, data, px })?;
        Ok(json!({ "id": id, "px": px }))
    }
}

/// Merges `set` (fields as in the create command) into an item.
fn merge<T: Serialize + for<'de> Deserialize<'de>>(item: &T, set: &Option<Value>) -> Result<T, CommandError> {
    let Some(set) = set else { return serde_json::from_value(serde_json::to_value(item).unwrap()).map_err(|e| bad(e.to_string())) };
    let mut v = serde_json::to_value(item).map_err(|e| bad(e.to_string()))?;
    let (Some(o), Some(n)) = (v.as_object_mut(), set.as_object()) else { return Err(bad("`set` is an object of fields")) };
    for (k, x) in n {
        if k == "schematic" || k == "sheet" {
            continue;
        }
        if k == "data" {
            continue;
        }
        if x.is_null() {
            o.remove(k);
        } else {
            o.insert(k.clone(), x.clone());
        }
    }
    serde_json::from_value(v).map_err(|e| bad(format!("set: {e}")))
}

/// `pcb.schematic.item.update` for graphics (false when `id` is none).
pub(crate) fn update(tx: &mut Tx, id: StableId, at: Option<[f64; 2]>, points: Option<Vec<[f64; 2]>>, delta: Option<[f64; 2]>, set: Option<Value>) -> Result<bool, CommandError> {
    let d = delta.unwrap_or([0.0, 0.0]);
    let mv = |p: [f64; 2]| [round(p[0] + d[0]), round(p[1] + d[1])];
    if let Some(old) = tx.get::<SchShape>(id).cloned() {
        let mut s: SchShape = merge(&old, &set)?;
        if let Some(p) = points {
            s.points = p;
        }
        s.points = s.points.iter().map(|p| mv(*p)).collect();
        check_shape(&mut s)?;
        tx.set(id, s)?;
        return Ok(true);
    }
    if let Some(old) = tx.get::<SchTextBox>(id).cloned() {
        let mut t: SchTextBox = merge(&old, &set)?;
        t.at = mv(at.unwrap_or(t.at));
        check_box(&mut t)?;
        tx.set(id, t)?;
        return Ok(true);
    }
    if let Some(old) = tx.get::<SchTable>(id).cloned() {
        let mut t: SchTable = merge(&old, &set)?;
        t.at = mv(at.unwrap_or(t.at));
        check_table(&mut t)?;
        tx.set(id, t)?;
        return Ok(true);
    }
    if let Some(old) = tx.get::<SchImage>(id).cloned() {
        let mut t: SchImage = merge(&old, &set)?;
        if let Some(data) = set.as_ref().and_then(|s| s.get("data")).and_then(|v| v.as_str()) {
            let (mime, data, px) = decode_image(data)?;
            t.mime = mime;
            t.data = data;
            t.px = px;
        }
        t.at = mv(at.unwrap_or(t.at));
        if !(1.0..=1000.0).contains(&t.width) {
            return Err(bad("a picture is 1 to 1000 mm wide"));
        }
        tx.set(id, t)?;
        return Ok(true);
    }
    Ok(false)
}

// ---- Drawing --------------------------------------------------------------------

const NOTE: &str = "#3c4a63";

fn dash_name(d: &str) -> &'static str {
    match d {
        "dash" => "dash",
        "dot" => "dot",
        "dash_dot" => "dash_dot",
        _ => "",
    }
}

/// Points on the circle through three points, from the first to the last via the middle.
pub fn arc_through(a: P, m: P, b: P) -> Vec<P> {
    let d = 2.0 * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
    if d.abs() < 1e-9 {
        return vec![a, b];
    }
    let sq = |p: P| p[0] * p[0] + p[1] * p[1];
    let cx = (sq(a) * (m[1] - b[1]) + sq(m) * (b[1] - a[1]) + sq(b) * (a[1] - m[1])) / d;
    let cy = (sq(a) * (b[0] - m[0]) + sq(m) * (a[0] - b[0]) + sq(b) * (m[0] - a[0])) / d;
    let r = ((a[0] - cx).powi(2) + (a[1] - cy).powi(2)).sqrt();
    let ang = |p: P| (p[1] - cy).atan2(p[0] - cx);
    let (t0, tm, t1) = (ang(a), ang(m), ang(b));
    let tau = std::f64::consts::TAU;
    let norm = |t: f64| ((t % tau) + tau) % tau;
    // Sweep from a to b the way that passes m.
    let ccw_to = |t: f64| norm(t - t0);
    let sweep = if ccw_to(tm) <= ccw_to(t1) { ccw_to(t1) } else { -(tau - ccw_to(t1)) };
    let n = ((sweep.abs() / tau * 72.0).ceil() as usize).max(4);
    (0..=n).map(|i| {
        let t = t0 + sweep * i as f64 / n as f64;
        [cx + r * t.cos(), cy + r * t.sin()]
    }).collect()
}

fn bezier(p: &[P]) -> Vec<P> {
    (0..=32)
        .map(|i| {
            let t = i as f64 / 32.0;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [a * p[0][0] + b * p[1][0] + c * p[2][0] + d * p[3][0], a * p[0][1] + b * p[1][1] + c * p[2][1] + d * p[3][1]]
        })
        .collect()
}

pub fn shape_draw(s: &SchShape) -> Vec<Draw> {
    let color = s.color.clone().unwrap_or(NOTE.into());
    let (p, closed): (Vec<P>, bool) = match s.shape.as_str() {
        "rect" => {
            let (a, b) = (s.points[0], s.points[1]);
            (vec![a, [b[0], a[1]], b, [a[0], b[1]]], true)
        }
        "circle" => {
            let (c, e) = (s.points[0], s.points[1]);
            let r = ((e[0] - c[0]).powi(2) + (e[1] - c[1]).powi(2)).sqrt();
            ((0..72).map(|i| {
                let t = std::f64::consts::TAU * i as f64 / 72.0;
                [c[0] + r * t.cos(), c[1] + r * t.sin()]
            }).collect(), true)
        }
        "arc" => (arc_through(s.points[0], s.points[1], s.points[2]), false),
        "bezier" => (bezier(&s.points), false),
        "polygon" => (s.points.clone(), true),
        _ => (s.points.clone(), false),
    };
    vec![Draw::Styled { p, w: s.width, color, fill: if closed { s.fill.clone() } else { None }, dash: dash_name(&s.dash), closed }]
}

/// Words wrapped to a width.
fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for w in para.split(' ') {
            let cand = if line.is_empty() { w.to_string() } else { format!("{line} {w}") };
            if !line.is_empty() && text_width(&cand, size) > width {
                out.push(std::mem::take(&mut line));
                line = w.to_string();
            } else {
                line = cand;
            }
        }
        out.push(line);
    }
    out
}

pub fn text_box_draw(t: &SchTextBox, s: &Schematic) -> Vec<Draw> {
    let mut d = Vec::new();
    let color = t.color.clone().unwrap_or(NOTE.into());
    let [x, y] = t.at;
    let [w, h] = t.size;
    let rect = vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    if t.border || t.fill.is_some() {
        d.push(Draw::Styled { p: rect, w: if t.border { t.width } else { 0.0 }, color: color.clone(), fill: t.fill.clone(), dash: "", closed: true });
    }
    let m = 0.8;
    let text = s.expand_vars(&t.text, Some(&t.sheet));
    let lines = wrap(&text, t.text_size, w - 2.0 * m);
    let (ax, anchor) = match t.justify.as_str() {
        "center" => (x + w / 2.0, "middle"),
        "right" => (x + w - m, "end"),
        _ => (x + m, "start"),
    };
    for (i, l) in lines.iter().enumerate() {
        let ly = y + m + t.text_size * 0.5 + i as f64 * t.text_size * 1.35;
        if ly > y + h {
            break;
        }
        d.push(Draw::Rich { at: [ax, ly], s: l.clone(), size: t.text_size, angle: 0.0, anchor, color: color.clone(), bold: t.bold, italic: t.italic });
    }
    d
}

/// Column widths and row height of a table.
pub fn table_layout(t: &SchTable) -> (Vec<f64>, f64) {
    let cols = t.cells.iter().map(|r| r.len()).max().unwrap_or(0);
    let widths = (0..cols)
        .map(|c| {
            t.col_widths.get(c).copied().unwrap_or_else(|| {
                let w = t.cells.iter().filter_map(|r| r.get(c)).map(|s| text_width(s, t.text_size)).fold(0.0, f64::max);
                ((w + 2.0) / 1.27).ceil() * 1.27
            })
        })
        .collect();
    (widths, t.row_height.unwrap_or(t.text_size * 1.9))
}

pub fn table_draw(t: &SchTable, s: &Schematic) -> Vec<Draw> {
    let mut d = Vec::new();
    let color = t.color.clone().unwrap_or(NOTE.into());
    let (widths, rh) = table_layout(t);
    let total_w: f64 = widths.iter().sum();
    let total_h = rh * t.cells.len() as f64;
    let [x, y] = t.at;
    if t.header && !t.cells.is_empty() {
        d.push(Draw::Styled { p: vec![[x, y], [x + total_w, y], [x + total_w, y + rh], [x, y + rh]], w: 0.0, color: color.clone(), fill: Some("#eef0f6".into()), dash: "", closed: true });
    }
    d.push(Draw::Styled { p: vec![[x, y], [x + total_w, y], [x + total_w, y + total_h], [x, y + total_h]], w: t.width, color: color.clone(), fill: None, dash: "", closed: true });
    for r in 1..t.cells.len() {
        let yy = y + rh * r as f64;
        d.push(Draw::Styled { p: vec![[x, yy], [x + total_w, yy]], w: t.width * 0.7, color: color.clone(), fill: None, dash: "", closed: false });
    }
    let mut xx = x;
    for (c, w) in widths.iter().enumerate() {
        if c > 0 {
            d.push(Draw::Styled { p: vec![[xx, y], [xx, y + total_h]], w: t.width * 0.7, color: color.clone(), fill: None, dash: "", closed: false });
        }
        for (r, row) in t.cells.iter().enumerate() {
            if let Some(cell) = row.get(c).filter(|c| !c.is_empty()) {
                d.push(Draw::Rich { at: [xx + 1.0, y + rh * r as f64 + rh / 2.0], s: s.expand_vars(cell, Some(&t.sheet)), size: t.text_size, angle: 0.0, anchor: "start", color: color.clone(), bold: t.header && r == 0, italic: false });
            }
        }
        xx += w;
    }
    d
}

pub fn image_draw(i: &SchImage) -> Vec<Draw> {
    vec![Draw::Image { at: i.at, size: i.size(), mime: i.mime.clone(), data: i.data.clone() }]
}
