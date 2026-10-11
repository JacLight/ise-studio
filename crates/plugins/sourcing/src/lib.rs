//! From design to quotes to suppliers.
//!
//! - [`bom`]: the bill of materials of a shape, an assembly or the whole
//!   project, configuration-aware, quantities rolled up through nested
//!   assemblies, identical items merged: **Make** lines (fabricated parts,
//!   with an estimated cost) and **Buy** lines (stock items, pack-rounded).
//! - [`rates`]: what making costs: process rates (per hour, setup, how fast)
//!   as one project-level settings entity; materials carry their own prices
//!   (`de_solid::MaterialCost`).
//! - Suppliers, requests for quote, quotes and awards are entities here, so
//!   they are journaled, undoable and synced like the design. An awarded
//!   line's price replaces the estimate everywhere the BOM is costed.
//! - [`package`]: an RFQ package per supplier (only their lines' files, a BOM
//!   CSV and a cover letter). Nothing is ever sent from here: the person
//!   sends it.

use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, Plugin, Registry, StableId, Tx, Value,
    bevy_ecs::{self, component::Component},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod bom;
pub mod package;
pub mod rates;

pub use bom::{Bom, BuyItem, BuySource, Scope};
pub use rates::{Basis, ProcessRate, Rates};

pub struct SourcingPlugin;

impl Plugin for SourcingPlugin {
    fn name(&self) -> &'static str {
        "sourcing"
    }

    fn register(&self, r: &mut Registry) {
        r.component::<Supplier>()
            .component::<Rfq>()
            .component::<Quote>()
            .component::<Award>()
            .component::<Rates>()
            .command::<CreateSupplier>()
            .command::<UpdateSupplier>()
            .command::<CreateRfq>()
            .command::<UpdateRfq>()
            .command::<CreateQuote>()
            .command::<UpdateQuote>()
            .command::<AwardQuote>()
            .command::<SetAward>()
            .command::<ClearAward>()
            .command::<rates::SetRates>();
    }
}

/// Processes a part can name (as in programs' `part(…, { process })`).
pub const PROCESSES: [&str; 12] = ["3d_print", "cnc_mill", "lathe", "cnc_router", "laser", "sheet_metal", "weld", "cast", "extrusion", "glazing", "pcb_fab", "buy"];

pub(crate) fn bad(code: ErrorCode, m: impl Into<String>) -> CommandError {
    CommandError::new(code, m)
}

fn invalid(m: impl Into<String>) -> CommandError {
    bad(ErrorCode::InvalidParameters, m)
}

fn check_currency(c: &str) -> Result<String, CommandError> {
    let c = c.trim().to_ascii_uppercase();
    if c.len() == 3 && c.chars().all(|c| c.is_ascii_alphabetic()) { Ok(c) } else { Err(invalid(format!("currency “{c}” is not a three-letter code such as USD, EUR, GBP"))) }
}

fn check_date(d: &str, what: &str) -> Result<String, CommandError> {
    let d = d.trim();
    let ok = d.len() == 10
        && d.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() })
        && (1..=12).contains(&d[5..7].parse::<u32>().unwrap_or(0))
        && (1..=31).contains(&d[8..10].parse::<u32>().unwrap_or(0));
    if ok { Ok(d.to_string()) } else { Err(invalid(format!("{what} is a date written YYYY-MM-DD, not “{d}”"))) }
}

