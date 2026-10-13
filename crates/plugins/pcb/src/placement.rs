//! Placement tools for any board items (components, tracks, vias, graphics,
//! zones, rule areas): move by or to a point, move exactly relative to
//! another item, turn about a point, align (left, right, top, bottom, centres)
//! and distribute (centres or equal gaps), swap two items' places, flip to the
//! other side, and arrays (grid or circular copies, references numbered on).
//!
//! Components move through `pcb.component.update`, so the tracks on their
//! pads follow as when the person drags them.

use crate::areas::RuleArea;
use crate::geom::{self, P};
use crate::graphics::Graphic;
use crate::{PcbComponent, Side, Track, Via, Zone, bad};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone)]
pub(crate) enum Thing {
    Component(PcbComponent),
    Track(Track),
    Via(Via),
    Graphic(Graphic),
    Zone(Zone),
    Area(RuleArea),
}

pub(crate) fn load(tx: &Tx, id: StableId) -> Result<Thing, CommandError> {
    if let Some(c) = tx.get::<PcbComponent>(id) {
        return Ok(Thing::Component(c.clone()));
    }
    if let Some(t) = tx.get::<Track>(id) {
        return Ok(Thing::Track(t.clone()));
    }
    if let Some(v) = tx.get::<Via>(id) {
        return Ok(Thing::Via(v.clone()));
    }
    if let Some(g) = tx.get::<Graphic>(id) {
        return Ok(Thing::Graphic(g.clone()));
    }
    if let Some(z) = tx.get::<Zone>(id) {
        return Ok(Thing::Zone(z.clone()));
    }
    if let Some(a) = tx.get::<RuleArea>(id) {
        return Ok(Thing::Area(a.clone()));
    }
    Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a component, track, via, graphic, zone or rule area")))
}

impl Thing {
    pub(crate) fn board(&self) -> StableId {
        match self {
            Thing::Component(c) => c.board,
            Thing::Track(t) => t.board,
            Thing::Via(v) => v.board,
            Thing::Graphic(g) => g.board,
            Thing::Zone(z) => z.board,
            Thing::Area(a) => a.board,
        }
    }

    pub(crate) fn origin(&self) -> P {
        match self {
            Thing::Component(c) => c.at,
            Thing::Track(t) => t.points[0],
            Thing::Via(v) => v.at,
            Thing::Graphic(g) => g.anchor(),
            Thing::Zone(z) => z.outline.first().copied().unwrap_or_default(),
            Thing::Area(a) => a.outline[0],
        }
    }
}

