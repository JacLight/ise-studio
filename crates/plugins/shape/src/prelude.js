// The vocabulary shape programs are written in. Programs are plain
// JavaScript function bodies: they read parameters from `p` (millimeters and
// degrees) and return a Solid. Anything can be built from these pieces; when
// a shape needs math the library does not have, compute the points yourself
// and use fromMesh() or polygon().
//
// Deterministic by construction: there is no clock and no unseeded random
// (use rng(seed)), no I/O, and curve resolution is explicit or derived from
// `tolerance`.
//
// Accuracy is chosen by whoever evaluates the program, not by the program:
// `tolerance` (mm) is the largest distance any approximated curve may stray
// from the true formula. The same program renders coarse for the viewport,
// fine for requirement checks and exact for export. Derive every curve's
// resolution from it (segmentsFor) so the shape honours the request.
"use strict";

const __n = globalThis.__native;
delete globalThis.__native;
const tolerance = globalThis.__tolerance;
delete globalThis.__tolerance;
const __inputs = JSON.parse(globalThis.__inputs || "{}");
const __sketches = __inputs.sketches ?? {};
const __finishes = __inputs.finishes ?? [];
delete globalThis.__inputs;
delete globalThis.Date;
Math.random = () => {
  throw new Error("Math.random is not available: shape programs must be deterministic. Use rng(seed).");
};

/**
 * Segments for a full circle of this radius so that no chord strays more than
 * `tolerance` from the true arc (sagitta r·(1 − cos(π/n)) ≤ tolerance).
 */
function segmentsFor(radius) {
  const r = Math.abs(radius);
  if (r <= tolerance) return 8;
  return Math.max(8, Math.min(4096, Math.ceil(Math.PI / Math.acos(1 - tolerance / r))));
}

const vec3 = (x, y, z) => (Array.isArray(x) ? [x[0] ?? 0, x[1] ?? 0, x[2] ?? 0] : [x ?? 0, y ?? 0, z ?? 0]);
const asSolid = (s) => {
  if (!(s instanceof Solid)) throw new TypeError("expected a Solid");
  return s;
};
const asProfile = (s) => {
  if (!(s instanceof Profile)) throw new TypeError("expected a Profile");
  return s;
};

class Solid {
  constructor(h) {
    this.h = h;
    Object.freeze(this);
  }
  add(...others) {
    return others.flat().reduce((a, b) => new Solid(__n.sbool("union", a.h, asSolid(b).h)), this);
  }
  subtract(...others) {
    return others.flat().reduce((a, b) => new Solid(__n.sbool("difference", a.h, asSolid(b).h)), this);
  }
  intersect(...others) {
    return others.flat().reduce((a, b) => new Solid(__n.sbool("intersection", a.h, asSolid(b).h)), this);
  }
  translate(x, y, z) {
    return new Solid(__n.sop("translate", this.h, vec3(x, y, z)));
  }
  /** Degrees about X, then Y, then Z, around the origin. */
  rotate(x, y, z) {
    return new Solid(__n.sop("rotate", this.h, vec3(x, y, z)));
  }
  scale(x, y = x, z = x) {
    return new Solid(__n.sop("scale", this.h, Array.isArray(x) ? vec3(x) : [x, y, z]));
  }
  /** Mirror across the plane through the origin with this normal. */
  mirror(nx, ny, nz) {
    return new Solid(__n.sop("mirror", this.h, vec3(nx, ny, nz)));
  }
  hull() {
    return new Solid(__n.sop("hull", this.h, []));
  }
  get volume() {
    return __n.squery("volume", this.h)[0];
  }
  get area() {
    return __n.squery("area", this.h)[0];
  }
  get genus() {
    return __n.squery("genus", this.h)[0];
  }
  get bounds() {
    const b = __n.squery("bounds", this.h);
    return { min: [b[0], b[1], b[2]], max: [b[3], b[4], b[5]] };
  }
  /** Smallest distance to another solid, if closer than `search` mm. */
  minGap(other, search = 10) {
    return __n.minGap(this.h, asSolid(other).h, search);
  }
  /**
   * Every edge a person would see (seams of curved faces left out):
   * { index, kind: "line" | "circle" | "curve", points, mid, and for lines
   * dir and length, for circles center, axis and radius }.
   */
  get edges() {
    return JSON.parse(__n.edges(this.h))
      .map((e, index) => Object.freeze({ index, ...e, mid: e.points.length ? __midOf(e.points) : null }))
      .filter((e) => e.kind !== "seam");
  }
  /**
   * Rounds edges with true arcs: every edge, or those `which` picks (see
   * selectEdges). Exact: the view and the STEP file both get the real surface.
   *   box(60, 40, 10).fillet(3, { parallel: "z" })        // the four upright edges
   *   plate.fillet(1, { near: [[0, 0, p.t], [p.w, 0, p.t]] })
   */
  fillet(radius, which) {
    return __blend(this, "fillet", radius, which);
  }
  /** Bevels edges back `distance` mm along each face; picks edges like fillet. */
  chamfer(distance, which) {
    return __blend(this, "chamfer", distance, which);
  }
  /**
   * Every face: { index, flat, normal (outward, flat faces only), center, area }.
   */
  get faces() {
    return JSON.parse(__n.faces(this.h)).map((f, index) => Object.freeze({ index, flat: !!f.normal, ...f }));
  }
  /**
   * Pushes a flat face out by `distance` mm (adding material), or pulls it in
   * with a negative distance (cutting), along its normal. `which` picks the
   * face: { normal: [x, y, z], near: [x, y, z] } (the flat face facing that
   * way nearest the point), or a face from solid.faces. Exact.
   *   box(60, 40, 10).push(5, { normal: [0, 0, 1] })   // 15 mm tall now
   */
  push(distance, which) {
    if (typeof distance !== "number" || !Number.isFinite(distance)) throw new Error("push: the distance must be a number of mm");
    if (distance === 0) return this;
    const face = selectFace(this, which);
    return new Solid(__n.pushFace(this.h, face.index, distance));
  }
}

