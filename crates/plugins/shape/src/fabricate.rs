//! Fabrication output: what a shop needs to make each part.
//!
//! Every part leaves as its own file. Sheet parts (router, laser, plate) get a
//! layered DXF: the part is laid flat on its thinnest axis and cut at every
//! depth where its section changes, one layer per depth. Sheet-metal parts
//! built with sheet() get their flat pattern instead: the blank with its bend
//! lines (see `sheet`), plus a bend table. Everything gets an STL. A package
//! bundles all files with a cut list, identical parts merged into one line
//! with a quantity.

use crate::{ShapeProgram, evaluated_at, stl_of};

/// Exact STEP files for programs, each evaluated once (a cache by program key).
pub struct StepMaker {
    arena: de_occt::Arena,
    tolerance: f64,
    /// program key → (its parts by name, faceted notes) or why it failed.
    done: std::collections::HashMap<String, Result<(Vec<(String, de_occt::Shape)>, Vec<String>), String>>,
    /// program key → each part's material name (for colour in the file).
    materials: std::collections::HashMap<String, Vec<(String, Option<String>)>>,
}

impl StepMaker {
    pub fn new(tolerance: f64) -> StepMaker {
        StepMaker { arena: de_occt::Arena::new(), tolerance, done: Default::default(), materials: Default::default() }
    }

    /// The exact parts of a program (evaluated the first time it is asked for).
    pub fn parts(&mut self, program: &ShapeProgram) -> Result<(Vec<(String, de_occt::Shape)>, Vec<String>), String> {
        let key = program_key(program);
        if !self.done.contains_key(&key) {
            let values: Vec<(String, f64)> = program.params.iter().map(|p| (p.key.clone(), p.value)).collect();
            let arena = std::mem::take(&mut self.arena);
            let r = crate::exact::evaluate_exact_in(arena, &program.name, &program.source, &values, self.tolerance, &program.inputs);
            let entry = match r {
                Ok((arena, exact)) => {
                    self.arena = arena;
                    self.materials.insert(key.clone(), exact.parts.iter().map(|p| (p.name.clone(), p.material.clone())).collect());
                    Ok((exact.parts.into_iter().map(|p| (p.name, p.shape)).collect(), exact.faceted))
                }
                Err((arena, e)) => {
                    self.arena = arena;
                    Err(e)
                }
            };
            self.done.insert(key.clone(), entry);
        }
        self.done[&key].clone()
    }

    pub fn arena(&mut self) -> &mut de_occt::Arena {
        &mut self.arena
    }

    /// The material name of a program's part (once its parts are made).
    pub fn material(&self, program: &ShapeProgram, part: &str) -> Option<String> {
        self.materials.get(&program_key(program))?.iter().find(|(n, _)| n == part).and_then(|(_, m)| m.clone())
    }

    /// STEP bytes for named shapes in this maker's arena.
    pub fn step(&mut self, parts: &[(String, de_occt::Shape)]) -> Result<Vec<u8>, String> {
        let parts: Vec<_> = parts.iter().map(|(n, s)| (n.clone(), *s, None)).collect();
        self.file(&parts, de_occt::CadFormat::Step)
    }

