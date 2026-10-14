// Photoreal rendering: the model path-traced (three-gpu-pathtracer, MIT) in a
// lit studio, its materials made physical from their looks: metals with true
// reflections, plastics with a satin sheen, glass and LED epoxy transmissive,
// boards with their mask and silk. The view's own scene is never touched: a
// render works on a copy of what is shown, from the view's camera.

import * as THREE from "three";
import { WebGLPathTracer } from "three-gpu-pathtracer";
import { picturesReady } from "./pictures";
import { type PreparedSkin, type Skin, type SkinWorld, prepareSkins, skinPieces, skinWorld } from "./skins";

/**
 * What the view shows: its part groups and its camera; `skins`, product photos to lay onto the
 * model (see skins.ts), `prepared` once they are loaded (withSkins).
 */
export type RenderSource = { objects: THREE.Object3D[]; camera: THREE.PerspectiveCamera; skins?: Skin[]; prepared?: PreparedSkin[] };

/** The source with its photo skins loaded, ready for buildRenderScene. */
export async function withSkins(source: RenderSource): Promise<RenderSource> {
  return source.skins?.length ? { ...source, prepared: await prepareSkins(source.skins) } : source;
}

/** A skinned face's material: the part's own finish over its photograph (the photo is its colour). */
const skinned = new Map<THREE.Material, Map<PreparedSkin, THREE.Material>>();
function skinMaterial(base: THREE.Material, skin: PreparedSkin): THREE.Material {
  let byBase = skinned.get(base);
  if (!byBase) skinned.set(base, (byBase = new Map()));
  let m = byBase.get(skin);
  if (!m) {
    const p = (base as THREE.MeshPhysicalMaterial).clone();
    p.map = skin.texture;
    p.color.set(0xffffff);
    // The photo carries the surface's look (grain, stipple, print): no texture of our own over it.
    p.normalMap = null;
    p.userData = { ...p.userData, texture: undefined };
    byBase.set(skin, (m = p));
  }
  return m;
}

/** Which modelled materials take a photo: plastics, rubber, paint and print (metals and see-through keep their own). */
const takesSkin = (m: THREE.Material) => {
  const p = m as THREE.MeshPhysicalMaterial;
  return p.metalness < 0.5 && !(p.transmission > 0);
};

/** Skinned pieces, worked out once per geometry, placement and set of photos. */
const skinCache = new WeakMap<THREE.BufferGeometry, { key: string; pieces: [THREE.BufferGeometry, number][] }>();

/**
 * The room the model is shown in: "studio", a grey dome with softboxes and a
 * floor fading into it; "white", a seamless white sweep (floor and backdrop
 * one pure white, only the model and its soft shadows on it), as a catalogue
 * product shot.
 */
export type Studio = "studio" | "white";

export type RenderOptions = { width: number; height: number; samples: number; studio?: Studio };

/**
 * A studio as an equirectangular HDR, Y up: a soft grey dome brighter
 * overhead, a dark floor bounce, and softboxes (a big key above front left, a
 * fill right, a strip overhead, a rim behind) that metals mirror as clean
 * highlights. `softboxes` scales the softboxes (the live view uses them
 * dimmed, beside its own sun). `userData.floor` is what a white matte floor under it sends back (its
 * radiance per unit albedo), so a backdrop can be made to match it.
 */
export function studioEnvironment(width = 1024, softboxes = 1): THREE.DataTexture {
  const height = width / 2;
  const data = new Float32Array(width * height * 4);
  // Softboxes: centre (azimuth, elevation in degrees), half sizes (degrees), intensity.
  const boxes: [number, number, number, number, number][] = [
    [-35, 42, 26, 15, 7.5], // key, front left, high
    [62, 20, 16, 13, 2.6], // fill, front right
    [0, 78, 70, 7, 4.5], // overhead strip
    [175, 28, 30, 10, 5.5], // rim, behind
  ];
  for (let y = 0; y < height; y++) {
    const elevation = 90 - (y / (height - 1)) * 180;
    for (let x = 0; x < width; x++) {
      const azimuth = (x / width) * 360 - 180;
      // Dome: a cool grey brightening upward; the floor below the horizon dimmer and warm.
      const up = Math.max(0, Math.sin(THREE.MathUtils.degToRad(elevation)));
      let r: number, g: number, b: number;
      if (elevation >= 0) {
        const k = 0.5 + 0.55 * Math.pow(up, 0.6);
        [r, g, b] = [0.92 * k, 0.95 * k, 1.0 * k];
      } else {
        const k = 0.26 + 0.12 * Math.exp(elevation / 12);
        [r, g, b] = [1.0 * k, 0.97 * k, 0.93 * k];
      }
      for (const [az, el, ha, he, intensity] of boxes) {
        let da = Math.abs(azimuth - az);
        if (da > 180) da = 360 - da;
        const de = Math.abs(elevation - el);
        // Soft edges: full inside, falling off over two degrees.
        const fa = THREE.MathUtils.clamp((ha - da) / 2 + 0.5, 0, 1);
        const fe = THREE.MathUtils.clamp((he - de) / 2 + 0.5, 0, 1);
        const f = fa * fe * intensity * softboxes;
        r += f;
        g += f;
        b += f * 0.97;
      }
      const i = (y * width + x) * 4;
      data[i] = r;
      data[i + 1] = g;
      data[i + 2] = b;
      data[i + 3] = 1;
    }
  }
  return equirect(data, width, height);
}

