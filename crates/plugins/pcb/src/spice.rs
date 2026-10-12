//! Circuit simulation with ngspice (BSD), run as a separate program in batch
//! mode: the schematic becomes a SPICE netlist, every part gets a model, the
//! analysis runs, and the waveforms come back with measurements, the
//! datasheet checks the models carry, and the person's expectations
//! ("min(V(3V3), burst) > 3.0") evaluated.
//!
//! Models, and how far to trust them (each result lists them):
//! - resistors, capacitors (with ESR; MLCC DC-bias derating optional),
//!   inductors (with DCR), fuses: ideal values from the part's value.
//! - diodes, LEDs (fitted to a forward voltage by colour), Zeners,
//!   transistors: generic models, **approximate**.
//! - push buttons: a switch the request presses and releases.
//! - LDOs (ME6211, AMS1117, XC6206, AP2112, or any "x.xV" LDO): a behavioural
//!   model from the datasheet's static numbers (output, dropout, current
//!   limit with foldback, enable threshold, quiescent current); the dynamic
//!   response (no maker publishes a model) is an estimate, **approximate**.
//! - the ESP32 module: a supply load that runs when EN is above 0.75·VDD,
//!   80 mA while running and Wi-Fi TX bursts of 500 mA (Espressif: 379 mA
//!   peak at 802.11b 19.5 dBm, supply rated ≥ 0.5 A), **approximate**.
//! - a USB-C receptacle is plugged into a 5 V source through a cable
//!   (0.2 Ω), advertising default USB power on CC1 (Rp 56 kΩ to 5 V).
//! - anything else is left out and listed.

use crate::schematic::is_ground;
use crate::schematic_view::{SchematicView, SymbolView};
use de_core::bevy_ecs::{self, component::Component};
use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

// ---- Request --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Analysis {
    /// Transient: `stop` seconds, printing every `step` (default stop/2000).
    Tran { stop: f64, step: Option<f64> },
    /// DC sweep of the source on `net` from `from` to `to` volts.
    Dc { net: String, from: f64, to: f64, step: f64 },
    /// Small-signal: a 1 V AC stimulus on the source on `net`, `from`…`to` Hz.
    Ac { net: String, from: f64, to: f64, points: Option<u32> },
    /// Operating point.
    Op,
}

impl Default for Analysis {
    fn default() -> Self {
        Analysis::Tran { stop: 0.03, step: None }
    }
}

/// A supply on a net (against ground), replacing what is assumed there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    pub net: String,
    pub volts: f64,
    /// Series resistance (Ω), default 0.05.
    pub r: Option<f64>,
    /// Ramp from 0 (s), default 0.5 ms.
    pub rise: Option<f64>,
    /// When the ramp starts (s), default 0.
    pub delay: Option<f64>,
}

/// A push button pressed at `at` for `hold` seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Press {
    pub part: String,
    pub at: f64,
    pub hold: f64,
}

/// A load model's burst: `at` seconds after the part starts running, `length` long, `current` amps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Burst {
    pub part: String,
    pub at: f64,
    pub length: f64,
    pub current: f64,
    /// Running current outside bursts (A).
    pub base: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SimRequest {
    #[serde(default)]
    pub analysis: Analysis,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub presses: Vec<Press>,
    #[serde(default)]
    pub bursts: Vec<Burst>,
    /// Expectations, e.g. "min(V(3V3), burst) > 3.0", "cross(V(EN), 2.475) < 20ms".
    #[serde(default)]
    pub expect: Vec<String>,
    /// Share of nominal capacitance an MLCC keeps under DC bias (1 = none lost; 0.6 is typical of X5R at half its rating).
    pub mlcc_derating: Option<f64>,
}

// ---- Netlist ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct PartModel {
    pub reference: String,
    pub model: String,
    pub approximate: bool,
    pub source: String,
}

/// A check a model carries from its datasheet.
#[derive(Clone, Debug)]
enum Rule {
    /// The part's VDD stays within [lo, hi] while it runs.
    SupplyRange { part: String, vdd: String, lo: f64, hi: f64, why: String },
    /// EN rises ≥ `min` seconds after VDD reaches `stable` volts.
    EnableDelay { part: String, vdd: String, stable: f64, min: f64, why: String },
    /// A regulator's output current stays ≤ imax, its dissipation reported against pd.
    Regulator { part: String, vin: String, vout: String, imax: f64, pd: f64, why: String },
}

#[derive(Clone, Debug)]
pub struct Netlist {
    pub text: String,
    /// Net name → SPICE node.
    pub nodes: BTreeMap<String, String>,
    /// Signal name → SPICE vector, saved and returned.
    pub saves: Vec<(String, String)>,
    /// Supply currents are reported positive: vectors to negate.
    negate: Vec<String>,
    pub parts: Vec<PartModel>,
    pub skipped: Vec<String>,
    pub assumptions: Vec<String>,
    rules: Vec<Rule>,
    /// Load parts with run/burst flags (reference → subcircuit instance).
    loads: Vec<String>,
}

fn node_name(net: &str) -> String {
    if is_ground(net) {
        return "0".into();
    }
    let s: String = net.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    format!("n_{s}")
}

/// "4.7k", "10uF", "100 nF", "4k7", "2R2", "1µF", "5.1 kΩ", "10k 1%" → SI value.
pub fn parse_value(s: &str) -> Option<f64> {
    let t: String = s.trim().replace(['Ω', ' '], "").replace("µ", "u").replace("ohm", "").replace("Ohm", "");
    let t = t.split(['%', '/', ',']).next().unwrap_or("");
    let mult = |c: char| match c {
        'p' => Some(1e-12),
        'n' => Some(1e-9),
        'u' => Some(1e-6),
        'm' => Some(1e-3),
        'R' | 'r' => Some(1.0),
        'k' | 'K' => Some(1e3),
        'M' => Some(1e6),
        'G' => Some(1e9),
        _ => None,
    };
    let num_end = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let head: f64 = t[..num_end].parse().ok()?;
    let rest = &t[num_end..];
    let mut chars = rest.chars();
    match chars.next() {
        None => Some(head),
        Some(c) => match mult(c) {
            Some(m) => {
                // RKM: "4k7" → 4.7k
                let tail: String = chars.take_while(|c| c.is_ascii_digit()).collect();
                let frac = if tail.is_empty() { 0.0 } else { format!("0.{tail}").parse::<f64>().unwrap_or(0.0) };
                Some((head + frac) * m)
            }
            None => Some(head),
        },
    }
}

