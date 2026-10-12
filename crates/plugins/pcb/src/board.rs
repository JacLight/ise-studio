//! A board resolved: every pad in board coordinates with its net, the zone
//! fills, connectivity (which copper touches which), the ratsnest of
//! connections still to route, and the design-rule check. This is what the
//! 2D editor draws, what the outputs are written from and what the AI reads.

use crate::footprints::{BodyShape, PadKind, PadShape};
use crate::geom::{self, P, Prim};
use crate::{Board, BoardResult, PadConnection, PcbComponent, Rules, Side, Zone};
use de_core::{IdIndex, StableId, bevy_ecs::{entity::Entity, world::World}};
use manifold_csg::{CrossSection, JoinType};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
pub struct BoardView {
    pub id: StableId,
    pub name: String,
    pub layers: Vec<String>,
    pub thickness: f64,
    pub finish: String,
    pub copper_oz: f64,
    pub mask: String,
    pub silk: String,
    pub revision: String,
    pub quantity: u32,
    pub fab: Option<String>,
    pub rules: Rules,
    pub model: Option<StableId>,
    /// Counter-clockwise outline and clockwise-free cutouts, mm.
    pub outline: Vec<P>,
    pub cutouts: Vec<Vec<P>>,
    /// [min x, min y, max x, max y] of the outline.
    pub bounds: [f64; 4],
    pub components: Vec<ComponentView>,
    pub pads: Vec<PadView>,
    pub tracks: Vec<TrackView>,
    pub vias: Vec<ViaView>,
    pub zones: Vec<ZoneView>,
    pub holes: Vec<Hole>,
    pub nets: Vec<NetView>,
    pub ratsnest: Vec<Airwire>,
    pub violations: Vec<Violation>,
    pub stats: Stats,
}

