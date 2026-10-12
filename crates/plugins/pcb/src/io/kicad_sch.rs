//! KiCad schematics (`.kicad_sch`, the documented S-expression format of
//! KiCad 6 to 9) in and out.
//!
//! Both use the sheet's own coordinates (mm, y down) as Forma does. A symbol
//! instance's `(at x y angle)` and `(mirror y)` are Forma's `rotation` and
//! `mirror`; `(mirror x)` is `mirror` turned a further 180°. Library symbols
//! travel inside the file (`lib_symbols`): Forma's built-in symbols are
//! written as `forma:<name>` and come back as themselves; any other symbol
//! comes back carrying the definition the file holds. KiCad power symbols
//! are Forma power ports (their Value is the net), and Forma writes each
//! power net as a `power:<net>` symbol of its own drawing.
//!
//! Hierarchy: the root sheet is the file asked for; each `(sheet …)` box
//! names its file, read from the companions given with it. Forma sheets after
//! the first are written as KiCad sub-sheets (boxes on their parent, or on
//! the root sheet when they have none).

use super::sch::SchIn;
use crate::schematic::{self, LabelKind, SchJunction, SchLabel, SchNoConnect, SchSymbol, SchText, SchWire, Sheet, SheetPin, TitleBlock};
use crate::sexpr::{self, Sexp, atom, list, n, s};
use crate::symbols::{Dir, Fill, Gfx, PinDef, PinType, SymbolDef};
use de_core::{StableId, bevy_ecs::world::World};
use std::collections::BTreeMap;

type P = [f64; 2];

// ---- Reading -------------------------------------------------------------------------

fn paper_of(e: &Sexp) -> (String, bool) {
    match e.find("paper") {
        Some(p) => {
            let name = p.arg_text(0).unwrap_or("A4").to_string();
            let portrait = p.has_atom("portrait");
            if name == "User" {
                let w = p.arg_num(1).unwrap_or(297.0);
                let h = p.arg_num(2).unwrap_or(210.0);
                let (big, small) = (w.max(h), w.min(h));
                let named = if big > 600.0 { "A1" } else if big > 430.0 { "A2" } else if big > 300.0 { "A3" } else { "A4" };
                let _ = small;
                (named.into(), h > w)
            } else {
                (name, portrait)
            }
        }
        None => ("A4".into(), false),
    }
}

fn prop<'a>(e: &'a Sexp, key: &str) -> Option<&'a str> {
    e.find_all("property").find(|p| p.arg_text(0).is_some_and(|k| k.eq_ignore_ascii_case(key))).and_then(|p| p.arg_text(1))
}

fn at_of(e: &Sexp) -> (P, f64) {
    let v = e.nums("at");
    ([*v.first().unwrap_or(&0.0), *v.get(1).unwrap_or(&0.0)], *v.get(2).unwrap_or(&0.0))
}

fn dir_angle(d: Dir) -> f64 {
    match d {
        Dir::Right => 0.0,
        Dir::Up => 90.0,
        Dir::Left => 180.0,
        Dir::Down => 270.0,
    }
}

/// A library symbol with its name made bare (`R` rather than `Device:R`) so its units
/// (`R_1_1`, `R_2_1`) are read by number.
fn lib_def(lib_id: &str, sym: &Sexp) -> SymbolDef {
    let (lib, bare) = lib_id.split_once(':').unwrap_or(("", lib_id));
    let mut e = sym.clone();
    if let Sexp::List(v) = &mut e {
        if v.len() > 1 {
            v[1] = Sexp::Str(bare.to_string());
        }
    }
    let mut d = crate::kicad_lib::symbol_def(if lib.is_empty() { "kicad" } else { lib }, &e, None);
    d.name = lib_id.to_string();
    d
}

struct Reader<'a> {
    files: &'a BTreeMap<String, String>,
    out: SchIn,
    next_sheet: usize,
    depth: usize,
}