/**
 * Picks one flat face: { normal, near } (facing that way, nearest the point),
 * a face from solid.faces, or { at, dir } as picked in the view (where it
 * sits relative to the bounds, so it follows when the part is resized).
 */
function selectFace(solid, which) {
  if (which && typeof which.index === "number") return which;
  const flat = asSolid(solid).faces.filter((f) => f.flat);
  if (!flat.length) throw new Error("push: the shape has no flat face");
  let normal = which && (which.normal ?? which.dir);
  let near = which && which.near;
  if (which && which.at) {
    const { min, max } = solid.bounds;
    near = which.at.map((f, k) => min[k] + f * (max[k] - min[k]));
  }
  let pool = flat;
  if (normal) {
    const n = __unit(vec3(normal));
    const facing = flat.filter((f) => __dot(f.normal, n) > 0.999);
    if (!facing.length) throw new Error(`push: no flat face faces [${n.map((v) => v.toFixed(2)).join(", ")}]`);
    pool = facing;
  }
  if (!near) return pool.reduce((a, b) => (b.area > a.area ? b : a));
  const q = vec3(near);
  // Nearest by its plane first (the face itself), then by its middle.
  const score = (f) => Math.abs(__dot(f.normal, [q[0] - f.center[0], q[1] - f.center[1], q[2] - f.center[2]])) * 10 + __dist(q, f.center);
  return pool.reduce((a, b) => (score(b) < score(a) ? b : a));
}

function __midOf(points) {
  // The point halfway along the polyline.
  let total = 0;
  for (let i = 1; i < points.length; i++) total += __dist(points[i - 1], points[i]);
  let half = total / 2;
  for (let i = 1; i < points.length; i++) {
    const d = __dist(points[i - 1], points[i]);
    if (d >= half && d > 0) return points[i - 1].map((v, k) => v + ((points[i][k] - v) * half) / d);
    half -= d;
  }
  return points[0];
}
const __dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
const __dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const __unit = (v) => {
  const l = Math.hypot(...v) || 1;
  return v.map((x) => x / l);
};
const __axis = (a) => (typeof a === "string" ? { x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] }[a.toLowerCase()] : __unit(vec3(a)));

