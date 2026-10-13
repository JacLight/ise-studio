//! The populated board as a mesh model, for renderers and MCAD tools that
//! do not take STEP: VRML 2.0 (`.wrl`, KiCad's mesh export) and glTF 2.0
//! binary (`.glb`). The board is the FR4 outline with its cutouts and every
//! hole, the exposed copper (pads) on both faces in the finish's colour, and
//! each component's body solids as the 3D view draws them. Both files are y
//! up in metres (the formats' convention); the geometry is built z up in
//! millimetres and turned on the way out.

use crate::board::BoardView;
use crate::footprints::PadKind;
use crate::geom::{P, Prim};
use crate::{Board, Items, Side};
use manifold_csg::{CrossSection, Manifold};
use std::fmt::Write;

/// One coloured mesh: triangles of points (mm, z up).
pub struct Part {
    pub name: String,
    pub color: [f32; 3],
    pub metal: bool,
    pub tris: Vec<[[f32; 3]; 3]>,
}

fn mesh_of(m: &Manifold) -> Vec<[[f32; 3]; 3]> {
    let (props, n, tris) = m.to_mesh_f32();
    let v = |i: u32| -> [f32; 3] {
        let k = i as usize * n;
        [props[k], props[k + 1], props[k + 2]]
    };
    tris.chunks(3).filter(|t| t.len() == 3).map(|t| [v(t[0]), v(t[1]), v(t[2])]).collect()
}

fn section(polys: &[Vec<P>]) -> CrossSection {
    let mut v: Vec<Vec<P>> = Vec::new();
    for p in polys {
        if p.len() < 3 {
            continue;
        }
        let mut q = p.clone();
        if crate::geom::polygon_area(&q) < 0.0 {
            q.reverse();
        }
        v.push(q);
    }
    CrossSection::batch_union(&v.iter().map(|p| CrossSection::from_polygons(std::slice::from_ref(p))).collect::<Vec<_>>())
}

fn mask_color(m: &str) -> [f32; 3] {
    match m.to_ascii_lowercase().as_str() {
        "black" => [0.08, 0.08, 0.09],
        "blue" => [0.07, 0.2, 0.55],
        "red" => [0.6, 0.08, 0.08],
        "white" => [0.92, 0.92, 0.9],
        "purple" => [0.35, 0.12, 0.5],
        "yellow" => [0.85, 0.75, 0.1],
        _ => [0.06, 0.38, 0.16],
    }
}

fn mat_color(m: &str) -> ([f32; 3], bool) {
    match m {
        "ceramic" => ([0.72, 0.6, 0.42], false),
        "res_top" | "black" | "epoxy" => ([0.1, 0.1, 0.1], false),
        "tin" => ([0.8, 0.8, 0.82], true),
        "gold" => ([0.86, 0.68, 0.28], true),
        "steel" => ([0.7, 0.7, 0.72], true),
        "nylon" => ([0.92, 0.9, 0.82], false),
        "dark" => ([0.2, 0.2, 0.22], false),
        "module_pcb" => ([0.08, 0.3, 0.15], false),
        m if m.starts_with("led_") && m != "led_body" => (
            match &m[4..] {
                "red" => [0.9, 0.1, 0.1],
                "green" => [0.1, 0.8, 0.2],
                "blue" => [0.15, 0.3, 0.95],
                "yellow" => [0.95, 0.85, 0.1],
                _ => [0.95, 0.95, 0.9],
            },
            false,
        ),
        _ => ([0.35, 0.35, 0.37], false),
    }
}

