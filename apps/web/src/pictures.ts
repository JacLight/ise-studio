// Printed pictures: a label, a decal or a pad-printed logo is a part whose
// material names a picture added to the project ("Label print
// battery-label.png"). The picture is laid across the part flat, as a
// printer would put it on a thin sheet: projected along the part's thinnest
// direction, filling the part's extent, upright as the face is seen. The
// view and the photoreal render show it; the part stays an ordinary solid
// (its own mass, BOM line and outline).

import * as THREE from "three";
import { projectUrl } from "./engine";

type Entry = { id: string; name: string };
let listing: Promise<Entry[]> | null = null;
const pictures = () =>
  (listing ??= fetch(projectUrl("/images"))
    .then((r) => (r.ok ? r.json() : []))
    .catch(() => [] as Entry[]));

/** The picture a reference names: its file name, its id, or the id's first characters. */
const named = (e: Entry, ref: string) => {
  const r = ref.toLowerCase();
  return e.name.toLowerCase() === r || e.id === r || (/^[0-9a-f]{8,}$/.test(r) && e.id.startsWith(r));
};

const textures = new Map<string, THREE.Texture>();
const loading = new Set<Promise<void>>();
/** Called when a picture arrives (the view redraws). */
export const pictureEvents = { onLoaded: null as (() => void) | null };

/** White until the picture has loaded, so a label never flashes black. */
function blank() {
  const c = document.createElement("canvas");
  c.width = c.height = 1;
  const g = c.getContext("2d")!;
  g.fillStyle = "#fff";
  g.fillRect(0, 0, 1, 1);
  return c;
}

/** The texture for a picture named in a material (shared by every part that prints it). */
export function pictureTexture(ref: string): THREE.Texture {
  let t = textures.get(ref);
  if (t) return t;
  const texture = (t = new THREE.Texture(blank()));
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = 8;
  texture.needsUpdate = true;
  textures.set(ref, texture);
  const job = (async () => {
    const list = await pictures();
    // A name the project lists, newest first; else the reference is an id.
    const entry = [...list].reverse().find((e) => named(e, ref));
    if (!entry) {
      // A picture added after the list was read: read it again once.
      listing = null;
      const again = await pictures();
      const late = [...again].reverse().find((e) => named(e, ref));
      if (!late) return console.warn(`printed picture ${ref}: no picture of that name in the project (add it with ./forma image add)`);
      return load(texture, late.id);
    }
    return load(texture, entry.id);
  })();
  loading.add(job);
  void job.finally(() => loading.delete(job));
  return texture;
}

async function load(texture: THREE.Texture, id: string) {
  const img = new Image();
  img.crossOrigin = "anonymous";
  img.src = projectUrl(`/images/${id}`);
  try {
    await img.decode();
  } catch {
    return console.warn(`printed picture ${id}: could not be read`);
  }
  // The placeholder's storage is one pixel and fixed in size: let it go, so the picture is uploaded whole.
  texture.dispose();
  texture.image = img;
  texture.needsUpdate = true;
  pictureEvents.onLoaded?.();
}

/** Resolves when every picture asked for so far has loaded (a render waits for its labels). */
export async function picturesReady() {
  while (loading.size) await Promise.all([...loading]);
}

/**
 * Lays each printed part's picture on its triangles: `printed[i]` is the
 * picture of geometry group i (or of the whole geometry when it has no
 * groups). Seen from the side a face looks out of along the part's thinnest
 * direction, the picture's right and up are the view's (from +Z: right +X,
 * up +Y; from −Y, the front: right +X, up +Z; from −X: right −Y, up +Z; the
 * opposite sides mirrored to read the right way round from there).
 */
export function applyPictureUVs(geometry: THREE.BufferGeometry, printed: (string | undefined)[]) {
  if (!printed.some(Boolean)) return;
  const pos = geometry.getAttribute("position") as THREE.BufferAttribute;
  const normal = geometry.getAttribute("normal") as THREE.BufferAttribute | undefined;
  let uv = geometry.getAttribute("uv") as THREE.BufferAttribute | undefined;
  if (!uv) {
    uv = new THREE.BufferAttribute(new Float32Array(pos.count * 2), 2);
    geometry.setAttribute("uv", uv);
  }
  const index = geometry.getIndex();
  const total = index ? index.count : pos.count;
  const groups = geometry.groups.length ? geometry.groups : [{ start: 0, count: total, materialIndex: 0 }];
  for (const g of groups) {
    if (!printed[g.materialIndex ?? 0]) continue;
    const verts = new Set<number>();
    for (let t = g.start; t < g.start + g.count; t++) verts.add(index ? index.getX(t) : t);
    const lo = [Infinity, Infinity, Infinity];
    const hi = [-Infinity, -Infinity, -Infinity];
    for (const v of verts) {
      for (let k = 0; k < 3; k++) {
        const x = pos.getComponent(v, k);
        lo[k] = Math.min(lo[k], x);
        hi[k] = Math.max(hi[k], x);
      }
    }
    const size = [0, 1, 2].map((k) => Math.max(hi[k] - lo[k], 1e-9));
    const thin = size.indexOf(Math.min(...size));
    // Right and up axes as seen from the + side of the thin direction.
    const [right, up] = thin === 2 ? [0, 1] : thin === 1 ? [0, 2] : [1, 2];
    const across = (v: number, k: number) => (pos.getComponent(v, k) - lo[k]) / size[k];
    for (const v of verts) {
      const n = normal ? normal.getComponent(v, thin) : 1;
      const back = n < -0.5;
      let u = across(v, right);
      let w = across(v, up);
      // From −Z the picture's up is −Y; from +Y and −X its right runs the other way.
      if (thin === 2 && back) w = 1 - w;
      if (thin === 1 && !back) u = 1 - u;
      if (thin === 0 && back) u = 1 - u;
      uv.setXY(v, u, w);
    }
  }
  uv.needsUpdate = true;
}
