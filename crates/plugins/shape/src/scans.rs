//! Scans in Forma: point clouds and meshes from scanners, photogrammetry or
//! other programs, read once by content hash (as imported files are) and
//! worked on with `de_scan`: checked and repaired, thinned, smoothed,
//! remeshed, reconstructed into surfaces, fitted with the primitives and
//! B-spline patches an exact part is rebuilt from, cut into sections,
//! unwrapped, and compared against the CAD part they should match.
//!
//! The shape programs reach all of it through the prelude (`scanMesh`,
//! `fitScan`, `scanSection`, `reconstruct`, `fitSurface`, `.decimate()`,
//! …); the engine's `/scans` routes and `./forma mesh|scan|inspect` reach it
//! through the JSON operations here. Any producer of points or meshes plugs
//! in at [`read`]: a scanner's export, COLMAP's fused PLY, Apple Object
//! Capture's OBJ are all just a file of a kind listed in [`FORMATS`].

use crate::formats::{self, Kind};
use de_scan::fit::{self, Kind as FitKind};
use de_scan::points::{self, Cloud, Orient};
use de_scan::{Mesh, V3, analyze, bspline, decimate, holes, inspect, reconstruct, remesh, repair, section, segment, smooth, spatial, unwrap};
use manifold_csg::Manifold;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, OnceLock};

/// What a scan file is read as: points alone, or a mesh (with its vertices as points).
pub struct ScanData {
    pub mesh: Option<Mesh>,
    pub cloud: Cloud,
}

/// The formats a scan can come in, for messages.
pub const FORMATS: &str = "point clouds as XYZ/ASC/CSV/PTS text, PLY, PCD or E57; meshes as STL, OBJ, PLY, glTF/GLB or 3MF";

/// Extensions of point-cloud files kept in the store (besides the mesh formats).
pub const POINT_EXTS: [&str; 3] = ["xyz", "pcd", "e57"];

/// The extension a scan file is kept under: what its bytes say, else its name.
pub fn ext_of(bytes: &[u8], name: Option<&str>) -> Option<&'static str> {
    if bytes.starts_with(b"ASTM-E57") {
        return Some("e57");
    }
    if let Some(k) = Kind::sniff(bytes).filter(|k| k.becomes() == formats::Becomes::Mesh) {
        return Some(k.ext());
    }
    if let Some(f) = points::Format::sniff(bytes) {
        return Some(match f {
            points::Format::Ply => "ply",
            f => f.ext(),
        });
    }
    if let Some(k) = Kind::sniff(bytes).filter(|k| k.becomes() == formats::Becomes::Mesh) {
        return Some(k.ext());
    }
    let name = name?;
    if let Some(f) = points::Format::from_name(name) {
        return Some(f.ext());
    }
    Kind::from_name(name).filter(|k| k.becomes() == formats::Becomes::Mesh).map(Kind::ext)
}

/// Reads a scan file's bytes as points or a mesh.
pub fn read(bytes: &[u8], ext: &str) -> Result<ScanData, String> {
    match ext {
        "xyz" => Ok(ScanData { mesh: None, cloud: points::read(bytes, points::Format::Xyz)? }),
        "pcd" => Ok(ScanData { mesh: None, cloud: points::read(bytes, points::Format::Pcd)? }),
        "e57" => Ok(ScanData { mesh: None, cloud: points::read(bytes, points::Format::E57)? }),
        "ply" => {
            let (cloud, faces) = points::read_ply(bytes)?;
            if cloud.is_empty() {
                return Err("the file has no points".into());
            }
            if faces.is_empty() {
                return Ok(ScanData { mesh: None, cloud });
            }
            let mut m = Mesh::new(cloud.positions.clone(), faces);
            m.weld(m.weld_eps());
            Ok(with_mesh(m, cloud.colors))
        }
        other => {
            let kind = Kind::from_name(&format!(".{other}")).ok_or_else(|| format!("a .{other} file is not a scan ({FORMATS})"))?;
            let bodies = formats::read_mesh(bytes, kind)?;
            let mut m = Mesh::default();
            for b in &bodies {
                m.merge(&Mesh::from_flat(&b.positions, &b.triangles));
            }
            m.weld(m.weld_eps());
            Ok(with_mesh(m, Vec::new()))
        }
    }
}

fn with_mesh(m: Mesh, colors: Vec<[u8; 3]>) -> ScanData {
    let normals = m.vertex_normals();
    let colors = if colors.len() == m.positions.len() { colors } else { Vec::new() };
    ScanData { cloud: Cloud { positions: m.positions.clone(), normals, colors }, mesh: Some(m) }
}

/// A stored scan by its hash (the few most recent kept read).
pub fn load(sha: &str) -> Result<Arc<ScanData>, String> {
    type Entry = (String, Arc<ScanData>);
    static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, d)) = cache.lock().unwrap().iter().find(|(s, _)| s == sha) {
        return Ok(d.clone());
    }
    let path = crate::assets::path(sha)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let data = Arc::new(read(&bytes, &ext)?);
    let mut c = cache.lock().unwrap();
    if c.len() >= 6 {
        c.remove(0);
    }
    c.push((sha.to_string(), data.clone()));
    Ok(data)
}

