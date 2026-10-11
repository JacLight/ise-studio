//! S-expressions as KiCad writes them (boards, footprints, schematics):
//! lists, bare atoms and quoted strings. Shared by every KiCad reader (the
//! board importer today, a schematic importer later) and by the writers.

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Sexp {
    List(Vec<Sexp>),
    /// A bare word or number: `segment`, `0.25`, `yes`.
    Atom(String),
    /// A quoted string, unescaped.
    Str(String),
}

impl Sexp {
    /// The list's first atom (`segment` in `(segment (start …) …)`).
    pub fn head(&self) -> Option<&str> {
        match self {
            Sexp::List(v) => match v.first() {
                Some(Sexp::Atom(a)) => Some(a.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn is(&self, name: &str) -> bool {
        self.head() == Some(name)
    }

    /// Items after the head.
    pub fn args(&self) -> &[Sexp] {
        match self {
            Sexp::List(v) if !v.is_empty() => &v[1..],
            _ => &[],
        }
    }

    /// The first child list named `name`.
    pub fn find(&self, name: &str) -> Option<&Sexp> {
        self.args().iter().find(|c| c.is(name))
    }

    /// Every child list named `name`.
    pub fn find_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Sexp> + 'a {
        self.args().iter().filter(move |c| c.is(name))
    }

    /// Text of an atom or string.
    pub fn text(&self) -> Option<&str> {
        match self {
            Sexp::Atom(s) | Sexp::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Sexp::Atom(s) => s.parse().ok(),
            Sexp::Str(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// The i-th argument as text / number.
    pub fn arg_text(&self, i: usize) -> Option<&str> {
        self.args().get(i).and_then(|a| a.text())
    }
    pub fn arg_num(&self, i: usize) -> Option<f64> {
        self.args().get(i).and_then(|a| a.num())
    }

    /// `(name v)` child's first value as text / number.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.find(name).and_then(|c| c.arg_text(0))
    }
    pub fn value_num(&self, name: &str) -> Option<f64> {
        self.find(name).and_then(|c| c.arg_num(0))
    }

    /// Every numeric argument of the child `name` (`(at 1 2 90)` → [1, 2, 90]).
    pub fn nums(&self, name: &str) -> Vec<f64> {
        self.find(name).map(|c| c.args().iter().filter_map(|a| a.num()).collect()).unwrap_or_default()
    }

    /// Whether a bare atom `word` appears among the arguments (`hide`, `locked`).
    pub fn has_atom(&self, word: &str) -> bool {
        self.args().iter().any(|a| matches!(a, Sexp::Atom(s) if s == word))
    }
}

/// Every top-level expression in `text`.
pub fn parse(text: &str) -> Result<Vec<Sexp>, String> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut stack: Vec<Vec<Sexp>> = vec![vec![]];
    let mut line = 1;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'(' => {
                stack.push(vec![]);
                i += 1;
            }
            b')' => {
                if stack.len() < 2 {
                    return Err(format!("line {line}: unbalanced ')'"));
                }
                let l = stack.pop().unwrap();
                stack.last_mut().unwrap().push(Sexp::List(l));
                i += 1;
            }
            b'"' => {
                let mut s = Vec::new();
                i += 1;
                loop {
                    if i >= b.len() {
                        return Err(format!("line {line}: unterminated string"));
                    }
                    match b[i] {
                        b'"' => {
                            i += 1;
                            break;
                        }
                        b'\\' if i + 1 < b.len() => {
                            let e = b[i + 1];
                            s.push(match e {
                                b'n' => b'\n',
                                b't' => b'\t',
                                b'r' => b'\r',
                                other => other,
                            });
                            i += 2;
                        }
                        ch => {
                            if ch == b'\n' {
                                line += 1;
                            }
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                stack.last_mut().unwrap().push(Sexp::Str(String::from_utf8_lossy(&s).into_owned()));
            }
            _ => {
                let start = i;
                while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b'"') {
                    i += 1;
                }
                stack.last_mut().unwrap().push(Sexp::Atom(String::from_utf8_lossy(&b[start..i]).into_owned()));
            }
        }
    }
    if stack.len() != 1 {
        return Err("unbalanced '(': the file ends inside a list".into());
    }
    Ok(stack.pop().unwrap())
}

/// A string quoted the way KiCad writes it.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A number as KiCad writes it: up to 6 decimals, no trailing zeros.
pub fn num(v: f64) -> String {
    let s = format!("{:.6}", if v.abs() < 5e-7 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

/// Writes an expression with KiCad-like indentation (short lists on one line).
pub fn write(e: &Sexp) -> String {
    let mut out = String::new();
    write_into(e, 0, &mut out);
    out.push('\n');
    out
}

fn flat(e: &Sexp) -> String {
    match e {
        Sexp::Atom(a) => a.clone(),
        Sexp::Str(s) => quote(s),
        Sexp::List(v) => format!("({})", v.iter().map(flat).collect::<Vec<_>>().join(" ")),
    }
}

fn write_into(e: &Sexp, depth: usize, out: &mut String) {
    let f = flat(e);
    if f.len() <= 100 || !matches!(e, Sexp::List(_)) {
        out.push_str(&f);
        return;
    }
    let Sexp::List(v) = e else { unreachable!() };
    out.push('(');
    let mut first = true;
    for item in v {
        if first {
            out.push_str(&flat(item));
            first = false;
            continue;
        }
        if matches!(item, Sexp::List(_)) {
            out.push('\n');
            let _ = write!(out, "{}", "\t".repeat(depth + 1));
            write_into(item, depth + 1, out);
        } else {
            out.push(' ');
            out.push_str(&flat(item));
        }
    }
    out.push('\n');
    let _ = write!(out, "{}", "\t".repeat(depth));
    out.push(')');
}

/// Builders for writers.
pub fn atom(s: impl Into<String>) -> Sexp {
    Sexp::Atom(s.into())
}
pub fn s(s: impl Into<String>) -> Sexp {
    Sexp::Str(s.into())
}
pub fn n(v: f64) -> Sexp {
    Sexp::Atom(num(v))
}
pub fn list(head: &str, items: Vec<Sexp>) -> Sexp {
    let mut v = Vec::with_capacity(items.len() + 1);
    v.push(atom(head));
    v.extend(items);
    Sexp::List(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes_back() {
        let t = r#"(kicad_pcb (version 20240108) (net 1 "GND \"x\"") (at 1.5 -2 90) hide)"#;
        let e = &parse(t).unwrap()[0];
        assert!(e.is("kicad_pcb"));
        assert_eq!(e.value_num("version"), Some(20240108.0));
        assert_eq!(e.find("net").unwrap().arg_text(1), Some("GND \"x\""));
        assert_eq!(e.nums("at"), vec![1.5, -2.0, 90.0]);
        assert!(e.has_atom("hide"));
        let back = parse(&write(e)).unwrap();
        assert_eq!(&back[0], e);
        assert!(parse("(a (b)").is_err());
        assert!(parse("(a))").is_err());
        assert_eq!(num(0.10000001), "0.1");
        assert_eq!(num(-0.0000001), "0");
    }
}