fn volts_in(s: &str) -> Option<f64> {
    // "3.3V", "ME6211C33" (33 → 3.3), "AMS1117-3.3", "5V1"
    let u = s.to_ascii_uppercase();
    if let Some(i) = u.find('V') {
        let before: String = u[..i].chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>().chars().rev().collect();
        let after: String = u[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = before.parse::<f64>() {
            return Some(if after.is_empty() { v } else { v + format!("0.{after}").parse::<f64>().unwrap_or(0.0) });
        }
    }
    if let Some(i) = u.find("-") {
        if let Ok(v) = u[i + 1..].trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<f64>() {
            return Some(v);
        }
    }
    None
}

struct Ldo {
    name: &'static str,
    vset: f64,
    vdo0: f64,
    rdo: f64,
    ilim: f64,
    ishort: f64,
    ro: f64,
    iq: f64,
    ven: f64,
    tss: f64,
    imax: f64,
    pd: f64,
    source: &'static str,
}

fn ldo_for(value: &str, mpn: &str) -> Ldo {
    let key = format!("{value} {mpn}").to_ascii_uppercase();
    let v = volts_in(value).or_else(|| {
        // ME6211C33 → 3.3, XC6206P332 → 3.3
        let digits: String = key.chars().skip_while(|c| !c.is_ascii_digit()).collect();
        let d: String = digits.chars().skip(4).take(2).collect();
        d.parse::<f64>().ok().map(|x| x / 10.0)
    });
    if key.contains("ME6211") {
        let vset = key.split("ME6211C").nth(1).and_then(|r| r.get(..2)).and_then(|d| d.parse::<f64>().ok()).map(|d| d / 10.0).unwrap_or(3.3);
        return Ldo { name: "ME6211", vset, vdo0: 0.0, rdo: 1.3, ilim: 0.7, ishort: 0.15, ro: 0.09, iq: 60e-6, ven: 0.75, tss: 60e-6, imax: 0.5, pd: 0.3, source: "Microne ME6211 datasheet V14 (C33): 500 mA, dropout 120 mV at 100 mA and 260 mV at 200 mA, load regulation 9 mV over 1–100 mA, 60 µA supply current, CE high ≥ 1.0 V and low ≤ 0.5 V, short-circuit current 150 mA, 300 mW in SOT-23; start-up time and loop response estimated" };
    }
    if key.contains("AMS1117") || key.contains("LM1117") {
        return Ldo { name: "AMS1117", vset: v.unwrap_or(3.3), vdo0: 0.9, rdo: 0.25, ilim: 1.1, ishort: 0.6, ro: 0.05, iq: 5e-3, ven: -1.0, tss: 100e-6, imax: 1.0, pd: 0.8, source: "AMS1117 datasheet: 1 A, dropout 1.1 V at 800 mA, 5 mA quiescent current; SOT-223 dissipation and loop response estimated" };
    }
    if key.contains("XC6206") {
        return Ldo { name: "XC6206", vset: v.unwrap_or(3.3), vdo0: 0.0, rdo: 1.25, ilim: 0.25, ishort: 0.05, ro: 0.2, iq: 1e-6, ven: -1.0, tss: 100e-6, imax: 0.2, pd: 0.25, source: "Torex XC6206 datasheet: 200 mA, dropout 250 mV at 200 mA (typ.), 1 µA supply current; loop response estimated" };
    }
    if key.contains("AP2112") {
        return Ldo { name: "AP2112", vset: v.unwrap_or(3.3), vdo0: 0.0, rdo: 0.42, ilim: 0.8, ishort: 0.1, ro: 0.05, iq: 55e-6, ven: 1.0, tss: 100e-6, imax: 0.6, pd: 0.4, source: "Diodes AP2112 datasheet: 600 mA, dropout 250 mV at 600 mA, 55 µA, EN high ≥ 1.5 V; loop response estimated" };
    }
    Ldo { name: "LDO", vset: v.unwrap_or(3.3), vdo0: 0.0, rdo: 1.0, ilim: 0.6, ishort: 0.15, ro: 0.1, iq: 50e-6, ven: 0.9, tss: 100e-6, imax: 0.5, pd: 0.3, source: "a generic LDO: no datasheet model for this part; every figure assumed" }
}

const LIBRARY: &str = r#"
* --- Forma behavioural models (approximate; see each part's source) ---
.subckt forma_ldo vin gnd en vout params: vset=3.3 vdo0=0 rdo=1 ilim=0.6 ishort=0.15 ro=0.1 iq=50u ven=0.9 tss=100u
Ben ena gnd V = 1/(1+exp(-(V(en,gnd)-ven)/0.02))
Rss ena ss 1k
Css ss gnd {tss/2.2/1000}
Bint int gnd V = max(min(vset*V(ss,gnd), V(vin,gnd) - vdo0 - rdo*max(V(mid,vout)/ro,0)) - 20*max(V(mid,vout)/ro - (ishort + (ilim-ishort)*min(max(V(vout,gnd),0)/vset,1)), 0), 0)
Lo int mid 0.6u
Rd int mid 0.15
Ro mid s1 {ro}
Vs s1 vout 0
Fin vin gnd Vs 1
Iq vin gnd {iq}
.ends
.subckt forma_load vdd gnd en params: ibase=0.08 iburst=0.5 tb=5m tl=2m ireset=2m
Bon on gnd V = (1/(1+exp(-(V(en,gnd)-0.75*V(vdd,gnd))/0.02))) * (1/(1+exp(-(V(vdd,gnd)-2.3)/0.02)))
Bt gnd ton I = V(on,gnd) > 0.5 ? 1 : -V(ton,gnd)*1000
Ct ton gnd 1
Rt ton gnd 1e9
Bburst burst gnd V = 0.25*(1+tanh((V(ton,gnd)-tb)/2e-7))*(1+tanh((tb+tl-V(ton,gnd))/2e-7))
Bload vi gnd I = (V(on,gnd)*(ibase + (iburst-ibase)*V(burst,gnd)) + (1-V(on,gnd))*ireset) * min(1, max(V(vdd,gnd),0)/2.0)
Vsense vdd vi 0
.ends
.model forma_d D(Is=2.5n N=1.75 Rs=0.6 Cjo=4p Bv=100 Ibv=100u)
.model forma_schottky D(Is=5u N=1.05 Rs=0.05 Cjo=100p Bv=40 Ibv=1m)
.model forma_npn NPN(Is=1e-14 Bf=200 Vaf=100 Cje=4p Cjc=3p Tf=0.4n)
.model forma_pnp PNP(Is=1e-14 Bf=150 Vaf=100 Cje=5p Cjc=4p Tf=0.5n)
.model forma_nmos NMOS(Level=1 Vto=1.8 Kp=0.3 Lambda=0.02 Rd=1 Rs=1)
.model forma_pmos PMOS(Level=1 Vto=-1.8 Kp=0.2 Lambda=0.02 Rd=1 Rs=1)
.model forma_switch SW(Ron=0.05 Roff=1e9 Vt=0.5 Vh=0.1)
"#;

fn pin_net<'a>(s: &'a SymbolView, names: &[&str]) -> Option<&'a Option<String>> {
    s.pins.iter().find(|p| names.iter().any(|n| p.name.eq_ignore_ascii_case(n))).map(|p| &p.net)
}

fn led_vf(value: &str) -> (f64, &'static str) {
    let v = value.to_ascii_lowercase();
    if v.contains("blue") || v.contains("white") || v.contains("green") && !v.contains("yellow") {
        (3.0, "InGaN (blue, white, true green): 3.0 V at 20 mA")
    } else if v.contains("ir") || v.contains("infrared") {
        (1.3, "infrared: 1.3 V at 20 mA")
    } else {
        (2.0, "red, orange, yellow, yellow-green: 2.0 V at 20 mA")
    }
}

