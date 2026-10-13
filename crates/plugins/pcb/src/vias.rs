//! Via types and via tools: blind, buried and micro vias between a pair of
//! copper layers, per-layer via pads (a via padstack), editing vias, and
//! laying many at once — stitching an area or a pour with a grid of vias,
//! and fencing a track with a row of vias on each side.
//!
//! A through via joins every copper layer. A *blind* via joins an outer layer
//! to an inner one, a *buried* via two inner layers, a *micro* via (laser
//! drilled) two neighbouring layers across one prepreg. The design-rule check
//! (`drc`) holds them to that, to the stackup and to the micro via limits.

use crate::board::{BoardView, Cu};
use crate::geom::{self, P, Prim};
use crate::{Board, Via, bad, board_of};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViaKind {
    #[default]
    Through,
    Blind,
    Buried,
    Micro,
}

impl ViaKind {
    pub fn name(self) -> &'static str {
        match self {
            ViaKind::Through => "through",
            ViaKind::Blind => "blind",
            ViaKind::Buried => "buried",
            ViaKind::Micro => "micro",
        }
    }
}

fn is_through(k: &ViaKind) -> bool {
    *k == ViaKind::Through
}

/// What a via is beyond its place and size.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ViaSpec {
    #[serde(default, skip_serializing_if = "is_through")]
    pub kind: ViaKind,
    /// First copper layer of a blind, buried or micro via.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Last copper layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Pad diameter per copper layer (mm); 0 leaves no pad there (unused pads removed).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stack: BTreeMap<String, f64>,
    /// Laid as part of a stitching area or fence ("stitch:GND", "fence:RF").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// The copper layers a via spans, as indices (low, high) into `layers`.
pub fn span(layers: &[String], spec: &ViaSpec) -> (usize, usize) {
    let n = layers.len();
    if spec.kind == ViaKind::Through {
        return (0, n - 1);
    }
    let idx = |l: &Option<String>| l.as_deref().and_then(|l| crate::board::layer_index(layers, l));
    let a = idx(&spec.from).unwrap_or(0);
    let b = idx(&spec.to).unwrap_or(match spec.kind {
        ViaKind::Micro | ViaKind::Blind => (a + 1).min(n - 1),
        _ => n - 1,
    });
    (a.min(b), a.max(b))
}

/// Layer mask of a span.
pub fn mask(lo: usize, hi: usize) -> u32 {
    let mut m = 0u32;
    for k in lo..=hi {
        m |= 1 << k;
    }
    m
}

/// Hard limits a via must meet to exist (the rest is the design-rule check's).
pub fn check_via(board: &Board, v: &Via) -> Result<(), String> {
    let layers = board.copper_layers();
    for l in [&v.spec.from, &v.spec.to].into_iter().flatten() {
        if !layers.contains(&l.as_str()) {
            return Err(format!("layer “{l}” is not on this board (use {})", layers.join(", ")));
        }
    }
    if v.spec.kind != ViaKind::Through {
        if layers.len() < 4 && v.spec.kind != ViaKind::Micro {
            return Err(format!("{} vias need 4 or more copper layers", v.spec.kind.name()));
        }
        if v.spec.from.is_some() && v.spec.from == v.spec.to {
            return Err("a via joins two different layers (from ≠ to)".into());
        }
    }
    for (l, d) in &v.spec.stack {
        if !layers.contains(&l.as_str()) {
            return Err(format!("via pad stack: layer “{l}” is not on this board"));
        }
        if !(d.is_finite() && (*d == 0.0 || (*d > v.drill && *d < 10.0))) {
            return Err(format!("via pad stack: the pad on {l} is larger than the drill (or 0 for none)"));
        }
    }
    Ok(())
}

