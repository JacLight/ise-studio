//! Schematic symbols: drawn here, not copied from any library. The look is
//! IEC/ANSI-neutral and close to what KiCad users expect (rectangle
//! resistors, plate capacitors, filled-outline diodes, yellow IC boxes).
//!
//! Coordinates: millimetres, **y down** (as on the sheet), origin at the
//! symbol's centre. Pin connection points sit on the 1.27 mm (50 mil) grid,
//! which is the schematic's snapping grid. A pin's `at` is its connection
//! point (the outer end, where wires attach); `side` is the direction it
//! leaves the body in, so the body end is `at − side·length`.
//!
//! Every pin lands on one or more footprint pads (`pads`, default the pin
//! number), so a symbol pin "GND" of the USB-C receptacle lands on A1, A12,
//! B1 and B12 at once. Instances may override this with their own pin map.
//! Families are parametric: `Conn_01x07`, `Conn_02x05`, `JST_PH_4`, … work
//! for any count.

use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Electrical type of a pin, for the electrical-rules check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PinType {
    Input,
    Output,
    Bidirectional,
    TriState,
    #[default]
    Passive,
    PowerIn,
    PowerOut,
    OpenCollector,
    OpenEmitter,
    NoConnect,
    Unspecified,
}

/// Which way a pin leaves the body (and so which way a wire leaves it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    /// Unit vector on the sheet (y down).
    pub fn v(self) -> [f64; 2] {
        match self {
            Dir::Left => [-1.0, 0.0],
            Dir::Right => [1.0, 0.0],
            Dir::Up => [0.0, -1.0],
            Dir::Down => [0.0, 1.0],
        }
    }
    pub fn from_v(v: [f64; 2]) -> Dir {
        if v[0].abs() >= v[1].abs() {
            if v[0] < 0.0 { Dir::Left } else { Dir::Right }
        } else if v[1] < 0.0 {
            Dir::Up
        } else {
            Dir::Down
        }
    }
}

/// How a pin is drawn where it meets the body (IEEE 91 / KiCad's graphic styles).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PinShape {
    #[default]
    Line,
    /// A bubble: active low / inverted.
    Inverted,
    /// A clock wedge inside the body.
    Clock,
    InvertedClock,
    /// Active-low input: a half-arrow outside the body.
    InputLow,
    ClockLow,
    /// Active-low output.
    OutputLow,
    /// Falling-edge clock.
    EdgeClockHigh,
    /// Not a logic connection (a cross on the pin).
    NonLogic,
}

impl PinShape {
    pub fn is_line(&self) -> bool {
        *self == PinShape::Line
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PinDef {
    /// Pin number as the datasheet gives it ("1", "A4").
    pub number: String,
    /// Pin function ("GND", "EN", "IO0"); "~" for none.
    #[serde(default)]
    pub name: String,
    /// Connection point (symbol coordinates, mm, y down).
    pub at: [f64; 2],
    pub side: Dir,
    #[serde(default = "pin_len")]
    pub length: f64,
    #[serde(default)]
    pub etype: PinType,
    /// Not drawn (no-connect pins of a module); still mapped to pads.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Unit it belongs to (1-based); 0: every unit.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unit: u32,
    /// Footprint pads it lands on; empty: the pad numbered like the pin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pads: Vec<String>,
    /// Graphic style where it meets the body (bubble, clock, …).
    #[serde(default, skip_serializing_if = "PinShape::is_line")]
    pub shape: PinShape,
}

fn pin_len() -> f64 {
    2.54
}
fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl PinDef {
    pub fn pads(&self) -> Vec<String> {
        if self.pads.is_empty() { vec![self.number.clone()] } else { self.pads.clone() }
    }
    pub fn in_unit(&self, unit: u32) -> bool {
        self.unit == 0 || self.unit == unit
    }
}

/// How a closed shape is filled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    #[default]
    None,
    /// Solid, in the outline's colour (a diode's triangle).
    Outline,
    /// The pale body colour (an IC box).
    Body,
}

/// Drawing of a symbol body (symbol coordinates).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Gfx {
    /// An open polyline.
    Line {
        pts: Vec<[f64; 2]>,
        #[serde(default)]
        width: f64,
    },
    /// A closed polygon.
    Poly {
        pts: Vec<[f64; 2]>,
        #[serde(default)]
        fill: Fill,
    },
    Rect {
        a: [f64; 2],
        b: [f64; 2],
        #[serde(default)]
        fill: Fill,
    },
    Circle {
        c: [f64; 2],
        r: f64,
        #[serde(default)]
        fill: Fill,
    },
    /// Counter-clockwise as seen on the sheet, degrees (0 = +x).
    Arc { c: [f64; 2], r: f64, start: f64, end: f64 },
    Text { at: [f64; 2], text: String, size: f64 },
}

/// A library symbol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolDef {
    pub name: String,
    pub description: String,
    /// Reference prefix: R, C, U, J…
    pub prefix: String,
    /// Group in the picker: Passive, Diode, Transistor, Connector, Power IC, Module, IC, Power, Misc.
    pub category: String,
    #[serde(default)]
    pub value: String,
    /// Default footprint ("" for none chosen: pick one).
    #[serde(default)]
    pub footprint: String,
    /// Other footprints that fit the same pin map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footprints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcsc: Option<String>,
    pub pins: Vec<PinDef>,
    /// Drawing per unit (index 0 is unit 1); a single-unit symbol has one.
    pub units: Vec<Vec<Gfx>>,
    #[serde(default)]
    pub show_pin_numbers: bool,
    #[serde(default)]
    pub show_pin_names: bool,
    /// Not a board part (a power flag): no footprint, never on the board.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub virtual_part: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// The other body style per unit (De Morgan: an AND drawn as an OR with
    /// inverted pins); empty when the symbol has one. A placed part draws
    /// `units`; changing style swaps the two (`pcb.lib.body_style`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub demorgan: Vec<Vec<Gfx>>,
    /// Pin shapes of the other body style, by pin number (De Morgan's inverted pins).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub demorgan_pins: BTreeMap<String, PinShape>,
    /// Which body style `units` draws now: 1 normal, 2 De Morgan.
    #[serde(default = "one_style", skip_serializing_if = "is_one_style")]
    pub body_style: u8,
    /// Other library fields (Datasheet, Tolerance, Voltage, …), copied to every placement.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

