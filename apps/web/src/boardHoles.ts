// Which holes the 3D board cuts through its faces, and which it paints.
//
// Cutting a hole into the board's outline makes the face a polygon with one
// more hole for the triangulator (three.js's earcut), whose hole joining grows
// with the square of the count: a dense board's few hundred vias took minutes
// to triangulate. Vias and small plated holes (a thermal pad's vias, a module's
// ground vias) are drawn as drill marks in the face's texture instead; at
// these sizes they read the same, and a fab tents vias under the mask anyway.
// Slots, unplated holes, cutouts and plated holes from CUT_MIN up go through.

export type Hole = { at: [number, number]; size: [number, number]; rot: number; plated: boolean };
export type Via = [number, number, number, number];

/** Plated round holes smaller than this (mm) are painted, not cut. */
export const CUT_MIN = 0.8;
/** At most this many holes are cut (the largest first); the rest are painted. */
export const CUT_MAX = 150;

export function splitHoles(holes: Hole[], vias: Via[]): { cut: Hole[]; painted: { at: [number, number]; d: number }[] } {
  const size = (h: Hole) => Math.max(h.size[0], h.size[1]);
  const round = (h: Hole) => Math.abs(h.size[0] - h.size[1]) < 1e-6;
  const wanted = holes.filter((h) => !h.plated || !round(h) || size(h) >= CUT_MIN);
  const cut = [...wanted].sort((a, b) => size(b) - size(a)).slice(0, CUT_MAX);
  const painted = holes.filter((h) => !cut.includes(h)).map((h) => ({ at: h.at, d: Math.min(h.size[0], h.size[1]) }));
  for (const v of vias) painted.push({ at: [v[0], v[1]], d: v[3] });
  return { cut, painted };
}
