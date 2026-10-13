//! The physical board stackup: what lies between the copper layers.
//!
//! A board of `n` copper layers (an even count, 2 to 32) has `n − 1`
//! dielectrics between them, each a *core* (cured laminate, copper on both
//! sides) or *prepreg* (the bonding sheets pressed between cores), with a
//! material, thickness, relative permittivity and loss tangent, plus a copper
//! weight per layer. When the board gives none, a standard stackup is made for
//! its layer count and finished thickness (prepreg next to the outer layers,
//! cores and prepregs alternating inside, as fabs build them).
//!
//! The stackup is what blind and micro via checks read (a micro via may only
//! cross one prepreg, never a core), what the fab files describe and what the
//! KiCad export writes. Impedance work on it is parked.

use crate::{Board, bad, board_of};
use de_core::{Command, CommandError, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Copper foil thickness per ounce per square foot (mm).
pub const OZ_MM: f64 = 0.035;
/// Solder mask thickness each side (mm).
pub const MASK_MM: f64 = 0.01;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DielectricKind {
    #[default]
    Core,
    Prepreg,
}

/// One dielectric layer between two copper layers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Dielectric {
    #[serde(default)]
    pub kind: DielectricKind,
    /// mm.
    pub thickness: f64,
    /// "FR4", "FR408HR", "Rogers 4350B", "Polyimide", "Isola 370HR" …
    #[serde(default = "fr4")]
    pub material: String,
    /// Relative permittivity (εr); FR4 4.5 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epsilon_r: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_tangent: Option<f64>,
}

fn fr4() -> String {
    "FR4".into()
}

/// A stackup the person (or the AI) chose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stackup {
    /// Top down, one between each pair of neighbouring copper layers (layers − 1).
    pub dielectrics: Vec<Dielectric>,
    /// Copper weight per layer top down (oz); empty: the board's outer and inner weights.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copper_oz: Vec<f64>,
}

impl Stackup {
    pub fn check(&self, layers: usize) -> Result<(), String> {
        if self.dielectrics.len() + 1 != layers {
            return Err(format!("a {layers}-layer stackup has {} dielectrics (one between each pair of copper layers); {} given", layers - 1, self.dielectrics.len()));
        }
        for (i, d) in self.dielectrics.iter().enumerate() {
            if !(d.thickness.is_finite() && (0.01..=5.0).contains(&d.thickness)) {
                return Err(format!("dielectric {} is 0.01 to 5 mm thick", i + 1));
            }
            if d.epsilon_r.is_some_and(|e| !(e.is_finite() && (1.0..=20.0).contains(&e))) {
                return Err(format!("dielectric {}: εr is 1 to 20", i + 1));
            }
            if d.loss_tangent.is_some_and(|t| !(t.is_finite() && (0.0..=1.0).contains(&t))) {
                return Err(format!("dielectric {}: loss tangent is 0 to 1", i + 1));
            }
            if d.material.trim().is_empty() {
                return Err(format!("dielectric {} needs a material", i + 1));
            }
        }
        if !self.copper_oz.is_empty() && self.copper_oz.len() != layers {
            return Err(format!("copper_oz gives one weight per copper layer ({layers})"));
        }
        if self.copper_oz.iter().any(|w| !(w.is_finite() && (0.25..=6.0).contains(w))) {
            return Err("copper weights are 0.25 to 6 oz".into());
        }
        Ok(())
    }
}

