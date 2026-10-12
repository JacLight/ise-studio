//! Routing by hand, as commands: lay a route between points or pads in
//! walkaround, shove or highlight mode (`pcb.route.interactive`), drag a
//! track's segment, corner or arc, or a via, pushing what is in the way
//! (`pcb.route.drag`), smooth routed copper (`pcb.route.optimize`), clean it up
//! (`pcb.route.cleanup`), unroute nets or parts (`pcb.route.unroute`), break a
//! track (`pcb.track.break`), round its corners (`pcb.track.fillet`) and fan
//! out a part's pads to vias (`pcb.fanout`).
//!
//! Each command works out a [`Plan`] with the router (`pns`) from the board as
//! it stands, then makes it. The editor asks for the same plan for every
//! cursor position (`POST /pcb/<id>/router`) to show the line, what it pushes
//! and what it would break before anything is committed.

use crate::board::{self, BoardView, layer_index};
use crate::geom::{self, P, Prim};
use crate::pns::{self, Angle, Collision, Mode, Node, Pusher, Ref, Shape};
use crate::{Board, Track, Via, bad, board_of};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn unresolved(m: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::UnresolvedReference, m)
}

fn resolved(tx: &Tx, id: StableId) -> Result<(Board, BoardView), CommandError> {
    let b = board_of(tx, id)?;
    let v = board::resolve(tx.world(), id, &b).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
    Ok((b, v))
}

/// A point on the board, or a pad ("U1.3").
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Endpoint {
    Point([f64; 2]),
    Pad(String),
}

struct Spot {
    at: P,
    layer: Option<String>,
    net: Option<String>,
}

fn spot(v: &BoardView, e: &Endpoint, layer: Option<&str>) -> Result<Spot, String> {
    match e {
        Endpoint::Pad(name) => {
            let p = v.pads.iter().find(|p| format!("{}.{}", p.reference, p.number) == *name && !p.layers.is_empty()).ok_or_else(|| format!("no pad {name} on this board (reference.pad, e.g. U1.3)"))?;
            let l = if p.layers.len() == 1 { Some(p.layers[0].clone()) } else { layer.map(str::to_string) };
            Ok(Spot { at: p.at, layer: l, net: p.net.clone() })
        }
        Endpoint::Point(at) => {
            if !(at[0].is_finite() && at[1].is_finite()) {
                return Err("a point needs finite coordinates".into());
            }
            let (net, l) = copper_at(v, *at, layer);
            Ok(Spot { at: *at, layer: l, net })
        }
    }
}

/// The net (and layer) of the copper under a point: a pad, a track, a via.
fn copper_at(v: &BoardView, at: P, layer: Option<&str>) -> (Option<String>, Option<String>) {
    let probe = Prim::circle(at, 0.0);
    let on = |l: &str| layer.is_none_or(|x| x == l);
    for p in &v.pads {
        if p.layers.iter().any(|l| on(l)) && geom::gap(&p.prim, &probe) <= 1e-6 {
            let l = if p.layers.len() == 1 { Some(p.layers[0].clone()) } else { layer.map(str::to_string) };
            return (p.net.clone(), l);
        }
    }
    for t in &v.tracks {
        if on(&t.layer) && t.points.windows(2).any(|w| geom::point_seg_dist(at, w[0], w[1]) <= t.width / 2.0 + 1e-6) {
            return (t.net.clone(), Some(t.layer.clone()));
        }
    }
    for x in &v.vias {
        if geom::dist(x.at, at) <= x.diameter / 2.0 + 1e-6 {
            return (x.net.clone(), layer.map(str::to_string));
        }
    }
    (None, None)
}

fn layer_idx(v: &BoardView, l: &str) -> Result<usize, String> {
    layer_index(&v.layers, l).ok_or_else(|| format!("layer “{l}” is not on this board (use {})", v.layers.join(", ")))
}

// ---- Plans -------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct PlanRun {
    pub layer: String,
    pub points: Vec<P>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bulges: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MovedTrack {
    pub id: StableId,
    pub points: Vec<P>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bulges: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MovedVia {
    pub id: StableId,
    pub at: P,
}

/// What a route or a drag would do.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Plan {
    /// Nothing it makes or moves breaks a clearance.
    pub ok: bool,
    /// The line got to its last point (walkaround stops short of what it cannot pass).
    pub reached: bool,
    /// The mode that made it (shove falls back to walkaround).
    pub mode: Mode,
    pub net: Option<String>,
    pub width: f64,
    pub via_drill: f64,
    pub via_diameter: f64,
    /// The net's clearance, for drawing the halo.
    pub clearance: f64,
    /// New copper: one run per layer stretch, a via where each meets the next.
    pub runs: Vec<PlanRun>,
    pub vias: Vec<P>,
    /// Existing tracks and vias moved (pushed or dragged).
    pub tracks: Vec<MovedTrack>,
    pub moved_vias: Vec<MovedVia>,
    pub collisions: Vec<Collision>,
    /// Where the line ends and on which layer.
    pub end: P,
    pub layer: String,
    pub notes: Vec<String>,
}

/// The changes of a node against the board it came from.
fn moved_of(node: &Node, keep: &BTreeSet<StableId>, plan: &mut Plan) {
    for l in &node.lines {
        if let Some(id) = l.id {
            if l.moved && !keep.contains(&id) {
                plan.tracks.push(MovedTrack { id, points: l.points.clone(), bulges: vec![] });
            }
        }
    }
    for v in &node.vias {
        if let Some(id) = v.id {
            if v.moved {
                plan.moved_vias.push(MovedVia { id, at: v.at });
            }
        }
    }
}

/// Clearances broken by everything moved in `node`.
fn moved_collisions(node: &Node) -> Vec<Collision> {
    let mut out = Vec::new();
    for (i, l) in node.lines.iter().enumerate() {
        if l.moved {
            out.extend(node.line_report(&l.points, l.width, l.layer, &l.net, &|r| matches!(r, Ref::Line(j, _) if j == i)));
        }
    }
    for (i, v) in node.vias.iter().enumerate() {
        if v.moved {
            out.extend(node.via_report(v.at, v.diameter, &v.net, &|r| r == Ref::Via(i)));
        }
    }
    out
}