/// The scan's mesh, or why it has none.
pub fn mesh_of(sha: &str) -> Result<Mesh, String> {
    load(sha)?.mesh.clone().ok_or_else(|| "this scan is points only: reconstruct a surface from it first (reconstruct)".into())
}

// ---- Meshes and solids --------------------------------------------------------------

pub fn from_manifold(m: &Manifold) -> Mesh {
    let (props, n, idx) = m.to_mesh_f64();
    let n = n.max(3);
    Mesh {
        positions: props.chunks_exact(n).map(|v| [v[0], v[1], v[2]]).collect(),
        triangles: idx.chunks_exact(3).map(|t| [t[0] as u32, t[1] as u32, t[2] as u32]).collect(),
    }
}

/// A closed mesh as a solid (turned outward if it was inside out); a mesh
/// that is not closed is refused with what is wrong with it.
pub fn to_manifold(m: &Mesh) -> Result<Manifold, String> {
    let mut m = m.clone();
    m.weld(m.weld_eps());
    let tris: Vec<u64> = m.triangles.iter().flatten().map(|v| *v as u64).collect();
    let pos = m.flat_positions();
    match Manifold::from_mesh_f64(&pos, 3, &tris) {
        Ok(s) if s.volume() >= 0.0 => Ok(s),
        Ok(_) => {
            m.flip();
            let tris: Vec<u64> = m.triangles.iter().flatten().map(|v| *v as u64).collect();
            Manifold::from_mesh_f64(&pos, 3, &tris).map_err(|e| e.to_string())
        }
        Err(e) => {
            let r = analyze::analyze(&m);
            Err(format!("not a closed solid ({e}): {}; repair it (repair: true)", problems(&r)))
        }
    }
}

/// A report's problems in a phrase.
pub fn problems(r: &analyze::Report) -> String {
    let mut out = Vec::new();
    let mut say = |n: usize, one: &str, many: &str| {
        if n > 0 {
            out.push(if n == 1 { format!("1 {one}") } else { format!("{n} {many}") });
        }
    };
    say(r.holes, "hole", "holes");
    say(r.non_manifold_edges, "non-manifold edge", "non-manifold edges");
    say(r.non_manifold_vertices, "non-manifold vertex", "non-manifold vertices");
    say(r.flipped, "flipped triangle", "flipped triangles");
    say(r.degenerate, "degenerate triangle", "degenerate triangles");
    say(r.duplicate, "duplicate triangle", "duplicate triangles");
    say(r.self_intersections, "self-intersecting pair", "self-intersecting pairs");
    if r.components > 1 {
        out.push(format!("{} separate pieces", r.components));
    }
    if out.is_empty() { "no defects".into() } else { out.join(", ") }
}

fn num(o: &Value, k: &str) -> Option<f64> {
    o.get(k).and_then(Value::as_f64)
}

