//! Sweeps, lofts and the exact solid vocabulary, through shape programs:
//! both kernels against each other and against the formulas.

use de_shape::{exact, runtime};
use std::f64::consts::PI;

const TOL: f64 = 0.01;

/// Volumes from the view's mesh and from the exact kernel.
fn both(source: &str) -> (f64, f64) {
    let mesh = runtime::evaluate(source, &[], TOL).unwrap_or_else(|e| panic!("mesh: {e}"));
    let ex = exact::evaluate_exact("Test", source, &[], TOL).unwrap_or_else(|e| panic!("exact: {e}"));
    let v: f64 = ex.parts.iter().map(|p| ex.arena.volume(p.shape)).sum();
    assert!(ex.faceted.is_empty(), "nothing faceted: {:?}", ex.faceted);
    (mesh.volume, v)
}

fn report(what: &str, want: f64, mesh: f64, exact: f64) {
    eprintln!("{what}: analytic {want:.6}, exact {exact:.6} ({:+.2e}), mesh {mesh:.6} ({:+.2e})", (exact - want) / want, (mesh - want) / want);
}

#[test]
fn a_spring_is_a_helical_tube() {
    let (r, pitch, turns, wire) = (10.0, 4.0, 5.0, 1.0);
    let src = format!("return sweep(circle({wire}), helix({{ radius: {r}, pitch: {pitch}, turns: {turns} }}));");
    let (mesh, ex) = both(&src);
    let length = turns * ((2.0 * PI * r).powi(2) + pitch * pitch).sqrt();
    let want = PI * wire * wire * length;
    report("spring", want, mesh, ex);
    assert!((ex - want).abs() / want < 1e-5);
    // A 1 mm round as chords within 0.01 mm loses about 2·tol/r of its area (as any cylinder this thin would).
    assert!((mesh - want).abs() / want < 2.0 * TOL / wire, "the mesh within its faceting");
    // The path's own length, measured exactly.
    let len = runtime::evaluate(&format!("metric('L', helix({{ radius: {r}, pitch: {pitch}, turns: {turns} }}).length); return box(1, 1, 1);"), &[], TOL).unwrap();
    assert!((len.metrics["L"].value - length).abs() < 1e-6 * length, "helix length {} vs {length}", len.metrics["L"].value);
}

/// An M8×1.25 bolt's thread: the ISO basic profile (60° flanks, P/8 crest
/// flat at the major diameter, root at the basic minor diameter d − 1.0825 P)
/// swept as a screw round a core; volume by Pappus, turn by turn.
const THREAD: &str = r#"
const P = 1.25, d = 8, H = Math.sqrt(3) / 2 * P;
const r1 = (d - 2 * 5 / 8 * H) / 2, R = d / 2, e = 0.1 * P;
const hw = (r) => P / 16 + Math.tan(Math.PI / 6) * (R - r);
const tooth = polygon([[r1 - e, -hw(r1 - e)], [R, -P / 16], [R, P / 16], [r1 - e, hw(r1 - e)]]);
const L = 16;
const thread = sweep(tooth, helix({ radius: R, pitch: P, height: L }), { place: "axial" });
const core = cylinder(L + 4, r1).translate(0, 0, -2);
return core.add(thread);
"#;

