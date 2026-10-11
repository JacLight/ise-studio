//! A simple autorouter: nets one at a time on a fine grid over every copper
//! layer, A* with costs for length, bends and vias. A cell is open to a net
//! when a track of the net's width centred there keeps the net class's
//! clearance (plus a small margin for the stretch between cell centres) from
//! every other net's copper and the board edge, or lies wholly inside one of
//! the net's own pads. Vias go where their pad keeps clear on every layer.
//! What it makes is ordinary tracks and vias (marked `auto`), checked by the
//! same design-rule check as hand-made copper; nets it cannot finish stay in
//! the ratsnest and are reported.

use crate::board::{self, BoardView};
use crate::footprints::PadKind;
use crate::geom::{self, P, Prim};
use crate::{Board, Track, Via};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, BTreeMap, HashSet};

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
    /// Remove earlier autorouted copper of these nets first (default true).
    pub replace: Option<bool>,
    /// Layers to use (default all).
    pub layers: Option<Vec<String>>,
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

impl Command for Autoroute {
    const NAME: &'static str = "pcb.autoroute";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = tx.get::<Board>(self.board).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a board", self.board)))?;
        let g = self.grid.unwrap_or(0.25);
        if !(0.05..=2.0).contains(&g) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "grid is 0.05 to 2 mm"));
        }
        let wanted: Option<HashSet<String>> = self.nets.as_ref().map(|v| v.iter().cloned().collect());
        let skip: HashSet<String> = self.skip.iter().cloned().collect();
        let pick = |n: &str| wanted.as_ref().is_none_or(|w| w.contains(n)) && !skip.contains(n);
        if self.replace.unwrap_or(true) {
            let it = crate::items(tx.world(), self.board);
            let gone: Vec<StableId> = it.tracks.iter().filter(|(_, t)| t.auto && t.net.as_deref().is_some_and(pick)).map(|(id, _)| *id).chain(it.vias.iter().filter(|(_, v)| v.auto && v.net.as_deref().is_some_and(pick)).map(|(id, _)| *id)).collect();
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
                .map(|n| {
                    let w = board.class_of(Some(&n.name)).track_width;
                    let span = spread(&view, &n.name);
                    (-w, span, n.name.clone())
                })
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            v.into_iter().map(|x| x.2).collect()
        };
        let grid = Grid { x0: (view.bounds[0] / g).floor() * g, y0: (view.bounds[1] / g).floor() * g, g, nx: ((view.bounds[2] - (view.bounds[0] / g).floor() * g) / g).ceil() as usize + 1, ny: ((view.bounds[3] - (view.bounds[1] / g).floor() * g) / g).ceil() as usize + 1 };
        if grid.nx * grid.ny > 4_000_000 {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "the board is too large for this grid: use a coarser grid"));
        }
        let nl = all_layers.len();
        let mut obstacles: Vec<Obstacle> = Vec::new();
        for p in &view.pads {
            let mut mask = 0u8;
            for l in &p.layers {
                if let Some(k) = board::layer_index(&all_layers, l) {
                    mask |= 1 << k;
                }
            }
            if mask != 0 {
                obstacles.push(Obstacle { prim: p.prim.clone(), layers: mask, net: p.net.clone() });
            }
        }
        for t in &view.tracks {
            if let Some(k) = board::layer_index(&all_layers, &t.layer) {
                for w in t.points.windows(2) {
                    obstacles.push(Obstacle { prim: Prim::capsule(w[0], w[1], t.width / 2.0), layers: 1 << k, net: t.net.clone() });
                }
            }
        }
        for v in &view.vias {
            obstacles.push(Obstacle { prim: Prim::circle(v.at, v.diameter / 2.0), layers: ((1u16 << nl) - 1) as u8, net: v.net.clone() });
        }
        for z in &view.zones {
            // Pours are refilled around new copper; they do not block.
            let _ = z;
        }
        let holes: Vec<Prim> = view.holes.iter().map(|h| Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0)).collect();
        let edges: Vec<Vec<P>> = std::iter::once(view.outline.clone()).chain(view.cutouts.iter().cloned()).collect();

        // Cells inside the board, with their distance to the edge.
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

        // Passes: a net that fails goes first next time, everything this
        // command made is ripped up between passes; the best order is kept.
        const PASSES: usize = 5;
        let base = obstacles;
        let mut order = todo;
        let mut best: Option<(usize, Vec<String>)> = None;
        let mut pass = 0;
        let (routed, failed, made_tracks, made_vias) = loop {
        pass += 1;
        let mut obstacles = base.clone();
        let mut created: Vec<StableId> = Vec::new();
        let mut routed = Vec::new();
        let mut failed: Vec<Value> = Vec::new();
        let mut made_tracks = 0;
        let mut made_vias = 0;
        for net in order.clone() {
            let class = board.class_of(Some(&net));
            let w = class.track_width;
            let clear = class.clearance.max(board.rules.min_space);
            let via_r = class.via_diameter / 2.0;
            // Open cells per layer, and where a via may stand.
            let mut open = vec![vec![false; ncell]; nl];
            let mut inside_own = vec![vec![false; ncell]; nl];
            for &l in &use_layers {
                for k in 0..ncell {
                    open[l][k] = edge_gap[k] >= board.rules.edge_clearance + w / 2.0;
                }
            }
            let mut via_ok: Vec<bool> = (0..ncell).map(|k| edge_gap[k] >= board.rules.edge_clearance + via_r).collect();
            for o in &obstacles {
                let mine = o.net.as_deref() == Some(net.as_str());
                let b = o.prim.bbox();
                let reach = clear.max(board.class_of(o.net.as_deref()).clearance) + w.max(2.0 * via_r) / 2.0 + MARGIN + g;
                let (i0, j0, i1, j1) = grid.range([b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach]);
                let need = clear.max(board.class_of(o.net.as_deref()).clearance) + MARGIN;
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let k = j * grid.nx + i;
                        let p = grid.at(i, j);
                        let d = depth(&o.prim, p);
                        if mine {
                            if d >= w / 2.0 - 1e-9 {
                                for &l in &use_layers {
                                    if o.layers & (1 << l) != 0 {
                                        inside_own[l][k] = true;
                                    }
                                }
                            }
                            // No via in a pad.
                            if d > -via_r {
                                via_ok[k] = false;
                            }
                            continue;
                        }
                        // Gap from a disc of radius w/2 at p to the obstacle is -d - w/2.
                        if -d - w / 2.0 < need {
                            for &l in &use_layers {
                                if o.layers & (1 << l) != 0 {
                                    open[l][k] = false;
                                }
                            }
                        }
                        if -d - via_r < need {
                            via_ok[k] = false;
                        }
                    }
                }
            }
            for h in &holes {
                let b = h.bbox();
                let reach = board.rules.hole_to_hole + via_r + g;
                let (i0, j0, i1, j1) = grid.range([b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach]);
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let d = depth(h, grid.at(i, j));
                        if -d - class.via_drill / 2.0 < board.rules.hole_to_hole {
                            via_ok[j * grid.nx + i] = false;
                        }
                    }
                }
            }
            for &l in &use_layers {
                for k in 0..ncell {
                    if inside_own[l][k] {
                        open[l][k] = true;
                    }
                }
            }
            if use_layers.len() < 2 {
                via_ok.iter_mut().for_each(|v| *v = false);
            }

            // The net's pads, each with its open cells.
            let pads: Vec<&crate::board::PadView> = view.pads.iter().filter(|p| p.net.as_deref() == Some(net.as_str()) && p.kind != PadKind::Npth).collect();
            let mut pad_cells: Vec<Vec<usize>> = Vec::new();
            for p in &pads {
                let mut cells = Vec::new();
                let (i0, j0, i1, j1) = grid.range(p.prim.bbox());
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let k = j * grid.nx + i;
                        for &l in &use_layers {
                            if inside_own[l][k] && p.layers.iter().any(|x| board::layer_index(&all_layers, x) == Some(l)) && depth(&p.prim, grid.at(i, j)) >= w / 2.0 - 1e-9 {
                                cells.push(l * ncell + k);
                            }
                        }
                    }
                }
                pad_cells.push(cells);
            }
            if pad_cells.iter().any(|c| c.is_empty()) {
                let small: Vec<String> = pads.iter().zip(&pad_cells).filter(|(_, c)| c.is_empty()).map(|(p, _)| format!("{}.{}", p.reference, p.number)).collect();
                failed.push(json!({ "net": net, "reason": format!("no grid point fits a {w} mm track inside {} (try a finer grid or a narrower net class)", small.join(", ")) }));
                continue;
            }
            let mut connected: Vec<usize> = pad_cells[0].clone();
            // Pads sharing a land (USB-C's paired contacts) are reached together.
            let mut reached: Vec<bool> = pad_cells.iter().map(|cells| cells.iter().any(|c| connected.contains(c))).collect();
            let mut new_obstacles: Vec<Obstacle> = Vec::new();
            let mut ok = true;
            while reached.iter().any(|r| !r) {
                let mut target: BTreeMap<usize, usize> = BTreeMap::new();
                for (pi, cells) in pad_cells.iter().enumerate() {
                    if !reached[pi] {
                        for c in cells {
                            target.insert(*c, pi);
                        }
                    }
                }
                let goals: Vec<P> = pads.iter().enumerate().filter(|(i, _)| !reached[*i]).map(|(_, p)| p.at).collect();
                let Some(path) = search(&grid, nl, &open, &via_ok, &connected, &target, &goals, ncell) else {
                    if std::env::var("FORMA_ROUTE_DEBUG").is_ok() {
                        let q = pads.iter().zip(&reached).find(|(_, r)| !**r).unwrap().0.at;
                        for jj in (0..40).rev() {
                            let mut line = String::new();
                            for ii in 0..40 {
                                let x = q[0] - 5.0 + ii as f64 * g;
                                let y = q[1] - 5.0 + jj as f64 * g;
                                let i = ((x - grid.x0) / g).round() as usize;
                                let j = ((y - grid.y0) / g).round() as usize;
                                let k = j * grid.nx + i;
                                line.push(if pad_cells.iter().any(|c| c.contains(&k)) { 'P' } else if inside_own[0][k] { 'o' } else if open[0][k] { '.' } else { '#' });
                            }
                            eprintln!("{line}");
                        }
                    }
                    let missing: Vec<String> = pads.iter().zip(&reached).filter(|(_, r)| !**r).map(|(p, _)| format!("{}.{}", p.reference, p.number)).collect();
                    failed.push(json!({ "net": net, "reason": format!("no path to {}", missing.join(", ")) }));
                    ok = false;
                    break;
                };
                let end = *path.last().unwrap();
                // Every pad the path ends in or crosses is reached.
                for n in &path {
                    if let Some(pi) = target.get(n) {
                        reached[*pi] = true;
                    }
                }
                reached[target[&end]] = true;
                // Copper: runs on one layer become tracks; layer changes become vias.
                let mut run: Vec<P> = Vec::new();
                let mut run_layer = path[0] / ncell;
                for (idx, n) in path.iter().enumerate() {
                    let l = n / ncell;
                    let k = n % ncell;
                    let p = grid.at(k % grid.nx, k / grid.nx);
                    if l != run_layer {
                        if run.len() >= 2 {
                            made_tracks += emit_track(tx, self.board, &net, &all_layers[run_layer], w, &run, &mut new_obstacles, run_layer, &mut created)?;
                        }
                        made_vias += emit_via(tx, self.board, &net, p, class.via_drill, class.via_diameter, &mut new_obstacles, nl, &mut created)?;
                        run = vec![p];
                        run_layer = l;
                    } else {
                        run.push(p);
                    }
                    let _ = idx;
                }
                if run.len() >= 2 {
                    made_tracks += emit_track(tx, self.board, &net, &all_layers[run_layer], w, &run, &mut new_obstacles, run_layer, &mut created)?;
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
            obstacles.extend(new_obstacles);
            if ok {
                routed.push(net);
            }
        }
        if failed.is_empty() {
            break (routed, failed, made_tracks, made_vias);
        }
        let better = best.as_ref().is_none_or(|(n, _)| failed.len() < *n);
        if better {
            best = Some((failed.len(), order.clone()));
        }
        let best_order = best.as_ref().map(|b| b.1.clone()).unwrap();
        if pass >= PASSES && order == best_order {
            break (routed, failed, made_tracks, made_vias);
        }
        for id in created {
            tx.remove_dependency(self.board, id)?;
            tx.delete(id)?;
        }
        if pass >= PASSES {
            order = best_order;
        } else {
            let bad: Vec<String> = failed.iter().filter_map(|f| f["net"].as_str().map(String::from)).collect();
            let mut next: Vec<String> = bad.clone();
            next.extend(order.iter().filter(|n| !bad.contains(n)).cloned());
            order = next;
        }
        };
        Ok(json!({ "board": self.board, "routed": routed, "failed": failed, "tracks": made_tracks, "vias": made_vias, "grid": g, "passes": pass }))
    }
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

