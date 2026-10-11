//! Named versions: the project marked at a point in its history ("Sent to
//! supplier", "v1.2"), so anyone can look at it, compare it with now, put
//! everything back to it, or branch from it.
//!
//! A version is part of the project like anything else (a journaled entity,
//! synced to teammates). It names a state by a version vector — the latest
//! change taken from each engine — not by a revision number: revisions are
//! this copy's own count and differ between teammates' copies, while the
//! vector means the same set of changes everywhere. Any copy evaluates the
//! version by replaying exactly the changes it covers (see
//! [`crate::Project::as_of`]).

use crate::{
    command::Command,
    error::{CommandError, ErrorCode},
    ids::{IdIndex, Revision, StableId},
    registry::Persist,
    tx::Tx,
};
use bevy_ecs::{component::Component, resource::Resource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A version vector: the latest change (clock) taken from each engine.
pub type Vector = BTreeMap<String, u64>;

/// Longest version name and note, in characters.
pub const MAX_NAME: usize = 80;
pub const MAX_NOTE: usize = 2000;

/// The project as it was at a point in its history, by name.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Version {
    pub name: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub author: String,
    /// When it was marked (ms since 1970, UTC).
    #[serde(default)]
    pub created: u64,
    /// Which changes it holds: every change whose clock is at or below its
    /// engine's entry here.
    pub seen: Vector,
}

impl Persist for Version {
    const KIND: &'static str = "core.version";
}

/// The version vector after each revision of this copy (index = revision),
/// kept by the project so commands can name a past revision.
#[derive(Resource, Default, Clone, Debug)]
pub struct Clocks {
    pub after: Vec<Vector>,
}

impl Clocks {
    /// The changes held at `revision` on this copy.
    pub fn at(&self, revision: Revision) -> Option<&Vector> {
        self.after.get(revision.0 as usize)
    }

    /// This copy's revision holding exactly `seen`, if there is one (another
    /// copy's version may never have existed here as one revision).
    pub fn revision_of(&self, seen: &Vector) -> Option<Revision> {
        let norm = |v: &Vector| v.iter().filter(|(_, c)| **c > 0).map(|(k, c)| (k.clone(), *c)).collect::<Vector>();
        let want = norm(seen);
        self.after.iter().position(|v| norm(v) == want).map(|i| Revision(i as u64))
    }
}

/// Whether a change stamped `(origin, clock)` is part of the state `seen` names.
pub fn covers(seen: &Vector, origin: &str, clock: u64) -> bool {
    clock <= seen.get(origin).copied().unwrap_or(0)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn bad(m: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, m)
}

fn name_of(name: &str) -> Result<String, CommandError> {
    let n = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() || n.chars().count() > MAX_NAME {
        return Err(bad(format!("a version name is 1 to {MAX_NAME} characters")));
    }
    Ok(n)
}

fn note_of(note: &str) -> Result<String, CommandError> {
    let n = note.trim();
    if n.chars().count() > MAX_NOTE {
        return Err(bad(format!("a version note is up to {MAX_NOTE} characters")));
    }
    Ok(n.to_string())
}

/// `version.create`: names the project as it is now, or as it was at an
/// earlier revision of this copy (`revision`).
#[derive(Deserialize, JsonSchema)]
pub(crate) struct CreateVersion {
    name: String,
    #[serde(default)]
    note: String,
    /// Who marked it (shown with it).
    #[serde(default)]
    author: String,
    /// A revision of this copy (default: the current one).
    #[serde(default)]
    revision: Option<u64>,
}

impl Command for CreateVersion {
    const NAME: &'static str = "version.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let name = name_of(&self.name)?;
        let note = note_of(&self.note)?;
        let clocks = tx.world().get_resource::<Clocks>().cloned().unwrap_or_default();
        let current = clocks.after.len().saturating_sub(1) as u64;
        let revision = self.revision.unwrap_or(current);
        if revision > current {
            return Err(bad(format!("there is no revision {revision} yet (the project is at {current})")));
        }
        let seen = clocks.at(Revision(revision)).cloned().unwrap_or_default();
        let taken = tx.world().resource::<IdIndex>().iter().any(|(_, e)| tx.world().get::<Version>(e).is_some_and(|v| v.name.eq_ignore_ascii_case(&name)));
        if taken {
            return Err(bad(format!("there is already a version called “{name}”")));
        }
        let author = self.author.trim().chars().take(80).collect::<String>();
        let id = tx.create();
        tx.set(id, Version { name: name.clone(), note, author, created: now_ms(), seen })?;
        Ok(json!({ "id": id, "name": name, "revision": revision }))
    }
}

/// `version.update`: renames a version or changes its note.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct UpdateVersion {
    target: StableId,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

impl Command for UpdateVersion {
    const NAME: &'static str = "version.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut v = tx.get::<Version>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a version", self.target)))?;
        if let Some(n) = self.name {
            let n = name_of(&n)?;
            let target = self.target;
            let taken = tx.world().resource::<IdIndex>().iter().any(|(id, e)| id != target && tx.world().get::<Version>(e).is_some_and(|v| v.name.eq_ignore_ascii_case(&n)));
            if taken {
                return Err(bad(format!("there is already a version called “{n}”")));
            }
            v.name = n;
        }
        if let Some(n) = self.note {
            v.note = note_of(&n)?;
        }
        tx.set(self.target, v)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `version.delete`: forgets a version's name (the history itself stays).
#[derive(Deserialize, JsonSchema)]
pub(crate) struct DeleteVersion {
    target: StableId,
}

impl Command for DeleteVersion {
    const NAME: &'static str = "version.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if tx.get::<Version>(self.target).is_none() {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a version", self.target)));
        }
        tx.delete(self.target)?;
        Ok(json!({ "id": self.target }))
    }
}

