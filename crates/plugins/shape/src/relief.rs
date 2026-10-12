//! Reliefs from pictures: brightness becomes height.
//!
//! A picture is sampled onto a (2^k + 1)-point square grid stretched over the
//! relief's width and height, and meshed as a right-triangulated irregular
//! network: the grid is split recursively into right triangles, each split
//! only where the flat triangle would stray more than the allowed error from
//! the picture's height there. Flat areas become a few large triangles,
//! detail gets as many as it needs, and every split is shared across the
//! diamond it sits in, so the surface never cracks. The result is one closed
//! solid: the height surface, four walls and a flat side, watertight and
//! manifold, ready for booleans, printing and carving.

use crate::image::{Channel, Field, Picture};
use manifold_csg::Manifold;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A plaque: flat back at z = 0, front at `base + depth × value` (light is high).
    Emboss,
    /// A cutter to subtract from a face at z = 0: it reaches down
    /// `depth × (1 − value)` (dark is deep) and up to `clear` above the face.
    Engrave,
    /// Light through plastic: flat back at z = 0, thickness
    /// `base + depth × (1 − value)` (dark is thick, light is thin).
    Lithophane,
}

/// What to build, in millimeters.
#[derive(Clone, Debug)]
pub struct Spec {
    pub width: f64,
    pub height: f64,
    pub depth: f64,
    pub base: f64,
    pub mode: Mode,
    pub invert: bool,
    pub gamma: f64,
    /// Gaussian blur, mm.
    pub smooth: f64,
    pub channel: Channel,
    /// Engrave: how far the cutter reaches above the face.
    pub clear: f64,
    /// Largest height error allowed, mm.
    pub tolerance: f64,
    /// At most this many triangles (the error grows to fit).
    pub budget: usize,
    /// Largest grid, points per side minus one (a power of two).
    pub detail: usize,
}

/// What came out, for reporting.
#[derive(Clone, Debug)]
pub struct Built {
    pub solid: Manifold,
    pub grid: usize,
    pub triangles: usize,
    /// The height error the mesh keeps to, mm.
    pub error: f64,
}

/// The picture's values on the relief grid, after channel, blur, gamma and invert.
pub fn values(picture: &Picture, spec: &Spec, n: usize) -> Field {
    let mut f = picture.field(spec.channel);
    if spec.smooth > 0.0 {
        // mm to pixels: the picture spans the relief's width.
        f.blur(spec.smooth * picture.width as f64 / spec.width);
    }
    let mut g = f.resample(n, n);
    for v in &mut g.v {
        let mut x = v.clamp(0.0, 1.0);
        if spec.gamma != 1.0 {
            x = x.powf(spec.gamma as f32);
        }
        *v = if spec.invert { 1.0 - x } else { x };
    }
    g
}

