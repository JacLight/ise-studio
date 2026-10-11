// Local command interpreter for the assistant. It turns plain requests into the
// same typed engine commands the UI uses. No language model is connected; a
// model adapter will produce the same Proposal shape.

import type { EntityView } from "./engine";
import { type Rule, type World, suggestRule } from "./intent";
import { KNOWN_MATERIALS, box, fmt, formatMass, material, materialRef, partName, placement, toMm } from "./model";

type Params = Record<string, unknown>;

export type Step = {
  operation: string;
  /** Static parameters, or built from earlier steps' outputs at apply time. */
  parameters: Params | ((outputs: Params[]) => Params);
};

export type Proposal = { label: string; steps: Step[]; selectCreated?: boolean };

export type Reply = { text: string; proposal?: Proposal; action?: "undo" | "redo" | "check" };

export type Context = { selected?: EntityView; parts: EntityView[]; materials: EntityView[]; world: World };

const UNITS: Record<string, number> = { mm: 1, millimeter: 1, millimeters: 1, cm: 10, m: 1000, meter: 1000, meters: 1000, in: 25.4, inch: 25.4, inches: 25.4, '"': 25.4, ft: 304.8 };
const NUM = String.raw`(\d+(?:\.\d+)?)`;
const UNIT = String.raw`\s*(mm|millimeters?|cm|meters?|m|inch(?:es)?|in|ft|")?`;

const mmQ = (v: number) => ({ value: v, unit: "mm" });
const toMmValue = (n: string, unit?: string) => Number(n) * (unit ? UNITS[unit] ?? 1 : 1);

export const HELP =
  "I can change the model through the engine. Try:\n" +
  "• “Add a steel box 60 x 40 x 20”\n" +
  "• “Make it 80 mm wide and 30 tall”\n" +
  "• “Double the height”\n" +
  "• “Use aluminium”\n" +
  "• “Move it 50 mm right”\n" +
  "• “Set steel density to 8000”\n" +
  "• “Keep it under 1 kg” · “Must fit within 120 x 80 x 40”\n" +
  "• “Keep 2 mm clearance” · “No collisions” · “Check”\n" +
  "• “How heavy is it?” · “Undo”";

function findMaterialWord(text: string) {
  const word = Object.keys(KNOWN_MATERIALS).find((k) => new RegExp(`\\b${k}\\b`).test(text));
  return word ? KNOWN_MATERIALS[word] : undefined;
}

function existingMaterial(ctx: Context, name: string) {
  return ctx.materials.find((m) => material(m)!.name.toLowerCase() === name.toLowerCase());
}

/** Steps that make a material available; returns how to read its id later. */
function ensureMaterial(ctx: Context, known: { name: string; density: number }, steps: Step[]): (outputs: Params[]) => string {
  const found = existingMaterial(ctx, known.name);
  if (found) return () => found.id;
  const index = steps.length;
  steps.push({ operation: "solid.material.create", parameters: { name: known.name, density: { value: known.density, unit: "kg/m3" } } });
  return (outputs) => (outputs[index] as { output: { id: string } }).output.id;
}

function triple(text: string) {
  const m = text.match(new RegExp(`${NUM}${UNIT}\\s*(?:x|×|by)\\s*${NUM}${UNIT}\\s*(?:x|×|by)\\s*${NUM}${UNIT}`));
  if (!m) return null;
  const unit = m[6] ?? m[4] ?? m[2];
  return [toMmValue(m[1], m[2] ?? unit), toMmValue(m[3], m[4] ?? unit), toMmValue(m[5], m[6] ?? unit)] as const;
}

const AXES = {
  width: { words: "wide|width|long|length", label: "width" },
  depth: { words: "deep|depth", label: "depth" },
  height: { words: "tall|high|height|thick|thickness", label: "height" },
} as const;

