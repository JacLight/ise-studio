//! Tables in and out: CSV and Excel workbooks (.xlsx), for sheets, parameter
//! sets and configuration tables, and the zip container both XLSX files and
//! project bundles use.
//!
//! Everything here is plain data: a table is rows of cells, a cell what was
//! typed in it (a number, text, or a formula starting with `=`) and, for a
//! formula read from a workbook, the value the workbook last worked out.

use std::collections::BTreeMap;

/// One cell as read from a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InCell {
    /// What it holds: a number or text as written, or a formula (`=B1*2`).
    pub input: String,
    /// For a formula, the value its workbook last worked out (Excel keeps it).
    pub cached: Option<String>,
}

/// A table read from a file: cells by address ("B2"), and the names a
/// workbook gives single cells (its defined names), which become aliases.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InTable {
    pub cells: BTreeMap<(u32, u32), InCell>,
    /// name → (column, row), both from 0.
    pub names: BTreeMap<String, (u32, u32)>,
}

impl InTable {
    /// Rows top to bottom, cells left to right ("" where empty), as text.
    pub fn rows(&self) -> Vec<Vec<String>> {
        let rows = self.cells.keys().map(|(_, r)| *r + 1).max().unwrap_or(0);
        let cols = self.cells.keys().map(|(c, _)| *c + 1).max().unwrap_or(0);
        (0..rows)
            .map(|r| (0..cols).map(|c| self.cells.get(&(c, r)).map(|x| x.cached.clone().filter(|_| x.input.starts_with('=')).unwrap_or_else(|| x.input.clone())).unwrap_or_default()).collect())
            .collect()
    }
}

// ---- Addresses ---------------------------------------------------------------------

