//! Results too slow to work out again each time a project opens (an
//! assembly swept through a joint's range: thousands of closest-approach
//! measurements), kept on disk by a hash of everything they were worked out
//! from. Opening a project unchanged since it was last open reads them back
//! instead of measuring again; any change to what they depend on is a new
//! key, so a stale result is never read.
//!
//! The server sets the folder (`<projects>/.cache`); without one (tests,
//! tools) nothing is kept.

use serde::{Serialize, de::DeserializeOwned};
use std::path::PathBuf;
use std::sync::OnceLock;

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Bumped when a cached result's meaning or format changes.
const VERSION: u32 = 1;
/// The folder is trimmed (oldest first) past this many bytes.
const LIMIT_BYTES: u64 = 256 << 20;

/// Where results are kept (set once by the server at startup).
pub fn set_dir(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = DIR.set(dir);
}

/// This build of the engine: a result worked out by another build (whose
/// measuring may differ) is not read back.
fn build() -> u64 {
    static BUILD: OnceLock<u64> = OnceLock::new();
    *BUILD.get_or_init(|| {
        let stamp = std::env::current_exe().and_then(std::fs::metadata).and_then(|m| m.modified()).ok();
        let secs = stamp.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        secs ^ ((VERSION as u64) << 56)
    })
}

fn file(kind: &str, key: u64) -> Option<PathBuf> {
    Some(DIR.get()?.join(format!("{kind}-{:x}-{key:016x}.json", build())))
}

/// A result kept under this key, if there is one.
pub fn load<T: DeserializeOwned>(kind: &str, key: u64) -> Option<T> {
    let bytes = std::fs::read(file(kind, key)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Keeps a result under this key (best effort: a full disk only costs speed).
pub fn store<T: Serialize>(kind: &str, key: u64, value: &T) {
    let Some(path) = file(kind, key) else { return };
    let Ok(bytes) = serde_json::to_vec(value) else { return };
    // Written aside and renamed, so a reader never sees half a file.
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
        trim();
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Drops the oldest results once the folder is past its limit.
fn trim() {
    let Some(dir) = DIR.get() else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= LIMIT_BYTES {
        return;
    }
    files.sort();
    for (_, len, path) in files {
        if total <= LIMIT_BYTES / 2 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// A key for a program as evaluated (source, parameter values, inputs).
pub fn program_key(program: &crate::ShapeProgram, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    program.source.hash(h);
    for p in &program.params {
        p.key.hash(h);
        p.value.to_bits().hash(h);
    }
    program.inputs.hash(h);
    program.display_tolerance().to_bits().hash(h);
    // Finishes (fillets, shells…) change the geometry too.
    serde_json::to_string(&program.finishes).unwrap_or_default().hash(h);
}
