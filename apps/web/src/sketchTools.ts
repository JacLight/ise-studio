// Drawing tools that place shapes by clicks (a polygon, a slot, a conic, a
// B-spline…): how many clicks each takes, what each click means, a dashed
// preview while placing, and the `sketch.add` shape the clicks make. The
// engine builds the geometry and its constraints exactly.

import { flatKnots, nurbsAt } from "./sketchCurves";
import type { Segment } from "./sketchModel";

type P = [number, number];
/** A click: where, and the point it snapped to (reused, so the shape joins it). */
export type Click = { at: P; id: number | null };

export type Tool =
  | "select"
  | "pen"
  | "bspline"
  | "interpolate"
  | "line"
  | "point"
  | "rect"
  | "crect"
  | "rrect"
  | "circle"
  | "circle3"
  | "ellipse"
  | "arc"
  | "arc3"
  | "earc"
  | "hyperbola"
  | "parabola"
  | "polygon"
  | "slot"
  | "aslot"
  | "dimension"
  | "trim"
  | "extend"
  | "split"
  | "fillet";

export type ToolInfo = { tool: Tool; label: string; hint: string };

/** Tools in families: one button each, its key cycles through the family. */
export const FAMILIES: { key: string; tools: ToolInfo[] }[] = [
  { key: "S", tools: [{ tool: "select", label: "Select", hint: "Click to select · Shift adds · drag a box (right to left: touching) · drag to move · arrows nudge · ⌘D copies · Delete removes" }] },
  {
    key: "L",
    tools: [
      { tool: "line", label: "Line", hint: "Click point to point (a polyline) · click the first point to close · double-click or Esc to stop · Shift keeps to 15° steps" },
      { tool: "point", label: "Point", hint: "Click to place a point (on a curve it stays on it)" },
    ],
  },
  {
    key: "R",
    tools: [
      { tool: "rect", label: "Rectangle", hint: "Click two opposite corners" },
      { tool: "crect", label: "Centred rectangle", hint: "Click the centre, then a corner" },
      { tool: "rrect", label: "Rounded rectangle", hint: "Click two opposite corners · the corner radius is in the field" },
    ],
  },
  {
    key: "C",
    tools: [
      { tool: "circle", label: "Circle", hint: "Click the centre, then the radius" },
      { tool: "circle3", label: "3-point circle", hint: "Click three points on the circle" },
      { tool: "ellipse", label: "Ellipse", hint: "Click the centre, the end of one axis, then a point on the ellipse" },
    ],
  },
  {
    key: "A",
    tools: [
      { tool: "arc", label: "Arc", hint: "Click the centre, the start, then the end (counter-clockwise)" },
      { tool: "arc3", label: "3-point arc", hint: "Click the start, the end, then a point the arc runs through" },
      { tool: "earc", label: "Arc of ellipse", hint: "Click the centre, the end of one axis, a point on the ellipse, then the start and end (counter-clockwise)" },
      { tool: "hyperbola", label: "Hyperbola", hint: "Click the centre, the vertex, a point on the branch, then the start and end" },
      { tool: "parabola", label: "Parabola", hint: "Click the vertex, the focus, then the start and end" },
    ],
  },
  {
    key: "P",
    tools: [
      { tool: "pen", label: "Pen", hint: "Click for a corner · press and drag for a smooth curve · click the first point to close · Enter or Esc to finish" },
      { tool: "bspline", label: "B-spline", hint: "Click its control points · click the first to close (periodic) · Enter or double-click to finish · degree in the field" },
      { tool: "interpolate", label: "Curve through points", hint: "Click the points it runs through · click the first to close · Enter or double-click to finish" },
    ],
  },
  { key: "G", tools: [{ tool: "polygon", label: "Polygon", hint: "Click the centre, then a corner · sides in the field" }] },
  {
    key: "O",
    tools: [
      { tool: "slot", label: "Slot", hint: "Click the two centres, then the width" },
      { tool: "aslot", label: "Arc slot", hint: "Click the centre, the start, the end (counter-clockwise), then the width" },
    ],
  },
  { key: "D", tools: [{ tool: "dimension", label: "Dimension", hint: "Click a line, circle or arc; or two points, a point and a curve, or two lines or circles; then type a value or =formula" }] },
  { key: "T", tools: [{ tool: "trim", label: "Trim", hint: "Click the piece of a curve to cut away, back to where other curves cross it" }] },
  { key: "E", tools: [{ tool: "extend", label: "Extend", hint: "Click near the end of a line or arc to carry it on to the next curve" }] },
  { key: "X", tools: [{ tool: "split", label: "Split", hint: "Click a curve where it should be cut in two" }] },
  { key: "F", tools: [{ tool: "fillet", label: "Fillet", hint: "Click a corner to round it · drag out from the corner to size it · or click two curves (the pieces to keep)" }] },
];

