// Design intent on the client: requirement types, plus a live mirror of the
// engine's exact box math so gauges and viewport overlays react while a part
// is being dragged. On commit the engine's own verdict replaces the preview.

import type { EntityView, RequirementResult } from "./engine";
import { type Shape, fmt, formatMass, instance, isPart, material, materialRef, program, shapeOf } from "./model";

type Q = { value: number; unit: string };

export type Rule =
  | { kind: "max_mass"; parts: string[]; limit: Q }
  | { kind: "fits_within"; parts: string[]; width: Q; depth: Q; height: Q }
  | { kind: "clearance"; a: string; b: string; min: Q }
  | { kind: "no_collisions"; parts: string[] };

export type Requirement = { name: string; rule: Rule };

export const requirement = (e: EntityView) => e.components["intent.requirement"] as Requirement | undefined;

export const targetsOf = (rule: Rule) => (rule.kind === "clearance" ? [rule.a, rule.b] : rule.parts);

const toMm = (q: Q) => q.value * 1000; // engine stores meters
const toKg = (q: Q) => q.value; // engine stores kilograms

export type Box3 = { min: [number, number, number]; max: [number, number, number] };

export function boxOf(s: Shape): Box3 {
  return { min: [s.x, s.y, s.z], max: [s.x + s.width, s.y + s.depth, s.z + s.height] };
}

const EPS = 1e-9;
const axes = [0, 1, 2] as const;

function separation(a: Box3, b: Box3) {
  return axes.map((k) => Math.max(b.min[k] - a.max[k], a.min[k] - b.max[k]));
}

export function distance(a: Box3, b: Box3) {
  return Math.sqrt(separation(a, b).reduce((sum, s) => sum + Math.max(s, 0) ** 2, 0));
}

export function overlap(a: Box3, b: Box3): Box3 | null {
  const min = axes.map((k) => Math.max(a.min[k], b.min[k])) as Box3["min"];
  const max = axes.map((k) => Math.min(a.max[k], b.max[k])) as Box3["max"];
  return axes.every((k) => max[k] - min[k] > EPS) ? { min, max } : null;
}

/** Closest points between two boxes (for drawing a clearance line). */
export function closestPoints(a: Box3, b: Box3): [[number, number, number], [number, number, number]] {
  const pa: number[] = [];
  const pb: number[] = [];
  for (const k of axes) {
    if (b.min[k] > a.max[k]) {
      pa.push(a.max[k]);
      pb.push(b.min[k]);
    } else if (a.min[k] > b.max[k]) {
      pa.push(a.min[k]);
      pb.push(b.max[k]);
    } else {
      const mid = (Math.max(a.min[k], b.min[k]) + Math.min(a.max[k], b.max[k])) / 2;
      pa.push(mid);
      pb.push(mid);
    }
  }
  return [pa as [number, number, number], pb as [number, number, number]];
}

export function union(boxes: Box3[]): Box3 {
  return {
    min: axes.map((k) => Math.min(...boxes.map((b) => b.min[k]))) as Box3["min"],
    max: axes.map((k) => Math.max(...boxes.map((b) => b.max[k]))) as Box3["max"],
  };
}

export type World = {
  shapes: Map<string, Shape>;
  massKg: Map<string, number | null>;
  /** AI-made shapes: only the engine's exact geometry can judge their contact. */
  exact: Set<string>;
};