/// Builds the netlist (no analysis lines yet).
pub fn netlist(v: &SchematicView, req: &SimRequest) -> Netlist {
    let mut nl = Netlist { text: String::new(), nodes: BTreeMap::new(), saves: vec![], negate: vec![], parts: vec![], skipped: vec![], assumptions: vec![], rules: vec![], loads: vec![] };
    for n in &v.nets {
        nl.nodes.insert(n.name.clone(), node_name(&n.name));
    }
    if v.nets.iter().filter(|n| is_ground(&n.name)).count() > 1 {
        nl.assumptions.push("every ground-named net is the same node".into());
    }
    let mut body = String::new();
    let derate = req.mlcc_derating.unwrap_or(1.0).clamp(0.05, 1.0);
    // One entry per part (multi-unit parts merge their pins).
    let mut parts: BTreeMap<String, SymbolView> = BTreeMap::new();
    for s in &v.symbols {
        if s.virtual_part || s.dnp {
            continue;
        }
        parts.entry(s.reference.clone()).and_modify(|e| e.pins.extend(s.pins.iter().cloned())).or_insert_with(|| s.clone());
    }
    let mut nc = 0;
    let mut node = |net: &Option<String>| -> String {
        match net {
            Some(n) => node_name(n),
            None => {
                nc += 1;
                format!("nc_{nc}")
            }
        }
    };
    let mut sourced: Vec<String> = req.sources.iter().map(|s| s.net.clone()).collect();
    let add_source = |body: &mut String, nl: &mut Netlist, name: &str, net: &str, volts: f64, r: f64, rise: f64, delay: f64| {
        let n = node_name(net);
        let ac = match &req.analysis {
            Analysis::Ac { net: an, .. } if an.eq_ignore_ascii_case(net) => " AC 1",
            _ => "",
        };
        let ramp = if delay > 0.0 { format!("0 0 {delay} 0") } else { "0 0".to_string() };
        let _ = writeln!(body, "V_{name} {n}_src 0 DC {volts} PWL({ramp} {} {volts}){ac}", delay + rise.max(1e-9));
        let _ = writeln!(body, "R_{name}_out {n}_src {n} {r}");
        nl.saves.push((format!("I({name})"), format!("i(v_{})", name.to_ascii_lowercase())));
        nl.negate.push(format!("I({name})"));
    };
    for s in &req.sources {
        let name = format!("src_{}", node_name(&s.net).trim_start_matches("n_"));
        add_source(&mut body, &mut nl, &name, &s.net, s.volts, s.r.unwrap_or(0.05), s.rise.unwrap_or(0.5e-3), s.delay.unwrap_or(0.0));
        nl.assumptions.push(format!("{} V on {} through {} Ω, rising over {} ms", s.volts, s.net, s.r.unwrap_or(0.05), s.rise.unwrap_or(0.5e-3) * 1e3));
    }
    let mut models: Vec<PartModel> = Vec::new();
    for (r, s) in &parts {
        let sym = s.symbol.rsplit(':').next().unwrap_or(&s.symbol).to_string();
        let lib = s.symbol.split_once(':').map(|(l, _)| l).unwrap_or("");
        let lower = sym.to_ascii_lowercase();
        let pins: Vec<&crate::schematic_view::PinView> = s.pins.iter().filter(|p| p.etype != crate::symbols::PinType::NoConnect).collect();
        let two = |nodef: &mut dyn FnMut(&Option<String>) -> String| -> Option<(String, String)> {
            let mut p: Vec<&&crate::schematic_view::PinView> = pins.iter().collect();
            p.sort_by(|a, b| crate::natural(&a.number, &b.number));
            if p.len() < 2 {
                return None;
            }
            Some((nodef(&p[0].net), nodef(&p[1].net)))
        };
        let value = parse_value(&s.value);
        let mut model = |m: &str, approx: bool, src: &str| models.push(PartModel { reference: r.clone(), model: m.into(), approximate: approx, source: src.into() });
        let generic = lib.is_empty() || lib == "Device";
        if generic && (sym == "R" || lower.starts_with("r_")) || lib == "Device" && sym.starts_with('R') && !sym.starts_with("RF") {
            let (Some((a, b)), Some(val)) = (two(&mut node), value) else {
                nl.skipped.push(format!("{r}: no resistance in “{}”", s.value));
                continue;
            };
            let _ = writeln!(body, "R_{r} {a} {b} {val}");
            model(&format!("resistor {}", s.value), false, "value");
        } else if generic && (sym == "C" || sym == "C_Polarized" || sym.starts_with("C_") || lib == "Device" && sym.starts_with("C")) {
            let (Some((a, b)), Some(val)) = (two(&mut node), value) else {
                nl.skipped.push(format!("{r}: no capacitance in “{}”", s.value));
                continue;
            };
            let polar = sym.contains("Polar") || sym == "CP";
            let (esr, keep) = if polar { (0.3, 1.0) } else { (0.005, derate) };
            let _ = writeln!(body, "C_{r} {a} {r}_esr {}\nR_{r}_esr {r}_esr {b} {esr}", val * keep);
            model(&format!("capacitor {}{} with {esr} Ω ESR", s.value, if keep < 1.0 { format!(" × {keep} for DC bias") } else { String::new() }), polar, "value; ESR typical");
        } else if generic && (sym == "L" || lib == "Device" && sym.starts_with('L') && !sym.starts_with("LED")) {
            let (Some((a, b)), Some(val)) = (two(&mut node), value) else {
                nl.skipped.push(format!("{r}: no inductance in “{}”", s.value));
                continue;
            };
            let _ = writeln!(body, "L_{r} {a} {r}_dcr {val}\nR_{r}_dcr {r}_dcr {b} 0.05");
            model(&format!("inductor {} with 50 mΩ DCR", s.value), true, "value; DCR assumed");
        } else if sym == "Fuse" || lib == "Device" && sym.starts_with("Fuse") || sym.starts_with("Polyfuse") {
            let Some((a, b)) = two(&mut node) else { continue };
            let _ = writeln!(body, "R_{r} {a} {b} 0.05");
            model("fuse as 50 mΩ", true, "assumed");
        } else if sym == "LED" || sym.starts_with("LED") && (generic || lib.starts_with("LED")) {
            let (Some(k), Some(a)) = (pin_net(s, &["K", "A"]).filter(|_| s.pins.iter().any(|p| p.name.eq_ignore_ascii_case("K"))).or_else(|| s.pins.iter().find(|p| p.number == "1").map(|p| &p.net)), s.pins.iter().find(|p| p.name.eq_ignore_ascii_case("A") || p.number == "2").map(|p| &p.net)) else { continue };
            let (vf, what) = led_vf(&s.value);
            // Shockley fit through Vf at 20 mA with n = 2 and 5 Ω.
            let is = 0.02 / ((vf - 0.1) / (2.0 * 0.025852)).exp();
            let (ka, kk) = (node(a), node(k));
            let _ = writeln!(body, ".model led_{r} D(Is={is:.4e} N=2 Rs=5 Cjo=20p)\nV_{r} {ka} {r}_a 0\nD_{r} {r}_a {kk} led_{r}");
            nl.saves.push((format!("I({r})"), format!("i(v_{})", r.to_ascii_lowercase())));
            model(&format!("LED, {what}"), true, "generic fit");
        } else if sym.starts_with('D') && (generic || lib.starts_with("Diode")) {
            let Some(k) = s.pins.iter().find(|p| p.name.eq_ignore_ascii_case("K") || p.number == "1") else { continue };
            let Some(a) = s.pins.iter().find(|p| p.name.eq_ignore_ascii_case("A") || p.number == "2") else { continue };
            let (ka, kk) = (node(&a.net), node(&k.net));
            if sym.contains("Zener") || lower.contains("zener") {
                let bv = volts_in(&s.value).unwrap_or(5.1);
                let _ = writeln!(body, ".model z_{r} D(Is=1n N=1.5 Rs=1 Bv={bv} Ibv=5m)\nD_{r} {ka} {kk} z_{r}");
                model(&format!("Zener {bv} V"), true, "generic");
            } else if sym.contains("Schottky") || lower.contains("schottky") {
                let _ = writeln!(body, "D_{r} {ka} {kk} forma_schottky");
                model("Schottky diode", true, "generic (SS34-like)");
            } else {
                let _ = writeln!(body, "D_{r} {ka} {kk} forma_d");
                model("diode", true, "generic (1N4148-like)");
            }
        } else if sym.starts_with("Q_") {
            let get = |names: &[&str], num: &str| s.pins.iter().find(|p| names.iter().any(|n| p.name.eq_ignore_ascii_case(n)) || p.number == num).map(|p| p.net.clone());
            if sym.contains("MOS") {
                let (Some(g), Some(sr), Some(d)) = (get(&["G"], "1"), get(&["S"], "2"), get(&["D"], "3")) else { continue };
                let (dn, gn, sn) = (node(&d), node(&g), node(&sr));
                let m = if sym.contains("PMOS") { "forma_pmos" } else { "forma_nmos" };
                let _ = writeln!(body, "M_{r} {dn} {gn} {sn} {sn} {m}");
                model(&format!("MOSFET ({m})"), true, "generic level-1");
            } else {
                let (Some(b), Some(e), Some(c)) = (get(&["B"], "1"), get(&["E"], "2"), get(&["C"], "3")) else { continue };
                let (cn, bn, en) = (node(&c), node(&b), node(&e));
                let m = if sym.contains("PNP") { "forma_pnp" } else { "forma_npn" };
                let _ = writeln!(body, "Q_{r} {cn} {bn} {en} {m}");
                model(&format!("BJT ({m})"), true, "generic");
            }
        } else if sym == "SW_Push" || lib == "Switch" && lower.starts_with("sw_push") {
            let Some((a, b)) = two(&mut node) else { continue };
            let mut pwl = String::from("0 0");
            for p in req.presses.iter().filter(|p| p.part.eq_ignore_ascii_case(r)) {
                let _ = write!(pwl, " {} 0 {} 1 {} 1 {} 0", p.at, p.at + 1e-6, p.at + p.hold, p.at + p.hold + 1e-6);
            }
            let _ = writeln!(body, "V_{r}_ctl {r}_ctl 0 PWL({pwl})\nS_{r} {a} {b} {r}_ctl 0 forma_switch");
            let pressed = req.presses.iter().any(|p| p.part.eq_ignore_ascii_case(r));
            model(if pressed { "push button, pressed as asked" } else { "push button, open" }, false, "switch 50 mΩ / 1 GΩ");
        } else if sym.starts_with("USB_C") || lower.contains("usb_c_receptacle") {
            let (Some(vbus), Some(cc1)) = (pin_net(s, &["VBUS"]), pin_net(s, &["CC1", "CC"])) else { continue };
            let Some(net) = vbus.clone() else { continue };
            if sourced.iter().any(|x| x.eq_ignore_ascii_case(&net)) {
                model("USB-C receptacle (the request supplies its VBUS)", false, "");
                continue;
            }
            sourced.push(net.clone());
            let name = r.clone();
            add_source(&mut body, &mut nl, &name, &net, 5.0, 0.2, 0.5e-3, 0.0);
            let cc = node(cc1);
            let _ = writeln!(body, "R_{r}_rp {}_src {cc} 56k", node_name(&net));
            nl.assumptions.push(format!("{r} is plugged into a 5 V USB source: 0.2 Ω of cable, VBUS rising over 0.5 ms, Rp 56 kΩ on CC1 (default USB power)"));
            model("USB-C plugged into a 5 V source (0.2 Ω cable, Rp 56 kΩ on CC1)", true, "USB Type-C spec Rp for default power; cable resistance assumed");
        } else if s.category == "Power IC" || lib.starts_with("Regulator_Linear") || sym.starts_with("LDO") {
            let (Some(vin), Some(vout)) = (pin_net(s, &["VIN", "VI", "IN"]), pin_net(s, &["VOUT", "VO", "OUT"])) else {
                nl.skipped.push(format!("{r}: a power part without VIN/VOUT pins"));
                continue;
            };
            let gnd = pin_net(s, &["GND", "VSS", "ADJ", "GND/ADJ"]).cloned().unwrap_or(Some("GND".into()));
            let en = pin_net(s, &["EN", "CE", "SHDN", "ON", "~SHDN"]).cloned();
            let l = ldo_for(&s.value, s.mpn.as_deref().unwrap_or(""));
            let (ni, no, ng) = (node(vin), node(vout), node(&gnd));
            let ne = match &en {
                Some(e) => node(e),
                None => ni.clone(),
            };
            let _ = writeln!(body, "X_{r} {ni} {ng} {ne} {no} forma_ldo vset={} vdo0={} rdo={} ilim={} ishort={} ro={} iq={} ven={} tss={}", l.vset, l.vdo0, l.rdo, l.ilim, l.ishort, l.ro, l.iq, if l.ven < 0.0 { 0.0 } else { l.ven }, l.tss);
            nl.saves.push((format!("I({r})"), format!("i(v.x_{}.vs)", r.to_ascii_lowercase())));
            if let (Some(i), Some(o)) = (vin.clone(), vout.clone()) {
                nl.rules.push(Rule::Regulator { part: r.clone(), vin: i, vout: o, imax: l.imax, pd: l.pd, why: l.source.into() });
            }
            model(&format!("{} {} V LDO, behavioural", l.name, l.vset), true, l.source);
        } else if sym.starts_with("ESP32") || lib == "RF_Module" && lower.starts_with("esp32") {
            let (Some(vdd), Some(en)) = (pin_net(s, &["3V3", "VDD", "VCC"]), pin_net(s, &["EN", "CHIP_PU"])) else { continue };
            let gnd = pin_net(s, &["GND"]).cloned().unwrap_or(Some("GND".into()));
            let b = req.bursts.iter().find(|b| b.part.eq_ignore_ascii_case(r));
            let (base, cur, at, len) = b.map(|b| (b.base.unwrap_or(0.08), b.current, b.at, b.length)).unwrap_or((0.08, 0.5, 5e-3, 2e-3));
            let (nv, ng, ne) = (node(vdd), node(&gnd), node(en));
            let _ = writeln!(body, "X_{r} {nv} {ng} {ne} forma_load ibase={base} iburst={cur} tb={at} tl={len}");
            nl.saves.push((format!("I({r})"), format!("i(v.x_{}.vsense)", r.to_ascii_lowercase())));
            nl.loads.push(r.clone());
            if let Some(vd) = vdd.clone() {
                nl.rules.push(Rule::SupplyRange { part: r.clone(), vdd: vd.clone(), lo: 3.0, hi: 3.6, why: "Espressif ESP32-WROOM-32E datasheet v2.1, table 14: VDD33 3.0–3.6 V".into() });
                nl.rules.push(Rule::EnableDelay { part: r.clone(), vdd: vd, stable: 3.0, min: 50e-6, why: "Espressif, table 9: tSTBL ≥ 50 µs from a stable 3.3 V rail to CHIP_PU (EN) high; EN high is 0.75 × VDD (table 15)".into() });
            }
            model(&format!("ESP32 module as a supply load: runs when EN > 0.75·VDD; {} mA running, a {} mA Wi-Fi burst {} ms after start for {} ms", base * 1e3, cur * 1e3, at * 1e3, len * 1e3), true, "Espressif ESP32-WROOM-32E datasheet v2.1, tables 9, 14–16 (TX 802.11b 379 mA peak; supply ≥ 0.5 A); profile assumed");
        } else {
            nl.skipped.push(format!("{r} ({}): no model; its pins are left open", s.symbol));
        }
    }
    nl.parts = models;
    // Every named net is probed.
    for (net, n) in &nl.nodes {
        if n != "0" && body.split_whitespace().any(|w| w == n) {
            nl.saves.push((format!("V({net})"), format!("v({n})")));
        }
    }
    for l in &nl.loads {
        let x = format!("x_{}", l.to_ascii_lowercase());
        nl.saves.push((format!("{l}.on"), format!("v({x}.on)")));
        nl.saves.push((format!("{l}.burst"), format!("v({x}.burst)")));
    }
    nl.text = format!("* {} — made by Forma\n.options rshunt=1e12 reltol=1e-4\n{LIBRARY}\n{body}", v.name);
    nl
}

