// ---- Sheet metal ---------------------------------------------------------------
//
// Parts cut flat and bent on a press brake. A sheet-metal part is built the
// way a shop makes it: a base flange (a closed outline, or an open profile
// bent along its corners), edge flanges on its straight edges, flanges on
// those, hems, jogs, tabs, holes and louvres. The same description gives the
// bent solid, the exact flat pattern (bend allowance per bend) and the bend
// table, so the 3D part and the laser file can never disagree.
//
//   const box = sheet({ width: 60, depth: 46 }, { thickness: 1, material: "Aluminium 5052-H32" });
//   const front = box.flange("-y", { length: 25 });      // up 90°, outer face flush with the edge
//   front.hole(12, 10, 9.2);                             // x along the edge, y up from the outside of the base
//   front.flange("tip", { length: 8, dir: "in" });       // a return lip, curling inward
//   return [box.part("Tray", { material: "Aluminium 5052-H32" })];
//
// Conventions (the ones SolidWorks calls "outer virtual sharp", "material
// inside"):
//   - The base outline is the outside of the part: a flange's outer face sits
//     flush on the edge it is bent from.
//   - A flange's length runs from the parent's outer face (the side away from
//     the bend's centre) to the flange's tip, outside dimension. For bends over
//     90° (hems) the parent's edge is the outermost point of the fold.
//   - "up" bends toward the flat pattern's top side (+Z of the base), "down"
//     away from it; on a flange, "in" curls on the way its own bend went and
//     "out" turns back the other way (a jog).
//   - Bend allowance BA = θ·(R + K·T); outside setback OSSB = (R + T)·tan(θ/2)
//     (R + T past 90°); bend deduction BD = 2·OSSB − BA. Flat length = sum of
//     the outside dimensions − Σ BD.

/**
 * Sheet materials: how hard they bend (K-factor class), the smallest inside
 * radius that bends without cracking (× thickness), density, and the
 * thicknesses stocked (mm). Name a material by its key or any distinctive part
 * of it ("5052", "304", "galvanised").
 */
const SHEET_MATERIALS = {
  "Aluminium 5052-H32": { match: ["5052"], hardness: "soft", minRadius: 1, density: 2680, gauges: [0.5, 0.6, 0.8, 1, 1.2, 1.5, 1.6, 2, 2.5, 3, 4, 5, 6] },
  "Aluminium 5754-H22": { match: ["5754"], hardness: "soft", minRadius: 1, density: 2670, gauges: [0.8, 1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6] },
  "Aluminium 3003-H14": { match: ["3003"], hardness: "soft", minRadius: 1, density: 2730, gauges: [0.5, 0.8, 1, 1.2, 1.5, 2, 2.5, 3] },
  "Aluminium 6061-T6": { match: ["6061"], hardness: "hard", minRadius: 2.5, density: 2700, gauges: [0.8, 1, 1.2, 1.6, 2, 2.5, 3, 4, 5, 6] },
  "Steel DC01 (cold rolled)": { match: ["dc01", "cold rolled", "crs", "mild steel", "1008", "spcc"], hardness: "medium", minRadius: 0.5, density: 7850, gauges: [0.5, 0.6, 0.8, 1, 1.2, 1.5, 2, 2.5, 3] },
  "Steel DX51D (galvanised)": { match: ["dx51", "galvanised", "galvanized", "zintec", "secc"], hardness: "medium", minRadius: 1, density: 7850, gauges: [0.5, 0.6, 0.8, 1, 1.2, 1.5, 2, 2.5, 3] },
  "Steel S355 (hot rolled)": { match: ["s355", "s235", "hot rolled", "a36"], hardness: "medium", minRadius: 1.5, density: 7850, gauges: [2, 2.5, 3, 4, 5, 6, 8, 10] },
  "Stainless 304 (1.4301)": { match: ["304", "1.4301"], hardness: "hard", minRadius: 1, density: 7930, gauges: [0.5, 0.8, 1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6] },
  "Stainless 316 (1.4404)": { match: ["316", "1.4404", "1.4401"], hardness: "hard", minRadius: 1, density: 7980, gauges: [0.8, 1, 1.2, 1.5, 2, 2.5, 3, 4] },
  "Copper C110": { match: ["c110", "copper", "cu-etp"], hardness: "soft", minRadius: 0.5, density: 8940, gauges: [0.5, 0.8, 1, 1.5, 2, 3] },
  "Brass C260": { match: ["c260", "brass", "cuzn30"], hardness: "medium", minRadius: 1, density: 8530, gauges: [0.5, 0.8, 1, 1.5, 2, 3] },
};

/**
 * K-factor for air bending by hardness and inside radius over thickness
 * (R/T under 1, 1 to 3, 3 and over): the neutral axis sits K·T from the
 * inside face. The table every press-brake handbook carries.
 */
const SHEET_K_TABLE = { soft: [0.33, 0.4, 0.5], medium: [0.38, 0.43, 0.5], hard: [0.4, 0.45, 0.5] };

/** The material entry for a name (or a generic medium-hard one). */
function sheetMaterial(name) {
  if (name && SHEET_MATERIALS[name]) return { name, ...SHEET_MATERIALS[name], known: true };
  const n = String(name ?? "").toLowerCase();
  for (const [key, m] of Object.entries(SHEET_MATERIALS)) if (m.match.some((w) => n.includes(w))) return { name: key, ...m, known: true };
  const hardness = /stainless|inox/.test(n) ? "hard" : /alumin|copper/.test(n) ? "soft" : "medium";
  return { name: name ?? "Sheet", match: [], hardness, minRadius: 1, density: null, gauges: [], known: false };
}

/** K-factor for a material bent at inside radius `radius` from `thickness` (mm). */
function sheetKFactor(material, radius, thickness) {
  const m = typeof material === "string" || material == null ? sheetMaterial(material) : material;
  const ratio = radius / thickness;
  return SHEET_K_TABLE[m.hardness][ratio < 1 ? 0 : ratio < 3 ? 1 : 2];
}

/** Bend allowance (mm): the neutral axis's arc through a bend of `angle` degrees. */
function bendAllowance(angle, radius, thickness, k) {
  return rad(angle) * (radius + k * thickness);
}

/** Outside setback (mm): from the outer virtual sharp to where the bend begins. */
function outsideSetback(angle, radius, thickness) {
  return angle <= 90 ? (radius + thickness) * Math.tan(rad(angle) / 2) : radius + thickness;
}

/** Bend deduction (mm): what one bend takes off the sum of the outside dimensions. */
function bendDeduction(angle, radius, thickness, k) {
  return 2 * outsideSetback(angle, radius, thickness) - bendAllowance(angle, radius, thickness, k);
}

/** Standard V-die openings (mm) a press brake shop stocks. */
const SHEET_V_DIES = [4, 6, 8, 10, 12, 16, 20, 24, 32, 40, 50, 63, 80];

