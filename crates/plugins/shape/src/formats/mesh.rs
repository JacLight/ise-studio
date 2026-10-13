//! Triangle-mesh files: STL (ASCII and binary), OBJ and PLY, read and written.
//! Units are millimetres both ways (the 3D-printing convention these formats
//! follow in practice), Z up.

use super::{Look, MeshBody};
use std::fmt::Write as _;

// ---- STL ---------------------------------------------------------------------------

/// An STL file's solids (an ASCII file may hold several, each named).
pub fn read_stl(data: &[u8]) -> Result<Vec<MeshBody>, String> {
    let binary_size = (data.len() >= 84).then(|| 84 + 50 * u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize);
    // Binary when the size adds up (some binary files start with "solid" too).
    if binary_size == Some(data.len()) {
        let n = (data.len() - 84) / 50;
        let mut positions = Vec::with_capacity(n * 9);
        for t in 0..n {
            let at = 84 + 50 * t + 12;
            for k in 0..9 {
                let b = &data[at + 4 * k..at + 4 * k + 4];
                positions.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64);
            }
        }
        let header = String::from_utf8_lossy(&data[..80]).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        let name = header.strip_prefix("solid").map(|s| s.trim().to_string()).unwrap_or_default();
        return Ok(vec![MeshBody::soup(name, positions)]);
    }
    let text = std::str::from_utf8(data).map_err(|_| "not an STL file (neither binary nor text)")?;
    if !text.trim_start().starts_with("solid") {
        return Err("not an STL file".into());
    }
    let mut bodies = Vec::new();
    let mut name = String::new();
    let mut positions = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("vertex") {
            let v: Vec<f64> = rest.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            if v.len() != 3 {
                return Err(format!("a bad STL vertex: “{line}”"));
            }
            positions.extend(v);
        } else if line.starts_with("endsolid") {
            bodies.push(MeshBody::soup(std::mem::take(&mut name), std::mem::take(&mut positions)));
        } else if let Some(rest) = line.strip_prefix("solid") {
            name = rest.trim().to_string();
        }
    }
    if !positions.is_empty() {
        bodies.push(MeshBody::soup(name, positions));
    }
    bodies.retain(|b| !b.triangles.is_empty());
    Ok(bodies)
}

// ---- OBJ ---------------------------------------------------------------------------

