//! Sandboxed evaluation of shape programs.
//!
//! Each evaluation gets a fresh QuickJS runtime with a memory limit and a
//! deadline, no clock, no unseeded randomness and no I/O. Geometry calls go to
//! native Manifold through numeric handles into a per-evaluation arena, so the
//! same program and parameters always produce the same solid.

use de_core::schemars::{self, JsonSchema};
use manifold_csg::{CrossSection, JoinType, Manifold};
use rquickjs::{Context, Ctx, Exception, Function, Object, Runtime, Value as JsValue};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

/// The vocabulary: the prelude, then the sheet-metal tools built on it.
const PRELUDE: &str = concat!(include_str!("prelude.js"), "\n", include_str!("sheet_metal.js"));
const MEMORY_LIMIT: usize = 512 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);
/// Exact booleans on curved geometry are slower than meshes; exports get longer.
pub(crate) const DEADLINE_EXACT: Duration = Duration::from_secs(120);
/// The view of a program with fillets or chamfers, evaluated exactly.
pub(crate) const DEADLINE_MESHED: Duration = Duration::from_secs(30);
/// Thrown by operations only the exact kernel has; the view then evaluates
/// the program exactly and meshes the true surfaces.
const NEEDS_EXACT: &str = "\u{1}needs the exact kernel";
/// Feature angle (degrees) above which normals stay sharp (crisp edges).
const SHARP_ANGLE: f64 = 40.0;

/// A successfully evaluated shape, in millimeters.
pub struct Evaluated {
    pub solid: Manifold,
    /// x, y, z per vertex.
    pub positions: Vec<f32>,
    /// nx, ny, nz per vertex (smooth on curves, split at sharp edges).
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub volume: f64,
    pub area: f64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub centroid: [f64; 3],
    pub genus: i32,
    pub triangles: usize,
    /// For assemblies: the individual parts, in the order the program listed
    /// them. Each owns a contiguous range of `indices`. Empty for a single solid.
    pub parts: Vec<EvaluatedPart>,
    /// Facts the program reported with metric().
    pub metrics: std::collections::BTreeMap<String, Metric>,
    /// Hinges and gear meshes the program declared (see `gears`).
    pub mechanism: Option<serde_json::Value>,
    /// What the configuration it was evaluated in left out.
    pub left_out: LeftOut,
    /// Stock items and connectors the program declared.
    pub declared: crate::stock::Declared,
    /// Sheet-metal parts' flat patterns and bend tables (see sheet_metal.js).
    pub sheets: Vec<SheetPattern>,
}

/// A sheet-metal part as the shop cuts it: the flat blank (T thick, its
/// corner at the origin, bend allowances included) and its bend table,
/// flat size and checks as sheet_metal.js worked them out.
pub struct SheetPattern {
    /// The part it unfolds (empty for a program returning one solid).
    pub part: String,
    pub flat: Manifold,
    pub data: serde_json::Value,
}

/// What a configuration left out of a program's result (see
/// `ConfigRow::parts_off`): the parts, and the motion that only they held.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct LeftOut {
    /// Parts left out, in the order the program made them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// Every part the program made, left out or not (only when some are).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<String>,
    /// Parts the configuration leaves out that the program does not make
    /// (renamed or removed since): reported, never silently ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Hinges, joints, gear meshes, drives and couplings that went with
    /// those parts, in words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<String>,
}

impl LeftOut {
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty() && self.missing.is_empty() && self.dropped.is_empty()
    }
}

/// One fabricable item of an assembly, in millimeters (program coordinates).
pub struct EvaluatedPart {
    pub name: String,
    /// Material name, resolved against the shape's material table.
    pub material: Option<String>,
    /// How the part is made: "3d_print", "cnc_mill", "cnc_router", "laser", …
    pub process: Option<String>,
    pub solid: Manifold,
    pub volume: f64,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub centroid: [f64; 3],
    /// First index into `Evaluated::indices`, and how many.
    pub index_start: usize,
    pub index_count: usize,
}

