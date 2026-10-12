//! Where to get the components: BOM lines grouped by MPN, search links for
//! each distributor built from the MPN (nothing is fetched: the person
//! follows a link when they choose), LCSC part numbers with JLCPCB's
//! Basic/Extended class for common parts, and the supplier and price the
//! person recorded.
//!
//! No distributor offers a free, keyless pricing API (checked 2026-10-11:
//! LCSC, JLCPCB, Digi-Key, Mouser, Nexar/Octopart, element14, TME all need an
//! account key), so live prices wait for the person's own key; the engine
//! keeps a local settings slot for it and never sends anything on its own.
//!
//! `buy_lines` is the clean hand-off to a project-wide BOM: one line per
//! distinct bought item, quantities per board and for the order.

use crate::board::BoardView;
use crate::footprints::PadKind;
use crate::{Board, Choice};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct Link {
    pub name: &'static str,
    pub url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BomLine {
    /// The MPN, or "value|footprint" for parts without one.
    pub key: String,
    pub references: Vec<String>,
    /// Per board.
    pub quantity: usize,
    pub value: String,
    pub footprint: String,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    /// "Basic" or "Extended" at JLCPCB, where known.
    pub lcsc_class: Option<&'static str>,
    pub description: String,
    pub dnp: bool,
    pub tht: bool,
    pub kind: String,
    pub choice: Option<Choice>,
    pub links: Vec<Link>,
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Search links for an MPN (and the LCSC/JLCPCB pages for a C-number).
/// LCSC, JLCPCB, Mouser and Farnell patterns answered when checked; the
/// others are their documented search forms (they block scripted requests).
pub fn links(mpn: Option<&str>, lcsc: Option<&str>) -> Vec<Link> {
    let mut v = Vec::new();
    if let Some(c) = lcsc {
        v.push(Link { name: "LCSC", url: format!("https://www.lcsc.com/product-detail/{}.html", enc(c)) });
        v.push(Link { name: "JLCPCB parts", url: format!("https://jlcpcb.com/partdetail/{}", enc(c)) });
    }
    if let Some(m) = mpn {
        let q = enc(m);
        if lcsc.is_none() {
            v.push(Link { name: "LCSC", url: format!("https://www.lcsc.com/search?q={q}") });
        }
        v.push(Link { name: "Digi-Key", url: format!("https://www.digikey.com/en/products/result?keywords={q}") });
        v.push(Link { name: "Mouser", url: format!("https://www.mouser.com/c/?q={q}") });
        v.push(Link { name: "Farnell", url: format!("https://www.farnell.com/search?st={q}") });
        v.push(Link { name: "Arrow", url: format!("https://www.arrow.com/en/products/search?q={q}") });
        v.push(Link { name: "TME", url: format!("https://www.tme.eu/en/katalog/?queryPhrase={q}") });
        v.push(Link { name: "Octopart", url: format!("https://octopart.com/search?q={q}") });
    }
    v
}

/// A part LCSC stocks, with its JLCPCB class (verified on jlcpcb.com/partdetail, 2026-10-11).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Known {
    pub mpn: &'static str,
    pub manufacturer: &'static str,
    pub lcsc: &'static str,
    pub class: &'static str,
    pub value: &'static str,
    pub footprint: &'static str,
    pub description: &'static str,
}

pub const KNOWN: &[Known] = &[
    Known { mpn: "0603WAF1002T5E", manufacturer: "UNI-ROYAL", lcsc: "C25804", class: "Basic", value: "10k", footprint: "R_0603", description: "Resistor 10 kΩ ±1% 1/10 W 0603" },
    Known { mpn: "0603WAF5101T5E", manufacturer: "UNI-ROYAL", lcsc: "C23186", class: "Basic", value: "5.1k", footprint: "R_0603", description: "Resistor 5.1 kΩ ±1% 0603" },
    Known { mpn: "0603WAF1001T5E", manufacturer: "UNI-ROYAL", lcsc: "C21190", class: "Basic", value: "1k", footprint: "R_0603", description: "Resistor 1 kΩ ±1% 0603" },
    Known { mpn: "0603WAF3300T5E", manufacturer: "UNI-ROYAL", lcsc: "C23138", class: "Basic", value: "330", footprint: "R_0603", description: "Resistor 330 Ω ±1% 0603" },
    Known { mpn: "CC0603KRX7R9BB104", manufacturer: "YAGEO", lcsc: "C14663", class: "Basic", value: "100nF", footprint: "C_0603", description: "Capacitor 100 nF 50 V X7R 0603" },
    Known { mpn: "CL10A105KB8NNNC", manufacturer: "Samsung", lcsc: "C15849", class: "Basic", value: "1uF", footprint: "C_0603", description: "Capacitor 1 µF 50 V X5R 0603" },
    Known { mpn: "CL21A106KAYNNNE", manufacturer: "Samsung", lcsc: "C15850", class: "Basic", value: "10uF", footprint: "C_0805", description: "Capacitor 10 µF 25 V X5R 0805" },
    Known { mpn: "KT-0603R", manufacturer: "Hubei KENTO", lcsc: "C2286", class: "Basic", value: "Red", footprint: "LED_0603", description: "LED red 0603" },
    Known { mpn: "KT-0603W", manufacturer: "Hubei KENTO", lcsc: "C2290", class: "Basic", value: "White", footprint: "LED_0603", description: "LED white 0603" },
    Known { mpn: "KT-0805G", manufacturer: "Hubei KENTO", lcsc: "C2297", class: "Basic", value: "Green", footprint: "LED_0805", description: "LED green 0805" },
    Known { mpn: "KT-0603G", manufacturer: "Hubei KENTO", lcsc: "C12624", class: "Extended", value: "Green", footprint: "LED_0603", description: "LED green 0603" },
    Known { mpn: "AMS1117-3.3", manufacturer: "AMS", lcsc: "C6186", class: "Basic", value: "3.3V LDO", footprint: "SOT-223", description: "LDO 3.3 V 1 A" },
    Known { mpn: "XC6206P332MR-G", manufacturer: "Torex", lcsc: "C5446", class: "Basic", value: "3.3V LDO", footprint: "SOT-23", description: "LDO 3.3 V 200 mA" },
    Known { mpn: "ME6211C33M5G-N", manufacturer: "MICRONE", lcsc: "C82942", class: "Extended", value: "3.3V LDO", footprint: "SOT-23-5", description: "LDO 3.3 V 500 mA" },
    Known { mpn: "CH340C", manufacturer: "WCH", lcsc: "C84681", class: "Extended", value: "CH340C", footprint: "SOIC-16", description: "USB-UART bridge" },
    Known { mpn: "CH340N", manufacturer: "WCH", lcsc: "C2977777", class: "Extended", value: "CH340N", footprint: "SOIC-8", description: "USB-UART bridge" },
    Known { mpn: "ESP32-WROOM-32E-N4", manufacturer: "Espressif", lcsc: "C701341", class: "Extended", value: "ESP32-WROOM-32E", footprint: "ESP32-WROOM-32", description: "Wi-Fi + Bluetooth module, 4 MB flash" },
    Known { mpn: "TYPE-C-31-M-12", manufacturer: "HRO", lcsc: "C165948", class: "Extended", value: "USB-C", footprint: "USB-C_16P", description: "USB Type-C receptacle 16-pin" },
    Known { mpn: "S2B-PH-K-S", manufacturer: "JST", lcsc: "C173752", class: "Extended", value: "PH 2-pin", footprint: "JST_PH_2_side", description: "JST PH 2-pin side entry" },
    Known { mpn: "B2B-XH-A", manufacturer: "JST", lcsc: "C158012", class: "Extended", value: "XH 2-pin", footprint: "JST_XH_2", description: "JST XH 2-pin top entry" },
    Known { mpn: "TS-1187A-B-A-B", manufacturer: "XKB", lcsc: "C318884", class: "Basic", value: "Tactile", footprint: "SW_SMD_5.1x5.1", description: "Tactile switch 5.1 × 5.1 mm SMD" },
    Known { mpn: "2N7002", manufacturer: "CJ", lcsc: "C8545", class: "Basic", value: "2N7002", footprint: "SOT-23", description: "N-MOSFET 60 V 115 mA" },
    Known { mpn: "SS34", manufacturer: "MDD", lcsc: "C8678", class: "Basic", value: "SS34", footprint: "SMA", description: "Schottky diode 40 V 3 A" },
    Known { mpn: "PCA9685PW,118", manufacturer: "NXP", lcsc: "C2678753", class: "Extended", value: "PCA9685", footprint: "TSSOP-28", description: "16-channel 12-bit PWM driver, I²C" },
    Known { mpn: "STM32G031K8U6", manufacturer: "ST", lcsc: "C432207", class: "Extended", value: "STM32G031K8U6", footprint: "QFN-32_5x5", description: "Arm Cortex-M0+ MCU, 64 KB flash, UFQFPN-32" },
];

pub fn known(mpn: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.mpn.eq_ignore_ascii_case(mpn.trim()))
}

