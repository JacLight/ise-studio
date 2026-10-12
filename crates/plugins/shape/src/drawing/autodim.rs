//! The dimensions a machinist needs to make the part, found by the drawing
//! itself from the part's features: every step, wall, hole and slot centre
//! placed once (ordinate dimensions from the part's lower left corner, in the
//! view where the feature shows best), every hole called out by size with its
//! depth, counterbore, countersink, thread and fit, every cutter radius and
//! chamfer, and a turned part's diameters (with their fits and threads) and
//! shoulder positions along its axis. Each dimension has a key ("x:25",
//! "d:20", "hole:5", "r:8", "c:1") the person can hide it or tolerance it by.

use super::annotate::{self, Grid, Label};
use super::features::{self, Features, Hole, Stretch};
use super::*;

/// Dimension and callout lettering (mm cap height).
const H: f64 = 3.5;

/// A view as the auto dimensions see it: its lines (model mm along its axes) and where it sits.
pub(super) struct ViewIn<'a> {
    pub name: ViewName,
    pub lines: &'a [ViewLine],
    pub at: [f64; 2],
    pub bounds: [f64; 4],
    /// The front view is a section: what it cuts shows, nothing is hidden.
    pub section: Option<f64>,
}

impl ViewIn<'_> {
    /// The model axis the view looks along.
    fn look(&self) -> usize {
        match self.name {
            ViewName::Front => 1,
            ViewName::Top => 2,
            ViewName::Right => 0,
        }
    }
    fn sheet(&self, p: [f64; 3], s: f64) -> [f64; 2] {
        let [a, b] = self.name.axes();
        [self.at[0] + p[a] * s, self.at[1] + p[b] * s]
    }
    /// Whether a flat square to the view's axis `i` (0 across, 1 up) at `v`
    /// shows as a visible line here.
    fn shows(&self, i: usize, v: f64) -> bool {
        self.lines.iter().any(|l| {
            !l.hidden && l.points.iter().all(|p| (p[i] - v).abs() < 0.01) && {
                let (lo, hi) = l.points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p[1 - i]), hi.max(p[1 - i])));
                hi - lo > 0.3
            }
        })
    }
}

pub(super) struct Ctx<'a> {
    pub views: Vec<ViewIn<'a>>,
    pub s: f64,
    /// The part's box (model mm): [x0, y0, z0, x1, y1, z1].
    pub bounds: [f64; 6],
    pub feats: &'a Features,
    pub options: &'a Options,
    /// The sheet frame's inside.
    pub frame: [f64; 4],
    /// Stretches a broken view leaves out: values are stated true.
    pub breaks: &'a Breaks,
}

/// Where an automatic dimension landed, for the person to pick it (hide it, tolerance it).
#[derive(Clone, Debug, Serialize)]
pub struct AutoMark {
    pub key: String,
    pub text: String,
    pub view: ViewName,
    pub bounds: [f64; 4],
    /// The model value it states (mm).
    pub value: f64,
}

/// What the auto dimensions took care of: the axes whose overall size their ordinates already give.
#[derive(Default)]
pub(super) struct Done {
    pub axes: [bool; 3],
}

const AXIS: [char; 3] = ['x', 'y', 'z'];

fn key_value(v: f64) -> String {
    length_text(v)
}

/// The tolerance the person set on an automatic dimension.
fn tol_for<'a>(options: &'a Options, key: &str) -> Option<&'a Tolerance> {
    options.auto.tolerances.iter().find(|t| t.key == key).map(|t| &t.tolerance)
}

fn hidden(options: &Options, key: &str) -> bool {
    options.auto.hide.iter().any(|h| h == key)
}

/// Positions spread along a line so neighbours stay `gap` apart, each as near
/// its own as can be (least squares, order kept: pool adjacent violators).
pub(super) fn spread(c: &[f64], gap: f64) -> Vec<f64> {
    let u: Vec<f64> = c.iter().enumerate().map(|(j, v)| v - j as f64 * gap).collect();
    // Blocks of (sum, count), merged while out of order.
    let mut blocks: Vec<(f64, usize)> = vec![];
    for &x in &u {
        blocks.push((x, 1));
        while blocks.len() > 1 {
            let n = blocks.len();
            let (a, b) = (blocks[n - 2], blocks[n - 1]);
            if a.0 / a.1 as f64 <= b.0 / b.1 as f64 {
                break;
            }
            blocks.truncate(n - 2);
            blocks.push((a.0 + b.0, a.1 + b.1));
        }
    }
    let mut out = vec![];
    for (sum, n) in blocks {
        for _ in 0..n {
            out.push(sum / n as f64);
        }
    }
    out.iter().enumerate().map(|(j, v)| v + j as f64 * gap).collect()
}

/// Marks where an annotation drew (strokes, texts, fills from these indices on).
fn mark_from(ink: &mut Grid, d: &Drawing, s0: usize, t0: usize, f0: usize) {
    ink.mark_drawn(d, s0..d.strokes.len(), t0..d.texts.len());
    for f in &d.fills[f0..] {
        for p in f {
            ink.mark([p[0] - 0.3, p[1] - 0.3, p[0] + 0.3, p[1] + 0.3]);
        }
    }
}

