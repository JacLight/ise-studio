//! Custom design rules: the KiCad `.kicad_dru` language, read and evaluated
//! by Forma's own parser.
//!
//! ```text
//! (version 1)
//! (rule "HV clearance"
//!    (layer outer)
//!    (condition "A.NetClass == 'HV' && B.NetClass != 'HV'")
//!    (constraint clearance (min 1.5mm)))
//! (rule "No vias under the antenna"
//!    (condition "A.insideArea('Antenna')")
//!    (constraint disallow via track))
//! ```
//!
//! A rule has a name, an optional layer (`outer`, `inner`, or a layer name),
//! an optional condition and one or more constraints, and an optional
//! `(severity error|warning|ignore)`. Later rules win over earlier ones (as in
//! KiCad); board minima always hold. Conditions read item properties through
//! `A.` (and `B.`, the other item of a pair): `Type` (Track, Via, Pad, Zone,
//! Footprint, Graphic, Text, Hole), `NetName`, `NetClass`, `Layer`,
//! `Reference`, `Footprint`, `Via_Type` (Through, Blind/buried, Micro),
//! `Pad_Type` (SMD, Through-hole, NPTH), `Width`, `Hole`, `Name`; and
//! functions `insideArea('n')`, `enclosedByArea('n')`, `intersectsArea('n')`,
//! `existsOnLayer('F.Cu')`, `isPlated()`, `hasNetclass('n')`. Strings match
//! with `*` and `?` wildcards, ignoring case; numbers take mm, mil, in or um.
//!
//! Constraints checked: clearance, hole_clearance, hole_to_hole,
//! edge_clearance, courtyard_clearance, track_width, via_diameter, hole_size,
//! annular_width, and disallow (track, via, micro_via, buried_via, pad, zone,
//! text, graphic, hole, footprint). Length, skew and diff-pair constraints
//! are read but not checked (routing constraints are parked).

use crate::sexpr::{self, Sexp};
use crate::{bad, board_of};
use de_core::{Command, CommandError, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---- Expressions ------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Str(String),
    Num(f64),
    Bool(bool),
    /// `A.Prop` (side 'A' or 'B').
    Prop(char, String),
    /// `A.fn(args)`.
    Call(char, String, Vec<Expr>),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Cmp(Op, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Str(String),
    Num(f64),
    Ident(String),
    Sym(&'static str),
}

/// A length with an optional unit, in mm.
pub fn length(s: &str) -> Option<f64> {
    let t = s.trim();
    let i = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(t.len());
    let (n, u) = t.split_at(i);
    let v: f64 = n.parse().ok()?;
    let k = match u.trim().to_ascii_lowercase().as_str() {
        "" | "mm" => 1.0,
        "mil" | "mils" | "th" => 0.0254,
        "in" | "inch" | "\"" => 25.4,
        "um" | "µm" => 0.001,
        "cm" => 10.0,
        "deg" | "°" => 1.0,
        _ => return None,
    };
    Some(v * k)
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '\'' || ch == '"' {
            let q = ch;
            let mut j = i + 1;
            let mut v = String::new();
            while j < c.len() && c[j] != q {
                v.push(c[j]);
                j += 1;
            }
            if j >= c.len() {
                return Err(format!("unclosed string in “{s}”"));
            }
            out.push(Tok::Str(v));
            i = j + 1;
            continue;
        }
        if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut j = i;
            while j < c.len() && (c[j].is_ascii_alphanumeric() || c[j] == '.') {
                j += 1;
            }
            let word: String = c[i..j].iter().collect();
            out.push(Tok::Num(length(&word).ok_or_else(|| format!("“{word}” is not a number with a unit (mm, mil, in, um)"))?));
            i = j;
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let mut j = i;
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_' || c[j] == '.') {
                j += 1;
            }
            out.push(Tok::Ident(c[i..j].iter().collect()));
            i = j;
            continue;
        }
        let two: String = c[i..(i + 2).min(c.len())].iter().collect();
        let sym = match two.as_str() {
            "==" => Some("=="),
            "!=" => Some("!="),
            "<=" => Some("<="),
            ">=" => Some(">="),
            "&&" => Some("&&"),
            "||" => Some("||"),
            _ => None,
        };
        if let Some(s2) = sym {
            out.push(Tok::Sym(s2));
            i += 2;
            continue;
        }
        let one = match ch {
            '<' => "<",
            '>' => ">",
            '!' => "!",
            '(' => "(",
            ')' => ")",
            ',' => ",",
            '=' => "==",
            _ => return Err(format!("unexpected “{ch}” in “{s}”")),
        };
        out.push(Tok::Sym(one));
        i += 1;
    }
    Ok(out)
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn eat(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        while self.eat("||") {
            l = Expr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.cmp()?;
        while self.eat("&&") {
            l = Expr::And(Box::new(l), Box::new(self.cmp()?));
        }
        Ok(l)
    }
    fn cmp(&mut self) -> Result<Expr, String> {
        let l = self.unary()?;
        for (s, op) in [("==", Op::Eq), ("!=", Op::Ne), ("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)] {
            if self.eat(s) {
                return Ok(Expr::Cmp(op, Box::new(l), Box::new(self.unary()?)));
            }
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("(") {
            let e = self.or()?;
            if !self.eat(")") {
                return Err("a ( without its )".into());
            }
            return Ok(e);
        }
        match self.t.get(self.i).cloned() {
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(Expr::Str(s))
            }
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Ident(id)) => {
                self.i += 1;
                if id.eq_ignore_ascii_case("true") {
                    return Ok(Expr::Bool(true));
                }
                if id.eq_ignore_ascii_case("false") {
                    return Ok(Expr::Bool(false));
                }
                let (side, name) = match id.split_once('.') {
                    Some((s, n)) if s.eq_ignore_ascii_case("A") || s.eq_ignore_ascii_case("B") => (s.to_ascii_uppercase().chars().next().unwrap(), n.to_string()),
                    _ => return Err(format!("“{id}”: properties are read as A.Name or B.Name")),
                };
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.or()?);
                            if self.eat(")") {
                                break;
                            }
                            if !self.eat(",") {
                                return Err(format!("{id}(…): arguments are separated by commas"));
                            }
                        }
                    }
                    return Ok(Expr::Call(side, name, args));
                }
                Ok(Expr::Prop(side, name))
            }
            other => Err(format!("expected a value, found {other:?}")),
        }
    }
}