/// Everything to draw.
pub fn parts(v: &BoardView, board: &Board, items: &Items) -> Vec<Part> {
    let t = v.thickness;
    let mut out = Vec::new();
    // The board: outline less cutouts and holes.
    let mut cs = section(std::slice::from_ref(&v.outline));
    let mut cut: Vec<CrossSection> = v.cutouts.iter().map(|c| section(std::slice::from_ref(c))).collect();
    for h in &v.holes {
        let prim = if (h.size[0] - h.size[1]).abs() < 1e-9 { Prim::circle(h.at, h.size[0] / 2.0) } else { Prim::rect(h.at, h.size[0], h.size[1], h.rotation, h.size[0].min(h.size[1]) / 2.0) };
        cut.push(prim.section(0.0, 0.02));
    }
    for x in &v.vias {
        cut.push(Prim::circle(x.at, x.drill / 2.0).section(0.0, 0.02));
    }
    if !cut.is_empty() {
        cs = cs.difference(&CrossSection::batch_union(&cut));
    }
    out.push(Part { name: "board".into(), color: mask_color(&v.mask), metal: false, tris: mesh_of(&cs.extrude(t)) });
    // Exposed copper: pads on each face.
    let finish = if v.finish.to_ascii_lowercase().contains("enig") { [0.86, 0.68, 0.28] } else { [0.82, 0.82, 0.84] };
    let cu = 0.035;
    for (side, z) in [(Side::Top, t), (Side::Bottom, -cu)] {
        let layer = if side == Side::Top { &v.layers[0] } else { &v.layers[v.layers.len() - 1] };
        let pads: Vec<CrossSection> = v
            .pads
            .iter()
            .filter(|p| p.kind != PadKind::Npth && p.layers.contains(layer))
            .map(|p| match &p.polygon {
                Some(pg) => section(std::slice::from_ref(pg)),
                None => p.prim.section(0.0, 0.02),
            })
            .collect();
        if pads.is_empty() {
            continue;
        }
        let mut s = CrossSection::batch_union(&pads);
        let holes: Vec<CrossSection> = v.holes.iter().map(|h| Prim::circle(h.at, h.size[0].min(h.size[1]) / 2.0).section(0.0, 0.02)).collect();
        if !holes.is_empty() {
            s = s.difference(&CrossSection::batch_union(&holes));
        }
        out.push(Part { name: format!("pads_{}", if side == Side::Top { "top" } else { "bottom" }), color: finish, metal: true, tris: mesh_of(&s.extrude(cu).translate(0.0, 0.0, z)) });
    }
    // Component bodies.
    for b in crate::bodies::bodies(board, &items.components) {
        for (i, s) in b.solids.iter().enumerate() {
            let [sx, sy, h] = s.size;
            let m = match s.shape {
                "cyl" => Manifold::cylinder(h, sx / 2.0, sx / 2.0, 24, false),
                "hcyl" => Manifold::cylinder(sy, sx / 2.0, sx / 2.0, 24, true).rotate(90.0, 0.0, 0.0).translate(0.0, 0.0, sx / 2.0),
                "sphere" => Manifold::sphere(h / 2.0, 24).translate(0.0, 0.0, h / 2.0),
                _ => Manifold::cube(sx, sy, h, false).translate(-sx / 2.0, -sy / 2.0, 0.0),
            };
            let m = m.rotate(0.0, 0.0, s.rot).translate(s.at[0], s.at[1], s.at[2]);
            let (color, metal) = mat_color(&s.mat);
            out.push(Part { name: format!("{}_{i}", b.reference), color, metal, tris: mesh_of(&m) });
        }
    }
    out.retain(|p| !p.tris.is_empty());
    out
}

fn normal(t: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let b = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l > 0.0 { [n[0] / l, n[1] / l, n[2] / l] } else { [0.0, 0.0, 1.0] }
}

