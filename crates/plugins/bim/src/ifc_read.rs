//! IFC in (IFC2X3, IFC4, IFC4X3 STEP files): the spatial structure, every
//! element with its body as closed triangle meshes in millimetres, openings
//! as voids to cut from their host, spaces as outlines with heights, zones,
//! grids, materials, property sets, quantities, types and classification.
//!
//! Geometry read: IfcTriangulatedFaceSet, IfcPolygonalFaceSet, IfcFacetedBrep
//! (and shell-based surface models), IfcExtrudedAreaSolid over rectangle,
//! hollow rectangle, circle, hollow circle, I-shape and arbitrary (polyline,
//! indexed poly curve, composite) profiles with voids, IfcSweptDiskSolid
//! along polylines, IfcMappedItem, and IfcBooleanResult/ClippingResult
//! (a solid subtracted becomes a void; half-space clips are left out, with
//! a note). Placements follow IfcLocalPlacement chains.

use crate::geom::{self, Affine, IDENTITY};
use crate::step::{self, Arg, Entity, File};
use crate::{Classification, Element, Grid, GridAxis, Group, Level, Mesh, PValue, Psets, Space, Zone};
use std::collections::{BTreeMap, HashMap, HashSet};

/// An element read from a file.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Part {
    pub name: String,
    pub info: Element,
    pub material: Option<String>,
    /// Closed meshes (mm, project coordinates) making its body.
    pub solids: Vec<Mesh>,
    /// Closed meshes to cut out of it (its openings, boolean differences).
    pub voids: Vec<Mesh>,
    /// Pieces that did not close, replaced by their convex hull points.
    pub hulls: Vec<Vec<[f64; 3]>>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Imported {
    pub schema: String,
    pub project: String,
    pub site: crate::Site,
    pub building: crate::Building,
    pub levels: Vec<Level>,
    pub parts: Vec<Part>,
    pub spaces: Vec<Space>,
    pub zones: Vec<Zone>,
    pub grids: Vec<Grid>,
    pub groups: Vec<Group>,
    /// Openings that are boxes, kept as openings (their host and filler by part name).
    pub openings: Vec<crate::Opening>,
    pub notes: Vec<String>,
}

struct Reader<'a> {
    f: &'a File,
    /// Millimetres per file length unit.
    mm: f64,
    /// Square metres per file area unit, cubic metres per volume unit.
    m2: f64,
    m3: f64,
    notes: Vec<String>,
    placements: HashMap<u64, Affine>,
}

fn ty(e: Option<&Entity>) -> &str {
    e.map(|e| e.ty.as_str()).unwrap_or("")
}

