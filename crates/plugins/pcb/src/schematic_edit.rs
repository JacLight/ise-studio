//! Editing many schematic items at once: move, turn and mirror a selection
//! (attached wires follow), align and distribute, duplicate (references
//! renumbered, label numbers stepped), find and replace, bulk field edits,
//! and the schematic's settings: ERC severities, pin-conflict matrix and
//! exclusions, net classes and net colours.

use crate::bad;
use crate::schematic::{
    self, FieldTemplate, LabelKind, SchBus, SchBusEntry, SchImage, SchItems, SchJunction, SchLabel, SchNetClass, SchNoConnect, SchShape, SchSymbol, SchTable, SchText, SchTextBox, SchWire, Schematic, add_item, check_color, check_reference, check_rotation, next_reference, owner, schematic_of, sheet_or_first, snap, snap1,
};
use crate::schematic_view::{self, P, PIN_TYPES, pin_pair_key, pin_type_name};
use crate::symbols::PinType;
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

fn yes() -> bool {
    true
}

/// A change of place for sheet points: mirror and quarter-turn about a pivot, then move.
#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub delta: P,
    /// 0, 90, 180 or 270, counter-clockwise on the sheet.
    pub rotate: f64,
    /// 'x' flips left-right, 'y' flips up-down.
    pub mirror: Option<char>,
    pub pivot: P,
}

impl Xf {
    pub fn shift(d: P) -> Xf {
        Xf { delta: d, rotate: 0.0, mirror: None, pivot: [0.0, 0.0] }
    }
    pub fn v(&self, v: P) -> P {
        let mut q = v;
        match self.mirror {
            Some('x') => q[0] = -q[0],
            Some('y') => q[1] = -q[1],
            _ => {}
        }
        schematic::xf([0.0, 0.0], self.rotate, false, q)
    }
    pub fn p(&self, p: P) -> P {
        let q = self.v([p[0] - self.pivot[0], p[1] - self.pivot[1]]);
        [q[0] + self.pivot[0] + self.delta[0], q[1] + self.pivot[1] + self.delta[1]]
    }
    /// A symbol's rotation and mirror after the change.
    pub fn symbol(&self, rotation: f64, mirror: bool) -> (f64, bool) {
        match self.mirror {
            Some('x') => ((self.rotate - rotation).rem_euclid(360.0), !mirror),
            Some('y') => ((self.rotate + 180.0 - rotation).rem_euclid(360.0), !mirror),
            _ => ((rotation + self.rotate).rem_euclid(360.0), mirror),
        }
    }
    /// A label's text direction after the change.
    pub fn direction(&self, rotation: f64) -> f64 {
        let u = schematic::xf([0.0, 0.0], rotation, false, [1.0, 0.0]);
        let w = self.v(u);
        let a = (-w[1]).atan2(w[0]).to_degrees();
        ((a / 90.0).round() * 90.0).rem_euclid(360.0)
    }
}

fn round(p: P) -> P {
    [(p[0] * 1e4).round() / 1e4, (p[1] * 1e4).round() / 1e4]
}

/// Where an item sits: the points that place it (for the pivot).
fn anchor_points(it: &SchItems, id: StableId) -> Vec<P> {
    if let Some((_, s)) = it.symbols.iter().find(|(x, _)| *x == id) {
        return vec![s.at];
    }
    if let Some((_, w)) = it.wires.iter().find(|(x, _)| *x == id) {
        return w.points.clone();
    }
    if let Some((_, w)) = it.buses.iter().find(|(x, _)| *x == id) {
        return w.points.clone();
    }
    if let Some((_, l)) = it.labels.iter().find(|(x, _)| *x == id) {
        return vec![l.at];
    }
    if let Some((_, j)) = it.junctions.iter().find(|(x, _)| *x == id) {
        return vec![j.at];
    }
    if let Some((_, j)) = it.no_connects.iter().find(|(x, _)| *x == id) {
        return vec![j.at];
    }
    if let Some((_, j)) = it.texts.iter().find(|(x, _)| *x == id) {
        return vec![j.at];
    }
    if let Some((_, j)) = it.bus_entries.iter().find(|(x, _)| *x == id) {
        return vec![j.at, j.wire_end()];
    }
    if let Some((_, j)) = it.shapes.iter().find(|(x, _)| *x == id) {
        return j.points.clone();
    }
    if let Some((_, j)) = it.text_boxes.iter().find(|(x, _)| *x == id) {
        return vec![j.at, [j.at[0] + j.size[0], j.at[1] + j.size[1]]];
    }
    if let Some((_, j)) = it.tables.iter().find(|(x, _)| *x == id) {
        let (w, h) = crate::schematic_graphics::table_layout(j);
        return vec![j.at, [j.at[0] + w.iter().sum::<f64>(), j.at[1] + h * j.cells.len() as f64]];
    }
    if let Some((_, j)) = it.images.iter().find(|(x, _)| *x == id) {
        let s = j.size();
        return vec![j.at, [j.at[0] + s[0], j.at[1] + s[1]]];
    }
    vec![]
}

/// A box (top-left `at`, `size`) after a change: same size, centre moved.
fn moved_box(x: &Xf, at: P, size: P) -> P {
    let c = x.p([at[0] + size[0] / 2.0, at[1] + size[1] / 2.0]);
    round([c[0] - size[0] / 2.0, c[1] - size[1] / 2.0])
}

