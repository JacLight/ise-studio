//! Requests for quote as packages a person sends: one folder (or zip) per
//! supplier holding only the lines that supplier's kind covers, each with the
//! files a shop needs for its process (STEP + drawing PDF for machined parts,
//! STL + 3MF for prints, DXF for sheet parts), the BOM of those lines as CSV
//! and a cover letter (`RFQ.md`). Nothing here sends anything.

use crate::{
    Rfq, Supplier,
    bom::{Bom, csv, files_for},
};
use de_core::StableId;
use de_shape::{ShapeProgram, drawing, fabricate};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

/// Who is asking, for the cover letter and title blocks.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The project's title.
    pub project: String,
    /// Today, YYYY-MM-DD.
    pub date: String,
    /// The person sending it (signs the letter; "" leaves a blank to sign).
    pub from: String,
}

/// One line of an RFQ as one supplier gets it.
#[derive(Clone, Debug)]
pub struct Planned {
    pub key: String,
    pub item: usize,
    pub name: String,
    /// None: a bought item.
    pub process: Option<String>,
    pub material: Option<String>,
    /// Per finished unit.
    pub qty: u32,
    /// File extensions sent with it.
    pub files: Vec<&'static str>,
    /// Gone from the design since the RFQ was made.
    pub missing: bool,
    pub part_number: Option<String>,
    pub manufacturer: Option<String>,
    pub mpn: Option<String>,
    pub description: Option<String>,
    pub pack: Option<u32>,
    pub size_mm: Option<[f64; 3]>,
}

/// The lines of `rfq` a supplier is asked about: those its kind covers.
pub fn plan(rfq: &Rfq, bom: &Bom, supplier: &Supplier) -> Vec<Planned> {
    let mut out = vec![];
    for rl in &rfq.lines {
        if let Some(l) = bom.make_line(&rl.line) {
            if !supplier.covers(l.process.as_deref()) {
                continue;
            }
            out.push(Planned {
                key: l.key.clone(),
                item: l.item,
                name: l.name.clone(),
                process: Some(l.process.clone().unwrap_or_default()),
                material: l.material.clone(),
                qty: l.qty,
                files: files_for(l.process.as_deref()),
                missing: false,
                part_number: None,
                manufacturer: None,
                mpn: None,
                description: None,
                pack: None,
                size_mm: Some(l.size_mm),
            });
        } else if let Some(l) = bom.buy_line(&rl.line) {
            if !supplier.covers(None) {
                continue;
            }
            let s = l.stock.as_ref();
            out.push(Planned {
                key: l.key.clone(),
                item: l.item,
                name: l.name.clone(),
                process: None,
                material: None,
                qty: l.qty,
                files: vec![],
                missing: false,
                part_number: s.map(|s| s.part_number.clone()),
                manufacturer: s.and_then(|s| s.manufacturer.clone()),
                mpn: s.and_then(|s| s.mpn.clone()),
                description: s.and_then(|s| s.description.clone()),
                pack: s.and_then(|s| s.pack),
                size_mm: None,
            });
        } else {
            // Gone from the design: still listed, without files, so the request reads.
            let bought = rl.line.starts_with("buy:");
            if (bought && !supplier.covers(None)) || (!bought && supplier.kind.iter().all(|k| k == "distributor") && !supplier.kind.is_empty()) {
                continue;
            }
            out.push(Planned { key: rl.line.clone(), item: 0, name: rl.name.clone(), process: None, material: None, qty: rl.qty, files: vec![], missing: true, part_number: None, manufacturer: None, mpn: None, description: None, pack: None, size_mm: None });
        }
    }
    out
}

/// A file's name in a package: item number and part name.
pub fn file_name(p: &Planned, ext: &str) -> String {
    format!("{:02}-{}.{ext}", p.item, fabricate::file_name(&p.name))
}