impl<'a> Reader<'a> {
    fn e(&self, id: u64) -> Option<&'a Entity> {
        self.f.get(id)
    }
    fn note(&mut self, n: String) {
        if !self.notes.contains(&n) && self.notes.len() < 40 {
            self.notes.push(n);
        }
    }
    fn coords(&self, a: &Arg) -> Option<Vec<f64>> {
        let e = self.e(a.as_ref()?)?;
        Some(e.arg(0).nums())
    }
    fn point3(&self, a: &Arg) -> [f64; 3] {
        let c = self.coords(a).unwrap_or_default();
        [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)]
    }
    fn dir(&self, a: &Arg) -> Option<[f64; 3]> {
        let c = self.coords(a)?;
        let v = [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)];
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        (n > 1e-12).then(|| v.map(|x| x / n))
    }
    /// IfcAxis2Placement3D / 2D as a transform (lengths in file units).
    fn axis_placement(&self, id: Option<u64>) -> Affine {
        let Some(e) = id.and_then(|i| self.e(i)) else { return IDENTITY };
        let o = self.point3(e.arg(0));
        if e.ty == "IFCAXIS2PLACEMENT2D" {
            let x = self.dir(e.arg(1)).unwrap_or([1.0, 0.0, 0.0]);
            return [[x[0], x[1], 0.0], [-x[1], x[0], 0.0], [0.0, 0.0, 1.0], o];
        }
        let z = self.dir(e.arg(1)).unwrap_or([0.0, 0.0, 1.0]);
        let xr = self.dir(e.arg(2)).unwrap_or(if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] });
        // X is the reference direction made square to Z.
        let d = xr[0] * z[0] + xr[1] * z[1] + xr[2] * z[2];
        let mut x = [xr[0] - d * z[0], xr[1] - d * z[1], xr[2] - d * z[2]];
        let n = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        x = if n > 1e-12 { x.map(|v| v / n) } else { [1.0, 0.0, 0.0] };
        let y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
        [x, y, z, o]
    }
    /// An object placement, resolved through its chain (file units).
    fn placement(&mut self, id: Option<u64>) -> Affine {
        let Some(id) = id else { return IDENTITY };
        if let Some(t) = self.placements.get(&id) {
            return *t;
        }
        let t = match self.e(id) {
            Some(e) if e.ty == "IFCLOCALPLACEMENT" => {
                let parent = e.arg(0).as_ref();
                let rel = self.axis_placement(e.arg(1).as_ref());
                if parent == Some(id) {
                    rel
                } else {
                    let p = self.placement(parent);
                    geom::compose(&p, &rel)
                }
            }
            _ => IDENTITY,
        };
        self.placements.insert(id, t);
        t
    }
    fn cartesian_operator(&self, id: Option<u64>) -> Affine {
        let Some(e) = id.and_then(|i| self.e(i)) else { return IDENTITY };
        let x = self.dir(e.arg(0)).unwrap_or([1.0, 0.0, 0.0]);
        let y = self.dir(e.arg(1)).unwrap_or([-x[1], x[0], 0.0]);
        let o = self.point3(e.arg(2));
        let s = e.arg(3).as_num().unwrap_or(1.0);
        let z = self.dir(e.arg(4)).unwrap_or([x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]]);
        [x.map(|v| v * s), y.map(|v| v * s), z.map(|v| v * s), o]
    }

    /// A closed 2D curve's points (file units).
    fn curve(&mut self, id: Option<u64>) -> Vec<[f64; 2]> {
        let Some(e) = id.and_then(|i| self.e(i)) else { return vec![] };
        match e.ty.as_str() {
            "IFCPOLYLINE" => e.arg(0).refs().iter().map(|p| {
                let c = self.e(*p).map(|x| x.arg(0).nums()).unwrap_or_default();
                [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0)]
            }).collect(),
            "IFCINDEXEDPOLYCURVE" => {
                let pts: Vec<[f64; 2]> = self.e(e.arg(0).as_ref().unwrap_or(0)).map(|l| l.arg(0).as_list().iter().map(|p| {
                    let c = p.nums();
                    [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0)]
                }).collect()).unwrap_or_default();
                let segs = e.arg(1).as_list();
                if segs.is_empty() {
                    return pts;
                }
                let mut out = vec![];
                for s in segs {
                    if let Arg::Typed(t, inner) = s {
                        let idx: Vec<usize> = inner.nums().iter().map(|i| (*i as usize).saturating_sub(1)).collect();
                        if t == "IFCARCINDEX" && idx.len() == 3 {
                            out.extend(arc3(pts[idx[0]], pts[idx[1]], pts[idx[2]], 12));
                        } else {
                            out.extend(idx.iter().filter_map(|i| pts.get(*i).copied()));
                        }
                    }
                }
                out
            }
            "IFCCOMPOSITECURVE" => {
                let mut out = vec![];
                for s in e.arg(0).refs() {
                    let parent = self.e(s).and_then(|x| x.arg(2).as_ref());
                    out.extend(self.curve(parent));
                }
                out
            }
            "IFCCIRCLE" => {
                let t = self.axis_placement(e.arg(0).as_ref());
                let r = e.arg(1).as_num().unwrap_or(0.0);
                circle(r, 24).into_iter().map(|p| {
                    let q = geom::apply(&t, [p[0], p[1], 0.0]);
                    [q[0], q[1]]
                }).collect()
            }
            other => {
                self.note(format!("profile curve {other} is not read; left out"));
                vec![]
            }
        }
    }

    /// A profile as an outer loop and holes (file units).
    fn profile(&mut self, id: Option<u64>) -> Option<(Vec<[f64; 2]>, Vec<Vec<[f64; 2]>>)> {
        let e = self.e(id?)?;
        let place = |r: &Reader, at: usize| r.axis_placement(e.arg(at).as_ref());
        let moved = |t: &Affine, l: Vec<[f64; 2]>| l.into_iter().map(|p| { let q = geom::apply(t, [p[0], p[1], 0.0]); [q[0], q[1]] }).collect::<Vec<_>>();
        let n = |i: usize| e.arg(i).as_num().unwrap_or(0.0);
        let rect = |w: f64, h: f64| vec![[-w / 2.0, -h / 2.0], [w / 2.0, -h / 2.0], [w / 2.0, h / 2.0], [-w / 2.0, h / 2.0]];
        Some(match e.ty.as_str() {
            "IFCRECTANGLEPROFILEDEF" => (moved(&place(self, 2), rect(n(3), n(4))), vec![]),
            "IFCRECTANGLEHOLLOWPROFILEDEF" => {
                let t = place(self, 2);
                let w = n(5);
                (moved(&t, rect(n(3), n(4))), vec![moved(&t, rect(n(3) - 2.0 * w, n(4) - 2.0 * w))])
            }
            "IFCCIRCLEPROFILEDEF" => (moved(&place(self, 2), circle(n(3), 32)), vec![]),
            "IFCCIRCLEHOLLOWPROFILEDEF" => {
                let t = place(self, 2);
                (moved(&t, circle(n(3), 32)), vec![moved(&t, circle(n(3) - n(4), 32))])
            }
            "IFCISHAPEPROFILEDEF" => {
                let (b, h, tw, tf) = (n(3), n(4), n(5), n(6));
                let pts = vec![[-b / 2.0, -h / 2.0], [b / 2.0, -h / 2.0], [b / 2.0, -h / 2.0 + tf], [tw / 2.0, -h / 2.0 + tf], [tw / 2.0, h / 2.0 - tf], [b / 2.0, h / 2.0 - tf], [b / 2.0, h / 2.0], [-b / 2.0, h / 2.0], [-b / 2.0, h / 2.0 - tf], [-tw / 2.0, h / 2.0 - tf], [-tw / 2.0, -h / 2.0 + tf], [-b / 2.0, -h / 2.0 + tf]];
                (moved(&place(self, 2), pts), vec![])
            }
            "IFCARBITRARYCLOSEDPROFILEDEF" => (self.curve(e.arg(2).as_ref()), vec![]),
            "IFCARBITRARYPROFILEDEFWITHVOIDS" => {
                let outer = self.curve(e.arg(2).as_ref());
                let holes = e.arg(3).refs().into_iter().map(|h| self.curve(Some(h))).collect();
                (outer, holes)
            }
            other => {
                self.note(format!("profile {other} is not read; its solids are left out"));
                return None;
            }
        })
    }

    /// The meshes of one representation item (file units, item's frame),
    /// and the meshes it subtracts.
    fn item(&mut self, id: u64, depth: usize, solids: &mut Vec<Mesh>, voids: &mut Vec<Mesh>, open: &mut Vec<Mesh>) {
        let Some(e) = self.e(id) else { return };
        if depth > 16 {
            return;
        }
        match e.ty.as_str() {
            "IFCTRIANGULATEDFACESET" | "IFCTRIANGULATEDIRREGULARNETWORK" => {
                let pts = self.point_list(e.arg(0));
                let pn: Vec<usize> = e.arg(4).nums().iter().map(|i| *i as usize).collect();
                let at = |i: f64| -> u32 {
                    let i = i as usize;
                    let j = if pn.is_empty() { i } else { pn.get(i.saturating_sub(1)).copied().unwrap_or(1) };
                    j.saturating_sub(1) as u32
                };
                let tris = e.arg(3).as_list().iter().map(|t| { let v = t.nums(); [at(v[0]), at(v[1]), at(v[2])] }).collect();
                let m = Mesh { positions: pts, triangles: tris };
                push_closed(m, solids, open);
            }
            "IFCPOLYGONALFACESET" => {
                let pts = self.point_list(e.arg(0));
                let pn: Vec<usize> = e.arg(3).nums().iter().map(|i| *i as usize).collect();
                let map = |i: f64| -> usize {
                    let i = i as usize;
                    (if pn.is_empty() { i } else { pn.get(i.saturating_sub(1)).copied().unwrap_or(1) }).saturating_sub(1)
                };
                let mut m = Mesh { positions: pts, triangles: vec![] };
                for f in e.arg(2).refs() {
                    let Some(face) = self.e(f) else { continue };
                    let outer: Vec<usize> = face.arg(0).nums().into_iter().map(map).collect();
                    let holes: Vec<Vec<usize>> = if face.ty == "IFCINDEXEDPOLYGONALFACEWITHVOIDS" { face.arg(1).as_list().iter().map(|h| h.nums().into_iter().map(map).collect()).collect() } else { vec![] };
                    face_triangles(&mut m, &outer, &holes);
                }
                push_closed(m, solids, open);
            }
            "IFCFACETEDBREP" | "IFCFACETEDBREPWITHVOIDS" | "IFCADVANCEDBREP" => {
                if e.ty == "IFCADVANCEDBREP" {
                    self.note("curved B-rep faces (IfcAdvancedBrep) are read by their edge loops only".into());
                }
                let m = self.shell(e.arg(0).as_ref());
                push_closed(m, solids, open);
                for v in e.arg(1).refs() {
                    let m = self.shell(Some(v));
                    if let Some(s) = geom::solid(&m, 1e-6) {
                        voids.push(s);
                    }
                }
            }
            "IFCSHELLBASEDSURFACEMODEL" | "IFCFACEBASEDSURFACEMODEL" => {
                for s in e.arg(0).refs() {
                    let m = self.shell(Some(s));
                    push_closed(m, solids, open);
                }
            }
            "IFCCLOSEDSHELL" | "IFCOPENSHELL" => {
                let m = self.shell(Some(id));
                push_closed(m, solids, open);
            }
            "IFCEXTRUDEDAREASOLID" | "IFCEXTRUDEDAREASOLIDTAPERED" => {
                let Some((outer, holes)) = self.profile(e.arg(0).as_ref()) else { return };
                let pos = self.axis_placement(e.arg(1).as_ref());
                let d = self.dir(e.arg(2)).unwrap_or([0.0, 0.0, 1.0]);
                let depth = e.arg(3).as_num().unwrap_or(0.0);
                if outer.len() < 3 || depth <= 0.0 {
                    return;
                }
                let mut m = geom::extrude(&outer, &holes, d.map(|v| v * depth));
                geom::transform_mesh(&pos, &mut m);
                push_closed(m, solids, open);
            }
            "IFCSWEPTDISKSOLID" => {
                let r = e.arg(1).as_num().unwrap_or(0.0);
                let pts = self.curve3(e.arg(0).as_ref());
                for w in pts.windows(2) {
                    if let Some(m) = tube(w[0], w[1], r, 8) {
                        solids.push(m);
                    }
                }
            }
            "IFCBOOLEANRESULT" | "IFCBOOLEANCLIPPINGRESULT" => {
                let op = e.arg(0).as_enum().unwrap_or("DIFFERENCE").to_string();
                if let Some(a) = e.arg(1).as_ref() {
                    self.item(a, depth + 1, solids, voids, open);
                }
                let Some(b) = e.arg(2).as_ref() else { return };
                let bt = ty(self.e(b)).to_string();
                if bt.contains("HALFSPACE") {
                    self.note("half-space clips (IfcBooleanClippingResult) are left uncut".into());
                    return;
                }
                match op.as_str() {
                    "UNION" => self.item(b, depth + 1, solids, voids, open),
                    "DIFFERENCE" => {
                        let mut cut = vec![];
                        let mut cut_open = vec![];
                        self.item(b, depth + 1, &mut cut, voids, &mut cut_open);
                        voids.extend(cut);
                    }
                    _ => self.note("boolean intersections are read as their first operand".into()),
                }
            }
            "IFCMAPPEDITEM" => {
                let Some(map) = e.arg(0).as_ref().and_then(|m| self.e(m)) else { return };
                let origin = self.axis_placement(map.arg(0).as_ref());
                let target = self.cartesian_operator(e.arg(1).as_ref());
                let t = geom::compose(&target, &geom::invert(&origin));
                let Some(rep) = map.arg(1).as_ref().and_then(|r| self.e(r)) else { return };
                let (mut s2, mut v2, mut o2) = (vec![], vec![], vec![]);
                for it in rep.arg(3).refs() {
                    self.item(it, depth + 1, &mut s2, &mut v2, &mut o2);
                }
                for mut m in s2.into_iter().chain(o2.into_iter()) {
                    geom::transform_mesh(&t, &mut m);
                    if geom::is_closed(&m) { solids.push(m) } else { open.push(m) }
                }
                for mut m in v2 {
                    geom::transform_mesh(&t, &mut m);
                    voids.push(m);
                }
            }
            "IFCBLOCK" => {
                let pos = self.axis_placement(e.arg(0).as_ref());
                let (x, y, z) = (e.arg(1).as_num().unwrap_or(0.0), e.arg(2).as_num().unwrap_or(0.0), e.arg(3).as_num().unwrap_or(0.0));
                let mut m = geom::extrude(&[[0.0, 0.0], [x, 0.0], [x, y], [0.0, y]], &[], [0.0, 0.0, z]);
                geom::transform_mesh(&pos, &mut m);
                push_closed(m, solids, open);
            }
            "IFCCSGSOLID" => {
                if let Some(t) = e.arg(0).as_ref() {
                    self.item(t, depth + 1, solids, voids, open);
                }
            }
            "IFCSTYLEDITEM" | "IFCBOUNDINGBOX" | "IFCPOLYLINE" | "IFCGEOMETRICCURVESET" | "IFCGEOMETRICSET" | "IFCTEXTLITERALWITHEXTENT" | "IFCANNOTATIONFILLAREA" => {}
            other => self.note(format!("{other} geometry is not read; left out")),
        }
    }

    fn point_list(&self, a: &Arg) -> Vec<[f64; 3]> {
        self.e(a.as_ref().unwrap_or(0))
            .map(|l| l.arg(0).as_list().iter().map(|p| { let c = p.nums(); [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)] }).collect())
            .unwrap_or_default()
    }

    fn curve3(&self, id: Option<u64>) -> Vec<[f64; 3]> {
        let Some(e) = id.and_then(|i| self.e(i)) else { return vec![] };
        match e.ty.as_str() {
            "IFCPOLYLINE" => e.arg(0).refs().iter().map(|p| self.point3(&Arg::Ref(*p))).collect(),
            "IFCINDEXEDPOLYCURVE" => self.point_list(e.arg(0)),
            "IFCCOMPOSITECURVE" => e.arg(0).refs().iter().flat_map(|s| self.curve3(self.e(*s).and_then(|x| x.arg(2).as_ref()))).collect(),
            _ => vec![],
        }
    }

    /// The faces of a shell as one triangle mesh.
    fn shell(&mut self, id: Option<u64>) -> Mesh {
        let mut m = Mesh::default();
        let Some(e) = id.and_then(|i| self.e(i)) else { return m };
        for f in e.arg(0).refs() {
            let Some(face) = self.e(f) else { continue };
            let mut outer: Vec<usize> = vec![];
            let mut holes: Vec<Vec<usize>> = vec![];
            for b in face.arg(0).refs() {
                let Some(bound) = self.e(b) else { continue };
                let Some(lp) = bound.arg(0).as_ref().and_then(|l| self.e(l)) else { continue };
                if lp.ty != "IFCPOLYLOOP" {
                    continue;
                }
                let mut idx: Vec<usize> = lp.arg(0).refs().iter().map(|p| {
                    m.positions.push(self.point3(&Arg::Ref(*p)));
                    m.positions.len() - 1
                }).collect();
                if bound.arg(1).as_enum() == Some("F") {
                    idx.reverse();
                }
                if bound.ty == "IFCFACEOUTERBOUND" || outer.is_empty() && holes.is_empty() && face.arg(0).refs().len() == 1 {
                    outer = idx;
                } else {
                    holes.push(idx);
                }
            }
            if outer.is_empty() && !holes.is_empty() {
                // No bound marked outer: the largest is.
                holes.sort_by_key(|h| std::cmp::Reverse(h.len()));
                outer = holes.remove(0);
            }
            face_triangles(&mut m, &outer, &holes);
        }
        geom::weld(&m, 1e-9)
    }

    fn scale_mesh(&self, m: &mut Mesh, t: &Affine) {
        let s = self.mm;
        let st: Affine = [[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, s], [0.0, 0.0, 0.0]];
        geom::transform_mesh(&geom::compose(&st, t), m);
    }

    /// An object's body (mm, project frame): solids, voids and unclosed pieces.
    fn body(&mut self, e: &Entity) -> (Vec<Mesh>, Vec<Mesh>, Vec<Mesh>) {
        let place = self.placement(e.arg(5).as_ref());
        let (mut solids, mut voids, mut open) = (vec![], vec![], vec![]);
        let Some(shape) = e.arg(6).as_ref().and_then(|s| self.e(s)) else { return (solids, voids, open) };
        let reps = shape.arg(2).refs();
        // The body; else any representation with solids.
        let pick = reps.iter().copied().find(|r| self.e(*r).and_then(|x| x.arg(1).as_str()).is_some_and(|i| i.eq_ignore_ascii_case("Body"))).or_else(|| {
            reps.iter().copied().find(|r| self.e(*r).and_then(|x| x.arg(2).as_str()).is_some_and(|t| !matches!(t, "Curve2D" | "GeometricCurveSet" | "BoundingBox" | "Annotation2D" | "Curve3D")))
        });
        if let Some(rep) = pick.and_then(|r| self.e(r)) {
            for it in rep.arg(3).refs() {
                self.item(it, 0, &mut solids, &mut voids, &mut open);
            }
        }
        for m in solids.iter_mut().chain(voids.iter_mut()).chain(open.iter_mut()) {
            self.scale_mesh(m, &place);
        }
        (solids, voids, open)
    }
}

