//! Engineering drawings: a sheet with the part's orthographic views (hidden
//! lines dashed), an isometric view, its overall dimensions and a title
//! block, laid out at a standard scale. The views come from the exact solid
//! (OpenCascade's hidden-line removal), so they are what the STEP file is.
//! The drawing is plain lines and text in sheet millimetres; it is written
//! as a vector PDF or a DXF here, and drawn as SVG by the app.

use crate::ShapeProgram;
use de_core::schemars::{self, JsonSchema};
use de_occt::{Arena, Bool, Shape, ViewLine};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

mod annotate;
mod autodim;
mod extra;
pub mod check;
mod sheet;
pub mod features;
pub mod fits;
pub use autodim::AutoMark;
pub use extra::*;
pub use annotate::{rename_datum, validate, AnnotationMark, DatumSpec, DetailFrame, DetailSpec, FrameSpec, GdtSymbol, HoleFit, MarkKind, Modifier, Tolerance};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
pub enum Sheet {
    A4,
    #[default]
    A3,
    A2,
    A1,
    A0,
    /// ANSI/ASME Y14.1 sizes (inches, landscape): A 11×8.5 … E 44×34.
    #[serde(rename = "ANSI_A")]
    AnsiA,
    #[serde(rename = "ANSI_B")]
    AnsiB,
    #[serde(rename = "ANSI_C")]
    AnsiC,
    #[serde(rename = "ANSI_D")]
    AnsiD,
    #[serde(rename = "ANSI_E")]
    AnsiE,
}

impl Sheet {
    /// Every size, ISO first.
    pub const ALL: [Sheet; 10] = [Sheet::A4, Sheet::A3, Sheet::A2, Sheet::A1, Sheet::A0, Sheet::AnsiA, Sheet::AnsiB, Sheet::AnsiC, Sheet::AnsiD, Sheet::AnsiE];
    /// The size as written ("A3", "ANSI_B").
    pub fn name(self) -> &'static str {
        match self {
            Sheet::A4 => "A4",
            Sheet::A3 => "A3",
            Sheet::A2 => "A2",
            Sheet::A1 => "A1",
            Sheet::A0 => "A0",
            Sheet::AnsiA => "ANSI A",
            Sheet::AnsiB => "ANSI B",
            Sheet::AnsiC => "ANSI C",
            Sheet::AnsiD => "ANSI D",
            Sheet::AnsiE => "ANSI E",
        }
    }
    /// Landscape width and height (mm, ISO 216).
    pub fn size(self) -> [f64; 2] {
        match self {
            Sheet::A4 => [297.0, 210.0],
            Sheet::A3 => [420.0, 297.0],
            Sheet::A2 => [594.0, 420.0],
            Sheet::A1 => [841.0, 594.0],
            Sheet::A0 => [1189.0, 841.0],
            Sheet::AnsiA => [279.4, 215.9],
            Sheet::AnsiB => [431.8, 279.4],
            Sheet::AnsiC => [558.8, 431.8],
            Sheet::AnsiD => [863.6, 558.8],
            Sheet::AnsiE => [1117.6, 863.6],
        }
    }
}

/// Where the views go around the front view (ISO 128-30).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    /// European: the top view below the front, the right view on its left.
    #[default]
    First,
    /// American: the top view above the front, the right view on its right.
    Third,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct Options {
    #[serde(default)]
    pub sheet: Sheet,
    #[serde(default)]
    pub projection: Projection,
    /// Only this part of an assembly (all of it when absent).
    #[serde(default)]
    pub part: Option<String>,
    /// Show the front view cut through (section A–A), this far back through
    /// the part (percent of its depth).
    #[serde(default)]
    pub section: Option<f64>,
    /// Dimensions the person added.
    #[serde(default)]
    pub dimensions: Vec<DimSpec>,
    /// Detail views: circles of a view drawn again larger.
    #[serde(default)]
    pub details: Vec<DetailSpec>,
    /// Geometrical tolerances (feature control frames).
    #[serde(default)]
    pub frames: Vec<FrameSpec>,
    /// Datum feature symbols.
    #[serde(default)]
    pub datums: Vec<DatumSpec>,
    /// Fits on holes, by diameter.
    #[serde(default)]
    pub hole_fits: Vec<HoleFit>,
    /// Which views the sheet shows (all four when empty).
    #[serde(default)]
    pub views: Vec<ViewKind>,
    /// The views' scale (snapped to a standard one); the largest that fits when absent.
    #[serde(default)]
    pub scale: Option<f64>,
    /// The dimensions the drawing finds for itself: which to leave out, which to tolerance.
    #[serde(default)]
    pub auto: AutoSpec,
    /// Threads on holes and diameters, by the diameter modelled.
    #[serde(default)]
    pub threads: Vec<ThreadSpec>,
    /// What the title block says beyond what the part gives it.
    #[serde(default)]
    pub title_block: TitleFields,
    /// General notes, numbered, beside the title block.
    #[serde(default)]
    pub notes: Vec<String>,
    /// The revision table, oldest first.
    #[serde(default)]
    pub revisions: Vec<Revision>,
    /// The surface texture everywhere not marked otherwise (ISO 1302 general symbol).
    #[serde(default)]
    pub roughness: Option<Roughness>,
    /// Surface texture symbols on single surfaces.
    #[serde(default)]
    pub finishes: Vec<FinishSpec>,
    /// Weld symbols (ISO 2553).
    #[serde(default)]
    pub welds: Vec<WeldSpec>,
    /// Draw an assembly's isometric exploded: parts slid apart along an axis.
    #[serde(default)]
    pub explode: Option<Explode>,
    /// Sections, cosmetic lines, leaders, texts, tables, angle, arc and area
    /// dimensions, auxiliary and broken views.
    #[serde(flatten)]
    pub extra: Extras,
}

/// Section hatching: lines at `angle`° (from the horizontal), `spacing` mm apart, crossed when `cross`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct Hatch {
    #[serde(default = "default_hatch_angle")]
    pub angle: f64,
    #[serde(default = "default_hatch_spacing")]
    pub spacing: f64,
    #[serde(default)]
    pub cross: bool,
}

fn default_hatch_angle() -> f64 {
    45.0
}

fn default_hatch_spacing() -> f64 {
    2.5
}

/// An exploded isometric: each part slid along `axis` (x, y or z) clear of
/// those it stacks on, `gap` sheet-independent mm apart (8 % of the
/// assembly's size along the axis when absent).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct Explode {
    #[serde(default)]
    pub axis: ExplodeAxis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExplodeAxis {
    X,
    Y,
    #[default]
    Z,
}

/// Title block fields the person sets (each one replaces what the drawing would say).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct TitleFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Who owns the drawing (company or project).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawn_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Finish or treatment: "Anodise clear, 10 µm", "Black oxide".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    /// The general tolerances (ISO 2768-m when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<String>,
}

/// One row of the revision table.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct Revision {
    pub rev: String,
    pub description: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub by: String,
}

/// A surface texture requirement (ISO 1302 / ISO 21920-1).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct Roughness {
    /// Ra in µm (0.8, 1.6, 3.2…).
    pub ra: f64,
    /// How it is made: "ground", "turned", "milled", "lapped".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    /// Lay direction symbol: "=", "⊥", "X", "M", "C", "R".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lay: Option<String>,
    /// Material must not be removed (an as-cast or as-rolled surface): the circle symbol.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_removal: bool,
}

/// A surface texture symbol on a surface: a point on its edge in a view (model mm along the view's axes).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct FinishSpec {
    pub view: ViewName,
    pub at: [f64; 2],
    #[serde(flatten)]
    pub roughness: Roughness,
}

/// Weld kinds with their ISO 2553 symbols.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WeldKind {
    #[default]
    Fillet,
    SquareButt,
    VButt,
    BevelButt,
    Plug,
    Spot,
}

/// A weld symbol pointing at a joint (a point in a view, model mm along its axes).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct WeldSpec {
    pub view: ViewName,
    pub at: [f64; 2],
    pub kind: WeldKind,
    /// Its size: a fillet's throat (a5), a butt weld's penetration (s4), a spot's diameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Weld length (mm), when it does not run the whole joint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// Both sides of the joint.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub both_sides: bool,
    /// On the other side of the joint from the arrow.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub other_side: bool,
    /// All round the part.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_around: bool,
    /// Made on site, not in the shop.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub field: bool,
}

/// How the drawing's own dimensions are kept: every feature is dimensioned
/// once unless `off`; keys (as the drawing JSON's `autos` lists them, like
/// "x:25", "d:20", "hole:5", "r:8", "c:1") pick single ones out.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct AutoSpec {
    /// No automatic dimensions at all (only overall sizes and the person's own).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off: bool,
    /// Keys of automatic dimensions left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// Tolerances on automatic dimensions, by key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerances: Vec<AutoTolerance>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct AutoTolerance {
    pub key: String,
    pub tolerance: Tolerance,
}

/// A thread on the holes or the diameter modelled at `diameter` (a tapped
/// hole is modelled at its tap drill, a threaded shank at its nominal size):
/// called out ISO 965 style (M6-6H, M16×1.5-6g) and drawn the ISO 6410 way.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct ThreadSpec {
    pub diameter: f64,
    /// "M6", "M16×1.5" (the pitch when it is not the coarse one).
    pub size: String,
    /// Tolerance class: "6H" for a tapped hole, "6g" for a bolt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// How long the thread is (a blind hole's full depth, a shank's whole stretch when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
}

/// A view a sheet can show: the three orthographic views and the isometric.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Front,
    Top,
    Right,
    Iso,
}

/// One of the three orthographic views.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewName {
    #[default]
    Front,
    Top,
    Right,
}