fn apply(tx: &mut Tx, board: StableId, plan: &Plan) -> Result<Value, CommandError> {
    let mut tracks = Vec::new();
    let mut vias = Vec::new();
    for r in &plan.runs {
        let points = r.points.clone();
        if points.len() < 2 {
            continue;
        }
        let id = tx.create();
        tx.set(id, Track { board, net: plan.net.clone(), layer: r.layer.clone(), width: plan.width, points, auto: false, reroute: false, bulges: r.bulges.clone() })?;
        tx.add_dependency(board, id)?;
        tracks.push(id);
    }
    for at in &plan.vias {
        let id = tx.create();
        tx.set(id, Via { board, net: plan.net.clone(), at: *at, drill: plan.via_drill, diameter: plan.via_diameter, auto: false })?;
        tx.add_dependency(board, id)?;
        vias.push(id);
    }
    for m in &plan.tracks {
        let mut t = tx.get::<Track>(m.id).cloned().ok_or_else(|| unresolved(format!("{} is not a track", m.id)))?;
        t.points = m.points.clone();
        t.bulges = m.bulges.clone();
        t.reroute = false;
        tx.set(m.id, t)?;
    }
    for m in &plan.moved_vias {
        let mut v = tx.get::<Via>(m.id).cloned().ok_or_else(|| unresolved(format!("{} is not a via", m.id)))?;
        v.at = m.at;
        tx.set(m.id, v)?;
    }
    Ok(json!({
        "tracks": tracks, "vias": vias, "net": plan.net, "width": plan.width, "ok": plan.ok, "reached": plan.reached, "mode": plan.mode,
        "pushed_tracks": plan.tracks.len(), "pushed_vias": plan.moved_vias.len(), "collisions": plan.collisions, "end": plan.end, "notes": plan.notes,
    }))
}

// ---- Laying a route ------------------------------------------------------------

/// One click of a route: the point (or pad) the line goes to next.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Step {
    pub at: Endpoint,
    /// The layer of the leg into this point; a change of layer puts a via where the leg starts.
    pub layer: Option<String>,
    /// 45°: diagonal first instead of straight first (90°: horizontal first). The `/` key.
    pub diagonal_first: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Corners {
    /// Sharp 45° (or 90°) corners.
    #[default]
    Sharp,
    /// Corners rounded into arcs (radius `arc_radius`, default 3 × the width).
    Arc,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Steps {
    One(Endpoint),
    Many(Vec<Step>),
}

/// `pcb.route.interactive`: lay a route from a point, pad or track end through
/// each step, as the router does under the cursor.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RouteInteractive {
    pub board: StableId,
    /// The net; the copper's under `from` (or the last step) when left out.
    pub net: Option<String>,
    pub from: Endpoint,
    /// Starting layer: the pad's for an SMD pad, else top.
    pub layer: Option<String>,
    /// Where to: a pad or point, or the clicks in order.
    pub to: Steps,
    /// walkaround (default), shove or highlight.
    pub mode: Option<Mode>,
    /// "45" (default), "90" or "free".
    pub angle: Option<Angle>,
    pub diagonal_first: Option<bool>,
    /// Track width; the net class's when left out (as are the via sizes).
    pub width: Option<f64>,
    pub via_drill: Option<f64>,
    pub via_diameter: Option<f64>,
    /// Smooth each leg (default true).
    pub optimize: Option<bool>,
    pub corners: Option<Corners>,
    pub arc_radius: Option<f64>,
}

pub fn plan_route(b: &Board, v: &BoardView, r: &RouteInteractive) -> Result<Plan, String> {
    let start = spot(v, &r.from, r.layer.as_deref())?;
    let steps: Vec<Step> = match &r.to {
        Steps::One(e) => vec![Step { at: e.clone(), layer: None, diagonal_first: None }],
        Steps::Many(s) => s.clone(),
    };
    let mut layer = layer_idx(v, r.layer.as_deref().or(start.layer.as_deref()).unwrap_or(&v.layers[0]))?;
    let mut targets = Vec::new();
    {
        let mut l = layer;
        for s in &steps {
            if let Some(x) = &s.layer {
                l = layer_idx(v, x)?;
            }
            targets.push((spot(v, &s.at, Some(&v.layers[l]))?, l));
        }
    }
    let net = r.net.clone().filter(|n| !n.trim().is_empty()).or(start.net.clone()).or_else(|| targets.last().and_then(|(s, _)| s.net.clone()));
    let class = b.class_of(net.as_deref());
    let width = r.width.unwrap_or(class.track_width);
    if !(width > 0.0 && width <= 20.0) {
        return Err("a track width is above 0 and up to 20 mm".into());
    }
    let via_drill = r.via_drill.unwrap_or(class.via_drill);
    let via_diameter = r.via_diameter.unwrap_or(class.via_diameter);
    if !(via_drill > 0.0 && via_diameter > via_drill && via_diameter < 10.0) {
        return Err("a via's diameter is larger than its drill".into());
    }
    let mode = r.mode.unwrap_or_default();
    let angle = r.angle.unwrap_or_default();
    let mut node = Node::new(b, v);
    let (lines0, vias0) = (node.lines.len(), node.vias.len());
    let mut plan = Plan { ok: true, reached: true, mode, net: net.clone(), width, via_drill, via_diameter, clearance: node.clearance(&net, &None), ..Default::default() };
    let mut legs: Vec<(usize, Vec<P>)> = Vec::new();
    let mut cur = start.at;
    for (i, (target, l)) in targets.iter().enumerate() {
        if *l != layer {
            node.vias.push(pns::Via { id: None, net: net.clone(), at: cur, diameter: via_diameter, drill: via_drill, locked: true, moved: false });
            plan.vias.push(cur);
            if mode == Mode::Shove {
                let mut trial = node.clone();
                match trial.shove(&[Pusher::Via(node.vias.len() - 1)]) {
                    Ok(()) => node = trial,
                    Err(e) => plan.notes.push(format!("via: cannot shove: {e}")),
                }
            }
            layer = *l;
        }
        let first = steps[i].diagonal_first.or(r.diagonal_first).unwrap_or(false);
        let laid = node.lay(&pns::LegSpec { from: cur, to: target.at, layer, width, net: &net, mode, angle, first, optimize: r.optimize.unwrap_or(true) });
        if laid.mode != mode {
            plan.mode = laid.mode;
        }
        if let Some(n) = laid.note {
            plan.notes.push(n);
        }
        cur = *laid.points.last().unwrap_or(&cur);
        node.lines.push(pns::Line { id: None, net: net.clone(), layer, width, points: laid.points.clone(), locked: true, arcs: false, moved: false });
        legs.push((layer, laid.points));
        if !laid.reached {
            plan.reached = false;
            break;
        }
    }
    // Runs: legs on one layer joined.
    for (l, pts) in legs {
        match plan.runs.last_mut() {
            Some(run) if run.layer == v.layers[l] => run.points.extend(pts.into_iter().skip(1)),
            _ => plan.runs.push(PlanRun { layer: v.layers[l].clone(), points: pts, bulges: vec![] }),
        }
    }
    for run in &mut plan.runs {
        run.points = pns::clean(&run.points);
    }
    plan.end = cur;
    plan.layer = v.layers[layer].clone();
    // Collisions of the new copper with the board as it stands (with what was pushed).
    let ours = |r: Ref| matches!(r, Ref::Line(i, _) if i >= lines0) || matches!(r, Ref::Via(i) if i >= vias0);
    if r.corners == Some(Corners::Arc) && angle != Angle::Free {
        let radius = r.arc_radius.unwrap_or(3.0 * width);
        for run in &mut plan.runs {
            let l = layer_index(&v.layers, &run.layer).unwrap();
            let (pts, bulges) = pns::fillet(&run.points, radius);
            if node.line_report(&pns::flatten(&pts, &bulges), width, l, &net, &ours).len() <= node.line_report(&run.points, width, l, &net, &ours).len() {
                run.points = pts;
                run.bulges = bulges;
            }
        }
    }
    for run in &plan.runs {
        let l = layer_index(&v.layers, &run.layer).unwrap();
        plan.collisions.extend(node.line_report(&pns::flatten(&run.points, &run.bulges), width, l, &net, &ours));
    }
    for at in &plan.vias {
        plan.collisions.extend(node.via_report(*at, via_diameter, &net, &ours));
    }
    plan.collisions.extend(moved_collisions(&node));
    moved_of(&node, &BTreeSet::new(), &mut plan);
    plan.ok = plan.collisions.is_empty();
    Ok(plan)
}

