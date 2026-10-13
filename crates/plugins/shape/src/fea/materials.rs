//! The material library: typical engineering properties by material name,
//! each with where its figures come from, and the person's own figures laid
//! over them (`solid.material.set_properties`).
//!
//! The figures are design guidance, not a datasheet: a real alloy, temper or
//! print setting can differ a lot. Units: MPa, kg/m³, W/m·K, J/kg·K, 1/K, °C.

use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

/// Everything the studies need to know about a material.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaterialProps {
    /// The library entry it came from ("Steel", "PLA, printed"…).
    pub name: String,
    /// "metal", "polymer", "printed", "wood", "glass" or "concrete".
    pub family: String,
    /// Young's modulus (MPa) and Poisson's ratio.
    pub e: f64,
    pub nu: f64,
    pub density: f64,
    pub yield_strength: f64,
    /// Ultimate tensile strength (MPa).
    pub uts: f64,
    /// Elongation at break (%).
    pub elongation: f64,
    /// Slope past yield (MPa) for bilinear plasticity.
    pub tangent_modulus: f64,
    /// Fully reversed fatigue strength (MPa) of a polished specimen at
    /// `fatigue_cycles`; with `endurance_limit`, it holds for any longer life.
    pub fatigue_strength: Option<f64>,
    pub fatigue_cycles: f64,
    pub endurance_limit: bool,
    pub conductivity: f64,
    pub specific_heat: f64,
    pub expansion: f64,
    /// Highest temperature for structural use (°C), where it matters (glass transition of plastics).
    pub max_temperature: Option<f64>,
    /// Where the figures come from.
    pub source: String,
    /// Which figures the person set (the rest are the library's).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edited: Vec<String>,
}

struct Entry {
    keys: &'static [&'static str],
    name: &'static str,
    family: &'static str,
    // e, nu, density, yield, uts, elongation, tangent
    mech: [f64; 7],
    // fatigue strength (0: no data), cycles, endurance limit
    fatigue: (f64, f64, bool),
    // conductivity, specific heat, expansion (µm/m·K)
    thermal: [f64; 3],
    max_temperature: Option<f64>,
    source: &'static str,
}

const PRINTED_FATIGUE: &str = " Printed parts have no fatigue limit; their fatigue strength at 10⁶ cycles is taken as about a quarter of the tensile strength along the layers (FDM tests such as Afrose et al. 2016 for PLA report roughly 20–30 %), and less across layers.";