pub fn known_lcsc(lcsc: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.lcsc.eq_ignore_ascii_case(lcsc.trim()))
}

/// The BOM key of a component: its MPN, else value and footprint.
pub fn key(mpn: Option<&str>, value: &str, footprint: &str) -> String {
    match mpn {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => format!("{}|{}", value.trim(), footprint),
    }
}

/// BOM lines, one per distinct part (merged by MPN), references in natural order.
pub fn bom_lines(view: &BoardView, board: &Board) -> Vec<BomLine> {
    let mut by: BTreeMap<String, BomLine> = BTreeMap::new();
    for c in &view.components {
        if c.kind == "hole" || c.kind == "logo" {
            continue;
        }
        let k = key(c.mpn.as_deref(), &c.value, &c.footprint);
        let tht = view.pads.iter().any(|p| p.component == c.id && p.kind == PadKind::Tht) && !view.pads.iter().any(|p| p.component == c.id && p.kind == PadKind::Smd);
        let known = c.mpn.as_deref().and_then(known).or_else(|| c.lcsc.as_deref().and_then(known_lcsc));
        let line = by.entry(format!("{}{}", if c.dnp { "~dnp~" } else { "" }, k)).or_insert_with(|| {
            let lcsc = c.lcsc.clone().or_else(|| known.map(|k| k.lcsc.to_string()));
            BomLine {
                key: k.clone(),
                references: vec![],
                quantity: 0,
                value: c.value.clone(),
                footprint: c.footprint.clone(),
                mpn: c.mpn.clone(),
                manufacturer: c.manufacturer.clone().or_else(|| known.map(|k| k.manufacturer.to_string())),
                lcsc_class: lcsc.as_deref().and_then(known_lcsc).map(|k| k.class),
                links: links(c.mpn.as_deref(), lcsc.as_deref()),
                lcsc,
                description: if c.description.is_empty() { known.map(|k| k.description.to_string()).unwrap_or_default() } else { c.description.clone() },
                dnp: c.dnp,
                tht,
                kind: c.kind.clone(),
                choice: board.sourcing.get(&k).cloned(),
            }
        });
        line.references.push(c.reference.clone());
        line.quantity += 1;
    }
    let mut lines: Vec<BomLine> = by.into_values().collect();
    for l in &mut lines {
        l.references.sort_by(|a, b| crate::natural(a, b));
    }
    lines.sort_by(|a, b| a.dnp.cmp(&b.dnp).then_with(|| crate::natural(&a.references[0], &b.references[0])));
    lines
}