impl ViewName {
    /// The model axes across and up the view (0 x, 1 y, 2 z).
    pub fn axes(self) -> [usize; 2] {
        match self {
            ViewName::Front => [0, 2],
            ViewName::Top => [0, 1],
            ViewName::Right => [1, 2],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DimKind {
    /// Across the view.
    #[default]
    Horizontal,
    /// Up the view.
    Vertical,
    /// Straight from one point to the other.
    Aligned,
}

/// A dimension between two points of a view, kept in model millimetres so
/// it stays on the part when the sheet or scale changes.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
pub struct DimSpec {
    pub view: ViewName,
    /// The two points, as model coordinates along the view's axes.
    pub a: [f64; 2],
    pub b: [f64; 2],
    #[serde(default)]
    pub kind: DimKind,
    /// How far the dimension line stands off the points (sheet mm; negative: the other side).
    #[serde(default = "default_offset")]
    pub offset: f64,
    /// Its tolerance: ±, limit deviations, or an ISO 286 fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<Tolerance>,
    /// A diameter: the value is written ⌀25.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diameter: bool,
    /// Drawn in this detail view (its index in the details) instead of the
    /// view itself: the points are still model mm along the view's axes, so
    /// the value is the true size, whatever the detail's scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<usize>,
    /// The value to state when the points are not the true size apart (a broken view).
    #[serde(skip)]
    #[schemars(skip)]
    pub stated: Option<f64>,
}

fn default_offset() -> f64 {
    10.0
}

/// One row of the parts list.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PartRow {
    pub name: String,
    pub quantity: usize,
    pub material: String,
    pub mass: String,
    /// How it is made ("cnc_mill", "3d_print") or "buy".
    pub process: String,
    /// The parts of the program this row stands for, when it merges several
    /// of the same part ("Spacer 1" … "Spacer 4" as "Spacer").
    pub members: Vec<String>,
}

/// Where a view sits on the sheet: a model point p shows at
/// `at + [p[axes[0]], p[axes[1]]] · scale`.
#[derive(Clone, Debug, Serialize)]
pub struct ViewFrame {
    pub view: ViewName,
    pub at: [f64; 2],
    pub scale: f64,
    pub axes: [usize; 2],
    /// The view's outline on the sheet: [x0, y0, x1, y1].
    pub bounds: [f64; 4],
}

/// What the title block says.
#[derive(Clone, Debug, Default)]
pub struct Title {
    pub title: String,
    pub number: String,
    pub material: String,
    pub mass: String,
    pub drawn_by: String,
    pub date: String,
    /// The parts list (shown when there are two or more parts).
    pub parts: Vec<PartRow>,
    /// This sheet's number and how many sheets the drawing has (1 of 1 when 0).
    pub page: usize,
    pub pages: usize,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Pen {
    /// Visible edges: wide (0.5 mm).
    Visible,
    /// Hidden edges: dashed, narrow.
    Hidden,
    /// Dimension and extension lines, cells of the title block: narrow (0.25 mm).
    Thin,
    /// Centre lines: narrow long-dash-dot (chain) lines.
    Center,
    /// The sheet frame: widest (0.7 mm).
    Frame,
}

impl Pen {
    pub fn width(self) -> f64 {
        match self {
            Pen::Visible => 0.5,
            Pen::Hidden => 0.25,
            Pen::Thin | Pen::Center => 0.25,
            Pen::Frame => 0.7,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Stroke {
    pub pen: Pen,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug, Serialize)]
pub struct Text {
    /// The baseline point the anchor refers to (sheet mm, y up).
    pub at: [f64; 2],
    /// Cap height (mm).
    pub size: f64,
    pub text: String,
    pub anchor: Anchor,
    /// Degrees anticlockwise.
    pub angle: f64,
    pub bold: bool,
}

/// A drawing in sheet millimetres, origin at the bottom left.
#[derive(Clone, Debug, Serialize)]
pub struct Drawing {
    pub width: f64,
    pub height: f64,
    /// Drawing size over real size (0.5 for 1:2).
    pub scale: f64,
    pub scale_text: String,
    pub strokes: Vec<Stroke>,
    /// Filled shapes (arrowheads).
    pub fills: Vec<Vec<[f64; 2]>>,
    pub texts: Vec<Text>,
    /// Where the orthographic views are, for picking points on them.
    pub views: Vec<ViewFrame>,
    /// Where each of the person's dimensions landed: its index in
    /// `Options::dimensions` and its box on the sheet.
    pub marks: Vec<DimMark>,
    /// Where the isometric view is, when there is room for one.
    pub iso: Option<IsoFrame>,
    /// The item balloons on the isometric view, one per row of the parts list.
    pub balloons: Vec<Balloon>,
    /// Where the details, frames, datums and hole callouts landed.
    pub annotations: Vec<AnnotationMark>,
    /// Where the detail views landed, for picking points on them.
    pub details: Vec<DetailFrame>,
    /// The dimensions the drawing found for itself, by key.
    pub autos: Vec<AutoMark>,
    /// This sheet's number and how many there are.
    pub page: usize,
    pub pages: usize,
    /// What could not go on the sheet.
    pub warnings: Vec<String>,
    /// The checker's measurements of the finished sheet.
    pub quality: check::Quality,
}

impl Drawing {
    /// An empty sheet.
    pub fn blank(width: f64, height: f64) -> Drawing {
        Drawing { width, height, scale: 1.0, scale_text: String::new(), strokes: vec![], fills: vec![], texts: vec![], views: vec![], marks: vec![], iso: None, balloons: vec![], annotations: vec![], details: vec![], autos: vec![], page: 1, pages: 1, warnings: vec![], quality: Default::default() }
    }
}

/// Where the isometric view sits: a model point p shows at
/// `at + iso_point(p) · scale`.
#[derive(Clone, Debug, Serialize)]
pub struct IsoFrame {
    pub at: [f64; 2],
    pub scale: f64,
    /// Its outline on the sheet: [x0, y0, x1, y1].
    pub bounds: [f64; 4],
}

/// A numbered balloon (ISO 6433): its parts-list item, the circle, and the
/// point on the part its leader ends on (sheet mm).
#[derive(Clone, Debug, Serialize)]
pub struct Balloon {
    pub item: usize,
    pub centre: [f64; 2],
    pub radius: f64,
    pub anchor: [f64; 2],
}

#[derive(Clone, Debug, Serialize)]
pub struct DimMark {
    pub dimension: usize,
    pub bounds: [f64; 4],
}

/// Standard scales (ISO 5455), largest first.
const SCALES: [(f64, &str); 16] = [
    (10.0, "10:1"),
    (5.0, "5:1"),
    (2.0, "2:1"),
    (1.0, "1:1"),
    (0.5, "1:2"),
    (0.2, "1:5"),
    (0.1, "1:10"),
    (0.05, "1:20"),
    (0.02, "1:50"),
    (0.01, "1:100"),
    (0.005, "1:200"),
    (0.002, "1:500"),
    (0.001, "1:1000"),
    (0.0005, "1:2000"),
    (0.0002, "1:5000"),
    (0.0001, "1:10000"),
];

struct View {
    lines: Vec<ViewLine>,
    min: [f64; 2],
    max: [f64; 2],
}

impl View {
    fn of(lines: Vec<ViewLine>) -> View {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in lines.iter().flat_map(|l| &l.points) {
            for k in 0..2 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        if !min[0].is_finite() {
            (min, max) = ([0.0; 2], [0.0; 2]);
        }
        View { lines, min, max }
    }
    fn size(&self) -> [f64; 2] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1]]
    }
}

/// Lengths as a drawing shows them: whole millimetres, or to 0.1 / 0.01 when they need it.
pub fn length_text(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    let s = if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else if (r * 10.0 - (r * 10.0).round()).abs() < 1e-9 { format!("{r:.1}") } else { format!("{r:.2}") };
    s
}

/// The drawing of a program (its exact solid), on a sheet.
pub fn drawing(program: &ShapeProgram, options: &Options, title: &Title) -> Result<Drawing, String> {
    let mut all = drawings(program, &[(options.clone(), title.clone())])?;
    Ok(all.remove(0))
}

/// The sheets of a drawing, each with its own options and title block, from
/// one evaluation of the program. A feature control frame may refer to a
/// datum marked on any of the sheets of the same part.
pub fn drawings(program: &ShapeProgram, sheets: &[(Options, Title)]) -> Result<Vec<Drawing>, String> {
    let values = program.values();
    let tolerance = program.display_tolerance();
    let (mut arena, exact) = crate::exact::evaluate_exact_in(Arena::new(), &program.name, &program.source, &values, tolerance, &program.inputs).map_err(|(_, e)| e)?;
    let mut out = vec![];
    for (options, title) in sheets {
        let letters: Vec<&str> = sheets.iter().filter(|(o, _)| o.part == options.part).flat_map(|(o, _)| o.datums.iter().map(|d| d.letter.as_str())).collect();
        let parts: Vec<(String, Shape)> = exact.parts.iter().filter(|p| options.part.as_ref().is_none_or(|n| &p.name == n)).map(|p| (p.name.clone(), p.shape)).collect();
        if parts.is_empty() {
            return Err(match &options.part {
                Some(n) if exact.left_out.parts.contains(n) => format!("“{n}” is left out of this configuration: draw it in one that has it"),
                Some(n) => format!("no part called “{n}”"),
                None => "the program made nothing to draw".into(),
            });
        }
        let shape = if parts.len() == 1 { parts[0].1 } else { arena.compound(&parts.iter().map(|p| p.1).collect::<Vec<_>>())? };
        let mut d = layout(&mut arena, shape, &parts, options, title)?;
        for f in &options.frames {
            for r in f.datums.iter().flat_map(|r| r.split('-')) {
                if !letters.contains(&r) {
                    let w = format!("the {} frame refers to datum {r}, which no datum symbol marks", format!("{:?}", f.symbol).to_lowercase());
                    if !d.warnings.contains(&w) {
                        d.warnings.push(w);
                    }
                }
            }
        }
        out.push(d);
    }
    Ok(out)
}

/// The isometric view's direction and up (the view looks down at the front right).
fn iso_frame() -> ([f64; 3], [f64; 3]) {
    let k = 1.0 / 3f64.sqrt();
    ([-k, k, -k], [0.0, 0.0, 1.0])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: [f64; 3]) -> [f64; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Where a model point lands in a hidden-line view looking along `dir` with
/// `up` up the page, in the frame [`Arena::hlr_view`] draws in: x across and
/// y up the view (mm), and how near the eye it is (larger is nearer).
pub fn view_point(dir: [f64; 3], up: [f64; 3], p: [f64; 3]) -> [f64; 3] {
    let toward_eye = unit([-dir[0], -dir[1], -dir[2]]);
    let right = unit(cross(up, toward_eye));
    let upward = cross(toward_eye, right);
    let dot = |a: [f64; 3]| a[0] * p[0] + a[1] * p[1] + a[2] * p[2];
    [dot(right), dot(upward), dot(toward_eye)]
}

/// Where the isometric view shows a model point (view mm, as [`view_point`]).
pub fn iso_point(p: [f64; 3]) -> [f64; 2] {
    let (dir, up) = iso_frame();
    let v = view_point(dir, up, p);
    [v[0], v[1]]
}

/// Lays out the views of `shape` on the sheet. `parts` are its parts by
/// name, for the balloons that key the isometric to the parts list.
pub fn layout(arena: &mut Arena, shape: Shape, parts: &[(String, Shape)], options: &Options, title: &Title) -> Result<Drawing, String> {
    if options.extra.breaks.is_empty() || parts.len() != 1 {
        return layout_in(arena, shape, parts, options, title, &Breaks::default());
    }
    // A broken view: the stretches left out are cut away and the far side
    // slid back, a short gap kept to show the break; positions are told
    // the true way round.
    let b = arena.bounds(shape)?;
    let mut list: Vec<(usize, f64, f64)> = options.extra.breaks.iter().map(|x| (x.axis as usize, x.from.min(x.to), x.from.max(x.to))).filter(|x| x.2 - x.1 > 1e-6).collect();
    list.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut cut = shape;
    let mut brk = Breaks::default();
    for (k, from, to) in list {
        if from <= b[k] || to >= b[k + 3] {
            continue;
        }
        let gap = 0.03 * (b[k + 3] - b[k] - (to - from));
        let m = 1.0 + (b[3] - b[0]).max(b[4] - b[1]).max(b[5] - b[2]);
        let slab = |arena: &mut Arena, lo: f64, hi: f64| -> Result<Shape, String> {
            let mut size = [b[3] - b[0] + 2.0 * m, b[4] - b[1] + 2.0 * m, b[5] - b[2] + 2.0 * m];
            size[k] = hi - lo;
            let mut at = [b[0] - m, b[1] - m, b[2] - m];
            at[k] = lo;
            let c = arena.cube(size[0], size[1], size[2], false)?;
            arena.translate(c, at)
        };
        let low = slab(arena, b[k] - m, from)?;
        let high = slab(arena, to, b[k + 3] + m)?;
        let low = arena.boolean(Bool::Intersection, cut, low)?;
        let high = arena.boolean(Bool::Intersection, cut, high)?;
        let mut v = [0.0; 3];
        v[k] = -(to - from - gap);
        let high = arena.translate(high, v)?;
        cut = arena.compound(&[low, high])?;
        brk.cuts.push(Cut { axis: k, at: from, from, gap, removed: to - from - gap });
    }
    // Where each shows: its true place less what the breaks below it left out.
    let all = brk.cuts.clone();
    for c in brk.cuts.iter_mut() {
        c.at = c.from - all.iter().filter(|o| o.axis == c.axis && o.from < c.from).map(|o| o.removed).sum::<f64>();
    }
    // The person's annotations are in true positions: moved where the broken view shows them.
    let mut o = options.clone();
    let map = |view: ViewName, p: [f64; 2]| {
        let ax = view.axes();
        [brk.drawn_at(ax[0], p[0]), brk.drawn_at(ax[1], p[1])]
    };
    for dim in o.dimensions.iter_mut() {
        dim.stated = Some(dim.value());
        (dim.a, dim.b) = (map(dim.view, dim.a), map(dim.view, dim.b));
    }
    for x in o.details.iter_mut() {
        x.center = map(x.view, x.center);
    }
    for x in o.frames.iter_mut() {
        x.at = map(x.view, x.at);
    }
    for x in o.datums.iter_mut() {
        x.at = map(x.view, x.at);
    }
    for x in o.finishes.iter_mut() {
        x.at = map(x.view, x.at);
    }
    for x in o.welds.iter_mut() {
        x.at = map(x.view, x.at);
    }
    for x in o.extra.leaders.iter_mut() {
        x.at = map(x.view, x.at);
    }
    for x in o.extra.areas.iter_mut() {
        x.at = map(x.view, x.at);
    }
    for x in o.extra.lines.iter_mut() {
        for p in x.points.iter_mut() {
            *p = map(x.view, *p);
        }
    }
    for x in o.extra.angles.iter_mut() {
        (x.vertex, x.a, x.b) = (map(x.view, x.vertex), map(x.view, x.a), map(x.view, x.b));
    }
    for x in o.extra.arcs.iter_mut() {
        x.centre = map(x.view, x.centre);
    }
    let name = parts[0].0.clone();
    layout_in(arena, cut, &[(name, cut)], &o, title, &brk)
}

/// Where a broken view leaves a stretch out.
#[derive(Clone, Debug, Default)]
pub(super) struct Breaks {
    pub cuts: Vec<Cut>,
}

/// One break: along `axis`, at `at` (where the view shows it), a `gap` drawn, `removed` mm left out.
#[derive(Clone, Debug)]
pub(super) struct Cut {
    pub axis: usize,
    pub at: f64,
    /// Where the stretch left out starts, truly.
    pub from: f64,
    pub gap: f64,
    pub removed: f64,
}

impl Breaks {
    /// The true coordinate of a point the broken view shows at `v`.
    pub fn true_at(&self, k: usize, v: f64) -> f64 {
        v + self.cuts.iter().filter(|c| c.axis == k && v >= c.at + c.gap * 0.5).map(|c| c.removed).sum::<f64>()
    }
    /// Where the broken view shows a point at true coordinate `v`.
    pub fn drawn_at(&self, k: usize, v: f64) -> f64 {
        v - self.cuts.iter().filter(|c| c.axis == k && v >= c.from + c.removed).map(|c| c.removed).sum::<f64>()
    }
    /// Whether a flat face at `v` along `k` is one the break cut (not the part's own).
    pub fn is_cut(&self, k: usize, v: f64) -> bool {
        self.cuts.iter().any(|c| c.axis == k && ((v - c.at).abs() < 1e-3 || (v - c.at - c.gap).abs() < 1e-3))
    }
}

fn layout_in(arena: &mut Arena, shape: Shape, parts: &[(String, Shape)], options: &Options, title: &Title, brk: &Breaks) -> Result<Drawing, String> {
    let b = arena.bounds(shape)?;
    let size3 = [b[3] - b[0], b[4] - b[1], b[5] - b[2]];
    let deflection = (size3.iter().cloned().fold(0.0, f64::max) / 2000.0).clamp(0.005, 1.0);
    // One part is a detail drawing: its features are read off the solid and
    // dimensioned. An assembly drawing shows how parts go together instead.
    let feats = if parts.len() == 1 { Some(features::read(&arena.face_info(shape)?)) } else { None };
    let dimensioned = feats.is_some() && !options.auto.off;
    // A turned part shows its profile once, axis across the sheet where it can be.
    let turned_views: Vec<ViewKind> = match feats.as_ref().and_then(|f| f.turned.as_ref()).and_then(|t| features::principal(t.line.dir)) {
        Some(1) => vec![ViewKind::Right, ViewKind::Iso],
        Some(_) => vec![ViewKind::Front, ViewKind::Iso],
        None => vec![],
    };
    let exploded_only = [ViewKind::Iso];
    let wanted: &[ViewKind] = if !options.views.is_empty() {
        &options.views
    } else if options.explode.is_some() && parts.len() >= 2 {
        &exploded_only
    } else {
        &turned_views
    };
    let shown = |k: ViewKind| wanted.is_empty() || wanted.contains(&k);
    let (has_f, has_t, has_r, has_i) = (shown(ViewKind::Front), shown(ViewKind::Top), shown(ViewKind::Right), shown(ViewKind::Iso));
    let mut warnings = vec![];
    // A view the sheet leaves out is an empty one: no size, nothing drawn.
    let view = |arena: &mut Arena, dir: [f64; 3], up: [f64; 3], on: bool| if on { arena.hlr_view(shape, dir, up, deflection).map(View::of) } else { Ok(View::of(vec![])) };
    // Section A–A: a view of what lies beyond the cutting plane, no hidden
    // lines (the front view cut square to y, the top square to z, the right square to x).
    let section_view = options.extra.section_view.unwrap_or(ViewName::Front);
    let look = section_look(section_view);
    let section_shown = match section_view {
        ViewName::Front => has_f,
        ViewName::Top => has_t,
        ViewName::Right => has_r,
    };
    let wants_section = options.section.is_some() || options.extra.section_at.is_some();
    let cut = if wants_section && section_shown {
        Some(options.extra.section_at.unwrap_or_else(|| b[look] + size3[look] * (options.section.unwrap_or(50.0) / 100.0).clamp(0.01, 0.99)).clamp(b[look] + 1e-3, b[look + 3] - 1e-3))
    } else {
        None
    };
    if wants_section && !section_shown {
        warnings.push(format!("section A–A is the {} view, which this sheet leaves out", format!("{section_view:?}").to_lowercase()));
    }
    let view_dir = |n: ViewName| match n {
        ViewName::Front => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ViewName::Top => ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ViewName::Right => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    };
    // The middle across a half section (the view's horizontal axis).
    let half_at = (b[section_view.axes()[0]] + b[section_view.axes()[0] + 3]) / 2.0;
    let cut_view = |arena: &mut Arena, n: ViewName, on: bool| -> Result<View, String> {
        let (dir, up) = view_dir(n);
        match cut.filter(|_| n == section_view) {
            None => view(arena, dir, up, on),
            Some(c) => {
                let m = size3.iter().cloned().fold(0.0, f64::max) + 1.0;
                // What lies beyond the plane, seen from the near side.
                let (lo, hi) = if look == 1 { (c, b[4] + m) } else { (b[look] - m, c) };
                let mut size = [size3[0] + 2.0 * m, size3[1] + 2.0 * m, size3[2] + 2.0 * m];
                size[look] = hi - lo;
                let mut at = [b[0] - m, b[1] - m, b[2] - m];
                at[look] = lo;
                let keep = arena.cube(size[0], size[1], size[2], false)?;
                let keep = arena.translate(keep, at)?;
                let behind = arena.boolean(Bool::Intersection, shape, keep)?;
                let mut lines = arena.hlr_view(behind, dir, up, deflection)?;
                lines.retain(|l| !l.hidden);
                if options.extra.half_section {
                    // The cut half right of the middle, the outside left of it.
                    let mut cut_half = clip_lines(&lines, half_at, true);
                    let outside = arena.hlr_view(shape, dir, up, deflection)?;
                    cut_half.extend(clip_lines(&outside, half_at, false));
                    lines = cut_half;
                }
                Ok(View::of(lines))
            }
        }
    };
    let front = cut_view(arena, ViewName::Front, has_f)?;
    let top = cut_view(arena, ViewName::Top, has_t)?;
    let right = cut_view(arena, ViewName::Right, has_r)?;
    let (iso_dir, iso_up) = iso_frame();
    // An exploded isometric: each part slid clear along the axis.
    let exploded: Option<Vec<(String, Shape)>> = match (&options.explode, parts.len() >= 2 && has_i) {
        (Some(e), true) => {
            let k = e.axis as usize;
            let boxes: Vec<[f64; 6]> = parts.iter().map(|p| arena.bounds(p.1)).collect::<Result<_, _>>()?;
            let gap = e.gap.unwrap_or_else(|| (0.12 * size3[k]).max(0.05 * (size3[0].hypot(size3[1]).hypot(size3[2]))));
            let shifts = explode_shifts(&boxes, k, gap);
            let mut moved = vec![];
            for (p, sh) in parts.iter().zip(shifts) {
                let mut v = [0.0; 3];
                v[k] = sh;
                moved.push((p.0.clone(), if sh.abs() > 1e-9 { arena.translate(p.1, v)? } else { p.1 }));
            }
            Some(moved)
        }
        _ => None,
    };
    let iso = match &exploded {
        Some(moved) => {
            let all = arena.compound(&moved.iter().map(|p| p.1).collect::<Vec<_>>())?;
            View::of(arena.hlr_view(all, iso_dir, iso_up, deflection)?)
        }
        None => view(arena, iso_dir, iso_up, has_i)?,
    };
    let has_ortho = has_f || has_t || has_r;

    let [w, h] = options.sheet.size();
    let mut d = Drawing::blank(w, h);
    d.page = title.page.max(1);
    d.pages = title.pages.max(d.page);
    d.warnings = warnings;
    // Frame: 20 mm binding margin on the left, 10 mm elsewhere (ISO 5457).
    let (fx0, fy0, fx1, fy1) = (20.0, 10.0, w - 10.0, h - 10.0);
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[fx0, fy0], [fx1, fy0], [fx1, fy1], [fx0, fy1], [fx0, fy0]] });
    let block = [180.0f64.min(fx1 - fx0), TITLE_H];
    // The parts list sits on the title block, one row per part plus a header.
    let list_h = if title.parts.len() >= 2 { 6.0 * (title.parts.len() + 1) as f64 } else { 0.0 };
    // With a parts list, the isometric keeps a margin round it for its balloons.
    let ballooned = list_h > 0.0 && parts.len() >= 2;
    let room = if ballooned { BALLOON_GAP + BALLOON_R + 2.0 } else { 0.0 };
    // Above the parts list: the line of general notes (units, scale, the general roughness).
    let stack_h = block[1] + list_h + if options.roughness.is_some() { 10.0 } else { 7.0 };
    // The numbered notes take the strip left of the title block (higher when they need it),
    // the revision table the top right corner.
    let notes_w = fx1 - block[0] - fx0 - 10.0;
    let notes_h = sheet::notes_height(&options.notes, notes_w);
    let table_h = sheet::revisions_height(&options.revisions);
    // The revision table goes over the notes beside the title block when
    // there is room there (ISO 7200 puts it next to the title block), else in
    // the top right corner.
    let rev_beside = table_h > 0.0 && notes_h + table_h + 4.0 <= stack_h && notes_w >= sheet::REV_W;
    let rev_h = if rev_beside { 0.0 } else { table_h };
    let notes_h = if rev_beside { notes_h + table_h + 4.0 } else { notes_h };

    // The views in a grid: front in the middle, the others where the projection puts them.
    // Sheet mm between views: room for a dimension, or a row of ordinates.
    let gap = if dimensioned { 24.0 } else { 22.0 };
    // Round the outside of the views: room for their dimensions.
    let margin = 20.0;
    let (fw, fh) = (front.size()[0], front.size()[1]);
    let (tw, th) = (top.size()[0], top.size()[1]);
    let (rw, rh) = (right.size()[0], right.size()[1]);
    let (iw, ih) = (iso.size()[0], iso.size()[1]);
    // Gaps between the columns and rows of views only where both sides have one.
    let gx = if has_r && (has_f || has_t) { gap } else { 0.0 };
    let gy = if has_t && (has_f || has_r) { gap } else { 0.0 };
    // The main scale fits the three orthographic views; the isometric takes
    // whatever scale fits the room left beside them (and says so). A sheet
    // of the isometric alone draws it at the main scale.
    let reserve = if has_i { 40.0 } else { 0.0 };
    // Where the views go: above the title block (and parts list), or beside
    // them when a long parts list leaves more room there. [x0, y0, x1, y1].
    let regions = [[fx0, fy0 + stack_h.max(notes_h), fx1, fy1 - rev_h], [fx0, fy0 + notes_h, fx1 - block[0] - 4.0, fy1 - rev_h]];
    let rev_rect = [fx1 - sheet::REV_W - 6.0, fy1 - rev_h - 6.0, fx1, fy1];
    let fits_in = |r: [f64; 4], s: f64| {
        let area = [r[2] - r[0] - 2.0 * margin, r[3] - r[1] - 2.0 * margin];
        if has_ortho {
            (rw + fw.max(tw)) * s + gx <= area[0] - reserve && (fh.max(rh) + th) * s + gy <= area[1]
        } else {
            iw * s <= area[0] + margin - 2.0 * room && ih * s <= area[1] + margin - 8.0 - 2.0 * room
        }
    };
    let best_in = |r: [f64; 4]| SCALES.iter().find(|(s, _)| fits_in(r, *s)).copied().unwrap_or(SCALES[SCALES.len() - 1]);
    let mut region = if list_h > 0.0 && best_in(regions[1]).0 > best_in(regions[0]).0 { regions[1] } else { regions[0] };
    // The views may reach up past the revision table's bottom when they then
    // come out larger and still keep clear of it (it sits in the corner).
    if rev_h > 0.0 {
        let tall = [region[0], region[1], region[2], fy1];
        let k = best_in(tall).0;
        if k > best_in(region).0 && has_ortho {
            let area = [tall[2] - tall[0] - 2.0 * margin, tall[3] - tall[1] - 2.0 * margin];
            let (w_all, h_all) = ((rw + fw.max(tw)) * k + gx, (fh.max(rh) + th) * k + gy);
            let x0 = tall[0] + margin + (area[0] - w_all).max(0.0) / 2.0;
            let y0 = tall[1] + margin + (area[1] - h_all).max(0.0) / 2.0;
            let r = [x0 - 20.0, y0 - 20.0, x0 + w_all + 20.0, y0 + h_all + 20.0];
            if !(r[0] < rev_rect[2] && rev_rect[0] < r[2] && r[1] < rev_rect[3] && rev_rect[1] < r[3]) {
                region = tall;
            }
        }
    }
    let area = [region[2] - region[0] - 2.0 * margin, region[3] - region[1] - 2.0 * margin];
    let fits = |s: f64| fits_in(region, s);
    let auto = best_in(region);
    let (scale, scale_text) = match options.scale {
        None => auto,
        Some(want) => {
            let near = SCALES.iter().copied().min_by(|a, b| (a.0.ln() - want.ln()).abs().total_cmp(&(b.0.ln() - want.ln()).abs())).unwrap_or(auto);
            if fits(near.0) {
                near
            } else {
                d.warnings.push(format!("the views do not fit the sheet at {}: drawn at {}", near.1, auto.1));
                auto
            }
        }
    };
    d.scale = scale;
    d.scale_text = scale_text.to_string();
    let s = scale;
    // Columns: right view then front/top (first angle), or front/top then right view (third).
    let row_front = fh.max(rh) * s;
    let mid_w = fw.max(tw) * s;
    let main_w = rw * s + mid_w + gx;
    let total_h = row_front + th * s + gy;
    // The isometric goes where it comes out largest: the empty corner of the
    // grid (beside the top view, under or over the side view), or a column
    // right of the views. Its scale is the largest standard one that fits.
    let iso_fit = |w: f64, h: f64| SCALES.iter().find(|(k, _)| *k <= s && iw * k <= w - 2.0 * room && ih * k <= h - 8.0 - 2.0 * room).copied();
    let corner = if has_i && has_ortho { iso_fit(rw * s, th * s) } else { None };
    let column = if !has_i {
        None
    } else if has_ortho {
        iso_fit(area[0] - main_w - gap, (fy1 - rev_h).min(region[3]) - region[1] - 2.0 * margin)
    } else {
        iso_fit(area[0] + margin, area[1] + margin)
    };
    let in_corner = match (corner, column) {
        (Some(a), Some(b)) => a.0 >= b.0,
        (a, _) => a.is_some(),
    };
    // Centred across the region when nothing stands beside them.
    let x0 = if in_corner || !has_i { region[0] + margin + (area[0] - main_w).max(0.0) / 2.0 } else { region[0] + margin };
    let y0 = region[1] + margin + (area[1] - total_h).max(0.0) / 2.0;
    let (col_side, col_main) = match options.projection {
        Projection::First => (x0, x0 + rw * s + gx),
        Projection::Third => (x0 + mid_w + gx, x0),
    };
    let (row_top, row_main) = match options.projection {
        Projection::First => (y0, y0 + th * s + gy),
        Projection::Third => (y0 + row_front + gy, y0),
    };
    let column_x = if has_ortho { x0 + main_w + gap } else { region[0] + margin };
    let column_top = if has_ortho { (fy1 - rev_h).min(region[3]) - margin } else { region[3] - margin };
    let (iso_cell, iso_scale) = if in_corner { ([col_side, row_top, col_side + rw * s, row_top + th * s], corner) } else { ([column_x, region[1] + margin, region[2] - margin, column_top], column) };
    // Where each view's lower left lands (sheet mm), the views centred in their cells.
    let place = |v: &View, cell: [f64; 2], cw: f64, ch: f64, k: f64| [cell[0] + (cw - v.size()[0] * k) / 2.0 - v.min[0] * k, cell[1] + (ch - v.size()[1] * k) / 2.0 - v.min[1] * k];
    let at_front = place(&front, [col_main, row_main], mid_w, row_front, s);
    let at_top = place(&top, [col_main, row_top], mid_w, th * s, s);
    let at_right = place(&right, [col_side, row_main], rw * s, row_front, s);
    let geometry_start = d.strokes.len();
    let mut section_segments = vec![];
    let ortho = [(ViewName::Front, &front, at_front, has_f), (ViewName::Top, &top, at_top, has_t), (ViewName::Right, &right, at_right, has_r)];
    for (_, v, at, _) in ortho.iter().filter(|o| o.3) {
        for l in &v.lines {
            d.strokes.push(Stroke { pen: if l.hidden { Pen::Hidden } else { Pen::Visible }, points: l.points.iter().map(|p| [at[0] + p[0] * s, at[1] + p[1] * s]).collect() });
        }
    }
    for (name, v, at, _) in ortho.iter().filter(|o| o.3) {
        let (name, at) = (*name, *at);
        d.views.push(ViewFrame { view: name, at, scale: s, axes: name.axes(), bounds: [at[0] + v.min[0] * s, at[1] + v.min[1] * s, at[0] + v.max[0] * s, at[1] + v.max[1] * s] });
    }
    if let Some(c) = cut {
        // Hatch the cut faces (the half beyond the middle of a half section).
        let at_cut = match section_view {
            ViewName::Front => at_front,
            ViewName::Top => at_top,
            ViewName::Right => at_right,
        };
        let mesh = arena.mesh(shape, deflection)?;
        let segments: Vec<[[f64; 2]; 2]> = slice_along(&mesh.positions, &mesh.triangles, look, c + size3[look] * 1e-7, section_view.axes())
            .into_iter()
            .map(|[p, q]| [[at_cut[0] + p[0] * s, at_cut[1] + p[1] * s], [at_cut[0] + q[0] * s, at_cut[1] + q[1] * s]])
            .collect();
        let pattern = options.extra.hatch.clone().unwrap_or(Hatch { angle: 45.0, spacing: 2.5, cross: false });
        let middle = at_cut[0] + half_at * s;
        let mut lines = hatch_pattern(&segments, &pattern);
        if options.extra.half_section {
            lines = lines.into_iter().filter_map(|[a, b2]| clip_segment(a, b2, middle)).collect();
        }
        for [a, b2] in lines {
            d.strokes.push(Stroke { pen: Pen::Thin, points: vec![a, b2] });
        }
        if section_view == ViewName::Front {
            section_segments = segments;
        }
        // The cutting plane, marked on the view beside: thick ends, chain line, arrows the way it is looked at.
        let (on, at_on, vertical, toward) = match section_view {
            ViewName::Front => (has_t, at_top, false, [0.0, 1.0]),
            ViewName::Top => (has_f, at_front, false, [0.0, -1.0]),
            ViewName::Right => (has_f, at_front, true, [-1.0, 0.0]),
        };
        if on {
            let (along_lo, along_hi, pos) = if vertical { (at_on[1] + b[2] * s, at_on[1] + b[5] * s, at_on[0] + c * s) } else { (at_on[0] + b[0] * s, at_on[0] + b[3] * s, at_on[1] + c * s) };
            let pt = |a: f64| if vertical { [pos, a] } else { [a, pos] };
            let (xa, xb) = (along_lo - 8.0, along_hi + 8.0);
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![pt(xa + 5.0), pt(xb - 5.0)] });
            for (x, inward) in [(xa, 1.0), (xb, -1.0)] {
                d.strokes.push(Stroke { pen: Pen::Frame, points: vec![pt(x), pt(x + inward * 5.0)] });
                let foot = pt(x + inward * 1.5);
                let tip = [foot[0] + toward[0] * 7.0, foot[1] + toward[1] * 7.0];
                d.strokes.push(Stroke { pen: Pen::Thin, points: vec![foot, [foot[0] + toward[0] * 6.0, foot[1] + toward[1] * 6.0]] });
                arrow(&mut d, tip, toward);
                let letter = pt(x + inward * 1.5 - inward * 3.5);
                let lift = if vertical { [letter[0] + toward[0] * 3.0, letter[1] - 1.75] } else { [letter[0], letter[1] + toward[1] * 3.0 - if toward[1] < 0.0 { 3.5 } else { 0.0 }] };
                d.texts.push(Text { at: lift, size: 3.5, text: "A".into(), anchor: Anchor::Middle, angle: 0.0, bold: true });
            }
        }
    }
    // Break lines (ISO 128-2, narrow with a zigzag) where a broken view leaves a stretch out.
    for c in &brk.cuts {
        for (name, v, at, on) in &ortho {
            let Some(i) = name.axes().iter().position(|&a| a == c.axis).filter(|_| *on) else { continue };
            let (lo, hi) = (at[1 - i] + v.min[1 - i] * s - 3.0, at[1 - i] + v.max[1 - i] * s + 3.0);
            for pos in [c.at, c.at + c.gap] {
                let x = at[i] + pos * s;
                let mid = (lo + hi) / 2.0;
                let z = 1.6;
                let along = [lo, mid - z, mid - z / 2.0, mid + z / 2.0, mid + z, hi];
                let off = [0.0, 0.0, z, -z, 0.0, 0.0];
                let pts: Vec<[f64; 2]> = along.iter().zip(off).map(|(a, o)| if i == 0 { [x + o, *a] } else { [*a, x + o] }).collect();
                d.strokes.push(Stroke { pen: Pen::Thin, points: pts });
            }
        }
    }
    let hole_views: Vec<([f64; 2], usize)> = [(at_front, 1, has_f), (at_top, 2, has_t), (at_right, 0, has_r)].into_iter().filter(|v| v.2).map(|v| (v.0, v.1)).collect();
    holes(&mut d, arena, shape, deflection, s, hole_views);
    let geometry_end = d.strokes.len();
    if let Some((k, k_text)) = iso_scale {
        let at = place(&iso, [iso_cell[0] + room, iso_cell[1] + 8.0 + room], iso_cell[2] - iso_cell[0] - 2.0 * room, iso_cell[3] - iso_cell[1] - 8.0 - 2.0 * room, k);
        // Hidden lines are left out of the isometric.
        for l in iso.lines.iter().filter(|l| !l.hidden) {
            d.strokes.push(Stroke { pen: Pen::Visible, points: l.points.iter().map(|p| [at[0] + p[0] * k, at[1] + p[1] * k]).collect() });
        }
        let outline = [at[0] + iso.min[0] * k, at[1] + iso.min[1] * k, at[0] + iso.max[0] * k, at[1] + iso.max[1] * k];
        d.iso = Some(IsoFrame { at, scale: k, bounds: outline });
        let mut label_y = outline[1] - 6.0;
        if ballooned {
            balloons(&mut d, arena, exploded.as_deref().unwrap_or(parts), &title.parts, deflection, &iso, at, k, outline);
            // Under the ring of balloons.
            label_y = label_y.min(outline[1] - BALLOON_GAP - BALLOON_R - 6.0);
        }
        let name = if exploded.is_some() { "EXPLODED VIEW" } else { "ISOMETRIC" };
        let text = if k == s { name.to_string() } else { format!("{name} {k_text}") };
        d.texts.push(Text { at: [(outline[0] + outline[2]) / 2.0, label_y], size: 2.5, text, anchor: Anchor::Middle, angle: 0.0, bold: false });
    }
    // What the sheet keeps for its title block and parts list, notes and revisions.
    let reserved = vec![[fx1 - block[0] - 2.0, fy0, fx1, fy0 + stack_h], [fx0, fy0, fx0 + notes_w + 6.0, fy0 + notes_h], [fx1 - sheet::REV_W - 2.0, fy1 - rev_h - 2.0, fx1, fy1]];
    // The drawing's own dimensions of the part's features, clear of everything drawn so far.
    let mut done = autodim::Done::default();
    if let Some(f) = feats.as_ref().filter(|_| dimensioned) {
        let mut ink = annotate::Grid::new(d.width, d.height);
        let mut words = annotate::Grid::new(d.width, d.height);
        for st in &d.strokes {
            for w in st.points.windows(2) {
                ink.mark_line(w[0], w[1], 0.4);
            }
        }
        for t in &d.texts {
            ink.mark(text_box(t));
            words.mark(text_box(t));
        }
        // The title block, parts list, notes and revisions to come.
        for r in &reserved {
            ink.mark(*r);
        }
        let frame_inside = [fx0, fy0, fx1, fy1];
        let views: Vec<autodim::ViewIn> = ortho
            .iter()
            .filter(|o| o.3)
            .map(|(name, v, at, _)| {
                let fr = d.views.iter().find(|f| f.view == *name).map(|f| f.bounds).unwrap_or([0.0; 4]);
                autodim::ViewIn { name: *name, lines: &v.lines, at: *at, bounds: fr, section: if *name == section_view { cut } else { None } }
            })
            .collect();
        let cx = autodim::Ctx { views, s, bounds: b, feats: f, options, frame: frame_inside, breaks: brk };
        done = autodim::dimension(&mut d, &mut ink, &mut words, &cx);
    }
    // Overall dimensions: width over the front view, height beside it, depth
    // beside the top view (those the ordinates do not already give).
    let box_of = |v: &View, at: [f64; 2]| ([at[0] + v.min[0] * s, at[1] + v.min[1] * s], [at[0] + v.max[0] * s, at[1] + v.max[1] * s]);
    let (f0, f1) = box_of(&front, at_front);
    let (t0, t1) = box_of(&top, at_top);
    let above_front = match options.projection {
        Projection::First => true,  // the top view is below
        Projection::Third => false, // the top view is above: dimension below instead
    };
    let (r0, r1) = box_of(&right, at_right);
    let [want_x, want_y, want_z] = [!done.axes[0], !done.axes[1], !done.axes[2]];
    let size3 = [0, 1, 2].map(|k| brk.true_at(k, b[k + 3]) - brk.true_at(k, b[k]));
    if has_f {
        if want_x {
            if above_front {
                dim_h(&mut d, f0[0], f1[0], f1[1], f1[1] + 9.0, size3[0]);
            } else {
                dim_h(&mut d, f0[0], f1[0], f0[1], f0[1] - 9.0, size3[0]);
            }
        }
        if want_z {
            dim_v(&mut d, f0[1], f1[1], f1[0], f1[0] + 9.0, size3[2]);
        }
    } else {
        // Without the front view: the width over the top view, the height beside the side view.
        if has_t && want_x {
            dim_h(&mut d, t0[0], t1[0], t1[1], t1[1] + 9.0, size3[0]);
        }
        if has_r && want_z {
            dim_v(&mut d, r0[1], r1[1], r1[0], r1[0] + 9.0, size3[2]);
        }
    }
    if want_y {
        if has_t {
            // Clear of the cutting-plane arrows when there is a section.
            dim_v(&mut d, t0[1], t1[1], t1[0], t1[0] + if cut.is_some() && section_view == ViewName::Front { 23.0 } else { 9.0 }, size3[1]);
        } else if has_r {
            dim_h(&mut d, r0[0], r1[0], r1[1], r1[1] + 9.0, size3[1]);
        }
    }
    if cut.is_some() {
        let (lo, hi) = match section_view {
            ViewName::Front => (f0, f1),
            ViewName::Top => (t0, t1),
            ViewName::Right => (r0, r1),
        };
        let y = if section_view == ViewName::Front && !above_front { lo[1] - 16.0 } else { lo[1] - 7.0 };
        let _ = hi;
        d.texts.push(Text { at: [(lo[0] + hi[0]) / 2.0, y], size: 3.5, text: if options.extra.half_section { "HALF SECTION A–A" } else { "SECTION A–A" }.into(), anchor: Anchor::Middle, angle: 0.0, bold: true });
    }
    // The person's own dimensions.
    for (index, dim) in options.dimensions.iter().enumerate() {
        if dim.detail.is_some() {
            continue; // drawn with its detail view
        }
        let Some(frame) = d.views.iter().find(|v| v.view == dim.view).cloned() else {
            d.warnings.push(format!("a dimension is on the {:?} view, which this sheet leaves out", dim.view).to_lowercase());
            continue;
        };
        let on_sheet = |p: [f64; 2]| [frame.at[0] + p[0] * s, frame.at[1] + p[1] * s];
        user_dim(&mut d, index, dim, on_sheet(dim.a), on_sheet(dim.b));
    }

    sheet::title_block(&mut d, [fx1 - block[0], fy0], block, title, options);
    if list_h > 0.0 {
        parts_list(&mut d, [fx1 - block[0], fy0 + block[1]], block[0], &title.parts);
    }
    sheet::general_line(&mut d, [fx1 - block[0], fy0 + block[1] + list_h + 2.6], block[0], options);
    if rev_beside {
        sheet::revisions(&mut d, [fx0 + 5.0 + sheet::REV_W, fy0 + notes_h], &options.revisions);
        sheet::notes(&mut d, [fx0 + 5.0, fy0 + notes_h - table_h - 4.0], notes_w, &options.notes);
    } else {
        sheet::notes(&mut d, [fx0 + 5.0, fy0 + notes_h], notes_w, &options.notes);
        sheet::revisions(&mut d, [fx1, fy1], &options.revisions);
    }
    // Auxiliary views (looking along any direction), and area notes read off the faces.
    let mut aux = vec![];
    for a in &options.extra.aux {
        let l = (a.dir[0] * a.dir[0] + a.dir[1] * a.dir[1] + a.dir[2] * a.dir[2]).sqrt();
        if !(l > 1e-9) {
            d.warnings.push("an auxiliary view needs a direction to look along".into());
            continue;
        }
        let dir = [a.dir[0] / l, a.dir[1] / l, a.dir[2] / l];
        let up = a.up.unwrap_or(if dir[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [0.0, 1.0, 0.0] });
        let v = View::of(arena.hlr_view(shape, dir, up, deflection)?);
        let scale = match a.scale {
            Some(k) => SCALES.iter().copied().min_by(|x, y| (x.0.ln() - k.ln()).abs().total_cmp(&(y.0.ln() - k.ln()).abs())).unwrap_or((s, "")),
            None => (s, ""),
        };
        aux.push(annotate::AuxIn { lines: v.lines, min: v.min, max: v.max, scale, dir });
    }
    let mut notes = vec![];
    if !options.extra.areas.is_empty() {
        let faces = arena.face_info(shape)?;
        for a in &options.extra.areas {
            let look = section_look(a.view);
            let ax = a.view.axes();
            // Flat faces square to the way the view looks, under the point; the nearest the eye.
            let toward = if look == 1 { -1.0 } else { 1.0 };
            let face = faces
                .iter()
                .filter(|f| f.kind == de_occt::SurfaceKind::Plane && f.dir[look].abs() > 1.0 - 1e-6)
                .filter(|f| (0..2).all(|i| a.at[i] >= f.bounds[ax[i]] - 1e-6 && a.at[i] <= f.bounds[ax[i] + 3] + 1e-6))
                .max_by(|x, y| (x.point[look] * toward).total_cmp(&(y.point[look] * toward)));
            match face {
                Some(f) => {
                    let mut at = a.at;
                    // The arrow inside the face, not on its edge.
                    for i in 0..2 {
                        at[i] = at[i].clamp(f.bounds[ax[i]] + 1e-3, f.bounds[ax[i] + 3] - 1e-3);
                    }
                    notes.push(LeaderSpec { view: a.view, at, text: format!("A = {} mm²", length_text((f.area * 10.0).round() / 10.0)) });
                }
                None => d.warnings.push("no flat face faces that view under the area's point".into()),
            }
        }
    }
    annotate::annotate(&mut d, options, &annotate::Geometry { strokes: geometry_start..geometry_end, section: section_segments, frame: [fx0, fy0, fx1, fy1], reserved, aux, notes });
    d.quality = check::inspect(&d, [fx0, fy0, fx1, fy1]);
    Ok(d)
}

