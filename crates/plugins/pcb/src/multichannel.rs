//! Multichannel layout: lay out and route one channel (an amplifier, an LED
//! driver, one of eight identical inputs), then repeat it onto the others —
//! every matching component placed the same way relative to its channel's
//! anchor, and the channel's tracks and vias copied with their nets mapped
//! through the matching pads. Placement rule areas can hold each channel.

use crate::geom::{self, P};
use crate::{PcbComponent, bad, board_of};
use de_core::{Command, CommandError, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

/// One channel to repeat onto: its components, matched to the source's by
/// footprint (then value) in reference order, or an explicit map source → target.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Channel {
    List(Vec<String>),
    Map(BTreeMap<String, String>),
}

fn yes() -> bool {
    true
}

/// `pcb.channel.repeat`: copy the placement (and routing) of the `source`
/// channel onto each of `channels`. Each channel's anchor (the component
/// matched to the first source reference) stays where it is and sets the
/// channel's angle, unless `step` [dx, dy] places channel k at the source
/// plus (k+1)·step. `routing` copies tracks and vias lying within the source
/// channel (replacing the target channel's on the mapped nets); `areas` adds a
/// placement rule area round every channel.
#[derive(Deserialize, JsonSchema)]
pub struct Repeat {
    pub board: StableId,
    pub source: Vec<String>,
    pub channels: Vec<Channel>,
    pub step: Option<[f64; 2]>,
    #[serde(default = "yes")]
    pub routing: bool,
    #[serde(default)]
    pub areas: bool,
}

fn rect(b: [f64; 4]) -> Vec<P> {
    vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]
}