/// Changes the place of items; wires (and labels, flags) attached to what
/// moves follow it when `drag`. Returns how many moved.
pub fn transform_items(tx: &mut Tx, targets: &[StableId], x: &Xf, drag: bool) -> Result<usize, CommandError> {
    let Some(first) = targets.first() else { return Ok(0) };
    let schematic = owner(tx.world(), *first).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{first} is not on a schematic")))?;
    let it = schematic::items(tx.world(), schematic);
    let mut moves: BTreeMap<String, Vec<(P, P)>> = BTreeMap::new();
    let mut n = 0;
    let turning = x.rotate != 0.0 || x.mirror.is_some();
    for id in targets {
        if owner(tx.world(), *id) != Some(schematic) {
            return Err(bad(format!("{id} is not on the same schematic")));
        }
        if let Some(s) = tx.get::<SchSymbol>(*id).cloned() {
            let (rotation, mirror) = x.symbol(s.rotation, s.mirror);
            let mut c = SchSymbol { at: snap(x.p(s.at)), rotation: check_rotation(rotation)?, mirror, ..s.clone() };
            if !turning {
                c.ref_at = s.ref_at.map(|p| x.p(p));
                c.value_at = s.value_at.map(|p| x.p(p));
            } else {
                c.ref_at = None;
                c.value_at = None;
            }
            let before = schematic::pin_points(&s);
            let after = schematic::pin_points(&c);
            moves.entry(s.sheet.clone()).or_default().extend(before.into_iter().zip(after));
            tx.set(*id, c)?;
        } else if let Some(w) = tx.get::<SchWire>(*id).cloned() {
            let pts: Vec<P> = w.points.iter().map(|p| snap(x.p(*p))).collect();
            let m = moves.entry(w.sheet.clone()).or_default();
            m.push((w.points[0], pts[0]));
            m.push((*w.points.last().unwrap(), *pts.last().unwrap()));
            tx.set(*id, SchWire { points: schematic::simplify(&pts), ..w })?;
        } else if let Some(w) = tx.get::<SchBus>(*id).cloned() {
            let pts: Vec<P> = w.points.iter().map(|p| snap(x.p(*p))).collect();
            tx.set(*id, SchBus { points: schematic::simplify(&pts), ..w })?;
        } else if let Some(l) = tx.get::<SchLabel>(*id).cloned() {
            let at = snap(x.p(l.at));
            moves.entry(l.sheet.clone()).or_default().push((l.at, at));
            tx.set(*id, SchLabel { at, rotation: x.direction(l.rotation), ..l })?;
        } else if let Some(j) = tx.get::<SchJunction>(*id).cloned() {
            let at = snap(x.p(j.at));
            moves.entry(j.sheet.clone()).or_default().push((j.at, at));
            tx.set(*id, SchJunction { at, ..j })?;
        } else if let Some(j) = tx.get::<SchNoConnect>(*id).cloned() {
            tx.set(*id, SchNoConnect { at: snap(x.p(j.at)), ..j })?;
        } else if let Some(t) = tx.get::<SchText>(*id).cloned() {
            let angle = if turning && (x.rotate == 90.0 || x.rotate == 270.0) { if t.angle == 0.0 { 90.0 } else { 0.0 } } else { t.angle };
            tx.set(*id, SchText { at: round(x.p(t.at)), angle, ..t })?;
        } else if let Some(e) = tx.get::<SchBusEntry>(*id).cloned() {
            let at = snap(x.p(e.at));
            let size = snap(x.v(e.size));
            moves.entry(e.sheet.clone()).or_default().push((e.wire_end(), [at[0] + size[0], at[1] + size[1]]));
            tx.set(*id, SchBusEntry { at, size, ..e })?;
        } else if let Some(s) = tx.get::<SchShape>(*id).cloned() {
            tx.set(*id, SchShape { points: s.points.iter().map(|p| round(x.p(*p))).collect(), ..s })?;
        } else if let Some(t) = tx.get::<SchTextBox>(*id).cloned() {
            tx.set(*id, SchTextBox { at: moved_box(x, t.at, t.size), ..t })?;
        } else if let Some(t) = tx.get::<SchTable>(*id).cloned() {
            let (w, h) = crate::schematic_graphics::table_layout(&t);
            let size = [w.iter().sum::<f64>(), h * t.cells.len() as f64];
            tx.set(*id, SchTable { at: moved_box(x, t.at, size), ..t })?;
        } else if let Some(t) = tx.get::<SchImage>(*id).cloned() {
            tx.set(*id, SchImage { at: moved_box(x, t.at, t.size()), ..t })?;
        } else {
            return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a schematic item")));
        }
        n += 1;
    }
    let _ = &it;
    if drag {
        for (sheet, m) in moves {
            schematic::drag_attached(tx, schematic, &sheet, &m, targets)?;
        }
    }
    Ok(n)
}

/// `pcb.schematic.move`: move, turn (`rotate` 90, 180, 270, counter-clockwise)
/// or mirror (`mirror` "x" flips left-right, "y" up-down) several items together,
/// about their middle or `about`. Wires attached to moved pins and labels follow
/// unless `drag: false`.
#[derive(Deserialize, JsonSchema)]
pub struct Move {
    pub targets: Vec<StableId>,
    pub delta: Option<[f64; 2]>,
    pub rotate: Option<f64>,
    pub mirror: Option<String>,
    pub about: Option<[f64; 2]>,
    #[serde(default = "yes")]
    pub drag: bool,
}

