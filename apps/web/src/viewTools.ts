// The view's tools, kept out of Viewport.tsx: orthographic/perspective,
// display modes, section planes, the measure tool, box selection and the
// view cube. Viewport calls in at a few points (before and after each draw,
// on pointer events) and its handle forwards commands here; the toolbar
// (ViewToolbar.tsx) and App follow the state through `subscribe`.

import * as THREE from "three";
import { Display, type DisplayMode } from "./display";
import { type MeasureCard, MeasureTool } from "./measure";
import { Projection, VIEW_DIRECTIONS, type ViewName, upFor } from "./projection";
import { type Axis, Section, type SectionPlane, axisPlane } from "./section";
import { type CubeStep, ViewCube } from "./viewcube";
import { tumble } from "./tumble";
import { Topology, worldMesh } from "./measureMath";

export type { DisplayMode, ViewName, SectionPlane, Axis };

export type ViewToolsState = {
  orthographic: boolean;
  display: DisplayMode;
  section: { planes: SectionPlane[]; capped: boolean; picking: boolean };
  measuring: boolean;
  card: MeasureCard;
  /** Model bounds (mm), for the section sliders. */
  bounds: { min: [number, number, number]; max: [number, number, number] } | null;
};

/** What the tools need from the viewport. */
export type ViewHost = {
  el: HTMLElement;
  canvas: HTMLCanvasElement;
  camera: THREE.PerspectiveCamera;
  target: () => THREE.Vector3;
  scene: THREE.Scene;
  /** Every visible solid drawn for the model. */
  bodies: () => THREE.Mesh[];
  materialsOf: (m: THREE.Mesh) => THREE.MeshStandardMaterial[];
  extent: () => THREE.Sphere;
  invalidate: () => void;
  /** Frames everything (or one item) looking along `direction`, gliding. */
  fit: (only?: string, direction?: THREE.Vector3, up?: THREE.Vector3) => void;
  selected: () => string | null;
  /** An item's name, for labels. */
  nameOf?: (id: string) => string;
  /** Rubber-band selection result. */
  onSelectMany: (ids: string[], add: boolean) => void;
};

/** Whether a box from `a` to `b` (screen px) selects what it fully contains (left to right) or what it touches (right to left). */
export const boxMode = (a: { x: number }, b: { x: number }) => (b.x >= a.x ? "window" : "crossing");

/** Whether points (screen px) of a part fall in a rectangle: all of them (window) or any (crossing). */
export function inBox(points: { x: number; y: number }[], r: { x0: number; y0: number; x1: number; y1: number }, mode: "window" | "crossing") {
  const [x0, x1] = [Math.min(r.x0, r.x1), Math.max(r.x0, r.x1)];
  const [y0, y1] = [Math.min(r.y0, r.y1), Math.max(r.y0, r.y1)];
  const inside = (p: { x: number; y: number }) => p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1;
  if (!points.length) return false;
  if (mode === "window") return points.every(inside);
  if (points.some(inside)) return true;
  // Crossing also takes a part the box sits inside of or cuts across without
  // catching a sampled point: its screen bounds overlap the box.
  const bx0 = Math.min(...points.map((p) => p.x));
  const bx1 = Math.max(...points.map((p) => p.x));
  const by0 = Math.min(...points.map((p) => p.y));
  const by1 = Math.max(...points.map((p) => p.y));
  return bx0 <= x1 && bx1 >= x0 && by0 <= y1 && by1 >= y0 && (x1 - x0 < bx1 - bx0 || y1 - y0 < by1 - by0);
}

export class ViewTools {
  readonly projection = new Projection();
  readonly display = new Display();
  readonly section = new Section();
  readonly measure: MeasureTool;
  readonly cube: ViewCube;
  private listeners = new Set<() => void>();
  private band: HTMLDivElement;
  private boxing: { x: number; y: number; ex: number; ey: number; add: boolean; pointer: number } | null = null;
  private clickFrom: { x: number; y: number } | null = null;
  state: ViewToolsState = { orthographic: false, display: "shaded", section: { planes: [], capped: true, picking: false }, measuring: false, card: [], bounds: null };