/// Copper pieces of a via: (layer mask, diameter) for each group of layers with one pad size.
pub fn pads(layers: &[String], v: &Via) -> Vec<(u32, f64)> {
    let (lo, hi) = span(layers, &v.spec);
    let mut groups: Vec<(u32, f64)> = Vec::new();
    for k in lo..=hi {
        let d = v.spec.stack.get(&layers[k]).copied().unwrap_or(v.diameter);
        if d <= 0.0 {
            continue;
        }
        match groups.iter_mut().find(|g| (g.1 - d).abs() < 1e-9) {
            Some(g) => g.0 |= 1 << k,
            None => groups.push((1 << k, d)),
        }
    }
    groups
}

// ---- Placing many vias -------------------------------------------------------

/// Whether a via of radius `r` (copper) and `drill` at `p` on layers `m` for
/// `net` keeps clear of everything else on the board (copper of other nets,
/// holes, the edge, keepouts and the other new vias).
pub struct Spot<'a> {
    pub view: &'a BoardView,
    pub board: &'a Board,
    pub cu: &'a [Cu],
}

impl Spot<'_> {
    pub fn free(&self, p: P, r: f64, drill: f64, m: u32, net: &str, placed: &[P]) -> bool {
        let class = self.board.class_of(Some(net));
        let gap = class.clearance.max(self.board.rules.min_space);
        let me = Prim::circle(p, r);
        for c in self.cu {
            if c.layers & m == 0 || c.net.as_deref() == Some(net) {
                continue;
            }
            let need = gap.max(self.board.class_of(c.net.as_deref()).clearance);
            if c.bbox[0] > p[0] + r + need || c.bbox[2] < p[0] - r - need || c.bbox[1] > p[1] + r + need || c.bbox[3] < p[1] - r - need {
                continue;
            }
            if geom::gap(&me, &c.prim) < need {
                return false;
            }
        }
        let h2h = self.board.rules.hole_to_hole;
        for h in &self.view.holes {
            let hp = Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0);
            if geom::gap(&Prim::circle(p, drill / 2.0), &hp) < h2h || geom::gap(&me, &hp) < gap {
                return false;
            }
        }
        for q in placed {
            if geom::dist(*q, p) < drill + h2h.max(2.0 * r + gap - drill) {
                return false;
            }
        }
        let edge = self.board.rules.edge_clearance + r;
        if !geom::in_polygon(p, &self.view.outline) || geom::dist_to_loop(p, &self.view.outline) < edge {
            return false;
        }
        if self.view.cutouts.iter().any(|c| geom::in_polygon(p, c) || geom::dist_to_loop(p, c) < edge) {
            return false;
        }
        for a in &self.view.areas {
            if a.no_vias && a.mask & m != 0 && (geom::in_polygon(p, &a.outline) || geom::dist_to_loop(p, &a.outline) < r) {
                return false;
            }
        }
        true
    }
}

fn view_of(tx: &Tx, board: StableId) -> Result<std::sync::Arc<BoardView>, CommandError> {
    crate::view(tx.world(), board).ok_or_else(|| CommandError::new(ErrorCode::StaleDerivedOutput, "the board is not worked out yet"))
}

fn new_via(tx: &mut Tx, board: StableId, net: &str, at: P, drill: f64, diameter: f64, group: &str) -> Result<StableId, CommandError> {
    let id = tx.create();
    tx.set(id, Via { board, net: Some(net.to_string()), at: [r4(at[0]), r4(at[1])], drill, diameter, auto: false, spec: ViaSpec { group: Some(group.to_string()), ..Default::default() } })?;
    tx.add_dependency(board, id)?;
    Ok(id)
}

