// Selection and transforms in a sketch, as in a drawing program: move,
// rotate, mirror, scale, align and distribute what is selected. A transform
// moves points to where it puts them and keeps the constraints true: pins
// move with their points, horizontal and vertical swap or go when a
// rotation needs it, and dimensions on what moved take their new values
// (a linked dimension keeps its formula, and the solve pulls it back).

import { type Constraint, type Curve, type Dim, type Geometry, type Solved, curvePoints, positions, radiusOf } from "./sketchModel";

type P = [number, number];

/** The points a selection moves: its points, and every point of its curves (handles and centres too). */
export function movedPoints(g: Geometry, sel: ReadonlySet<number>): Set<number> {
  const out = new Set<number>();
  for (const p of g.points) if (sel.has(p.id)) out.add(p.id);
  for (const c of g.curves) if (sel.has(c.id)) for (const id of curvePoints(c)) out.add(id);
  return out;
}

/** Everything in a box (plane mm): inside it, or (crossing) touching it. */
export function inBox(g: Geometry, s: Solved | undefined, a: P, b: P, crossing: boolean): Set<number> {
  const [x0, x1, y0, y1] = [Math.min(a[0], b[0]), Math.max(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[1], b[1])];
  const pos = positions(g, s);
  const inside = (id: number) => {
    const p = pos.get(id);
    return !!p && p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1;
  };
  // Does the segment p–q pass through the box? (Liang–Barsky clipping.)
  const crosses = (p: P, q: P) => {
    let [t0, t1] = [0, 1];
    const d = [q[0] - p[0], q[1] - p[1]];
    for (const [pk, qk] of [
      [-d[0], p[0] - x0],
      [d[0], x1 - p[0]],
      [-d[1], p[1] - y0],
      [d[1], y1 - p[1]],
    ]) {
      if (pk === 0) {
        if (qk < 0) return false;
      } else {
        const t = qk / pk;
        if (pk < 0) t0 = Math.max(t0, t);
        else t1 = Math.min(t1, t);
        if (t0 > t1) return false;
      }
    }
    return true;
  };
  const out = new Set<number>();
  for (const c of g.curves) {
    const trace = traceOf(c, pos, s);
    const all = trace.every((p) => p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1);
    const touch = curvePoints(c).some(inside) || trace.some((p, i) => i > 0 && crosses(trace[i - 1], p));
    if (all || (crossing && touch)) out.add(c.id);
  }
  // Points on their own (not on any curve) are picked by themselves.
  const onCurves = new Set(g.curves.flatMap(curvePoints));
  for (const p of g.points) if (!onCurves.has(p.id) && inside(p.id)) out.add(p.id);
  return out;
}

/** Points along a curve (ends of a line; samples of anything curved). */
function traceOf(c: Curve, pos: Map<number, P>, s: Solved | undefined): P[] {
  const at = (id: number): P => pos.get(id) ?? [0, 0];
  const n = 48;
  if (c.kind === "line") return [at(c.a), at(c.b)];
  if (c.kind === "circle") {
    const [ce, r] = [at(c.center), radiusOf(c, s)];
    return Array.from({ length: n + 1 }, (_, i) => [ce[0] + r * Math.cos((2 * Math.PI * i) / n), ce[1] + r * Math.sin((2 * Math.PI * i) / n)] as P);
  }
  if (c.kind === "arc") {
    const [ce, st, en] = [at(c.center), at(c.start), at(c.end)];
    const r = Math.hypot(st[0] - ce[0], st[1] - ce[1]);
    const a0 = Math.atan2(st[1] - ce[1], st[0] - ce[0]);
    let a1 = Math.atan2(en[1] - ce[1], en[0] - ce[0]);
    while (a1 <= a0) a1 += 2 * Math.PI;
    return Array.from({ length: n + 1 }, (_, i) => [ce[0] + r * Math.cos(a0 + ((a1 - a0) * i) / n), ce[1] + r * Math.sin(a0 + ((a1 - a0) * i) / n)] as P);
  }
  const [a, b, h1, h2] = [at(c.a), at(c.b), at(c.c1), at(c.c2)];
  return Array.from({ length: n + 1 }, (_, i) => {
    const t = i / n;
    const u = 1 - t;
    const [w0, w1, w2, w3] = [u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t];
    return [w0 * a[0] + w1 * h1[0] + w2 * h2[0] + w3 * b[0], w0 * a[1] + w1 * h1[1] + w2 * h2[1] + w3 * b[1]] as P;
  });
}