/// Builds the relief solid.
pub fn build(picture: &Picture, spec: &Spec) -> Result<Built, String> {
    for (what, v) in [("width", spec.width), ("height", spec.height)] {
        if !(v.is_finite() && v > 0.0) {
            return Err(format!("relief: {what} must be a positive number of mm"));
        }
    }
    if !(spec.depth.is_finite() && spec.depth > 0.0) {
        return Err("relief: depth must be a positive number of mm".into());
    }
    if !(spec.base.is_finite() && spec.base >= 0.0) {
        return Err("relief: base must be a number of mm, 0 or more".into());
    }
    if spec.mode != Mode::Engrave && spec.base <= 0.0 {
        return Err("relief: base must be more than 0 (sink it into the face you add it to)".into());
    }
    // Grid: fine enough for every pixel, no finer than the tolerance calls for, at most `detail`.
    let pixels = picture.width.max(picture.height).max(2);
    let by_tolerance = (spec.width.max(spec.height) / spec.tolerance.max(1e-6)).max(2.0) as usize;
    let cells = pixels.min(by_tolerance).min(spec.detail.max(16)).next_power_of_two().min(spec.detail.max(16).next_power_of_two());
    let n = cells + 1;
    let v = values(picture, spec, n);
    // Heights above the flat side (always positive), and where that side sits.
    let (heights, flat): (Vec<f32>, f64) = match spec.mode {
        Mode::Emboss => (v.v.iter().map(|x| (spec.base + spec.depth * *x as f64) as f32).collect(), 0.0),
        Mode::Lithophane => (v.v.iter().map(|x| (spec.base + spec.depth * (1.0 - *x as f64)) as f32).collect(), 0.0),
        // Built upside down (cut depth up from −clear) and mirrored after.
        Mode::Engrave => (v.v.iter().map(|x| (spec.base + spec.depth * (1.0 - *x as f64)) as f32).collect(), -spec.clear.max(0.01)),
    };
    let levels = if picture.bits == 16 { 65535.0 } else { 255.0 };
    // Half a grey level is below what the picture can say.
    let floor = spec.tolerance.max(spec.depth / (2.0 * levels));
    let errors = rtin_errors(&heights, n);
    let mut error = floor;
    let mut count = rtin_count(&errors, n, error as f32);
    while count > spec.budget {
        error *= 1.5;
        count = rtin_count(&errors, n, error as f32);
    }
    let tris = rtin_triangles(&errors, n, error as f32);
    let solid = close(&heights, n, &tris, spec.width, spec.height, flat, spec.mode == Mode::Engrave)?;
    let triangles = solid.num_tri();
    Ok(Built { solid, grid: n, triangles, error })
}

/// Grid coordinates of every triangle's corners: (ax, ay, bx, by, cx, cy), the
/// hypotenuse a–b and the right angle at c. Triangle i is the (i + 2)th node
/// of the binary split tree, read bit by bit from the root.
fn corners(i: usize, size: usize) -> [usize; 6] {
    let mut id = i + 2;
    let (mut ax, mut ay, mut bx, mut by, mut cx, mut cy) = (0, 0, 0, 0, 0, 0);
    if id & 1 == 1 {
        bx = size;
        by = size;
        cx = size;
    } else {
        ax = size;
        ay = size;
        cy = size;
    }
    loop {
        id >>= 1;
        if id <= 1 {
            break;
        }
        let (mx, my) = ((ax + bx) / 2, (ay + by) / 2);
        if id & 1 == 1 {
            bx = ax;
            by = ay;
            ax = cx;
            ay = cy;
        } else {
            ax = bx;
            ay = by;
            bx = cx;
            by = cy;
        }
        cx = mx;
        cy = my;
    }
    [ax, ay, bx, by, cx, cy]
}

/// For every grid point, the largest error of leaving it out: how far the
/// surface there is from the hypotenuse it would sit on, carried up from the
/// smaller triangles it splits into.
fn rtin_errors(h: &[f32], n: usize) -> Vec<f32> {
    let size = n - 1;
    let total = size * size * 2 - 2;
    let parents = total - size * size;
    let mut errors = vec![0.0f32; n * n];
    for i in (0..total).rev() {
        let [ax, ay, bx, by, cx, cy] = corners(i, size);
        let (mx, my) = ((ax + bx) / 2, (ay + by) / 2);
        let mid = my * n + mx;
        let interpolated = (h[ay * n + ax] + h[by * n + bx]) / 2.0;
        let mut e = errors[mid].max((interpolated - h[mid]).abs());
        if i < parents {
            let left = ((ay + cy) / 2) * n + (ax + cx) / 2;
            let right = ((by + cy) / 2) * n + (bx + cx) / 2;
            e = e.max(errors[left]).max(errors[right]);
        }
        errors[mid] = e;
    }
    errors
}

fn rtin_walk(errors: &[f32], n: usize, max: f32, t: [usize; 6], out: &mut dyn FnMut([usize; 6])) {
    let [ax, ay, bx, by, cx, cy] = t;
    let (mx, my) = ((ax + bx) / 2, (ay + by) / 2);
    if ax.abs_diff(cx) + ay.abs_diff(cy) > 1 && errors[my * n + mx] > max {
        rtin_walk(errors, n, max, [cx, cy, ax, ay, mx, my], out);
        rtin_walk(errors, n, max, [bx, by, cx, cy, mx, my], out);
    } else {
        out(t);
    }
}

