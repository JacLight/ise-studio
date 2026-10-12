//! Exact evaluation: the same shape program, run against OpenCascade.
//!
//! Every geometry call the prelude makes has an exact counterpart here, so a
//! box stays six planes and a hole stays a true cylinder: what a machine shop
//! needs in a STEP file. A few operations have no exact form (a mesh the
//! program computed itself, convex hulls); those parts come out faceted and
//! are reported, never silently passed off as exact.

use crate::runtime::{DEADLINE_EXACT, Metric, Output, arg, run_program, throw};
use de_occt::{Arena, Blend, Bool, EdgeKind, Join, Shape};
use manifold_csg::{CrossSection, Manifold};
use rquickjs::{Ctx, Function, Object};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

/// A program evaluated exactly.
pub struct Exact {
    pub arena: Arena,
    /// Every part (or the single solid, named after the program).
    pub parts: Vec<ExactPart>,
    /// Operations that could only be represented with facets.
    pub faceted: Vec<String>,
    pub metrics: BTreeMap<String, Metric>,
    /// Hinges and gear meshes the program declared.
    pub mechanism: Option<serde_json::Value>,
    /// The program returned one solid rather than a list of parts.
    pub single: bool,
    /// What the configuration it was evaluated in left out.
    pub left_out: crate::runtime::LeftOut,
    /// Stock items and connectors the program declared.
    pub declared: crate::stock::Declared,
}

pub struct ExactPart {
    pub name: String,
    pub material: Option<String>,
    pub process: Option<String>,
    pub shape: Shape,
}

pub(crate) struct State {
    pub(crate) arena: Arena,
    pub(crate) faceted: Vec<String>,
    pub(crate) tolerance: f64,
}

/// Runs `source` exactly. `name` names a program that returns a single solid.
pub fn evaluate_exact(name: &str, source: &str, params: &[(String, f64)], tolerance: f64) -> Result<Exact, String> {
    match evaluate_exact_in(Arena::new(), name, source, params, tolerance, "") {
        Ok((arena, mut exact)) => {
            exact.arena = arena;
            Ok(exact)
        }
        Err((_, e)) => Err(e),
    }
}

/// Runs `source` exactly into an existing arena (so several programs can be
/// written to one STEP file). The arena comes back either way.
pub fn evaluate_exact_in(arena: Arena, name: &str, source: &str, params: &[(String, f64)], tolerance: f64, inputs: &str) -> Result<(Arena, Exact), (Arena, String)> {
    exact_within(arena, name, source, params, tolerance, inputs, DEADLINE_EXACT)
}

fn exact_within(arena: Arena, name: &str, source: &str, params: &[(String, f64)], tolerance: f64, inputs: &str, limit: std::time::Duration) -> Result<(Arena, Exact), (Arena, String)> {
    let state = Rc::new(RefCell::new(State { arena, faceted: Vec::new(), tolerance }));
    let output = run_program(source, params, tolerance, inputs, limit, |ctx| install(ctx, &state));
    let state = match Rc::try_unwrap(state) {
        Ok(s) => s.into_inner(),
        Err(_) => unreachable!("the sandbox is gone once run_program returns"),
    };
    let output: Output = match output {
        Ok(o) => o,
        Err(e) => return Err((state.arena, e)),
    };
    let single = output.solid.is_some();
    let parts = match (output.solid, output.parts) {
        (Some(h), _) => vec![ExactPart { name: name.to_string(), material: None, process: None, shape: shape(h) }],
        (None, Some(specs)) => specs.into_iter().map(|s| ExactPart { name: s.name, material: s.material, process: s.process, shape: shape(s.h) }).collect(),
        (None, None) => return Err((state.arena, "the program must return a Solid or a list of parts".into())),
    };
    let mut state = state;
    // Parts show in STEP in their material's colour (unless painted otherwise).
    for p in &parts {
        if let Some(c) = p.material.as_deref().and_then(crate::sweep::colour_of) {
            let _ = state.arena.paint_default(p.shape, c);
        }
    }
    let mut faceted = state.faceted;
    faceted.sort();
    faceted.dedup();
    Ok((state.arena, Exact { arena: Arena::empty(), parts, faceted, metrics: output.metrics, mechanism: output.mechanism, single, left_out: output.left_out.unwrap_or_default(), declared: output.declared }))
}

