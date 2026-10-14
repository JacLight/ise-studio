// A model for a website: the view's parts as one binary glTF (.glb) with the
// materials the photoreal render uses (plastics, metals, chrome, clear and
// tinted plastics, stippled rubber, printed labels), in metres with Y up,
// centred over the origin and standing on the ground, as web viewers
// (<model-viewer>, three.js, Babylon, Shopify, WooCommerce) and AR on phones
// expect. Hidden parts stay out; what the view shows is what goes in.
// A web viewer lights a model only by which way each surface faces: it casts
// no shadow into a slot, a groove or the gap under a rib, so mouldings read
// flat. Each vertex's openness (ambient occlusion: the share of the hemisphere
// over it that the model itself blocks within a few millimetres) is baked into
// the file as vertex colour, which every viewer multiplies into the material,
// so the depth shows under any light.

import * as THREE from "three";
import { picturesReady } from "./pictures";
import { buildRenderScene, type RenderSource, withSkins } from "./render";
import { skinWorld } from "./skins";

/** Largest texture side written (printed labels, the stipple map): plenty for a web page, small to download. */
const MAX_TEXTURE = 2048;

export async function modelForWeb(source: RenderSource): Promise<ArrayBuffer> {
  await picturesReady();
  source = await withSkins(source);
  const { GLTFExporter } = await import("three/examples/jsm/exporters/GLTFExporter.js");
  // The render's own scene (no studio needed: the page lights it).
  const { scene, bounds } = buildRenderScene(source, new THREE.Texture(), "studio");
  if (bounds.isEmpty()) throw new Error("nothing is shown to export");
  bakeOcclusion(scene);
  const centre = bounds.getCenter(new THREE.Vector3());
  // Millimetres, Z up → metres, Y up; centred in plan, its lowest point on the ground.
  const product = new THREE.Group();
  product.name = "Product";
  product.rotation.x = -Math.PI / 2;
  product.scale.setScalar(0.001);
  product.position.set(-centre.x / 1000, -bounds.min.z / 1000, centre.y / 1000);
  for (const child of [...scene.children]) {
    // The studio floor is for renders only.
    if (child.userData.floor) continue;
    product.add(child);
  }
  // Materials as glTF reads them: no environment of their own; textures made from raw
  // pixels (the stipple normal map) as pictures, which is what a glTF file holds.
  const pictures = new Map<THREE.Texture, THREE.Texture>();
  const asPicture = (t: THREE.Texture | null): THREE.Texture | null => {
    if (!t || !(t as THREE.DataTexture).isDataTexture) return t;
    let done = pictures.get(t);
    if (!done) {
      const { data, width, height } = t.image as { data: ArrayLike<number>; width: number; height: number };
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const rgba = new Uint8ClampedArray(width * height * 4);
      const channels = data.length / (width * height);
      for (let i = 0; i < width * height; i++) {
        for (let c = 0; c < 4; c++) rgba[i * 4 + c] = c < channels ? Number(data[i * channels + c]) : 255;
      }
      canvas.getContext("2d")!.putImageData(new ImageData(rgba, width, height), 0, 0);
      done = new THREE.CanvasTexture(canvas);
      done.wrapS = t.wrapS;
      done.wrapT = t.wrapT;
      done.repeat.copy(t.repeat);
      done.colorSpace = t.colorSpace;
      done.flipY = t.flipY;
      pictures.set(t, done);
    }
    return done;
  };
  product.traverse((o) => {
    const mesh = o as THREE.Mesh;
    if (!mesh.isMesh) return;
    for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) {
      const p = m as THREE.MeshPhysicalMaterial;
      p.envMap = null;
      for (const key of ["map", "normalMap", "roughnessMap", "metalnessMap", "alphaMap", "emissiveMap", "aoMap"] as const) p[key] = asPicture(p[key]);
    }
  });
  const out = await new GLTFExporter().parseAsync(product, { binary: true, maxTextureSize: MAX_TEXTURE, onlyVisible: true });
  return out as ArrayBuffer;
}

/** How far (mm) a surface looks for what shades it, how many rays per vertex, and how dark a fully enclosed point gets. */
const AO_REACH = 6, AO_RAYS = 20, AO_STRENGTH = 0.8;

/** Bakes ambient occlusion into every mesh of the scene (millimetres) as vertex colours. */
function bakeOcclusion(scene: THREE.Scene) {
  const meshes: THREE.Mesh[] = [];
  scene.updateMatrixWorld(true);
  scene.traverse((o) => {
    const m = o as THREE.Mesh;
    if (m.isMesh && !m.userData.floor) meshes.push(m);
  });
  const world = skinWorld(meshes.map((m) => ({ geometry: m.geometry, matrix: m.matrixWorld })));
  if (!world) return;
  // Cosine-weighted directions over a hemisphere round +Z (a fixed spiral, so the bake is the same every time).
  const dirs: THREE.Vector3[] = [];
  for (let i = 0; i < AO_RAYS; i++) {
    const r = Math.sqrt((i + 0.5) / AO_RAYS), a = i * 2.399963;
    dirs.push(new THREE.Vector3(r * Math.cos(a), r * Math.sin(a), Math.sqrt(1 - r * r)));
  }
  const ray = new THREE.Ray(), p = new THREE.Vector3(), n = new THREE.Vector3(), d = new THREE.Vector3(), q = new THREE.Quaternion(), up = new THREE.Vector3(0, 0, 1);
  const done = new Map<THREE.BufferGeometry, THREE.BufferGeometry>();
  for (const mesh of meshes) {
    // Each placed copy is shaded where it stands: its own copy of the geometry.
    let g = done.get(mesh.geometry);
    if (g) g = g.clone();
    else done.set(mesh.geometry, (g = mesh.geometry.clone()));
    if (!g.getAttribute("normal")) g.computeVertexNormals();
    const pos = g.getAttribute("position"), nor = g.getAttribute("normal");
    const normalMatrix = new THREE.Matrix3().getNormalMatrix(mesh.matrixWorld);
    const colour = new Float32Array(pos.count * 3);
    for (let i = 0; i < pos.count; i++) {
      p.fromBufferAttribute(pos, i).applyMatrix4(mesh.matrixWorld);
      n.fromBufferAttribute(nor, i).applyMatrix3(normalMatrix).normalize();
      q.setFromUnitVectors(up, n);
      ray.origin.copy(p).addScaledVector(n, 0.02);
      let hit = 0;
      for (const dir of dirs) {
        ray.direction.copy(d.copy(dir).applyQuaternion(q));
        if (world.bvh.raycastFirst(ray, THREE.DoubleSide, 0, AO_REACH)) hit++;
      }
      const v = 1 - AO_STRENGTH * (hit / AO_RAYS);
      colour[i * 3] = colour[i * 3 + 1] = colour[i * 3 + 2] = v;
    }
    g.setAttribute("color", new THREE.BufferAttribute(colour, 3));
    mesh.geometry = g;
    for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) (m as THREE.MeshStandardMaterial).vertexColors = true;
  }
}
