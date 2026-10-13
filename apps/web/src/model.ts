// Typed readers over engine entity views.

import type { Configured, EntityView } from "./engine";

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
export type EdgeFinish = {
  kind: "fillet" | "chamfer" | "push" | "shell" | "draft";
  key: string;
  part?: string;
  /** Edges, or faces (push; shell: those left open; draft: those tapered, none for every wall). */
  edges: EdgePick[];
  /** Draft: the direction the mould opens, and the neutral plane as a fraction (0–1) of the part along it. */
  pull?: [number, number, number];
  neutral?: number;
};

/** An AI-made shape's program, if this entity is one. */
export const program = (e: EntityView) => e.components["shape.program"] as Program | undefined;

/** One configuration of a shape: a value for each column, finishes it leaves off (by size parameter) and parts it leaves out (by name). */
export type ConfigRow = { name: string; values: Record<string, number>; description?: string; suppress?: string[]; parts_off?: string[] };
/** A shape's configuration table: columns (parameter keys), rows, and the one the shape shows. */
export type Configurations = { params: string[]; rows: ConfigRow[]; active?: string | null };
export const configurations = (e: EntityView | undefined) => e?.components["shape.configurations"] as Configurations | undefined;
/** “Bolt (M8×30)”. */
export const configName = (name: string, config?: string | null) => (config ? `${name} (${config})` : name);

/** An assembly's material table: names its parts use → material entity ids. */
export const partMaterials = (e: EntityView) => (e.components["shape.materials"] as Record<string, string> | undefined) ?? {};

/** The material entity a part of an assembly is made of (or the shape's own). */
export function partMaterial(e: EntityView, materialName: string | null, entities: ReadonlyMap<string, EntityView>): EntityView | undefined {
  const id = materialName ? partMaterials(e)[materialName] : materialRef(e);
  return id ? entities.get(id) : undefined;
}

/** The parts of an assembly (empty for a single solid). */
export const assemblyParts = (e: EntityView | undefined) => e?.shape?.parts ?? [];
/** The key that hides one piece of a part made of several (the tree's eye on a sub-part). */
export const subPartKey = (id: string, name: string) => `${id}#${name}`;

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
  pcb_fab: "PCB fab",
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
  /** Degrees about X, then Y, then Z. */
  rotation: [number, number, number];
  properties?: Record<string, string>;
  /** The configuration of its shape it shows (none: the definition as it is). */
  config?: string | null;
  /** Values of its own for parameters of its shape: a number, or a formula (stored by id) it follows. */
  params?: Record<string, { value: number; expr?: string }>;
};

/** Whether an instance shows its shape other than as defined: in a configuration, or with values of its own. */
export const variesShape = (i: InstanceData | undefined) => !!i && (!!i.config || Object.keys(i.params ?? {}).length > 0);

/** A stored formula (`=#<id>.angle * 2`) with references by name, for people (`=Shoulder.angle * 2`). */
export function formulaText(stored: string, entities: ReadonlyMap<string, EntityView>) {
  return stored.replace(/#([0-9a-f-]{36})\.([A-Za-z_][\w]*)/g, (_, id: string, key: string) => {
    const e = entities.get(id);
    const name = (e?.components["shape.parameter_set"] as { name?: string } | undefined)?.name ?? (e?.components["shape.program"] as { name?: string } | undefined)?.name ?? "(missing)";
    return `${name}.${key}`;
  });
}

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
  const index = indexOf(entities);
  let list = index.sorted.get(defId);
  if (!list) {
    list = (index.children.get(defId) ?? []).sort((a, b) => instance(a)!.at[2] - instance(b)!.at[2] || instance(a)!.name.localeCompare(instance(b)!.name, undefined, { numeric: true }));
    index.sorted.set(defId, list);
  }
  return [...list];
}

/** How many instances use this definition. */
export const usesOf = (defId: string, entities: ReadonlyMap<string, EntityView>) => indexOf(entities).uses.get(defId)?.length ?? 0;

/** The first instance of a definition (where an assembly is placed), if any. */
export const firstUseOf = (defId: string, entities: ReadonlyMap<string, EntityView>) => indexOf(entities).uses.get(defId)?.[0];

type Index = { children: Map<string, EntityView[]>; sorted: Map<string, EntityView[]>; uses: Map<string, EntityView[]> };
const indexes = new WeakMap<ReadonlyMap<string, EntityView>, Index>();