fn bbox_of(pts: impl Iterator<Item = P>) -> [f64; 4] {
    pts.fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// The item's extent on the board (a component by its courtyard).
pub(crate) fn bbox(tx: &Tx, id: StableId, t: &Thing) -> [f64; 4] {
    let view = crate::view(tx.world(), t.board());
    let b = match t {
        Thing::Component(c) => view.as_ref().and_then(|v| v.components.iter().find(|x| x.id == id).filter(|x| !x.courtyard.is_empty()).map(|x| bbox_of(x.courtyard.iter().copied()))).unwrap_or([c.at[0], c.at[1], c.at[0], c.at[1]]),
        Thing::Track(tr) => {
            let b = bbox_of(tr.points.iter().copied());
            [b[0] - tr.width / 2.0, b[1] - tr.width / 2.0, b[2] + tr.width / 2.0, b[3] + tr.width / 2.0]
        }
        Thing::Via(v) => [v.at[0] - v.diameter / 2.0, v.at[1] - v.diameter / 2.0, v.at[0] + v.diameter / 2.0, v.at[1] + v.diameter / 2.0],
        Thing::Graphic(g) => crate::graphics::resolve(id, g).map(|v| v.bbox).unwrap_or_else(|_| {
            let a = g.anchor();
            [a[0], a[1], a[0], a[1]]
        }),
        Thing::Zone(z) => {
            if z.outline.is_empty() {
                view.map(|v| v.bounds).unwrap_or_default()
            } else {
                bbox_of(z.outline.iter().copied())
            }
        }
        Thing::Area(a) => bbox_of(a.outline.iter().copied()),
    };
    if b[0].is_finite() { b } else { [0.0; 4] }
}

/// Writes an item back moved by `f` (points), turned `rot` degrees, and
/// mirrored (flipped to the other side) when `flip`.
pub(crate) fn put(tx: &mut Tx, id: StableId, t: Thing, f: &dyn Fn(P) -> P, rot: f64, flip: bool) -> Result<(), CommandError> {
    let copper: Vec<&'static str> = crate::board_of(tx, t.board())?.copper_layers();
    let fl = |l: &str| crate::graphics::flip_layer(l, &copper);
    match t {
        Thing::Component(c) => {
            let at = f(c.at);
            let mut rotation = c.rotation + rot;
            let mut side = c.side;
            if flip {
                rotation = -c.rotation + rot;
                side = if c.side == Side::Top { Side::Bottom } else { Side::Top };
            }
            tx.run("pcb.component.update", json!({ "target": id, "at": [r6(at[0]), r6(at[1])], "rotation": rotation.rem_euclid(360.0), "side": side }))?;
        }
        Thing::Track(mut tr) => {
            tr.points = tr.points.iter().map(|p| r6p(f(*p))).collect();
            if flip {
                tr.layer = fl(&tr.layer);
                tr.bulges.iter_mut().for_each(|b| *b = -*b);
            }
            tr.auto = false;
            tx.set(id, tr)?;
        }
        Thing::Via(mut v) => {
            v.at = r6p(f(v.at));
            if flip {
                v.spec.from = v.spec.from.as_deref().map(fl);
                v.spec.to = v.spec.to.as_deref().map(fl);
                v.spec.stack = v.spec.stack.iter().map(|(l, d)| (fl(l), *d)).collect();
            }
            v.auto = false;
            tx.set(id, v)?;
        }
        Thing::Graphic(mut g) => {
            g.map(&|p| r6p(f(p)), rot, flip);
            if flip {
                g.layer = fl(&g.layer);
            }
            tx.set(id, g)?;
        }
        Thing::Zone(mut z) => {
            z.outline = z.outline.iter().map(|p| r6p(f(*p))).collect();
            z.cutouts = z.cutouts.iter().map(|c| c.iter().map(|p| r6p(f(*p))).collect()).collect();
            if flip {
                z.layer = fl(&z.layer);
            }
            tx.set(id, z)?;
        }
        Thing::Area(mut a) => {
            a.outline = a.outline.iter().map(|p| r6p(f(*p))).collect();
            if flip {
                a.layers = a.layers.iter().map(|l| fl(l)).collect();
            }
            tx.set(id, a)?;
        }
    }
    Ok(())
}

fn r6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}
fn r6p(p: P) -> P {
    [r6(p[0]), r6(p[1])]
}

fn load_all(tx: &Tx, targets: &[StableId]) -> Result<Vec<(StableId, Thing)>, CommandError> {
    if targets.is_empty() {
        return Err(bad("say which items (targets)"));
    }
    let mut out = Vec::new();
    for id in targets {
        out.push((*id, load(tx, *id)?));
    }
    let b = out[0].1.board();
    if out.iter().any(|(_, t)| t.board() != b) {
        return Err(bad("the items are on different boards"));
    }
    Ok(out)
}

fn locked(t: &Thing) -> bool {
    matches!(t, Thing::Graphic(g) if g.locked)
}

/// `pcb.move`: move items `by` [dx, dy], or so the first lands `to` [x, y], or
/// exactly relative to another item (`relative_to` its origin plus `offset`);
/// and/or turn them `rotate` degrees about `about` (default: their centre).
#[derive(Deserialize, JsonSchema)]
pub struct Move {
    pub targets: Vec<StableId>,
    pub by: Option<[f64; 2]>,
    pub to: Option<[f64; 2]>,
    pub relative_to: Option<StableId>,
    pub offset: Option<[f64; 2]>,
    pub rotate: Option<f64>,
    pub about: Option<[f64; 2]>,
}