/// Matched in order: the first entry with a key in the name wins ("stainless" before "steel").
const LIBRARY: &[Entry] = &[
    Entry { keys: &["stainless"], name: "Stainless steel (304)", family: "metal", mech: [193_000.0, 0.29, 8000.0, 215.0, 505.0, 40.0, 2_000.0], fatigue: (240.0, 1e6, true), thermal: [16.0, 500.0, 17.3], max_temperature: None, source: "AISI 304 annealed, typical (ASTM A240 minimum yield 205 MPa, tensile 515 MPa; ASM Handbook Vol. 1)." },
    Entry { keys: &["steel"], name: "Steel (structural, S235/A36)", family: "metal", mech: [200_000.0, 0.29, 7850.0, 250.0, 400.0, 20.0, 2_000.0], fatigue: (200.0, 1e6, true), thermal: [50.0, 486.0, 12.0], max_temperature: None, source: "ASTM A36 / EN S235 minimums (yield 250 MPa, tensile 400 MPa); endurance limit 0.5 × tensile strength (Shigley, Mechanical Engineering Design, eq. 6-8)." },
    Entry { keys: &["alumin"], name: "Aluminium (6061-T6)", family: "metal", mech: [69_000.0, 0.33, 2700.0, 240.0, 290.0, 8.0, 700.0], fatigue: (96.0, 5e8, false), thermal: [167.0, 896.0, 23.1], max_temperature: Some(150.0), source: "6061-T6, ASTM B209 minimums (yield 240 MPa, tensile 290 MPa); fatigue strength 96 MPa at 5×10⁸ cycles (Aluminum Association / ASM). Aluminium has no endurance limit." },
    Entry { keys: &["titan"], name: "Titanium (Ti-6Al-4V)", family: "metal", mech: [114_000.0, 0.34, 4430.0, 880.0, 950.0, 14.0, 1_500.0], fatigue: (510.0, 1e7, true), thermal: [7.0, 526.0, 8.6], max_temperature: Some(400.0), source: "Ti-6Al-4V grade 5 annealed, typical (ASM Handbook Vol. 2; unnotched fatigue about 510 MPa at 10⁷)." },
    Entry { keys: &["brass"], name: "Brass", family: "metal", mech: [100_000.0, 0.34, 8500.0, 200.0, 340.0, 25.0, 1_000.0], fatigue: (130.0, 1e8, false), thermal: [110.0, 380.0, 19.0], max_temperature: None, source: "Free-machining brass (C360 / CW614N) typical, conservative end of the range (Copper Development Association)." },
    Entry { keys: &["copper"], name: "Copper (annealed)", family: "metal", mech: [117_000.0, 0.34, 8960.0, 70.0, 220.0, 45.0, 800.0], fatigue: (70.0, 1e8, false), thermal: [400.0, 385.0, 16.5], max_temperature: None, source: "C11000 annealed, typical (ASTM B152; Copper Development Association)." },
    Entry { keys: &["pla"], name: "PLA, printed", family: "printed", mech: [3_500.0, 0.36, 1240.0, 50.0, 50.0, 4.0, 0.0], fatigue: (12.5, 1e6, false), thermal: [0.13, 1800.0, 68.0], max_temperature: Some(55.0), source: "Typical FDM PLA along the layers at full infill (manufacturer datasheets range 35–60 MPa tensile); softens near its 60 °C glass transition." },
    Entry { keys: &["petg"], name: "PETG, printed", family: "printed", mech: [2_100.0, 0.38, 1270.0, 50.0, 50.0, 8.0, 0.0], fatigue: (12.5, 1e6, false), thermal: [0.2, 1250.0, 60.0], max_temperature: Some(70.0), source: "Typical FDM PETG along the layers (manufacturer datasheets); glass transition about 80 °C." },
    Entry { keys: &["abs", "asa"], name: "ABS, printed", family: "printed", mech: [2_200.0, 0.35, 1040.0, 40.0, 40.0, 6.0, 0.0], fatigue: (10.0, 1e6, false), thermal: [0.17, 1400.0, 90.0], max_temperature: Some(90.0), source: "Typical FDM ABS along the layers (manufacturer datasheets); glass transition about 105 °C." },
    Entry { keys: &["nylon", "pa12", "pa6", "polyamide"], name: "Nylon", family: "polymer", mech: [2_700.0, 0.39, 1140.0, 70.0, 75.0, 30.0, 300.0], fatigue: (20.0, 1e7, false), thermal: [0.25, 1700.0, 80.0], max_temperature: Some(120.0), source: "PA6 dry as moulded, typical (MatWeb overview); moisture lowers stiffness and strength markedly." },
    Entry { keys: &["acetal", "pom", "delrin"], name: "Acetal (POM)", family: "polymer", mech: [2_800.0, 0.35, 1410.0, 65.0, 70.0, 30.0, 300.0], fatigue: (30.0, 1e7, false), thermal: [0.31, 1460.0, 110.0], max_temperature: Some(100.0), source: "POM homopolymer typical (Delrin 100 datasheet ranges); fatigue about 30 MPa at 10⁷." },
    Entry { keys: &["oak"], name: "Oak", family: "wood", mech: [11_000.0, 0.3, 750.0, 40.0, 90.0, 1.0, 0.0], fatigue: (0.0, 1e6, false), thermal: [0.17, 2000.0, 5.0], max_temperature: None, source: "White oak along the grain, typical (USDA Wood Handbook, FPL-GTR-190); much weaker across the grain." },
    Entry { keys: &["ply"], name: "Plywood", family: "wood", mech: [9_000.0, 0.3, 600.0, 30.0, 40.0, 1.0, 0.0], fatigue: (0.0, 1e6, false), thermal: [0.13, 1880.0, 7.0], max_temperature: None, source: "Birch plywood, in-plane typical (USDA Wood Handbook)." },
    Entry { keys: &["mdf"], name: "MDF", family: "wood", mech: [3_500.0, 0.25, 750.0, 15.0, 18.0, 1.0, 0.0], fatigue: (0.0, 1e6, false), thermal: [0.12, 1700.0, 10.0], max_temperature: None, source: "Standard MDF (EN 622-5 typical bending strength and modulus)." },
    Entry { keys: &["glass"], name: "Glass (soda-lime)", family: "glass", mech: [70_000.0, 0.22, 2500.0, 33.0, 33.0, 0.0, 0.0], fatigue: (0.0, 1e6, false), thermal: [1.0, 840.0, 9.0], max_temperature: None, source: "Annealed soda-lime glass, design strength (it breaks, it does not yield; EN 572)." },
    Entry { keys: &["concrete"], name: "Concrete", family: "concrete", mech: [30_000.0, 0.2, 2400.0, 3.0, 3.0, 0.0, 0.0], fatigue: (0.0, 1e6, false), thermal: [1.7, 880.0, 12.0], max_temperature: None, source: "Normal-weight C30 concrete (EN 1992-1-1); the strength given is its tensile strength." },
];

impl Entry {
    fn props(&self) -> MaterialProps {
        let [e, nu, density, yield_strength, uts, elongation, tangent_modulus] = self.mech;
        let (fatigue, cycles, endurance_limit) = self.fatigue;
        let [conductivity, specific_heat, expansion] = self.thermal;
        let source = if self.family == "printed" { format!("{}{PRINTED_FATIGUE}", self.source) } else { self.source.to_string() };
        MaterialProps {
            name: self.name.into(),
            family: self.family.into(),
            e,
            nu,
            density,
            yield_strength,
            uts,
            elongation,
            tangent_modulus,
            fatigue_strength: (fatigue > 0.0).then_some(fatigue),
            fatigue_cycles: cycles,
            endurance_limit,
            conductivity,
            specific_heat,
            expansion: expansion * 1e-6,
            max_temperature: self.max_temperature,
            source,
            edited: vec![],
        }
    }
}