impl Reader<'_> {
    fn sheet(&mut self, root: &Sexp, id: &str) {
        let placeholder = StableId::new();
        let mut libs: BTreeMap<String, SymbolDef> = BTreeMap::new();
        let mut power_libs: BTreeMap<String, bool> = BTreeMap::new();
        if let Some(ls) = root.find("lib_symbols") {
            for sym in ls.find_all("symbol") {
                let name = sym.arg_text(0).unwrap_or("").to_string();
                power_libs.insert(name.clone(), sym.find("power").is_some());
                libs.insert(name.clone(), lib_def(&name, sym));
            }
        }
        for e in root.args() {
            match e.head() {
                Some("wire") => {
                    let pts: Vec<P> = e.find("pts").map(|p| p.find_all("xy").map(|q| [q.arg_num(0).unwrap_or(0.0), q.arg_num(1).unwrap_or(0.0)]).collect()).unwrap_or_default();
                    if pts.len() >= 2 {
                        self.out.wires.push(SchWire { schematic: placeholder, sheet: id.into(), points: pts });
                    }
                }
                Some("junction") => self.out.junctions.push(SchJunction { schematic: placeholder, sheet: id.into(), at: at_of(e).0 }),
                Some("no_connect") => self.out.no_connects.push(SchNoConnect { schematic: placeholder, sheet: id.into(), at: at_of(e).0 }),
                Some(k @ ("label" | "global_label" | "hierarchical_label")) => {
                    let (at, angle) = at_of(e);
                    let kind = match k {
                        "global_label" => LabelKind::Global,
                        "hierarchical_label" => LabelKind::Hierarchical,
                        _ => LabelKind::Local,
                    };
                    let shape = e.find("shape").and_then(|x| x.arg_text(0)).map(str::to_string);
                    self.out.labels.push(SchLabel { schematic: placeholder, sheet: id.into(), kind, text: crate::kicad_lib::plain(e.arg_text(0).unwrap_or("")), at, rotation: angle.rem_euclid(360.0), shape: if kind == LabelKind::Local { None } else { shape }, style: None });
                }
                Some("text") => {
                    let (at, _) = at_of(e);
                    let size = e.find("effects").and_then(|x| x.find("font")).map(|f| f.nums("size")).and_then(|v| v.first().copied()).unwrap_or(1.27);
                    self.out.texts.push(SchText { schematic: placeholder, sheet: id.into(), at, text: e.arg_text(0).unwrap_or("").replace("\\n", "\n"), size: size.max(0.5) });
                }
                Some("bus") | Some("bus_entry") => self.out.note_once("buses are not read (their member nets join through labels)"),
                Some("symbol") => self.symbol(e, id, &libs, &power_libs, placeholder),
                Some("sheet") => self.child(e, id),
                _ => {}
            }
        }
    }

    fn symbol(&mut self, e: &Sexp, sheet: &str, libs: &BTreeMap<String, SymbolDef>, power_libs: &BTreeMap<String, bool>, placeholder: StableId) {
        let lib_id = e.value("lib_id").unwrap_or("").to_string();
        let (at, angle) = at_of(e);
        let (mirror, rotation) = match e.find("mirror").and_then(|m| m.arg_text(0)) {
            Some("y") => (true, angle),
            Some("x") => (true, angle + 180.0),
            _ => (false, angle),
        };
        let rotation = rotation.rem_euclid(360.0);
        let reference = e
            .find("instances")
            .and_then(|i| i.find("project"))
            .and_then(|p| p.find("path"))
            .and_then(|p| p.value("reference"))
            .or_else(|| prop(e, "Reference"))
            .unwrap_or("U?")
            .to_string();
        let value = prop(e, "Value").unwrap_or("").to_string();
        let def = libs.get(&lib_id).cloned();
        if power_libs.get(&lib_id).copied().unwrap_or(false) || reference.starts_with("#PWR") {
            // A power port: the net is its Value; it points from its pin into its body.
            let net = if value.is_empty() { def.as_ref().and_then(|d| d.pins.first()).map(|p| p.name.clone()).unwrap_or_default() } else { crate::kicad_lib::plain(&value) };
            let pin = def.as_ref().and_then(|d| d.pins.first()).cloned();
            let (anchor, rot) = match &pin {
                Some(p) => {
                    let pa = schematic::xf(at, rotation, mirror, p.at);
                    let v = schematic::xf([0.0, 0.0], rotation, mirror, p.side.v());
                    let out_dir = Dir::from_v([-v[0], -v[1]]);
                    (pa, dir_angle(out_dir))
                }
                None => (at, 270.0),
            };
            if net.is_empty() || net == "PWR_FLAG" || lib_id.ends_with("PWR_FLAG") {
                if lib_id.ends_with("PWR_FLAG") {
                    self.flag(at, rotation, mirror, sheet, placeholder, &reference);
                }
                return;
            }
            self.out.labels.push(SchLabel { schematic: placeholder, sheet: sheet.into(), kind: LabelKind::Power, text: net, at: anchor, rotation: rot, shape: None, style: None });
            return;
        }
        let unit = e.value_num("unit").unwrap_or(1.0).max(1.0) as u32;
        let footprint = prop(e, "Footprint").unwrap_or("").to_string();
        let mut fields = BTreeMap::new();
        let mut mpn = None;
        let mut manufacturer = None;
        let mut lcsc = None;
        let mut description = String::new();
        for p in e.find_all("property") {
            let (Some(k), Some(v)) = (p.arg_text(0), p.arg_text(1)) else { continue };
            if v.is_empty() || v == "~" {
                continue;
            }
            match k {
                "Reference" | "Value" | "Footprint" => {}
                "Description" | "ki_description" => description = v.to_string(),
                "MPN" | "Mfr Part #" | "Manufacturer_Part_Number" | "Part Number" | "MFR_PN" => mpn = Some(v.to_string()),
                "Manufacturer" | "MFR" | "Manufacturer_Name" => manufacturer = Some(v.to_string()),
                "LCSC" | "LCSC Part" | "LCSC Part #" | "JLCPCB Part" => lcsc = Some(v.to_string()),
                k if k.starts_with("ki_") || k.starts_with("Sim.") => {}
                k => {
                    fields.insert(k.to_string(), v.to_string());
                }
            }
        }
        let dnp = e.value("dnp") == Some("yes");
        let exclude_from_board = e.value("on_board") == Some("no");
        // Forma's own symbols come back as themselves.
        let builtin = lib_id.strip_prefix("forma:").and_then(crate::symbols::get);
        let (symbol, definition) = match (builtin, def) {
            (Some(b), _) => (b.name.clone(), None),
            (None, Some(d)) => (lib_id.clone(), Some(Box::new(d))),
            (None, None) => {
                self.out.note_once(format!("symbol {lib_id} is not in the file's lib_symbols; {reference} is drawn as a box"));
                ("IC".to_string(), None)
            }
        };
        let virtual_part = definition.as_ref().is_some_and(|d| d.virtual_part) || reference.starts_with('#');
        self.out.symbols.push(SchSymbol {
            schematic: placeholder,
            sheet: sheet.into(),
            symbol,
            reference,
            value,
            footprint,
            at,
            rotation,
            mirror,
            unit,
            mpn,
            manufacturer,
            lcsc,
            description,
            dnp,
            pin_map: BTreeMap::new(),
            pins: vec![],
            fields,
            ref_at: None,
            value_at: None,
            exclude_from_board: exclude_from_board && !virtual_part,
            component: None,
            definition,
        });
    }

    /// KiCad's PWR_FLAG: Forma's built-in flag symbol.
    fn flag(&mut self, at: P, rotation: f64, mirror: bool, sheet: &str, placeholder: StableId, reference: &str) {
        self.out.symbols.push(SchSymbol {
            schematic: placeholder,
            sheet: sheet.into(),
            symbol: "PWR_FLAG".into(),
            reference: reference.to_string(),
            value: "PWR_FLAG".into(),
            footprint: String::new(),
            at,
            rotation,
            mirror,
            unit: 1,
            mpn: None,
            manufacturer: None,
            lcsc: None,
            description: String::new(),
            dnp: false,
            pin_map: BTreeMap::new(),
            pins: vec![],
            fields: BTreeMap::new(),
            ref_at: None,
            value_at: None,
            exclude_from_board: false,
            component: None,
            definition: None,
        });
    }

    fn child(&mut self, e: &Sexp, parent: &str) {
        let (at, _) = at_of(e);
        let size = e.nums("size");
        let size = [*size.first().unwrap_or(&20.0), *size.get(1).unwrap_or(&15.0)];
        let name = prop(e, "Sheetname").or_else(|| prop(e, "Sheet name")).unwrap_or("Sheet").to_string();
        let file = prop(e, "Sheetfile").or_else(|| prop(e, "Sheet file")).unwrap_or("").to_string();
        let pins: Vec<SheetPin> = e
            .find_all("pin")
            .map(|p| {
                let (pa, _) = at_of(p);
                let tol = 0.01;
                let side = if (pa[0] - at[0]).abs() < tol {
                    Dir::Left
                } else if (pa[0] - at[0] - size[0]).abs() < tol {
                    Dir::Right
                } else if (pa[1] - at[1]).abs() < tol {
                    Dir::Up
                } else {
                    Dir::Down
                };
                SheetPin { name: crate::kicad_lib::plain(p.arg_text(0).unwrap_or("")), at: pa, side }
            })
            .collect();
        self.next_sheet += 1;
        let id = self.next_sheet.to_string();
        let base = file.rsplit(['/', '\\']).next().unwrap_or(&file).to_string();
        let text = self.files.get(&file).or_else(|| self.files.get(&base)).or_else(|| self.files.iter().find(|(k, _)| k.rsplit(['/', '\\']).next() == Some(base.as_str())).map(|(_, v)| v));
        let (paper, portrait) = text.and_then(|t| sexpr::parse(t).ok()).and_then(|v| v.into_iter().find(|x| x.is("kicad_sch"))).map(|r| paper_of(&r)).unwrap_or(("A4".into(), false));
        self.out.sheets.push(Sheet { id: id.clone(), name, paper, portrait, parent: Some(parent.to_string()), at: Some(at), size: Some(size), pins });
        match text {
            Some(t) if self.depth < 16 => match sexpr::parse(t).ok().and_then(|v| v.into_iter().find(|x| x.is("kicad_sch"))) {
                Some(r) => {
                    self.depth += 1;
                    self.sheet(&r, &id);
                    self.depth -= 1;
                }
                None => self.out.note_once(format!("sub-sheet file {file} is not a KiCad schematic")),
            },
            _ => self.out.note_once(format!("sub-sheet file {file} was not given; its sheet is empty (import the project bundle, or give the file alongside)")),
        }
    }
}

