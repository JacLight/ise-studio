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
  /** Minkowski sum: this swept over every point of the others (faceted in STEP). */
  minkowski(...others) {
    return others.flat().reduce((a, b) => new Solid(__n.sbool("minkowski", a.h, asSolid(b).h)), this);
  }
  /** Any affine map, a 3×4 matrix row by row ([[a, b, c, tx], …]); exact unless it shears. */
  transform(m) {
    const rows = m.length === 12 ? [m.slice(0, 4), m.slice(4, 8), m.slice(8, 12)] : m;
    return new Solid(__n.sop("transform", this.h, [0, 1, 2].flatMap((i) => [0, 1, 2, 3].map((j) => rows[i]?.[j] ?? (i === j ? 1 : 0)))));
  }
  /**
   * Bends a solid built flat around the Z axis, as a label goes round a
   * bottle: x runs around the circle (x = 0 lands on +X, +x turning
   * counter-clockwise seen from above), y runs up the axis, and z = 0 lands
   * on `radius` (+z outward). Build lettering, a traced logo or a relief
   * flat, then wrap it onto the cylinder it decorates (see emboss/engrave
   * with `cylinder`). Faceted in exact exports.
   *   text("FORMA", { size: 6, align: "center" }).extrude(1).wrap(20)
   */
  wrap(radius) {
    if (typeof radius !== "number" || !(radius > 0)) throw new Error("wrap: the radius must be a positive number of mm");
    return new Solid(__n.wrap(this.h, radius, tolerance));
  }
  /**
   * Adds a decoration raised off a face: lettering or a traced outline (a
   * Profile, extruded `height` mm), or a solid such as a relief (placed as
   * it is: its back, z = 0, on the face). Where it goes:
   *   at, normal, x   a flat face: the point the decoration's origin lands
   *                   on, the face's outward normal, and the direction its
   *                   +x runs (see placeOn). Default the XY plane, +Z out.
   *   cylinder: { radius, angle = 0, z = 0 }   wrapped around a cylinder on
   *                   the Z axis: its middle at `angle` degrees from +X, its
   *                   origin at height z.
   * A profile sinks `sink` mm (0.2) into the face so the two fuse.
   *   plate.emboss(text("BATCH 7", { size: 5, align: "center" }), { at: [40, 20, 6], height: 0.8 })
   *   mug.emboss(relief(photo, { width: 60, depth: 1.2, base: 0.4 }).translate(0, 0, -0.4), { cylinder: { radius: 40, z: 45 } })
   */
  emboss(decor, options = {}) {
    const { height, sink = 0.2 } = options;
    let s = decor;
    if (decor instanceof Profile) {
      if (typeof height !== "number" || !(height > 0)) throw new Error("emboss: give the height a profile stands up, in mm");
      s = decor.extrude(height + sink).translate(0, 0, -sink);
    }
    return this.add(__onto(asSolid(s), options, "emboss"));
  }
  /**
   * Cuts a decoration into a face: lettering or a traced outline (a Profile,
   * cut `depth` mm deep), or a cutter solid such as relief(…, { mode:
   * "engrave" }) (placed as it is, z = 0 on the face). Placed as emboss
   * places it. `through: true` cuts a profile all the way through (to
   * `depth` if given, else 1000 mm).
   *   lid.engrave(text("12 V", { size: 4, weight: 700, align: "center" }), { at: [30, 30, 3], depth: 0.6 })
   *   ring.engrave(trace(logo, { width: 14 }), { cylinder: { radius: 11, angle: 90, z: 4 }, depth: 0.4 })
   */
  engrave(decor, options = {}) {
    const { depth, clear = 1, through = false } = options;
    let s = decor;
    if (decor instanceof Profile) {
      const d = through ? (typeof depth === "number" ? depth : 1000) : depth;
      if (typeof d !== "number" || !(d > 0)) throw new Error("engrave: give the depth to cut, in mm");
      s = decor.extrude(d + clear).translate(0, 0, -d);
    }
    return this.subtract(__onto(asSolid(s), options, "engrave"));
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
  /**
   * The outlines as points: [[[x, y], …], …], counter-clockwise around the
   * area and clockwise around holes. For measuring a traced picture, or
   * taking part of an outline to build from (a silhouette's half to revolve).
   */
  get loops() {
    return JSON.parse(__n.ppoints(this.h));
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
function part(name, solid, { material, process, stock: bought } = {}) {
  if (typeof name !== "string" || !name.trim()) throw new TypeError("part() needs a name");
  asSolid(solid);
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
  // Cut to length: one piece of `cutLength` mm at `pricePerM`; the piece's
  // price follows from them when no unitPrice is given.
  const cutLength = __optNum(where, "cutLength", s.cutLength);
  const pricePerM = __optNum(where, "pricePerM", s.pricePerM);
  let unitPrice = __optNum(where, "unitPrice", s.unitPrice);
  if (unitPrice === undefined && cutLength !== undefined && pricePerM !== undefined) unitPrice = Math.round((pricePerM * cutLength) / 10) / 100;
  return {
    standard: __optText(where, "standard", s.standard),
    part_number: partNumber,
    description: __optText(where, "description", s.description),
    manufacturer: __optText(where, "manufacturer", s.manufacturer),
    mpn: __optText(where, "mpn", s.mpn),
    unit_price: unitPrice,
    currency: __optText(where, "currency", s.currency),
    pack,
    mass_g: __optNum(where, "massG", s.massG),
    cut_length_mm: cutLength,
    price_per_m: pricePerM,
    sources,
  };
}

/**
 * The whole shape is one bought-in item, as a buyer orders it. Evaluated per
 * configuration, so the part number can follow the size:
 *   stock({ standard: "ISO 4762", partNumber: `ISO 4762 M3×${p.length} A2`,
 *           description: "Socket head cap screw", unitPrice: 0.04, pack: 100,
 *           sources: [{ supplier: "McMaster-Carr", sku: "91292A113" }] })
 * A cut-to-length item (beam, tube, bar) gives its piece length and price per
 * metre; the piece's unitPrice follows from them when left out:
 *   stock({ partNumber: `IPE 200 × ${p.length} S235JR`, cutLength: p.length, pricePerM: 38.1 })
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

// ---- Placing decorations ---------------------------------------------------------

/**
 * Moves a solid built in XY (+Z out of the face, +X along it) onto a plane:
 * its origin to `at`, its +Z along `normal`. Its +X runs along `x` if given;
 * otherwise "up" (+Y) points as near world +Z as the plane allows (+Y on a
 * face looking up or down), so lettering on a wall reads level.
 *   placeOn(text("A1", { size: 4 }).extrude(0.6), { at: [0, -20, 10], normal: [0, -1, 0] })
 */
function placeOn(solid, { at = [0, 0, 0], normal = [0, 0, 1], x } = {}) {
  const z = __unit(vec3(normal));
  const reject = (v) => {
    const d = __dot(v, z);
    return [v[0] - d * z[0], v[1] - d * z[1], v[2] - d * z[2]];
  };
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  let ax, ay;
  if (x !== undefined) {
    ax = reject(vec3(x));
    if (Math.hypot(...ax) < 1e-9) throw new Error("placeOn: x must not run along the normal");
    ax = __unit(ax);
    ay = cross(z, ax);
  } else {
    ay = Math.abs(z[2]) > 0.999 ? [0, 1, 0] : __unit(reject([0, 0, 1]));
    ax = cross(ay, z);
  }
  const [rx, ry, rz] = planeRotation(ax, ay);
  return asSolid(solid).rotate(rx, ry, rz).translate(vec3(at));
}

function __onto(solid, options, what) {
  const c = options.cylinder;
  if (c) {
    if (typeof c.radius !== "number" || !(c.radius > 0)) throw new Error(`${what}: cylinder needs its radius, in mm`);
    return solid.wrap(c.radius).rotate(0, 0, c.angle ?? 0).translate(0, 0, c.z ?? 0);
  }
  return placeOn(solid, options);
}

// ---- Pictures --------------------------------------------------------------------

/**
 * A picture stored in the project (PNG or JPEG; add one with
 * `./forma image add <file>`, which prints its id). Pictures have no size
 * of their own: reliefs and traces are given a width or height in mm.
 *   const photo = image("3f9a…");   // .width, .height (pixels), .aspect (h / w)
 *   photo.grid(200)                  // brightness, 200 columns × rows to keep the aspect, rows from the top, 0–1
 *   photo.grid(100, 50, { channel: "alpha" })
 */
class Picture {
  constructor(id) {
    if (typeof id !== "string" || !/^[0-9a-fA-F]{64}$/.test(id)) {
      throw new Error(`image: ${JSON.stringify(id)} is not a picture id (add the picture with ./forma image add <file> and use the id it prints)`);
    }
    const [w, h, bits, alpha] = __n.imageInfo(id);
    this.id = id;
    this.width = w;
    this.height = h;
    this.bits = bits;
    /** Some pixels are see-through (a logo on a transparent background). */
    this.transparent = alpha === 1;
    Object.freeze(this);
  }
  get aspect() {
    return this.height / this.width;
  }
  /**
   * The picture as rows of values 0–1 (top row first), resampled to `nx` ×
   * `ny` points spanning it edge to edge: brightness (1 white), or
   * channel "alpha" | "red" | "green" | "blue".
   */
  grid(nx, ny = Math.max(2, Math.round(nx * this.aspect)), { channel = "brightness" } = {}) {
    const flat = __n.imageGrid(this.id, channel, nx, ny);
    const rows = [];
    for (let j = 0; j < ny; j++) rows.push(flat.slice(j * nx, (j + 1) * nx));
    return rows;
  }
}

function image(id) {
  return new Picture(id);
}

const __picture = (img, what) => {
  if (img instanceof Picture) return img;
  if (typeof img === "string") return new Picture(img);
  throw new TypeError(`${what}: give a picture (image("<id>") or its id)`);
};

/** Width and height in mm: one given, the other follows the picture's aspect. */
function __pictureSize(pic, { width, height }, what) {
  const ok = (v) => typeof v === "number" && Number.isFinite(v) && v > 0;
  if (ok(width) && ok(height)) return [width, height];
  if (ok(width)) return [width, width * pic.aspect];
  if (ok(height)) return [height / pic.aspect, height];
  throw new Error(`${what}: give its width or height in mm`);
}

/**
 * A relief from a picture: brightness becomes height. Centred on the
 * origin, its back flat at z = 0 (the picture's top towards +Y). Modes:
 *   "emboss"      a plaque: base + depth × brightness (light stands high);
 *                 sink the base into a face to raise a picture off it.
 *   "engrave"     a cutter to subtract from a face at z = 0: it reaches
 *                 depth × darkness down (dark cuts deep, white not at all)
 *                 and `clear` mm up; part.engrave(relief(…, { mode: "engrave" }), …).
 *   "lithophane"  light through thin plastic: base + depth × darkness thick
 *                 (dark blocks light); print upright, light behind it.
 * Options (mm): width or height (the other follows the picture), depth (2;
 * lithophane 2.4), base (1; lithophane 0.8, engrave 0 — extra depth),
 * invert (swap light and dark), gamma (1; above 1 deepens mid-tones),
 * smooth (Gaussian blur, mm: removes noise and pixel steps), channel
 * ("brightness", or "alpha" | "red" | "green" | "blue"), border (a frame
 * this wide at full height, emboss and lithophane), clear (engrave, 1),
 * detail (grid points a side, 1024; 2048 or 4096 for large fine pictures).
 * The surface follows the picture to within `tolerance` (no finer than half
 * a grey level), as few triangles as that needs; exports get more detail
 * than the view. Watertight, ready to print or to carve.
 *   relief(photo, { width: 100, depth: 3, base: 2, smooth: 0.2 })
 *   relief(photo, { width: 80, mode: "lithophane", border: 3 })
 */
function relief(img, options = {}) {
  const pic = __picture(img, "relief");
  const mode = options.mode ?? "emboss";
  const litho = mode === "lithophane";
  const [width, height] = __pictureSize(pic, options, "relief");
  const depth = options.depth ?? (litho ? 2.4 : 2);
  const base = options.base ?? (litho ? 0.8 : mode === "engrave" ? 0 : 1);
  const border = options.border ?? 0;
  if (border && mode === "engrave") throw new Error("relief: an engraving has no border (cut a frame separately)");
  if (border && !(border > 0 && 2 * border < Math.min(width, height))) throw new Error("relief: the border must leave room for the picture");
  const inner = [width - 2 * border, height - 2 * border];
  const json = JSON.stringify({
    width: inner[0],
    height: inner[1],
    depth,
    base,
    mode,
    invert: !!options.invert,
    gamma: options.gamma ?? 1,
    smooth: options.smooth ?? 0,
    channel: options.channel ?? "brightness",
    clear: options.clear ?? 1,
    detail: options.detail ?? 1024,
  });
  const r = new Solid(__n.relief(pic.id, json, tolerance));
  if (!border) return r;
  const frame = rect(width, height, { center: true }).subtract(rect(inner[0], inner[1], { center: true })).extrude(base + depth);
  return r.add(frame);
}

/**
 * Traces a picture into outlines: a logo, lettering, a line drawing, a
 * silhouette. The ink is what is dark (or, on a transparent background,
 * what is opaque); invert traces the light parts. Centred on the origin,
 * sized by width or height in mm. Extrude it, emboss or engrave it, cut it
 * through, revolve half of it (see Profile.loops).
 * Options: threshold (0.5: the ink level the outline follows), invert,
 * smooth (mm; default 0.7 px, rounding pixel steps; 0 keeps them),
 * minArea (mm², specks and holes smaller are dropped; default 4 px²),
 * channel ("brightness" | "alpha" | "red" | "green" | "blue"), simplify
 * (mm the outline may stray, default `tolerance`).
 *   const mark = trace(logo, { width: 30 });     // mark.bounds, mark.area, mark.loops
 *   lid.engrave(mark, { at: [0, 0, 4], depth: 0.5 })
 */
function trace(img, options = {}) {
  const pic = __picture(img, "trace");
  const [width, height] = __pictureSize(pic, options, "trace");
  const px = width / pic.width;
  const json = JSON.stringify({
    width,
    height,
    threshold: options.threshold ?? 0.5,
    invert: !!options.invert,
    smooth: options.smooth ?? 0.7 * px,
    minArea: options.minArea ?? 4 * px * px,
    channel: options.channel,
    simplify: options.simplify,
  });
  return new Profile(__n.trace(pic.id, json, tolerance));
}

// ---- Lettering ---------------------------------------------------------------------

/**
 * Text as a profile, from real font outlines: engrave it, emboss it, cut it
 * through, wrap it round a cylinder. Options:
 *   size        height of a capital letter, mm (5)
 *   weight      100–900 (400 regular, 700 bold) for the built-in font
 *   font        "Inter" (built in, a clean sans), or a stored font's id
 *               (`./forma image add font.ttf` stores TrueType/OpenType too)
 *   align       "left" | "center" | "right"       (about x = 0)
 *   valign      "baseline" | "top" | "middle" | "bottom"   (about y = 0)
 *   spacing     extra mm between letters; lineHeight: baseline to baseline,
 *               × size (1.6); "\n" starts a new line
 *   arc         set along a circle of this radius (baseline on it), centred
 *               on +Y reading clockwise over the top; negative reads left to
 *               right along the bottom, tops toward the centre (a coin)
 *   text("Forma", { size: 8, weight: 700, align: "center", valign: "middle" })
 *   text("EST. 2026", { size: 3, arc: -18, align: "center" })
 */
function text(str, options = {}) {
  if (typeof str !== "string" && typeof str !== "number") throw new TypeError("text() needs the text to draw");
  const json = JSON.stringify({
    text: String(str),
    size: options.size ?? 5,
    font: options.font,
    weight: options.weight ?? 400,
    align: options.align ?? "left",
    baseline: options.valign ?? "baseline",
    spacing: options.spacing ?? 0,
    lineHeight: options.lineHeight ?? 1.6,
    arc: options.arc,
  });
  return new Profile(__n.text(json, tolerance));
}

/**
 * A scale's tick marks and numbers, as a profile to engrave or emboss.
 * Straight, along +X from the origin (ticks rising +Y from y = 0):
 *   graduations({ length: 100, step: 1, every: [5, 10] })      // a 100 mm rule
 * Round, a dial (ticks outward from `radius`; inward: true for inside):
 *   graduations({ radius: 30, from: 0, to: 270, step: 5, every: [15, 45], values: [0, 10] })
 * Options: step (mm or degrees between ticks), every ([mid, major] in the
 * same units: ticks at multiples are longer), ticks ([minor, mid, major]
 * lengths, mm), width (line width, mm), labels (true: numbers at major
 * ticks), size (number height, mm), values ([first, last]: what the ends
 * read; default the distance or angle), decimals, gap (tick to number, mm),
 * weight. Angles run clockwise from +Y (12 o'clock), as a dial reads;
 * counterClockwise: true runs them the other way.
 */
function graduations(options = {}) {
  const round = typeof options.radius === "number";
  const step = options.step ?? (round ? 10 : 1);
  if (!(step > 0)) throw new Error("graduations: step must be positive");
  const [mid, major] = options.every ?? (round ? [step * 3, step * 9] : [step * 5, step * 10]);
  const size = options.size ?? (round ? options.radius / 10 : 2.5);
  const [tMinor, tMid, tMajor] = options.ticks ?? [size * 0.8, size * 1.2, size * 1.8];
  const width = options.width ?? Math.max(0.2, size * 0.12);
  const gap = options.gap ?? size * 0.4;
  const labels = options.labels ?? true;
  const from = round ? options.from ?? 0 : 0;
  const to = round ? options.to ?? 360 : options.length;
  if (!(typeof to === "number" && to > from)) throw new Error(round ? "graduations: to must be past from (degrees)" : "graduations: give the length in mm");
  const [v0, v1] = options.values ?? [from, to];
  const near = (a, b) => Math.abs(a / b - Math.round(a / b)) < 1e-6;
  const count = Math.floor((to - from) / step + 1e-9);
  const full = round && Math.abs(to - from - 360) < 1e-9;
  const pieces = [];
  for (let i = 0; i <= count; i++) {
    if (full && i === count) break; // 360° is 0°
    const at = from + i * step;
    const off = at - from;
    const isMajor = near(off, major);
    const len = isMajor ? tMajor : near(off, mid) ? tMid : tMinor;
    const value = v0 + ((at - from) / (to - from)) * (v1 - v0);
    const label = labels && isMajor ? text(Number(value.toFixed(options.decimals ?? 0)).toString(), { size, align: "center", weight: options.weight ?? 500 }) : null;
    if (!round) {
      pieces.push(rect(width, len).translate(at - width / 2, 0));
      if (label) pieces.push(label.translate(at, len + gap));
    } else {
      const inward = !!options.inward;
      const r0 = inward ? options.radius - len : options.radius;
      const angle = options.counterClockwise ? at : -at; // degrees from +Y
      pieces.push(rect(width, len).translate(-width / 2, r0).rotate(angle));
      if (label) {
        const rl = inward ? options.radius - len - gap - size : options.radius + len + gap;
        pieces.push(label.translate(0, rl).rotate(angle));
      }
    }
  }
  return pieces[0].add(pieces.slice(1));
}

// ---- Knurls and threads --------------------------------------------------------------

/**
 * A knurled cylinder along +Z from z = 0: a grip for a knob or a thumb
 * screw. Union it with the part, or use it as the part's outside.
 *   radius, length   mm (the tips of the teeth reach the radius)
 *   pitch            mm between teeth around the circumference (1)
 *   depth            tooth depth, mm (pitch × 0.5: 90° teeth)
 *   style            "diamond" (crossed helices) | "straight" (axial ridges)
 *   angle            helix angle from the axis, degrees (30)
 *   chamfer          mm taken off both ends at 45° (depth; 0 for none)
 *   knurl({ radius: 15, length: 12, pitch: 1.2 })
 */
function knurl({ radius, length, pitch = 1, depth, style = "diamond", angle = 30, chamfer } = {}) {
  if (!(radius > 0 && length > 0)) throw new Error("knurl: give the radius and length in mm");
  const teeth = Math.max(6, Math.round((2 * Math.PI * radius) / pitch));
  const d = depth ?? (Math.PI * radius / teeth); // half the actual pitch: 90° teeth
  if (!(d > 0 && d < radius)) throw new Error("knurl: the depth must be less than the radius");
  const pts = [];
  for (let i = 0; i < teeth; i++) {
    const a0 = (2 * Math.PI * i) / teeth;
    const a1 = (2 * Math.PI * (i + 0.5)) / teeth;
    pts.push([radius * Math.cos(a0), radius * Math.sin(a0)], [(radius - d) * Math.cos(a1), (radius - d) * Math.sin(a1)]);
  }
  const star = polygon(pts);
  let body;
  if (style === "straight") body = star.extrude(length);
  else if (style === "diamond") {
    const twist = (length * Math.tan(rad(angle)) / radius) * (180 / Math.PI);
    // Slices so the helix stays within tolerance of its curve.
    const per = 2 * Math.acos(Math.max(-1, 1 - tolerance / radius)) * (180 / Math.PI);
    const slices = Math.max(1, Math.ceil(twist / per));
    body = star.extrude(length, { twist, slices }).intersect(star.extrude(length, { twist: -twist, slices }));
  } else throw new Error(`knurl: style is "diamond" or "straight", not ${JSON.stringify(style)}`);
  const c = chamfer ?? d;
  if (c > 0) {
    const r = radius + 0.01;
    body = body.intersect(polygon([[0, 0], [r - c, 0], [r, c], [r, length - c], [r - c, length], [0, length]]).revolve());
  }
  return body;
}

/** ISO 261 coarse pitches, by nominal diameter (mm). */
const ISO_COARSE = { 1: 0.25, 1.2: 0.25, 1.4: 0.3, 1.6: 0.35, 2: 0.4, 2.5: 0.45, 3: 0.5, 3.5: 0.6, 4: 0.7, 5: 0.8, 6: 1, 7: 1, 8: 1.25, 10: 1.5, 12: 1.75, 14: 2, 16: 2, 18: 2.5, 20: 2.5, 22: 2.5, 24: 3, 27: 3, 30: 3.5, 33: 3.5, 36: 4, 39: 4, 42: 4.5, 45: 4.5, 48: 5, 52: 5, 56: 5.5, 60: 5.5, 64: 6 };

/**
 * A real ISO metric thread (ISO 68-1 profile: 60° flanks, P/8 crest and P/4
 * root flats), modelled along +Z from z = 0 to `length`.
 *   External (a bolt's threaded shank): the solid rod; union a head on it.
 *   internal: true: the cutter for a tapped hole or a nut, to subtract.
 * Options: diameter (nominal, mm), pitch (default ISO coarse for the
 * diameter), length, clearance (radial mm: shrinks an external thread,
 * grows an internal one; 0.1–0.2 for printed bolts and nuts), chamfer (45°
 * lead-in at both ends, true by default; "start" | "end" for one),
 * leftHand, starts (1), phase (degrees the thread is turned about Z, to put
 * mating threads in step).
 *   const shank = thread({ diameter: 8, length: 30 });               // M8 × 1.25
 *   nut.subtract(thread({ diameter: 8, length: 6.5, internal: true, clearance: 0.15 }))
 */
function thread({ diameter, pitch, length, internal = false, clearance = 0, chamfer = true, leftHand = false, starts = 1, phase = 0 } = {}) {
  if (!(diameter > 0 && length > 0)) throw new Error("thread: give the diameter and length in mm");
  const P = pitch ?? ISO_COARSE[diameter];
  if (!(P > 0)) throw new Error(`thread: M${diameter} has no standard coarse pitch: give pitch`);
  if (!(Number.isInteger(starts) && starts >= 1)) throw new Error("thread: starts is a whole number");
  const H = (Math.sqrt(3) / 2) * P;
  const grow = internal ? clearance : -clearance;
  const rMaj = diameter / 2 + grow;
  const rMin = diameter / 2 - (5 * H) / 8 + grow;
  if (!(rMin > 0)) throw new Error("thread: the pitch is too coarse for the diameter");
  const lead = starts * P;
  // The profile's corners through one pitch, crest centred at 0: root → crest → crest → root.
  const prof = [[-3 * P / 8, rMin], [-P / 16, rMaj], [P / 16, rMaj], [3 * P / 8, rMin]];
  const k = prof.length;
  const K = k * starts; // corners per lead
  const n = segmentsFor(rMaj);
  // Rows of corners, from below the start to above the end.
  const first = Math.floor(-lead / P) - 2;
  const last = Math.ceil((length + lead) / P) + 2;
  const rows = [];
  for (let t = first; t <= last; t++) for (const [s, r] of prof) rows.push([t * P + s, r]);
  const M = rows.length;
  const positions = [];
  for (let i = 0; i < n; i++) {
    const a = (2 * Math.PI * i) / n;
    const rise = (lead * i) / n;
    for (const [z, r] of rows) positions.push([r * Math.cos(a), r * Math.sin(a), z + rise]);
  }
  const at = (i, j) => i * M + j;
  const tris = [];
  for (let i = 0; i < n; i++) {
    for (let j = 0; j + 1 < M; j++) {
      // Going once round climbs one lead: the last column meets the first K rows up.
      const [a, b] = i + 1 < n ? [at(i + 1, j), at(i + 1, j + 1)] : j + 1 + K < M ? [at(0, j + K), at(0, j + 1 + K)] : [-1, -1];
      if (a < 0) continue;
      tris.push([at(i, j), a, b], [at(i, j), b, at(i, j + 1)]);
    }
  }
  // Close both open ends with a fan from a point on the axis.
  const count = new Map();
  const key = (a, b) => (a < b ? `${a},${b}` : `${b},${a}`);
  for (const t of tris) for (let e = 0; e < 3; e++) count.set(key(t[e], t[(e + 1) % 3]), (count.get(key(t[e], t[(e + 1) % 3])) ?? 0) + 1);
  const open = [];
  for (const t of tris) for (let e = 0; e < 3; e++) if (count.get(key(t[e], t[(e + 1) % 3])) === 1) open.push([t[e], t[(e + 1) % 3]]);
  const next = new Map(open.map(([a, b]) => [a, b]));
  while (next.size) {
    const [start] = next.keys();
    const loop = [];
    for (let v = start; next.has(v); ) {
      const w = next.get(v);
      next.delete(v);
      loop.push([v, w]);
      v = w;
    }
    const zc = loop.reduce((s, [v]) => s + positions[v][2], 0) / loop.length;
    positions.push([0, 0, zc]);
    const c = positions.length - 1;
    for (const [v, w] of loop) tris.push([c, w, v]);
  }
  let rod = fromMesh(positions, tris);
  if (phase) rod = rod.rotate(0, 0, phase);
  if (leftHand) rod = rod.mirror(1, 0, 0);
  // Trimmed to length, with 45° lead-ins: a bolt's tip down to the root, a
  // hole's mouth countersunk out to the major diameter.
  const ends = chamfer === true ? ["start", "end"] : chamfer ? [chamfer] : [];
  const depth = rMaj - rMin;
  const room = length > 2 * depth + 2 * tolerance;
  const [lo, hi] = [room && ends.includes("start"), room && ends.includes("end")];
  const R = rMaj + 1;
  let out = rod.intersect(box(2 * R, 2 * R, length, { center: true }).translate(0, 0, length / 2));
  if (internal) {
    const sink = depth + 0.01;
    if (lo) out = out.add(cylinder(sink, rMaj + 0.01, { top: rMaj + 0.01 - sink }));
    if (hi) out = out.add(cylinder(sink, rMaj + 0.01 - sink, { top: rMaj + 0.01 }).translate(0, 0, length - sink));
    return out;
  }
  if (lo || hi) {
    const e = rMaj + 0.01;
    const outline = [[0, 0]];
    outline.push(...(lo ? [[rMin, 0], [e, e - rMin]] : [[e, 0]]));
    outline.push(...(hi ? [[e, length - (e - rMin)], [rMin, length]] : [[e, length]]));
    outline.push([0, length]);
    out = out.intersect(polygon(outline).revolve());
  }
  return out;
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
