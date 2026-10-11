//! Configurations: named sets of parameter values for one shape ("M6×20",
//! "M8×30"; "S", "M", "L"), kept as a table whose columns are the parameters
//! it sets and whose rows are the configurations.
//!
//! Nothing is implicit. Activating a configuration writes its values into the
//! shape's parameters (one journaled, undoable change); an instance in an
//! assembly can name a configuration of its definition and is evaluated with
//! that row's values, so one definition placed three times can show three
//! sizes. A parameter driven by a formula stays driven: the table cannot set it.
//! A row can also leave out finishes (fillets, chamfers, shells…), named by
//! the parameter that sizes them.

use crate::{Param, ShapeProgram, ShapeStats, params};
use de_core::{
    Command, CommandError, ErrorCode, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    json, schemars,
};
use de_solid::{DisplayMesh, MassProperties};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One configuration: a value for every column of the table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigRow {
    /// What people call it: "M8×30", "Large".
    pub name: String,
    /// Parameter key → value, one for each column.
    pub values: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Finishes left off in this configuration, by the parameter that sizes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppress: Vec<String>,
}

/// A shape's configuration table.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShapeConfigurations {
    /// The parameters the table sets (its columns), by key, in column order.
    pub params: Vec<String>,
    pub rows: Vec<ConfigRow>,
    /// The configuration the shape itself shows (its values are written into
    /// the shape's parameters when it is activated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
}

impl Persist for ShapeConfigurations {
    const KIND: &'static str = "shape.configurations";
}

impl ShapeConfigurations {
    pub fn row(&self, name: &str) -> Option<&ConfigRow> {
        self.rows.iter().find(|r| r.name == name)
    }

    pub fn active_row(&self) -> Option<&ConfigRow> {
        self.active.as_deref().and_then(|n| self.row(n))
    }

    /// The configuration names, for messages: “S”, “M” or “L”.
    pub fn names(&self) -> String {
        let n: Vec<String> = self.rows.iter().map(|r| format!("“{}”", r.name)).collect();
        match n.len() {
            0 => "none".into(),
            1 => n[0].clone(),
            k => format!("{} or {}", n[..k - 1].join(", "), n[k - 1]),
        }
    }
}

/// How a shape is named in a configuration: “Bolt (M8×30)”.
pub fn display_name(name: &str, config: Option<&str>) -> String {
    match config {
        Some(c) if !c.is_empty() => format!("{name} ({c})"),
        _ => name.to_string(),
    }
}

fn invalid(m: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, m)
}

/// Why a linked parameter cannot be a column, in words people can act on.
pub fn linked_refusal(world: &World, p: &Param) -> String {
    let f = p.expr.as_deref().map(|f| params::display(world, f)).unwrap_or_default();
    format!(
        "{} follows {f}: a linked parameter is driven by its formula, so a configuration cannot set it. Unlink it (shape.link_param with no formula) or leave it out of the table",
        p.label
    )
}

/// Checks a table against the shape's parameters: columns are plain
/// (unlinked) parameters, every row names each column once with a value in
/// range, names are unique, and the active configuration is one of the rows.
pub fn check_table(world: &World, program: &ShapeProgram, table: &ShapeConfigurations) -> Result<(), CommandError> {
    let mut seen = std::collections::HashSet::new();
    for key in &table.params {
        let p = program.params.iter().find(|p| &p.key == key).ok_or_else(|| {
            invalid(format!("“{key}” is a column of the configuration table, but {} has no such parameter: take it out of the table (shape.config.set_table)", program.name))
        })?;
        if p.expr.is_some() {
            return Err(invalid(linked_refusal(world, p)));
        }
        if !seen.insert(key) {
            return Err(invalid(format!("“{key}” is a column twice")));
        }
    }
    if !table.rows.is_empty() && table.params.is_empty() {
        return Err(invalid("a configuration table needs at least one parameter as a column"));
    }
    let mut names = std::collections::HashSet::new();
    for row in &table.rows {
        let name = row.name.trim();
        if name.is_empty() || name != row.name {
            return Err(invalid("a configuration needs a name without leading or trailing spaces"));
        }
        if name.chars().count() > 60 {
            return Err(invalid(format!("“{name}” is a long name: keep configuration names to 60 characters")));
        }
        if !names.insert(name) {
            return Err(invalid(format!("two configurations are called “{name}”")));
        }
        for key in row.values.keys() {
            if !table.params.contains(key) {
                let linked = program.params.iter().find(|p| &p.key == key && p.expr.is_some());
                return Err(invalid(match linked {
                    Some(p) => format!("{name}: {}", linked_refusal(world, p)),
                    None => format!("{name} sets “{key}”, which is not a column of the table"),
                }));
            }
        }
        for key in &table.params {
            let p = program.params.iter().find(|p| &p.key == key).unwrap();
            let v = *row.values.get(key).ok_or_else(|| invalid(format!("{name} has no value for {}", p.label)))?;
            if !v.is_finite() {
                return Err(invalid(format!("{name}: {} must be a finite number", p.label)));
            }
            if p.min.is_some_and(|m| v < m) || p.max.is_some_and(|m| v > m) {
                return Err(invalid(format!(
                    "{name}: {} is {v}, outside {}–{}",
                    p.label,
                    p.min.map(|m| m.to_string()).unwrap_or("−∞".into()),
                    p.max.map(|m| m.to_string()).unwrap_or("∞".into())
                )));
            }
        }
        for key in &row.suppress {
            if !program.finishes.iter().any(|f| &f.key == key) {
                return Err(invalid(format!("{name} leaves off the finish sized by “{key}”, but no finish of {} is", program.name)));
            }
        }
    }
    if let Some(a) = &table.active
        && table.row(a).is_none()
    {
        return Err(invalid(format!("there is no configuration “{a}” (there is {})", table.names())));
    }
    Ok(())
}

