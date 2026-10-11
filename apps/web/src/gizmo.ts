// Manipulators drawn over the model.
//
// MoveGizmo: three axis arrows from one origin (X red, Y green, Z blue, the
// CAD convention), plane handles for two-axis moves, and a center grip for
// moving on the ground. FaceHandle: the selected face, highlighted, with one
// push-pull arrow along its normal. All are drawn on top and keep a constant
// size on screen.

import * as THREE from "three";
import type { Shape } from "./model";

export type Axis = 0 | 1 | 2;
export type Face = { axis: Axis; sign: 1 | -1 };
export type GizmoPick = { kind: "axis"; axis: Axis } | { kind: "plane"; normal: Axis } | { kind: "center" } | { kind: "push" };

export const AXIS_COLORS = ["#e5484d", "#2f9e64", "#3e63dd"] as const;
export const ACCENT = "#7352d6";
export const AXIS_NAMES = ["X", "Y", "Z"] as const;
export const SIZE_NAMES = ["Width", "Depth", "Height"] as const;
export const UNIT = [new THREE.Vector3(1, 0, 0), new THREE.Vector3(0, 1, 0), new THREE.Vector3(0, 0, 1)] as const;

function overlay(color: THREE.ColorRepresentation, opacity = 1, side: THREE.Side = THREE.FrontSide) {
  return new THREE.MeshBasicMaterial({
    color,
    opacity,
    transparent: true,
    depthTest: false,
    depthWrite: false,
    toneMapped: false,
    fog: false,
    side,
  });
}

/** Orients a +Y-built object along an axis. */
function alongAxis(obj: THREE.Object3D, axis: Axis, sign = 1) {
  if (axis === 0) obj.rotation.z = sign > 0 ? -Math.PI / 2 : Math.PI / 2;
  if (axis === 1 && sign < 0) obj.rotation.z = Math.PI;
  if (axis === 2) obj.rotation.x = sign > 0 ? Math.PI / 2 : -Math.PI / 2;
}

type Handle = {
  pick: GizmoPick;
  root: THREE.Object3D;
  paint: { material: THREE.MeshBasicMaterial; color: THREE.Color; opacity: number }[];
  hit: THREE.Object3D;
};

function arrow(color: THREE.ColorRepresentation, length = 1): { root: THREE.Group; paint: Handle["paint"]; hit: THREE.Mesh } {
  const shaftMat = overlay(color, 0.95);
  const tipMat = overlay(color, 1);
  const shaft = new THREE.Mesh(new THREE.CylinderGeometry(0.018, 0.018, length * 0.78, 10), shaftMat);
  shaft.position.y = length * 0.39 + 0.06;
  const tip = new THREE.Mesh(new THREE.ConeGeometry(0.065, 0.2, 28), tipMat);
  tip.position.y = length * 0.78 + 0.16;
  const hit = new THREE.Mesh(new THREE.CylinderGeometry(0.11, 0.11, length, 8), overlay(0x000000, 0));
  hit.position.y = length / 2 + 0.08;
  const root = new THREE.Group();
  root.add(shaft, tip, hit);
  const c = new THREE.Color(color);
  return { root, hit, paint: [shaftMat, tipMat].map((material) => ({ material, color: c.clone(), opacity: material.opacity })) };
}

export class MoveGizmo {
  readonly root = new THREE.Group();
  private handles: Handle[] = [];
  private center: THREE.Group;
  private hovered: GizmoPick | null = null;
  private active: GizmoPick | null = null;

