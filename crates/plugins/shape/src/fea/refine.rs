//! Sharper peaks: the most stressed spot re-solved on finer cubes.
//!
//! Cubes read stress at their centres, half a cube inside the surface where
//! the peak is, and a staircase stands in for a curved surface, so the
//! coarse model reads a stress concentration low. This module takes a box
//! around the most stressed spot (away from supports and loads, whose own
//! peaks are artefacts of a perfectly rigid hold or a force on a face),
//! fills it with cubes 2×, 4× and (if the peak is still moving) 8× finer, holds its cut faces where the
//! coarse solution moved them (a submodel), solves, and reads the stress at
//! the surface itself: at each surface cube, the stress of the well-filled
//! cubes around is fitted linearly (least squares, every component) and
//! extrapolated to the nearest point of the true surface. How far the peak
//! moves between the last two says how settled it is.
//!
//! A sharp inside corner (no fillet) has no finite peak in linear
//! elasticity: the stress there grows without limit as the cubes shrink.
//! Such a corner is found from the part's own mesh and reported as that,
//! never as a number.

use super::*;

/// The most stressed spot, re-solved finer.
#[derive(Clone, Debug, Serialize)]
pub struct Peak {
    /// On the surface (mm, part coordinates).
    pub at: [f64; 3],
    /// MPa: read at the surface from the finest cubes, and the largest
    /// cube-centre stress of the coarse cubes there.
    pub stress: f64,
    pub coarse: f64,
    /// The finest cubes (mm), and how much the peak moved from the previous
    /// refinement (a share of it: what may be left to gain).
    pub cell: f64,
    pub change: f64,
    /// At a sharp inside corner: no finite peak; `stress` is only what this
    /// resolution reads.
    pub singular: bool,
    /// The most stressed cube of all is where a support holds the part (or
    /// a load presses): that number grows with resolution and is left out.
    pub at_support: bool,
}

pub(super) struct Context<'a> {
    pub positions: &'a [f32],
    pub triangles: &'a [u32],
    pub material: Elastic,
    pub study: &'a Study,
    /// Warming above the stress-free temperature at a point (K).
    pub warm: &'a dyn Fn([f64; 3]) -> f64,
}

/// Coarse cubes either side of the hot cube that the box takes in.
const REACH: i64 = 5;
/// Fine cubes are kept under this many (the 4× level shrinks its box to fit).
const BUDGET: usize = 600_000;

