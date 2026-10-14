//! Simulation checked against textbook answers: each test prints the case,
//! the analytic value, Iṣẹ́ Studio's and the error.

use de_shape::fea::{self, Face, Load, Study};
use de_shape::runtime;

const PI: f64 = std::f64::consts::PI;

fn report(case: &str, analytic: f64, got: f64) -> f64 {
    let err = (got - analytic) / analytic;
    eprintln!("VALIDATION | {case} | {analytic:.5} | {got:.5} | {:+.2} %", err * 100.0);
    err
}

/// A flat rectangle on the plane `axis` = `at`, facing `sign` along it, spanning a..b in the other two axes.
fn rect(axis: usize, at: f64, sign: f64, a: [f64; 2], b: [f64; 2]) -> Face {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let p = |x: f64, y: f64| {
        let mut q = [0.0; 3];
        q[axis] = at;
        q[u] = x;
        q[v] = y;
        q
    };
    let corners = [p(a[0], a[1]), p(b[0], a[1]), p(b[0], b[1]), p(a[0], b[1])];
    let mut triangles = vec![];
    for t in [[0, 1, 2], [0, 2, 3]] {
        for k in t {
            triangles.extend_from_slice(&corners[k]);
        }
    }
    let mut normal = [0.0; 3];
    normal[axis] = sign;
    Face { point: p((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0), normal, triangles, part: None, curved: false }
}

/// A part's triangles picked by a test on their centre and normal, as one curved face.
fn picked(e: &runtime::Evaluated, part: Option<&str>, keep: impl Fn([f64; 3], [f64; 3]) -> bool) -> Face {
    let (start, count) = match part {
        Some(n) => {
            let p = e.parts.iter().find(|p| p.name == n).unwrap();
            (p.index_start, p.index_count)
        }
        None => (0, e.indices.len()),
    };
    let at = |i: u32| {
        let i = i as usize * 3;
        [e.positions[i] as f64, e.positions[i + 1] as f64, e.positions[i + 2] as f64]
    };
    let mut triangles = vec![];
    let (mut point, mut normal) = ([0.0; 3], [0.0; 3]);
    for t in e.indices[start..start + count].chunks_exact(3) {
        let [a, b, c] = [at(t[0]), at(t[1]), at(t[2])];
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = n.map(|x| x / l);
        let mid = [0, 1, 2].map(|d| (a[d] + b[d] + c[d]) / 3.0);
        if keep(mid, n) {
            for q in [a, b, c] {
                triangles.extend_from_slice(&q);
            }
            point = mid;
            normal = n;
        }
    }
    assert!(!triangles.is_empty(), "no triangles picked");
    Face { point, normal, triangles, part: part.map(String::from), curved: true }
}

fn steel() -> fea::Elastic {
    fea::elastic("Steel").unwrap()
}

#[test]
fn a_column_buckles_at_the_euler_load() {
    // Steel 10 × 10 × 200 held at its foot, pushed down on its head: Pcr = π²EI/(4L²).
    let e = runtime::evaluate("return box(10, 10, 200);", &[], 0.01).unwrap();
    let study = Study {
        supports: vec![rect(2, 0.0, -1.0, [0.0, 0.0], [10.0, 10.0])],
        loads: vec![Load { face: rect(2, 200.0, 1.0, [0.0, 0.0], [10.0, 10.0]), force: [0.0, 0.0, -1000.0], pressure: 0.0 }],
        resolution: 160,
        ..Default::default()
    };
    let body = fea::Body { name: "Column".into(), positions: &e.positions, triangles: &e.indices, material: steel() };
    let b = fea::buckling(&body, &study, 2).unwrap();
    let pcr = PI * PI * 200_000.0 * (10f64.powi(4) / 12.0) / (4.0 * 200f64.powi(2));
    let err = report("Column buckling, fixed–free, load factor (Euler π²EI/4L²)", pcr / 1000.0, b.modes[0].factor);
    assert!(err.abs() < 0.03, "{:?}", b.notes);
    // The square section buckles alike both ways.
    assert!((b.modes[1].factor - b.modes[0].factor).abs() < 0.02 * b.modes[0].factor);
    assert_ne!(b.modes[0].direction, "z");
}

/// The fixed–fixed beam frequency equation with tension T (Euler–Bernoulli):
/// 2αβ(1 − cosh αL·cos βL) + (α² − β²)·sinh αL·sin βL = 0.
fn tensioned_fixed_fixed(ei: f64, rho_a: f64, l: f64, t: f64, near: f64) -> f64 {
    let g = |w: f64| {
        let (a, b) = (t / ei, rho_a * w * w / ei);
        let root = (a * a + 4.0 * b).sqrt();
        let (al, be) = (((a + root) / 2.0).sqrt(), ((-a + root) / 2.0).sqrt());
        2.0 * al * be * (1.0 - (al * l).cosh() * (be * l).cos()) + (al * al - be * be) * (al * l).sinh() * (be * l).sin()
    };
    // Bracket the root near the guess, then bisect.
    let (mut lo, mut hi) = (near * 0.6, near * 0.6);
    let start = g(lo);
    while g(hi).signum() == start.signum() {
        lo = hi;
        hi *= 1.01;
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if g(mid).signum() == g(lo).signum() { lo = mid } else { hi = mid }
    }
    (lo + hi) / 2.0
}

#[test]
fn a_stretched_beam_rings_higher_as_tension_theory_says() {
    // Steel 200 × 10 × 10, both ends clamped; the far end pulled 0.4 mm along the beam (T = EAδ/L = 40 kN).
    let e = runtime::evaluate("return box(200, 10, 10);", &[], 0.01).unwrap();
    let delta = 0.4;
    let study = Study {
        supports: vec![rect(0, 0.0, -1.0, [0.0, 0.0], [10.0, 10.0])],
        displacements: vec![fea::Displacement { face: rect(0, 200.0, 1.0, [0.0, 0.0], [10.0, 10.0]), by: [Some(delta), Some(0.0), Some(0.0)] }],
        resolution: 160,
        ..Default::default()
    };
    let mut m = steel();
    m.density = 7850.0;
    let body = fea::Body { name: "Beam".into(), positions: &e.positions, triangles: &e.indices, material: m };
    let (ei, rho_a, l) = (200e9 * 0.01f64.powi(4) / 12.0, 7850.0 * 1e-4, 0.2f64);
    let t = 200e9 * 1e-4 * delta / 200.0;
    let free = fea::modes_study(&body, &study, 2, false).unwrap();
    let f0 = 4.730f64.powi(2) / (2.0 * PI) * (ei / (rho_a * l.powi(4))).sqrt();
    let e0 = report("Fixed–fixed beam, first frequency (Hz), no tension", f0, free.modes[0].frequency);
    let tight = fea::modes_study(&body, &study, 2, true).unwrap();
    let w = tensioned_fixed_fixed(ei, rho_a, l, t, 2.0 * PI * f0);
    let ft = w / (2.0 * PI);
    let et = report("Fixed–fixed beam stretched to 40 kN, first frequency (Hz), prestressed modal", ft, tight.modes[0].frequency);
    assert!(e0.abs() < 0.03 && et.abs() < 0.03, "{:?}", tight.notes);
    assert!(ft > f0 * 1.05, "the tension matters in this case ({ft} vs {f0})");
}

#[test]
fn a_spinning_disc_is_stressed_as_lames_disc_says() {
    // A quarter of a steel disc (radius 50, 4 thick) at 10 000 rpm, held on its two cut faces and its underside by sliding.
    let e = runtime::evaluate("return cylinder(4, 50, { segments: 256 }).intersect(box(60, 60, 10));", &[], 0.01).unwrap();
    let mut m = steel();
    m.density = 7850.0;
    let study = Study {
        sliding: vec![rect(0, 0.0, -1.0, [0.0, 0.0], [50.0, 4.0]), rect(1, 0.0, -1.0, [0.0, 0.0], [4.0, 50.0]), rect(2, 0.0, -1.0, [0.0, 0.0], [50.0, 50.0])],
        spin: Some(fea::Spin { at: [0.0, 0.0, 0.0], axis: [0.0, 0.0, 1.0], rpm: 10_000.0 }),
        resolution: 100,
        ..Default::default()
    };
    let body = fea::Body { name: "Disc".into(), positions: &e.positions, triangles: &e.indices, material: m };
    let r = fea::solve_parts(&[body], &study).unwrap();
    let w: f64 = 10_000.0 * 2.0 * PI / 60.0;
    let q = 7850.0 * w * w * 0.05f64.powi(2) / 1e6; // ρω²R², MPa
    let nu = 0.29;
    // Near the centre: σr = σθ = (3 + ν)/8·ρω²R².
    let centre = fea::probe(&r, [2.0, 2.0, 2.0]).unwrap();
    let ec = report("Spinning disc, stress at the centre (MPa), (3+ν)/8·ρω²R²", (3.0 + nu) / 8.0 * q, centre.von_mises);
    // The rim grows (1 − ν)·ρω²R³/(4E).
    let grow = fea::probe(&r, [50.0 * 0.7071, 50.0 * 0.7071, 2.0]).unwrap().displacement;
    let radial = (grow[0] + grow[1]) * 0.7071;
    let er = report("Spinning disc, rim growth (mm), (1−ν)ρω²R³/4E", (1.0 - nu) * q * 50.0 / (4.0 * 200_000.0), radial);
    assert!(ec.abs() < 0.04 && er.abs() < 0.04, "{:?}", r.notes);
}

#[test]
fn a_cantilever_sags_under_its_own_weight_as_beam_theory_says() {
    let e = runtime::evaluate("return box(200, 20, 20);", &[], 0.01).unwrap();
    let mut m = steel();
    m.density = 7850.0;
    let study = Study { supports: vec![rect(0, 0.0, -1.0, [0.0, 0.0], [20.0, 20.0])], gravity: Some([0.0, 0.0, -9.81]), resolution: 80, ..Default::default() };
    let r = fea::solve_parts(&[fea::Body { name: "Beam".into(), positions: &e.positions, triangles: &e.indices, material: m }], &study).unwrap();
    let w = 7850.0 * 9.81 * 4e-4 / 1000.0; // N/mm
    let i = 20f64.powi(4) / 12.0;
    let want = w * 200f64.powi(4) / (8.0 * 200_000.0 * i);
    let tip = -fea::displacement_at(&r, [200.0, 10.0, 10.0]).unwrap()[2];
    let err = report("Cantilever under its own weight, tip (mm), wL⁴/8EI", want, tip);
    assert!(err.abs() < 0.04);
    // The support carries the whole weight.
    let weight = 7850.0 * 9.81 * 200.0 * 400.0 * 1e-9;
    let e2 = report("Cantilever under its own weight, support reaction (N), ρgV", weight, r.reactions[0].force[2]);
    assert!(e2.abs() < 0.01, "{:?}", r.reactions);
}

#[test]
fn a_moment_at_a_distance_bends_a_cantilever_as_beam_theory_says() {
    // A 20 N·m moment about y, given at a point 50 mm past the free end, carried to the end face: δ = ML²/2EI.
    let e = runtime::evaluate("return box(200, 20, 20);", &[], 0.01).unwrap();
    let m = 20_000.0;
    let study = Study {
        supports: vec![rect(0, 0.0, -1.0, [0.0, 0.0], [20.0, 20.0])],
        remote_loads: vec![fea::RemoteLoad { face: rect(0, 200.0, 1.0, [0.0, 0.0], [20.0, 20.0]), point: [250.0, 10.0, 10.0], force: [0.0; 3], moment: [0.0, -m, 0.0] }],
        resolution: 80,
        ..Default::default()
    };
    let r = fea::solve(&e.positions, &e.indices, &steel(), &study).unwrap();
    let i = 20f64.powi(4) / 12.0;
    let want = m * 200f64.powi(2) / (2.0 * 200_000.0 * i);
    let tip = fea::displacement_at(&r, [200.0, 10.0, 10.0]).unwrap()[2];
    let err = report("Cantilever with a remote moment, tip (mm), ML²/2EI", want, tip);
    assert!(err.abs() < 0.03);
    // The support's reaction moment balances it.
    let e2 = report("Cantilever with a remote moment, support moment (N·mm)", m, r.reactions[0].moment[1]);
    assert!(e2.abs() < 0.01, "{:?}", r.reactions);
}

/// A quarter of a 10 × 10 bar held on its three symmetry planes: pure tension.
fn quarter_bar_study(top: Option<Load>, by: Option<f64>) -> Study {
    Study {
        sliding: vec![rect(0, 0.0, -1.0, [0.0, 0.0], [5.0, 50.0]), rect(1, 0.0, -1.0, [0.0, 0.0], [50.0, 5.0]), rect(2, 0.0, -1.0, [0.0, 0.0], [5.0, 5.0])],
        loads: top.into_iter().collect(),
        displacements: by.map(|d| fea::Displacement { face: rect(2, 50.0, 1.0, [0.0, 0.0], [5.0, 5.0]), by: [None, None, Some(d)] }).into_iter().collect(),
        resolution: 20,
        ..Default::default()
    }
}

#[test]
fn a_bar_pulled_a_set_amount_pushes_back_ea_delta_over_l() {
    let e = runtime::evaluate("return box(5, 5, 50);", &[], 0.01).unwrap();
    let r = fea::solve(&e.positions, &e.indices, &steel(), &quarter_bar_study(None, Some(0.025))).unwrap();
    let want = 200_000.0 * 25.0 * 0.025 / 50.0;
    let pulled = r.reactions.iter().find(|x| x.name == "displacement 1").unwrap().force[2];
    let err = report("Bar pulled 0.025 mm by a set displacement, reaction (N), EAδ/L", want, pulled);
    assert!(err.abs() < 0.005, "{:?}", r.reactions);
    let s = fea::probe(&r, [2.5, 2.5, 25.0]).unwrap();
    assert!((s.stress[2] - 100.0).abs() < 0.5, "σz {}", s.stress[2]);
}

#[test]
fn a_bar_pulled_past_yield_stretches_as_the_bilinear_law_says() {
    // σ = 300 MPa on steel (yield 250, tangent 2000 MPa): ε = σy/E + (σ − σy)/Et.
    let e = runtime::evaluate("return box(5, 5, 50);", &[], 0.01).unwrap();
    let load = Load { face: rect(2, 50.0, 1.0, [0.0, 0.0], [5.0, 5.0]), force: [0.0, 0.0, 300.0 * 25.0], pressure: 0.0 };
    let mut study = quarter_bar_study(Some(load), None);
    study.plastic = true;
    let m = steel();
    let r = fea::solve(&e.positions, &e.indices, &m, &study).unwrap();
    let want = 50.0 * (250.0 / 200_000.0 + 50.0 / m.tangent);
    let top = fea::displacement_at(&r, [2.5, 2.5, 50.0]).unwrap()[2];
    let err = report("Bar pulled to 300 MPa, bilinear steel (Et 2 GPa), stretch (mm)", want, top);
    assert!(err.abs() < 0.01, "{:?}", r.notes);
    let p = r.plastic.unwrap();
    assert!(p.settled);
    assert!((p.max_strain - 50.0 / m.tangent).abs() < 0.01 * 50.0 / m.tangent, "plastic strain {}", p.max_strain);
}

#[test]
fn a_block_on_a_foundation_settles_q_over_k() {
    // 20 × 20 × 10 block on springs of 50 N/mm³ under 2 MPa: settles q/k + qh/E (the block's own squash, laterally free).
    let e = runtime::evaluate("return box(20, 20, 10);", &[], 0.01).unwrap();
    let study = Study {
        elastic_supports: vec![fea::ElasticSupport { face: rect(2, 0.0, -1.0, [0.0, 0.0], [20.0, 20.0]), stiffness: 50.0, normal_only: true }],
        sliding: vec![rect(0, 0.0, -1.0, [0.0, 0.0], [20.0, 10.0]), rect(1, 0.0, -1.0, [0.0, 0.0], [10.0, 20.0])],
        loads: vec![Load { face: rect(2, 10.0, 1.0, [0.0, 0.0], [20.0, 20.0]), force: [0.0; 3], pressure: 2.0 }],
        resolution: 20,
        ..Default::default()
    };
    let r = fea::solve(&e.positions, &e.indices, &fea::elastic("Aluminium").unwrap(), &study).unwrap();
    let want = 2.0 / 50.0 + 2.0 * 10.0 / 69_000.0;
    let top = -fea::displacement_at(&r, [10.0, 10.0, 10.0]).unwrap()[2];
    let err = report("Block on an elastic foundation, settlement (mm), q/k + qh/E", want, top);
    assert!(err.abs() < 0.01);
    let held = r.reactions.iter().find(|x| x.name == "elastic 1").unwrap().force[2];
    assert!((held - 800.0).abs() < 8.0, "foundation carries {held} N of 800");
}

#[test]
fn a_bearing_load_is_carried_whole_to_the_support() {
    // A lug with a 10 mm bore, a 2 kN bearing load in it pulling along +x, held at its far end.
    let e = runtime::evaluate("return box(60, 30, 10).subtract(cylinder(12, 5, { segments: 96 }).translate(15, 15, -1));", &[], 0.01).unwrap();
    let bore = picked(&e, None, |c, n| n[2].abs() < 0.1 && ((c[0] - 15.0).powi(2) + (c[1] - 15.0).powi(2)).sqrt() < 5.5);
    let study = Study { supports: vec![rect(0, 60.0, 1.0, [0.0, 0.0], [30.0, 10.0])], bearing_loads: vec![fea::BearingLoad { face: bore, force: [-2000.0, 0.0, 0.0] }], resolution: 60, ..Default::default() };
    let r = fea::solve(&e.positions, &e.indices, &steel(), &study).unwrap();
    let err = report("Lug with a bearing load, support reaction (N)", 2000.0, r.reactions[0].force[0]);
    assert!(err.abs() < 0.01, "{:?}", r.reactions);
    assert!(r.reactions[0].force[1].abs() < 20.0, "no sideways push: {:?}", r.reactions[0].force);
    // The bore side the load presses on carries it; the far side is nearly unstressed.
    let near = fea::probe(&r, [9.5, 15.0, 5.0]).unwrap().von_mises;
    let far = fea::probe(&r, [20.5, 15.0, 5.0]).unwrap().von_mises;
    assert!(near > 3.0 * far, "pressed side {near} MPa, other side {far} MPa");
}

#[test]
fn a_rotating_shaft_has_the_fatigue_life_goodman_and_marin_give() {
    // A machined steel shaft (d 20, 100 long) held at one end, 1 kN sideways on the other, turning: fully reversed bending.
    let e = runtime::evaluate("return cylinder(100, 10, { segments: 128 }).rotate(0, 90, 0);", &[], 0.01).unwrap();
    let end = |x: f64, s: f64| picked(&e, None, move |c, n| (c[0] - x).abs() < 1e-6 && (n[0] - s).abs() < 1e-6);
    let mut end_face = end(100.0, 1.0);
    end_face.curved = false;
    let mut root = end(0.0, -1.0);
    root.curved = false;
    let study = Study { supports: vec![root], loads: vec![Load { face: end_face, force: [0.0, 0.0, -1000.0], pressure: 0.0 }], resolution: 120, ..Default::default() };
    let r = fea::solve(&e.positions, &e.indices, &steel(), &study).unwrap();
    let props = fea::materials::lookup("Steel").unwrap();
    let f = fea::fatigue(&r, &props, &fea::FatigueSpec { diameter: Some(20.0), ..Default::default() }).unwrap();
    // At mid-span, the top fibre's cube: σ = M·z/I.
    let i_sec = PI * 20f64.powi(4) / 64.0;
    let k = fea::cube_near(&r, [50.0, 0.0, 10.0 - r.cell / 2.0]).unwrap();
    let c = r.cells[k].0;
    let (x, z) = (r.origin[0] + (c[0] as f64 + 0.5) * r.cell, r.origin[2] + (c[2] as f64 + 0.5) * r.cell);
    let sigma = 1000.0 * (100.0 - x) * z.abs() / i_sec;
    // Shigley: Se′ = 0.5·Sut = 200; ka = 4.51·400^−0.265; kb = 1.24·20^−0.107.
    let se = 200.0 * 4.51 * 400f64.powf(-0.265) * 1.24 * 20f64.powf(-0.107);
    let err = report("Rotating shaft, fatigue safety at mid-span (Goodman, R = −1), Se/σa", se / sigma, f.factors[k] as f64);
    assert!(err.abs() < 0.05, "{:?}", f.notes);
    assert!((f.strength - se).abs() < 0.01 * se);
    // Over the endurance limit at the root: a finite life from the S–N line.
    let over = fea::fatigue(&r, &props, &fea::FatigueSpec { diameter: Some(20.0), ..Default::default() }).unwrap();
    assert!(over.min_factor < f.factors[k] as f64);
}

#[test]
fn a_bolt_pulled_tight_clamps_its_sleeve_and_shares_a_load_by_stiffness() {
    // A steel bolt (shank r 4, heads r 10 × 4) through a steel sleeve (r 6–10, 40 long), glued where they meet.
    let src = r#"
        const bolt = cylinder(48, 4, { segments: 64 }).translate(0, 0, -4)
          .add(cylinder(4, 10, { segments: 64 }).translate(0, 0, -4))
          .add(cylinder(4, 10, { segments: 64 }).translate(0, 0, 40));
        const sleeve = cylinder(40, 10, { segments: 64 }).subtract(cylinder(42, 6, { segments: 64 }).translate(0, 0, -1));
        return [part("Bolt", bolt, { material: "Steel" }), part("Sleeve", sleeve, { material: "Steel" })];
    "#;
    let e = runtime::evaluate(src, &[], 0.01).unwrap();
    let bodies: Vec<fea::Body> = e.parts.iter().map(|p| fea::Body { name: p.name.clone(), positions: &e.positions, triangles: &e.indices[p.index_start..p.index_start + p.index_count], material: steel() }).collect();
    let foot = { let mut f = rect(2, -4.0, -1.0, [-10.0, -10.0], [10.0, 10.0]); f.part = Some("Bolt".into()); f };
    let head = { let mut f = rect(2, 44.0, 1.0, [-10.0, -10.0], [10.0, 10.0]); f.part = Some("Bolt".into()); f };
    let glued = vec![fea::ContactSpec { parts: ["Bolt".into(), "Sleeve".into()], kind: fea::ContactKind::Bonded, friction: 0.0, press_fit: false }];
    let bolt = fea::Bolt { part: Some("Bolt".into()), at: [0.0, 0.0, 20.0], axis: [0.0, 0.0, 1.0], preload: 10_000.0 };
    let pre = Study { supports: vec![foot.clone()], contacts: glued.clone(), bolts: vec![bolt.clone()], resolution: 56, ..Default::default() };
    let r = fea::solve_parts(&bodies, &pre).unwrap();
    // The sleeve carries the preload in compression: σ = −P/A.
    let a_sleeve = PI * (100.0 - 36.0);
    let s = fea::probe(&r, [8.0, 0.0, 10.0]).unwrap();
    let e1 = report("Bolt preload 10 kN, sleeve stress (MPa), −P/A", -10_000.0 / a_sleeve, s.stress[2]);
    let e0 = report("Bolt preload 10 kN, bolt force (N)", 10_000.0, r.bolts[0].force);
    // A 5 kN pull on the head: the bolt takes C = kb/(kb + km) of it.
    let loaded = Study { loads: vec![Load { face: head, force: [0.0, 0.0, 5000.0], pressure: 0.0 }], ..pre };
    let r2 = fea::solve_parts(&bodies, &loaded).unwrap();
    let (kb, km) = (16.0 * PI, a_sleeve);
    let gain = r2.bolts[0].force - 10_000.0;
    let e2 = report("Bolted joint, bolt force gain under 5 kN (N), C·Fe with C = kb/(kb+km)", 5000.0 * kb / (kb + km), gain);
    assert!(e0.abs() < 0.01 && e1.abs() < 0.05 && e2.abs() < 0.1, "{:?}", r2.notes);
}

#[test]
fn a_bar_heated_at_one_end_warms_through_time_as_the_series_solution_says() {
    // Aluminium 100 × 10 × 10 at 20 °C, its end at x = 0 held at 100 °C from t = 0, the rest insulated.
    let e = runtime::evaluate("return box(100, 10, 10);", &[], 0.01).unwrap();
    let m = fea::materials::lookup("Aluminium").unwrap();
    let study = fea::ThermalStudy { holds: vec![fea::Hold { face: rect(0, 0.0, -1.0, [0.0, 0.0], [10.0, 10.0]), temperature: 100.0 }], heats: vec![], convection: None, resolution: 100 };
    let run = fea::Transient { duration: 30.0, steps: 300, initial: 20.0 };
    let x = 50.5;
    let r = fea::thermal_transient(&e.positions, &e.indices, m.conductivity, m.density, m.specific_heat, &study, &run, &[[x, 5.5, 5.5]]).unwrap();
    // T = T1 + (T0 − T1)·Σ 4/((2n+1)π)·sin((2n+1)πx/2L)·exp(−((2n+1)π/2L)²·a·t), a = k/ρc in mm²/s.
    let a = m.conductivity / (m.density * m.specific_heat) * 1e6;
    let l = 100.0;
    let series: f64 = (0..200).map(|n| {
        let k = (2 * n + 1) as f64 * PI / (2.0 * l);
        4.0 / ((2 * n + 1) as f64 * PI) * (k * x).sin() * (-k * k * a * 30.0).exp()
    }).sum();
    let want = 100.0 + (20.0 - 100.0) * series;
    let got = *r.probes[0].last().unwrap();
    let err = report("Bar heated at one end, temperature rise at mid-length after 30 s (K)", want - 20.0, got - 20.0);
    assert!(err.abs() < 0.03, "{:?}", r.notes);
    assert!(r.max.windows(2).all(|w| w[1] >= w[0] - 1e-9), "never cools");
}

fn frame_cantilever(load: fea::frame::NodeLoad) -> fea::frame::Frame {
    fea::frame::Frame {
        nodes: vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0]],
        members: vec![fea::frame::Member { a: 0, b: 1, section: fea::frame::SectionSpec::Rect { w: 20.0, h: 40.0 }, material: "Steel".into(), up: None, load: None }],
        supports: vec![fea::frame::FrameSupport { node: 0, kind: "fixed".into(), hold: None }],
        loads: vec![load],
        gravity: None,
    }
}

