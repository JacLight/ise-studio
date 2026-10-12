//! Shape programs: geometry written by the AI, not picked from a feature list.
//!
//! A shape is a small JavaScript program plus named parameters, stored in the
//! journal like any other design data. The engine evaluates it in a sandbox
//! over native Manifold (see `runtime`) and derives the solid, its display
//! mesh and its mass. Humans change parameters (dragging, typing); the AI
//! writes and rewrites programs. Nothing about the shape is predefined.

pub mod assets;
pub mod cache;
pub mod cam;
pub mod configs;
pub mod decor;
pub mod drawing;
pub mod exact;
pub mod fea;
pub mod fabricate;
pub mod fillet;
pub mod formats;
pub mod gears;
pub mod image;
pub mod library;
pub mod measure;
pub mod mechanism;
pub mod params;
pub mod pathfinder;
pub mod pattern;
pub mod project;
pub mod paramtable;
pub mod relief;
pub mod render;
pub mod runtime;
pub mod scad;
pub mod sheet;
pub mod sheet_metal;
pub mod sketch;
pub mod stock;
pub mod sweep;
pub mod table;
pub mod text;
pub mod trace;
pub mod trim;

pub use configs::{ConfigRow, ShapeConfigurations};
pub use params::{ParameterSet, ResolvedParams, resolved_program};

use de_core::{
    Command, CommandError, Derived, ErrorCode, IdIndex, Persist, Plugin, Registry, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_solid::{DisplayMesh, MassProperties, Material, MaterialRef, Placement};
use manifold_csg::Manifold;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub struct ShapePlugin;

impl Plugin for ShapePlugin {
    fn name(&self) -> &'static str {
        "shape"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<ShapeProgram>()
            .component::<PartMaterials>()
            .component::<ShapeStudies>()
            .component::<ShapeDrawings>()
            .component::<ParameterSet>()
            .component::<ShapeConfigurations>()
            .component::<sketch::Sketch>()
            .component::<library::StockFamily>()
            .command::<library::InsertStock>()
            .command::<sketch::CreateSketch>()
            .command::<sketch::UpdateSketch>()
            .command::<pathfinder::CombineSketch>()
            .command::<pathfinder::OffsetSketch>()
            .command::<trim::TrimSketch>()
            .command::<trim::ExtendSketch>()
            .command::<fillet::FilletSketch>()
            .command::<fillet::ChamferSketch>()
            .command::<pattern::PatternSketch>()
            .command::<pattern::MirrorSketch>()
            .command::<project::ProjectSketch>()
            .command::<CreateParameterSet>()
            .command::<SetParameters>()
            .command::<LinkParam>()
            .command::<paramtable::ImportParameters>()
            .component::<sheet::Sheet>()
            .command::<sheet::CreateSheet>()
            .command::<sheet::SetSheet>()
            .command::<sheet::ImportSheet>()
            .command::<scad::ImportScad>()
            .command::<CreateShape>()
            .command::<SetShapeParams>()
            .command::<UpdateShape>()
            .command::<SetStudy>()
            .command::<SetDrawing>()
            .command::<configs::ActivateConfig>()
            .command::<RenameDatum>()
            // Formulas first: geometry is evaluated from the resolved values, and
            // re-evaluated when anything a formula reads changes.
            .derived(Derived {
                name: "shape.params",
                inputs: &[ShapeProgram::KIND, ParameterSet::KIND, sketch::Sketch::KIND, sheet::Sheet::KIND],
                reads_dependencies: true,
                compute: params::resolved_params,
            })
            // What each cell of a sheet shows (its numbers are in shape.params above).
            .derived(Derived {
                name: "sheet.values",
                inputs: &[sheet::Sheet::KIND],
                reads_dependencies: true,
                compute: sheet::sheet_values,
            })
            // Sketches solve from their worked-out dimensions, before the shapes that read them.
            .derived(Derived {
                name: "sketch.solve",
                inputs: &[sketch::Sketch::KIND],
                reads_dependencies: true,
                compute: sketch::solve_sketch,
            })
            .derived(Derived {
                name: "shape.evaluate",
                // The active configuration can leave finishes off.
                inputs: &[ShapeProgram::KIND, ShapeConfigurations::KIND],
                reads_dependencies: true,
                compute: evaluate_program,
            })
            .derived(Derived {
                name: "shape.display_mesh",
                inputs: &[ShapeProgram::KIND, ShapeConfigurations::KIND],
                reads_dependencies: true,
                compute: display_mesh,
            })
            .derived(Derived {
                name: "shape.mass_properties",
                inputs: &[ShapeProgram::KIND, ShapeConfigurations::KIND, MaterialRef::KIND, PartMaterials::KIND],
                reads_dependencies: true,
                compute: mass_properties,
            });
    }
}

/// A parameter humans can change without rewriting the program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Param {
    /// Name used in the program as `p.<key>`.
    pub key: String,
    pub label: String,
    pub value: f64,
    /// "mm", "deg", "count" or "" (plain number).
    #[serde(default = "default_unit")]
    pub unit: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// A formula that drives the value (stored with references by id, see
    /// `params`). `value` then holds the last value it was set to by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
}

fn default_unit() -> String {
    "mm".into()
}

/// A named height, e.g. a building storey, in the shape's own coordinates (mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Level {
    pub name: String,
    pub elevation: f64,
}

/// The source of truth for an AI-made shape.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShapeProgram {
    pub name: String,
    /// What the shape is for, in the author's words.
    #[serde(default)]
    pub description: String,
    /// JavaScript function body: reads `p.<key>` (mm, degrees) and returns a Solid.
    pub source: String,
    pub params: Vec<Param>,
    /// How closely the viewport follows the formula (mm). A bolt wants 0.01,
    /// a tower 5: the same relative quality at very different scales.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_tolerance: Option<f64>,
    /// Storeys or other named heights, used for plan views.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<Level>,
    /// Sketches the program reads (`sketch("Name")`), by the name it uses.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub uses: std::collections::BTreeMap<String, StableId>,
    /// Edges rounded or bevelled after the program runs, picked in the view.
    /// Each is sized by one of `params`, so its size can be linked like any other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finishes: Vec<EdgeFinish>,
    /// What those sketches hold right now (JSON for the runtime). Filled when
    /// the program is resolved for evaluation; never stored.
    #[serde(skip)]
    #[schemars(skip)]
    pub inputs: String,
}

/// Rounded or bevelled edges, picked in the view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EdgeFinish {
    pub kind: FinishKind,
    /// The parameter giving the radius (fillet) or distance (chamfer), mm.
    pub key: String,
    /// For programs returning parts: the part whose edges these are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// The edges (fillet, chamfer) or faces (push; shell: the faces left
    /// open, none for a closed hollow; draft: the faces tapered, none for
    /// every wall along the pull).
    pub edges: Vec<EdgePick>,
    /// Draft: the direction the mould opens (default up, [0, 0, 1]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<[f64; 3]>,
    /// Draft: where the neutral plane sits, as a fraction (0–1) of the part's
    /// extent along the pull; 0 (the default) is the end the pull starts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neutral: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinishKind {
    Fillet,
    Chamfer,
    /// A flat face pushed out (positive) or pulled in (negative) along its normal.
    Push,
    /// Hollowed to walls this thick (positive inward, negative outward), open
    /// at the picked faces.
    Shell,
    /// The picked faces tapered by this many degrees from the pull direction.
    Draft,
}

/// An edge as picked: found again on the shape as it is now by kind,
/// direction and where it sits relative to the part's bounds, so it follows
/// when the part is resized.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EdgePick {
    /// Where the edge was (its middle; a circle's center), as fractions of
    /// the part's bounds on x, y and z.
    pub at: [f64; 3],
    /// "line", "circle" or "curve" for an edge; "face" for a pushed face.
    pub kind: String,
    /// A line's direction, or a circle's axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<[f64; 3]>,
    /// A circle's radius (mm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
}

/// What a program reads besides its parameters, as JSON for the runtime:
/// its sketches, solved, and its edge finishes.
pub fn program_inputs(world: &World, program: &ShapeProgram) -> Result<String, String> {
    let sketches = sketch::inputs(world, &program.uses)?;
    Ok(format!(r#"{{"sketches":{sketches},"finishes":{}}}"#, serde_json::to_string(&program.finishes).unwrap()))
}

/// Resolved inputs with other edge finishes (for previews).
pub fn with_finishes(inputs: &str, finishes: &[EdgeFinish]) -> String {
    let mut v: serde_json::Value = serde_json::from_str(inputs).unwrap_or_else(|_| json!({}));
    v["finishes"] = serde_json::to_value(finishes).unwrap();
    v.to_string()
}

impl Persist for ShapeProgram {
    const KIND: &'static str = "shape.program";
}

impl ShapeProgram {
    fn values(&self) -> Vec<(String, f64)> {
        self.params.iter().map(|p| (p.key.clone(), p.value)).collect()
    }

    pub fn display_tolerance(&self) -> f64 {
        self.display_tolerance.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(runtime::DISPLAY_TOLERANCE)
    }
}

/// The materials an assembly's parts name, mapped to material entities. A
/// part without a material uses the shape's own material.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PartMaterials(pub std::collections::BTreeMap<String, StableId>);

impl Persist for PartMaterials {
    const KIND: &'static str = "shape.materials";
}

/// Stress and vibration studies kept with a shape, one per part (or for the
/// whole solid): which faces hold it and which take loads, found again by
/// where they sit when the part changes.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShapeStudies(pub Vec<fea::StudySpec>);

impl Persist for ShapeStudies {
    const KIND: &'static str = "shape.studies";
}

/// Keeps a part's study with the shape (no supports and no loads removes it).
#[derive(Deserialize, JsonSchema)]
pub struct SetStudy {
    pub target: StableId,
    pub study: fea::StudySpec,
}

impl Command for SetStudy {
    const NAME: &'static str = "shape.set_study";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if tx.get::<ShapeProgram>(self.target).is_none() {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a shape", self.target)));
        }
        let mut studies = tx.get::<ShapeStudies>(self.target).cloned().unwrap_or_default();
        studies.0.retain(|s| s.part != self.study.part);
        if !self.study.is_empty() {
            studies.0.push(self.study);
        }
        if studies.0.is_empty() {
            tx.remove::<ShapeStudies>(self.target)?;
        } else {
            tx.set(self.target, studies)?;
        }
        Ok(json!({ "id": self.target }))
    }
}

