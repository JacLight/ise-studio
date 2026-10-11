// Previews for the sketch operations the engine does exactly (fillet,
// chamfer, patterns, mirror): the same geometry worked out here, so the
// drawing shows what a click will make before it is made. The engine's
// result is the truth; these only draw it ahead of time.

import { type Curve, type Geometry, type Segment, type Solved, positions, radiusOf } from "./sketchModel";

type P = [number, number];
const TAU = 2 * Math.PI;

const sub = (a: P, b: P): P => [a[0] - b[0], a[1] - b[1]];
const dot = (a: P, b: P) => a[0] * b[0] + a[1] * b[1];
const cross = (a: P, b: P) => a[0] * b[1] - a[1] * b[0];
const dist = (a: P, b: P) => Math.hypot(a[0] - b[0], a[1] - b[1]);
const unit = (v: P): P => {
  const l = Math.hypot(v[0], v[1]) || 1e-300;
  return [v[0] / l, v[1] / l];
};
const wrap = (a: number) => ((a % TAU) + TAU) % TAU;
const angle = (c: P, p: P) => Math.atan2(p[1] - c[1], p[0] - c[0]);

type Prim = { kind: "seg"; a: P; b: P } | { kind: "circ"; c: P; r: number };

/** One of the two curves at a corner: the end that gives way, the end that stays. */
type Side = { curve: Curve; prim: Prim; cut: number | null; keep: number | null; pick: P | null };

function ends(c: Curve): [number, number] | null {
  return c.kind === "line" ? [c.a, c.b] : c.kind === "arc" ? [c.start, c.end] : null;
}

function primOf(c: Curve, pos: Map<number, P>, s: Solved | undefined): Prim | null {
  const at = (id: number) => pos.get(id) ?? [0, 0];
  if (c.kind === "line") return { kind: "seg", a: at(c.a), b: at(c.b) };
  if (c.kind === "arc") return { kind: "circ", c: at(c.center), r: dist(at(c.center), at(c.start)) };
  if (c.kind === "circle") return { kind: "circ", c: at(c.center), r: radiusOf(c, s) };
  return null;
}

function lineCircle(p: P, d: P, c: P, r: number): P[] {
  const f = sub(p, c);
  const [qa, qb, qc] = [dot(d, d), 2 * dot(f, d), dot(f, f) - r * r];
  const disc = qb * qb - 4 * qa * qc;
  if (disc < -1e-12 || qa < 1e-300) return [];
  const s = Math.sqrt(Math.max(disc, 0));
  return [(-qb - s) / (2 * qa), (-qb + s) / (2 * qa)].map((t) => [p[0] + d[0] * t, p[1] + d[1] * t] as P);
}

function circleCircle(c1: P, r1: number, c2: P, r2: number): P[] {
  const d = dist(c1, c2);
  if (d < 1e-12 || d > r1 + r2 + 1e-9 || d < Math.abs(r1 - r2) - 1e-9) return [];
  const a = (r1 * r1 - r2 * r2 + d * d) / (2 * d);
  const h = Math.sqrt(Math.max(r1 * r1 - a * a, 0));
  const m: P = [c1[0] + ((c2[0] - c1[0]) * a) / d, c1[1] + ((c2[1] - c1[1]) * a) / d];
  const n: P = [-(c2[1] - c1[1]) / d, (c2[0] - c1[0]) / d];
  return h < 1e-9 ? [m] : [[m[0] + n[0] * h, m[1] + n[1] * h], [m[0] - n[0] * h, m[1] - n[1] * h]];
}

function lineLine(p: P, d: P, q: P, e: P): P[] {
  const den = cross(d, e);
  if (Math.abs(den) < 1e-12) return [];
  const t = cross(sub(q, p), e) / den;
  return [[p[0] + d[0] * t, p[1] + d[1] * t]];
}

/** Where two curves' whole lines or circles meet. */
function meets(a: Prim, b: Prim): P[] {
  if (a.kind === "seg" && b.kind === "seg") return lineLine(a.a, sub(a.b, a.a), b.a, sub(b.b, b.a));
  if (a.kind === "seg" && b.kind === "circ") return lineCircle(a.a, sub(a.b, a.a), b.c, b.r);
  if (a.kind === "circ" && b.kind === "seg") return lineCircle(b.a, sub(b.b, b.a), a.c, a.r);
  if (a.kind === "circ" && b.kind === "circ") return circleCircle(a.c, a.r, b.c, b.r);
  return [];
}

