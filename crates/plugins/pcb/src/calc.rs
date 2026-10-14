//! Board calculators: the arithmetic a designer looks up while laying out a
//! board, from the published formulas, each with its source named in the
//! result. Track width against current (IPC-2221 and a curve fit of the
//! IPC-2152 baseline chart), via current, resistance and thermal resistance,
//! plated-hole and land size, conductor spacing (IPC-2221B table 6-1), board
//! classes (IPC-6012), impedance of microstrip, stripline and coplanar lines
//! (closed forms; a calculator, not a field solver), attenuators, E-series,
//! resistor combinations, colour codes, regulator dividers, wavelength, wire
//! gauge, fusing current (Onderdonk) and galvanic corrosion.
//!
//! Every calculator takes named numeric or word arguments and returns
//! `{ title, results: [{ label, value, unit }], notes, source, … }`, so the UI
//! and `./ise pcb calc` print any of them the same way.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::f64::consts::PI;

/// Copper resistivity at 20 °C (Ω·m) and its temperature coefficient (1/K).
pub const RHO_CU: f64 = 1.72e-8;
pub const ALPHA_CU: f64 = 0.00393;
/// Copper thermal conductivity (W/m·K).
pub const K_CU: f64 = 401.0;
/// Thickness of one ounce of copper per square foot (µm).
pub const OZ_UM: f64 = 34.79;
const C0: f64 = 299_792_458.0;

#[derive(Clone, Debug, Serialize)]
pub struct Input {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    /// The default, as text (a number or one of `options`).
    pub default: &'static str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub options: &'static [&'static str],
}

#[derive(Clone, Debug, Serialize)]
pub struct Calc {
    pub id: &'static str,
    pub title: &'static str,
    pub about: &'static str,
    pub inputs: Vec<Input>,
}

const fn num(key: &'static str, label: &'static str, unit: &'static str, default: &'static str) -> Input {
    Input { key, label, unit, default, options: &[] }
}
const fn pick(key: &'static str, label: &'static str, default: &'static str, options: &'static [&'static str]) -> Input {
    Input { key, label, unit: "", default, options }
}

const SERIES: &[&str] = &["E1", "E3", "E6", "E12", "E24", "E48", "E96", "E192"];

/// Every calculator with its inputs.
pub fn list() -> Vec<Calc> {
    vec![
        Calc {
            id: "track",
            title: "Track width for a current",
            about: "Width that carries a current with a given temperature rise (IPC-2221 or the IPC-2152 baseline fit); give a width instead to get the current it carries. Resistance, voltage drop and loss over a length.",
            inputs: vec![
                num("current", "Current", "A", "1"),
                num("rise", "Temperature rise", "°C", "10"),
                num("copper", "Copper", "oz", "1"),
                num("length", "Length", "mm", "50"),
                num("width", "Width (0 = solve for it)", "mm", "0"),
                num("ambient", "Ambient", "°C", "25"),
                pick("standard", "Standard", "IPC-2221", &["IPC-2221", "IPC-2152"]),
            ],
        },
        Calc {
            id: "via",
            title: "Via current and resistance",
            about: "A plated via's barrel: resistance, voltage drop, thermal resistance, the current for a temperature rise (IPC-2221 on the barrel's cross-section), and its capacitance and inductance (Johnson).",
            inputs: vec![
                num("hole", "Finished hole", "mm", "0.3"),
                num("plating", "Plating", "µm", "25"),
                num("length", "Length (board thickness)", "mm", "1.6"),
                num("pad", "Pad diameter", "mm", "0.6"),
                num("antipad", "Clearance hole in planes", "mm", "1.0"),
                num("current", "Current", "A", "1"),
                num("rise", "Temperature rise", "°C", "10"),
                num("er", "Dielectric εr", "", "4.5"),
                num("risetime", "Signal rise time", "ns", "1"),
            ],
        },
        Calc {
            id: "pth",
            title: "Plated hole and land for a lead",
            about: "Finished hole for a lead (IPC-2222 hole-to-lead allowance per producibility level) and the land around it: hole + 2 × minimum annular ring + fabrication allowance (IPC-2221 §9.1).",
            inputs: vec![num("lead", "Lead diameter (max)", "mm", "0.6"), pick("level", "Producibility level", "B", &["A", "B", "C"]), pick("class", "Class", "2", &["1", "2", "3"])],
        },
        Calc {
            id: "spacing",
            title: "Conductor spacing for a voltage",
            about: "Minimum spacing between conductors from IPC-2221B table 6-1 (DC or AC peak voltage), for every situation the table lists.",
            inputs: vec![num("voltage", "Voltage (DC or AC peak)", "V", "48")],
        },
        Calc { id: "classes", title: "Board classes", about: "IPC-6012 performance classes and IPC-2221 producibility levels: what each asks of the board.", inputs: vec![] },
        Calc {
            id: "impedance",
            title: "Line impedance",
            about: "Characteristic impedance from closed forms: microstrip (Hammerstad–Jensen with thickness correction), symmetric stripline (Cohn, Wheeler with thickness), coplanar waveguide with or without a ground below (conformal mapping); differential pairs from IPC-2141. A calculator for planning, not a field solver.",
            inputs: vec![
                pick("line", "Line", "microstrip", &["microstrip", "stripline", "cpw", "gcpw"]),
                num("width", "Track width", "mm", "0.3"),
                num("height", "Dielectric height (to plane; stripline: plane to plane)", "mm", "0.2"),
                num("thickness", "Copper thickness", "mm", "0.035"),
                num("er", "Dielectric εr", "", "4.3"),
                num("gap", "Gap to coplanar ground", "mm", "0.2"),
                num("pair", "Gap to the pair's other track (0 = single)", "mm", "0"),
            ],
        },
        Calc {
            id: "attenuator",
            title: "Attenuator",
            about: "Resistor values for a matched attenuator or splitter between equal impedances.",
            inputs: vec![pick("kind", "Type", "pi", &["pi", "tee", "bridged-tee", "splitter"]), num("db", "Attenuation", "dB", "6"), num("z0", "Impedance", "Ω", "50")],
        },
        Calc { id: "eseries", title: "E-series values", about: "The preferred values of a series (IEC 60063) in one decade.", inputs: vec![pick("series", "Series", "E24", SERIES)] },
        Calc {
            id: "resistors",
            title: "Resistors for a value",
            about: "The closest one, two or three standard resistors in series and parallel to a target.",
            inputs: vec![num("target", "Target", "Ω", "3300"), pick("series", "Series", "E24", SERIES), num("exclude", "Exclude (Ω, 0 = none)", "Ω", "0")],
        },
        Calc {
            id: "colour",
            title: "Resistor colour code",
            about: "Bands for a value (4 bands for 2 % and wider, 5 for closer), or the value of bands given as words (brown,black,red,gold).",
            inputs: vec![num("value", "Value", "Ω", "4700"), num("tolerance", "Tolerance", "%", "5"), num("tempco", "Temperature coefficient (6 bands; 0 = none)", "ppm/K", "0")],
        },
        Calc {
            id: "regulator",
            title: "Regulator divider",
            about: "Output of an adjustable regulator from its divider, or the divider for an output (the nearest standard pair). Feedback type: Vout = Vref·(1 + R1/R2), R1 from the output to FB. Three-terminal (LM317 type): Vout = Vref·(1 + R2/R1) + Iadj·R2, R1 from OUT to ADJ.",
            inputs: vec![
                pick("kind", "Type", "feedback", &["feedback", "3-terminal"]),
                num("vout", "Output (0 = solve from R1, R2)", "V", "3.3"),
                num("vref", "Reference", "V", "0.8"),
                num("r1", "R1 (0 = solve)", "Ω", "0"),
                num("r2", "R2", "Ω", "10000"),
                num("iadj", "Adjust pin current", "µA", "0"),
                pick("series", "Series", "E96", SERIES),
            ],
        },
        Calc { id: "wavelength", title: "Wavelength", about: "Wavelength, period and speed of a signal in a medium.", inputs: vec![num("frequency", "Frequency", "MHz", "2400"), num("er", "εr", "", "4.3"), num("mur", "µr", "", "1")] },
        Calc {
            id: "wire",
            title: "Wire gauge",
            about: "A solid copper wire of an AWG size (ASTM B258 diameters): resistance, voltage drop and loss over a length.",
            inputs: vec![num("awg", "AWG", "", "24"), num("length", "Length", "m", "1"), num("current", "Current", "A", "1"), num("temperature", "Wire temperature", "°C", "20")],
        },
        Calc {
            id: "fusing",
            title: "Fusing current",
            about: "The current that melts a copper conductor in a time, from Onderdonk's equation (adiabatic: no heat leaves, so a pessimistic bound for short pulses).",
            inputs: vec![num("width", "Track width", "mm", "0.25"), num("copper", "Copper", "oz", "1"), num("time", "Time", "s", "1"), num("ambient", "Ambient", "°C", "25"), num("melt", "Melting point", "°C", "1084")],
        },
        Calc {
            id: "galvanic",
            title: "Galvanic corrosion",
            about: "Anodic index of two metals (MIL-STD-889 table) and whether the pair is acceptable in harsh, normal or controlled surroundings.",
            inputs: vec![pick("a", "Metal", "gold", METAL_KEYS), pick("b", "Against", "tin", METAL_KEYS)],
        },
    ]
}

type Args = BTreeMap<String, String>;

fn arg(a: &Args, key: &str, def: f64) -> Result<f64, String> {
    match a.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(def),
        Some(s) => s.parse::<f64>().map_err(|_| format!("{key}: “{s}” is not a number")).and_then(|v| if v.is_finite() { Ok(v) } else { Err(format!("{key} is not finite")) }),
    }
}

fn word<'a>(a: &'a Args, key: &str, def: &'a str) -> &'a str {
    a.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or(def)
}

fn positive(v: f64, what: &str) -> Result<f64, String> {
    if v > 0.0 { Ok(v) } else { Err(format!("{what} must be more than 0")) }
}

fn row(label: &str, value: f64, unit: &str) -> Value {
    json!({ "label": label, "value": sig(value, 4), "unit": unit })
}

/// Rounded to `n` significant figures.
pub fn sig(v: f64, n: i32) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let p = 10f64.powi(n - 1 - v.abs().log10().floor() as i32);
    (v * p).round() / p
}