/**
 * The white sweep's light, set as a product photographer sets it, in the
 * camera's frame (azimuth 0 toward the camera, +90 its right, elevation from
 * the floor): the white sweep behind and below the product, black cards round
 * the camera and the sides, a big softbox high on the camera's left (the key),
 * a smaller one on its right (fill), one overhead toward the camera, a graded
 * diffuser over the top, and two
 * tall strips behind for rim light. Gloss and chrome need the dark: a face
 * turned to the camera mirrors the black cards and reads deep, a badge's
 * dome carries a crisp highlight, steel shows bright and dark bands and reads
 * as metal, where a room white all round made them all a flat grey or white.
 * `turnStudio` turns it to the camera for each picture.
 */
export function productStudio(width = 1024): THREE.DataTexture {
  const height = width / 2;
  const data = new Float32Array(width * height * 4);
  // Softboxes: centre (azimuth, elevation in degrees), half sizes (degrees), intensity.
  const boxes: [number, number, number, number, number][] = [
    // A smaller, harder key raking across the sides from high on the left, so every slot, groove and rib
    // casts a crisp shadow; little fill, so the shadow side stays deep; bright strips behind trace the edges.
    [-80, 26, 13, 15, 22], // key, from the camera's left side, low: it skims the side faces, so grooves and ribs shade
    [55, 15, 16, 12, 1.5], // fill, camera right, low
    [0, 62, 30, 8, 2.6], // overhead, toward the camera
    [-122, 24, 3, 30, 12], // strip, behind left (rim)
    [124, 24, 3, 30, 10], // strip, behind right (rim)
  ];
  const smooth = (e0: number, e1: number, x: number) => {
    const t = THREE.MathUtils.clamp((x - e0) / (e1 - e0), 0, 1);
    return t * t * (3 - 2 * t);
  };
  for (let y = 0; y < height; y++) {
    const elevation = 90 - (y / (height - 1)) * 180;
    for (let x = 0; x < width; x++) {
      const azimuth = (x / width) * 360 - 180;
      // Black cards round the camera, the white sweep behind the product, the white floor.
      const behind = smooth(95, 135, Math.abs(azimuth));
      let v = elevation < 0 ? 0.8 : 0.02 + 0.7 * behind * (1 - smooth(45, 75, elevation));
      // Overhead, a big diffuser graded from bright on the camera's left to dim on its right (across,
      // not round the zenith): flat steel or a gloss top seen from above mirrors a silver sweep, not a grey.
      if (elevation >= 0) {
        const across = Math.cos(THREE.MathUtils.degToRad(elevation)) * Math.sin(THREE.MathUtils.degToRad(azimuth));
        v += smooth(55, 75, elevation) * (0.25 + 1.6 * (1 - smooth(-0.3, 0.3, across)));
      }
      let r = v, g = v, b = v;
      for (const [az, el, ha, he, intensity] of boxes) {
        let da = Math.abs(azimuth - az);
        if (da > 180) da = 360 - da;
        const de = Math.abs(elevation - el);
        // Soft edges: full inside, falling off over three degrees.
        const f = THREE.MathUtils.clamp((ha - da) / 3 + 0.5, 0, 1) * THREE.MathUtils.clamp((he - de) / 3 + 0.5, 0, 1) * intensity;
        r += f;
        g += f;
        b += f * 0.98;
      }
      const i = (y * width + x) * 4;
      data[i] = r;
      data[i + 1] = g;
      data[i + 2] = b;
      data[i + 3] = 1;
    }
  }
  return equirect(data, width, height);
}

/** An equirectangular HDR from its pixels (top row the zenith), with what a white matte floor under it sends back. */
function equirect(data: Float32Array, width: number, height: number): THREE.DataTexture {
  // Light on a floor facing up: the radiance above the horizon, cosine weighted (E / π).
  let floor = 0;
  for (let y = 0; y < height; y++) {
    const elevation = THREE.MathUtils.degToRad(90 - (y / (height - 1)) * 180);
    if (elevation <= 0) continue;
    const solidAngle = ((2 * Math.PI) / width) * (Math.PI / height) * Math.cos(elevation);
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      floor += ((data[i] + data[i + 1] + data[i + 2]) / 3) * Math.sin(elevation) * solidAngle;
    }
  }
  const tex = new THREE.DataTexture(data, width, height, THREE.RGBAFormat, THREE.FloatType);
  tex.userData.floor = floor / Math.PI;
  tex.mapping = THREE.EquirectangularReflectionMapping;
  tex.colorSpace = THREE.LinearSRGBColorSpace;
  tex.minFilter = THREE.LinearFilter;
  tex.magFilter = THREE.LinearFilter;
  tex.flipY = false;
  tex.needsUpdate = true;
  return tex;
}

