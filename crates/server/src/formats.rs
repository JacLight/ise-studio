//! Files in and out beyond the fabrication package: every format Forma reads
//! becomes a part or a sketch (`POST /import/<file>`), and a design leaves as
//! exact STEP or IGES, or as an OBJ, PLY or glTF mesh (`GET /export/<id>.<ext>`).

use crate::{Actor, Server, Source, StableId, fabricate, json};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use de_core::Value;
use de_shape::formats::{self, Becomes, Kind, Look, MeshBody};
use std::{collections::HashMap, sync::Arc};

/// The formats an import takes, for messages.
pub const ACCEPTED: &str = "STEP, IGES, STL, OBJ, PLY, glTF/GLB, 3MF, DXF and SVG";

impl Source {
    /// The look of one of this source's parts by its material name (the
    /// table's material, else the name itself: "PLA black", "#c8372f"); None
    /// when it has no material at all.
    pub(crate) fn look(&self, material: Option<&str>) -> Option<Look> {
        let (name, _) = self.material(material);
        (!name.trim().is_empty()).then(|| formats::looks::look(&name))
    }

    /// Every part as a mesh where it sits in the design (assemblies: every
    /// part of every member, placed), named, with its material's look.
    fn meshes(&self, tolerance: f64, normals: bool, out: &mut Vec<MeshBody>) -> Result<(), String> {
        for m in &self.members {
            m.meshes(tolerance, normals, out)?;
        }
        if self.program.is_none() && self.boxed.is_none() {
            return Ok(());
        }
        let place = de_assembly::Transform { at: self.at, rot: self.rotation };
        for item in self.items(tolerance)? {
            let name = if self.prefix.is_empty() { item.name.clone() } else { format!("{} › {}", self.prefix, item.name) };
            out.push(MeshBody::of_solid(&name, &place.solid(&item.solid), self.look(item.material.as_deref()), normals));
        }
        Ok(())
    }
}

/// `/export/<id>.<obj|ply|glb>[?part=<name>]`: the design's parts as one mesh
/// file, placed (or one part, at the origin).
pub(crate) fn mesh_file(s: &Source, ext: &str, part: Option<&str>, tolerance: f64) -> Result<(String, &'static str, Vec<u8>), String> {
    let normals = ext == "glb";
    let mut parts = Vec::new();
    let name = match part {
        Some(p) => {
            let items = s.items(tolerance)?;
            let item = items.iter().find(|i| i.name == p).ok_or_else(|| format!("{} has no part named {p}", s.name))?;
            parts.push(MeshBody::of_solid(&item.name, &fabricate::at_origin(&item.solid), s.look(item.material.as_deref()), normals));
            item.name.clone()
        }
        None => {
            s.meshes(tolerance, normals, &mut parts)?;
            s.name.clone()
        }
    };
    if parts.is_empty() {
        return Err("nothing to export".into());
    }
    let base = fabricate::file_name(&name);
    Ok(match ext {
        "obj" => (format!("{base}.obj"), "model/obj", formats::mesh::write_obj(&parts)),
        "ply" => (format!("{base}.ply"), "application/x-ply", formats::mesh::write_ply(&parts)),
        _ => (format!("{base}.glb"), "model/gltf-binary", formats::gltf::write_glb(&parts)),
    })
}

/// `/export/<id>.<step|iges>[?part=<name>]`: exact B-rep, every part named
/// and coloured by its material's look; one part at the origin, or the whole
/// design placed.
pub(crate) fn cad_file(s: &Source, part: Option<&str>, tolerance: f64, format: de_occt::CadFormat) -> Result<(String, &'static str, Vec<u8>), String> {
    let (ext, mime) = if format == de_occt::CadFormat::Iges { ("igs", "model/iges") } else { ("step", "model/step") };
    let mut maker = fabricate::StepMaker::new(tolerance);
    if let Some(name) = part {
        let program = s.program.as_ref().ok_or("pick a part of a shape")?;
        maker.parts(program)?;
        let color = s.look(maker.material(program, name).as_deref()).map(|l| l.color);
        let bytes = maker.part_file(program, name, color, format)?;
        return Ok((format!("{}.{ext}", fabricate::file_name(name)), mime, bytes));
    }
    let mut shapes = Vec::new();
    let mut faceted = Vec::new();
    s.exact(&mut maker, &mut shapes, &mut faceted)?;
    if shapes.is_empty() {
        return Err("nothing to export".into());
    }
    if !faceted.is_empty() {
        eprintln!("{ext} export of {}: faceted where the program built geometry itself: {}", s.name, faceted.join("; "));
    }
    Ok((format!("{}.{ext}", fabricate::file_name(&s.name)), mime, maker.file(&shapes, format)?))
}

