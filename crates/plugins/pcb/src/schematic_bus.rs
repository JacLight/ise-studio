//! Buses: a bus is a thick polyline that carries several nets at once, named
//! by a bus label on it. Wires leave it through 45° bus entries and take the
//! member they carry from their own net label.
//!
//! Bus names (as KiCad writes them):
//!
//! - vector: `D[0..7]` carries D0 … D7 (`A[15..0]` counts down);
//! - group: `{SDA SCL}` carries SDA and SCL; `I2C{SDA SCL}` carries
//!   I2C.SDA and I2C.SCL; members may be vectors (`{D[0..3] CLK}`);
//! - alias: a name defined on the schematic (`USB` → DP, DM, VBUS), used
//!   alone or inside a group (`{USB}`, `PORT1{USB}`).
//!
//! A bus's members are nets on its sheet (or everywhere when a global label
//! names it). Bus labels join buses as net labels join wires; a sheet pin and
//! the hierarchical label it stands for join their buses member by member, in
//! order, so `DATA[0..7]` on the parent can feed `D[0..7]` inside.

use crate::schematic::{self, LabelKind, SchLabel, SchWire, add_item, check_xy, schematic_of, snap};
use crate::bad;
use de_core::{
    Command, CommandError, ErrorCode, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `pcb.sch_bus`: a bus.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchBus {
    pub schematic: StableId,
    pub sheet: String,
    pub points: Vec<[f64; 2]>,
}

impl Persist for SchBus {
    const KIND: &'static str = "pcb.sch_bus";
}

/// `pcb.sch_bus_entry`: a 45° entry from a bus (`at`, on the bus) to a wire end (`at + size`).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchBusEntry {
    pub schematic: StableId,
    pub sheet: String,
    pub at: [f64; 2],
    #[serde(default = "entry_size")]
    pub size: [f64; 2],
}

fn entry_size() -> [f64; 2] {
    [2.54, 2.54]
}

impl Persist for SchBusEntry {
    const KIND: &'static str = "pcb.sch_bus_entry";
}

impl SchBusEntry {
    /// Where the wire joins it.
    pub fn wire_end(&self) -> [f64; 2] {
        [self.at[0] + self.size[0], self.at[1] + self.size[1]]
    }
}

// ---- Names ----------------------------------------------------------------------

/// The members a bus name carries, or None when the text is not a bus name.
pub fn members(text: &str, aliases: &BTreeMap<String, Vec<String>>) -> Option<Vec<String>> {
    expand(text.trim(), aliases, 0)
}

pub fn is_bus_name(text: &str, aliases: &BTreeMap<String, Vec<String>>) -> bool {
    members(text, aliases).is_some()
}

fn alias<'a>(name: &str, aliases: &'a BTreeMap<String, Vec<String>>) -> Option<&'a Vec<String>> {
    aliases.get(name).or_else(|| aliases.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))
}

fn expand(t: &str, aliases: &BTreeMap<String, Vec<String>>, depth: usize) -> Option<Vec<String>> {
    if depth > 6 || t.is_empty() {
        return None;
    }
    if let Some(open) = t.find('{') {
        if !t.ends_with('}') {
            return None;
        }
        let prefix = t[..open].trim();
        let inner = &t[open + 1..t.len() - 1];
        let mut out = Vec::new();
        for tok in split_group(inner) {
            let list = expand(&tok, aliases, depth + 1).unwrap_or_else(|| vec![tok.clone()]);
            for m in list {
                out.push(if prefix.is_empty() { m } else { format!("{prefix}.{m}") });
            }
        }
        return if out.is_empty() { None } else { Some(out) };
    }
    if let Some(v) = vector(t) {
        return Some(v);
    }
    let a = alias(t, aliases)?;
    let mut out = Vec::new();
    for m in a {
        out.extend(expand(m, aliases, depth + 1).unwrap_or_else(|| vec![m.clone()]));
    }
    Some(out)
}

