//! STEP physical files (ISO 10303-21), the text form IFC is written in:
//! a reader for the DATA section and helpers for writing one.

use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// `$`: not given.
    Null,
    /// `*`: derived.
    Derived,
    Ref(u64),
    Num(f64),
    Str(String),
    /// `.ELEMENT.`, `.T.`
    Enum(String),
    List(Vec<Arg>),
    /// `IFCLABEL('x')`: a typed value.
    Typed(String, Box<Arg>),
}

impl Arg {
    pub fn as_ref(&self) -> Option<u64> {
        match self {
            Arg::Ref(r) => Some(*r),
            _ => None,
        }
    }
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Arg::Num(n) => Some(*n),
            Arg::Typed(_, a) => a.as_num(),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            Arg::Typed(_, a) => a.as_str(),
            _ => None,
        }
    }
    pub fn as_enum(&self) -> Option<&str> {
        match self {
            Arg::Enum(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_list(&self) -> &[Arg] {
        match self {
            Arg::List(l) => l,
            _ => &[],
        }
    }
    /// The ids in a list of references.
    pub fn refs(&self) -> Vec<u64> {
        self.as_list().iter().filter_map(Arg::as_ref).collect()
    }
    pub fn nums(&self) -> Vec<f64> {
        self.as_list().iter().filter_map(Arg::as_num).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Entity {
    /// Upper case: "IFCWALL".
    pub ty: String,
    pub args: Vec<Arg>,
}

impl Entity {
    pub fn arg(&self, i: usize) -> &Arg {
        self.args.get(i).unwrap_or(&Arg::Null)
    }
}

#[derive(Debug, Default)]
pub struct File {
    /// "IFC4", "IFC2X3", "IFC4X3"…
    pub schema: String,
    pub entities: HashMap<u64, Entity>,
}

impl File {
    pub fn get(&self, id: u64) -> Option<&Entity> {
        self.entities.get(&id)
    }
    /// Every entity of a type (upper case), in id order.
    pub fn all(&self, ty: &str) -> Vec<(u64, &Entity)> {
        let mut v: Vec<(u64, &Entity)> = self.entities.iter().filter(|(_, e)| e.ty == ty).map(|(i, e)| (*i, e)).collect();
        v.sort_by_key(|(i, _)| *i);
        v
    }
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn skip(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"/*") {
                match find(&self.s[self.i + 2..], b"*/") {
                    Some(k) => self.i += k + 4,
                    None => self.i = self.s.len(),
                }
                continue;
            }
            break;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }
    fn word(&mut self) -> String {
        self.skip();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).to_ascii_uppercase()
    }
    fn args(&mut self) -> Result<Vec<Arg>, String> {
        self.expect(b'(')?;
        let mut out = Vec::new();
        if self.peek() == Some(b')') {
            self.i += 1;
            return Ok(out);
        }
        loop {
            out.push(self.arg()?);
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b')') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err(format!("expected ',' or ')' at byte {}", self.i)),
            }
        }
    }
    fn arg(&mut self) -> Result<Arg, String> {
        match self.peek() {
            Some(b'$') => {
                self.i += 1;
                Ok(Arg::Null)
            }
            Some(b'*') => {
                self.i += 1;
                Ok(Arg::Derived)
            }
            Some(b'#') => {
                self.i += 1;
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).ok().and_then(|t| t.parse().ok()).map(Arg::Ref).ok_or_else(|| format!("bad reference at byte {start}"))
            }
            Some(b'\'') => {
                self.i += 1;
                let mut raw = Vec::new();
                loop {
                    let c = *self.s.get(self.i).ok_or("unterminated string")?;
                    self.i += 1;
                    if c == b'\'' {
                        if self.s.get(self.i) == Some(&b'\'') {
                            raw.push(b'\'');
                            self.i += 1;
                            continue;
                        }
                        break;
                    }
                    raw.push(c);
                }
                Ok(Arg::Str(decode_string(&raw)))
            }
            Some(b'.') => {
                self.i += 1;
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'.' {
                    self.i += 1;
                }
                let e = String::from_utf8_lossy(&self.s[start..self.i]).to_ascii_uppercase();
                self.i += 1;
                Ok(Arg::Enum(e))
            }
            Some(b'(') => Ok(Arg::List(self.args()?)),
            Some(b'"') => {
                // Binary: kept as text.
                self.i += 1;
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i] != b'"' {
                    self.i += 1;
                }
                let t = String::from_utf8_lossy(&self.s[start..self.i]).to_string();
                self.i += 1;
                Ok(Arg::Str(t))
            }
            Some(c) if c == b'-' || c == b'+' || c.is_ascii_digit() => {
                let start = self.i;
                self.i += 1;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], b'.' | b'E' | b'e' | b'-' | b'+')) {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                t.parse::<f64>().map(Arg::Num).map_err(|_| format!("bad number {t}"))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let ty = self.word();
                let mut a = self.args()?;
                Ok(Arg::Typed(ty, Box::new(if a.len() == 1 { a.remove(0) } else { Arg::List(a) })))
            }
            _ => Err(format!("unexpected input at byte {}", self.i)),
        }
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// A STEP string's text: `\X2\0041\X0\`, `\X\E9`, `\S\i`, `\\`.
pub fn decode_string(raw: &[u8]) -> String {
    let mut out = String::new();
    let mut bytes: Vec<u8> = Vec::new();
    let flush = |bytes: &mut Vec<u8>, out: &mut String| {
        out.push_str(&String::from_utf8_lossy(bytes));
        bytes.clear();
    };
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' {
            if raw[i..].starts_with(b"\\\\") {
                bytes.push(b'\\');
                i += 2;
                continue;
            }
            if raw[i..].starts_with(b"\\X2\\") || raw[i..].starts_with(b"\\X4\\") {
                flush(&mut bytes, &mut out);
                let width = if raw[i + 2] == b'2' { 4 } else { 8 };
                i += 4;
                let mut units: Vec<u16> = Vec::new();
                while i + width <= raw.len() && !raw[i..].starts_with(b"\\X0\\") {
                    let h = std::str::from_utf8(&raw[i..i + width]).ok().and_then(|t| u32::from_str_radix(t, 16).ok()).unwrap_or(0xfffd);
                    if width == 4 {
                        units.push(h as u16);
                    } else if let Some(c) = char::from_u32(h) {
                        let mut b = [0u16; 2];
                        units.extend_from_slice(c.encode_utf16(&mut b));
                    }
                    i += width;
                }
                out.push_str(&String::from_utf16_lossy(&units));
                i += 4;
                continue;
            }
            if raw[i..].starts_with(b"\\X\\") && i + 5 <= raw.len() {
                flush(&mut bytes, &mut out);
                if let Some(c) = std::str::from_utf8(&raw[i + 3..i + 5]).ok().and_then(|t| u32::from_str_radix(t, 16).ok()).and_then(char::from_u32) {
                    out.push(c);
                }
                i += 5;
                continue;
            }
            if raw[i..].starts_with(b"\\S\\") && i + 3 < raw.len() {
                flush(&mut bytes, &mut out);
                if let Some(c) = char::from_u32(raw[i + 3] as u32 + 128) {
                    out.push(c);
                }
                i += 4;
                continue;
            }
            if raw[i..].starts_with(b"\\P") && i + 3 < raw.len() && raw[i + 3] == b'\\' {
                i += 4;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    flush(&mut bytes, &mut out);
    out
}

/// Reads a STEP physical file's schema and DATA section.
pub fn parse(text: &str) -> Result<File, String> {
    let s = text.as_bytes();
    let mut file = File::default();
    if let Some(k) = find(s, b"FILE_SCHEMA") {
        let rest = &text[k..];
        if let (Some(a), Some(b)) = (rest.find('\''), rest[rest.find('\'').unwrap_or(0) + 1..].find('\'')) {
            file.schema = rest[a + 1..a + 1 + b].to_ascii_uppercase();
        }
    }
    let start = find(s, b"DATA;").ok_or("not a STEP file: no DATA section")? + 5;
    let mut lx = Lexer { s, i: start };
    loop {
        match lx.peek() {
            None => break,
            Some(b'#') => {
                lx.i += 1;
                let st = lx.i;
                while lx.i < s.len() && s[lx.i].is_ascii_digit() {
                    lx.i += 1;
                }
                let id: u64 = text[st..lx.i].parse().map_err(|_| format!("bad entity id at byte {st}"))?;
                lx.expect(b'=')?;
                if lx.peek() == Some(b'(') {
                    // A complex instance: skipped.
                    let _ = lx.args()?;
                } else {
                    let ty = lx.word();
                    let args = lx.args().map_err(|e| format!("#{id} {ty}: {e}"))?;
                    file.entities.insert(id, Entity { ty, args });
                }
                lx.expect(b';')?;
            }
            Some(_) => {
                let w = lx.word();
                if w == "ENDSEC" || w.is_empty() {
                    break;
                }
            }
        }
    }
    Ok(file)
}

/// A string as STEP writes it: quoted, `'` doubled, `\` doubled, anything
/// outside printable ASCII as `\X2\…\X0\`.
pub fn string(t: &str) -> String {
    let mut out = String::from("'");
    let mut wide: Vec<u16> = Vec::new();
    let flush = |wide: &mut Vec<u16>, out: &mut String| {
        if !wide.is_empty() {
            out.push_str("\\X2\\");
            for u in wide.iter() {
                let _ = write!(out, "{u:04X}");
            }
            out.push_str("\\X0\\");
            wide.clear();
        }
    };
    for c in t.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut wide, &mut out);
            match c {
                '\'' => out.push_str("''"),
                '\\' => out.push_str("\\\\"),
                _ => out.push(c),
            }
        } else {
            let mut b = [0u16; 2];
            wide.extend_from_slice(c.encode_utf16(&mut b));
        }
    }
    flush(&mut wide, &mut out);
    out.push('\'');
    out
}

