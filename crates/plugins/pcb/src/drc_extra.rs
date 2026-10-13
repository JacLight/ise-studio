//! Design-rule checks for blind, buried and micro vias, keepouts, placement
//! rule areas, the stackup, board graphics and the custom rules language
//! (`rules`): what `drc::check` adds on top of the board minima and net
//! classes.

use crate::board::{BoardView, Cu, Owner, Violation};
use crate::footprints::PadKind;
use crate::geom::{self, P, Prim};
use crate::rules::{Item, RuleSet};
use crate::{Board, Side};
use de_core::StableId;
use std::collections::{BTreeMap, BTreeSet};

/// Named areas conditions can test: rule areas and named zones.
fn named_areas(v: &BoardView) -> Vec<(String, Vec<P>)> {
    let mut out: Vec<(String, Vec<P>)> = v.areas.iter().map(|a| (a.name.clone(), a.outline.clone())).collect();
    out.extend(v.zones.iter().filter_map(|z| z.name.clone().map(|n| (n, z.outline.clone()))));
    out
}

fn prim_inside(p: &Prim, poly: &[P]) -> bool {
    p.pts.iter().all(|q| geom::in_polygon(*q, poly)) && geom::gap_to_loop(p, poly) >= 0.0
}

fn prim_touches(p: &Prim, poly: &[P]) -> bool {
    geom::in_polygon(p.center(), poly) || p.pts.iter().any(|q| geom::in_polygon(*q, poly)) || geom::gap_to_loop(p, poly) <= 1e-9
}

fn poly_touches(a: &[P], poly: &[P]) -> bool {
    if a.is_empty() {
        return false;
    }
    a.iter().any(|q| geom::in_polygon(*q, poly)) || poly.iter().any(|q| geom::in_polygon(*q, a)) || {
        let n = a.len();
        (0..n).any(|i| geom::dist_to_loop(a[i], poly) <= 1e-9 || poly.iter().any(|q| geom::point_seg_dist(*q, a[i], a[(i + 1) % n]) <= 1e-9))
    }
}

fn layer_names(v: &BoardView, mask: u32) -> Vec<String> {
    (0..v.layers.len()).filter(|k| mask & (1 << k) != 0).map(|k| v.layers[k].clone()).collect()
}

/// The custom rules with every copper piece described for them.
pub struct Custom {
    set: Option<RuleSet>,
    items: Vec<Item>,
}

impl Custom {
    pub fn new(board: &Board, v: &BoardView, cu: &[Cu]) -> Custom {
        let set = crate::rules::parse(&board.drc_rules).ok().filter(|s| !s.rules.is_empty());
        let Some(_) = &set else { return Custom { set: None, items: vec![] } };
        let areas = named_areas(v);
        let track_ix: BTreeMap<StableId, usize> = v.tracks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let via_ix: BTreeMap<StableId, usize> = v.vias.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let comp_ix: BTreeMap<StableId, usize> = v.components.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let gfx_ix: BTreeMap<StableId, usize> = v.graphics.iter().enumerate().map(|(i, g)| (g.id, i)).collect();
        let items = cu
            .iter()
            .map(|c| {
                let mut it = Item { net: c.net.clone(), netclass: Some(board.class_of(c.net.as_deref()).name), layers: layer_names(v, c.layers), ..Default::default() };
                match &c.owner {
                    Owner::Pad(i) => {
                        let p = &v.pads[*i];
                        it.kind = "pad";
                        it.reference = Some(p.reference.clone());
                        it.footprint = comp_ix.get(&p.component).map(|k| v.components[*k].footprint.clone());
                        it.pad_type = Some(match p.kind {
                            PadKind::Smd => "smd",
                            PadKind::Tht => "tht",
                            PadKind::Npth => "npth",
                        });
                        it.hole = p.drill.map(|d| d[0].min(d[1]));
                        it.plated = p.kind == PadKind::Tht;
                        it.width = Some(p.size[0].min(p.size[1]));
                    }
                    Owner::Track(id, _) => {
                        it.kind = "track";
                        it.width = track_ix.get(id).map(|k| v.tracks[*k].width);
                    }
                    Owner::Via(id) => {
                        it.kind = "via";
                        if let Some(k) = via_ix.get(id) {
                            let x = &v.vias[*k];
                            it.via_type = Some(x.kind);
                            it.hole = Some(x.drill);
                            it.width = Some(x.diameter);
                            it.layers = v.layers.iter().skip_while(|l| **l != x.from).take_while(|l| **l != x.to).cloned().chain(std::iter::once(x.to.clone())).collect();
                        }
                        it.plated = true;
                    }
                    Owner::Graphic(id) => {
                        it.kind = if gfx_ix.get(id).is_some_and(|k| v.graphics[*k].kind == "text") { "text" } else { "graphic" };
                        it.width = gfx_ix.get(id).map(|k| v.graphics[*k].width);
                    }
                }
                for (name, poly) in &areas {
                    if prim_inside(&c.prim, poly) {
                        it.inside.push(name.clone());
                    } else if prim_touches(&c.prim, poly) {
                        it.touches.push(name.clone());
                    }
                }
                it
            })
            .collect();
        Custom { set, items }
    }

