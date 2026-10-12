//! Linked parameters: a parameter's value can be a formula over other
//! parameters, so one change drives everything derived from it.
//!
//! Formulas are arithmetic with references. People and the AI write
//! references by name (`Tower footprint.width`, `Desk.depth`, or a sibling
//! parameter of the same shape, `width`); the engine stores them by stable id
//! (`#<id>.width`), so renaming never breaks a link. A referenced entity
//! becomes a dependency: change it and every formula that reads it, the
//! shapes they drive, the assemblies above and the requirements on them are
//! recomputed.
//!
//! Grammar: numbers (with optional unit mm, cm, m, in, ft, deg), + − × ÷ ^,
//! parentheses, functions min max abs round floor ceil sqrt sin cos tan
//! (degrees), and references.

use crate::{Param, ShapeProgram};
use de_core::{
    CommandError, ErrorCode, IdIndex, Persist, StableId, Tx,
    bevy_ecs::{self, component::Component, entity::Entity, world::World},
    schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// Named values for a whole project (or a building, a product line): the
/// numbers many shapes derive from. No geometry of its own.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ParameterSet {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub params: Vec<Param>,
}

impl Persist for ParameterSet {
    const KIND: &'static str = "shape.parameter_set";
}

/// Derived: every parameter's value after formulas are worked out.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct ResolvedParams(pub BTreeMap<String, f64>);

// ---- Formulas ----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ref(String),
    Op(char),
    Open,
    Close,
    Comma,
}

/// Splits a formula into tokens. References run until an operator; names may
/// contain spaces and dots (`Tower footprint.width`).
fn tokens(src: &str) -> Result<Vec<Tok>, String> {
    let s: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && s.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < s.len() && (s[i].is_ascii_digit() || s[i] == '.' || ((s[i] == 'e' || s[i] == 'E') && s.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '-'))) {
                if s[i] == 'e' || s[i] == 'E' {
                    i += 1;
                }
                i += 1;
            }
            let mut v: f64 = s[start..i].iter().collect::<String>().parse().map_err(|_| format!("bad number near “{}”", s[start..i].iter().collect::<String>()))?;
            // A unit right after a number converts it to millimetres (degrees stay degrees).
            let rest: String = s[i..].iter().collect();
            for (unit, factor) in [("mm", 1.0), ("cm", 10.0), ("m", 1000.0), ("in", 25.4), ("ft", 304.8), ("deg", 1.0)] {
                let after = rest.chars().nth(unit.len());
                if rest.starts_with(unit) && !after.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    v *= factor;
                    i += unit.len();
                    break;
                }
            }
            out.push(Tok::Num(v));
        } else if "+-*/^×÷".contains(c) {
            out.push(Tok::Op(match c {
                '×' => '*',
                '÷' => '/',
                c => c,
            }));
            i += 1;
        } else if c == '(' {
            out.push(Tok::Open);
            i += 1;
        } else if c == ')' {
            out.push(Tok::Close);
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if c == '#' {
            // A stored reference: #<uuid>.<key> (the id has hyphens, which are not minus here).
            let start = i;
            i += 1;
            while i < s.len() && (s[i].is_ascii_hexdigit() || s[i] == '-') {
                i += 1;
            }
            if s.get(i) == Some(&'.') {
                i += 1;
                while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == '_') {
                    i += 1;
                }
            }
            out.push(Tok::Ref(s[start..i].iter().collect()));
        } else if c.is_alphabetic() || c == '_' || c == '“' || c == '"' {
            let start = i;
            while i < s.len() && !"+-*/^×÷(),".contains(s[i]) {
                i += 1;
            }
            let name: String = s[start..i].iter().collect::<String>().trim().trim_matches(|c| c == '"' || c == '“' || c == '”').to_string();
            out.push(Tok::Ref(name));
        } else {
            return Err(format!("unexpected “{c}”"));
        }
    }
    Ok(out)
}

