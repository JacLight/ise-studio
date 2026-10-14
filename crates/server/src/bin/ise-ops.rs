//! `ise-ops`: every engine operation with its parameters' JSON schema (from the code's own documentation),
//! as JSON on stdout. tools/gen-agent-docs.mjs turns it into the Iṣẹ́ Studio skill's operation reference, so
//! the agent documentation follows the code: change a command, regenerate, and the docs say what it does.
//! The plugin list is the engine's (crates/server/src/main.rs, `Registry::with_plugins`): keep the two alike.

use de_core::Registry;
use serde_json::{Value, json};

fn main() {
    let registry = Registry::with_plugins(&[
        &de_solid::SolidPlugin,
        &de_shape::ShapePlugin,
        &de_assembly::AssemblyPlugin,
        &de_draft::DraftPlugin,
        &de_intent::IntentPlugin,
        &de_sourcing::SourcingPlugin,
        &de_pcb::PcbPlugin,
    ]);
    let mut commands: Vec<Value> = registry.commands().map(|c| json!({ "name": c.name, "plugin": c.plugin, "schema": c.schema })).collect();
    commands.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    println!("{}", serde_json::to_string_pretty(&json!({ "commands": commands })).unwrap());
}