// ---- Comparing two states --------------------------------------------------

/// One field that differs: where in the component (readable path), and the
/// value then and now (null when absent).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldChange {
    pub kind: String,
    pub path: String,
    pub before: Value,
    pub after: Value,
}

/// How one entity differs between two states.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EntityChange {
    pub id: StableId,
    /// "added", "removed" or "changed" (going from the first state to the second).
    pub change: &'static str,
    /// What it is called (its `name` field, if any component has one).
    pub name: Option<String>,
    /// The component kinds it has (in whichever state it exists).
    pub kinds: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldChange>,
}

/// Most field changes listed for one entity.
const MAX_FIELDS: usize = 60;

fn label_of(v: &Value) -> Option<String> {
    let o = v.as_object()?;
    ["key", "name", "id", "label"].iter().find_map(|k| o.get(*k).and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_u64().map(|n| n.to_string()))))
}

/// The leaf differences between two JSON values, with paths that read well:
/// array items that carry a key or name are named by it ("params/width/value").
pub fn field_diff(kind: &str, before: &Value, after: &Value, path: &str, out: &mut Vec<FieldChange>) {
    if before == after || out.len() >= MAX_FIELDS {
        return;
    }
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let (x, y) = (a.get(k).unwrap_or(&Value::Null), b.get(k).unwrap_or(&Value::Null));
                field_diff(kind, x, y, &format!("{path}/{k}"), out);
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            let labels_a: Vec<Option<String>> = a.iter().map(label_of).collect();
            let labels_b: Vec<Option<String>> = b.iter().map(label_of).collect();
            let keyed = labels_a.iter().chain(labels_b.iter()).all(Option::is_some);
            if keyed && !(a.is_empty() && b.is_empty()) {
                let mut names: Vec<&String> = labels_a.iter().flatten().collect();
                for l in labels_b.iter().flatten() {
                    if !names.contains(&l) {
                        names.push(l);
                    }
                }
                for n in names {
                    let x = labels_a.iter().position(|l| l.as_ref() == Some(n)).map(|i| &a[i]).unwrap_or(&Value::Null);
                    let y = labels_b.iter().position(|l| l.as_ref() == Some(n)).map(|i| &b[i]).unwrap_or(&Value::Null);
                    field_diff(kind, x, y, &format!("{path}/{n}"), out);
                }
            } else if a.len() == b.len() && a.iter().all(|v| v.is_object() || v.is_array()) {
                for (i, (x, y)) in a.iter().zip(b).enumerate() {
                    field_diff(kind, x, y, &format!("{path}/{i}"), out);
                }
            } else {
                out.push(FieldChange { kind: kind.into(), path: path.into(), before: before.clone(), after: after.clone() });
            }
        }
        _ => out.push(FieldChange { kind: kind.into(), path: path.into(), before: before.clone(), after: after.clone() }),
    }
}

fn name_in(state: &BTreeMap<&'static str, Value>) -> Option<String> {
    state.values().find_map(|v| v.get("name").and_then(Value::as_str).map(str::to_string)).or_else(|| state.values().find_map(|v| v.get("title").and_then(Value::as_str).map(str::to_string)))
}

/// What changed going from state `from` to state `to`, entity by entity.
/// Entities holding any of `skip` kinds are left out (versions themselves,
/// for instance).
pub fn compare(from: &crate::Project, to: &crate::Project, skip: &[&str]) -> Vec<EntityChange> {
    let a = from.export_state();
    let b = to.export_state();
    let skipped = |s: &BTreeMap<&'static str, Value>| s.keys().any(|k| skip.contains(k));
    let mut out = Vec::new();
    let ids: std::collections::BTreeSet<StableId> = a.keys().chain(b.keys()).copied().collect();
    for id in ids {
        match (a.get(&id), b.get(&id)) {
            (Some(x), None) if !skipped(x) => out.push(EntityChange { id, change: "removed", name: name_in(x), kinds: x.keys().map(|k| k.to_string()).collect(), fields: vec![] }),
            (None, Some(y)) if !skipped(y) => out.push(EntityChange { id, change: "added", name: name_in(y), kinds: y.keys().map(|k| k.to_string()).collect(), fields: vec![] }),
            (Some(x), Some(y)) if x != y && !skipped(x) && !skipped(y) => {
                let mut fields = Vec::new();
                let kinds: std::collections::BTreeSet<&&str> = x.keys().chain(y.keys()).collect();
                for k in kinds {
                    let (p, q) = (x.get(*k).unwrap_or(&Value::Null), y.get(*k).unwrap_or(&Value::Null));
                    field_diff(k, p, q, "", &mut fields);
                }
                out.push(EntityChange { id, change: "changed", name: name_in(y).or_else(|| name_in(x)), kinds: y.keys().map(|k| k.to_string()).collect(), fields });
            }
            _ => {}
        }
    }
    out
}
