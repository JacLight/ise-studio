// Photoreal rendering: the model path-traced (three-gpu-pathtracer, MIT) in a
// lit studio, its materials made physical from their looks: metals with true
// reflections, plastics with a satin sheen, glass and LED epoxy transmissive,
// boards with their mask and silk. The view's own scene is never touched: a
// render works on a copy of what is shown, from the view's camera.

import * as THREE from "three";
import { WebGLPathTracer } from "three-gpu-pathtracer";
import { shown } from "./batchLayer";

/** What the view shows: its part groups and its camera. */
export type RenderSource = { objects: THREE.Object3D[]; camera: THREE.PerspectiveCamera };

/**
 * What the model stands in: "studio" (a soft grey dome with softboxes, its
 * floor fading into the room) or "white" (a seamless white sweep, as product
 * photographs are shot: the backdrop and floor read paper white, the model
 * keeps its true colours and casts a soft contact shadow).
 */
export type Backdrop = "studio" | "white";

export type RenderOptions = { width: number; height: number; samples: number; backdrop?: Backdrop };

/** Tone mapping per backdrop: the white sweep needs colours kept true and whites reaching white. */
function exposeFor(renderer: THREE.WebGLRenderer, backdrop: Backdrop) {
  if (backdrop === "white") {
    // Khronos PBR Neutral: base colours below the shoulder pass unchanged (a
    // blue stays that blue), so the sweep, lit past one, reads clean white.
    renderer.toneMapping = THREE.NeutralToneMapping;
    renderer.toneMappingExposure = 1;
  } else {
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 0.85;
  }
}

/**
 * A studio as an equirectangular HDR, Y up: a soft grey dome brighter
 * overhead, a dark floor bounce, and softboxes (a big key above front left, a
 * fill right, a strip overhead, a rim behind) that metals mirror as clean
 * highlights. `softboxes` scales the softboxes (the live view uses them
 * dimmed, beside its own sun).
 */