/// How a part's drawing is set up: a section, dimensions (with tolerances),
/// detail views and geometrical tolerances the person added (one setup per
/// part, or for the whole shape).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DrawingSpec {
    #[serde(default)]
    pub part: Option<String>,
    #[serde(default)]
    pub section: Option<f64>,
    #[serde(default)]
    pub dimensions: Vec<drawing::DimSpec>,
    /// Detail views (circles of a view drawn larger).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<drawing::DetailSpec>,
    /// Geometrical tolerances: feature control frames (ISO 1101).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<drawing::FrameSpec>,
    /// Datum feature symbols.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datums: Vec<drawing::DatumSpec>,
    /// ISO 286 fits on hole callouts, by diameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hole_fits: Vec<drawing::HoleFit>,
    /// The drawing's own dimensions: which to leave out or tolerance (by key).
    #[serde(default, skip_serializing_if = "is_default")]
    pub auto: drawing::AutoSpec,
    /// Threads on holes and diameters, by the diameter modelled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<drawing::ThreadSpec>,
    /// Surface texture symbols on single surfaces (ISO 1302).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finishes: Vec<drawing::FinishSpec>,
    /// Weld symbols (ISO 2553).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub welds: Vec<drawing::WeldSpec>,
    /// Title block fields (every sheet's).
    #[serde(default, skip_serializing_if = "is_default")]
    pub title_block: drawing::TitleFields,
    /// General notes (every sheet's).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// The revision table (every sheet's), oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<drawing::Revision>,
    /// The general surface texture (every sheet's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<drawing::Roughness>,
    /// The exploded isometric of an assembly (first sheet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explode: Option<drawing::Explode>,
    /// Sections, breaks, auxiliary views, cosmetic lines, leaders, texts, tables, angle/arc/area dimensions.
    #[serde(flatten)]
    pub extra: drawing::Extras,
    /// Sheets after the first (which shows the standard views set up above):
    /// each its own part, views, scale and annotations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<SheetSpec>,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// A further sheet of a drawing: one part of the assembly (or the whole
/// thing), the views it shows at a scale, and its own annotations.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SheetSpec {
    /// The part it draws (the drawing's own part, or the whole assembly, when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// The views it shows: front, top, right, iso (all of them when empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<drawing::ViewKind>,
    /// Its scale (2 for 2:1, 0.5 for 1:2); the largest standard one that fits when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// Its sheet size (the first sheet's when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<drawing::Sheet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<drawing::DimSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<drawing::DetailSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<drawing::FrameSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datums: Vec<drawing::DatumSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hole_fits: Vec<drawing::HoleFit>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub auto: drawing::AutoSpec,
    /// Its threads (the first sheet's when empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<drawing::ThreadSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finishes: Vec<drawing::FinishSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub welds: Vec<drawing::WeldSpec>,
    /// Its isometric exploded (an assembly's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explode: Option<drawing::Explode>,
    #[serde(flatten)]
    pub extra: drawing::Extras,
}

impl DrawingSpec {
    /// Nothing set: the default drawing.
    pub fn is_empty(&self) -> bool {
        self.section.is_none() && self.dimensions.is_empty() && self.details.is_empty() && self.frames.is_empty() && self.datums.is_empty() && self.hole_fits.is_empty() && self.sheets.is_empty() && is_default(&self.auto) && self.threads.is_empty() && self.finishes.is_empty() && self.welds.is_empty() && is_default(&self.title_block) && self.notes.is_empty() && self.revisions.is_empty() && self.roughness.is_none() && self.explode.is_none() && is_default(&self.extra)
    }

    /// Each sheet's options and the part it draws, first sheet first; `base`
    /// carries the sheet size and projection asked for.
    pub fn sheet_options(&self, base: &drawing::Options) -> Vec<drawing::Options> {
        let first = drawing::Options {
            part: self.part.clone().or(base.part.clone()),
            section: self.section,
            dimensions: self.dimensions.clone(),
            details: self.details.clone(),
            frames: self.frames.clone(),
            datums: self.datums.clone(),
            hole_fits: self.hole_fits.clone(),
            auto: self.auto.clone(),
            threads: self.threads.clone(),
            finishes: self.finishes.clone(),
            welds: self.welds.clone(),
            title_block: self.title_block.clone(),
            notes: self.notes.clone(),
            revisions: self.revisions.clone(),
            roughness: self.roughness.clone(),
            explode: self.explode.clone(),
            extra: self.extra.clone(),
            ..base.clone()
        };
        let first_part = first.part.clone();
        let rest = self.sheets.iter().map(|s| drawing::Options {
            sheet: s.size.unwrap_or(base.sheet),
            projection: base.projection,
            part: s.part.clone().or(first_part.clone()),
            section: s.section,
            dimensions: s.dimensions.clone(),
            details: s.details.clone(),
            frames: s.frames.clone(),
            datums: s.datums.clone(),
            hole_fits: if s.hole_fits.is_empty() { self.hole_fits.clone() } else { s.hole_fits.clone() },
            views: s.views.clone(),
            scale: s.scale,
            auto: if is_default(&s.auto) { self.auto.clone() } else { s.auto.clone() },
            threads: if s.threads.is_empty() { self.threads.clone() } else { s.threads.clone() },
            finishes: s.finishes.clone(),
            welds: s.welds.clone(),
            title_block: self.title_block.clone(),
            notes: self.notes.clone(),
            revisions: self.revisions.clone(),
            roughness: self.roughness.clone(),
            explode: s.explode.clone(),
            extra: s.extra.clone(),
        });
        std::iter::once(first).chain(rest).collect()
    }

    /// The part each sheet draws (None: the whole assembly), first sheet first.
    pub fn sheet_parts(&self) -> Vec<Option<String>> {
        std::iter::once(self.part.clone()).chain(self.sheets.iter().map(|s| s.part.clone().or(self.part.clone()))).collect()
    }

    /// Gives datum `from` the letter `to` on sheet `sheet` (1 the first) and
    /// every other sheet of the same part (datum letters belong to a part),
    /// frames' references with it. How many references changed.
    pub fn rename_datum(&mut self, sheet: usize, from: &str, to: &str) -> usize {
        let parts = self.sheet_parts();
        let Some(part) = parts.get(sheet.max(1) - 1).cloned() else { return 0 };
        let mut n = 0;
        if parts[0] == part {
            n += drawing::rename_datum(&mut self.frames, &mut self.datums, from, to);
        }
        for (s, p) in self.sheets.iter_mut().zip(&parts[1..]) {
            if *p == part {
                n += drawing::rename_datum(&mut s.frames, &mut s.datums, from, to);
            }
        }
        n
    }

    fn validate(&self) -> Result<(), String> {
        let check = |section: Option<f64>, dims: &[drawing::DimSpec], details: &[drawing::DetailSpec], frames: &[drawing::FrameSpec], datums: &[drawing::DatumSpec], fits: &[drawing::HoleFit]| {
            if section.is_some_and(|p| !(0.0..=100.0).contains(&p)) {
                return Err("section is a percentage of the depth, 0 to 100".to_string());
            }
            drawing::validate(dims, details, frames, datums, fits)
        };
        check(self.section, &self.dimensions, &self.details, &self.frames, &self.datums, &self.hole_fits)?;
        for (i, s) in self.sheets.iter().enumerate() {
            check(s.section, &s.dimensions, &s.details, &s.frames, &s.datums, &s.hole_fits).map_err(|e| format!("sheet {}: {e}", i + 2))?;
            if s.scale.is_some_and(|k| !(k.is_finite() && k > 0.0)) {
                return Err(format!("sheet {}: a scale is a positive number (2 for 2:1, 0.5 for 1:2)", i + 2));
            }
        }
        // A datum letter marks one feature of a part.
        let parts = self.sheet_parts();
        let datums: Vec<&[drawing::DatumSpec]> = std::iter::once(self.datums.as_slice()).chain(self.sheets.iter().map(|s| s.datums.as_slice())).collect();
        let mut seen: Vec<(&Option<String>, &str)> = vec![];
        for (part, list) in parts.iter().zip(datums) {
            for d in list {
                if seen.contains(&(part, d.letter.as_str())) {
                    return Err(format!("datum {} is marked twice", d.letter));
                }
                seen.push((part, &d.letter));
            }
        }
        Ok(())
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShapeDrawings(pub Vec<DrawingSpec>);

impl Persist for ShapeDrawings {
    const KIND: &'static str = "shape.drawings";
}

/// Keeps a drawing's setup with the shape (an empty setup removes it).
#[derive(Deserialize, JsonSchema)]
pub struct SetDrawing {
    pub target: StableId,
    pub drawing: DrawingSpec,
}

impl Command for SetDrawing {
    const NAME: &'static str = "shape.set_drawing";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if tx.get::<ShapeProgram>(self.target).is_none() {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a shape", self.target)));
        }
        self.drawing.validate().map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?;
        if let Some(s) = self.drawing.sheets.iter().position(|s| s.part.is_some() && self.drawing.part.is_some() && s.part != self.drawing.part) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, format!("sheet {} draws another part than the drawing of {}", s + 2, self.drawing.part.as_deref().unwrap_or_default())));
        }
        let mut all = tx.get::<ShapeDrawings>(self.target).cloned().unwrap_or_default();
        all.0.retain(|d| d.part != self.drawing.part);
        if !self.drawing.is_empty() {
            all.0.push(self.drawing);
        }
        if all.0.is_empty() {
            tx.remove::<ShapeDrawings>(self.target)?;
        } else {
            tx.set(self.target, all)?;
        }
        Ok(json!({ "id": self.target }))
    }
}