impl Command for Repeat {
    const NAME: &'static str = "pcb.channel.repeat";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let _ = board_of(tx, self.board)?;
        let it = crate::items(tx.world(), self.board);
        let by_ref: BTreeMap<String, (StableId, PcbComponent)> = it.components.iter().map(|(id, c)| (c.reference.to_ascii_uppercase(), (*id, c.clone()))).collect();
        let get = |r: &str| by_ref.get(&r.trim().to_ascii_uppercase()).cloned().ok_or_else(|| bad(format!("no component {r} on the board")));
        let src: Vec<(StableId, PcbComponent)> = self.source.iter().map(|r| get(r)).collect::<Result<_, _>>()?;
        if src.is_empty() {
            return Err(bad("name the source channel's components"));
        }
        let anchor = src[0].1.clone();
        let view = crate::view(tx.world(), self.board);
        // The source channel's extent: its courtyards, a little grown.
        let mut region = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for (id, c) in &src {
            let pts: Vec<P> = view.as_ref().and_then(|v| v.components.iter().find(|x| x.id == *id).map(|x| x.courtyard.clone())).filter(|v| !v.is_empty()).unwrap_or_else(|| vec![c.at]);
            for p in pts {
                region = [region[0].min(p[0]), region[1].min(p[1]), region[2].max(p[0]), region[3].max(p[1])];
            }
        }
        region = [region[0] - 0.5, region[1] - 0.5, region[2] + 0.5, region[3] + 0.5];
        let inside = |p: P| p[0] >= region[0] && p[0] <= region[2] && p[1] >= region[1] && p[1] <= region[3];
        let src_tracks: Vec<crate::Track> = it.tracks.iter().filter(|(_, t)| t.points.iter().all(|p| inside(*p))).map(|(_, t)| t.clone()).collect();
        let src_vias: Vec<crate::Via> = it.vias.iter().filter(|(_, v)| inside(v.at)).map(|(_, v)| v.clone()).collect();
        let mut report = Vec::new();
        let mut areas_made = 0;
        if self.areas {
            tx.run("pcb.area.create", json!({ "board": self.board, "name": "Channel 1", "outline": rect(region), "placement": self.source }))?;
            areas_made += 1;
        }
        for (k, ch) in self.channels.iter().enumerate() {
            // Match each source component to one of the channel's.
            let mut pairs: Vec<((StableId, PcbComponent), (StableId, PcbComponent))> = Vec::new();
            match ch {
                Channel::Map(m) => {
                    for (s, t) in m {
                        let sc = src.iter().find(|(_, c)| c.reference.eq_ignore_ascii_case(s)).cloned().ok_or_else(|| bad(format!("{s} is not in the source channel")))?;
                        pairs.push((sc, get(t)?));
                    }
                }
                Channel::List(refs) => {
                    let mut pool: Vec<(StableId, PcbComponent)> = refs.iter().map(|r| get(r)).collect::<Result<_, _>>()?;
                    pool.sort_by(|a, b| crate::natural(&a.1.reference, &b.1.reference));
                    for s in &src {
                        let pick = pool.iter().position(|(_, c)| c.footprint == s.1.footprint && c.value == s.1.value).or_else(|| pool.iter().position(|(_, c)| c.footprint == s.1.footprint));
                        match pick {
                            Some(i) => pairs.push((s.clone(), pool.remove(i))),
                            None => return Err(bad(format!("channel {}: nothing matches {} ({})", k + 2, s.1.reference, s.1.footprint))),
                        }
                    }
                }
            }
            let Some(t_anchor) = pairs.iter().find(|(s, _)| s.0 == src[0].0).map(|(_, t)| t.1.clone()) else {
                return Err(bad(format!("channel {}: no match for the anchor {}", k + 2, anchor.reference)));
            };
            let (t_at, t_rot) = match self.step {
                Some(s) => ([anchor.at[0] + s[0] * (k + 1) as f64, anchor.at[1] + s[1] * (k + 1) as f64], anchor.rotation),
                None => (t_anchor.at, t_anchor.rotation),
            };
            let d_rot = t_rot - anchor.rotation;
            let f = |p: P| geom::add(t_at, geom::rotate(geom::sub(p, anchor.at), d_rot));
            let mut net_map: BTreeMap<String, String> = BTreeMap::new();
            for ((_, s), (tid, t)) in &pairs {
                let at = f(s.at);
                tx.run("pcb.component.update", json!({ "target": tid, "at": at, "rotation": (s.rotation + d_rot).rem_euclid(360.0), "side": s.side, "drag_tracks": false }))?;
                for (pad, net) in &s.pins {
                    if let Some(tn) = t.pins.get(pad) {
                        net_map.entry(net.clone()).or_insert_with(|| tn.clone());
                    }
                }
            }
            let mut copied = (0, 0, 0);
            if self.routing {
                let corners: Vec<P> = rect(region).into_iter().map(f).collect();
                let tb = corners.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
                let in_t = |p: P| p[0] >= tb[0] - 1e-6 && p[0] <= tb[2] + 1e-6 && p[1] >= tb[1] - 1e-6 && p[1] <= tb[3] + 1e-6;
                let targets: Vec<&String> = net_map.values().collect();
                // The target channel's old copper on its nets goes.
                let now = crate::items(tx.world(), self.board);
                for (id, t) in &now.tracks {
                    if t.net.as_ref().is_some_and(|n| targets.contains(&n)) && t.points.iter().all(|p| in_t(*p)) {
                        tx.remove_dependency(self.board, *id)?;
                        tx.delete(*id)?;
                        copied.2 += 1;
                    }
                }
                for (id, v) in &now.vias {
                    if v.net.as_ref().is_some_and(|n| targets.contains(&n)) && in_t(v.at) {
                        tx.remove_dependency(self.board, *id)?;
                        tx.delete(*id)?;
                        copied.2 += 1;
                    }
                }
                let map_net = |n: &Option<String>| n.as_ref().map(|n| net_map.get(n).cloned().unwrap_or_else(|| n.clone()));
                for t in &src_tracks {
                    let mut c = t.clone();
                    c.points = c.points.iter().map(|p| f(*p)).collect();
                    c.net = map_net(&t.net);
                    c.auto = false;
                    c.reroute = false;
                    let id = tx.create();
                    tx.set(id, c)?;
                    tx.add_dependency(self.board, id)?;
                    copied.0 += 1;
                }
                for v in &src_vias {
                    let mut c = v.clone();
                    c.at = f(v.at);
                    c.net = map_net(&v.net);
                    c.auto = false;
                    let id = tx.create();
                    tx.set(id, c)?;
                    tx.add_dependency(self.board, id)?;
                    copied.1 += 1;
                }
                if self.areas {
                    let refs: Vec<String> = pairs.iter().map(|(_, t)| t.1.reference.clone()).collect();
                    tx.run("pcb.area.create", json!({ "board": self.board, "name": format!("Channel {}", k + 2), "outline": rect(tb), "placement": refs }))?;
                    areas_made += 1;
                }
            }
            report.push(json!({ "channel": k + 2, "components": pairs.len(), "tracks": copied.0, "vias": copied.1, "replaced": copied.2, "nets": net_map }));
        }
        crate::model3d::sync(tx, self.board)?;
        Ok(json!({ "board": self.board, "channels": report, "areas": areas_made }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<Repeat>();
}