/// `D[0..7]` → D0 … D7.
fn vector(t: &str) -> Option<Vec<String>> {
    if !t.ends_with(']') {
        return None;
    }
    let open = t.rfind('[')?;
    let prefix = &t[..open];
    let (a, b) = t[open + 1..t.len() - 1].split_once("..")?;
    let (a, b): (i64, i64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    if prefix.is_empty() || prefix.contains(char::is_whitespace) || (a - b).abs() > 1023 {
        return None;
    }
    let range: Vec<i64> = if a <= b { (a..=b).collect() } else { (b..=a).rev().collect() };
    Some(range.into_iter().map(|n| format!("{prefix}{n}")).collect())
}

/// Splits a group's members on spaces and commas, keeping `[…]` and `{…}` whole.
fn split_group(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            c if (c.is_whitespace() || c == ',') && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ---- Commands -------------------------------------------------------------------

/// `pcb.schematic.bus`: a bus through `points` (snapped, orthogonal or 45°),
/// optionally named at once by a bus label (`label: "D[0..7]"`) on its first point.
#[derive(Deserialize, JsonSchema)]
pub struct CreateBus {
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub points: Vec<[f64; 2]>,
    /// A bus name: D[0..7], {SDA SCL}, I2C{SDA SCL}, or a bus alias.
    pub label: Option<String>,
    /// local (default), global or hierarchical, for `label`.
    pub kind: Option<LabelKind>,
}

impl Command for CreateBus {
    const NAME: &'static str = "pcb.schematic.bus";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let sheet = schematic::sheet_or_first(&s, self.sheet)?;
        for p in &self.points {
            check_xy(*p, "bus")?;
        }
        let points = schematic::simplify(&self.points.iter().map(|p| snap(*p)).collect::<Vec<_>>());
        if points.len() < 2 {
            return Err(bad("a bus needs two different points"));
        }
        let id = add_item(tx, self.schematic, SchBus { schematic: self.schematic, sheet: sheet.clone(), points: points.clone() })?;
        let mut label = None;
        if let Some(t) = self.label.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            if !is_bus_name(&t, &s.bus_aliases) {
                return Err(bad(format!("“{t}” is not a bus name: D[0..7], {{SDA SCL}}, I2C{{SDA SCL}} or a bus alias")));
            }
            let kind = self.kind.unwrap_or(LabelKind::Local);
            let horizontal = (points[0][1] - points[1][1]).abs() < 1e-6;
            let rotation = if horizontal { if points[1][0] >= points[0][0] { 0.0 } else { 180.0 } } else { 90.0 };
            label = Some(add_item(tx, self.schematic, SchLabel { schematic: self.schematic, sheet: sheet.clone(), kind, text: t, at: points[0], rotation, shape: None, style: None })?);
        }
        Ok(json!({ "id": id, "points": points, "label": label }))
    }
}

/// `pcb.schematic.bus_entry`: a bus entry with its bus end at `at` (on a bus)
/// and its wire end at `at + size` (default [2.54, 2.54]: down and right).
#[derive(Deserialize, JsonSchema)]
pub struct CreateBusEntry {
    pub schematic: StableId,
    pub sheet: Option<String>,
    pub at: [f64; 2],
    pub size: Option<[f64; 2]>,
}

fn check_size(size: [f64; 2]) -> Result<[f64; 2], CommandError> {
    let s = snap(size);
    if s[0].abs() < 1e-6 || s[1].abs() < 1e-6 || s[0].abs() > 12.7 || s[1].abs() > 12.7 {
        return Err(bad("a bus entry's size is [±dx, ±dy], both non-zero (usually 2.54)"));
    }
    Ok(s)
}

impl Command for CreateBusEntry {
    const NAME: &'static str = "pcb.schematic.bus_entry";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let s = schematic_of(tx, self.schematic)?;
        let sheet = schematic::sheet_or_first(&s, self.sheet)?;
        check_xy(self.at, "at")?;
        let size = check_size(self.size.unwrap_or_else(entry_size))?;
        let id = add_item(tx, self.schematic, SchBusEntry { schematic: self.schematic, sheet, at: snap(self.at), size })?;
        Ok(json!({ "id": id }))
    }
}

