// Sketches in the UI: the stored geometry, what the engine solved, and the
// edits a drawing tool makes. Every edit is a new geometry sent as one
// `sketch.update` (one undo step); the engine solves it and sends back where
// the constraints put everything.

import type { EntityView } from "./engine";

export type Plane = { origin: [number, number, number]; x: [number, number, number]; y: [number, number, number] };
export type SPoint = { id: number; x: number; y: number };
export type Curve =
  | { kind: "line"; id: number; a: number; b: number; construction?: boolean }
  | { kind: "arc"; id: number; center: number; start: number; end: number; construction?: boolean }
  | { kind: "circle"; id: number; center: number; radius: number; construction?: boolean }
  /** A cubic Bézier from a to b; c1 and c2 are its handles (points the solver moves too). */
  | { kind: "bezier"; id: number; a: number; b: number; c1: number; c2: number; construction?: boolean }
  /** A whole ellipse: `major` is the end of one semi-axis, `minor` the other semi-axis (mm, solved in radii). */
  | { kind: "ellipse"; id: number; center: number; major: number; minor: number; construction?: boolean }
  /** Part of an ellipse, counter-clockwise from start to end. */
  | { kind: "ellipse_arc"; id: number; center: number; major: number; minor: number; start: number; end: number; construction?: boolean }
  /** Part of one branch of a hyperbola: `major` is its vertex. */
  | { kind: "hyperbola_arc"; id: number; center: number; major: number; minor: number; start: number; end: number; construction?: boolean }
  | { kind: "parabola_arc"; id: number; vertex: number; focus: number; start: number; end: number; construction?: boolean }
  /** A B-spline on its control points; default uniform clamped knots; periodic = closed. */
  | { kind: "bspline"; id: number; poles: number[]; degree: number; weights?: number[]; knots?: number[]; mults?: number[]; periodic?: boolean; construction?: boolean };
export type Constraint =
  | { kind: "coincident"; a: number; b: number }
  | { kind: "horizontal"; line: number }
  | { kind: "vertical"; line: number }
  | { kind: "parallel"; a: number; b: number }
  | { kind: "perpendicular"; a: number; b: number; at?: number }
  | { kind: "equal"; a: number; b: number }
  | { kind: "on_curve"; point: number; curve: number }
  | { kind: "tangent"; a: number; b: number; at?: number }
  | { kind: "midpoint"; point: number; line: number }
  | { kind: "fixed"; point: number; x: number; y: number }
  | { kind: "distance"; a: number; b: number; key: string }
  | { kind: "horizontal_distance"; a: number; b: number; key: string }
  | { kind: "vertical_distance"; a: number; b: number; key: string }
  | { kind: "length"; line: number; key: string }
  | { kind: "radius"; curve: number; key: string }
  | { kind: "diameter"; curve: number; key: string }
  | { kind: "angle"; a: number; b: number; key: string }
  /** Point b is point a moved by line `along` (start to end): a step of a linear pattern. */
  | { kind: "translate"; a: number; b: number; along: number }
  /** Point b is point a turned about line `from`'s start by the angle from `from` to `to`: a step of a circular pattern. */
  | { kind: "rotate"; a: number; b: number; from: number; to: number }
  /** Points a and b mirror each other about a line, or about a point (`line` names a point). */
  | { kind: "symmetric"; a: number; b: number; line: number }
  /** Holds a whole curve where it is. */
  | { kind: "block"; curve: number }
  /** Snell's law at point `at` on `boundary`: ray a in, ray b out; key = n2/n1. */
  | { kind: "snell"; a: number; b: number; boundary: number; at: number; key: string }
  | { kind: "internal_alignment"; curve: number; element: number; role: AlignRole; index?: number }
  /** A dimension that measures and does not drive. */
  | { kind: "reference"; of: Constraint }
  /** A constraint switched off. */
  | { kind: "inactive"; of: Constraint };
