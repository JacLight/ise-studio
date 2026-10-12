//! Interchange with other EDA tools: every file Forma reads from them or
//! writes for them, beyond the `.kicad_pcb` board in [`crate::kicad`].
//!
//! Reading (`pcb.import`, any of these, told apart by content):
//! - KiCad: `.kicad_sch` schematics (with sub-sheets given alongside), project
//!   bundles (a zip with `.kicad_pro`, `.kicad_pcb`, `.kicad_sch`; the
//!   project's net classes are kept)
//! - Eagle `.brd` / `.sch` (XML), EasyEDA Standard (JSON) and Pro (`.epro`
//!   zip or the JSON-lines documents), Altium `.PcbDoc` / `.SchDoc` (binary
//!   OLE compound files and the ASCII variants), CADSTAR PCB archives
//!   (`.cpa`), Allegro Fabmaster extracts (`.fab`/`.txt`), PADS ASCII
//!   (`.asc`), gEDA/pcb (`.pcb`)
//!
//! Each reader is written from the format as its maker documents it or as it
//! appears in files the tool writes; none copies another program's code.
//! Boards come through [`build`] (parts with pads where they sit, copper,
//! edges) and schematics through [`sch`] (symbols drawn where they sit), so
//! every format lands the same way. What could not be carried over is
//! listed in the result's `notes`.
//!
//! Writing (`GET /pcb/<id>/file/<name>` and `GET /pcb/schematic/<id>/file/<name>`):
//! KiCad schematic and project, netlists (KiCad, XML, OrcadPCB2, PADS,
//! Allegro, CADSTAR), IPC-D-356, IPC-2581 rev C, ODB++, Gerber job file,
//! drill map and report, board plots (SVG per layer and together, PDF),
//! VRML and glTF boards, position files (KiCad ASCII and CSV, Gerber X3).

pub mod altium;
pub mod archive;
pub mod build;
pub mod cadstar;
pub mod drill;
pub mod eagle;
pub mod easyeda;
pub mod fabmaster;
pub mod gbrjob;
pub mod geda;
pub mod ipc2581;
pub mod ipc356;
pub mod kicad_pro;
pub mod kicad_sch;
pub mod model;
pub mod netlist;
pub mod odb;
pub mod pads;
pub mod plot;
pub mod pos;
pub mod sch;
pub mod xml;