fn opt_date(d: Option<String>, what: &str) -> Result<Option<String>, CommandError> {
    match d {
        Some(d) if !d.trim().is_empty() => check_date(&d, what).map(Some),
        _ => Ok(None),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Every entity of one component kind, with its id (in id order: creation order).
pub fn all<T: Component + Clone>(world: &de_core::bevy_ecs::world::World) -> Vec<(StableId, T)> {
    let mut out: Vec<(StableId, T)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<T>(e).map(|t| (id, t.clone()))).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

// ---- Suppliers ---------------------------------------------------------------

/// A company that makes parts or sells stock items.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Supplier {
    pub name: String,
    /// What they do: processes they make with ("cnc_mill", "laser", "3d_print"…)
    /// and/or "distributor" (sells stock items). An RFQ sends each supplier
    /// only the lines their kind covers.
    #[serde(default)]
    pub kind: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub website: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub country: String,
    /// ISO 4217 code they quote in.
    #[serde(default = "usd")]
    pub currency: String,
    /// Typical lead time, days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_days: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

fn usd() -> String {
    "USD".into()
}

impl Persist for Supplier {
    const KIND: &'static str = "sourcing.supplier";
}

impl Supplier {
    /// Whether a line made by `process` (None: bought) is theirs to quote.
    pub fn covers(&self, process: Option<&str>) -> bool {
        if self.kind.is_empty() {
            return true;
        }
        match process {
            None | Some("buy") => self.kind.iter().any(|k| k == "distributor"),
            Some(p) => self.kind.iter().any(|k| k == p),
        }
    }

    fn check(mut self) -> Result<Supplier, CommandError> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err(invalid("a supplier needs a name"));
        }
        for k in &self.kind {
            if k != "distributor" && !PROCESSES.contains(&k.as_str()) {
                return Err(invalid(format!("supplier kind “{k}” is not a process ({}) or \"distributor\"", PROCESSES.join(", "))));
            }
        }
        self.kind.dedup();
        self.currency = check_currency(&self.currency)?;
        let email = self.email.trim();
        if !email.is_empty() && !(email.contains('@') && !email.contains(' ')) {
            return Err(invalid(format!("“{email}” is not an email address")));
        }
        self.email = email.to_string();
        Ok(self)
    }
}

/// `sourcing.supplier.create`: adds a supplier.
#[derive(Deserialize, JsonSchema)]
pub struct CreateSupplier {
    #[serde(flatten)]
    pub supplier: Supplier,
}

impl Command for CreateSupplier {
    const NAME: &'static str = "sourcing.supplier.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = self.supplier.check()?;
        let id = tx.create();
        tx.set(id, s)?;
        Ok(json!({ "id": id }))
    }
}

/// `sourcing.supplier.update`: changes any of a supplier's fields (what is
/// left out stays as it was). Delete with `core.entity.delete`.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateSupplier {
    pub target: StableId,
    pub name: Option<String>,
    pub kind: Option<Vec<String>>,
    pub website: Option<String>,
    pub email: Option<String>,
    /// "" removes it.
    pub phone: Option<String>,
    pub country: Option<String>,
    pub currency: Option<String>,
    /// 0 removes it.
    pub lead_days: Option<u32>,
    pub notes: Option<String>,
}

impl Command for UpdateSupplier {
    const NAME: &'static str = "sourcing.supplier.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = tx.get::<Supplier>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a supplier", self.target)))?;
        if let Some(v) = self.name {
            s.name = v;
        }
        if let Some(v) = self.kind {
            s.kind = v;
        }
        if let Some(v) = self.website {
            s.website = v;
        }
        if let Some(v) = self.email {
            s.email = v;
        }
        if let Some(v) = self.phone {
            s.phone = Some(v).filter(|p| !p.trim().is_empty());
        }
        if let Some(v) = self.country {
            s.country = v;
        }
        if let Some(v) = self.currency {
            s.currency = v;
        }
        if let Some(v) = self.lead_days {
            s.lead_days = Some(v).filter(|d| *d > 0);
        }
        if let Some(v) = self.notes {
            s.notes = v;
        }
        tx.set(self.target, s.check()?)?;
        Ok(json!({ "id": self.target }))
    }
}

// ---- Requests for quote ------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RfqStatus {
    /// Being put together.
    #[default]
    Draft,
    /// The person has sent it to the suppliers.
    Sent,
    /// Quotes are in and decided.
    Closed,
}

/// One BOM line asked about, as it was when the RFQ was made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RfqLine {
    /// The BOM line's key (`make:…` / `buy:…`).
    pub line: String,
    /// What it was called, so the request still reads if the design moves on.
    pub name: String,
    /// How many one finished unit needs.
    pub qty: u32,
}

/// A request for quote: which BOM lines, at which production quantities, by
/// when, sent to which suppliers.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rfq {
    pub name: String,
    /// What the BOM is of: a shape, assembly or instance (none: the project).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<StableId>,
    /// The configuration of `scope` quoted (none: as it is shown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    pub lines: Vec<RfqLine>,
    /// Production quantities to price (finished units): 1, 10, 100.
    pub quantities: Vec<u32>,
    /// Quotes wanted by (YYYY-MM-DD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    pub suppliers: Vec<StableId>,
    #[serde(default)]
    pub status: RfqStatus,
    /// Finish asked for, unless a line says otherwise ("as machined, deburred").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub finish: String,
    /// Tolerances note ("ISO 2768-m unless the drawing says otherwise").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tolerances: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// When it was made (ms since 1970).
    #[serde(default)]
    pub created: u64,
}

