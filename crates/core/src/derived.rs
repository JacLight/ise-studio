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

/// What a project is: its title, a description, and tags to find it by.
/// Its name never changes (files, links and teammates' computers know it by
/// that); these do, like any other part of the design (synced, undoable).
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectInfo {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Persist for ProjectInfo {
    const KIND: &'static str = "core.project";
}

/// `project.set_info`: changes the project's title, description or tags
/// (what is left out stays as it was).
#[derive(Deserialize, JsonSchema)]
pub(crate) struct SetProjectInfo {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

impl Command for SetProjectInfo {
    const NAME: &'static str = "project.set_info";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let bad = |m: &str| CommandError::new(crate::ErrorCode::InvalidParameters, m.to_string());
        let existing = tx.world().resource::<crate::IdIndex>().iter().find(|(_, e)| tx.world().get::<ProjectInfo>(*e).is_some()).map(|(id, _)| id);
        let mut info = existing.and_then(|id| tx.get::<ProjectInfo>(id).cloned()).unwrap_or_default();
        if let Some(t) = self.title {
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if t.is_empty() || t.chars().count() > 120 {
                return Err(bad("a title is 1 to 120 characters"));
            }
            info.title = t;
        }
        if let Some(d) = self.description {
            if d.chars().count() > 4000 {
                return Err(bad("a description is up to 4000 characters"));
            }
            info.description = d.trim().to_string();
        }
        if let Some(tags) = self.tags {
            let mut clean: Vec<String> = vec![];
            for t in tags {
                let t = t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
                if !t.is_empty() && t.chars().count() <= 40 && !clean.contains(&t) {
                    clean.push(t);
                }
            }
            if clean.len() > 30 {
                return Err(bad("up to 30 tags"));
            }
            info.tags = clean;
        }
        let id = match existing {
            Some(id) => id,
            None => tx.create(),
        };
        tx.set(id, info.clone())?;
        Ok(json!({ "id": id, "title": info.title, "description": info.description, "tags": info.tags }))
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