#[test]
fn an_iso_thread_measures_by_pappus() {
    let (mesh, ex) = both(THREAD);
    let p = 1.25;
    let h = 3f64.sqrt() / 2.0 * p;
    let (r1, rr) = ((8.0 - 1.25 * h) / 2.0, 4.0);
    let hw = |r: f64| p / 16.0 + (PI / 6.0).tan() * (rr - r);
    // The tooth outside the core: a trapezoid from r1 to R.
    let (w1, w2, ht) = (2.0 * hw(r1), p / 8.0, rr - r1);
    let area = (w1 + w2) / 2.0 * ht;
    let xbar = r1 + ht * (w1 + 2.0 * w2) / (3.0 * (w1 + w2));
    let want = PI * r1 * r1 * 20.0 + 16.0 / p * 2.0 * PI * area * xbar;
    report("M8 thread", want, mesh, ex);
    assert!((ex - want).abs() / want < 1e-5);
    assert!((mesh - want).abs() / want < 2.0 * TOL / 3.3);
    // Exact faces: the flanks are B-spline sweeps, not facets.
    let mut e = exact::evaluate_exact("Bolt", THREAD, &[], TOL).unwrap();
    let path = std::env::temp_dir().join(format!("ise-thread-{}.step", std::process::id()));
    let parts: Vec<(String, de_occt::Shape)> = e.parts.iter().map(|p| (p.name.clone(), p.shape)).collect();
    e.arena.write_step(&parts, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(text.contains("B_SPLINE_SURFACE"), "swept flanks");
    assert!(text.contains("CYLINDRICAL_SURFACE"), "a true core");
    let check = e.arena.check_geometry(e.parts[0].shape).unwrap();
    assert_eq!(check["valid"], true, "{check}");
}

#[test]
fn pipes_follow_bent_paths() {
    // A handrail: up 900, along 1500, down 900, with 80 mm bends; 42.4 × 2.6 tube.
    let src = "return pipe(polyline([[0, 0, 0], [0, 0, 900], [1500, 0, 900], [1500, 0, 0]], { radius: 80 }), 21.2, { inner: 18.6 });";
    let (mesh, ex) = both(src);
    let length = 900.0 * 2.0 + 1500.0 - 4.0 * 80.0 + 2.0 * PI * 80.0 / 2.0;
    let want = PI * (21.2f64.powi(2) - 18.6f64.powi(2)) * length;
    report("handrail", want, mesh, ex);
    assert!((ex - want).abs() / want < 1e-6);
    assert!((mesh - want).abs() / want < 2e-3);
    // Mitred (no bend radius): a square tube frame, mesh against exact.
    let (mesh, ex) = both("return sweep(rect(20, 20, { center: true }).subtract(rect(16, 16, { center: true })), polyline([[0, 0, 0], [0, 0, 300], [400, 0, 300]]));");
    // Two legs to the centre line of the mitre: 144 mm² × (300 + 400) on centre.
    let want = 144.0 * 700.0;
    report("mitred frame", want, mesh, ex);
    assert!((ex - want).abs() / want < 1e-6);
    assert!((mesh - want).abs() / want < 1e-6);
}

#[test]
fn twists_scales_and_orientations_agree() {
    // Each with its thinnest feature: chords within tol lose about 2·tol/size of a round's area.
    for (what, size, src) in [
        ("twisted bar", 4.0, "return sweep(rect(10, 4, { center: true }), polyline([[0, 0, 0], [0, 0, 60]]), { twist: 180 });"),
        ("tapered horn", 4.0, "return sweep(circle(10), arcPath([0, 0, 0], [30, 0, 30], [60, 0, 0]), { scale: 0.4 });"),
        ("fixed", 6.0, "return sweep(rect(6, 6, { center: true }), spline([[0, 0, 0], [20, 10, 20], [40, 0, 50]]), { orientation: 'fixed' });"),
        ("binormal", 1.0, "return sweep(rect(2, 1), helix({ radius: 8, pitch: 3, turns: 2 }), { orientation: 'binormal', binormal: [0, 0, 1] });"),
        ("conical spring", 0.8, "return sweep(circle(0.8), helix({ radius: 12, pitch: 4, turns: 4, taper: -20 }));"),
        ("left thread", 0.4, "return sweep(polygon([[5, 0], [6, 0.4], [6, 0.6], [5, 1]]), helix({ radius: 5, pitch: 1.5, height: 6, leftHand: true }), { place: 'axial' });"),
    ] {
        let (mesh, ex) = both(src);
        report(what, ex, mesh, ex);
        assert!((mesh - ex).abs() / ex < 2.0 * TOL / size, "{what}: mesh {mesh} vs exact {ex}");
    }
}

#[test]
fn lofts_bottles_and_ducts() {
    // Ruled between squares: a frustum; smooth through equal circles: a cylinder.
    let (mesh, ex) = both("return loft([rect(20, 20, { center: true }).at(0), rect(10, 10, { center: true }).at(15)], { ruled: true });");
    let want = 15.0 / 3.0 * (400.0 + 100.0 + 200.0);
    report("ruled frustum", want, mesh, ex);
    assert!((ex - want).abs() / want < 1e-9 && (mesh - want).abs() / want < 1e-6);
    // A bottle: rounded-rectangle body to a round shoulder and neck, smooth.
    let bottle = "const body = rect(60, 40, { center: true }).offset(-8).offset(8);\n\
                  return loft([body.at(0), body.at(120), circle(22).at(170), circle(12).at(200), circle(12).at(220)]);";
    let (mesh, ex) = both(bottle);
    report("bottle", ex, mesh, ex);
    assert!(ex > PI * 144.0 * 220.0 && ex < 2400.0 * 220.0);
    assert!((mesh - ex).abs() / ex < 1e-3, "the view meshes the exact skin");
    // A square-to-round duct with a 2 mm wall (same holes in every section).
    let duct = "return loft([rect(60, 60, { center: true }).subtract(rect(56, 56, { center: true })).at(0), circle(25).subtract(circle(23)).place({ origin: [0, 20, 80], normal: [0, 0.3, 1] })]);";
    let (mesh, ex) = both(duct);
    report("duct", ex, mesh, ex);
    assert!(ex > 0.0 && (mesh - ex).abs() / ex < 2e-3);
}

#[test]
fn offsets_defeaturing_and_checks() {
    // Steiner's formula for a box grown 2 mm with round edges.
    let (a, b, c, d) = (40.0, 30.0, 20.0, 2.0);
    let (mesh, ex) = both("return box(40, 30, 20).offset(2);");
    let want = a * b * c + 2.0 * (a * b + b * c + c * a) * d + PI * (a + b + c) * d * d + 4.0 / 3.0 * PI * d * d * d;
    report("offset box", want, mesh, ex);
    assert!((ex - want).abs() / want < 1e-6 && (mesh - want).abs() / want < 1e-3);
    // Holes and rounds removed: back to the block.
    let (_, ex) = both("return box(40, 30, 20).fillet(2, { parallel: 'z' }).subtract(cylinder(30, 3).translate(12, 15, -5), cylinder(30, 4).translate(28, 15, -5)).defeature({ holes: 10, fillets: 3 });");
    assert!((ex - 24000.0).abs() < 1e-6, "defeatured {ex}");
    // Only the small hole: the 8 mm one stays.
    let (_, ex) = both("return box(40, 30, 20).subtract(cylinder(30, 3).translate(12, 15, -5), cylinder(30, 4).translate(28, 15, -5)).defeature({ holes: 6 });");
    assert!((ex - (24000.0 - PI * 16.0 * 20.0)).abs() < 1e-6, "{ex}");
    // A thickened sheet: a hollowed box's walls, open at the top, rounded joins.
    let (_, ex) = both("return box(40, 30, 20).thicken(2, { faces: { normal: [0, 0, 1] } });");
    assert!((ex - (24000.0 - 36.0 * 26.0 * 18.0)).abs() < 1e-6, "{ex}");
    // A variable fillet sits between its two ends' constant ones.
    let (_, lo) = both("return box(40, 30, 20).fillet(4, { parallel: 'z', near: [0, 0, 10] });");
    let (_, var) = both("return box(40, 30, 20).fillet([1, 4], { parallel: 'z', near: [0, 0, 10] });");
    assert!(var > lo && var < 24000.0);
    // The check, from a program.
    let r = runtime::evaluate("const c = box(10, 10, 10).subtract(cylinder(20, 2).translate(5, 5, -5)).checkGeometry(); metric('valid', c.valid ? 1 : 0); metric('faces', c.faces); return box(1, 1, 1);", &[], TOL).unwrap();
    assert_eq!(r.metrics["valid"].value, 1.0);
    assert_eq!(r.metrics["faces"].value, 7.0);
}

#[test]
fn paint_and_materials_colour_the_step_file() {
    let src = "return [part('Lid', box(30, 20, 4).paint('#c8372f', { normal: [0, 0, 1] }, { name: 'Sealing face' }), { material: 'PETG (black)' }), part('Base', box(30, 20, 10).translate(0, 0, -10), { material: 'Aluminium 6061' })];";
    let mut e = exact::evaluate_exact("Box", src, &[], TOL).unwrap();
    let path = std::env::temp_dir().join(format!("ise-paint-{}.step", std::process::id()));
    let parts: Vec<(String, de_occt::Shape)> = e.parts.iter().map(|p| (p.name.clone(), p.shape)).collect();
    e.arena.write_step(&parts, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(text.contains("'Sealing face'"));
    assert!(text.contains("DRAUGHTING_PRE_DEFINED_COLOUR('red')") || text.contains("COLOUR_RGB('',0.78"), "the painted face");
    // Black PETG and aluminium.
    assert!(text.contains("COLOUR_RGB('',0.137"), "the lid in black");
    assert!(text.contains("COLOUR_RGB('',0.78"), "the base in aluminium");
    // The view is not affected by paint.
    assert!(runtime::evaluate(src, &[], TOL).is_ok());
}
