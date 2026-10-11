use crate::{
    command::Actor,
    derived::DependsOn,
    error::{CommandError, ErrorCode},
    ids::{IdIndex, StableId},
    journal::Op,
    registry::{Persist, Registry},
    value::CompValue,
};
use bevy_ecs::{component::Component, entity::Entity, world::World};
use std::sync::Arc;

/// A transaction over the shared project space. Commands read anything and
/// write through these methods; every write is recorded so the transaction can
/// be committed, previewed, rolled back, undone or sent to peers.
pub struct Tx<'a> {
    world: &'a mut World,
    registry: &'a Registry,
    actor: &'a Actor,
    ops: Vec<Op>,
}

impl<'a> Tx<'a> {
    pub(crate) fn new(world: &'a mut World, registry: &'a Registry, actor: &'a Actor) -> Self {
        Self { world, registry, actor, ops: Vec::new() }
    }

    pub(crate) fn into_ops(self) -> Vec<Op> {
        self.ops
    }

    pub fn actor(&self) -> &Actor {
        self.actor
    }

    /// Read access to the whole project space, including derived results.
    pub fn world(&self) -> &World {
        self.world
    }

    pub fn exists(&self, id: StableId) -> bool {
        self.world.resource::<IdIndex>().contains(id)
    }

    pub fn entity(&self, id: StableId) -> Result<Entity, CommandError> {
        self.world
            .resource::<IdIndex>()
            .get(id)
            .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("entity {id} does not exist")))
    }

    pub fn get<T: Component>(&self, id: StableId) -> Option<&T> {
        let entity = self.world.resource::<IdIndex>().get(id)?;
        self.world.get::<T>(entity)
    }

    pub fn create(&mut self) -> StableId {
        let id = StableId::new();
        let entity = self.world.spawn(id).id();
        self.world.resource_mut::<IdIndex>().insert(id, entity);
        self.ops.push(Op::Create { id });
        id
    }

    pub fn set<T: Persist>(&mut self, id: StableId, value: T) -> Result<(), CommandError> {
        let entity = self.entity(id)?;
        let before = self.world.get::<T>(entity).cloned();
        if before.as_ref() == Some(&value) {
            return Ok(());
        }
        let after: CompValue = Arc::new(value.clone());
        self.world.entity_mut(entity).insert(value);
        self.ops.push(Op::Set {
            id,
            kind: T::KIND,
            before: before.map(|b| Arc::new(b) as CompValue),
            after: Some(after),
        });
        Ok(())
    }

    pub fn remove<T: Persist>(&mut self, id: StableId) -> Result<(), CommandError> {
        let entity = self.entity(id)?;
        if let Some(before) = self.world.entity_mut(entity).take::<T>() {
            self.ops.push(Op::Set { id, kind: T::KIND, before: Some(Arc::new(before)), after: None });
        }
        Ok(())
    }

    /// Removes the entity and every persistent component on it. Entities that
    /// depend on it become blocked until the reference is repaired.
    pub fn delete(&mut self, id: StableId) -> Result<(), CommandError> {
        let entity = self.entity(id)?;
        for info in self.registry.component_infos() {
            let before = (info.capture)(self.world, entity);
            if before.is_some() {
                (info.write)(self.world, entity, None)?;
                self.ops.push(Op::Set { id, kind: info.kind, before, after: None });
            }
        }
        self.world.despawn(entity);
        self.world.resource_mut::<IdIndex>().remove(id);
        self.ops.push(Op::Delete { id });
        Ok(())
    }

    pub fn add_dependency(&mut self, id: StableId, on: StableId) -> Result<(), CommandError> {
        self.entity(on)?;
        let mut deps = self.get::<DependsOn>(id).cloned().unwrap_or_default();
        if !deps.0.contains(&on) {
            deps.0.push(on);
            self.set(id, deps)?;
        }
        Ok(())
    }

    pub fn remove_dependency(&mut self, id: StableId, on: StableId) -> Result<(), CommandError> {
        let Some(mut deps) = self.get::<DependsOn>(id).cloned() else { return Ok(()) };
        deps.0.retain(|d| *d != on);
        if deps.0.is_empty() { self.remove::<DependsOn>(id) } else { self.set(id, deps) }
    }

    /// Ops recorded so far (what a preview shows).
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }
}
