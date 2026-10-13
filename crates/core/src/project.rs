use crate::{
    apply::{apply_atomic, rollback},
    command::{Actor, Envelope, SCHEMA_VERSION},
    derived::{DependsOn, Status},
    error::{CommandError, ErrorCode},
    ids::{IdIndex, Revision, StableId},
    journal::{ChangeSet, Op, Stamp, WireChangeSet, WireOp},
    registry::{Persist, Registry},
    tx::Tx,
};
use bevy_ecs::{component::Component, entity::Entity, world::World};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use serde_json::{Value, json};
use smallvec::SmallVec;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Committed {
    pub revision: Revision,
    pub output: Value,
    pub op_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub base_revision: Revision,
    pub output: Value,
    pub ops: Vec<WireOp>,
}

/// A peer's change set merged into this copy: the change it made here (if
/// any), and the fields both sides changed differently (ours kept).
#[derive(Clone, Debug, Serialize)]
pub struct Merge {
    pub committed: Option<Committed>,
    pub conflicts: Vec<crate::merge::Conflict>,
}

#[derive(Clone, Copy)]
enum Origin {
    Command,
    Undo,
    Redo,
    /// A change made on this copy, replayed from its log.
    Replay,
    /// A change made on another copy, merged.
    Remote,
}

#[derive(Default)]
struct Dirty {
    /// Persistent kinds changed directly on the entity.
    kinds: SmallVec<[&'static str; 4]>,
    /// A dependency changed.
    upstream: bool,
    /// Created, or its dependency list changed: recompute everything.
    structural: bool,
}

/// One open project: the shared space, its journal and its derived state.
pub struct Project {
    id: String,
    world: World,
    registry: Arc<Registry>,
    revision: Revision,
    journal: Vec<ChangeSet>,
    /// Reverse dependency edges: entity -> entities that depend on it.
    dependents: FxHashMap<StableId, FxHashSet<StableId>>,
    committed: FxHashMap<String, Committed>,
    undo: FxHashMap<String, Vec<usize>>,
    redo: FxHashMap<String, Vec<usize>>,
    /// This copy's engine, stamped on every change made here.
    origin: String,
    /// Lamport clock: above every stamp seen so far.
    clock: u64,
    /// Who last wrote each field (entity, kind, JSON pointer), to pick the
    /// same winner on every copy when two changes clash.
    writers: FxHashMap<(StableId, &'static str), Vec<(String, Stamp)>>,
    /// Every change set in the order this copy took it, as shared with peers
    /// and saved: changes made here, and peers' changes exactly as received.
    log: Vec<WireChangeSet>,
    /// Every change (by stamp) that wrote to each entity here, winning or not:
    /// a delete that had not seen one of them loses to it.
    touched: FxHashMap<StableId, Vec<Stamp>>,
    /// Latest clock taken from each engine (this copy's version vector).
    known: BTreeMap<String, u64>,
    /// The version vector of the peer change being recorded (else ours is used).
    incoming_seen: Option<BTreeMap<String, u64>>,
    /// Peers' change sets that arrived before something they build on.
    waiting: Vec<WireChangeSet>,
    /// Derived results wait for [`Project::finish_replay`] (while a saved log is replayed).
    deferring: bool,
}

impl Project {
    pub fn new(id: impl Into<String>, registry: Arc<Registry>) -> Self {
        let mut world = World::new();
        world.insert_resource(IdIndex::default());
        world.insert_resource(crate::versions::Clocks { after: vec![BTreeMap::new()] });
        Self {
            id: id.into(),
            world,
            registry,
            revision: Revision::default(),
            journal: Vec::new(),
            dependents: FxHashMap::default(),
            committed: FxHashMap::default(),
            undo: FxHashMap::default(),
            redo: FxHashMap::default(),
            origin: "local".into(),
            deferring: false,
            clock: 0,
            writers: FxHashMap::default(),
            log: Vec::new(),
            touched: FxHashMap::default(),
            known: BTreeMap::new(),
            incoming_seen: None,
            waiting: Vec::new(),
        }
    }

    /// Names this copy's engine (stamped on changes made here).
    pub fn set_origin(&mut self, origin: impl Into<String>) {
        self.origin = origin.into();
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The latest clock this copy has taken from each engine. A peer sends
    /// what is past these; everything at or below is already here.
    pub fn known(&self) -> &BTreeMap<String, u64> {
        &self.known
    }

    /// Takes a peer's change set: merges it once everything it builds on is
    /// here (causal order), holding it back until then. Returns every merge
    /// that happened, possibly several waiting ones it unblocked.
    pub fn receive(&mut self, wire: WireChangeSet) -> Result<Vec<Merge>, CommandError> {
        if self.committed.contains_key(&wire.command_id) || self.waiting.iter().any(|w| w.command_id == wire.command_id) {
            return Ok(Vec::new());
        }
        self.waiting.push(wire);
        let mut merged = Vec::new();
        while let Some(i) = self.waiting.iter().position(|w| self.ready(w)) {
            let w = self.waiting.remove(i);
            merged.push(self.merge_remote(w)?);
        }
        Ok(merged)
    }

    /// Change sets held back because something they build on has not arrived.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    fn ready(&self, w: &WireChangeSet) -> bool {
        w.seen.iter().all(|(origin, clock)| self.known.get(origin).copied().unwrap_or(0) >= *clock)
    }

