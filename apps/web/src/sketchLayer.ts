// Draws a sketch on its plane in the 3D view: curves, points, a faint grid,
// the preview of what the current tool would add, and labels for
// constraints and dimensions (HTML, laid out each frame).

import * as THREE from "three";
import { type Constraint, type Geometry, type Plane, type Segment, type SketchData, type Solved, glyph, handleIds, normalOf, positions, radiusOf, toWorld } from "./sketchModel";

export type SketchPicture = {
  data: SketchData;
  solved?: Solved;
  hoverPoint: number | null;
  hoverCurve: number | null;
  /** What is selected: points and curves. */
  selected: number[];
  /** What the current tool would add (dashed). */
  preview: Segment[];
  /** Where the next click lands (a snap marker). */
  cursor: [number, number] | null;
  /** Grid spacing (mm) for the faint grid. */
  grid: number;
};

const INK = new THREE.Color("#5b3fc4");
const SELECT = new THREE.Color("#e0761f");
const CONSTRUCTION = new THREE.Color("#9a92ad");
const HANDLE = new THREE.Color("#2f8ad0");

function arcPoints(center: [number, number], r: number, a0: number, a1: number, n = 64): [number, number][] {
  const out: [number, number][] = [];
  for (let i = 0; i <= n; i++) {
    const a = a0 + ((a1 - a0) * i) / n;
    out.push([center[0] + r * Math.cos(a), center[1] + r * Math.sin(a)]);
  }
  return out;
}

export function segmentPoints(s: Segment): [number, number][] {
  if (s.kind === "line") return [s.from, s.to];
  if (s.kind === "bezier") {
    const out: [number, number][] = [];
    for (let i = 0; i <= 64; i++) {
      const t = i / 64;
      const u = 1 - t;
      const [a, b, c, d] = [u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t];
      out.push([a * s.from[0] + b * s.c1[0] + c * s.c2[0] + d * s.to[0], a * s.from[1] + b * s.c1[1] + c * s.c2[1] + d * s.to[1]]);
    }
    return out;
  }
  if (s.kind === "circle") return arcPoints(s.center, Math.abs(s.radius), 0, 2 * Math.PI);
  const r = Math.hypot(s.from[0] - s.center[0], s.from[1] - s.center[1]);
  const a0 = Math.atan2(s.from[1] - s.center[1], s.from[0] - s.center[0]);
  let a1 = Math.atan2(s.to[1] - s.center[1], s.to[0] - s.center[0]);
  if (s.ccw) while (a1 <= a0) a1 += 2 * Math.PI;
  else while (a1 >= a0) a1 -= 2 * Math.PI;
  return arcPoints(s.center, r, a0, a1);
}

export class SketchLayer {
  root = new THREE.Group();
  /** What the sketch is (rebuilt only when it changes). */
  private base = new THREE.Group();
  /** Hover, selection, preview and cursor (rebuilt on every pointer move; a few objects). */
  private overlay = new THREE.Group();
  private baseKey = "";
  private labels: { el: HTMLDivElement; at: THREE.Vector3 }[] = [];
  private host: HTMLElement | null = null;
  /** A dimension label was clicked (to edit its value). */
  onEditDimension: ((key: string, x: number, y: number) => void) | null = null;

  constructor() {
    this.root.renderOrder = 950;
    this.root.add(this.base, this.overlay);
  }

  setHost(el: HTMLElement) {
    this.host = el;
  }

  private static empty(group: THREE.Group) {
    group.traverse((o) => {
      if (o instanceof THREE.Line || o instanceof THREE.Points || o instanceof THREE.Mesh) {
        o.geometry.dispose();
        (o.material as THREE.Material).dispose();
      }
    });
    group.clear();
  }

  private clearLabels() {
    for (const l of this.labels) l.el.remove();
    this.labels = [];
  }

  /** Many polylines as one object (one draw call): each polyline becomes segment pairs. */
  private lines(into: THREE.Group, plane: Plane, polylines: [number, number][][], color: THREE.Color, opts: { dashed?: boolean; opacity?: number } = {}) {
    const pts: THREE.Vector3[] = [];
    for (const pl of polylines) {
      for (let i = 1; i < pl.length; i++) pts.push(new THREE.Vector3(...toWorld(plane, ...pl[i - 1])), new THREE.Vector3(...toWorld(plane, ...pl[i])));
    }
    if (!pts.length) return;
    const g = new THREE.BufferGeometry().setFromPoints(pts);
    const opacity = opts.opacity ?? 1;
    const m = opts.dashed
      ? new THREE.LineDashedMaterial({ color, dashSize: 3, gapSize: 2, depthTest: false, transparent: true, opacity, toneMapped: false, fog: false })
      : new THREE.LineBasicMaterial({ color, depthTest: false, transparent: true, opacity, toneMapped: false, fog: false });
    const l = new THREE.LineSegments(g, m);
    if (opts.dashed) l.computeLineDistances();
    l.renderOrder = 951;
    into.add(l);
  }