/**
 * How the shop bends: tooling and rules. Every field is optional; what is not
 * given keeps the default.
 *   radius(t, material)  inside radius it bends to (default: the punch makes
 *                        R = T, never under the material's minimum)
 *   vDie(t)              V-die opening (default 8·T, rounded up to a stocked die)
 *   minFlange(t, v, r)   shortest flange the die can hold (default 0.7·V + R... see below)
 *   holeToBend           hole edge to bend line, × T (default 2.5)
 *   reliefWidth          bend relief width, × T (default 1)
 *   reliefDepth          how far a relief runs past the bend line, × T (default 1)
 *   cornerGap            gap at a closed corner, mm (default 0.25)
 *   k                    a K-factor (number) or table like SHEET_K_TABLE, if the shop has measured its own
 *   sheetShop({ name: "Brake 1", vDie: (t) => 6 * t, k: 0.42 })
 */
const __sheetShop = {
  name: "Generic air bending",
  radius: (t, m) => Math.max(t, m.minRadius * t),
  vDie: (t) => SHEET_V_DIES.find((v) => v >= 8 * t - 1e-9) ?? 8 * t,
  // The flange must still span the die's shoulder after the bend: about
  // 0.7 × the V opening, measured as an outside dimension.
  minFlange: (t, v, r) => Math.max(0.7 * v, r + 2.5 * t),
  holeToBend: 2.5,
  reliefWidth: 1,
  reliefDepth: 1,
  cornerGap: 0.25,
  k: null,
};
function sheetShop(options = {}) {
  for (const [key, value] of Object.entries(options)) {
    if (!(key in __sheetShop)) throw new Error(`sheetShop: unknown setting “${key}” (${Object.keys(__sheetShop).join(", ")})`);
    if (typeof __sheetShop[key] === "function" && typeof value === "number") __sheetShop[key] = () => value;
    else __sheetShop[key] = value;
  }
  return { ...__sheetShop };
}

// 2D and 3D vector helpers (internal).
const __v2 = {
  add: (a, b) => [a[0] + b[0], a[1] + b[1]],
  sub: (a, b) => [a[0] - b[0], a[1] - b[1]],
  mul: (a, s) => [a[0] * s, a[1] * s],
  dot: (a, b) => a[0] * b[0] + a[1] * b[1],
  len: (a) => Math.hypot(a[0], a[1]),
  near: (a, b, eps = 1e-6) => Math.hypot(a[0] - b[0], a[1] - b[1]) <= eps,
};
const __v3 = {
  add: (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]],
  mul: (a, s) => [a[0] * s, a[1] * s, a[2] * s],
  cross: (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]],
  lin: (...terms) => terms.reduce((acc, [v, s]) => [acc[0] + v[0] * s, acc[1] + v[1] * s, acc[2] + v[2] * s], [0, 0, 0]),
};
const __round = (v, n = 3) => Math.round(v * 10 ** n) / 10 ** n;

/** A frame (origin and axes) placed in another frame's coordinates. */
function __compose3(outer, inner) {
  const at = (p) => __v3.lin([outer.o, 1], [outer.x, p[0]], [outer.y, p[1]], [outer.z, p[2]]);
  const dir = (v) => __v3.lin([outer.x, v[0]], [outer.y, v[1]], [outer.z, v[2]]);
  return { o: at(inner.o), x: dir(inner.x), y: dir(inner.y), z: dir(inner.z) };
}
function __compose2(outer, inner) {
  const at = (p) => __v2.add(outer.o, __v2.add(__v2.mul(outer.x, p[0]), __v2.mul(outer.y, p[1])));
  const dir = (v) => __v2.add(__v2.mul(outer.x, v[0]), __v2.mul(outer.y, v[1]));
  return { o: at(inner.o), x: dir(inner.x), y: dir(inner.y) };
}
const __apply2 = (f, p) => __v2.add(f.o, __v2.add(__v2.mul(f.x, p[0]), __v2.mul(f.y, p[1])));
const __identity3 = Object.freeze({ o: [0, 0, 0], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] });
const __identity2 = Object.freeze({ o: [0, 0], x: [1, 0], y: [0, 1] });
function __place3(solid, f) {
  const turned = f.x[0] === 1 && f.y[1] === 1 && f.z[2] === 1 ? solid : solid.rotate(...planeRotation(f.x, f.y));
  return turned.translate(f.o);
}
function __place2(profile, f) {
  const deg = (Math.atan2(f.x[1], f.x[0]) * 180) / Math.PI;
  return (Math.abs(deg) < 1e-12 ? profile : profile.rotate(deg)).translate(f.o[0], f.o[1]);
}
/** A rectangle along an edge: params p0…p1 along `d` from `origin`, offsets q0…q1 along `o`. */
function __edgeRect(origin, d, o, p0, p1, q0, q1) {
  const at = (p, q) => __v2.add(origin, __v2.add(__v2.mul(d, p), __v2.mul(o, q)));
  return [at(p0, q0), at(p1, q0), at(p1, q1), at(p0, q1)];
}
/** Distance from point q to segment a–b. */
function __segDist(q, a, b) {
  const ab = __v2.sub(b, a);
  const l2 = __v2.dot(ab, ab);
  const t = l2 > 0 ? Math.max(0, Math.min(1, __v2.dot(__v2.sub(q, a), ab) / l2)) : 0;
  return __v2.len(__v2.sub(q, __v2.add(a, __v2.mul(ab, t))));
}

const __sheets = {};
let __sheetCount = 0;

/** One flat face of a sheet-metal part: the base, or a flange. */
class SheetFace {
  constructor(sm, parent, spec) {
    this.sm = sm;
    this.parent = parent;
    this.spec = spec;
    this.children = [];
    this.cuts = [];
    this.tabs = [];
    this.forms = [];
    this.reliefs = [];
    this.notes = [];
    this.depth = parent ? parent.depth + 1 : 0;
    this.name = spec.name ?? (parent ? `Flange ${++sm.flangeCount}` : "Base");
    this.rootSb = spec.rootSb ?? 0;
  }
  /** A point in this face's own coordinates, from the coordinates people use. */
  __in(x, y) {
    return this.parent ? [-x, y - this.rootSb] : [x, y];
  }
  __dirIn(dx, dy) {
    return this.parent ? [-dx, dy] : [dx, dy];
  }
  __changed() {
    this.sm.built = null;
    return this;
  }