export type AlignRole = "major_axis" | "minor_axis" | "focus1" | "focus2" | "knot";
export type Geometry = { points: SPoint[]; curves: Curve[]; constraints: Constraint[] };
export type Dim = { key: string; label: string; value: number; unit: string; min?: number | null; max?: number | null; step?: number | null; expr?: string };
export type SketchData = { name: string; plane: Plane; geometry: Geometry; params: Dim[] };
export type Segment =
  | { kind: "line"; from: [number, number]; to: [number, number] }
  | { kind: "bezier"; from: [number, number]; to: [number, number]; c1: [number, number]; c2: [number, number] }
  | { kind: "arc"; from: [number, number]; to: [number, number]; center: [number, number]; ccw: boolean }
  | { kind: "circle"; center: [number, number]; radius: number }
  /** An exact rational B-spline: full clamped knot vector. */
  | { kind: "nurbs"; degree: number; poles: [number, number][]; weights: number[]; knots: number[] }
  /** Any curve as points (drawing only). */
  | { kind: "poly"; points: [number, number][] };
export type Solved = {
  solved: {
    points: Record<string, [number, number]>;
    radii: Record<string, number>;
    free: number;
    conflicts: number[];
    redundant?: number[];
    partially_redundant?: number[];
    malformed?: number[];
    free_points?: number[];
    free_curves?: number[];
    measured?: Record<string, number>;
    residual: number;
  };
  loops: Segment[][];
  open: number;
};

export const sketchOf = (e: EntityView | undefined) => e?.components["shape.sketch"] as SketchData | undefined;
export const solvedOf = (e: EntityView | undefined) => (e as (EntityView & { sketch?: Solved }) | undefined)?.sketch;

export const PLANES: Record<"XY" | "XZ" | "YZ", { label: string; plane: Plane }> = {
  XY: { label: "Ground (top view)", plane: { origin: [0, 0, 0], x: [1, 0, 0], y: [0, 1, 0] } },
  XZ: { label: "Front", plane: { origin: [0, 0, 0], x: [1, 0, 0], y: [0, 0, 1] } },
  YZ: { label: "Side", plane: { origin: [0, 0, 0], x: [0, 1, 0], y: [0, 0, 1] } },
};

/** Plane coordinates (mm) to the world. */
export function toWorld(p: Plane, u: number, v: number): [number, number, number] {
  return [0, 1, 2].map((k) => p.origin[k] + p.x[k] * u + p.y[k] * v) as [number, number, number];
}

