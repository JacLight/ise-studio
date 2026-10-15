//! The populated board as a binary glTF (`.glb`) for renderers, web pages
//! and other 3D tools: the laminate in the mask colour with its holes, the
//! exposed pads in the finish, the silkscreen, and every component body in
//! its look; metres, Y up (glTF's convention), one mesh per material.
//! Flat-shaded: the file carries no normals, which glTF readers then compute
//! per face.

use crate::board::BoardView;
use crate::footprints::PadKind;
use crate::geom::{self, P, Prim};
use crate::{Board, Side};
use manifold_csg::CrossSection;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Colour (sRGB), metalness, roughness of a body look (as the web view draws them).
fn look(name: &str) -> ([f64; 3], f64, f64) {
    let (hex, metal, rough) = match name {
        "epoxy" => (0x1b1b1d, 0.0, 0.6),
        "mark" => (0x3c3c40, 0.0, 0.4),
        "ceramic" => (0xa8875c, 0.0, 0.65),
        "ceramic_white" => (0xe6e1d6, 0.0, 0.6),
        "res_top" => (0x131313, 0.0, 0.75),
        "tin" => (0xd4d7dc, 1.0, 0.26),
        "gold" => (0xe6b85c, 1.0, 0.22),
        "steel" => (0xcdd1d7, 1.0, 0.28),
        "steel_dark" => (0x8f959d, 1.0, 0.4),
        "nylon" => (0xeee5cc, 0.0, 0.55),
        "black" => (0x19191b, 0.0, 0.5),
        "dark" => (0x040404, 0.0, 0.9),
        "module_pcb" => (0x1b2433, 0.0, 0.35),
        "antenna" => (0xd6ad5e, 1.0, 0.3),
        "led_body" => (0xf3f0e7, 0.0, 0.45),
        "led_red" => (0xff3524, 0.0, 0.15),
        "led_green" => (0x3cff6a, 0.0, 0.15),
        "led_blue" => (0x3b7bff, 0.0, 0.15),
        "led_yellow" => (0xffd83a, 0.0, 0.15),
        "led_orange" => (0xff9a2a, 0.0, 0.15),
        "led_white" => (0xf7fbff, 0.0, 0.1),
        _ => (0x1b1b1d, 0.0, 0.6),
    };
    (rgb(hex), metal, rough)
}

fn rgb(hex: u32) -> [f64; 3] {
    [((hex >> 16) & 255) as f64 / 255.0, ((hex >> 8) & 255) as f64 / 255.0, (hex & 255) as f64 / 255.0]
}

fn mask_colour(mask: &str) -> u32 {
    match mask.to_ascii_lowercase().as_str() {
        "black" => 0x0a0b0d,
        "blue" => 0x123c6e,
        "red" => 0x7a1418,
        "white" => 0xe9e9e4,
        "purple" => 0x3b1d63,
        "yellow" => 0xc9a51c,
        "matte_black" | "matte black" => 0x17181a,
        _ => 0x14532f,
    }
}

#[derive(Default)]
struct Mesh {
    pos: Vec<[f32; 3]>,
    idx: Vec<u32>,
}

impl Mesh {
    /// Adds triangles given in board millimetres (z up), stored in glTF metres (y up).
    fn add(&mut self, verts: &[[f64; 3]], tris: &[[u32; 3]]) {
        let base = self.pos.len() as u32;
        self.pos.extend(verts.iter().map(|v| [(v[0] / 1000.0) as f32, (v[2] / 1000.0) as f32, (-v[1] / 1000.0) as f32]));
        self.idx.extend(tris.iter().flat_map(|t| [base + t[0], base + t[1], base + t[2]]));
    }

    fn add_manifold(&mut self, cs: &CrossSection, z0: f64, h: f64) {
        if cs.is_empty() || h <= 0.0 {
            return;
        }
        let (vp, np, tri) = cs.extrude(h).to_mesh_f64();
        let verts: Vec<[f64; 3]> = vp.chunks(np).map(|c| [c[0], c[1], c[2] + z0]).collect();
        let tris: Vec<[u32; 3]> = tri.chunks(3).map(|t| [t[0] as u32, t[1] as u32, t[2] as u32]).collect();
        self.add(&verts, &tris);
    }

