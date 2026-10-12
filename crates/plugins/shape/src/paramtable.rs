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

/// A configuration table as rows: configuration, one column per parameter
/// (by key), description, suppress and parts_off (lists joined by "; ").
pub fn config_rows(table: &crate::ShapeConfigurations) -> Vec<Vec<String>> {
    let mut head = vec!["configuration".to_string()];
    head.extend(table.params.iter().cloned());
    head.extend(["description", "suppress", "parts_off"].map(String::from));
    let mut rows = vec![head];
    for r in &table.rows {
        let mut row = vec![r.name.clone()];
        row.extend(table.params.iter().map(|k| r.values.get(k).map(|v| format!("{}", (v * 1e9).round() / 1e9)).unwrap_or_default()));
        row.extend([r.description.clone(), r.suppress.join("; "), r.parts_off.join("; ")]);
        rows.push(row);
    }
    rows
}

/// Rows (with a header) as a configuration table for a shape whose
/// parameters are `params`: columns are matched to parameters by key or label.
pub fn config_from_rows(rows: &[Vec<String>], params: &[Param]) -> Result<(Vec<String>, Vec<crate::ConfigRow>), String> {
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect();
    let head = rows.first().ok_or("the table is empty")?;
    let mut name_col = None;
    let (mut desc, mut supp, mut off) = (None, None, None);
    let mut cols: Vec<(usize, &Param)> = Vec::new();
    for (i, h) in head.iter().enumerate() {
        let h = h.trim();
        let l = h.to_lowercase();
        match l.as_str() {
            "configuration" | "name" | "config" | "size" if name_col.is_none() => name_col = Some(i),
            "description" => desc = Some(i),
            "suppress" => supp = Some(i),
            "parts_off" | "parts off" => off = Some(i),
            "" => {}
            _ => match params.iter().find(|p| p.key == h).or_else(|| params.iter().find(|p| p.label.eq_ignore_ascii_case(h))) {
                Some(p) => cols.push((i, p)),
                None if name_col.is_none() && i == 0 => name_col = Some(0),
                None => return Err(format!("column “{h}” is not a parameter of the shape (it has {})", params.iter().map(|p| p.key.as_str()).collect::<Vec<_>>().join(", "))),
            },
        }
    }
    let name_col = name_col.ok_or("the first column must name the configurations (head it “configuration”)")?;
    if cols.is_empty() {
        return Err("no column names a parameter of the shape".into());
    }
    let list = |s: Option<&String>| s.map(|x| x.split(';').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for (n, r) in rows.iter().enumerate().skip(1) {
        let name = r.get(name_col).map(|s| s.trim().to_string()).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let mut values = std::collections::BTreeMap::new();
        for (i, p) in &cols {
            let t = r.get(*i).map(|s| s.trim()).unwrap_or("");
            let unit = match p.unit.as_str() {
                "mm" => Some("mm"),
                "deg" => Some("deg"),
                _ => None,
            };
            let (v, _) = sheet::number(t, unit).ok_or_else(|| format!("row {} ({name}): {} “{t}” is not a number", n + 1, p.key))?;
            values.insert(p.key.clone(), v);
        }
        out.push(crate::ConfigRow { name, values, description: desc.and_then(|i| r.get(i)).map(|s| s.trim().to_string()).unwrap_or_default(), suppress: list(supp.and_then(|i| r.get(i))), parts_off: list(off.and_then(|i| r.get(i))) });
    }
    if out.is_empty() {
        return Err("the table has no configurations".into());
    }
    Ok((cols.iter().map(|(_, p)| p.key.clone()).collect(), out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_tables() {
        let ps: Vec<Param> = serde_json::from_value(serde_json::json!([
            { "key": "d", "label": "Diameter", "value": 6, "unit": "mm", "min": null, "max": null, "step": null },
            { "key": "len", "label": "Length", "value": 20, "unit": "mm", "min": null, "max": null, "step": null }
        ]))
        .unwrap();
        let rows = table::read_csv("configuration,Diameter,len,parts_off\nM6×20,6,20,\nM8×30,8 mm,1.25in,Washer; Nut\n");
        let (cols, rs) = config_from_rows(&rows, &ps).unwrap();
        assert_eq!(cols, ["d", "len"]);
        assert_eq!(rs[1].values["len"], 31.75);
        assert_eq!(rs[1].parts_off, ["Washer", "Nut"]);
        let t = crate::ShapeConfigurations { params: cols, rows: rs, active: None };
        let back = config_rows(&t);
        assert_eq!(back[0], ["configuration", "d", "len", "description", "suppress", "parts_off"]);
        assert_eq!(config_from_rows(&back, &ps).unwrap().1, t.rows);
        assert!(config_from_rows(&table::read_csv("configuration,width\nA,1\n"), &ps).unwrap_err().contains("width"));
    }

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