/// A parsed formula.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    /// A reference: entity (None = this one) and parameter key.
    Ref(Option<StableId>, String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

const FUNCTIONS: &[&str] = &[
    "min", "max", "abs", "round", "floor", "ceil", "sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sum", "average", "avg", "count", "mod", "pow", "hypot", "exp", "ln", "log", "pi",
];

struct Parser<'a> {
    toks: Vec<Tok>,
    at: usize,
    resolve: &'a dyn Fn(&str) -> Result<(Option<StableId>, String), String>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }
    fn next(&mut self) -> Option<Tok> {
        self.at += 1;
        self.toks.get(self.at - 1).cloned()
    }
    fn expr(&mut self) -> Result<Expr, String> {
        let mut left = self.term()?;
        while let Some(Tok::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.next();
            left = Expr::Bin(op, Box::new(left), Box::new(self.term()?));
        }
        Ok(left)
    }
    fn term(&mut self) -> Result<Expr, String> {
        let mut left = self.power()?;
        while let Some(Tok::Op(op @ ('*' | '/'))) = self.peek().cloned() {
            self.next();
            left = Expr::Bin(op, Box::new(left), Box::new(self.power()?));
        }
        Ok(left)
    }
    fn power(&mut self) -> Result<Expr, String> {
        let base = self.unary()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.next();
            return Ok(Expr::Bin('^', Box::new(base), Box::new(self.power()?)));
        }
        Ok(base)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Tok::Op('-')) => {
                self.next();
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some(Tok::Op('+')) => {
                self.next();
                self.unary()
            }
            _ => self.atom(),
        }
    }
    fn atom(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(v)) => Ok(Expr::Num(v)),
            Some(Tok::Open) => {
                let e = self.expr()?;
                match self.next() {
                    Some(Tok::Close) => Ok(e),
                    _ => Err("missing “)”".into()),
                }
            }
            Some(Tok::Ref(name)) => {
                // Function names in any case (SUM, Sum, sum: spreadsheets write them in capitals).
                let lower = name.to_ascii_lowercase();
                if FUNCTIONS.contains(&lower.as_str()) && self.peek() == Some(&Tok::Open) {
                    self.next();
                    if self.peek() == Some(&Tok::Close) {
                        self.next();
                        return Ok(Expr::Call(lower, vec![]));
                    }
                    let name = lower;
                    let mut args = vec![self.expr()?];
                    loop {
                        match self.next() {
                            Some(Tok::Comma) => args.push(self.expr()?),
                            Some(Tok::Close) => break,
                            _ => return Err(format!("missing “)” after {name}(…")),
                        }
                    }
                    return Ok(Expr::Call(name, args));
                }
                let (id, key) = (self.resolve)(&name)?;
                Ok(Expr::Ref(id, key))
            }
            Some(t) => Err(format!("unexpected {t:?}")),
            None => Err("the formula ends too early".into()),
        }
    }
}

/// Parses a formula, resolving each reference with `resolve`.
pub fn parse(src: &str, resolve: &dyn Fn(&str) -> Result<(Option<StableId>, String), String>) -> Result<Expr, String> {
    let src = src.trim().trim_start_matches('=');
    let mut p = Parser { toks: tokens(src)?, at: 0, resolve };
    let e = p.expr()?;
    if p.at != p.toks.len() {
        return Err(format!("unexpected {:?}", p.toks[p.at]));
    }
    Ok(e)
}

