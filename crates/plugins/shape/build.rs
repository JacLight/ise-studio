//! Embeds the stock library (`library/stock/*.js`, `*.json` at the repo root)
//! and the made parts programs `use()` (`library/parts/*.js`) in the engine,
//! so a built engine carries them with it.

use std::path::PathBuf;

/// Writes `OUT_DIR/<out>`: a static `FILES` of (file name, contents) for every
/// file in `library/<folder>` with one of `extensions`.
fn embed(folder: &str, extensions: &[&str], out: &str, what: &str) {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../library").join(folder);
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|x| x.to_str()).is_some_and(|x| extensions.contains(&x))).collect())
        .unwrap_or_default();
    files.sort();
    let mut text = format!("/// Every file of {what}: (file name, contents).\npub static FILES: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let name = f.file_name().unwrap().to_string_lossy();
        let path = f.canonicalize().unwrap();
        text.push_str(&format!("    ({name:?}, include_str!({:?})),\n", path.display().to_string()));
    }
    text.push_str("];\n");
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(out), text).unwrap();
}

fn main() {
    embed("stock", &["js", "json"], "stock_files.rs", "the stock library");
    embed("parts", &["js"], "parts_files.rs", "the made parts library");
}
