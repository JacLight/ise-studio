//! Assembly variants: one schematic and board, built several ways. A
//! variant (`pcb.variant`) names the parts it leaves off (do not populate),
//! the parts it fits that are normally left off, and per-part value, MPN,
//! maker, LCSC and description overrides. The board stays one board; the
//! BOM, the placement file and the fab package come out per variant.

use crate::board::{BoardView, ComponentView};
use crate::schematic;
use crate::{bad, natural};
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a variant changes on one part ("" or absent: as designed).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PartOverride {
    /// Another value; the design's MPN, maker and LCSC are dropped unless given here too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcsc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl PartOverride {
    fn is_empty(&self) -> bool {
        self.value.is_none() && self.mpn.is_none() && self.manufacturer.is_none() && self.lcsc.is_none() && self.description.is_none()
    }
}

/// `pcb.variant`: an assembly variant of a schematic's board.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Variant {
    pub schematic: StableId,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// References not fitted in this variant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dnp: Vec<String>,
    /// References fitted in this variant though marked DNP in the design.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fitted: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, PartOverride>,
}

impl Persist for Variant {
    const KIND: &'static str = "pcb.variant";
}

pub fn variants(world: &World, schematic: Option<StableId>) -> Vec<(StableId, Variant)> {
    let mut v: Vec<(StableId, Variant)> = world.resource::<IdIndex>().iter().filter_map(|(id, e)| world.get::<Variant>(e).filter(|x| schematic.is_none_or(|s| x.schematic == s)).map(|x| (id, x.clone()))).collect();
    v.sort_by(|a, b| natural(&a.1.name, &b.1.name));
    v
}

/// The variant of a board by name (through the schematic that drives it).
pub fn of_board(world: &World, board: StableId, name: &str) -> Option<Variant> {
    let (sid, _) = schematic::of_board(world, board)?;
    variants(world, Some(sid)).into_iter().find(|(_, v)| v.name.eq_ignore_ascii_case(name)).map(|(_, v)| v)
}

impl Variant {
    fn has(list: &[String], r: &str) -> bool {
        list.iter().any(|x| x.eq_ignore_ascii_case(r))
    }
    /// One component as this variant builds it.
    pub fn apply_to(&self, c: &mut ComponentView) {
        if Self::has(&self.dnp, &c.reference) {
            c.dnp = true;
        } else if Self::has(&self.fitted, &c.reference) {
            c.dnp = false;
        }
        if let Some(o) = self.overrides.iter().find(|(k, _)| k.eq_ignore_ascii_case(&c.reference)).map(|(_, o)| o) {
            if let Some(v) = &o.value {
                // Another value is another part: the design's part numbers no longer apply.
                if *v != c.value {
                    c.mpn = None;
                    c.manufacturer = None;
                    c.lcsc = None;
                }
                c.value = v.clone();
            }
            if let Some(v) = &o.mpn {
                c.mpn = Some(v.clone()).filter(|s| !s.is_empty());
            }
            if let Some(v) = &o.manufacturer {
                c.manufacturer = Some(v.clone()).filter(|s| !s.is_empty());
            }
            if let Some(v) = &o.lcsc {
                c.lcsc = Some(v.clone()).filter(|s| !s.is_empty());
            }
            if let Some(v) = &o.description {
                c.description = v.clone();
            }
        }
    }
    /// The board as this variant builds it (for its BOM, placement and package).
    pub fn apply(&self, view: &BoardView) -> BoardView {
        let mut v = view.clone();
        for c in &mut v.components {
            self.apply_to(c);
        }
        v
    }
}

fn check(tx: &Tx, v: &Variant, id: Option<StableId>) -> Result<(), CommandError> {
    let name = v.name.trim();
    if name.is_empty() || name.len() > 64 || name.contains('/') {
        return Err(bad("a variant needs a name (1 to 64 characters, no '/')"));
    }
    if variants(tx.world(), Some(v.schematic)).iter().any(|(oid, o)| Some(*oid) != id && o.name.eq_ignore_ascii_case(name)) {
        return Err(bad(format!("the schematic already has a variant “{name}”")));
    }
    let refs: Vec<String> = schematic::items(tx.world(), v.schematic).symbols.into_iter().map(|(_, s)| s.reference).collect();
    let known = |r: &String| refs.iter().any(|x| x.eq_ignore_ascii_case(r));
    for r in v.dnp.iter().chain(v.fitted.iter()).chain(v.overrides.keys()) {
        if !known(r) {
            return Err(bad(format!("no {r} on the schematic")));
        }
    }
    if let Some(r) = v.dnp.iter().find(|r| Variant::has(&v.fitted, r)) {
        return Err(bad(format!("{r} is both fitted and not fitted")));
    }
    Ok(())
}

/// `pcb.variant.create`: a new assembly variant (a copy of another with `copy_from`).
#[derive(Deserialize, JsonSchema)]
pub struct CreateVariant {
    pub schematic: StableId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A variant's name to start from.
    pub copy_from: Option<String>,
    #[serde(default)]
    pub dnp: Vec<String>,
    #[serde(default)]
    pub fitted: Vec<String>,
    #[serde(default)]
    pub overrides: BTreeMap<String, PartOverride>,
}

