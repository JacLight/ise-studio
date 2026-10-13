//! IPC-2581 revision C: the whole board in one XML file for fabrication and
//! assembly. Written from the published standard (IPC-2581C and its
//! schema): the content list and dictionaries (pad shapes, line widths,
//! colours), the logistic header and history, the BOM, and the ECAD data:
//! the layers and stackup, the board profile with cutouts, every package
//! with its pins, every placed component, the logical nets, and each
//! layer's features (pads with their pin and net, tracks, pours with their
//! holes, solder mask and paste openings, silkscreen, drilled holes).
//! Millimetres throughout.

use super::xml::Writer;
use crate::board::{BoardView, PadView};
use crate::footprints::{PadKind, PadShape};
use crate::geom::{P, Prim};
use crate::{Board, Side};
use std::collections::BTreeMap;

fn f(v: f64) -> String {
    super::num(v, 4)
}

/// IPC-2581 layer name for a Forma copper layer.
pub fn layer_name(l: &str) -> String {
    match l {
        "top" => "TOP".into(),
        "bottom" => "BOTTOM".into(),
        x => x.to_ascii_uppercase(),
    }
}

/// A pad's standard primitive (id, element name, attributes) before rotation.
fn primitive(p: &PadView) -> (String, &'static str, Vec<(&'static str, String)>) {
    let (w, h) = (p.size[0], p.size[1]);
    match p.shape {
        PadShape::Circle => (format!("CIRCLE_{}", f(w)), "Circle", vec![("diameter", f(w))]),
        PadShape::Oval => (format!("OVAL_{}_{}", f(w), f(h)), "Oval", vec![("width", f(w)), ("height", f(h))]),
        PadShape::RoundRect if p.corner > 0.0 => (format!("RRECT_{}_{}_{}", f(w), f(h), f(p.corner)), "RectRound", vec![("width", f(w)), ("height", f(h)), ("radius", f(p.corner)), ("upperRight", "true".into()), ("upperLeft", "true".into()), ("lowerRight", "true".into()), ("lowerLeft", "true".into())]),
        _ => (format!("RECT_{}_{}", f(w), f(h)), "RectCenter", vec![("width", f(w)), ("height", f(h))]),
    }
}

fn polygon(w: &mut Writer, pts: &[P]) {
    if pts.len() < 3 {
        return;
    }
    w.open("Polygon", &[]);
    w.empty("PolyBegin", &[("x", f(pts[0][0])), ("y", f(pts[0][1]))]);
    for q in pts.iter().skip(1).chain(std::iter::once(&pts[0])) {
        w.empty("PolyStepSegment", &[("x", f(q[0])), ("y", f(q[1]))]);
    }
    w.close();
}

fn contour(w: &mut Writer, outer: &[P], holes: &[Vec<P>]) {
    w.open("Contour", &[]);
    polygon(w, outer);
    for h in holes {
        if h.len() >= 3 {
            w.open("Cutout", &[]);
            w.empty("PolyBegin", &[("x", f(h[0][0])), ("y", f(h[0][1]))]);
            for q in h.iter().skip(1).chain(std::iter::once(&h[0])) {
                w.empty("PolyStepSegment", &[("x", f(q[0])), ("y", f(q[1]))]);
            }
            w.close();
        }
    }
    w.close();
}

/// A board graphic as features: filled regions and polylines.
fn graphic(w: &mut Writer, g: &crate::graphics::GraphicView) {
    for island in g.fills.iter().filter(|i| !i.is_empty()) {
        contour(w, &island[0], &island[1..]);
    }
    for s in g.strokes.iter().filter(|s| s.len() >= 2) {
        w.open("Polyline", &[]);
        w.empty("PolyBegin", &[("x", f(s[0][0])), ("y", f(s[0][1]))]);
        for q in &s[1..] {
            w.empty("PolyStepSegment", &[("x", f(q[0])), ("y", f(q[1]))]);
        }
        w.empty("LineDescRef", &[("id", line_id(g.width))]);
        w.close();
    }
}

fn line_id(width: f64) -> String {
    format!("LINE_{}", f(width))
}