    /// Every change set in the order this copy took it (what to save and to share).
    pub fn log(&self) -> &[WireChangeSet] {
        &self.log
    }

    /// Holds derived results back while a saved log is replayed: each entry
    /// would otherwise recompute what it touches (a project with motion
    /// requirements re-ran every sweep once per saved change).
    pub fn begin_replay(&mut self) {
        self.deferring = true;
    }

    /// Computes every derived result once, after [`Project::begin_replay`].
    pub fn finish_replay(&mut self) {
        if !std::mem::take(&mut self.deferring) {
            return;
        }
        let ops: Vec<Op> = self.world.resource::<IdIndex>().iter().map(|(id, _)| Op::Create { id }).collect();
        self.recompute(&ops);
    }

    /// Replays one saved log entry: a change made here, or a peer's change merged again.
    pub fn replay(&mut self, wire: WireChangeSet) -> Result<(), CommandError> {
        if wire.merged {
            self.merge_remote(wire).map(|_| ())
        } else {
            self.apply_remote(wire).map(|_| ())
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Read access to the whole shared space.
    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn entity(&self, id: StableId) -> Option<Entity> {
        self.world.resource::<IdIndex>().get(id)
    }

    pub fn entity_count(&self) -> usize {
        self.world.resource::<IdIndex>().len()
    }

    pub fn get<T: Component>(&self, id: StableId) -> Option<&T> {
        self.world.get::<T>(self.entity(id)?)
    }

    pub fn status(&self, id: StableId) -> Option<&Status> {
        self.get::<Status>(id)
    }

    pub fn journal(&self) -> &[ChangeSet] {
        &self.journal
    }

    /// Change sets a peer at `revision` is missing.
    pub fn changes_since(&self, revision: Revision) -> &[ChangeSet] {
        &self.journal[(revision.0 as usize).min(self.journal.len())..]
    }

    /// Runs a command and reports its effect without keeping it.
    pub fn preview(&mut self, envelope: &Envelope) -> Result<Preview, CommandError> {
        self.check(envelope)?;
        let (output, ops) = self.run(envelope)?;
        rollback(&mut self.world, &self.registry, &ops);
        Ok(Preview { base_revision: self.revision, output, ops: ops.iter().map(Op::to_wire).collect() })
    }

    pub fn commit(&mut self, envelope: Envelope) -> Result<Committed, CommandError> {
        if let Some(done) = self.committed.get(&envelope.command_id) {
            return Ok(done.clone());
        }
        self.check(&envelope)?;
        let (output, ops) = self.run(&envelope)?;
        Ok(self.record(envelope.command_id, envelope.operation, envelope.actor, ops, output, Origin::Command))
    }

    /// Reverts the actor's latest change. Fails with `UNDO_CONFLICT` if
    /// someone has since changed the same data.
    pub fn undo(&mut self, actor: &Actor) -> Result<Committed, CommandError> {
        let index = *self
            .undo
            .get(&actor.user_id)
            .and_then(|s| s.last())
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToUndo, "nothing to undo"))?;
        let undone = self.journal[index].revision;
        self.revert(index, actor, "core.undo", Origin::Undo, json!({ "undid": undone }))?;
        self.undo.get_mut(&actor.user_id).map(Vec::pop);
        Ok(self.committed_for(self.journal.len() - 1))
    }

    pub fn redo(&mut self, actor: &Actor) -> Result<Committed, CommandError> {
        let index = *self
            .redo
            .get(&actor.user_id)
            .and_then(|s| s.last())
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToRedo, "nothing to redo"))?;
        let undo_revision = self.journal[index].revision;
        self.revert(index, actor, "core.redo", Origin::Redo, json!({ "redid": undo_revision }))?;
        self.redo.get_mut(&actor.user_id).map(Vec::pop);
        Ok(self.committed_for(self.journal.len() - 1))
    }

