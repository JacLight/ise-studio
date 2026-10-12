//! glTF 2.0: `.glb` (binary) and `.gltf` (JSON with embedded buffers) in,
//! `.glb` out. glTF is metres and Y up; Forma is millimetres and Z up, so
//! both directions convert.

use super::{Look, MeshBody};
use serde_json::{Value, json};

const MM_PER_M: f64 = 1000.0;

/// sRGB (how colours are named) to linear (what glTF stores), and back.
fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Column-major 4 × 4 matrices, as glTF writes them.
type M4 = [f64; 16];
const IDENTITY: M4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

fn mul(a: &M4, b: &M4) -> M4 {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    m
}

fn node_matrix(node: &Value) -> M4 {
    if let Some(m) = node["matrix"].as_array().filter(|m| m.len() == 16) {
        let mut out = [0.0; 16];
        for (o, v) in out.iter_mut().zip(m) {
            *o = v.as_f64().unwrap_or(0.0);
        }
        return out;
    }
    let get = |k: &str, d: &[f64]| -> Vec<f64> { node[k].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect()).unwrap_or_else(|| d.to_vec()) };
    let t = get("translation", &[0.0, 0.0, 0.0]);
    let q = get("rotation", &[0.0, 0.0, 0.0, 1.0]);
    let s = get("scale", &[1.0, 1.0, 1.0]);
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let r = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
    ];
    let mut m = IDENTITY;
    for c in 0..3 {
        for row in 0..3 {
            m[c * 4 + row] = r[c * 3 + row] * s[c];
        }
    }
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

