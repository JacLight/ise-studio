// Sketch curves as points along them: lines, arcs, circles, Béziers,
// ellipses and their arcs, hyperbolas, parabolas and B-splines (rational,
// any knots, periodic). One evaluator for drawing, hit-testing, snapping
// and previews, so every part of the editor sees the same curve.

import type { Curve, Segment, Solved } from "./sketchModel";
import { minorOf, radiusOf } from "./sketchModel";

type P = [number, number];

const sub = (a: P, b: P): P => [a[0] - b[0], a[1] - b[1]];
const len = (a: P) => Math.hypot(a[0], a[1]);

/** A conic's frame: centre (vertex), unit major direction, sizes. */
export function conicFrame(c: P, m: P): { c: P; u: P; v: P; a: number } {
  const a = Math.max(len(sub(m, c)), 1e-9);
  const u: P = [(m[0] - c[0]) / a, (m[1] - c[1]) / a];
  return { c, u, v: [-u[1], u[0]], a };
}

const at2 = (f: { c: P; u: P; v: P }, x: number, y: number): P => [f.c[0] + f.u[0] * x + f.v[0] * y, f.c[1] + f.u[1] * x + f.v[1] * y];
const local = (f: { c: P; u: P; v: P }, p: P) => {
  const d = sub(p, f.c);
  return [d[0] * f.u[0] + d[1] * f.u[1], d[0] * f.v[0] + d[1] * f.v[1]];
};

/** The full knot vector of a B-spline as stored (default: uniform clamped). */
export function flatKnots(degree: number, n: number, knots?: number[], mults?: number[]): number[] {
  if (knots?.length && mults?.length === knots.length) return knots.flatMap((k, i) => Array(mults[i]).fill(k));
  const spans = n - degree;
  return [...Array(degree + 1).fill(0), ...Array.from({ length: Math.max(0, spans - 1) }, (_, i) => i + 1), ...Array(degree + 1).fill(Math.max(1, spans))];
}

/** A rational B-spline evaluator (de Boor in homogeneous coordinates). */
export function nurbsAt(degree: number, poles: P[], weights: number[], knots: number[], u: number): P {
  const p = degree;
  const n = poles.length - 1;
  let k = p;
  while (k < n && knots[k + 1] <= u) k++;
  const d = Array.from({ length: p + 1 }, (_, j) => {
    const i = j + k - p;
    const w = weights[i] ?? 1;
    return [poles[i][0] * w, poles[i][1] * w, w];
  });
  for (let r = 1; r <= p; r++) {
    for (let j = p; j >= r; j--) {
      const i = j + k - p;
      const den = knots[j + 1 + k - r] - knots[i];
      const a = Math.abs(den) < 1e-300 ? 0 : (u - knots[i]) / den;
      d[j] = [0, 1, 2].map((c) => d[j - 1][c] + (d[j][c] - d[j - 1][c]) * a);
    }
  }
  return [d[p][0] / d[p][2], d[p][1] / d[p][2]];
}

/** A B-spline's domain, poles, weights and knots ready to evaluate (a periodic one wrapped round). */
export function splineForm(c: Extract<Curve, { kind: "bspline" }>, at: (id: number) => P) {
  const poles = c.poles.map(at);
  const w = c.weights?.length === poles.length ? c.weights : poles.map(() => 1);
  const p = c.degree;
  if (c.periodic) {
    const ext = [...poles, ...poles.slice(0, p)];
    const we = [...w, ...w.slice(0, p)];
    const knots = Array.from({ length: ext.length + p + 1 }, (_, i) => i);
    return { degree: p, poles: ext, weights: we, knots, domain: [p, poles.length + p] as P };
  }
  const knots = flatKnots(p, poles.length, c.knots, c.mults);
  return { degree: p, poles, weights: w, knots, domain: [knots[p], knots[poles.length]] as P };
}