fn bounds_since(d: &Drawing, s0: usize, t0: usize, f0: usize) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.strokes[s0..].iter().flat_map(|s| &s.points).chain(d.fills[f0..].iter().flatten()) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    for t in &d.texts[t0..] {
        let r = text_box(t);
        b = [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])];
    }
    b
}

/// One ordinate value: where it is (model mm), what it says, its key.
struct Ord {
    v: f64,
    label: Label,
    key: String,
}

/// Ordinate dimensions along one axis of a view (ISO 129-1): a 0 at the
/// datum, each value at the end of its extension line, lines jogged where
/// values crowd. On whichever side of the view has room. False when no side has.
fn ordinates(d: &mut Drawing, ink: &mut Grid, view: &ViewIn, i: usize, k: usize, values: &[Ord], s: f64, frame: [f64; 4]) -> bool {
    if values.is_empty() {
        return true;
    }
    let c: Vec<f64> = values.iter().map(|o| view.at[i] + o.v * s).collect();
    let gap = H + 1.4;
    let t = spread(&c, gap);
    let widths: Vec<f64> = values.iter().map(|o| annotate::label_width(&o.label, H)).collect();
    let longest = widths.iter().cloned().fold(0.0, f64::max);
    let (lo, hi) = (t.first().unwrap() - gap / 2.0, t.last().unwrap() + gap / 2.0);
    // The band each side would take: across (along i) and out from the view.
    let reach = 10.0 + longest + 1.0;
    let band = |side: f64| -> [f64; 4] {
        let edge = if i == 0 { if side > 0.0 { view.bounds[3] } else { view.bounds[1] } } else if side > 0.0 { view.bounds[2] } else { view.bounds[0] };
        let (n0, n1) = if side > 0.0 { (edge + 1.0, edge + reach) } else { (edge - reach, edge - 1.0) };
        if i == 0 { [lo, n0, hi, n1] } else { [n0, lo, n1, hi] }
    };
    let inside = |r: [f64; 4]| r[0] >= frame[0] + 1.0 && r[1] >= frame[1] + 1.0 && r[2] <= frame[2] - 1.0 && r[3] <= frame[3] - 1.0;
    let mut sides: Vec<(usize, f64)> = [-1.0, 1.0].iter().map(|&side| (if inside(band(side)) { ink.used(band(side)) } else { usize::MAX }, side)).map(|(u, s)| (u, s)).collect::<Vec<_>>().into_iter().collect();
    // Below and right first when both are clear (the way a drawing reads).
    sides.sort_by_key(|(u, side)| (*u, if (i == 0 && *side < 0.0) || (i == 1 && *side > 0.0) { 0 } else { 1 }));
    let (used, side) = sides[0];
    if used == usize::MAX {
        return false;
    }
    let edge = if i == 0 { if side > 0.0 { view.bounds[3] } else { view.bounds[1] } } else if side > 0.0 { view.bounds[2] } else { view.bounds[0] };
    let pt = |along: f64, out: f64| if i == 0 { [along, edge + side * out] } else { [edge + side * out, along] };
    for (j, o) in values.iter().enumerate() {
        let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
        let points = if (t[j] - c[j]).abs() < 0.05 { vec![pt(c[j], 1.5), pt(c[j], 9.0)] } else { vec![pt(c[j], 1.5), pt(c[j], 4.5), pt(t[j], 7.5), pt(t[j], 9.0)] };
        d.strokes.push(Stroke { pen: Pen::Thin, points });
        let start = 9.6;
        if i == 0 {
            // Upright, reading from the right: the letters sit left of the baseline.
            let anchor = if side > 0.0 { Anchor::Start } else { Anchor::End };
            annotate::place_label(d, &o.label, [t[j] + H / 2.0, edge + side * start], anchor, 90.0, H);
        } else {
            let anchor = if side > 0.0 { Anchor::Start } else { Anchor::End };
            annotate::place_label(d, &o.label, [edge + side * start, t[j] - H / 2.0], anchor, 0.0, H);
        }
        let b = bounds_since(d, s0, t0, f0);
        mark_from(ink, d, s0, t0, f0);
        if !o.key.is_empty() {
            d.autos.push(AutoMark { key: o.key.clone(), text: o.label.main.clone(), view: view.name, bounds: b, value: o.v });
        }
    }
    // The band is the ordinates' own: callouts keep out of it.
    ink.mark(band(side));
    let _ = (k, used);
    true
}