impl SchIn {
    fn note_once(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }
}

/// A `.kicad_sch` (and the sub-sheet files it names, from `files`) in Forma's terms.
pub fn read(text: &str, file: &str, files: &BTreeMap<String, String>) -> Result<SchIn, String> {
    let top = sexpr::parse(text)?;
    let root = top.iter().find(|e| e.is("kicad_sch")).ok_or("not a KiCad schematic (no kicad_sch)")?;
    let version = root.value_num("version").unwrap_or(0.0);
    if version > 0.0 && version < 20211014.0 {
        return Err("a KiCad 5 or older schematic: open and save it in KiCad 6 or later first".into());
    }
    let (paper, portrait) = paper_of(root);
    let tb = root.find("title_block");
    let title = TitleBlock {
        title: tb.and_then(|t| t.value("title")).unwrap_or("").to_string(),
        company: tb.and_then(|t| t.value("company")).unwrap_or("").to_string(),
        revision: tb.and_then(|t| t.value("rev")).unwrap_or("").to_string(),
        date: tb.and_then(|t| t.value("date")).unwrap_or("").to_string(),
        comments: tb.map(|t| t.find_all("comment").filter_map(|c| c.arg_text(1).map(str::to_string)).collect()).unwrap_or_default(),
    };
    let stem = file.rsplit(['/', '\\']).next().unwrap_or(file).trim_end_matches(".kicad_sch").to_string();
    let mut r = Reader { files, out: SchIn { name: (!title.title.is_empty()).then(|| title.title.clone()).or_else(|| (!stem.is_empty()).then(|| stem.clone())), title, ..Default::default() }, next_sheet: 1, depth: 0 };
    r.out.sheets.push(Sheet { id: "1".into(), name: "Root".into(), paper, portrait, parent: None, at: None, size: None, pins: vec![] });
    r.sheet(root, "1");
    Ok(r.out)
}

