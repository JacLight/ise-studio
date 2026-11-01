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

/**
 * A closed solid's geometry (world coordinates) cut by planes, as real
 * geometry for the path tracer, which has no clipping: what each plane cuts
 * away is gone (three.js's rule: kept where n·p + c ≥ 0) and the cut is
 * closed by a cap in the solid's own surface, facing out of the cut. Planes
 * apply one after another, so a box's caps meet at its corners. Null when
 * nothing is left.
 */
export function cutSolid(geometry: THREE.BufferGeometry, planes: THREE.Plane[]): THREE.BufferGeometry | null {
  let g: THREE.BufferGeometry | null = geometry.index ? geometry.toNonIndexed() : geometry;
  for (const plane of planes) {
    if (!g) return null;
    g = cutOnce(g, plane);
  }
  return g;
}

function cutOnce(g: THREE.BufferGeometry, plane: THREE.Plane): THREE.BufferGeometry | null {
  const names = Object.keys(g.attributes);
  const attrs = names.map((n) => g.getAttribute(n) as THREE.BufferAttribute);
  const pos = g.getAttribute("position") as THREE.BufferAttribute;
  const out: number[][] = names.map(() => []);
  const n = pos.count;
  const p = new THREE.Vector3();
  const dist = new Float64Array(n);
  let cut = false;
  let anyKept = false;
  for (let i = 0; i < n; i++) {
    dist[i] = plane.distanceToPoint(p.fromBufferAttribute(pos, i));
    if (dist[i] < -1e-9) cut = true;
    else anyKept = true;
  }
  if (!cut) return g;
  if (!anyKept) return null;
  // A vertex: an original one, or one on the plane along an edge (the same point whichever way round the edge is met).
  type V = { i: number } | { a: number; b: number; t: number };
  const value = (v: V, k: number) => {
    const a = attrs[k];
    const s = a.itemSize;
    if ("i" in v) return Array.from({ length: s }, (_, c) => a.getComponent(v.i, c));
    return Array.from({ length: s }, (_, c) => a.getComponent(v.a, c) * (1 - v.t) + a.getComponent(v.b, c) * v.t);
  };
  const along = (a: number, b: number): V => {
    // Ordered by position, so both triangles sharing the edge find the very same point.
    const pa = new THREE.Vector3().fromBufferAttribute(pos, a);
    const pb = new THREE.Vector3().fromBufferAttribute(pos, b);
    const swap = pa.x > pb.x || (pa.x === pb.x && (pa.y > pb.y || (pa.y === pb.y && pa.z > pb.z)));
    const [x, y] = swap ? [b, a] : [a, b];
    return { a: x, b: y, t: dist[x] / (dist[x] - dist[y]) };
  };
  const segments: [V, V][] = [];
  const emit = (v: V) => names.forEach((_, k) => out[k].push(...value(v, k)));
  for (let t = 0; t + 2 < n; t += 3) {
    const idx = [t, t + 1, t + 2];
    const inside = idx.map((i) => dist[i] >= -1e-9);
    if (inside.every(Boolean)) {
      idx.forEach((i) => emit({ i }));
      continue;
    }
    if (!inside.some(Boolean)) continue;
    // Sutherland–Hodgman against the one plane, keeping the triangle's winding.
    const poly: V[] = [];
    const onPlane: V[] = [];
    for (let k = 0; k < 3; k++) {
      const [a, b] = [idx[k], idx[(k + 1) % 3]];
      if (inside[k]) poly.push({ i: a });
      if (inside[k] !== inside[(k + 1) % 3]) {
        const v = along(a, b);
        poly.push(v);
        onPlane.push(v);
      }
    }
    for (let k = 1; k + 1 < poly.length; k++) [poly[0], poly[k], poly[k + 1]].forEach(emit);
    if (onPlane.length === 2) segments.push([onPlane[0], onPlane[1]]);
  }
  // The cap: the cut edges joined into loops on the plane, outlines with their holes, filled.
  const key = (v: THREE.Vector3) => `${Math.round(v.x * 1e4)},${Math.round(v.y * 1e4)},${Math.round(v.z * 1e4)}`;
  const point = (v: V) => new THREE.Vector3(...(value(v, names.indexOf("position")) as [number, number, number]));
  const next = new Map<string, { to: string; at: THREE.Vector3; v: V }[]>();
  const at = new Map<string, { p: THREE.Vector3; v: V }>();
  for (const [a, b] of segments) {
    const [pa, pb] = [point(a), point(b)];
    const [ka, kb] = [key(pa), key(pb)];
    if (ka === kb) continue;
    at.set(ka, { p: pa, v: a });
    at.set(kb, { p: pb, v: b });
    for (const [x, y, py, vy] of [[ka, kb, pb, b], [kb, ka, pa, a]] as const) {
      const list = next.get(x) ?? [];
      list.push({ to: y, at: py, v: vy });
      next.set(x, list);
    }
  }
  const used = new Set<string>();
  const edgeKey = (x: string, y: string) => (x < y ? `${x}|${y}` : `${y}|${x}`);
  const loops: { p: THREE.Vector3; v: V }[][] = [];
  for (const start of next.keys()) {
    for (const first of next.get(start)!) {
      if (used.has(edgeKey(start, first.to))) continue;
      const loop = [at.get(start)!];
      used.add(edgeKey(start, first.to));
      let cur = first.to;
      let closed = false;
      for (let guard = 0; guard < segments.length + 2; guard++) {
        if (cur === start) {
          closed = true;
          break;
        }
        loop.push(at.get(cur)!);
        const step = next.get(cur)?.find((e) => !used.has(edgeKey(cur, e.to)));
        if (!step) break;
        used.add(edgeKey(cur, step.to));
        cur = step.to;
      }
      if (closed && loop.length >= 3) loops.push(loop);
    }
  }
  if (loops.length) {
    const nrm = plane.normal.clone().normalize();
    const u = new THREE.Vector3().crossVectors(Math.abs(nrm.z) < 0.9 ? new THREE.Vector3(0, 0, 1) : new THREE.Vector3(1, 0, 0), nrm).normalize();
    const w = new THREE.Vector3().crossVectors(nrm, u);
    const flat = loops.map((l) => l.map(({ p }) => new THREE.Vector2(p.dot(u), p.dot(w))));
    const area = (pts: THREE.Vector2[]) => Math.abs(THREE.ShapeUtils.area(pts));
    const inside = (q: THREE.Vector2, pts: THREE.Vector2[]) => {
      let c = false;
      for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
        if (pts[i].y > q.y !== pts[j].y > q.y && q.x < ((pts[j].x - pts[i].x) * (q.y - pts[i].y)) / (pts[j].y - pts[i].y) + pts[i].x) c = !c;
      }
      return c;
    };
    // How deep each loop sits inside the others: even depth an outline, odd a hole in the smallest outline round it.
    const within = flat.map((pts, i) => flat.map((o, j) => (j !== i && area(o) > area(pts) && inside(pts[0], o) ? j : -1)).filter((j) => j >= 0));
    const holes = new Map<number, number[]>();
    flat.forEach((_, i) => {
      if (within[i].length % 2 === 0) holes.set(i, []);
    });
    flat.forEach((_, i) => {
      if (within[i].length % 2 === 1) {
        const owner = within[i].filter((j) => holes.has(j)).sort((a, b) => area(flat[a]) - area(flat[b]))[0];
        if (owner !== undefined) holes.get(owner)!.push(i);
      }
    });
    const outward = nrm.clone().negate();
    const normalAt = names.indexOf("normal");
    for (const [o, hs] of holes) {
      const contour = flat[o];
      const tris = THREE.ShapeUtils.triangulateShape(contour, hs.map((h) => flat[h]));
      const all = [loops[o], ...hs.map((h) => loops[h])].flat();
      for (const tri of tris) {
        const [a, b, c] = tri.map((k) => all[k]);
        const face = new THREE.Vector3().crossVectors(b.p.clone().sub(a.p), c.p.clone().sub(a.p));
        const order = face.dot(outward) >= 0 ? [a, b, c] : [a, c, b];
        for (const v of order)
          names.forEach((_, k) => {
            if (k === normalAt) out[k].push(outward.x, outward.y, outward.z);
            else out[k].push(...value(v.v, k));
          });
      }
    }
  }
  if (!out[names.indexOf("position")].length) return null;
  const r = new THREE.BufferGeometry();
  names.forEach((name, k) => r.setAttribute(name, new THREE.Float32BufferAttribute(out[k], attrs[k].itemSize, attrs[k].normalized)));
  return r;
}