fn v3(o: &Value, k: &str) -> Option<V3> {
    let a = o.get(k)?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

/// Applies one mesh operation; returns the new mesh and what it did.
///   repair     {fillHoles, fair, largestOnly, minComponent, selfIntersections, flip}
///   decimate   {triangles | ratio, maxError}
///   smooth     {method: "taubin"|"laplacian", iterations, lambda}
///   remesh     {edge, iterations}
///   fillHoles  {maxEdges, fair}
///   harmonize · flip · largest · removeComponents {fraction}
pub fn mesh_op(m: &Mesh, op: &str, o: &Value) -> Result<(Mesh, Value), String> {
    let before = (m.positions.len(), m.triangles.len());
    let (out, extra) = match op {
        "repair" => {
            let mut opts = repair::Options::default();
            if let Some(v) = o.get("fillHoles") {
                opts.fill_holes = if v.as_bool() == Some(false) { 0 } else { v.as_u64().map(|n| n as usize).unwrap_or(2000) };
            }
            if let Some(v) = o.get("fair").and_then(Value::as_bool) {
                opts.fair = v;
            }
            if let Some(v) = o.get("largestOnly").and_then(Value::as_bool) {
                opts.largest_only = v;
            }
            if let Some(v) = num(o, "minComponent") {
                opts.min_component = v;
            }
            if let Some(v) = o.get("selfIntersections").and_then(Value::as_bool) {
                opts.self_intersections = v;
            }
            if let Some(v) = o.get("flip").and_then(Value::as_bool) {
                opts.flip = v;
            }
            let (fixed, rep) = repair::repair(m, &opts);
            (fixed, serde_json::to_value(&rep).unwrap())
        }
        "decimate" => {
            let target = match (num(o, "triangles"), num(o, "ratio")) {
                (Some(t), _) => t as usize,
                (None, Some(r)) => ((m.triangles.len() as f64) * r.clamp(0.0, 1.0)) as usize,
                _ => m.triangles.len() / 2,
            };
            let d = decimate::decimate(m, decimate::Target { triangles: target.max(4), max_error: num(o, "maxError") });
            // How far the lighter mesh strays from the original.
            let bvh = spatial::TriBvh::new(m);
            let dev = inspect::deviation(&d.positions, &bvh, f64::INFINITY, 0.0);
            (d, json!({ "max_deviation": dev.stats.max.abs().max(dev.stats.min.abs()), "mean_deviation": dev.stats.mean_abs }))
        }
        "smooth" => {
            let mut s = m.clone();
            let method = match o.get("method").and_then(Value::as_str) {
                Some("laplacian") => smooth::Method::Laplacian,
                _ => smooth::Method::Taubin,
            };
            let mut opts = smooth::Options { method, ..Default::default() };
            if let Some(n) = num(o, "iterations") {
                opts.iterations = n.clamp(1.0, 500.0) as usize;
            }
            if let Some(l) = num(o, "lambda") {
                opts.lambda = l.clamp(0.0, 1.0);
                opts.mu = -(opts.lambda + 0.03);
            }
            smooth::smooth(&mut s, &opts);
            let dv = s.volume() - m.volume();
            (s, json!({ "volume_change": dv }))
        }
        "remesh" => {
            let edge = num(o, "edge").unwrap_or_else(|| m.mean_edge());
            if !(edge > 0.0) {
                return Err("remesh: the edge length must be positive".into());
            }
            // Guard against asking for millions of triangles.
            let estimate = m.area() / (edge * edge * 0.433);
            if estimate > 3.0e6 {
                return Err(format!("remesh: {edge} mm edges would make about {:.0} triangles; choose a longer edge", estimate));
            }
            let r = remesh::remesh(m, edge, num(o, "iterations").unwrap_or(5.0).clamp(1.0, 20.0) as usize);
            (r, json!({ "edge": edge }))
        }
        "fillHoles" => {
            let mut f = m.clone();
            let r = holes::fill_holes(&mut f, num(o, "maxEdges").unwrap_or(2000.0) as usize, o.get("fair").and_then(Value::as_bool).unwrap_or(true));
            (f, serde_json::to_value(&r).unwrap())
        }
        "harmonize" => {
            let mut f = m.clone();
            let n = repair::harmonize_normals(&mut f);
            (f, json!({ "turned": n }))
        }
        "flip" => {
            let mut f = m.clone();
            f.flip();
            (f, json!({}))
        }
        "largest" | "removeComponents" => {
            let mut f = m.clone();
            let n = repair::remove_components(&mut f, num(o, "fraction").unwrap_or(0.0), op == "largest");
            f.compact();
            (f, json!({ "removed": n }))
        }
        other => return Err(format!("unknown mesh operation {other} (repair, decimate, smooth, remesh, fillHoles, harmonize, flip, largest, removeComponents)")),
    };
    let mut report = json!({ "operation": op, "vertices": [before.0, out.positions.len()], "triangles": [before.1, out.triangles.len()] });
    if let (Some(r), Some(e)) = (report.as_object_mut(), extra.as_object()) {
        r.extend(e.clone());
    }
    Ok((out, report))
}

/// Applies one point-cloud operation.
///   downsample {voxel | count} · outliers {neighbours, sigma} · normals {neighbours, toward}
pub fn cloud_op(c: &Cloud, op: &str, o: &Value) -> Result<(Cloud, Value), String> {
    let before = c.len();
    let (out, extra) = match op {
        "downsample" => match (num(o, "voxel"), num(o, "count")) {
            (Some(v), _) if v > 0.0 => (points::voxel_downsample(c, v), json!({ "voxel": v })),
            (_, Some(n)) => (points::take_count(c, n as usize), json!({})),
            _ => {
                let v = c.spacing() * 2.0;
                (points::voxel_downsample(c, v.max(1e-9)), json!({ "voxel": v }))
            }
        },
        "outliers" => {
            let (k, s) = (num(o, "neighbours").unwrap_or(16.0) as usize, num(o, "sigma").unwrap_or(2.0));
            let (clean, dropped) = points::remove_outliers(c, k.max(2), s);
            (clean, json!({ "removed": dropped }))
        }
        "normals" => {
            let mut n = c.clone();
            let orient = match v3(o, "toward") {
                Some(p) => Orient::Toward(p),
                None => Orient::Outward,
            };
            points::estimate_normals(&mut n, num(o, "neighbours").unwrap_or(12.0) as usize, orient);
            (n, json!({}))
        }
        other => return Err(format!("unknown point operation {other} (downsample, outliers, normals)")),
    };
    let mut report = json!({ "operation": op, "points": [before, out.len()] });
    if let (Some(r), Some(e)) = (report.as_object_mut(), extra.as_object()) {
        r.extend(e.clone());
    }
    Ok((out, report))
}

/// Poisson reconstruction {depth, trim, smooth}.
pub fn reconstruct(c: &Cloud, o: &Value) -> Result<(Mesh, Value), String> {
    let mut opts = reconstruct::Options::default();
    if let Some(d) = num(o, "depth") {
        opts.depth = d.clamp(4.0, 9.0) as u32;
    }
    if let Some(t) = num(o, "trim") {
        opts.trim = t.max(0.0);
    }
    if let Some(s) = num(o, "smooth") {
        opts.smooth = s.max(0.0) as usize;
    }
    let m = reconstruct::poisson(c, &opts)?;
    // How closely the surface passes the points.
    let bvh = spatial::TriBvh::new(&m);
    let step = (c.len() / 20000).max(1);
    let sample: Vec<V3> = c.positions.iter().step_by(step).copied().collect();
    let dev = inspect::deviation(&sample, &bvh, f64::INFINITY, 0.0);
    let r = analyze::analyze(&m);
    Ok((m, json!({ "operation": "reconstruct", "depth": opts.depth, "points": c.len(), "triangles": r.triangles, "closed": r.solid, "holes": r.holes, "rms": dev.stats.rms, "max": dev.stats.max.abs().max(dev.stats.min.abs()) })))
}

/// Points of a scan narrowed to a region: {box: [x0, y0, z0, x1, y1, z1]} or {near: [x, y, z], within: r}.
pub fn region(c: &Cloud, o: &Value) -> Cloud {
    let keep: Vec<u32> = if let Some(b) = o.get("box").and_then(Value::as_array).filter(|a| a.len() == 6) {
        let b: Vec<f64> = b.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
        (0..c.len() as u32).filter(|&i| (0..3).all(|k| c.positions[i as usize][k] >= b[k].min(b[k + 3]) && c.positions[i as usize][k] <= b[k].max(b[k + 3]))).collect()
    } else if let (Some(p), Some(r)) = (v3(o, "near"), num(o, "within")) {
        (0..c.len() as u32).filter(|&i| de_scan::linalg::dist(c.positions[i as usize], p) <= r).collect()
    } else {
        return c.clone();
    };
    c.select(&keep)
}

/// A fit as flat JSON: the primitive's fields with the fit's quality.
pub fn fit_json(f: &fit::Fit) -> Value {
    let mut v = serde_json::to_value(&f.primitive).unwrap();
    if let Some(o) = v.as_object_mut() {
        o.insert("rms".into(), json!(f.rms));
        o.insert("max".into(), json!(f.max));
        o.insert("points".into(), json!(f.points));
        o.insert("of".into(), json!(f.of));
        if let fit::Primitive::Cone { half_angle, .. } = f.primitive {
            o.insert("half_angle_degrees".into(), json!(half_angle.to_degrees()));
        }
    }
    v
}

fn fit_kinds(o: &Value) -> Vec<FitKind> {
    match o.get("kinds").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(|k| k.as_str().and_then(FitKind::parse)).collect(),
        None => vec![FitKind::Plane, FitKind::Cylinder, FitKind::Sphere, FitKind::Cone],
    }
}