/// `pcb.schematic.bus.unfold`: break members out of a bus: for each, a bus
/// entry, a short wire and a net label naming it, in a row along the bus.
#[derive(Deserialize, JsonSchema)]
pub struct Unfold {
    pub bus: StableId,
    /// Which members (default: every member not yet broken out on the sheet).
    pub members: Option<Vec<String>>,
    /// Which way the wires leave: right or left off an upright bus (default right), down or up off a level one (default down).
    pub side: Option<String>,
    /// Where on the bus to start (default: its first point).
    pub start: Option<[f64; 2]>,
    /// Wire length past the entry (mm, default 5.08).
    pub length: Option<f64>,
}

impl Command for Unfold {
    const NAME: &'static str = "pcb.schematic.bus.unfold";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let bus = tx.get::<SchBus>(self.bus).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a bus", self.bus)))?;
        let v = schematic::resolve(tx.world(), bus.schematic).map_err(bad)?;
        let bv = v.buses.iter().find(|b| b.id == self.bus && b.sheet == bus.sheet).ok_or_else(|| bad("bus not found in the schematic"))?;
        let name = bv.name.clone().ok_or_else(|| bad("the bus has no name yet: put a bus label on it (D[0..7], {SDA SCL})"))?;
        let all: Vec<String> = bv.members.iter().map(|m| m.member.clone()).collect();
        // Already broken out: members named on a wire leaving this bus through an entry.
        let on_bus = |p: [f64; 2]| bus.points.windows(2).any(|w| crate::schematic_view::on_segment(p, w[0], w[1]));
        let wires: Vec<&crate::schematic_view::WireView> = v
            .bus_entries
            .iter()
            .filter(|e| e.sheet == bus.sheet && on_bus(e.at))
            .filter_map(|e| v.wires.iter().find(|w| w.sheet == bus.sheet && (schematic::same(w.points[0], e.end) || schematic::same(*w.points.last().unwrap(), e.end))))
            .collect();
        let taken: Vec<String> = v.labels.iter().filter(|l| l.sheet == bus.sheet && wires.iter().any(|w| w.points.windows(2).any(|s| crate::schematic_view::on_segment(l.at, s[0], s[1])))).map(|l| l.text.clone()).collect();
        let want: Vec<String> = match self.members {
            Some(m) => {
                for x in &m {
                    if !all.contains(x) {
                        return Err(bad(format!("{x} is not a member of {name} ({})", all.join(", "))));
                    }
                }
                m
            }
            None => all.iter().filter(|m| !taken.contains(m)).cloned().collect(),
        };
        if want.is_empty() {
            return Ok(json!({ "entries": [], "note": "every member is already broken out" }));
        }
        // The longest straight run of the bus.
        let seg = bus.points.windows(2).filter(|w| (w[0][0] - w[1][0]).abs() < 1e-6 || (w[0][1] - w[1][1]).abs() < 1e-6).max_by(|a, b| {
            let la = (a[0][0] - a[1][0]).abs() + (a[0][1] - a[1][1]).abs();
            let lb = (b[0][0] - b[1][0]).abs() + (b[0][1] - b[1][1]).abs();
            la.total_cmp(&lb)
        });
        let Some(seg) = seg else { return Err(bad("the bus has no straight run to break out along")) };
        let (a, b) = (seg[0], seg[1]);
        let upright = (a[0] - b[0]).abs() < 1e-6;
        let side = self.side.as_deref().unwrap_or(if upright { "right" } else { "down" });
        let (perp, rotation): ([f64; 2], f64) = match (upright, side) {
            (true, "left") => ([-1.0, 0.0], 180.0),
            (true, _) => ([1.0, 0.0], 0.0),
            (false, "up") => ([0.0, -1.0], 90.0),
            (false, _) => ([0.0, 1.0], 270.0),
        };
        let along: [f64; 2] = if upright { [0.0, if b[1] >= a[1] { 1.0 } else { -1.0 }] } else { [if b[0] >= a[0] { 1.0 } else { -1.0 }, 0.0] };
        let g = 2.54;
        let len = self.length.unwrap_or(5.08).max(0.0);
        let start = self.start.map(snap).unwrap_or([a[0] + along[0] * g, a[1] + along[1] * g]);
        let it = schematic::items(tx.world(), bus.schematic);
        let used: Vec<[f64; 2]> = it.bus_entries.iter().filter(|(_, e)| e.sheet == bus.sheet).map(|(_, e)| e.at).collect();
        let mut t = 0.0;
        let mut out = Vec::new();
        for m in want {
            // Next free spot along the run.
            let at = loop {
                let p = snap([start[0] + along[0] * t, start[1] + along[1] * t]);
                t += g;
                if !used.iter().any(|u| schematic::same(*u, p)) && !out.iter().any(|(q, _): &([f64; 2], String)| schematic::same(*q, p)) {
                    break p;
                }
                if t > 2000.0 {
                    return Err(bad("no room along the bus"));
                }
            };
            out.push((at, m));
        }
        let mut ids = Vec::new();
        for (at, m) in out {
            let size = [perp[0] * g + along[0] * g, perp[1] * g + along[1] * g];
            let e = add_item(tx, bus.schematic, SchBusEntry { schematic: bus.schematic, sheet: bus.sheet.clone(), at, size })?;
            let w0 = [at[0] + size[0], at[1] + size[1]];
            let w1 = snap([w0[0] + perp[0] * len, w0[1] + perp[1] * len]);
            let mut wire = None;
            if len > 1e-6 {
                wire = Some(add_item(tx, bus.schematic, SchWire { schematic: bus.schematic, sheet: bus.sheet.clone(), points: vec![w0, w1] })?);
            }
            let l = add_item(tx, bus.schematic, SchLabel { schematic: bus.schematic, sheet: bus.sheet.clone(), kind: LabelKind::Local, text: m.clone(), at: w1, rotation, shape: None, style: None })?;
            ids.push(json!({ "member": m, "entry": e, "wire": wire, "label": l }));
        }
        Ok(json!({ "bus": name, "entries": ids }))
    }
}