fn rtin_count(errors: &[f32], n: usize, max: f32) -> usize {
    let s = n - 1;
    let mut count = 0;
    rtin_walk(errors, n, max, [0, 0, s, s, s, 0], &mut |_| count += 1);
    rtin_walk(errors, n, max, [s, s, 0, 0, 0, s], &mut |_| count += 1);
    count
}

fn rtin_triangles(errors: &[f32], n: usize, max: f32) -> Vec<[usize; 6]> {
    let s = n - 1;
    let mut out = Vec::new();
    rtin_walk(errors, n, max, [0, 0, s, s, s, 0], &mut |t| out.push(t));
    rtin_walk(errors, n, max, [s, s, 0, 0, 0, s], &mut |t| out.push(t));
    out
}

/// The height surface closed into a solid: walls down each edge to the flat
/// side at z = `flat`. Grid column i runs along +x, row j down the picture
/// (−y). `mirror` turns it upside down (z → −z), for engraving cutters.
fn close(h: &[f32], n: usize, tris: &[[usize; 6]], width: f64, height: f64, flat: f64, mirror: bool) -> Result<Manifold, String> {
    let s = (n - 1) as f64;
    let xy = |i: usize, j: usize| [(i as f64 / s - 0.5) * width, (0.5 - j as f64 / s) * height];
    let mut index = vec![u32::MAX; n * n];
    let mut pos: Vec<f64> = Vec::new();
    let mut vertex = |i: usize, j: usize, pos: &mut Vec<f64>| -> u32 {
        let k = j * n + i;
        if index[k] == u32::MAX {
            let [x, y] = xy(i, j);
            pos.extend_from_slice(&[x, y, h[k] as f64]);
            index[k] = (pos.len() / 3 - 1) as u32;
        }
        index[k]
    };
    let mut out: Vec<u64> = Vec::with_capacity(tris.len() * 3 + 64);
    for t in tris {
        let [ax, ay, bx, by, cx, cy] = *t;
        let (a, b, c) = (vertex(ax, ay, &mut pos), vertex(bx, by, &mut pos), vertex(cx, cy, &mut pos));
        // Facing up: counter-clockwise seen from above.
        let p = |v: u32| [pos[v as usize * 3], pos[v as usize * 3 + 1]];
        let (pa, pb, pc) = (p(a), p(b), p(c));
        let cross = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
        if cross > 0.0 { out.extend([a as u64, b as u64, c as u64]) } else { out.extend([a as u64, c as u64, b as u64]) }
    }
    // The points the surface uses along each edge, in order, and the flat corners under them.
    let used = |i: usize, j: usize| index[j * n + i] != u32::MAX;
    let last = n - 1;
    let edges: [Vec<(usize, usize)>; 4] = [
        (0..n).map(|i| (i, last)).filter(|&(i, j)| used(i, j)).collect(), // bottom (−y)
        (0..n).rev().map(|j| (last, j)).filter(|&(i, j)| used(i, j)).collect(), // right (+x), going up
        (0..n).rev().map(|i| (i, 0)).filter(|&(i, j)| used(i, j)).collect(), // top (+y), going left
        (0..n).map(|j| (0, j)).filter(|&(i, j)| used(i, j)).collect(), // left (−x), going down
    ];
    let corner = |pos: &mut Vec<f64>, i: usize, j: usize| -> u64 {
        let [x, y] = xy(i, j);
        pos.extend_from_slice(&[x, y, flat]);
        (pos.len() / 3 - 1) as u64
    };
    let flat_corners = [corner(&mut pos, 0, last), corner(&mut pos, last, last), corner(&mut pos, last, 0), corner(&mut pos, 0, 0)];
    for (e, run) in edges.iter().enumerate() {
        let (c0, c1) = (flat_corners[e], flat_corners[(e + 1) % 4]);
        let top: Vec<u64> = run.iter().map(|&(i, j)| index[j * n + i] as u64).collect();
        // Seen from outside the wall runs left to right along the edge: the
        // surface above, the flat side below.
        let half = top.len() / 2;
        for k in 0..half {
            out.extend([c0, top[k + 1], top[k]]);
        }
        out.extend([c0, c1, top[half]]);
        for k in half..top.len() - 1 {
            out.extend([c1, top[k + 1], top[k]]);
        }
    }
    // The flat side, facing down.
    out.extend([flat_corners[0], flat_corners[2], flat_corners[1], flat_corners[0], flat_corners[3], flat_corners[2]]);
    if mirror {
        for z in pos.iter_mut().skip(2).step_by(3) {
            *z = -*z;
        }
        for t in out.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }
    Manifold::from_mesh_f64(&pos, 3, &out).map_err(|e| format!("relief: the mesh did not close ({e})"))
}

