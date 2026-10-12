//! Files a program reads, stored by content: imported STEP, IGES, mesh
//! files and drawings, and images and fonts. A program refers to a file by
//! its SHA-256, so the same bytes always give the same part and a project
//! never depends on where a file once was. Each is kept as `<sha>.<format>`
//! (the format read from its bytes).

use crate::formats::{Kind, Look};
use manifold_csg::Manifold;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where stored files live (set once by the server at startup).
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// Images and fonts kept besides the imported formats (`Kind`), by extension.
const PICTURES: [&str; 4] = ["png", "jpg", "ttf", "otf"];

/// What these bytes are, by how they start: "png", "jpg", "ttf", "otf", or
/// "step" for anything else (STEP files are text).
pub fn sniff(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg"
    } else if bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true") {
        "ttf"
    } else if bytes.starts_with(b"OTTO") {
        "otf"
    } else {
        "step"
    }
}

/// The content hash of some bytes, as stored files are named.
pub fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn valid(sha: &str) -> Result<(), String> {
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("“{sha}” is not a file hash"));
    }
    Ok(())
}

/// The stored file for a content hash, if it exists.
pub fn path(sha: &str) -> Result<PathBuf, String> {
    valid(sha)?;
    let dir = DIR.get().ok_or("stored files are not available here")?;
    Kind::ALL
        .iter()
        .map(|k| k.ext())
        .chain(PICTURES)
        .map(|k| dir.join(format!("{sha}.{k}")))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("the file {} is not on this computer yet: it comes from a teammate who has it, as soon as one is online", &sha[..12]))
}

/// Keeps these bytes (once) and returns their hash.
pub fn store(bytes: &[u8]) -> Result<String, String> {
    let sha = sha256(bytes);
    store_as(&sha, bytes)?;
    Ok(sha)
}

/// Keeps bytes that arrived under a hash, if they really have it.
pub fn store_as(sha: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    valid(sha)?;
    if sha256(bytes) != sha {
        return Err("the bytes do not match their hash".into());
    }
    let dir = DIR.get().ok_or("stored files are not available here")?;
    // A picture or font by its signature, else the imported format its bytes show.
    let ext = match sniff(bytes) {
        "step" => Kind::sniff(bytes).map(Kind::ext).unwrap_or("step"),
        picture => picture,
    };
    let p = dir.join(format!("{sha}.{ext}"));
    if !p.is_file() {
        std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&p, bytes)).map_err(|e| format!("could not store the file: {e}"))?;
    }
    Ok(p)
}

/// The folder stored files live in, if set.
pub fn dir() -> Option<PathBuf> {
    DIR.get().cloned()
}

/// The format of a stored file.
pub fn kind(sha: &str) -> Result<Kind, String> {
    let p = path(sha)?;
    p.extension().and_then(|e| e.to_str()).and_then(|e| Kind::from_name(&format!(".{e}"))).ok_or_else(|| format!("{} is not a file Forma reads", p.display()))
}

/// Keeps a file by its hash (once), named by its format: what its bytes say,
/// else `hint` (the name it came with). Returns where it is.
pub fn store_import(sha: &str, bytes: &[u8], hint: Option<Kind>) -> Result<PathBuf, String> {
    valid(sha)?;
    if let Ok(p) = path(sha) {
        return Ok(p);
    }
    let dir = DIR.get().ok_or("imported files are not available here")?;
    let kind = Kind::sniff(bytes).or(hint).ok_or("not a file Forma reads")?;
    let p = dir.join(format!("{sha}.{}", kind.ext()));
    std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&p, bytes)).map_err(|e| format!("could not store the file: {e}"))?;
    Ok(p)
}