/// Runs one calculator by id.
pub fn run(id: &str, a: &Args) -> Result<Value, String> {
    let def = |k: &str| list().into_iter().find(|c| c.id == id).and_then(|c| c.inputs.into_iter().find(|i| i.key == k)).map(|i| i.default.to_string()).unwrap_or_default();
    let mut full = Args::new();
    if let Some(c) = list().into_iter().find(|c| c.id == id) {
        for i in c.inputs {
            full.insert(i.key.into(), a.get(i.key).cloned().unwrap_or_else(|| def(i.key)));
        }
    }
    for (k, v) in a {
        full.entry(k.clone()).or_insert_with(|| v.clone());
    }
    let a = &full;
    let mut out = match id {
        "track" => track(a)?,
        "via" => via(a)?,
        "pth" => pth(a)?,
        "spacing" => spacing(arg(a, "voltage", 48.0)?)?,
        "classes" => classes(),
        "impedance" => impedance(a)?,
        "attenuator" => attenuator(word(a, "kind", "pi"), arg(a, "db", 6.0)?, arg(a, "z0", 50.0)?)?,
        "eseries" => {
            let s = word(a, "series", "E24");
            let v = eseries(s).ok_or_else(|| format!("no series {s}"))?;
            json!({ "results": [row("Values per decade", v.len() as f64, "")], "values": v, "source": "IEC 60063" })
        }
        "resistors" => resistors(arg(a, "target", 3300.0)?, word(a, "series", "E24"), arg(a, "exclude", 0.0)?)?,
        "colour" => match a.get("bands").filter(|b| !b.trim().is_empty()) {
            Some(b) => decode_bands(b)?,
            None => colour_code(arg(a, "value", 4700.0)?, arg(a, "tolerance", 5.0)?, arg(a, "tempco", 0.0)?)?,
        },
        "regulator" => regulator(a)?,
        "wavelength" => wavelength(arg(a, "frequency", 2400.0)?, arg(a, "er", 4.3)?, arg(a, "mur", 1.0)?)?,
        "wire" => wire(arg(a, "awg", 24.0)?, arg(a, "length", 1.0)?, arg(a, "current", 1.0)?, arg(a, "temperature", 20.0)?)?,
        "fusing" => fusing(a)?,
        "galvanic" => galvanic(word(a, "a", "gold"), word(a, "b", "tin"))?,
        _ => return Err(format!("no calculator “{id}”; there are: {}", list().iter().map(|c| c.id).collect::<Vec<_>>().join(", "))),
    };
    if let (Value::Object(m), Some(c)) = (&mut out, list().into_iter().find(|c| c.id == id)) {
        m.insert("id".into(), json!(c.id));
        m.insert("title".into(), json!(c.title));
        m.insert("inputs".into(), json!(a));
        m.entry("notes").or_insert(json!([]));
    }
    Ok(out)
}

// ---- Track width -------------------------------------------------------------------

/// IPC-2221 cross-section (mil²) for a current and rise: I = k·ΔT^0.44·A^0.725.
pub fn ipc2221_area(current: f64, rise: f64, external: bool) -> f64 {
    let k = if external { 0.048 } else { 0.024 };
    (current / (k * rise.powf(0.44))).powf(1.0 / 0.725)
}

/// IPC-2221 current (A) for a cross-section (mil²).
pub fn ipc2221_current(area_mil2: f64, rise: f64, external: bool) -> f64 {
    let k = if external { 0.048 } else { 0.024 };
    k * rise.powf(0.44) * area_mil2.powf(0.725)
}

/// The IPC-2152 baseline chart (one track, 1.57 mm FR-4, no planes, still air) as the published
/// curve fit: A = (117.555·ΔT^−0.913 + 1.15)·I^(0.84·ΔT^−0.018 + 1.159), A in mil².
pub fn ipc2152_area(current: f64, rise: f64) -> f64 {
    (117.555 * rise.powf(-0.913) + 1.15) * current.powf(0.84 * rise.powf(-0.018) + 1.159)
}

fn ipc2152_current(area: f64, rise: f64) -> f64 {
    (area / (117.555 * rise.powf(-0.913) + 1.15)).powf(1.0 / (0.84 * rise.powf(-0.018) + 1.159))
}

const MIL: f64 = 0.0254; // mm

fn track(a: &Args) -> Result<Value, String> {
    let rise = positive(arg(a, "rise", 10.0)?, "the temperature rise")?;
    let oz = positive(arg(a, "copper", 1.0)?, "the copper weight")?;
    let len = arg(a, "length", 50.0)?.max(0.0);
    let width = arg(a, "width", 0.0)?;
    let ambient = arg(a, "ambient", 25.0)?;
    let std = word(a, "standard", "IPC-2221").to_ascii_uppercase();
    let t_mm = oz * OZ_UM / 1000.0;
    let t_mil = t_mm / MIL;
    let mut results = Vec::new();
    let mut notes = Vec::new();
    let (ext_w, int_w, current);
    if width > 0.0 {
        let area = (width / MIL) * t_mil;
        let (ie, ii) = if std == "IPC-2152" { let i = ipc2152_current(area, rise); (i, i) } else { (ipc2221_current(area, rise, true), ipc2221_current(area, rise, false)) };
        results.push(row("Current, outer layer", ie, "A"));
        results.push(row("Current, inner layer", ii, "A"));
        ext_w = width;
        int_w = width;
        current = arg(a, "current", 1.0)?;
    } else {
        current = positive(arg(a, "current", 1.0)?, "the current")?;
        let (ae, ai) = if std == "IPC-2152" { let x = ipc2152_area(current, rise); (x, x) } else { (ipc2221_area(current, rise, true), ipc2221_area(current, rise, false)) };
        ext_w = ae / t_mil * MIL;
        int_w = ai / t_mil * MIL;
        results.push(row("Width, outer layer", ext_w, "mm"));
        results.push(row("Width, inner layer", int_w, "mm"));
        results.push(row("Cross-section, outer", ae * MIL * MIL, "mm²"));
    }
    let hot = ambient + rise;
    let rho = RHO_CU * (1.0 + ALPHA_CU * (hot - 20.0));
    for (label, w) in [("outer", ext_w), ("inner", int_w)] {
        if len > 0.0 {
            let r = rho * (len / 1000.0) / ((w / 1000.0) * (t_mm / 1000.0));
            results.push(row(&format!("Resistance, {label} (at {} °C)", sig(hot, 3)), r * 1000.0, "mΩ"));
            results.push(row(&format!("Voltage drop, {label}"), r * current * 1000.0, "mV"));
            results.push(row(&format!("Power loss, {label}"), r * current * current * 1000.0, "mW"));
        }
        if std == "IPC-2152" {
            break;
        }
    }
    if std == "IPC-2152" {
        notes.push("IPC-2152 baseline: one track on 1.57 mm FR-4 with no copper planes nearby, in still air; a plane under it carries heat away, so real tracks run cooler (the standard's modifiers)".to_string());
        notes.push("IPC-2152 gives the same chart for inner and outer layers".to_string());
    } else {
        notes.push("IPC-2221 (from 1950s charts) is conservative for inner layers; IPC-2152 is the newer data".to_string());
    }
    Ok(json!({ "results": results, "notes": notes, "source": if std == "IPC-2152" { "IPC-2152 baseline chart, curve fit" } else { "IPC-2221 §6.2: I = k·ΔT^0.44·A^0.725 (k = 0.048 outer, 0.024 inner; A in mil²)" } }))
}