/**
 * Instances by the assembly they sit in and by what they place, built once
 * per state of the project (every change makes a new entity map): asking for
 * an assembly's children is then a lookup, not a pass over every entity,
 * which a 10,000-part project asks thousands of times per update.
 */
function indexOf(entities: ReadonlyMap<string, EntityView>): Index {
  let index = indexes.get(entities);
  if (!index) {
    index = { children: new Map(), sorted: new Map(), uses: new Map() };
    for (const e of entities.values()) {
      const inst = instance(e);
      if (!inst) continue;
      if (inst.parent) (index.children.get(inst.parent) ?? index.children.set(inst.parent, []).get(inst.parent)!).push(e);
      (index.uses.get(inst.of) ?? index.uses.set(inst.of, []).get(inst.of)!).push(e);
    }
    indexes.set(entities, index);
  }
  return index;
}

export type V3 = [number, number, number];
export type Quat = [number, number, number, number];
/** A rigid placement: a point p goes to q·p + at (q a unit quaternion [x, y, z, w]). */
export type Transform = { at: V3; q: Quat };
export const IDENTITY: Transform = { at: [0, 0, 0], q: [0, 0, 0, 1] };

export function qmul(a: Quat, b: Quat): Quat {
  const [ax, ay, az, aw] = a;
  const [bx, by, bz, bw] = b;
  return [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz];
}

export function qrotate(q: Quat, v: V3): V3 {
  const [x, y, z, w] = q;
  // v + 2w(u×v) + 2u×(u×v), u = (x, y, z)
  const cx = y * v[2] - z * v[1], cy = z * v[0] - x * v[2], cz = x * v[1] - y * v[0];
  return [v[0] + 2 * (w * cx + y * cz - z * cy), v[1] + 2 * (w * cy + z * cx - x * cz), v[2] + 2 * (w * cz + x * cy - y * cx)];
}

/** Degrees about X, then Y, then Z (fixed axes), as a quaternion. */
export function quatFromEuler(deg: readonly number[]): Quat {
  const half = deg.map((d) => (d * Math.PI) / 360);
  const qx: Quat = [Math.sin(half[0]), 0, 0, Math.cos(half[0])];
  const qy: Quat = [0, Math.sin(half[1]), 0, Math.cos(half[1])];
  const qz: Quat = [0, 0, Math.sin(half[2]), Math.cos(half[2])];
  return qmul(qz, qmul(qy, qx));
}

export const applyT = (t: Transform, p: V3): V3 => {
  const r = qrotate(t.q, p);
  return [r[0] + t.at[0], r[1] + t.at[1], r[2] + t.at[2]];
};

export const invertT = (t: Transform): Transform => {
  const q: Quat = [-t.q[0], -t.q[1], -t.q[2], t.q[3]];
  const r = qrotate(q, t.at);
  return { at: [-r[0], -r[1], -r[2]], q };
};

/** `inner` placed inside `outer` (a point goes through inner, then outer). */
export function compose(inner: Transform, outer: Transform): Transform {
  return { at: applyT(outer, inner.at), q: qmul(outer.q, inner.q) };
}

/** Where an instance sits in its parent now: where its joints put it, else its own position and rotation. */
export function placedOf(e: EntityView | undefined): Transform {
  const inst = instance(e);
  if (!inst) return IDENTITY;
  if (e?.placed) return { at: e.placed.at, q: e.placed.quaternion };
  const r = Array.isArray(inst.rotation) ? inst.rotation : [0, 0, Number(inst.rotation) || 0];
  return { at: inst.at, q: quatFromEuler(r) };
}

/** An instance in its own definition's coordinates, placed in the project (through every assembly above it). */
export function worldOf(id: string, entities: ReadonlyMap<string, EntityView>, parent = false): Transform {
  let t = IDENTITY;
  let current = id;
  for (let i = 0; i < 32; i++) {
    const e = entities.get(current);
    const inst = instance(e);
    if (!inst) break;
    // `parent`: where it and its siblings are placed, without its own placement.
    if (!(parent && i === 0)) t = compose(t, placedOf(e));
    if (!inst.parent) break;
    const owner = firstUseOf(inst.parent, entities);
    if (!owner) break;
    current = owner.id;
  }
  return t;
}

