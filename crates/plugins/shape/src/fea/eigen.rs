//! Eigenproblems on the voxel model: natural frequencies (K·φ = ω²·M·φ,
//! optionally with the stiffness a load's stress adds or takes away) and
//! linear buckling (K·φ = λ·(−K_G)·φ, K_G the geometric stiffness of the
//! stress the study's loads make, λ the factor on those loads that buckles it).
//!
//! Both are found by subspace iteration: a block of vectors is repeatedly
//! pushed through K⁻¹·B (each solve by the same multigrid-preconditioned
//! conjugate gradients as a static study, from the last answer), and the
//! block's best combinations taken by Rayleigh–Ritz, until the wanted
//! eigenvalues stop moving.

use super::*;

/// ∫ ∂Nₐ/∂xᵢ · ∂N_b/∂xⱼ dV over a cube of side h, for i, j = 0..3: the
/// pieces a constant stress's geometric stiffness is made of (2×2×2 Gauss, exact).
fn grad_products(h: f64) -> [[[f64; 64]; 3]; 3] {
    let gp = 1.0 / 3f64.sqrt();
    let w = (h / 2.0).powi(3);
    let mut out = [[[0.0; 64]; 3]; 3];
    for c in CORNERS {
        let xi = [c[0] * gp, c[1] * gp, c[2] * gp];
        let grads: Vec<[f64; 3]> = CORNERS
            .iter()
            .map(|n| {
                let f = |q: usize| 1.0 + xi[q] * n[q];
                [n[0] * f(1) * f(2) / 8.0 * 2.0 / h, n[1] * f(0) * f(2) / 8.0 * 2.0 / h, n[2] * f(0) * f(1) / 8.0 * 2.0 / h]
            })
            .collect();
        for i in 0..3 {
            for j in 0..3 {
                for a in 0..8 {
                    for b in 0..8 {
                        out[i][j][8 * a + b] += w * grads[a][i] * grads[b][j];
                    }
                }
            }
        }
    }
    out
}

/// Each cube's geometric stiffness (8 × 8, the same in each direction) from
/// its stress (xx yy zz xy yz zx), scaled by how full it is.
pub(super) fn geometric(model: &Model, tensors: &[[f64; 6]]) -> Vec<[f64; 64]> {
    let g = grad_products(model.grid.h);
    tensors
        .iter()
        .zip(&model.weight)
        .map(|(s, &w)| {
            // The stress as a 3 × 3 matrix.
            let m = [[s[0], s[3], s[5]], [s[3], s[1], s[4]], [s[5], s[4], s[2]]];
            std::array::from_fn(|k| w * (0..3).flat_map(|i| (0..3).map(move |j| (i, j))).map(|(i, j)| m[i][j] * g[i][j][k]).sum::<f64>())
        })
        .collect()
}

/// K_G·x over every cube, in parallel (each cube's 8 × 8 acts alike on x, y and z).
pub(super) fn geo_apply(corners: &[[usize; 8]], geo: &[[f64; 64]], x: &[f64]) -> Vec<f64> {
    let n = x.len();
    corners
        .par_chunks(4096)
        .zip(geo.par_chunks(4096))
        .fold(
            || vec![0.0; n],
            |mut y, (cs, gs)| {
                for (c, g) in cs.iter().zip(gs) {
                    for (a, &na) in c.iter().enumerate() {
                        let mut s = [0.0; 3];
                        for (b, &nb) in c.iter().enumerate() {
                            let k = g[8 * a + b];
                            for d in 0..3 {
                                s[d] += k * x[3 * nb + d];
                            }
                        }
                        for d in 0..3 {
                            y[3 * na + d] += s[d];
                        }
                    }
                }
                y
            },
        )
        .reduce(|| vec![0.0; n], |mut a, b| {
            a.iter_mut().zip(&b).for_each(|(p, q)| *p += q);
            a
        })
}