/// One row of the resolved stackup, top to bottom.
#[derive(Clone, Debug, Serialize)]
pub struct StackRow {
    /// "silk", "mask", "copper", "core", "prepreg".
    pub kind: &'static str,
    /// "top", "in1", … for copper; "Dielectric 1" …; "Top mask".
    pub name: String,
    pub thickness: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epsilon_r: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_tangent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copper_oz: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StackupView {
    pub rows: Vec<StackRow>,
    /// The dielectrics as used (given, or the standard ones).
    pub dielectrics: Vec<Dielectric>,
    pub copper_oz: Vec<f64>,
    /// Sum of every row (mm).
    pub total: f64,
    /// The person set it (otherwise it is the standard stackup).
    pub custom: bool,
    pub finish: String,
}

impl StackupView {
    /// Whether any dielectric between copper layers `a` and `b` (indices) is a core.
    pub fn crosses_core(&self, a: usize, b: usize) -> bool {
        let (lo, hi) = (a.min(b), a.max(b));
        self.dielectrics.get(lo..hi).is_some_and(|ds| ds.iter().any(|d| d.kind == DielectricKind::Core))
    }
    /// Distance from the top of copper layer `a` to copper layer `b` (mm), for a via barrel.
    pub fn depth(&self, a: usize, b: usize) -> f64 {
        let (lo, hi) = (a.min(b), a.max(b));
        self.dielectrics.get(lo..hi).map(|ds| ds.iter().map(|d| d.thickness).sum::<f64>()).unwrap_or(0.0) + (lo..=hi).map(|i| self.copper_oz.get(i).copied().unwrap_or(1.0) * OZ_MM).sum::<f64>()
    }
}

/// The standard stackup for `layers` copper layers and a finished `thickness`:
/// prepreg against the outer layers and alternating inside, cores taking up the rest.
pub fn standard(layers: usize, thickness: f64, outer_oz: f64, inner_oz: f64) -> (Vec<Dielectric>, Vec<f64>) {
    let n = layers.clamp(2, 32);
    let copper: Vec<f64> = (0..n).map(|i| if i == 0 || i == n - 1 { outer_oz } else { inner_oz }).collect();
    let cu: f64 = copper.iter().map(|w| w * OZ_MM).sum();
    let left = (thickness - cu - 2.0 * MASK_MM).max(0.05 * (n - 1) as f64);
    let kinds: Vec<DielectricKind> = (0..n - 1).map(|i| if n == 2 || i % 2 == 1 { DielectricKind::Core } else { DielectricKind::Prepreg }).collect();
    let prepregs = kinds.iter().filter(|k| **k == DielectricKind::Prepreg).count();
    let cores = kinds.len() - prepregs;
    // Prepreg 0.1 mm (thinner when the board is too thin for it), the cores share the rest.
    let mut pp = 0.1_f64;
    if cores > 0 && left - pp * prepregs as f64 < 0.1 * cores as f64 {
        pp = (left / (prepregs + cores) as f64).max(0.02);
    }
    let core = if cores > 0 { ((left - pp * prepregs as f64) / cores as f64).max(0.02) } else { 0.0 };
    let r = |v: f64| (v * 10000.0).round() / 10000.0;
    let ds = kinds
        .into_iter()
        .map(|k| Dielectric { kind: k, thickness: r(if k == DielectricKind::Core { core } else { pp }), material: fr4(), epsilon_r: Some(if k == DielectricKind::Core { 4.5 } else { 4.4 }), loss_tangent: Some(0.02) })
        .collect();
    (ds, copper)
}

pub fn resolve(board: &Board) -> StackupView {
    let n = board.copper_layers().len();
    let custom = board.stackup.as_ref().filter(|s| s.check(n).is_ok());
    let (std_d, std_cu) = standard(n, board.thickness, board.copper_oz, board.inner_copper_oz);
    let dielectrics = custom.map(|s| s.dielectrics.clone()).unwrap_or(std_d);
    let copper_oz = custom.map(|s| s.copper_oz.clone()).filter(|c| !c.is_empty()).unwrap_or(std_cu);
    let names = board.copper_layers();
    let mut rows = vec![
        StackRow { kind: "silk", name: "Top silkscreen".into(), thickness: 0.0, material: Some(format!("{} ink", board.silk)), epsilon_r: None, loss_tangent: None, copper_oz: None },
        StackRow { kind: "mask", name: "Top mask".into(), thickness: MASK_MM, material: Some(format!("{} solder mask", board.mask)), epsilon_r: Some(3.3), loss_tangent: None, copper_oz: None },
    ];
    for i in 0..n {
        rows.push(StackRow { kind: "copper", name: names[i].to_string(), thickness: copper_oz[i] * OZ_MM, material: Some("Copper".into()), epsilon_r: None, loss_tangent: None, copper_oz: Some(copper_oz[i]) });
        if let Some(d) = dielectrics.get(i) {
            rows.push(StackRow {
                kind: if d.kind == DielectricKind::Core { "core" } else { "prepreg" },
                name: format!("Dielectric {}", i + 1),
                thickness: d.thickness,
                material: Some(d.material.clone()),
                epsilon_r: Some(d.epsilon_r.unwrap_or(4.5)),
                loss_tangent: d.loss_tangent,
                copper_oz: None,
            });
        }
    }
    rows.push(StackRow { kind: "mask", name: "Bottom mask".into(), thickness: MASK_MM, material: Some(format!("{} solder mask", board.mask)), epsilon_r: Some(3.3), loss_tangent: None, copper_oz: None });
    rows.push(StackRow { kind: "silk", name: "Bottom silkscreen".into(), thickness: 0.0, material: Some(format!("{} ink", board.silk)), epsilon_r: None, loss_tangent: None, copper_oz: None });
    let total = rows.iter().map(|r| r.thickness).sum::<f64>();
    StackupView { rows, dielectrics, copper_oz, total: (total * 1e4).round() / 1e4, custom: custom.is_some(), finish: board.finish.clone() }
}

/// `pcb.stackup.set`: the board's physical stackup. Give `layers` to change
/// the copper layer count (2 to 32, even), `dielectrics` (top down, layers − 1
/// of them) and `copper_oz` (per layer) to set it; `standard: true` goes back
/// to the standard stackup for the layer count and thickness.
#[derive(Deserialize, JsonSchema)]
pub struct SetStackup {
    pub board: StableId,
    pub layers: Option<u8>,
    pub thickness: Option<f64>,
    pub dielectrics: Option<Vec<Dielectric>>,
    pub copper_oz: Option<Vec<f64>>,
    #[serde(default)]
    pub standard: bool,
    /// Set the board's finished thickness to the stackup's total.
    #[serde(default)]
    pub fit_thickness: bool,
}

impl Command for SetStackup {
    const NAME: &'static str = "pcb.stackup.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut update = json!({ "target": self.board });
        if let Some(l) = self.layers {
            update["layers"] = json!(l);
        }
        if let Some(t) = self.thickness {
            update["thickness"] = json!(t);
        }
        if self.layers.is_some() || self.thickness.is_some() {
            tx.run("pcb.board.update", update)?;
        }
        let mut b: Board = board_of(tx, self.board)?;
        let n = b.copper_layers().len();
        if self.standard {
            b.stackup = None;
        } else if self.dielectrics.is_some() || self.copper_oz.is_some() {
            let current = resolve(&b);
            let s = Stackup { dielectrics: self.dielectrics.unwrap_or(current.dielectrics), copper_oz: self.copper_oz.unwrap_or_default() };
            s.check(n).map_err(bad)?;
            if let (Some(first), Some(last)) = (s.copper_oz.first(), s.copper_oz.last()) {
                b.copper_oz = first.max(*last);
                if n > 2 {
                    b.inner_copper_oz = s.copper_oz[1];
                }
            }
            b.stackup = Some(s);
        }
        if self.fit_thickness {
            let v = resolve(&b);
            b.thickness = ((v.total * 100.0).round() / 100.0).clamp(0.2, 8.0);
        }
        crate::check_board(&b)?;
        tx.set(self.board, b.clone())?;
        crate::model3d::sync(tx, self.board)?;
        let v = resolve(&b);
        Ok(json!({ "board": self.board, "layers": n, "total": v.total, "thickness": b.thickness, "custom": v.custom, "rows": v.rows }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<SetStackup>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_stackups_add_up_to_the_board() {
        for n in [2usize, 4, 6, 8, 12, 32] {
            let t = if n >= 12 { 3.2 } else { 1.6 };
            let (d, cu) = standard(n, t, 1.0, 0.5);
            assert_eq!(d.len(), n - 1);
            assert_eq!(cu.len(), n);
            let total: f64 = d.iter().map(|x| x.thickness).sum::<f64>() + cu.iter().map(|w| w * OZ_MM).sum::<f64>() + 2.0 * MASK_MM;
            assert!((total - t).abs() < 0.01, "{n} layers: {total} vs {t}");
            if n > 2 {
                assert_eq!(d[0].kind, DielectricKind::Prepreg, "prepreg next to the outer copper");
                assert_eq!(d[n - 2].kind, DielectricKind::Prepreg);
                assert_eq!(d[1].kind, DielectricKind::Core);
            }
        }
    }

    #[test]
    fn crosses_core_reads_the_dielectrics() {
        let (d, cu) = standard(6, 1.6, 1.0, 0.5);
        let v = StackupView { rows: vec![], dielectrics: d, copper_oz: cu, total: 1.6, custom: false, finish: String::new() };
        assert!(!v.crosses_core(0, 1), "top to in1 is prepreg");
        assert!(v.crosses_core(1, 2), "in1 to in2 is a core");
        assert!(v.crosses_core(0, 2));
        assert!(v.depth(0, 1) > 0.1);
    }

    #[test]
    fn a_stackup_must_match_the_layer_count() {
        let (d, _) = standard(4, 1.6, 1.0, 0.5);
        let s = Stackup { dielectrics: d.clone(), copper_oz: vec![] };
        assert!(s.check(4).is_ok());
        assert!(s.check(6).is_err());
        let bad = Stackup { dielectrics: d, copper_oz: vec![1.0, 1.0] };
        assert!(bad.check(4).is_err());
    }
}
