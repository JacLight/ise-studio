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

export class Display {
  mode: DisplayMode = "shaded";
  /** Clipping planes the edge lines are cut by too (a section). */
  clipping: THREE.Plane[] | null = null;
  private saved = new Map<THREE.MeshStandardMaterial, Saved>();
  private lines = new Map<THREE.Mesh, THREE.LineSegments>();
  private lineMaterial = new THREE.LineBasicMaterial({ color: "#2a2f3a", toneMapped: false, fog: false });
  private xrayLineMaterial = new THREE.LineBasicMaterial({ color: "#3d4a63", transparent: true, opacity: 0.55, depthTest: false, toneMapped: false, fog: false });

  /** Brings every body (and its edges) to the mode; call before each draw. */
  apply(bodies: THREE.Mesh[], materialsOf: (m: THREE.Mesh) => THREE.MeshStandardMaterial[]) {
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
          set("opacity", 0.22);
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
