//! Exact geometry from scans (the C side is `scan.cpp`): B-spline faces from
//! fitted control nets, flat faces merged on solids sewn from meshes, and
//! tessellation with angular control.

use super::{Arena, Mesh, Shape};
use std::ffi::{c_int, c_void};

unsafe extern "C" {
    fn occ_bspline_face(a: *mut c_void, poles: *const f64, nu: c_int, nv: c_int, uk: *const f64, um: *const c_int, nuk: c_int, vk: *const f64, vm: *const c_int, nvk: c_int, udeg: c_int, vdeg: c_int) -> c_int;
    fn occ_unify(a: *mut c_void, h: c_int) -> c_int;
    fn occ_mesh_angular(a: *mut c_void, h: c_int, deflection: f64, angle: f64, positions: *mut *const f64, npos: *mut c_int, triangles: *mut *const c_int, ntri: *mut c_int) -> c_int;
}

/// A B-spline surface: poles (nu rows of nv), distinct knots with their multiplicities, degrees.
pub struct BSplineSurface<'a> {
    pub poles: &'a [[f64; 3]],
    pub nu: usize,
    pub nv: usize,
    pub uknots: &'a [f64],
    pub umults: &'a [u32],
    pub vknots: &'a [f64],
    pub vmults: &'a [u32],
    pub udegree: usize,
    pub vdegree: usize,
}

impl Arena {
    /// A face on an exact B-spline surface.
    pub fn bspline_face(&mut self, s: &BSplineSurface) -> Result<Shape, String> {
        if s.poles.len() != s.nu * s.nv {
            return Err("the B-spline's poles do not match its size".into());
        }
        let poles: Vec<f64> = s.poles.iter().flatten().copied().collect();
        let um: Vec<c_int> = s.umults.iter().map(|m| *m as c_int).collect();
        let vm: Vec<c_int> = s.vmults.iter().map(|m| *m as c_int).collect();
        self.check(unsafe {
            occ_bspline_face(
                self.0,
                poles.as_ptr(),
                s.nu as c_int,
                s.nv as c_int,
                s.uknots.as_ptr(),
                um.as_ptr(),
                s.uknots.len() as c_int,
                s.vknots.as_ptr(),
                vm.as_ptr(),
                s.vknots.len() as c_int,
                s.udegree as c_int,
                s.vdegree as c_int,
            )
        })
    }

    /// Faces lying on one surface merged into one (a mesh-made solid's flat
    /// triangles become true planar faces).
    pub fn unify(&mut self, s: Shape) -> Result<Shape, String> {
        self.check(unsafe { occ_unify(self.0, s.0) })
    }

    /// Triangles within `deflection` mm and `angle` radians of the true surfaces.
    pub fn mesh_angular(&mut self, s: Shape, deflection: f64, angle: f64) -> Result<Mesh, String> {
        let (mut pos, mut npos, mut tri, mut ntri) = (std::ptr::null(), 0, std::ptr::null(), 0);
        let r = unsafe { occ_mesh_angular(self.0, s.0, deflection, angle, &mut pos, &mut npos, &mut tri, &mut ntri) };
        if r < 0 {
            return Err(self.error());
        }
        let positions = unsafe { std::slice::from_raw_parts(pos, npos as usize * 3) }.to_vec();
        let triangles = unsafe { std::slice::from_raw_parts(tri, ntri as usize * 3) }.iter().map(|t| *t as u32).collect();
        Ok(Mesh { positions, triangles })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meshed_box_becomes_six_planar_faces_and_a_bspline_patch_is_a_face() {
        let mut a = Arena::new();
        let b = a.cube(10.0, 20.0, 30.0, false).unwrap();
        let m = a.mesh(b, 0.01).unwrap();
        let sewn = a.from_mesh(&m.positions, &m.triangles).unwrap();
        let faces_before = a.faces(sewn).unwrap().len();
        let flat = a.unify(sewn).unwrap();
        assert_eq!(a.faces(flat).unwrap().len(), 6, "{faces_before} triangles became 6 faces");
        assert!((a.volume(flat) - 6000.0).abs() < 1e-6);
        // A bilinear patch as a degree-1 B-spline.
        let poles = [[0.0, 0.0, 0.0], [0.0, 10.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 5.0]];
        let f = a
            .bspline_face(&BSplineSurface { poles: &poles, nu: 2, nv: 2, uknots: &[0.0, 1.0], umults: &[2, 2], vknots: &[0.0, 1.0], vmults: &[2, 2], udegree: 1, vdegree: 1 })
            .unwrap();
        assert!(a.area(f) > 100.0 && a.area(f) < 120.0, "{}", a.area(f));
        // Fine angles mesh a cylinder with more triangles.
        let c = a.cylinder(10.0, 5.0, 5.0, false).unwrap();
        let coarse = a.mesh_angular(c, 0.5, 0.8).unwrap().triangles.len();
        let fine = a.mesh_angular(c, 0.5, 0.05).unwrap().triangles.len();
        assert!(fine > coarse * 2, "{fine} vs {coarse}");
    }
}