impl Command for Move {
    const NAME: &'static str = "pcb.schematic.move";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.targets.is_empty() {
            return Err(bad("nothing to move: give targets"));
        }
        let rotate = check_rotation(self.rotate.unwrap_or(0.0))?;
        let mirror = match self.mirror.as_deref().map(|m| m.trim().to_ascii_lowercase()) {
            None => None,
            Some(m) if m.is_empty() => None,
            Some(m) if m == "x" || m == "h" || m == "horizontal" => Some('x'),
            Some(m) if m == "y" || m == "v" || m == "vertical" => Some('y'),
            Some(m) => return Err(bad(format!("mirror is x (left-right) or y (up-down), not {m}"))),
        };
        let pivot = match self.about {
            Some(p) => snap(p),
            None => {
                let s = owner(tx.world(), self.targets[0]).ok_or_else(|| bad("not on a schematic"))?;
                let it = schematic::items(tx.world(), s);
                let pts: Vec<P> = self.targets.iter().flat_map(|t| anchor_points(&it, *t)).collect();
                if pts.is_empty() {
                    [0.0, 0.0]
                } else {
                    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[0]), a.1.max(p[0])));
                    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
                    snap([(x0 + x1) / 2.0, (y0 + y1) / 2.0])
                }
            }
        };
        let d = self.delta.unwrap_or([0.0, 0.0]);
        let x = Xf { delta: [snap1(d[0]), snap1(d[1])], rotate, mirror, pivot };
        let n = transform_items(tx, &self.targets, &x, self.drag)?;
        Ok(json!({ "moved": n }))
    }
}

/// `pcb.schematic.align`: line items up by an edge or centre (left, right, top,
/// bottom, center_x, center_y) or space them evenly (distribute_x, distribute_y).
#[derive(Deserialize, JsonSchema)]
pub struct Align {
    pub targets: Vec<StableId>,
    pub how: String,
}

impl Command for Align {
    const NAME: &'static str = "pcb.schematic.align";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        if self.targets.len() < 2 {
            return Err(bad("align needs two or more items"));
        }
        let s = owner(tx.world(), self.targets[0]).ok_or_else(|| bad("not on a schematic"))?;
        let v = schematic::resolve(tx.world(), s).map_err(bad)?;
        let it = schematic::items(tx.world(), s);
        // Each item's box: symbol bodies, label and graphic boxes, else its points.
        let boxes: Vec<(StableId, [f64; 4])> = self
            .targets
            .iter()
            .filter_map(|t| {
                let b = v.symbols.iter().find(|x| x.id == *t && x.instance.is_none()).map(|x| x.body).or_else(|| v.labels.iter().find(|x| x.id == *t).map(|x| [x.at[0], x.at[1], x.at[0], x.at[1]])).or_else(|| v.graphics.iter().find(|x| x.id == *t).map(|x| x.bbox)).or_else(|| {
                    let p = anchor_points(&it, *t);
                    if p.is_empty() {
                        return None;
                    }
                    Some(p.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, q| [a[0].min(q[0]), a[1].min(q[1]), a[2].max(q[0]), a[3].max(q[1])]))
                })?;
                Some((*t, b))
            })
            .collect();
        if boxes.len() < 2 {
            return Err(bad("align needs two or more schematic items"));
        }
        let cx = |b: &[f64; 4]| (b[0] + b[2]) / 2.0;
        let cy = |b: &[f64; 4]| (b[1] + b[3]) / 2.0;
        let mut deltas: Vec<(StableId, P)> = Vec::new();
        match self.how.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "left" => {
                let x = boxes.iter().map(|b| b.1[0]).fold(f64::MAX, f64::min);
                deltas = boxes.iter().map(|(i, b)| (*i, [x - b[0], 0.0])).collect();
            }
            "right" => {
                let x = boxes.iter().map(|b| b.1[2]).fold(f64::MIN, f64::max);
                deltas = boxes.iter().map(|(i, b)| (*i, [x - b[2], 0.0])).collect();
            }
            "top" => {
                let y = boxes.iter().map(|b| b.1[1]).fold(f64::MAX, f64::min);
                deltas = boxes.iter().map(|(i, b)| (*i, [0.0, y - b[1]])).collect();
            }
            "bottom" => {
                let y = boxes.iter().map(|b| b.1[3]).fold(f64::MIN, f64::max);
                deltas = boxes.iter().map(|(i, b)| (*i, [0.0, y - b[3]])).collect();
            }
            "center_x" | "centre_x" | "center" => {
                let x = boxes.iter().map(|b| cx(&b.1)).sum::<f64>() / boxes.len() as f64;
                deltas = boxes.iter().map(|(i, b)| (*i, [x - cx(b), 0.0])).collect();
            }
            "center_y" | "centre_y" | "middle" => {
                let y = boxes.iter().map(|b| cy(&b.1)).sum::<f64>() / boxes.len() as f64;
                deltas = boxes.iter().map(|(i, b)| (*i, [0.0, y - cy(b)])).collect();
            }
            "distribute_x" | "distribute_y" => {
                let horizontal = self.how.ends_with('x');
                let mut list = boxes.clone();
                list.sort_by(|a, b| if horizontal { cx(&a.1).total_cmp(&cx(&b.1)) } else { cy(&a.1).total_cmp(&cy(&b.1)) });
                let (a, b) = (list.first().unwrap().1, list.last().unwrap().1);
                let (c0, c1) = if horizontal { (cx(&a), cx(&b)) } else { (cy(&a), cy(&b)) };
                let n = list.len() as f64 - 1.0;
                for (k, (i, bx)) in list.iter().enumerate() {
                    let want = c0 + (c1 - c0) * k as f64 / n;
                    let have = if horizontal { cx(bx) } else { cy(bx) };
                    deltas.push((*i, if horizontal { [want - have, 0.0] } else { [0.0, want - have] }));
                }
            }
            other => return Err(bad(format!("how is left, right, top, bottom, center_x, center_y, distribute_x or distribute_y (not {other})"))),
        }
        let mut moved = 0;
        for (id, d) in deltas {
            let d = [snap1(d[0]), snap1(d[1])];
            if d[0].abs() < 1e-6 && d[1].abs() < 1e-6 {
                continue;
            }
            moved += transform_items(tx, &[id], &Xf::shift(d), true)?;
        }
        Ok(json!({ "moved": moved }))
    }
}