/// Re-solves the most stressed spot of a solved single-part model.
pub(super) fn sharpen(m: &Model, u: &[f64], f: &[f64], vm: &[f64], ctx: &Context) -> Result<(Peak, Vec<String>), String> {
    let h = m.grid.h;
    // Cubes touching a held node or a loaded one: their peaks are the hold's and the load's, not the part's.
    let near_hold = |e: usize| m.corners[e].iter().any(|&n| m.fixed[3 * n] || f[3 * n..3 * n + 3].iter().any(|v| *v != 0.0));
    let hot = (0..m.cells.len()).filter(|&e| !near_hold(e) && m.weight[e] > 0.5).max_by(|&a, &b| vm[a].total_cmp(&vm[b])).ok_or("Peak: every cube touches a support or a load, so there is no spot to sharpen.")?;
    let top = (0..m.cells.len()).max_by(|&a, &b| vm[a].total_cmp(&vm[b])).unwrap();
    let at_support = top != hot && vm[top] > vm[hot] * 1.02;
    let c = m.cells[hot].map(|v| v as i64);
    let mut levels: Vec<(f64, [f64; 3], f64)> = vec![];
    let mut notes = vec![];
    for factor in [2usize, 4, 8] {
        // The box: whole coarse cubes, padded past the grid's edge where it reaches it (no cut there).
        let mut reach = REACH;
        let (lo, hi) = loop {
            let lo: [i64; 3] = std::array::from_fn(|d| if c[d] - reach <= 0 { -1 } else { c[d] - reach });
            let hi: [i64; 3] = std::array::from_fn(|d| if c[d] + reach + 1 >= m.grid.n[d] as i64 { m.grid.n[d] as i64 + 1 } else { c[d] + reach + 1 });
            let cubes: usize = (0..3).map(|d| (hi[d] - lo[d]) as usize * factor).product();
            if cubes <= BUDGET * 2 || reach <= 2 {
                break (lo, hi);
            }
            reach -= 1;
        };
        let cubes: usize = (0..3).map(|d| (hi[d] - lo[d]) as usize * factor).product();
        if factor == 8 && cubes > BUDGET * 2 {
            break;
        }
        let level = solve_box(m, u, ctx, lo, hi, factor)?;
        levels.push(level);
        // Settled (within 5 %) from 2× to 4×: no need to go to 8×.
        if levels.len() == 2 && (levels[1].0 - levels[0].0).abs() < 0.05 * levels[1].0 {
            break;
        }
    }
    let k = levels.len() - 1;
    let (stress, at, cell) = levels[k];
    let change = (levels[k].0 - levels[k - 1].0).abs() / levels[k].0.max(1e-300);
    let singular = sharp_inside_corner(ctx.positions, ctx.triangles, at, 1.5 * h);
    let coarse = vm[hot];
    if singular {
        notes.push(format!(
            "Peak: the most stressed spot (away from supports and loads) is a sharp inside corner. Its true stress has no finite value in this theory: it grows as the cubes shrink ({} MPa here at {} mm cubes, {} MPa at twice that), and in the real part the material yields locally. Round the corner with a fillet to get a real number.",
            n1(stress),
            crate::drawing::length_text(cell),
            n1(levels[k - 1].0)
        ));
    } else {
        notes.push(format!(
            "Peak: re-solved on {} mm cubes around the most stressed spot and read at the surface: {} MPa (the coarse cube centres read {} MPa there). It moved {}% from cubes twice as big{}",
            crate::drawing::length_text(cell),
            n1(stress),
            n1(coarse),
            n1(change * 100.0),
            if change > 0.1 { ": still settling, so read it as a lower bound." } else { "." }
        ));
    }
    if at_support {
        notes.push(format!("The very highest cube ({} MPa) is where a support holds the part or a load presses on it: a perfectly rigid hold or a force on a face makes its own sharp peak there that a real mount spreads, so it is left out of the peak.", n1(vm[top])));
    }
    Ok((Peak { at, stress, coarse, cell, change, singular, at_support }, notes))
}

fn n1(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(String::from).unwrap_or(s)
}

