//! Files a program reads (imported STEP files, images), stored by content: a
//! program refers to a file by its SHA-256, so the same bytes always give the
//! same part and a project never depends on where a file once was.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where stored files live (set once by the server at startup).
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// The kinds of file kept, by extension.
const KINDS: [&str; 5] = ["step", "png", "jpg", "ttf", "otf"];

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
    KINDS
        .iter()
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
    let p = dir.join(format!("{sha}.{}", sniff(bytes)));
    if !p.is_file() {
        std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&p, bytes)).map_err(|e| format!("could not store the file: {e}"))?;
    }
    Ok(p)
}

/// The folder stored files live in, if set.
pub fn dir() -> Option<PathBuf> {
    DIR.get().cloned()
}

<<<<<<< HEAD
/// Every imported file a program reads (`importStep("<sha>")`,
/// `importIfc("<sha>")`; an IFC file is a STEP physical file too, stored the same way).
pub fn referenced(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for call in ["importStep(", "importIfc("] {
        let mut rest = source;
        while let Some(i) = rest.find(call) {
            rest = &rest[i + call.len()..];
            let r = rest.trim_start().trim_start_matches(['"', '\'', '`']);
            let sha: String = r.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            if sha.len() == 64 && !out.contains(&sha) {
                out.push(sha);
=======
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
>>>>>>> main
            }
        }
        i += 1;
    }
    out
}

/// An imported IFC file read into elements, levels, spaces and the rest, as
/// JSON for `importIfc` (cached: reading a file is the slow part).
pub fn ifc_json(sha: &str) -> Result<std::sync::Arc<String>, String> {
    static CACHE: OnceLock<Mutex<Vec<(String, std::sync::Arc<String>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, j)) = cache.lock().unwrap().iter().find(|(s, _)| s == sha) {
        return Ok(j.clone());
    }
    let bytes = std::fs::read(path(sha)?).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    let read = de_bim::ifc_read::read(&text)?;
    let json = std::sync::Arc::new(serde_json::to_string(&read).map_err(|e| e.to_string())?);
    let mut c = cache.lock().unwrap();
    if c.len() >= 4 {
        c.remove(0);
    }
    c.push((sha.to_string(), json.clone()));
    Ok(json)
}

/// A STEP file's bodies as meshes within `tolerance` mm of the exact surfaces,
/// cached (reading and meshing a file is the slow part).
pub fn meshes(sha: &str, tolerance: f64) -> Result<Vec<(String, manifold_csg::Manifold)>, String> {
    type Entry = (String, u64, Vec<(String, manifold_csg::Manifold)>);
    static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, _, m)) = cache.lock().unwrap().iter().find(|(s, t, _)| s == sha && *t == tolerance.to_bits()) {
        return Ok(m.iter().map(|(n, s)| (n.clone(), s.translate(0.0, 0.0, 0.0))).collect());
    }
    let mut arena = de_occt::Arena::new();
    let bodies = arena.read_step(&path(sha)?)?;
    let mut out = Vec::new();
    for (name, shape) in bodies {
        let m = arena.mesh(shape, tolerance)?;
        let (pos, tri) = crate::exact::weld_mesh(&m.positions, &m.triangles, tolerance * 1e-3);
        let tris: Vec<u64> = tri.iter().map(|t| *t as u64).collect();
        let solid = manifold_csg::Manifold::from_mesh_f64(&pos, 3, &tris).map_err(|e| format!("{name}: not a closed solid ({e})"))?;
        out.push((name, solid));
    }
    let mut c = cache.lock().unwrap();
    if c.len() >= 8 {
        c.remove(0);
    }
    c.push((sha.to_string(), tolerance.to_bits(), out.iter().map(|(n, s)| (n.clone(), s.translate(0.0, 0.0, 0.0))).collect()));
    Ok(out)
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
