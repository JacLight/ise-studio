//! Panelization: copies of a board in a grid on one fab panel, held by tabs
//! broken along rows of mouse-bite holes, or butted together and V-scored;
//! with rails or a frame carrying fiducials and tooling holes for the
//! assembly line. It follows what KiKit (MIT) does for a grid panel: the
//! panel's substrate is the union of the boards, the tabs and the frame, its
//! profile is that union's outline, the mouse bites sit on each tab where it
//! meets a board (shifted into the board by an offset so the broken edge is
//! clean), V-cuts run edge to edge on their own layer.
//!
//! The panel's fabrication files are the board's own files with every copy
//! moved into place (coordinates translated; apertures and attributes as
//! written), plus the panel's profile, fiducials (copper and mask opening),
//! mouse bites and tooling holes (unplated drill), and the V-cut lines; the
//! assembly files list every copy's parts with designators made unique.

use crate::board::BoardView;
use crate::geom::{self, P};
use crate::{Board, gerber, outputs, reader};
use manifold_csg::CrossSection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Spec {
    pub rows: usize,
    pub cols: usize,
    /// Milled gap between boards and between boards and rails (mm); V-scored panels butt them (0).
    pub gap: f64,
    /// "mousebites", "vscore" or "tabs" (solid tabs, cut by hand or router).
    pub separation: String,
    /// Tab width along the board edge, and tabs per board edge.
    pub tab_width: f64,
    pub tabs: usize,
    pub bite_drill: f64,
    pub bite_spacing: f64,
    /// How far the bites sit inside the board (mm).
    pub bite_offset: f64,
    /// "none", "rails-tb" (top and bottom), "rails-lr" (left and right) or "frame".
    pub frame: String,
    pub rail: f64,
    pub fiducials: bool,
    pub fiducial_copper: f64,
    pub fiducial_opening: f64,
    pub tooling: bool,
    pub tooling_hole: f64,
}

impl Default for Spec {
    fn default() -> Spec {
        Spec {
            rows: 2,
            cols: 2,
            gap: 2.0,
            separation: "mousebites".into(),
            tab_width: 3.0,
            tabs: 1,
            bite_drill: 0.5,
            bite_spacing: 0.8,
            bite_offset: 0.25,
            frame: "rails-tb".into(),
            rail: 5.0,
            fiducials: true,
            fiducial_copper: 1.0,
            fiducial_opening: 2.0,
            tooling: true,
            tooling_hole: 1.5,
        }
    }
}

