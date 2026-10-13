//! Items brought in from another project: copied, or linked so they follow
//! the project they came from.
//!
//! The engine that serves both projects reads the source and hands these
//! commands a snapshot of what to bring: every entity the picked items need,
//! with all its components as JSON, named by its ids there. The commands are
//! generic over components: they give every entity a new id here, rewrite
//! every reference to the source's ids inside every component (fields, map
//! keys, formulas such as `#<id>.key`, program text), and write the result as
//! one change (one undo step, synced to teammates like any other).
//!
//! A linked insert also records a [`Link`] (where the items came from, the
//! source revision they match, source id → id here) and marks each entity
//! it made with [`Linked`]: the hash of what each component was when last
//! taken from the source. Updating compares three values per component (the
//! source now, the last one taken, and this project's own) so the source's
//! changes arrive, local edits the source did not touch stay, and fields
//! named in `keep` (where a part is placed, a name given here to avoid a
//! clash) always stay as they are here. Breaking the link drops the record
//! and the marks: what remains is an ordinary local copy.

use crate::{
    command::Command,
    error::{CommandError, ErrorCode},
    ids::{IdIndex, StableId},
    registry::Persist,
    tx::Tx,
};
use bevy_ecs::component::Component;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Where linked items came from, and how they map to this project.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Link {
    /// The source project (its file name).
    pub project: String,
    /// Its title when last synced (for people).
    #[serde(default)]
    pub title: String,
    /// What was picked there (source ids); empty with `whole`.
    #[serde(default)]
    pub roots: Vec<StableId>,
    /// The whole project was brought.
    #[serde(default)]
    pub whole: bool,
    /// Every entity that follows the source: source id → id here.
    #[serde(default)]
    pub ids: BTreeMap<StableId, StableId>,
    /// Source entities matched to ones already here (a material identical
    /// to one this project has): used, never changed by updates.
    #[serde(default)]
    pub shared: BTreeMap<StableId, StableId>,
    /// Entities made here for the insert (a placement of a sub-assembly
    /// picked from inside an assembly): local, never touched by updates.
    #[serde(default)]
    pub placed: Vec<StableId>,
    /// The source's revision these match.
    #[serde(default)]
    pub revision: u64,
    /// Take the source's changes as soon as they happen.
    #[serde(default)]
    pub auto_update: bool,
    /// The engine that inserted it (the one that updates automatically, so
    /// two teammates' engines never both bring the same change).
    #[serde(default)]
    pub engine: String,
    /// When it was inserted and last updated (ms since 1970).
    #[serde(default)]
    pub inserted: u64,
    #[serde(default)]
    pub synced: u64,
}

impl Persist for Link {
    const KIND: &'static str = "core.link";
}

/// On an entity that follows a source project.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Linked {
    /// The [`Link`] it belongs to.
    pub link: StableId,
    /// Its id in the source project.
    pub source: StableId,
    /// Hash of each component as last taken from the source (fields kept
    /// here left out).
    #[serde(default)]
    pub base: BTreeMap<String, String>,
    /// Fields that stay as they are here, by kind.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keep: BTreeMap<String, Vec<String>>,
}

impl Persist for Linked {
    const KIND: &'static str = "core.linked";
}

/// A source entity: every component as JSON, by kind.
pub type Snapshot = BTreeMap<StableId, BTreeMap<String, Value>>;

fn bad(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, message)
}

// ---- Ids ------------------------------------------------------------------------

