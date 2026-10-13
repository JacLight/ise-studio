//! Small geometry for IFC: polygon area, triangulation with holes, prisms
//! from profiles, welding and checking triangle meshes, footprints.

use crate::Mesh;
use std::collections::HashMap;

/// The signed area of a closed polygon (positive anticlockwise).
pub fn area2(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..n {
        let (x0, y0) = (p[i][0], p[i][1]);
        let (x1, y1) = (p[(i + 1) % n][0], p[(i + 1) % n][1]);
        a += x0 * y1 - x1 * y0;
    }
    a / 2.0
}

pub fn perimeter(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    if n < 2 {
        return 0.0;
    }
    (0..n).map(|i| dist2(p[i], p[(i + 1) % n])).sum()
}

pub fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// A loop without its closing repeat and without repeated points.
pub fn clean_loop(p: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(p.len());
    for &q in p {
        if out.last().is_none_or(|l| dist2(*l, q) > 1e-9) {
            out.push(q);
        }
    }
    while out.len() > 1 && dist2(out[0], *out.last().unwrap()) <= 1e-9 {
        out.pop();
    }
    out
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    d1 >= -1e-12 && d2 >= -1e-12 && d3 >= -1e-12
}

/// Triangles (indices into the points of `outer` then each hole, in order)
/// covering a polygon with holes. Ear clipping after bridging each hole to
/// the outer loop.
pub fn triangulate(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> Vec<[usize; 3]> {
    // Every point with its index in the concatenated input.
    let mut pts: Vec<[f64; 2]> = outer.to_vec();
    let mut ring: Vec<usize> = (0..outer.len()).collect();
    if area2(outer) < 0.0 {
        ring.reverse();
    }
    // Holes, rightmost first, each joined to the ring by a bridge from its rightmost point.
    let mut hs: Vec<Vec<usize>> = Vec::new();
    for h in holes {
        let start = pts.len();
        pts.extend_from_slice(h);
        let mut idx: Vec<usize> = (start..start + h.len()).collect();
        if area2(h) > 0.0 {
            idx.reverse();
        }
        if idx.len() >= 3 {
            hs.push(idx);
        }
    }
    hs.sort_by(|a, b| {
        let ma = a.iter().map(|&i| pts[i][0]).fold(f64::NEG_INFINITY, f64::max);
        let mb = b.iter().map(|&i| pts[i][0]).fold(f64::NEG_INFINITY, f64::max);
        mb.total_cmp(&ma)
    });
    for h in hs {
        let (hi, &m) = h.iter().enumerate().max_by(|a, b| pts[*a.1][0].total_cmp(&pts[*b.1][0])).unwrap();
        let mp = pts[m];
        // The nearest ring vertex visible from m (no ring edge crosses the bridge).
        let mut best: Option<(f64, usize)> = None;
        for (k, &v) in ring.iter().enumerate() {
            let vp = pts[v];
            let d = (vp[0] - mp[0]).powi(2) + (vp[1] - mp[1]).powi(2);
            if best.is_some_and(|(bd, _)| bd <= d) {
                continue;
            }
            let n = ring.len();
            let blocked = (0..n).any(|e| {
                let (a, b) = (ring[e], ring[(e + 1) % n]);
                if a == v || b == v {
                    return false;
                }
                segments_cross(mp, vp, pts[a], pts[b])
            }) || h.windows(2).chain(std::iter::once(&[h[h.len() - 1], h[0]][..])).any(|w| {
                let (a, b) = (w[0], w[1]);
                a != m && b != m && segments_cross(mp, vp, pts[a], pts[b])
            });
            if !blocked {
                best = Some((d, k));
            }
        }
        let Some((_, k)) = best else { continue };
        // ring[..=k], hole from m round to m, back to ring[k], rest of ring.
        let mut next = ring[..=k].to_vec();
        for j in 0..=h.len() {
            next.push(h[(hi + j) % h.len()]);
        }
        next.push(ring[k]);
        next.extend_from_slice(&ring[k + 1..]);
        ring = next;
    }
    ear_clip(&pts, ring)
}

fn segments_cross(p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]) -> bool {
    let d1 = cross(p, q, r);
    let d2 = cross(p, q, s);
    let d3 = cross(r, s, p);
    let d4 = cross(r, s, q);
    ((d1 > 1e-12 && d2 < -1e-12) || (d1 < -1e-12 && d2 > 1e-12)) && ((d3 > 1e-12 && d4 < -1e-12) || (d3 < -1e-12 && d4 > 1e-12))
}

