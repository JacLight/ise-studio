//! Comments pinned to the model, the way a team marks up a design review.
//!
//! A thread is pinned to a point on a part, held in the part's own
//! coordinates, so the pin travels with the part when it is moved or turned.
//! Threads and replies live in the project like everything else: they are
//! saved in its journal, work offline, and reach teammates peer to peer.
//!
//! Each reply is its own entity pointing at its thread, rather than an entry
//! in a list on the thread: two people replying at the same moment on
//! different computers then never clash (copies merge field by field), and
//! every copy shows both replies.

use crate::Instance;
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use de_shape::ShapeProgram;
use de_solid::{BoxFeature, Placement};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Longest comment or reply, in characters.
pub const MAX_TEXT: usize = 4000;
/// Longest author name, in characters.
pub const MAX_AUTHOR: usize = 80;
/// Most markup strokes on a thread, and points in one stroke.
pub const MAX_STROKES: usize = 24;
pub const MAX_STROKE_POINTS: usize = 400;

/// Where a thread is pinned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Anchor {
    /// The part pinned to: a box, a placed shape or a top-level instance.
    /// `None` pins to a fixed point in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<StableId>,
    /// For shapes made of several parts: the part the pin is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// The point (mm) in the target's own coordinates (project coordinates
    /// when there is no target).
    pub point: [f64; 3],
    /// The surface's outward direction there, in the same coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
}

/// A freehand stroke drawn on the model with a comment: points (mm) in the
/// anchor target's own coordinates, so the drawing moves with the part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stroke {
    pub points: Vec<[f64; 3]>,
    /// Ends in an arrowhead (a pointer at something), or a plain line.
    #[serde(default)]
    pub arrow: bool,
}

/// The first comment of a thread, pinned to the model.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommentThread {
    pub anchor: Anchor,
    pub author: String,
    /// Milliseconds since 1970 (UTC).
    pub created: u64,
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
    /// Who resolved it, and when (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<u64>,
    /// When the text was last changed (ms), if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<u64>,
    /// Markup drawn on the model with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markup: Vec<Stroke>,
}

impl Persist for CommentThread {
    const KIND: &'static str = "comment.thread";
}

/// A reply in a thread.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommentReply {
    pub thread: StableId,
    pub author: String,
    pub created: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<u64>,
    /// Markup drawn on the model with the reply, in the thread's part
    /// coordinates (like the thread's own).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markup: Vec<Stroke>,
}

impl Persist for CommentReply {
    const KIND: &'static str = "comment.reply";
}

// ---- Coordinates -----------------------------------------------------------

/// How a part's own coordinates sit in the project: rotation about Z, then
/// translation (mm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: [f64; 3],
    pub rotation: f64,
}

impl Frame {
    /// A point in the part's coordinates, in project coordinates.
    pub fn to_world(&self, p: [f64; 3]) -> [f64; 3] {
        let [x, y] = rotate([p[0], p[1]], self.rotation);
        [x + self.origin[0], y + self.origin[1], p[2] + self.origin[2]]
    }

    /// A project point in the part's coordinates.
    pub fn to_local(&self, p: [f64; 3]) -> [f64; 3] {
        let [x, y] = rotate([p[0] - self.origin[0], p[1] - self.origin[1]], -self.rotation);
        [x, y, p[2] - self.origin[2]]
    }

    /// A direction in the part's coordinates, in project coordinates.
    pub fn dir_to_world(&self, d: [f64; 3]) -> [f64; 3] {
        let [x, y] = rotate([d[0], d[1]], self.rotation);
        [x, y, d[2]]
    }

    pub fn dir_to_local(&self, d: [f64; 3]) -> [f64; 3] {
        let [x, y] = rotate([d[0], d[1]], -self.rotation);
        [x, y, d[2]]
    }
}

fn rotate([x, y]: [f64; 2], degrees: f64) -> [f64; 2] {
    let (s, c) = degrees.to_radians().sin_cos();
    [c * x - s * y, s * x + c * y]
}

