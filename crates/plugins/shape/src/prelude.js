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
/**
 * The configuration the shape is evaluated in ("Basic", "M8×30"), or null
 * when none is active: a program can branch on it, e.g.
 *   if (config === "Pro") list.push(part("Fan", fan));
 */
const config = typeof __inputs.config === "string" ? __inputs.config : null;
/** Parts that configuration leaves out, by name: dropped from the result. */
const __partsOff = Array.isArray(__inputs.parts_off) ? __inputs.parts_off : [];
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
   * Pushes a face out by `distance` mm (adding material), or pulls it in
   * with a negative distance (cutting), along its normal. A flat face is
   * swept; a curved one (a cylinder's side, a hole, a sphere) is offset, its
   * neighbours meeting it. `which` picks the face: { normal: [x, y, z], near:
   * [x, y, z] } (the flat face facing that way nearest the point),
   * { curved: true, near } (the face nearest the point, flat or curved), or a
   * face from solid.faces. Exact.
   *   box(60, 40, 10).push(5, { normal: [0, 0, 1] })        // 15 mm tall now
   *   cylinder(20, 10).push(2, { curved: true, near: [10, 0, 10] })  // radius 12
   */
  push(distance, which) {
    if (typeof distance !== "number" || !Number.isFinite(distance)) throw new Error("push: the distance must be a number of mm");
    if (distance === 0) return this;
    const face = selectFace(this, which);
    return new Solid(__n.pushFace(this.h, face.index, distance));
  }
  /**
   * Hollows the solid to walls `thickness` mm thick. Positive grows the walls
   * inward (the outside stays exactly where it is); negative grows them
   * outward (the solid as it is becomes the cavity). `which` names the faces
   * to remove, leaving the shell open there: one face picked as push picks
   * it, or a list of them. Leave it out for a closed hollow (a sealed void).
   * Exact.
   *   box(40, 40, 20).shell(2, { normal: [0, 0, 1] })   // an open-top tray
   *   box(40, 40, 20).shell(2)                          // sealed, 2 mm walls
   */
  shell(thickness, which) {
    if (typeof thickness !== "number" || !Number.isFinite(thickness) || thickness === 0) throw new Error("shell: the wall thickness must be a non-zero number of mm");
    const open = which == null ? [] : (Array.isArray(which) ? which : [which]).map((w) => selectFace(this, w).index);
    return new Solid(__n.shell(this.h, thickness, [...new Set(open)]));
  }
  /**
   * Tapers faces by `angle` degrees so the part releases from a mould pulled
   * along `pull` (default [0, 0, 1]). The faces hinge about the neutral plane
   * (square to the pull) where they keep their size; positive angles lean
   * them inward going along the pull, negative outward. Options:
   *   faces / which: the faces (one, or a list, picked as push picks them);
   *     default every wall running along the pull (flat sides and bosses).
   *   pull: [x, y, z], the direction the mould opens.
   *   neutral: where the neutral plane sits, as a distance along the pull
   *     (for pull [0, 0, 1], a height z); default the end the pull starts
   *     from (the bottom). neutralAt: the same as a fraction 0–1 of the part.
   * Exact.
   *   box(40, 40, 20).draft(5)                          // top 36.5 mm square
   *   boss.draft(2, { pull: [0, 0, 1], neutral: 10 })
   */
  draft(angle, options = {}) {
    if (typeof angle !== "number" || !Number.isFinite(angle) || Math.abs(angle) >= 89) throw new Error("draft: the angle must be a number of degrees under 89");
    if (angle === 0) return this;
    const pull = __unit(vec3(options.pull ?? [0, 0, 1]));
    const pick = options.faces ?? options.which;
    const faces = pick == null ? [] : (Array.isArray(pick) ? pick : [pick]).map((w) => selectFace(this, w).index);
    // The part's extent along the pull: its bounds' corners projected.
    const { min, max } = this.bounds;
    let lo = Infinity, hi = -Infinity;
    for (const x of [min[0], max[0]]) for (const y of [min[1], max[1]]) for (const z of [min[2], max[2]]) {
      const d = __dot([x, y, z], pull);
      lo = Math.min(lo, d), hi = Math.max(hi, d);
    }
    let at = lo;
    if (typeof options.neutral === "number") at = options.neutral;
    else if (typeof options.neutralAt === "number") at = lo + options.neutralAt * (hi - lo);
    else if (options.neutral != null) at = __dot(vec3(options.neutral), pull);
    return new Solid(__n.draft(this.h, [...new Set(faces)], angle, pull, pull.map((c) => c * at)));
  }
}

