//! Surface modelling through shape programs: the three proofs (a vase from a
//! section network, a fender patch filled G1 into a four-sided hole, a mouse
//! shell with G2 blends), with continuity measured across their seams.

use de_shape::{exact, runtime};

const TOL: f64 = 0.01;

fn program(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/surfacing").join(name)).unwrap()
}

fn metrics(e: &exact::Exact) -> std::collections::BTreeMap<String, f64> {
    e.metrics.iter().map(|(k, m)| (k.clone(), m.value)).collect()
}

/// Evaluates exactly (a valid solid, exact surfaces in STEP) and for the view (meshed from it).
fn run(name: &str) -> (exact::Exact, runtime::Evaluated) {
    let src = program(name);
    let mut ex = exact::evaluate_exact(name, &src, &[], TOL).unwrap_or_else(|e| panic!("{name} exact: {e}"));
    assert!(ex.faceted.is_empty(), "{name}: nothing faceted: {:?}", ex.faceted);
    let view = runtime::evaluate(&src, &[], TOL).unwrap_or_else(|e| panic!("{name} view: {e}"));
    let v: f64 = ex.parts.iter().map(|p| ex.arena.volume(p.shape)).sum();
    eprintln!("{name}: exact volume {v:.1} mm³, view {:.1} mm³, metrics {:?}", view.volume, metrics(&ex));
    assert!((view.volume - v).abs() / v < 0.01, "{name}: the view within 1% of the exact volume");
    for p in &ex.parts {
        let c = ex.arena.check_geometry(p.shape).unwrap();
        assert_eq!(c["valid"], true, "{name}/{}: {c}", p.name);
        assert_eq!(c["solids"], 1, "{name}/{}: one solid", p.name);
    }
    let path = std::env::temp_dir().join(format!("forma-surf-{}-{name}.step", std::process::id()));
    let parts: Vec<(String, de_occt::Shape)> = ex.parts.iter().map(|p| (p.name.clone(), p.shape)).collect();
    ex.arena.write_step(&parts, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(text.contains("B_SPLINE_SURFACE"), "{name}: exact B-spline surfaces in STEP");
    (ex, view)
}

#[test]
fn a_vase_from_a_section_network() {
    let (ex, _) = run("vase.js");
    let m = metrics(&ex);
    assert!(m["network gap"] < 1e-6 && m["network deviation"] < 1e-3);
    assert_eq!(m["open edges"], 1.0, "open only at the mouth");
}

#[test]
fn a_fender_patch_meets_its_neighbours_g1() {
    let (ex, _) = run("fender.js");
    let m = metrics(&ex);
    assert!(m["seam gap"] < 1e-3, "{m:?}");
    assert!(m["seam crease"] < 0.05, "G1 across the seam: {m:?}");
}

#[test]
fn a_mouse_shell_with_g2_blends() {
    let (ex, _) = run("mouse.js");
    let m = metrics(&ex);
    assert_eq!(m["closed"], 1.0);
    for side in ["skirt", "crown"] {
        assert!(m[&format!("shoulder–{side} gap")] < 1e-3, "{m:?}");
        assert!(m[&format!("shoulder–{side} crease")] < 0.05, "{m:?}");
        assert!(m[&format!("shoulder–{side} curvature jump")] < 1e-3, "G2: {m:?}");
    }
}

#[test]
fn sheets_show_in_the_view_and_analyse() {
    // An open sheet returned as is: drawn as a thin skin; analysis reports it.
    let src = r#"
      const a = spline([[0, 0, 0], [20, 0, 6], [40, 0, 0]]), b = spline([[0, 40, 0], [20, 40, 6], [40, 40, 0]]);
      const s = sectionSurface([a, b]);
      metric("sheet", s.isSheet ? 1 : 0);
      metric("faces", s.body.faces);
      metric("min radius", s.curvature().minRadius, "mm");
      metric("under draft", s.draftAnalysis({ min: 5 }).under.length);
      const up = s.offset(2);
      metric("offset", deviation(s, up).max, "mm");
      const iso = s.iso("u", 0.5);
      metric("iso length", iso.length, "mm");
      const e = s.extendFace(5);
      metric("extended", e.body.faces);
      return s;
    "#;
    let view = runtime::evaluate(src, &[], TOL).unwrap();
    let m: std::collections::BTreeMap<_, _> = view.metrics.iter().map(|(k, v)| (k.clone(), v.value)).collect();
    eprintln!("sheet: {m:?}, {} triangles, volume {}", view.triangles, view.volume);
    assert_eq!(m["sheet"], 1.0);
    assert!((m["offset"] - 2.0).abs() < 1e-3);
    assert!(view.triangles > 0);
}