use crate::board::BoardView;
use crate::kicad::KicadBoard;
use crate::{Board, NetClass};
use de_core::{
    Command, CommandError, ErrorCode, StableId, Tx, Value,
    bevy_ecs::world::World,
    json, schemars,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

pub fn register(r: &mut de_core::Registry) {
    r.command::<Import>();
}

/// What a file held, in Forma's terms.
#[derive(Default)]
pub struct Imported {
    /// The format read ("eagle_brd", "altium_pcb", …).
    pub format: String,
    pub board: Option<KicadBoard>,
    pub schematic: Option<sch::SchIn>,
    /// Net classes from a project file (KiCad `.kicad_pro`).
    pub net_classes: Vec<NetClass>,
    pub notes: Vec<String>,
}

/// Formats `pcb.import` reads, with the file types they come in.
pub const FORMATS: [(&str, &str); 15] = [
    ("kicad_pcb", "KiCad board (.kicad_pcb)"),
    ("kicad_sch", "KiCad schematic (.kicad_sch)"),
    ("kicad_project", "KiCad project bundle (.zip with .kicad_pro/.kicad_pcb/.kicad_sch)"),
    ("eagle_brd", "Eagle board (.brd, XML)"),
    ("eagle_sch", "Eagle schematic (.sch, XML)"),
    ("easyeda_pcb", "EasyEDA Standard PCB (.json)"),
    ("easyeda_sch", "EasyEDA Standard schematic (.json)"),
    ("easyeda_pro", "EasyEDA Pro project (.epro) or document (.epcb/.esch)"),
    ("altium_pcb", "Altium PCB (.PcbDoc, binary or ASCII)"),
    ("altium_sch", "Altium schematic (.SchDoc, binary or ASCII)"),
    ("cadstar_pcb", "CADSTAR PCB archive (.cpa)"),
    ("fabmaster", "Allegro Fabmaster extract (.fab, .txt)"),
    ("pads", "PADS ASCII board (.asc)"),
    ("geda_pcb", "gEDA/pcb board (.pcb)"),
    ("auto", "Tell from the content"),
];

fn text_of(b: &[u8]) -> String {
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        // Older tools write Latin-1 / Windows-1252.
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

/// The format of a file, from its content (and its name when the content is ambiguous).
pub fn detect(name: &str, b: &[u8]) -> Result<&'static str, String> {
    if archive::is_cfb(b) {
        let streams = archive::cfb_streams(b)?;
        if streams.keys().any(|k| k.starts_with("Board6/") || k.starts_with("Pads6/") || k.starts_with("Components6/")) {
            return Ok("altium_pcb");
        }
        if streams.contains_key("FileHeader") {
            return Ok("altium_sch");
        }
        return Err("an OLE compound file that is neither an Altium PcbDoc nor a SchDoc".into());
    }
    if archive::is_zip(b) {
        let files = archive::unzip(b)?;
        if files.keys().any(|k| k.ends_with(".kicad_pcb") || k.ends_with(".kicad_sch") || k.ends_with(".kicad_pro")) {
            return Ok("kicad_project");
        }
        if files.keys().any(|k| k.ends_with(".epcb") || k.ends_with(".esch") || k.ends_with(".esys") || k.ends_with(".efoo")) || files.contains_key("project.json") {
            return Ok("easyeda_pro");
        }
        return Err("a zip archive with no KiCad or EasyEDA Pro files in it".into());
    }
    let t = text_of(&b[..b.len().min(1 << 16)]);
    let head = t.trim_start();
    if head.starts_with("(kicad_pcb") {
        return Ok("kicad_pcb");
    }
    if head.starts_with("(kicad_sch") {
        return Ok("kicad_sch");
    }
    if head.starts_with("(CADSTARPCB") {
        return Ok("cadstar_pcb");
    }
    if head.starts_with("(CADSTARSCH") {
        return Err("CADSTAR schematic archives are not read yet (the PCB archive .cpa is)".into());
    }
    if head.starts_with("!PADS-") {
        return Ok("pads");
    }
    if t.contains("<eagle") {
        if t.contains("<board") {
            return Ok("eagle_brd");
        }
        if t.contains("<schematic") {
            return Ok("eagle_sch");
        }
        return Err("an Eagle file that is neither a board nor a schematic (a library?)".into());
    }
    if head.starts_with("[\"DOCTYPE\"") || head.starts_with("[\"DOCTYPE\",") {
        return Ok("easyeda_pro");
    }
    if head.starts_with('{') {
        let full = text_of(b);
        if let Ok(v) = serde_json::from_str::<Value>(&full) {
            let doc = easyeda::document(&v);
            match doc.and_then(|d| d.get("head")).and_then(|h| h.get("docType")).and_then(|d| d.as_str().map(str::to_string).or_else(|| d.as_i64().map(|n| n.to_string()))).as_deref() {
                Some("3") => return Ok("easyeda_pcb"),
                Some("1") | Some("5") => return Ok("easyeda_sch"),
                _ => {}
            }
        }
    }
    if t.contains("|RECORD=Board") || t.contains("|RECORD=Pad|") || t.contains("|RECORD=Component|") {
        return Ok("altium_pcb");
    }
    if t.contains("Schematic Capture Ascii") || t.contains("|RECORD=31|") {
        return Ok("altium_sch");
    }
    if t.lines().any(|l| l.starts_with("A!")) {
        return Ok("fabmaster");
    }
    if (t.contains("PCB[") || t.contains("PCB(")) && (t.contains("Element[") || t.contains("Element(") || t.contains("Layer(") || t.contains("Via[")) {
        return Ok("geda_pcb");
    }
    Err(format!(
        "could not tell what {} is: Forma reads {}",
        if name.is_empty() { "the file" } else { name },
        FORMATS.iter().filter(|f| f.0 != "auto").map(|f| f.1).collect::<Vec<_>>().join(", ")
    ))
}

/// Reads one file (with any companions by name) in the given or detected format.
pub fn read(name: &str, b: &[u8], format: Option<&str>, files: &BTreeMap<String, Vec<u8>>) -> Result<Imported, String> {
    let format = match format.filter(|f| !f.is_empty() && *f != "auto") {
        Some(f) => FORMATS.iter().find(|x| x.0 == f).map(|x| x.0).ok_or_else(|| format!("unknown format {f} (one of {})", FORMATS.iter().map(|x| x.0).collect::<Vec<_>>().join(", ")))?,
        None => detect(name, b)?,
    };
    let mut out = Imported { format: format.to_string(), ..Default::default() };
    match format {
        "kicad_pcb" => out.board = Some(crate::kicad::parse_board(&text_of(b))?),
        "kicad_sch" => {
            let texts: BTreeMap<String, String> = files.iter().map(|(k, v)| (k.clone(), text_of(v))).collect();
            out.schematic = Some(kicad_sch::read(&text_of(b), name, &texts)?);
        }
        "kicad_project" => {
            let all: BTreeMap<String, Vec<u8>> = if archive::is_zip(b) { archive::unzip(b)? } else { files.clone() };
            let base = |k: &str| k.rsplit('/').next().unwrap_or(k).to_string();
            let texts: BTreeMap<String, String> = all.iter().map(|(k, v)| (base(k), text_of(v))).collect();
            if let Some((_, t)) = texts.iter().find(|(k, _)| k.ends_with(".kicad_pcb")) {
                out.board = Some(crate::kicad::parse_board(t)?);
            }
            if let Some((_, t)) = texts.iter().find(|(k, _)| k.ends_with(".kicad_pro")) {
                let (classes, notes) = kicad_pro::read(t)?;
                out.net_classes = classes;
                out.notes.extend(notes);
            }
            // The root schematic: the one named like the project, else one no other sheet refers to.
            let schs: Vec<(&String, &String)> = texts.iter().filter(|(k, _)| k.ends_with(".kicad_sch")).collect();
            let pro_stem = texts.keys().find(|k| k.ends_with(".kicad_pro")).map(|k| k.trim_end_matches(".kicad_pro").to_string());
            let root = schs.iter().find(|(k, _)| pro_stem.as_deref().is_some_and(|s| k.trim_end_matches(".kicad_sch") == s)).or_else(|| schs.iter().find(|(k, _)| !schs.iter().any(|(_, t)| t.contains(&format!("\"{k}\""))))).copied();
            if let Some((k, t)) = root {
                out.schematic = Some(kicad_sch::read(t, k, &texts)?);
            }
            if out.board.is_none() && out.schematic.is_none() {
                return Err("the bundle has no .kicad_pcb or .kicad_sch".into());
            }
        }
        "eagle_brd" => out.board = Some(build::finish(eagle::read_board(&text_of(b))?)?),
        "eagle_sch" => out.schematic = Some(eagle::read_schematic(&text_of(b))?.finish()),
        "easyeda_pcb" => out.board = Some(build::finish(easyeda::read_std_board(&text_of(b))?)?),
        "easyeda_sch" => out.schematic = Some(easyeda::read_std_schematic(&text_of(b))?.finish()),
        "easyeda_pro" => {
            let (board, schematic) = easyeda::read_pro(b)?;
            out.board = board.map(build::finish).transpose()?;
            out.schematic = schematic.map(|d| d.finish());
            if out.board.is_none() && out.schematic.is_none() {
                return Err("the EasyEDA Pro file has no PCB or schematic document".into());
            }
        }
        "altium_pcb" => out.board = Some(build::finish(altium::read_pcb(b)?)?),
        "altium_sch" => out.schematic = Some(altium::read_sch(b)?.finish()),
        "cadstar_pcb" => out.board = Some(build::finish(cadstar::read(&text_of(b))?)?),
        "fabmaster" => out.board = Some(build::finish(fabmaster::read(&text_of(b))?)?),
        "pads" => out.board = Some(build::finish(pads::read(&text_of(b))?)?),
        "geda_pcb" => out.board = Some(build::finish(geda::read(&text_of(b))?)?),
        f => return Err(format!("{f} is not a format Forma reads")),
    }
    Ok(out)
}

/// `pcb.import`: a board and/or schematic from another EDA tool's file (KiCad, Eagle, EasyEDA
/// Standard and Pro, Altium, CADSTAR, Fabmaster, PADS, gEDA), told apart by content.
#[derive(Deserialize, JsonSchema)]
pub struct Import {
    /// The file's name (its extension and stem name the result).
    #[serde(default)]
    pub file: String,
    /// A text file's content.
    pub text: Option<String>,
    /// A binary file's content (Altium .PcbDoc/.SchDoc, EasyEDA Pro .epro, a zip bundle), base64.
    pub data: Option<String>,
    /// kicad_pcb, kicad_sch, kicad_project, eagle_brd, eagle_sch, easyeda_pcb, easyeda_sch,
    /// easyeda_pro, altium_pcb, altium_sch, cadstar_pcb, fabmaster, pads, geda_pcb; default: from the content.
    pub format: Option<String>,
    /// Name for the board / schematic (default: the file's title or name).
    pub name: Option<String>,
    /// Where the 3D board sits in the project (mm).
    pub at: Option<[f64; 3]>,
    /// The board an imported schematic drives (default: the board imported with it).
    pub board: Option<StableId>,
    /// Companion files by name (a KiCad schematic's sub-sheets, its .kicad_pro), as text.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let s: String = s.split_once("base64,").map(|x| x.1).unwrap_or(s).chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(s.as_bytes()).map_err(|e| format!("data is not base64: {e}"))
}

fn stem(file: &str) -> Option<String> {
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let s = base.split('.').next().unwrap_or(base).replace(['_', '-'], " ").trim().to_string();
    (!s.is_empty()).then_some(s)
}

impl Command for Import {
    const NAME: &'static str = "pcb.import";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let bad = |m: String| CommandError::new(ErrorCode::InvalidParameters, m);
        let bytes = match (&self.text, &self.data) {
            (Some(t), _) => t.clone().into_bytes(),
            (None, Some(d)) => decode_base64(d).map_err(bad)?,
            _ => return Err(bad("give the file as text or as base64 data".into())),
        };
        let files: BTreeMap<String, Vec<u8>> = self.files.iter().map(|(k, v)| (k.clone(), v.clone().into_bytes())).collect();
        let imp = read(&self.file, &bytes, self.format.as_deref(), &files).map_err(bad)?;
        let name = self.name.clone().filter(|n| !n.trim().is_empty());
        let fallback = stem(&self.file);
        let mut board_out = Value::Null;
        let mut board_id = self.board;
        if let Some(k) = &imp.board {
            let n = name.clone().or(k.title.clone()).or(fallback.clone());
            let out = crate::kicad::create_board(tx, k, n, self.at)?;
            let id: StableId = serde_json::from_value(out["id"].clone()).map_err(|e| bad(e.to_string()))?;
            if !imp.net_classes.is_empty() {
                let mut b: Board = crate::board_of(tx, id)?;
                b.net_classes = imp.net_classes.clone();
                if !b.net_classes.iter().any(|c| c.name.eq_ignore_ascii_case("default")) {
                    b.net_classes.insert(0, k.default_class.clone());
                }
                tx.set(id, b)?;
            }
            board_id = board_id.or(Some(id));
            board_out = out;
        }
        let mut sch_out = Value::Null;
        if let Some(s) = &imp.schematic {
            let n = name.clone().or(s.name.clone()).or(fallback.clone());
            sch_out = sch::create(tx, s, n, board_id)?;
        }
        Ok(json!({
            "format": imp.format,
            "board": board_out,
            "schematic": sch_out,
            "id": board_out.get("id").cloned().or_else(|| sch_out.get("id").cloned()),
            "notes": imp.notes,
        }))
    }
}