  constructor() {
    for (const axis of [0, 1, 2] as Axis[]) {
      const a = arrow(AXIS_COLORS[axis]);
      alongAxis(a.root, axis);
      this.root.add(a.root);
      this.handles.push({ pick: { kind: "axis", axis }, root: a.root, paint: a.paint, hit: a.hit });
    }
    // Plane handles: a small square between two axes, colored by the axis it moves along neither of.
    for (const normal of [0, 1, 2] as Axis[]) {
      const mat = overlay(AXIS_COLORS[normal], 0.32, THREE.DoubleSide);
      const square = new THREE.Mesh(new THREE.PlaneGeometry(0.2, 0.2), mat);
      const edge = new THREE.LineSegments(
        new THREE.EdgesGeometry(square.geometry),
        new THREE.LineBasicMaterial({ color: AXIS_COLORS[normal], transparent: true, opacity: 0.9, depthTest: false, toneMapped: false, fog: false }),
      );
      square.add(edge);
      const [u, v] = ([0, 1, 2] as Axis[]).filter((k) => k !== normal);
      const at = new THREE.Vector3();
      at.setComponent(u, 0.3);
      at.setComponent(v, 0.3);
      square.position.copy(at);
      if (normal === 0) square.rotation.y = Math.PI / 2;
      if (normal === 1) square.rotation.x = Math.PI / 2;
      this.root.add(square);
      this.handles.push({
        pick: { kind: "plane", normal },
        root: square,
        paint: [{ material: mat, color: new THREE.Color(AXIS_COLORS[normal]), opacity: 0.32 }],
        hit: square,
      });
    }
    // Center grip: a white disc with a dark ring, always facing the camera.
    this.center = new THREE.Group();
    const discMat = overlay("#ffffff", 1, THREE.DoubleSide);
    const ringMat = overlay("#4a4458", 0.95, THREE.DoubleSide);
    const disc = new THREE.Mesh(new THREE.CircleGeometry(0.07, 32), discMat);
    const ring = new THREE.Mesh(new THREE.RingGeometry(0.07, 0.095, 32), ringMat);
    const hit = new THREE.Mesh(new THREE.CircleGeometry(0.13, 16), overlay(0x000000, 0, THREE.DoubleSide));
    this.center.add(disc, ring, hit);
    this.root.add(this.center);
    this.handles.push({
      pick: { kind: "center" },
      root: this.center,
      paint: [
        { material: ringMat, color: new THREE.Color("#4a4458"), opacity: 0.95 },
        { material: discMat, color: new THREE.Color("#ffffff"), opacity: 1 },
      ],
      hit,
    });
    this.root.traverse((o) => (o.renderOrder = 1000));
    this.center.traverse((o) => (o.renderOrder = 1002));
    this.root.visible = false;
  }

  hits() {
    return this.handles.map((h) => h.hit);
  }

  pickOf(object: THREE.Object3D): GizmoPick | null {
    return this.handles.find((h) => h.hit === object)?.pick ?? null;
  }

  setHover(pick: GizmoPick | null) {
    this.hovered = pick;
    this.restyle();
  }

  setActive(pick: GizmoPick | null) {
    this.active = pick;
    this.restyle();
  }

  private restyle() {
    const focus = this.active ?? this.hovered;
    const same = (a: GizmoPick, b: GizmoPick) => JSON.stringify(a) === JSON.stringify(b);
    for (const h of this.handles) {
      const on = !!focus && same(h.pick, focus);
      const dim = !!focus && !on;
      h.root.visible = !this.active || on;
      h.root.scale.setScalar(on ? 1.12 : 1);
      for (const p of h.paint) {
        p.material.color.copy(p.color);
        if (on && h.pick.kind !== "center") p.material.color.lerp(new THREE.Color("#ffffff"), 0.28);
        p.material.opacity = dim ? p.opacity * 0.4 : on && h.pick.kind === "plane" ? 0.62 : p.opacity;
      }
    }
  }

  layout(origin: THREE.Vector3, scale: number, camera: THREE.Camera) {
    this.root.position.copy(origin);
    this.root.scale.setScalar(scale);
    this.center.quaternion.copy(camera.quaternion);
  }
}

/** Geometry of one face of a shape, in scene millimeters. */
export function faceFrame(s: Shape, face: Face) {
  const min = [s.x, s.y, s.z];
  const size = [s.width, s.depth, s.height];
  const center = new THREE.Vector3(...min.map((m, k) => m + size[k] / 2));
  center.setComponent(face.axis, face.sign > 0 ? min[face.axis] + size[face.axis] : min[face.axis]);
  const normal = UNIT[face.axis].clone().multiplyScalar(face.sign);
  // Extents of the face in the plane, ordered to match the quad's local x/y after rotation.
  const extents: [number, number] =
    face.axis === 0 ? [size[2], size[1]] : face.axis === 1 ? [size[0], size[2]] : [size[0], size[1]];
  return { center, normal, extents };
}

function orientQuad(quad: THREE.Object3D, axis: Axis) {
  quad.rotation.set(0, 0, 0);
  if (axis === 0) quad.rotation.y = Math.PI / 2;
  if (axis === 1) quad.rotation.x = -Math.PI / 2;
}

/** A tinted face with an outline: used for hover and for the selected face. */
export class FaceTint {
  readonly root = new THREE.Group();
  private fill: THREE.Mesh;
  private edge: THREE.LineLoop;

