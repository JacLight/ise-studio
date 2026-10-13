// Draws instances of shared geometry as GPU instances.
//
// Every instance in the view keeps its own THREE.Mesh: picking, hiding,
// selection tints, joints moving it and every tool that looks at a part go
// on working with it as before. Those meshes are not drawn themselves (their
// materials are switched off for drawing, not for picking); each frame this
// layer gathers them by geometry and look into one InstancedMesh per
// (geometry, detail, look), so a thousand M3 screws are one draw call.
//
// Per instance and per frame it also
// - leaves out what is outside the view (frustum) or smaller than a pixel;
// - draws what is small on screen with a simplified mesh (vertex
//   clustering, made once per geometry when first needed): a screw a few
//   pixels tall does not need its thread.

import * as THREE from "three";

/** Below this many pixels across, an instance is not drawn. */
const CULL_PX = 0.75;
/** Below these sizes (pixels across) the coarser meshes are drawn. */
const LOD_PX = [90, 18];
/** Cells per bounding radius for each coarser mesh. */
const LOD_CELLS = [14, 4];

type Lods = { radius: number; levels: (THREE.BufferGeometry | null)[] };
type Batch = { mesh: THREE.InstancedMesh; matrices: Float32Array; count: number; used: boolean };

const sphere = new THREE.Sphere();
const frustum = new THREE.Frustum();
const viewProjection = new THREE.Matrix4();
const eyeAt = new THREE.Vector3();

/** Everything about a material that changes how it draws (display modes and sections change these in place). */
type Look = { values: number[]; planes: THREE.Plane[] | null; key: string };
const looksSeen = new WeakMap<THREE.Material, Look>();
const planeIds = new WeakMap<THREE.Plane[], number>();
let nextPlanes = 1;

const NO_COLOR = { r: -1, g: -1, b: -1 };
/** Pictures (printed labels) by number, so meshes printing different ones never share a draw call. */
const mapIds = new WeakMap<THREE.Texture, number>();
let nextMap = 1;
const mapId = (t: THREE.Texture | null | undefined) => {
  if (!t) return 0;
  let n = mapIds.get(t);
  if (n === undefined) mapIds.set(t, (n = nextMap++));
  return n;
};

/** Writes the look into `out`; true when it differs from what `out` held. */
function lookValues(m: THREE.Material, out: number[]): boolean {
  const s = m as THREE.MeshStandardMaterial;
  const c = s.color ?? NO_COLOR;
  const e = s.emissive ?? NO_COLOR;
  const flags = +s.transparent | (+s.depthWrite << 1) | (+s.colorWrite << 2) | (+(s.wireframe ?? false) << 3) | (+s.depthTest << 4) | (s.side << 5);
  const now = [c.r, c.g, c.b, e.r, e.g, e.b, s.emissiveIntensity ?? 0, s.roughness ?? 0, s.metalness ?? 0, s.opacity, flags, mapId(s.map)];
  let changed = out.length !== now.length;
  for (let i = 0; i < now.length; i++) {
    if (out[i] !== now[i]) {
      out[i] = now[i];
      changed = true;
    }
  }
  return changed;
}

/**
 * A material's look, as a key: meshes that share it can share a draw call.
 * Compared field by field each frame (a key string is made only when it changed).
 */
function lookKey(m: THREE.Material): string {
  let seen = looksSeen.get(m);
  if (!seen) {
    seen = { values: [], planes: null, key: "" };
    looksSeen.set(m, seen);
  }
  const changed = lookValues(m, seen.values);
  if (!changed && seen.planes === m.clippingPlanes && seen.key) return seen.key;
  const planes = m.clippingPlanes;
  let pid = 0;
  if (planes) {
    pid = planeIds.get(planes) ?? nextPlanes++;
    planeIds.set(planes, pid);
  }
  seen.planes = planes;
  seen.key = `${m.type}:${seen.values.join(",")}:${pid}`;
  return seen.key;
}

/**
 * Vertex clustering: vertices snapped to a grid of `cell` and merged, and the
 * triangles that collapse dropped. Keeps the geometry's groups (one material
 * per assembly part). Flat-shaded.
 */