/// An OBJ file's objects (`o`) or groups (`g`), each a body; vertex colours
/// (`v x y z r g b`) give a body its colour.
pub fn read_obj(data: &[u8]) -> Result<Vec<MeshBody>, String> {
    let text = String::from_utf8_lossy(data);
    let mut verts: Vec<[f64; 3]> = Vec::new();
    let mut colors: Vec<Option<[f64; 3]>> = Vec::new();
    // Per body: name and faces (as indices into `verts`).
    let mut bodies: Vec<(String, Vec<[usize; 3]>)> = vec![(String::new(), Vec::new())];
    for (n, line) in text.lines().enumerate() {
        let mut t = line.split_whitespace();
        match t.next() {
            Some("v") => {
                let v: Vec<f64> = t.filter_map(|x| x.parse().ok()).collect();
                if v.len() < 3 {
                    return Err(format!("line {}: a vertex needs x, y and z", n + 1));
                }
                verts.push([v[0], v[1], v[2]]);
                colors.push((v.len() >= 6).then(|| [v[3], v[4], v[5]]));
            }
            Some("f") => {
                let mut idx = Vec::new();
                for item in t {
                    let first = item.split('/').next().unwrap_or("");
                    let i: i64 = first.parse().map_err(|_| format!("line {}: a bad face index “{item}”", n + 1))?;
                    // 1-based, or negative counting back from the latest vertex.
                    let i = if i < 0 { verts.len() as i64 + i } else { i - 1 };
                    if i < 0 || i as usize >= verts.len() {
                        return Err(format!("line {}: face index {first} is out of range", n + 1));
                    }
                    idx.push(i as usize);
                }
                let faces = &mut bodies.last_mut().unwrap().1;
                for k in 1..idx.len().saturating_sub(1) {
                    faces.push([idx[0], idx[k], idx[k + 1]]);
                }
            }
            Some("o") | Some("g") => {
                let name = t.collect::<Vec<_>>().join(" ");
                let last = bodies.last_mut().unwrap();
                if last.1.is_empty() {
                    last.0 = name;
                } else {
                    bodies.push((name, Vec::new()));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (name, faces) in bodies.into_iter().filter(|(_, f)| !f.is_empty()) {
        // Only the vertices this body uses, renumbered.
        let mut map = std::collections::HashMap::new();
        let mut positions = Vec::new();
        let mut color_sum = [0.0; 3];
        let mut colored = 0;
        let mut triangles = Vec::with_capacity(faces.len() * 3);
        for f in &faces {
            for &v in f {
                let next = map.len() as u32;
                let i = *map.entry(v).or_insert_with(|| {
                    positions.extend(verts[v]);
                    if let Some(c) = colors[v] {
                        (0..3).for_each(|k| color_sum[k] += c[k]);
                        colored += 1;
                    }
                    next
                });
                triangles.push(i);
            }
        }
        let color = (colored > 0).then(|| color_sum.map(|c| (c / colored as f64).clamp(0.0, 1.0)));
        out.push(MeshBody { name, positions, triangles, look: color.map(Look::color), normals: Vec::new() });
    }
    Ok(out)
}

/// An OBJ file: each part an object, millimetres, Z up, with its colour on
/// every vertex (the common `v x y z r g b` extension) when it has one.
pub fn write_obj(parts: &[MeshBody]) -> Vec<u8> {
    let mut s = String::from("# Forma export: millimetres, Z up\n");
    let mut base = 1;
    for p in parts {
        let _ = writeln!(s, "o {}", p.name.replace(['\n', '\r'], " "));
        let rgb = p.look.as_ref().map(|l| l.color);
        for v in p.positions.chunks_exact(3) {
            match rgb {
                Some(c) => {
                    let _ = writeln!(s, "v {} {} {} {:.4} {:.4} {:.4}", v[0], v[1], v[2], c[0], c[1], c[2]);
                }
                None => {
                    let _ = writeln!(s, "v {} {} {}", v[0], v[1], v[2]);
                }
            }
        }
        for t in p.triangles.chunks_exact(3) {
            let _ = writeln!(s, "f {} {} {}", t[0] as usize + base, t[1] as usize + base, t[2] as usize + base);
        }
        base += p.positions.len() / 3;
    }
    s.into_bytes()
}

// ---- PLY ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> Option<Scalar> {
        Some(match s {
            "char" | "int8" => Scalar::I8,
            "uchar" | "uint8" => Scalar::U8,
            "short" | "int16" => Scalar::I16,
            "ushort" | "uint16" => Scalar::U16,
            "int" | "int32" => Scalar::I32,
            "uint" | "uint32" => Scalar::U32,
            "float" | "float32" => Scalar::F32,
            "double" | "float64" => Scalar::F64,
            _ => return None,
        })
    }
    fn size(self) -> usize {
        match self {
            Scalar::I8 | Scalar::U8 => 1,
            Scalar::I16 | Scalar::U16 => 2,
            Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
            Scalar::F64 => 8,
        }
    }
}

struct Property {
    name: String,
    /// For a list: the count's type, then the items'.
    list: Option<Scalar>,
    kind: Scalar,
}

struct Element {
    name: String,
    count: usize,
    props: Vec<Property>,
}

/// Values read in order, from text or from bytes of either order.
enum Reader<'a> {
    Text(std::str::SplitAsciiWhitespace<'a>),
    Binary { data: &'a [u8], at: usize, big: bool },
}

impl Reader<'_> {
    fn next(&mut self, kind: Scalar) -> Result<f64, String> {
        match self {
            Reader::Text(t) => t.next().and_then(|v| v.parse().ok()).ok_or_else(|| "the PLY data is cut short or not a number".to_string()),
            Reader::Binary { data, at, big } => {
                let n = kind.size();
                let b = data.get(*at..*at + n).ok_or("the PLY data is cut short")?;
                *at += n;
                let mut a = [0u8; 8];
                a[..n].copy_from_slice(b);
                if *big {
                    a[..n].reverse();
                }
                Ok(match kind {
                    Scalar::I8 => a[0] as i8 as f64,
                    Scalar::U8 => a[0] as f64,
                    Scalar::I16 => i16::from_le_bytes([a[0], a[1]]) as f64,
                    Scalar::U16 => u16::from_le_bytes([a[0], a[1]]) as f64,
                    Scalar::I32 => i32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
                    Scalar::U32 => u32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
                    Scalar::F32 => f32::from_le_bytes([a[0], a[1], a[2], a[3]]) as f64,
                    Scalar::F64 => f64::from_le_bytes(a),
                })
            }
        }
    }
}

/// A PLY file (text or binary) as one body; vertex colours give it its colour.
pub fn read_ply(data: &[u8]) -> Result<Vec<MeshBody>, String> {
    let end = data.windows(10).position(|w| w == b"end_header").ok_or("not a PLY file (no end_header)")?;
    let header = std::str::from_utf8(&data[..end]).map_err(|_| "not a PLY file")?;
    if !header.starts_with("ply") {
        return Err("not a PLY file".into());
    }
    let mut body = end + 10;
    while body < data.len() && data[body] != b'\n' {
        body += 1;
    }
    body += 1;
    let mut format = "";
    let mut elements: Vec<Element> = Vec::new();
    for line in header.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.as_slice() {
            ["format", f, ..] => format = f,
            ["element", name, count] => elements.push(Element { name: name.to_string(), count: count.parse().map_err(|_| "a bad PLY element count")?, props: Vec::new() }),
            ["property", "list", count, item, name] => {
                let el = elements.last_mut().ok_or("a PLY property outside an element")?;
                el.props.push(Property { name: name.to_string(), list: Some(Scalar::parse(count).ok_or("a bad PLY type")?), kind: Scalar::parse(item).ok_or("a bad PLY type")? });
            }
            ["property", kind, name] => {
                let el = elements.last_mut().ok_or("a PLY property outside an element")?;
                el.props.push(Property { name: name.to_string(), list: None, kind: Scalar::parse(kind).ok_or_else(|| format!("unknown PLY type {kind}"))? });
            }
            _ => {}
        }
    }
    let rest = data.get(body..).unwrap_or_default();
    let mut r = match format {
        "ascii" => Reader::Text(std::str::from_utf8(rest).map_err(|_| "the PLY text is not UTF-8")?.split_ascii_whitespace()),
        "binary_little_endian" => Reader::Binary { data: rest, at: 0, big: false },
        "binary_big_endian" => Reader::Binary { data: rest, at: 0, big: true },
        f => return Err(format!("unknown PLY format “{f}”")),
    };
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    let mut color_sum = [0.0; 3];
    let mut colored = 0usize;
    for el in &elements {
        let find = |n: &str| el.props.iter().position(|p| p.name == n);
        let (x, y, z) = (find("x"), find("y"), find("z"));
        let rgb = [find("red").or(find("r")), find("green").or(find("g")), find("blue").or(find("b"))];
        let faces = el.props.iter().position(|p| p.list.is_some() && (p.name == "vertex_indices" || p.name == "vertex_index"));
        let mut values = vec![0.0; el.props.len()];
        for _ in 0..el.count {
            let mut list = Vec::new();
            for (k, p) in el.props.iter().enumerate() {
                match p.list {
                    Some(count) => {
                        let n = r.next(count)? as usize;
                        let items: Vec<f64> = (0..n).map(|_| r.next(p.kind)).collect::<Result<_, _>>()?;
                        if Some(k) == faces {
                            list = items;
                        }
                    }
                    None => values[k] = r.next(p.kind)?,
                }
            }
            if el.name == "vertex" {
                let (Some(x), Some(y), Some(z)) = (x, y, z) else { return Err("PLY vertices need x, y and z".into()) };
                positions.extend([values[x], values[y], values[z]]);
                if let [Some(rr), Some(gg), Some(bb)] = rgb {
                    // Bytes 0…255, or floats 0…1.
                    let scale = if el.props[rr].kind == Scalar::U8 { 255.0 } else { 1.0 };
                    color_sum[0] += values[rr] / scale;
                    color_sum[1] += values[gg] / scale;
                    color_sum[2] += values[bb] / scale;
                    colored += 1;
                }
            } else if el.name == "face" {
                if list.iter().any(|v| *v < 0.0) {
                    return Err("a PLY face index is negative".into());
                }
                for k in 1..list.len().saturating_sub(1) {
                    triangles.extend([list[0], list[k], list[k + 1]].map(|v| v as u32));
                }
            }
        }
    }
    let n = (positions.len() / 3) as u32;
    if triangles.iter().any(|&t| t >= n) {
        return Err("a PLY face uses a vertex that is not in the file".into());
    }
    let color = (colored > 0).then(|| color_sum.map(|c| (c / colored as f64).clamp(0.0, 1.0)));
    Ok(vec![MeshBody { name: String::new(), positions, triangles, look: color.map(Look::color), normals: Vec::new() }])
}

/// A binary PLY: every part in one mesh (PLY has no objects), each vertex
/// coloured with its part's colour, millimetres, Z up.
pub fn write_ply(parts: &[MeshBody]) -> Vec<u8> {
    let verts: usize = parts.iter().map(|p| p.positions.len() / 3).sum();
    let faces: usize = parts.iter().map(|p| p.triangles.len() / 3).sum();
    let mut out = String::from("ply\nformat binary_little_endian 1.0\ncomment Forma export: millimetres, Z up\n");
    for p in parts {
        let _ = writeln!(out, "comment part {}", p.name.replace(['\n', '\r'], " "));
    }
    let _ = write!(
        out,
        "element vertex {verts}\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face {faces}\nproperty list uchar int vertex_indices\nend_header\n"
    );
    let mut b = out.into_bytes();
    b.reserve(verts * 15 + faces * 13);
    for p in parts {
        let c = p.look.as_ref().map(|l| l.color).unwrap_or(Look::DEFAULT.color).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        for v in p.positions.chunks_exact(3) {
            for x in v {
                b.extend_from_slice(&(*x as f32).to_le_bytes());
            }
            b.extend_from_slice(&c);
        }
    }
    let mut base = 0u32;
    for p in parts {
        for t in p.triangles.chunks_exact(3) {
            b.push(3);
            for i in t {
                b.extend_from_slice(&((i + base) as i32).to_le_bytes());
            }
        }
        base += (p.positions.len() / 3) as u32;
    }
    b
}
