// Connectors drawn for picking a joint's ends: a small triad at each one —
// its axis long and blue, its x red, its y green, round a white grip — kept
// the same size on screen and drawn over the model. The one under the
// pointer grows and is named; a chosen one turns to the accent; ones that
// cannot be picked fade.

import * as THREE from "three";

type V3 = [number, number, number];

export type ConnectorMark = {
  key: string;
  /** "Servo › Output shaft" */
  label: string;
  /** A second line: what picking it does. */
  hint?: string;
  at: V3;
  axis: V3;
  x: V3;
  state: "normal" | "dim" | "chosen";
};

export type ConnectorPicture = { marks: ConnectorMark[] };

const ACCENT = "#7352d6";
const AXIS = "#3e63dd";
const XC = "#e5484d";
const YC = "#2f9e64";
/** Triad length on screen (px), and how near (px) the pointer must come to pick one. */
const SIZE = 44;
const REACH = 16;

function onTop(color: string, opacity = 1) {
  return new THREE.MeshBasicMaterial({ color, transparent: true, opacity, depthTest: false, depthWrite: false, toneMapped: false, fog: false, side: THREE.DoubleSide });
}

type Item = { mark: ConnectorMark; group: THREE.Group; grip: THREE.Group; mats: { m: THREE.MeshBasicMaterial; base: number }[]; ring: THREE.MeshBasicMaterial; core: THREE.MeshBasicMaterial };

const shaft = new THREE.CylinderGeometry(0.035, 0.035, 1, 10);
const cone = new THREE.ConeGeometry(0.11, 0.3, 24);
const disc = new THREE.CircleGeometry(0.12, 28);
const ring = new THREE.RingGeometry(0.12, 0.19, 32);

export class ConnectorLayer {
  readonly root = new THREE.Group();
  private items: Item[] = [];
  private pic: ConnectorPicture | null = null;
  hovered: string | null = null;
  private labels = new Map<string, HTMLDivElement>();

  get active() {
    return !!this.pic && this.pic.marks.length > 0;
  }

  update(pic: ConnectorPicture | null) {
    const same = pic && this.pic && pic.marks.length === this.pic.marks.length && pic.marks.every((m, i) => {
      const o = this.pic!.marks[i];
      return o.key === m.key && o.state === m.state && o.at.every((v, k) => v === m.at[k]) && o.axis.every((v, k) => v === m.axis[k]) && o.x.every((v, k) => v === m.x[k]);
    });
    this.pic = pic;
    if (same) return;
    this.clear();
    for (const mark of pic?.marks ?? []) this.items.push(this.build(mark));
    this.restyle();
  }