#[derive(serde::Deserialize)]
pub(crate) struct PartSpec {
    pub(crate) name: String,
    pub(crate) h: u32,
    pub(crate) material: Option<String>,
    pub(crate) process: Option<String>,
}

impl Evaluated {
    /// One part by name; one the configuration left out is said to be.
    pub fn part(&self, name: &str) -> Result<&EvaluatedPart, String> {
        self.parts.iter().find(|p| p.name == name).ok_or_else(|| {
            if self.left_out.parts.iter().any(|n| n == name) {
                format!("“{name}” is left out of this configuration: study it in one that has it")
            } else {
                format!("no part called “{name}”")
            }
        })
    }
}

impl std::fmt::Debug for Evaluated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Evaluated").field("volume", &self.volume).field("triangles", &self.triangles).finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Arena {
    solids: Vec<Manifold>,
    profiles: Vec<CrossSection>,
}

pub(crate) fn throw(ctx: &Ctx<'_>, message: impl AsRef<str>) -> rquickjs::Error {
    Exception::throw_message(ctx, message.as_ref())
}

pub(crate) fn arg(args: &[f64], i: usize, ctx: &Ctx<'_>, what: &str) -> rquickjs::Result<f64> {
    let v = *args.get(i).ok_or_else(|| throw(ctx, format!("{what}: missing argument {i}")))?;
    if v.is_finite() { Ok(v) } else { Err(throw(ctx, format!("{what}: argument {i} must be a finite number"))) }
}

/// Accuracy for the viewport: curves within 0.01 mm of the true formula.
pub const DISPLAY_TOLERANCE: f64 = 0.01;

/// Runs a program in a fresh sandbox whose geometry calls go to the natives
/// `install` provides (the mesh kernel, or the exact one), and returns what
/// it built (handles into that kernel's arena).
pub(crate) fn run_program<'f>(
    source: &str,
    params: &[(String, f64)],
    tolerance: f64,
    inputs: &str,
    limit: Duration,
    install: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<()> + 'f,
) -> Result<Output, String> {
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Err("tolerance must be a positive number of millimeters".into());
    }
    let rt = Runtime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(MEMORY_LIMIT);
    rt.set_max_stack_size(4 * 1024 * 1024);
    let deadline = Instant::now() + limit;
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let context = Context::full(&rt).map_err(|e| e.to_string())?;
    let output = context.with(|ctx| -> Result<Output, String> {
        install(&ctx).map_err(|e| format!("runtime setup failed: {e}"))?;
        ctx.globals().set("__tolerance", tolerance).map_err(|e| e.to_string())?;
        ctx.globals().set("__inputs", if inputs.is_empty() { "{}" } else { inputs }).map_err(|e| e.to_string())?;
        run(&ctx, PRELUDE, "prelude")?;
        let p = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
        for (key, value) in params {
            p.set(key.as_str(), *value).map_err(|e| e.to_string())?;
        }
        ctx.globals().set("__params", p).map_err(|e| e.to_string())?;
        let program = format!(
            "(() => {{ const p = Object.freeze(__params); delete globalThis.__params;\n\
             const __out = (function (p) {{\n{source}\n}})(p);\n\
             return __result(__out, p); }})()"
        );
        let value = run(&ctx, &program, "program")?;
        let text = value.as_string().and_then(|s| s.to_string().ok()).ok_or("the program must return a Solid or a list of parts")?;
        serde_json::from_str::<Output>(&text).map_err(|e| e.to_string())
    })?;
    if Instant::now() > deadline {
        return Err(format!("the program took longer than {} seconds", limit.as_secs()));
    }
    if output.parts.as_ref().is_some_and(|p| p.is_empty()) {
        let off = output.left_out.as_ref().map(|l| l.parts.iter().map(|n| format!("“{n}”")).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        return Err(format!("every part ({off}) is left out: a configuration has to keep at least one"));
    }
    Ok(output)
}

