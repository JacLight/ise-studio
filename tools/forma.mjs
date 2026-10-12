#!/usr/bin/env node
// Talk to a running Forma engine from the command line, through the same API
// the UI and the AI use.
//
//   node tools/forma.mjs run steps.json [--project name] [--url ws://127.0.0.1:7420/ws]
//   node tools/forma.mjs check
//   node tools/forma.mjs gears <id> [torque N·m] [rpm]   turn a gear train through a full turn; tooth strength with a load
//   node tools/forma.mjs motion <joint id> [steps] [--set '{"Elbow":40}']   sweep a joint through its limits: what moves, closest approaches, clashes, where it binds
//   node tools/forma.mjs fit <assembly or instance id | project> [--motion]   do screws, inserts, nuts, bearings and joined parts fit (and, with --motion, every joint's sweep)
//   node tools/forma.mjs bom <assembly id | project> [qty]   the bill of materials as CSV: what is made (and how), what is bought, totals
//   node tools/forma.mjs stock [search]       the stock library: families, or the sizes matching (M3×12, 608, servo)
//   node tools/forma.mjs pcb parts|add|footprint|verify-footprints|gerber-check|library …   KiCad library parts, footprint and Gerber checks
//   node tools/forma.mjs pcb route|drag|cleanup|optimize …   the interactive router: pad to pad (walkaround/shove), drag a track or via, tidy copper
//   node tools/forma.mjs simulate <schematic> [--tran 30ms] [--expect "min(V(3V3), burst) > 3.0"] [--save name]   ngspice on a schematic
//   node tools/forma.mjs list                 every entity, one line each
//   node tools/forma.mjs tree                 what is placed, as a tree (instances, parts)
//   node tools/forma.mjs get <id>             one entity in full (program source, params, results)
//   node tools/forma.mjs commit <operation> '<json>'   one command (same $file inlining)
//   node tools/forma.mjs image add <file> …   store pictures (PNG, JPEG) or fonts (TTF, OTF) for programs; prints each id
//   node tools/forma.mjs image list           the pictures and fonts in the project
//   node tools/forma.mjs look [what] [--view iso|front|top|…|az,el] [--size 1000x750] [--out file.png]
//        a picture of the model (or one thing in it) drawn by the engine, saved to look/…png
//   node tools/forma.mjs ops [filter]         the commands the engine accepts, with schemas
//   node tools/forma.mjs view <action> [what] show something in the person's view (not saved):
//        select L26 · frame L26 · isolate L26 · show_all · hide L19 · show L19
//        color status|material · camera iso|front|back|left|right|top|bottom
//        explode Bracket [spread] · explode Bracket off
//        projection ortho|perspective · display shaded|edges|wireframe|hidden|xray
//        section x 25 [flip] · section box x0 y0 z0 x1 y1 z1 · section off
//        measure Bracket [Plate]   (part, or part to part, measured in their view)
//   node tools/forma.mjs measure <part> [other part] [--part name]   volume, area, mass, centre of mass,
//   node tools/forma.mjs drawing <id> [--sheet A3] [--part Name] [--page N] [--pdf f] [--svg f] [--dxf f] [--zip f]   the engineering drawing: sheets, dimensions, the checker's findings; saves files
//        moments of inertia (kg·mm² about the centre of mass); with two: centre to centre and the gap per axis
//
// steps.json is an array of { "operation": "...", "parameters": { ... } }.
// Any string "$N.path" in parameters is replaced by step N's output at that
// path (e.g. "$0.id"), and { "$file": "path.js" } inlines a file's text (paths
// are relative to steps.json; several files can be joined with an array).

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const args = process.argv.slice(2);
const urlFlag = args.indexOf("--url");
const base = urlFlag >= 0 ? args.splice(urlFlag, 2)[1] : "ws://127.0.0.1:7420/ws?user=forma-ai";
const projectFlag = args.indexOf("--project");
const project = projectFlag >= 0 ? args.splice(projectFlag, 2)[1] : null;
const url = project ? `${base}${base.includes("?") ? "&" : "?"}project=${encodeURIComponent(project)}` : base;
const [command, file] = args;

function connect() {
  return new Promise((resolveConn, reject) => {
    const ws = new WebSocket(url);
    const waiters = new Map();
    let revision = 0;
    let request = 1;
    let ready;
    const snapshot = new Promise((r) => (ready = r));
    ws.onerror = () => reject(new Error(`cannot reach the engine at ${url}`));
    ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.type === "snapshot") {
        revision = m.revision;
        ready(m);
      } else if (m.type === "changed") {
        revision = m.revision;
      } else if (m.type === "result") {
        // A commit's result carries the new revision; the "changed" broadcast may come after it.
        if (m.ok && typeof m.value?.revision === "number") revision = Math.max(revision, m.value.revision);
        waiters.get(m.request_id)?.(m);
      }
    };
    ws.onopen = () =>
      resolveConn({
        snapshot,
        send(type, extra = {}) {
          const id = request++;
          return new Promise((done) => {
            waiters.set(id, done);
            ws.send(JSON.stringify({ type, request_id: id, ...extra }));
          });
        },
        commit(operation, parameters) {
          return this.send("commit", { operation, parameters, expected_revision: revision });
        },
        close: () => ws.close(),
      });
  });
}

function resolveRefs(value, outputs, base) {
  if (typeof value === "string") {
    const m = value.match(/^\$(\d+)\.(.+)$/);
    if (!m) return value;
    let v = outputs[Number(m[1])];
    for (const key of m[2].split(".")) v = v?.[key];
    if (v === undefined) throw new Error(`${value} does not resolve`);
    return v;
  }
  if (Array.isArray(value)) return value.map((v) => resolveRefs(v, outputs, base));
  if (value && typeof value === "object") {
    if ("$file" in value) {
      const files = Array.isArray(value.$file) ? value.$file : [value.$file];
      return files.map((f) => readFileSync(resolve(base, f), "utf8")).join("\n");
    }
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolveRefs(v, outputs, base)]));
  }
  return value;
}

const engine = await connect();
await engine.snapshot;
let failed = false;