/// Where a part's own coordinates are now, as the view draws it:
/// - a box: its placement is its minimum corner, the origin of its coordinates;
/// - a placed shape: its placement is the origin of its program's coordinates
///   (a definition, placed only through instances, has none);
/// - a top-level instance: its position and rotation.
///
/// `None` for anything that is not placed in the project.
pub fn frame(world: &World, id: StableId) -> Option<Frame> {
    let e = world.resource::<IdIndex>().get(id)?;
    let mm = |p: &Placement| [p.x.canonical() * 1000.0, p.y.canonical() * 1000.0, p.z.canonical() * 1000.0];
    if let Some(inst) = world.get::<Instance>(e) {
        return inst.parent.is_none().then_some(Frame { origin: inst.at, rotation: inst.rotation });
    }
    let placement = world.get::<Placement>(e);
    if world.get::<ShapeProgram>(e).is_some() {
        return Some(Frame { origin: mm(placement?), rotation: 0.0 });
    }
    if world.get::<BoxFeature>(e).is_some() {
        return Some(Frame { origin: placement.map(mm).unwrap_or([0.0; 3]), rotation: 0.0 });
    }
    None
}

/// Where a pin is in the project now (mm): it follows its part. `None` when
/// the part it was pinned to is gone.
pub fn anchor_world(world: &World, anchor: &Anchor) -> Option<[f64; 3]> {
    match anchor.target {
        None => Some(anchor.point),
        Some(t) => frame(world, t).map(|f| f.to_world(anchor.point)),
    }
}

// ---- Validation ------------------------------------------------------------

