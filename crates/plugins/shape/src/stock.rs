//! What a program declares about itself besides geometry: that it is (or
//! holds) a bought-in stock item, and the connectors other parts attach to.
//!
//! - `stock({ standard, partNumber, … })` says the whole shape is one bought
//!   item (an ISO 4762 M3×12 screw, a 608 bearing, an MG996R servo); it is
//!   evaluated per configuration, so the part number follows the size.
//! - `part(name, solid, { stock: { … } })` says one part of a multi-part
//!   program is bought (its process is then "buy").
//! - `connector(name, { at, axis, x, part })` names a frame in the program's
//!   own coordinates where something attaches: a servo's output shaft, a
//!   bearing's bore, a screw hole. Assemblies join instances connector to
//!   connector.

use de_core::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A bought-in item as a buyer orders it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stock {
    /// The standard it is made to: "ISO 4762", "ISO 15", "DIN 7991".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
    /// What a buyer writes on the order: "ISO 4762 M3×12 A2", "608-2RS", "MG996R".
    pub part_number: String,
    /// One line for the BOM: "Socket head cap screw M3×12, stainless A2-70".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// Manufacturer part number, when it is a branded item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    /// Typical unit price, for estimates before quotes come in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<f64>,
    /// ISO 4217 code of `unit_price` (default USD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Sold in packs of this many (screws by 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<u32>,
    /// Catalogue mass in grams (the BOM uses it over the modelled volume).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass_g: Option<f64>,
    /// Where it can be bought.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
}

/// One place an item can be bought.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    /// "McMaster-Carr", "Misumi", "LCSC", "Digi-Key", "AliExpress"…
    pub supplier: String,
    /// The supplier's own part number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

/// A named frame in program coordinates (mm) where something attaches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Connector {
    pub name: String,
    /// The part it is on (multi-part programs); none for a single solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    pub at: [f64; 3],
    /// Unit axis: a shaft's or hole's axis, a face's outward normal.
    pub axis: [f64; 3],
    /// Unit reference direction square to `axis` (fixes the turn about it).
    pub x: [f64; 3],
}

/// Everything a program declared besides its geometry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Declared {
    /// The whole shape is this bought item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock: Option<Stock>,
    /// Bought parts of a multi-part program, by part name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parts: BTreeMap<String, Stock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connectors: Vec<Connector>,
}

impl Declared {
    pub fn is_empty(&self) -> bool {
        self.stock.is_none() && self.parts.is_empty() && self.connectors.is_empty()
    }
}
