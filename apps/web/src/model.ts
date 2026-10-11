// Typed readers over engine entity views.

import type { EntityView } from "./engine";

export type Q = { value: number; unit: string };
export type BoxFeature = { width: Q; depth: Q; height: Q };
export type Placement = { x: Q; y: Q; z: Q };
export type Material = { name: string; density: Q };

/** Engine stores meters; the UI works in millimeters. */
export const toMm = (q: Q | undefined) => (q ? q.value * 1000 : 0);
export const box = (e: EntityView) => e.components["solid.box"] as BoxFeature | undefined;
export const placement = (e: EntityView) => e.components["solid.placement"] as Placement | undefined;
export const material = (e: EntityView) => e.components["solid.material"] as Material | undefined;
export const materialRef = (e: EntityView) => e.components["solid.material_ref"] as string | undefined;

export type ProgramParam = { key: string; label: string; value: number; unit: string; min?: number | null; max?: number | null; step?: number | null };
export type Program = {
  name: string;
  description: string;
  source: string;
  params: ProgramParam[];
  display_tolerance?: number;
  levels?: { name: string; elevation: number }[];
  /** Edges rounded or bevelled after the program runs, picked in the view. */
  finishes?: EdgeFinish[];
};

/** An edge as picked: kind, direction and place relative to the part's bounds (fractions). */
export type EdgePick = { at: [number, number, number]; kind: string; dir?: [number, number, number]; radius?: number };
export type EdgeFinish = { kind: "fillet" | "chamfer" | "push"; key: string; part?: string; edges: EdgePick[] };

/** An AI-made shape's program, if this entity is one. */
export const program = (e: EntityView) => e.components["shape.program"] as Program | undefined;

/** An assembly's material table: names its parts use → material entity ids. */
export const partMaterials = (e: EntityView) => (e.components["shape.materials"] as Record<string, string> | undefined) ?? {};

/** The material entity a part of an assembly is made of (or the shape's own). */
export function partMaterial(e: EntityView, materialName: string | null, entities: ReadonlyMap<string, EntityView>): EntityView | undefined {
  const id = materialName ? partMaterials(e)[materialName] : materialRef(e);
  return id ? entities.get(id) : undefined;
}

/** The parts of an assembly (empty for a single solid). */
export const assemblyParts = (e: EntityView | undefined) => e?.shape?.parts ?? [];

/** How a part is made, in shop words. */
export const PROCESS_NAMES: Record<string, string> = {
  "3d_print": "3D print",
  cnc_mill: "CNC mill",
  lathe: "Lathe",
  cnc_router: "CNC router",
  laser: "Laser / waterjet",
  sheet_metal: "Sheet metal",
  weld: "Welded",
  cast: "Cast",
  extrusion: "Cut extrusion",
  glazing: "Glazing",
  buy: "Bought in",
};

/** Sheet processes get a flat, layered DXF cut file as well as an STL. */
export const isSheetProcess = (p: string | null | undefined) => p === "cnc_router" || p === "laser" || p === "sheet_metal";

/** Length × width × thickness, largest first (mm). */
export function stockSize(p: { bounds_min: number[]; bounds_max: number[] }) {
  return [0, 1, 2].map((k) => p.bounds_max[k] - p.bounds_min[k]).sort((a, b) => b - a);
}

/** Millimeters for small parts, meters once a part is building-sized. */
export function fmtLength(mm: number) {
  return Math.abs(mm) >= 10000 ? `${(mm / 1000).toLocaleString("en-US", { maximumFractionDigits: 2 })} m` : `${fmt(mm)} mm`;
}

// ---- Components and assemblies ----

export type InstanceData = {
  name: string;
  of: string;
  parent: string | null;
  at: [number, number, number];
  rotation: number;
  properties?: Record<string, string>;
};

export const instance = (e: EntityView | undefined) => e?.components["assembly.instance"] as InstanceData | undefined;
export const assemblyDef = (e: EntityView | undefined) => e?.components["assembly.definition"] as { name: string; description: string } | undefined;
/** A shape with no placement of its own: a component, shown only through its instances. */
export const isDefinition = (e: EntityView) => !!assemblyDef(e) || (!!program(e) && !placement(e));

/** Name of a definition (component or assembly). */
export function definitionName(id: string, entities: ReadonlyMap<string, EntityView>) {
  const d = entities.get(id);
  return (d && (program(d)?.name ?? assemblyDef(d)?.name)) ?? "Missing definition";
}

