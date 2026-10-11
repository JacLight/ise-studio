//! Linear static stress analysis: where a part bends, how far, and how hard
//! its material is worked.
//!
//! The part is filled with cubes (voxels) cut from its exact outline, each a
//! hexahedral element with Wilson's incompatible modes so bending is not
//! stiffened by shear locking. Every element is the same cube, so one
//! element stiffness serves them all, and the equations are solved by
//! conjugate gradients without ever forming the global matrix. Supports fix
//! the nodes on chosen faces; loads spread a force over the nodes of others.
//! Results: displacement at every node, von Mises stress in every element,
//! and the safety factor against the material's yield strength.
//!
//! Units: millimetres, newtons, megapascals (N/mm²).

use rayon::prelude::*;
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

/// Elastic modulus (MPa), Poisson's ratio and yield strength (MPa).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Elastic {
    pub e: f64,
    pub nu: f64,
    pub yield_strength: f64,
}

/// Typical properties by material name: design guidance, not a datasheet.
pub fn elastic(material: &str) -> Option<Elastic> {
    let m = material.to_lowercase();
    let (e, nu, y) = if m.contains("stainless") {
        (193_000.0, 0.29, 215.0)
    } else if m.contains("steel") {
        (200_000.0, 0.29, 250.0)
    } else if m.contains("alumin") {
        (69_000.0, 0.33, 240.0)
    } else if m.contains("titan") {
        (114_000.0, 0.34, 880.0)
    } else if m.contains("brass") {
        (100_000.0, 0.34, 200.0)
    } else if m.contains("copper") {
        (117_000.0, 0.34, 70.0)
    } else if m.contains("pla") {
        (3_500.0, 0.36, 50.0)
    } else if m.contains("petg") {
        (2_100.0, 0.38, 50.0)
    } else if m.contains("abs") {
        (2_200.0, 0.35, 40.0)
    } else if m.contains("nylon") {
        (2_700.0, 0.39, 70.0)
    } else if m.contains("acetal") || m.contains("pom") {
        (2_800.0, 0.35, 65.0)
    } else if m.contains("oak") {
        (11_000.0, 0.3, 40.0)
    } else if m.contains("ply") {
        (9_000.0, 0.3, 30.0)
    } else if m.contains("mdf") {
        (3_500.0, 0.25, 15.0)
    } else if m.contains("glass") {
        (70_000.0, 0.22, 33.0)
    } else if m.contains("concrete") {
        (30_000.0, 0.2, 3.0)
    } else {
        return None;
    };
    Some(Elastic { e, nu, yield_strength: y })
}

/// A flat face as picked: a point on it, its outward normal, and its
/// triangles (x y z per vertex, 9 numbers per triangle), in part coordinates.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Face {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub triangles: Vec<f64>,
}

/// A face as kept with a study: where on the part it sits (fractions of the
/// part's bounds: its middle, and a point on one of its triangles) and which
/// way it faces, so it is found again after the part changes.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct FacePick {
    pub at: [f64; 3],
    pub probe: [f64; 3],
    pub normal: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct LoadPick {
    pub face: FacePick,
    /// Newtons.
    pub magnitude: f64,
    /// "into" (the face) or an axis: "-z", "+z", "-x", "+x", "-y", "+y".
    pub direction: String,
}

