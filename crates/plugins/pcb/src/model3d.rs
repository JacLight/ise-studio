//! The board as a real part: a shape program the engine writes and keeps in
//! step with the board. FR4 with every hole drilled, the exposed copper
//! (pads, in the finish's colour), and every populated component's body at
//! its package height, each a bought part declaring its MPN and LCSC number
//! as stock. Enclosures are then designed around it and clash-checked like
//! any part; moving the 3D part moves the board in the assembly.
//!
//! The program is regenerated whenever the board, its footprints or its
//! components change; edit the board, not the program.

use crate::footprints::{BodyShape, PadKind};
use crate::geom::{self, P};
use crate::{Board, Side};
use de_core::{CommandError, ErrorCode, IdIndex, StableId, Tx, Value, json};
use std::collections::BTreeMap;

/// (material name, density kg/m³).
fn material_for(role: &str, board: &Board, value: &str) -> (String, f64) {
    match role {
        "Board" => (format!("FR4 {} mask", board.mask.to_ascii_lowercase()), 1850.0),
        "Copper" => {
            let f = board.finish.to_ascii_lowercase();
            if f.contains("enig") || f.contains("gold") { ("ENIG gold".into(), 8900.0) } else if f.contains("osp") { ("OSP copper".into(), 8900.0) } else { ("HASL tin".into(), 8900.0) }
        }
        "Epoxy" => ("Epoxy moulding (black)".into(), 1900.0),
        "Ceramic" => ("Ceramic (MLCC)".into(), 4000.0),
        "Steel" => ("Stainless steel".into(), 7900.0),
        "Nylon" => ("Nylon (PA66, connector)".into(), 1140.0),
        "Black plastic" => ("Black plastic (PBT)".into(), 1310.0),
        "Gold" => ("Gold-plated pins".into(), 8500.0),
        "FR4" => ("FR4 black mask".into(), 1850.0),
        "LED" => {
            let v = value.to_ascii_lowercase();
            let colour = ["red", "green", "blue", "yellow", "orange", "white"].into_iter().find(|c| v.contains(c)).unwrap_or("clear");
            (format!("LED epoxy {colour}"), 1200.0)
        }
        other => (other.to_string(), 1500.0),
    }
}

fn r4(v: f64) -> f64 {
    (v * 1e4).round() / 1e4
}

fn pt(p: P) -> Value {
    json!([r4(p[0]), r4(p[1])])
}