impl Command for RouteInteractive {
    const NAME: &'static str = "pcb.route.interactive";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (b, v) = resolved(tx, self.board)?;
        let plan = plan_route(&b, &v, &self).map_err(bad)?;
        if plan.runs.iter().all(|r| r.points.len() < 2) && plan.vias.is_empty() {
            return Err(bad(format!("nothing to lay: {}", plan.notes.join("; "))));
        }
        apply(tx, self.board, &plan)
    }
}

// ---- Dragging -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DragKind {
    /// Move segment `index` sideways; its neighbours keep their directions.
    Segment,
    /// Move corner `index` (0 or the last: the track's end).
    Corner,
    /// Bend segment `index` into an arc through `to`.
    Arc,
    /// Move the via; its tracks follow.
    Via,
}

/// `pcb.route.drag`: drag a track's segment, corner or arc, or a via, at 45°
/// (or free), pushing what is in the way (shove, default) or just reporting it.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RouteDrag {
    pub board: StableId,
    pub target: StableId,
    pub kind: Option<DragKind>,
    /// Segment or corner index.
    #[serde(default)]
    pub index: usize,
    /// Where the drag started (a segment drag moves by `to` − `from`); the segment's middle when left out.
    pub from: Option<[f64; 2]>,
    pub to: [f64; 2],
    /// shove (default) or highlight (walkaround drags as highlight).
    pub mode: Option<Mode>,
    /// "45" keeps 45° neighbours (default); "free" moves straight.
    pub angle: Option<Angle>,
}

fn line_meet(p: P, d: P, q: P, e: P) -> Option<f64> {
    let det = geom::cross(d, e);
    if det.abs() < 1e-12 { None } else { Some(geom::cross(geom::sub(q, p), e) / det) }
}

/// Segment `i` moved by `delta` (its normal part at 45°), neighbours kept straight.
pub fn drag_segment(pts: &[P], i: usize, delta: P, angle: Angle) -> Vec<P> {
    let n = pts.len();
    let (a, b) = (pts[i], pts[i + 1]);
    let d = geom::sub(b, a);
    let l = geom::len(d).max(1e-12);
    let u = [d[0] / l, d[1] / l];
    let off = if angle == Angle::Free {
        delta
    } else {
        let nrm = [-u[1], u[0]];
        let k = geom::dot(delta, nrm);
        [nrm[0] * k, nrm[1] * k]
    };
    let (a2, b2) = (geom::add(a, off), geom::add(b, off));
    let mut out: Vec<P> = Vec::new();
    if angle == Angle::Free {
        out.extend_from_slice(&pts[..i.max(1)]);
        if i == 0 {
            out.truncate(1);
        }
        out.push(a2);
        out.push(b2);
        if i + 1 == n - 1 {
            out.push(pts[n - 1]);
        } else {
            out.extend_from_slice(&pts[i + 2..]);
        }
        return pns::clean(&out);
    }
    // Before the segment.
    if i == 0 {
        out.extend(crate::drag::dogleg(pts[0], a2, Some(u)));
    } else {
        let p = pts[i - 1];
        let pd = geom::sub(a, p);
        out.extend_from_slice(&pts[..i]);
        match line_meet(p, pd, a2, u) {
            Some(s) if s > 1e-6 => out.push(geom::add(p, [pd[0] * s, pd[1] * s])),
            _ => out.extend(crate::drag::dogleg(p, a2, Some(u)).into_iter().skip(1)),
        }
    }
    // After it.
    if i + 1 == n - 1 {
        let mut back = crate::drag::dogleg(pts[n - 1], b2, Some([-u[0], -u[1]]));
        back.reverse();
        out.extend(back);
    } else {
        let q = pts[i + 2];
        let nd = geom::sub(b, q);
        match line_meet(q, nd, b2, u) {
            Some(s) if s > 1e-6 => out.push(geom::add(q, [nd[0] * s, nd[1] * s])),
            _ => {
                let mut back = crate::drag::dogleg(q, b2, None);
                back.reverse();
                out.extend(back.into_iter().take_while(|p| geom::dist(*p, q) > 1e-9));
            }
        }
        out.extend_from_slice(&pts[i + 2..]);
    }
    pns::clean(&out)
}

/// Corner `k` moved to `to`; at 45° the legs either side become doglegs.
pub fn drag_corner(pts: &[P], k: usize, to: P, angle: Angle) -> Vec<P> {
    let n = pts.len();
    let mut out: Vec<P> = Vec::new();
    if k > 0 {
        out.extend_from_slice(&pts[..k - 1]);
        if angle == Angle::Free { out.extend([pts[k - 1], to]) } else { out.extend(pns::connect(pts[k - 1], to, angle, false)) }
    } else {
        out.push(to);
    }
    if k + 1 < n {
        let right = if angle == Angle::Free { vec![to, pts[k + 1]] } else { pns::connect(to, pts[k + 1], angle, true) };
        out.extend(right.into_iter().skip(1));
        out.extend_from_slice(&pts[k + 2..]);
    }
    pns::clean(&out)
}