fn r4(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

/// Deletes the vias of a group (a stitching area or fence laid again).
fn clear_group(tx: &mut Tx, board: StableId, group: &str) -> Result<usize, CommandError> {
    let it = crate::items(tx.world(), board);
    let mut n = 0;
    for (id, v) in it.vias {
        if v.spec.group.as_deref() == Some(group) {
            tx.remove_dependency(board, id)?;
            tx.delete(id)?;
            n += 1;
        }
    }
    Ok(n)
}

/// `pcb.via.stitch`: fill an area with a grid of vias of `net` (ground
/// stitching between pours): inside a zone's poured copper (`zone`), or an
/// `outline` on the board, every `pitch` mm, wherever a via keeps clear of
/// other copper, holes, the edge and keepouts. The vias form a group, laid
/// again (replacing the old ones) when stitched again; `exclude` points keep
/// a via out of a spot.
#[derive(Deserialize, JsonSchema)]
pub struct Stitch {
    pub board: StableId,
    pub net: Option<String>,
    pub zone: Option<StableId>,
    #[serde(default)]
    pub outline: Vec<[f64; 2]>,
    /// Grid pitch (mm), default 2.5.
    pub pitch: Option<f64>,
    /// Offset every other row by half a pitch.
    #[serde(default)]
    pub stagger: bool,
    pub drill: Option<f64>,
    pub diameter: Option<f64>,
    /// Spots to leave empty (a via within 1 pitch/2 of one is not placed).
    #[serde(default)]
    pub exclude: Vec<[f64; 2]>,
    /// Group name (default "stitch:<net>"); vias already in it are replaced.
    pub group: Option<String>,
}

impl Command for Stitch {
    const NAME: &'static str = "pcb.via.stitch";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        let zone = match self.zone {
            Some(z) => Some(tx.get::<crate::Zone>(z).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{z} is not a zone")))?),
            None => None,
        };
        let net = crate::clean(self.net).or_else(|| zone.as_ref().map(|z| z.net.clone())).ok_or_else(|| bad("say which net to stitch (or give a zone)"))?;
        let pitch = self.pitch.unwrap_or(2.5);
        if !(0.3..=50.0).contains(&pitch) {
            return Err(bad("pitch is 0.3 to 50 mm"));
        }
        let class = board.class_of(Some(&net));
        let drill = self.drill.unwrap_or(class.via_drill);
        let diameter = self.diameter.unwrap_or(class.via_diameter);
        if !(drill > 0.0 && diameter > drill) {
            return Err(bad("a via's diameter is larger than its drill"));
        }
        let group = self.group.unwrap_or_else(|| format!("stitch:{net}"));
        let removed = clear_group(tx, self.board, &group)?;
        let view = view_of(tx, self.board)?;
        // Region: the zone's fill (or outline), or the given outline, or the board.
        let zone_view = self.zone.and_then(|z| view.zones.iter().find(|v| v.id == z));
        let region: Vec<Vec<Vec<P>>> = if let Some(zv) = zone_view {
            zv.fill.clone()
        } else if self.outline.len() >= 3 {
            vec![vec![self.outline.clone()]]
        } else {
            vec![std::iter::once(view.outline.clone()).chain(view.cutouts.iter().cloned()).collect()]
        };
        let bb = region.iter().flatten().flatten().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
        if !bb[0].is_finite() {
            return Err(bad("the zone has no poured copper to stitch (fill it first)"));
        }
        // Vias go through every layer: kept from other nets on all of them.
        let n = view.layers.len();
        let m = mask(0, n - 1);
        // Removed group vias are still in the view's copper: leave them out.
        let alive: Vec<StableId> = crate::items(tx.world(), self.board).vias.iter().map(|(id, _)| *id).collect();
        let cu: Vec<Cu> = view.cu.iter().filter(|c| !matches!(&c.owner, crate::board::Owner::Via(id) if !alive.contains(id))).cloned().collect();
        let spot = Spot { view: &view, board: &board, cu: &cu };
        let r = diameter / 2.0;
        let mut placed: Vec<P> = Vec::new();
        let mut row = 0;
        let mut y = bb[1] + pitch / 2.0;
        while y <= bb[3] {
            let mut x = bb[0] + pitch / 2.0 + if self.stagger && row % 2 == 1 { pitch / 2.0 } else { 0.0 };
            while x <= bb[2] {
                let p = [x, y];
                let inside = region.iter().any(|island| geom::inside_area(p, island) && island.iter().all(|c| geom::dist_to_loop(p, c) >= r));
                if inside && !self.exclude.iter().any(|e| geom::dist(*e, p) < pitch / 2.0) && spot.free(p, r, drill, m, &net, &placed) {
                    placed.push(p);
                }
                x += pitch;
            }
            y += pitch;
            row += 1;
        }
        if placed.len() > 5000 {
            return Err(bad(format!("{} vias: use a larger pitch", placed.len())));
        }
        let mut ids = Vec::new();
        for p in &placed {
            ids.push(new_via(tx, self.board, &net, *p, drill, diameter, &group)?);
        }
        Ok(json!({ "board": self.board, "net": net, "group": group, "vias": ids.len(), "replaced": removed, "ids": ids }))
    }
}

