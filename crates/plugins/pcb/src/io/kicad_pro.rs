//! KiCad project files (`.kicad_pro`, JSON): the net classes (widths,
//! clearances, vias, and which nets are in each) and the board's design
//! rules, which KiCad keeps here rather than in the board file. Read for a
//! project bundle; written beside an exported `.kicad_pcb`.

use crate::board::BoardView;
use crate::{Board, NetClass};
use de_core::{Value, json};

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| x.as_f64())
}

/// The project's net classes (with their nets) and notes on what was left out.
pub fn read(text: &str) -> Result<(Vec<NetClass>, Vec<String>), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not a KiCad project file: {e}"))?;
    let ns = v.get("net_settings").ok_or("the project file has no net_settings")?;
    let mut classes: Vec<NetClass> = Vec::new();
    for c in ns.get("classes").and_then(|c| c.as_array()).into_iter().flatten() {
        let Some(name) = c.get("name").and_then(|x| x.as_str()) else { continue };
        let mut nc = NetClass::default_class();
        nc.name = name.to_string();
        nc.track_width = f(c, "track_width").unwrap_or(nc.track_width);
        nc.clearance = f(c, "clearance").unwrap_or(nc.clearance);
        nc.via_diameter = f(c, "via_diameter").unwrap_or(nc.via_diameter);
        nc.via_drill = f(c, "via_drill").unwrap_or(nc.via_drill);
        if nc.via_diameter <= nc.via_drill {
            nc.via_diameter = nc.via_drill + 0.2;
        }
        // KiCad 6 listed each class's nets with the class.
        nc.nets = c.get("nets").and_then(|n| n.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        classes.push(nc);
    }
    let mut notes = Vec::new();
    // KiCad 7+: explicit assignments and wildcard patterns.
    if let Some(m) = ns.get("netclass_assignments").and_then(|m| m.as_object()) {
        for (net, class) in m {
            let class = class.as_str().or_else(|| class.as_array().and_then(|a| a.first()).and_then(|x| x.as_str())).unwrap_or("");
            if let Some(c) = classes.iter_mut().find(|c| c.name == class) {
                if !c.nets.contains(net) {
                    c.nets.push(net.clone());
                }
            }
        }
    }
    for p in ns.get("netclass_patterns").and_then(|p| p.as_array()).into_iter().flatten() {
        let (Some(class), Some(pat)) = (p.get("netclass").and_then(|x| x.as_str()), p.get("pattern").and_then(|x| x.as_str())) else { continue };
        if pat.contains(['*', '?', '[']) {
            notes.push(format!("net class pattern “{pat}” → {class} is a wildcard; Forma classes list nets by name, so assign them after the import"));
            continue;
        }
        if let Some(c) = classes.iter_mut().find(|c| c.name == class) {
            let net = pat.to_string();
            if !c.nets.contains(&net) {
                c.nets.push(net);
            }
        }
    }
    Ok((classes, notes))
}

/// The project file for a board: its net classes and rules.
pub fn write(view: &BoardView, board: &Board) -> String {
    let mut classes: Vec<NetClass> = board.net_classes.clone();
    if !classes.iter().any(|c| c.name.eq_ignore_ascii_case("default")) {
        classes.insert(0, NetClass::default_class());
    }
    let class_json: Vec<Value> = classes
        .iter()
        .map(|c| {
            json!({
                "name": if c.name.eq_ignore_ascii_case("default") { "Default" } else { c.name.as_str() },
                "clearance": c.clearance,
                "track_width": c.track_width,
                "via_diameter": c.via_diameter,
                "via_drill": c.via_drill,
                "uvia_diameter": 0.3,
                "uvia_drill": 0.1,
                "diff_pair_gap": 0.25,
                "diff_pair_via_gap": 0.25,
                "diff_pair_width": 0.2,
                "line_style": 0,
                "wire_width": 6,
                "bus_width": 12,
                "schematic_color": "rgba(0, 0, 0, 0.000)",
                "pcb_color": "rgba(0, 0, 0, 0.000)",
                "priority": 2147483647,
            })
        })
        .collect();
    let patterns: Vec<Value> = classes.iter().filter(|c| !c.name.eq_ignore_ascii_case("default")).flat_map(|c| c.nets.iter().map(move |n| json!({ "netclass": c.name, "pattern": n }))).collect();
    let r = &view.rules;
    let stem = crate::gerber::file_stem(&view.name);
    let v = json!({
        "board": {
            "3dviewports": [],
            "design_settings": {
                "defaults": { "board_outline_line_width": 0.1, "copper_line_width": 0.2, "silk_line_width": r.silk_width, "silk_text_size_v": r.silk_height, "silk_text_size_h": r.silk_height },
                "rules": {
                    "min_clearance": r.min_space,
                    "min_track_width": r.min_track,
                    "min_via_annular_width": r.min_annular,
                    "min_via_diameter": r.min_drill + 2.0 * r.min_annular,
                    "min_through_hole_diameter": r.min_drill,
                    "min_copper_edge_clearance": r.edge_clearance,
                    "min_hole_to_hole": r.hole_to_hole,
                    "min_hole_clearance": r.min_space,
                    "min_silk_clearance": 0.0,
                    "solder_mask_to_copper_clearance": 0.0,
                },
                "track_widths": [],
                "via_dimensions": [],
            },
            "layer_presets": [],
            "viewports": [],
        },
        "boards": [],
        "libraries": { "pinned_footprint_libs": [], "pinned_symbol_libs": [] },
        "meta": { "filename": format!("{stem}.kicad_pro"), "version": 1 },
        "net_settings": {
            "classes": class_json,
            "meta": { "version": 3 },
            "net_colors": null,
            "netclass_assignments": null,
            "netclass_patterns": patterns,
        },
        "pcbnew": { "last_paths": {}, "page_layout_descr_file": "" },
        "schematic": { "legacy_lib_dir": "", "legacy_lib_list": [] },
        "sheets": [],
        "text_variables": { "REVISION": board.revision },
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}