/// Gives a datum another letter on a drawing (every sheet), and every
/// feature control frame that refers to it the new letter too.
#[derive(Deserialize, JsonSchema)]
pub struct RenameDatum {
    pub target: StableId,
    /// The drawing's part (the whole shape's drawing when absent).
    #[serde(default)]
    pub part: Option<String>,
    /// The sheet the datum is on (1, the first, when absent); the letter
    /// changes on every sheet of the same part.
    #[serde(default)]
    pub sheet: Option<usize>,
    pub from: String,
    pub to: String,
}

impl Command for RenameDatum {
    const NAME: &'static str = "shape.rename_datum";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut all = tx.get::<ShapeDrawings>(self.target).cloned().unwrap_or_default();
        let Some(dw) = all.0.iter_mut().find(|d| d.part == self.part) else {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, "that drawing has no datums"));
        };
        let sheet = self.sheet.unwrap_or(1);
        let on_sheet = if sheet <= 1 { Some(&dw.datums) } else { dw.sheets.get(sheet - 2).map(|s| &s.datums) };
        if !on_sheet.is_some_and(|l| l.iter().any(|d| d.letter == self.from)) {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("no datum {} on sheet {sheet}", self.from)));
        }
        let references = dw.rename_datum(sheet, &self.from, &self.to);
        dw.validate().map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?;
        tx.set(self.target, all)?;
        Ok(json!({ "id": self.target, "references": references }))
    }
}

/// A stored face found on the shape as it is now: its triangles, a point on it and its normal.
///
/// A pick on another part studied with this one (`pick.part`) is found on
/// that part, and the face says so.
pub fn resolve_face(program: &ShapeProgram, part: Option<&str>, pick: &fea::FacePick) -> Option<fea::Face> {
    let part = pick.part.as_deref().or(part);
    let e = evaluated_at(program, program.display_tolerance()).ok()?;
    let (lo, hi) = match part.and_then(|n| e.parts.iter().find(|p| p.name == n)) {
        Some(p) => (p.bounds_min, p.bounds_max),
        None => (e.bounds_min, e.bounds_max),
    };
    let at = |f: [f64; 3]| [0, 1, 2].map(|k| lo[k] + f[k] * (hi[k] - lo[k]));
    let flatten = |tris: &[[[f64; 3]; 3]]| tris.iter().flat_map(|t| t.iter().flat_map(|p| p.iter().copied())).collect();
    if pick.curved {
        // Its point and normal come from where the probe lands now (the surface may have moved).
        let (tris, point, normal) = smooth_face(&e, part, at(pick.probe), pick.normal)?;
        return Some(fea::Face { point, normal, triangles: flatten(&tris), part: pick.part.clone(), curved: true });
    }
    let tris = flat_face(&e, part, at(pick.probe), pick.normal)?;
    Some(fea::Face { point: at(pick.at), normal: pick.normal, triangles: flatten(&tris), part: pick.part.clone(), curved: false })
}

/// Derived: the evaluated solid (millimeters, program coordinates).
#[derive(Component, Clone)]
pub struct ShapeSolid {
    pub solid: Arc<Manifold>,
    pub stats: ShapeStats,
    positions: Arc<Vec<f32>>,
    normals: Arc<Vec<f32>>,
    indices: Arc<Vec<u32>>,
}

/// Measured facts about an evaluated shape, in millimeters.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ShapeStats {
    pub volume_mm3: f64,
    pub area_mm2: f64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub centroid: [f64; 3],
    pub genus: i32,
    pub triangles: usize,
    /// For assemblies, every part; empty for a single solid.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartStats>,
    /// Facts the program reported with metric().
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub metrics: std::collections::BTreeMap<String, runtime::Metric>,
    /// The program declares how its parts move (hinges, gear meshes).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub moves: bool,
    /// How its parts turn, for showing the motion: each hinged body and how
    /// fast it turns for one turn of the driving body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motion: Option<gears::Motion>,
    /// The program joins its parts with joint(): its motion is solved as a linkage.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub linkage: bool,    /// What the configuration it is shown in leaves out: parts, and the
    /// hinges, joints and gear meshes that went with them.
    #[serde(skip_serializing_if = "runtime::LeftOut::is_empty")]
    pub left_out: runtime::LeftOut,
    /// Stock items and connectors the program declared (see `stock`).
    #[serde(skip_serializing_if = "stock::Declared::is_empty")]
    pub declared: stock::Declared,
    /// Sheet-metal parts by name ("" for a program returning one solid):
    /// thickness, material, K-factor, flat size, bend table and checks.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sheet_metal: std::collections::BTreeMap<String, serde_json::Value>,
}

/// One part of an assembly, in millimeters (program coordinates).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PartStats {
    pub name: String,
    pub material: Option<String>,
    pub process: Option<String>,
    pub volume_mm3: f64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub centroid: [f64; 3],
    /// Range of the display mesh's index buffer that draws this part.
    pub index_start: usize,
    pub index_count: usize,
}

/// The program evaluated at a chosen accuracy (mm), cached: requirement
/// checks and exports re-derive the formula as finely as they need.
pub fn evaluated_at(program: &ShapeProgram, tolerance: f64) -> Result<Arc<runtime::Evaluated>, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<Vec<(u64, Arc<runtime::Evaluated>)>>> = OnceLock::new();
    // Assemblies show many configured sizes (screws, inserts, bearings) at once;
    // a small cache re-evaluates them on every sweep.
    const CAPACITY: usize = 64;

    let mut h = DefaultHasher::new();
    program.source.hash(&mut h);
    for p in &program.params {
        p.key.hash(&mut h);
        p.value.to_bits().hash(&mut h);
    }
    tolerance.to_bits().hash(&mut h);
    program.inputs.hash(&mut h);
    let key = h.finish();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, solid)) = cache.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return Ok(solid.clone());
    }
    let solid = Arc::new(runtime::evaluate_in(&program.source, &program.values(), tolerance, &program.inputs)?);
    let mut entries = cache.lock().unwrap();
    if entries.len() >= CAPACITY {
        entries.remove(0);
    }
    entries.push((key, solid.clone()));
    Ok(solid)
}

/// The program's whole solid at a chosen accuracy (mm).
pub fn solid_at(program: &ShapeProgram, tolerance: f64) -> Result<Manifold, String> {
    Ok(evaluated_at(program, tolerance)?.solid.translate(0.0, 0.0, 0.0))
}

/// Binary STL of a program's solid at a chosen accuracy, in millimeters.
pub fn stl(program: &ShapeProgram, tolerance: f64, offset: [f64; 3]) -> Result<Vec<u8>, String> {
    Ok(stl_of(&solid_at(program, tolerance)?, &format!("Forma {} (+/-{tolerance} mm)", program.name), offset))
}

