// Photo skins: the product's own photographs laid back onto the model, as a
// visual-effects artist projects a plate onto geometry. Each calibrated
// canvas (a reference photo placed square on to a side of the model, see
// canvasLayer.ts) is projected straight along its view onto the faces that
// look toward that camera and that it can see (nothing of the model in
// front of them), so every line, groove, logo, button and print of the real
// product is on the model exactly as photographed. Faces no photo sees keep
// the modelled material. The photo's backdrop is pushed out of the way
// first (the product's own colours grown into it), so a face at the
// silhouette's edge never picks up the white sweep behind the product.

import * as THREE from "three";
import { MeshBVH } from "three-mesh-bvh";
import { type CanvasData, apply, toPixel, worldToPlane } from "./canvasLayer";
import { projectUrl } from "./engine";

/** A photo to skin the model with: a canvas of the project, by its data. */
export type Skin = { name: string; canvas: CanvasData };

/** A skin ready to project: its picture (backdrop filled), where it looks from, and its pixel mapping. */
export type PreparedSkin = { name: string; canvas: CanvasData; texture: THREE.Texture; toward: THREE.Vector3; width: number; height: number; pixel: (w: THREE.Vector3) => [number, number] };

/** A face takes the photo that looks most squarely at it, if at least this squarely (cosine). */
const FACING = 0.3;

const prepared = new Map<string, Promise<PreparedSkin | null>>();

/** Loads and prepares the skins (cached by picture and placement). */
export function prepareSkins(skins: Skin[] | undefined): Promise<PreparedSkin[]> {
  if (!skins?.length) return Promise.resolve([]);
  return Promise.all(
    skins.map((s) => {
      const key = JSON.stringify(s.canvas);
      let p = prepared.get(key);
      if (!p) prepared.set(key, (p = prepare(s).catch((e) => (console.warn(`skin ${s.name}: ${e}`), null))));
      return p;
    }),
  ).then((list) => list.filter((s): s is PreparedSkin => !!s));
}

async function prepare(s: Skin): Promise<PreparedSkin> {
  const c = s.canvas;
  const img = new Image();
  img.crossOrigin = "anonymous";
  await new Promise<void>((done, fail) => {
    img.onload = () => done();
    img.onerror = () => fail(new Error("picture not read"));
    img.src = projectUrl(`/images/${c.image}`);
  });
  const [w, h] = [img.naturalWidth, img.naturalHeight];
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const g = canvas.getContext("2d", { willReadFrequently: true })!;
  g.drawImage(img, 0, 0);
  const data = g.getImageData(0, 0, w, h);
  fillBackdrop(data, c);
  g.putImageData(data, 0, 0);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = 8;
  texture.flipY = false;
  // Toward the camera that took it: the plane's normal (planes are named for where the camera stands).
  const x = new THREE.Vector3(...c.plane.x), y = new THREE.Vector3(...c.plane.y);
  const toward = new THREE.Vector3().crossVectors(x, y).normalize();
  const m = toPixel(c);
  const pixel = (p: THREE.Vector3) => apply(m, worldToPlane(c, [p.x, p.y, p.z]));
  return { name: s.name, canvas: c, texture, toward, width: w, height: h, pixel };
}

/**
 * Grows the product's colours over the photo's backdrop (near-white, or the
 * canvas's own background colour): rings of backdrop pixels take the mean of
 * their product neighbours, 40 rings deep, then the rest the mean product
 * colour. A face whose projection lands a pixel or two off the silhouette
 * then shows the product's edge colour, not a white fringe.
 */
function fillBackdrop(img: ImageData, c: CanvasData) {
  const { width: w, height: h, data } = img;
  const bg = c.background ? c.background.map((v) => (v <= 1 ? v * 255 : v)) : [255, 255, 255];
  const tol = (c.threshold ?? 0.12) * 255 * 1.5;
  const known = new Uint8Array(w * h);
  let sum = [0, 0, 0], count = 0;
  for (let i = 0; i < w * h; i++) {
    const d = Math.max(Math.abs(data[i * 4] - bg[0]), Math.abs(data[i * 4 + 1] - bg[1]), Math.abs(data[i * 4 + 2] - bg[2]));
    if (d > tol) {
      known[i] = 1;
      sum = [sum[0] + data[i * 4], sum[1] + data[i * 4 + 1], sum[2] + data[i * 4 + 2]];
      count++;
    }
  }
  let edge: number[] = [];
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) if (!known[y * w + x] && near(known, w, h, x, y)) edge.push(y * w + x);
  for (let ring = 0; ring < 40 && edge.length; ring++) {
    const filled: number[] = [];
    for (const i of edge) {
      if (known[i]) continue;
      const x = i % w, y = (i / w) | 0;
      let r = 0, gg = 0, b = 0, n = 0;
      for (let dy = -1; dy <= 1; dy++)
        for (let dx = -1; dx <= 1; dx++) {
          const xx = x + dx, yy = y + dy;
          if (xx < 0 || yy < 0 || xx >= w || yy >= h) continue;
          const j = yy * w + xx;
          if (known[j] !== 1) continue;
          r += data[j * 4];
          gg += data[j * 4 + 1];
          b += data[j * 4 + 2];
          n++;
        }
      if (!n) continue;
      data[i * 4] = r / n;
      data[i * 4 + 1] = gg / n;
      data[i * 4 + 2] = b / n;
      filled.push(i);
    }
    for (const i of filled) known[i] = 2;
    for (const i of filled) known[i] = 1;
    const next = new Set<number>();
    for (const i of filled) {
      const x = i % w, y = (i / w) | 0;
      for (let dy = -1; dy <= 1; dy++)
        for (let dx = -1; dx <= 1; dx++) {
          const xx = x + dx, yy = y + dy;
          if (xx >= 0 && yy >= 0 && xx < w && yy < h && !known[yy * w + xx]) next.add(yy * w + xx);
        }
    }
    edge = [...next];
  }
  const mean = count ? sum.map((v) => v / count) : [128, 128, 128];
  for (let i = 0; i < w * h; i++)
    if (!known[i]) {
      data[i * 4] = mean[0];
      data[i * 4 + 1] = mean[1];
      data[i * 4 + 2] = mean[2];
    }
}