    pub fn max_clearance(&self) -> f64 {
        self.set.as_ref().map_or(0.0, |s| s.max_clearance())
    }

    /// The clearance a custom rule asks between copper `i` and `j` on `layer`, and the rule's name.
    pub fn clearance(&self, i: usize, j: usize, layer: Option<&str>) -> Option<(f64, String)> {
        let s = self.set.as_ref()?;
        let h = s.pair("clearance", &self.items[i], &self.items[j], layer)?;
        if h.rule.severity == "ignore" {
            return Some((0.0, h.rule.name.clone()));
        }
        Some((h.constraint.min?, h.rule.name.clone()))
    }
}

fn sev(s: &str) -> &'static str {
    if s == "warning" { "warning" } else { "error" }
}

fn push(out: &mut Vec<Violation>, code: &'static str, severity: &'static str, message: String, at: P, layer: Option<String>, objects: Vec<StableId>, refs: Vec<String>) {
    out.push(Violation { code, severity, message, at, layer, objects, refs });
}

fn hole_prim(h: &crate::board::Hole) -> Prim {
    Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0)
}

pub fn check(board: &Board, v: &BoardView, cu: &[Cu], custom: &Custom, out: &mut Vec<Violation>) {
    let n = v.layers.len();
    let rules = &board.rules;
    let idx = |l: &str| crate::board::layer_index(&v.layers, l).unwrap_or(0);

    // Blind, buried and micro vias.
    for via in v.vias.iter().filter(|x| x.kind != "through") {
        let (a, b) = (idx(&via.from), idx(&via.to));
        let (lo, hi) = (a.min(b), a.max(b));
        let label = format!("{} via on {} ({} → {})", via.kind, via.net.as_deref().unwrap_or("no net"), via.from, via.to);
        let mut bad = |code: &'static str, severity: &'static str, msg: String| push(out, code, severity, msg, via.at, None, vec![via.id], vec![label.clone()]);
        let outer = (lo == 0) as u8 + (hi == n - 1) as u8;
        match via.kind {
            "blind" if outer == 2 => bad("via_span", "error", format!("{label} goes through every layer: make it a through via")),
            "blind" if outer == 0 => bad("via_span", "error", format!("{label} reaches no outer layer: that is a buried via")),
            "buried" if outer > 0 => bad("via_span", "error", format!("{label} reaches an outer layer: that is a blind via")),
            "micro" => {
                if hi - lo != 1 {
                    bad("via_span", "error", format!("{label} crosses {} dielectrics; a micro via crosses one (stack them for more)", hi - lo));
                }
                if v.stackup.crosses_core(lo, hi) {
                    bad("microvia_crosses_core", "error", format!("{label} crosses a core; laser micro vias only cross prepreg"));
                }
                if via.drill + 1e-9 < rules.min_microvia_drill {
                    bad("microvia_drill", "error", format!("{label}: drill ø{:.3} mm is below the micro via minimum ø{:.3} mm", via.drill, rules.min_microvia_drill));
                }
                if via.drill > 0.2 + 1e-9 {
                    bad("microvia_drill", "warning", format!("{label}: ø{:.3} mm is larger than a laser drills (0.2 mm at most): make it a blind via", via.drill));
                }
                if via.diameter + 1e-9 < rules.min_microvia_diameter {
                    bad("microvia_drill", "error", format!("{label}: pad ø{:.3} mm is below the micro via minimum ø{:.3} mm", via.diameter, rules.min_microvia_diameter));
                }
                let depth: f64 = v.stackup.dielectrics.get(lo..hi).map(|d| d.iter().map(|x| x.thickness).sum()).unwrap_or(0.0);
                if depth > via.drill * 1.0 + 1e-9 {
                    bad("microvia_aspect", "warning", format!("{label}: {depth:.3} mm deep for a ø{:.3} mm hole (aspect over 1:1 plates poorly)", via.drill));
                }
            }
            _ => {}
        }
        for (l, d) in &via.stack {
            let k = idx(l);
            if *d > 0.0 && (k < lo || k > hi) {
                bad("via_stack", "warning", format!("{label}: a pad on {l}, which the via does not reach"));
            }
        }
    }
    // Micro vias stacked on one another must be filled.
    let micros: Vec<&crate::board::ViaView> = v.vias.iter().filter(|x| x.kind == "micro").collect();
    for i in 0..micros.len() {
        for j in i + 1..micros.len() {
            let (a, b) = (micros[i], micros[j]);
            if geom::dist(a.at, b.at) < (a.drill + b.drill) / 2.0 && (a.to == b.from || b.to == a.from) {
                push(out, "microvia_stack", "warning", format!("Micro vias at ({:.2}, {:.2}) are stacked: order them copper-filled", a.at[0], a.at[1]), a.at, None, vec![a.id, b.id], vec!["micro via".into(), "micro via".into()]);
            }
        }
    }

    // Keepouts.
    let mut seen: BTreeSet<(StableId, StableId)> = BTreeSet::new();
    for a in v.areas.iter().filter(|a| a.keepout() && a.outline.len() >= 3) {
        for c in cu {
            if c.layers & a.mask == 0 {
                continue;
            }
            let (what, obj, allowed) = match &c.owner {
                Owner::Track(id, _) => ("track", *id, !a.no_tracks),
                Owner::Via(id) => ("via", *id, !a.no_vias),
                Owner::Pad(i) => ("pad", v.pads[*i].component, !a.no_pads),
                Owner::Graphic(_) => continue,
            };
            if allowed || !prim_touches(&c.prim, &a.outline) || !seen.insert((obj, a.id)) {
                continue;
            }
            let name = match &c.owner {
                Owner::Pad(i) => format!("{}.{}", v.pads[*i].reference, v.pads[*i].number),
                _ => format!("{what} on {}", c.net.as_deref().unwrap_or("no net")),
            };
            push(out, "keepout", "error", format!("{name} is inside keepout “{}” (no {what}s)", a.name), c.prim.center(), layer_names(v, c.layers & a.mask).first().cloned(), vec![obj, a.id], vec![name, a.name.clone()]);
        }
        if a.no_footprints {
            for c in &v.components {
                let side_layer = if c.side == Side::Bottom { v.layers[n - 1].as_str() } else { "top" };
                if !a.layers.iter().any(|l| l == side_layer) || !poly_touches(&c.courtyard, &a.outline) {
                    continue;
                }
                if seen.insert((c.id, a.id)) {
                    push(out, "keepout", "error", format!("{} is inside keepout “{}” (no footprints)", c.reference, a.name), c.at, Some(side_layer.to_string()), vec![c.id, a.id], vec![c.reference.clone(), a.name.clone()]);
                }
            }
        }
    }
    // Placement rule areas: their components belong inside.
    for a in v.areas.iter().filter(|a| !a.placement.is_empty()) {
        for c in v.components.iter().filter(|c| a.placement.iter().any(|r| crate::rules::wild(r, &c.reference))) {
            let pts = if c.courtyard.is_empty() { vec![c.at] } else { c.courtyard.clone() };
            if !pts.iter().all(|p| geom::in_polygon(*p, &a.outline)) {
                push(out, "placement_area", "warning", format!("{} is outside its placement area “{}”", c.reference, a.name), c.at, None, vec![c.id, a.id], vec![c.reference.clone(), a.name.clone()]);
            }
        }
    }

    // Stackup against the finished thickness.
    let t = v.stackup.total;
    if (t - v.thickness).abs() > (0.1f64).max(v.thickness * 0.1) {
        push(out, "stackup_thickness", "warning", format!("The stackup adds up to {t:.3} mm but the board is {:.2} mm thick (pcb.stackup.set fit_thickness, or change the dielectrics)", v.thickness), [v.bounds[0], v.bounds[1]], None, vec![v.id], vec!["stackup".into()]);
    }

    // Silkscreen graphics: legible text, on the board.
    for g in v.graphics.iter().filter(|g| g.layer.starts_with("silk")) {
        if g.kind == "text" && g.fills.is_empty() && g.width + 1e-9 < rules.silk_width {
            push(out, "silk_text", "warning", format!("Silkscreen text “{}” is drawn {:.3} mm wide; fabs print {:.2} mm or more", g.text.clone().unwrap_or_default(), g.width, rules.silk_width), [g.bbox[0], g.bbox[1]], Some(g.layer.clone()), vec![g.id], vec![g.text.clone().unwrap_or_default()]);
        }
        let corners = [[g.bbox[0], g.bbox[1]], [g.bbox[2], g.bbox[1]], [g.bbox[2], g.bbox[3]], [g.bbox[0], g.bbox[3]]];
        if !corners.iter().all(|p| geom::in_polygon(*p, &v.outline)) {
            push(out, "silk_edge", "warning", format!("A silkscreen {} runs off the board (it is not printed there)", g.kind), [(g.bbox[0] + g.bbox[2]) / 2.0, (g.bbox[1] + g.bbox[3]) / 2.0], Some(g.layer.clone()), vec![g.id], vec![g.kind.to_string()]);
        }
    }

    custom_rules(board, v, cu, custom, out);
}