/// Steps the number at the end of a name ("D3" + 1 → "D4"; "A" + 1 → "B"); names without one stay.
pub fn increment_text(t: &str, by: i64) -> String {
    if by == 0 || t.is_empty() {
        return t.to_string();
    }
    let digits = t.len() - t.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 {
        let (p, n) = t.split_at(t.len() - digits);
        if let Ok(n) = n.parse::<i64>() {
            let v = (n + by).max(0);
            return format!("{p}{:0width$}", v, width = if n.to_string().len() < digits { digits } else { 0 });
        }
    }
    let last = t.chars().last().unwrap();
    if last.is_ascii_alphabetic() && t.len() == 1 {
        let base = if last.is_ascii_uppercase() { b'A' } else { b'a' };
        let k = ((last as u8 - base) as i64 + by).rem_euclid(26) as u8;
        return format!("{}", (base + k) as char);
    }
    t.to_string()
}

/// `pcb.schematic.duplicate`: copies of items moved by `delta` (default 5.08, 5.08),
/// onto `sheet` when given (paste). Copied symbols get the next free references (units
/// of one part stay together); with `increment`, label and note numbers step (D0 → D1).
#[derive(Deserialize, JsonSchema)]
pub struct Duplicate {
    pub targets: Vec<StableId>,
    pub delta: Option<[f64; 2]>,
    #[serde(default)]
    pub increment: i64,
    pub sheet: Option<String>,
}

impl Command for Duplicate {
    const NAME: &'static str = "pcb.schematic.duplicate";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let Some(first) = self.targets.first() else { return Err(bad("nothing to duplicate: give targets")) };
        let schematic = owner(tx.world(), *first).ok_or_else(|| bad(format!("{first} is not on a schematic")))?;
        let s = schematic_of(tx, schematic)?;
        let sheet = match &self.sheet {
            Some(x) => Some(sheet_or_first(&s, Some(x.clone()))?),
            None => None,
        };
        let d = self.delta.map(|d| [snap1(d[0]), snap1(d[1])]).unwrap_or([5.08, 5.08]);
        let mv = |p: P| snap([p[0] + d[0], p[1] + d[1]]);
        let mvr = |p: P| round([p[0] + d[0], p[1] + d[1]]);
        let on = |x: &String| sheet.clone().unwrap_or_else(|| x.clone());
        let mut it = schematic::items(tx.world(), schematic);
        let mut taken: Vec<String> = it.symbols.iter().map(|(_, x)| x.reference.clone()).collect();
        let mut new_refs: BTreeMap<(String, String), String> = BTreeMap::new();
        let mut ids = Vec::new();
        for id in &self.targets {
            if owner(tx.world(), *id) != Some(schematic) {
                return Err(bad(format!("{id} is not on the same schematic")));
            }
            let nid = if let Some(x) = tx.get::<SchSymbol>(*id).cloned() {
                let key = (x.reference.clone(), x.symbol.clone());
                let r = match new_refs.get(&key) {
                    Some(r) => r.clone(),
                    None => {
                        let r = if x.reference.starts_with('#') || x.reference.contains('?') {
                            let (p, _) = (x.reference.trim_end_matches(|c: char| c.is_ascii_digit() || c == '?').to_string(), ());
                            if x.reference.contains('?') { x.reference.clone() } else { next_reference(&it, &p, &taken) }
                        } else {
                            let p = x.reference.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
                            next_reference(&it, &p, &taken)
                        };
                        taken.push(r.clone());
                        new_refs.insert(key, r.clone());
                        r
                    }
                };
                let c = SchSymbol { sheet: on(&x.sheet), at: mv(x.at), reference: r, ref_at: x.ref_at.map(mvr), value_at: x.value_at.map(mvr), component: None, ..x };
                let nid = add_item(tx, schematic, c.clone())?;
                it.symbols.push((nid, c));
                nid
            } else if let Some(x) = tx.get::<SchWire>(*id).cloned() {
                add_item(tx, schematic, SchWire { sheet: on(&x.sheet), points: x.points.iter().map(|p| mv(*p)).collect(), ..x })?
            } else if let Some(x) = tx.get::<SchBus>(*id).cloned() {
                add_item(tx, schematic, SchBus { sheet: on(&x.sheet), points: x.points.iter().map(|p| mv(*p)).collect(), ..x })?
            } else if let Some(x) = tx.get::<SchLabel>(*id).cloned() {
                let text = if x.kind == LabelKind::Power || x.kind == LabelKind::Directive { x.text.clone() } else { increment_text(&x.text, self.increment) };
                add_item(tx, schematic, SchLabel { sheet: on(&x.sheet), at: mv(x.at), text, ..x })?
            } else if let Some(x) = tx.get::<SchJunction>(*id).cloned() {
                add_item(tx, schematic, SchJunction { sheet: on(&x.sheet), at: mv(x.at), ..x })?
            } else if let Some(x) = tx.get::<SchNoConnect>(*id).cloned() {
                add_item(tx, schematic, SchNoConnect { sheet: on(&x.sheet), at: mv(x.at), ..x })?
            } else if let Some(x) = tx.get::<SchText>(*id).cloned() {
                let text = increment_text(&x.text, self.increment);
                add_item(tx, schematic, SchText { sheet: on(&x.sheet), at: mvr(x.at), text, ..x })?
            } else if let Some(x) = tx.get::<SchBusEntry>(*id).cloned() {
                add_item(tx, schematic, SchBusEntry { sheet: on(&x.sheet), at: mv(x.at), ..x })?
            } else if let Some(x) = tx.get::<SchShape>(*id).cloned() {
                add_item(tx, schematic, SchShape { sheet: on(&x.sheet), points: x.points.iter().map(|p| mvr(*p)).collect(), ..x })?
            } else if let Some(x) = tx.get::<SchTextBox>(*id).cloned() {
                add_item(tx, schematic, SchTextBox { sheet: on(&x.sheet), at: mvr(x.at), ..x })?
            } else if let Some(x) = tx.get::<SchTable>(*id).cloned() {
                add_item(tx, schematic, SchTable { sheet: on(&x.sheet), at: mvr(x.at), ..x })?
            } else if let Some(x) = tx.get::<SchImage>(*id).cloned() {
                add_item(tx, schematic, SchImage { sheet: on(&x.sheet), at: mvr(x.at), ..x })?
            } else {
                return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a schematic item")));
            };
            ids.push(nid);
        }
        Ok(json!({ "ids": ids, "references": new_refs.into_iter().map(|((from, _), to)| json!({ "from": from, "to": to })).collect::<Vec<_>>() }))
    }
}