fn one_style() -> u8 {
    1
}
fn is_one_style(v: &u8) -> bool {
    *v <= 1
}

impl SymbolDef {
    pub fn unit_count(&self) -> u32 {
        self.units.len().max(1) as u32
    }
    pub fn unit_pins(&self, unit: u32) -> impl Iterator<Item = &PinDef> {
        self.pins.iter().filter(move |p| p.in_unit(unit))
    }
    /// Body extents of a unit [x0, y0, x1, y1] (graphics and pin body ends).
    pub fn body_bbox(&self, unit: u32) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut add = |p: [f64; 2]| {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        };
        let gfx = self.units.get(unit.saturating_sub(1) as usize).or(self.units.first());
        for g in gfx.into_iter().flatten() {
            match g {
                Gfx::Line { pts, .. } | Gfx::Poly { pts, .. } => pts.iter().for_each(|p| add(*p)),
                Gfx::Rect { a, b, .. } => {
                    add(*a);
                    add(*b);
                }
                Gfx::Circle { c, r, .. } | Gfx::Arc { c, r, .. } => {
                    add([c[0] - r, c[1] - r]);
                    add([c[0] + r, c[1] + r]);
                }
                Gfx::Text { at, .. } => add(*at),
            }
        }
        for p in self.unit_pins(unit).filter(|p| !p.hidden) {
            let v = p.side.v();
            add([p.at[0] - v[0] * p.length, p.at[1] - v[1] * p.length]);
        }
        if !b[0].is_finite() {
            return [-1.27, -1.27, 1.27, 1.27];
        }
        b
    }
}

// ---- Building blocks --------------------------------------------------------

const G: f64 = 2.54;

fn pin(number: &str, name: &str, at: [f64; 2], side: Dir, length: f64, etype: PinType) -> PinDef {
    PinDef { number: number.into(), name: name.into(), at, side, length, etype, hidden: false, unit: 0, pads: vec![], shape: PinShape::Line }
}

fn line(pts: &[[f64; 2]]) -> Gfx {
    Gfx::Line { pts: pts.to_vec(), width: 0.0 }
}
fn thick(pts: &[[f64; 2]], width: f64) -> Gfx {
    Gfx::Line { pts: pts.to_vec(), width }
}
fn poly(pts: &[[f64; 2]], fill: Fill) -> Gfx {
    Gfx::Poly { pts: pts.to_vec(), fill }
}
fn rect(a: [f64; 2], b: [f64; 2], fill: Fill) -> Gfx {
    Gfx::Rect { a, b, fill }
}

fn base(name: &str, prefix: &str, category: &str, description: &str) -> SymbolDef {
    SymbolDef {
        name: name.into(),
        description: description.into(),
        prefix: prefix.into(),
        category: category.into(),
        value: String::new(),
        footprint: String::new(),
        footprints: vec![],
        mpn: None,
        manufacturer: None,
        lcsc: None,
        pins: vec![],
        units: vec![vec![]],
        show_pin_numbers: false,
        show_pin_names: false,
        virtual_part: false,
        keywords: vec![],
        demorgan: vec![],
        demorgan_pins: BTreeMap::new(),
        body_style: 1,
        fields: BTreeMap::new(),
    }
}

fn chip_footprints(p: &str) -> Vec<String> {
    ["0402", "0603", "0805", "1206"].iter().map(|c| format!("{p}_{c}")).collect()
}

/// A two-pin vertical part: pin 1 on top at (0, −3.81), pin 2 below.
fn two_pin_vertical(s: &mut SymbolDef, body_half: f64) {
    let l = 3.81 - body_half;
    s.pins = vec![pin("1", "~", [0.0, -3.81], Dir::Up, l, PinType::Passive), pin("2", "~", [0.0, 3.81], Dir::Down, l, PinType::Passive)];
}

fn resistor() -> SymbolDef {
    let mut s = base("R", "R", "Passive", "Resistor");
    s.value = "10k".into();
    s.footprint = "R_0603".into();
    s.footprints = chip_footprints("R");
    s.keywords = vec!["resistor".into(), "res".into()];
    s.units = vec![vec![rect([-1.016, -2.54], [1.016, 2.54], Fill::None)]];
    two_pin_vertical(&mut s, 2.54);
    s
}

fn capacitor() -> SymbolDef {
    let mut s = base("C", "C", "Passive", "Capacitor, non-polarised (ceramic)");
    s.value = "100nF".into();
    s.footprint = "C_0603".into();
    s.footprints = chip_footprints("C");
    s.keywords = vec!["capacitor".into(), "cap".into(), "ceramic".into(), "mlcc".into()];
    s.units = vec![vec![thick(&[[-2.032, -0.762], [2.032, -0.762]], 0.508), thick(&[[-2.032, 0.762], [2.032, 0.762]], 0.508)]];
    two_pin_vertical(&mut s, 0.762);
    s
}

fn capacitor_polarized() -> SymbolDef {
    let mut s = base("C_Polarized", "C", "Passive", "Capacitor, polarised (electrolytic, tantalum); pin 1 is +");
    s.value = "100uF".into();
    s.footprint = "C_1206".into();
    s.footprints = chip_footprints("C");
    s.keywords = vec!["electrolytic".into(), "tantalum".into(), "polar".into()];
    s.units = vec![vec![
        rect([-2.286, -1.27], [2.286, -0.508], Fill::None),
        rect([-2.286, 0.508], [2.286, 1.27], Fill::Outline),
        line(&[[-1.778, -2.286], [-0.762, -2.286]]),
        line(&[[-1.27, -2.794], [-1.27, -1.778]]),
    ]];
    two_pin_vertical(&mut s, 1.27);
    s
}