impl Persist for Rfq {
    const KIND: &'static str = "sourcing.rfq";
}

fn check_quantities(q: &mut Vec<u32>) -> Result<(), CommandError> {
    q.retain(|n| *n > 0);
    q.sort_unstable();
    q.dedup();
    if q.is_empty() {
        return Err(invalid("an RFQ needs at least one production quantity (e.g. [1, 10, 100])"));
    }
    Ok(())
}

fn check_suppliers(tx: &Tx, ids: &mut Vec<StableId>) -> Result<(), CommandError> {
    ids.dedup();
    for id in ids.iter() {
        if tx.get::<Supplier>(*id).is_none() {
            return Err(bad(ErrorCode::UnresolvedReference, format!("{id} is not a supplier")));
        }
    }
    Ok(())
}

fn check_lines(lines: &mut Vec<RfqLine>) -> Result<(), CommandError> {
    let mut seen = std::collections::HashSet::new();
    lines.retain(|l| seen.insert(l.line.clone()));
    if lines.is_empty() {
        return Err(invalid("an RFQ needs at least one BOM line"));
    }
    for l in lines.iter() {
        if !(l.line.starts_with("make:") || l.line.starts_with("buy:")) {
            return Err(invalid(format!("“{}” is not a BOM line key (make:… or buy:…)", l.line)));
        }
        if l.qty == 0 {
            return Err(invalid(format!("{}: quantity per unit must be at least 1", l.name)));
        }
    }
    Ok(())
}

/// `sourcing.rfq.create`: a request for quote over some BOM lines.
#[derive(Deserialize, JsonSchema)]
pub struct CreateRfq {
    pub name: String,
    #[serde(default)]
    pub scope: Option<StableId>,
    #[serde(default)]
    pub config: Option<String>,
    pub lines: Vec<RfqLine>,
    #[serde(default = "default_quantities")]
    pub quantities: Vec<u32>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub suppliers: Vec<StableId>,
    #[serde(default)]
    pub finish: String,
    #[serde(default)]
    pub tolerances: String,
    #[serde(default)]
    pub notes: String,
}

fn default_quantities() -> Vec<u32> {
    vec![1, 10, 100]
}

impl Command for CreateRfq {
    const NAME: &'static str = "sourcing.rfq.create";

    fn apply(mut self, tx: &mut Tx) -> Result<Value, CommandError> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(invalid("an RFQ needs a name"));
        }
        check_lines(&mut self.lines)?;
        check_quantities(&mut self.quantities)?;
        check_suppliers(tx, &mut self.suppliers)?;
        if let Some(s) = self.scope {
            if !tx.exists(s) {
                return Err(bad(ErrorCode::UnresolvedReference, format!("{s} does not exist")));
            }
        }
        let rfq = Rfq {
            name,
            scope: self.scope,
            config: self.config.filter(|c| !c.is_empty()),
            lines: self.lines,
            quantities: self.quantities,
            due: opt_date(self.due, "due")?,
            suppliers: self.suppliers,
            status: RfqStatus::Draft,
            finish: self.finish,
            tolerances: self.tolerances,
            notes: self.notes,
            created: now_ms(),
        };
        let id = tx.create();
        tx.set(id, rfq)?;
        Ok(json!({ "id": id }))
    }
}

/// `sourcing.rfq.update`: changes an RFQ (what is left out stays). Delete
/// with `core.entity.delete`.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateRfq {
    pub target: StableId,
    pub name: Option<String>,
    pub lines: Option<Vec<RfqLine>>,
    pub quantities: Option<Vec<u32>>,
    /// "" removes it.
    pub due: Option<String>,
    pub suppliers: Option<Vec<StableId>>,
    pub status: Option<RfqStatus>,
    pub finish: Option<String>,
    pub tolerances: Option<String>,
    pub notes: Option<String>,
}

