//! The autorouter: nets one at a time on a fine grid over every copper layer,
//! A* with costs for length, bends and vias, then rip-up and reroute.
//!
//! A cell is open to a net when a track of the net's width centred there
//! keeps the net class's clearance (plus a small margin for the stretch
//! between cell centres) from every other net's copper and the board edge, or
//! lies wholly inside one of the net's own pads. Near the net's pads a track
//! may neck down to the class's `neck_width` (fine-pitch pads a full-width
//! track cannot leave), at a small cost so full width is used wherever it
//! fits. Vias go where their pad keeps clear on every layer.
//!
//! When a net finds no way through, the router looks again treating copper it
//! laid itself for other nets as passable at a high cost; the nets in the way
//! are ripped up, the blocked net is routed, and the ripped nets go back in
//! the queue (each a bounded number of times). Whole passes in a different
//! order follow if anything is still left.
//!
//! What it makes is ordinary tracks and vias (marked `auto`), checked by the
//! same design-rule check as hand-made copper; nets it cannot finish stay in
//! the ratsnest and are reported with a reason.

use crate::board::{self, BoardView, PadView};
use crate::footprints::PadKind;
use crate::geom::{self, P, Prim};
use crate::{Board, NetClass, Track, Via};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashSet, VecDeque};

/// `pcb.autoroute`: route the board's unrouted nets (or the ones named).
#[derive(Deserialize, JsonSchema)]
pub struct Autoroute {
    pub board: StableId,
    /// Nets to route; all unrouted nets when left out.
    pub nets: Option<Vec<String>>,
    /// Nets to leave alone (a ground left to a pour).
    #[serde(default)]
    pub skip: Vec<String>,
    /// Grid pitch (mm), default 0.25; finer finds more room, slower.
    pub grid: Option<f64>,
    /// Remove earlier autorouted copper of these nets first (default true); dragged tracks
    /// (`reroute`) of these nets that now break a rule go too.
    pub replace: Option<bool>,
    /// Layers to use (default all).
    pub layers: Option<Vec<String>>,
    /// Rip up and reroute nets in the way of one that cannot finish (default true).
    pub rip_up: Option<bool>,
    /// What a via costs, in millimetres of track (default 5).
    pub via_cost: Option<f64>,
}

#[derive(Clone, Copy, PartialEq)]
struct State {
    cost: f64,
    node: usize,
}
impl Eq for State {}
impl Ord for State {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost).then(self.node.cmp(&o.node))
    }
}
impl PartialOrd for State {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// How far inside a rounded convex a point is (negative outside).
fn depth(prim: &Prim, p: P) -> f64 {
    let hull = geom::convex_hull(&prim.pts);
    if hull.len() >= 3 && geom::in_convex(p, &hull) {
        prim.r + geom::dist_to_loop(p, &hull)
    } else {
        let d = match hull.len() {
            1 => geom::dist(p, hull[0]),
            2 => geom::point_seg_dist(p, hull[0], hull[1]),
            _ => geom::dist_to_loop(p, &hull),
        };
        prim.r - d
    }
}

const DIRS: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
const MARGIN: f64 = 0.01;
/// How often one net may be ripped up for others in one pass.
const MAX_RIPS: usize = 3;

struct Grid {
    x0: f64,
    y0: f64,
    g: f64,
    nx: usize,
    ny: usize,
}

impl Grid {
    fn at(&self, i: usize, j: usize) -> P {
        [self.x0 + i as f64 * self.g, self.y0 + j as f64 * self.g]
    }
    fn of(&self, k: usize) -> P {
        self.at(k % self.nx, k / self.nx)
    }
    fn range(&self, b: [f64; 4]) -> (usize, usize, usize, usize) {
        let i0 = ((b[0] - self.x0) / self.g).floor().max(0.0) as usize;
        let j0 = ((b[1] - self.y0) / self.g).floor().max(0.0) as usize;
        let i1 = (((b[2] - self.x0) / self.g).ceil().max(0.0) as usize).min(self.nx - 1);
        let j1 = (((b[3] - self.y0) / self.g).ceil().max(0.0) as usize).min(self.ny - 1);
        (i0, j0, i1, j1)
    }
}

/// Copper that is fixed while one net routes.
#[derive(Clone)]
struct Obstacle {
    prim: Prim,
    layers: u8,
    net: Option<String>,
}

/// One net's view of the board: what each cell costs (infinite: closed) and
/// whether a full-width track fits there.
struct Maps {
    cost: Vec<Vec<f64>>,
    wide: Vec<Vec<bool>>,
    via_ok: Vec<bool>,
    /// Via spots only blocked by copper that could be ripped up.
    via_soft: Vec<bool>,
    soft: Vec<Vec<bool>>,
}

/// The fixed context of one autoroute run.
struct Ctx<'a> {
    board: &'a Board,
    view: &'a BoardView,
    grid: Grid,
    nl: usize,
    ncell: usize,
    use_layers: Vec<usize>,
    edge_gap: Vec<f64>,
    holes: Vec<Prim>,
    via_cost: f64,
}