/// Binary STL of any solid, translated by `offset` (mm).
pub fn stl_of(solid: &Manifold, title: &str, offset: [f64; 3]) -> Vec<u8> {
    let (positions, n_props, indices) = solid.to_mesh_f32();
    let n_props = n_props.max(3);
    let v = |i: u32| {
        let i = i as usize * n_props;
        [positions[i] as f64 + offset[0], positions[i + 1] as f64 + offset[1], positions[i + 2] as f64 + offset[2]]
    };
    let triangles = indices.len() / 3;
    let mut out = Vec::with_capacity(84 + triangles * 50);
    let mut header = [0u8; 80];
    let bytes = title.as_bytes();
    header[..bytes.len().min(80)].copy_from_slice(&bytes[..bytes.len().min(80)]);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(triangles as u32).to_le_bytes());
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-30);
        let normal = n.map(|x| x / len);
        for x in normal.iter().chain(&a).chain(&b).chain(&c) {
            out.extend_from_slice(&(*x as f32).to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

/// Cut outlines of a solid at height `z` (its own coordinates, mm): closed
/// polygons, outer boundaries counter-clockwise and holes clockwise.
pub fn slice(solid: &Manifold, z: f64) -> Vec<Vec<[f64; 2]>> {
    solid.slice_at_z(z)
}

/// A parameter that moves a face: dragging the face by d changes it by d / rate.
#[derive(Clone, Debug, Serialize)]
pub struct FaceDriver {
    pub key: String,
    pub label: String,
    pub value: f64,
    /// How far the face moves along its normal per unit of the parameter.
    pub rate: f64,
}

/// Which parameter drives a flat face, if one does: each plain length
/// parameter is nudged and the face watched. The one that moves it most
/// (rather than resizing it sideways) is its driver; None means the face is
/// not any parameter's, and pushing it is a finish of its own.
pub fn face_driver(program: &ShapeProgram, part: Option<&str>, point: [f64; 3], normal: [f64; 3]) -> Option<FaceDriver> {
    let tol = program.display_tolerance() * 4.0;
    let n = {
        let l = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if l < 1e-9 {
            return None;
        }
        normal.map(|c| c / l)
    };
    let eval = |params: &[(String, f64)]| runtime::evaluate_in(&program.source, params, tol, &program.inputs).ok();
    let values = program.values();
    let base = eval(&values)?;
    let c0 = face_offset(&base, part, point, n)?;
    // A parameter drives this face only if it moves nothing else: the volume
    // then changes by the face's area times how far it moved. (One depth
    // under two outlines extruded together moves both; push this one alone.)
    let (area, v0) = (face_area(&base, part, point, n)?, part_volume(&base, part));
    let mut best: Option<FaceDriver> = None;
    for p in program.params.iter().filter(|p| p.expr.is_none() && p.unit == "mm") {
        let mut d = (p.value.abs() * 0.02).max(1.0);
        if p.max.is_some_and(|m| p.value + d > m) {
            d = -d;
        }
        if p.min.is_some_and(|m| p.value + d < m) {
            continue;
        }
        let nudged: Vec<(String, f64)> = values.iter().map(|(k, v)| (k.clone(), if *k == p.key { v + d } else { *v })).collect();
        let Some(e) = eval(&nudged) else { continue };
        let Some(c1) = face_offset(&e, part, point, n) else { continue };
        let rate = (c1 - c0) / d;
        if rate.abs() < 0.2 {
            continue;
        }
        let (grew, moved) = (part_volume(&e, part) - v0, area * (c1 - c0));
        if (grew - moved).abs() > 0.1 * moved.abs() + 1e-6 {
            continue;
        }
        let better = match &best {
            None => true,
            Some(b) => rate.abs() > b.rate.abs() + 1e-6 || ((rate.abs() - b.rate.abs()).abs() < 1e-6 && (rate.abs() - 1.0).abs() < (b.rate.abs() - 1.0).abs()),
        };
        if better {
            best = Some(FaceDriver { key: p.key.clone(), label: p.label.clone(), value: p.value, rate });
        }
    }
    best
}

/// The triangles of one part of an evaluated shape (or all of it).
fn part_triangles<'a>(e: &'a runtime::Evaluated, part: Option<&str>) -> impl Iterator<Item = [[f64; 3]; 3]> + 'a {
    let (start, count) = match part.and_then(|name| e.parts.iter().find(|p| p.name == name)) {
        Some(p) => (p.index_start, p.index_count),
        None => (0, e.indices.len()),
    };
    let v = |i: u32| {
        let i = i as usize * 3;
        [e.positions[i] as f64, e.positions[i + 1] as f64, e.positions[i + 2] as f64]
    };
    e.indices[start..start + count].chunks_exact(3).map(move |t| [v(t[0]), v(t[1]), v(t[2])])
}

fn part_volume(e: &runtime::Evaluated, part: Option<&str>) -> f64 {
    part_triangles(e, part).map(|[a, b, c]| (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0).sum()
}

/// The area of the whole flat face facing `n` under `point`: every triangle
/// connected to the one under the point and lying in its plane.
fn face_area(e: &runtime::Evaluated, part: Option<&str>, point: [f64; 3], n: [f64; 3]) -> Option<f64> {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    Some(
        flat_face(e, part, point, n)?
            .iter()
            .map(|t| {
                let c = cross(sub(t[1], t[0]), sub(t[2], t[0]));
                dot(c, c).sqrt() / 2.0
            })
            .sum(),
    )
}

/// The triangles of the whole flat face facing `n` under `point`.
pub fn flat_face(e: &runtime::Evaluated, part: Option<&str>, point: [f64; 3], n: [f64; 3]) -> Option<Vec<[[f64; 3]; 3]>> {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let offset = face_offset(e, part, point, n)?;
    let tris: Vec<[[f64; 3]; 3]> = part_triangles(e, part).collect();
    let flat = |t: &[[f64; 3]; 3]| {
        let tn = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        let len = dot(tn, tn).sqrt();
        (len > 1e-12 && dot(tn, n) / len > 0.9999 && t.iter().all(|p| (dot(n, *p) - offset).abs() < 1e-4)).then_some(len / 2.0)
    };
    let key = |p: [f64; 3]| p.map(|c| (c * 1e4).round() as i64);
    let mut around: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    let mut areas = vec![None; tris.len()];
    for (i, t) in tris.iter().enumerate() {
        areas[i] = flat(t);
        if areas[i].is_some() {
            for p in t {
                around.entry(key(*p)).or_default().push(i);
            }
        }
    }
    // The flat triangle under the point starts the walk.
    let here = dot(n, point);
    let start = (0..tris.len()).find(|&i| {
        let t = tris[i];
        areas[i].is_some() && {
            let tn = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let q = sub(point, n.map(|k| k * (here - offset)));
            [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])].iter().all(|(p0, p1)| dot(cross(sub(*p1, *p0), sub(q, *p0)), tn) >= -1e-6 * dot(tn, tn).sqrt())
        }
    })?;
    let mut seen = vec![false; tris.len()];
    seen[start] = true;
    let (mut queue, mut out) = (vec![start], vec![]);
    while let Some(i) = queue.pop() {
        if areas[i].is_some() {
            out.push(tris[i]);
        }
        for p in tris[i] {
            for &j in around.get(&key(p)).into_iter().flatten() {
                if !seen[j] {
                    seen[j] = true;
                    queue.push(j);
                }
            }
        }
    }
    Some(out)
}

/// Facets of one smooth surface meet at under about 20° (cos 0.94); a real edge is sharper.
const SMOOTH: f64 = 0.94;