fn ear_clip(pts: &[[f64; 2]], mut ring: Vec<usize>) -> Vec<[usize; 3]> {
    let mut out = Vec::new();
    let mut guard = 0;
    while ring.len() > 3 && guard < 100_000 {
        guard += 1;
        let n = ring.len();
        let mut clipped = false;
        for i in 0..n {
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            let (pa, pb, pc) = (pts[a], pts[b], pts[c]);
            let turn = cross(pa, pb, pc);
            if turn <= 1e-12 {
                // Reflex or flat: a flat one with a repeated point goes as a sliver.
                if turn.abs() <= 1e-12 && (dist2(pa, pb) < 1e-12 || dist2(pb, pc) < 1e-12) {
                    ring.remove(i);
                    clipped = true;
                    break;
                }
                continue;
            }
            let blocked = ring.iter().any(|&v| v != a && v != b && v != c && pts[v] != pa && pts[v] != pb && pts[v] != pc && in_triangle(pts[v], pa, pb, pc));
            if blocked {
                continue;
            }
            out.push([a, b, c]);
            ring.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // Degenerate input: fan what is left so nothing is lost.
            for i in 1..ring.len() - 1 {
                out.push([ring[0], ring[i], ring[i + 1]]);
            }
            return out;
        }
    }
    if ring.len() == 3 {
        out.push([ring[0], ring[1], ring[2]]);
    }
    out
}

/// A prism: the profile (outer loop and holes, in its plane) swept along
/// `dir` (a vector, its length the depth). Points are 2D in the XY plane at z = 0.
pub fn extrude(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>], dir: [f64; 3]) -> Mesh {
    let outer = clean_loop(outer);
    let holes: Vec<Vec<[f64; 2]>> = holes.iter().map(|h| clean_loop(h)).filter(|h| h.len() >= 3).collect();
    let mut m = Mesh::default();
    if outer.len() < 3 {
        return m;
    }
    let mut loops: Vec<Vec<[f64; 2]>> = vec![outer.clone()];
    loops.extend(holes.iter().cloned());
    let count: usize = loops.iter().map(Vec::len).sum();
    for l in &loops {
        for p in l {
            m.positions.push([p[0], p[1], 0.0]);
        }
    }
    for l in &loops {
        for p in l {
            m.positions.push([p[0] + dir[0], p[1] + dir[1], dir[2]]);
        }
    }
    let up = dir[2] >= 0.0;
    for t in triangulate(&outer, &holes) {
        let (a, b, c) = (t[0] as u32, t[1] as u32, t[2] as u32);
        let k = count as u32;
        // Triangulation is anticlockwise seen from +Z: the bottom faces down.
        if up {
            m.triangles.push([a, c, b]);
            m.triangles.push([a + k, b + k, c + k]);
        } else {
            m.triangles.push([a, b, c]);
            m.triangles.push([a + k, c + k, b + k]);
        }
    }
    let mut start = 0;
    for (li, l) in loops.iter().enumerate() {
        let n = l.len();
        // The outer loop anticlockwise, holes clockwise: sides face out.
        let ccw = area2(l) > 0.0;
        let want_ccw = li == 0;
        for i in 0..n {
            let (mut i0, mut i1) = (start + i, start + (i + 1) % n);
            if ccw != want_ccw {
                std::mem::swap(&mut i0, &mut i1);
            }
            let (a, b) = (i0 as u32, i1 as u32);
            let (a2, b2) = (a + count as u32, b + count as u32);
            if up {
                m.triangles.push([a, b, b2]);
                m.triangles.push([a, b2, a2]);
            } else {
                m.triangles.push([a, b2, b]);
                m.triangles.push([a, a2, b2]);
            }
        }
        start += n;
    }
    m
}

/// A 4 × 3 affine transform: rows are the images of X, Y, Z, then the origin.
pub type Affine = [[f64; 3]; 4];

pub const IDENTITY: Affine = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]];

pub fn apply(t: &Affine, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|k| t[0][k] * p[0] + t[1][k] * p[1] + t[2][k] * p[2] + t[3][k])
}

