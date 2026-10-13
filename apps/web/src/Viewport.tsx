// 3D viewport. Renderer, camera and scene are created once; UI changes (chat
// modes, panels, docking) never recreate them. Engine updates only add,
// replace or remove the affected meshes.
//
// Manipulation, the way CAD tools do it:
// • Select a part → a move gizmo at its center: X/Y/Z arrows from one origin,
//   plane handles for two-axis moves, a center grip to slide on the ground.
//   Dragging the part itself also slides it.
// • Hover a face of the selected part → it tints. Click it → the face is
//   selected (highlighted) with one push-pull arrow on its normal. Pulling a
//   minus-side face grows the part backwards; the opposite face stays put.
// • Esc steps back from face to part. Values snap to 1 mm (⇧ 10, ⌥ 0.1),
//   preview live, and commit once on release; Esc mid-drag cancels.

import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { attachTumble, attachWalk, type Tumble } from "./tumble";
import { pcbLooks } from "./pcb3d";
import type { RenderSource } from "./render";
import { type DisplayMode, type SectionPlane, type ViewName, ViewTools } from "./viewTools";
import { ViewToolbar } from "./ViewToolbar";
import type { EntityView, MeshData } from "./engine";
import { BatchLayer } from "./batchLayer";
import {
  AXIS_NAMES,
  ACCENT,
  type Axis,
  AxisGuide,
  type Face,
  FaceTint,
  Ghost,
  type GizmoPick,
  MoveGizmo,
  PushHandle,
  SIZE_NAMES,
  UNIT,
  faceFrame,
} from "./gizmo";
import type { Overlay } from "./intent";
import type { CommentPin, CommentsView, PinAnchor } from "./Comments";
import { EdgeLayer, type EdgePicture } from "./edgeLayer";
import { ConnectorLayer, type ConnectorPicture } from "./connectorLayer";
import { FaceLayer, type FacePicture, type FlatFace, PickedFacesLayer, curvedFaceAt, flatFaceAt } from "./faceLayer";
import { SketchLayer, type SketchPicture } from "./sketchLayer";
import { type Snap, snapSketch } from "./sketchSnap";
import { DraftLayer, type DraftPicture } from "./draftLayer";
import { type ScanLayer, buildScanLayer, disposeScanLayer } from "./scanLayer";
import { type Plane, normalOf, toWorld } from "./sketchModel";
import {
  type Frac,
  type LiveEdit,
  SNAP_FRACS,
  type Shape,
  fmt,
  material,
  assemblyParts,
  subPartKey,
  partMaterial,
  instance,
  isDefinition,
  occurrences,
  type Look,
  materialLook,
  materialRef,
  originOffset,
  partName,
  program,
  pivotName,
  pivotOf,
  pivotPoint,
  shapeOf,
  placedOf,
  applyT,
  invertT,
  type Transform,
  type V3,
} from "./model";

export type { ViewName, DisplayMode, SectionPlane };

/**
 * Driving a mechanism by hand: pressing on one of `groups` of part `id`
 * and dragging turns (or slides) the drive joint at `at` along `axis`
 * (part coordinates); `onDelta` gets each step in degrees (or mm).
 */
export type MotionGrab = {
  id: string;
  groups: number[];
  at: [number, number, number];
  axis: [number, number, number];
  slide: boolean;
  onStart: () => void;
  onDelta: (d: number) => void;
};

/**
 * Where to draw a pressure's arrows: up to `count` triangles spread over the
 * face (the largest of each run of triangles, in the order they come), each
 * arrow at its middle pointing in along its normal.
 */
function pressureArrows(triangles: number[], count: number): { at: THREE.Vector3; dir: THREE.Vector3 }[] {
  const n = Math.floor(triangles.length / 9);
  const run = Math.max(1, Math.ceil(n / count));
  const out: { at: THREE.Vector3; dir: THREE.Vector3 }[] = [];
  const p = (t: number, k: number) => new THREE.Vector3(triangles[9 * t + 3 * k], triangles[9 * t + 3 * k + 1], triangles[9 * t + 3 * k + 2]);
  for (let first = 0; first < n; first += run) {
    let best: { at: THREE.Vector3; dir: THREE.Vector3; area: number } | null = null;
    for (let t = first; t < Math.min(n, first + run); t++) {
      const [a, b, c] = [p(t, 0), p(t, 1), p(t, 2)];
      const cross = new THREE.Vector3().subVectors(b, a).cross(new THREE.Vector3().subVectors(c, a));
      const area = cross.length();
      if (area > 0 && (!best || area > best.area)) best = { at: a.add(b).add(c).divideScalar(3), dir: cross.normalize().negate(), area };
    }
    if (best) out.push(best);
  }
  return out;
}

/** A stress study as the view shows it (part coordinates, mm). */
export type StressView = {
  /** `pressure`: a load pushing every bit of its face straight in (drawn as arrows square to it all over). */
  faces: { kind: "support" | "load" | "hold" | "heat"; triangles: number[]; point: [number, number, number]; force?: [number, number, number]; pressure?: boolean }[];
  result?: {
    cell: number;
    origin: [number, number, number];
    cells: [[number, number, number], number][];
    displacements: [[number, number, number], [number, number, number]][];
    max_stress: number;
    max_displacement: number;
    /** Several parts: which part each cube and each displacement is of. */
    cell_parts?: number[];
    node_parts?: number[];
  };
  /** Where parts studied together touch: pressure on each cube face where they meet, and where they came apart. */
  contact?: ContactView;
  /** Show the contact pressure instead of the stress (the parts see-through). */
  layer?: "stress" | "contact";
  /** How many times to exaggerate the displacement (0: as built). */
  deform?: number;
  /** Swing back and forth through the displacement (a vibration mode). */
  oscillate?: boolean;
  /** The most stressed spot, re-solved finer (part coordinates); `singular`: a sharp inside corner. */
  peak?: { at: [number, number, number]; singular: boolean };
  /** Spots to mark (part coordinates): the largest stress, the largest movement. */
  markers?: { at: [number, number, number]; color: string; label?: string }[];
  /** Cut the result open: cubes past `at` (mm) along the axis are left out, so the inside shows. */
  section?: { axis: 0 | 1 | 2; at: number } | null;
};

export type ContactView = {
  /** The cube on the first part's side, which way the face looks (0 −x, 1 +x, 2 −y, 3 +y, 4 −z, 5 +z), and its pressure (MPa). */
  faces: { cell: [number, number, number]; dir: number; pressure: number }[];
  /** Touching faces that came apart: x, y, z (mm) and the gap (mm). */
  opened: [number, number, number, number][];
  max_pressure: number;
};

/** Each cube face: the neighbour that would hide it, and its four corners (outward winding). */
const CUBE_FACES: { n: [number, number, number]; c: [number, number, number][] }[] = [
  { n: [-1, 0, 0], c: [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]] },
  { n: [1, 0, 0], c: [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]] },
  { n: [0, -1, 0], c: [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]] },
  { n: [0, 1, 0], c: [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]] },
  { n: [0, 0, -1], c: [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]] },
  { n: [0, 0, 1], c: [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]] },
];

/** The contact layer: each face where parts meet, coloured by its pressure, and a marker where touching faces came apart. */
function contactMeshes(r: NonNullable<StressView["result"]>, c: ContactView): THREE.Object3D[] {
  const pos: number[] = [];
  const col: number[] = [];
  const color = new THREE.Color();
  const top = c.max_pressure > 0 ? c.max_pressure : 1;
  for (const f of c.faces) {
    // Pulling (a glued face in tension) shows as the bottom of the scale.
    heat(Math.max(f.pressure, 0) / top, color);
    const q = CUBE_FACES[f.dir].c.map((o) => [0, 1, 2].map((k) => r.origin[k] + (f.cell[k] + o[k]) * r.cell));
    for (const v of [q[0], q[1], q[2], q[0], q[2], q[3]]) {
      pos.push(...v);
      col.push(color.r, color.g, color.b);
    }
  }
  const out: THREE.Object3D[] = [];
  if (pos.length) {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    g.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
    const m = new THREE.Mesh(g, new THREE.MeshBasicMaterial({ vertexColors: true, side: THREE.DoubleSide, polygonOffset: true, polygonOffsetFactor: -2, polygonOffsetUnits: -2 }));
    m.renderOrder = 958;
    out.push(m);
  }
  if (c.opened.length) {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(c.opened.flatMap((o) => [o[0], o[1], o[2]]), 3));
    const pts = new THREE.Points(g, new THREE.PointsMaterial({ color: "#b04fd6", size: 6, sizeAttenuation: false, depthTest: false, transparent: true, opacity: 0.9 }));
    pts.renderOrder = 962;
    out.push(pts);
  }
  return out;
}

/** Blue (low) through green and yellow to red (high). */
function heat(t: number, out: THREE.Color) {
  const stops = ["#2b59c3", "#2bb3c3", "#4fc35a", "#f2c230", "#e0452f"].map((c) => new THREE.Color(c));
  const x = Math.max(0, Math.min(1, t)) * (stops.length - 1);
  const i = Math.min(Math.floor(x), stops.length - 2);
  return out.copy(stops[i]).lerp(stops[i + 1], x - i);
}

/** A result cut open: the cubes whose centres lie past the cut left out, so the faces left show the inside. */
function cutAway(r: NonNullable<StressView["result"]>, section: StressView["section"]): NonNullable<StressView["result"]> {
  if (!section) return r;
  const keep = r.cells.map(([c]) => r.origin[section.axis] + (c[section.axis] + 0.5) * r.cell <= section.at);
  return { ...r, cells: r.cells.filter((_, i) => keep[i]), cell_parts: r.cell_parts?.length ? r.cell_parts.filter((_, i) => keep[i]) : r.cell_parts };
}

