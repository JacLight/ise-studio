//! The Gerber job file (`.gbrjob`, Ucamco's JSON companion to a Gerber
//! set): who wrote it, the board's size, layer count, thickness and finish,
//! the design rules, every file with its function and polarity, and the
//! material stack from top legend to bottom legend.

use crate::Board;
use crate::board::BoardView;
use de_core::{Value, json};

fn finish_code(f: &str) -> &'static str {
    let f = f.to_ascii_lowercase();
    if f.contains("enig") {
        "ENIG"
    } else if f.contains("lead-free") || f.contains("lead free") {
        "HAL-NSN"
    } else if f.contains("hasl") {
        "HAL-SnPb"
    } else if f.contains("osp") {
        "OSP"
    } else if f.contains("silver") {
        "Immersion-Ag"
    } else if f.contains("tin") {
        "Immersion-Sn"
    } else {
        "Other"
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

pub fn write(v: &BoardView, board: &Board) -> String {
    let files: Vec<Value> = crate::gerber::files(v)
        .into_iter()
        .filter_map(|(name, text)| {
            let ff = text.lines().find_map(|l| l.strip_prefix("%TF.FileFunction,").map(|r| r.trim_end_matches("*%").to_string()).or_else(|| l.strip_prefix("; #@! TF.FileFunction,").map(str::to_string)))?;
            let pol = text.lines().find_map(|l| l.strip_prefix("%TF.FilePolarity,").map(|r| r.trim_end_matches("*%").to_string())).unwrap_or_else(|| "Positive".into());
            Some(json!({ "Path": name, "FileFunction": ff, "FilePolarity": pol }))
        })
        .collect();
    let size = v.size();
    let cu = 0.035 * v.copper_oz;
    let inner = 0.035 * board.inner_copper_oz;
    let n = v.layers.len();
    let mask = cap(&v.mask);
    let silk = cap(&v.silk);
    let mut stack: Vec<Value> = vec![
        json!({ "Type": "Legend", "Color": silk, "Name": "Top Silk Screen" }),
        json!({ "Type": "SolderPaste", "Name": "Top Solder Paste" }),
        json!({ "Type": "SolderMask", "Color": mask, "Thickness": 0.01, "Name": "Top Solder Mask" }),
    ];
    let mask_cu = 2.0 * 0.01 + 2.0 * cu + (n.saturating_sub(2)) as f64 * inner;
    let dielectric = ((v.thickness - mask_cu) / (n - 1) as f64).max(0.05);
    for (i, l) in v.layers.iter().enumerate() {
        let name = match l.as_str() {
            "top" => "F.Cu".to_string(),
            "bottom" => "B.Cu".to_string(),
            x => format!("In{}.Cu", x.trim_start_matches("in")),
        };
        let t = v.stackup.copper_oz.get(i).map(|oz| oz * crate::stackup::OZ_MM).unwrap_or(if i == 0 || i + 1 == n { cu } else { inner });
        stack.push(json!({ "Type": "Copper", "Thickness": (t * 1e4).round() / 1e4, "Name": name }));
        if i + 1 < n {
            match v.stackup.dielectrics.get(i) {
                Some(d) => stack.push(json!({ "Type": "Dielectric", "Thickness": (d.thickness * 1e4).round() / 1e4, "Material": d.material, "DielectricConstant": d.epsilon_r.unwrap_or(4.5), "LossTangent": d.loss_tangent.unwrap_or(0.02), "Name": format!("Dielectric {}", i + 1), "Notes": format!("{} between copper layers {} and {}", if d.kind == crate::stackup::DielectricKind::Core { "Core" } else { "Prepreg" }, i + 1, i + 2) })),
                None => stack.push(json!({ "Type": "Dielectric", "Thickness": (dielectric * 1e4).round() / 1e4, "Material": "FR4", "Name": format!("Dielectric {}", i + 1) })),
            }
        }
    }
    stack.push(json!({ "Type": "SolderMask", "Color": mask, "Thickness": 0.01, "Name": "Bottom Solder Mask" }));
    stack.push(json!({ "Type": "SolderPaste", "Name": "Bottom Solder Paste" }));
    stack.push(json!({ "Type": "Legend", "Color": silk, "Name": "Bottom Silk Screen" }));
    let r = &v.rules;
    let st = &v.stats;
    let j = json!({
        "Header": {
            "GenerationSoftware": { "Vendor": "ISE Studio", "Application": "ISE Studio PCB", "Version": env!("CARGO_PKG_VERSION") },
            "CreationDate": super::now_iso(),
        },
        "GeneralSpecs": {
            "ProjectId": { "Name": v.name, "GUID": crate::kicad::uuid(&v.id.to_string()), "Revision": v.revision },
            "Size": { "X": (size[0] * 1e4).round() / 1e4, "Y": (size[1] * 1e4).round() / 1e4 },
            "LayerNumber": n,
            "BoardThickness": v.thickness,
            "Finish": finish_code(&v.finish),
            "ImpedanceControlled": !v.signal.classes.is_empty(),
        },
        "DesignRules": [{
            "Layers": "Outer",
            "PadToPad": r.min_space,
            "PadToTrack": r.min_space,
            "TrackToTrack": r.min_space,
            "MinLineWidth": st.smallest_track.unwrap_or(r.min_track),
            "TrackToRegion": r.min_space,
            "RegionToRegion": r.min_space,
        }],
        "FilesAttributes": files,
        "MaterialStackup": stack,
    });
    serde_json::to_string_pretty(&j).unwrap_or_default()
}