/// Solves the box of coarse cubes [lo, hi) on cubes `factor` times finer,
/// held at its cut faces as the coarse solution moved; returns the surface
/// peak (MPa), where it is, and the fine cube size.
fn solve_box(m: &Model, u: &[f64], ctx: &Context, lo: [i64; 3], hi: [i64; 3], factor: usize) -> Result<(f64, [f64; 3], f64), String> {
    let h = m.grid.h;
    let hf = h / factor as f64;
    let n: [usize; 3] = std::array::from_fn(|d| (hi[d] - lo[d]) as usize * factor);
    let origin: [f64; 3] = std::array::from_fn(|d| m.grid.origin[d] + lo[d] as f64 * h);
    let mut grid = Grid { h: hf, origin, n, fill: vec![] };
    grid.fill = fill_of(&grid, ctx.positions, ctx.triangles);
    let fill = grid.fill.clone();
    let mut fine = Model::assemble(grid, &[fill], &[ctx.material], &["the part".into()], &[], &[], false)?;
    // Coarse cube index → element, to read the coarse movement anywhere.
    let gn = m.grid.n;
    let mut element = vec![u32::MAX; gn[0] * gn[1] * gn[2]];
    for (e, c) in m.cells.iter().enumerate() {
        element[m.grid.cell(c[0], c[1], c[2])] = e as u32;
    }
    // The coarse movement at a point, and whether the coarse nodes it comes from are all held.
    let coarse_at = |x: [f64; 3]| -> Option<([f64; 3], bool)> {
        let g: [f64; 3] = std::array::from_fn(|d| (x[d] - m.grid.origin[d]) / h);
        let base: [i64; 3] = std::array::from_fn(|d| (g[d].floor() as i64).clamp(0, gn[d] as i64 - 1));
        // Its own cube, else the nearest filled one about it (movement carried on linearly).
        let mut best: Option<(f64, usize, [i64; 3])> = None;
        for o in (-1..=1).flat_map(|a| (-1..=1).flat_map(move |b| (-1..=1).map(move |c| [a, b, c]))) {
            let q: [i64; 3] = std::array::from_fn(|d| base[d] + o[d]);
            if (0..3).any(|d| q[d] < 0 || q[d] >= gn[d] as i64) {
                continue;
            }
            let e = element[m.grid.cell(q[0] as usize, q[1] as usize, q[2] as usize)];
            if e == u32::MAX {
                continue;
            }
            let dist: f64 = (0..3).map(|d| (g[d] - (q[d] as f64 + 0.5)).powi(2)).sum();
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, e as usize, q));
            }
        }
        let (_, e, q) = best?;
        let t: [f64; 3] = std::array::from_fn(|d| (g[d] - q[d] as f64).clamp(0.0, 1.0));
        let mut out = [0.0; 3];
        let mut held = true;
        for (a, corner) in CORNERS.iter().enumerate() {
            let w: f64 = (0..3).map(|d| if corner[d] > 0.0 { t[d] } else { 1.0 - t[d] }).product();
            if w < 1e-12 {
                continue;
            }
            let node = m.corners[e][a];
            held &= m.fixed[3 * node];
            for d in 0..3 {
                out[d] += w * u[3 * node + d];
            }
        }
        Some((out, held))
    };
    // Held: the cut faces (where the coarse solution moved them) and whatever lies on held coarse nodes.
    let mut uf = vec![0.0; 3 * fine.nodes];
    for id in (0..fine.nodes).filter(|&id| fine.used[id]) {
        let g = fine.node_at(id);
        let x = fine.position(id);
        let cut = (0..3).any(|d| g[d] == 0 || g[d] == n[d]);
        let Some((v, held)) = coarse_at(x) else { continue };
        if cut || held {
            fine.fixed[3 * id..3 * id + 3].fill(true);
            if !held {
                uf[3 * id..3 * id + 3].copy_from_slice(&v);
            }
        }
    }
    if !fine.fixed.iter().any(|&x| x) {
        return Err("Peak: the spot could not be cut out to re-solve.".into());
    }
    fine.finish(&[false]);
    // Loads on faces inside the box: each its share by the coarse nodes it covered there.
    let mut rhs = vec![0.0; 3 * fine.nodes];
    let inside = |x: [f64; 3]| -> f64 {
        let mut w = 1.0;
        for d in 0..3 {
            let (a, b) = (origin[d], origin[d] + n[d] as f64 * hf);
            if x[d] < a - 1e-9 * h || x[d] > b + 1e-9 * h {
                return 0.0;
            }
            if (x[d] - a).abs() < 1e-9 * h || (x[d] - b).abs() < 1e-9 * h {
                w *= 0.5;
            }
        }
        w
    };
    let coarse_share = node_shares(m.nodes, &m.corners, &m.weight);
    let fine_share = node_shares(fine.nodes, &fine.corners, &fine.weight);
    let put = |coarse_on: Vec<usize>, fine_on: Vec<usize>, force: [f64; 3], rhs: &mut Vec<f64>| {
        let total: f64 = coarse_on.iter().map(|&k| coarse_share[k]).sum();
        let here: f64 = coarse_on.iter().map(|&k| coarse_share[k] * inside(m.position(k))).sum();
        let on: Vec<usize> = fine_on.into_iter().filter(|&k| !fine.fixed[3 * k]).collect();
        if total <= 0.0 || here <= 0.0 || on.is_empty() {
            return;
        }
        for d in 0..3 {
            for (k, v) in spread(&on, &fine_share, force[d] * here / total) {
                rhs[3 * k + d] += v;
            }
        }
    };
    for load in &ctx.study.loads {
        let coarse_on: Vec<usize> = nodes_on_face(m.part_nodes(0), |id| m.position(id), h, &load.face).into_iter().filter(|&k| !m.fixed[3 * k]).collect();
        let fine_on = nodes_on_face(fine.part_nodes(0), |id| fine.position(id), hf, &load.face);
        put(coarse_on, fine_on, load.force, &mut rhs);
    }
    for load in &ctx.study.bore_loads {
        let coarse_on: Vec<usize> = nodes_near_axis(m.part_nodes(0), |id| m.position(id), h, &load.bore).into_iter().filter(|&k| !m.fixed[3 * k]).collect();
        // The fine nodes as close to the axis as the coarse ones were.
        let reach = coarse_on.iter().map(|&k| axis_distance(m.position(k), &load.bore)).fold(0.0, f64::max);
        let fine_on: Vec<usize> = fine.part_nodes(0).filter(|&k| axis_distance(fine.position(k), &load.bore) <= reach + 0.5 * hf).collect();
        put(coarse_on, fine_on, load.force, &mut rhs);
    }
    // Heat, cube by cube.
    let dt: Vec<f64> = fine.cells.iter().map(|&c| (ctx.warm)(cell_centre(&fine.grid, c))).collect();
    let materials = [ctx.material];
    if dt.iter().any(|t| *t != 0.0) {
        let fh = thermal_forces(&fine, &materials, &dt);
        rhs.iter_mut().zip(&fh).for_each(|(a, b)| *a += b);
    }
    let (_, ok) = fine.cg(&rhs, &mut uf, 1e-9);
    if !ok {
        return Err("Peak: the finer solve did not settle; the coarse stress stands.".into());
    }
    let tensors = cube_stresses(&fine, &[d_matrix(&ctx.material)], &materials, &uf, &dt);
    surface_peak(&fine, &tensors, ctx, lo, hi, m.grid.n, h)
}

