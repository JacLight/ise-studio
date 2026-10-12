//! Parameter sets in and out of tables (CSV, XLSX).

use crate::{Param, ParameterSet, ResolvedParams, check_params, params, sheet, store_params, table};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, bevy_ecs::world::World, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;

/// `parameters.import`: fills a parameter set from a CSV or XLSX table (a new
/// set named `name` unless `target` names one). With a header row, columns
/// are found by name (key or name, label, value, unit, min, max, step,
/// formula); without, they are key, value, unit. Values may carry units
/// (`200 mm`, `2 in`, `45°`); keys that are not plain names are made into
/// one and the original kept as the label. Existing parameters take the new
/// values (a linked one keeps its formula); new ones are added.
#[derive(Deserialize, JsonSchema)]
pub struct ImportParameters {
    #[serde(default)]
    pub target: Option<StableId>,
    #[serde(default)]
    pub name: Option<String>,
    /// "csv" or "xlsx".
    pub format: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub base64: Option<String>,
    #[serde(default)]
    pub sheet: Option<String>,
}

/// Rows of a table as parameters (key, label, value, unit, min, max, step, formula).
pub fn params_from_rows(rows: &[Vec<String>]) -> Result<Vec<Param>, String> {
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect();
    let first = rows.first().ok_or("the table is empty")?;
    let names: Vec<String> = first.iter().map(|c| c.trim().to_lowercase()).collect();
    let col = |want: &[&str]| names.iter().position(|n| want.contains(&n.as_str()));
    let header = col(&["key", "name", "parameter"]).is_some() && col(&["value"]).is_some();
    let (k, l, v, u, mn, mx, st, fx) = if header {
        (col(&["key", "parameter", "name"]).unwrap(), col(&["label"]), col(&["value"]).unwrap(), col(&["unit"]), col(&["min"]), col(&["max"]), col(&["step"]), col(&["formula", "expr", "expression"]))
    } else {
        (0, None, 1, Some(2), None, None, None, None)
    };
    let mut out: Vec<Param> = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(header as usize) {
        let cell = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let Some(raw) = cell(Some(k)) else { continue };
        let mut key: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>().trim_matches('_').to_string();
        if key.is_empty() || key.starts_with(|c: char| c.is_ascii_digit()) {
            key = format!("p_{key}");
        }
        let unit_col = cell(u).filter(|x| x != "count");
        let counted = cell(u).as_deref() == Some("count");
        let num = |s: &str| sheet::number(s, unit_col.as_deref());
        let value_text = cell(Some(v)).ok_or_else(|| format!("row {}: {raw} has no value", i + 1))?;
        let (value, kind) = num(&value_text).ok_or_else(|| format!("row {}: “{value_text}” is not a number", i + 1))?;
        let unit = if counted {
            "count"
        } else {
            match kind {
                sheet::Kind::Length => "mm",
                sheet::Kind::Angle => "deg",
                sheet::Kind::Plain => "",
            }
        };
        let bound = |c: Option<usize>| -> Result<Option<f64>, String> { cell(c).map(|s| num(&s).map(|(x, _)| x).ok_or_else(|| format!("row {}: “{s}” is not a number", i + 1))).transpose() };
        if out.iter().any(|p| p.key == key) {
            return Err(format!("row {}: “{raw}” appears twice", i + 1));
        }
        out.push(Param {
            label: cell(l).unwrap_or_else(|| raw.clone()),
            key,
            value,
            unit: unit.into(),
            min: bound(mn)?,
            max: bound(mx)?,
            step: bound(st)?,
            expr: cell(fx).map(|f| if f.starts_with('=') { f } else { format!("={f}") }),
        });
    }
    if out.is_empty() {
        return Err("the table has no parameters".into());
    }
    Ok(out)
}

