// Faces of any part, for push and pull. A click finds the whole flat face
// under the pointer on the display mesh (every connected triangle in the
// same plane), or the whole smooth curved surface (a cylinder's side, a hole),
// and draws it tinted with an arrow along its outward normal there. Dragging
// the arrow slides both along the normal.

import * as THREE from "three";

const ACCENT = "#6d4ae0";

/** A flat face of a part, in the part's own coordinates (mm). */
export type FlatFace = {
  /** The triangles of the face: x, y, z per vertex. */
  triangles: Float32Array;
  /** Middle of its area, and its outward normal. */
  centroid: [number, number, number];
  normal: [number, number, number];
  area: number;
  /** A curved surface: `centroid` is where it was clicked, `normal` the surface's there. */
  curved?: boolean;
};

/**
 * The smooth surface around triangle `start` (local coordinates): every
 * triangle reached without crossing a sharp edge, nor into facets of another
 * size (a flat face a fillet runs into, or a fillet a shaft runs into: each
 * surface's facets are alike). `at` is the clicked point, where the arrow
 * goes. The engine finds a curved face picked for a study the same way
 * (`smooth_face`), so what is shown is what is held or loaded.
 */
export function curvedFaceAt(geometry: THREE.BufferGeometry, start: number, at: THREE.Vector3, range?: [number, number]): FlatFace | null {
  const pos = geometry.getAttribute("position") as THREE.BufferAttribute;
  const index = geometry.getIndex();
  const tri = (t: number): [number, number, number] => (index ? [index.getX(3 * t), index.getX(3 * t + 1), index.getX(3 * t + 2)] : [3 * t, 3 * t + 1, 3 * t + 2]);
  const v = (i: number) => new THREE.Vector3(pos.getX(i), pos.getY(i), pos.getZ(i));
  const count = (index ? index.count : pos.count) / 3;
  const [lo, hi] = range ? [range[0] / 3, (range[0] + range[1]) / 3] : [0, count];
  const normalOf = (t: number) => {
    const [a, b, c] = tri(t).map(v);
    return new THREE.Vector3().subVectors(b, a).cross(new THREE.Vector3().subVectors(c, a)).normalize();
  };
  // Neighbours across an edge (two shared vertex positions).
  const key = (i: number) => `${pos.getX(i).toFixed(4)},${pos.getY(i).toFixed(4)},${pos.getZ(i).toFixed(4)}`;
  const edges = new Map<string, number[]>();
  for (let t = lo; t < hi; t++) {
    const k = tri(t).map(key);
    for (const [a, b] of [[k[0], k[1]], [k[1], k[2]], [k[2], k[0]]] as [string, string][]) {
      const e = a < b ? `${a}|${b}` : `${b}|${a}`;
      const list = edges.get(e);
      if (list) list.push(t);
      else edges.set(e, [t]);
    }
  }
  const neighbours = (t: number) => {
    const k = tri(t).map(key);
    return ([[k[0], k[1]], [k[1], k[2]], [k[2], k[0]]] as [string, string][]).flatMap(([a, b]) => (edges.get(a < b ? `${a}|${b}` : `${b}|${a}`) ?? []).filter((u) => u !== t));
  };
  // How wide a triangle's flat group (it and its coplanar neighbours) is: area over its box's diagonal.
  const widths = new Map<number, number>();
  const widthOf = (t: number) => {
    const known = widths.get(t);
    if (known !== undefined) return known;
    const group = new Set<number>([t]);
    const stack = [t];
    const box = new THREE.Box3();
    let area = 0;
    while (stack.length) {
      const g = stack.pop()!;
      const [a, b, c] = tri(g).map(v);
      area += new THREE.Vector3().subVectors(b, a).cross(new THREE.Vector3().subVectors(c, a)).length() / 2;
      box.expandByPoint(a).expandByPoint(b).expandByPoint(c);
      const n = normalOf(g);
      for (const u of neighbours(g)) {
        if (!group.has(u) && normalOf(u).dot(n) > 0.9999) {
          group.add(u);
          stack.push(u);
        }
      }
    }
    const diag = box.getSize(new THREE.Vector3()).length();
    const w = diag > 0 ? area / diag : 0;
    for (const g of group) widths.set(g, w);
    return w;
  };
  const w0 = widthOf(start);
  const alike = (u: number) => {
    const w = widthOf(u);
    return w <= 3 * w0 && w0 <= 3 * w;
  };
  const seen = new Set<number>([start]);
  const queue = [start];
  while (queue.length) {
    const t = queue.pop()!;
    const n = normalOf(t);
    for (const u of neighbours(t)) {
      // Facets of one smooth surface meet at under ~20°; a real edge is sharper.
      if (!seen.has(u) && normalOf(u).dot(n) > 0.94 && alike(u)) {
        seen.add(u);
        queue.push(u);
      }
    }
  }
  const faces = [...seen];
  const out = new Float32Array(faces.length * 9);
  let area = 0;
  faces.forEach((t, k) => {
    const [a, b, d] = tri(t).map(v);
    area += new THREE.Vector3().subVectors(b, a).cross(new THREE.Vector3().subVectors(d, a)).length() / 2;
    [a, b, d].forEach((p, j) => out.set([p.x, p.y, p.z], k * 9 + j * 3));
  });
  if (area <= 0) return null;
  // The normal where it was clicked: the clicked facet's, smoothed with its neighbours'.
  const n = normalOf(start);
  return { triangles: out, centroid: [at.x, at.y, at.z], normal: [n.x, n.y, n.z], area, curved: true };
}