/// Makes the files of Make lines, each once however many suppliers get it.
pub struct Files<'a> {
    programs: &'a BTreeMap<String, (ShapeProgram, Option<String>)>,
    tolerance: f64,
    maker: fabricate::StepMaker,
    done: HashMap<(String, &'static str), Result<Vec<u8>, String>>,
    context: Context,
}

impl<'a> Files<'a> {
    pub fn new(programs: &'a BTreeMap<String, (ShapeProgram, Option<String>)>, tolerance: f64, context: Context) -> Self {
        Files { programs, tolerance, maker: fabricate::StepMaker::new(tolerance), done: HashMap::new(), context }
    }

    /// One file of a Make line: "step", "stl", "3mf", "dxf" or "pdf".
    pub fn file(&mut self, key: &str, ext: &'static str, name: &str, material: Option<&str>, number: &str) -> Result<Vec<u8>, String> {
        let id = (key.to_string(), ext);
        if let Some(r) = self.done.get(&id) {
            return r.clone();
        }
        let r = self.make(key, ext, name, material, number);
        self.done.insert(id, r.clone());
        r
    }

    fn make(&mut self, key: &str, ext: &'static str, name: &str, material: Option<&str>, number: &str) -> Result<Vec<u8>, String> {
        let (program, part) = self.programs.get(key).ok_or_else(|| format!("{name}: its shape is gone"))?;
        let part_name = part.clone().unwrap_or_else(|| program.name.clone());
        match ext {
            "step" => self.maker.part_step(program, &part_name),
            "pdf" => {
                let options = drawing::Options { part: part.clone(), ..Default::default() };
                let title = drawing::Title {
                    title: name.to_string(),
                    number: number.to_string(),
                    material: material.unwrap_or("—").to_string(),
                    mass: "—".into(),
                    drawn_by: if self.context.from.is_empty() { "Forma".into() } else { self.context.from.clone() },
                    date: self.context.date.clone(),
                    parts: vec![],
                    page: 1,
                    pages: 1,
                };
                let sheets = drawing::drawings(program, &[(options, title)])?;
                Ok(drawing::pdf_pages(&sheets))
            }
            _ => {
                let items = fabricate::items(program, self.tolerance)?;
                let item = match part {
                    Some(n) => items.iter().find(|i| &i.name == n).ok_or_else(|| format!("no part {n}"))?,
                    None => items.first().ok_or("nothing to export")?,
                };
                Ok(match ext {
                    "stl" => de_shape::stl_of(&fabricate::at_origin(&item.solid), &format!("Forma {name}"), [0.0; 3]),
                    "3mf" => fabricate::three_mf(name, &item.solid),
                    "dxf" => fabricate::dxf(name, &item.solid).into_bytes(),
                    other => return Err(format!("unknown file kind .{other}")),
                })
            }
        }
    }
}

/// The BOM CSV of one supplier's lines, with an order quantity per production quantity.
pub fn lines_csv(rfq: &Rfq, lines: &[Planned]) -> String {
    let mut out = String::from("Item,Part,Part number,Manufacturer,MPN,Description,Material,Process,Length mm,Width mm,Thickness mm,Qty per unit");
    for q in &rfq.quantities {
        let _ = write!(out, ",Qty for {q} units");
    }
    out.push_str(",Pack,Files\n");
    for l in lines {
        let size = l.size_mm.map(|s| [s[0], s[1], s[2]].map(|v| format!("{v:.2}"))).unwrap_or_default();
        let _ = write!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            l.item,
            csv(&l.name),
            csv(l.part_number.as_deref().unwrap_or("")),
            csv(l.manufacturer.as_deref().unwrap_or("")),
            csv(l.mpn.as_deref().unwrap_or("")),
            csv(l.description.as_deref().unwrap_or("")),
            csv(l.material.as_deref().unwrap_or("")),
            csv(l.process.as_deref().unwrap_or(if l.missing { "" } else { "buy" })),
            size.first().cloned().unwrap_or_default(),
            size.get(1).cloned().unwrap_or_default(),
            size.get(2).cloned().unwrap_or_default(),
            l.qty
        );
        for q in &rfq.quantities {
            let _ = write!(out, ",{}", l.qty * q);
        }
        let files: Vec<String> = l.files.iter().map(|e| file_name(l, e)).collect();
        let _ = writeln!(out, ",{},{}", l.pack.map(|p| p.to_string()).unwrap_or_default(), csv(&files.join(" ")));
    }
    out
}