export function normalOf(p: Plane): [number, number, number] {
  const [a, b] = [p.x, p.y];
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

/** Where each point is now: solved if the engine has solved it, else as drawn. */
export function positions(g: Geometry, s: Solved | undefined): Map<number, [number, number]> {
  const m = new Map<number, [number, number]>();
  for (const p of g.points) m.set(p.id, s?.solved.points[p.id] ?? [p.x, p.y]);
  return m;
}

export function radiusOf(c: Extract<Curve, { kind: "circle" }>, s: Solved | undefined) {
  return s?.solved.radii[c.id] ?? c.radius;
}

/** A conic's other semi-axis as solved. */
export function minorOf(c: Extract<Curve, { kind: "ellipse" | "ellipse_arc" | "hyperbola_arc" }>, s: Solved | undefined) {
  return s?.solved.radii[c.id] ?? c.minor;
}

/** A curve's own size (circle radius, conic minor semi-axis), if it has one. */
export function sizeOf(c: Curve, s: Solved | undefined): number | null {
  if (c.kind === "circle") return radiusOf(c, s);
  if (c.kind === "ellipse" || c.kind === "ellipse_arc" || c.kind === "hyperbola_arc") return minorOf(c, s);
  return null;
}

/** The constraint itself, out of a reference or off switch. */
export function innerOf(c: Constraint): Exclude<Constraint, { kind: "reference" | "inactive" }> {
  return c.kind === "reference" || c.kind === "inactive" ? innerOf(c.of) : c;
}

/** A constraint's dimension key (inside a wrapper too). */
export function keyOf(c: Constraint): string | null {
  const i = innerOf(c);
  return "key" in i ? i.key : null;
}

/** The points and curves a constraint refers to. */
export function refsOf(c: Constraint): number[] {
  const i = innerOf(c);
  return Object.entries(i)
    .filter(([k, v]) => !["key", "kind", "x", "y", "role", "index"].includes(k) && typeof v === "number")
    .map(([, v]) => v as number);
}

const nextId = (g: Geometry) => Math.max(0, ...g.points.map((p) => p.id), ...g.curves.map((c) => c.id)) + 1;

/** A point at (x, y), or the existing point there. Returns the new geometry and the point's id. */
export function pointAt(g: Geometry, x: number, y: number, existing: number | null): { g: Geometry; id: number } {
  if (existing !== null) return { g, id: existing };
  const id = nextId(g);
  return { g: { ...g, points: [...g.points, { id, x, y }] }, id };
}

/** Lines drawn within this angle of level or upright are made horizontal or vertical. */
const SNAP_DEGREES = 4;

/** A line from a to b, kept horizontal or vertical when drawn nearly so. */
export function addLine(g: Geometry, a: number, b: number, pos: Map<number, [number, number]>): Geometry {
  const id = nextId(g);
  const [p, q] = [pos.get(a)!, pos.get(b)!];
  const angle = (Math.atan2(q[1] - p[1], q[0] - p[0]) * 180) / Math.PI;
  const near = (t: number) => Math.abs(((angle - t + 540) % 360) - 180) < SNAP_DEGREES;
  const constraints = [...g.constraints];
  if (near(0) || near(180)) constraints.push({ kind: "horizontal", line: id });
  else if (near(90) || near(-90)) constraints.push({ kind: "vertical", line: id });
  return { ...g, curves: [...g.curves, { kind: "line", id, a, b }], constraints };
}

/** A rectangle from two corners: four lines, two horizontal and two vertical. */
export function addRectangle(g: Geometry, [x0, y0]: [number, number], [x1, y1]: [number, number]): Geometry {
  let id = nextId(g);
  const pts = [
    [x0, y0],
    [x1, y0],
    [x1, y1],
    [x0, y1],
  ].map(([x, y]) => ({ id: id++, x, y }));
  const lines = pts.map((p, i) => ({ kind: "line" as const, id: id++, a: p.id, b: pts[(i + 1) % 4].id }));
  return {
    points: [...g.points, ...pts],
    curves: [...g.curves, ...lines],
    constraints: [
      ...g.constraints,
      { kind: "horizontal", line: lines[0].id },
      { kind: "vertical", line: lines[1].id },
      { kind: "horizontal", line: lines[2].id },
      { kind: "vertical", line: lines[3].id },
    ],
  };
}

export function addCircle(g: Geometry, center: number, radius: number): Geometry {
  return { ...g, curves: [...g.curves, { kind: "circle", id: nextId(g), center, radius: Math.max(radius, 0.01) }] };
}

/** An arc counter-clockwise around `center` from `start` to `end`. */
export function addArc(g: Geometry, center: number, start: number, end: number): Geometry {
  return { ...g, curves: [...g.curves, { kind: "arc", id: nextId(g), center, start, end }] };
}

/** A cubic Bézier from a to b with handles at c1 and c2 (new handle points). */
export function addBezier(g: Geometry, a: number, b: number, c1: [number, number], c2: [number, number]): { g: Geometry; id: number } {
  let id = nextId(g);
  const h1 = { id: id++, x: c1[0], y: c1[1] };
  const h2 = { id: id++, x: c2[0], y: c2[1] };
  const curve = { kind: "bezier" as const, id: id++, a, b, c1: h1.id, c2: h2.id };
  return { g: { ...g, points: [...g.points, h1, h2], curves: [...g.curves, curve] }, id: curve.id };
}

/** A curve's points: ends first, then (for a Bézier) its handles. */
export function curvePoints(c: Curve): number[] {
  switch (c.kind) {
    case "line":
      return [c.a, c.b];
    case "arc":
      return [c.center, c.start, c.end];
    case "bezier":
      return [c.a, c.b, c.c1, c.c2];
    case "circle":
      return [c.center];
    case "ellipse":
      return [c.center, c.major];
    case "ellipse_arc":
    case "hyperbola_arc":
      return [c.center, c.major, c.start, c.end];
    case "parabola_arc":
      return [c.vertex, c.focus, c.start, c.end];
    case "bspline":
      return [...c.poles];
  }
}

/** Where a curve starts and ends (null for closed curves). */
export function curveEnds(c: Curve): [number, number] | null {
  switch (c.kind) {
    case "line":
    case "bezier":
      return [c.a, c.b];
    case "arc":
    case "ellipse_arc":
    case "hyperbola_arc":
    case "parabola_arc":
      return [c.start, c.end];
    case "bspline":
      return c.periodic ? null : [c.poles[0], c.poles[c.poles.length - 1]];
    default:
      return null;
  }
}

/** Points that are some Bézier's handles (drawn as handles, not corners). */
export function handleIds(g: Geometry): Set<number> {
  return new Set(g.curves.flatMap((c) => (c.kind === "bezier" ? [c.c1, c.c2] : c.kind === "bspline" ? c.poles.filter((_, i) => c.periodic || (i > 0 && i < c.poles.length - 1)) : [])));
}

/** A key for a new dimension: d1, d2, … */
export function newKey(params: Dim[]) {
  let n = 1;
  while (params.some((p) => p.key === `d${n}`)) n++;
  return `d${n}`;
}

/** Removes a curve or point and everything that depends on it. */
export function remove(g: Geometry, params: Dim[], id: number): { g: Geometry; params: Dim[] } {
  // A Bézier's handle is part of it: removing a handle removes the curve.
  const curves = g.curves.filter((c) => c.id !== id && !curvePoints(c).includes(id));
  const alive = new Set([...curves.map((c) => c.id), ...g.points.map((p) => p.id)]);
  const used = new Set(curves.flatMap(curvePoints));
  // Points no curve uses any more go too (unless that is all the sketch has).
  const points = g.points.filter((p) => p.id !== id && (used.has(p.id) || !curves.length));
  for (const p of g.points) if (!points.includes(p)) alive.delete(p.id);
  alive.delete(id);
  const constraints = g.constraints.filter((c) => refsOf(c).every((r) => alive.has(r)));
  const keys = new Set(constraints.map(keyOf).filter(Boolean));
  return { g: { points, curves, constraints }, params: params.filter((p) => keys.has(p.key)) };
}

/** What a constraint looks like on the drawing (dimensions show their value instead). */
export function glyph(c: Constraint): string | null {
  switch (c.kind) {
    case "inactive":
      return glyph(c.of);
    case "block":
      return "▣";
    case "symmetric":
      return "⇋";
    case "snell":
      return "n";
    case "internal_alignment":
      return "⌖";
    case "coincident":
      return null;
    case "on_curve":
      return "∘";
    case "midpoint":
      return "M";
    case "horizontal":
      return "H";
    case "vertical":
      return "V";
    case "parallel":
      return "∥";
    case "perpendicular":
      return "⊥";
    case "equal":
      return "=";
    case "tangent":
      return "T";
    case "fixed":
      return "⚲";
    default:
      return null;
  }
}

/** A one-line state of the sketch, for its toolbar. */
export function status(e: EntityView | undefined): { text: string; tone: "ok" | "free" | "bad" | "warn" } {
  if (e?.status?.state === "failed") return { text: e.status.reason?.replace(/^sketch\.solve: /, "") ?? "Cannot be solved", tone: "bad" };
  const s = solvedOf(e);
  if (!s) return { text: "Solving…", tone: "free" };
  if (!sketchOf(e)?.geometry.points.length) return { text: "Empty · pick a tool and click to draw", tone: "free" };
  const loops = s.loops.length;
  const shape = loops ? `${loops} closed ${loops === 1 ? "outline" : "outlines"}` : "no closed outline yet";
  const n = (k: number, one: string, many: string) => `${k} ${k === 1 ? one : many}`;
  const bad = [
    s.solved.conflicts.length ? n(s.solved.conflicts.length, "conflicting constraint", "conflicting constraints") : "",
    s.solved.malformed?.length ? n(s.solved.malformed.length, "malformed constraint", "malformed constraints") : "",
  ].filter(Boolean);
  if (bad.length) return { text: `${bad.join(" · ")} · see Constraints`, tone: "bad" };
  const extra = (s.solved.redundant?.length ?? 0) + (s.solved.partially_redundant?.length ?? 0);
  if (extra) return { text: `${n(extra, "redundant constraint", "redundant constraints")} · ${s.solved.free === 0 ? "fully constrained" : n(s.solved.free, "freedom left", "freedoms left")} · ${shape}`, tone: "warn" };
  if (s.solved.free === 0) return { text: `Fully constrained · ${shape}`, tone: "ok" };
  return { text: `${s.solved.free} ${s.solved.free === 1 ? "freedom" : "freedoms"} left · ${shape}`, tone: "free" };
}

/** The same curve on other points (`m` maps old point ids to new), its size as solved. */
export function remapCurve(c: Curve, id: number, m: (id: number) => number, s?: Solved): Curve {
  switch (c.kind) {
    case "line":
      return { ...c, id, a: m(c.a), b: m(c.b) };
    case "arc":
      return { ...c, id, center: m(c.center), start: m(c.start), end: m(c.end) };
    case "bezier":
      return { ...c, id, a: m(c.a), b: m(c.b), c1: m(c.c1), c2: m(c.c2) };
    case "circle":
      return { ...c, id, center: m(c.center), radius: radiusOf(c, s) };
    case "ellipse":
      return { ...c, id, center: m(c.center), major: m(c.major), minor: minorOf(c, s) };
    case "ellipse_arc":
    case "hyperbola_arc":
      return { ...c, id, center: m(c.center), major: m(c.major), start: m(c.start), end: m(c.end), minor: minorOf(c, s) };
    case "parabola_arc":
      return { ...c, id, vertex: m(c.vertex), focus: m(c.focus), start: m(c.start), end: m(c.end) };
    case "bspline":
      return { ...c, id, poles: c.poles.map(m) };
  }
}

/** The same constraint on other points and curves (inside a wrapper too). */
export function remapConstraint(c: Constraint, m: (id: number) => number): Constraint {
  if (c.kind === "reference" || c.kind === "inactive") return { ...c, of: remapConstraint(c.of, m) } as Constraint;
  return Object.fromEntries(Object.entries(c).map(([k, v]) => [k, !["kind", "key", "x", "y", "role", "index"].includes(k) && typeof v === "number" ? m(v) : v])) as Constraint;
}

/** Gives a dimension (inside a wrapper too) another key, in place. */
export function setKey(c: Constraint, key: string) {
  if (c.kind === "reference" || c.kind === "inactive") {
    setKey(c.of, key);
    return;
  }
  if ("key" in c) (c as { key: string }).key = key;
}

/** A short name for a constraint, for lists. */
export function describe(c: Constraint): string {
  if (c.kind === "reference") return `${describe(c.of)} (reference)`;
  if (c.kind === "inactive") return `${describe(c.of)} (off)`;
  const names: Record<string, string> = {
    coincident: "Coincident",
    horizontal: "Horizontal",
    vertical: "Vertical",
    parallel: "Parallel",
    perpendicular: "Perpendicular",
    equal: "Equal",
    on_curve: "Point on curve",
    tangent: "Tangent",
    midpoint: "Midpoint",
    fixed: "Fixed",
    block: "Block",
    distance: "Distance",
    horizontal_distance: "Horizontal distance",
    vertical_distance: "Vertical distance",
    length: "Length",
    radius: "Radius",
    diameter: "Diameter",
    angle: "Angle",
    translate: "Pattern step",
    rotate: "Pattern turn",
    symmetric: "Symmetric",
    snell: "Snell's law",
    internal_alignment: "Internal alignment",
  };
  const base = names[c.kind] ?? c.kind;
  if ((c.kind === "horizontal_distance" || c.kind === "vertical_distance") && c.a === c.b) return c.kind === "horizontal_distance" ? "Lock X" : "Lock Y";
  if (c.kind === "internal_alignment") return `${base}: ${c.role.replace("_", " ")}`;
  return base;
}
