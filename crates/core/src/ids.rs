use bevy_ecs::{component::Component, entity::Entity, resource::Resource};
use rustc_hash::FxHashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Identity of a design object. Survives save, reload, collaboration and
/// rebuilds; ECS entity handles do not, so they never leave the engine.
#[derive(
    Component, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct StableId(pub Uuid);

impl StableId {
    /// Time-ordered (UUIDv7), so IDs created together sort and index well.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for StableId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Project revision. Revision N means N change sets have been committed.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Revision(pub u64);

impl Revision {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Maps stable IDs to live entities. Stored in the world so plugin code
/// (commands and derived computations) can resolve references directly.
#[derive(Resource, Default)]
pub struct IdIndex {
    map: FxHashMap<StableId, Entity>,
}

impl IdIndex {
    pub fn get(&self, id: StableId) -> Option<Entity> {
        self.map.get(&id).copied()
    }

    pub fn contains(&self, id: StableId) -> bool {
        self.map.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (StableId, Entity)> + '_ {
        self.map.iter().map(|(id, e)| (*id, *e))
    }

    pub(crate) fn insert(&mut self, id: StableId, entity: Entity) {
        self.map.insert(id, entity);
    }

    pub(crate) fn remove(&mut self, id: StableId) {
        self.map.remove(&id);
    }
}