/** How far along the kept piece (from corner j) the point t lies, if on it. */
function along(s: Side, pos: Map<number, P>, j: P, t: P): number | null {
  if (s.prim.kind === "seg" && s.keep !== null) {
    const k = pos.get(s.keep)!;
    const d = sub(k, j);
    const l2 = dot(d, d);
    const u = dot(sub(t, j), d) / Math.max(l2, 1e-300);
    return u > 1e-9 && u <= 1 + 1e-9 ? u * Math.sqrt(l2) : null;
  }
  if (s.prim.kind === "circ" && s.keep !== null) {
    const c = s.prim.c;
    const [aj, ak, at] = [angle(c, j), angle(c, pos.get(s.keep)!), angle(c, t)];
    const ccw = s.curve.kind === "arc" && s.curve.end === s.keep;
    let span = ccw ? wrap(ak - aj) : wrap(aj - ak);
    if (span < 1e-12) span = TAU;
    const here = ccw ? wrap(at - aj) : wrap(aj - at);
    return here > 1e-9 && here <= span + 1e-9 ? here * s.prim.r : null;
  }
  if (s.prim.kind === "circ") {
    const c = s.prim.c;
    const d = wrap(angle(c, t) - angle(c, j));
    if (!s.pick) return Math.min(d, TAU - d) * s.prim.r;
    const ccw = wrap(angle(c, s.pick) - angle(c, j)) < Math.PI;
    const [here, ok] = ccw ? [d, d < Math.PI] : [TAU - d, d > Math.PI];
    return ok && here > 1e-9 ? here * s.prim.r : null;
  }
  return null;
}

/** The curves (lines and arcs) that end at a point. */
export function curvesAt(g: Geometry, point: number): Curve[] {
  return g.curves.filter((c) => {
    if (c.kind === "bezier") return c.a === point || c.b === point;
    const e = ends(c);
    return !!e && (e[0] === point || e[1] === point);
  });
}

/** Whether a point is a corner that can be rounded: exactly two lines or arcs end there. */
export function isCorner(g: Geometry, point: number): boolean {
  const at = curvesAt(g, point);
  return at.length === 2 && at.every((c) => c.kind === "line" || c.kind === "arc");
}

export type CornerPick = { point: number } | { curves: [number, number]; picks: [P, P] };

function corner(g: Geometry, s: Solved | undefined, pick: CornerPick): { j: P; sides: [Side, Side] } | null {
  const pos = positions(g, s);
  const side = (c: Curve, cut: number | null, keep: number | null, p: P | null): Side | null => {
    const prim = primOf(c, pos, s);
    return prim ? { curve: c, prim, cut, keep, pick: p } : null;
  };
  if ("point" in pick) {
    const at = curvesAt(g, pick.point);
    if (at.length !== 2 || at.some((c) => c.kind !== "line" && c.kind !== "arc")) return null;
    const mk = (c: Curve) => {
      const [a, b] = ends(c)!;
      return a === pick.point ? side(c, a, b, null) : side(c, b, a, null);
    };
    const [s1, s2] = [mk(at[0]), mk(at[1])];
    return s1 && s2 ? { j: pos.get(pick.point)!, sides: [s1, s2] } : null;
  }
  const [ca, cb] = pick.curves.map((id) => g.curves.find((c) => c.id === id));
  if (!ca || !cb || ca.id === cb.id || ca.kind === "bezier" || cb.kind === "bezier") return null;
  // Sharing an end: that is the corner.
  const ea = ends(ca);
  const eb = ends(cb);
  const shared = ea && eb ? ea.find((p) => eb.includes(p)) : undefined;
  if (shared !== undefined) return corner(g, s, { point: shared });
  const [pa, pb] = [primOf(ca, pos, s)!, primOf(cb, pos, s)!];
  const score = (q: P) => dist(q, pick.picks[0]) + dist(q, pick.picks[1]);
  const js = meets(pa, pb).sort((x, y) => score(x) - score(y));
  if (!js.length) return null;
  const j = js[0];
  const mk = (c: Curve, prim: Prim, q: P): Side | null => {
    const e = ends(c);
    if (!e) return side(c, null, null, q);
    const [A, B] = [pos.get(e[0])!, pos.get(e[1])!];
    let keepA: boolean;
    if (prim.kind === "seg") keepA = dot(sub(A, j), sub(q, j)) > dot(sub(B, j), sub(q, j));
    else keepA = wrap(angle(prim.c, j) - angle(prim.c, q)) <= wrap(angle(prim.c, j) - angle(prim.c, A)) + 1e-12;
    return keepA ? side(c, e[1], e[0], null) : side(c, e[0], e[1], null);
  };
  const [s1, s2] = [mk(ca, pa, pick.picks[0]), mk(cb, pb, pick.picks[1])];
  return s1 && s2 ? { j, sides: [s1, s2] } : null;
}

