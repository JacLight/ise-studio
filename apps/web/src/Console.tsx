// The console: the same commands Forma AI runs with ./forma (list, tree,
// get, commit, run, check, sheet, draft, measure, bom, export, view…),
// typed here against the open project, and JavaScript with a `forma`
// object for people who script. Everything goes through the engine, so it
// is saved, synced and undoable like any other change.

import { useEffect, useRef, useState } from "react";
import { type Engine, type EntityView, projectName, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import "./data.css";

type Line = { kind: "in" | "out" | "err" | "note"; text: string };

const HISTORY_KEY = "forma.console.history";

const HELP = `Commands (as ./forma takes them; "./forma" in front is fine):
  list · tree · get <id|name> · ops [filter] · check
  commit <operation> <json>      e.g. commit sheet.set {"target":"…","cells":{"B2":"=B1*2"}}
  run <json steps>               [{"operation":…,"parameters":…}] — "$0.id" refers to step 0's output
  sheet [name] · draft · measure <a> [b] · bom <id|project> [qty] · fit <id|project>
  export <id|name> <step|stl|3mf|dxf|scad|zip|csv|xlsx> · bundle
  view <select|frame|isolate|hide|show|show_all|camera|display|projection|section|explode|measure> [what]
  undo · redo · clear · help
Anything else runs as JavaScript, with:
  forma.entities · forma.get(idOrName) · forma.find(name) · forma.commit(op, params)
  forma.run(steps) · forma.check() · forma.view(action, what?, value?) · forma.fetch(path)
  print(…)  — e.g.  for (const e of forma.entities) if (e.draft?.value) print(e.id, e.draft.value)`;

const nameOf = (e: EntityView) => {
  const c = e.components as Record<string, { name?: string } | undefined>;
  return c["assembly.instance"]?.name ?? c["shape.program"]?.name ?? c["assembly.definition"]?.name ?? c["shape.parameter_set"]?.name ?? c["shape.sheet"]?.name ?? c["draft.object"]?.name ?? c["shape.sketch"]?.name ?? c["draft.layer"]?.name ?? c["draft.plane"]?.name;
};

function kindOf(e: EntityView) {
  const c = e.components as Record<string, Record<string, unknown> | undefined>;
  if (c["shape.program"]) return `shape “${c["shape.program"].name}”`;
  if (c["assembly.instance"]) return `instance ${c["assembly.instance"].name}`;
  if (c["assembly.definition"]) return `assembly “${c["assembly.definition"].name}”`;
  if (c["draft.array"]) return `array “${c["shape.parameter_set"]?.name}”`;
  if (c["shape.parameter_set"]) return `parameters “${c["shape.parameter_set"].name}”`;
  if (c["shape.sheet"]) return `sheet “${c["shape.sheet"].name}”`;
  if (c["shape.sketch"]) return `sketch “${c["shape.sketch"].name}”`;
  if (c["draft.object"]) return `draft ${c["draft.object"].kind} “${c["draft.object"].name}”`;
  if (c["draft.layer"]) return `layer “${c["draft.layer"].name}”`;
  if (c["draft.plane"]) return `working plane “${c["draft.plane"].name}”`;
  if (c["solid.material"]) return `material ${c["solid.material"].name}`;
  if (c["intent.requirement"]) return `requirement “${c["intent.requirement"].name}”`;
  if (c["solid.box"]) return "box";
  return Object.keys(c)[0] ?? "entity";
}

/** Splits "commit op {json}" into words, keeping the JSON (or quoted text) whole. */
function words(line: string): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < line.length) {
    if (line[i] === " ") {
      i++;
      continue;
    }
    if (line[i] === "{" || line[i] === "[") {
      out.push(line.slice(i).trim());
      break;
    }
    if (line[i] === '"' || line[i] === "'") {
      const q = line[i];
      const end = line.indexOf(q, i + 1);
      out.push(line.slice(i + 1, end < 0 ? undefined : end));
      i = end < 0 ? line.length : end + 1;
      continue;
    }
    const end = line.indexOf(" ", i);
    out.push(line.slice(i, end < 0 ? undefined : end));
    i = end < 0 ? line.length : end;
  }
  return out;
}

const COMMANDS = new Set(["help", "list", "tree", "get", "ops", "check", "commit", "run", "sheet", "draft", "measure", "bom", "fit", "export", "bundle", "view", "undo", "redo", "clear"]);