/// The program source and the materials it names, from the board as it is in `tx`.
fn program(tx: &Tx, id: StableId) -> Result<(String, Vec<(String, f64)>), CommandError> {
    let board = tx.get::<Board>(id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, "no board"))?;
    let items = crate::items(tx.world(), id);
    let mut materials: BTreeMap<String, f64> = BTreeMap::new();
    let mut mat = |role: &str, value: &str| -> String {
        let (name, density) = material_for(role, &board, value);
        materials.insert(name.clone(), density);
        name
    };
    let board_mat = mat("Board", "");
    let copper_mat = mat("Copper", "");
    let sketch_name = board.sketch.and_then(|s| tx.get::<de_shape::sketch::Sketch>(s)).map(|s| s.name.clone());
    let (outline, cutouts) = if sketch_name.is_some() {
        (vec![], vec![])
    } else {
        let mut o = geom::round_corners(&board.outline, board.corner_radius, 0.02);
        if geom::polygon_area(&o) < 0.0 {
            o.reverse();
        }
        let cuts: Vec<Vec<P>> = board.cutouts.iter().map(|c| {
            let mut c = c.clone();
            if geom::polygon_area(&c) > 0.0 {
                c.reverse();
            }
            c
        }).collect();
        (o, cuts)
    };
    let mut holes = Vec::new();
    let mut pads_top = Vec::new();
    let mut pads_bottom = Vec::new();
    let mut parts = Vec::new();
    for (_, c) in &items.components {
        let Some(fp) = board.footprint(&c.footprint) else { continue };
        let bottom = c.side == Side::Bottom;
        let place = |p: P| {
            let q = if bottom { [-p[0], p[1]] } else { p };
            geom::add(c.at, geom::rotate(q, c.rotation))
        };
        for p in &fp.pads {
            let prim = crate::footprints::pad_prim(p, c.at, c.rotation, bottom);
            if let Some(d) = p.drill {
                let rot = if bottom { -p.rotation } else { p.rotation } + c.rotation;
                let half = (d[0].max(d[1]) - d[0].min(d[1])) / 2.0;
                let dir = geom::rotate(if d[0] > d[1] { [half, 0.0] } else { [0.0, half] }, rot);
                let at = place(p.at);
                holes.push(json!({ "a": pt(geom::sub(at, dir)), "b": pt(geom::add(at, dir)), "r": r4(d[0].min(d[1]) / 2.0) }));
            }
            if p.kind == PadKind::Npth {
                continue;
            }
            let outline: Vec<Value> = prim.outline(0.05).into_iter().map(pt).collect();
            if p.kind == PadKind::Tht || !bottom {
                pads_top.push(Value::Array(outline.clone()));
            }
            if p.kind == PadKind::Tht || bottom {
                pads_bottom.push(Value::Array(outline));
            }
        }
        if c.dnp || fp.body.is_empty() {
            continue;
        }
        let mut blocks = Vec::new();
        for b in &fp.body {
            let center = place(b.at);
            let m = mat(&b.material, &c.value);
            blocks.push(json!({ "c": pt(center), "w": r4(b.size[0]), "l": r4(b.size[1]), "z": [r4(b.z[0]), r4(b.z[1])], "cyl": b.shape == BodyShape::Cylinder, "m": m }));
        }
        let part_number = c.mpn.clone().unwrap_or_else(|| format!("{} {}", c.value, c.footprint).trim().to_string());
        let mut stock = json!({ "partNumber": part_number, "description": format!("{} {} ({})", c.reference, if c.value.is_empty() { &c.footprint } else { &c.value }, c.footprint) });
        if let Some(m) = &c.mpn {
            stock["mpn"] = json!(m);
        }
        if let Some(m) = &c.manufacturer {
            stock["manufacturer"] = json!(m);
        }
        if let Some(l) = &c.lcsc {
            stock["sources"] = json!([{ "supplier": "LCSC", "sku": l, "url": format!("https://www.lcsc.com/product-detail/{l}.html") }]);
        }
        let name = if c.value.is_empty() { c.reference.clone() } else { format!("{} {}", c.reference, c.value) };
        parts.push(json!({ "name": name, "rot": r4(c.rotation), "bottom": bottom, "blocks": blocks, "stock": stock }));
    }
    let data = json!({
        "t": board.thickness,
        "sketch": sketch_name,
        "outline": outline.into_iter().map(pt).collect::<Vec<_>>(),
        "cutouts": cutouts.into_iter().map(|c| c.into_iter().map(pt).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "holes": holes,
        "padsTop": pads_top,
        "padsBottom": pads_bottom,
        "parts": parts,
        "board": board_mat,
        "copper": copper_mat,
    });
    let outline_js = match &sketch_name {
        Some(n) => format!("sketch({}).profile", serde_json::to_string(n).unwrap_or_default()),
        None => "polygons([B.outline, ...B.cutouts])".to_string(),
    };
    let source = format!("// Generated by Forma from the board “{}”. Edit the board (pcb.* commands), not this program: it is rewritten when the board changes.\nconst B = {};\n{}", board.name, data, BODY.replace("__OUTLINE__", &outline_js));
    Ok((source, materials.into_iter().collect()))
}