/// Replaces `find` in `s`: whole field with wildcards (`*`, `?`), else every
/// occurrence (whole words only with `whole_word`). None when nothing matches.
pub fn replace_in(s: &str, find: &str, with: &str, match_case: bool, whole_word: bool, wildcards: bool) -> Option<String> {
    if find.is_empty() {
        return None;
    }
    if wildcards && (find.contains('*') || find.contains('?')) {
        let ok = if match_case { glob_case(find, s) } else { schematic_view::glob(find, s) };
        return ok.then(|| with.to_string());
    }
    let (hay, needle) = if match_case { (s.to_string(), find.to_string()) } else { (s.to_lowercase(), find.to_lowercase()) };
    if hay.len() != s.len() {
        // Case folding changed byte lengths: fall back to an exact match.
        return if s == find { Some(with.to_string()) } else { None };
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::new();
    let mut i = 0;
    let mut hit = false;
    while let Some(k) = hay[i..].find(&needle) {
        let at = i + k;
        let end = at + needle.len();
        if whole_word && (word(s[..at].chars().last()) || word(s[end..].chars().next())) {
            out.push_str(&s[i..end]);
            i = end;
            continue;
        }
        out.push_str(&s[i..at]);
        out.push_str(with);
        i = end;
        hit = true;
    }
    out.push_str(&s[i..]);
    hit.then_some(out)
}

fn glob_case(p: &str, s: &str) -> bool {
    // Case-sensitive: the case-insensitive glob plus an exact check of the literal parts.
    schematic_view::glob(p, s) && p.split(['*', '?']).all(|lit| lit.is_empty() || s.contains(lit))
}

/// `pcb.schematic.replace`: find and replace text in references, values,
/// footprints, fields, labels and notes/text boxes/tables (`in`, default values,
/// fields, labels, texts). `dry_run` lists what would change.
#[derive(Deserialize, JsonSchema)]
pub struct Replace {
    pub schematic: StableId,
    pub find: String,
    #[serde(default)]
    pub replace: String,
    /// references, values, footprints, fields, labels, texts.
    #[serde(default, rename = "in")]
    pub scope: Vec<String>,
    #[serde(default)]
    pub match_case: bool,
    #[serde(default)]
    pub whole_word: bool,
    /// `*` and `?` match the whole text.
    #[serde(default)]
    pub wildcards: bool,
    /// Only these items.
    pub targets: Option<Vec<StableId>>,
    #[serde(default)]
    pub dry_run: bool,
}

impl Command for Replace {
    const NAME: &'static str = "pcb.schematic.replace";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        schematic_of(tx, self.schematic)?;
        let scope: Vec<String> = if self.scope.is_empty() { vec!["values".into(), "fields".into(), "labels".into(), "texts".into()] } else { self.scope.iter().map(|x| x.to_ascii_lowercase()).collect() };
        for x in &scope {
            if !["references", "values", "footprints", "fields", "labels", "texts"].contains(&x.as_str()) {
                return Err(bad(format!("`in` lists references, values, footprints, fields, labels, texts (not {x})")));
            }
        }
        let has = |k: &str| scope.iter().any(|x| x == k);
        let rep = |s: &str| replace_in(s, &self.find, &self.replace, self.match_case, self.whole_word, self.wildcards);
        let chosen = |id: &StableId| self.targets.as_ref().is_none_or(|t| t.contains(id));
        let it = schematic::items(tx.world(), self.schematic);
        let mut changes = Vec::new();
        let mut renames: Vec<(StableId, String)> = Vec::new();
        for (id, sym) in it.symbols.iter().filter(|(id, _)| chosen(id)) {
            let mut c = sym.clone();
            let mut any = false;
            if has("references") {
                if let Some(r) = rep(&c.reference) {
                    check_reference(&r)?;
                    changes.push(json!({ "id": id, "what": "reference", "from": c.reference, "to": r }));
                    renames.push((*id, r));
                }
            }
            if has("values") {
                if let Some(v) = rep(&c.value) {
                    changes.push(json!({ "id": id, "what": "value", "from": c.value, "to": v }));
                    c.value = v;
                    any = true;
                }
            }
            if has("footprints") {
                if let Some(v) = rep(&c.footprint) {
                    changes.push(json!({ "id": id, "what": "footprint", "from": c.footprint, "to": v }));
                    c.footprint = v;
                    any = true;
                }
            }
            if has("fields") {
                for (k, val) in c.fields.clone() {
                    if let Some(v) = rep(&val) {
                        changes.push(json!({ "id": id, "what": format!("field {k}"), "from": val, "to": v }));
                        c.fields.insert(k, v);
                        any = true;
                    }
                }
                for (name, f) in [("MPN", &mut c.mpn), ("Manufacturer", &mut c.manufacturer), ("LCSC", &mut c.lcsc)] {
                    if let Some(v) = f.as_deref().and_then(rep) {
                        changes.push(json!({ "id": id, "what": name, "from": f, "to": v }));
                        *f = Some(v);
                        any = true;
                    }
                }
            }
            if any && !self.dry_run {
                tx.set(*id, c)?;
            }
        }
        if has("labels") {
            for (id, l) in it.labels.iter().filter(|(id, _)| chosen(id)) {
                if let Some(v) = rep(&l.text).filter(|v| !v.trim().is_empty()) {
                    changes.push(json!({ "id": id, "what": "label", "from": l.text, "to": v }));
                    if !self.dry_run {
                        tx.set(*id, SchLabel { text: v.trim().to_string(), ..l.clone() })?;
                    }
                }
            }
        }
        if has("texts") {
            for (id, t) in it.texts.iter().filter(|(id, _)| chosen(id)) {
                if let Some(v) = rep(&t.text) {
                    changes.push(json!({ "id": id, "what": "note", "from": t.text, "to": v }));
                    if !self.dry_run {
                        tx.set(*id, SchText { text: v, ..t.clone() })?;
                    }
                }
            }
            for (id, t) in it.text_boxes.iter().filter(|(id, _)| chosen(id)) {
                if let Some(v) = rep(&t.text) {
                    changes.push(json!({ "id": id, "what": "text box", "from": t.text, "to": v }));
                    if !self.dry_run {
                        tx.set(*id, SchTextBox { text: v, ..t.clone() })?;
                    }
                }
            }
            for (id, t) in it.tables.iter().filter(|(id, _)| chosen(id)) {
                let mut c = t.clone();
                let mut any = false;
                for row in c.cells.iter_mut() {
                    for cell in row.iter_mut() {
                        if let Some(v) = rep(cell) {
                            changes.push(json!({ "id": id, "what": "table cell", "from": cell, "to": v }));
                            *cell = v;
                            any = true;
                        }
                    }
                }
                if any && !self.dry_run {
                    tx.set(*id, c)?;
                }
            }
        }
        if !self.dry_run {
            for (id, r) in renames {
                schematic::UpdateSymbol { reference: Some(r), ..schematic::UpdateSymbol::empty(id) }.apply(tx)?;
            }
        }
        Ok(json!({ "count": changes.len(), "changes": changes, "dry_run": self.dry_run }))
    }
}

