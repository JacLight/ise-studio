// Canvases: reference pictures placed on a plane of the model (a product
// photo, a scanned drawing), drawn in the 3D view at their calibrated size.
// The mapping from the picture's pixels to the plane is the engine's own
// (crates/plugins/shape/src/canvas.rs, Canvas::to_plane): affine
//   plane = D · R · F · Y · (pixel − anchor)
// or, squared up from four points of a rectangle (perspective),
//   plane = R · F · H · pixel.
// Each canvas is drawn as a fine grid laid over its (cropped) picture, so a
// perspective correction bends the picture exactly as the engine reads it.

import * as THREE from "three";
import type { EntityView } from "./engine";
import { projectUrl } from "./engine";

type V2 = [number, number];
type V3 = [number, number, number];
export type M3 = number[]; // row-major 3 × 3

export type CanvasData = {
  name: string;
  image: string;
  pixels: [number, number];
  plane: { origin: V3; x: V3; y: V3 };
  view?: string | null;
  scale: number;
  aspect?: number;
  anchor: V2;
  rotation?: number;
  flip?: boolean;
  perspective?: { points: [V2, V2, V2, V2]; width: number; height: number } | null;
  calibration?: { a: V2; b: V2; distance: number } | null;
  crop?: [number, number, number, number] | null;
  opacity: number;
  visible: boolean;
  through?: boolean;
  background?: V3 | null;
  threshold?: number | null;
  sized?: boolean;
};

export const canvasOf = (e: EntityView | undefined | null): CanvasData | null => (e?.components["shape.canvas"] as CanvasData | undefined) ?? null;

const mul = (a: M3, b: M3): M3 => {
  const m = new Array(9).fill(0);
  for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) for (let k = 0; k < 3; k++) m[i * 3 + j] += a[i * 3 + k] * b[k * 3 + j];
  return m;
};

export const apply = (m: M3, p: V2): V2 => {
  const w = m[6] * p[0] + m[7] * p[1] + m[8];
  return [(m[0] * p[0] + m[1] * p[1] + m[2]) / w, (m[3] * p[0] + m[4] * p[1] + m[5]) / w];
};

export function inverse(m: M3): M3 | null {
  const [a, b, c, d, e, f, g, h, i] = m;
  const det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
  if (!Number.isFinite(det) || Math.abs(det) < 1e-300) return null;
  return [(e * i - f * h) / det, -(b * i - c * h) / det, (b * f - c * e) / det, -(d * i - f * g) / det, (a * i - c * g) / det, -(a * f - c * d) / det, (d * h - e * g) / det, -(a * h - b * g) / det, (a * e - b * d) / det];
}

/** The projective map taking each `from` point to its `to` point (h₃₃ = 1). */
export function homography(from: V2[], to: V2[]): M3 | null {
  const a: number[][] = [];
  for (let k = 0; k < 4; k++) {
    const [x, y] = from[k];
    const [u, v] = to[k];
    a.push([x, y, 1, 0, 0, 0, -u * x, -u * y, u]);
    a.push([0, 0, 0, x, y, 1, -v * x, -v * y, v]);
  }
  for (let col = 0; col < 8; col++) {
    let pivot = col;
    for (let r = col + 1; r < 8; r++) if (Math.abs(a[r][col]) > Math.abs(a[pivot][col])) pivot = r;
    if (Math.abs(a[pivot][col]) < 1e-12) return null;
    [a[col], a[pivot]] = [a[pivot], a[col]];
    for (let r = 0; r < 8; r++) {
      if (r === col) continue;
      const f = a[r][col] / a[col][col];
      for (let k = col; k < 9; k++) a[r][k] -= f * a[col][k];
    }
  }
  const h = a.map((row, i) => row[8] / row[i]);
  return [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1];
}

/** R · F: the turn after the mirror. */
function turn(rotation: number, flip: boolean): M3 {
  const r = (rotation * Math.PI) / 180;
  const [s, c] = [Math.sin(r), Math.cos(r)];
  const f = flip ? -1 : 1;
  return [c * f, -s, 0, s * f, c, 0, 0, 0, 1];
}

/** Pixel → plane millimetres, as the engine computes it. */
export function toPlane(c: CanvasData): M3 {
  const rf = turn(c.rotation ?? 0, !!c.flip);
  if (c.perspective) {
    const [w, h] = [c.perspective.width / 2, c.perspective.height / 2];
    const hm = homography(c.perspective.points, [[-w, h], [w, h], [w, -h], [-w, -h]]);
    if (hm) return mul(rf, hm);
  }
  const aspect = c.aspect ?? 1;
  const d = [c.scale, 0, 0, 0, c.scale * aspect, 0, 0, 0, 1];
  const y = [1, 0, -c.anchor[0], 0, -1, c.anchor[1], 0, 0, 1];
  return mul(d, mul(rf, y));
}

