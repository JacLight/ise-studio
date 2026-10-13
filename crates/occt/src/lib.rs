//! Exact B-rep geometry through OpenCascade.
//!
//! An [`Arena`] holds shapes addressed by [`Shape`] handles, mirroring the
//! shape-program vocabulary (solids, profiles, booleans, transforms, extrude,
//! revolve), so the same program can be evaluated exactly. It also reads and
//! writes STEP and IGES: named, coloured parts out, named, coloured bodies in.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

mod solids;
pub use solids::*;
mod scan;
pub use scan::*;

unsafe extern "C" {
    fn occ_arena_new() -> *mut c_void;
    fn occ_arena_free(a: *mut c_void);
    fn occ_error(a: *mut c_void) -> *const c_char;
    fn occ_box(a: *mut c_void, x: f64, y: f64, z: f64, center: c_int) -> c_int;
    fn occ_cylinder(a: *mut c_void, h: f64, r1: f64, r2: f64, center: c_int) -> c_int;
    fn occ_sphere(a: *mut c_void, r: f64) -> c_int;
    fn occ_bool(a: *mut c_void, op: c_int, x: c_int, y: c_int) -> c_int;
    fn occ_union_all(a: *mut c_void, hs: *const c_int, n: c_int) -> c_int;
    fn occ_transform(a: *mut c_void, h: c_int, kind: c_int, x: f64, y: f64, z: f64) -> c_int;
    fn occ_polygons(a: *mut c_void, xy: *const f64, counts: *const c_int, n: c_int) -> c_int;
    fn occ_circle(a: *mut c_void, r: f64) -> c_int;
    fn occ_sketch_profile(a: *mut c_void, nloops: c_int, d: *const f64, len: c_int) -> c_int;
    fn occ_compound(a: *mut c_void, hs: *const c_int, n: c_int) -> c_int;
    fn occ_section(a: *mut c_void, h: c_int, ox: f64, oy: f64, oz: f64, nx: f64, ny: f64, nz: f64) -> c_int;
    fn occ_profile_loops(a: *mut c_void, h: c_int, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_hlr_view(a: *mut c_void, h: c_int, dx: f64, dy: f64, dz: f64, ux: f64, uy: f64, uz: f64, deflection: f64, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_rect(a: *mut c_void, x: f64, y: f64, center: c_int) -> c_int;
    fn occ_offset(a: *mut c_void, h: c_int, d: f64, join: c_int) -> c_int;
    fn occ_extrude(a: *mut c_void, h: c_int, height: f64, twist: f64, sx: f64, sy: f64, slices: c_int) -> c_int;
    fn occ_revolve(a: *mut c_void, h: c_int, degrees: f64) -> c_int;
    fn occ_from_mesh(a: *mut c_void, pos: *const f64, npos: c_int, tri: *const c_int, ntri: c_int) -> c_int;
    fn occ_volume(a: *mut c_void, h: c_int) -> f64;
    fn occ_area(a: *mut c_void, h: c_int) -> f64;
    fn occ_bounds(a: *mut c_void, h: c_int, out: *mut f64) -> c_int;
    fn occ_distance(a: *mut c_void, x: c_int, y: c_int) -> f64;
    fn occ_mesh(a: *mut c_void, h: c_int, deflection: f64, pos: *mut *const f64, npos: *mut c_int, tri: *mut *const c_int, ntri: *mut c_int) -> c_int;
    fn occ_write_step(a: *mut c_void, hs: *const c_int, names: *const *const c_char, n: c_int, path: *const c_char) -> c_int;
    fn occ_read_step(a: *mut c_void, path: *const c_char, first: *mut c_int) -> c_int;
    fn occ_write_cad(a: *mut c_void, hs: *const c_int, names: *const *const c_char, colors: *const f64, n: c_int, path: *const c_char, iges: c_int) -> c_int;
    fn occ_read_cad(a: *mut c_void, path: *const c_char, iges: c_int, first: *mut c_int) -> c_int;
    fn occ_color(a: *mut c_void, h: c_int, out: *mut f64) -> c_int;
    fn occ_name(a: *mut c_void, h: c_int) -> *const c_char;
    fn occ_edges(a: *mut c_void, h: c_int, deflection: f64, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_blend(a: *mut c_void, h: c_int, kind: c_int, size: f64, idx: *const c_int, n: c_int) -> c_int;
    fn occ_faces(a: *mut c_void, h: c_int, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_face_info(a: *mut c_void, h: c_int, data: *mut *const f64, len: *mut c_int) -> c_int;
    fn occ_push_face(a: *mut c_void, h: c_int, index: c_int, distance: f64) -> c_int;
    fn occ_face_near(a: *mut c_void, h: c_int, x: f64, y: f64, z: f64) -> c_int;
    fn occ_shell(a: *mut c_void, h: c_int, thickness: f64, idx: *const c_int, n: c_int) -> c_int;
    fn occ_draft(a: *mut c_void, h: c_int, idx: *const c_int, n: c_int, degrees: f64, dx: f64, dy: f64, dz: f64, px: f64, py: f64, pz: f64) -> c_int;
}

/// A shape in an [`Arena`] (a solid, or a planar profile on z = 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape(c_int);

impl Shape {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// A handle from its index (e.g. one a script passed back). The arena
    /// checks every handle it is given, so a bad one is an error, not a crash.
    pub fn from_index(i: u32) -> Shape {
        Shape(i.min(i32::MAX as u32) as c_int)
    }
}

pub enum Bool {
    Union,
    Difference,
    Intersection,
}

/// One line of a hidden-line view (view millimetres).
#[derive(Clone, Debug)]
pub struct ViewLine {
    /// Behind something: drawn dashed.
    pub hidden: bool,
    /// The outline of a curved face (a cylinder's sides), not an edge.
    pub outline: bool,
    pub points: Vec<[f64; 2]>,
}

pub enum Join {
    Square,
    Round,
    Miter,
    Bevel,
}

/// What an edge is, for picking it again after the shape changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeKind {
    Line { dir: [f64; 3], length: f64 },
    Circle { center: [f64; 3], axis: [f64; 3], radius: f64 },
    Curve { length: f64 },
    /// A seam or degenerate edge: part of how a surface is closed, not an edge a person sees.
    Seam,
}

/// One edge of a shape: what it is, and a polyline along it.
#[derive(Clone, Debug)]
pub struct Edge {
    pub kind: EdgeKind,
    pub points: Vec<[f64; 3]>,
}

/// One face of a shape: flat ones have an outward normal.
#[derive(Clone, Debug)]
pub struct Face {
    pub normal: Option<[f64; 3]>,
    pub center: [f64; 3],
    pub area: f64,
}

/// What kind of surface a face lies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    Plane,
    Cylinder,
    Cone,
    Torus,
    Sphere,
    Other,
}

/// A face as feature recognition reads it (drawings: holes, diameters, steps).
#[derive(Clone, Debug)]
pub struct FaceInfo {
    pub kind: SurfaceKind,
    /// A plane's centre of area; else a point on the axis (a sphere's centre).
    pub point: [f64; 3],
    /// A plane's outward normal; else the axis.
    pub dir: [f64; 3],
    /// The radius at the low and high end along the axis (equal on a
    /// cylinder; a torus: its major and minor radius).
    pub radius: [f64; 2],
    /// The material is outside the surface (a hole's wall, an inside corner).
    pub concave: bool,
    /// How far along the axis from `point` the face reaches (a plane: its
    /// distance from the origin along its normal, twice).
    pub along: [f64; 2],
    /// The angle it sweeps round its axis (radians; 2π for a whole one).
    pub sweep: f64,
    pub area: f64,
    /// [x0, y0, z0, x1, y1, z1].
    pub bounds: [f64; 6],
}

pub enum Blend {
    /// Rounded, by radius.
    Fillet,
    /// Bevelled, by the distance back along each face.
    Chamfer,
}

/// The exact exchange formats OpenCascade reads and writes here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CadFormat {
    Step,
    Iges,
}

/// A body read from a STEP or IGES file.
#[derive(Clone, Debug)]
pub struct CadBody {
    pub name: String,
    pub shape: Shape,
    /// sRGB, 0…1, when the file colours it.
    pub color: Option<[f64; 3]>,
}

/// A triangulated shape: positions (x, y, z per vertex) and triangle indices.
pub struct Mesh {
    pub positions: Vec<f64>,
    pub triangles: Vec<u32>,
}

/// Exact shapes, alive as long as the arena. Not shared between threads.
pub struct Arena(*mut c_void);

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

// Safe to move between threads (never shared): every use goes through `&mut self` or `&self` on one thread.
unsafe impl Send for Arena {}

impl Drop for Arena {
    fn drop(&mut self) {
        unsafe { occ_arena_free(self.0) }
    }
}

impl Arena {
    pub fn new() -> Arena {
        Arena(unsafe { occ_arena_new() })
    }

    /// A placeholder arena (holds nothing; allocates on first use like any other).
    pub fn empty() -> Arena {
        Self::new()
    }

    fn check(&self, h: c_int) -> Result<Shape, String> {
        if h >= 0 { Ok(Shape(h)) } else { Err(self.error()) }
    }

    fn error(&self) -> String {
        let e = unsafe { CStr::from_ptr(occ_error(self.0)) }.to_string_lossy().into_owned();
        if e.is_empty() { "the exact kernel failed".into() } else { e }
    }

    pub fn cube(&mut self, x: f64, y: f64, z: f64, center: bool) -> Result<Shape, String> {
        self.check(unsafe { occ_box(self.0, x, y, z, center as c_int) })
    }
    pub fn cylinder(&mut self, h: f64, r1: f64, r2: f64, center: bool) -> Result<Shape, String> {
        self.check(unsafe { occ_cylinder(self.0, h, r1, r2, center as c_int) })
    }
    pub fn sphere(&mut self, r: f64) -> Result<Shape, String> {
        self.check(unsafe { occ_sphere(self.0, r) })
    }
    pub fn boolean(&mut self, op: Bool, a: Shape, b: Shape) -> Result<Shape, String> {
        let op = match op {
            Bool::Union => 0,
            Bool::Difference => 1,
            Bool::Intersection => 2,
        };
        self.check(unsafe { occ_bool(self.0, op, a.0, b.0) })
    }
    pub fn union_all(&mut self, shapes: &[Shape]) -> Result<Shape, String> {
        let hs: Vec<c_int> = shapes.iter().map(|s| s.0).collect();
        self.check(unsafe { occ_union_all(self.0, hs.as_ptr(), hs.len() as c_int) })
    }
    pub fn translate(&mut self, s: Shape, v: [f64; 3]) -> Result<Shape, String> {
        self.check(unsafe { occ_transform(self.0, s.0, 0, v[0], v[1], v[2]) })
    }
    /// Degrees about X, then Y, then Z.
    pub fn rotate(&mut self, s: Shape, v: [f64; 3]) -> Result<Shape, String> {
        self.check(unsafe { occ_transform(self.0, s.0, 1, v[0], v[1], v[2]) })
    }
    pub fn scale(&mut self, s: Shape, v: [f64; 3]) -> Result<Shape, String> {
        self.check(unsafe { occ_transform(self.0, s.0, 2, v[0], v[1], v[2]) })
    }
    /// Mirror across the plane through the origin with this normal.
    pub fn mirror(&mut self, s: Shape, n: [f64; 3]) -> Result<Shape, String> {
        self.check(unsafe { occ_transform(self.0, s.0, 3, n[0], n[1], n[2]) })
    }
    /// Closed contours, counter-clockwise adding and clockwise cutting.
    pub fn polygons(&mut self, contours: &[Vec<[f64; 2]>]) -> Result<Shape, String> {
        let xy: Vec<f64> = contours.iter().flatten().flat_map(|p| [p[0], p[1]]).collect();
        let counts: Vec<c_int> = contours.iter().map(|c| c.len() as c_int).collect();
        self.check(unsafe { occ_polygons(self.0, xy.as_ptr(), counts.as_ptr(), counts.len() as c_int) })
    }
    /// A sketch's loops with true arcs (see the C layer for the encoding).
    pub fn sketch_profile(&mut self, loops: usize, data: &[f64]) -> Result<Shape, String> {
        self.check(unsafe { occ_sketch_profile(self.0, loops as c_int, data.as_ptr(), data.len() as c_int) })
    }
    /// A flat profile's loops as segments, in [`Arena::sketch_profile`]'s
    /// format: (number of loops, the flat data).
    pub fn profile_loops(&mut self, s: Shape) -> Result<(usize, Vec<f64>), String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        let n = unsafe { occ_profile_loops(self.0, s.0, &mut data, &mut len) };
        if n < 0 {
            return Err(self.error());
        }
        Ok((n as usize, unsafe { std::slice::from_raw_parts(data, len as usize) }.to_vec()))
    }
    /// A hidden-line view looking along `dir` with `up` up the page:
    /// polylines in view millimetres, each with what it is.
    pub fn hlr_view(&mut self, s: Shape, dir: [f64; 3], up: [f64; 3], deflection: f64) -> Result<Vec<ViewLine>, String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        let n = unsafe { occ_hlr_view(self.0, s.0, dir[0], dir[1], dir[2], up[0], up[1], up[2], deflection, &mut data, &mut len) };
        if n < 0 {
            return Err(self.error());
        }
        let d = unsafe { std::slice::from_raw_parts(data, len as usize) };
        let mut out = Vec::with_capacity(n as usize);
        let mut i = 0;
        while i < d.len() {
            let kind = d[i] as i32;
            let count = d[i + 1] as usize;
            let points = d[i + 2..i + 2 + count * 2].chunks_exact(2).map(|p| [p[0], p[1]]).collect();
            out.push(ViewLine { hidden: kind >= 2, outline: kind % 2 == 1, points });
            i += 2 + count * 2;
        }
        Ok(out)
    }
    /// Where `s` crosses the plane through `origin` square to `normal`: its
    /// edges (read them with [`Arena::edges`]).
    pub fn section(&mut self, s: Shape, origin: [f64; 3], normal: [f64; 3]) -> Result<Shape, String> {
        self.check(unsafe { occ_section(self.0, s.0, origin[0], origin[1], origin[2], normal[0], normal[1], normal[2]) })
    }
    /// Shapes kept together as one, without fusing them.
    pub fn compound(&mut self, shapes: &[Shape]) -> Result<Shape, String> {
        let hs: Vec<c_int> = shapes.iter().map(|s| s.0).collect();
        self.check(unsafe { occ_compound(self.0, hs.as_ptr(), hs.len() as c_int) })
    }
    pub fn circle(&mut self, r: f64) -> Result<Shape, String> {
        self.check(unsafe { occ_circle(self.0, r) })
    }
    pub fn rect(&mut self, x: f64, y: f64, center: bool) -> Result<Shape, String> {
        self.check(unsafe { occ_rect(self.0, x, y, center as c_int) })
    }
    pub fn offset(&mut self, s: Shape, d: f64, join: Join) -> Result<Shape, String> {
        let j = match join {
            Join::Square => 0,
            Join::Round => 1,
            Join::Miter => 2,
            Join::Bevel => 3,
        };
        self.check(unsafe { occ_offset(self.0, s.0, d, j) })
    }
    pub fn extrude(&mut self, s: Shape, height: f64, twist: f64, scale_top: [f64; 2], slices: i32) -> Result<Shape, String> {
        self.check(unsafe { occ_extrude(self.0, s.0, height, twist, scale_top[0], scale_top[1], slices) })
    }
    /// The profile's x is the radius and its y the height along Z.
    pub fn revolve(&mut self, s: Shape, degrees: f64) -> Result<Shape, String> {
        self.check(unsafe { occ_revolve(self.0, s.0, degrees) })
    }
    /// A closed triangle mesh as a faceted solid.
    pub fn from_mesh(&mut self, positions: &[f64], triangles: &[u32]) -> Result<Shape, String> {
        let tri: Vec<c_int> = triangles.iter().map(|t| *t as c_int).collect();
        self.check(unsafe { occ_from_mesh(self.0, positions.as_ptr(), (positions.len() / 3) as c_int, tri.as_ptr(), (tri.len() / 3) as c_int) })
    }
    pub fn volume(&self, s: Shape) -> f64 {
        unsafe { occ_volume(self.0, s.0) }
    }
    pub fn area(&self, s: Shape) -> f64 {
        unsafe { occ_area(self.0, s.0) }
    }
    /// [min x, y, z, max x, y, z].
    pub fn bounds(&self, s: Shape) -> Result<[f64; 6], String> {
        let mut out = [0.0; 6];
        let r = unsafe { occ_bounds(self.0, s.0, out.as_mut_ptr()) };
        if r < 0 { Err(self.error()) } else { Ok(out) }
    }
    pub fn distance(&self, a: Shape, b: Shape) -> Option<f64> {
        let d = unsafe { occ_distance(self.0, a.0, b.0) };
        d.is_finite().then_some(d)
    }
    /// Triangles within `deflection` mm of the true surfaces.
    pub fn mesh(&mut self, s: Shape, deflection: f64) -> Result<Mesh, String> {
        let (mut pos, mut npos, mut tri, mut ntri) = (std::ptr::null(), 0, std::ptr::null(), 0);
        let r = unsafe { occ_mesh(self.0, s.0, deflection, &mut pos, &mut npos, &mut tri, &mut ntri) };
        if r < 0 {
            return Err(self.error());
        }
        let positions = unsafe { std::slice::from_raw_parts(pos, npos as usize * 3) }.to_vec();
        let triangles = unsafe { std::slice::from_raw_parts(tri, ntri as usize * 3) }.iter().map(|t| *t as u32).collect();
        Ok(Mesh { positions, triangles })
    }
    /// Every edge, numbered as [`Arena::blend`] numbers them; polylines within `deflection` mm.
    pub fn edges(&mut self, s: Shape, deflection: f64) -> Result<Vec<Edge>, String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        let n = unsafe { occ_edges(self.0, s.0, deflection, &mut data, &mut len) };
        if n < 0 {
            return Err(self.error());
        }
        let d = unsafe { std::slice::from_raw_parts(data, len as usize) };
        let mut out = Vec::with_capacity(n as usize);
        let mut i = 0;
        while i < d.len() {
            let f = &d[i + 1..i + 8];
            let kind = match d[i] as i32 {
                0 => EdgeKind::Line { dir: [f[0], f[1], f[2]], length: f[6] },
                1 => EdgeKind::Circle { center: [f[0], f[1], f[2]], axis: [f[3], f[4], f[5]], radius: f[6] },
                2 => EdgeKind::Curve { length: f[6] },
                _ => EdgeKind::Seam,
            };
            let count = d[i + 8] as usize;
            let points = d[i + 9..i + 9 + count * 3].chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
            out.push(Edge { kind, points });
            i += 9 + count * 3;
        }
        Ok(out)
    }
    /// Rounds or bevels the edges with these numbers (from [`Arena::edges`]).
    pub fn blend(&mut self, s: Shape, kind: Blend, size: f64, edges: &[usize]) -> Result<Shape, String> {
        let idx: Vec<c_int> = edges.iter().map(|e| *e as c_int).collect();
        let k = match kind {
            Blend::Fillet => 0,
            Blend::Chamfer => 1,
        };
        self.check(unsafe { occ_blend(self.0, s.0, k, size, idx.as_ptr(), idx.len() as c_int) })
    }
    /// Every face, numbered as [`Arena::push_face`] numbers them.
    pub fn faces(&mut self, s: Shape) -> Result<Vec<Face>, String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        let n = unsafe { occ_faces(self.0, s.0, &mut data, &mut len) };
        if n < 0 {
            return Err(self.error());
        }
        let d = unsafe { std::slice::from_raw_parts(data, len as usize) };
        Ok(d.chunks_exact(8).map(|f| Face { normal: (f[0] > 0.5).then_some([f[1], f[2], f[3]]), center: [f[4], f[5], f[6]], area: f[7] }).collect())
    }
    /// Every face's surface, axis and extent, numbered as [`Arena::faces`] numbers them.
    pub fn face_info(&mut self, s: Shape) -> Result<Vec<FaceInfo>, String> {
        let (mut data, mut len) = (std::ptr::null(), 0);
        let n = unsafe { occ_face_info(self.0, s.0, &mut data, &mut len) };
        if n < 0 {
            return Err(self.error());
        }
        let d = unsafe { std::slice::from_raw_parts(data, len as usize) };
        Ok(d.chunks_exact(20)
            .map(|f| FaceInfo {
                kind: match f[0] as i32 {
                    0 => SurfaceKind::Plane,
                    1 => SurfaceKind::Cylinder,
                    2 => SurfaceKind::Cone,
                    3 => SurfaceKind::Torus,
                    4 => SurfaceKind::Sphere,
                    _ => SurfaceKind::Other,
                },
                point: [f[1], f[2], f[3]],
                dir: [f[4], f[5], f[6]],
                radius: [f[7], f[8]],
                concave: f[9] > 0.5,
                along: [f[10], f[11]],
                sweep: f[12],
                area: f[13],
                bounds: [f[14], f[15], f[16], f[17], f[18], f[19]],
            })
            .collect())
    }
    /// The face nearest a point, numbered as [`Arena::faces`] numbers them.
    pub fn face_near(&mut self, s: Shape, p: [f64; 3]) -> Result<usize, String> {
        let i = unsafe { occ_face_near(self.0, s.0, p[0], p[1], p[2]) };
        if i < 0 { Err(self.error()) } else { Ok(i as usize) }
    }
    /// Pushes (positive, adds material) or pulls (negative, cuts) a face along
    /// its outward normal: flat faces by sweeping, curved ones by offsetting.
    pub fn push_face(&mut self, s: Shape, face: usize, distance: f64) -> Result<Shape, String> {
        self.check(unsafe { occ_push_face(self.0, s.0, face as c_int, distance) })
    }
    /// Hollows a solid to walls `thickness` thick, opening the listed faces
    /// (numbered as [`Arena::faces`] numbers them). Positive thickness grows the
    /// walls inward (the outside stays put); negative grows them outward (the
    /// original shape becomes the cavity). No open faces: a closed hollow.
    pub fn shell(&mut self, s: Shape, thickness: f64, open_faces: &[usize]) -> Result<Shape, String> {
        let idx: Vec<c_int> = open_faces.iter().map(|e| *e as c_int).collect();
        self.check(unsafe { occ_shell(self.0, s.0, thickness, idx.as_ptr(), idx.len() as c_int) })
    }
    /// Tapers faces by `degrees` relative to the pull direction, hinging about
    /// the plane through `neutral` normal to the pull: positive angles lean the
    /// faces inward going along the pull (a mould draft).
    pub fn draft(&mut self, s: Shape, faces: &[usize], degrees: f64, pull: [f64; 3], neutral: [f64; 3]) -> Result<Shape, String> {
        let idx: Vec<c_int> = faces.iter().map(|e| *e as c_int).collect();
        self.check(unsafe {
            occ_draft(self.0, s.0, idx.as_ptr(), idx.len() as c_int, degrees, pull[0], pull[1], pull[2], neutral[0], neutral[1], neutral[2])
        })
    }
    /// Writes named shapes to a STEP file (AP214, millimetres).
    pub fn write_step(&mut self, parts: &[(String, Shape)], path: &std::path::Path) -> Result<(), String> {
        let names: Vec<CString> = parts.iter().map(|(n, _)| CString::new(n.replace('\0', "")).unwrap()).collect();
        let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        let hs: Vec<c_int> = parts.iter().map(|(_, s)| s.0).collect();
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let r = unsafe { occ_write_step(self.0, hs.as_ptr(), ptrs.as_ptr(), hs.len() as c_int, p.as_ptr()) };
        if r < 0 { Err(self.error()) } else { Ok(()) }
    }
    /// Writes named shapes, each with an optional colour (sRGB, 0…1), to a
    /// STEP (AP214) or IGES file in millimetres.
    pub fn write_cad(&mut self, parts: &[(String, Shape, Option<[f64; 3]>)], path: &std::path::Path, format: CadFormat) -> Result<(), String> {
        let names: Vec<CString> = parts.iter().map(|(n, _, _)| CString::new(n.replace('\0', "")).unwrap()).collect();
        let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        let hs: Vec<c_int> = parts.iter().map(|(_, s, _)| s.0).collect();
        let colors: Vec<f64> = parts.iter().flat_map(|(_, _, c)| c.map(|c| c.map(|v| v.clamp(0.0, 1.0))).unwrap_or([-1.0; 3])).collect();
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let iges = matches!(format, CadFormat::Iges) as c_int;
        let r = unsafe { occ_write_cad(self.0, hs.as_ptr(), ptrs.as_ptr(), colors.as_ptr(), hs.len() as c_int, p.as_ptr(), iges) };
        if r < 0 { Err(self.error()) } else { Ok(()) }
    }
    /// Reads a STEP or IGES file: every body, named (assembly paths joined
    /// with " / ") and coloured as the file has it. Surfaces that close are
    /// sewn into solids (IGES files usually carry faces, not solids).
    pub fn read_cad(&mut self, path: &std::path::Path, format: CadFormat) -> Result<Vec<CadBody>, String> {
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mut first = 0;
        let n = unsafe { occ_read_cad(self.0, p.as_ptr(), matches!(format, CadFormat::Iges) as c_int, &mut first) };
        if n < 0 {
            return Err(self.error());
        }
        Ok((first..first + n)
            .map(|h| {
                let mut c = [0.0; 3];
                let color = (unsafe { occ_color(self.0, h, c.as_mut_ptr()) } == 1).then_some(c);
                CadBody { name: unsafe { CStr::from_ptr(occ_name(self.0, h)) }.to_string_lossy().into_owned(), shape: Shape(h), color }
            })
            .collect())
    }
    /// Reads a STEP file: every body, named (assembly paths joined with " / ").
    pub fn read_step(&mut self, path: &std::path::Path) -> Result<Vec<(String, Shape)>, String> {
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mut first = 0;
        let n = unsafe { occ_read_step(self.0, p.as_ptr(), &mut first) };
        if n < 0 {
            return Err(self.error());
        }
        Ok((first..first + n)
            .map(|h| (unsafe { CStr::from_ptr(occ_name(self.0, h)) }.to_string_lossy().into_owned(), Shape(h)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_solids_measure_exactly() {
        let mut a = Arena::new();
        let c = a.cylinder(10.0, 5.0, 5.0, false).unwrap();
        let exact = std::f64::consts::PI * 25.0 * 10.0;
        assert!((a.volume(c) - exact).abs() < 1e-6, "a true cylinder, not facets: {}", a.volume(c));
        let b = a.cube(20.0, 20.0, 5.0, false).unwrap();
        let hole = a.translate(c, [10.0, 10.0, -2.0]).unwrap();
        let plate = a.boolean(Bool::Difference, b, hole).unwrap();
        assert!((a.volume(plate) - (2000.0 - std::f64::consts::PI * 25.0 * 5.0)).abs() < 1e-6);
    }

    #[test]
    fn profiles_extrude_and_revolve() {
        let mut a = Arena::new();
        // A square with a square hole (clockwise), extruded.
        let p = a.polygons(&[vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], vec![[3.0, 3.0], [3.0, 7.0], [7.0, 7.0], [7.0, 3.0]]]).unwrap();
        assert!((a.area(p) - 84.0).abs() < 1e-9);
        let s = a.extrude(p, 2.0, 0.0, [1.0, 1.0], 0).unwrap();
        assert!((a.volume(s) - 168.0).abs() < 1e-9);
        // A rounded offset grows the outline by exact quarter circles.
        let r = a.rect(10.0, 10.0, false).unwrap();
        let grown = a.offset(r, 1.0, Join::Round).unwrap();
        assert!((a.area(grown) - (100.0 + 40.0 + std::f64::consts::PI)).abs() < 1e-6, "{}", a.area(grown));
        // A ring revolved: a true torus-like washer.
        let ring = a.rect(2.0, 1.0, false).unwrap();
        let ring = a.translate(ring, [3.0, 0.0, 0.0]).unwrap();
        let washer = a.revolve(ring, 360.0).unwrap();
        let exact = std::f64::consts::PI * (25.0 - 9.0) * 1.0;
        assert!((a.volume(washer) - exact).abs() < 1e-6, "{}", a.volume(washer));
    }

    #[test]
    fn fillets_and_chamfers_are_exact() {
        let mut a = Arena::new();
        let b = a.cube(20.0, 20.0, 10.0, false).unwrap();
        let edges = a.edges(b, 0.01).unwrap();
        assert_eq!(edges.len(), 12);
        // The four vertical edges, rounded r = 2: each corner loses (4 − π)·r²/… of a square.
        let vertical: Vec<usize> = edges.iter().enumerate().filter(|(_, e)| matches!(e.kind, EdgeKind::Line { dir, .. } if dir[2].abs() > 0.99)).map(|(i, _)| i).collect();
        assert_eq!(vertical.len(), 4);
        let r = a.blend(b, Blend::Fillet, 2.0, &vertical).unwrap();
        let exact = 4000.0 - 4.0 * (4.0 - std::f64::consts::PI) * 10.0;
        assert!((a.volume(r) - exact).abs() < 1e-6, "{}", a.volume(r));
        let arcs = a.edges(r, 0.01).unwrap().iter().filter(|e| matches!(e.kind, EdgeKind::Circle { radius, .. } if (radius - 2.0).abs() < 1e-9)).count();
        assert_eq!(arcs, 8, "true arcs where the fillets meet the top and bottom");
        // A 1 mm chamfer on the same four edges cuts four right triangles of ½ mm².
        let c = a.blend(b, Blend::Chamfer, 1.0, &vertical).unwrap();
        assert!((a.volume(c) - (4000.0 - 4.0 * 0.5 * 10.0)).abs() < 1e-6, "{}", a.volume(c));
        // Too large a radius is an error, not a broken solid.
        assert!(a.blend(b, Blend::Fillet, 15.0, &vertical).is_err());
        // A cylinder's seam is not an edge anyone sees.
        let cyl = a.cylinder(10.0, 5.0, 5.0, false).unwrap();
        let e = a.edges(cyl, 0.01).unwrap();
        assert_eq!(e.iter().filter(|e| matches!(e.kind, EdgeKind::Circle { .. })).count(), 2);
        assert!(e.iter().any(|e| e.kind == EdgeKind::Seam));
    }

    #[test]
    fn bezier_profiles_are_exact() {
        // A closed shape: the line (0,0)→(100,0), then a Bézier back over the top.
        let mut a = Arena::new();
        let data = [2.0, 0.0, 0.0, 0.0, 100.0, 0.0, 3.0, 100.0, 0.0, 100.0, 60.0, 0.0, 60.0, 0.0, 0.0];
        let p = a.sketch_profile(1, &data).unwrap();
        // The area between a cubic Bézier and its chord, by fine numerical integration.
        let b = |t: f64| {
            let u = 1.0 - t;
            let (c0, c1, c2, c3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (c0 * 100.0 + c1 * 100.0, c1 * 60.0 + c2 * 60.0)
        };
        let n = 200_000;
        let mut area = 0.0;
        let mut prev = b(0.0);
        for i in 1..=n {
            let q = b(i as f64 / n as f64);
            area += prev.0 * q.1 - q.0 * prev.1;
            prev = q;
        }
        let area = area.abs() / 2.0;
        assert!((a.area(p) - area).abs() < 1e-3, "exact {} vs {area}", a.area(p));
        let s = a.extrude(p, 10.0, 0.0, [1.0, 1.0], 0).unwrap();
        assert!((a.volume(s) - area * 10.0).abs() < 1e-2);
    }

    #[test]
    fn faces_push_and_pull() {
        let mut a = Arena::new();
        let b = a.cube(20.0, 10.0, 5.0, false).unwrap();
        let faces = a.faces(b).unwrap();
        assert_eq!(faces.len(), 6);
        let top = faces.iter().position(|f| f.normal.is_some_and(|n| n[2] > 0.99)).unwrap();
        assert!((faces[top].center[2] - 5.0).abs() < 1e-9 && (faces[top].area - 200.0).abs() < 1e-9);
        let taller = a.push_face(b, top, 3.0).unwrap();
        assert!((a.volume(taller) - 1600.0).abs() < 1e-6, "{}", a.volume(taller));
        assert_eq!(a.faces(taller).unwrap().len(), 6, "the pushed face merges into one clean box");
        let lower = a.push_face(b, top, -2.0).unwrap();
        assert!((a.volume(lower) - 600.0).abs() < 1e-6);
        let cyl = a.cylinder(10.0, 3.0, 3.0, false).unwrap();
        let side = a.faces(cyl).unwrap().iter().position(|f| f.normal.is_none()).unwrap();
        // A cylinder's side (r 3, h 10), pushed out 2 mm: the radius grows to 5, the height stays.
        let fat = a.push_face(cyl, side, 2.0).unwrap();
        let want = std::f64::consts::PI * 5.0 * 5.0 * 10.0;
        assert!((a.volume(fat) - want).abs() < want * 1e-4, "volume {} vs {want}", a.volume(fat));
        let thin = a.push_face(cyl, side, -1.0).unwrap();
        let want = std::f64::consts::PI * 2.0 * 2.0 * 10.0;
        assert!((a.volume(thin) - want).abs() < want * 1e-4, "volume {} vs {want}", a.volume(thin));
        assert_eq!(a.face_near(cyl, [3.0, 0.0, 5.0]).unwrap(), side, "the side is the face nearest a point on it");
    }

    #[test]
    fn shells_hollow_exactly() {
        let mut a = Arena::new();
        let b = a.cube(40.0, 40.0, 20.0, false).unwrap();
        let top = a.face_near(b, [20.0, 20.0, 20.0]).unwrap();
        // Open at the top, 2 mm walls inward: the outside stays 40×40×20.
        let cup = a.shell(b, 2.0, &[top]).unwrap();
        let want = 40.0 * 40.0 * 20.0 - 36.0 * 36.0 * 18.0;
        assert!((a.volume(cup) - want).abs() < 1e-6, "open shell {} vs {want}", a.volume(cup));
        let bb = a.bounds(cup).unwrap();
        assert!(bb.iter().zip([0.0, 0.0, 0.0, 40.0, 40.0, 20.0]).all(|(a, b)| (a - b).abs() < 1e-6), "{bb:?}");
        assert_eq!(a.faces(cup).unwrap().len(), 11, "five outer, five inner, the rim");
        // Closed: a sealed 36×36×16 void inside.
        let sealed = a.shell(b, 2.0, &[]).unwrap();
        let want = 40.0 * 40.0 * 20.0 - 36.0 * 36.0 * 16.0;
        assert!((a.volume(sealed) - want).abs() < 1e-6, "closed shell {} vs {want}", a.volume(sealed));
        // Outward: the box becomes the cavity of a 44×44×22 cup.
        let out = a.shell(b, -2.0, &[top]).unwrap();
        let want = 44.0 * 44.0 * 22.0 - 40.0 * 40.0 * 20.0;
        assert!((a.volume(out) - want).abs() < 1e-6, "outward shell {} vs {want}", a.volume(out));
        // Two open faces (top and one side) on the same box.
        let side = a.face_near(b, [20.0, 0.0, 10.0]).unwrap();
        let channel = a.shell(b, 2.0, &[top, side]).unwrap();
        let want = 40.0 * 40.0 * 20.0 - 36.0 * 38.0 * 18.0;
        assert!((a.volume(channel) - want).abs() < 1e-6, "two open faces {} vs {want}", a.volume(channel));
        // A cylinder cup: exact circles.
        let cyl = a.cylinder(10.0, 5.0, 5.0, false).unwrap();
        let lid = a.face_near(cyl, [0.0, 0.0, 10.0]).unwrap();
        let mug = a.shell(cyl, 1.0, &[lid]).unwrap();
        let want = std::f64::consts::PI * (25.0 * 10.0 - 16.0 * 9.0);
        assert!((a.volume(mug) - want).abs() < want * 1e-6, "cylinder cup {} vs {want}", a.volume(mug));
        // Walls thicker than the part: an error, not a broken solid.
        assert!(a.shell(b, 25.0, &[top]).is_err());
    }

    #[test]
    fn drafts_taper_exactly() {
        let mut a = Arena::new();
        let b = a.cube(40.0, 40.0, 20.0, false).unwrap();
        let sides: Vec<usize> = a.faces(b).unwrap().iter().enumerate().filter(|(_, f)| f.normal.is_some_and(|n| n[2].abs() < 1e-9)).map(|(i, _)| i).collect();
        assert_eq!(sides.len(), 4);
        // 5° about the bottom, pulled up: a frustum whose top shrinks by 2·20·tan 5° each way.
        let t = 5f64.to_radians().tan();
        let d = a.draft(b, &sides, 5.0, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]).unwrap();
        let top = 40.0 - 2.0 * 20.0 * t;
        let faces = a.faces(d).unwrap();
        let lid = faces.iter().find(|f| f.normal.is_some_and(|n| n[2] > 0.99)).unwrap();
        assert!((lid.area - top * top).abs() < 1e-6, "top {} vs {}", lid.area, top * top);
        let base = faces.iter().find(|f| f.normal.is_some_and(|n| n[2] < -0.99)).unwrap();
        assert!((base.area - 1600.0).abs() < 1e-6, "the neutral plane keeps its size");
        let frustum = 20.0 / 3.0 * (1600.0 + top * top + 40.0 * top);
        assert!((a.volume(d) - frustum).abs() < 1e-6, "volume {} vs {frustum}", a.volume(d));
        // About the middle: the top shrinks, the bottom grows, by 10·tan 5° each side.
        let mid = a.draft(b, &sides, 5.0, [0.0, 0.0, 1.0], [0.0, 0.0, 10.0]).unwrap();
        let (top, bot) = (40.0 - 20.0 * t, 40.0 + 20.0 * t);
        let frustum = 20.0 / 3.0 * (top * top + bot * bot + top * bot);
        assert!((a.volume(mid) - frustum).abs() < 1e-6, "volume {} vs {frustum}", a.volume(mid));
        // No faces named: every wall along the pull, the same four.
        let auto = a.draft(b, &[], 5.0, [0.0, 0.0, 1.0], [0.0, 0.0, 10.0]).unwrap();
        assert!((a.volume(auto) - frustum).abs() < 1e-6);
        // A cylinder's side, drafted 5° from its base: an exact cone frustum.
        let cyl = a.cylinder(20.0, 10.0, 10.0, false).unwrap();
        let cone = a.draft(cyl, &[], 5.0, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]).unwrap();
        let r = 10.0 - 20.0 * t;
        let want = std::f64::consts::PI * 20.0 / 3.0 * (100.0 + 10.0 * r + r * r);
        assert!((a.volume(cone) - want).abs() < want * 1e-9, "cone {} vs {want}", a.volume(cone));
        // Negative leans outward: the top grows.
        let flare = a.draft(b, &sides, -5.0, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]).unwrap();
        let top = 40.0 + 2.0 * 20.0 * t;
        let frustum = 20.0 / 3.0 * (1600.0 + top * top + 40.0 * top);
        assert!((a.volume(flare) - frustum).abs() < 1e-6, "flare {} vs {frustum}", a.volume(flare));
        // Rounded corners taper with the walls; then shelled, the cavity follows.
        let edges = a.edges(b, 0.01).unwrap();
        let vertical: Vec<usize> = edges.iter().enumerate().filter(|(_, e)| matches!(e.kind, EdgeKind::Line { dir, .. } if dir[2].abs() > 0.99)).map(|(i, _)| i).collect();
        let rounded = a.blend(b, Blend::Fillet, 3.0, &vertical).unwrap();
        let drafted = a.draft(rounded, &[], 3.0, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]).unwrap();
        assert!(a.volume(drafted) < a.volume(rounded));
        let lid = a.face_near(drafted, [20.0, 20.0, 20.0]).unwrap();
        let open = a.shell(drafted, 2.0, &[lid]).unwrap();
        let sealed = a.shell(drafted, 2.0, &[]).unwrap();
        assert!(a.volume(open) < a.volume(sealed) && a.volume(sealed) < a.volume(drafted), "a drafted, rounded part hollows: {} < {} < {}", a.volume(open), a.volume(sealed), a.volume(drafted));
        // At 5° the 3 mm rounds shrink to 1.25 mm at the top: a 2 mm wall cannot follow them inward, and says so.
        let steep = a.draft(rounded, &[], 5.0, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]).unwrap();
        let lid = a.face_near(steep, [20.0, 20.0, 20.0]).unwrap();
        let e = a.shell(steep, 2.0, &[lid]).unwrap_err();
        assert!(e.contains("rounded corner") && e.contains("1.25"), "{e}");
        let thin = a.shell(steep, 1.0, &[lid]).unwrap();
        assert!(a.volume(thin) > 0.0 && a.volume(thin) < a.volume(steep), "a 1 mm wall fits");
        let tight = a.blend(b, Blend::Fillet, 1.0, &vertical).unwrap();
        let lid = a.face_near(tight, [20.0, 20.0, 20.0]).unwrap();
        let e = a.shell(tight, 2.0, &[lid]).unwrap_err();
        assert!(e.contains("(1 mm radius)"), "{e}");
        // A drafted box then shelled, by hand: the cavity's walls sit 2/cos 3° in, its floor 2 up.
        let t3 = 3f64.to_radians().tan();
        let db = a.draft(b, &[], 3.0, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]).unwrap();
        let lid = a.face_near(db, [20.0, 20.0, 20.0]).unwrap();
        let tray = a.shell(db, 2.0, &[lid]).unwrap();
        let top = 40.0 - 40.0 * t3;
        let outer = 20.0 / 3.0 * (1600.0 + top * top + 40.0 * top);
        let ib = 40.0 - 4.0 * t3 - 4.0 / 3f64.to_radians().cos();
        let it = ib - 36.0 * t3;
        let want = outer - 18.0 / 3.0 * (ib * ib + it * it + ib * it);
        assert!((a.volume(tray) - want).abs() < 1e-6, "drafted tray {} vs {want}", a.volume(tray));
        // A face parallel to the neutral plane cannot be drafted.
        let lid = a.face_near(b, [20.0, 20.0, 20.0]).unwrap();
        assert!(a.draft(b, &[lid], 5.0, [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]).is_err());
    }

    #[test]
    fn step_round_trip_keeps_names_and_geometry() {
        let mut a = Arena::new();
        let b = a.cube(30.0, 20.0, 10.0, false).unwrap();
        let c = a.cylinder(15.0, 4.0, 4.0, false).unwrap();
        let path = std::env::temp_dir().join(format!("forma-occt-test-{}.step", std::process::id()));
        a.write_step(&[("Base plate".into(), b), ("Pin".into(), c)], &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("PRODUCT('Base plate'") && text.contains("PRODUCT('Pin'"));
        assert!(!text.contains("PRODUCT('SOLID'") && !text.contains("PRODUCT('COMPOUND'"), "no auto-named entries");
        assert!(text.contains("CYLINDRICAL_SURFACE"), "the pin is a true cylinder");
        let mut r = Arena::new();
        let read = r.read_step(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let names: Vec<&str> = read.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Base plate", "Pin"]);
        assert!((r.volume(read[0].1) - 6000.0).abs() < 1e-6);
        assert!((r.volume(read[1].1) - std::f64::consts::PI * 16.0 * 15.0).abs() < 1e-6);
        let m = r.mesh(read[1].1, 0.01).unwrap();
        assert!(m.triangles.len() > 30);
    }

    #[test]
    fn step_and_iges_round_trip_keep_names_colours_and_volume() {
        for format in [CadFormat::Step, CadFormat::Iges] {
            let mut a = Arena::new();
            let b = a.cube(30.0, 20.0, 10.0, false).unwrap();
            let c = a.cylinder(15.0, 4.0, 4.0, false).unwrap();
            let c = a.translate(c, [50.0, 0.0, 0.0]).unwrap();
            let red = [0.8, 0.2, 0.1];
            let path = std::env::temp_dir().join(format!("forma-occt-cad-{}-{format:?}", std::process::id()));
            a.write_cad(&[("Base plate".into(), b, Some(red)), ("Pin".into(), c, None)], &path, format).unwrap();
            let mut r = Arena::new();
            let read = r.read_cad(&path, format).unwrap();
            let _ = std::fs::remove_file(&path);
            let names: Vec<&str> = read.iter().map(|b| b.name.as_str()).collect();
            assert_eq!(read.len(), 2, "{format:?}: {names:?}");
            let plate = read.iter().find(|b| b.name == "Base plate").unwrap_or_else(|| panic!("{format:?}: names {names:?}"));
            let pin = read.iter().find(|b| b.name == "Pin").unwrap_or_else(|| panic!("{format:?}: names {names:?}"));
            assert!((r.volume(plate.shape) - 6000.0).abs() < 1e-3, "{format:?} plate {}", r.volume(plate.shape));
            assert!((r.volume(pin.shape) - std::f64::consts::PI * 16.0 * 15.0).abs() < 1e-3, "{format:?} pin {}", r.volume(pin.shape));
            let got = plate.color.unwrap_or_else(|| panic!("{format:?}: the plate's colour"));
            assert!(got.iter().zip(red).all(|(g, w)| (g - w).abs() < 0.01), "{format:?} colour {got:?}");
            assert!(pin.color.is_none(), "{format:?}: an uncoloured part stays uncoloured");
            let bounds = r.bounds(pin.shape).unwrap();
            assert!((bounds[0] - 46.0).abs() < 1e-3, "{format:?}: placed where it was");
        }
    }
}