/// A leader callout: lines of text on a shoulder, the leader's arrow on
/// `tip`, coming in from direction `from` (sheet mm). Tries angles and
/// lengths for a spot clear of everything drawn; None when none is.
fn callout(d: &mut Drawing, ink: &mut Grid, words: &mut Grid, tip: [f64; 2], away: f64, lines: &[Label], frame: [f64; 4], prefer: Option<[f64; 2]>) -> Option<[f64; 4]> {
    let widths: Vec<f64> = lines.iter().map(|l| annotate::label_width(l, H)).collect();
    let width = widths.iter().cloned().fold(0.0, f64::max);
    let line_h = |l: &Label| if l.devs.is_some() { 7.0 } else { 5.2 };
    let tall: f64 = lines.iter().map(line_h).sum();
    let mut angles = vec![45.0f64, 135.0, -45.0, -135.0, 60.0, 120.0, -60.0, -120.0, 30.0, 150.0, -30.0, -150.0];
    if let Some(p) = prefer {
        // Towards where the callout is wanted first (outside the view).
        let a = (p[1] - tip[1]).atan2(p[0] - tip[0]).to_degrees();
        angles.sort_by(|x, y| ang_diff(*x, a).total_cmp(&ang_diff(*y, a)));
    }
    // Of the spots whose words are clear of everything and whose leader
    // crosses no lettering, the one with the shortest leader over the fewest
    // lines, in the way preferred.
    let mut best: Option<(f64, f64, f64)> = None; // score, angle, reach
    for reach in [6.0, 10.0, 15.0, 21.0, 28.0, 36.0, 46.0] {
        for (rank, &deg) in angles.iter().enumerate() {
            let (sn, cs) = deg.to_radians().sin_cos();
            let start = [tip[0] + away * cs, tip[1] + away * sn];
            let knee = [start[0] + reach * cs, start[1] + reach * sn];
            let right = cs >= 0.0;
            let end = [knee[0] + if right { width + 2.0 } else { -(width + 2.0) }, knee[1]];
            // The first line sits on the shoulder; the rest hang under it.
            let bx = [knee[0].min(end[0]) - 0.6, knee[1] - (tall - 5.2) - 1.2, knee[0].max(end[0]) + 0.6, knee[1] + line_h(&lines[0]) - 0.6];
            if bx[0] < frame[0] + 1.0 || bx[2] > frame[2] - 1.0 || bx[1] < frame[1] + 1.0 || bx[3] > frame[3] - 1.0 {
                continue;
            }
            if !ink.free(bx) || !words.line_free(start, knee, 0.5, 0.5) {
                continue;
            }
            let score = reach + 4.0 * ink.hits(start, knee, 1.5, 0.8) as f64 + 1.5 * rank as f64;
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, deg, reach));
            }
        }
    }
    let (_, deg, reach) = best?;
    let (sn, cs) = deg.to_radians().sin_cos();
    let start = [tip[0] + away * cs, tip[1] + away * sn];
    let knee = [start[0] + reach * cs, start[1] + reach * sn];
    let right = cs >= 0.0;
    let end = [knee[0] + if right { width + 2.0 } else { -(width + 2.0) }, knee[1]];
    let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
    // The arrow on the feature (a circle's edge, `away` out from its centre).
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![start, knee, end] });
    arrow(d, start, [-cs, -sn]);
    let mut y = knee[1] + 1.0;
    for l in lines {
        annotate::place_label(d, l, [knee[0] + if right { 1.0 } else { -1.0 }, y], if right { Anchor::Start } else { Anchor::End }, 0.0, H);
        y -= line_h(l);
    }
    let b = bounds_since(d, s0, t0, f0);
    mark_from(ink, d, s0, t0, f0);
    words.mark_drawn(d, s0..s0, t0..d.texts.len());
    Some(b)
}

fn ang_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn plain(text: String) -> Label {
    Label { main: text, devs: None, paren: false }
}

/// The thread the person gave holes or a diameter of this (modelled) size.
fn thread_for(options: &Options, diameter: f64) -> Option<&ThreadSpec> {
    options.threads.iter().find(|t| (t.diameter - diameter).abs() < 0.011)
}

/// A thread's designation: M6, M16×1.5 (a fine pitch is written), its tolerance class after a dash.
fn designation(t: &ThreadSpec) -> String {
    let (d, p) = features::parse_thread(&t.size).unwrap_or((t.diameter, 1.0));
    let mut s = if features::coarse_pitch(d).is_some_and(|c| (c - p).abs() < 1e-9) { format!("M{}", length_text(d)) } else { format!("M{}×{}", length_text(d), length_text(p)) };
    if let Some(c) = t.class.as_ref().filter(|c| !c.trim().is_empty()) {
        s.push('-');
        s.push_str(c.trim());
    }
    s
}

