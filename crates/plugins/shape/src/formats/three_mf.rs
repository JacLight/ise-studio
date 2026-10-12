//! Reading 3MF (what slicers save): every build item's objects, placed, in
//! millimetres, named as in the file, coloured by their base material.
//! Components (assemblies, and Bambu/Prusa projects that keep each object
//! in its own model file) are followed.

use super::xml::{Event, attr, events};
use super::{Look, MeshBody};
use std::collections::HashMap;

/// A 3MF transform ("m00 m01 m02 m10 … m32": rows of a 4 × 3 matrix) applied to a point.
type Affine = [f64; 12];
const IDENTITY: Affine = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

fn parse_transform(s: Option<&str>) -> Affine {
    let v: Vec<f64> = s.unwrap_or("").split_whitespace().filter_map(|t| t.parse().ok()).collect();
    if v.len() == 12 { v.try_into().unwrap() } else { IDENTITY }
}

fn apply(m: &Affine, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|c| p[0] * m[c] + p[1] * m[3 + c] + p[2] * m[6 + c] + m[9 + c])
}

/// `a` then `b`.
fn then(a: &Affine, b: &Affine) -> Affine {
    let mut m = [0.0; 12];
    for r in 0..4 {
        let p = [a[r * 3], a[r * 3 + 1], a[r * 3 + 2]];
        let q = if r == 3 { apply(b, p) } else { [0, 1, 2].map(|c| p[0] * b[c] + p[1] * b[3 + c] + p[2] * b[6 + c]) };
        m[r * 3..r * 3 + 3].copy_from_slice(&q);
    }
    m
}

fn det(m: &Affine) -> f64 {
    m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6])
}

/// "#RRGGBB" or "#RRGGBBAA" to sRGB 0…1.
fn hex_color(s: &str) -> Option<[f64; 3]> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() < 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|b| b as f64 / 255.0);
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[derive(Default)]
struct Object {
    name: String,
    positions: Vec<f64>,
    triangles: Vec<u32>,
    color: Option<[f64; 3]>,
    /// (model file, object id, transform).
    components: Vec<(String, String, Affine)>,
}

/// One model file's objects (by id) and build items (object id, transform).
struct Model {
    objects: HashMap<String, Object>,
    build: Vec<(String, Affine)>,
    scale: f64,
}

fn read_model(path: &str, text: &str) -> Result<Model, String> {
    let mut model = Model { objects: HashMap::new(), build: Vec::new(), scale: 1.0 };
    let mut colors: HashMap<String, Vec<[f64; 3]>> = HashMap::new(); // basematerials / colorgroup id → colours
    let mut group: Option<(String, Vec<[f64; 3]>)> = None;
    let mut current: Option<(String, Object)> = None;
    let mut object_color: Option<(String, usize)> = None;
    for e in events(text)? {
        match e {
            Event::Start { name, attrs } => match name {
                "model" => {
                    model.scale = match attr(&attrs, "unit").unwrap_or("millimeter") {
                        "micron" => 0.001,
                        "centimeter" => 10.0,
                        "inch" => 25.4,
                        "foot" => 304.8,
                        "meter" => 1000.0,
                        _ => 1.0,
                    }
                }
                "basematerials" | "colorgroup" => group = Some((attr(&attrs, "id").unwrap_or("").to_string(), Vec::new())),
                "base" | "color" => {
                    if let Some((_, list)) = &mut group {
                        list.push(hex_color(attr(&attrs, "displaycolor").or(attr(&attrs, "color")).unwrap_or("")).unwrap_or([0.8; 3]));
                    }
                }
                "object" => {
                    let id = attr(&attrs, "id").unwrap_or("").to_string();
                    let name = attr(&attrs, "name").unwrap_or("").to_string();
                    object_color = attr(&attrs, "pid").map(|p| (p.to_string(), attr(&attrs, "pindex").and_then(|i| i.parse().ok()).unwrap_or(0)));
                    current = Some((id, Object { name, ..Default::default() }));
                }
                "vertex" => {
                    if let Some((_, o)) = &mut current {
                        let c = |k| attr(&attrs, k).and_then(|v| v.parse::<f64>().ok()).ok_or_else(|| format!("{path}: a vertex without {k}"));
                        o.positions.extend([c("x")?, c("y")?, c("z")?]);
                    }
                }
                "triangle" => {
                    if let Some((_, o)) = &mut current {
                        let v = |k| attr(&attrs, k).and_then(|v| v.parse::<u32>().ok()).ok_or_else(|| format!("{path}: a triangle without {k}"));
                        o.triangles.extend([v("v1")?, v("v2")?, v("v3")?]);
                        // The first triangle's own colour stands for an object that has none.
                        if o.color.is_none()
                            && object_color.is_none()
                            && let Some(pid) = attr(&attrs, "pid")
                        {
                            object_color = Some((pid.to_string(), attr(&attrs, "p1").and_then(|i| i.parse().ok()).unwrap_or(0)));
                        }
                    }
                }
                "component" => {
                    if let Some((_, o)) = &mut current {
                        let file = attr(&attrs, "path").map(|p| p.trim_start_matches('/').to_string()).unwrap_or_else(|| path.to_string());
                        o.components.push((file, attr(&attrs, "objectid").unwrap_or("").to_string(), parse_transform(attr(&attrs, "transform"))));
                    }
                }
                "item" => model.build.push((attr(&attrs, "objectid").unwrap_or("").to_string(), parse_transform(attr(&attrs, "transform")))),
                _ => {}
            },
            Event::End { name } => match name {
                "basematerials" | "colorgroup" => {
                    if let Some((id, list)) = group.take() {
                        colors.insert(id, list);
                    }
                }
                "object" => {
                    if let Some((id, mut o)) = current.take() {
                        o.color = object_color.take().and_then(|(pid, i)| colors.get(&pid).and_then(|c| c.get(i)).copied());
                        model.objects.insert(id, o);
                    }
                }
                _ => {}
            },
        }
    }
    Ok(model)
}