  /**
   * A flange bent from a straight edge of this face; returns the new face.
   *   edge: "+x" | "-x" | "+y" | "-y" (the base's outermost edge facing that
   *     way), "tip" | "start" | "end" (a flange's far edge and its sides), an
   *     edge index, or { near: [x, y] }.
   *   length: outside dimension from this face's outer side to the tip (mm).
   *   angle: bend angle, degrees (default 90; 180 folds it back flat).
   *   dir: "up" (default) | "down" | "in" | "out" (see the conventions above).
   *   radius, k: this bend's inside radius and K-factor (default the sheet's).
   *   from, to: where along the edge the flange runs (mm from the edge's
   *     start; a flange's tip measures like its own x); or inset: mm (or
   *     [start, end]) in from the edge's ends; or width (+ at: its middle).
   *   position: "inside" (default: outer face flush with the edge) |
   *     "outside" (inner face on the edge) | "bend" (the bend starts at the edge).
   *   relief: "rect" (default) | "obround" | "none", or { kind, width, depth }
   *     (mm), cut where the flange stops short of the edge's ends.
   *   overlap: at a corner with a neighbouring flange, true runs this one past
   *     the corner over the other's edge (default: the flange made first).
   *   name: what the face is called in the bend table.
   */
  flange(edge, options = {}) {
    const sm = this.sm;
    const o = { ...options };
    if (typeof o.length !== "number" || !(o.length > 0)) throw new Error("flange: give its length (mm, outside dimension)");
    const angle = o.angle ?? 90;
    if (typeof angle !== "number" || !(angle > 0 && angle <= 180)) throw new Error("flange: the angle is between 0 and 180 degrees");
    let dir = o.dir ?? "up";
    if (dir === "in" || dir === "out") {
      if (!this.parent) throw new Error(`flange: "${dir}" is for a flange on a flange; on the base say "up" or "down"`);
      const own = this.spec.dir;
      dir = dir === "in" ? own : own === "up" ? "down" : "up";
    }
    if (dir !== "up" && dir !== "down") throw new Error(`flange: dir is "up", "down", "in" or "out", not ${JSON.stringify(o.dir)}`);
    const radius = o.radius ?? sm.radius;
    if (!(radius >= 0)) throw new Error("flange: the radius is a number of mm, 0 or more");
    const k = o.k ?? (o.hem ? 0.5 : sm.kFor(radius));
    const sb = outsideSetback(angle, radius, sm.thickness);
    const position = o.position ?? "inside";
    const shift = { inside: 0, outside: sm.thickness, bend: sb }[position];
    if (shift === undefined) throw new Error(`flange: position is "inside", "outside" or "bend", not ${JSON.stringify(position)}`);
    const id = ++sm.created;
    const spec = { ...o, edge, angle, dir, radius, k, sb, shift, position, id, rootSb: sb };
    const face = new SheetFace(sm, this, spec);
    this.children.push(face);
    sm.faces.push(face);
    this.__changed();
    return face;
  }

  /**
   * A hem: the edge folded back on itself (180°) for a safe, stiff edge.
   *   length: outside, from the fold. gap: between the sheet and the hem
   *   (0 = closed). dir: "down" (default, under) or "up".
   */
  hem(edge, { length, gap = 0, dir = "down", ...rest } = {}) {
    const radius = Math.max(gap / 2, 0.01);
    return this.flange(edge, { ...rest, length, angle: 180, radius, dir, hem: true, k: rest.k ?? 0.5, name: rest.name ?? `Hem ${this.sm.flangeCount + 1}` });
  }

  /**
   * A jog (Z-bend): the sheet steps by `offset` mm (face to same face) and
   * runs on parallel to this face for `length` mm. Returns the last face.
   */
  jog(edge, { offset, length, angle = 90, dir = "up", name, ...rest } = {}) {
    if (!(offset > 0 && length > 0)) throw new Error("jog: give offset and length (mm)");
    const riser = this.flange(edge, { ...rest, length: offset + this.sm.thickness, angle, dir, name: name ? `${name} riser` : undefined });
    return riser.flange("tip", { length, angle, dir: "out", name, radius: rest.radius, k: rest.k });
  }

  /** A round hole of diameter `d` at (x, y). */
  hole(x, y, d) {
    if (!(d > 0)) throw new Error("hole: the diameter must be positive");
    const c = this.__in(x, y);
    this.cuts.push({ kind: "hole", c, r: d / 2, label: `Ø${__round(d, 2)} hole at (${__round(x, 2)}, ${__round(y, 2)})` });
    return this.__changed();
  }

  /** A slot of width `w` from (x1, y1) to (x2, y2) (round ends). */
  slot(x1, y1, x2, y2, w) {
    if (!(w > 0)) throw new Error("slot: the width must be positive");
    this.cuts.push({ kind: "slot", a: this.__in(x1, y1), b: this.__in(x2, y2), r: w / 2, label: `${__round(w, 2)} mm slot` });
    return this.__changed();
  }

  /** A rectangular cut-out w × h centred at (x, y), corners rounded to `r`. */
  rectCut(x, y, w, h, { r = 0 } = {}) {
    if (!(w > 0 && h > 0)) throw new Error("rectCut: width and height must be positive");
    this.cuts.push({ kind: "rect", c: this.__in(x, y), w, h, r: Math.min(r, w / 2, h / 2), label: `${__round(w, 2)}×${__round(h, 2)} cut-out at (${__round(x, 2)}, ${__round(y, 2)})` });
    return this.__changed();
  }

  /** A cut-out of any outline [[x, y], …]. */
  cut(points) {
    if (!Array.isArray(points) || points.length < 3) throw new Error("cut: an outline of at least 3 points");
    this.cuts.push({ kind: "poly", pts: points.map(([x, y]) => this.__in(x, y)), label: "cut-out" });
    return this.__changed();
  }

  /**
   * A tab standing proud of an edge, in the face's plane: width × depth (mm),
   * centred `at` mm along the edge (default its middle). Tabs and matching
   * slots (rectCut) lock two sheet parts together.
   */
  tab(edge, { width, depth, at, r = 0 } = {}) {
    if (!(width > 0 && depth > 0)) throw new Error("tab: give width and depth (mm)");
    this.tabs.push({ edge, width, depth, at, r });
    return this.__changed();
  }

  /**
   * A louvre: a vent lanced along one side and formed out of the sheet, at
   * (x, y). length along `angle` (degrees in this face's coordinates; 0 runs
   * along x), width across it, raised `height` (mm) on the open side, up
   * (default) or down. The flat pattern marks it for the forming tool (layer
   * FORM); it is not cut by the laser.
   */
  louvre(x, y, { length, width, height = width / 2, angle = 0, dir = "up" } = {}) {
    if (!(length > 0 && width > 0 && height > 0)) throw new Error("louvre: give length, width and height (mm)");
    if (dir !== "up" && dir !== "down") throw new Error('louvre: dir is "up" or "down"');
    const a = rad(angle);
    this.forms.push({ kind: "louvre", c: this.__in(x, y), u: this.__dirIn(Math.cos(a), Math.sin(a)), length, width, height, dir });
    return this.__changed();
  }

  // ---- Build (internal) ----

  /** This face's outline in its own coordinates, counter-clockwise. */
  __outline() {
    if (!this.parent) return this.sm.outline;
    const [x0, x1] = this.span;
    return [[-x1, 0], [-x0, 0], [-x0, this.flatLength], [-x1, this.flatLength]];
  }