/// A sensible RANSAC distance for a cloud: a few point spacings.
fn threshold(c: &Cloud, o: &Value) -> f64 {
    num(o, "threshold").unwrap_or_else(|| (c.spacing() * 1.5).max(1e-6))
}

/// The data operations (JSON in, JSON out) on a stored scan:
///   info · check · fit {kind, region, threshold} · detect {kinds, threshold, minPoints, max}
///   section {origin, normal} | {normal, count} · curvature · segment {by, angle, curvature, threshold}
///   unwrap {region} · boundaries · fitCurve {points, poles, degree} · surface {poles, degree, smoothing, region}
pub fn query(op: &str, sha: &str, o: &Value) -> Result<Value, String> {
    let data = || load(sha);
    match op {
        "info" => {
            let data = data()?;
            let s = points::summary(&data.cloud);
            let mut v = json!({ "kind": if data.mesh.is_some() { "mesh" } else { "points" }, "points": s.points, "bounds": s.bounds, "spacing": s.spacing, "normals": s.normals, "colors": s.colors });
            if let Some(m) = &data.mesh {
                let r = analyze::analyze(m);
                v["triangles"] = json!(m.triangles.len());
                v["solid"] = json!(r.solid);
                v["problems"] = json!(problems(&r));
                v["volume"] = json!(r.volume);
                v["area"] = json!(r.area);
            }
            Ok(v)
        }
        "check" => Ok(serde_json::to_value(analyze::analyze(&mesh_of(sha)?)).unwrap()),
        "fit" => {
            let c = region(&data()?.cloud, o);
            let kind = o.get("kind").and_then(Value::as_str).unwrap_or("auto");
            if c.len() < 6 {
                return Err(format!("fit: only {} points in the region", c.len()));
            }
            if o.get("ransac").and_then(Value::as_bool) == Some(true) {
                let k = FitKind::parse(kind).ok_or_else(|| format!("fit: RANSAC needs a kind (plane, sphere, cylinder, cone, torus), not {kind}"))?;
                let normals = if c.normals.len() == c.len() { c.normals.clone() } else {
                    let mut n = c.clone();
                    points::estimate_normals(&mut n, 12, Orient::None);
                    n.normals
                };
                let all: Vec<u32> = (0..c.len() as u32).collect();
                let f = fit::ransac(&c, &normals, k, &fit::Ransac { threshold: threshold(&c, o), ..Default::default() }, &all).ok_or("fit: no such surface found among the points")?;
                return Ok(fit_json(&f));
            }
            if kind == "auto" {
                let fits = fit::fit_best(&c);
                let best = fits.first().ok_or("fit: nothing fits these points")?;
                let mut v = fit_json(best);
                v["alternatives"] = json!(fits.iter().skip(1).map(|f| json!({ "kind": f.primitive.kind(), "rms": f.rms })).collect::<Vec<_>>());
                return Ok(v);
            }
            let k = FitKind::parse(kind).ok_or_else(|| format!("fit: kind is auto, plane, sphere, cylinder, cone or torus, not {kind}"))?;
            fit::fit(&c, k).map(|f| fit_json(&f)).ok_or_else(|| format!("fit: no {kind} fits these points"))
        }
        "detect" => {
            let c = region(&data()?.cloud, o);
            let min = num(o, "minPoints").map(|v| v as usize).unwrap_or((c.len() / 50).max(30));
            let found = fit::detect(&c, &fit_kinds(o), &fit::Ransac { threshold: threshold(&c, o), ..Default::default() }, min, num(o, "max").unwrap_or(20.0) as usize);
            let left = c.len().saturating_sub(found.iter().map(|f| f.points).sum::<usize>());
            Ok(json!({ "shapes": found.iter().map(fit_json).collect::<Vec<_>>(), "unassigned": left, "points": c.len() }))
        }
        "section" => {
            let m = mesh_of(sha)?;
            let normal = v3(o, "normal").unwrap_or([0.0, 0.0, 1.0]);
            let sections = match (v3(o, "origin"), num(o, "count")) {
                (Some(origin), _) => vec![section::section(&m, origin, normal)],
                (None, Some(n)) => section::sections(&m, normal, n.clamp(1.0, 200.0) as usize),
                (None, None) => {
                    let (lo, hi) = m.bounds();
                    vec![section::section(&m, de_scan::linalg::mid(lo, hi), normal)]
                }
            };
            Ok(serde_json::to_value(sections).unwrap())
        }
        "curvature" => {
            let m = mesh_of(sha)?;
            let c = de_scan::curvature::curvature(&m);
            let stat = |v: &[f64]| {
                let mut s: Vec<f64> = v.to_vec();
                s.sort_by(|a, b| a.total_cmp(b));
                let q = |f: f64| s[((s.len() - 1) as f64 * f) as usize];
                json!({ "min": s[0], "p5": q(0.05), "median": q(0.5), "p95": q(0.95), "max": s[s.len() - 1] })
            };
            if c.mean.is_empty() {
                return Err("curvature: the mesh has no vertices".into());
            }
            let mut v = json!({ "mean": stat(&c.mean), "gaussian": stat(&c.gaussian), "k1": stat(&c.k1), "k2": stat(&c.k2) });
            if o.get("values").and_then(Value::as_bool) == Some(true) {
                v["values"] = json!({ "mean": c.mean, "gaussian": c.gaussian });
            }
            Ok(v)
        }
        "segment" => {
            let m = mesh_of(sha)?;
            let s = match o.get("by").and_then(Value::as_str).unwrap_or("curvature") {
                "planes" => {
                    let d = data()?;
                    segment::by_planes(&m, threshold(&d.cloud, o), num(o, "minPoints").unwrap_or(30.0) as usize, num(o, "max").unwrap_or(30.0) as usize)
                }
                _ => segment::by_curvature(&m, num(o, "angle").unwrap_or(25.0), num(o, "curvature").unwrap_or(0.0), num(o, "minTriangles").unwrap_or(10.0) as usize),
            };
            let segs: Vec<Value> = s.segments.iter().map(|g| json!({ "id": g.id, "triangles": g.triangles, "area": g.area, "normal": g.normal, "centroid": g.centroid, "fit": g.fit.as_ref().map(fit_json) })).collect();
            let mut v = json!({ "segments": segs });
            if o.get("labels").and_then(Value::as_bool) == Some(true) {
                v["labels"] = json!(s.labels);
            }
            Ok(v)
        }
        "unwrap" => {
            let mut m = mesh_of(sha)?;
            if o.get("box").is_some() || o.get("near").is_some() {
                // Only the triangles whose corners are all in the region.
                let c = Cloud { positions: m.positions.clone(), normals: vec![], colors: vec![] };
                let inside = region(&c, o);
                let set: std::collections::HashSet<[u64; 3]> = inside.positions.iter().map(|p| p.map(f64::to_bits)).collect();
                m.triangles.retain(|t| t.iter().all(|&v| set.contains(&m.positions[v as usize].map(f64::to_bits))));
                m.compact();
            }
            if let Some(seg) = num(o, "segment") {
                let s = segment::by_curvature(&m, num(o, "angle").unwrap_or(25.0), 0.0, 10);
                let id = seg as u32;
                let keep: Vec<bool> = s.labels.iter().map(|l| *l == id).collect();
                let mut i = 0;
                m.triangles.retain(|_| {
                    i += 1;
                    keep[i - 1]
                });
                m.compact();
            }
            let f = unwrap::unwrap(&m)?;
            let mut v = serde_json::to_value(&f).unwrap();
            v["triangles"] = json!(f.triangles);
            Ok(v)
        }
        "boundaries" => {
            let m = mesh_of(sha)?;
            Ok(json!({ "loops": segment::boundaries(&m), "sharp": if o.get("sharp").is_some() { json!(segment::sharp_edges(&m, num(o, "sharp").unwrap_or(30.0))) } else { Value::Null } }))
        }
        "fitCurve" => {
            let pts: Vec<V3> = match o.get("points").and_then(Value::as_array) {
                Some(list) => list.iter().filter_map(|p| p.as_array().and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?]))).collect(),
                None => {
                    // Points of the region, ordered along their main direction.
                    let c = region(&data()?.cloud, o);
                    let (cen, cov) = de_scan::linalg::covariance(&c.positions);
                    let (_, ev) = de_scan::linalg::sym_eigen3(cov);
                    let mut p = c.positions.clone();
                    p.sort_by(|a, b| de_scan::linalg::dot(de_scan::linalg::sub(*a, cen), ev[2]).total_cmp(&de_scan::linalg::dot(de_scan::linalg::sub(*b, cen), ev[2])));
                    p
                }
            };
            let c = bspline::fit_curve(&pts, num(o, "poles").unwrap_or(8.0) as usize, num(o, "degree").unwrap_or(3.0) as usize)?;
            Ok(serde_json::to_value(c).unwrap())
        }
        "surface" => {
            let s = fit_surface(&data()?.cloud, o)?;
            Ok(serde_json::to_value(s).unwrap())
        }
        other => Err(format!("unknown scan query {other}")),
    }
}