pub fn parse_expr(s: &str) -> Result<Expr, String> {
    let mut p = Parser { t: lex(s)?, i: 0 };
    if p.t.is_empty() {
        return Ok(Expr::Bool(true));
    }
    let e = p.or()?;
    if p.i != p.t.len() {
        return Err(format!("unexpected {:?} in “{s}”", p.t[p.i]));
    }
    Ok(e)
}

// ---- Items ------------------------------------------------------------------

/// What a condition can read about one item.
#[derive(Clone, Debug, Default)]
pub struct Item {
    /// "track", "via", "pad", "zone", "footprint", "graphic", "text", "hole".
    pub kind: &'static str,
    pub net: Option<String>,
    pub netclass: Option<String>,
    /// Forma layer names ("top", "in1", "bottom", "silk_top" …).
    pub layers: Vec<String>,
    pub reference: Option<String>,
    pub footprint: Option<String>,
    /// "through", "blind", "buried", "micro".
    pub via_type: Option<&'static str>,
    /// "smd", "tht", "npth".
    pub pad_type: Option<&'static str>,
    pub width: Option<f64>,
    pub hole: Option<f64>,
    pub plated: bool,
    pub name: Option<String>,
    /// Areas (rule areas and named zones) it lies wholly inside.
    pub inside: Vec<String>,
    /// Areas it touches.
    pub touches: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Str(String),
    Num(f64),
    Bool(bool),
    Undef,
}

/// A layer name in Forma's terms ("F.Cu" → "top").
pub fn forma_layer(l: &str) -> String {
    crate::graphics::normalize_layer(l)
}

/// KiCad's name for a Forma layer, for comparisons written either way.
pub fn kicad_layer(l: &str) -> String {
    match l {
        "top" => "F.Cu".into(),
        "bottom" => "B.Cu".into(),
        "silk_top" => "F.SilkS".into(),
        "silk_bottom" => "B.SilkS".into(),
        "fab_top" => "F.Fab".into(),
        "fab_bottom" => "B.Fab".into(),
        "mask_top" => "F.Mask".into(),
        "mask_bottom" => "B.Mask".into(),
        "paste_top" => "F.Paste".into(),
        "paste_bottom" => "B.Paste".into(),
        "courtyard_top" => "F.CrtYd".into(),
        "courtyard_bottom" => "B.CrtYd".into(),
        "user" => "Dwgs.User".into(),
        "comments" => "Cmts.User".into(),
        _ => match l.strip_prefix("in") {
            Some(n) => format!("In{n}.Cu"),
            None => l.to_string(),
        },
    }
}