#[test]
fn a_beam_frame_bends_twists_and_buckles_as_beam_theory_says() {
    use fea::frame::*;
    let (e, nu) = (200_000.0, 0.29);
    let (w, h, l) = (20.0f64, 40.0f64, 500.0f64);
    // Cantilever, 1 kN down at the tip: PL³/3EI, stress PL·c/I.
    let r = solve(&frame_cantilever(NodeLoad { node: 1, force: [0.0, 0.0, -1000.0], moment: [0.0; 3] })).unwrap();
    let iy = w * h * h * h / 12.0;
    let e1 = report("Frame cantilever, tip deflection (mm), PL³/3EI", 1000.0 * l.powi(3) / (3.0 * e * iy), -r.displacements[1][2]);
    let e2 = report("Frame cantilever, root stress (MPa), PL·c/I", 1000.0 * l * (h / 2.0) / iy, r.members[0].max_stress);
    // Twisted: θ = TL/GJ (J of a 2:1 rectangle from Roark).
    let r2 = solve(&frame_cantilever(NodeLoad { node: 1, force: [0.0; 3], moment: [100_000.0, 0.0, 0.0] })).unwrap();
    let j = SectionSpec::Rect { w, h }.props().unwrap().j;
    let e3 = report("Frame cantilever, twist (rad), TL/GJ", 100_000.0 * l / (e / (2.0 * (1.0 + nu)) * j), r2.displacements[1][3]);
    // Simply supported (pinned at one end, a roller at the other), 2 N/mm down, two members: 5wL⁴/384EI at the middle.
    let ss = Frame {
        nodes: vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0], [1000.0, 0.0, 0.0]],
        members: (0..2).map(|i| Member { a: i, b: i + 1, section: SectionSpec::Rect { w, h }, material: "Steel".into(), up: None, load: Some([0.0, 0.0, -2.0]) }).collect(),
        supports: vec![FrameSupport { node: 0, kind: "pinned".into(), hold: Some([true, true, true, true, false, false]) }, FrameSupport { node: 2, kind: "".into(), hold: Some([false, true, true, false, false, false]) }],
        loads: vec![],
        gravity: None,
    };
    let r3 = solve(&ss).unwrap();
    let e4 = report("Frame simply supported beam, mid-span sag under 2 N/mm (mm), 5wL⁴/384EI", 5.0 * 2.0 * 1000f64.powi(4) / (384.0 * e * iy), -r3.displacements[1][2]);
    let e5 = report("Frame simply supported beam, mid-span stress (MPa), wL²/8·c/I", 2.0 * 1e6 / 8.0 * (h / 2.0) / iy, r3.members[0].max_stress.max(r3.members[1].max_stress));
    // A pushed strut reports its Euler load.
    let r4 = solve(&frame_cantilever(NodeLoad { node: 1, force: [-1000.0, 0.0, 0.0], moment: [0.0; 3] })).unwrap();
    let iz = h * w * w * w / 12.0;
    let e6 = report("Frame strut, Euler load between its joints (N), π²EI/L²", PI * PI * e * iz / (l * l), r4.members[0].buckling_load.unwrap());
    for err in [e1, e2, e3, e4, e5, e6] {
        assert!(err.abs() < 1e-6, "frame error {err}");
    }
    assert!((r.reactions[0].1[2] - 1000.0).abs() < 1e-6);
}