impl Expr {
    pub fn eval(&self, get: &dyn Fn(Option<StableId>, &str) -> Result<f64, String>) -> Result<f64, String> {
        Ok(match self {
            Expr::Num(v) => *v,
            Expr::Ref(id, key) => get(*id, key)?,
            Expr::Neg(e) => -e.eval(get)?,
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(get)?, b.eval(get)?);
                match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    '/' if b == 0.0 => return Err("division by zero".into()),
                    '/' => a / b,
                    _ => a.powf(b),
                }
            }
            Expr::Call(f, args) => {
                let v: Vec<f64> = args.iter().map(|a| a.eval(get)).collect::<Result<_, _>>()?;
                let one = || v.first().copied().ok_or_else(|| format!("{f}() needs a value"));
                match f.as_str() {
                    "min" => v.iter().copied().fold(f64::INFINITY, f64::min),
                    "max" => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    "abs" => one()?.abs(),
                    "round" => one()?.round(),
                    "floor" => one()?.floor(),
                    "ceil" => one()?.ceil(),
                    "sqrt" => one()?.sqrt(),
                    "sin" => one()?.to_radians().sin(),
                    "cos" => one()?.to_radians().cos(),
                    "tan" => one()?.to_radians().tan(),
                    "asin" => one()?.asin().to_degrees(),
                    "acos" => one()?.acos().to_degrees(),
                    "atan" => one()?.atan().to_degrees(),
                    "atan2" if v.len() == 2 => v[0].atan2(v[1]).to_degrees(),
                    "sum" => v.iter().sum(),
                    "average" | "avg" if !v.is_empty() => v.iter().sum::<f64>() / v.len() as f64,
                    "count" => v.len() as f64,
                    "mod" if v.len() == 2 && v[1] != 0.0 => v[0] - v[1] * (v[0] / v[1]).floor(),
                    "pow" if v.len() == 2 => v[0].powf(v[1]),
                    "hypot" => v.iter().map(|x| x * x).sum::<f64>().sqrt(),
                    "exp" => one()?.exp(),
                    "ln" => one()?.ln(),
                    "log" => one()?.log10(),
                    "pi" if v.is_empty() => std::f64::consts::PI,
                    "atan2" | "pow" => return Err(format!("{f}() takes two values")),
                    "mod" => return Err("mod() takes two values, the second not zero".into()),
                    "pi" => return Err("pi() takes no values".into()),
                    "average" | "avg" => return Err(format!("{f}() needs a value")),
                    _ => return Err(format!("unknown function {f}")),
                }
            }
        })
    }

    /// Entities this formula reads.
    pub fn refs(&self, out: &mut Vec<(Option<StableId>, String)>) {
        match self {
            Expr::Ref(id, key) => out.push((*id, key.clone())),
            Expr::Neg(e) => e.refs(out),
            Expr::Bin(_, a, b) => {
                a.refs(out);
                b.refs(out);
            }
            Expr::Call(_, args) => args.iter().for_each(|a| a.refs(out)),
            Expr::Num(_) => {}
        }
    }
}

/// Stored formulas reference by id: `#<uuid>.key`. Parses one of those.
pub(crate) fn stored_ref(name: &str) -> Result<(Option<StableId>, String), String> {
    if let Some(rest) = name.strip_prefix('#') {
        let (id, key) = rest.split_once('.').ok_or_else(|| format!("bad reference {name}"))?;
        let id: StableId = serde_json::from_value(serde_json::json!(id)).map_err(|_| format!("bad reference {name}"))?;
        return Ok((Some(id), key.to_string()));
    }
    Ok((None, name.to_string()))
}

/// Parses a stored formula (references by id).
pub fn parse_stored(src: &str) -> Result<Expr, String> {
    parse(src, &stored_ref)
}

/// The parameters of an entity that can be referenced: a shape's or a parameter set's.
pub fn params_of(world: &World, e: Entity) -> Option<(&str, &[Param])> {
    if let Some(p) = world.get::<ShapeProgram>(e) {
        return Some((&p.name, &p.params));
    }
    if let Some(s) = world.get::<crate::sketch::Sketch>(e) {
        return Some((&s.name, &s.params));
    }
    world.get::<ParameterSet>(e).map(|s| (s.name.as_str(), s.params.as_slice()))
}