/// The `count` largest μ of B·φ = μ·K·φ (the smallest λ = 1/μ of K·φ = λ·B·φ),
/// with their vectors, by subspace iteration on K⁻¹·B. `b` applies B
/// (masked to the free directions). Returns them largest μ first, and
/// whether they settled.
fn subspace(model: &Model, count: usize, b: &(dyn Fn(&[f64]) -> Vec<f64> + Sync)) -> (Vec<f64>, Vec<Vec<f64>>, bool) {
    let p = count + 3;
    let n3 = 3 * model.nodes;
    // Start from varied vectors (a fixed sequence, so results repeat).
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 2000) as f64 / 1000.0 - 1.0
    };
    let mut x: Vec<Vec<f64>> = (0..p)
        .map(|_| {
            let mut v: Vec<f64> = (0..n3).map(|_| rnd()).collect();
            model.mask(&mut v);
            v
        })
        .collect();
    let mut z: Vec<Vec<f64>> = vec![vec![0.0; n3]; p];
    let mut last = vec![0.0; p];
    let mut mu = vec![0.0; p];
    let mut settled = false;
    for round in 0..60 {
        // Loose solves while the subspace is still far off, tight once it settles.
        let tol = if round < 3 { 1e-4 } else { 1e-7 };
        z = (0..p)
            .into_par_iter()
            .map(|i| {
                let y = b(&x[i]);
                let mut zi = z[i].clone();
                model.cg(&y, &mut zi, tol);
                zi
            })
            .collect();
        // Rayleigh–Ritz: Bᵣ·q = μ·Kᵣ·q, with Kᵣ = L·Lᵀ.
        let kz: Vec<Vec<f64>> = z.par_iter().map(|zi| model.apply(zi)).collect();
        let bz: Vec<Vec<f64>> = z.par_iter().map(|zi| b(zi)).collect();
        let dot = |a: &[f64], c: &[f64]| a.iter().zip(c).map(|(p, q)| p * q).sum::<f64>();
        let kr: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| (dot(&z[i], &kz[j]) + dot(&z[j], &kz[i])) / 2.0).collect()).collect();
        let br: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| (dot(&z[i], &bz[j]) + dot(&z[j], &bz[i])) / 2.0).collect()).collect();
        let mut l = vec![vec![0.0; p]; p];
        for i in 0..p {
            for j in 0..=i {
                let s: f64 = kr[i][j] - (0..j).map(|k| l[i][k] * l[j][k]).sum::<f64>();
                if i == j {
                    l[i][i] = s.max(1e-300).sqrt();
                } else {
                    l[i][j] = s / l[j][j];
                }
            }
        }
        let solve_l = |v: &[f64]| -> Vec<f64> {
            let mut y = vec![0.0; p];
            for i in 0..p {
                y[i] = (v[i] - (0..i).map(|k| l[i][k] * y[k]).sum::<f64>()) / l[i][i];
            }
            y
        };
        let solve_lt = |v: &[f64]| -> Vec<f64> {
            let mut y = vec![0.0; p];
            for i in (0..p).rev() {
                y[i] = (v[i] - (i + 1..p).map(|k| l[k][i] * y[k]).sum::<f64>()) / l[i][i];
            }
            y
        };
        // A = L⁻¹·Bᵣ·L⁻ᵀ.
        let cols: Vec<Vec<f64>> = (0..p).map(|j| solve_l(&(0..p).map(|i| br[i][j]).collect::<Vec<_>>())).collect();
        let a: Vec<Vec<f64>> = (0..p).map(|i| solve_l(&(0..p).map(|j| cols[j][i]).collect::<Vec<_>>())).collect();
        let a: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| (a[i][j] + a[j][i]) / 2.0).collect()).collect();
        let (vals, vecs) = symmetric_eigen(a);
        // Largest μ first.
        let order: Vec<usize> = (0..p).rev().collect();
        let q: Vec<Vec<f64>> = order.iter().map(|&k| solve_lt(&vecs[k])).collect();
        mu = order.iter().map(|&k| vals[k]).collect();
        x = q.iter().map(|qi| (0..n3).map(|d| (0..p).map(|j| z[j][d] * qi[j]).sum()).collect()).collect();
        // Each vector scaled to K-norm 1 (keeps the next solves well sized).
        for xi in x.iter_mut() {
            let k = dot(xi, &model.apply(xi)).max(1e-300).sqrt();
            xi.iter_mut().for_each(|v| *v /= k);
        }
        // The next solves' answers are close to μ·x: start there.
        z = x.iter().zip(&mu).map(|(xi, m)| xi.iter().map(|v| v * m).collect()).collect();
        // Measured against the largest: a μ near zero (no buckling that way) never settles relatively.
        let top = mu[..count].iter().fold(0.0f64, |a, m| a.max(m.abs())).max(1e-300);
        let done = (0..count).all(|i| (mu[i] - last[i]).abs() <= 1e-5 * mu[i].abs().max(1e-3 * top));
        last = mu.clone();
        if done && round > 1 {
            settled = true;
            break;
        }
    }
    (mu[..count].to_vec(), x[..count].to_vec(), settled)
}