pub fn plan_drag(b: &Board, v: &BoardView, d: &RouteDrag) -> Result<Plan, String> {
    let mut node = Node::new(b, v);
    let mode = d.mode.unwrap_or(Mode::Shove);
    let angle = d.angle.unwrap_or_default();
    let mut plan = Plan { ok: true, reached: true, mode, ..Default::default() };
    let mut pushers = Vec::new();
    let mut keep = BTreeSet::new();
    if let Some(vi) = node.via_by_id(d.target) {
        let from = node.vias[vi].at;
        let to = d.to;
        let net = node.vias[vi].net.clone();
        for i in 0..node.lines.len() {
            let l = &node.lines[i];
            if l.net == net && !l.arcs && (geom::dist(l.points[0], from) < 1e-6 || geom::dist(*l.points.last().unwrap(), from) < 1e-6) {
                let pts = if angle == Angle::Free {
                    let mut p = l.points.clone();
                    if geom::dist(p[0], from) < 1e-6 { p[0] = to } else { *p.last_mut().unwrap() = to }
                    p
                } else {
                    pns::follow_end(&l.points, from, to)
                };
                let line = &mut node.lines[i];
                line.points = pts;
                line.moved = true;
                line.locked = true;
                pushers.push(Pusher::Line(i));
            }
        }
        node.vias[vi].at = to;
        node.vias[vi].moved = true;
        node.vias[vi].locked = true;
        pushers.insert(0, Pusher::Via(vi));
        plan.net = net;
        plan.via_diameter = node.vias[vi].diameter;
        plan.via_drill = node.vias[vi].drill;
        plan.end = to;
    } else {
        let li = node.line_by_id(d.target).ok_or_else(|| format!("{} is not a track or via on this board", d.target))?;
        let tv = v.tracks.iter().find(|t| t.id == d.target).unwrap();
        let kind = d.kind.unwrap_or(if tv.bulges.is_empty() { DragKind::Segment } else { DragKind::Arc });
        let (vertices, mut bulges) = if tv.bulges.is_empty() { (tv.points.clone(), vec![0.0; tv.points.len() - 1]) } else { (tv.vertices.clone(), tv.bulges.clone()) };
        let nseg = vertices.len() - 1;
        let new_vertices = match kind {
            DragKind::Via => return Err("that is a track: drag a segment, corner or arc".into()),
            DragKind::Arc => {
                if d.index >= nseg {
                    return Err(format!("segment {} is past the track's last ({})", d.index, nseg - 1));
                }
                bulges.resize(nseg, 0.0);
                bulges[d.index] = pns::bulge_through(vertices[d.index], vertices[d.index + 1], d.to);
                vertices.clone()
            }
            _ if !tv.bulges.is_empty() => return Err("a track with arcs drags by its arcs (kind: arc)".into()),
            DragKind::Segment => {
                if d.index >= nseg {
                    return Err(format!("segment {} is past the track's last ({})", d.index, nseg - 1));
                }
                let from = d.from.unwrap_or([(vertices[d.index][0] + vertices[d.index + 1][0]) / 2.0, (vertices[d.index][1] + vertices[d.index + 1][1]) / 2.0]);
                bulges.clear();
                drag_segment(&vertices, d.index, geom::sub(d.to, from), angle)
            }
            DragKind::Corner => {
                if d.index > nseg {
                    return Err(format!("corner {} is past the track's last ({nseg})", d.index));
                }
                bulges.clear();
                drag_corner(&vertices, d.index, d.to, angle)
            }
        };
        if new_vertices.len() < 2 {
            return Err("the drag would fold the track away".into());
        }
        let any_arc = bulges.iter().any(|b| b.abs() > 1e-9);
        let flat = if any_arc { pns::flatten(&new_vertices, &bulges) } else { new_vertices.clone() };
        let line = &mut node.lines[li];
        line.points = flat;
        line.moved = true;
        line.locked = true;
        pushers.push(Pusher::Line(li));
        keep.insert(d.target);
        plan.tracks.push(MovedTrack { id: d.target, points: new_vertices, bulges: if any_arc { bulges } else { vec![] } });
        plan.net = tv.net.clone();
        plan.width = tv.width;
        plan.end = d.to;
        plan.layer = tv.layer.clone();
    }
    plan.clearance = node.clearance(&plan.net, &None);
    if mode == Mode::Shove {
        let mut trial = node.clone();
        match trial.shove(&pushers) {
            Ok(()) => node = trial,
            Err(e) => {
                plan.notes.push(format!("cannot shove: {e}"));
                plan.mode = Mode::Highlight;
            }
        }
    } else {
        plan.mode = Mode::Highlight;
    }
    plan.collisions = moved_collisions(&node);
    moved_of(&node, &keep, &mut plan);
    plan.ok = plan.collisions.is_empty();
    Ok(plan)
}

impl Command for RouteDrag {
    const NAME: &'static str = "pcb.route.drag";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (b, v) = resolved(tx, self.board)?;
        let plan = plan_drag(&b, &v, &self).map_err(bad)?;
        if self.mode.unwrap_or(Mode::Shove) == Mode::Shove && !plan.ok {
            let why = plan.notes.first().cloned().unwrap_or_else(|| plan.collisions.iter().map(|c| format!("too close to {}", c.with)).collect::<Vec<_>>().join(", "));
            return Err(bad(format!("that drag would break a clearance ({why}); drag in highlight mode to place it anyway")));
        }
        apply(tx, self.board, &plan)
    }
}

// ---- Connections ----------------------------------------------------------------