  /** Its straight edges: { index, name, A, B, d, o, e, origin, pmin, pmax }. */
  __edges() {
    const pts = this.__outline();
    const n = pts.length;
    const edges = [];
    for (let i = 0; i < n; i++) {
      const A = pts[i];
      const B = pts[(i + 1) % n];
      const len = __v2.len(__v2.sub(B, A));
      if (len < 1e-9) continue;
      const d = __v2.mul(__v2.sub(B, A), 1 / len);
      const o = [d[1], -d[0]];
      const edge = { index: i, A, B, d, o, e: __v2.dot(o, A), origin: A, pmin: 0, pmax: len, len };
      if (this.parent) {
        edge.name = ["root", "start", "tip", "end"][i];
        if (i === 2) Object.assign(edge, { origin: [0, this.flatLength], pmin: this.span[0], pmax: this.span[1] });
      }
      edges.push(edge);
    }
    return edges;
  }

  __pickEdge(edges, which) {
    if (typeof which === "number") {
      const e = edges.find((x) => x.index === which);
      if (!e) throw new Error(`${this.name}: no edge ${which} (it has ${edges.length})`);
      return e;
    }
    if (typeof which === "string") {
      const axis = { "+x": [1, 0], "-x": [-1, 0], "+y": [0, 1], "-y": [0, -1] }[which];
      if (axis && !this.parent) {
        const facing = edges.filter((e) => __v2.dot(e.o, axis) > 0.9999);
        if (!facing.length) throw new Error(`${this.name}: no straight edge faces ${which}`);
        return facing.reduce((a, b) => (b.e > a.e + 1e-9 || (Math.abs(b.e - a.e) <= 1e-9 && b.len > a.len) ? b : a));
      }
      const named = edges.find((e) => e.name === which);
      if (named && named.name !== "root") return named;
      throw new Error(`${this.name}: no edge “${which}” (${this.parent ? '"tip", "start", "end"' : '"+x", "-x", "+y", "-y"'}, an index, or { near: [x, y] })`);
    }
    if (which && Array.isArray(which.near)) {
      const q = this.__in(which.near[0], which.near[1]);
      return edges.filter((e) => e.name !== "root").reduce((a, b) => (__segDist(q, b.A, b.B) < __segDist(q, a.A, a.B) ? b : a));
    }
    throw new Error(`${this.name}: pick an edge by name, index or { near: [x, y] }`);
  }

  /** Where along its edge a flange runs (params along the edge). */
  static __span(edge, spec) {
    let lo = edge.pmin;
    let hi = edge.pmax;
    if (spec.from !== undefined || spec.to !== undefined) {
      lo = spec.from ?? lo;
      hi = spec.to ?? hi;
    } else if (spec.width !== undefined) {
      const mid = spec.at ?? (edge.pmin + edge.pmax) / 2;
      lo = mid - spec.width / 2;
      hi = mid + spec.width / 2;
    } else if (spec.inset !== undefined) {
      const [a, b] = Array.isArray(spec.inset) ? spec.inset : [spec.inset, spec.inset];
      lo += a;
      hi -= b;
    }
    if (!(hi - lo > 1e-6)) throw new Error(`flange ${spec.name ?? spec.id}: it runs nowhere along its edge (${__round(lo)} to ${__round(hi)})`);
    if (lo < edge.pmin - 1e-6 || hi > edge.pmax + 1e-6) throw new Error(`flange ${spec.name ?? spec.id}: it runs past the ends of its edge (${__round(edge.pmin)} to ${__round(edge.pmax)})`);
    return [lo, hi];
  }
}

/** A sheet-metal part (see sheet()). */
class SheetMetal {
  constructor(outline, options) {
    const t = options.thickness;
    if (typeof t !== "number" || !(t > 0)) throw new Error("sheet: give the thickness (mm)");
    this.thickness = t;
    this.material = sheetMaterial(options.material);
    this.shop = { ...__sheetShop, ...(options.shop ?? {}) };
    this.radius = options.radius ?? this.shop.radius(t, this.material);
    this.k = options.k ?? null;
    this.fold = options.fold ?? 1;
    if (!(this.fold >= 0 && this.fold <= 1)) throw new Error("sheet: fold runs from 0 (flat) to 1 (bent)");
    this.cornerGap = options.cornerGap ?? this.shop.cornerGap;
    this.corner = options.corner ?? "overlap";
    if (!["overlap", "open"].includes(this.corner)) throw new Error('sheet: corner is "overlap" or "open"');
    this.outline = __ccw(outline);
    this.faces = [];
    this.created = 0;
    this.flangeCount = 0;
    this.built = null;
    this.id = ++__sheetCount;
    this.base = new SheetFace(this, null, { name: options.name ?? "Base", dir: "up" });
    // Where the bent part sits in the program's coordinates (sheetProfile turns it onto its points).
    this.placeSolid = (s) => s;
    this.faces.push(this.base);
  }
  /** K-factor for a bend of inside radius r: the sheet's, the shop's, or the table's. */
  kFor(r) {
    if (typeof this.k === "number") return this.k;
    const shopK = this.shop.k;
    if (typeof shopK === "number") return shopK;
    const table = shopK && typeof shopK === "object" ? shopK : SHEET_K_TABLE;
    const ratio = r / this.thickness;
    return table[this.material.hardness][ratio < 1 ? 0 : ratio < 3 ? 1 : 2];
  }
  // The base face's tools, on the part itself.
  flange(edge, o) { return this.base.flange(edge, o); }
  hem(edge, o) { return this.base.hem(edge, o); }
  jog(edge, o) { return this.base.jog(edge, o); }
  hole(x, y, d) { this.base.hole(x, y, d); return this; }
  slot(...a) { this.base.slot(...a); return this; }
  rectCut(...a) { this.base.rectCut(...a); return this; }
  cut(points) { this.base.cut(points); return this; }
  tab(edge, o) { this.base.tab(edge, o); return this; }
  louvre(x, y, o) { this.base.louvre(x, y, o); return this; }

  /** The bent part as a Solid (base flange on z = 0…T). */
  solid() {
    return this.placeSolid(this.__build().solid);
  }
  /** The flat pattern as a Profile (bend allowances included), its corner at the origin. */
  flat() {
    return this.__build().flat;
  }
  /** The flat blank as a Solid, T thick. */
  flatSolid() {
    return this.__build().flatSolid;
  }
  /** Bend table, flat size, K-factors and checks, as plain data. */
  get data() {
    return this.__build().data;
  }

  /**
   * The bent part as a part() of an assembly, process "sheet_metal", with its
   * flat pattern and bend table attached (DXF for the laser, drawings, checks).
   *   box.part("Base", { material: "Aluminium 5052-H32", rotate: [0, 0, 90], at: [0, 0, 5] })
   */
  part(name, { material, process = "sheet_metal", rotate, at, stock: bought } = {}) {
    const b = this.__build();
    let s = this.solid();
    if (rotate) s = s.rotate(...vec3(rotate));
    if (at) s = s.translate(at);
    const made = part(name, s, { material: material ?? this.material.name, process, stock: bought });
    __sheets[made.name] = { flat: b.flatSolid.h, data: { ...b.data, part: made.name, material: material ?? this.material.name } };
    return made;
  }
  /** The bent part as the program's single solid, its flat pattern attached. */
  single({ rotate, at } = {}) {
    const b = this.__build();
    let s = this.solid();
    if (rotate) s = s.rotate(...vec3(rotate));
    if (at) s = s.translate(at);
    __sheets[""] = { flat: b.flatSolid.h, data: b.data };
    return s;
  }

