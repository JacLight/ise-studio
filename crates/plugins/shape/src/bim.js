// Building information: what parts are as building elements, and the
// building's structure around them. Loaded after the prelude.
//
// Spatial structure:  site({ name, latitude, longitude, elevation, address, north })
//                     building({ name, address })
//                     const L1 = level("First floor", 3000, { height: 2800 })
// Elements (each returns a list of parts: put them in what the program returns):
//   wall, walls, slab, roof, column, beam, stair, ramp, railing, curtainWall, rebar
//   door(…), window(…), opening(…) go in a wall's `openings`: they cut it and fill the hole
// Not parts:          space(…), zone(…), grid(…) declare rooms, zones and grid lines
// Any part:           part(name, solid, { material, ifc: { class: "IfcFurniture", level, psets, … } })
// Read an IFC file:   return importIfc("<sha>")
//
// All lengths in mm; a level's elevation is project z.

const __bim = { site: null, building: null, levels: [], elements: {}, spaces: [], openings: [], zones: [], grids: [], groups: [], tags: {} };

function __ifcClass(c) {
  if (typeof c !== "string" || !c.trim()) throw new TypeError("an IFC class is a name like \"IfcWall\" (or \"wall\")");
  const t = c.trim();
  if (/^ifc/i.test(t)) return "Ifc" + t.slice(3);
  return "Ifc" + t.split(/[\s_-]+/).map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join("");
}

function __pvalue(where, v) {
  if (typeof v === "boolean" || typeof v === "string") return v;
  if (typeof v === "number" && Number.isFinite(v)) return v;
  throw new TypeError(`${where}: a property is true/false, a number or text`);
}

function __psetsOf(where, ps) {
  if (ps === undefined || ps === null) return {};
  if (typeof ps !== "object") throw new TypeError(`${where}: psets is { Pset_WallCommon: { IsExternal: true }, … }`);
  const out = {};
  for (const [set, props] of Object.entries(ps)) {
    if (!props || typeof props !== "object") throw new TypeError(`${where}: property set ${set} is { name: value }`);
    out[set] = {};
    for (const [k, v] of Object.entries(props)) if (v !== undefined) out[set][k] = __pvalue(`${where} ${set}.${k}`, v);
  }
  return out;
}

function __mergePsets(a, b) {
  const out = { ...a };
  for (const [k, v] of Object.entries(b ?? {})) out[k] = { ...(out[k] ?? {}), ...v };
  return out;
}

const __levelName = (l) => (l === undefined || l === null ? undefined : typeof l === "string" ? l : l.name);

/** The level something is on: a level object, a declared level's name, or none (elevation 0). */
function __level(l) {
  if (l === undefined || l === null) return { name: undefined, elevation: 0, height: undefined };
  if (typeof l === "string") {
    const found = __bim.levels.find((x) => x.name === l);
    if (!found) throw new Error(`there is no level "${l}" (declare it with level("${l}", elevation))`);
    return found;
  }
  if (typeof l === "object" && typeof l.elevation === "number") return l;
  throw new TypeError("level is a level(…) or a level's name");
}

function __classifications(where, c) {
  if (c === undefined) return [];
  const list = Array.isArray(c) ? c : [c];
  return list.map((x) => {
    if (!x || typeof x.system !== "string" || typeof x.code !== "string") throw new TypeError(`${where}: classification is { system: "Uniclass 2015", code: "EF_25_10", name? }`);
    return { system: x.system, code: x.code, name: x.name };
  });
}

function __bimElement(name, info) {
  const i = typeof info === "string" ? { class: info } : info;
  if (!i || typeof i !== "object") throw new TypeError(`part ${name}: ifc is a class name or { class, level, psets, … }`);
  const where = `part ${name}`;
  const q = {};
  for (const [k, v] of Object.entries(i.quantities ?? {})) {
    if (typeof v !== "number" || !Number.isFinite(v)) throw new TypeError(`${where}: quantity ${k} is a number`);
    q[k] = v;
  }
  const attributes = {};
  for (const [k, v] of Object.entries(i.attributes ?? {})) attributes[k] = __pvalue(`${where} ${k}`, v);
  __bim.elements[name] = {
    class: __ifcClass(i.class ?? "IfcBuildingElementProxy"),
    predefined: i.predefined,
    type: i.type,
    level: __levelName(i.level),
    tag: i.tag,
    description: i.description,
    host: i.host,
    parent: i.parent,
    psets: __psetsOf(where, i.psets),
    quantities: q,
    attributes,
    classification: __classifications(where, i.classification),
    plan: (i.plan ?? []).map((m) => ({ points: m.points.map((p) => [p[0], p[1]]), dashed: !!m.dashed })),
  };
}

/** What the program declared, for the parts it made (`names`: those kept). */
function __bimDeclared(names) {
  const elements = {};
  for (const [n, e] of Object.entries(__bim.elements)) if (names && names.has(n)) elements[n] = e;
  const out = {
    site: __bim.site ?? undefined,
    building: __bim.building ?? undefined,
    levels: __bim.levels.map((l) => ({ name: l.name, elevation: l.elevation, height: l.height })),
    elements,
    spaces: __bim.spaces,
    openings: __bim.openings.filter((o) => names && names.has(o.host)).map((o) => ({ ...o, fills: o.fills && names.has(o.fills) ? o.fills : undefined })),
    zones: __bim.zones,
    grids: __bim.grids,
    groups: __bim.groups,
  };
  const empty = !out.site && !out.building && !out.levels.length && !Object.keys(elements).length && !out.spaces.length && !out.openings.length && !out.zones.length && !out.grids.length && !out.groups.length;
  return empty ? undefined : out;
}

// ---- Spatial structure ---------------------------------------------------------

/** The site: name, latitude/longitude (degrees), elevation (m above sea level of z = 0), address, north (degrees anticlockwise from +Y). */
function site({ name = "Site", latitude, longitude, elevation, address, north, psets } = {}) {
  __bim.site = { name, latitude, longitude, elevation, address, north, psets: __psetsOf("site", psets) };
  return [];
}

/** The building: name, address, description. */
function building({ name = "Building", address, description, psets } = {}) {
  __bim.building = { name, address, description, psets: __psetsOf("building", psets) };
  return [];
}

/**
 * A storey: its floor's elevation (mm, project z) and height (floor to floor).
 * Returns { name, elevation, height, top }; elements take it as `level`.
 *   const L0 = level("Ground floor", 0, { height: 3000 });
 */
function level(name, elevation, { height } = {}) {
  if (typeof name !== "string" || !name.trim()) throw new TypeError("level() needs a name");
  if (typeof elevation !== "number" || !Number.isFinite(elevation)) throw new TypeError(`level ${name}: elevation is a number of mm`);
  if (__bim.levels.some((l) => l.name === name.trim())) throw new Error(`two levels are called "${name}"`);
  const l = Object.freeze({ name: name.trim(), elevation, height, top: height === undefined ? undefined : elevation + height });
  __bim.levels.push(l);
  return l;
}

// ---- Placing in a wall's frame ----------------------------------------------------

const __deg = (r) => (r * 180) / Math.PI;
const __v2 = (p, where) => {
  if (!Array.isArray(p) || p.length < 2 || !Number.isFinite(p[0]) || !Number.isFinite(p[1])) throw new TypeError(`${where}: a point is [x, y]`);
  return [p[0], p[1]];
};