/// One field edit in `pcb.schematic.fields`.
#[derive(Deserialize, JsonSchema)]
pub struct FieldEdit {
    /// The symbol (or give `reference`: every unit of that part).
    pub target: Option<StableId>,
    pub reference: Option<String>,
    /// For a symbol on a shared sheet: which instance's reference (Reference only).
    pub instance: Option<String>,
    /// Reference, Value, Footprint, MPN, Manufacturer, LCSC, Description, DNP, Exclude from board, or any custom field ("" removes a custom field).
    pub field: String,
    pub value: String,
}

/// `pcb.schematic.fields`: edit fields of many symbols at once (the field table),
/// rename or remove a custom field everywhere.
#[derive(Deserialize, JsonSchema)]
pub struct Fields {
    pub schematic: StableId,
    #[serde(default)]
    pub set: Vec<FieldEdit>,
    /// [from, to]: rename a custom field on every symbol (and its template).
    pub rename: Option<[String; 2]>,
    /// Remove a custom field from every symbol (and its template).
    pub remove: Option<String>,
}

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "yes" | "true" | "dnp" | "y" | "x")
}

impl Command for Fields {
    const NAME: &'static str = "pcb.schematic.fields";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = schematic_of(tx, self.schematic)?;
        let mut changed = 0;
        for e in self.set {
            let it = schematic::items(tx.world(), self.schematic);
            let ids: Vec<StableId> = match (&e.target, &e.reference) {
                (Some(t), _) => vec![*t],
                (None, Some(r)) => it.symbols.iter().filter(|(_, x)| x.reference.eq_ignore_ascii_case(r.trim())).map(|(id, _)| *id).collect(),
                _ => return Err(bad("each edit names a `target` or a `reference`")),
            };
            if ids.is_empty() {
                return Err(bad(format!("no symbol {}", e.reference.clone().unwrap_or_default())));
            }
            let f = e.field.trim();
            let val = e.value.trim().to_string();
            match f.to_ascii_lowercase().as_str() {
                "reference" | "ref" => {
                    schematic::UpdateSymbol { reference: Some(val.clone()), instance: e.instance.clone(), ..schematic::UpdateSymbol::empty(ids[0]) }.apply(tx)?;
                    changed += 1;
                    continue;
                }
                _ => {}
            }
            for id in ids {
                let mut c = tx.get::<SchSymbol>(id).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{id} is not a schematic symbol")))?;
                let opt = |v: &str| if v.is_empty() { None } else { Some(v.to_string()) };
                match f.to_ascii_lowercase().as_str() {
                    "value" => c.value = val.clone(),
                    "footprint" => c.footprint = val.clone(),
                    "mpn" => c.mpn = opt(&val),
                    "manufacturer" | "maker" => c.manufacturer = opt(&val),
                    "lcsc" => c.lcsc = opt(&val),
                    "description" => c.description = val.clone(),
                    "dnp" => c.dnp = truthy(&val),
                    "exclude from board" | "exclude_from_board" => c.exclude_from_board = truthy(&val),
                    "" => return Err(bad("a field needs a name")),
                    _ => {
                        if val.is_empty() {
                            c.fields.remove(f);
                        } else {
                            c.fields.insert(f.to_string(), val.clone());
                        }
                    }
                }
                tx.set(id, c)?;
                changed += 1;
            }
        }
        let it = schematic::items(tx.world(), self.schematic);
        if let Some([from, to]) = &self.rename {
            let (from, to) = (from.trim(), to.trim());
            if to.is_empty() {
                return Err(bad("rename a field to a name"));
            }
            for (id, x) in &it.symbols {
                if let Some(v) = x.fields.get(from).cloned() {
                    let mut c = x.clone();
                    c.fields.remove(from);
                    c.fields.insert(to.to_string(), v);
                    tx.set(*id, c)?;
                    changed += 1;
                }
            }
            for t in s.field_templates.iter_mut().filter(|t| t.name == from) {
                t.name = to.to_string();
            }
            tx.set(self.schematic, s.clone())?;
        }
        if let Some(name) = &self.remove {
            for (id, x) in &it.symbols {
                if x.fields.contains_key(name.trim()) {
                    let mut c = x.clone();
                    c.fields.remove(name.trim());
                    tx.set(*id, c)?;
                    changed += 1;
                }
            }
            s.field_templates.retain(|t: &FieldTemplate| t.name != name.trim());
            tx.set(self.schematic, s)?;
        }
        Ok(json!({ "changed": changed }))
    }
}

