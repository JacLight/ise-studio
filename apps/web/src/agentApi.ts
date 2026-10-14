// `ise`: the commands the Iṣẹ́ AI runs, as a JavaScript object. The console gives it to what people type, and
// the page sets it on `window.ise` so an assistant working in the browser (Claude in Chrome and the like) can
// change the design with one call through its page-script tool instead of clicking the interface.
// Everything goes through the engine: saved, synced, undoable, shown live, like any other change.
//
//   ise.help()                 what this object does, and where the full guide is
//   ise.docs(topic?)           the Iṣẹ́ Studio guide (generated from the code): the overview, or a topic's page
//   ise.ops(filter?)           the engine's operations with their parameters' JSON schemas
//   ise.tree()                 what the project holds, as { id, name, kind } rows
//   ise.get(idOrName) · ise.find(name) · ise.entities
//   ise.commit(op, params)     one change (one undo step) · ise.run(steps) several, "$0.id" naming step 0's output
//   ise.check() · ise.view(action, what?, value?) · ise.undo() · ise.redo() · ise.fetch(path)

import { type Engine, type EntityView, projectUrl } from "./engine";

/** An item's own name, whatever kind it is. */
export const nameOf = (e: EntityView) => {
  const c = e.components as Record<string, { name?: string } | undefined>;
  return c["assembly.instance"]?.name ?? c["shape.program"]?.name ?? c["assembly.definition"]?.name ?? c["shape.parameter_set"]?.name ?? c["shape.sheet"]?.name ?? c["draft.object"]?.name ?? c["shape.sketch"]?.name ?? c["draft.layer"]?.name ?? c["draft.plane"]?.name;
};

/** What kind of item it is, in a few words. */
export function kindOf(e: EntityView) {
  const c = e.components as Record<string, Record<string, unknown> | undefined>;
  if (c["shape.program"]) return `shape “${c["shape.program"].name}”`;
  if (c["assembly.instance"]) return `instance ${c["assembly.instance"].name}`;
  if (c["assembly.definition"]) return `assembly “${c["assembly.definition"].name}”`;
  if (c["draft.array"]) return `array “${c["shape.parameter_set"]?.name}”`;
  if (c["shape.parameter_set"]) return `parameters “${c["shape.parameter_set"].name}”`;
  if (c["shape.sheet"]) return `sheet “${c["shape.sheet"].name}”`;
  if (c["shape.sketch"]) return `sketch “${c["shape.sketch"].name}”`;
  if (c["draft.object"]) return `draft ${String(c["draft.object"].kind)} “${String(c["draft.object"].name)}”`;
  if (c["draft.layer"]) return `layer “${String(c["draft.layer"].name)}”`;
  if (c["draft.plane"]) return `working plane “${String(c["draft.plane"].name)}”`;
  if (c["solid.material"]) return `material ${String(c["solid.material"].name)}`;
  if (c["intent.requirement"]) return `requirement “${String(c["intent.requirement"].name)}”`;
  if (c["solid.box"]) return "box";
  return Object.keys(c)[0] ?? "entity";
}

const HELP = `Iṣẹ́ Studio: \`ise\` changes the open project through its engine (saved, synced, undoable, shown live).

  ise.docs()                 the guide: how Iṣẹ́ Studio works and how to model well (start here)
  ise.docs("sketches")       a topic's page (e.g. "shape-programs", "components", "pcb", "operations/shape", "cli")
  ise.ops("shape")           operations whose name has "shape", with their parameters' JSON schemas
  ise.tree()                 what the project holds: [{ id, name, kind }]
  ise.get("Bracket")         one item in full (by id or name)
  ise.commit("shape.update", { target, source })   one change, one undo step
  ise.run([{ operation, parameters }, …])          several, one after another ("$0.id" is step 0's output id)
  ise.check()                every requirement and geometry check
  ise.view("isolate", "Bracket")   show the person something: select, isolate, frame, hide, show_all, camera, explode…
  ise.undo() · ise.redo()

The full guide is also at /agent/ on this page's address (every operation, the command line, the HTTP API).`;

export type AgentApi = ReturnType<typeof agentApi>;