function dimensionEdits(text: string, current: { width: number; depth: number; height: number }) {
  const edits: Partial<Record<keyof typeof AXES, number>> = {};
  for (const [axis, { words }] of Object.entries(AXES) as [keyof typeof AXES, (typeof AXES)[keyof typeof AXES]][]) {
    const after = text.match(new RegExp(`${NUM}${UNIT}\\s*(?:${words})\\b`));
    const before = text.match(new RegExp(`\\b(?:${words})\\s*(?:to|=|of|:)?\\s*${NUM}${UNIT}`));
    const scale = text.match(new RegExp(`\\b(double|twice|triple|half|halve)\\b[^.]*?\\b(?:${words}|${axis === "width" ? "wider" : axis === "height" ? "taller" : "deeper"})\\b`))
      ?? text.match(new RegExp(`\\b(twice|half)\\s+as\\s+(?:${words})`));
    const percent = text.match(new RegExp(`${NUM}\\s*%\\s*(${axis === "width" ? "wider|narrower" : axis === "height" ? "taller|shorter" : "deeper|shallower"})`));
    if (after) edits[axis] = toMmValue(after[1], after[2]);
    else if (before) edits[axis] = toMmValue(before[1], before[2]);
    else if (scale) edits[axis] = current[axis] * ({ double: 2, twice: 2, triple: 3, half: 0.5, halve: 0.5 } as const)[scale[1] as "double"];
    else if (percent) {
      const grow = /wider|taller|deeper/.test(percent[2]);
      edits[axis] = current[axis] * (1 + (grow ? 1 : -1) * (Number(percent[1]) / 100));
    }
  }
  return edits;
}

const DIRECTIONS: Record<string, [number, number, number]> = {
  right: [1, 0, 0], left: [-1, 0, 0], forward: [0, 1, 0], back: [0, -1, 0], backward: [0, -1, 0], up: [0, 0, 1], down: [0, 0, -1],
};

function nextSlot(parts: EntityView[]) {
  return parts.reduce((max, p) => Math.max(max, toMm(placement(p)?.x) + toMm(box(p)?.width)), -20) + 20;
}