/// `POST /import/<file name>?project=…&user=…` with the file as the body:
/// stores it by content, checks it reads, and adds it to the project. STEP
/// and IGES become a shape of the file's exact bodies; mesh files (STL, OBJ,
/// PLY, glTF, 3MF) a shape of its meshes; DXF and SVG drawings a sketch, its
/// lines, arcs and circles kept as such, ready to extrude.
pub(crate) async fn import(State(server): State<Arc<Server>>, Path(file): Path<String>, Query(query): Query<HashMap<String, String>>, body: axum::body::Bytes) -> axum::response::Response {
    let fail = |code: StatusCode, message: String| (code, message).into_response();
    if file.to_lowercase().ends_with(".dwg") {
        return fail(StatusCode::UNSUPPORTED_MEDIA_TYPE, format!("{}.", formats::DWG));
    }
    let Some(kind) = Kind::sniff(&body).or(Kind::from_name(&file)) else {
        return fail(StatusCode::BAD_REQUEST, format!("Forma imports {ACCEPTED} files"));
    };
    let app = match server.project(query.get("project")) {
        Ok(app) => app,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    use sha2::Digest;
    let sha: String = sha2::Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
    let name = file.rsplit_once('.').map(|(n, _)| n).unwrap_or(&file).replace(['_', '-', '.'], " ").trim().to_string();
    let name = if name.is_empty() { "Imported".to_string() } else { name };
    let user = query.get("user").filter(|u| !u.is_empty()).cloned().unwrap_or_else(|| "local".into());
    let actor = Actor { user_id: user, session_id: StableId::new().to_string() };
    let commit = |operation: &str, parameters: Value| -> Result<Value, String> {
        let revision = app.project.lock().unwrap().revision();
        let message = json!({ "type": "commit", "request_id": 1, "operation": operation, "expected_revision": revision, "parameters": parameters });
        let reply: Value = serde_json::from_str(&app.handle(&actor, &message.to_string())).unwrap_or_default();
        if reply["ok"] == true { Ok(reply["value"]["output"].clone()) } else { Err(reply["error"]["message"].as_str().unwrap_or("the import failed").to_string()) }
    };

    if kind.becomes() == Becomes::Sketch {
        // A drawing becomes a sketch: its geometry lives in the project, so the file itself is not kept.
        let read = tokio::task::spawn_blocking(move || formats::read_sketch(&body, kind)).await.unwrap_or_else(|e| Err(e.to_string()));
        let (geometry, counts, size) = match read {
            Ok(r) => r,
            Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{file} could not be read: {e}")),
        };
        // Sketches are found by name: a second import of the same drawing gets its own.
        let taken: Vec<String> = {
            let project = app.project.lock().unwrap();
            project.entity_ids().into_iter().filter_map(|id| project.get::<de_shape::sketch::Sketch>(id).map(|s| s.name.clone())).collect()
        };
        let mut unique = name.clone();
        let mut n = 1;
        while taken.contains(&unique) {
            n += 1;
            unique = format!("{name} {n}");
        }
        return match commit("sketch.create", json!({ "name": unique, "plane": { "name": "XY" }, "geometry": geometry })) {
            Ok(out) => (StatusCode::OK, axum::Json(json!({ "id": out["id"], "kind": "sketch", "name": unique, "format": kind.ext(), "counts": counts, "size": size }))).into_response(),
            Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
        };
    }

    let dir = server.dir.join("assets");
    let stored = dir.join(format!("{sha}.{}", kind.ext()));
    let existed = stored.exists();
    if !existed && let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&stored, &body)) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, format!("could not store the file: {e}"));
    }
    // Read it once now: a file that does not read is refused, not stored as a broken part.
    let check = {
        let stored = stored.clone();
        // Bodies, bounds, and the colours the file gives its bodies.
        tokio::task::spawn_blocking(move || -> Result<(usize, [f64; 6], Vec<(String, String)>), String> {
            let mut b = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            let mut grow = |x: [f64; 6]| {
                for k in 0..3 {
                    b[k] = b[k].min(x[k]);
                    b[k + 3] = b[k + 3].max(x[k + 3]);
                }
            };
            let mut colors = Vec::new();
            let count = if kind.becomes() == Becomes::Exact {
                let mut a = de_occt::Arena::new();
                let format = if kind == Kind::Iges { de_occt::CadFormat::Iges } else { de_occt::CadFormat::Step };
                let bodies = a.read_cad(&stored, format)?;
                for body in &bodies {
                    grow(a.bounds(body.shape)?);
                    colors.extend(body.color.map(|c| (body.name.clone(), de_shape::assets::hex(c))));
                }
                bodies.len()
            } else {
                let bodies = formats::read_mesh(&body, kind)?;
                for (i, m) in bodies.iter().enumerate() {
                    let name = if m.name.trim().is_empty() { format!("Body {}", i + 1) } else { m.name.trim().to_string() };
                    let solid = m.to_solid().map_err(|e| if bodies.len() > 1 { format!("{name}: {e}") } else { e })?;
                    if let Some(bb) = solid.bounding_box() {
                        grow([bb.min()[0], bb.min()[1], bb.min()[2], bb.max()[0], bb.max()[1], bb.max()[2]]);
                    }
                    colors.extend(m.look.as_ref().map(|l| (name, l.hex())));
                }
                bodies.len()
            };
            if count == 0 {
                return Err("the file has no solid bodies".into());
            }
            Ok((count, b, colors))
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    };
    let (count, bounds, colors) = match check {
        Ok(c) => c,
        Err(e) => {
            if !existed {
                let _ = std::fs::remove_file(&stored);
            }
            return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("{file} could not be read: {e}"));
        }
    };
    // Show it about as finely as its size calls for (a bolt to ±0.01 mm, a building to ±5).
    let size = (0..3).map(|k| bounds[k + 3] - bounds[k]).fold(0.0, f64::max);
    let shown = (size / 4000.0).clamp(0.005, 5.0);
    let bodies = if count == 1 { "body" } else { "bodies" };
    // The file's colours, for whoever gives the parts their materials (a colour says nothing of density).
    let palette = if colors.is_empty() {
        String::new()
    } else {
        let shown: Vec<String> = colors.iter().take(12).map(|(n, c)| format!("{n} {c}")).collect();
        format!("\n// Colours in the file: {}{}.", shown.join(", "), if colors.len() > 12 { ", …" } else { "" })
    };
    let source = match kind {
        Kind::Step => format!("// Imported from “{file}” ({count} {bodies}). The geometry is the file's own exact B-rep:\n// the view meshes it, STEP exports pass it through untouched.{palette}\nreturn importStep(\"{sha}\");"),
        Kind::Iges => format!("// Imported from “{file}” ({count} {bodies}). The geometry is the file's own exact surfaces,\n// sewn into solids: the view meshes it, STEP and IGES exports pass it through.{palette}\nreturn importIges(\"{sha}\");"),
        _ => format!("// Imported from “{file}” ({count} {bodies}), a mesh: triangles, not exact surfaces.\n// Booleans, placement and fit checks work on it; fillets and exact edges do not.{palette}\nreturn importMesh(\"{sha}\");"),
    };
    match commit("shape.create", json!({ "name": name, "description": format!("Imported from {file}."), "source": source, "params": [], "display_tolerance": shown })) {
        Ok(out) => (StatusCode::OK, axum::Json(json!({ "id": out["id"], "kind": "shape", "bodies": count, "sha": sha, "format": kind.ext(), "exact": kind.becomes() == Becomes::Exact, "colors": colors.iter().map(|(n, c)| json!({ "part": n, "color": c })).collect::<Vec<_>>() }))).into_response(),
        Err(e) => fail(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{serve, server_in};
    use manifold_csg::Manifold;

    async fn http(at: std::net::SocketAddr, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(at).await.unwrap();
        s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: {at}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        s.write_all(body).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let status = String::from_utf8_lossy(&buf[..split]).split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, buf[split + 4..].to_vec())
    }

    fn volume(bodies: &[MeshBody]) -> f64 {
        bodies.iter().map(|b| b.to_solid().unwrap().volume()).sum()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn files_come_in_as_parts_and_sketches_and_leave_in_every_format() {
        // Imported files live where programs read them (one folder per process).
        let dir = std::env::temp_dir().join(format!("forma-formats-{}", StableId::new()));
        if de_shape::assets::dir().is_none() {
            de_shape::assets::set_dir(dir.join("assets"));
        }
        let dir = de_shape::assets::dir().unwrap().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let server = server_in(&dir);
        let at = serve(server.clone()).await;

        // A mesh file with two objects, one coloured, becomes one shape with two parts.
        let b = Manifold::cube(30.0, 20.0, 10.0, false);
        let c = Manifold::cylinder(15.0, 4.0, 4.0, 64, false).translate(50.0, 0.0, 0.0);
        let want = b.volume() + c.volume();
        let obj = formats::mesh::write_obj(&[MeshBody::of_solid("Base plate", &b, Some(Look::color([0.8, 0.2, 0.4])), false), MeshBody::of_solid("Pin", &c, None, false)]);
        let (status, reply) = http(at, "POST", "/import/bracket_v2.obj", &obj).await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&reply));
        let reply: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!((reply["kind"].as_str(), reply["bodies"].as_u64(), reply["exact"].as_bool()), (Some("shape"), Some(2), Some(false)));
        assert_eq!(reply["colors"], json!([{ "part": "Base plate", "color": "#cc3366" }]), "the file's colours are reported");
        let id = reply["id"].as_str().unwrap().to_string();
        // Made of orange PETG: every format that carries colour carries its look.
        let app = server.project(None).unwrap();
        let petg = app.commit_here("solid.material.create", json!({ "name": "PETG orange", "density": { "value": 1270, "unit": "kg/m3" } })).unwrap()["output"]["id"].clone();
        app.commit_here("solid.material.assign", json!({ "target": id, "material": petg })).unwrap();
        let orange = formats::looks::look("PETG orange");

        // Out as glTF: a node per part, named, coloured, the same solid.
        let (status, glb) = http(at, "GET", &format!("/export/{id}.glb"), b"").await;
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&glb));
        let back = formats::read_mesh(&glb, Kind::Glb).unwrap();
        assert_eq!(back.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Base plate", "Pin"]);
        assert_eq!(back[0].look.as_ref().map(Look::hex), Some(orange.hex()), "the material's look");
        assert!((back[0].look.as_ref().unwrap().roughness - orange.roughness).abs() < 1e-9);
        assert!((volume(&back) - want).abs() < 1e-4 * want);
        // OBJ, PLY, and one part at the origin.
        let (_, obj) = http(at, "GET", &format!("/export/{id}.obj"), b"").await;
        assert!((volume(&formats::read_mesh(&obj, Kind::Obj).unwrap()) - want).abs() < 1e-6 * want);
        let (_, ply) = http(at, "GET", &format!("/export/{id}.ply"), b"").await;
        assert!((volume(&formats::read_mesh(&ply, Kind::Ply).unwrap()) - want).abs() < 1e-4 * want);
        let (status, pin) = http(at, "GET", &format!("/export/{id}.glb?part=Pin"), b"").await;
        assert_eq!(status, 200);
        let pin = formats::read_mesh(&pin, Kind::Glb).unwrap();
        assert_eq!(pin.len(), 1);
        assert!((volume(&pin) - c.volume()).abs() < 1e-4 * c.volume());

        // Out as STEP and IGES (faceted, from a mesh), named and coloured, and back in exactly.
        for (ext, kind) in [("step", Kind::Step), ("igs", Kind::Iges)] {
            let (status, file) = http(at, "GET", &format!("/export/{id}.{ext}"), b"").await;
            assert_eq!(status, 200, "{ext}: {}", String::from_utf8_lossy(&file));
            assert_eq!(Kind::sniff(&file), Some(kind));
            let path = dir.join(format!("out.{ext}"));
            std::fs::write(&path, &file).unwrap();
            let format = if kind == Kind::Iges { de_occt::CadFormat::Iges } else { de_occt::CadFormat::Step };
            let read = de_occt::Arena::new().read_cad(&path, format).unwrap();
            assert_eq!(read.iter().map(|b| b.color.map(de_shape::assets::hex)).collect::<Vec<_>>(), [Some(orange.hex()), Some(orange.hex())], "{ext}: coloured by material");
            let (status, reply) = http(at, "POST", &format!("/import/again.{ext}"), &file).await;
            assert_eq!(status, 200, "{ext}: {}", String::from_utf8_lossy(&reply));
            let reply: Value = serde_json::from_slice(&reply).unwrap();
            assert_eq!((reply["bodies"].as_u64(), reply["exact"].as_bool()), (Some(2), Some(true)), "{ext}");
            assert_eq!(reply["colors"].as_array().map(Vec::len), Some(2), "{ext}: colours read back");
            let again = reply["id"].as_str().unwrap();
            let (_, glb) = http(at, "GET", &format!("/export/{again}.glb"), b"").await;
            let back = formats::read_mesh(&glb, Kind::Glb).unwrap();
            assert_eq!(back.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Base plate", "Pin"], "{ext}");
            assert!((volume(&back) - want).abs() < 1e-3 * want, "{ext}: {} vs {want}", volume(&back));
        }

        // A drawing becomes a sketch; importing it again gives a second one.
        let dxf = "0\nSECTION\n2\nENTITIES\n0\nCIRCLE\n10\n0\n20\n0\n40\n12.5\n0\nLWPOLYLINE\n90\n4\n70\n1\n10\n-20\n20\n-20\n10\n20\n20\n-20\n10\n20\n20\n20\n10\n-20\n20\n20\n0\nENDSEC\n0\nEOF\n";
        for name in ["Plate", "Plate 2"] {
            let (status, reply) = http(at, "POST", "/import/Plate.dxf", dxf.as_bytes()).await;
            assert_eq!(status, 200, "{}", String::from_utf8_lossy(&reply));
            let reply: Value = serde_json::from_slice(&reply).unwrap();
            assert_eq!((reply["kind"].as_str(), reply["name"].as_str()), (Some("sketch"), Some(name)));
            assert_eq!((reply["counts"]["lines"].as_u64(), reply["counts"]["circles"].as_u64()), (Some(4), Some(1)));
        }

        // DWG and unknown files are refused, saying what to do instead.
        let (status, reply) = http(at, "POST", "/import/plan.dwg", b"AC1032").await;
        assert_eq!(status, 415);
        assert!(String::from_utf8_lossy(&reply).contains("DXF"));
        let (status, reply) = http(at, "POST", "/import/notes.txt", b"hello").await;
        assert_eq!(status, 400);
        assert!(String::from_utf8_lossy(&reply).contains(ACCEPTED));
        // A mesh with a hole is refused.
        let mut open = MeshBody::of_solid("Lid", &b, None, false);
        open.triangles.truncate(open.triangles.len() - 3);
        let (status, reply) = http(at, "POST", "/import/lid.obj", &formats::mesh::write_obj(&[open])).await;
        assert_eq!(status, 422);
        assert!(String::from_utf8_lossy(&reply).contains("not a closed solid"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