// ---- Via ---------------------------------------------------------------------------

fn via(a: &Args) -> Result<Value, String> {
    let d = positive(arg(a, "hole", 0.3)?, "the hole")?;
    let t = positive(arg(a, "plating", 25.0)?, "the plating")? / 1000.0;
    let len = positive(arg(a, "length", 1.6)?, "the length")?;
    let pad = arg(a, "pad", 0.6)?;
    let anti = arg(a, "antipad", 1.0)?;
    let current = arg(a, "current", 1.0)?.max(0.0);
    let rise = positive(arg(a, "rise", 10.0)?, "the temperature rise")?;
    let er = positive(arg(a, "er", 4.5)?, "εr")?;
    let tr = arg(a, "risetime", 1.0)?;
    // Barrel cross-section: a tube of outer diameter d + 2t (the plating grows inward to the finished hole).
    let area = PI * (d + t) * t; // mm²
    let r = RHO_CU * (len / 1000.0) / (area * 1e-6);
    let rth = (len / 1000.0) / (K_CU * area * 1e-6);
    let amps = ipc2221_current(area / (MIL * MIL), rise, true);
    let mut results = vec![
        row("Barrel cross-section", area, "mm²"),
        row("Resistance", r * 1000.0, "mΩ"),
        row("Voltage drop", r * current * 1000.0, "mV"),
        row("Power loss", r * current * current * 1000.0, "mW"),
        row("Thermal resistance", rth, "K/W"),
        row(&format!("Current for a {} °C rise", sig(rise, 3)), amps, "A"),
    ];
    if current > 0.0 {
        let dt = (current / (0.048 * (area / (MIL * MIL)).powf(0.725))).powf(1.0 / 0.44);
        results.push(row(&format!("Rise at {} A", sig(current, 3)), dt, "°C"));
    }
    let mut notes = vec!["current from IPC-2221's outer-layer formula on the barrel's cross-section".to_string()];
    let inch = |mm: f64| mm / 25.4;
    if anti > pad && pad > 0.0 {
        // Johnson: C = 1.41·εr·T·D1/(D2 − D1) pF, T and D in inches.
        let c = 1.41 * er * inch(len) * inch(pad) / (inch(anti) - inch(pad));
        results.push(row("Capacitance", c, "pF"));
        if tr > 0.0 {
            // A 50 Ω line's rise time degraded by the via: T10-90 = 2.2·C·Z0/2.
            results.push(row("Rise time added (50 Ω)", 2.2 * c * 25.0, "ps"));
        }
    } else {
        notes.push("capacitance needs a clearance hole wider than the pad".into());
    }
    // Johnson: L = 5.08·h·(ln(4h/d) + 1) nH, h and d in inches.
    let l = 5.08 * inch(len) * ((4.0 * len / d).ln() + 1.0);
    results.push(row("Inductance", l, "nH"));
    if tr > 0.0 {
        results.push(row(&format!("Reactance at {} ns rise", sig(tr, 3)), PI * l / tr, "Ω"));
    }
    Ok(json!({ "results": results, "notes": notes, "source": "IPC-2221 (current); Johnson & Graham, High-Speed Digital Design (C, L)" }))
}

// ---- Plated holes ------------------------------------------------------------------

fn pth(a: &Args) -> Result<Value, String> {
    let lead = positive(arg(a, "lead", 0.6)?, "the lead")?;
    let level = word(a, "level", "B").to_ascii_uppercase();
    let class = word(a, "class", "2");
    // IPC-2222 §9.2: minimum hole minus maximum lead; IPC-2221 table 9-1: fabrication allowance.
    let (gap, allowance) = match level.as_str() {
        "A" => (0.25, 0.4),
        "C" => (0.15, 0.2),
        _ => (0.20, 0.25),
    };
    let ring = 0.05; // IPC-2221 §9.1.1 minimum external annular ring
    let hole = lead + gap;
    let land = hole + 2.0 * ring + allowance;
    let accept = match class {
        "1" => "Class 1 accepts the hole breaking out of the land up to 180° (IPC-6012)",
        "3" => "Class 3 needs at least 0.050 mm of ring outside and 0.025 mm inside, no breakout (IPC-6012)",
        _ => "Class 2 accepts a 90° breakout of the hole from the land (IPC-6012)",
    };
    Ok(json!({
        "results": [row("Finished hole (minimum)", hole, "mm"), row("Drill (finished + 0.1 for plating)", hole + 0.1, "mm"), row("Land diameter (minimum)", land, "mm"), row("Annular ring designed", (land - hole) / 2.0, "mm")],
        "notes": [format!("level {level}: hole = lead + {gap} mm, land = hole + 2 × {ring} mm ring + {allowance} mm fabrication allowance"), accept.to_string()],
        "source": "IPC-2222 §9.2 (hole to lead), IPC-2221 §9.1 and table 9-1 (land)",
    }))
}

// ---- Spacing -----------------------------------------------------------------------

/// IPC-2221B table 6-1, mm: (upper volts, [B1..A7]); the last row is per volt above 500.
const SPACING: &[(f64, [f64; 7])] = &[
    (15.0, [0.05, 0.1, 0.1, 0.05, 0.13, 0.13, 0.13]),
    (30.0, [0.05, 0.1, 0.1, 0.05, 0.13, 0.25, 0.13]),
    (50.0, [0.1, 0.6, 0.6, 0.13, 0.13, 0.4, 0.13]),
    (100.0, [0.1, 0.6, 1.5, 0.13, 0.13, 0.5, 0.13]),
    (150.0, [0.2, 0.6, 3.2, 0.4, 0.4, 0.8, 0.4]),
    (170.0, [0.2, 1.25, 3.2, 0.4, 0.4, 0.8, 0.4]),
    (250.0, [0.2, 1.25, 6.4, 0.4, 0.4, 0.8, 0.4]),
    (300.0, [0.2, 1.25, 12.5, 0.4, 0.4, 0.8, 0.8]),
    (500.0, [0.25, 2.5, 12.5, 0.8, 0.8, 1.5, 0.8]),
];
const PER_VOLT: [f64; 7] = [0.0025, 0.005, 0.025, 0.00305, 0.00305, 0.00305, 0.00305];
const SPACING_COLUMNS: [&str; 7] = [
    "B1 inner layers",
    "B2 outer, bare, sea level to 3050 m",
    "B3 outer, bare, above 3050 m",
    "B4 outer, with permanent polymer coating",
    "A5 outer, conformal coating over the assembly",
    "A6 component lead, bare",
    "A7 component lead, conformal coated",
];

/// IPC-2221B minimum spacing (mm) per column for a voltage.
pub fn ipc2221_spacing(volts: f64) -> [f64; 7] {
    let v = volts.abs();
    if v > 500.0 {
        let base = SPACING[SPACING.len() - 1].1;
        let mut out = [0.0; 7];
        for i in 0..7 {
            out[i] = base[i] + PER_VOLT[i] * (v - 500.0);
        }
        return out;
    }
    SPACING.iter().find(|(top, _)| v <= *top).map(|r| r.1).unwrap_or(SPACING[0].1)
}

fn spacing(volts: f64) -> Result<Value, String> {
    let s = ipc2221_spacing(volts);
    let results: Vec<Value> = SPACING_COLUMNS.iter().zip(s).map(|(l, v)| row(l, v, "mm")).collect();
    Ok(json!({ "results": results, "notes": ["for mains and safety isolation the product standard (IEC 62368-1, IEC 60664-1) usually asks more: creepage by pollution degree and material group"], "source": "IPC-2221B table 6-1" }))
}