/// A net's widths: full, and necked near its pads.
struct Widths {
    w: f64,
    neck: f64,
    clear: f64,
    via_r: f64,
    class: NetClass,
}

fn widths(board: &Board, net: &str) -> Widths {
    let class = board.class_of(Some(net));
    let w = class.track_width;
    let neck = class.neck_width.unwrap_or(w * 0.6).max(board.rules.min_track).min(w);
    Widths { w, neck, clear: class.clearance.max(board.rules.min_space), via_r: class.via_diameter / 2.0, class }
}

impl Ctx<'_> {
    /// Cells near the net's pads, where a track may neck down.
    fn neck_zone(&self, pads: &[&PadView], wd: &Widths) -> Vec<bool> {
        let mut z = vec![false; self.ncell];
        if wd.neck >= wd.w - 1e-9 {
            return z;
        }
        for p in pads {
            let b = p.prim.bbox();
            let r = 1.2;
            let (i0, j0, i1, j1) = self.grid.range([b[0] - r, b[1] - r, b[2] + r, b[3] + r]);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    if depth(&p.prim, self.grid.at(i, j)) > -r {
                        z[j * self.grid.nx + i] = true;
                    }
                }
            }
        }
        z
    }

    /// What each cell costs `net`, with `hard` copper closing cells and `soft`
    /// copper (another net's autorouted copper) closing them too, unless `rip`
    /// is set: then it only costs.
    fn maps(&self, net: &str, wd: &Widths, neck: &[bool], hard: &[&Obstacle], soft: &[&Obstacle], rip: bool) -> Maps {
        let (nl, ncell, g) = (self.nl, self.ncell, self.grid.g);
        const HARD_W: u8 = 1;
        const SOFT_W: u8 = 2;
        const HARD_N: u8 = 4;
        const SOFT_N: u8 = 8;
        let mut block = vec![vec![0u8; ncell]; nl];
        let mut inside_w = vec![vec![false; ncell]; nl];
        let mut inside_n = vec![vec![false; ncell]; nl];
        let mut via_hard = vec![false; ncell];
        let mut via_softb = vec![false; ncell];
        let rules = &self.board.rules;
        for (is_soft, list) in [(false, hard), (true, soft)] {
            for o in list.iter() {
                let mine = o.net.as_deref() == Some(net);
                let b = o.prim.bbox();
                let oc = self.board.class_of(o.net.as_deref()).clearance;
                let need = wd.clear.max(oc) + MARGIN;
                let reach = need + wd.w.max(2.0 * wd.via_r) / 2.0 + g;
                let (i0, j0, i1, j1) = self.grid.range([b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach]);
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let k = j * self.grid.nx + i;
                        let d = depth(&o.prim, self.grid.at(i, j));
                        if mine {
                            for &l in &self.use_layers {
                                if o.layers & (1 << l) != 0 {
                                    if d >= wd.w / 2.0 - 1e-9 {
                                        inside_w[l][k] = true;
                                    }
                                    if d >= wd.neck / 2.0 - 1e-9 {
                                        inside_n[l][k] = true;
                                    }
                                }
                            }
                            // No via in a pad.
                            if d > -wd.via_r {
                                via_hard[k] = true;
                            }
                            continue;
                        }
                        // Gap from a disc of radius w/2 at p to the obstacle is −d − w/2.
                        let bw = -d - wd.w / 2.0 < need;
                        let bn = -d - wd.neck / 2.0 < need;
                        for &l in &self.use_layers {
                            if o.layers & (1 << l) != 0 {
                                if bw {
                                    block[l][k] |= if is_soft { SOFT_W } else { HARD_W };
                                }
                                if bn {
                                    block[l][k] |= if is_soft { SOFT_N } else { HARD_N };
                                }
                            }
                        }
                        if -d - wd.via_r < need {
                            if is_soft { via_softb[k] = true } else { via_hard[k] = true }
                        }
                    }
                }
            }
        }
        for h in &self.holes {
            let b = h.bbox();
            let reach = rules.hole_to_hole + wd.via_r + g;
            let (i0, j0, i1, j1) = self.grid.range([b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach]);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    if -depth(h, self.grid.at(i, j)) - wd.class.via_drill / 2.0 < rules.hole_to_hole {
                        via_hard[j * self.grid.nx + i] = true;
                    }
                }
            }
        }
        let neck_cost = 0.6 * g;
        let soft_cost = 40.0 * g;
        let mut cost = vec![vec![f64::INFINITY; ncell]; nl];
        let mut wide = vec![vec![false; ncell]; nl];
        let mut softm = vec![vec![false; ncell]; nl];
        for &l in &self.use_layers {
            for k in 0..ncell {
                let e = self.edge_gap[k] - rules.edge_clearance;
                let edge_w = e >= wd.w / 2.0;
                let edge_n = e >= wd.neck / 2.0;
                let bl = block[l][k];
                if inside_w[l][k] || (edge_w && bl & (HARD_W | SOFT_W) == 0) {
                    cost[l][k] = 0.0;
                    wide[l][k] = true;
                } else if neck[k] && (inside_n[l][k] || (edge_n && bl & (HARD_N | SOFT_N) == 0)) {
                    cost[l][k] = neck_cost;
                } else if rip && edge_w && bl & HARD_W == 0 {
                    cost[l][k] = soft_cost;
                    wide[l][k] = true;
                    softm[l][k] = true;
                } else if rip && neck[k] && edge_n && bl & HARD_N == 0 {
                    cost[l][k] = soft_cost + neck_cost;
                    softm[l][k] = true;
                }
            }
        }
        let edge_v: Vec<bool> = (0..ncell).map(|k| self.edge_gap[k] >= rules.edge_clearance + wd.via_r).collect();
        let usable = self.use_layers.len() >= 2;
        let via_ok: Vec<bool> = (0..ncell).map(|k| usable && edge_v[k] && !via_hard[k] && (!via_softb[k] || rip)).collect();
        let via_soft: Vec<bool> = (0..ncell).map(|k| via_softb[k] && !via_hard[k]).collect();
        Maps { cost, wide, via_ok, via_soft, soft: softm }
    }

    /// A* from any connected node to any target node. Nodes are layer·ncell + cell;
    /// the direction last moved is part of the state so bends cost.
    fn search(&self, m: &Maps, source_list: &[usize], targets: &BTreeMap<usize, usize>, goals: &[P]) -> Option<Vec<usize>> {
        let (grid, nl, ncell) = (&self.grid, self.nl, self.ncell);
        let g = grid.g;
        let sources: HashSet<usize> = source_list.iter().copied().collect();
        let nd = 9; // 8 directions + "arrived by via / start"
        let total = nl * ncell * nd;
        let mut best = vec![f64::INFINITY; total];
        let mut prev = vec![usize::MAX; total];
        let mut heap = BinaryHeap::new();
        let h = |node: usize| -> f64 {
            let p = grid.of(node % ncell);
            goals.iter().map(|q| {
                let (dx, dy) = ((p[0] - q[0]).abs(), (p[1] - q[1]).abs());
                dx.max(dy) + (std::f64::consts::SQRT_2 - 1.0) * dx.min(dy)
            }).fold(f64::INFINITY, f64::min) * 0.95
        };
        for &s in source_list {
            let st = s * nd + 8;
            best[st] = 0.0;
            heap.push(State { cost: h(s), node: st });
        }
        let mut found = None;
        let mut pops = 0usize;
        while let Some(State { cost, node }) = heap.pop() {
            pops += 1;
            if pops > 8_000_000 {
                return None;
            }
            let cell_node = node / nd;
            let dir = node % nd;
            let here = best[node];
            if cost - h(cell_node) > here + 1e-9 {
                continue; // stale entry
            }
            if targets.contains_key(&cell_node) && !sources.contains(&cell_node) {
                found = Some(node);
                break;
            }
            let l = cell_node / ncell;
            let k = cell_node % ncell;
            let (i, j) = ((k % grid.nx) as i32, (k / grid.nx) as i32);
            for (d, (di, dj)) in DIRS.iter().enumerate() {
                let (ni, nj) = (i + di, j + dj);
                if ni < 0 || nj < 0 || ni >= grid.nx as i32 || nj >= grid.ny as i32 {
                    continue;
                }
                let nk = nj as usize * grid.nx + ni as usize;
                let extra = m.cost[l][nk];
                if !extra.is_finite() {
                    continue;
                }
                // A diagonal step must not cut a closed corner.
                if *di != 0 && *dj != 0 && (!m.cost[l][j as usize * grid.nx + ni as usize].is_finite() || !m.cost[l][nj as usize * grid.nx + i as usize].is_finite()) {
                    continue;
                }
                let step = if *di != 0 && *dj != 0 { std::f64::consts::SQRT_2 * g } else { g };
                let turn = if dir == 8 || dir == d {
                    0.0
                } else {
                    let a = (dir as i32 - d as i32).rem_euclid(8);
                    match a.min(8 - a) {
                        1 => 0.8 * g,
                        2 => 2.5 * g,
                        _ => 10.0 * g,
                    }
                };
                // A light preference: top runs across, bottom runs along (crossings need fewer vias).
                let lean = if (l == 0 && *dj != 0 && *di == 0) || (l == nl - 1 && *di != 0 && *dj == 0) { 0.15 * step } else { 0.0 };
                let nn = (l * ncell + nk) * nd + d;
                let c = here + step + turn + lean + extra;
                if c < best[nn] {
                    best[nn] = c;
                    prev[nn] = node;
                    heap.push(State { cost: c + h(l * ncell + nk), node: nn });
                }
            }
            if m.via_ok[k] {
                let vc = self.via_cost + if m.via_soft[k] { 40.0 * g } else { 0.0 };
                for l2 in 0..nl {
                    if l2 == l || !m.cost[l2][k].is_finite() {
                        continue;
                    }
                    let nn = (l2 * ncell + k) * nd + 8;
                    let c = here + vc + m.cost[l2][k];
                    if c < best[nn] {
                        best[nn] = c;
                        prev[nn] = node;
                        heap.push(State { cost: c + h(l2 * ncell + k), node: nn });
                    }
                }
            }
        }
        let mut node = found?;
        let mut path = vec![node / nd];
        while prev[node] != usize::MAX {
            node = prev[node];
            let c = node / nd;
            if *path.last().unwrap() != c {
                path.push(c);
            }
        }
        path.reverse();
        Some(path)
    }
}