fn process_words(p: Option<&str>) -> &str {
    match p {
        Some("3d_print") => "3D print",
        Some("cnc_mill") => "CNC mill",
        Some("lathe") => "Lathe",
        Some("cnc_router") => "CNC router",
        Some("laser") => "Laser cut",
        Some("sheet_metal") => "Sheet metal",
        Some("weld") => "Welded",
        Some("cast") => "Cast",
        Some("extrusion") => "Cut extrusion",
        Some("glazing") => "Glazing",
        Some("") | None => "—",
        Some(other) => other,
    }
}

/// The cover letter (Markdown) for one supplier.
pub fn cover_letter(rfq: &Rfq, supplier: &Supplier, lines: &[Planned], context: &Context) -> String {
    let mut m = String::new();
    let _ = writeln!(m, "# Request for quote — {}\n", rfq.name);
    let project = if context.project.is_empty() { String::new() } else { format!("{} · ", context.project) };
    let _ = writeln!(m, "{project}{}\n", context.date);
    let _ = writeln!(m, "Dear {} team,\n", supplier.name);
    let qs: Vec<String> = rfq.quantities.iter().map(|q| q.to_string()).collect();
    let _ = writeln!(
        m,
        "Please quote the {} below for **{}** finished unit{}. The quantity columns are pieces of each line.{}\n",
        if lines.len() == 1 { "line" } else { "lines" },
        qs.join(", "),
        if rfq.quantities == [1] { "" } else { "s" },
        rfq.due.as_ref().map(|d| format!(" Quotes are wanted by **{d}**.")).unwrap_or_default()
    );
    let _ = write!(m, "| Item | Part | Material | Process |");
    for q in &rfq.quantities {
        let _ = write!(m, " Qty @ {q} |");
    }
    m.push_str(" Files |\n|---|---|---|---|");
    for _ in &rfq.quantities {
        m.push_str("---:|");
    }
    m.push_str("---|\n");
    for l in lines {
        let what = match (&l.part_number, l.process.is_none() && !l.missing) {
            (Some(pn), _) => format!("{pn} — {}", l.description.clone().unwrap_or_else(|| l.name.clone())),
            (None, true) => format!("{} (no part number yet)", l.name),
            (None, false) => l.name.clone(),
        };
        let what = if l.missing { format!("{what} *(changed in the design since — please skip)*") } else { what };
        let process = if l.process.is_none() && !l.missing { "Buy".to_string() } else { process_words(l.process.as_deref()).to_string() };
        let _ = write!(m, "| {} | {} | {} | {} |", l.item, what.replace('|', "/"), l.material.clone().unwrap_or_else(|| "—".into()), process);
        for q in &rfq.quantities {
            let _ = write!(m, " {} |", l.qty * q);
        }
        let files: Vec<String> = l.files.iter().map(|e| file_name(l, e)).collect();
        let _ = writeln!(m, " {} |", if files.is_empty() { "—".into() } else { files.join(", ") });
    }
    let bought: Vec<&Planned> = lines.iter().filter(|l| l.pack.is_some()).collect();
    if !bought.is_empty() {
        let _ = writeln!(m, "\nPack sizes we expect: {}. Quote your own pack size if it differs.", bought.iter().map(|l| format!("item {} in {}s", l.item, l.pack.unwrap())).collect::<Vec<_>>().join(", "));
    }
    let made = lines.iter().any(|l| l.process.is_some());
    if made {
        let _ = writeln!(m, "\n**Finish:** {}", if rfq.finish.trim().is_empty() { "none specified — please state the finish you quote" } else { rfq.finish.trim() });
        let _ = writeln!(m, "\n**Tolerances:** {}", if rfq.tolerances.trim().is_empty() { "ISO 2768-m unless a drawing states otherwise" } else { rfq.tolerances.trim() });
        let _ = writeln!(m, "\nThe drawings and models are the authority on geometry; the BOM.csv lists sizes and quantities for checking.");
    }
    if !rfq.notes.trim().is_empty() {
        let _ = writeln!(m, "\n**Notes:** {}", rfq.notes.trim());
    }
    let _ = writeln!(m, "\nFor each line please give the unit price at each quantity, your lead time, how long the quote is valid, and anything you would change to make it cheaper or quicker.\n");
    let _ = writeln!(m, "Thank you,\n\n{}", if context.from.is_empty() { "—" } else { &context.from });
    m
}