/**
 * Turns the product studio so its azimuth 0 faces the camera: the direction
 * to the camera across the floor (for a view straight down, the bottom of the
 * picture). `under`: the camera is below the floor and the studio is turned
 * over. The studio's azimuth is the world's atan2(y, x) and its elevation the
 * world's, once turned Z up.
 */
export function turnStudio(scene: THREE.Scene, camera: THREE.Camera, under = false) {
  const forward = camera.getWorldDirection(new THREE.Vector3());
  let toward = new THREE.Vector2(-forward.x, -forward.y);
  if (toward.length() < 0.05) {
    const up = new THREE.Vector3(0, 1, 0).applyQuaternion(camera.getWorldQuaternion(new THREE.Quaternion()));
    toward = new THREE.Vector2(-up.x, -up.y);
  }
  const azimuth = Math.atan2(toward.y, toward.x);
  scene.environmentRotation.set(under ? Math.PI / 2 : -Math.PI / 2, 0, azimuth, "ZYX");
}

/** Size of one tile of a surface texture, mm. */
const TEXTURE_TILE_MM = 10;

/** A tiling height field (n × n, any units) as a normal map; `strength` steepens it. */
function normalMap(height: Float32Array, n: number, strength: number): THREE.DataTexture {
  const data = new Uint8Array(n * n * 4);
  const at = (x: number, y: number) => height[((y + n) % n) * n + ((x + n) % n)];
  for (let y = 0; y < n; y++)
    for (let x = 0; x < n; x++) {
      const nx = (at(x - 1, y) - at(x + 1, y)) * strength, ny = (at(x, y - 1) - at(x, y + 1)) * strength;
      const len = Math.hypot(nx, ny, 1);
      const i = (y * n + x) * 4;
      data[i] = Math.round(((nx / len) * 0.5 + 0.5) * 255);
      data[i + 1] = Math.round(((ny / len) * 0.5 + 0.5) * 255);
      data[i + 2] = Math.round(((1 / len) * 0.5 + 0.5) * 255);
      data[i + 3] = 255;
    }
  const tex = new THREE.DataTexture(data, n, n, THREE.RGBAFormat);
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  tex.magFilter = THREE.LinearFilter;
  tex.minFilter = THREE.LinearMipmapLinearFilter;
  tex.generateMipmaps = true;
  tex.colorSpace = THREE.NoColorSpace;
  tex.needsUpdate = true;
  return tex;
}

/** A seeded random in [0, 1). */
function random(seed: number) {
  return () => (seed = (seed * 16807) % 2147483647) / 2147483647;
}

/** Smooth noise that tiles over `n`: random values on a `cells` × `cells` lattice, eased between. */
function tilingNoise(n: number, cells: number, rand: () => number): Float32Array {
  const lattice = Float32Array.from({ length: cells * cells }, rand);
  const out = new Float32Array(n * n);
  const ease = (t: number) => t * t * (3 - 2 * t);
  for (let y = 0; y < n; y++)
    for (let x = 0; x < n; x++) {
      const fx = (x / n) * cells, fy = (y / n) * cells;
      const x0 = Math.floor(fx), y0 = Math.floor(fy);
      const tx = ease(fx - x0), ty = ease(fy - y0);
      const v = (i: number, j: number) => lattice[((j + cells) % cells) * cells + ((i + cells) % cells)];
      const top = v(x0, y0) * (1 - tx) + v(x0 + 1, y0) * tx;
      const bottom = v(x0, y0 + 1) * (1 - tx) + v(x0 + 1, y0 + 1) * tx;
      out[y * n + x] = top * (1 - ty) + bottom * ty;
    }
  return out;
}

const surfaceMaps = new Map<string, THREE.DataTexture>();

/** The normal map of a surface texture a look names, over a 10 mm tile. */
function surfaceTexture(kind: string): THREE.DataTexture | null {
  const made = surfaceMaps.get(kind);
  if (made) return made;
  const n = 512;
  const height = new Float32Array(n * n);
  let strength: number;
  if (kind === "stipple") {
    // A moulded stipple (a grip's pebbled rubber): round bumps 0.2–0.35 mm across, packed at random.
    const rand = random(7);
    for (let k = 0; k < 2600; k++) {
      const cx = rand() * n, cy = rand() * n, r = 8 + rand() * 7, h = 0.6 + rand() * 0.4;
      for (let dy = -Math.ceil(r); dy <= Math.ceil(r); dy++)
        for (let dx = -Math.ceil(r); dx <= Math.ceil(r); dx++) {
          const d = Math.hypot(dx, dy) / r;
          if (d >= 1) continue;
          const x = (Math.floor(cx) + dx + n) % n, y = (Math.floor(cy) + dy + n) % n;
          const bump = h * Math.sqrt(1 - d * d);
          if (bump > height[y * n + x]) height[y * n + x] = bump;
        }
    }
    strength = 3;
  } else if (kind === "grain") {
    // A mould's fine texture (as on a matte ABS or PC/ABS housing, the like of VDI 3400
    // 27–30 or a fine Mold-Tech grain): an irregular sandy grain about 0.1–0.4 mm across,
    // shallow, which breaks highlights up into the soft sheen of moulded plastic.
    const rand = random(11);
    for (const [cells, weight] of [[128, 0.55], [64, 0.3], [32, 0.15]] as const) {
      const layer = tilingNoise(n, cells, rand);
      for (let i = 0; i < n * n; i++) height[i] += layer[i] * weight;
    }
    strength = 2.2;
  } else if (kind === "brushed") {
    // Brushed metal: fine parallel lines of the abrasive along the tile's u, each a random
    // depth, running a random length before the next starts, a faint waviness across.
    const rand = random(23);
    const wave = tilingNoise(n, 8, rand);
    for (let y = 0; y < n; y++) {
      let x = 0;
      while (x < n) {
        const run = 40 + Math.floor(rand() * 300), depth = rand();
        for (let k = 0; k < run && x + k < n; k++) height[y * n + x + k] = depth;
        x += run;
      }
    }
    for (let i = 0; i < n * n; i++) height[i] = height[i] * 0.8 + wave[i] * 0.2;
    strength = 1.1;
  } else return null;
  const tex = normalMap(height, n, strength);
  surfaceMaps.set(kind, tex);
  return tex;
}

