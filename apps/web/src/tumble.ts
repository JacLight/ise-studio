// Free tumble for the model view.
//
// OrbitControls stops the camera at the poles, so dragging up or down hits a
// wall at the top and the bottom. Here a left drag turns the view about the
// world's vertical (Z) and the screen's horizontal axis with no stop: the
// camera rolls over the top and underneath as often as it is dragged, its up
// direction turning with it. OrbitControls keeps panning and zooming.

import * as THREE from "three";
import type { OrbitControls } from "three/addons/controls/OrbitControls.js";

const Z = new THREE.Vector3(0, 0, 1);
/** Pixels a press may move and still be a click (picking), not a drag. */
const SLOP = 3;

export type Tumble = { enabled: boolean; dispose: () => void };

/** Turns the camera about the target by a drag of (dx, dy) pixels, in a view `height` pixels tall. */
export function tumble(camera: THREE.Camera, target: THREE.Vector3, dx: number, dy: number, height: number, speed = 1) {
  const k = ((2 * Math.PI) / Math.max(height, 1)) * speed;
  const offset = camera.position.clone().sub(target);
  const up = camera.up.clone();
  // Dragging right turns the scene right, upside down too.
  const yaw = new THREE.Quaternion().setFromAxisAngle(Z, -dx * k * (up.dot(Z) < 0 ? -1 : 1));
  offset.applyQuaternion(yaw);
  up.applyQuaternion(yaw);
  // Dragging down raises the camera, on over the top.
  const right = new THREE.Vector3().crossVectors(offset.clone().negate(), up).normalize();
  if (right.lengthSq() > 0.5) {
    const pitch = new THREE.Quaternion().setFromAxisAngle(right, -dy * k);
    offset.applyQuaternion(pitch);
    up.applyQuaternion(pitch);
  }
  camera.position.copy(target).add(offset);
  camera.up.copy(up.normalize());
  camera.lookAt(target);
}

/** Left-drag (or one finger) tumbles the view; OrbitControls' own rotation is switched off. */
export function attachTumble(dom: HTMLElement, camera: THREE.Camera, controls: OrbitControls): Tumble {
  controls.enableRotate = false;
  const state: Tumble = { enabled: true, dispose: () => {} };
  let drag: { id: number; x: number; y: number; moving: boolean } | null = null;
  const touches = new Set<number>();

  const down = (e: PointerEvent) => {
    if (e.pointerType === "touch") touches.add(e.pointerId);
    if (!state.enabled || !controls.enabled || e.button !== 0 || touches.size > 1) {
      drag = null; // a second finger pinches and pans instead
      return;
    }
    drag = { id: e.pointerId, x: e.clientX, y: e.clientY, moving: false };
  };
  const move = (e: PointerEvent) => {
    if (!drag || e.pointerId !== drag.id) return;
    if (!controls.enabled) return void (drag = null); // a handle took the drag
    const dx = e.clientX - drag.x;
    const dy = e.clientY - drag.y;
    if (!drag.moving) {
      if (Math.hypot(dx, dy) < SLOP) return;
      drag.moving = true;
      dom.setPointerCapture?.(e.pointerId);
      controls.dispatchEvent({ type: "start" });
    }
    drag.x = e.clientX;
    drag.y = e.clientY;
    tumble(camera, controls.target, dx, dy, dom.clientHeight, controls.rotateSpeed);
    controls.dispatchEvent({ type: "change" });
  };
  const up = (e: PointerEvent) => {
    touches.delete(e.pointerId);
    if (drag && e.pointerId === drag.id) {
      if (drag.moving) controls.dispatchEvent({ type: "end" });
      drag = null;
    }
  };
  dom.addEventListener("pointerdown", down);
  dom.addEventListener("pointermove", move);
  dom.addEventListener("pointerup", up);
  dom.addEventListener("pointercancel", up);
  state.dispose = () => {
    dom.removeEventListener("pointerdown", down);
    dom.removeEventListener("pointermove", move);
    dom.removeEventListener("pointerup", up);
    dom.removeEventListener("pointercancel", up);
  };
  return state;
}

/**
 * Zooming in walks on once the view is close: OrbitControls only ever halves
 * the way to its target, so the camera would stall in front of the nearest
 * wall. Within `floor()` of the target, a wheel step moves the camera and the
 * target together along the ray under the pointer, through a case's lid or a
 * floor into what is inside. Zooming out is OrbitControls' as usual.
 */
export function attachWalk(dom: HTMLElement, camera: THREE.PerspectiveCamera, controls: OrbitControls, floor: () => number): () => void {
  const wheel = (e: WheelEvent) => {
    if (!controls.enabled || !controls.enableZoom || e.deltaY >= 0) return;
    const reach = floor();
    if (camera.position.distanceTo(controls.target) > reach * 1.5) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const rect = dom.getBoundingClientRect();
    const ndc = new THREE.Vector2(((e.clientX - rect.left) / rect.width) * 2 - 1, -((e.clientY - rect.top) / rect.height) * 2 + 1);
    const ray = new THREE.Raycaster();
    ray.setFromCamera(ndc, camera);
    // A notch of a mouse wheel (100) walks a third of the reach; a trackpad's small deltas walk less.
    const step = reach * 0.35 * Math.min(Math.abs(e.deltaY) / 100, 2);
    const move = ray.ray.direction.multiplyScalar(step);
    camera.position.add(move);
    controls.target.add(move);
    controls.dispatchEvent({ type: "change" });
  };
  dom.addEventListener("wheel", wheel, { capture: true, passive: false });
  return () => dom.removeEventListener("wheel", wheel, { capture: true });
}