// ---- Exports ----------------------------------------------------------------------

/// Files written for other tools, by name, with what each is.
pub fn board_exports(view: &BoardView, has_schematic: bool) -> Vec<(String, &'static str)> {
    let s = crate::gerber::file_stem(&view.name);
    let mut v: Vec<(String, &'static str)> = vec![
        (format!("{s}.kicad_pro"), "KiCad project (net classes, rules)"),
        (format!("{s}-kicad.zip"), "KiCad project bundle (.kicad_pro, .kicad_pcb, .kicad_sch)"),
        (format!("{s}.net"), "KiCad netlist (S-expression)"),
        (format!("{s}-netlist.xml"), "KiCad XML netlist"),
        (format!("{s}-orcad.net"), "OrcadPCB2 netlist"),
        (format!("{s}-pads.net"), "PADS netlist"),
        (format!("{s}-allegro.net"), "Allegro (Telesis) netlist"),
        (format!("{s}-cadstar.frp"), "CADSTAR netlist"),
        (format!("{s}.d356"), "IPC-D-356 bare-board test netlist"),
        (format!("{s}-ipc2581.xml"), "IPC-2581 rev C"),
        (format!("{s}-odb.zip"), "ODB++ (zip)"),
        (format!("{s}-job.gbrjob"), "Gerber job file"),
        (format!("{s}-drill-map.gbr"), "Drill map (Gerber)"),
        (format!("{s}-drill-map.svg"), "Drill map (SVG)"),
        (format!("{s}-drill-map.pdf"), "Drill map (PDF)"),
        (format!("{s}-drill-report.txt"), "Drill report"),
        (format!("{s}-board.svg"), "Board plot, all layers (SVG)"),
        (format!("{s}-board.pdf"), "Board plot, a page per layer (PDF)"),
        (format!("{s}.wrl"), "VRML 2.0 board"),
        (format!("{s}.glb"), "glTF binary board"),
        (format!("{s}-both.pos"), "KiCad position file (ASCII)"),
        (format!("{s}-pos.csv"), "KiCad position file (CSV)"),
        (format!("{s}-top-pnp.gbr"), "Gerber X3 placement, top"),
        (format!("{s}-bottom-pnp.gbr"), "Gerber X3 placement, bottom"),
    ];
    for l in plot::layers(view) {
        v.push((format!("{s}-{}.svg", l.file), "Board plot, one layer (SVG)"));
    }
    if has_schematic {
        v.push((format!("{s}.kicad_sch"), "KiCad schematic"));
    }
    v
}