/** The geometry with texture coordinates projected from the box side each vertex faces (mm / tile), for surface textures. */
function boxUvs(geometry: THREE.BufferGeometry, mmPerUnit: number): THREE.BufferGeometry {
  const g = geometry.clone();
  const pos = g.getAttribute("position");
  if (!g.getAttribute("normal")) g.computeVertexNormals();
  const nor = g.getAttribute("normal");
  const uv = new Float32Array(pos.count * 2);
  const k = mmPerUnit / TEXTURE_TILE_MM;
  for (let i = 0; i < pos.count; i++) {
    const [x, y, z] = [pos.getX(i) * k, pos.getY(i) * k, pos.getZ(i) * k];
    const [ax, ay, az] = [Math.abs(nor.getX(i)), Math.abs(nor.getY(i)), Math.abs(nor.getZ(i))];
    const [u, v] = ax >= ay && ax >= az ? [y, z] : ay >= az ? [x, z] : [x, y];
    uv[i * 2] = u;
    uv[i * 2 + 1] = v;
  }
  g.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
  return g;
}

/** A view material made physical for the path tracer. */
function physical(m: THREE.Material): THREE.Material {
  if (!(m instanceof THREE.MeshStandardMaterial)) return m;
  // A part's material as designed (its look), not as the view paints it at
  // the moment: a picked piece leaves the rest see-through (they rendered as
  // glass), x-ray lights what is hovered. None of that is the product.
  const look = m.userData.look as { color?: string; roughness?: number; metalness?: number; opacity?: number; texture?: string } | undefined;
  const color = look?.color ? new THREE.Color(look.color) : m.color.clone();
  const rough = look?.roughness ?? m.roughness;
  const metal = look?.metalness ?? m.metalness;
  const out = new THREE.MeshPhysicalMaterial({
    color,
    map: m.map,
    roughness: rough,
    metalness: metal,
    roughnessMap: m.roughnessMap,
    metalnessMap: m.metalnessMap,
    // A surface texture the look names (stippled rubber, a mould's grain, brushed steel) as a normal map.
    normalMap: (look?.texture && surfaceTexture(look.texture)) || m.normalMap,
    // A part's look has no glow of its own: what glows in the view is its
    // selection or hover highlight, which a render leaves out.
    emissive: look ? 0x000000 : m.emissive.clone(),
    emissiveMap: look ? null : m.emissiveMap,
    emissiveIntensity: look ? 0 : m.emissiveIntensity,
    alphaTest: m.alphaTest,
    side: THREE.DoubleSide,
  });
  if (look?.texture) out.userData.texture = look.texture;
  const hsl = { h: 0, s: 0, l: 0 };
  color.getHSL(hsl);
  const seeThrough = (look ? (look.opacity ?? 1) : m.opacity) < 0.999;
  if (seeThrough) {
    // Glass, acrylic, LED epoxy, water: transmissive, tinted by the colour.
    out.transmission = 1;
    out.opacity = 1;
    out.transparent = false;
    out.ior = 1.5;
    out.thickness = 2;
    out.roughness = Math.min(rough, 0.08);
    out.metalness = 0;
    out.attenuationColor = color.clone();
    out.attenuationDistance = 40;
    // Light reaches what lies under clear plastic or glass (a shipping cover
    // over a cartridge, a window over a display) instead of leaving it in a
    // shadow only rare refracted paths would lighten.
    (out as THREE.MeshPhysicalMaterial & { castShadow: boolean }).castShadow = false;
    // Strongly coloured, judged as the eye sees it (sRGB): a glass's or acrylic's faint tint is not.
    const shown = color.clone().convertLinearToSRGB();
    if (Math.max(shown.r, shown.g, shown.b) - Math.min(shown.r, shown.g, shown.b) > 0.3) {
      // A coloured translucent plastic (green blast doors, a red lens) glows its
      // colour, dyed through and partly scattering, rather than showing what is behind it darkly.
      out.transmission = 0.45;
      (out as THREE.MeshPhysicalMaterial & { castShadow: boolean }).castShadow = true;
      out.attenuationDistance = 3;
      out.roughness = Math.max(0.15, Math.min(rough, 0.3));
    }
  } else if (metal >= 0.5) {
    // Machined and sheet metal: a fine brushed sheen, never a mirror; a
    // saturated colour reads as anodised (a clear layer over the dye).
    out.roughness = THREE.MathUtils.clamp(rough * 0.85, 0.16, 0.5);
    out.metalness = 1;
    if (hsl.s > 0.25 || hsl.l < 0.2) {
      out.clearcoat = 0.6;
      out.clearcoatRoughness = 0.25;
      out.metalness = 0.85;
    }
  } else if (metal < 0.2) {
    // Plastics, masks, wood, paint: a dielectric; only a glossy one (an epoxy dome, a polished
    // moulding) gets a clear coat, so satin and matte mouldings keep their full colour.
    out.metalness = 0;
    out.specularIntensity = 0.6;
    out.clearcoat = rough < 0.3 ? 0.35 : 0;
    out.clearcoatRoughness = Math.max(0.04, rough * 0.5);
  }
  return out;
}

