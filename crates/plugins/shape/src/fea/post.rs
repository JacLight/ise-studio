//! Reading a solved stress study: the stress and movement at a point, along
//! a line and across a plane, as a table (CSV), and the fatigue life the
//! stresses give when the loads come and go.

use super::materials::MaterialProps;
use super::*;

/// Everything known at a point of a solved study.
#[derive(Clone, Debug, Serialize)]
pub struct Probe {
    pub at: [f64; 3],
    /// The stress in the cube holding the point (xx yy zz xy yz zx, MPa).
    pub stress: [f64; 6],
    pub von_mises: f64,
    /// Principal stresses, largest first (MPa).
    pub principal: [f64; 3],
    /// Movement there (mm), from the cube's corners.
    pub displacement: [f64; 3],
    /// Yield strength over von Mises there.
    pub safety: f64,
    /// Equivalent plastic strain, when the study let the material yield.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plastic_strain: Option<f64>,
}

/// The cube of a result nearest a point (its index), within a cube and a half.
pub fn cube_near(r: &Result_, point: [f64; 3]) -> Option<usize> {
    let g: [f64; 3] = std::array::from_fn(|k| (point[k] - r.origin[k]) / r.cell - 0.5);
    let mut best: Option<(f64, usize)> = None;
    for (i, (c, _)) in r.cells.iter().enumerate() {
        let d: f64 = (0..3).map(|k| (c[k] as f64 - g[k]).powi(2)).sum();
        if d <= 2.25 && best.is_none_or(|b| d < b.0) {
            best = Some((d, i));
        }
    }
    best.map(|b| b.1)
}

/// Principal stresses of a stress, largest first.
pub fn principal(s: &[f64; 6]) -> [f64; 3] {
    let (vals, _) = symmetric_eigen(vec![vec![s[0], s[3], s[5]], vec![s[3], s[1], s[4]], vec![s[5], s[4], s[2]]]);
    [vals[2], vals[1], vals[0]]
}

/// The study's results at a point (None when it is not in the part).
pub fn probe(r: &Result_, point: [f64; 3]) -> Option<Probe> {
    let i = cube_near(r, point)?;
    let stress = r.tensors.get(i).map(|t| t.map(|v| v as f64)).unwrap_or([0.0; 6]);
    let c = r.cells[i].0;
    // Movement: trilinear from the cube's corners, in the cube's own part.
    let part = r.cell_parts.get(i).copied();
    let corner: HashMap<[u32; 3], [f32; 3]> = r
        .displacements
        .iter()
        .enumerate()
        .filter(|(k, (g, _))| (0..3).all(|d| g[d] >= c[d] && g[d] <= c[d] + 1) && r.node_parts.get(*k).copied() == part)
        .map(|(_, (g, v))| (*g, *v))
        .collect();
    let t: [f64; 3] = std::array::from_fn(|k| ((point[k] - r.origin[k]) / r.cell - c[k] as f64).clamp(0.0, 1.0));
    let mut displacement = [0.0; 3];
    for o in CORNERS {
        let g = [c[0] + (o[0] > 0.0) as u32, c[1] + (o[1] > 0.0) as u32, c[2] + (o[2] > 0.0) as u32];
        let w: f64 = (0..3).map(|k| if o[k] > 0.0 { t[k] } else { 1.0 - t[k] }).product();
        if let Some(v) = corner.get(&g) {
            (0..3).for_each(|k| displacement[k] += w * v[k] as f64);
        }
    }
    let vm = r.cells[i].1 as f64;
    let y = r.yields.get(part.map_or(0, |p| p as usize)).copied().unwrap_or(r.yield_strength);
    Some(Probe { at: point, stress, von_mises: vm, principal: principal(&stress), displacement, safety: if vm > 0.0 { y / vm } else { f64::INFINITY }, plastic_strain: r.plastic_strain.get(i).map(|v| *v as f64) })
}

/// The study's results at evenly spaced points from `a` to `b` (those in the part).
pub fn path(r: &Result_, a: [f64; 3], b: [f64; 3], points: usize) -> Vec<(f64, Probe)> {
    let n = points.clamp(2, 1000);
    let length = v_norm(v_sub(b, a));
    (0..n)
        .filter_map(|i| {
            let t = i as f64 / (n - 1) as f64;
            let p = std::array::from_fn(|k| a[k] + t * (b[k] - a[k]));
            probe(r, p).map(|q| (t * length, q))
        })
        .collect()
}

/// A plane cut through the results.
#[derive(Clone, Debug, Serialize)]
pub struct Section {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    /// The cubes the plane passes through: (i, j, k), von Mises (MPa), and
    /// the stress square to the plane (MPa, pulling positive).
    pub cells: Vec<([u32; 3], f32, f32)>,
    pub max: f64,
    /// The force carried across the plane (N): the stress over its cut area.
    pub force: [f64; 3],
    pub area: f64,
}