/** Distance from a point to an edge (along its polyline). */
function edgeDistance(edge, point) {
  const q = vec3(point);
  const pts = edge.points;
  if (pts.length === 1) return __dist(pts[0], q);
  let best = Infinity;
  for (let i = 1; i < pts.length; i++) {
    const [a, b] = [pts[i - 1], pts[i]];
    const ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    const len2 = __dot(ab, ab);
    const t = len2 > 0 ? Math.max(0, Math.min(1, __dot([q[0] - a[0], q[1] - a[1], q[2] - a[2]], ab) / len2)) : 0;
    best = Math.min(best, __dist(q, [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]));
  }
  return best;
}

/**
 * Picks edges of a solid. `which` is any of:
 *   undefined                       every edge
 *   (edge) => boolean               a filter over solid.edges
 *   [edge, …]                       edges from solid.edges
 *   [[x, y, z], …]                  the edge nearest each point
 *   { kind, parallel, within, near } all must hold: kind "line" | "circle" |
 *     "curve"; parallel "x" | "y" | "z" | [dx, dy, dz] (lines along it,
 *     circles around it); within { min, max } (whole edge inside the box);
 *     near a point or points (then the nearest such edge to each).
 */
function selectEdges(solid, which) {
  const all = asSolid(solid).edges;
  if (which === undefined || which === null) return all;
  if (typeof which === "function") return all.filter(which);
  if (Array.isArray(which)) {
    if (which.every((w) => w && typeof w.index === "number")) return which;
    return selectEdges(solid, { near: which });
  }
  let list = all;
  if (which.kind) list = list.filter((e) => e.kind === which.kind);
  if (which.parallel !== undefined) {
    const d = __axis(which.parallel);
    list = list.filter((e) => (e.kind === "line" && Math.abs(__dot(e.dir, d)) > 0.9999) || (e.kind === "circle" && Math.abs(__dot(e.axis, d)) > 0.9999));
  }
  if (which.within) {
    const [lo, hi] = [vec3(which.within.min), vec3(which.within.max)];
    const eps = 1e-6 + tolerance;
    list = list.filter((e) => e.points.every((p) => p.every((v, k) => v >= lo[k] - eps && v <= hi[k] + eps)));
  }
  if (which.picks) list = which.picks.map((pick) => __resolvePick(solid, list, pick));
  if (which.near !== undefined) {
    const pts = typeof which.near[0] === "number" ? [which.near] : which.near;
    list = pts.map((q) => {
      let best = null;
      for (const e of list) if (!best || edgeDistance(e, q) < edgeDistance(best, q)) best = e;
      if (!best) throw new Error(`no edge near [${vec3(q).join(", ")}]`);
      return best;
    });
  }
  return [...new Map(list.map((e) => [e.index, e])).values()];
}

/**
 * An edge picked in the view, found again on the shape as it is now: the
 * same kind and direction, nearest the same place relative to the bounds
 * (so it follows when the part is resized).
 */
function __resolvePick(solid, edges, pick) {
  const { min, max } = solid.bounds;
  const at = pick.at.map((f, k) => min[k] + f * (max[k] - min[k]));
  const same = edges.filter((e) => e.kind === pick.kind && (!pick.dir || Math.abs(__dot(e.kind === "circle" ? e.axis : e.dir ?? [0, 0, 0], pick.dir)) > 0.999));
  const pool = same.length ? same : edges;
  const score = (e) => (e.kind === "circle" && pick.kind === "circle" ? __dist(e.center, at) + Math.abs(e.radius - (pick.radius ?? e.radius)) : edgeDistance(e, at));
  let best = null;
  for (const e of pool) if (!best || score(e) < score(best)) best = e;
  if (!best) throw new Error("a picked edge is no longer on the shape");
  return best;
}

function __blend(solid, op, size, which) {
  if (typeof size !== "number" || !Number.isFinite(size) || size <= 0) throw new Error(`${op}: the ${op === "fillet" ? "radius" : "distance"} must be a positive number of mm`);
  const chosen = selectEdges(solid, which);
  if (!chosen.length) throw new Error(`${op}: no edge matches`);
  return new Solid(__n.blend(op, solid.h, size, chosen.map((e) => e.index)));
}

