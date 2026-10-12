// Section views: the model cut by one or more planes (on X, Y, Z, or any
// plane through a picked face), with the cut faces filled and hatched like a
// drawing's section, not left as hollow shells.
//
// Caps use the stencil buffer. For each plane, every body is drawn (colour
// off) with its back faces counting up and its front faces counting down,
// cut by that plane only: where the count is not zero the ray at that pixel
// enters a solid behind the plane, so the plane there is inside material. A
// hatched quad on the plane is drawn there, cut by the other planes (so a
// section box's caps meet at its corners), and tested against depth so what
// stands in front still hides it.

import * as THREE from "three";

export type Axis = "x" | "y" | "z";

/** One cut: material on the side its normal points away from is removed. */
export type SectionPlane = {
  id: string;
  /** Unit normal of the kept side's boundary, pointing into what is cut away. */
  normal: [number, number, number];
  /** Where it passes: a point on it (mm). */
  at: [number, number, number];
  /** Keep the other side instead. */
  flip?: boolean;
};

/** A plane on an axis at an offset (mm): keeps what is below the offset (flipped: above). */
export function axisPlane(axis: Axis, offset: number, flip = false): SectionPlane {
  const n: [number, number, number] = axis === "x" ? [1, 0, 0] : axis === "y" ? [0, 1, 0] : [0, 0, 1];
  const at: [number, number, number] = [0, 0, 0];
  at["xyz".indexOf(axis)] = offset;
  return { id: axis, normal: n, at, flip };
}

/** The three.js clipping plane (it keeps points where n·p + c ≥ 0). */
export function clipPlane(p: SectionPlane): THREE.Plane {
  const n = new THREE.Vector3(...p.normal).normalize();
  // Keep the side the normal points away from: the plane's normal is −n.
  const keep = p.flip ? n : n.clone().negate();
  return new THREE.Plane().setFromNormalAndCoplanarPoint(keep, new THREE.Vector3(...p.at));
}

/** Whether a point survives every cut. */
export function kept(planes: SectionPlane[], point: [number, number, number]) {
  const v = new THREE.Vector3(...point);
  return planes.every((p) => clipPlane(p).distanceToPoint(v) >= -1e-9);
}

let hatch: THREE.CanvasTexture | null = null;
function hatchTexture() {
  if (hatch) return hatch;
  const c = document.createElement("canvas");
  c.width = c.height = 64;
  const g = c.getContext("2d")!;
  g.fillStyle = "#f0d9b5";
  g.fillRect(0, 0, 64, 64);
  g.strokeStyle = "#a0662a";
  g.lineWidth = 3;
  // 45° lines that tile seamlessly.
  for (const o of [-64, 0, 64]) {
    g.beginPath();
    g.moveTo(o, 64);
    g.lineTo(o + 64, 0);
    g.stroke();
  }
  hatch = new THREE.CanvasTexture(c);
  hatch.wrapS = hatch.wrapT = THREE.RepeatWrapping;
  hatch.colorSpace = THREE.SRGBColorSpace;
  hatch.anisotropy = 4;
  return hatch;
}

const stencilMaterial = (side: THREE.Side, op: THREE.StencilOp) =>
  new THREE.MeshBasicMaterial({
    side,
    colorWrite: false,
    depthWrite: false,
    depthTest: false,
    stencilWrite: true,
    stencilFunc: THREE.AlwaysStencilFunc,
    stencilFail: op,
    stencilZFail: op,
    stencilZPass: op,
  });

export class Section {
  planes: SectionPlane[] = [];
  /** Fill and hatch the cut faces. */
  capped = true;
  /** The clipping planes for the bodies' materials (shared array: edited in place). */
  readonly clipping: THREE.Plane[] = [];
  private proxies = new Map<THREE.Mesh, THREE.Mesh>();
  private stencilScene = new THREE.Scene();
  private capScene = new THREE.Scene();
  private back = stencilMaterial(THREE.BackSide, THREE.IncrementWrapStencilOp);
  private front = stencilMaterial(THREE.FrontSide, THREE.DecrementWrapStencilOp);
  private caps: THREE.Mesh[] = [];
  private touched = new Set<THREE.Material>();

  get active() {
    return this.planes.length > 0;
  }

  set(planes: SectionPlane[]) {
    this.planes = planes;
    this.clipping.length = 0;
    for (const p of planes) this.clipping.push(clipPlane(p));
  }