/// Every stored file a program reads: each quoted content hash in its source
/// (`importStep("<sha>")`, `image("<sha>")`, `relief("<sha>", …)`).
pub fn referenced(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b'"' | b'\'' | b'`') {
            let run = bytes[i + 1..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
            let close = bytes.get(i + 1 + run).copied();
            if run == 64 && close == Some(bytes[i]) {
                let sha = source[i + 1..i + 65].to_ascii_lowercase();
                if !out.contains(&sha) {
                    out.push(sha);
                }
                i += 66;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A body read from an imported file: name, solid, colour (sRGB) if the file gives one.
pub type Body = (String, Manifold, Option<[f64; 3]>);

fn copy(list: &[Body]) -> Vec<Body> {
    list.iter().map(|(n, s, c)| (n.clone(), s.translate(0.0, 0.0, 0.0), *c)).collect()
}

/// Recently read files, by hash and accuracy: reading and meshing is the slow part.
fn cached(sha: &str, tolerance: f64, read: impl FnOnce() -> Result<Vec<Body>, String>) -> Result<Vec<Body>, String> {
    type Entry = (String, u64, Vec<Body>);
    static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, _, m)) = cache.lock().unwrap().iter().find(|(s, t, _)| s == sha && *t == tolerance.to_bits()) {
        return Ok(copy(m));
    }
    let out = read()?;
    let mut c = cache.lock().unwrap();
    if c.len() >= 8 {
        c.remove(0);
    }
    c.push((sha.to_string(), tolerance.to_bits(), copy(&out)));
    Ok(out)
}

/// The exact format of a stored STEP or IGES file.
pub fn cad_format(sha: &str) -> Result<de_occt::CadFormat, String> {
    match kind(sha)? {
        Kind::Step => Ok(de_occt::CadFormat::Step),
        Kind::Iges => Ok(de_occt::CadFormat::Iges),
        k => Err(format!("the imported file is a .{} file, not STEP or IGES (read it with importMesh)", k.ext())),
    }
}

/// A STEP or IGES file's bodies as meshes within `tolerance` mm of the exact surfaces.
pub fn meshes(sha: &str, tolerance: f64) -> Result<Vec<Body>, String> {
    cached(sha, tolerance, || {
        let mut arena = de_occt::Arena::new();
        let bodies = arena.read_cad(&path(sha)?, cad_format(sha)?)?;
        let mut out = Vec::new();
        for b in bodies {
            let m = arena.mesh(b.shape, tolerance)?;
            let (pos, tri) = crate::exact::weld_mesh(&m.positions, &m.triangles, tolerance * 1e-3);
            let tris: Vec<u64> = tri.iter().map(|t| *t as u64).collect();
            let solid = Manifold::from_mesh_f64(&pos, 3, &tris).map_err(|e| format!("{}: not a closed solid ({e})", b.name))?;
            out.push((b.name, solid, b.color));
        }
        Ok(out)
    })
}

/// A mesh file's bodies (STL, OBJ, PLY, glTF, 3MF) as solids, named as in
/// the file (else “Body n”).
pub fn mesh_bodies(sha: &str) -> Result<Vec<Body>, String> {
    cached(sha, 0.0, || {
        let kind = kind(sha)?;
        let bytes = std::fs::read(path(sha)?).map_err(|e| e.to_string())?;
        let bodies = crate::formats::read_mesh(&bytes, kind)?;
        let mut out = Vec::new();
        for (i, b) in bodies.iter().enumerate() {
            let name = if b.name.trim().is_empty() { format!("Body {}", i + 1) } else { b.name.trim().to_string() };
            let solid = b.to_solid().map_err(|e| format!("{name}: {e}"))?;
            out.push((name, solid, b.look.as_ref().map(|l: &Look| l.color)));
        }
        Ok(out)
    })
}

/// "#rrggbb" for a colour, as the view reads it in a material name.
pub fn hex(c: [f64; 3]) -> String {
    Look::color(c).hex()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_quoted_hash_is_a_reference() {
        let a = "a".repeat(64);
        let b = "0123456789abcdef".repeat(4);
        let src = format!("const m = importStep(\"{a}\");\nconst r = relief('{b}', {{ width: 80 }});\nimage(`{a}`); // \"{}\" is too short", "f".repeat(63));
        assert_eq!(referenced(&src), vec![a, b]);
    }

    #[test]
    fn files_are_told_apart_by_how_they_start() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), "png");
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), "jpg");
        assert_eq!(sniff(b"ISO-10303-21;"), "step");
    }
}