export const toPixel = (c: CanvasData): M3 => inverse(toPlane(c)) ?? [1, 0, 0, 0, 1, 0, 0, 0, 1];

/** The counted (cropped) region in pixels [left, top, right, bottom]. */
export function region(c: CanvasData): [number, number, number, number] {
  const [w, h] = c.pixels;
  if (!c.crop) return [0, 0, w, h];
  const cl = (v: number, hi: number) => Math.max(0, Math.min(hi, v));
  return [cl(c.crop[0], w), cl(c.crop[1], h), cl(c.crop[2], w), cl(c.crop[3], h)];
}

/** A plane point (mm) in the world. */
export const planeToWorld = (c: CanvasData, p: V2): V3 => [0, 1, 2].map((k) => c.plane.origin[k] + c.plane.x[k] * p[0] + c.plane.y[k] * p[1]) as V3;

/** A world point dropped onto the plane (mm). */
export function worldToPlane(c: CanvasData, w: V3): V2 {
  const d = [w[0] - c.plane.origin[0], w[1] - c.plane.origin[1], w[2] - c.plane.origin[2]];
  const dot = (v: V3) => d[0] * v[0] + d[1] * v[1] + d[2] * v[2];
  return [dot(c.plane.x), dot(c.plane.y)];
}

export const pixelToWorld = (c: CanvasData, p: V2): V3 => planeToWorld(c, apply(toPlane(c), p));
export const worldToPixel = (c: CanvasData, w: V3): V2 => apply(toPixel(c), worldToPlane(c, w));

/** Millimetres per pixel near a pixel. */
export function mmPerPx(c: CanvasData, p?: V2): number {
  const m = toPlane(c);
  const [x0, y0, x1, y1] = region(c);
  const q = p ?? [(x0 + x1) / 2, (y0 + y1) / 2];
  const a = apply(m, q);
  const b = apply(m, [q[0] + 1, q[1]]);
  const d = apply(m, [q[0], q[1] + 1]);
  return Math.sqrt(Math.abs((b[0] - a[0]) * (d[1] - a[1]) - (b[1] - a[1]) * (d[0] - a[0])));
}

/** The picture's size in mm on its plane (of the cropped region, along its top and left edges). */
export function sizeMm(c: CanvasData): [number, number] {
  const m = toPlane(c);
  const [x0, y0, x1, y1] = region(c);
  const a = apply(m, [x0, y0]);
  const b = apply(m, [x1, y0]);
  const d = apply(m, [x0, y1]);
  return [Math.hypot(b[0] - a[0], b[1] - a[1]), Math.hypot(d[0] - a[0], d[1] - a[1])];
}

export const VIEW_LABEL: Record<string, string> = { front: "Front", back: "Back", left: "Left", right: "Right", top: "Top", bottom: "Bottom" };

const GRID = 24;

/** What the layer draws besides the pictures: points picked for a calibration. */
export type CanvasPicking = { id: string; points: V2[] } | null;

type Drawn = { key: string; mesh: THREE.Mesh; frame: THREE.LineSegments };

/** The canvases in the 3D view (one textured grid each), kept in step with the project. */
export class CanvasLayer {
  root = new THREE.Group();
  private drawn = new Map<string, Drawn>();
  private textures = new Map<string, THREE.Texture>();
  private pickRoot = new THREE.Group();
  private loader = new THREE.TextureLoader();
  /** Called when a picture finishes loading (redraw). */
  onLoaded: (() => void) | null = null;

  constructor() {
    this.root.name = "canvases";
    this.root.add(this.pickRoot);
  }

  private texture(sha: string): THREE.Texture {
    let t = this.textures.get(sha);
    if (!t) {
      t = this.loader.load(projectUrl(`/images/${sha}`), () => this.onLoaded?.());
      t.colorSpace = THREE.SRGBColorSpace;
      t.anisotropy = 4;
      this.textures.set(sha, t);
    }
    return t;
  }

  /** Brings the drawn canvases up to date (only what changed is rebuilt). */
  update(entities: ReadonlyMap<string, EntityView>, selected: string | null, picking: CanvasPicking) {
    const seen = new Set<string>();
    for (const e of entities.values()) {
      const c = canvasOf(e);
      if (!c) continue;
      seen.add(e.id);
      const chosen = selected === e.id || picking?.id === e.id;
      const key = JSON.stringify([c, chosen]);
      const old = this.drawn.get(e.id);
      if (old && old.key === key) continue;
      if (old) this.drop(e.id);
      this.drawn.set(e.id, { key, ...this.build(e.id, c, chosen) });
    }
    for (const id of [...this.drawn.keys()]) if (!seen.has(id)) this.drop(id);
    this.drawPicks(entities, picking);
  }

