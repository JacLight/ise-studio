// Trapezoidal (ACME-like) lead screw thread helpers shared by lead_screw_t8
// and lead_screw_nut_t8.
//
// Source: ISO 2904 basic trapezoidal profile, 30° included angle — pitch P,
// thread depth H1 = 0.5·P, clearance ac = 0.25 (P 1.5–5), external minor
// d3 = d − P − 2·ac, pitch diameter d2 = d − 0.5·P, crest flat 0.366·P.
// The T8 lead screws sold for 3D printers and CNC are Tr8 with P = 2 and
// 1, 2 or 4 starts (lead 2, 4, 8 mm: "Tr8×2", "Tr8×4 (P2)", "Tr8×8 (P2)").

const TR_AC = 0.25;

/** Points (r, z) of one trapezoidal tooth per pitch from z0 to z1 at major radius R, depth h. */
function trapezoidRings({ R, pitch, depth, z0, z1 }) {
  const r = R - depth;
  const crest = 0.366 * pitch;
  const flank = depth * Math.tan(rad(15));
  const root = pitch - crest - 2 * flank;
  const n = Math.max(1, Math.floor((z1 - z0) / pitch));
  const z = z0 + (z1 - z0 - n * pitch) / 2;
  const pts = [[0, z0], [r, z0]];
  for (let i = 0; i < n; i++) {
    const b = z + i * pitch + root / 2;
    pts.push([r, b], [R, b + flank], [R, b + flank + crest], [r, b + 2 * flank + crest]);
  }
  pts.push([r, z1], [0, z1]);
  return polygon(pts).revolve(360);
}

/**
 * A trapezoidal thread swept along `starts` helices (lead = starts·pitch)
 * from z0 to z1, over its core: the true multi-start screw (faceted in STEP).
 */
function trapezoidHelix({ R, pitch, depth, starts, z0, z1 }) {
  const lead = starts * pitch;
  const r = R - depth;
  const sink = 0.1 * pitch;
  const crest = 0.366 * pitch;
  const flank = depth * Math.tan(rad(15));
  const half = crest / 2 + flank + sink * Math.tan(rad(15));
  const profile = [[r - sink, -half], [R, -crest / 2], [R, crest / 2], [r - sink, half]];
  const perTurn = segmentsFor(R);
  const ridges = [];
  for (let k = 0; k < starts; k++) {
    const phase = z0 + k * pitch;
    const t0 = Math.floor((z0 - phase) / lead) - 1;
    const t1 = Math.ceil((z1 - phase) / lead) + 1;
    const steps = (t1 - t0) * perTurn;
    const positions = [];
    for (let s = 0; s <= steps; s++) {
      const turn = t0 + s / perTurn;
      const angle = 2 * Math.PI * turn;
      const zc = phase + turn * lead;
      for (const [rr, dz] of profile) positions.push([rr * Math.cos(angle), rr * Math.sin(angle), zc + dz]);
    }
    const at = (s, j) => s * 4 + j;
    const triangles = [];
    for (let s = 0; s < steps; s++) {
      for (let j = 0; j < 4; j++) {
        const q = (j + 1) % 4;
        triangles.push([at(s, j), at(s + 1, j), at(s + 1, q)], [at(s, j), at(s + 1, q), at(s, q)]);
      }
    }
    triangles.push([at(0, 0), at(0, 1), at(0, 2)], [at(0, 0), at(0, 2), at(0, 3)]);
    triangles.push([at(steps, 0), at(steps, 2), at(steps, 1)], [at(steps, 0), at(steps, 3), at(steps, 2)]);
    ridges.push(fromMesh(positions, triangles));
  }
  const core = cylinder(z1 - z0, r).translate(0, 0, z0);
  const slab = cylinder(z1 - z0, R + 1).translate(0, 0, z0);
  return union(core, ...ridges).intersect(slab);
}

/** The basic Tr d × lead (P pitch) external thread's depth (H1/2 + ac at the root). */
const trDepth = (pitch) => 0.5 * pitch + TR_AC;

/** The tool that cuts the nut's internal thread (rings), grown by `clearance`. */
function trNutTool({ d, pitch, z0, z1, clearance = 0.1 }) {
  return trapezoidRings({ R: d / 2 + clearance + TR_AC, pitch, depth: trDepth(pitch), z0: z0 - pitch, z1: z1 + pitch });
}