/// Typical properties for a material name (case and extra words ignored:
/// "PLA (Bambu, black)" is PLA), or None when the name is not known.
pub fn lookup(material: &str) -> Option<MaterialProps> {
    let m = material.to_lowercase();
    LIBRARY.iter().find(|e| e.keys.iter().any(|k| m.contains(k))).map(Entry::props)
}

/// Every entry in the library.
pub fn library() -> Vec<MaterialProps> {
    LIBRARY.iter().map(Entry::props).collect()
}

/// The library's figures for a name with the person's own laid over them
/// (`density`: the material's own density, which sets its mass, when it has one).
pub fn resolve(material: &str, own: Option<&de_solid::EngineeringProps>, density: Option<f64>) -> Option<MaterialProps> {
    let mut p = match lookup(material) {
        Some(p) => p,
        // An unknown name works when the person gave the figures a study needs.
        None => {
            let o = own?;
            let (e, y) = (o.e?, o.yield_strength?);
            MaterialProps {
                name: material.into(),
                family: "custom".into(),
                e,
                nu: o.nu.unwrap_or(0.3),
                density: density.unwrap_or(0.0),
                yield_strength: y,
                uts: o.uts.unwrap_or(y),
                elongation: 0.0,
                tangent_modulus: 0.0,
                fatigue_strength: None,
                fatigue_cycles: 1e6,
                endurance_limit: false,
                conductivity: 0.0,
                specific_heat: 0.0,
                expansion: 0.0,
                max_temperature: None,
                source: String::new(),
                edited: vec![],
            }
        }
    };
    if let Some(d) = density.filter(|d| *d > 0.0) {
        if (d - p.density).abs() > 1e-6 * p.density.max(1.0) {
            p.edited.push("density".into());
        }
        p.density = d;
    }
    let Some(o) = own else { return Some(p) };
    macro_rules! take {
        ($field:ident, $what:expr) => {
            if let Some(v) = o.$field {
                p.$field = v;
                p.edited.push($what.into());
            }
        };
    }
    take!(e, "E");
    take!(nu, "ν");
    take!(yield_strength, "yield strength");
    take!(uts, "tensile strength");
    take!(tangent_modulus, "tangent modulus");
    take!(fatigue_cycles, "fatigue cycles");
    take!(conductivity, "conductivity");
    take!(specific_heat, "specific heat");
    take!(expansion, "expansion");
    if let Some(v) = o.fatigue_strength {
        p.fatigue_strength = Some(v);
        p.edited.push("fatigue strength".into());
    }
    if let Some(s) = &o.source {
        p.source = format!("{s} (set on this material; the rest: {})", p.source);
    } else if !p.edited.is_empty() {
        p.source = format!("{} — edited here: {}.", p.source, p.edited.join(", "));
    }
    Some(p)
}

impl MaterialProps {
    /// The figures the stress solver needs.
    pub fn elastic(&self) -> super::Elastic {
        super::Elastic { e: self.e, nu: self.nu, yield_strength: self.yield_strength, alpha: self.expansion, density: self.density, tangent: self.tangent_modulus }
    }

    /// Fully reversed strength (MPa) for a life of `cycles` from a polished
    /// specimen's S–N line (Basquin, through 0.9 × tensile strength at 10³
    /// cycles and the fatigue strength at its cycles; Shigley §6-7), flat
    /// past the endurance limit where there is one. None without fatigue data.
    pub fn sn_strength(&self, cycles: f64) -> Option<f64> {
        let sf = self.fatigue_strength?;
        let n = cycles.max(1e3);
        let s3 = 0.9 * self.uts;
        if self.endurance_limit && n >= self.fatigue_cycles {
            return Some(sf);
        }
        let b = (sf / s3).log10() / (self.fatigue_cycles / 1e3).log10();
        Some(s3 * (n / 1e3).powf(b))
    }

    /// Cycles to failure at a fully reversed stress (MPa): infinite below an
    /// endurance limit, the S–N line otherwise.
    pub fn sn_life(&self, amplitude: f64) -> Option<f64> {
        let sf = self.fatigue_strength?;
        if amplitude <= 0.0 {
            return Some(f64::INFINITY);
        }
        if self.endurance_limit && amplitude <= sf {
            return Some(f64::INFINITY);
        }
        let s3 = 0.9 * self.uts;
        if amplitude >= s3 {
            return Some(1e3 * (s3 / amplitude).max(0.0));
        }
        let b = (sf / s3).log10() / (self.fatigue_cycles / 1e3).log10();
        Some(1e3 * (amplitude / s3).powf(1.0 / b))
    }
}
