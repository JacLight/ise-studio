//! KiCad and IDF interop, checked without KiCad: the worked example goes
//! Forma → .kicad_pcb → Forma and must come back with the same nets, pads,
//! copper and pours; hand-written files in the KiCad 8 and KiCad 6 styles
//! (written from the format documentation) land where they should in
//! Forma's coordinates; and the IDF files parse back with a small reader.

use de_core::{Actor, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value, json};
use de_pcb::board::BoardView;
use de_pcb::footprints::PadShape;
use de_pcb::{PcbPlugin, kicad};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn substitute(v: &Value, outputs: &[Value]) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') => {
            let (n, path) = s[1..].split_once('.').unwrap();
            outputs[n.parse::<usize>().unwrap()][path].clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, outputs)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), substitute(x, outputs))).collect()),
        other => other.clone(),
    }
}

fn project() -> Project {
    Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])))
}

fn commit(p: &mut Project, op: &str, params: Value) -> Value {
    let env = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: "p".into(), expected_revision: p.revision(), operation: op.into(), parameters: params, actor: Actor { user_id: "u".into(), session_id: "s".into() } };
    p.commit(env).unwrap_or_else(|e| panic!("{op}: {e}")).output
}

fn example() -> (Project, StableId) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("steps.json")).unwrap()).unwrap();
    let mut p = project();
    let mut outs = Vec::new();
    for s in &steps {
        let params = substitute(&s["parameters"], &outs);
        outs.push(commit(&mut p, s["operation"].as_str().unwrap(), params));
    }
    let id = serde_json::from_value(outs[0]["id"].clone()).unwrap();
    (p, id)
}

fn import(text: &str) -> (Project, StableId, Value) {
    let mut p = project();
    let out = commit(&mut p, "pcb.board.import_kicad", json!({ "text": text }));
    let id = serde_json::from_value(out["id"].clone()).unwrap();
    (p, id, out)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-4
}

fn nets(v: &BoardView) -> BTreeMap<String, BTreeSet<String>> {
    let mut m: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in &v.pads {
        if let Some(n) = &p.net {
            m.entry(n.clone()).or_default().insert(format!("{}.{}", p.reference, p.number));
        }
    }
    m
}

/// Pads as comparable rows, sorted.
fn pads(v: &BoardView) -> Vec<(String, String, [f64; 2], [f64; 2], String, Option<[f64; 2]>, Vec<String>, f64)> {
    let mut rows: Vec<_> = v.pads.iter().map(|p| (p.reference.clone(), p.number.clone(), p.at, p.size, format!("{:?}", p.shape), p.drill, p.layers.clone(), p.rotation.rem_euclid(180.0))).collect();
    rows.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())).then(((a.2[0] * 1e3).round() as i64, (a.2[1] * 1e3).round() as i64).cmp(&((b.2[0] * 1e3).round() as i64, (b.2[1] * 1e3).round() as i64))));
    rows
}

fn segments(v: &BoardView) -> Vec<(String, i64, String, [i64; 4])> {
    let k = |x: f64| (x * 1e4).round() as i64;
    let mut out: Vec<_> = v
        .tracks
        .iter()
        .flat_map(|t| {
            t.points.windows(2).map(move |w| {
                let (a, b) = if (k(w[0][0]), k(w[0][1])) <= (k(w[1][0]), k(w[1][1])) { (w[0], w[1]) } else { (w[1], w[0]) };
                (t.layer.clone(), k(t.width), t.net.clone().unwrap_or_default(), [k(a[0]), k(a[1]), k(b[0]), k(b[1])])
            })
        })
        .collect();
    out.sort();
    out
}

