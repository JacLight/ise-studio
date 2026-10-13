// Free tumble for the model view, and panning without the right button:
// Space + drag, the middle button, or (when chosen in Preferences) two fingers
// on a trackpad with a pinch to zoom.
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

/** Slides the view (camera and target together) by a drag of (dx, dy) pixels. */
export function pan(camera: THREE.PerspectiveCamera, target: THREE.Vector3, dx: number, dy: number, height: number) {
  const distance = camera.position.distanceTo(target);
  const perPixel = (2 * distance * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2))) / Math.max(height, 1);
  camera.updateMatrixWorld();
  const right = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 0);
  const up = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 1);
  const move = right.multiplyScalar(-dx * perPixel).add(up.multiplyScalar(dy * perPixel));
  camera.position.add(move);
  target.add(move);
}

/** Whether two-finger trackpad scrolling pans (and a pinch zooms), from Preferences. */
export const trackpadPans = () => {
  try {
    return localStorage.getItem("forma.trackpadPan") === "1";
  } catch {
    return false;
  }
};

/** Left-drag (or one finger) tumbles the view; OrbitControls' own rotation is switched off. */
export function attachTumble(dom: HTMLElement, camera: THREE.PerspectiveCamera, controls: OrbitControls): Tumble {
  controls.enableRotate = false;
  // The middle button pans, as in most CAD tools (the wheel still zooms).
  controls.mouseButtons.MIDDLE = THREE.MOUSE.PAN;
  // Space held: a left drag pans instead of turning.
  let space = false;
  // Not while typing, or when Space would press a focused button.
  const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT|BUTTON|A|SUMMARY)$/.test(t.tagName) || !!t.closest("[role=button],[role=tab],[role=menuitem],dialog[open]"));
  const keyDown = (e: KeyboardEvent) => {
    if (e.code !== "Space" || typing(e.target) || e.repeat) return;
    space = true;
    dom.classList.add("is-panning");
    e.preventDefault();
  };
  const keyUp = (e: KeyboardEvent) => {
    if (e.code !== "Space") return;
    space = false;
    dom.classList.remove("is-panning");
  };
  const blur = () => {
    space = false;
    dom.classList.remove("is-panning");
  };
  window.addEventListener("keydown", keyDown);
  window.addEventListener("keyup", keyUp);
  window.addEventListener("blur", blur);
  // Two fingers on a trackpad pan when chosen; a pinch (ctrl + wheel) zooms as before.
  const wheel = (e: WheelEvent) => {
    if (!state.enabled || !controls.enabled || e.ctrlKey || !trackpadPans()) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    pan(camera, controls.target, -e.deltaX, -e.deltaY, dom.clientHeight);
    controls.dispatchEvent({ type: "change" });
  };
  dom.addEventListener("wheel", wheel, { capture: true, passive: false });
  const state: Tumble = { enabled: true, dispose: () => {} };
  let drag: { id: number; x: number; y: number; moving: boolean; pan: boolean } | null = null;
  const touches = new Set<number>();

  const down = (e: PointerEvent) => {
    if (e.pointerType === "touch") touches.add(e.pointerId);
    if (!state.enabled || !controls.enabled || e.button !== 0 || touches.size > 1) {
      drag = null; // a second finger pinches and pans instead
      return;
    }
    drag = { id: e.pointerId, x: e.clientX, y: e.clientY, moving: false, pan: space };
    // Panning takes the press from whatever else would use it (picking, moving a part).
    if (space) e.stopImmediatePropagation();
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
    if (drag.pan) pan(camera, controls.target, dx, dy, dom.clientHeight);
    else tumble(camera, controls.target, dx, dy, dom.clientHeight, controls.rotateSpeed);
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
    dom.removeEventListener("wheel", wheel, { capture: true });
    window.removeEventListener("keydown", keyDown);
    window.removeEventListener("keyup", keyUp);
    window.removeEventListener("blur", blur);
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