class Profile {
  constructor(h) {
    this.h = h;
    Object.freeze(this);
  }
  add(...others) {
    return others.flat().reduce((a, b) => new Profile(__n.pbool("union", a.h, asProfile(b).h)), this);
  }
  subtract(...others) {
    return others.flat().reduce((a, b) => new Profile(__n.pbool("difference", a.h, asProfile(b).h)), this);
  }
  intersect(...others) {
    return others.flat().reduce((a, b) => new Profile(__n.pbool("intersection", a.h, asProfile(b).h)), this);
  }
  translate(x = 0, y = 0) {
    return new Profile(__n.pop("translate", this.h, Array.isArray(x) ? [x[0], x[1]] : [x, y]));
  }
  rotate(degrees) {
    return new Profile(__n.pop("rotate", this.h, [degrees]));
  }
  scale(x, y = x) {
    return new Profile(__n.pop("scale", this.h, [x, y]));
  }
  mirror(ax, ay) {
    return new Profile(__n.pop("mirror", this.h, [ax, ay]));
  }
  hull() {
    return new Profile(__n.pop("hull", this.h, []));
  }
  /** Grow (positive) or shrink (negative) the outline. join: "round" | "square" | "miter" | "bevel". */
  offset(delta, { join = "round", segments = segmentsFor(Math.abs(delta)) } = {}) {
    const j = { square: 0, round: 1, miter: 2, bevel: 3 }[join];
    if (j === undefined) throw new Error(`unknown join ${join}`);
    return new Profile(__n.pop("offset", this.h, [delta, j, segments]));
  }
  /** Extrude along +Z. `twist` in degrees over the height; `scaleTop` tapers. */
  extrude(height, { twist = 0, slices = 0, scaleTop = 1 } = {}) {
    const [sx, sy] = Array.isArray(scaleTop) ? scaleTop : [scaleTop, scaleTop];
    return new Solid(__n.extrude(this.h, [height, slices, twist, sx, sy]));
  }
  /** Revolve the profile (x = radius, y = height) around the Z axis. */
  revolve(degrees = 360, { segments = 0 } = {}) {
    const [, , maxX] = __n.pquery("bounds", this.h);
    return new Solid(__n.revolve(this.h, [segments || segmentsFor(maxX), degrees]));
  }
  get area() {
    return __n.pquery("area", this.h)[0];
  }
  get bounds() {
    const b = __n.pquery("bounds", this.h);
    return { min: [b[0], b[1]], max: [b[2], b[3]] };
  }
}

// ---- Solids ----------------------------------------------------------------

function box(x, y, z, { center = false } = {}) {
  return new Solid(__n.solid("cube", [x, y, z, center ? 1 : 0]));
}

/** Along +Z from z = 0 (or centered). `top` radius makes a cone or frustum. */
function cylinder(height, radius, { top = radius, segments = segmentsFor(Math.max(radius, top)), center = false } = {}) {
  return new Solid(__n.solid("cylinder", [height, radius, top, segments, center ? 1 : 0]));
}

function sphere(radius, { segments = segmentsFor(radius) } = {}) {
  return new Solid(__n.solid("sphere", [radius, segments]));
}

/**
 * A solid from your own computed geometry: positions as [[x,y,z], ...] (or
 * flat) and triangles as [[a,b,c], ...] (or flat), counter-clockwise seen from
 * outside. It must be closed and manifold.
 */
function fromMesh(positions, triangles) {
  return new Solid(__n.fromMesh(positions.flat(), triangles.flat()));
}

/** Convex hull of points [[x,y,z], ...]. */
function hullPoints(points) {
  return new Solid(__n.hullPoints(points.flat()));
}

/** Union of many solids at once: much faster than adding them one by one. */
function union(...solids) {
  const list = solids.flat();
  if (!list.length) throw new Error("union() needs at least one solid");
  if (list.length === 1) return list[0];
  return new Solid(__n.unionAll(list.map((s) => asSolid(s).h)));
}

// ---- Assemblies ----------------------------------------------------------------

/**
 * Processes a part can be made by. Exports follow from it: sheet processes get
 * a layered DXF cut file, everything gets an STL.
 */
const PROCESSES = [
  "3d_print", // FDM / SLA / SLS
  "cnc_mill", // 3- or 5-axis milling
  "lathe", // turned parts
  "cnc_router", // sheet goods: plywood, MDF, HPL
  "laser", // laser or waterjet cut plate
  "sheet_metal", // cut and bent
  "weld", // welded fabrication
  "cast", // casting, or concrete cast in place
  "extrusion", // cut-to-length profile (aluminium, steel section)
  "glazing", // glass units
  "buy", // purchased off the shelf
];

