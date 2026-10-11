use crate::{
    command::Actor,
    ids::{Revision, StableId},
    value::CompValue,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One reversible change to the shared project space.
#[derive(Clone, Debug)]
pub enum Op {
    Create { id: StableId },
    Delete { id: StableId },
    Set { id: StableId, kind: &'static str, before: Option<CompValue>, after: Option<CompValue> },
}

impl Op {
    pub fn id(&self) -> StableId {
        match self {
            Op::Create { id } | Op::Delete { id } | Op::Set { id, .. } => *id,
        }
    }

    pub fn inverse(&self) -> Op {
        match self {
            Op::Create { id } => Op::Delete { id: *id },
            Op::Delete { id } => Op::Create { id: *id },
            Op::Set { id, kind, before, after } => {
                Op::Set { id: *id, kind, before: after.clone(), after: before.clone() }
            }
        }
    }

    pub fn to_wire(&self) -> WireOp {
        match self {
            Op::Create { id } => WireOp::Create { id: *id },
            Op::Delete { id } => WireOp::Delete { id: *id },
            Op::Set { id, kind, before, after } => WireOp::Set {
                id: *id,
                kind: (*kind).to_owned(),
                before: before.as_ref().map(|v| v.to_json()),
                after: after.as_ref().map(|v| v.to_json()),
            },
        }
    }
}

/// When and where a change was made, comparable on every copy: a Lamport
/// clock (not wall time, so a wrong clock cannot reorder edits) and the engine
/// that made it as a tie-break. Later stamps win conflicting fields everywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Stamp {
    pub clock: u64,
    pub origin: String,
}

/// A committed transaction: the unit of history, undo and collaboration.
#[derive(Clone, Debug)]
pub struct ChangeSet {
    pub revision: Revision,
    pub base_revision: Revision,
    pub command_id: String,
    pub operation: String,
    pub actor: Actor,
    pub ops: Vec<Op>,
    pub stamp: Stamp,
    /// What its author had already taken from each engine (latest clock per
    /// engine): a version vector, so any copy can tell what it knew about.
    pub seen: std::collections::BTreeMap<String, u64>,
}

impl ChangeSet {
    pub fn to_wire(&self) -> WireChangeSet {
        WireChangeSet {
            revision: self.revision,
            base_revision: self.base_revision,
            command_id: self.command_id.clone(),
            operation: self.operation.clone(),
            actor: self.actor.clone(),
            ops: self.ops.iter().map(Op::to_wire).collect(),
            stamp: Some(self.stamp.clone()),
            seen: self.seen.clone(),
            merged: false,
        }
    }
}

/// Serialized form of [`Op`] for peers, the UI and storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WireOp {
    Create {
        id: StableId,
    },
    Delete {
        id: StableId,
    },
    Set {
        id: StableId,
        kind: String,
        #[serde(default)]
        before: Option<Value>,
        #[serde(default)]
        after: Option<Value>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WireChangeSet {
    pub revision: Revision,
    pub base_revision: Revision,
    pub command_id: String,
    pub operation: String,
    pub actor: Actor,
    pub ops: Vec<WireOp>,
    /// Absent in journals written before collaboration: the revision stands in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    /// Its author's version vector when it was made (see [`ChangeSet::seen`]).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub seen: std::collections::BTreeMap<String, u64>,
    /// Made on another member's copy and merged here (stored as it arrived, so
    /// replaying the log re-merges it exactly the same way).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merged: bool,
}