/// Evaluates `source` (a function body returning a Solid) with parameters `p`,
/// approximating curves to within `tolerance` mm of the true formula.
pub fn evaluate(source: &str, params: &[(String, f64)], tolerance: f64) -> Result<Evaluated, String> {
    evaluate_in(source, params, tolerance, "")
}

/// As [`evaluate`], with the program's inputs (its sketches, as JSON).
pub fn evaluate_in(source: &str, params: &[(String, f64)], tolerance: f64, inputs: &str) -> Result<Evaluated, String> {
    let arena = Rc::new(RefCell::new(Arena::default()));
    let output = match run_program(source, params, tolerance, inputs, DEADLINE, |ctx| install_native(ctx, &arena)) {
        Err(e) if e.contains(NEEDS_EXACT) => return crate::exact::evaluate_meshed(source, params, tolerance, inputs),
        r => r?,
    };
    let take = |h: u32| arena.borrow().solids.get(h as usize).map(|m| m.translate(0.0, 0.0, 0.0)).ok_or("the program returned an unknown solid");
    let result = match (output.solid, output.parts) {
        (Some(h), _) => finish(take(h)?),
        (None, Some(specs)) => {
            let mut parts = Vec::with_capacity(specs.len());
            for s in specs {
                parts.push((s.name, s.material, s.process, take(s.h)?));
            }
            finish_parts(parts)
        }
        (None, None) => Err("the program must return a Solid or a list of parts".into()),
    };
    let mut sheets = Vec::new();
    for (part, s) in output.sheet.unwrap_or_default() {
        sheets.push(SheetPattern { part, flat: take(s.flat)?, data: s.data });
    }
    result.map(|mut e| {
        e.metrics = output.metrics;
        e.mechanism = output.mechanism;
        e.left_out = output.left_out.unwrap_or_default();
        e.declared = output.declared;
        e.sheets = sheets;
        e
    })
}

#[derive(serde::Deserialize)]
pub(crate) struct Output {
    pub(crate) solid: Option<u32>,
    pub(crate) parts: Option<Vec<PartSpec>>,
    #[serde(default)]
    pub(crate) metrics: std::collections::BTreeMap<String, Metric>,
    /// How parts move: hinges and gear meshes the program declared.
    #[serde(default)]
    pub(crate) mechanism: Option<serde_json::Value>,
    /// What the configuration left out.
    #[serde(default)]
    pub(crate) left_out: Option<LeftOut>,
    /// Stock items and connectors.
    #[serde(default)]
    pub(crate) declared: crate::stock::Declared,
    /// Sheet-metal flat patterns, by part name.
    #[serde(default)]
    pub(crate) sheet: Option<std::collections::BTreeMap<String, SheetSpec>>,
}

#[derive(serde::Deserialize)]
pub(crate) struct SheetSpec {
    pub(crate) flat: u32,
    pub(crate) data: serde_json::Value,
}

/// A measured fact a program reports with metric(): area, keys, seats…
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize, JsonSchema)]
pub struct Metric {
    pub value: f64,
    #[serde(default)]
    pub unit: String,
}

/// Positions, normals and indices of a solid, normals split at sharp edges.
pub fn mesh_with_normals(solid: &Manifold) -> (Vec<f32>, Vec<f32>, Vec<u32>) {
    // Normal channel 0 is the first property after x, y, z: vertices export as [x, y, z, nx, ny, nz].
    let with_normals = solid.calculate_normals(0, SHARP_ANGLE);
    let (props, n_props, indices) = with_normals.to_mesh_f32();
    let n_props = n_props.max(3);
    let vertex_count = props.len() / n_props;
    let mut positions = Vec::with_capacity(vertex_count * 3);
    let mut normals = Vec::with_capacity(vertex_count * 3);
    for v in props.chunks_exact(n_props) {
        positions.extend_from_slice(&v[0..3]);
        if n_props >= 6 { normals.extend_from_slice(&v[3..6]) } else { normals.extend_from_slice(&[0.0, 0.0, 0.0]) }
    }
    (positions, normals, indices)
}