impl Command for UpdateRfq {
    const NAME: &'static str = "sourcing.rfq.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut r = tx.get::<Rfq>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not an RFQ", self.target)))?;
        if let Some(v) = self.name {
            if v.trim().is_empty() {
                return Err(invalid("an RFQ needs a name"));
            }
            r.name = v.trim().to_string();
        }
        if let Some(mut v) = self.lines {
            check_lines(&mut v)?;
            r.lines = v;
        }
        if let Some(mut v) = self.quantities {
            check_quantities(&mut v)?;
            r.quantities = v;
        }
        if let Some(v) = self.due {
            r.due = opt_date(Some(v), "due")?;
        }
        if let Some(mut v) = self.suppliers {
            check_suppliers(tx, &mut v)?;
            r.suppliers = v;
        }
        if let Some(v) = self.status {
            r.status = v;
        }
        if let Some(v) = self.finish {
            r.finish = v;
        }
        if let Some(v) = self.tolerances {
            r.tolerances = v;
        }
        if let Some(v) = self.notes {
            r.notes = v;
        }
        tx.set(self.target, r)?;
        Ok(json!({ "id": self.target }))
    }
}

// ---- Quotes ------------------------------------------------------------------

/// A unit price at an order quantity (pieces of that line).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PriceBreak {
    pub qty: u32,
    pub unit_price: f64,
}

/// The unit price that applies when ordering `qty`: the largest break at or
/// below it, else the smallest break.
pub fn price_at(breaks: &[PriceBreak], qty: u32) -> Option<f64> {
    breaks.iter().filter(|b| b.qty <= qty).max_by_key(|b| b.qty).or_else(|| breaks.iter().min_by_key(|b| b.qty)).map(|b| b.unit_price)
}

fn check_breaks(breaks: &mut Vec<PriceBreak>, what: &str) -> Result<(), CommandError> {
    for b in breaks.iter() {
        if b.qty == 0 || !b.unit_price.is_finite() || b.unit_price < 0.0 {
            return Err(invalid(format!("{what}: each price break is a quantity of at least 1 and a price of at least 0")));
        }
    }
    breaks.sort_by_key(|b| b.qty);
    breaks.dedup_by_key(|b| b.qty);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QuoteLine {
    /// The BOM line's key.
    pub line: String,
    /// Unit prices by order quantity (pieces of this line).
    pub prices: Vec<PriceBreak>,
    /// This line's lead time when it differs from the quote's (days).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_days: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuoteStatus {
    #[default]
    Received,
    Accepted,
    Declined,
}

/// A supplier's answer: prices per line and quantity, lead time, validity.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Quote {
    pub supplier: StableId,
    /// The RFQ it answers, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfq: Option<StableId>,
    /// The supplier's own quote number.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference: String,
    pub lines: Vec<QuoteLine>,
    pub currency: String,
    /// Lead time, days, for the whole quote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_days: Option<u32>,
    /// YYYY-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    /// YYYY-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// Files that came with it, by name (the PDF quote, terms).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
    #[serde(default)]
    pub status: QuoteStatus,
}

impl Persist for Quote {
    const KIND: &'static str = "sourcing.quote";
}

impl Quote {
    pub fn line(&self, key: &str) -> Option<&QuoteLine> {
        self.lines.iter().find(|l| l.line == key)
    }

    /// Lead time of one line (its own, else the quote's).
    pub fn lead(&self, key: &str) -> Option<u32> {
        self.line(key).and_then(|l| l.lead_days).or(self.lead_days)
    }
}