export function simplify(source: THREE.BufferGeometry, cell: number): THREE.BufferGeometry | null {
  const pos = source.getAttribute("position") as THREE.BufferAttribute;
  const index = source.getIndex();
  const n = pos.count;
  const cells = new Map<string, number>();
  const remap = new Int32Array(n);
  const sums: number[] = [];
  const counts: number[] = [];
  for (let i = 0; i < n; i++) {
    const x = pos.getX(i), y = pos.getY(i), z = pos.getZ(i);
    const key = `${Math.round(x / cell)},${Math.round(y / cell)},${Math.round(z / cell)}`;
    let c = cells.get(key);
    if (c === undefined) {
      c = counts.length;
      cells.set(key, c);
      sums.push(0, 0, 0);
      counts.push(0);
    }
    sums[c * 3] += x;
    sums[c * 3 + 1] += y;
    sums[c * 3 + 2] += z;
    counts[c]++;
    remap[i] = c;
  }
  const tri = (t: number) => (index ? index.getX(t) : t);
  const total = index ? index.count : n;
  const groups = source.groups.length ? source.groups : [{ start: 0, count: total, materialIndex: 0 }];
  const out: number[] = [];
  const outGroups: { start: number; count: number; materialIndex: number }[] = [];
  for (const g of groups) {
    const start = out.length / 3;
    const seen = new Set<string>();
    for (let t = g.start; t + 2 < g.start + g.count; t += 3) {
      const a = remap[tri(t)], b = remap[tri(t + 1)], c = remap[tri(t + 2)];
      if (a === b || b === c || a === c) continue;
      const key = [a, b, c].sort((p, q) => p - q).join(",");
      if (seen.has(key)) continue;
      seen.add(key);
      for (const v of [a, b, c]) out.push(sums[v * 3] / counts[v], sums[v * 3 + 1] / counts[v], sums[v * 3 + 2] / counts[v]);
    }
    const count = out.length / 3 - start;
    if (count) outGroups.push({ start, count, materialIndex: g.materialIndex ?? 0 });
  }
  if (!out.length) return null;
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute(out, 3));
  for (const x of outGroups) g.addGroup(x.start, x.count, x.materialIndex);
  g.computeVertexNormals();
  g.computeBoundingSphere();
  return g;
}

export class BatchLayer {
  readonly root = new THREE.Group();
  private lods = new WeakMap<THREE.BufferGeometry, Lods>();
  private batches = new Map<string, Batch>();
  /** One drawing material per look (the meshes' own are switched off for drawing). */
  private looks = new Map<string, THREE.Material>();
  /** Instances drawn last frame, by detail level (for measuring). */
  readonly drawn = [0, 0, 0];
  culled = 0;

  constructor() {
    this.root.name = "instances";
    // Matrices are written each frame in world space.
    this.root.matrixAutoUpdate = false;
  }

  private lodsOf(g: THREE.BufferGeometry): Lods {
    let l = this.lods.get(g);
    if (!l) {
      if (!g.boundingSphere) g.computeBoundingSphere();
      l = { radius: g.boundingSphere!.radius, levels: [g] };
      this.lods.set(g, l);
    }
    return l;
  }

  private level(g: THREE.BufferGeometry, lods: Lods, k: number): THREE.BufferGeometry {
    if (lods.levels[k] === undefined) {
      const coarse = simplify(g, (lods.radius * 2) / LOD_CELLS[k - 1]);
      // Only worth it when it saves most of the triangles.
      const full = (g.index ? g.index.count : g.getAttribute("position").count) / 3;
      lods.levels[k] = coarse && coarse.getAttribute("position").count / 3 < full * 0.6 ? coarse : null;
    }
    return lods.levels[k] ?? (k > 1 ? this.level(g, lods, k - 1) : g);
  }

  private drawingMaterial(m: THREE.Material): THREE.Material {
    const key = lookKey(m);
    let d = this.looks.get(key);
    if (!d) {
      d = m.clone();
      d.visible = true;
      // A section's planes are moved in place: share them, not a copy.
      d.clippingPlanes = m.clippingPlanes;
      this.looks.set(key, d);
    }
    return d;
  }

