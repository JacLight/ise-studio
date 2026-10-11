//! Engine core: one shared project space that every plugin works in.
//!
//! An entity is only a [`StableId`]. Any plugin can attach its own components to
//! any entity and read every other plugin's components directly. All changes go
//! through typed [`Command`]s that run inside a [`Tx`]; each commit becomes a
//! [`ChangeSet`] in the journal, which provides undo, history, collaboration and
//! recovery for every plugin at once. Derived results are recomputed
//! incrementally, in parallel, for only the entities an edit affects.

mod apply;
pub mod command;
pub mod derived;
pub mod error;
pub mod ids;
pub mod journal;
pub mod merge;
pub mod plugin;
pub mod project;
pub mod registry;
pub mod tx;
pub mod units;
pub mod value;

pub use bevy_ecs;
pub use command::{Actor, Command, Envelope, SCHEMA_VERSION};
pub use derived::{DependsOn, Derived, ProjectInfo, Status};
pub use error::{CommandError, ErrorCode};
pub use ids::{IdIndex, Revision, StableId};
pub use journal::{ChangeSet, Op, Stamp, WireChangeSet, WireOp};
pub use merge::Conflict;
pub use plugin::Plugin;
pub use project::{Committed, Merge, Preview, Project};
pub use registry::{Persist, Registry};
pub use schemars;
pub use serde_json::{Value, json};
pub use tx::Tx;
pub use units::{Density, DensityUnit, Length, LengthUnit, Mass, MassUnit};
