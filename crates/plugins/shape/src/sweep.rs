//! Paths, sweeps and lofts, and the rest of the exact solid vocabulary
//! (offsets, thickening, variable fillets, defeaturing, paint, geometry
//! checks), for both kernels.
//!
//! Exactly, everything is OpenCascade (see `de_occt::solids`). For the view a
//! sweep is meshed here, section by section, in the very frames the exact
//! sweep rides in (OpenCascade samples the path for both), so the mesh lies
//! within the tolerance of the exact solid. The rest have no mesh form: a
//! program using them is evaluated exactly and its true surfaces meshed.

use crate::runtime::{NEEDS_EXACT, throw};
use de_occt::{Arena as Occ, Blend, FrameSample, Orientation, Place, Shape};
use manifold_csg::{CrossSection, Manifold};
use rquickjs::{Ctx, Function, Object};
use std::{cell::RefCell, rc::Rc};

/// A sweep as the program asks for it (`__n.sweep(h, JSON)`).
#[derive(serde::Deserialize)]
struct SweepSpec {
    segs: Vec<f64>,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    binormal: Option<[f64; 3]>,
    #[serde(default)]
    aux: Option<Vec<f64>>,
    #[serde(default)]
    place: String,
    #[serde(default)]
    up: Option<[f64; 3]>,
    #[serde(default)]
    frame: Option<[f64; 9]>,
    #[serde(default)]
    twist: f64,
    #[serde(default = "one")]
    scale: f64,
}

fn one() -> f64 {
    1.0
}

impl SweepSpec {
    fn orientation(&self) -> Result<Orientation, String> {
        Ok(match self.mode.as_str() {
            "" | "corrected" => Orientation::Corrected,
            "frenet" => Orientation::Frenet,
            "fixed" => Orientation::Fixed,
            "binormal" => Orientation::Binormal(self.binormal.unwrap_or([0.0, 0.0, 1.0])),
            "auxiliary" => Orientation::Auxiliary(self.aux.clone().ok_or("an auxiliary orientation needs the auxiliary path")?),
            other => return Err(format!("unknown orientation “{other}” (corrected, frenet, fixed, binormal or auxiliary)")),
        })
    }
    fn place(&self) -> Result<Place, String> {
        Ok(match self.place.as_str() {
            "" | "normal" => Place::Normal { up: self.up.unwrap_or([0.0, 0.0, 1.0]) },
            "axial" => Place::Axial,
            "frame" => {
                let f = self.frame.ok_or("a placed profile needs its origin, x and y")?;
                Place::Frame { origin: [f[0], f[1], f[2]], x: [f[3], f[4], f[5]], y: [f[6], f[7], f[8]] }
            }
            other => return Err(format!("unknown placement “{other}” (normal, axial, or {{ origin, x, y }})")),
        })
    }
    fn check(&self) -> Result<(), String> {
        if !(self.scale.is_finite() && self.scale > 0.0) {
            return Err("the end scale must be a positive number".into());
        }
        if !self.twist.is_finite() || self.segs.iter().any(|v| !v.is_finite()) {
            return Err("the path and twist must be finite numbers".into());
        }
        Ok(())
    }
}

/// Loft sections: profile handles (-1 for a point) and their placements.
#[derive(serde::Deserialize)]
struct LoftSpec {
    sections: Vec<(i64, [f64; 12])>,
    #[serde(default)]
    ruled: bool,
    #[serde(default)]
    closed: bool,
    #[serde(default = "two")]
    continuity: i32,
}

fn two() -> i32 {
    2
}