  constructor(private host: ViewHost) {
    this.measure = new MeasureTool(host.el, (card) => this.set({ card }), host.invalidate);
    host.scene.add(this.measure.root);
    this.cube = new ViewCube({
      pick: (dir) => this.look(dir),
      home: () => this.view("iso"),
      drag: (dx, dy) => {
        // The cube turns the view like dragging the model does, a little faster for its size.
        tumble(host.camera, host.target(), dx, dy, host.el.clientHeight, 1.6);
        host.invalidate();
      },
      step: (step) => this.step(step),
      view: (name) => this.view(name as ViewName),
    });
    host.el.append(this.cube.root);
    this.band = document.createElement("div");
    this.band.className = "select-band";
    this.band.hidden = true;
    host.el.append(this.band);
  }

  subscribe = (l: () => void) => {
    this.listeners.add(l);
    return () => void this.listeners.delete(l);
  };
  getState = () => this.state;

  private set(patch: Partial<ViewToolsState>) {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
    this.host.invalidate();
  }

  // ---- Commands -----------------------------------------------------------

  /** Looks from a standard view, everything framed. */
  view(name: ViewName) {
    this.look(new THREE.Vector3(...VIEW_DIRECTIONS[name]));
  }

  /** Turns the view 90° about the screen's vertical (left, right) or horizontal (up, down) axis. */
  step(step: CubeStep) {
    const cam = this.host.camera;
    const back = cam.position.clone().sub(this.host.target()).normalize();
    const up = cam.up.clone().normalize();
    const right = new THREE.Vector3().crossVectors(up, back).normalize();
    const sign = step === "left" || step === "up" ? 1 : -1;
    const axis = step === "left" || step === "right" ? up : right;
    const q = new THREE.Quaternion().setFromAxisAngle(axis, (sign * Math.PI) / 2);
    const dir = back.applyQuaternion(q);
    // Snap to the nearest axis so steps land exactly on Front, Back, Top, Bottom…
    const snap = (v: THREE.Vector3) => v.set(...(v.toArray().map((c) => (Math.abs(c) < 1e-6 ? 0 : Math.round(c * 1e6) / 1e6)) as [number, number, number]));
    this.host.fit(undefined, snap(dir).normalize(), snap(up.applyQuaternion(q)).normalize());
  }

  look(direction: THREE.Vector3) {
    const dir = direction.clone().normalize();
    this.host.fit(undefined, dir, upFor(dir));
  }

  /** Frames the selection (or everything), keeping the direction. */
  fitSelection(id: string | null) {
    this.host.fit(id ?? undefined);
  }

  setOrthographic(on: boolean) {
    this.projection.orthographic = on;
    this.set({ orthographic: on });
  }

  setDisplay(mode: DisplayMode) {
    this.display.mode = mode;
    this.set({ display: mode });
  }

  setSection(planes: SectionPlane[], capped = this.state.section.capped) {
    this.section.set(planes);
    this.section.capped = capped;
    this.set({ section: { planes, capped, picking: false }, bounds: this.boundsNow() });
  }

  /** Adds or moves the plane on an axis (offset in mm; flip keeps the other side). */
  sectionAxis(axis: Axis, offset: number, flip?: boolean) {
    const old = this.state.section.planes.find((p) => p.id === axis);
    const plane = axisPlane(axis, offset, flip ?? old?.flip ?? false);
    this.setSection([...this.state.section.planes.filter((p) => p.id !== axis), plane]);
  }

  /** A section box: six planes (mm), by default a fifth in from each side of the model. */
  sectionBox(min?: [number, number, number], max?: [number, number, number]) {
    const b = this.boundsNow();
    if (!b) return;
    const inset = (k: number) => (b.max[k] - b.min[k]) * 0.2;
    const lo = min ?? ([0, 1, 2].map((k) => b.min[k] + inset(k)) as [number, number, number]);
    const hi = max ?? ([0, 1, 2].map((k) => b.max[k] - inset(k)) as [number, number, number]);
    const planes: SectionPlane[] = [];
    (["x", "y", "z"] as Axis[]).forEach((axis, k) => {
      planes.push({ ...axisPlane(axis, hi[k], false), id: `${axis}+` });
      planes.push({ ...axisPlane(axis, lo[k], true), id: `${axis}-` });
    });
    this.setSection(planes);
  }

  /** Waits for a face to be clicked, then cuts along its plane. */
  pickSectionFace(on = true) {
    this.set({ section: { ...this.state.section, picking: on } });
  }

  setMeasuring(on: boolean) {
    if (!on) this.measure.clear();
    this.measure.active = on;
    this.host.el.classList.toggle("is-measuring", on);
    this.set({ measuring: on });
  }

