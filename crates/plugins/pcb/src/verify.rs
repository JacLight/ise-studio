//! Footprints checked pad by pad against KiCad's official library.
//!
//! Every built-in footprint has a KiCad equivalent (same package, same pin
//! numbering). Both are read into Iṣẹ́ Studio's coordinates (y up), the KiCad one
//! moved and turned onto Iṣẹ́ Studio's origin and zero orientation by the pads that
//! share numbers (KiCad puts some connectors' origins at pin 1 or a mounting
//! peg; Iṣẹ́ Studio at the package centre), then each pad is paired with the
//! nearest one of the same number and compared: kind (SMD, plated, bare
//! hole), position, size, shape and drill.
//!
//! What a mismatch means:
//! - **error**: a pad missing or extra, a different kind, a hole more than
//!   0.05 mm off, a plated pad more than 0.05 mm from where KiCad puts it, or
//!   an SMD land that overlaps KiCad's by less than 75 %. A part made to that
//!   footprint may not fit.
//! - **warning**: a land 0.1 mm or more longer or shorter (and so its centre
//!   moved, still overlapping), a different shape, a hole up to 0.05 mm off,
//!   or the same copper under another pad number. Both libraries follow
//!   IPC-7351 or the maker's drawing; they round differently, and either
//!   solders. Listed so the choice is visible.
//! - **note**: where the origin or zero rotation differs (pick-and-place files
//!   use Iṣẹ́ Studio's, from the board), several pads against one (a thermal pad's
//!   squares and vias), and differences checked against the maker's drawing
//!   where Iṣẹ́ Studio follows the maker ([`REVIEWED`]).

use crate::footprints::{self, Footprint, Pad, PadKind, PadShape};
use crate::geom;
use crate::kicad_lib::Library;
use serde::Serialize;

