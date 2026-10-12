// Wire-spring helpers shared by spring_compression, spring_extension and
// spring_torsion: the wire is a round tube swept along its centre line, a
// path of points, as one closed mesh (faceted in STEP; a true helical sweep
// is no more exact to a spring maker than its own tolerance on coil
// diameter, ±2 % of OD per EN 15800 grade 2).
//
// Material: patented cold-drawn spring steel wire ("music wire", EN 10270-1
// SH / ASTM A228), G = 81.5 GPa, E = 206 GPa (EN 13906-1 table 1),
// 7.85 g/cm³.

const SPRING_G = 81500; // N/mm², shear modulus
const SPRING_E = 206000; // N/mm², Young's modulus

const v3add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const v3sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const v3mul = (a, k) => [a[0] * k, a[1] * k, a[2] * k];
const v3dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const v3cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const v3unit = (a) => v3mul(a, 1 / Math.hypot(a[0], a[1], a[2]));

/**
 * A round wire of radius `r` along the polyline `path` ([x, y, z] points,
 * no two equal), with flat ends: one closed mesh. Frames are carried along
 * by parallel transport, so the tube never twists.
 */
function wireAlong(path, r, segments = 12) {
  const n = path.length;
  const tangent = path.map((_, i) => v3unit(v3sub(path[Math.min(i + 1, n - 1)], path[Math.max(i - 1, 0)])));
  const t0 = tangent[0];
  let normal = v3unit(v3cross(t0, Math.abs(t0[2]) < 0.9 ? [0, 0, 1] : [1, 0, 0]));
  const positions = [];
  for (let i = 0; i < n; i++) {
    const t = tangent[i];
    normal = v3unit(v3sub(normal, v3mul(t, v3dot(normal, t))));
    const binormal = v3cross(t, normal);
    for (let j = 0; j < segments; j++) {
      const a = (2 * Math.PI * j) / segments;
      positions.push(v3add(path[i], v3add(v3mul(normal, r * Math.cos(a)), v3mul(binormal, r * Math.sin(a)))));
    }
  }
  const triangles = [];
  const at = (i, j) => i * segments + (j % segments);
  for (let i = 0; i < n - 1; i++) {
    for (let j = 0; j < segments; j++) {
      triangles.push([at(i, j), at(i, j + 1), at(i + 1, j + 1)], [at(i, j), at(i + 1, j + 1), at(i + 1, j)]);
    }
  }
  const s = positions.length;
  positions.push(path[0], path[n - 1]);
  for (let j = 0; j < segments; j++) {
    triangles.push([s, at(0, j + 1), at(0, j)], [s + 1, at(n - 1, j), at(n - 1, j + 1)]);
  }
  return fromMesh(positions, triangles);
}

/**
 * Points of a coil centre line of mean radius `R`: `turns(t)` gives the
 * height at t turns, from t = t0 to t1, starting at angle `a0` (radians).
 * `perTurn` points a turn.
 */
function coilPath(R, z, t0, t1, { a0 = 0, perTurn = 36 } = {}) {
  const steps = Math.max(2, Math.round((t1 - t0) * perTurn));
  const pts = [];
  for (let s = 0; s <= steps; s++) {
    const t = t0 + ((t1 - t0) * s) / steps;
    const a = a0 + 2 * Math.PI * t;
    pts.push([R * Math.cos(a), R * Math.sin(a), z(t)]);
  }
  return pts;
}

/** Points along a straight line, the first excluded (it is the previous segment's last). */
function linePath(a, b, steps = 1) {
  const pts = [];
  for (let s = 1; s <= steps; s++) pts.push(v3add(a, v3mul(v3sub(b, a), s / steps)));
  return pts;
}

/** Points per turn for a coil of mean radius R: fine enough to read as round. */
const coilSteps = (R) => Math.max(28, Math.min(64, Math.round(12 * Math.sqrt(R))));
/** Segments round the wire. */
const wireSegments = (d) => (d < 0.7 ? 10 : 12);