export function agentApi(engine: Engine, onStep?: (i: number, operation: string, output: unknown) => void) {
  const entities = () => [...engine.getState().entities.values()];
  const find = (what: string) => {
    const all = entities();
    const byId = all.find((e) => e.id === what);
    if (byId) return byId;
    const named = all.filter((e) => nameOf(e)?.toLowerCase() === what.toLowerCase());
    const placed = named.filter((e) => e.components["assembly.instance"] || e.components["solid.placement"]);
    const pick = placed.length ? placed : named;
    if (pick.length > 1) throw new Error(`“${what}” matches ${pick.length} things: use an id (${pick.map((e) => e.id).join(", ")})`);
    if (!pick.length) throw new Error(`nothing called “${what}”`);
    return pick[0];
  };
  const fetchPath = async (path: string) => {
    const res = await fetch(projectUrl(path));
    if (!res.ok) throw new Error(await res.text());
    return res.headers.get("content-type")?.includes("json") ? res.json() : res.text();
  };
  const commit = async (operation: string, parameters: Record<string, unknown> = {}) => {
    const r = (await engine.commit(operation, parameters)) as { output?: unknown; revision?: number };
    return r?.output ?? r;
  };
  const run = async (steps: { operation: string; parameters?: Record<string, unknown> }[]) => {
    const outputs: unknown[] = [];
    const resolve = (v: unknown): unknown => {
      if (typeof v === "string") {
        const m = /^\$(\d+)\.(.+)$/.exec(v);
        if (!m) return v;
        let x = outputs[Number(m[1])] as Record<string, unknown> | undefined;
        for (const k of m[2].split(".")) x = x?.[k] as Record<string, unknown> | undefined;
        if (x === undefined) throw new Error(`${v} does not resolve`);
        return x;
      }
      if (Array.isArray(v)) return v.map(resolve);
      if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, resolve(x)]));
      return v;
    };
    for (const [i, s] of steps.entries()) {
      const o = await commit(s.operation, resolve(s.parameters ?? {}) as Record<string, unknown>);
      outputs.push(o);
      onStep?.(i, s.operation, o);
    }
    return outputs;
  };
  const VALUED = ["color", "camera", "projection", "display", "section"];
  const view = (action: string, what?: string, value?: string) => {
    if (VALUED.includes(action)) return engine.emitView({ action, target: null, value: what ?? value ?? null });
    // measure takes two items; explode a spread (0.25–2.5, or "off"); the rest one item or none.
    return engine.emitView({ action, target: what ? find(what).id : null, value: action === "measure" && value ? find(value).id : (value ?? (action === "explode" ? "1" : null)) });
  };
  return {
    get entities() {
      return entities();
    },
    get: (what: string) => find(what),
    find: (name: string) => entities().filter((e) => nameOf(e)?.toLowerCase().includes(name.toLowerCase())),
    tree: () => entities().map((e) => ({ id: e.id, name: nameOf(e) ?? null, kind: kindOf(e) })),
    ops: (filter?: string) => engine.commands.filter((c) => !filter || c.name.includes(filter)).map((c) => ({ name: c.name, schema: c.schema })),
    commit,
    run,
    check: () => engine.check(),
    view,
    fetch: fetchPath,
    undo: () => engine.undo(),
    redo: () => engine.redo(),
    help: () => HELP,
    /** The generated guide (skills/ise-studio, served at /agent/): the overview, or the page whose name ends with or contains the topic. */
    docs: async (topic?: string) => {
      const index = (await (await fetch("/agent/files.json")).json()) as string[];
      if (!topic) return (await fetch("/agent/SKILL.md")).text();
      const t = topic.toLowerCase().replace(/[^a-z0-9/]+/g, "-");
      const hit = index.find((f) => f.toLowerCase().replace(/\.md$/, "").endsWith(t)) ?? index.find((f) => f.toLowerCase().includes(t));
      if (!hit) return `No page for “${topic}”. Pages:\n${index.map((f) => `- ${f.replace(/\.md$/, "")}`).join("\n")}`;
      return (await fetch(`/agent/${hit}`)).text();
    },
  };
}
