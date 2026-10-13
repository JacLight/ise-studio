// A model for a website: the view's parts as one binary glTF (.glb) with the
// materials the photoreal render uses (plastics, metals, chrome, clear and
// tinted plastics, stippled rubber, printed labels), in metres with Y up,
// centred over the origin and standing on the ground, as web viewers
// (<model-viewer>, three.js, Babylon, Shopify, WooCommerce) and AR on phones
// expect. Hidden parts stay out; what the view shows is what goes in.

import * as THREE from "three";
import { picturesReady } from "./pictures";
import { buildRenderScene, type RenderSource } from "./render";

/** Largest texture side written (printed labels, the stipple map): plenty for a web page, small to download. */
const MAX_TEXTURE = 2048;

export async function modelForWeb(source: RenderSource): Promise<ArrayBuffer> {
  await picturesReady();
  const { GLTFExporter } = await import("three/examples/jsm/exporters/GLTFExporter.js");
  // The render's own scene (no studio needed: the page lights it).
  const { scene, bounds } = buildRenderScene(source, new THREE.Texture(), "studio");
  if (bounds.isEmpty()) throw new Error("nothing is shown to export");
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