impl Command for ImportParameters {
    const NAME: &'static str = "parameters.import";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let invalid = |e: String| CommandError::new(ErrorCode::InvalidParameters, e);
        let bytes = table::payload(self.text.as_deref(), self.base64.as_deref()).map_err(invalid)?;
        let t = table::read_table(&self.format, &bytes, self.sheet.as_deref()).map_err(invalid)?;
        let incoming = params_from_rows(&t.rows()).map_err(invalid)?;
        let Some(target) = self.target else {
            let name = self.name.clone().filter(|n| !n.trim().is_empty()).ok_or_else(|| invalid("name the new parameter set (`name`) or give `target`".into()))?;
            let n = incoming.len();
            let out = tx.run("parameters.create", json!({ "name": name, "params": incoming }))?;
            return Ok(json!({ "id": out["id"], "added": n, "updated": 0 }));
        };
        let set = tx.get::<ParameterSet>(target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a parameter set")))?;
        let old = set.params.clone();
        let mut list = set.params.clone();
        let (mut added, mut updated) = (0, 0);
        for p in incoming {
            match list.iter_mut().find(|q| q.key == p.key) {
                Some(q) => {
                    if q.expr.is_none() {
                        q.value = p.value;
                    }
                    if p.expr.is_some() {
                        q.expr = p.expr;
                    }
                    q.min = p.min.or(q.min);
                    q.max = p.max.or(q.max);
                    updated += 1;
                }
                None => {
                    list.push(p);
                    added += 1;
                }
            }
        }
        check_params(&list)?;
        params::link(tx, target, &old, &mut list)?;
        store_params(tx, target, list)?;
        Ok(json!({ "id": target, "added": added, "updated": updated }))
    }
}

/// A parameter set as rows: key, label, value, unit, min, max, step, formula.
pub fn parameter_rows(world: &World, set: &ParameterSet, resolved: Option<&ResolvedParams>) -> Vec<Vec<String>> {
    let num = |v: Option<f64>| v.map(|x| format!("{}", (x * 1e9).round() / 1e9)).unwrap_or_default();
    let mut rows = vec![["key", "label", "value", "unit", "min", "max", "step", "formula"].map(String::from).to_vec()];
    for p in &set.params {
        let value = resolved.and_then(|r| r.0.get(&p.key).copied()).unwrap_or(p.value);
        rows.push(vec![p.key.clone(), p.label.clone(), num(Some(value)), p.unit.clone(), num(p.min), num(p.max), num(p.step), p.expr.as_deref().map(|f| params::display(world, f)).unwrap_or_default()]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(t: &str) -> Vec<Vec<String>> {
        table::read_csv(t)
    }

    #[test]
    fn with_header() {
        let ps = params_from_rows(&rows("Key,Value,Unit,Min,Max,Label\nwidth,200,mm,100,400,Overall width\nangle,30,deg,,,\ncount,4,count,,,\n")).unwrap();
        assert_eq!(ps.len(), 3);
        assert_eq!((ps[0].key.as_str(), ps[0].value, ps[0].unit.as_str(), ps[0].min, ps[0].max, ps[0].label.as_str()), ("width", 200.0, "mm", Some(100.0), Some(400.0), "Overall width"));
        assert_eq!((ps[1].unit.as_str(), ps[2].unit.as_str()), ("deg", "count"));
    }

    #[test]
    fn plain_rows_with_units() {
        let ps = params_from_rows(&rows("Desk width,1.6 m\nleg,2 in\nshelf count,3\n")).unwrap();
        assert_eq!((ps[0].key.as_str(), ps[0].label.as_str(), ps[0].value, ps[0].unit.as_str()), ("Desk_width", "Desk width", 1600.0, "mm"));
        assert_eq!(ps[1].value, 50.8);
        assert_eq!(ps[2].unit, "");
        assert!(params_from_rows(&rows("a,1\na,2\n")).unwrap_err().contains("twice"));
        assert!(params_from_rows(&rows("a,lots\n")).unwrap_err().contains("not a number"));
    }
}