/** The box around curves (a circle's or arc's full round counts) and points. */
export function extent(g: Geometry, s: Solved | undefined, curves: Curve[], points: number[] = []) {
  const pos = positions(g, s);
  const min: P = [Infinity, Infinity];
  const max: P = [-Infinity, -Infinity];
  const take = (p: P | undefined, r = 0) => {
    if (!p) return;
    min[0] = Math.min(min[0], p[0] - r);
    min[1] = Math.min(min[1], p[1] - r);
    max[0] = Math.max(max[0], p[0] + r);
    max[1] = Math.max(max[1], p[1] + r);
  };
  for (const c of curves) {
    if (c.kind === "circle") take(pos.get(c.center), radiusOf(c, s));
    else if (c.kind === "arc" || c.kind === "bezier") for (const p of traceOf(c, pos, s)) take(p);
    else for (const id of curvePoints(c)) take(pos.get(id));
  }
  for (const id of points) take(pos.get(id));
  return { min, max, center: [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2] as P };
}

/** The selection's separate objects: curves joined by shared points, and loose points. */
export function objects(g: Geometry, sel: ReadonlySet<number>): { curves: Curve[]; points: number[] }[] {
  const curves = g.curves.filter((c) => sel.has(c.id));
  const parent = new Map<number, number>();
  const find = (x: number): number => {
    while (parent.has(x) && parent.get(x) !== x) x = parent.get(x)!;
    return x;
  };
  const join = (a: number, b: number) => {
    const [ra, rb] = [find(a), find(b)];
    if (ra !== rb) parent.set(ra, rb);
  };
  for (const c of curves) {
    const ps = curvePoints(c);
    for (const p of ps) if (!parent.has(p)) parent.set(p, p);
    for (const p of ps.slice(1)) join(ps[0], p);
  }
  const groups = new Map<number, { curves: Curve[]; points: number[] }>();
  for (const c of curves) {
    const r = find(curvePoints(c)[0]);
    if (!groups.has(r)) groups.set(r, { curves: [], points: [] });
    groups.get(r)!.curves.push(c);
  }
  const out = [...groups.values()];
  for (const p of g.points) if (sel.has(p.id) && !parent.has(p.id)) out.push({ curves: [], points: [p.id] });
  return out;
}

/**
 * A new geometry with points moved to `to` (others where they are now), and
 * the constraints kept true. `scale` resizes circles in `circles`; `mirror`
 * turns arcs whose points all moved the other way round.
 */
