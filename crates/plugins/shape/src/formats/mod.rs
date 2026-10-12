//! File formats in and out, beyond STEP (which OpenCascade handles, as it
//! does IGES): mesh files (STL, OBJ, PLY, glTF, 3MF) read into parts and
//! written from them, and 2D drawings (DXF, SVG) read into sketches.
//!
//! Every reader here is plain Rust over the bytes, so an imported file is
//! read the same on every computer, and nothing is pulled in to do it.

pub mod dxf;
pub mod gltf;
pub mod looks;
pub mod mesh;
pub mod outline;
pub mod svg;
pub mod three_mf;
pub mod xml;
pub mod zip;

use manifold_csg::Manifold;

/// The kinds of file Forma reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Step,
    Iges,
    Stl,
    Obj,
    Ply,
    Gltf,
    Glb,
    ThreeMf,
    Dxf,
    Svg,
}

/// What an import becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Becomes {
    /// Exact solids (STEP, IGES).
    Exact,
    /// Faceted solids (mesh files).
    Mesh,
    /// A sketch (2D drawings).
    Sketch,
}

impl Kind {
    pub const ALL: [Kind; 10] = [Kind::Step, Kind::Iges, Kind::Stl, Kind::Obj, Kind::Ply, Kind::Gltf, Kind::Glb, Kind::ThreeMf, Kind::Dxf, Kind::Svg];

    /// The extension a stored file gets.
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Step => "step",
            Kind::Iges => "iges",
            Kind::Stl => "stl",
            Kind::Obj => "obj",
            Kind::Ply => "ply",
            Kind::Gltf => "gltf",
            Kind::Glb => "glb",
            Kind::ThreeMf => "3mf",
            Kind::Dxf => "dxf",
            Kind::Svg => "svg",
        }
    }

    /// From a file name's extension.
    pub fn from_name(name: &str) -> Option<Kind> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        Some(match ext.as_str() {
            "step" | "stp" => Kind::Step,
            "iges" | "igs" => Kind::Iges,
            "stl" => Kind::Stl,
            "obj" => Kind::Obj,
            "ply" => Kind::Ply,
            "gltf" => Kind::Gltf,
            "glb" => Kind::Glb,
            "3mf" => Kind::ThreeMf,
            "dxf" => Kind::Dxf,
            "svg" => Kind::Svg,
            _ => return None,
        })
    }

    pub fn becomes(self) -> Becomes {
        match self {
            Kind::Step | Kind::Iges => Becomes::Exact,
            Kind::Dxf | Kind::Svg => Becomes::Sketch,
            _ => Becomes::Mesh,
        }
    }

    /// The format of a file from its first bytes, where they say.
    pub fn sniff(data: &[u8]) -> Option<Kind> {
        let head = &data[..data.len().min(4096)];
        let text = String::from_utf8_lossy(head);
        let t = text.trim_start_matches('\u{feff}').trim_start();
        if head.starts_with(b"glTF") {
            return Some(Kind::Glb);
        }
        if head.starts_with(b"PK\x03\x04") {
            return Some(Kind::ThreeMf);
        }
        if t.starts_with("ISO-10303-21") {
            return Some(Kind::Step);
        }
        if t.starts_with("ply") {
            return Some(Kind::Ply);
        }
        // IGES: fixed 80-column lines, the start section marked S in column 73.
        if text.lines().next().is_some_and(|l| l.len() >= 73 && l.as_bytes()[72] == b'S') {
            return Some(Kind::Iges);
        }
        if data.len() >= 84 && 84 + 50 * u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize == data.len() {
            return Some(Kind::Stl);
        }
        if t.starts_with("solid") && text.contains("facet") {
            return Some(Kind::Stl);
        }
        if t.starts_with('{') && text.contains("\"asset\"") {
            return Some(Kind::Gltf);
        }
        if text.contains("<svg") {
            return Some(Kind::Svg);
        }
        let mut lines = t.lines().map(str::trim);
        if lines.next() == Some("0") && lines.next().is_some_and(|l| l == "SECTION") {
            return Some(Kind::Dxf);
        }
        if data.starts_with(b"AutoCAD Binary DXF") {
            return Some(Kind::Dxf);
        }
        if t.lines().any(|l| l.starts_with("v ")) {
            return Some(Kind::Obj);
        }
        None
    }
}

/// Why a DWG cannot come in (the format is closed; its only open reader is GPL).
pub const DWG: &str = "DWG files cannot be read here: the format is closed, and its only open-source reader (LibreDWG) is GPL, which Forma cannot include. Save the drawing as DXF (every CAD program can) and import that";

/// How a part looks in a file that carries colour (sRGB, 0…1).
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub color: [f64; 3],
    pub metalness: f64,
    pub roughness: f64,
    pub opacity: f64,
}