/// A study as kept with the shape.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct StudySpec {
    /// The part of an assembly it is on (none: the whole solid).
    #[serde(default)]
    pub part: Option<String>,
    #[serde(default)]
    pub supports: Vec<FacePick>,
    #[serde(default)]
    pub loads: Vec<LoadPick>,
    /// Heat: faces held at temperatures, heat coming in, air cooling.
    #[serde(default)]
    pub holds: Vec<HoldPick>,
    #[serde(default)]
    pub heats: Vec<HeatPick>,
    #[serde(default)]
    pub convection: Option<Convection>,
    #[serde(default = "default_resolution")]
    pub resolution: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct HoldPick {
    pub face: FacePick,
    /// °C.
    pub temperature: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct HeatPick {
    pub face: FacePick,
    pub watts: f64,
}

impl StudySpec {
    /// Nothing set: the study can go.
    pub fn is_empty(&self) -> bool {
        self.supports.is_empty() && self.loads.is_empty() && self.holds.is_empty() && self.heats.is_empty() && self.convection.is_none()
    }
}

impl LoadPick {
    /// The force (N) on a face facing `normal`.
    pub fn force(&self, normal: [f64; 3]) -> [f64; 3] {
        let unit = match self.direction.as_str() {
            "-z" => [0.0, 0.0, -1.0],
            "+z" => [0.0, 0.0, 1.0],
            "-x" => [-1.0, 0.0, 0.0],
            "+x" => [1.0, 0.0, 0.0],
            "-y" => [0.0, -1.0, 0.0],
            "+y" => [0.0, 1.0, 0.0],
            _ => normal.map(|c| -c),
        };
        unit.map(|c| c * self.magnitude)
    }
}

/// A joint's seat on a part: the material nearest an axis (the skin of a
/// bore, or the core of a shaft along it).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Bore {
    pub at: [f64; 3],
    pub axis: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BoreLoad {
    pub bore: Bore,
    pub force: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Load {
    pub face: Face,
    /// Total force (N), spread evenly over the face.
    pub force: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Study {
    pub supports: Vec<Face>,
    pub loads: Vec<Load>,
    /// Bores (a joint's seat: a pin's hole or a shaft) held still.
    #[serde(default)]
    pub bore_supports: Vec<Bore>,
    /// Forces spread over bores.
    #[serde(default)]
    pub bore_loads: Vec<BoreLoad>,
    /// Cubes along the part's longest side.
    #[serde(default = "default_resolution")]
    pub resolution: usize,
}

fn default_resolution() -> usize {
    48
}

#[derive(Clone, Debug, Serialize)]
pub struct Result_ {
    /// The cube size (mm) and the grid's corner and counts.
    pub cell: f64,
    pub origin: [f64; 3],
    pub counts: [usize; 3],
    /// Filled cubes: their (i, j, k) and von Mises stress (MPa).
    pub cells: Vec<([u32; 3], f32)>,
    /// Displacement (mm) at each corner of the grid that the part uses: (i, j, k) and (dx, dy, dz).
    pub displacements: Vec<([u32; 3], [f32; 3])>,
    pub max_stress: f64,
    pub max_displacement: f64,
    pub yield_strength: f64,
    /// Yield strength over the largest stress.
    pub safety: f64,
    pub elements: usize,
    pub iterations: usize,
    pub notes: Vec<String>,
}

// ---- The element ---------------------------------------------------------------

const CORNERS: [[f64; 3]; 8] = [[-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0], [-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]];

fn d_matrix(m: &Elastic) -> [[f64; 6]; 6] {
    let (e, nu) = (m.e, m.nu);
    let l = e * nu / ((1.0 + nu) * (1.0 - 2.0 * nu));
    let g = e / (2.0 * (1.0 + nu));
    let mut d = [[0.0; 6]; 6];
    for i in 0..3 {
        for j in 0..3 {
            d[i][j] = l;
        }
        d[i][i] = l + 2.0 * g;
        d[i + 3][i + 3] = g;
    }
    d
}

/// Strain from nodal displacements (6×24) at natural coordinates, cube side h.
fn b_matrix(xi: [f64; 3], h: f64) -> [[f64; 24]; 6] {
    let mut b = [[0.0; 24]; 6];
    for (n, c) in CORNERS.iter().enumerate() {
        // dN/dξ for N = ⅛(1+ξξn)(1+ηηn)(1+ζζn), times 2/h for x.
        let f = |k: usize| 1.0 + xi[k] * c[k];
        let d = [c[0] * f(1) * f(2) / 8.0 * 2.0 / h, c[1] * f(0) * f(2) / 8.0 * 2.0 / h, c[2] * f(0) * f(1) / 8.0 * 2.0 / h];
        let k = 3 * n;
        b[0][k] = d[0];
        b[1][k + 1] = d[1];
        b[2][k + 2] = d[2];
        b[3][k] = d[1];
        b[3][k + 1] = d[0];
        b[4][k + 1] = d[2];
        b[4][k + 2] = d[1];
        b[5][k] = d[2];
        b[5][k + 2] = d[0];
    }
    b
}

/// Strain from the nine incompatible modes (1−ξ², 1−η², 1−ζ² per component).
fn g_matrix(xi: [f64; 3], h: f64) -> [[f64; 9]; 6] {
    let mut g = [[0.0; 9]; 6];
    for m in 0..3 {
        // Mode m varies along axis m: d(1−ξm²)/dxm = −2ξm·2/h.
        let dm = -2.0 * xi[m] * 2.0 / h;
        for comp in 0..3 {
            let col = 3 * m + comp;
            let mut grad = [0.0; 3];
            grad[m] = dm;
            match comp {
                0 => {
                    g[0][col] = grad[0];
                    g[3][col] = grad[1];
                    g[5][col] = grad[2];
                }
                1 => {
                    g[1][col] = grad[1];
                    g[3][col] = grad[0];
                    g[4][col] = grad[2];
                }
                _ => {
                    g[2][col] = grad[2];
                    g[4][col] = grad[1];
                    g[5][col] = grad[0];
                }
            }
        }
    }
    g
}

/// The condensed 24×24 stiffness of a cube of side h.
fn element_stiffness(m: &Elastic, h: f64) -> Vec<f64> {
    let d = d_matrix(m);
    let gp = 1.0 / 3f64.sqrt();
    let w = (h / 2.0).powi(3); // Jacobian determinant × unit weights
    let mut kuu = vec![0.0; 24 * 24];
    let mut kua = vec![0.0; 24 * 9];
    let mut kaa = vec![0.0; 81];
    for c in CORNERS {
        let xi = [c[0] * gp, c[1] * gp, c[2] * gp];
        let b = b_matrix(xi, h);
        let g = g_matrix(xi, h);
        // D·B and D·G.
        let db: Vec<[f64; 24]> = (0..6).map(|r| std::array::from_fn(|j| (0..6).map(|k| d[r][k] * b[k][j]).sum())).collect();
        let dg: Vec<[f64; 9]> = (0..6).map(|r| std::array::from_fn(|j| (0..6).map(|k| d[r][k] * g[k][j]).sum())).collect();
        for i in 0..24 {
            for j in 0..24 {
                kuu[i * 24 + j] += w * (0..6).map(|r| b[r][i] * db[r][j]).sum::<f64>();
            }
            for j in 0..9 {
                kua[i * 9 + j] += w * (0..6).map(|r| b[r][i] * dg[r][j]).sum::<f64>();
            }
        }
        for i in 0..9 {
            for j in 0..9 {
                kaa[i * 9 + j] += w * (0..6).map(|r| g[r][i] * dg[r][j]).sum::<f64>();
            }
        }
    }
    // K = Kuu − Kua Kaa⁻¹ Kau.
    let inv = invert(&kaa, 9);
    let mut k = kuu;
    for i in 0..24 {
        for j in 0..24 {
            let mut s = 0.0;
            for a in 0..9 {
                for bb in 0..9 {
                    s += kua[i * 9 + a] * inv[a * 9 + bb] * kua[j * 9 + bb];
                }
            }
            k[i * 24 + j] -= s;
        }
    }
    k
}

fn invert(a: &[f64], n: usize) -> Vec<f64> {
    let mut m = a.to_vec();
    let mut inv: Vec<f64> = (0..n * n).map(|i| if i / n == i % n { 1.0 } else { 0.0 }).collect();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| m[i * n + c].abs().total_cmp(&m[j * n + c].abs())).unwrap();
        for k in 0..n {
            m.swap(c * n + k, p * n + k);
            inv.swap(c * n + k, p * n + k);
        }
        let d = m[c * n + c];
        for k in 0..n {
            m[c * n + k] /= d;
            inv[c * n + k] /= d;
        }
        for r in 0..n {
            if r != c {
                let f = m[r * n + c];
                for k in 0..n {
                    m[r * n + k] -= f * m[c * n + k];
                    inv[r * n + k] -= f * inv[c * n + k];
                }
            }
        }
    }
    inv
}

// ---- Voxels --------------------------------------------------------------------

struct Grid {
    h: f64,
    origin: [f64; 3],
    n: [usize; 3],
    /// How much of each cube is material (0–1).
    fill: Vec<f32>,
}

impl Grid {
    fn cell(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.n[0] * (j + self.n[1] * k)
    }
    fn node(&self, i: usize, j: usize, k: usize) -> usize {
        i + (self.n[0] + 1) * (j + (self.n[1] + 1) * k)
    }
    fn node_at(&self, id: usize) -> [usize; 3] {
        let nx = self.n[0] + 1;
        let ny = self.n[1] + 1;
        [id % nx, (id / nx) % ny, id / (nx * ny)]
    }
    fn position(&self, id: usize) -> [f64; 3] {
        let [i, j, k] = self.node_at(id);
        [self.origin[0] + i as f64 * self.h, self.origin[1] + j as f64 * self.h, self.origin[2] + k as f64 * self.h]
    }
    /// The 8 corner nodes of a cell, in the element's corner order.
    fn corners(&self, i: usize, j: usize, k: usize) -> [usize; 8] {
        CORNERS.map(|c| self.node(i + (c[0] > 0.0) as usize, j + (c[1] > 0.0) as usize, k + (c[2] > 0.0) as usize))
    }
}

/// Cubes less full than this are left out (slivers add nothing but trouble).
const MIN_FILL: f32 = 0.12;

/// How full of material each cube is (rays along x, by parity).
fn voxelize(positions: &[f32], triangles: &[u32], resolution: usize) -> Grid {
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for &t in triangles {
        let q = p(t);
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let longest = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    let h = longest / resolution.max(4) as f64;
    let n = [0, 1, 2].map(|k| (((hi[k] - lo[k]) / h) - 1e-9).ceil().max(1.0) as usize);
    // Centre the grid on the part.
    let origin = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0 - n[k] as f64 * h / 2.0);
    let mut grid = Grid { h, origin, n, fill: vec![0.0; n[0] * n[1] * n[2]] };
    // Triangles binned by the (j, k) columns their y–z boxes cover.
    let mut bins: Vec<Vec<usize>> = vec![vec![]; n[1] * n[2]];
    let tris: Vec<[[f64; 3]; 3]> = triangles.chunks_exact(3).map(|t| [p(t[0]), p(t[1]), p(t[2])]).collect();
    for (ti, t) in tris.iter().enumerate() {
        let col = |v: f64, k: usize| ((v - origin[k]) / h - 0.5).floor();
        let (y0, y1) = (t.iter().map(|q| q[1]).fold(f64::INFINITY, f64::min), t.iter().map(|q| q[1]).fold(f64::NEG_INFINITY, f64::max));
        let (z0, z1) = (t.iter().map(|q| q[2]).fold(f64::INFINITY, f64::min), t.iter().map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max));
        let (ja, jb) = ((col(y0, 1) as i64).max(0), (col(y1, 1) as i64 + 1).min(n[1] as i64 - 1));
        let (ka, kb) = ((col(z0, 2) as i64).max(0), (col(z1, 2) as i64 + 1).min(n[2] as i64 - 1));
        for k in ka..=kb {
            for j in ja..=jb {
                bins[j as usize + n[1] * k as usize].push(ti);
            }
        }
    }
    // Each column: 4 × 4 rays along x; each cube's fill is the share of its
    // length inside the part, averaged over the rays (exact along x).
    const RAYS: usize = 4;
    for k in 0..n[2] {
        for j in 0..n[1] {
            for (ry, rz) in (0..RAYS).flat_map(|a| (0..RAYS).map(move |b| (a, b))) {
                // Nudged off any edge it might graze.
                let y = origin[1] + (j as f64 + (ry as f64 + 0.5) / RAYS as f64) * h + h * 1.234e-7;
                let z = origin[2] + (k as f64 + (rz as f64 + 0.5) / RAYS as f64) * h + h * 2.345e-7;
                let mut xs: Vec<f64> = vec![];
                for &ti in &bins[j + n[1] * k] {
                    let [a, b, c] = tris[ti];
                    let det = (b[1] - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (b[2] - a[2]);
                    if det.abs() < 1e-18 {
                        continue;
                    }
                    let u = ((y - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (z - a[2])) / det;
                    let v = ((b[1] - a[1]) * (z - a[2]) - (y - a[1]) * (b[2] - a[2])) / det;
                    if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                        xs.push(a[0] + u * (b[0] - a[0]) + v * (c[0] - a[0]));
                    }
                }
                xs.sort_by(f64::total_cmp);
                for pair in xs.chunks_exact(2) {
                    let (i0, i1) = (((pair[0] - origin[0]) / h).floor().max(0.0) as usize, (((pair[1] - origin[0]) / h).ceil() as usize).min(n[0]));
                    for i in i0..i1 {
                        let (x0, x1) = (origin[0] + i as f64 * h, origin[0] + (i + 1) as f64 * h);
                        let inside = (pair[1].min(x1) - pair[0].max(x0)).max(0.0) / h;
                        let c = grid.cell(i, j, k);
                        grid.fill[c] += (inside / (RAYS * RAYS) as f64) as f32;
                    }
                }
            }
        }
    }
    grid
}

/// How much of the part each node serves (the cubes around it, by fill): a
/// face load shared by these weights spreads evenly over the face's area,
/// instead of piling onto its corners.
fn node_shares(nodes: usize, corners: &[[usize; 8]], weight: &[f64]) -> Vec<f64> {
    let mut share = vec![0.0; nodes];
    for (c, w) in corners.iter().zip(weight) {
        for &n in c {
            share[n] += w;
        }
    }
    share
}

/// Spreads `total` over `on` by their shares.
fn spread(on: &[usize], share: &[f64], total: f64) -> Vec<(usize, f64)> {
    let sum: f64 = on.iter().map(|&n| share[n]).sum();
    on.iter().map(|&n| (n, if sum > 0.0 { total * share[n] / sum } else { total / on.len() as f64 })).collect()
}

/// Grid nodes the part uses nearest an axis: within a cube and a half of the closest.
fn nodes_on_bore(grid: &Grid, used: &[bool], bore: &Bore) -> Vec<usize> {
    let l = (bore.axis[0] * bore.axis[0] + bore.axis[1] * bore.axis[1] + bore.axis[2] * bore.axis[2]).sqrt().max(1e-12);
    let u = bore.axis.map(|c| c / l);
    let off = |id: usize| {
        let q = grid.position(id);
        let d = [q[0] - bore.at[0], q[1] - bore.at[1], q[2] - bore.at[2]];
        let t = d[0] * u[0] + d[1] * u[1] + d[2] * u[2];
        ((d[0] - t * u[0]).powi(2) + (d[1] - t * u[1]).powi(2) + (d[2] - t * u[2]).powi(2)).sqrt()
    };
    let near: Vec<(usize, f64)> = (0..used.len()).filter(|&id| used[id]).map(|id| (id, off(id))).collect();
    let closest = near.iter().map(|(_, d)| *d).fold(f64::INFINITY, f64::min);
    near.into_iter().filter(|(_, d)| *d <= closest + 1.5 * grid.h).map(|(id, _)| id).collect()
}

/// Grid nodes the part uses that lie on a picked face.
fn nodes_on(grid: &Grid, used: &[bool], face: &Face) -> Vec<usize> {
    let n = face.normal;
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
    let n = n.map(|c| c / len);
    let tris: Vec<[[f64; 3]; 3]> = face.triangles.chunks_exact(9).map(|t| [[t[0], t[1], t[2]], [t[3], t[4], t[5]], [t[6], t[7], t[8]]]).collect();
    let margin = grid.h * 0.55;
    let inside = |q: [f64; 3]| {
        tris.iter().any(|t| {
            // Project onto the triangle's plane, then test with a margin of half a cube.
            let e = |a: [f64; 3], b: [f64; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
            // Edges' outward sides, whichever way the triangle winds.
            let wind = dot(cross(e(t[0], t[1]), e(t[0], t[2])), n).signum();
            (0..3).all(|k| {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let edge = e(a, b);
                let out = cross(edge, n).map(|c| c * wind);
                let l = dot(out, out).sqrt().max(1e-12);
                dot(e(a, q), out) / l <= margin
            })
        })
    };
    (0..used.len())
        .filter(|&id| used[id])
        .filter(|&id| {
            let q = grid.position(id);
            let d = (q[0] - face.point[0]) * n[0] + (q[1] - face.point[1]) * n[1] + (q[2] - face.point[2]) * n[2];
            d.abs() <= margin && inside(q)
        })
        .collect()
}

// ---- Solving -------------------------------------------------------------------

/// The cubes of a part, their stiffness, and which nodes its supports hold.
struct Model {
    grid: Grid,
    corners: Vec<[usize; 8]>,
    cells: Vec<[usize; 3]>,
    nodes: usize,
    used: Vec<bool>,
    fixed: Vec<bool>,
    ke: Vec<f64>,
    /// Each cube's share of material (scales its stiffness and mass).
    weight: Vec<f64>,
    diag: Vec<f64>,
}

impl Model {
    fn build(positions: &[f32], triangles: &[u32], material: &Elastic, supports: &[Face], bores: &[Bore], resolution: usize) -> Result<Model, String> {
        if supports.is_empty() && bores.is_empty() {
            return Err("add a support: a face the part is held by".into());
        }
        let grid = voxelize(positions, triangles, resolution.clamp(8, 160));
        let cells: Vec<[usize; 3]> = (0..grid.n[2]).flat_map(|k| (0..grid.n[1]).flat_map(move |j| (0..grid.n[0]).map(move |i| [i, j, k]))).filter(|&[i, j, k]| grid.fill[grid.cell(i, j, k)] >= MIN_FILL).collect();
        if cells.is_empty() {
            return Err("the part is too thin for this resolution: raise it".into());
        }
        let nodes = (grid.n[0] + 1) * (grid.n[1] + 1) * (grid.n[2] + 1);
        let mut used = vec![false; nodes];
        let corners: Vec<[usize; 8]> = cells.iter().map(|&[i, j, k]| grid.corners(i, j, k)).collect();
        for c in &corners {
            for &n in c {
                used[n] = true;
            }
        }
        let ke = element_stiffness(material, grid.h);
        let mut fixed = vec![false; 3 * nodes];
        for (s, face) in supports.iter().enumerate() {
            let on = nodes_on(&grid, &used, face);
            if on.is_empty() {
                return Err(format!("support {} touches no part of the part at this resolution", s + 1));
            }
            for n in on {
                fixed[3 * n..3 * n + 3].fill(true);
            }
        }
        for (s, bore) in bores.iter().enumerate() {
            let on = nodes_on_bore(&grid, &used, bore);
            if on.is_empty() {
                return Err(format!("bore {} touches no part of the part", s + 1));
            }
            for n in on {
                fixed[3 * n..3 * n + 3].fill(true);
            }
        }
        let weight: Vec<f64> = cells.iter().map(|&[i, j, k]| (grid.fill[grid.cell(i, j, k)] as f64).min(1.0)).collect();
        let mut diag = vec![0.0; 3 * nodes];
        for (c, w) in corners.iter().zip(&weight) {
            for (r, &n) in c.iter().enumerate() {
                for d in 0..3 {
                    diag[3 * n + d] += w * ke[(3 * r + d) * 24 + 3 * r + d];
                }
            }
        }
        Ok(Model { grid, corners, cells, nodes, used, fixed, ke, weight, diag })
    }

    fn free(&self, i: usize) -> bool {
        self.used[i / 3] && !self.fixed[i]
    }

    fn mask(&self, v: &mut [f64]) {
        v.iter_mut().enumerate().for_each(|(i, x)| {
            if !self.free(i) {
                *x = 0.0
            }
        });
    }

    /// K·x, element by element, in parallel (held and unused entries zero).
    fn apply(&self, x: &[f64]) -> Vec<f64> {
        let n3 = 3 * self.nodes;
        let ke = &self.ke;
        let mut y = self
            .corners
            .par_chunks(4096)
            .zip(self.weight.par_chunks(4096))
            .fold(
                || vec![0.0; n3],
                |mut y, (chunk, weights)| {
                    for (c, w) in chunk.iter().zip(weights) {
                        let mut xe = [0.0; 24];
                        for (a, &n) in c.iter().enumerate() {
                            xe[3 * a..3 * a + 3].copy_from_slice(&x[3 * n..3 * n + 3]);
                        }
                        for (r, &n) in c.iter().enumerate() {
                            for d in 0..3 {
                                let row = &ke[(3 * r + d) * 24..(3 * r + d + 1) * 24];
                                y[3 * n + d] += w * row.iter().zip(&xe).map(|(a, b)| a * b).sum::<f64>();
                            }
                        }
                    }
                    y
                },
            )
            .reduce(|| vec![0.0; n3], |mut a, b| {
                a.iter_mut().zip(&b).for_each(|(x, y)| *x += y);
                a
            });
        self.mask(&mut y);
        y
    }

    /// Solves K·u = f by Jacobi-preconditioned conjugate gradients from `u`;
    /// returns the iterations and whether it settled.
    fn cg(&self, f: &[f64], u: &mut [f64], tol: f64) -> (usize, bool) {
        let mut r: Vec<f64> = f.to_vec();
        self.mask(&mut r);
        let fnorm = r.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
        let ku = self.apply(u);
        r.iter_mut().zip(&ku).for_each(|(a, b)| *a -= b);
        let precondition = |r: &[f64]| -> Vec<f64> { r.iter().zip(&self.diag).enumerate().map(|(i, (v, d))| if self.free(i) { v / d } else { 0.0 }).collect() };
        let mut z = precondition(&r);
        let mut p = z.clone();
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let limit = (20 * 3 * self.cells.len().max(100)).min(40_000);
        let mut it = 0;
        while it < limit {
            if r.iter().map(|v| v * v).sum::<f64>().sqrt() < tol * fnorm {
                return (it, true);
            }
            let q = self.apply(&p);
            let pq: f64 = p.iter().zip(&q).map(|(a, b)| a * b).sum();
            if pq.abs() < 1e-300 {
                break;
            }
            let alpha = rz / pq;
            u.iter_mut().zip(&p).for_each(|(x, y)| *x += alpha * y);
            r.iter_mut().zip(&q).for_each(|(x, y)| *x -= alpha * y);
            it += 1;
            z = precondition(&r);
            let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            let beta = rz2 / rz;
            rz = rz2;
            p.iter_mut().zip(&z).for_each(|(x, y)| *x = y + beta * *x);
        }
        (it, r.iter().map(|v| v * v).sum::<f64>().sqrt() < tol * 1000.0 * fnorm)
    }
}

/// Runs a study on a closed mesh (part coordinates, mm).
pub fn solve(positions: &[f32], triangles: &[u32], material: &Elastic, study: &Study) -> Result<Result_, String> {
    if study.loads.is_empty() && study.bore_loads.is_empty() {
        return Err("add a load: a face and the force on it".into());
    }
    let model = Model::build(positions, triangles, material, &study.supports, &study.bore_supports, study.resolution)?;
    let (grid, cells, corners, nodes, used, fixed) = (&model.grid, &model.cells, &model.corners, model.nodes, &model.used, &model.fixed);
    let h = grid.h;
    let mut notes = vec![];
    let mut f = vec![0.0; 3 * nodes];
    let share = node_shares(nodes, corners, &model.weight);
    for (l, load) in study.loads.iter().enumerate() {
        let on: Vec<usize> = nodes_on(grid, used, &load.face).into_iter().filter(|&n| !fixed[3 * n]).collect();
        if on.is_empty() {
            return Err(format!("load {} touches no free part of the part", l + 1));
        }
        for d in 0..3 {
            for (n, v) in spread(&on, &share, load.force[d]) {
                f[3 * n + d] += v;
            }
        }
    }
    for (l, load) in study.bore_loads.iter().enumerate() {
        let on: Vec<usize> = nodes_on_bore(grid, used, &load.bore).into_iter().filter(|&n| !fixed[3 * n]).collect();
        if on.is_empty() {
            return Err(format!("bore load {} touches no free part of the part", l + 1));
        }
        for d in 0..3 {
            for (n, v) in spread(&on, &share, load.force[d]) {
                f[3 * n + d] += v;
            }
        }
    }
    let mut u = vec![0.0; 3 * nodes];
    let (iterations, converged) = model.cg(&f, &mut u, 1e-9);
    if !converged {
        notes.push("The solve did not fully settle: is the part held so it cannot spin or slide? Results are approximate.".into());
    }
    // Stress at each cube's centre: σ = D·B(0)·uₑ (the extra modes add no strain there).
    let d = d_matrix(material);
    let b0 = b_matrix([0.0; 3], h);
    let stresses: Vec<f64> = corners
        .par_iter()
        .map(|c| {
            let mut ue = [0.0; 24];
            for (a, &n) in c.iter().enumerate() {
                ue[3 * a..3 * a + 3].copy_from_slice(&u[3 * n..3 * n + 3]);
            }
            let eps: [f64; 6] = std::array::from_fn(|r| (0..24).map(|j| b0[r][j] * ue[j]).sum());
            let s: [f64; 6] = std::array::from_fn(|r| (0..6).map(|k| d[r][k] * eps[k]).sum());
            (0.5 * ((s[0] - s[1]).powi(2) + (s[1] - s[2]).powi(2) + (s[2] - s[0]).powi(2)) + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5])).sqrt()
        })
        .collect();
    let max_stress = stresses.iter().cloned().fold(0.0, f64::max);
    let mut max_displacement: f64 = 0.0;
    let displacements: Vec<([u32; 3], [f32; 3])> = (0..nodes)
        .filter(|&n| used[n])
        .map(|n| {
            let v = [u[3 * n], u[3 * n + 1], u[3 * n + 2]];
            max_displacement = max_displacement.max((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt());
            (grid.node_at(n).map(|c| c as u32), v.map(|c| c as f32))
        })
        .collect();
    notes.push(format!("{} cubes of {} mm; stress is read at each cube's centre, so a sharp inside corner shows lower than its true peak.", cells.len(), crate::drawing::length_text(h)));
    Ok(Result_ {
        cell: h,
        origin: grid.origin,
        counts: grid.n,
        cells: cells.iter().zip(&stresses).map(|(c, s)| (c.map(|v| v as u32), *s as f32)).collect(),
        displacements,
        max_stress,
        max_displacement,
        yield_strength: material.yield_strength,
        safety: if max_stress > 0.0 { material.yield_strength / max_stress } else { f64::INFINITY },
        elements: cells.len(),
        iterations,
        notes,
    })
}

/// The stress at the cube nearest a point (MPa), for checking results.
pub fn stress_at(r: &Result_, point: [f64; 3]) -> Option<f64> {
    let idx = [0, 1, 2].map(|k| ((point[k] - r.origin[k]) / r.cell - 0.5).round() as i64);
    r.cells.iter().find(|(c, _)| (0..3).all(|k| c[k] as i64 == idx[k])).map(|(_, s)| *s as f64)
}

/// The displacement at the grid node nearest a point (mm).
pub fn displacement_at(r: &Result_, point: [f64; 3]) -> Option<[f64; 3]> {
    let idx = [0, 1, 2].map(|k| ((point[k] - r.origin[k]) / r.cell).round() as i64);
    r.displacements.iter().find(|(c, _)| (0..3).all(|k| c[k] as i64 == idx[k])).map(|(_, d)| d.map(|v| v as f64))
}

// ---- Natural frequencies -------------------------------------------------------

/// One way the part vibrates.
#[derive(Clone, Debug, Serialize)]
pub struct Mode {
    /// Hz.
    pub frequency: f64,
    /// Which way it mostly moves: "x", "y" or "z".
    pub direction: String,
    /// Per filled cube (in `Modes::cells` order): how far it moves, 0–1.
    pub amount: Vec<f32>,
    /// Per node (in `Modes::nodes` order): the shape, largest movement 1.
    pub shape: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Modes {
    pub cell: f64,
    pub origin: [f64; 3],
    pub cells: Vec<[u32; 3]>,
    pub nodes: Vec<[u32; 3]>,
    pub modes: Vec<Mode>,
    pub mass: f64,
    pub notes: Vec<String>,
}

/// Eigenvalues and vectors of a small symmetric matrix (Jacobi rotations), ascending.
fn symmetric_eigen(mut a: Vec<Vec<f64>>) -> (Vec<f64>, Vec<Vec<f64>>) {
    let n = a.len();
    let mut v: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    for _ in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| a[i][j] * a[i][j]).sum();
        if off < 1e-24 * (0..n).map(|i| a[i][i] * a[i][i]).sum::<f64>().max(1e-300) {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                if a[p][q].abs() < 1e-300 {
                    continue;
                }
                let th = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = th.signum() / (th.abs() + (th * th + 1.0).sqrt());
                let t = if th == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k][p], a[k][q]);
                    a[k][p] = c * akp - s * akq;
                    a[k][q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p][k], a[q][k]);
                    a[p][k] = c * apk - s * aqk;
                    a[q][k] = s * apk + c * aqk;
                }
                for row in v.iter_mut() {
                    let (vkp, vkq) = (row[p], row[q]);
                    row[p] = c * vkp - s * vkq;
                    row[q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    (order.iter().map(|&i| a[i][i]).collect(), order.iter().map(|&i| (0..n).map(|k| v[k][i]).collect()).collect())
}

/// The lowest natural frequencies of a part held at its supports, with the
/// mode shapes: subspace iteration on K·φ = ω²·M·φ (lumped masses), each
/// step solving K·z = M·x by conjugate gradients from the last answer.
/// `density` in kg/m³.
pub fn modes(positions: &[f32], triangles: &[u32], material: &Elastic, density: f64, supports: &[Face], count: usize, resolution: usize) -> Result<Modes, String> {
    let model = Model::build(positions, triangles, material, supports, &[], resolution)?;
    let count = count.clamp(1, 8);
    let p = count + 3;
    let n3 = 3 * model.nodes;
    let h = model.grid.h;
    // Lumped mass: each cube's mass (kg) shared by its 8 corners.
    let cube = density * 1e-9 * h * h * h;
    let mut mass = vec![0.0; n3];
    for (c, w) in model.corners.iter().zip(&model.weight) {
        for &n in c {
            for d in 0..3 {
                mass[3 * n + d] += cube * w / 8.0;
            }
        }
    }
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
    let mut notes = vec![];
    let mut lambda = vec![0.0; p];
    for round in 0..40 {
        let mut settled = true;
        // Loose solves while the subspace is still far off, tight once it settles.
        let tol = if round < 3 { 1e-4 } else { 1e-7 };
        let solved: Vec<(Vec<f64>, bool)> = (0..p)
            .into_par_iter()
            .map(|i| {
                let y: Vec<f64> = x[i].iter().zip(&mass).map(|(a, m)| a * m).collect();
                let mut zi = z[i].clone();
                let (_, ok) = model.cg(&y, &mut zi, tol);
                (zi, ok)
            })
            .collect();
        for (i, (zi, ok)) in solved.into_iter().enumerate() {
            z[i] = zi;
            settled &= ok;
        }
        if !settled && round == 0 {
            notes.push("The part is not fully held: it can move freely, so the lowest frequencies are near zero. Add supports.".into());
        }
        // Rayleigh–Ritz in the space of z.
        let kz: Vec<Vec<f64>> = z.iter().map(|zi| model.apply(zi)).collect();
        let kr: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| z[i].iter().zip(&kz[j]).map(|(a, b)| a * b).sum()).collect()).collect();
        let mr: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| z[i].iter().zip(&z[j]).zip(&mass).map(|((a, b), m)| a * b * m).sum()).collect()).collect();
        // Mr = L Lᵀ; A = L⁻¹ Kr L⁻ᵀ.
        let mut l = vec![vec![0.0; p]; p];
        for i in 0..p {
            for j in 0..=i {
                let s: f64 = mr[i][j] - (0..j).map(|k| l[i][k] * l[j][k]).sum::<f64>();
                if i == j {
                    l[i][i] = s.max(1e-300).sqrt();
                } else {
                    l[i][j] = s / l[j][j];
                }
            }
        }
        let solve_l = |b: &[f64]| -> Vec<f64> {
            let mut y = vec![0.0; p];
            for i in 0..p {
                y[i] = (b[i] - (0..i).map(|k| l[i][k] * y[k]).sum::<f64>()) / l[i][i];
            }
            y
        };
        // B = L⁻¹ Kr (columns), A = L⁻¹ Bᵀ.
        let b: Vec<Vec<f64>> = (0..p).map(|j| solve_l(&(0..p).map(|i| kr[i][j]).collect::<Vec<_>>())).collect(); // b[j] = column j
        let a: Vec<Vec<f64>> = (0..p).map(|i| solve_l(&(0..p).map(|j| b[j][i]).collect::<Vec<_>>())).collect();
        let a: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| (a[i][j] + a[j][i]) / 2.0).collect()).collect();
        let (vals, vecs) = symmetric_eigen(a);
        // Q = L⁻ᵀ V.
        let solve_lt = |b: &[f64]| -> Vec<f64> {
            let mut y = vec![0.0; p];
            for i in (0..p).rev() {
                y[i] = (b[i] - (i + 1..p).map(|k| l[k][i] * y[k]).sum::<f64>()) / l[i][i];
            }
            y
        };
        let q: Vec<Vec<f64>> = vecs.iter().map(|v| solve_lt(v)).collect(); // q[i] = coefficients of mode i
        x = q.iter().map(|qi| (0..n3).map(|d| (0..p).map(|j| z[j][d] * qi[j]).sum()).collect()).collect();
        // The next solves' answers are close to x/λ: start there.
        z = x.iter().zip(&vals).map(|(xi, l)| xi.iter().map(|v| v / l.max(1e-300)).collect()).collect();
        lambda = vals;
        let done = (0..count).all(|i| (lambda[i] - last[i]).abs() <= 1e-5 * lambda[i].abs().max(1e-12));
        last = lambda.clone();
        if done && round > 1 {
            break;
        }
    }
    // Normalise each shape (largest movement 1) and read it per cube.
    let nodes: Vec<usize> = (0..model.nodes).filter(|&n| model.used[n]).collect();
    let modes = (0..count)
        .map(|i| {
            let v = &x[i];
            let mag = |n: usize| (v[3 * n] * v[3 * n] + v[3 * n + 1] * v[3 * n + 1] + v[3 * n + 2] * v[3 * n + 2]).sqrt();
            let top = nodes.iter().map(|&n| mag(n)).fold(0.0, f64::max).max(1e-300);
            let mut along = [0.0; 3];
            for &n in &nodes {
                for (d, a) in along.iter_mut().enumerate() {
                    *a += v[3 * n + d].abs();
                }
            }
            let dir = (0..3).max_by(|&a, &b| along[a].total_cmp(&along[b])).unwrap_or(2);
            Mode {
                frequency: (lambda[i].max(0.0) * 1000.0).sqrt() / std::f64::consts::TAU,
                direction: ["x", "y", "z"][dir].into(),
                amount: model.corners.iter().map(|c| (c.iter().map(|&n| mag(n)).sum::<f64>() / 8.0 / top) as f32).collect(),
                shape: nodes.iter().map(|&n| [v[3 * n] / top, v[3 * n + 1] / top, v[3 * n + 2] / top].map(|c| c as f32)).collect(),
            }
        })
        .collect();
    notes.push(format!("{} cubes of {} mm, masses lumped at their corners.", model.cells.len(), crate::drawing::length_text(h)));
    Ok(Modes {
        cell: h,
        origin: model.grid.origin,
        cells: model.cells.iter().map(|c| c.map(|v| v as u32)).collect(),
        nodes: nodes.iter().map(|&n| model.grid.node_at(n).map(|v| v as u32)).collect(),
        modes,
        mass: cube * model.weight.iter().sum::<f64>(),
        notes,
    })
}