const split = new WeakMap<THREE.BufferGeometry, THREE.BufferGeometry[]>();

/**
 * A mesh as (geometry, material) pieces, one per material: a part of several
 * pieces is one geometry with a group per piece, which the path tracer does
 * not map to its materials reliably (the floor came out in a part's yellow),
 * so each group becomes a geometry of its own (the vertex data shared).
 */
function pieces(mesh: THREE.Mesh): [THREE.BufferGeometry, THREE.Material | undefined][] {
  const g = mesh.geometry;
  if (!Array.isArray(mesh.material)) return [[g, mesh.material]];
  const index = g.getIndex();
  const total = index ? index.count : g.getAttribute("position").count;
  const groups = g.groups.length ? g.groups : [{ start: 0, count: total, materialIndex: 0 }];
  let parts = split.get(g);
  if (!parts) {
    parts = groups.map(({ start, count }) => {
      const n = Math.max(0, Math.min(count, total - start));
      const sub = new THREE.BufferGeometry();
      for (const [name, attribute] of Object.entries(g.attributes)) sub.setAttribute(name, attribute);
      sub.setIndex(new THREE.BufferAttribute(index ? index.array.slice(start, start + n) : Uint32Array.from({ length: n }, (_, i) => start + i), 1));
      return sub;
    });
    split.set(g, parts);
  }
  const materials = mesh.material;
  return groups.map((group, i) => [parts[i], materials[group.materialIndex ?? 0]]);
}

/** The white sweep's floor: a matte white, without sheen, so it is one even white from every angle. */
const WHITE_FLOOR = 0.9;

/**
 * A scene to render: copies of every visible mesh where it sits in the view,
 * physical materials, the studio light, and a large matte floor under the
 * model for soft contact shadows. Z is up, as in the view. In the white
 * studio the backdrop is exactly what the unshadowed floor sends back, so
 * floor and backdrop meet without a horizon; `white` is that radiance.
 * `layer` (white studio, for a saved image): "model" cuts the floor out of
 * the picture (it still bounces light and takes the shadow) and leaves the
 * backdrop empty; "floor" shows the floor on an empty backdrop, for its shading.
 */
