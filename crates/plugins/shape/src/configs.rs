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
//! the parameter that sizes them, and parts, by name: an enclosure's “Basic”
//! without the fan and its guard, its “Pro” with them. A part left out is
//! gone from everything the shape feeds (view, mass, parts list, exports,
//! drawings, studies), and the joints and gear meshes that only it held go
//! with it, reported in `ShapeStats::left_out`. The program itself can read
//! the configuration's name as `config`.

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
    /// Parts left out of this configuration, by the name the program gives
    /// them in part(): dropped from the result wherever the shape is used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts_off: Vec<String>,
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
        let mut off = std::collections::HashSet::new();
        for part in &row.parts_off {
            if part.trim().is_empty() || part.trim() != part {
                return Err(invalid(format!("{name} leaves out a part without a name: name parts as the program does in part()")));
            }
            if !off.insert(part) {
                return Err(invalid(format!("{name} leaves out “{part}” twice")));
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

/// A program's inputs (see `program_inputs`) in configuration `row`: its
/// name, which the program reads as `config`, and the parts it leaves out.
pub fn with_row(inputs: &str, row: &ConfigRow) -> String {
    let mut v: serde_json::Value = serde_json::from_str(inputs).unwrap_or_else(|_| json!({}));
    v["config"] = json!(row.name);
    if !row.parts_off.is_empty() {
        v["parts_off"] = json!(row.parts_off);
    }
    v.to_string()
}

/// A shape's program in one of its configurations, ready to evaluate: the
/// row's values in place, formulas worked out from them (a formula reading a
/// configured parameter follows it), its suppressed finishes left off, and
/// named “Bolt (M8×30)”. The parts it leaves out are dropped when it runs.
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
    program.inputs = with_row(&crate::program_inputs(world, &program)?, row);
    program.name = display_name(&program.name, Some(config));
    Ok(program)
}

// ---- One instance's own values ---------------------------------------------------

/// A value an instance gives one parameter of the shape it shows, in place of
/// the definition's: a number, or a formula (`=Shoulder.angle`: a servo's
/// output spline following its joint). Every other instance keeps sharing the
/// definition, its program and every edit to it; only this value differs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParamOverride {
    /// The value; with a formula, the last value given by hand (kept when it is unlinked).
    #[serde(default)]
    pub value: f64,
    /// A formula that drives it (stored with references by id, as a linked parameter's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
}

/// An instance's own parameter values, by key.
pub type Overrides = BTreeMap<String, ParamOverride>;

/// Puts an instance's own values into a program's parameters (a formula
/// replaces the definition's value or formula). A key the program no longer
/// has is an error naming the ones it has.
pub fn apply_overrides(program: &mut ShapeProgram, overrides: &Overrides) -> Result<(), String> {
    for (key, o) in overrides {
        let Some(p) = program.params.iter_mut().find(|p| &p.key == key) else {
            let keys: Vec<&str> = program.params.iter().map(|p| p.key.as_str()).collect();
            return Err(format!("{} has no parameter “{key}” (it has {}): change or drop the instance's value", program.name, keys.join(", ")));
        };
        p.value = o.value;
        p.expr = o.expr.clone();
    }
    Ok(())
}