/// What routing one net produced.
struct NetCopper {
    obstacles: Vec<Obstacle>,
    created: Vec<StableId>,
    tracks: usize,
    vias: usize,
}

enum Outcome {
    Routed(NetCopper),
    /// Partly routed (copper kept), with why the rest failed and, when ripping up
    /// would help, the nets in the way.
    Failed { copper: NetCopper, reason: String, blockers: BTreeSet<String> },
}

impl Command for Autoroute {
    const NAME: &'static str = "pcb.autoroute";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = tx.get::<Board>(self.board).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a board", self.board)))?;
        let g = self.grid.unwrap_or(0.25);
        if !(0.05..=2.0).contains(&g) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "grid is 0.05 to 2 mm"));
        }
        let via_cost = self.via_cost.unwrap_or(5.0);
        if !(0.0..=100.0).contains(&via_cost) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "via_cost is 0 to 100 mm"));
        }
        let wanted: Option<HashSet<String>> = self.nets.as_ref().map(|v| v.iter().cloned().collect());
        let skip: HashSet<String> = self.skip.iter().cloned().collect();
        let pick = |n: &str| wanted.as_ref().is_none_or(|w| w.contains(n)) && !skip.contains(n);
        if self.replace.unwrap_or(true) {
            let it = crate::items(tx.world(), self.board);
            let before = board::resolve(tx.world(), self.board, &board).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
            let broken: HashSet<StableId> = before.violations.iter().filter(|x| x.severity == "error" && x.code != "unrouted").flat_map(|x| x.objects.iter().copied()).collect();
            let gone: Vec<StableId> = it
                .tracks
                .iter()
                .filter(|(id, t)| t.net.as_deref().is_some_and(pick) && (t.auto || (t.reroute && broken.contains(id))))
                .map(|(id, _)| *id)
                .chain(it.vias.iter().filter(|(_, v)| v.auto && v.net.as_deref().is_some_and(pick)).map(|(id, _)| *id))
                .collect();
            for id in gone {
                tx.remove_dependency(self.board, id)?;
                tx.delete(id)?;
            }
        }
        let view = board::resolve(tx.world(), self.board, &board).map_err(|e| CommandError::new(ErrorCode::InvalidGeometry, e))?;
        let all_layers = view.layers.clone();
        let use_layers: Vec<usize> = match &self.layers {
            Some(ls) => ls.iter().filter_map(|l| board::layer_index(&all_layers, l)).collect(),
            None => (0..all_layers.len()).collect(),
        };
        if use_layers.is_empty() {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "no usable layers"));
        }
        let todo: Vec<String> = {
            let mut v: Vec<(f64, f64, String)> = view
                .nets
                .iter()
                .filter(|n| !n.routed && pick(&n.name))
                .map(|n| (-board.class_of(Some(&n.name)).track_width, spread(&view, &n.name), n.name.clone()))
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            v.into_iter().map(|x| x.2).collect()
        };
        let x0 = (view.bounds[0] / g).floor() * g;
        let y0 = (view.bounds[1] / g).floor() * g;
        let grid = Grid { x0, y0, g, nx: ((view.bounds[2] - x0) / g).ceil() as usize + 1, ny: ((view.bounds[3] - y0) / g).ceil() as usize + 1 };
        if grid.nx * grid.ny > 4_000_000 {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "the board is too large for this grid: use a coarser grid"));
        }
        let nl = all_layers.len();
        let mut base: Vec<Obstacle> = Vec::new();
        for p in &view.pads {
            let mut mask = 0u8;
            for l in &p.layers {
                if let Some(k) = board::layer_index(&all_layers, l) {
                    mask |= 1 << k;
                }
            }
            if mask != 0 {
                base.push(Obstacle { prim: p.prim.clone(), layers: mask, net: p.net.clone() });
            }
        }
        for t in &view.tracks {
            if let Some(k) = board::layer_index(&all_layers, &t.layer) {
                for w in t.points.windows(2) {
                    base.push(Obstacle { prim: Prim::capsule(w[0], w[1], t.width / 2.0), layers: 1 << k, net: t.net.clone() });
                }
            }
        }
        for v in &view.vias {
            base.push(Obstacle { prim: Prim::circle(v.at, v.diameter / 2.0), layers: ((1u16 << nl) - 1) as u8, net: v.net.clone() });
        }
        // Pours are refilled around new copper; they do not block.
        let holes: Vec<Prim> = view.holes.iter().map(|h| Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0)).collect();
        let edges: Vec<Vec<P>> = std::iter::once(view.outline.clone()).chain(view.cutouts.iter().cloned()).collect();
        let ncell = grid.nx * grid.ny;
        let mut edge_gap = vec![f64::NEG_INFINITY; ncell];
        for j in 0..grid.ny {
            for i in 0..grid.nx {
                let p = grid.at(i, j);
                if geom::in_polygon(p, &view.outline) && !view.cutouts.iter().any(|c| geom::in_polygon(p, c)) {
                    edge_gap[j * grid.nx + i] = edges.iter().map(|e| geom::dist_to_loop(p, e)).fold(f64::INFINITY, f64::min);
                }
            }
        }
        let ctx = Ctx { board: &board, view: &view, grid, nl, ncell, use_layers, edge_gap, holes, via_cost };
        let rip_up = self.rip_up.unwrap_or(true);

        // Passes: within a pass, rip-up and reroute; between passes the nets that
        // still failed go first. The best pass is kept.
        const PASSES: usize = 4;
        let mut order = todo;
        let mut best: Option<(usize, Vec<String>)> = None;
        let mut pass = 0;
        let mut ripped_total = 0;
        let (routed, failed, made_tracks, made_vias) = loop {
            pass += 1;
            let mut copper: BTreeMap<String, NetCopper> = BTreeMap::new();
            let mut rips: BTreeMap<String, usize> = BTreeMap::new();
            let mut failed: BTreeMap<String, String> = BTreeMap::new();
            let mut queue: VecDeque<String> = order.iter().cloned().collect();
            let mut budget = 6 * order.len().max(1);
            while let Some(net) = queue.pop_front() {
                failed.remove(&net);
                let outcome = route_net(tx, self.board, &ctx, &net, &base, &copper, false)?;
                match outcome {
                    Outcome::Routed(c) => {
                        copper.insert(net, c);
                    }
                    Outcome::Failed { copper: partial, reason, .. } => {
                        // Look again with other nets' autorouted copper passable: who is in the way?
                        let blockers = if rip_up && budget > 0 {
                            remove(tx, self.board, &partial)?;
                            match route_net(tx, self.board, &ctx, &net, &base, &copper, true)? {
                                Outcome::Failed { copper: probe, blockers, .. } => {
                                    remove(tx, self.board, &probe)?;
                                    blockers.into_iter().filter(|b| rips.get(b).copied().unwrap_or(0) < MAX_RIPS && b != &net).collect::<Vec<_>>()
                                }
                                Outcome::Routed(probe) => {
                                    remove(tx, self.board, &probe)?;
                                    vec![]
                                }
                            }
                        } else {
                            vec![]
                        };
                        if blockers.is_empty() {
                            if rip_up && budget > 0 {
                                // The probe removed the partial copper: lay it again.
                                let (Outcome::Routed(c) | Outcome::Failed { copper: c, .. }) = route_net(tx, self.board, &ctx, &net, &base, &copper, false)?;
                                copper.insert(net.clone(), c);
                            } else {
                                copper.insert(net.clone(), partial);
                            }
                            failed.insert(net, reason);
                            continue;
                        }
                        budget = budget.saturating_sub(1);
                        for b in &blockers {
                            if let Some(c) = copper.remove(b) {
                                remove(tx, self.board, &c)?;
                            }
                            *rips.entry(b.clone()).or_default() += 1;
                            ripped_total += 1;
                        }
                        match route_net(tx, self.board, &ctx, &net, &base, &copper, false)? {
                            Outcome::Routed(c) => {
                                copper.insert(net.clone(), c);
                            }
                            Outcome::Failed { copper: c, reason, .. } => {
                                copper.insert(net.clone(), c);
                                failed.insert(net.clone(), reason);
                            }
                        }
                        for b in blockers {
                            if !queue.contains(&b) {
                                queue.push_back(b);
                            }
                        }
                    }
                }
            }
            let routed: Vec<String> = order.iter().filter(|n| copper.contains_key(*n) && !failed.contains_key(*n)).cloned().collect();
            let made_tracks: usize = copper.values().map(|c| c.tracks).sum();
            let made_vias: usize = copper.values().map(|c| c.vias).sum();
            let failed_list: Vec<Value> = failed.iter().map(|(n, r)| json!({ "net": n, "reason": r })).collect();
            if failed.is_empty() {
                break (routed, failed_list, made_tracks, made_vias);
            }
            if best.as_ref().is_none_or(|(n, _)| failed.len() < *n) {
                best = Some((failed.len(), order.clone()));
            }
            let best_order = best.as_ref().map(|b| b.1.clone()).unwrap();
            if pass >= PASSES && order == best_order {
                break (routed, failed_list, made_tracks, made_vias);
            }
            for c in copper.values() {
                remove(tx, self.board, c)?;
            }
            if pass >= PASSES {
                order = best_order;
            } else {
                let bad: Vec<String> = failed.keys().cloned().collect();
                let mut next = bad.clone();
                next.extend(order.iter().filter(|n| !bad.contains(n)).cloned());
                order = next;
            }
        };
        Ok(json!({ "board": self.board, "routed": routed, "failed": failed, "tracks": made_tracks, "vias": made_vias, "grid": g, "passes": pass, "ripped": ripped_total }))
    }
}