/// The whole smooth curved surface nearest `point` that faces about `n` (a
/// cylinder's side, a bore, a fillet, a sphere): every triangle reached from
/// the one under the point across edges that bend gently, but not into
/// facets of another size (a flat face a fillet runs into, or a fillet a
/// shaft runs into), each surface's facets being alike. Also the point on
/// it nearest `point` and its outward normal there. The web app's
/// `curvedFaceAt` walks the same way, so what is shown is what is held.
pub fn smooth_face(e: &runtime::Evaluated, part: Option<&str>, point: [f64; 3], n: [f64; 3]) -> Option<(Vec<[[f64; 3]; 3]>, [f64; 3], [f64; 3])> {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let tris: Vec<[[f64; 3]; 3]> = part_triangles(e, part).collect();
    let (normals, areas): (Vec<[f64; 3]>, Vec<f64>) = tris
        .iter()
        .map(|t| {
            let c = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let len = dot(c, c).sqrt();
            (if len > 1e-12 { c.map(|v| v / len) } else { [0.0; 3] }, len / 2.0)
        })
        .unzip();
    let nl = dot(n, n).sqrt().max(1e-12);
    let n = n.map(|c| c / nl);
    // The triangle under the point: the nearest one facing about the same way.
    let (start, near) = (0..tris.len())
        .filter(|&i| areas[i] > 0.0 && dot(normals[i], n) > 0.5)
        .map(|i| (i, fea::nearest_on_triangle(point, tris[i][0], tris[i][1], tris[i][2])))
        .min_by(|a, b| {
            let d = |q: [f64; 3]| dot(sub(q, point), sub(q, point));
            d(a.1).total_cmp(&d(b.1))
        })?;
    // Neighbours across each edge (two shared vertex positions).
    let key = |p: [f64; 3]| p.map(|c| (c * 1e4).round() as i64);
    let edge = |a: [f64; 3], b: [f64; 3]| {
        let (ka, kb) = (key(a), key(b));
        if ka < kb { (ka, kb) } else { (kb, ka) }
    };
    let mut across: HashMap<([i64; 3], [i64; 3]), Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        for k in 0..3 {
            across.entry(edge(t[k], t[(k + 1) % 3])).or_default().push(i);
        }
    }
    let (across, all) = (&across, &tris);
    let neighbours = |i: usize| {
        let t = all[i];
        (0..3).flat_map(move |k| across.get(&edge(t[k], t[(k + 1) % 3])).into_iter().flatten().copied()).filter(move |&j| j != i)
    };
    // Each triangle's flat group (coplanar neighbours), and how wide the group is: area over its box's diagonal.
    let mut group = vec![usize::MAX; tris.len()];
    let mut width = vec![];
    for i in 0..tris.len() {
        if group[i] != usize::MAX {
            continue;
        }
        let g = width.len();
        group[i] = g;
        let (mut queue, mut area, mut lo, mut hi) = (vec![i], 0.0, [f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        while let Some(t) = queue.pop() {
            area += areas[t];
            for p in tris[t] {
                for d in 0..3 {
                    lo[d] = lo[d].min(p[d]);
                    hi[d] = hi[d].max(p[d]);
                }
            }
            for j in neighbours(t) {
                if group[j] == usize::MAX && dot(normals[j], normals[t]) > 0.9999 {
                    group[j] = g;
                    queue.push(j);
                }
            }
        }
        let diag = dot(sub(hi, lo), sub(hi, lo)).sqrt();
        width.push(if diag > 0.0 { area / diag } else { 0.0 });
    }
    let alike = |j: usize| {
        let (a, b) = (width[group[j]], width[group[start]]);
        a <= 3.0 * b && b <= 3.0 * a
    };
    let mut seen = vec![false; tris.len()];
    seen[start] = true;
    let (mut queue, mut out) = (vec![start], vec![]);
    while let Some(i) = queue.pop() {
        out.push(tris[i]);
        for j in neighbours(i) {
            if !seen[j] && areas[j] > 0.0 && dot(normals[j], normals[i]) > SMOOTH && alike(j) {
                seen[j] = true;
                queue.push(j);
            }
        }
    }
    Some((out, near, normals[start]))
}

/// Where the flat face facing `n` under `point` sits along `n` (its plane's
/// offset), in an evaluated shape (one part, or all of it).
fn face_offset(e: &runtime::Evaluated, part: Option<&str>, point: [f64; 3], n: [f64; 3]) -> Option<f64> {
    let (start, count) = match part.and_then(|name| e.parts.iter().find(|p| p.name == name)) {
        Some(p) => (p.index_start, p.index_count),
        None => (0, e.indices.len()),
    };
    let v = |i: u32| {
        let i = i as usize * 3;
        [e.positions[i] as f64, e.positions[i + 1] as f64, e.positions[i + 2] as f64]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let here = dot(n, point);
    let mut best: Option<f64> = None;
    for t in e.indices[start..start + count].chunks_exact(3) {
        let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
        let tn = cross(sub(b, a), sub(c, a));
        let len = dot(tn, tn).sqrt();
        if len < 1e-12 || dot(tn, n) / len < 0.999 {
            continue;
        }
        // Is the point, moved onto this triangle's plane along n, inside it?
        let off = dot(n, a);
        let q = sub(point, n.map(|k| k * (here - off)));
        let inside = [(a, b), (b, c), (c, a)].iter().all(|(p0, p1)| dot(cross(sub(*p1, *p0), sub(q, *p0)), tn) >= -1e-6 * len);
        if inside && best.is_none_or(|o| (off - here).abs() < (o - here).abs()) {
            best = Some(off);
        }
    }
    best
}

/// The gear check of a program (see `gears`), at its display accuracy and, for
/// pairs that seem to touch, twenty times finer. `load` is input torque (N·m) and speed (rpm).
pub fn gear_check(program: &ShapeProgram, load: Option<(f64, f64)>) -> Result<gears::Check, String> {
    let tol = program.display_tolerance();
    let e = evaluated_at(program, tol)?;
    let fine = (tol / 20.0).max(0.0005);
    let finer = || runtime::evaluate_in(&program.source, &program.values(), fine, &program.inputs);
    gears::check_at(&e, load, Some(&finer), (tol, fine))
}

/// A linkage's motion through its drive's range (the program declared joints).
pub fn motion_sweep(program: &ShapeProgram, steps: usize) -> Result<mechanism::Sweep, String> {
    motion_with_forces(program, steps, None, true)
}

/// The motion and, given each part's mass (kg, in the program's part order),
/// the joint forces and drive effort at the drive's speed.
pub fn motion_with_forces(program: &ShapeProgram, steps: usize, masses: Option<&[Option<f64>]>, gravity: bool) -> Result<mechanism::Sweep, String> {
    let e = evaluated_at(program, program.display_tolerance())?;
    let m = e.mechanism.as_ref().ok_or("the program declares no joints: add joint(…) for the parts that move")?;
    let linkage = mechanism::Linkage::parse(m).ok_or("the program declares no joints: add joint(…) for the parts that move")??;
    let size = (0..3).map(|k| e.bounds_max[k] - e.bounds_min[k]).fold(0.0, f64::max);
    for b in &linkage.bodies {
        for p in b {
            if !e.parts.iter().any(|q| &q.name == p) {
                return Err(format!("a joint names “{p}”, but the program has no part of that name"));
            }
        }
    }
    let mut sweep = linkage.sweep(steps.clamp(2, 3600), size);
    linkage_gaps(&e, &linkage, &mut sweep, (size * 0.05).clamp(2.0, 20.0));
    if let Some(masses) = masses {
        // Each moving body's mass properties from its parts' meshes and masses.
        let mut bodies = vec![];
        let mut missing = vec![];
        for body in &linkage.bodies {
            let mut props = vec![];
            for name in body {
                let Some(k) = e.parts.iter().position(|p| &p.name == name) else { continue };
                let part = &e.parts[k];
                let tris = &e.indices[part.index_start..part.index_start + part.index_count];
                let unit = mechanism::mesh_mass(&e.positions, tris, 1.0);
                match masses.get(k).copied().flatten() {
                    Some(m) if unit.mass > 0.0 => props.push(mechanism::mesh_mass(&e.positions, tris, m / unit.mass)),
                    _ => missing.push(name.clone()),
                }
            }
            bodies.push(mechanism::MassProps::combine(&props));
        }
        if missing.is_empty() {
            sweep.forces = linkage.forces(&sweep, &bodies, size, gravity);
        } else {
            sweep.notes.push(format!("Forces need every moving part's material: {} has none.", missing.join(", ")));
        }
    }
    Ok(sweep)
}

/// Watches every pair of parts that move relative to each other through the
/// motion (up to 180 of its positions) and keeps their closest approach.
fn linkage_gaps(e: &runtime::Evaluated, l: &mechanism::Linkage, sweep: &mut mechanism::Sweep, search: f64) {
    let body: Vec<Option<usize>> = e.parts.iter().map(|p| l.body_of(&p.name)).collect();
    let mut pairs = vec![];
    for i in 0..e.parts.len() {
        for j in i + 1..e.parts.len() {
            if body[i] != body[j] {
                pairs.push((i, j));
            }
        }
    }
    let frames = &sweep.frames;
    if frames.is_empty() || pairs.is_empty() {
        return;
    }
    let every = frames.len().div_ceil(180).max(1);
    let mut closest = vec![(f64::INFINITY, 0.0); pairs.len()];
    for f in frames.iter().step_by(every) {
        let posed: Vec<Manifold> = (0..e.parts.len())
            .map(|k| match body[k] {
                Some(b) => e.parts[k].solid.transform(&mechanism::pose_matrix(f.bodies[b].0, f.bodies[b].1)),
                None => e.parts[k].solid.translate(0.0, 0.0, 0.0),
            })
            .collect();
        for (n, (i, j)) in pairs.iter().enumerate() {
            let g = posed[*i].min_gap(&posed[*j], search);
            if g < closest[n].0 {
                closest[n] = (g, f.drive);
            }
        }
    }
    let mut out: Vec<mechanism::PairGap> = pairs
        .iter()
        .zip(&closest)
        .filter(|(_, (g, _))| *g < search)
        .map(|((i, j), (g, at))| mechanism::PairGap { a: e.parts[*i].name.clone(), b: e.parts[*j].name.clone(), gap: (g * 1e4).round() / 1e4, at: (at * 100.0).round() / 100.0, joined: l.joined(body[*i], body[*j]) })
        .collect();
    out.sort_by(|a, b| a.gap.total_cmp(&b.gap));
    sweep.clashes = out.iter().filter(|p| p.gap < 1e-4).cloned().collect();
    sweep.pairs = out;
}

/// A stress study on a program's solid (or one part of it), in a material
/// given by name (typical properties; see `fea::elastic`).
pub fn stress_study(program: &ShapeProgram, part: Option<&str>, material: &str, study: &fea::Study) -> Result<fea::Result_, String> {
    let props = fea::elastic(material).ok_or_else(|| format!("no stiffness figures for “{material}”: use steel, aluminium, titanium, brass, copper, PLA, PETG, ABS, nylon, acetal, oak, plywood, MDF, glass or concrete"))?;
    let e = evaluated_at(program, program.display_tolerance())?;
    let (start, count) = match part {
        Some(name) => {
            let p = e.part(name)?;
            (p.index_start, p.index_count)
        }
        None if e.parts.len() > 1 => return Err("pick one part of the assembly to study".into()),
        None => (0, e.indices.len()),
    };
    let study = with_heat(&e.positions, &e.indices[start..start + count], material, study)?;
    fea::solve(&e.positions, &e.indices[start..start + count], &props, &study)
}

/// A study whose heat comes from a heat study: that study run first on the
/// studied part (on the same cubes), its temperatures handed to the stress study.
fn with_heat(positions: &[f32], triangles: &[u32], material: &str, study: &fea::Study) -> Result<fea::Study, String> {
    let mut study = study.clone();
    if let Some(heat) = study.heat.as_mut() {
        if let Some(mut thermal) = heat.thermal.take() {
            let k = fea::conductivity(material).ok_or_else(|| format!("no conductivity figure for “{material}”"))?;
            thermal.resolution = study.resolution;
            heat.field = Some(fea::thermal(positions, triangles, k, &thermal).map_err(|e| format!("the heat study: {e}"))?);
        }
    }
    Ok(study)
}

/// A stress study on several parts of an assembly together, each in its own
/// material (name, material): the first is the studied part (faces that name
/// no part are on it); they press on each other where they touch, each pair
/// as `study.contacts` says (frictionless when not listed).
pub fn assembly_stress(program: &ShapeProgram, parts: &[(String, String)], study: &fea::Study) -> Result<fea::Result_, String> {
    let e = evaluated_at(program, program.display_tolerance())?;
    let bodies = parts
        .iter()
        .map(|(name, material)| {
            let props = fea::elastic(material).ok_or_else(|| format!("no stiffness figures for “{material}” (on “{name}”)"))?;
            let p = e.part(name)?;
            Ok(fea::Body { name: name.clone(), positions: &e.positions, triangles: &e.indices[p.index_start..p.index_start + p.index_count], material: props })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let study = match bodies.first() {
        Some(b) => with_heat(b.positions, b.triangles, &parts[0].1, study)?,
        None => study.clone(),
    };
    fea::solve_parts(&bodies, &study)
}

/// The other parts of an assembly that touch `part` (or come within about
/// a cube of a normal-resolution study of the two).
pub fn touching_parts(program: &ShapeProgram, part: &str) -> Result<Vec<String>, String> {
    let e = evaluated_at(program, program.display_tolerance())?;
    let me = e.part(part)?;
    let mesh = |p: &runtime::EvaluatedPart| (&e.positions[..], &e.indices[p.index_start..p.index_start + p.index_count], p.bounds_min, p.bounds_max);
    Ok(e.parts.iter().filter(|p| p.name != part && fea::touching(mesh(me), mesh(p))).map(|p| p.name.clone()).collect())
}

/// Steady temperatures through a program's solid (or one part) in its material.
pub fn thermal_study(program: &ShapeProgram, part: Option<&str>, material: &str, study: &fea::ThermalStudy) -> Result<fea::Temperatures, String> {
    let k = fea::conductivity(material).ok_or_else(|| format!("no conductivity figure for “{material}”"))?;
    let e = evaluated_at(program, program.display_tolerance())?;
    let (start, count) = match part {
        Some(name) => {
            let p = e.part(name)?;
            (p.index_start, p.index_count)
        }
        None if e.parts.len() > 1 => return Err("pick one part of the assembly to study".into()),
        None => (0, e.indices.len()),
    };
    fea::thermal(&e.positions, &e.indices[start..start + count], k, study)
}

/// A moving part checked at the worst moment of its motion.
#[derive(Clone, Debug, Serialize)]
pub struct MotionStress {
    pub result: fea::Result_,
    /// The drive's value at that moment (degrees or mm).
    pub at: f64,
    /// The joint whose seat holds the part.
    pub held_by: String,
    /// The joints whose forces load it then, and how hard (N).
    pub loads: Vec<(String, f64)>,
}

/// A part of a linkage checked for strength at the moment its joints pull
/// hardest: held at one joint's seat (the one to the ground if it has one),
/// loaded at the others' seats with the forces the motion puts through them.
/// The part's own inertia is left out (small beside its joint forces in a
/// slender member). `masses` per part (kg), `material` its name.
pub fn motion_stress(program: &ShapeProgram, part: &str, masses: &[Option<f64>], material: &str) -> Result<MotionStress, String> {
    let props = fea::elastic(material).ok_or_else(|| format!("no stiffness figures for “{material}”"))?;
    let sweep = motion_with_forces(program, 360, Some(masses), true)?;
    let forces = sweep.forces.as_ref().ok_or("forces need every moving part's material")?;
    let e = evaluated_at(program, program.display_tolerance())?;
    let m = e.mechanism.as_ref().ok_or("the program declares no joints")?;
    let linkage = mechanism::Linkage::parse(m).ok_or("the program declares no joints")??;
    let body = linkage.body_of(part).ok_or_else(|| format!("“{part}” is not a moving part"))?;
    let touching: Vec<usize> = (0..linkage.joints.len()).filter(|&q| linkage.joints[q].a == Some(body) || linkage.joints[q].b == Some(body)).collect();
    if touching.len() < 2 {
        return Err(format!("“{part}” needs two joints: one to hold it and one to load it"));
    }
    let held = touching.iter().copied().find(|&q| linkage.joints[q].a.is_none() || linkage.joints[q].b.is_none()).unwrap_or(touching[0]);
    let loaded: Vec<usize> = touching.iter().copied().filter(|&q| q != held).collect();
    // The force a joint puts on this body at frame k, in the body's drawn axes.
    let on_body = |q: usize, k: usize| {
        let v = forces.vectors[q][k];
        let v = if forces.on[q] == Some(body) { v } else { v.map(|c| -c) };
        mechanism::to_drawn(&sweep.frames[k], body, v)
    };
    let size = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let worst = (0..sweep.frames.len()).max_by(|&a, &b| {
        let peak = |k: usize| loaded.iter().map(|&q| size(on_body(q, k))).fold(0.0, f64::max);
        peak(a).total_cmp(&peak(b))
    }).ok_or("the motion has no positions")?;
    let bore = |q: usize| fea::Bore { at: linkage.joints[q].at, axis: linkage.joints[q].axis };
    let study = fea::Study {
        supports: vec![],
        loads: vec![],
        bore_supports: vec![bore(held)],
        bore_loads: loaded.iter().map(|&q| fea::BoreLoad { bore: bore(q), force: on_body(q, worst) }).collect(),
        resolution: 56,
        ..Default::default()
    };
    let k = e.part(part)?;
    let mut result = fea::solve(&e.positions, &e.indices[k.index_start..k.index_start + k.index_count], &props, &study)?;
    result.notes.push("Held at one joint's seat, loaded at the others; the part's own inertia is left out.".into());
    Ok(MotionStress {
        result,
        at: sweep.frames[worst].drive,
        held_by: linkage.joints[held].name.clone(),
        loads: loaded.iter().map(|&q| (linkage.joints[q].name.clone(), size(on_body(q, worst)))).collect(),
    })
}

/// Natural frequencies of a program's solid (or one part), held at `supports`.
pub fn vibration_study(program: &ShapeProgram, part: Option<&str>, material: &str, density: f64, supports: &[fea::Face], count: usize, resolution: usize) -> Result<fea::Modes, String> {
    let props = fea::elastic(material).ok_or_else(|| format!("no stiffness figures for “{material}”"))?;
    let e = evaluated_at(program, program.display_tolerance())?;
    let (start, count_idx) = match part {
        Some(name) => {
            let p = e.part(name)?;
            (p.index_start, p.index_count)
        }
        None if e.parts.len() > 1 => return Err("pick one part of the assembly to study".into()),
        None => (0, e.indices.len()),
    };
    fea::modes(&e.positions, &e.indices[start..start + count_idx], &props, density, supports, count, resolution)
}

/// Evaluates a program without storing anything: for previews while dragging.
pub fn preview(program: &ShapeProgram) -> Result<(ShapeStats, DisplayMesh), String> {
    let e = runtime::evaluate_in(&program.source, &program.values(), program.display_tolerance(), &program.inputs)?;
    let stats = stats_of(&e);
    Ok((stats, mesh_of(&e.positions, &e.normals, &e.indices)))
}

pub(crate) fn stats_of(e: &runtime::Evaluated) -> ShapeStats {
    ShapeStats {
        volume_mm3: e.volume,
        area_mm2: e.area,
        bounds_min: e.bounds_min,
        bounds_max: e.bounds_max,
        centroid: e.centroid,
        genus: e.genus,
        triangles: e.triangles,
        parts: e
            .parts
            .iter()
            .map(|p| PartStats {
                name: p.name.clone(),
                material: p.material.clone(),
                process: p.process.clone(),
                volume_mm3: p.volume,
                bounds_min: p.bounds_min,
                bounds_max: p.bounds_max,
                centroid: p.centroid,
                index_start: p.index_start,
                index_count: p.index_count,
            })
            .collect(),
        metrics: e.metrics.clone(),
        moves: e.mechanism.is_some(),
        motion: e.mechanism.as_ref().and_then(|m| gears::Mechanism::parse(m).ok()).and_then(|m| m.motion().ok()),
        linkage: e.mechanism.as_ref().is_some_and(|m| m.get("joints").and_then(serde_json::Value::as_array).is_some_and(|j| !j.is_empty())),
        left_out: e.left_out.clone(),
        declared: e.declared.clone(),
        sheet_metal: e.sheets.iter().map(|s| (s.part.clone(), s.data.clone())).collect(),
    }
}

pub(crate) fn mesh_of(positions: &[f32], normals: &[f32], indices: &[u32]) -> DisplayMesh {
    DisplayMesh {
        positions: positions.iter().map(|v| v / 1000.0).collect(),
        normals: normals.to_vec(),
        indices: indices.to_vec(),
    }
}

fn evaluate_program(world: &World, entity: Entity) -> Option<Result<ShapeSolid, String>> {
    let program = resolved_program(world, entity)?;
    Some(runtime::evaluate_in(&program.source, &program.values(), program.display_tolerance(), &program.inputs).map(|e| ShapeSolid {
        stats: stats_of(&e),
        positions: Arc::new(e.positions),
        normals: Arc::new(e.normals),
        indices: Arc::new(e.indices),
        solid: Arc::new(e.solid),
    }))
}

fn display_mesh(world: &World, entity: Entity) -> Option<Result<DisplayMesh, String>> {
    world.get::<ShapeProgram>(entity)?;
    let s = world.get::<ShapeSolid>(entity)?;
    Some(Ok(mesh_of(&s.positions, &s.normals, &s.indices)))
}

/// Density (kg/m³) of a material entity.
fn density(world: &World, id: StableId) -> Result<f64, String> {
    world
        .resource::<IdIndex>()
        .get(id)
        .and_then(|e| world.get::<Material>(e))
        .map(|m| m.density.canonical())
        .ok_or_else(|| format!("material {id} not found"))
}

/// Mass (kg) of every part of an assembly, or of the single solid, by its
/// material; `None` where no material applies.
pub fn part_masses(world: &World, entity: Entity) -> Result<Vec<Option<f64>>, String> {
    match world.get::<ShapeSolid>(entity) {
        Some(s) => masses_of(world, entity, &s.stats),
        None => Ok(Vec::new()),
    }
}

/// Mass (kg) of every part of `stats` (or of its single solid), made of shape
/// `entity`'s materials.
pub fn masses_of(world: &World, entity: Entity, stats: &ShapeStats) -> Result<Vec<Option<f64>>, String> {
    let default = world.get::<MaterialRef>(entity).map(|m| m.0);
    let table = world.get::<PartMaterials>(entity);
    if stats.parts.is_empty() {
        return Ok(vec![default.map(|id| density(world, id)).transpose()?.map(|d| stats.volume_mm3 / 1e9 * d)]);
    }
    stats
        .parts
        .iter()
        .map(|p| {
            let id = match &p.material {
                Some(name) => Some(table.and_then(|t| t.0.get(name)).copied().ok_or_else(|| format!("{}: material “{name}” is not in the shape's material table", p.name))?),
                None => default,
            };
            Ok(id.map(|id| density(world, id)).transpose()?.map(|d| p.volume_mm3 / 1e9 * d))
        })
        .collect()
}

/// Volume, mass and center of mass from measured stats and part masses.
pub(crate) fn mass_of(stats: &ShapeStats, masses: &[Option<f64>]) -> MassProperties {
    let volume_m3 = stats.volume_mm3 / 1e9;
    let mass_kg = masses.iter().copied().sum::<Option<f64>>();
    // With per-part densities the center of mass weights each part by its mass.
    let center = match (&mass_kg, stats.parts.is_empty()) {
        (Some(total), false) if *total > 0.0 => {
            let mut c = [0.0; 3];
            for (p, m) in stats.parts.iter().zip(masses) {
                for k in 0..3 {
                    c[k] += p.centroid[k] * m.unwrap_or(0.0) / total;
                }
            }
            c
        }
        _ => stats.centroid,
    };
    MassProperties { volume_m3, mass_kg, center_of_mass_m: center.map(|c| c / 1000.0) }
}

fn mass_properties(world: &World, entity: Entity) -> Option<Result<MassProperties, String>> {
    world.get::<ShapeProgram>(entity)?;
    let s = world.get::<ShapeSolid>(entity)?;
    Some(masses_of(world, entity, &s.stats).map(|m| mass_of(&s.stats, &m)))
}

// ---- Commands --------------------------------------------------------------

pub(crate) fn check_params(params: &[Param]) -> Result<(), CommandError> {
    let bad = |m: String| Err(CommandError::new(ErrorCode::InvalidParameters, m));
    let mut seen = std::collections::HashSet::new();
    for p in params {
        if p.key.is_empty() || !p.key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || p.key.starts_with(|c: char| c.is_ascii_digit()) {
            return bad(format!("parameter key “{}” must be a plain identifier", p.key));
        }
        if !seen.insert(&p.key) {
            return bad(format!("parameter “{}” is declared twice", p.key));
        }
        if !p.value.is_finite() {
            return bad(format!("parameter “{}” must be a finite number", p.key));
        }
        if p.min.is_some_and(|m| p.value < m) || p.max.is_some_and(|m| p.value > m) {
            return bad(format!("{} must be between {} and {}", p.label, p.min.unwrap_or(f64::NEG_INFINITY), p.max.unwrap_or(f64::INFINITY)));
        }
    }
    Ok(())
}

/// Rejects programs that do not produce a valid solid, before anything is stored.
/// Linked parameters are evaluated with their formulas worked out.
pub(crate) fn validate(world: &World, program: &ShapeProgram, materials: &PartMaterials) -> Result<(), CommandError> {
    validate_in(world, program, materials, None).map(|_| ())
}

/// As [`validate`], in a configuration (its name and the parts it leaves
/// out reach the program), returning what it evaluated to.
pub(crate) fn validate_in(world: &World, program: &ShapeProgram, materials: &PartMaterials, row: Option<&ConfigRow>) -> Result<runtime::Evaluated, CommandError> {
    check_params(&program.params)?;
    if program.display_tolerance.is_some_and(|t| !(t.is_finite() && t > 0.0 && t <= 100.0)) {
        return Err(CommandError::new(ErrorCode::InvalidParameters, "display tolerance must be between 0 and 100 mm"));
    }
    let values: Vec<(String, f64)> = params::resolve(world, &program.params)
        .map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?
        .into_iter()
        .collect();
    for f in &program.finishes {
        let size = values.iter().find(|(k, _)| *k == f.key).map(|(_, v)| *v);
        let ok = match f.kind {
            FinishKind::Push => size.is_some(),
            FinishKind::Shell => size.is_some_and(|s| s != 0.0),
            FinishKind::Draft => size.is_some_and(|s| s != 0.0 && s.abs() < 89.0),
            _ => size.is_some_and(|s| s > 0.0),
        };
        if !ok {
            return Err(CommandError::new(
                ErrorCode::InvalidParameters,
                format!("a finish is sized by “{}”, which must be a parameter of the shape (positive for fillets and chamfers, non-zero for shells, under 89° for drafts)", f.key),
            ));
        }
        if f.kind == FinishKind::Draft {
            if f.pull.is_some_and(|d| !d.iter().all(|c| c.is_finite()) || d.iter().map(|c| c * c).sum::<f64>() < 1e-12) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "a draft's pull direction must be a non-zero [x, y, z]"));
            }
            if f.neutral.is_some_and(|n| !(0.0..=1.0).contains(&n)) {
                return Err(CommandError::new(ErrorCode::InvalidParameters, "a draft's neutral plane is a fraction of the part, 0 to 1"));
            }
        }
        if f.edges.is_empty() && !matches!(f.kind, FinishKind::Shell | FinishKind::Draft) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "an edge finish needs at least one edge"));
        }
    }
    let inputs = program_inputs(world, program).map_err(|e| CommandError::new(ErrorCode::UnresolvedReference, e))?;
    let inputs = match row {
        Some(r) => configs::with_row(&inputs, r),
        None => inputs,
    };
    let e = runtime::evaluate_in(&program.source, &values, program.display_tolerance(), &inputs)
        .map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
    for p in &e.parts {
        if let Some(m) = &p.material
            && !materials.0.contains_key(m)
        {
            return Err(CommandError::new(
                ErrorCode::UnresolvedReference,
                format!("part “{}” is made of “{m}”, which the shape's materials do not define", p.name),
            ));
        }
    }
    Ok(e)
}

