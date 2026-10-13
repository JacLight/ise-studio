use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Machine-readable failure codes shared by the UI, CLI and AI agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    StaleRevision,
    WrongProject,
    UnsupportedSchemaVersion,
    UnknownOperation,
    InvalidParameters,
    UnresolvedReference,
    AmbiguousSelection,
    UnsatisfiableConstraint,
    InvalidGeometry,
    UnsupportedRepresentation,
    SolverDidNotConverge,
    StaleDerivedOutput,
    Conflict,
    UndoConflict,
    NothingToUndo,
    NothingToRedo,
    /// A view that cannot change anything (a past version, a shared link).
    ReadOnly,
    /// An item linked from another project, which follows its source: break
    /// the link or allow local edits on it to change it here.
    LinkedReadOnly,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}
