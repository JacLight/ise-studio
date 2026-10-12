// Orthographic or perspective. The perspective camera stays the one that is
// steered (orbit, tumble, walk, glides); in orthographic mode a parallel
// camera follows it each frame: same place and turn, and as tall a view at
// the target as the perspective one has there, so zoom (moving closer) still
// zooms and switching keeps the model the same size on screen.

import * as THREE from "three";

export type ViewName = "perspective" | "iso" | "front" | "back" | "left" | "right" | "top" | "bottom";

/** Where the camera looks from (towards the target) for each standard view, Z up. */
export const VIEW_DIRECTIONS: Record<ViewName, [number, number, number]> = {
  perspective: [0.55, -0.7, 0.45],
  iso: [1, -1, 1],
  front: [0, -1, 0],
  back: [0, 1, 0],
  left: [-1, 0, 0],
  right: [1, 0, 0],
  top: [0, 0, 1],
  bottom: [0, 0, -1],
};

/** Screen-up for a view from `dir`: Z, or Y when looking straight down or up it. */
export function upFor(dir: THREE.Vector3): THREE.Vector3 {
  const d = dir.clone().normalize();
  if (Math.abs(d.z) > 0.999) return new THREE.Vector3(0, 1, 0);
  return new THREE.Vector3(0, 0, 1);
}

export class Projection {
  readonly ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, -1, 1);
  orthographic = false;

  /** The camera to draw and pick with this frame. */
  camera(perspective: THREE.PerspectiveCamera, target: THREE.Vector3, depth: number): THREE.Camera {
    if (!this.orthographic) return perspective;
    const o = this.ortho;
    const distance = perspective.position.distanceTo(target);
    const half = distance * Math.tan(THREE.MathUtils.degToRad(perspective.fov / 2));
    o.top = half;
    o.bottom = -half;
    o.left = -half * perspective.aspect;
    o.right = half * perspective.aspect;
    // Everything in front of and behind the eye shows: a parallel view has no "too close".
    o.near = -depth;
    o.far = distance + depth;
    o.position.copy(perspective.position);
    o.quaternion.copy(perspective.quaternion);
    o.up.copy(perspective.up);
    o.updateProjectionMatrix();
    o.updateMatrixWorld();
    return o;
  }
}