/// The view of a program that uses operations only the exact kernel has
/// (fillets, chamfers): evaluated exactly, then meshed from the true surfaces.
pub(crate) fn evaluate_meshed(source: &str, params: &[(String, f64)], tolerance: f64, inputs: &str) -> Result<crate::runtime::Evaluated, String> {
    let (mut arena, exact) = exact_within(Arena::new(), "", source, params, tolerance, inputs, crate::runtime::DEADLINE_MESHED).map_err(|(_, e)| e)?;
    let mut list = Vec::with_capacity(exact.parts.len());
    for p in &exact.parts {
        let m = to_manifold(&mut arena, p.shape, tolerance).map_err(|e| if exact.single { e } else { format!("{}: {e}", p.name) })?;
        list.push((p.name.clone(), p.material.clone(), p.process.clone(), m));
    }
    let mut e = if exact.single { crate::runtime::finish(list.pop().expect("one solid").3)? } else { crate::runtime::finish_parts(list)? };
    e.metrics = exact.metrics;
    e.mechanism = exact.mechanism;
    e.left_out = exact.left_out;
    e.declared = exact.declared;
    Ok(e)
}

/// One edge of an evaluated shape, for picking in the view (program coordinates, mm).
#[derive(serde::Serialize, Clone, Debug)]
pub struct PickEdge {
    /// "line", "circle" or "curve".
    pub kind: &'static str,
    pub points: Vec<[f64; 3]>,
    /// Lines: unit direction. Circles: the axis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
}

/// The edges of every part (or the single solid) as the program now builds
/// it, edge finishes included, with each part's bounds.
#[derive(serde::Serialize, Clone, Debug)]
pub struct PartEdges {
    /// The part's name; None for a program returning one solid.
    pub part: Option<String>,
    pub bounds: [f64; 6],
    pub edges: Vec<PickEdge>,
}

pub fn edges_of(source: &str, params: &[(String, f64)], tolerance: f64, inputs: &str) -> Result<Vec<PartEdges>, String> {
    let (mut arena, exact) = exact_within(Arena::new(), "", source, params, tolerance, inputs, crate::runtime::DEADLINE_MESHED).map_err(|(_, e)| e)?;
    let mut out = Vec::new();
    for p in &exact.parts {
        let bounds = arena.bounds(p.shape)?;
        let edges = arena
            .edges(p.shape, tolerance)?
            .into_iter()
            .filter_map(|e| {
                let (kind, dir, center, radius) = match e.kind {
                    EdgeKind::Line { dir, .. } => ("line", Some(dir), None, None),
                    EdgeKind::Circle { center, axis, radius } => ("circle", Some(axis), Some(center), Some(radius)),
                    EdgeKind::Curve { .. } => ("curve", None, None, None),
                    EdgeKind::Seam => return None,
                };
                Some(PickEdge { kind, points: e.points, dir, center, radius })
            })
            .collect();
        out.push(PartEdges { part: if exact.single { None } else { Some(p.name.clone()) }, bounds, edges });
    }
    Ok(out)
}

fn shape(h: u32) -> Shape {
    Shape::from_index(h)
}

fn handle(s: Shape) -> u32 {
    s.index() as u32
}

/// A mesh from the exact kernel, as a Manifold (for the operations only meshes have).
fn to_manifold(a: &mut Arena, s: Shape, tolerance: f64) -> Result<Manifold, String> {
    let m = a.mesh(s, tolerance)?;
    let tris: Vec<u64> = m.triangles.iter().map(|t| *t as u64).collect();
    Manifold::from_mesh_f64(&m.positions, 3, &tris).or_else(|_| {
        // Faces meet at shared edges but their triangulations do not share
        // vertices; weld them so the mesh is closed.
        let (pos, tri) = weld(&m.positions, &m.triangles, tolerance * 1e-3);
        let tris: Vec<u64> = tri.iter().map(|t| *t as u64).collect();
        Manifold::from_mesh_f64(&pos, 3, &tris).map_err(|e| format!("could not mesh the exact shape ({e})"))
    })
}

