// The PCB as it comes from the fab and the assembler, drawn in the 3D view in
// place of the engine's plain mesh of the board's 3D part: FR4 edges, solder
// mask in its colour with a slight gloss, copper under the mask showing
// through as raised, lighter traces, exposed pads in the finish (HASL silver,
// ENIG gold), silkscreen, plated hole barrels, and every component body from
// the engine's `bodies` (one instanced mesh per shape and look). The engine's
// mesh stays for picking and clash checks; only its drawing is swapped.
//
// Built from GET /pcb/<id>/3d, refetched (debounced) when the project
// changes; nothing is rebuilt when the scene is unchanged.

import * as THREE from "three";
import { mergeGeometries } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import { projectUrl } from "./engine";

type P = [number, number];
type Side = { pads: P[][]; openings: P[][]; tracks: { w: number; p: P[] }[]; zones: P[][][]; silk: P[][] };
type Solid = { shape: "box" | "cyl" | "hcyl" | "sphere"; at: [number, number, number]; size: [number, number, number]; rot: number; mat: string };
export type Scene3D = {
  id: string;
  thickness: number;
  mask: string;
  silk: string;
  finish: string;
  outline: P[];
  cutouts: P[][];
  bounds: [number, number, number, number];
  silk_width: number;
  vias: [number, number, number, number][];
  holes: { at: P; size: P; rot: number; plated: boolean }[];
  sides: { top: Side; bottom: Side };
  bodies: { reference: string; solids: Solid[] }[];
  /** Bodies of parts not placed (DNP), drawn on request. */
  dnp_bodies?: { reference: string; solids: Solid[] }[];
  /** What each part is, for showing and hiding by type. */
  parts?: { reference: string; kind: "smd" | "tht" | "other"; dnp: boolean }[];
};

/** What the person shows of the board in the 3D view (view only, not saved). */
export type BoardShow = { mask: boolean; silk: boolean; copper: boolean; bodies: boolean; smd: boolean; tht: boolean; other: boolean; dnp: boolean; boxes: boolean };
export const SHOW_ALL: BoardShow = { mask: true, silk: true, copper: true, bodies: true, smd: true, tht: true, other: true, dnp: false, boxes: false };

/** The scene with what is hidden taken out. */
function shown(scene: Scene3D, o: BoardShow): Scene3D {
  const kind = new Map((scene.parts ?? []).map((p) => [p.reference, p.kind]));
  const keep = (r: string) => o.bodies && o[kind.get(r) ?? "other"];
  const side = (x: Side): Side => ({ pads: o.copper ? x.pads : [], openings: x.openings, tracks: o.copper ? x.tracks : [], zones: o.copper ? x.zones : [], silk: o.silk ? x.silk : [] });
  return {
    ...scene,
    mask: o.mask ? scene.mask : "none",
    sides: { top: side(scene.sides.top), bottom: side(scene.sides.bottom) },
    bodies: [...scene.bodies.filter((b) => keep(b.reference)), ...(o.dnp && o.bodies ? (scene.dnp_bodies ?? []).filter((b) => o[kind.get(b.reference) ?? "other"]) : [])],
  };
}

