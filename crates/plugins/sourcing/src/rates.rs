//! What making a part costs, roughly: each process's shop rate, its setup
//! time, and how fast it goes. One settings entity per project; processes it
//! does not mention use typical defaults. Estimates are deliberately simple
//! and always labelled as estimates: a quote replaces them.

use crate::{bad, invalid};
use de_core::{Command, CommandError, ErrorCode, Persist, StableId, Tx, Value, bevy_ecs::{self, component::Component}, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a process's time (or cost) follows from the part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "basis", rename_all = "snake_case")]
pub enum Basis {
    /// Time from the part's volume (3D printing): cm³ per hour.
    Volume { cm3_per_hour: f64 },
    /// Time from the length of the outline cut (laser, router, sheet metal):
    /// mm per minute, plus seconds per pierce (one per closed outline).
    CutLength { mm_per_minute: f64, pierce_seconds: f64 },
    /// Time from the material removed from the bounding block (milling,
    /// turning): cm³ per minute.
    Removed { cm3_per_minute: f64 },
    /// A price per kg of finished part (welding, casting, glazing).
    Mass { per_kg: f64 },
    /// A price per part (cutting an extrusion to length).
    Each { per_part: f64 },
}

/// One process's rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessRate {
    /// Shop rate, per hour of machine time.
    pub per_hour: f64,
    /// Setup per batch (programming, fixturing, first article), hours:
    /// spread over the batch, so larger runs cost less each.
    pub setup_hours: f64,
    pub basis: Basis,
}

/// Process rates for the project. Absent processes use [`default_rate`].
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rates {
    /// ISO 4217 code every cost is reckoned in (default USD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub processes: BTreeMap<String, ProcessRate>,
    /// Exchange rates: what one unit of another currency is worth in this
    /// one (`EUR: 1.08` when costs are in USD). Prices listed or quoted in a
    /// currency given here are converted; others are flagged, not converted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exchange: BTreeMap<String, f64>,
}

impl Persist for Rates {
    const KIND: &'static str = "sourcing.rates";
}

impl Rates {
    pub fn currency(&self) -> String {
        self.currency.clone().unwrap_or_else(|| "USD".into())
    }

    /// What one unit of `currency` is worth in the project's currency, if known.
    pub fn to_own(&self, currency: &str) -> Option<f64> {
        if currency.eq_ignore_ascii_case(&self.currency()) {
            return Some(1.0);
        }
        self.exchange.get(&currency.to_ascii_uppercase()).copied()
    }

    /// The rate for a process: the project's, else the typical default.
    pub fn rate(&self, process: &str) -> Option<ProcessRate> {
        self.processes.get(process).cloned().or_else(|| default_rate(process))
    }

    /// Every process with its rate in force and whether it is the default.
    pub fn table(&self) -> Vec<(String, ProcessRate, bool)> {
        crate::PROCESSES
            .iter()
            .filter(|p| **p != "buy")
            .filter_map(|p| match self.processes.get(*p) {
                Some(r) => Some((p.to_string(), r.clone(), false)),
                None => default_rate(p).map(|r| (p.to_string(), r, true)),
            })
            .collect()
    }
}

/// Typical rates (USD) for a small job shop or print farm: a starting point
/// for estimates, nothing more.
pub fn default_rate(process: &str) -> Option<ProcessRate> {
    let r = |per_hour: f64, setup_hours: f64, basis: Basis| Some(ProcessRate { per_hour, setup_hours, basis });
    match process {
        "3d_print" => r(4.0, 0.15, Basis::Volume { cm3_per_hour: 12.0 }),
        "laser" => r(90.0, 0.25, Basis::CutLength { mm_per_minute: 1500.0, pierce_seconds: 1.0 }),
        "cnc_router" => r(75.0, 0.5, Basis::CutLength { mm_per_minute: 2500.0, pierce_seconds: 4.0 }),
        "sheet_metal" => r(90.0, 0.5, Basis::CutLength { mm_per_minute: 2000.0, pierce_seconds: 1.5 }),
        "cnc_mill" => r(85.0, 1.0, Basis::Removed { cm3_per_minute: 8.0 }),
        "lathe" => r(70.0, 0.5, Basis::Removed { cm3_per_minute: 12.0 }),
        "weld" => r(65.0, 0.5, Basis::Mass { per_kg: 6.0 }),
        "cast" => r(0.0, 0.0, Basis::Mass { per_kg: 9.0 }),
        "extrusion" => r(60.0, 0.25, Basis::Each { per_part: 1.5 }),
        "glazing" => r(0.0, 0.0, Basis::Mass { per_kg: 4.0 }),
        _ => None,
    }
}