// ---- Writing -------------------------------------------------------------------------

fn uuid(seed: &str) -> Sexp {
    list("uuid", vec![s(crate::kicad::uuid(seed))])
}

fn font(size: f64) -> Sexp {
    list("font", vec![list("size", vec![n(size), n(size)])])
}

fn effects(size: f64, hide: bool, justify: Option<&str>) -> Sexp {
    let mut v = vec![font(size)];
    if let Some(j) = justify {
        v.push(list("justify", j.split(' ').map(atom).collect()));
    }
    if hide {
        v.push(atom("hide"));
    }
    list("effects", v)
}

fn property(key: &str, value: &str, at: P, angle: f64, hide: bool) -> Sexp {
    list("property", vec![s(key), s(value), list("at", vec![n(at[0]), n(at[1]), n(angle)]), effects(1.27, hide, None)])
}

fn stroke() -> Sexp {
    list("stroke", vec![list("width", vec![n(0.0)]), list("type", vec![atom("default")])])
}

fn fill(f: Fill) -> Sexp {
    list("fill", vec![list("type", vec![atom(match f {
        Fill::None => "none",
        Fill::Outline => "outline",
        Fill::Body => "background",
    })])])
}

/// Sheet (y down) → symbol library (y up).
fn up(p: P) -> P {
    [p[0], if p[1] == 0.0 { 0.0 } else { -p[1] }]
}

fn xy(p: P) -> Sexp {
    let q = up(p);
    list("xy", vec![n(q[0]), n(q[1])])
}

fn pt(head: &str, p: P) -> Sexp {
    let q = up(p);
    list(head, vec![n(q[0]), n(q[1])])
}

fn gfx(g: &Gfx) -> Sexp {
    match g {
        Gfx::Line { pts, .. } => list("polyline", vec![list("pts", pts.iter().map(|p| xy(*p)).collect()), stroke(), fill(Fill::None)]),
        Gfx::Poly { pts, fill: f } => list("polyline", vec![list("pts", pts.iter().chain(pts.first()).map(|p| xy(*p)).collect()), stroke(), fill(*f)]),
        Gfx::Rect { a, b, fill: f } => list("rectangle", vec![pt("start", *a), pt("end", *b), stroke(), fill(*f)]),
        Gfx::Circle { c, r, fill: f } => list("circle", vec![pt("center", *c), list("radius", vec![n(*r)]), stroke(), fill(*f)]),
        Gfx::Arc { c, r, start, end } => {
            // Counter-clockwise as seen on the sheet (y down).
            let at = |deg: f64| -> P {
                let t = deg.to_radians();
                [c[0] + r * t.cos(), c[1] - r * t.sin()]
            };
            let mut span = (end - start).rem_euclid(360.0);
            if span == 0.0 {
                span = 360.0;
            }
            list("arc", vec![pt("start", at(*start)), pt("mid", at(start + span / 2.0)), pt("end", at(start + span)), stroke(), fill(Fill::None)])
        }
        Gfx::Text { at, text, size } => {
            let q = up(*at);
            list("text", vec![s(text.clone()), list("at", vec![n(q[0]), n(q[1]), n(0.0)]), effects(*size, false, None)])
        }
    }
}

fn etype(t: PinType) -> &'static str {
    match t {
        PinType::Input => "input",
        PinType::Output => "output",
        PinType::Bidirectional => "bidirectional",
        PinType::TriState => "tri_state",
        PinType::Passive => "passive",
        PinType::PowerIn => "power_in",
        PinType::PowerOut => "power_out",
        PinType::OpenCollector => "open_collector",
        PinType::OpenEmitter => "open_emitter",
        PinType::NoConnect => "no_connect",
        PinType::Unspecified => "unspecified",
    }
}

/// KiCad's pin angle points from the connection point into the body.
fn pin_angle(d: Dir) -> f64 {
    match d {
        Dir::Left => 0.0,
        Dir::Down => 90.0,
        Dir::Right => 180.0,
        Dir::Up => 270.0,
    }
}