export function studioEnvironment(width = 1024, softboxes = 1, product = false): THREE.DataTexture {
  const height = width / 2;
  const data = new Float32Array(width * height * 4);
  // Softboxes: centre (azimuth, elevation in degrees), half sizes (degrees), intensity.
  const boxes: [number, number, number, number, number][] = product
    ? [
        // A product table: big soft sources either side of the camera and a
        // rim behind, nothing straight above or behind the camera, so a face
        // turned to the lens (a steel plate) reads as metal, not a mirror of a
        // light, and stamped edges catch the side light.
        [-55, 30, 30, 22, 4], // key, left of the camera
        [60, 25, 26, 18, 2.2], // fill, right of the camera
        [180, 40, 50, 18, 2.5], // rim, behind the model
      ]
    : [
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
      if (product) {
        // An even, dimmer room: the sources do the shaping.
        const k = elevation >= 0 ? 0.36 - 0.08 * up : 0.3;
        [r, g, b] = [k, k, k];
      } else if (elevation >= 0) {
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
  const tex = new THREE.DataTexture(data, width, height, THREE.RGBAFormat, THREE.FloatType);
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
    emissive: m.emissive.clone(),
    emissiveMap: m.emissiveMap,
    emissiveIntensity: m.emissiveIntensity,
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

/** One group of a geometry as a geometry of its own (only the vertices it uses), made once. */
const pieceCache = new WeakMap<THREE.BufferGeometry, THREE.BufferGeometry[]>();
function piece(geometry: THREE.BufferGeometry, group: number): THREE.BufferGeometry {
  let made = pieceCache.get(geometry);
  if (!made) pieceCache.set(geometry, (made = []));
  if (made[group]) return made[group];
  const { start, count } = geometry.groups[group];
  const index = geometry.index;
  const used = new Map<number, number>();
  const order: number[] = [];
  const indices = new Uint32Array(count);
  for (let k = 0; k < count; k++) {
    const v = index ? index.getX(start + k) : start + k;
    let at = used.get(v);
    if (at === undefined) {
      at = order.length;
      used.set(v, at);
      order.push(v);
    }
    indices[k] = at;
  }
  const out = new THREE.BufferGeometry();
  for (const [name, attribute] of Object.entries(geometry.attributes)) {
    const source = attribute as THREE.BufferAttribute;
    const size = source.itemSize;
    const array = new Float32Array(order.length * size);
    for (let n = 0; n < order.length; n++) for (let c = 0; c < size; c++) array[n * size + c] = source.getComponent(order[n], c);
    out.setAttribute(name, new THREE.BufferAttribute(array, size, source.normalized));
  }
  out.setIndex(new THREE.BufferAttribute(indices, 1));
  return (made[group] = out);
}

/**
 * A scene to render: copies of every visible mesh where it sits in the view,
 * physical materials, the studio light, and a large matte floor under the
 * model for soft contact shadows. Z is up, as in the view.
 */
export function buildRenderScene(source: RenderSource, environment: THREE.Texture, backdrop: Backdrop = "studio"): { scene: THREE.Scene; bounds: THREE.Box3 } {
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
      const mats = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
      // Only shaded surfaces (no overlays, picking proxies or see-through ghosts of the view);
      // instances of shared geometry count too, though the view draws them in batches.
      if (!mats.some((m) => m instanceof THREE.MeshStandardMaterial && !m.wireframe && shown(m))) return;
      // The path tracer merges every mesh into one and keeps one material per
      // mesh: a part of several pieces (a black body, a blue insert, a steel
      // plate) goes in as one mesh per piece, each with its own material; a
      // hidden piece stays out.
      const pieces = Array.isArray(mesh.material) && mesh.geometry.groups.length ? mesh.geometry.groups : null;
      const copies = pieces
        ? pieces.flatMap((g, i) => {
            const m = mats[g.materialIndex ?? 0];
            return m && shown(m) && g.count > 0 ? [new THREE.Mesh(piece(mesh.geometry, i), convert(m))] : [];
          })
        : [new THREE.Mesh(mesh.geometry, convert(mats[0]))];
      for (const copy of copies) {
        mesh.matrixWorld.decompose(copy.position, copy.quaternion, copy.scale);
        scene.add(copy);
      }
      if (!mesh.geometry.boundingBox) mesh.geometry.computeBoundingBox();
      bounds.union(mesh.geometry.boundingBox!.clone().applyMatrix4(mesh.matrixWorld));
    });
  }
  if (!bounds.isEmpty()) {
    const size = bounds.getSize(new THREE.Vector3());
    const span = Math.max(size.x, size.y, size.z, 1);
    // The white sweep glows a little of its own (a lit cyclorama is a light
    // to the model), so it reads white everywhere but where the model shades it.
    const white = backdrop === "white";
    const floor = new THREE.Mesh(
      new THREE.CircleGeometry(span * 40, 96),
      new THREE.MeshPhysicalMaterial({
        color: white ? 0xffffff : 0xf3f4f6,
        roughness: white ? 1 : 0.92,
        metalness: 0,
        emissive: white ? 0xffffff : 0x000000,
        emissiveIntensity: white ? WHITE_SWEEP.far : 0,
        // Near the model the sweep glows softly, so the model's shadow shows;
        // away from it, brighter and brighter, so it melts into the backdrop with no horizon.
        emissiveMap: white ? sweepGlow(WHITE_SWEEP.glow / WHITE_SWEEP.far) : null,
      }),
    );
    floor.position.set((bounds.min.x + bounds.max.x) / 2, (bounds.min.y + bounds.max.y) / 2, bounds.min.z - span * 1e-4);
    scene.add(floor);
  }
  scene.environment = environment;
  if (backdrop === "white") {
    // Seen past the model: paper white, past the tone curve's shoulder.
    // (Not exactly 1, 1, 1: the path tracer takes its colour backdrop for
    // already made when the colour equals its default, and draws it black.)
    scene.background = new THREE.Color(0.999, 0.999, 0.999);
    scene.backgroundIntensity = WHITE_SWEEP.background;
    scene.environmentIntensity = WHITE_SWEEP.light;
  } else {
    scene.background = environment;
    scene.backgroundBlurriness = 0.4;
  }
  // The studio is made Y up; the model is Z up.
  scene.environmentRotation.set(-Math.PI / 2, 0, 0);
  scene.backgroundRotation.set(-Math.PI / 2, 0, 0);
  return { scene, bounds };
}