#[test]
fn a_study_writes_a_calculix_deck() {
    let e = runtime::evaluate("return box(100, 10, 10);", &[], 0.01).unwrap();
    let study = Study {
        supports: vec![rect(0, 0.0, -1.0, [0.0, 0.0], [10.0, 10.0])],
        loads: vec![Load { face: rect(0, 100.0, 1.0, [0.0, 0.0], [10.0, 10.0]), force: [0.0, 0.0, -100.0], pressure: 0.0 }],
        resolution: 20,
        ..Default::default()
    };
    let body = fea::Body { name: "Bar".into(), positions: &e.positions, triangles: &e.indices, material: steel() };
    let deck = fea::calculix_inp(std::slice::from_ref(&body), &study, fea::InpKind::Static, 1).unwrap();
    assert!(deck.contains("*NODE") && deck.contains("TYPE=C3D8I") && deck.contains("*STATIC") && deck.contains("*CLOAD"));
    // The forces add up to the load.
    let total: f64 = deck.split("*CLOAD\n").nth(1).unwrap().lines().take_while(|l| !l.starts_with('*')).filter(|l| l.split(", ").nth(1) == Some("3")).map(|l| l.split(", ").nth(2).unwrap().parse::<f64>().unwrap()).sum();
    assert!((total + 100.0).abs() < 1e-6, "{total}");
    let elements = deck.lines().filter(|l| l.split(", ").count() == 9).count();
    assert_eq!(elements, 20 * 2 * 2);
}

