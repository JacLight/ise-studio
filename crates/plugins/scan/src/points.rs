//! Point clouds: reading the formats scanners and photogrammetry write
//! (XYZ/ASC/CSV text, PLY, PCD, E57), thinning them, dropping stray points,
//! estimating normals (principal components of each point's neighbours,
//! oriented by propagation over the neighbour graph after Hoppe et al.
//! 1992), and sampling a mesh into points.
//!
//! This is the entry point for everything that makes points: a scanner's
//! export, COLMAP's fused PLY, Apple Object Capture's point cloud. Whatever
//! produced them, they arrive here as a `Cloud`.

use crate::V3;
use crate::linalg::*;
use crate::mesh::Mesh;
use crate::spatial::KdTree;
use rayon::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cloud {
    pub positions: Vec<V3>,
    /// Unit normals, one per point, or empty.
    pub normals: Vec<V3>,
    /// sRGB colours, one per point, or empty.
    pub colors: Vec<[u8; 3]>,
}

/// A point cloud's formats, by extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Xyz,
    Ply,
    Pcd,
    E57,
}

impl Format {
    pub fn from_name(name: &str) -> Option<Format> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "xyz" | "asc" | "pts" | "txt" | "csv" | "xyzn" | "xyzrgb" => Format::Xyz,
            "ply" => Format::Ply,
            "pcd" => Format::Pcd,
            "e57" => Format::E57,
            _ => return None,
        })
    }

    pub fn sniff(data: &[u8]) -> Option<Format> {
        if data.starts_with(b"ASTM-E57") {
            return Some(Format::E57);
        }
        if data.starts_with(b"ply") {
            return Some(Format::Ply);
        }
        let head = String::from_utf8_lossy(&data[..data.len().min(2048)]);
        if head.lines().any(|l| l.trim_start().starts_with("FIELDS ")) && head.contains("DATA ") {
            return Some(Format::Pcd);
        }
        // Text of numbers, three or more a line.
        let mut lines = head.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#') && !l.starts_with("//"));
        let first = lines.next()?;
        let nums = |l: &str| l.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|t| !t.is_empty()).filter(|t| t.parse::<f64>().is_ok()).count();
        // A header line (a count, or names) may come first.
        let probe = if nums(first) >= 3 { first } else { lines.next()? };
        (nums(probe) >= 3).then_some(Format::Xyz)
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Xyz => "xyz",
            Format::Ply => "ply",
            Format::Pcd => "pcd",
            Format::E57 => "e57",
        }
    }
}

impl Cloud {
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    pub fn bounds(&self) -> (V3, V3) {
        crate::mesh::bounds(&self.positions)
    }

    /// Only the points at these indices.
    pub fn select(&self, keep: &[u32]) -> Cloud {
        Cloud {
            positions: keep.iter().map(|&i| self.positions[i as usize]).collect(),
            normals: if self.normals.is_empty() { vec![] } else { keep.iter().map(|&i| self.normals[i as usize]).collect() },
            colors: if self.colors.is_empty() { vec![] } else { keep.iter().map(|&i| self.colors[i as usize]).collect() },
        }
    }

    /// Mean distance from each point to its nearest neighbour (a sample of up to 2000 points).
    pub fn spacing(&self) -> f64 {
        if self.len() < 2 {
            return 0.0;
        }
        let tree = KdTree::new(&self.positions);
        let step = (self.len() / 2000).max(1);
        let ds: Vec<f64> = (0..self.len()).step_by(step).map(|i| tree.knn(self.positions[i], 2).get(1).map(|e| e.1.sqrt()).unwrap_or(0.0)).collect();
        ds.iter().sum::<f64>() / ds.len() as f64
    }

    /// Moved by a 3×4 affine matrix (rows); normals turned with it.
    pub fn transform(&mut self, m: &[[f64; 4]; 3]) {
        for p in self.positions.iter_mut() {
            *p = crate::mesh::apply_affine(m, *p);
        }
        let r = [[m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]];
        for n in self.normals.iter_mut() {
            *n = unit(mat_vec(&r, *n));
        }
    }
}

// ---- Reading ---------------------------------------------------------------------