/// FNV-1a, 64 bits.
fn fnv(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325 ^ seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The id an entity of the source gets here: made from the insert's id and
/// its source id, so every copy updating the same link names a newly arrived
/// entity the same (their changes then merge into one entity, not two).
/// Starts with the insert's time, like the engine's own ids.
pub fn derived_id(base: StableId, source: StableId) -> StableId {
    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(base.0.as_bytes());
    key.extend_from_slice(source.0.as_bytes());
    let (a, b) = (fnv(1, &key), fnv(2, &key));
    let mut out = [0u8; 16];
    out[..6].copy_from_slice(&base.0.as_bytes()[..6]);
    out[6..8].copy_from_slice(&a.to_be_bytes()[..2]);
    out[8..16].copy_from_slice(&b.to_be_bytes());
    out[6] = (out[6] & 0x0f) | 0x70; // version 7
    out[8] = (out[8] & 0x3f) | 0x80; // RFC 4122 variant
    StableId(uuid::Uuid::from_bytes(out))
}

/// Whether `s[i..i + 36]` is a hyphenated UUID.
fn uuid_at(b: &[u8], i: usize) -> bool {
    if i + 36 > b.len() {
        return false;
    }
    (0..36).all(|k| match k {
        8 | 13 | 18 | 23 => b[i + k] == b'-',
        _ => b[i + k].is_ascii_hexdigit(),
    })
}

/// Every UUID written anywhere in a string.
pub fn uuids_in(s: &str, out: &mut Vec<StableId>) {
    let b = s.as_bytes();
    if b.len() < 36 {
        return;
    }
    let mut i = 0;
    while i + 36 <= b.len() {
        if uuid_at(b, i) && (i == 0 || !b[i - 1].is_ascii_hexdigit()) && b.get(i + 36).is_none_or(|c| !c.is_ascii_hexdigit()) {
            if let Ok(u) = uuid::Uuid::parse_str(&s[i..i + 36]) {
                out.push(StableId(u));
            }
            i += 36;
        } else {
            i += 1;
        }
    }
}

/// Every UUID in a value: in strings and in map keys.
pub fn references(v: &Value, out: &mut Vec<StableId>) {
    match v {
        Value::String(s) => uuids_in(s, out),
        Value::Array(a) => a.iter().for_each(|x| references(x, out)),
        Value::Object(m) => {
            for (k, x) in m {
                uuids_in(k, out);
                references(x, out);
            }
        }
        _ => {}
    }
}

fn remap_str(s: &str, map: &HashMap<String, String>) -> Option<String> {
    let b = s.as_bytes();
    if b.len() < 36 {
        return None;
    }
    let mut out: Option<String> = None;
    let (mut i, mut copied) = (0, 0);
    while i + 36 <= b.len() {
        if uuid_at(b, i) && (i == 0 || !b[i - 1].is_ascii_hexdigit()) && b.get(i + 36).is_none_or(|c| !c.is_ascii_hexdigit())
            && let Some(to) = map.get(&s[i..i + 36].to_ascii_lowercase())
        {
            let o = out.get_or_insert_with(|| String::with_capacity(s.len()));
            o.push_str(&s[copied..i]);
            o.push_str(to);
            i += 36;
            copied = i;
        } else {
            i += 1;
        }
    }
    out.map(|mut o| {
        o.push_str(&s[copied..]);
        o
    })
}

/// The value with every source id it names (anywhere) replaced by its id here.
pub fn remap(v: &Value, map: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => remap_str(s, map).map(Value::String).unwrap_or_else(|| v.clone()),
        Value::Array(a) => Value::Array(a.iter().map(|x| remap(x, map)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (remap_str(k, map).unwrap_or_else(|| k.clone()), remap(x, map))).collect::<Map<String, Value>>()),
        _ => v.clone(),
    }
}

// ---- Hashes -----------------------------------------------------------------------

/// JSON with keys sorted at every level (the same text for the same value,
/// whatever order its maps were built in).
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (n, k) in keys.into_iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (n, x) in a.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        _ => out.push_str(&v.to_string()),
    }
}

/// Hash of a component, leaving out the fields kept here.
pub fn hash_of(v: &Value, keep: &[String]) -> String {
    let mut text = String::new();
    match v {
        Value::Object(m) if !keep.is_empty() => {
            let trimmed: Map<String, Value> = m.iter().filter(|(k, _)| !keep.contains(k)).map(|(k, x)| (k.clone(), x.clone())).collect();
            canonical(&Value::Object(trimmed), &mut text);
        }
        _ => canonical(v, &mut text),
    }
    format!("{:016x}", fnv(0, text.as_bytes()))
}

/// The source's value with the kept fields as they are here.
fn with_kept(mut new: Value, current: Option<&Value>, keep: &[String]) -> Value {
    if let (Value::Object(m), Some(Value::Object(cur))) = (&mut new, current) {
        for k in keep {
            match cur.get(k) {
                Some(v) => {
                    m.insert(k.clone(), v.clone());
                }
                None => {
                    m.remove(k);
                }
            }
        }
    }
    new
}