fn parse_pin_type(t: &str) -> Option<PinType> {
    let t = t.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    PIN_TYPES.iter().copied().find(|x| pin_type_name(*x) == t || (t == "tristate" && *x == PinType::TriState) || (t == "bidi" && *x == PinType::Bidirectional))
}

/// An exclusion: a key, or a key with a comment.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Exclude {
    Key(String),
    With { key: String, comment: Option<String> },
}

/// `pcb.schematic.erc`: electrical-rules settings: `severities` per rule code
/// (error, warning, ignore; "default" restores), `pin_matrix` per pin-type pair
/// ("output/open_collector": error, warning, ok, default), `exclude` violations
/// by key (from GET /pcb/schematic/<id> violations), `include` them again, `reset` all.
#[derive(Deserialize, JsonSchema)]
pub struct ErcConfig {
    pub schematic: StableId,
    pub severities: Option<BTreeMap<String, String>>,
    pub pin_matrix: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub exclude: Vec<Exclude>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub reset: bool,
}

impl Command for ErcConfig {
    const NAME: &'static str = "pcb.schematic.erc";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s: Schematic = schematic_of(tx, self.schematic)?;
        if self.reset {
            s.erc = Default::default();
        }
        for (code, level) in self.severities.unwrap_or_default() {
            let level = level.trim().to_ascii_lowercase();
            match level.as_str() {
                "default" | "" => {
                    s.erc.severities.remove(&code);
                }
                "error" | "warning" | "ignore" => {
                    s.erc.severities.insert(code.trim().to_string(), level);
                }
                _ => return Err(bad(format!("{code}: severity is error, warning, ignore or default"))),
            }
        }
        for (pair, level) in self.pin_matrix.unwrap_or_default() {
            let (a, b) = pair.split_once(['/', ':']).ok_or_else(|| bad(format!("“{pair}”: write a pin pair as output/open_collector")))?;
            let (Some(a), Some(b)) = (parse_pin_type(a), parse_pin_type(b)) else {
                return Err(bad(format!("“{pair}”: pin types are {}", PIN_TYPES.iter().map(|t| pin_type_name(*t)).collect::<Vec<_>>().join(", "))));
            };
            let k = pin_pair_key(a, b);
            let level = level.trim().to_ascii_lowercase();
            match level.as_str() {
                "default" | "" => {
                    s.erc.pin_matrix.remove(&k);
                }
                "error" | "warning" | "ok" => {
                    if schematic_view::default_pin_level(a, b) == level {
                        s.erc.pin_matrix.remove(&k);
                    } else {
                        s.erc.pin_matrix.insert(k, level);
                    }
                }
                _ => return Err(bad(format!("{pair}: level is error, warning, ok or default"))),
            }
        }
        for e in self.exclude {
            let (key, comment) = match e {
                Exclude::Key(k) => (k, String::new()),
                Exclude::With { key, comment } => (key, comment.unwrap_or_default()),
            };
            if key.trim().is_empty() {
                return Err(bad("an exclusion needs a violation key"));
            }
            s.erc.exclusions.retain(|x| x.key != key);
            s.erc.exclusions.push(schematic::ErcExclusion { key, comment });
        }
        let inc: BTreeSet<String> = self.include.into_iter().collect();
        s.erc.exclusions.retain(|x| !inc.contains(&x.key));
        tx.set(self.schematic, s.clone())?;
        Ok(json!({ "erc": s.erc }))
    }
}

/// `pcb.schematic.netclass`: add or change a net class (colour and wire width
/// on the sheet; track width, clearance and vias for the board; `patterns` of
/// net names it takes, `USB_*`), rename it, or `delete` it.
#[derive(Deserialize, JsonSchema)]
pub struct NetClassCommand {
    pub schematic: StableId,
    pub name: String,
    pub rename: Option<String>,
    /// #rrggbb ("" clears).
    pub color: Option<String>,
    pub wire_width: Option<f64>,
    pub track_width: Option<f64>,
    pub clearance: Option<f64>,
    pub via_diameter: Option<f64>,
    pub via_drill: Option<f64>,
    pub patterns: Option<Vec<String>>,
    /// Impedance targets for the board (Ω; 0 clears): single-ended and differential.
    pub impedance: Option<f64>,
    pub diff_impedance: Option<f64>,
    /// Fixed gap of the class's pairs (mm).
    pub diff_gap: Option<f64>,
    /// %, default 10.
    pub impedance_tolerance: Option<f64>,
    pub max_uncoupled: Option<f64>,
    pub max_skew: Option<f64>,
    #[serde(default)]
    pub delete: bool,
}