/// `pcb.via.fence`: a row of vias of `net` (usually ground) beside a track on
/// each side — a via fence that shields an RF or fast line. Along `track` (or a
/// path of `points`) every `pitch` mm at `offset` from its centre line.
#[derive(Deserialize, JsonSchema)]
pub struct Fence {
    pub board: StableId,
    pub net: String,
    pub track: Option<StableId>,
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
    /// Along the path (mm), default 1.5.
    pub pitch: Option<f64>,
    /// From the centre line (mm); default half the track width + clearance + the via radius.
    pub offset: Option<f64>,
    /// "both" (default), "left" or "right" (seen walking from the first point).
    pub sides: Option<String>,
    pub drill: Option<f64>,
    pub diameter: Option<f64>,
    pub group: Option<String>,
}

impl Command for Fence {
    const NAME: &'static str = "pcb.via.fence";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        let (path, width, track_net) = match self.track {
            Some(t) => {
                let t = tx.get::<crate::Track>(t).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{t} is not a track")))?;
                (t.points, t.width, t.net)
            }
            None => (self.points.clone(), 0.0, None),
        };
        if path.len() < 2 {
            return Err(bad("give a track or a path of 2 or more points"));
        }
        let net = self.net.trim().to_string();
        let class = board.class_of(Some(&net));
        let drill = self.drill.unwrap_or(class.via_drill);
        let diameter = self.diameter.unwrap_or(class.via_diameter);
        let pitch = self.pitch.unwrap_or(1.5).max(diameter + board.rules.hole_to_hole.min(0.2));
        let clearance = board.class_of(track_net.as_deref()).clearance.max(class.clearance).max(board.rules.min_space);
        let offset = self.offset.unwrap_or(width / 2.0 + clearance + diameter / 2.0 + 0.05);
        let sides: Vec<f64> = match self.sides.as_deref().unwrap_or("both") {
            "left" => vec![1.0],
            "right" => vec![-1.0],
            "both" => vec![1.0, -1.0],
            s => return Err(bad(format!("sides is both, left or right, not “{s}”"))),
        };
        let group = self.group.unwrap_or_else(|| format!("fence:{}", track_net.as_deref().unwrap_or(&net)));
        let removed = clear_group(tx, self.board, &group)?;
        let view = view_of(tx, self.board)?;
        let alive: Vec<StableId> = crate::items(tx.world(), self.board).vias.iter().map(|(id, _)| *id).collect();
        let cu: Vec<Cu> = view.cu.iter().filter(|c| !matches!(&c.owner, crate::board::Owner::Via(id) if !alive.contains(id))).cloned().collect();
        let spot = Spot { view: &view, board: &board, cu: &cu };
        let n = view.layers.len();
        let mut placed: Vec<P> = Vec::new();
        let total: f64 = path.windows(2).map(|w| geom::dist(w[0], w[1])).sum();
        let mut s = 0.0;
        while s <= total + 1e-9 {
            // The point and direction at arc length s.
            let mut left = s;
            let mut at = path[0];
            let mut dir = [1.0, 0.0];
            for w in path.windows(2) {
                let l = geom::dist(w[0], w[1]);
                if l < 1e-9 {
                    continue;
                }
                dir = [(w[1][0] - w[0][0]) / l, (w[1][1] - w[0][1]) / l];
                if left <= l {
                    at = [w[0][0] + dir[0] * left, w[0][1] + dir[1] * left];
                    break;
                }
                left -= l;
                at = w[1];
            }
            for side in &sides {
                let p = [at[0] - dir[1] * offset * side, at[1] + dir[0] * offset * side];
                if spot.free(p, diameter / 2.0, drill, mask(0, n - 1), &net, &placed) {
                    placed.push(p);
                }
            }
            s += pitch;
        }
        let mut ids = Vec::new();
        for p in &placed {
            ids.push(new_via(tx, self.board, &net, *p, drill, diameter, &group)?);
        }
        Ok(json!({ "board": self.board, "net": net, "group": group, "vias": ids.len(), "replaced": removed, "ids": ids, "offset": offset, "pitch": pitch }))
    }
}

