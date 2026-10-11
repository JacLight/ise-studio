// Shared constants and helpers for the robot arm's printed components.
//
// Every component is its own part, drawn in its own coordinates (origin on
// its pivot axis or its seat), and declares connectors where other parts
// attach: servo pockets, horn faces, bearing seats, insert and screw holes.
// The assembly puts them together with joints on those connectors only.
// `finish()` lays the part in its print orientation on z = 0 (its 3MF opens
// ready to slice) and moves its connectors with it.
//
// The numbers below are the stock library's (library/stock), so pockets and
// holes match the true bought parts.

const C = 0.25; // servo pocket clearance per side (slide fit, profile 0.2–0.3 mm)
const PRESS = 0.1; // press fit interference on a printed bore (profile)

// Hobby servos (servo_hobby.js): case L × W from x = −off along the case, tabs'
// top face at tabTop above the case bottom, holes through the tabs, gear-top boss.
const SERVO = {
  MG996R: { L: 40.7, W: 19.7, off: 10, tabL: 54.5, tabT: 2.5, tabTop: 29.1, caseTop: 36, boss: 39.5, holes: [[-14.4, -5], [-14.4, 5], [35.1, -5], [35.1, 5]], hornH: 6.5 },
  MG90S: { L: 22.8, W: 12.2, off: 6, tabL: 32.5, tabT: 2.5, tabTop: 18.1, caseTop: 21.6, boss: 25.4, holes: [[-8.5, 0], [19.3, 0]], hornH: 5.6 },
};
/** From the mounting face (the tabs' top) to the horn's face, and to the case top. */
const hornFace = (s) => s.boss - s.tabTop + 0.3 + s.hornH; // MG996R 17.2, MG90S 13.2
const caseAbove = (s) => s.caseTop - s.tabTop; // MG996R 6.9, MG90S 3.5

// Aluminium disc horns (servo_horn.js): tapped holes on a circle at 45° + k·90°.
// A horn sits in a recess of the part it drives: `recess` deep, `floor` behind its face.
const HORN = {
  MG996R: { pcd: 16, disc: 24, tap: 3, height: 6.5, recess: 6, floor: 7, centre: 6.5 }, // screws counterbored 3.5 into the floor
  MG90S: { pcd: 11, disc: 16, tap: 2, height: 5.6, recess: 5, floor: 3, centre: 4.5 },
};
/** Gap between a servo housing's face (case top flush) and the driven part's face. */
const hornGap = (model) => hornFace(SERVO[model]) - caseAbove(SERVO[model]) - HORN[model].recess; // 4.3 / 4.7

// Heat-set inserts (heatset_insert.js): printed hole Ø and depth (length + 1).
const INSERT = { M2: { hole: 3.2, depth: 5 }, M3: { hole: 4.0, depth: 6.7 }, "M3 short": { hole: 4.0, depth: 4 } };
const CLEAR = { M2: 2.4, M3: 3.4 }; // ISO 273 medium
const PILOT = { M2: 1.7 }; // M2 self-tapping into PLA (0.85 d)
const CBORE = { M3: 6.5, M2: 4.6 }; // counterbores for socket heads

// Interfaces between components (mm), shared so mating parts agree.
const ARM = {
  L1: 126, // shoulder to elbow (the two big servos' leads end to end inside)
  L2: 100, // elbow to wrist
  uaHalf: 22, // upper arm: faces at y = ±22 (its mid-plane y = 0)
  uaSplit: -10, // upper arm halves meet at y = −10
  uaWall: 6.9, // the servos' wall (left), case tops flush with the left face
  uaR: [24, 23], // shoulder and elbow end radii (the servos' tab corners need 20.5 inside)
  faFaces: [-20, 16], // forearm body faces (in the upper arm's y)
  faSplit: -2,
  bearingGap: 1, // a bearing cheek's inner face off the turning part's face
  yokeAxis: 34, // shoulder axis above the yoke's foot
  yokeX: -8, // the yoke's axis behind the turntable's centre
  yokeBolts: [[-24, -14], [24, -14], [-24, 14], [24, 14]], // yoke foot screws, about the axis
};
/** y of the horn's face and of the bearing cheek's inner face, for a part of half-width h (y = 0 its mid-plane) driven on −y. */
const hornY = (model, faceY) => faceY - hornGap(model) - HORN[model].recess; // the horn's face
// Upper arm: horn face at −32.3, cheek inner face −26.3, outer −36.3; bearing cheek 23…32.

// ---- Frames -----------------------------------------------------------------------

const sub3 = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add3 = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul3 = (a, k) => [a[0] * k, a[1] * k, a[2] * k];
const cross3 = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];

/** Euler angles (degrees about X, then Y, then Z, as Solid.rotate) of the frame with these axes. */
function eulerOf(ex, ey, ez) {
  const r = [[ex[0], ey[0], ez[0]], [ex[1], ey[1], ez[1]], [ex[2], ey[2], ez[2]]];
  const sy = Math.max(-1, Math.min(1, -r[2][0]));
  let x, y, z;
  if (Math.abs(sy) < 1 - 1e-12) {
    x = Math.atan2(r[2][1], r[2][2]);
    y = Math.asin(sy);
    z = Math.atan2(r[1][0], r[0][0]);
  } else {
    x = 0;
    y = (sy > 0 ? 1 : -1) * Math.PI / 2;
    z = Math.atan2(-r[0][1], r[1][1]);
  }
  const d = (a) => (Math.abs(a) < 1e-12 ? 0 : (a * 180) / Math.PI);
  return [d(x), d(y), d(z)];
}