/// A mesh that is a box standing upright (eight corners, a level bottom):
/// as an opening, its longer side along.
fn as_box(m: &Mesh) -> Option<crate::Opening> {
    if m.positions.len() != 8 {
        return None;
    }
    let (lo, hi) = geom::bounds(m);
    let mut bottom: Vec<[f64; 3]> = m.positions.iter().filter(|p| (p[2] - lo[2]).abs() < 1e-6).copied().collect();
    if bottom.len() != 4 || m.positions.iter().any(|p| (p[2] - lo[2]).abs() > 1e-6 && (p[2] - hi[2]).abs() > 1e-6) {
        return None;
    }
    let o = bottom.remove(0);
    let d = |p: &[f64; 3]| ((p[0] - o[0]).powi(2) + (p[1] - o[1]).powi(2)).sqrt();
    bottom.sort_by(|a, b| d(a).total_cmp(&d(b)));
    // The two nearest corners are o's sides; the farthest its diagonal.
    let (s1, s2) = (bottom[0], bottom[1]);
    let v1 = [s1[0] - o[0], s1[1] - o[1]];
    let v2 = [s2[0] - o[0], s2[1] - o[1]];
    if (v1[0] * v2[0] + v1[1] * v2[1]).abs() > 1e-3 * d(&s1) * d(&s2) {
        return None;
    }
    let (along, through) = if d(&s2) >= d(&s1) { (v2, v1) } else { (v1, v2) };
    let la = (along[0] * along[0] + along[1] * along[1]).sqrt();
    let lt = (through[0] * through[0] + through[1] * through[1]).sqrt();
    let dir = [along[0] / la, along[1] / la];
    // Through runs to the left of along; else start from the far side.
    let left = -dir[1] * through[0] + dir[0] * through[1] > 0.0;
    let origin = if left { o } else { [o[0] + through[0], o[1] + through[1], o[2]] };
    Some(crate::Opening { name: String::new(), host: String::new(), origin, dir, size: [la, lt, hi[2] - lo[2]], fills: None })
}

