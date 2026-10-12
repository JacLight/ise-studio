// Forma stock library: helpers shared by every catalogue item. A family's
// program is this file followed by the family's own file (see catalogue.json).
//
// Conventions for every item:
//   - Millimetres, the item's main axis along +Z. Fasteners stand with the
//     bearing face of the head (or the nut's / washer's seat) on z = 0 and
//     the shank running down −Z, so a screw dropped into a hole at a face
//     sits on that face.
//   - Sizes are configurations: the program reads the row's values from `p`
//     and looks every other dimension up in the standard's table below.
//   - stock() carries what a buyer orders; part numbers follow the size.
//   - connector()s mark where things attach (axes are outward / along the
//     direction the mating part comes from).

/** ISO 261 coarse pitches (mm) by nominal diameter. */
const ISO_COARSE_PITCH = { 1.6: 0.35, 2: 0.4, 2.5: 0.45, 3: 0.5, 4: 0.7, 5: 0.8, 6: 1, 8: 1.25, 10: 1.5, 12: 1.75 };

function coarsePitch(d) {
  const pitch = ISO_COARSE_PITCH[d];
  if (pitch === undefined) throw new Error(`no ISO coarse pitch for M${d}`);
  return pitch;
}

/** Looks a nominal size up in a standard's table, with a clear message when it is not there. */
function lookup(table, key, what) {
  const row = table[key];
  if (row === undefined) throw new Error(`${what}: size ${key} is not in the table (sizes: ${Object.keys(table).join(", ")})`);
  return row;
}

/** "3" → "M3", 2.5 → "M2.5". */
const metric_ = (d) => `M${d}`;
/** 12 → "12", 2.5 → "2.5" (no trailing zeros). */
const num = (v) => String(+v.toFixed(3));

// ---- Threads ------------------------------------------------------------------

/**
 * ISO 68-1 thread by rings: the basic profile (60° flanks, P/8 crest flat at
 * the major radius, P/4 root flat at the minor radius) revolved as a stack of
 * rings instead of a helix. It looks threaded, weighs what a thread weighs
 * (within a few percent of the helical one) and stays exact in STEP (a
 * revolved polygon). Runs from z0 up to z1 along +Z.
 */
function cosmeticThread({ d, pitch, z0, z1, clearance = 0 }) {
  const R = d / 2 - clearance;
  const H = (Math.sqrt(3) / 2) * pitch;
  const r = R - (5 * H) / 8; // minor radius (external, basic)
  const n = Math.max(1, Math.floor((z1 - z0) / pitch));
  const z = z0 + (z1 - z0 - n * pitch) / 2;
  const pts = [[0, z0], [r, z0]];
  // One tooth per pitch: root flat P/4, flank, crest flat P/8, flank.
  for (let i = 0; i < n; i++) {
    const b = z + i * pitch;
    pts.push([r, b + pitch / 8], [R, b + pitch / 8 + (5 * pitch) / 16], [R, b + pitch / 8 + (7 * pitch) / 16], [r, b + (7 * pitch) / 8]);
  }
  pts.push([r, z1], [0, z1]);
  return polygon(pts).revolve(360);
}

/**
 * ISO 68-1 thread as the true helix: the trapezoidal tooth swept along the
 * helix (chords within `tolerance`), over a core at the minor radius. Heavy
 * (faceted in STEP): items use it only when their `threads` parameter is on.
 * `phase` is the height of a crest centre at angle 0.
 */
function helicalThread({ d, pitch, z0, z1, clearance = 0, phase = z0 }) {
  const majorRadius = d / 2 - clearance;
  const H = (Math.sqrt(3) / 2) * pitch;
  const minorRadius = majorRadius - (5 * H) / 8;
  const sink = 0.1 * pitch;
  const base = (3 * pitch) / 8 + sink * Math.tan(Math.PI / 6);
  const profile = [
    [minorRadius - sink, -base],
    [majorRadius, -pitch / 16],
    [majorRadius, pitch / 16],
    [minorRadius - sink, base],
  ];
  const perTurn = segmentsFor(majorRadius);
  const t0 = Math.floor((z0 - phase) / pitch) - 1;
  const t1 = Math.ceil((z1 - phase) / pitch) + 1;
  const steps = (t1 - t0) * perTurn;
  const positions = [];
  for (let s = 0; s <= steps; s++) {
    const turn = t0 + s / perTurn;
    const angle = 2 * Math.PI * turn;
    const zc = phase + turn * pitch;
    for (const [rr, dz] of profile) positions.push([rr * Math.cos(angle), rr * Math.sin(angle), zc + dz]);
  }
  const at = (s, j) => s * 4 + j;
  const triangles = [];
  for (let s = 0; s < steps; s++) {
    for (let j = 0; j < 4; j++) {
      const k = (j + 1) % 4;
      triangles.push([at(s, j), at(s + 1, j), at(s + 1, k)], [at(s, j), at(s + 1, k), at(s, k)]);
    }
  }
  triangles.push([at(0, 0), at(0, 1), at(0, 2)], [at(0, 0), at(0, 2), at(0, 3)]);
  triangles.push([at(steps, 0), at(steps, 2), at(steps, 1)], [at(steps, 0), at(steps, 3), at(steps, 2)]);
  const ridge = fromMesh(positions, triangles);
  const core = cylinder(z1 - z0, minorRadius).translate(0, 0, z0);
  const slab = cylinder(z1 - z0, majorRadius + 1).translate(0, 0, z0);
  return core.add(ridge).intersect(slab);
}