impl BoardView {
    pub fn size(&self) -> [f64; 2] {
        [self.bounds[2] - self.bounds[0], self.bounds[3] - self.bounds[1]]
    }
    pub fn errors(&self) -> usize {
        self.violations.iter().filter(|v| v.severity == "error").count()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ComponentView {
    pub id: StableId,
    pub reference: String,
    pub value: String,
    pub footprint: String,
    pub kind: String,
    pub at: P,
    pub rotation: f64,
    pub side: Side,
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
    pub lcsc: Option<String>,
    pub description: String,
    pub dnp: bool,
    /// Tallest point above the board surface (mm).
    pub height: f64,
    /// Courtyard corners in board coordinates.
    pub courtyard: Vec<P>,
    /// Silkscreen polylines in board coordinates.
    pub silk: Vec<Vec<P>>,
    /// Body blocks: outline in board coordinates and z range above its surface.
    pub bodies: Vec<BodyView>,
    /// Where the reference designator is printed.
    pub label: P,
    pub pins: BTreeMap<String, String>,
    pub pin_names: BTreeMap<String, String>,
    /// The footprint was found (otherwise it shows as a violation, with no pads).
    pub known: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BodyView {
    pub outline: Vec<P>,
    pub z: [f64; 2],
    pub material: String,
    pub cylinder: bool,
    pub center: P,
    pub size: [f64; 2],
}

#[derive(Clone, Debug, Serialize)]
pub struct PadView {
    pub component: StableId,
    pub reference: String,
    pub number: String,
    pub net: Option<String>,
    pub pin_name: Option<String>,
    pub kind: PadKind,
    pub shape: PadShape,
    pub layers: Vec<String>,
    pub prim: Prim,
    /// Centre, size and total rotation, for apertures.
    pub at: P,
    pub size: [f64; 2],
    pub rotation: f64,
    pub corner: f64,
    pub drill: Option<[f64; 2]>,
    /// Paste openings (the pad itself unless the footprint gives windows).
    pub paste: Vec<Prim>,
    pub side: Side,
    /// A custom pad's real outline on the board (`prim` is its convex hull).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<P>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackView {
    pub id: StableId,
    pub net: Option<String>,
    pub layer: String,
    pub width: f64,
    pub points: Vec<P>,
    pub auto: bool,
    /// Its end was dragged with a part: check it, or re-route it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reroute: bool,
    /// A track with arcs: `points` is the centreline flattened, these are its corners and bulges.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vertices: Vec<P>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bulges: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ViaView {
    pub id: StableId,
    pub net: Option<String>,
    pub at: P,
    pub drill: f64,
    pub diameter: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ZoneView {
    pub id: StableId,
    pub net: String,
    pub layer: String,
    pub outline: Vec<P>,
    /// The poured copper: islands, each a list of contours (outer counter-clockwise, holes clockwise).
    pub fill: Vec<Vec<Vec<P>>>,
    pub priority: i32,
    pub area: f64,
    /// Pads of the zone's net joined through thermal reliefs, with how many spokes reached the pour.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reliefs: Vec<Relief>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Relief {
    /// "J1.S1".
    pub pad: String,
    pub component: StableId,
    pub at: P,
    pub spokes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Hole {
    pub at: P,
    /// [width, height] (equal: round).
    pub size: [f64; 2],
    pub rotation: f64,
    pub plated: bool,
    /// "R1.2", "via", "H1".
    pub owner: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct NetView {
    pub name: String,
    pub pins: Vec<String>,
    pub class: String,
    /// Separate pieces of copper the net's pads are in (1 when fully routed).
    pub islands: usize,
    pub routed: bool,
    /// Total track length (mm).
    pub length: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Airwire {
    pub net: String,
    pub a: P,
    pub b: P,
}

#[derive(Clone, Debug, Serialize)]
pub struct Violation {
    /// clearance, short, track_width, drill, annular_ring, edge_clearance, outside_board,
    /// unrouted, courtyard_overlap, hole_to_hole, unknown_footprint, silk_text.
    pub code: &'static str,
    /// "error" or "warning".
    pub severity: &'static str,
    pub message: String,
    pub at: P,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// The entities it is about (components, tracks, vias, zones).
    pub objects: Vec<StableId>,
    /// Their names ("R1.2", "track on GND").
    pub refs: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub components: usize,
    pub smd_pads: usize,
    pub tht_pads: usize,
    pub nets: usize,
    pub routed_nets: usize,
    pub tracks: usize,
    pub vias: usize,
    pub holes: usize,
    pub track_length: f64,
    pub smallest_track: Option<f64>,
    pub smallest_drill: Option<f64>,
    pub smallest_annular: Option<f64>,
    pub smallest_gap: Option<f64>,
    pub tallest_top: f64,
    pub tallest_bottom: f64,
    /// Components on each side (for assembly).
    pub top_parts: usize,
    pub bottom_parts: usize,
}

/// One piece of copper for connectivity and clearance.
#[derive(Clone, Debug)]
pub struct Cu {
    pub prim: Prim,
    /// Indices into the board's copper layers.
    pub layers: u8,
    pub net: Option<String>,
    pub owner: Owner,
    pub bbox: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Owner {
    Pad(usize),
    Track(StableId, usize),
    Via(StableId),
}

pub fn layer_index(layers: &[String], name: &str) -> Option<usize> {
    layers.iter().position(|l| l == name)
}

fn all_layers(n: usize) -> u8 {
    ((1u16 << n) - 1) as u8
}

/// Derived computation for `pcb.board`.
pub(crate) fn compute(world: &World, e: Entity) -> Option<Result<BoardResult, String>> {
    let board = world.get::<Board>(e)?;
    let id = world.resource::<IdIndex>().iter().find(|(_, x)| *x == e).map(|(id, _)| id)?;
    Some(resolve(world, id, board).map(|v| BoardResult(Arc::new(v))))
}

/// The outline (counter-clockwise) and cutouts of a board, from its polygon or sketch.
pub fn outline_of(world: &World, board: &Board) -> Result<(Vec<P>, Vec<Vec<P>>), String> {
    let (mut outline, mut cutouts) = if let Some(s) = board.sketch {
        let e = world.resource::<IdIndex>().get(s).ok_or("the board's sketch is gone")?;
        let solved = world.get::<de_shape::sketch::SolvedSketch>(e).ok_or("the board's sketch is not solved yet")?;
        let mut polys: Vec<Vec<P>> = solved.loops.iter().map(|l| de_shape::sketch::polygon(l, 0.01)).filter(|p| p.len() >= 3).collect();
        if polys.is_empty() {
            return Err("the board's sketch has no closed outline".into());
        }
        polys.sort_by(|a, b| geom::polygon_area(b).abs().total_cmp(&geom::polygon_area(a).abs()));
        let outline = polys.remove(0);
        (outline, polys)
    } else {
        (geom::round_corners(&board.outline, board.corner_radius, 0.01), board.cutouts.clone())
    };
    if geom::polygon_area(&outline) < 0.0 {
        outline.reverse();
    }
    for c in &mut cutouts {
        if geom::polygon_area(c) > 0.0 {
            c.reverse();
        }
    }
    Ok((outline, cutouts))
}

fn place(at: P, rotation: f64, bottom: bool, p: P) -> P {
    let q = if bottom { [-p[0], p[1]] } else { p };
    geom::add(at, geom::rotate(q, rotation))
}

pub fn resolve(world: &World, id: StableId, board: &Board) -> Result<BoardView, String> {
    let items = crate::items(world, id);
    let (outline, cutouts) = outline_of(world, board)?;
    let layers: Vec<String> = board.copper_layers().iter().map(|s| s.to_string()).collect();
    let nl = layers.len();
    let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in &outline {
        bounds = [bounds[0].min(p[0]), bounds[1].min(p[1]), bounds[2].max(p[0]), bounds[3].max(p[1])];
    }
    let mut violations = Vec::new();
    let mut components = Vec::new();
    let mut pads: Vec<PadView> = Vec::new();
    let mut holes = Vec::new();

    for (cid, c) in &items.components {
        let bottom = c.side == Side::Bottom;
        let fp = board.footprint(&c.footprint);
        let Some(fp) = fp else {
            violations.push(Violation { code: "unknown_footprint", severity: "error", message: format!("{}: no footprint “{}”", c.reference, c.footprint), at: c.at, layer: None, objects: vec![*cid], refs: vec![c.reference.clone()] });
            components.push(component_view(*cid, c, None));
            continue;
        };
        let first = pads.len();
        for p in &fp.pads {
            let prim = crate::footprints::pad_prim(p, c.at, c.rotation, bottom);
            let rot = (if bottom { -p.rotation } else { p.rotation } + c.rotation).rem_euclid(360.0);
            let at = place(c.at, c.rotation, bottom, p.at);
            let side_layer = if bottom { layers[nl - 1].clone() } else { layers[0].clone() };
            let pad_layers = match p.kind {
                PadKind::Smd => vec![side_layer],
                PadKind::Tht => layers.clone(),
                PadKind::Npth => vec![],
            };
            let paste = if p.kind != PadKind::Smd {
                vec![]
            } else if p.paste.is_empty() {
                vec![prim.clone()]
            } else {
                p.paste.iter().map(|w| Prim::rect(place(c.at, c.rotation, bottom, [w[0], w[1]]), w[2], w[3], rot, 0.0)).collect()
            };
            if let Some(d) = p.drill {
                holes.push(Hole { at, size: d, rotation: rot, plated: p.kind == PadKind::Tht, owner: if p.number.is_empty() { c.reference.clone() } else { format!("{}.{}", c.reference, p.number) } });
            }
            let corner = match p.shape {
                PadShape::RoundRect => p.corner.unwrap_or(0.25) * p.size[0].min(p.size[1]),
                PadShape::Oval => p.size[0].min(p.size[1]) / 2.0,
                _ => 0.0,
            };
            pads.push(PadView {
                component: *cid,
                reference: c.reference.clone(),
                number: p.number.clone(),
                net: if p.kind == PadKind::Npth { None } else { c.pins.get(&p.number).cloned() },
                pin_name: c.pin_names.get(&p.number).cloned(),
                kind: p.kind,
                shape: p.shape,
                layers: pad_layers,
                prim,
                at,
                size: p.size,
                rotation: rot,
                corner,
                drill: p.drill,
                paste,
                side: c.side,
                polygon: Some(crate::footprints::pad_polygon(p, c.at, c.rotation, bottom)).filter(|v| !v.is_empty()),
            });
        }
        // Pads of one part that overlap are one piece of copper (USB-C's paired contacts, a switch's legs): one net.
        for i in first..pads.len() {
            for j in first..pads.len() {
                if i != j && pads[i].net.is_none() && pads[j].net.is_some() && !pads[i].layers.is_empty() && geom::gap(&pads[i].prim, &pads[j].prim) <= 0.0 {
                    pads[i].net = pads[j].net.clone();
                }
            }
        }
        components.push(component_view(*cid, c, Some(&fp)));
    }

    for (vid, v) in &items.vias {
        holes.push(Hole { at: v.at, size: [v.drill, v.drill], rotation: 0.0, plated: true, owner: "via".into() });
        let _ = vid;
    }

    // Copper for connectivity and clearance.
    let mut cu: Vec<Cu> = Vec::new();
    for (i, p) in pads.iter().enumerate() {
        let mut mask = 0u8;
        for l in &p.layers {
            if let Some(k) = layer_index(&layers, l) {
                mask |= 1 << k;
            }
        }
        if mask != 0 {
            cu.push(Cu { bbox: p.prim.bbox(), prim: p.prim.clone(), layers: mask, net: p.net.clone(), owner: Owner::Pad(i) });
        }
    }
    // Arcs flattened: everything downstream (copper, clearance, Gerber) sees the centreline as a polyline.
    let flat: BTreeMap<StableId, Vec<P>> = items.tracks.iter().filter(|(_, t)| t.bulges.iter().any(|b| *b != 0.0)).map(|(id, t)| (*id, crate::pns::flatten(&t.points, &t.bulges))).collect();
    let line_of = |id: &StableId, t: &crate::Track| -> Vec<P> { flat.get(id).cloned().unwrap_or_else(|| t.points.clone()) };
    for (tid, t) in &items.tracks {
        let Some(k) = layer_index(&layers, &t.layer) else { continue };
        for (s, w) in line_of(tid, t).windows(2).enumerate() {
            let prim = Prim::capsule(w[0], w[1], t.width / 2.0);
            cu.push(Cu { bbox: prim.bbox(), prim, layers: 1 << k, net: t.net.clone(), owner: Owner::Track(*tid, s) });
        }
    }
    for (vid, v) in &items.vias {
        let prim = Prim::circle(v.at, v.diameter / 2.0);
        cu.push(Cu { bbox: prim.bbox(), prim, layers: all_layers(nl), net: v.net.clone(), owner: Owner::Via(*vid) });
    }

    let zones = fill_zones(board, &items.zones, &outline, &cutouts, &cu, &holes, &layers, &pads);

    // Connectivity: union-find over copper of the same net that touches.
    let n = cu.len();
    let zone_islands: Vec<(usize, usize)> = zones.iter().enumerate().flat_map(|(zi, z)| (0..z.fill.len()).map(move |ii| (zi, ii))).collect();
    let mut uf = UnionFind::new(n + zone_islands.len());
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| cu[*a].bbox[0].total_cmp(&cu[*b].bbox[0]));
    let mut touching: Vec<(usize, usize)> = Vec::new();
    for (oi, &i) in order.iter().enumerate() {
        for &j in &order[oi + 1..] {
            if cu[j].bbox[0] > cu[i].bbox[2] + 1e-6 {
                break;
            }
            if cu[i].layers & cu[j].layers == 0 || !geom::bbox_overlap(&cu[i].bbox, &cu[j].bbox, 1e-6) {
                continue;
            }
            // Segments of one track join.
            let same_track = matches!((&cu[i].owner, &cu[j].owner), (Owner::Track(a, _), Owner::Track(b, _)) if a == b);
            if same_track || geom::gap(&cu[i].prim, &cu[j].prim) <= 1e-6 {
                touching.push((i, j));
                if cu[i].net == cu[j].net || same_track {
                    uf.union(i, j);
                }
            }
        }
    }
    for (k, (zi, ii)) in zone_islands.iter().enumerate() {
        let z = &zones[*zi];
        let Some(li) = layer_index(&layers, &z.layer) else { continue };
        for (i, c) in cu.iter().enumerate() {
            if c.layers & (1 << li) != 0 && c.net.as_deref() == Some(z.net.as_str()) && geom::gap_to_area(&c.prim, &z.fill[*ii]) <= 1e-6 {
                uf.union(i, n + k);
            }
        }
    }

    // Shorts: touching copper of different nets.
    let name_of = |c: &Cu| -> (String, Option<StableId>) {
        match &c.owner {
            Owner::Pad(i) => (format!("{}.{}", pads[*i].reference, pads[*i].number), Some(pads[*i].component)),
            Owner::Track(id, _) => (format!("track on {}", c.net.as_deref().unwrap_or("no net")), Some(*id)),
            Owner::Via(id) => (format!("via on {}", c.net.as_deref().unwrap_or("no net")), Some(*id)),
        }
    };
    let mut shorted: BTreeSet<(String, String)> = BTreeSet::new();
    for &(i, j) in &touching {
        let (a, b) = (&cu[i], &cu[j]);
        if a.net == b.net || matches!((&a.owner, &b.owner), (Owner::Track(x, _), Owner::Track(y, _)) if x == y) {
            continue;
        }
        if let (Owner::Pad(pi), Owner::Pad(pj)) = (&a.owner, &b.owner) {
            if pads[*pi].component == pads[*pj].component && (a.net.is_none() || b.net.is_none()) {
                continue;
            }
        }
        let (na, oa) = name_of(a);
        let (nb, ob) = name_of(b);
        let key = (na.clone().min(nb.clone()), na.clone().max(nb.clone()));
        if !shorted.insert(key) {
            continue;
        }
        let at = midpoint(&a.prim, &b.prim);
        let layer = (0..nl).find(|k| a.layers & b.layers & (1 << k) != 0).map(|k| layers[k].clone());
        violations.push(Violation {
            code: "short",
            severity: "error",
            message: format!("Short: {na} ({}) touches {nb} ({})", a.net.as_deref().unwrap_or("no net"), b.net.as_deref().unwrap_or("no net")),
            at,
            layer,
            objects: [oa, ob].into_iter().flatten().collect(),
            refs: vec![na, nb],
        });
    }

    // Nets.
    let mut net_pins: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in &pads {
        if let Some(n) = &p.net {
            let label = format!("{}.{}", p.reference, p.number);
            let v = net_pins.entry(n.clone()).or_default();
            if !v.contains(&label) {
                v.push(label);
            }
        }
    }
    for (_, t) in &items.tracks {
        if let Some(n) = &t.net {
            net_pins.entry(n.clone()).or_default();
        }
    }
    for (_, z) in &items.zones {
        net_pins.entry(z.net.clone()).or_default();
    }
    for v in net_pins.values_mut() {
        v.sort_by(|a, b| crate::natural(a, b));
    }
    let mut nets = Vec::new();
    let mut ratsnest = Vec::new();
    for (name, pins) in &net_pins {
        let members: Vec<usize> = cu.iter().enumerate().filter(|(_, c)| matches!(c.owner, Owner::Pad(_)) && c.net.as_deref() == Some(name)).map(|(i, _)| i).collect();
        let mut roots: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &m in &members {
            roots.entry(uf.find(m)).or_default().push(m);
        }
        let islands = roots.len();
        let length: f64 = items.tracks.iter().filter(|(_, t)| t.net.as_deref() == Some(name)).map(|(id, t)| line_of(id, t).windows(2).map(|w| geom::dist(w[0], w[1])).sum::<f64>()).sum();
        if islands > 1 {
            // Anchors of each island: its pads, plus the vias and track points joined to it.
            let groups: Vec<Vec<P>> = roots
                .keys()
                .map(|r| cu.iter().enumerate().filter(|(i, c)| c.net.as_deref() == Some(name) && uf.find(*i) == *r).flat_map(|(_, c)| c.prim.pts.clone()).collect())
                .collect();
            let wires = spanning(&groups);
            for (a, b) in &wires {
                ratsnest.push(Airwire { net: name.clone(), a: *a, b: *b });
            }
            if let Some((a, b)) = wires.first() {
                violations.push(Violation {
                    code: "unrouted",
                    severity: "error",
                    message: format!("{name}: {} connection{} still to route ({} pads in {islands} pieces)", wires.len(), if wires.len() == 1 { "" } else { "s" }, members.len()),
                    at: [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0],
                    layer: None,
                    objects: pads.iter().filter(|p| p.net.as_deref() == Some(name)).map(|p| p.component).collect::<BTreeSet<_>>().into_iter().collect(),
                    refs: pins.clone(),
                });
            }
        }
        nets.push(NetView { name: name.clone(), pins: pins.clone(), class: board.class_of(Some(name)).name, islands, routed: islands <= 1, length });
    }
    nets.sort_by(|a, b| crate::natural(&a.name, &b.name));

    let tracks: Vec<TrackView> = items.tracks.iter().map(|(id, t)| {
            let arcs = flat.contains_key(id);
            TrackView { id: *id, net: t.net.clone(), layer: t.layer.clone(), width: t.width, points: line_of(id, t), auto: t.auto, reroute: t.reroute, vertices: if arcs { t.points.clone() } else { vec![] }, bulges: if arcs { t.bulges.clone() } else { vec![] } }
        }).collect();
    let vias: Vec<ViaView> = items.vias.iter().map(|(id, v)| ViaView { id: *id, net: v.net.clone(), at: v.at, drill: v.drill, diameter: v.diameter }).collect();

    let mut view = BoardView {
        id,
        name: board.name.clone(),
        layers,
        thickness: board.thickness,
        finish: board.finish.clone(),
        copper_oz: board.copper_oz,
        mask: board.mask.clone(),
        silk: board.silk.clone(),
        revision: board.revision.clone(),
        quantity: board.quantity,
        fab: board.fab.clone(),
        rules: board.rules.clone(),
        model: board.model,
        outline,
        cutouts,
        bounds,
        components,
        pads,
        tracks,
        vias,
        zones,
        holes,
        nets,
        ratsnest,
        violations,
        stats: Stats::default(),
    };
    view.stats = stats(&view, &items);
    crate::drc::check(board, &mut view, &cu);
    Ok(view)
}

fn stats(v: &BoardView, items: &crate::Items) -> Stats {
    let min = |it: &mut dyn Iterator<Item = f64>| it.fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))));
    Stats {
        components: v.components.len(),
        smd_pads: v.pads.iter().filter(|p| p.kind == PadKind::Smd).count(),
        tht_pads: v.pads.iter().filter(|p| p.kind == PadKind::Tht).count(),
        nets: v.nets.len(),
        routed_nets: v.nets.iter().filter(|n| n.routed && n.pins.len() > 1).count(),
        tracks: v.tracks.len(),
        vias: v.vias.len(),
        holes: v.holes.len(),
        track_length: v.nets.iter().map(|n| n.length).sum(),
        smallest_track: min(&mut v.tracks.iter().map(|t| t.width)),
        smallest_drill: min(&mut v.holes.iter().filter(|h| h.plated).map(|h| h.size[0].min(h.size[1]))),
        smallest_annular: min(&mut v.pads.iter().filter_map(|p| p.drill.filter(|_| p.kind == PadKind::Tht).map(|d| (p.size[0].min(p.size[1]) - d[0].min(d[1])) / 2.0)).chain(items.vias.iter().map(|(_, x)| (x.diameter - x.drill) / 2.0))),
        smallest_gap: None,
        tallest_top: v.components.iter().filter(|c| c.side == Side::Top).map(|c| c.height).fold(0.0, f64::max),
        tallest_bottom: v.components.iter().filter(|c| c.side == Side::Bottom).map(|c| c.height).fold(0.0, f64::max),
        top_parts: v.components.iter().filter(|c| c.side == Side::Top && !c.dnp && c.kind != "hole").count(),
        bottom_parts: v.components.iter().filter(|c| c.side == Side::Bottom && !c.dnp && c.kind != "hole").count(),
    }
}

fn component_view(id: StableId, c: &PcbComponent, fp: Option<&crate::Footprint>) -> ComponentView {
    let bottom = c.side == Side::Bottom;
    let pl = |p: P| place(c.at, c.rotation, bottom, p);
    let (courtyard, silk, bodies, height, kind) = match fp {
        Some(fp) => {
            let cr = fp.courtyard_rect();
            let courtyard = [[cr[0], cr[1]], [cr[2], cr[1]], [cr[2], cr[3]], [cr[0], cr[3]]].iter().map(|p| pl(*p)).collect();
            let silk = fp.silk_lines().iter().map(|l| l.iter().map(|p| pl(*p)).collect()).collect();
            let bodies = fp
                .body
                .iter()
                .map(|b| {
                    let (hw, hh) = (b.size[0] / 2.0, b.size[1] / 2.0);
                    let outline = if b.shape == BodyShape::Cylinder {
                        (0..24).map(|i| {
                            let a = std::f64::consts::TAU * i as f64 / 24.0;
                            pl([b.at[0] + hw * a.cos(), b.at[1] + hw * a.sin()])
                        }).collect()
                    } else {
                        [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].iter().map(|q| pl([b.at[0] + q[0], b.at[1] + q[1]])).collect()
                    };
                    BodyView { outline, z: b.z, material: b.material.clone(), cylinder: b.shape == BodyShape::Cylinder, center: pl(b.at), size: b.size }
                })
                .collect();
            (courtyard, silk, bodies, fp.height(), fp.kind.clone())
        }
        None => (vec![], vec![], vec![], 0.0, String::new()),
    };
    // The reference sits just above the courtyard (in board coordinates).
    let label = if courtyard.is_empty() {
        c.at
    } else {
        let top = courtyard.iter().map(|p: &P| p[1]).fold(f64::NEG_INFINITY, f64::max);
        let xs: Vec<f64> = courtyard.iter().map(|p| p[0]).collect();
        [(xs.iter().cloned().fold(f64::INFINITY, f64::min) + xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)) / 2.0, top + 0.75]
    };
    ComponentView {
        id,
        reference: c.reference.clone(),
        value: c.value.clone(),
        footprint: c.footprint.clone(),
        kind,
        at: c.at,
        rotation: c.rotation,
        side: c.side,
        mpn: c.mpn.clone(),
        manufacturer: c.manufacturer.clone(),
        lcsc: c.lcsc.clone(),
        description: c.description.clone(),
        dnp: c.dnp,
        height,
        courtyard,
        silk,
        bodies,
        label,
        pins: c.pins.clone(),
        pin_names: c.pin_names.clone(),
        known: fp.is_some(),
    }
}

fn midpoint(a: &Prim, b: &Prim) -> P {
    let (ca, cb) = (a.center(), b.center());
    [(ca[0] + cb[0]) / 2.0, (ca[1] + cb[1]) / 2.0]
}

/// The shortest connections joining every group (Kruskal over group pairs).
fn spanning(groups: &[Vec<P>]) -> Vec<(P, P)> {
    let mut edges = Vec::new();
    for i in 0..groups.len() {
        for j in i + 1..groups.len() {
            let mut best = (f64::INFINITY, [0.0; 2], [0.0; 2]);
            for a in &groups[i] {
                for b in &groups[j] {
                    let d = geom::dist(*a, *b);
                    if d < best.0 {
                        best = (d, *a, *b);
                    }
                }
            }
            edges.push((best.0, i, j, best.1, best.2));
        }
    }
    edges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut uf = UnionFind::new(groups.len());
    let mut out = Vec::new();
    for (_, i, j, a, b) in edges {
        if uf.find(i) != uf.find(j) {
            uf.union(i, j);
            out.push((a, b));
        }
    }
    out
}

pub struct UnionFind(Vec<usize>);

impl UnionFind {
    pub fn new(n: usize) -> Self {
        UnionFind((0..n).collect())
    }
    pub fn find(&mut self, i: usize) -> usize {
        let mut r = i;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut k = i;
        while self.0[k] != r {
            let next = self.0[k];
            self.0[k] = r;
            k = next;
        }
        r
    }
    pub fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra.max(rb)] = ra.min(rb);
        }
    }
}