  private dots(into: THREE.Group, plane: Plane, at: [number, number][], color: THREE.Color, size: number) {
    if (!at.length) return;
    const g = new THREE.BufferGeometry().setFromPoints(at.map(([u, v]) => new THREE.Vector3(...toWorld(plane, u, v))));
    const p = new THREE.Points(g, new THREE.PointsMaterial({ size, sizeAttenuation: false, color, depthTest: false, toneMapped: false, fog: false }));
    p.renderOrder = 952;
    into.add(p);
  }

  private label(plane: Plane, at: [number, number], text: string, cls: string, key?: string) {
    if (!this.host) return;
    const el = document.createElement("div");
    el.className = `sketch-label ${cls}`;
    el.textContent = text;
    if (key) {
      el.dataset.key = key;
      el.title = "Click to change this dimension (a number, or =formula)";
      el.addEventListener("pointerdown", (e) => {
        e.stopPropagation();
        e.preventDefault();
        const r = el.getBoundingClientRect();
        this.onEditDimension?.(key, r.left + r.width / 2, r.top + r.height / 2);
      });
    }
    this.host.appendChild(el);
    this.labels.push({ el, at: new THREE.Vector3(...toWorld(plane, at[0], at[1])) });
  }

  /** Redraws the sketch: the drawing itself only when it changed, the highlights every time. */
  update(pic: SketchPicture | null) {
    if (!pic) {
      SketchLayer.empty(this.base);
      SketchLayer.empty(this.overlay);
      this.clearLabels();
      this.baseKey = "";
      return;
    }
    const { plane, geometry: g, params } = pic.data;
    const pos = positions(g, pic.solved);
    const at = (id: number) => pos.get(id) ?? ([0, 0] as [number, number]);
    const segOf = (id: number): Segment | null => {
      const c = g.curves.find((x) => x.id === id);
      if (!c) return null;
      if (c.kind === "line") return { kind: "line", from: at(c.a), to: at(c.b) };
      if (c.kind === "circle") return { kind: "circle", center: at(c.center), radius: radiusOf(c, pic.solved) };
      if (c.kind === "bezier") return { kind: "bezier", from: at(c.a), to: at(c.b), c1: at(c.c1), c2: at(c.c2) };
      return { kind: "arc", from: at(c.start), to: at(c.end), center: at(c.center), ccw: true };
    };

    const key = JSON.stringify([pic.data, pic.solved?.solved.points, pic.solved?.solved.radii, pic.grid]);
    if (key !== this.baseKey) {
      this.baseKey = key;
      SketchLayer.empty(this.base);
      this.clearLabels();

      // A faint grid around what is drawn (or the origin), and its axes.
      // Centred on what is drawn, on whole grid steps, reaching past it.
      const all = [...pos.values()];
      const step = pic.grid > 0 ? pic.grid : 10;
      const xs = all.map((p) => p[0]);
      const ys = all.map((p) => p[1]);
      const [cx, cy] = all.length ? [(Math.min(...xs) + Math.max(...xs)) / 2, (Math.min(...ys) + Math.max(...ys)) / 2] : [0, 0];
      const reach = Math.max(100, all.length ? Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)) * 1.2 : 0);
      const n = Math.min(80, Math.ceil(reach / step));
      const [gx, gy] = [Math.round(cx / step) * step, Math.round(cy / step) * step];
      const gridLines: [number, number][][] = [];
      for (let i = -n; i <= n; i++) {
        gridLines.push([[gx + i * step, gy - n * step], [gx + i * step, gy + n * step]], [[gx - n * step, gy + i * step], [gx + n * step, gy + i * step]]);
      }
      const gridColor = new THREE.Color("#b9b2cc");
      this.lines(this.base, plane, gridLines, gridColor, { opacity: 0.18 });
      // The sketch's own axes, where they fall inside the grid.
      const axes: [number, number][][] = [];
      if (Math.abs(gx) <= n * step) axes.push([[0, gy - n * step], [0, gy + n * step]]);
      if (Math.abs(gy) <= n * step) axes.push([[gx - n * step, 0], [gx + n * step, 0]]);
      this.lines(this.base, plane, axes, gridColor, { opacity: 0.6 });

      // Curves: drawn and construction.
      const drawn: [number, number][][] = [];
      const construction: [number, number][][] = [];
      for (const c of g.curves) {
        const seg = segOf(c.id);
        if (seg) (c.construction ? construction : drawn).push(segmentPoints(seg));
      }
      this.lines(this.base, plane, drawn, INK);
      this.lines(this.base, plane, construction, CONSTRUCTION, { dashed: true });
      // Bézier handles: a thin line from each end to its handle, the handle a smaller dot.
      const handles = handleIds(g);
      const arms: [number, number][][] = g.curves.flatMap((c) => (c.kind === "bezier" ? [[at(c.a), at(c.c1)], [at(c.b), at(c.c2)]] : []));
      this.lines(this.base, plane, arms, HANDLE, { opacity: 0.7 });
      this.dots(this.base, plane, g.points.filter((p) => !handles.has(p.id)).map((p) => at(p.id)), INK, 7);
      this.dots(this.base, plane, g.points.filter((p) => handles.has(p.id)).map((p) => at(p.id)), HANDLE, 6);

