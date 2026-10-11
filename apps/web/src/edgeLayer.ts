// The edges of a part, drawn for picking: every edge thin, the one under the
// pointer and the picked ones as thick accent lines. Edges come from the
// exact kernel (true lines, circles and curves), in the part's own
// coordinates; the layer sits where the part is drawn.

import * as THREE from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import type { PartEdges } from "./engine";

/** What to draw: the part's edges and which are picked ("part:edge"). */
export type EdgePicture = { id: string; parts: PartEdges[]; picked: ReadonlySet<string>; previewing?: boolean };

const ACCENT = "#6d4ae0";
const HOVER = "#9a7bff";
/** How near (px) the pointer must be to an edge to pick it. */
const REACH = 9;

const pairs = (points: [number, number, number][]) => {
  const out: number[] = [];
  for (let i = 1; i < points.length; i++) out.push(...points[i - 1], ...points[i]);
  return out;
};

export class EdgeLayer {
  readonly root = new THREE.Group();
  private pic: EdgePicture | null = null;
  private base: THREE.LineSegments | null = null;
  private picked: LineSegments2 | null = null;
  private hover: LineSegments2 | null = null;
  private hoverKey: string | null = null;
  private resolution = new THREE.Vector2(1, 1);
  private materials: LineMaterial[] = [];

  constructor() {
    this.root.renderOrder = 900;
  }

  setResolution(width: number, height: number) {
    this.resolution.set(width, height);
    for (const m of this.materials) m.resolution.set(width, height);
  }

  /** Redraws when the edges or the picks change; `origin` is where the part sits (scene mm). */
  update(pic: EdgePicture | null, origin: THREE.Vector3 | null) {
    const sameEdges = pic && this.pic && pic.parts === this.pic.parts;
    this.pic = pic;
    if (!pic) {
      this.clear();
      return;
    }
    if (origin) this.root.position.copy(origin);
    if (!sameEdges) {
      this.clear();
      const all = pic.parts.flatMap((p) => p.edges.flatMap((e) => pairs(e.points)));
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", new THREE.Float32BufferAttribute(all, 3));
      this.base = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: "#2d2840", transparent: true, opacity: 0.55, toneMapped: false, fog: false }));
      this.base.renderOrder = 900;
      this.root.add(this.base);
    }
    this.setPicked(pic.picked);
    this.setHover(this.hoverKey);
  }

  private fat(points: number[], color: string, width: number, opacity = 1) {
    const g = new LineSegmentsGeometry();
    g.setPositions(points);
    const m = new LineMaterial({ color, linewidth: width, transparent: opacity < 1, opacity, toneMapped: false, fog: false, depthTest: true });
    m.resolution.copy(this.resolution);
    this.materials.push(m);
    const l = new LineSegments2(g, m);
    l.renderOrder = 901;
    return l;
  }

  private drop(l: LineSegments2 | null) {
    if (!l) return;
    this.root.remove(l);
    l.geometry.dispose();
    const m = l.material as LineMaterial;
    m.dispose();
    this.materials = this.materials.filter((x) => x !== m);
  }

  private edge(key: string) {
    const [p, e] = key.split(":").map(Number);
    return this.pic?.parts[p]?.edges[e];
  }

  private setPicked(keys: ReadonlySet<string>) {
    this.drop(this.picked);
    this.picked = null;
    const pts = [...keys].flatMap((k) => (this.edge(k) ? pairs(this.edge(k)!.points) : []));
    // While the result is previewed the picked edges step back: the rounded surface is the news.
    if (pts.length) this.root.add((this.picked = this.pic?.previewing ? this.fat(pts, ACCENT, 2.5, 0.35) : this.fat(pts, ACCENT, 4)));
  }

  setHover(key: string | null) {
    this.hoverKey = key;
    this.drop(this.hover);
    this.hover = null;
    const e = key ? this.edge(key) : undefined;
    if (e && !this.pic?.picked.has(key!)) this.root.add((this.hover = this.fat(pairs(e.points), HOVER, 3.5, 0.9)));
  }

  get hovered() {
    return this.hoverKey;
  }

  /**
   * The edge under the pointer: nearest on screen within reach, skipping
   * edges hidden behind the part (`hidden` says whether a scene point is).
   */
  hit(toScreen: (p: THREE.Vector3) => { x: number; y: number; behind: boolean }, mx: number, my: number, hidden: (p: THREE.Vector3) => boolean): string | null {
    if (!this.pic) return null;
    const o = this.root.position;
    const found: { key: string; d: number; at: THREE.Vector3 }[] = [];
    this.pic.parts.forEach((part, pi) =>
      part.edges.forEach((edge, ei) => {
        let best: { d: number; at: THREE.Vector3 } | null = null;
        for (let i = 1; i < edge.points.length; i++) {
          const a = new THREE.Vector3(...edge.points[i - 1]).add(o);
          const b = new THREE.Vector3(...edge.points[i]).add(o);
          const [sa, sb] = [toScreen(a), toScreen(b)];
          if (sa.behind || sb.behind) continue;
          const [dx, dy] = [sb.x - sa.x, sb.y - sa.y];
          const len2 = dx * dx + dy * dy;
          const t = len2 > 0 ? Math.max(0, Math.min(1, ((mx - sa.x) * dx + (my - sa.y) * dy) / len2)) : 0;
          const d = Math.hypot(sa.x + dx * t - mx, sa.y + dy * t - my);
          if (d < REACH && (!best || d < best.d)) best = { d, at: a.lerp(b, t) };
        }
        if (best) found.push({ key: `${pi}:${ei}`, ...best });
      }),
    );
    found.sort((a, b) => a.d - b.d);
    return found.find((f) => !hidden(f.at))?.key ?? null;
  }

  private clear() {
    this.drop(this.picked);
    this.drop(this.hover);
    this.picked = this.hover = null;
    if (this.base) {
      this.root.remove(this.base);
      this.base.geometry.dispose();
      (this.base.material as THREE.Material).dispose();
      this.base = null;
    }
  }
}
