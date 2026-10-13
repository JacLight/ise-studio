// The sun's shadow map, fitted each frame to what it has to cover: the model
// (and where its shadow lands on the ground), cut down to the part of it the
// camera sees. A whole house gets texels of a few millimetres; zoomed in on
// a window, the recess's shadow is drawn from texels a fraction of that.
//
// The fit is made in the light's own frame, its size moves in steps and its
// centre snaps to whole texels, so shadow edges hold still while the view
// turns; the biases follow the texel size, so neither acne nor floating
// shadows come back at any scale.

import * as THREE from "three";

/** PCF filter radius, in texels: soft, clean edges with no stair steps. */
export const SHADOW_RADIUS = 2.5;

const X = new THREE.Vector3();
const Y = new THREE.Vector3();
const Z = new THREE.Vector3();
const eye = new THREE.Vector3();
const at = new THREE.Vector3();
const d = new THREE.Vector3();
const p = new THREE.Vector3();
const lightDir = new THREE.Vector3();
const camAt = new THREE.Vector3();
const camFwd = new THREE.Vector3();
const basis = new THREE.Matrix4();
const receivers = Array.from({ length: 16 }, () => new THREE.Vector3());

/** Sizes move in steps of a quarter octave (about 19 %). */
const step = (v: number) => 2 ** (Math.ceil(Math.log2(Math.max(v, 1e-6)) * 4) / 4);

/**
 * Fits `sun`'s shadow camera to `bounds` (world mm) as seen by `camera`;
 * `ground` is the height shadows fall on outside the model.
 */
export function fitSunShadow(sun: THREE.DirectionalLight, bounds: THREE.Box3, ground: number, camera: THREE.Camera) {
  if (bounds.isEmpty()) return;
  const shadow = sun.shadow;
  const size = shadow.mapSize.x;
  eye.setFromMatrixPosition(sun.matrixWorld);
  at.setFromMatrixPosition(sun.target.matrixWorld);
  basis.lookAt(eye, at, shadow.camera.up);
  basis.extractBasis(X, Y, Z);
  lightDir.subVectors(eye, at).normalize();

  // What can catch a shadow: the model's corners and where they cast onto the ground.
  let n = 0;
  for (let i = 0; i < 8; i++) {
    const c = receivers[n++].set(i & 1 ? bounds.max.x : bounds.min.x, i & 2 ? bounds.max.y : bounds.min.y, i & 4 ? bounds.max.z : bounds.min.z);
    if (lightDir.z > 1e-3 && c.z > ground) receivers[n++].copy(c).addScaledVector(lightDir, -(c.z - ground) / lightDir.z);
  }

  // The model in the light's frame (x, y across the beam, z towards the light).
  let x0 = Infinity, y0 = Infinity, z0 = Infinity, x1 = -Infinity, y1 = -Infinity, z1 = -Infinity;
  for (let i = 0; i < n; i++) {
    d.subVectors(receivers[i], eye);
    const x = d.dot(X), y = d.dot(Y), z = d.dot(Z);
    x0 = Math.min(x0, x), x1 = Math.max(x1, x);
    y0 = Math.min(y0, y), y1 = Math.max(y1, y);
    z0 = Math.min(z0, z), z1 = Math.max(z1, z);
  }

  // What the camera sees of it: the view's volume out to the farthest receiver.
  camAt.setFromMatrixPosition(camera.matrixWorld);
  camFwd.setFromMatrixColumn(camera.matrixWorld, 2).negate();
  let far = -Infinity;
  for (let i = 0; i < n; i++) far = Math.max(far, d.subVectors(receivers[i], camAt).dot(camFwd));
  const projection = camera as THREE.PerspectiveCamera | THREE.OrthographicCamera;
  const perspective = (projection as THREE.PerspectiveCamera).isPerspectiveCamera === true;
  const near = perspective ? projection.near : -Infinity;
  if (far > near && far > 0) {
    let fx0 = Infinity, fy0 = Infinity, fx1 = -Infinity, fy1 = -Infinity;
    for (let i = 0; i < 8; i++) {
      // A corner of the view's near rectangle, in the camera's frame…
      p.set(i & 1 ? 1 : -1, i & 2 ? 1 : -1, -1).applyMatrix4(camera.projectionMatrixInverse);
      // …and of its far one, at the depth of the farthest receiver.
      if (i & 4) {
        if (perspective) p.multiplyScalar(far / -p.z);
        else p.z = -far;
      }
      p.applyMatrix4(camera.matrixWorld);
      d.subVectors(p, eye);
      const x = d.dot(X), y = d.dot(Y);
      fx0 = Math.min(fx0, x), fx1 = Math.max(fx1, x);
      fy0 = Math.min(fy0, y), fy1 = Math.max(fy1, y);
    }
    // An orthographic view's volume runs back towards the camera without end; its rectangle across is what limits.
    if (Math.max(fx0, x0) < Math.min(fx1, x1) && Math.max(fy0, y0) < Math.min(fy1, y1)) {
      x0 = Math.max(fx0, x0), x1 = Math.min(fx1, x1);
      y0 = Math.max(fy0, y0), y1 = Math.min(fy1, y1);
    }
  }

  // Sizes in steps, the centre on whole texels, a margin for the filter.
  const margin = (SHADOW_RADIUS + 3) / size;
  const w = step((x1 - x0) * (1 + 2 * margin));
  const h = step((y1 - y0) * (1 + 2 * margin));
  const tx = w / size, ty = h / size;
  const cx = Math.round((x0 + x1) / 2 / tx) * tx;
  const cy = Math.round((y0 + y1) / 2 / ty) * ty;
  const depth = z1 - z0;
  const cam = shadow.camera;
  cam.left = cx - w / 2;
  cam.right = cx + w / 2;
  cam.bottom = cy - h / 2;
  cam.top = cy + h / 2;
  cam.near = -z1 - depth * 0.02 - 1;
  cam.far = -z0 + depth * 0.02 + 1;
  cam.updateProjectionMatrix();
  // Biases by the texel: surfaces never shadow themselves, shadows still touch what casts them.
  const texel = Math.max(tx, ty);
  shadow.normalBias = texel * 1.4;
  shadow.bias = -(texel * 0.35) / (cam.far - cam.near);
  shadow.radius = SHADOW_RADIUS;
}