/// A shape's look: per cube how far it moves (0–1) and per node the shape (largest movement 1), and its main direction.
fn shape_of(model: &Model, v: &[f64], nodes: &[usize]) -> (Vec<f32>, Vec<[f32; 3]>, String) {
    let mag = |n: usize| (v[3 * n] * v[3 * n] + v[3 * n + 1] * v[3 * n + 1] + v[3 * n + 2] * v[3 * n + 2]).sqrt();
    let top = nodes.iter().map(|&n| mag(n)).fold(0.0, f64::max).max(1e-300);
    let mut along = [0.0; 3];
    for &n in nodes {
        for (d, a) in along.iter_mut().enumerate() {
            *a += v[3 * n + d].abs();
        }
    }
    let dir = (0..3).max_by(|&a, &b| along[a].total_cmp(&along[b])).unwrap_or(2);
    (
        model.corners.iter().map(|c| (c.iter().map(|&n| mag(n)).sum::<f64>() / 8.0 / top) as f32).collect(),
        nodes.iter().map(|&n| [v[3 * n] / top, v[3 * n + 1] / top, v[3 * n + 2] / top].map(|c| c as f32)).collect(),
        ["x", "y", "z"][dir].into(),
    )
}

/// The static stress a study's loads make, cube by cube (the prestress).
fn static_stress(s: &mut Setup) -> (Vec<[f64; 6]>, Vec<String>) {
    let ds: Vec<[[f64; 6]; 6]> = s.materials.iter().map(d_matrix).collect();
    let mut u = s.u0.clone();
    let fe = eigen_forces(&s.model, &ds, &s.eps_heat);
    let rhs: Vec<f64> = s.f.iter().zip(&fe).map(|(a, b)| a + b).collect();
    let (_, ok, _) = solve_rhs(&mut s.model, None, &rhs, &mut u);
    let mut notes = vec![];
    if !ok {
        notes.push("The static solve for the load's stress did not fully settle: is the part held so it cannot spin or slide?".into());
    }
    (strain_stresses(&s.model, &ds, &u, &s.eps_heat), notes)
}

/// Natural frequencies of one part held by a study's supports (any kind),
/// with masses lumped at the cube corners. With `prestress`, the study's
/// loads are applied first and the stiffness their stress adds (tension) or
/// takes away (compression) is counted: a string or spoke under tension rings higher.
pub fn modes_study(body: &Body, study: &Study, count: usize, prestress: bool) -> Result<Modes, String> {
    if body.material.density <= 0.0 {
        return Err("the part has no density: its mass comes from its material".into());
    }
    let mut s = Setup::build(std::slice::from_ref(body), study, prestress)?;
    let mut notes = std::mem::take(&mut s.notes);
    if prestress {
        let (tensors, more) = static_stress(&mut s);
        notes.extend(more);
        s.model.geo = Some(geometric(&s.model, &tensors));
        let peak = tensors.iter().map(von_mises).fold(0.0, f64::max);
        notes.push(format!("Prestressed: the loads' stress (up to {} MPa) stiffens the part where it pulls and softens it where it pushes.", trim(peak)));
    }
    let model = &s.model;
    let count = count.clamp(1, 8);
    let h = model.grid.h;
    let cube = body.material.density * 1e-9 * h * h * h;
    let mut mass = vec![0.0; 3 * model.nodes];
    for (c, w) in model.corners.iter().zip(&model.weight) {
        for &n in c {
            for d in 0..3 {
                mass[3 * n + d] += cube * w / 8.0;
            }
        }
    }
    let b = |x: &[f64]| -> Vec<f64> {
        let mut y: Vec<f64> = x.iter().zip(&mass).map(|(a, m)| a * m).collect();
        model.mask(&mut y);
        y
    };
    let (mu, vecs, settled) = subspace(model, count, &b);
    if !settled {
        notes.push("The frequencies did not fully settle: they are approximate.".into());
    }
    if mu.iter().any(|m| *m <= 0.0) {
        notes.push("Some mode has no stiffness at all: the part can move freely (add supports), or the prestress buckles it.".into());
    }
    let nodes: Vec<usize> = (0..model.nodes).filter(|&n| model.used[n]).collect();
    let modes = mu
        .iter()
        .zip(&vecs)
        .map(|(&m, v)| {
            let (amount, shape, direction) = shape_of(model, v, &nodes);
            // ω² = 1/μ in (N/mm)/kg = 1000/s².
            let w2 = if m > 0.0 { 1.0 / m * 1000.0 } else { 0.0 };
            Mode { frequency: w2.sqrt() / std::f64::consts::TAU, direction, amount, shape }
        })
        .collect();
    notes.push(format!("{} cubes of {} mm, masses lumped at their corners.", model.cells.len(), crate::drawing::length_text(h)));
    Ok(Modes {
        cell: h,
        origin: model.grid.origin,
        cells: model.cells.iter().map(|c| c.map(|v| v as u32)).collect(),
        nodes: nodes.iter().map(|&n| model.node_at(n).map(|v| v as u32)).collect(),
        modes,
        mass: cube * model.weight.iter().sum::<f64>(),
        notes,
    })
}