  __build() {
    if (this.built) return this.built;
    const T = this.thickness;
    const pieces3 = []; // { name, solid, kind }
    const piecesFlat = []; // { name, profile }
    const bends = [];
    const forms = [];
    const checks = [];
    const check = (severity, code, message, at) => checks.push({ severity, code, message, ...(at ? { at } : {}) });
    const sm = this;
    const minFlange = this.shop.minFlange(T, this.shop.vDie(T), this.radius);

    if (this.radius < this.material.minRadius * T - 1e-9) {
      check("error", "BEND_RADIUS_TOO_SMALL", `inside radius ${__round(this.radius)} mm is under ${this.material.name}'s minimum ${__round(this.material.minRadius * T)} mm (${this.material.minRadius}×T): it cracks on the outside`);
    }
    if (this.material.known && this.material.gauges.length && !this.material.gauges.some((g) => Math.abs(g - T) < 1e-6)) {
      check("info", "NONSTANDARD_GAUGE", `${__round(T)} mm ${this.material.name} is not a stocked thickness (${this.material.gauges.join(", ")} mm)`);
    }
    if (!this.material.known) check("info", "UNKNOWN_MATERIAL", `“${this.material.name}” is not in the sheet table: K-factors assume a ${this.material.hardness} material`);

    const visit = (face, world3, flat2) => {
      face.world3 = world3;
      face.flat2 = flat2;
      const edges = face.__edges();
      const outline = face.__outline();
      const adds = [];
      const removes = [];

      // Tabs.
      for (const t of face.tabs) {
        const e = face.__pickEdge(edges, t.edge);
        const mid = t.at ?? (e.pmin + e.pmax) / 2;
        const pts = __edgeRect(e.origin, e.d, e.o, mid - t.width / 2, mid + t.width / 2, e.e - __v2.dot(e.o, e.origin) - 1e-3, e.e - __v2.dot(e.o, e.origin) + t.depth);
        adds.push(polygon(pts));
      }

      // Each flange: its edge, where it runs, its bend.
      for (const c of face.children) {
        const s = c.spec;
        c.edge = face.__pickEdge(edges, s.edge);
        c.span = SheetFace.__span(c.edge, s);
        c.bendSpan = [...c.span];
        c.flatLength = s.length - c.rootSb;
        if (!(c.flatLength > 1e-6)) throw new Error(`${c.name}: ${__round(s.length)} mm is shorter than its bend (${__round(c.rootSb)} mm from the outside to where the bend ends)`);
      }

      // Corners: two flanges meeting at a corner of this face.
      for (const a of face.children) {
        for (const b of face.children) {
          if (a === b || !__v2.near(a.edge.B, b.edge.A, 1e-6)) continue;
          const aEnd = Math.abs(a.span[1] - a.edge.pmax) < 1e-6;
          const bStart = Math.abs(b.span[0] - b.edge.pmin) < 1e-6;
          if (!aEnd || !bStart) continue;
          const square = Math.abs(__v2.dot(a.edge.d, b.edge.d)) < 1e-6 && a.edge.d[0] * b.edge.d[1] - a.edge.d[1] * b.edge.d[0] > 0;
          const treatable = square && Math.abs(a.spec.angle - 90) < 0.5 && Math.abs(b.spec.angle - 90) < 0.5 && a.spec.dir === b.spec.dir && a.spec.shift === 0 && b.spec.shift === 0;
          const g = sm.cornerGap;
          const rc = Math.max(0.5 * T, g);
          if (!treatable) {
            check("warning", "CORNER_UNTREATED", `${a.name} and ${b.name} meet at a corner that is not square, or bend differently: trim them (from/to/inset) so they clear`);
            continue;
          }
          // Who runs past the corner: the one marked overlap, else the one made first.
          const aOver = a.spec.overlap ?? (b.spec.overlap === true ? false : a.spec.id < b.spec.id);
          const trim = (f, end, other) => {
            const k = end === "start" ? 0 : 1;
            const sign = end === "start" ? 1 : -1;
            f.span[k] += sign * (T + g);
            f.bendSpan[k] += sign * (other.spec.sb + rc);
            // The corner relief: the parent's material beside the trimmed bend.
            const e = f.edge;
            const p0 = end === "start" ? e.pmin : e.pmax - (other.spec.sb + rc);
            const p1 = end === "start" ? e.pmin + other.spec.sb + rc : e.pmax;
            const sAt = e.e + f.spec.shift - f.spec.sb;
            face.reliefs.push(__edgeRect(e.origin, e.d, e.o, p0, p1, sAt - __v2.dot(e.o, e.origin), e.e - __v2.dot(e.o, e.origin) + T));
          };
          if (sm.corner === "open") {
            trim(a, "end", b);
            trim(b, "start", a);
          } else if (aOver) trim(b, "start", a);
          else trim(a, "end", b);
        }
      }

      // Bends: the parent's flat stops at the bend line; reliefs where a flange stops short.
      for (const c of face.children) {
        const s = c.spec;
        const e = c.edge;
        const q = (v) => v - __v2.dot(e.o, e.origin); // an o-coordinate, relative to the edge's origin
        const ev = e.e + s.shift;
        const sAt = ev - s.sb; // where this face's flat ends
        const [b0, b1] = c.bendSpan;
        if (sAt < e.e - 1e-9) removes.push(polygon(__edgeRect(e.origin, e.d, e.o, b0, b1, q(sAt), q(e.e) + 0.5 * T)));
        else if (sAt > e.e + 1e-9) adds.push(polygon(__edgeRect(e.origin, e.d, e.o, b0, b1, q(e.e) - 1e-3, q(sAt))));
        const relief = typeof s.relief === "object" ? s.relief : { kind: s.relief ?? "rect" };
        const rw = relief.width ?? sm.shop.reliefWidth * T;
        const rd = relief.depth ?? sm.shop.reliefDepth * T;
        for (const [end, p] of [["start", b0], ["end", b1]]) {
          const atVertex = end === "start" ? Math.abs(p - e.pmin) < 1e-6 : Math.abs(p - e.pmax) < 1e-6;
          const cornered = end === "start" ? c.span[0] !== p : c.span[1] !== p;
          if (atVertex || cornered) continue;
          if (relief.kind === "none") {
            check("error", "NO_BEND_RELIEF", `${c.name} stops ${__round(end === "start" ? p - e.pmin : e.pmax - p)} mm short of its edge's ${end} with no bend relief: the sheet tears there`);
            continue;
          }
          if (rw < T - 1e-9) check("warning", "RELIEF_TOO_NARROW", `${c.name}: bend relief ${__round(rw)} mm wide is under the thickness ${__round(T)} mm`);
          const [p0, p1] = end === "start" ? [p - rw, p] : [p, p + rw];
          const lo = Math.min(sAt, e.e) - rd;
          let cut = polygon(__edgeRect(e.origin, e.d, e.o, p0, p1, q(lo + (relief.kind === "obround" ? rw / 2 : 0)), q(e.e) + 0.5 * T));
          if (relief.kind === "obround") {
            const mid = __v2.add(e.origin, __v2.add(__v2.mul(e.d, (p0 + p1) / 2), __v2.mul(e.o, q(lo + rw / 2))));
            cut = cut.add(circle(rw / 2).translate(mid[0], mid[1]));
          }
          removes.push(cut);
        }
      }
      for (const r of face.reliefs) removes.push(polygon(r));

      // The face's own material, flat, in its own coordinates.
      let region = polygon(outline);
      if (adds.length) region = region.add(...adds);
      const cutProfiles = face.cuts.map((c) => __cutProfile(c));
      const all = [...removes, ...cutProfiles];
      if (all.length) region = region.subtract(...all);
      if (!(region.area > 1e-9)) throw new Error(`${face.name}: nothing is left of it after its bends, reliefs and cut-outs`);

      // 3D: the plate, louvres formed out of it.
      let plate = region.extrude(T);
      for (const f of face.forms) {
        const [hood, opening] = __louvre(f, T);
        plate = plate.subtract(opening).add(hood);
      }
      pieces3.push({ name: face.name, solid: __place3(plate, world3), kind: "face", face });
      piecesFlat.push({ name: face.name, profile: __place2(region, flat2), kind: "face", face });
      for (const f of face.forms) {
        const half = [__v2.mul(f.u, f.length / 2), __v2.mul([-f.u[1], f.u[0]], f.width / 2)];
        const corners = [[-1, -1], [1, -1], [1, 1], [-1, 1]].map(([i, j]) => __apply2(flat2, __v2.add(f.c, __v2.add(__v2.mul(half[0], i), __v2.mul(half[1], j)))));
        forms.push({ kind: f.kind, face: face.name, outline: corners, dir: f.dir, height: f.height });
      }

      // Checks on this face: holes near bends and edges.
      const bendLines = [];
      if (face.parent) bendLines.push({ name: `its bend from ${face.parent.name}`, a: [-face.bendSpan[0], 0], b: [-face.bendSpan[1], 0], o: [0, -1], radius: face.spec.radius });
      for (const c of face.children) {
        const e = c.edge;
        const sAt = e.e + c.spec.shift - c.spec.sb;
        const P = (p) => __v2.add(e.origin, __v2.add(__v2.mul(e.d, p), __v2.mul(e.o, sAt - __v2.dot(e.o, e.origin))));
        bendLines.push({ name: `the bend to ${c.name}`, a: P(c.bendSpan[0]), b: P(c.bendSpan[1]), o: e.o, radius: c.spec.radius });
      }
      const need = sm.shop.holeToBend * T;
      for (const cut of face.cuts) {
        for (const line of bendLines) {
          const gap = __cutToLine(cut, line);
          if (gap === null) continue;
          const at = __apply2(flat2, cut.c ?? cut.a ?? cut.pts[0]);
          if (gap < 0) check("error", "CUT_IN_BEND", `${face.name}: the ${cut.label} runs into ${line.name}: it tears and distorts when bent`, at);
          else if (gap < need - 1e-6) check("warning", "HOLE_NEAR_BEND", `${face.name}: the ${cut.label} is ${__round(gap, 2)} mm from ${line.name}; keep ${__round(need, 2)} mm (${sm.shop.holeToBend}×T) or it pulls oval`, at);
        }
        if (cut.kind === "hole") {
          for (let i = 0; i < outline.length; i++) {
            const A = outline[i];
            const B = outline[(i + 1) % outline.length];
            const bent = face.children.some((c) => c.edge.index === i) || (face.parent && i === 0);
            if (bent) continue;
            const gap = __segDist(cut.c, A, B) - cut.r;
            if (gap < T - 1e-6) check("warning", "HOLE_NEAR_EDGE", `${face.name}: the ${cut.label} is ${__round(gap, 2)} mm from the edge; keep at least the thickness (${__round(T)} mm)`, __apply2(flat2, cut.c));
          }
        }
      }

      // Children: frames, bend solids, bend strips in the flat.
      for (const c of face.children) {
        const s = c.spec;
        const e = c.edge;
        const sAt = e.e + s.shift - s.sb;
        const [b0, b1] = c.bendSpan;
        const ba = bendAllowance(s.angle, s.radius, T, s.k);
        const theta = rad(s.angle * sm.fold);
        const up = s.dir === "up";
        const d3 = [e.d[0], e.d[1], 0];
        const o3 = [e.o[0], e.o[1], 0];
        const z3 = [0, 0, 1];
        const base2 = __v2.add(e.origin, __v2.mul(e.o, sAt - __v2.dot(e.o, e.origin))); // the bend line at param 0
        const base3 = [base2[0], base2[1], 0];
        let local3;
        let bend3;
        if (theta < 1e-9) {
          // Flat: the bend is a strip BA wide.
          local3 = { o: __v3.lin([base3, 1], [o3, ba]), x: __v3.mul(d3, -1), y: o3, z: z3 };
          bend3 = polygon(__edgeRect(base2, e.d, e.o, b0, b1, 0, ba)).extrude(T);
        } else {
          // Partly folded, the neutral axis keeps its length: R grows as θ shrinks.
          const R = sm.fold >= 1 ? s.radius : ba / theta - s.k * T;
          const [st, ct] = [Math.sin(theta), Math.cos(theta)];
          const u = up ? __v3.lin([o3, ct], [z3, st]) : __v3.lin([o3, ct], [z3, -st]);
          const n = up ? __v3.lin([o3, -st], [z3, ct]) : __v3.lin([o3, st], [z3, ct]);
          const delta = up ? __v3.lin([o3, (R + T) * st], [z3, (R + T) * (1 - ct)]) : __v3.lin([o3, R * st], [z3, -R * (1 - ct)]);
          local3 = { o: __v3.add(base3, delta), x: __v3.mul(d3, -1), y: u, z: n };
          // The bend: an annular sector about its axis.
          const axis = __v3.add(base3, __v3.mul(z3, up ? T + R : -R));
          const X = up ? __v3.mul(z3, -1) : z3;
          const Z = __v3.cross(X, o3);
          const startP = Z[0] * d3[0] + Z[1] * d3[1] > 0 ? b0 : b1;
          const frame = { o: __v3.add(axis, __v3.mul(d3, startP)), x: X, y: o3, z: Z };
          const sector = rect(T, b1 - b0).translate(R, 0).revolve(s.angle * sm.fold, { segments: segmentsFor(R + T) });
          bend3 = __place3(sector, frame);
        }
        pieces3.push({ name: `bend to ${c.name}`, solid: __place3(bend3, world3), kind: "bend", face: c, parent: face });
        const strip = polygon(__edgeRect(base2, e.d, e.o, b0, b1, 0, ba));
        piecesFlat.push({ name: `bend to ${c.name}`, profile: __place2(strip, flat2), kind: "bend", face: c, parent: face });
        const local2 = { o: __v2.add(base2, __v2.mul(e.o, ba)), x: __v2.mul(e.d, -1), y: e.o };
        const P = (p, q) => __apply2(flat2, __v2.add(base2, __v2.add(__v2.mul(e.d, p), __v2.mul(e.o, q))));
        bends.push({
          from: face.name,
          to: c.name,
          dir: s.dir,
          angle: s.angle,
          radius: __round(s.radius, 4),
          k: s.k,
          allowance: __round(ba, 4),
          deduction: __round(2 * s.sb - ba, 4),
          setback: __round(s.sb, 4),
          length: __round(b1 - b0, 4),
          line: [P(b0, ba / 2), P(b1, ba / 2)],
          tangents: [[P(b0, 0), P(b1, 0)], [P(b0, ba), P(b1, ba)]],
          depth: c.depth,
          hem: !!s.hem,
        });
        if (s.hem) {
          if (s.length < 4 * T - 1e-6) check("warning", "HEM_TOO_SHORT", `${c.name}: a ${__round(s.length)} mm hem is under 4×T (${__round(4 * T)} mm); the brake cannot flatten it`);
        } else if (s.length < minFlange - 1e-6) {
          check("error", "FLANGE_TOO_SHORT", `${c.name}: ${__round(s.length)} mm is shorter than the shop's minimum flange ${__round(minFlange, 2)} mm for ${__round(T)} mm sheet (V-die ${__round(sm.shop.vDie(T))} mm)`);
        }
        c.bendFrame = local3;
        visit(c, __compose3(world3, local3), __compose2(flat2, local2));
      }
    };
    visit(this.base, __identity3, __identity2);

    // Bend order: the outermost bends (lips, hems) first, then shorter before longer.
    const order = [...bends].sort((a, b) => b.depth - a.depth || a.length - b.length);
    order.forEach((b, i) => (b.order = i + 1));
    bends.forEach((b, i) => (b.id = `B${i + 1}`));

    // The flat pattern, its corner at the origin.
    let flat = piecesFlat[0].profile;
    if (piecesFlat.length > 1) flat = flat.add(...piecesFlat.slice(1).map((p) => p.profile));
    const { min, max } = flat.bounds;
    const shift = (pt) => [__round(pt[0] - min[0], 4), __round(pt[1] - min[1], 4)];
    flat = flat.translate(-min[0], -min[1]);
    for (const b of bends) {
      b.line = b.line.map(shift);
      b.tangents = b.tangents.map((l) => l.map(shift));
      delete b.depth;
    }
    for (const f of forms) f.outline = f.outline.map(shift);
    for (const c of checks) if (c.at) c.at = shift(c.at);

    // Self-intersection: flat (pieces overlapping when laid out) and folded.
    for (let i = 0; i < piecesFlat.length; i++) {
      for (let j = i + 1; j < piecesFlat.length; j++) {
        const [a, b] = [piecesFlat[i], piecesFlat[j]];
        const ba = a.profile.bounds;
        const bb = b.profile.bounds;
        if (ba.max[0] < bb.min[0] || bb.max[0] < ba.min[0] || ba.max[1] < bb.min[1] || bb.max[1] < ba.min[1]) continue;
        const area = a.profile.intersect(b.profile).area;
        if (area > 0.01) check("error", "FLAT_OVERLAP", `the flat pattern overlaps itself: ${a.name} and ${b.name} share ${__round(area, 2)} mm² laid flat (shorten or trim one)`);
      }
    }
    for (let i = 0; i < pieces3.length; i++) {
      for (let j = i + 1; j < pieces3.length; j++) {
        const [a, b] = [pieces3[i], pieces3[j]];
        const ba = a.solid.bounds;
        const bb = b.solid.bounds;
        if ([0, 1, 2].some((k) => ba.max[k] < bb.min[k] - 1e-6 || bb.max[k] < ba.min[k] - 1e-6)) continue;
        const v = a.solid.intersect(b.solid).volume;
        if (v > 1e-3) check("error", "FOLDED_CLASH", `bent up, ${a.name} runs into ${b.name} (${__round(v, 3)} mm³): trim one or change the bend order`);
      }
    }

    const solid = union(pieces3.map((p) => p.solid));
    const flatSolid = flat.extrude(T);
    const size = [__round(max[0] - min[0], 4), __round(max[1] - min[1], 4)];
    const data = {
      thickness: T,
      material: this.material.name,
      radius: __round(this.radius, 4),
      k: __round(this.kFor(this.radius), 4),
      shop: this.shop.name,
      vDie: this.shop.vDie(T),
      minFlange: __round(minFlange, 3),
      flat: { size, area: __round(flat.area, 3) },
      faces: this.faces.map((f) => ({ name: f.name, parent: f.parent?.name ?? null })),
      bends,
      forms,
      checks,
    };
    this.built = { solid, flat, flatSolid, data };
    return this.built;
  }
}