/** A solid built along +X from the origin, turned and moved to run from a to b (3D). */
function __along(s, a, b) {
  const d = [b[0] - a[0], b[1] - a[1], (b[2] ?? 0) - (a[2] ?? 0)];
  const pitch = Math.atan2(d[2], Math.hypot(d[0], d[1]));
  const yaw = Math.atan2(d[1], d[0]);
  return s.rotate(0, -__deg(pitch), 0).rotate(0, 0, __deg(yaw)).translate(a[0], a[1], a[2] ?? 0);
}

/** A frame along a wall: x from `a` toward `b`, y to the left, z up from `z`. */
function __frame(a, b, z) {
  const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
  const u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
  const n = [-u[1], u[0]];
  return {
    len,
    u,
    n,
    angle: __deg(Math.atan2(u[1], u[0])),
    point: (x, y) => [a[0] + u[0] * x + n[0] * y, a[1] + u[1] * x + n[1] * y],
    place: (s) => s.rotate(0, 0, __deg(Math.atan2(u[1], u[0]))).translate(a[0], a[1], z),
  };
}

const __arc = (c, r, a0, a1, n = 16) => Array.from({ length: n + 1 }, (_, i) => {
  const t = a0 + ((a1 - a0) * i) / n;
  return [c[0] + r * Math.cos(t), c[1] + r * Math.sin(t)];
});

// ---- Doors, windows and openings (in a wall's `openings`) ---------------------------

const DOOR_PRESETS = {
  single: { width: 900, height: 2100, leaves: 1, operation: "SINGLE_SWING" },
  double: { width: 1600, height: 2100, leaves: 2, operation: "DOUBLE_DOOR_SINGLE_SWING" },
  sliding: { width: 1000, height: 2100, leaves: 1, operation: "SLIDING", sliding: true },
  entrance: { width: 1000, height: 2200, leaves: 1, operation: "SINGLE_SWING" },
  garage: { width: 2500, height: 2200, leaves: 1, operation: "ROLLINGUP", rolling: true },
};

const WINDOW_PRESETS = {
  fixed: { width: 1200, height: 1200, sill: 900, panes: 1, partitioning: "SINGLE_PANEL" },
  casement: { width: 600, height: 1200, sill: 900, panes: 1, partitioning: "SINGLE_PANEL" },
  double: { width: 1200, height: 1200, sill: 900, panes: 2, partitioning: "DOUBLE_PANEL_VERTICAL" },
  triple: { width: 1800, height: 1200, sill: 900, panes: 3, partitioning: "TRIPLE_PANEL_VERTICAL" },
  sliding: { width: 1800, height: 2100, sill: 0, panes: 2, partitioning: "DOUBLE_PANEL_VERTICAL", sliding: true },
  high: { width: 1200, height: 500, sill: 1600, panes: 1, partitioning: "SINGLE_PANEL" },
};

/**
 * A door in a wall: `at` is its centre's distance along the wall from its start.
 * door({ at: 1200, preset: "single" | "double" | "sliding" | "entrance" | "garage",
 *        width, height, swing: "left" | "right", side: 1 | -1 (which face it opens to:
 *        1 the wall's left, looking from its start), name, tag, material })
 */
function door(o = {}) {
  const pr = DOOR_PRESETS[o.preset ?? "single"];
  if (!pr) throw new Error(`door: preset is one of ${Object.keys(DOOR_PRESETS).join(", ")}`);
  if (typeof o.at !== "number") throw new TypeError("door: at (the centre's distance along the wall) is a number of mm");
  return { kind: "door", preset: o.preset ?? "single", ...pr, sill: 0, frame: 50, swing: "left", side: 1, material: "Timber", ...o };
}

/** A window in a wall: window({ at, preset: "fixed" | "casement" | "double" | "triple" | "sliding" | "high", width, height, sill, name, tag, material }). */
function window(o = {}) {
  const pr = WINDOW_PRESETS[o.preset ?? "double"];
  if (!pr) throw new Error(`window: preset is one of ${Object.keys(WINDOW_PRESETS).join(", ")}`);
  if (typeof o.at !== "number") throw new TypeError("window: at (the centre's distance along the wall) is a number of mm");
  return { kind: "window", preset: o.preset ?? "double", ...pr, frame: 60, material: "Aluminium", ...o };
}

/** A plain hole through a wall: opening({ at, width, height, sill, name }). */
function opening(o = {}) {
  if (typeof o.at !== "number" || typeof o.width !== "number" || typeof o.height !== "number") throw new TypeError("opening: at, width and height are numbers of mm");
  return { kind: "opening", sill: 0, ...o };
}

function __nextTag(prefix) {
  __bim.tags[prefix] = (__bim.tags[prefix] ?? 0) + 1;
  return `${prefix}${String(__bim.tags[prefix]).padStart(2, "0")}`;
}