/// One of the person's dimensions, between two sheet points, and its mark.
fn user_dim(d: &mut Drawing, index: usize, dim: &DimSpec, pa: [f64; 2], pb: [f64; 2]) {
    let value = dim.value();
    if value <= 1e-9 {
        return;
    }
    let (strokes, texts, fills) = (d.strokes.len(), d.texts.len(), d.fills.len());
    dim_between(d, pa, pb, dim.kind, dim.offset, &annotate::label(value, dim.diameter, dim.tolerance.as_ref()));
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.strokes[strokes..].iter().flat_map(|s| &s.points).chain(d.fills[fills..].iter().flatten()) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    for t in &d.texts[texts..] {
        let r = text_box(t);
        b = [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])];
    }
    d.marks.push(DimMark { dimension: index, bounds: b });
}

/// How far each part slides along axis `k` in an exploded view: the long
/// parts through the middle (shafts, frames) stay; the others move out from
/// the middle, each clear of the parts already placed that it would pass
/// through (overlapping across the axis), `gap` apart. Parts on a shaft
/// slide along it, so the parts that stay do not push anything.
pub fn explode_shifts(boxes: &[[f64; 6]], k: usize, gap: f64) -> Vec<f64> {
    let n = boxes.len();
    let (lo, hi) = boxes.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| (a.min(x[k]), b.max(x[k + 3])));
    let mid = (lo + hi) / 2.0;
    let centre = |i: usize| (boxes[i][k] + boxes[i][k + 3]) / 2.0;
    let across = |i: usize, j: usize| (0..3).filter(|&a| a != k).all(|a| boxes[i][a] < boxes[j][a + 3] - 1e-6 && boxes[j][a] < boxes[i][a + 3] - 1e-6);
    let stays: Vec<bool> = (0..n).map(|i| boxes[i][k] <= mid + 1e-6 && boxes[i][k + 3] >= mid - 1e-6 && boxes[i][k + 3] - boxes[i][k] >= 0.5 * (hi - lo)).collect();
    let mut shift = vec![0.0; n];
    let mut placed: Vec<usize> = (0..n).filter(|&i| stays[i]).collect();
    // The far side, nearest the middle first, then the near side the same way.
    for up in [true, false] {
        let mut order: Vec<usize> = (0..n).filter(|&i| !stays[i] && (centre(i) >= mid) == up).collect();
        order.sort_by(|&a, &b| if up { centre(a).total_cmp(&centre(b)) } else { centre(b).total_cmp(&centre(a)) });
        for j in order {
            let mut s = 0.0f64;
            for &i in placed.iter().filter(|&&i| !stays[i] && across(i, j)) {
                if up && centre(i) <= centre(j) + 1e-9 {
                    s = s.max(boxes[i][k + 3] + shift[i] + gap - boxes[j][k]);
                } else if !up && centre(i) >= centre(j) - 1e-9 {
                    s = s.min(boxes[i][k] + shift[i] - gap - boxes[j][k + 3]);
                }
            }
            shift[j] = if up { s.max(0.0) } else { s.min(0.0) };
            placed.push(j);
        }
    }
    shift
}