type Off = { kind: "line"; p: P; d: P } | { kind: "circ"; c: P; r: number };

function offsets(s: Side, r: number): Off[] {
  if (s.prim.kind === "seg") {
    const d = unit(sub(s.prim.b, s.prim.a));
    const n: P = [-d[1], d[0]];
    const a = s.prim.a;
    return [1, -1].map((k) => ({ kind: "line", p: [a[0] + k * r * n[0], a[1] + k * r * n[1]], d }) as Off);
  }
  const out: Off[] = [{ kind: "circ", c: s.prim.c, r: s.prim.r + r }];
  if (s.prim.r - r > 1e-9) out.push({ kind: "circ", c: s.prim.c, r: s.prim.r - r });
  return out;
}

function offMeets(a: Off, b: Off): P[] {
  if (a.kind === "line" && b.kind === "line") return lineLine(a.p, a.d, b.p, b.d);
  if (a.kind === "line" && b.kind === "circ") return lineCircle(a.p, a.d, b.c, b.r);
  if (a.kind === "circ" && b.kind === "line") return lineCircle(b.p, b.d, a.c, a.r);
  if (a.kind === "circ" && b.kind === "circ") return circleCircle(a.c, a.r, b.c, b.r);
  return [];
}

function touch(s: Side, o: P): P {
  if (s.prim.kind === "seg") {
    const d = unit(sub(s.prim.b, s.prim.a));
    const t = dot(sub(o, s.prim.a), d);
    return [s.prim.a[0] + d[0] * t, s.prim.a[1] + d[1] * t];
  }
  const d = unit(sub(o, s.prim.c));
  return [s.prim.c[0] + d[0] * s.prim.r, s.prim.c[1] + d[1] * s.prim.r];
}

/** A side from its new end `t` to the end it keeps, for the preview. */
function keptPiece(s: Side, pos: Map<number, P>, t: P): Segment | null {
  if (s.keep === null) return null;
  const k = pos.get(s.keep)!;
  if (s.curve.kind === "line") return { kind: "line", from: t, to: k };
  if (s.curve.kind === "arc" && s.prim.kind === "circ") {
    const ccw = s.curve.end === s.keep;
    return ccw ? { kind: "arc", center: s.prim.c, from: t, to: k, ccw: true } : { kind: "arc", center: s.prim.c, from: k, to: t, ccw: true };
  }
  return null;
}

/** What rounding a corner with radius r would draw: the round and the trimmed sides. Null when it does not fit. */
export function filletPreview(g: Geometry, s: Solved | undefined, pick: CornerPick, r: number): Segment[] | null {
  if (!(r > 0)) return null;
  const k = corner(g, s, pick);
  if (!k) return null;
  const pos = positions(g, s);
  const { j, sides } = k;
  let best: { score: number; o: P; t1: P; t2: P } | null = null;
  for (const a of offsets(sides[0], r))
    for (const b of offsets(sides[1], r))
      for (const o of offMeets(a, b)) {
        const [t1, t2] = [touch(sides[0], o), touch(sides[1], o)];
        if (Math.abs(dist(o, t1) - r) > 1e-6 * Math.max(r, 1) || Math.abs(dist(o, t2) - r) > 1e-6 * Math.max(r, 1) || dist(t1, t2) < 1e-9) continue;
        const [s1, s2] = [along(sides[0], pos, j, t1), along(sides[1], pos, j, t2)];
        if (s1 === null || s2 === null) continue;
        if (!best || s1 + s2 < best.score) best = { score: s1 + s2, o, t1, t2 };
      }
  if (!best) return null;
  const { o, t1, t2 } = best;
  const [a1, a2] = [angle(o, t1), angle(o, t2)];
  const sweep = wrap(a2 - a1);
  const mid = (a: number): P => [o[0] + r * Math.cos(a), o[1] + r * Math.sin(a)];
  const forward = dist(mid(a1 + sweep / 2), j) <= dist(mid(a2 + (TAU - sweep) / 2), j);
  const out: Segment[] = [forward ? { kind: "arc", center: o, from: t1, to: t2, ccw: true } : { kind: "arc", center: o, from: t2, to: t1, ccw: true }];
  for (const [i, t] of [t1, t2].entries()) {
    const piece = keptPiece(sides[i], pos, t);
    if (piece) out.push(piece);
  }
  return out;
}