// ---- Running ngspice ---------------------------------------------------------------------

/// The ngspice program: `FORMA_NGSPICE`, else on the PATH or where Homebrew and Linux put it.
pub fn ngspice() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("FORMA_NGSPICE") {
        return Some(PathBuf::from(p));
    }
    let mut dirs: Vec<PathBuf> = std::env::var("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(if cfg!(windows) { "ngspice.exe" } else { "ngspice" })).find(|p| p.is_file())
}

#[derive(Clone, Debug, Default)]
struct Raw {
    x_name: String,
    x: Vec<f64>,
    complex: bool,
    vecs: Vec<(String, Vec<f64>, Vec<f64>)>,
}

fn parse_raw(text: &str) -> Result<Raw, String> {
    let mut raw = Raw::default();
    let mut lines = text.lines();
    let mut nvars = 0;
    let mut npts = 0;
    let mut names = Vec::new();
    while let Some(l) = lines.next() {
        if let Some(f) = l.strip_prefix("Flags:") {
            raw.complex = f.contains("complex");
        } else if let Some(n) = l.strip_prefix("No. Variables:") {
            nvars = n.trim().parse().map_err(|_| "bad raw header")?;
        } else if let Some(n) = l.strip_prefix("No. Points:") {
            npts = n.trim().parse().map_err(|_| "bad raw header")?;
        } else if l.starts_with("Variables:") {
            for _ in 0..nvars {
                let v = lines.next().ok_or("raw file ends in its variables")?;
                names.push(v.split_whitespace().nth(1).unwrap_or("").to_string());
            }
        } else if l.starts_with("Values:") {
            break;
        }
    }
    let mut cols: Vec<(Vec<f64>, Vec<f64>)> = vec![(Vec::with_capacity(npts), Vec::new()); nvars];
    let tokens: Vec<&str> = lines.flat_map(|l| l.split_whitespace()).collect();
    let mut k = 0;
    for _ in 0..npts {
        k += 1; // point index
        for c in cols.iter_mut() {
            let t = tokens.get(k).ok_or("raw file ends early")?;
            k += 1;
            if raw.complex {
                let (re, im) = t.split_once(',').unwrap_or((t, "0"));
                c.0.push(re.parse().map_err(|_| "bad number in raw file")?);
                c.1.push(im.parse().map_err(|_| "bad number in raw file")?);
            } else {
                c.0.push(t.parse().map_err(|_| "bad number in raw file")?);
            }
        }
    }
    if names.is_empty() {
        return Err("ngspice wrote no vectors".into());
    }
    raw.x_name = names[0].clone();
    raw.x = cols[0].0.clone();
    for (n, c) in names.into_iter().zip(cols).skip(1) {
        raw.vecs.push((n.to_ascii_lowercase(), c.0, c.1));
    }
    Ok(raw)
}