/** Counter-clockwise outline points from a list, { width, depth }, or a rect. */
function __ccw(outline) {
  let pts;
  if (Array.isArray(outline)) pts = outline.map((p) => [p[0], p[1]]);
  else if (outline && typeof outline.width === "number" && typeof outline.depth === "number") {
    const [w, d] = [outline.width, outline.depth];
    pts = [[0, 0], [w, 0], [w, d], [0, d]];
  } else throw new Error("sheet: the base outline is [[x, y], …] or { width, depth } (mm)");
  if (pts.length < 3) throw new Error("sheet: the base outline needs at least 3 points");
  return windingArea(pts) >= 0 ? pts : pts.reverse();
}

function __cutProfile(c) {
  if (c.kind === "hole") return circle(c.r).translate(c.c[0], c.c[1]);
  if (c.kind === "slot") {
    const len = __v2.len(__v2.sub(c.b, c.a));
    const ang = (Math.atan2(c.b[1] - c.a[1], c.b[0] - c.a[0]) * 180) / Math.PI;
    const body = len > 1e-9 ? rect(len, 2 * c.r).translate(0, -c.r).add(circle(c.r).translate(len, 0)) : rect(0, 0);
    return (len > 1e-9 ? body.add(circle(c.r)) : circle(c.r)).rotate(ang).translate(c.a[0], c.a[1]);
  }
  if (c.kind === "rect") {
    let p = rect(c.w, c.h, { center: true });
    if (c.r > 0) p = rect(c.w - 2 * c.r, c.h - 2 * c.r, { center: true }).offset(c.r);
    return p.translate(c.c[0], c.c[1]);
  }
  return polygon(c.pts);
}

