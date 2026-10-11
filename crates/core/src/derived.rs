//! Dependencies, derived computations and result status.

use crate::{
    command::Command,
    error::CommandError,
    ids::StableId,
    registry::Persist,
    tx::Tx,
};
use bevy_ecs::{component::Component, entity::Entity, world::World};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Entities whose data or derived results this entity's derived results read.
/// Changing any of them makes this entity stale.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DependsOn(pub Vec<StableId>);

impl Persist for DependsOn {
    const KIND: &'static str = "core.depends_on";
}

/// State of an entity's derived results.
#[derive(Component, Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum Status {
    Current,
    Stale,
    Computing,
    Failed(String),
    Blocked(String),
}

impl Status {
    pub fn is_ok(&self) -> bool {
        matches!(self, Status::Current)
    }
}

/// A pure computation producing a derived component from the shared space.
pub struct Derived<Out> {
    pub name: &'static str,
    /// Persistent component kinds on the same entity that this reads.
    pub inputs: &'static [&'static str],
    /// Whether this reads data or results of the entity's dependencies.
    pub reads_dependencies: bool,
    /// `None` when not applicable to the entity; `Err` marks it failed.
    pub compute: fn(&World, Entity) -> Option<Result<Out, String>>,
}

pub(crate) struct Batch {
    pub failures: Vec<(Entity, String)>,
    pub write: Box<dyn FnOnce(&mut World) + Send>,
}

type Runner = Box<dyn Fn(&World, &[Entity]) -> Batch + Send + Sync>;

pub(crate) struct DerivedInfo {
    pub name: &'static str,
    pub inputs: &'static [&'static str],
    pub reads_dependencies: bool,
    pub run: Runner,
}

/// Below this, parallel dispatch costs more than it saves.
const PARALLEL_THRESHOLD: usize = 512;

impl DerivedInfo {
    pub fn new<Out: Component>(spec: Derived<Out>) -> Self {
        let compute = spec.compute;
        let run: Runner = Box::new(move |world: &World, entities: &[Entity]| {
            let results: Vec<(Entity, Option<Result<Out, String>>)> = if entities.len() >= PARALLEL_THRESHOLD {
                entities.par_iter().map(|&e| (e, compute(world, e))).collect()
            } else {
                entities.iter().map(|&e| (e, compute(world, e))).collect()
            };
            let mut failures = Vec::new();
            let mut outputs = Vec::with_capacity(results.len());
            let mut cleared = Vec::new();
            for (entity, result) in results {
                match result {
                    None => {}
                    Some(Ok(out)) => outputs.push((entity, out)),
                    Some(Err(message)) => {
                        failures.push((entity, message));
                        cleared.push(entity);
                    }
                }
            }
            Batch {
                failures,
                write: Box::new(move |world: &mut World| {
                    world.insert_batch(outputs);
                    // A failed computation must not leave an old result looking current.
                    for entity in cleared {
                        world.entity_mut(entity).remove::<Out>();
                    }
                }),
            }
        });
        Self { name: spec.name, inputs: spec.inputs, reads_dependencies: spec.reads_dependencies, run }
    }
}

/// `core.entity.delete`
#[derive(Deserialize, JsonSchema)]
pub(crate) struct DeleteEntity {
    target: StableId,
}

impl Command for DeleteEntity {
    const NAME: &'static str = "core.entity.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        tx.delete(self.target)?;
        Ok(json!({ "deleted": self.target }))
    }
}

/// One step of a batch: an operation and its parameters.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct BatchStep {
    operation: String,
    #[serde(default)]
    parameters: Value,
}

/// `core.batch`: several commands as one change (one undo step); if any
/// fails, none of them happens.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct BatchCommand {
    steps: Vec<BatchStep>,
}

impl Command for BatchCommand {
    const NAME: &'static str = "core.batch";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut outputs = Vec::with_capacity(self.steps.len());
        for (i, step) in self.steps.into_iter().enumerate() {
            let out = tx.run(&step.operation, step.parameters).map_err(|e| CommandError::new(e.code, format!("step {} ({}): {}", i + 1, step.operation, e.message)))?;
            outputs.push(out);
        }
        Ok(json!({ "outputs": outputs }))
    }
}