fn run(nl: &Netlist, req: &SimRequest) -> Result<(Raw, String), String> {
    let exe = ngspice().ok_or("ngspice is not installed: `brew install ngspice` (macOS), `apt install ngspice` (Linux), or ngspice.sourceforge.io; FORMA_NGSPICE points at another copy")?;
    static N: Mutex<u64> = Mutex::new(0);
    let k = {
        let mut n = N.lock().unwrap_or_else(|e| e.into_inner());
        *n += 1;
        *n
    };
    let dir = std::env::temp_dir().join(format!("forma-spice-{}-{k}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join("out.raw");
    let analysis = match &req.analysis {
        Analysis::Tran { stop, step } => {
            let step = step.unwrap_or(stop / 2000.0);
            format!(".tran {step} {stop} 0 {}", step.min(stop / 500.0))
        }
        Analysis::Dc { net, from, to, step } => format!(".dc v_{} {from} {to} {step}", source_name(nl, net)?),
        Analysis::Ac { net, from, to, points } => {
            source_name(nl, net)?;
            format!(".ac dec {} {from} {to}", points.unwrap_or(20))
        }
        Analysis::Op => ".op".into(),
    };
    let saves: Vec<&str> = nl.saves.iter().map(|(_, v)| v.as_str()).collect();
    let deck = format!("{}\n{analysis}\n.control\nset filetype=ascii\nset noaskquit\nrun\nwrite {} {}\nquit\n.endc\n.end\n", nl.text, out.display(), saves.join(" "));
    std::fs::write(dir.join("circuit.cir"), &deck).map_err(|e| e.to_string())?;
    let o = std::process::Command::new(&exe).arg("-b").arg(dir.join("circuit.cir")).output().map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    let log = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let raw = std::fs::read_to_string(&out).map_err(|_| format!("ngspice did not finish: {}", log.lines().filter(|l| l.to_ascii_lowercase().contains("error") || l.contains("singular") || l.contains("aborted")).take(6).collect::<Vec<_>>().join(" | ")));
    if raw.is_ok() || std::env::var("FORMA_SPICE_KEEP").is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok((parse_raw(&raw?)?, deck))
}

fn source_name(nl: &Netlist, net: &str) -> Result<String, String> {
    let n = node_name(net);
    let want = format!("{n}_src");
    nl.text.lines().find(|l| l.starts_with("V_") && l.split_whitespace().nth(1) == Some(want.as_str())).map(|l| l.split_whitespace().next().unwrap()[2..].to_ascii_lowercase()).ok_or_else(|| format!("no source on {net}: add one in `sources` (or plug in a USB-C receptacle there)"))
}

// ---- Results -------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct Signal {
    pub name: String,
    pub unit: &'static str,
    pub values: Vec<f64>,
    /// AC: phase in degrees (values are then magnitude in dB).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phase: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Window {
    pub name: String,
    pub from: f64,
    pub to: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub value: Option<f64>,
    pub unit: &'static str,
    pub limit: String,
    /// pass, warn or fail.
    pub status: &'static str,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SimResult {
    pub analysis: Analysis,
    pub x_name: String,
    pub x_unit: &'static str,
    pub x: Vec<f64>,
    pub signals: Vec<Signal>,
    pub windows: Vec<Window>,
    /// Datasheet checks the models carry.
    pub checks: Vec<Check>,
    /// The request's expectations.
    pub expectations: Vec<Check>,
    pub measurements: Vec<Check>,
    pub parts: Vec<PartModel>,
    pub skipped: Vec<String>,
    pub assumptions: Vec<String>,
    pub netlist: String,
    pub ok: bool,
}

struct Data<'a> {
    x: &'a [f64],
    sig: HashMap<String, &'a [f64]>,
    windows: &'a [Window],
}

impl Data<'_> {
    fn get(&self, name: &str) -> Result<&[f64], String> {
        let key = name.to_ascii_uppercase();
        self.sig.iter().find(|(k, _)| k.to_ascii_uppercase() == key).map(|(_, v)| *v).ok_or_else(|| format!("no signal {name} (probed: V(<net>), I(<part>))"))
    }
    fn range(&self, from: f64, to: f64) -> std::ops::Range<usize> {
        let a = self.x.partition_point(|t| *t < from);
        let b = self.x.partition_point(|t| *t <= to).max(a + 1).min(self.x.len());
        a.min(self.x.len().saturating_sub(1))..b
    }
    fn cross(&self, v: &[f64], level: f64, rising: bool, from: f64) -> Option<f64> {
        let start = self.x.partition_point(|t| *t < from);
        for i in start.max(1)..v.len() {
            let (a, b) = (v[i - 1], v[i]);
            if (rising && a < level && b >= level) || (!rising && a > level && b <= level) {
                let f = (level - a) / (b - a);
                return Some(self.x[i - 1] + f * (self.x[i] - self.x[i - 1]));
            }
        }
        None
    }
}

/// The largest average of `v` over any `tau`-long stretch within `r`.
fn windowed_max(x: &[f64], v: &[f64], r: std::ops::Range<usize>, tau: f64) -> f64 {
    let mut best = f64::NEG_INFINITY;
    let (mut j, mut acc) = (r.start, 0.0);
    for i in r.start..r.end {
        // Integral of v from x[i] forward until tau has passed.
        while j + 1 < r.end && x[j + 1] - x[i] <= tau {
            acc += (v[j] + v[j + 1]) / 2.0 * (x[j + 1] - x[j]);
            j += 1;
        }
        if x[j] > x[i] {
            best = best.max(acc / (x[j] - x[i]));
        }
        if j > i {
            acc -= (v[i] + v[i + 1]) / 2.0 * (x[i + 1] - x[i]);
        } else {
            j = i + 1;
            acc = 0.0;
        }
    }
    if best.is_finite() { best } else { v[r].iter().cloned().fold(0.0, f64::max) }
}

fn fmt_si(v: f64, unit: &str) -> String {
    let a = v.abs();
    let (m, p) = if a == 0.0 { (1.0, "") } else if a < 1e-6 { (1e9, "n") } else if a < 1e-3 { (1e6, "µ") } else if a < 1.0 { (1e3, "m") } else { (1.0, "") };
    format!("{:.4}", v * m).trim_end_matches('0').trim_end_matches('.').to_string() + " " + p + unit
}

