//! Design-rule check: every violation names the objects it is about and
//! where it is, so the editor can point at it and the AI can fix it.
//!
//! Checked: copper clearance between nets (pads, tracks, vias, per net
//! class), shorts and unrouted connections (from connectivity, in `board`),
//! track width, plated drill size, annular rings, copper and holes to the
//! board edge, hole to hole, courtyard overlap, nets with a single pad and
//! tracks with no net.

use crate::board::{BoardView, Cu, Owner, Violation};
use crate::footprints::PadKind;
use crate::geom::{self, P, Prim};
use crate::{Board, Side};
use de_core::StableId;
use std::collections::BTreeSet;

fn closest_on(prim: &Prim, p: P) -> P {
    let hull = geom::convex_hull(&prim.pts);
    let mut best = (f64::INFINITY, hull[0]);
    let n = hull.len();
    let edges: Vec<(P, P)> = if n == 1 { vec![(hull[0], hull[0])] } else if n == 2 { vec![(hull[0], hull[1])] } else { (0..n).map(|i| (hull[i], hull[(i + 1) % n])).collect() };
    for (a, b) in edges {
        let ab = geom::sub(b, a);
        let l2 = geom::dot(ab, ab);
        let t = if l2 < 1e-24 { 0.0 } else { (geom::dot(geom::sub(p, a), ab) / l2).clamp(0.0, 1.0) };
        let q = geom::add(a, [ab[0] * t, ab[1] * t]);
        let d = geom::dist(p, q);
        if d < best.0 {
            best = (d, q);
        }
    }
    let q = best.1;
    let d = geom::dist(p, q);
    if d < 1e-12 { q } else { geom::add(q, [(p[0] - q[0]) / d * prim.r.min(d), (p[1] - q[1]) / d * prim.r.min(d)]) }
}

/// The point midway across the gap between two shapes.
pub fn gap_point(a: &Prim, b: &Prim) -> P {
    let qa = closest_on(a, b.center());
    let qb = closest_on(b, qa);
    let qa = closest_on(a, qb);
    [(qa[0] + qb[0]) / 2.0, (qa[1] + qb[1]) / 2.0]
}