/// `outer` after `inner`.
pub fn compose(outer: &Affine, inner: &Affine) -> Affine {
    let turn = |v: [f64; 3]| [0, 1, 2].map(|k| outer[0][k] * v[0] + outer[1][k] * v[1] + outer[2][k] * v[2]);
    [turn(inner[0]), turn(inner[1]), turn(inner[2]), apply(outer, inner[3])]
}

/// Inverse of a rigid (or uniformly scaled) transform.
pub fn invert(t: &Affine) -> Affine {
    // General 3 × 3 inverse.
    let m = [[t[0][0], t[1][0], t[2][0]], [t[0][1], t[1][1], t[2][1]], [t[0][2], t[1][2], t[2][2]]];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-300 {
        return IDENTITY;
    }
    let inv = [
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) / det, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / det, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / det],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) / det, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / det, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / det],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) / det, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / det, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / det],
    ];
    let o = t[3];
    let to = [0, 1, 2].map(|r| -(inv[r][0] * o[0] + inv[r][1] * o[1] + inv[r][2] * o[2]));
    [[inv[0][0], inv[1][0], inv[2][0]], [inv[0][1], inv[1][1], inv[2][1]], [inv[0][2], inv[1][2], inv[2][2]], to]
}

pub fn transform_mesh(t: &Affine, m: &mut Mesh) {
    for p in &mut m.positions {
        *p = apply(t, *p);
    }
    // A mirroring transform turns the triangles inside out.
    let d = t[0][0] * (t[1][1] * t[2][2] - t[1][2] * t[2][1]) - t[0][1] * (t[1][0] * t[2][2] - t[1][2] * t[2][0]) + t[0][2] * (t[1][0] * t[2][1] - t[1][1] * t[2][0]);
    if d < 0.0 {
        for tri in &mut m.triangles {
            tri.swap(1, 2);
        }
    }
}

pub fn append(into: &mut Mesh, m: &Mesh) {
    let off = into.positions.len() as u32;
    into.positions.extend_from_slice(&m.positions);
    into.triangles.extend(m.triangles.iter().map(|t| t.map(|i| i + off)));
}

/// Merges points closer than `tol` and drops triangles that collapse.
pub fn weld(m: &Mesh, tol: f64) -> Mesh {
    let q = |v: f64| (v / tol).round() as i64;
    let mut map: HashMap<[i64; 3], u32> = HashMap::new();
    let mut out = Mesh::default();
    let mut remap = Vec::with_capacity(m.positions.len());
    for p in &m.positions {
        let k = [q(p[0]), q(p[1]), q(p[2])];
        let i = *map.entry(k).or_insert_with(|| {
            out.positions.push(*p);
            (out.positions.len() - 1) as u32
        });
        remap.push(i);
    }
    for t in &m.triangles {
        let r = t.map(|i| remap[i as usize]);
        if r[0] != r[1] && r[1] != r[2] && r[0] != r[2] {
            out.triangles.push(r);
        }
    }
    out
}

/// Signed volume (mm³; negative when the triangles face inwards).
pub fn volume(m: &Mesh) -> f64 {
    let mut v = 0.0;
    for t in &m.triangles {
        let [a, b, c] = t.map(|i| m.positions[i as usize]);
        v += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    v / 6.0
}

/// Every edge is shared by exactly two triangles running opposite ways.
pub fn is_closed(m: &Mesh) -> bool {
    let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
    for t in &m.triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *edges.entry((a, b)).or_default() += 1;
        }
    }
    !m.triangles.is_empty() && edges.iter().all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
}

/// Welded, facing outwards; `None` when it does not close.
pub fn solid(m: &Mesh, tol: f64) -> Option<Mesh> {
    let mut w = weld(m, tol);
    if !is_closed(&w) {
        return None;
    }
    if volume(&w) < 0.0 {
        for t in &mut w.triangles {
            t.swap(1, 2);
        }
    }
    Some(w)
}