fn bad(code: ErrorCode, m: impl Into<String>) -> CommandError {
    CommandError::new(code, m)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn text_of(text: String) -> Result<String, CommandError> {
    let t = text.trim();
    if t.is_empty() {
        return Err(bad(ErrorCode::InvalidParameters, "a comment needs some text"));
    }
    if t.chars().count() > MAX_TEXT {
        return Err(bad(ErrorCode::InvalidParameters, format!("a comment is at most {MAX_TEXT} characters")));
    }
    Ok(t.to_string())
}

fn author_of(author: &str) -> Result<String, CommandError> {
    let a = author.trim();
    if a.is_empty() {
        return Err(bad(ErrorCode::InvalidParameters, "say who is commenting (author)"));
    }
    if a.chars().count() > MAX_AUTHOR {
        return Err(bad(ErrorCode::InvalidParameters, format!("an author name is at most {MAX_AUTHOR} characters")));
    }
    Ok(a.to_string())
}

fn finite(p: &[f64; 3]) -> bool {
    p.iter().all(|v| v.is_finite())
}

fn unit(n: [f64; 3]) -> Result<[f64; 3], CommandError> {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !finite(&n) || len < 1e-9 {
        return Err(bad(ErrorCode::InvalidParameters, "a normal must be a non-zero direction"));
    }
    Ok([n[0] / len, n[1] / len, n[2] / len])
}

fn thread(tx: &Tx, id: StableId) -> Result<CommentThread, CommandError> {
    tx.get::<CommentThread>(id).cloned().ok_or_else(|| bad(ErrorCode::UnresolvedReference, format!("{id} is not a comment thread")))
}

fn markup_of(strokes: Vec<Stroke>, frame: Option<Frame>, world_coords: bool) -> Result<Vec<Stroke>, CommandError> {
    if strokes.len() > MAX_STROKES {
        return Err(bad(ErrorCode::InvalidParameters, format!("at most {MAX_STROKES} markup strokes")));
    }
    strokes
        .into_iter()
        .map(|s| {
            if s.points.len() < 2 || s.points.len() > MAX_STROKE_POINTS {
                return Err(bad(ErrorCode::InvalidParameters, format!("a markup stroke has 2 to {MAX_STROKE_POINTS} points")));
            }
            if !s.points.iter().all(finite) {
                return Err(bad(ErrorCode::InvalidParameters, "markup points must be finite numbers"));
            }
            let points = match (frame, world_coords) {
                (Some(f), true) => s.points.iter().map(|p| f.to_local(*p)).collect(),
                _ => s.points,
            };
            Ok(Stroke { points, arrow: s.arrow })
        })
        .collect()
}

// ---- Commands --------------------------------------------------------------

/// `comment.create`: start a thread pinned to the model.
///
/// Give the point either in the target's own coordinates (`point`) or in
/// project coordinates (`at`, converted to the target's). Without a target
/// the pin stays at a fixed project point.
#[derive(Deserialize, JsonSchema)]
pub struct CreateComment {
    pub text: String,
    pub author: String,
    #[serde(default)]
    pub target: Option<StableId>,
    #[serde(default)]
    pub part: Option<String>,
    /// Point (mm) in the target's own coordinates.
    #[serde(default)]
    pub point: Option<[f64; 3]>,
    /// Point (mm) in project coordinates.
    #[serde(default)]
    pub at: Option<[f64; 3]>,
    /// Surface direction there, in the same coordinates as the point.
    #[serde(default)]
    pub normal: Option<[f64; 3]>,
    /// Freehand strokes or arrows drawn on the model, in the same coordinates as the point.
    #[serde(default)]
    pub markup: Vec<Stroke>,
}

impl Command for CreateComment {
    const NAME: &'static str = "comment.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let text = text_of(self.text)?;
        let author = author_of(&self.author)?;
        let frame = match self.target {
            Some(t) => {
                if !tx.exists(t) {
                    return Err(bad(ErrorCode::UnresolvedReference, format!("{t} does not exist")));
                }
                Some(frame(tx.world(), t).ok_or_else(|| bad(ErrorCode::InvalidParameters, format!("{t} is not placed in the project: pin a box, a placed shape or a top-level instance")))?)
            }
            None => None,
        };
        let (point, world_coords) = match (self.point, self.at) {
            (Some(p), None) => (p, frame.is_none()),
            (None, Some(p)) => (p, true),
            (Some(_), Some(_)) => return Err(bad(ErrorCode::InvalidParameters, "give the point either in the part's coordinates (point) or the project's (at), not both")),
            (None, None) => return Err(bad(ErrorCode::InvalidParameters, "say where the comment is pinned (point or at)")),
        };
        if !finite(&point) {
            return Err(bad(ErrorCode::InvalidParameters, "the point must be finite numbers"));
        }
        let point = match frame {
            Some(f) if world_coords => f.to_local(point),
            _ => point,
        };
        let normal = match self.normal {
            Some(n) => {
                let n = unit(n)?;
                Some(match frame {
                    Some(f) if world_coords => f.dir_to_local(n),
                    _ => n,
                })
            }
            None => None,
        };
        let markup = markup_of(self.markup, frame, world_coords)?;
        let part = self.part.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        let id = tx.create();
        tx.set(
            id,
            CommentThread {
                anchor: Anchor { target: self.target, part, point, normal },
                author,
                created: now_ms(),
                text,
                resolved: false,
                resolved_by: None,
                resolved_at: None,
                edited: None,
                markup,
            },
        )?;
        Ok(json!({ "id": id }))
    }
}

/// `comment.reply`: answer in a thread (a reply reopens nothing; resolve or reopen separately).
///
/// A reply can carry markup too, drawn on the thread's part: give strokes in
/// the part's own coordinates (`markup`) or in project coordinates
/// (`markup_at`, converted). With markup, the text may be left empty.
#[derive(Deserialize, JsonSchema)]
pub struct ReplyComment {
    pub thread: StableId,
    #[serde(default)]
    pub text: String,
    pub author: String,
    /// Strokes in the thread's part coordinates.
    #[serde(default)]
    pub markup: Vec<Stroke>,
    /// Strokes in project coordinates.
    #[serde(default)]
    pub markup_at: Vec<Stroke>,
}