    /// Applies a peer's change set made on top of this project's current revision.
    pub fn apply_remote(&mut self, wire: WireChangeSet) -> Result<Committed, CommandError> {
        if let Some(done) = self.committed.get(&wire.command_id) {
            return Ok(done.clone());
        }
        if wire.base_revision != self.revision {
            return Err(CommandError::new(
                ErrorCode::StaleRevision,
                format!("change set is based on {:?}; project is at {:?}", wire.base_revision, self.revision),
            ));
        }
        let stamp = wire.stamp.clone().unwrap_or_else(|| Stamp { clock: self.clock + 1, origin: self.origin.clone() });
        let had_seen = !wire.seen.is_empty() || wire.stamp.is_some();
        let change = self.registry.changeset_from_wire(wire)?;
        if had_seen {
            self.incoming_seen = Some(change.seen.clone());
        }
        apply_atomic(&mut self.world, &self.registry, &change.ops, true)?;
        Ok(self.record_stamped(change.command_id, change.operation, change.actor, change.ops, Value::Null, Origin::Replay, Some(stamp)))
    }

    /// Merges a change set made on another member's copy, field by field (see
    /// [`crate::merge`]): it applies whatever does not clash with this copy,
    /// keeps ours where both changed the same field, and reports those as
    /// conflicts. Merging a change set twice does nothing the second time.
    pub fn merge_remote(&mut self, wire: WireChangeSet) -> Result<Merge, CommandError> {
        use crate::merge::{Conflict, merge};
        if let Some(done) = self.committed.get(&wire.command_id) {
            return Ok(Merge { committed: Some(done.clone()), conflicts: Vec::new() });
        }
        let stamp = wire.stamp.clone().unwrap_or_default();
        // Seeing a change moves our clock past it, even if it changes nothing here:
        // whatever we do next is later than it on every copy.
        self.clock = self.clock.max(stamp.clock);
        let conflict = |id: StableId, kind: &str, path: String, ours: Value, theirs: Value| Conflict {
            kept: String::new(),
            id,
            kind: kind.to_string(),
            path,
            ours,
            theirs,
            by: wire.actor.clone(),
            command_id: wire.command_id.clone(),
        };
        let mut conflicts = Vec::new();

        // An entity they deleted that we have changed since stays, as a conflict.
        let mut kept = FxHashSet::default();
        for op in &wire.ops {
            let WireOp::Delete { id } = op else { continue };
            let Some(now) = self.entity_state(*id) else { continue };
            let theirs_before: BTreeMap<&str, &Value> = wire
                .ops
                .iter()
                .filter_map(|o| match o {
                    WireOp::Set { id: i, kind, before: Some(b), .. } if i == id => Some((kind.as_str(), b)),
                    _ => None,
                })
                .collect();
            let _ = theirs_before;
            let _ = now;
            // Kept if this copy holds a write to it the deleter had not seen.
            let unseen = self.touched.get(id).into_iter().flatten().any(|s| wire.seen.get(&s.origin).copied().unwrap_or(0) < s.clock);
            if unseen {
                kept.insert(*id);
                let mut c = conflict(*id, "", String::new(), json!("changed here"), json!("deleted"));
                c.kept = "ours".into();
                conflicts.push(c);
            }
        }

        let mut ops = Vec::new();
        // Values this change set has already produced here, by (entity, kind).
        let mut pending: FxHashMap<(StableId, String), Option<Value>> = FxHashMap::default();
        let mut created = FxHashSet::default();
        for op in wire.ops.iter().cloned() {
            match op {
                WireOp::Create { id } => {
                    if self.entity(id).is_none() && created.insert(id) {
                        ops.push(WireOp::Create { id });
                    }
                }
                WireOp::Delete { id } => {
                    if !kept.contains(&id) && (self.entity(id).is_some() || created.contains(&id)) {
                        ops.push(WireOp::Delete { id });
                        created.remove(&id);
                    }
                }
                WireOp::Set { id, kind, before, after } => {
                    if kept.contains(&id) {
                        continue;
                    }
                    let exists = self.entity(id).is_some() || created.contains(&id);
                    if !exists {
                        // We deleted it while they changed it. A change beats a delete on
                        // every copy (no one's work is lost): bring it back as it was
                        // when we deleted it, then merge their change on top.
                        let Some((last, delete_seen)) = self.last_state_before_delete(id) else { continue };
                        let delete_saw_it = delete_seen.get(&stamp.origin).copied().unwrap_or(0) >= stamp.clock;
                        if after.is_none() || delete_saw_it {
                            continue;
                        }
                        ops.push(WireOp::Create { id });
                        created.insert(id);
                        for (k, v) in last {
                            pending.insert((id, k.clone()), Some(v.clone()));
                            ops.push(WireOp::Set { id, kind: k, before: None, after: Some(v) });
                        }
                        let mut c = conflict(id, "", String::new(), json!("deleted here"), json!("changed"));
                        c.kept = "theirs".into();
                        conflicts.push(c);
                    }
                    let ours = match pending.get(&(id, kind.clone())) {
                        Some(v) => v.clone(),
                        None => self.component_json(id, &kind)?,
                    };
                    if after.is_some() {
                        self.touched.entry(id).or_default().push(stamp.clone());
                    }
                    // Every field takes its latest write by stamp, on every copy.
                    let m = merge(before.as_ref(), after.as_ref(), ours.as_ref(), |path| stamp > self.writer_of(id, &kind, path));
                    // Their write is the field's latest even where it set the value we
                    // already had: remember its stamp, or an older change could win here.
                    let mut written = Vec::new();
                    if let (Some(b), Some(a)) = (&before, &after) {
                        crate::merge::changed_paths(b, a, "", &mut written);
                    }
                    for p in written {
                        if stamp > self.writer_of(id, &kind, &p) {
                            self.note_writer(id, &kind, p, &stamp)?;
                        }
                    }
                    for (path, theirs_win) in m.conflicts {
                        let pick = |v: &Option<Value>| v.as_ref().and_then(|v| if path.is_empty() { Some(v.clone()) } else { v.pointer(&path).cloned() }).unwrap_or(Value::Null);
                        let mut c = conflict(id, &kind, path.clone(), pick(&ours), pick(&after));
                        c.kept = if theirs_win { "theirs" } else { "ours" }.into();
                        conflicts.push(c);
                    }
                    if m.value != ours {
                        pending.insert((id, kind.clone()), m.value.clone());
                        ops.push(WireOp::Set { id, kind, before: ours, after: m.value });
                    }
                }
            }
        }
        let original = WireChangeSet { merged: true, stamp: Some(stamp.clone()), ..wire.clone() };
        if ops.is_empty() {
            let unchanged = Committed { revision: self.revision, output: Value::Null, op_count: 0 };
            self.committed.insert(wire.command_id, unchanged.clone());
            self.log.push(original);
            let k = self.known.entry(stamp.origin.clone()).or_default();
            *k = (*k).max(stamp.clock);
            return Ok(Merge { committed: Some(unchanged), conflicts });
        }
        let change = self.registry.changeset_from_wire(WireChangeSet { ops, stamp: Some(stamp.clone()), merged: false, ..wire })?;
        self.incoming_seen = Some(change.seen.clone());
        apply_atomic(&mut self.world, &self.registry, &change.ops, true)?;
        let committed = self.record_stamped(change.command_id, change.operation, change.actor, change.ops, Value::Null, Origin::Remote, Some(stamp));
        // The log keeps their change as it arrived, not as it landed here.
        if let Some(last) = self.log.last_mut() {
            *last = original;
        }
        Ok(Merge { committed: Some(committed), conflicts })
    }