impl Look {
    /// A part with no material: the view's unassigned grey.
    pub const DEFAULT: Look = Look { color: [0xd6 as f64 / 255.0, 0xd4 as f64 / 255.0, 0xc7 as f64 / 255.0], metalness: 0.1, roughness: 0.65, opacity: 1.0 };

    /// Just a colour, plastic-like.
    pub fn color(color: [f64; 3]) -> Look {
        Look { color, metalness: 0.0, roughness: 0.6, opacity: 1.0 }
    }

    /// "#rrggbb".
    pub fn hex(&self) -> String {
        let c = self.color.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    }
}

/// A named triangle mesh, millimetres, Z up.
#[derive(Clone, Debug, Default)]
pub struct MeshBody {
    pub name: String,
    pub positions: Vec<f64>,
    pub triangles: Vec<u32>,
    pub look: Option<Look>,
    /// Per-vertex normals (empty: none), for formats that render with them.
    pub normals: Vec<f64>,
}

impl MeshBody {
    /// Separate triangles (three new vertices each), as STL stores them.
    pub fn soup(name: String, positions: Vec<f64>) -> MeshBody {
        let n = (positions.len() / 9 * 3) as u32;
        MeshBody { name, positions, triangles: (0..n).collect(), look: None, normals: Vec::new() }
    }

    /// A solid's mesh; with `normals`, smooth across curved faces and sharp at edges.
    pub fn of_solid(name: &str, solid: &Manifold, look: Option<Look>, normals: bool) -> MeshBody {
        let (props, n, idx) = if normals { solid.calculate_normals(3, 40.0).to_mesh_f64_with_normals(3) } else { solid.to_mesh_f64() };
        let n = n.max(3);
        let mut positions = Vec::with_capacity(props.len() / n * 3);
        let mut ns = Vec::new();
        for v in props.chunks_exact(n) {
            positions.extend_from_slice(&v[..3]);
            if normals && n >= 6 {
                ns.extend_from_slice(&v[3..6]);
            }
        }
        MeshBody { name: name.to_string(), positions, triangles: idx.into_iter().map(|i| i as u32).collect(), look, normals: ns }
    }

    /// Closed and consistently wound: a solid. Ends that touch are joined
    /// first (mesh files often repeat a vertex for every triangle).
    pub fn to_solid(&self) -> Result<Manifold, String> {
        if self.triangles.is_empty() {
            return Err("no triangles".into());
        }
        let size = self.positions.chunks_exact(3).fold([f64::INFINITY, f64::NEG_INFINITY], |m, v| [m[0].min(v[0]).min(v[1]).min(v[2]), m[1].max(v[0]).max(v[1]).max(v[2])]);
        let eps = ((size[1] - size[0]) * 1e-7).max(1e-9);
        let (pos, tri) = crate::exact::weld_mesh(&self.positions, &self.triangles, eps);
        // Triangles that welding collapsed carry nothing.
        let mut tri: Vec<u64> = tri.chunks_exact(3).filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]).flatten().map(|&t| t as u64).collect();
        let make = |tri: &[u64]| Manifold::from_mesh_f64(&pos, 3, tri).map_err(|e| format!("not a closed solid ({e}): it has holes or loose edges; repair it (e.g. in your slicer or Meshmixer) and import it again"));
        let solid = make(&tri)?;
        if solid.volume() >= 0.0 {
            return Ok(solid);
        }
        // Inside out (wound clockwise): turned the right way.
        for t in tri.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
        make(&tri)
    }
}

/// A mesh file's bodies, millimetres, Z up, each named (the file's names,
/// else the file's own).
pub fn read_mesh(data: &[u8], kind: Kind) -> Result<Vec<MeshBody>, String> {
    let bodies = match kind {
        Kind::Stl => mesh::read_stl(data)?,
        Kind::Obj => mesh::read_obj(data)?,
        Kind::Ply => mesh::read_ply(data)?,
        Kind::Gltf | Kind::Glb => gltf::read(data)?,
        Kind::ThreeMf => three_mf::read(data)?,
        other => return Err(format!("a .{} file is not a mesh", other.ext())),
    };
    let bodies: Vec<MeshBody> = bodies.into_iter().filter(|b| !b.triangles.is_empty()).collect();
    if bodies.is_empty() {
        return Err("the file has no triangles".into());
    }
    Ok(bodies)
}

/// A 2D drawing as sketch geometry (moved to start at the origin), what it
/// is made of, and its size (mm).
pub fn read_sketch(data: &[u8], kind: Kind) -> Result<(crate::sketch::Geometry, outline::Counts, [f64; 2]), String> {
    match kind {
        Kind::Dxf => dxf::read(data)?.finish(),
        Kind::Svg => svg::read(data)?.finish(),
        other => Err(format!("a .{} file is not a 2D drawing", other.ext())),
    }
}

#[cfg(test)]
mod tests;