/// The name of an entity whose values formulas can read, and the keys it
/// has: a shape's, sketch's or parameter set's parameters, or a sheet's
/// cells (by address) and their aliases.
pub fn keys_of(world: &World, e: Entity) -> Option<(String, Vec<String>)> {
    if let Some((name, params)) = params_of(world, e) {
        return Some((name.to_string(), params.iter().map(|p| p.key.clone()).collect()));
    }
    world.get::<crate::sheet::Sheet>(e).map(|s| (s.name.clone(), s.keys()))
}

/// Entities with referenceable values, by name.
pub(crate) fn find_by_name(world: &World, name: &str) -> Vec<StableId> {
    let index = world.resource::<IdIndex>();
    index
        .iter()
        .filter(|(_, e)| keys_of(world, *e).is_some_and(|(n, _)| n == name))
        .map(|(id, _)| id)
        .collect()
}

/// Resolves a reference to another entity's value as people write it
/// (`Tower footprint.width`, `Sizes.B2`) or as stored (`#<id>.key`).
pub(crate) fn resolve_external(world: &World, name: &str) -> Result<(Option<StableId>, String), String> {
    if name.starts_with('#') {
        let (id, key) = stored_ref(name)?;
        let e = id.and_then(|id| world.resource::<IdIndex>().get(id)).ok_or_else(|| format!("{name} does not exist"))?;
        let (_, keys) = keys_of(world, e).ok_or_else(|| format!("{name} has no parameters"))?;
        if !keys.iter().any(|k| *k == key) {
            return Err(format!("{name}: no parameter “{key}”"));
        }
        return Ok((id, key));
    }
    // "Entity name.key": the name may itself contain dots, so try every split.
    for (i, _) in name.match_indices('.').rev() {
        let (ent, key) = (&name[..i], &name[i + 1..]);
        let found = find_by_name(world, ent.trim());
        match found.as_slice() {
            [] => continue,
            [id] => {
                let e = world.resource::<IdIndex>().get(*id).unwrap();
                let (_, keys) = keys_of(world, e).unwrap();
                // A sheet's cells are written in any case (b2 is B2).
                let key = key.trim();
                let found = keys.iter().find(|k| *k == key).or_else(|| keys.iter().find(|k| k.eq_ignore_ascii_case(key) && crate::sheet::is_address(k))).cloned();
                let Some(key) = found else {
                    let shown: Vec<&str> = keys.iter().map(String::as_str).take(40).collect();
                    return Err(format!("“{ent}” has no parameter “{key}” (it has {})", shown.join(", ")));
                };
                return Ok((Some(*id), key));
            }
            _ => return Err(format!("more than one thing is called “{ent}”; rename one")),
        }
    }
    Err(format!("“{name}” is not a parameter here or Name.parameter of anything in the project"))
}

/// Turns a formula as people write it (names) into its stored form (ids),
/// checking that every reference exists. `own` are this entity's parameter keys.
pub fn canonical(world: &World, src: &str, own: &[String]) -> Result<String, String> {
    let resolve = |name: &str| -> Result<(Option<StableId>, String), String> {
        if !name.starts_with('#') && own.iter().any(|k| k == name) {
            return Ok((None, name.to_string()));
        }
        resolve_external(world, name)
    };
    let e = parse(src, &resolve)?;
    Ok(format!("={}", write(&e, &|id, key| match id {
        Some(id) => format!("#{id}.{key}"),
        None => key.to_string(),
    })))
}

/// A formula with references shown by current names (for people).
pub fn display(world: &World, stored: &str) -> String {
    match parse_stored(stored) {
        Ok(e) => format!(
            "={}",
            write(&e, &|id, key| match id.and_then(|id| world.resource::<IdIndex>().get(id)).and_then(|e| keys_of(world, e)) {
                Some((name, _)) => format!("{name}.{key}"),
                None if id.is_some() => format!("(missing).{key}"),
                None => key.to_string(),
            })
        ),
        Err(_) => stored.to_string(),
    }
}

pub(crate) fn write(e: &Expr, name: &dyn Fn(Option<StableId>, &str) -> String) -> String {
    write_at(e, name, 0)
}