/** One shape somewhere inside an instance: which component, where, and the instance path to it
 * (and, when its instance shows a configuration, that configuration evaluated). */
export type Occurrence = { shape: string; path: string[]; transform: Transform; configured?: Configured };

/** Every shape under an instance (or assembly definition), in the instance's parent coordinates.
 * `poses` stands in for instances' own placements (a joint being scrubbed). */
export function occurrences(id: string, entities: ReadonlyMap<string, EntityView>, poses?: ReadonlyMap<string, Transform> | null): Occurrence[] {
  const out: Occurrence[] = [];
  const walk = (eid: string, t: Transform, path: string[], depth: number) => {
    if (depth > 32) return;
    const e = entities.get(eid);
    if (!e) return;
    const inst = instance(e);
    if (inst) return walk(inst.of, compose(poses?.get(eid) ?? placedOf(e), t), [...path, eid], depth + 1);
    if (assemblyDef(e)) {
      for (const c of childrenOf(eid, entities)) walk(c.id, t, path, depth + 1);
      return;
    }
    if (program(e)) {
      const holder = entities.get(path[path.length - 1]);
      const configured = variesShape(instance(holder)) ? (holder?.configured ?? undefined) : undefined;
      out.push({ shape: eid, path, transform: t, configured });
    }
  };
  walk(id, IDENTITY, [], 0);
  return out;
}

