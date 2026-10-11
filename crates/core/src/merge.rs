//! Merging a peer's change set into this copy of a project.
//!
//! There is no host: every member keeps a full copy, and change sets travel
//! between copies in whatever order the network delivers them. Each change
//! records every component's value before and after, so a change made on
//! another copy is merged field by field against what this copy holds now:
//!
//! - each field takes its latest write by stamp (last writer wins, per
//!   field, as Cassandra does per column), so what only one side changed is
//!   kept and every copy computes the same result in any order;
//! - a field both sides changed to different values is a [`Conflict`]: as in
//!   multi-master databases, every copy picks the same winner without talking
//!   (the later [`crate::journal::Stamp`]), and the losing value is kept in the
//!   conflict for someone to choose instead. Copies never drift apart.
//!
//! Merging the same change sets in any order gives the same values wherever
//! the edits do not conflict.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{command::Actor, ids::StableId};

/// A field two members changed differently at the same time. Every copy keeps
/// the later change (by stamp); the other value is reported for someone to
/// choose instead.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Conflict {
    /// Which value every copy now holds: "theirs" or "ours" (the later change
    /// wins everywhere); the other is kept here so nothing is lost.
    pub kept: String,
    pub id: StableId,
    pub kind: String,
    /// JSON pointer within the component ("" for the whole component).
    pub path: String,
    pub ours: Value,
    pub theirs: Value,
    /// Who made their change, and in which change set.
    pub by: Actor,
    pub command_id: String,
}

/// Pointers to every leaf that differs between two values. Objects compare
/// key by key; lists of the same length item by item; anything else whole.
pub(crate) fn changed_paths(before: &Value, after: &Value, at: &str, out: &mut Vec<String>) {
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let p = format!("{at}/{}", k.replace('~', "~0").replace('/', "~1"));
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => changed_paths(x, y, &p, out),
                    (x, y) if x != y => out.push(p),
                    _ => {}
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() && !is_plain_list(a) => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                changed_paths(x, y, &format!("{at}/{i}"), out);
            }
        }
        (x, y) if x != y => out.push(at.to_string()),
        _ => {}
    }
}

/// A list of strings, numbers or booleans: compared (and won) whole.
fn is_plain_list(items: &[Value]) -> bool {
    items.iter().all(|v| matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_)))
}

fn get<'v>(v: &'v Value, path: &str) -> Option<&'v Value> {
    if path.is_empty() { Some(v) } else { v.pointer(path) }
}

/// Sets (or, with None, removes) the value at `path`, creating objects on the way.
fn put(v: &mut Value, path: &str, new: Option<Value>) {
    if path.is_empty() {
        *v = new.unwrap_or(Value::Null);
        return;
    }
    let (parent, last) = path.rsplit_once('/').unwrap_or(("", path));
    let key = last.replace("~1", "/").replace("~0", "~");
    if v.pointer(parent).is_none() {
        put(v, parent, Some(Value::Object(Map::new())));
    }
    match v.pointer_mut(parent) {
        Some(Value::Object(m)) => match new {
            Some(x) => {
                m.insert(key, x);
            }
            None => {
                m.remove(&key);
            }
        },
        Some(Value::Array(items)) => {
            if let (Ok(i), Some(x)) = (key.parse::<usize>(), new)
                && i < items.len()
            {
                items[i] = x;
            }
        }
        _ => {}
    }
}

/// `value` with `path` set to what `source` holds there (removed if absent).
pub(crate) fn replace_at(value: Option<Value>, path: &str, source: Option<&Value>) -> Option<Value> {
    let new = source.and_then(|s| get(s, path)).cloned();
    if path.is_empty() {
        return new;
    }
    let mut v = value.unwrap_or(Value::Object(Map::new()));
    put(&mut v, path, new);
    Some(v)
}

/// The outcome of merging one component.
pub(crate) struct Merged {
    /// What the component becomes (None: removed).
    pub value: Option<Value>,
    /// Fields both sides changed differently: (pointer, whether theirs won).
    pub conflicts: Vec<(String, bool)>,
}

/// Merges their change to one component (`before` → `theirs`) into `ours`,
/// field by field. Each field ends up with the value of its latest write by
/// stamp, so every copy agrees whatever order changes arrive in:
/// `newer(path)` says whether their change is later than the last write this
/// copy holds for that field. A field both sides changed to different values
/// is reported (with which side won), so the losing value is not lost.
pub(crate) fn merge(before: Option<&Value>, theirs: Option<&Value>, ours: Option<&Value>, newer: impl Fn(&str) -> bool) -> Merged {
    if theirs == ours {
        return Merged { value: ours.cloned(), conflicts: vec![] };
    }
    let paths = match (before, theirs) {
        (Some(b), Some(t)) => {
            let mut p = Vec::new();
            changed_paths(b, t, "", &mut p);
            p
        }
        _ => vec![String::new()], // added or removed whole
    };
    let at = |v: Option<&Value>, p: &str| v.and_then(|v| get(v, p)).cloned();
    let mut value = ours.cloned();
    let mut conflicts = Vec::new();
    for p in paths {
        let (b, t, o) = (at(before, &p), at(theirs, &p), at(ours, &p));
        if t == o {
            continue;
        }
        let win = newer(&p);
        // A clash: both moved this field away from where their change started.
        if o != b {
            conflicts.push((p.clone(), win));
        }
        if win {
            value = replace_at(value, &p, theirs);
        }
    }
    Merged { value, conflicts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_field_takes_its_latest_write() {
        let before = json!({ "w": 100, "h": 50 });
        let theirs = json!({ "w": 120, "h": 50 });
        // We changed h earlier; their w is newer: both survive.
        let m = merge(Some(&before), Some(&theirs), Some(&json!({ "w": 100, "h": 60 })), |_| true);
        assert_eq!(m.value, Some(json!({ "w": 120, "h": 60 })));
        assert!(m.conflicts.is_empty());
        // We changed w too, later than theirs: ours stays, the clash is reported.
        let m = merge(Some(&before), Some(&theirs), Some(&json!({ "w": 90, "h": 50 })), |_| false);
        assert_eq!(m.value, Some(json!({ "w": 90, "h": 50 })));
        assert_eq!(m.conflicts, vec![("/w".to_string(), false)]);
    }

    #[test]
    fn records_in_lists_merge_item_by_item() {
        let p = |a: i64, b: i64| json!({ "params": [{ "key": "a", "value": a }, { "key": "b", "value": b }] });
        let m = merge(Some(&p(1, 2)), Some(&p(5, 2)), Some(&p(1, 7)), |_| true);
        assert_eq!(m.value, Some(p(5, 7)));
    }

    #[test]
    fn removal_follows_the_stamps_too() {
        let before = json!({ "w": 1 });
        let m = merge(Some(&before), None, Some(&json!({ "w": 2 })), |_| false);
        assert_eq!((m.value, m.conflicts), (Some(json!({ "w": 2 })), vec![(String::new(), false)]));
        assert_eq!(merge(Some(&before), None, Some(&before), |_| true).value, None);
    }
}