/** Instances placed inside an assembly definition, bottom to top, then by name. */
export function childrenOf(defId: string, entities: ReadonlyMap<string, EntityView>): EntityView[] {
  return [...entities.values()]
    .filter((e) => instance(e)?.parent === defId)
    .sort((a, b) => instance(a)!.at[2] - instance(b)!.at[2] || instance(a)!.name.localeCompare(instance(b)!.name, undefined, { numeric: true }));
}

/** How many instances use this definition. */
export const usesOf = (defId: string, entities: ReadonlyMap<string, EntityView>) => [...entities.values()].filter((e) => instance(e)?.of === defId).length;

export type Transform = { at: [number, number, number]; rotation: number };

/** `inner` placed inside `outer` (rotation about Z, then translation). */
export function compose(inner: Transform, outer: Transform): Transform {
  const r = (outer.rotation * Math.PI) / 180;
  const [x, y, z] = inner.at;
  return {
    at: [Math.cos(r) * x - Math.sin(r) * y + outer.at[0], Math.sin(r) * x + Math.cos(r) * y + outer.at[1], z + outer.at[2]],
    rotation: outer.rotation + inner.rotation,
  };
}

/** One shape somewhere inside an instance: which component, where, and the instance path to it. */
export type Occurrence = { shape: string; path: string[]; transform: Transform };

/** Every shape under an instance (or assembly definition), in the instance's parent coordinates. */
export function occurrences(id: string, entities: ReadonlyMap<string, EntityView>): Occurrence[] {
  const out: Occurrence[] = [];
  const walk = (eid: string, t: Transform, path: string[], depth: number) => {
    if (depth > 32) return;
    const e = entities.get(eid);
    if (!e) return;
    const inst = instance(e);
    if (inst) return walk(inst.of, compose({ at: inst.at, rotation: inst.rotation }, t), [...path, eid], depth + 1);
    if (assemblyDef(e)) {
      for (const c of childrenOf(eid, entities)) walk(c.id, t, path, depth + 1);
      return;
    }
    if (program(e)) out.push({ shape: eid, path, transform: t });
  };
  walk(id, { at: [0, 0, 0], rotation: 0 }, [], 0);
  return out;
}

/** The top-level instance an instance sits in (itself when it is top-level). */
export function topInstanceOf(id: string, entities: ReadonlyMap<string, EntityView>): string {
  let current = id;
  for (let i = 0; i < 32; i++) {
    const inst = instance(entities.get(current));
    if (!inst?.parent) return current;
    const parentUse = [...entities.values()].find((e) => instance(e)?.of === inst.parent);
    if (!parentUse) return current;
    current = parentUse.id;
  }
  return current;
}

/** Anything placed in the scene: boxes, placed shapes and top-level instances. */
export type GroupData = { name: string; members: string[] };
export const groupOf = (e: EntityView | undefined) => e?.components["assembly.group"] as GroupData | undefined;
export const isPart = (e: EntityView) => !!box(e) || (!!program(e) && !!placement(e)) || (!!instance(e) && !instance(e)!.parent);

/** Where the part's own origin sits relative to its bounds' minimum corner (mm). */
export function originOffset(e: EntityView): [number, number, number] {
  // Instances are drawn in place; their group sits at the origin until moved.
  if (instance(e)) return e.assembly ? e.assembly.bounds_min : [0, 0, 0];
  return program(e) && e.shape ? e.shape.bounds_min : [0, 0, 0];
}

export const fmt = (n: number) => n.toLocaleString("en-US", { maximumFractionDigits: 2 });

export function partName(e: EntityView) {
  const inst = instance(e);
  if (inst) return inst.name;
  const prog = program(e);
  if (prog) return prog.name;
  const b = box(e);
  return b ? `Box ${fmt(toMm(b.width))} × ${fmt(toMm(b.depth))} × ${fmt(toMm(b.height))}` : "Object";
}

export function formatMass(kg: number | null | undefined) {
  if (kg == null) return "No material";
  return kg >= 1
    ? `${kg.toLocaleString("en-US", { maximumFractionDigits: 3 })} kg`
    : `${(kg * 1000).toLocaleString("en-US", { maximumFractionDigits: 2 })} g`;
}