/// `*` and `?` wildcards, case ignored.
pub fn wild(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = s.to_lowercase().chars().collect();
    let (mut i, mut j, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

fn type_name(kind: &str) -> &'static str {
    match kind {
        "track" => "Track",
        "via" => "Via",
        "pad" => "Pad",
        "zone" => "Zone",
        "footprint" => "Footprint",
        "graphic" => "Graphic",
        "text" => "Text",
        "hole" => "Hole",
        _ => "Unknown",
    }
}

fn prop(it: Option<&Item>, name: &str) -> Val {
    let Some(it) = it else { return Val::Undef };
    let s = |v: &Option<String>| v.clone().map(Val::Str).unwrap_or(Val::Undef);
    match name.to_ascii_lowercase().replace('_', "").as_str() {
        "type" => Val::Str(type_name(it.kind).into()),
        "netname" | "net" => s(&it.net),
        "netclass" => s(&it.netclass),
        "layer" => it.layers.first().map(|l| Val::Str(l.clone())).unwrap_or(Val::Undef),
        "reference" | "parent" => s(&it.reference),
        "footprint" | "libraryid" => s(&it.footprint),
        "viatype" => it.via_type.map(|v| Val::Str(match v { "through" => "Through", "micro" => "Micro", _ => "Blind/buried" }.into())).unwrap_or(Val::Undef),
        "padtype" => it.pad_type.map(|v| Val::Str(match v { "smd" => "SMD", "tht" => "Through-hole", _ => "NPTH, mechanical" }.into())).unwrap_or(Val::Undef),
        "width" | "trackwidth" => it.width.map(Val::Num).unwrap_or(Val::Undef),
        "hole" | "holesize" | "drill" => it.hole.map(Val::Num).unwrap_or(Val::Undef),
        "name" | "zonename" => s(&it.name),
        _ => Val::Undef,
    }
}

fn truthy(v: &Val) -> bool {
    match v {
        Val::Bool(b) => *b,
        Val::Num(n) => *n != 0.0,
        Val::Str(s) => !s.is_empty(),
        Val::Undef => false,
    }
}

fn equal(a: &Val, b: &Val, layer_prop: bool, multi: &[String]) -> bool {
    match (a, b) {
        (Val::Num(x), Val::Num(y)) => (x - y).abs() < 1e-9,
        (Val::Num(x), Val::Str(y)) | (Val::Str(y), Val::Num(x)) => length(y).is_some_and(|v| (v - x).abs() < 1e-9),
        (Val::Str(x), Val::Str(y)) => {
            if layer_prop {
                // A layer compares against every layer the item is on, by either name.
                let pat = y;
                return multi.iter().any(|l| wild(pat, l) || wild(pat, &kicad_layer(l))) || wild(pat, x) || wild(x, pat);
            }
            wild(y, x) || wild(x, y)
        }
        (Val::Bool(x), Val::Bool(y)) => x == y,
        _ => false,
    }
}

/// Evaluate a condition with items A and B.
pub fn eval(e: &Expr, a: Option<&Item>, b: Option<&Item>) -> bool {
    truthy(&value(e, a, b))
}

fn value(e: &Expr, a: Option<&Item>, b: Option<&Item>) -> Val {
    let side = |c: char| if c == 'A' { a } else { b };
    match e {
        Expr::Str(s) => Val::Str(s.clone()),
        Expr::Num(n) => Val::Num(*n),
        Expr::Bool(v) => Val::Bool(*v),
        Expr::Prop(c, n) => prop(side(*c), n),
        Expr::Call(c, f, args) => {
            let Some(it) = side(*c) else { return Val::Bool(false) };
            let arg = args.first().map(|x| value(x, a, b));
            let arg_s = match &arg {
                Some(Val::Str(s)) => s.clone(),
                _ => String::new(),
            };
            Val::Bool(match f.to_ascii_lowercase().as_str() {
                "insidearea" | "enclosedbyarea" => it.inside.iter().any(|n| wild(&arg_s, n)),
                "intersectsarea" | "intersectsfrontcourtyard" | "intersectsbackcourtyard" => it.touches.iter().any(|n| wild(&arg_s, n)) || it.inside.iter().any(|n| wild(&arg_s, n)),
                "existsonlayer" => {
                    let want = forma_layer(&arg_s);
                    it.layers.iter().any(|l| wild(&want, l) || wild(&arg_s, &kicad_layer(l)))
                }
                "isplated" => it.plated,
                "hasnetclass" => it.netclass.as_deref().is_some_and(|n| wild(&arg_s, n)),
                "memberof" | "memberofgroup" | "memberoffootprint" => it.reference.as_deref().is_some_and(|r| wild(&arg_s, r)),
                "fromto" | "indiffpair" | "iscoupleddiffpair" | "memberofsheet" => false,
                _ => false,
            })
        }
        Expr::Not(x) => Val::Bool(!truthy(&value(x, a, b))),
        Expr::And(x, y) => Val::Bool(truthy(&value(x, a, b)) && truthy(&value(y, a, b))),
        Expr::Or(x, y) => Val::Bool(truthy(&value(x, a, b)) || truthy(&value(y, a, b))),
        Expr::Cmp(op, x, y) => {
            let layer_side = |e: &Expr| match e {
                Expr::Prop(c, n) if n.eq_ignore_ascii_case("layer") => side(*c).map(|i| i.layers.clone()),
                _ => None,
            };
            let lv = value(x, a, b);
            let rv = value(y, a, b);
            if lv == Val::Undef || rv == Val::Undef {
                return Val::Bool(*op == Op::Ne && !(lv == Val::Undef && rv == Val::Undef));
            }
            let layers = layer_side(x).or_else(|| layer_side(y));
            match op {
                Op::Eq => Val::Bool(equal(&lv, &rv, layers.is_some(), layers.as_deref().unwrap_or(&[]))),
                Op::Ne => Val::Bool(!equal(&lv, &rv, layers.is_some(), layers.as_deref().unwrap_or(&[]))),
                _ => {
                    let n = |v: &Val| match v {
                        Val::Num(n) => Some(*n),
                        Val::Str(s) => length(s),
                        _ => None,
                    };
                    match (n(&lv), n(&rv)) {
                        (Some(l), Some(r)) => Val::Bool(match op {
                            Op::Lt => l < r,
                            Op::Le => l <= r,
                            Op::Gt => l > r,
                            Op::Ge => l >= r,
                            _ => unreachable!(),
                        }),
                        _ => Val::Bool(false),
                    }
                }
            }
        }
    }
}

// ---- Rules --------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Constraint {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Item types a disallow constraint keeps out.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disallow: Vec<String>,
    /// Read but not checked (routing constraints are parked).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unchecked: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Rule {
    pub name: String,
    /// "outer", "inner" or a layer name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub condition: String,
    #[serde(skip)]
    pub expr: Expr,
    pub constraints: Vec<Constraint>,
    /// "error" (default), "warning" or "ignore".
    pub severity: String,
}