/** Shapes and masses as currently displayed, including a live edit. */
export function displayedWorld(entities: ReadonlyMap<string, EntityView>, live: (Shape & { id: string }) | null): World {
  const shapes = new Map<string, Shape>();
  const massKg = new Map<string, number | null>();
  const exact = new Set<string>();
  for (const e of entities.values()) {
    if (!isPart(e)) continue;
    const s = live?.id === e.id ? live : shapeOf(e);
    shapes.set(e.id, s);
    if (program(e)) {
      exact.add(e.id);
      massKg.set(e.id, e.mass?.mass_kg ?? null);
      continue;
    }
    if (instance(e)) {
      exact.add(e.id);
      massKg.set(e.id, e.assembly?.mass_kg ?? null);
      continue;
    }
    const ref = materialRef(e);
    const mat = ref ? entities.get(ref) : undefined;
    const density = mat ? material(mat)?.density.value : undefined;
    massKg.set(e.id, density === undefined ? null : ((s.width * s.depth * s.height) / 1e9) * density);
  }
  return { shapes, massKg, exact };
}

/** Client-side evaluation mirroring the engine (preview only). */
export function evaluateRule(rule: Rule, world: World): Pick<RequirementResult, "passed" | "measured" | "limit" | "unit" | "upper_bound" | "summary"> | null {
  const shapes = targetsOf(rule).map((id) => world.shapes.get(id));
  if (shapes.some((s) => !s)) return null;
  // Contact between AI-made shapes needs their exact solids: leave it to the engine.
  if ((rule.kind === "clearance" || rule.kind === "no_collisions") && targetsOf(rule).some((id) => world.exact.has(id))) return null;
  const boxes = (shapes as Shape[]).map(boxOf);
  switch (rule.kind) {
    case "max_mass": {
      const masses = rule.parts.map((id) => world.massKg.get(id));
      if (masses.some((m) => m == null)) return null;
      const total = (masses as number[]).reduce((a, b) => a + b, 0);
      const limit = toKg(rule.limit);
      return { passed: total <= limit * (1 + 1e-12), measured: total, limit, unit: "kg", upper_bound: true, summary: `${formatMass(total)} of ${formatMass(limit)} max` };
    }
    case "fits_within": {
      const u = union(boxes);
      const size = axes.map((k) => u.max[k] - u.min[k]);
      const env = [toMm(rule.width), toMm(rule.depth), toMm(rule.height)];
      const worst = Math.max(...axes.map((k) => size[k] / env[k]));
      return {
        passed: axes.every((k) => size[k] <= env[k] + EPS),
        measured: worst * 100,
        limit: 100,
        unit: "%",
        upper_bound: true,
        summary: `${size.map(fmt).join(" × ")} in ${env.map(fmt).join(" × ")} mm`,
      };
    }
    case "clearance": {
      const [a, b] = boxes;
      const o = overlap(a, b);
      const d = distance(a, b);
      const min = toMm(rule.min);
      return {
        passed: !o && d + EPS >= min,
        measured: o ? 0 : d,
        limit: min,
        unit: "mm",
        upper_bound: false,
        summary: o ? "Parts overlap" : `${fmt(d)} mm apart, ${fmt(min)} mm required`,
      };
    }
    case "no_collisions": {
      let n = 0;
      for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) if (overlap(boxes[i], boxes[j])) n++;
      return {
        passed: n === 0,
        measured: n,
        limit: 0,
        unit: "collisions",
        upper_bound: true,
        summary: n === 0 ? `${boxes.length} parts, none overlapping` : n === 1 ? "1 pair of parts overlaps" : `${n} pairs of parts overlap`,
      };
    }
  }
}

/** What the viewport draws for intent. */
export type Overlay =
  | { kind: "gap"; from: [number, number, number]; to: [number, number, number]; label: string; ok: boolean; focus: boolean }
  | { kind: "region"; box: Box3; label?: string }
  | { kind: "envelope"; box: Box3; ok: boolean; focus: boolean };

