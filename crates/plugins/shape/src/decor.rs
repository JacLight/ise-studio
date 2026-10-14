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

/// A relief from a stored picture: adaptive within `budget` triangles, or
/// (`budget` 0) on a plain 64-cell grid for the exact kernel.
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
    if budget == 0 {
        return relief::build_regular(&picture, &spec, 64);
    }
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

/// The outline of the object a picture shows, measured across `rows` rows
/// (see `image::silhouette`), flat: top, bottom, axis, drift, cap top, cap bottom, then each row's
/// y, left, right (pixels).
pub fn silhouette(sha: &str, rows: usize, threshold: Option<f32>, rim: bool) -> Result<Vec<f64>, String> {
    if !(2..=2000).contains(&rows) {
        return Err("silhouette: 2 to 2000 rows".into());
    }
    let p = crate::image::load(sha)?;
    let s = crate::image::silhouette(&p, rows, threshold, rim)?;
    let mut out = vec![s.top, s.bottom, s.axis, s.drift, s.cap_top, s.cap_bottom];
    for r in &s.rows {
        out.extend([r.y, r.left, r.right]);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::runtime::evaluate;

    /// Stores a picture for programs to read (tests share one folder).
    pub(crate) fn stored(w: usize, h: usize, f: impl Fn(f64, f64) -> [u8; 4]) -> String {
        if crate::assets::dir().is_none() {
            crate::assets::set_dir(std::env::temp_dir().join(format!("ise-test-assets-{}", std::process::id())));
        }
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&f(x as f64 + 0.5, y as f64 + 0.5));
            }
        }
        crate::assets::store(&crate::image::encode_png(w as u32, h as u32, &px)).unwrap()
    }

    fn grey(v: f64) -> [u8; 4] {
        let g = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [g, g, g, 255]
    }

    /// A soft dome: white in the middle, black at the edges.
    fn dome() -> String {
        stored(120, 80, |x, y| grey(1.0 - (((x - 60.0) / 60.0).powi(2) + ((y - 40.0) / 40.0).powi(2)).sqrt()))
    }

    #[test]
    fn a_relief_plaque_is_sized_from_the_picture_and_closed() {
        let id = dome();
        let e = evaluate(&format!("return relief(\"{id}\", {{ width: 60, depth: 3, base: 2 }});"), &[], 0.01).unwrap();
        // 120 × 80 px at 60 mm wide: 40 mm tall; 2 mm back, up to 5 mm in the middle.
        assert!((e.bounds_max[0] - e.bounds_min[0] - 60.0).abs() < 1e-6 && (e.bounds_max[1] - e.bounds_min[1] - 40.0).abs() < 1e-6);
        assert!(e.bounds_min[2].abs() < 1e-9 && (e.bounds_max[2] - 5.0).abs() < 0.05, "{:?}", e.bounds_max);
        assert_eq!(e.genus, 0);
    }

    #[test]
    fn reliefs_emboss_and_engrave_faces_and_wrap_cylinders() {
        let id = dome();
        // Raised 1 mm off a 4 mm plate (base sunk into it), then carved 0.8 mm into another.
        let raised = evaluate(&format!("const plate = box(80, 60, 4, {{ center: true }}).translate(0, 0, -2);\nreturn plate.emboss(relief(\"{id}\", {{ width: 60, depth: 1, base: 0.3 }}).translate(0, 0, -0.3), {{ at: [0, 0, 2] }});"), &[], 0.01).unwrap();
        assert!((raised.bounds_max[2] - 3.0).abs() < 0.02, "{:?}", raised.bounds_max);
        let carved = evaluate(&format!("const plate = box(80, 60, 4, {{ center: true }});\nreturn plate.engrave(relief(\"{id}\", {{ width: 60, depth: 0.8, mode: \"engrave\" }}), {{ at: [0, 0, 2] }});"), &[], 0.01).unwrap();
        assert!(carved.volume < 80.0 * 60.0 * 4.0 - 100.0 && (carved.bounds_max[2] - 2.0).abs() < 1e-6, "{}", carved.volume);
        // Around a 30 mm mug: 60 mm of picture is about 115° of it.
        let mug = evaluate(&format!("return cylinder(80, 30).emboss(relief(\"{id}\", {{ width: 60, depth: 1, base: 0.3 }}).translate(0, 0, -0.3), {{ cylinder: {{ radius: 30, angle: 90, z: 40 }} }});"), &[], 0.02).unwrap();
        assert!(mug.bounds_max[1] > 30.9 && mug.bounds_max[1] < 31.05 && mug.genus == 0, "{:?}", mug.bounds_max);
    }

    #[test]
    fn a_lithophane_has_its_frame_and_light_is_thin() {
        let id = dome();
        let e = evaluate(&format!("return relief(\"{id}\", {{ width: 60, mode: \"lithophane\", border: 3 }});"), &[], 0.01).unwrap();
        assert!((e.bounds_max[2] - 3.2).abs() < 1e-6);
        // Thin (0.8 mm plus a little) at the light centre: a ray down the middle.
        let probe = evaluate(&format!("return relief(\"{id}\", {{ width: 60, mode: \"lithophane\", border: 3 }}).intersect(box(1, 1, 10, {{ center: true }}));"), &[], 0.01).unwrap();
        assert!(probe.bounds_max[2] < 0.9, "centre {} mm thick", probe.bounds_max[2]);
    }

    #[test]
    fn a_traced_logo_extrudes_and_cuts_through() {
        // A black ring on white, 100 px = 20 mm.
        let id = stored(100, 100, |x, y| {
            let r = ((x - 50.0).powi(2) + (y - 50.0).powi(2)).sqrt();
            grey(if (25.0..=45.0).contains(&r) { 0.0 } else { 1.0 })
        });
        let e = evaluate(&format!("const m = trace(\"{id}\", {{ width: 20 }});\nmetric(\"loops\", m.loops.length);\nreturn m.extrude(2);"), &[], 0.01).unwrap();
        assert_eq!(e.metrics["loops"].value, 2.0);
        let ring = std::f64::consts::PI * (9.0f64.powi(2) - 5.0f64.powi(2)) * 2.0;
        assert!((e.volume - ring).abs() / ring < 0.03, "{} vs {ring}", e.volume);
        // Cut through a 3 mm plate: the ring falls out.
        let cut = evaluate(&format!("return box(30, 30, 3, {{ center: true }}).engrave(trace(\"{id}\", {{ width: 20 }}), {{ at: [0, 0, 1.5], through: true }});"), &[], 0.01).unwrap();
        assert!((30.0 * 30.0 * 3.0 - cut.volume - ring * 1.5).abs() < 25.0, "{}", cut.volume);
    }

    #[test]
    fn text_engraves_embosses_and_wraps() {
        let flat = evaluate("return box(60, 20, 3).engrave(text(\"ISE\", { size: 8, weight: 700, align: \"center\", valign: \"middle\" }), { at: [30, 10, 3], depth: 0.6 });", &[], 0.01).unwrap();
        assert!(flat.volume < 60.0 * 20.0 * 3.0 - 20.0);
        let round = evaluate("return cylinder(30, 12).emboss(text(\"12 V\", { size: 5, align: \"center\", valign: \"middle\" }), { cylinder: { radius: 12, z: 15 }, height: 0.8 });", &[], 0.01).unwrap();
        assert!((round.bounds_max[0] - 12.8).abs() < 0.05, "{:?}", round.bounds_max);
        let rule = evaluate("return graduations({ length: 100, step: 1, every: [5, 10] }).extrude(0.4);", &[], 0.01).unwrap();
        assert!((rule.bounds_max[0] - rule.bounds_min[0] - 100.0).abs() < 4.0, "{:?} {:?}", rule.bounds_min, rule.bounds_max);
        let dial = evaluate("return graduations({ radius: 30, from: 0, to: 270, step: 5, every: [15, 45], values: [0, 10] }).extrude(0.4);", &[], 0.01).unwrap();
        assert!(dial.bounds_max[1] > 30.0);
    }

    #[test]
    fn knurls_grip_and_threads_fit() {
        let k = evaluate("return knurl({ radius: 10, length: 12, pitch: 1 });", &[], 0.01).unwrap();
        let full = std::f64::consts::PI * 100.0 * 12.0;
        assert!(k.volume < full && k.volume > 0.8 * full && (k.bounds_max[2] - 12.0).abs() < 1e-6, "{} of {full}", k.volume);
        // M8 × 1.25: the rod's volume sits between the minor and major cylinders.
        let bolt = evaluate("return thread({ diameter: 8, length: 20 });", &[], 0.005).unwrap();
        let (lo, hi) = (std::f64::consts::PI * (4.0 - 0.6766f64).powi(2) * 20.0, std::f64::consts::PI * 16.0 * 20.0);
        assert!(bolt.volume > lo && bolt.volume < hi && bolt.genus == 0, "{} in {lo}…{hi}", bolt.volume);
        assert!((bolt.bounds_max[0] - 4.0).abs() < 0.01);
        // A printed nut on the bolt: with clearance on both, nothing overlaps.
        let fit = evaluate(
            "const bolt = thread({ diameter: 8, length: 16, clearance: 0.1 });\n\
             const nut = cylinder(8, 7).translate(0, 0, 5).subtract(thread({ diameter: 8, length: 8, internal: true, clearance: 0.1, chamfer: false }).translate(0, 0, 5));\n\
             metric(\"overlap\", bolt.intersect(nut).volume);\nmetric(\"gap\", bolt.minGap(nut, 1));\nreturn [part(\"Bolt\", bolt), part(\"Nut\", nut)];",
            &[],
            0.005,
        )
        .unwrap();
        assert!(fit.metrics["overlap"].value < 1e-3, "{}", fit.metrics["overlap"].value);
        assert!(fit.metrics["gap"].value > 0.03 && fit.metrics["gap"].value < 0.2, "gap {}", fit.metrics["gap"].value);
    }

    #[test]
    fn the_exact_kernel_letters_carves_and_wraps_too() {
        // A fillet sends the whole program to the exact kernel.
        let id = dome();
        let e = evaluate(
            &format!(
                "const tag = box(40, 20, 5).fillet(1, {{ parallel: \"z\" }});\n\
                 const lettered = tag.engrave(text(\"A1\", {{ size: 6, weight: 700, align: \"center\", valign: \"middle\" }}), {{ at: [20, 10, 5], depth: 0.5 }});\n\
                 const logo = trace(\"{id}\", {{ width: 8, threshold: 0.3 }});\n\
                 metric(\"logo loops\", logo.loops.length);\n\
                 return lettered.add(relief(\"{id}\", {{ width: 12, depth: 0.5, base: 0.2 }}).translate(8, 10, 4.9), cylinder(4, 3).add(text(\"8\", {{ size: 2, align: \"center\" }}).extrude(0.4).wrap(3).translate(0, 0, 1)).translate(34, 10, 5));"
            ),
            &[],
            0.02,
        )
        .unwrap();
        assert!((3900.0..4300.0).contains(&e.volume) && (e.bounds_max[2] - 9.0).abs() < 1e-6, "{} {:?}", e.volume, e.bounds_max);
        // The dark rim of the dome: an outline and the light hole in it.
        assert_eq!(e.metrics["logo loops"].value, 2.0);
    }

    #[test]
    fn pictures_read_as_grids() {
        let id = stored(4, 2, |x, _| grey(if x < 2.0 { 0.0 } else { 1.0 }));
        let e = evaluate(&format!("const pic = image(\"{id}\");\nconst g = pic.grid(4, 2);\nmetric(\"w\", pic.width);\nmetric(\"left\", g[0][0]);\nmetric(\"right\", g[1][3]);\nreturn box(1, 1, 1);"), &[], 0.01).unwrap();
        assert_eq!((e.metrics["w"].value, e.metrics["left"].value, e.metrics["right"].value), (4.0, 0.0, 1.0));
        assert!(evaluate("return relief(\"abc\", { width: 10 });", &[], 0.01).unwrap_err().contains("not a picture id"));
    }
}