  /** Measures whole items (one, or one to another), as Alt-clicking them would. */
  async measureItems(a: string, b: string | null) {
    this.measure.clear();
    for (const id of [a, b]) {
      if (!id) continue;
      const meshes = this.host.bodies().filter((m) => m.userData.id === id || (m.userData.path as string[] | undefined)?.includes(id));
      if (!meshes.length) continue;
      // One world-space mesh of everything drawn for it.
      const positions: number[] = [];
      const index: number[] = [];
      for (const m of meshes) {
        m.updateMatrixWorld();
        const w = worldMesh(m.geometry, m.matrixWorld);
        const base = positions.length / 3;
        for (let i = 0; i < w.positions.length; i++) positions.push(w.positions[i]);
        for (let i = 0; i < w.index.length; i++) index.push(w.index[i] + base);
      }
      const topo = new Topology({ positions, index });
      const triangles = topo.tris.map((_, i) => i);
      const c = new THREE.Box3().setFromArray(positions).getCenter(new THREE.Vector3());
      const name = this.host.nameOf?.(id) ?? id;
      await this.measure.pick({ feature: { kind: "part", id, name, triangles, centroid: [c.x, c.y, c.z] }, topo, id, name });
    }
  }

  // ---- Frame hooks --------------------------------------------------------

  /** The model's bounds now (mm). */
  boundsNow(): ViewToolsState["bounds"] {
    const b = new THREE.Box3();
    for (const m of this.host.bodies()) b.expandByObject(m);
    if (b.isEmpty()) return null;
    return { min: b.min.toArray() as [number, number, number], max: b.max.toArray() as [number, number, number] };
  }

  /** Before a draw: materials to the display mode and section; returns the camera to draw with. */
  beforeRender(): THREE.Camera {
    const bodies = this.host.bodies();
    const camera = this.camera();
    camera.updateMatrixWorld();
    this.display.clipping = this.section.active ? this.section.clipping : null;
    this.display.apply(bodies, this.host.materialsOf, camera, this.host.canvas.clientHeight || 800);
    // Detail still being found on a big model: another frame finishes it.
    if (this.display.detail.pending) requestAnimationFrame(() => this.host.invalidate());
    this.section.clip(bodies, this.host.materialsOf);
    return camera;
  }

  /** The camera to draw and pick with. */
  camera(): THREE.Camera {
    const e = this.host.extent();
    return this.projection.camera(this.host.camera, this.host.target(), e.radius * 4 + this.host.camera.position.distanceTo(e.center));
  }

  /** After the model is drawn: section caps; then labels and the cube follow the view. */
  afterRender(renderer: THREE.WebGLRenderer, camera: THREE.Camera) {
    if (this.section.active) this.section.renderCaps(renderer, camera, this.host.bodies(), this.host.extent());
    this.cube.update(camera);
    this.measure.layout((p) => this.toScreen(p, camera));
  }

  private toScreen(p: THREE.Vector3, camera: THREE.Camera) {
    const v = p.clone().project(camera);
    const rect = this.host.canvas.getBoundingClientRect();
    return new THREE.Vector2(((v.x + 1) / 2) * rect.width, ((1 - v.y) / 2) * rect.height);
  }

  // ---- Pointer hooks ------------------------------------------------------

  /** A press on the view: true when a tool takes it (the viewport's own handling then stands aside). */
  down(e: PointerEvent): boolean {
    if (e.button !== 0) return false;
    if ((e.ctrlKey || e.metaKey) && !this.state.measuring) {
      // Ctrl/⌘-drag: a selection box.
      const r = this.host.canvas.getBoundingClientRect();
      this.boxing = { x: e.clientX - r.left, y: e.clientY - r.top, ex: e.clientX - r.left, ey: e.clientY - r.top, add: e.shiftKey, pointer: e.pointerId };
      this.host.el.setPointerCapture(e.pointerId);
      e.stopPropagation();
      e.preventDefault();
      return true;
    }
    if (this.state.measuring || this.state.section.picking) {
      this.clickFrom = { x: e.clientX, y: e.clientY };
      return true; // orbiting still works; a click (no drag) picks
    }
    return false;
  }

