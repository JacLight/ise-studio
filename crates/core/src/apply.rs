use crate::{
    error::{CommandError, ErrorCode},
    ids::IdIndex,
    journal::Op,
    registry::Registry,
    value::same,
};
use bevy_ecs::world::World;

/// Applies one op. With `verify`, a `Set` only applies if the current value
/// still equals the op's `before` (used for undo and peer change sets).
pub(crate) fn apply_op(world: &mut World, registry: &Registry, op: &Op, verify: bool) -> Result<(), CommandError> {
    match op {
        Op::Create { id } => {
            if world.resource::<IdIndex>().contains(*id) {
                return Err(CommandError::new(ErrorCode::Conflict, format!("entity {id} already exists")));
            }
            let entity = world.spawn(*id).id();
            world.resource_mut::<IdIndex>().insert(*id, entity);
        }
        Op::Delete { id } => {
            let entity = lookup(world, *id)?;
            world.despawn(entity);
            world.resource_mut::<IdIndex>().remove(*id);
        }
        Op::Set { id, kind, before, after } => {
            let entity = lookup(world, *id)?;
            let info = registry
                .component_info(kind)
                .ok_or_else(|| CommandError::new(ErrorCode::Internal, format!("unknown component kind {kind}")))?;
            if verify && !same((info.capture)(world, entity).as_ref(), before.as_ref()) {
                return Err(CommandError::new(
                    ErrorCode::Conflict,
                    format!("{kind} on {id} changed since this change was made"),
                ));
            }
            (info.write)(world, entity, after.as_ref())?;
        }
    }
    Ok(())
}

/// Applies all ops or none.
pub(crate) fn apply_atomic(world: &mut World, registry: &Registry, ops: &[Op], verify: bool) -> Result<(), CommandError> {
    for (i, op) in ops.iter().enumerate() {
        if let Err(err) = apply_op(world, registry, op, verify) {
            rollback(world, registry, &ops[..i]);
            return Err(err);
        }
    }
    Ok(())
}

/// Reverts ops that were just applied, newest first.
pub(crate) fn rollback(world: &mut World, registry: &Registry, applied: &[Op]) {
    for op in applied.iter().rev() {
        apply_op(world, registry, &op.inverse(), false).expect("reverting just-applied ops cannot fail");
    }
}

fn lookup(world: &World, id: crate::ids::StableId) -> Result<bevy_ecs::entity::Entity, CommandError> {
    world
        .resource::<IdIndex>()
        .get(id)
        .ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("entity {id} does not exist")))
}