/// Balloon radius and how far balloon centres stand off the view (sheet mm).
const BALLOON_R: f64 = 4.0;
const BALLOON_GAP: f64 = 10.0;

/// The title block's height (ISO 7200 fields in four rows).
const TITLE_H: f64 = 36.0;

/// Item balloons round the isometric view, one per row of the parts list:
/// each leader ends on the part where it shows widest, the balloons ring
/// the view in the order their parts lie round it so no two leaders cross.
#[allow(clippy::too_many_arguments)]
fn balloons(d: &mut Drawing, arena: &mut Arena, parts: &[(String, Shape)], rows: &[PartRow], deflection: f64, iso: &View, at: [f64; 2], k: f64, outline: [f64; 4]) {
    let (dir, up) = iso_frame();
    let shapes: Vec<Shape> = parts.iter().map(|p| p.1).collect();
    let seen = visible_points(arena, &shapes, deflection, dir, up, iso.min, iso.max);
    let mut items: Vec<(usize, [f64; 2])> = vec![];
    for (i, row) in rows.iter().enumerate() {
        let copies: Vec<usize> = (0..parts.len()).filter(|&j| parts[j].0 == row.name || row.members.contains(&parts[j].0)).collect();
        // The copy that shows most; one hidden behind the others gets its middle.
        let best = copies.iter().filter_map(|&j| seen[j]).max_by(|a, b| a.1.total_cmp(&b.1)).map(|v| v.0);
        let point = best.or_else(|| {
            let b = arena.bounds(parts[*copies.first()?].1).ok()?;
            Some(iso_point([(b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0, (b[2] + b[5]) / 2.0]))
        });
        if let Some(p) = point {
            items.push((i + 1, [at[0] + p[0] * k, at[1] + p[1] * k]));
        }
    }
    let anchors: Vec<[f64; 2]> = items.iter().map(|i| i.1).collect();
    let centres = balloon_ring(&anchors, outline, BALLOON_R, BALLOON_GAP);
    for ((item, a), c) in items.into_iter().zip(centres) {
        let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
        let len = dx.hypot(dy);
        if len > BALLOON_R {
            d.strokes.push(Stroke { pen: Pen::Thin, points: vec![a, [c[0] - dx / len * BALLOON_R, c[1] - dy / len * BALLOON_R]] });
        }
        // The leader ends in a dot on the part (ISO 128-22).
        d.fills.push((0..12).map(|i| circle_point(a, 0.75, i as f64 / 12.0)).collect());
        d.strokes.push(Stroke { pen: Pen::Thin, points: (0..=48).map(|i| circle_point(c, BALLOON_R, i as f64 / 48.0)).collect() });
        d.texts.push(Text { at: [c[0], c[1] - 1.75], size: 3.5, text: item.to_string(), anchor: Anchor::Middle, angle: 0.0, bold: false });
        d.balloons.push(Balloon { item, centre: c, radius: BALLOON_R, anchor: a });
    }
}

fn circle_point(c: [f64; 2], r: f64, turn: f64) -> [f64; 2] {
    let (s, co) = (turn * std::f64::consts::TAU).sin_cos();
    [c[0] + r * co, c[1] + r * s]
}

/// Each shape's most visible point in a view, and how far it is from the
/// edge of what shows of it (view mm); None when nothing of it shows. A depth
/// buffer over the view (the meshes projected as the hidden-line view
/// projects them), then per shape the pixel deepest inside one of its faces
/// as they show (so a leader lands on a face, not on the edge between two).
fn visible_points(arena: &mut Arena, shapes: &[Shape], deflection: f64, dir: [f64; 3], up: [f64; 3], min: [f64; 2], max: [f64; 2]) -> Vec<Option<([f64; 2], f64)>> {
    let px = ((max[0] - min[0]).max(max[1] - min[1]) / 360.0).max(1e-6);
    let origin = [min[0] - 2.0 * px, min[1] - 2.0 * px];
    let (nx, ny) = (((max[0] - min[0]) / px) as usize + 5, ((max[1] - min[1]) / px) as usize + 5);
    let mut depth = vec![f64::NEG_INFINITY; nx * ny];
    let mut owner = vec![usize::MAX; nx * ny];
    let mut normal = vec![[0.0f64; 3]; nx * ny];
    for (i, &s) in shapes.iter().enumerate() {
        let Ok(mesh) = arena.mesh(s, deflection) else { continue };
        let v: Vec<[f64; 3]> = mesh
            .positions
            .chunks_exact(3)
            .map(|p| {
                let q = view_point(dir, up, [p[0], p[1], p[2]]);
                [(q[0] - origin[0]) / px, (q[1] - origin[1]) / px, q[2]]
            })
            .collect();
        for t in mesh.triangles.chunks_exact(3) {
            let [a, b, c] = [v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]];
            let corner = |k: u32| [mesh.positions[k as usize * 3], mesh.positions[k as usize * 3 + 1], mesh.positions[k as usize * 3 + 2]];
            let (p0, p1, p2) = (corner(t[0]), corner(t[1]), corner(t[2]));
            let facing = cross([p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]], [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]]);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-12 {
                continue; // seen edge on
            }
            let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
            let x1 = (a[0].max(b[0]).max(c[0]).ceil().max(0.0) as usize).min(nx - 1);
            let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
            let y1 = (a[1].max(b[1]).max(c[1]).ceil().max(0.0) as usize).min(ny - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = [x as f64 + 0.5, y as f64 + 0.5];
                    // Barycentric weights of the pixel centre.
                    let wa = ((b[0] - p[0]) * (c[1] - p[1]) - (b[1] - p[1]) * (c[0] - p[0])) / area;
                    let wb = ((c[0] - p[0]) * (a[1] - p[1]) - (c[1] - p[1]) * (a[0] - p[0])) / area;
                    let wc = 1.0 - wa - wb;
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    let z = wa * a[2] + wb * b[2] + wc * c[2];
                    let k = y * nx + x;
                    if z > depth[k] {
                        depth[k] = z;
                        owner[k] = i;
                        normal[k] = facing;
                    }
                }
            }
        }
    }
    for n in normal.iter_mut().filter(|n| n.iter().any(|v| *v != 0.0)) {
        *n = unit(*n);
    }
    // Faces meet where the surface turns sharply (more than 30°); a curved
    // face's facets turn far less from one to the next.
    let other = |a: usize, b: usize| owner[a] != owner[b] || normal[a][0] * normal[b][0] + normal[a][1] * normal[b][1] + normal[a][2] * normal[b][2] < 0.866;
    // Distance (pixels) to the nearest pixel showing another face: a
    // two-pass chamfer transform from the edges of every region at once.
    let mut dist = vec![f64::INFINITY; nx * ny];
    for y in 0..ny {
        for x in 0..nx {
            let k = y * nx + x;
            if x == 0 || y == 0 || x == nx - 1 || y == ny - 1 || [k - 1, k + 1, k - nx, k + nx].iter().any(|&n| other(n, k)) {
                dist[k] = 0.0;
            }
        }
    }
    let diag = std::f64::consts::SQRT_2;
    for y in 1..ny - 1 {
        for x in 1..nx - 1 {
            let k = y * nx + x;
            dist[k] = dist[k].min(dist[k - 1] + 1.0).min(dist[k - nx] + 1.0).min(dist[k - nx - 1] + diag).min(dist[k - nx + 1] + diag);
        }
    }
    for y in (1..ny - 1).rev() {
        for x in (1..nx - 1).rev() {
            let k = y * nx + x;
            dist[k] = dist[k].min(dist[k + 1] + 1.0).min(dist[k + nx] + 1.0).min(dist[k + nx + 1] + diag).min(dist[k + nx - 1] + diag);
        }
    }
    let mut best: Vec<Option<([f64; 2], f64)>> = vec![None; shapes.len()];
    for (k, &o) in owner.iter().enumerate() {
        if o == usize::MAX || best[o].is_some_and(|b| b.1 >= dist[k] * px) {
            continue;
        }
        let (x, y) = (k % nx, k / nx);
        best[o] = Some(([origin[0] + (x as f64 + 0.5) * px, origin[1] + (y as f64 + 0.5) * px], dist[k] * px));
    }
    best
}

