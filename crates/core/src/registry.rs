use crate::{
    command::Command,
    derived::{BatchCommand, DeleteEntity, DependsOn, Derived, DerivedInfo},
    error::{CommandError, ErrorCode},
    journal::{ChangeSet, Op, WireChangeSet, WireOp},
    plugin::Plugin,
    tx::Tx,
    value::CompValue,
};
use bevy_ecs::{component::Component, entity::Entity, world::World};
use rustc_hash::FxHashMap;
use schemars::{JsonSchema, Schema};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::BTreeMap, fmt::Debug, sync::Arc};

/// A component that is part of the design (journaled, saved, synchronized).
/// Derived components are plain [`Component`]s and are never journaled.
pub trait Persist:
    Component + Clone + PartialEq + Debug + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
    /// Globally unique kind name, conventionally `"<plugin>.<component>"`.
    const KIND: &'static str;
}

pub(crate) struct ComponentInfo {
    pub kind: &'static str,
    pub capture: fn(&World, Entity) -> Option<CompValue>,
    pub write: fn(&mut World, Entity, Option<&CompValue>) -> Result<(), CommandError>,
    pub decode: fn(Value) -> Result<CompValue, String>,
}

pub struct CommandInfo {
    pub name: &'static str,
    pub plugin: &'static str,
    /// JSON Schema of the parameters; the AI agent's tool definition.
    pub schema: Schema,
    pub(crate) exec: fn(Value, &mut Tx) -> Result<Value, CommandError>,
}

pub struct Registry {
    components: Vec<ComponentInfo>,
    by_kind: FxHashMap<&'static str, usize>,
    commands: BTreeMap<&'static str, CommandInfo>,
    pub(crate) derived: Vec<DerivedInfo>,
    plugins: Vec<&'static str>,
    current_plugin: &'static str,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        let mut registry = Self {
            components: Vec::new(),
            by_kind: FxHashMap::default(),
            commands: BTreeMap::new(),
            derived: Vec::new(),
            plugins: vec!["core"],
            current_plugin: "core",
        };
        registry.component::<DependsOn>().command::<DeleteEntity>().command::<BatchCommand>();
        registry
    }

    pub fn with_plugins(plugins: &[&dyn Plugin]) -> Self {
        let mut registry = Self::new();
        for plugin in plugins {
            registry.plugin(*plugin);
        }
        registry
    }

    pub fn plugin(&mut self, plugin: &dyn Plugin) -> &mut Self {
        assert!(!self.plugins.contains(&plugin.name()), "plugin {} registered twice", plugin.name());
        self.current_plugin = plugin.name();
        plugin.register(self);
        self.plugins.push(plugin.name());
        self.current_plugin = "core";
        self
    }

    pub fn component<T: Persist>(&mut self) -> &mut Self {
        assert!(!self.by_kind.contains_key(T::KIND), "component kind {} registered twice", T::KIND);
        self.by_kind.insert(T::KIND, self.components.len());
        self.components.push(ComponentInfo {
            kind: T::KIND,
            capture: capture::<T>,
            write: write::<T>,
            decode: decode::<T>,
        });
        self
    }

    pub fn command<C: Command>(&mut self) -> &mut Self {
        assert!(!self.commands.contains_key(C::NAME), "command {} registered twice", C::NAME);
        let schema = schemars::SchemaGenerator::default().into_root_schema_for::<C>();
        self.commands.insert(
            C::NAME,
            CommandInfo { name: C::NAME, plugin: self.current_plugin, schema, exec: exec::<C> },
        );
        self
    }

    /// Derived computations run in registration order within a dependency
    /// level, so a later one may read an earlier one's output on the same entity.
    pub fn derived<Out: Component>(&mut self, spec: Derived<Out>) -> &mut Self {
        self.derived.push(DerivedInfo::new(spec));
        self
    }

    pub fn plugins(&self) -> &[&'static str] {
        &self.plugins
    }

    pub fn commands(&self) -> impl Iterator<Item = &CommandInfo> {
        self.commands.values()
    }

    pub fn component_kinds(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.components.iter().map(|c| c.kind)
    }

    pub(crate) fn command_info(&self, name: &str) -> Option<&CommandInfo> {
        self.commands.get(name)
    }

    pub(crate) fn component_info(&self, kind: &str) -> Option<&ComponentInfo> {
        self.by_kind.get(kind).map(|&i| &self.components[i])
    }

    pub(crate) fn component_infos(&self) -> &[ComponentInfo] {
        &self.components
    }

    pub fn op_from_wire(&self, op: WireOp) -> Result<Op, CommandError> {
        Ok(match op {
            WireOp::Create { id } => Op::Create { id },
            WireOp::Delete { id } => Op::Delete { id },
            WireOp::Set { id, kind, before, after } => {
                let info = self.component_info(&kind).ok_or_else(|| {
                    CommandError::new(ErrorCode::UnsupportedRepresentation, format!("unknown component kind {kind}"))
                })?;
                let decode = |v: Option<Value>| -> Result<Option<CompValue>, CommandError> {
                    v.map(info.decode)
                        .transpose()
                        .map_err(|e| CommandError::new(ErrorCode::InvalidParameters, format!("{kind}: {e}")))
                };
                Op::Set { id, kind: info.kind, before: decode(before)?, after: decode(after)? }
            }
        })
    }

    pub fn changeset_from_wire(&self, wire: WireChangeSet) -> Result<ChangeSet, CommandError> {
        Ok(ChangeSet {
            revision: wire.revision,
            base_revision: wire.base_revision,
            command_id: wire.command_id,
            operation: wire.operation,
            stamp: wire.stamp.unwrap_or_default(),
            seen: wire.seen,
            actor: wire.actor,
            ops: wire.ops.into_iter().map(|op| self.op_from_wire(op)).collect::<Result<_, _>>()?,
        })
    }
}

fn capture<T: Persist>(world: &World, entity: Entity) -> Option<CompValue> {
    world.get::<T>(entity).map(|c| Arc::new(c.clone()) as CompValue)
}

fn write<T: Persist>(world: &mut World, entity: Entity, value: Option<&CompValue>) -> Result<(), CommandError> {
    let mut entity = world.entity_mut(entity);
    match value {
        Some(value) => {
            let typed = value.as_any().downcast_ref::<T>().ok_or_else(|| {
                CommandError::new(ErrorCode::Internal, format!("value is not a {}", T::KIND))
            })?;
            entity.insert(typed.clone());
        }
        None => {
            entity.remove::<T>();
        }
    }
    Ok(())
}

fn decode<T: Persist>(value: Value) -> Result<CompValue, String> {
    serde_json::from_value::<T>(value).map(|v| Arc::new(v) as CompValue).map_err(|e| e.to_string())
}

fn exec<C: Command>(parameters: Value, tx: &mut Tx) -> Result<Value, CommandError> {
    let command: C = serde_json::from_value(parameters)
        .map_err(|e| CommandError::new(ErrorCode::InvalidParameters, format!("{}: {e}", C::NAME)))?;
    command.apply(tx)
}