type V3 = [f64; 3];
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The outlines of a profile, grouped so each group is one outline followed
/// by its holes (as the triangulator wants them).
fn grouped(profile: &CrossSection) -> Vec<Vec<Vec<[f64; 2]>>> {
    let area = |c: &[[f64; 2]]| {
        let mut a = 0.0;
        for i in 0..c.len() {
            let (p, q) = (c[i], c[(i + 1) % c.len()]);
            a += p[0] * q[1] - q[0] * p[1];
        }
        a / 2.0
    };
    profile
        .decompose()
        .iter()
        .map(|part| {
            let mut cs = part.to_polygons();
            cs.sort_by(|a, b| area(b).partial_cmp(&area(a)).unwrap_or(std::cmp::Ordering::Equal));
            cs
        })
        .filter(|cs| !cs.is_empty())
        .collect()
}

/// Sweeps a profile (as polygons) along a path for the view: a ring of the
/// profile's points per sample, in the exact sweep's frames, capped at the ends.
pub(crate) fn mesh_sweep(profile: &CrossSection, spec_json: &str, tolerance: f64) -> Result<Manifold, String> {
    let spec: SweepSpec = serde_json::from_str(spec_json).map_err(|e| format!("sweep: {e}"))?;
    spec.check()?;
    let groups = grouped(profile);
    if groups.is_empty() {
        return Err("the profile is empty".into());
    }
    let (orient, place) = (spec.orientation()?, spec.place()?);
    let mut occ = Occ::new();
    // First where the profile sits (to know how far it reaches from the path).
    let probe = occ.path_frames(&spec.segs, &orient, &place, 0.5, 0.0)?;
    let [o, x, y, z] = probe.place;
    let p0 = probe.samples[0].p;
    let world = |q: [f64; 2]| add(o, add(mul(x, q[0]), mul(y, q[1])));
    let reach = groups.iter().flatten().flatten().map(|q| {
        let d = sub(world(*q), p0);
        dot(d, d).sqrt()
    });
    let reach = reach.fold(0.0f64, f64::max).max(1e-6) * spec.scale.max(1.0);
    let dphi = if reach <= tolerance { 0.5 } else { (2.0 * (1.0 - tolerance / reach).acos()).clamp(1e-3, 0.5) };
    // Each profile point in the start frame: across (n, b) and along (t).
    let f0 = probe.samples[0];
    let contours: Vec<&Vec<[f64; 2]>> = groups.iter().flatten().collect();
    let local: Vec<V3> = contours
        .iter()
        .flat_map(|c| c.iter())
        .map(|q| {
            let d = sub(world(*q), f0.p);
            [dot(d, f0.n), dot(d, f0.b), dot(d, f0.t)]
        })
        .collect();
    // The profile's normal against the path: a profile facing back sweeps inside out.
    let facing = dot(z, f0.t);
    if facing.abs() < 1e-6 {
        return Err("the profile lies along the path at its start: it would sweep no volume".into());
    }
    let n = local.len();
    // Rings of the profile's points, finer until the facets lie within the
    // tolerance: the chords along the sweep (sagitta, from the second
    // difference) and the quads between rings (a twisted quad bows off its
    // two triangles by a quarter of its corner's lift).
    let mut dphi = dphi;
    let mut built = None;
    for _ in 0..6 {
        let frames = occ.path_frames(&spec.segs, &orient, &place, dphi, spec.twist)?;
        let s = &frames.samples;
        let length = s.last().map(|f| f.s).unwrap_or(0.0);
        if length <= 0.0 {
            return Err("the path has no length".into());
        }
        let place_at = |f: &FrameSample, c: V3| -> V3 {
            let u = f.s / length;
            let (k, th) = (1.0 + (spec.scale - 1.0) * u, spec.twist.to_radians() * u);
            let (cn, cb) = (c[0] * th.cos() - c[1] * th.sin(), c[0] * th.sin() + c[1] * th.cos());
            add(f.p, mul(add(add(mul(f.n, cn), mul(f.b, cb)), mul(f.t, c[2])), k))
        };
        // A corner (two samples at one place) becomes one mitred ring.
        let mut rings: Vec<Vec<V3>> = Vec::new();
        let mut corner = Vec::new();
        let mut i = 0;
        while i < s.len() {
            if i + 1 < s.len() && (s[i + 1].s - s[i].s).abs() < 1e-12 {
                let (a, b) = (s[i], s[i + 1]);
                let m = add(a.t, b.t);
                let m = mul(m, 1.0 / dot(m, m).sqrt().max(1e-12));
                rings.push(
                    local
                        .iter()
                        .map(|c| {
                            let q = place_at(&a, *c);
                            sub(q, mul(a.t, dot(sub(q, a.p), m) / dot(a.t, m)))
                        })
                        .collect(),
                );
                corner.push(true);
                i += 2;
            } else {
                rings.push(local.iter().map(|c| place_at(&s[i], *c)).collect());
                corner.push(false);
                i += 1;
            }
        }
        let (mut sag, mut lift) = (0.0f64, 0.0f64);
        for r in 1..rings.len() {
            let mut start = 0;
            for c in &contours {
                let m = c.len();
                for k in 0..m {
                    let (a, b) = (start + k, start + (k + 1) % m);
                    let (a0, b0, a1, b1) = (rings[r - 1][a], rings[r - 1][b], rings[r][a], rings[r][b]);
                    let nrm = cross(sub(b0, a0), sub(a1, a0));
                    let l = dot(nrm, nrm).sqrt();
                    if l > 1e-15 {
                        lift = lift.max((dot(nrm, sub(b1, a0)) / l).abs() / 4.0);
                    }
                    if r + 1 < rings.len() && !corner[r] {
                        let d2 = add(sub(rings[r + 1][a], mul(a1, 2.0)), a0);
                        sag = sag.max(dot(d2, d2).sqrt() / 8.0);
                    }
                }
                start += m;
            }
        }
        let last = s[s.len() - 1];
        let worst = sag.max(lift);
        built = Some((rings, last));
        if worst <= tolerance || dphi <= 1e-3 {
            break;
        }
        // Lift shrinks with the step, sagitta with its square.
        let k = if lift >= sag { tolerance / lift } else { (tolerance / sag).sqrt() };
        dphi = (dphi * k * 0.9).max(1e-3);
    }
    let (mut rings, last) = built.expect("at least one pass");
    // A closed path whose last ring lands on the first joins up instead of capping.
    let closed = rings.len() > 2
        && dot(sub(last.p, f0.p), sub(last.p, f0.p)).sqrt() < 1e-6
        && rings[rings.len() - 1].iter().zip(&rings[0]).all(|(a, b)| dot(sub(*a, *b), sub(*a, *b)).sqrt() < tolerance.max(1e-6));
    if closed {
        rings.pop();
    }
    let mut pos: Vec<f64> = Vec::with_capacity(rings.len() * n * 3);
    for r in &rings {
        for p in r {
            pos.extend_from_slice(p);
        }
    }
    let flip = facing < 0.0;
    let mut tri: Vec<u64> = Vec::new();
    let mut put = |a: usize, b: usize, c: usize| {
        if flip {
            tri.extend([a as u64, c as u64, b as u64]);
        } else {
            tri.extend([a as u64, b as u64, c as u64]);
        }
    };
    let nr = rings.len();
    let spans = if closed { nr } else { nr - 1 };
    let mut start = 0;
    for c in &contours {
        let m = c.len();
        for r in 0..spans {
            let (r0, r1) = (r * n, ((r + 1) % nr) * n);
            for k in 0..m {
                let (a, b) = (start + k, start + (k + 1) % m);
                put(r0 + a, r0 + b, r1 + b);
                put(r0 + a, r1 + b, r1 + a);
            }
        }
        start += m;
    }
    if !closed {
        let mut base = 0;
        for g in &groups {
            let count: usize = g.iter().map(|c| c.len()).sum();
            let tris = manifold_csg::triangulate_polygons(g, 1e-9).ok_or("could not cap the sweep (is the profile degenerate?)")?;
            let end = (nr - 1) * n;
            for t in &tris {
                let [a, b, c] = t.map(|v| base + v as usize);
                put(c, b, a);
                put(end + a, end + b, end + c);
            }
            base += count;
        }
    }
    let m = Manifold::from_mesh_f64(&pos, 3, &tri).map_err(|e| format!("the sweep is not a closed solid ({e})"))?;
    if m.volume() < 0.0 { Err("the sweep came out inside out".into()) } else { Ok(m) }
}