/// A hole callout's lines: count and size, depth, then its counterbore or countersink.
fn hole_lines(h: &Hole, n: usize, options: &Options) -> Vec<Label> {
    let count = if n > 1 { format!("{n}× ") } else { String::new() };
    let depth = |v: f64| format!("↧{}", length_text(v));
    let mut lines = vec![];
    if let Some(t) = thread_for(options, h.diameter) {
        let thread_depth = t.depth.filter(|_| !h.through).unwrap_or(h.depth);
        let end = if h.through && t.depth.is_none() { " THRU".to_string() } else { format!(" {}", depth(thread_depth)) };
        lines.push(plain(format!("{count}{}{end}", designation(t))));
        // The tap drill, when it goes deeper than the thread.
        if !h.through && thread_depth + 1e-6 < h.depth {
            lines.push(plain(format!("⌀{} {}", length_text(h.diameter), depth(h.depth))));
        }
    } else {
        let fit = options.hole_fits.iter().find(|f| (f.diameter - h.diameter).abs() < 1e-3).map(|f| Tolerance::Fit { fit: f.fit.clone() });
        let mut l = annotate::label(h.diameter, true, fit.as_ref());
        l.main = format!("{count}{}{}", l.main, if h.through { " THRU".to_string() } else { format!(" {}", depth(h.depth)) });
        if l.devs.is_some() {
            // The depth goes on its own line after a toleranced size.
            let end = if h.through { "THRU".to_string() } else { depth(h.depth) };
            let mut sized = annotate::label(h.diameter, true, fit.as_ref());
            sized.main = format!("{count}{}", sized.main);
            lines.push(sized);
            lines.push(plain(end));
        } else {
            lines.push(l);
        }
    }
    if let Some((cd, cdepth)) = h.cbore {
        lines.push(plain(format!("⌴⌀{} {}", length_text(cd), depth(cdepth))));
    }
    if let Some((cd, angle)) = h.csk {
        lines.push(plain(format!("⌵⌀{} × {}°", length_text(cd), length_text(angle.round()))));
    }
    lines
}

/// The automatic dimensions of one part, laid out on the views.
pub(super) fn dimension(d: &mut Drawing, ink: &mut Grid, words: &mut Grid, cx: &Ctx) -> Done {
    let mut done = Done::default();
    if cx.options.auto.off {
        return done;
    }
    let f = cx.feats;
    match &f.turned {
        Some(t) => turned(d, ink, words, cx, t, &mut done),
        None => prismatic(d, ink, words, cx, &mut done),
    }
    if let Ok(path) = std::env::var("DEBUG_INK") {
        ink.dump(&path);
    }
    holes(d, ink, words, cx);
    rounds(d, ink, words, cx);
    done
}

/// A coordinate to dimension along an axis, and the view it reads best in.
struct Item {
    axis: usize,
    v: f64,
    /// It is a hole's or slot's centre: the view that shows it round.
    round_in: Option<ViewName>,
}

fn prismatic(d: &mut Drawing, ink: &mut Grid, _words: &mut Grid, cx: &Ctx, done: &mut Done) {
    let f = cx.feats;
    let look_view = |axis: usize| cx.views.iter().find(|v| v.look() == axis).map(|v| v.name);
    let mut items: Vec<Item> = vec![];
    for h in &f.holes {
        let Some(a) = h.axis() else { continue };
        for k in (0..3).filter(|&k| k != a) {
            items.push(Item { axis: k, v: h.entry[k], round_in: look_view(a) });
        }
    }
    for (line, _, _) in &f.bosses {
        let Some(a) = features::principal(line.dir) else { continue };
        for k in (0..3).filter(|&k| k != a) {
            items.push(Item { axis: k, v: line.point[k], round_in: look_view(a) });
        }
    }
    // Slot ends and rounded ends (half rounds or more) are placed by their centres.
    for r in f.rounds.iter().filter(|r| r.sweep > std::f64::consts::PI - 0.02) {
        for k in (0..3).filter(|&k| k != r.axis) {
            items.push(Item { axis: k, v: r.centre[k], round_in: look_view(r.axis) });
        }
    }
    for fl in f.flats.iter().filter(|fl| !cx.breaks.is_cut(fl.axis, fl.at)) {
        items.push(Item { axis: fl.axis, v: fl.at, round_in: None });
    }
    for k in 0..3 {
        let lo = cx.bounds[k];
        let hi = cx.bounds[k + 3];
        let all: Vec<&Item> = items.iter().filter(|it| it.axis == k && it.v > lo + 5e-3 && it.v < hi - 5e-3).collect();
        if all.is_empty() {
            continue;
        }
        // The views that can state a position along k, and where k lies in each (0 across, 1 up).
        let cands: Vec<(usize, usize)> = cx.views.iter().enumerate().filter_map(|(vi, v)| v.name.axes().iter().position(|&a| a == k).map(|i| (vi, i))).collect();
        if cands.is_empty() {
            continue;
        }
        // Holes go where they show round; the view with most of them is this axis's home.
        let mut votes = vec![0usize; cx.views.len()];
        for it in &all {
            if let Some(n) = it.round_in {
                if let Some(&(vi, _)) = cands.iter().find(|(vi, _)| cx.views[*vi].name == n) {
                    votes[vi] += 1;
                }
            }
        }
        let main_name = match k {
            0 | 1 => ViewName::Top,
            _ => ViewName::Front,
        };
        let home = cands.iter().map(|c| c.0).max_by_key(|&vi| (votes[vi], cx.views[vi].name == main_name)).unwrap();
        // One per value: a centre that shows round at home first, then any centre, then a flat.
        let rank = |it: &Item| match it.round_in {
            Some(n) if n == cx.views[home].name => 0,
            Some(_) => 1,
            None => 2,
        };
        let mut interior = all;
        interior.sort_by(|a, b| a.v.total_cmp(&b.v).then(rank(a).cmp(&rank(b))));
        interior.dedup_by(|b, a| (a.v - b.v).abs() < 5e-3);
        let mut groups: Vec<Vec<f64>> = vec![vec![]; cx.views.len()];
        for it in &interior {
            let vi = match it.round_in.and_then(|n| cands.iter().find(|(vi, _)| cx.views[*vi].name == n)) {
                Some(&(vi, _)) => vi,
                None => {
                    let i_home = cands.iter().find(|c| c.0 == home).unwrap().1;
                    if cx.views[home].shows(i_home, it.v) {
                        home
                    } else {
                        cands.iter().find(|(vi, i)| cx.views[*vi].shows(*i, it.v)).map(|c| c.0).unwrap_or(home)
                    }
                }
            };
            groups[vi].push(it.v);
        }
        // The total goes with the largest group; every group starts at the datum.
        let biggest = (0..groups.len()).max_by_key(|&vi| (groups[vi].len(), vi == home)).unwrap();
        for (vi, mut vals) in groups.into_iter().enumerate() {
            if vals.is_empty() {
                continue;
            }
            vals.insert(0, lo);
            if vi == biggest {
                vals.push(hi);
            }
            let i = cands.iter().find(|c| c.0 == vi).unwrap().1;
            let values: Vec<Ord> = vals
                .iter()
                .filter_map(|&v| {
                    let shown = cx.breaks.true_at(k, v) - cx.breaks.true_at(k, lo);
                    let key = if shown.abs() < 1e-9 { String::new() } else { format!("{}:{}", AXIS[k], key_value(shown)) };
                    if !key.is_empty() && hidden(cx.options, &key) {
                        return None;
                    }
                    let label = if key.is_empty() { plain("0".into()) } else { annotate::label(shown, false, tol_for(cx.options, &key)) };
                    Some(Ord { v, label, key })
                })
                .collect();
            if ordinates(d, ink, &cx.views[vi], i, k, &values, cx.s, cx.frame) {
                if vi == biggest {
                    done.axes[k] = true;
                }
            } else {
                d.warnings.push(format!("no room for the {} positions beside the {} view", AXIS[k], format!("{:?}", cx.views[vi].name).to_lowercase()));
            }
        }
    }
}