/// The cubes a plane cuts through, with their stress, and the force across it.
pub fn section(r: &Result_, point: [f64; 3], normal: [f64; 3]) -> Section {
    let n = normal.map(|c| c / v_norm(normal).max(1e-12));
    let h = r.cell;
    // A cube is cut when its centre lies within half its extent along the normal.
    let reach = 0.5 * h * (n[0].abs() + n[1].abs() + n[2].abs());
    let mut cells = vec![];
    let mut force = [0.0; 3];
    let mut area = 0.0;
    let mut max: f64 = 0.0;
    for (i, (c, vm)) in r.cells.iter().enumerate() {
        let x: [f64; 3] = std::array::from_fn(|k| r.origin[k] + (c[k] as f64 + 0.5) * h);
        let off = v_dot(v_sub(x, point), n);
        if off.abs() > reach {
            continue;
        }
        let s = r.tensors.get(i).map(|t| t.map(|v| v as f64)).unwrap_or([0.0; 6]);
        let m = [[s[0], s[3], s[5]], [s[3], s[1], s[4]], [s[5], s[4], s[2]]];
        let traction: [f64; 3] = std::array::from_fn(|a| (0..3).map(|b| m[a][b] * n[b]).sum());
        // Each cube's share of the cut: its volume over the slab's thickness.
        let share = h * h * h / (2.0 * reach);
        (0..3).for_each(|k| force[k] += traction[k] * share);
        area += share;
        max = max.max(*vm as f64);
        cells.push((*c, *vm, v_dot(traction, n) as f32));
    }
    Section { point, normal: n, cells, max, force, area }
}

/// The results as a table: one row per cube (centre, von Mises, the stress,
/// safety, plastic strain), then one per node (its place and movement).
pub fn csv(r: &Result_) -> String {
    let mut out = String::from("kind,x_mm,y_mm,z_mm,von_mises_mpa,sxx,syy,szz,sxy,syz,szx,safety,plastic_strain,dx_mm,dy_mm,dz_mm,part\n");
    let h = r.cell;
    let at = |c: [u32; 3], half: f64| -> [f64; 3] { std::array::from_fn(|k| r.origin[k] + (c[k] as f64 + half) * h) };
    for (i, (c, vm)) in r.cells.iter().enumerate() {
        let x = at(*c, 0.5);
        let s = r.tensors.get(i).copied().unwrap_or([0.0; 6]);
        let part = r.cell_parts.get(i).map_or(0, |p| *p as usize);
        let y = r.yields.get(part).copied().unwrap_or(r.yield_strength);
        let ep = r.plastic_strain.get(i).map_or(String::new(), |v| format!("{v:.6}"));
        out += &format!("cube,{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{ep},,,,{part}\n", x[0], x[1], x[2], vm, s[0], s[1], s[2], s[3], s[4], s[5], if *vm > 0.0 { y / *vm as f64 } else { 1e9 });
    }
    for (k, (g, d)) in r.displacements.iter().enumerate() {
        let x = at(*g, 0.0);
        let part = r.node_parts.get(k).map_or(0, |p| *p as usize);
        out += &format!("node,{:.4},{:.4},{:.4},,,,,,,,,,{:.7},{:.7},{:.7},{part}\n", x[0], x[1], x[2], d[0], d[1], d[2]);
    }
    out
}

// ---- Fatigue -------------------------------------------------------------------

/// How the loads come and go, and how the part is made, for a fatigue estimate.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct FatigueSpec {
    /// Smallest load over largest in each cycle: −1 fully reversed (a
    /// rotating shaft), 0 on and off, 0.5 a steady load with a ripple.
    #[serde(default = "reversed")]
    pub ratio: f64,
    /// The life wanted (cycles).
    #[serde(default = "million")]
    pub cycles: f64,
    /// "polished", "ground", "machined" (also cold-drawn), "hot-rolled", "forged".
    #[serde(default = "machined")]
    pub surface: String,
    /// The chance it survives: 0.5, 0.9, 0.95, 0.99, 0.999.
    #[serde(default = "half")]
    pub reliability: f64,
    /// A round part's diameter (mm) for the size factor (none: 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<f64>,
}

fn reversed() -> f64 {
    -1.0
}
fn million() -> f64 {
    1e6
}
fn machined() -> String {
    "machined".into()
}
fn half() -> f64 {
    0.5
}

impl Default for FatigueSpec {
    fn default() -> FatigueSpec {
        FatigueSpec { ratio: -1.0, cycles: 1e6, surface: machined(), reliability: 0.5, diameter: None }
    }
}

/// A fatigue estimate over a solved study.
#[derive(Clone, Debug, Serialize)]
pub struct Fatigue {
    /// Per cube: the Goodman safety factor at the wanted life.
    pub factors: Vec<f32>,
    /// The lowest, where, and the life (cycles) there (infinite below an endurance limit).
    pub min_factor: f64,
    pub at: [f64; 3],
    pub life: f64,
    /// The fatigue strength used at the wanted life (MPa), with the Marin
    /// factors that reduced the polished specimen's: surface, size, reliability.
    pub strength: f64,
    pub marin: [f64; 3],
    /// The stress amplitude and mean at the worst spot (MPa, von Mises).
    pub amplitude: f64,
    pub mean: f64,
    pub notes: Vec<String>,
}