/// Joins vertices closer than `eps`: faces of an exact shape are meshed
/// separately, so their shared edges need welding into one closed mesh.
pub fn weld_mesh(positions: &[f64], triangles: &[u32], eps: f64) -> (Vec<f64>, Vec<u32>) {
    weld(positions, triangles, eps)
}

fn weld(positions: &[f64], triangles: &[u32], eps: f64) -> (Vec<f64>, Vec<u32>) {
    use std::collections::HashMap;
    let key = |p: &[f64]| ((p[0] / eps).round() as i64, (p[1] / eps).round() as i64, (p[2] / eps).round() as i64);
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut out = Vec::new();
    let mut remap = Vec::with_capacity(positions.len() / 3);
    for p in positions.chunks_exact(3) {
        let k = key(p);
        let i = *map.entry(k).or_insert_with(|| {
            out.extend_from_slice(p);
            (out.len() / 3 - 1) as u32
        });
        remap.push(i);
    }
    let tris = triangles
        .chunks_exact(3)
        .map(|t| [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]])
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .flatten()
        .collect();
    (out, tris)
}

fn from_manifold(a: &mut Arena, m: &Manifold) -> Result<Shape, String> {
    let (props, n, idx) = m.to_mesh_f64();
    let n = n.max(3);
    let pos: Vec<f64> = props.chunks_exact(n).flat_map(|v| [v[0], v[1], v[2]]).collect();
    let tri: Vec<u32> = idx.iter().map(|i| *i as u32).collect();
    a.from_mesh(&pos, &tri)
}