/// An assembly: every part stays its own solid (and its own mesh range); the
/// whole is their union, for contact checks against other shapes.
pub(crate) fn finish_parts(list: Vec<(String, Option<String>, Option<String>, Manifold)>) -> Result<Evaluated, String> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let mut parts = Vec::with_capacity(list.len());
    let (mut volume, mut area, mut moment) = (0.0, 0.0, [0.0; 3]);
    for (name, material, process, solid) in list {
        solid.status().map_err(|e| format!("{name}: invalid geometry: {e}"))?;
        if solid.is_empty() {
            return Err(format!("{name} is empty"));
        }
        let bounds = solid.bounding_box().ok_or_else(|| format!("{name} has no bounds"))?;
        let (p, n, i) = mesh_with_normals(&solid);
        let offset = (positions.len() / 3) as u32;
        let index_start = indices.len();
        let c = centroid(&p, &i);
        let v = solid.volume();
        indices.extend(i.iter().map(|k| k + offset));
        positions.extend_from_slice(&p);
        normals.extend_from_slice(&n);
        volume += v;
        area += solid.surface_area();
        for k in 0..3 {
            moment[k] += c[k] * v;
        }
        parts.push(EvaluatedPart {
            name,
            material,
            process,
            volume: v,
            bounds_min: bounds.min(),
            bounds_max: bounds.max(),
            centroid: c,
            index_start,
            index_count: indices.len() - index_start,
            solid,
        });
    }
    let whole = Manifold::batch_union(&parts.iter().map(|p| p.solid.translate(0.0, 0.0, 0.0)).collect::<Vec<_>>());
    let lo = (0..3).map(|k| parts.iter().map(|p| p.bounds_min[k]).fold(f64::INFINITY, f64::min)).collect::<Vec<_>>();
    let hi = (0..3).map(|k| parts.iter().map(|p| p.bounds_max[k]).fold(f64::NEG_INFINITY, f64::max)).collect::<Vec<_>>();
    Ok(Evaluated {
        volume,
        area,
        bounds_min: [lo[0], lo[1], lo[2]],
        bounds_max: [hi[0], hi[1], hi[2]],
        centroid: if volume > 0.0 { moment.map(|m| m / volume) } else { [0.0; 3] },
        genus: whole.genus(),
        triangles: indices.len() / 3,
        positions,
        normals,
        indices,
        solid: whole,
        parts,
        metrics: Default::default(),
        mechanism: None,
        left_out: LeftOut::default(),
        declared: Default::default(),
        sheets: Vec::new(),
    })
}

fn run<'js>(ctx: &Ctx<'js>, source: &str, label: &str) -> Result<JsValue<'js>, String> {
    use rquickjs::CatchResultExt;
    ctx.eval::<JsValue, _>(source).catch(ctx).map_err(|e| {
        let text = e.to_string();
        if text.contains("interrupted") {
            format!("{label}: stopped after {} seconds (infinite loop?)", DEADLINE.as_secs())
        } else {
            format!("{label}: {text}")
        }
    })
}

pub(crate) fn finish(solid: Manifold) -> Result<Evaluated, String> {
    solid.status().map_err(|e| format!("invalid geometry: {e}"))?;
    if solid.is_empty() {
        return Err("the program produced an empty solid".into());
    }
    let bounds = solid.bounding_box().ok_or("the solid has no bounds")?;
    let (positions, normals, indices) = mesh_with_normals(&solid);
    let centroid = centroid(&positions, &indices);
    Ok(Evaluated {
        volume: solid.volume(),
        area: solid.surface_area(),
        bounds_min: bounds.min(),
        bounds_max: bounds.max(),
        genus: solid.genus(),
        triangles: indices.len() / 3,
        positions,
        normals,
        indices,
        centroid,
        solid,
        parts: Vec::new(),
        metrics: Default::default(),
        mechanism: None,
        left_out: LeftOut::default(),
        declared: Default::default(),
        sheets: Vec::new(),
    })
}