/// A B-spline surface through a scan's points {poles: n | [nu, nv], degree, smoothing, region}.
pub fn fit_surface(c: &Cloud, o: &Value) -> Result<bspline::Surface, String> {
    let c = region(c, o);
    let (nu, nv) = match o.get("poles") {
        Some(Value::Array(a)) if a.len() == 2 => (a[0].as_f64().unwrap_or(8.0) as usize, a[1].as_f64().unwrap_or(8.0) as usize),
        Some(v) => {
            let n = v.as_f64().unwrap_or(8.0) as usize;
            (n, n)
        }
        None => (8, 8),
    };
    bspline::fit_surface(&c.positions, nu.clamp(2, 60), nv.clamp(2, 60), num(o, "degree").unwrap_or(3.0) as usize, num(o, "smoothing").unwrap_or(1e-4))
}

/// A fitted surface as a closed slab `thickness` thick (grown along the
/// surface's normal), meshed for the mesh kernel.
pub fn surface_slab(s: &bspline::Surface, thickness: f64, n: usize) -> Result<Manifold, String> {
    use de_scan::linalg::*;
    let (pts, tris) = s.tessellate(n);
    let grid = Mesh::new(pts, tris);
    let normals = grid.vertex_normals();
    let w = n + 1;
    let mut m = Mesh::default();
    m.positions.extend(grid.positions.iter().copied());
    m.positions.extend(grid.positions.iter().zip(&normals).map(|(p, nr)| add(*p, scale(*nr, thickness))));
    let off = (w * w) as u32;
    for t in &grid.triangles {
        m.triangles.push([t[0], t[2], t[1]]);
        m.triangles.push([t[0] + off, t[1] + off, t[2] + off]);
    }
    // The four sides.
    let id = |i: usize, j: usize| (i * w + j) as u32;
    let mut rim: Vec<u32> = Vec::new();
    for j in 0..n {
        rim.push(id(0, j));
    }
    for i in 0..n {
        rim.push(id(i, n));
    }
    for j in (1..=n).rev() {
        rim.push(id(n, j));
    }
    for i in (1..=n).rev() {
        rim.push(id(i, 0));
    }
    for k in 0..rim.len() {
        let (a, b) = (rim[k], rim[(k + 1) % rim.len()]);
        m.triangles.push([a, b + off, b]);
        m.triangles.push([a, a + off, b + off]);
    }
    to_manifold(&m)
}