fn classes() -> Value {
    json!({
        "results": [],
        "table": {
            "columns": ["", "Class 1", "Class 2", "Class 3"],
            "rows": [
                ["What it is", "General electronics: function matters, looks do not", "Dedicated service: long life, uninterrupted service desired", "High reliability: must work on demand (medical, aerospace)"],
                ["Outer annular ring", "breakout up to 180°", "breakout up to 90°", "0.050 mm minimum"],
                ["Inner annular ring", "breakout up to 180°", "breakout up to 90°", "0.025 mm minimum"],
                ["Hole copper (average)", "20 µm", "20 µm", "25 µm"],
                ["Bow and twist", "0.75 % with surface-mount parts, 1.5 % without", "0.75 % / 1.5 %", "0.75 % / 1.5 %"],
            ],
        },
        "levels": {
            "columns": ["", "Level A", "Level B", "Level C"],
            "rows": [
                ["Meaning", "General design: easiest to make", "Moderate", "High: hardest to make"],
                ["Land fabrication allowance", "0.4 mm", "0.25 mm", "0.2 mm"],
                ["Hole minus lead", "0.25 mm", "0.20 mm", "0.15 mm"],
            ],
        },
        "notes": ["the fab limits under Fab tell whether a fab can make the board; the class is what the board must pass when inspected"],
        "source": "IPC-6012 (classes), IPC-2221 / IPC-2222 (levels)",
    })
}

// ---- Impedance ---------------------------------------------------------------------

/// Complete elliptic integral of the first kind K(k), by the arithmetic–geometric mean.
pub fn ellip_k(k: f64) -> f64 {
    let (mut a, mut b) = (1.0f64, (1.0 - k * k).max(0.0).sqrt());
    for _ in 0..40 {
        let (na, nb) = ((a + b) / 2.0, (a * b).sqrt());
        a = na;
        b = nb;
        if (a - b).abs() < 1e-15 {
            break;
        }
    }
    PI / (2.0 * a)
}

/// K(k)/K(k') with k' = √(1−k²).
fn kk(k: f64) -> f64 {
    ellip_k(k) / ellip_k((1.0 - k * k).max(0.0).sqrt())
}

fn z_air_microstrip(u: f64) -> f64 {
    let f = 6.0 + (2.0 * PI - 6.0) * (-(30.666 / u).powf(0.7528)).exp();
    59.9585 * (f / u + (1.0 + 4.0 / (u * u)).sqrt()).ln()
}

fn e_eff_microstrip(u: f64, er: f64) -> f64 {
    let a = 1.0 + ((u.powi(4) + (u / 52.0).powi(2)) / (u.powi(4) + 0.432)).ln() / 49.0 + (1.0 + (u / 18.1).powi(3)).ln() / 18.7;
    let b = 0.564 * ((er - 0.9) / (er + 3.0)).powf(0.053);
    (er + 1.0) / 2.0 + (er - 1.0) / 2.0 * (1.0 + 10.0 / u).powf(-a * b)
}

/// Microstrip (Hammerstad & Jensen 1980, with their thickness correction): (Z0 Ω, εeff).
pub fn microstrip(w: f64, h: f64, t: f64, er: f64) -> (f64, f64) {
    let u = w / h;
    let (u1, ur) = if t > 0.0 {
        let th = t / h;
        let coth = 1.0 / (6.517 * u).sqrt().tanh();
        let du1 = th / PI * (1.0 + 4.0 * std::f64::consts::E / (th * coth * coth)).ln();
        let dur = 0.5 * (1.0 + 1.0 / (er - 1.0).max(0.0).sqrt().cosh()) * du1;
        (u + du1, u + dur)
    } else {
        (u, u)
    };
    let ee = e_eff_microstrip(ur, er);
    let z = z_air_microstrip(ur) / ee.sqrt();
    let eeff = ee * (z_air_microstrip(u1) / z_air_microstrip(ur)).powi(2);
    (z, eeff)
}

/// Symmetric stripline centred between planes `b` apart: Cohn's exact form for a thin strip,
/// Wheeler's for a thick one. Z0 in Ω.
pub fn stripline(w: f64, b: f64, t: f64, er: f64) -> f64 {
    if t <= 0.0 {
        let k = 1.0 / (PI * w / (2.0 * b)).cosh();
        return 30.0 * PI / er.sqrt() * kk(k);
    }
    let x = t / b;
    let m = 6.0 / (3.0 + 2.0 * x / (1.0 - x));
    let dw = x / (PI * (1.0 - x)) * (1.0 - 0.5 * ((x / (2.0 - x)).powi(2) + (0.0796 * x / (w / b + 1.1 * x)).powf(m)).ln());
    let mp = w / (b - t) + dw;
    let q = 4.0 / (PI * mp);
    30.0 / er.sqrt() * (1.0 + q * (2.0 * q + ((2.0 * q).powi(2) + 6.27).sqrt())).ln()
}

/// Coplanar waveguide on a substrate of height h (ground beside the track only): (Z0, εeff).
pub fn cpw(w: f64, s: f64, h: f64, er: f64) -> (f64, f64) {
    let k0 = w / (w + 2.0 * s);
    let k1 = (PI * w / (4.0 * h)).sinh() / (PI * (w + 2.0 * s) / (4.0 * h)).sinh();
    let eeff = 1.0 + (er - 1.0) / 2.0 * kk(k1) / kk(k0);
    (30.0 * PI / eeff.sqrt() / kk(k0), eeff)
}

/// Coplanar waveguide with a ground plane below (conductor-backed): (Z0, εeff).
pub fn gcpw(w: f64, s: f64, h: f64, er: f64) -> (f64, f64) {
    let k0 = w / (w + 2.0 * s);
    let k3 = (PI * w / (4.0 * h)).tanh() / (PI * (w + 2.0 * s) / (4.0 * h)).tanh();
    let q = kk(k3) / kk(k0);
    let eeff = (1.0 + er * q) / (1.0 + q);
    (60.0 * PI / eeff.sqrt() / (kk(k0) + kk(k3)), eeff)
}

/// Edge-coupled symmetric stripline of zero thickness between planes `b` apart
/// (Cohn 1955, exact by conformal mapping): (Z0 even, Z0 odd) in Ω.
pub fn coupled_stripline_thin(w: f64, s: f64, b: f64, er: f64) -> (f64, f64) {
    let a = (PI * w / (2.0 * b)).tanh();
    let c = (PI * (w + s) / (2.0 * b)).tanh();
    let z = |k: f64| 30.0 * PI / er.sqrt() / kk(k.clamp(1e-12, 1.0 - 1e-12));
    (z(a * c), z(a / c))
}

/// Stripline off centre: `h1` of εr1 to the plane above, `h2` of εr2 below.
/// Each side as a centred stripline of twice its height, combined in
/// parallel (Cohn's offset approximation; IPC-2141A §4.2.3).
pub fn stripline_offset(w: f64, h1: f64, er1: f64, h2: f64, er2: f64, t: f64) -> f64 {
    let z1 = stripline(w, 2.0 * h1 + t, t, er1);
    let z2 = stripline(w, 2.0 * h2 + t, t, er2);
    2.0 * z1 * z2 / (z1 + z2)
}

/// Differential impedance of an edge-coupled offset stripline: Cohn's exact
/// odd mode per side, scaled by the thick-strip correction of the single
/// line, the two sides combined as for `stripline_offset`. Ω.
pub fn zdiff_stripline(w: f64, s: f64, h1: f64, er1: f64, h2: f64, er2: f64, t: f64) -> f64 {
    let side = |h: f64, er: f64| {
        let b = 2.0 * h + t;
        let (_, zo) = coupled_stripline_thin(w, s, b, er);
        let thick = if t > 0.0 && t < b { stripline(w, b, t, er) / stripline(w, b, 0.0, er) } else { 1.0 };
        zo * thick
    };
    let (a, b) = (side(h1, er1), side(h2, er2));
    2.0 * (2.0 * a * b / (a + b))
}

/// Differential impedance of an edge-coupled microstrip: Hammerstad–Jensen's
/// single line (with thickness) and IPC-2141's coupling term. Ω.
pub fn zdiff_microstrip(w: f64, s: f64, h: f64, t: f64, er: f64) -> f64 {
    2.0 * microstrip(w, h, t, er).0 * (1.0 - 0.48 * (-0.96 * s / h).exp())
}

