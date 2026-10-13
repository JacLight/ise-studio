// Draws draft objects in the 3D view: lines, wires, arcs, B-splines and
// outlines as the engine resolved them (in project coordinates), hatches,
// points, dimension and leader arrows, and text, labels and dimension values
// as HTML laid out each frame. Layers hide or colour them.

import * as THREE from "three";
import type { DraftShape } from "./engine";
import { displayPrefs, showLength } from "./units";

export type DraftItem = {
  id: string;
  kind: string;
  shape: DraftShape;
  color: string | null;
  size?: number;
};

export type DraftPicture = {
  items: DraftItem[];
  selected: string | null;
};

const INK = "#2563c9";
const NOTE = "#b4471d";
const SELECT = "#e0761f";

export class DraftLayer {
  root = new THREE.Group();
  private labels: { el: HTMLDivElement; at: THREE.Vector3 }[] = [];
  private host: HTMLElement | null = null;
  private key = "";
  onSelect: ((id: string) => void) | null = null;

  constructor() {
    this.root.renderOrder = 940;
  }

  setHost(el: HTMLElement) {
    this.host = el;
  }

  private clear() {
    this.root.traverse((o) => {
      const m = o as THREE.Mesh;
      m.geometry?.dispose?.();
      const mat = m.material as THREE.Material | THREE.Material[] | undefined;
      if (Array.isArray(mat)) mat.forEach((x) => x.dispose());
      else mat?.dispose?.();
    });
    this.root.clear();
    for (const l of this.labels) l.el.remove();
    this.labels = [];
  }

  update(pic: DraftPicture | null) {
    const prefs = displayPrefs();
    const key = pic ? JSON.stringify([pic.selected, prefs.length, prefs.decimals, pic.items.map((i) => [i.id, i.color, i.shape])]) : "";
    if (key === this.key) return;
    this.key = key;
    this.clear();
    if (!pic) return;
    for (const item of pic.items) {
      const chosen = pic.selected === item.id;
      const annotation = item.kind === "dimension" || item.kind === "label";
      const color = new THREE.Color(chosen ? SELECT : (item.color ?? (annotation ? NOTE : INK)));
      const segs: number[] = [];
      for (const line of item.shape.lines) {
        for (let i = 1; i < line.length; i++) segs.push(...line[i - 1], ...line[i]);
      }
      for (const [a, b] of item.shape.hatch ?? []) segs.push(...a, ...b);
      // Arrowheads as two short strokes, sized to the line they end.
      const span = Math.max(1, ...item.shape.lines.flat().map((p, _, all) => Math.hypot(p[0] - all[0][0], p[1] - all[0][1], p[2] - all[0][2])));
      const head = Math.min(6, span * 0.08);
      for (const [tip, dir] of item.shape.arrows ?? []) {
        const d = new THREE.Vector3(...dir).normalize();
        const side = new THREE.Vector3().crossVectors(d, Math.abs(d.z) < 0.9 ? new THREE.Vector3(0, 0, 1) : new THREE.Vector3(1, 0, 0)).normalize();
        const t = new THREE.Vector3(...tip);
        for (const s of [1, -1]) {
          const p = t.clone().addScaledVector(d, -head).addScaledVector(side, (s * head) / 3);
          segs.push(t.x, t.y, t.z, p.x, p.y, p.z);
        }
      }
      if (segs.length) {
        const g = new THREE.BufferGeometry();
        g.setAttribute("position", new THREE.Float32BufferAttribute(segs, 3));
        const m = new THREE.LineBasicMaterial({ color, depthTest: annotation ? false : true, transparent: true, opacity: chosen ? 1 : 0.92 });
        const lines = new THREE.LineSegments(g, m);
        lines.renderOrder = 940;
        lines.userData.draft = item.id;
        this.root.add(lines);
      }
      if (item.kind === "point" || (chosen && item.kind !== "text")) {
        const g = new THREE.BufferGeometry();
        g.setAttribute("position", new THREE.Float32BufferAttribute(item.shape.points.flat(), 3));
        const pts = new THREE.Points(g, new THREE.PointsMaterial({ color, size: item.kind === "point" ? 7 : 5, sizeAttenuation: false, depthTest: false }));
        pts.renderOrder = 941;
        this.root.add(pts);
      }
      if (item.shape.label && this.host) {
        const [at, raw] = item.shape.label;
        // Dimension values in the person's length unit (the engine says mm).
        const text = item.kind === "dimension" && item.shape.value != null && / mm$/.test(raw) ? raw.replace(/[\d.,-]+ mm$/, showLength(item.shape.value, prefs)) : raw;
        const el = document.createElement("div");
        el.className = `draft-label ${item.kind} ${chosen ? "chosen" : ""}`;
        el.textContent = text;
        el.style.setProperty("--ink", chosen ? SELECT : (item.color ?? (item.kind === "text" ? "#2f3140" : NOTE)));
        if (item.size && item.kind === "text") el.style.fontSize = `${Math.max(10, Math.min(40, item.size * 1.6))}px`;
        el.addEventListener("pointerdown", (e) => {
          e.stopPropagation();
          this.onSelect?.(item.id);
        });
        this.host.appendChild(el);
        this.labels.push({ el, at: new THREE.Vector3(...at) });
      }
    }
  }

  /** Places the HTML labels where their points are on screen. */
  layout(toScreen: (p: THREE.Vector3) => { x: number; y: number; behind: boolean }) {
    for (const l of this.labels) {
      const p = toScreen(l.at);
      l.el.style.display = p.behind || !this.root.visible ? "none" : "block";
      l.el.style.transform = `translate(${p.x}px, ${p.y}px) translate(-50%, -50%)`;
    }
  }

  setVisible(v: boolean) {
    this.root.visible = v;
  }
}