/// Where the balloons for `anchors` go: on a ring `gap` outside `outline`
/// ([x0, y0, x1, y1]), each first where the ray from the view's middle
/// through its anchor meets the ring, then spread apart along it (keeping
/// their order round it), and any two whose leaders still cross swapped.
pub fn balloon_ring(anchors: &[[f64; 2]], outline: [f64; 4], r: f64, gap: f64) -> Vec<[f64; 2]> {
    let n = anchors.len();
    if n == 0 {
        return vec![];
    }
    let ring = [outline[0] - gap, outline[1] - gap, outline[2] + gap, outline[3] + gap];
    let (w, h) = (ring[2] - ring[0], ring[3] - ring[1]);
    let perimeter = 2.0 * (w + h);
    let centre = [(ring[0] + ring[2]) / 2.0, (ring[1] + ring[3]) / 2.0];
    // A point round the ring, by its distance anticlockwise from the lower left corner.
    let point = |t: f64| {
        let t = t.rem_euclid(perimeter);
        if t < w {
            [ring[0] + t, ring[1]]
        } else if t < w + h {
            [ring[2], ring[1] + t - w]
        } else if t < 2.0 * w + h {
            [ring[2] - (t - w - h), ring[3]]
        } else {
            [ring[0], ring[3] - (t - 2.0 * w - h)]
        }
    };
    let ideal: Vec<f64> = anchors
        .iter()
        .map(|a| {
            let (mut dx, mut dy) = (a[0] - centre[0], a[1] - centre[1]);
            if dx.hypot(dy) < 1e-9 {
                (dx, dy) = (-1.0, 1.0);
            }
            // Out along the ray to whichever side it meets first.
            let (sx, sy) = (w / 2.0 / dx.abs().max(1e-12), h / 2.0 / dy.abs().max(1e-12));
            if sx <= sy {
                let y = centre[1] + dy * sx;
                if dx > 0.0 { w + y - ring[1] } else { 2.0 * w + h + ring[3] - y }
            } else {
                let x = centre[0] + dx * sy;
                if dy < 0.0 { x - ring[0] } else { w + h + ring[2] - x }
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| ideal[a].total_cmp(&ideal[b]));
    // Spread them along the ring; round a corner the straight distance is
    // shorter than the distance along, so widen the spacing until it is enough.
    let mut spacing = 2.0 * r + 2.0;
    let mut t = ideal.clone();
    for _ in 0..12 {
        let space = spacing.min(perimeter / n as f64);
        t = ideal.clone();
        if n > 1 {
            for _ in 0..1000 {
                let mut moved = false;
                for j in 0..n {
                    let (a, b) = (order[j], order[(j + 1) % n]);
                    let between = if j + 1 == n { t[b] + perimeter - t[a] } else { t[b] - t[a] };
                    if between < space - 1e-6 {
                        let push = (space - between) / 2.0 + 1e-7;
                        t[a] -= push;
                        t[b] += push;
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        let pts: Vec<[f64; 2]> = t.iter().map(|&v| point(v)).collect();
        let clear = (0..n).all(|i| (i + 1..n).all(|j| (pts[i][0] - pts[j][0]).hypot(pts[i][1] - pts[j][1]) >= 2.0 * r + 1.0));
        if clear || space < spacing {
            break;
        }
        spacing *= 1.15;
    }
    let mut out: Vec<[f64; 2]> = t.iter().map(|&v| point(v)).collect();
    // Swapping the ends of two crossing leaders always shortens them, so this ends.
    for _ in 0..100 {
        let mut swapped = false;
        for i in 0..n {
            for j in i + 1..n {
                if segments_cross(anchors[i], out[i], anchors[j], out[j]) {
                    out.swap(i, j);
                    swapped = true;
                }
            }
        }
        if !swapped {
            break;
        }
    }
    out
}

/// Whether segments pq and rs cross (properly, not just touching).
pub fn segments_cross(p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]) -> bool {
    let side = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let (d1, d2, d3, d4) = (side(p, q, r), side(p, q, s), side(r, s, p), side(r, s, q));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Holes and round bosses facing a view: centre marks in that view, centre
/// lines along them in the others (their callouts are the auto dimensions').
/// `views` are each view's placement and the axis it looks along (0 x, 1 y, 2 z).
fn holes(d: &mut Drawing, arena: &mut Arena, shape: Shape, deflection: f64, s: f64, views: Vec<([f64; 2], usize)>) {
    let Ok(edges) = arena.edges(shape, deflection) else { return };
    // Full circles only (an arc is a rounded edge, not a hole).
    let circles: Vec<([f64; 3], [f64; 3], f64)> = edges
        .iter()
        .filter_map(|e| match e.kind {
            de_occt::EdgeKind::Circle { center, axis, radius } => {
                let (a, b) = (e.points.first()?, e.points.last()?);
                (e.points.len() > 2 && (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]) < 1e-6).then_some((center, axis, radius))
            }
            _ => None,
        })
        .collect();
    // A view's sheet coordinates of a model point.
    let project = |p: [f64; 3], axis: usize| -> [f64; 2] {
        match axis {
            1 => [p[0], p[2]], // front: x right, z up
            2 => [p[0], p[1]], // top: x right, y up
            _ => [p[1], p[2]], // right: y right, z up
        }
    };
    for (at, axis) in views {
        let mut seen: Vec<([f64; 2], f64)> = vec![];
        for (c, ax, r) in &circles {
            if ax[axis].abs() < 0.999 {
                continue;
            }
            let p = project(*c, axis);
            // Too small to read at this scale (a chamfer's sliver): left out.
            if r * s < 1.0 {
                continue;
            }
            // One mark per centre, as wide as its widest circle (a counterbore's).
            match seen.iter_mut().find(|(q, _)| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-4) {
                Some(m) => m.1 = m.1.max(*r),
                None => seen.push((p, *r)),
            }
        }
        for (p, r) in &seen {
            let c = [at[0] + p[0] * s, at[1] + p[1] * s];
            let reach = r * s + 2.5;
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![[c[0] - reach, c[1]], [c[0] + reach, c[1]]] });
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![[c[0], c[1] - reach], [c[0], c[1] + reach]] });
        }
        // Holes seen from the side: a centre line along the axis, end to end.
        let along: Vec<(usize, f64)> = (0..3).filter(|k| *k != axis).map(|k| (k, 0.0)).collect();
        let mut runs: Vec<(usize, [f64; 2], f64, f64, f64)> = vec![]; // (axis k, centre across, from, to, r)
        for (c, ax, r) in &circles {
            let Some(&(k, _)) = along.iter().find(|(k, _)| ax[*k].abs() > 0.999) else { continue };
            let p = project(*c, axis);
            // In view coordinates, the axis runs along x when k is the view's right, else along y.
            let horizontal = matches!((axis, k), (1, 0) | (2, 0) | (0, 1));
            let (t, across) = if horizontal { (p[0], [p[1], 0.0]) } else { (p[1], [p[0], 0.0]) };
            match runs.iter_mut().find(|(kk, a, _, _, _)| *kk == k && (a[0] - across[0]).abs() < 1e-4) {
                Some(run) => {
                    run.2 = run.2.min(t);
                    run.3 = run.3.max(t);
                    run.4 = run.4.max(*r);
                }
                None => runs.push((k, across, t, t, *r)),
            }
        }
        for (k, across, from, to, _) in runs {
            if to - from < 1e-6 {
                continue;
            }
            let horizontal = matches!((axis, k), (1, 0) | (2, 0) | (0, 1));
            let (a, b) = if horizontal { ([from, across[0]], [to, across[0]]) } else { ([across[0], from], [across[0], to]) };
            let (a, b) = ([at[0] + a[0] * s, at[1] + a[1] * s], [at[0] + b[0] * s, at[1] + b[1] * s]);
            let dir = if horizontal { [1.0, 0.0] } else { [0.0, 1.0] };
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![[a[0] - dir[0] * 2.5, a[1] - dir[1] * 2.5], [b[0] + dir[0] * 2.5, b[1] + dir[1] * 2.5]] });
        }
    }
}