/// The `x` in [lo, hi] (log-spaced bisection) where a monotonic `f(x)` meets `target`.
pub fn solve(f: &dyn Fn(f64) -> f64, target: f64, lo: f64, hi: f64) -> Option<f64> {
    let (flo, fhi) = (f(lo) - target, f(hi) - target);
    if !(flo.is_finite() && fhi.is_finite()) || flo * fhi > 0.0 {
        return None;
    }
    let (mut a, mut b) = (lo.ln(), hi.ln());
    let up = fhi > flo;
    for _ in 0..80 {
        let m = (a + b) / 2.0;
        let v = f(m.exp()) - target;
        if (v > 0.0) == up {
            b = m;
        } else {
            a = m;
        }
    }
    Some(((a + b) / 2.0).exp())
}

fn impedance(a: &Args) -> Result<Value, String> {
    let line = word(a, "line", "microstrip").to_ascii_lowercase();
    let w = positive(arg(a, "width", 0.3)?, "the width")?;
    let h = positive(arg(a, "height", 0.2)?, "the height")?;
    let t = arg(a, "thickness", 0.035)?.max(0.0);
    let er = positive(arg(a, "er", 4.3)?, "εr")?;
    let s = arg(a, "gap", 0.2)?;
    let pair = arg(a, "pair", 0.0)?;
    let mut notes = Vec::new();
    let (z, eeff, source) = match line.as_str() {
        "microstrip" => {
            let (z, e) = microstrip(w, h, t, er);
            (z, e, "Hammerstad & Jensen, “Accurate models for microstrip computer-aided design” (1980)")
        }
        "stripline" => {
            if t >= h {
                return Err("the track is thicker than the space between the planes".into());
            }
            (stripline(w, h, t, er), er, "Cohn (1954) for a thin strip, Wheeler (1978) with thickness; Wadell, Transmission Line Design Handbook §3.5")
        }
        "cpw" | "gcpw" => {
            positive(s, "the gap")?;
            notes.push("coplanar forms ignore the copper's thickness".to_string());
            let (z, e) = if line == "cpw" { cpw(w, s, h, er) } else { gcpw(w, s, h, er) };
            (z, e, "conformal mapping; Wadell, Transmission Line Design Handbook §3.4")
        }
        _ => return Err(format!("no line type {line}: microstrip, stripline, cpw or gcpw")),
    };
    let delay = eeff.sqrt() / C0 * 1e12 / 1000.0; // ps per mm
    let mut results = vec![row("Impedance Z0", z, "Ω"), row("Effective εr", eeff, ""), row("Delay", delay, "ps/mm"), row("Velocity", 1.0 / eeff.sqrt() * 100.0, "% of c")];
    if pair > 0.0 {
        let zd = match line.as_str() {
            "stripline" => 2.0 * z * (1.0 - 0.347 * (-2.9 * pair / h).exp()),
            _ => 2.0 * z * (1.0 - 0.48 * (-0.96 * pair / h).exp()),
        };
        results.push(row("Differential impedance", zd, "Ω"));
        notes.push("differential impedance from IPC-2141's edge-coupled approximation (good for gap/height of 0.1 to 3)".into());
    }
    notes.push("closed forms agree with a field solver to a few per cent inside their ranges; ask the fab for their stackup's numbers before a controlled-impedance order".into());
    Ok(json!({ "results": results, "notes": notes, "source": source }))
}

// ---- Attenuators -------------------------------------------------------------------

fn attenuator(kind: &str, db: f64, z0: f64) -> Result<Value, String> {
    let z0 = positive(z0, "the impedance")?;
    let k = 10f64.powf(db / 20.0);
    let (results, note) = match kind {
        "pi" => {
            positive(db, "the attenuation")?;
            (vec![row("R1, R3 (to ground)", z0 * (k + 1.0) / (k - 1.0), "Ω"), row("R2 (series)", z0 * (k * k - 1.0) / (2.0 * k), "Ω")], "R1 from input to ground, R2 in series, R3 from output to ground")
        }
        "tee" => {
            positive(db, "the attenuation")?;
            (vec![row("R1, R3 (series)", z0 * (k - 1.0) / (k + 1.0), "Ω"), row("R2 (to ground)", 2.0 * z0 * k / (k * k - 1.0), "Ω")], "R1 and R3 in series, R2 from their middle to ground")
        }
        "bridged-tee" | "bridged_tee" => {
            positive(db, "the attenuation")?;
            (vec![row("R1 (bridge)", z0 * (k - 1.0), "Ω"), row("R2 (to ground)", z0 / (k - 1.0), "Ω"), row("R3, R4 (the tee)", z0, "Ω")], "R3 and R4 in series, R1 across them, R2 from their middle to ground")
        }
        "splitter" => (vec![row("R1, R2, R3", z0 / 3.0, "Ω"), row("Loss to each output", 6.0206, "dB")], "a resistive two-way splitter: one resistor to each port from a common node"),
        _ => return Err(format!("no attenuator {kind}: pi, tee, bridged-tee or splitter")),
    };
    Ok(json!({ "results": results, "notes": [note], "source": "matched resistive attenuator equations (K = 10^(dB/20))" }))
}

// ---- E-series and resistors --------------------------------------------------------

const E24: [f64; 24] = [1.0, 1.1, 1.2, 1.3, 1.5, 1.6, 1.8, 2.0, 2.2, 2.4, 2.7, 3.0, 3.3, 3.6, 3.9, 4.3, 4.7, 5.1, 5.6, 6.2, 6.8, 7.5, 8.2, 9.1];

/// The values of an E-series in the decade 1–10 (IEC 60063).
pub fn eseries(name: &str) -> Option<Vec<f64>> {
    let n: usize = name.trim().trim_start_matches(['E', 'e']).parse().ok()?;
    match n {
        1 => Some(vec![1.0]),
        3 => Some(vec![1.0, 2.2, 4.7]),
        6 => Some(vec![1.0, 1.5, 2.2, 3.3, 4.7, 6.8]),
        12 => Some(E24.iter().step_by(2).copied().collect()),
        24 => Some(E24.to_vec()),
        48 | 96 | 192 => Some(
            (0..n)
                .map(|i| {
                    let v = (10f64.powf(i as f64 / n as f64) * 100.0).round() / 100.0;
                    // The one exception in the standard: E192 lists 9.20 where the formula gives 9.19.
                    if n == 192 && (v - 9.19).abs() < 1e-9 { 9.20 } else { v }
                })
                .collect(),
        ),
        _ => None,
    }
}

fn values_around(series: &[f64], target: f64, decades: i32) -> Vec<f64> {
    let d0 = target.log10().floor() as i32;
    let mut out = Vec::new();
    for d in d0 - decades..=d0 + decades {
        for v in series {
            let x = v * 10f64.powi(d);
            if x >= 0.1 && x <= 1e9 {
                out.push(x);
            }
        }
    }
    out
}

fn ohms(v: f64) -> String {
    let (x, unit) = if v >= 1e6 { (v / 1e6, "MΩ") } else if v >= 1e3 { (v / 1e3, "kΩ") } else { (v, "Ω") };
    format!("{}{unit}", sig(x, 4))
}