class Part {
  constructor(name, solid, material, process) {
    this.name = name;
    this.solid = solid;
    this.material = material;
    this.process = process;
    Object.freeze(this);
  }
}

/**
 * One fabricable item of an assembly. Return a list of these instead of a
 * single Solid; every part stays separate (its own mesh, mass, cut list line
 * and fabrication file). Names must be unique within the assembly.
 *   part("Left side", panel, { material: "Birch ply", process: "cnc_router" })
 */
function part(name, solid, { material, process } = {}) {
  if (typeof name !== "string" || !name.trim()) throw new TypeError("part() needs a name");
  asSolid(solid);
  if (process !== undefined && !PROCESSES.includes(process)) {
    throw new Error(`part ${name}: unknown process "${process}" (use one of ${PROCESSES.join(", ")})`);
  }
  if (material !== undefined && typeof material !== "string") throw new TypeError(`part ${name}: material is a name`);
  return new Part(name.trim(), solid, material, process);
}

const __metrics = {};
/**
 * Reports a measured fact about the design, shown in schedules and summed
 * up through assemblies: metric("Net area", m2, "m²"), metric("Keys", 4).
 */
function metric(name, value, unit = "") {
  if (typeof name !== "string" || !name.trim()) throw new TypeError("metric() needs a name");
  if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError(`metric ${name}: value must be a finite number`);
  __metrics[name.trim()] = { value: (__metrics[name.trim()]?.value ?? 0) + value, unit };
}

/** Edge finishes picked in the view, applied in order after the program. */
function __finish(out, p) {
  for (const f of __finishes) {
    const size = p[f.key];
    if (typeof size !== "number") throw new Error(`edge finish: the shape has no parameter “${f.key}”`);
    const apply = (s) => {
      if (f.kind === "push") return f.edges.reduce((acc, pick) => acc.push(size, pick), s);
      return f.kind === "chamfer" ? s.chamfer(size, { picks: f.edges }) : s.fillet(size, { picks: f.edges });
    };
    if (out instanceof Solid) out = apply(out);
    else {
      const list = Array.isArray(out) ? out.flat(Infinity) : [];
      if (!list.some((x) => x instanceof Part && x.name === f.part)) throw new Error(`edge finish: there is no part “${f.part}”`);
      out = list.map((x) => (x instanceof Part && x.name === f.part ? new Part(x.name, apply(x.solid), x.material, x.process) : x));
    }
  }
  return out;
}

// ---- Mechanisms ------------------------------------------------------------------

const __mechanism = { bodies: [], meshes: [], joints: [], drive: null, couplings: [] };

/**
 * Parts that turn together about an axis, relative to the parts that stay put
 * (the housing, the frame): hinge(["Input pinion", "Input shaft"], { at: [0, 0, 0], axis: [0, 0, 1] }).
 * The engine can then turn them (gear checks, motion).
 */
function hinge(parts, { at = [0, 0, 0], axis = [0, 0, 1] } = {}) {
  const names = [parts].flat();
  if (!names.length || !names.every((n) => typeof n === "string" && n.trim())) throw new TypeError("hinge() needs part names");
  __mechanism.bodies.push({ parts: names.map((n) => n.trim()), at: vec3(at), axis: __unit(vec3(axis)) });
}

/**
 * Two gears in mesh, the first driving the second: the parts each gear is on,
 * and the gears' definition. teeth: [driver, driven]; module and width in mm.
 *   mesh("Input pinion", "Stage 1 wheel", { module: 1.5, teeth: [18, 54], width: 10 })
 * `internal: true` for a pinion inside a ring gear.
 */
function mesh(driver, driven, { module, teeth, pressureAngle = 20, width, backlash = 0, internal = false } = {}) {
  for (const [what, v] of [["module", module], ["width", width]]) {
    if (typeof v !== "number" || !(v > 0)) throw new TypeError(`mesh(): ${what} must be a positive number of mm`);
  }
  if (!Array.isArray(teeth) || teeth.length !== 2 || !teeth.every((z) => Number.isInteger(z) && z >= 6)) throw new TypeError("mesh(): teeth is [driver, driven], whole numbers of at least 6");
  __mechanism.meshes.push({ driver, driven, module, teeth, pressure_angle: pressureAngle, width, backlash, internal: !!internal });
}