export function buildRenderScene(
  source: RenderSource,
  environment: THREE.Texture,
  studio: Studio = "studio",
  layer?: "model" | "floor",
): { scene: THREE.Scene; bounds: THREE.Box3; white: number } {
  const scene = new THREE.Scene();
  const materials = new Map<THREE.Material, THREE.Material>();
  const convert = (m: THREE.Material) => {
    let p = materials.get(m);
    if (!p) materials.set(m, (p = physical(m)));
    return p;
  };
  const bounds = new THREE.Box3();
  const skins = source.prepared ?? [];
  const skinKey = skins.map((k) => k.name).join("|");
  let world: SkinWorld | null = null;
  const visible = (o: THREE.Object3D) => {
    for (let x: THREE.Object3D | null = o; x; x = x.parent) if (!x.visible) return false;
    return true;
  };
  if (skins.length) {
    // Everything shown, for the skins' sight lines (a photo paints only what its camera saw).
    const all: { geometry: THREE.BufferGeometry; matrix: THREE.Matrix4 }[] = [];
    for (const root of source.objects) {
      root.updateMatrixWorld(true);
      root.traverse((o) => {
        const mesh = o as THREE.Mesh;
        if (mesh.isMesh && visible(mesh) && !(mesh as unknown as THREE.InstancedMesh).isInstancedMesh) all.push({ geometry: mesh.geometry, matrix: mesh.matrixWorld });
      });
    }
    world = skinWorld(all);
  }
  for (const root of source.objects) {
    root.updateMatrixWorld(true);
    root.traverse((o) => {
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh || !visible(mesh) || (mesh as unknown as THREE.InstancedMesh).isInstancedMesh) return;
      // Only shaded surfaces (no overlays, picking proxies or see-through ghosts of the view);
      // instances drawn as GPU instances have their own materials off, and are parts all the same.
      const batched = mesh.userData.batched === true;
      const shaded = (m: THREE.Material | undefined): m is THREE.MeshStandardMaterial => m instanceof THREE.MeshStandardMaterial && !m.wireframe && (m.visible || batched);
      let added = false;
      for (const [geometry, m] of pieces(mesh)) {
        if (!shaded(m)) continue;
        const material = convert(m);
        // A textured surface needs texture coordinates: projected from the box side it faces, in mm.
        const scale = new THREE.Vector3().setFromMatrixScale(mesh.matrixWorld).x || 1;
        const own = material.userData.texture ? boxUvs(geometry, 1000 * scale) : geometry;
        // Photo skins: the faces the product's photos saw take the photo, the rest their own finish.
        let parts: [THREE.BufferGeometry, THREE.Material][] = [[own, material]];
        if (skins.length && takesSkin(material)) {
          const key = `${skinKey}@${mesh.matrixWorld.elements.map((v) => v.toFixed(4)).join(",")}`;
          let cached = skinCache.get(geometry);
          if (!cached || cached.key !== key) skinCache.set(geometry, (cached = { key, pieces: skinPieces(geometry, mesh.matrixWorld, skins, world) }));
          parts = cached.pieces.map(([g, k]) => [k < 0 ? (material.userData.texture ? boxUvs(g, 1000 * scale) : g) : g, k < 0 ? material : skinMaterial(material, skins[k])]);
        }
        for (const [g, mat] of parts) {
          const copy = new THREE.Mesh(g, mat);
          mesh.matrixWorld.decompose(copy.position, copy.quaternion, copy.scale);
          scene.add(copy);
        }
        added = true;
      }
      if (!added) return;
      if (!mesh.geometry.boundingBox) mesh.geometry.computeBoundingBox();
      bounds.union(mesh.geometry.boundingBox!.clone().applyMatrix4(mesh.matrixWorld));
    });
  }
  const sweep = studio === "white";
  const white = sweep ? WHITE_FLOOR * (environment.userData.floor ?? 1) : 1;
  if (!bounds.isEmpty()) {
    const size = bounds.getSize(new THREE.Vector3());
    const span = Math.max(size.x, size.y, size.z, 1);
    const floor = new THREE.Mesh(
      new THREE.CircleGeometry(span * 40, 96),
      sweep
        ? // Purely Lambertian: no reflection at all (index 1, no specular), and the path
          // tracer's Disney diffuse is exactly Lambert at roughness 0, so the floor does
          // not darken toward the horizon (a rough one does, by half at grazing).
          new THREE.MeshPhysicalMaterial({ color: new THREE.Color().setRGB(WHITE_FLOOR, WHITE_FLOOR, WHITE_FLOOR, THREE.LinearSRGBColorSpace), roughness: 0.02, metalness: 0, ior: 1, specularIntensity: 0 })
        : new THREE.MeshPhysicalMaterial({ color: 0xf3f4f6, roughness: 0.92, metalness: 0 }),
    );
    floor.position.set((bounds.min.x + bounds.max.x) / 2, (bounds.min.y + bounds.max.y) / 2, bounds.min.z - span * 1e-4);
    floor.userData.floor = true;
    if (sweep && layer === "model") (floor.material as unknown as { matte: boolean }).matte = true;
    scene.add(floor);
  }
  scene.environment = environment;
  if (sweep && layer) scene.background = null;
  else if (sweep) scene.background = new THREE.Color().setRGB(white, white, white, THREE.LinearSRGBColorSpace);
  else {
    scene.background = environment;
    scene.backgroundBlurriness = 0.4;
  }
  // The studio is made Y up; the model is Z up. The white sweep's lights face the camera.
  scene.environmentRotation.set(-Math.PI / 2, 0, 0);
  scene.backgroundRotation.set(-Math.PI / 2, 0, 0);
  if (sweep) turnStudio(scene, source.camera);
  return { scene, bounds, white };
}

/**
 * A camera like the view's, for an image `aspect` wide: as much of the scene
 * across as the view shows (a square or upright image widens the angle
 * rather than cropping the sides).
 */
function cameraFor(source: RenderSource, aspect: number) {
  const c = source.camera.clone();
  if (aspect < c.aspect) {
    const half = Math.tan(THREE.MathUtils.degToRad(c.fov / 2)) * (c.aspect / aspect);
    c.fov = THREE.MathUtils.radToDeg(2 * Math.atan(half));
  }
  c.aspect = aspect;
  c.updateProjectionMatrix();
  return c;
}

const environments = new Map<Studio, THREE.DataTexture>();
function environment(studio: Studio) {
  let e = environments.get(studio);
  if (!e) environments.set(studio, (e = studio === "white" ? productStudio() : studioEnvironment()));
  return e;
}

/**
 * Tone mapping for a studio: ACES for the grey studio. The white sweep is
 * linear, as a product photo is exposed: the unshadowed floor a little over
 * white so it is pure white once the samples settle, the model's colours
 * true below it (black plastic stays a readable deep grey, not crushed), and
 * only highlights brighter than the backdrop clip.
 */