impl Command for Move {
    const NAME: &'static str = "pcb.move";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let things = load_all(tx, &self.targets)?;
        let first = things[0].1.origin();
        let d = if let Some(b) = self.by {
            b
        } else if let Some(r) = self.relative_to {
            let o = load(tx, r)?.origin();
            let off = self.offset.unwrap_or_default();
            [o[0] + off[0] - first[0], o[1] + off[1] - first[1]]
        } else if let Some(t) = self.to {
            [t[0] - first[0], t[1] - first[1]]
        } else {
            [0.0, 0.0]
        };
        let rot = self.rotate.unwrap_or(0.0);
        let bb = bbox_of(things.iter().map(|(id, t)| bbox(tx, *id, t)).flat_map(|b| [[b[0], b[1]], [b[2], b[3]]]));
        let about = self.about.unwrap_or([(bb[0] + bb[2]) / 2.0, (bb[1] + bb[3]) / 2.0]);
        let about = [about[0] + d[0], about[1] + d[1]];
        let f = move |p: P| {
            let q = [p[0] + d[0], p[1] + d[1]];
            geom::add(about, geom::rotate(geom::sub(q, about), rot))
        };
        let mut n = 0;
        for (id, t) in things {
            if locked(&t) {
                continue;
            }
            put(tx, id, t, &f, rot, false)?;
            n += 1;
        }
        Ok(json!({ "moved": n, "by": d, "rotate": rot }))
    }
}

/// `pcb.align`: line items up: "left", "right", "top", "bottom", "center_x"
/// (one vertical centre line) or "center_y"; to the `reference` item's edge
/// or centre when given, else to the outermost of them (their mean for centres).
#[derive(Deserialize, JsonSchema)]
pub struct Align {
    pub targets: Vec<StableId>,
    pub to: String,
    pub reference: Option<StableId>,
}

impl Command for Align {
    const NAME: &'static str = "pcb.align";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let things = load_all(tx, &self.targets)?;
        let boxes: Vec<[f64; 4]> = things.iter().map(|(id, t)| bbox(tx, *id, t)).collect();
        let key = |b: &[f64; 4]| -> f64 {
            match self.to.as_str() {
                "left" => b[0],
                "right" => b[2],
                "bottom" => b[1],
                "top" => b[3],
                "center_x" => (b[0] + b[2]) / 2.0,
                _ => (b[1] + b[3]) / 2.0,
            }
        };
        if !["left", "right", "top", "bottom", "center_x", "center_y"].contains(&self.to.as_str()) {
            return Err(bad("to is left, right, top, bottom, center_x or center_y"));
        }
        let target = match self.reference {
            Some(r) => {
                let t = load(tx, r)?;
                key(&bbox(tx, r, &t))
            }
            None => {
                let ks: Vec<f64> = boxes.iter().map(key).collect();
                match self.to.as_str() {
                    "left" | "bottom" => ks.iter().cloned().fold(f64::INFINITY, f64::min),
                    "right" | "top" => ks.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    _ => ks.iter().sum::<f64>() / ks.len() as f64,
                }
            }
        };
        let horizontal = matches!(self.to.as_str(), "left" | "right" | "center_x");
        let mut n = 0;
        for ((id, t), b) in things.into_iter().zip(boxes) {
            if Some(id) == self.reference || locked(&t) {
                continue;
            }
            let delta = target - key(&b);
            if delta.abs() < 1e-9 {
                continue;
            }
            let d = if horizontal { [delta, 0.0] } else { [0.0, delta] };
            put(tx, id, t, &move |p: P| [p[0] + d[0], p[1] + d[1]], 0.0, false)?;
            n += 1;
        }
        Ok(json!({ "moved": n, "to": self.to, "at": target }))
    }
}