/// Bends a solid built flat around the Z axis: x runs around the circle
/// (x = 0 lands on +X, +x turning counter-clockwise seen from above), y runs
/// up the axis, and z = 0 lands on `radius` (+z outward). Long edges are first
/// split so no chord strays more than `tolerance` from the true curve.
pub fn wrap(solid: &Manifold, radius: f64, tolerance: f64) -> Result<Manifold, String> {
    if !(radius.is_finite() && radius > 0.0) {
        return Err("wrap: the radius must be a positive number of mm".into());
    }
    let b = solid.bounding_box().ok_or("wrap: the solid is empty")?;
    let (lo, hi) = (b.min(), b.max());
    if radius + lo[2] <= 0.0 {
        return Err(format!("wrap: the solid reaches {:.3} mm inward, past the axis (radius {radius} mm)", -lo[2]));
    }
    let span = (hi[0] - lo[0]) / radius;
    if span >= 2.0 * std::f64::consts::PI - 1e-9 {
        return Err(format!("wrap: {:.1} mm does not fit around a {radius} mm radius ({:.1} mm all the way round)", hi[0] - lo[0], 2.0 * std::f64::consts::PI * radius));
    }
    // A chord of length L on the outermost radius strays L²/(8r) from its arc.
    let outer = radius + hi[2].max(0.0);
    let length = (8.0 * outer * tolerance.max(1e-4)).sqrt().max(1e-3);
    let refined = solid.refine_to_length(length);
    Ok(refined.warp(|x, y, z| {
        let (r, a) = (radius + z, x / radius);
        [r * a.cos(), r * a.sin(), y]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Picture {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = f(x, y);
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        crate::image::decode(&crate::image::encode_png(w as u32, h as u32, &px)).unwrap()
    }

    fn spec(mode: Mode) -> Spec {
        Spec { width: 40.0, height: 20.0, depth: 2.0, base: 1.0, mode, invert: false, gamma: 1.0, smooth: 0.0, channel: Channel::Brightness, clear: 1.0, tolerance: 0.01, budget: 2_000_000, detail: 1024 }
    }

    #[test]
    fn a_flat_picture_is_a_box() {
        let p = picture(64, 32, |_, _| 255);
        let b = build(&p, &spec(Mode::Emboss)).unwrap();
        // White is high: base + depth = 3 mm everywhere, two triangles a face.
        assert!((b.solid.volume() - 40.0 * 20.0 * 3.0).abs() < 1e-6, "{}", b.solid.volume());
        assert!(b.triangles < 40, "{} triangles for a flat picture", b.triangles);
        assert_eq!(b.solid.genus(), 0);
    }

    #[test]
    fn a_gradient_is_meshed_within_tolerance_and_closed() {
        // Left black to right white, with a bump: emboss rises left to right.
        let p = picture(200, 100, |x, y| {
            let bump = (-(((x as f64 - 100.0).powi(2) + (y as f64 - 50.0).powi(2)) / 200.0)).exp();
            ((x as f64 / 199.0 * 0.7 + 0.3 * bump) * 255.0) as u8
        });
        let b = build(&p, &spec(Mode::Emboss)).unwrap();
        assert_eq!(b.solid.genus(), 0);
        let bb = b.solid.bounding_box().unwrap();
        assert!((bb.min()[2]).abs() < 1e-9 && bb.max()[2] <= 3.0 + 1e-6 && bb.max()[2] > 2.3);
        // The volume matches the mean height to within a grey level.
        let mean: f64 = values(&p, &spec(Mode::Emboss), b.grid).v.iter().map(|v| *v as f64).sum::<f64>() / (b.grid * b.grid) as f64;
        let expect = 40.0 * 20.0 * (1.0 + 2.0 * mean);
        assert!((b.solid.volume() - expect).abs() / expect < 0.01, "{} vs {expect}", b.solid.volume());
    }

    #[test]
    fn an_engraving_cutter_reaches_down_where_the_picture_is_dark() {
        // A black square in white: cut 2 mm deep there, nothing elsewhere.
        let p = picture(100, 100, |x, y| if (30..70).contains(&x) && (30..70).contains(&y) { 0 } else { 255 });
        let mut s = spec(Mode::Engrave);
        s.base = 0.0;
        s.width = 20.0;
        s.height = 20.0;
        let b = build(&p, &s).unwrap();
        let bb = b.solid.bounding_box().unwrap();
        assert!((bb.min()[2] + 2.0).abs() < 1e-6 && (bb.max()[2] - 1.0).abs() < 1e-6, "{:?} {:?}", bb.min(), bb.max());
        let plate = Manifold::cube(20.0, 20.0, 5.0, true).translate(0.0, 0.0, -2.5);
        let carved = plate.difference(&b.solid);
        // The pocket: about 8 × 8 mm, 2 mm deep (edges blend over a pixel).
        let removed = plate.volume() - carved.volume();
        assert!((removed - 8.0 * 8.0 * 2.0).abs() < 8.0, "removed {removed} mm³");
    }

    #[test]
    fn a_lithophane_is_thick_where_dark() {
        let p = picture(10, 10, |x, _| if x < 5 { 0 } else { 255 });
        let mut s = spec(Mode::Lithophane);
        s.base = 0.8;
        s.depth = 2.4;
        let b = build(&p, &s).unwrap();
        let bb = b.solid.bounding_box().unwrap();
        assert!((bb.max()[2] - 3.2).abs() < 1e-6);
        let left = b.solid.intersection(&Manifold::cube(10.0, 20.0, 10.0, false).translate(-20.0, -10.0, 0.0));
        assert!((left.volume() - 10.0 * 20.0 * 3.2).abs() < 1.0, "{}", left.volume());
    }

    #[test]
    fn a_thousand_pixel_photo_meshes_in_seconds() {
        let p = picture(1000, 1000, |x, y| (((x as f64 * 0.05).sin() * (y as f64 * 0.031).cos() * 0.5 + 0.5) * 255.0) as u8);
        let started = std::time::Instant::now();
        let mut s = spec(Mode::Emboss);
        s.width = 100.0;
        s.height = 100.0;
        s.budget = 600_000;
        let b = build(&p, &s).unwrap();
        let took = started.elapsed().as_secs_f64();
        assert!(b.triangles <= 620_000 && b.solid.genus() == 0, "{} triangles", b.triangles);
        assert!(took < 8.0, "took {took:.2} s");
    }

    #[test]
    fn wrapping_keeps_the_solid_closed_and_on_its_radius() {
        let flat = Manifold::cube(20.0, 10.0, 1.0, false).translate(-10.0, 0.0, 0.0);
        let w = wrap(&flat, 15.0, 0.01).unwrap();
        let bb = w.bounding_box().unwrap();
        assert!((bb.max()[0] - 16.0).abs() < 0.02 && w.genus() == 0);
        // The volume of a ring segment: angle/2 × (R² − r²) × height.
        let expect = (20.0 / 15.0) / 2.0 * (16.0f64.powi(2) - 15.0f64.powi(2)) * 10.0;
        assert!((w.volume() - expect).abs() / expect < 0.01, "{} vs {expect}", w.volume());
        assert!(wrap(&Manifold::cube(100.0, 1.0, 1.0, false), 10.0, 0.01).is_err());
    }
}
