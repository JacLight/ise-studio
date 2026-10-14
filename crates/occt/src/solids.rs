//! The exact solid vocabulary beyond prisms and revolves (the C side is
//! `solids.cpp`): paths and sweeps along them, lofts, offset and thickened
//! solids, variable fillets, defeaturing, paint for STEP, geometry checks and
//! BREP files.

use super::{Arena, Shape};
use std::ffi::{CStr, CString, c_char, c_int, c_void};

unsafe extern "C" {
    fn occ_path(a: *mut c_void, segs: *const f64, n: c_int) -> c_int;
    fn occ_path_frames(
        a: *mut c_void,
        segs: *const f64,
        nsegs: c_int,
        mode: c_int,
        margs: *const f64,
        nmargs: c_int,
        place: c_int,
        pargs: *const f64,
        dphi: f64,
        twist: f64,
        data: *mut *const f64,
        len: *mut c_int,
    ) -> c_int;
    fn occ_sweep(
        a: *mut c_void,
        h: c_int,
        segs: *const f64,
        nsegs: c_int,
        mode: c_int,
        margs: *const f64,
        nmargs: c_int,
        place: c_int,
        pargs: *const f64,
        twist: f64,
        scale: f64,
    ) -> c_int;
    fn occ_loft(a: *mut c_void, hs: *const c_int, mats: *const f64, n: c_int, ruled: c_int, closed: c_int, solid: c_int, continuity: c_int) -> c_int;
    fn occ_offset_solid(a: *mut c_void, h: c_int, d: f64, join: c_int) -> c_int;
    fn occ_thicken(a: *mut c_void, h: c_int, t: f64, idx: *const c_int, n: c_int, join: c_int) -> c_int;
    fn occ_blend_var(a: *mut c_void, h: c_int, kind: c_int, r1: f64, r2: f64, idx: *const c_int, n: c_int) -> c_int;
    fn occ_surface_info(a: *mut c_void, h: c_int, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_defeature(a: *mut c_void, h: c_int, idx: *const c_int, n: c_int) -> c_int;
    fn occ_paint(a: *mut c_void, h: c_int, idx: *const c_int, n: c_int, r: f64, g: f64, b: f64, name: *const c_char) -> c_int;
    fn occ_check(a: *mut c_void, h: c_int) -> *const c_char;
    fn occ_write_brep(a: *mut c_void, hs: *const c_int, n: c_int, path: *const c_char) -> c_int;
    fn occ_read_brep(a: *mut c_void, path: *const c_char, first: *mut c_int) -> c_int;
}

/// How a swept section turns as it follows its path.
#[derive(Clone, Debug, PartialEq)]
pub enum Orientation {
    /// Rotation minimising (corrected Frenet): no twist the path does not force. The default.
    Corrected,
    /// Frenet: square to the path's curvature (a helix's sections stay radial).
    Frenet,
    /// Never turns: the section is only carried along.
    Fixed,
    /// Its binormal held along one direction (threads: the helix's axis).
    Binormal([f64; 3]),
    /// Its normal pointing at another path (same fraction of the way along).
    Auxiliary(Vec<f64>),
}

impl Orientation {
    fn code(&self) -> (c_int, Vec<f64>) {
        match self {
            Orientation::Corrected => (0, vec![]),
            Orientation::Frenet => (1, vec![]),
            Orientation::Fixed => (2, vec![]),
            Orientation::Binormal(d) => (3, d.to_vec()),
            Orientation::Auxiliary(segs) => (4, segs.clone()),
        }
    }
}

/// Where a profile (drawn on z = 0) sits when it starts along a path.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// Square to the path at its start, centred on it, the profile's y toward `up` where it can.
    Normal { up: [f64; 3] },
    /// For a path starting with a helix: drawn as for revolve (x the radius,
    /// y along the axis), in the plane through the axis and the start.
    Axial,
    /// As given: the profile's origin, x and y directions.
    Frame { origin: [f64; 3], x: [f64; 3], y: [f64; 3] },
}