const BODY: &str = r#"const t = B.t;
const cu = 0.035;
let pcb = (__OUTLINE__).extrude(t);
const drill = (h) => {
  const len = Math.hypot(h.b[0] - h.a[0], h.b[1] - h.a[1]);
  const c = cylinder(t + 2, h.r).translate(h.a[0], h.a[1], -1);
  return len < 1e-6 ? c : c.add(cylinder(t + 2, h.r).translate(h.b[0], h.b[1], -1)).hull();
};
if (B.holes.length) pcb = pcb.subtract(union(B.holes.map(drill)));
const out = [part("PCB", pcb, { material: B.board })];
const copper = [
  ...B.padsTop.map((o) => polygon(o).extrude(cu).translate(0, 0, t)),
  ...B.padsBottom.map((o) => polygon(o).extrude(cu).translate(0, 0, -cu)),
];
if (copper.length) {
  let pads = union(copper);
  if (B.holes.length) pads = pads.subtract(union(B.holes.map(drill)));
  out.push(part("Exposed copper", pads, { material: B.copper }));
}
for (const c of B.parts) {
  const blocks = c.blocks.map((b) => {
    const h = b.z[1] - b.z[0];
    let s = b.cyl ? cylinder(h, b.w / 2) : box(b.w, b.l, h).translate(-b.w / 2, -b.l / 2, 0).rotate(0, 0, c.rot);
    const z = c.bottom ? -b.z[1] : t + b.z[0];
    return { s: s.translate(b.c[0], b.c[1], z), m: b.m };
  });
  const byMaterial = {};
  for (const b of blocks) (byMaterial[b.m] ??= []).push(b.s);
  const names = Object.keys(byMaterial);
  names.forEach((m, i) => {
    const name = names.length === 1 ? c.name : `${c.name} (${m})`;
    out.push(part(name, union(byMaterial[m]), i === 0 ? { material: m, stock: c.stock } : { material: m, process: "buy" }));
  });
}
return out;
"#;

/// The material entity named `name`, made if there is none.
fn material_id(tx: &mut Tx, name: &str, density: f64) -> Result<StableId, CommandError> {
    let found = tx.world().resource::<IdIndex>().iter().find(|(_, e)| tx.world().get::<de_solid::Material>(*e).is_some_and(|m| m.name == name)).map(|(id, _)| id);
    if let Some(id) = found {
        return Ok(id);
    }
    let out = tx.run("solid.material.create", json!({ "name": name, "density": { "value": density, "unit": "kg/m3" } }))?;
    serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))
}

fn materials_table(tx: &mut Tx, list: &[(String, f64)]) -> Result<BTreeMap<String, StableId>, CommandError> {
    let mut t = BTreeMap::new();
    for (name, density) in list {
        t.insert(name.clone(), material_id(tx, name, *density)?);
    }
    Ok(t)
}

/// Makes the board's 3D part at `at` and links it to the board.
pub fn create(tx: &mut Tx, board: StableId, at: [f64; 3]) -> Result<StableId, CommandError> {
    let (source, list) = program(tx, board)?;
    let materials = materials_table(tx, &list)?;
    let name = tx.get::<Board>(board).map(|b| b.name.clone()).unwrap_or_default();
    let mm = |v: f64| json!({ "value": v, "unit": "mm" });
    let out = tx.run(
        "shape.create",
        json!({
            "name": format!("{name} (PCB)"),
            "description": "The printed circuit board, kept in step with the PCB workspace.",
            "source": source,
            "params": [],
            "display_tolerance": 0.05,
            "materials": materials,
            "placement": { "x": mm(at[0]), "y": mm(at[1]), "z": mm(at[2]) },
        }),
    )?;
    let id: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
    let mut b = tx.get::<Board>(board).cloned().unwrap();
    b.model = Some(id);
    tx.set(board, b)?;
    Ok(id)
}

/// Rewrites the 3D part's program when what it shows has changed.
pub fn sync(tx: &mut Tx, board: StableId) -> Result<(), CommandError> {
    let Some(model) = tx.get::<Board>(board).and_then(|b| b.model) else { return Ok(()) };
    let Some(current) = tx.get::<de_shape::ShapeProgram>(model).cloned() else { return Ok(()) };
    let (source, list) = program(tx, board)?;
    let name = tx.get::<Board>(board).map(|b| format!("{} (PCB)", b.name)).unwrap_or_default();
    if current.source == source && current.name == name {
        return Ok(());
    }
    let materials = materials_table(tx, &list)?;
    tx.run("shape.update", json!({ "target": model, "source": source, "name": name, "materials": materials }))?;
    Ok(())
}