// ---- Expectation language -------------------------------------------------------------------

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    d: &'a Data<'a>,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected “{}” at {}", c as char, self.i))
        }
    }
    fn ident(&mut self) -> String {
        self.ws();
        let st = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[st..self.i]).into()
    }
    fn number(&mut self) -> Result<f64, String> {
        self.ws();
        let st = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.' || (self.s[self.i] == b'e' && self.i + 1 < self.s.len() && (self.s[self.i + 1].is_ascii_digit() || self.s[self.i + 1] == b'-'))) {
            if self.s[self.i] == b'e' {
                self.i += 1;
            }
            self.i += 1;
        }
        let n: f64 = std::str::from_utf8(&self.s[st..self.i]).unwrap().parse().map_err(|_| format!("expected a number at {st}"))?;
        // Unit: an SI prefix then letters (ms, us, µs, mV, mA, %, kHz, dB).
        let rest = std::str::from_utf8(&self.s[self.i..]).unwrap();
        let unit: String = rest.chars().take_while(|c| c.is_alphabetic() || *c == '%' || *c == 'µ').collect();
        self.i += unit.len();
        let mult = match unit.as_str() {
            "%" => 0.01,
            u if u.starts_with('m') && u.len() > 1 => 1e-3,
            u if (u.starts_with('u') || u.starts_with('µ')) && u.len() > 1 => 1e-6,
            u if u.starts_with('n') && u.len() > 1 => 1e-9,
            u if u.starts_with('k') => 1e3,
            u if u.starts_with('M') => 1e6,
            _ => 1.0,
        };
        Ok(n * mult)
    }
    /// V(net), V(a,b) or I(part): the signal's samples.
    fn signal(&mut self) -> Result<Vec<f64>, String> {
        let k = self.ident().to_ascii_uppercase();
        self.eat(b'(')?;
        self.ws();
        let st = self.i;
        while self.i < self.s.len() && self.s[self.i] != b')' {
            self.i += 1;
        }
        let inner = String::from_utf8_lossy(&self.s[st..self.i]).trim().to_string();
        self.eat(b')')?;
        match (k.as_str(), inner.split_once(',')) {
            ("V", Some((a, b))) => {
                let (a, b) = (self.d.get(&format!("V({})", a.trim()))?, self.d.get(&format!("V({})", b.trim()))?);
                Ok(a.iter().zip(b).map(|(x, y)| x - y).collect())
            }
            ("V", None) | ("I", None) => Ok(self.d.get(&format!("{k}({inner})"))?.to_vec()),
            _ => Err(format!("{k}(…) is not a signal: use V(net) or I(part)")),
        }
    }
    /// After the signal: a named window or two times.
    fn window(&mut self) -> Result<(f64, f64), String> {
        if self.peek() != Some(b',') {
            return Ok((f64::NEG_INFINITY, f64::INFINITY));
        }
        self.eat(b',')?;
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            let name = self.ident();
            let w = self.d.windows.iter().find(|w| w.name.eq_ignore_ascii_case(&name)).ok_or_else(|| format!("no window “{name}” (this run has: {})", self.d.windows.iter().map(|w| w.name.clone()).collect::<Vec<_>>().join(", ")))?;
            return Ok((w.from, w.to));
        }
        let a = self.number()?;
        self.eat(b',')?;
        let b = self.number()?;
        Ok((a, b))
    }
    fn term(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let v = self.sum()?;
                self.eat(b')')?;
                Ok(v)
            }
            Some(b'-') => {
                self.i += 1;
                Ok(-self.term()?)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
            _ => {
                let f = self.ident().to_ascii_lowercase();
                self.eat(b'(')?;
                let v = self.signal()?;
                let d = self.d;
                let out = match f.as_str() {
                    "min" | "max" | "avg" | "pp" => {
                        let (a, b) = self.window()?;
                        let r = d.range(a, b);
                        let s = &v[r.clone()];
                        match f.as_str() {
                            "min" => s.iter().cloned().fold(f64::INFINITY, f64::min),
                            "max" => s.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                            "pp" => s.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - s.iter().cloned().fold(f64::INFINITY, f64::min),
                            _ => {
                                let xs = &d.x[r];
                                if xs.len() < 2 {
                                    s[0]
                                } else {
                                    let mut acc = 0.0;
                                    for i in 1..xs.len() {
                                        acc += (s[i] + s[i - 1]) / 2.0 * (xs[i] - xs[i - 1]);
                                    }
                                    acc / (xs[xs.len() - 1] - xs[0])
                                }
                            }
                        }
                    }
                    "final" => *v.last().ok_or("empty signal")?,
                    "at" => {
                        self.eat(b',')?;
                        let t = self.number()?;
                        let i = d.x.partition_point(|x| *x < t).min(v.len() - 1);
                        v[i]
                    }
                    "cross" | "fall" => {
                        self.eat(b',')?;
                        let level = self.number()?;
                        d.cross(&v, level, f == "cross", f64::NEG_INFINITY).ok_or_else(|| format!("{} never {} {level}", "the signal", if f == "cross" { "rises through" } else { "falls through" }))?
                    }
                    "settle" => {
                        self.eat(b',')?;
                        let pct = self.number()?;
                        let fin = *v.last().ok_or("empty signal")?;
                        let band = (fin.abs() * pct).max(1e-12);
                        let i = v.iter().rposition(|x| (x - fin).abs() > band).map(|i| i + 1).unwrap_or(0);
                        d.x[i.min(d.x.len() - 1)]
                    }
                    "rise" => {
                        let fin = *v.last().ok_or("empty signal")?;
                        let a = d.cross(&v, 0.1 * fin, true, f64::NEG_INFINITY).ok_or("never reaches 10 %")?;
                        let b = d.cross(&v, 0.9 * fin, true, a).ok_or("never reaches 90 %")?;
                        b - a
                    }
                    other => return Err(format!("unknown function {other}: min, max, avg, pp, final, at, cross, fall, settle, rise")),
                };
                self.eat(b')')?;
                Ok(out)
            }
        }
    }
    fn sum(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            match self.peek() {
                Some(b'+') => {
                    self.i += 1;
                    v += self.term()?;
                }
                Some(b'-') => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => return Ok(v),
            }
        }
    }
}

/// Evaluates "lhs op rhs": (lhs value, pass).
fn expect(d: &Data, text: &str) -> Result<(f64, bool), String> {
    let ops = [">=", "<=", ">", "<"];
    let (pos, op) = ops.iter().filter_map(|o| text.find(o).map(|p| (p, *o))).min_by_key(|(p, o)| (*p, std::cmp::Reverse(o.len()))).ok_or("an expectation compares: min(V(3V3), burst) > 3.0")?;
    let eval = |s: &str| -> Result<f64, String> {
        let mut p = Parser { s: s.as_bytes(), i: 0, d };
        let v = p.sum()?;
        p.ws();
        if p.i != s.len() {
            return Err(format!("cannot read “{}”", &s[p.i..]));
        }
        Ok(v)
    };
    let (a, b) = (eval(&text[..pos])?, eval(&text[pos + op.len()..])?);
    let pass = match op {
        ">=" => a >= b,
        "<=" => a <= b,
        ">" => a > b,
        _ => a < b,
    };
    Ok((a, pass))
}

/// Keeps a waveform's shape in at most `n` points (each bucket's first, min and max).
fn decimate(x: &[f64], sigs: &mut [Signal], n: usize) -> Vec<f64> {
    if x.len() <= n {
        return x.to_vec();
    }
    let bucket = x.len().div_ceil(n / 2);
    let mut keep: Vec<usize> = Vec::new();
    for b in (0..x.len()).step_by(bucket) {
        let e = (b + bucket).min(x.len());
        let mut idx = vec![b, e - 1];
        for s in sigs.iter() {
            let r = &s.values[b..e];
            if let Some((i, _)) = r.iter().enumerate().min_by(|a, c| a.1.total_cmp(c.1)) {
                idx.push(b + i);
            }
            if let Some((i, _)) = r.iter().enumerate().max_by(|a, c| a.1.total_cmp(c.1)) {
                idx.push(b + i);
            }
        }
        keep.extend(idx);
    }
    keep.sort_unstable();
    keep.dedup();
    for s in sigs.iter_mut() {
        s.values = keep.iter().map(|i| s.values[*i]).collect();
        if !s.phase.is_empty() {
            s.phase = keep.iter().map(|i| s.phase[*i]).collect();
        }
    }
    keep.iter().map(|i| x[*i]).collect()
}

/// Results of identical runs, by netlist and request.
static CACHE: OnceLock<Mutex<HashMap<u64, Arc<SimResult>>>> = OnceLock::new();

