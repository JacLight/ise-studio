//! What the 3D view needs to draw a board as it will come from the fab and
//! the assembler: per side, the copper (under the mask: tracks, pours, vias;
//! exposed: the pads, in the finish), the mask openings and the silkscreen
//! (outlines and designators as strokes), the holes, and every component's
//! body solids (from `bodies`). `GET /pcb/<id>/3d` returns it.

use crate::board::BoardView;
use crate::footprints::PadKind;
use crate::geom::{P, Prim};
use crate::{Board, Side};
use de_core::{StableId, Value, bevy_ecs::world::World, json};

fn r(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

fn poly(p: &[P]) -> Value {
    Value::Array(p.iter().map(|q| json!([r(q[0]), r(q[1])])).collect())
}

pub fn scene(world: &World, id: StableId) -> Option<Value> {
    let view = crate::view(world, id)?;
    let board = world.resource::<de_core::IdIndex>().get(id).and_then(|e| world.get::<Board>(e)).cloned()?;
    let items = crate::items(world, id);
    Some(scene_of(&view, &board, &items))
}

pub fn scene_of(view: &BoardView, board: &Board, items: &crate::Items) -> Value {
    let e = view.rules.mask_expansion;
    let nl = view.layers.len();
    let mut sides = serde_json::Map::new();
    for (side, layer) in [("top", view.layers[0].clone()), ("bottom", view.layers[nl - 1].clone())] {
        let on = |p: &crate::board::PadView| p.layers.contains(&layer);
        let pads: Vec<Value> = view.pads.iter().filter(|p| on(p)).map(|p| poly(&p.polygon.clone().unwrap_or_else(|| p.prim.outline(0.01)))).collect();
        let openings: Vec<Value> = view
            .pads
            .iter()
            .filter(|p| on(p) || p.kind == PadKind::Npth)
            .map(|p| match &p.polygon {
                Some(pg) => poly(pg),
                None => poly(&Prim { pts: p.prim.pts.clone(), r: p.prim.r + if p.kind == PadKind::Npth { 0.0 } else { e } }.outline(0.01)),
            })
            .collect();
        let mut tracks: Vec<Value> = view.tracks.iter().filter(|t| t.layer == layer).map(|t| json!({ "w": r(t.width), "p": poly(&t.points) })).collect();
        let mut zones: Vec<Value> = view.zones.iter().filter(|z| z.layer == layer).flat_map(|z| z.fill.iter().map(|island| Value::Array(island.iter().map(|c| poly(c)).collect()))).collect();
        // Graphics on the copper (lettering, logos, filled shapes) are copper too: strokes drawn
        // as tracks of their width, filled shapes as pours.
        for gv in view.graphics.iter().filter(|g| g.layer == layer) {
            tracks.extend(gv.strokes.iter().filter(|l| l.len() >= 2).map(|l| json!({ "w": r(gv.width), "p": poly(l) })));
            zones.extend(gv.fills.iter().map(|island| Value::Array(island.iter().map(|c| poly(c)).collect())));
        }
        let s = if side == "top" { Side::Top } else { Side::Bottom };
        let mut silk: Vec<Value> = Vec::new();
        for c in view.components.iter().filter(|c| c.side == s) {
            for l in &c.silk {
                silk.push(poly(l));
            }
            if c.kind != "hole" {
                for l in crate::font::strokes_turned(&c.reference, c.label, view.rules.silk_height, s == Side::Bottom, c.label_rot) {
                    silk.push(poly(&l));
                }
            }
        }
        // The board's own legend (names, labels, logos) as the Gerber has it: strokes, and filled
        // shapes (outline-font lettering, images) as islands of contours.
        let legend = if side == "top" { "silk_top" } else { "silk_bottom" };
        let mut silk_fill: Vec<Value> = Vec::new();
        for gv in view.graphics.iter().filter(|g| g.layer == legend) {
            for l in &gv.strokes {
                silk.push(poly(l));
            }
            for island in &gv.fills {
                silk_fill.push(Value::Array(island.iter().map(|c| poly(c)).collect()));
            }
        }
        sides.insert(side.into(), json!({ "pads": pads, "openings": openings, "tracks": tracks, "zones": zones, "silk": silk, "silk_fill": silk_fill }));
    }
    let bodies = crate::bodies::bodies(board, &items.components);
    // Parts not placed (DNP) drawn on request: their bodies apart.
    let unplaced: Vec<(StableId, crate::PcbComponent)> = items.components.iter().filter(|(_, c)| c.dnp).map(|(id, c)| (*id, crate::PcbComponent { dnp: false, ..c.clone() })).collect();
    let dnp_bodies = crate::bodies::bodies(board, &unplaced);
    // What each part is, for showing and hiding by type in the 3D view.
    let parts: Vec<Value> = view
        .components
        .iter()
        .map(|c| {
            let pads: Vec<&crate::board::PadView> = view.pads.iter().filter(|p| p.reference == c.reference).collect();
            let kind = if pads.iter().any(|p| p.kind == PadKind::Tht) { "tht" } else if pads.iter().any(|p| p.kind == PadKind::Smd) { "smd" } else { "other" };
            json!({ "reference": c.reference, "value": c.value, "footprint": c.footprint, "kind": kind, "dnp": c.dnp, "side": c.side })
        })
        .collect();
    json!({
        "id": view.id,
        "thickness": view.thickness,
        "mask": view.mask,
        "silk": view.silk,
        "finish": view.finish,
        "outline": poly(&view.outline),
        "cutouts": view.cutouts.iter().map(|c| poly(c)).collect::<Vec<_>>(),
        "bounds": view.bounds,
        "silk_width": view.rules.silk_width,
        "vias": view.vias.iter().map(|v| json!([r(v.at[0]), r(v.at[1]), r(v.diameter), r(v.drill)])).collect::<Vec<_>>(),
        "holes": view.holes.iter().map(|h| json!({ "at": [r(h.at[0]), r(h.at[1])], "size": [r(h.size[0]), r(h.size[1])], "rot": r(h.rotation), "plated": h.plated })).collect::<Vec<_>>(),
        "sides": sides,
        "bodies": bodies,
        "dnp_bodies": dnp_bodies,
        "parts": parts,
    })
}