function expose(renderer: THREE.WebGLRenderer, studio: Studio, white: number) {
  if (studio !== "white") {
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 0.85;
    return;
  }
  renderer.toneMapping = THREE.LinearToneMapping;
  renderer.toneMappingExposure = 1.3 / white;
}

/**
 * Progressive render on a canvas: each frame adds samples, a camera move
 * starts over. The caller puts the canvas over the view.
 */
export class LiveRender {
  readonly canvas: HTMLCanvasElement;
  private renderer: THREE.WebGLRenderer;
  private tracer: WebGLPathTracer;
  private camera: THREE.PerspectiveCamera;
  private scene: THREE.Scene;
  private seen = new THREE.Matrix4();
  private seenAspect = 0;
  private frame = 0;
  private stopped = false;
  ready = false;

  constructor(private source: RenderSource, onFrame: (samples: number) => void, private studio: Studio = "studio") {
    this.canvas = document.createElement("canvas");
    this.renderer = new THREE.WebGLRenderer({ canvas: this.canvas, antialias: false, preserveDrawingBuffer: true });
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
    const built = buildRenderScene(source, environment(studio), studio);
    this.scene = built.scene;
    expose(this.renderer, studio, built.white);
    this.camera = cameraFor(source, 1);
    this.tracer = new WebGLPathTracer(this.renderer);
    this.tracer.tiles.set(2, 2);
    this.tracer.filterGlossyFactor = 0.5;
    this.tracer.bounces = 6;
    this.tracer.transmissiveBounces = 8;
    this.tracer.minSamples = 1;
    this.tracer.renderDelay = 0;
    this.tracer.fadeDuration = 200;
    this.tracer.dynamicLowRes = true;
    this.tracer.setScene(this.scene, this.camera);
    this.ready = true;
    // Labels whose pictures arrive later: printed once they are in.
    void picturesReady().then(() => !this.stopped && this.tracer.updateMaterials());
    const loop = () => {
      if (this.stopped) return;
      this.frame = requestAnimationFrame(loop);
      this.tick();
      onFrame(this.ready ? Math.floor(this.tracer.samples) : -1);
    };
    this.frame = requestAnimationFrame(loop);
  }

  private tick() {
    const host = this.canvas.parentElement;
    if (!host || !this.ready) return;
    const w = host.clientWidth;
    const h = host.clientHeight;
    const size = this.renderer.getSize(new THREE.Vector2());
    if (size.x !== w || size.y !== h) this.renderer.setSize(w, h, false);
    // Follow the view's camera; any move starts the image over.
    const view = this.source.camera;
    view.updateMatrixWorld();
    if (!view.matrixWorld.equals(this.seen) || this.seenAspect !== w / h || this.camera.fov !== view.fov) {
      this.seen.copy(view.matrixWorld);
      this.seenAspect = w / h;
      this.camera.copy(view);
      this.camera.aspect = w / h;
      this.camera.updateProjectionMatrix();
      this.tracer.updateCamera();
      if (this.studio === "white") {
        turnStudio(this.scene, this.camera);
        this.tracer.updateEnvironment();
      }
    }
    this.tracer.renderSample();
  }

  get samples() {
    return this.ready ? Math.floor(this.tracer.samples) : 0;
  }

  dispose() {
    this.stopped = true;
    cancelAnimationFrame(this.frame);
    this.tracer.dispose();
    this.renderer.dispose();
    this.renderer.forceContextLoss();
  }
}

/** Yields to the page once (a message round trip, which hidden tabs do not throttle as they do timers). */
function breathe(): Promise<void> {
  return new Promise((done) => {
    const channel = new MessageChannel();
    channel.port1.onmessage = () => {
      channel.port1.close();
      done();
    };
    channel.port2.postMessage(null);
  });
}

/**
 * One image at a chosen size, rendered off screen until `samples` samples
 * have landed, as a PNG. `onProgress` hears the samples so far.
 */
export async function renderImage(source: RenderSource, options: RenderOptions, onProgress?: (samples: number) => void): Promise<Blob> {
  const { width, height, samples, studio = "studio" } = options;
  // Every printed picture (labels, decals) and photo skin in before the first sample.
  await picturesReady();
  source = await withSkins(source);
  if (studio !== "white") return png((await trace(source, studio, undefined, width, height, samples, onProgress)).pixels, width, height);
  // The white sweep: the model on a cut-out floor, then the floor alone (fewer samples) for
  // its shading, laid on pure white. One pass with the floor in the picture leaves a grey
  // horizon and floor grain: the big softboxes shade the floor a little far round the model.
  const model = await trace(source, studio, "model", width, height, samples, onProgress);
  const shade = model.floorless ? model : await trace(source, studio, "floor", width, height, Math.max(48, Math.round(samples / 3)), (n) => onProgress?.(samples + n));
  return png(onWhite(model.pixels, shade.pixels, width, height), width, height);
}

/**
 * The model (straight-alpha pixels over a cut-out floor) laid on pure white,
 * with the floor's shadow from `floor` (the floor alone on an empty backdrop):
 * each floor pixel's light against the open floor's (its median: the shadow
 * covers little of it), smoothed of grain, the last 3 % counted as open.
 */