/// The KiCad library footprint for a built-in one.
pub fn kicad_equivalent(name: &str) -> Option<String> {
    let chip = |lib: &str, prefix: &str, code: &str| {
        let metric = match code {
            "0402" => "1005",
            "0603" => "1608",
            "0805" => "2012",
            "1206" => "3216",
            _ => return None,
        };
        Some(format!("{lib}:{prefix}_{code}_{metric}Metric"))
    };
    if let Some(code) = name.strip_prefix("R_") {
        return chip("Resistor_SMD", "R", code);
    }
    if let Some(code) = name.strip_prefix("C_") {
        return chip("Capacitor_SMD", "C", code);
    }
    if let Some(code) = name.strip_prefix("LED_").filter(|c| c.len() == 4 && c.chars().all(|x| x.is_ascii_digit())) {
        return chip("LED_SMD", "LED", code);
    }
    if let Some(n) = name.strip_prefix("JST_PH_").and_then(|n| n.parse::<usize>().ok()) {
        return Some(format!("Connector_JST:JST_PH_B{n}B-PH-K_1x{n:02}_P2.00mm_Vertical"));
    }
    if let Some(n) = name.strip_prefix("JST_XH_").and_then(|n| n.parse::<usize>().ok()) {
        return Some(format!("Connector_JST:JST_XH_B{n}B-XH-A_1x{n:02}_P2.50mm_Vertical"));
    }
    if let Some(rest) = name.strip_prefix("PinHeader_").and_then(|r| r.strip_suffix("_2.54")) {
        return Some(format!("Connector_PinHeader_2.54mm:PinHeader_{rest}_P2.54mm_Vertical"));
    }
    Some(
        match name {
            "SOT-23" => "Package_TO_SOT_SMD:SOT-23",
            "SOT-23-5" => "Package_TO_SOT_SMD:SOT-23-5",
            "SOT-223" => "Package_TO_SOT_SMD:SOT-223-3_TabPin2",
            "SOIC-8" => "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
            "TSSOP-20" => "Package_SO:TSSOP-20_4.4x6.5mm_P0.65mm",
            "QFN-32_5x5" => "Package_DFN_QFN:QFN-32-1EP_5x5mm_P0.5mm_EP3.45x3.45mm",
            "USB-C_16P" => "Connector_USB:USB_C_Receptacle_HRO_TYPE-C-31-M-12",
            "ESP32-WROOM-32" => "RF_Module:ESP32-WROOM-32E",
            "SW_Tactile_6x6" => "Button_Switch_THT:SW_PUSH_6mm",
            "LED_3mm" => "LED_THT:LED_D3.0mm",
            "LED_5mm" => "LED_THT:LED_D5.0mm",
            "MountingHole_M2" => "MountingHole:MountingHole_2.2mm_M2",
            "MountingHole_M25" => "MountingHole:MountingHole_2.7mm_M2.5",
            "MountingHole_M3" => "MountingHole:MountingHole_3.2mm_M3",
            "MountingHole_M3_Pad" => "MountingHole:MountingHole_3.2mm_M3_Pad",
            _ => return None,
        }
        .to_string(),
    )
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    /// error, warning or note.
    pub severity: &'static str,
    /// Pad number ("" for the footprint as a whole).
    pub pad: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FootprintReport {
    pub footprint: String,
    pub kicad: String,
    /// Where KiCad's origin is in Iṣẹ́ Studio's footprint (mm), and how far KiCad's is turned (° counter-clockwise).
    pub offset: [f64; 2],
    pub rotation: f64,
    pub pads: usize,
    pub kicad_pads: usize,
    /// Largest pad-centre distance after alignment (mm).
    pub worst_position: f64,
    pub findings: Vec<Finding>,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub checked: usize,
    pub errors: usize,
    pub warnings: usize,
    /// Footprints with no KiCad file (named, not compared).
    pub missing: Vec<String>,
    pub footprints: Vec<FootprintReport>,
    pub library: String,
}

/// Differences checked against the maker's own drawing, where Iṣẹ́ Studio follows the maker and KiCad
/// differs: (footprint, pad, start of the finding, why). They are reported as notes.
const REVIEWED: [(&str, &str, &str, &str); 3] = [
    ("USB-C_16P", "S1", "hole 0.6 × 1.4", "HRO's TYPE-C-31-M-12 drawing (2020-12-08) recommends 0.6 × 1.4 mm rear slots in 1.7 mm pads; KiCad's footprint has 1.2"),
    ("USB-C_16P", "hole", "hole ø0.6", "HRO's drawing recommends ø0.60 locating holes"),
    ("SW_Tactile_6x6", "*", "hole ø1 mm", "Omron's B3F datasheet: four ø1.0 ±0.1 mm holes; KiCad's generic SW_PUSH_6mm has 1.1"),
];

const POS_TOL: f64 = 0.05;
const DRILL_TOL: f64 = 0.05;
const SIZE_TOL: f64 = 0.1;

/// A pad's size along x and y once turned by `deg` (quarter turns swap them).
fn turned_size(p: &Pad, deg: f64) -> [f64; 2] {
    let quarter = (((p.rotation + deg) / 90.0).round() as i64).rem_euclid(2);
    if quarter == 1 { [p.size[1], p.size[0]] } else { p.size }
}

/// Shapes that solder alike: the same, a round oval and a circle, or a rectangle and a rounded one.
fn same_shape(a: PadShape, b: PadShape, size: [f64; 2]) -> bool {
    use PadShape::*;
    let round = (size[0] - size[1]).abs() < 1e-6;
    a == b || (round && matches!((a, b), (Circle, Oval) | (Oval, Circle))) || matches!((a, b), (Rect, RoundRect) | (RoundRect, Rect))
}

/// A land to compare: one pad, or several of one number taken together (a thermal pad's
/// copper squares and vias against a single pad).
#[derive(Clone, Debug)]
struct Land {
    pad: Pad,
    /// Pads it stands for, and how many of them are vias (small plated holes).
    parts: usize,
    vias: usize,
}

/// Whether pads form one cluster: each within 0.8 mm (edge to edge) of another.
fn clustered(group: &[&Pad]) -> bool {
    if group.len() < 2 {
        return true;
    }
    let prims: Vec<geom::Prim> = group.iter().map(|q| footprints::pad_prim(q, [0.0, 0.0], 0.0, false)).collect();
    let mut seen = vec![false; prims.len()];
    let mut stack = vec![0];
    seen[0] = true;
    while let Some(i) = stack.pop() {
        for j in 0..prims.len() {
            if !seen[j] && geom::gap(&prims[i], &prims[j]) <= 0.8 {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    seen.iter().all(|s| *s)
}

/// Pads as lands: numbers that appear a different number of times on the two sides are merged into their bounding box.
fn lands(pads: &[Pad], other: &[Pad]) -> Vec<Land> {
    let count = |v: &[Pad], n: &str| v.iter().filter(|p| p.number == n).count();
    let mut out: Vec<Land> = Vec::new();
    let mut merged: Vec<String> = Vec::new();
    for p in pads {
        let n = p.number.as_str();
        let (mine, theirs) = (count(pads, n), count(other, n));
        if n.is_empty() || mine == theirs || theirs == 0 {
            out.push(Land { pad: p.clone(), parts: 1, vias: 0 });
            continue;
        }
        if merged.iter().any(|m| m == n) {
            continue;
        }
        let group: Vec<&Pad> = pads.iter().filter(|q| q.number == n).collect();
        if !clustered(&group) {
            // Pads of one number apart from each other (a regulator's pin 2 and its tab): compared one by one.
            out.push(Land { pad: p.clone(), parts: 1, vias: 0 });
            continue;
        }
        merged.push(n.to_string());
        let is_via = |q: &Pad| q.kind == PadKind::Tht && q.drill.is_some_and(|d| d[0].max(d[1]) < 0.4) && group.iter().any(|g| g.kind == PadKind::Smd);
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for q in group.iter().filter(|q| !is_via(q)) {
            let qb = footprints::pad_prim(q, [0.0, 0.0], 0.0, false).bbox();
            b = [b[0].min(qb[0]), b[1].min(qb[1]), b[2].max(qb[2]), b[3].max(qb[3])];
        }
        let vias = group.iter().filter(|q| is_via(q)).count();
        let first = group.iter().find(|q| !is_via(q)).copied().unwrap_or(group[0]);
        let pad = Pad { at: [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0], size: [b[2] - b[0], b[3] - b[1]], rotation: 0.0, drill: if first.kind == PadKind::Smd { None } else { first.drill }, ..first.clone() };
        out.push(Land { pad, parts: group.len(), vias });
    }
    out
}

/// Pairs lands of the same number (unnumbered holes only with their own kind), nearest first.
fn pair(ours: &[Land], theirs: &[Land], placed: &[[f64; 2]]) -> Vec<(usize, usize)> {
    let mut cands: Vec<(f64, usize, usize)> = Vec::new();
    for (i, p) in ours.iter().enumerate() {
        for (j, q) in theirs.iter().enumerate() {
            if p.pad.number == q.pad.number && (p.pad.kind == q.pad.kind || !p.pad.number.is_empty()) {
                cands.push((geom::dist(p.pad.at, placed[j]), i, j));
            }
        }
    }
    greedy(cands, ours.len(), theirs.len())
}

fn greedy(mut cands: Vec<(f64, usize, usize)>, n: usize, m: usize) -> Vec<(usize, usize)> {
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut done, mut used) = (vec![false; n], vec![false; m]);
    let mut out = Vec::new();
    for (_, i, j) in cands {
        if !done[i] && !used[j] {
            done[i] = true;
            used[j] = true;
            out.push((i, j));
        }
    }
    out
}

fn mean(v: &[[f64; 2]]) -> [f64; 2] {
    if v.is_empty() {
        return [0.0, 0.0];
    }
    let s = v.iter().fold([0.0, 0.0], |a, b| geom::add(a, *b));
    [s[0] / v.len() as f64, s[1] / v.len() as f64]
}

/// KiCad's lands turned by `deg` and shifted onto ours: by the holes when two or more pair
/// (they are where the part's pegs and leads are), else by every paired land.
fn align(ours: &[Land], theirs: &[Land], deg: f64) -> (Vec<[f64; 2]>, [f64; 2], usize, f64) {
    let turned: Vec<[f64; 2]> = theirs.iter().map(|q| geom::rotate(q.pad.at, deg)).collect();
    let numbered = |l: &Land| !l.pad.number.is_empty();
    let mut shift = geom::sub(mean(&ours.iter().filter(|l| numbered(l)).map(|l| l.pad.at).collect::<Vec<_>>()), mean(&theirs.iter().zip(&turned).filter(|(l, _)| numbered(l)).map(|(_, t)| *t).collect::<Vec<_>>()));
    let mut placed: Vec<[f64; 2]> = turned.iter().map(|t| geom::add(*t, shift)).collect();
    for _ in 0..3 {
        let pairs = pair(ours, theirs, &placed);
        let holes: Vec<&(usize, usize)> = pairs.iter().filter(|(i, j)| ours[*i].pad.drill.is_some() && theirs[*j].pad.drill.is_some()).collect();
        let basis: Vec<&(usize, usize)> = if holes.len() >= 2 { holes } else { pairs.iter().collect() };
        if basis.is_empty() {
            break;
        }
        let d = mean(&basis.iter().map(|(i, j)| geom::sub(ours[*i].pad.at, placed[*j])).collect::<Vec<_>>());
        shift = geom::add(shift, d);
        placed = turned.iter().map(|t| geom::add(*t, shift)).collect();
    }
    let pairs = pair(ours, theirs, &placed);
    let err = pairs.iter().map(|(i, j)| geom::dist(ours[*i].pad.at, placed[*j])).fold(0.0, f64::max);
    let shift = [if shift[0].abs() < 1e-6 { 0.0 } else { shift[0] }, if shift[1].abs() < 1e-6 { 0.0 } else { shift[1] }];
    (placed, shift, pairs.len(), err)
}

fn f3(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

fn kind_name(k: PadKind) -> &'static str {
    match k {
        PadKind::Smd => "SMD",
        PadKind::Tht => "plated through-hole",
        PadKind::Npth => "unplated hole",
    }
}

/// Overlap of two axis-aligned lands as a share of the smaller.
fn overlap(a: [f64; 2], sa: [f64; 2], b: [f64; 2], sb: [f64; 2]) -> f64 {
    let ov = |c1: f64, w1: f64, c2: f64, w2: f64| ((c1 + w1 / 2.0).min(c2 + w2 / 2.0) - (c1 - w1 / 2.0).max(c2 - w2 / 2.0)).max(0.0);
    let area = ov(a[0], sa[0], b[0], sb[0]) * ov(a[1], sa[1], b[1], sb[1]);
    area / (sa[0] * sa[1]).min(sb[0] * sb[1]).max(1e-12)
}

fn label(p: &Pad) -> String {
    if p.number.is_empty() { "hole".into() } else { p.number.clone() }
}

/// Compares one Iṣẹ́ Studio footprint with a KiCad one (both in Iṣẹ́ Studio coordinates).
pub fn compare(ours_fp: &Footprint, kicad: &Footprint, kicad_id: &str) -> FootprintReport {
    let ours = lands(&ours_fp.pads, &kicad.pads);
    let theirs = lands(&kicad.pads, &ours_fp.pads);
    // Try the four quarter turns; keep the one whose lands pair up best.
    let mut best: Option<(f64, Vec<[f64; 2]>, [f64; 2], usize, f64)> = None;
    for deg in [0.0, 90.0, 180.0, 270.0] {
        let (placed, shift, n, err) = align(&ours, &theirs, deg);
        if best.as_ref().is_none_or(|b| n > b.3 || n == b.3 && err + 1e-9 < b.4) {
            best = Some((deg, placed, shift, n, err));
        }
    }
    let (deg, placed, shift, _, _) = best.unwrap();
    let mut pairs: Vec<(usize, usize, bool)> = pair(&ours, &theirs, &placed).into_iter().map(|(i, j)| (i, j, false)).collect();
    // Lands left over pair by place and kind: the same copper under another number (a tab, a shield).
    let left_ours: Vec<usize> = (0..ours.len()).filter(|i| !pairs.iter().any(|p| p.0 == *i)).collect();
    let left_theirs: Vec<usize> = (0..theirs.len()).filter(|j| !pairs.iter().any(|p| p.1 == *j)).collect();
    let mut cands = Vec::new();
    for (a, i) in left_ours.iter().enumerate() {
        for (b, j) in left_theirs.iter().enumerate() {
            let d = geom::dist(ours[*i].pad.at, placed[*j]);
            if ours[*i].pad.kind == theirs[*j].pad.kind && d < 0.5 {
                cands.push((d, a, b));
            }
        }
    }
    for (a, b) in greedy(cands, left_ours.len(), left_theirs.len()) {
        pairs.push((left_ours[a], left_theirs[b], true));
    }
    let mut findings = Vec::new();
    let mut worst = 0.0f64;
    for &(i, j, renumbered) in &pairs {
        let (lp, lq) = (&ours[i], &theirs[j]);
        let (p, q) = (&lp.pad, &lq.pad);
        let name = label(p);
        if renumbered {
            findings.push(Finding { severity: "warning", pad: name.clone(), message: format!("numbered “{}” here, “{}” in KiCad (same place: a symbol made for one maps its pins to the other's pads differently)", p.number, q.number) });
        }
        if lp.parts != lq.parts {
            let describe = |l: &Land| if l.parts == 1 { "one pad".to_string() } else if l.vias > 0 { format!("{} pads with {} vias", l.parts - l.vias, l.vias) } else { format!("{} pads", l.parts) };
            findings.push(Finding { severity: "note", pad: name.clone(), message: format!("{} here, {} in KiCad; compared by their outline", describe(lp), describe(lq)) });
        }
        let d = geom::dist(p.at, placed[j]);
        worst = worst.max(d);
        if p.kind != q.kind {
            findings.push(Finding { severity: "error", pad: name.clone(), message: format!("{} here, {} in KiCad", kind_name(p.kind), kind_name(q.kind)) });
        }
        let (sp, sq) = (turned_size(p, 0.0), turned_size(q, deg));
        if d > POS_TOL {
            let share = overlap(p.at, sp, placed[j], sq);
            let holed = p.drill.is_some() || q.drill.is_some();
            let (severity, why) = if !holed && share >= 0.75 { ("warning", format!("; the lands overlap {:.0} % (a longer or shorter land)", share * 100.0)) } else { ("error", if holed { String::new() } else { format!("; the lands overlap only {:.0} %", share * 100.0) }) };
            findings.push(Finding { severity, pad: name.clone(), message: format!("centre at ({}, {}), KiCad's at ({}, {}): {} mm apart{why}", f3(p.at[0]), f3(p.at[1]), f3(placed[j][0]), f3(placed[j][1]), f3(d)) });
        }
        let ds = [(sp[0] - sq[0]).abs(), (sp[1] - sq[1]).abs()];
        if p.kind != PadKind::Npth && ds[0].max(ds[1]) >= SIZE_TOL - 1e-9 {
            findings.push(Finding { severity: "warning", pad: name.clone(), message: format!("land {} × {} mm, KiCad's {} × {} mm", f3(sp[0]), f3(sp[1]), f3(sq[0]), f3(sq[1])) });
        }
        if p.kind != PadKind::Npth && lp.parts == lq.parts && !same_shape(p.shape, q.shape, sp) {
            findings.push(Finding { severity: "warning", pad: name.clone(), message: format!("{:?} here, {:?} in KiCad", p.shape, q.shape).to_lowercase() });
        }
        match (p.drill, q.drill) {
            (Some(a), Some(b)) => {
                let (a, b) = ([a[0].min(a[1]), a[0].max(a[1])], [b[0].min(b[1]), b[0].max(b[1])]);
                let off = (a[0] - b[0]).abs().max((a[1] - b[1]).abs());
                if off > DRILL_TOL + 1e-9 {
                    findings.push(Finding { severity: "error", pad: name.clone(), message: format!("hole {} mm, KiCad's {} mm", hole(a), hole(b)) });
                } else if off > 1e-6 {
                    findings.push(Finding { severity: "warning", pad: name.clone(), message: format!("hole {} mm, KiCad's {} mm", hole(a), hole(b)) });
                }
            }
            (Some(a), None) => findings.push(Finding { severity: "error", pad: name.clone(), message: format!("hole {} mm here, none in KiCad", hole(a)) }),
            (None, Some(b)) => findings.push(Finding { severity: "error", pad: name.clone(), message: format!("no hole here, {} mm in KiCad", hole(b)) }),
            (None, None) => {}
        }
    }
    for (i, l) in ours.iter().enumerate().filter(|(i, _)| !pairs.iter().any(|p| p.0 == *i)) {
        let _ = i;
        findings.push(Finding { severity: "error", pad: l.pad.number.clone(), message: format!("{} pad {} at ({}, {}) has no KiCad counterpart", kind_name(l.pad.kind), if l.pad.number.is_empty() { "(unnumbered)" } else { &l.pad.number }, f3(l.pad.at[0]), f3(l.pad.at[1])) });
    }
    for (j, l) in theirs.iter().enumerate().filter(|(j, _)| !pairs.iter().any(|p| p.1 == *j)) {
        findings.push(Finding { severity: "error", pad: l.pad.number.clone(), message: format!("KiCad has {} pad {} at ({}, {}) that this footprint lacks", kind_name(l.pad.kind), if l.pad.number.is_empty() { "(unnumbered)" } else { &l.pad.number }, f3(placed[j][0]), f3(placed[j][1])) });
    }
    if shift[0].abs() > 1e-3 || shift[1].abs() > 1e-3 || deg != 0.0 {
        findings.push(Finding {
            severity: "note",
            pad: String::new(),
            message: format!(
                "KiCad's origin is at ({}, {}) here{}; positions files use ISE Studio's",
                f3(shift[0]),
                f3(shift[1]),
                if deg != 0.0 { format!(", its zero orientation turned {deg}°") } else { String::new() }
            ),
        });
    }
    for f in &mut findings {
        if let Some((_, _, _, why)) = REVIEWED.iter().find(|(fp, pad, start, _)| *fp == ours_fp.name && (*pad == "*" || *pad == f.pad || *pad == "hole" && f.pad.is_empty() || *pad == "hole" && f.pad == "hole") && f.message.starts_with(start)) {
            f.severity = "note";
            f.message = format!("{} — reviewed: {why}", f.message);
        }
    }
    let errors = findings.iter().filter(|f| f.severity == "error").count();
    let warnings = findings.iter().filter(|f| f.severity == "warning").count();
    FootprintReport { footprint: ours_fp.name.clone(), kicad: kicad_id.into(), offset: shift, rotation: deg, pads: ours_fp.pads.len(), kicad_pads: kicad.pads.len(), worst_position: (worst * 1e4).round() / 1e4, findings, errors, warnings }
}


fn hole(d: [f64; 2]) -> String {
    if (d[0] - d[1]).abs() < 1e-9 { format!("ø{}", f3(d[0])) } else { format!("{} × {}", f3(d[0]), f3(d[1])) }
}

/// Every built-in footprint (or those named) against the library.
pub fn verify(lib: &Library, only: &[String]) -> Report {
    let names: Vec<String> = if only.is_empty() { footprints::names() } else { only.to_vec() };
    let mut out = Report { checked: 0, errors: 0, warnings: 0, missing: vec![], footprints: vec![], library: lib.root.display().to_string() };
    for n in names {
        let (Some(ours), Some(id)) = (footprints::get(&n), kicad_equivalent(&n)) else {
            out.missing.push(n);
            continue;
        };
        match lib.footprint(&id) {
            Ok((theirs, _)) => {
                let r = compare(&ours, &theirs, &id);
                out.errors += r.errors;
                out.warnings += r.warnings;
                out.checked += 1;
                out.footprints.push(r);
            }
            Err(e) => out.missing.push(format!("{n} ({id}: {e})")),
        }
    }
    out
}