/// Tracks and vias joined to `seeds` (tracks, vias, or components: their pads)
/// through copper of the same net, stopping at pads — "select connection".
pub fn connection(v: &BoardView, seeds: &[StableId]) -> BTreeSet<StableId> {
    let nl = v.layers.len();
    let all = ((1u16 << nl) - 1) as u8;
    // Pieces: (owner, shape, mask, net).
    let mut pieces: Vec<(StableId, Shape, u8, Option<String>)> = Vec::new();
    for t in &v.tracks {
        let Some(k) = layer_index(&v.layers, &t.layer) else { continue };
        for w in t.points.windows(2) {
            pieces.push((t.id, Shape::capsule(w[0], w[1], t.width / 2.0), 1 << k, t.net.clone()));
        }
    }
    for x in &v.vias {
        pieces.push((x.id, Shape::circle(x.at, x.diameter / 2.0), all, x.net.clone()));
    }
    let mut out: BTreeSet<StableId> = BTreeSet::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, p) in pieces.iter().enumerate() {
        if seeds.contains(&p.0) {
            queue.push_back(i);
        }
    }
    for p in v.pads.iter().filter(|p| seeds.contains(&p.component)) {
        let mut mask = 0u8;
        for l in &p.layers {
            if let Some(k) = layer_index(&v.layers, l) {
                mask |= 1 << k;
            }
        }
        let s = Shape::of(&p.prim);
        for (i, q) in pieces.iter().enumerate() {
            if q.2 & mask != 0 && q.3 == p.net && pns::gap(&s, &q.1) <= 1e-6 {
                queue.push_back(i);
            }
        }
    }
    let mut seen = vec![false; pieces.len()];
    while let Some(i) = queue.pop_front() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        out.insert(pieces[i].0);
        let (id, s, m, net) = &pieces[i];
        for (j, q) in pieces.iter().enumerate() {
            if !seen[j] && q.2 & m != 0 && (q.0 == *id || (q.3 == *net && geom::bbox_overlap(&s.bbox, &q.1.bbox, 1e-6) && pns::gap(s, &q.1) <= 1e-6)) {
                queue.push_back(j);
            }
        }
    }
    out
}

/// `pcb.route.unroute`: remove the tracks and vias of nets, of what joins
/// parts' pads, or of the connections through given tracks and vias.
#[derive(Deserialize, JsonSchema)]
pub struct Unroute {
    pub board: StableId,
    #[serde(default)]
    pub nets: Vec<String>,
    /// Components: the copper joined to their pads.
    #[serde(default)]
    pub components: Vec<StableId>,
    /// Tracks or vias: their whole connection (pad to pad).
    #[serde(default)]
    pub targets: Vec<StableId>,
}

impl Command for Unroute {
    const NAME: &'static str = "pcb.route.unroute";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (_, v) = resolved(tx, self.board)?;
        let mut gone: BTreeSet<StableId> = BTreeSet::new();
        for t in &v.tracks {
            if t.net.as_ref().is_some_and(|n| self.nets.contains(n)) {
                gone.insert(t.id);
            }
        }
        for x in &v.vias {
            if x.net.as_ref().is_some_and(|n| self.nets.contains(n)) {
                gone.insert(x.id);
            }
        }
        let seeds: Vec<StableId> = self.components.iter().chain(self.targets.iter()).copied().collect();
        gone.extend(connection(&v, &seeds));
        let (mut tracks, mut vias) = (0, 0);
        for id in &gone {
            if tx.get::<Track>(*id).is_some() {
                tracks += 1;
            } else {
                vias += 1;
            }
            tx.remove_dependency(self.board, *id)?;
            tx.delete(*id)?;
        }
        Ok(json!({ "tracks": tracks, "vias": vias, "removed": gone }))
    }
}

// ---- Optimise ------------------------------------------------------------------

fn octilinear(pts: &[P]) -> bool {
    pts.windows(2).all(|w| {
        let d = geom::sub(w[1], w[0]);
        let l = geom::len(d);
        l < 1e-9 || d[0].abs() < 1e-7 * l || d[1].abs() < 1e-7 * l || (d[0].abs() - d[1].abs()).abs() < 1e-7 * l
    })
}