fn inductor() -> SymbolDef {
    let mut s = base("L", "L", "Passive", "Inductor");
    s.value = "10uH".into();
    s.footprint = "R_0805".into();
    s.footprints = chip_footprints("R");
    s.keywords = vec!["inductor".into(), "choke".into(), "coil".into()];
    let r = 0.635;
    s.units = vec![[-1.905, -0.635, 0.635, 1.905].iter().map(|y| Gfx::Arc { c: [0.0, *y], r, start: -90.0, end: 90.0 }).collect()];
    two_pin_vertical(&mut s, 2.54);
    s
}

fn fuse() -> SymbolDef {
    let mut s = base("Fuse", "F", "Passive", "Fuse (chip fuse on a 1206 land, or PTC)");
    s.value = "500mA".into();
    s.footprint = "R_1206".into();
    s.footprints = chip_footprints("R");
    s.keywords = vec!["fuse".into(), "ptc".into(), "polyfuse".into()];
    s.units = vec![vec![rect([-0.762, -2.54], [0.762, 2.54], Fill::None), line(&[[0.0, -2.54], [0.0, 2.54]])]];
    two_pin_vertical(&mut s, 2.54);
    s
}

fn crystal() -> SymbolDef {
    let mut s = base("Crystal", "Y", "Passive", "Quartz crystal, two pins");
    s.value = "16MHz".into();
    s.keywords = vec!["crystal".into(), "xtal".into(), "resonator".into()];
    s.units = vec![vec![
        rect([-0.762, -1.778], [0.762, 1.778], Fill::None),
        thick(&[[-1.524, -1.27], [-1.524, 1.27]], 0.381),
        thick(&[[1.524, -1.27], [1.524, 1.27]], 0.381),
    ]];
    s.pins = vec![pin("1", "1", [-3.81, 0.0], Dir::Left, 2.286, PinType::Passive), pin("2", "2", [3.81, 0.0], Dir::Right, 2.286, PinType::Passive)];
    s
}

/// Diode family, horizontal: anode (2) left, cathode (1) right (pin 1 = cathode, as chip LED and SOD footprints).
fn diode(kind: &str) -> SymbolDef {
    let (name, desc, prefix) = match kind {
        "led" => ("LED", "Light-emitting diode (pin 1 cathode, 2 anode)", "D"),
        "schottky" => ("D_Schottky", "Schottky diode (pin 1 cathode)", "D"),
        "zener" => ("D_Zener", "Zener diode (pin 1 cathode)", "D"),
        _ => ("D", "Diode (pin 1 cathode, 2 anode)", "D"),
    };
    let mut s = base(name, prefix, "Diode", desc);
    let tri = poly(&[[-1.27, -1.27], [1.27, 0.0], [-1.27, 1.27]], Fill::Outline);
    let mut g = vec![tri, line(&[[-1.27, 0.0], [1.27, 0.0]])];
    match kind {
        "schottky" => g.push(line(&[[1.905, -1.27], [1.905, -1.651], [1.27, -1.651], [1.27, 1.651], [0.635, 1.651], [0.635, 1.27]])),
        "zener" => g.push(line(&[[0.762, -1.651], [1.27, -1.27], [1.27, 1.27], [1.778, 1.651]])),
        _ => g.push(thick(&[[1.27, -1.27], [1.27, 1.27]], 0.254)),
    }
    if kind == "led" {
        // Light leaving: two arrows up and to the right.
        for dx in [-0.508, 0.762] {
            let a = [dx, -1.778];
            let b = [dx + 1.27, -3.048];
            g.push(line(&[a, b]));
            g.push(poly(&[b, [b[0] - 0.635, b[1] + 0.127], [b[0] - 0.127, b[1] + 0.635]], Fill::Outline));
        }
        s.value = "Red".into();
        s.footprint = "LED_0603".into();
        s.footprints = vec!["LED_0402".into(), "LED_0603".into(), "LED_0805".into(), "LED_1206".into(), "LED_3mm".into(), "LED_5mm".into()];
        s.keywords = vec!["led".into(), "light".into()];
    } else {
        s.value = match kind {
            "schottky" => "SS34".into(),
            "zener" => "5V1".into(),
            _ => "1N4148".into(),
        };
        s.keywords = vec!["diode".into(), kind.into()];
    }
    s.units = vec![g];
    s.pins = vec![pin("1", "K", [3.81, 0.0], Dir::Right, 2.54, PinType::Passive), pin("2", "A", [-3.81, 0.0], Dir::Left, 2.54, PinType::Passive)];
    s
}

