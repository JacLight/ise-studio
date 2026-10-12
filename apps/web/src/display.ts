// Display modes for the model: shaded, shaded with edges, wireframe (edges
// only), hidden line (edges, with what is behind a face hidden) and x-ray
// (see-through bodies, edges on top). The look each part was given is left
// alone in shaded mode; the other modes adjust its materials each frame (so
// a part that arrives or is repainted follows the mode) and put back exactly
// what they changed when the mode goes back to shaded.

import * as THREE from "three";

export type DisplayMode = "shaded" | "edges" | "wireframe" | "hidden" | "xray";

export const DISPLAY_MODES: { mode: DisplayMode; label: string }[] = [
  { mode: "shaded", label: "Shaded" },
  { mode: "edges", label: "Shaded with edges" },
  { mode: "wireframe", label: "Wireframe" },
  { mode: "hidden", label: "Hidden line" },
  { mode: "xray", label: "X-ray" },
];

type Saved = { color: THREE.Color; emissive: THREE.Color; emissiveIntensity: number; transparent: boolean; opacity: number; depthWrite: boolean; colorWrite: boolean };

const EDGE_ANGLE = 30;
const edgeGeometries = new WeakMap<THREE.BufferGeometry, THREE.EdgesGeometry>();
const edgesOf = (g: THREE.BufferGeometry) => {
  let e = edgeGeometries.get(g);
  if (!e) edgeGeometries.set(g, (e = new THREE.EdgesGeometry(g, EDGE_ANGLE)));
  return e;
};

/** Smooth edges of a geometry (between faces bent less than EDGE_ANGLE): ends and the two faces' normals, for silhouettes. */
type Smooth = { data: Float32Array; count: number };
const smoothEdges = new WeakMap<THREE.BufferGeometry, Smooth>();
function smoothOf(g: THREE.BufferGeometry): Smooth {
  const known = smoothEdges.get(g);
  if (known) return known;
  const pos = g.getAttribute("position") as THREE.BufferAttribute;
  const index = g.getIndex();
  const n = index ? index.count : pos.count;
  const ids = new Map<string, number>();
  const weld = (i: number) => {
    const k = `${Math.round(pos.getX(i) * 1000)},${Math.round(pos.getY(i) * 1000)},${Math.round(pos.getZ(i) * 1000)}`;
    let id = ids.get(k);
    if (id === undefined) ids.set(k, (id = i));
    return id;
  };
  const corner = (k: number) => weld(index ? index.getX(k) : k);
  const a = new THREE.Vector3();
  const b = new THREE.Vector3();
  const c = new THREE.Vector3();
  const normals: THREE.Vector3[] = [];
  const edges = new Map<string, number[]>();
  for (let t = 0; t < n / 3; t++) {
    const v = [corner(3 * t), corner(3 * t + 1), corner(3 * t + 2)];
    a.fromBufferAttribute(pos, v[0]);
    b.fromBufferAttribute(pos, v[1]);
    c.fromBufferAttribute(pos, v[2]);
    normals.push(b.clone().sub(a).cross(c.clone().sub(a)).normalize());
    for (let e = 0; e < 3; e++) {
      const [p, q] = [v[e], v[(e + 1) % 3]];
      const key = p < q ? `${p}:${q}` : `${q}:${p}`;
      const list = edges.get(key);
      if (list) list.push(t);
      else edges.set(key, [t]);
    }
  }
  const cos = Math.cos(THREE.MathUtils.degToRad(EDGE_ANGLE));
  const out: number[] = [];
  for (const [key, ts] of edges) {
    if (ts.length !== 2) continue;
    const [n1, n2] = [normals[ts[0]], normals[ts[1]]];
    if (n1.dot(n2) < cos) continue; // a crease: drawn as a feature edge already
    const [p, q] = key.split(":").map(Number);
    out.push(pos.getX(p), pos.getY(p), pos.getZ(p), pos.getX(q), pos.getY(q), pos.getZ(q), n1.x, n1.y, n1.z, n2.x, n2.y, n2.z);
  }
  const smooth = { data: new Float32Array(out), count: out.length / 12 };
  smoothEdges.set(g, smooth);
  return smooth;
}

/** The outline of a curved body as seen from the camera: smooth edges between a face turned towards it and one turned away. */
function silhouette(line: THREE.LineSegments, body: THREE.Mesh, camera: THREE.Camera) {
  const smooth = smoothOf(body.geometry);
  if (!smooth.count || smooth.count > 300000) return;
  const key = camera.matrixWorld.elements.join(",") + body.matrixWorld.elements.join(",");
  if (line.userData.key === key) return;
  line.userData.key = key;
  const inverse = new THREE.Matrix4().copy(body.matrixWorld).invert();
  const ortho = (camera as THREE.OrthographicCamera).isOrthographicCamera === true;
  const eye = new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld).applyMatrix4(inverse);
  // Towards the eye, for a parallel view.
  const back = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 2).transformDirection(inverse);
  const geometry = line.geometry;
  let attr = geometry.getAttribute("position") as THREE.BufferAttribute | undefined;
  if (!attr || attr.count < smooth.count * 2) {
    attr = new THREE.BufferAttribute(new Float32Array(smooth.count * 6), 3);
    attr.setUsage(THREE.DynamicDrawUsage);
    geometry.setAttribute("position", attr);
  }
  const d = smooth.data;
  const out = attr.array as Float32Array;
  let k = 0;
  for (let i = 0; i < smooth.count; i++) {
    const o = i * 12;
    const tx = ortho ? back.x : eye.x - d[o];
    const ty = ortho ? back.y : eye.y - d[o + 1];
    const tz = ortho ? back.z : eye.z - d[o + 2];
    const f1 = d[o + 6] * tx + d[o + 7] * ty + d[o + 8] * tz;
    const f2 = d[o + 9] * tx + d[o + 10] * ty + d[o + 11] * tz;
    if (f1 > 0 === f2 > 0) continue;
    for (let j = 0; j < 6; j++) out[k++] = d[o + j];
  }
  attr.needsUpdate = true;
  geometry.setDrawRange(0, k / 3);
}