/// Replaces a shape's material table, keeping its dependencies in step.
fn set_materials(tx: &mut Tx, id: StableId, table: PartMaterials) -> Result<(), CommandError> {
    let keep = tx.get::<MaterialRef>(id).map(|m| m.0);
    if let Some(old) = tx.get::<PartMaterials>(id).cloned() {
        for m in old.0.values() {
            if Some(*m) != keep && !table.0.values().any(|n| n == m) {
                tx.remove_dependency(id, *m)?;
            }
        }
    }
    for m in table.0.values() {
        if tx.get::<Material>(*m).is_none() {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{m} is not a material")));
        }
        tx.add_dependency(id, *m)?;
    }
    if table.0.is_empty() { tx.remove::<PartMaterials>(id) } else { tx.set(id, table) }
}

/// `shape.create`: a new part from an AI-written program.
#[derive(Deserialize, JsonSchema)]
pub struct CreateShape {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source: String,
    pub params: Vec<Param>,
    pub display_tolerance: Option<f64>,
    #[serde(default)]
    pub levels: Vec<Level>,
    pub placement: Option<Placement>,
    pub material: Option<StableId>,
    /// For assemblies: material names used by `part(…, { material })`.
    #[serde(default)]
    pub materials: std::collections::BTreeMap<String, StableId>,
    /// A component only placed through instances: it has no placement of its own.
    #[serde(default)]
    pub definition: bool,
}