fn resistors(target: f64, series: &str, exclude: f64) -> Result<Value, String> {
    let target = positive(target, "the target")?;
    let s = eseries(series).ok_or_else(|| format!("no series {series}"))?;
    let vals: Vec<f64> = values_around(&s, target, 2).into_iter().filter(|v| exclude <= 0.0 || (v - exclude).abs() > 1e-9 * exclude).collect();
    let err = |v: f64| (v - target) / target;
    let mut best: Vec<(f64, f64, String)> = Vec::new(); // (|error|, value, how)
    // One.
    if let Some(v) = vals.iter().copied().min_by(|a, b| err(*a).abs().total_cmp(&err(*b).abs())) {
        best.push((err(v).abs(), v, ohms(v)));
    }
    // Two.
    let mut two: Option<(f64, f64, String)> = None;
    let take = |slot: &mut Option<(f64, f64, String)>, v: f64, how: &dyn Fn() -> String| {
        let e = err(v).abs();
        if slot.as_ref().is_none_or(|b| e < b.0 - 1e-15) {
            *slot = Some((e, v, how()));
        }
    };
    for (i, &a) in vals.iter().enumerate() {
        for &b in &vals[i..] {
            take(&mut two, a + b, &|| format!("{} + {}", ohms(a), ohms(b)));
            take(&mut two, a * b / (a + b), &|| format!("{} ∥ {}", ohms(a), ohms(b)));
        }
    }
    best.extend(two);
    // Three: only values within the reach of the target.
    // (Fine series would make the three-resistor search slow: it uses E24 then.)
    let coarse: Vec<f64> = if s.len() > 24 { values_around(&E24, target, 2).into_iter().filter(|v| exclude <= 0.0 || (v - exclude).abs() > 1e-9 * exclude).collect() } else { vals.clone() };
    let near: Vec<f64> = coarse.iter().copied().filter(|v| *v <= target * 20.0 && *v >= target / 20.0).collect();
    let mut three: Option<(f64, f64, String)> = None;
    for (i, &a) in near.iter().enumerate() {
        for (j, &b) in near.iter().enumerate().skip(i) {
            for &c in &near[j..] {
                take(&mut three, a + b + c, &|| format!("{} + {} + {}", ohms(a), ohms(b), ohms(c)));
                take(&mut three, 1.0 / (1.0 / a + 1.0 / b + 1.0 / c), &|| format!("{} ∥ {} ∥ {}", ohms(a), ohms(b), ohms(c)));
            }
        }
        for &b in &near {
            for &c in &near {
                take(&mut three, (a + b) * c / (a + b + c), &|| format!("({} + {}) ∥ {}", ohms(a), ohms(b), ohms(c)));
                take(&mut three, a * b / (a + b) + c, &|| format!("{} ∥ {} + {}", ohms(a), ohms(b), ohms(c)));
            }
        }
    }
    best.extend(three);
    let results: Vec<Value> = best
        .iter()
        .zip(["One resistor", "Two resistors", "Three resistors"])
        .map(|((e, v, how), label)| json!({ "label": label, "value": how, "unit": format!("= {} ({:+.4} %)", ohms(*v), e * 100.0 * (*v - target).signum()) }))
        .collect();
    Ok(json!({ "results": results, "source": format!("{series} values from 1/100 to 100 × the target") }))
}

// ---- Colour code -------------------------------------------------------------------

const COLOURS: [(&str, &str); 12] = [
    ("black", "#000000"),
    ("brown", "#7a4a1d"),
    ("red", "#d32222"),
    ("orange", "#f07f13"),
    ("yellow", "#f4d81f"),
    ("green", "#2a9d3c"),
    ("blue", "#2251d3"),
    ("violet", "#8d3ac8"),
    ("grey", "#8a8a8a"),
    ("white", "#f6f6f6"),
    ("gold", "#c9a227"),
    ("silver", "#c0c0c0"),
];
const TOLERANCES: [(f64, &str); 10] = [(1.0, "brown"), (2.0, "red"), (0.5, "green"), (0.25, "blue"), (0.1, "violet"), (0.05, "grey"), (5.0, "gold"), (10.0, "silver"), (0.02, "orange"), (0.01, "yellow")];
const TEMPCOS: [(f64, &str); 8] = [(100.0, "brown"), (50.0, "red"), (15.0, "orange"), (25.0, "yellow"), (10.0, "blue"), (5.0, "violet"), (1.0, "grey"), (250.0, "black")];

fn band(name: &str, role: &str) -> Value {
    let hex = COLOURS.iter().find(|c| c.0 == name).map(|c| c.1).unwrap_or("#000");
    json!({ "colour": name, "hex": hex, "role": role })
}

fn colour_code(value: f64, tol: f64, tempco: f64) -> Result<Value, String> {
    let value = positive(value, "the value")?;
    let digits = if tol < 2.0 || tempco > 0.0 { 3 } else { 2 };
    let exp = value.log10().floor() as i32 - (digits - 1);
    let mantissa = (value / 10f64.powi(exp)).round() as i64;
    let (mantissa, exp) = if mantissa >= 10i64.pow(digits as u32) { (mantissa / 10, exp + 1) } else { (mantissa, exp) };
    if !(-2..=9).contains(&exp) {
        return Err("the value is out of the colour code's range (0.01 Ω to 999 GΩ)".into());
    }
    let mut bands = Vec::new();
    let s = format!("{:0width$}", mantissa, width = digits as usize);
    for (i, ch) in s.chars().enumerate() {
        bands.push(band(COLOURS[ch.to_digit(10).unwrap() as usize].0, &format!("digit {}", i + 1)));
    }
    let mult = match exp {
        -2 => "silver",
        -1 => "gold",
        e => COLOURS[e as usize].0,
    };
    bands.push(band(mult, "multiplier"));
    let tband = TOLERANCES.iter().min_by(|a, b| (a.0 - tol).abs().total_cmp(&(b.0 - tol).abs())).unwrap();
    bands.push(band(tband.1, &format!("tolerance ±{} %", tband.0)));
    if tempco > 0.0 {
        let t = TEMPCOS.iter().min_by(|a, b| (a.0 - tempco).abs().total_cmp(&(b.0 - tempco).abs())).unwrap();
        bands.push(band(t.1, &format!("{} ppm/K", t.0)));
    }
    let actual = mantissa as f64 * 10f64.powi(exp);
    let mut notes = vec![];
    if (actual - value).abs() > 1e-9 * value {
        notes.push(format!("{} rounds to {} in {} digits", ohms(value), ohms(actual), digits));
    }
    Ok(json!({ "results": [json!({ "label": "Value", "value": ohms(actual), "unit": format!("±{} %", tband.0) }), json!({ "label": "Bands", "value": bands.iter().map(|b| b["colour"].as_str().unwrap_or("")).collect::<Vec<_>>().join(" "), "unit": "" })], "bands": bands, "notes": notes, "source": "IEC 60062" }))
}

fn decode_bands(words: &str) -> Result<Value, String> {
    let list: Vec<String> = words.split([',', ' ', '-']).map(|w| w.trim().to_ascii_lowercase().replace("gray", "grey").replace("purple", "violet")).filter(|w| !w.is_empty()).collect();
    if !(3..=6).contains(&list.len()) {
        return Err("give 3 to 6 bands, e.g. yellow,violet,red,gold".into());
    }
    let digit = |w: &str| COLOURS[..10].iter().position(|c| c.0 == w).ok_or_else(|| format!("{w} is not a digit colour"));
    let nd = if list.len() >= 5 { 3 } else { 2 };
    let mut m = 0i64;
    for w in &list[..nd] {
        m = m * 10 + digit(w)? as i64;
    }
    let mw = &list[nd];
    let exp = match mw.as_str() {
        "gold" => -1,
        "silver" => -2,
        w => digit(w)? as i32,
    };
    let value = m as f64 * 10f64.powi(exp);
    let tol = list.get(nd + 1).map(|w| TOLERANCES.iter().find(|t| t.1 == w).map(|t| t.0).ok_or_else(|| format!("{w} is not a tolerance colour"))).transpose()?.unwrap_or(20.0);
    let bands: Vec<Value> = list.iter().map(|w| band(w, "")).collect();
    Ok(json!({ "results": [json!({ "label": "Value", "value": ohms(value), "unit": format!("±{} %", tol) })], "ohms": value, "bands": bands, "source": "IEC 60062" }))
}

// ---- Regulator ---------------------------------------------------------------------

fn nearest(series: &[f64], v: f64) -> f64 {
    values_around(series, v, 1).into_iter().min_by(|a, b| ((a - v) / v).abs().total_cmp(&((b - v) / v).abs())).unwrap_or(v)
}