/**
 * The flat face around triangle `start` of a mesh (local coordinates), or
 * null when the surface there is curved (its neighbours bend gently away).
 * `range` limits the search to one part of an assembly.
 */
export function flatFaceAt(geometry: THREE.BufferGeometry, start: number, range?: [number, number]): FlatFace | null {
  const pos = geometry.getAttribute("position") as THREE.BufferAttribute;
  const index = geometry.getIndex();
  const tri = (t: number): [number, number, number] => (index ? [index.getX(3 * t), index.getX(3 * t + 1), index.getX(3 * t + 2)] : [3 * t, 3 * t + 1, 3 * t + 2]);
  const v = (i: number) => new THREE.Vector3(pos.getX(i), pos.getY(i), pos.getZ(i));
  const count = (index ? index.count : pos.count) / 3;
  const [lo, hi] = range ? [range[0] / 3, (range[0] + range[1]) / 3] : [0, count];
  const normalOf = (t: number) => {
    const [a, b, c] = tri(t).map(v);
    return new THREE.Vector3().subVectors(b, a).cross(new THREE.Vector3().subVectors(c, a));
  };
  // Triangles sharing a vertex position, so split normals do not break the walk.
  const key = (i: number) => `${pos.getX(i).toFixed(4)},${pos.getY(i).toFixed(4)},${pos.getZ(i).toFixed(4)}`;
  const around = new Map<string, number[]>();
  for (let t = lo; t < hi; t++) {
    for (const i of tri(t)) {
      const k = key(i);
      const list = around.get(k);
      if (list) list.push(t);
      else around.set(k, [t]);
    }
  }
  const n0 = normalOf(start).normalize();
  const plane = n0.dot(v(tri(start)[0]));
  const size = new THREE.Box3().setFromBufferAttribute(pos).getSize(new THREE.Vector3()).length();
  const flatTol = Math.max(1e-4, size * 1e-5);
  const seen = new Set<number>([start]);
  const queue = [start];
  let gentle = 0;
  while (queue.length) {
    const t = queue.pop()!;
    for (const i of tri(t)) {
      for (const u of around.get(key(i)) ?? []) {
        if (seen.has(u)) continue;
        const n = normalOf(u);
        if (n.lengthSq() < 1e-20) continue;
        n.normalize();
        const cos = n.dot(n0);
        if (cos > 0.9999 && Math.abs(n0.dot(v(tri(u)[0])) - plane) < flatTol) {
          seen.add(u);
          queue.push(u);
        } else if (cos > 0.94) {
          gentle++; // a neighbour bending away by under ~20°: a faceted curve, not an edge
        }
      }
    }
  }
  const faces = [...seen];
  // A curved surface shows as a strip whose neighbours bend gently; a flat face meets sharp edges.
  if (gentle > Math.max(2, faces.length * 0.5)) return null;
  const out = new Float32Array(faces.length * 9);
  let area = 0;
  const c = new THREE.Vector3();
  faces.forEach((t, k) => {
    const [a, b, d] = tri(t).map(v);
    const ar = new THREE.Vector3().subVectors(b, a).cross(new THREE.Vector3().subVectors(d, a)).length() / 2;
    area += ar;
    c.addScaledVector(a.clone().add(b).add(d).divideScalar(3), ar);
    [a, b, d].forEach((p, j) => out.set([p.x, p.y, p.z], k * 9 + j * 3));
  });
  if (area <= 0) return null;
  c.divideScalar(area);
  return { triangles: out, centroid: [c.x, c.y, c.z], normal: [n0.x, n0.y, n0.z], area };
}