pub const CHECKED: [&str; 10] = ["clearance", "hole_clearance", "hole_to_hole", "edge_clearance", "courtyard_clearance", "track_width", "via_diameter", "hole_size", "annular_width", "disallow"];
const KNOWN: [&str; 21] = [
    "clearance", "hole_clearance", "hole_to_hole", "edge_clearance", "courtyard_clearance", "track_width", "via_diameter", "hole_size", "annular_width", "disallow", "silk_clearance", "text_height", "text_thickness", "length", "skew", "diff_pair_gap", "diff_pair_uncoupled", "via_count", "zone_connection", "thermal_relief_gap", "physical_clearance",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

fn atom_text(e: &Sexp) -> Option<String> {
    e.text().map(|s| s.to_string())
}

/// Read a rules file.
pub fn parse(text: &str) -> Result<RuleSet, String> {
    // `#` starts a comment line, as in KiCad's rules files.
    let text: String = text.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n");
    let top = sexpr::parse(&text)?;
    let mut rules = Vec::new();
    for e in &top {
        if e.is("version") {
            continue;
        }
        if !e.is("rule") {
            return Err(format!("expected (rule …) or (version …), found ({})", e.head().unwrap_or("?")));
        }
        let name = e.arg_text(0).ok_or("a rule needs a name: (rule \"name\" …)")?.to_string();
        let mut r = Rule { name: name.clone(), layer: None, condition: String::new(), expr: Expr::Bool(true), constraints: vec![], severity: "error".into() };
        for item in e.args().iter().skip(1) {
            match item.head() {
                Some("layer") => r.layer = Some(item.arg_text(0).ok_or_else(|| format!("rule {name}: (layer outer|inner|name)"))?.to_string()),
                Some("condition") => {
                    let c = item.arg_text(0).ok_or_else(|| format!("rule {name}: (condition \"…\")"))?.to_string();
                    r.expr = parse_expr(&c).map_err(|m| format!("rule {name}: {m}"))?;
                    r.condition = c;
                }
                Some("severity") => {
                    let s = item.arg_text(0).unwrap_or("error").to_ascii_lowercase();
                    if !["error", "warning", "ignore", "exclusion", "info"].contains(&s.as_str()) {
                        return Err(format!("rule {name}: severity is error, warning or ignore"));
                    }
                    r.severity = if s == "exclusion" { "ignore".into() } else if s == "info" { "warning".into() } else { s };
                }
                Some("constraint") => {
                    let kind = item.arg_text(0).ok_or_else(|| format!("rule {name}: (constraint kind …)"))?.to_string();
                    if !KNOWN.contains(&kind.as_str()) {
                        return Err(format!("rule {name}: unknown constraint “{kind}” (use {})", CHECKED.join(", ")));
                    }
                    let mut c = Constraint { kind: kind.clone(), min: None, opt: None, max: None, disallow: vec![], unchecked: !CHECKED.contains(&kind.as_str()) };
                    for a in item.args().iter().skip(1) {
                        let num = |h: &str| -> Result<Option<f64>, String> {
                            match a.args().first().and_then(atom_text) {
                                Some(t) => length(&t).map(Some).ok_or_else(|| format!("rule {name}: ({h} {t}) is not a length")),
                                None => Ok(None),
                            }
                        };
                        match a.head() {
                            Some("min") => c.min = num("min")?,
                            Some("opt") => c.opt = num("opt")?,
                            Some("max") => c.max = num("max")?,
                            _ => {
                                if let Some(t) = atom_text(a) {
                                    c.disallow.push(t.to_ascii_lowercase());
                                }
                            }
                        }
                    }
                    if kind == "disallow" && c.disallow.is_empty() {
                        return Err(format!("rule {name}: (constraint disallow track via pad …) names what to keep out"));
                    }
                    r.constraints.push(c);
                }
                Some(h) => return Err(format!("rule {name}: unknown clause ({h} …)")),
                None => return Err(format!("rule {name}: unexpected atom")),
            }
        }
        if r.constraints.is_empty() {
            return Err(format!("rule {name} has no constraint"));
        }
        rules.push(r);
    }
    Ok(RuleSet { rules })
}

fn layer_ok(sel: &Option<String>, layer: Option<&str>, layers: &[String]) -> bool {
    let (Some(sel), Some(l)) = (sel, layer) else { return true };
    match sel.to_ascii_lowercase().as_str() {
        "outer" => l == "top" || l == "bottom",
        "inner" => l.starts_with("in"),
        _ => {
            let want = forma_layer(sel);
            let _ = layers;
            wild(&want, l) || wild(sel, &kicad_layer(l))
        }
    }
}

/// A rule's verdict: the constraint and which rule gave it.
pub struct Hit<'a> {
    pub rule: &'a Rule,
    pub constraint: &'a Constraint,
}