export function rewrite(
  g: Geometry,
  params: Dim[],
  s: Solved | undefined,
  to: Map<number, P>,
  o: { scale?: number; mirror?: boolean; circles?: Set<number> } = {},
): { g: Geometry; params: Dim[]; linked: string[] } {
  const now = positions(g, s);
  const at = (id: number): P => to.get(id) ?? now.get(id) ?? [0, 0];
  const radius = new Map<number, number>();
  const curves = g.curves.map((c): Curve => {
    if (c.kind === "circle") {
      const r = radiusOf(c, s) * (o.circles?.has(c.id) ? (o.scale ?? 1) : 1);
      radius.set(c.id, r);
      return { ...c, radius: r };
    }
    if (c.kind === "arc" && o.mirror && [c.center, c.start, c.end].every((id) => to.has(id))) return { ...c, start: c.end, end: c.start };
    return c;
  });
  const points = g.points.map((p) => {
    const [x, y] = at(p.id);
    return { ...p, x, y };
  });
  const lineOf = (id: number) => curves.find((c) => c.id === id && c.kind === "line") as Extract<Curve, { kind: "line" }> | undefined;
  const level = (id: number, kind: "horizontal" | "vertical") => {
    const l = lineOf(id);
    if (!l) return true;
    const [p, q] = [at(l.a), at(l.b)];
    const len = Math.hypot(q[0] - p[0], q[1] - p[1]) || 1;
    return Math.abs(kind === "horizontal" ? q[1] - p[1] : q[0] - p[0]) < 1e-6 * Math.max(1, len);
  };
  const measure = (c: Constraint): number | null => {
    switch (c.kind) {
      case "length": {
        const l = lineOf(c.line);
        return l ? Math.hypot(at(l.b)[0] - at(l.a)[0], at(l.b)[1] - at(l.a)[1]) : null;
      }
      case "distance":
        return Math.hypot(at(c.b)[0] - at(c.a)[0], at(c.b)[1] - at(c.a)[1]);
      case "horizontal_distance":
        return at(c.b)[0] - at(c.a)[0];
      case "vertical_distance":
        return at(c.b)[1] - at(c.a)[1];
      case "radius":
      case "diameter": {
        const cur = curves.find((x) => x.id === c.curve);
        const r = cur?.kind === "circle" ? radius.get(cur.id)! : cur?.kind === "arc" ? Math.hypot(at(cur.start)[0] - at(cur.center)[0], at(cur.start)[1] - at(cur.center)[1]) : null;
        return r === null ? null : c.kind === "radius" ? r : 2 * r;
      }
      default:
        return null;
    }
  };
  const values = new Map(params.map((p) => [p.key, p]));
  const linked: string[] = [];
  const constraints: Constraint[] = [];
  for (const c of g.constraints) {
    if (c.kind === "fixed") {
      const p = to.get(c.point);
      constraints.push(p ? { ...c, x: p[0], y: p[1] } : c);
    } else if (c.kind === "horizontal" || c.kind === "vertical") {
      const other = c.kind === "horizontal" ? "vertical" : "horizontal";
      if (level(c.line, c.kind)) constraints.push(c);
      else if (level(c.line, other)) constraints.push({ kind: other, line: c.line });
      // Neither: the line is at an angle now, so the constraint goes.
    } else {
      constraints.push(c);
      if ("key" in c) {
        const v = measure(c);
        const d = values.get(c.key);
        if (v === null || !d || Math.abs(v - d.value) < 1e-6) continue;
        if (d.expr) linked.push(d.label);
        else values.set(c.key, { ...d, value: Math.round(v * 1e6) / 1e6 });
      }
    }
  }
  return { g: { points, curves, constraints }, params: params.map((p) => values.get(p.key)!), linked };
}

/** Every moved point put through `f`. */
export function mapPoints(g: Geometry, s: Solved | undefined, ids: Iterable<number>, f: (p: P) => P): Map<number, P> {
  const now = positions(g, s);
  const out = new Map<number, P>();
  for (const id of ids) {
    const p = now.get(id);
    if (p) out.set(id, f(p));
  }
  return out;
}

export const rotateAbout = (c: P, deg: number) => {
  const a = (deg * Math.PI) / 180;
  // Exact quarter turns, so level lines stay exactly level.
  const [cs, sn] = deg % 90 === 0 ? [Math.round(Math.cos(a)), Math.round(Math.sin(a))] : [Math.cos(a), Math.sin(a)];
  return (p: P): P => [c[0] + (p[0] - c[0]) * cs - (p[1] - c[1]) * sn, c[1] + (p[0] - c[0]) * sn + (p[1] - c[1]) * cs];
};
export const mirrorAbout = (c: P, axis: "x" | "y") => (p: P): P => (axis === "x" ? [2 * c[0] - p[0], p[1]] : [p[0], 2 * c[1] - p[1]]);
export const scaleAbout = (c: P, k: number) => (p: P): P => [c[0] + (p[0] - c[0]) * k, c[1] + (p[1] - c[1]) * k];
export const moveBy = (d: P) => (p: P): P => [p[0] + d[0], p[1] + d[1]];

export type Align = "left" | "hcenter" | "right" | "top" | "vcenter" | "bottom";

/** Each object moved so its box lines up with the selection's box. */
export function align(g: Geometry, s: Solved | undefined, sel: ReadonlySet<number>, how: Align): Map<number, P> {
  const objs = objects(g, sel);
  const all = extent(g, s, objs.flatMap((o) => o.curves), objs.flatMap((o) => o.points));
  const out = new Map<number, P>();
  for (const o of objs) {
    const b = extent(g, s, o.curves, o.points);
    const d: P =
      how === "left"
        ? [all.min[0] - b.min[0], 0]
        : how === "right"
          ? [all.max[0] - b.max[0], 0]
          : how === "hcenter"
            ? [all.center[0] - b.center[0], 0]
            : how === "bottom"
              ? [0, all.min[1] - b.min[1]]
              : how === "top"
                ? [0, all.max[1] - b.max[1]]
                : [0, all.center[1] - b.center[1]];
    for (const [id, p] of mapPoints(g, s, movedPoints(g, new Set([...o.curves.map((c) => c.id), ...o.points])), moveBy(d))) out.set(id, p);
  }
  return out;
}

