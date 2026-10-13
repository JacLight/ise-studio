//! Meshes and point clouds as they come from scanners, photogrammetry and
//! other programs: what is wrong with them and mending it, making them
//! lighter or smoother, turning points into surfaces, reading the design
//! intent back out of them (planes, cylinders, spheres, cones, tori and
//! free-form B-spline patches), flattening curved skins, and measuring how
//! far a made part strays from its CAD model.
//!
//! Everything here is plain Rust over plain arrays (millimetres), so the
//! shape programs, the engine's HTTP routes and the tests all share it, and
//! a scan reads the same on every computer. Methods follow the published
//! algorithms named in each module; FreeCAD's Mesh, Points and
//! ReverseEngineering workbenches (LGPL) were the feature reference, no code
//! is copied from them.

pub mod analyze;
pub mod bspline;
pub mod curvature;
pub mod decimate;
pub mod e57;
pub mod fit;
pub mod holes;
pub mod inspect;
pub mod linalg;
pub mod markers;
pub mod mesh;
pub mod photo;
pub mod points;
pub mod reconstruct;
pub mod remesh;
pub mod repair;
pub mod section;
pub mod segment;
pub mod smooth;
pub mod spatial;
pub mod unwrap;

pub use mesh::Mesh;
pub use points::Cloud;

pub type V3 = [f64; 3];