/** The top-level instance an instance sits in (itself when it is top-level). */
export function topInstanceOf(id: string, entities: ReadonlyMap<string, EntityView>): string {
  let current = id;
  for (let i = 0; i < 32; i++) {
    const inst = instance(entities.get(current));
    if (!inst?.parent) return current;
    const parentUse = firstUseOf(inst.parent, entities);
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
  if (prog) return configName(prog.name, configurations(e)?.active);
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
  "shape.config.set_table": "Edited configurations",
  "shape.config.activate": "Switched configuration",
  "assembly.create": "Created an assembly",
  "assembly.update": "Renamed an assembly",
  "assembly.instance.create": "Placed a component",
  "assembly.instance.update": "Changed an instance",
  "assembly.instance.make_unique": "Made an instance unique",
  "assembly.instance.delete": "Removed an instance",
  "solid.part.set_pivot": "Set a pivot",
  "solid.material.create": "Added a material",
  "solid.material.set_density": "Changed a density",
  "solid.material.set_cost": "Priced a material",
  "sourcing.supplier.create": "Added a supplier",
  "sourcing.supplier.update": "Changed a supplier",
  "sourcing.rfq.create": "Made a request for quote",
  "sourcing.rfq.update": "Changed a request for quote",
  "sourcing.quote.create": "Recorded a quote",
  "sourcing.quote.update": "Changed a quote",
  "sourcing.quote.award": "Accepted a quote",
  "sourcing.award": "Awarded a line",
  "sourcing.award.clear": "Withdrew an award",
  "sourcing.rates.set": "Changed process rates",
  "solid.material.assign": "Changed a part's material",
  "core.entity.delete": "Deleted an object",
  "intent.requirement.create": "Added a requirement",
  "intent.requirement.update": "Changed a requirement",
  "comment.create": "Commented",
  "comment.reply": "Replied to a comment",
  "comment.resolve": "Resolved a comment",
  "comment.reopen": "Reopened a comment",
  "comment.edit": "Edited a comment",
  "comment.delete": "Deleted a comment",
  "comment.markup": "Drew on a comment",
  "version.create": "Marked a version",
  "version.update": "Changed a version",
  "version.delete": "Removed a version name",
  "version.restore": "Restored a version",
  "project.set_info": "Changed the project's details",
  "core.undo": "Undid a change",
  "core.redo": "Redid a change",
  "core.batch": "Made several changes",
  "assembly.group.create": "Grouped parts",
  "assembly.group.update": "Changed a group",
  "parameters.create": "Added parameters",
  "parameters.set": "Changed parameters",
  "shape.link_param": "Linked a parameter",
  "shape.rename_datum": "Renamed a datum",
  "shape.set_drawing": "Edited a drawing",
  "shape.set_study": "Changed a stress or heat study",
  "sketch.create": "Started a sketch",
  "sketch.update": "Edited a sketch",
  "sketch.combine": "Combined sketch outlines",
  "sketch.offset": "Offset a sketch outline",
  "sketch.trim": "Trimmed sketch lines",
  "sketch.extend": "Extended sketch lines",
  "sketch.fillet": "Rounded sketch corners",
  "sketch.chamfer": "Bevelled sketch corners",
  "sketch.pattern": "Patterned sketch geometry",
  "sketch.mirror": "Mirrored sketch geometry",
  "sketch.project": "Projected a part into a sketch",
};

/** What a change did, in words: "shape.set_study" → "Changed a stress or heat study"; an
 * operation without a label still reads as words ("solid.thing.set_size" → "Set size (thing)"). */
export const describeOperation = (op: string) => {
  if (OPERATIONS[op]) return OPERATIONS[op];
  const parts = op.split(".");
  const verb = (parts.pop() ?? op).replace(/_/g, " ");
  const what = parts.length > 1 ? ` (${parts.slice(1).join(" ").replace(/_/g, " ")})` : "";
  return verb.charAt(0).toUpperCase() + verb.slice(1) + what;
};

export type Look = { color: string; metalness: number; roughness: number; opacity?: number; picture?: string };

const LOOKS: Record<string, Look> = {
  // Printed circuit boards (the PCB workspace's 3D part): exact names, matched before any family.
  "fr4 green mask": { color: "#2b6e45", metalness: 0.05, roughness: 0.42 },
  "fr4 black mask": { color: "#1d2024", metalness: 0.05, roughness: 0.4 },
  "fr4 blue mask": { color: "#1f4f8a", metalness: 0.05, roughness: 0.42 },
  "fr4 red mask": { color: "#8c2a2a", metalness: 0.05, roughness: 0.42 },
  "fr4 white mask": { color: "#e9e9e4", metalness: 0.02, roughness: 0.5 },
  "fr4 purple mask": { color: "#4b2a78", metalness: 0.05, roughness: 0.42 },
  "fr4 yellow mask": { color: "#c2a42a", metalness: 0.05, roughness: 0.45 },
  "epoxy moulding (black)": { color: "#25262a", metalness: 0.05, roughness: 0.6 },
  "ceramic (mlcc)": { color: "#b8996a", metalness: 0.05, roughness: 0.6 },
  "black plastic (pbt)": { color: "#202125", metalness: 0, roughness: 0.65 },
  "nylon (pa66, connector)": { color: "#efe8d6", metalness: 0, roughness: 0.7 },
  "gold-plated pins": { color: "#d4af37", metalness: 0.85, roughness: 0.25 },
  "led epoxy red": { color: "#e8392f", metalness: 0, roughness: 0.2, opacity: 0.88 },
  "led epoxy green": { color: "#3cc76a", metalness: 0, roughness: 0.2, opacity: 0.88 },
  "led epoxy blue": { color: "#3a6ff0", metalness: 0, roughness: 0.2, opacity: 0.88 },
  "led epoxy yellow": { color: "#f2c230", metalness: 0, roughness: 0.2, opacity: 0.88 },
  "led epoxy orange": { color: "#f28a2a", metalness: 0, roughness: 0.2, opacity: 0.88 },
  "led epoxy white": { color: "#f4f4ef", metalness: 0, roughness: 0.2, opacity: 0.9 },
  "led epoxy clear": { color: "#e8eef2", metalness: 0, roughness: 0.1, opacity: 0.7 },
  // Stock-library bodies (lumped materials named for the item).
  "sg90 body": { color: "#2f62d4", metalness: 0, roughness: 0.38 },
  "mg90s body": { color: "#232428", metalness: 0.05, roughness: 0.42 },
  "mg996r body": { color: "#1d1e22", metalness: 0.05, roughness: 0.42 },
  "servo lead": { color: "#b0532c", metalness: 0, roughness: 0.6 },
  stator: { color: "#2c2f34", metalness: 0.45, roughness: 0.42 },
  glass: { color: "#8fb9cc", metalness: 0.1, roughness: 0.04, opacity: 0.38 },
  concrete: { color: "#b3afa6", metalness: 0, roughness: 0.92 },
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
  steel: { color: "#a2a9ae", metalness: 0.62, roughness: 0.36 },
  aluminium: { color: "#c8d2da", metalness: 0.75, roughness: 0.28 },
  "abs plastic": { color: "#e7e2d8", metalness: 0, roughness: 0.6 },
  oak: { color: "#c69c6d", metalness: 0, roughness: 0.75 },
  brass: { color: "#c9a85c", metalness: 0.8, roughness: 0.3 },
  copper: { color: "#c27c56", metalness: 0.8, roughness: 0.3 },
  titanium: { color: "#9aa2a8", metalness: 0.7, roughness: 0.35 },
  // Building materials (after the metals and plastics, so "aluminium block" stays aluminium).
  brick: { color: "#985a45", metalness: 0, roughness: 0.9 },
  "roof tile": { color: "#a4523a", metalness: 0, roughness: 0.85 },
  "clay tile": { color: "#a4523a", metalness: 0, roughness: 0.85 },
  timber: { color: "#c19467", metalness: 0, roughness: 0.75 },
  stone: { color: "#b5aa95", metalness: 0, roughness: 0.85 },
  granite: { color: "#8e8a86", metalness: 0, roughness: 0.6 },
  marble: { color: "#e9e6e0", metalness: 0, roughness: 0.3 },
  slate: { color: "#50565d", metalness: 0, roughness: 0.7 },
  plaster: { color: "#ebe6dc", metalness: 0, roughness: 0.9 },
  asphalt: { color: "#3e3f42", metalness: 0, roughness: 0.95 },
  block: { color: "#b2afa7", metalness: 0, roughness: 0.92 },
};

export const UNASSIGNED_LOOK: Look = { color: "#d6d4c7", metalness: 0.1, roughness: 0.65 };

/** Colour words a material name may carry: "PLA (Bambu, black)", "PETG orange", "PLA charcoal". */
const COLOUR_WORDS: Record<string, string> = {
  black: "#232427",
  charcoal: "#3a3c40",
  "dark grey": "#4a4d52",
  "dark gray": "#4a4d52",
  grey: "#8d9196",
  gray: "#8d9196",
  silver: "#c3c7cb",
  white: "#f1f0ec",
  ivory: "#eee8d8",
  natural: "#e8e1cf",
  beige: "#d9c9a8",
  red: "#c8372f",
  orange: "#e8742c",
  yellow: "#f0c430",
  gold: "#c9a24a",
  green: "#3e9a54",
  olive: "#6f7a3a",
  teal: "#2a8c8c",
  cyan: "#3fb3cf",
  blue: "#3a6fd0",
  navy: "#243a66",
  purple: "#7a4fc4",
  violet: "#7a4fc4",
  pink: "#e27aa8",
  brown: "#7a5236",
};

/**
 * A picture a material prints (see pictures.ts): a stored picture's file
 * name in its name ("Label print battery-label.png"), or its id or the id's
 * first 8 or more characters ("Label print 1f6b353f").
 */
export function pictureIn(name: string): string | undefined {
  const file = name.match(/[\w.-]+\.(?:png|jpe?g)\b/i)?.[0];
  if (file) return file;
  return name.match(/\b[0-9a-f]{8,64}\b/gi)?.find((t) => /[0-9]/.test(t) && /[a-f]/i.test(t));
}

/** How a material renders; unknown materials get a neutral look. A colour in the name wins over the family's own. */
export function materialLook(name: string | undefined): Look {
  if (!name) return UNASSIGNED_LOOK;
  const key = name.toLowerCase();
  if (LOOKS[key]) return LOOKS[key];
  // "Tempered glass", "Stainless steel", "6061 aluminium": match the family.
  // A family is a whole word of the name (plurals too): "nickel-plated steel" is steel, not PLA.
  const family = Object.keys(LOOKS).find((k) => new RegExp(`(^|[^a-z])${k.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(s|es)?([^a-z]|$)`).test(key));
  const look = family ? LOOKS[family] : { color: "#c4c9d4", metalness: 0.2, roughness: 0.5 };
  const hex = key.match(/#[0-9a-f]{6}\b/)?.[0];
  const word = Object.keys(COLOUR_WORDS)
    .sort((a, b) => b.length - a.length) // "dark grey" before "grey"
    .find((w) => new RegExp(`\\b${w}\\b`).test(key));
  const color = hex ?? (word ? COLOUR_WORDS[word] : undefined);
  // A printed picture ("Label print battery-label.png"): its own colours on a satin print.
  const picture = pictureIn(name);
  if (picture) return { color: "#ffffff", metalness: 0, roughness: family ? look.roughness : 0.45, picture };
  return color ? { ...look, color } : look;
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