    /// STEP or IGES bytes for named, optionally coloured shapes in this maker's arena.
    pub fn file(&mut self, parts: &[(String, de_occt::Shape, Option<[f64; 3]>)], format: de_occt::CadFormat) -> Result<Vec<u8>, String> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let ext = if format == de_occt::CadFormat::Iges { "igs" } else { "step" };
        let path = std::env::temp_dir().join(format!("forma-{}-{}.{ext}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let written = self.arena.write_cad(parts, &path, format);
        let bytes = written.and_then(|_| std::fs::read(&path).map_err(|e| e.to_string()));
        let _ = std::fs::remove_file(&path);
        bytes
    }

    /// One part of a program as its own STEP file (at the origin, like its STL).
    pub fn part_step(&mut self, program: &ShapeProgram, part: &str) -> Result<Vec<u8>, String> {
        self.part_file(program, part, None, de_occt::CadFormat::Step)
    }

    /// One part of a program as its own STEP or IGES file, at the origin.
    pub fn part_file(&mut self, program: &ShapeProgram, part: &str, color: Option<[f64; 3]>, format: de_occt::CadFormat) -> Result<Vec<u8>, String> {
        let (parts, _) = self.parts(program)?;
        let shape = parts.iter().find(|(n, _)| n == part).map(|(_, s)| *s).ok_or_else(|| format!("no part {part}"))?;
        let b = self.arena.bounds(shape)?;
        let at_origin = self.arena.translate(shape, [-b[0], -b[1], -b[2]])?;
        self.file(&[(part.to_string(), at_origin, color)], format)
    }
}
use manifold_csg::Manifold;
use std::fmt::Write as _;

/// Processes whose parts are cut from flat stock and want a DXF.
pub fn is_sheet(process: Option<&str>) -> bool {
    matches!(process, Some("cnc_router" | "laser" | "sheet_metal"))
}

/// One item to make, in its own coordinates (mm).
pub struct Item {
    pub name: String,
    pub material: Option<String>,
    pub process: Option<String>,
    pub solid: Manifold,
    pub volume_mm3: f64,
    /// Which program and part this is ("<program key>#<part>"), so its exact
    /// STEP can be made from the same program. Empty for plain boxes.
    pub key: String,
    /// A sheet-metal part's flat pattern (built with sheet()).
    pub flat: Option<crate::sheet_metal::FlatPattern>,
}

impl Item {
    /// The file the cutter takes: the flat pattern with bend lines for a
    /// sheet-metal part, else the part laid flat in depth layers.
    pub fn cut_file(&self) -> String {
        match &self.flat {
            Some(f) => crate::sheet_metal::dxf(f),
            None => dxf(&self.name, &self.solid),
        }
    }
}

/// A key naming a program with its parameter values (same program, same key).
pub fn program_key(program: &ShapeProgram) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    program.source.hash(&mut h);
    for p in &program.params {
        p.key.hash(&mut h);
        p.value.to_bits().hash(&mut h);
    }
    program.inputs.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// The program's parts (or its single solid) at a chosen accuracy.
pub fn items(program: &ShapeProgram, tolerance: f64) -> Result<Vec<Item>, String> {
    let e = evaluated_at(program, tolerance)?;
    let key = program_key(program);
    let flat = |part: &str, name: &str| -> Result<Option<crate::sheet_metal::FlatPattern>, String> {
        e.sheets.iter().find(|s| s.part == part).map(|s| crate::sheet_metal::FlatPattern::from_parts(name, &s.flat, &s.data)).transpose()
    };
    if e.parts.is_empty() {
        let flat = flat("", &program.name)?;
        let process = flat.as_ref().map(|_| "sheet_metal".to_string());
        return Ok(vec![Item { name: program.name.clone(), material: None, process, solid: e.solid.translate(0.0, 0.0, 0.0), volume_mm3: e.volume, key: format!("{key}#{}", program.name), flat }]);
    }
    e.parts
        .iter()
        .map(|p| {
            Ok(Item {
                name: p.name.clone(),
                material: p.material.clone(),
                process: p.process.clone(),
                solid: p.solid.translate(0.0, 0.0, 0.0),
                volume_mm3: p.volume,
                key: format!("{key}#{}", p.name),
                flat: flat(&p.name, &p.name)?,
            })
        })
        .collect()
}

/// Sizes along x, y, z.
fn size(solid: &Manifold) -> [f64; 3] {
    let b = solid.bounding_box().expect("non-empty solid");
    let (lo, hi) = (b.min(), b.max());
    [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]]
}