/// Simulates a schematic.
pub fn simulate(v: &SchematicView, req: &SimRequest) -> Result<Arc<SimResult>, String> {
    let nl = netlist(v, req);
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        nl.text.hash(&mut h);
        serde_json::to_string(req).unwrap_or_default().hash(&mut h);
        h.finish()
    };
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(r) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(r.clone());
    }
    let (raw, deck) = run(&nl, req)?;
    let r = Arc::new(evaluate(&nl, req, &raw, deck));
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    if c.len() > 64 {
        c.clear();
    }
    c.insert(key, r.clone());
    Ok(r)
}

fn evaluate(nl: &Netlist, req: &SimRequest, raw: &Raw, deck: String) -> SimResult {
    let ac = raw.complex;
    let mut sigs: Vec<Signal> = Vec::new();
    for (name, vec) in &nl.saves {
        let want = vec.to_ascii_lowercase();
        let alt = want.trim_start_matches("v(").trim_end_matches(')').to_string();
        let Some((_, re, im)) = raw.vecs.iter().find(|(n, _, _)| *n == want || *n == alt) else { continue };
        let unit = if name.starts_with("I(") { "A" } else { "V" };
        if ac {
            let mag: Vec<f64> = re.iter().zip(im).map(|(a, b)| 20.0 * (a.hypot(*b)).max(1e-30).log10()).collect();
            let ph: Vec<f64> = re.iter().zip(im).map(|(a, b)| b.atan2(*a).to_degrees()).collect();
            sigs.push(Signal { name: name.clone(), unit: "dB", values: mag, phase: ph });
        } else {
            let neg = nl.negate.contains(name);
            sigs.push(Signal { name: name.clone(), unit, values: re.iter().map(|x| if neg { -x } else { *x }).collect(), phase: vec![] });
        }
    }
    let x_unit = match &req.analysis {
        Analysis::Tran { .. } => "s",
        Analysis::Dc { .. } => "V",
        Analysis::Ac { .. } => "Hz",
        Analysis::Op => "",
    };
    // Windows from the load models: when each runs, and its first burst.
    let mut windows = vec![Window { name: "all".into(), from: raw.x.first().copied().unwrap_or(0.0), to: raw.x.last().copied().unwrap_or(0.0) }];
    let end = raw.x.last().copied().unwrap_or(0.0);
    let flag = |name: &str| sigs.iter().find(|s| s.name == name).map(|s| s.values.clone());
    let edges = |v: &[f64]| -> Option<(f64, f64)> {
        let a = v.iter().position(|x| *x > 0.5)?;
        let b = v[a..].iter().position(|x| *x < 0.5).map(|i| raw.x[a + i]).unwrap_or(end);
        Some((raw.x[a], b))
    };
    for l in &nl.loads {
        if !matches!(req.analysis, Analysis::Tran { .. }) {
            break;
        }
        if let Some((a, b)) = flag(&format!("{l}.on")).as_deref().and_then(edges) {
            windows.push(Window { name: if nl.loads.len() == 1 { "running".into() } else { format!("{l}_running") }, from: a, to: b });
        }
        if let Some((a, b)) = flag(&format!("{l}.burst")).as_deref().and_then(edges) {
            windows.push(Window { name: if nl.loads.len() == 1 { "burst".into() } else { format!("{l}_burst") }, from: a, to: b });
        }
    }
    sigs.retain(|s| !s.name.ends_with(".on") && !s.name.ends_with(".burst"));
    let data = Data { x: &raw.x, sig: sigs.iter().map(|s| (s.name.clone(), s.values.as_slice())).collect(), windows: &windows };
    let mut checks = Vec::new();
    let mut measurements = Vec::new();
    if matches!(req.analysis, Analysis::Tran { .. }) {
        for rule in &nl.rules {
            match rule {
                Rule::SupplyRange { part, vdd, lo, hi, why } => {
                    let Some(w) = windows.iter().find(|w| w.name.contains("running")) else {
                        checks.push(Check { name: format!("{part} starts"), value: None, unit: "", limit: "EN above 0.75·VDD".into(), status: "fail", detail: format!("{part} never comes out of reset in this run ({why})") });
                        continue;
                    };
                    // From 100 µs after it starts (the rail is checked by tSTBL before that).
                    if let Ok(v) = data.get(&format!("V({vdd})")) {
                        let r = data.range(w.from + 100e-6, w.to);
                        let mn = v[r.clone()].iter().cloned().fold(f64::INFINITY, f64::min);
                        let mx = v[r].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                        let ok = mn >= *lo && mx <= *hi;
                        checks.push(Check { name: format!("{part} supply {vdd} while running"), value: Some(mn), unit: "V", limit: format!("{lo}–{hi} V"), status: if ok { "pass" } else { "fail" }, detail: format!("{} … {} ({why})", fmt_si(mn, "V"), fmt_si(mx, "V")) });
                    }
                }
                Rule::EnableDelay { part, vdd, stable, min, why } => {
                    let (Ok(v), Some(w)) = (data.get(&format!("V({vdd})")), windows.iter().find(|w| w.name.contains("running"))) else { continue };
                    let Some(t) = data.cross(v, *stable, true, f64::NEG_INFINITY) else { continue };
                    let delay = w.from - t;
                    checks.push(Check { name: format!("{part} released from reset after {vdd} is stable"), value: Some(delay), unit: "s", limit: format!("≥ {}", fmt_si(*min, "s")), status: if delay >= *min { "pass" } else { "fail" }, detail: format!("{vdd} reaches {stable} V at {}, EN releases at {} ({why})", fmt_si(t, "s"), fmt_si(w.from, "s")) });
                }
                Rule::Regulator { part, vin, vout, imax, pd, why } => {
                    let (Ok(i), Ok(a), Ok(b)) = (data.get(&format!("I({part})")), data.get(&format!("V({vin})")), data.get(&format!("V({vout})"))) else { continue };
                    // After start-up (charging the output capacitors runs into the current limit by design).
                    let run = windows.iter().find(|w| w.name.contains("running")).map(|w| data.range(w.from + 100e-6, w.to)).unwrap_or(0..i.len());
                    let inrush = i[..run.start].iter().cloned().fold(0.0, f64::max);
                    // The rating is a continuous current: averaged over 50 µs, so the edge of a burst recharging the output is not counted.
                    let ipk = windowed_max(data.x, i, run.clone(), 50e-6);
                    checks.push(Check { name: format!("{part} output current"), value: Some(ipk), unit: "A", limit: format!("≤ {}", fmt_si(*imax, "A")), status: if ipk <= *imax { "pass" } else if ipk <= 1.2 * imax { "warn" } else { "fail" }, detail: format!("{} at most (50 µs average) while the load runs; {} charging the output at start-up ({why})", fmt_si(ipk, "A"), fmt_si(inrush, "A")) });
                    let p: Vec<f64> = i.iter().zip(a.iter().zip(b)).map(|(i, (a, b))| i.max(0.0) * (a - b).max(0.0)).collect();
                    let ppk = p[run.clone()].iter().cloned().fold(0.0, f64::max);
                    let status = if ppk <= *pd { "pass" } else { "warn" };
                    checks.push(Check { name: format!("{part} dissipation at the heaviest load"), value: Some(ppk), unit: "W", limit: format!("≤ {} continuous", fmt_si(*pd, "W")), status, detail: format!("{} at peak; {}", fmt_si(ppk, "W"), if status == "pass" { "within the package rating even if sustained".to_string() } else { format!("fine as a burst, too much if sustained: keep the average below {} ({} at this drop)", fmt_si(*pd, "W"), fmt_si(pd / (a.last().unwrap_or(&5.0) - b.last().unwrap_or(&3.3)).max(0.01), "A")) }) });
                    let drop: Vec<f64> = a.iter().zip(b).map(|(a, b)| a - b).collect();
                    if let Some(w) = windows.iter().find(|w| w.name.contains("running")) {
                        let r = data.range(w.from, w.to);
                        let m = drop[r].iter().cloned().fold(f64::INFINITY, f64::min);
                        measurements.push(Check { name: format!("{part} headroom (VIN − VOUT) while running"), value: Some(m), unit: "V", limit: String::new(), status: "pass", detail: fmt_si(m, "V") });
                    }
                }
            }
        }
        // Each probed voltage: its range and final value; start-up and burst figures where they apply.
        for s in sigs.iter().filter(|s| s.unit == "V") {
            let mn = s.values.iter().cloned().fold(f64::INFINITY, f64::min);
            let mx = s.values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let fin = *s.values.last().unwrap_or(&0.0);
            if mx.abs() < 1e-3 {
                continue;
            }
            let mut detail = format!("{} … {}, ends at {}", fmt_si(mn, "V"), fmt_si(mx, "V"), fmt_si(fin, "V"));
            if let (Some(a), Some(b)) = (data.cross(&s.values, 0.1 * fin, true, f64::NEG_INFINITY), data.cross(&s.values, 0.9 * fin, true, f64::NEG_INFINITY)) {
                if b > a {
                    let _ = write!(detail, "; 10–90 % rise {} (from {})", fmt_si(b - a, "s"), fmt_si(a, "s"));
                }
            }
            if let Some(w) = windows.iter().find(|w| w.name == "burst") {
                let r = data.range(w.from, w.to + (w.to - w.from));
                let lo = s.values[r].iter().cloned().fold(f64::INFINITY, f64::min);
                let before = s.values[data.range(w.from - 1e-5, w.from).start.saturating_sub(1)];
                if before - lo > 1e-3 {
                    let _ = write!(detail, "; dips {} to {} in the burst", fmt_si(before - lo, "V"), fmt_si(lo, "V"));
                }
            }
            measurements.push(Check { name: s.name.clone(), value: Some(fin), unit: "V", limit: String::new(), status: "pass", detail });
        }
        for s in sigs.iter().filter(|s| s.unit == "A") {
            let mx = s.values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            measurements.push(Check { name: s.name.clone(), value: Some(mx), unit: "A", limit: String::new(), status: "pass", detail: format!("peak {}, ends at {}", fmt_si(mx, "A"), fmt_si(*s.values.last().unwrap_or(&0.0), "A")) });
        }
    }
    let expectations: Vec<Check> = req
        .expect
        .iter()
        .map(|e| match expect(&data, e) {
            Ok((v, pass)) => Check { name: e.clone(), value: Some(v), unit: "", limit: String::new(), status: if pass { "pass" } else { "fail" }, detail: format!("{v:.6}") },
            Err(err) => Check { name: e.clone(), value: None, unit: "", limit: String::new(), status: "fail", detail: err },
        })
        .collect();
    let ok = checks.iter().chain(&expectations).all(|c| c.status != "fail");
    let x = decimate(&raw.x, &mut sigs, 3000);
    SimResult { analysis: req.analysis.clone(), x_name: raw.x_name.clone(), x_unit, x, signals: sigs, windows, checks, expectations, measurements, parts: nl.parts.clone(), skipped: nl.skipped.clone(), assumptions: nl.assumptions.clone(), netlist: deck, ok }
}

