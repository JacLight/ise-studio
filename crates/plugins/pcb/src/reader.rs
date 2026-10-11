//! A small, independent reader for the Gerber and Excellon this crate writes
//! (and the common subset others write): it parses the files back into
//! apertures, flashes, draws, regions and drill hits so tests can check what
//! a fab would see, coordinate by coordinate. It is strict: an undefined
//! aperture, a missing format or unit, an unterminated region or a missing
//! end-of-file marker is an error.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Aperture {
    Circle(f64),
    Rect(f64, f64),
    Obround(f64, f64),
    /// A macro by name with its parameters.
    Macro(String, Vec<f64>),
}

impl Aperture {
    /// Half extents [x, y] of what a flash covers.
    pub fn half(&self) -> [f64; 2] {
        match self {
            Aperture::Circle(d) => [d / 2.0, d / 2.0],
            Aperture::Rect(w, h) | Aperture::Obround(w, h) => [w / 2.0, h / 2.0],
            Aperture::Macro(name, p) if name == "RoundRect" && p.len() >= 9 => {
                let r = p[0];
                let xs = [p[1], p[3], p[5], p[7]];
                let ys = [p[2], p[4], p[6], p[8]];
                [xs.iter().fold(0.0f64, |m, v| m.max(v.abs())) + r, ys.iter().fold(0.0f64, |m, v| m.max(v.abs())) + r]
            }
            Aperture::Macro(..) => [0.0, 0.0],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GerberFile {
    pub units_mm: bool,
    /// Integer and decimal digits.
    pub format: (u32, u32),
    pub attributes: BTreeMap<String, String>,
    pub apertures: BTreeMap<u32, Aperture>,
    pub aperture_functions: BTreeMap<u32, String>,
    pub macros: Vec<String>,
    pub flashes: Vec<(u32, [f64; 2])>,
    pub draws: Vec<(u32, [f64; 2], [f64; 2])>,
    pub regions: Vec<Vec<[f64; 2]>>,
    /// Net names seen in object attributes.
    pub nets: Vec<String>,
}

impl GerberFile {
    /// Bounds of everything drawn: [min x, min y, max x, max y].
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut add = |p: [f64; 2], h: [f64; 2]| {
            b = [b[0].min(p[0] - h[0]), b[1].min(p[1] - h[1]), b[2].max(p[0] + h[0]), b[3].max(p[1] + h[1])];
        };
        for (d, p) in &self.flashes {
            add(*p, self.apertures[d].half());
        }
        for (d, a, z) in &self.draws {
            let h = self.apertures[d].half();
            add(*a, h);
            add(*z, h);
        }
        for r in &self.regions {
            for p in r {
                add(*p, [0.0, 0.0]);
            }
        }
        b
    }
}

fn num(s: &str) -> Result<f64, String> {
    s.trim().parse::<f64>().map_err(|_| format!("bad number “{s}”"))
}

pub fn gerber(text: &str) -> Result<GerberFile, String> {
    let mut g = GerberFile::default();
    let mut have_fs = false;
    let mut have_mo = false;
    let mut current: Option<u32> = None;
    let mut pos = [0.0f64, 0.0];
    let mut in_region = false;
    let mut contour: Vec<[f64; 2]> = Vec::new();
    let mut pending_function: Option<String> = None;
    let mut ended = false;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '%' {
            let end = text[i + 1..].find('%').ok_or("unterminated extended command")? + i + 1;
            let body = &text[i + 1..end];
            i = end + 1;
            let cmd = body.trim_end_matches(['*', '\n', ' ']);
            if let Some(rest) = cmd.strip_prefix("FS") {
                let x = rest.find('X').ok_or("FS without X")?;
                let digits = &rest[x + 1..x + 3];
                g.format = (digits[0..1].parse().map_err(|_| "FS digits")?, digits[1..2].parse().map_err(|_| "FS digits")?);
                if !rest.starts_with("LA") {
                    return Err("only leading-zero-omitted absolute coordinates are read".into());
                }
                have_fs = true;
            } else if let Some(u) = cmd.strip_prefix("MO") {
                g.units_mm = u == "MM";
                have_mo = true;
            } else if let Some(rest) = cmd.strip_prefix("TF.") {
                let (k, v) = rest.split_once(',').unwrap_or((rest, ""));
                g.attributes.insert(k.to_string(), v.to_string());
            } else if let Some(rest) = cmd.strip_prefix("TA.AperFunction,") {
                pending_function = Some(rest.to_string());
            } else if let Some(rest) = cmd.strip_prefix("TO.N,") {
                if !g.nets.contains(&rest.to_string()) {
                    g.nets.push(rest.to_string());
                }
            } else if cmd.starts_with("TD") {
                if cmd == "TD" || cmd.starts_with("TD.AperFunction") {
                    pending_function = None;
                }
            } else if let Some(rest) = cmd.strip_prefix("AM") {
                let name = rest.split('*').next().unwrap_or("").to_string();
                g.macros.push(name);
            } else if let Some(rest) = cmd.strip_prefix("ADD") {
                let k = rest.find(|c: char| !c.is_ascii_digit()).ok_or("bad AD")?;
                let d: u32 = rest[..k].parse().map_err(|_| "bad D-code")?;
                if d < 10 {
                    return Err(format!("aperture D{d}: D-codes start at 10"));
                }
                let def = &rest[k..];
                let (shape, params) = def.split_once(',').unwrap_or((def, ""));
                let p: Vec<f64> = if params.is_empty() { vec![] } else { params.split('X').map(num).collect::<Result<_, _>>()? };
                let ap = match shape {
                    "C" => Aperture::Circle(*p.first().ok_or("C needs a diameter")?),
                    "R" => Aperture::Rect(p[0], *p.get(1).ok_or("R needs w and h")?),
                    "O" => Aperture::Obround(p[0], *p.get(1).ok_or("O needs w and h")?),
                    other => {
                        if !g.macros.iter().any(|m| m == other) {
                            return Err(format!("aperture D{d} uses undefined macro {other}"));
                        }
                        Aperture::Macro(other.to_string(), p)
                    }
                };
                if g.apertures.insert(d, ap).is_some() {
                    return Err(format!("aperture D{d} defined twice"));
                }
                if let Some(f) = &pending_function {
                    g.aperture_functions.insert(d, f.clone());
                }
            } else if cmd.starts_with("LP") || cmd.starts_with("TO.") {
                // polarity / other object attributes: accepted
            } else {
                return Err(format!("unsupported extended command %{cmd}%"));
            }
            continue;
        }
        let end = text[i..].find('*').ok_or("word without *")? + i;
        let word = text[i..end].trim();
        i = end + 1;
        if word.starts_with("G04") {
            continue;
        }
        if ended {
            return Err("content after M02".into());
        }
        match word {
            "G01" | "G75" => continue,
            "G36" => {
                in_region = true;
                contour.clear();
                continue;
            }
            "G37" => {
                if !in_region {
                    return Err("G37 without G36".into());
                }
                if contour.len() >= 3 {
                    g.regions.push(std::mem::take(&mut contour));
                }
                in_region = false;
                continue;
            }
            "M02" => {
                ended = true;
                continue;
            }
            _ => {}
        }
        if let Some(d) = word.strip_prefix('D').and_then(|s| s.parse::<u32>().ok()) {
            if d >= 10 {
                if !g.apertures.contains_key(&d) {
                    return Err(format!("D{d} selected but not defined"));
                }
                current = Some(d);
                continue;
            }
        }
        if !(have_fs && have_mo) {
            return Err("coordinates before the format (FS) and unit (MO)".into());
        }
        // X…Y…Dnn
        let scale = 10f64.powi(g.format.1 as i32);
        let mut next = pos;
        let mut op = None;
        let mut rest = word;
        while !rest.is_empty() {
            let key = rest.chars().next().unwrap();
            let k = rest[1..].find(|c: char| c.is_ascii_alphabetic()).map(|k| k + 1).unwrap_or(rest.len());
            let val = &rest[1..k];
            match key {
                'X' => next[0] = val.parse::<i64>().map_err(|_| format!("bad X in {word}"))? as f64 / scale,
                'Y' => next[1] = val.parse::<i64>().map_err(|_| format!("bad Y in {word}"))? as f64 / scale,
                'D' => op = Some(val.parse::<u32>().map_err(|_| format!("bad D in {word}"))?),
                other => return Err(format!("unexpected {other} in {word}")),
            }
            rest = &rest[k..];
        }
        match op {
            Some(1) => {
                if in_region {
                    if contour.is_empty() {
                        contour.push(pos);
                    }
                    contour.push(next);
                } else {
                    let d = current.ok_or("draw with no aperture")?;
                    g.draws.push((d, pos, next));
                }
            }
            Some(2) => {
                if in_region && contour.len() >= 3 {
                    g.regions.push(std::mem::take(&mut contour));
                }
                contour.clear();
            }
            Some(3) => {
                if in_region {
                    return Err("flash inside a region".into());
                }
                let d = current.ok_or("flash with no aperture")?;
                g.flashes.push((d, next));
            }
            _ => return Err(format!("coordinate word without operation: {word}")),
        }
        pos = next;
    }
    if !ended {
        return Err("no M02 at the end".into());
    }
    if in_region {
        return Err("region not closed".into());
    }
    Ok(g)
}

#[derive(Clone, Debug, Default)]
pub struct DrillFile {
    pub metric: bool,
    pub plated: Option<bool>,
    pub tools: BTreeMap<u32, f64>,
    pub hits: Vec<(u32, [f64; 2])>,
    pub slots: Vec<(u32, [f64; 2], [f64; 2])>,
}

pub fn excellon(text: &str) -> Result<DrillFile, String> {
    let mut f = DrillFile::default();
    let mut header = false;
    let mut body = false;
    let mut tool = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(c) = line.strip_prefix(';') {
            if c.contains("TF.FileFunction,Plated") {
                f.plated = Some(true);
            } else if c.contains("TF.FileFunction,NonPlated") {
                f.plated = Some(false);
            }
            continue;
        }
        match line {
            "M48" => {
                header = true;
                continue;
            }
            "%" | "M95" => {
                body = true;
                continue;
            }
            "METRIC" => {
                f.metric = true;
                continue;
            }
            "G90" | "G05" | "FMAT,2" => continue,
            "M30" => return if header { Ok(f) } else { Err("no M48 header".into()) },
            _ => {}
        }
        if let Some(rest) = line.strip_prefix('T') {
            let k = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            let t: u32 = rest[..k].parse().map_err(|_| format!("bad tool {line}"))?;
            if let Some(d) = rest[k..].strip_prefix('C') {
                if body {
                    return Err("tool defined after the header".into());
                }
                f.tools.insert(t, num(d)?);
            } else {
                tool = if t == 0 { None } else { Some(t) };
                if t != 0 && !f.tools.contains_key(&t) {
                    return Err(format!("T{t} used but not defined"));
                }
            }
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            let t = tool.ok_or("hole before a tool")?;
            let parse = |s: &str| -> Result<[f64; 2], String> {
                let x = s.find('X').ok_or("no X")?;
                let y = s.find('Y').ok_or("no Y")?;
                Ok([num(&s[x + 1..y])?, num(&s[y + 1..])?])
            };
            if let Some((a, b)) = line.split_once("G85") {
                f.slots.push((t, parse(a)?, parse(b)?));
            } else {
                f.hits.push((t, parse(line)?));
            }
            continue;
        }
        return Err(format!("unsupported line {line}"));
    }
    Err("no M30 at the end".into())
}