export function Console(props: { engine: Engine; onClose: () => void }) {
  const { engine } = props;
  const [lines, setLines] = useState<Line[]>([{ kind: "note", text: "Forma console · the commands Forma AI uses, and JavaScript. Type help." }]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const history = useRef<string[]>([]);
  const at = useRef(-1);
  const out = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    try {
      history.current = JSON.parse(localStorage.getItem(HISTORY_KEY) ?? "[]");
    } catch {
      history.current = [];
    }
    field.current?.focus();
  }, []);
  useEffect(() => {
    out.current?.scrollTo({ top: out.current.scrollHeight });
  }, [lines]);

  const print = (kind: Line["kind"], text: string) => setLines((l) => [...l.slice(-800), { kind, text }]);
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
  const getJson = async (path: string) => {
    const res = await fetch(projectUrl(path));
    if (!res.ok) throw new Error(await res.text());
    return res.headers.get("content-type")?.includes("json") ? res.json() : res.text();
  };
  const save = (url: string) => {
    const a = document.createElement("a");
    a.href = url;
    a.download = "";
    document.body.appendChild(a);
    a.click();
    a.remove();
  };
  const commit = async (operation: string, parameters: Record<string, unknown>) => {
    const r = (await engine.commit(operation, parameters)) as { output?: unknown; revision?: number };
    return r?.output ?? r;
  };
  const runSteps = async (steps: { operation: string; parameters?: Record<string, unknown> }[]) => {
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
      print("out", `✓ ${i} ${s.operation} → ${JSON.stringify(o)}`);
    }
    return outputs;
  };
  const view = (action: string, target: string | null = null, value: string | null = null) => engine.emitView({ action, target, value });

  const api = {
    get entities() {
      return entities();
    },
    get: (what: string) => find(what),
    find: (name: string) => entities().filter((e) => nameOf(e)?.toLowerCase().includes(name.toLowerCase())),
    commit,
    run: runSteps,
    check: () => engine.check(),
    view: (action: string, what?: string, value?: string) => view(action, what ? find(what).id : null, value ?? null),
    fetch: getJson,
    undo: () => engine.undo(),
    redo: () => engine.redo(),
  };

  async function command(w: string[]) {
    const [cmd, ...rest] = w;
    switch (cmd) {
      case "help":
        print("note", HELP);
        return;
      case "clear":
        setLines([]);
        return;
      case "undo":
        await engine.undo();
        print("out", "✓ undone");
        return;
      case "redo":
        await engine.redo();
        print("out", "✓ redone");
        return;
      case "list":
        for (const e of entities()) {
          const extra = e.result ? ` — ${e.result.passed ? "met" : "FAILING"}: ${e.result.summary}` : e.draft?.label && e.draft.value != null ? ` — ${e.draft.label[1]}` : e.sheet ? ` — ${Object.keys(e.sheet.cells).length} cells` : e.mass ? ` — ${(e.mass.volume_m3 * 1e9).toFixed(1)} mm³` : "";
          print("out", `${e.id}  ${kindOf(e)}${extra}`);
        }
        return;
      case "tree": {
        const all = entities();
        const inst = (e: EntityView) => e.components["assembly.instance"] as { name: string; of: string; parent?: string | null; at: number[] } | undefined;
        const walk = (e: EntityView, depth: number) => {
          const i = inst(e);
          print("out", `${"  ".repeat(depth)}${i ? `${i.name}  [${nameOf(all.find((d) => d.id === i.of)!) ?? "?"}] at ${i.at.map((v) => Math.round(v)).join(",")}` : nameOf(e) ?? kindOf(e)}  (${e.id})`);
          if (!i) return;
          for (const c of all.filter((x) => inst(x)?.parent === i.of)) walk(c, depth + 1);
        };
        for (const e of all) {
          const i = inst(e);
          if (i ? !i.parent : (e.components["shape.program"] && e.components["solid.placement"]) || e.components["solid.box"]) walk(e, 0);
        }
        return;
      }
      case "get": {
        const e = find(rest.join(" "));
        const { mesh, configured, ...shown } = e;
        print("out", JSON.stringify({ ...shown, mesh: mesh ? { triangles: mesh.indices.length / 3 } : null, configured: configured ? { ...configured, mesh: undefined } : undefined }, null, 2));
        return;
      }
      case "ops":
        for (const c of engine.commands.filter((c) => !rest[0] || c.name.includes(rest[0]))) print("out", `${c.name}\n  ${JSON.stringify(c.schema)}`);
        return;
      case "check": {
        const r = await engine.check();
        print(r.ok ? "out" : "err", `${r.ok ? "✓" : "✗"} ${r.requirements_passing}/${r.requirements_total} requirements met, revision ${r.revision}`);
        for (const f of r.findings) print("out", `  ${f.severity.padEnd(7)} ${f.code}: ${f.message}`);
        return;
      }
      case "commit": {
        const [op, json] = rest;
        if (!op) throw new Error("commit <operation> <json>");
        const o = await commit(op, json ? JSON.parse(json) : {});
        print("out", `✓ ${op} → ${JSON.stringify(o)}`);
        return;
      }
      case "run":
        await runSteps(JSON.parse(rest.join(" ")));
        return;
      case "sheet": {
        const sheets = entities().filter((e) => e.components["shape.sheet"]);
        const want = rest.join(" ");
        const e = sheets.find((x) => x.id === want || nameOf(x)?.toLowerCase() === want.toLowerCase()) ?? (sheets.length === 1 && !want ? sheets[0] : undefined);
        if (!e) throw new Error(`sheets: ${sheets.map(nameOf).join(", ") || "none"}`);
        const cells = e.sheet?.cells ?? {};
        const rows = new Map<number, string[]>();
        for (const [a, v] of Object.entries(cells)) {
          const m = /^([A-Z]+)(\d+)$/.exec(a)!;
          const r = Number(m[2]);
          if (!rows.has(r)) rows.set(r, []);
          rows.get(r)!.push(`${a} ${v.shown}${v.formula ? ` (${v.formula})` : ""}`);
        }
        print("out", `${nameOf(e)} (${e.id})`);
        for (const r of [...rows.keys()].sort((a, b) => a - b)) print("out", `  ${rows.get(r)!.join(" │ ")}`);
        const al = Object.entries(e.sheet?.aliases ?? {});
        if (al.length) print("out", `  aliases: ${al.map(([k, v]) => `${k}=${v}`).join(", ")}`);
        return;
      }
      case "draft":
        for (const e of entities()) {
          const o = e.components["draft.object"] as { kind: string; name: string } | undefined;
          if (o) print("out", `${e.id}  ${o.kind} “${o.name}”${e.draft?.value != null ? ` = ${e.draft.label?.[1]}` : e.draft ? ` length ${+e.draft.length.toFixed(3)} mm${e.draft.area != null ? `, area ${+e.draft.area.toFixed(3)} mm²` : ""}` : ""}`);
        }
        return;
      case "measure": {
        const [a, b] = rest;
        for (const w of [a, b].filter(Boolean)) {
          const id = find(w).id;
          const m = (await getJson(`/measure/${encodeURIComponent(id)}`)) as { volume_mm3: number; area_mm2: number; mass_kg: number | null; center_of_mass: number[] };
          print("out", `${w}: volume ${m.volume_mm3.toFixed(2)} mm³, area ${m.area_mm2.toFixed(2)} mm², mass ${m.mass_kg == null ? "—" : `${m.mass_kg.toFixed(6)} kg`}, centre ${m.center_of_mass.map((v) => v.toFixed(3)).join(", ")}`);
        }
        return;
      }
      case "bom": {
        const [w, qty] = rest;
        const id = !w || w === "project" ? "project" : find(w).id;
        print("out", String(await getJson(`/bom/${id}?format=csv${qty ? `&qty=${qty}` : ""}`)));
        return;
      }
      case "fit": {
        const w = rest[0];
        const id = !w || w === "project" ? "project" : find(w).id;
        const r = (await getJson(`/fit/${id}?motion=off`)) as { ok: boolean; name: string; pass: number; warn: number; fail: number };
        print(r.ok ? "out" : "err", `${r.ok ? "✓" : "✗"} ${r.name}: ${r.pass} fit, ${r.warn} with warnings, ${r.fail} failing`);
        return;
      }
      case "export": {
        const [w, ext] = rest;
        if (!w || !ext) throw new Error("export <id|name> <step|stl|3mf|dxf|scad|zip|csv|xlsx>");
        const e = find(w);
        const x = ext.replace(/^\./, "").toLowerCase();
        save(projectUrl(`/${x === "csv" || x === "xlsx" ? "table" : "export"}/${e.id}.${x}`));
        print("out", `✓ downloading ${nameOf(e)}.${x}`);
        return;
      }
      case "bundle":
        save(`/projects/${encodeURIComponent(projectName || "default")}/bundle`);
        print("out", "✓ downloading the project file (.forma): its history and assets come along");
        return;
      case "view": {
        const [action, ...what] = rest;
        const arg = what.join(" ");
        const valued = ["color", "camera", "projection", "display", "section"].includes(action);
        if (valued) view(action, null, arg);
        else if (action === "measure") {
          const parts = what.map((x) => find(x).id);
          view("measure", parts[0] ?? null, parts[1] ?? null);
        } else if (action === "explode") {
          const m = /\s+(off|\d*\.?\d+)$/.exec(arg);
          view("explode", find(m ? arg.slice(0, m.index) : arg).id, m ? m[1] : "1");
        } else view(action, arg ? find(arg).id : null, null);
        print("out", `✓ ${action}${arg ? ` ${arg}` : ""}`);
        return;
      }
    }
  }

  async function submit() {
    const text = input.trim();
    if (!text || busy) return;
    history.current = [...history.current.filter((h) => h !== text), text].slice(-200);
    try {
      localStorage.setItem(HISTORY_KEY, JSON.stringify(history.current));
    } catch {
      // keep going without storage
    }
    at.current = -1;
    setInput("");
    print("in", text);
    setBusy(true);
    try {
      const line = text.replace(/^\.?\/?forma(\.mjs)?\s+/, "");
      const w = words(line);
      if (COMMANDS.has(w[0]) && !/^[a-z]+\s*[=(.]/.test(line)) await command(w);
      else {
        const show = (...a: unknown[]) => print("out", a.map((x) => (typeof x === "string" ? x : JSON.stringify(x, null, 2))).join(" "));
        const js = line.replace(/^js\s+/, "");
        // An expression shows its value; statements run as they are.
        let fn: (forma: typeof api, print: typeof show) => Promise<unknown>;
        try {
          fn = new Function("forma", "print", `return (async () => (${js}))()`) as typeof fn;
        } catch {
          fn = new Function("forma", "print", `return (async () => { ${js} })()`) as typeof fn;
        }
        const v = await fn(api, show);
        if (v !== undefined) show(v);
      }
    } catch (e) {
      const err = e as { message?: string; code?: string };
      print("err", `✗ ${err.message ?? String(e)}`);
    } finally {
      setBusy(false);
      requestAnimationFrame(() => field.current?.focus());
    }
  }

  return (
    <section className="console floating" aria-label="Console">
      <header className="console-head">
        <Icon name="terminal" />
        <strong>Console</strong>
        <span className="muted small">./forma commands and JavaScript · ↑ history · ` to close</span>
        <IconButton icon="close" label="Close console" onClick={props.onClose} />
      </header>
      <div className="console-out" ref={out} role="log" aria-live="polite">
        {lines.map((l, i) => (
          <pre key={i} className={`console-line ${l.kind}`}>
            {l.kind === "in" ? `› ${l.text}` : l.text}
          </pre>
        ))}
        {busy && <pre className="console-line note">…</pre>}
      </div>
      <div className="console-in">
        <span aria-hidden>›</span>
        <textarea
          ref={field}
          rows={1}
          value={input}
          spellCheck={false}
          aria-label="Console input"
          placeholder="list · get Bracket · commit sheet.create {…} · forma.entities.length"
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void submit();
            } else if (e.key === "Escape" || (e.key === "`" && !input)) {
              e.preventDefault();
              props.onClose();
            } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !input.includes("\n")) {
              const h = history.current;
              if (!h.length) return;
              e.preventDefault();
              const next = e.key === "ArrowUp" ? (at.current < 0 ? h.length - 1 : Math.max(0, at.current - 1)) : at.current < 0 ? -1 : at.current + 1;
              at.current = next >= h.length ? -1 : next;
              setInput(at.current < 0 ? "" : h[at.current]);
            }
          }}
        />
      </div>
    </section>
  );
}