/// A mesh file's bodies with mesh operations applied first (repair,
/// decimate, smooth, remesh), as solids named as in the file.
pub fn import_mesh_with(sha: &str, o: &Value) -> Result<Vec<crate::assets::Body>, String> {
    let kind = crate::assets::kind(sha)?;
    let bytes = std::fs::read(crate::assets::path(sha)?).map_err(|e| e.to_string())?;
    let bodies = formats::read_mesh(&bytes, kind)?;
    let mut out = Vec::new();
    for (i, b) in bodies.iter().enumerate() {
        let name = if b.name.trim().is_empty() { format!("Body {}", i + 1) } else { b.name.trim().to_string() };
        let mut m = Mesh::from_flat(&b.positions, &b.triangles);
        m.weld(m.weld_eps());
        m = apply_ops(m, o).map_err(|e| format!("{name}: {e}"))?;
        let solid = to_manifold(&m).map_err(|e| format!("{name}: {e}"))?;
        out.push((name, solid, b.look.as_ref().map(|l| l.color)));
    }
    Ok(out)
}

/// The options' mesh operations in order: repair, remesh, decimate, smooth.
pub fn apply_ops(mut m: Mesh, o: &Value) -> Result<Mesh, String> {
    let opt = |k: &str| o.get(k).filter(|v| !v.is_null() && v.as_bool() != Some(false));
    if let Some(r) = opt("repair") {
        m = mesh_op(&m, "repair", if r.is_object() { r } else { &Value::Null })?.0;
    }
    if let Some(r) = opt("remesh") {
        m = mesh_op(&m, "remesh", &if r.is_number() { json!({ "edge": r }) } else { r.clone() })?.0;
    }
    if let Some(d) = opt("decimate") {
        let spec = match d.as_f64() {
            Some(x) if x < 1.0 => json!({ "ratio": x }),
            Some(x) => json!({ "triangles": x }),
            None => d.clone(),
        };
        m = mesh_op(&m, "decimate", &spec)?.0;
    }
    if let Some(s) = opt("smooth") {
        m = mesh_op(&m, "smooth", &if s.is_number() { json!({ "iterations": s }) } else if s.is_object() { s.clone() } else { json!({}) })?.0;
    }
    Ok(m)
}

