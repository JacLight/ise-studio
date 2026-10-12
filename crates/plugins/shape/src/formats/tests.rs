//! Round trips through every format: a box and a cylinder out and back in,
//! measured; and 2D drawings read into sketches whose outlines are measured.

use super::*;
use manifold_csg::Manifold;

fn parts() -> (Vec<MeshBody>, f64, f64) {
    let b = Manifold::cube(30.0, 20.0, 10.0, false);
    let c = Manifold::cylinder(15.0, 4.0, 4.0, 64, false).translate(50.0, 0.0, 0.0);
    let (vb, vc) = (b.volume(), c.volume());
    let red = Look { color: [0.8, 0.2, 0.4], metalness: 0.0, roughness: 0.5, opacity: 1.0 };
    (vec![MeshBody::of_solid("Base plate", &b, Some(red), false), MeshBody::of_solid("Pin", &c, None, false)], vb, vc)
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

fn bounds(b: &MeshBody) -> [f64; 6] {
    b.positions.chunks_exact(3).fold([f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |m, v| {
        [m[0].min(v[0]), m[1].min(v[1]), m[2].min(v[2]), m[3].max(v[0]), m[4].max(v[1]), m[5].max(v[2])]
    })
}

#[test]
fn stl_binary_and_text_round_trip() {
    let (p, vb, _) = parts();
    let solid = p[0].to_solid().unwrap();
    let bin = crate::stl_of(&solid, "Forma Base plate", [0.0; 3]);
    assert_eq!(Kind::sniff(&bin), Some(Kind::Stl));
    let back = read_mesh(&bin, Kind::Stl).unwrap();
    assert_eq!(back.len(), 1);
    assert!(close(back[0].to_solid().unwrap().volume(), vb, 1e-6));
    // Text STL, two solids, each named.
    let mut text = String::new();
    for b in &p {
        text.push_str(&format!("solid {}\n", b.name));
        for t in b.triangles.chunks_exact(3) {
            text.push_str(" facet normal 0 0 0\n  outer loop\n");
            for &i in t {
                let v = &b.positions[i as usize * 3..i as usize * 3 + 3];
                text.push_str(&format!("   vertex {} {} {}\n", v[0], v[1], v[2]));
            }
            text.push_str("  endloop\n endfacet\n");
        }
        text.push_str(&format!("endsolid {}\n", b.name));
    }
    assert_eq!(Kind::sniff(text.as_bytes()), Some(Kind::Stl));
    let back = read_mesh(text.as_bytes(), Kind::Stl).unwrap();
    assert_eq!(back.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Base plate", "Pin"]);
    assert!(close(back[0].to_solid().unwrap().volume(), vb, 1e-9));
}

#[test]
fn obj_round_trip_keeps_objects_and_colours() {
    let (p, vb, vc) = parts();
    let obj = mesh::write_obj(&p);
    assert_eq!(Kind::sniff(&obj), Some(Kind::Obj));
    let back = read_mesh(&obj, Kind::Obj).unwrap();
    assert_eq!(back.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Base plate", "Pin"]);
    assert!(close(back[0].to_solid().unwrap().volume(), vb, 1e-9));
    assert!(close(back[1].to_solid().unwrap().volume(), vc, 1e-9));
    assert_eq!(back[0].look.as_ref().map(Look::hex).as_deref(), Some("#cc3366"));
    assert!(back[1].look.is_none());
    assert!((bounds(&back[1])[0] - 46.0).abs() < 1e-9, "placed where it was");
}

#[test]
fn ply_round_trip_binary_and_text() {
    let (p, vb, vc) = parts();
    let ply = mesh::write_ply(&p);
    assert_eq!(Kind::sniff(&ply), Some(Kind::Ply));
    let back = read_mesh(&ply, Kind::Ply).unwrap();
    assert_eq!(back.len(), 1, "PLY holds one mesh");
    assert!(close(back[0].to_solid().unwrap().volume(), vb + vc, 1e-5));
    let text = "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 4\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255 0 0\n10 0 0 255 0 0\n0 10 0 255 0 0\n0 0 10 255 0 0\n3 0 2 1\n3 0 1 3\n3 0 3 2\n3 1 2 3\n";
    let back = read_mesh(text.as_bytes(), Kind::Ply).unwrap();
    assert!(close(back[0].to_solid().unwrap().volume(), 1000.0 / 6.0, 1e-9));
    assert_eq!(back[0].look.as_ref().map(Look::hex).as_deref(), Some("#ff0000"));
}

#[test]
fn glb_round_trip_keeps_nodes_names_colours_and_units() {
    let (mut p, vb, vc) = parts();
    p[1] = MeshBody::of_solid("Pin", &p[1].to_solid().unwrap(), None, true);
    assert_eq!(p[1].normals.len(), p[1].positions.len(), "normals for every vertex");
    let glb = gltf::write_glb(&p);
    assert_eq!(Kind::sniff(&glb), Some(Kind::Glb));
    let back = read_mesh(&glb, Kind::Glb).unwrap();
    assert_eq!(back.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Base plate", "Pin"]);
    // Metres as f32 there: a tenth of a micron here.
    assert!(close(back[0].to_solid().unwrap().volume(), vb, 1e-5));
    assert!(close(back[1].to_solid().unwrap().volume(), vc, 1e-5));
    let b = bounds(&back[0]);
    assert!(b.iter().zip([0.0, 0.0, 0.0, 30.0, 20.0, 10.0]).all(|(x, w)| (x - w).abs() < 1e-4), "Z up, millimetres: {b:?}");
    let look = back[0].look.as_ref().unwrap();
    assert_eq!(look.hex(), "#cc3366");
    assert!((look.roughness - 0.5).abs() < 1e-9);
    assert_eq!(back[1].look.as_ref().unwrap().hex(), Look::DEFAULT.hex(), "an unassigned part gets the view's grey");
    // The same scene as .gltf with the buffer embedded reads the same.
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let mut doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let bin = &glb[20 + json_len + 8..];
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut enc = String::new();
    for c in bin.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            enc.push(if k <= c.len() { B64[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    doc["buffers"][0]["uri"] = serde_json::json!(format!("data:application/octet-stream;base64,{enc}"));
    let text = doc.to_string();
    assert_eq!(Kind::sniff(text.as_bytes()), Some(Kind::Gltf));
    let again = read_mesh(text.as_bytes(), Kind::Gltf).unwrap();
    assert!(close(again[1].to_solid().unwrap().volume(), vc, 1e-5));
}

#[test]
fn three_mf_round_trip() {
    let (p, _, vc) = parts();
    let pin = p[1].to_solid().unwrap();
    let file = crate::fabricate::three_mf("Pin", &pin);
    assert_eq!(Kind::sniff(&file), Some(Kind::ThreeMf));
    let back = read_mesh(&file, Kind::ThreeMf).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].name, "Pin");
    assert!(close(back[0].to_solid().unwrap().volume(), vc, 1e-5));
    assert!(bounds(&back[0])[2].abs() < 1e-9, "a 3MF rests on the plate");
}

#[test]
fn three_mf_components_colours_and_units() {
    // A Bambu-style project: the build item is an assembly of a component in another model file, in centimetres, coloured.
    let root = r##"<?xml version="1.0"?><model unit="centimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
<resources><basematerials id="5"><base name="Orange" displaycolor="#E8742CFF"/></basematerials>
<object id="2" type="model" name="Bracket" pid="5" pindex="0"><components><component p:path="/3D/Objects/object_1.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 1 0 0"/></components></object></resources>
<build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 0 0"/></build></model>"##;
    let part = r#"<?xml version="1.0"?><model unit="centimeter"><resources><object id="1" type="model"><mesh><vertices>
<vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices>
<triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="0" v2="3" v3="2"/><triangle v1="1" v2="2" v3="3"/></triangles></mesh></object></resources></model>"#;
    let rels = r#"<Relationships><Relationship Target="/3D/3dmodel.model" Id="r" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
    let file = crate::fabricate::zip(&[("_rels/.rels".into(), rels.into()), ("3D/3dmodel.model".into(), root.into()), ("3D/Objects/object_1.model".into(), part.into())]);
    let back = read_mesh(&file, Kind::ThreeMf).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].name, "Bracket");
    assert_eq!(back[0].look.as_ref().unwrap().hex(), "#e8742c");
    assert!(close(back[0].to_solid().unwrap().volume(), 1000.0 / 6.0, 1e-9), "1 cm³/6 in mm³");
    assert!((bounds(&back[0])[0] - 10.0).abs() < 1e-9, "moved 1 cm by its component");
}

#[test]
fn open_and_inside_out_meshes() {
    let (p, vb, _) = parts();
    let mut open = p[0].clone();
    open.triangles.truncate(open.triangles.len() - 3);
    assert!(open.to_solid().unwrap_err().contains("not a closed solid"));
    let mut flipped = p[0].clone();
    for t in flipped.triangles.chunks_exact_mut(3) {
        t.swap(1, 2);
    }
    assert!(close(flipped.to_solid().unwrap().volume(), vb, 1e-9), "turned right way out");
}

/// The net area of a sketch's closed outlines (holes subtract) and how many there are.
fn sketch_area(g: &crate::sketch::Geometry) -> (f64, usize, usize) {
    let solved = crate::sketch::solve(g, &Default::default()).unwrap();
    let (loops, open) = crate::sketch::loops(g, &solved);
    let area: f64 = loops
        .iter()
        .map(|l| {
            let p = crate::sketch::polygon(l, 0.001);
            (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum::<f64>() / 2.0
        })
        .sum();
    (area, loops.len(), open)
}

#[test]
fn dxf_becomes_an_editable_sketch() {
    // A slot (polyline with two half-circle bulges) with a round hole, a
    // separate square of lines, a quarter-circle ARC closing a wedge, in inches.
    let dxf = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
0\nLWPOLYLINE\n90\n4\n70\n1\n10\n0\n20\n0\n10\n4\n20\n0\n42\n1\n10\n4\n20\n1\n10\n0\n20\n1\n42\n1\n\
0\nCIRCLE\n10\n2\n20\n0.5\n40\n0.25\n\
0\nLINE\n10\n10\n20\n0\n11\n12\n21\n0\n0\nLINE\n10\n10\n20\n0\n11\n10\n21\n2\n\
0\nARC\n10\n10\n20\n0\n40\n2\n50\n0\n51\n90\n\
0\nENDSEC\n0\nEOF\n";
    assert_eq!(Kind::sniff(dxf.as_bytes()), Some(Kind::Dxf));
    let (g, counts, size) = read_sketch(dxf.as_bytes(), Kind::Dxf).unwrap();
    assert_eq!((counts.lines, counts.arcs, counts.circles), (4, 3, 1), "{counts:?}");
    let inch2 = 25.4 * 25.4;
    let slot = 4.0 + std::f64::consts::PI * 0.25 - std::f64::consts::PI * 0.0625;
    let wedge = std::f64::consts::PI * 4.0 / 4.0;
    let (area, loops, open) = sketch_area(&g);
    assert_eq!((loops, open), (3, 0));
    assert!(close(area, (slot + wedge) * inch2, 2e-5), "area {area} vs {}", (slot + wedge) * inch2);
    assert!(close(size[0], 12.5 * 25.4, 1e-9), "from the slot's left end to the wedge: {size:?}");
}

#[test]
fn dxf_blocks_and_splines() {
    // A block holding a square, inserted twice (once turned 90° and doubled); a cubic spline closed by a line.
    let dxf = "0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n2\nSQ\n10\n0\n20\n0\n\
0\nLWPOLYLINE\n90\n4\n70\n1\n10\n0\n20\n0\n10\n1\n20\n0\n10\n1\n20\n1\n10\n0\n20\n1\n0\nENDBLK\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n0\nINSERT\n2\nSQ\n10\n5\n20\n5\n0\nINSERT\n2\nSQ\n10\n20\n20\n5\n41\n2\n42\n2\n50\n90\n\
0\nSPLINE\n70\n8\n71\n3\n72\n8\n73\n4\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n10\n0\n20\n-10\n10\n0\n20\n-14\n10\n10\n20\n-14\n10\n10\n20\n-10\n\
0\nLINE\n10\n0\n20\n-10\n11\n10\n21\n-10\n0\nENDSEC\n0\nEOF\n";
    let (g, counts, _) = read_sketch(dxf.as_bytes(), Kind::Dxf).unwrap();
    assert_eq!((counts.lines, counts.curves), (9, 1), "{counts:?}");
    let (area, loops, open) = sketch_area(&g);
    assert_eq!((loops, open), (3, 0));
    // Two squares (1 and, doubled, 4) and the cubic's bulge below its chord (3/5 · 10 · 4 = 24).
    assert!(close(area, 1.0 + 4.0 + 24.0, 1e-3), "area {area}");
}

#[test]
fn svg_becomes_an_editable_sketch() {
    let svg = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 200 100">
<defs><rect width="999" height="999"/></defs>
<rect x="0" y="0" width="200" height="100" rx="20"/>
<g transform="translate(100 50)"><circle r="20"/></g>
<path d="M 20 50 a 10 10 0 0 1 20 0 a 10 10 0 0 1 -20 0 z"/>
<path d="M170,30 c0,-10 10,-10 10,0 s-10,10 -10,0z" style="display: none"/>
</svg>"#;
    assert_eq!(Kind::sniff(svg.as_bytes()), Some(Kind::Svg));
    let (g, counts, size) = read_sketch(svg.as_bytes(), Kind::Svg).unwrap();
    assert_eq!((counts.lines, counts.arcs, counts.circles, counts.curves), (4, 6, 1, 0), "{counts:?}");
    assert!(close(size[0], 100.0, 1e-9) && close(size[1], 50.0, 1e-9), "{size:?}");
    let pi = std::f64::consts::PI;
    let want = (5000.0 - (4.0 - pi) * 100.0) - pi * 100.0 - pi * 25.0;
    let (area, loops, open) = sketch_area(&g);
    assert_eq!((loops, open), (3, 0));
    assert!(close(area, want, 2e-5), "area {area} vs {want}");
}

#[test]
fn svg_curves_transforms_and_px() {
    // 96 px to the inch; a skewed circle is no longer round: Béziers.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0 H96 V96 Q48 144 0 96 Z"/><circle cx="200" cy="0" r="10" transform="skewX(20)"/></svg>"#;
    let (g, counts, _) = read_sketch(svg.as_bytes(), Kind::Svg).unwrap();
    assert_eq!((counts.lines, counts.curves), (3, 5), "{counts:?}");
    let (area, loops, _) = sketch_area(&g);
    assert_eq!(loops, 2);
    let mm = 25.4 / 96.0;
    // Square plus the quadratic's bulge (2/3 · chord · sagitta), and the skewed circle (a shear keeps area).
    let want = (96.0 * 96.0 + 2.0 / 3.0 * 96.0 * 24.0 + std::f64::consts::PI * 100.0) * mm * mm;
    assert!(close(area, want, 1e-3), "area {area} vs {want}");
}

#[test]
fn dwg_and_unknown_files_say_why() {
    assert_eq!(Kind::from_name("part.STP"), Some(Kind::Step));
    assert_eq!(Kind::from_name("part.igs"), Some(Kind::Iges));
    assert_eq!(Kind::from_name("plan.dwg"), None);
    assert!(DWG.contains("DXF"));
}

/// A file stored as an import would store it (the asset folder is set once per process).
fn stored(bytes: &[u8], hint: Kind) -> String {
    if crate::assets::dir().is_none() {
        crate::assets::set_dir(std::env::temp_dir().join(format!("forma-format-assets-{}", std::process::id())));
    }
    // Not a real SHA-256: tests only need a stable 64-hex name per content.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    let sha = format!("{:016x}", h.finish()).repeat(4);
    crate::assets::store(&sha, bytes, Some(hint)).unwrap();
    sha
}

#[test]
fn programs_read_imported_mesh_and_iges_files() {
    let (p, vb, vc) = parts();
    let obj = stored(&mesh::write_obj(&p), Kind::Obj);
    assert_eq!(crate::assets::kind(&obj), Ok(Kind::Obj));
    let source = format!("return importMesh(\"{obj}\");");
    assert_eq!(crate::assets::referenced(&source), vec![obj.clone()]);
    let e = crate::runtime::evaluate(&source, &[], 0.01).unwrap();
    let names: Vec<&str> = e.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Base plate", "Pin"]);
    assert_eq!(e.parts[0].material.as_deref(), Some("Imported #cc3366"), "the file's colour, as a material name the view reads");
    assert!(close(e.parts[1].volume, vc, 1e-9));
    // Exactly: the same parts, faceted, and said to be.
    let x = crate::exact::evaluate_exact("Imported", &source, &[], 0.01).unwrap();
    assert_eq!(x.parts.len(), 2);
    assert!(close(x.arena.volume(x.parts[0].shape), vb, 1e-6));
    assert!(x.faceted.iter().any(|f| f.contains("importMesh")));
    // An IGES file through the same exact path as STEP, and meshed for the view.
    let mut a = de_occt::Arena::new();
    let cyl = a.cylinder(15.0, 4.0, 4.0, false).unwrap();
    let path = std::env::temp_dir().join(format!("forma-format-iges-{}.igs", std::process::id()));
    a.write_cad(&[("Pin".into(), cyl, Some([0.2, 0.4, 0.8]))], &path, de_occt::CadFormat::Iges).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(Kind::sniff(&bytes), Some(Kind::Iges));
    let iges = stored(&bytes, Kind::Iges);
    let source = format!("return importIges(\"{iges}\");");
    let x = crate::exact::evaluate_exact("Pin", &source, &[], 0.01).unwrap();
    assert!((x.arena.volume(x.parts[0].shape) - std::f64::consts::PI * 16.0 * 15.0).abs() < 1e-3, "exact: a true cylinder");
    let e = crate::runtime::evaluate(&source, &[], 0.01).unwrap();
    assert_eq!(e.parts[0].name, "Pin");
    assert_eq!(e.parts[0].material.as_deref(), Some("Imported #3366cc"));
    assert!(close(e.parts[0].volume, std::f64::consts::PI * 16.0 * 15.0, 1e-2));
}
