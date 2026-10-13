//! Differential pairs routed as one (`pcb.route.pair`) and dragged as one
//! (`pcb.pair.drag`).
//!
//! The pair's centreline is laid by the router (walkaround or shove) as a
//! single line as wide as the pair (two tracks and the gap), with the pair's
//! own pads and copper merged into one net so they never block it; each
//! track is that centreline offset half a pitch to its side (corners mitred,
//! or arcs whose radii differ by the offset), so the gap holds everywhere.
//! A change of layer puts two coupled vias side by side across the line,
//! `pair_via_pitch` apart. Short necks join the pads to the coupled run,
//! narrower where full width would crowd the partner's pad. The autorouter
//! routes pairs this way first, where they fit.

use crate::board::{BoardView, PadView, layer_index};
use crate::geom::{self, P};
use crate::meander::{self, Step};
use crate::pns::{self, Angle, Mode, Node, Pusher, Ref};
use crate::routing::{self, Corners, Plan, PlanRun};
use crate::signal::{self, PairView};
use crate::{Board, bad};
use de_core::{Command, CommandError, StableId, Tx, Value, json, schemars};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::f64::consts::PI;

/// A point the pair's centreline goes through; a new layer there puts coupled vias.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PairStep {
    pub at: [f64; 2],
    pub layer: Option<String>,
}

/// `pcb.route.pair`: route a differential pair's two tracks together, pad
/// pair to pad pair (`from`/`to`: the positive pad then the negative; found
/// when each net has two pads), through `steps`, keeping the gap; coupled
/// vias where a step changes layer; widths and gap from the net class's
/// impedance targets per layer unless given.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct RoutePair {
    pub board: StableId,
    /// The pair by name, or either of its nets.
    pub pair: String,
    pub from: Option<[String; 2]>,
    pub to: Option<[String; 2]>,
    #[serde(default)]
    pub steps: Vec<PairStep>,
    /// Starting layer (an SMD pad's own otherwise, else top).
    pub layer: Option<String>,
    pub mode: Option<Mode>,
    pub angle: Option<Angle>,
    pub width: Option<f64>,
    pub gap: Option<f64>,
    /// Centre-to-centre of the coupled vias.
    pub via_pitch: Option<f64>,
    pub corners: Option<Corners>,
    pub arc_radius: Option<f64>,
    pub optimize: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PairPlan {
    pub ok: bool,
    pub reached: bool,
    pub mode: Mode,
    pub pair: String,
    pub width: f64,
    pub gap: f64,
    /// The centreline, for drawing.
    pub centre: Vec<P>,
    pub p: Plan,
    pub n: Plan,
    pub length_p: f64,
    pub length_n: f64,
    pub skew: f64,
    pub notes: Vec<String>,
}

fn pads_of<'a>(v: &'a BoardView, net: &str) -> Vec<&'a PadView> {
    v.pads.iter().filter(|p| p.net.as_deref() == Some(net) && !p.layers.is_empty()).collect()
}

fn pad_named<'a>(v: &'a BoardView, name: &str) -> Result<&'a PadView, String> {
    v.pads.iter().find(|p| format!("{}.{}", p.reference, p.number) == name && !p.layers.is_empty()).ok_or_else(|| format!("no pad {name} on this board (reference.pad, e.g. J1.A6)"))
}

