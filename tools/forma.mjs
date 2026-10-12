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
//   node tools/forma.mjs study <shape id> study.json [--part X]   save a part's study (faces as { normal, near: [x, y, z] })
//   node tools/forma.mjs stress|fatigue|modes|buckle|heat|converge|inp <shape id> [--part X] [--count N] [--resolutions 32,48,64]
//                                             run the part's saved study (inp: a CalculiX deck on stdout)
//   node tools/forma.mjs probe <key> x,y,z · section <key> x,y,z nx,ny,nz · path <key> x,y,z x,y,z [n] · csv <key>
//                                             read a stress result back (the key a stress run prints)
//   node tools/forma.mjs frame frame.json     a frame of beams (joints, members with sections, supports, loads)
//   node tools/forma.mjs materials [search]   the material library with sources, and the project's materials
//   node tools/forma.mjs stock [search]       the stock library: families, or the sizes matching (M3×12, 608, servo)
//   node tools/forma.mjs list                 every entity, one line each
//   node tools/forma.mjs tree                 what is placed, as a tree (instances, parts)
//   node tools/forma.mjs get <id>             one entity in full (program source, params, results)
//   node tools/forma.mjs commit <operation> '<json>'   one command (same $file inlining)
//   node tools/forma.mjs ops [filter]         the commands the engine accepts, with schemas
//   node tools/forma.mjs view <action> [what] show something in the person's view (not saved):
//        select L26 · frame L26 · isolate L26 · show_all · hide L19 · show L19
//        color status|material · camera top|front|bottom|perspective
//        explode Bracket [spread] · explode Bracket off
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
} else if (["stress", "fatigue", "modes", "buckle", "heat", "converge", "inp", "probe", "section", "path", "csv", "frame", "materials", "study"].includes(command)) {
  // Simulation: a part's saved study run as kept, results read back, frames, materials.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const part = flag("--part");
  const count = flag("--count");
  const resolutions = flag("--resolutions");
  const [, id, ...rest] = args;
  const q = new URLSearchParams({ ...(project ? { project } : {}), ...(part ? { part } : {}) });
  const n = (v, d = 2) => (v == null || !Number.isFinite(v) ? String(v) : Number(v.toFixed(d)).toString());
  const v3 = (v) => `[${v.map((x) => n(x, 2)).join(", ")}]`;
  const get = async (path, init) => {
    const res = await fetch(`${http}${path}`, init);
    if (!res.ok) throw new Error(await res.text());
    return res;
  };
  try {
    if (command === "study") {
      // forma study <shape id> study.json [--part X]: save a study (faces as { normal, near: [x, y, z] }).
      const study = JSON.parse(readFileSync(rest[0], "utf8"));
      const r = await engine.commit("shape.set_study", { target: id, study: { ...study, part: part ?? study.part ?? null } });
      if (!r.ok) throw new Error(`${r.error.code} ${r.error.message}`);
      console.log(`✓ study saved on ${id}${part ? ` (${part})` : ""}`);
    } else if (command === "materials") {
      const m = await (await get(`/materials?${q}`)).json();
      const words = rest.join(" ").toLowerCase();
      for (const x of m.library.filter((x) => !words || x.name.toLowerCase().includes(words))) {
        console.log(`${x.name} (${x.family}): E ${n(x.e / 1000, 1)} GPa, ν ${x.nu}, ρ ${x.density} kg/m³, yield ${x.yield_strength} MPa, UTS ${x.uts} MPa, Et ${x.tangent_modulus} MPa, fatigue ${x.fatigue_strength ?? "—"} MPa @ ${x.fatigue_cycles.toExponential(0)}${x.endurance_limit ? " (limit)" : ""}, k ${x.conductivity} W/m·K, c ${x.specific_heat} J/kg·K, α ${n(x.expansion * 1e6, 1)} µm/m·K\n    source: ${x.source}`);
      }
      for (const x of m.project) console.log(`project material ${x.id} “${x.name}”${x.set ? ` — set here: ${JSON.stringify(x.set)}` : ""}${x.properties ? "" : " — no figures (unknown name: set its properties)"}`);
    } else if (command === "frame") {
      const f = JSON.parse(readFileSync(id, "utf8"));
      const r = await (await get(`/frame`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(f) })).json();
      console.log(`${r.safety >= 1 ? "✓" : "✗"} frame: safety ×${n(r.safety)} (${n(r.max_stress, 1)} MPa at most), moves ${n(r.max_displacement, 3)} mm at most, ${n(r.mass, 3)} kg${r.buckling != null ? `, lowest buckling ×${n(r.buckling)}` : ""}`);
      r.members.forEach((m, i) => console.log(`  member ${i + 1} (${m.a}→${m.b}, ${n(m.length, 0)} mm, ${m.material}): ${n(m.axial, 0)} N axial, ${n(m.max_stress, 1)} MPa, ×${n(m.safety)}${m.buckling_factor != null ? `, buckles ×${n(m.buckling_factor)}` : ""}`));
      for (const [node, f] of r.reactions) console.log(`  support at joint ${node}: ${v3(f.slice(0, 3))} N, ${v3(f.slice(3))} N·mm`);
      for (const x of r.notes) console.log(`  ! ${x}`);
    } else if (["probe", "section", "path", "csv"].includes(command)) {
      // forma probe <key> x,y,z · section <key> x,y,z nx,ny,nz · path <key> x,y,z x,y,z [points] · csv <key>
      const key = id;
      if (command === "csv") process.stdout.write(await (await get(`/results/${key}/csv`)).text());
      else if (command === "probe") {
        const p = await (await get(`/results/${key}/probe?at=${rest[0]}`)).json();
        console.log(`at ${v3(p.at)}: von Mises ${n(p.von_mises, 2)} MPa, principal ${v3(p.principal)} MPa, stress [xx yy zz xy yz zx] ${v3(p.stress)}, moves ${v3(p.displacement.map((x) => x * 1000))} µm, safety ×${n(p.safety)}${p.plastic_strain != null ? `, plastic strain ${n(p.plastic_strain, 5)}` : ""}`);
      } else if (command === "section") {
        const s = await (await get(`/results/${key}/section?point=${rest[0]}&normal=${rest[1]}`)).json();
        console.log(`section through ${v3(s.point)} square to ${v3(s.normal)}: ${s.cells.length} cubes, ${n(s.area, 1)} mm² cut, up to ${n(s.max, 1)} MPa, force across ${v3(s.force)} N`);
      } else {
        const p = await (await get(`/results/${key}/path?a=${rest[0]}&b=${rest[1]}&points=${rest[2] ?? 21}`)).json();
        console.log("distance_mm,von_mises_mpa,dx_um,dy_um,dz_um");
        for (const s of p.samples) console.log(`${n(s.distance, 2)},${n(s.probe.von_mises, 2)},${s.probe.displacement.map((x) => n(x * 1000, 2)).join(",")}`);
      }
    } else {
      if (!id) throw new Error(`say which shape: forma ${command} <shape id> [--part Name]`);
      q.set("kind", command);
      if (count) q.set("count", count);
      if (resolutions) q.set("resolutions", resolutions);
      const res = await get(`/simulate/${id}?${q}`, { method: "POST" });
      if (command === "inp") process.stdout.write(await res.text());
      else {
        const r = await res.json();
        if (command === "stress" || command === "fatigue") {
          const x = r.result;
          console.log(`${x.safety >= 1 ? "✓" : "✗"} ${r.material}: safety ×${n(x.safety)} against yield (${n(x.max_stress, 1)} MPa at ${v3(x.max_stress_at)}, yield ${n(x.yield_strength, 0)} MPa); moves ${n(x.max_displacement, 4)} mm at most (at ${v3(x.max_displacement_at)}); ${x.elements} cubes of ${n(x.cell, 2)} mm`);
          for (const p of x.parts ?? []) console.log(`  ${p.name}: ${n(p.max_stress, 1)} MPa, ${n(p.max_displacement, 4)} mm, ×${n(p.safety)}${p.floating ? " (not held: slides away)" : ""}`);
          for (const rc of x.reactions ?? []) console.log(`  ${rc.name} holds with ${v3(rc.force)} N, ${v3(rc.moment)} N·mm about ${v3(rc.at)}`);
          for (const b of x.bolts ?? []) console.log(`  bolt in ${b.part}: preload ${n(b.preload, 0)} N, now ${n(b.force, 0)} N`);
          if (x.plastic) console.log(`  yielded: ${x.plastic.yielded} cubes, plastic strain up to ${n(x.plastic.max_strain, 5)} at ${v3(x.plastic.at)}${x.plastic.settled ? "" : " (not settled)"}`);
          if (r.fatigue) console.log(`  fatigue: Goodman ×${n(r.fatigue.min_factor)} at ${v3(r.fatigue.at)}, life ${r.fatigue.life === null || r.fatigue.life > 1e15 ? "unlimited" : r.fatigue.life.toExponential(2) + " cycles"} (σa ${n(r.fatigue.amplitude, 1)}, σm ${n(r.fatigue.mean, 1)} MPa, Se ${n(r.fatigue.strength, 1)} MPa)`);
          for (const t of [...x.notes, ...(r.fatigue?.notes ?? [])]) console.log(`  ! ${t}`);
          console.log(`  result key ${r.key} (forma probe|section|path|csv ${r.key} …)`);
          failed = x.safety < 1;
        } else if (command === "modes") {
          r.modes.forEach((m, i) => console.log(`  mode ${i + 1}: ${n(m.frequency, 1)} Hz (${n(m.frequency * 60, 0)} rpm), mostly ${m.direction}`));
          console.log(`  mass ${n(r.mass, 4)} kg${r.prestress ? ", prestressed" : ""}`);
          for (const t of r.notes) console.log(`  ! ${t}`);
        } else if (command === "buckle") {
          r.modes.forEach((m, i) => console.log(`  buckling mode ${i + 1}: ×${n(m.factor)} the loads, mostly ${m.direction}`));
          console.log(`  largest stress ${n(r.max_stress, 1)} MPa (yields at ×${n(r.yield_factor)})`);
          for (const t of r.notes) console.log(`  ! ${t}`);
        } else if (command === "heat") {
          if (r.times) console.log(`  after ${n(r.times.at(-1), 1)} s: ${n(r.last.min, 1)}–${n(r.last.max, 1)} °C (hottest through time ${n(Math.max(...r.max), 1)} °C)`);
          else console.log(`  ${n(r.min, 1)}–${n(r.max, 1)} °C; ${n(r.heat_in, 2)} W in, ${n(r.to_holds, 2)} W to held faces, ${n(r.to_air, 2)} W to air`);
          for (const t of r.notes) console.log(`  ! ${t}`);
        } else if (command === "converge") {
          console.log("  resolution  cube_mm  cubes   max_stress_MPa  change  max_move_mm  change  seconds");
          for (const row of r.rows) console.log(`  ${String(row.resolution).padEnd(10)}  ${n(row.cell, 3).padEnd(7)}  ${String(row.elements).padEnd(6)}  ${n(row.max_stress, 2).padEnd(14)}  ${row.stress_change == null ? "—     " : n(row.stress_change * 100, 1) + " %"}  ${n(row.max_displacement, 5).padEnd(11)}  ${row.displacement_change == null ? "—     " : n(row.displacement_change * 100, 1) + " %"}  ${n(row.seconds, 1)}`);
          if (r.displacement_limit != null) console.log(`  movement extrapolated to fine cubes: ${n(r.displacement_limit, 5)} mm`);
          for (const t of r.notes) console.log(`  ! ${t}`);
          failed = !r.converged;
        }
      }
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
  if (action === "color" || action === "camera") value = arg;
  else if (arg) {
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
  console.error("usage: forma.mjs run steps.json | commit <op> <json> | stock [search] | get <id> | tree | list | check | ops | view <action> [what]  [--project name] [--url ws://…]");
  failed = true;
}

engine.close();
process.exit(failed ? 1 : 0);