/**
 * Smallest distance from a cut-out to a bend line (on the flat side, along
 * its normal into the face), or null if the cut-out is beside the bend, not
 * in front of it. Negative: the cut-out reaches into the bend.
 */
function __cutToLine(c, line) {
  const along = __v2.sub(line.b, line.a);
  const len = __v2.len(along);
  const t = __v2.mul(along, 1 / len);
  const inward = __v2.mul(line.o, -1);
  const pts =
    c.kind === "hole" ? [c.c] : c.kind === "slot" ? [c.a, c.b] : c.kind === "rect" ? [[-1, -1], [1, -1], [1, 1], [-1, 1]].map(([i, j]) => [c.c[0] + (i * c.w) / 2, c.c[1] + (j * c.h) / 2]) : c.pts;
  const pad = c.kind === "hole" || c.kind === "slot" ? c.r : 0;
  let best = null;
  for (const q of pts) {
    const rel = __v2.sub(q, line.a);
    const s = __v2.dot(rel, t);
    if (s < -pad || s > len + pad) continue;
    const dist = __v2.dot(rel, inward) - pad;
    best = best === null ? dist : Math.min(best, dist);
  }
  return best;
}

/** A louvre in face coordinates: [the formed hood, the opening under it]. */
function __louvre(f, T) {
  const ang = (Math.atan2(f.u[1], f.u[0]) * 180) / Math.PI;
  const { length: L, width: W, height: H } = f;
  // Cross-section across the louvre (v, z): a slab rising from the hinge side to the open side.
  const slab = polygon([[-W / 2, 0], [W / 2, H], [W / 2, H + T], [-W / 2, T]]);
  const sideWall = polygon([[-W / 2, T], [W / 2, T], [W / 2, H + T]]);
  // Built with the length along X: the section in the YZ plane, extruded along X.
  const along = (p, from, len) => p.extrude(len).rotate(90, 0, 90).translate(from, 0, 0);
  let hood = along(slab, -L / 2, L).add(along(sideWall, -L / 2, T), along(sideWall, L / 2 - T, T));
  if (f.dir === "down") hood = hood.mirror(0, 0, 1).translate(0, 0, T);
  const opening = box(L - 2 * T, W, 3 * T + 2 * H, { center: true });
  const place = (s) => s.rotate(0, 0, ang).translate(f.c[0], f.c[1], 0);
  return [place(hood), place(opening)];
}