/// A turned part: its diameters across the profile, shoulders along the
/// axis, chamfers and rounds, threads drawn the ISO 6410 way.
fn turned(d: &mut Drawing, ink: &mut Grid, words: &mut Grid, cx: &Ctx, t: &features::Turned, done: &mut Done) {
    let Some(a) = features::principal(t.line.dir) else { return };
    // The view the axis lies in, across it if one does.
    let pick = cx.views.iter().enumerate().filter_map(|(vi, v)| v.name.axes().iter().position(|&x| x == a).map(|i| (vi, i))).min_by_key(|(_, i)| *i);
    let Some((vi, i)) = pick else { return };
    let view = &cx.views[vi];
    let s = cx.s;
    // The view's other axis: the profile's radius runs along it.
    let other = view.name.axes()[1 - i];
    let centre = t.line.point;
    let on_sheet = |along: f64, radial: f64| {
        let mut p = centre;
        p[a] = along;
        p[other] = centre[other] + radial;
        view.sheet(p, s)
    };
    // Diameters: each once, on its longest stretch.
    let mut diameters: Vec<Stretch> = vec![];
    for st in t.outside.iter().filter(|st| st.is_cylinder()) {
        match diameters.iter_mut().find(|o| (o.r0 - st.r0).abs() < 1e-3) {
            Some(o) if o.length() < st.length() => *o = *st,
            Some(_) => {}
            None => diameters.push(*st),
        }
    }
    diameters.sort_by(|x, y| x.from.total_cmp(&y.from));
    for st in &diameters {
        let dia = 2.0 * st.r0;
        let key = format!("d:{}", key_value(dia));
        if hidden(cx.options, &key) {
            continue;
        }
        let thread = thread_for(cx.options, dia);
        let label = match thread {
            Some(th) => plain(designation(th)),
            None => {
                let fit = cx.options.hole_fits.iter().find(|h| (h.diameter - dia).abs() < 1e-3 && !fits::is_hole(&h.fit)).map(|h| Tolerance::Fit { fit: h.fit.clone() });
                let tol = tol_for(cx.options, &key).cloned().or(fit);
                annotate::label(dia, true, tol.as_ref())
            }
        };
        // Across the profile at the stretch's middle, text up beside the line.
        let mid = (st.from + st.to) / 2.0;
        let (pa, pb) = (on_sheet(mid, -st.r0), on_sheet(mid, st.r0));
        let (s0, t0, f0) = (d.strokes.len(), d.texts.len(), d.fills.len());
        across(d, ink, pa, pb, &label, (st.to - st.from) * s);
        let b = bounds_since(d, s0, t0, f0);
        mark_from(ink, d, s0, t0, f0);
        d.autos.push(AutoMark { key, text: label.main.clone(), view: view.name, bounds: b, value: dia });
        // An external thread: thin lines at its minor diameter along it (ISO 6410-1).
        if let Some(th) = thread {
            if let Ok((nominal, pitch)) = features::parse_thread(&th.size) {
                let r = features::minor_diameter(nominal, pitch, false) / 2.0;
                let (from, to) = thread_run(t, st, th);
                for side in [-1.0, 1.0] {
                    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![on_sheet(from, side * r), on_sheet(to, side * r)] });
                }
                ink.mark_line(on_sheet(from, r), on_sheet(to, r), 0.3);
                ink.mark_line(on_sheet(from, -r), on_sheet(to, -r), 0.3);
            }
        }
    }
    // Shoulders: ordinates along the axis from the left (or lower) end.
    let lo = cx.bounds[a];
    let hi = cx.bounds[a + 3];
    let mut vals: Vec<f64> = t.shoulders.iter().copied().filter(|v| *v > lo - 1e-3 && *v < hi + 1e-3 && !cx.breaks.is_cut(a, *v)).collect();
    vals.extend([lo, hi]);
    vals.sort_by(f64::total_cmp);
    vals.dedup_by(|x, y| (*x - *y).abs() < 5e-3);
    let values: Vec<Ord> = vals
        .iter()
        .filter_map(|&v| {
            let shown = cx.breaks.true_at(a, v) - cx.breaks.true_at(a, lo);
            let key = if shown.abs() < 1e-9 { String::new() } else { format!("{}:{}", AXIS[a], key_value(shown)) };
            if !key.is_empty() && hidden(cx.options, &key) {
                return None;
            }
            let label = if key.is_empty() { plain("0".into()) } else { annotate::label(shown, false, tol_for(cx.options, &key)) };
            Some(Ord { v, label, key })
        })
        .collect();
    if ordinates(d, ink, view, i, a, &values, s, cx.frame) {
        done.axes[a] = true;
    }
    // Both diameters across the axis are the profile's: no overall sizes there.
    done.axes[other] = true;
    if let Some(look) = (0..3).find(|&k| k != a && k != other) {
        done.axes[look] = true;
    }
    // Chamfers: 45° cones, each called out where it is.
    for st in t.outside.iter().filter(|st| !st.is_cylinder()) {
        let dr = (st.r1 - st.r0).abs();
        if (dr - st.length()).abs() > 1e-3 {
            continue;
        }
        let key = format!("c:{}@{}", key_value(st.length()), key_value(st.from.min(st.to) - lo));
        if hidden(cx.options, &key) {
            continue;
        }
        let mid = (st.from + st.to) / 2.0;
        let r = (st.r0 + st.r1) / 2.0;
        let tip = on_sheet(mid, r);
        let text = format!("{}×45°", length_text(st.length()));
        let above = on_sheet(mid, r + 30.0);
        if let Some(b) = callout(d, ink, words, tip, 0.0, &[plain(text.clone())], cx.frame, Some(above)) {
            d.autos.push(AutoMark { key, text, view: view.name, bounds: b, value: st.length() });
        }
    }
    for &(at, r, _) in &t.fillets {
        let key = format!("r:{}", key_value(r));
        if hidden(cx.options, &key) {
            continue;
        }
        let near = t.outside.iter().filter(|st| st.from <= at + 1e-3 && st.to >= at - 1e-3).map(|st| st.r0.max(st.r1)).fold(0.0, f64::max);
        let tip = on_sheet(at, near);
        let text = format!("R{}", length_text(r));
        if let Some(b) = callout(d, ink, words, tip, 0.0, &[plain(text.clone())], cx.frame, Some(on_sheet(at, near + 30.0))) {
            d.autos.push(AutoMark { key, text, view: view.name, bounds: b, value: r });
        }
    }
}