fn remove(tx: &mut Tx, board: StableId, c: &NetCopper) -> Result<(), CommandError> {
    for id in &c.created {
        if tx.get::<Track>(*id).is_some() || tx.get::<Via>(*id).is_some() {
            tx.remove_dependency(board, *id)?;
            tx.delete(*id)?;
        }
    }
    Ok(())
}

/// Routes one net against the fixed copper and the other nets' autorouted
/// copper. With `rip`, other nets' autorouted copper is passable at a cost:
/// nothing is laid, and the nets crossed come back as blockers.
fn route_net(tx: &mut Tx, board_id: StableId, ctx: &Ctx, net: &str, base: &[Obstacle], copper: &BTreeMap<String, NetCopper>, rip: bool) -> Result<Outcome, CommandError> {
    let view = ctx.view;
    let wd = widths(ctx.board, net);
    let pads: Vec<&PadView> = view.pads.iter().filter(|p| p.net.as_deref() == Some(net) && p.kind != PadKind::Npth).collect();
    let neck = ctx.neck_zone(&pads, &wd);
    let mut out = NetCopper { obstacles: vec![], created: vec![], tracks: 0, vias: 0 };
    let soft: Vec<&Obstacle> = copper.iter().filter(|(n, _)| n.as_str() != net).flat_map(|(_, c)| c.obstacles.iter()).collect();
    let ncell = ctx.ncell;
    let all_layers = &view.layers;
    // The net's pads, each with the cells a track can start or end in.
    let mut pad_cells: Vec<Vec<usize>> = Vec::new();
    for p in &pads {
        let mut cells = Vec::new();
        let (i0, j0, i1, j1) = ctx.grid.range(p.prim.bbox());
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = j * ctx.grid.nx + i;
                let d = depth(&p.prim, ctx.grid.at(i, j));
                for &l in &ctx.use_layers {
                    let on = p.layers.iter().any(|x| board::layer_index(all_layers, x) == Some(l));
                    if on && (d >= wd.w / 2.0 - 1e-9 || (neck[k] && d >= wd.neck / 2.0 - 1e-9)) {
                        cells.push(l * ncell + k);
                    }
                }
            }
        }
        pad_cells.push(cells);
    }
    // Pads off the grid (no grid point fits a track inside them) are reached by a
    // short stub from a nearby open cell straight to the pad's centre, checked exactly.
    let mut access: BTreeMap<usize, (usize, f64)> = BTreeMap::new();
    let edges: Vec<&Vec<P>> = std::iter::once(&view.outline).chain(view.cutouts.iter()).collect();
    for (pi, p) in pads.iter().enumerate() {
        if !pad_cells[pi].is_empty() {
            continue;
        }
        let reach = 1.0;
        let b = p.prim.bbox();
        let (i0, j0, i1, j1) = ctx.grid.range([b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach]);
        let mut found: Vec<(f64, usize, f64)> = Vec::new();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = j * ctx.grid.nx + i;
                let q = ctx.grid.at(i, j);
                if depth(&p.prim, q) < -reach {
                    continue;
                }
                for &l in &ctx.use_layers {
                    if !p.layers.iter().any(|x| board::layer_index(all_layers, x) == Some(l)) {
                        continue;
                    }
                    let widths: &[f64] = if neck[k] { &[wd.w, wd.neck] } else { &[wd.w] };
                    for &sw in widths {
                        let stub = Prim::capsule(q, p.at, sw / 2.0);
                        let clear_edge = edges.iter().all(|e| geom::gap_to_loop(&stub, e) >= ctx.board.rules.edge_clearance + MARGIN);
                        let clear = clear_edge
                            && base.iter().chain(soft.iter().copied()).all(|o| {
                                o.net.as_deref() == Some(net) || o.layers & (1 << l) == 0 || geom::gap(&stub, &o.prim) >= wd.clear.max(ctx.board.class_of(o.net.as_deref()).clearance) + MARGIN
                            });
                        if clear {
                            found.push((geom::dist(q, p.at), l * ncell + k, sw));
                            break;
                        }
                    }
                }
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, node, sw) in found.into_iter().take(24) {
            pad_cells[pi].push(node);
            access.insert(node, (pi, sw));
        }
    }
    if pad_cells.iter().any(|c| c.is_empty()) {
        let small: Vec<String> = pads.iter().zip(&pad_cells).filter(|(_, c)| c.is_empty()).map(|(p, _)| format!("{}.{}", p.reference, p.number)).collect();
        return Ok(Outcome::Failed { copper: out, reason: format!("no grid point fits a {} mm track inside {} (try a finer grid or a narrower neck_width)", wd.neck, small.join(", ")), blockers: BTreeSet::new() });
    }
    if pads.len() < 2 {
        return Ok(Outcome::Routed(out));
    }
    let mut connected: Vec<usize> = pad_cells[0].clone();
    let mut reached: Vec<bool> = pad_cells.iter().map(|cells| cells.iter().any(|c| connected.contains(c))).collect();
    let mut blockers = BTreeSet::new();
    while reached.iter().any(|r| !r) {
        let mut hard: Vec<&Obstacle> = base.iter().collect();
        hard.extend(out.obstacles.iter());
        let m = ctx.maps(net, &wd, &neck, &hard, &soft, rip);
        let mut target: BTreeMap<usize, usize> = BTreeMap::new();
        for (pi, cells) in pad_cells.iter().enumerate() {
            if !reached[pi] {
                for c in cells {
                    target.insert(*c, pi);
                }
            }
        }
        let goals: Vec<P> = pads.iter().enumerate().filter(|(i, _)| !reached[*i]).map(|(_, p)| p.at).collect();
        let Some(path) = ctx.search(&m, &connected, &target, &goals) else {
            let missing: Vec<String> = pads.iter().zip(&reached).filter(|(_, r)| !**r).map(|(p, _)| format!("{}.{}", p.reference, p.number)).collect();
            return Ok(Outcome::Failed { copper: out, reason: format!("no path to {}", missing.join(", ")), blockers });
        };
        if rip {
            // Who owns the soft copper the path crosses (cells and via spots)?
            let mut probe: Vec<(P, f64, u8)> = Vec::new();
            for (idx, n) in path.iter().enumerate() {
                let (l, k) = (n / ncell, n % ncell);
                if m.soft[l][k] {
                    probe.push((ctx.grid.of(k), if m.wide[l][k] { wd.w / 2.0 } else { wd.neck / 2.0 }, 1 << l));
                }
                if idx > 0 && path[idx - 1] / ncell != l && m.via_soft[k] {
                    probe.push((ctx.grid.of(k), wd.via_r, 0xff));
                }
            }
            for (other, c) in copper.iter().filter(|(n, _)| n.as_str() != net) {
                let need = wd.clear.max(ctx.board.class_of(Some(other)).clearance) + MARGIN;
                if c.obstacles.iter().any(|o| probe.iter().any(|(p, r, layers)| o.layers & layers != 0 && -depth(&o.prim, *p) - r < need)) {
                    blockers.insert(other.clone());
                }
            }
            let end = *path.last().unwrap();
            for n in &path {
                if let Some(pi) = target.get(n) {
                    reached[*pi] = true;
                }
            }
            reached[target[&end]] = true;
            connected.extend(path.iter().copied());
            continue;
        }
        let end = *path.last().unwrap();
        for n in &path {
            if let Some(pi) = target.get(n) {
                reached[*pi] = true;
            }
        }
        reached[target[&end]] = true;
        // Stubs into off-grid pads at either end.
        for (cell, into) in [(path[0], false), (end, true)] {
            if let Some((pi, sw)) = access.get(&cell) {
                if into && target.get(&cell) != Some(pi) {
                    continue;
                }
                let (l, k) = (cell / ncell, cell % ncell);
                emit_track(tx, board_id, net, &all_layers[l], *sw, &[pads[*pi].at, ctx.grid.of(k)], l, &mut out)?;
            }
        }
        // Copper: runs on one layer at one width become tracks; layer changes become vias.
        let mut run: Vec<P> = Vec::new();
        let mut run_layer = path[0] / ncell;
        let mut run_wide = m.wide[run_layer][path[0] % ncell];
        for (idx, n) in path.iter().enumerate() {
            let (l, k) = (n / ncell, n % ncell);
            let p = ctx.grid.of(k);
            if l != run_layer {
                if run.len() >= 2 {
                    emit_track(tx, board_id, net, &all_layers[run_layer], if run_wide { wd.w } else { wd.neck }, &run, run_layer, &mut out)?;
                }
                emit_via(tx, board_id, net, p, wd.class.via_drill, wd.class.via_diameter, ctx.nl, &mut out)?;
                run = vec![p];
                run_layer = l;
                run_wide = m.wide[l][k];
                continue;
            }
            // A segment is full width only when both its ends are.
            let w_here = m.wide[l][k];
            if idx > 0 && w_here != run_wide && run.len() >= 1 {
                if run_wide && !w_here {
                    // Narrowing: the full-width run ends at the last wide cell; the neck starts there.
                    if run.len() >= 2 {
                        emit_track(tx, board_id, net, &all_layers[run_layer], wd.w, &run, run_layer, &mut out)?;
                    }
                    run = vec![*run.last().unwrap(), p];
                    run_wide = false;
                } else {
                    // Widening: this segment is still necked; the wide run starts here.
                    run.push(p);
                    emit_track(tx, board_id, net, &all_layers[run_layer], wd.neck, &run, run_layer, &mut out)?;
                    run = vec![p];
                    run_wide = true;
                }
                continue;
            }
            run.push(p);
        }
        if run.len() >= 2 {
            emit_track(tx, board_id, net, &all_layers[run_layer], if run_wide { wd.w } else { wd.neck }, &run, run_layer, &mut out)?;
        }
        connected.extend(path.iter().copied());
        for (pi, cells) in pad_cells.iter().enumerate() {
            if reached[pi] {
                connected.extend(cells.iter().copied());
            }
        }
        connected.sort_unstable();
        connected.dedup();
        for (pi, cells) in pad_cells.iter().enumerate() {
            if !reached[pi] && cells.iter().any(|c| connected.binary_search(c).is_ok()) {
                reached[pi] = true;
            }
        }
    }
    if rip {
        return Ok(Outcome::Failed { copper: out, reason: String::new(), blockers });
    }
    Ok(Outcome::Routed(out))
}