/// Marin factors (Shigley, Mechanical Engineering Design, §6-9): surface
/// a·Sutᵇ (table 6-2), size 1.24·d^−0.107 (2.79–51 mm) or 1.51·d^−0.157
/// (51–254 mm), reliability (table 6-5).
pub fn marin(spec: &FatigueSpec, uts: f64) -> [f64; 3] {
    let (a, b) = match spec.surface.as_str() {
        "polished" => (1.0, 0.0),
        "ground" => (1.58, -0.085),
        "hot-rolled" => (57.7, -0.718),
        "forged" | "as-forged" => (272.0, -0.995),
        _ => (4.51, -0.265),
    };
    let ka = (a * uts.powf(b)).min(1.0);
    let kb = match spec.diameter {
        Some(d) if d > 51.0 => 1.51 * d.min(254.0).powf(-0.157),
        Some(d) if d > 2.79 => 1.24 * d.powf(-0.107),
        _ => 1.0,
    };
    let r = spec.reliability;
    let ke = if r >= 0.9999 { 0.702 } else if r >= 0.999 { 0.753 } else if r >= 0.99 { 0.814 } else if r >= 0.95 { 0.868 } else if r >= 0.9 { 0.897 } else { 1.0 };
    [ka, kb, ke]
}

/// Fatigue over a solved study whose loads are the peak of each cycle: per
/// cube, the von Mises stress split into amplitude (1 − R)/2 and mean
/// (1 + R)/2 of the peak (a mean that squeezes, by the stress's pressure,
/// counts as none), against the Marin-reduced S–N strength at the wanted
/// life by modified Goodman: n = 1 / (σa/Se + σm/Sut). One material: the studied part's.
pub fn fatigue(r: &Result_, material: &MaterialProps, spec: &FatigueSpec) -> Result<Fatigue, String> {
    let mut notes = vec![];
    let printed = material.family == "printed";
    let factors_m = if material.family == "metal" { marin(spec, material.uts) } else { [1.0, 1.0, marin(spec, material.uts)[2]] };
    let base = material.sn_strength(spec.cycles).ok_or_else(|| format!("no fatigue figures for {}: set its fatigue strength on the material", material.name))?;
    let k = factors_m[0] * factors_m[1] * factors_m[2];
    let se = base * k;
    // The S–N line through the reduced strength, for lives.
    let mut reduced = material.clone();
    reduced.fatigue_strength = material.fatigue_strength.map(|s| s * k);
    let ra = (1.0 - spec.ratio) / 2.0;
    let rm = (1.0 + spec.ratio) / 2.0;
    let mut worst = (f64::INFINITY, 0usize, 0.0, 0.0);
    let factors: Vec<f32> = r
        .cells
        .iter()
        .enumerate()
        .map(|(i, (_, vm))| {
            let vm = *vm as f64;
            let t = r.tensors.get(i).copied().unwrap_or([0.0; 6]);
            let pulls = t[0] + t[1] + t[2] >= 0.0;
            let (sa, sm) = (ra.abs() * vm, if pulls { rm * vm } else { 0.0 });
            let n = if sa <= 0.0 && sm <= 0.0 { 1e9 } else { 1.0 / (sa / se + sm / material.uts) };
            if n < worst.0 {
                worst = (n, i, sa, sm);
            }
            n.min(1e9) as f32
        })
        .collect();
    let (min_factor, i, sa, sm) = worst;
    // Life at the worst spot: its equivalent fully reversed stress σa/(1 − σm/Sut) on the S–N line.
    let equivalent = if sm < material.uts { sa / (1.0 - sm / material.uts) } else { f64::INFINITY };
    let life = reduced.sn_life(equivalent).unwrap_or(0.0);
    if printed {
        notes.push("Printed plastic: no endurance limit; its S–N line is an estimate (about a quarter of the tensile strength at 10⁶ cycles along the layers, less across them, and lower with voids, heat or moisture). Test a printed sample under the real load before trusting a long life.".into());
    } else if material.family != "metal" {
        notes.push(format!("{}: the fatigue figures are rough estimates.", material.name));
    } else if !material.endurance_limit {
        notes.push(format!("{} has no endurance limit: it fails at last however low the stress; the strength used is for {:.0e} cycles.", material.name, spec.cycles));
    }
    notes.push(format!(
        "Modified Goodman at {:.0e} cycles: fatigue strength {:.1} MPa (polished {:.1} × surface {:.2} × size {:.2} × reliability {:.2}), load ratio {}; stress concentrations count only as far as the mesh resolves them.",
        spec.cycles, se, base, factors_m[0], factors_m[1], factors_m[2], spec.ratio
    ));
    let c = r.cells.get(i).map(|c| c.0).unwrap_or([0; 3]);
    Ok(Fatigue {
        factors,
        min_factor,
        at: std::array::from_fn(|k| r.origin[k] + (c[k] as f64 + 0.5) * r.cell),
        life,
        strength: se,
        marin: factors_m,
        amplitude: sa,
        mean: sm,
        notes,
    })
}