/// A text's box on the sheet (x0, y0, x1, y1), for keeping labels apart.
fn text_box(t: &Text) -> [f64; 4] {
    let w = text_width(t);
    let (x0, x1) = match t.anchor {
        Anchor::Start => (t.at[0], t.at[0] + w),
        Anchor::Middle => (t.at[0] - w / 2.0, t.at[0] + w / 2.0),
        Anchor::End => (t.at[0] - w, t.at[0]),
    };
    if t.angle.abs() > 45.0 {
        // Turned upright: the box turns with it (about its anchor).
        let (cx, cy) = (t.at[0], t.at[1]);
        return [cx - t.size * 1.2, cy - (cx - x0), cx + 0.2, cy + (x1 - cx)];
    }
    [x0, t.at[1] - t.size * 0.25, x1, t.at[1] + t.size * 1.2]
}

fn arrow(d: &mut Drawing, tip: [f64; 2], dir: [f64; 2]) {
    // A filled arrowhead 3 mm long, 1 mm wide (ISO 129).
    let (l, half) = (3.0, 0.5);
    let n = [-dir[1], dir[0]];
    let back = [tip[0] - dir[0] * l, tip[1] - dir[1] * l];
    d.fills.push(vec![tip, [back[0] + n[0] * half, back[1] + n[1] * half], [back[0] - n[0] * half, back[1] - n[1] * half]]);
}

/// A horizontal dimension from x0 to x1, measured at `from_y`, its line at `y`
/// (a short one with its arrows and value outside, as any other).
fn dim_h(d: &mut Drawing, x0: f64, x1: f64, from_y: f64, y: f64, value: f64) {
    dim_between(d, [x0, from_y], [x1, from_y], DimKind::Horizontal, y - from_y, &annotate::label(value, false, None));
}

/// A vertical dimension from y0 to y1, measured at `from_x`, its line at `x`;
/// read from the right (ISO 129): the text runs up the page, left of the line.
fn dim_v(d: &mut Drawing, y0: f64, y1: f64, from_x: f64, x: f64, value: f64) {
    dim_between(d, [from_x, y0], [from_x, y1], DimKind::Vertical, from_x - x, &annotate::label(value, false, None));
}

