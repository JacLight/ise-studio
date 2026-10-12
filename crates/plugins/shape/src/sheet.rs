//! Spreadsheets: a grid of cells holding numbers (with units), text and
//! formulas, for the calculations a design runs on — a load table, a
//! clearance budget, a cut list — with intermediate rows that are not
//! parameters of anything.
//!
//! A sheet drives the model only through explicit links: any parameter can
//! follow a cell (`=Sizes.B2`) or a cell's alias (`=Sizes.width`), exactly as
//! it follows another parameter, and a cell can read any parameter in the
//! project (`=Desk.width / 2`). References to other things are stored by id,
//! so renaming never breaks them.
//!
//! Cells hold what was typed:
//! - a number, optionally with a unit: `200`, `200 mm`, `1.2m`, `2in`, `45°`;
//!   lengths are worked in millimetres and angles in degrees, and a cell shows
//!   its value in the unit it was written in (or the cell's own `unit`, which
//!   also says what a bare number in it means);
//! - text: anything else (`'200` keeps a number as text);
//! - a formula: `=B2*2 + 5mm`, `=sum(B2:B9)`, `=Desk.width - 2*thickness`.
//!   Formulas use the parameter grammar (+ − × ÷ ^, min max abs round floor
//!   ceil sqrt sin cos tan asin acos atan atan2 sum average count mod pow
//!   hypot exp ln log pi) plus cell references (`B2`, `$B$2`), ranges inside
//!   functions (`B2:B9`, empty and text cells skipped) and aliases.

use crate::{
    ResolvedParams,
    params::{self, Expr},
    table::{self, OutCell, OutSheet},
};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

/// A spreadsheet.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sheet {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Cells by address ("B2"); only cells with something in them.
    #[serde(default)]
    pub cells: BTreeMap<String, Cell>,
}

impl Persist for Sheet {
    const KIND: &'static str = "shape.sheet";
}

/// One cell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Cell {
    /// What was typed (formulas keep references to other things by id).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub input: String,
    /// A name for this cell, usable in formulas here and as `Sheet.alias` elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Unit to show the value in (mm, cm, m, in, ft, deg): a bare number typed here is in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl Sheet {
    /// What other things can read: every cell's address and every alias.
    pub fn keys(&self) -> Vec<String> {
        let mut k: Vec<String> = self.cells.iter().filter(|(_, c)| !c.input.is_empty()).map(|(a, _)| a.clone()).collect();
        k.extend(self.cells.values().filter_map(|c| c.alias.clone()));
        k
    }

    fn alias_map(&self) -> HashMap<String, String> {
        self.cells.iter().filter_map(|(a, c)| c.alias.clone().map(|n| (n, a.clone()))).collect()
    }
}

/// Derived: what every cell works out to.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct SheetValues {
    pub cells: BTreeMap<String, CellValue>,
    /// alias → address
    pub aliases: BTreeMap<String, String>,
    /// How many cells show an error.
    pub errors: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct CellValue {
    /// The number in base units (mm, degrees), if it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Its text, if it is text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The unit it shows in ("" for a plain number).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// What the cell shows.
    pub shown: String,
    /// The formula with references by name, for editing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SheetValues {
    /// Numbers by address and alias (what other entities' formulas read).
    pub fn numbers(&self) -> BTreeMap<String, f64> {
        let mut out: BTreeMap<String, f64> = self.cells.iter().filter_map(|(a, c)| c.value.map(|v| (a.clone(), v))).collect();
        for (alias, at) in &self.aliases {
            if let Some(v) = self.cells.get(at).and_then(|c| c.value) {
                out.insert(alias.clone(), v);
            }
        }
        out
    }
}

// ---- Units and literals --------------------------------------------------------------

/// Millimetres (or degrees) per unit, and the unit as shown.
pub fn unit_factor(u: &str) -> Option<(f64, &'static str)> {
    Some(match u.trim() {
        "" => (1.0, ""),
        "mm" => (1.0, "mm"),
        "cm" => (10.0, "cm"),
        "m" => (1000.0, "m"),
        "in" | "\"" | "inch" => (25.4, "in"),
        "ft" | "'" => (304.8, "ft"),
        "deg" | "°" => (1.0, "°"),
        _ => return None,
    })
}

