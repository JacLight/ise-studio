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
  | { kind: "bezier"; id: number; a: number; b: number; c1: number; c2: number; construction?: boolean };
export type Constraint =
  | { kind: "coincident"; a: number; b: number }
  | { kind: "horizontal"; line: number }
  | { kind: "vertical"; line: number }
  | { kind: "parallel"; a: number; b: number }
  | { kind: "perpendicular"; a: number; b: number }
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
  | { kind: "angle"; a: number; b: number; key: string };
export type Geometry = { points: SPoint[]; curves: Curve[]; constraints: Constraint[] };
export type Dim = { key: string; label: string; value: number; unit: string; min?: number | null; max?: number | null; step?: number | null; expr?: string };
export type SketchData = { name: string; plane: Plane; geometry: Geometry; params: Dim[] };
export type Segment =
  | { kind: "line"; from: [number, number]; to: [number, number] }
  | { kind: "bezier"; from: [number, number]; to: [number, number]; c1: [number, number]; c2: [number, number] }
  | { kind: "arc"; from: [number, number]; to: [number, number]; center: [number, number]; ccw: boolean }
  | { kind: "circle"; center: [number, number]; radius: number };
export type Solved = {
  solved: { points: Record<string, [number, number]>; radii: Record<string, number>; free: number; conflicts: number[]; residual: number };
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
  return c.kind === "line" ? [c.a, c.b] : c.kind === "arc" ? [c.center, c.start, c.end] : c.kind === "bezier" ? [c.a, c.b, c.c1, c.c2] : [c.center];
}

/** Points that are some Bézier's handles (drawn as handles, not corners). */
export function handleIds(g: Geometry): Set<number> {
  return new Set(g.curves.flatMap((c) => (c.kind === "bezier" ? [c.c1, c.c2] : [])));
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
  const refs = (c: Constraint): number[] => Object.entries(c).filter(([k, v]) => k !== "key" && k !== "kind" && k !== "x" && k !== "y" && typeof v === "number").map(([, v]) => v as number);
  const constraints = g.constraints.filter((c) => refs(c).every((r) => alive.has(r)));
  const keys = new Set(constraints.map((c) => ("key" in c ? c.key : null)).filter(Boolean));
  return { g: { points, curves, constraints }, params: params.filter((p) => keys.has(p.key)) };
}

/** What a constraint looks like on the drawing (dimensions show their value instead). */
export function glyph(c: Constraint): string | null {
  switch (c.kind) {
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
export function status(e: EntityView | undefined): { text: string; tone: "ok" | "free" | "bad" } {
  if (e?.status?.state === "failed") return { text: e.status.reason?.replace(/^sketch\.solve: /, "") ?? "Cannot be solved", tone: "bad" };
  const s = solvedOf(e);
  if (!s) return { text: "Solving…", tone: "free" };
  if (!sketchOf(e)?.geometry.points.length) return { text: "Empty · pick a tool and click to draw", tone: "free" };
  const loops = s.loops.length;
  const shape = loops ? `${loops} closed ${loops === 1 ? "outline" : "outlines"}` : "no closed outline yet";
  if (s.solved.free === 0) return { text: `Fully constrained · ${shape}`, tone: "ok" };
  return { text: `${s.solved.free} ${s.solved.free === 1 ? "freedom" : "freedoms"} left · ${shape}`, tone: "free" };
}