  private build(id: string, c: CanvasData, chosen: boolean) {
    const m = toPlane(c);
    const [x0, y0, x1, y1] = region(c);
    const [w, h] = c.pixels;
    const n = GRID + 1;
    const pos = new Float32Array(n * n * 3);
    const uv = new Float32Array(n * n * 2);
    for (let j = 0; j < n; j++) {
      for (let i = 0; i < n; i++) {
        const u = x0 + ((x1 - x0) * i) / GRID;
        const v = y0 + ((y1 - y0) * j) / GRID;
        const p = planeToWorld(c, apply(m, [u, v]));
        const k = j * n + i;
        pos.set(p, k * 3);
        uv[k * 2] = u / w;
        uv[k * 2 + 1] = 1 - v / h;
      }
    }
    const index: number[] = [];
    for (let j = 0; j < GRID; j++) {
      for (let i = 0; i < GRID; i++) {
        const a = j * n + i;
        index.push(a, a + n, a + 1, a + 1, a + n, a + n + 1);
      }
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
    geo.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
    geo.setIndex(index);
    const through = !!c.through;
    const mat = new THREE.MeshBasicMaterial({
      map: this.texture(c.image),
      transparent: true,
      opacity: Math.max(0, Math.min(1, c.opacity)),
      side: THREE.DoubleSide,
      depthWrite: false,
      depthTest: !through,
      toneMapped: false,
    });
    const mesh = new THREE.Mesh(geo, mat);
    mesh.renderOrder = through ? 900 : -1;
    mesh.visible = c.visible;
    mesh.userData.canvas = id;
    // Never picked as a part: clicks reach the model (calibration casts its own rays).
    mesh.raycast = () => {};
    // The picture's edge, so it reads as a sheet even when faint; the chosen one in the accent.
    const corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map((p) => new THREE.Vector3(...planeToWorld(c, apply(m, p as V2))));
    const edge: THREE.Vector3[] = [];
    for (let k = 0; k < 4; k++) edge.push(corners[k], corners[(k + 1) % 4]);
    const frame = new THREE.LineSegments(
      new THREE.BufferGeometry().setFromPoints(edge),
      new THREE.LineBasicMaterial({ color: chosen ? 0x8b66ef : 0x9aa3b2, transparent: true, opacity: chosen ? 0.95 : 0.45, depthTest: !through }),
    );
    frame.renderOrder = through ? 901 : 0;
    frame.visible = c.visible;
    frame.raycast = () => {};
    this.root.add(mesh, frame);
    return { mesh, frame };
  }

  private drop(id: string) {
    const d = this.drawn.get(id);
    if (!d) return;
    this.root.remove(d.mesh, d.frame);
    d.mesh.geometry.dispose();
    (d.mesh.material as THREE.Material).dispose();
    d.frame.geometry.dispose();
    (d.frame.material as THREE.Material).dispose();
    this.drawn.delete(id);
  }

  private drawPicks(entities: ReadonlyMap<string, EntityView>, picking: CanvasPicking) {
    for (const o of [...this.pickRoot.children]) {
      this.pickRoot.remove(o);
      const m = o as THREE.Points;
      m.geometry.dispose();
      (m.material as THREE.Material).dispose();
    }
    const c = picking ? canvasOf(entities.get(picking.id)) : null;
    if (!picking || !c || !picking.points.length) return;
    const pts = picking.points.map((p) => new THREE.Vector3(...pixelToWorld(c, p)));
    const dots = new THREE.Points(new THREE.BufferGeometry().setFromPoints(pts), new THREE.PointsMaterial({ color: 0xe0761f, size: 10, sizeAttenuation: false, depthTest: false, transparent: true }));
    dots.renderOrder = 950;
    dots.raycast = () => {};
    this.pickRoot.add(dots);
    if (pts.length > 1) {
      const line = new THREE.Line(new THREE.BufferGeometry().setFromPoints(pts), new THREE.LineBasicMaterial({ color: 0xe0761f, depthTest: false, transparent: true }));
      line.renderOrder = 950;
      line.raycast = () => {};
      this.pickRoot.add(line);
    }
  }

  /** Where a ray meets a canvas's picture, as a pixel of it (null: it misses). */
  hit(ray: THREE.Ray, entities: ReadonlyMap<string, EntityView>, id: string): V2 | null {
    const c = canvasOf(entities.get(id));
    if (!c) return null;
    const n = new THREE.Vector3(...c.plane.x).cross(new THREE.Vector3(...c.plane.y)).normalize();
    const plane = new THREE.Plane().setFromNormalAndCoplanarPoint(n, new THREE.Vector3(...c.plane.origin));
    const at = ray.intersectPlane(plane, new THREE.Vector3());
    if (!at) return null;
    const px = worldToPixel(c, at.toArray() as V3);
    const [x0, y0, x1, y1] = region(c);
    if (!(px[0] >= x0 && px[0] <= x1 && px[1] >= y0 && px[1] <= y1)) return null;
    return px;
  }

  dispose() {
    for (const id of [...this.drawn.keys()]) this.drop(id);
    for (const t of this.textures.values()) t.dispose();
    this.textures.clear();
  }
}