impl ProcessRate {
    fn check(&self, name: &str) -> Result<(), CommandError> {
        let ok = |v: f64| v.is_finite() && v >= 0.0;
        let pos = |v: f64| v.is_finite() && v > 0.0;
        let basis_ok = match &self.basis {
            Basis::Volume { cm3_per_hour } => pos(*cm3_per_hour),
            Basis::CutLength { mm_per_minute, pierce_seconds } => pos(*mm_per_minute) && ok(*pierce_seconds),
            Basis::Removed { cm3_per_minute } => pos(*cm3_per_minute),
            Basis::Mass { per_kg } => ok(*per_kg),
            Basis::Each { per_part } => ok(*per_part),
        };
        if !(ok(self.per_hour) && ok(self.setup_hours) && basis_ok) {
            return Err(invalid(format!("{name}: rates are at least 0 and speeds greater than 0")));
        }
        Ok(())
    }
}

/// `sourcing.rates.set`: sets the project's process rates (merged into the
/// ones it has; `reset` lists processes to put back to the defaults) and the
/// currency costs are reckoned in.
#[derive(Deserialize, JsonSchema)]
pub struct SetRates {
    pub currency: Option<String>,
    #[serde(default)]
    pub processes: BTreeMap<String, ProcessRate>,
    #[serde(default)]
    pub reset: Vec<String>,
    /// Exchange rates to merge in: what one unit of each currency is worth
    /// in the project's (`{ "EUR": 1.08 }`); 0 removes one.
    #[serde(default)]
    pub exchange: BTreeMap<String, f64>,
}

impl Command for SetRates {
    const NAME: &'static str = "sourcing.rates.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let found: Option<(StableId, Rates)> = crate::all::<Rates>(tx.world()).into_iter().next();
        let (id, mut rates) = match found {
            Some((id, r)) => (id, r),
            None => (tx.create(), Rates::default()),
        };
        if let Some(c) = self.currency {
            rates.currency = if c.trim().is_empty() { None } else { Some(crate::check_currency(&c)?) };
        }
        for (name, rate) in self.processes {
            if !crate::PROCESSES.contains(&name.as_str()) || name == "buy" {
                return Err(bad(ErrorCode::InvalidParameters, format!("“{name}” is not a process that makes parts")));
            }
            rate.check(&name)?;
            rates.processes.insert(name, rate);
        }
        for name in self.reset {
            rates.processes.remove(&name);
        }
        for (code, rate) in self.exchange {
            let code = crate::check_currency(&code)?;
            if !rate.is_finite() || rate < 0.0 {
                return Err(bad(ErrorCode::InvalidParameters, format!("the exchange rate for {code} must be a positive number (0 removes it)")));
            }
            if rate == 0.0 {
                rates.exchange.remove(&code);
            } else {
                rates.exchange.insert(code, rate);
            }
        }
        tx.set(id, rates)?;
        Ok(json!({ "id": id }))
    }
}

/// The project's rates (defaults when it has none).
pub fn rates_of(world: &de_core::bevy_ecs::world::World) -> Rates {
    crate::all::<Rates>(world).into_iter().next().map(|(_, r)| r).unwrap_or_default()
}

/// What one part is, for estimating.
#[derive(Clone, Debug, Default)]
pub struct PartFacts {
    pub process: Option<String>,
    /// Overall size, largest first (mm).
    pub size: [f64; 3],
    pub volume_mm3: f64,
    pub mass_kg: Option<f64>,
    /// kg/m³.
    pub density: Option<f64>,
    /// Through-cut outline length (mm) and outline count, for sheet parts.
    pub cut: Option<(f64, usize)>,
}

/// An estimated cost of one part, in the rates' currency.
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct Estimate {
    /// Material for one part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<f64>,
    /// Process cost for one part (machine time × rate, or a per-kg/per-part price).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<f64>,
    /// Setup for one batch, spread over the batch.
    pub setup: f64,
    /// Machine time for one part, hours.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours: Option<f64>,
    /// How it was worked out, in words.
    pub basis: Vec<String>,
}