impl Spec {
    /// From query or CLI words (`rows=2 cols=3 separation=vscore …`); unknown keys are an error.
    pub fn from_pairs(pairs: &BTreeMap<String, String>) -> Result<Spec, String> {
        let mut v = serde_json::to_value(Spec::default()).unwrap();
        let m = v.as_object_mut().unwrap();
        for (k, val) in pairs {
            if k == "project" || k == "fab" || k == "download" || k == "format" {
                continue;
            }
            let Some(slot) = m.get_mut(k.as_str()) else { return Err(format!("no panel setting “{k}”: {}", Spec::keys().join(", "))) };
            *slot = match slot {
                serde_json::Value::Bool(_) => serde_json::Value::Bool(matches!(val.as_str(), "1" | "true" | "yes" | "on")),
                serde_json::Value::String(_) => serde_json::Value::String(val.clone()),
                _ => serde_json::from_str(val).map_err(|_| format!("{k}: “{val}” is not a number"))?,
            };
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    pub fn keys() -> Vec<String> {
        serde_json::to_value(Spec::default()).unwrap().as_object().unwrap().keys().cloned().collect()
    }

    fn vscore(&self) -> bool {
        self.separation == "vscore"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Tab {
    /// [min x, min y, max x, max y].
    pub rect: [f64; 4],
    /// Where each end meets a board: the line (two points) the bites run along.
    pub cuts: Vec<[P; 2]>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Panel {
    pub spec: Spec,
    /// Panel size (mm); its lower-left corner is the origin.
    pub size: [f64; 2],
    /// Where each copy's board coordinates move to: panel = board + offset.
    pub offsets: Vec<P>,
    pub board_size: [f64; 2],
    /// The substrate's contours (outer counter-clockwise, holes clockwise).
    pub outline: Vec<Vec<P>>,
    pub rails: Vec<[f64; 4]>,
    pub tabs: Vec<Tab>,
    pub bites: Vec<P>,
    pub vcuts: Vec<[P; 2]>,
    pub fiducials: Vec<P>,
    pub tooling: Vec<P>,
    pub area: f64,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

fn bbox(pts: &[P]) -> [f64; 4] {
    pts.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

fn rect_poly(r: [f64; 4]) -> Vec<P> {
    vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

/// Lays out a panel of a board given by its outline and cutouts.
pub fn layout(outline: &[P], cutouts: &[Vec<P>], spec: &Spec) -> Result<Panel, String> {
    if outline.len() < 3 {
        return Err("the board has no outline".into());
    }
    if spec.rows == 0 || spec.cols == 0 || spec.rows * spec.cols > 400 {
        return Err("rows and columns must be at least 1 (and at most 400 boards)".into());
    }
    let vs = spec.vscore();
    if !matches!(spec.separation.as_str(), "mousebites" | "vscore" | "tabs") {
        return Err(format!("separation “{}”: mousebites, vscore or tabs", spec.separation));
    }
    if !matches!(spec.frame.as_str(), "none" | "rails-tb" | "rails-lr" | "frame") {
        return Err(format!("frame “{}”: none, rails-tb, rails-lr or frame", spec.frame));
    }
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let g = if vs { 0.0 } else { spec.gap.max(0.0) };
    if !vs && g < 1.0 {
        problems.push(format!("a {g} mm gap is narrower than a fab's smallest router bit (about 1 mm)"));
    }
    let b = bbox(outline);
    let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
    let tb = spec.frame == "rails-tb" || spec.frame == "frame";
    let lr = spec.frame == "rails-lr" || spec.frame == "frame";
    let r = spec.rail.max(0.0);
    let x0 = if lr { r + g } else { 0.0 };
    let y0 = if tb { r + g } else { 0.0 };
    let w_in = spec.cols as f64 * bw + (spec.cols - 1) as f64 * g;
    let h_in = spec.rows as f64 * bh + (spec.rows - 1) as f64 * g;
    let (pw, ph) = (w_in + if lr { 2.0 * (r + g) } else { 0.0 }, h_in + if tb { 2.0 * (r + g) } else { 0.0 });
    let mut offsets = Vec::new();
    let mut cells = Vec::new(); // each copy's bounding box in the panel
    for i in 0..spec.rows {
        for j in 0..spec.cols {
            let ll = [x0 + j as f64 * (bw + g), y0 + i as f64 * (bh + g)];
            offsets.push([ll[0] - b[0], ll[1] - b[1]]);
            cells.push([ll[0], ll[1], ll[0] + bw, ll[1] + bh]);
        }
    }
    let mut rails = Vec::new();
    if tb {
        rails.push([0.0, 0.0, pw, r]);
        rails.push([0.0, ph - r, pw, ph]);
    }
    if lr {
        rails.push([0.0, 0.0, r, ph]);
        rails.push([pw - r, 0.0, pw, ph]);
    }
    // Tabs across each gap: between neighbours, and from the outer boards to the rails.
    let mut tabs = Vec::new();
    let n = spec.tabs.max(1);
    let tw = spec.tab_width;
    if !vs && g > 0.0 {
        if tw <= 0.0 {
            return Err("tab width must be more than 0".into());
        }
        let into = 0.5_f64.min(bw / 4.0).min(bh / 4.0); // how far a tab reaches into what it joins
        // Vertical gaps (tabs run in x), from x = a to x = a + g, over the y range [lo, hi].
        let across_x = |a: f64, lo: f64, hi: f64, left_board: bool, right_board: bool, tabs: &mut Vec<Tab>| {
            for k in 0..n {
                let y = lo + (hi - lo) * (k as f64 + 0.5) / n as f64;
                let mut cuts = Vec::new();
                if left_board {
                    cuts.push([[a, y - tw / 2.0], [a, y + tw / 2.0]]);
                }
                if right_board {
                    cuts.push([[a + g, y - tw / 2.0], [a + g, y + tw / 2.0]]);
                }
                tabs.push(Tab { rect: [a - into, y - tw / 2.0, a + g + into, y + tw / 2.0], cuts });
            }
        };
        for i in 0..spec.rows {
            for j in 0..spec.cols {
                let c = cells[i * spec.cols + j];
                if j + 1 < spec.cols {
                    across_x(c[2], c[1], c[3], true, true, &mut tabs);
                }
                if lr && j == 0 {
                    across_x(c[0] - g, c[1], c[3], false, true, &mut tabs);
                }
                if lr && j + 1 == spec.cols {
                    across_x(c[2], c[1], c[3], true, false, &mut tabs);
                }
            }
        }
        let across_y = |a: f64, lo: f64, hi: f64, low_board: bool, high_board: bool, tabs: &mut Vec<Tab>| {
            for k in 0..n {
                let x = lo + (hi - lo) * (k as f64 + 0.5) / n as f64;
                let mut cuts = Vec::new();
                if low_board {
                    cuts.push([[x - tw / 2.0, a], [x + tw / 2.0, a]]);
                }
                if high_board {
                    cuts.push([[x - tw / 2.0, a + g], [x + tw / 2.0, a + g]]);
                }
                tabs.push(Tab { rect: [x - tw / 2.0, a - into, x + tw / 2.0, a + g + into], cuts });
            }
        };
        for i in 0..spec.rows {
            for j in 0..spec.cols {
                let c = cells[i * spec.cols + j];
                if i + 1 < spec.rows {
                    across_y(c[3], c[0], c[2], true, true, &mut tabs);
                }
                if tb && i == 0 {
                    across_y(c[1] - g, c[0], c[2], false, true, &mut tabs);
                }
                if tb && i + 1 == spec.rows {
                    across_y(c[3], c[0], c[2], true, false, &mut tabs);
                }
            }
        }
        if tw * n as f64 > bw.min(bh) * 0.8 {
            problems.push(format!("{n} tabs of {tw} mm crowd a {:.1} mm edge", bw.min(bh)));
        }
    }
    // Every tab end must land on its board (the outline may not fill its bounding box).
    let inside = |p: P| geom::in_polygon(p, outline);
    let mut floating = 0;
    for t in &tabs {
        for cut in &t.cuts {
            let mid = [(cut[0][0] + cut[1][0]) / 2.0, (cut[0][1] + cut[1][1]) / 2.0];
            // Back into board coordinates of whichever copy holds it.
            let held = offsets.iter().any(|o| {
                let q = [mid[0] - o[0], mid[1] - o[1]];
                // Probe across the cut line, on both sides.
                let d = if (cut[0][0] - cut[1][0]).abs() < 1e-9 { [0.3, 0.0] } else { [0.0, 0.3] };
                inside([q[0] + d[0], q[1] + d[1]]) || inside([q[0] - d[0], q[1] - d[1]])
            });
            if !held {
                floating += 1;
            }
        }
    }
    if floating > 0 {
        problems.push(format!("{floating} tab ends do not reach a board edge: tabs sit on the bounding box, so this outline needs another layout"));
    }
    // Mouse bites along each cut, shifted into the board.
    let mut bites = Vec::new();
    if spec.separation == "mousebites" {
        if spec.bite_drill <= 0.0 || spec.bite_spacing < spec.bite_drill {
            return Err("mouse-bite spacing must be at least the drill".into());
        }
        for t in &tabs {
            let centre = [(t.rect[0] + t.rect[2]) / 2.0, (t.rect[1] + t.rect[3]) / 2.0];
            for cut in &t.cuts {
                let along = geom::sub(cut[1], cut[0]);
                let len = geom::len(along);
                let u = [along[0] / len, along[1] / len];
                let mid = [(cut[0][0] + cut[1][0]) / 2.0, (cut[0][1] + cut[1][1]) / 2.0];
                // Into the board: away from the tab's centre.
                let away = geom::sub(mid, centre);
                let nrm = if along[0].abs() > along[1].abs() { [0.0, away[1].signum()] } else { [away[0].signum(), 0.0] };
                let count = (len / spec.bite_spacing).floor() as usize + 1;
                let start = -(count as f64 - 1.0) * spec.bite_spacing / 2.0;
                for k in 0..count {
                    let s = start + k as f64 * spec.bite_spacing;
                    bites.push([mid[0] + u[0] * s + nrm[0] * spec.bite_offset, mid[1] + u[1] * s + nrm[1] * spec.bite_offset]);
                }
            }
        }
    }
    // V-cuts: every line between rows and columns, and between the boards and butted rails.
    let mut vcuts = Vec::new();
    if vs {
        let mut xs: Vec<f64> = (1..spec.cols).map(|j| x0 + j as f64 * bw).collect();
        let mut ys: Vec<f64> = (1..spec.rows).map(|i| y0 + i as f64 * bh).collect();
        if lr {
            xs.extend([r, pw - r]);
        }
        if tb {
            ys.extend([r, ph - r]);
        }
        for x in xs {
            vcuts.push([[x, 0.0], [x, ph]]);
        }
        for y in ys {
            vcuts.push([[0.0, y], [pw, y]]);
        }
        let area = geom::polygon_area(outline).abs();
        if area < 0.98 * bw * bh {
            problems.push("V-scoring cuts in straight lines edge to edge: this board is not a rectangle, so its shape would need routing (use mouse bites)".into());
        }
    }
    // Fiducials and tooling holes on the rails.
    let mut fiducials = Vec::new();
    let mut tooling = Vec::new();
    if (spec.fiducials || spec.tooling) && rails.is_empty() {
        notes.push("no rails: fiducials and tooling holes need a frame or rails, so there are none".into());
    } else if !rails.is_empty() {
        let need = spec.fiducial_opening.max(spec.tooling_hole) + 1.0;
        if r < need {
            problems.push(format!("{r} mm rails are too narrow for the fiducials and tooling holes ({need} mm)"));
        }
        // Along the rails' length: tooling holes 5 mm from the ends, fiducials 10 mm in; three fiducials, not symmetric,
        // so the machine sees which way round the panel is.
        let (len, at): (f64, Box<dyn Fn(f64, bool) -> P>) = if tb { (pw, Box::new(move |s, top| [s, if top { ph - r / 2.0 } else { r / 2.0 }])) } else { (ph, Box::new(move |s, right| [if right { pw - r / 2.0 } else { r / 2.0 }, s])) };
        if spec.tooling {
            for (s, far) in [(5.0, false), (len - 5.0, false), (5.0, true), (len - 5.0, true)] {
                tooling.push(at(s, far));
            }
        }
        if spec.fiducials {
            for (s, far) in [(12.0, false), (len - 12.0, false), (12.0, true)] {
                fiducials.push(at(s, far));
            }
        }
    }
    // The substrate.
    let mut board_cs = CrossSection::from_polygons(&[outline.to_vec()]);
    for c in cutouts {
        let mut ccw = c.clone();
        if geom::polygon_area(&ccw) < 0.0 {
            ccw.reverse();
        }
        board_cs = board_cs.difference(&CrossSection::from_polygons(&[ccw]));
    }
    let mut parts: Vec<CrossSection> = offsets.iter().map(|o| board_cs.translate(o[0], o[1])).collect();
    parts.extend(rails.iter().map(|r| CrossSection::from_polygons(&[rect_poly(*r)])));
    // A tab joins only where it covers gap (the part inside the boards adds nothing, and stays off their cutouts).
    for t in &tabs {
        parts.push(CrossSection::from_polygons(&[rect_poly(t.rect)]));
    }
    let substrate = CrossSection::batch_union(&parts);
    // Tab overlaps into a board must not fill a cutout: take away each copy's cutouts again.
    let mut substrate = substrate;
    for o in &offsets {
        for c in cutouts {
            let mut ccw = c.clone();
            if geom::polygon_area(&ccw) < 0.0 {
                ccw.reverse();
            }
            substrate = substrate.difference(&CrossSection::from_polygons(&[ccw]).translate(o[0], o[1]));
        }
    }
    let area = substrate.area();
    let outline_out = geom::contours(&substrate);
    let pieces = outline_out.iter().filter(|c| geom::polygon_area(c) > 0.0).count();
    if pieces > 1 {
        problems.push(format!("the panel falls apart into {pieces} pieces: add rails, tabs or V-scoring"));
    }
    if pw > 500.0 || ph > 500.0 {
        notes.push(format!("{pw:.0} × {ph:.0} mm is larger than most fabs' panel size"));
    }
    if spec.frame == "none" {
        notes.push("an assembly line needs rails to hold the panel (about 5 mm, top and bottom)".into());
    }
    Ok(Panel { spec: spec.clone(), size: [pw, ph], offsets, board_size: [bw, bh], outline: outline_out, rails, tabs, bites, vcuts, fiducials, tooling, area, problems, notes })
}

// ---- Files ---------------------------------------------------------------------------

/// Gerber coordinates of this crate's writer: 4.6, mm.
fn gc(v: f64) -> i64 {
    (v * 1e6).round() as i64
}

/// Moves every X and Y coordinate of one body line by (dx, dy) in file units (I and J are relative: unchanged).
fn shift_line(line: &str, dx: i64, dy: i64) -> String {
    if line.starts_with('%') || line.starts_with("G04") {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + 8);
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let ch = b[i] as char;
        if ch == 'X' || ch == 'Y' {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
                j += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if let Ok(v) = line[i + 1..j].parse::<i64>() {
                let _ = write!(out, "{ch}{}", v + if ch == 'X' { dx } else { dy });
                i = j;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// A Gerber file from this crate's writer split into header (format, attributes, apertures) and body.
fn split(text: &str) -> (Vec<&str>, Vec<&str>, u32) {
    let lines: Vec<&str> = text.lines().collect();
    let mut end = 0;
    let mut max_d = 9;
    for (i, l) in lines.iter().enumerate() {
        if let Some(rest) = l.strip_prefix("%ADD") {
            let d: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            max_d = max_d.max(d.parse().unwrap_or(0));
            end = i + 1;
            if lines.get(i + 1) == Some(&"%TD*%") {
                end = i + 2;
            }
        }
    }
    if end == 0 {
        end = lines.iter().position(|l| *l == "G01*").map(|i| i + 1).unwrap_or(0);
    }
    let stop = lines.iter().rposition(|l| l.starts_with("M02")).unwrap_or(lines.len());
    (lines[..end].to_vec(), lines[end..stop.max(end)].to_vec(), max_d)
}

/// What the panel adds to one Gerber file.
#[derive(Default)]
struct Extra {
    flashes: Vec<(f64, &'static str, P)>,
    strokes: Vec<(f64, &'static str, Vec<P>)>,
}

fn n6(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn compose(text: &str, offsets: &[P], extra: &Extra, keep_copies: bool, note: &str) -> String {
    let (head, body, max_d) = split(text);
    let mut s = String::new();
    for l in &head {
        s.push_str(l);
        s.push('\n');
    }
    // Extra apertures after the board's own.
    let mut defs: Vec<(String, &str)> = Vec::new();
    let d_of = |def: String, f: &'static str, defs: &mut Vec<(String, &str)>| -> u32 {
        if let Some(i) = defs.iter().position(|x| x.0 == def && x.1 == f) {
            return max_d + 1 + i as u32;
        }
        defs.push((def, f));
        max_d + defs.len() as u32
    };
    let flash_d: Vec<u32> = extra.flashes.iter().map(|(dia, f, _)| d_of(format!("C,{}", n6(*dia)), f, &mut defs)).collect();
    let stroke_d: Vec<u32> = extra.strokes.iter().map(|(w, f, _)| d_of(format!("C,{}", n6(*w)), f, &mut defs)).collect();
    for (i, (def, f)) in defs.iter().enumerate() {
        let _ = writeln!(s, "%TA.AperFunction,{f}*%\n%ADD{}{def}*%\n%TD*%", max_d + 1 + i as u32);
    }
    let _ = writeln!(s, "G04 {note}*");
    if keep_copies {
        for (k, o) in offsets.iter().enumerate() {
            let _ = writeln!(s, "G04 board {} of {}*", k + 1, offsets.len());
            // Each copy starts from the same state the board's body expects.
            let _ = writeln!(s, "%LPD*%\nG01*");
            let (dx, dy) = (gc(o[0]), gc(o[1]));
            for l in &body {
                s.push_str(&shift_line(l, dx, dy));
                s.push('\n');
            }
        }
    }
    if !extra.flashes.is_empty() || !extra.strokes.is_empty() {
        let _ = writeln!(s, "G04 the panel*\n%LPD*%\nG01*");
    }
    let mut current = 0;
    for ((_, _, at), d) in extra.flashes.iter().zip(&flash_d) {
        if current != *d {
            let _ = writeln!(s, "D{d}*");
            current = *d;
        }
        let _ = writeln!(s, "X{}Y{}D03*", gc(at[0]), gc(at[1]));
    }
    for ((_, _, pts), d) in extra.strokes.iter().zip(&stroke_d) {
        if pts.len() < 2 {
            continue;
        }
        if current != *d {
            let _ = writeln!(s, "D{d}*");
            current = *d;
        }
        let _ = writeln!(s, "X{}Y{}D02*", gc(pts[0][0]), gc(pts[0][1]));
        for p in &pts[1..] {
            let _ = writeln!(s, "X{}Y{}D01*", gc(p[0]), gc(p[1]));
        }
    }
    s.push_str("M02*\n");
    s
}

/// An Excellon file of the copies' holes plus the panel's (unplated) holes.
fn drill(text: &str, offsets: &[P], extra: &[(f64, P)], plated: bool, name: &str) -> Result<String, String> {
    let d = reader::excellon(text).map_err(|e| format!("{name}: {e}"))?;
    let mut tools: Vec<f64> = d.tools.values().copied().chain(extra.iter().map(|e| e.0)).map(|t| (t * 1000.0).round() / 1000.0).collect();
    tools.sort_by(f64::total_cmp);
    tools.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let tool_of = |dia: f64| tools.iter().position(|t| (t - (dia * 1000.0).round() / 1000.0).abs() < 1e-9).unwrap() + 1;
    let f = |v: f64| format!("{v:.3}");
    let mut s = String::new();
    let _ = writeln!(s, "M48");
    let _ = writeln!(s, "; DRILL file: panel of {} boards ({})", offsets.len(), if plated { "plated" } else { "not plated" });
    let _ = writeln!(s, "; FORMAT={{-:-/ absolute / metric / decimal}}");
    let _ = writeln!(s, "; #@! TF.GenerationSoftware,ISE Studio,PCB,{}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "; #@! TF.FileFunction,{},1,2,{}", if plated { "Plated" } else { "NonPlated" }, if plated { "PTH" } else { "NPTH" });
    let _ = writeln!(s, "FMAT,2\nMETRIC");
    for (i, t) in tools.iter().enumerate() {
        let _ = writeln!(s, "T{}C{:.3}", i + 1, t);
    }
    let _ = writeln!(s, "%\nG90\nG05");
    for (i, t) in tools.iter().enumerate() {
        let mut lines = Vec::new();
        for o in offsets {
            for (tool, at) in &d.hits {
                if tool_of(d.tools[tool]) == i + 1 {
                    lines.push(format!("X{}Y{}", f(at[0] + o[0]), f(at[1] + o[1])));
                }
            }
            for (tool, a, b) in &d.slots {
                if tool_of(d.tools[tool]) == i + 1 {
                    lines.push(format!("X{}Y{}G85X{}Y{}", f(a[0] + o[0]), f(a[1] + o[1]), f(b[0] + o[0]), f(b[1] + o[1])));
                }
            }
        }
        for (dia, at) in extra {
            if tool_of(*dia) == i + 1 {
                lines.push(format!("X{}Y{}", f(at[0]), f(at[1])));
            }
        }
        if lines.is_empty() {
            continue;
        }
        let _ = writeln!(s, "T{}", i + 1);
        let _ = t;
        for l in lines {
            s.push_str(&l);
            s.push('\n');
        }
    }
    s.push_str("T0\nM30\n");
    Ok(s)
}

/// Splits one CSV line (quoted fields may hold commas).
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn csv_join(fields: &[String]) -> String {
    fields.iter().map(|f| if f.contains([',', '"', '\n']) { format!("\"{}\"", f.replace('"', "\"\"")) } else { f.clone() }).collect::<Vec<_>>().join(",")
}

/// Each copy's designator: R1 on board 3 is R1_3.
fn unique(reference: &str, k: usize) -> String {
    format!("{reference}_{k}")
}

/// The positions file with every copy's parts in panel coordinates.
fn positions(text: &str, view: &BoardView, offsets: &[P]) -> String {
    let mut lines = text.lines();
    let Some(head) = lines.next() else { return String::new() };
    let cols = csv_fields(head);
    let ix = cols.iter().position(|c| c == "Mid X" || c == "PosX");
    let iy = cols.iter().position(|c| c == "Mid Y" || c == "PosY");
    let rows: Vec<Vec<String>> = lines.map(csv_fields).collect();
    let mut s = format!("{head}\n");
    for (k, o) in offsets.iter().enumerate() {
        // Board files are relative to the board's lower-left corner; the panel's to its own.
        let base = [view.bounds[0] + o[0], view.bounds[1] + o[1]];
        for r in &rows {
            let mut r = r.clone();
            r[0] = unique(&r[0], k + 1);
            for (i, b) in [(ix, base[0]), (iy, base[1])] {
                if let Some(i) = i {
                    let mm = r[i].ends_with("mm");
                    let v: f64 = r[i].trim_end_matches("mm").parse().unwrap_or(0.0);
                    r[i] = format!("{:.4}{}", v + b, if mm { "mm" } else { "" });
                }
            }
            s.push_str(&csv_join(&r));
            s.push('\n');
        }
    }
    s
}

/// The BOM with quantities for the whole panel and every copy's designators.
fn bom(text: &str, copies: usize) -> String {
    let mut lines = text.lines();
    let Some(head) = lines.next() else { return String::new() };
    let cols = csv_fields(head);
    let iq = cols.iter().position(|c| c == "Quantity" || c == "Qty");
    let id = cols.iter().position(|c| c == "Designators" || c == "Designator" || c == "Reference");
    let mut s = format!("{head}\n");
    for l in lines {
        let mut r = csv_fields(l);
        if let Some(i) = iq.filter(|i| *i < r.len()) {
            if let Ok(q) = r[i].parse::<usize>() {
                r[i] = (q * copies).to_string();
            }
        }
        if let Some(i) = id.filter(|i| *i < r.len()) {
            let sep = if r[i].contains(',') { "," } else { " " };
            let refs: Vec<&str> = r[i].split([' ', ',']).filter(|x| !x.is_empty()).collect();
            r[i] = (1..=copies).flat_map(|k| refs.iter().map(move |x| unique(x, k))).collect::<Vec<_>>().join(sep);
        }
        s.push_str(&csv_join(&r));
        s.push('\n');
    }
    s
}

/// Every fabrication and assembly file of the panel.
pub fn files(view: &BoardView, board: &Board, panel: &Panel, fab: Option<&str>) -> Result<Vec<(String, Vec<u8>)>, String> {
    let stem = format!("{}-panel", gerber::file_stem(&view.name));
    let spec = &panel.spec;
    let note = format!("Panel of {} × {} boards ({}), {:.2} × {:.2} mm, made by ISE Studio", spec.rows, spec.cols, spec.separation, panel.size[0], panel.size[1]);
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let board_stem = gerber::file_stem(&view.name);
    for (name, text) in gerber::files(view) {
        let suffix = name.strip_prefix(&board_stem).unwrap_or(&name).to_string();
        let new_name = format!("{stem}{suffix}");
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".drl") {
            let plated = !name.contains("NPTH");
            let mut extra: Vec<(f64, P)> = Vec::new();
            if !plated {
                extra.extend(panel.bites.iter().map(|p| (spec.bite_drill, *p)));
                extra.extend(panel.tooling.iter().map(|p| (spec.tooling_hole, *p)));
            }
            out.push((new_name, drill(&text, &panel.offsets, &extra, plated, &name)?.into_bytes()));
            continue;
        }
        let function = text.lines().find_map(|l| l.strip_prefix("%TF.FileFunction,")).map(|f| f.trim_end_matches("*%").to_string()).unwrap_or_default();
        let mut extra = Extra::default();
        let mut keep = true;
        if function.starts_with("Profile") {
            keep = false;
            for c in &panel.outline {
                let mut pts = c.clone();
                pts.push(c[0]);
                extra.strokes.push((0.1, "Profile", pts));
            }
        } else if function == "Copper,L1,Top" || (function.starts_with("Copper") && function.ends_with(",Bot")) {
            for f in &panel.fiducials {
                extra.flashes.push((spec.fiducial_copper, "FiducialPad,Global", *f));
            }
        } else if function.starts_with("Soldermask") {
            for f in &panel.fiducials {
                extra.flashes.push((spec.fiducial_opening, "FiducialPad,Global", *f));
            }
        }
        out.push((new_name, compose(&text, &panel.offsets, &extra, keep, &note).into_bytes()));
    }
    if !panel.vcuts.is_empty() {
        // The V-cut lines on their own layer (X2 file function Vcut), named the way fabs ask for them.
        let profile = gerber::profile(view);
        let text = profile.replace("%TF.FileFunction,Profile,NP*%", "%TF.FileFunction,Vcut*%");
        let mut extra = Extra::default();
        for v in &panel.vcuts {
            extra.strokes.push((0.1, "Other,V-cut", vec![v[0], v[1]]));
        }
        out.push((format!("{stem}-V_Score.gbr"), compose(&text, &panel.offsets, &extra, false, &note).into_bytes()));
    }
    let format = outputs::Format::parse(fab.unwrap_or(""));
    out.push(("positions.csv".into(), positions(&outputs::pick_and_place(view, format), view, &panel.offsets).into_bytes()));
    out.push(("bom.csv".into(), bom(&outputs::bom(view, board, format), panel.offsets.len()).into_bytes()));
    out.push(("README.txt".into(), readme(view, panel).into_bytes()));
    Ok(out)
}

fn readme(view: &BoardView, p: &Panel) -> String {
    let s = &p.spec;
    let mut r = String::new();
    let _ = writeln!(r, "{} rev {}: panel of {} boards ({} rows × {} columns)", view.name, view.revision, s.rows * s.cols, s.rows, s.cols);
    let _ = writeln!(r, "Panel size: {:.2} × {:.2} mm; each board {:.2} × {:.2} mm; {} mm thick", p.size[0], p.size[1], p.board_size[0], p.board_size[1], view.thickness);
    match s.separation.as_str() {
        "vscore" => {
            let _ = writeln!(r, "Separation: V-score, {} lines, drawn in the *-V_Score.gbr layer (order with V-cut; 1/3 depth from each side)", p.vcuts.len());
        }
        "mousebites" => {
            let _ = writeln!(r, "Separation: {} tabs of {} mm with mouse bites (ø{} mm every {} mm, {} holes, in the NPTH drill file)", p.tabs.len(), s.tab_width, s.bite_drill, s.bite_spacing, p.bites.len());
        }
        _ => {
            let _ = writeln!(r, "Separation: {} solid tabs of {} mm (route or cut them)", p.tabs.len(), s.tab_width);
        }
    }
    if !p.rails.is_empty() {
        let _ = writeln!(r, "Frame: {} ({} mm); {} fiducials (ø{} mm copper in ø{} mm mask opening); {} tooling holes ø{} mm", s.frame, s.rail, p.fiducials.len(), s.fiducial_copper, s.fiducial_opening, p.tooling.len(), s.tooling_hole);
    }
    let _ = writeln!(r, "Assembly: positions.csv and bom.csv list every copy; designators end in _<board number> (board 1 is bottom left, counting along the rows).");
    let _ = writeln!(r, "Order it as a panel made by you (\"panel by customer\"), quantity in panels.");
    for x in p.problems.iter().chain(&p.notes) {
        let _ = writeln!(r, "Note: {x}");
    }
    r
}

/// The panel's files read back by the independent reader: what each holds.
pub fn readback(files: &[(String, Vec<u8>)]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        let text = String::from_utf8_lossy(bytes);
        if name.ends_with(".drl") {
            match reader::excellon(&text) {
                Ok(d) => out.push(serde_json::json!({ "file": name, "ok": true, "holes": d.hits.len() + d.slots.len() })),
                Err(e) => out.push(serde_json::json!({ "file": name, "ok": false, "error": e })),
            }
        } else if name.ends_with(".csv") || name.ends_with(".txt") {
            continue;
        } else {
            match reader::gerber(&text) {
                Ok(g) => out.push(serde_json::json!({ "file": name, "ok": true, "objects": g.objects.len(), "function": g.attributes.get("FileFunction") })),
                Err(e) => out.push(serde_json::json!({ "file": name, "ok": false, "error": e })),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, h: f64) -> Vec<P> {
        vec![[10.0, 20.0], [10.0 + w, 20.0], [10.0 + w, 20.0 + h], [10.0, 20.0 + h]]
    }

    #[test]
    fn mouse_bite_grid() {
        let spec = Spec { rows: 2, cols: 3, ..Spec::default() };
        let p = layout(&rect(30.0, 20.0), &[], &spec).unwrap();
        // 3 × 30 + 2 × 2 gaps wide; 2 × 20 + 1 gap + 2 × (5 rail + 2 gap) high.
        assert_eq!(p.size, [94.0, 56.0]);
        assert_eq!(p.offsets.len(), 6);
        assert_eq!(p.offsets[0], [-10.0, -13.0]);
        assert_eq!(p.offsets[4], [32.0 - 10.0, 7.0 + 22.0 - 20.0]);
        // Tabs: 2 rows × 2 inner gaps across x, 3 columns × 1 inner gap across y, plus 3 + 3 to the rails.
        assert_eq!(p.tabs.len(), 4 + 3 + 6);
        // Board-to-board tabs are cut at both ends, board-to-rail at one: 7 × 2 + 6 = 20 cuts,
        // each a 3 mm tab with bites every 0.8 mm: 4 holes.
        assert_eq!(p.bites.len(), 20 * 4);
        // One piece; the substrate is the boards, rails and the tabs' share of the gaps.
        assert_eq!(p.outline.iter().filter(|c| geom::polygon_area(c) > 0.0).count(), 1);
        let want = 6.0 * 600.0 + 2.0 * 94.0 * 5.0 + 13.0 * 3.0 * 2.0;
        assert!((p.area - want).abs() < 1e-6, "{} vs {want}", p.area);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert_eq!(p.fiducials.len(), 3);
        assert_eq!(p.tooling.len(), 4);
        // Bites sit 0.25 mm inside a board.
        let b = p.bites[0];
        let inside = p.offsets.iter().any(|o| geom::in_polygon([b[0] - o[0], b[1] - o[1]], &rect(30.0, 20.0)));
        assert!(inside, "{b:?}");
    }

    #[test]
    fn v_score_grid() {
        let spec = Spec { rows: 3, cols: 2, separation: "vscore".into(), frame: "frame".into(), ..Spec::default() };
        let p = layout(&rect(40.0, 25.0), &[], &spec).unwrap();
        assert_eq!(p.size, [90.0, 85.0]);
        assert!(p.tabs.is_empty() && p.bites.is_empty());
        // One line between the columns, two between the rows, and one at each of the four rails.
        assert_eq!(p.vcuts.len(), 1 + 2 + 4);
        assert!((p.area - 90.0 * 85.0).abs() < 1e-6);
        assert_eq!(p.outline.len(), 1);
    }

    #[test]
    fn falls_apart_without_rails_or_tabs() {
        let spec = Spec { rows: 1, cols: 2, separation: "mousebites".into(), frame: "none".into(), ..Spec::default() };
        let p = layout(&rect(30.0, 20.0), &[], &spec).unwrap();
        assert!(p.problems.is_empty(), "{:?}", p.problems); // the tab between them holds them
        assert!(!p.notes.is_empty());
        let spec = Spec { tab_width: 3.0, gap: 0.5, ..spec };
        assert!(!layout(&rect(30.0, 20.0), &[], &spec).unwrap().problems.is_empty());
    }

    #[test]
    fn shifting_coordinates() {
        assert_eq!(shift_line("X1000Y-2000D01*", 5, 7), "X1005Y-1993D01*");
        assert_eq!(shift_line("X1000Y2000I30J40D01*", 1, 1), "X1001Y2001I30J40D01*");
        assert_eq!(shift_line("%TO.N,X1*%", 5, 5), "%TO.N,X1*%");
        assert_eq!(shift_line("D10*", 5, 5), "D10*");
    }

    #[test]
    fn panel_csv() {
        assert_eq!(csv_fields("a,\"b,c\",d"), vec!["a", "b,c", "d"]);
        let b = bom("Line,Quantity,Designators,Value\n1,2,C1 C2,10uF\n", 2);
        assert_eq!(b.lines().nth(1).unwrap(), "1,4,C1_1 C2_1 C1_2 C2_2,10uF");
    }
}