/// Moves a part so its bounding box starts at the origin (sitting on z = 0).
pub fn at_origin(solid: &Manifold) -> Manifold {
    let lo = solid.bounding_box().expect("non-empty solid").min();
    solid.translate(-lo[0], -lo[1], -lo[2])
}

/// Lays a part flat: its thinnest dimension becomes Z, the face with the
/// fullest outline down on the machine bed (so pockets are cut from above),
/// then it moves to the origin.
pub fn lay_flat(solid: &Manifold) -> Manifold {
    let s = size(solid);
    let thin = (0..3).min_by(|a, b| s[*a].total_cmp(&s[*b])).unwrap();
    let turned = at_origin(&match thin {
        0 => solid.rotate(0.0, 90.0, 0.0),
        1 => solid.rotate(90.0, 0.0, 0.0),
        _ => solid.translate(0.0, 0.0, 0.0),
    });
    let flipped = at_origin(&turned.rotate(180.0, 0.0, 0.0));
    let bed = |m: &Manifold| {
        let t = size(m)[2];
        m.slice_at_z(t * 1e-3).iter().map(|ring| area(ring)).sum::<f64>()
    };
    if bed(&flipped) > bed(&turned) + 1e-6 { flipped } else { turned }
}

/// Signed area of a ring (positive counter-clockwise, so holes subtract).
fn area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        a[0] * b[1] - b[0] * a[1]
    })
    .sum::<f64>()
        / 2.0
}

/// Overall dimensions, largest first: length × width × thickness (mm).
pub fn dimensions(solid: &Manifold) -> [f64; 3] {
    let mut s = size(solid);
    s.sort_by(|a, b| b.total_cmp(a));
    s
}

/// Heights (mm) of the flat faces of a part laid flat: where its section changes.
fn steps(solid: &Manifold) -> Vec<f64> {
    let (positions, n_props, indices) = solid.to_mesh_f32();
    let n = n_props.max(3);
    let p = |i: u32| {
        let i = i as usize * n;
        [positions[i] as f64, positions[i + 1] as f64, positions[i + 2] as f64]
    };
    let mut zs = Vec::new();
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let nz = u[0] * w[1] - u[1] * w[0];
        let len = ((u[1] * w[2] - u[2] * w[1]).powi(2) + (u[2] * w[0] - u[0] * w[2]).powi(2) + nz * nz).sqrt();
        if len > 0.0 && nz.abs() / len > 0.9999 {
            zs.push((a[2] + b[2] + c[2]) / 3.0);
        }
    }
    zs.sort_by(f64::total_cmp);
    zs.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    zs
}

/// A layered DXF (millimeters) of a part laid flat. Layer `DEPTH_<d>` is the
/// outline of material at depth `d` below the top face: clear everything
/// outside it to that depth. The deepest layer is the through cut.
pub fn dxf(name: &str, solid: &Manifold) -> String {
    let flat = lay_flat(solid);
    let top = size(&flat)[2];
    let zs = steps(&flat);
    let mut levels: Vec<f64> = zs.into_iter().filter(|z| *z > 1e-3 && *z < top - 1e-3).collect();
    levels.insert(0, 0.0);
    levels.push(top);

    let mut out = String::new();
    let mut pair = |code: i32, value: &str| {
        let _ = writeln!(out, "{code}\n{value}");
    };
    pair(999, &format!("Forma {name}: layer DEPTH_d = outline at depth d mm below the top face"));
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    pair(9, "$INSUNITS");
    pair(70, "4");
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for band in levels.windows(2) {
        let (lo, hi) = (band[0], band[1]);
        let layer = format!("DEPTH_{:.2}", top - lo);
        for ring in flat.slice_at_z((lo + hi) / 2.0) {
            if ring.len() < 3 {
                continue;
            }
            pair(0, "POLYLINE");
            pair(8, &layer);
            pair(66, "1");
            pair(70, "1");
            for [x, y] in &ring {
                pair(0, "VERTEX");
                pair(8, &layer);
                pair(10, &format!("{x:.4}"));
                pair(20, &format!("{y:.4}"));
            }
            pair(0, "SEQEND");
        }
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    out
}

/// A file-system-safe name.
pub fn file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "part".into() } else { s }
}

