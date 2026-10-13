//! Surface modelling for shape programs: the natives behind the prelude's
//! fillSurface, networkSurface, boundarySurface, sectionSurface, blendSurface,
//! blendCurve, sew, and the sheet methods (split, trimWith, extendFace,
//! offset, project, iso, border) and analysis (continuity, deviation,
//! curvature and draft samples).
//!
//! Exact only (see `de_occt::surfaces`): a program that uses them is
//! evaluated exactly and its true surfaces meshed for the view. A sheet that
//! is still open when returned is drawn as a thin skin (see `exact`).

use crate::runtime::{NEEDS_EXACT, throw};
use de_occt::{FillConstraint, FillOptions, Shape};
use rquickjs::{Ctx, Function, Object};
use std::{cell::RefCell, rc::Rc};

/// A filling as the program asks for it (`__n.fill(JSON)`).
#[derive(serde::Deserialize)]
struct FillSpec {
    /// Each: { segs } (a bounding curve), { h, edge, order } (a body's edge),
    /// { through: segs } (an inner curve), { point: [x, y, z] }.
    constraints: Vec<serde_json::Value>,
    #[serde(default)]
    degree: Option<u32>,
    #[serde(default)]
    points: Option<u32>,
    #[serde(default)]
    iterations: Option<u32>,
    #[serde(default)]
    max_degree: Option<u32>,
    #[serde(default)]
    segments: Option<u32>,
}

fn fill_constraints(spec: &FillSpec) -> Result<(Vec<FillConstraint>, FillOptions), String> {
    let mut out = Vec::new();
    let mut top = 0u8;
    let nums = |v: &serde_json::Value, what: &str| -> Result<Vec<f64>, String> {
        v.as_array().ok_or(format!("{what} must be a list of numbers"))?.iter().map(|x| x.as_f64().ok_or(format!("{what} must be numbers"))).collect()
    };
    for c in &spec.constraints {
        if let Some(s) = c.get("segs") {
            out.push(FillConstraint::Curve(nums(s, "a curve")?));
        } else if let Some(h) = c.get("h").and_then(|v| v.as_u64()) {
            let edge = c.get("edge").and_then(|v| v.as_u64()).ok_or("an edge constraint needs its edge")? as usize;
            let order = c.get("order").and_then(|v| v.as_u64()).unwrap_or(0).min(2) as u8;
            top = top.max(order);
            out.push(FillConstraint::Edge { body: Shape::from_index(h as u32), edge, order });
        } else if let Some(s) = c.get("through") {
            out.push(FillConstraint::Through(nums(s, "a curve")?));
        } else if let Some(p) = c.get("point") {
            let p = nums(p, "a point")?;
            if p.len() != 3 {
                return Err("a point is [x, y, z]".into());
            }
            out.push(FillConstraint::Point([p[0], p[1], p[2]]));
        } else {
            return Err("unknown fill constraint".into());
        }
    }
    let d = FillOptions::default();
    // Degree i + 2 for constraints of order i, as GeomPlate recommends; more points along curves for G2.
    let opts = FillOptions {
        degree: spec.degree.unwrap_or((top as u32 + 2).max(3)),
        points_per_curve: spec.points.unwrap_or(if top >= 1 { 30 } else { d.points_per_curve }),
        iterations: spec.iterations.unwrap_or(if top >= 1 { 3 } else { d.iterations }),
        max_degree: spec.max_degree.unwrap_or(d.max_degree),
        max_segments: spec.segments.unwrap_or(d.max_segments),
        tolerance: d.tolerance,
    };
    Ok((out, opts))
}

fn shapes(hs: &[u32]) -> Vec<Shape> {
    hs.iter().map(|h| Shape::from_index(*h)).collect()
}

