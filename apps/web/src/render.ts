// Photoreal rendering: the model path-traced (three-gpu-pathtracer, MIT) in a
// lit studio, its materials made physical from their looks: metals with true
// reflections, plastics with a satin sheen, glass and LED epoxy transmissive,
// boards with their mask and silk. The view's own scene is never touched: a
// render works on a copy of what is shown, from the view's camera.

import * as THREE from "three";
import { WebGLPathTracer } from "three-gpu-pathtracer";

/** What the view shows: its part groups and its camera. */
export type RenderSource = { objects: THREE.Object3D[]; camera: THREE.PerspectiveCamera };

export type RenderOptions = { width: number; height: number; samples: number };

/**
 * A studio as an equirectangular HDR, Y up: a soft grey dome brighter
 * overhead, a dark floor bounce, and softboxes (a big key above front left, a
 * fill right, a strip overhead, a rim behind) that metals mirror as clean
 * highlights.
 */
export function studioEnvironment(width = 1024): THREE.DataTexture {
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
        const f = fa * fe * intensity;
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

/**
 * A scene to render: copies of every visible mesh where it sits in the view,
 * physical materials, the studio light, and a large matte floor under the
 * model for soft contact shadows. Z is up, as in the view.
 */
export function buildRenderScene(source: RenderSource, environment: THREE.Texture): { scene: THREE.Scene; bounds: THREE.Box3 } {
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
      // Only shaded surfaces (no overlays, picking proxies or see-through ghosts of the view).
      if (!mats.some((m) => m instanceof THREE.MeshStandardMaterial && !m.wireframe && m.visible)) return;
      const copy = new THREE.Mesh(mesh.geometry, Array.isArray(mesh.material) ? mats.map(convert) : convert(mesh.material));
      mesh.matrixWorld.decompose(copy.position, copy.quaternion, copy.scale);
      scene.add(copy);
      if (!mesh.geometry.boundingBox) mesh.geometry.computeBoundingBox();
      bounds.union(mesh.geometry.boundingBox!.clone().applyMatrix4(mesh.matrixWorld));
    });
  }
  if (!bounds.isEmpty()) {
    const size = bounds.getSize(new THREE.Vector3());
    const span = Math.max(size.x, size.y, size.z, 1);
    const floor = new THREE.Mesh(
      new THREE.CircleGeometry(span * 40, 96),
      new THREE.MeshPhysicalMaterial({ color: 0xf3f4f6, roughness: 0.92, metalness: 0 }),
    );
    floor.position.set((bounds.min.x + bounds.max.x) / 2, (bounds.min.y + bounds.max.y) / 2, bounds.min.z - span * 1e-4);
    scene.add(floor);
  }
  scene.environment = environment;
  scene.background = environment;
  scene.backgroundBlurriness = 0.4;
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

let sharedEnvironment: THREE.DataTexture | null = null;
const environment = () => (sharedEnvironment ??= studioEnvironment());

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

  constructor(private source: RenderSource, onFrame: (samples: number) => void) {
    this.canvas = document.createElement("canvas");
    this.renderer = new THREE.WebGLRenderer({ canvas: this.canvas, antialias: false, preserveDrawingBuffer: true });
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 0.85;
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
    this.scene = buildRenderScene(source, environment()).scene;
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
  const { width, height, samples } = options;
  const canvas = document.createElement("canvas");
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, preserveDrawingBuffer: true });
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 0.85;
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.setPixelRatio(1);
  renderer.setSize(width, height, false);
  const { scene } = buildRenderScene(source, environment());
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