  /** Cuts the bodies' materials (and uncuts them when the section ends). */
  clip(bodies: THREE.Mesh[], materialsOf: (m: THREE.Mesh) => THREE.Material[]) {
    const want = this.active ? this.clipping : null;
    for (const body of bodies)
      for (const m of materialsOf(body)) {
        if (m.clippingPlanes === want) continue;
        m.clippingPlanes = want;
        m.clipShadows = !!want; // what is cut away casts no shadow
        m.needsUpdate = true;
        if (want) this.touched.add(m);
      }
    if (!want) {
      for (const m of this.touched) {
        if (m.clippingPlanes) {
          m.clippingPlanes = null;
          m.needsUpdate = true;
        }
      }
      this.touched.clear();
    }
  }

  /**
   * Draws the caps over a frame already drawn (call right after it, with
   * autoClear off). `bodies` are the visible solids; `extent` sizes the caps.
   */
  renderCaps(renderer: THREE.WebGLRenderer, camera: THREE.Camera, bodies: THREE.Mesh[], extent: THREE.Sphere) {
    if (!this.active || !this.capped) return;
    // Proxies: the bodies' geometry where the bodies are, for the stencil count.
    const live = new Set<THREE.Mesh>();
    for (const body of bodies) {
      if (!isShown(body)) continue;
      live.add(body);
      let proxy = this.proxies.get(body);
      if (!proxy || proxy.geometry !== body.geometry) {
        if (proxy) proxy.removeFromParent();
        if (body instanceof THREE.InstancedMesh) {
          const many = new THREE.InstancedMesh(body.geometry, undefined, body.count);
          many.instanceMatrix = body.instanceMatrix;
          proxy = many;
        } else proxy = new THREE.Mesh(body.geometry);
        proxy.matrixAutoUpdate = false;
        proxy.matrixWorldAutoUpdate = false;
        this.stencilScene.add(proxy);
        this.proxies.set(body, proxy);
      }
      proxy.matrixWorld.copy(body.matrixWorld);
      if (body instanceof THREE.InstancedMesh) (proxy as THREE.InstancedMesh).count = body.count;
    }
    for (const [body, proxy] of this.proxies)
      if (!live.has(body)) {
        proxy.removeFromParent();
        this.proxies.delete(body);
      }
    if (!live.size) return;
    while (this.caps.length < this.planes.length) {
      const cap = new THREE.Mesh(
        new THREE.PlaneGeometry(1, 1),
        new THREE.MeshBasicMaterial({
          map: hatchTexture(),
          side: THREE.DoubleSide,
          stencilWrite: true,
          stencilRef: 0,
          stencilFunc: THREE.NotEqualStencilFunc,
          stencilFail: THREE.ReplaceStencilOp,
          stencilZFail: THREE.ReplaceStencilOp,
          stencilZPass: THREE.ReplaceStencilOp,
          toneMapped: false,
          fog: false,
          polygonOffset: true,
          polygonOffsetFactor: -1,
          polygonOffsetUnits: -1,
        }),
      );
      cap.matrixAutoUpdate = false;
      this.caps.push(cap);
    }
    const autoClear = renderer.autoClear;
    renderer.autoClear = false;
    const size = extent.radius * 4 + 10;
    const z = new THREE.Vector3(0, 0, 1);
    this.planes.forEach((p, i) => {
      const plane = this.clipping[i];
      const others = this.clipping.filter((_, k) => k !== i);
      renderer.clearStencil();
      // Count with this plane's cut only.
      this.back.clippingPlanes = this.front.clippingPlanes = [plane];
      this.stencilScene.overrideMaterial = this.back;
      renderer.render(this.stencilScene, camera);
      this.stencilScene.overrideMaterial = this.front;
      renderer.render(this.stencilScene, camera);
      // The cap: a square on the plane, centred where the model is.
      const cap = this.caps[i];
      const material = cap.material as THREE.MeshBasicMaterial;
      material.clippingPlanes = others;
      // Hatch lines a few millimetres apart on a part, wider on a building.
      const tile = Math.max(extent.radius / 40, 1.5);
      material.map!.repeat.set(size / tile, size / tile);
      const centre = plane.projectPoint(extent.center, new THREE.Vector3());
      const q = new THREE.Quaternion().setFromUnitVectors(z, plane.normal);
      cap.matrixWorld.compose(centre, q, new THREE.Vector3(size, size, 1));
      cap.matrix.copy(cap.matrixWorld);
      this.capScene.clear();
      this.capScene.add(cap);
      renderer.render(this.capScene, camera);
      void p;
    });
    renderer.autoClear = autoClear;
  }

  dispose() {
    for (const proxy of this.proxies.values()) proxy.removeFromParent();
    this.proxies.clear();
    for (const c of this.caps) {
      c.geometry.dispose();
      (c.material as THREE.Material).dispose();
    }
    this.caps = [];
  }
}

/** Shown on screen: it and everything above it visible. */
function isShown(o: THREE.Object3D) {
  for (let p: THREE.Object3D | null = o; p; p = p.parent) if (!p.visible) return false;
  return true;
}