/// Writes a row's values into a program's parameters (linked ones stay driven).
pub fn apply_row(program: &mut ShapeProgram, row: &ConfigRow) {
    for p in &mut program.params {
        if p.expr.is_none()
            && let Some(v) = row.values.get(&p.key)
        {
            p.value = *v;
        }
    }
}

/// Leaves off the finishes a row suppresses.
pub fn suppress_finishes(program: &mut ShapeProgram, row: &ConfigRow) {
    if !row.suppress.is_empty() {
        program.finishes.retain(|f| !row.suppress.contains(&f.key));
    }
}

/// A shape's program in one of its configurations, ready to evaluate: the
/// row's values in place, formulas worked out from them (a formula reading a
/// configured parameter follows it), its suppressed finishes left off, and
/// named “Bolt (M8×30)”.
pub fn configured_program(world: &World, e: Entity, config: &str) -> Result<ShapeProgram, String> {
    let mut program = world.get::<ShapeProgram>(e).ok_or("not a shape")?.clone();
    let table = world.get::<ShapeConfigurations>(e);
    let row = table
        .and_then(|t| t.row(config))
        .ok_or_else(|| format!("{} has no configuration “{config}” (it has {})", program.name, table.map(|t| t.names()).unwrap_or_else(|| "none".into())))?;
    apply_row(&mut program, row);
    suppress_finishes(&mut program, row);
    let values = params::resolve(world, &program.params)?;
    for p in &mut program.params {
        if let Some(v) = values.get(&p.key) {
            p.value = *v;
        }
    }
    program.inputs = crate::program_inputs(world, &program)?;
    program.name = display_name(&program.name, Some(config));
    Ok(program)
}

/// A shape evaluated in one configuration: what an instance that names it shows.
#[derive(Clone, Debug, Serialize)]
pub struct Configured {
    pub config: String,
    /// “Bolt (M8×30)”.
    pub name: String,
    pub stats: ShapeStats,
    pub mesh: DisplayMesh,
    pub mass: MassProperties,
    /// Mass (kg) of each part, in the order of `stats.parts`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub part_masses: Vec<Option<f64>>,
}

/// Evaluates shape `e` in configuration `config` (cached by program, so
/// three instances of one size evaluate once).
pub fn evaluate(world: &World, e: Entity, config: &str) -> Result<Configured, String> {
    let program = configured_program(world, e, config)?;
    let ev = crate::evaluated_at(&program, program.display_tolerance())?;
    let stats = crate::stats_of(&ev);
    let masses = crate::masses_of(world, e, &stats)?;
    let mass = crate::mass_of(&stats, &masses);
    Ok(Configured {
        config: config.to_string(),
        name: program.name.clone(),
        mesh: crate::mesh_of(&ev.positions, &ev.normals, &ev.indices),
        part_masses: if stats.parts.is_empty() { vec![] } else { masses },
        stats,
        mass,
    })
}

/// Applies a table to a shape: checks it, checks that every configuration
/// still makes a valid solid, and, when one is active, writes its values into
/// the shape's parameters. An empty table removes it.
pub fn store_table(tx: &mut Tx, target: StableId, table: ShapeConfigurations) -> Result<(), CommandError> {
    let mut program = tx.get::<ShapeProgram>(target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a shape")))?;
    check_table(tx.world(), &program, &table)?;
    let materials = tx.get::<crate::PartMaterials>(target).cloned().unwrap_or_default();
    for row in &table.rows {
        let mut p = program.clone();
        apply_row(&mut p, row);
        suppress_finishes(&mut p, row);
        crate::validate(tx.world(), &p, &materials).map_err(|e| CommandError::new(e.code, format!("{}: {}", row.name, e.message)))?;
    }
    if let Some(row) = table.active_row() {
        apply_row(&mut program, row);
        if tx.get::<ShapeProgram>(target) != Some(&program) {
            tx.set(target, program)?;
        }
    }
    if table.rows.is_empty() && table.params.is_empty() {
        tx.remove::<ShapeConfigurations>(target)
    } else {
        tx.set(target, table)
    }
}

/// `shape.config.activate`: show one configuration of a shape (its values are
/// written into the shape's parameters); no name keeps the values and leaves
/// no configuration active.
#[derive(Deserialize, JsonSchema)]
pub struct ActivateConfig {
    pub target: StableId,
    pub name: Option<String>,
}

impl Command for ActivateConfig {
    const NAME: &'static str = "shape.config.activate";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let program = tx.get::<ShapeProgram>(self.target).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a shape", self.target)))?;
        let mut table = tx
            .get::<ShapeConfigurations>(self.target)
            .cloned()
            .ok_or_else(|| invalid(format!("{} has no configurations: make a table first (shape.config.set_table)", program.name)))?;
        let name = self.name.filter(|n| !n.trim().is_empty());
        if let Some(n) = &name
            && table.row(n).is_none()
        {
            return Err(invalid(format!("{} has no configuration “{n}” (it has {})", program.name, table.names())));
        }
        table.active = name.clone();
        store_table(tx, self.target, table)?;
        Ok(json!({ "id": self.target, "active": name }))
    }
}
