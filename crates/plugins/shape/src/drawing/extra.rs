//! What else a drawing can carry, as FreeCAD TechDraw, SolidWorks and
//! Inventor drawings do: sections of any view (full or half, hatched in a
//! pattern), broken views of long parts, auxiliary views square to a slanted
//! face, cosmetic lines, leader notes, free text, tables, and angle, radius,
//! arc length and area dimensions.

use super::*;

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct Extras {
    /// Which view the section replaces (front when absent): front cuts square
    /// to y, top square to z, right square to x.
    #[serde(default)]
    pub section_view: Option<ViewName>,
    /// Where the cutting plane is (model mm along the axis the view looks
    /// along); `section` gives it as a percentage of the depth instead.
    #[serde(default)]
    pub section_at: Option<f64>,
    /// A half section: one half of a symmetric part cut, the other half outside.
    #[serde(default)]
    pub half_section: bool,
    /// How cut faces are hatched (ISO 128-50: 45° thin lines when absent).
    #[serde(default)]
    pub hatch: Option<Hatch>,
    /// Notes on leaders: an arrow on a point of a view, the text on its shoulder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leaders: Vec<LeaderSpec>,
    /// Free text anywhere on the sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<TextSpec>,
    /// Tables (a spreadsheet's worth of cells), placed in free space.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<TableSpec>,
    /// Cosmetic lines on views: centre lines, bend lines, a thread's end, a sketch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<LineSpec>,
    /// Angle dimensions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub angles: Vec<AngleSpec>,
    /// Radius and arc length dimensions on arcs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arcs: Vec<ArcSpec>,
    /// Area callouts: the flat face under a point of a view, its area.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub areas: Vec<AreaSpec>,
    /// Auxiliary views, looking along any direction (square to a slanted face).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aux: Vec<AuxSpec>,
    /// Broken views: a stretch of a long part left out, its dimensions still true.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<BreakSpec>,
}

/// A note on a leader pointing at a point of a view (model mm along its axes).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct LeaderSpec {
    pub view: ViewName,
    pub at: [f64; 2],
    pub text: String,
}

/// Text at a place on the sheet (sheet mm from the lower left).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct TextSpec {
    pub at: [f64; 2],
    pub text: String,
    /// Cap height (mm; 3.5 when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
}

/// A table: rows of cells, the first row its header.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct TableSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rows: Vec<Vec<String>>,
    /// Its upper left corner (sheet mm); found free space when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineStyle {
    /// Narrow continuous.
    #[default]
    Thin,
    /// Wide continuous.
    Visible,
    Hidden,
    /// Chain: centre lines, bend lines.
    Center,
}

/// A line drawn on a view between two points (model mm along its axes), or through more.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct LineSpec {
    pub view: ViewName,
    pub points: Vec<[f64; 2]>,
    #[serde(default)]
    pub style: LineStyle,
}

/// An angle between the lines from `vertex` through `a` and through `b` (model mm along a view's axes).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct AngleSpec {
    pub view: ViewName,
    pub vertex: [f64; 2],
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// The arc's radius (sheet mm; 12 when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
}

/// An arc of a view: its centre, radius and the angles it runs between (°,
/// anticlockwise from the view's right). Dimensioned by its radius, or by
/// its length (⌒) when `length`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct ArcSpec {
    pub view: ViewName,
    pub centre: [f64; 2],
    pub radius: f64,
    #[serde(default)]
    pub from: f64,
    #[serde(default = "full_turn")]
    pub to: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub length: bool,
}

fn full_turn() -> f64 {
    90.0
}

/// The area of the flat face under a point of a view.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct AreaSpec {
    pub view: ViewName,
    pub at: [f64; 2],
}

/// An auxiliary view looking along `dir` (model), with `up` up the page.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct AuxSpec {
    pub dir: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<[f64; 3]>,
    /// Its scale (the sheet's when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    #[default]
    X,
    Y,
    Z,
}

/// A stretch of the part along an axis left out of the views (model mm from `from` to `to`).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct BreakSpec {
    pub axis: Axis,
    pub from: f64,
    pub to: f64,
}