/** A camera like the view's, for an image `aspect` wide. */
function cameraFor(source: RenderSource, aspect: number) {
  const c = source.camera.clone();
  c.aspect = aspect;
  c.updateProjectionMatrix();
  return c;
}

/**
 * The sweep's glow over the floor disc (its UVs: the centre at 0.5, 0.5, the
 * rim at 0.5 away): `near` round the model (within 2 of the disc's 40 spans),
 * rising to 1 by a quarter of the way out.
 */
function sweepGlow(near: number): THREE.DataTexture {
  const n = 256;
  const data = new Uint8Array(n * n * 4);
  for (let y = 0; y < n; y++)
    for (let x = 0; x < n; x++) {
      const r = Math.hypot((x + 0.5) / n - 0.5, (y + 0.5) / n - 0.5);
      const t = THREE.MathUtils.smoothstep(r, 0.06, 0.25);
      const v = Math.round(255 * (near + (1 - near) * t));
      data.set([v, v, v, 255], (y * n + x) * 4);
    }
  const tex = new THREE.DataTexture(data, n, n, THREE.RGBAFormat);
  tex.colorSpace = THREE.LinearSRGBColorSpace;
  tex.magFilter = THREE.LinearFilter;
  tex.minFilter = THREE.LinearFilter;
  tex.needsUpdate = true;
  return tex;
}

/** The white sweep's light: how bright the backdrop, the floor's own glow (near the model, far away) and the studio around it are. */
const WHITE_SWEEP = { background: 4, glow: 0.45, far: 2, light: 1 };
/** Turns the product studio's azimuth 0 (between its key and fill) to the camera. */
const LIGHTS_TURN = 0;

let sharedEnvironment: THREE.DataTexture | null = null;
let productEnvironment: THREE.DataTexture | null = null;
const environment = (backdrop: Backdrop) =>
  backdrop === "white" ? (productEnvironment ??= studioEnvironment(1024, 1, true)) : (sharedEnvironment ??= studioEnvironment());

/**
 * On the white sweep the lights stand where a photographer puts them, round
 * the camera: the studio turns about the vertical with the camera's heading
 * (for a camera looking straight down, the way the picture's top points).
 */
function followCamera(scene: THREE.Scene, camera: THREE.Camera, backdrop: Backdrop) {
  if (backdrop !== "white") return;
  camera.updateMatrixWorld();
  const e = camera.matrixWorld.elements;
  const forward = new THREE.Vector3(-e[8], -e[9], -e[10]);
  const up = new THREE.Vector3(e[4], e[5], e[6]);
  const heading = Math.hypot(forward.x, forward.y) > 0.25 ? forward : up;
  const turn = Math.atan2(heading.y, heading.x) + LIGHTS_TURN;
  scene.environmentRotation.set(-Math.PI / 2, 0, turn, "ZYX");
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
  private backdrop: Backdrop = "studio";
  private frame = 0;
  private stopped = false;
  ready = false;

  constructor(private source: RenderSource, onFrame: (samples: number) => void, backdrop: Backdrop = "studio") {
    this.canvas = document.createElement("canvas");
    this.renderer = new THREE.WebGLRenderer({ canvas: this.canvas, antialias: false, preserveDrawingBuffer: true });
    exposeFor(this.renderer, backdrop);
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
    this.backdrop = backdrop;
    this.scene = buildRenderScene(source, environment(backdrop), backdrop).scene;
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
      if (this.backdrop === "white") {
        followCamera(this.scene, this.camera, this.backdrop);
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

/**
 * One image at a chosen size, rendered off screen until `samples` samples
 * have landed, as a PNG. `onProgress` hears the samples so far.
 */
export async function renderImage(source: RenderSource, options: RenderOptions, onProgress?: (samples: number) => void): Promise<Blob> {
  const { width, height, samples, backdrop = "studio" } = options;
  const canvas = document.createElement("canvas");
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, preserveDrawingBuffer: true });
  exposeFor(renderer, backdrop);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.setPixelRatio(1);
  renderer.setSize(width, height, false);
  const { scene } = buildRenderScene(source, environment(backdrop), backdrop);
  const camera = cameraFor(source, width / height);
  followCamera(scene, camera, backdrop);
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
