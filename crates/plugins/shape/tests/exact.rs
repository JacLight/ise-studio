//! Every example program, evaluated by both kernels: the exact (OpenCascade)
//! result must match the mesh result to within the mesh's faceting.

use de_shape::{exact, runtime};
use std::path::{Path, PathBuf};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples")
}

/// (example, shape name, source, parameter values, display tolerance) for every shape.create step.
fn programs() -> Vec<(String, String, String, Vec<(String, f64)>, f64)> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir(examples()).unwrap().flatten() {
        let steps = dir.path().join("steps.json");
        let Ok(text) = std::fs::read_to_string(&steps) else { continue };
        let steps: serde_json::Value = serde_json::from_str(&text).unwrap();
        for step in steps.as_array().unwrap() {
            if step["operation"] != "shape.create" {
                continue;
            }
            let p = &step["parameters"];
            let files: Vec<String> = match &p["source"]["$file"] {
                serde_json::Value::String(f) => vec![f.clone()],
                serde_json::Value::Array(a) => a.iter().map(|f| f.as_str().unwrap().to_string()).collect(),
                _ => continue,
            };
            let source = files.iter().map(|f| std::fs::read_to_string(dir.path().join(f)).unwrap()).collect::<Vec<_>>().join("\n");
            let params = p["params"].as_array().unwrap().iter().map(|q| (q["key"].as_str().unwrap().to_string(), q["value"].as_f64().unwrap())).collect();
            let tol = p["display_tolerance"].as_f64().unwrap_or(0.01);
            out.push((dir.file_name().to_string_lossy().into_owned(), p["name"].as_str().unwrap().to_string(), source, params, tol));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

#[test]
fn every_example_evaluates_exactly_and_matches_the_mesh() {
    let mut report = Vec::new();
    let mut failures = Vec::new();
    // Examples read their pictures by content hash: store each example's pictures.
    de_shape::assets::set_dir(std::env::temp_dir().join(format!("ise-example-assets-{}", std::process::id())));
    for dir in std::fs::read_dir(examples()).unwrap().flatten() {
        for f in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let path = f.path();
            if matches!(path.extension().and_then(|e| e.to_str()), Some("png" | "jpg")) {
                de_shape::assets::store(&std::fs::read(&path).unwrap()).unwrap();
            }
        }
    }
    for (example, name, source, params, tol) in programs() {
        let t0 = std::time::Instant::now();
        let mesh = runtime::evaluate(&source, &params, tol).unwrap();
        let t1 = std::time::Instant::now();
        match exact::evaluate_exact(&name, &source, &params, tol) {
            Ok(e) => {
                let ms = t1.elapsed().as_millis();
                let volume: f64 = e.parts.iter().map(|p| e.arena.volume(p.shape)).sum();
                let diff = (volume - mesh.volume).abs() / mesh.volume;
                let parts = e.parts.len();
                report.push(format!(
                    "{example:<12} {name:<34} {parts:>4} parts  exact {:>14.1} mm³  mesh {:>14.1}  Δ {:.4}%  {ms:>6} ms (mesh {} ms){}",
                    volume,
                    mesh.volume,
                    diff * 100.0,
                    (t1 - t0).as_millis(),
                    if e.faceted.is_empty() { String::new() } else { format!("  faceted: {}", e.faceted.join(", ")) }
                ));
                // Facets cut a little off every curve; exact geometry is the truth, within 1 %.
                if diff > 0.01 {
                    failures.push(format!("{example} {name}: exact {volume} vs mesh {}", mesh.volume));
                }
            }
            Err(err) => failures.push(format!("{example} {name}: {err}")),
        }
    }
    println!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