      // Constraint glyphs and dimension values, near what they constrain.
      const value = (k: string) => params.find((p) => p.key === k);
      const mid = (a: [number, number], b: [number, number]): [number, number] => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
      const lineMid = (id: number) => {
        const c = g.curves.find((x) => x.id === id);
        return c?.kind === "line" ? mid(at(c.a), at(c.b)) : null;
      };
      const fmt = (v: number) => Number(v.toFixed(2)).toLocaleString("en-US");
      const offsets = new Map<string, number>();
      const place = (p: [number, number]): [number, number] => {
        // Several labels on one spot step apart instead of stacking.
        const k = `${p[0].toFixed(3)},${p[1].toFixed(3)}`;
        const i = offsets.get(k) ?? 0;
        offsets.set(k, i + 1);
        return [p[0], p[1] + i * step * 0.8];
      };
      for (const c of g.constraints) {
        const sym = glyph(c);
        if (sym) {
          const target = "line" in c ? lineMid(c.line) : "a" in c ? lineMid(c.a) : "point" in c ? at(c.point) : null;
          if (target) this.label(plane, place(target), sym, "glyph");
          continue;
        }
        if (!("key" in c)) continue;
        const d = value(c.key);
        if (!d) continue;
        const text = `${d.expr ? "ƒ " : ""}${fmt(pic.solved ? dimValue(c, pos, pic.solved, g) : d.value)}${c.kind === "angle" ? "°" : ""}`;
        let where: [number, number] | null = null;
        if (c.kind === "length") where = lineMid(c.line);
        else if (c.kind === "radius" || c.kind === "diameter") {
          const cur = g.curves.find((x) => x.id === c.curve);
          if (cur && (cur.kind === "circle" || cur.kind === "arc")) where = at(cur.center);
        } else if ("a" in c && "b" in c) {
          const pa = g.points.some((p) => p.id === c.a) ? at(c.a) : lineMid(c.a);
          const pb = g.points.some((p) => p.id === c.b) ? at(c.b) : lineMid(c.b);
          if (pa && pb) where = mid(pa, pb);
        }
        if (where) this.label(plane, place(where), `${c.kind === "diameter" ? "⌀" : c.kind === "radius" ? "R" : ""}${text}`, `dim ${d.expr ? "linked" : ""}`, c.key);
      }
    }

    // Highlights, preview and cursor: a handful of objects, redrawn each time.
    SketchLayer.empty(this.overlay);
    const hot = [pic.hoverCurve, ...pic.selected].map((id) => (id === null ? null : segOf(id))).filter((x): x is Segment => !!x);
    this.lines(this.overlay, plane, hot.map(segmentPoints), SELECT);
    this.lines(this.overlay, plane, pic.preview.map(segmentPoints), SELECT, { dashed: true, opacity: 0.9 });
    const hotPoints = [pic.hoverPoint, ...pic.selected].filter((id): id is number => id !== null && pos.has(id)).map(at);
    this.dots(this.overlay, plane, hotPoints, SELECT, 9);
    if (pic.cursor) this.dots(this.overlay, plane, [pic.cursor], SELECT, 9);
  }

  /** Positions the labels on screen; call every frame. */
  layout(toScreen: (v: THREE.Vector3) => { x: number; y: number; behind: boolean }) {
    for (const { el, at } of this.labels) {
      const p = toScreen(at);
      el.style.display = p.behind ? "none" : "block";
      el.style.transform = `translate(${p.x}px, ${p.y}px) translate(-50%, -50%)`;
    }
  }
}

/** A dimension's value as solved (what the drawing shows). */
function dimValue(c: Constraint, pos: Map<number, [number, number]>, s: Solved, g: Geometry): number {
  const p = (id: number) => pos.get(id) ?? [0, 0];
  switch (c.kind) {
    case "length": {
      const l = g.curves.find((x) => x.id === c.line);
      return l?.kind === "line" ? Math.hypot(p(l.b)[0] - p(l.a)[0], p(l.b)[1] - p(l.a)[1]) : 0;
    }
    case "distance":
      return Math.hypot(p(c.b)[0] - p(c.a)[0], p(c.b)[1] - p(c.a)[1]);
    case "horizontal_distance":
      return p(c.b)[0] - p(c.a)[0];
    case "vertical_distance":
      return p(c.b)[1] - p(c.a)[1];
    case "radius":
    case "diameter": {
      const cur = g.curves.find((x) => x.id === c.curve);
      const r = cur?.kind === "circle" ? (s.solved.radii[cur.id] ?? cur.radius) : cur?.kind === "arc" ? Math.hypot(p(cur.start)[0] - p(cur.center)[0], p(cur.start)[1] - p(cur.center)[1]) : 0;
      return c.kind === "radius" ? r : 2 * r;
    }
    default:
      return 0;
  }
}

/** The camera direction that faces a sketch's plane (looking down its normal). */
export function facing(plane: Plane): { dir: THREE.Vector3; up: THREE.Vector3 } {
  const n = new THREE.Vector3(...normalOf(plane));
  return { dir: n, up: new THREE.Vector3(...plane.y) };
}