/// Strokes given for a thread, in its part's coordinates.
fn thread_markup(tx: &Tx, t: &CommentThread, markup: Vec<Stroke>, markup_at: Vec<Stroke>) -> Result<Vec<Stroke>, CommandError> {
    let frame = t.anchor.target.and_then(|id| frame(tx.world(), id));
    if t.anchor.target.is_some() && frame.is_none() && !markup_at.is_empty() {
        return Err(bad(ErrorCode::InvalidParameters, "the part this thread is pinned to is gone: give markup in its own coordinates (markup)"));
    }
    let mut out = markup_of(markup, frame, false)?;
    out.extend(markup_of(markup_at, frame, true)?);
    if out.len() > MAX_STROKES {
        return Err(bad(ErrorCode::InvalidParameters, format!("at most {MAX_STROKES} markup strokes")));
    }
    Ok(out)
}

impl Command for ReplyComment {
    const NAME: &'static str = "comment.reply";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let t = thread(tx, self.thread)?;
        let markup = thread_markup(tx, &t, self.markup, self.markup_at)?;
        let text = if markup.is_empty() || !self.text.trim().is_empty() { text_of(self.text)? } else { String::new() };
        let author = author_of(&self.author)?;
        let id = tx.create();
        tx.set(id, CommentReply { thread: self.thread, author, created: now_ms(), text, edited: None, markup })?;
        Ok(json!({ "id": id }))
    }
}

/// `comment.markup`: add pen or arrow strokes to your own comment or reply
/// after posting it (or, with `replace`, swap its markup for these; an empty
/// list with `replace` clears it). Coordinates as for `comment.reply`.
#[derive(Deserialize, JsonSchema)]
pub struct MarkupComment {
    /// A thread or a reply.
    pub target: StableId,
    /// Who is drawing: only the author may.
    pub author: String,
    #[serde(default)]
    pub markup: Vec<Stroke>,
    #[serde(default)]
    pub markup_at: Vec<Stroke>,
    #[serde(default)]
    pub replace: bool,
}

impl Command for MarkupComment {
    const NAME: &'static str = "comment.markup";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (thread_id, reply) = match (tx.get::<CommentThread>(self.target), tx.get::<CommentReply>(self.target)) {
            (Some(_), _) => (self.target, None),
            (None, Some(r)) => (r.thread, Some(r.clone())),
            _ => return Err(bad(ErrorCode::UnresolvedReference, format!("{} is not a comment", self.target))),
        };
        let t = thread(tx, thread_id)?;
        let strokes = thread_markup(tx, &t, self.markup, self.markup_at)?;
        if strokes.is_empty() && !self.replace {
            return Err(bad(ErrorCode::InvalidParameters, "draw at least one stroke (markup or markup_at)"));
        }
        let replace = self.replace;
        let merged = |old: &[Stroke]| -> Result<Vec<Stroke>, CommandError> {
            let all: Vec<Stroke> = if replace { strokes.clone() } else { old.iter().cloned().chain(strokes.iter().cloned()).collect() };
            if all.len() > MAX_STROKES {
                return Err(bad(ErrorCode::InvalidParameters, format!("at most {MAX_STROKES} markup strokes on one comment")));
            }
            Ok(all)
        };
        match reply {
            Some(mut r) => {
                own(&r.author, &self.author, "draw on")?;
                r.markup = merged(&r.markup)?;
                if r.markup.is_empty() && r.text.is_empty() {
                    return Err(bad(ErrorCode::InvalidParameters, "a reply with no text keeps at least one stroke (delete it instead)"));
                }
                let n = r.markup.len();
                tx.set(self.target, r)?;
                Ok(json!({ "id": self.target, "strokes": n }))
            }
            None => {
                let mut t = t;
                own(&t.author, &self.author, "draw on")?;
                t.markup = merged(&t.markup)?;
                let n = t.markup.len();
                tx.set(self.target, t)?;
                Ok(json!({ "id": self.target, "strokes": n }))
            }
        }
    }
}