/// One way the part buckles.
#[derive(Clone, Debug, Serialize)]
pub struct BucklingMode {
    /// The loads times this buckle it (below 1: they already do).
    pub factor: f64,
    pub direction: String,
    pub amount: Vec<f32>,
    pub shape: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Buckling {
    pub cell: f64,
    pub origin: [f64; 3],
    pub cells: Vec<[u32; 3]>,
    pub nodes: Vec<[u32; 3]>,
    pub modes: Vec<BucklingMode>,
    /// The largest stress the loads make as given (MPa), and the factor
    /// against yield, to compare: a part yields first if that is lower.
    pub max_stress: f64,
    pub yield_factor: f64,
    pub notes: Vec<String>,
}

/// Linear (eigenvalue) buckling of one part under a study's loads: the
/// factors on the loads at which it buckles, lowest first, with the shapes.
pub fn buckling(body: &Body, study: &Study, count: usize) -> Result<Buckling, String> {
    let mut s = Setup::build(std::slice::from_ref(body), study, true)?;
    let mut notes = std::mem::take(&mut s.notes);
    let (tensors, more) = static_stress(&mut s);
    notes.extend(more);
    let geo = geometric(&s.model, &tensors);
    let model = &s.model;
    let count = count.clamp(1, 6);
    // B = −K_G.
    let b = |x: &[f64]| -> Vec<f64> {
        let mut y: Vec<f64> = geo_apply(&model.corners, &geo, x).into_iter().map(|v| -v).collect();
        model.mask(&mut y);
        y
    };
    let (mu, vecs, settled) = subspace(model, count, &b);
    if !settled {
        notes.push("The buckling factors did not fully settle: they are approximate.".into());
    }
    let nodes: Vec<usize> = (0..model.nodes).filter(|&n| model.used[n]).collect();
    let mut modes: Vec<BucklingMode> = mu
        .iter()
        .zip(&vecs)
        .filter(|(m, _)| **m > 0.0)
        .map(|(&m, v)| {
            let (amount, shape, direction) = shape_of(model, v, &nodes);
            BucklingMode { factor: 1.0 / m, direction, amount, shape }
        })
        .collect();
    modes.sort_by(|a, b| a.factor.total_cmp(&b.factor));
    if modes.is_empty() {
        notes.push("The loads stretch the part everywhere it could buckle: it does not buckle under them (only if they were reversed).".into());
    }
    let max_stress = tensors.iter().map(von_mises).fold(0.0, f64::max);
    let yield_factor = if max_stress > 0.0 { body.material.yield_strength / max_stress } else { f64::INFINITY };
    if let Some(first) = modes.first() {
        if yield_factor < first.factor {
            notes.push(format!("It yields (at {} × the loads) before it buckles (at {} ×): strength, not buckling, limits it.", trim(yield_factor), trim(first.factor)));
        }
    }
    notes.push(format!("Linear buckling: {} cubes of {} mm; real parts buckle somewhat below these factors (imperfections), so keep a margin.", model.cells.len(), crate::drawing::length_text(model.grid.h)));
    Ok(Buckling {
        cell: model.grid.h,
        origin: model.grid.origin,
        cells: model.cells.iter().map(|c| c.map(|v| v as u32)).collect(),
        nodes: nodes.iter().map(|&n| model.node_at(n).map(|v| v as u32)).collect(),
        modes,
        max_stress,
        yield_factor,
        notes,
    })
}