/// `pcb.schematic.item.update` for buses and bus entries (false when `id` is neither).
pub(crate) fn update(tx: &mut Tx, id: StableId, at: Option<[f64; 2]>, points: Option<Vec<[f64; 2]>>, delta: Option<[f64; 2]>, size: Option<[f64; 2]>) -> Result<bool, CommandError> {
    let shift = |p: [f64; 2]| delta.map(|d| snap([p[0] + d[0], p[1] + d[1]])).unwrap_or(p);
    if let Some(mut b) = tx.get::<SchBus>(id).cloned() {
        if let Some(p) = points {
            for q in &p {
                check_xy(*q, "bus")?;
            }
            b.points = schematic::simplify(&p.iter().map(|q| snap(*q)).collect::<Vec<_>>());
        }
        b.points = b.points.iter().map(|p| shift(*p)).collect();
        if b.points.len() < 2 {
            return Err(bad("a bus needs two different points"));
        }
        tx.set(id, b)?;
        return Ok(true);
    }
    if let Some(mut e) = tx.get::<SchBusEntry>(id).cloned() {
        if let Some(a) = at {
            check_xy(a, "at")?;
            e.at = snap(a);
        }
        e.at = shift(e.at);
        if let Some(s) = size {
            e.size = check_size(s)?;
        }
        tx.set(id, e)?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_names_expand() {
        let mut al = BTreeMap::new();
        al.insert("USB".to_string(), vec!["DP".to_string(), "DM".to_string(), "VBUS".to_string()]);
        assert_eq!(members("D[0..3]", &al).unwrap(), vec!["D0", "D1", "D2", "D3"]);
        assert_eq!(members("A[2..0]", &al).unwrap(), vec!["A2", "A1", "A0"]);
        assert_eq!(members("{SDA SCL}", &al).unwrap(), vec!["SDA", "SCL"]);
        assert_eq!(members("I2C{SDA, SCL}", &al).unwrap(), vec!["I2C.SDA", "I2C.SCL"]);
        assert_eq!(members("{D[0..1] CLK}", &al).unwrap(), vec!["D0", "D1", "CLK"]);
        assert_eq!(members("USB", &al).unwrap(), vec!["DP", "DM", "VBUS"]);
        assert_eq!(members("P1{USB}", &al).unwrap(), vec!["P1.DP", "P1.DM", "P1.VBUS"]);
        assert!(members("SDA", &al).is_none());
        assert!(members("[0..3]", &al).is_none());
        assert!(members("D[x..3]", &al).is_none());
    }
}