function near(known: Uint8Array, w: number, h: number, x: number, y: number) {
  for (let dy = -1; dy <= 1; dy++)
    for (let dx = -1; dx <= 1; dx++) {
      const xx = x + dx, yy = y + dy;
      if (xx >= 0 && yy >= 0 && xx < w && yy < h && known[yy * w + xx]) return true;
    }
  return false;
}

/** What the skins can see: the whole shown model in world space, for occlusion rays. */
export type SkinWorld = { bvh: MeshBVH; size: number };

export function skinWorld(meshes: { geometry: THREE.BufferGeometry; matrix: THREE.Matrix4 }[]): SkinWorld | null {
  const parts: Float32Array[] = [];
  let total = 0;
  const v = new THREE.Vector3();
  for (const { geometry, matrix } of meshes) {
    const pos = geometry.getAttribute("position");
    const index = geometry.getIndex();
    const n = index ? index.count : pos.count;
    const out = new Float32Array(n * 3);
    for (let i = 0; i < n; i++) {
      v.fromBufferAttribute(pos, index ? index.getX(i) : i).applyMatrix4(matrix);
      out[i * 3] = v.x;
      out[i * 3 + 1] = v.y;
      out[i * 3 + 2] = v.z;
    }
    parts.push(out);
    total += n;
  }
  if (!total) return null;
  const all = new Float32Array(total * 3);
  let at = 0;
  for (const p of parts) {
    all.set(p, at);
    at += p.length;
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(all, 3));
  g.computeBoundingBox();
  const size = g.boundingBox!.getSize(new THREE.Vector3()).length();
  return { bvh: new MeshBVH(g), size };
}

/**
 * The geometry split by which photo skins each triangle: `[geometry, skin
 * index or -1][]`, each with texture coordinates into its photo. A triangle
 * takes the photo that faces it most squarely, if the photo sees it: a ray
 * from the triangle toward that camera meets nothing else of the model.
 */
export function skinPieces(geometry: THREE.BufferGeometry, matrix: THREE.Matrix4, skins: PreparedSkin[], world: SkinWorld | null): [THREE.BufferGeometry, number][] {
  const g = geometry.index ? geometry.toNonIndexed() : geometry.clone();
  const pos = g.getAttribute("position");
  const tris = pos.count / 3;
  const a = new THREE.Vector3(), b = new THREE.Vector3(), c = new THREE.Vector3(), n = new THREE.Vector3(), mid = new THREE.Vector3();
  const choice = new Int32Array(tris).fill(-1);
  const ray = new THREE.Ray();
  const eps = world ? world.size * 2e-4 : 0.01;
  for (let t = 0; t < tris; t++) {
    a.fromBufferAttribute(pos, t * 3).applyMatrix4(matrix);
    b.fromBufferAttribute(pos, t * 3 + 1).applyMatrix4(matrix);
    c.fromBufferAttribute(pos, t * 3 + 2).applyMatrix4(matrix);
    n.crossVectors(b.clone().sub(a), c.clone().sub(a));
    if (n.lengthSq() < 1e-18) continue;
    n.normalize();
    let best = -1, bestDot = FACING;
    for (let k = 0; k < skins.length; k++) {
      const d = n.dot(skins[k].toward);
      if (d > bestDot) [best, bestDot] = [k, d];
    }
    if (best < 0) continue;
    if (world) {
      // Seen by that camera: nothing of the model between the face and it.
      mid.copy(a).add(b).add(c).multiplyScalar(1 / 3);
      ray.origin.copy(mid).addScaledVector(skins[best].toward, eps * 4);
      ray.direction.copy(skins[best].toward);
      if (world.bvh.raycastFirst(ray, THREE.DoubleSide)) continue;
    }
    choice[t] = best;
  }
  // Texture coordinates: each vertex of a skinned triangle in its photo.
  const uv = new Float32Array(pos.count * 2);
  const w = new THREE.Vector3();
  for (let t = 0; t < tris; t++) {
    const k = choice[t];
    if (k < 0) continue;
    const s = skins[k];
    for (let j = 0; j < 3; j++) {
      w.fromBufferAttribute(pos, t * 3 + j).applyMatrix4(matrix);
      const [px, py] = s.pixel(w);
      uv[(t * 3 + j) * 2] = px / s.width;
      uv[(t * 3 + j) * 2 + 1] = py / s.height;
    }
  }
  g.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
  // One piece per photo (and one for what none sees), sharing the vertex data.
  const out: [THREE.BufferGeometry, number][] = [];
  for (let k = -1; k < skins.length; k++) {
    const idx: number[] = [];
    for (let t = 0; t < tris; t++) if (choice[t] === k) idx.push(t * 3, t * 3 + 1, t * 3 + 2);
    if (!idx.length) continue;
    const piece = new THREE.BufferGeometry();
    for (const [name, attr] of Object.entries(g.attributes)) piece.setAttribute(name, attr);
    piece.setIndex(idx);
    out.push([piece, k]);
  }
  return out;
}