/// `pcb.via.update`: change vias (what is left out stays): place, size, net,
/// type and layer span, per-layer pads.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateVia {
    /// One via or several.
    pub targets: Vec<StableId>,
    pub at: Option<[f64; 2]>,
    pub drill: Option<f64>,
    pub diameter: Option<f64>,
    /// "" for no net.
    pub net: Option<String>,
    pub kind: Option<ViaKind>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub stack: Option<BTreeMap<String, f64>>,
}

impl Command for UpdateVia {
    const NAME: &'static str = "pcb.via.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        for id in &self.targets {
            let mut v = tx.get::<Via>(*id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a via")))?;
            let board = board_of(tx, v.board)?;
            if let Some(a) = self.at {
                crate::check_points(&[a], "via")?;
                v.at = a;
            }
            if let Some(d) = self.drill {
                v.drill = d;
            }
            if let Some(d) = self.diameter {
                v.diameter = d;
            }
            if self.net.is_some() {
                v.net = crate::clean(self.net.clone());
            }
            if let Some(k) = self.kind {
                v.spec.kind = k;
                if k == ViaKind::Through {
                    v.spec.from = None;
                    v.spec.to = None;
                }
            }
            if self.from.is_some() {
                v.spec.from = crate::clean(self.from.clone());
            }
            if self.to.is_some() {
                v.spec.to = crate::clean(self.to.clone());
            }
            if let Some(s) = &self.stack {
                v.spec.stack = s.iter().map(|(k, d)| (k.clone(), *d)).collect();
            }
            if !(v.drill > 0.0 && v.diameter > v.drill && v.diameter < 10.0) {
                return Err(bad("a via's diameter is larger than its drill"));
            }
            check_via(&board, &v).map_err(bad)?;
            v.auto = false;
            tx.set(*id, v)?;
        }
        Ok(json!({ "updated": self.targets.len() }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<UpdateVia>().command::<Stitch>().command::<Fence>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: usize) -> Vec<String> {
        let mut v = vec!["top".to_string()];
        v.extend(crate::COPPER_NAMES[1..n - 1].iter().map(|s| s.to_string()));
        v.push("bottom".into());
        v
    }

    #[test]
    fn spans_and_masks() {
        let l = names(6);
        let through = ViaSpec::default();
        assert_eq!(span(&l, &through), (0, 5));
        let blind = ViaSpec { kind: ViaKind::Blind, from: Some("top".into()), to: Some("in2".into()), ..Default::default() };
        assert_eq!(span(&l, &blind), (0, 2));
        assert_eq!(mask(0, 2), 0b111);
        let micro = ViaSpec { kind: ViaKind::Micro, ..Default::default() };
        assert_eq!(span(&l, &micro), (0, 1), "a micro via defaults to top → in1");
        let buried = ViaSpec { kind: ViaKind::Buried, from: Some("in4".into()), to: Some("in1".into()), ..Default::default() };
        assert_eq!(span(&l, &buried), (1, 4), "either order");
    }
}