pub fn read(data: &[u8]) -> Result<Vec<MeshBody>, String> {
    let files = super::zip::read(data)?;
    let text_of = |p: &str| files.iter().find(|(n, _)| n.trim_start_matches('/').eq_ignore_ascii_case(p)).map(|(_, b)| String::from_utf8_lossy(b).into_owned());
    // The root model: as the package's relationships say, else the usual place.
    let rels = text_of("_rels/.rels").unwrap_or_default();
    let root = events(&rels)
        .ok()
        .and_then(|ev| {
            ev.into_iter().find_map(|e| match e {
                Event::Start { name: "Relationship", attrs } if attr(&attrs, "Type").is_some_and(|t| t.ends_with("/3dmodel")) => attr(&attrs, "Target").map(|t| t.trim_start_matches('/').to_string()),
                _ => None,
            })
        })
        .unwrap_or_else(|| "3D/3dmodel.model".to_string());
    let mut models: HashMap<String, Model> = HashMap::new();
    let load = |p: &str, models: &mut HashMap<String, Model>| -> Result<(), String> {
        if !models.contains_key(p) {
            let text = text_of(p).ok_or_else(|| format!("the 3MF file has no {p}"))?;
            models.insert(p.to_string(), read_model(p, &text)?);
        }
        Ok(())
    };
    load(&root, &mut models)?;
    let scale = models[&root].scale;
    let mut out: Vec<MeshBody> = Vec::new();
    let build = models[&root].build.clone();
    for (n, (id, at)) in build.iter().enumerate() {
        // Walk the object and its components, placing each mesh.
        // (model file, object id, placement, colour from the object holding it, depth).
        let mut stack = vec![(root.clone(), id.clone(), *at, None, 0)];
        let item_name = models[&root].objects.get(id).map(|o| o.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Object {}", n + 1));
        let mut meshes: Vec<MeshBody> = Vec::new();
        while let Some((file, id, m, held, depth)) = stack.pop() {
            if depth > 32 {
                return Err("the 3MF components nest too deep (a cycle?)".into());
            }
            load(&file, &mut models)?;
            let o = models[&file].objects.get(&id).ok_or_else(|| format!("the 3MF file refers to object {id}, which it does not have"))?;
            let color = o.color.or(held);
            for (f, cid, cm) in &o.components {
                stack.push((f.clone(), cid.clone(), then(cm, &m), color, depth + 1));
            }
            if o.triangles.is_empty() {
                continue;
            }
            let n = (o.positions.len() / 3) as u32;
            if o.triangles.iter().any(|&t| t >= n) {
                return Err(format!("{}: a triangle uses a vertex that is not there", o.name));
            }
            let mut positions = Vec::with_capacity(o.positions.len());
            for v in o.positions.chunks_exact(3) {
                positions.extend(apply(&m, [v[0], v[1], v[2]]).map(|x| x * scale));
            }
            let mut triangles = o.triangles.clone();
            if det(&m) < 0.0 {
                for t in triangles.chunks_exact_mut(3) {
                    t.swap(1, 2);
                }
            }
            let name = if o.name.is_empty() { item_name.clone() } else { o.name.clone() };
            meshes.push(MeshBody { name, positions, triangles, look: color.map(Look::color), normals: Vec::new() });
        }
        // A part split into components in the file stays one part when they share a name.
        if meshes.len() > 1 && meshes.iter().all(|b| b.name == meshes[0].name) {
            let mut one = MeshBody { name: item_name.clone(), positions: Vec::new(), triangles: Vec::new(), look: meshes[0].look.clone(), normals: Vec::new() };
            for b in meshes {
                let base = (one.positions.len() / 3) as u32;
                one.positions.extend(b.positions);
                one.triangles.extend(b.triangles.into_iter().map(|t| t + base));
            }
            out.push(one);
        } else {
            out.extend(meshes);
        }
    }
    Ok(out)
}