/// Where an external thread runs along its stretch: from the free end (its
/// chamfer included) for the length given, else the whole stretch.
fn thread_run(t: &features::Turned, st: &Stretch, th: &ThreadSpec) -> (f64, f64) {
    // A chamfer next to it belongs to the thread.
    let near = |at: f64| t.outside.iter().find(|o| !o.is_cylinder() && ((o.to - at).abs() < 1e-3 || (o.from - at).abs() < 1e-3)).copied();
    let (mut from, mut to) = (st.from, st.to);
    let lead_lo = near(st.from);
    let lead_hi = near(st.to);
    if let Some(c) = lead_lo {
        from = c.from;
    }
    if let Some(c) = lead_hi {
        to = c.to;
    }
    if let Some(len) = th.depth {
        // Measured from the free end: the end with the chamfer.
        if lead_lo.is_some() || lead_hi.is_none() {
            to = (from + len).min(to);
        } else {
            from = (to - len).max(from);
        }
    }
    (from, to)
}

/// A diameter dimension straight across a profile, from `pa` to `pb` (sheet
/// mm): arrows on the outline, the value beside the line between the axis
/// and the outline, or past the end when the stretch is too short for it.
fn across(d: &mut Drawing, ink: &mut Grid, pa: [f64; 2], pb: [f64; 2], label: &Label, room: f64) {
    let l = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
    let u = [(pb[0] - pa[0]) / l, (pb[1] - pa[1]) / l];
    let w = annotate::label_width(label, H);
    let mut angle = u[1].atan2(u[0]).to_degrees();
    if angle > 90.0 + 1e-6 {
        angle -= 180.0;
    } else if angle <= -90.0 + 1e-6 {
        angle += 180.0;
    }
    let read = [angle.to_radians().cos(), angle.to_radians().sin()];
    let up = [-read[1], read[0]];
    // Which end the text reads towards.
    let (near, far) = if (pb[0] - pa[0]) * read[0] + (pb[1] - pa[1]) * read[1] >= 0.0 { (pa, pb) } else { (pb, pa) };
    let half = l / 2.0;
    let _ = ink;
    if l >= 8.0 {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![pa, pb] });
        arrow(d, pa, [-u[0], -u[1]]);
        arrow(d, pb, u);
    } else {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[pa[0] - u[0] * 5.0, pa[1] - u[1] * 5.0], [pb[0] + u[0] * 5.0, pb[1] + u[1] * 5.0]] });
        arrow(d, pa, u);
        arrow(d, pb, [-u[0], -u[1]]);
    }
    // Beside the line: on the side the stretch has more room (its narrow side shows past it).
    let beside = H * 0.35 + 0.9;
    let fits_half = w + 3.0 <= half && room >= 2.0 * (H + 1.0);
    if fits_half {
        // In the outer half, centred between the axis and the outline.
        let c = [near[0] + (far[0] - near[0]) * 0.72, near[1] + (far[1] - near[1]) * 0.72];
        annotate::place_label(d, label, [c[0] + up[0] * beside, c[1] + up[1] * beside], Anchor::Middle, angle, H);
    } else {
        // Past the far end, the line carried on under it.
        let start = [far[0] + read[0] * 6.0, far[1] + read[1] * 6.0];
        let tail = [start[0] + read[0] * (w + 1.0), start[1] + read[1] * (w + 1.0)];
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![far, tail] });
        annotate::place_label(d, label, [start[0] + up[0] * 1.2, start[1] + up[1] * 1.2], Anchor::Start, angle, H);
    }
}