const TOL: f64 = 0.01;

fn segs(r: f64) -> i32 {
    geom::segments_for(r, TOL) as i32
}

/// Pours each zone: its outline inside the board (kept `edge_clearance` from
/// the edge), minus other nets' copper and every hole grown by the clearance,
/// minus higher-priority pours of other nets; necks thinner than `min_width`
/// removed; islands that join nothing of the net dropped.
#[allow(clippy::too_many_arguments)]
fn fill_zones(board: &Board, zones: &[(StableId, Zone)], outline: &[P], cutouts: &[Vec<P>], cu: &[Cu], holes: &[Hole], layers: &[String], pads: &[PadView]) -> Vec<ZoneView> {
    let mut order: Vec<&(StableId, Zone)> = zones.iter().collect();
    order.sort_by(|a, b| b.1.priority.cmp(&a.1.priority).then(a.0.cmp(&b.0)));
    let edge = board.rules.edge_clearance;
    let mut area = CrossSection::from_polygons(&[outline.to_vec()]);
    for c in cutouts {
        let mut ccw = c.clone();
        if geom::polygon_area(&ccw) < 0.0 {
            ccw.reverse();
        }
        area = area.difference(&CrossSection::from_polygons(&[ccw]));
    }
    let inside = area.offset(-edge, JoinType::Round, 2.0, segs(edge.max(0.1)));
    let mut done: Vec<(String, String, CrossSection)> = Vec::new();
    let mut out = Vec::new();
    for (zid, z) in order {
        let Some(li) = layer_index(layers, &z.layer) else { continue };
        let class = board.class_of(Some(&z.net));
        let gap = z.clearance.unwrap_or(class.clearance).max(board.rules.min_space);
        let mut shape = if z.outline.len() >= 3 {
            let mut o = z.outline.clone();
            if geom::polygon_area(&o) < 0.0 {
                o.reverse();
            }
            CrossSection::from_polygons(&[o]).intersection(&inside)
        } else {
            inside.clone()
        };
        let mut cut = Vec::new();
        for c in cu {
            if c.layers & (1 << li) == 0 || c.net.as_deref() == Some(z.net.as_str()) {
                continue;
            }
            let g = gap.max(board.class_of(c.net.as_deref()).clearance);
            cut.push(c.prim.section(g, TOL));
        }
        for h in holes {
            let r = h.size[0].max(h.size[1]) / 2.0;
            if !h.plated {
                cut.push(Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0).section(gap, TOL));
            } else {
                let _ = r;
            }
        }
        for (net, layer, fill) in &done {
            if layer == &z.layer && net != &z.net {
                cut.push(fill.offset(gap, JoinType::Round, 2.0, segs(gap)));
            }
        }
        if !cut.is_empty() {
            shape = shape.difference(&CrossSection::batch_union(&cut));
        }
        // Pads of the zone's own net: joined solid, through thermal reliefs (a gap
        // ring crossed by four spokes along the pad's axes), or kept clear.
        let relieved: Vec<usize> = cu
            .iter()
            .filter_map(|c| match c.owner {
                Owner::Pad(i) if c.layers & (1 << li) != 0 && c.net.as_deref() == Some(z.net.as_str()) => Some(i),
                _ => None,
            })
            .filter(|i| match z.connection {
                PadConnection::Solid => false,
                PadConnection::ThermalTht => pads[*i].kind == PadKind::Tht,
                PadConnection::Thermal | PadConnection::None => true,
            })
            .collect();
        let allowed = shape.clone();
        let tgap = z.gap_thermal();
        if !relieved.is_empty() {
            let rings: Vec<CrossSection> = relieved.iter().map(|i| pads[*i].prim.section(tgap, TOL)).collect();
            shape = shape.difference(&CrossSection::batch_union(&rings));
        }
        let half = z.min_width / 2.0;
        if half > 0.0 {
            shape = shape.offset(-half, JoinType::Round, 2.0, segs(half)).offset(half, JoinType::Round, 2.0, segs(half));
        }
        let mut reliefs = Vec::new();
        if z.connection != PadConnection::None && !relieved.is_empty() {
            let sw = z.spoke_width();
            let mut spokes = Vec::new();
            for &i in &relieved {
                let p = &pads[i];
                let mut n = 0;
                for k in 0..4 {
                    let a = p.rotation + 90.0 * k as f64;
                    let along = if k % 2 == 0 { p.size[0] } else { p.size[1] } / 2.0;
                    let reach = along + tgap + sw;
                    let dir = geom::rotate([1.0, 0.0], a);
                    let end = geom::add(p.at, [dir[0] * reach, dir[1] * reach]);
                    let spoke = Prim::rect([(p.at[0] + end[0]) / 2.0, (p.at[1] + end[1]) / 2.0], reach, sw, a, 0.0).section(0.0, TOL).intersection(&allowed);
                    // Only a spoke that reaches the pour joins anything.
                    let tip = Prim::rect(geom::add(p.at, [dir[0] * (along + tgap + sw * 0.5), dir[1] * (along + tgap + sw * 0.5)]), sw, sw, a, 0.0).section(0.0, TOL);
                    if spoke.area() > 1e-6 && tip.intersection(&shape).area() > sw * sw * 0.2 && spoke.intersection(&tip).area() > sw * sw * 0.5 {
                        spokes.push(spoke);
                        n += 1;
                    }
                }
                reliefs.push(Relief { pad: format!("{}.{}", p.reference, p.number), component: p.component, at: p.at, spokes: n });
            }
            if !spokes.is_empty() {
                shape = shape.union(&CrossSection::batch_union(&spokes));
            }
        }
        let mut islands: Vec<Vec<Vec<P>>> = Vec::new();
        let mut kept = Vec::new();
        for piece in shape.decompose() {
            let contours = geom::contours(&piece);
            if contours.is_empty() {
                continue;
            }
            let joins = cu.iter().any(|c| c.layers & (1 << li) != 0 && c.net.as_deref() == Some(z.net.as_str()) && geom::gap_to_area(&c.prim, &contours) <= 1e-6);
            if z.remove_islands && !joins {
                continue;
            }
            islands.push(contours);
            kept.push(piece);
        }
        let fill = if kept.is_empty() { CrossSection::empty() } else { CrossSection::batch_union(&kept) };
        let a = fill.area();
        done.push((z.net.clone(), z.layer.clone(), fill));
        out.push(ZoneView { id: *zid, net: z.net.clone(), layer: z.layer.clone(), outline: if z.outline.len() >= 3 { z.outline.clone() } else { outline.to_vec() }, fill: islands, priority: z.priority, area: a, reliefs });
    }
    out
}