/// BJT in SOT-23: 1 base, 2 emitter, 3 collector (MMBT3904/3906 pinout).
fn bjt(pnp: bool) -> SymbolDef {
    let mut s = base(if pnp { "Q_PNP" } else { "Q_NPN" }, "Q", "Transistor", if pnp { "PNP transistor, SOT-23 (1 B, 2 E, 3 C)" } else { "NPN transistor, SOT-23 (1 B, 2 E, 3 C)" });
    s.value = if pnp { "MMBT3906".into() } else { "MMBT3904".into() };
    s.footprint = "SOT-23".into();
    s.keywords = vec!["transistor".into(), "bjt".into(), if pnp { "pnp" } else { "npn" }.into()];
    let bar = -0.635;
    let e0 = [bar, 0.635];
    let e1 = [2.54, 2.54];
    let mut g = vec![
        Gfx::Circle { c: [0.635, 0.0], r: 3.175, fill: Fill::None },
        thick(&[[bar, -1.905], [bar, 1.905]], 0.508),
        line(&[[bar, -0.635], [2.54, -2.54]]),
        line(&[e0, e1]),
    ];
    // Arrow on the emitter: out of the base for NPN, into it for PNP.
    let (tip, from) = if pnp { (lerp(e1, e0, 0.75), lerp(e1, e0, 0.35)) } else { (lerp(e0, e1, 0.85), lerp(e0, e1, 0.45)) };
    g.push(arrow_head(from, tip, 1.0));
    s.units = vec![g];
    s.pins = vec![
        pin("1", "B", [-5.08, 0.0], Dir::Left, 5.08 + bar, PinType::Input),
        pin("3", "C", [2.54, -5.08], Dir::Up, 2.54, PinType::Passive),
        pin("2", "E", [2.54, 5.08], Dir::Down, 2.54, PinType::Passive),
    ];
    s
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// A filled arrow head at `tip`, pointing away from `from`.
fn arrow_head(from: [f64; 2], tip: [f64; 2], size: f64) -> Gfx {
    let d = [tip[0] - from[0], tip[1] - from[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    let u = [d[0] / l, d[1] / l];
    let n = [-u[1], u[0]];
    let back = [tip[0] - u[0] * size, tip[1] - u[1] * size];
    poly(&[tip, [back[0] + n[0] * size * 0.4, back[1] + n[1] * size * 0.4], [back[0] - n[0] * size * 0.4, back[1] - n[1] * size * 0.4]], Fill::Outline)
}

/// Enhancement MOSFET in SOT-23: 1 gate, 2 source, 3 drain (2N7002/AO3400 pinout).
fn mosfet(p: bool) -> SymbolDef {
    let mut s = base(if p { "Q_PMOS" } else { "Q_NMOS" }, "Q", "Transistor", if p { "P-channel MOSFET, SOT-23 (1 G, 2 S, 3 D)" } else { "N-channel MOSFET, SOT-23 (1 G, 2 S, 3 D)" });
    s.value = if p { "AO3401A".into() } else { "2N7002".into() };
    s.footprint = "SOT-23".into();
    s.keywords = vec!["mosfet".into(), "fet".into(), if p { "pmos" } else { "nmos" }.into()];
    let ch = -0.508;
    let mut g = vec![
        Gfx::Circle { c: [0.635, 0.0], r: 3.175, fill: Fill::None },
        thick(&[[-1.27, -1.905], [-1.27, 1.905]], 0.254),
        thick(&[[ch, -2.032], [ch, -1.016]], 0.381),
        thick(&[[ch, -0.508], [ch, 0.508]], 0.381),
        thick(&[[ch, 1.016], [ch, 2.032]], 0.381),
        line(&[[ch, -1.524], [2.54, -1.524], [2.54, -2.54]]),
        line(&[[ch, 1.524], [2.54, 1.524], [2.54, 2.54]]),
        line(&[[ch, 0.0], [2.54, 0.0], [2.54, 1.524]]),
    ];
    if p {
        g.push(arrow_head([ch + 0.2, 0.0], [ch + 1.5, 0.0], 1.0));
    } else {
        g.push(arrow_head([ch + 1.5, 0.0], [ch + 0.2, 0.0], 1.0));
    }
    s.units = vec![g];
    s.pins = vec![
        pin("1", "G", [-5.08, 0.0], Dir::Left, 3.81, PinType::Input),
        pin("3", "D", [2.54, -5.08], Dir::Up, 2.54, PinType::Passive),
        pin("2", "S", [2.54, 5.08], Dir::Down, 2.54, PinType::Passive),
    ];
    s
}

fn switch_push() -> SymbolDef {
    let mut s = base("SW_Push", "SW", "Misc", "Push button, normally open (tactile switch: legs 1–1 and 2–2 joined)");
    s.value = "SW_Push".into();
    s.footprint = "SW_Tactile_6x6".into();
    s.keywords = vec!["switch".into(), "button".into(), "tactile".into(), "push".into()];
    s.units = vec![vec![
        Gfx::Circle { c: [-2.032, 0.0], r: 0.508, fill: Fill::None },
        Gfx::Circle { c: [2.032, 0.0], r: 0.508, fill: Fill::None },
        line(&[[-3.048, -1.27], [3.048, -1.27]]),
        line(&[[0.0, -1.27], [0.0, -3.048]]),
        line(&[[-1.016, -3.048], [1.016, -3.048]]),
    ]];
    s.pins = vec![pin("1", "1", [-5.08, 0.0], Dir::Left, 2.54, PinType::Passive), pin("2", "2", [5.08, 0.0], Dir::Right, 2.54, PinType::Passive)];
    s
}

/// One-row connector: pins on the left, 2.54 mm apart, pin 1 on top.
fn conn_1row(n: usize, name: String, footprint: String, desc: String) -> SymbolDef {
    let mut s = base(&name, "J", "Connector", &desc);
    s.value = format!("Conn_01x{n:02}");
    s.footprint = footprint;
    s.show_pin_numbers = true;
    s.keywords = vec!["connector".into(), "header".into(), "jst".into()];
    let y0 = -(n as f64 - 1.0) * G / 2.0;
    let top = y0 - 1.27;
    let bottom = y0 + (n as f64 - 1.0) * G + 1.27;
    let mut g = vec![rect([-1.27, top], [1.27, bottom], Fill::Body)];
    for i in 0..n {
        let y = y0 + i as f64 * G;
        g.push(rect([-1.27, y - 0.127], [-0.254, y + 0.127], Fill::Outline));
        s.pins.push(pin(&(i + 1).to_string(), &format!("Pin_{}", i + 1), [-5.08, y], Dir::Left, 3.81, PinType::Passive));
    }
    s.units = vec![g];
    s
}

/// Two-row header: odd pins left, even right, as 2.54 mm headers number them.
fn conn_2row(n: usize) -> SymbolDef {
    let mut s = base(&format!("Conn_02x{n:02}"), "J", "Connector", &format!("Pin header 2×{n}, 2.54 mm (odd pins left, even right)"));
    s.value = format!("Conn_02x{n:02}");
    s.footprint = format!("PinHeader_2x{n:02}_2.54");
    s.show_pin_numbers = true;
    s.keywords = vec!["connector".into(), "header".into()];
    let y0 = -(n as f64 - 1.0) * G / 2.0;
    let mut g = vec![rect([-1.27, y0 - 1.27], [1.27, y0 + (n as f64 - 1.0) * G + 1.27], Fill::Body)];
    for i in 0..n {
        let y = y0 + i as f64 * G;
        g.push(rect([-1.27, y - 0.127], [-0.254, y + 0.127], Fill::Outline));
        g.push(rect([0.254, y - 0.127], [1.27, y + 0.127], Fill::Outline));
        s.pins.push(pin(&(2 * i + 1).to_string(), &format!("Pin_{}", 2 * i + 1), [-5.08, y], Dir::Left, 3.81, PinType::Passive));
        s.pins.push(pin(&(2 * i + 2).to_string(), &format!("Pin_{}", 2 * i + 2), [5.08, y], Dir::Right, 3.81, PinType::Passive));
    }
    s.units = vec![g];
    s
}

/// One entry of a pin table for a box symbol (`ic`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PinSpec {
    /// Pin number; "" leaves a gap (to group pins).
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub etype: PinType,
    /// left, right, up (top) or down (bottom); worked out from the type when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Dir>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pads: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl PinSpec {
    fn new(number: &str, name: &str, etype: PinType, side: Dir) -> PinSpec {
        PinSpec { number: number.into(), name: name.into(), etype, side: Some(side), pads: vec![], hidden: false }
    }
    fn gap(side: Dir) -> PinSpec {
        PinSpec { number: String::new(), name: String::new(), etype: PinType::Passive, side: Some(side), pads: vec![], hidden: false }
    }
}

/// Width of text in Helvetica/Arial, as a fraction of the font size.
pub fn text_width(s: &str, size: f64) -> f64 {
    const W: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
    ];
    s.chars().map(|c| if (' '..='~').contains(&c) { W[c as usize - 32] as f64 } else { 600.0 }).sum::<f64>() / 1000.0 * size
}

/// Font size of pin names inside a box (mm, em).
pub const PIN_NAME_SIZE: f64 = 1.5;

/// A box symbol from a pin table: inputs left, outputs right, power in on
/// top (ground at the bottom), sized to the pin names.
pub fn ic(name: &str, pins: &[PinSpec]) -> SymbolDef {
    let mut s = base(name, "U", "IC", "Integrated circuit (box from its pin table)");
    s.value = name.into();
    s.show_pin_numbers = true;
    s.show_pin_names = true;
    let mut sides: [Vec<&PinSpec>; 4] = [vec![], vec![], vec![], vec![]]; // left, right, up, down
    for p in pins {
        let side = p.side.unwrap_or_else(|| {
            let n = p.name.to_ascii_uppercase();
            match p.etype {
                PinType::PowerIn | PinType::PowerOut if n.contains("GND") || n.starts_with("VSS") || n == "V-" || n == "VEE" => Dir::Down,
                PinType::PowerIn => Dir::Up,
                PinType::Output | PinType::PowerOut | PinType::OpenCollector | PinType::TriState => Dir::Right,
                PinType::Input => Dir::Left,
                _ => Dir::Left,
            }
        });
        let i = match side {
            Dir::Left => 0,
            Dir::Right => 1,
            Dir::Up => 2,
            Dir::Down => 3,
        };
        if p.hidden {
            continue;
        }
        sides[i].push(p);
    }
    // Balance unsided passives between left and right.
    if pins.iter().all(|p| p.side.is_none()) && sides[0].len() > sides[1].len() + 1 {
        let move_n = (sides[0].len() - sides[1].len()) / 2;
        let moved: Vec<&PinSpec> = sides[0].split_off(sides[0].len() - move_n);
        sides[1].extend(moved);
    }
    let rows = sides[0].len().max(sides[1].len()).max(1);
    let cols = sides[2].len().max(sides[3].len());
    let wname = |v: &Vec<&PinSpec>| v.iter().map(|p| text_width(&p.name, PIN_NAME_SIZE)).fold(0.0f64, f64::max);
    let need_w = wname(&sides[0]) + wname(&sides[1]) + 3.0 * G;
    let need_w = need_w.max(cols as f64 * G + G).max(4.0 * G);
    let half_w = (need_w / 2.0 / G).ceil() * G;
    // Top/bottom names stand vertically inside the box: leave room for them.
    let tb = sides[2].iter().chain(sides[3].iter()).map(|p| text_width(&p.name, PIN_NAME_SIZE)).fold(0.0f64, f64::max);
    let need_h = ((rows as f64 + 1.0) * G).max(if cols > 0 { 2.0 * tb + 2.0 * G } else { 0.0 });
    let half_h = (need_h / 2.0 / 1.27 - 1e-9).ceil() * 1.27;
    let mut out = Vec::new();
    let mut place = |list: &Vec<&PinSpec>, side: Dir| {
        let n = list.len();
        for (i, p) in list.iter().enumerate() {
            if p.number.is_empty() {
                continue;
            }
            let row = (i as f64 - (rows as f64 - 1.0) / 2.0) * G;
            let at = match side {
                Dir::Left => [-half_w - G, row],
                Dir::Right => [half_w + G, row],
                Dir::Up => [(i as f64 - (n as f64 - 1.0) / 2.0) * G, -half_h - G],
                Dir::Down => [(i as f64 - (n as f64 - 1.0) / 2.0) * G, half_h + G],
            };
            let at = [snap(at[0]), snap(at[1])];
            out.push(PinDef { number: p.number.clone(), name: p.name.clone(), at, side, length: G, etype: p.etype, hidden: false, unit: 0, pads: p.pads.clone(), shape: PinShape::Line });
        }
    };
    place(&sides[0], Dir::Left);
    place(&sides[1], Dir::Right);
    place(&sides[2], Dir::Up);
    place(&sides[3], Dir::Down);
    for p in pins.iter().filter(|p| p.hidden && !p.number.is_empty()) {
        out.push(PinDef { number: p.number.clone(), name: p.name.clone(), at: [0.0, 0.0], side: Dir::Left, length: 0.0, etype: p.etype, hidden: true, unit: 0, pads: p.pads.clone(), shape: PinShape::Line });
    }
    s.pins = out;
    s.units = vec![vec![rect([-half_w, -half_h], [half_w, half_h], Fill::Body)]];
    s
}

fn snap(v: f64) -> f64 {
    (v / 1.27).round() * 1.27
}

fn esp32() -> SymbolDef {
    use Dir::*;
    use PinType::*;
    let mut t = vec![PinSpec::new("3", "EN", Input, Left), PinSpec::new("4", "SENSOR_VP", Input, Left), PinSpec::new("5", "SENSOR_VN", Input, Left), PinSpec::new("6", "IO34", Input, Left), PinSpec::new("7", "IO35", Input, Left), PinSpec::gap(Left)];
    for (n, name) in [("8", "IO32"), ("9", "IO33"), ("10", "IO25"), ("11", "IO26"), ("12", "IO27"), ("13", "IO14"), ("14", "IO12"), ("16", "IO13"), ("23", "IO15")] {
        t.push(PinSpec::new(n, name, Bidirectional, Left));
    }
    for (n, name) in [("25", "IO0"), ("24", "IO2"), ("26", "IO4"), ("29", "IO5"), ("27", "IO16"), ("28", "IO17"), ("30", "IO18"), ("31", "IO19"), ("33", "IO21"), ("36", "IO22"), ("37", "IO23")] {
        t.push(PinSpec::new(n, name, Bidirectional, Right));
    }
    t.push(PinSpec::gap(Right));
    t.push(PinSpec::new("34", "RXD0", Bidirectional, Right));
    t.push(PinSpec::new("35", "TXD0", Bidirectional, Right));
    t.push(PinSpec::new("2", "3V3", PowerIn, Up));
    t.push(PinSpec { pads: vec!["1".into(), "15".into(), "38".into(), "39".into()], ..PinSpec::new("1", "GND", PowerIn, Down) });
    for n in ["17", "18", "19", "20", "21", "22", "32"] {
        t.push(PinSpec { hidden: true, ..PinSpec::new(n, "NC", NoConnect, Right) });
    }
    let mut s = ic("ESP32-WROOM-32E", &t);
    s.prefix = "U".into();
    s.category = "Module".into();
    s.description = "Espressif ESP32-WROOM-32E Wi-Fi + Bluetooth module (pins 17–22 and 32 not connected; GND on 1, 15, 38 and the 39 pad)".into();
    s.value = "ESP32-WROOM-32E".into();
    s.footprint = "ESP32-WROOM-32".into();
    s.mpn = Some("ESP32-WROOM-32E-N4".into());
    s.manufacturer = Some("Espressif".into());
    s.lcsc = Some("C701341".into());
    s.keywords = vec!["esp32".into(), "wifi".into(), "bluetooth".into(), "module".into(), "mcu".into()];
    s
}

fn usb_c() -> SymbolDef {
    let mut s = base("USB_C_16P", "J", "Connector", "USB Type-C receptacle, 16 pins (USB 2.0): each signal lands on both rows' pads");
    s.value = "USB-C".into();
    s.footprint = "USB-C_16P".into();
    s.mpn = Some("TYPE-C-31-M-12".into());
    s.manufacturer = Some("HRO".into());
    s.lcsc = Some("C165948".into());
    s.show_pin_numbers = true;
    s.show_pin_names = true;
    s.keywords = vec!["usb".into(), "type-c".into(), "connector".into()];
    let x = 10.16;
    let p = |num: &str, name: &str, y: f64, pads: &[&str]| PinDef { number: num.into(), name: name.into(), at: [x + 5.08, y], side: Dir::Right, length: 5.08, etype: PinType::Passive, hidden: false, unit: 0, pads: pads.iter().map(|s| s.to_string()).collect(), shape: PinShape::Line };
    s.pins = vec![
        p("A4", "VBUS", -10.16, &["A4", "A9", "B4", "B9"]),
        p("A5", "CC1", -5.08, &["A5"]),
        p("B5", "CC2", -2.54, &["B5"]),
        p("A6", "D+", 2.54, &["A6", "B6"]),
        p("A7", "D-", 5.08, &["A7", "B7"]),
        p("A8", "SBU1", 10.16, &["A8"]),
        p("B8", "SBU2", 12.7, &["B8"]),
        PinDef { number: "A1".into(), name: "GND".into(), at: [0.0, 20.32], side: Dir::Down, length: 5.08, etype: PinType::Passive, hidden: false, unit: 0, pads: vec!["A1".into(), "A12".into(), "B1".into(), "B12".into()], shape: PinShape::Line },
        PinDef { number: "S1".into(), name: "SHIELD".into(), at: [-7.62, 20.32], side: Dir::Down, length: 5.08, etype: PinType::Passive, hidden: false, unit: 0, pads: vec!["S1".into()], shape: PinShape::Line },
    ];
    let mut g = vec![rect([-x, -15.24], [x, 15.24], Fill::Body)];
    // The receptacle, drawn: a rounded slot with its tongue.
    g.push(rect([-8.89, -6.35], [-6.35, 6.35], Fill::None));
    g.push(rect([-7.874, -4.064], [-7.366, 4.064], Fill::Outline));
    s.units = vec![g];
    s
}

fn regulator(name: &str, five: bool) -> SymbolDef {
    let mut s = base(name, "U", "Power IC", "");
    s.show_pin_numbers = true;
    s.show_pin_names = true;
    s.keywords = vec!["regulator".into(), "ldo".into(), "3v3".into(), "power".into()];
    let hw = 7.62;
    if five {
        // ME6211-style SOT-23-5: 1 VIN, 2 GND, 3 CE, 4 NC, 5 VOUT.
        s.description = "Low-dropout regulator, SOT-23-5 (1 VIN, 2 GND, 3 EN/CE, 4 NC, 5 VOUT), e.g. ME6211C33, XC6220".into();
        s.value = "ME6211C33".into();
        s.footprint = "SOT-23-5".into();
        s.mpn = Some("ME6211C33M5G-N".into());
        s.manufacturer = Some("MICRONE".into());
        s.lcsc = Some("C82942".into());
        s.pins = vec![
            pin("1", "VIN", [-hw - G, -2.54], Dir::Left, G, PinType::PowerIn),
            pin("3", "EN", [-hw - G, 2.54], Dir::Left, G, PinType::Input),
            pin("5", "VOUT", [hw + G, -2.54], Dir::Right, G, PinType::PowerOut),
            pin("4", "NC", [hw + G, 2.54], Dir::Right, G, PinType::NoConnect),
            pin("2", "GND", [0.0, 7.62], Dir::Down, G, PinType::PowerIn),
        ];
        s.units = vec![vec![rect([-hw, -5.08], [hw, 5.08], Fill::Body)]];
    } else {
        // AMS1117-style SOT-223: 1 GND/ADJ, 2 VOUT (tab 4), 3 VIN.
        s.description = "Low-dropout regulator, SOT-223 (1 GND/ADJ, 2 VOUT with the tab, 3 VIN), e.g. AMS1117-3.3".into();
        s.value = "AMS1117-3.3".into();
        s.footprint = "SOT-223".into();
        s.mpn = Some("AMS1117-3.3".into());
        s.manufacturer = Some("AMS".into());
        s.lcsc = Some("C6186".into());
        s.pins = vec![
            pin("3", "VI", [-hw - G, 0.0], Dir::Left, G, PinType::PowerIn),
            PinDef { pads: vec!["2".into(), "4".into()], ..pin("2", "VO", [hw + G, 0.0], Dir::Right, G, PinType::PowerOut) },
            pin("1", "GND", [0.0, 6.35], Dir::Down, G, PinType::PowerIn),
        ];
        s.units = vec![vec![rect([-hw, -3.81], [hw, 3.81], Fill::Body)]];
    }
    s
}

/// Dual op-amp (LM358 pinout, SOIC-8): units A and B, and the supply as unit C.
fn opamp_dual() -> SymbolDef {
    let mut s = base("OpAmp_Dual", "U", "IC", "Dual operational amplifier (LM358 pinout, SOIC-8): unit A 1/2/3, unit B 7/6/5, unit C supply 8/4");
    s.value = "LM358".into();
    s.footprint = "SOIC-8".into();
    s.show_pin_numbers = true;
    s.keywords = vec!["opamp".into(), "op-amp".into(), "amplifier".into(), "lm358".into()];
    let tri = || vec![poly(&[[-5.08, -5.08], [5.08, 0.0], [-5.08, 5.08]], Fill::Body), line(&[[-4.318, -2.54], [-3.302, -2.54]]), line(&[[-3.81, -3.048], [-3.81, -2.032]]), line(&[[-4.318, 2.54], [-3.302, 2.54]])];
    let mut pins = Vec::new();
    for (unit, (out, minus, plus)) in [(1u32, ("1", "2", "3")), (2, ("7", "6", "5"))] {
        pins.push(PinDef { unit, ..pin(plus, "+", [-7.62, -2.54], Dir::Left, G, PinType::Input) });
        pins.push(PinDef { unit, ..pin(minus, "-", [-7.62, 2.54], Dir::Left, G, PinType::Input) });
        pins.push(PinDef { unit, ..pin(out, "~", [7.62, 0.0], Dir::Right, G, PinType::Output) });
    }
    pins.push(PinDef { unit: 3, ..pin("8", "V+", [0.0, -7.62], Dir::Up, 3.81, PinType::PowerIn) });
    pins.push(PinDef { unit: 3, ..pin("4", "V-", [0.0, 7.62], Dir::Down, 3.81, PinType::PowerIn) });
    s.pins = pins;
    s.units = vec![tri(), tri(), vec![rect([-2.54, -3.81], [2.54, 3.81], Fill::Body), Gfx::Text { at: [0.0, -2.2], text: "V+".into(), size: 1.27 }, Gfx::Text { at: [0.0, 2.2], text: "V-".into(), size: 1.27 }]];
    s
}

/// Single op-amp, SOT-23-5 (1 OUT, 2 V−, 3 +IN, 4 −IN, 5 V+), supply pins on the triangle.
fn opamp() -> SymbolDef {
    let mut s = base("OpAmp", "U", "IC", "Operational amplifier, SOT-23-5 (1 OUT, 2 V−, 3 +, 4 −, 5 V+), e.g. LMV321, MCP6001");
    s.value = "MCP6001".into();
    s.footprint = "SOT-23-5".into();
    s.show_pin_numbers = true;
    s.keywords = vec!["opamp".into(), "op-amp".into(), "amplifier".into()];
    s.units = vec![vec![poly(&[[-5.08, -5.08], [5.08, 0.0], [-5.08, 5.08]], Fill::Body), line(&[[-4.318, -2.54], [-3.302, -2.54]]), line(&[[-3.81, -3.048], [-3.81, -2.032]]), line(&[[-4.318, 2.54], [-3.302, 2.54]])]];
    s.pins = vec![
        pin("3", "+", [-7.62, -2.54], Dir::Left, G, PinType::Input),
        pin("4", "-", [-7.62, 2.54], Dir::Left, G, PinType::Input),
        pin("1", "~", [7.62, 0.0], Dir::Right, G, PinType::Output),
        pin("5", "V+", [-2.54, -7.62], Dir::Up, 3.81, PinType::PowerIn),
        pin("2", "V-", [-2.54, 7.62], Dir::Down, 3.81, PinType::PowerIn),
    ];
    s
}

fn pwr_flag() -> SymbolDef {
    let mut s = base("PWR_FLAG", "#FLG", "Power", "Power flag: marks a net as driven (a supply coming in through a connector), for the electrical-rules check; not a part");
    s.value = "PWR_FLAG".into();
    s.virtual_part = true;
    s.keywords = vec!["power".into(), "flag".into(), "erc".into()];
    s.units = vec![vec![line(&[[0.0, 0.0], [0.0, -1.27]]), poly(&[[0.0, -1.27], [-1.016, -1.905], [0.0, -2.54], [1.016, -1.905]], Fill::None)]];
    s.pins = vec![pin("1", "pwr", [0.0, 0.0], Dir::Down, 0.0, PinType::PowerOut)];
    s
}

/// Every built-in symbol name (families written with a sample count).
pub fn names() -> Vec<String> {
    let mut v: Vec<String> = [
        "R", "C", "C_Polarized", "L", "Fuse", "Crystal", "D", "D_Schottky", "D_Zener", "LED", "Q_NPN", "Q_PNP", "Q_NMOS", "Q_PMOS", "SW_Push", "USB_C_16P", "LDO_SOT23-5", "LDO_SOT223", "ESP32-WROOM-32E", "OpAmp", "OpAmp_Dual", "PWR_FLAG",
    ]
    .map(String::from)
    .to_vec();
    for n in [1, 2, 3, 4, 5, 6, 8, 10] {
        v.push(format!("Conn_01x{n:02}"));
    }
    for n in [3, 4, 5, 10] {
        v.push(format!("Conn_02x{n:02}"));
    }
    for n in 2..=6 {
        v.push(format!("JST_PH_{n}"));
    }
    for n in 2..=6 {
        v.push(format!("JST_XH_{n}"));
    }
    v
}

/// A built-in symbol by name. Families are parametric (`Conn_01x07`,
/// `Conn_02x08`, `JST_PH_9`, `JST_XH_12`).
pub fn get(name: &str) -> Option<SymbolDef> {
    Some(match name {
        "R" => resistor(),
        "C" => capacitor(),
        "C_Polarized" => capacitor_polarized(),
        "L" => inductor(),
        "Fuse" => fuse(),
        "Crystal" => crystal(),
        "D" => diode("d"),
        "D_Schottky" => diode("schottky"),
        "D_Zener" => diode("zener"),
        "LED" => diode("led"),
        "Q_NPN" => bjt(false),
        "Q_PNP" => bjt(true),
        "Q_NMOS" => mosfet(false),
        "Q_PMOS" => mosfet(true),
        "SW_Push" => switch_push(),
        "USB_C_16P" => usb_c(),
        "LDO_SOT23-5" => regulator("LDO_SOT23-5", true),
        "LDO_SOT223" => regulator("LDO_SOT223", false),
        "ESP32-WROOM-32E" => esp32(),
        "OpAmp" => opamp(),
        "OpAmp_Dual" => opamp_dual(),
        "PWR_FLAG" => pwr_flag(),
        _ => {
            if let Some(rest) = name.strip_prefix("Conn_01x") {
                let n: usize = rest.parse().ok().filter(|n| (1..=40).contains(n))?;
                return Some(conn_1row(n, name.into(), format!("PinHeader_1x{n:02}_2.54"), format!("Connector, one row of {n} (2.54 mm header by default)")));
            }
            if let Some(rest) = name.strip_prefix("Conn_02x") {
                let n: usize = rest.parse().ok().filter(|n| (1..=40).contains(n))?;
                return Some(conn_2row(n));
            }
            for (fam, pitch) in [("PH", "2.0"), ("XH", "2.5")] {
                if let Some(rest) = name.strip_prefix(&format!("JST_{fam}_")) {
                    let n: usize = rest.parse().ok().filter(|n| (1..=16).contains(n))?;
                    let mut s = conn_1row(n, name.into(), format!("JST_{fam}_{n}"), format!("JST {fam} {n}-pin, {pitch} mm, top entry"));
                    s.value = format!("{fam} {n}-pin");
                    s.mpn = Some(if fam == "PH" { format!("B{n}B-PH-K-S") } else { format!("B{n}B-XH-A") });
                    s.manufacturer = Some("JST".into());
                    return Some(s);
                }
            }
            return None;
        }
    })
}

/// A symbol from an instance's own pin table (a part the library lacks).
pub fn custom(name: &str, pins: &[PinSpec]) -> SymbolDef {
    ic(name, pins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_symbol_builds_on_grid_and_maps_to_its_footprint() {
        for n in names() {
            let s = get(&n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(s.name, n);
            for p in &s.pins {
                for v in p.at {
                    assert!(((v / 1.27).round() * 1.27 - v).abs() < 1e-6, "{n} pin {} at {:?} is off the 1.27 mm grid", p.number, p.at);
                }
            }
            if !s.footprint.is_empty() {
                let f = crate::footprints::get(&s.footprint).unwrap_or_else(|| panic!("{n}: footprint {}", s.footprint));
                for p in &s.pins {
                    for pad in p.pads() {
                        assert!(f.pads.iter().any(|x| x.number == pad), "{n}: pin {} → pad {pad} not on {}", p.number, f.name);
                    }
                }
            }
        }
    }
}
