//! What the prelude's picture and lettering helpers ask the engine for
//! (`relief`, `trace`, `text`, reading a picture's pixels), as both kernels
//! answer it: the options arrive as JSON, already filled in by the prelude.

use crate::image::{Channel, Field};
use crate::{relief, text, trace};
use manifold_csg::{CrossSection, FillRule, Manifold};
use serde::Deserialize;

/// Triangles a relief may use: the finer the accuracy asked for, the more
/// (the view gets a light mesh, a print export the full detail).
pub fn relief_budget(tolerance: f64) -> usize {
    ((6000.0 / tolerance.max(1e-6)) as usize).clamp(100_000, 6_000_000)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReliefJson {
    width: f64,
    height: f64,
    depth: f64,
    base: f64,
    mode: String,
    #[serde(default)]
    invert: bool,
    #[serde(default = "one")]
    gamma: f64,
    #[serde(default)]
    smooth: f64,
    #[serde(default = "brightness")]
    channel: String,
    #[serde(default = "one")]
    clear: f64,
    #[serde(default = "detail")]
    detail: usize,
}

fn one() -> f64 {
    1.0
}
fn brightness() -> String {
    "brightness".into()
}
fn detail() -> usize {
    1024
}

/// A relief from a stored picture.
pub fn relief(sha: &str, options: &str, tolerance: f64, budget: usize) -> Result<Manifold, String> {
    let o: ReliefJson = serde_json::from_str(options).map_err(|e| format!("relief: {e}"))?;
    let mode = match o.mode.as_str() {
        "emboss" => relief::Mode::Emboss,
        "engrave" => relief::Mode::Engrave,
        "lithophane" => relief::Mode::Lithophane,
        other => return Err(format!("relief: mode is \"emboss\", \"engrave\" or \"lithophane\", not \"{other}\"")),
    };
    if !(o.gamma.is_finite() && o.gamma > 0.0) {
        return Err("relief: gamma must be a positive number".into());
    }
    let picture = crate::image::load(sha).map_err(|e| format!("relief: {e}"))?;
    let spec = relief::Spec {
        width: o.width,
        height: o.height,
        depth: o.depth,
        base: o.base,
        mode,
        invert: o.invert,
        gamma: o.gamma,
        smooth: o.smooth.max(0.0),
        channel: Channel::parse(&o.channel).map_err(|e| format!("relief: {e}"))?,
        clear: o.clear,
        tolerance,
        budget,
        detail: o.detail.clamp(16, 4096),
    };
    Ok(relief::build(&picture, &spec)?.solid)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TraceJson {
    width: f64,
    height: f64,
    #[serde(default = "half")]
    threshold: f64,
    #[serde(default)]
    invert: bool,
    smooth: f64,
    min_area: f64,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    simplify: Option<f64>,
}

fn half() -> f64 {
    0.5
}

/// A stored picture's ink as outlines (counter-clockwise around ink).
pub fn trace(sha: &str, options: &str, tolerance: f64) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let o: TraceJson = serde_json::from_str(options).map_err(|e| format!("trace: {e}"))?;
    let picture = crate::image::load(sha).map_err(|e| format!("trace: {e}"))?;
    let spec = trace::Spec {
        width: o.width,
        height: o.height,
        threshold: o.threshold,
        invert: o.invert,
        smooth: o.smooth.max(0.0),
        min_area: o.min_area.max(0.0),
        tolerance: o.simplify.unwrap_or(tolerance).max(1e-6),
        channel: o.channel.as_deref().map(Channel::parse).transpose().map_err(|e| format!("trace: {e}"))?,
    };
    let loops = trace::trace(&picture, &spec)?;
    if loops.is_empty() {
        return Err(format!("trace: nothing above the threshold {} (try invert, or another threshold)", o.threshold));
    }
    Ok(loops)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextJson {
    text: String,
    size: f64,
    #[serde(default)]
    font: Option<String>,
    #[serde(default = "regular")]
    weight: f64,
    #[serde(default = "start")]
    align: String,
    #[serde(default = "baseline")]
    baseline: String,
    #[serde(default)]
    spacing: f64,
    #[serde(default = "line_height")]
    line_height: f64,
    #[serde(default)]
    arc: Option<f64>,
}

fn regular() -> f64 {
    400.0
}
fn start() -> String {
    "left".into()
}
fn baseline() -> String {
    "baseline".into()
}
fn line_height() -> f64 {
    1.6
}

/// Lettering as one profile (overlaps merged).
pub fn text(options: &str, tolerance: f64) -> Result<CrossSection, String> {
    let o: TextJson = serde_json::from_str(options).map_err(|e| format!("text: {e}"))?;
    let spec = text::Spec {
        text: o.text,
        size: o.size,
        font: o.font,
        weight: o.weight,
        align: match o.align.as_str() {
            "left" | "start" => text::Align::Start,
            "center" | "centre" | "middle" => text::Align::Center,
            "right" | "end" => text::Align::End,
            other => return Err(format!("text: align is \"left\", \"center\" or \"right\", not \"{other}\"")),
        },
        baseline: match o.baseline.as_str() {
            "baseline" => text::Baseline::Baseline,
            "top" => text::Baseline::Top,
            "middle" | "center" | "centre" => text::Baseline::Middle,
            "bottom" => text::Baseline::Bottom,
            other => return Err(format!("text: valign is \"baseline\", \"top\", \"middle\" or \"bottom\", not \"{other}\"")),
        },
        spacing: o.spacing,
        line_height: o.line_height,
        arc: o.arc.filter(|r| *r != 0.0),
        tolerance,
    };
    let (loops, _) = text::outlines(&spec)?;
    if loops.is_empty() {
        return Err("text: nothing to draw (only spaces?)".into());
    }
    Ok(CrossSection::from_polygons_with_fill_rule(&loops, FillRule::NonZero))
}

/// Outlines as a profile: counter-clockwise adds, clockwise cuts.
pub fn profile(loops: &[Vec<[f64; 2]>]) -> CrossSection {
    CrossSection::from_polygons_with_fill_rule(loops, FillRule::Positive)
}

/// [width, height, bits, has alpha] of a stored picture.
pub fn info(sha: &str) -> Result<Vec<f64>, String> {
    let p = crate::image::load(sha)?;
    Ok(vec![p.width as f64, p.height as f64, p.bits as f64, if p.has_alpha() { 1.0 } else { 0.0 }])
}

/// A picture's channel on an `nx × ny` grid spanning it edge to edge, rows
/// from the top.
pub fn grid(sha: &str, channel: &str, nx: usize, ny: usize) -> Result<Vec<f64>, String> {
    if nx < 2 || ny < 2 || nx * ny > 4_000_000 {
        return Err("image: a grid has 2 to 2000 points a side".into());
    }
    let p = crate::image::load(sha)?;
    let f: Field = p.field(Channel::parse(channel)?);
    Ok(f.resample(nx, ny).v.into_iter().map(|v| v as f64).collect())
}