// ---- Shared steps -------------------------------------------------------------------

/// What the server decides about the source's entities (see the commands).
#[derive(Deserialize, JsonSchema, Default)]
pub(crate) struct Policy {
    /// Fields that stay as they are here when the source changes (where a
    /// placed item sits), by source id then kind.
    #[serde(default)]
    keep: BTreeMap<StableId, BTreeMap<String, Vec<String>>>,
    /// Kinds of each source entity whose `name` must not repeat a name of
    /// the same kind already here: renamed "Name (Source title)".
    #[serde(default)]
    rename: BTreeMap<StableId, Vec<String>>,
    /// Kinds that, when an entity holds only that kind and one identical to
    /// it is already here (a material), use the one here instead.
    #[serde(default)]
    reuse: Vec<String>,
}

/// An entity already here that is identical to a source entity of a reused kind.
fn reusable(tx: &Tx, comps: &BTreeMap<String, Value>, reuse: &[String], index: &mut Option<HashMap<(String, String), StableId>>) -> Result<Option<StableId>, CommandError> {
    let [(kind, value)] = comps.iter().collect::<Vec<_>>()[..] else { return Ok(None) };
    if !reuse.contains(kind) {
        return Ok(None);
    }
    // Built once: every entity here holding exactly one reused kind, by its value.
    if index.is_none() {
        let mut found = HashMap::new();
        let ids: Vec<StableId> = tx.world().resource::<IdIndex>().iter().map(|(id, _)| id).collect();
        for id in ids {
            let kinds = tx.kinds_of(id);
            if let [k] = kinds[..]
                && reuse.iter().any(|r| r == k)
                && let Some(v) = tx.get_json(id, k)?
            {
                found.entry((k.to_string(), hash_of(&v, &[]))).or_insert(id);
            }
        }
        *index = Some(found);
    }
    let v = tx.normalize(kind, value.clone())?;
    Ok(index.as_ref().unwrap().get(&(kind.clone(), hash_of(&v, &[]))).copied())
}

/// Names already used here, by kind.
fn names_here(tx: &Tx, kinds: &BTreeSet<String>) -> Result<HashMap<String, BTreeSet<String>>, CommandError> {
    let mut out: HashMap<String, BTreeSet<String>> = HashMap::new();
    if kinds.is_empty() {
        return Ok(out);
    }
    let ids: Vec<StableId> = tx.world().resource::<IdIndex>().iter().map(|(id, _)| id).collect();
    for id in ids {
        for k in kinds {
            if let Some(Value::String(n)) = tx.get_json(id, k)?.as_ref().and_then(|v| v.get("name")) {
                out.entry(k.clone()).or_default().insert(n.clone());
            }
        }
    }
    Ok(out)
}

/// A name not yet used: "Bracket (Robot arm)", then "Bracket (Robot arm) 2"…
fn fresh_name(name: &str, title: &str, used: &BTreeSet<String>) -> String {
    let base = if title.is_empty() { format!("{name} (inserted)") } else { format!("{name} ({title})") };
    if !used.contains(&base) {
        return base;
    }
    (2..).map(|n| format!("{base} {n}")).find(|c| !used.contains(c)).unwrap()
}

/// Brings new source entities in: reused or made (with derived ids).
struct Made {
    /// Source id → id here, for every source entity named.
    map: HashMap<String, String>,
    created: Vec<(StableId, StableId)>,
    shared: Vec<(StableId, StableId)>,
}

fn make(tx: &mut Tx, base: StableId, new: &[(&StableId, &BTreeMap<String, Value>)], reuse: &[String], mut map: HashMap<String, String>) -> Result<Made, CommandError> {
    let mut index = None;
    let (mut created, mut shared) = (Vec::new(), Vec::new());
    for (source, comps) in new {
        if let Some(here) = reusable(tx, comps, reuse, &mut index)? {
            map.insert(source.to_string(), here.to_string());
            shared.push((**source, here));
            continue;
        }
        let id = derived_id(base, **source);
        if !tx.exists(id) {
            tx.create_with(id)?;
        }
        map.insert(source.to_string(), id.to_string());
        created.push((**source, id));
    }
    Ok(Made { map, created, shared })
}