fn pin_sexp(p: &PinDef, number: &str, hide: bool) -> Sexp {
    let q = up(p.at);
    let mut v = vec![atom(etype(p.etype)), atom("line"), list("at", vec![n(q[0]), n(q[1]), n(pin_angle(p.side))]), list("length", vec![n(p.length)])];
    if hide {
        v.push(atom("hide"));
    }
    v.push(list("name", vec![s(if p.name.is_empty() { "~".to_string() } else { p.name.clone() }), effects(1.27, false, None)]));
    v.push(list("number", vec![s(number.to_string()), effects(1.27, false, None)]));
    list("pin", v)
}

/// A library symbol for `lib_symbols`. A pin landing on several pads becomes KiCad stacked pins.
fn lib_symbol(lib_id: &str, d: &SymbolDef, pin_map: &BTreeMap<String, Vec<String>>) -> Sexp {
    let bare = lib_id.split_once(':').map(|x| x.1).unwrap_or(lib_id).to_string();
    let mut v = vec![s(lib_id)];
    if d.virtual_part {
        v.push(list("power", vec![]));
    }
    if !d.show_pin_numbers {
        v.push(list("pin_numbers", vec![atom("hide")]));
    }
    let mut pn = vec![list("offset", vec![n(0.508)])];
    if !d.show_pin_names {
        pn.push(atom("hide"));
    }
    v.push(list("pin_names", pn));
    v.push(list("exclude_from_sim", vec![atom("no")]));
    v.push(list("in_bom", vec![atom(if d.virtual_part { "no" } else { "yes" })]));
    v.push(list("on_board", vec![atom(if d.virtual_part { "no" } else { "yes" })]));
    v.push(property("Reference", &d.prefix, [0.0, 0.0], 0.0, false));
    v.push(property("Value", if d.value.is_empty() { &bare } else { &d.value }, [0.0, 0.0], 0.0, false));
    v.push(property("Footprint", &d.footprint, [0.0, 0.0], 0.0, true));
    v.push(property("Datasheet", "~", [0.0, 0.0], 0.0, true));
    v.push(property("Description", &d.description, [0.0, 0.0], 0.0, true));
    let units = d.unit_count();
    for u in 1..=units {
        let mut body = vec![s(format!("{bare}_{u}_1"))];
        for g in d.units.get(u as usize - 1).into_iter().flatten() {
            body.push(gfx(g));
        }
        for p in d.pins.iter().filter(|p| if units == 1 { true } else { p.unit == u || p.unit == 0 }) {
            let pads = pin_map.get(&p.number).cloned().unwrap_or_else(|| p.pads());
            for (i, pad) in pads.iter().enumerate() {
                body.push(pin_sexp(p, pad, p.hidden || i > 0));
            }
        }
        v.push(list("symbol", body));
    }
    list("symbol", v)
}

/// Forma's drawing of a power port as a KiCad power symbol: the pin at the origin, the body below.
fn power_symbol(lib_id: &str, net: &str) -> Sexp {
    let ground = schematic::is_ground(net);
    let mut body = vec![s(format!("{}_0_1", lib_id.split_once(':').map(|x| x.1).unwrap_or(lib_id)))];
    if ground {
        body.push(gfx(&Gfx::Line { pts: vec![[0.0, 0.0], [0.0, 1.27]], width: 0.0 }));
        body.push(gfx(&Gfx::Poly { pts: vec![[-1.27, 1.27], [1.27, 1.27], [0.0, 2.54]], fill: Fill::None }));
    } else {
        body.push(gfx(&Gfx::Line { pts: vec![[0.0, 0.0], [0.0, 1.27]], width: 0.0 }));
        body.push(gfx(&Gfx::Line { pts: vec![[-0.762, 1.27], [0.762, 1.27]], width: 0.0 }));
    }
    let pin = PinDef { number: "1".into(), name: net.into(), at: [0.0, 0.0], side: Dir::Up, length: 0.0, etype: PinType::PowerIn, hidden: true, unit: 0, pads: vec![] };
    let mut v = vec![s(lib_id), list("power", vec![]), list("pin_numbers", vec![atom("hide")]), list("pin_names", vec![list("offset", vec![n(0.0)]), atom("hide")]), list("exclude_from_sim", vec![atom("no")]), list("in_bom", vec![atom("no")]), list("on_board", vec![atom("yes")])];
    v.push(property("Reference", "#PWR", [0.0, 0.0], 0.0, true));
    v.push(property("Value", net, [0.0, if ground { -3.81 } else { 3.81 }], 0.0, false));
    v.push(property("Footprint", "", [0.0, 0.0], 0.0, true));
    v.push(property("Datasheet", "", [0.0, 0.0], 0.0, true));
    v.push(property("Description", &format!("Power port {net}"), [0.0, 0.0], 0.0, true));
    v.push(list("symbol", body));
    let pins_unit = format!("{}_1_1", lib_id.split_once(':').map(|x| x.1).unwrap_or(lib_id));
    v.push(list("symbol", vec![s(pins_unit), pin_sexp(&pin, "1", true)]));
    list("symbol", v)
}

fn safe(sv: &str) -> String {
    sv.chars().map(|c| if c.is_alphanumeric() || matches!(c, '_' | '-' | '+' | '.') { c } else { '_' }).collect()
}

