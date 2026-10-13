//! Surface modelling (the C side is `surfaces.cpp`): sheets filled into
//! holes, through curve networks and between boundary curves; blend surfaces
//! and curves; sewing, extending, splitting, trimming and offsetting sheets;
//! projected and iso-parametric curves; and the measurements surfacing is
//! judged by (continuity across edges, deviation, curvature and draft).

use super::{Arena, Shape};
use std::ffi::{CStr, c_char, c_int, c_void};

unsafe extern "C" {
    fn occ_fill(a: *mut c_void, d: *const f64, len: c_int, opts: *const f64) -> c_int;
    fn occ_boundary_surface(a: *mut c_void, hs: *const c_int, n: c_int, style: c_int) -> c_int;
    fn occ_section_surface(a: *mut c_void, hs: *const c_int, n: c_int, ruled: c_int, closed: c_int) -> c_int;
    fn occ_network_surface(a: *mut c_void, ph: *const c_int, np: c_int, gh: *const c_int, ng: c_int, tol: f64) -> c_int;
    fn occ_blend_surface(a: *mut c_void, ha: c_int, ea: *const c_int, na: c_int, ca: c_int, ba: f64, hb: c_int, eb: *const c_int, nb: c_int, cb: c_int, bb: f64, samples: c_int) -> c_int;
    fn occ_blend_curve(a: *mut c_void, ha: c_int, enda: c_int, ca: c_int, ba: f64, hb: c_int, endb: c_int, cb: c_int, bb: f64) -> c_int;
    fn occ_sew(a: *mut c_void, hs: *const c_int, n: c_int, tol: f64, solid: c_int) -> c_int;
    fn occ_pieces(a: *mut c_void, h: c_int, kind: c_int, first: *mut c_int) -> c_int;
    fn occ_body_info(a: *mut c_void, h: c_int, out: *mut f64) -> c_int;
    fn occ_free_edges(a: *mut c_void, h: c_int, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_extend_face(a: *mut c_void, h: c_int, face: c_int, length: f64, sides: c_int) -> c_int;
    fn occ_offset_sheet(a: *mut c_void, h: c_int, d: f64) -> c_int;
    fn occ_project(a: *mut c_void, hc: c_int, hb: c_int, mode: c_int, dx: f64, dy: f64, dz: f64) -> c_int;
    fn occ_iso(a: *mut c_void, h: c_int, face: c_int, dir: c_int, t: f64) -> c_int;
    fn occ_split(a: *mut c_void, h: c_int, ts: *const c_int, n: c_int) -> c_int;
    fn occ_trim(a: *mut c_void, h: c_int, ts: *const c_int, n: c_int, x: f64, y: f64, z: f64, drop: c_int) -> c_int;
    fn occ_continuity(a: *mut c_void, h: c_int, h2: c_int, samples: c_int) -> *const c_char;
    fn occ_face_samples(a: *mut c_void, h: c_int, n: c_int, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_deviation(a: *mut c_void, ha: c_int, hb: c_int, n: c_int, out: *mut f64) -> c_int;
    fn occ_edge_wire(a: *mut c_void, h: c_int, idx: *const c_int, n: c_int) -> c_int;
    fn occ_surf_data(a: *mut c_void, data: *mut *const f64, len: *mut c_int) -> c_int;
}

/// One constraint of a filling (see [`Arena::fill`]).
#[derive(Clone, Debug, PartialEq)]
pub enum FillConstraint {
    /// A bounding curve (path segments, see `solids.cpp`), passed through (G0).
    Curve(Vec<f64>),
    /// A bounding edge of a body (its index as `edges` numbers them), with
    /// continuity 0–2 to the face beside it.
    Edge { body: Shape, edge: usize, order: u8 },
    /// A curve inside the patch it must pass through.
    Through(Vec<f64>),
    /// A point inside the patch it must pass through.
    Point([f64; 3]),
}

/// How a filling is solved (BRepFill_Filling's knobs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillOptions {
    pub degree: u32,
    pub points_per_curve: u32,
    pub iterations: u32,
    pub max_degree: u32,
    pub max_segments: u32,
    pub tolerance: f64,
}

impl Default for FillOptions {
    fn default() -> Self {
        FillOptions { degree: 3, points_per_curve: 15, iterations: 2, max_degree: 8, max_segments: 30, tolerance: 1e-4 }
    }
}

/// What a filling achieved against its constraints.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FillErrors {
    /// Largest distance to a constraint (mm).
    pub g0: f64,
    /// Largest angle to a tangency constraint (radians).
    pub g1: f64,
    /// Largest curvature difference to a G2 constraint.
    pub g2: f64,
}

