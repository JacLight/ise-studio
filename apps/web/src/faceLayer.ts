// Flat faces of any part, for push and pull. A click finds the whole flat
// face under the pointer on the display mesh (every connected triangle in the
// same plane), refuses curved surfaces, and draws it tinted with an arrow
// along its outward normal. Dragging the arrow slides both along the normal.

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
};

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

  /** Shows a face (or nothing), placed where its part is drawn. */
  show(face: FlatFace | null, at: THREE.Vector3 | null) {
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
    this.root.position.copy(at);
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

  /** Slides the face and its arrow along the normal while dragging (mm). */
  slide(distance: number) {
    this.offset = distance;
    this.place();
  }

  /** Keeps the arrow a constant size on screen. */
  scale(s: number) {
    this.arrow.scale.setScalar(s);
  }

  /** The arrow's base in scene coordinates, and the normal. */
  frame(): { origin: THREE.Vector3; normal: THREE.Vector3 } | null {
    if (!this.face) return null;
    const n = new THREE.Vector3(...this.face.normal);
    return { origin: new THREE.Vector3(...this.face.centroid).add(this.root.position), normal: n };
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