fn regulator(a: &Args) -> Result<Value, String> {
    let three = word(a, "kind", "feedback") == "3-terminal";
    let vref = positive(arg(a, "vref", if three { 1.25 } else { 0.8 })?, "the reference")?;
    let iadj = arg(a, "iadj", 0.0)? * 1e-6;
    let r1 = arg(a, "r1", 0.0)?;
    let r2 = arg(a, "r2", 10000.0)?;
    let vout = arg(a, "vout", 3.3)?;
    let sname = word(a, "series", "E96");
    let s = eseries(sname).ok_or_else(|| format!("no series {sname}"))?;
    let out_of = |r1: f64, r2: f64| if three { vref * (1.0 + r2 / r1) + iadj * r2 } else { vref * (1.0 + r1 / r2) };
    let mut results = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if r1 > 0.0 && r2 > 0.0 && vout <= 0.0 {
        results.push(row("Output", out_of(r1, r2), "V"));
    } else {
        let vout = positive(vout, "the output")?;
        if vout <= vref {
            return Err(format!("the output must be above the reference ({vref} V)"));
        }
        // Keep the given resistor, solve the other.
        let (k1, k2) = if three {
            let r1 = if r1 > 0.0 { r1 } else { 240.0 };
            let r2 = (vout - vref) / (vref / r1 + iadj);
            (r1, r2)
        } else {
            let r2 = positive(r2, "R2")?;
            (r2 * (vout / vref - 1.0), r2)
        };
        results.push(row("R1 exact", k1, "Ω"));
        results.push(row("R2 exact", k2, "Ω"));
        // Nearest standard pair: each standard value near the kept resistor, the other the nearest standard to its exact value.
        let mut best = (f64::INFINITY, 0.0, 0.0);
        for kept in values_around(&s, if three { k1 } else { k2 }, 1) {
            let (c1, c2) = if three { (kept, nearest(&s, (vout - vref) / (vref / kept + iadj))) } else { (nearest(&s, kept * (vout / vref - 1.0)), kept) };
            let e = (out_of(c1, c2) - vout).abs();
            if e < best.0 - 1e-12 {
                best = (e, c1, c2);
            }
        }
        results.push(json!({ "label": format!("Nearest {sname} pair"), "value": format!("R1 {} · R2 {}", ohms(best.1), ohms(best.2)), "unit": format!("→ {:.4} V ({:+.2} %)", out_of(best.1, best.2), (out_of(best.1, best.2) - vout) / vout * 100.0) }));
        if !three && r1 > 0.0 {
            notes.push("R1 is solved from R2; give vout 0 to compute the output from both".into());
        }
    }
    if three {
        notes.push("LM317-type: keep ≥ 3.5 mA flowing through R1 (240 Ω at 1.25 V does)".into());
    }
    Ok(json!({ "results": results, "notes": notes, "source": if three { "Vout = Vref·(1 + R2/R1) + Iadj·R2" } else { "Vout = Vref·(1 + R1/R2)" } }))
}

// ---- Wavelength, wire, fusing ------------------------------------------------------

fn wavelength(mhz: f64, er: f64, mur: f64) -> Result<Value, String> {
    let f = positive(mhz, "the frequency")? * 1e6;
    let v = C0 / (positive(er, "εr")? * positive(mur, "µr")?).sqrt();
    let l = v / f * 1000.0;
    Ok(json!({ "results": [row("Wavelength", l, "mm"), row("Quarter wave", l / 4.0, "mm"), row("Wavelength in free space", C0 / f * 1000.0, "mm"), row("Speed", v / C0 * 100.0, "% of c"), row("Period", 1e9 / f, "ns"), row("Delay", 1e12 / v / 1000.0, "ps/mm")], "source": "λ = c / (f·√(εr·µr))" }))
}

/// Diameter (mm) of an AWG gauge (ASTM B258): 0.127 × 92^((36 − n)/39); 0000 is −3.
pub fn awg_diameter(n: f64) -> f64 {
    0.127 * 92f64.powf((36.0 - n) / 39.0)
}

fn wire(awg: f64, len: f64, current: f64, temp: f64) -> Result<Value, String> {
    if !(-3.0..=50.0).contains(&awg) {
        return Err("AWG runs from −3 (0000) to 50".into());
    }
    let d = awg_diameter(awg);
    let area = PI * d * d / 4.0;
    let rho = RHO_CU * (1.0 + ALPHA_CU * (temp - 20.0));
    let r_m = rho / (area * 1e-6);
    let r = r_m * len;
    Ok(json!({
        "results": [row("Diameter", d, "mm"), row("Area", area, "mm²"), row("Resistance per metre", r_m * 1000.0, "mΩ/m"), row("Resistance", r * 1000.0, "mΩ"), row("Voltage drop", r * current, "V"), row("Power loss", r * current * current, "W"), row("Current density", current / area, "A/mm²")],
        "notes": ["a chassis wire carries far more than a bundled one: compare the current density with the cable maker's table"],
        "source": "ASTM B258 diameters, copper ρ = 1.72e-8 Ω·m at 20 °C",
    }))
}

/// Onderdonk: I = A·√(log10(1 + (Tm − Ta)/(234 + Ta)) / (33·t)), A in circular mils.
pub fn onderdonk(area_mm2: f64, seconds: f64, ambient: f64, melt: f64) -> f64 {
    let cmil = area_mm2 / (PI / 4.0 * MIL * MIL);
    cmil * ((1.0 + (melt - ambient) / (234.0 + ambient)).log10() / (33.0 * seconds)).sqrt()
}

fn fusing(a: &Args) -> Result<Value, String> {
    let w = positive(arg(a, "width", 0.25)?, "the width")?;
    let oz = positive(arg(a, "copper", 1.0)?, "the copper")?;
    let t = positive(arg(a, "time", 1.0)?, "the time")?;
    let ta = arg(a, "ambient", 25.0)?;
    let tm = arg(a, "melt", 1084.0)?;
    if tm <= ta {
        return Err("the melting point must be above the ambient".into());
    }
    let area = w * oz * OZ_UM / 1000.0;
    Ok(json!({ "results": [row("Fusing current", onderdonk(area, t, ta, tm), "A"), row("Cross-section", area, "mm²")], "notes": ["adiabatic: right for pulses under a few seconds; for longer ones the track sheds heat and carries more"], "source": "Onderdonk's equation (adiabatic heating of copper)" }))
}

// ---- Galvanic corrosion ------------------------------------------------------------

const METALS: [(&str, &str, f64); 20] = [
    ("gold", "Gold, solid and plated; gold-platinum", 0.00),
    ("rhodium", "Rhodium plated on silver-plated copper", 0.05),
    ("silver", "Silver, solid or plated; high-silver alloys", 0.15),
    ("nickel", "Nickel, solid or plated; Monel; high nickel-copper alloys", 0.30),
    ("copper", "Copper, solid or plated; low brasses or bronzes; silver solder", 0.35),
    ("brass", "Brass and bronzes", 0.40),
    ("high-brass", "High brasses and bronzes", 0.45),
    ("stainless-18cr", "18 % chromium stainless steels", 0.50),
    ("chromium", "Chromium plated; tin plated; 12 % chromium stainless steels", 0.60),
    ("tin", "Tin plate; tin-lead solder", 0.65),
    ("lead", "Lead, solid or plated; high-lead alloys", 0.70),
    ("aluminium-2000", "2000-series wrought aluminium", 0.75),
    ("steel", "Iron, wrought, grey or malleable; plain carbon and low-alloy steels", 0.85),
    ("aluminium", "Aluminium, wrought alloys other than 2000 series; silicon-type cast alloys", 0.90),
    ("aluminium-cast", "Aluminium, other cast alloys; cadmium plated and chromated", 0.95),
    ("galvanised", "Hot-dip zinc plate; galvanised steel", 1.20),
    ("zinc", "Zinc, wrought; zinc die-casting alloys; zinc plated", 1.25),
    ("magnesium", "Magnesium and magnesium alloys", 1.75),
    ("beryllium", "Beryllium", 1.85),
    ("enig", "ENIG finish (gold over nickel; the gold is thin: treat as nickel where worn)", 0.00),
];
const METAL_KEYS: &[&str] = &["gold", "rhodium", "silver", "nickel", "copper", "brass", "high-brass", "stainless-18cr", "chromium", "tin", "lead", "aluminium-2000", "steel", "aluminium", "aluminium-cast", "galvanised", "zinc", "magnesium", "beryllium", "enig"];

