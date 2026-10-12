// Selection helpers: select all, by type, invert; and copy, paste and
// duplicate parts and instances as new items (new ids) set off to one side.

import type { EntityView } from "./engine";
import { box, instance, isPart, placement, program, type Program } from "./model";

export type ItemType = "part" | "box" | "instance" | "assembly";

export const ITEM_TYPES: { type: ItemType; label: string }[] = [
  { type: "part", label: "Parts" },
  { type: "box", label: "Boxes" },
  { type: "instance", label: "Component instances" },
  { type: "assembly", label: "Assembly instances" },
];

/** What kind of top-level item this is. */
export function typeOf(e: EntityView, entities: ReadonlyMap<string, EntityView>): ItemType | null {
  if (!isPart(e)) return null;
  if (box(e)) return "box";
  const inst = instance(e);
  if (inst) return entities.get(inst.of)?.components["assembly.definition"] ? "assembly" : "instance";
  return "part";
}

/** Top-level items that can be selected in the view (not hidden). */
export function selectable(entities: ReadonlyMap<string, EntityView>, hidden: ReadonlySet<string> = new Set()): string[] {
  return [...entities.values()].filter((e) => isPart(e) && !hidden.has(e.id)).map((e) => e.id);
}

export function ofType(entities: ReadonlyMap<string, EntityView>, type: ItemType, hidden?: ReadonlySet<string>): string[] {
  return selectable(entities, hidden).filter((id) => typeOf(entities.get(id)!, entities) === type);
}

/** Everything selectable that is not selected now. */
export function inverted(entities: ReadonlyMap<string, EntityView>, selected: readonly string[], hidden?: ReadonlySet<string>): string[] {
  const now = new Set(selected);
  return selectable(entities, hidden).filter((id) => !now.has(id));
}

export type Step = { operation: string; parameters: Record<string, unknown> };

/** A copy of one item: the steps that make it, and changes to apply to what they made. */
export type CopyRecipe = { step: Step; after?: (id: string) => Step | null };

const metres = (mm: number) => ({ value: mm / 1000, unit: "m" });

/** The name of a copy: "Bracket 2", "Bracket 3"… whichever is free. */
export function copyName(name: string, taken: ReadonlySet<string>): string {
  const base = name.replace(/\s+\d+$/, "");
  for (let n = 2; ; n++) {
    const candidate = `${base} ${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/**
 * Steps that make copies of `ids`, each set off by `offset` (mm) from its
 * original. Unknown or unplaced items are skipped.
 */
export function copySteps(ids: readonly string[], entities: ReadonlyMap<string, EntityView>, offset: [number, number, number]): CopyRecipe[] {
  const taken = new Set<string>();
  for (const e of entities.values()) {
    const n = instance(e)?.name ?? program(e)?.name;
    if (n) taken.add(n);
  }
  const out: CopyRecipe[] = [];
  for (const id of ids) {
    const e = entities.get(id);
    if (!e) continue;
    const inst = instance(e);
    if (inst) {
      const name = copyName(inst.name, taken);
      taken.add(name);
      out.push({
        step: {
          operation: "assembly.instance.create",
          parameters: {
            of: inst.of,
            parent: inst.parent ?? null,
            name,
            at: [inst.at[0] + offset[0], inst.at[1] + offset[1], inst.at[2] + offset[2]],
            rotation: Array.isArray(inst.rotation) ? inst.rotation : [0, 0, Number(inst.rotation) || 0],
            properties: inst.properties ?? {},
            config: inst.config ?? null,
          },
        },
      });
      continue;
    }
    const p = placement(e);
    const at = p ? [p.x.value * 1000 + offset[0], p.y.value * 1000 + offset[1], p.z.value * 1000 + offset[2]] : null;
    const material = (e.components["solid.material_ref"] as string | undefined) ?? null;
    const b = box(e);
    if (b && at) {
      out.push({ step: { operation: "solid.box.create", parameters: { width: b.width, depth: b.depth, height: b.height, material, placement: { x: metres(at[0]), y: metres(at[1]), z: metres(at[2]) } } } });
      continue;
    }
    const prog = program(e) as (Program & { uses?: Record<string, string> }) | undefined;
    if (prog && at) {
      const name = copyName(prog.name, taken);
      taken.add(name);
      out.push({
        step: {
          operation: "shape.create",
          parameters: {
            name,
            description: prog.description,
            source: prog.source,
            params: prog.params,
            display_tolerance: prog.display_tolerance ?? null,
            levels: prog.levels ?? [],
            placement: { x: metres(at[0]), y: metres(at[1]), z: metres(at[2]) },
            material,
            materials: (e.components["shape.materials"] as Record<string, string> | undefined) ?? {},
          },
        },
        // Rounded and bevelled edges, and sketches the program reads, are set after it is made.
        after: prog.finishes?.length || prog.uses ? (made) => ({ operation: "shape.update", parameters: { target: made, ...(prog.finishes?.length ? { finishes: prog.finishes } : {}) } }) : undefined,
      });
    }
  }
  return out;
}

/** How far a copy is set off: beside the originals along X, with a gap. */
export function copyOffset(ids: readonly string[], entities: ReadonlyMap<string, EntityView>): [number, number, number] {
  let lo = Infinity;
  let hi = -Infinity;
  for (const id of ids) {
    const e = entities.get(id);
    const s = e?.assembly ?? e?.shape;
    if (!e || !s) continue;
    const p = placement(e);
    const dx = instance(e) ? 0 : p ? p.x.value * 1000 : 0;
    lo = Math.min(lo, s.bounds_min[0] + dx);
    hi = Math.max(hi, s.bounds_max[0] + dx);
  }
  const width = Number.isFinite(hi - lo) ? hi - lo : 50;
  return [Math.round(width + Math.max(10, width * 0.1)), 0, 0];
}