/// Checks an instance's own values against its shape and stores their
/// formulas by id: every key is a parameter of the shape, a plain value is
/// within its min/max, and formulas read things that exist (a bare key is
/// another parameter of the same shape).
pub fn check_overrides(world: &World, program: &ShapeProgram, overrides: &mut Overrides) -> Result<(), CommandError> {
    let own: Vec<String> = program.params.iter().map(|p| p.key.clone()).collect();
    for (key, o) in overrides.iter_mut() {
        let p = program.params.iter().find(|p| &p.key == key).ok_or_else(|| invalid(format!("{} has no parameter “{key}” (it has {})", program.name, own.join(", "))))?;
        match &o.expr {
            Some(f) => {
                o.expr = Some(params::canonical(world, f, &own).map_err(|e| invalid(format!("{}: {e}", p.label)))?);
            }
            None => {
                if !o.value.is_finite() {
                    return Err(invalid(format!("{} must be a finite number", p.label)));
                }
                if p.min.is_some_and(|m| o.value < m) || p.max.is_some_and(|m| o.value > m) {
                    return Err(invalid(format!(
                        "{} {} is outside {}–{}",
                        p.label,
                        o.value,
                        p.min.map(|m| m.to_string()).unwrap_or("−∞".into()),
                        p.max.map(|m| m.to_string()).unwrap_or("∞".into())
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Every entity an instance's own formulas read (for dependencies).
pub fn override_references(overrides: &Overrides) -> Vec<StableId> {
    let as_params: Vec<Param> = overrides
        .iter()
        .map(|(k, o)| Param { key: k.clone(), label: k.clone(), value: o.value, unit: String::new(), min: None, max: None, step: None, expr: o.expr.clone() })
        .collect();
    params::references(&as_params)
}

/// Shape `e`'s program as one instance shows it: in configuration `config`
/// (else as the shape is), with the instance's own values `overrides` in
/// place of the definition's, every formula worked out — those that read a
/// value `given` here (a joint at one step of a sweep) with that value — its
/// suppressed finishes left off, and named for the configuration.
pub fn variant_program(world: &World, e: Entity, config: Option<&str>, overrides: &Overrides, given: &dyn Fn(StableId, &str) -> Option<f64>) -> Result<ShapeProgram, String> {
    let mut program = world.get::<ShapeProgram>(e).ok_or("not a shape")?.clone();
    let table = world.get::<ShapeConfigurations>(e);
    let row = match config {
        Some(c) => {
            let row = table
                .and_then(|t| t.row(c))
                .ok_or_else(|| format!("{} has no configuration “{c}” (it has {})", program.name, table.map(|t| t.names()).unwrap_or_else(|| "none".into())))?;
            apply_row(&mut program, row);
            Some(row)
        }
        None => table.and_then(|t| t.active_row()),
    };
    if let Some(row) = row {
        suppress_finishes(&mut program, row);
    }
    apply_overrides(&mut program, overrides)?;
    let values = params::resolve_with(world, &program.params, given)?;
    for p in &mut program.params {
        if let Some(v) = values.get(&p.key) {
            p.value = *v;
        }
    }
    program.inputs = match config {
        Some(_) => crate::program_inputs(world, &program)?,
        None => crate::program_inputs(world, &program).unwrap_or_else(|_| format!(r#"{{"finishes":{}}}"#, serde_json::to_string(&program.finishes).unwrap())),
    };
    if let Some(row) = row {
        program.inputs = with_row(&program.inputs, row);
        program.name = display_name(&program.name, Some(&row.name));
    }
    Ok(program)
}

/// Shape `e` evaluated as an instance shows it: in configuration `config`
/// and with the instance's own values (see [`variant_program`]).
pub fn evaluate_variant(world: &World, e: Entity, config: Option<&str>, overrides: &Overrides) -> Result<Configured, String> {
    let program = variant_program(world, e, config, overrides, &|_, _| None)?;
    configured_from(world, e, config.unwrap_or_default(), program)
}

fn configured_from(world: &World, e: Entity, config: &str, program: ShapeProgram) -> Result<Configured, String> {
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
    configured_from(world, e, config, program)
}

/// Applies a table to a shape: checks it, checks that every configuration
/// still makes a valid solid (and keeps at least one part), and, when one is
/// active, writes its values into the shape's parameters. An empty table
/// removes it. A part newly left out must be one the program makes; one
/// left out before and renamed in the program since is reported where the
/// configuration is shown (`ShapeStats::left_out`), not refused here.
pub fn store_table(tx: &mut Tx, target: StableId, table: ShapeConfigurations) -> Result<(), CommandError> {
    let mut program = tx.get::<ShapeProgram>(target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{target} is not a shape")))?;
    check_table(tx.world(), &program, &table)?;
    let materials = tx.get::<crate::PartMaterials>(target).cloned().unwrap_or_default();
    let old = tx.get::<ShapeConfigurations>(target).cloned().unwrap_or_default();
    for row in &table.rows {
        let mut p = program.clone();
        apply_row(&mut p, row);
        suppress_finishes(&mut p, row);
        let e = crate::validate_in(tx.world(), &p, &materials, Some(row)).map_err(|e| CommandError::new(e.code, format!("{}: {}", row.name, e.message)))?;
        let before = old.row(&row.name).map(|r| r.parts_off.as_slice()).unwrap_or_default();
        let new: Vec<String> = e.left_out.missing.iter().filter(|m| !before.contains(m)).map(|n| format!("“{n}”")).collect();
        if !new.is_empty() {
            return Err(invalid(if e.parts.is_empty() {
                format!("{}: {} makes one solid, not parts, so there is no part to leave out", row.name, program.name)
            } else {
                let made: Vec<String> = if e.left_out.all.is_empty() { e.parts.iter().map(|p| p.name.clone()).collect() } else { e.left_out.all.clone() };
                let made: Vec<String> = made.iter().map(|n| format!("“{n}”")).collect();
                format!("{}: {} makes no part called {} (its parts are {})", row.name, program.name, new.join(", "), made.join(", "))
            }));
        }
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