/// VRML 2.0, millimetres turned to metres and z up to y up by the outer transform.
pub fn vrml(v: &BoardView, board: &Board, items: &Items) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "#VRML V2.0 utf8\nWorldInfo {{ title \"{}\" info [ \"Made with Forma\" \"Units: millimetres, scaled to metres; z up turned to VRML's y up\" ] }}", v.name.replace('"', "'"));
    let _ = writeln!(s, "Transform {{\n  scale 0.001 0.001 0.001\n  rotation 1 0 0 -1.5707963\n  children [");
    for p in parts(v, board, items) {
        let _ = writeln!(s, "  DEF {} Shape {{", p.name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_"));
        let spec = if p.metal { "0.6 0.6 0.6" } else { "0.1 0.1 0.1" };
        let _ = writeln!(s, "    appearance Appearance {{ material Material {{ diffuseColor {:.3} {:.3} {:.3} specularColor {spec} shininess {} }} }}", p.color[0], p.color[1], p.color[2], if p.metal { "0.6" } else { "0.2" });
        let _ = writeln!(s, "    geometry IndexedFaceSet {{\n      creaseAngle 0.5\n      coord Coordinate {{ point [");
        for t in &p.tris {
            for q in t {
                let _ = writeln!(s, "        {} {} {},", super::num(q[0] as f64, 4), super::num(q[1] as f64, 4), super::num(q[2] as f64, 4));
            }
        }
        let _ = writeln!(s, "      ] }}\n      coordIndex [");
        for i in 0..p.tris.len() {
            let _ = writeln!(s, "        {} {} {} -1,", i * 3, i * 3 + 1, i * 3 + 2);
        }
        let _ = writeln!(s, "      ]\n    }}\n  }}");
    }
    let _ = writeln!(s, "  ]\n}}");
    s
}

/// glTF 2.0 binary: one node and mesh per part, flat normals, metres, y up.
pub fn glb(v: &BoardView, board: &Board, items: &Items) -> Vec<u8> {
    let ps = parts(v, board, items);
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut materials = Vec::new();
    let conv = |q: [f32; 3]| [q[0] / 1000.0, q[2] / 1000.0, -q[1] / 1000.0];
    for (i, p) in ps.iter().enumerate() {
        let mut pos: Vec<[f32; 3]> = Vec::with_capacity(p.tris.len() * 3);
        let mut nor: Vec<[f32; 3]> = Vec::with_capacity(p.tris.len() * 3);
        for t in &p.tris {
            let n = normal(t);
            let n = [n[0], n[2], -n[1]];
            for q in t {
                pos.push(conv(*q));
                nor.push(n);
            }
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for q in &pos {
            for k in 0..3 {
                min[k] = min[k].min(q[k]);
                max[k] = max[k].max(q[k]);
            }
        }
        let add_view = |bin: &mut Vec<u8>, data: &[[f32; 3]], views: &mut Vec<de_core::Value>| -> usize {
            let off = bin.len();
            for q in data {
                for c in q {
                    bin.extend_from_slice(&c.to_le_bytes());
                }
            }
            views.push(de_core::json!({ "buffer": 0, "byteOffset": off, "byteLength": data.len() * 12, "target": 34962 }));
            views.len() - 1
        };
        let pv = add_view(&mut bin, &pos, &mut views);
        let nv = add_view(&mut bin, &nor, &mut views);
        accessors.push(de_core::json!({ "bufferView": pv, "componentType": 5126, "count": pos.len(), "type": "VEC3", "min": min, "max": max }));
        accessors.push(de_core::json!({ "bufferView": nv, "componentType": 5126, "count": nor.len(), "type": "VEC3" }));
        materials.push(de_core::json!({ "name": p.name, "pbrMetallicRoughness": { "baseColorFactor": [p.color[0], p.color[1], p.color[2], 1.0], "metallicFactor": if p.metal { 0.8 } else { 0.0 }, "roughnessFactor": if p.metal { 0.35 } else { 0.7 } } }));
        meshes.push(de_core::json!({ "name": p.name, "primitives": [{ "attributes": { "POSITION": i * 2, "NORMAL": i * 2 + 1 }, "material": i, "mode": 4 }] }));
        nodes.push(de_core::json!({ "name": p.name, "mesh": i }));
    }
    let json = de_core::json!({
        "asset": { "version": "2.0", "generator": "Forma" },
        "scene": 0,
        "scenes": [{ "name": v.name, "nodes": (0..ps.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }],
    });
    let mut j = serde_json::to_vec(&json).unwrap_or_default();
    while j.len() % 4 != 0 {
        j.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + j.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&j);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}