if (command === "run") {
  const steps = JSON.parse(readFileSync(file, "utf8"));
  const base = dirname(resolve(file));
  const outputs = [];
  for (const [i, step] of steps.entries()) {
    const parameters = resolveRefs(step.parameters ?? {}, outputs, base);
    const started = Date.now();
    const r = await engine.commit(step.operation, parameters);
    if (!r.ok) {
      console.error(`✗ ${i} ${step.operation}: ${r.error.code} ${r.error.message}`);
      failed = true;
      break;
    }
    outputs.push(r.value.output);
    console.log(`✓ ${i} ${step.operation} → ${JSON.stringify(r.value.output)} (rev ${r.value.revision}, ${Date.now() - started} ms)`);
  }
} else if (command === "check") {
  const r = await engine.send("check");
  const report = r.value;
  console.log(`${report.ok ? "✓" : "✗"} ${report.requirements_passing}/${report.requirements_total} requirements met, revision ${report.revision}`);
  for (const f of report.findings) console.log(`  ${f.severity.padEnd(7)} ${f.code}: ${f.message}${f.fix ? `\n          fix: ${f.fix.label}` : ""}`);
  failed = !report.ok;
} else if (command === "gears") {
  // Turns the shape's gear train through a full turn; with a load, tooth strength too.
  const [, id, torque, rpm] = args;
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = new URLSearchParams({ ...(project ? { project } : {}), ...(torque ? { torque } : {}), ...(rpm ? { rpm } : {}) });
  const res = await fetch(`${http}/gears/${id}?${q}`, { method: "POST" });
  if (!res.ok) {
    console.error(`✗ ${await res.text()}`);
    failed = true;
  } else {
    const c = await res.json();
    const s = c.sweep;
    const fits = s.pairs.filter((p) => p.gap > 0 && p.gap <= s.resolution);
    const near = s.pairs.find((p) => p.gap > s.resolution);
    console.log(s.clashes.length ? `✗ clash: ${s.clashes.map((p) => `${p.a} hits ${p.b} at ${p.at}°`).join("; ")}` : `✓ turns cleanly through ${s.steps} positions${near ? `; closest ${near.gap} mm (${near.a} ↔ ${near.b} at ${near.at}°)` : ""}${fits.length ? `; ${fits.length} running fits closer than ${s.resolution} mm` : ""}`);
    for (const m of c.meshes) {
      const st = m.strength;
      console.log(`  ${m.driver} → ${m.driven}: ${m.teeth.join(":")} m${m.module}, ratio ${m.ratio}:1, contact ratio ${m.contact_ratio}, backlash ${m.backlash} mm${st ? `; bending safety ${st.bending_safety.map((x) => x ?? "?").join(" / ")}, surface safety ${st.contact_safety ?? "?"}` : ""}`);
      for (const n of m.notes) console.log(`    ! ${n}`);
    }
    failed = s.clashes.length > 0;
  }
} else if (command === "motion" || command === "fit" || command === "bom") {
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const setFlag = args.indexOf("--set");
  const set = setFlag >= 0 ? args.splice(setFlag, 2)[1] : null;
  const motionFlag = args.indexOf("--motion");
  if (motionFlag >= 0) args.splice(motionFlag, 1);
  const [, id, extra] = args;
  if (!id) {
    console.error(`✗ say which: forma ${command} <id${command === "motion" ? "" : " | project"}>`);
    process.exit(1);
  }
  const q = new URLSearchParams(project ? { project } : {});
  let path;
  if (command === "motion") {
    path = `assembly-motion/${id}`;
    if (extra) q.set("steps", extra);
    if (set) q.set("set", set);
  } else if (command === "fit") {
    path = `fit/${id}`;
    if (motionFlag < 0) q.set("motion", "off");
  } else {
    path = `bom/${id}`;
    q.set("format", "csv");
    if (extra) q.set("qty", extra);
  }
  const res = await fetch(`${http}/${path}?${q}`);
  if (!res.ok) {
    console.error(`✗ ${await res.text()}`);
    failed = true;
  } else if (command === "bom") {
    process.stdout.write(await res.text());
  } else if (command === "motion") {
    const s = await res.json();
    const moving = s.instances.filter((i) => i.moves).map((i) => i.name);
    console.log(`${s.clashes.length || s.binds_at != null ? "✗" : "✓"} ${s.name}: ${s.from}…${s.to}${s.unit} in ${s.frames.length - 1} steps; ${moving.length} of ${s.instances.length} instances move`);
    for (const c of s.clashes) console.log(`  clash  ${c.a} ↔ ${c.b} from ${c.range?.[0] ?? c.at} to ${c.range?.[1] ?? c.at}${s.unit}`);
    const near = s.pairs.filter((p) => !p.joined && p.gap >= 1e-4).slice(0, 8);
    for (const p of near) console.log(`  close  ${p.a} ↔ ${p.b}: ${p.gap} mm at ${p.at}${s.unit}`);
    const touching = s.pairs.filter((p) => p.joined && p.gap < 1e-4).length;
    if (touching) console.log(`  (${touching} joined or turning-in-contact pairs touch as designed)`);
    for (const n of s.notes) console.log(`  ! ${n}`);
    failed = s.clashes.length > 0 || s.binds_at != null;
  } else {
    const r = await res.json();
    console.log(`${r.ok ? "✓" : "✗"} ${r.name}: ${r.pass} fit, ${r.warn} with warnings, ${r.fail} failing (${r.profile?.printer ?? "printer"} profile)`);
    const order = { fail: 0, warn: 1, pass: 2 };
    for (const it of [...r.items].sort((a, b) => order[a.status] - order[b.status])) {
      if (it.status === "pass" && extra !== "all") continue;
      console.log(`  ${it.status.padEnd(4)} ${it.kind.padEnd(12)} ${it.title}${it.joint ? ` (${it.joint})` : ""}`);
      for (const c of it.checks.filter((c) => c.status !== "pass")) console.log(`         ${c.what}: ${c.text}`);
    }
    if (r.pass && extra !== "all") console.log(`  (${r.pass} passing items hidden; add "all" to list them)`);
    for (const n of r.notes) console.log(`  ! ${n}`);
    failed = !r.ok;
  }
} else if (command === "pcb") {
  // Electronics: KiCad library parts, footprint verification, the Gerber read-back check.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = (extra = {}) => new URLSearchParams({ ...(project ? { project } : {}), ...extra });
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const get = async (path) => {
    const res = await fetch(`${http}${path}`);
    if (!res.ok) throw new Error(await res.text());
    return res.json();
  };
  const sub = file;
  try {
    if (sub === "parts") {
      const kind = args.includes("--footprints") ? "footprints" : "symbols";
      const words = args.slice(2).filter((a) => a !== "--footprints").join(" ");
      if (!words) throw new Error("say what to look for: forma pcb parts <words> [--footprints]");
      const r = await get(`/pcb/kicad/search?${q({ q: words, kind, limit: "30" })}`);
      if (!r.results.length) console.log(`nothing in the KiCad ${kind} matches “${words}”`);
      for (const e of r.results) console.log(`${e.id}  (${e.count} ${kind === "symbols" ? "pins" : "pads"})${e.footprint ? `  → ${e.footprint}` : e.filters ? `  footprints: ${e.filters}` : ""}\n    ${e.description || e.keywords}`);
      console.log(`(${r.licence})`);
    } else if (sub === "add") {
      // Place a KiCad library symbol, its footprint defined on the board in the same step.
      const fpFlag = flag("--footprint");
      const at = (flag("--at") ?? "50,50").split(",").map(Number);
      const reference = flag("--ref");
      const value = flag("--value");
      const [, , schematic, symbolId] = args;
      if (!schematic || !symbolId) throw new Error("usage: forma pcb add <schematic id> <Library:Symbol> [--footprint Library:Footprint] [--at x,y] [--ref U3] [--value …]");
      const s = await get(`/pcb/kicad/symbol/${encodeURIComponent(symbolId)}`);
      let footprint = s.footprint;
      if (fpFlag) footprint = (await get(`/pcb/kicad/footprint/${encodeURIComponent(fpFlag)}`)).footprint;
      if (!footprint) console.log(`! ${symbolId} names no footprint${s.definition.footprint ? ` it can read (${s.footprint_error ?? s.definition.footprint})` : ""}: give --footprint Library:Name (search with forma pcb parts <words> --footprints)`);
      const missing = fpFlag ? s.definition.pins.filter((p) => !footprint.pads.some((x) => x.number === p.number)).map((p) => p.number) : s.unmapped_pins;
      if (missing?.length) console.log(`! pins ${missing.join(", ")} have no pad of that number on ${footprint.name}: give pin_map in pcb.schematic.place`);
      const params = { schematic, symbol: s.definition.name, definition: s.definition, at, ...(footprint ? { footprint_definition: footprint, footprint: footprint.name } : {}), ...(reference ? { reference } : {}), ...(value ? { value } : {}) };
      const r = await engine.commit("pcb.schematic.place", params);
      if (!r.ok) throw new Error(`${r.error.code} ${r.error.message}`);
      console.log(`✓ placed ${r.value.output.reference} = ${symbolId}${footprint ? ` on ${footprint.name}` : ""} (rev ${r.value.revision})${s.definition.lcsc ? `; LCSC ${s.definition.lcsc}` : ""}`);
      for (const n of s.footprint_notes ?? []) console.log(`  note: ${n}`);
    } else if (sub === "footprint") {
      // Define a KiCad library footprint on a board (for pcb.component.create).
      const [, , board, fpId] = args;
      if (!board || !fpId) throw new Error("usage: forma pcb footprint <board id> <Library:Name>");
      const f = await get(`/pcb/kicad/footprint/${encodeURIComponent(fpId)}`);
      const r = await engine.commit("pcb.footprint.define", { board, footprint: f.footprint });
      if (!r.ok) throw new Error(`${r.error.code} ${r.error.message}`);
      console.log(`✓ ${fpId} defined on the board: ${f.footprint.pads.length} pads (use it as footprint "${fpId}")`);
      for (const n of f.notes) console.log(`  note: ${n}`);
    } else if (sub === "verify-footprints") {
      const names = args.slice(2).join(",");
      const r = await get(`/pcb/verify-footprints?${q(names ? { names } : {})}`);
      console.log(`${r.errors ? "✗" : "✓"} ${r.checked} footprints against KiCad's library: ${r.errors} errors, ${r.warnings} warnings`);
      for (const f of r.footprints) {
        const shown = f.findings.filter((x) => x.severity !== "note" || names);
        if (!shown.length) {
          if (names) console.log(`  ✓ ${f.footprint} = ${f.kicad}`);
          continue;
        }
        console.log(`  ${f.errors ? "✗" : "~"} ${f.footprint} vs ${f.kicad} (${f.pads}/${f.kicad_pads} pads, worst centre ${f.worst_position} mm)`);
        for (const x of shown) console.log(`      ${x.severity.padEnd(7)} ${x.pad ? `${x.pad}: ` : ""}${x.message}`);
      }
      for (const m of r.missing) console.log(`  ? ${m}`);
      failed = r.errors > 0;
    } else if (sub === "gerber-check") {
      // The fab package read back by an independent reader and compared with the board.
      const board = args[2];
      if (!board) throw new Error("usage: forma pcb gerber-check <board id>");
      const c = await get(`/pcb/${board}/gerber-check?${q()}`);
      console.log(`${c.ok ? "✓" : "✗"} ${c.summary}`);
      for (const l of c.layers) {
        const area = l.board_area != null ? `, copper ${l.file_area} mm² (board ${l.board_area}, ${l.differ_area} differ)` : "";
        console.log(`  ${l.ok ? "✓" : "✗"} ${l.file} (${l.function}): ${l.objects} objects${l.pads ? `, ${l.pads_ok}/${l.pads} pads` : ""}${area}${l.nets.length ? `, ${l.nets.filter((n) => n.ok).length}/${l.nets.length} nets match` : ""}`);
        for (const p of l.problems) console.log(`      ✗ ${p}`);
        for (const n of l.notes) console.log(`      · ${n}`);
      }
      for (const d of c.drills) {
        console.log(`  ${d.ok ? "✓" : "✗"} ${d.file}: ${d.file_holes}/${d.board_holes} holes (${d.sizes.map(([s, b, f]) => `ø${s}×${f}`).join(", ")})`);
        for (const p of d.problems) console.log(`      ✗ ${p}`);
      }
      if (c.outline) {
        const o = c.outline;
        console.log(`  ${o.ok ? "✓" : "✗"} ${o.file}: ${o.loops} closed loop(s), ${o.file_size.join(" × ")} mm, ${o.file_area} mm²`);
        for (const p of o.problems) console.log(`      ✗ ${p}`);
      }
      for (const u of c.unreadable) console.log(`  ✗ ${u}`);
      failed = !c.ok;
    } else if (sub === "library") {
      if (args[2] === "update") {
        const res = await fetch(`${http}/pcb/kicad/fetch?repo=all`, { method: "POST" });
        if (!res.ok) throw new Error(await res.text());
      }
      const s = await get("/pcb/kicad");
      console.log(`KiCad libraries in ${s.root}\n  footprints: ${s.footprints ?? "not downloaded yet"}\n  symbols:    ${s.symbols ?? "not downloaded yet"}\n  ${s.licence}`);
    } else if (sub === "route" || sub === "drag" || sub === "cleanup" || sub === "optimize") {
      // The interactive router from the shell: lay a route pad to pad, drag a track or via, tidy copper.
      const point = (v) => (/^-?[\d.]+,-?[\d.]+$/.test(v) ? v.split(",").map(Number) : v);
      const mode = flag("--mode");
      const layer = flag("--layer");
      const index = flag("--index");
      const kind = flag("--kind");
      const [, , board, a, b] = args;
      let op;
      let params;
      if (sub === "route") {
        if (!board || !a || !b) throw new Error("usage: forma pcb route <board> <from: U1.3 | x,y> <to: R4.1 | x,y> [--mode walkaround|shove|highlight] [--layer top]");
        op = "pcb.route.interactive";
        params = { board, from: point(a), to: point(b), ...(mode ? { mode } : {}), ...(layer ? { layer } : {}) };
      } else if (sub === "drag") {
        if (!board || !a || !b) throw new Error("usage: forma pcb drag <board> <track or via id> <x,y> [--index n] [--kind segment|corner|arc|via] [--mode shove|highlight]");
        op = "pcb.route.drag";
        params = { board, target: a, to: point(b), ...(index ? { index: Number(index) } : {}), ...(kind ? { kind } : {}), ...(mode ? { mode } : {}) };
      } else {
        if (!board) throw new Error(`usage: forma pcb ${sub} <board>`);
        op = sub === "cleanup" ? "pcb.route.cleanup" : "pcb.route.optimize";
        params = { board };
      }
      const r = await engine.commit(op, params);
      if (!r.ok) throw new Error(`${r.error.code} ${r.error.message}`);
      const o = r.value.output;
      if (sub === "route" || sub === "drag") {
        console.log(`${o.ok ? "✓" : "✗"} ${sub === "route" ? `${o.net ?? "no net"}: ${o.tracks.length} track(s), ${o.vias.length} via(s)` : "dragged"} in ${o.mode}${o.reached === false ? ", stopped short" : ""}; pushed ${o.pushed_tracks} track(s), ${o.pushed_vias} via(s) (rev ${r.value.revision})`);
        for (const c of o.collisions) console.log(`  ✗ ${c.layer ?? ""} too close to ${c.with}: ${c.gap} mm (needs ${c.need}) at ${c.at.map((x) => x.toFixed(2)).join(", ")}`);
        for (const n of o.notes) console.log(`  · ${n}`);
        failed = !o.ok;
      } else console.log(`✓ ${JSON.stringify(o)} (rev ${r.value.revision})`);
    } else {
      throw new Error("usage: forma pcb parts <words> [--footprints] | add <schematic> <Library:Symbol> [--footprint L:F] [--at x,y] | footprint <board> <Library:Name> | verify-footprints [names…] | gerber-check <board> | library [update] | route <board> <from> <to> [--mode shove] | drag <board> <item> <x,y> | cleanup <board> | optimize <board>");
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else if (command === "simulate") {
  // ngspice on a schematic: waveforms, measurements, datasheet checks and expectations.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const all = (name) => {
    const out = [];
    for (let i = args.indexOf(name); i >= 0; i = args.indexOf(name)) out.push(args.splice(i, 2)[1]);
    return out;
  };
  const time = (s) => {
    const m = String(s).trim().match(/^([-\d.e+]+)\s*(s|ms|us|µs|ns)?$/i);
    if (!m) throw new Error(`“${s}” is not a time (30ms, 50us, 0.02)`);
    return Number(m[1]) * ({ s: 1, ms: 1e-3, us: 1e-6, µs: 1e-6, ns: 1e-9 }[(m[2] ?? "s").toLowerCase()] ?? 1);
  };
  const num = (s) => {
    const m = String(s).trim().match(/^([-\d.e+]+)\s*([a-zµ]*)$/i);
    if (!m) throw new Error(`“${s}” is not a number`);
    const p = { m: 1e-3, u: 1e-6, µ: 1e-6, n: 1e-9, k: 1e3, meg: 1e6 }[m[2].replace(/[vahzs]$/i, "").toLowerCase()] ?? 1;
    return Number(m[1]) * p;
  };
  try {
    const tran = all("--tran")[0];
    const dc = all("--dc")[0];
    const ac = all("--ac")[0];
    const save = all("--save")[0];
    const csv = all("--csv")[0];
    const derate = all("--derate")[0];
    const step = all("--step")[0];
    const schematic = args[1];
    if (!schematic) throw new Error('usage: forma simulate <schematic id> [--tran 30ms] [--dc NET:0:6:0.1] [--ac NET:10:1e6] [--source NET=5V,r=0.2,rise=1ms] [--press SW2@20ms:5ms] [--burst U1@5ms:2ms:0.5A] [--expect "min(V(3V3), burst) > 3.0"]… [--derate 0.6] [--save name] [--csv out.csv]');
    let analysis = { kind: "tran", stop: tran ? time(tran) : 0.03, step: step ? time(step) : null };
    if (dc) {
      const [net, from, to, st] = dc.split(":");
      analysis = { kind: "dc", net, from: num(from), to: num(to), step: num(st ?? "0.05") };
    } else if (ac) {
      const [net, from, to] = ac.split(":");
      analysis = { kind: "ac", net, from: num(from ?? "10"), to: num(to ?? "1e6"), points: 20 };
    } else if (args.includes("--op")) {
      args.splice(args.indexOf("--op"), 1);
      analysis = { kind: "op" };
    }
    const sources = all("--source").map((s) => {
      const [net, rest] = s.split("=");
      const [v, ...kv] = rest.split(",");
      const o = Object.fromEntries(kv.map((x) => x.split("=")));
      return { net, volts: num(v), r: o.r != null ? num(o.r) : null, rise: o.rise != null ? time(o.rise) : null, delay: o.delay != null ? time(o.delay) : null };
    });
    const presses = all("--press").map((s) => {
      const [part, rest] = s.split("@");
      const [at, hold] = rest.split(":");
      return { part, at: time(at), hold: time(hold ?? "10ms") };
    });
    const bursts = all("--burst").map((s) => {
      const [part, rest] = s.split("@");
      const [at, length, current, baseA] = rest.split(":");
      return { part, at: time(at), length: time(length ?? "2ms"), current: num(current ?? "0.5"), base: baseA ? num(baseA) : null };
    });
    const request = { analysis, sources, presses, bursts, expect: all("--expect"), mlcc_derating: derate ? Number(derate) : null };
    const res = await fetch(`${http}/pcb/schematic/${schematic}/simulate${project ? `?project=${encodeURIComponent(project)}` : ""}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(request) });
    if (!res.ok) throw new Error(await res.text());
    const r = await res.json();
    const mark = { pass: "✓", warn: "!", fail: "✗" };
    console.log(`${r.ok ? "✓" : "✗"} ${analysis.kind} analysis, ${r.x.length} points; ${r.parts.length} parts modelled${r.skipped.length ? `, ${r.skipped.length} left out` : ""}`);
    for (const p of r.parts) console.log(`  ${p.reference.padEnd(5)} ${p.model}${p.approximate ? "  [approximate]" : ""}`);
    for (const s of r.skipped) console.log(`  ·     ${s}`);
    for (const a of r.assumptions) console.log(`  assumed: ${a}`);
    if (r.windows.length > 1) console.log(`  windows: ${r.windows.map((w) => `${w.name} ${(w.from * 1e3).toFixed(3)}–${(w.to * 1e3).toFixed(3)} ms`).join(", ")}`);
    if (r.checks.length) console.log("Datasheet checks:");
    for (const c of r.checks) console.log(`  ${mark[c.status]} ${c.name} (${c.limit}): ${c.detail}`);
    if (r.expectations.length) console.log("Expectations:");
    for (const c of r.expectations) console.log(`  ${mark[c.status]} ${c.name}: ${c.detail}`);
    if (r.measurements.length) console.log("Measurements:");
    for (const c of r.measurements) console.log(`  ${c.name}: ${c.detail}`);
    if (analysis.kind === "op" || analysis.kind === "dc" || analysis.kind === "ac") {
      for (const s of r.signals) console.log(`  ${s.name}: ${s.values.length === 1 ? s.values[0] : `${s.values[0].toPrecision(4)} … ${s.values[s.values.length - 1].toPrecision(4)} ${s.unit}`}`);
    }
    if (csv) {
      const { writeFileSync } = await import("node:fs");
      const head = [r.x_name, ...r.signals.map((s) => s.name)].join(",");
      writeFileSync(csv, [head, ...r.x.map((x, i) => [x, ...r.signals.map((s) => s.values[i])].join(","))].join("\n") + "\n");
      console.log(`waveforms written to ${csv}`);
    }
    if (save) {
      const c = await engine.commit("pcb.simulation.save", { schematic, name: save, request });
      if (!c.ok) throw new Error(`${c.error.code} ${c.error.message}`);
      console.log(`✓ saved as “${save}”: ./forma check runs it from now on`);
    }
    failed = !r.ok;
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else if (command === "drawing") {
  // A shape's engineering drawing: what each sheet shows and what the checker
  // measured on it; --pdf/--svg/--dxf/--zip save the files.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const files = { pdf: flag("--pdf"), svg: flag("--svg"), dxf: flag("--dxf"), zip: flag("--zip") };
  const q = new URLSearchParams(project ? { project } : {});
  for (const [k, f] of [["sheet", "--sheet"], ["part", "--part"], ["projection", "--projection"], ["config", "--config"]]) {
    const v = flag(f);
    if (v) q.set(k, v);
  }
  const only = flag("--page");
  const id = args[1];
  if (!id) {
    console.error("✗ say which: forma drawing <shape or instance id> [--sheet A3] [--part Name] [--page N] [--pdf out.pdf] [--svg out.svg] [--dxf out.dxf] [--zip out.zip]");
    process.exit(1);
  }
  const sheet = async (page) => {
    const p = new URLSearchParams(q);
    p.set("page", String(page));
    const res = await fetch(`${http}/drawing/${id}?${p}`);
    if (!res.ok) throw new Error(await res.text());
    return res.json();
  };
  try {
    const first = await sheet(only ? +only : 1);
    const pages = only ? [first] : [first, ...(await Promise.all(Array.from({ length: first.pages - 1 }, (_, i) => sheet(i + 2))))];
    for (const d of pages) {
      const views = d.views.map((v) => v.view).concat(d.iso ? ["iso"] : []).join(", ");
      const quality = d.quality;
      const clean = !quality.overlaps.length && !quality.on_lines.length && !quality.off_sheet.length && !quality.duplicates.length;
      console.log(`${clean && !d.warnings.length ? "✓" : "!"} sheet ${d.page} of ${d.pages}: ${d.width}×${d.height} mm at ${d.scale_text}; views ${views || "none"}${d.balloons.length ? `; ${d.balloons.length} balloons` : ""}`);
      const by = {};
      for (const a of d.autos) (by[a.key.split(":")[0]] ??= []).push(a);
      const kinds = { x: "x positions", y: "y positions", z: "z positions", d: "diameters", hole: "hole callouts", r: "radii", c: "chamfers" };
      for (const [k, list] of Object.entries(by)) console.log(`  ${(kinds[k] ?? k).padEnd(14)} ${list.map((a) => `${a.key.slice(k.length + 1)}${a.text && !a.text.match(/^[\d.]+$/) ? ` (${a.text})` : ""}`).join(" · ")}`);
      if (d.marks.length) console.log(`  your dimensions: ${d.marks.length}`);
      const kinds2 = {};
      for (const a of d.annotations) kinds2[a.kind] = (kinds2[a.kind] ?? 0) + 1;
      const ann = Object.entries(kinds2).filter(([k]) => k !== "hole").map(([k, n]) => `${n} ${k}${n > 1 ? (k.endsWith("sh") ? "es" : "s") : ""}`);
      if (ann.length) console.log(`  annotations: ${ann.join(", ")}`);
      for (const w of d.warnings) console.log(`  ! ${w}`);
      for (const [a, b] of quality.overlaps) console.log(`  ✗ overlapping text: “${a}” and “${b}”`);
      for (const t of quality.on_lines) console.log(`  ✗ text over the part's lines: “${t}”`);
      for (const t of quality.off_sheet) console.log(`  ✗ off the sheet: “${t}”`);
      for (const t of quality.duplicates) console.log(`  ✗ dimensioned twice: ${t}`);
      if (clean) console.log(`  checker: no overlaps, nothing over the part's lines, nothing said twice; smallest lettering ${quality.smallest_text} mm`);
    }
    const { writeFileSync } = await import("node:fs");
    for (const [ext, out] of Object.entries(files)) {
      if (!out) continue;
      const p = new URLSearchParams(q);
      if (only) p.set("page", only);
      const res = await fetch(`${http}/drawing/${id}.${ext}?${p}`);
      if (!res.ok) throw new Error(await res.text());
      writeFileSync(out, Buffer.from(await res.arrayBuffer()));
      console.log(`  saved ${out}`);
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else if (command === "stock") {
  // The stock library: families, or the sizes matching a search ("M3×12", "608", "servo").
  const query = args.slice(1).join(" ");
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const res = await fetch(`${http}/stock${project ? `?project=${encodeURIComponent(project)}` : ""}`);
  if (!res.ok) {
    console.error(`✗ ${await res.text()}`);
    failed = true;
  } else {
    const cat = await res.json();
    const norm = (s) => String(s ?? "").toLowerCase().replace(/[×x*]/g, "x");
    const words = norm(query).split(/\s+/).filter(Boolean);
    let shown = 0;
    for (const f of cat.families) {
      const held = cat.inProject[f.id] ? `  [in project: ${cat.inProject[f.id]}]` : "";
      if (!words.length) {
        console.log(`${f.id.padEnd(22)} ${f.title} · ${f.sizes.length} sizes (${f.sizes[0].name} … ${f.sizes[f.sizes.length - 1].name}), default ${f.default}${held}`);
        continue;
      }
      const ft = norm([f.id, f.name, f.standard, f.category, ...f.keywords].join(" "));
      const rest = words.filter((w) => !ft.includes(w));
      const sizes = f.sizes.filter((s) => rest.every((w) => norm([s.name, s.partNumber, s.mpn].join(" ")).includes(w)));
      if (!sizes.length) continue;
      const most = rest.length ? 40 : 12;
      console.log(`${f.id}  ${f.title}${held}\n  ${f.summary}`);
      for (const s of sizes.slice(0, most)) {
        const m = Object.entries(s.metrics ?? {}).map(([k, v]) => `${k} ${+v.value.toFixed(3)}${v.unit ? ` ${v.unit}` : ""}`).join(", ");
        console.log(`  ${s.name.padEnd(12)} ${String(s.partNumber).padEnd(34)} ${s.unitPrice != null ? `$${s.unitPrice}` : ""}${s.pack > 1 ? `/pc (pack ${s.pack})` : ""}  ${s.massG} g  ${s.size?.join("×")} mm${m ? `  ${m}` : ""}\n      connectors: ${(s.connectors ?? []).join(", ")}`);
      }
      if (sizes.length > most) console.log(`  … ${sizes.length - most} more sizes (narrow the search)`);
      shown++;
    }
    if (words.length && !shown) console.log(`nothing in the stock library matches “${query}”`);
  }
} else if (command === "image") {
  // Pictures (and fonts) shape programs read: add one, or list them.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = project ? `?project=${encodeURIComponent(project)}` : "";
  const sub = file;
  if (sub === "add") {
    const files = args.slice(2);
    if (!files.length) {
      console.error("✗ say which file: forma image add <photo.png|logo.jpg|font.ttf> …");
      failed = true;
    }
    for (const f of files) {
      let bytes;
      try {
        bytes = readFileSync(f);
      } catch (e) {
        console.error(`✗ ${f}: ${e.message}`);
        failed = true;
        continue;
      }
      const name = f.split("/").pop();
      const res = await fetch(`${http}/images/${encodeURIComponent(name)}${q}`, { method: "POST", body: bytes });
      if (!res.ok) {
        console.error(`✗ ${await res.text()}`);
        failed = true;
        continue;
      }
      const e = await res.json();
      const size = e.width ? `, ${e.width} × ${e.height} px` : "";
      const use = e.kind === "ttf" || e.kind === "otf" ? `text("…", { font: "${e.id}" })` : `image("${e.id}")`;
      console.log(`✓ ${name} (${e.kind.toUpperCase()}${size}) → ${e.id}\n  in a program: ${use}`);
    }
  } else if (sub === "list" || !sub) {
    const res = await fetch(`${http}/images${q}`);
    const list = await res.json();
    if (!list.length) console.log("no pictures or fonts in this project yet (forma image add <file>)");
    for (const e of list) console.log(`${e.id}  ${e.name} (${e.kind.toUpperCase()}${e.width ? `, ${e.width} × ${e.height} px` : ""})`);
  } else {
    console.error("usage: forma image add <file> … | forma image list");
    failed = true;
  }
} else if (command === "look") {
  // A picture of the model drawn by the engine, saved here to read.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const viewName = flag("--view") ?? "iso";
  const size = flag("--size");
  const out = flag("--out");
  const what = args.slice(1).join(" ").trim();
  let id = null;
  if (what) {
    const s = await engine.snapshot;
    const name = (e) => {
      const c = e.components;
      return c["assembly.instance"]?.name ?? c["shape.program"]?.name ?? c["assembly.definition"]?.name;
    };
    const byId = s.entities.find((e) => e.id === what);
    const named = byId ? [byId] : s.entities.filter((e) => name(e)?.toLowerCase() === what.toLowerCase());
    const placed = named.filter((e) => e.components["assembly.instance"] || e.components["solid.placement"]);
    const pick = placed.length ? placed : named;
    if (pick.length !== 1) {
      console.error(pick.length ? `✗ “${what}” matches ${pick.length} things: use an id (${pick.map((e) => e.id).join(", ")})` : `✗ nothing called “${what}”`);
      failed = true;
    } else id = pick[0].id;
  }
  if (!failed) {
    const q = new URLSearchParams({ ...(project ? { project } : {}), ...(id ? { id } : {}), view: viewName, ...(size ? { size } : {}) });
    const res = await fetch(`${http}/look?${q}`);
    if (!res.ok) {
      console.error(`✗ ${await res.text()}`);
      failed = true;
    } else {
      const { mkdirSync, writeFileSync } = await import("node:fs");
      const slug = (what || "all").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "part";
      const path = out ?? `look/${slug}-${viewName.replace(/[^a-z0-9]+/gi, "_")}.png`;
      if (path.includes("/")) mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, Buffer.from(await res.arrayBuffer()));
      const b = res.headers.get("x-forma-bounds").split(",").map(Number);
      console.log(`✓ ${path}  (${viewName} view; read the file to see it)\n  ${res.headers.get("x-forma-mm-per-px")} mm per pixel; bounds x ${b[0]}…${b[3]}, y ${b[1]}…${b[4]}, z ${b[2]}…${b[5]} mm; ${res.headers.get("x-forma-triangles")} triangles`);
    }
  }
} else if (command === "list") {
  const s = await engine.snapshot;
  for (const e of s.entities) {
    const c = e.components;
    const kind = c["shape.program"] ? `shape “${c["shape.program"].name}”` : c["solid.box"] ? "box" : c["solid.material"] ? `material ${c["solid.material"].name}` : c["intent.requirement"] ? `requirement “${c["intent.requirement"].name}”` : "entity";
    const extra = e.result ? ` — ${e.result.passed ? "met" : "FAILING"}: ${e.result.summary}` : e.mass ? ` — ${(e.mass.volume_m3 * 1e9).toFixed(1)} mm³` : "";
    console.log(`${e.id}  ${kind}${extra}`);
  }
} else if (command === "get") {
  const s = await engine.snapshot;
  const e = s.entities.find((x) => x.id === file);
  if (!e) {
    console.error(`no entity ${file}`);
    failed = true;
  } else {
    // Meshes are for drawing, not reading: report their size instead.
    const { mesh, ...rest } = e;
    console.log(JSON.stringify({ ...rest, mesh: mesh ? { triangles: mesh.indices.length / 3 } : null }, null, 2));
  }
} else if (command === "commit") {
  const operation = file;
  const raw = args[2] ?? "{}";
  const parameters = resolveRefs(JSON.parse(raw), [], process.cwd());
  const r = await engine.commit(operation, parameters);
  if (r.ok) console.log(`✓ ${operation} → ${JSON.stringify(r.value.output)} (rev ${r.value.revision})`);
  else {
    console.error(`✗ ${operation}: ${r.error.code} ${r.error.message}`);
    failed = true;
  }
} else if (command === "ops") {
  const s = await engine.snapshot;
  for (const c of s.commands.filter((c) => !file || c.name.includes(file))) console.log(`${c.name}\n  ${JSON.stringify(c.schema)}\n`);
} else if (command === "tree") {
  const s = await engine.snapshot;
  const byId = new Map(s.entities.map((e) => [e.id, e]));
  const inst = (e) => e.components["assembly.instance"];
  const label = (e) => {
    const c = e.components;
    const i = inst(e);
    if (i) {
      const def = byId.get(i.of);
      const defName = def?.components["shape.program"]?.name ?? def?.components["assembly.definition"]?.name ?? "?";
      const users = s.entities.filter((x) => inst(x)?.of === i.of).length;
      const props = Object.entries(i.properties ?? {}).map(([k, v]) => `${k}=${v}`).join(" ");
      return `${i.name}  [${defName}${users > 1 ? `, shared ×${users}` : ""}] at ${i.at.map((v) => Math.round(v)).join(",")}${(Array.isArray(i.rotation) ? i.rotation.some((v) => v) : i.rotation) ? ` rot ${[].concat(i.rotation).join(",")}°` : ""}${props ? `  {${props}}` : ""}  (${e.id})`;
    }
    if (c["shape.program"]) return `${c["shape.program"].name}${e.shape?.parts?.length ? ` (${e.shape.parts.length} parts)` : ""}  (${e.id})`;
    if (c["solid.box"]) return `Box  (${e.id})`;
    return e.id;
  };
  const walk = (e, depth) => {
    console.log(`${"  ".repeat(depth)}${label(e)}`);
    const i = inst(e);
    if (!i) return;
    const kids = s.entities.filter((x) => inst(x)?.parent === i.of).sort((a, b) => inst(a).at[2] - inst(b).at[2] || inst(a).name.localeCompare(inst(b).name, undefined, { numeric: true }));
    for (const child of kids) walk(child, depth + 1);
  };
  for (const e of s.entities) {
    const i = inst(e);
    if (i ? !i.parent : (e.components["shape.program"] && e.components["solid.placement"]) || e.components["solid.box"]) walk(e, 0);
  }
  const defs = s.entities.filter((e) => e.components["assembly.definition"] || (e.components["shape.program"] && !e.components["solid.placement"]));
  if (defs.length) {
    console.log("\nDefinitions (components and assemblies):");
    for (const d of defs) console.log(`  ${d.components["assembly.definition"]?.name ?? d.components["shape.program"].name}  (${d.id})`);
  }
} else if (command === "measure") {
  // Mass properties from the engine (exact on flat-faced solids), and between two items their centres and gaps.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const partFlag = args.indexOf("--part");
  const part = partFlag >= 0 ? args.splice(partFlag, 2)[1] : null;
  const [, a, b] = args;
  if (!a) {
    console.error("✗ say what: forma measure <part or instance> [other] [--part name]");
    process.exit(1);
  }
  const get = async (id) => {
    const q = new URLSearchParams({ ...(project ? { project } : {}), ...(part ? { part } : {}) });
    const res = await fetch(`${http}/measure/${encodeURIComponent(id)}?${q}`);
    if (!res.ok) throw new Error(`${id}: ${await res.text()}`);
    return res.json();
  };
  const n = (x, d = 3) => Number(x.toFixed(d)).toString();
  const v3 = (v, d = 3) => v.map((x) => n(x, d)).join(", ");
  try {
    const list = [await get(a), ...(b ? [await get(b)] : [])];
    list.forEach((m, i) => {
      const name = i ? b : a;
      console.log(`${name}${part ? ` (${part})` : ""}:`);
      console.log(`  volume ${n(m.volume_mm3, 2)} mm³, surface area ${n(m.area_mm2, 2)} mm²`);
      console.log(`  bounds ${v3(m.bounds_min, 2)} … ${v3(m.bounds_max, 2)} mm (size ${v3(m.bounds_max.map((x, k) => x - m.bounds_min[k]), 2)})`);
      console.log(`  mass ${m.mass_kg == null ? "— (no material)" : `${n(m.mass_kg, 6)} kg`}; centre of mass ${v3(m.center_of_mass)} mm`);
      if (m.inertia_kg_mm2) {
        const I = m.inertia_kg_mm2;
        console.log(`  inertia about the centre of mass (kg·mm²): Ixx ${n(I[0][0], 4)}, Iyy ${n(I[1][1], 4)}, Izz ${n(I[2][2], 4)}; Ixy ${n(I[0][1], 4)}, Ixz ${n(I[0][2], 4)}, Iyz ${n(I[1][2], 4)}`);
        console.log(`  principal moments ${v3(m.principal_kg_mm2, 4)} kg·mm²`);
      } else {
        const J = m.inertia_mm5;
        console.log(`  second moments of volume (mm⁵, inertia at unit density): ${n(J[0][0], 1)}, ${n(J[1][1], 1)}, ${n(J[2][2], 1)}`);
      }
    });
    if (list.length === 2) {
      const [p, q] = list;
      const d = q.center_of_mass.map((x, k) => x - p.center_of_mass[k]);
      console.log(`between: centre to centre ${n(Math.hypot(...d))} mm (Δ ${v3(d)})`);
      // Gap between bounding boxes along each axis (negative: they overlap that much).
      const gap = [0, 1, 2].map((k) => Math.max(p.bounds_min[k], q.bounds_min[k]) - Math.min(p.bounds_max[k], q.bounds_max[k]));
      console.log(`  box gaps X ${n(gap[0], 2)}, Y ${n(gap[1], 2)}, Z ${n(gap[2], 2)} mm (negative: overlapping); boxes ${gap.some((g) => g > 0) ? `apart by at least ${n(Math.hypot(...gap.map((g) => Math.max(g, 0))), 2)} mm` : "overlap"}`);
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else if (command === "view") {
  const action = file;
  let arg = args.slice(2).join(" ").trim();
  let target = null;
  let value = null;
  // explode: how far apart (a number, 1 by default) or "off", after the name.
  const how = action === "explode" ? arg.match(/\s+(off|\d*\.?\d+)$/) : null;
  if (how) {
    value = how[1];
    arg = arg.slice(0, how.index).trim();
  }
  if (["color", "camera", "projection", "display", "section"].includes(action)) value = arg;
  else if (action === "measure" && arg) {
    // One or two items: their ids (names resolved below, one at a time).
    const s = await engine.snapshot;
    const name = (e) => e.components["assembly.instance"]?.name ?? e.components["shape.program"]?.name;
    const find = (x) => s.entities.find((e) => e.id === x) ?? s.entities.find((e) => name(e)?.toLowerCase() === x.toLowerCase() && (e.components["assembly.instance"] || e.components["solid.placement"]));
    const words = arg.split(/\s+/);
    // Names can have spaces: try every split into two.
    let found = null;
    for (let i = words.length; i >= 1 && !found; i--) {
      const x = find(words.slice(0, i).join(" "));
      const rest = words.slice(i).join(" ");
      const y = rest ? find(rest) : null;
      if (x && (!rest || y)) found = [x.id, y?.id ?? null];
    }
    if (!found) {
      console.error(`✗ cannot find what to measure in “${arg}”`);
      failed = true;
    } else [target, value] = found;
  } else if (arg) {
    // An id, or a name: an instance (L26), a shape, an assembly or a parameter set.
    const s = await engine.snapshot;
    const name = (e) => {
      const c = e.components;
      return c["assembly.instance"]?.name ?? c["shape.program"]?.name ?? c["assembly.definition"]?.name ?? c["shape.parameter_set"]?.name;
    };
    const byId = s.entities.find((e) => e.id === arg);
    const named = byId ? [byId] : s.entities.filter((e) => name(e)?.toLowerCase() === arg.toLowerCase());
    // Prefer what is placed (an instance) over the definition it shows.
    const placed = named.filter((e) => e.components["assembly.instance"] || e.components["solid.placement"]);
    const pick = placed.length ? placed : named;
    if (pick.length !== 1) {
      console.error(pick.length ? `“${arg}” matches ${pick.length} things: use an id (${pick.map((e) => e.id).join(", ")})` : `nothing called “${arg}”`);
      failed = true;
    } else target = pick[0].id;
  }
  if (!failed) {
    const r = await engine.send("view", { action, target, value });
    if (r.ok) console.log(`✓ ${action}${arg ? ` ${arg}` : ""} shown to ${r.value.shown_to}`);
    else {
      console.error(`✗ ${r.error.message}`);
      failed = true;
    }
  }
} else {
  console.error("usage: forma.mjs run steps.json | commit <op> <json> | stock [search] | get <id> | tree | list | check | ops | measure <part> [other] | view <action> [what] | image add <file> | image list | look [what] [--view v]  [--project name] [--url ws://…]");
  failed = true;
}

engine.close();
process.exit(failed ? 1 : 0);