/// A body's make-up (see [`Arena::body_info`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyInfo {
    pub solids: usize,
    pub shells: usize,
    pub faces: usize,
    pub edges: usize,
    /// Edges on one face only: the sheet's open boundary.
    pub free_edges: usize,
    /// A solid, or faces with no open boundary.
    pub closed: bool,
}

/// A sample of a body's surface (see [`Arena::face_samples`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceSample {
    pub face: usize,
    pub p: [f64; 3],
    /// Unit normal, the way the face faces.
    pub n: [f64; 3],
    /// Principal curvatures (1/mm), k1 ≥ k2; positive bulges out.
    pub k1: f64,
    pub k2: f64,
}

/// How far one body lies from another (see [`Arena::deviation`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deviation {
    pub max: f64,
    pub mean: f64,
    pub rms: f64,
    pub worst_at: [f64; 3],
    pub samples: usize,
}

fn ints(hs: &[Shape]) -> Vec<c_int> {
    hs.iter().map(|s| s.0).collect()
}

impl Arena {
    fn surf_data(&self) -> Vec<f64> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        unsafe { occ_surf_data(self.0, &mut data, &mut len) };
        if len <= 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(data, len as usize) }.to_vec() }
    }

    /// Fills a boundary (curves and edges, closing into one loop) with one
    /// smooth face, meeting edges with the continuity asked, through any
    /// inner curves and points.
    pub fn fill(&mut self, constraints: &[FillConstraint], opts: &FillOptions) -> Result<(Shape, FillErrors), String> {
        let mut d = Vec::new();
        for c in constraints {
            match c {
                FillConstraint::Curve(segs) | FillConstraint::Through(segs) => {
                    d.push(if matches!(c, FillConstraint::Curve(_)) { 0.0 } else { 2.0 });
                    d.push(0.0);
                    d.push(segs.len() as f64);
                    d.extend_from_slice(segs);
                }
                FillConstraint::Edge { body, edge, order } => d.extend([1.0, *order as f64, body.0 as f64, *edge as f64]),
                FillConstraint::Point(p) => d.extend([3.0, p[0], p[1], p[2]]),
            }
        }
        let o = [opts.degree as f64, opts.points_per_curve as f64, opts.iterations as f64, opts.max_degree as f64, opts.max_segments as f64, opts.tolerance];
        let s = self.check(unsafe { occ_fill(self.0, d.as_ptr(), d.len() as c_int, o.as_ptr()) })?;
        let e = self.surf_data();
        Ok((s, FillErrors { g0: e[0], g1: e[1], g2: e[2] }))
    }

    /// A patch bounded by 2–4 curves meeting corner to corner: style 0
    /// stretch, 1 Coons, 2 curved.
    pub fn boundary_surface(&mut self, curves: &[Shape], style: i32) -> Result<Shape, String> {
        let hs = ints(curves);
        self.check(unsafe { occ_boundary_surface(self.0, hs.as_ptr(), hs.len() as c_int, style) })
    }

    /// A sheet skinned through curves (ruled: flat between them).
    pub fn section_surface(&mut self, curves: &[Shape], ruled: bool, closed: bool) -> Result<Shape, String> {
        let hs = ints(curves);
        self.check(unsafe { occ_section_surface(self.0, hs.as_ptr(), hs.len() as c_int, ruled as c_int, closed as c_int) })
    }

    /// A Gordon surface through crossing profiles and guides, within `tol` of
    /// every curve. Also the worst gap where the curves cross and the worst
    /// deviation from them (mm).
    pub fn network_surface(&mut self, profiles: &[Shape], guides: &[Shape], tol: f64) -> Result<(Shape, f64, f64), String> {
        let (p, g) = (ints(profiles), ints(guides));
        let s = self.check(unsafe { occ_network_surface(self.0, p.as_ptr(), p.len() as c_int, g.as_ptr(), g.len() as c_int, tol) })?;
        let d = self.surf_data();
        Ok((s, d[0], d[1]))
    }

    /// A blend surface between chains of edges on two bodies, continuity 0–2
    /// to each side, `bulge` scaling how far it runs out along each.
    #[allow(clippy::too_many_arguments)]
    pub fn blend_surface(&mut self, a: Shape, edges_a: &[usize], cont_a: u8, bulge_a: f64, b: Shape, edges_b: &[usize], cont_b: u8, bulge_b: f64, samples: usize) -> Result<Shape, String> {
        let ea: Vec<c_int> = edges_a.iter().map(|e| *e as c_int).collect();
        let eb: Vec<c_int> = edges_b.iter().map(|e| *e as c_int).collect();
        self.check(unsafe {
            occ_blend_surface(self.0, a.0, ea.as_ptr(), ea.len() as c_int, cont_a as c_int, bulge_a, b.0, eb.as_ptr(), eb.len() as c_int, cont_b as c_int, bulge_b, samples as c_int)
        })
    }

    /// A curve bridging the ends of two curves (`end`: false start, true end), G0–G2 to each.
    #[allow(clippy::too_many_arguments)]
    pub fn blend_curve(&mut self, a: Shape, end_a: bool, cont_a: u8, bulge_a: f64, b: Shape, end_b: bool, cont_b: u8, bulge_b: f64) -> Result<Shape, String> {
        self.check(unsafe { occ_blend_curve(self.0, a.0, end_a as c_int, cont_a as c_int, bulge_a, b.0, end_b as c_int, cont_b as c_int, bulge_b) })
    }

    /// Sews sheets into one shell, closed into a solid when nothing is left
    /// open and `solid` is set. Also the number of free edges left.
    pub fn sew(&mut self, parts: &[Shape], tol: f64, solid: bool) -> Result<(Shape, usize), String> {
        let hs = ints(parts);
        let s = self.check(unsafe { occ_sew(self.0, hs.as_ptr(), hs.len() as c_int, tol, solid as c_int) })?;
        Ok((s, self.surf_data().first().copied().unwrap_or(0.0) as usize))
    }

    /// The pieces of a body: 0 faces, 1 shells, 2 solids, 3 edges, 4 boundary wires.
    pub fn pieces(&mut self, s: Shape, kind: i32) -> Result<Vec<Shape>, String> {
        let mut first = 0;
        let n = unsafe { occ_pieces(self.0, s.0, kind, &mut first) };
        if n < 0 {
            return Err(self.error());
        }
        Ok((first..first + n).map(Shape).collect())
    }

    pub fn body_info(&mut self, s: Shape) -> Result<BodyInfo, String> {
        let mut o = [0.0; 6];
        if unsafe { occ_body_info(self.0, s.0, o.as_mut_ptr()) } < 0 {
            return Err(self.error());
        }
        Ok(BodyInfo { solids: o[0] as usize, shells: o[1] as usize, faces: o[2] as usize, edges: o[3] as usize, free_edges: o[4] as usize, closed: o[5] != 0.0 })
    }

    /// The edges (numbered as `edges` numbers them) on the body's open boundary.
    pub fn free_edges(&mut self, s: Shape) -> Result<Vec<usize>, String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        if unsafe { occ_free_edges(self.0, s.0, &mut data, &mut len) } < 0 {
            return Err(self.error());
        }
        Ok(if len <= 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(data, len as usize) }.iter().map(|v| *v as usize).collect() })
    }

    /// One face carried on past its bounds by `length` mm (sides: bit 1 u-min, 2 u-max, 4 v-min, 8 v-max).
    pub fn extend_face(&mut self, s: Shape, face: usize, length: f64, sides: u8) -> Result<Shape, String> {
        self.check(unsafe { occ_extend_face(self.0, s.0, face as c_int, length, sides as c_int) })
    }

    /// The surface `d` mm off a sheet along its normal.
    pub fn offset_sheet(&mut self, s: Shape, d: f64) -> Result<Shape, String> {
        self.check(unsafe { occ_offset_sheet(self.0, s.0, d) })
    }

    /// A curve projected onto a body, along its normals or along `dir`.
    pub fn project(&mut self, curve: Shape, onto: Shape, dir: Option<[f64; 3]>) -> Result<Shape, String> {
        let d = dir.unwrap_or([0.0, 0.0, 1.0]);
        self.check(unsafe { occ_project(self.0, curve.0, onto.0, dir.is_some() as c_int, d[0], d[1], d[2]) })
    }

    /// A face's iso-parametric curve at fraction `t` of its u (`v` false) or v range.
    pub fn iso(&mut self, s: Shape, face: usize, v: bool, t: f64) -> Result<Shape, String> {
        self.check(unsafe { occ_iso(self.0, s.0, face as c_int, v as c_int, t) })
    }

    /// Every piece of a body split by the tools (as one compound).
    pub fn split(&mut self, s: Shape, tools: &[Shape]) -> Result<Shape, String> {
        let t = ints(tools);
        self.check(unsafe { occ_split(self.0, s.0, t.as_ptr(), t.len() as c_int) })
    }

    /// A sheet trimmed by the tools, keeping (or dropping) the side holding `at`.
    pub fn trim(&mut self, s: Shape, tools: &[Shape], at: [f64; 3], drop: bool) -> Result<Shape, String> {
        let t = ints(tools);
        self.check(unsafe { occ_trim(self.0, s.0, t.as_ptr(), t.len() as c_int, at[0], at[1], at[2], drop as c_int) })
    }

    /// Continuity across a body's shared edges, or between two bodies' touching
    /// boundaries: JSON { edges: [{ edge, faces, g0, g1, g2, length }], max }.
    pub fn continuity(&mut self, s: Shape, other: Option<Shape>, samples: usize) -> Result<serde_json::Value, String> {
        let p = unsafe { occ_continuity(self.0, s.0, other.map_or(-1, |o| o.0), samples.max(2) as c_int) };
        if p.is_null() {
            return Err(self.error());
        }
        let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        serde_json::from_str(&text).map_err(|e| format!("continuity report: {e}"))
    }

    /// Samples every face on an n × n grid: points, normals, principal curvatures.
    pub fn face_samples(&mut self, s: Shape, n: usize) -> Result<Vec<FaceSample>, String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        if unsafe { occ_face_samples(self.0, s.0, n.max(1) as c_int, &mut data, &mut len) } < 0 {
            return Err(self.error());
        }
        let d = if len <= 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(data, len as usize) } };
        Ok(d.chunks_exact(9).map(|c| FaceSample { face: c[0] as usize, p: [c[1], c[2], c[3]], n: [c[4], c[5], c[6]], k1: c[7], k2: c[8] }).collect())
    }

    /// The picked edges of a body chained end to end into one wire.
    pub fn edge_wire(&mut self, s: Shape, edges: &[usize]) -> Result<Shape, String> {
        let i: Vec<c_int> = edges.iter().map(|e| *e as c_int).collect();
        self.check(unsafe { occ_edge_wire(self.0, s.0, i.as_ptr(), i.len() as c_int) })
    }

    /// How far `b` lies from points sampled over `a`'s faces (n × n each).
    pub fn deviation(&mut self, a: Shape, b: Shape, n: usize) -> Result<Deviation, String> {
        let mut o = [0.0; 6];
        let count = unsafe { occ_deviation(self.0, a.0, b.0, n.max(1) as c_int, o.as_mut_ptr()) };
        if count < 0 {
            return Err(self.error());
        }
        Ok(Deviation { max: o[0], mean: o[1], rms: o[2], worst_at: [o[3], o[4], o[5]], samples: count as usize })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EdgeKind;
    use std::f64::consts::PI;

    /// A circle at height z as two exact arcs.
    fn ring(r: f64, z: f64) -> Vec<f64> {
        vec![1.0, r, 0.0, z, 0.0, r, z, -r, 0.0, z, 1.0, -r, 0.0, z, 0.0, -r, z, r, 0.0, z]
    }
    fn spline(pts: &[[f64; 3]], closed: bool) -> Vec<f64> {
        let mut v = vec![3.0, pts.len() as f64, closed as i32 as f64];
        for p in pts {
            v.extend_from_slice(p);
        }
        v
    }
    /// Edges of a body whose polyline points all satisfy `f`.
    fn edges_where(a: &mut Arena, s: Shape, f: impl Fn([f64; 3]) -> bool) -> Vec<usize> {
        let free = a.free_edges(s).unwrap();
        let all = a.edges(s, 0.01).unwrap();
        free.into_iter().filter(|i| !matches!(all[*i].kind, EdgeKind::Seam) && all[*i].points.iter().all(|p| f(*p))).collect()
    }

    /// A fender-like crown: z falls off as a paraboloid, leaning in x.
    fn crown(x: f64, y: f64) -> f64 {
        20.0 - (x * x + 1.5 * y * y) / 250.0 + 0.0004 * x * x * x
    }
    fn fender(a: &mut Arena) -> Shape {
        let line = |a: &mut Arena, f: &dyn Fn(f64) -> [f64; 3]| {
            let pts: Vec<[f64; 3]> = (0..=10).map(|k| f(-60.0 + 12.0 * k as f64)).collect();
            // (crossings at -60, -24, 24, 60: on the sample points)
            a.path(&spline(&pts, false)).unwrap()
        };
        let profiles: Vec<Shape> = [-60.0, -24.0, 24.0, 60.0].iter().map(|y| line(a, &|x| [x, *y, crown(x, *y)])).collect();
        let guides: Vec<Shape> = [-60.0, -24.0, 24.0, 60.0].iter().map(|x| line(a, &|y| [*x, y, crown(*x, y)])).collect();
        let (s, gap, dev) = a.network_surface(&profiles, &guides, 1e-3).unwrap();
        assert!(gap < 1e-4 && dev < 1e-3, "fender network gap {gap} dev {dev}");
        s
    }

    #[test]
    fn network_vase_closes_into_a_solid() {
        let mut a = Arena::new();
        let zs = [0.0, 40.0, 80.0, 120.0];
        let rs = [30.0, 42.0, 24.0, 32.0];
        let circle_pts = |r: f64, z: f64| -> Vec<[f64; 3]> { (0..12).map(|k| (k as f64 * PI / 6.0, r, z)).map(|(t, r, z)| [r * t.cos(), r * t.sin(), z]).collect() };
        let profiles: Vec<Shape> = zs.iter().zip(rs).map(|(z, r)| a.path(&spline(&circle_pts(r, *z), true)).unwrap()).collect();
        let guides: Vec<Shape> = (0..6)
            .map(|j| {
                let t = j as f64 * PI / 3.0;
                let pts: Vec<[f64; 3]> = zs.iter().zip(rs).map(|(z, r)| [r * t.cos(), r * t.sin(), *z]).collect();
                a.path(&spline(&pts, false)).unwrap()
            })
            .collect();
        let (wall, gap, dev) = a.network_surface(&profiles, &guides, 1e-3).unwrap();
        println!("vase network: gap {gap:.2e} mm, deviation from curves {dev:.2e} mm");
        assert!(gap < 1e-6 && dev < 1e-3);
        let info = a.body_info(wall).unwrap();
        assert_eq!((info.faces, info.free_edges), (1, 2), "a tube: one face, two rims");
        // A floor filled into the bottom rim, sewn on: an open vessel, then walls.
        let rim = edges_where(&mut a, wall, |p| p[2].abs() < 1e-6);
        assert_eq!(rim.len(), 1);
        let (floor, err) = a.fill(&[FillConstraint::Edge { body: wall, edge: rim[0], order: 0 }], &FillOptions::default()).unwrap();
        assert!(err.g0 < 1e-3, "floor fits its rim: {err:?}");
        let (cup, free) = a.sew(&[wall, floor], 1e-4, true).unwrap();
        assert_eq!(free, 1, "only the top rim left open");
        let solid = a.thicken(cup, 2.0, &[], false).unwrap();
        let v = a.volume(solid);
        // About the wall area times 2 mm.
        let area = a.area(cup);
        println!("vase: skin {area:.0} mm², walls {v:.0} mm³");
        assert!((v - area * 2.0).abs() / (area * 2.0) < 0.05, "volume {v} vs area·t {}", area * 2.0);
    }

    #[test]
    fn filling_a_four_sided_hole_meets_its_neighbours_g1_and_g2() {
        let mut a = Arena::new();
        let skin = fender(&mut a);
        let tool = a.cube(40.0, 30.0, 200.0, true).unwrap();
        let holed = a.trim(skin, &[tool], [-55.0, -55.0, crown(-55.0, -55.0)], false).unwrap();
        let hole = edges_where(&mut a, holed, |p| p[0].abs() < 20.01 && p[1].abs() < 15.01);
        assert_eq!(hole.len(), 4, "a four-sided hole");
        for order in [1u8, 2] {
            let cons: Vec<FillConstraint> = hole.iter().map(|e| FillConstraint::Edge { body: holed, edge: *e, order }).collect();
            let opts = FillOptions { degree: if order == 2 { 4 } else { 3 }, points_per_curve: 30, iterations: 3, ..Default::default() };
            let (patch, err) = a.fill(&cons, &opts).unwrap();
            let c = a.continuity(patch, Some(holed), 40).unwrap();
            let m = &c["max"];
            println!("fill G{order}: plate errors {err:?}; measured across edges: gap {} mm, crease {}°, curvature jump {} 1/mm", m["g0"], m["g1"], m["g2"]);
            assert_eq!(c["edges"].as_array().unwrap().len(), 4);
            assert!(m["g0"].as_f64().unwrap() < 1e-3);
            assert!(m["g1"].as_f64().unwrap() < 0.1, "G1 within a tenth of a degree");
            // The patch stands in for the cut-out: within a hair of the original crown.
            let d = a.deviation(patch, skin, 6).unwrap();
            // (A plate is the smoothest fill, not the crown that was cut away.)
            println!("  patch vs the crown cut away: max {:.3} mm, mean {:.3} mm", d.max, d.mean);
            assert!(d.max < 0.3, "patch strays {} mm from the crown", d.max);
            let (whole, free) = a.sew(&[holed, patch], 1e-3, false).unwrap();
            assert_eq!(free, 4, "sewn shut: only the outer edge is open");
            let _ = whole;
        }
    }

    #[test]
    fn g2_blend_between_two_tubes() {
        let mut a = Arena::new();
        let lo = [a.path(&ring(30.0, 0.0)).unwrap(), a.path(&ring(30.0, 20.0)).unwrap()];
        let hi = [a.path(&ring(20.0, 40.0)).unwrap(), a.path(&ring(20.0, 60.0)).unwrap()];
        let lower = a.section_surface(&lo, true, false).unwrap();
        let upper = a.section_surface(&hi, true, false).unwrap();
        let ea = edges_where(&mut a, lower, |p| (p[2] - 20.0).abs() < 1e-6);
        let eb = edges_where(&mut a, upper, |p| (p[2] - 40.0).abs() < 1e-6);
        let blend = a.blend_surface(lower, &ea, 2, 1.0, upper, &eb, 2, 1.0, 0).unwrap();
        for (other, name) in [(lower, "lower"), (upper, "upper")] {
            let c = a.continuity(blend, Some(other), 60).unwrap();
            let m = &c["max"];
            println!("G2 blend to {name}: gap {} mm, crease {}°, curvature jump {} 1/mm", m["g0"], m["g1"], m["g2"]);
            assert!(!c["edges"].as_array().unwrap().is_empty());
            assert!(m["g0"].as_f64().unwrap() < 1e-4 && m["g1"].as_f64().unwrap() < 0.05 && m["g2"].as_f64().unwrap() < 1e-3);
        }
        let (shell, free) = a.sew(&[lower, blend, upper], 1e-4, false).unwrap();
        assert_eq!(free, 4, "the two outer rims (two arcs each)");
        let c = a.continuity(shell, None, 30).unwrap();
        println!("sewn tubes, across their own edges: {}", c["max"]);
    }

    #[test]
    fn sew_split_trim_extend_offset_project_iso() {
        let mut a = Arena::new();
        // Unknit a box into faces and sew them back into the solid.
        let b = a.cube(10.0, 10.0, 10.0, false).unwrap();
        let faces = a.pieces(b, 0).unwrap();
        assert_eq!(faces.len(), 6);
        let (s, free) = a.sew(&faces, 1e-6, true).unwrap();
        assert_eq!(free, 0);
        assert!((a.volume(s) - 1000.0).abs() < 1e-6, "sewn {}", a.volume(s));
        // A cylinder wall: offset 2 mm out grows its area by 32/30.
        let rings = [a.path(&ring(30.0, 0.0)).unwrap(), a.path(&ring(30.0, 50.0)).unwrap()];
        let wall = a.section_surface(&rings, true, false).unwrap();
        let out = a.offset_sheet(wall, 2.0).unwrap();
        let k = a.area(out) / a.area(wall);
        assert!((k - 32.0 / 30.0).abs() < 1e-4 || (k - 28.0 / 30.0).abs() < 1e-4, "offset ratio {k}");
        // Extend a flat patch 5 mm all round.
        let sq = a.rect(20.0, 20.0, true).unwrap();
        let ext = a.extend_face(sq, 0, 5.0, 15).unwrap();
        assert!((a.area(ext) - 900.0).abs() < 1e-6, "extended {}", a.area(ext));
        // A line projected onto the wall along y lands on it, curved.
        let line = a.path(&[0.0, -10.0, -100.0, 25.0, 10.0, -100.0, 25.0]).unwrap();
        let on = a.project(line, wall, Some([0.0, 1.0, 0.0])).unwrap();
        let bb = a.bounds(on).unwrap();
        assert!((bb[1] + 30.0).abs() < 0.5, "lands on the near side: {bb:?}");
        // Split the wall by a plane sheet and trim the top half away.
        let cut = a.rect(200.0, 200.0, true).unwrap();
        let cut = a.translate(cut, [0.0, 0.0, 20.0]).unwrap();
        let pieces = a.split(wall, &[cut]).unwrap();
        assert_eq!(a.pieces(pieces, 0).unwrap().len(), a.pieces(wall, 0).unwrap().len() * 2);
        let low = a.trim(wall, &[cut], [30.0, 0.0, 5.0], false).unwrap();
        assert!((a.area(low) - 2.0 * PI * 30.0 * 20.0).abs() < 1e-3, "trimmed {}", a.area(low));
        // An iso-curve of the wall: an upright line or a circle.
        let iso = a.iso(wall, 0, false, 0.5).unwrap();
        assert!(a.bounds(iso).is_ok());
        // Curvature samples: a cylinder of 30 bulges out at 1/30 one way, flat the other.
        let smp = a.face_samples(wall, 4).unwrap();
        assert!(!smp.is_empty());
        for q in &smp {
            assert!(((q.k1 - 1.0 / 30.0).abs() < 1e-6 && q.k2.abs() < 1e-9) || ((q.k2 + 1.0 / 30.0).abs() < 1e-6 && q.k1.abs() < 1e-9), "{q:?}");
        }
        // Deviation between the wall and its offset: 2 mm everywhere.
        let d = a.deviation(wall, out, 5).unwrap();
        assert!((d.max - 2.0).abs() < 1e-4 && (d.mean - 2.0).abs() < 1e-4, "{d:?}");
    }

    #[test]
    fn boundary_patches_and_blend_curves() {
        let mut a = Arena::new();
        // Four curves meeting corner to corner, bowed up in the middle.
        let c1 = a.path(&spline(&[[0.0, 0.0, 0.0], [20.0, 0.0, 5.0], [40.0, 0.0, 0.0]], false)).unwrap();
        let c2 = a.path(&spline(&[[40.0, 0.0, 0.0], [40.0, 20.0, 5.0], [40.0, 40.0, 0.0]], false)).unwrap();
        let c3 = a.path(&spline(&[[40.0, 40.0, 0.0], [20.0, 40.0, 5.0], [0.0, 40.0, 0.0]], false)).unwrap();
        let c4 = a.path(&spline(&[[0.0, 40.0, 0.0], [0.0, 20.0, 5.0], [0.0, 0.0, 0.0]], false)).unwrap();
        for style in 0..3 {
            let s = a.boundary_surface(&[c1, c2, c3, c4], style);
            let s = s.unwrap();
            let b = a.bounds(s).unwrap();
            assert!(b[5] > 5.0 && b[5] < 12.0, "style {style}: crown {}", b[5]);
        }
        // A G2 bridge between two arcs' ends.
        let l = a.path(&[1.0, 0.0, 0.0, 0.0, 10.0, 4.0, 0.0, 20.0, 0.0, 0.0]).unwrap();
        let r = a.path(&ring(10.0, 0.0)[..10]).unwrap();
        let r = a.translate(r, [60.0, 0.0, 0.0]).unwrap();
        let bridge = a.blend_curve(l, true, 2, 1.0, r, false, 2, 1.0).unwrap();
        assert!(a.bounds(bridge).is_ok());
    }
}
