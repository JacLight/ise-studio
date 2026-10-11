//! The stock library: every family evaluates in every size, by the mesh
//! kernel and exactly (STEP written), with a part number that follows the
//! size, a modelled mass within 10 % of the catalogue's, and the key
//! dimensions each family's manifest asserts (`checks`).
//!
//! `FORMA_STOCK_FAMILY=iso4762,nema17` limits the run to some families.

use de_shape::library::{self, Family};
use de_shape::{exact, runtime};
use rayon::prelude::*;
use serde_json::Value;

fn families() -> Vec<&'static Family> {
    let cat = library::catalogue().expect("the catalogue loads");
    let only: Option<Vec<String>> = std::env::var("FORMA_STOCK_FAMILY").ok().map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
    cat.families.iter().filter(|f| only.as_ref().is_none_or(|o| o.contains(&f.id))).collect()
}

fn density(f: &Family, material: Option<&str>) -> Option<f64> {
    material.or(f.material.as_deref()).and_then(|m| f.materials.get(m)).copied()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn the_catalogue_lists_every_family_it_promises() {
    let cat = library::catalogue().expect("the catalogue loads");
    let index: Value = serde_json::from_str(library::files().iter().find(|(n, _)| *n == "catalogue.json").unwrap().1).unwrap();
    for id in index["order"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        assert!(cat.family(id).is_some(), "catalogue.json lists {id}, but library/stock has no {id}.json");
    }
    for f in &cat.families {
        assert!(cat.categories.contains(&f.category));
        assert!(!f.rows.is_empty(), "{} has sizes", f.id);
    }
}

#[test]
fn every_size_evaluates_with_its_part_number_mass_and_dimensions() {
    let mut failures = Vec::new();
    let mut report = Vec::new();
    for f in families() {
        let source = f.program_source();
        let tol = f.display_tolerance.unwrap_or(runtime::DISPLAY_TOLERANCE);
        let results: Vec<Result<(String, String, f64, f64), String>> = f
            .rows
            .par_iter()
            .map(|row| {
                let tag = format!("{} {}", f.id, row.name);
                let e = runtime::evaluate_in(&source, &f.values_for(row), tol, &f.inputs_for(row)).map_err(|e| format!("{tag}: {e}"))?;
                let stock = e.declared.stock.clone().ok_or_else(|| format!("{tag}: declares no stock()"))?;
                // Modelled mass from the true geometry and the manifest's densities.
                let mass_g = if e.parts.is_empty() {
                    e.volume * density(f, None).ok_or_else(|| format!("{tag}: no material"))? / 1e6
                } else {
                    let mut m = 0.0;
                    for p in &e.parts {
                        m += p.volume * density(f, p.material.as_deref()).ok_or_else(|| format!("{tag}: part {} has no material density", p.name))? / 1e6;
                    }
                    m
                };
                let catalogue = stock.mass_g.ok_or_else(|| format!("{tag}: stock() gives no massG"))?;
                if (mass_g - catalogue).abs() > 0.10 * catalogue {
                    return Err(format!("{tag}: modelled mass {mass_g:.3} g vs catalogue {catalogue:.3} g ({:+.1} %)", (mass_g / catalogue - 1.0) * 100.0));
                }
                if stock.unit_price.is_none() {
                    return Err(format!("{tag}: no typical unit price"));
                }
                if stock.sources.is_empty() {
                    return Err(format!("{tag}: no source to buy it from"));
                }
                if e.declared.connectors.is_empty() {
                    return Err(format!("{tag}: declares no connectors"));
                }
                if let Some(c) = f.checks.get(&row.name) {
                    let size = [0, 1, 2].map(|k| e.bounds_max[k] - e.bounds_min[k]);
                    if let Some(want) = c["size"].as_array() {
                        let t = c["tolerance"].as_f64().unwrap_or(0.05);
                        for k in 0..3 {
                            let w = want[k].as_f64().unwrap();
                            if !close(size[k], w, t) {
                                return Err(format!("{tag}: overall size {size:?}, expected {want:?} (±{t})"));
                            }
                        }
                    }
                    if let Some(names) = c["connectors"].as_array() {
                        for n in names {
                            let n = n.as_str().unwrap();
                            if !e.declared.connectors.iter().any(|x| x.name == n) {
                                return Err(format!("{tag}: no connector {n}"));
                            }
                        }
                    }
                    if let Some(at) = c["connector_at"].as_object() {
                        for (n, want) in at {
                            let x = e.declared.connectors.iter().find(|x| &x.name == n).ok_or_else(|| format!("{tag}: no connector {n}"))?;
                            for k in 0..3 {
                                if !close(x.at[k], want[k].as_f64().unwrap(), 0.01) {
                                    return Err(format!("{tag}: connector {n} at {:?}, expected {want}", x.at));
                                }
                            }
                        }
                    }
                    if let Some(ms) = c["metrics"].as_object() {
                        for (n, want) in ms {
                            let got = e.metrics.get(n).map(|m| m.value).ok_or_else(|| format!("{tag}: no metric {n}"))?;
                            if !close(got, want.as_f64().unwrap(), 1e-6) {
                                return Err(format!("{tag}: metric {n} = {got}, expected {want}"));
                            }
                        }
                    }
                    if let Some(pn) = c["partNumber"].as_str()
                        && pn != stock.part_number
                    {
                        return Err(format!("{tag}: part number {}, expected {pn}", stock.part_number));
                    }
                    if let Some(want) = c["massG"].as_f64()
                        && (catalogue - want).abs() > 0.005 * want
                    {
                        return Err(format!("{tag}: catalogue mass {catalogue} g, expected {want} g"));
                    }
                }
                Ok((row.name.clone(), stock.part_number, mass_g, catalogue))
            })
            .collect();
        let mut numbers = std::collections::HashMap::new();
        for r in results {
            match r {
                Ok((size, pn, m, c)) => {
                    report.push(format!("{:<22} {:<12} {:<40} {:>10.3} g  catalogue {:>10.3} g  {:+.1} %", f.id, size, pn, m, c, (m / c - 1.0) * 100.0));
                    if let Some(other) = numbers.insert(pn.clone(), size.clone()) {
                        failures.push(format!("{}: sizes {other} and {size} share the part number {pn}", f.id));
                    }
                }
                Err(e) => failures.push(e),
            }
        }
    }
    println!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// OpenCascade's STEP writer keeps global state: one file at a time.
static STEP_WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn every_size_evaluates_exactly_and_writes_step() {
    let dir = std::env::temp_dir().join(format!("forma-stock-step-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    let mut faceted = Vec::new();
    for f in families() {
        let source = f.program_source();
        let tol = f.display_tolerance.unwrap_or(runtime::DISPLAY_TOLERANCE);
        let results: Vec<Result<Vec<String>, String>> = f
            .rows
            .par_iter()
            .map(|row| {
                let tag = format!("{} {}", f.id, row.name);
                let values = f.values_for(row);
                let inputs = f.inputs_for(row);
                let mesh = runtime::evaluate_in(&source, &values, tol, &inputs).map_err(|e| format!("{tag}: mesh: {e}"))?;
                let (mut arena, e) = exact::evaluate_exact_in(de_occt::Arena::new(), &f.title(), &source, &values, tol, &inputs).map_err(|(_, e)| format!("{tag}: exact: {e}"))?;
                let volume: f64 = e.parts.iter().map(|p| arena.volume(p.shape)).sum();
                let diff = (volume - mesh.volume).abs() / mesh.volume;
                // Facets cut a little off every curve (a few tenths of a percent on an M2
                // thread at 0.01 mm); exact geometry is the truth, within 2 %.
                if diff > 0.02 {
                    return Err(format!("{tag}: exact volume {volume:.3} vs mesh {:.3} ({:.2} %)", mesh.volume, diff * 100.0));
                }
                let path = dir.join(format!("{}-{}.step", f.id, row.name.replace(['/', ' '], "_")));
                let parts: Vec<(String, de_occt::Shape)> = e.parts.iter().map(|p| (p.name.clone(), p.shape)).collect();
                let _guard = STEP_WRITER.lock().unwrap_or_else(|e| e.into_inner());
                arena.write_step(&parts, &path).map_err(|e| format!("{tag}: STEP: {e}"))?;
                let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                if len < 1000 {
                    return Err(format!("{tag}: STEP file has {len} bytes"));
                }
                let _ = std::fs::remove_file(&path);
                Ok(e.faceted.iter().map(|x| format!("{tag}: {x}")).collect())
            })
            .collect();
        for r in results {
            match r {
                Ok(fc) => faceted.extend(fc),
                Err(e) => failures.push(e),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if !faceted.is_empty() {
        println!("faceted in STEP:\n{}", faceted.join("\n"));
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn true_helical_threads_evaluate_when_asked_for() {
    let cat = library::catalogue().unwrap();
    for (id, size) in [("iso4762", "M3×12"), ("iso4032", "M3")] {
        let f = cat.family(id).unwrap();
        let row = f.row(size).unwrap();
        let rings: Vec<(String, f64)> = f.values_for(row);
        let helix: Vec<(String, f64)> = rings.iter().map(|(k, v)| (k.clone(), if k == "threads" { 1.0 } else { *v })).collect();
        let tol = f.display_tolerance.unwrap_or(0.01);
        let a = runtime::evaluate_in(&f.program_source(), &rings, tol, &f.inputs_for(row)).unwrap();
        let b = runtime::evaluate_in(&f.program_source(), &helix, tol, &f.inputs_for(row)).unwrap();
        // The helix weighs what the rings weigh, within a few percent.
        assert!((b.volume / a.volume - 1.0).abs() < 0.04, "{id} {size}: helix {} vs rings {}", b.volume, a.volume);
        assert!(b.triangles > a.triangles, "{id} {size}: the helix is finer");
        let e = exact::evaluate_exact_in(de_occt::Arena::new(), &f.title(), &f.program_source(), &helix, tol, &f.inputs_for(row));
        assert!(e.is_ok(), "{id} {size}: exact with the helix: {:?}", e.err().map(|x| x.1));
    }
}