/// How spread out a net's pads are (for routing short nets first).
fn spread(view: &BoardView, net: &str) -> f64 {
    let pts: Vec<P> = view.pads.iter().filter(|p| p.net.as_deref() == Some(net)).map(|p| p.at).collect();
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in &pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    if pts.is_empty() { 0.0 } else { (b[2] - b[0]) + (b[3] - b[1]) }
}

#[allow(clippy::too_many_arguments)]
fn emit_track(tx: &mut Tx, board: StableId, net: &str, layer: &str, w: f64, run: &[P], li: usize, out: &mut NetCopper) -> Result<(), CommandError> {
    let points = crate::drag::tidy(run);
    if points.len() < 2 {
        return Ok(());
    }
    for s in points.windows(2) {
        out.obstacles.push(Obstacle { prim: Prim::capsule(s[0], s[1], w / 2.0), layers: 1 << li, net: Some(net.to_string()) });
    }
    let id = tx.create();
    tx.set(id, Track { board, net: Some(net.to_string()), layer: layer.to_string(), width: w, points, auto: true, reroute: false })?;
    tx.add_dependency(board, id)?;
    out.created.push(id);
    out.tracks += 1;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_via(tx: &mut Tx, board: StableId, net: &str, at: P, drill: f64, diameter: f64, nl: usize, out: &mut NetCopper) -> Result<(), CommandError> {
    out.obstacles.push(Obstacle { prim: Prim::circle(at, diameter / 2.0), layers: ((1u16 << nl) - 1) as u8, net: Some(net.to_string()) });
    let id = tx.create();
    tx.set(id, Via { board, net: Some(net.to_string()), at, drill, diameter, auto: true })?;
    tx.add_dependency(board, id)?;
    out.created.push(id);
    out.vias += 1;
    Ok(())
}