pub struct FileOut {
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

fn txt(s: String) -> Option<Result<FileOut, String>> {
    Some(Ok(FileOut { content_type: "text/plain; charset=utf-8", bytes: s.into_bytes() }))
}

/// One export of a board by file name; None when the name is not one of these.
pub fn board_file(world: &World, id: StableId, name: &str) -> Option<Result<FileOut, String>> {
    let view = crate::view(world, id)?;
    let board = world.resource::<de_core::IdIndex>().get(id).and_then(|e| world.get::<Board>(e)).cloned()?;
    let s = crate::gerber::file_stem(&view.name);
    let sch = crate::schematic::of_board(world, id);
    let net = || match &sch {
        Some((sid, _)) => crate::schematic::view(world, *sid).map(|v| netlist::from_schematic(&v)).unwrap_or_else(|| netlist::from_board(&view)),
        None => netlist::from_board(&view),
    };
    let rest = name.strip_prefix(&s)?;
    match rest {
        ".kicad_pro" => txt(kicad_pro::write(&view, &board)),
        "-kicad.zip" => {
            let mut files = vec![(format!("{s}.kicad_pro"), kicad_pro::write(&view, &board).into_bytes()), (format!("{s}.kicad_pcb"), crate::kicad::export_world(world, id)?.into_bytes())];
            if let Some((sid, _)) = &sch {
                for (n, t) in kicad_sch::write_world(world, *sid, &s)? {
                    files.push((n, t.into_bytes()));
                }
            }
            Some(Ok(FileOut { content_type: "application/zip", bytes: de_shape::fabricate::zip(&files) }))
        }
        ".kicad_sch" => {
            let (sid, _) = sch.as_ref()?;
            kicad_sch::write_world(world, *sid, &s).and_then(|f| f.into_iter().next()).and_then(|(_, t)| txt(t))
        }
        ".net" => txt(netlist::kicad(&net(), &view.name)),
        "-netlist.xml" => Some(Ok(FileOut { content_type: "application/xml", bytes: netlist::kicad_xml(&net(), &view.name).into_bytes() })),
        "-orcad.net" => txt(netlist::orcad(&net())),
        "-pads.net" => txt(netlist::pads(&net())),
        "-allegro.net" => txt(netlist::allegro(&net())),
        "-cadstar.frp" => txt(netlist::cadstar(&net(), &view.name)),
        ".d356" => txt(ipc356::write(&view)),
        "-ipc2581.xml" => Some(Ok(FileOut { content_type: "application/xml", bytes: ipc2581::write(&view, &board).into_bytes() })),
        "-odb.zip" => Some(Ok(FileOut { content_type: "application/zip", bytes: de_shape::fabricate::zip(&odb::files(&view, &board)) })),
        "-job.gbrjob" => Some(Ok(FileOut { content_type: "application/json", bytes: gbrjob::write(&view, &board).into_bytes() })),
        "-drill-map.gbr" => txt(drill::map_gerber(&view)),
        "-drill-map.svg" => Some(Ok(FileOut { content_type: "image/svg+xml", bytes: drill::map_svg(&view).into_bytes() })),
        "-drill-map.pdf" => Some(Ok(FileOut { content_type: "application/pdf", bytes: drill::map_pdf(&view) })),
        "-drill-report.txt" => txt(drill::report(&view)),
        "-board.svg" => Some(Ok(FileOut { content_type: "image/svg+xml", bytes: plot::svg(&view, None).into_bytes() })),
        "-board.pdf" => Some(Ok(FileOut { content_type: "application/pdf", bytes: plot::pdf(&view) })),
        ".wrl" => {
            let items = crate::items(world, id);
            Some(Ok(FileOut { content_type: "model/vrml", bytes: model::vrml(&view, &board, &items).into_bytes() }))
        }
        ".glb" => {
            let items = crate::items(world, id);
            Some(Ok(FileOut { content_type: "model/gltf-binary", bytes: model::glb(&view, &board, &items) }))
        }
        "-both.pos" => txt(pos::ascii(&view)),
        "-pos.csv" => Some(Ok(FileOut { content_type: "text/csv; charset=utf-8", bytes: pos::csv(&view).into_bytes() })),
        "-top-pnp.gbr" => txt(pos::gerber_x3(&view, true)),
        "-bottom-pnp.gbr" => txt(pos::gerber_x3(&view, false)),
        other => {
            let layer = other.strip_prefix('-')?.strip_suffix(".svg")?;
            plot::layers(&view).into_iter().find(|l| l.file == layer).map(|l| Ok(FileOut { content_type: "image/svg+xml", bytes: plot::svg(&view, Some(&l)).into_bytes() }))
        }
    }
}

/// Files written from a schematic, with what each is.
pub fn schematic_exports(name: &str, sheets: usize) -> Vec<(String, &'static str)> {
    let s = crate::gerber::file_stem(name);
    let mut v = vec![
        (format!("{s}.kicad_sch"), "KiCad schematic (root sheet; sub-sheets in the bundle)"),
        (format!("{s}-kicad-sch.zip"), "KiCad schematic with every sheet"),
        (format!("{s}.net"), "KiCad netlist (S-expression)"),
        (format!("{s}-netlist.xml"), "KiCad XML netlist"),
        (format!("{s}-orcad.net"), "OrcadPCB2 netlist"),
        (format!("{s}-pads.net"), "PADS netlist"),
        (format!("{s}-allegro.net"), "Allegro (Telesis) netlist"),
        (format!("{s}-cadstar.frp"), "CADSTAR netlist"),
    ];
    for i in 1..=sheets {
        v.push((format!("{s}-sheet{i}.svg"), "Sheet plot (SVG)"));
    }
    v
}

/// One export of a schematic by file name.
pub fn schematic_file(world: &World, id: StableId, name: &str) -> Option<Result<FileOut, String>> {
    let v = crate::schematic::view(world, id)?;
    let s = crate::gerber::file_stem(&v.name);
    let rest = name.strip_prefix(&s)?;
    let net = || netlist::from_schematic(&v);
    match rest {
        ".kicad_sch" => kicad_sch::write_world(world, id, &s).and_then(|f| f.into_iter().next()).and_then(|(_, t)| txt(t)),
        "-kicad-sch.zip" => {
            let files: Vec<(String, Vec<u8>)> = kicad_sch::write_world(world, id, &s)?.into_iter().map(|(n, t)| (n, t.into_bytes())).collect();
            Some(Ok(FileOut { content_type: "application/zip", bytes: de_shape::fabricate::zip(&files) }))
        }
        ".net" => txt(netlist::kicad(&net(), &v.name)),
        "-netlist.xml" => Some(Ok(FileOut { content_type: "application/xml", bytes: netlist::kicad_xml(&net(), &v.name).into_bytes() })),
        "-orcad.net" => txt(netlist::orcad(&net())),
        "-pads.net" => txt(netlist::pads(&net())),
        "-allegro.net" => txt(netlist::allegro(&net())),
        "-cadstar.frp" => txt(netlist::cadstar(&net(), &v.name)),
        other => {
            let i: usize = other.strip_prefix("-sheet")?.strip_suffix(".svg")?.parse().ok()?;
            let sh = v.sheets.get(i.checked_sub(1)?)?;
            Some(Ok(FileOut { content_type: "image/svg+xml", bytes: plot::schematic_svg(&v, &sh.id).into_bytes() }))
        }
    }
}

// ---- Small shared helpers ------------------------------------------------------------

/// Now as (year, month, day, hour, minute, second), UTC.
pub(crate) fn now() -> (i64, i64, i64, i64, i64, i64) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, rem / 3600, (rem / 60) % 60, rem % 60)
}

/// Today, ISO 8601 (2026-10-12).
pub(crate) fn today() -> String {
    let (y, m, d, ..) = now();
    format!("{y:04}-{m:02}-{d:02}")
}

/// Now, ISO 8601 with a UTC offset (2026-10-12T09:30:00+00:00).
pub(crate) fn now_iso() -> String {
    let (y, m, d, h, mi, s) = now();
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}+00:00")
}

/// A number with up to `dp` decimals, no trailing zeros.
pub(crate) fn num(v: f64, dp: usize) -> String {
    let s = format!("{:.*}", dp, if v.abs() < 0.5 * 10f64.powi(-(dp as i32)) { 0.0 } else { v });
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s }
}