/// The axis a section through a view is square to (the axis the view looks along).
fn section_look(v: ViewName) -> usize {
    match v {
        ViewName::Front => 1,
        ViewName::Top => 2,
        ViewName::Right => 0,
    }
}

/// Where a closed mesh crosses the plane square to axis `k` at `at`:
/// segments in the coordinates of a view's two axes.
fn slice_along(positions: &[f64], triangles: &[u32], k: usize, at: f64, axes: [usize; 2]) -> Vec<[[f64; 2]; 2]> {
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i], positions[i + 1], positions[i + 2]]
    };
    let mut out = vec![];
    for t in triangles.chunks_exact(3) {
        let v = [p(t[0]), p(t[1]), p(t[2])];
        let mut hits = vec![];
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let (da, db) = (v[a][k] - at, v[b][k] - at);
            if (da < 0.0) != (db < 0.0) {
                let f = da / (da - db);
                hits.push([v[a][axes[0]] + f * (v[b][axes[0]] - v[a][axes[0]]), v[a][axes[1]] + f * (v[b][axes[1]] - v[a][axes[1]])]);
            }
        }
        if hits.len() == 2 {
            out.push([hits[0], hits[1]]);
        }
    }
    out
}

/// The parts of view lines right of `x` (`right`) or left of it.
fn clip_lines(lines: &[ViewLine], x: f64, right: bool) -> Vec<ViewLine> {
    let mut out = vec![];
    for l in lines {
        let mut cur: Vec<[f64; 2]> = vec![];
        let keep = |p: [f64; 2]| if right { p[0] >= x } else { p[0] <= x };
        for w in l.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            match (keep(a), keep(b)) {
                (true, true) => {
                    if cur.is_empty() {
                        cur.push(a);
                    }
                    cur.push(b);
                }
                (true, false) | (false, true) => {
                    let t = (x - a[0]) / (b[0] - a[0]);
                    let m = [x, a[1] + t * (b[1] - a[1])];
                    if keep(a) {
                        if cur.is_empty() {
                            cur.push(a);
                        }
                        cur.push(m);
                        out.push(ViewLine { hidden: l.hidden, outline: l.outline, points: std::mem::take(&mut cur) });
                    } else {
                        cur = vec![m, b];
                    }
                }
                (false, false) => {
                    if cur.len() >= 2 {
                        out.push(ViewLine { hidden: l.hidden, outline: l.outline, points: std::mem::take(&mut cur) });
                    }
                    cur.clear();
                }
            }
        }
        if cur.len() >= 2 {
            out.push(ViewLine { hidden: l.hidden, outline: l.outline, points: cur });
        }
    }
    out
}

/// The part of a segment right of `x`.
fn clip_segment(a: [f64; 2], b: [f64; 2], x: f64) -> Option<[[f64; 2]; 2]> {
    if a[0] < x && b[0] < x {
        return None;
    }
    if a[0] >= x && b[0] >= x {
        return Some([a, b]);
    }
    let t = (x - a[0]) / (b[0] - a[0]);
    let m = [x, a[1] + t * (b[1] - a[1])];
    Some(if a[0] >= x { [a, m] } else { [m, b] })
}

/// Hatching in a pattern: the 45° lines turned to the pattern's angle, crossed if asked.
fn hatch_pattern(segments: &[[[f64; 2]; 2]], h: &Hatch) -> Vec<[[f64; 2]; 2]> {
    let spacing = h.spacing.clamp(0.5, 20.0);
    let mut out = vec![];
    let angles: Vec<f64> = if h.cross { vec![h.angle, h.angle + 90.0] } else { vec![h.angle] };
    for angle in angles {
        // Turn the outline so the wanted angle becomes 45°, hatch, turn back.
        let turn = (45.0 - angle).to_radians();
        let (sn, cs) = turn.sin_cos();
        let rot = |p: [f64; 2], s: f64| [p[0] * cs - p[1] * s, p[0] * s + p[1] * cs];
        let turned: Vec<[[f64; 2]; 2]> = segments.iter().map(|[a, b]| [rot(*a, sn), rot(*b, sn)]).collect();
        for [a, b] in hatch_lines(&turned, spacing) {
            out.push([rot(a, -sn), rot(b, -sn)]);
        }
    }
    out
}

fn hatch_lines(segments: &[[[f64; 2]; 2]], spacing: f64) -> Vec<[[f64; 2]; 2]> {
    let mut out = vec![];
    if segments.is_empty() {
        return out;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in segments.iter().flatten() {
        lo = lo.min(p[0] - p[1]);
        hi = hi.max(p[0] - p[1]);
    }
    let step = spacing * std::f64::consts::SQRT_2;
    // Offset off the grid so lines do not pass exactly through corners.
    let mut c = lo + step * 0.437;
    while c < hi {
        let mut cross: Vec<[f64; 2]> = segments
            .iter()
            .filter_map(|[p, q]| {
                let (fp, fq) = (p[0] - p[1] - c, q[0] - q[1] - c);
                if (fp < 0.0) == (fq < 0.0) {
                    return None;
                }
                let k = fp / (fp - fq);
                Some([p[0] + k * (q[0] - p[0]), p[1] + k * (q[1] - p[1])])
            })
            .collect();
        cross.sort_by(|a, b| (a[0] + a[1]).total_cmp(&(b[0] + b[1])));
        for pair in cross.chunks_exact(2) {
            out.push([pair[0], pair[1]]);
        }
        c += step;
    }
    out
}

/// A dimension between two sheet points, its line `offset` mm off them.
fn dim_between(d: &mut Drawing, pa: [f64; 2], pb: [f64; 2], kind: DimKind, offset: f64, label: &annotate::Label) {
    let (u, n) = match kind {
        DimKind::Horizontal => ([1.0, 0.0], [0.0, 1.0]),
        DimKind::Vertical => ([0.0, 1.0], [-1.0, 0.0]),
        DimKind::Aligned => {
            let l = (pb[0] - pa[0]).hypot(pb[1] - pa[1]).max(1e-9);
            let u = [(pb[0] - pa[0]) / l, (pb[1] - pa[1]) / l];
            (u, [-u[1], u[0]])
        }
    };
    let dot = |p: [f64; 2], v: [f64; 2]| p[0] * v[0] + p[1] * v[1];
    // The line stands `offset` beyond the farther point (along n).
    let level = if offset >= 0.0 { dot(pa, n).max(dot(pb, n)) + offset } else { dot(pa, n).min(dot(pb, n)) + offset };
    let side = offset.signum();
    let foot = |p: [f64; 2]| {
        let k = level - dot(p, n);
        [p[0] + n[0] * k, p[1] + n[1] * k]
    };
    let (ea, eb) = (foot(pa), foot(pb));
    for (p, e) in [(pa, ea), (pb, eb)] {
        if (dot(e, n) - dot(p, n)).abs() > 1.5 {
            d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[p[0] + n[0] * side, p[1] + n[1] * side], [e[0] + n[0] * side * 2.0, e[1] + n[1] * side * 2.0]] });
        }
    }
    let along = if dot(eb, u) >= dot(ea, u) { u } else { [-u[0], -u[1]] };
    let length = (eb[0] - ea[0]).hypot(eb[1] - ea[1]);
    if length >= 7.0 {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![ea, eb] });
        arrow(d, ea, [-along[0], -along[1]]);
        arrow(d, eb, along);
    } else {
        // Too short for arrows inside: they point in from outside (ISO 129-1).
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[ea[0] - along[0] * 5.0, ea[1] - along[1] * 5.0], [eb[0] + along[0] * 5.0, eb[1] + along[1] * 5.0]] });
        arrow(d, ea, along);
        arrow(d, eb, [-along[0], -along[1]]);
    }
    // Text above the line, upright or reading from the right (ISO 129).
    let mut angle = along[1].atan2(along[0]).to_degrees();
    if angle > 90.0 + 1e-6 {
        angle -= 180.0;
    } else if angle <= -90.0 + 1e-6 {
        angle += 180.0;
    }
    let up = [-(angle.to_radians().sin()), angle.to_radians().cos()];
    let mid = [(ea[0] + eb[0]) / 2.0 + up[0] * 1.2, (ea[1] + eb[1]) / 2.0 + up[1] * 1.2];
    let width = annotate::label_width(label, 3.5);
    if width + 2.0 <= length {
        annotate::place_label(d, label, mid, Anchor::Middle, angle, 3.5);
    } else {
        // No room between the extension lines: the value goes beyond the
        // end it reads towards, over the dimension line drawn on under it.
        let read = [angle.to_radians().cos(), angle.to_radians().sin()];
        let end = if dot(eb, read) >= dot(ea, read) { eb } else { ea };
        let reach = if length >= 7.0 { 1.5 } else { 5.0 };
        let start = [end[0] + read[0] * reach, end[1] + read[1] * reach];
        let tail = [start[0] + read[0] * (width + 1.0), start[1] + read[1] * (width + 1.0)];
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![end, tail] });
        annotate::place_label(d, label, [start[0] + up[0] * 1.2, start[1] + up[1] * 1.2], Anchor::Start, angle, 3.5);
    }
}

/// The parts list on top of the title block, numbered from the bottom up (ISO 7573).
fn parts_list(d: &mut Drawing, at: [f64; 2], width: f64, rows: &[PartRow]) {
    let [x, y] = at;
    let row_h = 6.0;
    let h = row_h * (rows.len() + 1) as f64;
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[x, y], [x + width, y], [x + width, y + h], [x, y + h], [x, y]] });
    let edges = [0.0, 0.07, 0.45, 0.52, 0.74, 0.87, 1.0].map(|c| x + c * width);
    for c in &edges[1..6] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[*c, y], [*c, y + h]] });
    }
    for r in 1..=rows.len() {
        let ry = y + r as f64 * row_h;
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, ry], [x + width, ry]] });
    }
    let process = |p: &str| match p {
        "" => "—".to_string(),
        "buy" => "Bought".to_string(),
        "3d_print" => "3D print".to_string(),
        "cnc_mill" => "CNC mill".to_string(),
        "cnc_router" => "CNC router".to_string(),
        other => {
            let t = other.replace('_', " ");
            t[..1].to_uppercase() + &t[1..]
        }
    };
    let cell = |d: &mut Drawing, r: usize, values: [&str; 6], bold: bool| {
        let ry = y + r as f64 * row_h + 1.9;
        for (k, v) in values.iter().enumerate() {
            let room = edges[k + 1] - edges[k] - 2.4;
            let base = if bold { 2.0 } else { 2.5 };
            let t = Text { at: [0.0; 2], size: base, text: v.to_string(), anchor: Anchor::Start, angle: 0.0, bold };
            let size = if text_width(&t) > room { (base * room / text_width(&t)).max(1.8) } else { base };
            let centred = k == 0 || k == 2;
            let (cx, anchor) = if centred { ((edges[k] + edges[k + 1]) / 2.0, Anchor::Middle) } else { (edges[k] + 1.2, Anchor::Start) };
            d.texts.push(Text { at: [cx, ry], size, text: v.to_string(), anchor, angle: 0.0, bold });
        }
    };
    // The header is the top row.
    cell(d, rows.len(), ["ITEM", "NAME", "QTY", "MATERIAL", "PROCESS", "MASS EACH"], true);
    for (i, row) in rows.iter().enumerate() {
        cell(d, i, [&(i + 1).to_string(), &row.name, &row.quantity.to_string(), &row.material, &process(&row.process), &row.mass], false);
    }
}

fn sheet_name(width: f64) -> Sheet {
    Sheet::ALL.into_iter().find(|s| (s.size()[0] - width).abs() < 1.0).unwrap_or_default()
}

/// The projection symbol (ISO 128-30): a truncated cone and its end view.
fn projection_symbol(d: &mut Drawing, c: [f64; 2], projection: Projection) {
    let (h, small, len) = (3.2, 1.6, 6.0);
    // The cone's side view: narrow end on the left.
    let (cone_x, circles_x) = match projection {
        Projection::First => (c[0] - 5.5, c[0] + 4.5),
        Projection::Third => (c[0] - 1.5, c[0] - 6.5),
    };
    d.strokes.push(Stroke {
        pen: Pen::Thin,
        points: vec![[cone_x, c[1] - small], [cone_x + len, c[1] - h], [cone_x + len, c[1] + h], [cone_x, c[1] + small], [cone_x, c[1] - small]],
    });
    for r in [h, small] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: (0..=48).map(|i| (i as f64) / 48.0 * std::f64::consts::TAU).map(|a| [circles_x + r * a.cos(), c[1] + r * a.sin()]).collect() });
    }
}

// ---- Writers ------------------------------------------------------------------

const PT: f64 = 72.0 / 25.4; // points per millimetre