pub fn bounds(m: &Mesh) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &m.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// The loops bounding the downward-facing triangles at the lowest level of a
/// closed mesh: a prism's floor outline (outer anticlockwise, holes clockwise).
pub fn footprint(m: &Mesh) -> Vec<Vec<[f64; 2]>> {
    let (lo, hi) = bounds(m);
    let tol = ((hi[2] - lo[2]) * 1e-6).max(1e-6);
    let mut count: HashMap<(u32, u32), i32> = HashMap::new();
    for t in &m.triangles {
        let [a, b, c] = t.map(|i| m.positions[i as usize]);
        if [a, b, c].iter().any(|p| (p[2] - lo[2]).abs() > tol) {
            continue;
        }
        let nz = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if nz >= 0.0 {
            continue;
        }
        for k in 0..3 {
            // Seen from above, a downward triangle runs clockwise: reverse its edges.
            let (u, v) = (t[(k + 1) % 3], t[k]);
            if let Some(n) = count.get_mut(&(v, u)) {
                *n -= 1;
                continue;
            }
            *count.entry((u, v)).or_default() += 1;
        }
    }
    let mut next: HashMap<u32, u32> = HashMap::new();
    for ((u, v), n) in count {
        if n > 0 {
            next.insert(u, v);
        }
    }
    let mut loops = Vec::new();
    let mut keys: Vec<u32> = next.keys().copied().collect();
    keys.sort();
    let mut used = std::collections::HashSet::new();
    for s in keys {
        if used.contains(&s) {
            continue;
        }
        let mut l = Vec::new();
        let mut v = s;
        while used.insert(v) {
            let p = m.positions[v as usize];
            l.push([p[0], p[1]]);
            match next.get(&v) {
                Some(&w) => v = w,
                None => break,
            }
        }
        // Drop points in the middle of straight runs.
        let l = simplify_loop(&l);
        if l.len() >= 3 {
            loops.push(l);
        }
    }
    loops.sort_by(|a, b| area2(b).abs().total_cmp(&area2(a).abs()));
    loops
}

fn simplify_loop(l: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = l.len();
    if n < 4 {
        return l.to_vec();
    }
    (0..n)
        .filter(|&i| {
            let (a, b, c) = (l[(i + n - 1) % n], l[i], l[(i + 1) % n]);
            cross(a, b, c).abs() > 1e-9 * (dist2(a, b) * dist2(b, c)).max(1e-12).sqrt().max(1.0)
        })
        .map(|i| l[i])
        .collect()
}

/// A point well inside a polygon (for a label): the middle of the longest
/// horizontal run through it, at the height of its centre.
pub fn label_point(outer: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> [f64; 2] {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in outer {
        lo = lo.min(p[1]);
        hi = hi.max(p[1]);
    }
    let mut best = ([0.0, 0.0], -1.0);
    for k in 1..8 {
        let y = lo + (hi - lo) * k as f64 / 8.0;
        let mut xs = Vec::new();
        for l in std::iter::once(outer).chain(holes.iter().map(Vec::as_slice)) {
            let n = l.len();
            for i in 0..n {
                let (a, b) = (l[i], l[(i + 1) % n]);
                if (a[1] <= y) != (b[1] <= y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for w in xs.chunks(2) {
            if w.len() == 2 {
                // Prefer runs near the middle height a little.
                let len = (w[1] - w[0]) * (1.0 - 0.3 * ((k as f64 - 4.0) / 4.0).abs());
                if len > best.1 {
                    best = ([(w[0] + w[1]) / 2.0, y], len);
                }
            }
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_with_hole_triangulates_to_its_area() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[3.0, 3.0], [3.0, 6.0], [6.0, 6.0], [6.0, 3.0]];
        let tris = triangulate(&outer, &[hole.clone()]);
        let pts: Vec<[f64; 2]> = outer.iter().chain(hole.iter()).copied().collect();
        let a: f64 = tris.iter().map(|t| area2(&[pts[t[0]], pts[t[1]], pts[t[2]]])).sum();
        assert!((a - 91.0).abs() < 1e-9, "area {a}");
    }

    #[test]
    fn l_shaped_prism_is_closed_with_the_right_volume() {
        let l = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 1.0], [1.0, 1.0], [1.0, 3.0], [0.0, 3.0]];
        let m = extrude(&l, &[], [0.0, 0.0, 2.0]);
        let s = solid(&m, 1e-9).expect("closed");
        assert!((volume(&s) - 12.0).abs() < 1e-9);
        let f = footprint(&s);
        assert_eq!(f.len(), 1);
        assert!((area2(&f[0]) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn transforms_invert() {
        let t: Affine = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [5.0, 6.0, 7.0]];
        let p = apply(&invert(&t), apply(&t, [1.0, 2.0, 3.0]));
        assert!((p[0] - 1.0).abs() < 1e-12 && (p[1] - 2.0).abs() < 1e-12 && (p[2] - 3.0).abs() < 1e-12);
    }
}
