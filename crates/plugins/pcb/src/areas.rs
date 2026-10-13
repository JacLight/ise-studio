//! Rule areas and zone extras.
//!
//! A *rule area* (`pcb.area`) is an outline on some copper layers that keeps
//! things out — tracks, vias, pads, pours, footprints (a keepout under an
//! antenna, around a mounting hole) — and that the custom rules language
//! names in conditions (`A.insideArea('HV')`). A *placement* rule area holds
//! a group of components (a multichannel channel): the check warns when one
//! of them leaves it.
//!
//! Zones gain hatched fills, cutouts, an island area limit, a name, priority
//! moves (raise, lower, top, bottom) and duplication onto another layer.

use crate::geom::{self, P};
use crate::{Zone, bad, board_of};
use de_core::{Command, CommandError, ErrorCode, StableId, Tx, Value, bevy_ecs::{self, component::Component}, json, schemars};
use manifold_csg::{CrossSection, JoinType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A hatched (grid) zone fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hatch {
    /// Copper line width (mm).
    pub width: f64,
    /// Opening between lines (mm).
    pub gap: f64,
    /// Grid angle (degrees).
    #[serde(default)]
    pub angle: f64,
    /// Solid border kept round the edge (mm); default the line width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<f64>,
}

pub fn check_zone_extras(z: &Zone) -> Result<(), String> {
    if let Some(h) = &z.hatch {
        if !(h.width.is_finite() && h.width > 0.0 && h.width <= 10.0 && h.gap.is_finite() && h.gap > 0.0 && h.gap <= 50.0) {
            return Err("a hatch needs a line width (0 to 10 mm) and a gap (0 to 50 mm)".into());
        }
        if h.border.is_some_and(|b| !(b.is_finite() && (0.0..=10.0).contains(&b))) {
            return Err("a hatch border is 0 to 10 mm".into());
        }
    }
    for c in &z.cutouts {
        if c.len() < 3 {
            return Err("a zone cutout needs 3 or more points".into());
        }
        if c.iter().flatten().any(|v| !v.is_finite()) {
            return Err("zone cutout coordinates are numbers".into());
        }
    }
    if z.min_island_area.is_some_and(|a| !(a.is_finite() && a >= 0.0)) {
        return Err("min_island_area is 0 or more (mm²)".into());
    }
    Ok(())
}

/// The hatch pattern applied to a filled shape: grid lines inside it plus a solid border.
pub fn hatch(shape: &CrossSection, h: &Hatch) -> CrossSection {
    if shape.is_empty() {
        return shape.clone();
    }
    let b = shape.bounds_rect2();
    let c = [(b.min_x + b.max_x) / 2.0, (b.min_y + b.max_y) / 2.0];
    let reach = ((b.max_x - b.min_x).powi(2) + (b.max_y - b.min_y).powi(2)).sqrt() / 2.0 + h.width + h.gap;
    let pitch = h.width + h.gap;
    let n = (reach / pitch).ceil() as i64;
    let mut lines = Vec::new();
    for dir in [0.0, 90.0] {
        for i in -n..=n {
            let o = i as f64 * pitch;
            let a = geom::add(c, geom::rotate([-reach, o], h.angle + dir));
            let bb = geom::add(c, geom::rotate([reach, o], h.angle + dir));
            let d = geom::rotate([0.0, h.width / 2.0], h.angle + dir);
            lines.push(CrossSection::from_polygons(&[vec![geom::sub(a, d), geom::sub(bb, d), geom::add(bb, d), geom::add(a, d)]]));
        }
    }
    let grid = CrossSection::batch_union(&lines);
    let border = h.border.unwrap_or(h.width);
    let ring = if border > 0.0 { shape.difference(&shape.offset(-border, JoinType::Miter, 2.0, 8)) } else { CrossSection::empty() };
    shape.intersection(&grid).union(&ring)
}

/// A rule area / keepout on a board.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuleArea {
    pub board: StableId,
    /// Name the custom rules refer to (`insideArea('name')`).
    pub name: String,
    pub outline: Vec<[f64; 2]>,
    /// Copper layers it applies to; empty: every copper layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
    #[serde(default)]
    pub no_tracks: bool,
    #[serde(default)]
    pub no_vias: bool,
    #[serde(default)]
    pub no_pads: bool,
    #[serde(default)]
    pub no_pours: bool,
    #[serde(default)]
    pub no_footprints: bool,
    /// Components that belong inside (a placement rule area, e.g. one channel).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placement: Vec<String>,
}