impl Command for CreateVariant {
    const NAME: &'static str = "pcb.variant.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        tx.get::<schematic::Schematic>(self.schematic).ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a schematic", self.schematic)))?;
        let mut v = match &self.copy_from {
            Some(n) => variants(tx.world(), Some(self.schematic)).into_iter().find(|(_, v)| v.name.eq_ignore_ascii_case(n)).map(|(_, v)| v).ok_or_else(|| bad(format!("no variant “{n}” to copy")))?,
            None => Variant { schematic: self.schematic, name: String::new(), description: String::new(), dnp: vec![], fitted: vec![], overrides: BTreeMap::new() },
        };
        v.name = self.name.trim().to_string();
        if !self.description.is_empty() || self.copy_from.is_none() {
            v.description = self.description;
        }
        v.dnp.extend(self.dnp);
        v.fitted.extend(self.fitted);
        v.overrides.extend(self.overrides);
        check(tx, &v, None)?;
        let id = tx.create();
        tx.set(id, v)?;
        tx.add_dependency(self.schematic, id)?;
        Ok(json!({ "id": id }))
    }
}

/// Per-part change in `pcb.variant.update`'s `parts`.
#[derive(Deserialize, JsonSchema, Default)]
pub struct PartChange {
    /// true: not fitted; false: fitted (overrides a design DNP); absent: as designed.
    pub dnp: Option<bool>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub mpn: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub lcsc: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Back to the design for this part.
    #[serde(default)]
    pub reset: bool,
}

/// `pcb.variant.update`: rename, describe, or change parts of a variant (what is left out stays).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateVariant {
    pub target: StableId,
    pub name: Option<String>,
    pub description: Option<String>,
    /// Replace the lists outright.
    pub dnp: Option<Vec<String>>,
    pub fitted: Option<Vec<String>>,
    pub overrides: Option<BTreeMap<String, PartOverride>>,
    /// Change parts one by one: { "R7": { dnp: true }, "U2": { value: "LM358B", mpn: "…" } } ("" in a field clears it).
    #[serde(default)]
    pub parts: BTreeMap<String, PartChange>,
}

impl Command for UpdateVariant {
    const NAME: &'static str = "pcb.variant.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut v = tx.get::<Variant>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a variant", self.target)))?;
        if let Some(n) = self.name {
            v.name = n.trim().to_string();
        }
        if let Some(d) = self.description {
            v.description = d;
        }
        if let Some(x) = self.dnp {
            v.dnp = x;
        }
        if let Some(x) = self.fitted {
            v.fitted = x;
        }
        if let Some(x) = self.overrides {
            v.overrides = x;
        }
        for (r, ch) in self.parts {
            v.dnp.retain(|x| !x.eq_ignore_ascii_case(&r));
            v.fitted.retain(|x| !x.eq_ignore_ascii_case(&r));
            if ch.reset {
                v.overrides.retain(|k, _| !k.eq_ignore_ascii_case(&r));
                continue;
            }
            match ch.dnp {
                Some(true) => v.dnp.push(r.clone()),
                Some(false) => v.fitted.push(r.clone()),
                None => {}
            }
            let o = v.overrides.entry(r.clone()).or_default();
            let set = |slot: &mut Option<String>, val: Option<String>| {
                if let Some(s) = val {
                    *slot = if s.trim().is_empty() { None } else { Some(s.trim().to_string()) };
                }
            };
            set(&mut o.value, ch.value);
            set(&mut o.mpn, ch.mpn);
            set(&mut o.manufacturer, ch.manufacturer);
            set(&mut o.lcsc, ch.lcsc);
            set(&mut o.description, ch.description);
            if o.is_empty() {
                v.overrides.remove(&r);
            }
        }
        v.dnp.sort_by(|a, b| natural(a, b));
        v.fitted.sort_by(|a, b| natural(a, b));
        check(tx, &v, Some(self.target))?;
        tx.set(self.target, v)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.variant.delete`.
#[derive(Deserialize, JsonSchema)]
pub struct DeleteVariant {
    pub target: StableId,
}

impl Command for DeleteVariant {
    const NAME: &'static str = "pcb.variant.delete";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let v = tx.get::<Variant>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a variant", self.target)))?;
        tx.remove_dependency(v.schematic, self.target)?;
        tx.delete(self.target)?;
        Ok(json!({ "deleted": v.name }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.component::<Variant>().command::<CreateVariant>().command::<UpdateVariant>().command::<DeleteVariant>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(r: &str, dnp: bool) -> ComponentView {
        ComponentView {
            id: StableId::new(),
            reference: r.into(),
            value: "10k".into(),
            footprint: "R_0603".into(),
            kind: "resistor".into(),
            at: [0.0, 0.0],
            rotation: 0.0,
            side: crate::Side::Top,
            mpn: None,
            manufacturer: None,
            lcsc: None,
            description: String::new(),
            dnp,
            height: 0.5,
            courtyard: vec![],
            silk: vec![],
            bodies: vec![],
            label: [0.0, 0.0],
            label_rot: 0.0,
            label_fixed: false,
            pins: BTreeMap::new(),
            pin_names: BTreeMap::new(),
            known: true,
        }
    }

    #[test]
    fn variant_fits_unfits_and_overrides() {
        let mut o = BTreeMap::new();
        o.insert("R2".to_string(), PartOverride { value: Some("4k7".into()), lcsc: Some("C23162".into()), ..Default::default() });
        let v = Variant { schematic: StableId::new(), name: "Lite".into(), description: String::new(), dnp: vec!["R1".into()], fitted: vec!["R3".into()], overrides: o };
        let mut a = comp("R1", false);
        let mut b = comp("r2", false);
        let mut c = comp("R3", true);
        v.apply_to(&mut a);
        v.apply_to(&mut b);
        v.apply_to(&mut c);
        assert!(a.dnp);
        assert_eq!(b.value, "4k7");
        assert_eq!(b.lcsc.as_deref(), Some("C23162"));
        assert!(!c.dnp);
    }
}