fn same_board(a: &BoardView, b: &BoardView) {
    assert_eq!(nets(a), nets(b), "nets");
    let (pa, pb) = (pads(a), pads(b));
    assert_eq!(pa.len(), pb.len(), "pad count");
    for (x, y) in pa.iter().zip(&pb) {
        let label = format!("{}.{}", x.0, x.1);
        assert_eq!((&x.0, &x.1), (&y.0, &y.1));
        assert!(close(x.2[0], y.2[0]) && close(x.2[1], y.2[1]), "{label} at {:?} vs {:?}", x.2, y.2);
        assert!(close(x.3[0], y.3[0]) && close(x.3[1], y.3[1]), "{label} size {:?} vs {:?}", x.3, y.3);
        assert_eq!(x.4, y.4, "{label} shape");
        assert_eq!(x.5.map(|d| [(d[0] * 1e4).round(), (d[1] * 1e4).round()]), y.5.map(|d| [(d[0] * 1e4).round(), (d[1] * 1e4).round()]), "{label} drill");
        assert_eq!(x.6, y.6, "{label} layers");
        assert!(close(x.7, y.7) || close((x.7 - y.7).abs(), 180.0), "{label} rotation {} vs {}", x.7, y.7);
    }
    let (sa, sb) = (segments(a), segments(b));
    assert_eq!(sa.len(), sb.len(), "segment count");
    for (x, y) in sa.iter().zip(&sb) {
        assert_eq!(x.0, y.0);
        assert_eq!(x.1, y.1);
        assert_eq!(x.2, y.2);
        assert!(x.3.iter().zip(&y.3).all(|(p, q)| (p - q).abs() <= 1), "segment {x:?} vs {y:?}");
    }
    let vias = |v: &BoardView| {
        let mut r: Vec<_> = v.vias.iter().map(|x| ((x.at[0] * 1e4).round() as i64, (x.at[1] * 1e4).round() as i64, (x.drill * 1e4).round() as i64, (x.diameter * 1e4).round() as i64, x.net.clone())).collect();
        r.sort();
        r
    };
    assert_eq!(vias(a), vias(b), "vias");
    assert_eq!(a.zones.len(), b.zones.len(), "zones");
    for (x, y) in a.zones.iter().zip(&b.zones) {
        assert_eq!((&x.net, &x.layer, x.priority), (&y.net, &y.layer, y.priority));
        let area = |o: &[[f64; 2]]| de_pcb::geom::polygon_area(o).abs();
        assert!((area(&x.outline) - area(&y.outline)).abs() < 1e-3);
        assert!((x.area - y.area).abs() / x.area.max(1.0) < 0.01, "zone fill {} vs {}", x.area, y.area);
    }
    let oa = de_pcb::geom::polygon_area(&a.outline).abs();
    let ob = de_pcb::geom::polygon_area(&b.outline).abs();
    assert!((oa - ob).abs() < 1e-3, "outline area {oa} vs {ob}");
    assert_eq!(a.components.len(), b.components.len());
    for c in &a.components {
        let d = b.components.iter().find(|d| d.reference == c.reference).unwrap_or_else(|| panic!("{} missing", c.reference));
        assert_eq!((&c.value, c.side, &c.mpn, &c.lcsc), (&d.value, d.side, &d.mpn, &d.lcsc), "{}", c.reference);
    }
    assert_eq!(a.errors(), b.errors(), "DRC errors: {:#?}", b.violations.iter().filter(|v| v.severity == "error").map(|v| &v.message).collect::<Vec<_>>());
}

#[test]
fn the_example_round_trips_through_kicad() {
    let (p, id) = example();
    let a = de_pcb::view(p.world(), id).unwrap();
    let text = kicad::export_world(p.world(), id).unwrap();
    assert!(text.starts_with("(kicad_pcb"));
    assert!(text.contains("(version 20240108)"));
    de_pcb::sexpr::parse(&text).expect("the export is well-formed");
    let (p2, id2, out) = import(&text);
    eprintln!("notes: {}", out["notes"]);
    let b = de_pcb::view(p2.world(), id2).unwrap();
    same_board(&a, &b);
    // Zone settings survive.
    let za = de_pcb::items(p.world(), id).zones;
    let zb = de_pcb::items(p2.world(), id2).zones;
    assert_eq!(za.len(), zb.len());
    for ((_, x), (_, y)) in za.iter().zip(&zb) {
        assert_eq!(x.connection, y.connection);
        assert!((x.gap_thermal() - y.gap_thermal()).abs() < 1e-9 && (x.spoke_width() - y.spoke_width()).abs() < 1e-9);
        assert!((x.min_width - y.min_width).abs() < 1e-9);
    }
    // And again: Forma's own import exports and re-imports to the same board.
    let text2 = kicad::export_world(p2.world(), id2).unwrap();
    let (p3, id3, _) = import(&text2);
    same_board(&b, &de_pcb::view(p3.world(), id3).unwrap());
    // The 3D board was made with every body.
    let b2 = p2.get::<de_pcb::Board>(id2).unwrap();
    assert!(b2.model.is_some());
}