  private build(mark: ConnectorMark): Item {
    const group = new THREE.Group();
    const mats: Item["mats"] = [];
    const arrow = (dir: THREE.Vector3, color: string, length: number, tip: boolean) => {
      const m = onTop(color, 0.95);
      mats.push({ m, base: 0.95 });
      const g = new THREE.Group();
      const s = new THREE.Mesh(shaft, m);
      s.scale.set(1, length, 1);
      s.position.y = length / 2;
      g.add(s);
      if (tip) {
        const c = new THREE.Mesh(cone, m);
        c.position.y = length + 0.12;
        g.add(c);
      }
      g.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir.clone().normalize());
      group.add(g);
    };
    const z = new THREE.Vector3(...mark.axis);
    const x = new THREE.Vector3(...mark.x);
    const y = z.clone().cross(x);
    arrow(z, AXIS, 0.85, true);
    arrow(x, XC, 0.5, false);
    arrow(y, YC, 0.5, false);
    // The grip: a white disc in an accent ring, facing the camera.
    const grip = new THREE.Group();
    const ringMat = onTop(ACCENT, 0.95);
    const coreMat = onTop("#ffffff", 1);
    grip.add(new THREE.Mesh(ring, ringMat), new THREE.Mesh(disc, coreMat));
    group.add(grip);
    group.position.set(...mark.at);
    group.traverse((o) => (o.renderOrder = 1015));
    grip.traverse((o) => (o.renderOrder = 1016));
    this.root.add(group);
    return { mark, group, grip, mats, ring: ringMat, core: coreMat };
  }

  private clear() {
    for (const it of this.items) {
      this.root.remove(it.group);
      for (const { m } of it.mats) m.dispose();
      it.ring.dispose();
      it.core.dispose();
    }
    this.items = [];
    for (const el of this.labels.values()) el.remove();
    this.labels.clear();
    this.hovered = null;
  }

  setHover(key: string | null) {
    if (key === this.hovered) return false;
    this.hovered = key;
    this.restyle();
    return true;
  }

  private restyle() {
    for (const it of this.items) {
      const { state } = it.mark;
      const hot = it.mark.key === this.hovered;
      const dim = state === "dim" && !hot;
      for (const { m, base } of it.mats) m.opacity = dim ? 0.22 : base;
      it.ring.color.set(state === "chosen" || hot ? "#ffffff" : ACCENT);
      it.core.color.set(state === "chosen" || hot ? ACCENT : "#ffffff");
      it.ring.opacity = dim ? 0.3 : 0.95;
      it.core.opacity = dim ? 0.35 : 1;
    }
  }

  /** Keeps every triad the same size on screen and its grip facing the camera; names the hovered and chosen ones. */
  layout(camera: THREE.PerspectiveCamera, heightPx: number, toScreen: (p: THREE.Vector3) => { x: number; y: number; behind: boolean }, host: HTMLElement | null) {
    const perPixel = (at: THREE.Vector3) => (2 * camera.position.distanceTo(at) * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2))) / Math.max(1, heightPx);
    const named = new Set<string>();
    for (const it of this.items) {
      const hot = it.mark.key === this.hovered;
      const k = perPixel(it.group.position) * SIZE * (hot ? 1.3 : it.mark.state === "chosen" ? 1.15 : 1);
      it.group.scale.setScalar(k);
      it.grip.quaternion.copy(camera.quaternion).premultiply(it.group.quaternion.clone().invert());
      if (hot || it.mark.state === "chosen") named.add(it.mark.key);
    }
    if (!host) return;
    for (const [key, el] of this.labels) {
      if (!named.has(key)) {
        el.remove();
        this.labels.delete(key);
      }
    }
    for (const it of this.items) {
      if (!named.has(it.mark.key)) continue;
      let el = this.labels.get(it.mark.key);
      if (!el) {
        el = document.createElement("div");
        el.className = "connector-tag";
        host.appendChild(el);
        this.labels.set(it.mark.key, el);
      }
      const hot = it.mark.key === this.hovered;
      el.classList.toggle("is-chosen", it.mark.state === "chosen");
      el.innerHTML = "<strong></strong><span></span>";
      (el.children[0] as HTMLElement).textContent = it.mark.label;
      (el.children[1] as HTMLElement).textContent = hot ? (it.mark.state === "dim" ? "Already chosen as the other end" : (it.mark.hint ?? "")) : it.mark.state === "chosen" ? "Chosen" : "";
      const p = toScreen(it.group.position);
      el.style.display = p.behind ? "none" : "flex";
      el.style.transform = `translate(${p.x.toFixed(1)}px, ${p.y.toFixed(1)}px) translate(18px, -50%)`;
    }
  }

  /** The pickable connector nearest the pointer (px), if within reach. */
  hit(toScreen: (p: THREE.Vector3) => { x: number; y: number; behind: boolean }, mx: number, my: number): string | null {
    let best: string | null = null;
    let reach = REACH;
    for (const it of this.items) {
      if (it.mark.state === "dim") continue;
      const p = toScreen(it.group.position);
      if (p.behind) continue;
      const d = Math.hypot(p.x - mx, p.y - my);
      if (d < reach) [reach, best] = [d, it.mark.key];
    }
    return best;
  }
}