/** The part filling an opening (door or window), built in the wall's frame, and its plan marks. */
function __filler(o, f, y0, y1, levelName, external, z0) {
  const w = o.width, h = o.height, x0 = o.at - w / 2, ym = (y0 + y1) / 2;
  const t = y1 - y0;
  const fw = o.frame;
  const fd = Math.min(t, o.kind === "door" ? 120 : 90);
  const tag = o.tag ?? __nextTag(o.kind === "door" ? "D" : "W");
  const name = o.name ?? tag;
  const local = [];
  const plan = [];
  const P = (x, y) => f.point(x, y);
  if (o.kind === "door") {
    // Frame: two jambs and a head; leaves hung in it.
    local.push(box(fw, fd, h).translate(x0, ym - fd / 2, 0));
    local.push(box(fw, fd, h).translate(x0 + w - fw, ym - fd / 2, 0));
    local.push(box(w - 2 * fw, fd, fw).translate(x0 + fw, ym - fd / 2, h - fw));
    const clear = w - 2 * fw;
    const lt = 40;
    if (o.rolling) {
      local.push(box(clear - 4, 30, h - fw - 12).translate(x0 + fw + 2, ym - 15, 10));
      plan.push({ points: [P(x0 + fw, ym), P(x0 + w - fw, ym)], dashed: true });
    } else if (o.sliding) {
      local.push(box(clear * 0.55, lt, h - fw - 12).translate(x0 + fw + 2, ym - lt - 2, 10));
      local.push(box(clear * 0.55, lt, h - fw - 12).translate(x0 + w - fw - 2 - clear * 0.55, ym + 2, 10));
      plan.push({ points: [P(x0 + fw, ym - lt / 2), P(x0 + fw + clear * 0.55, ym - lt / 2)] });
      plan.push({ points: [P(x0 + w - fw - clear * 0.55, ym + lt / 2), P(x0 + w - fw, ym + lt / 2)] });
    } else {
      const leaves = o.leaves;
      const lw = clear / leaves - 3;
      const s = Math.sign(o.side ?? 1) || 1;
      // Hung on the opening's face: the side it opens to.
      const face = s > 0 ? ym + fd / 2 : ym - fd / 2;
      for (let k = 0; k < leaves; k++) {
        const lx = x0 + fw + 1.5 + k * (lw + 3);
        local.push(box(lw, lt, h - fw - 12).translate(lx, s > 0 ? face - lt : face, 10));
        // Swing: hinge at the jamb, leaf drawn open square to the wall, arc back to closed.
        const leftHinge = leaves === 2 ? k === 0 : o.swing !== "right";
        const hx = leftHinge ? x0 + fw : x0 + w - fw;
        const closed = leftHinge ? 0 : Math.PI;
        const open = s > 0 ? Math.PI / 2 : -Math.PI / 2;
        plan.push({ points: [P(hx, face), P(hx, face + s * lw)] });
        plan.push({ points: __arc([hx, face], lw, open, closed).map(([x, y]) => P(x, y)) });
      }
    }
  } else {
    // Window: frame ring, mullions between panes, glass in the middle.
    const ring = rect(w, h).subtract(rect(w - 2 * fw, h - 2 * fw).translate(fw, fw));
    const panes = o.panes;
    const parts = [ring];
    const pw = (w - 2 * fw - (panes - 1) * fw) / panes;
    for (let k = 1; k < panes; k++) parts.push(rect(fw, h - 2 * fw).translate(fw + k * pw + (k - 1) * fw, fw));
    const frame = union(parts.map((p) => p.extrude(fd))).rotate(90, 0, 0).translate(x0, ym + fd / 2, o.sill);
    local.push(frame);
    const glass = box(w - 2 * fw, 24, h - 2 * fw).translate(x0 + fw, ym - 12, o.sill + fw);
    local.push(glass);
    plan.push({ points: [P(x0, ym), P(x0 + w, ym)] });
    plan.push({ points: [P(x0, y0), P(x0 + w, y0)] });
    plan.push({ points: [P(x0, y1), P(x0 + w, y1)] });
  }
  const solid = f.place(union(local));
  const isDoor = o.kind === "door";
  const op = isDoor
    ? o.rolling ? "ROLLINGUP" : o.sliding ? "SLIDING_TO_LEFT" : o.leaves === 2 ? "DOUBLE_DOOR_SINGLE_SWING" : o.swing === "right" ? "SINGLE_SWING_RIGHT" : "SINGLE_SWING_LEFT"
    : undefined;
  const type = o.type ?? `${isDoor ? "Door" : "Window"} ${o.preset} ${w} × ${h}`;
  const material = o.material;
  const p = part(name, solid, {
    material,
    stock: { partNumber: type, description: `${isDoor ? "Door set" : "Window unit"}, ${o.preset}, ${w} × ${h} mm${isDoor ? "" : `, sill ${o.sill}`}` },
    ifc: {
      class: isDoor ? "IfcDoor" : "IfcWindow",
      predefined: isDoor ? (o.preset === "garage" ? "GATE" : "DOOR") : "WINDOW",
      type,
      tag,
      level: levelName,
      psets: __mergePsets({ [isDoor ? "Pset_DoorCommon" : "Pset_WindowCommon"]: { IsExternal: !!external } }, __psetsOf(name, o.psets)),
      quantities: { Width: w, Height: h, Area: (w * h) / 1e6 },
      attributes: isDoor ? { OverallWidth: w, OverallHeight: h, OperationType: op } : { OverallWidth: w, OverallHeight: h, PartitioningType: o.partitioning },
      classification: o.classification,
      plan,
    },
  });
  return { part: p, name };
}

// ---- Walls ------------------------------------------------------------------------

function __wallPart(name, o, a, b, quad, lv) {
  const where = `wall ${name}`;
  const t = o.thickness ?? 200;
  const z0 = lv.elevation + (o.base ?? 0);
  const h = o.height ?? lv.height ?? 2700;
  const f = __frame(a, b, z0);
  if (!(f.len > 0)) throw new Error(`${where}: its start and end are the same point`);
  const align = o.align ?? "center";
  const [y0, y1] = align === "left" ? [0, t] : align === "right" ? [-t, 0] : [-t / 2, t / 2];
  let solid;
  let gross;
  if (o.profile) {
    // Its elevation: [along, up] points, extruded through the thickness.
    const prof = polygon(o.profile.map((p) => __v2(p, where)));
    gross = prof.area;
    solid = f.place(prof.extrude(t).rotate(90, 0, 0).translate(0, y1, 0));
  } else {
    const footprint = quad ? polygon(quad) : null;
    solid = footprint ? footprint.extrude(h).translate(0, 0, z0) : f.place(box(f.len, t, h).translate(0, y0, 0));
    gross = f.len * h;
  }
  const parts = [];
  let cut = 0;
  const cuts = [];
  for (const op of o.openings ?? []) {
    if (!op || !["door", "window", "opening"].includes(op.kind)) throw new TypeError(`${where}: openings are door(…), window(…) or opening(…)`);
    const sill = op.sill ?? 0;
    if (op.at - op.width / 2 < -1 || op.at + op.width / 2 > f.len + 1) throw new Error(`${where}: an opening at ${op.at} (${op.width} wide) runs past the wall's ends (0 to ${Math.round(f.len)})`);
    cuts.push(f.place(box(op.width, t + 20, op.height).translate(op.at - op.width / 2, y0 - 10, sill)));
    cut += op.width * op.height;
    let fills;
    if (op.kind !== "opening") {
      const fl = __filler(op, f, y0, y1, lv.name, o.external, z0);
      parts.push(fl.part);
      fills = fl.name;
    }
    const corner = f.point(op.at - op.width / 2, y0);
    __bim.openings.push({ name: `${fills ?? op.name ?? name} opening`, host: name, origin: [corner[0], corner[1], z0 + sill], dir: f.u, size: [op.width, t, op.height], fills });
  }
  if (cuts.length) solid = solid.subtract(union(cuts));
  const pset = { Pset_WallCommon: { IsExternal: !!o.external, LoadBearing: !!o.loadBearing } };
  parts.unshift(
    part(name, solid, {
      material: o.material ?? "Concrete",
      process: o.process ?? "cast",
      ifc: {
        class: "IfcWall",
        predefined: o.predefined ?? (o.external ? "SOLIDWALL" : "STANDARD"),
        type: o.type,
        tag: o.tag,
        level: lv.name,
        psets: __mergePsets(pset, __psetsOf(where, o.psets)),
        quantities: {
          Length: f.len,
          Width: t,
          Height: h,
          GrossSideArea: gross / 1e6,
          NetSideArea: (gross - cut) / 1e6,
          GrossVolume: (gross * t) / 1e9,
          NetVolume: ((gross - cut) * t) / 1e9,
        },
        classification: o.classification,
      },
    }),
  );
  return parts;
}

/**
 * One wall on a baseline from `from` to `to` ([x, y] mm):
 *   wall("North wall", { from: [0, 0], to: [8000, 0], thickness: 250, height: 2700, level: L0,
 *        align: "center" | "left" | "right", base: 0, external: true, loadBearing: true,
 *        openings: [door({ at: 1200 }), window({ at: 4000, preset: "double" })],
 *        profile: [[0, 0], [8000, 0], [8000, 2700], [4000, 4200], [0, 2700]],   // a gable: its elevation
 *        material: "Concrete", process: "cast", psets, type, tag, classification })
 * Returns the wall and its doors and windows as parts; openings cut it and are declared.
 */