    /// An entity's components just before this copy deleted it, and what the
    /// delete's author had seen (from the journal).
    #[allow(clippy::type_complexity)]
    fn last_state_before_delete(&self, id: StableId) -> Option<(Vec<(String, Value)>, BTreeMap<String, u64>)> {
        let change = self.journal.iter().rev().find(|c| c.ops.iter().any(|o| matches!(o, Op::Delete { id: i } if *i == id)))?;
        let state = change
            .ops
            .iter()
            .filter_map(|o| match o {
                Op::Set { id: i, kind, before: Some(b), .. } if *i == id => Some(((*kind).to_string(), b.to_json())),
                _ => None,
            })
            .collect();
        Some((state, change.seen.clone()))
    }

    /// The latest stamp that wrote `path` (or anything inside or above it) of a component.
    fn writer_of(&self, id: StableId, kind: &str, path: &str) -> Stamp {
        let overlaps = |a: &str| a == path || a.starts_with(&format!("{path}/")) || path.starts_with(&format!("{a}/")) || a.is_empty() || path.is_empty();
        self.writers
            .iter()
            .filter(|((i, k), _)| *i == id && *k == kind)
            .flat_map(|(_, list)| list.iter())
            .filter(|(p, _)| overlaps(p))
            .map(|(_, s)| s.clone())
            .max()
            .unwrap_or_default()
    }

    fn note_writer(&mut self, id: StableId, kind: &str, path: String, stamp: &Stamp) -> Result<(), CommandError> {
        let kind = self
            .registry
            .component_info(kind)
            .ok_or_else(|| CommandError::new(ErrorCode::UnsupportedRepresentation, format!("unknown component kind {kind}")))?
            .kind;
        let list = self.writers.entry((id, kind)).or_default();
        match list.iter_mut().find(|(q, _)| *q == path) {
            Some(entry) => entry.1 = entry.1.clone().max(stamp.clone()),
            None => list.push((path, stamp.clone())),
        }
        Ok(())
    }