impl Estimate {
    /// Cost of one part in a batch of `batch` parts; None if nothing is known.
    pub fn unit(&self, batch: u32) -> Option<f64> {
        if self.material.is_none() && self.process.is_none() {
            return None;
        }
        Some(self.material.unwrap_or(0.0) + self.process.unwrap_or(0.0) + self.setup / batch.max(1) as f64)
    }

    /// Whether both material and process were priced.
    pub fn complete(&self) -> bool {
        self.material.is_some() && self.process.is_some()
    }
}

fn money(v: f64) -> String {
    format!("{v:.2}")
}

/// Estimates one part: material from its price (stock for machined parts,
/// sheet share for sheet parts, bar share for extrusions, else weight), and
/// process from the rate.
pub fn estimate(part: &PartFacts, cost: Option<&de_solid::MaterialCost>, rate: Option<&ProcessRate>) -> Estimate {
    let mut e = Estimate::default();
    let [l, w, t] = part.size;
    let process = part.process.as_deref().unwrap_or("");
    let sheet_part = matches!(process, "laser" | "cnc_router" | "sheet_metal");
    let machined = matches!(process, "cnc_mill" | "lathe");

    // Material.
    if let Some(c) = cost {
        let block_kg = part.density.map(|d| l * w * t / 1e9 * d);
        if let (true, Some(s)) = (sheet_part, &c.sheet) {
            // The part's rectangle as a share of the sheet, with 15 % lost to nesting.
            let share = (l * w) / (s.length * s.width) / 0.85;
            e.material = Some(s.price * share);
            e.basis.push(format!("material: {:.0} × {:.0} mm of a {:.0} × {:.0} sheet at {} (+15 % nesting)", l, w, s.length, s.width, money(s.price)));
        } else if let (true, Some(b)) = (process == "extrusion", &c.bar) {
            e.material = Some(b.price * l / b.length);
            e.basis.push(format!("material: {:.0} mm of a {:.0} mm bar at {}", l, b.length, money(b.price)));
        } else if let Some(per_kg) = c.per_kg {
            // Machined and sheet parts are cut from a block or plate their size.
            let (kg, what) = if machined || sheet_part { (block_kg, "stock block") } else { (part.mass_kg, "part") };
            if let Some(kg) = kg {
                e.material = Some(kg * per_kg);
                e.basis.push(format!("material: {:.3} kg {what} at {}/kg", kg, money(per_kg)));
            }
        }
    }

    // Process.
    if let Some(r) = rate {
        e.setup = r.setup_hours * r.per_hour;
        match &r.basis {
            Basis::Volume { cm3_per_hour } => {
                let h = part.volume_mm3 / 1000.0 / cm3_per_hour;
                e.hours = Some(h);
                e.process = Some(h * r.per_hour);
                e.basis.push(format!("print: {:.1} cm³ at {cm3_per_hour} cm³/h = {:.2} h", part.volume_mm3 / 1000.0, h));
            }
            Basis::CutLength { mm_per_minute, pierce_seconds } => {
                if let Some((len, pierces)) = part.cut {
                    let h = (len / mm_per_minute + pierces as f64 * pierce_seconds / 60.0) / 60.0;
                    e.hours = Some(h);
                    e.process = Some(h * r.per_hour);
                    e.basis.push(format!("cut: {:.0} mm at {mm_per_minute} mm/min + {pierces} pierce{} = {:.1} min", len, if pierces == 1 { "" } else { "s" }, h * 60.0));
                }
            }
            Basis::Removed { cm3_per_minute } => {
                let removed = ((l * w * t - part.volume_mm3) / 1000.0).max(0.0);
                let h = removed / cm3_per_minute / 60.0;
                e.hours = Some(h);
                e.process = Some(h * r.per_hour);
                e.basis.push(format!("machining: {:.1} cm³ removed from the {:.0} × {:.0} × {:.0} block at {cm3_per_minute} cm³/min = {:.1} min", removed, l, w, t, h * 60.0));
            }
            Basis::Mass { per_kg } => {
                if let Some(kg) = part.mass_kg {
                    e.process = Some(kg * per_kg);
                    e.basis.push(format!("process: {:.3} kg at {}/kg", kg, money(*per_kg)));
                }
            }
            Basis::Each { per_part } => {
                e.process = Some(*per_part);
                e.basis.push(format!("process: {} per part", money(*per_part)));
            }
        }
        if e.setup > 0.0 {
            e.basis.push(format!("setup: {} h at {}/h per batch", r.setup_hours, money(r.per_hour)));
        }
    }
    e
}