function onWhite(model: Uint8ClampedArray, floor: Uint8ClampedArray, width: number, height: number): Uint8ClampedArray {
  const n = width * height;
  const toLinear = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  };
  const toSrgb = (c: number) => 255 * (c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055);
  const lum = new Float32Array(n);
  const seen: number[] = [];
  for (let i = 0; i < n; i++) {
    if (floor[i * 4 + 3] < 250) {
      lum[i] = -1; // past the floor: open white
      continue;
    }
    lum[i] = 0.2126 * toLinear(floor[i * 4]) + 0.7152 * toLinear(floor[i * 4 + 1]) + 0.0722 * toLinear(floor[i * 4 + 2]);
    if (model[i * 4 + 3] === 0 && i % 7 === 0) seen.push(lum[i]);
  }
  seen.sort((a, b) => a - b);
  const open = seen.length ? seen[Math.floor(seen.length / 2)] : 1;
  let shadow = new Float32Array(n);
  for (let i = 0; i < n; i++) shadow[i] = lum[i] < 0 ? 1 : Math.min(1, lum[i] / Math.max(open, 1e-6) / 0.97);
  const radius = Math.max(1, Math.round(Math.max(width, height) / 600));
  for (let pass = 0; pass < 2; pass++) {
    const next = new Float32Array(n);
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        let sum = 0, count = 0;
        for (let d = -radius; d <= radius; d++) {
          const xx = pass === 0 ? x + d : x, yy = pass === 0 ? y : y + d;
          if (xx < 0 || yy < 0 || xx >= width || yy >= height) continue;
          sum += shadow[yy * width + xx];
          count++;
        }
        next[y * width + x] = sum / count;
      }
    shadow = next;
  }
  const out = new Uint8ClampedArray(n * 4);
  for (let i = 0; i < n; i++) {
    const bg = toSrgb(1 - 0.65 * (1 - shadow[i]));
    const alpha = model[i * 4 + 3] / 255;
    for (let c = 0; c < 3; c++) out[i * 4 + c] = Math.round(alpha * model[i * 4 + c] + (1 - alpha) * bg);
    out[i * 4 + 3] = 255;
  }
  return out;
}

/** RGBA pixels as a PNG. */
async function png(pixels: Uint8ClampedArray, width: number, height: number): Promise<Blob> {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  canvas.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(pixels), width, height), 0, 0);
  const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
  if (!blob) throw new Error("the image could not be encoded");
  return blob;
}

/**
 * Path-traces the scene until `samples` have landed: its pixels (RGBA,
 * straight alpha, top row first), and whether the floor was left out (a
 * camera under it, as for a view from below: the studio is turned over too).
 */
async function trace(
  source: RenderSource,
  studio: Studio,
  layer: "model" | "floor" | undefined,
  width: number,
  height: number,
  samples: number,
  onProgress?: (samples: number) => void,
): Promise<{ pixels: Uint8ClampedArray; floorless: boolean }> {
  const canvas = document.createElement("canvas");
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, preserveDrawingBuffer: true, alpha: !!layer, premultipliedAlpha: false });
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.setPixelRatio(1);
  renderer.setSize(width, height, false);
  if (layer) renderer.setClearColor(0x000000, 0);
  const { scene, white } = buildRenderScene(source, environment(studio), studio, layer);
  expose(renderer, studio, white);
  const camera = cameraFor(source, width / height);
  const ground = scene.children.find((o) => o.userData.floor);
  if (layer && ground && camera.position.z < ground.position.z) {
    scene.remove(ground);
    turnStudio(scene, camera, true);
  }
  const tracer = new WebGLPathTracer(renderer);
  // Big images in tiles, so each draw call stays short.
  const tiles = Math.max(1, Math.ceil(Math.max(width, height) / 1024));
  tracer.tiles.set(tiles, tiles);
  tracer.filterGlossyFactor = 0.5;
  tracer.bounces = 6;
  tracer.transmissiveBounces = 8;
  tracer.minSamples = 1;
  tracer.renderDelay = 0;
  tracer.fadeDuration = 0;
  tracer.dynamicLowRes = false;
  tracer.rasterizeScene = false;
  tracer.synchronizeRenderSize = true;
  try {
    tracer.setScene(scene, camera);
    let last = -1;
    while (tracer.samples < samples) {
      tracer.renderSample();
      const now = Math.floor(tracer.samples);
      if (now !== last) onProgress?.((last = now));
      // Let the page (and the GPU) breathe between draws. A message, not a
      // timer: a render asked for by ./ise often runs in a tab nobody is
      // looking at, where timers wait a second or more each (it never finished).
      await breathe();
    }
    // The pixels as drawn, straight from the GL buffer (bottom row first).
    const gl = renderer.getContext();
    const raw = new Uint8Array(width * height * 4);
    gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, raw);
    const pixels = new Uint8ClampedArray(width * height * 4);
    for (let y = 0; y < height; y++) pixels.set(raw.subarray((height - 1 - y) * width * 4, (height - y) * width * 4), y * width * 4);
    return { pixels, floorless: !!ground && !ground.parent };
  } finally {
    tracer.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
  }
}