fn galvanic(a: &str, b: &str) -> Result<Value, String> {
    let find = |k: &str| METALS.iter().find(|m| m.0 == k.to_ascii_lowercase()).ok_or_else(|| format!("unknown metal {k}; known: {}", METAL_KEYS.join(", ")));
    let (ma, mb) = (find(a)?, find(b)?);
    let d = (ma.2 - mb.2).abs();
    let verdict = |limit: f64| if d <= limit + 1e-9 { "acceptable" } else { "corrodes" };
    let anode = if ma.2 > mb.2 { ma.1 } else { mb.1 };
    Ok(json!({
        "results": [row("Anodic index difference", d, "V"), json!({ "label": "Harsh (salt spray, outdoors; ≤ 0.15 V)", "value": verdict(0.15), "unit": "" }), json!({ "label": "Normal (indoors, no humidity control; ≤ 0.25 V)", "value": verdict(0.25), "unit": "" }), json!({ "label": "Controlled (≤ 0.50 V)", "value": verdict(0.50), "unit": "" })],
        "notes": [format!("{} is the anode: it is the one that corrodes", anode)],
        "table": { "columns": ["Metal", "Anodic index (V)"], "rows": METALS.iter().map(|m| json!([m.1, m.2])).collect::<Vec<_>>() },
        "source": "MIL-STD-889 anodic index",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[(&str, &str)]) -> Args {
        list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn value(out: &Value, label: &str) -> f64 {
        out["results"].as_array().unwrap().iter().find(|r| r["label"].as_str().unwrap().starts_with(label)).unwrap_or_else(|| panic!("no {label} in {out}"))["value"].as_f64().unwrap()
    }

    #[test]
    fn track_width_matches_ipc2221() {
        // 1 A, 10 °C rise, 1 oz: the usual published answer is 0.30 mm outside, 0.79 mm inside.
        let out = run("track", &args(&[("current", "1"), ("rise", "10"), ("copper", "1")])).unwrap();
        assert!((value(&out, "Width, outer") - 0.30).abs() < 0.01, "{out}");
        assert!((value(&out, "Width, inner") - 0.78).abs() < 0.02, "{out}");
        // And back: that width carries 1 A.
        assert!((ipc2221_current(ipc2221_area(2.5, 20.0, true), 20.0, true) - 2.5).abs() < 1e-9);
        // IPC-2152 fit at 1 A, 10 °C: about 15.5 mil².
        assert!((ipc2152_area(1.0, 10.0) - 15.52).abs() < 0.05);
    }

    #[test]
    fn via_resistance() {
        // 0.3 mm hole, 25 µm plating, 1.6 mm: π·0.325·0.025 mm² → 1.078 mΩ.
        let out = run("via", &args(&[("hole", "0.3"), ("plating", "25"), ("length", "1.6")])).unwrap();
        assert!((value(&out, "Resistance") - 1.078).abs() < 0.002, "{out}");
        assert!(value(&out, "Inductance") > 1.0 && value(&out, "Inductance") < 1.5);
    }

    #[test]
    fn spacing_table() {
        assert_eq!(ipc2221_spacing(48.0)[1], 0.6);
        assert_eq!(ipc2221_spacing(325.0)[1], 2.5);
        assert!((ipc2221_spacing(600.0)[0] - 0.5).abs() < 1e-12);
        assert_eq!(ipc2221_spacing(5.0)[0], 0.05);
    }

    #[test]
    fn line_impedances() {
        // K(1/√2) = K(1/√2)': stripline and CPW in air are exactly 30π Ω there.
        let w = (2.0f64.sqrt()).acosh() * 2.0 / PI; // sech(πw/2b) = 1/√2 with b = 1
        assert!((stripline(w, 1.0, 0.0, 1.0) - 30.0 * PI).abs() < 1e-9);
        let s = (2.0f64.sqrt() - 1.0) / 2.0; // w/(w+2s) = 1/√2 with w = 1
        let (z, e) = cpw(1.0, s, 1e6, 1.0);
        assert!((z - 30.0 * PI).abs() < 1e-6 && (e - 1.0).abs() < 1e-9);
        // Wheeler with a vanishing strip agrees with Cohn.
        assert!((stripline(0.3, 1.0, 1e-6, 4.3) - stripline(0.3, 1.0, 0.0, 4.3)).abs() / stripline(0.3, 1.0, 0.0, 4.3) < 0.02);
        // The textbook 50 Ω microstrip: 3.0 mm on 1.6 mm FR-4 (εr 4.5).
        let (z, eeff) = microstrip(3.0, 1.6, 0.035, 4.5);
        assert!((z - 50.0).abs() < 2.0, "{z}");
        assert!(eeff > 3.2 && eeff < 3.6, "{eeff}");
        // W/H = 1 in air: Hammerstad-Jensen gives 126 Ω.
        assert!((microstrip(1.0, 1.0, 0.0, 1.0).0 - 126.0).abs() < 1.5);
        // A conductor-backed CPW is lower than one without the plane.
        assert!(gcpw(0.3, 0.15, 0.2, 4.3).0 < cpw(0.3, 0.15, 0.2, 4.3).0);
    }

    #[test]
    fn attenuators() {
        let pi = run("attenuator", &args(&[("kind", "pi"), ("db", "3"), ("z0", "50")])).unwrap();
        assert!((value(&pi, "R1") - 292.4).abs() < 0.2 && (value(&pi, "R2") - 17.61).abs() < 0.02, "{pi}");
        let tee = run("attenuator", &args(&[("kind", "tee"), ("db", "3"), ("z0", "50")])).unwrap();
        assert!((value(&tee, "R1") - 8.55).abs() < 0.01 && (value(&tee, "R2") - 141.9).abs() < 0.1, "{tee}");
    }

    #[test]
    fn eseries_values() {
        let e96 = eseries("E96").unwrap();
        assert_eq!(e96.len(), 96);
        assert_eq!(&e96[..4], &[1.0, 1.02, 1.05, 1.07]);
        assert_eq!(e96[95], 9.76);
        assert!(eseries("E192").unwrap().contains(&9.20));
        assert_eq!(eseries("E12").unwrap(), vec![1.0, 1.2, 1.5, 1.8, 2.2, 2.7, 3.3, 3.9, 4.7, 5.6, 6.8, 8.2]);
    }

    #[test]
    fn resistor_combinations() {
        let out = run("resistors", &args(&[("target", "3300"), ("series", "E24"), ("exclude", "3300")])).unwrap();
        let rows = out["results"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        // 3.3k excluded: two resistors reach it exactly (e.g. 1.5k + 1.8k).
        assert!(rows[1]["unit"].as_str().unwrap().contains("+0.0000 %") || rows[1]["unit"].as_str().unwrap().contains("-0.0000 %"), "{out}");
    }

    #[test]
    fn colour_codes() {
        let out = colour_code(4700.0, 5.0, 0.0).unwrap();
        let names: Vec<&str> = out["bands"].as_array().unwrap().iter().map(|b| b["colour"].as_str().unwrap()).collect();
        assert_eq!(names, ["yellow", "violet", "red", "gold"]);
        let out = colour_code(10_000.0, 1.0, 0.0).unwrap();
        let names: Vec<&str> = out["bands"].as_array().unwrap().iter().map(|b| b["colour"].as_str().unwrap()).collect();
        assert_eq!(names, ["brown", "black", "black", "red", "brown"]);
        assert_eq!(decode_bands("yellow,violet,red,gold").unwrap()["ohms"].as_f64().unwrap(), 4700.0);
        assert!((decode_bands("brown black gold gold").unwrap()["ohms"].as_f64().unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn regulator_divider() {
        let out = run("regulator", &args(&[("kind", "feedback"), ("vout", "3.3"), ("vref", "0.8"), ("r2", "10000")])).unwrap();
        assert!((value(&out, "R1 exact") - 31250.0).abs() < 1.0, "{out}");
        let out = run("regulator", &args(&[("kind", "3-terminal"), ("vout", "0"), ("vref", "1.25"), ("r1", "240"), ("r2", "720")])).unwrap();
        assert!((value(&out, "Output") - 5.0).abs() < 1e-9, "{out}");
    }

    #[test]
    fn wire_and_fusing() {
        // AWG 24 is 0.511 mm; AWG 10 is 2.588 mm.
        assert!((awg_diameter(24.0) - 0.511).abs() < 0.001);
        assert!((awg_diameter(10.0) - 2.588).abs() < 0.001);
        // Onderdonk by hand for 1 oz × 0.25 mm (17.17 cmil) over 1 s from 25 °C:
        // 17.17 · √(log10(1 + 1059/259) / 33) = 2.51 A.
        let i = onderdonk(0.25 * 0.03479, 1.0, 25.0, 1084.0);
        assert!((i - 2.51).abs() < 0.01, "{i}");
        // A tenth of the time: √10 times the current.
        assert!((onderdonk(1.0, 0.1, 25.0, 1084.0) / onderdonk(1.0, 1.0, 25.0, 1084.0) - 10f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn galvanic_pairs() {
        let out = run("galvanic", &args(&[("a", "gold"), ("b", "tin")])).unwrap();
        assert!((value(&out, "Anodic") - 0.65).abs() < 1e-9);
        assert_eq!(out["results"][3]["value"], "corrodes");
        let out = run("galvanic", &args(&[("a", "nickel"), ("b", "copper")])).unwrap();
        assert_eq!(out["results"][1]["value"], "acceptable");
    }

    #[test]
    fn every_calculator_runs_on_its_defaults() {
        for c in list() {
            run(c.id, &Args::new()).unwrap_or_else(|e| panic!("{}: {e}", c.id));
        }
    }
}