fn check_quote(tx: &Tx, mut q: Quote) -> Result<Quote, CommandError> {
    let s = tx.get::<Supplier>(q.supplier).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a supplier", q.supplier)))?;
    if q.currency.trim().is_empty() {
        q.currency = s.currency.clone();
    }
    q.currency = check_currency(&q.currency)?;
    if let Some(r) = q.rfq {
        if tx.get::<Rfq>(r).is_none() {
            return Err(bad(ErrorCode::UnresolvedReference, format!("{r} is not an RFQ")));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for l in &mut q.lines {
        if !seen.insert(l.line.clone()) {
            return Err(invalid(format!("line {} is quoted twice", l.line)));
        }
        let key = l.line.clone();
        check_breaks(&mut l.prices, &key)?;
    }
    q.valid_until = opt_date(q.valid_until, "valid until")?;
    q.received = opt_date(q.received, "received")?;
    Ok(q)
}

/// `sourcing.quote.create`: records a supplier's quote.
#[derive(Deserialize, JsonSchema)]
pub struct CreateQuote {
    pub supplier: StableId,
    #[serde(default)]
    pub rfq: Option<StableId>,
    #[serde(default)]
    pub reference: String,
    pub lines: Vec<QuoteLine>,
    /// Default: the supplier's currency.
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub lead_days: Option<u32>,
    #[serde(default)]
    pub valid_until: Option<String>,
    #[serde(default)]
    pub received: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub attachments: Vec<String>,
}

impl Command for CreateQuote {
    const NAME: &'static str = "sourcing.quote.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let q = check_quote(
            tx,
            Quote {
                supplier: self.supplier,
                rfq: self.rfq,
                reference: self.reference,
                lines: self.lines,
                currency: self.currency,
                lead_days: self.lead_days,
                valid_until: self.valid_until,
                received: self.received,
                notes: self.notes,
                attachments: self.attachments,
                status: QuoteStatus::Received,
            },
        )?;
        let id = tx.create();
        tx.set(id, q)?;
        Ok(json!({ "id": id }))
    }
}

/// `sourcing.quote.update`: changes a quote (what is left out stays).
/// Declining a quote withdraws the awards made from it.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateQuote {
    pub target: StableId,
    pub reference: Option<String>,
    pub lines: Option<Vec<QuoteLine>>,
    pub currency: Option<String>,
    /// 0 removes it.
    pub lead_days: Option<u32>,
    pub valid_until: Option<String>,
    pub received: Option<String>,
    pub notes: Option<String>,
    pub attachments: Option<Vec<String>>,
    pub status: Option<QuoteStatus>,
}

impl Command for UpdateQuote {
    const NAME: &'static str = "sourcing.quote.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut q = tx.get::<Quote>(self.target).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a quote", self.target)))?;
        if let Some(v) = self.reference {
            q.reference = v;
        }
        if let Some(v) = self.lines {
            q.lines = v;
        }
        if let Some(v) = self.currency {
            q.currency = v;
        }
        if let Some(v) = self.lead_days {
            q.lead_days = Some(v).filter(|d| *d > 0);
        }
        if let Some(v) = self.valid_until {
            q.valid_until = Some(v);
        }
        if let Some(v) = self.received {
            q.received = Some(v);
        }
        if let Some(v) = self.notes {
            q.notes = v;
        }
        if let Some(v) = self.attachments {
            q.attachments = v;
        }
        if let Some(v) = self.status {
            q.status = v;
        }
        let q = check_quote(tx, q)?;
        let withdrawn = if q.status == QuoteStatus::Declined { withdraw_awards(tx, self.target)? } else { 0 };
        tx.set(self.target, q)?;
        Ok(json!({ "id": self.target, "awards_withdrawn": withdrawn }))
    }
}

fn withdraw_awards(tx: &mut Tx, quote: StableId) -> Result<usize, CommandError> {
    let from: Vec<StableId> = all::<Award>(tx.world()).into_iter().filter(|(_, a)| a.quote == Some(quote)).map(|(id, _)| id).collect();
    for id in &from {
        tx.delete(*id)?;
    }
    Ok(from.len())
}

// ---- Awards ------------------------------------------------------------------

/// Who makes or supplies one BOM line, and at what price. The BOM's costs
/// use an awarded price over the estimate.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Award {
    /// The BOM line's key.
    pub line: String,
    /// What the line was called when awarded (to say which award went stale).
    #[serde(default)]
    pub name: String,
    pub supplier: StableId,
    /// The quote it came from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<StableId>,
    /// Unit prices by order quantity.
    pub prices: Vec<PriceBreak>,
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_days: Option<u32>,
}

impl Persist for Award {
    const KIND: &'static str = "sourcing.award";
}

/// The award of a line, if it has one.
pub fn award_of(world: &de_core::bevy_ecs::world::World, line: &str) -> Option<(StableId, Award)> {
    all::<Award>(world).into_iter().find(|(_, a)| a.line == line)
}

fn put_award(tx: &mut Tx, award: Award) -> Result<StableId, CommandError> {
    let id = match award_of(tx.world(), &award.line) {
        Some((id, _)) => id,
        None => tx.create(),
    };
    tx.set(id, award)?;
    Ok(id)
}