/// A supplier's package as (path, bytes), paths under `folder/` ("" for none).
pub fn supplier_package(rfq: &Rfq, bom: &Bom, supplier: &Supplier, files: &mut Files, folder: &str) -> (Vec<(String, Vec<u8>)>, Vec<String>) {
    let lines = plan(rfq, bom, supplier);
    let at = |name: &str| if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") };
    let mut out = vec![];
    let mut problems = vec![];
    let project = fabricate::file_name(&files.context.project).to_uppercase();
    for l in &lines {
        for ext in &l.files {
            let number = format!("{project}-{:02}", l.item);
            match files.file(&l.key, ext, &l.name, l.material.as_deref(), &number) {
                Ok(bytes) => out.push((at(&file_name(l, ext)), bytes)),
                Err(e) => problems.push(format!("{} ({ext}): {e}", l.name)),
            }
        }
    }
    out.push((at("BOM.csv"), lines_csv(rfq, &lines).into_bytes()));
    out.push((at("RFQ.md"), cover_letter(rfq, supplier, &lines, &files.context).into_bytes()));
    if !problems.is_empty() {
        out.push((at("NOTES.txt"), (problems.join("\n") + "\n").into_bytes()));
    }
    (out, problems)
}

/// Every supplier's package in one archive, a folder each.
pub fn rfq_package(rfq: &Rfq, bom: &Bom, suppliers: &[(StableId, Supplier)], files: &mut Files) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![];
    let mut used: HashMap<String, usize> = HashMap::new();
    for (_, s) in suppliers {
        let base = fabricate::file_name(&s.name);
        let n = used.entry(base.clone()).or_default();
        *n += 1;
        let folder = if *n == 1 { base } else { format!("{base}-{n}") };
        out.extend(supplier_package(rfq, bom, s, files, &folder).0);
    }
    out
}

/// A mailto: draft for one supplier (the person sends it, attaching the package).
pub fn mailto(rfq: &Rfq, supplier: &Supplier, lines: usize, context: &Context) -> String {
    fn enc(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    let subject = format!("Request for quote: {}", rfq.name);
    let qs: Vec<String> = rfq.quantities.iter().map(|q| q.to_string()).collect();
    let body = format!(
        "Dear {} team,\n\nPlease find attached our request for quote \"{}\": {} line{} at {} units{}. The attached RFQ.md lists quantities, materials and finish; BOM.csv and the drawings and models are included.\n\nThank you,\n{}",
        supplier.name,
        rfq.name,
        lines,
        if lines == 1 { "" } else { "s" },
        qs.join(" / "),
        rfq.due.as_ref().map(|d| format!(", quotes wanted by {d}")).unwrap_or_default(),
        context.from
    );
    format!("mailto:{}?subject={}&body={}", supplier.email, enc(&subject), enc(&body))
}