  move(e: PointerEvent, ray: (x: number, y: number) => THREE.Raycaster): boolean {
    if (this.boxing) {
      const r = this.host.canvas.getBoundingClientRect();
      this.boxing.ex = e.clientX - r.left;
      this.boxing.ey = e.clientY - r.top;
      const b = this.boxing;
      Object.assign(this.band.style, { left: `${Math.min(b.x, b.ex)}px`, top: `${Math.min(b.y, b.ey)}px`, width: `${Math.abs(b.ex - b.x)}px`, height: `${Math.abs(b.ey - b.y)}px` });
      this.band.hidden = false;
      this.band.classList.toggle("is-crossing", boxMode({ x: b.x }, { x: b.ex }) === "crossing");
      e.stopPropagation();
      return true;
    }
    if (this.state.measuring && e.buttons === 0) {
      const hit = this.hitAt(e, ray);
      this.measure.setHover(hit ? this.measure.featureAt(hit, this.pointer(e), (p) => this.toScreen(p, this.camera()), e.altKey) : null);
    }
    return false;
  }

  /** A release while drawing a selection box. */
  boxUp(e: PointerEvent): boolean {
    if (this.boxing) {
      const b = this.boxing;
      this.boxing = null;
      this.band.hidden = true;
      try {
        this.host.el.releasePointerCapture(b.pointer);
      } catch {
        /* already released */
      }
      e.stopPropagation();
      if (Math.hypot(b.ex - b.x, b.ey - b.y) > 3) this.host.onSelectMany(this.inRect({ x0: b.x, y0: b.y, x1: b.ex, y1: b.ey }, boxMode({ x: b.x }, { x: b.ex })), b.add);
      return true;
    }
    return false;
  }

  /** A release on the canvas: a click (not a drag) picks for the measure tool or a section face. */
  click(e: PointerEvent, ray: (x: number, y: number) => THREE.Raycaster): boolean {
    const from = this.clickFrom;
    this.clickFrom = null;
    if (!from || Math.hypot(e.clientX - from.x, e.clientY - from.y) > 4) return !!from;
    const hit = this.hitAt(e, ray);
    if (this.state.section.picking) {
      if (hit?.face) {
        const n = hit.face.normal.clone().transformDirection(hit.object.matrixWorld).normalize();
        const plane: SectionPlane = { id: "face", normal: n.toArray() as [number, number, number], at: hit.point.toArray() as [number, number, number] };
        this.setSection([...this.state.section.planes.filter((p) => p.id !== "face"), plane]);
      } else this.pickSectionFace(false);
      return true;
    }
    if (this.state.measuring) {
      const pick = hit ? this.measure.featureAt(hit, this.pointer(e), (p) => this.toScreen(p, this.camera()), e.altKey) : null;
      if (pick) void this.measure.pick(pick);
      else this.measure.clear();
      return true;
    }
    return false;
  }

  private pointer(e: PointerEvent) {
    const r = this.host.canvas.getBoundingClientRect();
    return new THREE.Vector2(e.clientX - r.left, e.clientY - r.top);
  }

  /** The nearest body under the pointer that the section leaves standing. */
  private hitAt(e: PointerEvent, ray: (x: number, y: number) => THREE.Raycaster) {
    const hits = ray(e.clientX, e.clientY).intersectObjects(this.host.bodies(), false);
    return hits.find((h) => !this.section.active || this.section.clipping.every((p) => p.distanceToPoint(h.point) >= -1e-6)) ?? null;
  }

  /** Items whose drawn shape the rectangle selects. */
  private inRect(r: { x0: number; y0: number; x1: number; y1: number }, mode: "window" | "crossing"): string[] {
    const camera = this.camera();
    const byId = new Map<string, { x: number; y: number }[]>();
    const p = new THREE.Vector3();
    for (const body of this.host.bodies()) {
      const path = body.userData.path as string[] | undefined;
      // Instances by the top one (as a click picks), parts by their own id.
      const id = (path?.length ? path[0] : body.userData.id) as string | undefined;
      if (!id) continue;
      const pos = body.geometry.getAttribute("position") as THREE.BufferAttribute;
      const step = Math.max(1, Math.floor(pos.count / 1500));
      const list = byId.get(id) ?? [];
      for (let i = 0; i < pos.count; i += step) {
        p.fromBufferAttribute(pos, i).applyMatrix4(body.matrixWorld);
        if (this.section.active && this.section.clipping.some((c) => c.distanceToPoint(p) < 0)) continue;
        list.push(this.toScreen(p, camera));
      }
      byId.set(id, list);
    }
    return [...byId].filter(([, pts]) => inBox(pts, r, mode)).map(([id]) => id);
  }

  dispose() {
    this.measure.dispose();
    this.section.dispose();
    this.cube.root.remove();
    this.band.remove();
  }
}