/// `sourcing.award`: gives one BOM line to a supplier at a price — from a
/// quote (`quote`, its prices for that line) or stated (`prices`). Replaces
/// the line's earlier award.
#[derive(Deserialize, JsonSchema)]
pub struct SetAward {
    pub line: String,
    /// What the line is called (shown if the award later goes stale).
    #[serde(default)]
    pub name: String,
    /// Default: the quote's supplier.
    pub supplier: Option<StableId>,
    pub quote: Option<StableId>,
    /// Default: the quote's prices for this line.
    pub prices: Option<Vec<PriceBreak>>,
    pub currency: Option<String>,
    pub lead_days: Option<u32>,
}

impl Command for SetAward {
    const NAME: &'static str = "sourcing.award";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let quote = match self.quote {
            Some(q) => Some(tx.get::<Quote>(q).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{q} is not a quote")))?),
            None => None,
        };
        let supplier = self.supplier.or(quote.as_ref().map(|q| q.supplier)).ok_or_else(|| invalid("an award names a supplier or a quote"))?;
        let s = tx.get::<Supplier>(supplier).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{supplier} is not a supplier")))?;
        let mut prices = match (self.prices, &quote) {
            (Some(p), _) => p,
            (None, Some(q)) => q.line(&self.line).map(|l| l.prices.clone()).ok_or_else(|| invalid(format!("the quote has no price for line {}", self.line)))?,
            (None, None) => return Err(invalid("an award needs prices, or a quote to take them from")),
        };
        check_breaks(&mut prices, &self.line)?;
        if prices.is_empty() {
            return Err(invalid("an award needs at least one price"));
        }
        let currency = check_currency(&self.currency.or(quote.as_ref().map(|q| q.currency.clone())).unwrap_or(s.currency))?;
        let lead_days = self.lead_days.or(quote.as_ref().and_then(|q| q.lead(&self.line))).or(s.lead_days);
        let id = put_award(tx, Award { line: self.line, name: self.name, supplier, quote: self.quote, prices, currency, lead_days })?;
        Ok(json!({ "id": id }))
    }
}

/// `sourcing.award.clear`: the line goes back to its estimate.
#[derive(Deserialize, JsonSchema)]
pub struct ClearAward {
    pub line: String,
}

impl Command for ClearAward {
    const NAME: &'static str = "sourcing.award.clear";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        match award_of(tx.world(), &self.line) {
            Some((id, _)) => {
                tx.delete(id)?;
                Ok(json!({ "cleared": id }))
            }
            None => Err(invalid(format!("line {} has no award", self.line))),
        }
    }
}

/// `sourcing.quote.award`: accepts a quote and awards it the lines it prices
/// (all of them, or those listed); other quotes for those lines keep their
/// status. Names, when given, label the awards (line key → name).
#[derive(Deserialize, JsonSchema)]
pub struct AwardQuote {
    pub quote: StableId,
    #[serde(default)]
    pub lines: Option<Vec<String>>,
    #[serde(default)]
    pub names: std::collections::BTreeMap<String, String>,
}

impl Command for AwardQuote {
    const NAME: &'static str = "sourcing.quote.award";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut q = tx.get::<Quote>(self.quote).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a quote", self.quote)))?;
        let keys: Vec<String> = match self.lines {
            Some(l) => l,
            None => q.lines.iter().filter(|l| !l.prices.is_empty()).map(|l| l.line.clone()).collect(),
        };
        if keys.is_empty() {
            return Err(invalid("the quote prices no lines"));
        }
        let mut ids = vec![];
        for key in &keys {
            let line = q.line(key).ok_or_else(|| invalid(format!("the quote has no price for line {key}")))?;
            if line.prices.is_empty() {
                return Err(invalid(format!("the quote has no price for line {key}")));
            }
            let award = Award {
                line: key.clone(),
                name: self.names.get(key).cloned().unwrap_or_default(),
                supplier: q.supplier,
                quote: Some(self.quote),
                prices: line.prices.clone(),
                currency: q.currency.clone(),
                lead_days: q.lead(key),
            };
            ids.push(put_award(tx, award)?);
        }
        q.status = QuoteStatus::Accepted;
        tx.set(self.quote, q)?;
        Ok(json!({ "id": self.quote, "awards": ids }))
    }
}
