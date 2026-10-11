// A row of private offices fitted into an open floor: a glazed aluminium
// front (head and floor tracks, mullions, 12 mm toughened glass, timber door
// with a glass transom) and drywall separating walls (C-studs at 600 mm in
// U-tracks, 12.5 mm gypsum both sides). Every member is its own part.
// X along the corridor, Y from the corridor (0) into the offices, Z up.
const N = p.offices, W = p.officeWidth, Dp = p.officeDepth, H = p.height;
const parts = [];
const alu = { material: "Aluminium", process: "extrusion" };
const glass = { material: "Glass", process: "glazing" };
const steel = { material: "Galvanised steel", process: "extrusion" };
const board = { material: "Gypsum", process: "cnc_router" };

// ---- Glazed front --------------------------------------------------------
const fd = 50; // depth of the front system (Y)
const trackH = 40, trackT = 2;
const mull = 50; // mullion width (X)
const gT = p.glass;
const gy = (fd - gT) / 2; // glass sits on the system's centre line
const doorW = p.doorWidth, doorH = p.doorHeight, doorT = 40;

/** U-channel along X, open up (floor) or down (head). */
function track(x0, len, z0, openUp) {
  const u = box(len, fd, trackH).subtract(box(len + 2, fd - 2 * trackT, trackH).translate(-1, trackT, openUp ? trackT : -trackT));
  return u.translate(x0, 0, z0);
}
/** Hollow rectangular mullion, sized to sit inside the tracks' channels. */
function mullion(x0, z0, h) {
  const md = fd - 2 * trackT;
  return box(mull, md, h).subtract(box(mull - 4, md - 4, h + 2).translate(2, 2, -1)).translate(x0, trackT, z0);
}

const gauge = 0.6; // steel thickness of studs and tracks
const wallT = p.studWidth + 2 * gauge + 2 * p.board; // separating wall: track steel plus a board each side
for (let i = 0; i < N; i++) {
  const name = `Office ${i + 1}`;
  const x0 = i * W + wallT / 2;
  const x1 = (i + 1) * W - wallT / 2;
  // The front fills between the walls; the door sits at the office's right end.
  const doorX0 = x1 - 2 * mull - doorW - 6; // hinge-side jamb; the opening is the leaf plus 3 mm each side
  parts.push(part(`${name} head track`, track(x0, x1 - x0, H - trackH, false), alu));
  parts.push(part(`${name} floor track`, track(x0, doorX0 + mull - x0, 0, true), alu));
  const z0 = trackT, h = H - 2 * trackT;
  parts.push(part(`${name} end mullion`, mullion(x0, z0, h), alu));
  parts.push(part(`${name} door jamb, hinge side`, mullion(doorX0, z0, h), alu));
  // No floor track across the doorway: the lock-side jamb stands on the floor.
  parts.push(part(`${name} door jamb, lock side`, mullion(x1 - mull, 0, H - trackT), alu));
  // Fixed glass between the end mullion and the door: one or two lites.
  const span = doorX0 - (x0 + mull);
  const lites = span > p.maxLite ? 2 : 1;
  const liteW = (span - (lites - 1) * mull) / lites;
  for (let k = 0; k < lites; k++) {
    const lx = x0 + mull + k * (liteW + mull);
    if (k > 0) parts.push(part(`${name} mullion ${k}`, mullion(lx - mull, z0, h), alu));
    // Glass stands on setting blocks 8 mm above the track base, 10 mm into the head.
    parts.push(part(`${name} glass ${k + 1}`, box(liteW - 6, gT, H - 2 * trackT - 16).translate(lx + 3, gy, trackT + 8), glass));
  }
  // Door head (transom bar), transom glass, door leaf and its pull.
  const dx0 = doorX0 + mull, dx1 = x1 - mull;
  const headZ = doorH + 20;
  parts.push(part(`${name} transom bar`, box(dx1 - dx0, fd, mull).subtract(box(dx1 - dx0 + 2, fd - 4, mull - 4).translate(-1, 2, 2)).translate(dx0, 0, headZ), alu));
  const tz0 = headZ + mull + 8, tz1 = H - trackT - 8;
  if (tz1 - tz0 > 50) parts.push(part(`${name} transom glass`, box(dx1 - dx0 - 6, gT, tz1 - tz0).translate(dx0 + 3, gy, tz0), glass));
  const leaf = box(dx1 - dx0 - 6, doorT, doorH - 8).translate(dx0 + 3, (fd - doorT) / 2, 10);
  // Vision slot in the leaf, glazed.
  const slotW = 120, slotH = doorH * 0.6;
  const slot = box(slotW, doorT + 2, slotH).translate(dx1 - 3 - 150 - slotW, (fd - doorT) / 2 - 1, 10 + (doorH - slotH) / 2);
  parts.push(part(`${name} door leaf`, leaf.subtract(slot), { material: "Oak", process: "cnc_router" }));
  parts.push(part(`${name} vision glass`, box(slotW - 4, 8, slotH - 4).translate(dx1 - 3 - 150 - slotW + 2, fd / 2 - 4, 10 + (doorH - slotH) / 2 + 2), glass));
  // Pull handle on the corridor side: two standoffs and a bar.
  const hx = dx1 - 3 - 70, hz = 950;
  const y0 = (fd - doorT) / 2; // corridor face of the leaf
  const standoff = (z) => cylinder(50, 9).rotate(90, 0, 0).translate(hx, y0, z);
  const bar = cylinder(700, 14).translate(hx, y0 - 50 - 14, hz - 350 + 0);
  parts.push(part(`${name} pull handle`, union([standoff(hz - 300), standoff(hz + 300), bar]), { material: "Stainless steel", process: "buy" }));
}