/** Draws the picked face and its arrow, inside the part's group. */
export class FaceLayer {
  readonly root = new THREE.Group();
  private tint: THREE.Mesh | null = null;
  private arrow = new THREE.Group();
  private hitShaft: THREE.Mesh;
  private face: FlatFace | null = null;
  private offset = 0;

  constructor() {
    const mat = (opacity: number) => new THREE.MeshBasicMaterial({ color: ACCENT, transparent: true, opacity, depthTest: false, depthWrite: false, toneMapped: false, fog: false });
    const shaft = new THREE.Mesh(new THREE.CylinderGeometry(0.018, 0.018, 0.78, 10), mat(0.95));
    shaft.position.y = 0.45;
    const tip = new THREE.Mesh(new THREE.ConeGeometry(0.065, 0.2, 28), mat(1));
    tip.position.y = 0.94;
    this.hitShaft = new THREE.Mesh(new THREE.CylinderGeometry(0.11, 0.11, 1, 8), mat(0));
    this.hitShaft.position.y = 0.58;
    this.arrow.add(shaft, tip, this.hitShaft);
    this.arrow.traverse((o) => (o.renderOrder = 1000));
    this.root.add(this.arrow);
    this.root.visible = false;
  }

  get active() {
    return !!this.face;
  }

  /**
   * Shows a face (or nothing), placed where its part is drawn: at the part's
   * position, and moved as its body is (an exploded or turning part) by
   * `pose`, the body's rigid move within the part.
   */
  show(face: FlatFace | null, at: THREE.Vector3 | null, pose?: THREE.Matrix4 | null) {
    if (this.tint) {
      this.root.remove(this.tint);
      this.tint.geometry.dispose();
      (this.tint.material as THREE.Material).dispose();
      this.tint = null;
    }
    this.face = face;
    this.offset = 0;
    this.root.visible = !!face && !!at;
    if (!face || !at) return;
    this.setPose(at, pose ?? null);
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(face.triangles, 3));
    this.tint = new THREE.Mesh(
      g,
      new THREE.MeshBasicMaterial({ color: ACCENT, transparent: true, opacity: 0.26, depthWrite: false, toneMapped: false, fog: false, side: THREE.DoubleSide, polygonOffset: true, polygonOffsetFactor: -3, polygonOffsetUnits: -3 }),
    );
    this.tint.renderOrder = 950;
    this.root.add(this.tint);
    this.arrow.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), new THREE.Vector3(...face.normal));
    this.place();
  }

  /** Places the face where its part (and its moved body) now is. */
  setPose(at: THREE.Vector3, pose: THREE.Matrix4 | null) {
    const m = new THREE.Matrix4().makeTranslation(at.x, at.y, at.z);
    if (pose) m.multiply(pose);
    m.decompose(this.root.position, this.root.quaternion, new THREE.Vector3());
    this.root.updateMatrixWorld(true);
  }

  /** Slides the face and its arrow along the normal while dragging (mm). */
  slide(distance: number) {
    this.offset = distance;
    this.place();
  }

  /** Keeps the arrow a constant size on screen. */
  scale(s: number) {
    this.arrow.scale.setScalar(s);
  }

  /** The arrow's base in scene coordinates, and the normal (both as drawn). */
  frame(): { origin: THREE.Vector3; normal: THREE.Vector3 } | null {
    if (!this.face) return null;
    const n = new THREE.Vector3(...this.face.normal).applyQuaternion(this.root.quaternion);
    return { origin: new THREE.Vector3(...this.face.centroid).applyQuaternion(this.root.quaternion).add(this.root.position), normal: n };
  }

  /** Whether a ray grabs the arrow or the face itself. */
  grabs(r: THREE.Raycaster) {
    if (!this.face) return false;
    return r.intersectObject(this.hitShaft, false).length > 0 || (!!this.tint && r.intersectObject(this.tint, false).length > 0);
  }

  private place() {
    if (!this.face) return;
    const n = new THREE.Vector3(...this.face.normal);
    const shift = n.clone().multiplyScalar(this.offset);
    if (this.tint) this.tint.position.copy(shift);
    this.arrow.position.copy(new THREE.Vector3(...this.face.centroid).add(shift));
  }
}