fn paper_sexp(sh: &Sheet) -> Sexp {
    let mut v = vec![s(sh.paper.clone())];
    if sh.portrait {
        v.push(atom("portrait"));
    }
    list("paper", v)
}

/// A schematic's sheets as KiCad files: (file name, text), the root first.
pub fn write(stem: &str, sch: &schematic::Schematic, it: &schematic::SchItems) -> Vec<(String, String)> {
    let project = stem.to_string();
    let root_uuid = crate::kicad::uuid(&format!("{stem}/root"));
    let sheets: Vec<Sheet> = if sch.sheets.is_empty() { vec![Sheet { id: "1".into(), name: "Root".into(), paper: "A4".into(), portrait: false, parent: None, at: None, size: None, pins: vec![] }] } else { sch.sheets.clone() };
    let root_id = sheets[0].id.clone();
    // Each non-root sheet hangs off its parent (or the root): its file, its box's uuid, its path.
    let file_of = |sh: &Sheet| if sh.id == root_id { format!("{stem}.kicad_sch") } else { format!("{stem}-{}.kicad_sch", safe(&sh.name)) };
    let parent_of = |sh: &Sheet| sh.parent.clone().filter(|p| *p != sh.id && sheets.iter().any(|x| x.id == *p)).unwrap_or_else(|| root_id.clone());
    let box_uuid = |sh: &Sheet| crate::kicad::uuid(&format!("{stem}/sheet/{}", sh.id));
    let path_of = |id: &str| -> String {
        let mut chain = vec![];
        let mut cur = sheets.iter().find(|x| x.id == id);
        let mut guard = 0;
        while let Some(sh) = cur {
            if sh.id == root_id || guard > 32 {
                break;
            }
            chain.push(box_uuid(sh));
            let p = parent_of(sh);
            cur = sheets.iter().find(|x| x.id == p);
            guard += 1;
        }
        chain.reverse();
        let mut p = format!("/{root_uuid}");
        for c in chain {
            p.push('/');
            p.push_str(&c);
        }
        p
    };
    // Library symbols: one entry per distinct definition.
    let mut lib_ids: BTreeMap<StableId, String> = BTreeMap::new();
    let mut libs: Vec<(String, SymbolDef, BTreeMap<String, Vec<String>>)> = Vec::new();
    for (id, sym) in &it.symbols {
        let Some(d) = sym.def() else { continue };
        let builtin = sym.definition.is_none() && sym.pins.is_empty() && crate::symbols::get(&sym.symbol).is_some();
        let base = if builtin {
            format!("forma:{}", sym.symbol)
        } else if d.name.contains(':') {
            d.name.clone()
        } else {
            format!("imported:{}", safe(&d.name))
        };
        let mut name = base.clone();
        let mut k = 1;
        loop {
            match libs.iter().find(|(n, _, _)| *n == name) {
                Some((_, ed, em)) if *ed == d && *em == sym.pin_map => break,
                Some(_) => {
                    k += 1;
                    name = format!("{base}_{k}");
                }
                None => {
                    libs.push((name.clone(), d.clone(), sym.pin_map.clone()));
                    break;
                }
            }
        }
        lib_ids.insert(*id, name);
    }
    let mut power_nets: Vec<String> = it.labels.iter().filter(|(_, l)| l.kind == LabelKind::Power).map(|(_, l)| l.text.clone()).collect();
    power_nets.sort();
    power_nets.dedup();
    let mut pwr_n = 0;
    let mut files = Vec::new();
    for sh in &sheets {
        let mut items: Vec<Sexp> = vec![
            list("version", vec![atom("20231120")]),
            list("generator", vec![s("forma")]),
            list("generator_version", vec![s("8.0")]),
            uuid(&format!("{stem}/file/{}", sh.id)),
            paper_sexp(sh),
        ];
        if sh.id == root_id {
            let mut tb = vec![list("title", vec![s(sch.title.title.clone())])];
            if !sch.title.date.is_empty() {
                tb.push(list("date", vec![s(sch.title.date.clone())]));
            }
            tb.push(list("rev", vec![s(sch.title.revision.clone())]));
            tb.push(list("company", vec![s(sch.title.company.clone())]));
            for (i, c) in sch.title.comments.iter().enumerate() {
                tb.push(list("comment", vec![n((i + 1) as f64), s(c.clone())]));
            }
            items.push(list("title_block", tb));
        }
        // lib_symbols: what this sheet uses.
        let on_sheet: Vec<&(StableId, SchSymbol)> = it.symbols.iter().filter(|(_, x)| x.sheet == sh.id).collect();
        let mut used: Vec<&String> = on_sheet.iter().filter_map(|(id, _)| lib_ids.get(id)).collect();
        used.sort();
        used.dedup();
        let mut lsyms: Vec<Sexp> = used.iter().filter_map(|nme| libs.iter().find(|(x, _, _)| x == *nme)).map(|(nme, d, pm)| lib_symbol(nme, d, pm)).collect();
        let sheet_power: Vec<&String> = power_nets.iter().filter(|net| it.labels.iter().any(|(_, l)| l.sheet == sh.id && l.kind == LabelKind::Power && l.text == **net)).collect();
        for net in &sheet_power {
            lsyms.push(power_symbol(&format!("power:{}", safe(net)), net));
        }
        items.push(list("lib_symbols", lsyms));
        for (id, j) in it.junctions.iter().filter(|(_, x)| x.sheet == sh.id) {
            items.push(list("junction", vec![list("at", vec![n(j.at[0]), n(j.at[1])]), list("diameter", vec![n(0.0)]), list("color", vec![n(0.0), n(0.0), n(0.0), n(0.0)]), uuid(&id.to_string())]));
        }
        for (id, x) in it.no_connects.iter().filter(|(_, x)| x.sheet == sh.id) {
            items.push(list("no_connect", vec![list("at", vec![n(x.at[0]), n(x.at[1])]), uuid(&id.to_string())]));
        }
        for (id, w) in it.wires.iter().filter(|(_, x)| x.sheet == sh.id) {
            for (k, seg) in w.points.windows(2).enumerate() {
                items.push(list("wire", vec![list("pts", vec![list("xy", vec![n(seg[0][0]), n(seg[0][1])]), list("xy", vec![n(seg[1][0]), n(seg[1][1])])]), stroke(), uuid(&format!("{id}/{k}"))]));
            }
        }
        for (id, t) in it.texts.iter().filter(|(_, x)| x.sheet == sh.id) {
            items.push(list("text", vec![s(t.text.clone()), list("exclude_from_sim", vec![atom("no")]), list("at", vec![n(t.at[0]), n(t.at[1]), n(0.0)]), effects(t.size, false, Some("left")), uuid(&id.to_string())]));
        }
        for (id, l) in it.labels.iter().filter(|(_, x)| x.sheet == sh.id) {
            let at = list("at", vec![n(l.at[0]), n(l.at[1]), n(l.rotation.rem_euclid(360.0))]);
            let just = if (90.0..270.0).contains(&l.rotation.rem_euclid(360.0)) { "right bottom" } else { "left bottom" };
            match l.kind {
                LabelKind::Local => items.push(list("label", vec![s(l.text.clone()), at, effects(1.27, false, Some(just)), uuid(&id.to_string())])),
                LabelKind::Global | LabelKind::Hierarchical => {
                    let head = if l.kind == LabelKind::Global { "global_label" } else { "hierarchical_label" };
                    let shape = l.shape.clone().unwrap_or_else(|| "passive".into());
                    let mut v = vec![s(l.text.clone()), list("shape", vec![atom(shape)]), at, effects(1.27, false, Some(if just.starts_with("right") { "right" } else { "left" })), uuid(&id.to_string())];
                    if l.kind == LabelKind::Global {
                        v.push(property("Intersheetrefs", "${INTERSHEET_REFS}", l.at, 0.0, true));
                    }
                    items.push(list(head, v));
                }
                LabelKind::Power => {
                    pwr_n += 1;
                    let lib = format!("power:{}", safe(&l.text));
                    let rot = (l.rotation + 90.0).rem_euclid(360.0);
                    let r = format!("#PWR{pwr_n:03}");
                    let label_at = schematic::xf(l.at, rot, false, [0.0, if schematic::is_ground(&l.text) { 3.81 } else { -3.81 }]);
                    items.push(list(
                        "symbol",
                        vec![
                            list("lib_id", vec![s(lib)]),
                            list("at", vec![n(l.at[0]), n(l.at[1]), n(rot)]),
                            list("unit", vec![n(1.0)]),
                            list("exclude_from_sim", vec![atom("no")]),
                            list("in_bom", vec![atom("yes")]),
                            list("on_board", vec![atom("yes")]),
                            list("dnp", vec![atom("no")]),
                            uuid(&id.to_string()),
                            property("Reference", &r, l.at, 0.0, true),
                            property("Value", &l.text, label_at, 0.0, false),
                            property("Footprint", "", l.at, 0.0, true),
                            property("Datasheet", "", l.at, 0.0, true),
                            list("pin", vec![s("1"), uuid(&format!("{id}/pin"))]),
                            list("instances", vec![list("project", vec![s(project.clone()), list("path", vec![s(path_of(&sh.id)), list("reference", vec![s(r.clone())]), list("unit", vec![n(1.0)])])])]),
                        ],
                    ));
                }
            }
        }
        for (id, sym) in &on_sheet {
            let Some(lib) = lib_ids.get(id) else { continue };
            let d = sym.def();
            let mut v = vec![list("lib_id", vec![s(lib.clone())]), list("at", vec![n(sym.at[0]), n(sym.at[1]), n(sym.rotation.rem_euclid(360.0))])];
            if sym.mirror {
                v.push(list("mirror", vec![atom("y")]));
            }
            v.push(list("unit", vec![n(sym.unit as f64)]));
            v.push(list("exclude_from_sim", vec![atom("no")]));
            v.push(list("in_bom", vec![atom(if d.as_ref().is_some_and(|d| d.virtual_part) { "no" } else { "yes" })]));
            v.push(list("on_board", vec![atom(if sym.exclude_from_board { "no" } else { "yes" })]));
            v.push(list("dnp", vec![atom(if sym.dnp { "yes" } else { "no" })]));
            v.push(uuid(&id.to_string()));
            let body = d.as_ref().map(|d| d.body_bbox(sym.unit)).unwrap_or([0.0, 0.0, 0.0, 0.0]);
            let ref_at = sym.ref_at.unwrap_or_else(|| schematic::xf(sym.at, 0.0, false, [body[2] + 1.27, body[1].min(0.0) + 1.27]));
            let value_at = sym.value_at.unwrap_or([ref_at[0], ref_at[1] + 2.54]);
            v.push(property("Reference", &sym.reference, ref_at, 0.0, false));
            v.push(property("Value", &sym.value, value_at, 0.0, false));
            v.push(property("Footprint", &sym.footprint, sym.at, 0.0, true));
            v.push(property("Datasheet", sym.fields.get("Datasheet").map(String::as_str).unwrap_or("~"), sym.at, 0.0, true));
            if !sym.description.is_empty() {
                v.push(property("Description", &sym.description, sym.at, 0.0, true));
            }
            for (k, val) in [("MPN", &sym.mpn), ("Manufacturer", &sym.manufacturer), ("LCSC", &sym.lcsc)] {
                if let Some(x) = val {
                    v.push(property(k, x, sym.at, 0.0, true));
                }
            }
            for (k, val) in &sym.fields {
                if k != "Datasheet" {
                    v.push(property(k, val, sym.at, 0.0, true));
                }
            }
            if let Some(d) = &d {
                for p in d.unit_pins(sym.unit) {
                    for pad in sym.pads_of(p) {
                        v.push(list("pin", vec![s(pad.clone()), uuid(&format!("{id}/{}/{pad}", p.number))]));
                    }
                }
            }
            v.push(list("instances", vec![list("project", vec![s(project.clone()), list("path", vec![s(path_of(&sh.id)), list("reference", vec![s(sym.reference.clone())]), list("unit", vec![n(sym.unit as f64)])])])]));
            items.push(list("symbol", v));
        }
        // Boxes of the sheets that hang off this one.
        let children: Vec<&Sheet> = sheets.iter().filter(|c| c.id != root_id && parent_of(c) == sh.id).collect();
        let paper = sh.paper_size();
        for (k, c) in children.iter().enumerate() {
            let size = c.size.unwrap_or([30.48, 20.32]);
            let at = c.at.unwrap_or([schematic::snap1(paper[0] - 60.0), schematic::snap1(25.4 + k as f64 * 30.48)]);
            let mut v = vec![
                list("at", vec![n(at[0]), n(at[1])]),
                list("size", vec![n(size[0]), n(size[1])]),
                list("fields_autoplaced", vec![atom("yes")]),
                list("stroke", vec![list("width", vec![n(0.1524)]), list("type", vec![atom("solid")])]),
                list("fill", vec![list("color", vec![n(0.0), n(0.0), n(0.0), n(0.0)])]),
                uuid(&format!("{stem}/sheet/{}", c.id)),
                list("property", vec![s("Sheetname"), s(c.name.clone()), list("at", vec![n(at[0]), n(at[1] - 0.7116), n(0.0)]), effects(1.27, false, Some("left bottom"))]),
                list("property", vec![s("Sheetfile"), s(file_of(c)), list("at", vec![n(at[0]), n(at[1] + size[1] + 0.5844), n(0.0)]), effects(1.27, false, Some("left top"))]),
            ];
            for (pi, p) in c.pins.iter().enumerate() {
                let angle = match p.side {
                    Dir::Left => 180.0,
                    Dir::Right => 0.0,
                    Dir::Up => 90.0,
                    Dir::Down => 270.0,
                };
                v.push(list("pin", vec![s(p.name.clone()), atom("passive"), list("at", vec![n(p.at[0]), n(p.at[1]), n(angle)]), effects(1.27, false, Some(if p.side == Dir::Left { "left" } else { "right" })), uuid(&format!("{stem}/sheet/{}/pin/{pi}", c.id))]));
            }
            let page = sheets.iter().position(|x| x.id == c.id).unwrap_or(0) + 1;
            v.push(list("instances", vec![list("project", vec![s(project.clone()), list("path", vec![s(path_of(&sh.id)), list("page", vec![s(page.to_string())])])])]));
            items.push(list("sheet", v));
        }
        if sh.id == root_id {
            items.push(list("sheet_instances", vec![list("path", vec![s("/"), list("page", vec![s("1")])])]));
        }
        let mut file = list("kicad_sch", items);
        if let Sexp::List(v) = &mut file {
            // The root uuid KiCad paths start from.
            if sh.id == root_id {
                v[4] = list("uuid", vec![s(root_uuid.clone())]);
            }
        }
        files.push((file_of(sh), sexpr::write(&file)));
    }
    files
}

/// A schematic in the project as KiCad files (root first).
pub fn write_world(world: &World, id: StableId, stem: &str) -> Option<Vec<(String, String)>> {
    let e = world.resource::<de_core::IdIndex>().get(id)?;
    let sch = world.get::<schematic::Schematic>(e)?.clone();
    let it = schematic::items(world, id);
    Some(write(stem, &sch, &it))
}
