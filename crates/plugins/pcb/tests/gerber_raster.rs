//! The fab files read back by the independent reader and compared with the
//! board (`gerber_check`): the ESP32 example's package must match it layer by
//! layer, net by net and hole by hole; and files tampered with the way a
//! writer bug would (a pad dropped, a net mislabelled, two nets bridged, a
//! hole lost, the outline broken) must each be caught.

use de_core::{Actor, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value};
use de_pcb::gerber_check::{self, GerberCheck};
use de_pcb::{BoardView, PcbPlugin, gerber};
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use std::sync::{Arc, OnceLock};

fn subst(v: &Value, outputs: &[Value]) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') => {
            let (n, path) = s[1..].split_once('.').unwrap();
            outputs[n.parse::<usize>().unwrap()][path].clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| subst(x, outputs)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), subst(x, outputs))).collect()),
        other => other.clone(),
    }
}

/// The example board, built once for every test here.
fn board() -> Arc<BoardView> {
    static B: OnceLock<Arc<BoardView>> = OnceLock::new();
    B.get_or_init(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
        let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("steps.json")).unwrap()).unwrap();
        let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
        let mut outs = Vec::new();
        for s in &steps {
            let env = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: "p".into(), expected_revision: p.revision(), operation: s["operation"].as_str().unwrap().into(), parameters: subst(&s["parameters"], &outs), actor: Actor { user_id: "u".into(), session_id: "s".into() } };
            outs.push(p.commit(env).unwrap().output);
        }
        let id: StableId = serde_json::from_value(outs[0]["id"].clone()).unwrap();
        de_pcb::view(p.world(), id).unwrap()
    })
    .clone()
}

fn problems(c: &GerberCheck) -> Vec<String> {
    let mut v: Vec<String> = c.layers.iter().flat_map(|l| l.problems.iter().map(move |p| format!("{}: {p}", l.file))).collect();
    v.extend(c.drills.iter().flat_map(|d| d.problems.iter().map(move |p| format!("{}: {p}", d.file))));
    v.extend(c.outline.iter().flat_map(|o| o.problems.iter().map(move |p| format!("{}: {p}", o.file))));
    v.extend(c.unreadable.iter().cloned());
    v
}

#[test]
fn the_package_reads_back_as_the_board() {
    let v = board();
    let files = gerber::files(&v);
    let c = gerber_check::check(&v, &files, 0.02);
    eprintln!("{}", c.summary);
    for l in &c.layers {
        eprintln!("{} {} objects, board {:?} mm², file {:?} mm², differ {:?} mm², pads {}/{}, nets {}", l.file, l.objects, l.board_area, l.file_area, l.differ_area, l.pads_ok, l.pads, l.nets.len());
    }
    assert!(c.ok, "{:#?}", problems(&c));
    let top = c.layers.iter().find(|l| l.function.starts_with("Copper,L1")).unwrap();
    assert!(top.board_area.unwrap() > 50.0 && top.pads > 50 && top.pads_ok == top.pads);
    assert!(top.nets.iter().any(|n| n.net == "3V3" && n.board > 1.0 && n.ok));
    let bottom = c.layers.iter().find(|l| l.function.contains("Bot") && l.function.starts_with("Copper")).unwrap();
    assert!(bottom.nets.iter().any(|n| n.net == "GND" && n.board > 500.0), "the bottom pour is GND");
    assert_eq!(c.drills.iter().map(|d| d.file_holes).sum::<usize>(), v.holes.len());
    let o = c.outline.as_ref().unwrap();
    assert!((o.file_size[0] - 54.0).abs() < 1e-6 && o.loops == 1);
}

fn tamper(name_has: &str, f: impl Fn(&str) -> String) -> GerberCheck {
    let v = board();
    let files: Vec<(String, String)> = gerber::files(&v).into_iter().map(|(n, t)| if n.contains(name_has) { let t2 = f(&t); assert_ne!(t, t2, "{n} was changed"); (n, t2) } else { (n, t) }).collect();
    gerber_check::check(&v, &files, 0.02)
}

#[test]
fn a_dropped_pad_is_caught() {
    // Delete the flash of the first SMD pad on the top copper.
    let c = tamper("F_Cu", |t| {
        let i = t.find("%TO.P,").unwrap();
        let flash = i + t[i..].find("D03*").unwrap() + 4;
        let start = t[..flash].rfind('\n').unwrap() + 1;
        format!("{}{}", &t[..start], &t[flash..])
    });
    let p = problems(&c);
    assert!(!c.ok && p.iter().any(|x| x.contains("pads not exposed")), "{p:#?}");
}

#[test]
fn a_mislabelled_net_and_a_bridge_are_caught() {
    // The 3V3 track's net attribute says EN: both nets' copper differs.
    let c = tamper("F_Cu", |t| t.replacen("%TO.N,3V3*%", "%TO.N,EN*%", 1));
    assert!(problems(&c).iter().any(|x| x.contains("net 3V3") || x.contains("net EN")), "{:#?}", problems(&c));
    // A fat line drawn from a 3V3 pad to a GND pad shorts them.
    let v = board();
    let a = v.pads.iter().find(|p| p.net.as_deref() == Some("3V3") && p.layers.iter().any(|l| l == "top")).unwrap().at;
    let b = v.pads.iter().find(|p| p.net.as_deref() == Some("GND") && p.layers.iter().any(|l| l == "top") && de_pcb::geom::dist(p.at, a) < 6.0).unwrap().at;
    let c = tamper("F_Cu", |t| {
        let c = |x: f64| (x * 1e6).round() as i64;
        t.replace("M02*", &format!("%ADD999C,0.5*%\n%TO.N,3V3*%\nD999*\nX{}Y{}D02*\nX{}Y{}D01*\n%TD*%\nM02*", c(a[0]), c(a[1]), c(b[0]), c(b[1])))
    });
    assert!(c.layers.iter().any(|l| l.shorts.iter().any(|s| s.contains("3V3") && s.contains("GND"))), "{:#?}", problems(&c));
}

#[test]
fn a_lost_hole_and_a_broken_outline_are_caught() {
    let c = tamper("PTH.drl", |t| {
        let i = t.find("\nX").unwrap() + 1;
        let j = i + t[i..].find('\n').unwrap() + 1;
        format!("{}{}", &t[..i], &t[j..])
    });
    assert!(problems(&c).iter().any(|x| x.contains("holes missing")), "{:#?}", problems(&c));
    let c = tamper("Edge_Cuts", |t| {
        // Drop one corner of the profile: the outline no longer closes.
        let i = t.rfind("D01*").unwrap();
        let start = t[..i].rfind('\n').unwrap() + 1;
        format!("{}{}", &t[..start], &t[i + 5..])
    });
    assert!(c.outline.as_ref().is_some_and(|o| !o.ok), "{:#?}", c.outline);
}

#[test]
fn layers_draw_as_svg_from_what_was_read() {
    let v = board();
    let (_, text) = gerber::files(&v).into_iter().find(|(n, _)| n.contains("F_Cu")).unwrap();
    let g = de_pcb::reader::gerber(&text).unwrap();
    let s = gerber_check::svg(&g);
    assert!(s.contains("<path") && s.contains("<polyline"));
}