fn simplify(points: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for p in points {
        if out.last().is_some_and(|q| geom::dist(*q, *p) < 1e-9) {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            if geom::cross(geom::sub(b, a), geom::sub(*p, b)).abs() < 1e-9 && geom::dot(geom::sub(b, a), geom::sub(*p, b)) > 0.0 {
                out.pop();
            }
        }
        out.push(*p);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_track(tx: &mut Tx, board: StableId, net: &str, layer: &str, w: f64, run: &[P], obstacles: &mut Vec<Obstacle>, li: usize, created: &mut Vec<StableId>) -> Result<usize, CommandError> {
    let points = simplify(run);
    if points.len() < 2 {
        return Ok(0);
    }
    for s in points.windows(2) {
        obstacles.push(Obstacle { prim: Prim::capsule(s[0], s[1], w / 2.0), layers: 1 << li, net: Some(net.to_string()) });
    }
    let id = tx.create();
    tx.set(id, Track { board, net: Some(net.to_string()), layer: layer.to_string(), width: w, points, auto: true, reroute: false })?;
    created.push(id);
    tx.add_dependency(board, id)?;
    Ok(1)
}

#[allow(clippy::too_many_arguments)]
fn emit_via(tx: &mut Tx, board: StableId, net: &str, at: P, drill: f64, diameter: f64, obstacles: &mut Vec<Obstacle>, nl: usize, created: &mut Vec<StableId>) -> Result<usize, CommandError> {
    obstacles.push(Obstacle { prim: Prim::circle(at, diameter / 2.0), layers: ((1u16 << nl) - 1) as u8, net: Some(net.to_string()) });
    let id = tx.create();
    tx.set(id, Via { board, net: Some(net.to_string()), at, drill, diameter, auto: true })?;
    created.push(id);
    tx.add_dependency(board, id)?;
    Ok(1)
}

/// A* from any connected node to any target node. Nodes are layer·ncell + cell;
/// the direction last moved is part of the state so bends cost.
#[allow(clippy::too_many_arguments)]
fn search(grid: &Grid, nl: usize, open: &[Vec<bool>], via_ok: &[bool], source_list: &[usize], targets: &BTreeMap<usize, usize>, goals: &[P], ncell: usize) -> Option<Vec<usize>> {
    let g = grid.g;
    let sources: HashSet<usize> = source_list.iter().copied().collect();
    let nd = 9; // 8 directions + "arrived by via / start"
    let total = nl * ncell * nd;
    let mut best = vec![f64::INFINITY; total];
    let mut prev = vec![usize::MAX; total];
    let mut heap = BinaryHeap::new();
    let h = |node: usize| -> f64 {
        let k = node % ncell;
        let p = grid.at(k % grid.nx, k / grid.nx);
        goals.iter().map(|q| {
            let (dx, dy) = ((p[0] - q[0]).abs(), (p[1] - q[1]).abs());
            dx.max(dy) + (std::f64::consts::SQRT_2 - 1.0) * dx.min(dy)
        }).fold(f64::INFINITY, f64::min) * 0.9
    };
    for &s in source_list {
        let st = s * nd + 8;
        best[st] = 0.0;
        heap.push(State { cost: h(s), node: st });
    }
    let via_cost = 12.0 * g;
    let mut found = None;
    let mut pops = 0usize;
    while let Some(State { cost: _, node }) = heap.pop() {
        pops += 1;
        if pops > 6_000_000 {
            return None;
        }
        let cell_node = node / nd;
        let dir = node % nd;
        let here = best[node];
        if targets.contains_key(&cell_node) && !sources.contains(&cell_node) {
            found = Some(node);
            break;
        }
        let l = cell_node / ncell;
        let k = cell_node % ncell;
        let (i, j) = ((k % grid.nx) as i32, (k / grid.nx) as i32);
        // Moves on the layer.
        for (d, (di, dj)) in DIRS.iter().enumerate() {
            let (ni, nj) = (i + di, j + dj);
            if ni < 0 || nj < 0 || ni >= grid.nx as i32 || nj >= grid.ny as i32 {
                continue;
            }
            let nk = nj as usize * grid.nx + ni as usize;
            if !open[l][nk] {
                continue;
            }
            // A diagonal step must not cut a closed corner.
            if *di != 0 && *dj != 0 && (!open[l][j as usize * grid.nx + ni as usize] || !open[l][nj as usize * grid.nx + i as usize]) {
                continue;
            }
            let step = if *di != 0 && *dj != 0 { std::f64::consts::SQRT_2 * g } else { g };
            let turn = if dir == 8 || dir == d { 0.0 } else {
                let a = (dir as i32 - d as i32).rem_euclid(8);
                let a = a.min(8 - a);
                match a {
                    1 => 0.6 * g,
                    2 => 2.0 * g,
                    _ => 8.0 * g,
                }
            };
            // A light preference: top runs across, bottom runs along.
            let lean = if (l == 0 && *dj != 0 && *di == 0) || (l == nl - 1 && *di != 0 && *dj == 0) { 0.15 * step } else { 0.0 };
            let nn = (l * ncell + nk) * nd + d;
            let c = here + step + turn + lean;
            if c < best[nn] {
                best[nn] = c;
                prev[nn] = node;
                heap.push(State { cost: c + h(l * ncell + nk), node: nn });
            }
        }
        // Vias.
        if via_ok[k] {
            for nl2 in 0..nl {
                if nl2 == l || !open[nl2][k] {
                    continue;
                }
                let nn = (nl2 * ncell + k) * nd + 8;
                let c = here + via_cost;
                if c < best[nn] {
                    best[nn] = c;
                    prev[nn] = node;
                    heap.push(State { cost: c + h(nl2 * ncell + k), node: nn });
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