/** Points along a NURBS loop segment or spline form. */
export function sampleNurbs(f: { degree: number; poles: P[]; weights: number[]; knots: number[] }, n = 96): P[] {
  const [a, b] = [f.knots[f.degree], f.knots[f.poles.length]];
  return Array.from({ length: n + 1 }, (_, i) => nurbsAt(f.degree, f.poles, f.weights, f.knots, a + ((b - a) * i) / n));
}

/** Points along any curve (ends included), where its points are now. */
export function curveTrace(c: Curve, at: (id: number) => P, s: Solved | undefined, n = 96): P[] {
  switch (c.kind) {
    case "line":
      return [at(c.a), at(c.b)];
    case "circle": {
      const [ce, r] = [at(c.center), radiusOf(c, s)];
      return Array.from({ length: n + 1 }, (_, i) => [ce[0] + r * Math.cos((2 * Math.PI * i) / n), ce[1] + r * Math.sin((2 * Math.PI * i) / n)] as P);
    }
    case "arc": {
      const [ce, st, en] = [at(c.center), at(c.start), at(c.end)];
      const r = len(sub(st, ce));
      const a0 = Math.atan2(st[1] - ce[1], st[0] - ce[0]);
      let a1 = Math.atan2(en[1] - ce[1], en[0] - ce[0]);
      while (a1 <= a0 + 1e-12) a1 += 2 * Math.PI;
      return Array.from({ length: n + 1 }, (_, i) => [ce[0] + r * Math.cos(a0 + ((a1 - a0) * i) / n), ce[1] + r * Math.sin(a0 + ((a1 - a0) * i) / n)] as P);
    }
    case "bezier": {
      const [a, b, h1, h2] = [at(c.a), at(c.b), at(c.c1), at(c.c2)];
      return Array.from({ length: n + 1 }, (_, i) => {
        const t = i / n;
        const u = 1 - t;
        const [w0, w1, w2, w3] = [u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t];
        return [w0 * a[0] + w1 * h1[0] + w2 * h2[0] + w3 * b[0], w0 * a[1] + w1 * h1[1] + w2 * h2[1] + w3 * b[1]] as P;
      });
    }
    case "ellipse":
    case "ellipse_arc": {
      const f = conicFrame(at(c.center), at(c.major));
      const b = Math.max(Math.abs(minorOf(c, s)), 1e-9);
      let [t0, t1] = [0, 2 * Math.PI];
      if (c.kind === "ellipse_arc") {
        const t = (p: P) => {
          const [x, y] = local(f, p);
          return Math.atan2(y / b, x / f.a);
        };
        t0 = t(at(c.start));
        t1 = t(at(c.end));
        while (t1 <= t0 + 1e-12) t1 += 2 * Math.PI;
      }
      return Array.from({ length: n + 1 }, (_, i) => {
        const t = t0 + ((t1 - t0) * i) / n;
        return at2(f, f.a * Math.cos(t), b * Math.sin(t));
      });
    }
    case "hyperbola_arc": {
      const f = conicFrame(at(c.center), at(c.major));
      const b = Math.max(Math.abs(minorOf(c, s)), 1e-9);
      const t = (p: P) => Math.asinh(local(f, p)[1] / b);
      const [t0, t1] = [t(at(c.start)), t(at(c.end))];
      return Array.from({ length: n + 1 }, (_, i) => {
        const tt = t0 + ((t1 - t0) * i) / n;
        return at2(f, f.a * Math.cosh(tt), b * Math.sinh(tt));
      });
    }
    case "parabola_arc": {
      const f = conicFrame(at(c.vertex), at(c.focus));
      const s0 = local(f, at(c.start))[1];
      const s1 = local(f, at(c.end))[1];
      return Array.from({ length: n + 1 }, (_, i) => {
        const q = s0 + ((s1 - s0) * i) / n;
        return at2(f, (q * q) / (4 * f.a), q);
      });
    }
    case "bspline": {
      if (c.poles.length < c.degree + 1) return c.poles.map(at);
      const f = splineForm(c, at);
      const [a, b] = f.domain;
      const m = Math.max(n, 24 * (c.poles.length - c.degree));
      return Array.from({ length: m + 1 }, (_, i) => nurbsAt(f.degree, f.poles, f.weights, f.knots, a + ((b - a) * i) / m));
    }
  }
}