/** The stress map: the outside faces of the solved cubes, coloured by stress, displaced if asked (see-through when `ghost`). */
function stressMesh(r: NonNullable<StressView["result"]>, deform: number, ghost = false): THREE.Mesh {
  const key = (i: number, j: number, k: number) => `${i},${j},${k}`;
  const filled = new Map(r.cells.map(([c, sv]) => [key(c[0], c[1], c[2]), sv]));
  // Parts studied together move apart where they slide or open: displacements per part.
  const several = !!r.cell_parts?.length && !!r.node_parts?.length;
  const moved = new Map(r.displacements.map(([c, d], n) => [`${several ? r.node_parts![n] : 0}:${key(c[0], c[1], c[2])}`, d]));
  const at = (part: number, i: number, j: number, k: number): [number, number, number] => {
    const d = moved.get(`${part}:${key(i, j, k)}`) ?? [0, 0, 0];
    return [r.origin[0] + i * r.cell + d[0] * deform, r.origin[1] + j * r.cell + d[1] * deform, r.origin[2] + k * r.cell + d[2] * deform];
  };
  const pos: number[] = [];
  const col: number[] = [];
  const color = new THREE.Color();
  const top = r.max_stress || 1;
  for (const [n, [[i, j, k], sv]] of r.cells.entries()) {
    heat(sv / top, color);
    const part = several ? r.cell_parts![n] : 0;
    for (const f of CUBE_FACES) {
      if (filled.has(key(i + f.n[0], j + f.n[1], k + f.n[2]))) continue;
      const q = f.c.map((o) => at(part, i + o[0], j + o[1], k + o[2]));
      for (const v of [q[0], q[1], q[2], q[0], q[2], q[3]]) {
        pos.push(...v);
        col.push(color.r, color.g, color.b);
      }
    }
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
  g.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
  g.computeVertexNormals();
  const m = new THREE.Mesh(g, ghost ? new THREE.MeshLambertMaterial({ color: "#9aa3ad", transparent: true, opacity: 0.16, depthWrite: false }) : new THREE.MeshLambertMaterial({ vertexColors: true }));
  m.castShadow = !ghost;
  return m;
}

/** A moving body: turned about an axis, or placed by a pose (a point as drawn goes to q·p + t). */
export type TurnBody =
  | { groups: number[]; at: [number, number, number]; axis: [number, number, number]; degrees: number }
  | { groups: number[]; position: [number, number, number]; quaternion: [number, number, number, number] };

/** Options for showing a part turned: still parts see-through, and trail lines (part coordinates, from → to) behind moved bodies. */
export type TurnOptions = { ghost?: boolean; trails?: [[number, number, number], [number, number, number]][] };

/** A part drawn turning: its moving bodies split out of its mesh, each on a pivot. */
type Turning = {
  id: string;
  body: THREE.Mesh;
  original: THREE.BufferGeometry;
  ghosted: { m: THREE.MeshStandardMaterial; opacity: number; transparent: boolean; depthWrite: boolean }[];
  pivots: { pivot: THREE.Group; mesh: THREE.Mesh; key: string; groups: number[] }[];
  trails: THREE.LineSegments | null;
};

/**
 * How a turning (or exploded) part has moved one of its bodies (a mesh
 * group) within the part, or null when that body sits where it was built.
 */
function bodyPose(m: Turning | null, id: string, group: number): THREE.Matrix4 | null {
  const p = m?.id === id ? m.pivots.find((q) => q.groups.includes(group)) : undefined;
  if (!p) return null;
  p.pivot.updateMatrix();
  p.mesh.updateMatrix();
  return p.pivot.matrix.clone().multiply(p.mesh.matrix);
}

/** Takes away a turning part's trail lines. */
function clearTrails(m: Turning) {
  if (!m.trails) return;
  m.body.remove(m.trails);
  m.trails.geometry.dispose();
  (m.trails.material as THREE.Material).dispose();
  m.trails = null;
}

/** Puts see-through still parts back as they were. */
function unghost(m: Turning) {
  for (const g of m.ghosted) {
    g.m.opacity = g.opacity;
    g.m.transparent = g.transparent;
    g.m.depthWrite = g.depthWrite;
    g.m.needsUpdate = true;
  }
  m.ghosted = [];
}

/** A geometry drawing only some groups of another, sharing its buffers. */
function groupsOf(g: THREE.BufferGeometry, keep: (i: number) => boolean) {
  const out = new THREE.BufferGeometry();
  if (g.index) out.setIndex(g.index);
  for (const [name, attr] of Object.entries(g.attributes)) out.setAttribute(name, attr);
  g.groups.forEach((gr, i) => keep(i) && out.addGroup(gr.start, gr.count, gr.materialIndex));
  // Bounds of only what it draws, so framing and picking fit the bodies kept.
  const pos = g.getAttribute("position");
  const box = new THREE.Box3();
  const p = new THREE.Vector3();
  for (const gr of out.groups) for (let k = gr.start; k < gr.start + gr.count; k++) box.expandByPoint(p.fromBufferAttribute(pos, g.index ? g.index.getX(k) : k));
  out.boundingBox = box;
  out.boundingSphere = box.isEmpty() ? new THREE.Sphere(new THREE.Vector3(), -1) : box.getBoundingSphere(new THREE.Sphere());
  return out;
}

export type ViewportHandle = {
  fit: () => void;
  /** Frames one item: a part, or an instance at any depth inside one. */
  frame: (id: string, glide?: boolean) => void;
  /** Turns the camera to look straight at a sketch plane, centred on what is drawn (or back to normal with null). */
  faceSketch: (plane: Plane | null, focus?: { center: [number, number]; size: number }) => void;
  /** Leaving a sketch: glides from looking straight at it to a three-quarter view, everything framed. */
  leaveSketch: () => void;
  /**
   * Shows a part's mechanism turned: each body (its mesh groups) rotated by
   * `degrees` about its axis through `at` (mm, program coordinates). Null puts it back.
   */
  turn: (id: string, bodies: TurnBody[] | null, options?: TurnOptions) => void;
  /** Lets a moving part be grabbed and dragged to drive a mechanism (null stops it). */
  grabMotion: (spec: MotionGrab | null) => void;
  /** Shows a stress study on a part: its supports and loads, and (when solved) the stress map. Null clears it. */
  stress: (id: string, view: StressView | null) => void;
  view: (name: ViewName) => void;
  /** The view's tools (projection, display mode, section, measure), for commands from elsewhere. */
  tools: () => ViewTools | null;
  zoom: (factor: number) => void;
  setWireframe: (on: boolean) => void;
  setTurntable: (on: boolean) => void;
  snapshot: () => string | null;
  /** What a photoreal render starts from: the parts as shown and the live camera. */
  renderSource: () => RenderSource | null;
  resize: () => void;
  /** Glides to a comment pin, turning to face the surface it is on. */
  framePin: (id: string) => void;
};

type Props = {
  entities: ReadonlyMap<string, EntityView>;
  selected: string | null;
  /** Teammates online: where each points (world mm) and what they have selected, in their colour. */
  teammates?: { engine: string; name: string; color: string; selected: string | null; pointer: [number, number, number] | null }[];
  /** Where the pointer is over the model (world mm), or null off it; for teammates to see. */
  onHover3D?: (p: [number, number, number] | null) => void;
  /** Comment pins on the model, the open thread's card, and comment mode. */
  comments?: CommentsView | null;
  /** Other items picked with it (Shift-click): outlined, and moved with it while it is dragged. */
  also?: string[];
  live: LiveEdit | null;
  /** Design-intent visuals: clearances, collisions, envelopes. */
  overlays: Overlay[];
  /** Choosing where the selected part moves and rotates from. */
  pivotMode: boolean;
  /** Pivot being edited in the properties panel, shown live. */
  pivotLive: Frac | null;
  /** A pivot was chosen (fractions of the box), or null to leave pivot mode. `keepMode` stays in pivot mode. */
  onPickPivot: (frac: Frac | null, keepMode?: boolean) => void;
  /** add: Shift-click (grow or shrink the selection); single: Alt-click (one item, not its whole group). */
  onSelect: (id: string | null, add?: boolean, single?: boolean) => void;
  /** Part of the selected assembly that is picked (by name), if any. */
  selectedPart: string | null;
  /** Clicking an already selected assembly picks the part under the pointer. */
  onSelectPart: (name: string | null) => void;
  /** Other parts of the selected assembly whose faces can be picked too (a study taking them in). */
  pickableParts?: string[] | null;
  /** Items (or instances at any depth) hidden from view. */
  hidden: ReadonlySet<string>;
  /** The item (or "id#piece") the pointer is over in the components tree: lit up here. */
  hovered?: string | null;
  /** Show only this item or instance, if set. */
  isolated: string | null;
  /** Paint instances by their sale status instead of their materials. */
  colorBy: "material" | "status";
  /** Draft objects to draw (lines, hatches, text, dimensions), with what is selected. */
  draft?: DraftPicture | null;
  onSelectDraft?: (id: string) => void;
  /** Colours layers give items (an item, or an instance at any depth, by id). */
  tints?: ReadonlyMap<string, string>;
  /** The sketch being edited, if any: clicks then draw on its plane. */
  sketch: SketchPicture | null;
  onSketchPointer: (e: SketchPointer) => void;
  onEditDimension: (key: string, x: number, y: number) => void;
  /** Picking edges of the selected part to round or bevel: clicks pick edges ("part:edge"). */
  edges: EdgePicture | null;
  onEdgeClick: (key: string) => void;
  /** A flat face of a program part picked for push and pull (part coordinates), and its drag. */
  programFace: ProgramFace | null;
  onProgramFace: (face: ProgramFace | null, why?: "curved") => void;
  onFaceDrag: (distance: number) => void;
  onFaceDragEnd: (commit: boolean, distance: number) => void;
  /** Faces picked for a shell or draft (clicks then pick faces through onProgramFace), and a draft's pull. */
  facePicks: FacePicture | null;
  /** Live change from a drag (absolute millimeters for the changed fields). */
  onDrag: (id: string, patch: Partial<Shape>) => void;
  /** Drag finished: commit or cancel. */
  onDragEnd: (commit: boolean) => void;
  /** Connectors offered for a joint's ends: clicks pick one instead of selecting. */
  connectors?: ConnectorPicture | null;
  onConnectorPick?: (key: string) => void;
  /** Instances drawn at these placements instead of their own (a joint being scrubbed or swept). */
  poses?: ReadonlyMap<string, Transform> | null;
  /** An instance turned with the gizmo's rings: its new position and rotation (false: refused, put it back). */
  onTurnInstance?: (id: string, at: V3, rotation: V3) => Promise<boolean>;
  /** Lines and dots drawn over the model (project mm): exploded trails, a robot's reach and path, picked geometry. */
  overlay?: AssemblyOverlay | null;
  /** A scan shown in place: its points or mesh, a deviation map, problem markers. */
  scan?: ScanLayer | null;
  /** Picking geometry for a mate: a click on a part reports where and which face (project mm), instead of selecting. */
  onGeometryPick?: ((hit: GeometryHit) => void) | null;
  /** A selection box closed on these items (add: Shift held, adding to what is selected). */
  onSelectMany?: (ids: string[], add: boolean) => void;
};

/** What an assembly tool draws over the model, in project millimetres. */
export type AssemblyOverlay = {
  lines?: { points: V3[]; color?: string; dashed?: boolean }[];
  points?: { at: V3[]; color?: string; size?: number }[];
};

/** A click on a part while picking geometry: the instance path, the point and the face's normal there (project mm). */
export type GeometryHit = { path: string[]; shape: string | null; point: V3; normal: V3 };

/** A flat face of a part made by a program, and which part of an assembly it is on. */
export type ProgramFace = FlatFace & { id: string; part: string | null };

/** A pointer event on the sketch plane: where (mm, plane coordinates), and what it snapped to. */
export type SketchPointer = {
  type: "down" | "move" | "up" | "double";
  u: number;
  v: number;
  point: number | null;
  curve: number | null;
  /** Grid spacing at the current zoom (mm). */
  grid: number;
  shift: boolean;
  /** What the place snapped to (a midpoint, a crossing, onto a curve…), if anything. */
  snap?: Snap | null;
};

const MM = 1000; // engine meters -> scene millimeters
/** Most points kept in one markup stroke, and the screen distance (px) between them. */
const STROKE_POINTS = 400;
const STROKE_STEP = 3;
// Clicks pass through what is not drawn: a piece switched off in the components
// tree (its material hidden) is not hit, so the part behind it is picked.
const meshRaycast = THREE.Mesh.prototype.raycast;
THREE.Mesh.prototype.raycast = function (raycaster, intersects) {
  const before = intersects.length;
  meshRaycast.call(this, raycaster, intersects);
  const mat = this.material;
  if (Array.isArray(mat) ? !mat.some((m) => m.visible === false) : mat.visible !== false) return;
  let w = before;
  for (let i = before; i < intersects.length; i++) {
    const hit = intersects[i];
    const m = Array.isArray(mat) ? mat[hit.face?.materialIndex ?? 0] : mat;
    if (m && m.visible !== false) intersects[w++] = hit;
  }
  intersects.length = w;
};

const OUTLINE = new THREE.Color("#8b66ef");
/** Pre-selection glow for what the pointer is over in the components tree. */
const HOVER = "#9a7cf0";
const HOVER_GLOW = 0.5;
const POS_KEYS = ["x", "y", "z"] as const;
const SIZE_KEYS = ["width", "depth", "height"] as const;
const GIZMO_SCALE = 0.085; // of camera distance
const MIN_SIZE = 0.1;

type Part = {
  group: THREE.Group;
  body: THREE.Mesh;
  outline: THREE.LineSegments;
  mesh: unknown; // the engine mesh this was built from (identity)
  look: string;
  base: Shape;
  /** AI-made shapes are never scaled by box edits; boxes are. */
  isProgram: boolean;
  /** Everything drawn for this item: its body, or every shape inside an instance. */
  meshes: THREE.Mesh[];
  /** Instances share their components' geometry: it is released, not disposed. */
  shared: boolean;
};

/** A point other parts offer for magnetic snapping while moving. */
type SnapTarget = { at: THREE.Vector3; label: string };

type MoveDrag = { targets: SnapTarget[] };

type Drag =
  | ({ kind: "axis"; id: string; axis: Axis; origin: THREE.Vector3; startParam: number; start: Shape } & MoveDrag)
  | ({ kind: "plane"; id: string; normal: Axis; origin: THREE.Vector3; startHit: THREE.Vector3; start: Shape } & MoveDrag)
  | ({ kind: "ground"; id: string; offset: THREE.Vector2; start: Shape; downX: number; downY: number; moved: boolean } & MoveDrag)
  | { kind: "push"; id: string; face: Face; origin: THREE.Vector3; startParam: number; start: Shape }
  | { kind: "facePush"; id: string; origin: THREE.Vector3; normal: THREE.Vector3; startParam: number; start: Shape; distance: number }
  | ({ kind: "pivot"; id: string; pick: GizmoPick; origin: THREE.Vector3; startParam: number; startHit: THREE.Vector3; plane: THREE.Plane | null; start: Shape; moved: boolean } & MoveDrag)
  | { kind: "ring"; id: string; axis: Axis; origin: THREE.Vector3; startAngle: number; last: number; turned: number; start: Shape };

const SNAP_PIXELS = 12;

type Scene = {
  renderer: THREE.WebGLRenderer;
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  controls: OrbitControls;
  /** Left-drag turns the view with no stop at the poles. */
  tumble: Tumble;
  sun: THREE.DirectionalLight;
  parts: Map<string, Part>;
  gizmo: MoveGizmo;
  push: PushHandle;
  overlayRoot: THREE.Group;
  overlayLabels: { el: HTMLDivElement; at: THREE.Vector3 }[];
  ground: THREE.Mesh;
  grid: THREE.GridHelper;
  /** Grid cell (mm) the ground was last sized for. */
  gridCell: number;
  wireframe: boolean;
  invalidate: () => void;
  /** A camera move in progress (see `glideTo`). */
  flight: Flight | null;
  /** Everything placed, as a sphere: zoom walks on and fog starts by its size. */
  extent: THREE.Sphere;
};

type View3 = { position: THREE.Vector3; target: THREE.Vector3; up: THREE.Vector3 };
/** A glide; its clock starts on its first drawn frame (null until then). */
type Flight = { from: View3; to: View3; start: number | null; ms: number };

/**
 * Moves the camera to a view over a moment, swinging round the target rather
 * than cutting through it, so the eye keeps track of where it is. Instant
 * when reduced motion is asked for. Grabbing the view ends it.
 */
function glideTo(t: Scene, to: View3, ms = 700) {
  if (reducedMotion()) {
    t.flight = null;
    t.camera.position.copy(to.position);
    t.controls.target.copy(to.target);
    t.camera.up.copy(to.up);
    settleCamera(t);
    return;
  }
  t.flight = { from: { position: t.camera.position.clone(), target: t.controls.target.clone(), up: t.camera.up.clone() }, to, start: null, ms };
  t.invalidate();
}

/** One step of a glide; true while it is still moving. */
function stepFlight(t: Scene, now: number): boolean {
  const f = t.flight;
  if (!f) return false;
  // Timed from the first frame that draws it: a glide asked for in a
  // background tab (no frames there) plays when the tab is shown, instead of
  // having already run out unseen.
  f.start ??= now;
  const k = Math.max(0, Math.min(1, (now - f.start) / f.ms));
  const e = k < 0.5 ? 4 * k * k * k : 1 - (-2 * k + 2) ** 3 / 2;
  const target = f.from.target.clone().lerp(f.to.target, e);
  const a = f.from.position.clone().sub(f.from.target);
  const b = f.to.position.clone().sub(f.to.target);
  const turn = new THREE.Quaternion().slerpQuaternions(new THREE.Quaternion(), new THREE.Quaternion().setFromUnitVectors(a.clone().normalize(), b.clone().normalize()), e);
  const distance = THREE.MathUtils.lerp(a.length(), b.length(), e);
  t.controls.target.copy(target);
  t.camera.position.copy(target).add(a.normalize().applyQuaternion(turn).multiplyScalar(distance));
  const up = f.from.up.clone().lerp(f.to.up, e);
  t.camera.up.copy(up.lengthSq() > 1e-9 ? up.normalize() : f.to.up);
  if (k >= 1) t.flight = null;
  settleCamera(t);
  return k < 1;
}

/** Shown part meshes whose box holds `eye` (by a margin), drawn last first. */
function enclosing(t: Scene, eye: THREE.Vector3): THREE.Mesh[] {
  const out: THREE.Mesh[] = [];
  const box = new THREE.Box3();
  for (const part of t.parts.values()) {
    if (!part.group.visible) continue;
    for (const m of part.meshes) {
      if (!m.visible) continue;
      const g = m.geometry;
      // Most parts are nowhere near the eye: a sphere says so before a box is placed.
      if (!g.boundingSphere) g.computeBoundingSphere();
      const s = g.boundingSphere!;
      const e = m.matrixWorld.elements;
      const dx = e[12] - eye.x, dy = e[13] - eye.y, dz = e[14] - eye.z;
      const reach = (s.radius + s.center.length()) * m.matrixWorld.getMaxScaleOnAxis();
      if (dx * dx + dy * dy + dz * dz > reach * reach * 1.01) continue;
      if (!g.boundingBox) g.computeBoundingBox();
      box.copy(g.boundingBox!).applyMatrix4(m.matrixWorld);
      // Inside by more than a hair: grazing a face from outside does not hide it.
      const margin = Math.min(0.2, box.getSize(new THREE.Vector3()).length() * 0.002);
      if (box.expandByScalar(-margin).containsPoint(eye)) out.push(m);
    }
  }
  return out;
}

/** Near plane follows the scale, so a tower and a bolt both render without z-fighting. */
function settleCamera(t: Scene) {
  const distance = t.camera.position.distanceTo(t.controls.target);
  t.camera.near = Math.max(0.1, distance / 5000);
  t.camera.updateProjectionMatrix();
  t.controls.update();
}

function lookFor(e: EntityView, entities: ReadonlyMap<string, EntityView>) {
  const ref = materialRef(e);
  const mat = ref ? entities.get(ref) : undefined;
  return materialLook(mat ? material(mat)?.name : undefined);
}

/** One look per assembly part (or one for a single solid). */
function looksFor(e: EntityView, entities: ReadonlyMap<string, EntityView>): Look[] {
  const parts = assemblyParts(e);
  if (!parts.length) return [lookFor(e, entities)];
  return parts.map((p) => {
    const mat = partMaterial(e, p.material, entities);
    return materialLook(mat ? material(mat)?.name : undefined);
  });
}

const lookKey = (e: EntityView, entities: ReadonlyMap<string, EntityView>) => JSON.stringify(looksFor(e, entities));

function surface(look: Look, wireframe: boolean) {
  const glass = look.opacity !== undefined && look.opacity < 1;
  const m = new THREE.MeshStandardMaterial({
    ...look,
    wireframe,
    transparent: glass,
    opacity: look.opacity ?? 1,
    depthWrite: !glass,
    side: glass ? THREE.DoubleSide : THREE.FrontSide,
    polygonOffset: true,
    polygonOffsetFactor: 1,
    polygonOffsetUnits: 1,
  });
  m.userData.look = look;
  return m;
}

/** Scene geometry (mm) from an engine mesh (m). */
function makeGeometry(mesh: MeshData): THREE.BufferGeometry {
  const indexed = new THREE.BufferGeometry();
  const positions = Float32Array.from(mesh.positions);
  for (let i = 0; i < positions.length; i++) positions[i] *= MM;
  indexed.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  indexed.setIndex(new THREE.BufferAttribute(mesh.indices instanceof Uint32Array ? mesh.indices : Uint32Array.from(mesh.indices), 1));
  if (mesh.normals?.length) {
    // Smooth on curves, crisp at sharp edges: the engine already split the normals.
    indexed.setAttribute("normal", new THREE.BufferAttribute(mesh.normals instanceof Float32Array ? mesh.normals : Float32Array.from(mesh.normals), 3));
    return indexed;
  }
  const geometry = indexed.toNonIndexed();
  geometry.computeVertexNormals();
  indexed.dispose();
  return geometry;
}

/** A component's geometry, built once and shared by all its instances. */
const sharedGeometry = new Map<MeshData, { geometry: THREE.BufferGeometry; users: number }>();
function acquireGeometry(mesh: MeshData, groups: { index_start: number; index_count: number }[]) {
  let entry = sharedGeometry.get(mesh);
  if (!entry) {
    const geometry = makeGeometry(mesh);
    groups.forEach((p, i) => geometry.addGroup(p.index_start, p.index_count, i));
    entry = { geometry, users: 0 };
    sharedGeometry.set(mesh, entry);
  }
  entry.users++;
  return entry.geometry;
}
function releaseGeometry(geometry: THREE.BufferGeometry) {
  for (const [mesh, entry] of sharedGeometry) {
    if (entry.geometry !== geometry) continue;
    if (--entry.users <= 0) {
      geometry.dispose();
      sharedGeometry.delete(mesh);
    }
    return;
  }
}

const meshIds = new WeakMap<object, number>();
let nextMeshId = 1;
const meshId = (m: object) => meshIds.get(m) ?? (meshIds.set(m, nextMeshId++), nextMeshId - 1);

/** What an instance draws; it is rebuilt when this changes. */
function instanceKey(e: EntityView, entities: ReadonlyMap<string, EntityView>) {
  return occurrences(e.id, entities)
    .map((o) => {
      const def = entities.get(o.shape);
      const mesh = o.configured?.mesh ?? def?.mesh;
      return `${o.shape}@${mesh ? meshId(mesh) : 0}:${def ? lookKey(def, entities) : ""}:${o.transform.at.join(",")},${o.transform.q.join(",")}:${o.path.join("/")}`;
    })
    .join("|");
}

/** An instance: every shape inside it, each drawn with its component's shared geometry. */
function buildInstance(e: EntityView, entities: ReadonlyMap<string, EntityView>, wireframe: boolean, key: string): Part | null {
  const group = new THREE.Group();
  const meshes: THREE.Mesh[] = [];
  for (const o of occurrences(e.id, entities)) {
    const def = entities.get(o.shape);
    // An instance in a configuration draws that configuration's mesh.
    const meshData = o.configured?.mesh ?? def?.mesh;
    if (!def || !meshData) continue;
    const parts = o.configured ? o.configured.stats.parts ?? [] : assemblyParts(def);
    const geometry = acquireGeometry(meshData, parts);
    const materials = looksFor(def, entities).map((l) => surface(l, wireframe));
    const mesh = new THREE.Mesh(geometry, parts.length ? materials : materials[0]);
    mesh.position.set(...o.transform.at);
    mesh.quaternion.set(...o.transform.q);
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    mesh.userData = { id: e.id, path: o.path, shape: o.shape };
    // Drawn as a GPU instance with every other mesh of its geometry and look (see batchLayer).
    BatchLayer.adopt(mesh);
    group.add(mesh);
    meshes.push(mesh);
  }
  if (!meshes.length) return null;
  const outline = new THREE.LineSegments(new THREE.BufferGeometry(), new THREE.LineBasicMaterial({ color: OUTLINE }));
  outline.visible = false;
  return { group, body: meshes[0], outline, mesh: key, look: key, base: shapeOf(e), isProgram: true, meshes, shared: true };
}

function buildPart(e: EntityView, entities: ReadonlyMap<string, EntityView>, wireframe: boolean): Part | null {
  if (!e.mesh) return null;
  const geometry = makeGeometry(e.mesh);
  const isProgram = !!program(e);

  // Polygon offset pushes faces back so outlines and face tints are never hidden by them.
  // Assemblies draw each part with its own material (one geometry group per part).
  const looks = looksFor(e, entities);
  const parts = assemblyParts(e);
  parts.forEach((p, i) => geometry.addGroup(p.index_start, p.index_count, i));
  const materials = looks.map((l) => surface(l, wireframe));
  const body = new THREE.Mesh(geometry, parts.length ? materials : materials[0]);
  body.castShadow = true;
  body.receiveShadow = true;
  body.userData.id = e.id;
  const outline = new THREE.LineSegments(
    new THREE.EdgesGeometry(geometry, isProgram ? 35 : 1),
    new THREE.LineBasicMaterial({ color: OUTLINE, transparent: true, opacity: 0.95, toneMapped: false, fog: false }),
  );
  outline.visible = false;
  const group = new THREE.Group();
  group.add(body, outline);
  return { group, body, outline, mesh: e.mesh, look: lookKey(e, entities), base: shapeOf(e), isProgram, meshes: [body], shared: false };
}

const bodyMaterials = (body: THREE.Mesh) => (Array.isArray(body.material) ? body.material : [body.material]) as THREE.MeshStandardMaterial[];

/**
 * Sizes the ground, grid and far plane to the scene: a bolt gets a 20 mm
 * grid, a 150 m tower a 10 m one. The grid is rebuilt only when its cell changes.
 */
function fitGround(t: Scene, bounds: THREE.Box3) {
  const size = bounds.getSize(new THREE.Vector3());
  const extent = Math.max(size.x, size.y, size.z, 100);
  const raw = (extent * 4) / 300;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const cell = Math.max(20, [1, 2, 5, 10].map((m) => m * magnitude).find((c) => c >= raw)!);
  if (cell !== t.gridCell) {
    const old = t.grid;
    const grid = new THREE.GridHelper(cell * 300, 300, 0xc7cbd6, 0xd9dce4);
    grid.rotation.x = Math.PI / 2;
    grid.position.z = old.position.z;
    (grid.material as THREE.Material).transparent = true;
    (grid.material as THREE.Material).opacity = 0.38;
    t.scene.remove(old);
    old.geometry.dispose();
    (old.material as THREE.Material).dispose();
    t.scene.add(grid);
    t.grid = grid;
    t.gridCell = cell;
  }
  // The ground reaches past where the fog closes in, so its edge never shows.
  const scale = Math.max(1, (extent * 60) / 40000);
  t.ground.scale.set(scale, scale, 1);
  const far = Math.max(400000, extent * 60);
  if (t.camera.far !== far) {
    t.camera.far = far;
    t.camera.updateProjectionMatrix();
  }
}

/**
 * Frames every part (or only `only`'s meshes), keeping the viewing direction
 * (or the one a glide is heading for, or `direction`); gliding there if asked.
 */
function fitView(t: Scene, only?: string, glide = false, direction?: THREE.Vector3, up?: THREE.Vector3) {
  if (t.parts.size === 0) return;
  // Only what is shown: an isolated floor frames itself, not the whole tower.
  const bounds = new THREE.Box3();
  for (const [id, part] of t.parts) {
    if (!part.group.visible) continue;
    for (const m of part.meshes) {
      if (!m.visible) continue;
      if (only && id !== only && !(m.userData.path as string[] | undefined)?.includes(only)) continue;
      bounds.expandByObject(m);
    }
  }
  if (bounds.isEmpty()) return;
  const c = bounds.getCenter(new THREE.Vector3());
  const radius = bounds.getBoundingSphere(new THREE.Sphere()).radius;
  const distance = (radius / Math.sin(THREE.MathUtils.degToRad(t.camera.fov / 2))) * 1.2;
  const heading = t.flight ? t.flight.to : null;
  const dir = (direction ?? (heading ? heading.position.clone().sub(heading.target) : t.camera.position.clone().sub(t.controls.target))).normalize();
  const view = { position: c.clone().addScaledVector(dir, distance), target: c, up: (up ?? heading?.up ?? t.camera.up).clone() };
  if (glide) return glideTo(t, view);
  t.flight = null;
  t.camera.position.copy(view.position);
  t.controls.target.copy(view.target);
  t.camera.up.copy(view.up);
  settleCamera(t);
}

/** Sale status colours for a stacking plan. */
export const STATUS_COLORS: Record<string, string> = {
  available: "#5fb98a",
  reserved: "#e8b04a",
  sold: "#df6f63",
  hotel: "#5b8fd8",
  operator: "#5b8fd8",
  amenity: "#a07fd9",
};

export function statusColor(status: string | undefined) {
  if (!status) return null;
  const k = status.toLowerCase();
  return STATUS_COLORS[k] ?? Object.entries(STATUS_COLORS).find(([name]) => k.includes(name))?.[1] ?? "#9aa0ab";
}

/**
 * Shows an instance's shapes as asked: hidden or isolated items, the selected
 * instance (at any depth) tinted, and, by status, each shape painted with the
 * sale status of the nearest instance above it that has one.
 */
function paintInstance(part: Part, entities: ReadonlyMap<string, EntityView>, selected: string | null, hidden: ReadonlySet<string>, isolated: string | null, colorBy: "material" | "status", tints?: ReadonlyMap<string, string>, hovered?: string | null) {
  for (const mesh of part.meshes) {
    const path = mesh.userData.path as string[];
    mesh.visible = !path.some((id) => hidden.has(id)) && (!isolated || path.includes(isolated));
    const chosen = !!selected && path.includes(selected);
    const hot = !chosen && !!hovered && path.includes(hovered);
    let status: string | undefined;
    for (let i = path.length - 1; i >= 0 && !status; i--) status = instance(entities.get(path[i]))?.properties?.status;
    let tint: string | undefined;
    for (let i = path.length - 1; i >= 0 && !tint; i--) tint = tints?.get(path[i]);
    const paint = colorBy === "status" ? statusColor(status) : (tint ?? null);
    for (const m of bodyMaterials(mesh)) {
      const look = m.userData.look as Look;
      m.color.set(paint ?? look.color);
      const glass = (look.opacity ?? 1) < 1 && !paint;
      m.transparent = glass;
      m.opacity = glass ? look.opacity! : 1;
      m.depthWrite = !glass;
      m.emissive.set(chosen ? ACCENT : hot ? HOVER : "#000000");
      m.emissiveIntensity = chosen ? 0.3 : hot ? HOVER_GLOW : 0;
      m.needsUpdate = true;
    }
  }
}

function disposePart(part: Part) {
  for (const mesh of part.meshes) {
    if (part.shared) releaseGeometry(mesh.geometry);
    else mesh.geometry.dispose();
    for (const m of bodyMaterials(mesh)) m.dispose();
  }
  part.outline.geometry.dispose();
  (part.outline.material as THREE.Material).dispose();
}

/** Parameter along line (p0 + s·u) closest to the ray. */
function closestParam(ray: THREE.Ray, p0: THREE.Vector3, u: THREE.Vector3): number | null {
  const w0 = p0.clone().sub(ray.origin);
  const a = u.dot(u);
  const b = u.dot(ray.direction);
  const c = ray.direction.dot(ray.direction);
  const d = u.dot(w0);
  const e = ray.direction.dot(w0);
  const denom = a * c - b * b;
  if (Math.abs(denom) < 1e-6) return null;
  return (b * e - c * d) / denom;
}

/** Which face of an axis-aligned part a ray hit, from the hit normal. */
function faceFromHit(hit: THREE.Intersection): Face | null {
  const n = hit.face?.normal;
  if (!n) return null;
  const axis = ([0, 1, 2] as Axis[]).reduce((best, k) => (Math.abs(n.getComponent(k)) > Math.abs(n.getComponent(best)) ? k : best), 0 as Axis);
  return { axis, sign: n.getComponent(axis) >= 0 ? 1 : -1 };
}

const sameFace = (a: Face | null, b: Face | null) => !!a && !!b && a.axis === b.axis && a.sign === b.sign;
const snapStep = (e: { shiftKey: boolean; altKey: boolean }) => (e.shiftKey ? 10 : e.altKey ? 0.1 : 1);
const snap = (v: number, step: number) => Math.round(v / step) * step;
const signed = (v: number) => (v > 0 ? `+${fmt(v)}` : v < 0 ? `−${fmt(-v)}` : "±0");
const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
const FACE_NAMES: Record<string, string> = { "0:1": "Right face", "0:-1": "Left face", "1:1": "Back face", "1:-1": "Front face", "2:1": "Top face", "2:-1": "Bottom face" };

export const Viewport = forwardRef<ViewportHandle, Props>(function Viewport(props, ref) {
  const { entities, selected, live, overlays } = props;
  const host = useRef<HTMLDivElement>(null);
  const [viewTools, setViewTools] = useState<ViewTools | null>(null);
  const toolsRef = useRef<ViewTools | null>(null);
  const label = useRef<HTMLDivElement>(null);
  const readout = useRef<HTMLDivElement>(null);
  const cursorLayer = useRef<HTMLDivElement>(null);
  const pinLayer = useRef<HTMLDivElement>(null);
  const pinGhost = useRef<HTMLDivElement>(null);
  const markupLayer = useRef<SVGSVGElement>(null);
  const cardAnchor = useRef<HTMLDivElement>(null);
  const faceBadge = useRef<HTMLDivElement>(null);
  const labelLayer = useRef<HTMLDivElement>(null);
  const sketchLabels = useRef<HTMLDivElement>(null);
  const sketchLayerRef = useRef<SketchLayer | null>(null);
  if (!sketchLayerRef.current) sketchLayerRef.current = new SketchLayer();
  const sketchLayer = sketchLayerRef.current;
  const draftLayerRef = useRef<DraftLayer | null>(null);
  if (!draftLayerRef.current) draftLayerRef.current = new DraftLayer();
  const draftLayer = draftLayerRef.current;
  const draftLabels = useRef<HTMLDivElement>(null);
  const edgeLayerRef = useRef<EdgeLayer | null>(null);
  const turning = useRef<Turning | null>(null);
  const motionGrab = useRef<MotionGrab | null>(null);
  const stressShown = useRef<{ id: string; group: THREE.Group; swing?: { mesh: THREE.Mesh; base: Float32Array; delta: Float32Array } } | null>(null);
  if (!edgeLayerRef.current) edgeLayerRef.current = new EdgeLayer();
  const edgeLayer = edgeLayerRef.current;
  const faceLayerRef = useRef<FaceLayer | null>(null);
  if (!faceLayerRef.current) faceLayerRef.current = new FaceLayer();
  const faceLayer = faceLayerRef.current;
  const pickedLayerRef = useRef<PickedFacesLayer | null>(null);
  if (!pickedLayerRef.current) pickedLayerRef.current = new PickedFacesLayer();
  const pickedLayer = pickedLayerRef.current;
  const connectorLayerRef = useRef<ConnectorLayer | null>(null);
  if (!connectorLayerRef.current) connectorLayerRef.current = new ConnectorLayer();
  const connectorLayer = connectorLayerRef.current;
  const connectorTags = useRef<HTMLDivElement>(null);
  const three = useRef<Scene | null>(null);
  const callbacks = useRef(props);
  callbacks.current = props;
  /** Where a picked face's body has moved to in an exploded or turning part (null: where it was built). */
  const facePose = useCallback((pf: ProgramFace) => {
    const g = pf.part ? assemblyParts(callbacks.current.entities.get(pf.id)).findIndex((q) => q.name === pf.part) : 0;
    return g < 0 ? null : bodyPose(turning.current, pf.id, g);
  }, []);
  const shape = useRef<Shape | null>(null); // displayed shape of the selected part
  const drag = useRef<Drag | null>(null);
  const face = useRef<Face | null>(null); // selected face of the selected part
  const lastSelected = useRef<string | null>(null);
  const framed = useRef(false);
  const pivot = useRef<Frac>([0.5, 0.5, 0.5]); // pivot of the selected part
  const pivotPreview = useRef<Frac | null>(null); // pivot being dragged, until the engine confirms it
  const currentPivot = () => pivotPreview.current ?? callbacks.current.pivotLive ?? pivot.current;
  const snapped = useRef<SnapTarget | null>(null); // magnetic snap in effect during a move
  const pivotTip = useRef<HTMLDivElement>(null);
  const snapTag = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = host.current!;
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true, stencil: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);
    renderer.shadowMap.enabled = true;
    renderer.localClippingEnabled = true; // section planes cut the parts, not the ground
    renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.45;
    renderer.domElement.className = "model-canvas";
    el.prepend(renderer.domElement);

    const scene = new THREE.Scene();
    // World matrices are brought up to date once per frame, before the instance layer reads them.
    scene.matrixWorldAutoUpdate = false;
    scene.fog = new THREE.Fog(0xf5f6f8, 2000, 6000);
    const camera = new THREE.PerspectiveCamera(36, 1, 1, 400000);
    camera.up.set(0, 0, 1);
    camera.position.set(330, -420, 260);

    // Sky color comes from +Z (Z is up).
    const hemi = new THREE.HemisphereLight(0xe1f2ee, 0x4a5c62, 2.7);
    hemi.position.set(0, 0, 1);
    scene.add(hemi);
    const sun = new THREE.DirectionalLight(0xfff4df, 4.2);
    sun.castShadow = true;
    sun.shadow.mapSize.set(2048, 2048);
    sun.shadow.bias = -0.0008;
    sun.shadow.normalBias = 0.6;
    scene.add(sun, sun.target);
    const rim = new THREE.DirectionalLight(0xc4d5ff, 1.5);
    rim.position.set(900, 800, 500);
    const fill = new THREE.DirectionalLight(0xb9cfff, 1.3);
    fill.position.set(-900, 900, 300);
    // Light bounced up off the ground, so undersides read when the camera looks from below.
    const bounce = new THREE.DirectionalLight(0xe4ebf2, 1.6);
    bounce.position.set(300, -500, -900);
    scene.add(rim, fill, bounce);

    const ground = new THREE.Mesh(
      new THREE.PlaneGeometry(40000, 40000),
      new THREE.MeshStandardMaterial({ color: 0xf2f3f5, roughness: 0.95, metalness: 0.08 }),
    );
    ground.position.z = -0.6;
    ground.receiveShadow = true;
    scene.add(ground);
    const grid = new THREE.GridHelper(6000, 300, 0xc7cbd6, 0xd9dce4);
    grid.rotation.x = Math.PI / 2;
    grid.position.z = -0.4;
    (grid.material as THREE.Material).transparent = true;
    (grid.material as THREE.Material).opacity = 0.38;
    scene.add(grid);

    const overlayRoot = new THREE.Group();
    const gizmo = new MoveGizmo();
    const push = new PushHandle();
    const selectedFaceTint = new FaceTint(ACCENT, 0.24, 1);
    const hoverFaceTint = new FaceTint(ACCENT, 0.1, 0.55);
    const ghost = new Ghost();
    const guide = new AxisGuide();
    const batch = new BatchLayer();
    scene.add(batch.root);
    scene.add(overlayRoot, hoverFaceTint.root, selectedFaceTint.root, ghost.root, guide.root, gizmo.root, push.root);

    // Pivot mode: 27 pickable points on the selected part, drawn as flat
    // camera-facing markers with a constant size in pixels.
    const onTopMat = (color: string, opacity = 1) =>
      new THREE.MeshBasicMaterial({ color, transparent: true, opacity, depthTest: false, depthWrite: false, toneMapped: false, fog: false, side: THREE.DoubleSide });
    const coreGeometry = new THREE.CircleGeometry(1, 28);
    const rimGeometry = new THREE.RingGeometry(1, 1.55, 28);
    const hitGeometry = new THREE.CircleGeometry(2.4, 12);
    const pivotDots = SNAP_FRACS.map((frac) => {
      const dot = new THREE.Group();
      const core = new THREE.Mesh(coreGeometry, onTopMat("#ffffff"));
      const rim = new THREE.Mesh(rimGeometry, onTopMat(ACCENT, 0.95));
      const hit = new THREE.Mesh(hitGeometry, onTopMat("#000000", 0));
      dot.add(hit, rim, core);
      dot.userData.frac = frac;
      dot.traverse((o) => (o.renderOrder = 1010));
      core.renderOrder = 1011;
      return { dot, core, rim, frac };
    });
    const occluder = new THREE.Raycaster();
    // In pivot mode the pivot gets its own gizmo: it moves the pivot, not the part.
    const pivotGizmo = new MoveGizmo();
    scene.add(pivotGizmo.root);
    const pivotRoot = new THREE.Group();
    for (const d of pivotDots) pivotRoot.add(d.dot);
    pivotRoot.visible = false;
    scene.add(pivotRoot);
    let hoverDot: Frac | null = null;

    // Snap indicator: a ring that faces the camera.
    const snapRing = new THREE.Mesh(new THREE.RingGeometry(0.9, 1.25, 32), onTopMat(ACCENT));
    const snapCore = new THREE.Mesh(new THREE.CircleGeometry(0.42, 20), onTopMat(ACCENT));
    const snapMark = new THREE.Group();
    snapMark.add(snapRing, snapCore);
    snapMark.traverse((o) => (o.renderOrder = 1020));
    snapMark.visible = false;
    scene.add(snapMark);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.target.set(60, 20, 10);
    // The camera goes all the way round, underneath included: the ground is
    // one-sided, so from below it vanishes and the grid stays as a reference.
    controls.autoRotateSpeed = 1.2;
    // Zoom heads for what is under the pointer, as in other CAD tools.
    controls.zoomToCursor = true;
    const tumbler = attachTumble(renderer.domElement, camera, controls);
    // Close in, zoom walks through a lid or a floor instead of stalling at it.
    const unwalk = attachWalk(renderer.domElement, camera, controls, () => Math.max((three.current?.extent.radius ?? 100) * 0.04, 0.5));
    // Projection, display modes, sections, measuring, box select and the view cube.
    const tools = new ViewTools({
      el,
      canvas: renderer.domElement,
      camera,
      target: () => controls.target,
      scene,
      bodies: () => visibleBodies(),
      materialsOf: bodyMaterials,
      extent: () => three.current?.extent ?? new THREE.Sphere(new THREE.Vector3(), 100),
      invalidate: () => (dirty = true),
      fit: (only, direction, up) => three.current && fitView(three.current, only, true, direction, up),
      selected: () => callbacks.current.selected,
      onSelectMany: (ids, add) => callbacks.current.onSelectMany?.(ids, add),
      nameOf: (id) => {
        const e = callbacks.current.entities.get(id);
        return e ? partName(e) : id;
      },
    });
    tools.measure.nameOf = (id) => {
      const e = callbacks.current.entities.get(id);
      return e ? partName(e) : id;
    };
    toolsRef.current = tools;
    // Reachable from the element for automated checks (headless browser tests).
    (el as HTMLElement & { viewTools?: ViewTools }).viewTools = tools;
    setViewTools(tools);

    let dirty = true;
    let raf = 0;
    let hoverFace: Face | null = null;
    let hoverPush = false;
    const invalidate = () => (dirty = true);
    pcbLooks.setInvalidate(invalidate);
    controls.addEventListener("change", invalidate);
    // Taking hold of the view ends any glide.
    controls.addEventListener("start", () => {
      if (three.current) three.current.flight = null;
    });

    const toScreen = (p: THREE.Vector3) => {
      const v = p.clone().project(tools.camera());
      const rect = renderer.domElement.getBoundingClientRect();
      return { x: ((v.x + 1) / 2) * rect.width, y: ((1 - v.y) / 2) * rect.height, behind: v.z > 1 };
    };

    /** A pointer on the sketch plane: plane coordinates, snapped to a point, a curve or the grid. */
    const sketchPointer = (type: SketchPointer["type"], e: MouseEvent) => {
      const pic = callbacks.current.sketch;
      if (!pic) return;
      const plane = pic.data.plane;
      const n = new THREE.Vector3(...normalOf(plane));
      const o = new THREE.Vector3(...plane.origin);
      const hit = new THREE.Vector3();
      if (!rayAt(e.clientX, e.clientY).ray.intersectPlane(new THREE.Plane().setFromNormalAndCoplanarPoint(n, o), hit)) return;
      const rel = hit.sub(o);
      let u = rel.dot(new THREE.Vector3(...plane.x));
      let v = rel.dot(new THREE.Vector3(...plane.y));
      const rect = renderer.domElement.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;
      const px = (a: number, b: number) => toScreen(new THREE.Vector3(...toWorld(plane, a, b)));
      // Snap: points, then midpoints, centres, quadrants and crossings, then onto a curve, else the grid.
      const distance = camera.position.distanceTo(controls.target);
      const raw = distance / 60;
      const mag = 10 ** Math.floor(Math.log10(raw));
      const grid = [1, 2, 5, 10].map((m) => m * mag).find((g) => g >= raw)!;
      const s = snapSketch(pic, [u, v], { x: mx, y: my }, px, grid, { alt: e.altKey, shift: e.shiftKey, anchor: pic.anchor ?? null });
      [u, v] = [s.u, s.v];
      callbacks.current.onSketchPointer({ type, u, v, point: s.point, curve: s.curve, grid, shift: e.shiftKey, snap: s.snap });
      dirty = true;
    };
    const gizmoScale = (at: THREE.Vector3) => camera.position.distanceTo(at) * GIZMO_SCALE;

    const writeReadout = (main: string, sub: string, at: THREE.Vector3, hint?: string) => {
      const out = readout.current;
      if (!out) return;
      const p = toScreen(at);
      out.style.display = p.behind ? "none" : "flex";
      out.style.transform = `translate(${p.x}px, ${p.y}px) translate(-50%, calc(-100% - 16px))`;
      out.innerHTML = `<strong></strong><span></span><em>${hint ?? "⇧ 10 mm · ⌥ 0.1 mm · Esc cancels"}</em>`;
      (out.children[0] as HTMLElement).textContent = main;
      (out.children[1] as HTMLElement).textContent = sub;
    };

    // ---- Comment pins: points in a part's own coordinates, drawn where the part is now.
    /** An instance draws its contents already placed by its position and rotation; other parts draw in their own coordinates. */
    const instanceFrame = (target: string) => {
      const e = callbacks.current.entities.get(target);
      return instance(e) ? placedOf(e) : null;
    };
    const shownPart = (target: string) => {
      const part = three.current?.parts.get(target);
      return part && part.group.visible && part.meshes.some((m) => m.visible) ? part : undefined;
    };
    /** Where a point in a part's coordinates is now (null when the part is gone or hidden). */
    const pinWorld = (target: string | null, p: readonly number[]): THREE.Vector3 | null => {
      const v = new THREE.Vector3(p[0], p[1], p[2]);
      if (!target) return v;
      const part = shownPart(target);
      if (!part) return null;
      const f = instanceFrame(target);
      if (f) v.set(...applyT(f, [v.x, v.y, v.z]));
      part.group.updateMatrixWorld();
      return part.group.localToWorld(v);
    };
    const pinLocal = (target: string, w: THREE.Vector3): THREE.Vector3 | null => {
      const part = three.current?.parts.get(target);
      if (!part) return null;
      part.group.updateMatrixWorld();
      const v = part.group.worldToLocal(w.clone());
      const f = instanceFrame(target);
      if (f) v.set(...applyT(invertT(f), [v.x, v.y, v.z]));
      return v;
    };
    const dirWorld = (target: string | null, d: readonly number[]) => {
      const v = new THREE.Vector3(d[0], d[1], d[2]);
      const f = target ? instanceFrame(target) : null;
      return f ? v.applyQuaternion(new THREE.Quaternion(...f.q)) : v;
    };
    const visibleBodies = () => [...(three.current?.parts.values() ?? [])].flatMap((p) => (p.group.visible ? p.meshes.filter((m) => m.visible) : []));
    const round3 = (v: THREE.Vector3): [number, number, number] => [v.x, v.y, v.z].map((x) => Math.round(x * 1000) / 1000) as [number, number, number];
    const onCommentUi = (e: Event) => !!(e.target as Element | null)?.closest?.(".comment-pins, .comment-card-anchor");

    /**
     * Pins behind the model are dimmed. Ray casts are not free on a big model,
     * so they run once the view settles, not on every frame of an orbit.
     */
    const pinBehind = new Map<string, boolean>();
    let occlusionTimer = 0;
    const updateOcclusion = () => {
      const c = callbacks.current.comments;
      if (!c) return;
      const bodies = visibleBodies();
      const eye = camera.position;
      let changed = false;
      for (const pin of c.pins) {
        const w = pinWorld(pin.target, pin.point);
        let behind = false;
        if (w) {
          const n = pin.normal ? dirWorld(pin.target, pin.normal) : null;
          const toEye = eye.clone().sub(w);
          const dist = toEye.length();
          if (n && n.dot(toEye) < -1e-6 * dist) behind = true;
          else {
            const hit = new THREE.Raycaster(eye.clone(), w.clone().sub(eye).normalize(), 0, dist).intersectObjects(bodies, false)[0];
            behind = !!hit && hit.distance < dist - Math.max(0.5, dist * 0.004);
          }
        }
        if (pinBehind.get(pin.id) !== behind) {
          pinBehind.set(pin.id, behind);
          changed = true;
        }
      }
      if (changed) dirty = true;
    };
    const scheduleOcclusion = () => {
      window.clearTimeout(occlusionTimer);
      occlusionTimer = window.setTimeout(updateOcclusion, 90);
    };

    /** A markup stroke being drawn (project coordinates). */
    let liveStroke: { points: THREE.Vector3[]; arrow: boolean; pointer: number } | null = null;

    const strokePath = (pts: { x: number; y: number }[]) => pts.map((q, i) => `${i ? "L" : "M"}${q.x.toFixed(1)} ${q.y.toFixed(1)}`).join("");
    /** An arrowhead at the end of a stroke, pointing the way it was drawn on screen. */
    const arrowHead = (pts: { x: number; y: number }[], size: number) => {
      const tip = pts[pts.length - 1];
      let from = pts[0];
      for (let i = pts.length - 2; i >= 0; i--) {
        if (Math.hypot(pts[i].x - tip.x, pts[i].y - tip.y) >= size * 1.2) {
          from = pts[i];
          break;
        }
      }
      const a = Math.atan2(tip.y - from.y, tip.x - from.x);
      const wing = (s: number) => `${(tip.x - size * Math.cos(a + s * 0.48)).toFixed(1)} ${(tip.y - size * Math.sin(a + s * 0.48)).toFixed(1)}`;
      return `M${wing(1)}L${tip.x.toFixed(1)} ${tip.y.toFixed(1)}L${wing(-1)}`;
    };
    const drawMarkup = (c: CommentsView | null | undefined, placed: Map<string, boolean>) => {
      const svg = markupLayer.current;
      if (!svg) return;
      const lines: string[] = [];
      const draw = (pts: { x: number; y: number }[], arrow: boolean, color: string, opacity: number) => {
        if (pts.length < 2) return;
        const d = strokePath(pts) + (arrow ? arrowHead(pts, 13) : "");
        lines.push(`<g opacity="${opacity}"><path d="${d}" class="halo"/><path d="${d}" stroke="${color}"/></g>`);
      };
      const project = (w: THREE.Vector3 | null) => {
        if (!w) return null;
        const q = toScreen(w);
        return q.behind ? null : q;
      };
      for (const pin of c?.pins ?? []) {
        if (!pin.markup.length || !placed.has(pin.id)) continue;
        const active = c!.active === pin.id || pin.draft;
        const opacity = placed.get(pin.id) && !active ? 0.3 : active ? 1 : 0.72;
        for (const st of pin.markup) {
          const pts = st.points.map((p) => project(pinWorld(pin.target, p))).filter((q): q is { x: number; y: number; behind: boolean } => !!q);
          draw(pts, !!st.arrow, st.color ?? pin.color, opacity);
        }
      }
      if (liveStroke && c) {
        const pts = liveStroke.points.map((w) => project(w)).filter((q): q is { x: number; y: number; behind: boolean } => !!q);
        draw(pts, liveStroke.arrow, c.me.color, 1);
      }
      svg.innerHTML = lines.join("");
    };

    const layoutPins = () => {
      const c = callbacks.current.comments;
      const rect = renderer.domElement.getBoundingClientRect();
      // Pin id → whether it is dimmed (behind the model); only pins on screen.
      const placed = new Map<string, boolean>();
      const screen = new Map<string, { x: number; y: number }>();
      for (const tag of Array.from(pinLayer.current?.children ?? []) as HTMLElement[]) {
        const pin: CommentPin | undefined = c?.pins.find((p) => p.id === tag.dataset.pin);
        const w = pin ? pinWorld(pin.target, pin.point) : null;
        const q = w ? toScreen(w) : null;
        if (!pin || !q || q.behind) {
          tag.style.display = "none";
          continue;
        }
        const behind = !!pinBehind.get(pin.id) && c!.active !== pin.id && !pin.draft;
        tag.style.display = "block";
        tag.style.transform = `translate(${q.x.toFixed(1)}px, ${q.y.toFixed(1)}px)`;
        tag.classList.toggle("is-behind", behind);
        placed.set(pin.id, behind);
        screen.set(pin.id, q);
      }
      // The open thread's card sits beside its pin, on whichever side has room.
      const card = cardAnchor.current;
      if (card) {
        if (!c?.active || !c.card) card.style.display = "none";
        else {
          card.style.display = "block";
          const at = screen.get(c.active);
          const [w, h] = [rect.width, rect.height];
          const [cw, ch] = [card.offsetWidth || 320, card.offsetHeight || 180];
          let x = w - cw - 24;
          let y = 96;
          if (at) {
            x = at.x + 46 + cw <= w - 12 ? at.x + 46 : at.x - 18 - cw;
            y = at.y - 34;
          }
          x = Math.max(12, Math.min(x, w - cw - 12));
          y = Math.max(12, Math.min(y, h - ch - 12));
          card.style.transform = `translate(${x.toFixed(0)}px, ${y.toFixed(0)}px)`;
        }
      }
      drawMarkup(c, placed);
    };

    const layoutOverlays = () => {
      layoutPins();
      for (const { el: tag, at } of three.current?.overlayLabels ?? []) {
        const p = toScreen(at);
        tag.style.display = p.behind ? "none" : "block";
        tag.style.transform = `translate(${p.x}px, ${p.y}px) translate(-50%, -50%)`;
      }
      // Teammates' cursors, where they point on the model.
      for (const tag of Array.from(cursorLayer.current?.children ?? []) as HTMLElement[]) {
        const mate = callbacks.current.teammates?.find((m) => m.engine === tag.dataset.engine);
        if (!mate?.pointer) {
          tag.style.display = "none";
          continue;
        }
        const p = toScreen(new THREE.Vector3(...mate.pointer));
        tag.style.display = p.behind ? "none" : "block";
        tag.style.transform = `translate(${p.x}px, ${p.y}px)`;
      }
      const s = shape.current;
      const d = drag.current;
      const wire = !!three.current?.wireframe;
      const f = face.current;

      // Manipulators. The move gizmo sits on the part's pivot.
      const pivotMode = callbacks.current.pivotMode;
      const origin = s ? new THREE.Vector3(...pivotPoint(s, pivot.current)) : null;
      const selEntity = callbacks.current.selected ? callbacks.current.entities.get(callbacks.current.selected) : undefined;
      // A joint places what it holds: no handles to move it by hand.
      const held = !!selEntity?.placed?.by || !!callbacks.current.poses;
      gizmo.setTurnable(!!instance(selEntity) && !callbacks.current.also?.length);
      gizmo.root.visible = !!s && !f && !wire && !pivotMode && !held && !callbacks.current.edges && !callbacks.current.facePicks && !faceLayer.active && !connectorLayer.active && (!d || d.kind !== "push");
      connectorLayer.layout(camera, renderer.domElement.clientHeight, toScreen, connectorTags.current);
      const ff = faceLayer.frame();
      if (ff) faceLayer.scale(gizmoScale(ff.origin));
      pivotRoot.visible = !!s && pivotMode && (!d || d.kind === "pivot");
      pivotGizmo.root.visible = !!s && pivotMode && (!d || d.kind === "pivot");
      if (s && pivotMode) {
        const at = new THREE.Vector3(...pivotPoint(s, currentPivot()));
        pivotGizmo.layout(at, gizmoScale(at) * 0.8, camera);
      }
      if (s && pivotMode) {
        const body = selectedBody();
        const heightPx = renderer.domElement.clientHeight || 1;
        const perPixel = (at: THREE.Vector3) => (2 * camera.position.distanceTo(at) * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2))) / heightPx;
        for (const p of pivotDots) {
          const at = new THREE.Vector3(...pivotPoint(s, p.frac));
          const current = p.frac.every((v, k) => Math.abs(v - currentPivot()[k]) < 1e-9);
          const hovered = !!hoverDot && p.frac.every((v, k) => v === hoverDot![k]);
          // Points behind the part fade so the visible ones read first.
          const toPoint = at.clone().sub(camera.position);
          occluder.set(camera.position, toPoint.clone().normalize());
          const blocker = body ? occluder.intersectObjects(body, false)[0] : undefined;
          const hidden = !!blocker && blocker.distance < toPoint.length() - 0.5;
          p.dot.position.copy(at);
          p.dot.quaternion.copy(camera.quaternion);
          p.dot.scale.setScalar(perPixel(at) * (hovered ? 6.5 : current ? 5.5 : 4));
          const core = p.core.material as THREE.MeshBasicMaterial;
          const rim = p.rim.material as THREE.MeshBasicMaterial;
          core.color.set(hovered || current ? ACCENT : "#ffffff");
          rim.color.set(hovered || current ? "#ffffff" : ACCENT);
          core.opacity = hidden && !hovered && !current ? 0.35 : 1;
          rim.opacity = hidden && !hovered && !current ? 0.3 : 0.95;
        }
      }
      const tip = pivotTip.current;
      if (tip) {
        const shown = s && pivotMode && !d ? hoverDot ?? currentPivot() : null;
        if (s && shown) {
          const p = toScreen(new THREE.Vector3(...pivotPoint(s, shown)));
          tip.style.display = p.behind ? "none" : "flex";
          tip.style.transform = `translate(${p.x}px, ${p.y}px) translate(16px, -50%)`;
          tip.innerHTML = "<strong></strong><span></span>";
          (tip.children[0] as HTMLElement).textContent = pivotName(shown);
          (tip.children[1] as HTMLElement).textContent = hoverDot
            ? "Click to move and rotate from here"
            : "Drag it anywhere · click a point or the surface · Esc";
        } else {
          tip.style.display = "none";
        }
      }
      const snapAt = snapped.current;
      snapMark.visible = !!snapAt && !!d;
      if (snapAt && d) {
        snapMark.position.copy(snapAt.at);
        snapMark.quaternion.copy(camera.quaternion);
        snapMark.scale.setScalar(camera.position.distanceTo(snapAt.at) * 0.011);
      }
      const st = snapTag.current;
      if (st) {
        if (snapAt && d) {
          const p = toScreen(snapAt.at);
          st.style.display = p.behind ? "none" : "block";
          st.style.transform = `translate(${p.x}px, ${p.y}px) translate(-50%, 16px)`;
          st.textContent = `Snapped · ${snapAt.label}`;
        } else {
          st.style.display = "none";
        }
      }
      if (s && origin) gizmo.layout(origin, gizmoScale(origin), camera);
      selectedFaceTint.show(wire ? null : s, f);
      hoverFaceTint.show(!d && !wire && !sameFace(hoverFace, f) ? s : null, hoverFace);
      push.layout(wire ? null : s, f, s && f ? gizmoScale(faceFrame(s, f).center) : 1);
      ghost.show(d && d.kind !== "pivot" ? d.start : null);
      guide.show(
        d && s && (d.kind === "axis" || d.kind === "push") ? (d.kind === "axis" ? new THREE.Vector3(...pivotPoint(s, pivot.current)) : faceFrame(s, d.face).center) : null,
        d && (d.kind === "axis" ? d.axis : d.kind === "push" ? d.face.axis : null) as Axis | null,
        d?.kind === "push" ? ACCENT : undefined,
      );

      // Dimension label under the selected part.
      const dim = label.current;
      if (dim) {
        if (!s) {
          dim.style.display = "none";
        } else {
          const p = toScreen(new THREE.Vector3(s.x + s.width / 2, s.y, s.z));
          dim.style.display = p.behind ? "none" : "block";
          dim.style.transform = `translate(${p.x}px, ${p.y + 14}px) translateX(-50%)`;
          dim.textContent = `${fmt(s.width)} × ${fmt(s.depth)} × ${fmt(s.height)} mm`;
          dim.classList.toggle("is-live", !!d);
        }
      }

      // Face badge: names the selected face and how to use it.
      const badge = faceBadge.current;
      if (badge) {
        if (s && f && !d) {
          // Anchor at the arrow tip so the hint never covers the part or its labels.
          const frameAt = faceFrame(s, f);
          const tip = frameAt.center.clone().addScaledVector(frameAt.normal, gizmoScale(frameAt.center) * 1.08);
          const p = toScreen(tip);
          badge.style.display = p.behind ? "none" : "flex";
          badge.style.transform = `translate(${p.x}px, ${p.y}px) translate(14px, -50%)`;
          badge.innerHTML = "<strong></strong><span>Drag the arrow to push or pull · Esc</span>";
          (badge.children[0] as HTMLElement).textContent = FACE_NAMES[`${f.axis}:${f.sign}`];
        } else {
          badge.style.display = "none";
        }
      }

      // Live readout while dragging.
      if (!d || !s) {
        if (readout.current) readout.current.style.display = "none";
      } else if (d.kind === "axis") {
        const k = d.axis;
        const at = new THREE.Vector3(...pivotPoint(s, pivot.current));
        writeReadout(`${AXIS_NAMES[k]} ${signed(s[POS_KEYS[k]] - d.start[POS_KEYS[k]])} mm`, `${AXIS_NAMES[k]} = ${fmt(at.getComponent(k))} mm`, at.clone().addScaledVector(UNIT[k], gizmoScale(at) * 1.15));
      } else if (d.kind === "plane" || d.kind === "ground") {
        const axes = d.kind === "plane" ? ([0, 1, 2] as Axis[]).filter((k) => k !== d.normal) : ([0, 1] as Axis[]);
        writeReadout(
          axes.map((k) => `${AXIS_NAMES[k]} ${signed(s[POS_KEYS[k]] - d.start[POS_KEYS[k]])}`).join("  ·  ") + " mm",
          axes.map((k) => `${AXIS_NAMES[k]} = ${fmt(pivotPoint(s, pivot.current)[k])}`).join("  ·  ") + " mm",
          new THREE.Vector3(s.x + s.width / 2, s.y + s.depth / 2, s.z + s.height),
        );
      } else if (d.kind === "pivot") {
        const at = new THREE.Vector3(...pivotPoint(s, currentPivot()));
        writeReadout(
          `Pivot  X ${fmt(at.x)} · Y ${fmt(at.y)} · Z ${fmt(at.z)} mm`,
          snapped.current ? pivotName(currentPivot()) : `${pivotName(currentPivot())} · ⌘ no snapping`,
          at.clone().add(new THREE.Vector3(0, 0, gizmoScale(at) * 0.9)),
        );
      } else if (d.kind === "ring") {
        writeReadout(`Turn ${signed(d.turned)}° about ${AXIS_NAMES[d.axis]}`, "15° steps", d.origin.clone().addScaledVector(UNIT[(d.axis + 2) % 3], gizmoScale(d.origin) * 1.05), "⇧ 45° · ⌥ 1° · Esc cancels");
      } else if (d.kind === "facePush") {
        // The app's hint names what changes; the face itself shows where.
        writeReadout(`${d.distance >= 0 ? "Push" : "Pull"} ${signed(d.distance)} mm`, "release to apply · Esc cancels", d.origin.clone().addScaledVector(d.normal, d.distance + gizmoScale(d.origin) * 1.2));
      } else {
        const k = d.face.axis;
        const size = s[SIZE_KEYS[k]];
        const at = faceFrame(s, d.face).center.addScaledVector(UNIT[k], d.face.sign * gizmoScale(faceFrame(s, d.face).center) * 1.15);
        writeReadout(`${SIZE_NAMES[k]} ${fmt(size)} mm`, `${d.face.sign > 0 ? "Pushed" : "Pulled"} ${signed(size - d.start[SIZE_KEYS[k]])} mm`, at);
      }
    };

    const frame = (now: number) => {
      raf = requestAnimationFrame(frame);
      const t = three.current;
      if (t?.flight) {
        stepFlight(t, now);
        dirty = true;
      }
      // A vibration mode swinging (about one swing a second).
      const sw = stressShown.current?.swing;
      if (sw) {
        const k = Math.sin((now / 1000) * Math.PI * 2 * 0.9);
        const pos = sw.mesh.geometry.getAttribute("position") as THREE.BufferAttribute;
        const arr = pos.array as Float32Array;
        for (let i = 0; i < arr.length; i++) arr[i] = sw.base[i] + sw.delta[i] * k;
        pos.needsUpdate = true;
        sw.mesh.geometry.computeVertexNormals();
        dirty = true;
      }
      if (controls.autoRotate) {
        controls.update();
        dirty = true;
      }
      if (!dirty) return;
      dirty = false;
      draw();
    };
    const frameStats = { aside: 0, matrices: 0, batch: 0, render: 0 };
    function* instanceMeshes() {
      for (const part of three.current?.parts.values() ?? []) if (part.shared && part.group.visible) yield* part.meshes;
    }
    const draw = () => {
      const distance = camera.position.distanceTo(controls.target);
      // The haze starts past the model, however close in the camera is.
      const extent = three.current?.extent;
      const beyond = extent ? camera.position.distanceTo(extent.center) + extent.radius : 0;
      const fog = scene.fog as THREE.Fog;
      fog.near = Math.max(distance * 2.2, beyond);
      fog.far = Math.max(distance * 6, beyond * 2.5);
      layoutOverlays();
      sketchLayer.layout(toScreen);
      draftLayer.layout(toScreen);
      // A part the camera is inside (a case, a room's shell, a floor passed
      // through) steps aside, as if the camera had walked in: what it holds
      // shows instead of the inside of its walls.
      const t0 = performance.now();
      scene.updateMatrixWorld();
      const aside = three.current ? enclosing(three.current, camera.position) : [];
      for (const m of aside) m.visible = false;
      const t1 = performance.now();
      const view = tools.beforeRender();
      // Instances of shared geometry go to the GPU as instances, culled and simplified by size.
      const t2 = performance.now();
      batch.update(instanceMeshes(), view, renderer.domElement.clientHeight || 800, three.current?.wireframe ?? false);
      const t3 = performance.now();
      renderer.render(scene, view);
      tools.afterRender(renderer, view);
      frameStats.aside = t1 - t0;
      frameStats.matrices = t2 - t1;
      frameStats.batch = t3 - t2;
      frameStats.render = performance.now() - t3;
      for (const m of aside) m.visible = true;
      if (callbacks.current.comments?.pins.length) scheduleOcclusion();
    };
    raf = requestAnimationFrame(frame);
    // `?perf`: a handle for measuring the view (tools and benchmarks drive it
    // over the DevTools protocol): one frame drawn and waited for, in ms.
    if (new URLSearchParams(location.search).has("perf")) {
      const px = new Uint8Array(4);
      (window as unknown as { formaPerf: unknown }).formaPerf = {
        renderer,
        camera,
        controls,
        parts: () => three.current?.parts.size ?? 0,
        batch,
        frameStats,
        frame: () => {
          const t0 = performance.now();
          draw();
          const gl = renderer.getContext();
          gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
          return performance.now() - t0;
        },
      };
    }

    const resize = () => {
      const { clientWidth: w, clientHeight: h } = el;
      renderer.setSize(w, h, false);
      edgeLayer.setResolution(w, h);
      camera.aspect = w / Math.max(h, 1);
      camera.updateProjectionMatrix();
      dirty = true;
    };
    const observer = new ResizeObserver(resize);
    observer.observe(el);

    const raycaster = new THREE.Raycaster();
    const rayAt = (clientX: number, clientY: number) => {
      const rect = renderer.domElement.getBoundingClientRect();
      // Picking must see what is on screen now, not what was last drawn. A
      // frame can be pending (a part just arrived, the camera just framed it)
      // or, in a background tab, never come: no animation frames run there
      // and resize notices wait too, so the camera and the parts' world
      // matrices are brought up to date here before casting.
      const size = renderer.getSize(new THREE.Vector2());
      if (Math.round(size.x) !== el.clientWidth || Math.round(size.y) !== el.clientHeight) resize();
      if (dirty) scene.updateMatrixWorld();
      camera.updateMatrixWorld();
      raycaster.setFromCamera(new THREE.Vector2(((clientX - rect.left) / rect.width) * 2 - 1, -((clientY - rect.top) / rect.height) * 2 + 1), tools.camera());
      return raycaster;
    };
    /** What is drawn for the selection: a part, or the shapes inside a selected instance (at any depth). */
    const selectedMeshes = (): THREE.Mesh[] => {
      const id = callbacks.current.selected;
      if (!id || !three.current) return [];
      const own = three.current.parts.get(id);
      if (own) return own.meshes;
      return [...three.current.parts.values()].flatMap((p) => p.meshes.filter((m) => (m.userData.path as string[] | undefined)?.includes(id)));
    };
    const selectedBody = () => {
      const meshes = selectedMeshes();
      return meshes.length ? meshes : undefined;
    };
    const pickGizmo = (r: THREE.Raycaster): GizmoPick | null => {
      if (!gizmo.root.visible) return null;
      const hit = r.intersectObjects(gizmo.hits(), false)[0];
      return hit ? gizmo.pickOf(hit.object) : null;
    };
    const pickPush = (r: THREE.Raycaster) => push.root.visible && r.intersectObject(push.hit, false).length > 0;
    const selectedIsProgram = () => {
      const id = callbacks.current.selected;
      return !!(id && three.current?.parts.get(id)?.isProgram);
    };
    const hitSelectedBody = (r: THREE.Raycaster) => {
      const body = selectedBody();
      // Into turning or exploded bodies too (split out as children, drawn where they have moved).
      const hit = body ? r.intersectObjects(body, true)[0] : undefined;
      if (!hit) return undefined;
      // Only when nothing else stands in front of it: a click on a part in front picks that part.
      const all = [...(three.current?.parts.values() ?? [])].flatMap((p) => p.meshes.filter((m) => m.visible));
      const front = r.intersectObjects(all, true)[0];
      return front && front.distance < hit.distance - 1e-6 && front.object.userData.id !== callbacks.current.selected ? undefined : hit;
    };
    /** Faces exist on boxes; AI-made shapes change through their parameters instead. */
    const hitSelectedFace = (r: THREE.Raycaster): Face | null => {
      if (selectedIsProgram()) return null;
      const hit = hitSelectedBody(r);
      return hit ? faceFromHit(hit) : null;
    };

    const pickDot = (r: THREE.Raycaster): Frac | null => {
      if (!pivotRoot.visible) return null;
      const hit = r.intersectObjects(pivotDots.map((p) => p.dot), true)[0];
      let o: THREE.Object3D | null = hit?.object ?? null;
      while (o && !o.userData.frac) o = o.parent;
      return o ? (o.userData.frac as Frac) : null;
    };

    /** Snap points of every other part, for magnetic moves. */
    const snapTargets = (except: string): SnapTarget[] => {
      const out: SnapTarget[] = [];
      for (const [id, part] of three.current?.parts ?? []) {
        if (id === except) continue;
        const e = callbacks.current.entities.get(id);
        const name = e ? partName(e) : "part";
        for (const frac of SNAP_FRACS) out.push({ at: new THREE.Vector3(...pivotPoint(part.base, frac)), label: `${pivotName(frac).toLowerCase()} of ${name}` });
      }
      return out;
    };

    /**
     * Magnetic snap: if moving the dragged components of the pivot onto another
     * part's snap point keeps it within a few pixels on screen, lock to it.
     */
    const pointMagnet = (p: THREE.Vector3, axes: Axis[], targets: SnapTarget[], free: boolean): THREE.Vector3 => {
      snapped.current = null;
      if (free || !targets.length) return p;
      const ps = toScreen(p);
      let best: { target: SnapTarget; q: THREE.Vector3; px: number } | null = null;
      for (const t of targets) {
        const q = p.clone();
        for (const k of axes) q.setComponent(k, t.at.getComponent(k));
        const qs = toScreen(q);
        const px = Math.hypot(qs.x - ps.x, qs.y - ps.y);
        if (px < SNAP_PIXELS && (!best || px < best.px)) best = { target: t, q, px };
      }
      if (!best) return p;
      snapped.current = { at: best.q, label: best.target.label };
      return best.q;
    };

    const magnet = (patch: Partial<Shape>, start: Shape, axes: Axis[], targets: SnapTarget[], free: boolean): Partial<Shape> => {
      const next = { ...start, ...patch };
      const p = new THREE.Vector3(...pivotPoint(next, pivot.current));
      const q = pointMagnet(p, axes, targets, free);
      if (q === p) return patch;
      const out = { ...patch };
      const size = [next.width, next.depth, next.height];
      for (const k of axes) out[POS_KEYS[k]] = q.getComponent(k) - pivot.current[k] * size[k];
      return out;
    };

    const fracOf = (s: Shape, p: THREE.Vector3): Frac => {
      const min = [s.x, s.y, s.z];
      const size = [s.width, s.depth, s.height];
      return [0, 1, 2].map((k) => (size[k] > 0 ? (p.getComponent(k) - min[k]) / size[k] : 0.5)) as Frac;
    };

    /** The edge under the pointer, if any: near it on screen and not behind the part. */
    const edgeUnder = (e: MouseEvent): string | null => {
      const id = callbacks.current.edges?.id;
      const bodies = (id && three.current?.parts.get(id)?.meshes) || [];
      const rect = renderer.domElement.getBoundingClientRect();
      const hidden = (p: THREE.Vector3) => {
        const from = camera.position.clone();
        const dist = from.distanceTo(p);
        const r = new THREE.Raycaster(from, p.clone().sub(from).normalize(), 0, dist);
        const hit = r.intersectObjects(bodies, false)[0];
        return !!hit && hit.distance < dist - Math.max(0.3, dist * 0.002);
      };
      return edgeLayer.hit(toScreen, e.clientX - rect.left, e.clientY - rect.top, hidden);
    };

    /** Where the pointer is round a ring (degrees, anticlockwise about its axis), on the ring's plane. */
    const ringAngle = (r: THREE.Raycaster, origin: THREE.Vector3, axis: Axis): number | null => {
      const n = UNIT[axis];
      // Looking along the ring's plane: use the plane facing the eye through the axis instead.
      const plane = Math.abs(r.ray.direction.dot(n)) > 0.08 ? new THREE.Plane().setFromNormalAndCoplanarPoint(n, origin) : null;
      const hit = plane ? r.ray.intersectPlane(plane, new THREE.Vector3()) : null;
      if (!hit) return null;
      const v = hit.sub(origin);
      const u = UNIT[((axis + 1) % 3) as Axis];
      const w = UNIT[((axis + 2) % 3) as Axis];
      return THREE.MathUtils.radToDeg(Math.atan2(v.dot(w), v.dot(u)));
    };
    /** Shows an instance turned by `deg` about an axis through `origin` (0: as it is). */
    const turnPreview = (id: string, axis: Axis, deg: number, origin: THREE.Vector3) => {
      const part = three.current?.parts.get(id);
      if (!part) return;
      const q = new THREE.Quaternion().setFromAxisAngle(UNIT[axis], THREE.MathUtils.degToRad(deg));
      part.group.quaternion.copy(q);
      const base = part.group.userData.base as THREE.Vector3 | undefined;
      const at = base ?? part.group.position.clone();
      part.group.userData.base = at;
      // Turning about the pivot: the group's origin swings round it.
      part.group.position.copy(origin.clone().sub(origin.clone().sub(at).applyQuaternion(q)));
      if (deg === 0) {
        part.group.position.copy(at);
        delete part.group.userData.base;
      }
      dirty = true;
    };

    // Capture phase on the host runs before OrbitControls sees the event.
    // A mechanism being dragged by hand: where the pointer was last.
    let motionDrag: { x: number; y: number } | null = null;
    const onDown = (e: PointerEvent) => {
      if (onCommentUi(e)) return;
      // Measuring, picking a section face or drawing a selection box.
      if (tools.down(e)) return;
      // Picking a joint's connector: a click on a triad picks it; anywhere else orbits.
      if (connectorLayer.active && e.button === 0) {
        const rect = renderer.domElement.getBoundingClientRect();
        const key = connectorLayer.hit(toScreen, e.clientX - rect.left, e.clientY - rect.top);
        if (key) {
          e.stopPropagation();
          e.preventDefault();
          callbacks.current.onConnectorPick?.(key);
          dirty = true;
        }
        return;
      }
      // Picking geometry for a mate: a click on a part reports the face under it; anywhere else orbits.
      const geometryPick = callbacks.current.onGeometryPick;
      if (geometryPick && e.button === 0) {
        const hit = rayAt(e.clientX, e.clientY).intersectObjects(visibleBodies(), false)[0];
        if (!hit) return;
        e.stopPropagation();
        e.preventDefault();
        const n = hit.face ? hit.face.normal.clone().transformDirection(hit.object.matrixWorld).normalize() : new THREE.Vector3(0, 0, 1);
        geometryPick({ path: (hit.object.userData.path as string[] | undefined) ?? [], shape: (hit.object.userData.shape as string | undefined) ?? null, point: hit.point.toArray() as V3, normal: n.toArray() as V3 });
        dirty = true;
        return;
      }
      const cm = callbacks.current.comments;
      if (cm?.tool && e.button === 0) {
        e.stopPropagation();
        e.preventDefault();
        const hit = rayAt(e.clientX, e.clientY).intersectObjects(visibleBodies(), false)[0];
        if (!hit) return;
        const n = hit.face ? hit.face.normal.clone().transformDirection(hit.object.matrixWorld) : new THREE.Vector3();
        el.setPointerCapture(e.pointerId);
        controls.enabled = false;
        liveStroke = { points: [hit.point.clone().addScaledVector(n, camera.position.distanceTo(hit.point) * 0.0015)], arrow: cm.tool === "arrow", pointer: e.pointerId };
        dirty = true;
        return;
      }
      // Comment mode: a click pins a comment; dragging still orbits.
      if (cm?.placing) return;
      // Sketching: a left click draws on the plane (zoom and right-drag pan still work).
      if (callbacks.current.sketch) {
        if (e.button !== 0) return;
        e.stopPropagation();
        e.preventDefault();
        sketchPointer("down", e);
        return;
      }
      // Picking edges: a click on an edge picks it; anywhere else orbits as usual.
      if (callbacks.current.edges) {
        const key = e.button === 0 ? edgeUnder(e) : null;
        if (!key) return;
        e.stopPropagation();
        e.preventDefault();
        callbacks.current.onEdgeClick(key);
        return;
      }
      // A mechanism open for driving: pressing on a moving part grabs it.
      const mg = motionGrab.current;
      const mpart = mg ? three.current?.parts.get(mg.id) : undefined;
      if (mg && mpart && e.button === 0) {
        const hit = rayAt(e.clientX, e.clientY).intersectObject(mpart.body, true)[0];
        const groupOf = (fi: number) => mpart.body.geometry.groups.findIndex((g) => fi * 3 >= g.start && fi * 3 < g.start + g.count);
        const moving = hit && (hit.object !== mpart.body || (hit.faceIndex != null && mg.groups.includes(groupOf(hit.faceIndex))));
        if (moving) {
          e.stopPropagation();
          e.preventDefault();
          el.setPointerCapture(e.pointerId);
          controls.enabled = false;
          motionDrag = { x: e.clientX, y: e.clientY };
          el.classList.add("is-dragging");
          mg.onStart();
          return;
        }
      }
      const s = shape.current;
      const id = callbacks.current.selected;
      if (e.button !== 0 || !s || !id) return;
      const r = rayAt(e.clientX, e.clientY);
      if (callbacks.current.pivotMode) {
        const start = { ...s };
        const origin = new THREE.Vector3(...pivotPoint(start, currentPivot()));
        const gizmoHit = pivotGizmo.root.visible ? r.intersectObjects(pivotGizmo.hits(), false)[0] : undefined;
        const pick = gizmoHit ? pivotGizmo.pickOf(gizmoHit.object) : null;
        if (pick) {
          // Drag the pivot itself; the part stays where it is.
          const own: SnapTarget[] = SNAP_FRACS.map((frac) => ({ at: new THREE.Vector3(...pivotPoint(start, frac)), label: `${pivotName(frac).toLowerCase()} of this part` }));
          const plane =
            pick.kind === "plane"
              ? new THREE.Plane().setFromNormalAndCoplanarPoint(UNIT[pick.normal], origin)
              : pick.kind === "center"
                ? new THREE.Plane().setFromNormalAndCoplanarPoint(camera.getWorldDirection(new THREE.Vector3()), origin)
                : null;
          const startHit = plane ? r.ray.intersectPlane(plane, new THREE.Vector3()) : null;
          const startParam = pick.kind === "axis" ? closestParam(r.ray, origin, UNIT[pick.axis]) : 0;
          if ((plane && !startHit) || startParam === null) return;
          drag.current = { kind: "pivot", id, pick, origin, startParam, startHit: startHit ?? origin.clone(), plane, start, moved: false, targets: [...own, ...snapTargets(id)] };
          pivotGizmo.setActive(pick);
          e.stopPropagation();
          e.preventDefault();
          el.setPointerCapture(e.pointerId);
          el.classList.add("is-dragging");
          controls.enabled = false;
          dirty = true;
          return;
        }
        const frac = pickDot(r);
        const body = selectedBody();
        const surface = !frac && body ? r.intersectObjects(body, false)[0] : undefined;
        if (!frac && !surface) return; // orbiting stays available in pivot mode
        e.stopPropagation();
        e.preventDefault();
        hoverDot = null;
        if (frac) {
          callbacks.current.onPickPivot(frac);
        } else {
          // Drop the pivot exactly where the surface was clicked (to 0.1 mm).
          const at = surface!.point.clone();
          at.set(Math.round(at.x * 10) / 10, Math.round(at.y * 10) / 10, Math.round(at.z * 10) / 10);
          callbacks.current.onPickPivot(fracOf(start, at));
        }
        dirty = true;
        return;
      }
      const start = { ...s };
      const f = face.current;
      if (faceLayer.active && faceLayer.grabs(r)) {
        // A program part's flat face: push or pull it along its normal.
        const fr = faceLayer.frame()!;
        const startParam = closestParam(r.ray, fr.origin, fr.normal);
        if (startParam === null) return;
        drag.current = { kind: "facePush", id, origin: fr.origin, normal: fr.normal, startParam, start, distance: 0 };
      } else if (f) {
        // Face mode: the arrow, or the highlighted face itself, push-pulls.
        if (!pickPush(r) && !sameFace(hitSelectedFace(r), f)) return;
        const origin = faceFrame(start, f).center;
        const startParam = closestParam(r.ray, origin, UNIT[f.axis]);
        if (startParam === null) return;
        drag.current = { kind: "push", id, face: f, origin, startParam, start };
      } else {
        const pick = pickGizmo(r);
        const origin = new THREE.Vector3(...pivotPoint(start, pivot.current));
        const targets = snapTargets(id);
        if (pick?.kind === "ring") {
          const a = ringAngle(r, origin, pick.axis);
          if (a === null) return;
          drag.current = { kind: "ring", id, axis: pick.axis, origin, startAngle: a, last: a, turned: 0, start };
          gizmo.setActive(pick);
        } else if (pick?.kind === "axis") {
          const startParam = closestParam(r.ray, origin, UNIT[pick.axis]);
          if (startParam === null) return;
          drag.current = { kind: "axis", id, axis: pick.axis, origin, startParam, start, targets };
          gizmo.setActive(pick);
        } else if (pick?.kind === "plane") {
          const hit = r.ray.intersectPlane(new THREE.Plane().setFromNormalAndCoplanarPoint(UNIT[pick.normal], origin), new THREE.Vector3());
          if (!hit) return;
          drag.current = { kind: "plane", id, normal: pick.normal, origin, startHit: hit, start, targets };
          gizmo.setActive(pick);
        } else if ((pick?.kind === "center" || (e.altKey && !e.shiftKey && hitSelectedBody(r))) && !callbacks.current.entities.get(id)?.placed?.by) {
          // A plain press on the part only picks (a face, a piece, the part behind);
          // it moves from the centre handle, or with Alt held on its body.
          const hit = r.ray.intersectPlane(new THREE.Plane(new THREE.Vector3(0, 0, 1), -start.z), new THREE.Vector3());
          if (!hit) return;
          // Not moved until the pointer travels, even from the centre handle:
          // a click that lands on the handle still picks the face or part under it.
          drag.current = { kind: "ground", id, offset: new THREE.Vector2(hit.x - start.x, hit.y - start.y), start, downX: e.clientX, downY: e.clientY, moved: false, targets };
          if (pick) gizmo.setActive(pick);
        } else {
          return;
        }
      }
      e.stopPropagation();
      e.preventDefault();
      el.setPointerCapture(e.pointerId);
      el.classList.add("is-dragging");
      controls.enabled = false;
      dirty = true;
    };

    // Where the pointer is on the model, a few times a second, for teammates.
    let hoverSent = 0;
    let hoverWas: string | null = null;
    const reportHover = (e: PointerEvent) => {
      const report = callbacks.current.onHover3D;
      const now = performance.now();
      if (!report || now - hoverSent < 120) return;
      hoverSent = now;
      const bodies = [...(three.current?.parts.values() ?? [])].flatMap((p) => p.meshes.filter((m) => m.visible));
      const hit = rayAt(e.clientX, e.clientY).intersectObjects(bodies, true)[0];
      const at: [number, number, number] | null = hit ? [hit.point.x, hit.point.y, hit.point.z].map((v) => Math.round(v * 10) / 10) as [number, number, number] : null;
      const key = JSON.stringify(at);
      if (key !== hoverWas) {
        hoverWas = key;
        report(at);
      }
    };
    const onMove = (e: PointerEvent) => {
      if (tools.move(e, rayAt)) return;
      reportHover(e);
      if (connectorLayer.active && !(e.buttons & 1)) {
        const rect = renderer.domElement.getBoundingClientRect();
        const key = connectorLayer.hit(toScreen, e.clientX - rect.left, e.clientY - rect.top);
        if (connectorLayer.setHover(key)) dirty = true;
        renderer.domElement.style.cursor = key ? "pointer" : "";
        return;
      }
      const cm = callbacks.current.comments;
      if (liveStroke && cm) {
        // Markup follows the surface under the pointer; off the model it stays in the plane facing the eye.
        const r = rayAt(e.clientX, e.clientY);
        const hit = r.intersectObjects(visibleBodies(), false)[0];
        const last = liveStroke.points[liveStroke.points.length - 1];
        let at: THREE.Vector3 | null = null;
        if (hit) {
          const n = hit.face ? hit.face.normal.clone().transformDirection(hit.object.matrixWorld) : new THREE.Vector3();
          at = hit.point.clone().addScaledVector(n, camera.position.distanceTo(hit.point) * 0.0015);
        } else {
          const plane = new THREE.Plane().setFromNormalAndCoplanarPoint(camera.getWorldDirection(new THREE.Vector3()), last);
          at = r.ray.intersectPlane(plane, new THREE.Vector3());
        }
        if (at && liveStroke.points.length < STROKE_POINTS) {
          const a = toScreen(at);
          const b = toScreen(last);
          if (Math.hypot(a.x - b.x, a.y - b.y) >= STROKE_STEP) {
            liveStroke.points.push(at);
            dirty = true;
          }
        }
        return;
      }
      if (cm?.placing && !cm.tool && !drag.current) {
        // A ghost pin under the pointer shows where a click would pin the comment.
        const ghost = pinGhost.current;
        const hit = onCommentUi(e) ? undefined : rayAt(e.clientX, e.clientY).intersectObjects(visibleBodies(), false)[0];
        if (ghost) {
          if (hit) {
            const q = toScreen(hit.point);
            ghost.style.display = "block";
            ghost.style.transform = `translate(${q.x.toFixed(1)}px, ${q.y.toFixed(1)}px)`;
          } else ghost.style.display = "none";
        }
      } else if (pinGhost.current) pinGhost.current.style.display = "none";
      if (callbacks.current.sketch) {
        sketchPointer("move", e);
        return;
      }
      const mg = motionGrab.current;
      const mpart = mg ? three.current?.parts.get(mg.id) : undefined;
      if (motionDrag && mg && mpart) {
        const rect = renderer.domElement.getBoundingClientRect();
        const pivot = mpart.body.localToWorld(new THREE.Vector3(...mg.at));
        const axis = new THREE.Vector3(...mg.axis).transformDirection(mpart.body.matrixWorld);
        const p0 = toScreen(pivot);
        const [ax, ay] = [motionDrag.x - rect.left, motionDrag.y - rect.top];
        const [bx, by] = [e.clientX - rect.left, e.clientY - rect.top];
        if (mg.slide) {
          // Millimetres along the axis as drawn on screen.
          const p1 = toScreen(pivot.clone().addScaledVector(axis, 10));
          const v = [p1.x - p0.x, p1.y - p0.y];
          const len2 = v[0] * v[0] + v[1] * v[1];
          if (len2 > 1) mg.onDelta((((bx - ax) * v[0] + (by - ay) * v[1]) / len2) * 10);
        } else {
          // The angle swept round the axis on screen (y runs down, so anticlockwise is negative),
          // turned the right way for whether the axis points at the camera.
          const a1 = Math.atan2(ay - p0.y, ax - p0.x);
          const a2 = Math.atan2(by - p0.y, bx - p0.x);
          let d = a2 - a1;
          d = d - 2 * Math.PI * Math.floor((d + Math.PI) / (2 * Math.PI));
          const facing = axis.dot(camera.position.clone().sub(pivot)) >= 0 ? 1 : -1;
          mg.onDelta(THREE.MathUtils.radToDeg(-d) * facing);
        }
        motionDrag = { x: e.clientX, y: e.clientY };
        return;
      }
      if (callbacks.current.edges) {
        const key = e.buttons === 0 ? edgeUnder(e) : null;
        if (key !== edgeLayer.hovered) {
          edgeLayer.setHover(key);
          el.classList.toggle("over-handle", !!key);
          dirty = true;
        }
        return;
      }
      const d = drag.current;
      if (!d) {
        if (e.buttons !== 0 || !callbacks.current.selected) return;
        const r = rayAt(e.clientX, e.clientY);
        if (callbacks.current.pivotMode) {
          const gizmoHit = pivotGizmo.root.visible ? r.intersectObjects(pivotGizmo.hits(), false)[0] : undefined;
          const pick = gizmoHit ? pivotGizmo.pickOf(gizmoHit.object) : null;
          pivotGizmo.setHover(pick);
          const frac = pick ? null : pickDot(r);
          if (JSON.stringify(frac) !== JSON.stringify(hoverDot)) hoverDot = frac;
          const body = selectedBody();
          const overSurface = !pick && !frac && !!body && r.intersectObjects(body, false).length > 0;
          el.classList.toggle("over-handle", !!pick || !!frac);
          el.classList.toggle("over-surface", overSurface);
          dirty = true;
          return;
        }
        const pick = pickGizmo(r);
        gizmo.setHover(pick);
        const overPush = pickPush(r);
        if (overPush !== hoverPush) {
          hoverPush = overPush;
          push.setHover(overPush);
        }
        const next = pick || overPush ? null : hitSelectedFace(r);
        hoverFace = next;
        el.classList.toggle("over-handle", !!pick || overPush || (!!face.current && sameFace(next, face.current)));
        el.classList.toggle("over-face", !pick && !overPush && !!next && !sameFace(next, face.current));
        dirty = true;
        return;
      }
      e.stopPropagation();
      const r = rayAt(e.clientX, e.clientY);
      const step = snapStep(e);
      const s = d.start;
      if (d.kind === "pivot") {
        let p: THREE.Vector3;
        let axes: Axis[];
        if (d.pick.kind === "axis") {
          const t = closestParam(r.ray, d.origin, UNIT[d.pick.axis]);
          if (t === null) return;
          p = d.origin.clone().addScaledVector(UNIT[d.pick.axis], snap(t - d.startParam, step));
          axes = [d.pick.axis];
        } else {
          const hit = r.ray.intersectPlane(d.plane!, new THREE.Vector3());
          if (!hit) return;
          const normal = d.pick.kind === "plane" ? d.pick.normal : null;
          axes = ([0, 1, 2] as Axis[]).filter((k) => k !== normal);
          p = d.origin.clone();
          for (const k of axes) p.setComponent(k, d.origin.getComponent(k) + snap(hit.getComponent(k) - d.startHit.getComponent(k), step));
        }
        d.moved = true;
        pivotPreview.current = fracOf(s, pointMagnet(p, axes, d.targets, e.metaKey || e.ctrlKey));
        dirty = true;
        return;
      }
      if (d.kind === "ring") {
        const a = ringAngle(r, d.origin, d.axis);
        if (a === null) return;
        // Unwrapped, so turning past half a turn keeps counting.
        let delta = a - d.last;
        delta -= 360 * Math.round(delta / 360);
        d.last += delta;
        const unit = e.shiftKey ? 45 : e.altKey ? 1 : 15;
        const turned = snap(d.last - d.startAngle, unit);
        if (turned !== d.turned) {
          d.turned = turned;
          turnPreview(d.id, d.axis, turned, d.origin);
        }
        return;
      }
      if (d.kind === "axis") {
        const t = closestParam(r.ray, d.origin, UNIT[d.axis]);
        if (t === null) return;
        const k = POS_KEYS[d.axis];
        callbacks.current.onDrag(d.id, magnet({ [k]: s[k] + snap(t - d.startParam, step) }, s, [d.axis], d.targets, e.metaKey || e.ctrlKey));
      } else if (d.kind === "plane") {
        const hit = r.ray.intersectPlane(new THREE.Plane().setFromNormalAndCoplanarPoint(UNIT[d.normal], d.origin), new THREE.Vector3());
        if (!hit) return;
        const patch: Partial<Shape> = {};
        for (const k of [0, 1, 2] as Axis[]) {
          if (k === d.normal) continue;
          patch[POS_KEYS[k]] = s[POS_KEYS[k]] + snap(hit.getComponent(k) - d.startHit.getComponent(k), step);
        }
        callbacks.current.onDrag(d.id, magnet(patch, s, ([0, 1, 2] as Axis[]).filter((k) => k !== d.normal), d.targets, e.metaKey || e.ctrlKey));
      } else if (d.kind === "facePush") {
        const t = closestParam(r.ray, d.origin, d.normal);
        if (t === null) return;
        const distance = snap(t - d.startParam, step);
        if (distance !== d.distance) {
          d.distance = distance;
          faceLayer.slide(distance);
          callbacks.current.onFaceDrag(distance);
          dirty = true;
        }
      } else if (d.kind === "ground") {
        if (!d.moved && Math.hypot(e.clientX - d.downX, e.clientY - d.downY) < 3) return;
        d.moved = true;
        const hit = r.ray.intersectPlane(new THREE.Plane(new THREE.Vector3(0, 0, 1), -s.z), new THREE.Vector3());
        if (!hit) return;
        callbacks.current.onDrag(d.id, magnet({ x: snap(hit.x - d.offset.x, step), y: snap(hit.y - d.offset.y, step) }, s, [0, 1], d.targets, e.metaKey || e.ctrlKey));
      } else {
        const t = closestParam(r.ray, d.origin, UNIT[d.face.axis]);
        if (t === null) return;
        const k = d.face.axis;
        const sizeKey = SIZE_KEYS[k];
        const posKey = POS_KEYS[k];
        const delta = snap(t - d.startParam, step) * d.face.sign;
        const size = Math.max(MIN_SIZE, s[sizeKey] + delta);
        // A minus-side face moves the part's origin; the opposite face stays put.
        callbacks.current.onDrag(d.id, d.face.sign > 0 ? { [sizeKey]: size } : { [sizeKey]: size, [posKey]: s[posKey] + s[sizeKey] - size });
      }
    };

    const endDrag = (commit: boolean) => {
      const d = drag.current;
      if (!d) return;
      if (d.kind === "ring") {
        drag.current = null;
        gizmo.setActive(null);
        el.classList.remove("is-dragging");
        controls.enabled = true;
        dirty = true;
        const e = callbacks.current.entities.get(d.id);
        if (!commit || d.turned === 0 || !e || !callbacks.current.onTurnInstance) {
          turnPreview(d.id, d.axis, 0, d.origin);
          return;
        }
        // Turned about the pivot: q' = qΔ·q, at' = pivot + qΔ·(at − pivot).
        const own = placedOf(e);
        const qd = new THREE.Quaternion().setFromAxisAngle(UNIT[d.axis], THREE.MathUtils.degToRad(d.turned));
        const q = qd.clone().multiply(new THREE.Quaternion(...own.q));
        const at = new THREE.Vector3(...own.at).sub(d.origin).applyQuaternion(qd).add(d.origin);
        const eu = new THREE.Euler().setFromQuaternion(q, "ZYX");
        const deg = (v: number) => Math.round(THREE.MathUtils.radToDeg(v) * 1e6) / 1e6 || 0;
        const r3 = (v: number) => Math.round(v * 1e4) / 1e4;
        void callbacks.current.onTurnInstance(d.id, [r3(at.x), r3(at.y), r3(at.z)], [deg(eu.x), deg(eu.y), deg(eu.z)]).then((ok) => {
          if (!ok) turnPreview(d.id, d.axis, 0, d.origin);
        });
        return;
      }
      if (d.kind === "facePush") {
        drag.current = null;
        el.classList.remove("is-dragging");
        controls.enabled = true;
        dirty = true;
        callbacks.current.onFaceDragEnd(commit, d.distance);
        return;
      }
      if (d.kind === "pivot") {
        drag.current = null;
        snapped.current = null;
        pivotGizmo.setActive(null);
        el.classList.remove("is-dragging");
        controls.enabled = true;
        dirty = true;
        if (commit && d.moved && pivotPreview.current) {
          // Stay in pivot mode so the user can keep refining; the preview holds until the engine confirms.
          callbacks.current.onPickPivot(pivotPreview.current, true);
        } else {
          pivotPreview.current = null;
        }
        return;
      }
      const moved = d.kind !== "ground" || d.moved;
      drag.current = null;
      snapped.current = null;
      gizmo.setActive(null);
      el.classList.remove("is-dragging");
      controls.enabled = true;
      dirty = true;
      callbacks.current.onDragEnd(commit && moved);
    };
    const onUp = (e: PointerEvent) => {
      if (tools.boxUp(e)) return;
      if (liveStroke) {
        const st = liveStroke;
        liveStroke = null;
        e.stopPropagation();
        if (el.hasPointerCapture(st.pointer)) el.releasePointerCapture(st.pointer);
        controls.enabled = true;
        dirty = true;
        const cm = callbacks.current.comments;
        const target = cm?.pins.find((p) => p.draft || p.id === cm.active)?.target ?? null;
        if (cm && st.points.length >= 2) {
          const pts = st.points.map((w) => (target ? pinLocal(target, w) : w)).filter((v): v is THREE.Vector3 => !!v);
          if (pts.length >= 2) cm.onStroke({ points: pts.map(round3), arrow: st.arrow });
        }
        return;
      }
      if (callbacks.current.sketch) {
        if (e.button === 0) sketchPointer("up", e);
        return;
      }
      if (motionDrag) {
        motionDrag = null;
        e.stopPropagation();
        if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
        controls.enabled = true;
        el.classList.remove("is-dragging");
        return;
      }
      if (!drag.current) return;
      e.stopPropagation();
      if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
      const d = drag.current;
      endDrag(true);
      // A click (no movement) on a face of the selected part selects that face.
      if (d.kind === "ground" && !d.moved) {
        const r = rayAt(e.clientX, e.clientY);
        if (selectedIsProgram()) {
          const pf = programFaceAt(r);
          if (pf !== undefined) {
            if (pf) callbacks.current.onProgramFace(pf);
            else callbacks.current.onProgramFace(null, "curved");
            dirty = true;
            return;
          }
        }
        const f = hitSelectedFace(r);
        if (f) {
          face.current = f;
          hoverFace = null;
          dirty = true;
        }
        if (!drill(r)) pickAssemblyPart(r);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (callbacks.current.pivotMode && !drag.current) {
        e.stopPropagation();
        e.preventDefault();
        hoverDot = null;
        callbacks.current.onPickPivot(null);
        dirty = true;
        return;
      }
      if (drag.current) {
        e.stopPropagation();
        e.preventDefault();
        endDrag(false);
      } else if (face.current) {
        // Step back from face to part.
        e.stopPropagation();
        e.preventDefault();
        face.current = null;
        dirty = true;
      }
    };

    /**
     * Inside instances, each click goes one level deeper: the tower, then the
     * floor under the pointer, then what is in it. True when it selected something.
     */
    const drill = (r: THREE.Raycaster) => {
      const bodies = [...(three.current?.parts.values() ?? [])].flatMap((p) => p.meshes.filter((m) => m.visible));
      const hit = r.intersectObjects(bodies, false)[0];
      const path = hit?.object.userData.path as string[] | undefined;
      if (!path?.length) return false;
      const sel = callbacks.current.selected;
      const at = sel ? path.indexOf(sel) : -1;
      const next = at < 0 ? path[0] : path[Math.min(at + 1, path.length - 1)];
      if (next === sel) return false;
      face.current = null;
      callbacks.current.onSelect(next);
      dirty = true;
      return true;
    };

    // Inside a selected assembly, a click picks the part under the pointer
    // (or clears the pick when it is clicked again).
    /**
     * The flat face of the selected program part under the pointer: null on a
     * curved surface, undefined when the click means something else (picking
     * a part of an assembly first).
     */
    const programFaceAt = (r: THREE.Raycaster): ProgramFace | null | undefined => {
      const id = callbacks.current.selected;
      const part = id ? three.current?.parts.get(id) : undefined;
      if (!id || !part || part.shared) return undefined;
      // Into turning or exploded bodies too: they share the part's triangle
      // numbering, and each is drawn moved, so the hit is taken back into
      // the part's own coordinates through the body that was hit.
      const hit = r.intersectObject(part.body, true)[0];
      if (!hit || hit.faceIndex == null) return undefined;
      const parts = assemblyParts(callbacks.current.entities.get(id));
      let range: [number, number] | undefined;
      let name: string | null = null;
      if (parts.length) {
        const p = parts.find((q) => hit.faceIndex! * 3 >= q.index_start && hit.faceIndex! * 3 < q.index_start + q.index_count);
        if (!p || (p.name !== callbacks.current.selectedPart && !callbacks.current.pickableParts?.includes(p.name))) return undefined; // pick the part first
        range = [p.index_start, p.index_count];
        name = p.name;
      }
      const local = hit.object.worldToLocal(hit.point.clone());
      const face = flatFaceAt(part.body.geometry, hit.faceIndex, range) ?? curvedFaceAt(part.body.geometry, hit.faceIndex, local, range);
      return face ? { ...face, id, part: name } : null;
    };

    const pickAssemblyPart = (r: THREE.Raycaster) => {
      const id = callbacks.current.selected;
      const body = id ? three.current?.parts.get(id)?.body : undefined;
      const parts = assemblyParts(id ? callbacks.current.entities.get(id) : undefined);
      if (!body || !parts.length) return;
      // Into turning bodies too (children of the body, same triangle numbering).
      const hit = r.intersectObject(body, true).find((h) => {
        // Skip parts shown as X-ray: clicking through them reaches what is inside.
        const g = parts.findIndex((q) => h.faceIndex! * 3 >= q.index_start && h.faceIndex! * 3 < q.index_start + q.index_count);
        return g >= 0 && (!callbacks.current.selectedPart || parts[g].name === callbacks.current.selectedPart || bodyMaterials(body)[g].opacity >= 0.5);
      });
      const p = hit?.faceIndex != null ? parts.find((q) => hit.faceIndex! * 3 >= q.index_start && hit.faceIndex! * 3 < q.index_start + q.index_count) : undefined;
      callbacks.current.onSelectPart(p && p.name !== callbacks.current.selectedPart ? p.name : null);
    };

    // Plain click (not a drag) on the canvas selects a part, a face of the selected part, or clears.
    let down: { x: number; y: number } | null = null;
    const onCanvasDown = (e: PointerEvent) => {
      if (callbacks.current.sketch || callbacks.current.edges) return;
      down = { x: e.clientX, y: e.clientY };
      renderer.domElement.classList.add("dragging");
      controls.autoRotate = false;
    };
    const onCanvasUp = (e: PointerEvent) => {
      if (callbacks.current.sketch || callbacks.current.edges || connectorLayer.active) return;
      renderer.domElement.classList.remove("dragging");
      if (tools.click(e, rayAt)) {
        down = null;
        return;
      }
      // Each press makes at most one click: a press that ended elsewhere must
      // not pair with a later release near the same spot.
      const from = down;
      down = null;
      if (!from || Math.hypot(e.clientX - from.x, e.clientY - from.y) > 4) return;
      const r = rayAt(e.clientX, e.clientY);
      const cm = callbacks.current.comments;
      if (cm?.tool) return;
      if (cm?.placing) {
        const hit = r.intersectObjects(visibleBodies(), false)[0];
        const target = hit ? (hit.object.userData.id as string | undefined) : undefined;
        if (!hit || !target) return cm.onOpen(null);
        const local = pinLocal(target, hit.point);
        if (!local) return;
        const nWorld = hit.face ? hit.face.normal.clone().transformDirection(hit.object.matrixWorld) : null;
        const f = instanceFrame(target);
        const normal = nWorld ? (f ? nWorld.applyQuaternion(new THREE.Quaternion(...f.q).invert()) : nWorld) : null;
        // Which part of a multi-part shape, by the triangle hit.
        const parts = assemblyParts(callbacks.current.entities.get(target));
        const own = three.current?.parts.get(target);
        const tri = hit.faceIndex != null && own && !own.shared ? hit.faceIndex * 3 : -1;
        const named = tri >= 0 ? parts.find((q) => tri >= q.index_start && tri < q.index_start + q.index_count)?.name : undefined;
        const anchor: PinAnchor = { target, part: named ?? null, point: round3(local), normal: normal ? round3(normal.normalize()) : null };
        cm.onPlace(anchor);
        return;
      }
      if (cm?.active) cm.onOpen(null);
      if (drill(r)) return;
      const bodies = [...(three.current?.parts.values() ?? [])].flatMap((p) => p.meshes.filter((m) => m.visible));
      const hit = r.intersectObjects(bodies, true)[0];
      const id = hit ? (hit.object.userData.id as string) : null;
      if (e.shiftKey && id) {
        // Shift-click: add to (or take from) the group.
        face.current = null;
        callbacks.current.onSelect(id, true);
      } else if (id && id === callbacks.current.selected) {
        face.current = selectedIsProgram() ? null : faceFromHit(hit!);
        pickAssemblyPart(r);
      } else {
        face.current = null;
        callbacks.current.onSelect(id, false, e.altKey);
      }
      dirty = true;
    };

    el.addEventListener("pointerdown", onDown, true);
    const onDouble = (e: MouseEvent) => {
      if (onCommentUi(e)) return;
      if (callbacks.current.sketch) sketchPointer("double", e);
    };
    el.addEventListener("dblclick", onDouble, true);
    el.addEventListener("pointermove", onMove, true);
    el.addEventListener("pointerup", onUp, true);
    el.addEventListener("pointercancel", () => endDrag(false), true);
    window.addEventListener("keydown", onKey, true);
    renderer.domElement.addEventListener("pointerdown", onCanvasDown);
    renderer.domElement.addEventListener("pointerup", onCanvasUp);

    three.current = { renderer, scene, camera, controls, tumble: tumbler, sun, parts: new Map(), gizmo, push, overlayRoot, overlayLabels: [], ground, grid, gridCell: 20, wireframe: false, invalidate, flight: null, extent: new THREE.Sphere(new THREE.Vector3(), 100) };
    scene.add(sketchLayer.root, draftLayer.root, edgeLayer.root, faceLayer.root, pickedLayer.root, connectorLayer.root);
    if (sketchLabels.current) sketchLayer.setHost(sketchLabels.current);
    if (draftLabels.current) draftLayer.setHost(draftLabels.current);
    draftLayer.onSelect = (id) => callbacks.current.onSelectDraft?.(id);
    sketchLayer.onEditDimension = (key, x, y) => callbacks.current.onEditDimension(key, x, y);
    resize();

    return () => {
      cancelAnimationFrame(raf);
      window.clearTimeout(occlusionTimer);
      observer.disconnect();
      controls.dispose();
      tumbler.dispose();
      unwalk();
      tools.dispose();
      toolsRef.current = null;
      window.removeEventListener("keydown", onKey, true);
      for (const part of three.current?.parts.values() ?? []) disposePart(part);
      batch.dispose();
      scene.traverse((o) => {
        if (o instanceof THREE.Mesh || o instanceof THREE.Line) {
          o.geometry.dispose();
          (o.material as THREE.Material).dispose();
        }
      });
      renderer.dispose();
      renderer.domElement.remove();
      three.current = null;
    };
  }, []);

  // Sync meshes with engine state and any live edit.
  useEffect(() => {
    const t = three.current;
    if (!t) return;
    if (lastSelected.current !== selected) {
      face.current = null;
      lastSelected.current = selected;
    }
    // Drawn directly: boxes, placed shapes and top-level instances. Components
    // (definitions) are drawn only through their instances.
    pcbLooks.touch();
    const drawn = (e: EntityView | undefined) => !!e && (instance(e) ? !instance(e)!.parent : !!e.mesh && !isDefinition(e));
    for (const [id, part] of t.parts) {
      if (!drawn(entities.get(id))) {
        t.scene.remove(part.group);
        pcbLooks.detach(id);
        disposePart(part);
        t.parts.delete(id);
      }
    }
    const liveBase = live ? entities.get(live.id) : undefined;
    const lb = liveBase ? shapeOf(liveBase) : null;
    const groupDelta: [number, number, number] | null = live && lb ? [live.x - lb.x, live.y - lb.y, live.z - lb.z] : null;
    for (const e of entities.values()) {
      if (!drawn(e)) continue;
      let part = t.parts.get(e.id);
      const key = instance(e) ? instanceKey(e, entities) : null;
      const stale = key !== null ? part?.mesh !== key : part?.mesh !== e.mesh || part?.look !== lookKey(e, entities);
      if (!part || stale) {
        if (part) {
          t.scene.remove(part.group);
          disposePart(part);
        }
        part = (key !== null ? buildInstance(e, entities, t.wireframe, key) : buildPart(e, entities, t.wireframe)) ?? undefined;
        if (!part) continue;
        t.scene.add(part.group);
        t.parts.set(e.id, part);
      }
      // A board's 3D part is drawn as the board (mask, copper, silk, real bodies).
      if (!part.shared) pcbLooks.attach(e.id, part.group, part.meshes);
      // The engine's shape is the truth for placement, refreshed on every update. Only the
      // mesh is cached: it is rebuilt when the size changes, so its size always matches `base`.
      part.base = shapeOf(e);
      // A live edit scales and moves the cached mesh until the engine commits it;
      // the rest of a group follows the dragged item by the same distance.
      const s = live?.id === e.id ? live : groupDelta && props.also?.includes(e.id) ? { ...part.base, x: part.base.x + groupDelta[0], y: part.base.y + groupDelta[1], z: part.base.z + groupDelta[2] } : part.base;
      if (part.isProgram) part.group.scale.set(1, 1, 1);
      else part.group.scale.set(s.width / part.base.width || 1, s.depth / part.base.depth || 1, s.height / part.base.height || 1);
      // Meshes are in the part's own coordinates; shapes may extend below their origin.
      const off = originOffset(e);
      part.group.position.set(s.x - off[0], s.y - off[1], s.z - off[2]);
      if (part.group.userData.base && drag.current?.kind !== "ring") {
        // A turn the engine has now confirmed (or refused): drawn from its placement again.
        part.group.quaternion.identity();
        delete part.group.userData.base;
      }
    }
    for (const [id, part] of t.parts) {
      // A picked part shows by its glow; the whole assembly's outline would bury it.
      const mine = (id === selected || !!props.also?.includes(id)) && !props.selectedPart;
      const mate = mine ? undefined : props.teammates?.find((m) => m.selected === id);
      part.outline.visible = (mine || !!mate) && turning.current?.id !== id;
      (part.outline.material as THREE.LineBasicMaterial).color.set(mate ? mate.color : OUTLINE);
      // The picked part of an assembly glows; the rest of the assembly stays as built.
      const picked = id === selected && props.selectedPart ? assemblyParts(entities.get(id)).findIndex((p) => p.name === props.selectedPart) : -1;
      if (part.shared) {
        paintInstance(part, entities, selected, props.hidden, props.isolated, props.colorBy, props.tints, props.hovered);
        continue;
      }
      // "Show only" may name one piece of a part ("id#piece").
      const [isoId, isoPiece] = props.isolated ? [props.isolated.split("#")[0], props.isolated.includes("#") ? props.isolated.slice(props.isolated.indexOf("#") + 1) : null] : [null, null];
      part.group.visible = !props.hidden.has(id) && (!isoId || isoId === id);
      const pieces = assemblyParts(entities.get(id));
      const hov = props.hovered ?? null;
      const hotAll = hov === id && id !== selected;
      const hotPiece = hov?.startsWith(`${id}#`) ? hov.slice(id.length + 1) : null;
      // The rest of the assembly turns to X-ray so a part inside stays visible.
      bodyMaterials(part.body).forEach((m, i) => {
        const base = m.userData.look as Look;
        const ghost = picked >= 0 && i !== picked;
        const opacity = ghost ? Math.min(base.opacity ?? 1, 0.14) : (base.opacity ?? 1);
        // A layer's colour paints its parts; without one, the part's own look.
        m.color.set(props.colorBy === "material" ? (props.tints?.get(id) ?? base.color) : m.color);
        // The piece (or whole part) under the pointer in the components tree glows softly.
        const hot = i !== picked && (hotAll || (!!hotPiece && !!pieces[i] && (pieces[i].name === hotPiece || pieces[i].name.startsWith(`${hotPiece}/`))));
        m.emissive.set(i === picked ? ACCENT : hot ? HOVER : "#000000");
        m.emissiveIntensity = i === picked ? 0.45 : hot ? HOVER_GLOW : 0;
        m.transparent = opacity < 1;
        m.opacity = opacity;
        m.depthWrite = opacity >= 1;
        // A piece switched off in the tree (or not the one shown alone) is not drawn.
        const piece = pieces[i]?.name;
        m.visible = !piece || (!props.hidden.has(subPartKey(id, piece)) && (!isoPiece || isoPiece === piece));
        m.needsUpdate = true;
      });
      part.body.castShadow = !(id === selected && props.selectedPart);
    }
    const sel = selected ? t.parts.get(selected) : undefined;
    shape.current = sel ? (live?.id === selected ? { ...live } : { ...sel.base }) : null;
    pivot.current = pivotOf(selected ? entities.get(selected) : undefined);
    if (!drag.current) pivotPreview.current = null;

    // Fit the shadow camera to the parts.
    const bounds = new THREE.Box3();
    for (const part of t.parts.values()) bounds.expandByObject(part.group);
    if (!bounds.isEmpty()) bounds.getBoundingSphere(t.extent);
    // Anything below the ground shows through it, dimmed, instead of vanishing behind it.
    const buried = !bounds.isEmpty() && bounds.min.z < -1;
    const groundMat = t.ground.material as THREE.MeshStandardMaterial;
    if (groundMat.transparent !== buried) {
      groundMat.transparent = buried;
      groundMat.opacity = buried ? 0.55 : 1;
      groundMat.depthWrite = !buried;
      groundMat.needsUpdate = true;
    }
    if (!bounds.isEmpty()) {
      fitGround(t, bounds);
      // The first time a project's parts arrive, frame them: a tower and a bolt both open in view.
      if (!framed.current) {
        framed.current = true;
        fitView(t);
      }
      const c = bounds.getCenter(new THREE.Vector3());
      const radius = Math.max(bounds.getBoundingSphere(new THREE.Sphere()).radius, 50) * 1.6;
      t.sun.position.copy(c).add(new THREE.Vector3(-0.45, -0.65, 1).multiplyScalar(radius * 2));
      t.sun.target.position.copy(c);
      const cam = t.sun.shadow.camera;
      cam.left = cam.bottom = -radius;
      cam.right = cam.top = radius;
      cam.near = 1;
      cam.far = radius * 5;
      cam.updateProjectionMatrix();
    }
    t.invalidate();
  }, [entities, selected, live, props.also, props.pivotMode, props.pivotLive, props.selectedPart, props.hidden, props.isolated, props.colorBy, props.teammates, props.tints, props.hovered]);

  // Draft objects: redrawn when they or the selection change.
  useEffect(() => {
    draftLayer.update(props.draft ?? null);
    three.current?.invalidate();
  }, [props.draft, draftLayer]);

  // The sketch being edited: redrawn on every change; orbiting waits while sketching.
  useEffect(() => {
    sketchLayer.update(props.sketch);
    const t = three.current;
    if (t) {
      t.tumble.enabled = !props.sketch;
      t.invalidate();
    }
  }, [props.sketch, sketchLayer]);

  // A program part's face picked for push and pull, drawn where the part sits.
  useEffect(() => {
    const t = three.current;
    const pf = props.programFace;
    const at = pf ? (t?.parts.get(pf.id)?.group.position ?? null) : null;
    faceLayer.show(pf, at, pf ? facePose(pf) : null);
    t?.invalidate();
  }, [props.programFace, faceLayer, facePose]);

  // Faces picked for a shell or draft: drawn where the part sits.
  useEffect(() => {
    const t = three.current;
    const fp = props.facePicks;
    pickedLayer.update(fp, fp ? (t?.parts.get(fp.id)?.group.position ?? null) : null);
    t?.invalidate();
  }, [props.facePicks, entities, pickedLayer]);

  // Edges being picked: drawn where the part sits (which can move while picking).
  useEffect(() => {
    const t = three.current;
    const origin = props.edges ? (t?.parts.get(props.edges.id)?.group.position ?? null) : null;
    edgeLayer.update(props.edges, origin);
    if (!props.edges) host.current?.classList.remove("over-handle");
    t?.invalidate();
  }, [props.edges, entities, edgeLayer]);

  // Connectors to pick a joint's ends from.
  useEffect(() => {
    connectorLayer.update(props.connectors ?? null);
    if (!props.connectors && three.current) three.current.renderer.domElement.style.cursor = "";
    three.current?.invalidate();
  }, [props.connectors, connectorLayer]);

  // Instances at the placements a joint being scrubbed or swept gives them (or back at their own).
  useEffect(() => {
    const t = three.current;
    if (!t) return;
    for (const [id, part] of t.parts) {
      if (!part.shared) continue;
      const placed = new Map(occurrences(id, entities, props.poses ?? null).map((o) => [`${o.path.join("/")}|${o.shape}`, o.transform]));
      for (const m of part.meshes) {
        const tr = placed.get(`${(m.userData.path as string[]).join("/")}|${m.userData.shape}`);
        if (!tr) continue;
        m.position.set(...tr.at);
        m.quaternion.set(...tr.q);
      }
    }
    t.invalidate();
  }, [props.poses, entities]);

  // Lines and dots an assembly tool draws over the model (never picked: clicks reach the parts).
  // A scan (points, mesh, deviation colours, problem markers), drawn in place.
  const scanGroup = useRef<THREE.Group | null>(null);
  useEffect(() => {
    const t = three.current;
    if (!t) return;
    if (scanGroup.current) {
      t.scene.remove(scanGroup.current);
      disposeScanLayer(scanGroup.current);
      scanGroup.current = null;
    }
    if (props.scan) {
      scanGroup.current = buildScanLayer(props.scan);
      t.scene.add(scanGroup.current);
    }
    t.invalidate();
  }, [props.scan]);

  const assemblyOverlay = useRef<THREE.Group | null>(null);
  useEffect(() => {
    const t = three.current;
    if (!t) return;
    let g = assemblyOverlay.current;
    if (!g) {
      g = new THREE.Group();
      g.renderOrder = 3;
      assemblyOverlay.current = g;
      t.scene.add(g);
    }
    for (const o of [...g.children]) {
      g.remove(o);
      const m = o as THREE.Line | THREE.Points;
      m.geometry.dispose();
      (m.material as THREE.Material).dispose();
    }
    const ov = props.overlay;
    for (const l of ov?.lines ?? []) {
      if (l.points.length < 2) continue;
      const geometry = new THREE.BufferGeometry().setFromPoints(l.points.map((p) => new THREE.Vector3(...p)));
      const material = l.dashed
        ? new THREE.LineDashedMaterial({ color: l.color ?? "#6d4ae0", dashSize: 3, gapSize: 2, depthTest: false, transparent: true, opacity: 0.9 })
        : new THREE.LineBasicMaterial({ color: l.color ?? "#6d4ae0", depthTest: false, transparent: true, opacity: 0.9 });
      const line = new THREE.Line(geometry, material);
      if (l.dashed) line.computeLineDistances();
      line.raycast = () => {};
      line.renderOrder = 3;
      g.add(line);
    }
    for (const set of ov?.points ?? []) {
      if (!set.at.length) continue;
      const geometry = new THREE.BufferGeometry().setFromPoints(set.at.map((p) => new THREE.Vector3(...p)));
      const dots = new THREE.Points(geometry, new THREE.PointsMaterial({ color: set.color ?? "#6d4ae0", size: set.size ?? 3, sizeAttenuation: false, depthTest: false, transparent: true, opacity: 0.85 }));
      dots.raycast = () => {};
      dots.renderOrder = 3;
      g.add(dots);
    }
    t.invalidate();
  }, [props.overlay]);

  // Comment pins and the open card: laid out again when they change, and when the card grows (a reply posted).
  useEffect(() => void three.current?.invalidate(), [props.comments]);
  useEffect(() => {
    const card = cardAnchor.current;
    if (!card) return;
    const observer = new ResizeObserver(() => three.current?.invalidate());
    observer.observe(card);
    return () => observer.disconnect();
  }, []);
  useEffect(() => void host.current?.classList.toggle("is-commenting", !!props.comments?.placing && !props.comments.tool), [props.comments?.placing, props.comments?.tool]);
  useEffect(() => void host.current?.classList.toggle("is-marking", !!props.comments?.tool), [props.comments?.tool]);

  // Isolating something (or showing everything again) frames what is now in view.
  const lastIsolated = useRef<string | null>(null);
  useEffect(() => {
    if (lastIsolated.current === props.isolated) return;
    lastIsolated.current = props.isolated;
    if (three.current) {
      fitView(three.current);
      three.current.invalidate();
    }
  }, [props.isolated]);

  // Intent overlays: rebuilt when requirements or the displayed shapes change.
  useEffect(() => {
    const t = three.current;
    const layer = labelLayer.current;
    if (!t || !layer) return;
    t.overlayRoot.traverse((o) => {
      if (o instanceof THREE.Mesh || o instanceof THREE.Line) {
        o.geometry.dispose();
        (o.material as THREE.Material).dispose();
      }
    });
    t.overlayRoot.clear();
    layer.replaceChildren();
    t.overlayLabels = [];

    const onTop = <M extends THREE.Material & { fog?: boolean }>(m: M) => {
      m.depthTest = false;
      m.depthWrite = false;
      m.transparent = true;
      m.toneMapped = false;
      if ("fog" in m) m.fog = false;
      return m;
    };
    const tag = (text: string, at: THREE.Vector3, tone: "ok" | "bad" | "warn") => {
      const el = document.createElement("div");
      el.className = `intent-tag ${tone}`;
      el.textContent = text;
      layer.appendChild(el);
      t.overlayLabels.push({ el, at });
    };
    const v = (p: number[]) => new THREE.Vector3(p[0], p[1], p[2]);
    const boxGeometry = (min: number[], max: number[]) => {
      const g = new THREE.BoxGeometry(max[0] - min[0], max[1] - min[1], max[2] - min[2]);
      g.translate((min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2);
      return g;
    };

    for (const o of overlays) {
      if (o.kind === "gap") {
        const color = o.ok ? "#3c9d67" : "#df5b63";
        const from = v(o.from);
        const to = v(o.to);
        if (from.distanceTo(to) < 1e-6) continue;
        const line = new THREE.Line(
          new THREE.BufferGeometry().setFromPoints([from, to]),
          onTop(new THREE.LineDashedMaterial({ color, dashSize: 1.6, gapSize: 1.1, opacity: o.focus ? 1 : 0.85 })),
        );
        line.computeLineDistances();
        line.renderOrder = 900;
        t.overlayRoot.add(line);
        for (const end of [from, to]) {
          const dot = new THREE.Mesh(new THREE.SphereGeometry(0.9, 12, 8), onTop(new THREE.MeshBasicMaterial({ color })));
          dot.position.copy(end);
          dot.renderOrder = 901;
          t.overlayRoot.add(dot);
        }
        tag(o.label, from.clone().lerp(to, 0.5), o.ok ? "ok" : "bad");
      } else if (o.kind === "region") {
        const geometry = boxGeometry(o.box.min, o.box.max);
        const fillMesh = new THREE.Mesh(geometry, onTop(new THREE.MeshBasicMaterial({ color: "#ef4f5f", opacity: 0.32 })));
        fillMesh.renderOrder = 902;
        const edges = new THREE.LineSegments(new THREE.EdgesGeometry(geometry), onTop(new THREE.LineBasicMaterial({ color: "#d63a4c", opacity: 0.95 })));
        edges.renderOrder = 903;
        t.overlayRoot.add(fillMesh, edges);
        if (o.label) tag(`Overlap ${o.label}`, v(o.box.max).add(v(o.box.min)).multiplyScalar(0.5), "bad");
      } else {
        const geometry = boxGeometry(o.box.min, o.box.max);
        const edges = new THREE.LineSegments(
          new THREE.EdgesGeometry(geometry),
          onTop(new THREE.LineDashedMaterial({ color: o.ok ? "#8b6fd8" : "#df5b63", dashSize: 2.4, gapSize: 1.6, opacity: o.focus ? 0.95 : 0.5 })),
        );
        edges.computeLineDistances();
        edges.renderOrder = 899;
        geometry.dispose();
        t.overlayRoot.add(edges);
        if (!o.ok || o.focus) tag(o.ok ? "Envelope" : "Outside envelope", v(o.box.max), o.ok ? "ok" : "bad");
      }
    }
    t.invalidate();
  }, [overlays]);

  useImperativeHandle(ref, () => ({
    fit() {
      if (three.current) fitView(three.current);
    },
    faceSketch(plane, focus) {
      const t = three.current;
      if (!t) return;
      t.controls.autoRotate = false;
      t.flight = null;
      if (!plane) {
        // Back to the usual Z-up orbit.
        t.camera.up.set(0, 0, 1);
        t.controls.update();
        t.invalidate();
        return;
      }
      const n = new THREE.Vector3(...normalOf(plane));
      const target = new THREE.Vector3(...(focus ? toWorld(plane, ...focus.center) : plane.origin));
      // Frame what is drawn (its size fits the view's height), or keep the zoom for an empty sketch.
      const fitDistance = focus ? focus.size / 2 / Math.tan(THREE.MathUtils.degToRad(t.camera.fov / 2)) * 1.5 : 0;
      const distance = Math.max(300, fitDistance || t.camera.position.distanceTo(t.controls.target));
      t.camera.up.set(...plane.y);
      t.controls.target.copy(target);
      t.camera.position.copy(target).addScaledVector(n, distance);
      t.controls.update();
      t.invalidate();
    },
    framePin(id) {
      const t = three.current;
      const pin = props.comments?.pins.find((p) => p.id === id) ?? callbacks.current.comments?.pins.find((p) => p.id === id);
      if (!t || !pin) return;
      const part = pin.target ? t.parts.get(pin.target) : undefined;
      if (part) part.group.updateMatrixWorld();
      const holder = pin.target ? callbacks.current.entities.get(pin.target) : undefined;
      const inst = instance(holder) ? placedOf(holder) : undefined;
      const local = new THREE.Vector3(...pin.point);
      if (inst) local.set(...applyT(inst, pin.point as V3));
      const at = part ? part.group.localToWorld(local) : local;
      // Close enough to read the spot, far enough to see the part around it.
      const radius = part ? new THREE.Box3().setFromObject(part.group).getBoundingSphere(new THREE.Sphere()).radius : 200;
      const now = t.camera.position.distanceTo(t.controls.target);
      const distance = Math.max(Math.min(now, radius * 2.8), radius * 0.9, 40);
      const dir = t.camera.position.clone().sub(t.controls.target).normalize();
      if (pin.normal) {
        // Swing round to the side the comment is on, keeping some of the current angle.
        const n = new THREE.Vector3(...pin.normal);
        if (inst) n.applyQuaternion(new THREE.Quaternion(...inst.q));
        if (dir.dot(n) < 0.35) dir.addScaledVector(n, 1.4).normalize();
      }
      t.controls.autoRotate = false;
      glideTo(t, { position: at.clone().addScaledVector(dir, distance), target: at, up: new THREE.Vector3(0, 0, 1) });
      t.invalidate();
    },
    frame(id, glide) {
      if (!three.current) return;
      three.current.controls.autoRotate = false;
      fitView(three.current, id, glide);
      three.current.invalidate();
    },
    grabMotion(spec) {
      motionGrab.current = spec;
    },
    stress(id, view) {
      const t = three.current;
      if (!t) return;
      const old = stressShown.current;
      if (old) {
        old.group.parent?.remove(old.group);
        old.group.traverse((o) => {
          if (o instanceof THREE.Mesh || o instanceof THREE.Line) {
            o.geometry.dispose();
            (o.material as THREE.Material).dispose();
          }
        });
        const body = t.parts.get(old.id)?.body;
        if (body) body.visible = true;
        stressShown.current = null;
      }
      const part = t.parts.get(id);
      if (!view || !part) return t.invalidate();
      const group = new THREE.Group();
      // Placed like the part's body (its program coordinates).
      group.position.copy(part.body.position);
      group.quaternion.copy(part.body.quaternion);
      const size = new THREE.Box3().setFromObject(part.body).getSize(new THREE.Vector3()).length();
      for (const f of view.faces) {
        const g = new THREE.BufferGeometry();
        g.setAttribute("position", new THREE.Float32BufferAttribute(f.triangles, 3));
        const tint = new THREE.Mesh(
          g,
          new THREE.MeshBasicMaterial({ color: { support: "#2f8a5b", load: "#e0761f", hold: "#2b6fd6", heat: "#d6372b" }[f.kind], transparent: true, opacity: view.result ? 0.22 : 0.55, side: THREE.DoubleSide, depthWrite: false, polygonOffset: true, polygonOffsetFactor: -4, polygonOffsetUnits: -4 }),
        );
        tint.renderOrder = 960;
        group.add(tint);
        // Each arrow ends on the face, pointing the way the load pushes there.
        const arrowAt = (tip: THREE.Vector3, dir: THREE.Vector3, len: number) => {
          const arrow = new THREE.ArrowHelper(dir, tip.clone().addScaledVector(dir, -len), len, 0xe0761f, len * 0.28, len * 0.14);
          arrow.traverse((o) => {
            o.renderOrder = 961;
            const m = (o as THREE.Mesh).material as THREE.Material | undefined;
            if (m) m.depthTest = false;
          });
          group.add(arrow);
        };
        if (f.kind === "load" && f.pressure) {
          // A pressure: short arrows square to the face, spread over it (into a bore they fan out to its wall).
          for (const { at, dir } of pressureArrows(f.triangles, 12)) arrowAt(at, dir, size * 0.08);
        } else if (f.kind === "load" && f.force) {
          const dir = new THREE.Vector3(...f.force);
          if (dir.lengthSq() > 0) arrowAt(new THREE.Vector3(...f.point), dir.normalize(), size * 0.18);
        }
      }
      let swing: { mesh: THREE.Mesh; base: Float32Array; delta: Float32Array } | undefined;
      if (view.result) {
        const r = cutAway(view.result, view.section);
        const amount = (size * 0.06) / Math.max(r.max_displacement, 1e-9);
        if (view.oscillate) {
          // The shape at rest and fully swung; the frame loop moves between them.
          const mesh = stressMesh(r, 0);
          const full = stressMesh(r, amount);
          const base = (mesh.geometry.getAttribute("position").array as Float32Array).slice();
          const end = full.geometry.getAttribute("position").array as Float32Array;
          const delta = end.map((v, i) => v - base[i]) as Float32Array;
          full.geometry.dispose();
          (full.material as THREE.Material).dispose();
          group.add(mesh);
          swing = { mesh, base, delta };
        } else if (view.layer === "contact" && view.contact) {
          // The parts see-through, the faces where they meet coloured by pressure.
          group.add(stressMesh(r, 0, true));
          for (const o of contactMeshes(r, view.contact)) group.add(o);
        } else {
          group.add(stressMesh(r, view.deform ? (view.deform * size * 0.04) / Math.max(r.max_displacement, 1e-9) : 0));
        }
        part.body.visible = false;
      }
      for (const mk of view.result && !view.oscillate ? view.markers ?? [] : []) {
        // A ring and a dot, always on top.
        const g = new THREE.BufferGeometry();
        g.setAttribute("position", new THREE.Float32BufferAttribute(mk.at, 3));
        const ring = new THREE.Points(g, new THREE.PointsMaterial({ color: "#ffffff", size: 12, sizeAttenuation: false, depthTest: false, transparent: true, opacity: 0.95 }));
        ring.renderOrder = 963;
        group.add(ring);
        const dot = new THREE.Points(g, new THREE.PointsMaterial({ color: mk.color, size: 7, sizeAttenuation: false, depthTest: false }));
        dot.renderOrder = 964;
        group.add(dot);
      }
      if (view.peak && view.result && !view.oscillate && view.layer !== "contact") {
        // A ring of points round the peak, always on top, so it is found at a glance.
        const g = new THREE.BufferGeometry();
        g.setAttribute("position", new THREE.Float32BufferAttribute(view.peak.at, 3));
        const dot = new THREE.Points(g, new THREE.PointsMaterial({ color: view.peak.singular ? "#b04fd6" : "#ffffff", size: 11, sizeAttenuation: false, depthTest: false, transparent: true, opacity: 0.95 }));
        dot.renderOrder = 963;
        group.add(dot);
        const core = new THREE.Points(g, new THREE.PointsMaterial({ color: "#e0452f", size: 6, sizeAttenuation: false, depthTest: false }));
        core.renderOrder = 964;
        group.add(core);
      }
      part.group.add(group);
      stressShown.current = { id, group, swing };
      t.invalidate();
    },
    leaveSketch() {
      const t = three.current;
      if (!t) return;
      t.controls.autoRotate = false;
      // Tilt from the sketch's face towards the usual three-quarter view, so
      // the depth just made shows and the turn stays easy to follow.
      const facing = t.camera.position.clone().sub(t.controls.target).normalize();
      const usual = new THREE.Vector3(0.55, -0.7, 0.45).normalize();
      const dir = facing.clone().add(usual);
      if (dir.lengthSq() < 0.05) dir.copy(usual);
      dir.normalize();
      // Never straight down or up the Z axis, where a Z-up orbit has no "up".
      if (Math.abs(dir.z) > 0.95) dir.copy(usual);
      if (t.parts.size) fitView(t, undefined, true, dir, new THREE.Vector3(0, 0, 1));
      else glideTo(t, { position: t.controls.target.clone().addScaledVector(dir, t.camera.position.distanceTo(t.controls.target)), target: t.controls.target.clone(), up: new THREE.Vector3(0, 0, 1) });
    },
    turn(id, bodies, options) {
      const t = three.current;
      if (!t) return;
      // A face picked for push and pull rides with its body as it moves.
      const reface = () => {
        const pf = callbacks.current.programFace;
        const at = pf ? t.parts.get(pf.id)?.group.position : undefined;
        if (pf && at && faceLayer.active) faceLayer.setPose(at, facePose(pf));
      };
      const part = t.parts.get(id);
      let m = turning.current;
      // Put back what is turning when asked, or when it is another part or was rebuilt.
      if (m && (!bodies || m.id !== id || part?.body !== m.body)) {
        // Not disposed: the split geometries share the part's buffers, which
        // disposing would free under the part itself.
        m.body.geometry = m.original;
        unghost(m);
        clearTrails(m);
        for (const p of m.pivots) m.body.remove(p.pivot);
        turning.current = m = null;
        reface();
        t.invalidate();
      }
      if (!bodies || !part || part.shared || !Array.isArray(part.body.material)) return;
      const key = (b: TurnBody) => `${"at" in b ? "hinge" : "pose"}:${b.groups.join(",")}`;
      if (!m || m.pivots.map((p) => p.key).join("|") !== bodies.map(key).join("|")) {
        if (m) return; // a different split while turning: put back first (next call rebuilds)
        const original = part.body.geometry;
        const moving = new Set(bodies.flatMap((b) => b.groups));
        const rest = groupsOf(original, (i) => !moving.has(i));
        const pivots = bodies.map((b) => {
          const mesh = new THREE.Mesh(groupsOf(original, (i) => b.groups.includes(i)), part.body.material);
          mesh.castShadow = mesh.receiveShadow = true;
          // Picked as the part itself: the split shares its triangle numbering.
          mesh.userData = part.body.userData;
          const pivot = new THREE.Group();
          if ("at" in b) {
            pivot.position.set(...b.at);
            mesh.position.set(-b.at[0], -b.at[1], -b.at[2]);
          }
          pivot.add(mesh);
          part.body.add(pivot);
          return { pivot, mesh, key: key(b), groups: b.groups };
        });
        part.body.geometry = rest;
        turning.current = m = { id, body: part.body, original, pivots, ghosted: [], trails: null };
      }
      bodies.forEach((b, i) => {
        const pivot = m!.pivots[i].pivot;
        if ("at" in b) pivot.quaternion.setFromAxisAngle(new THREE.Vector3(...b.axis).normalize(), THREE.MathUtils.degToRad(b.degrees));
        else {
          pivot.position.set(...b.position);
          pivot.quaternion.set(...b.quaternion);
        }
      });
      // Still parts see-through while it moves, so what moves inside shows.
      if (options?.ghost && !m.ghosted.length) {
        const moving = new Set(bodies.flatMap((b) => b.groups));
        const mats = part.body.material as THREE.Material[];
        m.original.groups.forEach((g, i) => {
          const mat = mats[g.materialIndex ?? i] as THREE.MeshStandardMaterial;
          if (moving.has(i) || !mat || m!.ghosted.some((x) => x.m === mat)) return;
          m!.ghosted.push({ m: mat, opacity: mat.opacity, transparent: mat.transparent, depthWrite: mat.depthWrite });
          mat.transparent = true;
          mat.opacity = Math.min(mat.opacity, 0.16);
          mat.depthWrite = false;
          mat.needsUpdate = true;
        });
      } else if (!options?.ghost && m.ghosted.length) unghost(m);
      // Trails: thin dashed lines from where each moved body sits at rest.
      const trails = (options?.trails ?? []).filter(([a, b]) => Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2]) > 1e-3);
      if (!trails.length) clearTrails(m);
      else {
        const flat = trails.flatMap(([a, b]) => [...a, ...b]);
        if (!m.trails || m.trails.geometry.getAttribute("position").count !== trails.length * 2) {
          clearTrails(m);
          const longest = Math.max(...trails.map(([a, b]) => Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2])));
          const dash = Math.max(longest / 28, 0.2);
          const material = new THREE.LineDashedMaterial({ color: "#7352d6", dashSize: dash, gapSize: dash * 0.7, transparent: true, opacity: 0.7, depthWrite: false });
          m.trails = new THREE.LineSegments(new THREE.BufferGeometry(), material);
          m.trails.renderOrder = 2;
          m.trails.raycast = () => {}; // never picked: clicks reach the parts
          m.body.add(m.trails);
        }
        m.trails.geometry.setAttribute("position", new THREE.Float32BufferAttribute(flat, 3));
        m.trails.geometry.computeBoundingSphere();
        m.trails.computeLineDistances();
      }
      // The selection outline is of the parts at rest: hidden while they turn.
      part.outline.visible = false;
      reface();
      t.invalidate();
    },
    view(name) {
      const t = three.current;
      if (!t) return;
      t.controls.autoRotate = false;
      // Glides round to the view, everything framed (a standard view in an orthographic camera is a true elevation).
      toolsRef.current?.view(name);
    },
    tools() {
      return toolsRef.current;
    },
    zoom(factor) {
      const t = three.current;
      if (!t) return;
      t.flight = null;
      const offset = t.camera.position.clone().sub(t.controls.target).multiplyScalar(factor);
      t.camera.position.copy(t.controls.target).add(offset);
      t.controls.update();
    },
    setWireframe(on) {
      const t = three.current;
      if (!t) return;
      t.wireframe = on;
      for (const part of t.parts.values()) for (const mesh of part.meshes) for (const m of bodyMaterials(mesh)) m.wireframe = on;
      t.invalidate();
    },
    setTurntable(on) {
      const t = three.current;
      if (!t) return;
      t.controls.autoRotate = on && !reducedMotion();
      t.invalidate();
    },
    snapshot() {
      const t = three.current;
      if (!t) return null;
      const shown = [t.gizmo.root.visible, t.push.root.visible];
      t.gizmo.root.visible = false;
      t.push.root.visible = false;
      t.scene.updateMatrixWorld();
      t.renderer.render(t.scene, t.camera);
      const url = t.renderer.domElement.toDataURL("image/png");
      [t.gizmo.root.visible, t.push.root.visible] = shown;
      t.invalidate();
      return url;
    },
    renderSource() {
      const t = three.current;
      if (!t) return null;
      return { objects: [...t.parts.values()].map((p) => p.group), camera: t.camera };
    },
    resize() {
      three.current?.invalidate();
    },
  }));

  return (
    <div ref={host} className="viewport" aria-label="3D model. Drag to orbit, scroll to zoom. Click a part to move it; click one of its faces to push or pull it.">
      <div ref={labelLayer} className="intent-layer" aria-hidden />
      {viewTools && <ViewToolbar tools={viewTools} selected={props.selected} onFitAll={() => three.current && fitView(three.current, undefined, true)} />}
      <div ref={sketchLabels} className="sketch-layer" />
      <div ref={draftLabels} className="draft-layer" />
      <div ref={label} className="model-dimension" aria-hidden />
      <div ref={faceBadge} className="face-badge" aria-hidden />
      <div ref={pivotTip} className="face-badge pivot-tip" aria-hidden />
      <div ref={snapTag} className="snap-tag" aria-hidden />
      <div ref={readout} className="drag-readout" aria-hidden />
      <div ref={connectorTags} className="connector-tags" aria-hidden />
      <svg ref={markupLayer} className="comment-markup" aria-hidden />
      <div ref={pinLayer} className="comment-pins">
        {(props.comments?.pins ?? []).map((p) => {
          const active = props.comments!.active === p.id;
          return (
            <button
              key={p.id}
              type="button"
              data-pin={p.id}
              className={`comment-pin ${active ? "is-active" : ""} ${p.resolved ? "is-resolved" : ""} ${p.draft ? "is-draft" : ""}`}
              style={{ "--who": p.color } as React.CSSProperties}
              aria-label={p.draft ? "New comment" : `Comment ${p.number}${p.replies ? `, ${p.replies} ${p.replies === 1 ? "reply" : "replies"}` : ""}${p.resolved ? ", resolved" : ""}`}
              onClick={() => !p.draft && props.comments!.onOpen(active ? null : p.id)}
            >
              <span className="pin-bubble">{p.initials}</span>
              {!p.draft && <span className="pin-num">{p.resolved ? "✓" : p.number}</span>}
            </button>
          );
        })}
      </div>
      {props.comments?.placing && !props.comments.tool && (
        <div ref={pinGhost} className="comment-pin is-ghost" style={{ "--who": props.comments.me.color } as React.CSSProperties} aria-hidden>
          <span className="pin-bubble">+</span>
        </div>
      )}
      <div ref={cardAnchor} className="comment-card-anchor">
        {props.comments?.card}
      </div>
      <div ref={cursorLayer} className="team-cursors" aria-hidden>
        {(props.teammates ?? []).map((m) => (
          <div key={m.engine} data-engine={m.engine} className="team-cursor" style={{ "--mate": m.color } as React.CSSProperties}>
            <svg viewBox="0 0 16 16" width="16" height="16">
              <path d="M1 1l5.5 13 1.8-5.2L13.5 7z" />
            </svg>
            <span>{m.name}</span>
          </div>
        ))}
      </div>
    </div>
  );
});