/// Column letters for a 0-based column: 0 → A, 25 → Z, 26 → AA.
pub fn column_name(mut c: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// "B12" → (1, 11); `$` signs (absolute references) are ignored.
pub fn parse_address(a: &str) -> Option<(u32, u32)> {
    let a: String = a.chars().filter(|c| *c != '$').collect();
    let letters = a.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    if letters == 0 || letters > 3 || letters == a.len() {
        return None;
    }
    let (l, d) = a.split_at(letters);
    if !d.chars().all(|c| c.is_ascii_digit()) || d.starts_with('0') {
        return None;
    }
    let mut col: u32 = 0;
    for ch in l.chars() {
        col = col * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    let row: u32 = d.parse().ok()?;
    if row == 0 || row > 1_048_576 {
        return None;
    }
    Some((col - 1, row - 1))
}

pub fn address(c: u32, r: u32) -> String {
    format!("{}{}", column_name(c), r + 1)
}

// ---- CSV ------------------------------------------------------------------------------

/// Reads CSV (RFC 4180: quoted fields, doubled quotes, line breaks in quotes).
/// The separator is found from the first line: comma, semicolon or tab.
pub fn read_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first = text.lines().next().unwrap_or("");
    let sep = [',', ';', '\t'].into_iter().max_by_key(|s| first.matches(*s).count()).filter(|s| first.contains(*s)).unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut field));
        } else if c == '\r' {
            // \r\n or a lone \r ends the row
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
            any = false;
        } else if c == '\n' {
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
            any = false;
        } else {
            field.push(c);
        }
    }
    if any {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Writes CSV, quoting only where needed.
pub fn write_csv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .map(|f| if f.contains([',', '"', '\n', '\r']) || f.starts_with(' ') || f.ends_with(' ') { format!("\"{}\"", f.replace('"', "\"\"")) } else { f.clone() })
            .collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

/// CSV as a table (each field what was typed).
pub fn csv_table(text: &str) -> InTable {
    let mut t = InTable::default();
    for (r, row) in read_csv(text).into_iter().enumerate() {
        for (c, v) in row.into_iter().enumerate() {
            if !v.trim().is_empty() {
                t.cells.insert((c as u32, r as u32), InCell { input: v.trim().to_string(), cached: None });
            }
        }
    }
    t
}

// ---- Zip ---------------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c = table[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// A zip archive of these files, deflated (stored when that is no smaller).
pub fn zip_deflated(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let crc = crc32(data);
        let packed = miniz_oxide::deflate::compress_to_vec(data, 6);
        let (method, body): (u16, &[u8]) = if packed.len() < data.len() { (8, &packed) } else { (0, data) };
        let offset = out.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central {
                h.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            h.extend_from_slice(&20u16.to_le_bytes()); // needed
            h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // time
            h.extend_from_slice(&0x21u16.to_le_bytes()); // date 1980-01-01
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(body.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            if central {
                h.extend_from_slice(&0u16.to_le_bytes()); // comment
                h.extend_from_slice(&0u16.to_le_bytes()); // disk
                h.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
                h.extend_from_slice(&0u32.to_le_bytes()); // external attrs
                h.extend_from_slice(&offset.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(header(0x0403_4b50, false));
        out.extend_from_slice(body);
        central.extend(header(0x0201_4b50, true));
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The files in a zip archive (stored or deflated), by name. Sizes are
/// capped so a hostile archive cannot exhaust memory.
pub fn unzip(data: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    const LIMIT: usize = 1 << 30;
    let u16_at = |i: usize| data.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or("the zip file is cut short");
    let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or("the zip file is cut short");
    // The end record is in the last 64 KiB + 22 bytes.
    let end = (0..data.len().saturating_sub(21)).rev().take(65_557).find(|&i| data[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06])).ok_or("not a zip file")?;
    let count = u16_at(end + 10)?;
    let mut at = u32_at(end + 16)?;
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 {
            return Err("the zip file's directory is damaged".into());
        }
        let method = u16_at(at + 10)?;
        let packed = u32_at(at + 20)?;
        let size = u32_at(at + 24)?;
        let (n, x, k) = (u16_at(at + 28)?, u16_at(at + 30)?, u16_at(at + 32)?);
        let local = u32_at(at + 42)?;
        let name = String::from_utf8_lossy(data.get(at + 46..at + 46 + n).ok_or("the zip file is cut short")?).to_string();
        at += 46 + n + x + k;
        if name.ends_with('/') {
            continue;
        }
        total += size;
        if total > LIMIT {
            return Err("the zip file unpacks to more than 1 GB".into());
        }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let body = data.get(start..start + packed).ok_or("the zip file is cut short")?;
        let bytes = match method {
            0 => body.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(body, size.max(1) + 64).map_err(|e| format!("{name}: cannot unpack ({e:?})"))?,
            m => return Err(format!("{name}: packed with method {m}, which Forma does not read")),
        };
        files.insert(name, bytes);
    }
    Ok(files)
}

// ---- A little XML ------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Xml<'a> {
    Open(&'a str, Vec<(&'a str, String)>, bool),
    Close(&'a str),
    Text(String),
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';') else { break };
        let ent = &rest[1..j];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") => out.extend(u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)),
            _ if ent.starts_with('#') => out.extend(ent[1..].parse::<u32>().ok().and_then(char::from_u32)),
            _ => out.push_str(&rest[..=j]),
        }
        rest = &rest[j + 1..];
    }
    out.push_str(rest);
    out
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Tags (local names, prefixes dropped) and text, in order.
fn xml(src: &str) -> Vec<Xml<'_>> {
    let mut out = Vec::new();
    let mut rest = src;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('<') {
            if r.starts_with('?') || r.starts_with('!') {
                let end = if r.starts_with("![CDATA[") {
                    let e = r.find("]]>").unwrap_or(r.len());
                    out.push(Xml::Text(r[8..e.min(r.len())].to_string()));
                    e + 3
                } else {
                    r.find('>').map(|e| e + 1).unwrap_or(r.len())
                };
                rest = &r[end.min(r.len())..];
                continue;
            }
            let end = r.find('>').unwrap_or(r.len());
            let tag = &r[..end];
            rest = &r[(end + 1).min(r.len())..];
            if let Some(name) = tag.strip_prefix('/') {
                let name = name.trim();
                out.push(Xml::Close(name.rsplit(':').next().unwrap_or(name)));
                continue;
            }
            let selfclosing = tag.ends_with('/');
            let tag = tag.trim_end_matches('/');
            let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
            let name = &tag[..name_end];
            let mut attrs = Vec::new();
            let mut a = &tag[name_end..];
            while let Some(eq) = a.find('=') {
                let key = a[..eq].trim();
                let after = a[eq + 1..].trim_start();
                let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
                let Some(close) = after[1..].find(q) else { break };
                attrs.push((key.rsplit(':').next().unwrap_or(key), unescape(&after[1..1 + close])));
                a = &after[close + 2..];
            }
            out.push(Xml::Open(name.rsplit(':').next().unwrap_or(name), attrs, selfclosing));
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            out.push(Xml::Text(unescape(&rest[..end])));
            rest = &rest[end..];
        }
    }
    out
}