pub fn read(data: &[u8], format: Format) -> Result<Cloud, String> {
    let c = match format {
        Format::Xyz => read_xyz(data)?,
        Format::Ply => read_ply(data)?.0,
        Format::Pcd => read_pcd(data)?,
        Format::E57 => crate::e57::read(data)?,
    };
    if c.is_empty() {
        return Err("the file has no points".into());
    }
    if c.positions.iter().any(|p| !p.iter().all(|v| v.is_finite())) {
        let keep: Vec<u32> = (0..c.len() as u32).filter(|&i| c.positions[i as usize].iter().all(|v| v.is_finite())).collect();
        return Ok(c.select(&keep));
    }
    Ok(c)
}

/// Text: x y z [nx ny nz] [r g b] per line, separated by spaces, commas or
/// semicolons; header lines and comments skipped. Six numbers are taken as
/// normals when they are unit length, else as colours.
pub fn read_xyz(data: &[u8]) -> Result<Cloud, String> {
    let text = String::from_utf8_lossy(data);
    let mut c = Cloud::default();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with("//") {
            continue;
        }
        let v: Vec<f64> = l.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|t| !t.is_empty()).map_while(|t| t.parse::<f64>().ok()).collect();
        if v.len() < 3 {
            continue;
        }
        c.positions.push([v[0], v[1], v[2]]);
        let as_colour = |s: &[f64]| -> [u8; 3] {
            let max = s.iter().fold(0.0f64, |m, x| m.max(*x));
            let k = if max <= 1.0 { 255.0 } else { 1.0 };
            [0, 1, 2].map(|i| (s[i] * k).round().clamp(0.0, 255.0) as u8)
        };
        match v.len() {
            6 => {
                let n = [v[3], v[4], v[5]];
                if (norm(n) - 1.0).abs() < 0.01 {
                    normals.push(n);
                } else {
                    colors.push(as_colour(&v[3..6]));
                }
            }
            7 => colors.push(as_colour(&v[4..7])), // x y z intensity r g b (PTS)
            n if n >= 9 => {
                normals.push(unit([v[3], v[4], v[5]]));
                colors.push(as_colour(&v[6..9]));
            }
            _ => {}
        }
    }
    if normals.len() == c.positions.len() {
        c.normals = normals;
    }
    if colors.len() == c.positions.len() {
        c.colors = colors;
    }
    Ok(c)
}

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
    fn parse(t: &str) -> Option<Scalar> {
        Some(match t {
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
    fn read(self, b: &[u8], big: bool) -> f64 {
        macro_rules! get {
            ($t:ty, $n:expr) => {{
                let a: [u8; $n] = b[..$n].try_into().unwrap();
                (if big { <$t>::from_be_bytes(a) } else { <$t>::from_le_bytes(a) }) as f64
            }};
        }
        match self {
            Scalar::I8 => b[0] as i8 as f64,
            Scalar::U8 => b[0] as f64,
            Scalar::I16 => get!(i16, 2),
            Scalar::U16 => get!(u16, 2),
            Scalar::I32 => get!(i32, 4),
            Scalar::U32 => get!(u32, 4),
            Scalar::F32 => get!(f32, 4),
            Scalar::F64 => get!(f64, 8),
        }
    }
}

/// A PLY file's vertices as points, and its faces (empty for a point cloud).
pub fn read_ply(data: &[u8]) -> Result<(Cloud, Vec<[u32; 3]>), String> {
    let end = data.windows(10).position(|w| w == b"end_header").ok_or("not a PLY file (no end_header)")?;
    let header = String::from_utf8_lossy(&data[..end]);
    let mut body = end + 10;
    while body < data.len() && (data[body] == b'\r' || data[body] == b'\n') {
        body += 1;
        if data[body - 1] == b'\n' {
            break;
        }
    }
    let mut format = "ascii";
    // Elements: name, count, properties (name, type, list count type).
    let mut elements: Vec<(String, usize, Vec<(String, Scalar, Option<Scalar>)>)> = Vec::new();
    for line in header.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.as_slice() {
            ["format", f, ..] => format = if f.starts_with("binary_little") { "le" } else if f.starts_with("binary_big") { "be" } else { "ascii" },
            ["element", name, n] => elements.push((name.to_string(), n.parse().map_err(|_| "bad PLY element count")?, Vec::new())),
            ["property", "list", c, ty, name] => {
                let (c, ty) = (Scalar::parse(c).ok_or("bad PLY type")?, Scalar::parse(ty).ok_or("bad PLY type")?);
                elements.last_mut().ok_or("PLY property before element")?.2.push((name.to_string(), ty, Some(c)));
            }
            ["property", ty, name] => {
                let ty = Scalar::parse(ty).ok_or_else(|| format!("unknown PLY type {ty}"))?;
                elements.last_mut().ok_or("PLY property before element")?.2.push((name.to_string(), ty, None));
            }
            _ => {}
        }
    }
    let mut cloud = Cloud::default();
    let mut faces = Vec::new();
    let big = format == "be";
    let mut pos = body;
    let mut words = if format == "ascii" { Some(std::str::from_utf8(&data[body..]).map_err(|_| "PLY text is not UTF-8")?.split_ascii_whitespace()) } else { None };
    for (name, count, props) in &elements {
        let idx = |n: &str| props.iter().position(|p| p.0 == n);
        let (x, y, z) = (idx("x"), idx("y"), idx("z"));
        let (nx, ny, nz) = (idx("nx"), idx("ny"), idx("nz"));
        let (r, g, b) = (idx("red").or(idx("r")), idx("green").or(idx("g")), idx("blue").or(idx("b")));
        let mut vals = vec![0.0f64; props.len()];
        for _ in 0..*count {
            let mut list: Vec<u32> = Vec::new();
            for (k, (_, ty, lc)) in props.iter().enumerate() {
                // One value of a type: the next word (text) or the next bytes (binary).
                let mut read_one = |t: Scalar| -> Result<f64, String> {
                    if let Some(w) = words.as_mut() {
                        return w.next().ok_or("PLY ends early")?.parse::<f64>().map_err(|_| "bad PLY number".to_string());
                    }
                    let v = t.read(data.get(pos..).filter(|s| s.len() >= t.size()).ok_or("PLY ends early")?, big);
                    pos += t.size();
                    Ok(v)
                };
                match lc {
                    Some(c) => {
                        let n = read_one(*c)? as usize;
                        list.clear();
                        for _ in 0..n {
                            list.push(read_one(*ty)? as u32);
                        }
                    }
                    None => vals[k] = read_one(*ty)?,
                }
            }
            if name == "vertex" {
                if let (Some(x), Some(y), Some(z)) = (x, y, z) {
                    cloud.positions.push([vals[x], vals[y], vals[z]]);
                }
                if let (Some(a), Some(b2), Some(c)) = (nx, ny, nz) {
                    cloud.normals.push(unit([vals[a], vals[b2], vals[c]]));
                }
                if let (Some(rr), Some(gg), Some(bb)) = (r, g, b) {
                    let float = matches!(props[rr].1, Scalar::F32 | Scalar::F64);
                    let k = if float { 255.0 } else { 1.0 };
                    cloud.colors.push([rr, gg, bb].map(|i| (vals[i] * k).round().clamp(0.0, 255.0) as u8));
                }
            } else if name == "face" && list.len() >= 3 {
                for i in 1..list.len() - 1 {
                    faces.push([list[0], list[i], list[i + 1]]);
                }
            }
        }
    }
    if cloud.normals.len() != cloud.positions.len() {
        cloud.normals.clear();
    }
    if cloud.colors.len() != cloud.positions.len() {
        cloud.colors.clear();
    }
    Ok((cloud, faces))
}

/// LZF decompression (PCD's binary_compressed).
fn lzf_decompress(input: &[u8], out_len: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(out_len);
    let mut i = 0;
    while i < input.len() {
        let ctrl = input[i] as usize;
        i += 1;
        if ctrl < 32 {
            let n = ctrl + 1;
            out.extend_from_slice(input.get(i..i + n).ok_or("PCD data ends early")?);
            i += n;
        } else {
            let mut len = ctrl >> 5;
            if len == 7 {
                len += *input.get(i).ok_or("PCD data ends early")? as usize;
                i += 1;
            }
            let back = ((ctrl & 0x1f) << 8) + *input.get(i).ok_or("PCD data ends early")? as usize + 1;
            i += 1;
            if back > out.len() {
                return Err("bad PCD compressed data".into());
            }
            let start = out.len() - back;
            for k in 0..len + 2 {
                let b = out[start + k];
                out.push(b);
            }
        }
    }
    if out.len() != out_len {
        return Err("bad PCD compressed data (size)".into());
    }
    Ok(out)
}

/// PCD (the Point Cloud Library's format): ascii, binary or binary_compressed.
pub fn read_pcd(data: &[u8]) -> Result<Cloud, String> {
    let mut fields: Vec<String> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();
    let mut types: Vec<char> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    let mut points = 0usize;
    let mut mode = String::new();
    let mut offset = 0usize;
    let mut viewpoint: Option<([f64; 3], [f64; 4])> = None;
    for line in data.split(|b| *b == b'\n') {
        offset += line.len() + 1;
        let l = String::from_utf8_lossy(line);
        let t: Vec<&str> = l.split_whitespace().collect();
        match t.first().copied() {
            Some("FIELDS") => fields = t[1..].iter().map(|s| s.to_string()).collect(),
            Some("SIZE") => sizes = t[1..].iter().filter_map(|s| s.parse().ok()).collect(),
            Some("TYPE") => types = t[1..].iter().filter_map(|s| s.chars().next()).collect(),
            Some("COUNT") => counts = t[1..].iter().filter_map(|s| s.parse().ok()).collect(),
            Some("POINTS") => points = t.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
            Some("VIEWPOINT") if t.len() == 8 => {
                let v: Vec<f64> = t[1..].iter().filter_map(|s| s.parse().ok()).collect();
                if v.len() == 7 {
                    viewpoint = Some(([v[0], v[1], v[2]], [v[3], v[4], v[5], v[6]]));
                }
            }
            Some("DATA") => {
                mode = t.get(1).unwrap_or(&"ascii").to_string();
                break;
            }
            _ => {}
        }
    }
    if fields.is_empty() || mode.is_empty() {
        return Err("not a PCD file (no FIELDS or DATA)".into());
    }
    if counts.len() != fields.len() {
        counts = vec![1; fields.len()];
    }
    if sizes.len() != fields.len() || types.len() != fields.len() {
        return Err("PCD SIZE/TYPE do not match FIELDS".into());
    }
    // Each field's scalars per point (fields may repeat, e.g. COUNT 3).
    let width: usize = counts.iter().sum();
    let col = |name: &str| -> Option<usize> {
        let i = fields.iter().position(|f| f == name)?;
        Some(counts[..i].iter().sum())
    };
    let mut values: Vec<f64> = Vec::with_capacity(points * width);
    let scalar = |ty: char, size: usize| -> Option<Scalar> {
        Some(match (ty, size) {
            ('F', 4) => Scalar::F32,
            ('F', 8) => Scalar::F64,
            ('U', 1) => Scalar::U8,
            ('U', 2) => Scalar::U16,
            ('U', 4) => Scalar::U32,
            ('I', 1) => Scalar::I8,
            ('I', 2) => Scalar::I16,
            ('I', 4) => Scalar::I32,
            _ => return None,
        })
    };
    let rgb_col = col("rgb").or(col("rgba"));
    let rgb_raw = |bits: u32| [((bits >> 16) & 255) as u8, ((bits >> 8) & 255) as u8, (bits & 255) as u8];
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let body = data.get(offset..).unwrap_or(&[]);
    match mode.as_str() {
        "ascii" => {
            let text = String::from_utf8_lossy(body);
            for line in text.lines() {
                let v: Vec<&str> = line.split_whitespace().collect();
                if v.len() < width {
                    continue;
                }
                let row: Vec<f64> = v[..width].iter().map(|s| s.parse::<f64>().unwrap_or(f64::NAN)).collect();
                if let Some(c) = rgb_col {
                    // rgb is a float holding packed bytes, or an integer.
                    let bits = if v[c].contains('.') || v[c].contains('e') { (row[c] as f32).to_bits() } else { row[c] as u32 };
                    colors.push(rgb_raw(bits));
                }
                values.extend(row);
            }
        }
        "binary" | "binary_compressed" => {
            let raw;
            let plain: &[u8] = if mode == "binary" {
                body
            } else {
                if body.len() < 8 {
                    return Err("PCD compressed data is missing".into());
                }
                let csize = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;
                let usize_ = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
                raw = lzf_decompress(body.get(8..8 + csize).ok_or("PCD compressed data ends early")?, usize_)?;
                &raw
            };
            let stride: usize = sizes.iter().zip(&counts).map(|(s, c)| s * c).sum();
            // binary: point after point; binary_compressed: field after field.
            let field_offset: Vec<usize> = (0..fields.len()).map(|i| sizes[..i].iter().zip(&counts[..i]).map(|(s, c)| s * c).sum()).collect();
            values.resize(points * width, 0.0);
            for p in 0..points {
                let mut k = 0;
                for f in 0..fields.len() {
                    let sc = scalar(types[f], sizes[f]).ok_or("unsupported PCD field type")?;
                    for c in 0..counts[f] {
                        let at = if mode == "binary" { p * stride + field_offset[f] + c * sizes[f] } else { points * field_offset[f] + (p * counts[f] + c) * sizes[f] };
                        let bytes = plain.get(at..at + sizes[f]).ok_or("PCD data ends early")?;
                        values[p * width + k] = sc.read(bytes, false);
                        if Some(k) == rgb_col {
                            colors.push(rgb_raw(u32::from_le_bytes(bytes[..4].try_into().unwrap_or([0; 4]))));
                        }
                        k += 1;
                    }
                }
            }
        }
        other => return Err(format!("unsupported PCD data mode {other}")),
    }
    let (x, y, z) = (col("x").ok_or("PCD has no x field")?, col("y").ok_or("PCD has no y field")?, col("z").ok_or("PCD has no z field")?);
    let normals = match (col("normal_x"), col("normal_y"), col("normal_z")) {
        (Some(a), Some(b), Some(c)) => Some((a, b, c)),
        _ => None,
    };
    let mut cloud = Cloud::default();
    let rows = values.len() / width.max(1);
    for p in 0..rows {
        let r = &values[p * width..p * width + width];
        if !(r[x].is_finite() && r[y].is_finite() && r[z].is_finite()) {
            continue;
        }
        cloud.positions.push([r[x], r[y], r[z]]);
        if let Some((a, b, c)) = normals {
            cloud.normals.push(unit([r[a], r[b], r[c]]));
        }
        if !colors.is_empty() {
            cloud.colors.push(colors[p]);
        }
    }
    if cloud.colors.len() != cloud.positions.len() {
        cloud.colors.clear();
    }
    if let Some((t, q)) = viewpoint {
        if t != [0.0; 3] || q != [1.0, 0.0, 0.0, 0.0] {
            let r = quat_matrix(q);
            cloud.transform(&[[r[0][0], r[0][1], r[0][2], t[0]], [r[1][0], r[1][1], r[1][2], t[1]], [r[2][0], r[2][1], r[2][2], t[2]]]);
        }
    }
    Ok(cloud)
}

// ---- Writing ---------------------------------------------------------------------

/// Binary little-endian PLY of the points (with normals and colours when present).
pub fn write_ply(c: &Cloud) -> Vec<u8> {
    let mut h = format!("ply\nformat binary_little_endian 1.0\ncomment Forma point cloud\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\n", c.len());
    if !c.normals.is_empty() {
        h.push_str("property float nx\nproperty float ny\nproperty float nz\n");
    }
    if !c.colors.is_empty() {
        h.push_str("property uchar red\nproperty uchar green\nproperty uchar blue\n");
    }
    h.push_str("end_header\n");
    let mut out = h.into_bytes();
    for i in 0..c.len() {
        for v in c.positions[i] {
            out.extend_from_slice(&(v as f32).to_le_bytes());
        }
        if !c.normals.is_empty() {
            for v in c.normals[i] {
                out.extend_from_slice(&(v as f32).to_le_bytes());
            }
        }
        if !c.colors.is_empty() {
            out.extend_from_slice(&c.colors[i]);
        }
    }
    out
}

/// Text: x y z [nx ny nz] per line.
pub fn write_xyz(c: &Cloud) -> Vec<u8> {
    let mut s = String::with_capacity(c.len() * 40);
    for i in 0..c.len() {
        let p = c.positions[i];
        s.push_str(&format!("{} {} {}", p[0], p[1], p[2]));
        if !c.normals.is_empty() {
            let n = c.normals[i];
            s.push_str(&format!(" {:.6} {:.6} {:.6}", n[0], n[1], n[2]));
        }
        s.push('\n');
    }
    s.into_bytes()
}

// ---- Processing ------------------------------------------------------------------

/// One point per cube of side `voxel` (the average of the points in it).
pub fn voxel_downsample(c: &Cloud, voxel: f64) -> Cloud {
    use std::collections::HashMap;
    let mut cells: HashMap<[i64; 3], (V3, V3, [u32; 3], u32)> = HashMap::new();
    let mut order = Vec::new();
    for i in 0..c.len() {
        let p = c.positions[i];
        let k = [(p[0] / voxel).floor() as i64, (p[1] / voxel).floor() as i64, (p[2] / voxel).floor() as i64];
        let e = cells.entry(k).or_insert_with(|| {
            order.push(k);
            ([0.0; 3], [0.0; 3], [0; 3], 0)
        });
        e.0 = add(e.0, p);
        if !c.normals.is_empty() {
            // Normals averaged in the first one's hemisphere.
            let n = c.normals[i];
            e.1 = add(e.1, if dot(e.1, n) < 0.0 { scale(n, -1.0) } else { n });
        }
        if !c.colors.is_empty() {
            for k in 0..3 {
                e.2[k] += c.colors[i][k] as u32;
            }
        }
        e.3 += 1;
    }
    let mut out = Cloud::default();
    for k in order {
        let (s, n, col, count) = cells[&k];
        out.positions.push(scale(s, 1.0 / count as f64));
        if !c.normals.is_empty() {
            out.normals.push(unit(n));
        }
        if !c.colors.is_empty() {
            out.colors.push(col.map(|v| (v / count) as u8));
        }
    }
    out
}

/// About `count` points, evenly through the file's order.
pub fn take_count(c: &Cloud, count: usize) -> Cloud {
    if count >= c.len() || count == 0 {
        return c.clone();
    }
    let keep: Vec<u32> = (0..count).map(|i| ((i as u64 * c.len() as u64) / count as u64) as u32).collect();
    c.select(&keep)
}

/// Points whose mean distance to their `k` neighbours is more than `sigma`
/// standard deviations above the average, dropped (statistical outlier removal).
pub fn remove_outliers(c: &Cloud, k: usize, sigma: f64) -> (Cloud, usize) {
    if c.len() <= k {
        return (c.clone(), 0);
    }
    let tree = KdTree::new(&c.positions);
    let mean_d: Vec<f64> = (0..c.len()).into_par_iter().map(|i| {
        let nn = tree.knn(c.positions[i], k + 1);
        nn.iter().skip(1).map(|e| e.1.sqrt()).sum::<f64>() / k as f64
    }).collect();
    let mu = mean_d.iter().sum::<f64>() / mean_d.len() as f64;
    let sd = (mean_d.iter().map(|d| (d - mu).powi(2)).sum::<f64>() / mean_d.len() as f64).sqrt();
    let limit = mu + sigma * sd;
    let keep: Vec<u32> = (0..c.len() as u32).filter(|&i| mean_d[i as usize] <= limit).collect();
    let dropped = c.len() - keep.len();
    (c.select(&keep), dropped)
}

#[derive(Clone, Copy, Debug)]
pub enum Orient {
    /// Propagated over the neighbour graph, then pointing away from the centre (closed objects).
    Outward,
    /// Toward a viewpoint (a scanner position).
    Toward(V3),
    /// Left as the principal components give them.
    None,
}

/// Normals from each point's `k` nearest neighbours (the direction they vary least).
pub fn estimate_normals(c: &mut Cloud, k: usize, orient: Orient) {
    let tree = KdTree::new(&c.positions);
    let k = k.max(3);
    let pts = &c.positions;
    let (normals, nbrs): (Vec<V3>, Vec<Vec<u32>>) = (0..pts.len())
        .into_par_iter()
        .map(|i| {
            let nn = tree.knn(pts[i], k);
            let local: Vec<V3> = nn.iter().map(|e| pts[e.0 as usize]).collect();
            let (_, cov) = covariance(&local);
            let (_, vecs) = sym_eigen3(cov);
            (vecs[0], nn.iter().map(|e| e.0).filter(|&j| j as usize != i).collect())
        })
        .unzip();
    c.normals = normals;
    match orient {
        Orient::None => {}
        Orient::Toward(view) => {
            for i in 0..c.len() {
                if dot(c.normals[i], sub(view, c.positions[i])) < 0.0 {
                    c.normals[i] = scale(c.normals[i], -1.0);
                }
            }
        }
        Orient::Outward => orient_outward(c, &nbrs),
    }
}

/// Consistent orientation by a maximum spanning tree over the neighbour
/// graph (weights |nᵢ·nⱼ|), seeded at the point farthest from the centre
/// pointing outward; each piece seeded separately.
fn orient_outward(c: &mut Cloud, nbrs: &[Vec<u32>]) {
    use std::collections::BinaryHeap;
    #[derive(PartialEq)]
    struct E(f64, u32, u32);
    impl Eq for E {}
    impl PartialOrd for E {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for E {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
    let n = c.len();
    let centre = scale(c.positions.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / n.max(1) as f64);
    let mut done = vec![false; n];
    let mut by_far: Vec<u32> = (0..n as u32).collect();
    by_far.sort_by(|&a, &b| dist2(c.positions[b as usize], centre).total_cmp(&dist2(c.positions[a as usize], centre)));
    let mut heap = BinaryHeap::new();
    for &seed in &by_far {
        if done[seed as usize] {
            continue;
        }
        let s = seed as usize;
        if dot(c.normals[s], sub(c.positions[s], centre)) < 0.0 {
            c.normals[s] = scale(c.normals[s], -1.0);
        }
        done[s] = true;
        for &j in &nbrs[s] {
            heap.push(E(dot(c.normals[s], c.normals[j as usize]).abs(), seed, j));
        }
        while let Some(E(_, from, to)) = heap.pop() {
            if done[to as usize] {
                continue;
            }
            done[to as usize] = true;
            if dot(c.normals[from as usize], c.normals[to as usize]) < 0.0 {
                c.normals[to as usize] = scale(c.normals[to as usize], -1.0);
            }
            for &j in &nbrs[to as usize] {
                if !done[j as usize] {
                    heap.push(E(dot(c.normals[to as usize], c.normals[j as usize]).abs(), to, j));
                }
            }
        }
    }
}

/// About `count` points spread evenly over a mesh's surface (with the
/// triangles' normals), as FreeCAD's Points_Convert.
pub fn sample_mesh(m: &Mesh, count: usize) -> Cloud {
    let areas: Vec<f64> = (0..m.triangles.len()).map(|t| m.face_area(t)).collect();
    let total: f64 = areas.iter().sum::<f64>().max(1e-300);
    let mut c = Cloud::default();
    let mut carry = 0.0;
    let mut seq = 0u64;
    for t in 0..m.triangles.len() {
        let want = areas[t] / total * count as f64 + carry;
        let k = want.floor() as usize;
        carry = want - k as f64;
        let [a, b, cc] = m.corners(t);
        let nrm = m.face_normal(t);
        for _ in 0..k {
            // A low-discrepancy pair (R2 sequence), folded into the triangle.
            seq += 1;
            let mut u = (0.5 + seq as f64 * 0.754_877_666_246_692_7).fract();
            let mut v = (0.5 + seq as f64 * 0.569_840_290_998_053_3).fract();
            if u + v > 1.0 {
                u = 1.0 - u;
                v = 1.0 - v;
            }
            c.positions.push(add(a, add(scale(sub(b, a), u), scale(sub(cc, a), v))));
            c.normals.push(nrm);
        }
    }
    c
}

/// A cloud's summary, for reports.
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub points: usize,
    pub bounds: [f64; 6],
    pub spacing: f64,
    pub normals: bool,
    pub colors: bool,
}

pub fn summary(c: &Cloud) -> Summary {
    let (lo, hi) = c.bounds();
    Summary { points: c.len(), bounds: [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]], spacing: c.spacing(), normals: !c.normals.is_empty(), colors: !c.colors.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::uv_sphere;

    #[test]
    fn formats_read_back_what_was_written() {
        let mut c = sample_mesh(&uv_sphere(10.0, 20, 40), 500);
        c.colors = (0..c.len()).map(|i| [(i % 256) as u8, 7, 200]).collect();
        let back = read(&write_ply(&c), Format::Ply).unwrap();
        assert_eq!(back.len(), c.len());
        assert!((back.positions[17][0] - c.positions[17][0]).abs() < 1e-5);
        assert_eq!(back.colors[300], c.colors[300]);
        assert!(!back.normals.is_empty());
        let back = read(&write_xyz(&c), Format::Xyz).unwrap();
        assert_eq!(back.len(), c.len());
        assert_eq!(back.normals.len(), c.len(), "six numbers of unit length are normals");
        assert_eq!(Format::sniff(&write_xyz(&c)), Some(Format::Xyz));
        assert_eq!(Format::sniff(b"ply\nformat ascii 1.0"), Some(Format::Ply));
    }

    #[test]
    fn pcd_ascii_binary_and_compressed() {
        let ascii = b"# .PCD v0.7\nVERSION 0.7\nFIELDS x y z rgb\nSIZE 4 4 4 4\nTYPE F F F U\nCOUNT 1 1 1 1\nWIDTH 2\nHEIGHT 1\nVIEWPOINT 0 0 0 1 0 0 0\nPOINTS 2\nDATA ascii\n1 2 3 16711680\n4 5 6 255\n";
        let c = read(ascii, Format::Pcd).unwrap();
        assert_eq!(c.positions, vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
        assert_eq!(c.colors, vec![[255, 0, 0], [0, 0, 255]]);
        let mut bin = b"VERSION 0.7\nFIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nCOUNT 1 1 1\nWIDTH 2\nHEIGHT 1\nPOINTS 2\nDATA binary\n".to_vec();
        for v in [1.0f32, 2.0, 3.0, -1.0, -2.0, -3.5] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        let c = read(&bin, Format::Pcd).unwrap();
        assert_eq!(c.positions[1], [-1.0, -2.0, -3.5]);
        // binary_compressed: fields one after another, LZF-packed (here as literal runs).
        let mut plain = Vec::new();
        for v in [1.0f32, -1.0, 2.0, -2.0, 3.0, -3.5] {
            plain.extend_from_slice(&v.to_le_bytes());
        }
        let mut lzf = vec![(plain.len() - 1) as u8];
        lzf.extend_from_slice(&plain);
        let mut comp = b"VERSION 0.7\nFIELDS x y z\nSIZE 4 4 4\nTYPE F F F\nCOUNT 1 1 1\nWIDTH 2\nHEIGHT 1\nPOINTS 2\nDATA binary_compressed\n".to_vec();
        comp.extend_from_slice(&(lzf.len() as u32).to_le_bytes());
        comp.extend_from_slice(&(plain.len() as u32).to_le_bytes());
        comp.extend_from_slice(&lzf);
        let c = read(&comp, Format::Pcd).unwrap();
        assert_eq!(c.positions, vec![[1.0, 2.0, 3.0], [-1.0, -2.0, -3.5]]);
        // An LZF back-reference repeats earlier bytes.
        assert_eq!(lzf_decompress(&[2, b'a', b'b', b'c', 0x20, 2], 6).unwrap(), b"abcabc");
    }

    #[test]
    fn normals_point_outward_and_thinning_keeps_the_shape() {
        let s = uv_sphere(10.0, 30, 60);
        let mut c = sample_mesh(&s, 4000);
        c.normals.clear();
        estimate_normals(&mut c, 12, Orient::Outward);
        let agree = (0..c.len()).filter(|&i| dot(c.normals[i], unit(c.positions[i])) > 0.95).count();
        assert!(agree as f64 > 0.99 * c.len() as f64, "{agree} of {}", c.len());
        let d = voxel_downsample(&c, 2.0);
        assert!(d.len() < c.len() / 3 && d.len() > 100, "{}", d.len());
        // Outliers: a few far points among the sphere's.
        let mut noisy = c.clone();
        for i in 0..20 {
            noisy.positions.push([30.0 + i as f64, 0.0, 0.0]);
            noisy.normals.push([1.0, 0.0, 0.0]);
        }
        let (clean, dropped) = remove_outliers(&noisy, 8, 2.0);
        assert!(dropped >= 20 && dropped < 80, "{dropped}");
        assert!(clean.positions.iter().all(|p| norm(*p) < 11.0));
    }
}