export const toolInfo = (t: Tool) => FAMILIES.flatMap((f) => f.tools).find((x) => x.tool === t)!;
export const familyOf = (t: Tool) => FAMILIES.find((f) => f.tools.some((x) => x.tool === t))!;

/** Options the click tools read. */
export type ToolOptions = { sides: number; radius: number; degree: number };

const ref = (c: Click) => (c.id !== null ? c.id : c.at);
const sub = (a: P, b: P): P => [a[0] - b[0], a[1] - b[1]];
const len = (a: P) => Math.hypot(a[0], a[1]);

/** How many clicks a tool takes (null: open-ended, finished by Enter or closing). */
export function clicksFor(t: Tool): number | null {
  return (
    { point: 1, crect: 2, rrect: 2, circle3: 3, ellipse: 3, arc3: 3, earc: 5, hyperbola: 5, parabola: 4, polygon: 2, slot: 3, aslot: 4, bspline: null, interpolate: null } as Partial<Record<Tool, number | null>>
  )[t] as number | null;
}

export const isClickTool = (t: Tool) => clicksFor(t) !== undefined;

/** The circle through three points. */
export function circumcircle(a: P, b: P, c: P): { c: P; r: number } | null {
  const d = 2 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
  if (Math.abs(d) < 1e-12) return null;
  const sq = (p: P) => p[0] * p[0] + p[1] * p[1];
  const ux = (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d;
  const uy = (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d;
  return { c: [ux, uy], r: Math.hypot(a[0] - ux, a[1] - uy) };
}

/** A conic's frame from its centre and axis point, and a point's local coordinates in it. */
function frame(c: P, m: P) {
  const a = Math.max(len(sub(m, c)), 1e-9);
  const u: P = [(m[0] - c[0]) / a, (m[1] - c[1]) / a];
  const v: P = [-u[1], u[0]];
  const loc = (p: P): P => {
    const d = sub(p, c);
    return [d[0] * u[0] + d[1] * u[1], d[0] * v[0] + d[1] * v[1]];
  };
  const at = (x: number, y: number): P => [c[0] + u[0] * x + v[0] * y, c[1] + u[1] * x + v[1] * y];
  return { a, u, v, loc, at };
}

/** An ellipse's other semi-axis from a point on it. */
function ellipseMinor(c: P, m: P, through: P) {
  const f = frame(c, m);
  const [x, y] = f.loc(through);
  const k = 1 - (x / f.a) ** 2;
  return k > 1e-6 ? Math.abs(y) / Math.sqrt(k) : Math.max(Math.abs(y), f.a * 0.5);
}
function hyperbolaMinor(c: P, m: P, through: P) {
  const f = frame(c, m);
  const [x, y] = f.loc(through);
  const k = (x / f.a) ** 2 - 1;
  return k > 1e-6 ? Math.abs(y) / Math.sqrt(k) : f.a;
}

/** The `sketch.add` shape for a finished set of clicks. */
export function buildShape(t: Tool, cl: Click[], o: ToolOptions, closed = false): Record<string, unknown> | null {
  const [p0, p1, p2, p3, p4] = cl;
  switch (t) {
    case "point":
      return { kind: "point", at: ref(p0) };
    case "crect":
      return { kind: "centered_rectangle", center: ref(p0), corner: ref(p1) };
    case "rrect":
      return { kind: "rounded_rectangle", corners: [ref(p0), ref(p1)], radius: o.radius };
    case "circle3":
      return { kind: "circle3", points: [ref(p0), ref(p1), ref(p2)] };
    case "ellipse":
      return { kind: "ellipse", center: ref(p0), major: ref(p1), through: p2.at };
    case "arc3":
      return { kind: "arc3", start: ref(p0), end: ref(p1), through: p2.at };
    case "earc":
      return { kind: "ellipse_arc", center: ref(p0), major: ref(p1), through: p2.at, start: p3.at, end: p4.at };
    case "hyperbola":
      return { kind: "hyperbola_arc", center: ref(p0), major: ref(p1), through: p2.at, start: p3.at, end: p4.at };
    case "parabola":
      return { kind: "parabola_arc", vertex: ref(p0), focus: ref(p1), start: p2.at, end: p3.at };
    case "polygon":
      return { kind: "polygon", center: ref(p0), corner: ref(p1), sides: o.sides };
    case "slot": {
      const d = sub(p1.at, p0.at);
      const l = len(d) || 1;
      const r = Math.abs((d[0] * (p2.at[1] - p0.at[1]) - d[1] * (p2.at[0] - p0.at[0])) / l);
      return r > 1e-6 ? { kind: "slot", a: ref(p0), b: ref(p1), radius: r } : null;
    }
    case "aslot": {
      const r = len(sub(p1.at, p0.at));
      const w = 2 * Math.abs(len(sub(p3.at, p0.at)) - r);
      return w > 1e-6 ? { kind: "arc_slot", center: ref(p0), start: ref(p1), end: p2.at, width: w } : null;
    }
    case "bspline":
      if (cl.length < 2) return null;
      return { kind: "bspline", poles: cl.map(ref), degree: Math.min(o.degree, closed ? cl.length - 1 : cl.length - 1), periodic: closed };
    case "interpolate":
      if (cl.length < (closed ? 3 : 2)) return null;
      return { kind: "interpolate", points: cl.map(ref), periodic: closed };
    default:
      return null;
  }
}

const N = 72;
const poly = (points: P[]): Segment => ({ kind: "poly", points });
const line = (a: P, b: P): Segment => ({ kind: "line", from: a, to: b });

/** What the clicks so far, with the cursor at `cur`, would make (dashed). */
export function previewShape(t: Tool, cl: Click[], cur: P, o: ToolOptions): Segment[] {
  const pts = [...cl.map((c) => c.at), cur];
  const [a, b, c, d, e] = pts;
  const out: Segment[] = [];
  switch (t) {
    case "point":
      return [];
    case "crect":
    case "rrect": {
      if (pts.length < 2) return [];
      const [x0, y0, x1, y1] = t === "crect" ? [2 * a[0] - b[0], 2 * a[1] - b[1], b[0], b[1]] : [a[0], a[1], b[0], b[1]];
      const r = t === "rrect" ? Math.min(o.radius, Math.abs(x1 - x0) / 2, Math.abs(y1 - y0) / 2) : 0;
      const [lx, hx, ly, hy] = [Math.min(x0, x1), Math.max(x0, x1), Math.min(y0, y1), Math.max(y0, y1)];
      out.push(line([lx + r, ly], [hx - r, ly]), line([hx, ly + r], [hx, hy - r]), line([hx - r, hy], [lx + r, hy]), line([lx, hy - r], [lx, ly + r]));
      if (r > 0)
        for (const [cx, cy, a0] of [
          [hx - r, ly + r, -Math.PI / 2],
          [hx - r, hy - r, 0],
          [lx + r, hy - r, Math.PI / 2],
          [lx + r, ly + r, Math.PI],
        ] as const)
          out.push({ kind: "arc", center: [cx, cy], from: [cx + r * Math.cos(a0), cy + r * Math.sin(a0)], to: [cx + r * Math.cos(a0 + Math.PI / 2), cy + r * Math.sin(a0 + Math.PI / 2)], ccw: true });
      return out;
    }
    case "circle3": {
      if (pts.length === 2) return [line(a, b)];
      const k = circumcircle(a, b, c);
      return k ? [{ kind: "circle", center: k.c, radius: k.r }] : [line(a, c)];
    }
    case "arc3": {
      if (pts.length === 2) return [line(a, b)];
      const k = circumcircle(a, b, c);
      if (!k) return [line(a, b)];
      // Counter-clockwise from start to end when the third point lies that way round.
      const ang = (p: P) => Math.atan2(p[1] - k.c[1], p[0] - k.c[0]);
      const w = (x: number) => ((x % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI);
      const ccw = w(ang(c) - ang(a)) < w(ang(b) - ang(a));
      return [ccw ? { kind: "arc", center: k.c, from: a, to: b, ccw: true } : { kind: "arc", center: k.c, from: b, to: a, ccw: true }];
    }
    case "ellipse":
    case "earc": {
      if (pts.length === 2) return [line([2 * a[0] - b[0], 2 * a[1] - b[1]], b)];
      const f = frame(a, b);
      const m = ellipseMinor(a, b, c);
      const tOf = (p: P) => {
        const [x, y] = f.loc(p);
        return Math.atan2(y / m, x / f.a);
      };
      let [t0, t1] = [0, 2 * Math.PI];
      if (t === "earc" && pts.length >= 4) {
        t0 = tOf(d);
        t1 = pts.length >= 5 ? tOf(e) : t0 + 2 * Math.PI;
        while (t1 <= t0) t1 += 2 * Math.PI;
      }
      out.push(poly(Array.from({ length: N + 1 }, (_, i) => f.at(f.a * Math.cos(t0 + ((t1 - t0) * i) / N), m * Math.sin(t0 + ((t1 - t0) * i) / N)))));
      return out;
    }
    case "hyperbola": {
      if (pts.length === 2) return [line(a, b)];
      const f = frame(a, b);
      const m = hyperbolaMinor(a, b, c);
      const tOf = (p: P) => Math.asinh(f.loc(p)[1] / m);
      const [t0, t1] = pts.length >= 4 ? [tOf(d), pts.length >= 5 ? tOf(e) : -tOf(d)] : [-1.5, 1.5];
      return [poly(Array.from({ length: N + 1 }, (_, i) => f.at(f.a * Math.cosh(t0 + ((t1 - t0) * i) / N), m * Math.sinh(t0 + ((t1 - t0) * i) / N))))];
    }
    case "parabola": {
      if (pts.length === 2) return [line(a, b)];
      const f = frame(a, b);
      const s = (p: P) => f.loc(p)[1];
      const [s0, s1] = pts.length >= 4 ? [s(c), s(d)] : [s(c), -s(c)];
      return [poly(Array.from({ length: N + 1 }, (_, i) => {
        const q = s0 + ((s1 - s0) * i) / N;
        return f.at((q * q) / (4 * f.a), q);
      }))];
    }
    case "polygon": {
      const n = Math.max(3, o.sides);
      const r = len(sub(b, a));
      const a0 = Math.atan2(b[1] - a[1], b[0] - a[0]);
      const corners = Array.from({ length: n + 1 }, (_, i) => [a[0] + r * Math.cos(a0 + (2 * Math.PI * i) / n), a[1] + r * Math.sin(a0 + (2 * Math.PI * i) / n)] as P);
      return [poly(corners), { kind: "circle", center: a, radius: Math.max(r, 1e-6) }];
    }
    case "slot": {
      if (pts.length === 2) return [line(a, b)];
      const dd = sub(b, a);
      const l = len(dd) || 1;
      const nrm: P = [-dd[1] / l, dd[0] / l];
      const r = Math.abs(nrm[0] * (c[0] - a[0]) + nrm[1] * (c[1] - a[1]));
      const off = (p: P, k: number): P => [p[0] + nrm[0] * r * k, p[1] + nrm[1] * r * k];
      return [line(off(a, 1), off(b, 1)), line(off(a, -1), off(b, -1)), { kind: "arc", center: b, from: off(b, -1), to: off(b, 1), ccw: true }, { kind: "arc", center: a, from: off(a, 1), to: off(a, -1), ccw: true }];
    }
    case "aslot": {
      if (pts.length === 2) return [line(a, b)];
      const r = len(sub(b, a));
      const a0 = Math.atan2(b[1] - a[1], b[0] - a[0]);
      let a1 = Math.atan2(c[1] - a[1], c[0] - a[0]);
      while (a1 <= a0) a1 += 2 * Math.PI;
      const at = (rr: number, ang: number): P => [a[0] + rr * Math.cos(ang), a[1] + rr * Math.sin(ang)];
      const w = pts.length >= 4 ? Math.abs(len(sub(d, a)) - r) : r * 0.15;
      return [
        { kind: "arc", center: a, from: at(r + w, a0), to: at(r + w, a1), ccw: true },
        { kind: "arc", center: a, from: at(Math.max(r - w, 1e-6), a0), to: at(Math.max(r - w, 1e-6), a1), ccw: true },
      ];
    }
    case "bspline": {
      if (pts.length < 2) return [];
      const deg = Math.min(o.degree, pts.length - 1);
      const knots = flatKnots(deg, pts.length);
      const [k0, k1] = [knots[deg], knots[pts.length]];
      return [poly(pts), poly(Array.from({ length: N + 1 }, (_, i) => nurbsAt(deg, pts, pts.map(() => 1), knots, k0 + ((k1 - k0) * i) / N)))];
    }
    case "interpolate":
      return pts.length >= 2 ? [poly(catmull(pts))] : [];
    default:
      return [];
  }
}

/** A smooth curve through points (preview only: the engine fits the true B-spline). */
function catmull(pts: P[]): P[] {
  const out: P[] = [];
  for (let i = 0; i < pts.length - 1; i++) {
    const [p0, p1, p2, p3] = [pts[Math.max(0, i - 1)], pts[i], pts[i + 1], pts[Math.min(pts.length - 1, i + 2)]];
    for (let k = 0; k < 16; k++) {
      const t = k / 16;
      const t2 = t * t;
      const t3 = t2 * t;
      out.push([0, 1].map((j) => 0.5 * (2 * p1[j] + (-p0[j] + p2[j]) * t + (2 * p0[j] - 5 * p1[j] + 4 * p2[j] - p3[j]) * t2 + (-p0[j] + 3 * p1[j] - 3 * p2[j] + p3[j]) * t3)) as P);
    }
  }
  out.push(pts[pts.length - 1]);
  return out;
}