fn rgb(c: &str) -> [u8; 3] {
    match c.to_ascii_lowercase().as_str() {
        "black" => [20, 20, 22],
        "blue" => [20, 50, 140],
        "red" => [160, 20, 20],
        "white" => [240, 240, 235],
        "purple" => [90, 30, 130],
        "yellow" => [220, 190, 30],
        _ => [20, 100, 40],
    }
}

pub fn write(v: &BoardView, board: &Board) -> String {
    let step = crate::gerber::file_stem(&v.name);
    let copper: Vec<String> = v.layers.iter().map(|l| layer_name(l)).collect();
    // Layers in stack order, with their functions.
    let mut layers: Vec<(String, &str, &str)> = vec![("SILKSCREEN_TOP".into(), "SILKSCREEN", "TOP"), ("SOLDERPASTE_TOP".into(), "SOLDERPASTE", "TOP"), ("SOLDERMASK_TOP".into(), "SOLDERMASK", "TOP")];
    for (i, c) in copper.iter().enumerate() {
        layers.push((c.clone(), if i == 0 || i + 1 == copper.len() { "SIGNAL" } else { "PLANE" }, if i == 0 { "TOP" } else if i + 1 == copper.len() { "BOTTOM" } else { "INTERNAL" }));
    }
    for x in [("SOLDERMASK_BOTTOM", "SOLDERMASK"), ("SOLDERPASTE_BOTTOM", "SOLDERPASTE"), ("SILKSCREEN_BOTTOM", "SILKSCREEN")] {
        layers.push((x.0.into(), x.1, "BOTTOM"));
    }
    let drill = format!("DRILL_{}-{}", copper[0], copper[copper.len() - 1]);
    layers.push((drill.clone(), "DRILL", "ALL"));
    // Blind and buried via drill layers, one per layer pair.
    let pairs: Vec<(String, [String; 2])> = crate::gerber::spans(v).into_iter().map(|(_, _, sp)| (format!("DRILL_{}-{}", layer_name(&sp[0]), layer_name(&sp[1])), sp)).collect();
    for (n, _) in &pairs {
        layers.push((n.clone(), "DRILL", "INTERNAL"));
    }
    // Dictionaries.
    let mut prims: BTreeMap<String, (&'static str, Vec<(&'static str, String)>)> = BTreeMap::new();
    for p in &v.pads {
        if p.polygon.is_none() {
            let (id, el, attrs) = primitive(p);
            prims.entry(id).or_insert((el, attrs));
        }
    }
    for x in &v.vias {
        for d in std::iter::once(x.diameter).chain(x.stack.values().copied().filter(|d| *d > 0.0)) {
            prims.entry(format!("CIRCLE_{}", f(d))).or_insert(("Circle", vec![("diameter", f(d))]));
        }
    }
    let mut widths: Vec<f64> = v.tracks.iter().map(|t| t.width).chain(v.graphics.iter().map(|g| g.width)).chain([0.1, v.rules.silk_width]).collect();
    widths.sort_by(f64::total_cmp);
    widths.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

    let mut w = Writer::new();
    w.open("IPC-2581", &[("revision", "C".into()), ("xmlns", "http://webstds.ipc.org/2581".into()), ("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance".into())]);
    w.open("Content", &[("roleRef", "Owner".into())]);
    w.empty("FunctionMode", &[("mode", "ASSEMBLY".into())]);
    w.empty("StepRef", &[("name", step.clone())]);
    for (n, _, _) in &layers {
        w.empty("LayerRef", &[("name", n.clone())]);
    }
    w.empty("BomRef", &[("name", format!("{step}_BOM"))]);
    w.open("DictionaryStandard", &[("units", "MILLIMETER".into())]);
    for (id, (el, attrs)) in &prims {
        w.open("EntryStandard", &[("id", id.clone())]);
        w.empty(el, attrs);
        w.close();
    }
    w.close();
    w.open("DictionaryUser", &[("units", "MILLIMETER".into())]);
    for (i, p) in v.pads.iter().enumerate() {
        if let Some(pg) = &p.polygon {
            // Custom pads: their outline about the pad centre.
            w.open("EntryUser", &[("id", format!("CUSTOM_{i}"))]);
            w.open("UserSpecial", &[]);
            let local: Vec<P> = pg.iter().map(|q| [q[0] - p.at[0], q[1] - p.at[1]]).collect();
            contour(&mut w, &local, &[]);
            w.close();
            w.close();
        }
    }
    w.close();
    w.open("DictionaryLineDesc", &[("units", "MILLIMETER".into())]);
    for x in &widths {
        w.open("EntryLineDesc", &[("id", line_id(*x))]);
        w.empty("LineDesc", &[("lineEnd", "ROUND".into()), ("lineWidth", f(*x))]);
        w.close();
    }
    w.close();
    w.open("DictionaryColor", &[]);
    let mask = rgb(&v.mask);
    for (id, c) in [("COLOR_MASK", mask), ("COLOR_SILK", if v.silk.eq_ignore_ascii_case("black") { [20, 20, 20] } else { [250, 250, 250] })] {
        w.open("EntryColor", &[("id", id.into())]);
        w.empty("Color", &[("r", c[0].to_string()), ("g", c[1].to_string()), ("b", c[2].to_string())]);
        w.close();
    }
    w.close();
    w.close(); // Content

    w.open("LogisticHeader", &[]);
    w.empty("Role", &[("id", "Owner".into()), ("roleFunction", "SENDER".into())]);
    w.empty("Enterprise", &[("id", "Forma".into()), ("code", "NONE".into())]);
    w.empty("Person", &[("name", "Forma".into()), ("enterpriseRef", "Forma".into()), ("roleRef", "Owner".into())]);
    w.close();
    let now = super::now_iso();
    w.open("HistoryRecord", &[("number", "1".into()), ("origination", now.clone()), ("software", "Forma".into()), ("lastChange", now)]);
    w.open("FileRevision", &[("fileRevisionId", board.revision.clone()), ("comment", format!("{} rev {}", v.name, v.revision)), ("label", board.revision.clone())]);
    w.open("SoftwarePackage", &[("name", "Forma".into()), ("revision", env!("CARGO_PKG_VERSION").into()), ("vendor", "Forma".into())]);
    w.empty("Certification", &[("certificationStatus", "SELFTEST".into())]);
    w.close();
    w.close();
    w.close();

    // BOM: one item per footprint and value.
    let mut items: BTreeMap<(String, String), Vec<&crate::board::ComponentView>> = BTreeMap::new();
    for c in v.components.iter().filter(|c| c.kind != "hole") {
        items.entry((c.footprint.clone(), c.value.clone())).or_default().push(c);
    }
    w.open("Bom", &[("name", format!("{step}_BOM"))]);
    w.open("BomHeader", &[("assembly", step.clone()), ("revision", v.revision.clone())]);
    w.empty("StepRef", &[("name", step.clone())]);
    w.close();
    for ((fp, val), cs) in &items {
        let pins = v.pads.iter().filter(|p| p.component == cs[0].id).count();
        w.open("BomItem", &[("OEMDesignNumberRef", format!("{fp}_{val}")), ("quantity", cs.len().to_string()), ("pinCount", pins.to_string()), ("category", "ELECTRICAL".into())]);
        for c in cs {
            w.empty("RefDes", &[("name", c.reference.clone()), ("packageRef", c.footprint.clone()), ("populate", (!c.dnp).to_string()), ("layerRef", if c.side == Side::Top { copper[0].clone() } else { copper[copper.len() - 1].clone() })]);
        }
        w.open("Characteristics", &[("category", "ELECTRICAL".into())]);
        w.empty("Textual", &[("definitionSource", "Forma".into()), ("textualCharacteristicName", "Value".into()), ("textualCharacteristicValue", val.clone())]);
        if let Some(m) = &cs[0].mpn {
            w.empty("Textual", &[("definitionSource", "Forma".into()), ("textualCharacteristicName", "MPN".into()), ("textualCharacteristicValue", m.clone())]);
        }
        w.close();
        w.close();
    }
    w.close();

    w.open("Ecad", &[("name", "Design".into())]);
    w.open("CadHeader", &[("units", "MILLIMETER".into())]);
    w.empty("Spec", &[("name", "Board".to_string())]);
    // Controlled impedances: one spec per class, layer and kind, referenced from its copper layer.
    let imp = crate::signal::impedance_table(board, v);
    for (i, r) in imp.iter().enumerate() {
        w.open("Spec", &[("name", format!("IMP_{}_{}_{}", r.class, r.layer, i + 1))]);
        let tol = r.tolerance.to_string();
        w.open("Impedance", &[("type", "IMPEDANCE".into()), ("transmission", if r.kind == "differential" { "EDGE_COUPLED".into() } else { "SINGLE_ENDED".into() }), ("structure", if r.model == "microstrip" { "MICROSTRIP".into() } else { "STRIPLINE".into() })]);
        w.empty("Property", &[("value", f(r.target)), ("unit", "OHMS".into()), ("tolPlus", tol.clone()), ("tolMinus", tol.clone()), ("tolPercent", "true".into())]);
        w.close();
        w.open("Impedance", &[("type", "LINEWIDTH".into())]);
        w.empty("Property", &[("value", f(r.width)), ("unit", "MM".into())]);
        w.close();
        if let Some(g) = r.gap {
            w.open("Impedance", &[("type", "SPACING".into())]);
            w.empty("Property", &[("value", f(g)), ("unit", "MM".into())]);
            w.close();
        }
        for rf in &r.refs {
            w.open("Impedance", &[("type", "REF_PLANE_LAYER_ID".into())]);
            w.empty("Property", &[("text", layer_name(rf))]);
            w.close();
        }
        w.close();
    }
    w.close();
    w.open("CadData", &[]);
    for (n, func, side) in &layers {
        if *func == "DRILL" {
            let (from, to) = match pairs.iter().find(|(pn, _)| pn == n) {
                Some((_, sp)) => (layer_name(&sp[0]), layer_name(&sp[1])),
                None => (copper[0].clone(), copper[copper.len() - 1].clone()),
            };
            w.open("Layer", &[("name", n.clone()), ("layerFunction", "DRILL".into()), ("side", side.to_string()), ("polarity", "POSITIVE".into())]);
            w.empty("Span", &[("fromLayer", from), ("toLayer", to)]);
            w.close();
        } else {
            w.empty("Layer", &[("name", n.clone()), ("layerFunction", func.to_string()), ("side", side.to_string()), ("polarity", "POSITIVE".into())]);
        }
    }
    // Stackup.
    let cu = 0.035 * v.copper_oz;
    let inner = 0.035 * board.inner_copper_oz;
    let n = copper.len();
    let diel_of = |i: usize| v.stackup.dielectrics.get(i).map(|d| d.thickness).unwrap_or(((v.thickness - 2.0 * cu - (n - 2) as f64 * inner) / (n - 1) as f64).max(0.05));
    w.open("Stackup", &[("name", "Primary_Stackup".into()), ("overallThickness", f(v.thickness)), ("tolPlus", "0".into()), ("tolMinus", "0".into()), ("whereMeasured", "METAL".into())]);
    w.open("StackupGroup", &[("name", "GROUP_PRIMARY".into()), ("thickness", f(v.thickness)), ("tolPlus", "0".into()), ("tolMinus", "0".into())]);
    let mut seq = 1;
    for (i, c) in copper.iter().enumerate() {
        let t = v.stackup.copper_oz.get(i).map(|oz| oz * crate::stackup::OZ_MM).unwrap_or(if i == 0 || i + 1 == n { cu } else { inner });
        let refs: Vec<usize> = imp.iter().enumerate().filter(|(_, r)| layer_name(&r.layer) == *c).map(|(k, _)| k).collect();
        if refs.is_empty() {
            w.empty("StackupLayer", &[("layerOrGroupRef", c.clone()), ("thickness", f(t)), ("tolPlus", "0".into()), ("tolMinus", "0".into()), ("sequence", seq.to_string())]);
        } else {
            w.open("StackupLayer", &[("layerOrGroupRef", c.clone()), ("thickness", f(t)), ("tolPlus", "0".into()), ("tolMinus", "0".into()), ("sequence", seq.to_string())]);
            for k in refs {
                w.empty("SpecRef", &[("id", format!("IMP_{}_{}_{}", imp[k].class, imp[k].layer, k + 1))]);
            }
            w.close();
        }
        seq += 1;
        if i + 1 < n {
            w.empty("StackupLayer", &[("layerOrGroupRef", format!("DIELECTRIC_{}", i + 1)), ("thickness", f(diel_of(i))), ("tolPlus", "0".into()), ("tolMinus", "0".into()), ("sequence", seq.to_string())]);
            seq += 1;
        }
    }
    w.close();
    w.close();
    w.open("Step", &[("name", step.clone())]);
    w.empty("Datum", &[("x", "0".into()), ("y", "0".into())]);
    w.open("Profile", &[]);
    polygon(&mut w, &v.outline);
    for c in &v.cutouts {
        if c.len() >= 3 {
            w.open("Cutout", &[]);
            w.empty("PolyBegin", &[("x", f(c[0][0])), ("y", f(c[0][1]))]);
            for q in c.iter().skip(1).chain(std::iter::once(&c[0])) {
                w.empty("PolyStepSegment", &[("x", f(q[0])), ("y", f(q[1]))]);
            }
            w.close();
        }
    }
    w.close();
    // Packages: one per footprint, pins in footprint coordinates (from the first part using it).
    let mut done: Vec<String> = Vec::new();
    for c in &v.components {
        if done.contains(&c.footprint) {
            continue;
        }
        done.push(c.footprint.clone());
        let pads: Vec<&PadView> = v.pads.iter().filter(|p| p.component == c.id).collect();
        let first = pads.iter().find(|p| p.number == "1").or(pads.first()).map(|p| p.number.clone()).unwrap_or_default();
        w.open("Package", &[("name", c.footprint.clone()), ("type", "OTHER".into()), ("pinOne", first), ("height", f(c.height))]);
        if c.courtyard.len() >= 3 {
            let local: Vec<P> = c.courtyard.iter().map(|q| to_local(c, *q)).collect();
            w.open("Outline", &[]);
            polygon(&mut w, &local);
            w.empty("LineDescRef", &[("id", line_id(0.1))]);
            w.close();
        }
        for (i, p) in v.pads.iter().enumerate().filter(|(_, p)| p.component == c.id) {
            let at = to_local(c, p.at);
            let kind = match p.kind {
                PadKind::Tht => "THRU",
                PadKind::Npth => "THRU",
                PadKind::Smd => "SURFACE",
            };
            w.open("Pin", &[("number", if p.number.is_empty() { format!("NP{i}") } else { p.number.clone() }), ("type", kind.into()), ("electricalType", if p.kind == PadKind::Npth { "MECHANICAL" } else { "ELECTRICAL" }.into())]);
            let r = local_rot(c, p.rotation);
            if r != 0.0 {
                w.empty("Xform", &[("rotation", f(r))]);
            }
            w.empty("Location", &[("x", f(at[0])), ("y", f(at[1]))]);
            prim_ref(&mut w, p, i);
            w.close();
        }
        w.close();
    }
    for c in &v.components {
        w.open("Component", &[("refDes", c.reference.clone()), ("packageRef", c.footprint.clone()), ("layerRef", if c.side == Side::Top { copper[0].clone() } else { copper[n - 1].clone() }), ("mountType", if v.pads.iter().any(|p| p.component == c.id && p.kind == PadKind::Tht) { "THMT" } else { "SMT" }.into()), ("part", if c.value.is_empty() { c.footprint.clone() } else { c.value.clone() })]);
        let mut x = vec![("rotation", f(c.rotation.rem_euclid(360.0)))];
        if c.side == Side::Bottom {
            x.push(("mirror", "true".into()));
        }
        w.empty("Xform", &x);
        w.empty("Location", &[("x", f(c.at[0])), ("y", f(c.at[1]))]);
        w.close();
    }
    for net in &v.nets {
        w.open("LogicalNet", &[("name", net.name.clone())]);
        for p in v.pads.iter().filter(|p| p.net.as_deref() == Some(net.name.as_str())) {
            w.empty("PinRef", &[("componentRef", p.reference.clone()), ("pin", p.number.clone())]);
        }
        w.close();
    }
    // Layer features.
    for (li, l) in v.layers.iter().enumerate() {
        let name = &copper[li];
        w.open("LayerFeature", &[("layerRef", name.clone())]);
        for (i, p) in v.pads.iter().enumerate().filter(|(_, p)| p.layers.contains(l)) {
            let mut a = vec![("geometryUsage", "PAD".to_string())];
            if let Some(nn) = &p.net {
                a.insert(0, ("net", nn.clone()));
            }
            w.open("Set", &a);
            w.open("Pad", &[]);
            if p.rotation != 0.0 && p.polygon.is_none() {
                w.empty("Xform", &[("rotation", f(p.rotation))]);
            }
            w.empty("Location", &[("x", f(p.at[0])), ("y", f(p.at[1]))]);
            prim_ref(&mut w, p, i);
            w.empty("PinRef", &[("componentRef", p.reference.clone()), ("pin", p.number.clone())]);
            w.close();
            w.close();
        }
        for x in &v.vias {
            let Some(dia) = crate::gerber::via_pad(v, x, l) else { continue };
            let mut a = vec![("padUsage", "VIA".to_string())];
            if let Some(nn) = &x.net {
                a.insert(0, ("net", nn.clone()));
            }
            w.open("Set", &a);
            w.open("Pad", &[]);
            w.empty("Location", &[("x", f(x.at[0])), ("y", f(x.at[1]))]);
            w.empty("StandardPrimitiveRef", &[("id", format!("CIRCLE_{}", f(dia)))]);
            w.close();
            w.close();
        }
        for t in v.tracks.iter().filter(|t| t.layer == *l && t.points.len() >= 2) {
            let a = t.net.as_ref().map(|nn| vec![("net", nn.clone())]).unwrap_or_default();
            w.open("Set", &a);
            w.open("Features", &[]);
            w.open("Polyline", &[]);
            w.empty("PolyBegin", &[("x", f(t.points[0][0])), ("y", f(t.points[0][1]))]);
            for q in &t.points[1..] {
                w.empty("PolyStepSegment", &[("x", f(q[0])), ("y", f(q[1]))]);
            }
            w.empty("LineDescRef", &[("id", line_id(t.width))]);
            w.close();
            w.close();
            w.close();
        }
        for z in v.zones.iter().filter(|z| z.layer == *l) {
            for island in &z.fill {
                if island.is_empty() {
                    continue;
                }
                w.open("Set", &[("net", z.net.clone()), ("geometryUsage", "AREA".into())]);
                w.open("Features", &[]);
                contour(&mut w, &island[0], &island[1..]);
                w.close();
                w.close();
            }
        }
        for t in v.teardrops.iter().filter(|t| t.layer == *l) {
            let a = t.net.as_ref().map(|nn| vec![("net", nn.clone())]).unwrap_or_default();
            w.open("Set", &a);
            w.open("Features", &[]);
            contour(&mut w, &t.polygon, &[]);
            w.close();
            w.close();
        }
        for g in v.graphics.iter().filter(|g| g.layer == *l) {
            let a = g.net.as_ref().map(|nn| vec![("net", nn.clone())]).unwrap_or_default();
            w.open("Set", &a);
            w.open("Features", &[]);
            graphic(&mut w, g);
            w.close();
            w.close();
        }
        w.close();
    }
    // Mask and paste openings, silkscreen.
    let e = v.rules.mask_expansion;
    for (side, layer_idx) in [(Side::Top, 0usize), (Side::Bottom, n - 1)] {
        let l = &v.layers[layer_idx];
        let sfx = if side == Side::Top { "TOP" } else { "BOTTOM" };
        w.open("LayerFeature", &[("layerRef", format!("SOLDERMASK_{sfx}"))]);
        w.open("Set", &[]);
        w.open("Features", &[]);
        for p in v.pads.iter().filter(|p| p.layers.contains(l) || p.kind == PadKind::Npth) {
            let pts = match &p.polygon {
                Some(pg) => pg.clone(),
                None => Prim { pts: p.prim.pts.clone(), r: p.prim.r + if p.kind == PadKind::Npth { 0.0 } else { e } }.outline(0.01),
            };
            contour(&mut w, &pts, &[]);
        }
        w.close();
        w.close();
        w.close();
        w.open("LayerFeature", &[("layerRef", format!("SOLDERPASTE_{sfx}"))]);
        w.open("Set", &[]);
        w.open("Features", &[]);
        for p in v.pads.iter().filter(|p| p.kind == PadKind::Smd && p.side == side) {
            for q in &p.paste {
                contour(&mut w, &q.outline(0.01), &[]);
            }
        }
        w.close();
        w.close();
        w.close();
        w.open("LayerFeature", &[("layerRef", format!("SILKSCREEN_{sfx}"))]);
        w.open("Set", &[]);
        w.open("Features", &[]);
        for c in v.components.iter().filter(|c| c.side == side) {
            for line in &c.silk {
                if line.len() < 2 {
                    continue;
                }
                w.open("Polyline", &[]);
                w.empty("PolyBegin", &[("x", f(line[0][0])), ("y", f(line[0][1]))]);
                for q in &line[1..] {
                    w.empty("PolyStepSegment", &[("x", f(q[0])), ("y", f(q[1]))]);
                }
                w.empty("LineDescRef", &[("id", line_id(v.rules.silk_width))]);
                w.close();
            }
        }
        for g in v.graphics.iter().filter(|g| g.layer == if side == Side::Top { "silk_top" } else { "silk_bottom" }) {
            graphic(&mut w, g);
        }
        w.close();
        w.close();
        w.close();
    }
    // Holes: through holes, then each blind/buried pair's.
    for (layer_ref, span) in std::iter::once((drill, None)).chain(pairs.iter().map(|(n, sp)| (n.clone(), Some(sp.clone())))) {
    w.open("LayerFeature", &[("layerRef", layer_ref)]);
    for (i, h) in v.holes.iter().enumerate().filter(|(_, h)| h.span == span) {
        let net = if h.owner == "via" { v.vias.iter().find(|x| (x.at[0] - h.at[0]).abs() < 1e-6 && (x.at[1] - h.at[1]).abs() < 1e-6).and_then(|x| x.net.clone()) } else { v.pads.iter().find(|p| format!("{}.{}", p.reference, p.number) == h.owner).and_then(|p| p.net.clone()) };
        let mut a = vec![("geometry", if h.owner == "via" { "VIA" } else { "COMPONENT_HOLE" }.to_string())];
        if let Some(nn) = net {
            a.insert(0, ("net", nn));
        }
        w.open("Set", &a);
        let d = h.size[0].min(h.size[1]);
        let status = if !h.plated { "NONPLATED" } else if h.owner == "via" { "VIA" } else { "PLATED" };
        if (h.size[0] - h.size[1]).abs() > 1e-9 {
            w.open("SlotCavity", &[("name", format!("S{i}")), ("platingStatus", status.into()), ("plusTol", "0".into()), ("minusTol", "0".into())]);
            w.empty("Location", &[("x", f(h.at[0])), ("y", f(h.at[1]))]);
            w.empty("Oval", &[("width", f(h.size[0])), ("height", f(h.size[1]))]);
            w.close();
        } else {
            w.empty("Hole", &[("name", format!("H{i}")), ("diameter", f(d)), ("platingStatus", status.into()), ("plusTol", "0".into()), ("minusTol", "0".into()), ("x", f(h.at[0])), ("y", f(h.at[1]))]);
        }
        w.close();
    }
    w.close();
    }
    w.close(); // Step
    w.close(); // CadData
    w.close(); // Ecad
    w.finish()
}

fn prim_ref(w: &mut Writer, p: &PadView, i: usize) {
    if p.polygon.is_some() {
        w.empty("UserPrimitiveRef", &[("id", format!("CUSTOM_{i}"))]);
    } else {
        w.empty("StandardPrimitiveRef", &[("id", primitive(p).0)]);
    }
}

/// A board point in the component's own coordinates (bottom parts unmirrored).
fn to_local(c: &crate::board::ComponentView, p: P) -> P {
    let q = crate::geom::rotate(crate::geom::sub(p, c.at), -c.rotation);
    if c.side == Side::Bottom { [-q[0], q[1]] } else { q }
}

fn local_rot(c: &crate::board::ComponentView, r: f64) -> f64 {
    let l = (r - c.rotation).rem_euclid(360.0);
    let l = if c.side == Side::Bottom { (-l).rem_euclid(360.0) } else { l };
    if (l - 360.0).abs() < 1e-9 { 0.0 } else { (l * 1e4).round() / 1e4 }
}
