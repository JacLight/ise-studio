// The BOM and the 3D view, linked: a line lights (and selects) every instance
// of what it lists, and a selection in the view finds its lines. A line says
// which definitions its parts come from (shape, configuration, piece); the
// scene says where each definition is drawn.

import type { EntityView } from "./engine";
import { box, configurations, instance, isPart, occurrences, program } from "./model";

/** One definition a BOM line's parts come from (the engine's `uses`). */
export type LineUse = { shape: string; config?: string; part?: string };

/** Where definitions are drawn: "shape|config" → the ids that light or select
 * each copy (the instance holding it, at any depth, or the placed shape). */
export type SceneIndex = Map<string, string[]>;

const at = (shape: string, config: string | null | undefined) => `${shape}|${config ?? ""}`;

/** Every copy of every definition in `scope` (an instance, assembly or placed item), or the whole project. */
export function sceneIndex(entities: ReadonlyMap<string, EntityView>, scope: string | null): SceneIndex {
  const index: SceneIndex = new Map();
  const add = (key: string, holder: string) => {
    const list = index.get(key);
    if (!list) index.set(key, [holder]);
    else if (!list.includes(holder)) list.push(holder);
  };
  const shown = (shape: string) => configurations(entities.get(shape))?.active ?? null;
  const visit = (e: EntityView) => {
    if (box(e)) return add(at(e.id, null), e.id);
    if (program(e) && !instance(e)) return add(at(e.id, shown(e.id)), e.id);
    for (const o of occurrences(e.id, entities)) {
      const holder = o.path[o.path.length - 1] ?? e.id;
      add(at(o.shape, instance(entities.get(holder))?.config || shown(o.shape)), holder);
    }
  };
  const root = scope ? entities.get(scope) : undefined;
  if (root && (instance(root) || box(root) || !program(root))) visit(root);
  else for (const e of entities.values()) if (isPart(e)) visit(e);
  return index;
}

/** The view keys ("id" or "id#piece") of every copy a line's uses stand for. */
export function keysFor(uses: readonly LineUse[] | undefined, index: SceneIndex): string[] {
  const out: string[] = [];
  for (const u of uses ?? []) {
    let holders = index.get(at(u.shape, u.config));
    // A configuration named differently here than in the engine: every copy of the shape.
    if (!holders) holders = [...index].filter(([k]) => k.startsWith(`${u.shape}|`)).flatMap(([, h]) => h);
    for (const h of holders) {
      const key = u.part ? `${h}#${u.part}` : h;
      if (!out.includes(key)) out.push(key);
    }
  }
  return out;
}

/** Every copy of a definition (a shape or an assembly) in the index: for the indented BOM's rows. */
export function definitionKeys(def: string, config: string | undefined, entities: ReadonlyMap<string, EntityView>, index: SceneIndex): string[] {
  const e = entities.get(def);
  if (!e) return [];
  if (program(e) || box(e)) return keysFor([{ shape: def, config }], index);
  // An assembly: its instances light everything inside them.
  return [...entities.values()].filter((x) => instance(x)?.of === def).map((x) => x.id);
}

/** The (shape, configuration, piece) a selection in the view stands for. */
export function selectionUses(entities: ReadonlyMap<string, EntityView>, selected: string | null, piece: string | null): LineUse[] {
  const e = selected ? entities.get(selected) : undefined;
  if (!e) return [];
  const shown = (shape: string) => configurations(entities.get(shape))?.active ?? undefined;
  if (box(e)) return [{ shape: e.id }];
  if (program(e) && !instance(e)) return [{ shape: e.id, config: shown(e.id), part: piece ?? undefined }];
  return occurrences(e.id, entities).map((o) => {
    const holder = o.path[o.path.length - 1];
    return { shape: o.shape, config: instance(entities.get(holder))?.config || shown(o.shape), part: piece ?? undefined };
  });
}

/** Whether a line covers any of a selection's uses (a picked piece narrows it to that piece's lines). */
export function lineMatches(uses: readonly LineUse[] | undefined, picked: readonly LineUse[], anyConfig = false): boolean {
  return (uses ?? []).some((u) =>
    picked.some((p) => p.shape === u.shape && (anyConfig || (p.config ?? "") === (u.config ?? "")) && (!p.part || !u.part || u.part === p.part || p.part.startsWith(`${u.part}/`) || u.part.startsWith(`${p.part}/`))),
  );
}