/**
 * A sheet-metal part from its base flange: a closed outline [[x, y], …] (or
 * { width, depth }) on z = 0…thickness. Options: thickness (mm, required),
 * material (see SHEET_MATERIALS), radius (inside bend radius; default the
 * shop's for the material), k (K-factor; default from SHEET_K_TABLE), fold
 * (0 flat … 1 bent, for showing it fold), corner ("overlap" | "open"),
 * cornerGap (mm), shop (overrides as in sheetShop), name.
 * Returns the part; its base face's tools (flange, hem, jog, hole, slot,
 * rectCut, cut, tab, louvre) are on it.
 */
function sheet(outline, options = {}) {
  return new SheetMetal(outline, options);
}

/**
 * A sheet-metal part from an open profile (a base flange from an open
 * sketch): `points` [[x, z], …] trace the sheet's bottom face (its flat
 * pattern's underside) in the XZ plane, bent at every corner with the inside
 * radius; `width` runs along +Y. Left turns bend up, right turns down.
 *   sheetProfile([[0, 30], [0, 0], [40, 0]], 25, { thickness: 2, material: "S355" })   // an L bracket
 * Returns the part (its first segment is the base; faces in order are
 * part.segments).
 */
function sheetProfile(points, width, options = {}) {
  if (!Array.isArray(points) || points.length < 2) throw new Error("sheetProfile: at least two points");
  if (!(width > 0)) throw new Error("sheetProfile: give the width (mm)");
  const T = options.thickness;
  const segs = [];
  for (let i = 0; i + 1 < points.length; i++) {
    const a = points[i];
    const b = points[i + 1];
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
    if (len < 1e-9) throw new Error(`sheetProfile: points ${i} and ${i + 1} are the same`);
    segs.push({ len, dir: [(b[0] - a[0]) / len, (b[1] - a[1]) / len] });
  }
  // Turns at the inner corners: angle and side.
  const turns = [];
  for (let i = 1; i < segs.length; i++) {
    const [p, q] = [segs[i - 1].dir, segs[i].dir];
    const cross = p[0] * q[1] - p[1] * q[0];
    const angle = (Math.acos(Math.max(-1, Math.min(1, p[0] * q[0] + p[1] * q[1]))) * 180) / Math.PI;
    if (angle < 1e-6) throw new Error(`sheetProfile: no bend at point ${i} (the line runs straight on)`);
    if (angle > 179.999) throw new Error(`sheetProfile: point ${i} turns straight back (use hem())`);
    turns.push({ angle, dir: cross > 0 ? "up" : "down" });
  }
  // Placeholder outline to learn the radius and K the sheet will use.
  const probe = new SheetMetal({ width: 1, depth: 1 }, options);
  const R = options.radius ?? probe.radius;
  // From the bottom-face corner back to where the bend starts: R+T for an
  // up bend (the bottom is outside), R for a down bend (the bottom is inside).
  const tangent = (t) => (t.dir === "up" ? R + T : R) * Math.tan(rad(t.angle) / 2);
  const sbOf = (t) => outsideSetback(t.angle, R, T);
  const flatLen = segs.map((s, i) => s.len - (i > 0 ? tangent(turns[i - 1]) : 0) - (i < turns.length ? tangent(turns[i]) : 0));
  flatLen.forEach((l, i) => {
    if (!(l > 1e-6)) throw new Error(`sheetProfile: segment ${i + 1} (${__round(segs[i].len)} mm) is too short for its bends at radius ${__round(R)} mm`);
  });
  const first = flatLen[0] + (turns.length ? sbOf(turns[0]) : 0);
  const sm = new SheetMetal({ width: first, depth: width }, options);
  sm.segments = [sm.base];
  let face = sm.base;
  for (let i = 0; i < turns.length; i++) {
    const t = turns[i];
    const tail = i + 1 < turns.length ? sbOf(turns[i + 1]) : 0;
    face = face.flange(i === 0 ? "+x" : "tip", { length: sbOf(t) + flatLen[i + 1] + tail, angle: t.angle, dir: t.dir, radius: R, name: `Segment ${i + 2}` });
    sm.segments.push(face);
  }
  sm.base.name = "Segment 1";
  // Placed so the bottom face follows the points: rotate the first segment onto its direction.
  const a0 = (Math.atan2(segs[0].dir[1], segs[0].dir[0]) * 180) / Math.PI;
  const [x0, z0] = points[0];
  sm.placeSolid = (s) => (Math.abs(a0) > 1e-12 ? s.rotate(0, -a0, 0) : s).translate(x0, 0, z0);
  return sm;
}

/** The flat patterns of the sheet parts this program returns (for __result). */
function __sheetResult(names, single) {
  const out = {};
  for (const [name, s] of Object.entries(__sheets)) {
    if (name === "" ? single : names && names.has(name)) out[name] = s;
  }
  return Object.keys(out).length ? out : undefined;
}
