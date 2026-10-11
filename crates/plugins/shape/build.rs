//! Embeds the stock library (`library/stock/*.js`, `*.json` at the repo root)
//! in the engine, so a built engine carries its catalogue with it.

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../library/stock");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("js" | "json"))).collect())
        .unwrap_or_default();
    files.sort();
    let mut out = String::from("/// Every file of the stock library: (file name, contents).\npub static FILES: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let name = f.file_name().unwrap().to_string_lossy();
        let path = f.canonicalize().unwrap();
        out.push_str(&format!("    ({name:?}, include_str!({:?})),\n", path.display().to_string()));
    }
    out.push_str("];\n");
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("stock_files.rs"), out).unwrap();
}