pub fn find<'a>(v: &'a BoardView, name: &str) -> Result<&'a PairView, String> {
    v.signal.pairs.iter().find(|p| p.name.eq_ignore_ascii_case(name) || p.p == name || p.n == name).ok_or_else(|| {
        format!("no differential pair {name}{}", if v.signal.pairs.is_empty() { " (none on the board: name nets D+/D-, _P/_N, or set one with pcb.pair.set)".to_string() } else { format!(" (pairs: {})", v.signal.pairs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")) })
    })
}

fn mid(a: P, b: P) -> P {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

fn heading(a: P, b: P) -> f64 {
    (b[1] - a[1]).atan2(b[0] - a[0])
}

fn left(dir: f64) -> P {
    [-dir.sin(), dir.cos()]
}

fn wrap(a: f64) -> f64 {
    let mut a = a;
    while a > PI {
        a -= 2.0 * PI;
    }
    while a <= -PI {
        a += 2.0 * PI;
    }
    a
}

/// A polyline as runs and turns (corners rounded to `radius` where the runs leave room).
pub fn steps_of(pts: &[P], radius: f64) -> (Vec<Step>, f64) {
    let n = pts.len();
    if n < 2 {
        return (vec![], 0.0);
    }
    let lens: Vec<f64> = pts.windows(2).map(|w| geom::dist(w[0], w[1])).collect();
    let dirs: Vec<f64> = pts.windows(2).map(|w| heading(w[0], w[1])).collect();
    let turns: Vec<f64> = (1..n - 1).map(|i| wrap(dirs[i] - dirs[i - 1])).collect();
    let tan: Vec<f64> = turns.iter().map(|a| (a.abs() / 2.0).tan()).collect();
    let r: Vec<f64> = (0..turns.len())
        .map(|i| {
            if radius <= 0.0 || tan[i] < 1e-9 {
                return 0.0;
            }
            let room_a = if i == 0 { lens[0] } else { lens[i] / 2.0 };
            let room_b = if i + 1 == lens.len() - 1 { lens[i + 1] } else { lens[i + 1] / 2.0 };
            radius.min(room_a / tan[i]).min(room_b / tan[i])
        })
        .collect();
    let mut out = Vec::new();
    for i in 0..lens.len() {
        let before = if i > 0 { r[i - 1] * tan[i - 1] } else { 0.0 };
        let after = if i < turns.len() { r[i] * tan[i] } else { 0.0 };
        out.push(Step::Line((lens[i] - before - after).max(0.0)));
        if i < turns.len() {
            out.push(Step::Turn { r: r[i], a: turns[i] });
        }
    }
    (out, dirs[0])
}

/// The two pads at each end: (P start, N start, P end, N end).
fn ends<'a>(v: &'a BoardView, p: &PairView, r: &RoutePair) -> Result<[&'a PadView; 4], String> {
    let named = |x: &Option<[String; 2]>| -> Result<Option<[&'a PadView; 2]>, String> {
        match x {
            Some([a, b]) => {
                let (pa, pb) = (pad_named(v, a)?, pad_named(v, b)?);
                if pa.net.as_deref() != Some(&p.p) || pb.net.as_deref() != Some(&p.n) {
                    return Err(format!("{a} and {b} are not on {} and {} (positive pad first)", p.p, p.n));
                }
                Ok(Some([pa, pb]))
            }
            None => Ok(None),
        }
    };
    let (f, t) = (named(&r.from)?, named(&r.to)?);
    if let (Some(f), Some(t)) = (f, t) {
        return Ok([f[0], f[1], t[0], t[1]]);
    }
    let pp = pads_of(v, &p.p);
    let nn = pads_of(v, &p.n);
    let pick_other = |a: &PadView, list: &[&'a PadView]| -> Option<&'a PadView> { list.iter().filter(|x| x.component != a.component || list.len() == 1).copied().min_by(|x, y| geom::dist(x.at, a.at).total_cmp(&geom::dist(y.at, a.at))) };
    if let Some(f) = f {
        let (op, on) = (pp.iter().find(|x| x.component != f[0].component).copied(), nn.iter().find(|x| x.component != f[1].component).copied());
        return match (op, on) {
            (Some(a), Some(b)) => Ok([f[0], f[1], a, b]),
            _ => Err("give `to`: the far positive and negative pads".into()),
        };
    }
    if let Some(t) = t {
        let (op, on) = (pp.iter().find(|x| x.component != t[0].component).copied(), nn.iter().find(|x| x.component != t[1].component).copied());
        return match (op, on) {
            (Some(a), Some(b)) => Ok([a, b, t[0], t[1]]),
            _ => Err("give `from`: the near positive and negative pads".into()),
        };
    }
    // Each net's pads, by part: the pair runs from one part to another.
    let parts: Vec<StableId> = pp.iter().map(|x| x.component).collect::<BTreeSet<_>>().into_iter().collect();
    if parts.len() != 2 {
        return Err(format!("{} has pads on {} parts: say which pads to join (from, to)", p.p, parts.len()));
    }
    let side = |c: StableId| -> Option<(&'a PadView, &'a PadView)> {
        let a = pp.iter().find(|x| x.component == c).copied()?;
        let b = nn.iter().filter(|x| x.component == c).copied().min_by(|x, y| geom::dist(x.at, a.at).total_cmp(&geom::dist(y.at, a.at)))?;
        Some((a, b))
    };
    let _ = pick_other;
    match (side(parts[0]), side(parts[1])) {
        (Some((a, b)), Some((c, d))) => Ok([a, b, c, d]),
        _ => Err(format!("{} and {} do not both reach the same two parts: say which pads to join (from, to)", p.p, p.n)),
    }
}

pub fn plan(b: &Board, v: &BoardView, r: &RoutePair) -> Result<PairPlan, String> {
    let p = find(v, &r.pair)?.clone();
    let [ps, ns, pe, ne] = ends(v, &p, r)?;
    let class = b.class_of(Some(&p.p));
    let mode = r.mode.unwrap_or_default();
    let angle = r.angle.unwrap_or_default();
    let layer_of = |pad: &PadView| if pad.layers.len() == 1 { Some(pad.layers[0].clone()) } else { None };
    let start_layer = r.layer.clone().or_else(|| layer_of(ps)).unwrap_or_else(|| v.layers[0].clone());
    let lidx = |l: &str| layer_index(&v.layers, l).ok_or_else(|| format!("layer “{l}” is not on this board (use {})", v.layers.join(", ")));
    let mut layer = lidx(&start_layer)?;
    let geo = |k: usize| -> (f64, f64) {
        let name = &v.layers[k];
        let w = r.width.unwrap_or_else(|| signal::width_on(b, v, Some(&p.p), name));
        let g = r.gap.or_else(|| signal::gap_on(b, v, &p.p, name)).unwrap_or(class.clearance.max(b.rules.min_space));
        (w, g)
    };
    for (x, what) in [(r.width, "width"), (r.gap, "gap"), (r.via_pitch, "via pitch")] {
        if x.is_some_and(|x| !(x.is_finite() && x > 0.0 && x <= 10.0)) {
            return Err(format!("{what} is above 0 and at most 10 mm"));
        }
    }
    let via_d = class.via_diameter;
    let via_drill = class.via_drill;
    let pitch = r.via_pitch.or(class.pair_via_pitch).unwrap_or(via_d + class.clearance.max(b.rules.min_space));
    let (w0, g0) = geo(layer);
    let mut out = PairPlan { ok: true, reached: true, mode, pair: p.name.clone(), width: w0, gap: g0, ..Default::default() };

    // The router's board with the pair as one net.
    let syn = Some(format!("\u{1}pair {}", p.name));
    let mut node = Node::new(b, v);
    let ours = |n: &Option<String>| n.as_deref() == Some(p.p.as_str()) || n.as_deref() == Some(p.n.as_str());
    let saved_fixed: Vec<Option<String>> = node.fixed.iter().map(|f| f.net.clone()).collect();
    let saved_lines: Vec<Option<String>> = node.lines.iter().map(|l| l.net.clone()).collect();
    let saved_vias: Vec<Option<String>> = node.vias.iter().map(|l| l.net.clone()).collect();
    for f in node.fixed.iter_mut().filter(|f| ours(&f.net)) {
        f.net = syn.clone();
    }
    for l in node.lines.iter_mut().filter(|l| ours(&l.net)) {
        l.net = syn.clone();
    }
    for x in node.vias.iter_mut().filter(|x| ours(&x.net)) {
        x.net = syn.clone();
    }
    node.set_clearance(syn.as_deref().unwrap(), class.clearance);
    let (lines0, vias0) = (node.lines.len(), node.vias.len());

    let start = mid(ps.at, ns.at);
    let end = mid(pe.at, ne.at);
    // Escapes: the pair leaves each pad pair square to the line through its pads, clear of the pad row.
    let escape = |a: &PadView, b: &PadView, toward: P| -> P {
        let c = mid(a.at, b.at);
        let d = geom::sub(b.at, a.at);
        let l = geom::len(d).max(1e-9);
        let mut n = [-d[1] / l, d[0] / l];
        if geom::dot(n, geom::sub(toward, c)) < 0.0 {
            n = [-n[0], -n[1]];
        }
        let e = a.size[0].max(a.size[1]).max(b.size[0].max(b.size[1])) / 2.0 + w0;
        geom::add(c, [n[0] * e, n[1] * e])
    };
    let first_target = r.steps.first().map(|s| s.at).unwrap_or(end);
    let last_from = r.steps.last().map(|s| s.at).unwrap_or(start);
    let start_esc = escape(ps, ns, first_target);
    let end_esc = escape(pe, ne, last_from);
    let mut targets: Vec<(P, usize)> = vec![(start_esc, layer)];
    {
        let mut l = layer;
        for s in &r.steps {
            if let Some(x) = &s.layer {
                l = lidx(x)?;
            }
            if !(s.at[0].is_finite() && s.at[1].is_finite()) {
                return Err("a step needs finite coordinates".into());
            }
            targets.push((s.at, l));
        }
        targets.push((end_esc, l));
        targets.push((end, l));
    }
    let mut legs: Vec<(usize, Vec<P>)> = Vec::new();
    let mut via_at: Vec<P> = Vec::new();
    let mut cur = start;
    for (to, l) in targets.iter() {
        if *l != layer {
            via_at.push(cur);
            layer = *l;
            // Out of the coupled vias straight on (they sit square to the incoming line), then on to the target.
            let dir = legs.last().and_then(|(_, p)| (p.len() >= 2).then(|| heading(p[p.len() - 2], p[p.len() - 1])));
            if let Some(d) = dir {
                let (w, g) = geo(layer);
                let stub = geom::add(cur, [d.cos() * pitch.max(via_d), d.sin() * pitch.max(via_d)]);
                node.lines.push(pns::Line { id: None, net: syn.clone(), layer, width: 2.0 * w + g, points: vec![cur, stub], locked: true, arcs: false, moved: false });
                legs.push((layer, vec![cur, stub]));
                cur = stub;
            }
        }
        let (w, g) = geo(layer);
        let env = 2.0 * w + g;
        let laid = node.lay(&pns::LegSpec { from: cur, to: *to, layer, width: env, net: &syn, mode, angle, first: false, optimize: r.optimize.unwrap_or(true) });
        if laid.mode != mode {
            out.mode = laid.mode;
        }
        if let Some(n) = laid.note {
            out.notes.push(n);
        }
        cur = *laid.points.last().unwrap_or(&cur);
        node.lines.push(pns::Line { id: None, net: syn.clone(), layer, width: env, points: laid.points.clone(), locked: true, arcs: false, moved: false });
        legs.push((layer, laid.points));
        out.reached = laid.reached;
    }
    // Centreline runs, one per layer stretch.
    let mut runs: Vec<(usize, Vec<P>)> = Vec::new();
    for (l, pts) in legs {
        match runs.last_mut() {
            Some((rl, run)) if *rl == l => run.extend(pts.into_iter().skip(1)),
            _ => runs.push((l, pts)),
        }
    }
    for run in &mut runs {
        run.1 = pns::clean(&run.1);
    }
    out.centre = runs.iter().flat_map(|r| r.1.clone()).collect();
    // Which side each track takes: the positive pad's side of the first leg.
    let first_dir = runs.iter().find(|r| r.1.len() >= 2).map(|r| heading(r.1[0], r.1[1])).unwrap_or(0.0);
    let sigma = if geom::cross([first_dir.cos(), first_dir.sin()], geom::sub(ps.at, start)) >= 0.0 { 1.0 } else { -1.0 };
    if let Some(last) = runs.last().filter(|r| r.1.len() >= 2) {
        let n = last.1.len();
        let d = heading(last.1[n - 2], last.1[n - 1]);
        let s_end = if geom::cross([d.cos(), d.sin()], geom::sub(pe.at, end)) >= 0.0 { 1.0 } else { -1.0 };
        if s_end != sigma && out.reached {
            out.notes.push(format!("{} and {} swap sides between the ends: the tracks would cross (swap the pins, or route with a crossover)", p.p, p.n));
        }
    }
    let radius_for = |w: f64| match r.corners {
        Some(Corners::Arc) => r.arc_radius.unwrap_or(3.0 * w),
        _ => 0.0,
    };
    let neck = class.neck_width.unwrap_or(w0 * 0.6).max(b.rules.min_track);
    let mut p_runs: Vec<PlanRun> = Vec::new();
    let mut n_runs: Vec<PlanRun> = Vec::new();
    let (mut p_vias, mut n_vias) = (Vec::new(), Vec::new());
    let nr = runs.len();
    for (k, (l, c)) in runs.iter().enumerate() {
        if c.len() < 2 {
            continue;
        }
        let (w, g) = geo(*l);
        let half = (w + g) / 2.0;
        let (steps, dir0) = steps_of(c, radius_for(w).max(if r.corners == Some(Corners::Arc) { half } else { 0.0 }));
        let n = c.len();
        let dir_end = heading(c[n - 2], c[n - 1]);
        for (side, runs_out, vias_out, pad_s, pad_e) in [(sigma, &mut p_runs, &mut p_vias, ps, pe), (-sigma, &mut n_runs, &mut n_vias, ns, ne)] {
            let (mut pts, mut bul) = meander::realize(&steps, c[0], dir0, side * half);
            let head = if k == 0 { pad_s.at } else { geom::add(c[0], [left(dir0)[0] * side * pitch / 2.0, left(dir0)[1] * side * pitch / 2.0]) };
            let tail = if k + 1 == nr { pad_e.at } else { geom::add(c[n - 1], [left(dir_end)[0] * side * pitch / 2.0, left(dir_end)[1] * side * pitch / 2.0]) };
            if k + 1 < nr {
                vias_out.push(tail);
            }
            // Necks: pad (or via) to the coupled run, on their own when full width would crowd the partner.
            let mut neck_runs = Vec::new();
            if geom::dist(head, pts[0]) > 1e-6 {
                neck_runs.push((vec![head, pts[0]], true));
            }
            if geom::dist(*pts.last().unwrap(), tail) > 1e-6 {
                neck_runs.push((vec![*pts.last().unwrap(), tail], false));
            }
            for (seg, at_start) in neck_runs {
                let partner_pad = if at_start { if side == sigma { ns } else { ps } } else if side == sigma { ne } else { pe };
                let crowded = (k == 0 && at_start) || (k + 1 == nr && !at_start);
                let gap = geom::gap(&geom::Prim::capsule(seg[0], seg[1], w / 2.0), &partner_pad.prim);
                if crowded && gap + 1e-6 < g.min(class.clearance) && neck < w {
                    runs_out.push(PlanRun { layer: v.layers[*l].clone(), points: seg, bulges: vec![], width: Some(neck) });
                } else if at_start {
                    pts.insert(0, seg[0]);
                    bul.insert(0, 0.0);
                } else {
                    pts.push(seg[1]);
                    bul.push(0.0);
                }
            }
            runs_out.push(PlanRun { layer: v.layers[*l].clone(), points: pts, bulges: if bul.iter().all(|x| x.abs() < 1e-12) { vec![] } else { bul }, width: Some(w) });
        }
    }
    // Back to the real nets: what was pushed, and every clearance the new copper breaks.
    for (f, n) in node.fixed.iter_mut().zip(saved_fixed) {
        f.net = n;
    }
    for (i, l) in node.lines.iter_mut().enumerate() {
        if i < saved_lines.len() {
            l.net = saved_lines[i].clone();
        }
    }
    node.lines.truncate(lines0);
    for (i, x) in node.vias.iter_mut().enumerate() {
        if i < saved_vias.len() {
            x.net = saved_vias[i].clone();
        }
    }
    node.vias.truncate(vias0);
    let first_new = node.lines.len();
    let mut idx = Vec::new();
    for (net, list) in [(&p.p, &p_runs), (&p.n, &n_runs)] {
        for run in list {
            let l = layer_index(&v.layers, &run.layer).unwrap();
            idx.push((Some(net.clone()), l, run.width.unwrap_or(w0), pns::flatten(&run.points, &run.bulges)));
            node.lines.push(pns::Line { id: None, net: Some(net.clone()), layer: l, width: run.width.unwrap_or(w0), points: pns::flatten(&run.points, &run.bulges), locked: true, arcs: true, moved: false });
        }
    }
    let first_via = node.vias.len();
    for (net, list) in [(&p.p, &p_vias), (&p.n, &n_vias)] {
        for at in list.iter() {
            node.vias.push(pns::Via { id: None, net: Some(net.clone()), at: *at, diameter: via_d, drill: via_drill, locked: true, moved: false });
        }
    }
    let mut collisions = Vec::new();
    for (j, (net, l, w, pts)) in idx.iter().enumerate() {
        let me = first_new + j;
        // A track's own neck and run meet end to end: same net, never reported.
        collisions.extend(node.line_report(pts, *w, *l, net, &|r| matches!(r, Ref::Line(i, _) if i == me)));
    }
    for (j, x) in node.vias.iter().enumerate().skip(first_via) {
        collisions.extend(node.via_report(x.at, via_d, &x.net, &|r| r == Ref::Via(j)));
    }
    collisions.extend(routing::moved_collisions(&node));
    let mut pp = Plan { ok: true, reached: out.reached, mode: out.mode, net: Some(p.p.clone()), width: w0, via_drill, via_diameter: via_d, clearance: class.clearance, runs: p_runs, vias: p_vias, end: pe.at, layer: v.layers[layer].clone(), ..Default::default() };
    routing::moved_of(&node, &BTreeSet::new(), &mut pp);
    let np = Plan { ok: true, reached: out.reached, mode: out.mode, net: Some(p.n.clone()), width: w0, via_drill, via_diameter: via_d, clearance: class.clearance, runs: n_runs, vias: n_vias, end: ne.at, layer: v.layers[layer].clone(), ..Default::default() };
    let (lp, tp) = signal::readout(b, v, &p.p, &pp.runs, &pp.vias);
    let (ln, _) = signal::readout(b, v, &p.n, &np.runs, &np.vias);
    pp.length = lp;
    pp.target = tp;
    let mut np = np;
    np.length = ln;
    out.length_p = lp;
    out.length_n = ln;
    out.skew = ((lp - ln).abs() * 1e4).round() / 1e4;
    out.ok = collisions.is_empty() && out.reached;
    pp.collisions = collisions;
    pp.ok = out.ok;
    np.ok = out.ok;
    out.p = pp;
    out.n = np;
    Ok(out)
}

fn apply(tx: &mut Tx, board: StableId, plan: &PairPlan, auto: bool) -> Result<Value, CommandError> {
    let mut a = plan.p.clone();
    let mut c = plan.n.clone();
    a.auto = auto;
    c.auto = auto;
    let rp = routing::apply(tx, board, &a)?;
    let rn = routing::apply(tx, board, &c)?;
    Ok(json!({
        "pair": plan.pair, "ok": plan.ok, "reached": plan.reached, "mode": plan.mode, "width": plan.width, "gap": plan.gap,
        "tracks": [rp["tracks"].clone(), rn["tracks"].clone()], "vias": [rp["vias"].clone(), rn["vias"].clone()],
        "length_p": plan.length_p, "length_n": plan.length_n, "skew": plan.skew, "collisions": plan.p.collisions, "notes": plan.notes,
    }))
}

impl Command for RoutePair {
    const NAME: &'static str = "pcb.route.pair";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (b, v) = routing::resolved(tx, self.board)?;
        let plan = plan(&b, &v, &self).map_err(bad)?;
        if plan.p.runs.is_empty() {
            return Err(bad(format!("nothing to lay: {}", plan.notes.join("; "))));
        }
        apply(tx, self.board, &plan, false)
    }
}

/// Route every unrouted pair (of the nets `pick` allows) as a pair, where it fits cleanly. Returns the nets routed.
pub fn autoroute_pairs(tx: &mut Tx, board: StableId, pick: &dyn Fn(&str) -> bool) -> Result<(Vec<String>, Vec<String>), CommandError> {
    let mut done = Vec::new();
    let mut notes = Vec::new();
    let (b, v) = routing::resolved(tx, board)?;
    let todo: Vec<String> = v
        .signal
        .pairs
        .iter()
        .filter(|p| pick(&p.p) && pick(&p.n))
        .filter(|p| v.nets.iter().filter(|n| n.name == p.p || n.name == p.n).all(|n| !n.routed && n.islands == n.pins.len() && n.pins.len() == 2))
        .map(|p| p.name.clone())
        .collect();
    let mut b = b;
    let mut v = v;
    for name in todo {
        let r = RoutePair { board, pair: name.clone(), from: None, to: None, steps: vec![], layer: None, mode: Some(Mode::Walkaround), angle: None, width: None, gap: None, via_pitch: None, corners: None, arc_radius: None, optimize: Some(true) };
        match plan(&b, &v, &r) {
            Ok(pl) if pl.ok && pl.reached => {
                let pp = find(&v, &name).map(|x| (x.p.clone(), x.n.clone())).ok();
                apply(tx, board, &pl, true)?;
                if let Some((a, c)) = pp {
                    done.push(a);
                    done.push(c);
                }
                let (nb, nv) = routing::resolved(tx, board)?;
                b = nb;
                v = nv;
            }
            Ok(pl) => notes.push(format!("pair {name}: not routed as a pair ({}), routed track by track", if pl.notes.is_empty() { format!("{} clearance problems", pl.p.collisions.len()) } else { pl.notes.join("; ") })),
            Err(e) => notes.push(format!("pair {name}: {e}")),
        }
    }
    Ok((done, notes))
}

// ---- Dragging a pair ------------------------------------------------------------------

/// `pcb.pair.drag`: move a segment of one track of a pair sideways and its
/// partner's coupled segment with it, pushing what is in the way (shove) or
/// reporting it (`mode: highlight`).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct DragPair {
    pub board: StableId,
    pub track: StableId,
    pub index: usize,
    pub from: Option<[f64; 2]>,
    pub to: [f64; 2],
    pub mode: Option<Mode>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DragPlan {
    pub ok: bool,
    pub partner: Option<StableId>,
    pub plan: Plan,
}

pub fn plan_drag(b: &Board, v: &BoardView, d: &DragPair) -> Result<DragPlan, String> {
    let t = v.tracks.iter().find(|t| t.id == d.track).ok_or_else(|| format!("{} is not a track", d.track))?;
    if !t.bulges.is_empty() {
        return Err("the track has arcs: drag its segments one track at a time".into());
    }
    let net = t.net.clone().ok_or("the track has no net")?;
    let pair = signal::pair_of(v, &net).ok_or_else(|| format!("{net} is in no differential pair"))?;
    let other = if pair.p == net { &pair.n } else { &pair.p };
    if d.index + 1 >= t.points.len() {
        return Err(format!("the track has {} segments", t.points.len() - 1));
    }
    let (a, c) = (t.points[d.index], t.points[d.index + 1]);
    let u = geom::sub(c, a);
    let lu = geom::len(u);
    let u = [u[0] / lu, u[1] / lu];
    // The partner's segment running alongside.
    let mut best: Option<(f64, StableId, usize)> = None;
    for o in v.tracks.iter().filter(|o| o.net.as_deref() == Some(other.as_str()) && o.layer == t.layer && o.bulges.is_empty()) {
        for (k, w) in o.points.windows(2).enumerate() {
            let e = geom::sub(w[1], w[0]);
            let le = geom::len(e);
            if le < 1e-9 || geom::cross(u, [e[0] / le, e[1] / le]).abs() > 1e-3 {
                continue;
            }
            let dist = geom::cross(u, geom::sub(w[0], a)).abs();
            let (t0, t1) = (geom::dot(geom::sub(w[0], a), u), geom::dot(geom::sub(w[1], a), u));
            if t0.max(t1) < 0.0 || t0.min(t1) > lu {
                continue;
            }
            if best.is_none_or(|x| dist < x.0) {
                best = Some((dist, o.id, k));
            }
        }
    }
    let (_, oid, ok_) = best.ok_or("no coupled segment of the partner beside this one")?;
    let from = d.from.unwrap_or(mid(a, c));
    let delta = geom::sub(d.to, from);
    let mut node = Node::new(b, v);
    let mode = d.mode.unwrap_or(Mode::Shove);
    let mut pushers = Vec::new();
    for (id, k) in [(t.id, d.index), (oid, ok_)] {
        let i = node.line_by_id(id).ok_or("track not found")?;
        let pts = routing::drag_segment(&node.lines[i].points, k, delta, Angle::Diagonal);
        node.lines[i].points = pts;
        node.lines[i].locked = true;
        node.lines[i].moved = true;
        pushers.push(Pusher::Line(i));
    }
    let mut plan = Plan { ok: true, reached: true, mode, net: Some(net), ..Default::default() };
    if mode == Mode::Shove {
        let mut trial = node.clone();
        match trial.shove(&pushers) {
            Ok(()) => node = trial,
            Err(e) => plan.notes.push(format!("cannot shove: {e}")),
        }
    }
    plan.collisions = routing::moved_collisions(&node);
    routing::moved_of(&node, &BTreeSet::new(), &mut plan);
    plan.ok = plan.collisions.is_empty();
    Ok(DragPlan { ok: plan.ok, partner: Some(oid), plan })
}

impl Command for DragPair {
    const NAME: &'static str = "pcb.pair.drag";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let (b, v) = routing::resolved(tx, self.board)?;
        let p = plan_drag(&b, &v, &self).map_err(bad)?;
        let r = routing::apply(tx, self.board, &p.plan)?;
        Ok(json!({ "ok": p.ok, "partner": p.partner, "moved": r["pushed_tracks"], "collisions": p.plan.collisions, "notes": p.plan.notes }))
    }
}

pub fn register(r: &mut de_core::Registry) {
    r.command::<RoutePair>().command::<DragPair>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_centreline_keeps_the_pitch() {
        // An L with a 45° chamfer: both offsets stay one pitch apart (mitred), and arcs too.
        let c = vec![[0.0, 0.0], [10.0, 0.0], [12.0, 2.0], [12.0, 10.0]];
        for radius in [0.0, 1.0] {
            let (steps, d0) = steps_of(&c, radius);
            let (a, ab) = meander::realize(&steps, c[0], d0, 0.175);
            let (z, zb) = meander::realize(&steps, c[0], d0, -0.175);
            let (fa, fz) = (pns::flatten(&a, &ab), pns::flatten(&z, &zb));
            for q in &fa {
                let d = fz.windows(2).map(|w| geom::point_seg_dist(*q, w[0], w[1])).fold(f64::INFINITY, f64::min);
                assert!((d - 0.35).abs() < 0.005, "radius {radius}: {d}");
            }
            // The centreline's ends are kept: the offsets end beside them.
            let end = *a.last().unwrap();
            assert!((geom::dist(end, [12.0, 10.0]) - 0.175).abs() < 1e-9, "{end:?}");
        }
    }
}