impl Command for NetClassCommand {
    const NAME: &'static str = "pcb.schematic.netclass";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = schematic_of(tx, self.schematic)?;
        let name = self.name.trim().to_string();
        if name.is_empty() || name.contains(['{', '}']) {
            return Err(bad("a net class needs a name (no braces)"));
        }
        let pos = s.netclasses.iter().position(|c| c.name.eq_ignore_ascii_case(&name));
        if self.delete {
            let i = pos.ok_or_else(|| bad(format!("no net class {name}")))?;
            s.netclasses.remove(i);
            tx.set(self.schematic, s)?;
            return Ok(json!({ "deleted": name }));
        }
        let mut c = pos.map(|i| s.netclasses[i].clone()).unwrap_or(SchNetClass { name: name.clone(), ..Default::default() });
        if let Some(r) = self.rename.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()) {
            if s.netclasses.iter().any(|x| x.name.eq_ignore_ascii_case(&r)) && !r.eq_ignore_ascii_case(&name) {
                return Err(bad(format!("there is a net class {r} already")));
            }
            c.name = r;
        }
        if self.color.is_some() {
            c.color = check_color(self.color)?;
        }
        let mm = |v: Option<f64>, what: &str, max: f64| -> Result<Option<f64>, CommandError> {
            match v {
                Some(x) if x <= 0.0 => Ok(None),
                Some(x) if x > max => Err(bad(format!("{what} is at most {max} mm"))),
                x => Ok(x),
            }
        };
        if self.wire_width.is_some() {
            c.wire_width = mm(self.wire_width, "wire width", 2.0)?;
        }
        if self.track_width.is_some() {
            c.track_width = mm(self.track_width, "track width", 10.0)?;
        }
        if self.clearance.is_some() {
            c.clearance = mm(self.clearance, "clearance", 10.0)?;
        }
        if self.via_diameter.is_some() {
            c.via_diameter = mm(self.via_diameter, "via diameter", 10.0)?;
        }
        if self.via_drill.is_some() {
            c.via_drill = mm(self.via_drill, "via drill", 10.0)?;
        }
        if let Some(p) = self.patterns {
            c.patterns = p.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        }
        for (slot, v, what, max) in [
            (&mut c.impedance, self.impedance, "impedance (Ω)", 500.0),
            (&mut c.diff_impedance, self.diff_impedance, "differential impedance (Ω)", 1000.0),
            (&mut c.diff_gap, self.diff_gap, "pair gap (mm)", 10.0),
            (&mut c.impedance_tolerance, self.impedance_tolerance, "impedance tolerance (%)", 50.0),
            (&mut c.max_uncoupled, self.max_uncoupled, "max uncoupled (mm)", 10000.0),
            (&mut c.max_skew, self.max_skew, "max skew (mm)", 1000.0),
        ] {
            if let Some(x) = v {
                if !x.is_finite() || x > max {
                    return Err(bad(format!("{what} is at most {max}")));
                }
                *slot = (x > 0.0).then_some(x);
            }
        }
        match pos {
            Some(i) => s.netclasses[i] = c.clone(),
            None => s.netclasses.push(c.clone()),
        }
        tx.set(self.schematic, s)?;
        Ok(json!({ "netclass": c }))
    }
}

/// `pcb.schematic.net_color`: colour one net's wires (#rrggbb; "" or null clears).
#[derive(Deserialize, JsonSchema)]
pub struct NetColor {
    pub schematic: StableId,
    pub net: String,
    pub color: Option<String>,
}

impl Command for NetColor {
    const NAME: &'static str = "pcb.schematic.net_color";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut s = schematic_of(tx, self.schematic)?;
        let net = self.net.trim().to_string();
        if net.is_empty() {
            return Err(bad("which net?"));
        }
        match check_color(self.color)? {
            Some(c) => {
                s.net_colors.insert(net.clone(), c);
            }
            None => {
                s.net_colors.remove(&net);
            }
        }
        tx.set(self.schematic, s)?;
        Ok(json!({ "net": net }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_turn_and_mirror_symbols_consistently() {
        // A symbol's local point placed, then the whole moved, matches placing with the new rotation and mirror.
        let local = [2.54, -1.27];
        for rot in [0.0, 90.0, 180.0, 270.0] {
            for m in [false, true] {
                for (r, mir) in [(90.0, None), (0.0, Some('x')), (0.0, Some('y')), (270.0, Some('x'))] {
                    let x = Xf { delta: [1.27, 0.0], rotate: r, mirror: mir, pivot: [10.16, 5.08] };
                    let at = [20.32, 12.7];
                    let placed = schematic::xf(at, rot, m, local);
                    let (nr, nm) = x.symbol(rot, m);
                    let again = schematic::xf(x.p(at), nr, nm, local);
                    let want = x.p(placed);
                    assert!((again[0] - want[0]).abs() < 1e-9 && (again[1] - want[1]).abs() < 1e-9, "rot {rot} mirror {m} by {r} {mir:?}: {again:?} vs {want:?}");
                }
            }
        }
    }

    #[test]
    fn text_steps_and_replaces() {
        assert_eq!(increment_text("D7", 1), "D8");
        assert_eq!(increment_text("ADC09", 1), "ADC10");
        assert_eq!(increment_text("A", 2), "C");
        assert_eq!(increment_text("SDA", 1), "SDA");
        assert_eq!(replace_in("10k 1%", "10k", "4k7", false, false, false).as_deref(), Some("4k7 1%"));
        assert_eq!(replace_in("SDA_1", "SDA", "X", false, true, false), None);
        assert_eq!(replace_in("SDA 1", "sda", "X", false, true, false).as_deref(), Some("X 1"));
        assert_eq!(replace_in("100nF", "*nF", "0.1uF", false, false, true).as_deref(), Some("0.1uF"));
        assert_eq!(replace_in("100nF", "NF", "x", true, false, false), None);
    }
}
