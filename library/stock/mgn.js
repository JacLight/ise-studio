// MGN miniature profile rail guides (HIWIN MGN, and the generic MGN-pattern
// rails and blocks sold for 3D printers): helpers shared by linear_rail_mgn
// and linear_carriage_mgn.
//
// Source: HIWIN "Linear Guideway" catalogue, MG series (MGN…C / MGN…H
// blocks, MGN…R rails) — rail width WR, height HR, counterbore D × h,
// through hole d, hole pitch P, rail mass; block height H (rail bottom to
// block top), H1 (rail bottom to block bottom), width W, hole spacing B × C,
// tapped M × l, steel body length L1, overall length L, block mass:
//   MGN7  WR 7  HR 4.8  D 4.2 h 2.3 d 2.4 P 15  0.22 kg/m  H 8  H1 1.5 W 17 B 12 M2×2.5  C 8/13  L1 13.5/21.8 L 22.5/30.8  10/15 g
//   MGN9  WR 9  HR 6.5  D 6   h 3.5 d 3.5 P 20  0.38 kg/m  H 10 H1 2   W 20 B 15 M3×3   C 10/16 L1 18.9/29.9 L 28.9/39.9  16/26 g
//   MGN12 WR 12 HR 8    D 6   h 4.5 d 3.5 P 25  0.65 kg/m  H 13 H1 3   W 27 B 20 M3×3.5 C 15/20 L1 21.7/32.4 L 34.7/45.4  34/54 g
//   MGN15 WR 15 HR 10   D 6   h 4.5 d 3.5 P 40  1.06 kg/m  H 16 H1 4   W 32 B 25 M3×4   C 20/25 L1 26.7/43.4 L 42.1/58.8  59/92 g
// Two rows of balls in gothic-arch grooves at the rail's upper shoulders
// (ball sizes typical of the series: 1.2, 1.5, 2, 2.381 mm).
//
// Coordinates: the rail lies along +X from x = 0, its base on z = 0, centred
// on y = 0. A block is in the same frame, centred on x = 0, so a block
// placed at the rail's origin sits on it; it slides along X.

const MGN = {
  //   WR  HR    D    h    d    P   kg/m  bolt  H   H1   W   B  tap  l    C       L1           L            mass g    Dw
  7: [7, 4.8, 4.2, 2.3, 2.4, 15, 0.22, 2, 8, 1.5, 17, 12, 2, 2.5, [8, 13], [13.5, 21.8], [22.5, 30.8], [10, 15], 1.2],
  9: [9, 6.5, 6, 3.5, 3.5, 20, 0.38, 3, 10, 2, 20, 15, 3, 3, [10, 16], [18.9, 29.9], [28.9, 39.9], [16, 26], 1.5],
  12: [12, 8, 6, 4.5, 3.5, 25, 0.65, 3, 13, 3, 27, 20, 3, 3.5, [15, 20], [21.7, 32.4], [34.7, 45.4], [34, 54], 2.0],
  15: [15, 10, 6, 4.5, 3.5, 40, 1.06, 3, 16, 4, 32, 25, 3, 4, [20, 25], [26.7, 43.4], [42.1, 58.8], [59, 92], 2.381],
};

function mgn(series) {
  const [WR, HR, Dc, hc, dh, P, kgPerM, bolt, H, H1, W, B, tap, l, C, L1, L, mass, Dw] = lookup(MGN, series, "MGN rail");
  return { WR, HR, Dc, hc, dh, P, kgPerM, bolt, H, H1, W, B, tap, l, C, L1, L, mass, Dw };
}

/** Where the two ball rows run (y = ±yb, z = zb) and the groove radius. */
function mgnBallRows(s) {
  return { yb: s.WR / 2 - 0.25 * s.Dw, zb: s.HR - 0.3 * s.HR, rg: 0.52 * s.Dw };
}

/** Turns a cross-section drawn in (y, z) and extruded along +Z to run along +X. */
const alongX = (solid) => solid.rotate(90, 0, 90);