/// Hole callouts: one per size (with how many), in the view that shows them round.
fn holes(d: &mut Drawing, ink: &mut Grid, words: &mut Grid, cx: &Ctx) {
    let f = cx.feats;
    let mut groups: Vec<(String, usize, Vec<&Hole>)> = vec![];
    for h in &f.holes {
        let Some(a) = h.axis() else { continue };
        let sig = format!("{a}|{}", h.signature());
        match groups.iter_mut().find(|g| g.0 == sig) {
            Some(g) => g.2.push(h),
            None => groups.push((sig, a, vec![h])),
        }
    }
    // Larger holes first: they get the best spots.
    groups.sort_by(|x, y| y.2[0].diameter.total_cmp(&x.2[0].diameter));
    let sizes: Vec<f64> = groups.iter().map(|g| g.2[0].diameter).collect();
    for (_, a, hs) in groups {
        // By its size; by its depth too when two kinds of hole share it.
        let shared = sizes.iter().filter(|d| (**d - hs[0].diameter).abs() < 1e-3).count() > 1;
        let key = if shared { format!("hole:{}@{}", key_value(hs[0].diameter), if hs[0].through { "thru".to_string() } else { key_value(hs[0].depth) }) } else { format!("hole:{}", key_value(hs[0].diameter)) };
        let lines = hole_lines(hs[0], hs.len(), cx.options);
        // Cosmetic threads first (ISO 6410-1): a thin three-quarter circle at the
        // thread's major diameter where the hole shows round, thin (or hidden)
        // lines along it in the other views.
        if let Some(th) = thread_for(cx.options, hs[0].diameter) {
            if let Ok((nominal, _)) = features::parse_thread(&th.size) {
                let len = th.depth.filter(|_| !hs[0].through).unwrap_or(hs[0].depth);
                for v in &cx.views {
                    for h in &hs {
                        if v.look() == a {
                            let c = v.sheet(h.entry, cx.s);
                            let r = nominal / 2.0 * cx.s;
                            let pts: Vec<[f64; 2]> = (0..=36).map(|j| j as f64 / 36.0 * 1.5 * std::f64::consts::PI + std::f64::consts::FRAC_PI_2).map(|t| [c[0] + r * t.cos(), c[1] + r * t.sin()]).collect();
                            ink.mark([c[0] - r, c[1] - r, c[0] + r, c[1] + r]);
                            d.strokes.push(Stroke { pen: Pen::Thin, points: pts });
                        } else {
                            // Seen from the side: the thread's extent at its major diameter.
                            let across = (0..3).find(|&k| k != a && k != v.look()).unwrap();
                            let cut = v.section.is_some_and(|y| (h.entry[v.look()] - y).abs() < nominal / 2.0);
                            let pen = if cut { Pen::Thin } else { Pen::Hidden };
                            for side in [-1.0, 1.0] {
                                let mut p = h.entry;
                                p[across] += side * nominal / 2.0;
                                let q = [p[0] + h.inward[0] * len, p[1] + h.inward[1] * len, p[2] + h.inward[2] * len];
                                d.strokes.push(Stroke { pen, points: vec![v.sheet(p, cx.s), v.sheet(q, cx.s)] });
                            }
                        }
                    }
                }
            }
        }
        if hidden(cx.options, &key) {
            continue;
        }
        let Some(view) = cx.views.iter().find(|v| v.look() == a).or_else(|| cx.views.first()) else { continue };
        let round = view.look() == a;
        // Try each hole of the size, nearest the view's edge first.
        let mut order: Vec<&&Hole> = hs.iter().collect();
        let centre = [(view.bounds[0] + view.bounds[2]) / 2.0, (view.bounds[1] + view.bounds[3]) / 2.0];
        order.sort_by(|x, y| {
            let (px, py) = (view.sheet(x.entry, cx.s), view.sheet(y.entry, cx.s));
            let dx = (px[0] - centre[0]).abs().max(px[1] - centre[1]);
            let dy = (py[0] - centre[0]).abs().max(py[1] - centre[1]);
            dy.total_cmp(&dx)
        });
        let mut placed = None;
        for h in order {
            let c = view.sheet(h.entry, cx.s);
            let r = h.stretches.iter().map(|s| s.r0.max(s.r1)).fold(0.0, f64::max) * cx.s;
            let outward = [c[0] + (c[0] - centre[0]) * 4.0, c[1] + (c[1] - centre[1]) * 4.0];
            // Round: the arrow on the circle, pointing at its centre. From the side: on the hole's end.
            let (tip, away) = if round { (c, r) } else { (c, 0.0) };
            if let Some(b) = callout_round(d, ink, words, tip, away, &lines, cx.frame, outward) {
                placed = Some((b, c));
                break;
            }
        }
        match placed {
            Some((b, c)) => {
                d.annotations.push(AnnotationMark { kind: MarkKind::Hole, index: 0, diameter: Some((hs[0].diameter * 1000.0).round() / 1000.0), bounds: b, origin: None, anchor: Some(c) });
                d.autos.push(AutoMark { key, text: lines[0].main.clone(), view: view.name, bounds: b, value: hs[0].diameter });
            }
            None => d.warnings.push(format!("no room for the ⌀{} hole callout", length_text(hs[0].diameter))),
        }
    }
}