/// Text in PDF's WinAnsi encoding (the standard fonts), escaped for a string.
fn pdf_text(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        let byte: u32 = match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
                continue;
            }
            '·' => 0xB7,
            '°' => 0xB0,
            '×' => 0xD7,
            '±' => 0xB1,
            'Ø' | '⌀' => 0xD8,
            '–' | '−' => 0x96,
            '—' => 0x97,
            '“' => 0x93,
            '”' => 0x94,
            '’' => 0x92,
            '…' => 0x85,
            '‰' => 0x89,
            c if (c as u32) < 0x80 => {
                out.push(c);
                continue;
            }
            c if (c as u32) < 0x100 => c as u32,
            _ => '?' as u32,
        };
        let _ = write!(out, "\\{byte:03o}");
    }
    out
}

/// Helvetica and Helvetica-Bold advance widths (1/1000 em) for ' ' to '~' (the AFM metrics).
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Width of Helvetica text (mm), for anchoring and keeping labels apart.
fn text_width(t: &Text) -> f64 {
    let table = if t.bold { &HELVETICA_BOLD } else { &HELVETICA };
    let em: u32 = t
        .text
        .chars()
        .map(|c| match c {
            ' '..='~' => table[c as usize - 32] as u32,
            '·' | '’' => 278,
            '°' => 400,
            '×' | '±' => 584,
            'Ø' | '⌀' => 778,
            '–' | '−' => 556,
            '—' | '…' => 1000,
            // Drawn callout symbols (counterbore, countersink, depth).
            c if annotate::GLYPHS.contains(&c) => (annotate::glyph_width(0.718) * 1000.0) as u32,
            _ => 556,
        })
        .sum();
    // The size is the cap height; Helvetica's caps are 0.718 em.
    em as f64 / 1000.0 * t.size / 0.718
}

/// The drawing as a one-page vector PDF.
pub fn pdf(d: &Drawing) -> Vec<u8> {
    pdf_pages(std::slice::from_ref(d))
}

/// Sheets as one vector PDF, a page each (each page its sheet's size).
pub fn pdf_pages(sheets: &[Drawing]) -> Vec<u8> {
    // 1 catalogue, 2 page tree, 3 and 4 the fonts, then a page and its content per sheet.
    let mut objects = vec![String::new(), String::new(), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_string()];
    let mut kids = vec![];
    for d in sheets {
        let content = pdf_content(d);
        let page = objects.len() + 1;
        kids.push(format!("{page} 0 R"));
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>", d.width * PT, d.height * PT, page + 1));
        objects.push(format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content));
    }
    objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".to_string();
    objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), sheets.len());
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for o in offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = write!(tail, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
    out.extend(tail.as_bytes());
    out
}

/// A sheet's page content: strokes, fills and text in PDF operators.
fn pdf_content(d: &Drawing) -> String {
    let mut content = String::new();
    let _ = writeln!(content, "1 J 1 j");
    for st in &d.strokes {
        if st.points.len() < 2 {
            continue;
        }
        let _ = write!(content, "{:.3} w ", st.pen.width() * PT);
        if st.pen == Pen::Hidden {
            let _ = write!(content, "[{:.2} {:.2}] 0 d ", 3.0 * PT, 1.5 * PT);
        } else if st.pen == Pen::Center {
            let _ = write!(content, "[{:.2} {:.2} {:.2} {:.2}] 0 d ", 8.0 * PT, 1.5 * PT, 0.5 * PT, 1.5 * PT);
        } else {
            content.push_str("[] 0 d ");
        }
        for (i, p) in st.points.iter().enumerate() {
            let _ = write!(content, "{:.3} {:.3} {} ", p[0] * PT, p[1] * PT, if i == 0 { "m" } else { "l" });
        }
        content.push_str("S\n");
    }
    for f in &d.fills {
        for (i, p) in f.iter().enumerate() {
            let _ = write!(content, "{:.3} {:.3} {} ", p[0] * PT, p[1] * PT, if i == 0 { "m" } else { "l" });
        }
        content.push_str("h f\n");
    }
    for t in &d.texts {
        // Font size from cap height (Helvetica caps are 0.718 em).
        let size = t.size / 0.718 * PT;
        let shift = match t.anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => -text_width(t) / 2.0,
            Anchor::End => -text_width(t),
        };
        let (sn, cs) = t.angle.to_radians().sin_cos();
        let (x, y) = (t.at[0] + shift * cs, t.at[1] + shift * sn);
        let _ = writeln!(content, "BT /{} {:.2} Tf {:.4} {:.4} {:.4} {:.4} {:.3} {:.3} Tm ({}) Tj ET", if t.bold { "F2" } else { "F1" }, size, cs, sn, -sn, cs, x * PT, y * PT, pdf_text(&t.text));
    }
    content
}

/// Files in a zip archive, stored as they are (no compression): one DXF per sheet.
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let crc_table: Vec<u32> = (0..256u32)
        .map(|n| {
            let mut c = n;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            c
        })
        .collect();
    let crc = |data: &[u8]| !data.iter().fold(!0u32, |c, &b| crc_table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8));
    let mut out = vec![];
    let mut central = vec![];
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend(x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_le_bytes());
    for (name, data) in files {
        let (c, size, offset) = (crc(data), data.len() as u32, out.len() as u32);
        // Local header: version 2.0, UTF-8 names, stored, a fixed 1980 timestamp.
        le32(&mut out, 0x0403_4B50);
        for v in [20, 0x0800, 0, 0, 0x21] {
            le16(&mut out, v);
        }
        for v in [c, size, size] {
            le32(&mut out, v);
        }
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend(name.as_bytes());
        out.extend(data);
        le32(&mut central, 0x0201_4B50);
        for v in [20, 20, 0x0800, 0, 0, 0x21] {
            le16(&mut central, v);
        }
        for v in [c, size, size] {
            le32(&mut central, v);
        }
        for v in [name.len() as u16, 0, 0, 0, 0] {
            le16(&mut central, v);
        }
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend(name.as_bytes());
    }
    let (start, len) = (out.len() as u32, central.len() as u32);
    out.extend(&central);
    le32(&mut out, 0x0605_4B50);
    for v in [0, 0, files.len() as u16, files.len() as u16] {
        le16(&mut out, v);
    }
    le32(&mut out, len);
    le32(&mut out, start);
    le16(&mut out, 0);
    out
}

/// The sheet as a standalone SVG, in millimetres (a viewBox the sheet's size,
/// y down as SVG has it): strokes by pen, filled arrowheads, Helvetica text.
pub fn svg(d: &Drawing) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let y = |v: f64| d.height - v;
    let mut o = String::new();
    let _ = writeln!(o, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">"#, w = d.width, h = d.height);
    let _ = writeln!(o, r#"<rect width="{}" height="{}" fill="white"/>"#, d.width, d.height);
    let _ = writeln!(o, r#"<g fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round">"#);
    for st in d.strokes.iter().filter(|s| s.points.len() >= 2) {
        let dash = match st.pen {
            Pen::Hidden => r#" stroke-dasharray="3 1.5""#,
            Pen::Center => r#" stroke-dasharray="8 1.5 0.5 1.5""#,
            _ => "",
        };
        let pts: Vec<String> = st.points.iter().map(|p| format!("{:.3},{:.3}", p[0], y(p[1]))).collect();
        let _ = writeln!(o, r#"<polyline stroke-width="{}"{dash} points="{}"/>"#, st.pen.width(), pts.join(" "));
    }
    let _ = writeln!(o, "</g>");
    let _ = writeln!(o, r#"<g fill="black" stroke="none">"#);
    for f in &d.fills {
        let pts: Vec<String> = f.iter().map(|p| format!("{:.3},{:.3}", p[0], y(p[1]))).collect();
        let _ = writeln!(o, r#"<polygon points="{}"/>"#, pts.join(" "));
    }
    for t in &d.texts {
        let anchor = match t.anchor {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        };
        let (x, yy) = (t.at[0], y(t.at[1]));
        let _ = writeln!(
            o,
            r#"<text x="{x:.3}" y="{yy:.3}" font-family="Helvetica, Arial, sans-serif" font-size="{:.3}" text-anchor="{anchor}"{}{}>{}</text>"#,
            t.size / 0.718,
            if t.bold { r#" font-weight="bold""# } else { "" },
            if t.angle != 0.0 { format!(r#" transform="rotate({:.3} {x:.3} {yy:.3})""#, -t.angle) } else { String::new() },
            esc(&t.text)
        );
    }
    let _ = writeln!(o, "</g>\n</svg>");
    o
}

/// Text as a DXF TEXT entity writes it: AutoCAD's control codes for ⌀ ° ±,
/// plain ASCII for the drawing's own punctuation, `\U+XXXX` for anything else.
fn dxf_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '⌀' | 'Ø' => out.push_str("%%c"),
            '°' => out.push_str("%%d"),
            '±' => out.push_str("%%p"),
            '·' | '−' | '–' | '—' => out.push('-'),
            '×' => out.push('x'),
            '“' | '”' => out.push('"'),
            '’' => out.push('\''),
            c if c.is_ascii() => out.push(c),
            c => {
                let _ = write!(out, "\\U+{:04X}", c as u32);
            }
        }
    }
    out
}

/// The drawing as a DXF (R12, millimetres): layers VISIBLE, HIDDEN (dashed),
/// DIMENSIONS, FRAME and TEXT, so a CAD or laser program can pick them apart.
pub fn dxf(d: &Drawing) -> String {
    let mut o = String::new();
    let mut pair = |code: i32, value: &str| {
        let _ = writeln!(o, "{code}\n{value}");
    };
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    pair(9, "$DWGCODEPAGE");
    pair(3, "ANSI_1252");
    pair(9, "$INSUNITS");
    pair(70, "4");
    pair(9, "$EXTMIN");
    pair(10, "0");
    pair(20, "0");
    pair(9, "$EXTMAX");
    pair(10, &format!("{:.3}", d.width));
    pair(20, &format!("{:.3}", d.height));
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "TABLES");
    pair(0, "TABLE");
    pair(2, "LTYPE");
    pair(70, "3");
    for (name, text, pattern) in [("CONTINUOUS", "Solid line", vec![]), ("HIDDEN", "Hidden __ __ __", vec![3.0, -1.5]), ("CENTER", "Center ____ _ ____", vec![8.0, -1.5, 0.5, -1.5])] {
        pair(0, "LTYPE");
        pair(2, name);
        pair(70, "0");
        pair(3, text);
        pair(72, "65");
        pair(73, &pattern.len().to_string());
        pair(40, &format!("{:.3}", pattern.iter().map(|v: &f64| v.abs()).sum::<f64>()));
        for v in pattern {
            pair(49, &format!("{v:.3}"));
        }
    }
    pair(0, "ENDTAB");
    pair(0, "TABLE");
    pair(2, "LAYER");
    pair(70, "6");
    for (name, colour, ltype) in [("VISIBLE", "7", "CONTINUOUS"), ("HIDDEN", "8", "HIDDEN"), ("CENTER", "1", "CENTER"), ("DIMENSIONS", "4", "CONTINUOUS"), ("FRAME", "7", "CONTINUOUS"), ("TEXT", "7", "CONTINUOUS")] {
        pair(0, "LAYER");
        pair(2, name);
        pair(70, "0");
        pair(62, colour);
        pair(6, ltype);
    }
    pair(0, "ENDTAB");
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for st in &d.strokes {
        let layer = match st.pen {
            Pen::Visible => "VISIBLE",
            Pen::Hidden => "HIDDEN",
            Pen::Thin => "DIMENSIONS",
            Pen::Center => "CENTER",
            Pen::Frame => "FRAME",
        };
        for w in st.points.windows(2) {
            pair(0, "LINE");
            pair(8, layer);
            pair(10, &format!("{:.4}", w[0][0]));
            pair(20, &format!("{:.4}", w[0][1]));
            pair(11, &format!("{:.4}", w[1][0]));
            pair(21, &format!("{:.4}", w[1][1]));
        }
    }
    for f in &d.fills {
        // Arrowheads: SOLID wants four corners (the last repeated for a triangle).
        // Rounder fills (leader dots) as a fan of them.
        for i in 1..f.len().saturating_sub(1) {
            pair(0, "SOLID");
            pair(8, "DIMENSIONS");
            for (k, p) in [f[0], f[i], f[i + 1], f[i + 1]].iter().enumerate() {
                pair(10 + k as i32, &format!("{:.4}", p[0]));
                pair(20 + k as i32, &format!("{:.4}", p[1]));
            }
        }
    }
    for t in &d.texts {
        pair(0, "TEXT");
        pair(8, "TEXT");
        pair(10, &format!("{:.4}", t.at[0]));
        pair(20, &format!("{:.4}", t.at[1]));
        pair(40, &format!("{:.3}", t.size));
        pair(1, &dxf_text(&t.text));
        if t.angle != 0.0 {
            pair(50, &format!("{:.3}", t.angle));
        }
        let h = match t.anchor {
            Anchor::Start => 0,
            Anchor::Middle => 1,
            Anchor::End => 2,
        };
        if h != 0 {
            pair(72, &h.to_string());
            pair(11, &format!("{:.4}", t.at[0]));
            pair(21, &format!("{:.4}", t.at[1]));
        }
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    o
}