/// Square corners cut at 45° where the cut keeps the rules.
fn mitre(pts: &[P], width: f64, ok: &dyn Fn(&[P]) -> bool) -> Vec<P> {
    let mut out = pts.to_vec();
    let mut i = 1;
    while i + 1 < out.len() {
        let (a, p, c) = (out[i - 1], out[i], out[i + 1]);
        let (u, w) = (geom::sub(p, a), geom::sub(c, p));
        let (lu, lw) = (geom::len(u), geom::len(w));
        if lu > 1e-9 && lw > 1e-9 && (geom::dot(u, w) / (lu * lw)).abs() < 1e-6 {
            let cut = (lu / 2.0).min(lw / 2.0).min(4.0 * width);
            let p1 = geom::sub(p, [u[0] / lu * cut, u[1] / lu * cut]);
            let p2 = geom::add(p, [w[0] / lw * cut, w[1] / lw * cut]);
            if cut > 1e-3 && ok(&[p1, p2]) {
                out.splice(i..=i, [p1, p2]);
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `pcb.route.optimize`: smooth routed tracks — runs of corners replaced by
/// one 45° dogleg where that keeps every clearance, square corners mitred —
/// keeping their ends. All tracks of the board, of `nets`, or `targets`.
#[derive(Deserialize, JsonSchema)]
pub struct Optimize {
    pub board: StableId,
    #[serde(default)]
    pub targets: Vec<StableId>,
    #[serde(default)]
    pub nets: Vec<String>,
}

impl Command for Optimize {
    const NAME: &'static str = "pcb.route.optimize";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (b, v) = resolved(tx, self.board)?;
        let mut node = Node::new(&b, &v);
        let mut changed = Vec::new();
        let (mut before_pts, mut after_pts, mut saved) = (0usize, 0usize, 0.0f64);
        for li in 0..node.lines.len() {
            let l = node.lines[li].clone();
            let Some(id) = l.id else { continue };
            let wanted = if !self.targets.is_empty() { self.targets.contains(&id) } else if !self.nets.is_empty() { l.net.as_ref().is_some_and(|n| self.nets.contains(n)) } else { true };
            if !wanted || l.arcs || l.points.len() < 3 {
                continue;
            }
            let angle = if octilinear(&l.points) { Angle::Diagonal } else { Angle::Free };
            let ok = |c: &[P]| node.path_clear(c, l.width, l.layer, &l.net, &|r| matches!(r, Ref::Line(j, _) if j == li));
            let mut pts = pns::smooth(&l.points, angle, &ok);
            if angle == Angle::Diagonal {
                pts = pns::clean(&mitre(&pts, l.width, &ok));
            }
            if pts != l.points && pns::length(&pts) <= pns::length(&l.points) + 1e-6 {
                before_pts += l.points.len();
                after_pts += pts.len();
                saved += pns::length(&l.points) - pns::length(&pts);
                let mut t = tx.get::<Track>(id).cloned().ok_or_else(|| unresolved(format!("{id} is not a track")))?;
                t.points = pts.clone();
                t.reroute = false;
                tx.set(id, t)?;
                node.lines[li].points = pts;
                changed.push(id);
            }
        }
        Ok(json!({ "changed": changed, "corners_before": before_pts.saturating_sub(2 * changed.len()), "corners_after": after_pts.saturating_sub(2 * changed.len()), "shorter_mm": (saved * 1000.0).round() / 1000.0 }))
    }
}

// ---- Clean-up ------------------------------------------------------------------

/// `pcb.route.cleanup`: tidy hand-made copper — straight-through and repeated
/// points out, duplicate tracks and vias removed, tracks that meet end to end
/// (nothing else there) joined, dangling stubs trimmed, vias that join fewer
/// than two layers removed.
#[derive(Deserialize, JsonSchema)]
pub struct Cleanup {
    pub board: StableId,
    /// Merge straight-through points and join tracks end to end (default true).
    pub merge: Option<bool>,
    /// Trim track ends that touch nothing of their net (default true).
    pub dangling: Option<bool>,
    /// Remove duplicate vias and vias joining fewer than two layers (default true).
    pub vias: Option<bool>,
}

struct CleanTrack {
    id: StableId,
    t: Track,
    gone: bool,
    changed: bool,
}

impl Command for Cleanup {
    const NAME: &'static str = "pcb.route.cleanup";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (_, v) = resolved(tx, self.board)?;
        let items = crate::items(tx.world(), self.board);
        let mut ts: Vec<CleanTrack> = items.tracks.into_iter().map(|(id, t)| CleanTrack { id, t, gone: false, changed: false }).collect();
        let mut vias: Vec<(StableId, Via, bool)> = items.vias.into_iter().map(|(id, x)| (id, x, false)).collect();
        let (mut points, mut duplicates, mut joined, mut trimmed, mut redundant) = (0usize, 0usize, 0usize, 0usize, 0usize);
        let merge = self.merge.unwrap_or(true);
        let arcs = |t: &Track| t.bulges.iter().any(|b| *b != 0.0);
        if merge {
            for c in ts.iter_mut().filter(|c| !arcs(&c.t)) {
                let p = pns::clean(&c.t.points);
                if p != c.t.points {
                    points += c.t.points.len() - p.len();
                    c.changed = true;
                    if p.len() < 2 {
                        c.gone = true;
                    }
                    c.t.points = p;
                }
            }
        }
        // Duplicates.
        for i in 0..ts.len() {
            for j in i + 1..ts.len() {
                if ts[i].gone || ts[j].gone {
                    continue;
                }
                let (a, b) = (&ts[i].t, &ts[j].t);
                let rev: Vec<P> = b.points.iter().rev().copied().collect();
                if a.layer == b.layer && a.net == b.net && (a.width - b.width).abs() < 1e-9 && a.bulges.is_empty() && b.bulges.is_empty() && (a.points == b.points || a.points == rev) {
                    ts[j].gone = true;
                    duplicates += 1;
                }
            }
        }
        if self.vias.unwrap_or(true) {
            for i in 0..vias.len() {
                for j in i + 1..vias.len() {
                    if !vias[i].2 && !vias[j].2 && vias[i].1.net == vias[j].1.net && geom::dist(vias[i].1.at, vias[j].1.at) < 1e-6 {
                        vias[j].2 = true;
                        duplicates += 1;
                    }
                }
            }
        }
        // What touches a point on a layer (pads, vias, other tracks, the net's pours).
        let touches = |ts: &[CleanTrack], vias: &[(StableId, Via, bool)], me: usize, at: P, layer: &str, net: &Option<String>, ends_only: bool| -> usize {
            let probe = Prim::circle(at, 0.0);
            let mut n = 0;
            n += v.pads.iter().filter(|p| p.layers.iter().any(|l| l == layer) && geom::gap(&p.prim, &probe) <= 1e-6).count();
            n += vias.iter().filter(|x| !x.2 && geom::dist(x.1.at, at) <= x.1.diameter / 2.0 + 1e-6).count();
            n += v.zones.iter().filter(|z| z.layer == layer && Some(&z.net) == net.as_ref() && z.fill.iter().any(|isl| geom::inside_area(at, isl))).count();
            for (k, c) in ts.iter().enumerate() {
                if k == me || c.gone || c.t.layer != layer {
                    continue;
                }
                let line = pns::flatten(&c.t.points, &c.t.bulges);
                let hit = if ends_only { geom::dist(line[0], at) < 1e-6 || geom::dist(*line.last().unwrap(), at) < 1e-6 } else { line.windows(2).any(|w| geom::point_seg_dist(at, w[0], w[1]) <= c.t.width / 2.0 + 1e-6) };
                if hit {
                    n += 1;
                }
            }
            n
        };
        if self.dangling.unwrap_or(true) {
            for _ in 0..50 {
                let mut any = false;
                for i in 0..ts.len() {
                    if ts[i].gone {
                        continue;
                    }
                    let t = ts[i].t.clone();
                    let (a, z) = (t.points[0], *t.points.last().unwrap());
                    let free_a = touches(&ts, &vias, i, a, &t.layer, &t.net, false) == 0;
                    let free_z = touches(&ts, &vias, i, z, &t.layer, &t.net, false) == 0;
                    if !free_a && !free_z {
                        continue;
                    }
                    let c = &mut ts[i];
                    if free_z {
                        c.t.points.pop();
                        if !c.t.bulges.is_empty() {
                            c.t.bulges.truncate(c.t.points.len().saturating_sub(1));
                        }
                    }
                    if free_a && c.t.points.len() >= 2 {
                        c.t.points.remove(0);
                        if !c.t.bulges.is_empty() {
                            c.t.bulges.remove(0);
                        }
                    }
                    trimmed += 1;
                    c.changed = true;
                    if c.t.points.len() < 2 {
                        c.gone = true;
                    }
                    any = true;
                }
                if !any {
                    break;
                }
            }
        }
        if merge {
            // Join tracks meeting end to end with nothing else at the joint.
            loop {
                let mut did = false;
                'outer: for i in 0..ts.len() {
                    for j in 0..ts.len() {
                        if i == j || ts[i].gone || ts[j].gone {
                            continue;
                        }
                        let (a, b) = (&ts[i].t, &ts[j].t);
                        if a.layer != b.layer || a.net != b.net || (a.width - b.width).abs() > 1e-9 || arcs(a) || arcs(b) || a.net.is_none() {
                            continue;
                        }
                        let (az, b0, bz) = (*a.points.last().unwrap(), b.points[0], *b.points.last().unwrap());
                        let rev = if geom::dist(az, b0) < 1e-6 {
                            false
                        } else if geom::dist(az, bz) < 1e-6 {
                            true
                        } else {
                            continue;
                        };
                        let probe = Prim::circle(az, 0.0);
                        let others = touches(&ts, &vias, i, az, &a.layer, &a.net, false) - 1;
                        let on_pad = v.pads.iter().any(|p| p.layers.iter().any(|l| *l == a.layer) && geom::gap(&p.prim, &probe) <= 1e-6);
                        if others > 0 || on_pad {
                            continue;
                        }
                        let mut more: Vec<P> = b.points.clone();
                        if rev {
                            more.reverse();
                        }
                        let mut pts = a.points.clone();
                        pts.extend(more.into_iter().skip(1));
                        ts[i].t.points = pns::clean(&pts);
                        ts[i].changed = true;
                        ts[j].gone = true;
                        joined += 1;
                        did = true;
                        break 'outer;
                    }
                }
                if !did {
                    break;
                }
            }
        }
        if self.vias.unwrap_or(true) {
            for i in 0..vias.len() {
                if vias[i].2 {
                    continue;
                }
                let x = vias[i].1.clone();
                let r = x.diameter / 2.0;
                let mut layers: BTreeSet<String> = BTreeSet::new();
                for c in ts.iter().filter(|c| !c.gone && c.t.net == x.net) {
                    let line = pns::flatten(&c.t.points, &c.t.bulges);
                    if line.windows(2).any(|w| geom::point_seg_dist(x.at, w[0], w[1]) <= r + c.t.width / 2.0 + 1e-6) {
                        layers.insert(c.t.layer.clone());
                    }
                }
                let probe = Prim::circle(x.at, r);
                for p in v.pads.iter().filter(|p| p.net == x.net && geom::gap(&p.prim, &probe) <= 1e-6) {
                    layers.extend(p.layers.iter().cloned());
                }
                for z in v.zones.iter().filter(|z| Some(&z.net) == x.net.as_ref() && z.fill.iter().any(|isl| geom::inside_area(x.at, isl))) {
                    layers.insert(z.layer.clone());
                }
                if layers.len() < 2 {
                    vias[i].2 = true;
                    redundant += 1;
                }
            }
        }
        let mut removed_tracks = 0;
        let mut changed_tracks = 0;
        for c in ts {
            if c.gone {
                tx.remove_dependency(self.board, c.id)?;
                tx.delete(c.id)?;
                removed_tracks += 1;
            } else if c.changed {
                tx.set(c.id, c.t)?;
                changed_tracks += 1;
            }
        }
        let mut removed_vias = 0;
        for (id, _, gone) in vias {
            if gone {
                tx.remove_dependency(self.board, id)?;
                tx.delete(id)?;
                removed_vias += 1;
            }
        }
        Ok(json!({ "points_merged": points, "duplicates": duplicates, "joined": joined, "trimmed": trimmed, "redundant_vias": redundant, "removed_tracks": removed_tracks, "removed_vias": removed_vias, "changed_tracks": changed_tracks }))
    }
}

// ---- Break, fillet, fan-out ---------------------------------------------------

/// `pcb.track.break`: split a track in two at the point on it nearest `at`.
#[derive(Deserialize, JsonSchema)]
pub struct BreakTrack {
    pub target: StableId,
    pub at: [f64; 2],
}

impl Command for BreakTrack {
    const NAME: &'static str = "pcb.track.break";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let t = tx.get::<Track>(self.target).cloned().ok_or_else(|| unresolved(format!("{} is not a track", self.target)))?;
        let mut best = (f64::INFINITY, 0usize, self.at);
        for (k, w) in t.points.windows(2).enumerate() {
            let ab = geom::sub(w[1], w[0]);
            let l2 = geom::dot(ab, ab).max(1e-24);
            let s = (geom::dot(geom::sub(self.at, w[0]), ab) / l2).clamp(0.0, 1.0);
            let q = geom::add(w[0], [ab[0] * s, ab[1] * s]);
            let d = geom::dist(q, self.at);
            if d < best.0 {
                best = (d, k, q);
            }
        }
        let (_, k, q) = best;
        let arcs = t.bulges.iter().any(|b| *b != 0.0);
        let bulge = |i: usize| t.bulges.get(i).copied().unwrap_or(0.0);
        let at_corner = t.points.iter().position(|p| geom::dist(*p, q) < 1e-6);
        if at_corner.is_none() && bulge(k).abs() > 1e-12 {
            return Err(bad("an arc breaks at its ends"));
        }
        let ((a, ba), (b, bb)): ((Vec<P>, Vec<f64>), (Vec<P>, Vec<f64>)) = match at_corner {
            Some(i) => ((t.points[..=i].to_vec(), (0..i).map(bulge).collect()), (t.points[i..].to_vec(), (i..t.points.len() - 1).map(bulge).collect())),
            None => {
                let mut a = t.points[..=k].to_vec();
                a.push(q);
                let mut b = vec![q];
                b.extend_from_slice(&t.points[k + 1..]);
                ((a, (0..=k).map(bulge).collect()), (b, (k..t.points.len() - 1).map(bulge).collect()))
            }
        };
        if a.len() < 2 || b.len() < 2 {
            return Err(bad("that is the track's end: nothing to break"));
        }
        let (ba, bb) = if arcs { (ba, bb) } else { (vec![], vec![]) };
        let board = t.board;
        tx.set(self.target, Track { points: a, bulges: ba, ..t.clone() })?;
        let id = tx.create();
        tx.set(id, Track { points: b, bulges: bb, reroute: false, ..t })?;
        tx.add_dependency(board, id)?;
        Ok(json!({ "tracks": [self.target, id], "at": q }))
    }
}