/** Faces picked for a shell or draft, and a draft's pull direction and neutral plane (part coordinates, mm). */
export type FacePicture = {
  id: string;
  faces: FlatFace[];
  /** Tint: faces a shell leaves open read as removed; drafted faces as accented. */
  tone: "open" | "draft";
  pull?: { dir: [number, number, number]; origin: [number, number, number]; size: number };
};

/** Draws picked faces tinted, plus (for a draft) the pull arrow and the neutral plane. */
export class PickedFacesLayer {
  readonly root = new THREE.Group();
  private items: THREE.Object3D[] = [];

  update(picture: FacePicture | null, at: THREE.Vector3 | null) {
    for (const o of this.items) {
      this.root.remove(o);
      o.traverse((m) => {
        if (m instanceof THREE.Mesh || m instanceof THREE.Line) {
          m.geometry.dispose();
          (m.material as THREE.Material).dispose();
        }
      });
    }
    this.items = [];
    this.root.visible = !!picture && !!at;
    if (!picture || !at) return;
    this.root.position.copy(at);
    const open = picture.tone === "open";
    for (const f of picture.faces) {
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", new THREE.BufferAttribute(f.triangles, 3));
      const m = new THREE.Mesh(
        g,
        new THREE.MeshBasicMaterial({
          color: open ? "#e0567a" : ACCENT,
          transparent: true,
          opacity: open ? 0.42 : 0.32,
          depthWrite: false,
          toneMapped: false,
          fog: false,
          side: THREE.DoubleSide,
          polygonOffset: true,
          polygonOffsetFactor: -3,
          polygonOffsetUnits: -3,
        }),
      );
      m.renderOrder = 950;
      this.add(m);
    }
    const p = picture.pull;
    if (!p) return;
    const dir = new THREE.Vector3(...p.dir).normalize();
    const origin = new THREE.Vector3(...p.origin);
    const q = new THREE.Quaternion().setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir);
    const s = p.size;
    const h = s * 0.62;
    // The neutral plane: where the drafted faces keep their size.
    const plane = new THREE.Mesh(
      new THREE.PlaneGeometry(2 * h, 2 * h).rotateX(-Math.PI / 2),
      new THREE.MeshBasicMaterial({ color: ACCENT, transparent: true, opacity: 0.08, depthWrite: false, toneMapped: false, fog: false, side: THREE.DoubleSide }),
    );
    plane.renderOrder = 940;
    const outline = new THREE.LineLoop(
      new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(-h, 0, -h), new THREE.Vector3(h, 0, -h), new THREE.Vector3(h, 0, h), new THREE.Vector3(-h, 0, h)]),
      new THREE.LineBasicMaterial({ color: ACCENT, transparent: true, opacity: 0.6, depthTest: false, toneMapped: false, fog: false }),
    );
    outline.renderOrder = 941;
    // The pull: the way the mould opens.
    const mat = () => new THREE.MeshBasicMaterial({ color: ACCENT, transparent: true, opacity: 0.95, depthTest: false, depthWrite: false, toneMapped: false, fog: false });
    const len = s * 0.55;
    const shaft = new THREE.Mesh(new THREE.CylinderGeometry(s * 0.009, s * 0.009, len, 10), mat());
    shaft.position.y = len / 2;
    const tip = new THREE.Mesh(new THREE.ConeGeometry(s * 0.035, s * 0.1, 28), mat());
    tip.position.y = len + s * 0.05;
    shaft.renderOrder = tip.renderOrder = 1000;
    const frame = new THREE.Group();
    frame.add(plane, outline, shaft, tip);
    frame.quaternion.copy(q);
    frame.position.copy(origin);
    this.add(frame);
  }

  private add(o: THREE.Object3D) {
    this.items.push(o);
    this.root.add(o);
  }
}