// ---- Heat ------------------------------------------------------------------------

/// Thermal conductivity (W/m·K), typical by material name.
pub fn conductivity(material: &str) -> Option<f64> {
    let m = material.to_lowercase();
    Some(if m.contains("stainless") {
        16.0
    } else if m.contains("steel") {
        50.0
    } else if m.contains("alumin") {
        167.0
    } else if m.contains("copper") {
        400.0
    } else if m.contains("brass") {
        110.0
    } else if m.contains("titan") {
        7.0
    } else if m.contains("pla") {
        0.13
    } else if m.contains("petg") {
        0.2
    } else if m.contains("abs") {
        0.17
    } else if m.contains("nylon") {
        0.25
    } else if m.contains("acetal") || m.contains("pom") {
        0.31
    } else if m.contains("oak") {
        0.17
    } else if m.contains("ply") {
        0.13
    } else if m.contains("mdf") {
        0.12
    } else if m.contains("glass") {
        1.0
    } else if m.contains("concrete") {
        1.7
    } else {
        return None;
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Hold {
    pub face: Face,
    /// °C.
    pub temperature: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Heat {
    pub face: Face,
    /// Watts going in (negative: taken out).
    pub watts: f64,
}

/// Air carrying heat away from every outside face.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct Convection {
    /// W/m²·K: about 5–10 still air, 25–100 a fan.
    pub h: f64,
    /// °C.
    pub ambient: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ThermalStudy {
    #[serde(default)]
    pub holds: Vec<Hold>,
    #[serde(default)]
    pub heats: Vec<Heat>,
    #[serde(default)]
    pub convection: Option<Convection>,
    #[serde(default = "default_resolution")]
    pub resolution: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Temperatures {
    pub cell: f64,
    pub origin: [f64; 3],
    /// Filled cubes and their temperature (°C).
    pub cells: Vec<([u32; 3], f32)>,
    pub max: f64,
    pub min: f64,
    /// Watts flowing in through the heated faces, and out through holds and air.
    pub heat_in: f64,
    pub to_holds: f64,
    pub to_air: f64,
    pub notes: Vec<String>,
}

/// Conductance of a cube of side h (mm) per unit conductivity (W/mm·K): 8×8.
fn cube_conductance(h: f64) -> [[f64; 8]; 8] {
    let gp = 1.0 / 3f64.sqrt();
    let w = (h / 2.0).powi(3);
    let mut k = [[0.0; 8]; 8];
    for c in CORNERS {
        let xi = [c[0] * gp, c[1] * gp, c[2] * gp];
        let grad: Vec<[f64; 3]> = CORNERS
            .iter()
            .map(|n| {
                let f = |q: usize| 1.0 + xi[q] * n[q];
                [n[0] * f(1) * f(2) / 8.0 * 2.0 / h, n[1] * f(0) * f(2) / 8.0 * 2.0 / h, n[2] * f(0) * f(1) / 8.0 * 2.0 / h]
            })
            .collect();
        for a in 0..8 {
            for b in 0..8 {
                k[a][b] += w * (grad[a][0] * grad[b][0] + grad[a][1] * grad[b][1] + grad[a][2] * grad[b][2]);
            }
        }
    }
    k
}

/// Steady temperatures through a part: conduction inside, faces held at
/// temperatures, heat in through faces, and air cooling on the outside.
/// `k` in W/m·K.
pub fn thermal(positions: &[f32], triangles: &[u32], k: f64, study: &ThermalStudy) -> Result<Temperatures, String> {
    if study.holds.is_empty() && study.convection.is_none() {
        return Err("hold a face at a temperature, or cool it with air: heat needs somewhere to go".into());
    }
    let grid = voxelize(positions, triangles, study.resolution.clamp(8, 160));
    let h = grid.h;
    let cells: Vec<[usize; 3]> = (0..grid.n[2]).flat_map(|z| (0..grid.n[1]).flat_map(move |y| (0..grid.n[0]).map(move |x| [x, y, z]))).filter(|&[i, j, kk]| grid.fill[grid.cell(i, j, kk)] >= MIN_FILL).collect();
    if cells.is_empty() {
        return Err("the part is too thin for this resolution: raise it".into());
    }
    let nodes = (grid.n[0] + 1) * (grid.n[1] + 1) * (grid.n[2] + 1);
    let corners: Vec<[usize; 8]> = cells.iter().map(|&[i, j, kk]| grid.corners(i, j, kk)).collect();
    let weight: Vec<f64> = cells.iter().map(|&[i, j, kk]| (grid.fill[grid.cell(i, j, kk)] as f64).min(1.0)).collect();
    let mut used = vec![false; nodes];
    for c in &corners {
        for &n in c {
            used[n] = true;
        }
    }
    let ke = cube_conductance(h);
    let k_mm = k / 1000.0; // W/mm·K
    // Air: each outside face of a cube (no filled neighbour) sheds h·A·(T − ambient), shared by its 4 corners.
    let mut air = vec![0.0; nodes]; // W/K at each node
    let filled = |i: i64, j: i64, kk: i64| i >= 0 && j >= 0 && kk >= 0 && (i as usize) < grid.n[0] && (j as usize) < grid.n[1] && (kk as usize) < grid.n[2] && grid.fill[grid.cell(i as usize, j as usize, kk as usize)] >= MIN_FILL;
    let mut air_area = 0.0;
    if let Some(cv) = study.convection {
        let per_face = cv.h * 1e-6 * h * h; // W/K for one cube face (W/m²K × mm² → ×1e-6)
        for &[i, j, kk] in &cells {
            let (i, j, kk) = (i as i64, j as i64, kk as i64);
            for (d, quad) in [([-1, 0, 0], [0usize, 3, 7, 4]), ([1, 0, 0], [1, 2, 6, 5]), ([0, -1, 0], [0, 1, 5, 4]), ([0, 1, 0], [3, 2, 6, 7]), ([0, 0, -1], [0, 1, 2, 3]), ([0, 0, 1], [4, 5, 6, 7])] {
                if filled(i + d[0], j + d[1], kk + d[2]) {
                    continue;
                }
                let c = grid.corners(i as usize, j as usize, kk as usize);
                for q in quad {
                    air[c[q]] += per_face / 4.0;
                }
                air_area += h * h;
            }
        }
    }
    // Held nodes and their temperatures; heat in at the nodes of heated faces.
    let mut fixed: Vec<Option<f64>> = vec![None; nodes];
    for (s, hold) in study.holds.iter().enumerate() {
        let on = nodes_on(&grid, &used, &hold.face);
        if on.is_empty() {
            return Err(format!("held face {} touches no part of the part", s + 1));
        }
        for n in on {
            fixed[n] = Some(hold.temperature);
        }
    }
    let mut q = vec![0.0; nodes];
    let mut heat_in = 0.0;
    let share = node_shares(nodes, &corners, &weight);
    for (s, heat) in study.heats.iter().enumerate() {
        let on: Vec<usize> = nodes_on(&grid, &used, &heat.face).into_iter().filter(|&n| fixed[n].is_none()).collect();
        if on.is_empty() {
            return Err(format!("heated face {} touches no free part of the part", s + 1));
        }
        for (n, v) in spread(&on, &share, heat.watts) {
            q[n] += v;
        }
        heat_in += heat.watts;
    }
    if let Some(cv) = study.convection {
        for n in 0..nodes {
            q[n] += air[n] * cv.ambient;
        }
    }
    // K·T (conduction plus air), all nodes.
    let apply = |t: &[f64]| -> Vec<f64> {
        let mut y = corners
            .par_chunks(4096)
            .zip(weight.par_chunks(4096))
            .fold(
                || vec![0.0; nodes],
                |mut y, (chunk, ws)| {
                    for (c, w) in chunk.iter().zip(ws) {
                        for a in 0..8 {
                            let mut s = 0.0;
                            for b in 0..8 {
                                s += ke[a][b] * t[c[b]];
                            }
                            y[c[a]] += w * k_mm * s;
                        }
                    }
                    y
                },
            )
            .reduce(|| vec![0.0; nodes], |mut a, b| {
                a.iter_mut().zip(&b).for_each(|(x, y)| *x += y);
                a
            });
        y.iter_mut().zip(&air).zip(t).for_each(|((y, a), t)| *y += a * t);
        y
    };
    let free = |n: usize| used[n] && fixed[n].is_none();
    // T = T_held on held nodes; solve the rest of K·T = q for the free ones.
    let mut t: Vec<f64> = (0..nodes).map(|n| fixed[n].unwrap_or(0.0)).collect();
    let mut diag = air.clone();
    for (c, w) in corners.iter().zip(&weight) {
        for a in 0..8 {
            diag[c[a]] += w * k_mm * ke[a][a];
        }
    }
    let mask = |v: &mut Vec<f64>| v.iter_mut().enumerate().for_each(|(n, x)| if !free(n) { *x = 0.0 });
    let kt = apply(&t);
    let mut r: Vec<f64> = q.iter().zip(&kt).map(|(a, b)| a - b).collect();
    mask(&mut r);
    let rnorm0 = r.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
    let pre = |r: &[f64]| -> Vec<f64> { r.iter().zip(&diag).enumerate().map(|(n, (v, d))| if free(n) && *d > 0.0 { v / d } else { 0.0 }).collect() };
    let mut z = pre(&r);
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let mut it = 0;
    while it < 20_000 && r.iter().map(|v| v * v).sum::<f64>().sqrt() > 1e-10 * rnorm0 {
        let mut ap = apply(&p);
        mask(&mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap.abs() < 1e-300 {
            break;
        }
        let alpha = rz / pap;
        t.iter_mut().zip(&p).for_each(|(x, y)| *x += alpha * y);
        r.iter_mut().zip(&ap).for_each(|(x, y)| *x -= alpha * y);
        z = pre(&r);
        let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        p.iter_mut().zip(&z).for_each(|(x, y)| *x = y + rz2 / rz * *x);
        rz = rz2;
        it += 1;
    }
    // Where the heat went: into the holds (their reaction) and to the air.
    let kt = apply(&t);
    // A held node takes out what conduction brings it beyond its own input: q − K·T there.
    let to_holds: f64 = (0..nodes).filter(|&n| used[n] && fixed[n].is_some()).map(|n| q[n] - kt[n]).sum();
    let to_air: f64 = study.convection.map_or(0.0, |cv| (0..nodes).filter(|&n| used[n]).map(|n| air[n] * (t[n] - cv.ambient)).sum());
    let cell_t: Vec<f64> = corners.iter().map(|c| c.iter().map(|&n| t[n]).sum::<f64>() / 8.0).collect();
    let (max, min) = (0..nodes).filter(|&n| used[n]).fold((f64::NEG_INFINITY, f64::INFINITY), |(hi, lo), n| (hi.max(t[n]), lo.min(t[n])));
    let mut notes = vec![format!("{} cubes of {} mm; conductivity {} W/m·K (typical).", cells.len(), crate::drawing::length_text(h), k)];
    if study.convection.is_some() {
        notes.push(format!("Air cools {} cm² of outside surface (as the cubes' faces).", crate::drawing::length_text(air_area / 100.0)));
    }
    Ok(Temperatures {
        cell: h,
        origin: grid.origin,
        cells: cells.iter().zip(&cell_t).map(|(c, v)| (c.map(|x| x as u32), *v as f32)).collect(),
        max,
        min,
        heat_in,
        to_holds,
        to_air,
        notes,
    })
}

/// The temperature at the cube nearest a point (°C).
pub fn temperature_at(r: &Temperatures, point: [f64; 3]) -> Option<f64> {
    let idx = [0, 1, 2].map(|k| ((point[k] - r.origin[k]) / r.cell - 0.5).round() as i64);
    r.cells.iter().find(|(c, _)| (0..3).all(|k| c[k] as i64 == idx[k])).map(|(_, s)| *s as f64)
}