impl Place {
    fn code(&self) -> (c_int, [f64; 9]) {
        match self {
            Place::Normal { up } => (0, [up[0], up[1], up[2], 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            Place::Axial => (1, [0.0; 9]),
            Place::Frame { origin: o, x, y } => (2, [o[0], o[1], o[2], x[0], x[1], x[2], y[0], y[1], y[2]]),
        }
    }
}

/// A path sampled for the mesh kernel, in the frames the exact sweep uses.
#[derive(Clone, Debug)]
pub struct PathFrames {
    /// The profile's origin, x, y and z (out of it) at the start.
    pub place: [[f64; 3]; 4],
    /// The path turns sharply somewhere (mitred, not bent).
    pub corners: bool,
    pub samples: Vec<FrameSample>,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameSample {
    /// Distance along the path from its start.
    pub s: f64,
    pub p: [f64; 3],
    /// Along the path.
    pub t: [f64; 3],
    /// Across it.
    pub n: [f64; 3],
    pub b: [f64; 3],
}

/// What a face is (see [`Arena::surface_info`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceInfo {
    /// "plane", "cylinder", "cone", "sphere", "torus" or "other".
    pub kind: &'static str,
    pub radius: f64,
    pub axis: [f64; 3],
    pub origin: [f64; 3],
    /// 1 bulging out (a boss, an outside round), -1 in (a hole, an inside round), 0 neither.
    pub bulge: i8,
    /// The angle it spans round its axis (radians; a torus: its round's own sweep).
    pub span: f64,
}

fn idx(list: &[usize]) -> Vec<c_int> {
    list.iter().map(|e| *e as c_int).collect()
}

impl Arena {
    /// A path (lines, arcs, helices, splines, spirals; see `solids.cpp`) as a wire.
    pub fn path(&mut self, segs: &[f64]) -> Result<Shape, String> {
        self.check(unsafe { occ_path(self.0, segs.as_ptr(), segs.len() as c_int) })
    }

    /// Samples a path so a section reaching `reach` mm from it turns at most `dphi`
    /// radians between samples (curvature, torsion and `twist` degrees all counted).
    pub fn path_frames(&mut self, segs: &[f64], orient: &Orientation, place: &Place, dphi: f64, twist: f64) -> Result<PathFrames, String> {
        let (mode, margs) = orient.code();
        let (pl, pargs) = place.code();
        let (mut data, mut len) = (std::ptr::null(), 0);
        let n = unsafe {
            occ_path_frames(self.0, segs.as_ptr(), segs.len() as c_int, mode, margs.as_ptr(), margs.len() as c_int, pl, pargs.as_ptr(), dphi, twist, &mut data, &mut len)
        };
        if n < 0 {
            return Err(self.error());
        }
        let d = unsafe { std::slice::from_raw_parts(data, len as usize) };
        let v = |i: usize| [d[i], d[i + 1], d[i + 2]];
        let samples = d[13..].chunks_exact(13).map(|c| FrameSample { s: c[0], p: [c[1], c[2], c[3]], t: [c[4], c[5], c[6]], n: [c[7], c[8], c[9]], b: [c[10], c[11], c[12]] }).collect();
        Ok(PathFrames { place: [v(0), v(3), v(6), v(9)], corners: d[12] != 0.0, samples })
    }

    /// Sweeps a profile along a path, exactly: `twist` degrees about the path
    /// over its length, the section `scale` times its size by the end.
    pub fn sweep(&mut self, profile: Shape, segs: &[f64], orient: &Orientation, place: &Place, twist: f64, scale: f64) -> Result<Shape, String> {
        let (mode, margs) = orient.code();
        let (pl, pargs) = place.code();
        self.check(unsafe { occ_sweep(self.0, profile.0, segs.as_ptr(), segs.len() as c_int, mode, margs.as_ptr(), margs.len() as c_int, pl, pargs.as_ptr(), twist, scale) })
    }

    /// A loft through sections: each a profile (None for a point) placed by
    /// origin, x, y, z (12 numbers). `continuity` 0–2 for smooth lofts.
    pub fn loft(&mut self, sections: &[(Option<Shape>, [f64; 12])], ruled: bool, closed: bool, solid: bool, continuity: i32) -> Result<Shape, String> {
        let hs: Vec<c_int> = sections.iter().map(|(s, _)| s.map_or(-1, |s| s.0)).collect();
        let mats: Vec<f64> = sections.iter().flat_map(|(_, m)| *m).collect();
        self.check(unsafe { occ_loft(self.0, hs.as_ptr(), mats.as_ptr(), hs.len() as c_int, ruled as c_int, closed as c_int, solid as c_int, continuity) })
    }

    /// Grows (positive) or shrinks the whole solid by `d` mm; `sharp` extends
    /// faces to meet instead of rounding the grown edges.
    pub fn offset_solid(&mut self, s: Shape, d: f64, sharp: bool) -> Result<Shape, String> {
        self.check(unsafe { occ_offset_solid(self.0, s.0, d, sharp as c_int) })
    }

    /// A sheet given thickness `t`, or a solid hollowed to walls `t` thick open at `faces`.
    pub fn thicken(&mut self, s: Shape, t: f64, faces: &[usize], sharp: bool) -> Result<Shape, String> {
        let i = idx(faces);
        self.check(unsafe { occ_thicken(self.0, s.0, t, i.as_ptr(), i.len() as c_int, sharp as c_int) })
    }

    /// A fillet whose radius runs from `a` to `b` along each edge, or a chamfer
    /// `a` back along one face and `b` along the other.
    pub fn blend_var(&mut self, s: Shape, kind: super::Blend, a: f64, b: f64, edges: &[usize]) -> Result<Shape, String> {
        let i = idx(edges);
        let k = match kind {
            super::Blend::Fillet => 0,
            super::Blend::Chamfer => 1,
        };
        self.check(unsafe { occ_blend_var(self.0, s.0, k, a, b, i.as_ptr(), i.len() as c_int) })
    }

    /// What every face is, numbered as [`Arena::faces`] numbers them.
    pub fn surface_info(&mut self, s: Shape) -> Result<Vec<SurfaceInfo>, String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        let n = unsafe { occ_surface_info(self.0, s.0, &mut data, &mut len) };
        if n < 0 {
            return Err(self.error());
        }
        let d = unsafe { std::slice::from_raw_parts(data, len as usize) };
        Ok(d
            .chunks_exact(10)
            .map(|f| SurfaceInfo {
                kind: ["plane", "cylinder", "cone", "sphere", "torus", "other"][(f[0] as usize).min(5)],
                radius: f[1],
                axis: [f[2], f[3], f[4]],
                origin: [f[5], f[6], f[7]],
                bulge: f[8] as i8,
                span: f[9],
            })
            .collect())
    }

    /// Removes the listed faces (a hole, a round, a boss) and heals over them.
    pub fn defeature(&mut self, s: Shape, faces: &[usize]) -> Result<Shape, String> {
        let i = idx(faces);
        self.check(unsafe { occ_defeature(self.0, s.0, i.as_ptr(), i.len() as c_int) })
    }

    /// Colours (sRGB 0–1) and names faces (None: the whole shape) for STEP.
    pub fn paint(&mut self, s: Shape, faces: Option<&[usize]>, colour: Option<[f64; 3]>, name: &str) -> Result<Shape, String> {
        let i = faces.map(idx).unwrap_or_default();
        let c = colour.unwrap_or([-1.0, 0.0, 0.0]);
        let n = CString::new(name.replace('\0', "")).unwrap();
        self.check(unsafe { occ_paint(self.0, s.0, i.as_ptr(), if faces.is_some() { i.len() as c_int } else { -1 }, c[0], c[1], c[2], n.as_ptr()) })
    }

    /// A colour for the whole shape that anything painted on it overrides (a part's material).
    pub fn paint_default(&mut self, s: Shape, colour: [f64; 3]) -> Result<Shape, String> {
        let n = CString::new("").unwrap();
        self.check(unsafe { occ_paint(self.0, s.0, std::ptr::null(), -2, colour[0], colour[1], colour[2], n.as_ptr()) })
    }

    /// A geometry check report (validity, self-intersection, tolerances), as JSON.
    pub fn check_geometry(&mut self, s: Shape) -> Result<serde_json::Value, String> {
        let p = unsafe { occ_check(self.0, s.0) };
        if p.is_null() {
            return Err(self.error());
        }
        let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        serde_json::from_str(&text).map_err(|e| format!("check report: {e}"))
    }

    pub fn write_brep(&mut self, shapes: &[Shape], path: &std::path::Path) -> Result<(), String> {
        let hs: Vec<c_int> = shapes.iter().map(|s| s.0).collect();
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        if unsafe { occ_write_brep(self.0, hs.as_ptr(), hs.len() as c_int, p.as_ptr()) } < 0 { Err(self.error()) } else { Ok(()) }
    }

    /// Reads a BREP file's solids (named "Imported body").
    pub fn read_brep(&mut self, path: &std::path::Path) -> Result<Vec<(String, Shape)>, String> {
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mut first = 0;
        let n = unsafe { occ_read_brep(self.0, p.as_ptr(), &mut first) };
        if n < 0 {
            return Err(self.error());
        }
        Ok((first..first + n).map(|h| (unsafe { CStr::from_ptr(super::occ_name(self.0, h)) }.to_string_lossy().into_owned(), Shape(h))).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn helix(r: f64, pitch: f64, height: f64, taper: f64, left: bool) -> Vec<f64> {
        vec![2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, r, pitch, height, taper, if left { 1.0 } else { 0.0 }]
    }

    #[test]
    fn spring_is_a_helical_tube() {
        let mut a = Arena::new();
        let (r, pitch, height, wire) = (10.0, 4.0, 20.0, 1.0);
        let path = helix(r, pitch, height, 0.0, false);
        let length = (height / pitch) * ((2.0 * PI * r).powi(2) + pitch * pitch).sqrt();
        let w = a.path(&path).unwrap();
        let measured = a.edges(w, 0.01).unwrap().iter().map(|e| if let crate::EdgeKind::Curve { length } = e.kind { length } else { 0.0 }).sum::<f64>();
        assert!((measured - length).abs() < 1e-6 * length, "helix length {measured} vs {length}");
        let c = a.circle(wire).unwrap();
        let spring = a.sweep(c, &path, &Orientation::Frenet, &Place::Normal { up: [0.0, 0.0, 1.0] }, 0.0, 1.0).unwrap();
        let want = PI * wire * wire * length;
        let err = (a.volume(spring) - want).abs() / want;
        assert!(err < 1e-5, "spring volume {} vs {want} ({err:e})", a.volume(spring));
        let corrected = a.sweep(c, &path, &Orientation::Corrected, &Place::Normal { up: [0.0, 0.0, 1.0] }, 0.0, 1.0).unwrap();
        assert!((a.volume(corrected) - want).abs() / want < 1e-5);
    }

    #[test]
    fn screw_sweep_of_an_axial_profile_follows_pappus() {
        // A square 1 × 1 at radius 5–6 swept 3 turns as a screw: 3 · 2π · x̄ · A.
        let mut a = Arena::new();
        let sq = a.polygons(&[vec![[5.0, 0.0], [6.0, 0.0], [6.0, 1.0], [5.0, 1.0]]]).unwrap();
        let path = helix(5.5, 2.0, 6.0, 0.0, false);
        let s = a.sweep(sq, &path, &Orientation::Binormal([0.0, 0.0, 1.0]), &Place::Axial, 0.0, 1.0).unwrap();
        let want = 3.0 * 2.0 * PI * 5.5 * 1.0;
        let err = (a.volume(s) - want).abs() / want;
        assert!(err < 1e-5, "screw volume {} vs {want} ({err:e})", a.volume(s));
        let b = a.bounds(s).unwrap();
        assert!((b[5] - 7.0).abs() < 1e-4 && b[2].abs() < 1e-4, "rises 6 mm plus the profile's 1: {b:?}");
        // Left-handed: the same volume, mirrored.
        let l = a.sweep(sq, &helix(5.5, 2.0, 6.0, 0.0, true), &Orientation::Frenet, &Place::Axial, 0.0, 1.0).unwrap();
        assert!((a.volume(l) - want).abs() / want < 1e-5);
    }

    #[test]
    fn pipes_bend_and_scale() {
        let mut a = Arena::new();
        // An L of two 50 mm legs joined by a 20 mm bend: lines and a true arc.
        let r = 20.0;
        let k = r * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
        let segs = vec![
            0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 0.0, //
            1.0, 50.0, 0.0, 0.0, 50.0 + r - k, 0.0, k, 50.0 + r, 0.0, r, //
            0.0, 50.0 + r, 0.0, r, 50.0 + r, 0.0, r + 50.0,
        ];
        let ring = a.circle(5.0).unwrap();
        let bore = a.circle(4.0).unwrap();
        let tube = a.boolean(crate::Bool::Difference, ring, bore).unwrap();
        let pipe = a.sweep(tube, &segs, &Orientation::Corrected, &Place::Normal { up: [0.0, 0.0, 1.0] }, 0.0, 1.0).unwrap();
        let length = 100.0 + PI * r / 2.0;
        let want = PI * (25.0 - 16.0) * length;
        assert!((a.volume(pipe) - want).abs() / want < 1e-6, "pipe {} vs {want}", a.volume(pipe));
        // A straight cone by scale law: a frustum r 5 → 10 over 30.
        let line = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 30.0];
        let c = a.circle(5.0).unwrap();
        let cone = a.sweep(c, &line, &Orientation::Corrected, &Place::Normal { up: [0.0, 0.0, 1.0] }, 0.0, 2.0).unwrap();
        let want = PI * 30.0 / 3.0 * (25.0 + 50.0 + 100.0);
        assert!((a.volume(cone) - want).abs() / want < 1e-4, "scaled {} vs {want}", a.volume(cone));
        // Twisted square, straight: the volume stays, the top turns.
        let sq = a.rect(10.0, 10.0, true).unwrap();
        let tw = a.sweep(sq, &line, &Orientation::Corrected, &Place::Normal { up: [0.0, 0.0, 1.0] }, 90.0, 1.0).unwrap();
        assert!((a.volume(tw) - 3000.0).abs() / 3000.0 < 1e-3, "twisted {}", a.volume(tw));
    }

    #[test]
    fn lofts_measure_like_frustums() {
        let mut a = Arena::new();
        let at = |z: f64| [0.0, 0.0, z, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let big = a.rect(20.0, 20.0, true).unwrap();
        let small = a.rect(10.0, 10.0, true).unwrap();
        let f = a.loft(&[(Some(big), at(0.0)), (Some(small), at(15.0))], true, false, true, 2).unwrap();
        let want = 15.0 / 3.0 * (400.0 + 100.0 + 200.0);
        assert!((a.volume(f) - want).abs() < 1e-6, "ruled frustum {} vs {want}", a.volume(f));
        // Smooth through three equal circles: a cylinder.
        let c = a.circle(5.0).unwrap();
        let cyl = a.loft(&[(Some(c), at(0.0)), (Some(c), at(10.0)), (Some(c), at(20.0))], false, false, true, 2).unwrap();
        let want = PI * 25.0 * 20.0;
        assert!((a.volume(cyl) - want).abs() / want < 1e-6, "smooth {} vs {want}", a.volume(cyl));
        // To a point: a pyramid.
        let p = a.loft(&[(Some(big), at(0.0)), (None, at(30.0))], true, false, true, 2).unwrap();
        assert!((a.volume(p) - 4000.0).abs() < 1e-6, "pyramid {}", a.volume(p));
        // Square to circle (a duct): between the two.
        let c10 = a.circle(10.0).unwrap();
        let duct = a.loft(&[(Some(big), at(0.0)), (Some(c10), at(30.0))], false, false, true, 2).unwrap();
        assert!(a.volume(duct) > PI * 100.0 * 30.0 && a.volume(duct) < 400.0 * 30.0);
    }

    #[test]
    fn offsets_thicken_and_defeature() {
        let mut a = Arena::new();
        let (x, y, z, d) = (40.0, 30.0, 20.0, 2.0);
        let b = a.cube(x, y, z, false).unwrap();
        let grown = a.offset_solid(b, d, false).unwrap();
        // Steiner: V + S·d + π(a+b+c)·d² + 4/3·π·d³.
        let want = x * y * z + 2.0 * (x * y + y * z + z * x) * d + PI * (x + y + z) * d * d + 4.0 / 3.0 * PI * d * d * d;
        assert!((a.volume(grown) - want).abs() / want < 1e-6, "round offset {} vs {want}", a.volume(grown));
        let sharp = a.offset_solid(b, d, true).unwrap();
        let want = (x + 2.0 * d) * (y + 2.0 * d) * (z + 2.0 * d);
        assert!((a.volume(sharp) - want).abs() < 1e-6);
        let top = a.face_near(b, [20.0, 15.0, 20.0]).unwrap();
        let cup = a.thicken(b, 2.0, &[top], true).unwrap();
        assert!((a.volume(cup) - (x * y * z - 36.0 * 26.0 * 18.0)).abs() < 1e-6);
        // A plate with a hole and rounded corners, defeatured back to the block.
        let hole = a.cylinder(30.0, 4.0, 4.0, false).unwrap();
        let hole = a.translate(hole, [20.0, 15.0, -5.0]).unwrap();
        let drilled = a.boolean(crate::Bool::Difference, b, hole).unwrap();
        let info = a.surface_info(drilled).unwrap();
        let bore: Vec<usize> = info.iter().enumerate().filter(|(_, f)| f.kind == "cylinder" && f.bulge < 0).map(|(i, _)| i).collect();
        assert!(!bore.is_empty());
        let healed = a.defeature(drilled, &bore).unwrap();
        assert!((a.volume(healed) - x * y * z).abs() < 1e-6, "healed {}", a.volume(healed));
        // A variable fillet lies between the two constant ones.
        let edges = a.edges(b, 0.01).unwrap();
        let e: Vec<usize> = edges.iter().enumerate().filter(|(_, e)| matches!(e.kind, crate::EdgeKind::Line { dir, .. } if dir[2].abs() > 0.99)).map(|(i, _)| i).take(1).collect();
        let v = a.blend_var(b, crate::Blend::Fillet, 1.0, 3.0, &e).unwrap();
        let lo = a.blend(b, crate::Blend::Fillet, 3.0, &e).unwrap();
        let hi = a.blend(b, crate::Blend::Fillet, 1.0, &e).unwrap();
        assert!(a.volume(v) > a.volume(lo) && a.volume(v) < a.volume(hi));
    }

    #[test]
    fn checks_report_and_brep_round_trips() {
        let mut a = Arena::new();
        let b = a.cube(10.0, 10.0, 10.0, false).unwrap();
        let r = a.check_geometry(b).unwrap();
        assert_eq!(r["valid"], true);
        assert_eq!(r["boolean_ready"], true);
        assert_eq!(r["faces"], 6);
        assert_eq!(r["free_edges"], 0);
        let path = std::env::temp_dir().join(format!("ise-occt-brep-{}.brep", std::process::id()));
        a.write_brep(&[b], &path).unwrap();
        let mut r = Arena::new();
        let read = r.read_brep(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(read.len(), 1);
        assert!((r.volume(read[0].1) - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn painted_faces_reach_the_step_file() {
        let mut a = Arena::new();
        let b = a.cube(10.0, 10.0, 10.0, false).unwrap();
        let top = a.face_near(b, [5.0, 5.0, 10.0]).unwrap();
        a.paint(b, None, Some([0.2, 0.4, 0.8]), "").unwrap();
        a.paint(b, Some(&[top]), Some([1.0, 0.0, 0.0]), "Sealing face").unwrap();
        let moved = a.translate(b, [100.0, 0.0, 0.0]).unwrap();
        let path = std::env::temp_dir().join(format!("ise-occt-paint-{}.step", std::process::id()));
        a.write_step(&[("Lid".into(), moved)], &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(text.contains("COLOUR_RGB"), "colours written");
        assert!(text.contains("Sealing face"), "the face's name written");
    }
}