/// `pcb.distribute`: space items evenly along `axis` "x" or "y", between the
/// outermost two: equal steps between `centers` (default) or equal `gaps`.
#[derive(Deserialize, JsonSchema)]
pub struct Distribute {
    pub targets: Vec<StableId>,
    pub axis: String,
    pub by: Option<String>,
}

impl Command for Distribute {
    const NAME: &'static str = "pcb.distribute";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let k = match self.axis.as_str() {
            "x" => 0,
            "y" => 1,
            _ => return Err(bad("axis is x or y")),
        };
        let mut things: Vec<(StableId, Thing, [f64; 4])> = load_all(tx, &self.targets)?.into_iter().map(|(id, t)| {
            let b = bbox(tx, id, &t);
            (id, t, b)
        }).collect();
        if things.len() < 3 {
            return Err(bad("distribute three or more items"));
        }
        let lo = |b: &[f64; 4]| b[k];
        let hi = |b: &[f64; 4]| b[k + 2];
        things.sort_by(|a, b| (lo(&a.2) + hi(&a.2)).total_cmp(&(lo(&b.2) + hi(&b.2))));
        let n = things.len();
        let gaps = self.by.as_deref() == Some("gaps");
        let mut wanted: Vec<f64> = Vec::new(); // new low edge (gaps) or centre (centres)
        if gaps {
            let span = hi(&things[n - 1].2) - lo(&things[0].2);
            let sizes: f64 = things.iter().map(|t| hi(&t.2) - lo(&t.2)).sum();
            let gap = (span - sizes) / (n - 1) as f64;
            let mut at = lo(&things[0].2);
            for t in &things {
                wanted.push(at);
                at += hi(&t.2) - lo(&t.2) + gap;
            }
        } else {
            let c0 = (lo(&things[0].2) + hi(&things[0].2)) / 2.0;
            let c1 = (lo(&things[n - 1].2) + hi(&things[n - 1].2)) / 2.0;
            for i in 0..n {
                wanted.push(c0 + (c1 - c0) * i as f64 / (n - 1) as f64);
            }
        }
        let mut moved = 0;
        for ((id, t, b), w) in things.into_iter().zip(wanted) {
            let now = if gaps { lo(&b) } else { (lo(&b) + hi(&b)) / 2.0 };
            let delta = w - now;
            if delta.abs() < 1e-9 || locked(&t) {
                continue;
            }
            let mut d = [0.0, 0.0];
            d[k] = delta;
            put(tx, id, t, &move |p: P| [p[0] + d[0], p[1] + d[1]], 0.0, false)?;
            moved += 1;
        }
        Ok(json!({ "moved": moved }))
    }
}

/// `pcb.swap`: two items trade places (components also trade rotation and side).
#[derive(Deserialize, JsonSchema)]
pub struct Swap {
    pub a: StableId,
    pub b: StableId,
}

impl Command for Swap {
    const NAME: &'static str = "pcb.swap";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (ta, tb) = (load(tx, self.a)?, load(tx, self.b)?);
        if let (Thing::Component(ca), Thing::Component(cb)) = (&ta, &tb) {
            // Move one aside first so the references stay unique and tracks follow each.
            tx.run("pcb.component.update", json!({ "target": self.a, "at": cb.at, "rotation": cb.rotation, "side": cb.side }))?;
            tx.run("pcb.component.update", json!({ "target": self.b, "at": ca.at, "rotation": ca.rotation, "side": ca.side }))?;
            return Ok(json!({ "swapped": [self.a, self.b] }));
        }
        let (oa, ob) = (ta.origin(), tb.origin());
        let da = [ob[0] - oa[0], ob[1] - oa[1]];
        put(tx, self.a, ta, &move |p: P| [p[0] + da[0], p[1] + da[1]], 0.0, false)?;
        put(tx, self.b, tb, &move |p: P| [p[0] - da[0], p[1] - da[1]], 0.0, false)?;
        Ok(json!({ "swapped": [self.a, self.b] }))
    }
}

/// `pcb.flip`: items to the other side of the board, mirrored left to right
/// about `about_x` (default: their centre); components change side, copper
/// and graphics change to the mirror layer.
#[derive(Deserialize, JsonSchema)]
pub struct Flip {
    pub targets: Vec<StableId>,
    pub about_x: Option<f64>,
}