export function interpret(input: string, ctx: Context): Reply {
  const text = input.toLowerCase().trim().replace(/[’']/g, "'");
  if (!text) return { text: HELP };
  if (/^(undo|undo that|revert)\b/.test(text)) return { text: "Undoing the last change.", action: "undo" };
  if (/^redo\b/.test(text)) return { text: "Redoing it.", action: "redo" };
  if (/^(help|what can you do|\?)/.test(text)) return { text: HELP };

  const target = ctx.selected ?? (ctx.parts.length === 1 ? ctx.parts[0] : undefined);
  const known = findMaterialWord(text);

  // Check gate.
  if (/^(check|run (a |the )?check|does it pass|any (problems|issues)|is (it|everything) ok)\b/.test(text)) {
    return { text: "Running the full check…", action: "check" };
  }

  // Requirements: things the design must keep satisfying.
  const requirementReply = interpretRequirement(text, ctx);
  if (requirementReply) return requirementReply;

  // Questions about mass or volume.
  if (/\b(how heavy|mass|weigh|weight|volume)\b/.test(text) && !/\b(make|set|change)\b/.test(text)) {
    if (target?.mass) {
      return { text: `${partName(target)} has a volume of ${(target.mass.volume_m3 * 1e6).toFixed(2)} cm³ and a mass of ${formatMass(target.mass.mass_kg)}.` };
    }
    const total = ctx.parts.reduce((sum, p) => sum + (p.mass?.mass_kg ?? 0), 0);
    return { text: ctx.parts.length ? `The ${ctx.parts.length} parts weigh ${formatMass(total)} in total (parts without a material count as zero).` : "There are no parts yet." };
  }

  // Density of a material.
  const density = text.match(new RegExp(`density[^\\d]*${NUM}\\s*(kg\\/m3|kg\\/m³|g\\/cm3|g\\/cm³)?`));
  if (density) {
    const named = known ? existingMaterial(ctx, known.name) : undefined;
    const mat = named ?? (target ? ctx.materials.find((m) => m.id === materialRef(target)) : undefined);
    if (!mat) return { text: "Which material? Say for example “set steel density to 8000”." };
    const unit = density[2]?.startsWith("g") ? "g/cm3" : "kg/m3";
    return {
      text: "Here is the change. Every part using this material will update.",
      proposal: {
        label: `Set ${material(mat)!.name} density to ${density[1]} ${unit === "g/cm3" ? "g/cm³" : "kg/m³"}`,
        steps: [{ operation: "solid.material.set_density", parameters: { target: mat.id, density: { value: Number(density[1]), unit } } }],
      },
    };
  }

  // Create a part.
  if (/\b(add|create|new|make an?|make another|insert)\b/.test(text) && /\b(box|block|part|cube|plate|brick)\b/.test(text)) {
    const dims = triple(text);
    const cube = text.match(new RegExp(`cube[^\\d]*${NUM}${UNIT}`)) ?? text.match(new RegExp(`${NUM}${UNIT}\\s*cube`));
    const [w, d, h] = dims ?? (cube ? [toMmValue(cube[1], cube[2]), toMmValue(cube[1], cube[2]), toMmValue(cube[1], cube[2])] : [40, 40, 20]);
    const steps: Step[] = [];
    const materialId = known ? ensureMaterial(ctx, known, steps) : ctx.materials[0] ? () => ctx.materials[0].id : () => null;
    const x = nextSlot(ctx.parts);
    steps.push({
      operation: "solid.box.create",
      parameters: (outputs) => ({
        width: mmQ(w), depth: mmQ(d), height: mmQ(h), material: materialId(outputs),
        placement: { x: mmQ(x), y: mmQ(0), z: mmQ(0) },
      }),
    });
    const matName = known?.name ?? (ctx.materials[0] ? material(ctx.materials[0])!.name : null);
    return {
      text: "I've prepared this part. Review it, then apply.",
      proposal: { label: `Add a ${fmt(w)} × ${fmt(d)} × ${fmt(h)} mm ${matName ? matName.toLowerCase() + " " : ""}box`, steps, selectCreated: true },
    };
  }

  if (!target) {
    return { text: ctx.parts.length ? "Select a part first (click it in the view), then tell me what to change." : `There are no parts yet. Try “add a steel box 60 x 40 x 20”.` };
  }
  const b = box(target)!;
  const current = { width: toMm(b.width), depth: toMm(b.depth), height: toMm(b.height) };

  if (/\b(delete|remove)\b/.test(text)) {
    return {
      text: "This removes the part. You can undo it.",
      proposal: { label: `Delete ${partName(target)}`, steps: [{ operation: "core.entity.delete", parameters: { target: target.id } }] },
    };
  }

  // Move.
  if (/\bmove|shift|nudge\b/.test(text)) {
    const p = placement(target);
    const pos = [toMm(p?.x), toMm(p?.y), toMm(p?.z)];
    let changed = false;
    for (const [i, axis] of ["x", "y", "z"].entries()) {
      const m = text.match(new RegExp(`\\b${axis}\\s*(?:to|=|:)?\\s*(-?\\d+(?:\\.\\d+)?)${UNIT}`));
      if (m) {
        pos[i] = toMmValue(m[1], m[2]);
        changed = true;
      }
    }
    const rel = text.match(new RegExp(`${NUM}${UNIT}\\s*(?:to the\\s+)?(right|left|forward|backward|back|up|down)`));
    if (rel) {
      const delta = toMmValue(rel[1], rel[2]);
      DIRECTIONS[rel[3]].forEach((s, i) => (pos[i] += s * delta));
      changed = true;
    }
    if (changed) {
      return {
        text: "Here is the new position.",
        proposal: {
          label: `Move ${partName(target)} to ${pos.map(fmt).join(", ")} mm`,
          steps: [{ operation: "solid.part.move", parameters: { target: target.id, placement: { x: mmQ(pos[0]), y: mmQ(pos[1]), z: mmQ(pos[2]) } } }],
        },
      };
    }
  }

  // Dimensions.
  const all = triple(text);
  const edits = all ? { width: all[0], depth: all[1], height: all[2] } : dimensionEdits(text, current);
  if (Object.keys(edits).length) {
    const parts = Object.entries(edits).map(([k, v]) => `${k} ${fmt(current[k as keyof typeof current])} → ${fmt(v!)} mm`);
    return {
      text: "I've prepared this change. Review it, then apply it to the model.",
      proposal: {
        label: `Set ${parts.join(", ")}`,
        steps: [{ operation: "solid.box.set_dimensions", parameters: { target: target.id, ...Object.fromEntries(Object.entries(edits).map(([k, v]) => [k, mmQ(v!)])) } }],
      },
    };
  }

  // Material.
  if (known) {
    const steps: Step[] = [];
    const id = ensureMaterial(ctx, known, steps);
    steps.push({ operation: "solid.material.assign", parameters: (outputs) => ({ target: target.id, material: id(outputs) }) });
    return {
      text: "Here is the material change. Mass updates when you apply it.",
      proposal: { label: `Use ${known.name.toLowerCase()} for ${partName(target)}`, steps },
    };
  }
  if (/\b(no material|remove (the )?material|clear (the )?material)\b/.test(text)) {
    return {
      text: "The part will have no material.",
      proposal: { label: `Clear the material of ${partName(target)}`, steps: [{ operation: "solid.material.assign", parameters: { target: target.id, material: null } }] },
    };
  }

  return { text: `I didn't catch a change I can make yet.\n\n${HELP}` };
}

const MASS_UNITS: Record<string, number> = { g: 0.001, gram: 0.001, grams: 0.001, kg: 1, kilo: 1, kilos: 1, kilogram: 1, kilograms: 1, lb: 0.45359237, lbs: 0.45359237, pound: 0.45359237, pounds: 0.45359237 };

function proposeRequirement(rule: Rule, label: string, note: string): Reply {
  return {
    text: `${note} The engine will re-check it on every change from now on.`,
    proposal: { label, steps: [{ operation: "intent.requirement.create", parameters: { rule } }] },
  };
}

function interpretRequirement(text: string, ctx: Context): Reply | null {
  const everything = /\b(everything|all( parts)?|total|assembly|whole thing)\b/.test(text);
  const scope = () => (ctx.selected && !everything ? [ctx.selected.id] : ctx.parts.map((p) => p.id));

  const mass = text.match(new RegExp(`\\b(?:under|below|less than|at most|max(?:imum)?|no more than|lighter than|up to)\\s*${NUM}\\s*(kg|kilos?|kilograms?|g|grams?|lbs?|pounds?)\\b`));
  if (mass && /\b(keep|must|should|make sure|weigh|weight|mass|limit|stay|requirement)\b/.test(text)) {
    const parts = scope();
    if (!parts.length) return { text: "Add a part first, then give it a mass limit." };
    const kg = Number(mass[1]) * MASS_UNITS[mass[2]];
    return proposeRequirement(
      { kind: "max_mass", parts, limit: { value: kg, unit: "kg" } },
      `Require ${parts.length > 1 ? "total mass" : "mass"} ≤ ${formatMass(kg)}`,
      "Here's the mass requirement.",
    );
  }

  if (/\bfits?\b|\benvelope\b/.test(text)) {
    const dims = triple(text);
    if (dims) {
      const parts = scope();
      if (!parts.length) return { text: "Add a part first, then give it an envelope." };
      return proposeRequirement(
        { kind: "fits_within", parts, width: mmQ(dims[0]), depth: mmQ(dims[1]), height: mmQ(dims[2]) },
        `Require ${parts.length > 1 ? "everything" : "the part"} to fit within ${dims.map(fmt).join(" × ")} mm`,
        "Here's the envelope.",
      );
    }
  }

  const gap = text.match(new RegExp(`${NUM}${UNIT}\\s*(?:of\\s+)?(?:clearance|gap|space|apart)`)) ?? text.match(new RegExp(`(?:clearance|gap)\\s*(?:of|=|:)?\\s*${NUM}${UNIT}`));
  if (gap) {
    const suggestion = suggestRule("clearance", ctx.parts, ctx.selected?.id ?? null, ctx.world);
    if (typeof suggestion === "string") return { text: suggestion };
    const min = toMmValue(gap[1], gap[2]);
    const rule = { ...suggestion, min: mmQ(min) } as Rule;
    const names = rule.kind === "clearance" ? [rule.a, rule.b].map((id) => partName(ctx.parts.find((p) => p.id === id)!)) : [];
    return proposeRequirement(rule, `Keep ${fmt(min)} mm between ${names.join(" and ")}`, "Here's the clearance requirement, between the selected part and its nearest neighbour.");
  }

  if (/\b(no (collisions?|overlaps?|interference)|nothing (overlaps|collides)|don'?t (overlap|collide)|must not (overlap|collide))\b/.test(text)) {
    if (ctx.parts.length < 2) return { text: "Collision checks need at least two parts." };
    return proposeRequirement({ kind: "no_collisions", parts: ctx.parts.map((p) => p.id) }, `Require that none of the ${ctx.parts.length} parts overlap`, "Here's the collision requirement.");
  }
  return null;
}