/**
 * An external thread from z0 to z1 (z1 > z0) with a 45° lead-in chamfer at
 * the `tip` end ("top" = z1, "bottom" = z0) down to just under the minor
 * diameter. Rings by default; the true helix when `real`.
 */
function externalThread({ d, pitch, z0, z1, real = false, tip = "bottom" }) {
  const rod = real ? helicalThread({ d, pitch, z0, z1 }) : cosmeticThread({ d, pitch, z0, z1 });
  const H = (Math.sqrt(3) / 2) * pitch;
  const minor = d / 2 - (5 * H) / 8;
  const R = d / 2 + 0.5;
  const c = R - (minor - 0.05); // axial length of the 45° cone
  const len = z1 - z0;
  const envelope = tip === "top"
    ? cylinder(len - c, R).add(cylinder(c, R, { top: minor - 0.05 }).translate(0, 0, len - c)).translate(0, 0, z0)
    : cylinder(c, minor - 0.05, { top: R }).add(cylinder(len - c, R).translate(0, 0, c)).translate(0, 0, z0);
  return rod.intersect(envelope);
}

/**
 * The tool that cuts an internal thread through z0…z1: the ISO profile
 * grown by `clearance`, as rings or the true helix. Subtract it.
 */
function internalThreadTool({ d, pitch, z0, z1, real = false, clearance = 0.02 }) {
  const ext = z1 - z0 + 2 * pitch;
  return real
    ? helicalThread({ d: d + 2 * clearance, pitch, z0: z0 - pitch, z1: z0 - pitch + ext, phase: z0 })
    : cosmeticThread({ d: d + 2 * clearance, pitch, z0: z0 - pitch, z1: z0 - pitch + ext });
}

// ---- Common features ------------------------------------------------------------

/** A hexagonal prism `af` across flats, `h` high, flats parallel to X. */
function hexPrism(af, h) {
  return regularPolygon(6, af / Math.sqrt(3), { rotate: 30 }).extrude(h);
}

/**
 * Hex socket (ISO 4762 etc.): `s` across flats, `t` deep to where the flats
 * end, with the 118° drill-point cone below, opening at z = top and going
 * down −Z. A small countersink at the mouth like a cold-headed socket.
 */
function hexSocket(s, t, top) {
  const e = s / Math.sqrt(3); // corner radius
  const cone = e / Math.tan(rad(59));
  const prism = hexPrism(s, t + 0.01).translate(0, 0, top - t);
  const point = cylinder(cone, 0.001, { top: e }).translate(0, 0, top - t - cone);
  const mouth = cylinder(e * 0.12 + 0.01, e, { top: e * 1.12 + 0.01 }).translate(0, 0, top - e * 0.12);
  return union(prism, point, mouth);
}

// ---- Suppliers --------------------------------------------------------------------

/** A search on a supplier's site for an item we hold no SKU for. */
function supplierSearch(supplier, query) {
  const q = encodeURIComponent(query);
  const url = {
    "McMaster-Carr": `https://www.mcmaster.com/products/${query.toLowerCase().replace(/[^a-z0-9.]+/g, "-").replace(/^-|-$/g, "")}/`,
    Misumi: `https://us.misumi-ec.com/vona2/result/?Keyword=${q}`,
    LCSC: `https://www.lcsc.com/search?q=${q}`,
    AliExpress: `https://www.aliexpress.com/w/wholesale-${q.replace(/%20/g, "-")}.html`,
    Amazon: `https://www.amazon.com/s?k=${q}`,
    "Bolt Depot": `https://www.boltdepot.com/Search.aspx?q=${q}`,
    "Accu": `https://accu.co.uk/search?q=${q}`,
    "VXB": `https://www.vxb.com/search?q=${q}`,
    "CNC Kitchen": `https://cnckitchen.store/search?q=${q}`,
    "Ruthex": `https://www.ruthex.de/search?sSearch=${q}`,
    "StepperOnline": `https://www.omc-stepperonline.com/search?keyword=${q}`,
    "Metal Supermarkets": `https://www.metalsupermarkets.com/?s=${q}`,
  }[supplier];
  if (!url) throw new Error(`no search link for ${supplier}`);
  return { supplier, url };
}

/** Rounds a price to cents. */
const cents = (v) => Math.round(v * 100) / 100;