/**
 * Picks one flat face: { normal, near } (facing that way, nearest the point),
 * a face from solid.faces, or { at, dir } as picked in the view (where it
 * sits relative to the bounds, so it follows when the part is resized).
 */
function selectFace(solid, which) {
  if (which && typeof which.index === "number") return which;
  // A curved face (or any face) by where it is: the face nearest the point.
  if (which && (which.curved || which.kind === "curved_face")) {
    let near = which.near;
    if (which.at) {
      const { min, max } = solid.bounds;
      near = which.at.map((f, k) => min[k] + f * (max[k] - min[k]));
    }
    if (!near) throw new Error("push: a curved face is picked by a point near it");
    const q = vec3(near);
    const s = asSolid(solid);
    return s.faces[__n.faceNear(s.h, q[0], q[1], q[2])];
  }
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
  "pcb_fab", // printed circuit board (Gerber files from the PCB workspace)
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
function part(name, solid, { material, process, stock: bought, ifc } = {}) {
  if (typeof name !== "string" || !name.trim()) throw new TypeError("part() needs a name");
  asSolid(solid);
  // What it is as a building element: "IfcWall", or { class, level, psets, … } (see bim.js).
  if (ifc !== undefined) __bimElement(name.trim(), ifc);
  if (bought !== undefined) {
    if (process !== undefined && process !== "buy") throw new Error(`part ${name}: a stock part is bought (process "buy")`);
    process = "buy";
    __declared.parts[name.trim()] = __stockInfo(`part ${name}`, bought);
  }
  if (process !== undefined && !PROCESSES.includes(process)) {
    throw new Error(`part ${name}: unknown process "${process}" (use one of ${PROCESSES.join(", ")})`);
  }
  if (material !== undefined && typeof material !== "string") throw new TypeError(`part ${name}: material is a name`);
  return new Part(name.trim(), solid, material, process);
}

// ---- Stock items and connectors ------------------------------------------------

const __declared = { stock: null, parts: {}, connectors: [] };
const __optText = (where, what, v) => {
  if (v === undefined || v === null) return undefined;
  if (typeof v !== "string" || !v.trim()) throw new TypeError(`${where}: ${what} is text`);
  return v.trim();
};
const __optNum = (where, what, v) => {
  if (v === undefined || v === null) return undefined;
  if (typeof v !== "number" || !Number.isFinite(v) || v < 0) throw new TypeError(`${where}: ${what} is a number of at least 0`);
  return v;
};
function __stockInfo(where, s) {
  if (!s || typeof s !== "object") throw new TypeError(`${where}: stock is { partNumber, standard?, description?, … }`);
  const partNumber = __optText(where, "partNumber", s.partNumber);
  if (!partNumber) throw new TypeError(`${where}: a stock item needs its partNumber (what a buyer orders)`);
  const pack = __optNum(where, "pack", s.pack);
  if (pack !== undefined && !Number.isInteger(pack)) throw new TypeError(`${where}: pack is a whole number`);
  const sources = (s.sources ?? []).map((x, i) => {
    const w = `${where}: sources[${i}]`;
    const supplier = __optText(w, "supplier", x?.supplier);
    if (!supplier) throw new TypeError(`${w} needs a supplier`);
    return { supplier, sku: __optText(w, "sku", x.sku), url: __optText(w, "url", x.url), unit_price: __optNum(w, "unitPrice", x.unitPrice), currency: __optText(w, "currency", x.currency) };
  });
  return {
    standard: __optText(where, "standard", s.standard),
    part_number: partNumber,
    description: __optText(where, "description", s.description),
    manufacturer: __optText(where, "manufacturer", s.manufacturer),
    mpn: __optText(where, "mpn", s.mpn),
    unit_price: __optNum(where, "unitPrice", s.unitPrice),
    currency: __optText(where, "currency", s.currency),
    pack,
    mass_g: __optNum(where, "massG", s.massG),
    sources,
  };
}

/**
 * The whole shape is one bought-in item, as a buyer orders it. Evaluated per
 * configuration, so the part number can follow the size:
 *   stock({ standard: "ISO 4762", partNumber: `ISO 4762 M3×${p.length} A2`,
 *           description: "Socket head cap screw", unitPrice: 0.04, pack: 100,
 *           sources: [{ supplier: "McMaster-Carr", sku: "91292A113" }] })
 * In a multi-part program, give a bought part `stock` instead:
 *   part("Servo", body, { material: "Servo", stock: { partNumber: "MG996R" } })
 */
function stock(info) {
  __declared.stock = __stockInfo("stock()", info);
}

/**
 * A named frame where something attaches, in the program's own coordinates:
 * a servo's output shaft, a bearing's bore, a mounting hole, a face. `axis`
 * is the shaft or hole axis (or a face's outward normal); `x` fixes the turn
 * about it (any direction not along the axis; default the one nearest +X).
 * Assemblies join instances connector to connector.
 *   connector("Output shaft", { at: [0, 0, 42.9], axis: [0, 0, 1] })
 */
function connector(name, { at = [0, 0, 0], axis = [0, 0, 1], x, part: on } = {}) {
  if (typeof name !== "string" || !name.trim()) throw new TypeError("connector() needs a name");
  const n = name.trim();
  if (__declared.connectors.some((c) => c.name === n)) throw new TypeError(`connector(): two connectors are called "${n}"`);
  const len = (v) => Math.hypot(...v);
  if (!(len(vec3(axis)) > 1e-9)) throw new TypeError(`connector ${n}: axis must be a direction`);
  const a = __unit(vec3(axis));
  let r = x === undefined ? (Math.abs(a[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0]) : vec3(x);
  const d = __dot(r, a);
  r = [r[0] - d * a[0], r[1] - d * a[1], r[2] - d * a[2]];
  if (!(len(r) > 1e-9)) throw new TypeError(`connector ${n}: x must not lie along the axis`);
  r = __unit(r);
  const pt = vec3(at);
  if (!pt.every(Number.isFinite)) throw new TypeError(`connector ${n}: at must be a point`);
  __declared.connectors.push({ name: n, part: on === undefined ? undefined : String(on).trim(), at: pt, axis: a, x: r });
  return n;
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
function __finish(out, p, off) {
  for (const f of __finishes) {
    if (f.part !== undefined && off.has(f.part)) continue; // its part is left out
    const size = p[f.key];
    if (typeof size !== "number") throw new Error(`edge finish: the shape has no parameter “${f.key}”`);
    const apply = (s) => {
      if (f.kind === "push") return f.edges.reduce((acc, pick) => acc.push(size, pick), s);
      if (f.kind === "shell") return s.shell(size, f.edges.length ? f.edges : undefined);
      if (f.kind === "draft") return s.draft(size, { faces: f.edges.length ? f.edges : undefined, pull: f.pull ?? [0, 0, 1], neutralAt: f.neutral ?? 0 });
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

/**
 * Drops the parts the configuration leaves out, and with them every hinge,
 * joint, gear mesh, drive and coupling that only they held; says what went.
 */
function __leaveOut(list) {
  const report = { parts: [], all: [], missing: [], dropped: [] };
  const names = new Set(list.map((x) => x.name));
  const off = new Set(__partsOff.filter((n) => names.has(n)));
  report.missing = __partsOff.filter((n) => !names.has(n));
  if (!off.size) return { list, off, report };
  const kept = list.filter((x) => !off.has(x.name));
  report.parts = list.filter((x) => off.has(x.name)).map((x) => x.name);
  report.all = list.map((x) => x.name);
  const q = (ns) => ns.map((n) => `“${n}”`).join(", ");
  const m = __mechanism;
  const notes = report.dropped;
  // A group of parts moving together loses the ones left out; it goes once none is left.
  const trim = (group, what) => {
    const gone = group.filter((n) => off.has(n));
    const rest = group.filter((n) => !off.has(n));
    if (gone.length && rest.length) notes.push(`${what} goes on without ${q(gone)}`);
    return { gone, rest };
  };
  m.bodies = m.bodies.filter((b, i) => {
    const { gone, rest } = trim(b.parts, `Hinge ${i + 1}`);
    if (gone.length && !rest.length) {
      notes.push(`Hinge ${i + 1} (${q(gone)}) is left out with ${gone.length > 1 ? "them" : "it"}`);
      return false;
    }
    b.parts = rest;
    return true;
  });
  m.meshes = m.meshes.filter((g) => {
    const gone = [g.driver, g.driven].filter((n) => off.has(n));
    if (gone.length) notes.push(`Gear mesh ${q([g.driver])} → ${q([g.driven])} is left out with ${q(gone)}`);
    return !gone.length;
  });
  const lost = new Set();
  m.joints = m.joints.filter((j) => {
    const a = trim(j.a, `Joint “${j.name}”`);
    const b = trim(j.b, `Joint “${j.name}”`);
    if ((a.gone.length && !a.rest.length) || (b.gone.length && !b.rest.length)) {
      notes.push(`Joint “${j.name}” is left out with ${q([...a.gone, ...b.gone])}`);
      lost.add(j.name);
      return false;
    }
    j.a = a.rest;
    j.b = b.rest;
    return true;
  });
  if (m.drive && lost.has(m.drive.joint)) {
    notes.push(`The drive on “${m.drive.joint}” is left out with its joint`);
    m.drive = null;
  }
  m.couplings = m.couplings.filter((c) => {
    const gone = lost.has(c.from) || lost.has(c.to);
    if (gone) notes.push(`The coupling “${c.from}” → “${c.to}” is left out with its joint`);
    return !gone;
  });
  return { list: kept, off, report };
}

function __result(out, p) {
  let list = null;
  let off = new Set();
  let left_out;
  if (!(out instanceof Solid)) {
    list = Array.isArray(out) ? out.flat(Infinity) : null;
    if (!list || !list.length || !list.every((x) => x instanceof Part)) {
      throw new TypeError("the program must return a Solid or a list of part()s (got " + typeof out + ")");
    }
    const seen = new Set();
    for (const x of list) {
      if (seen.has(x.name)) throw new Error(`two parts are named "${x.name}": part names must be unique`);
      seen.add(x.name);
    }
    if (__partsOff.length) ({ list, off, report: left_out } = __leaveOut(list));
    out = list;
  } else if (__partsOff.length) {
    // One solid has no parts to leave out: every name is reported as missing.
    left_out = { parts: [], all: [], missing: [...__partsOff], dropped: [] };
  }
  if (__finishes.length) out = __finish(out, p, off);
  const names = out instanceof Solid ? null : new Set(out.flat(Infinity).map((x) => x.name));
  for (const c of __declared.connectors) {
    if (c.part !== undefined && !names) throw new Error(`connector ${c.name}: the program returns one solid, it has no part "${c.part}"`);
    if (c.part !== undefined && !names.has(c.part) && !off.has(c.part)) throw new Error(`connector ${c.name}: there is no part "${c.part}"`);
  }
  const declared = {
    stock: __declared.stock ?? undefined,
    parts: Object.fromEntries(Object.entries(__declared.parts).filter(([n]) => names && names.has(n))),
    connectors: __declared.connectors.filter((c) => c.part === undefined || !off.has(c.part)),
    bim: __bimDeclared(names),
  };
  const m = __mechanism;
  const mechanism = m.bodies.length || m.meshes.length || m.joints.length ? m : undefined;
  if (out instanceof Solid) return JSON.stringify({ solid: out.h, metrics: __metrics, mechanism, left_out, declared });
  list = out.flat(Infinity);
  return JSON.stringify({ parts: list.map((x) => ({ name: x.name, h: x.solid.h, material: x.material ?? null, process: x.process ?? null })), metrics: __metrics, mechanism, left_out, declared });
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