// ---- Separating walls ------------------------------------------------------
// At every office boundary (and both ends), from behind the glazed front to the façade.
const sw = p.studWidth, flange = 35;
/** C-stud along Z, web across the wall (X), opening toward +Y. */
function stud(xc, y, h, z0) {
  const c = box(sw, flange, h)
    .subtract(box(sw - 2 * gauge, flange - gauge, h + 2).translate(gauge, gauge, -1))
    .add(box(sw, gauge, h)); // web closes the C on the −Y side
  return c.translate(xc - sw / 2, y, z0);
}
/** U-track along Y at the floor or ceiling, holding the studs. */
function wallTrack(xc, y0, len, z0, openUp) {
  const u = box(sw + 2 * gauge, len, 30).subtract(box(sw, len + 2, 30).translate(gauge, -1, openUp ? gauge : -gauge));
  return u.translate(xc - sw / 2 - gauge, y0, z0);
}
const wy0 = fd + 5; // walls start just behind the front system
const wallLen = Dp - wy0;
for (let k = 0; k <= N; k++) {
  const xc = k === 0 ? wallT / 2 : k === N ? N * W - wallT / 2 : k * W;
  const name = `Wall ${k + 1}`;
  parts.push(part(`${name} floor track`, wallTrack(xc, wy0, wallLen, 0, true), steel));
  parts.push(part(`${name} ceiling track`, wallTrack(xc, wy0, wallLen, H - 30, false), steel));
  const studH = H - 2 * gauge - 10;
  let s = 1;
  for (let y = wy0 + 5; y + flange <= wy0 + wallLen - 5; y += p.studSpacing) {
    parts.push(part(`${name} stud ${s++}`, stud(xc, y, studH, gauge), steel));
  }
  // Boards both sides, full height, up to 1200 wide, screwed to the studs.
  for (const side of [-1, 1]) {
    const bx = side < 0 ? xc - sw / 2 - gauge - p.board : xc + sw / 2 + gauge;
    let y = wy0, b = 1;
    while (y < wy0 + wallLen - 1) {
      const w = Math.min(1200, wy0 + wallLen - y);
      parts.push(part(`${name} board ${side < 0 ? "A" : "B"}${b++}`, box(p.board, w - 1, H - 10).translate(bx, y, 5), board));
      y += w;
    }
  }
}

return parts;