fn custom_rules(board: &Board, v: &BoardView, cu: &[Cu], custom: &Custom, out: &mut Vec<Violation>) {
    let Some(set) = &custom.set else { return };
    let objects = |c: &Cu| -> (StableId, String) {
        match &c.owner {
            Owner::Pad(i) => (v.pads[*i].component, format!("{}.{}", v.pads[*i].reference, v.pads[*i].number)),
            Owner::Track(id, _) => (*id, format!("track on {}", c.net.as_deref().unwrap_or("no net"))),
            Owner::Via(id) => (*id, format!("via on {}", c.net.as_deref().unwrap_or("no net"))),
            Owner::Graphic(id) => (*id, format!("copper graphic on {}", c.net.as_deref().unwrap_or("no net"))),
        }
    };
    let edges: Vec<&Vec<P>> = std::iter::once(&v.outline).chain(v.cutouts.iter()).collect();
    let mut once: BTreeSet<(StableId, String, &'static str)> = BTreeSet::new();
    for (i, c) in cu.iter().enumerate() {
        let it = &custom.items[i];
        let (obj, name) = objects(c);
        let layer = it.layers.first().cloned();
        // Disallowed items.
        for r in set.disallowed(it) {
            if r.severity != "ignore" && once.insert((obj, r.name.clone(), "disallow")) {
                push(out, "disallow", sev(&r.severity), format!("{name}: {}s are not allowed here (rule “{}”)", it.kind, r.name), c.prim.center(), layer.clone(), vec![obj], vec![name.clone()]);
            }
        }
        let mut range = |kind: &str, code: &'static str, what: &str, value: Option<f64>| {
            let Some(x) = value else { return };
            let Some(h) = set.single(kind, it, layer.as_deref()) else { return };
            if h.rule.severity == "ignore" || !once.insert((obj, h.rule.name.clone(), code)) {
                return;
            }
            let lo = h.constraint.min.is_some_and(|m| x + 1e-9 < m);
            let hi = h.constraint.max.is_some_and(|m| x - 1e-9 > m);
            if lo || hi {
                let limit = if lo { format!("at least {:.3}", h.constraint.min.unwrap()) } else { format!("at most {:.3}", h.constraint.max.unwrap()) };
                push(out, code, sev(&h.rule.severity), format!("{name}: {what} {x:.3} mm; rule “{}” asks {limit} mm", h.rule.name), c.prim.center(), layer.clone(), vec![obj], vec![name.clone()]);
            }
        };
        match it.kind {
            "track" => range("track_width", "track_width", "width", it.width),
            "via" => {
                range("via_diameter", "via_diameter", "diameter", it.width);
                range("hole_size", "hole_size", "hole", it.hole);
                range("annular_width", "annular_ring", "annular ring", it.width.zip(it.hole).map(|(d, h)| (d - h) / 2.0));
            }
            "pad" => {
                range("hole_size", "hole_size", "hole", it.hole);
                if it.plated {
                    range("annular_width", "annular_ring", "annular ring", it.width.zip(it.hole).map(|(d, h)| (d - h) / 2.0));
                }
            }
            _ => {}
        }
        // Edge clearance by rule.
        if let Some(h) = set.single("edge_clearance", it, layer.as_deref()) {
            if let Some(min) = h.constraint.min.filter(|_| h.rule.severity != "ignore") {
                let g = edges.iter().map(|e| geom::gap_to_loop(&c.prim, e)).fold(f64::INFINITY, f64::min);
                if g + 1e-6 < min && once.insert((obj, h.rule.name.clone(), "edge_clearance")) {
                    push(out, "edge_clearance", sev(&h.rule.severity), format!("{name} is {:.3} mm from the board edge; rule “{}” asks {min:.3} mm", g.max(0.0), h.rule.name), c.prim.center(), layer.clone(), vec![obj], vec![name.clone()]);
                }
            }
        }
    }

    // Components and zones: disallow footprints / zones, courtyard clearance.
    let areas = named_areas(v);
    let comp_item = |c: &crate::board::ComponentView| -> Item {
        let mut it = Item { kind: "footprint", reference: Some(c.reference.clone()), footprint: Some(c.footprint.clone()), layers: vec![if c.side == Side::Bottom { "bottom".into() } else { "top".into() }], ..Default::default() };
        for (name, poly) in &areas {
            if !c.courtyard.is_empty() && c.courtyard.iter().all(|p| geom::in_polygon(*p, poly)) {
                it.inside.push(name.clone());
            } else if poly_touches(&c.courtyard, poly) {
                it.touches.push(name.clone());
            }
        }
        it
    };
    let comps: Vec<Item> = v.components.iter().map(comp_item).collect();
    for (c, it) in v.components.iter().zip(&comps) {
        for r in set.disallowed(it) {
            if r.severity != "ignore" {
                push(out, "disallow", sev(&r.severity), format!("{}: footprints are not allowed here (rule “{}”)", c.reference, r.name), c.at, None, vec![c.id], vec![c.reference.clone()]);
            }
        }
    }
    for z in &v.zones {
        let mut it = Item { kind: "zone", net: Some(z.net.clone()), netclass: Some(board.class_of(Some(&z.net)).name), layers: vec![z.layer.clone()], name: z.name.clone(), ..Default::default() };
        for (name, poly) in &areas {
            if Some(name) != z.name.as_ref() && poly_touches(&z.outline, poly) {
                it.touches.push(name.clone());
            }
        }
        for r in set.disallowed(&it) {
            if r.severity != "ignore" && z.area > 0.0 {
                push(out, "disallow", sev(&r.severity), format!("The {} pour on {} is not allowed here (rule “{}”)", z.net, z.layer, r.name), z.outline.first().copied().unwrap_or_default(), Some(z.layer.clone()), vec![z.id], vec![z.net.clone()]);
            }
        }
    }
    if set.rules.iter().any(|r| r.constraints.iter().any(|c| c.kind == "courtyard_clearance")) {
        for i in 0..v.components.len() {
            for j in i + 1..v.components.len() {
                let (a, b) = (&v.components[i], &v.components[j]);
                if a.side != b.side || a.courtyard.len() < 3 || b.courtyard.len() < 3 {
                    continue;
                }
                let Some(h) = set.pair("courtyard_clearance", &comps[i], &comps[j], None) else { continue };
                let Some(min) = h.constraint.min.filter(|_| h.rule.severity != "ignore") else { continue };
                let (pa, pb) = (Prim { pts: a.courtyard.clone(), r: 0.0 }, Prim { pts: b.courtyard.clone(), r: 0.0 });
                let g = if geom::convex_overlap(&a.courtyard, &b.courtyard) { 0.0 } else { geom::gap(&pa, &pb) };
                if g + 1e-6 < min {
                    push(out, "courtyard_clearance", sev(&h.rule.severity), format!("{} and {} courtyards are {g:.3} mm apart; rule “{}” asks {min:.3} mm", a.reference, b.reference, h.rule.name), crate::drc::gap_point(&pa, &pb), None, vec![a.id, b.id], vec![a.reference.clone(), b.reference.clone()]);
                }
            }
        }
    }

    // Holes: to other nets' copper, and to each other.
    let wants_hc = set.rules.iter().any(|r| r.constraints.iter().any(|c| c.kind == "hole_clearance"));
    let wants_hh = set.rules.iter().any(|r| r.constraints.iter().any(|c| c.kind == "hole_to_hole"));
    if !(wants_hc || wants_hh) {
        return;
    }
    let hole_item = |h: &crate::board::Hole| -> (Item, Option<String>) {
        let net = if h.owner == "via" { v.vias.iter().find(|x| geom::dist(x.at, h.at) < 1e-6).and_then(|x| x.net.clone()) } else { v.pads.iter().find(|p| format!("{}.{}", p.reference, p.number) == h.owner).and_then(|p| p.net.clone()) };
        let mut it = Item { kind: "hole", net: net.clone(), netclass: Some(board.class_of(net.as_deref()).name), layers: v.layers.clone(), hole: Some(h.size[0].min(h.size[1])), plated: h.plated, ..Default::default() };
        for (name, poly) in &areas {
            if geom::in_polygon(h.at, poly) {
                it.inside.push(name.clone());
            }
        }
        (it, net)
    };
    let holes: Vec<(Item, Option<String>, Prim)> = v.holes.iter().map(|h| {
        let (it, net) = hole_item(h);
        (it, net, hole_prim(h))
    }).collect();
    let reach = set.max_clearance();
    if wants_hc {
        for (k, (hi, net, hp)) in holes.iter().enumerate() {
            let hb = hp.bbox();
            for (i, c) in cu.iter().enumerate() {
                if c.net == *net && net.is_some() || !geom::bbox_overlap(&hb, &c.bbox, reach) {
                    continue;
                }
                // A hole's own copper (its pad or via) is not a neighbour.
                if geom::gap(&Prim::circle(v.holes[k].at, 0.0), &c.prim) <= 0.0 && c.net == *net {
                    continue;
                }
                let Some(h) = set.pair("hole_clearance", hi, &custom.items[i], None) else { continue };
                let Some(min) = h.constraint.min.filter(|_| h.rule.severity != "ignore") else { continue };
                let g = geom::gap(hp, &c.prim);
                if g + 1e-6 < min && g > -1e9 {
                    let (obj, name) = objects(c);
                    if once.insert((obj, format!("{}:{}", h.rule.name, v.holes[k].owner), "hole_clearance")) {
                        push(out, "hole_clearance", sev(&h.rule.severity), format!("Hole {} is {:.3} mm from {name}; rule “{}” asks {min:.3} mm", v.holes[k].owner, g.max(0.0), h.rule.name), crate::drc::gap_point(hp, &c.prim), None, vec![obj], vec![v.holes[k].owner.clone(), name]);
                    }
                }
            }
        }
    }
    if wants_hh {
        for i in 0..holes.len() {
            for j in i + 1..holes.len() {
                let Some(h) = set.pair("hole_to_hole", &holes[i].0, &holes[j].0, None) else { continue };
                let Some(min) = h.constraint.min.filter(|_| h.rule.severity != "ignore") else { continue };
                let g = geom::gap(&holes[i].2, &holes[j].2);
                if g + 1e-6 < min {
                    push(out, "hole_to_hole", sev(&h.rule.severity), format!("Holes {} and {} are {:.3} mm apart; rule “{}” asks {min:.3} mm", v.holes[i].owner, v.holes[j].owner, g.max(0.0), h.rule.name), crate::drc::gap_point(&holes[i].2, &holes[j].2), None, vec![], vec![v.holes[i].owner.clone(), v.holes[j].owner.clone()]);
                }
            }
        }
    }
}