/** What bevelling a corner by d would draw. */
export function chamferPreview(g: Geometry, s: Solved | undefined, pick: CornerPick, d: number): Segment[] | null {
  if (!(d > 0)) return null;
  const k = corner(g, s, pick);
  if (!k || k.sides.some((x) => x.cut === null)) return null;
  const pos = positions(g, s);
  const { j, sides } = k;
  const cut = (x: Side): P | null => {
    const found: P[] =
      x.prim.kind === "seg"
        ? (() => {
            const u = unit(sub(pos.get(x.keep!)!, j));
            return [[j[0] + u[0] * d, j[1] + u[1] * d] as P];
          })()
        : circleCircle(x.prim.c, x.prim.r, j, d);
    let best: [number, P] | null = null;
    for (const t of found) {
      const a = along(x, pos, j, t);
      if (a !== null && (!best || a < best[0])) best = [a, t];
    }
    return best ? best[1] : null;
  };
  const [t1, t2] = [cut(sides[0]), cut(sides[1])];
  if (!t1 || !t2) return null;
  const out: Segment[] = [{ kind: "line", from: t1, to: t2 }];
  for (const [i, t] of [t1, t2].entries()) {
    const piece = keptPiece(sides[i], pos, t);
    if (piece) out.push(piece);
  }
  return out;
}

/** The curves `ids` drawn with every point put through `f` (arcs reversed when `flip`). */
export function mappedSegments(g: Geometry, s: Solved | undefined, ids: Iterable<number>, f: (p: P) => P, flip = false): Segment[] {
  const pos = positions(g, s);
  const at = (id: number) => f(pos.get(id) ?? [0, 0]);
  const out: Segment[] = [];
  const want = new Set(ids);
  for (const c of g.curves) {
    if (!want.has(c.id)) continue;
    if (c.kind === "line") out.push({ kind: "line", from: at(c.a), to: at(c.b) });
    else if (c.kind === "circle") out.push({ kind: "circle", center: at(c.center), radius: radiusOf(c, s) });
    else if (c.kind === "arc") out.push(flip ? { kind: "arc", center: at(c.center), from: at(c.end), to: at(c.start), ccw: true } : { kind: "arc", center: at(c.center), from: at(c.start), to: at(c.end), ccw: true });
    else out.push({ kind: "bezier", from: at(c.a), to: at(c.b), c1: at(c.c1), c2: at(c.c2) });
  }
  return out;
}

export type PatternSpec = { kind: "linear"; count: number; spacing: number; direction: number } | { kind: "circular"; count: number; angle: number; center: P };

/** Where copy k of a pattern puts a point. */
export function patternStep(spec: PatternSpec, k: number): (p: P) => P {
  if (spec.kind === "linear") {
    const a = (spec.direction * Math.PI) / 180;
    const [dx, dy] = [Math.cos(a) * spec.spacing * k, Math.sin(a) * spec.spacing * k];
    return (p) => [p[0] + dx, p[1] + dy];
  }
  const a = (spec.angle * Math.PI) / 180;
  const [cs, sn] = [Math.cos(a * k), Math.sin(a * k)];
  const c = spec.center;
  return (p) => [c[0] + (p[0] - c[0]) * cs - (p[1] - c[1]) * sn, c[1] + (p[0] - c[0]) * sn + (p[1] - c[1]) * cs];
}

export function patternPreview(g: Geometry, s: Solved | undefined, curves: number[], spec: PatternSpec): Segment[] {
  const out: Segment[] = [];
  for (let k = 1; k < Math.min(spec.count, 201); k++) out.push(...mappedSegments(g, s, curves, patternStep(spec, k)));
  return out;
}

/** The reflection of a point about the line through a and b. */
export function reflectAbout(a: P, b: P) {
  const d = unit(sub(b, a));
  return (p: P): P => {
    const t = dot(sub(p, a), d);
    const foot: P = [a[0] + d[0] * t, a[1] + d[1] * t];
    return [2 * foot[0] - p[0], 2 * foot[1] - p[1]];
  };
}
