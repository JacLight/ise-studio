// Snapping on the sketch plane, as in a drafting program: to existing
// points, then midpoints, centres, quadrants and crossings of curves, then
// onto a curve, else the grid (Alt turns snapping off). With Shift, a line
// from the last point keeps to 15° steps. Each snap says what it found, so
// the tool can tie the new point to it (on the curve, at the midpoint).

import { curveEnds, curvePoints, positions } from "./sketchModel";
import { curveTrace } from "./sketchCurves";
import type { SketchPicture } from "./sketchLayer";

type P = [number, number];
type Px = { x: number; y: number };

export type SnapKind = "point" | "end" | "center" | "midpoint" | "quadrant" | "intersection" | "on_curve" | "grid" | "angle";
export type Snap = { kind: SnapKind; label: string; curve?: number; curves?: [number, number] };
export type Snapped = { u: number; v: number; point: number | null; curve: number | null; snap: Snap | null };

const LABEL: Record<SnapKind, string> = {
  point: "Point",
  end: "Endpoint",
  center: "Centre",
  midpoint: "Midpoint",
  quadrant: "Quadrant",
  intersection: "Intersection",
  on_curve: "On curve",
  grid: "Grid",
  angle: "Angle",
};

/** Where two segments cross (inside both), if they do. */
function cross(a: P, b: P, c: P, d: P): P | null {
  const r: P = [b[0] - a[0], b[1] - a[1]];
  const s: P = [d[0] - c[0], d[1] - c[1]];
  const den = r[0] * s[1] - r[1] * s[0];
  if (Math.abs(den) < 1e-14) return null;
  const t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
  const u = ((c[0] - a[0]) * r[1] - (c[1] - a[1]) * r[0]) / den;
  return t >= -1e-9 && t <= 1 + 1e-9 && u >= -1e-9 && u <= 1 + 1e-9 ? [a[0] + r[0] * t, a[1] + r[1] * t] : null;
}

/**
 * The snapped place for a pointer at plane coordinates `raw` (and screen
 * `mouse`): `px` turns plane coordinates into screen pixels.
 */
export function snapSketch(pic: SketchPicture, raw: P, mouse: Px, px: (u: number, v: number) => Px, grid: number, o: { alt: boolean; shift: boolean; anchor?: P | null }): Snapped {
  const g = pic.data.geometry;
  const pos = positions(g, pic.solved);
  const at = (id: number): P => pos.get(id) ?? [0, 0];
  const d = (p: P) => {
    const q = px(p[0], p[1]);
    return Math.hypot(q.x - mouse.x, q.y - mouse.y);
  };
  // Existing points first, within 10 px.
  let point: number | null = null;
  let best = 10;
  for (const [id, p] of pos) {
    const k = d(p);
    if (k < best) [best, point] = [k, id];
  }
  // The nearest curve within 6 px (for picking and for snapping onto it).
  const traces = new Map<number, P[]>();
  const trace = (id: number) => {
    let t = traces.get(id);
    if (!t) {
      const c = g.curves.find((x) => x.id === id)!;
      t = curveTrace(c, at, pic.solved, 96);
      traces.set(id, t);
    }
    return t;
  };
  let curve: number | null = null;
  let onCurve: P | null = null;
  let bestC = 6;
  const near: number[] = [];
  for (const c of g.curves) {
    const pts = trace(c.id);
    let dc = Infinity;
    let foot: P | null = null;
    for (let i = 1; i < pts.length; i++) {
      const [p0, p1] = [px(...pts[i - 1]), px(...pts[i])];
      const dx = p1.x - p0.x;
      const dy = p1.y - p0.y;
      const t = Math.max(0, Math.min(1, ((mouse.x - p0.x) * dx + (mouse.y - p0.y) * dy) / (dx * dx + dy * dy || 1)));
      const k = Math.hypot(p0.x + t * dx - mouse.x, p0.y + t * dy - mouse.y);
      if (k < dc) {
        dc = k;
        foot = [pts[i - 1][0] + (pts[i][0] - pts[i - 1][0]) * t, pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * t];
      }
    }
    if (dc < 20) near.push(c.id);
    if (dc < bestC) [bestC, curve, onCurve] = [dc, c.id, foot];
  }
  if (point !== null) {
    const role = g.curves.some((c) => curveEnds(c)?.includes(point!)) ? "end" : g.curves.some((c) => "center" in c && c.center === point) ? "center" : "point";
    const p = at(point);
    return { u: p[0], v: p[1], point, curve, snap: { kind: role, label: LABEL[role] } };
  }
  if (o.alt) return { u: raw[0], v: raw[1], point: null, curve, snap: null };
  // Special places on curves, within 10 px.
  const cands: { p: P; snap: Snap }[] = [];
  for (const c of g.curves) {
    if (!near.includes(c.id) && !curvePoints(c).some((id) => d(at(id)) < 60)) continue;
    if (c.kind === "line" || c.kind === "arc" || c.kind === "ellipse_arc" || c.kind === "parabola_arc" || c.kind === "hyperbola_arc" || c.kind === "bezier" || c.kind === "bspline") {
      const t = trace(c.id);
      if (curveEnds(c)) {
        const mid: P = c.kind === "line" ? [(at(c.a)[0] + at(c.b)[0]) / 2, (at(c.a)[1] + at(c.b)[1]) / 2] : t[Math.floor(t.length / 2)];
        cands.push({ p: mid, snap: { kind: "midpoint", label: LABEL.midpoint, curve: c.id } });
      }
    }
    if (c.kind === "circle" || c.kind === "ellipse") {
      const t = trace(c.id);
      for (const k of [0, 0.25, 0.5, 0.75]) cands.push({ p: t[Math.round(k * (t.length - 1))], snap: { kind: "quadrant", label: LABEL.quadrant, curve: c.id } });
    }
  }
  for (let i = 0; i < near.length; i++) {
    for (let j = i + 1; j < near.length; j++) {
      const [a, b] = [trace(near[i]), trace(near[j])];
      for (let s = 1; s < a.length; s++) {
        for (let t = 1; t < b.length; t++) {
          const x = cross(a[s - 1], a[s], b[t - 1], b[t]);
          if (x) cands.push({ p: x, snap: { kind: "intersection", label: LABEL.intersection, curves: [near[i], near[j]] } });
        }
      }
    }
  }
  let pick: { p: P; snap: Snap } | null = null;
  let bestS = 10;
  for (const c of cands) {
    const k = d(c.p);
    if (k < bestS) [bestS, pick] = [k, c];
  }
  if (pick) return { u: pick.p[0], v: pick.p[1], point: null, curve, snap: pick.snap };
  if (curve !== null && onCurve) return { u: onCurve[0], v: onCurve[1], point: null, curve, snap: { kind: "on_curve", label: LABEL.on_curve, curve } };
  // Angle steps from the last point (Shift).
  if (o.shift && o.anchor) {
    const [ax, ay] = o.anchor;
    const r = Math.hypot(raw[0] - ax, raw[1] - ay);
    const step = Math.PI / 12;
    const a = Math.round(Math.atan2(raw[1] - ay, raw[0] - ax) / step) * step;
    const len = Math.round(r / grid) * grid || r;
    return { u: ax + len * Math.cos(a), v: ay + len * Math.sin(a), point: null, curve, snap: { kind: "angle", label: `${Math.round((a * 180) / Math.PI)}°` } };
  }
  return { u: Math.round(raw[0] / grid) * grid, v: Math.round(raw[1] / grid) * grid, point: null, curve, snap: null };
}