/** Objects spaced evenly between the outermost two (by their centres). */
export function distribute(g: Geometry, s: Solved | undefined, sel: ReadonlySet<number>, axis: "x" | "y"): Map<number, P> {
  const k = axis === "x" ? 0 : 1;
  const objs = objects(g, sel)
    .map((o) => ({ o, b: extent(g, s, o.curves, o.points) }))
    .sort((a, b) => a.b.center[k] - b.b.center[k]);
  const out = new Map<number, P>();
  if (objs.length < 3) return out;
  const [first, last] = [objs[0].b.center[k], objs[objs.length - 1].b.center[k]];
  objs.forEach(({ o, b }, i) => {
    const want = first + ((last - first) * i) / (objs.length - 1);
    const d: P = axis === "x" ? [want - b.center[0], 0] : [0, want - b.center[1]];
    for (const [id, p] of mapPoints(g, s, movedPoints(g, new Set([...o.curves.map((c) => c.id), ...o.points])), moveBy(d))) out.set(id, p);
  });
  return out;
}

/** A copy of the selection beside it (new ids; constraints among copied things come too). */
export function duplicate(g: Geometry, params: Dim[], s: Solved | undefined, sel: ReadonlySet<number>, offset: P): { g: Geometry; params: Dim[]; ids: Set<number> } {
  const pts = movedPoints(g, sel);
  const curves = g.curves.filter((c) => sel.has(c.id));
  let next = Math.max(0, ...g.points.map((p) => p.id), ...g.curves.map((c) => c.id)) + 1;
  const map = new Map<number, number>();
  const now = positions(g, s);
  const newPoints = [...pts].map((id) => {
    map.set(id, next);
    const [x, y] = now.get(id)!;
    return { id: next++, x: x + offset[0], y: y + offset[1] };
  });
  const m = (id: number) => map.get(id)!;
  const newCurves = curves.map((c): Curve => {
    const id = next++;
    map.set(c.id, id);
    if (c.kind === "line") return { ...c, id, a: m(c.a), b: m(c.b) };
    if (c.kind === "arc") return { ...c, id, center: m(c.center), start: m(c.start), end: m(c.end) };
    if (c.kind === "bezier") return { ...c, id, a: m(c.a), b: m(c.b), c1: m(c.c1), c2: m(c.c2) };
    return { ...c, id, center: m(c.center), radius: radiusOf(c, s) };
  });
  // Shape constraints among the copied things come along; dimensions get their own copies.
  const refs = (c: Constraint) => Object.entries(c).filter(([k, v]) => k !== "key" && k !== "kind" && k !== "x" && k !== "y" && typeof v === "number").map(([, v]) => v as number);
  const newParams = [...params];
  const newConstraints: Constraint[] = [];
  for (const c of g.constraints) {
    if (c.kind === "fixed" || !refs(c).every((r) => map.has(r))) continue;
    const copy = Object.fromEntries(Object.entries(c).map(([k, v]) => [k, k !== "kind" && k !== "key" && typeof v === "number" ? m(v) : v])) as Constraint;
    if ("key" in c) {
      const d = params.find((p) => p.key === c.key);
      let n = 1;
      while (newParams.some((p) => p.key === `d${n}`)) n++;
      const key = `d${n}`;
      if (d) newParams.push({ ...d, key, label: `${d.label} (copy)`, expr: undefined });
      (copy as { key: string }).key = key;
    }
    newConstraints.push(copy);
  }
  return {
    g: { points: [...g.points, ...newPoints], curves: [...g.curves, ...newCurves], constraints: [...g.constraints, ...newConstraints] },
    params: newParams,
    ids: new Set(newCurves.length ? newCurves.map((c) => c.id) : newPoints.map((p) => p.id)),
  };
}