/// One line of a cut list: identical parts merged.
pub struct Line {
    pub names: Vec<String>,
    pub material: String,
    pub process: String,
    pub dimensions: [f64; 3],
    pub volume_mm3: f64,
    pub mass_kg: Option<f64>,
    pub files: Vec<String>,
}

/// A fabrication package: every distinct part's files plus `cut-list.csv`,
/// as (path, bytes). `assembly` prefixes the paths; `materials` resolves a
/// part's material name (or the default) and its density (kg/m³).
pub fn package(
    assembly: &str,
    items: &[Item],
    material_of: &dyn Fn(Option<&str>) -> (String, Option<f64>),
    step_of: &dyn Fn(&Item) -> Option<Result<Vec<u8>, String>>,
) -> (Vec<(String, Vec<u8>)>, Vec<Line>) {
    let mut lines: Vec<(Line, usize)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        // A sheet-metal part is ordered as its flat blank.
        let dims = match &item.flat {
            Some(f) => {
                let mut d = [f.size[0], f.size[1], f.thickness];
                d.sort_by(|a, b| b.total_cmp(a));
                d
            }
            None => dimensions(&item.solid),
        };
        let (material, density) = material_of(item.material.as_deref());
        let process = item.process.clone().unwrap_or_default();
        let same = |l: &Line| {
            l.material == material
                && l.process == process
                && (0..3).all(|k| (l.dimensions[k] - dims[k]).abs() < 0.01)
                && (l.volume_mm3 - item.volume_mm3).abs() <= 1e-6 * item.volume_mm3.max(1.0)
        };
        if let Some((line, _)) = lines.iter_mut().find(|(l, _)| same(l)) {
            line.names.push(item.name.clone());
            continue;
        }
        let line = Line {
            names: vec![item.name.clone()],
            material,
            process,
            dimensions: dims,
            volume_mm3: item.volume_mm3,
            mass_kg: density.map(|d| item.volume_mm3 / 1e9 * d),
            files: Vec::new(),
        };
        lines.push((line, i));
    }

    let dir = file_name(assembly);
    let mut files = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for (line, first) in &mut lines {
        let item = &items[*first];
        let base = format!("{dir}/{}", file_name(&item.name));
        let stl = format!("{base}.stl");
        files.push((stl.clone(), stl_of(&at_origin(&item.solid), &format!("Forma {}", item.name), [0.0; 3])));
        line.files.push(stl);
        if is_sheet(item.process.as_deref()) || item.flat.is_some() {
            let dxf_path = format!("{base}.dxf");
            files.push((dxf_path.clone(), item.cut_file().into_bytes()));
            line.files.push(dxf_path);
        }
        if let Some(f) = &item.flat {
            let path = format!("{base} bends.csv");
            files.push((path.clone(), crate::sheet_metal::bend_table_csv(f).into_bytes()));
            line.files.push(path);
            for c in f.checks.iter().filter(|c| c.severity != "info") {
                notes.push(format!("{}: {} {}: {}", item.name, c.severity, c.code, c.message));
            }
        }
        match step_of(item) {
            Some(Ok(bytes)) => {
                let path = format!("{base}.step");
                files.push((path.clone(), bytes));
                line.files.push(path);
            }
            Some(Err(e)) => notes.push(format!("{}: no STEP ({e})", item.name)),
            None => {}
        }
    }
    if !notes.is_empty() {
        files.push((format!("{dir}/NOTES.txt"), (notes.join("\n") + "\n").into_bytes()));
    }
    let lines: Vec<Line> = lines.into_iter().map(|(l, _)| l).collect();
    files.push((format!("{dir}/cut-list.csv"), cut_list(&lines).into_bytes()));
    (files, lines)
}

fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n']) { format!("\"{}\"", field.replace('"', "\"\"")) } else { field.to_string() }
}

/// Cut list: one row per distinct part, with quantity, stock size and mass.
pub fn cut_list(lines: &[Line]) -> String {
    let mut out = String::from("Part,Qty,Material,Process,Length mm,Width mm,Thickness mm,Volume cm3,Mass each kg,Mass total kg,Files\n");
    for l in lines {
        let qty = l.names.len();
        let name = if qty == 1 { l.names[0].clone() } else { format!("{} (+{} identical)", l.names[0], qty - 1) };
        let _ = writeln!(
            out,
            "{},{qty},{},{},{:.2},{:.2},{:.2},{:.1},{},{},{}",
            csv(&name),
            csv(&l.material),
            csv(&l.process),
            l.dimensions[0],
            l.dimensions[1],
            l.dimensions[2],
            l.volume_mm3 / 1000.0,
            l.mass_kg.map(|m| format!("{m:.3}")).unwrap_or_default(),
            l.mass_kg.map(|m| format!("{:.3}", m * qty as f64)).unwrap_or_default(),
            csv(&l.files.join(" ")),
        );
    }
    out
}

/// A 3MF file (what Bambu Studio, PrusaSlicer and Cura open natively) with
/// the solid as one object, millimetres, resting on z = 0. A solid made of
/// separate bodies stays one object: a print-in-place mechanism prints whole.
pub fn three_mf(name: &str, solid: &Manifold) -> Vec<u8> {
    use std::fmt::Write;
    let solid = at_origin(solid);
    let (props, n, idx) = solid.to_mesh_f64();
    let n = n.max(3);
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let mut m = String::with_capacity(props.len() * 12 + idx.len() * 12 + 512);
    m.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <metadata name="Application">Forma</metadata>
"#);
    let _ = writeln!(m, r#" <metadata name="Title">{}</metadata>"#, esc(name));
    let _ = writeln!(m, r#" <resources>
  <object id="1" type="model" name="{}">
   <mesh>
    <vertices>"#, esc(name));
    for v in props.chunks_exact(n) {
        let _ = writeln!(m, r#"     <vertex x="{:.5}" y="{:.5}" z="{:.5}"/>"#, v[0], v[1], v[2]);
    }
    m.push_str("    </vertices>\n    <triangles>\n");
    for t in idx.chunks_exact(3) {
        let _ = writeln!(m, r#"     <triangle v1="{}" v2="{}" v3="{}"/>"#, t[0], t[1], t[2]);
    }
    m.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n <build>\n  <item objectid=\"1\"/>\n </build>\n</model>\n");
    let types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>
"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>
"#;
    zip(&[
        ("[Content_Types].xml".into(), types.as_bytes().to_vec()),
        ("_rels/.rels".into(), rels.as_bytes().to_vec()),
        ("3D/3dmodel.model".into(), m.into_bytes()),
    ])
}

/// A zip archive, each file deflated where that makes it smaller (a 3MF's
/// mesh XML shrinks to about a fifth), else stored.
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        !data.iter().fold(!0u32, |c, b| table[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8))
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (path, data) in files {
        let crc = crc32(data);
        let packed = miniz_oxide::deflate::compress_to_vec(data, 6);
        let (method, body): (u16, &[u8]) = if packed.len() < data.len() { (8, &packed) } else { (0, data) };
        let offset = out.len() as u32;
        let name = path.as_bytes();
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            h.extend_from_slice(&20u16.to_le_bytes()); // needed
            h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
            h.extend_from_slice(&method.to_le_bytes()); // 8 deflated, 0 stored
            h.extend_from_slice(&0u16.to_le_bytes()); // time
            h.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(body.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            if central {
                h.extend_from_slice(&[0; 6]); // comment length, disk, internal attributes
                h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name);
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend_from_slice(body);
        central.extend(header(0x0201_4b50, true));
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