impl de_core::Persist for RuleArea {
    const KIND: &'static str = "pcb.area";
}

#[derive(Clone, Debug, Serialize)]
pub struct AreaView {
    pub id: StableId,
    pub name: String,
    pub outline: Vec<P>,
    pub layers: Vec<String>,
    /// Bit k: copper layer k.
    #[serde(skip)]
    pub mask: u32,
    pub no_tracks: bool,
    pub no_vias: bool,
    pub no_pads: bool,
    pub no_pours: bool,
    pub no_footprints: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub placement: Vec<String>,
}

impl AreaView {
    pub fn keepout(&self) -> bool {
        self.no_tracks || self.no_vias || self.no_pads || self.no_pours || self.no_footprints
    }
}

pub fn view(id: StableId, a: &RuleArea, layers: &[String]) -> AreaView {
    let names: Vec<String> = if a.layers.is_empty() { layers.to_vec() } else { a.layers.iter().filter(|l| layers.contains(l)).cloned().collect() };
    let mut mask = 0u32;
    for l in &names {
        if let Some(k) = crate::board::layer_index(layers, l) {
            mask |= 1 << k;
        }
    }
    let mut outline = a.outline.clone();
    if geom::polygon_area(&outline) < 0.0 {
        outline.reverse();
    }
    AreaView { id, name: a.name.clone(), outline, layers: names, mask, no_tracks: a.no_tracks, no_vias: a.no_vias, no_pads: a.no_pads, no_pours: a.no_pours, no_footprints: a.no_footprints, placement: a.placement.clone() }
}

fn check_area(board: &crate::Board, a: &mut RuleArea) -> Result<(), CommandError> {
    if a.outline.len() < 3 {
        return Err(bad("a rule area needs an outline of 3 or more points"));
    }
    crate::check_points(&a.outline, "rule area")?;
    a.layers = a.layers.iter().map(|l| crate::graphics::normalize_layer(l)).collect();
    for l in &a.layers {
        crate::check_layer(board, l)?;
    }
    if a.name.trim().is_empty() {
        return Err(bad("a rule area needs a name"));
    }
    Ok(())
}

/// `pcb.area.create`: a rule area. With no `no_*` flag given it is a keepout
/// for tracks, vias and pours (KiCad's default); `placement` lists the
/// components that belong inside it.
#[derive(Deserialize, JsonSchema)]
pub struct CreateArea {
    pub board: StableId,
    pub name: Option<String>,
    pub outline: Vec<[f64; 2]>,
    #[serde(default)]
    pub layers: Vec<String>,
    pub no_tracks: Option<bool>,
    pub no_vias: Option<bool>,
    pub no_pads: Option<bool>,
    pub no_pours: Option<bool>,
    pub no_footprints: Option<bool>,
    #[serde(default)]
    pub placement: Vec<String>,
}

impl Command for CreateArea {
    const NAME: &'static str = "pcb.area.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let board = board_of(tx, self.board)?;
        let any = [self.no_tracks, self.no_vias, self.no_pads, self.no_pours, self.no_footprints].iter().any(|f| f.is_some());
        let default = !any && self.placement.is_empty();
        let n = crate::items(tx.world(), self.board).areas.len();
        let mut a = RuleArea {
            board: self.board,
            name: crate::clean(self.name).unwrap_or_else(|| format!("Area {}", n + 1)),
            outline: self.outline,
            layers: self.layers,
            no_tracks: self.no_tracks.unwrap_or(default),
            no_vias: self.no_vias.unwrap_or(default),
            no_pads: self.no_pads.unwrap_or(false),
            no_pours: self.no_pours.unwrap_or(default),
            no_footprints: self.no_footprints.unwrap_or(false),
            placement: self.placement,
        };
        check_area(&board, &mut a)?;
        let id = tx.create();
        let name = a.name.clone();
        tx.set(id, a)?;
        tx.add_dependency(self.board, id)?;
        Ok(json!({ "id": id, "name": name }))
    }
}