/// The natives the exact kernel adds for surfaces.
pub(crate) fn install_exact<'js>(ctx: &Ctx<'js>, native: &Object<'js>, state: &Rc<RefCell<crate::exact::State>>) -> rquickjs::Result<()> {
    let fail = |ctx: &Ctx<'js>, what: &str, e: String| throw(ctx, format!("{what}: {e}"));
    let h32 = |s: Shape| s.index() as u32;
    let sh = |h: u32| Shape::from_index(h);
    macro_rules! native {
        ($name:literal, $what:literal, |$st:ident $(, $arg:ident : $ty:ty)*| -> $ret:ty $body:block) => {{
            let st = state.clone();
            native.set(
                $name,
                Function::new(ctx.clone(), move |ctx: Ctx<'js>, $($arg: $ty),*| -> rquickjs::Result<$ret> {
                    let $st = &mut st.borrow_mut().arena;
                    let r: Result<$ret, String> = $body;
                    r.map_err(|e| fail(&ctx, $what, e))
                })?,
            )?;
        }};
    }
    native!("curve", "curve", |a, segs: Vec<f64>| -> u32 { a.path(&segs).map(h32) });
    native!("edgeWire", "edges", |a, h: u32, edges: Vec<u32>| -> u32 {
        let e: Vec<usize> = edges.iter().map(|i| *i as usize).collect();
        a.edge_wire(sh(h), &e).map(h32)
    });
    native!("fill", "fillSurface", |a, spec: String| -> Vec<f64> {
        (|| {
            let spec: FillSpec = serde_json::from_str(&spec).map_err(|e| e.to_string())?;
            let (cons, opts) = fill_constraints(&spec)?;
            let (s, e) = a.fill(&cons, &opts)?;
            Ok(vec![h32(s) as f64, e.g0, e.g1.to_degrees(), e.g2])
        })()
    });
    native!("boundarySurface", "boundarySurface", |a, hs: Vec<u32>, style: i32| -> u32 { a.boundary_surface(&shapes(&hs), style).map(h32) });
    native!("sectionSurface", "sectionSurface", |a, hs: Vec<u32>, ruled: bool, closed: bool| -> u32 { a.section_surface(&shapes(&hs), ruled, closed).map(h32) });
    native!("networkSurface", "networkSurface", |a, ph: Vec<u32>, gh: Vec<u32>, tol: f64| -> Vec<f64> {
        a.network_surface(&shapes(&ph), &shapes(&gh), tol).map(|(s, gap, dev)| vec![h32(s) as f64, gap, dev])
    });
    native!("blendSurface", "blendSurface", |a, ha: u32, ea: Vec<u32>, hb: u32, eb: Vec<u32>, opts: Vec<f64>| -> u32 {
        let ea: Vec<usize> = ea.iter().map(|i| *i as usize).collect();
        let eb: Vec<usize> = eb.iter().map(|i| *i as usize).collect();
        let o = |i: usize, d: f64| opts.get(i).copied().unwrap_or(d);
        a.blend_surface(sh(ha), &ea, o(0, 2.0) as u8, o(1, 1.0), sh(hb), &eb, o(2, 2.0) as u8, o(3, 1.0), o(4, 0.0) as usize).map(h32)
    });
    native!("blendCurve", "blendCurve", |a, ha: u32, hb: u32, opts: Vec<f64>| -> u32 {
        let o = |i: usize, d: f64| opts.get(i).copied().unwrap_or(d);
        a.blend_curve(sh(ha), o(0, 1.0) != 0.0, o(1, 2.0) as u8, o(2, 1.0), sh(hb), o(3, 0.0) != 0.0, o(4, 2.0) as u8, o(5, 1.0)).map(h32)
    });
    native!("sew", "sew", |a, hs: Vec<u32>, tol: f64, solid: bool| -> Vec<f64> { a.sew(&shapes(&hs), tol, solid).map(|(s, free)| vec![h32(s) as f64, free as f64]) });
    native!("pieces", "pieces", |a, h: u32, kind: i32| -> Vec<u32> { a.pieces(sh(h), kind).map(|v| v.into_iter().map(h32).collect()) });
    native!("bodyInfo", "body", |a, h: u32| -> Vec<f64> {
        a.body_info(sh(h)).map(|b| vec![b.solids as f64, b.shells as f64, b.faces as f64, b.edges as f64, b.free_edges as f64, b.closed as i32 as f64])
    });
    native!("freeEdges", "border", |a, h: u32| -> Vec<u32> { a.free_edges(sh(h)).map(|v| v.into_iter().map(|i| i as u32).collect()) });
    native!("extendFace", "extendFace", |a, h: u32, face: u32, length: f64, sides: u32| -> u32 { a.extend_face(sh(h), face as usize, length, sides as u8).map(h32) });
    native!("offsetSheet", "offset", |a, h: u32, d: f64| -> u32 { a.offset_sheet(sh(h), d).map(h32) });
    native!("project", "project", |a, curve: u32, onto: u32, dir: Vec<f64>| -> u32 {
        let d = (dir.len() == 3).then(|| [dir[0], dir[1], dir[2]]);
        a.project(sh(curve), sh(onto), d).map(h32)
    });
    native!("iso", "iso", |a, h: u32, face: u32, v: bool, t: f64| -> u32 { a.iso(sh(h), face as usize, v, t).map(h32) });
    native!("split", "split", |a, h: u32, tools: Vec<u32>| -> u32 { a.split(sh(h), &shapes(&tools)).map(h32) });
    native!("trim", "trimWith", |a, h: u32, tools: Vec<u32>, at: Vec<f64>, drop: bool| -> u32 {
        if at.len() != 3 { Err("keep is a point [x, y, z]".to_string()) } else { a.trim(sh(h), &shapes(&tools), [at[0], at[1], at[2]], drop).map(h32) }
    });
    native!("continuity", "continuity", |a, h: u32, other: i32, samples: u32| -> String {
        a.continuity(sh(h), (other >= 0).then(|| Shape::from_index(other as u32)), samples as usize).map(|v| v.to_string())
    });
    native!("faceSamples", "analysis", |a, h: u32, n: u32| -> Vec<f64> {
        a.face_samples(sh(h), n as usize).map(|v| v.iter().flat_map(|s| [s.face as f64, s.p[0], s.p[1], s.p[2], s.n[0], s.n[1], s.n[2], s.k1, s.k2]).collect())
    });
    native!("deviation", "deviation", |a, ha: u32, hb: u32, n: u32| -> Vec<f64> {
        a.deviation(sh(ha), sh(hb), n as usize).map(|d| vec![d.max, d.mean, d.rms, d.worst_at[0], d.worst_at[1], d.worst_at[2], d.samples as f64])
    });
    Ok(())
}

/// The mesh kernel has no surfaces: a program using them is evaluated exactly.
pub(crate) fn install_mesh<'js>(ctx: &Ctx<'js>, native: &Object<'js>) -> rquickjs::Result<()> {
    let names = [
        "curve",
        "edgeWire",
        "fill",
        "boundarySurface",
        "sectionSurface",
        "networkSurface",
        "blendSurface",
        "blendCurve",
        "sew",
        "pieces",
        "bodyInfo",
        "freeEdges",
        "extendFace",
        "offsetSheet",
        "project",
        "iso",
        "split",
        "trim",
        "continuity",
        "faceSamples",
        "deviation",
    ];
    for name in names {
        native.set(name, Function::new(ctx.clone(), |ctx: Ctx<'js>, _a: rquickjs::function::Rest<rquickjs::Value<'js>>| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    }
    Ok(())
}