    /// A solid of revolution or a box from the component bodies' shapes.
    fn add_solid(&mut self, shape: &str, at: [f64; 3], size: [f64; 3], rot: f64) {
        let local: (Vec<[f64; 3]>, Vec<[u32; 3]>) = match shape {
            "cyl" | "hcyl" | "sphere" => {
                let n = 24u32;
                let mut v = Vec::new();
                let mut t = Vec::new();
                if shape == "sphere" {
                    let rings = 12u32;
                    for i in 0..=rings {
                        let phi = std::f64::consts::PI * i as f64 / rings as f64;
                        for j in 0..n {
                            let th = 2.0 * std::f64::consts::PI * j as f64 / n as f64;
                            v.push([0.5 * phi.sin() * th.cos() * size[0], 0.5 * phi.sin() * th.sin() * size[1], (0.5 - 0.5 * phi.cos()) * size[2]]);
                        }
                    }
                    for i in 0..rings {
                        for j in 0..n {
                            let (a, b, c, d) = (i * n + j, i * n + (j + 1) % n, (i + 1) * n + j, (i + 1) * n + (j + 1) % n);
                            t.push([a, c, b]);
                            t.push([b, c, d]);
                        }
                    }
                } else {
                    // A unit cylinder along z, then laid along y for "hcyl".
                    for k in 0..2 {
                        for j in 0..n {
                            let th = 2.0 * std::f64::consts::PI * j as f64 / n as f64;
                            v.push([0.5 * th.cos(), 0.5 * th.sin(), k as f64]);
                        }
                    }
                    v.push([0.0, 0.0, 0.0]);
                    v.push([0.0, 0.0, 1.0]);
                    let (c0, c1) = (2 * n, 2 * n + 1);
                    for j in 0..n {
                        let (a, b) = (j, (j + 1) % n);
                        t.push([a, b, n + b]);
                        t.push([a, n + b, n + a]);
                        t.push([c0, b, a]);
                        t.push([c1, n + a, n + b]);
                    }
                    for p in &mut v {
                        *p = if shape == "hcyl" {
                            // Axis along local y, sitting on z: diameter size[0], length size[1], height size[2].
                            [p[0] * size[0], (p[2] - 0.5) * size[1], (p[1] + 0.5) * size[2]]
                        } else {
                            [p[0] * size[0], p[1] * size[1], p[2] * size[2]]
                        };
                    }
                }
                (v, t)
            }
            _ => {
                let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
                let v = vec![[-hx, -hy, 0.0], [hx, -hy, 0.0], [hx, hy, 0.0], [-hx, hy, 0.0], [-hx, -hy, size[2]], [hx, -hy, size[2]], [hx, hy, size[2]], [-hx, hy, size[2]]];
                let t = vec![[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
                (v, t)
            }
        };
        let (s, c) = rot.to_radians().sin_cos();
        let verts: Vec<[f64; 3]> = local.0.iter().map(|p| [at[0] + p[0] * c - p[1] * s, at[1] + p[0] * s + p[1] * c, at[2] + p[2]]).collect();
        self.add(&verts, &local.1);
    }
}

fn union(polys: Vec<Vec<P>>) -> CrossSection {
    let parts: Vec<CrossSection> = polys.into_iter().filter(|p| p.len() >= 3).map(|p| CrossSection::from_polygons(&[p])).collect();
    if parts.is_empty() { CrossSection::empty() } else { CrossSection::batch_union(&parts) }
}

/// The board as GLB bytes.
pub fn glb(view: &BoardView, board: &Board, items: &crate::Items) -> Vec<u8> {
    let t = view.thickness;
    let tol = 0.02;
    // Laminate: outline less cutouts, holes and vias.
    let mut cut: Vec<Vec<P>> = view.cutouts.iter().map(|c| if geom::polygon_area(c) < 0.0 { c.iter().rev().copied().collect() } else { c.clone() }).collect();
    for h in &view.holes {
        let w = h.size[0].min(h.size[1]);
        cut.push(Prim::rect(h.at, h.size[0], h.size[1], h.rotation, w / 2.0).outline(tol));
    }
    for v in &view.vias {
        cut.push(Prim::circle(v.at, v.drill / 2.0).outline(tol));
    }
    let holes = union(cut.clone());
    let laminate = CrossSection::from_polygons(&[view.outline.clone()]).difference(&holes);
    let mask = (rgb(mask_colour(&view.mask)), 0.0, 0.35);
    let mut board_mesh = Mesh::default();
    board_mesh.add_manifold(&laminate, 0.0, t);
    let finish = if view.finish.to_ascii_uppercase().contains("ENIG") { (rgb(0xe2b65a), 1.0, 0.22) } else { (rgb(0xc9ccd0), 1.0, 0.3) };
    let silk = (if view.silk.to_ascii_lowercase().contains("black") { rgb(0x111111) } else { rgb(0xf4f4f0) }, 0.0, 0.6);
    let mut copper = Mesh::default();
    let mut ink = Mesh::default();
    let nl = view.layers.len();
    for (side, layer) in [(Side::Top, &view.layers[0]), (Side::Bottom, &view.layers[nl - 1])] {
        let pads: Vec<Vec<P>> = view.pads.iter().filter(|p| p.layers.contains(layer) && p.kind != PadKind::Npth).map(|p| p.polygon.clone().unwrap_or_else(|| p.prim.outline(tol))).collect();
        let cs = union(pads).difference(&holes);
        let z = if side == Side::Top { t } else { -0.035 };
        copper.add_manifold(&cs, z, 0.035);
        let mut strokes: Vec<Vec<P>> = Vec::new();
        for c in view.components.iter().filter(|c| c.side == side) {
            for l in &c.silk {
                for w in l.windows(2) {
                    strokes.push(Prim::capsule(w[0], w[1], view.rules.silk_width / 2.0).outline(0.05));
                }
            }
            if c.kind != "hole" {
                for l in crate::font::strokes_turned(&c.reference, c.label, view.rules.silk_height, side == Side::Bottom, c.label_rot) {
                    for w in l.windows(2) {
                        strokes.push(Prim::capsule(w[0], w[1], view.rules.silk_width / 2.0).outline(0.05));
                    }
                }
            }
        }
        let openings = union(view.pads.iter().filter(|p| p.layers.contains(layer)).map(|p| Prim { pts: p.prim.pts.clone(), r: p.prim.r + view.rules.mask_expansion }.outline(tol)).collect());
        let cs = union(strokes).intersection(&laminate).difference(&openings);
        let z = if side == Side::Top { t } else { -0.02 };
        ink.add_manifold(&cs, z, 0.02);
    }
    let mut by_look: BTreeMap<String, Mesh> = BTreeMap::new();
    for b in crate::bodies::bodies(board, &items.components) {
        for s in &b.solids {
            by_look.entry(s.mat.clone()).or_default().add_solid(s.shape, s.at, s.size, s.rot);
        }
    }
    let mut list: Vec<(String, ([f64; 3], f64, f64), Mesh)> = vec![("board".into(), mask, board_mesh), ("pads".into(), finish, copper), ("silkscreen".into(), silk, ink)];
    for (k, m) in by_look {
        list.push((k.clone(), look(&k), m));
    }
    write(&view.name, list)
}

fn write(name: &str, list: Vec<(String, ([f64; 3], f64, f64), Mesh)>) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut materials = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for (key, (c, metal, rough), m) in list {
        if m.idx.is_empty() {
            continue;
        }
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let off = bin.len();
        for p in &m.pos {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
                bin.extend(p[k].to_le_bytes());
            }
        }
        views.push(json!({ "buffer": 0, "byteOffset": off, "byteLength": m.pos.len() * 12, "target": 34962 }));
        accessors.push(json!({ "bufferView": views.len() - 1, "componentType": 5126, "count": m.pos.len(), "type": "VEC3", "min": lo, "max": hi }));
        let off = bin.len();
        for i in &m.idx {
            bin.extend(i.to_le_bytes());
        }
        views.push(json!({ "buffer": 0, "byteOffset": off, "byteLength": m.idx.len() * 4, "target": 34963 }));
        accessors.push(json!({ "bufferView": views.len() - 1, "componentType": 5125, "count": m.idx.len(), "type": "SCALAR" }));
        // glTF colours are linear.
        let lin = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        materials.push(json!({ "name": key, "pbrMetallicRoughness": { "baseColorFactor": [lin(c[0]), lin(c[1]), lin(c[2]), 1.0], "metallicFactor": metal, "roughnessFactor": rough } }));
        meshes.push(json!({ "name": key, "primitives": [{ "attributes": { "POSITION": accessors.len() - 2 }, "indices": accessors.len() - 1, "material": materials.len() - 1 }] }));
        nodes.push(json!({ "name": key, "mesh": meshes.len() - 1 }));
    }
    let n = nodes.len();
    let doc: Value = json!({
        "asset": { "version": "2.0", "generator": "ISE Studio PCB" },
        "scene": 0,
        "scenes": [{ "name": name, "nodes": [n] }],
        "nodes": nodes.into_iter().chain(std::iter::once(json!({ "name": name, "children": (0..n).collect::<Vec<_>>() }))).collect::<Vec<_>>(),
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }],
    });
    let mut js = serde_json::to_vec(&doc).unwrap();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((js.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(js);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_writes_a_valid_glb() {
        let mut m = Mesh::default();
        m.add_solid("box", [0.0, 0.0, 0.0], [2.0, 2.0, 1.0], 0.0);
        let mut c = Mesh::default();
        c.add_solid("cyl", [5.0, 0.0, 0.0], [1.0, 1.0, 2.0], 30.0);
        let g = write("t", vec![("box".into(), look("epoxy"), m), ("cyl".into(), look("tin"), c)]);
        assert_eq!(&g[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(g[8..12].try_into().unwrap()) as usize, g.len());
        let jl = u32::from_le_bytes(g[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&g[20..20 + jl]).unwrap();
        assert_eq!(doc["meshes"].as_array().unwrap().len(), 2);
        assert_eq!(doc["accessors"][0]["count"], 8);
        // 2 mm box → 0.002 m, y up: its height (board z) is glTF y.
        assert!((doc["accessors"][0]["max"][1].as_f64().unwrap() - 0.001).abs() < 1e-9);
    }
}