/// A callout whose leader points at a circle's centre, its arrow on the circle.
#[allow(clippy::too_many_arguments)]
fn callout_round(d: &mut Drawing, ink: &mut Grid, words: &mut Grid, c: [f64; 2], r: f64, lines: &[Label], frame: [f64; 4], prefer: [f64; 2]) -> Option<[f64; 4]> {
    callout(d, ink, words, c, r, lines, frame, Some(prefer))
}

/// Cutter radii and rounded edges: one callout per radius, with how many.
fn rounds(d: &mut Drawing, ink: &mut Grid, words: &mut Grid, cx: &Ctx) {
    let f = cx.feats;
    if f.turned.is_some() {
        return;
    }
    let mut groups: Vec<(f64, usize, Vec<&features::Round>)> = vec![];
    for r in &f.rounds {
        match groups.iter_mut().find(|g| (g.0 - r.radius).abs() < 1e-3 && g.1 == r.axis) {
            Some(g) => g.2.push(r),
            None => groups.push((r.radius, r.axis, vec![r])),
        }
    }
    let radii: Vec<f64> = groups.iter().map(|g| g.0).collect();
    for (radius, axis, rs) in groups {
        let shared = radii.iter().filter(|r| (**r - radius).abs() < 1e-3).count() > 1;
        let key = if shared { format!("r:{}@{}", key_value(radius), AXIS[axis]) } else { format!("r:{}", key_value(radius)) };
        if hidden(cx.options, &key) {
            continue;
        }
        let Some(view) = cx.views.iter().find(|v| v.look() == axis) else { continue };
        if radius * cx.s < 0.8 {
            continue;
        }
        let n = rs.len();
        let text = if n > 1 { format!("{n}× R{}", length_text(radius)) } else { format!("R{}", length_text(radius)) };
        let mut placed = None;
        for r in &rs {
            let tip = view.sheet(r.mid, cx.s);
            let c = view.sheet(r.centre, cx.s);
            // Out from the arc's centre through its middle.
            let out = [tip[0] + (tip[0] - c[0]) * 6.0, tip[1] + (tip[1] - c[1]) * 6.0];
            if let Some(b) = callout(d, ink, words, tip, 0.0, &[plain(text.clone())], cx.frame, Some(out)) {
                placed = Some(b);
                break;
            }
        }
        match placed {
            Some(b) => d.autos.push(AutoMark { key, text, view: view.name, bounds: b, value: radius }),
            None => d.warnings.push(format!("no room for the R{} callout", length_text(radius))),
        }
    }
}