fn base64(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err("an embedded glTF buffer is not valid base64".into()),
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// A glTF scene's meshes as bodies: one per node and material, placed by the
/// node hierarchy, named after the node (or its mesh), coloured by its material.
pub fn read(data: &[u8]) -> Result<Vec<MeshBody>, String> {
    let (doc, bin): (Value, Option<&[u8]>) = if data.starts_with(b"glTF") {
        let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or("the glTF file is cut short");
        let mut at = 12;
        let (mut json, mut bin) = (None, None);
        while at + 8 <= data.len() {
            let (len, kind) = (u32_at(at)?, u32_at(at + 4)?);
            let chunk = data.get(at + 8..at + 8 + len).ok_or("the glTF file is cut short")?;
            match kind {
                0x4E4F_534A => json = Some(chunk),
                0x004E_4942 => bin = Some(chunk),
                _ => {}
            }
            at += 8 + len;
        }
        (serde_json::from_slice(json.ok_or("the glTF file has no JSON chunk")?).map_err(|e| format!("the glTF JSON does not read: {e}"))?, bin)
    } else {
        (serde_json::from_slice(data).map_err(|e| format!("not a glTF file: {e}"))?, None)
    };
    for ext in doc["extensionsRequired"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if ext.contains("draco") || ext.contains("meshopt") {
            return Err(format!("the file's meshes are compressed ({ext}); export it again without mesh compression"));
        }
    }
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for (i, b) in doc["buffers"].as_array().into_iter().flatten().enumerate() {
        buffers.push(match b["uri"].as_str() {
            None if i == 0 => bin.ok_or("the glTF file's buffer is missing")?.to_vec(),
            Some(uri) if uri.starts_with("data:") => base64(uri.split_once(',').map(|(_, d)| d).unwrap_or(""))?,
            Some(uri) => return Err(format!("the glTF file keeps its data in another file ({uri}): import the .glb, or a .gltf with embedded data")),
            None => return Err("a glTF buffer is missing".into()),
        });
    }
    let accessor = |i: &Value, want_vec3: bool| -> Result<Vec<f64>, String> {
        let a = &doc["accessors"][i.as_u64().ok_or("a glTF accessor index is missing")? as usize];
        let count = a["count"].as_u64().ok_or("a glTF accessor has no count")? as usize;
        let comps = match a["type"].as_str() {
            Some("SCALAR") => 1,
            Some("VEC3") => 3,
            t => return Err(format!("unexpected glTF accessor type {t:?}")),
        };
        if want_vec3 != (comps == 3) {
            return Err("a glTF accessor has the wrong shape".into());
        }
        let kind = a["componentType"].as_u64().unwrap_or(0);
        let size = match kind {
            5121 => 1,
            5123 => 2,
            5125 | 5126 => 4,
            k => return Err(format!("glTF component type {k} is not supported here")),
        };
        let Some(view) = a["bufferView"].as_u64() else { return Ok(vec![0.0; count * comps]) };
        let v = &doc["bufferViews"][view as usize];
        let buffer = buffers.get(v["buffer"].as_u64().unwrap_or(0) as usize).ok_or("a glTF buffer view points nowhere")?;
        let start = v["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = v["byteStride"].as_u64().map(|s| s as usize).unwrap_or(size * comps);
        let mut out = Vec::with_capacity(count * comps);
        for e in 0..count {
            for c in 0..comps {
                let at = start + e * stride + c * size;
                let b = buffer.get(at..at + size).ok_or("a glTF accessor runs past its buffer")?;
                out.push(match kind {
                    5121 => b[0] as f64,
                    5123 => u16::from_le_bytes([b[0], b[1]]) as f64,
                    5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                });
            }
        }
        Ok(out)
    };
    let look_of = |m: &Value| -> Option<Look> {
        let pbr = &m["pbrMetallicRoughness"];
        let f = pbr["baseColorFactor"].as_array()?;
        let c = [0, 1, 2].map(|k| to_srgb(f.get(k).and_then(Value::as_f64).unwrap_or(1.0)));
        Some(Look {
            color: c,
            metalness: pbr["metallicFactor"].as_f64().unwrap_or(1.0),
            roughness: pbr["roughnessFactor"].as_f64().unwrap_or(1.0),
            opacity: f.get(3).and_then(Value::as_f64).unwrap_or(1.0),
        })
    };
    // Y up, metres → Z up, millimetres.
    let to_forma: M4 = [MM_PER_M, 0.0, 0.0, 0.0, 0.0, 0.0, MM_PER_M, 0.0, 0.0, -MM_PER_M, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let mut out = Vec::new();
    let mut stack: Vec<(usize, M4, u32)> = Vec::new();
    let scene = &doc["scenes"][doc["scene"].as_u64().unwrap_or(0) as usize];
    let roots: Vec<usize> = match scene["nodes"].as_array() {
        Some(n) => n.iter().filter_map(Value::as_u64).map(|n| n as usize).collect(),
        // No scene: every node no other node holds.
        None => {
            let nodes = doc["nodes"].as_array().map(Vec::len).unwrap_or(0);
            let held: Vec<u64> = doc["nodes"].as_array().into_iter().flatten().flat_map(|n| n["children"].as_array().into_iter().flatten().filter_map(Value::as_u64)).collect();
            (0..nodes).filter(|i| !held.contains(&(*i as u64))).collect()
        }
    };
    for r in roots.into_iter().rev() {
        stack.push((r, to_forma, 0));
    }
    while let Some((n, parent, depth)) = stack.pop() {
        if depth > 64 {
            return Err("the glTF node tree is too deep (a cycle?)".into());
        }
        let node = &doc["nodes"][n];
        let world = mul(&parent, &node_matrix(node));
        for c in node["children"].as_array().into_iter().flatten().filter_map(Value::as_u64).rev() {
            stack.push((c as usize, world, depth + 1));
        }
        let Some(mi) = node["mesh"].as_u64() else { continue };
        let mesh = &doc["meshes"][mi as usize];
        let name = node["name"].as_str().or(mesh["name"].as_str()).map(str::to_string).unwrap_or_else(|| format!("Mesh {}", mi + 1));
        // One body per material within the node's mesh.
        let mut by_material: Vec<(Option<u64>, MeshBody)> = Vec::new();
        for p in mesh["primitives"].as_array().into_iter().flatten() {
            if p["mode"].as_u64().unwrap_or(4) != 4 {
                continue; // points and lines carry no solid
            }
            let pos = accessor(&p["attributes"]["POSITION"], true)?;
            let n_verts = pos.len() / 3;
            let idx: Vec<u32> = match p.get("indices") {
                Some(i) if !i.is_null() => accessor(i, false)?.into_iter().map(|v| v as u32).collect(),
                _ => (0..n_verts as u32).collect(),
            };
            if idx.iter().any(|&i| i as usize >= n_verts) {
                return Err(format!("{name}: a triangle uses a vertex that is not there"));
            }
            let mat = p["material"].as_u64();
            let slot = match by_material.iter().position(|(m, _)| *m == mat) {
                Some(s) => s,
                None => {
                    let look = mat.and_then(|m| look_of(&doc["materials"][m as usize]));
                    by_material.push((mat, MeshBody { name: name.clone(), positions: Vec::new(), triangles: Vec::new(), look, normals: Vec::new() }));
                    by_material.len() - 1
                }
            };
            let body = &mut by_material[slot].1;
            let base = (body.positions.len() / 3) as u32;
            for v in pos.chunks_exact(3) {
                for r in 0..3 {
                    body.positions.push(world[r] * v[0] + world[4 + r] * v[1] + world[8 + r] * v[2] + world[12 + r]);
                }
            }
            // A mirroring transform turns triangles inside out: wind them back.
            let det = world[0] * (world[5] * world[10] - world[9] * world[6]) - world[4] * (world[1] * world[10] - world[9] * world[2]) + world[8] * (world[1] * world[6] - world[5] * world[2]);
            for t in idx.chunks_exact(3) {
                if det < 0.0 {
                    body.triangles.extend([t[0] + base, t[2] + base, t[1] + base]);
                } else {
                    body.triangles.extend([t[0] + base, t[1] + base, t[2] + base]);
                }
            }
        }
        let several = by_material.len() > 1;
        for (m, mut body) in by_material {
            if several && let Some(mn) = m.and_then(|m| doc["materials"][m as usize]["name"].as_str()) {
                body.name = format!("{} ({mn})", body.name);
            }
            out.push(body);
        }
    }
    Ok(out)
}

/// A binary glTF: each part a node with its own mesh and a material with
/// its look (colour, metalness, roughness), metres, Y up. Parts that carry
/// normals get them (smooth across curves, sharp at edges).
pub fn write_glb(parts: &[MeshBody]) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let (mut nodes, mut meshes, mut accessors, mut views, mut materials) = (vec![], vec![], vec![], vec![], vec![]);
    let mut looks: Vec<Look> = Vec::new();
    let mut view = |bin: &mut Vec<u8>, bytes: &[u8], target: u32| -> usize {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        views.push(json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len(), "target": target }));
        bin.extend_from_slice(bytes);
        views.len() - 1
    };
    // Z up, millimetres → Y up, metres.
    let gl = |v: &[f64]| [v[0] / MM_PER_M, v[2] / MM_PER_M, -v[1] / MM_PER_M];
    for p in parts {
        if p.triangles.is_empty() {
            continue;
        }
        let mut pos = Vec::with_capacity(p.positions.len() * 4);
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in p.positions.chunks_exact(3) {
            let g = gl(v).map(|x| x as f32 as f64);
            for k in 0..3 {
                lo[k] = lo[k].min(g[k]);
                hi[k] = hi[k].max(g[k]);
                pos.extend_from_slice(&(g[k] as f32).to_le_bytes());
            }
        }
        let n = p.positions.len() / 3;
        let pv = view(&mut bin, &pos, 34962);
        accessors.push(json!({ "bufferView": pv, "componentType": 5126, "count": n, "type": "VEC3", "min": lo, "max": hi }));
        let mut attributes = json!({ "POSITION": accessors.len() - 1 });
        if p.normals.len() == p.positions.len() {
            let mut nb = Vec::with_capacity(pos.len());
            for v in p.normals.chunks_exact(3) {
                let g = [v[0], v[2], -v[1]];
                let l = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-12);
                for x in g {
                    nb.extend_from_slice(&((x / l) as f32).to_le_bytes());
                }
            }
            let nv = view(&mut bin, &nb, 34962);
            accessors.push(json!({ "bufferView": nv, "componentType": 5126, "count": n, "type": "VEC3" }));
            attributes["NORMAL"] = json!(accessors.len() - 1);
        }
        let idx: Vec<u8> = p.triangles.iter().flat_map(|i| i.to_le_bytes()).collect();
        let iv = view(&mut bin, &idx, 34963);
        accessors.push(json!({ "bufferView": iv, "componentType": 5125, "count": p.triangles.len(), "type": "SCALAR" }));
        let look = p.look.clone().unwrap_or(Look::DEFAULT);
        let m = match looks.iter().position(|l| *l == look) {
            Some(m) => m,
            None => {
                let c = look.color.map(to_linear);
                let mut mat = json!({
                    "name": format!("#{:02x}{:02x}{:02x}", (look.color[0] * 255.0).round() as u8, (look.color[1] * 255.0).round() as u8, (look.color[2] * 255.0).round() as u8),
                    "pbrMetallicRoughness": { "baseColorFactor": [c[0], c[1], c[2], look.opacity], "metallicFactor": look.metalness, "roughnessFactor": look.roughness },
                    "doubleSided": false,
                });
                if look.opacity < 1.0 {
                    mat["alphaMode"] = json!("BLEND");
                }
                materials.push(mat);
                looks.push(look);
                looks.len() - 1
            }
        };
        meshes.push(json!({ "name": p.name, "primitives": [{ "attributes": attributes, "indices": accessors.len() - 1, "material": m, "mode": 4 }] }));
        nodes.push(json!({ "name": p.name, "mesh": meshes.len() - 1 }));
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let doc = json!({
        "asset": { "version": "2.0", "generator": "Forma" },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }],
    });
    let mut js = doc.to_string().into_bytes();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out
}