/// `pcb.track.fillet`: round tracks' corners into arcs of `radius` (smaller
/// where a leg is short, or where the full radius would break a clearance).
#[derive(Deserialize, JsonSchema)]
pub struct FilletTracks {
    pub targets: Vec<StableId>,
    pub radius: f64,
}

impl Command for FilletTracks {
    const NAME: &'static str = "pcb.track.fillet";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if !(self.radius > 0.0 && self.radius <= 100.0) {
            return Err(bad("radius is above 0 and up to 100 mm"));
        }
        let mut out = Vec::new();
        let mut views: BTreeMap<StableId, (Board, BoardView)> = BTreeMap::new();
        for id in &self.targets {
            let t = tx.get::<Track>(*id).cloned().ok_or_else(|| unresolved(format!("{id} is not a track")))?;
            if !views.contains_key(&t.board) {
                views.insert(t.board, resolved(tx, t.board)?);
            }
            let (b, v) = &views[&t.board];
            let node = Node::new(b, v);
            let li = node.line_by_id(*id).ok_or_else(|| unresolved(format!("{id} is not on its board")))?;
            let straight = pns::clean(&if t.bulges.is_empty() { t.points.clone() } else { t.points.clone() });
            let skip = |r: Ref| matches!(r, Ref::Line(j, _) if j == li);
            let base = node.line_report(&pns::flatten(&t.points, &t.bulges), t.width, node.lines[li].layer, &t.net, &skip).len();
            let mut r = self.radius;
            let mut made = None;
            for _ in 0..4 {
                let (pts, bulges) = pns::fillet(&straight, r);
                if node.line_report(&pns::flatten(&pts, &bulges), t.width, node.lines[li].layer, &t.net, &skip).len() <= base {
                    made = Some((pts, bulges));
                    break;
                }
                r /= 2.0;
            }
            match made {
                Some((pts, bulges)) => {
                    tx.set(*id, Track { points: pts, bulges, reroute: false, ..t })?;
                    out.push(json!({ "id": id, "radius": r }));
                }
                None => out.push(json!({ "id": id, "radius": null, "why": "no room for an arc that keeps the clearances" })),
            }
        }
        Ok(json!({ "tracks": out }))
    }
}