/** A curve as a drawing segment: lines, arcs, circles and Béziers as themselves, the rest as points. */
export function curveSegment(c: Curve, at: (id: number) => P, s: Solved | undefined): Segment {
  switch (c.kind) {
    case "line":
      return { kind: "line", from: at(c.a), to: at(c.b) };
    case "circle":
      return { kind: "circle", center: at(c.center), radius: radiusOf(c, s) };
    case "bezier":
      return { kind: "bezier", from: at(c.a), to: at(c.b), c1: at(c.c1), c2: at(c.c2) };
    case "arc":
      return { kind: "arc", from: at(c.start), to: at(c.end), center: at(c.center), ccw: true };
    default:
      return { kind: "poly", points: curveTrace(c, at, s) };
  }
}

/** Curvature comb of a B-spline: for each sample, the point and the comb tip (scaled). */
export function curvatureComb(c: Extract<Curve, { kind: "bspline" }>, at: (id: number) => P, scale: number, n = 80): [P, P][] {
  if (c.poles.length < c.degree + 1) return [];
  const f = splineForm(c, at);
  const [a, b] = f.domain;
  const h = (b - a) * 1e-4;
  const C = (u: number) => nurbsAt(f.degree, f.poles, f.weights, f.knots, Math.min(b, Math.max(a, u)));
  const out: [P, P][] = [];
  for (let i = 0; i <= n; i++) {
    const u = a + ((b - a) * i) / n;
    const p = C(u);
    const d1 = sub(C(u + h), C(u - h)).map((v) => v / (2 * h)) as P;
    const pp = C(u + h);
    const pm = C(u - h);
    const d2: P = [(pp[0] - 2 * p[0] + pm[0]) / (h * h), (pp[1] - 2 * p[1] + pm[1]) / (h * h)];
    const sp = len(d1);
    if (sp < 1e-12) continue;
    const k = (d1[0] * d2[1] - d1[1] * d2[0]) / (sp * sp * sp);
    const nrm: P = [-d1[1] / sp, d1[0] / sp];
    out.push([p, [p[0] - nrm[0] * k * scale, p[1] - nrm[1] * k * scale]]);
  }
  return out;
}

/** The distinct knots of a B-spline and their multiplicities (as the engine stores them). */
export function knotList(c: Extract<Curve, { kind: "bspline" }>): { knots: number[]; mults: number[] } {
  if (c.knots?.length && c.mults?.length) return { knots: c.knots, mults: c.mults };
  const flat = c.periodic ? Array.from({ length: c.poles.length + 1 }, (_, i) => i) : flatKnots(c.degree, c.poles.length);
  const knots: number[] = [];
  const mults: number[] = [];
  for (const k of flat) {
    if (knots.length && Math.abs(knots[knots.length - 1] - k) < 1e-12) mults[mults.length - 1]++;
    else {
      knots.push(k);
      mults.push(1);
    }
  }
  return { knots, mults };
}

/** Where a B-spline's knots lie on it. */
export function knotPoints(c: Extract<Curve, { kind: "bspline" }>, at: (id: number) => P): P[] {
  if (c.poles.length < c.degree + 1) return [];
  const f = splineForm(c, at);
  const [a, b] = f.domain;
  const flat = [...new Set(f.knots)].filter((k) => k >= a - 1e-12 && k <= b + 1e-12);
  return flat.map((u) => nurbsAt(f.degree, f.poles, f.weights, f.knots, Math.min(b, u)));
}