// ---- Saved simulations: requirements `./forma check` runs ------------------------------------

/// `pcb.simulation`: a named simulation of a schematic, kept with the project; its datasheet
/// checks and expectations are requirements (`./forma check` runs it).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SavedSimulation {
    pub schematic: de_core::StableId,
    pub name: String,
    pub request: SimRequest,
}

impl de_core::Persist for SavedSimulation {
    const KIND: &'static str = "pcb.simulation";
}

/// `pcb.simulation.save`: keep a simulation (the same name replaces it).
#[derive(Deserialize, JsonSchema)]
pub struct SaveSimulation {
    pub schematic: de_core::StableId,
    pub name: String,
    #[serde(default)]
    pub request: SimRequest,
}

impl de_core::Command for SaveSimulation {
    const NAME: &'static str = "pcb.simulation.save";

    fn apply(self, tx: &mut de_core::Tx) -> Result<de_core::Value, de_core::CommandError> {
        if tx.get::<crate::schematic::Schematic>(self.schematic).is_none() {
            return Err(crate::bad(format!("{} is not a schematic", self.schematic)));
        }
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(crate::bad("a simulation needs a name"));
        }
        let existing = saved(tx.world()).into_iter().find(|(_, s)| s.schematic == self.schematic && s.name == name).map(|(id, _)| id);
        let id = existing.unwrap_or_else(|| tx.create());
        tx.set(id, SavedSimulation { schematic: self.schematic, name, request: self.request })?;
        Ok(de_core::json!({ "id": id, "replaced": existing.is_some() }))
    }
}

/// `pcb.simulation.delete`
#[derive(Deserialize, JsonSchema)]
pub struct DeleteSimulation {
    pub target: de_core::StableId,
}

impl de_core::Command for DeleteSimulation {
    const NAME: &'static str = "pcb.simulation.delete";

    fn apply(self, tx: &mut de_core::Tx) -> Result<de_core::Value, de_core::CommandError> {
        if tx.get::<SavedSimulation>(self.target).is_none() {
            return Err(crate::bad(format!("{} is not a saved simulation", self.target)));
        }
        tx.delete(self.target)?;
        Ok(de_core::json!({ "deleted": self.target }))
    }
}

/// Every saved simulation.
pub fn saved(world: &de_core::bevy_ecs::world::World) -> Vec<(de_core::StableId, SavedSimulation)> {
    let mut v: Vec<(de_core::StableId, SavedSimulation)> = world.resource::<de_core::IdIndex>().iter().filter_map(|(id, e)| world.get::<SavedSimulation>(e).map(|s| (id, s.clone()))).collect();
    v.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    v
}

pub fn register(r: &mut de_core::Registry) {
    r.component::<SavedSimulation>().command::<SaveSimulation>().command::<DeleteSimulation>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse() {
        assert_eq!(parse_value("10k"), Some(10e3));
        assert!((parse_value("4k7").unwrap() - 4700.0).abs() < 1e-9);
        assert!((parse_value("100nF").unwrap() - 1e-7).abs() < 1e-18);
        assert!((parse_value("1µF").unwrap() - 1e-6).abs() < 1e-18);
        assert_eq!(parse_value("5.1 kΩ"), Some(5100.0));
        assert!((parse_value("2R2").unwrap() - 2.2).abs() < 1e-12);
        assert_eq!(parse_value("1M"), Some(1e6));
        assert_eq!(volts_in("AMS1117-3.3"), Some(3.3));
        assert_eq!(volts_in("5V1"), Some(5.1));
        assert!((ldo_for("ME6211C33", "").vset - 3.3).abs() < 1e-12);
        assert!((ldo_for("ME6211C18", "").vset - 1.8).abs() < 1e-12);
    }

    #[test]
    fn raw_files_parse() {
        let t = "Title: x\nPlotname: Transient Analysis\nFlags: real\nNo. Variables: 2\nNo. Points: 2\nVariables:\n\t0\ttime\ttime\n\t1\tv(a)\tvoltage\nValues:\n 0\t0.0\n\t1.0\n 1\t1e-3\n\t2.5\n";
        let r = parse_raw(t).unwrap();
        assert_eq!(r.x, vec![0.0, 1e-3]);
        assert_eq!(r.vecs[0].1, vec![1.0, 2.5]);
    }

    #[test]
    fn expectations_evaluate() {
        let x: Vec<f64> = (0..=100).map(|i| i as f64 * 1e-3).collect();
        let v: Vec<f64> = x.iter().map(|t| if *t < 0.05 { 3.3 } else { 3.1 }).collect();
        let w = vec![Window { name: "burst".into(), from: 0.05, to: 0.06 }];
        let d = Data { x: &x, sig: HashMap::from([("V(3V3)".to_string(), v.as_slice())]), windows: &w };
        assert!(expect(&d, "min(V(3V3), burst) > 3.0").unwrap().1);
        assert!(!expect(&d, "min(V(3V3), 0, 100ms) >= 3.2V").unwrap().1);
        let (t, ok) = expect(&d, "fall(V(3V3), 3.2) < 60ms").unwrap();
        assert!(ok && (t - 0.0495).abs() < 1e-3, "{t}");
        assert!(expect(&d, "max(V(nope)) > 1").is_err());
    }
}