/**
 * A joint between parts, for exact motion (linkages: cranks, rods, pistons,
 * levers). `a` and `b` are a part name, a list of names moving together, or
 * "ground" (what stays still). Types:
 *   "hinge"    turns about `axis` through `at` (a pin in a hole)
 *   "slider"   slides along `axis` without turning (a piston in its bore)
 *   "cylinder" slides along and turns about `axis`
 *   "ball"     turns any way about `at`
 *   "fixed"    holds the two together
 *   joint("hinge", "Crank", "ground", { at: [0, 0, 0], axis: [0, 1, 0], name: "Main bearing" })
 * Returns the joint's name (for drive() and couple()).
 */
function joint(type, a, b, { at = [0, 0, 0], axis = [0, 0, 1], name } = {}) {
  if (!["hinge", "slider", "cylinder", "ball", "fixed"].includes(type)) throw new TypeError(`joint(): the type is "hinge", "slider", "cylinder", "ball" or "fixed", not ${JSON.stringify(type)}`);
  const parts = (x) => {
    const names = [x].flat();
    if (!names.length || !names.every((n) => typeof n === "string" && n.trim())) throw new TypeError('joint(): name the parts, or "ground"');
    return names.map((n) => n.trim());
  };
  const n = name ?? `Joint ${__mechanism.joints.length + 1}`;
  if (__mechanism.joints.some((j) => j.name === n)) throw new TypeError(`joint(): two joints are called "${n}"`);
  __mechanism.joints.push({ type, a: parts(a), b: parts(b), at: vec3(at), axis: __unit(vec3(axis)), name: n });
  return n;
}

/**
 * The joint a motor turns (or pushes): through `from`–`to` (degrees for a
 * hinge, 0–360 unless given; mm for a slider) at `speed` (rpm, or mm/s).
 *   drive("Main bearing", { speed: 3000 })
 */
function drive(jointName, { from = 0, to, speed = 60 } = {}) {
  if (typeof jointName !== "string") throw new TypeError("drive(): name the joint (what joint() returned)");
  __mechanism.drive = { joint: jointName, from, to, speed };
}

/**
 * One joint's movement tied to another's: gears (degrees to degrees), a rack
 * (degrees to mm), a lead screw (degrees to mm). The second joint's value is
 * `ratio` times the first's.
 *   couple("Pinion", "Wheel", -18 / 54)
 */
function couple(fromJoint, toJoint, ratio) {
  if (typeof ratio !== "number" || !isFinite(ratio)) throw new TypeError("couple(): the ratio is a number");
  __mechanism.couplings.push({ from: fromJoint, to: toJoint, ratio });
}

function __result(out, p) {
  if (__finishes.length) out = __finish(out, p);
  const m = __mechanism;
  const mechanism = m.bodies.length || m.meshes.length || m.joints.length ? m : undefined;
  if (out instanceof Solid) return JSON.stringify({ solid: out.h, metrics: __metrics, mechanism });
  const list = Array.isArray(out) ? out.flat(Infinity) : null;
  if (!list || !list.length || !list.every((x) => x instanceof Part)) {
    throw new TypeError("the program must return a Solid or a list of part()s (got " + typeof out + ")");
  }
  const seen = new Set();
  for (const x of list) {
    if (seen.has(x.name)) throw new Error(`two parts are named "${x.name}": part names must be unique`);
    seen.add(x.name);
  }
  return JSON.stringify({ parts: list.map((x) => ({ name: x.name, h: x.solid.h, material: x.material ?? null, process: x.process ?? null })), metrics: __metrics, mechanism });
}

/**
 * The bodies of an imported STEP file, as parts named as in the file. The
 * file is referred to by its content hash (Forma stores it on import); the
 * view meshes it to `tolerance`, exact exports pass its own B-rep through.
 */
function importStep(sha, { material, process } = {}) {
  const bodies = JSON.parse(__n.importStep(sha, tolerance));
  const seen = new Map();
  return bodies.map(([name, h]) => {
    // Bodies with the same name get numbered, as part names must be unique.
    const n = (seen.get(name) ?? 0) + 1;
    seen.set(name, n);
    return part(n === 1 ? name : `${name} ${n}`, new Solid(h), { material, process });
  });
}

