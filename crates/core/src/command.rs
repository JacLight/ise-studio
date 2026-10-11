use crate::{error::CommandError, ids::Revision, tx::Tx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;

/// The signed-in user. Commands the AI agent issues carry the same actor.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Actor {
    pub user_id: String,
    pub session_id: String,
}

/// Versioned envelope every mutating request uses, from the UI, CLI or agent.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Envelope {
    pub schema_version: u32,
    /// Idempotency key: repeating a committed command ID returns the original result.
    pub command_id: String,
    pub project_id: String,
    pub expected_revision: Revision,
    pub operation: String,
    #[serde(default)]
    pub parameters: Value,
    pub actor: Actor,
}

/// A typed operation a plugin exposes. Its parameter schema is published to
/// the UI and becomes an AI tool definition automatically.
pub trait Command: DeserializeOwned + JsonSchema + 'static {
    const NAME: &'static str;

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError>;
}