impl RuleSet {
    /// The constraint of `kind` between two items on `layer` (the last rule that matches either way round).
    pub fn pair(&self, kind: &str, a: &Item, b: &Item, layer: Option<&str>) -> Option<Hit<'_>> {
        for r in self.rules.iter().rev() {
            let Some(c) = r.constraints.iter().find(|c| c.kind == kind) else { continue };
            if !layer_ok(&r.layer, layer, &[]) {
                continue;
            }
            if eval(&r.expr, Some(a), Some(b)) || eval(&r.expr, Some(b), Some(a)) {
                return Some(Hit { rule: r, constraint: c });
            }
        }
        None
    }

    /// The constraint of `kind` on one item.
    pub fn single(&self, kind: &str, a: &Item, layer: Option<&str>) -> Option<Hit<'_>> {
        for r in self.rules.iter().rev() {
            let Some(c) = r.constraints.iter().find(|c| c.kind == kind) else { continue };
            if !layer_ok(&r.layer, layer.or(a.layers.first().map(String::as_str)), &[]) {
                continue;
            }
            if eval(&r.expr, Some(a), None) {
                return Some(Hit { rule: r, constraint: c });
            }
        }
        None
    }

    /// Rules that disallow this item (every matching one).
    pub fn disallowed(&self, a: &Item) -> Vec<&Rule> {
        let what: Vec<&str> = match (a.kind, a.via_type) {
            ("via", Some("micro")) => vec!["via", "micro_via"],
            ("via", Some("blind")) | ("via", Some("buried")) => vec!["via", "buried_via", "blind_via"],
            ("via", _) => vec!["via", "through_via"],
            (k, _) => vec![k],
        };
        self.rules
            .iter()
            .filter(|r| r.constraints.iter().any(|c| c.kind == "disallow" && c.disallow.iter().any(|d| what.contains(&d.as_str()))) && layer_ok(&r.layer, a.layers.first().map(String::as_str), &[]) && eval(&r.expr, Some(a), None))
            .collect()
    }

    /// The largest clearance any rule asks for (for search reach).
    pub fn max_clearance(&self) -> f64 {
        self.rules.iter().flat_map(|r| &r.constraints).filter(|c| matches!(c.kind.as_str(), "clearance" | "hole_clearance" | "hole_to_hole" | "edge_clearance" | "courtyard_clearance")).filter_map(|c| c.min).fold(0.0, f64::max)
    }
}

