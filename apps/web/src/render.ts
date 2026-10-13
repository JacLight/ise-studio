// Photoreal rendering: the model path-traced (three-gpu-pathtracer, MIT) in a
// lit studio, its materials made physical from their looks: metals with true
// reflections, plastics with a satin sheen, glass and LED epoxy transmissive,
// boards with their mask and silk. The view's own scene is never touched: a
// render works on a copy of what is shown, from the view's camera.

import * as THREE from "three";
import { WebGLPathTracer } from "three-gpu-pathtracer";

/** What the view shows: its part groups and its camera. */
export type RenderSource = { objects: THREE.Object3D[]; camera: THREE.PerspectiveCamera };

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
 * dimmed, beside its own sun). `white`: an even white dome all round (the
 * seamless sweep) with the softboxes over it for shape and highlights.
 * `userData.floor` is what a white matte floor under it sends back (its
 * radiance per unit albedo), so a backdrop can be made to match it.
 */
export function studioEnvironment(width = 1024, softboxes = 1, white = false): THREE.DataTexture {
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
      // The white sweep: white walls all round under a dimmer ceiling, as in a
      // product studio. A face turned up (a steel plate seen from the front)
      // mirrors the ceiling and reads as metal, not as a white mirror; a
      // stamped letter's sloped sides mirror the bright walls, so stamping reads.
      if (white) [r, g, b] = Array(3).fill(0.35 + 0.65 * Math.pow(1 - up, 2)) as [number, number, number];
      else if (elevation >= 0) {
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
        // (The overhead strip stays out of the white sweep: it would sit in every top-down reflection.)
        const f = white && el > 70 ? 0 : fa * fe * intensity * softboxes;
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

/** A view material made physical for the path tracer. */
function physical(m: THREE.Material): THREE.Material {
  if (!(m instanceof THREE.MeshStandardMaterial)) return m;
  const look = (m.userData.look ?? {}) as { opacity?: number };
  const out = new THREE.MeshPhysicalMaterial({
    color: m.color.clone(),
    map: m.map,
    roughness: m.roughness,
    metalness: m.metalness,
    roughnessMap: m.roughnessMap,
    metalnessMap: m.metalnessMap,
    normalMap: m.normalMap,
    // A part's look has no glow of its own: what glows in the view is its
    // selection or hover highlight, which a render leaves out.
    emissive: m.userData.look ? 0x000000 : m.emissive.clone(),
    emissiveMap: m.userData.look ? null : m.emissiveMap,
    emissiveIntensity: m.userData.look ? 0 : m.emissiveIntensity,
    side: THREE.DoubleSide,
  });
  const hsl = { h: 0, s: 0, l: 0 };
  m.color.getHSL(hsl);
  const seeThrough = (look.opacity ?? m.opacity) < 0.999;
  if (seeThrough) {
    // Glass, acrylic, LED epoxy, water: transmissive, tinted by the colour.
    out.transmission = 1;
    out.opacity = 1;
    out.transparent = false;
    out.ior = 1.5;
    out.thickness = 2;
    out.roughness = Math.min(m.roughness, 0.08);
    out.metalness = 0;
    out.attenuationColor = m.color.clone();
    out.attenuationDistance = 40;
  } else if (m.metalness >= 0.5) {
    // Machined and sheet metal: a fine brushed sheen, never a mirror; a
    // saturated colour reads as anodised (a clear layer over the dye).
    out.roughness = THREE.MathUtils.clamp(m.roughness * 0.85, 0.16, 0.5);
    out.metalness = 1;
    if (hsl.s > 0.25 || hsl.l < 0.2) {
      out.clearcoat = 0.6;
      out.clearcoatRoughness = 0.25;
      out.metalness = 0.85;
    }
  } else if (m.metalness < 0.2) {
    // Plastics, masks, wood, paint: a dielectric with a faint satin coat.
    out.metalness = 0;
    out.specularIntensity = 0.6;
    out.clearcoat = m.roughness < 0.5 ? 0.25 : 0.05;
    out.clearcoatRoughness = 0.4;
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
 */
export function buildRenderScene(source: RenderSource, environment: THREE.Texture, studio: Studio = "studio"): { scene: THREE.Scene; bounds: THREE.Box3; white: number } {
  const scene = new THREE.Scene();
  const materials = new Map<THREE.Material, THREE.Material>();
  const convert = (m: THREE.Material) => {
    let p = materials.get(m);
    if (!p) materials.set(m, (p = physical(m)));
    return p;
  };
  const bounds = new THREE.Box3();
  const visible = (o: THREE.Object3D) => {
    for (let x: THREE.Object3D | null = o; x; x = x.parent) if (!x.visible) return false;
    return true;
  };
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
        const copy = new THREE.Mesh(geometry, convert(m));
        mesh.matrixWorld.decompose(copy.position, copy.quaternion, copy.scale);
        scene.add(copy);
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
    scene.add(floor);
  }
  scene.environment = environment;
  if (sweep) scene.background = new THREE.Color().setRGB(white, white, white, THREE.LinearSRGBColorSpace);
  else {
    scene.background = environment;
    scene.backgroundBlurriness = 0.4;
  }
  // The studio is made Y up; the model is Z up.
  scene.environmentRotation.set(-Math.PI / 2, 0, 0);
  scene.backgroundRotation.set(-Math.PI / 2, 0, 0);
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
  if (!e) environments.set(studio, (e = studio === "white" ? studioEnvironment(1024, 0.55, true) : studioEnvironment()));
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
  renderer.toneMappingExposure = 1.06 / white;
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

  constructor(private source: RenderSource, onFrame: (samples: number) => void, studio: Studio = "studio") {
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

/**
 * One image at a chosen size, rendered off screen until `samples` samples
 * have landed, as a PNG. `onProgress` hears the samples so far.
 */
export async function renderImage(source: RenderSource, options: RenderOptions, onProgress?: (samples: number) => void): Promise<Blob> {
  const { width, height, samples, studio = "studio" } = options;
  const canvas = document.createElement("canvas");
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, preserveDrawingBuffer: true });
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.setPixelRatio(1);
  renderer.setSize(width, height, false);
  const { scene, white } = buildRenderScene(source, environment(studio), studio);
  expose(renderer, studio, white);
  const camera = cameraFor(source, width / height);
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
      // Let the page (and the GPU) breathe between draws.
      await new Promise((r) => setTimeout(r, 0));
    }
    const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
    if (!blob) throw new Error("the image could not be encoded");
    return blob;
  } finally {
    tracer.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
  }
}