/// `comment.resolve`: mark a thread done (its pin hides unless resolved threads are shown).
#[derive(Deserialize, JsonSchema)]
pub struct ResolveComment {
    pub target: StableId,
    /// Who resolved it.
    pub author: String,
}

impl Command for ResolveComment {
    const NAME: &'static str = "comment.resolve";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut t = thread(tx, self.target)?;
        let by = author_of(&self.author)?;
        if !t.resolved {
            t.resolved = true;
            t.resolved_by = Some(by);
            t.resolved_at = Some(now_ms());
            tx.set(self.target, t)?;
        }
        Ok(json!({ "id": self.target }))
    }
}

/// `comment.reopen`: open a resolved thread again.
#[derive(Deserialize, JsonSchema)]
pub struct ReopenComment {
    pub target: StableId,
}

impl Command for ReopenComment {
    const NAME: &'static str = "comment.reopen";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut t = thread(tx, self.target)?;
        if t.resolved {
            t.resolved = false;
            t.resolved_by = None;
            t.resolved_at = None;
            tx.set(self.target, t)?;
        }
        Ok(json!({ "id": self.target }))
    }
}

/// `comment.edit`: change the text of your own comment or reply.
#[derive(Deserialize, JsonSchema)]
pub struct EditComment {
    /// A thread or a reply.
    pub target: StableId,
    pub text: String,
    /// Who is editing: only the author may.
    pub author: String,
}

fn own(written_by: &str, author: &str, what: &str) -> Result<(), CommandError> {
    if written_by != author.trim() {
        return Err(bad(ErrorCode::InvalidParameters, format!("only {written_by} can {what} this comment")));
    }
    Ok(())
}

impl Command for EditComment {
    const NAME: &'static str = "comment.edit";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let text = text_of(self.text)?;
        if let Some(mut t) = tx.get::<CommentThread>(self.target).cloned() {
            own(&t.author, &self.author, "edit")?;
            if t.text != text {
                t.text = text;
                t.edited = Some(now_ms());
                tx.set(self.target, t)?;
            }
        } else if let Some(mut r) = tx.get::<CommentReply>(self.target).cloned() {
            own(&r.author, &self.author, "edit")?;
            if r.text != text {
                r.text = text;
                r.edited = Some(now_ms());
                tx.set(self.target, r)?;
            }
        } else {
            return Err(bad(ErrorCode::UnresolvedReference, format!("{} is not a comment", self.target)));
        }
        Ok(json!({ "id": self.target }))
    }
}

/// `comment.delete`: remove your own comment; deleting a thread removes its replies.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteComment {
    pub target: StableId,
    /// Who is deleting: only the author may.
    pub author: String,
}

/// The replies in a thread, oldest first.
pub fn replies(world: &World, thread: StableId) -> Vec<(StableId, CommentReply)> {
    let index = world.resource::<IdIndex>();
    let mut out: Vec<(StableId, CommentReply)> =
        index.iter().filter_map(|(id, e)| world.get::<CommentReply>(e).filter(|r| r.thread == thread).map(|r| (id, r.clone()))).collect();
    out.sort_by_key(|(id, r)| (r.created, *id));
    out
}

impl Command for DeleteComment {
    const NAME: &'static str = "comment.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if let Some(t) = tx.get::<CommentThread>(self.target) {
            own(&t.author.clone(), &self.author, "delete")?;
            let gone: Vec<StableId> = replies(tx.world(), self.target).into_iter().map(|(id, _)| id).collect();
            for r in &gone {
                tx.delete(*r)?;
            }
            tx.delete(self.target)?;
            Ok(json!({ "id": self.target, "replies": gone.len() }))
        } else if let Some(r) = tx.get::<CommentReply>(self.target) {
            own(&r.author.clone(), &self.author, "delete")?;
            tx.delete(self.target)?;
            Ok(json!({ "id": self.target }))
        } else {
            Err(bad(ErrorCode::UnresolvedReference, format!("{} is not a comment", self.target)))
        }
    }
}