    /// One component of an entity as JSON (None if the entity lacks it).
    fn component_json(&self, id: StableId, kind: &str) -> Result<Option<Value>, CommandError> {
        let info = self
            .registry
            .component_info(kind)
            .ok_or_else(|| CommandError::new(ErrorCode::UnsupportedRepresentation, format!("unknown component kind {kind}")))?;
        Ok(self.entity(id).and_then(|e| (info.capture)(&self.world, e)).map(|v| v.to_json()))
    }

    pub fn entity_ids(&self) -> Vec<StableId> {
        self.world.resource::<IdIndex>().iter().map(|(id, _)| id).collect()
    }

    /// Persistent components of one entity, as JSON keyed by kind.
    pub fn entity_state(&self, id: StableId) -> Option<BTreeMap<&'static str, Value>> {
        let entity = self.entity(id)?;
        Some(
            self.registry
                .component_infos()
                .iter()
                .filter_map(|info| (info.capture)(&self.world, entity).map(|v| (info.kind, v.to_json())))
                .collect(),
        )
    }

    /// Recomputes the derived results of these entities and everything
    /// downstream, as if they had just changed: for when something outside the
    /// project they read arrives (an imported file fetched from a teammate).
    pub fn refresh(&mut self, ids: &[StableId]) {
        let ops: Vec<Op> = ids.iter().map(|id| Op::Create { id: *id }).collect();
        self.recompute(&ops);
    }

    /// These entities and everything that depends on them.
    pub fn downstream(&self, ids: &[StableId]) -> Vec<StableId> {
        let mut seen: FxHashSet<StableId> = ids.iter().copied().collect();
        let mut queue: Vec<StableId> = ids.to_vec();
        while let Some(id) = queue.pop() {
            for dependent in self.dependents.get(&id).into_iter().flatten() {
                if seen.insert(*dependent) {
                    queue.push(*dependent);
                }
            }
        }
        seen.into_iter().collect()
    }

    /// Entities touched by a change set plus everything downstream of them:
    /// the set whose data or derived results may have changed.
    pub fn affected_by(&self, change: &ChangeSet) -> Vec<StableId> {
        let mut seen: FxHashSet<StableId> = change.ops.iter().map(Op::id).collect();
        let mut queue: Vec<StableId> = seen.iter().copied().collect();
        while let Some(id) = queue.pop() {
            for dependent in self.dependents.get(&id).into_iter().flatten() {
                if seen.insert(*dependent) {
                    queue.push(*dependent);
                }
            }
        }
        seen.into_iter().collect()
    }

    /// Every persistent component of every entity, for saving and comparison.
    pub fn export_state(&self) -> BTreeMap<StableId, BTreeMap<&'static str, Value>> {
        let index = self.world.resource::<IdIndex>();
        index
            .iter()
            .map(|(id, entity)| {
                let components = self
                    .registry
                    .component_infos()
                    .iter()
                    .filter_map(|info| (info.capture)(&self.world, entity).map(|v| (info.kind, v.to_json())))
                    .collect();
                (id, components)
            })
            .collect()
    }

    /// The changes this copy held right after `revision` (a version vector).
    pub fn vector_at(&self, revision: Revision) -> Option<crate::versions::Vector> {
        self.world.resource::<crate::versions::Clocks>().at(revision).cloned()
    }

    /// This copy's revision holding exactly the changes `seen` names, if any.
    pub fn revision_of(&self, seen: &crate::versions::Vector) -> Option<Revision> {
        self.world.resource::<crate::versions::Clocks>().revision_of(seen)
    }

    /// Log entries inside the state `seen` names, in this copy's order.
    pub fn log_within(&self, seen: &crate::versions::Vector) -> Vec<WireChangeSet> {
        self.log.iter().filter(|w| Self::inside(w, seen)).cloned().collect()
    }

    /// Log entries beyond the state `seen` names (made or taken after it).
    pub fn log_beyond(&self, seen: &crate::versions::Vector) -> Vec<WireChangeSet> {
        self.log.iter().filter(|w| !Self::inside(w, seen)).cloned().collect()
    }

    fn inside(w: &WireChangeSet, seen: &crate::versions::Vector) -> bool {
        match &w.stamp {
            Some(s) => crate::versions::covers(seen, &s.origin, s.clock),
            None => true,
        }
    }

    /// The project as it was when it held exactly the changes `seen` names:
    /// a separate copy, rebuilt by replaying those changes from this copy's
    /// log (this one is untouched). Works for any version, including one
    /// marked on a teammate's copy, as long as its changes have arrived here.
    pub fn as_of(&self, seen: &crate::versions::Vector) -> Result<Project, CommandError> {
        Self::rebuild(self.id.clone(), Arc::clone(&self.registry), &self.origin, self.log_within(seen))
    }

    /// A project rebuilt from log entries (all of a log, or the part of one
    /// that a version covers, as [`Project::log_within`] gives it).
    pub fn rebuild(id: String, registry: Arc<Registry>, origin: &str, wires: Vec<WireChangeSet>) -> Result<Project, CommandError> {
        let mut then = Project::new(id, registry);
        then.set_origin(origin);
        for wire in wires {
            if wire.merged {
                then.merge_remote(wire)?;
            } else if wire.base_revision == then.revision {
                then.apply_remote(wire)?;
            } else {
                // Built on changes this state leaves out (cannot happen for a vector
                // taken from a log, but a hand-made one might): merge it instead.
                then.merge_remote(WireChangeSet { merged: true, ..wire })?;
            }
        }
        Ok(then)
    }

    /// Sets the design back to the state of `then` (from [`Project::as_of`])
    /// as one new change: history is kept, and the restore can be undone like
    /// any edit. Entities holding any of `keep` kinds (versions, comments) are
    /// left as they are now.
    pub fn restore(&mut self, then: &Project, actor: &Actor, keep: &[&str], output: Value) -> Result<Committed, CommandError> {
        let infos = self.registry.component_infos();
        let held = |p: &Project, id: StableId| -> Vec<(&'static str, crate::value::CompValue)> {
            match p.entity(id) {
                Some(e) => infos.iter().filter_map(|i| (i.capture)(&p.world, e).map(|v| (i.kind, v))).collect(),
                None => Vec::new(),
            }
        };
        let kept = |state: &[(&'static str, crate::value::CompValue)]| state.iter().any(|(k, _)| keep.contains(k));
        let mut ids: Vec<StableId> = self.entity_ids();
        for id in then.entity_ids() {
            if self.entity(id).is_none() {
                ids.push(id);
            }
        }
        ids.sort();
        let mut ops: Vec<Op> = Vec::new();
        for id in ids {
            let (now, was) = (held(self, id), held(then, id));
            if kept(&now) || kept(&was) {
                continue;
            }
            let exists_now = self.entity(id).is_some();
            let existed = then.entity(id).is_some();
            if exists_now && !existed {
                for (kind, v) in now {
                    ops.push(Op::Set { id, kind, before: Some(v), after: None });
                }
                ops.push(Op::Delete { id });
            } else if !exists_now && existed {
                ops.push(Op::Create { id });
                for (kind, v) in was {
                    ops.push(Op::Set { id, kind, before: None, after: Some(v) });
                }
            } else {
                for info in infos {
                    let a = now.iter().find(|(k, _)| *k == info.kind).map(|(_, v)| v.clone());
                    let b = was.iter().find(|(k, _)| *k == info.kind).map(|(_, v)| v.clone());
                    if !crate::value::same(a.as_ref(), b.as_ref()) {
                        ops.push(Op::Set { id, kind: info.kind, before: a, after: b });
                    }
                }
            }
        }
        // Creations first, so anything restored can be referred to by what follows.
        ops.sort_by_key(|o| match o {
            Op::Create { .. } => 0,
            Op::Set { after: Some(_), .. } => 1,
            Op::Set { .. } => 2,
            Op::Delete { .. } => 3,
        });
        apply_atomic(&mut self.world, &self.registry, &ops, true)?;
        let command_id = format!("version.restore-{}", StableId::new());
        Ok(self.record(command_id, "version.restore".into(), actor.clone(), ops, output, Origin::Command))
    }

    fn check(&self, envelope: &Envelope) -> Result<(), CommandError> {
        if envelope.schema_version != SCHEMA_VERSION {
            return Err(CommandError::new(
                ErrorCode::UnsupportedSchemaVersion,
                format!("schema version {} is not supported", envelope.schema_version),
            ));
        }
        if envelope.project_id != self.id {
            return Err(CommandError::new(ErrorCode::WrongProject, format!("this is project {}", self.id)));
        }
        if envelope.expected_revision != self.revision {
            return Err(CommandError::new(
                ErrorCode::StaleRevision,
                format!("expected {:?}; project is at {:?}", envelope.expected_revision, self.revision),
            ));
        }
        Ok(())
    }

    fn run(&mut self, envelope: &Envelope) -> Result<(Value, Vec<Op>), CommandError> {
        let registry = Arc::clone(&self.registry);
        let info = registry.command_info(&envelope.operation).ok_or_else(|| {
            CommandError::new(ErrorCode::UnknownOperation, format!("unknown operation {}", envelope.operation))
        })?;
        let mut tx = Tx::new(&mut self.world, &registry, &envelope.actor);
        let result = (info.exec)(envelope.parameters.clone(), &mut tx);
        let ops = tx.into_ops();
        // Rules no single command owns may still refuse what it did.
        let result = result.and_then(|out| registry.guards.iter().try_for_each(|g| g(&self.world, &ops)).map(|_| out));
        match result {
            Ok(output) => Ok((output, ops)),
            Err(err) => {
                rollback(&mut self.world, &registry, &ops);
                Err(err)
            }
        }
    }

    fn revert(&mut self, index: usize, actor: &Actor, operation: &str, origin: Origin, output: Value) -> Result<(), CommandError> {
        let inverse: Vec<Op> = self.journal[index].ops.iter().rev().map(Op::inverse).collect();
        apply_atomic(&mut self.world, &self.registry, &inverse, true)
            .map_err(|e| CommandError::new(ErrorCode::UndoConflict, e.message))?;
        let command_id = format!("{operation}-{}", StableId::new());
        self.record(command_id, operation.to_owned(), actor.clone(), inverse, output, origin);
        Ok(())
    }

    fn committed_for(&self, index: usize) -> Committed {
        self.committed[&self.journal[index].command_id].clone()
    }

    fn record(&mut self, command_id: String, operation: String, actor: Actor, ops: Vec<Op>, output: Value, origin: Origin) -> Committed {
        self.record_stamped(command_id, operation, actor, ops, output, origin, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_stamped(&mut self, command_id: String, operation: String, actor: Actor, ops: Vec<Op>, output: Value, origin: Origin, stamp: Option<Stamp>) -> Committed {
        if ops.is_empty() {
            let unchanged = Committed { revision: self.revision, output, op_count: 0 };
            self.committed.insert(command_id, unchanged.clone());
            return unchanged;
        }
        let base_revision = self.revision;
        self.revision = base_revision.next();
        self.update_dependents(&ops);
        if !self.deferring {
            self.recompute(&ops);
        }

        let index = self.journal.len();
        match origin {
            Origin::Command => {
                self.undo.entry(actor.user_id.clone()).or_default().push(index);
                self.redo.remove(&actor.user_id);
            }
            Origin::Undo => self.redo.entry(actor.user_id.clone()).or_default().push(index),
            Origin::Redo => self.undo.entry(actor.user_id.clone()).or_default().push(index),
            Origin::Replay | Origin::Remote => {}
        }
        let stamp = match stamp {
            Some(s) => s,
            None => Stamp { clock: self.clock + 1, origin: self.origin.clone() },
        };
        self.clock = self.clock.max(stamp.clock);
        // Changes made on this copy write their fields now. (A peer's change
        // notes its writes while merging; creating or removing a component is
        // not a field write: a change beats a concurrent delete instead.)
        if !matches!(origin, Origin::Remote) {
            for op in &ops {
                if let Op::Set { id, after: Some(_), .. } = op {
                    self.touched.entry(*id).or_default().push(stamp.clone());
                }
            }
            for op in &ops {
                let Op::Set { id, kind, before: Some(b), after: Some(a) } = op else { continue };
                let mut paths = Vec::new();
                crate::merge::changed_paths(&b.to_json(), &a.to_json(), "", &mut paths);
                let list = self.writers.entry((*id, kind)).or_default();
                for p in paths {
                    match list.iter_mut().find(|(q, _)| *q == p) {
                        Some(entry) => entry.1 = entry.1.clone().max(stamp.clone()),
                        None => list.push((p, stamp.clone())),
                    }
                }
            }
        }
        let seen = self.incoming_seen.take().unwrap_or_else(|| self.known.clone());
        let k = self.known.entry(stamp.origin.clone()).or_default();
        *k = (*k).max(stamp.clock);
        let after = self.known.clone();
        self.world.resource_mut::<crate::versions::Clocks>().after.push(after);
        let committed = Committed { revision: self.revision, output, op_count: ops.len() };
        self.committed.insert(command_id.clone(), committed.clone());
        self.journal.push(ChangeSet { revision: self.revision, base_revision, command_id, operation, actor, ops, stamp, seen });
        let wire = self.journal.last().expect("just pushed").to_wire();
        self.log.push(wire);
        committed
    }

    fn update_dependents(&mut self, ops: &[Op]) {
        for op in ops {
            let Op::Set { id, kind, before, after } = op else { continue };
            if *kind != DependsOn::KIND {
                continue;
            }
            let deps = |v: &Option<crate::value::CompValue>| {
                v.as_ref()
                    .and_then(|v| v.as_any().downcast_ref::<DependsOn>())
                    .map(|d| d.0.clone())
                    .unwrap_or_default()
            };
            let (before, after) = (deps(before), deps(after));
            for target in before.iter().filter(|t| !after.contains(t)) {
                if let Some(set) = self.dependents.get_mut(target) {
                    set.remove(id);
                    if set.is_empty() {
                        self.dependents.remove(target);
                    }
                }
            }
            for target in after.iter().filter(|t| !before.contains(t)) {
                self.dependents.entry(*target).or_default().insert(*id);
            }
        }
    }

    /// Recomputes derived results for exactly the entities affected by `ops`,
    /// level by level in dependency order, in parallel within each level.
    fn recompute(&mut self, ops: &[Op]) {
        if self.registry.derived.is_empty() {
            return;
        }
        let mut dirty: FxHashMap<StableId, Dirty> = FxHashMap::default();
        for op in ops {
            let entry = dirty.entry(op.id()).or_default();
            match op {
                Op::Create { .. } | Op::Delete { .. } => entry.structural = true,
                Op::Set { kind, .. } if *kind == DependsOn::KIND => entry.structural = true,
                Op::Set { kind, .. } => {
                    if !entry.kinds.contains(kind) {
                        entry.kinds.push(kind);
                    }
                }
            }
        }

        let mut queue: Vec<StableId> = dirty.keys().copied().collect();
        while let Some(id) = queue.pop() {
            let Some(dependents) = self.dependents.get(&id) else { continue };
            for &dependent in dependents {
                let entry = dirty.entry(dependent).or_default();
                if !entry.upstream {
                    entry.upstream = true;
                    queue.push(dependent);
                }
            }
        }

        let index = self.world.resource::<IdIndex>();
        dirty.retain(|id, _| index.contains(*id));
        let deps_of = |world: &World, id: StableId| -> SmallVec<[StableId; 4]> {
            index
                .get(id)
                .and_then(|e| world.get::<DependsOn>(e))
                .map(|d| d.0.iter().copied().collect())
                .unwrap_or_default()
        };

        let mut indegree: FxHashMap<StableId, u32> = FxHashMap::default();
        let mut level = Vec::new();
        for &id in dirty.keys() {
            let n = deps_of(&self.world, id).iter().filter(|d| dirty.contains_key(d)).count() as u32;
            if n == 0 {
                level.push(id);
            } else {
                indegree.insert(id, n);
            }
        }

        while !level.is_empty() {
            self.compute_level(&level, &dirty);
            let mut next = Vec::new();
            for id in &level {
                let Some(dependents) = self.dependents.get(id) else { continue };
                for dependent in dependents {
                    if let Some(n) = indegree.get_mut(dependent) {
                        *n -= 1;
                        if *n == 0 {
                            indegree.remove(dependent);
                            next.push(*dependent);
                        }
                    }
                }
            }
            level = next;
        }

        let cyclic: Vec<(Entity, Status)> = indegree
            .keys()
            .filter_map(|id| self.entity(*id))
            .map(|e| (e, Status::Failed("dependency cycle".into())))
            .collect();
        self.world.insert_batch(cyclic);
    }

    fn compute_level(&mut self, level: &[StableId], dirty: &FxHashMap<StableId, Dirty>) {
        let registry = Arc::clone(&self.registry);
        let index = self.world.resource::<IdIndex>();
        let mut ready: Vec<(StableId, Entity)> = Vec::with_capacity(level.len());
        let mut statuses: Vec<(Entity, Status)> = Vec::new();

        for &id in level {
            let Some(entity) = index.get(id) else { continue };
            let blocked = self.world.get::<DependsOn>(entity).and_then(|deps| {
                deps.0.iter().find_map(|dep| match index.get(*dep) {
                    None => Some(format!("dependency {dep} does not exist")),
                    Some(de) => match self.world.get::<Status>(de) {
                        Some(Status::Failed(_) | Status::Blocked(_)) => Some(format!("dependency {dep} has no valid result")),
                        _ => None,
                    },
                })
            });
            match blocked {
                Some(reason) => statuses.push((entity, Status::Blocked(reason))),
                None => ready.push((id, entity)),
            }
        }

        let mut failures: FxHashMap<Entity, Vec<String>> = FxHashMap::default();
        for derived in &registry.derived {
            let selected: Vec<Entity> = ready
                .iter()
                .filter(|(id, _)| {
                    let d = &dirty[id];
                    d.structural
                        || (d.upstream && derived.reads_dependencies)
                        || d.kinds.iter().any(|k| derived.inputs.contains(k))
                })
                .map(|(_, e)| *e)
                .collect();
            if selected.is_empty() {
                continue;
            }
            let batch = (derived.run)(&self.world, &selected);
            (batch.write)(&mut self.world);
            for (entity, message) in batch.failures {
                failures.entry(entity).or_default().push(format!("{}: {message}", derived.name));
            }
        }

        let mut seen = FxHashSet::default();
        for (_, entity) in ready {
            if !seen.insert(entity) {
                continue;
            }
            let status = match failures.remove(&entity) {
                Some(messages) => Status::Failed(messages.join("; ")),
                None => Status::Current,
            };
            if self.world.get::<Status>(entity) != Some(&status) {
                statuses.push((entity, status));
            }
        }
        self.world.insert_batch(statuses);
    }
}