/// `pcb.fanout`: a short 45° stub and a via from each of a part's SMD pads,
/// outward, where they keep every clearance — for pads reached on other layers
/// (a pour's net, a fine-pitch part). `nets` limits it; pads already joined to
/// copper are left alone.
#[derive(Deserialize, JsonSchema)]
pub struct Fanout {
    pub component: StableId,
    #[serde(default)]
    pub nets: Vec<String>,
    #[serde(default)]
    pub skip: Vec<String>,
    /// Longest stub (mm), default 2.
    pub length: Option<f64>,
}

impl Command for Fanout {
    const NAME: &'static str = "pcb.fanout";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let c = tx.get::<crate::PcbComponent>(self.component).cloned().ok_or_else(|| unresolved(format!("{} is not a component", self.component)))?;
        let (b, v) = resolved(tx, c.board)?;
        let mut node = Node::new(&b, &v);
        let reach = self.length.unwrap_or(2.0).clamp(0.2, 20.0);
        let joined = connection(&v, &[self.component]);
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let dirs: [P; 8] = [[1.0, 0.0], [d, d], [0.0, 1.0], [-d, d], [-1.0, 0.0], [-d, -d], [0.0, -1.0], [d, -d]];
        let (mut fanned, mut skipped, mut tracks, mut vias) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for p in v.pads.iter().filter(|p| p.component == self.component && p.kind == crate::footprints::PadKind::Smd) {
            let Some(net) = p.net.clone() else { continue };
            if (!self.nets.is_empty() && !self.nets.contains(&net)) || self.skip.contains(&net) {
                continue;
            }
            let name = format!("{}.{}", p.reference, p.number);
            let s = Shape::of(&p.prim);
            if v.tracks.iter().any(|t| joined.contains(&t.id) && t.net.as_ref() == Some(&net) && t.points.windows(2).any(|w| pns::gap(&s, &Shape::capsule(w[0], w[1], t.width / 2.0)) <= 1e-6)) {
                skipped.push(json!({ "pad": name, "why": "already routed" }));
                continue;
            }
            let Some(layer) = p.layers.first().and_then(|l| layer_index(&v.layers, l)) else { continue };
            let class = b.class_of(Some(&net));
            let net = Some(net);
            let out = geom::sub(p.at, c.at);
            let mut order: Vec<P> = dirs.to_vec();
            order.sort_by(|a, b| geom::dot(*b, out).total_cmp(&geom::dot(*a, out)));
            let mut done = false;
            'dirs: for u in order.iter().take(5) {
                let edge = s.hull.iter().map(|q| geom::dot(geom::sub(*q, p.at), *u)).fold(0.0f64, f64::max) + s.r;
                let start = edge + class.via_diameter / 2.0 + node.clearance(&net, &None);
                let mut dist = start;
                while dist <= start + reach {
                    let q = [p.at[0] + u[0] * dist, p.at[1] + u[1] * dist];
                    let stub = vec![p.at, q];
                    let via_ok = node.via_report(q, class.via_diameter, &net, &|_| false).is_empty() && node.in_board(&Shape::circle(q, class.via_diameter / 2.0));
                    let stub_ok = node.path_clear(&stub, class.track_width, layer, &net, &|_| false);
                    if via_ok && stub_ok {
                        node.lines.push(pns::Line { id: None, net: net.clone(), layer, width: class.track_width, points: stub.clone(), locked: true, arcs: false, moved: false });
                        node.vias.push(pns::Via { id: None, net: net.clone(), at: q, diameter: class.via_diameter, drill: class.via_drill, locked: true, moved: false });
                        let tid = tx.create();
                        tx.set(tid, Track { board: c.board, net: net.clone(), layer: v.layers[layer].clone(), width: class.track_width, points: stub, auto: false, reroute: false, bulges: vec![] })?;
                        tx.add_dependency(c.board, tid)?;
                        let vid = tx.create();
                        tx.set(vid, Via { board: c.board, net: net.clone(), at: q, drill: class.via_drill, diameter: class.via_diameter, auto: false })?;
                        tx.add_dependency(c.board, vid)?;
                        tracks.push(tid);
                        vias.push(vid);
                        fanned.push(name.clone());
                        done = true;
                        break 'dirs;
                    }
                    dist += 0.1;
                }
            }
            if !done {
                skipped.push(json!({ "pad": name, "why": "no room for a via near it" }));
            }
        }
        Ok(json!({ "fanned": fanned, "skipped": skipped, "tracks": tracks, "vias": vias }))
    }
}

// ---- The editor's preview ------------------------------------------------------

/// What the editor asks while the person routes or drags (nothing is changed).
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Preview {
    Route(RouteInteractive),
    Drag(RouteDrag),
    Connection { targets: Vec<StableId> },
}

pub fn preview(b: &Board, v: &BoardView, p: Preview) -> Result<Value, String> {
    match p {
        Preview::Route(r) => plan_route(b, v, &r).map(|x| json!(x)),
        Preview::Drag(d) => plan_drag(b, v, &d).map(|x| json!(x)),
        Preview::Connection { targets } => Ok(json!({ "items": connection(v, &targets) })),
    }
}