#[test]
fn a_kicad8_file_lands_in_forma_coordinates() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kicad8_small.kicad_pcb")).unwrap();
    let k = kicad::parse_board(&text).unwrap();
    assert_eq!(k.title.as_deref(), Some("Fixture eight"));
    assert_eq!(k.layers, 2);
    assert!((k.thickness - 1.2).abs() < 1e-9);
    assert_eq!(k.cutouts.len(), 1, "the round hole on Edge.Cuts is a cutout");
    assert!((de_pcb::geom::polygon_area(&k.outline) - 600.0).abs() < 1e-6);
    assert!(k.notes.iter().any(|n| n.contains("keep-out")));
    assert_eq!(k.zones.len(), 1);
    let z = &k.zones[0];
    assert_eq!((z.net.as_str(), z.layer.as_str(), z.priority), ("GND", "bottom", 2));
    assert_eq!(z.connection, de_pcb::PadConnection::Solid);
    assert_eq!((z.clearance, z.min_width, z.thermal_gap, z.thermal_spoke), (Some(0.3), 0.2, Some(0.4), Some(0.35)));
    // The arc and the segment after it are one track; every arc point is 3 mm from the arc's centre (KiCad 110, 113 → Forma 10, 7).
    assert_eq!(k.tracks.len(), 1);
    let t = &k.tracks[0];
    assert_eq!(t.net.as_deref(), Some("GND"));
    let last = *t.points.last().unwrap();
    let first = t.points[0];
    let ends = [first, last];
    assert!(ends.iter().any(|p| close(p[0], 10.0) && close(p[1], 10.0)) && ends.iter().any(|p| close(p[0], 18.0) && close(p[1], 7.0)), "{ends:?}");
    let arc_pts = t.points.iter().filter(|p| p[0] <= 13.0 + 1e-9).count();
    assert!(arc_pts > 4);
    assert!(t.points.iter().filter(|p| p[0] <= 13.0 + 1e-9).all(|p| (((p[0] - 10.0).hypot(p[1] - 7.0)) - 3.0).abs() < 1e-3));
    assert_eq!(k.vias.len(), 1);
    assert!(close(k.vias[0].at[0], 18.0) && close(k.vias[0].at[1], 7.0));

    // Through the command: pads where KiCad puts them.
    let (p, id, out) = import(&text);
    assert_eq!(out["components"], 3);
    let v = de_pcb::view(p.world(), id).unwrap();
    let pad = |r: &str, n: &str| v.pads.iter().find(|p| p.reference == r && p.number == n).unwrap_or_else(|| panic!("{r}.{n}"));
    // R1 at (110, 105) turned 90°: pad 1 (x −0.825) goes down the screen, i.e. to Forma y − 0.825.
    let p1 = pad("R1", "1");
    assert!(close(p1.at[0], 10.0) && close(p1.at[1], 15.0 - 0.825), "{:?}", p1.at);
    assert!(close(p1.rotation.rem_euclid(360.0), 90.0));
    assert_eq!(p1.net.as_deref(), Some("GND"));
    assert_eq!(p1.shape, PadShape::RoundRect);
    let r1 = v.components.iter().find(|c| c.reference == "R1").unwrap();
    assert_eq!((r1.value.as_str(), r1.lcsc.as_deref()), ("10k", Some("C25804")));
    // TP1 on the bottom at (120, 110) turned 45°: pad 1 is 1 mm along 45° from it.
    let tp = pad("TP1", "1");
    let h = std::f64::consts::FRAC_1_SQRT_2;
    assert!(close(tp.at[0], 20.0 + h) && close(tp.at[1], 10.0 + h), "{:?}", tp.at);
    assert_eq!(tp.drill, Some([1.0, 1.0]));
    let tp2 = pad("TP1", "2");
    assert!(close(tp2.at[0], 20.0 - 1.5 * h) && close(tp2.at[1], 10.0 - 1.5 * h), "{:?}", tp2.at);
    assert_eq!(tp2.drill, Some([0.6, 1.6]));
    assert!(close(tp2.rotation.rem_euclid(360.0), 45.0), "{}", tp2.rotation);
    assert_eq!(v.components.iter().find(|c| c.reference == "TP1").unwrap().side, de_pcb::Side::Bottom);
    // The custom L-shaped pad: its outline is the L plus the anchor square's corner in the notch (4 + 0.0625 mm²), the arm at KiCad (+1.5, −0.5) is above-right in Forma.
    let j = pad("J1", "1");
    assert_eq!(j.shape, PadShape::Custom);
    let poly = j.polygon.as_ref().expect("custom outline");
    assert!((de_pcb::geom::polygon_area(poly) - 4.0625).abs() < 1e-3, "{}", de_pcb::geom::polygon_area(poly));
    assert!(de_pcb::geom::in_polygon([6.5, 5.5], poly));
    assert!(!de_pcb::geom::in_polygon([6.5, 4.5], poly), "the notch stays open");
    // It exports back out as a custom pad with its primitives.
    let text2 = kicad::export_world(p.world(), id).unwrap();
    assert!(text2.contains("custom") && text2.contains("gr_poly"));
    let (p2, id2, _) = import(&text2);
    same_board(&v, &de_pcb::view(p2.world(), id2).unwrap());
}

