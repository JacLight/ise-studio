//! Flat sheet parts as plates: a part much thinner than it is wide (a
//! panel, a lid, a laser-cut plate) is solved on its mid-surface instead of
//! filled with cubes, which would need far more cubes than a millimetre of
//! thickness allows.
//!
//! The sheet's outline is laid on a grid of rectangles across its two long
//! axes (a rectangle is in when the sheet is under its middle), each a
//! Mindlin plate element with the MITC4 treatment of transverse shear
//! (Bathe & Dvorkin 1985), so thin plates do not lock. Each node turns and
//! moves out of plane (w, βx, βy); bending carries the load. The equations
//! are banded and solved directly. Results: the sag everywhere, the bending
//! stress at the surfaces (6M/t²), and the safety against yield.
//!
//! Units: mm, N, MPa. Limits: a flat sheet square to x, y or z; loads
//! square to it (pressure, or forces on faces); no in-plane stretching.

use super::*;

/// How an edge of a plate is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EdgeHold {
    /// No sag and no turn.
    Clamped,
    /// No sag; free to turn about the edge (hard simple support: no twist along it).
    Simple,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlateSupport {
    /// A side face of the sheet (its edge).
    pub face: Face,
    pub hold: EdgeHold,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlateStudy {
    pub supports: Vec<PlateSupport>,
    /// Pressure on the whole sheet (MPa), pushing along −normal of its thin axis's + side (down for a sheet lying flat).
    #[serde(default)]
    pub pressure: f64,
    /// Forces on faces (their component square to the sheet counts).
    #[serde(default)]
    pub loads: Vec<Load>,
    /// Rectangles along the sheet's longest side.
    #[serde(default = "plate_resolution")]
    pub resolution: usize,
}

fn plate_resolution() -> usize {
    48
}

#[derive(Clone, Debug, Serialize)]
pub struct PlateResult {
    /// The axis the sheet is thin along, its thickness and mid-plane (mm).
    pub normal_axis: usize,
    pub thickness: f64,
    pub mid: f64,
    /// The grid: corner (in the two in-plane axes, in order), rectangle sizes, counts.
    pub origin: [f64; 2],
    pub size: [f64; 2],
    pub counts: [usize; 2],
    /// Rectangles in the sheet: (i, j), and the largest surface von Mises (MPa).
    pub cells: Vec<([u32; 2], f32)>,
    /// Sag (mm, along the thin axis) at every grid node in use: (i, j, w).
    pub sag: Vec<([u32; 2], f32)>,
    pub max_sag: f64,
    pub max_sag_at: [f64; 3],
    pub max_stress: f64,
    pub max_stress_at: [f64; 3],
    pub yield_strength: f64,
    pub safety: f64,
    pub elements: usize,
    pub notes: Vec<String>,
}

/// Bilinear shape functions on [−1, 1]² and their derivatives (ξ, η), corners counter-clockwise from (−1, −1).
const QC: [[f64; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

fn shape(xi: f64, eta: f64) -> ([f64; 4], [f64; 4], [f64; 4]) {
    let n = QC.map(|c| 0.25 * (1.0 + c[0] * xi) * (1.0 + c[1] * eta));
    let dxi = QC.map(|c| 0.25 * c[0] * (1.0 + c[1] * eta));
    let deta = QC.map(|c| 0.25 * c[1] * (1.0 + c[0] * xi));
    (n, dxi, deta)
}

/// The 12 × 12 stiffness of a rectangle a × b (dofs per corner: w, βx, βy; u = z·β).
fn mitc4(e: f64, nu: f64, t: f64, a: f64, b: f64) -> [[f64; 12]; 12] {
    let db = e * t.powi(3) / (12.0 * (1.0 - nu * nu));
    let dbm = [[db, db * nu, 0.0], [db * nu, db, 0.0], [0.0, 0.0, db * (1.0 - nu) / 2.0]];
    let ks = 5.0 / 6.0 * e / (2.0 * (1.0 + nu)) * t;
    let (jx, jy) = (2.0 / a, 2.0 / b); // d/dx = jx·d/dξ
    let det = a * b / 4.0;
    let gp = 1.0 / 3f64.sqrt();
    let mut k = [[0.0; 12]; 12];
    // The shear strains γxz, γyz at a point, as rows over the 12 dofs.
    let gxz = |xi: f64, eta: f64| -> [f64; 12] {
        let (n, dxi, _) = shape(xi, eta);
        let mut r = [0.0; 12];
        for i in 0..4 {
            r[3 * i] = dxi[i] * jx;
            r[3 * i + 1] = n[i];
        }
        r
    };
    let gyz = |xi: f64, eta: f64| -> [f64; 12] {
        let (n, _, deta) = shape(xi, eta);
        let mut r = [0.0; 12];
        for i in 0..4 {
            r[3 * i] = deta[i] * jy;
            r[3 * i + 2] = n[i];
        }
        r
    };
    // MITC4: γxz from the edges η = ±1 (at ξ = 0), γyz from ξ = ±1 (at η = 0).
    let (ga, gc) = (gxz(0.0, 1.0), gxz(0.0, -1.0));
    let (gd, gb) = (gyz(1.0, 0.0), gyz(-1.0, 0.0));
    for xi in [-gp, gp] {
        for eta in [-gp, gp] {
            let (_, dxi, deta) = shape(xi, eta);
            // Curvatures κx = ∂βx/∂x, κy = ∂βy/∂y, κxy = ∂βx/∂y + ∂βy/∂x.
            let mut bb = [[0.0; 12]; 3];
            for i in 0..4 {
                bb[0][3 * i + 1] = dxi[i] * jx;
                bb[1][3 * i + 2] = deta[i] * jy;
                bb[2][3 * i + 1] = deta[i] * jy;
                bb[2][3 * i + 2] = dxi[i] * jx;
            }
            let sx: [f64; 12] = std::array::from_fn(|c| 0.5 * (1.0 + eta) * ga[c] + 0.5 * (1.0 - eta) * gc[c]);
            let sy: [f64; 12] = std::array::from_fn(|c| 0.5 * (1.0 + xi) * gd[c] + 0.5 * (1.0 - xi) * gb[c]);
            for p in 0..12 {
                for q in 0..12 {
                    let mut v = 0.0;
                    for r in 0..3 {
                        for s in 0..3 {
                            v += bb[r][p] * dbm[r][s] * bb[s][q];
                        }
                    }
                    v += ks * (sx[p] * sx[q] + sy[p] * sy[q]);
                    k[p][q] += v * det;
                }
            }
        }
    }
    k
}

/// Solves a banded symmetric positive definite system (lower band of half-width `bw`, row-major `n × (bw+1)`).
fn band_solve(mut a: Vec<f64>, n: usize, bw: usize, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let w = bw + 1;
    // a[i*w + (j − i + bw)] holds A[i][j] for i − bw ≤ j ≤ i.
    let at = |i: usize, j: usize| i * w + (j + bw - i);
    let scale = (0..n).map(|i| a[at(i, i)].abs()).fold(0.0, f64::max).max(1e-300);
    for j in 0..n {
        let lo = j.saturating_sub(bw);
        let mut d = a[at(j, j)];
        for k in lo..j {
            d -= a[at(j, k)] * a[at(j, k)];
        }
        if d <= 1e-13 * scale {
            return None;
        }
        let d = d.sqrt();
        a[at(j, j)] = d;
        for i in j + 1..(j + bw + 1).min(n) {
            let lo_i = i.saturating_sub(bw).max(lo);
            let mut s = a[at(i, j)];
            for k in lo_i..j {
                s -= a[at(i, k)] * a[at(j, k)];
            }
            a[at(i, j)] = s / d;
        }
    }
    for i in 0..n {
        let lo = i.saturating_sub(bw);
        let s: f64 = (lo..i).map(|k| a[at(i, k)] * b[k]).sum();
        b[i] = (b[i] - s) / a[at(i, i)];
    }
    for i in (0..n).rev() {
        let mut s = b[i];
        for k in i + 1..(i + bw + 1).min(n) {
            s -= a[at(k, i)] * b[k];
        }
        b[i] = s / a[at(i, i)];
    }
    Some(b)
}

/// A flat sheet part's plate study.
pub fn plate(positions: &[f32], triangles: &[u32], material: &Elastic, study: &PlateStudy) -> Result<PlateResult, String> {
    let (lo, hi) = bounds(positions, triangles);
    let ext: [f64; 3] = std::array::from_fn(|d| hi[d] - lo[d]);
    let thin = (0..3).min_by(|&a, &b| ext[a].total_cmp(&ext[b])).unwrap();
    let (u, v) = ((thin + 1) % 3, (thin + 2) % 3);
    let t = ext[thin];
    if t > 0.25 * ext[u].min(ext[v]) {
        return Err(format!("not a thin sheet: {} mm thick across {} × {} mm (a plate study wants under a quarter of its width; use the stress study)", trim(t), trim(ext[u]), trim(ext[v])));
    }
    if study.supports.is_empty() {
        return Err("hold an edge of the sheet: add a support".into());
    }
    let longest = ext[u].max(ext[v]);
    let res = study.resolution.clamp(8, 120);
    let n = [((ext[u] / longest * res as f64).round() as usize).max(2), ((ext[v] / longest * res as f64).round() as usize).max(2)];
    let size = [ext[u] / n[0] as f64, ext[v] / n[1] as f64];
    let mid = (lo[thin] + hi[thin]) / 2.0;
    // Which rectangles the sheet covers: a ray through each middle along the thin axis.
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    let tris: Vec<[[f64; 3]; 3]> = triangles.chunks_exact(3).map(|c| [p(c[0]), p(c[1]), p(c[2])]).collect();
    let covered = |x: f64, y: f64| -> bool {
        let mut hits = 0;
        for tri in &tris {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let det = (b[u] - a[u]) * (c[v] - a[v]) - (c[u] - a[u]) * (b[v] - a[v]);
            if det.abs() < 1e-18 {
                continue;
            }
            let s = ((x - a[u]) * (c[v] - a[v]) - (c[u] - a[u]) * (y - a[v])) / det;
            let r = ((b[u] - a[u]) * (y - a[v]) - (x - a[u]) * (b[v] - a[v])) / det;
            if s >= 0.0 && r >= 0.0 && s + r <= 1.0 {
                hits += 1;
            }
        }
        hits % 2 == 1 || hits >= 2
    };
    let nx = n[0] + 1;
    let node = |i: usize, j: usize| i + nx * j;
    let mut cells = vec![];
    for j in 0..n[1] {
        for i in 0..n[0] {
            let (x, y) = (lo[u] + (i as f64 + 0.5) * size[0] + 1.234e-7, lo[v] + (j as f64 + 0.5) * size[1] + 2.345e-7);
            if covered(x, y) {
                cells.push([i, j]);
            }
        }
    }
    if cells.is_empty() {
        return Err("the sheet covers no rectangle at this resolution: raise it".into());
    }
    let nodes = nx * (n[1] + 1);
    let mut used = vec![false; nodes];
    for &[i, j] in &cells {
        for (di, dj) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
            used[node(i + di, j + dj)] = true;
        }
    }
    let position = |id: usize| -> [f64; 3] {
        let mut q = [0.0; 3];
        q[u] = lo[u] + (id % nx) as f64 * size[0];
        q[v] = lo[v] + (id / nx) as f64 * size[1];
        q[thin] = mid;
        q
    };
    // Number the used nodes' dofs in grid order (row by row: a narrow band).
    let mut dof = vec![usize::MAX; nodes];
    let mut count = 0;
    for id in 0..nodes {
        if used[id] {
            dof[id] = count;
            count += 3;
        }
    }
    let ke = mitc4(material.e, material.nu, t, size[0], size[1]);
    let corner = |i: usize, j: usize| [node(i, j), node(i + 1, j), node(i + 1, j + 1), node(i, j + 1)];
    let mut bw = 0;
    for &[i, j] in &cells {
        let c = corner(i, j);
        let ds: Vec<usize> = c.iter().map(|&k| dof[k]).collect();
        bw = bw.max(ds.iter().max().unwrap() + 2 - ds.iter().min().unwrap());
    }
    let w = bw + 1;
    let mut a = vec![0.0; count * w];
    let mut f = vec![0.0; count];
    for &[i, j] in &cells {
        let c = corner(i, j);
        for p in 0..12 {
            for q in 0..12 {
                let (r, s) = (dof[c[p / 3]] + p % 3, dof[c[q / 3]] + q % 3);
                if s <= r {
                    a[r * w + (s + bw - r)] += ke[p][q];
                }
            }
        }
        // Pressure: each rectangle's push shared by its corners (along −thin).
        for &k in &c {
            f[dof[k]] -= study.pressure * size[0] * size[1] / 4.0;
        }
    }
    // Forces on faces: their part square to the sheet, shared by the nodes on the face.
    let ids = || (0..nodes).filter(|&k| used[k]);
    let h = size[0].max(size[1]);
    for (l, load) in study.loads.iter().enumerate() {
        let on = nodes_on_face(ids(), position, h, &load.face);
        if on.is_empty() {
            return Err(format!("load {} touches no part of the sheet", l + 1));
        }
        let push = load.pressure * load.face.area();
        let nrm = load.face.normal.map(|c| c / v_norm(load.face.normal).max(1e-12));
        let total = load.force[thin] - push * nrm[thin];
        for &k in &on {
            f[dof[k]] += total / on.len() as f64;
        }
    }
    // Holds: big springs on the held dofs (exact to 1e−10 of the plate's stiffness).
    let big = 1e10 * (0..count).map(|r| a[r * w + bw].abs()).fold(0.0, f64::max).max(1e-300);
    let mut held = 0;
    for (s, sp) in study.supports.iter().enumerate() {
        let on = nodes_on_face(ids(), position, h, &sp.face);
        if on.is_empty() {
            return Err(format!("support {} touches no edge of the sheet", s + 1));
        }
        // The edge runs square to the face's normal, in the sheet's plane.
        let along_u = sp.face.normal[u].abs() < sp.face.normal[v].abs();
        for &k in &on {
            let d = dof[k];
            a[d * w + bw] += big;
            match sp.hold {
                EdgeHold::Clamped => {
                    a[(d + 1) * w + bw] += big;
                    a[(d + 2) * w + bw] += big;
                }
                // An edge along u: no turn about v? βu (u = z·βu) stays zero along it.
                EdgeHold::Simple => a[(d + if along_u { 1 } else { 2 }) * w + bw] += big,
            }
            held += 1;
        }
    }
    let x = band_solve(a, count, bw, f).ok_or("the sheet can move freely: hold more of its edges")?;
    // Moments at each rectangle's middle, and the surface stress 6M/t².
    let db = material.e * t.powi(3) / (12.0 * (1.0 - material.nu * material.nu));
    let (_, dxi, deta) = shape(0.0, 0.0);
    let (jx, jy) = (2.0 / size[0], 2.0 / size[1]);
    let mut out_cells = vec![];
    let (mut max_stress, mut max_stress_at) = (0.0f64, [0.0; 3]);
    for &[i, j] in &cells {
        let c = corner(i, j);
        let (mut kx, mut ky, mut kxy) = (0.0, 0.0, 0.0);
        for k in 0..4 {
            let d = dof[c[k]];
            kx += dxi[k] * jx * x[d + 1];
            ky += deta[k] * jy * x[d + 2];
            kxy += deta[k] * jy * x[d + 1] + dxi[k] * jx * x[d + 2];
        }
        let (mx, my, mxy) = (db * (kx + material.nu * ky), db * (ky + material.nu * kx), db * (1.0 - material.nu) / 2.0 * kxy);
        let s = 6.0 / (t * t);
        let (sx, sy, txy) = (mx * s, my * s, mxy * s);
        let vm = (sx * sx - sx * sy + sy * sy + 3.0 * txy * txy).sqrt();
        if vm > max_stress {
            max_stress = vm;
            let mut q = position(c[0]);
            q[u] += size[0] / 2.0;
            q[v] += size[1] / 2.0;
            max_stress_at = q;
        }
        out_cells.push(([i as u32, j as u32], vm as f32));
    }
    let sag: Vec<([u32; 2], f32)> = (0..nodes).filter(|&k| used[k]).map(|k| ([(k % nx) as u32, (k / nx) as u32], x[dof[k]] as f32)).collect();
    let far = (0..nodes).filter(|&k| used[k]).max_by(|&p, &q| x[dof[p]].abs().total_cmp(&x[dof[q]].abs())).unwrap_or(0);
    let safety = if max_stress > 0.0 { material.yield_strength / max_stress } else { f64::INFINITY };
    let mut notes = vec![format!(
        "{} plate elements of {} × {} mm (Mindlin, MITC4) on the sheet's mid-plane, {} mm thick; {} edge nodes held. Stress is the bending stress at the surfaces (6M/t²), read at each element's middle.",
        cells.len(),
        trim(size[0]),
        trim(size[1]),
        trim(t),
        held
    )];
    if t > 0.1 * ext[u].min(ext[v]) {
        notes.push("Fairly thick for a plate (over a tenth of its width): shear counts, and the cube study may suit it better.".into());
    }
    Ok(PlateResult {
        normal_axis: thin,
        thickness: t,
        mid,
        origin: [lo[u], lo[v]],
        size,
        counts: n,
        cells: out_cells,
        sag,
        max_sag: x[dof[far]].abs(),
        max_sag_at: position(far),
        max_stress,
        max_stress_at,
        yield_strength: material.yield_strength,
        safety,
        elements: cells.len(),
        notes,
    })
}