impl Command for Flip {
    const NAME: &'static str = "pcb.flip";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let things = load_all(tx, &self.targets)?;
        let bb = bbox_of(things.iter().map(|(id, t)| bbox(tx, *id, t)).flat_map(|b| [[b[0], b[1]], [b[2], b[3]]]));
        let cx = self.about_x.unwrap_or((bb[0] + bb[2]) / 2.0);
        let mut n = 0;
        for (id, t) in things {
            if locked(&t) {
                continue;
            }
            put(tx, id, t, &move |p: P| [2.0 * cx - p[0], p[1]], 0.0, true)?;
            n += 1;
        }
        Ok(json!({ "flipped": n, "about_x": cx }))
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct Grid {
    /// Copies along x (including the original), default 1.
    pub nx: Option<u32>,
    pub ny: Option<u32>,
    /// Pitch (mm).
    pub dx: Option<f64>,
    pub dy: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct Circular {
    pub center: [f64; 2],
    /// Items in the ring, including the original.
    pub count: u32,
    /// Total angle to spread over (default 360°, counter-clockwise).
    pub angle: Option<f64>,
    /// Turn each copy with the ring (default true).
    pub rotate: Option<bool>,
}

/// `pcb.array`: copies of items in a `grid` or round a `circular` ring;
/// component copies get the next free references of their prefix, without
/// nets unless `keep_nets`.
#[derive(Deserialize, JsonSchema)]
pub struct Array {
    pub targets: Vec<StableId>,
    pub grid: Option<Grid>,
    pub circular: Option<Circular>,
    #[serde(default)]
    pub keep_nets: bool,
}

fn next_ref(used: &mut Vec<String>, reference: &str) -> String {
    let i = reference.find(|c: char| c.is_ascii_digit()).unwrap_or(reference.len());
    let prefix = &reference[..i];
    let mut n = used.iter().filter_map(|r| r.strip_prefix(prefix).and_then(|x| x.parse::<u64>().ok())).max().unwrap_or(0) + 1;
    loop {
        let r = format!("{prefix}{n}");
        if !used.iter().any(|u| u.eq_ignore_ascii_case(&r)) {
            used.push(r.clone());
            return r;
        }
        n += 1;
    }
}

/// A copy of an item placed through `f`, turned `rot`; returns its id.
pub(crate) fn copy(tx: &mut Tx, t: &Thing, f: &dyn Fn(P) -> P, rot: f64, used: &mut Vec<String>, keep_nets: bool) -> Result<StableId, CommandError> {
    let board = t.board();
    let id = match t {
        Thing::Component(c) => {
            let r = next_ref(used, &c.reference);
            let at = f(c.at);
            let pins = if keep_nets { c.pins.clone() } else { Default::default() };
            let out = tx.run(
                "pcb.component.create",
                json!({ "board": board, "reference": r, "footprint": c.footprint, "value": c.value, "at": [r6(at[0]), r6(at[1])], "rotation": (c.rotation + rot).rem_euclid(360.0), "side": c.side, "mpn": c.mpn, "manufacturer": c.manufacturer, "lcsc": c.lcsc, "description": c.description, "pins": pins, "pin_names": c.pin_names, "dnp": c.dnp }),
            )?;
            return serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()));
        }
        Thing::Track(tr) => {
            let mut tr = tr.clone();
            tr.points = tr.points.iter().map(|p| r6p(f(*p))).collect();
            tr.auto = false;
            tr.reroute = false;
            let id = tx.create();
            tx.set(id, tr)?;
            id
        }
        Thing::Via(v) => {
            let mut v = v.clone();
            v.at = r6p(f(v.at));
            v.auto = false;
            let id = tx.create();
            tx.set(id, v)?;
            id
        }
        Thing::Graphic(g) => {
            let mut g = g.clone();
            g.map(&|p| r6p(f(p)), rot, false);
            let id = tx.create();
            tx.set(id, g)?;
            id
        }
        Thing::Zone(z) => {
            let mut z = z.clone();
            z.outline = z.outline.iter().map(|p| r6p(f(*p))).collect();
            z.cutouts = z.cutouts.iter().map(|c| c.iter().map(|p| r6p(f(*p))).collect()).collect();
            let id = tx.create();
            tx.set(id, z)?;
            id
        }
        Thing::Area(a) => {
            let mut a = a.clone();
            a.outline = a.outline.iter().map(|p| r6p(f(*p))).collect();
            let id = tx.create();
            tx.set(id, a)?;
            id
        }
    };
    tx.add_dependency(board, id)?;
    Ok(id)
}