  /** Takes a mesh out of drawing (it stays pickable); returns whether it was drawn by itself before. */
  static adopt(mesh: THREE.Mesh) {
    for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) m.visible = false;
    // Culled here per instance; the renderer need not test it again.
    mesh.frustumCulled = false;
    // Still a shaded part (a photoreal render takes it, though its own materials are off).
    mesh.userData.batched = true;
  }

  /**
   * Rebuilds the instance buffers from `meshes` (already adopted) as the
   * camera sees them. `aside`: meshes left out this frame.
   */
  update(meshes: Iterable<THREE.Mesh>, camera: THREE.Camera, heightPx: number, wireframe: boolean) {
    viewProjection.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
    frustum.setFromProjectionMatrix(viewProjection);
    // Pixels per mm at unit distance (perspective), or everywhere (a parallel view).
    const ortho = camera as THREE.OrthographicCamera;
    const parallel = ortho.isOrthographicCamera === true;
    const pxPerUnit = parallel
      ? heightPx / ((ortho.top - ortho.bottom) / ortho.zoom)
      : heightPx / 2 / Math.tan(THREE.MathUtils.degToRad((camera as THREE.PerspectiveCamera).fov / 2));
    const eye = eyeAt.setFromMatrixPosition(camera.matrixWorld);
    const near = parallel ? 1 : (camera as THREE.PerspectiveCamera).near;
    for (const b of this.batches.values()) {
      b.count = 0;
      b.used = false;
    }
    this.drawn.fill(0);
    this.culled = 0;
    for (const mesh of meshes) {
      if (!mesh.visible || !visibleUp(mesh)) continue;
      const g = mesh.geometry;
      const lods = this.lodsOf(g);
      sphere.center.copy(g.boundingSphere!.center).applyMatrix4(mesh.matrixWorld);
      sphere.radius = lods.radius * mesh.matrixWorld.getMaxScaleOnAxis();
      if (!frustum.intersectsSphere(sphere)) {
        this.culled++;
        continue;
      }
      const px = parallel ? sphere.radius * 2 * pxPerUnit : ((sphere.radius * 2) / Math.max(sphere.center.distanceTo(eye) - sphere.radius, near)) * pxPerUnit;
      if (px < CULL_PX) {
        this.culled++;
        continue;
      }
      const k = px < LOD_PX[1] ? 2 : px < LOD_PX[0] ? 1 : 0;
      const geometry = k ? this.level(g, lods, k) : g;
      this.drawn[k]++;
      const own = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
      const key = `${geometry.id}|${own.length === 1 ? lookKey(own[0]) : own.map(lookKey).join("/")}|${wireframe}`;
      let b = this.batches.get(key);
      if (!b || b.count * 16 >= b.matrices.length) b = this.grow(key, b, geometry, own);
      b.used = true;
      mesh.matrixWorld.toArray(b.matrices, b.count * 16);
      b.count++;
    }
    for (const [key, b] of this.batches) {
      if (!b.used) {
        // Unused for now: kept (and hidden) unless it holds a lot.
        b.mesh.visible = false;
        if (b.matrices.length > 4096 * 16) this.drop(key, b);
        continue;
      }
      b.mesh.visible = true;
      b.mesh.count = b.count;
      b.mesh.instanceMatrix.needsUpdate = true;
      b.mesh.instanceMatrix.clearUpdateRanges();
      b.mesh.instanceMatrix.addUpdateRange(0, b.count * 16);
    }
  }

  /** A batch with room for more instances (twice as many as before). */
  private grow(key: string, old: Batch | undefined, geometry: THREE.BufferGeometry, own: THREE.Material[]): Batch {
    const capacity = Math.max(16, (old ? old.matrices.length / 16 : 0) * 2);
    const matrices = new Float32Array(capacity * 16);
    if (old) {
      matrices.set(old.matrices.subarray(0, old.count * 16));
      this.root.remove(old.mesh);
      old.mesh.dispose();
    }
    const materials = own.map((m) => this.drawingMaterial(m));
    const mesh = new THREE.InstancedMesh(geometry, materials.length > 1 ? materials : materials[0], capacity);
    mesh.instanceMatrix = new THREE.InstancedBufferAttribute(matrices, 16);
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    // Culling is per instance, above: the whole batch's sphere is not kept.
    mesh.frustumCulled = false;
    mesh.raycast = () => {};
    mesh.matrixAutoUpdate = false;
    this.root.add(mesh);
    const b: Batch = { mesh, matrices, count: old?.count ?? 0, used: true };
    mesh.count = capacity;
    this.batches.set(key, b);
    return b;
  }

  private drop(key: string, b: Batch) {
    this.root.remove(b.mesh);
    b.mesh.dispose();
    this.batches.delete(key);
  }

  /** Draw calls this layer makes (for measuring). */
  get calls() {
    let n = 0;
    for (const b of this.batches.values()) if (b.mesh.visible) n += Array.isArray(b.mesh.material) ? b.mesh.material.length : 1;
    return n;
  }

  dispose() {
    for (const [k, b] of this.batches) this.drop(k, b);
    for (const m of this.looks.values()) m.dispose();
    this.looks.clear();
  }
}

/** Whether every parent up the tree is shown. */
function visibleUp(o: THREE.Object3D) {
  for (let p = o.parent; p; p = p.parent) if (!p.visible) return false;
  return true;
}