fn axis_distance(q: [f64; 3], bore: &Bore) -> f64 {
    let l = (bore.axis[0] * bore.axis[0] + bore.axis[1] * bore.axis[1] + bore.axis[2] * bore.axis[2]).sqrt().max(1e-12);
    let a = bore.axis.map(|c| c / l);
    let d = [q[0] - bore.at[0], q[1] - bore.at[1], q[2] - bore.at[2]];
    let t = d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
    ((d[0] - t * a[0]).powi(2) + (d[1] - t * a[1]).powi(2) + (d[2] - t * a[2]).powi(2)).sqrt()
}

/// The largest stress at the surface, kept a coarse cube and a half clear
/// of the box's cut faces (the held cut stiffens what is next to it): each
/// surface cube's nearest surface point gets the stress of the well-filled
/// cubes about it (within 3.5 cubes) fitted linearly and carried out to it.
fn surface_peak(fine: &Model, tensors: &[[f64; 6]], ctx: &Context, lo: [i64; 3], hi: [i64; 3], coarse_n: [usize; 3], h: f64) -> Result<(f64, [f64; 3], f64), String> {
    let g = &fine.grid;
    let hf = g.h;
    let n = g.n;
    let mut index = vec![u32::MAX; n[0] * n[1] * n[2]];
    for (e, c) in fine.cells.iter().enumerate() {
        index[g.cell(c[0], c[1], c[2])] = e as u32;
    }
    let at = |c: [i64; 3]| -> Option<usize> { ((0..3).all(|d| c[d] >= 0 && (c[d] as usize) < n[d])).then(|| index[g.cell(c[0] as usize, c[1] as usize, c[2] as usize)]).filter(|&e| e != u32::MAX).map(|e| e as usize) };
    // Clear of the cut faces (sides padded past the grid have no cut).
    let clear = |x: [f64; 3]| {
        (0..3).all(|d| {
            let (a, b) = (g.origin[d], g.origin[d] + n[d] as f64 * hf);
            let lo_cut = lo[d] >= 0;
            let hi_cut = hi[d] <= coarse_n[d] as i64;
            (!lo_cut || x[d] - a >= 1.5 * h) && (!hi_cut || b - x[d] >= 1.5 * h)
        })
    };
    let surface: Vec<usize> = (0..fine.cells.len())
        .filter(|&e| {
            let c = fine.cells[e].map(|v| v as i64);
            let open = (0..6).any(|k| {
                let mut q = c;
                q[k / 2] += if k % 2 == 0 { -1 } else { 1 };
                at(q).is_none()
            });
            (open || fine.weight[e] < 0.999) && clear(cell_centre(g, fine.cells[e]))
        })
        .collect();
    if surface.is_empty() {
        return Err("Peak: no surface near the most stressed spot to read.".into());
    }
    let centres: Vec<[f64; 3]> = surface.iter().map(|&e| cell_centre(g, fine.cells[e])).collect();
    let near = surface_near(&centres, ctx.positions, ctx.triangles, 2.0 * hf);
    let fits: Vec<(f64, [f64; 3])> = surface
        .par_iter()
        .zip(&near)
        .filter_map(|(&e, &(dist, point, _))| {
            if !dist.is_finite() {
                return None;
            }
            // Well-filled cubes within three and a half cubes of the surface point.
            let c = fine.cells[e].map(|v| v as i64);
            let mut rows: Vec<([f64; 4], [f64; 6], f64)> = vec![];
            for o in (-4..=4).flat_map(|a| (-4..=4).flat_map(move |b| (-4..=4).map(move |cc| [a, b, cc]))) {
                let Some(k) = at([c[0] + o[0], c[1] + o[1], c[2] + o[2]]) else { continue };
                if fine.weight[k] < 0.5 {
                    continue;
                }
                let x = cell_centre(g, fine.cells[k]);
                let r = [(x[0] - point[0]) / hf, (x[1] - point[1]) / hf, (x[2] - point[2]) / hf];
                if r[0] * r[0] + r[1] * r[1] + r[2] * r[2] > 3.5 * 3.5 {
                    continue;
                }
                rows.push(([1.0, r[0], r[1], r[2]], tensors[k], fine.weight[k]));
            }
            if rows.len() < 8 {
                return None;
            }
            // Weighted least squares for a + b·r, each component: (AᵀWA)⁻¹AᵀW s; the value at r = 0 is a.
            let mut ata = [0.0; 16];
            let mut atb = [[0.0; 6]; 4];
            for (a, s, w) in &rows {
                for i in 0..4 {
                    for j in 0..4 {
                        ata[i * 4 + j] += w * a[i] * a[j];
                    }
                    for k in 0..6 {
                        atb[i][k] += w * a[i] * s[k];
                    }
                }
            }
            // A poorly spread patch (all on one side of a line) leaves the fit loose: skip it.
            let inv = invert(&ata, 4);
            if inv.iter().any(|v| !v.is_finite()) || inv[0].abs() > 10.0 {
                return None;
            }
            let s: [f64; 6] = std::array::from_fn(|k| (0..4).map(|j| inv[j] * atb[j][k]).sum());
            Some((von_mises(&s), point))
        })
        .collect();
    let (stress, point) = fits.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)).ok_or("Peak: too few cubes at the surface to read the stress there.")?;
    Ok((stress, point, hf))
}