export class Display {
  mode: DisplayMode = "shaded";
  /** Clipping planes the edge lines are cut by too (a section). */
  clipping: THREE.Plane[] | null = null;
  private saved = new Map<THREE.MeshStandardMaterial, Saved>();
  private lines = new Map<THREE.Mesh, THREE.LineSegments>();
  private outlines = new Map<THREE.Mesh, THREE.LineSegments>();
  private lineMaterial = new THREE.LineBasicMaterial({ color: "#2a2f3a", toneMapped: false, fog: false });
  private xrayLineMaterial = new THREE.LineBasicMaterial({ color: "#3d4a63", transparent: true, opacity: 0.55, depthTest: false, toneMapped: false, fog: false });

  /** Brings every body (and its edges) to the mode; call before each draw. */
  apply(bodies: THREE.Mesh[], materialsOf: (m: THREE.Mesh) => THREE.MeshStandardMaterial[], camera: THREE.Camera) {
    const mode = this.mode;
    const edges = mode !== "shaded";
    // Edge lines: one per body, sharing the body's geometry's edges.
    const seen = new Set<THREE.Mesh>();
    if (edges) {
      const material = mode === "xray" ? this.xrayLineMaterial : this.lineMaterial;
      material.clippingPlanes = this.clipping;
      for (const body of bodies) {
        seen.add(body);
        let line = this.lines.get(body);
        if (!line || line.parent !== body || line.userData.geometry !== body.geometry) {
          if (line) line.removeFromParent();
          line = new THREE.LineSegments(edgesOf(body.geometry), material);
          line.userData.geometry = body.geometry;
          line.raycast = () => {}; // never picked
          line.renderOrder = mode === "xray" ? 960 : 1;
          body.add(line);
          this.lines.set(body, line);
        }
        line.material = material;
        line.renderOrder = mode === "xray" ? 960 : 1;
      }
    }
    for (const [body, line] of this.lines) {
      if (seen.has(body)) continue;
      line.removeFromParent();
      this.lines.delete(body);
    }
    // Outlines of curved faces where no shading shows them.
    const outlined = new Set<THREE.Mesh>();
    if (mode === "wireframe" || mode === "hidden" || mode === "xray") {
      const material = mode === "xray" ? this.xrayLineMaterial : this.lineMaterial;
      for (const body of bodies) {
        let line = this.outlines.get(body);
        if (!line || line.parent !== body) {
          line?.removeFromParent();
          line = new THREE.LineSegments(new THREE.BufferGeometry(), material);
          line.raycast = () => {};
          line.frustumCulled = false;
          body.add(line);
          this.outlines.set(body, line);
        }
        if (line.userData.geometry !== body.geometry) {
          line.userData.geometry = body.geometry;
          line.userData.key = null;
        }
        line.material = material;
        line.renderOrder = mode === "xray" ? 960 : 1;
        silhouette(line, body, camera);
        outlined.add(body);
      }
    }
    for (const [body, line] of this.outlines) {
      if (outlined.has(body)) continue;
      line.removeFromParent();
      line.geometry.dispose();
      this.outlines.delete(body);
    }
    if (mode === "shaded" || mode === "edges") {
      this.restore();
      return;
    }
    for (const body of bodies)
      for (const m of materialsOf(body)) {
        if (!this.saved.has(m))
          this.saved.set(m, { color: m.color.clone(), emissive: m.emissive.clone(), emissiveIntensity: m.emissiveIntensity, transparent: m.transparent, opacity: m.opacity, depthWrite: m.depthWrite, colorWrite: m.colorWrite });
        let changed = false;
        const set = <K extends keyof THREE.MeshStandardMaterial>(k: K, v: THREE.MeshStandardMaterial[K]) => {
          if (m[k] !== v) {
            (m[k] as unknown) = v;
            changed = true;
          }
        };
        if (mode === "wireframe") {
          // Depth only where the faces are is not drawn: every edge shows, front and back.
          set("colorWrite", false);
          set("depthWrite", false);
          set("transparent", false);
        } else if (mode === "hidden") {
          // Faces drawn in the background colour: edges behind them are hidden.
          set("colorWrite", true);
          set("depthWrite", true);
          set("transparent", false);
          set("opacity", 1);
          if (!m.color.equals(WHITE)) m.color.copy(WHITE);
          m.emissive.copy(WHITE);
          m.emissiveIntensity = 1;
        } else {
          set("colorWrite", true);
          set("transparent", true);
          set("opacity", 0.32);
          set("depthWrite", false);
        }
        if (changed) m.needsUpdate = true;
      }
  }

  /** Puts back every material this changed. */
  restore() {
    for (const [m, s] of this.saved) {
      m.color.copy(s.color);
      m.emissive.copy(s.emissive);
      m.emissiveIntensity = s.emissiveIntensity;
      m.transparent = s.transparent;
      m.opacity = s.opacity;
      m.depthWrite = s.depthWrite;
      m.colorWrite = s.colorWrite;
      m.needsUpdate = true;
    }
    this.saved.clear();
  }

  /** Bodies hidden in this mode's pass for shadows (wireframe and hidden line cast none). */
  get castsShadows() {
    return this.mode === "shaded" || this.mode === "edges";
  }
}

const WHITE = new THREE.Color("#ffffff");