export const KNOWN_MATERIALS: Record<string, { name: string; density: number }> = {
  steel: { name: "Steel", density: 7850 },
  aluminium: { name: "Aluminium", density: 2700 },
  aluminum: { name: "Aluminium", density: 2700 },
  abs: { name: "ABS plastic", density: 1040 },
  plastic: { name: "ABS plastic", density: 1040 },
  oak: { name: "Oak", density: 750 },
  ply: { name: "Birch ply", density: 680 },
  plywood: { name: "Birch ply", density: 680 },
  mdf: { name: "MDF", density: 750 },
  glass: { name: "Glass", density: 2500 },
  concrete: { name: "Concrete", density: 2400 },
  pla: { name: "PLA", density: 1240 },
  nylon: { name: "Nylon", density: 1150 },
  wood: { name: "Oak", density: 750 },
  brass: { name: "Brass", density: 8500 },
  copper: { name: "Copper", density: 8960 },
  titanium: { name: "Titanium", density: 4430 },
};

const OPERATIONS: Record<string, string> = {
  "solid.box.create": "Added a box part",
  "solid.box.set_dimensions": "Changed dimensions",
  "solid.part.move": "Moved a part",
  "solid.box.edit": "Reshaped a part",
  "shape.create": "Created a shape",
  "shape.set_params": "Changed shape parameters",
  "shape.update": "Rewrote a shape",
  "assembly.create": "Created an assembly",
  "assembly.update": "Renamed an assembly",
  "assembly.instance.create": "Placed a component",
  "assembly.instance.update": "Changed an instance",
  "assembly.instance.make_unique": "Made an instance unique",
  "assembly.instance.delete": "Removed an instance",
  "solid.part.set_pivot": "Set a pivot",
  "solid.material.create": "Added a material",
  "solid.material.set_density": "Changed a density",
  "solid.material.assign": "Changed a part's material",
  "core.entity.delete": "Deleted an object",
  "intent.requirement.create": "Added a requirement",
  "intent.requirement.update": "Changed a requirement",
  "core.undo": "Undid a change",
  "core.redo": "Redid a change",
};

export const describeOperation = (op: string) => OPERATIONS[op] ?? op;

export type Look = { color: string; metalness: number; roughness: number; opacity?: number };

const LOOKS: Record<string, Look> = {
  glass: { color: "#8fb9cc", metalness: 0.1, roughness: 0.04, opacity: 0.38 },
  concrete: { color: "#c3bfb6", metalness: 0, roughness: 0.92 },
  "birch ply": { color: "#e3cb9f", metalness: 0, roughness: 0.7 },
  plywood: { color: "#e3cb9f", metalness: 0, roughness: 0.7 },
  "painted mdf": { color: "#a3b49c", metalness: 0, roughness: 0.55 },
  mdf: { color: "#b99b77", metalness: 0, roughness: 0.85 },
  walnut: { color: "#6f4a2f", metalness: 0, roughness: 0.65 },
  "white laminate": { color: "#f2f1ed", metalness: 0, roughness: 0.45 },
  "painted steel": { color: "#34373d", metalness: 0.35, roughness: 0.5 },
  "powder-coated steel": { color: "#34373d", metalness: 0.35, roughness: 0.5 },
  quartz: { color: "#ebe8e2", metalness: 0, roughness: 0.25 },
  pla: { color: "#4f7dd9", metalness: 0, roughness: 0.55 },
  petg: { color: "#e86a3a", metalness: 0, roughness: 0.45 },
  nylon: { color: "#ece9e1", metalness: 0, roughness: 0.7 },
  acrylic: { color: "#dfeef5", metalness: 0, roughness: 0.08, opacity: 0.5 },
  rubber: { color: "#25272b", metalness: 0, roughness: 0.95 },
  gypsum: { color: "#f4f3ef", metalness: 0, roughness: 0.9 },
  "acoustic felt": { color: "#7d8a86", metalness: 0, roughness: 1 },
  acetal: { color: "#f1eee6", metalness: 0, roughness: 0.4 },
  "stepper motor": { color: "#2e3136", metalness: 0.5, roughness: 0.45 },
  fr4: { color: "#2f7a4a", metalness: 0.1, roughness: 0.5 },
  silicone: { color: "#d84a3a", metalness: 0, roughness: 0.7 },
  "pool water": { color: "#3fa7c7", metalness: 0.05, roughness: 0.05, opacity: 0.7 },
  "galvanised steel": { color: "#b8c0c6", metalness: 0.55, roughness: 0.42 },
  "stainless steel": { color: "#d0d5d9", metalness: 0.8, roughness: 0.22 },
  steel: { color: "#a9b6bf", metalness: 0.62, roughness: 0.36 },
  aluminium: { color: "#c8d2da", metalness: 0.75, roughness: 0.28 },
  "abs plastic": { color: "#e7e2d8", metalness: 0, roughness: 0.6 },
  oak: { color: "#c69c6d", metalness: 0, roughness: 0.75 },
  brass: { color: "#c9a85c", metalness: 0.8, roughness: 0.3 },
  copper: { color: "#c27c56", metalness: 0.8, roughness: 0.3 },
  titanium: { color: "#9aa2a8", metalness: 0.7, roughness: 0.35 },
};