/// Writes a newly made entity's components (and its mark, when linked).
#[allow(clippy::too_many_arguments)]
fn fill(tx: &mut Tx, link: Option<StableId>, source: StableId, id: StableId, comps: &BTreeMap<String, Value>, map: &HashMap<String, String>, policy: &Policy, title: &str, used: &mut HashMap<String, BTreeSet<String>>, renamed: &mut Vec<Value>) -> Result<(), CommandError> {
    let mut keep: BTreeMap<String, Vec<String>> = policy.keep.get(&source).cloned().unwrap_or_default();
    let mut base = BTreeMap::new();
    for (kind, value) in comps {
        let mut v = remap(value, map);
        if policy.rename.get(&source).is_some_and(|ks| ks.contains(kind))
            && let Some(Value::String(name)) = v.get("name").cloned()
        {
            let names = used.entry(kind.clone()).or_default();
            if names.contains(&name) {
                let to = fresh_name(&name, title, names);
                v["name"] = json!(to);
                names.insert(to.clone());
                keep.entry(kind.clone()).or_default().push("name".into());
                renamed.push(json!({ "id": id, "kind": kind, "from": name, "to": to }));
            } else {
                names.insert(name);
            }
        }
        tx.set_json(id, kind, v)?;
        let stored = tx.get_json(id, kind)?.unwrap_or(Value::Null);
        base.insert(kind.clone(), hash_of(&stored, keep.get(kind).map(Vec::as_slice).unwrap_or(&[])));
    }
    if let Some(link) = link {
        tx.set(id, Linked { link, source, base, keep })?;
    }
    Ok(())
}

fn rename_kinds(policy: &Policy) -> BTreeSet<String> {
    policy.rename.values().flatten().cloned().collect()
}

fn link_of(tx: &Tx, id: StableId) -> Result<Link, CommandError> {
    tx.get::<Link>(id).cloned().or_else(|| tx.get::<Linked>(id).and_then(|m| tx.get::<Link>(m.link)).cloned()).ok_or_else(|| bad(format!("{id} is not a link to another project, nor an item linked from one")))
}

fn link_id(tx: &Tx, id: StableId) -> Result<StableId, CommandError> {
    if tx.get::<Link>(id).is_some() {
        return Ok(id);
    }
    tx.get::<Linked>(id).map(|m| m.link).filter(|l| tx.get::<Link>(*l).is_some()).ok_or_else(|| bad(format!("{id} is not a link to another project, nor an item linked from one")))
}

// ---- Commands -------------------------------------------------------------------------

/// `link.insert`: brings entities from another project into this one, as a
/// local copy or linked to follow it. The engine reads the source and fills
/// this in (`POST /link/insert`); every reference among them is rewritten to
/// the new ids, so inserting twice, or from two projects, never collides.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct Insert {
    /// The source project's name.
    project: String,
    /// Its title (renamed clashes and the link show it).
    #[serde(default)]
    title: String,
    /// The source's revision this snapshot is of.
    #[serde(default)]
    revision: u64,
    /// The source entities to bring, by id there: every component as JSON.
    entities: Snapshot,
    /// What was picked there (the rest came along because these need it).
    #[serde(default)]
    roots: Vec<StableId>,
    /// The whole project was picked.
    #[serde(default)]
    whole: bool,
    /// Follow the source (true), or make an independent copy (false).
    #[serde(default)]
    linked: bool,
    #[serde(default)]
    auto_update: bool,
    /// New entities to make here as well, components naming source ids (a
    /// placement for a sub-assembly picked from inside an assembly).
    #[serde(default)]
    extra: Vec<BTreeMap<String, Value>>,
    #[serde(default, flatten)]
    policy: Policy,
    /// The engine inserting (updates automatically), and when (ms).
    #[serde(default)]
    engine: String,
    #[serde(default)]
    time: u64,
}