fn install<'js>(ctx: &Ctx<'js>, state: &Rc<RefCell<State>>) -> rquickjs::Result<()> {
    let native = Object::new(ctx.clone())?;
    let fail = |ctx: &Ctx<'js>, what: &str, e: String| throw(ctx, format!("{what}: {e}"));

    {
        let st = state.clone();
        native.set(
            "solid",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, args: Vec<f64>| -> rquickjs::Result<u32> {
                let v = |i| arg(&args, i, &ctx, &op);
                let mut s = st.borrow_mut();
                let r = match op.as_str() {
                    "cube" => {
                        let (x, y, z) = (v(0)?, v(1)?, v(2)?);
                        if x <= 0.0 || y <= 0.0 || z <= 0.0 {
                            return Err(throw(&ctx, "box: every size must be greater than zero"));
                        }
                        s.arena.cube(x, y, z, v(3)? != 0.0)
                    }
                    "cylinder" => {
                        let (h, r1, r2) = (v(0)?, v(1)?, v(2)?);
                        if h <= 0.0 || r1 < 0.0 || r2 < 0.0 || (r1 == 0.0 && r2 == 0.0) {
                            return Err(throw(&ctx, "cylinder: height must be positive and at least one radius positive"));
                        }
                        s.arena.cylinder(h, r1, r2, v(4)? != 0.0)
                    }
                    "sphere" => {
                        let r = v(0)?;
                        if r <= 0.0 {
                            return Err(throw(&ctx, "sphere: radius must be positive"));
                        }
                        s.arena.sphere(r)
                    }
                    other => return Err(throw(&ctx, format!("unknown solid {other}"))),
                };
                r.map(handle).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "sop",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let v = |i| arg(&args, i, &ctx, &op);
                let mut s = st.borrow_mut();
                let t = shape(h);
                let r = match op.as_str() {
                    "translate" => s.arena.translate(t, [v(0)?, v(1)?, v(2)?]),
                    "rotate" => s.arena.rotate(t, [v(0)?, v(1)?, v(2)?]),
                    "scale" => s.arena.scale(t, [v(0)?, v(1)?, v(2)?]),
                    "mirror" => s.arena.mirror(t, [v(0)?, v(1)?, v(2)?]),
                    "hull" => {
                        s.faceted.push("convex hull".into());
                        let tol = s.tolerance;
                        let m = to_manifold(&mut s.arena, t, tol).map_err(|e| fail(&ctx, "hull", e))?.hull();
                        from_manifold(&mut s.arena, &m)
                    }
                    other => return Err(throw(&ctx, format!("unknown operation {other}"))),
                };
                r.map(handle).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "sbool",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, a: u32, b: u32| -> rquickjs::Result<u32> {
                let kind = match op.as_str() {
                    "union" => Bool::Union,
                    "difference" => Bool::Difference,
                    "intersection" => Bool::Intersection,
                    other => return Err(throw(&ctx, format!("unknown boolean {other}"))),
                };
                st.borrow_mut().arena.boolean(kind, shape(a), shape(b)).map(handle).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "unionAll",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, handles: Vec<u32>| -> rquickjs::Result<u32> {
                let shapes: Vec<Shape> = handles.into_iter().map(shape).collect();
                st.borrow_mut().arena.union_all(&shapes).map(handle).map_err(|e| fail(&ctx, "union", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "squery",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32| -> rquickjs::Result<Vec<f64>> {
                let mut s = st.borrow_mut();
                let t = shape(h);
                Ok(match op.as_str() {
                    "volume" => vec![s.arena.volume(t)],
                    "area" => vec![s.arena.area(t)],
                    "bounds" => s.arena.bounds(t).map_err(|e| fail(&ctx, "bounds", e))?.to_vec(),
                    "genus" => {
                        let tol = s.tolerance;
                        vec![to_manifold(&mut s.arena, t, tol).map_err(|e| fail(&ctx, "genus", e))?.genus() as f64]
                    }
                    other => return Err(throw(&ctx, format!("unknown query {other}"))),
                })
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "minGap",
            Function::new(ctx.clone(), move |_ctx: Ctx<'js>, a: u32, b: u32, search: f64| -> rquickjs::Result<f64> {
                Ok(st.borrow().arena.distance(shape(a), shape(b)).unwrap_or(search).min(search))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "fromMesh",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, positions: Vec<f64>, triangles: Vec<f64>| -> rquickjs::Result<u32> {
                if positions.len() % 3 != 0 || triangles.len() % 3 != 0 || triangles.is_empty() {
                    return Err(throw(&ctx, "fromMesh: positions and triangles must be groups of three"));
                }
                let n = positions.len() / 3;
                let mut tris = Vec::with_capacity(triangles.len());
                for t in &triangles {
                    if !t.is_finite() || *t < 0.0 || t.fract() != 0.0 || (*t as usize) >= n {
                        return Err(throw(&ctx, format!("fromMesh: triangle index {t} is out of range")));
                    }
                    tris.push(*t as u32);
                }
                let mut s = st.borrow_mut();
                s.faceted.push("a mesh the program computed (fromMesh)".into());
                s.arena.from_mesh(&positions, &tris).map(handle).map_err(|e| fail(&ctx, "fromMesh", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "hullPoints",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, points: Vec<f64>| -> rquickjs::Result<u32> {
                if points.len() < 12 || points.len() % 3 != 0 {
                    return Err(throw(&ctx, "hullPoints: needs at least four [x, y, z] points"));
                }
                let pts: Vec<[f64; 3]> = points.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                let mut s = st.borrow_mut();
                s.faceted.push("convex hull of points".into());
                from_manifold(&mut s.arena, &Manifold::hull_pts(&pts)).map(handle).map_err(|e| fail(&ctx, "hullPoints", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "profile",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, args: Vec<f64>| -> rquickjs::Result<u32> {
                let v = |i| arg(&args, i, &ctx, &op);
                let mut s = st.borrow_mut();
                let r = match op.as_str() {
                    "circle" => {
                        let r = v(0)?;
                        if r <= 0.0 {
                            return Err(throw(&ctx, "circle: radius must be positive"));
                        }
                        s.arena.circle(r)
                    }
                    "square" => {
                        let (x, y) = (v(0)?, v(1)?);
                        if x <= 0.0 || y <= 0.0 {
                            return Err(throw(&ctx, "rect: sizes must be positive"));
                        }
                        s.arena.rect(x, y, v(2)? != 0.0)
                    }
                    other => return Err(throw(&ctx, format!("unknown profile {other}"))),
                };
                r.map(handle).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "polygons",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, contours: Vec<Vec<f64>>| -> rquickjs::Result<u32> {
                let mut polys = Vec::with_capacity(contours.len());
                for c in &contours {
                    if c.len() < 6 || c.len() % 2 != 0 || c.iter().any(|v| !v.is_finite()) {
                        return Err(throw(&ctx, "polygon: needs at least three finite [x, y] points"));
                    }
                    polys.push(c.chunks_exact(2).map(|p| [p[0], p[1]]).collect::<Vec<_>>());
                }
                st.borrow_mut().arena.polygons(&polys).map(handle).map_err(|e| fail(&ctx, "polygon", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "pop",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let v = |i| arg(&args, i, &ctx, &op);
                let mut s = st.borrow_mut();
                let t = shape(h);
                let r = match op.as_str() {
                    "translate" => s.arena.translate(t, [v(0)?, v(1)?, 0.0]),
                    "rotate" => s.arena.rotate(t, [0.0, 0.0, v(0)?]),
                    "scale" => s.arena.scale(t, [v(0)?, v(1)?, 1.0]),
                    "mirror" => s.arena.mirror(t, [v(0)?, v(1)?, 0.0]),
                    "offset" => {
                        let join = match v(1)? as i32 {
                            0 => Join::Square,
                            1 => Join::Round,
                            2 => Join::Miter,
                            _ => Join::Bevel,
                        };
                        s.arena.offset(t, v(0)?, join)
                    }
                    "hull" => {
                        // A 2D hull from the outline's points (faceted where it was curved).
                        s.faceted.push("convex hull of a profile".into());
                        let tol = s.tolerance;
                        let m = s.arena.mesh(t, tol).map_err(|e| fail(&ctx, "hull", e))?;
                        let pts: Vec<[f64; 2]> = m.positions.chunks_exact(3).map(|p| [p[0], p[1]]).collect();
                        let hull = CrossSection::from_polygons(&[pts]).hull();
                        let contours: Vec<Vec<[f64; 2]>> = hull.to_polygons();
                        s.arena.polygons(&contours)
                    }
                    other => return Err(throw(&ctx, format!("unknown profile operation {other}"))),
                };
                r.map(handle).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "pbool",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, a: u32, b: u32| -> rquickjs::Result<u32> {
                let kind = match op.as_str() {
                    "union" => Bool::Union,
                    "difference" => Bool::Difference,
                    "intersection" => Bool::Intersection,
                    other => return Err(throw(&ctx, format!("unknown boolean {other}"))),
                };
                st.borrow_mut().arena.boolean(kind, shape(a), shape(b)).map(handle).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "pquery",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32| -> rquickjs::Result<Vec<f64>> {
                let s = st.borrow();
                Ok(match op.as_str() {
                    "area" => vec![s.arena.area(shape(h))],
                    "bounds" => {
                        let b = s.arena.bounds(shape(h)).map_err(|e| fail(&ctx, "bounds", e))?;
                        vec![b[0], b[1], b[3], b[4]]
                    }
                    other => return Err(throw(&ctx, format!("unknown query {other}"))),
                })
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "extrude",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let v = |i| arg(&args, i, &ctx, "extrude");
                let height = v(0)?;
                if height <= 0.0 {
                    return Err(throw(&ctx, "extrude: height must be positive"));
                }
                let (slices, twist, sx, sy) = (v(1)? as i32, v(2)?, v(3)?, v(4)?);
                st.borrow_mut().arena.extrude(shape(h), height, twist, [sx, sy], slices).map(handle).map_err(|e| fail(&ctx, "extrude", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "revolve",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let degrees = arg(&args, 1, &ctx, "revolve")?;
                st.borrow_mut().arena.revolve(shape(h), degrees).map(handle).map_err(|e| fail(&ctx, "revolve", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "importStep",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, sha: String, _tolerance: f64| -> rquickjs::Result<String> {
                // The file's own exact geometry, untouched.
                let path = crate::assets::path(&sha).map_err(|e| throw(&ctx, e))?;
                let bodies = st.borrow_mut().arena.read_step(&path).map_err(|e| fail(&ctx, "importStep", e))?;
                let list: Vec<(String, u32)> = bodies.into_iter().map(|(n, s)| (n, handle(s))).collect();
                Ok(serde_json::to_string(&list).unwrap())
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "sketchProfile",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, loops: String, _tolerance: f64| -> rquickjs::Result<u32> {
                // True lines, arcs and circles: exact all the way to the STEP file.
                let loops: Vec<crate::sketch::Loop> = serde_json::from_str(&loops).map_err(|e| throw(&ctx, format!("sketch: {e}")))?;
                let data = crate::sketch::profile_data(&loops);
                st.borrow_mut().arena.sketch_profile(loops.len(), &data).map(handle).map_err(|e| fail(&ctx, "sketch", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "edges",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32| -> rquickjs::Result<String> {
                let mut s = st.borrow_mut();
                let tol = s.tolerance;
                let edges = s.arena.edges(shape(h), tol).map_err(|e| fail(&ctx, "edges", e))?;
                // [kind, facts…, points] per edge, in the kernel's numbering.
                let list: Vec<serde_json::Value> = edges
                    .iter()
                    .map(|e| match e.kind {
                        EdgeKind::Line { dir, length } => serde_json::json!({ "kind": "line", "dir": dir, "length": length, "points": e.points }),
                        EdgeKind::Circle { center, axis, radius } => serde_json::json!({ "kind": "circle", "center": center, "axis": axis, "radius": radius, "points": e.points }),
                        EdgeKind::Curve { length } => serde_json::json!({ "kind": "curve", "length": length, "points": e.points }),
                        EdgeKind::Seam => serde_json::json!({ "kind": "seam", "points": [] }),
                    })
                    .collect();
                Ok(serde_json::Value::Array(list).to_string())
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "faces",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32| -> rquickjs::Result<String> {
                let faces = st.borrow_mut().arena.faces(shape(h)).map_err(|e| fail(&ctx, "faces", e))?;
                let list: Vec<serde_json::Value> = faces.iter().map(|f| serde_json::json!({ "normal": f.normal, "center": f.center, "area": f.area })).collect();
                Ok(serde_json::Value::Array(list).to_string())
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "faceNear",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, x: f64, y: f64, z: f64| -> rquickjs::Result<u32> {
                st.borrow_mut().arena.face_near(shape(h), [x, y, z]).map(|i| i as u32).map_err(|e| fail(&ctx, "faces", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "pushFace",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, index: u32, distance: f64| -> rquickjs::Result<u32> {
                st.borrow_mut().arena.push_face(shape(h), index as usize, distance).map(handle).map_err(|e| fail(&ctx, "push", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "shell",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, thickness: f64, open: Vec<u32>| -> rquickjs::Result<u32> {
                let idx: Vec<usize> = open.iter().map(|e| *e as usize).collect();
                st.borrow_mut().arena.shell(shape(h), thickness, &idx).map(handle).map_err(|e| fail(&ctx, "shell", e))
            })?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "draft",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, h: u32, faces: Vec<u32>, degrees: f64, pull: Vec<f64>, neutral: Vec<f64>| -> rquickjs::Result<u32> {
                    if pull.len() != 3 || neutral.len() != 3 {
                        return Err(throw(&ctx, "draft: the pull direction and neutral point are [x, y, z]"));
                    }
                    let idx: Vec<usize> = faces.iter().map(|e| *e as usize).collect();
                    st.borrow_mut()
                        .arena
                        .draft(shape(h), &idx, degrees, [pull[0], pull[1], pull[2]], [neutral[0], neutral[1], neutral[2]])
                        .map(handle)
                        .map_err(|e| fail(&ctx, "draft", e))
                },
            )?,
        )?;
    }

    {
        let st = state.clone();
        native.set(
            "blend",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32, size: f64, edges: Vec<u32>| -> rquickjs::Result<u32> {
                let kind = match op.as_str() {
                    "fillet" => Blend::Fillet,
                    "chamfer" => Blend::Chamfer,
                    other => return Err(throw(&ctx, format!("unknown edge operation {other}"))),
                };
                let idx: Vec<usize> = edges.iter().map(|e| *e as usize).collect();
                st.borrow_mut().arena.blend(shape(h), kind, size, &idx).map(handle).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }

    // Sweeps, lofts, offsets and the rest (see `sweep`).
    crate::sweep::install_exact(ctx, &native, state)?;

    ctx.globals().set("__native", native)?;
    Ok(())
}