export const UNASSIGNED_LOOK: Look = { color: "#d6d4c7", metalness: 0.1, roughness: 0.65 };

/** How a material renders; unknown materials get a neutral look. */
export function materialLook(name: string | undefined): Look {
  if (!name) return UNASSIGNED_LOOK;
  const key = name.toLowerCase();
  if (LOOKS[key]) return LOOKS[key];
  // "Tempered glass", "Stainless steel", "6061 aluminium": match the family.
  const family = Object.keys(LOOKS).find((k) => key.includes(k));
  return family ? LOOKS[family] : { color: "#c4c9d4", metalness: 0.2, roughness: 0.5 };
}

/** A part's size and position in millimeters, as edited live in the UI. */
export type Shape = { width: number; depth: number; height: number; x: number; y: number; z: number };

export function shapeOf(e: EntityView): Shape {
  const inst = instance(e);
  if (inst) {
    const lo = e.assembly?.bounds_min ?? inst.at;
    const hi = e.assembly?.bounds_max ?? inst.at;
    return { x: lo[0], y: lo[1], z: lo[2], width: hi[0] - lo[0], depth: hi[1] - lo[1], height: hi[2] - lo[2] };
  }
  const p = placement(e);
  if (program(e)) {
    // Bounds in world space: the program's own coordinates, placed.
    const s = e.shape;
    const lo = s?.bounds_min ?? [0, 0, 0];
    const hi = s?.bounds_max ?? [0, 0, 0];
    return {
      x: toMm(p?.x) + lo[0], y: toMm(p?.y) + lo[1], z: toMm(p?.z) + lo[2],
      width: hi[0] - lo[0], depth: hi[1] - lo[1], height: hi[2] - lo[2],
    };
  }
  const b = box(e);
  return { width: toMm(b?.width), depth: toMm(b?.depth), height: toMm(b?.height), x: toMm(p?.x), y: toMm(p?.y), z: toMm(p?.z) };
}

export type LiveEdit = Shape & { id: string };

/** Pivot as fractions of the part's box (0 = min side, 1 = max side). Default: center. */
export type Frac = [number, number, number];

export function pivotOf(e: EntityView | undefined): Frac {
  const p = e?.components["solid.pivot"] as { u: number; v: number; w: number } | undefined;
  return p ? [p.u, p.v, p.w] : [0.5, 0.5, 0.5];
}

/** World position (mm) of a pivot on a shape. */
export function pivotPoint(s: Shape, f: Frac): [number, number, number] {
  return [s.x + f[0] * s.width, s.y + f[1] * s.depth, s.z + f[2] * s.height];
}

const SIDE = [
  ["left", "", "right"],
  ["front", "", "back"],
  ["bottom", "", "top"],
] as const;

/** Human name for a snap point: "Center", "Top face center", "Bottom left edge", "Top back right corner". */
export function pivotName(f: Frac) {
  const named = f.every((v) => Math.abs(v * 2 - Math.round(v * 2)) < 1e-9 && v >= 0 && v <= 1);
  if (!named) return f.some((v) => v < -1e-9 || v > 1 + 1e-9) ? "Custom point outside the part" : "Custom point";
  const words = [2, 1, 0].map((k) => SIDE[k][Math.round(f[k] * 2)]).filter(Boolean);
  const name = words.join(" ");
  const kind = ["Center", "face center", "edge midpoint", "corner"][words.length];
  if (!words.length) return "Center";
  return (name.charAt(0).toUpperCase() + name.slice(1)) + " " + kind;
}

/** The 27 snap points of a box: corners, edge midpoints, face centers and center. */
export const SNAP_FRACS: Frac[] = [0, 0.5, 1].flatMap((u) => [0, 0.5, 1].flatMap((v) => [0, 0.5, 1].map((w) => [u, v, w] as Frac)));