enum Literal {
    Number(f64, &'static str),
    Text(String),
}

fn literal(input: &str, cell_unit: Option<&str>) -> Literal {
    let t = input.trim();
    if let Some(text) = t.strip_prefix('\'') {
        return Literal::Text(text.to_string());
    }
    // Split the number from a unit after it.
    let end = t.char_indices().find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || ((*c == '-' || *c == '+') && (*i == 0 || t[..*i].ends_with(['e', 'E']))) || ((*c == 'e' || *c == 'E') && *i > 0))).map(|(i, _)| i).unwrap_or(t.len());
    let (num, rest) = t.split_at(end);
    let Ok(v) = num.parse::<f64>() else { return Literal::Text(t.to_string()) };
    if !v.is_finite() {
        return Literal::Text(t.to_string());
    }
    let rest = rest.trim();
    if rest == "%" {
        return Literal::Number(v / 100.0, "");
    }
    let unit = if rest.is_empty() { cell_unit.unwrap_or("") } else { rest };
    match unit_factor(unit) {
        Some((f, shown)) => Literal::Number(v * f, shown),
        None => Literal::Text(t.to_string()),
    }
}

/// What a number with a unit measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Length,
    Angle,
    Plain,
}

/// A number as typed in a table (`200`, `200 mm`, `2in`, `45°`) in
/// millimetres or degrees; `unit` is what a bare number is in.
pub fn number(s: &str, unit: Option<&str>) -> Option<(f64, Kind)> {
    match literal(s, unit) {
        Literal::Number(v, u) => Some((v, match u {
            "" => Kind::Plain,
            "°" => Kind::Angle,
            _ => Kind::Length,
        })),
        Literal::Text(_) => None,
    }
}

pub fn is_address(s: &str) -> bool {
    !s.contains('$') && table::parse_address(s).is_some_and(|(c, r)| table::address(c, r) == s)
}

/// "b$2" → "B2" when it is a cell address.
fn normal_address(s: &str) -> Option<String> {
    table::parse_address(s.trim()).map(|(c, r)| table::address(c, r))
}

fn shown(v: f64, unit: &str) -> String {
    let f = unit_factor(unit).map(|(f, _)| f).unwrap_or(1.0);
    let x = v / f;
    let mut s = format!("{:.6}", x);
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" {
        s = "0".into();
    }
    match unit {
        "" => s,
        "°" => format!("{s}°"),
        u => format!("{s} {u}"),
    }
}

// ---- Formulas ------------------------------------------------------------------------------