/// Prints with only the brackets the precedence needs (`a - (b - c)`, not `(a - b) - c`).
fn write_at(e: &Expr, name: &dyn Fn(Option<StableId>, &str) -> String, outer: u8) -> String {
    let prec = |op: char| match op {
        '+' | '-' => 1,
        '*' | '/' => 2,
        _ => 3,
    };
    match e {
        Expr::Num(v) => format!("{v}"),
        Expr::Ref(id, key) => name(*id, key),
        Expr::Neg(e) => format!("-{}", write_at(e, name, 4)),
        Expr::Bin(op, a, b) => {
            let p = prec(*op);
            // Right operands of - and / (and left of ^) need brackets at equal precedence.
            let (lp, rp) = if *op == '^' { (p + 1, p) } else { (p, p + 1) };
            let text = format!("{} {op} {}", write_at(a, name, lp), write_at(b, name, rp));
            if p < outer { format!("({text})") } else { text }
        }
        Expr::Call(f, args) => format!("{f}({})", args.iter().map(|a| write_at(a, name, 0)).collect::<Vec<_>>().join(", ")),
    }
}

/// Every entity a parameter list's formulas read (for dependencies).
pub fn references(params: &[Param]) -> Vec<StableId> {
    let mut out = Vec::new();
    for p in params {
        if let Some(f) = &p.expr
            && let Ok(e) = parse_stored(f)
        {
            let mut refs = Vec::new();
            e.refs(&mut refs);
            out.extend(refs.into_iter().filter_map(|(id, _)| id));
        }
    }
    out.sort_by_key(|id| id.to_string());
    out.dedup();
    out
}

/// Values of a parameter list after formulas, reading other entities' resolved values.
pub fn resolve(world: &World, params: &[Param]) -> Result<BTreeMap<String, f64>, String> {
    resolve_with(world, params, &|_, _| None)
}

/// [`resolve`], with some other entities' values given instead of read
/// (`given(entity, key)`): what a formula would work out to if they changed —
/// a joint linked to another follows it through a motion sweep.
pub fn resolve_with(world: &World, params: &[Param], given: &dyn Fn(StableId, &str) -> Option<f64>) -> Result<BTreeMap<String, f64>, String> {
    use std::cell::RefCell;
    let done: RefCell<BTreeMap<String, f64>> = RefCell::new(BTreeMap::new());
    let visiting: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    type Given<'a> = &'a dyn Fn(StableId, &str) -> Option<f64>;
    fn one(world: &World, params: &[Param], key: &str, done: &RefCell<BTreeMap<String, f64>>, visiting: &RefCell<HashSet<String>>, given: Given) -> Result<f64, String> {
        if let Some(v) = done.borrow().get(key) {
            return Ok(*v);
        }
        let p = params.iter().find(|p| p.key == key).ok_or_else(|| format!("no parameter “{key}”"))?;
        let v = match &p.expr {
            None => p.value,
            Some(f) => {
                if !visiting.borrow_mut().insert(key.to_string()) {
                    return Err(format!("“{}” refers to itself", p.label));
                }
                let e = parse_stored(f).map_err(|e| format!("{}: {e}", p.label))?;
                let v = e
                    .eval(&|id, k| match id {
                        None => one(world, params, k, done, visiting, given),
                        Some(id) if given(id, k).is_some() => Ok(given(id, k).unwrap()),
                        Some(id) => {
                            let ent = world.resource::<IdIndex>().get(id).ok_or_else(|| format!("{}: a referenced object was deleted", p.label))?;
                            world
                                .get::<ResolvedParams>(ent)
                                .and_then(|r| r.0.get(k).copied())
                                .ok_or_else(|| format!("{}: “{k}” of a referenced object has no value", p.label))
                        }
                    })
                    .map_err(|e| format!("{}: {e}", p.label))?;
                visiting.borrow_mut().remove(key);
                if !v.is_finite() {
                    return Err(format!("{} works out to {v}", p.label));
                }
                if p.min.is_some_and(|m| v < m - 1e-9) || p.max.is_some_and(|m| v > m + 1e-9) {
                    return Err(format!(
                        "{} works out to {}, outside {}–{}",
                        p.label,
                        (v * 100.0).round() / 100.0,
                        p.min.map(|m| m.to_string()).unwrap_or("−∞".into()),
                        p.max.map(|m| m.to_string()).unwrap_or("∞".into())
                    ));
                }
                v
            }
        };
        done.borrow_mut().insert(key.to_string(), v);
        Ok(v)
    }
    for p in params {
        one(world, params, &p.key, &done, &visiting, given)?;
    }
    Ok(done.into_inner())
}