/** Overlays for the requirements, plus any part overlap anywhere. */
export function overlaysFor(requirements: EntityView[], world: World, focus: string | null): Overlay[] {
  const out: Overlay[] = [];
  for (const e of requirements) {
    const req = requirement(e);
    if (!req) continue;
    const rule = req.rule;
    const isFocus = focus === e.id;
    if (rule.kind === "clearance") {
      const a = world.shapes.get(rule.a);
      const b = world.shapes.get(rule.b);
      if (!a || !b || world.exact.has(rule.a) || world.exact.has(rule.b)) continue;
      const [ba, bb] = [boxOf(a), boxOf(b)];
      if (overlap(ba, bb)) continue; // drawn as a collision region below
      const d = distance(ba, bb);
      const [from, to] = closestPoints(ba, bb);
      out.push({ kind: "gap", from, to, label: `${fmt(d)} mm`, ok: d + EPS >= toMm(rule.min), focus: isFocus });
    } else if (rule.kind === "fits_within") {
      const boxes = rule.parts.map((id) => world.shapes.get(id)).filter(Boolean).map((s) => boxOf(s!));
      if (!boxes.length) continue;
      const u = union(boxes);
      const env: Box3 = { min: u.min, max: [u.min[0] + toMm(rule.width), u.min[1] + toMm(rule.depth), u.min[2] + toMm(rule.height)] };
      const ok = axes.every((k) => u.max[k] <= env.max[k] + EPS);
      out.push({ kind: "envelope", box: env, ok, focus: isFocus });
    }
  }
  // Every overlap between any two parts is shown; collisions are never silent.
  const ids = [...world.shapes.keys()].filter((id) => !world.exact.has(id));
  for (let i = 0; i < ids.length; i++) {
    for (let j = i + 1; j < ids.length; j++) {
      const o = overlap(boxOf(world.shapes.get(ids[i])!), boxOf(world.shapes.get(ids[j])!));
      if (o) out.push({ kind: "region", box: o, label: `${fmt((o.max[0] - o.min[0]) * (o.max[1] - o.min[1]) * (o.max[2] - o.min[2]))} mm³` });
    }
  }
  return out;
}

/** Sensible starting rules from the current design, so a new requirement passes until tightened. */
export function suggestRule(kind: Rule["kind"], parts: EntityView[], selected: string | null, world: World): Rule | string {
  const ids = parts.map((p) => p.id);
  const sel = selected && world.shapes.has(selected) ? selected : null;
  switch (kind) {
    case "max_mass": {
      const target = sel ? [sel] : ids;
      if (!target.length) return "Add a part first.";
      const masses = target.map((id) => world.massKg.get(id));
      if (masses.some((m) => m == null)) return "Give the part a material first, so its mass is known.";
      const total = (masses as number[]).reduce((a, b) => a + b, 0);
      const nice = niceCeil(total * 1.1);
      return { kind, parts: target, limit: { value: nice, unit: "kg" } };
    }
    case "fits_within": {
      const target = sel ? [sel] : ids;
      if (!target.length) return "Add a part first.";
      const u = union(target.map((id) => boxOf(world.shapes.get(id)!)));
      const size = axes.map((k) => Math.ceil((u.max[k] - u.min[k] + 5) / 10) * 10);
      return { kind, parts: target, width: { value: size[0], unit: "mm" }, depth: { value: size[1], unit: "mm" }, height: { value: size[2], unit: "mm" } };
    }
    case "clearance": {
      if (ids.length < 2) return "Clearance needs two parts.";
      const a = sel ?? ids[0];
      const others = ids.filter((id) => id !== a);
      const ba = boxOf(world.shapes.get(a)!);
      const b = others.reduce((best, id) => (distance(ba, boxOf(world.shapes.get(id)!)) < distance(ba, boxOf(world.shapes.get(best)!)) ? id : best));
      return { kind, a, b, min: { value: 2, unit: "mm" } };
    }
    case "no_collisions":
      return ids.length < 2 ? "Collision checks need two parts." : { kind, parts: ids };
  }
}

function niceCeil(v: number) {
  if (v <= 0) return 0.1;
  const magnitude = 10 ** Math.floor(Math.log10(v));
  const step = magnitude / 2;
  return Math.ceil(v / step) * step;
}