#[test]
fn a_square_plate_sags_as_timoshenkos_plate_tables_say() {
    // Steel 100 × 100 × 1 under 0.01 MPa: w = α·q·a⁴/D (Timoshenko & Woinowsky-Krieger, tables 8 and 35).
    use fea::plate::*;
    let e = runtime::evaluate("return box(100, 100, 1);", &[], 0.01).unwrap();
    let edges = |hold: EdgeHold| -> Vec<PlateSupport> {
        vec![
            PlateSupport { face: rect(0, 0.0, -1.0, [0.0, 0.0], [100.0, 1.0]), hold },
            PlateSupport { face: rect(0, 100.0, 1.0, [0.0, 0.0], [100.0, 1.0]), hold },
            PlateSupport { face: rect(1, 0.0, -1.0, [0.0, 0.0], [1.0, 100.0]), hold },
            PlateSupport { face: rect(1, 100.0, 1.0, [0.0, 0.0], [1.0, 100.0]), hold },
        ]
    };
    let m = steel();
    let d = m.e * 1.0 / (12.0 * (1.0 - m.nu * m.nu));
    let q = 0.01;
    let simple = plate(&e.positions, &e.indices, &m, &PlateStudy { supports: edges(EdgeHold::Simple), pressure: q, loads: vec![], resolution: 40 }).unwrap();
    let e1 = report("Square plate, simply supported, centre sag (mm), 0.00406·qa⁴/D", 0.00406 * q * 1e8 / d, simple.max_sag);
    // Largest bending moment 0.0479·q·a² at the centre: σ = 6M/t².
    // (The largest von Mises is at the corners, where the twisting moment is.)
    let centre: Vec<f64> = simple.cells.iter().filter(|(c, _)| (19..=20).contains(&c[0]) && (19..=20).contains(&c[1])).map(|(_, s)| *s as f64).collect();
    let e2 = report("Square plate, simply supported, centre stress (MPa), 6·0.0479·qa²/t²", 6.0 * 0.0479 * q * 1e4, centre.iter().sum::<f64>() / centre.len() as f64);
    let clamped = plate(&e.positions, &e.indices, &m, &PlateStudy { supports: edges(EdgeHold::Clamped), pressure: q, loads: vec![], resolution: 40 }).unwrap();
    let e3 = report("Square plate, clamped, centre sag (mm), 0.00126·qa⁴/D", 0.00126 * q * 1e8 / d, clamped.max_sag);
    assert!(e1.abs() < 0.02 && e3.abs() < 0.03, "{:?}", simple.notes);
    // At the centre (its moment is 0.0479·qa²); the clamped edge's −0.0513·qa² is larger, read at element middles.
    assert!(e2.abs() < 0.05);
}