/// A real number as STEP writes it (always with a point).
pub fn real(v: f64) -> String {
    if !v.is_finite() {
        return "0.".into();
    }
    if v != 0.0 && (v.abs() < 1e-4 || v.abs() >= 1e15) {
        return format!("{v:.6E}");
    }
    let s = format!("{v:.9}");
    let s = s.trim_end_matches('0');
    let s = if s == "-0." { "0." } else { s };
    s.to_string()
}

/// Writes entities in order, numbering them.
#[derive(Default)]
pub struct Writer {
    pub data: String,
    next: u64,
}

impl Writer {
    pub fn add(&mut self, ty: &str, args: &str) -> u64 {
        self.next += 1;
        let _ = writeln!(self.data, "#{}={}({});", self.next, ty, args);
        self.next
    }
}

pub fn r(id: u64) -> String {
    format!("#{id}")
}

pub fn refs(ids: &[u64]) -> String {
    format!("({})", ids.iter().map(|i| r(*i)).collect::<Vec<_>>().join(","))
}

pub fn opt_string(t: Option<&str>) -> String {
    t.map(string).unwrap_or_else(|| "$".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_round_trip() {
        for t in ["Wall › Ground", "it's", "a\\b", "12 m²", "Ω"] {
            let w = string(t);
            let f = parse(&format!("DATA;\n#1=IFCLABEL({w});\nENDSEC;")).unwrap();
            assert_eq!(f.get(1).unwrap().arg(0).as_str(), Some(t), "{w}");
        }
    }

    #[test]
    fn parses_typed_values_lists_and_comments() {
        let f = parse("ISO-10303-21;HEADER;FILE_SCHEMA(('IFC4'));ENDSEC;DATA;\n/* c */#7=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);\n#8=IFCCARTESIANPOINT((1.,-2.5E1,3));\nENDSEC;END-ISO-10303-21;").unwrap();
        assert_eq!(f.schema, "IFC4");
        let p = f.get(7).unwrap();
        assert_eq!(p.ty, "IFCPROPERTYSINGLEVALUE");
        assert_eq!(p.arg(2), &Arg::Typed("IFCBOOLEAN".into(), Box::new(Arg::Enum("T".into()))));
        assert_eq!(f.get(8).unwrap().arg(0).nums(), vec![1.0, -25.0, 3.0]);
        assert_eq!(real(1.0), "1.");
        assert_eq!(real(0.25), "0.25");
        assert_eq!(real(-0.0), "0.");
        assert!(real(1e-6).contains('E'));
    }
}