/// Volume centroid by the divergence theorem over the closed mesh.
fn centroid(positions: &[f32], indices: &[u32]) -> [f64; 3] {
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    let mut volume = 0.0;
    let mut c = [0.0; 3];
    for t in indices.chunks_exact(3) {
        let (a, b, d) = (p(t[0]), p(t[1]), p(t[2]));
        let v = (a[0] * (b[1] * d[2] - b[2] * d[1]) - a[1] * (b[0] * d[2] - b[2] * d[0]) + a[2] * (b[0] * d[1] - b[1] * d[0])) / 6.0;
        volume += v;
        for k in 0..3 {
            c[k] += v * (a[k] + b[k] + d[k]) / 4.0;
        }
    }
    if volume.abs() < 1e-12 { [0.0; 3] } else { c.map(|x| x / volume) }
}

/// Reading pictures' pixels: the same in either kernel.
pub(crate) fn install_pictures<'js>(ctx: &Ctx<'js>, native: &Object<'js>) -> rquickjs::Result<()> {
    native.set(
        "imageInfo",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, sha: String| -> rquickjs::Result<Vec<f64>> { crate::decor::info(&sha).map_err(|e| throw(&ctx, format!("image: {e}"))) })?,
    )?;
    native.set(
        "imageGrid",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, sha: String, channel: String, nx: f64, ny: f64| -> rquickjs::Result<Vec<f64>> {
            crate::decor::grid(&sha, &channel, nx.max(0.0) as usize, ny.max(0.0) as usize).map_err(|e| throw(&ctx, format!("image: {e}")))
        })?,
    )?;
    Ok(())
}