function wall(name, o = {}) {
  const lv = __level(o.level);
  const a = __v2(o.from, `wall ${name}: from`), b = __v2(o.to, `wall ${name}: to`);
  const [e0, e1] = o.extend ?? [0, 0];
  const f = __frame(a, b, 0);
  const a2 = [a[0] - f.u[0] * e0, a[1] - f.u[1] * e0], b2 = [b[0] + f.u[0] * e1, b[1] + f.u[1] * e1];
  // Openings stay where they were along the original baseline.
  const shifted = { ...o, openings: (o.openings ?? []).map((op) => ({ ...op, at: op.at + e0 })) };
  return __wallPart(name, shifted, a2, b2, null, lv);
}

/**
 * Walls along a polyline, mitred where they meet: walls("Outer wall", { points: [[0,0], [9000,0], [9000,7000], [0,7000]],
 *   closed: true, thickness, height, level, align, external, openings: { 0: [door(…)], 2: [window(…)] } })
 * Segment i runs from points[i] to points[i + 1]; its name is "<name> <i + 1>" (or names[i]).
 */
function walls(name, o = {}) {
  const pts = (o.points ?? []).map((p, i) => __v2(p, `walls ${name}: points[${i}]`));
  const closed = !!o.closed;
  const n = pts.length;
  if (n < 2) throw new Error(`walls ${name}: needs at least two points`);
  const t = o.thickness ?? 200;
  const align = o.align ?? "center";
  const [y0, y1] = align === "left" ? [0, t] : align === "right" ? [-t, 0] : [-t / 2, t / 2];
  const segs = [];
  for (let i = 0; i < (closed ? n : n - 1); i++) segs.push([pts[i], pts[(i + 1) % n]]);
  const dirOf = ([a, b]) => {
    const l = Math.hypot(b[0] - a[0], b[1] - a[1]);
    return [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
  };
  const offsetLine = (s, y) => {
    const u = dirOf(s);
    const nn = [-u[1], u[0]];
    return [[s[0][0] + nn[0] * y, s[0][1] + nn[1] * y], u];
  };
  const meet = (s1, s2, y) => {
    // Where the offset lines of two segments cross (parallel: the shared end, offset).
    const [p, u] = offsetLine(s1, y), [q, v] = offsetLine(s2, y);
    const den = u[0] * v[1] - u[1] * v[0];
    if (Math.abs(den) < 1e-9) {
      const nn = [-u[1], u[0]];
      return [s1[1][0] + nn[0] * y, s1[1][1] + nn[1] * y];
    }
    const k = ((q[0] - p[0]) * v[1] - (q[1] - p[1]) * v[0]) / den;
    return [p[0] + u[0] * k, p[1] + u[1] * k];
  };
  const end = (s, at, y) => {
    const u = dirOf(s);
    const nn = [-u[1], u[0]];
    const p = s[at];
    return [p[0] + nn[0] * y, p[1] + nn[1] * y];
  };
  const lv = __level(o.level);
  const out = [];
  segs.forEach((s, i) => {
    const prev = i > 0 ? segs[i - 1] : closed ? segs[segs.length - 1] : null;
    const next = i < segs.length - 1 ? segs[i + 1] : closed ? segs[0] : null;
    const s0 = prev ? meet(prev, s, y0) : end(s, 0, y0);
    const s1 = prev ? meet(prev, s, y1) : end(s, 0, y1);
    const e0 = next ? meet(s, next, y0) : end(s, 1, y0);
    const e1 = next ? meet(s, next, y1) : end(s, 1, y1);
    const quad = [s0, e0, e1, s1];
    if (windingArea(quad) < 0) quad.reverse();
    const ops = Array.isArray(o.openings) ? o.openings[i] : o.openings?.[i];
    const segName = o.names?.[i] ?? `${name} ${i + 1}`;
    out.push(...__wallPart(segName, { ...o, openings: ops ?? [] }, s[0], s[1], quad, lv));
  });
  return out;
}

// ---- Slabs and roofs --------------------------------------------------------------

const __outline = (where, o) => {
  if (o instanceof Profile) return { profile: o, points: null };
  if (Array.isArray(o) && o.length >= 3 && Array.isArray(o[0]) && typeof o[0][0] === "number") {
    const pts = o.map((p, i) => __v2(p, `${where}[${i}]`));
    return { profile: polygon(pts), points: pts };
  }
  throw new TypeError(`${where}: an outline is a Profile or a list of [x, y] points`);
};

const __perimeter = (pts) => pts.reduce((s, p, i) => s + Math.hypot(pts[(i + 1) % pts.length][0] - p[0], pts[(i + 1) % pts.length][1] - p[1]), 0);

/**
 * A floor or roof slab: its top at the level (plus `base`), `thickness` down from there.
 *   slab("First floor", { outline: [[0,0], [9000,0], [9000,7000], [0,7000]] | profile, thickness: 200,
 *        level: L1, base: 0, holes: [rect(…)/points], kind: "FLOOR" | "ROOF" | "BASESLAB" | "LANDING", material })
 */
function slab(name, o = {}) {
  const where = `slab ${name}`;
  const lv = __level(o.level);
  const t = o.thickness ?? 200;
  const { profile, points } = __outline(`${where}: outline`, o.outline);
  let plan = profile;
  for (const h of o.holes ?? []) plan = plan.subtract(__outline(`${where}: holes`, h).profile);
  const top = lv.elevation + (o.base ?? 0);
  const solid = plan.extrude(t).translate(0, 0, top - t);
  const q = { Width: t, GrossArea: profile.area / 1e6, NetArea: plan.area / 1e6, GrossVolume: (profile.area * t) / 1e9, NetVolume: (plan.area * t) / 1e9 };
  if (points) q.Perimeter = __perimeter(points);
  return [
    part(name, solid, {
      material: o.material ?? "Concrete",
      process: o.process ?? "cast",
      ifc: { class: "IfcSlab", predefined: o.kind ?? "FLOOR", type: o.type, tag: o.tag, level: lv.name, psets: __mergePsets({ Pset_SlabCommon: { IsExternal: !!o.external, LoadBearing: o.loadBearing ?? true } }, __psetsOf(where, o.psets)), quantities: q, classification: o.classification },
    }),
  ];
}

/**
 * A pitched or flat roof over a rectangle: its eaves at the level (plus `base`).
 *   roof("Roof", { x: 0, y: 0, width: 9000, depth: 7000, kind: "gable" | "hip" | "shed" | "flat",
 *        slope: 35 (degrees), thickness: 250, overhang: 400, ridge: "x" | "y", level, material })
 * The footprint is the rectangle grown by the overhang; a gable's ridge runs along `ridge`
 * (default the longer side), a shed rises toward +y (or +x with ridge "y").
 */
function roof(name, o = {}) {
  const where = `roof ${name}`;
  const lv = __level(o.level);
  const kind = o.kind ?? "gable";
  const ov = o.overhang ?? 400;
  const t = o.thickness ?? 250;
  const slope = o.slope ?? 35;
  const x0 = (o.x ?? 0) - ov, y0 = (o.y ?? 0) - ov, w = o.width + 2 * ov, d = o.depth + 2 * ov;
  if (!(o.width > 0 && o.depth > 0)) throw new TypeError(`${where}: width and depth are mm`);
  const z = lv.elevation + (o.base ?? 0);
  const tan = Math.tan(rad(slope));
  const alongX = (o.ridge ?? (w >= d ? "x" : "y")) === "x";
  let pts;
  if (kind === "flat") {
    pts = null;
  } else if (kind === "shed") {
    const rise = (alongX ? d : w) * tan;
    pts = alongX
      ? [[x0, y0, z], [x0 + w, y0, z], [x0, y0 + d, z], [x0 + w, y0 + d, z], [x0, y0 + d, z + rise], [x0 + w, y0 + d, z + rise]]
      : [[x0, y0, z], [x0, y0 + d, z], [x0 + w, y0, z], [x0 + w, y0 + d, z], [x0 + w, y0, z + rise], [x0 + w, y0 + d, z + rise]];
  } else {
    const half = (alongX ? d : w) / 2;
    const rise = half * tan;
    // A hip roof's ridge stops short of each end by half the span.
    const inset = kind === "hip" ? half : 0;
    const eaves = [[x0, y0, z], [x0 + w, y0, z], [x0 + w, y0 + d, z], [x0, y0 + d, z]];
    const ridge = alongX ? [[x0 + inset, y0 + half, z + rise], [x0 + w - inset, y0 + half, z + rise]] : [[x0 + half, y0 + inset, z + rise], [x0 + half, y0 + d - inset, z + rise]];
    pts = [...eaves, ...ridge];
  }
  let solid;
  if (!pts) {
    solid = box(w, d, t).translate(x0, y0, z);
  } else {
    const outer = hullPoints(pts);
    // A skin `t` thick square to the slope: the same shape lowered t / cos(slope).
    solid = outer.subtract(outer.translate(0, 0, -t / Math.cos(rad(slope))));
  }
  const plan = w * d;
  const area = kind === "flat" ? plan : plan / Math.cos(rad(slope));
  const predefined = { gable: "GABLE_ROOF", hip: "HIP_ROOF", shed: "SHED_ROOF", flat: "FLAT_ROOF" }[kind];
  if (!predefined) throw new Error(`${where}: kind is gable, hip, shed or flat`);
  return [
    part(name, solid, {
      material: o.material ?? "Timber",
      process: o.process ?? "cnc_router",
      ifc: { class: "IfcRoof", predefined, type: o.type, level: lv.name, psets: __mergePsets({ Pset_RoofCommon: { IsExternal: true } }, __psetsOf(where, o.psets)), quantities: { GrossArea: area / 1e6, NetArea: area / 1e6, ProjectedArea: plan / 1e6, Slope: kind === "flat" ? 0 : slope, Thickness: t }, classification: o.classification },
    }),
  ];
}

// ---- Structure: sections, columns, beams --------------------------------------------

const __I = {
  IPE: { 80: [80, 46, 3.8, 5.2], 100: [100, 55, 4.1, 5.7], 120: [120, 64, 4.4, 6.3], 140: [140, 73, 4.7, 6.9], 160: [160, 82, 5, 7.4], 180: [180, 91, 5.3, 8], 200: [200, 100, 5.6, 8.5], 220: [220, 110, 5.9, 9.2], 240: [240, 120, 6.2, 9.8], 270: [270, 135, 6.6, 10.2], 300: [300, 150, 7.1, 10.7], 330: [330, 160, 7.5, 11.5], 360: [360, 170, 8, 12.7], 400: [400, 180, 8.6, 13.5], 450: [450, 190, 9.4, 14.6], 500: [500, 200, 10.2, 16], 600: [600, 220, 12, 19] },
  HEA: { 100: [96, 100, 5, 8], 120: [114, 120, 5, 8], 140: [133, 140, 5.5, 8.5], 160: [152, 160, 6, 9], 180: [171, 180, 6, 9.5], 200: [190, 200, 6.5, 10], 220: [210, 220, 7, 11], 240: [230, 240, 7.5, 12], 260: [250, 260, 7.5, 12.5], 280: [270, 280, 8, 13], 300: [290, 300, 8.5, 14], 340: [330, 300, 9.5, 16.5], 400: [390, 300, 11, 19] },
  HEB: { 100: [100, 100, 6, 10], 120: [120, 120, 6.5, 11], 140: [140, 140, 7, 12], 160: [160, 160, 8, 13], 180: [180, 180, 8.5, 14], 200: [200, 200, 9, 15], 220: [220, 220, 9.5, 16], 240: [240, 240, 10, 17], 260: [260, 260, 10, 17.5], 280: [280, 280, 10.5, 18], 300: [300, 300, 11, 19], 400: [400, 300, 13.5, 24] },
};

/**
 * A cross-section by name, centred on the origin (x across, y up): steel "IPE 200",
 * "HEA 160", "HEB 200" (EN 10365, root radii left out), "SHS 100x5", "RHS 120x80x5",
 * "CHS 114.3x5", or a rectangle "200x400" / "C24 47x200" (timber). Returns { profile, depth, width, area, name }.
 */
function section(spec) {
  const s = String(spec).trim().toUpperCase().replace(/×/g, "X");
  let m = s.match(/^(IPE|HEA|HEB)\s*(\d+)$/);
  if (m) {
    const d = __I[m[1]][+m[2]];
    if (!d) throw new Error(`section ${spec}: sizes are ${Object.keys(__I[m[1]]).join(", ")}`);
    const [h, b, tw, tf] = d;
    const pts = [[-b / 2, -h / 2], [b / 2, -h / 2], [b / 2, -h / 2 + tf], [tw / 2, -h / 2 + tf], [tw / 2, h / 2 - tf], [b / 2, h / 2 - tf], [b / 2, h / 2], [-b / 2, h / 2], [-b / 2, h / 2 - tf], [-tw / 2, h / 2 - tf], [-tw / 2, -h / 2 + tf], [-b / 2, -h / 2 + tf]];
    const p = polygon(pts);
    return { profile: p, depth: h, width: b, area: p.area, name: `${m[1]} ${m[2]}` };
  }
  m = s.match(/^(SHS|RHS)\s*([\d.]+)X([\d.]+)(?:X([\d.]+))?$/);
  if (m) {
    const [b, h, t] = m[4] ? [+m[2], +m[3], +m[4]] : [+m[2], +m[2], +m[3]];
    const p = rect(b, h, { center: true }).subtract(rect(b - 2 * t, h - 2 * t, { center: true }));
    return { profile: p, depth: h, width: b, area: p.area, name: `${m[1]} ${m[2]}x${m[3]}${m[4] ? `x${m[4]}` : ""}` };
  }
  m = s.match(/^CHS\s*([\d.]+)X([\d.]+)$/);
  if (m) {
    const r = +m[1] / 2, t = +m[2];
    const p = circle(r).subtract(circle(r - t));
    return { profile: p, depth: 2 * r, width: 2 * r, area: p.area, name: `CHS ${m[1]}x${m[2]}` };
  }
  m = s.match(/^(?:[A-Z]+\d*\s+)?([\d.]+)X([\d.]+)$/);
  if (m) {
    const b = +m[1], h = +m[2];
    return { profile: rect(b, h, { center: true }), depth: h, width: b, area: b * h, name: String(spec).trim() };
  }
  throw new Error(`section: "${spec}" is not a known section (IPE 200, HEA 160, HEB 200, SHS 100x5, RHS 120x80x5, CHS 114.3x5, 200x400)`);
}

/**
 * A column standing on the level: column("C1", { at: [x, y], size: [400, 400] | diameter: 400 | section: "HEB 200",
 *   height, level, base, rotation (degrees about its axis), material })
 */
function column(name, o = {}) {
  const where = `column ${name}`;
  const lv = __level(o.level);
  const at = __v2(o.at, `${where}: at`);
  const h = o.height ?? lv.height ?? 3000;
  let prof, type = o.type;
  if (o.section) {
    const s = section(o.section);
    prof = s.profile;
    type = type ?? s.name;
  } else if (o.diameter) {
    prof = circle(o.diameter / 2);
    type = type ?? `Ø${o.diameter}`;
  } else {
    const [w, d] = o.size ?? [300, 300];
    prof = rect(w, d, { center: true });
    type = type ?? `${w} × ${d}`;
  }
  const solid = prof.rotate(o.rotation ?? 0).extrude(h).translate(at[0], at[1], lv.elevation + (o.base ?? 0));
  return [
    part(name, solid, {
      material: o.material ?? (o.section && !/X/i.test(o.section.replace(/^(SHS|RHS|CHS).*/i, "")) ? "Steel" : "Concrete"),
      process: o.process ?? (o.section ? "extrusion" : "cast"),
      ifc: { class: "IfcColumn", predefined: "COLUMN", type, tag: o.tag, level: lv.name, psets: __mergePsets({ Pset_ColumnCommon: { LoadBearing: true } }, __psetsOf(where, o.psets)), quantities: { Length: h, CrossSectionArea: prof.area / 1e6, GrossVolume: (prof.area * h) / 1e9 }, classification: o.classification },
    }),
  ];
}

/**
 * A beam from `from` to `to` ([x, y, z], z the top of the beam), its section below the line:
 *   beam("B1", { from: [0, 0, 2800], to: [6000, 0, 2800], size: [200, 400] | section: "IPE 300", level, roll, material })
 */
function beam(name, o = {}) {
  const where = `beam ${name}`;
  const lv = __level(o.level);
  const a = vec3(o.from), b = vec3(o.to);
  const len = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
  if (!(len > 0)) throw new Error(`${where}: from and to are the same point`);
  let prof, depth, type = o.type;
  if (o.section) {
    const s = section(o.section);
    prof = s.profile;
    depth = s.depth;
    type = type ?? s.name;
  } else {
    const [w, h] = o.size ?? [200, 400];
    prof = rect(w, h, { center: true });
    depth = h;
    type = type ?? `${w} × ${h}`;
  }
  // Section in (y, z), hung below the line; extruded along +X, then laid from a to b.
  const local = prof.rotate(o.roll ?? 0).translate(0, -depth / 2).extrude(len).rotate(90, 0, 0).rotate(0, 0, 90);
  const solid = __along(local, a, b);
  return [
    part(name, solid, {
      material: o.material ?? (o.section ? "Steel" : "Concrete"),
      process: o.process ?? (o.section ? "extrusion" : "cast"),
      ifc: { class: "IfcBeam", predefined: o.predefined ?? "BEAM", type, tag: o.tag, level: lv.name, psets: __mergePsets({ Pset_BeamCommon: { LoadBearing: true } }, __psetsOf(where, o.psets)), quantities: { Length: len, CrossSectionArea: prof.area / 1e6, GrossVolume: (prof.area * len) / 1e9 }, classification: o.classification },
    }),
  ];
}

// ---- Stairs, ramps, railings ---------------------------------------------------------

/**
 * A straight flight: stair("Stair", { at: [x, y] (foot of the first riser, centre of the width),
 *   dir: degrees (0 runs toward +X), width: 1000, rise (default the level's height), going: 250,
 *   maxRiser: 180, waist: 150, level, base, cut: 1200, material })
 * Risers are equal (rise / n with n the fewest that keep each under maxRiser); the top
 * step is the floor above. Plans show its treads, dashed above the cut, and an arrow up.
 */
function stair(name, o = {}) {
  const where = `stair ${name}`;
  const lv = __level(o.level);
  const at = __v2(o.at, `${where}: at`);
  const rise = o.rise ?? lv.height;
  if (!(rise > 0)) throw new TypeError(`${where}: rise (the floor-to-floor height, mm) is needed`);
  const n = Math.ceil(rise / (o.maxRiser ?? 180) - 1e-9);
  const r = rise / n, g = o.going ?? 250, wdt = o.width ?? 1000, waist = o.waist ?? 150;
  const pts = [[0, 0]];
  for (let i = 0; i < n; i++) {
    pts.push([i * g, (i + 1) * r]);
    if (i < n - 1) pts.push([(i + 1) * g, (i + 1) * r]);
  }
  const run = (n - 1) * g;
  // Soffit parallel to the pitch, `waist` below the nosings.
  const wv = waist * Math.hypot(g, r) / g;
  pts.push([run, rise - wv]);
  const xb = run - ((rise - wv) * g) / r;
  if (xb > 0) pts.push([xb, 0]);
  else pts.push([0, (-xb * r) / g]);
  if (pts.length && pts[pts.length - 1][0] === 0 && pts[pts.length - 1][1] === 0) pts.pop();
  const z0 = lv.elevation + (o.base ?? 0);
  const local = polygon(pts).extrude(wdt).rotate(90, 0, 0).translate(0, wdt / 2, 0);
  const dir = o.dir ?? 0;
  const solid = local.rotate(0, 0, dir).translate(at[0], at[1], z0);
  // Plan: each nosing, dashed above the cut; the arrow runs up.
  const c = Math.cos(rad(dir)), s = Math.sin(rad(dir));
  const P = (x, y) => [at[0] + c * x - s * y, at[1] + s * x + c * y];
  const cutAt = o.cut ?? 1200;
  const plan = [];
  for (let i = 0; i <= n - 1; i++) {
    const above = i * r > cutAt;
    plan.push({ points: [P(i * g, -wdt / 2), P(i * g, wdt / 2)], dashed: above });
  }
  plan.push({ points: [P(0, -wdt / 2), P(run, -wdt / 2)] }, { points: [P(0, wdt / 2), P(run, wdt / 2)] });
  plan.push({ points: [P(g * 0.3, 0), P(run - g * 0.3, 0)] }, { points: [P(run - g * 0.3 - 150, -90), P(run - g * 0.3, 0), P(run - g * 0.3 - 150, 90)] });
  return [
    part(name, solid, {
      material: o.material ?? "Concrete",
      process: o.process ?? "cast",
      ifc: {
        class: "IfcStair",
        predefined: "STRAIGHT_RUN_STAIR",
        type: o.type ?? `Straight flight ${n} × ${Math.round(r * 10) / 10} / ${g}`,
        tag: o.tag,
        level: lv.name,
        psets: __mergePsets({ Pset_StairCommon: { NumberOfRiser: n, NumberOfTreads: n - 1, RiserHeight: Math.round(r * 100) / 100, TreadLength: g } }, __psetsOf(where, o.psets)),
        quantities: { Length: run, Width: wdt, Height: rise, NumberOfRisers: n, RiserHeight: r, TreadLength: g },
        attributes: { NumberOfRisers: n, NumberOfTreads: n - 1, RiserHeight: r, TreadLength: g },
        classification: o.classification,
        plan,
      },
    }),
  ];
}

/** A straight ramp: ramp("Ramp", { at: [x, y] (foot, centre), dir, length, width, rise, thickness?, level, material }). */
function ramp(name, o = {}) {
  const where = `ramp ${name}`;
  const lv = __level(o.level);
  const at = __v2(o.at, `${where}: at`);
  const L = o.length, W = o.width ?? 1200, R = o.rise;
  if (!(L > 0 && R > 0)) throw new TypeError(`${where}: length and rise are mm`);
  const pts = o.thickness ? [[0, 0], [L, R], [L, R - o.thickness], [o.thickness * L / R, 0]] : [[0, 0], [L, 0], [L, R]];
  const solid = polygon(pts).extrude(W).rotate(90, 0, 0).translate(0, W / 2, 0).rotate(0, 0, o.dir ?? 0).translate(at[0], at[1], lv.elevation + (o.base ?? 0));
  return [
    part(name, solid, {
      material: o.material ?? "Concrete",
      process: o.process ?? "cast",
      ifc: { class: "IfcRamp", predefined: "STRAIGHT_RUN_RAMP", type: o.type, level: lv.name, psets: __mergePsets({ Pset_RampCommon: { RequiredSlope: Math.round((R / L) * 1000) / 10 } }, __psetsOf(where, o.psets)), quantities: { Length: L, Width: W, Height: R, Slope: __deg(Math.atan2(R, L)) }, classification: o.classification },
    }),
  ];
}

/**
 * A railing along a path of [x, y] or [x, y, z] points (z above the level plus base):
 *   railing("Balustrade", { path, height: 1000, post: 40, spacing: 1200, rail: 50, bars: 120 (baluster spacing, 0 none), level, material })
 */
function railing(name, o = {}) {
  const where = `railing ${name}`;
  const lv = __level(o.level);
  const z0 = lv.elevation + (o.base ?? 0);
  const path = (o.path ?? []).map((p) => [p[0], p[1], (p[2] ?? 0) + z0]);
  if (path.length < 2) throw new Error(`${where}: path needs two points`);
  const H = o.height ?? 1000, post = o.post ?? 40, rail = o.rail ?? 50, spacing = o.spacing ?? 1200, bars = o.bars ?? 120;
  const pieces = [];
  const postAt = (p) => pieces.push(box(post, post, H).translate(p[0] - post / 2, p[1] - post / 2, p[2]));
  let length = 0;
  for (let i = 0; i < path.length - 1; i++) {
    const a = path[i], b = path[i + 1];
    const L = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    length += L;
    const k = Math.max(1, Math.ceil(L / spacing));
    for (let j = 0; j < k; j++) postAt(a.map((v, m) => v + ((b[m] - v) * j) / k));
    const top = (p) => [p[0], p[1], p[2] + H - rail / 2];
    pieces.push(__along(box(L, rail, rail).translate(0, -rail / 2, -rail / 2), top(a), top(b)));
    if (bars > 0) {
      const nb = Math.floor(L / bars);
      for (let j = 1; j < nb; j++) {
        const p = a.map((v, m) => v + ((b[m] - v) * j) / nb);
        pieces.push(cylinder(H - rail - 100, 8).translate(p[0], p[1], p[2] + 100));
      }
      pieces.push(__along(box(L, 20, 20).translate(0, -10, -10), [a[0], a[1], a[2] + 100], [b[0], b[1], b[2] + 100]));
    }
  }
  postAt(path[path.length - 1]);
  return [
    part(name, union(pieces), {
      material: o.material ?? "Steel",
      process: o.process ?? "weld",
      ifc: { class: "IfcRailing", predefined: o.predefined ?? "GUARDRAIL", type: o.type, level: lv.name, psets: __mergePsets({ Pset_RailingCommon: { Height: H } }, __psetsOf(where, o.psets)), quantities: { Length: length, Height: H }, classification: o.classification },
    }),
  ];
}

/**
 * A curtain wall from `from` to `to`: mullions and transoms on a grid, glass panels between.
 *   curtainWall("South glazing", { from, to, height, level, base, panel: [1500, 1350], mullion: [60, 150], glass: 24 })
 * Two parts (mullions as IfcMember, panels as IfcPlate) in one IfcCurtainWall.
 */
function curtainWall(name, o = {}) {
  const where = `curtain wall ${name}`;
  const lv = __level(o.level);
  const a = __v2(o.from, `${where}: from`), b = __v2(o.to, `${where}: to`);
  const z0 = lv.elevation + (o.base ?? 0);
  const f = __frame(a, b, z0);
  const H = o.height ?? lv.height ?? 3000;
  const [pw, ph] = o.panel ?? [1500, H];
  const [mw, md] = o.mullion ?? [60, 150];
  const gt = o.glass ?? 24;
  const nx = Math.max(1, Math.round(f.len / pw)), nz = Math.max(1, Math.round(H / ph));
  const dx = f.len / nx, dz = H / nz;
  const frame = [];
  for (let i = 0; i <= nx; i++) frame.push(box(mw, md, H).translate(Math.min(Math.max(i * dx - mw / 2, 0), f.len - mw), -md / 2, 0));
  for (let j = 0; j <= nz; j++) frame.push(box(f.len, md, mw).translate(0, -md / 2, Math.min(Math.max(j * dz - mw / 2, 0), H - mw)));
  const panes = [];
  for (let i = 0; i < nx; i++) for (let j = 0; j < nz; j++) panes.push(box(dx - mw, gt, dz - mw).translate(i * dx + mw / 2, -gt / 2, j * dz + mw / 2));
  __bim.groups.push({ name, class: "IfcCurtainWall", type: o.type, level: lv.name, psets: __mergePsets({ Pset_CurtainWallCommon: { IsExternal: true } }, __psetsOf(where, o.psets)), quantities: { Length: f.len, Height: H, GrossSideArea: (f.len * H) / 1e6 } });
  return [
    part(`${name} mullions`, f.place(union(frame)), { material: o.material ?? "Aluminium", process: "extrusion", ifc: { class: "IfcMember", predefined: "MULLION", level: lv.name, parent: name } }),
    part(`${name} panels`, f.place(union(panes)), { material: o.glassMaterial ?? "Glass", process: "glazing", ifc: { class: "IfcPlate", predefined: "CURTAIN_PANEL", level: lv.name, parent: name, quantities: { Area: (nx * nz * (dx - mw) * (dz - mw)) / 1e6 } } }),
  ];
}

/**
 * Reinforcing bars: one bar shape (a polyline of [x, y, z] points along its centre line),
 * repeated `count` times `spacing` apart along `direction`:
 *   rebar("B1 bottom", { path: [[0, 0, 40], [6000, 0, 40]], diameter: 12, count: 20, spacing: 200,
 *         direction: [0, 1, 0], mark: "01", grade: "B500B", level })
 * Quantities give a bending schedule: mark, diameter, length each, count, total length, mass.
 */
function rebar(name, o = {}) {
  const where = `rebar ${name}`;
  const lv = __level(o.level);
  const path = (o.path ?? []).map((p) => vec3(p));
  if (path.length < 2) throw new Error(`${where}: path needs two points`);
  const d = o.diameter ?? 12, count = o.count ?? 1, spacing = o.spacing ?? 200;
  const dirv = __unit(vec3(o.direction ?? [0, 1, 0]));
  const r = d / 2;
  const one = [];
  let L = 0;
  for (let i = 0; i < path.length - 1; i++) {
    const a = path[i], b = path[i + 1];
    const l = Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    L += l;
    one.push(__along(cylinder(l, r, { segments: 8 }).rotate(0, 90, 0), a, b));
    if (i > 0) one.push(sphere(r, { segments: 8 }).translate(...a));
  }
  const bar = union(one);
  const bars = [];
  for (let k = 0; k < count; k++) bars.push(bar.translate(dirv[0] * spacing * k, dirv[1] * spacing * k, dirv[2] * spacing * k));
  const kgPerM = (7850 * Math.PI * (d / 1000) ** 2) / 4;
  return [
    part(name, union(bars), {
      material: o.material ?? "Steel",
      process: o.process,
      ifc: {
        class: "IfcReinforcingBar",
        predefined: o.predefined ?? "MAIN",
        type: `Ø${d} ${o.grade ?? "B500B"}`,
        tag: o.mark,
        level: lv.name,
        attributes: { NominalDiameter: d, BarLength: L, SteelGrade: o.grade ?? "B500B" },
        quantities: { NominalDiameter: d, BarLength: L, Count: count, TotalLength: L * count, Weight: (kgPerM * L * count) / 1000 },
        psets: __psetsOf(where, o.psets),
        classification: o.classification,
      },
    }),
  ];
}

// ---- Spaces, zones, grids ------------------------------------------------------------

const __loops = (where, o) => {
  if (Array.isArray(o) && o.length && Array.isArray(o[0]) && typeof o[0][0] === "number") return [o.map((p, i) => __v2(p, `${where}[${i}]`))];
  if (Array.isArray(o) && o.length && Array.isArray(o[0]) && Array.isArray(o[0][0])) return o.map((l, j) => l.map((p, i) => __v2(p, `${where}[${j}][${i}]`)));
  throw new TypeError(`${where}: an outline is a list of [x, y] points (or a list of loops: outer, then holes)`);
};

/**
 * A room or other space (no material: rooms are the air between walls):
 *   space("Living", { outline: [[x, y], …] (or [outer, hole, …]), level, height (default the level's
 *         less `ceiling`), base, longName: "Living room", usage: "INTERNAL", psets, zone })
 * Schedules give its area, perimeter and volume; plans label it; IFC writes an IfcSpace.
 */
function space(name, o = {}) {
  const where = `space ${name}`;
  const lv = __level(o.level);
  const outline = __loops(`${where}: outline`, o.outline);
  if (outline[0].length < 3) throw new Error(`${where}: an outline needs three points`);
  if (windingArea(outline[0]) < 0) outline[0].reverse();
  const height = o.height ?? (lv.height !== undefined ? lv.height - (o.ceiling ?? 300) : 2700);
  if (__bim.spaces.some((s) => s.name === name)) throw new Error(`two spaces are called "${name}"`);
  __bim.spaces.push({ name, long_name: o.longName, level: lv.name, outline, elevation: lv.elevation + (o.base ?? 0), height, predefined: o.usage ?? "INTERNAL", psets: __psetsOf(where, o.psets), classification: __classifications(where, o.classification) });
  if (o.zone) {
    let z = __bim.zones.find((x) => x.name === o.zone);
    if (!z) __bim.zones.push((z = { name: o.zone, spaces: [], psets: {} }));
    z.spaces.push(name);
  }
  return [];
}

/** Spaces grouped (a flat, a fire compartment): zone("Flat 1", ["Living", "Kitchen"], { psets }). */
function zone(name, spaces = [], { psets } = {}) {
  let z = __bim.zones.find((x) => x.name === name);
  if (!z) __bim.zones.push((z = { name, spaces: [], psets: {} }));
  for (const s of spaces) if (!z.spaces.includes(s)) z.spaces.push(s);
  z.psets = __mergePsets(z.psets, __psetsOf(`zone ${name}`, psets));
  return [];
}

/**
 * A structural grid: lines square to X at each of `x` (numbered 1, 2, …) and square to Y at
 * each of `y` (lettered A, B, …), running `extend` mm past the outermost lines.
 *   const G = grid("Grid", { x: [0, 4500, 9000], y: [0, 3500, 7000], level: L0 })
 *   G.at(1, 2) → [4500, 7000]   (indices into x and y)
 */
function grid(name, { x = [], y = [], labelsX, labelsY, extend = 1500, level: lvl } = {}) {
  const xs = x.map(Number), ys = y.map(Number);
  const lx = labelsX ?? xs.map((_, i) => String(i + 1));
  const ly = labelsY ?? ys.map((_, i) => String.fromCharCode(65 + (i % 26)) + (i >= 26 ? Math.floor(i / 26) : ""));
  const [x0, x1] = xs.length ? [Math.min(...xs), Math.max(...xs)] : [0, 0];
  const [y0, y1] = ys.length ? [Math.min(...ys), Math.max(...ys)] : [0, 0];
  __bim.grids.push({
    name,
    level: __levelName(lvl),
    u: xs.map((v, i) => ({ tag: lx[i], from: [v, y0 - extend], to: [v, y1 + extend] })),
    v: ys.map((v, i) => ({ tag: ly[i], from: [x0 - extend, v], to: [x1 + extend, v] })),
  });
  return Object.freeze({ x: xs, y: ys, at: (i, j) => [xs[i], ys[j]] });
}

/** A group of parts that makes one element (a stair of flights, a truss): group("Stair 1", "IfcStair", { level, psets }); parts give it as ifc.parent. */
function group(name, cls, { level: lvl, predefined, type, tag, psets } = {}) {
  __bim.groups.push({ name, class: __ifcClass(cls), predefined, type, tag, level: __levelName(lvl), psets: __psetsOf(`group ${name}`, psets), quantities: {} });
  return [];
}

// ---- IFC in --------------------------------------------------------------------------

/**
 * The elements of an imported IFC file as parts, with their classes, levels, property sets,
 * quantities and materials; its site, building, levels, spaces, zones, grids and openings too.
 * The file is referred to by its content hash (Forma stores it on import).
 */
function importIfc(sha, { material } = {}) {
  const d = JSON.parse(__n.ifcData(sha));
  if (d.site) site(d.site);
  if (d.building) building(d.building);
  for (const l of d.levels) if (!__bim.levels.some((x) => x.name === l.name)) level(l.name, l.elevation, { height: l.height ?? undefined });
  for (const s of d.spaces) __bim.spaces.push({ ...s, psets: s.psets ?? {}, classification: s.classification ?? [] });
  for (const z of d.zones) __bim.zones.push({ ...z, psets: z.psets ?? {} });
  for (const g of d.grids) __bim.grids.push(g);
  for (const g of d.groups) __bim.groups.push({ ...g, psets: g.psets ?? {}, quantities: g.quantities ?? {} });
  for (const o of d.openings ?? []) __bim.openings.push(o);
  const mesh = (m) => fromMesh(m.positions.flat(), m.triangles.flat());
  const out = [];
  for (const p of d.parts) {
    const pieces = [...p.solids.map(mesh), ...p.hulls.map((h) => hullPoints(h))];
    if (!pieces.length) continue;
    let s = union(pieces);
    if (p.voids.length) s = s.subtract(union(p.voids.map(mesh)));
    out.push(part(p.name, s, { material: p.material ?? material, ifc: { ...p.info, type: p.info.type, psets: p.info.psets ?? {}, quantities: p.info.quantities ?? {}, attributes: p.info.attributes ?? {}, classification: p.info.classification ?? [], plan: p.info.plan ?? [] } }));
  }
  return out;
}