/// `pcb.area.update`: change a rule area (what is left out stays).
#[derive(Deserialize, JsonSchema)]
pub struct UpdateArea {
    pub target: StableId,
    pub name: Option<String>,
    pub outline: Option<Vec<[f64; 2]>>,
    pub layers: Option<Vec<String>>,
    pub no_tracks: Option<bool>,
    pub no_vias: Option<bool>,
    pub no_pads: Option<bool>,
    pub no_pours: Option<bool>,
    pub no_footprints: Option<bool>,
    pub placement: Option<Vec<String>>,
}

impl Command for UpdateArea {
    const NAME: &'static str = "pcb.area.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut a = tx.get::<RuleArea>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a rule area", self.target)))?;
        let board = board_of(tx, a.board)?;
        if let Some(v) = self.name {
            a.name = v.trim().to_string();
        }
        if let Some(v) = self.outline {
            a.outline = v;
        }
        if let Some(v) = self.layers {
            a.layers = v;
        }
        for (f, v) in [(&mut a.no_tracks, self.no_tracks), (&mut a.no_vias, self.no_vias), (&mut a.no_pads, self.no_pads), (&mut a.no_pours, self.no_pours), (&mut a.no_footprints, self.no_footprints)] {
            if let Some(v) = v {
                *f = v;
            }
        }
        if let Some(v) = self.placement {
            a.placement = v;
        }
        check_area(&board, &mut a)?;
        tx.set(self.target, a)?;
        Ok(json!({ "id": self.target }))
    }
}

/// `pcb.zone.priority`: move a zone's priority among the board's zones on its
/// layer: "raise" (above the next), "lower", "top" or "bottom".
#[derive(Deserialize, JsonSchema)]
pub struct ZonePriority {
    pub target: StableId,
    pub to: String,
}

impl Command for ZonePriority {
    const NAME: &'static str = "pcb.zone.priority";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut z = tx.get::<Zone>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a zone", self.target)))?;
        let others: Vec<i32> = crate::items(tx.world(), z.board).zones.iter().filter(|(id, o)| *id != self.target && o.layer == z.layer).map(|(_, o)| o.priority).collect();
        let max = others.iter().copied().max();
        let min = others.iter().copied().min();
        z.priority = match self.to.as_str() {
            "top" => max.map_or(z.priority, |m| m.max(z.priority - 1) + 1),
            "bottom" => min.map_or(z.priority, |m| m.min(z.priority + 1) - 1),
            "raise" => others.iter().copied().filter(|p| *p >= z.priority).min().map_or(z.priority, |p| p + 1),
            "lower" => others.iter().copied().filter(|p| *p <= z.priority).max().map_or(z.priority, |p| p - 1),
            s => return Err(bad(format!("to is raise, lower, top or bottom, not “{s}”"))),
        };
        let p = z.priority;
        tx.set(self.target, z)?;
        Ok(json!({ "id": self.target, "priority": p }))
    }
}

/// `pcb.zone.duplicate`: the same zone on another layer (or the same, for a
/// variant to edit); a new outline may be given.
#[derive(Deserialize, JsonSchema)]
pub struct DuplicateZone {
    pub target: StableId,
    pub layer: Option<String>,
    pub net: Option<String>,
    pub outline: Option<Vec<[f64; 2]>>,
}

impl Command for DuplicateZone {
    const NAME: &'static str = "pcb.zone.duplicate";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut z = tx.get::<Zone>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a zone", self.target)))?;
        let board = board_of(tx, z.board)?;
        if let Some(l) = self.layer {
            crate::check_layer(&board, &l)?;
            z.layer = l;
        }
        if let Some(n) = crate::clean(self.net) {
            z.net = n;
        }
        if let Some(o) = self.outline {
            crate::check_points(&o, "zone")?;
            z.outline = o;
        }
        let b = z.board;
        let id = tx.create();
        tx.set(id, z)?;
        tx.add_dependency(b, id)?;
        Ok(json!({ "id": id }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.component::<RuleArea>().command::<CreateArea>().command::<UpdateArea>().command::<ZonePriority>().command::<DuplicateZone>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hatch_keeps_a_border_and_opens_the_middle() {
        let sq = CrossSection::from_polygons(&[vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]]);
        let h = hatch(&sq, &Hatch { width: 0.5, gap: 1.0, angle: 0.0, border: None });
        let a = h.area();
        assert!(a > 20.0 && a < 80.0, "hatched area {a}");
        // The middle of an opening is empty, the border solid.
        let contours = geom::contours(&h);
        assert!(geom::inside_area([0.2, 5.0], &contours));
    }
}