fn arc3(a: [f64; 2], b: [f64; 2], c: [f64; 2], n: usize) -> Vec<[f64; 2]> {
    // Circle through three points.
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() < 1e-12 {
        return vec![a, b, c];
    }
    let sq = |p: [f64; 2]| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d;
    let uy = (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d;
    let r = ((a[0] - ux).powi(2) + (a[1] - uy).powi(2)).sqrt();
    let ang = |p: [f64; 2]| (p[1] - uy).atan2(p[0] - ux);
    let (t0, t1, t2) = (ang(a), ang(b), ang(c));
    let tau = std::f64::consts::TAU;
    let norm = |t: f64| ((t % tau) + tau) % tau;
    // Anticlockwise from a reaches b before c, else clockwise.
    let ccw = norm(t1 - t0) < norm(t2 - t0);
    let sweep = if ccw { norm(t2 - t0) } else { -norm(t0 - t2) };
    (0..=n).map(|i| {
        let t = t0 + sweep * i as f64 / n as f64;
        [ux + r * t.cos(), uy + r * t.sin()]
    }).collect()
}

fn circle(r: f64, n: usize) -> Vec<[f64; 2]> {
    (0..n).map(|i| {
        let t = std::f64::consts::TAU * i as f64 / n as f64;
        [r * t.cos(), r * t.sin()]
    }).collect()
}

/// A tube (n-sided) from a to b.
fn tube(a: [f64; 3], b: [f64; 3], r: f64, n: usize) -> Option<Mesh> {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len < 1e-9 || r <= 0.0 {
        return None;
    }
    let z = d.map(|v| v / len);
    let h = if z[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let mut x = [z[1] * h[2] - z[2] * h[1], z[2] * h[0] - z[0] * h[2], z[0] * h[1] - z[1] * h[0]];
    let nx = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
    x = x.map(|v| v / nx);
    let y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
    let mut m = geom::extrude(&circle(r, n), &[], [0.0, 0.0, len]);
    geom::transform_mesh(&[x, y, z, a], &mut m);
    Some(m)
}

/// Triangulates a planar face (outer loop, holes; indices into m.positions).
fn face_triangles(m: &mut Mesh, outer: &[usize], holes: &[Vec<usize>]) {
    if outer.len() < 3 {
        return;
    }
    // Newell normal, then project onto the plane square to it.
    let mut n = [0.0; 3];
    for i in 0..outer.len() {
        let (a, b) = (m.positions[outer[i]], m.positions[outer[(i + 1) % outer.len()]]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-15 {
        return;
    }
    let n = n.map(|v| v / len);
    let h = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let mut u = [n[1] * h[2] - n[2] * h[1], n[2] * h[0] - n[0] * h[2], n[0] * h[1] - n[1] * h[0]];
    let ul = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    u = u.map(|v| v / ul);
    let v = [n[1] * u[2] - n[2] * u[1], n[2] * u[0] - n[0] * u[2], n[0] * u[1] - n[1] * u[0]];
    let p2 = |p: [f64; 3]| [p[0] * u[0] + p[1] * u[1] + p[2] * u[2], p[0] * v[0] + p[1] * v[1] + p[2] * v[2]];
    let o2: Vec<[f64; 2]> = outer.iter().map(|&i| p2(m.positions[i])).collect();
    let h2: Vec<Vec<[f64; 2]>> = holes.iter().map(|h| h.iter().map(|&i| p2(m.positions[i])).collect()).collect();
    let all: Vec<usize> = outer.iter().copied().chain(holes.iter().flatten().copied()).collect();
    // The triangulation runs anticlockwise in (u, v), which faces +n: the loop's own way.
    for t in geom::triangulate(&o2, &h2) {
        m.triangles.push([all[t[0]] as u32, all[t[1]] as u32, all[t[2]] as u32]);
    }
}

fn push_closed(m: Mesh, solids: &mut Vec<Mesh>, open: &mut Vec<Mesh>) {
    if m.triangles.is_empty() {
        return;
    }
    match geom::solid(&m, 1e-6) {
        Some(s) => solids.push(s),
        None => open.push(geom::weld(&m, 1e-6)),
    }
}

fn pvalue(a: &Arg) -> Option<PValue> {
    match a {
        Arg::Typed(t, inner) => match (&**inner, t.as_str()) {
            (Arg::Enum(e), _) if e == "T" || e == "TRUE" => Some(PValue::Bool(true)),
            (Arg::Enum(e), _) if e == "F" || e == "FALSE" => Some(PValue::Bool(false)),
            (Arg::Enum(e), _) => Some(PValue::Text(e.clone())),
            (Arg::Num(n), _) => Some(PValue::Num(*n)),
            (Arg::Str(s), _) => Some(PValue::Text(s.clone())),
            _ => None,
        },
        Arg::Str(s) => Some(PValue::Text(s.clone())),
        Arg::Num(n) => Some(PValue::Num(*n)),
        _ => None,
    }
}

const SPATIAL: &[&str] = &["IFCPROJECT", "IFCSITE", "IFCBUILDING", "IFCBUILDINGSTOREY", "IFCSPACE", "IFCFACILITY", "IFCFACILITYPART"];
const NOT_ELEMENTS: &[&str] = &["IFCOPENINGELEMENT", "IFCOPENINGSTANDARDCASE", "IFCVIRTUALELEMENT", "IFCANNOTATION", "IFCGRID", "IFCSPACE"];

/// The class name as written in IFC's documentation ("IFCWALL" → "IfcWall").
fn class_case(upper: &str) -> String {
    const KNOWN: &[&str] = &[
        "IfcWall", "IfcWallStandardCase", "IfcWallElementedCase", "IfcSlab", "IfcSlabStandardCase", "IfcRoof", "IfcBeam", "IfcBeamStandardCase", "IfcColumn", "IfcColumnStandardCase", "IfcMember", "IfcMemberStandardCase", "IfcPlate",
        "IfcDoor", "IfcDoorStandardCase", "IfcWindow", "IfcWindowStandardCase", "IfcStair", "IfcStairFlight", "IfcRamp", "IfcRampFlight", "IfcRailing", "IfcCurtainWall", "IfcCovering", "IfcFooting", "IfcPile",
        "IfcChimney", "IfcShadingDevice", "IfcBuildingElementProxy", "IfcFurniture", "IfcFurnishingElement", "IfcSystemFurnitureElement", "IfcSanitaryTerminal", "IfcReinforcingBar", "IfcReinforcingMesh",
        "IfcElementAssembly", "IfcFlowTerminal", "IfcFlowSegment", "IfcFlowFitting", "IfcDistributionElement", "IfcLightFixture", "IfcTransportElement", "IfcDiscreteAccessory", "IfcMechanicalFastener",
        "IfcBuildingElementPart", "IfcCivilElement", "IfcGeographicElement", "IfcPipeSegment", "IfcDuctSegment", "IfcAirTerminal", "IfcSpaceHeater", "IfcElectricAppliance", "IfcOutlet", "IfcSwitchingDevice",
    ];
    KNOWN.iter().find(|k| k.eq_ignore_ascii_case(upper)).map(|s| s.to_string()).unwrap_or_else(|| {
        let rest = upper.strip_prefix("IFC").unwrap_or(upper).to_ascii_lowercase();
        let mut c = rest.chars();
        format!("Ifc{}{}", c.next().map(|x| x.to_ascii_uppercase()).unwrap_or_default(), c.as_str())
    })
}

/// Reads an IFC file's text.
pub fn read(text: &str) -> Result<Imported, String> {
    let f = step::parse(text)?;
    if !f.schema.is_empty() && !f.schema.starts_with("IFC") {
        return Err(format!("this STEP file is {}, not IFC", f.schema));
    }
    let mut rd = Reader { f: &f, mm: 1.0, m2: 1.0, m3: 1.0, notes: vec![], placements: HashMap::new() };
    units(&mut rd);
    let mut out = Imported { schema: f.schema.clone(), ..Default::default() };

    // Relations.
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new(); // aggregates
    let mut parent_of: HashMap<u64, u64> = HashMap::new();
    let mut container: HashMap<u64, u64> = HashMap::new();
    let mut voids_of: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut fills: HashMap<u64, u64> = HashMap::new(); // filler → opening
    let mut host_of_opening: HashMap<u64, u64> = HashMap::new();
    let mut props: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut types: HashMap<u64, u64> = HashMap::new();
    let mut material: HashMap<u64, String> = HashMap::new();
    let mut classes: HashMap<u64, Vec<Classification>> = HashMap::new();
    let mut zone_members: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut ids: Vec<u64> = f.entities.keys().copied().collect();
    ids.sort();
    for &id in &ids {
        let e = &f.entities[&id];
        match e.ty.as_str() {
            "IFCRELAGGREGATES" | "IFCRELNESTS" => {
                let Some(p) = e.arg(4).as_ref() else { continue };
                for c in e.arg(5).refs() {
                    children.entry(p).or_default().push(c);
                    parent_of.insert(c, p);
                }
            }
            "IFCRELCONTAINEDINSPATIALSTRUCTURE" => {
                let Some(s) = e.arg(5).as_ref() else { continue };
                for c in e.arg(4).refs() {
                    container.insert(c, s);
                }
            }
            "IFCRELVOIDSELEMENT" => {
                if let (Some(h), Some(o)) = (e.arg(4).as_ref(), e.arg(5).as_ref()) {
                    voids_of.entry(h).or_default().push(o);
                    host_of_opening.insert(o, h);
                }
            }
            "IFCRELFILLSELEMENT" => {
                if let (Some(o), Some(x)) = (e.arg(4).as_ref(), e.arg(5).as_ref()) {
                    fills.insert(x, o);
                }
            }
            "IFCRELDEFINESBYPROPERTIES" => {
                let Some(p) = e.arg(5).as_ref() else { continue };
                for c in e.arg(4).refs() {
                    props.entry(c).or_default().push(p);
                }
            }
            "IFCRELDEFINESBYTYPE" => {
                let Some(t) = e.arg(5).as_ref() else { continue };
                for c in e.arg(4).refs() {
                    types.insert(c, t);
                }
            }
            "IFCRELASSOCIATESMATERIAL" => {
                let Some(name) = e.arg(5).as_ref().and_then(|m| material_name(&f, m, 0)) else { continue };
                for c in e.arg(4).refs() {
                    material.insert(c, name.clone());
                }
            }
            "IFCRELASSOCIATESCLASSIFICATION" => {
                let Some(c) = e.arg(5).as_ref().and_then(|r| classification(&f, r)) else { continue };
                for o in e.arg(4).refs() {
                    classes.entry(o).or_default().push(c.clone());
                }
            }
            "IFCRELASSIGNSTOGROUP" => {
                let Some(g) = e.arg(6).as_ref() else { continue };
                zone_members.entry(g).or_default().extend(e.arg(4).refs());
            }
            _ => {}
        }
    }
    let (psets_of, quantities_of) = {
        let (mm, m2, m3) = (rd.mm, rd.m2, rd.m3);
        let f = &f;
        let props = &props;
        let types = &types;
        let pick = move |id: u64| -> (Psets, BTreeMap<String, f64>) {
            let mut ps: Psets = BTreeMap::new();
            let mut qs: BTreeMap<String, f64> = BTreeMap::new();
            let mut sets: Vec<u64> = vec![];
            // Type properties first; the occurrence's own win.
            if let Some(t) = types.get(&id).and_then(|t| f.get(*t)) {
                sets.extend(t.arg(5).refs());
            }
            sets.extend(props.get(&id).cloned().unwrap_or_default());
            for s in sets {
                let Some(set) = f.get(s) else { continue };
                let name = set.arg(2).as_str().unwrap_or("Properties").to_string();
                match set.ty.as_str() {
                    "IFCPROPERTYSET" => {
                        let map = ps.entry(name).or_default();
                        for p in set.arg(4).refs() {
                            let Some(pe) = f.get(p) else { continue };
                            if pe.ty != "IFCPROPERTYSINGLEVALUE" && pe.ty != "IFCPROPERTYENUMERATEDVALUE" {
                                continue;
                            }
                            let Some(k) = pe.arg(0).as_str() else { continue };
                            let v = if pe.ty == "IFCPROPERTYENUMERATEDVALUE" { pe.arg(2).as_list().first().and_then(pvalue) } else { pvalue(pe.arg(2)) };
                            if let Some(v) = v {
                                map.insert(k.to_string(), v);
                            }
                        }
                    }
                    "IFCELEMENTQUANTITY" => {
                        for q in set.arg(5).refs() {
                            let Some(qe) = f.get(q) else { continue };
                            let Some(k) = qe.arg(0).as_str() else { continue };
                            let Some(v) = qe.arg(3).as_num() else { continue };
                            let v = match qe.ty.as_str() {
                                "IFCQUANTITYLENGTH" => v * mm,
                                "IFCQUANTITYAREA" => v * m2,
                                "IFCQUANTITYVOLUME" => v * m3,
                                _ => v,
                            };
                            qs.insert(k.to_string(), v);
                        }
                    }
                    _ => {}
                }
            }
            ps.retain(|_, v| !v.is_empty());
            (ps, qs)
        };
        let p1 = pick.clone();
        (move |id: u64| p1(id).0, move |id: u64| pick(id).1)
    };

    // Project, site, building.
    if let Some((_, p)) = f.all("IFCPROJECT").first() {
        out.project = p.arg(2).as_str().unwrap_or("Imported").to_string();
    }
    let latlon = |a: &Arg| {
        let v = a.nums();
        (!v.is_empty()).then(|| v.iter().enumerate().map(|(i, x)| x / [1.0, 60.0, 3600.0, 3.6e9][i.min(3)]).sum::<f64>())
    };
    if let Some((id, s)) = f.all("IFCSITE").first() {
        out.site = crate::Site {
            name: s.arg(2).as_str().unwrap_or("Site").to_string(),
            latitude: latlon(s.arg(9)),
            longitude: latlon(s.arg(10)),
            elevation: s.arg(11).as_num().map(|e| e * rd.mm / 1000.0),
            address: s.arg(13).as_ref().and_then(|a| f.get(a)).map(|a| a.arg(4).as_list().iter().filter_map(Arg::as_str).collect::<Vec<_>>().join(", ")).filter(|a| !a.is_empty()),
            north: None,
            psets: psets_of(*id),
        };
    }
    if let Some(ctx) = f.all("IFCGEOMETRICREPRESENTATIONCONTEXT").first() {
        if let Some(n) = ctx.1.arg(5).as_ref().and_then(|d| f.get(d)) {
            let v = n.arg(0).nums();
            if v.len() >= 2 && (v[0].abs() > 1e-9 || (v[1] - 1.0).abs() > 1e-9) {
                out.site.north = Some((-v[0]).atan2(v[1]).to_degrees());
            }
        }
    }
    if let Some((id, b)) = f.all("IFCBUILDING").first() {
        out.building = crate::Building {
            name: b.arg(2).as_str().unwrap_or("Building").to_string(),
            description: b.arg(3).as_str().map(str::to_string),
            address: b.arg(11).as_ref().and_then(|a| f.get(a)).map(|a| a.arg(4).as_list().iter().filter_map(Arg::as_str).collect::<Vec<_>>().join(", ")).filter(|a| !a.is_empty()),
            psets: psets_of(*id),
        };
    }

    // Storeys.
    let mut storey_name: HashMap<u64, String> = HashMap::new();
    let mut used_level = HashSet::new();
    for (id, s) in f.all("IFCBUILDINGSTOREY") {
        let place = rd.placement(s.arg(5).as_ref());
        let z = s.arg(9).as_num().map(|e| e * rd.mm).unwrap_or(place[3][2] * rd.mm);
        let mut name = s.arg(2).as_str().unwrap_or("Level").to_string();
        let mut n = 2;
        while !used_level.insert(name.clone()) {
            name = format!("{} {n}", s.arg(2).as_str().unwrap_or("Level"));
            n += 1;
        }
        let height = quantities_of(id).get("GrossHeight").copied();
        out.levels.push(Level { name: name.clone(), elevation: z, height });
        storey_name.insert(id, name);
    }
    out.levels.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
    // The storey something is in: contained, or through its parent.
    let storey_of = |mut id: u64| -> Option<String> {
        for _ in 0..12 {
            if let Some(n) = storey_name.get(&id) {
                return Some(n.clone());
            }
            id = *container.get(&id).or_else(|| parent_of.get(&id))?;
        }
        None
    };

    // Elements.
    let mut used = HashSet::new();
    let mut part_names: HashMap<u64, String> = HashMap::new();
    let mut unique = |base: String| {
        let mut n = base.clone();
        let mut k = 2;
        while !used.insert(n.clone()) {
            n = format!("{base} {k}");
            k += 1;
        }
        n
    };
    let mut group_ids: Vec<u64> = vec![];
    let mut boxes: Vec<(u64, crate::Opening)> = vec![];
    for &id in &ids {
        let e = &f.entities[&id];
        let t = e.ty.as_str();
        if SPATIAL.contains(&t) || NOT_ELEMENTS.contains(&t) || !is_product(e) {
            continue;
        }
        let (solids, voids, open) = rd.body(e);
        let class = class_case(t);
        let has_kids = children.get(&id).is_some_and(|k| !k.is_empty());
        if solids.is_empty() && open.is_empty() {
            if has_kids {
                group_ids.push(id);
            }
            continue;
        }
        let base = e.arg(2).as_str().filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(|| {
            format!("{} {}", crate::class_words(&class), e.arg(7).as_str().unwrap_or(&e.arg(0).as_str().unwrap_or("")[..6.min(e.arg(0).as_str().unwrap_or("").len())]))
        });
        let name = unique(base.trim().to_string());
        part_names.insert(id, name.clone());
        let mut info = Element {
            class: class.clone(),
            predefined: predefined_of(e, &class).filter(|p| p != "NOTDEFINED"),
            type_name: types.get(&id).and_then(|t| f.get(*t)).and_then(|t| t.arg(2).as_str()).map(str::to_string).or_else(|| e.arg(4).as_str().map(str::to_string)),
            level: storey_of(id),
            tag: e.arg(7).as_str().filter(|s| !s.is_empty()).map(str::to_string),
            description: e.arg(3).as_str().map(str::to_string),
            psets: psets_of(id),
            quantities: quantities_of(id),
            classification: classes.get(&id).cloned().unwrap_or_default(),
            ..Default::default()
        };
        attributes(e, &class, rd.mm, &mut info);
        if let Some(p) = parent_of.get(&id).filter(|p| f.get(**p).is_some_and(|x| !SPATIAL.contains(&x.ty.as_str()))) {
            info.parent = f.get(*p).and_then(|x| x.arg(2).as_str()).map(str::to_string);
            if !group_ids.contains(p) {
                group_ids.push(*p);
            }
        }
        let mut voids = voids;
        for o in voids_of.get(&id).cloned().unwrap_or_default() {
            if let Some(oe) = f.get(o) {
                let (s, v, op) = rd.body(oe);
                if s.len() == 1 && v.is_empty() {
                    if let Some(mut b) = as_box(&s[0]) {
                        b.name = oe.arg(2).as_str().map(str::to_string).unwrap_or_else(|| format!("{name} opening"));
                        b.host = name.clone();
                        boxes.push((o, b));
                    }
                }
                voids.extend(s);
                voids.extend(v);
                voids.extend(op.into_iter().filter_map(|m| geom::solid(&m, 1e-6)));
            }
        }
        let hulls: Vec<Vec<[f64; 3]>> = open.iter().filter(|m| m.positions.len() >= 4).map(|m| m.positions.clone()).collect();
        if !open.is_empty() {
            rd.note(format!("{name}: {} piece(s) of its body do not close; shown as their convex hull", open.len()));
        }
        out.parts.push(Part { name, info, material: material.get(&id).cloned(), solids, voids, hulls });
    }
    // Openings: what fills each.
    let filler_of: HashMap<u64, u64> = fills.iter().map(|(x, o)| (*o, *x)).collect();
    for (oid, mut b) in boxes {
        b.fills = filler_of.get(&oid).and_then(|x| part_names.get(x)).cloned();
        out.openings.push(b);
    }
    // Hosts of doors and windows: the element whose opening they fill.
    let fills_host: Vec<(String, String)> = fills
        .iter()
        .filter_map(|(x, o)| Some((part_names.get(x)?.clone(), part_names.get(host_of_opening.get(o)?)?.clone())))
        .collect();
    for (filler, host) in fills_host {
        if let Some(p) = out.parts.iter_mut().find(|p| p.name == filler) {
            p.info.host = Some(host);
        }
    }
    // Groups: elements made of parts that have no body of their own.
    for g in group_ids {
        let Some(e) = f.get(g) else { continue };
        if part_names.contains_key(&g) {
            continue; // it has a body: an element in its own right
        }
        let class = class_case(&e.ty);
        out.groups.push(Group {
            name: e.arg(2).as_str().unwrap_or(&crate::class_words(&class)).to_string(),
            predefined: predefined_of(e, &class).filter(|p| p != "NOTDEFINED"),
            class,
            type_name: e.arg(4).as_str().map(str::to_string),
            level: storey_of(g),
            tag: e.arg(7).as_str().map(str::to_string),
            psets: psets_of(g),
            quantities: quantities_of(g),
        });
    }

    // Spaces.
    let mut space_names: HashMap<u64, String> = HashMap::new();
    for (id, s) in f.all("IFCSPACE") {
        let (solids, _, open) = rd.body(s);
        let mut whole = Mesh::default();
        for m in solids.iter().chain(open.iter()) {
            geom::append(&mut whole, m);
        }
        if whole.triangles.is_empty() {
            continue;
        }
        let (lo, hi) = geom::bounds(&whole);
        let mut outline = geom::footprint(&geom::weld(&whole, 1e-3));
        if outline.is_empty() {
            outline = vec![vec![[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]]];
            rd.note(format!("space {}: its floor outline is its bounding box", s.arg(2).as_str().unwrap_or("?")));
        }
        let name = s.arg(2).as_str().unwrap_or("Space").to_string();
        space_names.insert(id, name.clone());
        let mut psets = psets_of(id);
        psets.retain(|k, _| !k.starts_with("Qto_"));
        out.spaces.push(Space {
            name,
            long_name: s.arg(7).as_str().map(str::to_string),
            level: storey_of(id),
            outline,
            elevation: lo[2],
            height: hi[2] - lo[2],
            predefined: s.arg(9).as_enum().filter(|p| *p != "NOTDEFINED").map(str::to_string),
            psets,
            classification: classes.get(&id).cloned().unwrap_or_default(),
        });
    }

    // Zones.
    for (id, z) in f.all("IFCZONE") {
        out.zones.push(Zone {
            name: z.arg(2).as_str().unwrap_or("Zone").to_string(),
            spaces: zone_members.get(&id).map(|m| m.iter().filter_map(|s| space_names.get(s).cloned()).collect()).unwrap_or_default(),
            psets: psets_of(id),
        });
    }

    // Grids.
    for (gid, g) in f.all("IFCGRID") {
        let place = rd.placement(g.arg(5).as_ref());
        let axes = |rd: &mut Reader, a: &Arg| -> Vec<GridAxis> {
            a.refs()
                .into_iter()
                .filter_map(|ax| {
                    let ae = rd.e(ax)?;
                    let pts = rd.curve(ae.arg(1).as_ref());
                    let (p, q) = (pts.first()?, pts.last()?);
                    let to = |p: &[f64; 2]| {
                        let w = geom::apply(&place, [p[0], p[1], 0.0]);
                        [w[0] * rd.mm, w[1] * rd.mm]
                    };
                    Some(GridAxis { tag: ae.arg(0).as_str().unwrap_or("").to_string(), from: to(p), to: to(q) })
                })
                .collect()
        };
        let u = axes(&mut rd, g.arg(7));
        let v = axes(&mut rd, g.arg(8));
        out.grids.push(Grid { name: g.arg(2).as_str().unwrap_or("Grid").to_string(), level: storey_of(gid), u, v });
    }
    out.notes = rd.notes;
    if out.project.is_empty() {
        out.project = "Imported".into();
    }
    Ok(out)
}

fn is_product(e: &Entity) -> bool {
    // A product has an object placement (a reference) or a representation at 5 and 6.
    e.args.len() >= 8 && e.arg(0).as_str().is_some_and(|g| g.len() == 22) && (e.arg(5).as_ref().is_some() || e.arg(6).as_ref().is_some()) && !e.ty.starts_with("IFCREL") && !e.ty.ends_with("TYPE")
}

fn predefined_of(e: &Entity, class: &str) -> Option<String> {
    let at = match class {
        "IfcDoor" | "IfcWindow" | "IfcDoorStandardCase" | "IfcWindowStandardCase" => 10,
        "IfcStairFlight" => 12,
        "IfcReinforcingBar" => 12,
        "IfcElementAssembly" => 9,
        _ => 8,
    };
    e.arg(at).as_enum().map(str::to_string)
}

fn attributes(e: &Entity, class: &str, mm: f64, info: &mut Element) {
    let num = |info: &mut Element, k: &str, a: usize, scale: f64| {
        if let Some(v) = e.arg(a).as_num() {
            info.attributes.insert(k.into(), PValue::Num(v * scale));
        }
    };
    match class {
        "IfcDoor" | "IfcWindow" | "IfcDoorStandardCase" | "IfcWindowStandardCase" => {
            num(info, "OverallHeight", 8, mm);
            num(info, "OverallWidth", 9, mm);
            if let Some(op) = e.arg(11).as_enum().filter(|o| *o != "NOTDEFINED") {
                let k = if class.starts_with("IfcDoor") { "OperationType" } else { "PartitioningType" };
                info.attributes.insert(k.into(), PValue::Text(op.to_string()));
            }
        }
        "IfcStairFlight" => {
            num(info, "NumberOfRisers", 8, 1.0);
            num(info, "NumberOfTreads", 9, 1.0);
            num(info, "RiserHeight", 10, mm);
            num(info, "TreadLength", 11, mm);
        }
        "IfcReinforcingBar" => {
            if let Some(g) = e.arg(8).as_str() {
                info.attributes.insert("SteelGrade".into(), PValue::Text(g.to_string()));
            }
            num(info, "NominalDiameter", 9, mm);
            num(info, "BarLength", 11, mm);
        }
        _ => {}
    }
}

fn material_name(f: &File, id: u64, depth: usize) -> Option<String> {
    let e = f.get(id)?;
    if depth > 6 {
        return None;
    }
    match e.ty.as_str() {
        "IFCMATERIAL" => e.arg(0).as_str().map(str::to_string),
        "IFCMATERIALLAYERSETUSAGE" | "IFCMATERIALPROFILESETUSAGE" | "IFCMATERIALPROFILESETUSAGETAPERING" => material_name(f, e.arg(0).as_ref()?, depth + 1),
        "IFCMATERIALLAYERSET" => {
            // The thickest layer names it.
            let layers = e.arg(0).refs();
            let best = layers.iter().max_by(|a, b| {
                let t = |x: &u64| f.get(*x).and_then(|l| l.arg(1).as_num()).unwrap_or(0.0);
                t(a).total_cmp(&t(b))
            })?;
            material_name(f, *best, depth + 1)
        }
        "IFCMATERIALLAYER" | "IFCMATERIALLAYERWITHOFFSETS" => material_name(f, e.arg(0).as_ref()?, depth + 1),
        "IFCMATERIALLIST" => material_name(f, *e.arg(0).refs().first()?, depth + 1),
        "IFCMATERIALCONSTITUENTSET" | "IFCMATERIALPROFILESET" => material_name(f, *e.arg(2).refs().first()?, depth + 1),
        "IFCMATERIALCONSTITUENT" | "IFCMATERIALPROFILE" => material_name(f, e.arg(2).as_ref()?, depth + 1),
        _ => None,
    }
}

fn classification(f: &File, id: u64) -> Option<Classification> {
    let e = f.get(id)?;
    if e.ty != "IFCCLASSIFICATIONREFERENCE" {
        return None;
    }
    let system = e.arg(3).as_ref().and_then(|s| f.get(s)).and_then(|s| if s.ty == "IFCCLASSIFICATION" { s.arg(3).as_str() } else { s.arg(2).as_str() }).unwrap_or("Classification").to_string();
    let code = e.arg(1).as_str().or(e.arg(0).as_str()).unwrap_or("").to_string();
    Some(Classification { system, code, name: e.arg(2).as_str().map(str::to_string) })
}

/// Millimetres per length unit, and the area and volume units in m², m³.
fn units(rd: &mut Reader) {
    let f = rd.f;
    let prefix = |p: Option<&str>| match p {
        Some("EXA") => 1e18,
        Some("PETA") => 1e15,
        Some("TERA") => 1e12,
        Some("GIGA") => 1e9,
        Some("MEGA") => 1e6,
        Some("KILO") => 1e3,
        Some("HECTO") => 1e2,
        Some("DECA") => 1e1,
        Some("DECI") => 1e-1,
        Some("CENTI") => 1e-2,
        Some("MILLI") => 1e-3,
        Some("MICRO") => 1e-6,
        Some("NANO") => 1e-9,
        _ => 1.0,
    };
    let Some((_, ua)) = f.all("IFCUNITASSIGNMENT").into_iter().next() else { return };
    for u in ua.arg(0).refs() {
        let Some(e) = f.get(u) else { continue };
        let kind = e.arg(1).as_enum().unwrap_or("");
        match e.ty.as_str() {
            "IFCSIUNIT" => {
                let p = prefix(e.arg(2).as_enum());
                match kind {
                    "LENGTHUNIT" => rd.mm = p * 1000.0,
                    "AREAUNIT" => rd.m2 = p * p,
                    "VOLUMEUNIT" => rd.m3 = p * p * p,
                    _ => {}
                }
            }
            "IFCCONVERSIONBASEDUNIT" | "IFCCONVERSIONBASEDUNITWITHOFFSET" => {
                // The factor in SI (metres): value × the unit component's own SI factor.
                let Some(m) = e.arg(3).as_ref().and_then(|m| f.get(m)) else { continue };
                let v = m.arg(0).as_num().unwrap_or(1.0);
                let base = m.arg(1).as_ref().and_then(|b| f.get(b)).map(|b| prefix(b.arg(2).as_enum())).unwrap_or(1.0);
                match kind {
                    "LENGTHUNIT" => rd.mm = v * base * 1000.0,
                    "AREAUNIT" => rd.m2 = v * base * base,
                    "VOLUMEUNIT" => rd.m3 = v * base * base * base,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