/// The JSON mesh options a solid method passes, applied to a solid's mesh.
pub fn solid_op(solid: &Manifold, op: &str, o: &Value) -> Result<Manifold, String> {
    let m = from_manifold(solid);
    let (out, _) = mesh_op(&m, op, o)?;
    to_manifold(&out)
}

/// A whole scan's mesh as one solid (with options applied), or a surface
/// reconstructed from its points when it has no mesh.
pub fn scan_solid(op: &str, sha: &str, o: &Value) -> Result<Manifold, String> {
    match op {
        "mesh" => {
            let m = apply_ops(mesh_of(sha)?, o)?;
            to_manifold(&m)
        }
        "reconstruct" => {
            let data = load(sha)?;
            let c = region(&data.cloud, o);
            let mut opts = o.clone();
            // A part must be closed: never trimmed here.
            opts["trim"] = json!(0);
            let (m, _) = reconstruct(&c, &opts)?;
            to_manifold(&apply_ops(m, o)?)
        }
        "surface" => {
            let data = load(sha)?;
            let s = fit_surface(&data.cloud, o)?;
            surface_slab(&s, num(o, "thickness").unwrap_or(1.0), 48)
        }
        other => Err(format!("unknown scan solid {other}")),
    }
}

/// The prelude's data calls on scans, the same in both kernels:
/// `scanQuery(op, sha, optionsJson)` → JSON.
pub(crate) fn install_queries<'js>(ctx: &rquickjs::Ctx<'js>, native: &rquickjs::Object<'js>) -> rquickjs::Result<()> {
    native.set(
        "scanQuery",
        rquickjs::Function::new(ctx.clone(), |ctx: rquickjs::Ctx<'js>, op: String, sha: String, options: String| -> rquickjs::Result<String> {
            let o: Value = serde_json::from_str(&options).unwrap_or(Value::Null);
            query(&op, &sha, &o).map(|v| v.to_string()).map_err(|e| crate::runtime::throw(&ctx, format!("scan {op}: {e}")))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_files_are_told_apart_and_read() {
        let xyz = b"# scan\n1 2 3\n4 5 6\n7 8 9.5\n";
        assert_eq!(ext_of(xyz, None), Some("xyz"));
        assert_eq!(read(xyz, "xyz").unwrap().cloud.len(), 3);
        let cube = de_scan::mesh::cube(10.0, 10.0, 10.0);
        let body = formats::MeshBody { name: "Cube".into(), positions: cube.flat_positions(), triangles: cube.flat_triangles(), look: None, normals: vec![] };
        let obj = formats::mesh::write_obj(&[body]);
        assert_eq!(ext_of(&obj, Some("scan.obj")), Some("obj"));
        let d = read(&obj, "obj").unwrap();
        assert_eq!(d.mesh.as_ref().unwrap().triangles.len(), 12);
        assert!((to_manifold(d.mesh.as_ref().unwrap()).unwrap().volume() - 1000.0).abs() < 1e-9);
        assert_eq!(ext_of(b"ASTM-E57\0\0", None), Some("e57"));
    }

    #[test]
    fn a_fitted_patch_thickens_into_a_closed_slab() {
        let pts: Vec<V3> = (0..2000).map(|i| {
            let x = (i as f64 * 0.618034).fract() * 40.0;
            let y = (i as f64 * 0.7548776662).fract() * 30.0;
            [x, y, 0.002 * (x - 20.0) * (x - 20.0)]
        }).collect();
        let c = Cloud { positions: pts, normals: vec![], colors: vec![] };
        let s = fit_surface(&c, &json!({ "poles": 6 })).unwrap();
        assert!(s.rms < 0.01, "{}", s.rms);
        let slab = surface_slab(&s, 2.0, 24).unwrap();
        // About 40 × 30 × 2 (the curve adds a little length).
        assert!((slab.volume() - 2400.0).abs() < 60.0, "{}", slab.volume());
    }

    fn store(bytes: &[u8], ext: &str) -> String {
        if crate::assets::dir().is_none() {
            crate::assets::set_dir(std::env::temp_dir().join(format!("forma-scan-assets-{}", std::process::id())));
        }
        crate::assets::store_scan(bytes, ext).unwrap()
    }

    #[test]
    fn programs_repair_fit_reconstruct_and_refine_scans() {
        use de_scan::mesh::{cube, uv_sphere};
        // A sphere scan with a hole: refused as it is, a solid once repaired.
        let mut s = uv_sphere(10.0, 32, 64);
        s.triangles.drain(300..330);
        let body = formats::MeshBody { name: "Scan".into(), positions: s.flat_positions(), triangles: s.flat_triangles(), look: None, normals: vec![] };
        let obj = store(&formats::mesh::write_obj(&[body]), "obj");
        let bad = crate::runtime::evaluate(&format!("return importMesh(\"{obj}\");"), &[], 0.01);
        assert!(bad.is_err());
        let e = crate::runtime::evaluate(&format!("return importMesh(\"{obj}\", {{ repair: true, smooth: 2 }});"), &[], 0.01).unwrap();
        let want = 4.0 / 3.0 * std::f64::consts::PI * 1000.0;
        assert!((e.parts[0].volume - want).abs() / want < 0.02, "{}", e.parts[0].volume);

        // A cylinder of points: its numbers rebuild an exact cylinder.
        let mut pts = String::new();
        for i in 0..3000 {
            let a = i as f64 * 2.399963;
            let z = (i as f64 * 0.7548776662).fract() * 25.0;
            pts.push_str(&format!("{} {} {}\n", 6.0 * a.cos() + 3.0, 6.0 * a.sin(), z));
        }
        let xyz = store(pts.as_bytes(), "xyz");
        let src = format!("const c = fitScan(\"{xyz}\", \"cylinder\");\nmetric(\"r\", c.radius);\nreturn cylinder(c.length, c.radius).translate(c.base[0], c.base[1], c.base[2]);");
        let x = crate::exact::evaluate_exact("Fitted", &src, &[], 0.01).unwrap();
        let v = x.arena.volume(x.parts[0].shape);
        assert!((v - std::f64::consts::PI * 36.0 * 25.0).abs() < 2.0, "{v}");
        assert!(x.faceted.is_empty(), "built exactly from the fitted numbers: {:?}", x.faceted);
        assert!((x.metrics["r"].value - 6.0).abs() < 1e-3);

        // A sphere of points reconstructs closed; a mesh decimates.
        let cloud = points::sample_mesh(&uv_sphere(15.0, 30, 60), 5000);
        let ply = store(&points::write_ply(&Cloud { positions: cloud.positions, normals: vec![], colors: vec![] }), "ply");
        let e = crate::runtime::evaluate(&format!("return reconstruct(\"{ply}\", {{ depth: 6 }}).decimate(0.5);"), &[], 0.01).unwrap();
        let want = 4.0 / 3.0 * std::f64::consts::PI * 15f64.powi(3);
        assert!((e.volume - want).abs() / want < 0.05, "{}", e.volume);
        let info: Value = serde_json::from_str(&query("info", &ply, &Value::Null).unwrap().to_string()).unwrap();
        assert_eq!(info["kind"], "points");

        // A meshed box refined to six flat faces in the exact kernel.
        let b = cube(10.0, 20.0, 5.0);
        let bb = formats::MeshBody { name: "Block".into(), positions: b.flat_positions(), triangles: b.flat_triangles(), look: None, normals: vec![] };
        let stl = store(&formats::mesh::write_obj(&[bb]), "obj");
        let mut x = crate::exact::evaluate_exact("Block", &format!("return importMesh(\"{stl}\", {{ refine: true }});"), &[], 0.01).unwrap();
        let faces = x.arena.faces(x.parts[0].shape).unwrap().len();
        assert_eq!(faces, 6);

        // A fitted free-form patch is an exact B-spline, thickened.
        let mut pts = String::new();
        for i in 0..2500 {
            let (u, w) = ((i as f64 * 0.618034).fract() * 40.0, (i as f64 * 0.7548776662).fract() * 30.0);
            pts.push_str(&format!("{u} {w} {}\n", 3.0 * (u / 40.0 * std::f64::consts::PI).sin()));
        }
        let skin = store(pts.as_bytes(), "xyz");
        let x = crate::exact::evaluate_exact("Skin", &format!("return fitSurface(\"{skin}\", {{ poles: 7, thickness: 1.5 }});"), &[], 0.01).unwrap();
        let v = x.arena.volume(x.parts[0].shape);
        assert!(v > 1700.0 && v < 2000.0, "{v}");
        assert!(x.faceted.is_empty(), "{:?}", x.faceted);
    }

    #[test]
    fn mesh_operations_report_what_they_did() {
        let m = de_scan::mesh::uv_sphere(10.0, 40, 80);
        let (d, r) = mesh_op(&m, "decimate", &json!({ "ratio": 0.25 })).unwrap();
        assert!(d.triangles.len() <= m.triangles.len() / 4 + 2);
        assert!(r["max_deviation"].as_f64().unwrap() < 0.2, "{r}");
        let mut open = m.clone();
        open.triangles.truncate(open.triangles.len() - 40);
        assert!(to_manifold(&open).unwrap_err().contains("hole"));
        let (fixed, r) = mesh_op(&open, "repair", &json!({})).unwrap();
        assert_eq!(r["after"]["solid"], json!(true), "{r}");
        assert!(to_manifold(&fixed).is_ok());
    }
}