impl Command for Array {
    const NAME: &'static str = "pcb.array";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let things = load_all(tx, &self.targets)?;
        let board = things[0].1.board();
        let mut used: Vec<String> = crate::items(tx.world(), board).components.into_iter().map(|(_, c)| c.reference).collect();
        // Each copy: a move and a turn.
        let mut steps: Vec<(Box<dyn Fn(P) -> P>, f64)> = Vec::new();
        match (&self.grid, &self.circular) {
            (Some(g), None) => {
                let (nx, ny) = (g.nx.unwrap_or(1).max(1), g.ny.unwrap_or(1).max(1));
                let (dx, dy) = (g.dx.unwrap_or(0.0), g.dy.unwrap_or(0.0));
                if nx * ny > 1000 {
                    return Err(bad("at most 1000 copies"));
                }
                for j in 0..ny {
                    for i in 0..nx {
                        if i == 0 && j == 0 {
                            continue;
                        }
                        let d = [dx * i as f64, dy * j as f64];
                        steps.push((Box::new(move |p: P| [p[0] + d[0], p[1] + d[1]]), 0.0));
                    }
                }
            }
            (None, Some(c)) => {
                if !(2..=1000).contains(&c.count) {
                    return Err(bad("a circular array has 2 to 1000 items"));
                }
                let total = c.angle.unwrap_or(360.0);
                let full = (total.abs() - 360.0).abs() < 1e-9;
                let step = if full { total / c.count as f64 } else { total / (c.count - 1) as f64 };
                let turn = c.rotate.unwrap_or(true);
                for k in 1..c.count {
                    let a = step * k as f64;
                    let center = c.center;
                    steps.push((Box::new(move |p: P| geom::add(center, geom::rotate(geom::sub(p, center), a))), if turn { a } else { 0.0 }));
                }
            }
            _ => return Err(bad("give a grid {nx, ny, dx, dy} or a circular {center, count, angle}")),
        }
        let mut made = Vec::new();
        for (f, rot) in &steps {
            for (_, t) in &things {
                // Without turning copies, items still go round the ring but keep their angle.
                let id = if *rot == 0.0 && matches!(t, Thing::Component(_) | Thing::Graphic(_)) {
                    let o = t.origin();
                    let o2 = f(o);
                    let d = [o2[0] - o[0], o2[1] - o[1]];
                    copy(tx, t, &move |p: P| [p[0] + d[0], p[1] + d[1]], 0.0, &mut used, self.keep_nets)?
                } else {
                    copy(tx, t, f.as_ref(), *rot, &mut used, self.keep_nets)?
                };
                made.push(id);
            }
        }
        crate::model3d::sync(tx, board)?;
        Ok(json!({ "made": made.len(), "ids": made }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<Move>().command::<Align>().command::<Distribute>().command::<Swap>().command::<Flip>().command::<Array>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_references_skip_used_ones() {
        let mut used = vec!["R1".to_string(), "R2".into(), "R7".into(), "C1".into()];
        assert_eq!(next_ref(&mut used, "R2"), "R8");
        assert_eq!(next_ref(&mut used, "R2"), "R9");
        assert_eq!(next_ref(&mut used, "C1"), "C2");
        assert_eq!(next_ref(&mut used, "TP"), "TP1");
    }
}