impl Command for Insert {
    const NAME: &'static str = "link.insert";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.entities.is_empty() {
            return Err(bad("nothing to insert"));
        }
        if self.entities.values().any(|c| c.contains_key(Link::KIND) || c.contains_key(Linked::KIND)) {
            return Err(bad("links of the source come as plain items, not as links"));
        }
        let link = if self.linked { Some(tx.create()) } else { None };
        // A copy's ids come from a one-off base, a link's from the link (see `derived_id`).
        let base = link.unwrap_or_default();
        let mut used = names_here(tx, &rename_kinds(&self.policy))?;
        let new: Vec<(&StableId, &BTreeMap<String, Value>)> = self.entities.iter().collect();
        let made = make(tx, base, &new, &self.policy.reuse, HashMap::new())?;
        let mut renamed = Vec::new();
        for (source, id) in &made.created {
            fill(tx, link, *source, *id, &self.entities[source], &made.map, &self.policy, &self.title, &mut used, &mut renamed)?;
        }
        let mut placed = Vec::new();
        for comps in &self.extra {
            let id = tx.create();
            for (kind, value) in comps {
                tx.set_json(id, kind, remap(value, &made.map))?;
            }
            placed.push(id);
        }
        let here = |s: &StableId| made.map.get(&s.to_string()).cloned();
        let roots: Vec<String> = self.roots.iter().filter_map(here).collect();
        if let Some(link) = link {
            tx.set(
                link,
                Link {
                    project: self.project.clone(),
                    title: self.title.clone(),
                    roots: self.roots.clone(),
                    whole: self.whole,
                    ids: made.created.iter().copied().collect(),
                    shared: made.shared.iter().copied().collect(),
                    placed: placed.clone(),
                    revision: self.revision,
                    auto_update: self.auto_update,
                    engine: self.engine,
                    inserted: self.time,
                    synced: self.time,
                },
            )?;
        }
        Ok(json!({
            "link": link,
            "project": self.project,
            "linked": self.linked,
            "created": made.created.len(),
            "reused": made.shared.iter().map(|(_, id)| id).collect::<Vec<_>>(),
            "roots": roots,
            "placed": placed,
            "renamed": renamed,
            "ids": made.created.iter().map(|(s, id)| (s.to_string(), json!(id))).collect::<Map<String, Value>>(),
        }))
    }
}

/// `link.update`: takes the source project's current state into a link.
/// Its changes arrive; edits made here to things the source did not change
/// stay; where both changed the same component the source's wins (and is
/// reported); kept fields (placement, clash names) stay as they are here.
/// What the source no longer has is removed here; what it gained arrives.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct Update {
    /// The link (or any item linked by it).
    link: StableId,
    #[serde(default)]
    title: String,
    #[serde(default)]
    revision: u64,
    /// The source entities the link covers now (see `link.insert`).
    entities: Snapshot,
    #[serde(default, flatten)]
    policy: Policy,
    #[serde(default)]
    time: u64,
}