/// A path's length and its ends: [length, start xyz, end xyz, start tangent, end tangent].
fn path_info(segs: &[f64]) -> Result<Vec<f64>, String> {
    path_info_in(&mut Occ::new(), segs)
}

/// As `path_info`, in an arena whose shapes the path may name (edges taken from bodies).
fn path_info_in(occ: &mut Occ, segs: &[f64]) -> Result<Vec<f64>, String> {
    let f = occ.path_frames(segs, &Orientation::Corrected, &Place::Normal { up: [0.0, 0.0, 1.0] }, 0.5, 0.0)?;
    let (a, b) = (f.samples[0], f.samples[f.samples.len() - 1]);
    Ok([&[b.s][..], &a.p, &b.p, &a.t, &b.t].concat())
}

/// "#rrggbb" or a colour word, as sRGB 0–1 (the view's palette, so a STEP file
/// shows parts as Forma does).
pub fn colour_of(text: &str) -> Option<[f64; 3]> {
    let key = text.to_lowercase();
    let hex = |h: &str| -> Option<[f64; 3]> {
        let v = u32::from_str_radix(h, 16).ok()?;
        Some([((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0])
    };
    if let Some(i) = key.find('#') {
        if let Some(c) = key.get(i + 1..i + 7).and_then(hex) {
            return Some(c);
        }
    }
    const WORDS: &[(&str, &str)] = &[
        ("dark grey", "4a4d52"),
        ("dark gray", "4a4d52"),
        ("charcoal", "3a3c40"),
        ("black", "232427"),
        ("silver", "c3c7cb"),
        ("white", "f1f0ec"),
        ("ivory", "eee8d8"),
        ("natural", "e8e1cf"),
        ("beige", "d9c9a8"),
        ("orange", "e8742c"),
        ("yellow", "f0c430"),
        ("purple", "7a4fc4"),
        ("violet", "7a4fc4"),
        ("green", "3e9a54"),
        ("olive", "6f7a3a"),
        ("brown", "7a5236"),
        ("grey", "8d9196"),
        ("gray", "8d9196"),
        ("teal", "2a8c8c"),
        ("cyan", "3fb3cf"),
        ("blue", "3a6fd0"),
        ("navy", "243a66"),
        ("pink", "e27aa8"),
        ("gold", "c9a24a"),
        ("red", "c8372f"),
    ];
    let word = |w: &str| key.split(|c: char| !c.is_alphanumeric() && c != ' ').any(|part| part.trim() == w) || key.split(|c: char| !c.is_alphanumeric()).any(|p| p == w);
    for (w, h) in WORDS {
        if (w.contains(' ') && key.contains(w)) || word(w) {
            return hex(h);
        }
    }
    const FAMILIES: &[(&str, &str)] = &[
        ("stainless", "d0d5d9"),
        ("galvanised", "b8c0c6"),
        ("aluminium", "c8d2da"),
        ("aluminum", "c8d2da"),
        ("brass", "c9a85c"),
        ("copper", "c27c56"),
        ("titanium", "9aa2a8"),
        ("steel", "a9b6bf"),
        ("pla", "4f7dd9"),
        ("petg", "e86a3a"),
        ("abs", "e7e2d8"),
        ("nylon", "ece9e1"),
        ("acetal", "f1eee6"),
        ("acrylic", "dfeef5"),
        ("rubber", "25272b"),
        ("silicone", "d84a3a"),
        ("glass", "8fb9cc"),
        ("concrete", "c3bfb6"),
        ("plywood", "e3cb9f"),
        ("birch", "e3cb9f"),
        ("walnut", "6f4a2f"),
        ("oak", "c69c6d"),
        ("mdf", "b99b77"),
        ("fr4", "2f7a4a"),
    ];
    FAMILIES.iter().find(|(f, _)| key.split(|c: char| !c.is_alphanumeric()).any(|p| p == *f) || key.contains(f) && f.len() > 4).and_then(|(_, h)| hex(h))
}

fn sweep_exact(occ: &mut Occ, h: Shape, spec_json: &str) -> Result<Shape, String> {
    let spec: SweepSpec = serde_json::from_str(spec_json).map_err(|e| format!("sweep: {e}"))?;
    spec.check()?;
    occ.sweep(h, &spec.segs, &spec.orientation()?, &spec.place()?, spec.twist, spec.scale)
}

fn loft_exact(occ: &mut Occ, spec_json: &str) -> Result<Shape, String> {
    let spec: LoftSpec = serde_json::from_str(spec_json).map_err(|e| format!("loft: {e}"))?;
    let sections: Vec<(Option<Shape>, [f64; 12])> = spec.sections.iter().map(|(h, m)| ((*h >= 0).then(|| Shape::from_index(*h as u32)), *m)).collect();
    occ.loft(&sections, spec.ruled, spec.closed, true, spec.continuity)
}

/// The natives the exact kernel adds for this vocabulary.
pub(crate) fn install_exact<'js>(ctx: &Ctx<'js>, native: &Object<'js>, state: &Rc<RefCell<crate::exact::State>>) -> rquickjs::Result<()> {
    let fail = |ctx: &Ctx<'js>, what: &str, e: String| throw(ctx, format!("{what}: {e}"));
    let h32 = |s: Shape| s.index() as u32;
    let sh = |h: u32| Shape::from_index(h);
    {
        let st = state.clone();
        native.set(
            "sweep",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, spec: String| -> rquickjs::Result<u32> {
                sweep_exact(&mut st.borrow_mut().arena, sh(h), &spec).map(h32).map_err(|e| fail(&ctx, "sweep", e))
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "loft",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, spec: String| -> rquickjs::Result<u32> {
                loft_exact(&mut st.borrow_mut().arena, &spec).map(h32).map_err(|e| fail(&ctx, "loft", e))
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "pathInfo",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, segs: Vec<f64>| -> rquickjs::Result<Vec<f64>> { path_info_in(&mut st.borrow_mut().arena, &segs).map_err(|e| fail(&ctx, "path", e)) })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "offsetSolid",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, d: f64, sharp: bool| -> rquickjs::Result<u32> {
                st.borrow_mut().arena.offset_solid(sh(h), d, sharp).map(h32).map_err(|e| fail(&ctx, "offset", e))
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "thicken",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, t: f64, faces: Vec<u32>, sharp: bool| -> rquickjs::Result<u32> {
                let f: Vec<usize> = faces.iter().map(|i| *i as usize).collect();
                st.borrow_mut().arena.thicken(sh(h), t, &f, sharp).map(h32).map_err(|e| fail(&ctx, "thicken", e))
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "blendVar",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, op: String, h: u32, a: f64, b: f64, edges: Vec<u32>| -> rquickjs::Result<u32> {
                let kind = if op == "fillet" { Blend::Fillet } else { Blend::Chamfer };
                let e: Vec<usize> = edges.iter().map(|i| *i as usize).collect();
                st.borrow_mut().arena.blend_var(sh(h), kind, a, b, &e).map(h32).map_err(|e| fail(&ctx, &op, e))
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "faceInfo",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32| -> rquickjs::Result<String> {
                let info = st.borrow_mut().arena.surface_info(sh(h)).map_err(|e| fail(&ctx, "faces", e))?;
                let list: Vec<serde_json::Value> = info
                    .iter()
                    .map(|f| serde_json::json!({ "kind": f.kind, "radius": f.radius, "axis": f.axis, "origin": f.origin, "bulge": f.bulge, "span": f.span }))
                    .collect();
                Ok(serde_json::Value::Array(list).to_string())
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "defeature",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, faces: Vec<u32>| -> rquickjs::Result<u32> {
                let f: Vec<usize> = faces.iter().map(|i| *i as usize).collect();
                st.borrow_mut().arena.defeature(sh(h), &f).map(h32).map_err(|e| fail(&ctx, "defeature", e))
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "paint",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, whole: bool, faces: Vec<u32>, colour: Vec<f64>, name: String| -> rquickjs::Result<u32> {
                let f: Vec<usize> = faces.iter().map(|i| *i as usize).collect();
                let c = (colour.len() == 3).then(|| [colour[0], colour[1], colour[2]]);
                st.borrow_mut().arena.paint(sh(h), if whole { None } else { Some(&f) }, c, &name).map(h32).map_err(|e| fail(&ctx, "paint", e))
            })?,
        )?;
    }
    {
        let st = state.clone();
        native.set(
            "checkGeometry",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32| -> rquickjs::Result<String> {
                st.borrow_mut().arena.check_geometry(sh(h)).map(|v| v.to_string()).map_err(|e| fail(&ctx, "checkGeometry", e))
            })?,
        )?;
    }
    Ok(())
}

/// The natives the mesh kernel adds: sweeps meshed here, the rest exact only.
pub(crate) fn install_mesh<'js>(ctx: &Ctx<'js>, native: &Object<'js>, arena: &Rc<RefCell<crate::runtime::Arena>>, tolerance: f64) -> rquickjs::Result<()> {
    {
        let arena = arena.clone();
        native.set(
            "sweep",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, h: u32, spec: String| -> rquickjs::Result<u32> {
                let m = {
                    let a = arena.borrow();
                    let p = a.profiles.get(h as usize).ok_or_else(|| throw(&ctx, "unknown profile"))?;
                    mesh_sweep(p, &spec, tolerance).map_err(|e| throw(&ctx, format!("sweep: {e}")))?
                };
                let mut a = arena.borrow_mut();
                a.solids.push(m);
                Ok((a.solids.len() - 1) as u32)
            })?,
        )?;
    }
    native.set("pathInfo", Function::new(ctx.clone(), move |ctx: Ctx<'js>, segs: Vec<f64>| -> rquickjs::Result<Vec<f64>> { path_info(&segs).map_err(|e| throw(&ctx, format!("path: {e}"))) })?)?;
    // Painting changes nothing the view draws.
    native.set("paint", Function::new(ctx.clone(), |h: u32, _whole: bool, _f: Vec<u32>, _c: Vec<f64>, _n: String| -> rquickjs::Result<u32> { Ok(h) })?)?;
    // No mesh form: the program is evaluated exactly and its true surfaces meshed.
    native.set("loft", Function::new(ctx.clone(), |ctx: Ctx<'js>, _s: String| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set("offsetSolid", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32, _d: f64, _s: bool| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set("thicken", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32, _t: f64, _f: Vec<u32>, _s: bool| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set(
        "blendVar",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, _op: String, _h: u32, _a: f64, _b: f64, _e: Vec<u32>| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?,
    )?;
    native.set("faceInfo", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32| -> rquickjs::Result<String> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set("defeature", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32, _f: Vec<u32>| -> rquickjs::Result<u32> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    native.set("checkGeometry", Function::new(ctx.clone(), |ctx: Ctx<'js>, _h: u32| -> rquickjs::Result<String> { Err(throw(&ctx, NEEDS_EXACT)) })?)?;
    Ok(())
}