impl Command for CreateShape {
    const NAME: &'static str = "shape.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let program = ShapeProgram {
            name: self.name,
            description: self.description,
            source: self.source,
            params: self.params,
            display_tolerance: self.display_tolerance,
            levels: self.levels,
            uses: Default::default(),
            finishes: Vec::new(),
            inputs: String::new(),
        };
        let mut program = program;
        let table = PartMaterials(self.materials);
        let id = tx.create();
        params::link(tx, id, &[], &mut program.params)?;
        program.uses = sketch::resolve_uses(tx, &program.source, &Default::default())?;
        for s in program.uses.values() {
            tx.add_dependency(id, *s)?;
        }
        validate(tx.world(), &program, &table)?;
        tx.set(id, program)?;
        set_materials(tx, id, table)?;
        if !self.definition {
            tx.set(id, self.placement.unwrap_or_default())?;
        }
        if let Some(material) = self.material {
            if tx.get::<Material>(material).is_none() {
                return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{material} is not a material")));
            }
            tx.set(id, MaterialRef(material))?;
            tx.add_dependency(id, material)?;
        }
        Ok(json!({ "id": id }))
    }
}

/// `shape.set_params`: change parameter values (what humans drag and type).
#[derive(Deserialize, JsonSchema)]
pub struct SetShapeParams {
    pub target: StableId,
    pub values: std::collections::BTreeMap<String, f64>,
}