impl Command for Update {
    const NAME: &'static str = "link.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let link = link_id(tx, self.link)?;
        let mut rec = link_of(tx, link)?;
        if self.entities.values().any(|c| c.contains_key(Link::KIND) || c.contains_key(Linked::KIND)) {
            return Err(bad("links of the source come as plain items, not as links"));
        }
        let mut map: HashMap<String, String> = rec.ids.iter().chain(rec.shared.iter()).map(|(s, l)| (s.to_string(), l.to_string())).collect();
        // New in the source: made here (ids from the link, so every copy agrees).
        let new: Vec<(&StableId, &BTreeMap<String, Value>)> = self.entities.iter().filter(|(s, _)| !rec.ids.contains_key(s) && !rec.shared.contains_key(s)).collect();
        let made = make(tx, link, &new, &self.policy.reuse, std::mem::take(&mut map))?;
        map = made.map;
        let title = if self.title.is_empty() { rec.title.clone() } else { self.title.clone() };
        let mut used = names_here(tx, &rename_kinds(&self.policy))?;
        let mut renamed = Vec::new();
        for (source, id) in &made.created {
            fill(tx, Some(link), *source, *id, &self.entities[source], &map, &self.policy, &title, &mut used, &mut renamed)?;
        }
        let (mut changed, mut conflicts, mut gone_here) = (Vec::new(), Vec::new(), 0usize);
        for (source, id) in rec.ids.clone() {
            let Some(comps) = self.entities.get(&source) else { continue };
            if !tx.exists(id) {
                // Deleted here: stays deleted.
                gone_here += 1;
                continue;
            }
            let mut mark = tx.get::<Linked>(id).cloned().unwrap_or(Linked { link, source, ..Default::default() });
            let mut keep = mark.keep.clone();
            for (kind, fields) in self.policy.keep.get(&source).into_iter().flatten() {
                let k = keep.entry(kind.clone()).or_default();
                for f in fields {
                    if !k.contains(f) {
                        k.push(f.clone());
                    }
                }
            }
            let kinds: BTreeSet<String> = comps.keys().cloned().chain(mark.base.keys().cloned()).collect();
            let mut touched = false;
            for kind in kinds {
                let kept = keep.get(&kind).map(Vec::as_slice).unwrap_or(&[]);
                let current = tx.get_json(id, &kind)?;
                let new = match comps.get(&kind) {
                    Some(v) => Some(with_kept(tx.normalize(&kind, remap(v, &map))?, current.as_ref(), kept)),
                    None => None,
                };
                let new_hash = new.as_ref().map(|v| hash_of(v, kept));
                let base = mark.base.get(&kind).cloned();
                if new_hash == base {
                    continue; // the source did not change it: whatever is here stays
                }
                let local_hash = current.as_ref().map(|v| hash_of(v, kept));
                if local_hash != base && local_hash != new_hash {
                    conflicts.push(json!({ "id": id, "kind": kind }));
                }
                match new {
                    Some(v) => tx.set_json(id, &kind, v)?,
                    None => tx.remove_json(id, &kind)?,
                }
                match new_hash {
                    Some(h) => mark.base.insert(kind.clone(), h),
                    None => mark.base.remove(&kind),
                };
                touched = true;
            }
            if keep != mark.keep {
                mark.keep = keep;
                touched = true;
            }
            if touched {
                tx.set(id, mark)?;
                changed.push(id);
            }
        }
        // Gone from the source: gone here.
        let mut removed = Vec::new();
        for (source, id) in rec.ids.clone() {
            if self.entities.contains_key(&source) {
                continue;
            }
            rec.ids.remove(&source);
            if tx.exists(id) {
                tx.delete(id)?;
                removed.push(id);
            }
        }
        rec.shared.retain(|s, _| self.entities.contains_key(s));
        rec.ids.extend(made.created.iter().copied());
        rec.shared.extend(made.shared.iter().copied());
        let any = !made.created.is_empty() || !changed.is_empty() || !removed.is_empty() || !made.shared.is_empty();
        rec.revision = self.revision;
        rec.title = title;
        if any {
            rec.synced = self.time;
        }
        tx.set(link, rec)?;
        Ok(json!({
            "link": link,
            "created": made.created.iter().map(|(_, id)| id).collect::<Vec<_>>(),
            "changed": changed,
            "removed": removed,
            "conflicts": conflicts,
            "renamed": renamed,
            "deleted_here": gone_here,
            "up_to_date": !any,
        }))
    }
}

/// `link.break`: the link's items become an ordinary local copy (they stop
/// following the source; nothing else changes).
#[derive(Deserialize, JsonSchema)]
pub(crate) struct Break {
    /// The link, or any item linked by it.
    link: StableId,
}

impl Command for Break {
    const NAME: &'static str = "link.break";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let link = link_id(tx, self.link)?;
        let rec = link_of(tx, link)?;
        let mut items = 0;
        for id in rec.ids.values() {
            if tx.get::<Linked>(*id).is_some_and(|m| m.link == link) {
                tx.remove::<Linked>(*id)?;
                items += 1;
            }
        }
        tx.delete(link)?;
        Ok(json!({ "link": link, "project": rec.project, "items": items }))
    }
}

/// `link.settings`: whether a link takes its source's changes by itself.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct Settings {
    /// The link, or any item linked by it.
    link: StableId,
    auto_update: bool,
    /// The engine that updates it (this one, when turning it on).
    #[serde(default)]
    engine: Option<String>,
}

impl Command for Settings {
    const NAME: &'static str = "link.settings";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let link = link_id(tx, self.link)?;
        let mut rec = link_of(tx, link)?;
        rec.auto_update = self.auto_update;
        if let Some(e) = self.engine {
            rec.engine = e;
        }
        tx.set(link, rec)?;
        Ok(json!({ "link": link, "auto_update": self.auto_update }))
    }
}