// ---- Sketches -------------------------------------------------------------------

/** Euler angles (degrees, X then Y then Z) turning the XY plane onto a plane with these axes. */
function planeRotation(x, y) {
  const n = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
  // Columns of the rotation are x, y, n; it is Rz·Ry·Rx.
  const deg = (r) => (r * 180) / Math.PI;
  const s = Math.max(-1, Math.min(1, -x[2]));
  const ry = Math.asin(s);
  if (Math.abs(Math.cos(ry)) > 1e-9) return [deg(Math.atan2(y[2], n[2])), deg(ry), deg(Math.atan2(x[1], x[0]))];
  return [0, deg(ry), deg(Math.atan2(-y[0], y[1]))];
}

class SketchRef {
  constructor(name, data) {
    this.name = name;
    this.plane = data.plane;
    this.loops = data.loops;
    Object.freeze(this);
  }
  /** The closed outlines as a profile, in the sketch's own 2D coordinates (arcs stay arcs when exact). */
  get profile() {
    return new Profile(__n.sketchProfile(JSON.stringify(this.loops), tolerance));
  }
  /** Moves a solid built in sketch coordinates (XY, Z out of the sketch) onto the sketch's plane. */
  place(solid) {
    const [rx, ry, rz] = planeRotation(this.plane.x, this.plane.y);
    return solid.rotate(rx, ry, rz).translate(this.plane.origin);
  }
  /** Extrudes the sketch's outlines along its plane's normal. `from` starts the extrusion off the plane. */
  extrude(height, { from = 0, ...opts } = {}) {
    let s = this.profile.extrude(height, opts);
    if (from) s = s.translate(0, 0, from);
    return this.place(s);
  }
  /** Revolves the outlines about the sketch's own Y axis (x is the radius). */
  revolve(degrees = 360, opts = {}) {
    return this.place(this.profile.revolve(degrees, opts).rotate(-90, 0, 0));
  }
}

/** A sketch drawn in the project, by name: its outlines, solved, on its plane. */
function sketch(name) {
  const data = __sketches[name];
  if (!data) throw new Error(`no sketch called “${name}” is linked to this program`);
  if (!data.loops.length) throw new Error(`sketch “${name}” has no closed outline yet${data.open ? ` (${data.open} open chain${data.open > 1 ? "s" : ""})` : ""}`);
  return new SketchRef(name, data);
}

// ---- Profiles ----------------------------------------------------------------

function circle(radius, { segments = segmentsFor(radius) } = {}) {
  return new Profile(__n.profile("circle", [radius, segments]));
}

function rect(x, y, { center = false } = {}) {
  return new Profile(__n.profile("square", [x, y, center ? 1 : 0]));
}

/** Signed area of an outline: positive when its points run counter-clockwise. */
function windingArea(points) {
  let a = 0;
  for (let i = 0; i < points.length; i++) {
    const [x0, y0] = points[i];
    const [x1, y1] = points[(i + 1) % points.length];
    a += x0 * y1 - x1 * y0;
  }
  return a / 2;
}

/** A closed outline from points [[x,y], ...]; the area inside it, whichever way the points run. */
function polygon(points) {
  const ccw = windingArea(points) >= 0 ? points : [...points].reverse();
  return new Profile(__n.polygons([ccw.flat()]));
}

/** Several outlines: counter-clockwise ones add area, clockwise ones cut holes. */
function polygons(contours) {
  return new Profile(__n.polygons(contours.map((c) => c.flat())));
}

/** n-sided polygon with this circumradius; the first vertex at `rotate` degrees. */
function regularPolygon(n, circumradius, { rotate = 0 } = {}) {
  const pts = [];
  for (let i = 0; i < n; i++) {
    const a = ((rotate + (360 * i) / n) * Math.PI) / 180;
    pts.push([circumradius * Math.cos(a), circumradius * Math.sin(a)]);
  }
  return polygon(pts);
}

// ---- Helpers -------------------------------------------------------------------

const inch = (v) => v * 25.4;
const rad = (degrees) => (degrees * Math.PI) / 180;

/** Seeded random in [0, 1): same seed, same sequence. */
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