pub(crate) fn resolved_params(world: &World, e: Entity) -> Option<Result<ResolvedParams, String>> {
    if let Some(sheet) = world.get::<crate::sheet::Sheet>(e) {
        return Some(Ok(ResolvedParams(crate::sheet::evaluate(world, sheet).numbers())));
    }
    let (_, params) = params_of(world, e)?;
    Some(resolve(world, params).map(ResolvedParams))
}

/// A program with its parameters' formulas worked out, ready to evaluate. In
/// its active configuration, the finishes that configuration leaves off are
/// left off, the parts it leaves out are dropped when it runs, and it is
/// named for it (“Bolt (M8×30)”).
pub fn resolved_program(world: &World, e: Entity) -> Option<ShapeProgram> {
    let mut program = world.get::<ShapeProgram>(e)?.clone();
    if let Some(row) = world.get::<crate::ShapeConfigurations>(e).and_then(|t| t.active_row()) {
        crate::configs::suppress_finishes(&mut program, row);
        program.name = crate::configs::display_name(&program.name, Some(&row.name));
    }
    // A sketch that cannot be read leaves the inputs empty; evaluation then says which.
    program.inputs = crate::program_inputs(world, &program).unwrap_or_else(|_| format!(r#"{{"finishes":{}}}"#, serde_json::to_string(&program.finishes).unwrap()));
    if let Some(row) = world.get::<crate::ShapeConfigurations>(e).and_then(|t| t.active_row()) {
        program.inputs = crate::configs::with_row(&program.inputs, row);
    }
    if let Some(r) = world.get::<ResolvedParams>(e) {
        for p in &mut program.params {
            if let Some(v) = r.0.get(&p.key) {
                p.value = *v;
            }
        }
    }
    Some(program)
}

/// Checks formulas and keeps the entity's dependencies on what they read in
/// step: called whenever a shape's or parameter set's parameters change.
pub(crate) fn link(tx: &mut Tx, id: StableId, old: &[Param], params: &mut [Param]) -> Result<(), CommandError> {
    let own: Vec<String> = params.iter().map(|p| p.key.clone()).collect();
    for p in params.iter_mut() {
        if let Some(f) = &p.expr {
            let stored = canonical(tx.world(), f, &own).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, format!("{}: {e}", p.label)))?;
            p.expr = Some(stored);
        }
    }
    let before = references(old);
    let after = references(params);
    for r in &after {
        if *r == id {
            continue; // own parameters are read directly
        }
        if depends_on(tx, *r, id, 0) {
            return Err(CommandError::new(ErrorCode::InvalidParameters, "these formulas would make a loop: something they read already depends on this"));
        }
    }
    for r in before.iter().filter(|r| !after.contains(r)) {
        if *r != id {
            tx.remove_dependency(id, *r)?;
        }
    }
    for r in after.iter().filter(|r| **r != id) {
        tx.add_dependency(id, *r)?;
    }
    Ok(())
}

/// Whether `a` depends on `b`, directly or through others.
pub(crate) fn depends_on(tx: &Tx, a: StableId, b: StableId, depth: usize) -> bool {
    if a == b {
        return true;
    }
    if depth > 64 {
        return false;
    }
    tx.get::<de_core::DependsOn>(a).is_some_and(|d| d.0.iter().any(|x| depends_on(tx, *x, b, depth + 1)))
}
