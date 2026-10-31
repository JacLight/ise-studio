//! How a stacked product comes apart upwards, from its meshes: which part is
//! above which, and how far each lifts in an exploded view.
//!
//! A product is stacked when every part stands over the one with the biggest
//! footprint (the base: a gearbox's housing, a case's bottom). Over the ground
//! the parts share, one part is above another where its material reaches
//! higher across more of that ground. That holds for parts that interleave too
//! (a sun gear under a carrier whose knob sits on top of it, a shaft through a
//! hollow shaft), where comparing boxes puts the taller one on top. Layers
//! follow, bottom up: each part above every part in the layers under it, each
//! layer lifted clear of the risen layers under it, plus a gap.

/// One part: its triangles (mm) and its box.
pub struct PartMesh {
    pub triangles: Vec<[[f64; 3]; 3]>,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

const CELLS: usize = 96;

/// A solid as a part to stack.
pub fn part_of(solid: &manifold_csg::Manifold) -> Option<PartMesh> {
    let bb = solid.bounding_box()?;
    let (props, n, indices) = solid.to_mesh_f32();
    let n = n.max(3);
    let at = |i: u32| {
        let v = &props[i as usize * n..i as usize * n + 3];
        [v[0] as f64, v[1] as f64, v[2] as f64]
    };
    let triangles = indices.chunks_exact(3).map(|t| [at(t[0]), at(t[1]), at(t[2])]).collect();
    let (min, max) = (bb.min(), bb.max());
    Some(PartMesh { triangles, min: [min[0], min[1], min[2]], max: [max[0], max[1], max[2]] })
}

/// How far each part rises in an exploded view (0 for the base), when the parts are stacked.
pub fn rises(parts: &[PartMesh]) -> Option<Vec<f64>> {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in parts {
        for k in 0..3 {
            lo[k] = lo[k].min(p.min[k]);
            hi[k] = hi[k].max(p.max[k]);
        }
    }
    let diagonal = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    let layers = lift_layers(parts, diagonal * 0.1)?;
    let mut out = vec![0.0; parts.len()];
    for (members, rise) in layers {
        for i in members {
            out[i] = (rise * 1000.0).round() / 1000.0;
        }
    }
    Some(out)
}

/// The layers of a stacked product, bottom first, each with how far it rises
/// (the base, not listed, stays). None when the parts are not stacked.
pub fn lift_layers(parts: &[PartMesh], gap: f64) -> Option<Vec<(Vec<usize>, f64)>> {
    if parts.len() < 2 {
        return None;
    }
    let foot = |p: &PartMesh| (p.max[0] - p.min[0]).max(0.0) * (p.max[1] - p.min[1]).max(0.0);
    let base = (0..parts.len()).max_by(|&i, &j| foot(&parts[i]).total_cmp(&foot(&parts[j])).then(parts[j].min[2].total_cmp(&parts[i].min[2])))?;
    let b = &parts[base];
    let pad = [(b.max[0] - b.min[0]) * 0.05, (b.max[1] - b.min[1]) * 0.05];
    let over = parts.iter().all(|p| (0..2).all(|k| {
        let c = (p.min[k] + p.max[k]) / 2.0;
        c >= b.min[k] - pad[k] && c <= b.max[k] + pad[k]
    }));
    if !over {
        return None;
    }
    let tops = heights(parts);
    let rest: Vec<usize> = (0..parts.len()).filter(|&i| i != base).collect();
    // above[a][b]: a is above b.
    let n = parts.len();
    let mut above = vec![vec![false; n]; n];
    for &a in &rest {
        for &c in &rest {
            if a < c {
                let (mut a_wins, mut c_wins) = (0usize, 0usize);
                for (ta, tc) in tops[a].iter().zip(&tops[c]) {
                    if let (Some(ta), Some(tc)) = (ta, tc) {
                        if ta > tc {
                            a_wins += 1;
                        } else if tc > ta {
                            c_wins += 1;
                        }
                    }
                }
                if a_wins > c_wins {
                    above[a][c] = true;
                } else if c_wins > a_wins {
                    above[c][a] = true;
                }
            }
        }
    }
    // Layers bottom up: the parts above none of those left; a loop is broken at the lowest top.
    let mut left = rest;
    let mut layers: Vec<Vec<usize>> = vec![];
    while !left.is_empty() {
        let mut layer: Vec<usize> = left.iter().copied().filter(|&a| !left.iter().any(|&c| above[a][c])).collect();
        if layer.is_empty() {
            let low = *left.iter().min_by(|&&i, &&j| parts[i].max[2].total_cmp(&parts[j].max[2]))?;
            layer.push(low);
        }
        left.retain(|i| !layer.contains(i));
        layers.push(layer);
    }
    let (mut floor, mut rise) = (b.max[2], 0.0f64);
    Some(
        layers
            .into_iter()
            .map(|members| {
                rise = members.iter().map(|&i| floor - parts[i].min[2] + gap).fold(rise, f64::max);
                floor = members.iter().map(|&i| parts[i].max[2] + rise).fold(floor, f64::max);
                (members, rise)
            })
            .collect(),
    )
}

/// Each part's highest material over each cell of a grid across all of them (None: none there).
fn heights(parts: &[PartMesh]) -> Vec<Vec<Option<f64>>> {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in parts {
        for k in 0..2 {
            lo[k] = lo[k].min(p.min[k]);
            hi[k] = hi[k].max(p.max[k]);
        }
    }
    let cell = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / CELLS as f64).max(1e-6);
    parts
        .iter()
        .map(|p| {
            let mut top = vec![None::<f64>; CELLS * CELLS];
            let mut mark = |x: f64, y: f64, z: f64| {
                let i = (((x - lo[0]) / cell) as usize).min(CELLS - 1);
                let j = (((y - lo[1]) / cell) as usize).min(CELLS - 1);
                let t = &mut top[j * CELLS + i];
                *t = Some(t.map_or(z, |v: f64| v.max(z)));
            };
            for [a, b, c] in &p.triangles {
                // Points across the triangle about half a cell apart (its corners included).
                let long = [(a, b), (b, c), (c, a)].iter().map(|(u, v)| ((u[0] - v[0]).powi(2) + (u[1] - v[1]).powi(2)).sqrt()).fold(0.0, f64::max);
                let k = ((long / (cell * 0.5)).ceil() as usize).clamp(1, 64);
                for s in 0..=k {
                    for t in 0..=k - s {
                        let (u, v) = (s as f64 / k as f64, t as f64 / k as f64);
                        let w = 1.0 - u - v;
                        mark(a[0] * w + b[0] * u + c[0] * v, a[1] * w + b[1] * u + c[1] * v, a[2] * w + b[2] * u + c[2] * v);
                    }
                }
            }
            top
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box's 12 triangles.
    fn block(min: [f64; 3], max: [f64; 3]) -> PartMesh {
        let p = |i: usize| [if i & 1 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 4 == 0 { min[2] } else { max[2] }];
        let faces = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]];
        let triangles = faces.iter().flat_map(|f| [[p(f[0]), p(f[1]), p(f[2])], [p(f[0]), p(f[2]), p(f[3])]]).collect();
        PartMesh { triangles, min, max }
    }