/// One bought item for a project-wide BOM (the sourcing plugin's Buy lines).
#[derive(Clone, Debug, Serialize)]
pub struct BuyLine {
    /// Where it came from: "pcb:<board id>".
    pub origin: String,
    pub board: String,
    pub key: String,
    pub part_number: String,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub description: String,
    pub references: Vec<String>,
    pub per_board: usize,
    /// Per board × the board's quantity.
    pub quantity: usize,
    /// Supplier part numbers known: ("LCSC", "C25804").
    pub skus: Vec<(String, String)>,
    pub choice: Option<Choice>,
}

/// The board's components as Buy lines (populated parts only; the bare PCB
/// itself is a fab order, see `fabs`).
pub fn buy_lines(view: &BoardView, board: &Board) -> Vec<BuyLine> {
    bom_lines(view, board)
        .into_iter()
        .filter(|l| !l.dnp)
        .map(|l| BuyLine {
            origin: format!("pcb:{}", view.id),
            board: view.name.clone(),
            part_number: l.mpn.clone().unwrap_or_else(|| format!("{} {}", l.value, l.footprint).trim().to_string()),
            mpn: l.mpn.clone(),
            manufacturer: l.manufacturer.clone(),
            description: if l.description.is_empty() { format!("{} {}", l.value, l.footprint) } else { l.description.clone() },
            per_board: l.quantity,
            quantity: l.quantity * board.quantity as usize,
            skus: l.lcsc.iter().map(|c| ("LCSC".to_string(), c.clone())).collect(),
            references: l.references,
            choice: l.choice,
            key: l.key,
        })
        .collect()
}