/** Each part's bounding box as lines. */
function boxes(scene: Scene3D) {
  const pos: number[] = [];
  for (const b of scene.bodies) {
    const lo = [Infinity, Infinity, Infinity];
    const hi = [-Infinity, -Infinity, -Infinity];
    for (const s of b.solids) {
      const a = (s.rot * Math.PI) / 180;
      const [c, n] = [Math.abs(Math.cos(a)), Math.abs(Math.sin(a))];
      const hx = (s.size[0] * c + s.size[1] * n) / 2;
      const hy = (s.size[0] * n + s.size[1] * c) / 2;
      lo[0] = Math.min(lo[0], s.at[0] - hx);
      hi[0] = Math.max(hi[0], s.at[0] + hx);
      lo[1] = Math.min(lo[1], s.at[1] - hy);
      hi[1] = Math.max(hi[1], s.at[1] + hy);
      lo[2] = Math.min(lo[2], s.at[2]);
      hi[2] = Math.max(hi[2], s.at[2] + s.size[2]);
    }
    if (!Number.isFinite(lo[0])) continue;
    const e = new THREE.EdgesGeometry(new THREE.BoxGeometry(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]).translate((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2));
    pos.push(...(e.getAttribute("position").array as Float32Array));
    e.dispose();
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
  const lines = new THREE.LineSegments(g, new THREE.LineBasicMaterial({ color: "#ffb020" }));
  lines.name = "pcb-boxes";
  return lines;
}

const MASK: Record<string, string> = { green: "#14532f", black: "#0a0b0d", blue: "#123c6e", red: "#7a1418", white: "#e9e9e4", purple: "#3b1d63", yellow: "#c9a51c", matte_black: "#17181a" };

function finishColor(finish: string) {
  const f = finish.toLowerCase();
  if (f.includes("enig") || f.includes("gold")) return { color: "#e0b45c", rough: 0.22 };
  if (f.includes("osp")) return { color: "#c97e4b", rough: 0.3 };
  return { color: "#d3d6db", rough: 0.3 };
}

function mix(a: string, b: string, t: number) {
  return "#" + new THREE.Color(a).lerp(new THREE.Color(b), t).getHexString();
}

// ---- A studio environment for reflections (no network, no renderer needed) ----

let env: THREE.CubeTexture | null = null;
function environment() {
  if (env) return env;
  const face = (top: string, bottom: string, light?: boolean) => {
    const c = document.createElement("canvas");
    c.width = c.height = 128;
    const g = c.getContext("2d")!;
    const grad = g.createLinearGradient(0, 0, 0, 128);
    grad.addColorStop(0, top);
    grad.addColorStop(1, bottom);
    g.fillStyle = grad;
    g.fillRect(0, 0, 128, 128);
    if (light) {
      // Soft boxes.
      g.fillStyle = "rgba(255,255,255,0.95)";
      g.fillRect(18, 30, 36, 50);
      g.fillRect(76, 40, 34, 40);
    }
    return c;
  };
  // +x, −x, +y, −y, +z, −z (three's cube order); z is up in this scene, so +y/−y read as sky/floor.
  const side = () => face("#f4f6fa", "#7d838c", true);
  env = new THREE.CubeTexture([side(), side(), face("#ffffff", "#e8ecf2", true), face("#5f646b", "#3a3d42"), side(), side()]);
  env.colorSpace = THREE.SRGBColorSpace;
  env.needsUpdate = true;
  return env;
}

// ---- Materials for bodies ----

type Look = { color: string; metal?: number; rough?: number; clear?: number; opacity?: number; emissive?: string; transmission?: number };
const LOOKS: Record<string, Look> = {
  epoxy: { color: "#1b1b1d", rough: 0.6 },
  mark: { color: "#3c3c40", rough: 0.4 },
  ceramic: { color: "#a8875c", rough: 0.65 },
  ceramic_white: { color: "#e6e1d6", rough: 0.6 },
  res_top: { color: "#131313", rough: 0.75 },
  tin: { color: "#d4d7dc", metal: 1, rough: 0.26 },
  gold: { color: "#e6b85c", metal: 1, rough: 0.22 },
  steel: { color: "#cdd1d7", metal: 1, rough: 0.28 },
  steel_dark: { color: "#8f959d", metal: 1, rough: 0.4 },
  nylon: { color: "#eee5cc", rough: 0.55 },
  black: { color: "#19191b", rough: 0.5 },
  dark: { color: "#040404", rough: 0.9 },
  module_pcb: { color: "#1b2433", rough: 0.35, clear: 0.6 },
  antenna: { color: "#d6ad5e", metal: 1, rough: 0.3 },
  led_body: { color: "#f3f0e7", rough: 0.45 },
  led_red: { color: "#ff3524", rough: 0.15, opacity: 0.85, emissive: "#ff1a10", clear: 1 },
  led_green: { color: "#3cff6a", rough: 0.15, opacity: 0.85, emissive: "#10ff40", clear: 1 },
  led_blue: { color: "#3b7bff", rough: 0.15, opacity: 0.85, emissive: "#1050ff", clear: 1 },
  led_yellow: { color: "#ffd83a", rough: 0.15, opacity: 0.85, emissive: "#ffc010", clear: 1 },
  led_orange: { color: "#ff9a2a", rough: 0.15, opacity: 0.85, emissive: "#ff7010", clear: 1 },
  led_white: { color: "#f7fbff", rough: 0.1, opacity: 0.8, emissive: "#a0b0c0", clear: 1 },
};

function material(look: Look) {
  const m = new THREE.MeshPhysicalMaterial({
    color: look.color,
    metalness: look.metal ?? 0,
    roughness: look.rough ?? 0.5,
    clearcoat: look.clear ?? 0,
    clearcoatRoughness: 0.2,
    envMap: environment(),
    envMapIntensity: look.metal ? 1.1 : 0.55,
    transparent: (look.opacity ?? 1) < 1,
    opacity: look.opacity ?? 1,
  });
  if (look.emissive) {
    m.emissive.set(look.emissive);
    m.emissiveIntensity = 0.35;
  }
  return m;
}

const UNIT = {
  box: () => new THREE.BoxGeometry(1, 1, 1).translate(0, 0, 0.5),
  cyl: () => new THREE.CylinderGeometry(0.5, 0.5, 1, 40).rotateX(Math.PI / 2).translate(0, 0, 0.5),
  hcyl: () => new THREE.CylinderGeometry(0.5, 0.5, 1, 40).translate(0, 0, 0.5),
  sphere: () => new THREE.SphereGeometry(0.5, 40, 20).translate(0, 0, 0.5),
};

function bodiesGroup(scene: Scene3D) {
  const group = new THREE.Group();
  group.name = "pcb-bodies";
  const buckets = new Map<string, (Solid & { ref: string })[]>();
  for (const b of scene.bodies) for (const s of b.solids) {
    const key = `${s.shape}|${s.mat}`;
    (buckets.get(key) ?? buckets.set(key, []).get(key)!).push({ ...s, ref: b.reference });
  }
  const m4 = new THREE.Matrix4();
  const q = new THREE.Quaternion();
  const z = new THREE.Vector3(0, 0, 1);
  for (const [key, list] of buckets) {
    const [shape, mat] = key.split("|") as [Solid["shape"], string];
    const mesh = new THREE.InstancedMesh(UNIT[shape](), material(LOOKS[mat] ?? LOOKS.epoxy), list.length);
    list.forEach((s, i) => {
      q.setFromAxisAngle(z, (s.rot * Math.PI) / 180);
      m4.compose(new THREE.Vector3(...s.at), q, new THREE.Vector3(...s.size));
      mesh.setMatrixAt(i, m4);
    });
    mesh.instanceMatrix.needsUpdate = true;
    // Which part each instance belongs to, and where it sits, so the components
    // tree can light up, hide or show alone one part of the board.
    mesh.userData.refs = list.map((s) => s.ref);
    mesh.userData.base = Float32Array.from(mesh.instanceMatrix.array as ArrayLike<number>);
    mesh.castShadow = true;
    mesh.receiveShadow = true;
    mesh.computeBoundingSphere();
    group.add(mesh);
  }
  return group;
}

// ---- The board ----

function holePath(h: Scene3D["holes"][number]) {
  const path = new THREE.Path();
  const [w, l] = h.size;
  if (Math.abs(w - l) < 1e-6) {
    path.absarc(h.at[0], h.at[1], w / 2, 0, Math.PI * 2, true);
    return path;
  }
  const r = Math.min(w, l) / 2;
  const half = (Math.max(w, l) - Math.min(w, l)) / 2;
  const a = ((h.rot + (w > l ? 0 : 90)) * Math.PI) / 180;
  const d: P = [Math.cos(a) * half, Math.sin(a) * half];
  // A stadium, clockwise.
  path.absarc(h.at[0] + d[0], h.at[1] + d[1], r, a + Math.PI / 2, a - Math.PI / 2, true);
  path.absarc(h.at[0] - d[0], h.at[1] - d[1], r, a - Math.PI / 2, a - (3 * Math.PI) / 2, true);
  path.closePath();
  return path;
}

function boardShape(scene: Scene3D) {
  const shape = new THREE.Shape(scene.outline.map(([x, y]) => new THREE.Vector2(x, y)));
  for (const c of scene.cutouts) shape.holes.push(new THREE.Path(c.map(([x, y]) => new THREE.Vector2(x, y))));
  for (const h of scene.holes) shape.holes.push(holePath(h));
  for (const v of scene.vias) {
    const p = new THREE.Path();
    p.absarc(v[0], v[1], v[3] / 2, 0, Math.PI * 2, true);
    shape.holes.push(p);
  }
  return shape;
}

/** Paints one side's look into colour, roughness/metalness and bump canvases. */
function paint(scene: Scene3D, side: Side, px: number) {
  const [x0, y0, x1, y1] = scene.bounds;
  const W = Math.max(8, Math.round((x1 - x0) * px));
  const H = Math.max(8, Math.round((y1 - y0) * px));
  const make = () => {
    const c = document.createElement("canvas");
    c.width = W;
    c.height = H;
    const g = c.getContext("2d")!;
    g.setTransform(px, 0, 0, -px, -x0 * px, y1 * px);
    g.lineCap = "round";
    g.lineJoin = "round";
    return { c, g };
  };
  // Mask hidden: the bare laminate with its copper showing.
  const bare = scene.mask === "none";
  const mask = bare ? "#a39468" : (MASK[scene.mask.toLowerCase().replace(/\s+/g, "_")] ?? MASK.green);
  const fin = finishColor(scene.finish);
  const silkColor = scene.silk.toLowerCase() === "black" ? "#151515" : "#f4f3ee";
  const underMask = bare ? "#c98b52" : mix(mix(mask, "#c27a3c", scene.mask.toLowerCase() === "black" ? 0.1 : 0.22), "#ffffff", scene.mask.toLowerCase() === "white" ? 0 : 0.06);
  const polyPath = (g: CanvasRenderingContext2D, pts: P[]) => {
    g.moveTo(pts[0][0], pts[0][1]);
    for (let i = 1; i < pts.length; i++) g.lineTo(pts[i][0], pts[i][1]);
    g.closePath();
  };
  const fillPolys = (g: CanvasRenderingContext2D, polys: P[][], style: string) => {
    g.fillStyle = style;
    g.beginPath();
    for (const p of polys) if (p.length >= 3) polyPath(g, p);
    g.fill("nonzero");
  };
  const copper = (g: CanvasRenderingContext2D, style: string) => {
    g.fillStyle = style;
    for (const island of side.zones) {
      g.beginPath();
      for (const c of island) polyPath(g, c);
      g.fill("nonzero");
    }
    g.strokeStyle = style;
    for (const t of side.tracks) {
      g.lineWidth = t.w;
      g.beginPath();
      g.moveTo(t.p[0][0], t.p[0][1]);
      for (const q of t.p.slice(1)) g.lineTo(q[0], q[1]);
      g.stroke();
    }
    g.beginPath();
    for (const v of scene.vias) {
      g.moveTo(v[0] + v[2] / 2, v[1]);
      g.arc(v[0], v[1], v[2] / 2, 0, Math.PI * 2);
    }
    g.fill();
  };
  const silk = (g: CanvasRenderingContext2D, style: string) => {
    g.strokeStyle = style;
    g.lineWidth = Math.max(scene.silk_width, 0.12);
    g.beginPath();
    for (const l of side.silk) {
      g.moveTo(l[0][0], l[0][1]);
      for (const q of l.slice(1)) g.lineTo(q[0], q[1]);
    }
    g.stroke();
  };
  // Colour.
  const col = make();
  col.g.fillStyle = mask;
  col.g.fillRect(x0 - 1, y0 - 1, x1 - x0 + 2, y1 - y0 + 2);
  copper(col.g, underMask);
  fillPolys(col.g, side.openings, "#b9a77a");
  fillPolys(col.g, side.pads, fin.color);
  silk(col.g, silkColor);
  // Roughness (green) and metalness (blue).
  const rm = make();
  const rgb = (r: number, m: number) => `rgb(0,${Math.round(r * 255)},${Math.round(m * 255)})`;
  rm.g.fillStyle = rgb(0.44, 0);
  rm.g.fillRect(x0 - 1, y0 - 1, x1 - x0 + 2, y1 - y0 + 2);
  copper(rm.g, bare ? rgb(0.3, 1) : rgb(0.34, 0));
  fillPolys(rm.g, side.openings, rgb(0.85, 0));
  fillPolys(rm.g, side.pads, rgb(fin.rough, 1));
  silk(rm.g, rgb(0.85, 0));
  // Height: copper stands proud under the mask, silk above it.
  const bump = make();
  bump.g.fillStyle = "#000";
  bump.g.fillRect(x0 - 1, y0 - 1, x1 - x0 + 2, y1 - y0 + 2);
  copper(bump.g, "#7a7a7a");
  fillPolys(bump.g, side.openings, "#202020");
  fillPolys(bump.g, side.pads, "#8a8a8a");
  silk(bump.g, "#b0b0b0");
  return { color: col.c, rm: rm.c, bump: bump.c };
}

function texture(c: HTMLCanvasElement, scene: Scene3D, srgb: boolean) {
  const [x0, y0, x1, y1] = scene.bounds;
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = srgb ? THREE.SRGBColorSpace : THREE.NoColorSpace;
  t.repeat.set(1 / (x1 - x0), 1 / (y1 - y0));
  t.offset.set(-x0 / (x1 - x0), -y0 / (y1 - y0));
  t.anisotropy = 8;
  t.generateMipmaps = true;
  t.minFilter = THREE.LinearMipmapLinearFilter;
  return t;
}

function faceMaterial(scene: Scene3D, side: Side, px: number) {
  const c = paint(scene, side, px);
  const rm = texture(c.rm, scene, false);
  return new THREE.MeshPhysicalMaterial({
    map: texture(c.color, scene, true),
    roughnessMap: rm,
    metalnessMap: rm,
    bumpMap: texture(c.bump, scene, false),
    bumpScale: 1.4,
    roughness: 1,
    metalness: 1,
    clearcoat: scene.mask.toLowerCase().includes("matte") ? 0 : 0.22,
    clearcoatRoughness: 0.32,
    envMap: environment(),
    envMapIntensity: 0.22,
  });
}

/** The whole board as a group in board coordinates (mm, z up from its bottom face). */
export function buildBoard(full: Scene3D, show: BoardShow = SHOW_ALL): THREE.Group {
  const scene = shown(full, show);
  const group = new THREE.Group();
  group.name = "pcb-look";
  const t = scene.thickness;
  const [x0, y0, x1, y1] = scene.bounds;
  // About 32 px/mm (a 0.15 mm track is 5 px), at most 3072 px a side.
  const px = Math.min(32, 3072 / Math.max(x1 - x0, y1 - y0, 1));
  const shape = boardShape(scene);
  const face = new THREE.ShapeGeometry(shape, 24);
  const top = new THREE.Mesh(face, faceMaterial(scene, scene.sides.top, px));
  top.position.z = t;
  top.receiveShadow = true;
  // The bottom face: the same outline turned to face down.
  const under = face.clone();
  const idx = under.getIndex()!;
  const a = idx.array as unknown as number[];
  for (let i = 0; i < a.length; i += 3) [a[i + 1], a[i + 2]] = [a[i + 2], a[i + 1]];
  idx.needsUpdate = true;
  const n = under.getAttribute("normal") as THREE.BufferAttribute;
  for (let i = 0; i < n.count; i++) n.setXYZ(i, 0, 0, -1);
  const bottom = new THREE.Mesh(under, faceMaterial(scene, scene.sides.bottom, px));
  bottom.receiveShadow = true;
  // Edges: FR4 laminate (and the walls of slots and unplated holes).
  const edgeGeo = new THREE.ExtrudeGeometry(shape, { depth: t, bevelEnabled: false, curveSegments: 24 });
  const hidden = new THREE.MeshBasicMaterial({ visible: false });
  const fr4 = new THREE.MeshPhysicalMaterial({ color: "#a39468", roughness: 0.8, metalness: 0, envMap: environment(), envMapIntensity: 0.4 });
  const edges = new THREE.Mesh(edgeGeo, [hidden, fr4]);
  edges.castShadow = true;
  // Plated barrels inside round plated holes and vias.
  const barrels: THREE.BufferGeometry[] = [];
  for (const h of scene.holes) {
    if (!h.plated || Math.abs(h.size[0] - h.size[1]) > 1e-6) continue;
    barrels.push(new THREE.CylinderGeometry(h.size[0] / 2 - 0.01, h.size[0] / 2 - 0.01, t + 0.02, 24, 1, true).rotateX(Math.PI / 2).translate(h.at[0], h.at[1], t / 2));
  }
  for (const v of scene.vias) barrels.push(new THREE.CylinderGeometry(v[3] / 2 - 0.01, v[3] / 2 - 0.01, t + 0.02, 16, 1, true).rotateX(Math.PI / 2).translate(v[0], v[1], t / 2));
  group.add(top, bottom, edges);
  if (barrels.length) {
    const fin = finishColor(scene.finish);
    const m = new THREE.Mesh(mergeGeometries(barrels), new THREE.MeshPhysicalMaterial({ color: fin.color, metalness: 1, roughness: 0.3, side: THREE.DoubleSide, envMap: environment() }));
    group.add(m);
  }
  group.add(bodiesGroup(scene));
  if (show.boxes) group.add(boxes(scene));
  return group;
}

function dispose(o: THREE.Object3D) {
  o.traverse((x) => {
    const m = x as THREE.Mesh;
    m.geometry?.dispose();
    const mats = Array.isArray(m.material) ? m.material : m.material ? [m.material] : [];
    for (const mat of mats) {
      for (const v of Object.values(mat)) if (v instanceof THREE.Texture && v !== env) v.dispose();
      mat.dispose();
    }
  });
}

// ---- Keeping the view's board parts drawn this way ----

type Entry = { board: string; text: string; look: THREE.Group | null; group: THREE.Group | null; meshes: THREE.Mesh[] };

/** How the components tree shows a board's pieces: hidden, shown alone, lit up ("*" for all). */
export type BoardPaint = { pieces: string[]; hidden: (piece: string) => boolean; isolated: string | null; hot: "*" | string[] };

const HOT_TINT = new THREE.Color("#c4b2ff");
const PLAIN = new THREE.Color("#ffffff");
const ZERO = new THREE.Matrix4().makeScale(0, 0, 0);

class PcbLooks {
  private paints = new Map<string, BoardPaint>(); // model id → how the tree shows it

  /** Applies the tree's hide / show-only / hover to a board's own drawing. */
  paint(model: string, how: BoardPaint) {
    this.paints.set(model, how);
    const e = this.entries.get(model);
    if (e?.look) this.apply(e.look, how);
  }

  private apply(look: THREE.Group, how: BoardPaint) {
    // A piece is named by its reference first ("C1 10uF"); the rest of the board is the "PCB" piece.
    const refs = new Set<string>();
    look.traverse((o) => (o.userData.refs as string[] | undefined)?.forEach((r) => refs.add(r)));
    const pieceOf = new Map<string, string>();
    let boardPiece: string | null = null;
    for (const name of how.pieces) {
      const ref = name.split(/\s+/)[0];
      if (refs.has(ref)) pieceOf.set(ref, name);
      else boardPiece ??= name;
    }
    const shown = (piece: string | null) => !piece || (!how.hidden(piece) && (!how.isolated || how.isolated === piece));
    const hot = (piece: string | null) => how.hot === "*" || (!!piece && how.hot.includes(piece));
    look.traverse((o) => {
      const refsOf = o.userData.refs as string[] | undefined;
      if (o instanceof THREE.InstancedMesh && refsOf) {
        const base = o.userData.base as Float32Array;
        const m = new THREE.Matrix4();
        const coloured = !!o.instanceColor;
        refsOf.forEach((ref, i) => {
          const piece = pieceOf.get(ref) ?? null;
          if (shown(piece)) o.setMatrixAt(i, m.fromArray(base, i * 16));
          else o.setMatrixAt(i, ZERO);
          o.setColorAt(i, hot(piece) ? HOT_TINT : PLAIN);
        });
        o.instanceMatrix.needsUpdate = true;
        if (o.instanceColor) o.instanceColor.needsUpdate = true;
        // The first tint switches the material to per-instance colour.
        if (!coloured) for (const mat of Array.isArray(o.material) ? o.material : [o.material]) mat.needsUpdate = true;
        return;
      }
      if (o instanceof THREE.Mesh && !o.parent?.userData.refs && !insideBodies(o)) {
        o.visible = shown(boardPiece);
        for (const mat of Array.isArray(o.material) ? o.material : [o.material]) {
          if (mat instanceof THREE.MeshStandardMaterial) {
            mat.emissive.set(hot(boardPiece) ? "#9a7cf0" : "#000000");
            mat.emissiveIntensity = hot(boardPiece) ? 0.35 : 0;
          }
        }
      }
    });
    this.invalidate();
  }

  private models = new Map<string, string>(); // model id → board id
  private entries = new Map<string, Entry>(); // model id → look
  private seen = new Map<string, { group: THREE.Group; meshes: THREE.Mesh[] }>(); // every drawn part
  private invalidate: () => void = () => {};
  private timer: number | null = null;
  private busy = false;
  private again = false;
  private show: BoardShow = SHOW_ALL;
  private listeners = new Set<() => void>();

  /** What is shown of every board (view only). */
  setShow(show: BoardShow) {
    this.show = show;
    for (const e of this.entries.values()) e.text = "";
    void this.refresh();
  }

  getShow() {
    return this.show;
  }

  /** Called when the boards drawn change. */
  subscribe(fn: () => void) {
    this.listeners.add(fn);
    return () => void this.listeners.delete(fn);
  }

  /** The boards drawn now: model id, board id and its look. */
  drawn() {
    return [...this.entries.entries()].filter(([, e]) => e.look).map(([model, e]) => ({ model, board: e.board, look: e.look! }));
  }

  setInvalidate(fn: () => void) {
    this.invalidate = fn;
  }

  /** The project changed: look again (debounced). */
  touch() {
    if (this.timer !== null) window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      this.timer = null;
      void this.refresh();
    }, 120);
  }

  /** A part drawn by the view: when it is a board's 3D part, draw the board instead. */
  attach(id: string, group: THREE.Group, meshes: THREE.Mesh[]) {
    this.seen.set(id, { group, meshes });
    const e = this.entries.get(id);
    if (!e) {
      if (this.models.has(id)) {
        this.entries.set(id, { board: this.models.get(id)!, text: "", look: null, group, meshes });
        this.touch();
      }
      return;
    }
    if (e.group !== group) {
      e.group = group;
      e.meshes = meshes;
      if (e.look) group.add(e.look);
    }
    this.hideEngine(e);
  }

  detach(id: string) {
    this.seen.delete(id);
    this.drop(id);
  }

  private drop(id: string) {
    const e = this.entries.get(id);
    if (!e) return;
    if (e.look) {
      e.look.removeFromParent();
      dispose(e.look);
    }
    this.entries.delete(id);
  }

  private hideEngine(e: Entry) {
    if (!e.look) return;
    for (const m of e.meshes) for (const mat of Array.isArray(m.material) ? m.material : [m.material]) mat.visible = false;
  }

  private async refresh() {
    if (this.busy) {
      this.again = true;
      return;
    }
    this.busy = true;
    try {
      const list = (await (await fetch(projectUrl("/pcb/boards"))).json()) as { id: string; model: string | null }[];
      this.models = new Map(list.filter((b) => b.model).map((b) => [b.model!, b.id]));
      for (const model of [...this.entries.keys()]) if (!this.models.has(model)) this.drop(model);
      for (const [id, s] of this.seen) if (this.models.has(id) && !this.entries.has(id)) this.entries.set(id, { board: this.models.get(id)!, text: "", look: null, group: s.group, meshes: s.meshes });
      for (const [model, entry] of this.entries) {
        const res = await fetch(projectUrl(`/pcb/${entry.board}/3d`));
        if (!res.ok) continue;
        const text = await res.text();
        if (text === entry.text && entry.look) continue;
        entry.text = text;
        const scene = JSON.parse(text) as Scene3D;
        const look = buildBoard(scene, this.show);
        if (entry.look) {
          entry.look.removeFromParent();
          dispose(entry.look);
        }
        entry.look = look;
        entry.group?.add(look);
        const how = this.paints.get(model);
        if (how) this.apply(look, how);
        this.hideEngine(entry);
        void model;
        this.invalidate();
        for (const fn of this.listeners) fn();
      }
    } catch {
      // The engine is away: the plain mesh stays.
    } finally {
      this.busy = false;
      if (this.again) {
        this.again = false;
        void this.refresh();
      }
    }
  }

  /** Board models known so far (a part not yet known is attached on the next refresh). */
  isBoard(id: string) {
    return this.models.has(id);
  }
}

/** Whether a mesh is one of the component bodies (they are painted per instance). */
function insideBodies(o: THREE.Object3D) {
  for (let p: THREE.Object3D | null = o.parent; p; p = p.parent) if (p.name === "pcb-bodies") return true;
  return false;
}

export const pcbLooks = new PcbLooks();