/// Replaces `A1:B3` ranges with the listed cells that hold numbers (or
/// formulas), for use inside functions: `sum(B2:B9)`.
fn expand_ranges(src: &str, sheet: &Sheet) -> Result<String, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let name_char = |c: char| c.is_ascii_alphanumeric() || c == '$' || c == '_';
    while i < chars.len() {
        if chars[i] == '#' {
            // a stored reference: copy it whole
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '-' || chars[i] == '.' || chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.extend(&chars[start..i]);
            continue;
        }
        if chars[i] == ':' {
            // find the cell names either side
            let mut a = out.len();
            let bytes: Vec<char> = out.chars().collect();
            let mut k = bytes.len();
            while k > 0 && name_char(bytes[k - 1]) {
                k -= 1;
            }
            let left: String = bytes[k..].iter().collect();
            let mut j = i + 1;
            while j < chars.len() && name_char(chars[j]) {
                j += 1;
            }
            let right: String = chars[i + 1..j].iter().collect();
            let (Some(p), Some(q)) = (table::parse_address(&left), table::parse_address(&right)) else {
                return Err(format!("“{left}:{right}” is not a range of cells"));
            };
            a -= left.len();
            out.truncate(a);
            let (c0, c1, r0, r1) = (p.0.min(q.0), p.0.max(q.0), p.1.min(q.1), p.1.max(q.1));
            if (c1 - c0 + 1) as u64 * (r1 - r0 + 1) as u64 > 100_000 {
                return Err(format!("{left}:{right} is too large a range"));
            }
            let mut list = Vec::new();
            for c in c0..=c1 {
                for r in r0..=r1 {
                    let at = table::address(c, r);
                    if let Some(cell) = sheet.cells.get(&at)
                        && !cell.input.is_empty()
                        && (cell.input.trim_start().starts_with('=') || matches!(literal(&cell.input, cell.unit.as_deref()), Literal::Number(..)))
                    {
                        list.push(at);
                    }
                }
            }
            if list.is_empty() {
                return Err(format!("{left}:{right} has no numbers"));
            }
            out.push_str(&list.join(","));
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    Ok(out)
}

/// Parses a formula of this sheet (as stored).
fn parse_formula(src: &str, sheet: &Sheet, aliases: &HashMap<String, String>) -> Result<Expr, String> {
    let body = src.trim().trim_start_matches('=');
    let expanded = expand_ranges(body, sheet)?;
    params::parse(&expanded, &|name: &str| {
        if name.starts_with('#') {
            return params::stored_ref(name);
        }
        if let Some(a) = normal_address(name) {
            return Ok((None, a));
        }
        if let Some(a) = aliases.get(name) {
            return Ok((None, a.clone()));
        }
        Err(format!("“{name}” is not a cell or alias of this sheet"))
    })
}

/// Is this name one this sheet resolves itself (a cell address or one of its aliases)?
fn local(name: &str, aliases: &HashMap<String, String>) -> bool {
    normal_address(name).is_some() || aliases.contains_key(name)
}

/// The formula as people write it turned into its stored form: references to
/// other things by id. Fails, saying why, on anything it cannot resolve.
pub(crate) fn canonical_formula(world: &World, src: &str, sheet: &Sheet) -> Result<String, String> {
    let aliases = sheet.alias_map();
    let body = src.trim().trim_start_matches('=');
    // A range with nothing in it yet is fine to write (the cell shows an error until it fills).
    let expanded = expand_ranges(body, sheet).unwrap_or_else(|_| {
        let mut out = String::new();
        let mut skip = false;
        for c in body.chars() {
            if c == ':' {
                skip = true;
                continue;
            }
            if skip && (c.is_ascii_alphanumeric() || c == '$') {
                continue;
            }
            skip = false;
            out.push(c);
        }
        out
    });
    let found: RefCell<Vec<(String, String)>> = RefCell::new(Vec::new());
    params::parse(&expanded, &|name: &str| {
        if local(name, &aliases) {
            return Ok((None, name.to_string()));
        }
        let (id, key) = params::resolve_external(world, name)?;
        if !name.starts_with('#') {
            found.borrow_mut().push((name.to_string(), format!("#{}.{key}", id.unwrap())));
        }
        Ok((id, key))
    })?;
    let mut out = body.to_string();
    let mut found = found.into_inner();
    found.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
    for (name, stored) in found {
        out = replace_name(&out, &name, &stored);
    }
    Ok(format!("={out}"))
}

/// Replaces whole-name occurrences of `name` (between operators, brackets, commas or ends).
fn replace_name(src: &str, name: &str, with: &str) -> String {
    let bound = |c: Option<char>| c.is_none_or(|c| "+-*/^×÷(),: ".contains(c));
    let mut out = String::new();
    let mut rest = src;
    let mut prev: Option<char> = None;
    while let Some(i) = rest.find(name) {
        let before = if i == 0 { prev } else { rest[..i].chars().last() };
        let after = rest[i + name.len()..].chars().next();
        out.push_str(&rest[..i]);
        if bound(before) && bound(after) {
            out.push_str(with);
        } else {
            out.push_str(name);
        }
        prev = name.chars().last();
        rest = &rest[i + name.len()..];
    }
    out.push_str(rest);
    out
}

/// A stored formula with references by current names (for people).
pub fn display_formula(world: &World, stored: &str) -> String {
    let chars: Vec<char> = stored.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '-') {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
            }
            let r: String = chars[start..i].iter().collect();
            match params::stored_ref(&r) {
                Ok((Some(id), key)) => {
                    let name = world.resource::<IdIndex>().get(id).and_then(|e| params::keys_of(world, e)).map(|(n, _)| n).unwrap_or_else(|| "(missing)".into());
                    out.push_str(&format!("{name}.{key}"));
                }
                _ => out.push_str(&r),
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

// ---- Evaluation ------------------------------------------------------------------------------

/// Works out every cell of a sheet, reading other entities' resolved values.
pub fn evaluate(world: &World, sheet: &Sheet) -> SheetValues {
    let aliases = sheet.alias_map();
    let memo: RefCell<HashMap<String, Result<(f64, &'static str), String>>> = RefCell::new(HashMap::new());
    let visiting: RefCell<HashSet<String>> = RefCell::new(HashSet::new());

    fn number(world: &World, sheet: &Sheet, aliases: &HashMap<String, String>, at: &str, memo: &RefCell<HashMap<String, Result<(f64, &'static str), String>>>, visiting: &RefCell<HashSet<String>>) -> Result<(f64, &'static str), String> {
        if let Some(r) = memo.borrow().get(at) {
            return r.clone();
        }
        let Some(cell) = sheet.cells.get(at).filter(|c| !c.input.is_empty()) else {
            return Err(format!("{at} is empty"));
        };
        let r = if let Some(f) = cell.input.trim_start().strip_prefix('=') {
            if !visiting.borrow_mut().insert(at.to_string()) {
                return Err(format!("{at} refers to itself"));
            }
            let r = (|| {
                let e = parse_formula(f, sheet, aliases)?;
                let mut refs = Vec::new();
                e.refs(&mut refs);
                let v = e.eval(&|id, key| match id {
                    None => number(world, sheet, aliases, key, memo, visiting).map(|(v, _)| v).map_err(|m| if m.contains(" refers to itself") || m.starts_with(key) { m } else { format!("{key}: {m}") }),
                    Some(id) => {
                        let e = world.resource::<IdIndex>().get(id).ok_or("a referenced object was deleted")?;
                        world.get::<ResolvedParams>(e).and_then(|r| r.0.get(key).copied()).ok_or_else(|| format!("“{key}” of {} has no value", params::keys_of(world, e).map(|(n, _)| n).unwrap_or_default()))
                    }
                })?;
                if !v.is_finite() {
                    return Err(format!("works out to {v}"));
                }
                let unit = match cell.unit.as_deref().and_then(unit_factor) {
                    Some((_, u)) => u,
                    None => refs.iter().find_map(|(id, key)| ref_unit(world, sheet, aliases, *id, key, memo, visiting)).unwrap_or(""),
                };
                Ok((v, unit))
            })();
            visiting.borrow_mut().remove(at);
            r
        } else {
            match literal(&cell.input, cell.unit.as_deref()) {
                Literal::Number(v, u) => Ok((v, u)),
                Literal::Text(_) => Err(format!("{at} holds text")),
            }
        };
        memo.borrow_mut().insert(at.to_string(), r.clone());
        r
    }

    /// The unit a reference carries, for a formula that sets none: lengths stay lengths.
    fn ref_unit(world: &World, sheet: &Sheet, aliases: &HashMap<String, String>, id: Option<StableId>, key: &str, memo: &RefCell<HashMap<String, Result<(f64, &'static str), String>>>, visiting: &RefCell<HashSet<String>>) -> Option<&'static str> {
        match id {
            None => number(world, sheet, aliases, key, memo, visiting).ok().map(|(_, u)| u).filter(|u| !u.is_empty()),
            Some(id) => {
                let e = world.resource::<IdIndex>().get(id)?;
                if let Some((_, ps)) = params::params_of(world, e) {
                    let p = ps.iter().find(|p| p.key == key)?;
                    return match p.unit.as_str() {
                        "mm" => Some("mm"),
                        "deg" => Some("°"),
                        _ => None,
                    };
                }
                let v = world.get::<SheetValues>(e)?;
                let at = v.aliases.get(key).map(String::as_str).unwrap_or(key);
                v.cells.get(at).and_then(|c| unit_factor(&c.unit)).map(|(_, u)| u).filter(|u| !u.is_empty())
            }
        }
    }

    let mut out = SheetValues { aliases: aliases.iter().map(|(a, b)| (a.clone(), b.clone())).collect(), ..Default::default() };
    for (at, cell) in &sheet.cells {
        if cell.input.is_empty() {
            continue;
        }
        let mut v = CellValue::default();
        if cell.input.trim_start().starts_with('=') {
            v.formula = Some(display_formula(world, cell.input.trim_start()));
        }
        match literal(&cell.input, cell.unit.as_deref()) {
            Literal::Text(t) if v.formula.is_none() => {
                v.shown = t.clone();
                v.text = Some(t);
            }
            _ => match number(world, sheet, &aliases, at, &memo, &visiting) {
                Ok((x, u)) => {
                    v.value = Some(x);
                    v.unit = u.to_string();
                    v.shown = shown(x, u);
                }
                Err(e) => {
                    v.shown = "#ERROR".into();
                    v.error = Some(e);
                    out.errors += 1;
                }
            },
        }
        out.cells.insert(at.clone(), v);
    }
    out
}

pub(crate) fn sheet_values(world: &World, e: Entity) -> Option<Result<SheetValues, String>> {
    let sheet = world.get::<Sheet>(e)?;
    Some(Ok(evaluate(world, sheet)))
}

/// Every entity a sheet's formulas read.
fn references(sheet: &Sheet) -> Vec<StableId> {
    let aliases = sheet.alias_map();
    let mut out = Vec::new();
    for cell in sheet.cells.values() {
        if let Some(f) = cell.input.trim_start().strip_prefix('=')
            && let Ok(e) = parse_formula(f, sheet, &aliases)
        {
            let mut refs = Vec::new();
            e.refs(&mut refs);
            out.extend(refs.into_iter().filter_map(|(id, _)| id));
        }
    }
    out.sort_by_key(|id| id.to_string());
    out.dedup();
    out
}

// ---- Editing --------------------------------------------------------------------------------

fn bad(m: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, m)
}

fn check_alias(alias: &str) -> Result<(), CommandError> {
    let ok = !alias.is_empty() && alias.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !alias.starts_with(|c: char| c.is_ascii_digit());
    if !ok {
        return Err(bad(format!("alias “{alias}” must be a plain name (letters, digits, _)")));
    }
    if table::parse_address(alias).is_some() {
        return Err(bad(format!("alias “{alias}” reads as a cell address; choose another name")));
    }
    Ok(())
}

/// Applies edits to a sheet, checks it, and keeps its dependencies in step.
fn apply_edits(tx: &mut Tx, id: Option<StableId>, mut sheet: Sheet, cells: &BTreeMap<String, Option<String>>, aliases: &BTreeMap<String, String>, units: &BTreeMap<String, String>) -> Result<(StableId, Sheet), CommandError> {
    if sheet.name.trim().is_empty() || sheet.name.contains('.') {
        return Err(bad("a sheet needs a name without dots"));
    }
    let addr = |a: &str| normal_address(a).ok_or_else(|| bad(format!("“{a}” is not a cell address (like B2)")));
    let mut formulas = Vec::new();
    for (a, input) in cells {
        let at = addr(a)?;
        let input = input.clone().unwrap_or_default();
        if input.trim().is_empty() {
            if let Some(c) = sheet.cells.get_mut(&at) {
                c.input.clear();
            }
        } else {
            let t = input.trim().to_string();
            if t.starts_with('=') {
                formulas.push(at.clone());
            }
            sheet.cells.entry(at).or_default().input = t;
        }
    }
    for (a, alias) in aliases {
        let at = addr(a)?;
        let alias = alias.trim();
        if alias.is_empty() {
            if let Some(c) = sheet.cells.get_mut(&at) {
                c.alias = None;
            }
            continue;
        }
        check_alias(alias)?;
        // An alias names one cell: moving it takes it off the other.
        for (other, c) in sheet.cells.iter_mut() {
            if *other != at && c.alias.as_deref() == Some(alias) {
                c.alias = None;
            }
        }
        sheet.cells.entry(at).or_default().alias = Some(alias.to_string());
    }
    for (a, unit) in units {
        let at = addr(a)?;
        let u = unit.trim();
        if u.is_empty() {
            if let Some(c) = sheet.cells.get_mut(&at) {
                c.unit = None;
            }
            continue;
        }
        let (_, shown) = unit_factor(u).ok_or_else(|| bad(format!("unit “{u}”: use mm, cm, m, in, ft or deg")))?;
        sheet.cells.entry(at).or_default().unit = Some(if shown == "°" { "deg".into() } else { shown.into() });
    }
    sheet.cells.retain(|_, c| !c.input.is_empty() || c.alias.is_some() || c.unit.is_some());
    if sheet.cells.len() > 50_000 {
        return Err(bad("a sheet holds up to 50 000 cells"));
    }
    // Formulas: references to other things are stored by id; every name must resolve.
    for at in formulas {
        let input = sheet.cells[&at].input.clone();
        let stored = canonical_formula(tx.world(), &input, &sheet).map_err(|e| bad(format!("{at}: {e}")))?;
        sheet.cells.get_mut(&at).unwrap().input = stored;
    }
    let id = match id {
        Some(id) => id,
        None => tx.create(),
    };
    let after = references(&sheet);
    for r in &after {
        if *r == id || params::depends_on(tx, *r, id, 0) {
            return Err(bad("these formulas would make a loop: something they read already depends on this sheet"));
        }
    }
    let before: Vec<StableId> = tx.get::<Sheet>(id).map(references).unwrap_or_default();
    for r in before.iter().filter(|r| !after.contains(r)) {
        tx.remove_dependency(id, *r)?;
    }
    for r in &after {
        tx.add_dependency(id, *r)?;
    }
    tx.set(id, sheet.clone())?;
    Ok((id, sheet))
}

fn summary(world: &World, id: StableId, sheet: &Sheet) -> Value {
    let v = evaluate(world, sheet);
    let errors: BTreeMap<&String, &String> = v.cells.iter().filter_map(|(a, c)| c.error.as_ref().map(|e| (a, e))).collect();
    json!({ "id": id, "name": sheet.name, "cells": v.cells.len(), "aliases": v.aliases, "errors": errors })
}

/// `sheet.create`: a new spreadsheet.
#[derive(Deserialize, JsonSchema)]
pub struct CreateSheet {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// What to type into cells: { "A1": "Width", "B1": "200 mm", "B2": "=B1/2" }.
    #[serde(default)]
    pub cells: BTreeMap<String, Option<String>>,
    /// Names for cells: { "B1": "width" }.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    /// Units to show cells in: { "B1": "in" }.
    #[serde(default)]
    pub units: BTreeMap<String, String>,
}

impl Command for CreateSheet {
    const NAME: &'static str = "sheet.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let sheet = Sheet { name: self.name.trim().to_string(), description: self.description, cells: BTreeMap::new() };
        let (id, sheet) = apply_edits(tx, None, sheet, &self.cells, &self.aliases, &self.units)?;
        Ok(summary(tx.world(), id, &sheet))
    }
}

/// `sheet.set`: type into cells, name them, set their units ("" or null clears).
#[derive(Deserialize, JsonSchema)]
pub struct SetSheet {
    pub target: StableId,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub cells: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub units: BTreeMap<String, String>,
    /// Empty the whole sheet first.
    #[serde(default)]
    pub clear: bool,
}

impl Command for SetSheet {
    const NAME: &'static str = "sheet.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut sheet = tx.get::<Sheet>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a sheet", self.target)))?;
        if let Some(n) = self.name {
            sheet.name = n.trim().to_string();
        }
        if let Some(d) = self.description {
            sheet.description = d;
        }
        if self.clear {
            sheet.cells.clear();
        }
        let (id, sheet) = apply_edits(tx, Some(self.target), sheet, &self.cells, &self.aliases, &self.units)?;
        Ok(summary(tx.world(), id, &sheet))
    }
}

/// `sheet.import`: fills a sheet from a CSV or XLSX file (a new sheet unless
/// `target` names one). Workbook formulas come in as formulas where Forma
/// reads them, else as the value the workbook last worked out (listed in
/// `notes`); the workbook's names for single cells become aliases.
#[derive(Deserialize, JsonSchema)]
pub struct ImportSheet {
    #[serde(default)]
    pub target: Option<StableId>,
    /// Name of the new sheet (default: "Sheet N").
    #[serde(default)]
    pub name: Option<String>,
    /// "csv" or "xlsx".
    pub format: String,
    /// The file's text (CSV).
    #[serde(default)]
    pub text: Option<String>,
    /// The file's bytes as base64 (XLSX, or CSV).
    #[serde(default)]
    pub base64: Option<String>,
    /// Which worksheet of a workbook (default: the first).
    #[serde(default)]
    pub sheet: Option<String>,
    /// Top-left cell to put it at (default A1).
    #[serde(default)]
    pub at: Option<String>,
    /// Empty an existing sheet first.
    #[serde(default)]
    pub replace: bool,
}

impl Command for ImportSheet {
    const NAME: &'static str = "sheet.import";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let bytes = table::payload(self.text.as_deref(), self.base64.as_deref()).map_err(bad)?;
        let t = table::read_table(&self.format, &bytes, self.sheet.as_deref()).map_err(bad)?;
        if t.cells.is_empty() {
            return Err(bad("the file has no cells with anything in them"));
        }
        let (dc, dr) = match &self.at {
            Some(a) => table::parse_address(a).ok_or_else(|| bad(format!("“{a}” is not a cell address")))?,
            None => (0, 0),
        };
        let mut sheet = match self.target {
            Some(id) => tx.get::<Sheet>(id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a sheet")))?,
            None => {
                let name = self.name.clone().unwrap_or_else(|| {
                    let used: HashSet<String> = tx.world().resource::<IdIndex>().iter().filter_map(|(_, e)| tx.world().get::<Sheet>(e).map(|s| s.name.clone())).collect();
                    (1..).map(|n| format!("Sheet {n}")).find(|n| !used.contains(n)).unwrap()
                });
                Sheet { name, description: String::new(), cells: BTreeMap::new() }
            }
        };
        if self.replace {
            sheet.cells.clear();
        }
        // Plain values and aliases first, so formulas can resolve against them.
        let mut notes = Vec::new();
        let mut formulas = Vec::new();
        for ((c, r), cell) in &t.cells {
            let at = table::address(c + dc, r + dr);
            if cell.input.starts_with('=') {
                formulas.push((at, table::shift_formula(&cell.input, dc as i64, dr as i64), cell.cached.clone()));
            } else {
                sheet.cells.entry(at).or_default().input = cell.input.clone();
            }
        }
        for (alias, (c, r)) in &t.names {
            if check_alias(alias).is_ok() {
                for x in sheet.cells.values_mut() {
                    if x.alias.as_deref() == Some(alias.as_str()) {
                        x.alias = None;
                    }
                }
                sheet.cells.entry(table::address(c + dc, r + dr)).or_default().alias = Some(alias.clone());
            } else {
                notes.push(format!("the name “{alias}” is not a plain name; left out"));
            }
        }
        for (at, f, _) in &formulas {
            sheet.cells.entry(at.clone()).or_default().input = f.clone();
        }
        for (at, f, cached) in formulas {
            match canonical_formula(tx.world(), &f, &sheet) {
                Ok(stored) => sheet.cells.get_mut(&at).unwrap().input = stored,
                Err(e) => {
                    let keep = cached.clone().unwrap_or_default();
                    notes.push(format!("{at}: “{f}” kept as its value {} ({e})", if keep.is_empty() { "(none)".to_string() } else { keep.clone() }));
                    sheet.cells.get_mut(&at).unwrap().input = keep;
                }
            }
        }
        sheet.cells.retain(|_, c| !c.input.is_empty() || c.alias.is_some() || c.unit.is_some());
        let none = BTreeMap::new();
        let (id, sheet) = apply_edits(tx, self.target, sheet, &BTreeMap::new(), &none, &none)?;
        let mut out = summary(tx.world(), id, &sheet);
        out["notes"] = json!(notes);
        Ok(out)
    }
}

// ---- Export -------------------------------------------------------------------------------

/// The grid as text, as each cell shows (numbers with their units).
pub fn rows(sheet: &Sheet, values: &SheetValues) -> Vec<Vec<String>> {
    let at: Vec<(u32, u32)> = sheet.cells.keys().filter_map(|a| table::parse_address(a)).collect();
    let rows = at.iter().map(|(_, r)| r + 1).max().unwrap_or(0);
    let cols = at.iter().map(|(c, _)| c + 1).max().unwrap_or(0);
    (0..rows)
        .map(|r| (0..cols).map(|c| values.cells.get(&table::address(c, r)).map(|v| v.shown.clone()).unwrap_or_default()).collect())
        .collect()
}

/// Functions Excel knows under another name, or not at all.
fn excel_formula(f: &str) -> Option<String> {
    if f.contains('#') {
        return None; // reads something outside the sheet
    }
    let mut out = f.to_string();
    for (ours, theirs) in [("avg(", "AVERAGE("), ("log(", "LOG10("), ("pow(", "POWER(")] {
        out = out.replace(ours, theirs);
    }
    if out.to_lowercase().contains("hypot(") {
        return None;
    }
    // Units in a formula are millimetres there; Excel has none.
    let has_unit = out.char_indices().any(|(i, c)| c.is_ascii_digit() && ["mm", "cm", "m", "in", "ft", "deg"].iter().any(|u| out[i + 1..].trim_start().starts_with(u) && !out[i + 1..].trim_start()[u.len()..].starts_with(|ch: char| ch.is_ascii_alphanumeric())));
    if has_unit { None } else { Some(out) }
}

/// The sheet as an Excel worksheet: numbers in millimetres and degrees,
/// formulas that only read the sheet as formulas, the rest as values;
/// aliases as defined names.
pub fn to_xlsx(sheet: &Sheet, values: &SheetValues) -> Vec<u8> {
    let mut cells = BTreeMap::new();
    let mut names = BTreeMap::new();
    for (at, cell) in &sheet.cells {
        let Some(pos) = table::parse_address(at) else { continue };
        if let Some(a) = &cell.alias {
            names.insert(a.clone(), pos);
        }
        let Some(v) = values.cells.get(at) else { continue };
        let out = match (cell.input.trim_start().strip_prefix('='), v.value, &v.text) {
            (Some(f), value, _) => match excel_formula(f) {
                Some(xf) => OutCell::Formula(xf, value),
                None => match value {
                    Some(x) => OutCell::Number(x),
                    None => OutCell::Text(v.shown.clone()),
                },
            },
            (None, Some(x), _) => OutCell::Number(x),
            (None, None, Some(t)) => OutCell::Text(t.clone()),
            _ => OutCell::Text(v.shown.clone()),
        };
        cells.insert(pos, out);
    }
    table::write_xlsx(&[OutSheet { name: sheet.name.clone(), cells, names, bold_rows: vec![] }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        let n = |s: &str, u: Option<&str>| match literal(s, u) {
            Literal::Number(v, u) => Some((v, u)),
            Literal::Text(_) => None,
        };
        assert_eq!(n("200", None), Some((200.0, "")));
        assert_eq!(n("200 mm", None), Some((200.0, "mm")));
        assert_eq!(n("1.2m", None), Some((1200.0, "m")));
        assert_eq!(n("2in", None), Some((50.8, "in")));
        assert_eq!(n("-1.5e2", None), Some((-150.0, "")));
        assert_eq!(n("45°", None), Some((45.0, "°")));
        assert_eq!(n("50%", None), Some((0.5, "")));
        assert_eq!(n("2", Some("in")), Some((50.8, "in")));
        assert_eq!(n("'200", None), None);
        assert_eq!(n("Width", None), None);
        assert_eq!(n("3 apples", None), None);
        assert_eq!(shown(50.8, "in"), "2 in");
        assert_eq!(shown(1200.0, "m"), "1.2 m");
        assert_eq!(shown(1.0 / 3.0, ""), "0.333333");
        assert_eq!(shown(45.0, "°"), "45°");
    }

    #[test]
    fn ranges_and_names() {
        let mut s = Sheet { name: "S".into(), description: String::new(), cells: BTreeMap::new() };
        for (a, v) in [("B1", "1"), ("B2", "x"), ("B3", "=B1*2"), ("C1", "5")] {
            s.cells.insert(a.into(), Cell { input: v.into(), ..Default::default() });
        }
        assert_eq!(expand_ranges("sum(B1:B4)", &s).unwrap(), "sum(B1,B3)");
        assert_eq!(expand_ranges("SUM($B$1:C1)+1", &s).unwrap(), "SUM(B1,C1)+1");
        assert!(expand_ranges("sum(D1:D9)", &s).is_err());
        assert_eq!(replace_name("Desk.width+Desk.w*2", "Desk.w", "#x.w"), "Desk.width+#x.w*2");
        assert!(is_address("B2") && !is_address("b2") && !is_address("$B$2") && !is_address("width"));
        assert!(check_alias("width").is_ok() && check_alias("AB12").is_err() && check_alias("2x").is_err());
        assert_eq!(excel_formula("avg(B1,B2)").as_deref(), Some("AVERAGE(B1,B2)"));
        assert_eq!(excel_formula("B1+5mm"), None);
        assert_eq!(excel_formula("B1*2").as_deref(), Some("B1*2"));
    }
}