impl Command for SetShapeParams {
    const NAME: &'static str = "shape.set_params";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut program = tx
            .get::<ShapeProgram>(self.target)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a shape", self.target)))?;
        for (key, value) in &self.values {
            let param = program
                .params
                .iter_mut()
                .find(|p| &p.key == key)
                .ok_or_else(|| CommandError::new(ErrorCode::InvalidParameters, format!("the shape has no parameter “{key}”")))?;
            if let Some(f) = &param.expr {
                return Err(CommandError::new(
                    ErrorCode::InvalidParameters,
                    format!("{} follows {}: change that, or unlink it first (shape.link_param with no formula)", param.label, params::display(tx.world(), f)),
                ));
            }
            param.value = *value;
        }
        validate(tx.world(), &program, &tx.get::<PartMaterials>(self.target).cloned().unwrap_or_default())?;
        tx.set(self.target, program)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `shape.update`: the AI rewrites a shape (source, parameters, name).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateShape {
    pub target: StableId,
    pub name: Option<String>,
    pub description: Option<String>,
    pub source: Option<String>,
    pub params: Option<Vec<Param>>,
    pub display_tolerance: Option<f64>,
    pub levels: Option<Vec<Level>>,
    pub materials: Option<std::collections::BTreeMap<String, StableId>>,
    /// Replaces the edge finishes (rounded or bevelled edges picked in the view).
    pub finishes: Option<Vec<EdgeFinish>>,
}

impl Command for UpdateShape {
    const NAME: &'static str = "shape.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut program = tx
            .get::<ShapeProgram>(self.target)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a shape", self.target)))?;
        if let Some(name) = self.name {
            program.name = name;
        }
        if let Some(description) = self.description {
            program.description = description;
        }
        if let Some(source) = self.source {
            program.source = source;
            let uses = sketch::resolve_uses(tx, &program.source, &program.uses)?;
            for old in program.uses.values().filter(|id| !uses.values().any(|n| n == *id)) {
                tx.remove_dependency(self.target, *old)?;
            }
            for new in uses.values() {
                tx.add_dependency(self.target, *new)?;
            }
            program.uses = uses;
        }
        if let Some(mut params) = self.params {
            let old = program.params.clone();
            params::link(tx, self.target, &old, &mut params)?;
            program.params = params;
        }
        if let Some(t) = self.display_tolerance {
            program.display_tolerance = Some(t);
        }
        if let Some(levels) = self.levels {
            program.levels = levels;
        }
        if let Some(finishes) = self.finishes {
            program.finishes = finishes;
        }
        let table = match self.materials {
            Some(m) => PartMaterials(m),
            None => tx.get::<PartMaterials>(self.target).cloned().unwrap_or_default(),
        };
        validate(tx.world(), &program, &table)?;
        // The configuration table still fits: its columns are still plain
        // parameters; a finish that is gone is no longer left off.
        if let Some(mut configs) = tx.get::<ShapeConfigurations>(self.target).cloned() {
            let before = configs.clone();
            for row in &mut configs.rows {
                row.suppress.retain(|k| program.finishes.iter().any(|f| &f.key == k));
            }
            configs::check_table(tx.world(), &program, &configs)?;
            if configs != before {
                tx.set(self.target, configs)?;
            }
        }
        tx.set(self.target, program)?;
        set_materials(tx, self.target, table)?;
        Ok(json!({ "id": self.target }))
    }
}

// ---- Parameter sets and links ------------------------------------------------

/// `parameters.create`: named values many shapes can derive from.
#[derive(Deserialize, JsonSchema)]
pub struct CreateParameterSet {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub params: Vec<Param>,
}

impl Command for CreateParameterSet {
    const NAME: &'static str = "parameters.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.name.trim().is_empty() || self.name.contains('.') {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "a parameter set needs a name without dots"));
        }
        check_params(&self.params)?;
        let mut set = ParameterSet { name: self.name, description: self.description, params: self.params };
        let id = tx.create();
        params::link(tx, id, &[], &mut set.params)?;
        params::resolve(tx.world(), &set.params).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?;
        tx.set(id, set)?;
        Ok(json!({ "id": id }))
    }
}

/// `parameters.set`: change values in a parameter set; everything derived follows.
#[derive(Deserialize, JsonSchema)]
pub struct SetParameters {
    pub target: StableId,
    pub values: std::collections::BTreeMap<String, f64>,
}

impl Command for SetParameters {
    const NAME: &'static str = "parameters.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        // A parameter set's values, or a sketch's dimensions.
        let (name, mut list) = match (tx.get::<ParameterSet>(self.target), tx.get::<sketch::Sketch>(self.target)) {
            (Some(s), _) => (s.name.clone(), s.params.clone()),
            (None, Some(s)) => (s.name.clone(), s.params.clone()),
            _ => return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a parameter set or a sketch", self.target))),
        };
        for (key, value) in &self.values {
            let p = list
                .iter_mut()
                .find(|p| &p.key == key)
                .ok_or_else(|| CommandError::new(ErrorCode::InvalidParameters, format!("{name} has no parameter “{key}”")))?;
            if p.expr.is_some() {
                return Err(CommandError::new(ErrorCode::InvalidParameters, format!("{} is a formula; change what it reads, or unlink it", p.label)));
            }
            p.value = *value;
        }
        check_params(&list)?;
        store_params(tx, self.target, list)?;
        Ok(json!({ "id": self.target }))
    }
}

/// Stores a parameter list on whatever holds it, checking that it still works
/// out (and, for shapes and sketches, that the geometry still solves).
pub(crate) fn store_params(tx: &mut Tx, id: StableId, list: Vec<Param>) -> Result<(), CommandError> {
    let invalid = |e: String| CommandError::new(ErrorCode::InvalidParameters, e);
    if let Some(mut set) = tx.get::<ParameterSet>(id).cloned() {
        params::resolve(tx.world(), &list).map_err(invalid)?;
        set.params = list;
        return tx.set(id, set);
    }
    if let Some(mut s) = tx.get::<sketch::Sketch>(id).cloned() {
        let dims = params::resolve(tx.world(), &list).map_err(invalid)?;
        sketch::solve(&s.geometry, &dims).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        s.params = list;
        return tx.set(id, s);
    }
    let mut program = tx.get::<ShapeProgram>(id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} has no parameters")))?;
    program.params = list;
    validate(tx.world(), &program, &tx.get::<PartMaterials>(id).cloned().unwrap_or_default())?;
    tx.set(id, program)
}

/// `shape.link_param`: drive a parameter (of a shape or a parameter set) by a
/// formula such as `=Tower footprint.width - 600`, or unlink it (no formula),
/// keeping the value it had.
#[derive(Deserialize, JsonSchema)]
pub struct LinkParam {
    pub target: StableId,
    pub key: String,
    pub expr: Option<String>,
}

impl Command for LinkParam {
    const NAME: &'static str = "shape.link_param";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let not_found = || CommandError::new(ErrorCode::UnresolvedReference, format!("{} has no parameters", self.target));
        let mut list = match (tx.get::<ShapeProgram>(self.target), tx.get::<ParameterSet>(self.target), tx.get::<sketch::Sketch>(self.target)) {
            (Some(p), _, _) => p.params.clone(),
            (None, Some(s), _) => s.params.clone(),
            (None, None, Some(s)) => s.params.clone(),
            _ => return Err(not_found()),
        };
        let old = list.clone();
        let resolved = tx.world().resource::<IdIndex>().get(self.target).and_then(|e| tx.world().get::<ResolvedParams>(e)).cloned();
        let p = list
            .iter_mut()
            .find(|p| p.key == self.key)
            .ok_or_else(|| CommandError::new(ErrorCode::InvalidParameters, format!("no parameter “{}”", self.key)))?;
        match self.expr.filter(|e| !e.trim().trim_start_matches('=').trim().is_empty()) {
            Some(e) => {
                // A configuration sets this parameter: it cannot follow a formula as well.
                if let Some(t) = tx.get::<ShapeConfigurations>(self.target)
                    && t.params.contains(&self.key)
                {
                    return Err(CommandError::new(
                        ErrorCode::InvalidParameters,
                        format!("{} is a column of the configuration table, so each configuration sets it: take it out of the table before linking it to a formula", p.label),
                    ));
                }
                p.expr = Some(e)
            }
            None => {
                // Unlinking keeps the value the formula last gave.
                if let Some(v) = resolved.and_then(|r| r.0.get(&self.key).copied()) {
                    p.value = v;
                }
                p.expr = None;
            }
        }
        params::link(tx, self.target, &old, &mut list)?;
        store_params(tx, self.target, list)?;
        Ok(json!({ "id": self.target }))
    }
}