    fn join(parts: &[PartMesh]) -> PartMesh {
        let triangles = parts.iter().flat_map(|p| p.triangles.clone()).collect();
        let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in parts {
            for k in 0..3 {
                min[k] = min[k].min(p.min[k]);
                max[k] = max[k].max(p.max[k]);
            }
        }
        PartMesh { triangles, min, max }
    }

    #[test]
    fn a_knob_over_a_carrier_whose_shaft_rises_through_it_comes_off_first() {
        let housing = block([-50.0, -50.0, 0.0], [50.0, 50.0, 20.0]);
        let planet = block([20.0, -8.0, 1.0], [36.0, 8.0, 15.0]);
        // The carrier: a plate over the planets and a hollow shaft (four walls) up to 70.
        let carrier = join(&[
            block([-40.0, -40.0, 16.0], [40.0, 40.0, 22.0]),
            block([-10.0, -10.0, 22.0], [-6.0, 10.0, 70.0]),
            block([6.0, -10.0, 22.0], [10.0, 10.0, 70.0]),
            block([-6.0, -10.0, 22.0], [6.0, -6.0, 70.0]),
            block([-6.0, 6.0, 22.0], [6.0, 10.0, 70.0]),
        ]);
        // The knob: a sun under the plate, a shaft up the carrier's, and a ring round the carrier's shaft on top.
        let knob = join(&[
            block([-14.0, -14.0, 0.0], [14.0, 14.0, 14.0]),
            block([-4.0, -4.0, 14.0], [4.0, 4.0, 66.0]),
            block([-24.0, -24.0, 52.0], [-11.0, 24.0, 64.0]),
            block([11.0, -24.0, 52.0], [24.0, 24.0, 64.0]),
            block([-11.0, -24.0, 52.0], [11.0, -11.0, 64.0]),
            block([-11.0, 11.0, 52.0], [11.0, 24.0, 64.0]),
        ]);
        assert!(carrier.max[2] > knob.max[2], "the carrier is the taller");
        let layers = lift_layers(&[housing, planet, carrier, knob], 5.0).expect("stacked");
        let order: Vec<Vec<usize>> = layers.iter().map(|(m, _)| m.clone()).collect();
        assert_eq!(order, vec![vec![1], vec![2], vec![3]], "planet, carrier, then the knob on top");
        // Each clear of the risen layer under it.
        assert_eq!(layers[0].1, 20.0 - 1.0 + 5.0);
        assert_eq!(layers[1].1, (15.0 + 24.0) - 16.0 + 5.0);
        assert_eq!(layers[2].1, (70.0 + 28.0) - 0.0 + 5.0);
    }

    #[test]
    fn side_by_side_is_not_a_stack() {
        assert!(lift_layers(&[block([0.0; 3], [10.0; 3]), block([40.0, 0.0, 0.0], [50.0, 10.0, 10.0])], 5.0).is_none());
    }
}
