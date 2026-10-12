//! Editing a shape's configuration table where it reaches instances: a
//! renamed configuration is renamed on every instance showing it, and one an
//! instance still shows cannot be deleted from under it.

use crate::{Instance, bad};
use de_core::{Command, CommandError, ErrorCode, IdIndex, StableId, Tx, Value, json, schemars};
use de_shape::{ConfigRow, ShapeConfigurations, ShapeProgram};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

/// `shape.config.set_table`: create, edit or (with no rows and no columns)
/// remove a shape's configuration table. Columns are parameter keys; each
/// row gives every column a value. Linked parameters cannot be columns.
#[derive(Deserialize, JsonSchema)]
pub struct SetConfigTable {
    pub target: StableId,
    /// The parameters the table sets, by key, in column order.
    pub params: Vec<String>,
    pub rows: Vec<ConfigRow>,
    /// The configuration the shape shows (its values are written into the
    /// shape's parameters); "" for none. Left out, the active one stays
    /// (following a rename).
    pub active: Option<String>,
    /// Configurations renamed by this edit, old name → new, so instances
    /// showing them follow.
    #[serde(default)]
    pub renamed: BTreeMap<String, String>,
}

impl Command for SetConfigTable {
    const NAME: &'static str = "shape.config.set_table";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let program = tx.get::<ShapeProgram>(self.target).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a shape", self.target)))?;
        let name = program.name.clone();
        let old = tx.get::<ShapeConfigurations>(self.target).cloned().unwrap_or_default();
        let follow = |n: &String| self.renamed.get(n).cloned().unwrap_or_else(|| n.clone());
        let active = match self.active {
            Some(a) if a.is_empty() => None,
            Some(a) => Some(a),
            None => old.active.as_ref().map(follow).filter(|a| self.rows.iter().any(|r| &r.name == a)),
        };
        let table = ShapeConfigurations { params: self.params, rows: self.rows, active };

        // Every instance of this shape that shows a configuration keeps showing it.
        let users: Vec<(StableId, Instance)> = {
            let world = tx.world();
            world
                .resource::<IdIndex>()
                .iter()
                .filter_map(|(id, e)| world.get::<Instance>(e).filter(|i| i.of == self.target && i.config.is_some()).map(|i| (id, i.clone())))
                .collect()
        };
        let mut orphaned: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut moved = vec![];
        for (id, mut inst) in users {
            let was = inst.config.clone().unwrap();
            let now = follow(&was);
            if table.row(&now).is_none() {
                orphaned.entry(was).or_default().push(inst.name.clone());
            } else if now != was {
                inst.config = Some(now);
                moved.push((id, inst));
            }
        }
        if !orphaned.is_empty() {
            let list: Vec<String> = orphaned.iter().map(|(c, who)| format!("“{c}” is shown by {}", who.join(", "))).collect();
            return Err(bad(ErrorCode::InvalidParameters, format!("{} — switch those instances to another configuration first", list.join("; "))));
        }
        de_shape::configs::store_table(tx, self.target, table)?;
        for (id, inst) in moved {
            tx.set(id, inst)?;
        }
        Ok(json!({ "id": self.target, "shape": name }))
    }
}

/// `shape.config.import`: a shape's configuration table from a CSV or XLSX
/// table. The first column names the configurations; the others are
/// parameters (by key or label; values may carry units), and optionally
/// description, suppress and parts_off (lists separated by “;”). Replaces
/// the table, as `shape.config.set_table` does (instances keep showing a
/// configuration that is still there).
#[derive(Deserialize, JsonSchema)]
pub struct ImportConfigTable {
    pub target: StableId,
    /// "csv" or "xlsx".
    pub format: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub base64: Option<String>,
    #[serde(default)]
    pub sheet: Option<String>,
    /// The configuration to show ("" for none); left out, the active one stays if it is still there.
    #[serde(default)]
    pub active: Option<String>,
}

impl Command for ImportConfigTable {
    const NAME: &'static str = "shape.config.import";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let program = tx.get::<ShapeProgram>(self.target).ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{} is not a shape", self.target)))?;
        let params = program.params.clone();
        let invalid = |e: String| bad(ErrorCode::InvalidParameters, e);
        let bytes = de_shape::table::payload(self.text.as_deref(), self.base64.as_deref()).map_err(invalid)?;
        let t = de_shape::table::read_table(&self.format, &bytes, self.sheet.as_deref()).map_err(invalid)?;
        let (cols, rows) = de_shape::paramtable::config_from_rows(&t.rows(), &params).map_err(invalid)?;
        let n = rows.len();
        let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
        tx.run("shape.config.set_table", json!({ "target": self.target, "params": cols, "rows": rows, "active": self.active }))?;
        Ok(json!({ "id": self.target, "configurations": n, "names": names }))
    }
}