fn install_native<'js>(ctx: &Ctx<'js>, arena: &Rc<RefCell<Arena>>) -> rquickjs::Result<()> {
    let native = Object::new(ctx.clone())?;

    let push_solid = |arena: &Rc<RefCell<Arena>>, m: Manifold| -> u32 {
        let mut a = arena.borrow_mut();
        a.solids.push(m);
        (a.solids.len() - 1) as u32
    };
    let push_profile = |arena: &Rc<RefCell<Arena>>, c: CrossSection| -> u32 {
        let mut a = arena.borrow_mut();
        a.profiles.push(c);
        (a.profiles.len() - 1) as u32
    };

    {
        let arena = arena.clone();
        native.set(
            "solid",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, args: Vec<f64>| -> rquickjs::Result<u32> {
                let m = match op.as_str() {
                    "cube" => {
                        let (x, y, z) = (arg(&args, 0, &ctx, "box")?, arg(&args, 1, &ctx, "box")?, arg(&args, 2, &ctx, "box")?);
                        if x <= 0.0 || y <= 0.0 || z <= 0.0 {
                            return Err(throw(&ctx, "box: every size must be greater than zero"));
                        }
                        Manifold::cube(x, y, z, arg(&args, 3, &ctx, "box")? != 0.0)
                    }
                    "cylinder" => {
                        let (h, r1, r2) = (arg(&args, 0, &ctx, "cylinder")?, arg(&args, 1, &ctx, "cylinder")?, arg(&args, 2, &ctx, "cylinder")?);
                        if h <= 0.0 || r1 < 0.0 || r2 < 0.0 || (r1 == 0.0 && r2 == 0.0) {
                            return Err(throw(&ctx, "cylinder: height must be positive and at least one radius positive"));
                        }
                        Manifold::cylinder(h, r1, r2, arg(&args, 3, &ctx, "cylinder")?.max(3.0) as i32, arg(&args, 4, &ctx, "cylinder")? != 0.0)
                    }
                    "sphere" => {
                        let r = arg(&args, 0, &ctx, "sphere")?;
                        if r <= 0.0 {
                            return Err(throw(&ctx, "sphere: radius must be positive"));
                        }
                        Manifold::sphere(r, arg(&args, 1, &ctx, "sphere")?.max(4.0) as i32)
                    }
                    other => return Err(throw(&ctx, format!("unknown solid {other}"))),
                };
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "sop",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let m = {
                    let a = arena.borrow();
                    let s = a.solids.get(h as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?;
                    let v = |i| arg(&args, i, &ctx, &op);
                    match op.as_str() {
                        "translate" => s.translate(v(0)?, v(1)?, v(2)?),
                        "rotate" => s.rotate(v(0)?, v(1)?, v(2)?),
                        "scale" => s.scale(v(0)?, v(1)?, v(2)?),
                        "mirror" => s.mirror([v(0)?, v(1)?, v(2)?]),
                        "hull" => s.hull(),
                        other => return Err(throw(&ctx, format!("unknown operation {other}"))),
                    }
                };
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "sbool",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, a: u32, b: u32| -> rquickjs::Result<u32> {
                let m = {
                    let ar = arena.borrow();
                    let (x, y) = (
                        ar.solids.get(a as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?,
                        ar.solids.get(b as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?,
                    );
                    match op.as_str() {
                        "union" => x.union(y),
                        "difference" => x.difference(y),
                        "intersection" => x.intersection(y),
                        other => return Err(throw(&ctx, format!("unknown boolean {other}"))),
                    }
                };
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "unionAll",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, handles: Vec<u32>| -> rquickjs::Result<u32> {
                let m = {
                    let a = arena.borrow();
                    let mut parts = Vec::with_capacity(handles.len());
                    for h in &handles {
                        parts.push(a.solids.get(*h as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?.translate(0.0, 0.0, 0.0));
                    }
                    Manifold::batch_union(&parts)
                };
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "squery",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32| -> rquickjs::Result<Vec<f64>> {
                let a = arena.borrow();
                let s = a.solids.get(h as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?;
                Ok(match op.as_str() {
                    "volume" => vec![s.volume()],
                    "area" => vec![s.surface_area()],
                    "genus" => vec![s.genus() as f64],
                    "bounds" => match s.bounding_box() {
                        Some(b) => [b.min(), b.max()].concat(),
                        None => vec![0.0; 6],
                    },
                    other => return Err(throw(&ctx, format!("unknown query {other}"))),
                })
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "minGap",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, a: u32, b: u32, search: f64| -> rquickjs::Result<f64> {
                let ar = arena.borrow();
                let (x, y) = (
                    ar.solids.get(a as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?,
                    ar.solids.get(b as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?,
                );
                Ok(x.min_gap(y, search))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "fromMesh",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, positions: Vec<f64>, triangles: Vec<f64>| -> rquickjs::Result<u32> {
                if positions.len() % 3 != 0 || triangles.len() % 3 != 0 || triangles.is_empty() {
                    return Err(throw(&ctx, "fromMesh: positions and triangles must be groups of three"));
                }
                let n = (positions.len() / 3) as u64;
                let mut tris = Vec::with_capacity(triangles.len());
                for t in &triangles {
                    if !t.is_finite() || *t < 0.0 || t.fract() != 0.0 || (*t as u64) >= n {
                        return Err(throw(&ctx, format!("fromMesh: triangle index {t} is out of range")));
                    }
                    tris.push(*t as u64);
                }
                let m = Manifold::from_mesh_f64(&positions, 3, &tris)
                    .map_err(|e| throw(&ctx, format!("fromMesh: not a closed manifold solid ({e})")))?;
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "hullPoints",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, points: Vec<f64>| -> rquickjs::Result<u32> {
                if points.len() < 12 || points.len() % 3 != 0 {
                    return Err(throw(&ctx, "hullPoints: needs at least four [x, y, z] points"));
                }
                let pts: Vec<[f64; 3]> = points.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                Ok(push_solid(&arena, Manifold::hull_pts(&pts)))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "profile",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, args: Vec<f64>| -> rquickjs::Result<u32> {
                let c = match op.as_str() {
                    "circle" => {
                        let r = arg(&args, 0, &ctx, "circle")?;
                        if r <= 0.0 {
                            return Err(throw(&ctx, "circle: radius must be positive"));
                        }
                        CrossSection::circle(r, arg(&args, 1, &ctx, "circle")?.max(3.0) as i32)
                    }
                    "square" => {
                        let (x, y) = (arg(&args, 0, &ctx, "rect")?, arg(&args, 1, &ctx, "rect")?);
                        if x <= 0.0 || y <= 0.0 {
                            return Err(throw(&ctx, "rect: sizes must be positive"));
                        }
                        CrossSection::square(x, y, arg(&args, 2, &ctx, "rect")? != 0.0)
                    }
                    other => return Err(throw(&ctx, format!("unknown profile {other}"))),
                };
                Ok(push_profile(&arena, c))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
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
                Ok(push_profile(&arena, CrossSection::from_polygons(&polys)))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "pop",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let c = {
                    let a = arena.borrow();
                    let p = a.profiles.get(h as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?;
                    let v = |i| arg(&args, i, &ctx, &op);
                    match op.as_str() {
                        "translate" => p.translate(v(0)?, v(1)?),
                        "rotate" => p.rotate(v(0)?),
                        "scale" => p.scale(v(0)?, v(1)?),
                        "mirror" => p.mirror(v(0)?, v(1)?),
                        "hull" => p.hull(),
                        "offset" => {
                            let join = match v(1)? as i32 {
                                0 => JoinType::Square,
                                1 => JoinType::Round,
                                2 => JoinType::Miter,
                                _ => JoinType::Bevel,
                            };
                            p.offset(v(0)?, join, 2.0, v(2)?.max(3.0) as i32)
                        }
                        other => return Err(throw(&ctx, format!("unknown profile operation {other}"))),
                    }
                };
                Ok(push_profile(&arena, c))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "pbool",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, a: u32, b: u32| -> rquickjs::Result<u32> {
                let c = {
                    let ar = arena.borrow();
                    let (x, y) = (
                        ar.profiles.get(a as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?,
                        ar.profiles.get(b as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?,
                    );
                    match op.as_str() {
                        "union" => x.union(y),
                        "difference" => x.difference(y),
                        "intersection" => x.intersection(y),
                        other => return Err(throw(&ctx, format!("unknown boolean {other}"))),
                    }
                };
                Ok(push_profile(&arena, c))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "pquery",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32| -> rquickjs::Result<Vec<f64>> {
                let a = arena.borrow();
                let p = a.profiles.get(h as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?;
                Ok(match op.as_str() {
                    "area" => vec![p.area()],
                    "bounds" => {
                        let r = p.bounds();
                        [r.min(), r.max()].concat()
                    }
                    other => return Err(throw(&ctx, format!("unknown query {other}"))),
                })
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "extrude",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let m = {
                    let a = arena.borrow();
                    let p = a.profiles.get(h as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?;
                    let v = |i| arg(&args, i, &ctx, "extrude");
                    let height = v(0)?;
                    if height <= 0.0 {
                        return Err(throw(&ctx, "extrude: height must be positive"));
                    }
                    Manifold::extrude_with_options(p, height, v(1)?.max(0.0) as i32, v(2)?, v(3)?, v(4)?)
                };
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "revolve",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, args: Vec<f64>| -> rquickjs::Result<u32> {
                let m = {
                    let a = arena.borrow();
                    let p = a.profiles.get(h as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?;
                    let segments = arg(&args, 0, &ctx, "revolve")?.max(3.0) as i32;
                    let degrees = arg(&args, 1, &ctx, "revolve")?;
                    // The profile's x is the radius and its y becomes height along Z.
                    Manifold::revolve(p, segments, degrees)
                };
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "importStep",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, sha: String, tolerance: f64| -> rquickjs::Result<String> {
                let bodies = crate::assets::meshes(&sha, tolerance).map_err(|e| throw(&ctx, format!("importStep: {e}")))?;
                let list: Vec<(String, u32, Option<String>)> = bodies.into_iter().map(|(n, m, c)| (n, push_solid(&arena, m), c.map(crate::assets::hex))).collect();
                Ok(serde_json::to_string(&list).unwrap())
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "importMesh",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, sha: String| -> rquickjs::Result<String> {
                let bodies = crate::assets::mesh_bodies(&sha).map_err(|e| throw(&ctx, format!("importMesh: {e}")))?;
                let list: Vec<(String, u32, Option<String>)> = bodies.into_iter().map(|(n, m, c)| (n, push_solid(&arena, m), c.map(crate::assets::hex))).collect();
                Ok(serde_json::to_string(&list).unwrap())
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "sketchProfile",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, loops: String, tolerance: f64| -> rquickjs::Result<u32> {
                let loops: Vec<crate::sketch::Loop> = serde_json::from_str(&loops).map_err(|e| throw(&ctx, format!("sketch: {e}")))?;
                let polys: Vec<Vec<[f64; 2]>> = loops.iter().map(|l| crate::sketch::polygon(l, tolerance)).collect();
                Ok(push_profile(&arena, CrossSection::from_polygons(&polys)))
            })?,
        )?;
    }

    install_pictures(ctx, &native)?;

    {
        let arena = arena.clone();
        native.set(
            "relief",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, sha: String, options: String, tolerance: f64| -> rquickjs::Result<u32> {
                let m = crate::decor::relief(&sha, &options, tolerance, crate::decor::relief_budget(tolerance)).map_err(|e| throw(&ctx, e))?;
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "trace",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, sha: String, options: String, tolerance: f64| -> rquickjs::Result<u32> {
                let loops = crate::decor::trace(&sha, &options, tolerance).map_err(|e| throw(&ctx, e))?;
                Ok(push_profile(&arena, crate::decor::profile(&loops)))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "text",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, options: String, tolerance: f64| -> rquickjs::Result<u32> {
                let c = crate::decor::text(&options, tolerance).map_err(|e| throw(&ctx, e))?;
                Ok(push_profile(&arena, c))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "wrap",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, radius: f64, tolerance: f64| -> rquickjs::Result<u32> {
                let m = {
                    let a = arena.borrow();
                    let s = a.solids.get(h as usize).ok_or_else(|| throw(&ctx, "unknown solid"))?;
                    crate::relief::wrap(s, radius, tolerance).map_err(|e| throw(&ctx, e))?
                };
                Ok(push_solid(&arena, m))
            })?,
        )?;
    }

    {
        let arena = arena.clone();
        native.set(
            "ppoints",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32| -> rquickjs::Result<String> {
                let a = arena.borrow();
                let p = a.profiles.get(h as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?;
                Ok(serde_json::to_string(&p.to_polygons()).unwrap())
            })?,
        )?;
    }

    // Rounding, bevelling and pushing faces need the true B-rep: the whole program is then
    // evaluated exactly (see `evaluate_in`).
    native.set("edges", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32| -> rquickjs::Result<String> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set("faces", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32| -> rquickjs::Result<String> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set("faceNear", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32, _x: f64, _y: f64, _z: f64| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set(
        "pushFace",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32, _i: u32, _d: f64| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?,
    )?;
    native.set(
        "blend",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, _op: String, _h: u32, _size: f64, _edges: Vec<u32>| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?,
    )?;
    native.set(
        "shell",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32, _t: f64, _open: Vec<u32>| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?,
    )?;
    native.set(
        "draft",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32, _faces: Vec<u32>, _deg: f64, _pull: Vec<f64>, _neutral: Vec<f64>| -> rquickjs::Result<u32> {
            Err(throw(&ctx, NEEDS_EXACT))
        })?,
    )?;

    ctx.globals().set("__native", native)?;
    Ok(())
}