fn attr<'a>(attrs: &'a [(&str, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
}

// ---- XLSX in ---------------------------------------------------------------------------

/// The worksheets of a workbook, by name, in order.
pub fn xlsx_sheet_names(data: &[u8]) -> Result<Vec<String>, String> {
    let files = unzip(data)?;
    Ok(workbook_sheets(&files)?.into_iter().map(|(n, _)| n).collect())
}

fn text_of(files: &BTreeMap<String, Vec<u8>>, name: &str) -> Option<String> {
    files.get(name).map(|b| String::from_utf8_lossy(b).to_string())
}

/// (sheet name, worksheet part path) in workbook order.
fn workbook_sheets(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<(String, String)>, String> {
    let wb = text_of(files, "xl/workbook.xml").ok_or("not an Excel workbook (no xl/workbook.xml)")?;
    let rels = text_of(files, "xl/_rels/workbook.xml.rels").unwrap_or_default();
    let mut targets = BTreeMap::new();
    for t in xml(&rels) {
        if let Xml::Open("Relationship", a, _) = t
            && let (Some(id), Some(target)) = (attr(&a, "Id"), attr(&a, "Target"))
        {
            let target = target.trim_start_matches('/');
            let path = if target.starts_with("xl/") { target.to_string() } else { format!("xl/{target}") };
            targets.insert(id.to_string(), path);
        }
    }
    let mut out = Vec::new();
    for t in xml(&wb) {
        if let Xml::Open("sheet", a, _) = t {
            let name = attr(&a, "name").unwrap_or("Sheet").to_string();
            let path = attr(&a, "id").and_then(|id| targets.get(id).cloned()).unwrap_or_else(|| format!("xl/worksheets/sheet{}.xml", out.len() + 1));
            out.push((name, path));
        }
    }
    if out.is_empty() {
        return Err("the workbook has no worksheets".into());
    }
    Ok(out)
}

/// Reads one worksheet of an .xlsx workbook (the first when `sheet` is None):
/// values, formulas with their last worked-out values, and the workbook's
/// names for single cells on that sheet.
pub fn read_xlsx(data: &[u8], sheet: Option<&str>) -> Result<InTable, String> {
    let files = unzip(data)?;
    let sheets = workbook_sheets(&files)?;
    let (name, path) = match sheet {
        Some(s) => sheets.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).cloned().ok_or_else(|| format!("the workbook has no sheet “{s}” (it has {})", sheets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")))?,
        None => sheets[0].clone(),
    };
    // Shared strings: each <si> is one string (rich text runs joined).
    let mut shared = Vec::new();
    if let Some(ss) = text_of(&files, "xl/sharedStrings.xml") {
        let mut cur: Option<String> = None;
        let mut in_t = false;
        let mut in_rph = false;
        for t in xml(&ss) {
            match t {
                Xml::Open("si", _, false) => cur = Some(String::new()),
                Xml::Open("si", _, true) => shared.push(String::new()),
                Xml::Close("si") => shared.push(cur.take().unwrap_or_default()),
                Xml::Open("rPh", _, false) => in_rph = true,
                Xml::Close("rPh") => in_rph = false,
                Xml::Open("t", _, sc) => in_t = !sc,
                Xml::Close("t") => in_t = false,
                Xml::Text(s) if in_t && !in_rph => {
                    if let Some(c) = cur.as_mut() {
                        c.push_str(&s)
                    }
                }
                _ => {}
            }
        }
    }
    let ws = text_of(&files, &path).ok_or_else(|| format!("the workbook's sheet “{name}” is missing"))?;
    let mut table = InTable::default();
    // Shared formulas: the master's text by its index (others reuse it shifted).
    let mut masters: BTreeMap<String, ((u32, u32), String)> = BTreeMap::new();
    let mut cell: Option<((u32, u32), String)> = None; // address, type
    let mut v: Option<String> = None;
    let mut f: Option<String> = None;
    let mut fshared: Option<(String, bool)> = None;
    let mut inline = String::new();
    let mut what = "";
    for t in xml(&ws) {
        match t {
            Xml::Open("c", a, sc) => {
                let at = attr(&a, "r").and_then(parse_address);
                let kind = attr(&a, "t").unwrap_or("n").to_string();
                v = None;
                f = None;
                fshared = None;
                inline.clear();
                cell = at.map(|x| (x, kind));
                if sc {
                    cell = None;
                }
            }
            Xml::Open("v", _, false) => what = "v",
            Xml::Open("f", a, sc) => {
                what = "f";
                if attr(&a, "t") == Some("shared")
                    && let Some(si) = attr(&a, "si")
                {
                    fshared = Some((si.to_string(), !sc));
                }
                if sc {
                    what = "";
                }
            }
            Xml::Open("t", _, false) => what = "t",
            Xml::Close("v") | Xml::Close("f") | Xml::Close("t") => what = "",
            Xml::Text(s) => match what {
                "v" => v = Some(v.unwrap_or_default() + &s),
                "f" => f = Some(f.unwrap_or_default() + &s),
                "t" => inline.push_str(&s),
                _ => {}
            },
            Xml::Close("c") => {
                let Some((at, kind)) = cell.take() else { continue };
                let value = match kind.as_str() {
                    "s" => v.as_deref().and_then(|i| i.trim().parse::<usize>().ok()).and_then(|i| shared.get(i).cloned()),
                    "inlineStr" => Some(inline.clone()),
                    "b" => v.as_deref().map(|b| if b.trim() == "1" { "TRUE".into() } else { "FALSE".into() }),
                    "e" => v.clone().map(|e| e.trim().to_string()),
                    _ => v.clone().map(|x| tidy_number(x.trim())),
                };
                let formula = match (&f, &fshared) {
                    (Some(text), Some((si, true))) if !text.trim().is_empty() => {
                        masters.insert(si.clone(), (at, text.clone()));
                        Some(text.clone())
                    }
                    (_, Some((si, _))) => masters.get(si).map(|(from, text)| shift_formula(text, at.0 as i64 - from.0 as i64, at.1 as i64 - from.1 as i64)),
                    (Some(text), None) if !text.trim().is_empty() => Some(text.clone()),
                    _ => None,
                };
                let c = match formula {
                    Some(fx) => InCell { input: format!("={fx}"), cached: value },
                    None => match value {
                        Some(x) if !x.is_empty() => InCell { input: x, cached: None },
                        _ => continue,
                    },
                };
                table.cells.insert(at, c);
            }
            _ => {}
        }
    }
    // Names for single cells on this sheet: Sheet1!$B$2 → alias.
    if let Some(wb) = text_of(&files, "xl/workbook.xml") {
        let mut current: Option<String> = None;
        let mut body = String::new();
        for t in xml(&wb) {
            match t {
                Xml::Open("definedName", a, false) => {
                    current = attr(&a, "name").map(str::to_string);
                    body.clear();
                }
                Xml::Text(s) if current.is_some() => body.push_str(&s),
                Xml::Close("definedName") => {
                    if let Some(n) = current.take()
                        && let Some((sh, cellref)) = body.rsplit_once('!')
                        && sh.trim_matches('\'').eq_ignore_ascii_case(&name)
                        && let Some(at) = parse_address(cellref)
                        && !n.starts_with("_xlnm")
                    {
                        table.names.insert(n, at);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(table)
}

/// A number as Excel stores it, without float noise (0.30000000000000004 → 0.3).
fn tidy_number(s: &str) -> String {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() => {
            let r = format!("{}", (v * 1e12).round() / 1e12);
            if r.parse::<f64>().is_ok_and(|x| (x - v).abs() <= 1e-9 * v.abs().max(1.0)) { r } else { s.to_string() }
        }
        _ => s.to_string(),
    }
}

/// Moves a formula's relative cell references by (dc, dr), as Excel does
/// when a shared formula is filled across cells. `$` parts stay put.
pub fn shift_formula(f: &str, dc: i64, dr: i64) -> String {
    let chars: Vec<char> = f.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            in_string = !in_string;
        }
        let starts = !in_string && (c == '$' || c.is_ascii_alphabetic()) && (i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '.'));
        if starts {
            // [$]letters[$]digits not followed by a name character or "("
            let mut j = i;
            let col_abs = chars[j] == '$';
            if col_abs {
                j += 1;
            }
            let ls = j;
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let le = j;
            let row_abs = j < chars.len() && chars[j] == '$';
            if row_abs {
                j += 1;
            }
            let ds = j;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            let ok = le > ls && le - ls <= 3 && j > ds && !(j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_' || chars[j] == '('));
            if ok {
                let letters: String = chars[ls..le].iter().collect();
                let digits: String = chars[ds..j].iter().collect();
                if let Some((col, row)) = parse_address(&format!("{letters}{digits}")) {
                    let col = if col_abs { col as i64 } else { col as i64 + dc };
                    let row = if row_abs { row as i64 } else { row as i64 + dr };
                    if col >= 0 && row >= 0 {
                        out.push_str(&format!("{}{}{}{}", if col_abs { "$" } else { "" }, column_name(col as u32), if row_abs { "$" } else { "" }, row + 1));
                        i = j;
                        continue;
                    }
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

// ---- XLSX out -------------------------------------------------------------------------------

/// One cell to write: a number, text, or a formula with its value now.
#[derive(Clone, Debug, PartialEq)]
pub enum OutCell {
    Number(f64),
    Text(String),
    /// Formula text without "=", Excel syntax, and its current value.
    Formula(String, Option<f64>),
}

/// A worksheet to write: cells by (column, row) from 0, column widths in characters.
#[derive(Clone, Debug, Default)]
pub struct OutSheet {
    pub name: String,
    pub cells: BTreeMap<(u32, u32), OutCell>,
    /// Defined names for single cells of this sheet (aliases).
    pub names: BTreeMap<String, (u32, u32)>,
    /// Rows to show in bold (headers).
    pub bold_rows: Vec<u32>,
}

/// An .xlsx workbook of these sheets.
pub fn write_xlsx(sheets: &[OutSheet]) -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let n = sheets.len().max(1);
    let mut types = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#);
    for i in 1..=n {
        types.push_str(&format!(r#"<Override PartName="/xl/worksheets/sheet{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#));
    }
    types.push_str("</Types>");
    files.push(("[Content_Types].xml".into(), types.into_bytes()));
    files.push((
        "_rels/.rels".into(),
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_vec(),
    ));
    let mut wb = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#);
    let names: Vec<String> = sheets.iter().enumerate().map(|(i, s)| sheet_name(&s.name, i)).collect();
    for (i, name) in names.iter().enumerate() {
        wb.push_str(&format!(r#"<sheet name="{}" sheetId="{}" r:id="rId{}"/>"#, escape(name), i + 1, i + 1));
    }
    wb.push_str("</sheets>");
    let defined: Vec<String> = sheets
        .iter()
        .zip(&names)
        .flat_map(|(s, sn)| s.names.iter().map(move |(n, (c, r))| format!(r#"<definedName name="{}">'{}'!${}${}</definedName>"#, escape(n), escape(&sn.replace('\'', "''")), column_name(*c), r + 1)))
        .collect();
    if !defined.is_empty() {
        wb.push_str(&format!("<definedNames>{}</definedNames>", defined.join("")));
    }
    wb.push_str("</workbook>");
    files.push(("xl/workbook.xml".into(), wb.into_bytes()));
    let mut rels = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for i in 1..=n {
        rels.push_str(&format!(r#"<Relationship Id="rId{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{i}.xml"/>"#));
    }
    rels.push_str(&format!(r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#, n + 1));
    files.push(("xl/_rels/workbook.xml.rels".into(), rels.into_bytes()));
    files.push((
        "xl/styles.xml".into(),
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs></styleSheet>"#.to_vec(),
    ));
    let empty = OutSheet::default();
    for i in 0..n {
        let s = sheets.get(i).unwrap_or(&empty);
        let mut x = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#);
        // Columns as wide as their longest text (Excel does not size them itself).
        let mut widths: BTreeMap<u32, usize> = BTreeMap::new();
        for ((c, _), v) in &s.cells {
            let w = match v {
                OutCell::Number(n) => format!("{n}").len(),
                OutCell::Text(t) => t.chars().count(),
                OutCell::Formula(_, v) => v.map(|n| format!("{n}").len()).unwrap_or(6),
            };
            let e = widths.entry(*c).or_insert(8);
            *e = (*e).max(w + 2).min(60);
        }
        if !widths.is_empty() {
            x.push_str("<cols>");
            for (c, w) in &widths {
                x.push_str(&format!(r#"<col min="{0}" max="{0}" width="{1}" customWidth="1"/>"#, c + 1, w));
            }
            x.push_str("</cols>");
        }
        x.push_str("<sheetData>");
        let mut rows: BTreeMap<u32, Vec<(u32, &OutCell)>> = BTreeMap::new();
        for ((c, r), v) in &s.cells {
            rows.entry(*r).or_default().push((*c, v));
        }
        for (r, mut cells) in rows {
            cells.sort_by_key(|(c, _)| *c);
            x.push_str(&format!(r#"<row r="{}">"#, r + 1));
            let style = if s.bold_rows.contains(&r) { r#" s="1""# } else { "" };
            for (c, v) in cells {
                let at = address(c, r);
                match v {
                    OutCell::Number(n) => x.push_str(&format!(r#"<c r="{at}"{style}><v>{n}</v></c>"#)),
                    OutCell::Text(t) => x.push_str(&format!(r#"<c r="{at}"{style} t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#, escape(t))),
                    OutCell::Formula(f, v) => x.push_str(&format!(r#"<c r="{at}"{style}><f>{}</f>{}</c>"#, escape(f), v.map(|v| format!("<v>{v}</v>")).unwrap_or_default())),
                }
            }
            x.push_str("</row>");
        }
        x.push_str("</sheetData></worksheet>");
        files.push((format!("xl/worksheets/sheet{}.xml", i + 1), x.into_bytes()));
    }
    zip_deflated(&files)
}

/// A worksheet name Excel accepts: up to 31 characters, none of []:*?/\.
fn sheet_name(name: &str, i: usize) -> String {
    let n: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if n.trim().is_empty() { format!("Sheet{}", i + 1) } else { n }
}

/// Turns rows of text into cells: numbers where they read as numbers.
pub fn out_cells(rows: &[Vec<String>]) -> BTreeMap<(u32, u32), OutCell> {
    let mut cells = BTreeMap::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            let cell = match v.trim().parse::<f64>() {
                Ok(n) if n.is_finite() && !v.trim().starts_with('+') => OutCell::Number(n),
                _ => OutCell::Text(v.clone()),
            };
            cells.insert((c as u32, r as u32), cell);
        }
    }
    cells
}

/// Reads a table from CSV text or XLSX bytes.
pub fn read_table(format: &str, bytes: &[u8], sheet: Option<&str>) -> Result<InTable, String> {
    match format.to_ascii_lowercase().trim_start_matches('.') {
        "csv" | "tsv" | "txt" => Ok(csv_table(&String::from_utf8_lossy(bytes))),
        "xlsx" | "xlsm" => read_xlsx(bytes, sheet),
        "xls" => Err("old .xls workbooks are not read: save it as .xlsx or CSV".into()),
        "ods" => Err("OpenDocument sheets are not read: save it as .xlsx or CSV".into()),
        f => Err(format!("unknown table format “{f}” (csv or xlsx)")),
    }
}

/// The bytes a command was given: `text` as is, or `base64` decoded.
pub fn payload(text: Option<&str>, base64: Option<&str>) -> Result<Vec<u8>, String> {
    if let Some(t) = text {
        return Ok(t.as_bytes().to_vec());
    }
    let b = base64.ok_or("give the file as `text` (CSV) or `base64` (XLSX)")?;
    decode_base64(b)
}

fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    let s = s.split_once("base64,").map(|(_, b)| b).unwrap_or(s);
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err("the base64 text has a character that is not base64".into()),
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(parse_address("B12"), Some((1, 11)));
        assert_eq!(parse_address("$AA$3"), Some((26, 2)));
        assert_eq!(parse_address("width"), None);
        assert_eq!(parse_address("A0"), None);
        assert_eq!(parse_address("B"), None);
        assert_eq!(address(27, 0), "AB1");
    }

    #[test]
    fn csv_round_trip() {
        let text = "name,value,note\r\nwidth,200,\"wide, really\"\nquote,\"say \"\"hi\"\"\",\"two\nlines\"\n";
        let rows = read_csv(text);
        assert_eq!(rows, vec![vec!["name", "value", "note"], vec!["width", "200", "wide, really"], vec!["quote", "say \"hi\"", "two\nlines"]]);
        let back = read_csv(&write_csv(&rows));
        assert_eq!(back, rows);
        // Semicolons (European spreadsheets) and tabs are found from the first line.
        assert_eq!(read_csv("a;b\n1,5;2\n"), vec![vec!["a", "b"], vec!["1,5", "2"]]);
        assert_eq!(read_csv("\u{feff}a\tb\n1\t2"), vec![vec!["a", "b"], vec!["1", "2"]]);
    }

    #[test]
    fn zip_round_trip() {
        let big = "repeat me ".repeat(1000).into_bytes();
        let files = vec![("a.txt".to_string(), b"hello".to_vec()), ("dir/b.txt".to_string(), big.clone())];
        let z = zip_deflated(&files);
        assert!(z.len() < big.len() / 4, "the repeated text is deflated");
        let back = unzip(&z).unwrap();
        assert_eq!(back["a.txt"], b"hello");
        assert_eq!(back["dir/b.txt"], big);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }

    #[test]
    fn xlsx_round_trip() {
        let mut cells = BTreeMap::new();
        cells.insert((0, 0), OutCell::Text("Width".into()));
        cells.insert((1, 0), OutCell::Number(200.0));
        cells.insert((0, 1), OutCell::Text("Half & more <x>".into()));
        cells.insert((1, 1), OutCell::Formula("B1/2+SUM(B1:B1)".into(), Some(300.0)));
        let mut names = BTreeMap::new();
        names.insert("width".to_string(), (1, 0));
        let bytes = write_xlsx(&[OutSheet { name: "Sizes".into(), cells, names, bold_rows: vec![0] }]);
        assert_eq!(xlsx_sheet_names(&bytes).unwrap(), vec!["Sizes"]);
        let t = read_xlsx(&bytes, None).unwrap();
        assert_eq!(t.cells[&(0, 0)].input, "Width");
        assert_eq!(t.cells[&(1, 0)].input, "200");
        assert_eq!(t.cells[&(0, 1)].input, "Half & more <x>");
        assert_eq!(t.cells[&(1, 1)], InCell { input: "=B1/2+SUM(B1:B1)".into(), cached: Some("300".into()) });
        assert_eq!(t.names["width"], (1, 0));
        assert!(read_xlsx(&bytes, Some("Nope")).unwrap_err().contains("Sizes"));
    }

    #[test]
    fn xlsx_as_excel_writes_it() {
        // Shared strings, a shared formula filled down, a boolean, a sheet in a sub-path.
        let wb = r#"<workbook xmlns:r="r"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets><definedNames><definedName name="total">Data!$B$4</definedName><definedName name="_xlnm.Print_Area">Data!$A$1:$B$4</definedName></definedNames></workbook>"#;
        let rels = r#"<Relationships><Relationship Id="rId1" Type="x" Target="/xl/worksheets/sheet1.xml"/></Relationships>"#;
        let ss = r#"<sst><si><t>Item</t></si><si><r><t>Le</t></r><r><t xml:space="preserve">g </t></r><rPh><t>x</t></rPh></si></sst>"#;
        let ws = r#"<worksheet><sheetData>
          <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>1.5</v></c><c r="C1"><f>B1*2</f><v>3</v></c></row>
          <row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>0.30000000000000004</v></c><c r="C2"><f t="shared" ref="C2:C3" si="0">B2*2</f><v>0.6</v></c></row>
          <row r="3"><c r="B3" t="b"><v>1</v></c><c r="C3"><f t="shared" si="0"/><v>2</v></c></row>
          <row r="4"><c r="B4"><f>SUM($B$1:B2)</f><v>1.8</v></c><c r="C4" t="e"><v>#DIV/0!</v></c></row>
        </sheetData></worksheet>"#;
        let z = zip_deflated(&[
            ("xl/workbook.xml".into(), wb.as_bytes().to_vec()),
            ("xl/_rels/workbook.xml.rels".into(), rels.as_bytes().to_vec()),
            ("xl/sharedStrings.xml".into(), ss.as_bytes().to_vec()),
            ("xl/worksheets/sheet1.xml".into(), ws.as_bytes().to_vec()),
        ]);
        let t = read_xlsx(&z, None).unwrap();
        assert_eq!(t.cells[&(0, 0)].input, "Item");
        assert_eq!(t.cells[&(0, 1)].input, "Leg ");
        assert_eq!(t.cells[&(1, 1)].input, "0.3");
        assert_eq!(t.cells[&(2, 0)].input, "=B1*2");
        assert_eq!(t.cells[&(2, 2)].input, "=B3*2", "the shared formula shifts down a row");
        assert_eq!(t.cells[&(1, 2)].input, "TRUE");
        assert_eq!(t.cells[&(1, 3)].input, "=SUM($B$1:B2)");
        assert_eq!(t.cells[&(2, 3)].input, "#DIV/0!");
        assert_eq!(t.names.len(), 1);
        assert_eq!(t.names["total"], (1, 3));
    }

    #[test]
    fn shifting() {
        assert_eq!(shift_formula("A1+$B$2+B$3+$C4+SUM(D1:D3)", 1, 2), "B3+$B$2+C$3+$C6+SUM(E3:E5)");
        assert_eq!(shift_formula("LOG10(A1)&\"A1\"", 0, 1), "LOG10(A2)&\"A1\"");
    }

    #[test]
    fn base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("data:application/x;base64,aGk=").unwrap(), b"hi");
    }
}
