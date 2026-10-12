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
use std::collections::HashMap;
use std::sync::Arc;

/// A transaction over the shared project space. Commands read anything and
/// write through these methods; every write is recorded so the transaction can
/// be committed, previewed, rolled back, undone or sent to peers.
pub struct Tx<'a> {
    world: &'a mut World,
    registry: &'a Registry,
    actor: &'a Actor,
    ops: Vec<Op>,
    /// Where each component written so far was recorded: writing it again
    /// updates that op (first before, last after) instead of adding one, so a
    /// list grown one item at a time (an assembly's dependencies, joint by
    /// joint) journals once, not once per item with the whole list each time.
    /// Only writes after the last entity created fold (a value naming the new
    /// entity must not replay before it exists).
    written: HashMap<(StableId, &'static str), usize>,
    created_at: usize,
}

impl<'a> Tx<'a> {
    pub(crate) fn new(world: &'a mut World, registry: &'a Registry, actor: &'a Actor) -> Self {
        Self { world, registry, actor, ops: Vec::new(), written: HashMap::new(), created_at: 0 }
    }

    pub(crate) fn into_ops(self) -> Vec<Op> {
        self.ops
    }

    pub fn actor(&self) -> &Actor {
        self.actor
    }

    /// Runs another command inside this transaction (its changes join this
    /// one: one atomic change, one undo step).
    pub fn run(&mut self, operation: &str, parameters: crate::Value) -> Result<crate::Value, CommandError> {
        let info = self.registry.command_info(operation).ok_or_else(|| CommandError::new(crate::ErrorCode::UnknownOperation, format!("unknown operation {operation}")))?;
        if operation == "core.batch" {
            return Err(CommandError::new(crate::ErrorCode::InvalidParameters, "a batch cannot contain another batch"));
        }
        (info.exec)(parameters, self)
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
        self.created_at = self.ops.len();
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
        self.record_set(id, T::KIND, before.map(|b| Arc::new(b) as CompValue), Some(after));
        Ok(())
    }

    /// Records a write, folding it into an earlier write of the same component.
    fn record_set(&mut self, id: StableId, kind: &'static str, before: Option<CompValue>, after: Option<CompValue>) {
        if let Some(&i) = self.written.get(&(id, kind))
            && i >= self.created_at
            && let Some(Op::Set { after: last, .. }) = self.ops.get_mut(i)
        {
            *last = after;
            return;
        }
        self.written.insert((id, kind), self.ops.len());
        self.ops.push(Op::Set { id, kind, before, after });
    }

    pub fn remove<T: Persist>(&mut self, id: StableId) -> Result<(), CommandError> {
        let entity = self.entity(id)?;
        if let Some(before) = self.world.entity_mut(entity).take::<T>() {
            self.record_set(id, T::KIND, Some(Arc::new(before)), None);
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
        self.written.retain(|(w, _), _| *w != id);
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