  constructor(color: THREE.ColorRepresentation, fillOpacity: number, edgeOpacity: number) {
    this.fill = new THREE.Mesh(
      new THREE.PlaneGeometry(1, 1),
      new THREE.MeshBasicMaterial({ color, transparent: true, opacity: fillOpacity, depthWrite: false, toneMapped: false, fog: false, side: THREE.DoubleSide, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -2 }),
    );
    const loop = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(-0.5, -0.5, 0),
      new THREE.Vector3(0.5, -0.5, 0),
      new THREE.Vector3(0.5, 0.5, 0),
      new THREE.Vector3(-0.5, 0.5, 0),
    ]);
    this.edge = new THREE.LineLoop(loop, new THREE.LineBasicMaterial({ color, transparent: true, opacity: edgeOpacity, depthTest: false, toneMapped: false, fog: false }));
    this.root.add(this.fill, this.edge);
    this.fill.renderOrder = 950;
    this.edge.renderOrder = 951;
    this.root.visible = false;
  }

  show(s: Shape | null, face: Face | null) {
    if (!s || !face) {
      this.root.visible = false;
      return;
    }
    const { center, extents } = faceFrame(s, face);
    this.root.visible = true;
    this.root.position.copy(center);
    orientQuad(this.root, face.axis);
    this.root.scale.set(extents[0], extents[1], 1);
  }
}

/** The push-pull arrow for the selected face. */
export class PushHandle {
  readonly root = new THREE.Group();
  private arrow: ReturnType<typeof arrow>;
  private hovered = false;

  constructor() {
    this.arrow = arrow(ACCENT, 1);
    this.root.add(this.arrow.root);
    this.root.traverse((o) => (o.renderOrder = 1000));
    this.root.visible = false;
  }

  get hit() {
    return this.arrow.hit;
  }

  setHover(on: boolean) {
    if (on === this.hovered) return;
    this.hovered = on;
    this.arrow.root.scale.setScalar(on ? 1.12 : 1);
    for (const p of this.arrow.paint) {
      p.material.color.copy(p.color);
      if (on) p.material.color.lerp(new THREE.Color("#ffffff"), 0.25);
    }
  }

  layout(s: Shape | null, face: Face | null, scale: number) {
    if (!s || !face) {
      this.root.visible = false;
      return;
    }
    const { center } = faceFrame(s, face);
    this.root.visible = true;
    this.root.position.copy(center);
    this.arrow.root.rotation.set(0, 0, 0);
    alongAxis(this.arrow.root, face.axis, face.sign);
    this.root.scale.setScalar(scale);
  }
}

/** Faint outline of where a part started, shown while dragging. */
export class Ghost {
  readonly root: THREE.LineSegments;

  constructor() {
    this.root = new THREE.LineSegments(
      new THREE.EdgesGeometry(new THREE.BoxGeometry(1, 1, 1)),
      new THREE.LineDashedMaterial({ color: "#8b80a3", dashSize: 0.03, gapSize: 0.025, transparent: true, opacity: 0.8, depthTest: false, toneMapped: false, fog: false }),
    );
    this.root.computeLineDistances();
    this.root.renderOrder = 940;
    this.root.visible = false;
  }

  show(s: Shape | null) {
    if (!s) {
      this.root.visible = false;
      return;
    }
    this.root.visible = true;
    this.root.position.set(s.x + s.width / 2, s.y + s.depth / 2, s.z + s.height / 2);
    this.root.scale.set(s.width, s.depth, s.height);
  }
}

/** A long guide line along the axis being dragged. */
export class AxisGuide {
  readonly root: THREE.Line;

  constructor() {
    this.root = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(-1, 0, 0), new THREE.Vector3(1, 0, 0)]),
      new THREE.LineBasicMaterial({ color: "#ffffff", transparent: true, opacity: 0.85, depthTest: false, toneMapped: false, fog: false }),
    );
    this.root.renderOrder = 990;
    this.root.visible = false;
  }

  show(origin: THREE.Vector3 | null, axis: Axis | null, color?: string) {
    if (!origin || axis === null) {
      this.root.visible = false;
      return;
    }
    this.root.visible = true;
    (this.root.material as THREE.LineBasicMaterial).color.set(color ?? AXIS_COLORS[axis]);
    this.root.position.copy(origin);
    this.root.rotation.set(0, 0, 0);
    if (axis === 1) this.root.rotation.z = Math.PI / 2;
    if (axis === 2) this.root.rotation.y = -Math.PI / 2;
    this.root.scale.setScalar(20000);
  }
}