/// Lines from several boards (or several projects' boards) merged by MPN:
/// quantities add, references keep their board.
pub fn merge(lines: &[BuyLine]) -> Vec<BuyLine> {
    let mut by: BTreeMap<String, BuyLine> = BTreeMap::new();
    for l in lines {
        let k = l.mpn.clone().map(|m| m.to_ascii_uppercase()).unwrap_or_else(|| l.key.clone());
        match by.get_mut(&k) {
            Some(m) => {
                m.quantity += l.quantity;
                m.per_board += l.per_board;
                m.references.extend(l.references.iter().map(|r| format!("{}:{r}", l.board)));
                for s in &l.skus {
                    if !m.skus.contains(s) {
                        m.skus.push(s.clone());
                    }
                }
                if m.choice.is_none() {
                    m.choice = l.choice.clone();
                }
                if m.origin != l.origin {
                    m.origin = "several".into();
                }
            }
            None => {
                let mut first = l.clone();
                first.references = l.references.iter().map(|r| format!("{}:{r}", l.board)).collect();
                by.insert(k, first);
            }
        }
    }
    by.into_values().collect()
}

/// Order totals from the recorded prices: (sum per currency, lines without a price).
pub fn totals(lines: &[BomLine], boards: u32) -> (BTreeMap<String, f64>, usize) {
    let mut sums = BTreeMap::new();
    let mut missing = 0;
    for l in lines.iter().filter(|l| !l.dnp) {
        match l.choice.as_ref().and_then(|c| c.unit_price.map(|p| (p, c.currency.clone().unwrap_or_else(|| "USD".into())))) {
            Some((p, cur)) => *sums.entry(cur).or_insert(0.0) += p * (l.quantity * boards as usize) as f64,
            None => missing += 1,
        }
    }
    (sums, missing)
}

/// Board components as Buy lines in the project BOM (`de_sourcing::bom`).
///
/// A board with its 3D part placed needs nothing from here: the part's
/// program declares each component as stock (MPN, manufacturer, LCSC) and the
/// bare board as a `pcb_fab` part, so the BOM walk finds them like any other
/// part. A board without a 3D part is a root of its own, and this source
/// lists its components, merged by MPN.
pub struct PcbBuySource;

fn stock_of(l: &BomLine) -> de_shape::stock::Stock {
    let lcsc = l.lcsc.iter().map(|c| de_shape::stock::Source { supplier: "LCSC".into(), sku: Some(c.clone()), url: Some(format!("https://www.lcsc.com/product-detail/{c}.html")), unit_price: None, currency: None });
    let chosen = l.choice.iter().filter_map(|c| c.supplier.clone().map(|s| de_shape::stock::Source { supplier: s, sku: c.sku.clone(), url: c.url.clone(), unit_price: c.unit_price, currency: c.currency.clone() }));
    de_shape::stock::Stock {
        standard: None,
        part_number: l.mpn.clone().unwrap_or_else(|| format!("{} {}", l.value, l.footprint).trim().to_string()),
        description: Some(if l.description.is_empty() { format!("{} {}", l.value, l.footprint) } else { l.description.clone() }),
        manufacturer: l.manufacturer.clone(),
        mpn: l.mpn.clone(),
        unit_price: l.choice.as_ref().and_then(|c| c.unit_price),
        currency: l.choice.as_ref().and_then(|c| c.currency.clone()),
        pack: None,
        mass_g: None,
        cut_length_mm: None,
        price_per_m: None,
        sources: lcsc.chain(chosen).collect(),
    }
}

/// The Buy items of one board (populated parts, per board).
pub fn buy_items(view: &BoardView, board: &Board) -> Vec<de_sourcing::BuyItem> {
    bom_lines(view, board)
        .into_iter()
        .filter(|l| !l.dnp)
        .map(|l| de_sourcing::BuyItem { name: format!("{} ({})", if l.value.is_empty() { &l.footprint } else { &l.value }, l.references.join(", ")), stock: Some(stock_of(&l)), qty: l.quantity as u32, mass_kg: None })
        .collect()
}

fn has_model(world: &de_core::bevy_ecs::world::World, board: &Board) -> bool {
    board.model.is_some_and(|m| world.resource::<de_core::IdIndex>().contains(m))
}

impl de_sourcing::BuySource for PcbBuySource {
    fn name(&self) -> &'static str {
        "pcb"
    }

    fn items(&self, world: &de_core::bevy_ecs::world::World, entity: de_core::StableId, _config: Option<&str>) -> Option<Vec<de_sourcing::BuyItem>> {
        let e = world.resource::<de_core::IdIndex>().get(entity)?;
        let board = world.get::<Board>(e)?;
        if has_model(world, board) {
            return None; // its 3D part carries the lines
        }
        let view = crate::view(world, entity)?;
        Some(buy_items(&view, board))
    }

    fn roots(&self, world: &de_core::bevy_ecs::world::World) -> Vec<de_core::StableId> {
        crate::boards(world).into_iter().filter(|(_, b)| !has_model(world, b)).map(|(id, _)| id).collect()
    }
}
