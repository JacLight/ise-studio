//! Smoothing: Laplacian (each vertex toward its neighbours' average, which
//! also shrinks) and Taubin's λ|μ filter (1995), which alternates a shrinking
//! and an inflating step so noise goes and the volume stays.

use crate::linalg::*;
use crate::mesh::Mesh;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Laplacian,
    Taubin,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub method: Method,
    pub iterations: usize,
    /// Step toward the average (0…1), λ for Taubin.
    pub lambda: f64,
    /// Taubin's inflating step (negative, |μ| a little above λ).
    pub mu: f64,
    /// Leave open rims where they are.
    pub keep_boundary: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { method: Method::Taubin, iterations: 10, lambda: 0.5, mu: -0.53, keep_boundary: true }
    }
}

pub fn smooth(m: &mut Mesh, o: &Options) {
    let nb = m.vertex_neighbours();
    let fixed: Vec<bool> = if o.keep_boundary {
        let mut f = vec![false; m.positions.len()];
        for lp in m.boundary_loops() {
            for v in lp {
                f[v as usize] = true;
            }
        }
        f
    } else {
        vec![false; m.positions.len()]
    };
    let step = |m: &mut Mesh, factor: f64| {
        let new: Vec<_> = (0..m.positions.len())
            .map(|v| {
                let ns = &nb[v];
                if fixed[v] || ns.is_empty() {
                    return m.positions[v];
                }
                let avg = scale(ns.iter().fold([0.0; 3], |s, &u| add(s, m.positions[u as usize])), 1.0 / ns.len() as f64);
                add(m.positions[v], scale(sub(avg, m.positions[v]), factor))
            })
            .collect();
        m.positions = new;
    };
    for _ in 0..o.iterations {
        step(m, o.lambda);
        if o.method == Method::Taubin {
            step(m, o.mu);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::uv_sphere;

    fn noisy() -> (Mesh, f64) {
        let mut m = uv_sphere(20.0, 60, 120);
        let clean = m.volume();
        for (i, p) in m.positions.iter_mut().enumerate() {
            let r = 1.0 + 0.02 * (((i * 7919) % 1000) as f64 / 500.0 - 1.0);
            *p = scale(*p, r);
        }
        (m, clean)
    }

    fn roughness(m: &Mesh) -> f64 {
        let r: Vec<f64> = m.positions.iter().map(|p| norm(*p)).collect();
        let mean = r.iter().sum::<f64>() / r.len() as f64;
        (r.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / r.len() as f64).sqrt()
    }

    #[test]
    fn taubin_removes_noise_and_keeps_volume_laplacian_shrinks() {
        let (m, clean) = noisy();
        let before = roughness(&m);
        let mut t = m.clone();
        smooth(&mut t, &Options { iterations: 20, ..Default::default() });
        let mut l = m.clone();
        smooth(&mut l, &Options { method: Method::Laplacian, iterations: 20, ..Default::default() });
        assert!(roughness(&t) < before * 0.3, "{} vs {before}", roughness(&t));
        let tv = (t.volume() - clean).abs() / clean;
        let lv = (l.volume() - clean).abs() / clean;
        assert!(tv < 0.01, "Taubin volume change {:.3}%", tv * 100.0);
        assert!(lv > tv * 2.0, "Laplacian shrinks more ({:.3}% vs {:.3}%)", lv * 100.0, tv * 100.0);
    }
}
