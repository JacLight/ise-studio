//! Imported files (STEP), stored by content: a program refers to a file by
//! its SHA-256, so the same bytes always give the same part and a project
//! never depends on where a file once was.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where imported files live (set once by the server at startup).
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// The stored file for a content hash, if it exists.
pub fn path(sha: &str) -> Result<PathBuf, String> {
    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("“{sha}” is not a file hash"));
    }
    let dir = DIR.get().ok_or("imported files are not available here")?;
    let p = dir.join(format!("{sha}.step"));
    if p.is_file() { Ok(p) } else { Err(format!("the imported file {} is not on this computer yet: it comes from a teammate who has it, as soon as one is online", &sha[..12])) }
}

/// The folder imported files live in, if set.
pub fn dir() -> Option<PathBuf> {
    DIR.get().cloned()
}

/// Every imported file a program reads (`importStep("<sha>")`).
pub fn referenced(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(i) = rest.find("importStep(") {
        rest = &rest[i + "importStep(".len()..];
        let r = rest.trim_start().trim_start_matches(['"', '\'', '`']);
        let sha: String = r.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if sha.len() == 64 && !out.contains(&sha) {
            out.push(sha);
        }
    }
    out
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