/** Rotates a vector as Solid.rotate(x, y, z) does. */
function rotateVec(v, [ax, ay, az]) {
  const [cx, sx, cy, sy, cz, sz] = [Math.cos(rad(ax)), Math.sin(rad(ax)), Math.cos(rad(ay)), Math.sin(rad(ay)), Math.cos(rad(az)), Math.sin(rad(az))];
  let [x, y, z] = v;
  [y, z] = [y * cx - z * sx, y * sx + z * cx];
  [x, z] = [x * cy + z * sy, -x * sy + z * cy];
  [x, y] = [x * cz - y * sz, x * sz + y * cz];
  return [x, y, z];
}

/** A frame in the part: origin `o`, local x along `ex`, local z along `ez`. */
function frame(o, ex, ez) {
  const ey = cross3(ez, ex);
  const e = eulerOf(ex, ey, ez);
  return {
    o, ex, ey, ez,
    place: (s) => s.rotate(e[0], e[1], e[2]).translate(o[0], o[1], o[2]),
    pt: (p) => add3(o, add3(add3(mul3(ex, p[0]), mul3(ey, p[1])), mul3(ez, p[2]))),
    dir: (d) => add3(add3(mul3(ex, d[0]), mul3(ey, d[1])), mul3(ez, d[2])),
  };
}

/** A profile in the part's XZ plane (u along x, v along z) extruded across y from y0 to y1. */
function xz(profile, y0, y1) {
  return frame([0, y1, 0], [1, 0, 0], [0, -1, 0]).place(profile.extrude(y1 - y0));
}

/** A profile in the XY plane extruded from z0 to z1. */
function xy(profile, z0, z1) {
  return profile.extrude(z1 - z0).translate(0, 0, z0);
}

/** Rounds every corner of a profile: convex by `r`, concave by `rc` (default r). */
function rounded(profile, r, rc = r) {
  let p = profile;
  if (rc > 0) p = p.offset(rc).offset(-rc);
  if (r > 0) p = p.offset(-r).offset(r);
  return p;
}