/// `pcb.rules.set`: the board's custom design rules, in the KiCad `.kicad_dru`
/// language (see `GET /pcb/<id>` `custom_rules` for what was read, and the
/// violations they raise). "" clears them.
#[derive(Deserialize, JsonSchema)]
pub struct SetRules {
    pub board: StableId,
    pub text: String,
}

impl Command for SetRules {
    const NAME: &'static str = "pcb.rules.set";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut b = board_of(tx, self.board)?;
        let set = parse(&self.text).map_err(bad)?;
        b.drc_rules = self.text.trim().to_string();
        tx.set(self.board, b)?;
        let unchecked: Vec<String> = set.rules.iter().flat_map(|r| r.constraints.iter().filter(|c| c.unchecked).map(move |c| format!("{}: {}", r.name, c.kind))).collect();
        Ok(json!({ "board": self.board, "rules": set.rules, "unchecked": unchecked }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<SetRules>();
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRU: &str = r#"
        (version 1)
        # comments are allowed
        (rule "HV"
            (layer outer)
            (condition "A.NetClass == 'HV' && B.NetClass != 'HV'")
            (constraint clearance (min 1.5mm)))
        (rule "fine BGA"
            (condition "A.insideArea('BGA')")
            (constraint track_width (min 0.09mm) (max 8mil))
            (constraint clearance (min 0.09mm)))
        (rule "antenna"
            (condition "A.intersectsArea('ANT*')")
            (constraint disallow track via)
            (severity warning))
        (rule "power width"
            (condition "A.NetName == 'VBUS' || A.NetName == '+5V'")
            (constraint track_width (min 0.5mm)))
        (rule "micro"
            (condition "A.Via_Type == 'Micro'")
            (constraint hole_size (min 0.1mm) (max 0.15mm)))
        (rule "len" (constraint length (max 50mm)))
    "#;

    fn track(net: &str, class: &str, layer: &str) -> Item {
        Item { kind: "track", net: Some(net.into()), netclass: Some(class.into()), layers: vec![layer.into()], width: Some(0.2), ..Default::default() }
    }

    #[test]
    fn parses_a_rules_file() {
        let s = parse(DRU).unwrap();
        assert_eq!(s.rules.len(), 6);
        assert_eq!(s.rules[0].layer.as_deref(), Some("outer"));
        assert_eq!(s.rules[1].constraints[0].max, Some(8.0 * 0.0254));
        assert_eq!(s.rules[2].constraints[0].disallow, vec!["track", "via"]);
        assert_eq!(s.rules[2].severity, "warning");
        assert!(s.rules[5].constraints[0].unchecked, "length is parked");
        assert!((s.max_clearance() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn rejects_bad_rules() {
        assert!(parse("(rule x (constraint clearance (min 1mm))").is_err(), "unbalanced");
        assert!(parse("(rule x (constraint spacing (min 1mm)))").is_err(), "unknown constraint");
        assert!(parse("(rule x (condition \"A.NetClass == \") (constraint clearance (min 1mm)))").is_err(), "bad condition");
        assert!(parse("(rule x (condition \"NetClass == 'x'\") (constraint clearance (min 1mm)))").is_err(), "no A./B.");
        assert!(parse("(rule x (constraint clearance (min 1parsec)))").is_err(), "bad unit");
    }

    #[test]
    fn pair_rules_match_either_way_round_and_by_layer() {
        let s = parse(DRU).unwrap();
        let hv = track("MAINS", "HV", "top");
        let lv = track("SIG", "Default", "top");
        let hit = s.pair("clearance", &lv, &hv, Some("top")).unwrap();
        assert_eq!(hit.rule.name, "HV");
        assert_eq!(hit.constraint.min, Some(1.5));
        assert!(s.pair("clearance", &lv, &hv, Some("in1")).is_none(), "outer layers only");
        let hv2 = track("MAINS2", "HV", "top");
        assert!(s.pair("clearance", &hv, &hv2, Some("top")).is_none(), "HV to HV is not this rule");
    }

    #[test]
    fn later_rules_win() {
        let s = parse("(rule a (constraint clearance (min 0.3mm))) (rule b (condition \"A.Type == 'via'\") (constraint clearance (min 0.5mm)))").unwrap();
        let t = track("A", "Default", "top");
        let v = Item { kind: "via", layers: vec!["top".into(), "bottom".into()], ..Default::default() };
        assert_eq!(s.pair("clearance", &t, &t, None).unwrap().constraint.min, Some(0.3));
        assert_eq!(s.pair("clearance", &t, &v, None).unwrap().constraint.min, Some(0.5));
    }

    #[test]
    fn single_rules_areas_and_disallow() {
        let s = parse(DRU).unwrap();
        let mut t = track("VBUS", "Power", "bottom");
        assert_eq!(s.single("track_width", &t, None).unwrap().rule.name, "power width");
        t.inside = vec!["BGA".into()];
        assert_eq!(s.single("track_width", &t, None).unwrap().rule.name, "power width", "later rule wins");
        let mut sig = track("D0", "Default", "top");
        sig.inside = vec!["BGA".into()];
        assert_eq!(s.single("track_width", &sig, None).unwrap().rule.name, "fine BGA");
        sig.inside.clear();
        sig.touches = vec!["ANTENNA".into()];
        assert_eq!(s.disallowed(&sig).len(), 1);
        let pad = Item { kind: "pad", touches: vec!["ANTENNA".into()], ..Default::default() };
        assert!(s.disallowed(&pad).is_empty(), "pads are allowed");
        let micro = Item { kind: "via", via_type: Some("micro"), hole: Some(0.2), ..Default::default() };
        let h = s.single("hole_size", &micro, None).unwrap();
        assert_eq!(h.constraint.max, Some(0.15));
    }

    #[test]
    fn expressions() {
        let it = Item { kind: "pad", net: Some("/USB/D+".into()), reference: Some("U12".into()), layers: vec!["top".into()], pad_type: Some("smd"), plated: false, ..Default::default() };
        let t = |s: &str| eval(&parse_expr(s).unwrap(), Some(&it), None);
        assert!(t("A.NetName == '/USB/*'"));
        assert!(t("A.Reference == 'U1?'"));
        assert!(t("A.Layer == 'F.Cu'"), "KiCad layer names");
        assert!(t("A.Layer == 'top'"));
        assert!(t("A.existsOnLayer('F.Cu')"));
        assert!(t("A.Pad_Type == 'SMD' && !A.isPlated()"));
        assert!(t("A.Type == 'pad' || A.Type == 'via'"));
        assert!(!t("A.Type != 'Pad'"));
        assert!(!t("B.NetName == '/USB/D+'"), "no B for a single item");
        assert!(t("(A.NetClass == 'x') || true"));
        let w = Item { kind: "track", width: Some(0.25), ..Default::default() };
        assert!(eval(&parse_expr("A.Width > 8mil && A.Width <= 0.25mm").unwrap(), Some(&w), None));
        assert!(wild("GND*", "gnd_A") && !wild("GND?", "GND12"));
    }
}