#[test]
fn a_kicad6_file_with_fp_text_reads() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kicad6_fp_text.kicad_pcb")).unwrap();
    let k = kicad::parse_board(&text).unwrap();
    let want = 375.0 + 25.0 * std::f64::consts::PI / 4.0;
    assert!((de_pcb::geom::polygon_area(&k.outline) - want).abs() < 0.05, "{}", de_pcb::geom::polygon_area(&k.outline));
    assert_eq!(k.components.len(), 1);
    let c = &k.components[0];
    assert_eq!((c.reference.as_str(), c.value.as_str()), ("R7", "1k"));
    assert!(close(c.at[0], 10.0) && close(c.at[1], 10.0));
    assert_eq!(c.pins.get("1").map(String::as_str), Some("N1"));
    assert_eq!(k.tracks.len(), 1);
    assert_eq!(k.tracks[0].points.len(), 3);
    assert!(k.tracks[0].points.iter().any(|p| close(p[0], 15.0) && close(p[1], 15.0)));
    let (p, id, _) = import(&text);
    let v = de_pcb::view(p.world(), id).unwrap();
    let pad = v.pads.iter().find(|p| p.number == "2").unwrap();
    assert!(close(pad.at[0], 11.0) && close(pad.at[1], 10.0));
    assert!(v.nets.iter().find(|n| n.name == "N2").unwrap().length > 8.9);
}

#[test]
fn bad_files_are_refused_clearly() {
    assert!(kicad::parse_board("(kicad_sch (version 1))").unwrap_err().contains("not a KiCad board"));
    assert!(kicad::parse_board("(kicad_pcb (version 20240108))").unwrap_err().contains("outline"));
    let six = "(kicad_pcb (layers (0 \"F.Cu\" signal) (1 \"In1.Cu\" signal) (2 \"In2.Cu\" signal) (3 \"In3.Cu\" signal) (4 \"In4.Cu\" signal) (31 \"B.Cu\" signal)))";
    assert!(kicad::parse_board(six).unwrap_err().contains("6 copper layers"));
    assert!(kicad::parse_board("(kicad_pcb (a").is_err());
}

// ---- IDF ------------------------------------------------------------------------------

/// A section of an IDF file: its name, header words and record lines.
struct Section {
    name: String,
    lines: Vec<Vec<String>>,
}

fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                if !quoted {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    assert!(!quoted, "unbalanced quote in {line}");
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn idf_sections(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut open: Option<Section> = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(rest) = line.strip_prefix(".END_") {
            let s = open.take().unwrap_or_else(|| panic!(".END_{rest} with no section open"));
            assert_eq!(s.name, rest.split_whitespace().next().unwrap(), "section closed by the wrong end");
            out.push(s);
        } else if let Some(rest) = line.strip_prefix('.') {
            assert!(open.is_none(), "section {rest} opened inside another");
            open = Some(Section { name: rest.split_whitespace().next().unwrap().to_string(), lines: vec![] });
        } else {
            open.as_mut().expect("a record outside any section").lines.push(words(line));
        }
    }
    assert!(open.is_none(), "a section is left open");
    out
}

fn loops(records: &[Vec<String>]) -> BTreeMap<u32, Vec<[f64; 2]>> {
    let mut m: BTreeMap<u32, Vec<[f64; 2]>> = BTreeMap::new();
    for r in records {
        assert_eq!(r.len(), 4, "loop record {r:?}");
        m.entry(r[0].parse().unwrap()).or_default().push([r[1].parse().unwrap(), r[2].parse().unwrap()]);
    }
    m
}

#[test]
fn idf_files_parse_back() {
    let (p, id) = example();
    let v = de_pcb::view(p.world(), id).unwrap();
    let b = p.get::<de_pcb::Board>(id).unwrap().clone();
    let files = de_pcb::idf::files(&v, &b);
    assert_eq!(files.len(), 2);
    assert!(files[0].0.ends_with(".emn") && files[1].0.ends_with(".emp"));
    let emn = idf_sections(&files[0].1);
    let emp = idf_sections(&files[1].1);
    let names: Vec<&str> = emn.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["HEADER", "BOARD_OUTLINE", "DRILLED_HOLES", "PLACEMENT"]);
    assert_eq!(emn[0].lines[0][0], "BOARD_FILE");
    assert_eq!(emn[0].lines[0][1], "3.0");
    assert_eq!(emn[0].lines[1][1], "MM");
    // Outline: thickness, then loop 0 closed and counter-clockwise; cutouts clockwise.
    let outline = &emn[1].lines;
    assert!((outline[0][0].parse::<f64>().unwrap() - v.thickness).abs() < 1e-9);
    let ls = loops(&outline[1..]);
    for (k, pts) in &ls {
        assert!(pts.len() >= 4);
        assert_eq!(pts[0], *pts.last().unwrap(), "loop {k} closes");
        let a = de_pcb::geom::polygon_area(&pts[..pts.len() - 1]);
        if *k == 0 {
            assert!(a > 0.0 && (a - de_pcb::geom::polygon_area(&v.outline).abs()).abs() < 0.05, "outline area {a}");
        } else {
            assert!(a < 0.0, "cutout {k} clockwise");
        }
    }
    assert_eq!(emn[2].lines.len(), v.holes.len(), "every hole");
    for h in &emn[2].lines {
        assert_eq!(h.len(), 7, "{h:?}");
        assert!(h[3] == "PTH" || h[3] == "NPTH");
        assert_eq!(h[6], "ECAD");
    }
    let placed = v.components.iter().filter(|c| !c.dnp && c.kind != "hole").count();
    assert_eq!(emn[3].lines.len(), placed * 2);
    assert_eq!(emp[0].name, "HEADER");
    let packages: BTreeSet<(String, String)> = emp.iter().filter(|s| s.name == "ELECTRICAL").map(|s| {
        let h = &s.lines[0];
        assert_eq!(h[2], "MM");
        assert!(h[3].parse::<f64>().unwrap() > 0.0, "height");
        let ls = loops(&s.lines[1..]);
        assert_eq!(ls[&0][0], *ls[&0].last().unwrap());
        (h[0].clone(), h[1].clone())
    }).collect();
    for pair in emn[3].lines.chunks(2) {
        assert!(packages.contains(&(pair[0][0].clone(), pair[0][1].clone())), "package {:?} in the library", pair[0]);
        assert!(pair[1][4] == "TOP" || pair[1][4] == "BOTTOM");
        assert_eq!(pair[1][5], "PLACED");
    }
}