/// Whether a sharp inside (concave) edge of the mesh, its faces turning by
/// more than 55°, passes within `reach` of a point: a corner with no
/// fillet, or one too small to show.
fn sharp_inside_corner(positions: &[f32], triangles: &[u32], at: [f64; 3], reach: f64) -> bool {
    let p = |i: u32| {
        let i = i as usize * 3;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    // Vertices welded by position (a mesh may repeat them per face).
    let key = |v: [f64; 3]| v.map(|c| (c * 1e5).round() as i64);
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let near_seg = |a: [f64; 3], b: [f64; 3]| {
        let ab = sub(b, a);
        let t = (dot(sub(at, a), ab) / dot(ab, ab).max(1e-300)).clamp(0.0, 1.0);
        let q = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
        dot(sub(at, q), sub(at, q)).sqrt() <= reach
    };
    let mut edges: std::collections::HashMap<([i64; 3], [i64; 3]), Vec<(usize, usize)>> = std::collections::HashMap::new();
    let tris: Vec<[[f64; 3]; 3]> = triangles.chunks_exact(3).map(|t| [p(t[0]), p(t[1]), p(t[2])]).collect();
    for (ti, t) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !near_seg(a, b) {
                continue;
            }
            let (ka, kb) = (key(a), key(b));
            edges.entry(if ka < kb { (ka, kb) } else { (kb, ka) }).or_default().push((ti, (k + 2) % 3));
        }
    }
    let normal = |t: &[[f64; 3]; 3]| {
        let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
        let l = dot(n, n).sqrt();
        (l > 1e-18).then(|| n.map(|c| c / l))
    };
    edges.values().any(|faces| {
        faces.len() == 2 && {
            let (t1, _) = faces[0];
            let (t2, o2) = faces[1];
            match (normal(&tris[t1]), normal(&tris[t2])) {
                (Some(n1), Some(n2)) => {
                    // Concave: the second face's far corner is outside the first's plane.
                    let off = dot(n1, sub(tris[t2][o2], tris[t1][0]));
                    let scale = dot(sub(tris[t2][o2], tris[t1][0]), sub(tris[t2][o2], tris[t1][0])).sqrt();
                    off > 1e-6 * scale && dot(n1, n2) < 55f64.to_radians().cos()
                }
                _ => false,
            }
        }
    })
}
