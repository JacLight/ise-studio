//! OpenSCAD files in and out.
//!
//! An imported .scad file becomes a shape whose program runs the file through
//! the OpenSCAD reader (`scad.js`) with its top-level number values replaced
//! by the shape's parameters, so the model stays as parametric as the file
//! was: `width = 40; // [20:100]` becomes a parameter `width` from 20 to 100
//! that people scrub and formulas link. Export writes the file back with the
//! parameters' current values (OpenSCAD's last assignment wins, so they are
//! appended); any other shape exports as a polyhedron per part.

use crate::{Param, runtime};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;

/// A top-level value of a .scad file.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ScadParam {
    pub name: String,
    pub value: f64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub description: String,
}

/// The file's top-level number values (and customizer ranges).
pub fn parameters(source: &str) -> Result<Vec<ScadParam>, String> {
    let text = runtime::scad_query("parameters", source)?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

const MARK: &str = "const SCAD = `";

/// The program that builds a .scad source live, its values from `p`.
pub fn program(file: &str, source: &str, params: &[ScadParam], material: Option<&str>, process: Option<&str>) -> String {
    let body = source.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${");
    let values = params.iter().map(|p| format!("{0}: p.{0}", p.name)).collect::<Vec<_>>().join(", ");
    let mut options = Vec::new();
    if let Some(m) = material {
        options.push(format!("material: {}", serde_json::to_string(m).unwrap()));
    }
    if let Some(p) = process {
        options.push(format!("process: {}", serde_json::to_string(p).unwrap()));
    }
    format!(
        "// Imported from “{file}”: the OpenSCAD source, read live. Its top-level\n// values are this shape's parameters; edit the source below to change the design.\n{MARK}{body}`;\nreturn openscad(SCAD, {{ {values} }}{});\n",
        if options.is_empty() { String::new() } else { format!(", {{ {} }}", options.join(", ")) }
    )
}

/// The .scad source inside a program made by [`program`], if it is one.
pub fn embedded(program: &str) -> Option<String> {
    let start = program.find(MARK)? + MARK.len();
    let mut out = String::new();
    let mut chars = program[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '`' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

/// What a parameter of a .scad file measures, from its name.
fn unit_for(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if ["angle", "deg", "rot", "tilt", "twist", "slope"].iter().any(|w| n.contains(w)) {
        "deg"
    } else if ["count", "num", "sides", "teeth", "holes", "slots", "spokes", "copies", "steps", "segments", "rows", "cols", "columns", "fn"].iter().any(|w| n.contains(w)) || n == "n" {
        "count"
    } else {
        "mm"
    }
}

pub fn ise_params(params: &[ScadParam]) -> Vec<Param> {
    params
        .iter()
        .map(|p| {
            let label = if p.description.is_empty() || p.description.chars().count() > 60 {
                let mut s = p.name.replace('_', " ");
                if let Some(f) = s.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                s
            } else {
                p.description.clone()
            };
            // A customizer range that does not hold the value is widened to it.
            let min = p.min.map(|m| m.min(p.value));
            let max = p.max.map(|m| m.max(p.value));
            Param { key: p.name.clone(), label, value: p.value, unit: unit_for(&p.name).into(), min, max, step: p.step, expr: None }
        })
        .collect()
}

/// The file with the parameters' values now (OpenSCAD's last assignment wins).
pub fn export_source(program_source: &str, values: &[(String, f64)]) -> Option<String> {
    let src = embedded(program_source)?;
    let mut out = src.trim_end().to_string();
    if !values.is_empty() {
        out.push_str("\n\n// Values from ISE Studio (OpenSCAD's last assignment wins)\n");
        for (k, v) in values {
            out.push_str(&format!("{k} = {};\n", fmt(*v)));
        }
    }
    Some(out)
}

fn fmt(v: f64) -> String {
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// A part as an OpenSCAD polyhedron (Iṣẹ́ Studio's counter-clockwise triangles
/// turned clockwise, as OpenSCAD wants them seen from outside).
pub fn polyhedron(name: &str, color: Option<&str>, positions: &[f32], indices: &[u32]) -> String {
    let pts: Vec<String> = positions.chunks_exact(3).map(|p| format!("[{},{},{}]", fmt(p[0] as f64), fmt(p[1] as f64), fmt(p[2] as f64))).collect();
    let faces: Vec<String> = indices.chunks_exact(3).map(|t| format!("[{},{},{}]", t[0], t[2], t[1])).collect();
    let body = format!("polyhedron(\n  points = [{}],\n  faces = [{}]\n);", pts.join(","), faces.join(","));
    let body = match color {
        Some(c) => format!("color({}) {body}", serde_json::to_string(c).unwrap()),
        None => body,
    };
    format!("// {name}\n{body}\n")
}

/// `shape.import_scad`: an OpenSCAD file as a live, parametric shape.
#[derive(Deserialize, JsonSchema)]
pub struct ImportScad {
    /// The .scad source text.
    pub source: String,
    /// Shape name (default: from `file`).
    #[serde(default)]
    pub name: Option<String>,
    /// The file name, for the program's note.
    #[serde(default)]
    pub file: Option<String>,
    /// Material name for its part(s) (mapped through the shape's materials table).
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub process: Option<String>,
    /// Only a definition, to place through instances.
    #[serde(default)]
    pub definition: bool,
}

impl Command for ImportScad {
    const NAME: &'static str = "shape.import_scad";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let bad = |m: String| CommandError::new(ErrorCode::InvalidParameters, m);
        if self.source.trim().is_empty() {
            return Err(bad("the .scad file is empty".into()));
        }
        let ps = parameters(&self.source).map_err(|e| bad(format!("the .scad file does not read: {e}")))?;
        let file = self.file.clone().unwrap_or_else(|| "model.scad".into());
        let name = self.name.clone().unwrap_or_else(|| {
            let stem = file.rsplit_once('.').map(|(n, _)| n).unwrap_or(&file).replace(['_', '-'], " ");
            let mut s = stem.trim().to_string();
            if let Some(f) = s.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            if s.is_empty() { "OpenSCAD model".into() } else { s }
        });
        let source = program(&file, &self.source, &ps, self.material.as_deref(), self.process.as_deref());
        let params = ise_params(&ps);
        let out = tx.run(
            "shape.create",
            json!({ "name": name, "description": format!("Imported from {file} (OpenSCAD)."), "source": source, "params": params, "definition": self.definition }),
        )?;
        Ok(json!({ "id": out["id"], "name": name, "parameters": ps.iter().map(|p| &p.name).collect::<Vec<_>>() }))
    }
}

/// Reads the id out of a command's output.
pub fn output_id(v: &Value) -> Option<StableId> {
    serde_json::from_value(v.get("id")?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_round_trip() {
        let src = "w = 20; // [10:50]\n// Height of the block\nh = 5;\ns = \"a`b${c}\\\\n\";\ncube([w, w, h]);\n";
        let ps = parameters(src).unwrap();
        assert_eq!(ps.len(), 2);
        assert_eq!((ps[0].name.as_str(), ps[0].value, ps[0].min, ps[0].max), ("w", 20.0, Some(10.0), Some(50.0)));
        assert_eq!((ps[1].name.as_str(), ps[1].description.as_str()), ("h", "Height of the block"));
        let prog = program("block.scad", src, &ps, None, None);
        assert_eq!(embedded(&prog).as_deref(), Some(src));
        assert!(prog.contains("openscad(SCAD, { w: p.w, h: p.h })"));
        let out = export_source(&prog, &[("w".into(), 32.5), ("h".into(), 5.0)]).unwrap();
        assert!(out.ends_with("w = 32.5;\nh = 5;\n"));
        let fp = ise_params(&ps);
        assert_eq!((fp[1].label.as_str(), fp[0].unit.as_str()), ("Height of the block", "mm"));
        assert_eq!(unit_for("tilt_angle"), "deg");
        assert_eq!(unit_for("teeth"), "count");
    }

    #[test]
    fn parse_errors_say_where() {
        let e = parameters("a = 1;\ncube([1,2,3]\n").unwrap_err();
        assert!(e.contains("line"), "{e}");
        let e = parameters("include <MCAD/gears.scad>\n").unwrap_err();
        assert!(e.contains("paste"), "{e}");
    }
}