pub fn check(board: &Board, v: &mut BoardView, cu: &[Cu]) {
    let rules = board.rules.clone();
    let mut out: Vec<Violation> = Vec::new();
    let name = |c: &Cu| -> (String, StableId) {
        match &c.owner {
            Owner::Pad(i) => (format!("{}.{}", v.pads[*i].reference, v.pads[*i].number), v.pads[*i].component),
            Owner::Track(id, _) => (format!("track on {}", c.net.as_deref().unwrap_or("no net")), *id),
            Owner::Via(id) => (format!("via on {}", c.net.as_deref().unwrap_or("no net")), *id),
            Owner::Graphic(id) => (format!("copper graphic on {}", c.net.as_deref().unwrap_or("no net")), *id),
        }
    };
    let custom = crate::drc_extra::Custom::new(board, v, cu);
    let layer_name = |mask: u32| (0..v.layers.len()).find(|k| mask & (1 << k) != 0).map(|k| v.layers[k].clone());

    // Clearance between nets.
    let mut order: Vec<usize> = (0..cu.len()).collect();
    order.sort_by(|a, b| cu[*a].bbox[0].total_cmp(&cu[*b].bbox[0]));
    let reach = board.net_classes.iter().map(|c| c.clearance).fold(rules.min_space.max(0.2), f64::max).max(custom.max_clearance());
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut smallest: Option<f64> = None;
    for (oi, &i) in order.iter().enumerate() {
        for &j in &order[oi + 1..] {
            let (a, b) = (&cu[i], &cu[j]);
            if b.bbox[0] > a.bbox[2] + reach {
                break;
            }
            if a.layers & b.layers == 0 || a.net == b.net || !geom::bbox_overlap(&a.bbox, &b.bbox, reach) {
                continue;
            }
            if let (Owner::Track(x, _), Owner::Track(y, _)) = (&a.owner, &b.owner) {
                if x == y {
                    continue;
                }
            }
            if let (Owner::Pad(pi), Owner::Pad(pj)) = (&a.owner, &b.owner) {
                // A part's own unconnected pads may sit as close as the footprint puts them.
                if v.pads[*pi].component == v.pads[*pj].component && (a.net.is_none() || b.net.is_none()) {
                    continue;
                }
            }
            let g = geom::gap(&a.prim, &b.prim);
            if g <= 1e-6 {
                continue; // a short, reported by connectivity
            }
            smallest = Some(smallest.map_or(g, |s: f64| s.min(g)));
            let builtin = board.class_of(a.net.as_deref()).clearance.max(board.class_of(b.net.as_deref()).clearance).max(rules.min_space);
            let builtin = crate::signal::pair_clearance(board, &*v, a.net.as_deref(), b.net.as_deref(), builtin);
            let (need, rule) = match custom.clearance(i, j, layer_name(a.layers & b.layers).as_deref()) {
                Some((min, name)) => (min.max(rules.min_space), Some(name)),
                None => (builtin, None),
            };
            if g + 1e-6 < need {
                let (na, oa) = name(a);
                let (nb, ob) = name(b);
                let key = (na.clone().min(nb.clone()), na.clone().max(nb.clone()));
                if !seen.insert(key) {
                    continue;
                }
                out.push(Violation {
                    code: "clearance",
                    severity: "error",
                    message: format!("{na} is {g:.3} mm from {nb}; needs {need:.3} mm{}", rule.map(|r| format!(" (rule “{r}”)")).unwrap_or_default()),
                    at: gap_point(&a.prim, &b.prim),
                    layer: layer_name(a.layers & b.layers),
                    objects: vec![oa, ob],
                    refs: vec![na, nb],
                });
            }
        }
    }

    // Tracks.
    for t in &v.tracks {
        if t.width + 1e-9 < rules.min_track {
            out.push(Violation { code: "track_width", severity: "error", message: format!("Track on {} is {:.3} mm wide; the minimum is {:.3} mm", t.net.as_deref().unwrap_or("no net"), t.width, rules.min_track), at: t.points[0], layer: Some(t.layer.clone()), objects: vec![t.id], refs: vec![format!("track on {}", t.net.as_deref().unwrap_or("no net"))] });
        }
        if t.net.is_none() {
            out.push(Violation { code: "no_net", severity: "warning", message: "A track belongs to no net".into(), at: t.points[0], layer: Some(t.layer.clone()), objects: vec![t.id], refs: vec!["track".into()] });
        }
    }

    // Holes and rings.
    for p in &v.pads {
        let Some(d) = p.drill else { continue };
        let label = format!("{}.{}", p.reference, if p.number.is_empty() { "hole" } else { &p.number });
        let small = d[0].min(d[1]);
        if p.kind == PadKind::Tht {
            if small + 1e-9 < rules.min_drill {
                out.push(Violation { code: "drill", severity: "error", message: format!("{label}: hole ø{small:.2} mm is below the minimum ø{:.2} mm", rules.min_drill), at: p.at, layer: None, objects: vec![p.component], refs: vec![label.clone()] });
            }
            let ring = ((p.size[0] - d[0]).min(p.size[1] - d[1])) / 2.0;
            if ring + 1e-9 < rules.min_annular {
                out.push(Violation { code: "annular_ring", severity: "error", message: format!("{label}: annular ring {ring:.3} mm is below {:.3} mm", rules.min_annular), at: p.at, layer: None, objects: vec![p.component], refs: vec![label] });
            }
        }
    }
    for via in &v.vias {
        let label = format!("via on {}", via.net.as_deref().unwrap_or("no net"));
        if via.kind == "micro" {
            continue; // held to the micro via limits (drc_extra)
        }
        if via.drill + 1e-9 < rules.min_drill {
            out.push(Violation { code: "drill", severity: "error", message: format!("{label}: drill ø{:.2} mm is below the minimum ø{:.2} mm", via.drill, rules.min_drill), at: via.at, layer: None, objects: vec![via.id], refs: vec![label.clone()] });
        }
        let ring = (via.diameter - via.drill) / 2.0;
        if ring + 1e-9 < rules.min_annular {
            out.push(Violation { code: "annular_ring", severity: "error", message: format!("{label}: annular ring {ring:.3} mm is below {:.3} mm", rules.min_annular), at: via.at, layer: None, objects: vec![via.id], refs: vec![label] });
        }
    }

    // Board edge.
    let edges: Vec<&Vec<P>> = std::iter::once(&v.outline).chain(v.cutouts.iter()).collect();
    let in_board = |p: P| geom::in_polygon(p, &v.outline) && !v.cutouts.iter().any(|c| geom::in_polygon(p, c));
    let mut edge_seen: BTreeSet<String> = BTreeSet::new();
    for c in cu {
        let g = edges.iter().map(|e| geom::gap_to_loop(&c.prim, e)).fold(f64::INFINITY, f64::min);
        let inside = in_board(c.prim.center());
        if !inside || g + 1e-6 < rules.edge_clearance {
            let (n, o) = name(c);
            if !edge_seen.insert(n.clone()) {
                continue;
            }
            let (code, message) = if !inside { ("outside_board", format!("{n} is outside the board")) } else { ("edge_clearance", format!("{n} is {:.3} mm from the board edge; needs {:.3} mm", g.max(0.0), rules.edge_clearance)) };
            out.push(Violation { code, severity: "error", message, at: c.prim.center(), layer: layer_name(c.layers), objects: vec![o], refs: vec![n] });
        }
    }
    for h in &v.holes {
        let prim = Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0);
        let g = edges.iter().map(|e| geom::gap_to_loop(&prim, e)).fold(f64::INFINITY, f64::min);
        if (!in_board(h.at) || g + 1e-6 < rules.edge_clearance) && !edge_seen.contains(&h.owner) {
            edge_seen.insert(h.owner.clone());
            let objects = v.pads.iter().find(|p| format!("{}.{}", p.reference, p.number) == h.owner || p.reference == h.owner).map(|p| vec![p.component]).unwrap_or_default();
            out.push(Violation { code: "edge_clearance", severity: "error", message: format!("Hole {} is {:.3} mm from the board edge; needs {:.3} mm", h.owner, g.max(0.0), rules.edge_clearance), at: h.at, layer: None, objects, refs: vec![h.owner.clone()] });
        }
    }

    // Hole to hole.
    for i in 0..v.holes.len() {
        for j in i + 1..v.holes.len() {
            let (a, b) = (&v.holes[i], &v.holes[j]);
            if geom::dist(a.at, b.at) > a.size[0].max(a.size[1]) + b.size[0].max(b.size[1]) + rules.hole_to_hole {
                continue;
            }
            let pa = Prim::rect(a.at, a.size[0], a.size[1], a.rotation, a.size[0].min(a.size[1]) / 2.0);
            let pb = Prim::rect(b.at, b.size[0], b.size[1], b.rotation, b.size[0].min(b.size[1]) / 2.0);
            let g = geom::gap(&pa, &pb);
            if g + 1e-6 < rules.hole_to_hole {
                out.push(Violation { code: "hole_to_hole", severity: "error", message: format!("Holes {} and {} are {:.3} mm apart; need {:.3} mm", a.owner, b.owner, g.max(0.0), rules.hole_to_hole), at: gap_point(&pa, &pb), layer: None, objects: vec![], refs: vec![a.owner.clone(), b.owner.clone()] });
            }
        }
    }

    // Courtyards.
    if rules.courtyards {
        for i in 0..v.components.len() {
            for j in i + 1..v.components.len() {
                let (a, b) = (&v.components[i], &v.components[j]);
                let both_sides = a.kind == "hole" || b.kind == "hole";
                if (a.side != b.side && !both_sides) || a.courtyard.len() < 3 || b.courtyard.len() < 3 {
                    continue;
                }
                // Through-hole parts take room on both sides only through their pins: courtyards on one side.
                if geom::convex_overlap(&a.courtyard, &b.courtyard) {
                    let c = geom::convex_hull(&a.courtyard.iter().chain(b.courtyard.iter()).cloned().filter(|p| geom::in_convex(*p, &a.courtyard) && geom::in_convex(*p, &b.courtyard)).collect::<Vec<_>>());
                    let at = if c.is_empty() { [(a.at[0] + b.at[0]) / 2.0, (a.at[1] + b.at[1]) / 2.0] } else { Prim { pts: c, r: 0.0 }.center() };
                    out.push(Violation { code: "courtyard_overlap", severity: "error", message: format!("{} and {} overlap (courtyards)", a.reference, b.reference), at, layer: Some(if a.side == Side::Bottom { "bottom".into() } else { "top".into() }), objects: vec![a.id, b.id], refs: vec![a.reference.clone(), b.reference.clone()] });
                }
            }
        }
    }

    // Single-pad nets.
    for n in &v.nets {
        if n.pins.len() == 1 && !v.zones.iter().any(|z| z.net == n.name) {
            let pin = &n.pins[0];
            if let Some(p) = v.pads.iter().find(|p| format!("{}.{}", p.reference, p.number) == *pin) {
                out.push(Violation { code: "single_pad_net", severity: "warning", message: format!("{} has only one pad ({pin})", n.name), at: p.at, layer: None, objects: vec![p.component], refs: vec![pin.clone()] });
            }
        }
    }

    // Thermal reliefs: a pad held by fewer than two spokes joins its pour weakly (or not at all).
    for z in &v.zones {
        for r in z.reliefs.iter().filter(|r| r.spokes < 2) {
            let message = if r.spokes == 0 { format!("{} has no thermal spoke into the {} pour on {} (no room): route it, or join it solid", r.pad, z.net, z.layer) } else { format!("{} joins the {} pour on {} by 1 spoke (2 or more is sound)", r.pad, z.net, z.layer) };
            out.push(Violation { code: "thermal_relief", severity: "warning", message, at: r.at, layer: Some(z.layer.clone()), objects: vec![r.component, z.id], refs: vec![r.pad.clone()] });
        }
    }

    crate::drc_extra::check(board, v, cu, &custom, &mut out);
    crate::signal::check(board, v, &mut out);

    if rules.silk_height < 0.8 {
        out.push(Violation { code: "silk_text", severity: "warning", message: format!("Silkscreen text {:.2} mm tall is below what most fabs print legibly (0.8–1.0 mm)", rules.silk_height), at: [v.bounds[0], v.bounds[1]], layer: None, objects: vec![], refs: vec![] });
    }

    v.violations.extend(out);
    v.violations.sort_by(|a, b| (a.severity != "error").cmp(&(b.severity != "error")).then(a.code.cmp(b.code)));
    v.stats.smallest_gap = smallest;
    let _ = smallest;
}