/** A rectangle profile from corner to corner. */
function rectP(x0, y0, x1, y1) {
  return polygon([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
}

// ---- Connectors and the print layout ---------------------------------------------------

const __conns = [];
/** A connector in the part's design coordinates; emitted by finish() in the print layout. */
function conn(name, at, axis, x) {
  __conns.push({ name, at, axis, x });
}

/**
 * Lays the part in its print orientation (`rot`, degrees about X, Y, Z) on z = 0,
 * centred on the bed's origin, declares its connectors there, reports its print
 * size, and returns the part list. `orientation` says how it prints.
 */
function finish(name, solid, material, rot, orientation) {
  const r = solid.rotate(rot[0], rot[1], rot[2]);
  const b = r.bounds;
  const shift = [-(b.min[0] + b.max[0]) / 2, -(b.min[1] + b.max[1]) / 2, -b.min[2]];
  const out = r.translate(shift[0], shift[1], shift[2]);
  for (const c of __conns) {
    const at = add3(rotateVec(c.at, rot), shift);
    const axis = rotateVec(c.axis, rot);
    const x = c.x ? rotateVec(c.x, rot) : undefined;
    connector(c.name, { at, axis, x });
  }
  const size = [b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]];
  metric("Print size X", Math.round(size[0] * 10) / 10, "mm");
  metric("Print size Y", Math.round(size[1] * 10) / 10, "mm");
  metric("Print height", Math.round(size[2] * 10) / 10, "mm");
  if (size[0] > 256 || size[1] > 256 || size[2] > 256) throw new Error(`${name} (${size.map((v) => v.toFixed(1)).join(" × ")} mm) does not fit the X1's 256 mm bed`);
  return [part(name, out, { material, process: "3d_print" })];
}

// ---- Features ---------------------------------------------------------------------------

/** A cylinder from a to b (any direction), radius r. */
function rod(a, b, r) {
  const d = sub3(b, a);
  const len = Math.hypot(d[0], d[1], d[2]);
  const ez = mul3(d, 1 / len);
  const ex = Math.abs(ez[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
  const exo = sub3(ex, mul3(ez, ex[0] * ez[0] + ex[1] * ez[1] + ex[2] * ez[2]));
  return frame(a, mul3(exo, 1 / Math.hypot(...exo)), ez).place(cylinder(len, r));
}

/** A box between two corners. */
function boxAt(lo, hi) {
  return box(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]).translate(lo[0], lo[1], lo[2]);
}

/**
 * A servo mount in the servo's frame `f` (origin on the shaft axis in the
 * mounting face — the tabs' top — local z toward the output, local x along the
 * case): the case's opening through a wall `t` thick (case top flush with the
 * far face when t = caseAbove), the tab-screw holes (`kind` "M2 insert" or
 * "M2 pilot", `depth` deep, through by default), and connectors: "<prefix>
 * mount" (where the servo's Hole 1 seats, facing it) and "<prefix> hole k"
 * (each insert's seat).
 */
function servoMount(model, f, t, kind, prefix, depth = t + 1) {
  const s = SERVO[model];
  const cut = [f.place(boxAt([-s.off - C, -s.W / 2 - C, -1], [s.L - s.off + C, s.W / 2 + C, t + 1]))];
  s.holes.forEach(([hx, hy], i) => {
    const d = kind === "M2 insert" ? INSERT.M2.hole : PILOT.M2;
    cut.push(f.place(cylinder(depth + 1, d / 2).translate(hx, hy, -1)));
    conn(`${prefix} hole ${i + 1}`, f.pt([hx, hy, 0]), f.dir([0, 0, -1]), f.dir([1, 0, 0]));
  });
  const [h1x, h1y] = s.holes[0];
  conn(`${prefix} mount`, f.pt([h1x, h1y, 0]), f.dir([0, 0, -1]), f.dir([1, 0, 0]));
  return union(cut);
}

/**
 * The seat for a disc horn in the part it drives, in the horn's frame `f`
 * (origin on the horn's face, local z from the face into the part — away from
 * the servo — local x along the servo's case): the recess the horn sits in
 * (`HORN.recess` toward the servo), the screw holes through the floor behind
 * it with their heads' seats on the outside (`seat` = the floor's thickness,
 * counterbored `cb` deep), and the centre hole for the horn screw and its
 * key. Connectors "<prefix> horn" (facing the horn) and "<prefix> screw k".
 */
function hornSeat(model, f, prefix, { seat = HORN[model].floor, recess = HORN[model].recess, cb = 0 } = {}) {
  const h = HORN[model];
  const clear = CLEAR[`M${h.tap}`];
  const cut = [];
  if (recess > 0) cut.push(f.place(cylinder(recess + 0.01, (h.disc + 1) / 2).translate(0, 0, -recess)));
  cut.push(f.place(cylinder(seat + 2, h.centre / 2).translate(0, 0, -1)));
  for (let i = 0; i < 4; i++) {
    const a = rad(45 + 90 * i);
    const [x, y] = [(h.pcd / 2) * Math.cos(a), (h.pcd / 2) * Math.sin(a)];
    cut.push(f.place(cylinder(seat + 2, clear / 2).translate(x, y, -1)));
    if (cb > 0) cut.push(f.place(cylinder(cb + 1, CBORE[`M${h.tap}`] / 2).translate(x, y, seat - cb)));
    conn(`${prefix} screw ${i + 1}`, f.pt([x, y, seat - cb]), f.dir([0, 0, 1]), f.dir([1, 0, 0]));
  }
  conn(`${prefix} horn`, f.pt([0, 0, 0]), f.dir([0, 0, -1]), f.dir([1, 0, 0]));
  return union(cut);
}

/** A heat-set insert hole at p, `axis` pointing out of the face; its connector. */
function insertHole(size, p, axis, name, extra = 0) {
  const ins = INSERT[size];
  conn(name, p, axis);
  return rod(add3(p, mul3(axis, 0.5)), sub3(p, mul3(axis, ins.depth + extra)), ins.hole / 2);
}

/**
 * A screw's clearance hole from the face at p (axis out of the face) `len`
 * deep, counterbored `cb` deep for the head; connector at the head's seat.
 */
function clearHole(size, p, axis, len, name, cb = 0) {
  conn(name, sub3(p, mul3(axis, cb)), axis);
  const cut = [rod(add3(p, mul3(axis, 0.5)), sub3(p, mul3(axis, len + 0.5)), CLEAR[size] / 2)];
  if (cb > 0) cut.push(rod(add3(p, mul3(axis, 1)), sub3(p, mul3(axis, cb)), CBORE[size] / 2));
  return union(cut);
}

/** A press-fit hole for a dowel pin of diameter d, `depth` deep from the face at p; connector at its bottom (the pin's end). */
function pinHole(d, p, axis, depth, name) {
  conn(name, sub3(p, mul3(axis, depth)), axis);
  return rod(add3(p, mul3(axis, 0.5)), sub3(p, mul3(axis, depth)), (d - PRESS) / 2);
}

/** A bearing seat (press fit), from the face at p into the part along -axis, with a lip behind it; connector on the face. */
function bearingSeat(D, B, p, axis, lipT, lipD, name) {
  conn(name, p, axis);
  return union(
    rod(add3(p, mul3(axis, 0.5)), sub3(p, mul3(axis, B)), (D - PRESS) / 2),
    rod(add3(p, mul3(axis, 0.5)), sub3(p, mul3(axis, B + lipT + 1)), lipD / 2),
  );
}

/** The edges of a solid lying in the plane coord[k] = v (for chamfers and fillets on one face). */
const onPlane = (k, v) => (e) => e.points.every((pt) => Math.abs(pt[k] - v) < 1e-3);
