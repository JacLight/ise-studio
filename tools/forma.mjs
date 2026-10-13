#!/usr/bin/env node
// Talk to a running Forma engine from the command line, through the same API
// the UI and the AI use.
//
//   node tools/forma.mjs run steps.json [--project name] [--url ws://127.0.0.1:7420/ws]
//   node tools/forma.mjs check
//   node tools/forma.mjs gears <id> [torque N·m] [rpm]   turn a gear train through a full turn; tooth strength with a load
//   node tools/forma.mjs motion <joint id> [steps] [--set '{"Elbow":40}']   sweep a joint through its limits: what moves, closest approaches, clashes, where it binds
//   node tools/forma.mjs fit <assembly or instance id | project> [--motion]   do screws, inserts, nuts, bearings and joined parts fit (and, with --motion, every joint's sweep)
//   node tools/forma.mjs check-geometry <id> [part]   the exact kernel's check of each part: valid, self-intersections, tolerances
//   node tools/forma.mjs surfaces <id> [part] [--pull x,y,z] [--min deg]   surface analysis: continuity across every seam (gap, crease, curvature jump), tightest bend, draft
//   node tools/forma.mjs bom <assembly id | project> [qty]   the bill of materials as CSV: what is made (and how), what is bought, totals
//   node tools/forma.mjs set-study <shape id> study.json [--part X]   save a part's stress/heat study (faces as { normal, near: [x, y, z] })
//   node tools/forma.mjs stress|fatigue|modes|buckle|heat|converge|inp <shape id> [--part X] [--count N] [--resolutions 32,48,64]
//                                             run the part's saved study (inp: a CalculiX deck on stdout)
//   node tools/forma.mjs probe <key> x,y,z · section <key> x,y,z nx,ny,nz · path <key> x,y,z x,y,z [n] · csv <key>
//                                             read a stress result back (the key a stress run prints)
//   node tools/forma.mjs frame frame.json     a frame of beams (joints, members with sections, supports, loads)
//   node tools/forma.mjs plate <shape id> plate.json [--part X]   a flat sheet part as a plate (edges clamped or simply held, pressure)
//   node tools/forma.mjs materials [search]   the material library with sources, and the project's materials
//   node tools/forma.mjs dof <assembly | top>   what can still move (and how), mobility, redundant and conflicting joints
//   node tools/forma.mjs interference <assembly | instance | top> [clearance mm]   parts that overlap (volume, where), touch, or come close
//   node tools/forma.mjs exploded <view>      a saved exploded view's steps and trails
//   node tools/forma.mjs study <study> [fps] [--out frames.json]   play a motion study: what moves, clashes, binding; frames to a file
//   node tools/forma.mjs robot <tip> [fk '{"Shoulder":30}' | ik x,y,z [--axis 0,0,-1] | reach [samples] | path '[{"pose":"home"},{"to":[x,y,z],"linear":true}]'] [--tool connector]
//   node tools/forma.mjs stock [search]       the stock library: families, or the sizes matching (M3×12, 608, servo)
//   node tools/forma.mjs render [--width 1920] [--height 1080] [--samples 256] [--out f.png]   a photoreal PNG of the model from the open view (headless Chrome if none is open)
//   node tools/forma.mjs flat [id | project] [part] [--dxf f.dxf] [--svg f.svg] [--csv f.csv]   sheet-metal parts: flat size, bend table, checks; the laser DXF
//   node tools/forma.mjs pcb parts|add|footprint|verify-footprints|gerber-check|library …   KiCad library parts, footprint and Gerber checks
//   node tools/forma.mjs pcb calc|panel|gerbview|glb …   board calculators, panelization, the Gerber viewer, the board as glTF
//   node tools/forma.mjs pcb route|drag|cleanup|optimize …   the interactive router: pad to pad (walkaround/shove), drag a track or via, tidy copper
//   node tools/forma.mjs pcb import <file> [--format f] [--name n] [--board id] [companions…]   a board/schematic from KiCad, Eagle, EasyEDA, Altium, CADSTAR, Fabmaster, PADS, gEDA
//   node tools/forma.mjs pcb exports <id> | export <id> <file name | kicad_sch | ipc2581 | odb | d356 | …> [--out path]   files for other EDA tools
//   node tools/forma.mjs sch export <schematic> netlist|bom|svg|pdf [--format kicad|orcad|cadstar|spice] [--out f]   schematic files; sch find|erc|buses|sheets <schematic>
//   node tools/forma.mjs pcb lib|logo|assign|variant|block …   project/global libraries, wizards, logos, footprint assignment, variants, design blocks
//   node tools/forma.mjs pcb drc|setup|stackup|rules|teardrops <board> …   checks, stackup (2–32 layers), custom rules (.kicad_dru), teardrops
//   node tools/forma.mjs pcb netclass|pair|group|lengths|tune|route-pair|pad-to-die <board> …   impedance targets (widths per layer), diff pairs, length groups, meanders
//   node tools/forma.mjs pcb text|draw|image|keepout|stitch|fence <board> …   board graphics, keepouts, via stitching and fences
//   node tools/forma.mjs pcb align|distribute|move|array|flip|swap <items…> [--board id] · pcb channel <board> --source … --to …   placement, multichannel
//   node tools/forma.mjs simulate <schematic> [--tran 30ms] [--expect "min(V(3V3), burst) > 3.0"] [--save name]   ngspice on a schematic
//   node tools/forma.mjs list                 every entity, one line each
//   node tools/forma.mjs tree                 what is placed, as a tree (instances, parts)
//   node tools/forma.mjs get <id>             one entity in full (program source, params, results)
//   node tools/forma.mjs commit <operation> '<json>'   one command (same $file inlining)
//   node tools/forma.mjs image add <file> …   store pictures (PNG, JPEG) or fonts (TTF, OTF) for programs; prints each id
//   node tools/forma.mjs image list           the pictures and fonts in the project
//   node tools/forma.mjs image profile <file|id> --height mm   a photo's object measured: its outline as a profile to revolve
//   node tools/forma.mjs look [what] [--view iso|front|top|…|az,el] [--size 1000x750] [--box x0,y0,z0,x1,y1,z1] [--out file.png]
//        a picture of the model (or one thing in it) drawn by the engine, saved to look/…png
//   node tools/forma.mjs canvas add <photo|picture id> [--plane front|back|left|right|top|bottom] [--object WxH --align 'left bottom'
//        | --calibrate u1,v1,u2,v2,mm | --width mm | --scale mm/px] [--at x,y,z] [--anchor u,v] [--rotate deg | --level u1,v1,u2,v2]
//        [--flip] [--aspect k] [--crop l,t,r,b] [--perspective u1,v1,…,u4,v4,WxH] [--opacity 0.5] [--through] [--name N]
//        | list | update <canvas> [same] [--reset crop,…] | remove <canvas>   reference pictures placed and calibrated in the model
//   node tools/forma.mjs canvas measure <canvas> [--every mm] [--channel brightness --threshold 0.35] [--region l,t,r,b] [--all]   the outline in mm: bounds, spans at stations
//   node tools/forma.mjs canvas match [<canvas>|all] [--of <thing>] [--out f.png]  ·  look --overlay <canvas> [what]
//        the model drawn square to the canvas over its photo: silhouette IoU, outline distance, regions of extra/missing material
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
//   node tools/forma.mjs bim [id|project]     the building: levels and what stands on each, spaces, zones, grids
//   node tools/forma.mjs schedule [id|project] [--class wall,door] [--level name] [--by level,class] [--csv] [--psets] [--out f.csv]
//   node tools/forma.mjs ifc export [id|project] [file.ifc]  ·  ifc import <file.ifc>
//   node tools/forma.mjs plan [id|project] --level name [--cut 1200] | --section x|y <mm> [--looking east] | --elevation south
//        [--sheet A3] [--scale 100] [--pdf file.pdf] [--dxf file.dxf]
//   node tools/forma.mjs sheet <sheet>        a spreadsheet as a grid: what each cell shows, aliases, errors
//   node tools/forma.mjs draft                draft objects, dimensions with their values, layers, working planes, arrays
//   node tools/forma.mjs import <file> [--name N] [--as sheet|parameters|configurations] [--target id] [--sheet name]
//        STEP/IGES (exact, also .stpz, .brep) and STL/OBJ/PLY/glTF/GLB/3MF (mesh) become a shape, DXF/SVG a sketch,
//        .scad a live parametric shape, .csv/.xlsx a sheet, parameter set or configurations, .forma a new project
//   node tools/forma.mjs export <id|name|project> <file.ext> | --format step|iges|stl|obj|ply|glb|3mf|dxf|stpz|brep|scad|zip|csv|xlsx [--part P] [--out path]
//   node tools/forma.mjs bundle [file.forma]   the whole project (history, assets) as one file
//   node tools/forma.mjs insert --from <project> [ids or names…] [--all] [--copy] [--auto] [--at x,y,z | --origin] [--list] [--dry-run]
//        items of another project (parts, assemblies, sub-assemblies, boards… or --all), linked to follow it unless --copy
//   node tools/forma.mjs links [update [id] | break <id> | auto <id> on|off]   this project's links: up to date or not; take, cut, automate
//   node tools/forma.mjs scan add <file> [--source colmap] | list | info | fit <scan> [kind] [--near x,y,z --within r] | detect
//        | downsample | outliers | normals | reconstruct [--depth 7] | surface [--poles 8] | shape | export | remove
//                                            point clouds (XYZ/PLY/PCD/E57) and meshes: add, thin, fit primitives, rebuild surfaces
//   node tools/forma.mjs mesh check|repair|decimate|smooth|remesh|fill-holes|split|merge|section|curvature|segment|unwrap <scan> …
//        | mesh from <part> [--tolerance] [--angle] [--max-edge]   mesh evaluation and repair, reworking, sections, flattening
//   node tools/forma.mjs inspect <scan> <part> --tolerance mm [--align icp|best] [--required 0.99] [--require]
//                                            deviation of a scan from its CAD part: statistics, pass/fail, as a requirement
//   node tools/forma.mjs scan photos <folder | photos.zip | photo …> [--engine auto|object-capture|colmap] [--detail preview|reduced|medium|full] [--mask] [--name N] [--detach]
//        photogrammetry: photos → a sized, levelled scan and a cleaned one, with quality (photos used, reprojection, scale, coverage)
//   node tools/forma.mjs scan engines | jobs | cancel <job> | markers [--letter] [--out f.pdf] | quality <scan>
//   node tools/forma.mjs scan scale <scan> --from x,y,z --to x,y,z --distance mm [--pick mm] | --factor f  ·  scan level <scan>
//   node tools/forma.mjs scan render-photos <folder> [--count 40] [--rings 25,50] [--distance mm] [--fov 45] [--sheet]   a test photo set of the model
//
// steps.json is an array of { "operation": "...", "parameters": { ... } }.
// Any string "$N.path" in parameters is replaced by step N's output at that
// path (e.g. "$0.id"), and { "$file": "path.js" } inlines a file's text (paths
// are relative to steps.json; several files can be joined with an array).

import fsModule, { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { homedir } from "node:os";
import { basename, dirname, extname, join, resolve } from "node:path";
import { request as httpRequest } from "node:http";
import { pcbLib } from "./forma-pcblib.mjs";
import { pcbBoard, BOARD_SUBS } from "./forma-pcbboard.mjs";
import { pcbSignal, SIGNAL_SUBS } from "./forma-pcbsignal.mjs";

// Every file a command writes (--pdf out/drawing.pdf, --dxf flat/base.dxf…)
// gets its folder made first, wherever the command imports writeFileSync from.
{
  const write = fsModule.writeFileSync;
  fsModule.writeFileSync = (path, ...rest) => {
    if (typeof path === "string" && dirname(path) !== ".") fsModule.mkdirSync(dirname(resolve(path)), { recursive: true });
    return write(path, ...rest);
  };
  syncBuiltinESMExports();
}

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

// Names work wherever an id does: a shape's, instance's or assembly's name
// (any case) stands for its id in the commands that read one part.
if (["check-geometry", "surfaces", "drawing", "flat"].includes(command) && args[1] && !/^[0-9a-f]{8}-[0-9a-f]{4}-/i.test(args[1]) && args[1] !== "project") {
  const snap = await engine.snapshot;
  const want = args[1].toLowerCase();
  const nameOf = (e) => {
    const c = e.components ?? {};
    return c["assembly.instance"]?.name ?? c["shape.program"]?.name ?? c["assembly.definition"]?.name;
  };
  const hits = snap.entities.filter((e) => nameOf(e)?.toLowerCase() === want);
  if (hits.length === 1) args[1] = hits[0].id;
  else if (hits.length > 1) {
    console.error(`✗ “${args[1]}” names ${hits.length} things: use an id (${hits.map((e) => e.id).join(", ")})`);
    process.exit(1);
  }
}

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
} else if (command === "render") {
  // A photoreal PNG of the model as the person's view shows it; with no view open, a headless browser renders it.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const opt = (name, fallback) => {
    const i = args.indexOf(`--${name}`);
    return i >= 0 ? args.splice(i, 2)[1] : fallback;
  };
  const width = Number(opt("width", 1920));
  const height = Number(opt("height", Math.round((width * 9) / 16)));
  const samples = Number(opt("samples", 256));
  const out = opt("out", `render-${width}x${height}.png`);
  const q = new URLSearchParams({ ...(project ? { project } : {}), width, height, samples });
  // A render can take minutes: no header timeout (fetch gives up after five).
  const { request } = await import("node:http");
  const ask = () =>
    new Promise((done, fail) => {
      const r = request(`${http}/render?${q}`, { method: "POST" }, (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const body = Buffer.concat(chunks);
          done({ status: res.statusCode, ok: res.statusCode === 200, text: async () => body.toString(), arrayBuffer: async () => body });
        });
      });
      r.on("error", fail);
      r.end();
    });
  let res = await ask();
  let chrome = null;
  if (res.status === 409) {
    // No view open: start a headless browser on the project and ask again once it has loaded.
    const { spawn } = await import("node:child_process");
    const { existsSync, mkdtempSync } = await import("node:fs");
    const { tmpdir } = await import("node:os");
    const paths = [process.env.CHROME, "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium", "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"].filter(Boolean);
    const exe = paths.find((p) => existsSync(p));
    if (!exe) {
      console.error("✗ no view is open and no Chrome or Chromium to render in headless (set CHROME to its path)");
      process.exit(1);
    }
    const page = `${http}/${project ? `?project=${encodeURIComponent(project)}` : ""}`;
    chrome = spawn(exe, ["--headless=new", `--user-data-dir=${mkdtempSync(`${tmpdir()}/forma-render-`)}`, "--enable-unsafe-swiftshader", `--window-size=${Math.min(width, 1920)},${Math.min(height, 1080)}`, "--hide-scrollbars", page], { stdio: "ignore" });
    for (let i = 0; i < 30 && res.status === 409; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      res = await ask();
    }
  }
  if (!res.ok) {
    console.error(`✗ ${await res.text()}`);
    failed = true;
  } else {
    const png = Buffer.from(await res.arrayBuffer());
    writeFileSync(out, png);
    console.log(`✓ ${out}: ${width} × ${height} px, ${samples} samples, ${(png.length / 1024).toFixed(0)} KB${chrome ? " (rendered headless)" : ""}`);
  }
  chrome?.kill();
} else if (command === "flat") {
  // Sheet-metal flat patterns: every sheet part, or one with its bend table and checks; files on request.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const files = {};
  for (const ext of ["dxf", "svg", "csv"]) {
    const i = args.indexOf(`--${ext}`);
    if (i >= 0) files[ext] = args.splice(i, 2)[1];
  }
  const [, id = "project", part] = args;
  const q = new URLSearchParams({ ...(project ? { project } : {}), ...(part ? { part } : {}) });
  const res = await fetch(`${http}/sheet-metal/${id}?${q}`);
  if (!res.ok) {
    console.error(`✗ ${await res.text()}`);
    failed = true;
  } else if (id === "project") {
    const list = await res.json();
    if (!list.length) console.log("no sheet-metal parts (build them with sheet() or sheetProfile(); see the prelude)");
    for (const s of list) console.log(`${s.errors ? "✗" : "✓"} ${s.part} (${s.shape}, ${s.id}): ${s.thickness} mm ${s.material}, flat ${s.flat.join(" × ")} mm, ${s.bends} bends${s.errors ? `, ${s.errors} errors` : ""}${s.warnings ? `, ${s.warnings} warnings` : ""}`);
    failed = list.some((s) => s.errors);
  } else {
    const f = await res.json();
    const errors = f.checks.filter((c) => c.severity === "error");
    console.log(`${errors.length ? "✗" : "✓"} ${f.part}: ${f.thickness} mm ${f.material}, K ${f.k} at R${f.radius}; flat blank ${f.size.map((v) => v.toFixed(2)).join(" × ")} mm, ${(f.area / 100).toFixed(1)} cm², cut ${f.cut_length.toFixed(0)} mm in ${f.pierces} outline${f.pierces > 1 ? "s" : ""}`);
    if (f.bends.length) console.log("  order bend  dir   angle  R      K     BA      BD      length  from → to");
    for (const b of [...f.bends].sort((a, b) => a.order - b.order)) {
      console.log(`  ${String(b.order).padStart(5)} ${b.id.padEnd(5)} ${(b.dir + (b.hem ? "*" : "")).padEnd(5)} ${String(b.angle).padStart(5)}° ${String(b.radius).padEnd(6)} ${String(b.k).padEnd(5)} ${b.allowance.toFixed(3).padStart(7)} ${b.deduction.toFixed(3).padStart(7)} ${b.length.toFixed(1).padStart(7)}  ${b.from} → ${b.to}`);
    }
    if (f.bends.some((b) => b.hem)) console.log("  (* hem)");
    for (const c of f.checks) console.log(`  ${c.severity.padEnd(7)} ${c.code}: ${c.message}`);
    for (const [ext, path] of Object.entries(files)) {
      const r = await fetch(`${http}/sheet-metal/${id}?${new URLSearchParams({ ...Object.fromEntries(q), format: ext })}`);
      if (!r.ok) {
        console.error(`✗ ${ext}: ${await r.text()}`);
        failed = true;
        continue;
      }
      const bytes = Buffer.from(await r.arrayBuffer());
      writeFileSync(path, bytes);
      console.log(`  wrote ${path} (${bytes.length} bytes)`);
    }
    failed ||= errors.length > 0;
  }
} else if (["stress", "fatigue", "modes", "buckle", "heat", "converge", "inp", "probe", "section", "path", "csv", "frame", "materials", "set-study", "plate"].includes(command)) {
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
  // A study can take minutes: plain node:http, which (unlike fetch) never gives up waiting for the answer.
  const get = (path, init = {}) =>
    new Promise((done, fail) => {
      const req = httpRequest(`${http}${path}`, { method: init.method ?? "GET", headers: init.headers ?? {} }, (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const body = Buffer.concat(chunks);
          if (res.statusCode >= 400) return fail(new Error(body.toString() || `HTTP ${res.statusCode}`));
          done({ text: async () => body.toString(), json: async () => JSON.parse(body.toString()), arrayBuffer: async () => body });
        });
      });
      req.on("error", fail);
      req.setTimeout(0);
      if (init.body) req.write(init.body);
      req.end();
    });
  try {
    if (command === "set-study") {
      // forma set-study <shape id> study.json [--part X]: save a study (faces as { normal, near: [x, y, z] }).
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
    } else if (command === "plate") {
      // forma plate <shape id> plate.json [--part X]: a flat sheet as a plate ({ supports: [{ face: { normal, near }, hold: "clamped" | "simple" }], pressure, loads, resolution }).
      const body = { ...JSON.parse(readFileSync(rest[0], "utf8")), ...(part ? { part } : {}) };
      const r = await (await get(`/plate/${id}?${q}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })).json();
      const x = r.result;
      console.log(`${x.safety >= 1 ? "✓" : "✗"} ${r.material} sheet ${n(x.thickness, 2)} mm: sags ${n(x.max_sag, 4)} mm at most (at ${v3(x.max_sag_at)}), bending stress ${n(x.max_stress, 1)} MPa at most (at ${v3(x.max_stress_at)}), safety ×${n(x.safety)}`);
      for (const t of x.notes) console.log(`  ! ${t}`);
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
} else if (command === "check-geometry") {
  // The exact kernel's check of every part (as FreeCAD's Check geometry).
  const [, id, part] = args;
  if (!id) {
    console.error("✗ say which: forma check-geometry <id> [part]");
    process.exit(1);
  }
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = new URLSearchParams({ ...(project ? { project } : {}), ...(part ? { part } : {}) });
  const res = await fetch(`${http}/check-geometry/${id}?${q}`);
  if (!res.ok) {
    console.error(`✗ ${await res.text()}`);
    failed = true;
  } else {
    const r = await res.json();
    console.log(`${r.ok ? "✓" : "✗"} ${r.name}: ${r.parts.length} part${r.parts.length === 1 ? "" : "s"} checked`);
    for (const p of r.parts) {
      const c = p.report;
      const t = c.max_tolerance;
      console.log(`  ${c.valid && c.boolean_ready ? "ok  " : "FAIL"} ${p.name}: ${c.valid ? "valid" : "INVALID"}, ${c.boolean_ready ? "ready for booleans" : "booleans may fail"}; ${c.solids} solid${c.solids === 1 ? "" : "s"}, ${c.faces} faces, ${c.edges} edges${c.free_edges ? `, ${c.free_edges} FREE EDGES (not closed)` : ""}; volume ${+c.volume.toFixed(3)} mm³; largest tolerance ${t.vertex.toExponential(1)} mm (vertex), ${t.edge.toExponential(1)} (edge), ${t.face.toExponential(1)} (face)${c.complete === false ? '; the self-intersection check stopped after 20 s (unfinished)' : ''}`);
      for (const x of c.problems) console.log(`       ${x.shape}: ${x.status}`);
      for (const x of c.faults) console.log(`       ${x.check}${x.shapes ? ` (${x.shapes})` : ""}${x.distance ? `, off by ${x.distance} mm` : ""}`);
    }
    failed = !r.ok;
  }
} else if (command === "surfaces") {
  // Surface analysis of every part, exactly: G0/G1/G2 across each seam, curvature, draft.
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const pull = flag("--pull");
  const min = flag("--min");
  const [, id, part] = args;
  if (!id) {
    console.error("✗ say which: forma surfaces <id> [part] [--pull x,y,z] [--min deg]");
    process.exit(1);
  }
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = new URLSearchParams({ ...(project ? { project } : {}), ...(part ? { part } : {}), ...(pull ? { pull } : {}), ...(min ? { min } : {}) });
  const res = await fetch(`${http}/surfaces/${id}?${q}`);
  if (!res.ok) {
    console.error(`✗ ${await res.text()}`);
    failed = true;
  } else {
    const r = await res.json();
    const f = (v, d = 4) => (v == null ? "-" : Math.abs(v) < 1e-4 && v !== 0 ? v.toExponential(1) : +v.toFixed(d));
    console.log(`${r.name}: ${r.parts.length} part${r.parts.length === 1 ? "" : "s"}`);
    for (const p of r.parts) {
      const b = p.body, c = p.continuity;
      console.log(`  ${p.name}: ${b.solids ? `${b.solids} solid` : b.closed ? "closed shell" : `open sheet (${b.free_edges} free edges)`}, ${b.faces} faces`);
      console.log(`    seams: ${c.edges} shared edges, ${c.smooth} tangent (G1); worst gap ${f(c.max.g0)} mm, crease ${f(c.max.g1, 3)}°, curvature jump ${f(c.max.g2)} 1/mm`);
      for (const e of c.worst.filter((e) => e.g1 >= 0.1).slice(0, 5)) console.log(`      edge ${e.edge} (faces ${e.faces.join("/")}, ${f(e.length, 1)} mm): crease ${f(e.g1, 2)}°, curvature jump ${f(e.g2)} 1/mm`);
      console.log(`    tightest bend: radius ${p.curvature.min_radius == null ? "flat" : `${f(p.curvature.min_radius, 2)} mm`}`);
      console.log(`    draft along [${p.draft.pull.map((v) => f(v, 3)).join(", ")}]: ${p.draft.under.length ? `faces ${p.draft.under.join(", ")} within ${p.draft.min}° of square (need draft)` : `every face has ${p.draft.min}°+`}`);
    }
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
    } else if (sub === "calc") {
      // Board calculators: no name lists them; key=value words set inputs.
      const [, , id, ...rest] = args;
      if (!id) {
        const r = await get("/pcb/calc");
        for (const c of r.calculators) console.log(`${c.id.padEnd(11)} ${c.title}: ${c.inputs.map((i) => `${i.key}=${i.default}${i.unit ? ` ${i.unit}` : ""}${i.options ? ` (${i.options.join("|")})` : ""}`).join(", ") || "no inputs"}`);
      } else {
        const params = Object.fromEntries(rest.filter((w) => w.includes("=")).map((w) => [w.slice(0, w.indexOf("=")), w.slice(w.indexOf("=") + 1)]));
        const r = await get(`/pcb/calc/${encodeURIComponent(id)}?${q(params)}`);
        console.log(`${r.title} (${Object.entries(r.inputs).map(([k, v]) => `${k}=${v}`).join(" ")})`);
        for (const x of r.results) console.log(`  ${String(x.label).padEnd(40)} ${x.value} ${x.unit}`);
        if (r.bands) console.log(`  bands: ${r.bands.map((b) => b.colour).join(", ")}`);
        if (r.values) console.log(`  ${r.values.join("  ")}`);
        for (const t of [r.table, r.levels].filter(Boolean)) {
          console.log(`  ${t.columns.join(" | ")}`);
          for (const row of t.rows) console.log(`  ${row.join(" | ")}`);
        }
        for (const n of r.notes ?? []) console.log(`  · ${n}`);
        if (r.source) console.log(`  (${r.source})`);
      }
    } else if (sub === "panel") {
      // A panel of the board: layout and checks; --out writes its fab package.
      const out = flag("--out");
      const fab = flag("--fab");
      const [, , board, ...rest] = args;
      if (!board) throw new Error("usage: forma pcb panel <board id> [rows=2 cols=3 separation=mousebites|vscore|tabs frame=rails-tb|rails-lr|frame|none gap=2 tab_width=3 tabs=1 rail=5 fiducials=true tooling=true …] [--fab jlcpcb] [--out panel.zip]");
      const params = { ...Object.fromEntries(rest.filter((w) => w.includes("=")).map((w) => [w.slice(0, w.indexOf("=")), w.slice(w.indexOf("=") + 1)])), ...(fab ? { fab } : {}) };
      const r = await get(`/pcb/${board}/panel?${q(params)}`);
      const p = r.panel;
      const bad = r.readback.filter((x) => !x.ok);
      console.log(`${p.problems.length || bad.length ? "✗" : "✓"} ${p.spec.rows} × ${p.spec.cols} panel of ${r.board.name}: ${p.size.map((v) => v.toFixed(2)).join(" × ")} mm, ${p.spec.separation}, frame ${p.spec.frame}`);
      console.log(`  ${p.tabs.length} tabs, ${p.bites.length} mouse bites, ${p.vcuts.length} V-cuts, ${p.fiducials.length} fiducials, ${p.tooling.length} tooling holes; substrate ${p.area.toFixed(1)} mm² in ${p.outline.length} contour(s)`);
      for (const x of p.problems) console.log(`  ✗ ${x}`);
      for (const x of p.notes) console.log(`  · ${x}`);
      for (const x of r.readback) console.log(`  ${x.ok ? "✓" : "✗"} ${x.file}: ${x.ok ? (x.holes != null ? `${x.holes} holes` : `${x.objects} objects (${x.function})`) : x.error}`);
      if (out) {
        const { writeFileSync } = await import("node:fs");
        const res = await fetch(`${http}/pcb/${board}/panel.zip?${q(params)}`);
        if (!res.ok) throw new Error(await res.text());
        writeFileSync(out, Buffer.from(await res.arrayBuffer()));
        console.log(`  wrote ${out}`);
      }
      failed = p.problems.length > 0 || bad.length > 0;
    } else if (sub === "gerbview") {
      // Read any Gerber/Excellon files or zips (or a board's own package with --board): layers, apertures, nets;
      // --compare older files to see what changed; --pdf writes the layers as a PDF.
      const { writeFileSync } = await import("node:fs");
      const compare = flag("--compare");
      const pdf = flag("--pdf");
      const board = flag("--board");
      const paths = args.slice(2);
      const upload = (list) => list.map((f) => ({ name: f.split("/").pop(), data: readFileSync(f).toString("base64") }));
      let files;
      if (board) {
        const zip = await fetch(`${http}/pcb/${board}/fab.zip?${q()}`);
        if (!zip.ok) throw new Error(await zip.text());
        files = [{ name: "board.zip", data: Buffer.from(await zip.arrayBuffer()).toString("base64") }];
      } else {
        if (!paths.length) throw new Error("usage: forma pcb gerbview <files or zip…> | --board <id> [--compare <older files or zip>] [--pdf out.pdf]");
        files = upload(paths);
      }
      const post = async (path, body) => {
        const res = await fetch(`${http}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
        if (!res.ok) throw new Error(await res.text());
        return res;
      };
      const toBoard = flag("--to-board");
      if (toBoard) {
        // Steps that rebuild a board (outline, tracks, vias) from the files: run them with ./forma run.
        const s = await (await post("/pcb/gerbview", { files, to_board: flag("--name") ?? "From Gerbers" })).json();
        writeFileSync(toBoard, JSON.stringify(s.steps, null, 1));
        console.log(`wrote ${toBoard}: ${s.steps.length} steps (run them with ./forma run ${toBoard})`);
        for (const n of s.notes) console.log(`  · ${n}`);
      }
      const r = await (await post("/pcb/gerbview", { files, summary: true, ...(compare ? { against: upload(compare.split(",")) } : {}) })).json();
      console.log(`${r.layers.length} layers, ${r.size.join(" × ")} mm, ${r.nets} nets, ${r.parts} parts${r.job ? ", job file" : ""}`);
      for (const l of r.layers) console.log(`  ${l.function.padEnd(22)} ${l.file}${l.guessed ? " (layer guessed from its name)" : ""}: ${l.kind === "drill" ? l.tools.map(([t, d, h, s]) => `T${t} ø${d}×${h}${s ? `+${s} slots` : ""}`).join(", ") : `${l.objects} objects, ${l.apertures} apertures, ${l.nets} nets`}`);
      for (const [f, e] of r.unreadable) console.log(`  ✗ ${f}: ${e}`);
      if (r.other.length) console.log(`  other files: ${r.other.join(", ")}`);
      if (r.compare) {
        console.log("Compared with the earlier files:");
        for (const d of r.compare) {
          console.log(`  ${d.same ? "=" : "≠"} ${d.function.padEnd(22)} ${d.before ?? "(none)"} → ${d.after ?? "(none)"}${d.same ? "" : d.holes_added.length || d.holes_removed.length ? `: ${d.holes_added.length} holes added, ${d.holes_removed.length} removed` : `: +${d.added} mm², −${d.removed} mm²`}`);
          for (const h of [...d.holes_added.map((x) => `+ ${x}`), ...d.holes_removed.map((x) => `− ${x}`)].slice(0, 8)) console.log(`      ${h}`);
        }
      }
      if (pdf) {
        const looks = r.layers.filter((l) => !l.function.startsWith("Paste")).map((l) => ({ file: l.file }));
        const res = await post("/pcb/gerbview/pdf", { files, looks, background: "#ffffff" });
        writeFileSync(pdf, Buffer.from(await res.arrayBuffer()));
        console.log(`wrote ${pdf}`);
      }
      failed = r.unreadable.length > 0;
    } else if (sub === "glb") {
      // The populated board as binary glTF.
      const board = args[2];
      if (!board) throw new Error("usage: forma pcb glb <board id> [--out board.glb]");
      const out = flag("--out") ?? "board.glb";
      const res = await fetch(`${http}/pcb/${board}/board.glb?${q()}`);
      if (!res.ok) throw new Error(await res.text());
      const { writeFileSync } = await import("node:fs");
      const bytes = Buffer.from(await res.arrayBuffer());
      writeFileSync(out, bytes);
      console.log(`wrote ${out} (${(bytes.length / 1024).toFixed(0)} kB)`);
    } else if (sub === "import") {
      // A board and/or schematic from another EDA tool, told apart by content.
      const format = flag("--format");
      const name = flag("--name");
      const board = flag("--board");
      const path = args[2];
      if (!path) throw new Error("usage: forma pcb import <file> [--format eagle_brd|altium_pcb|…] [--name …] [--board <id>] [companion files…]");
      const { basename } = await import("node:path");
      const binary = /\.(pcbdoc|schdoc|epro|zip)$/i.test(path);
      const bytes = readFileSync(path);
      const files = {};
      for (const extra of args.slice(3)) files[basename(extra)] = readFileSync(extra, "utf8");
      const params = { file: basename(path), ...(binary ? { data: bytes.toString("base64") } : { text: bytes.toString("utf8") }), ...(format ? { format } : {}), ...(name ? { name } : {}), ...(board ? { board } : {}), ...(Object.keys(files).length ? { files } : {}) };
      const r = await engine.commit("pcb.import", params);
      if (!r.ok) throw new Error(`${r.error.code} ${r.error.message}`);
      const o = r.value.output;
      console.log(`✓ ${o.format}: ${o.board ? `board ${o.board.id} "${o.board.name}" (${o.board.components} parts, ${o.board.tracks} tracks, ${o.board.vias} vias, ${o.board.zones} pours)` : ""}${o.board && o.schematic ? "; " : ""}${o.schematic ? `schematic ${o.schematic.id} (${o.schematic.symbols} symbols, ${o.schematic.wires} wires)` : ""} (rev ${r.value.revision})`);
      for (const n of [...(o.notes ?? []), ...(o.board?.notes ?? []), ...(o.schematic?.notes ?? [])]) console.log(`  note: ${n}`);
    } else if (sub === "exports") {
      // What a board or schematic can be written as.
      const id = args[2];
      if (!id) throw new Error("usage: forma pcb exports <board or schematic id>");
      const r = await get(`/pcb/${id}/exports?${q()}`);
      for (const f of r.files) console.log(`  ${f.name.padEnd(36)} ${f.what}`);
    } else if (sub === "export") {
      // Write one of those files here: by its name, or by what it is (kicad_sch, ipc2581, odb, d356, …).
      const out = flag("--out");
      const [, , id, what] = args;
      if (!id || !what) throw new Error("usage: forma pcb export <board or schematic id> <file name | kicad_pcb | kicad_sch | kicad_pro | kicad_zip | net | xml_net | orcad | pads_net | allegro | cadstar_net | d356 | ipc2581 | odb | gbrjob | drill_map | drill_map_pdf | drill_report | svg | pdf | wrl | glb | pos | pos_csv | x3_top | x3_bottom | step | idf> [--out path]");
      const list = (await get(`/pcb/${id}/exports?${q()}`)).files.map((f) => f.name);
      const suffix = { kicad_pcb: ".kicad_pcb", kicad_sch: ".kicad_sch", kicad_pro: ".kicad_pro", kicad_zip: "-kicad.zip", net: ".net", xml_net: "-netlist.xml", orcad: "-orcad.net", pads_net: "-pads.net", allegro: "-allegro.net", cadstar_net: "-cadstar.frp", d356: ".d356", ipc2581: "-ipc2581.xml", odb: "-odb.zip", gbrjob: "-job.gbrjob", drill_map: "-drill-map.gbr", drill_map_svg: "-drill-map.svg", drill_map_pdf: "-drill-map.pdf", drill_report: "-drill-report.txt", svg: "-board.svg", pdf: "-board.pdf", wrl: ".wrl", glb: ".glb", pos: "-both.pos", pos_csv: "-pos.csv", x3_top: "-top-pnp.gbr", x3_bottom: "-bottom-pnp.gbr", step: ".step", idf: ".emn" }[what];
      const name = list.includes(what) ? what : list.find((n) => suffix && n.endsWith(suffix));
      if (!name) throw new Error(`no export “${what}”; this one has: ${list.join(", ")}`);
      const isSchematic = !(await fetch(`${http}/pcb/${id}?${q()}`)).ok;
      const res = await fetch(`${http}/pcb/${isSchematic ? "schematic/" : ""}${id}/file/${encodeURIComponent(name)}?${q({ download: "1" })}`);
      if (!res.ok) throw new Error(await res.text());
      const { writeFileSync } = await import("node:fs");
      const path = out ?? name;
      writeFileSync(path, Buffer.from(await res.arrayBuffer()));
      console.log(`✓ ${path}`);
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
    } else if (["lib", "logo", "assign", "variant", "block"].includes(sub)) {
      failed = await pcbLib(sub, args, { engine, get, http, q });
    } else if (SIGNAL_SUBS.includes(sub)) {
      failed = await pcbSignal(sub, args, { engine, get, http, q });
    } else if (BOARD_SUBS.includes(sub)) {
      failed = await pcbBoard(sub, args, { engine, get, q });
    } else {
      throw new Error("usage: forma pcb parts <words> [--footprints] | add <schematic> <Library:Symbol> [--footprint L:F] [--at x,y] | footprint <board> <Library:Name> | verify-footprints [names…] | gerber-check <board> | import <file> [--format f] | exports <id> | export <id> <name|kind> [--out path] | calc [name key=value…] | panel <board> [key=value…] [--out panel.zip] | gerbview <files…>|--board <id> [--compare old] [--pdf out.pdf] [--to-board steps.json] | glb <board> [--out f.glb] | library [update] | lib … | logo <image> | assign <schematic> | variant <schematic> … | block … | route <board> <from> <to> [--mode shove] | drag <board> <item> <x,y> | cleanup <board> | optimize <board>");
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else if (command === "sch") {
  // Schematic files and lookups: netlists, BOM, SVG/PDF plots, find, ERC with exclusion keys, buses.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = (extra = {}) => new URLSearchParams({ ...(project ? { project } : {}), ...extra });
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const fetchOk = async (path) => {
    const res = await fetch(`${http}${path}`);
    if (!res.ok) throw new Error(await res.text());
    return res;
  };
  const usage = "usage: forma sch export <schematic> netlist [--format kicad|orcad|cadstar|spice] | bom [--group Value,Footprint] [--fields Reference,Qty,Value,…] [--dnp] | svg [--sheet 1] | pdf  [--out file]\n       forma sch find <schematic> <text> · forma sch erc <schematic> [--all] · forma sch buses <schematic> · forma sch sheets <schematic>";
  try {
    const sub = file;
    const out = flag("--out");
    const write = async (body, binary) => {
      if (!out) return process.stdout.write(binary ? Buffer.from(body) : body);
      const { writeFileSync } = await import("node:fs");
      writeFileSync(out, binary ? Buffer.from(body) : body);
      console.log(`✓ wrote ${out} (${binary ? body.byteLength : body.length} bytes)`);
    };
    const id = args[2];
    if (!id && sub) throw new Error(usage);
    const detail = async () => (await (await fetchOk(`/pcb/schematic/${id}?${q()}`)).json()).schematic;
    if (sub === "export") {
      const what = args[3];
      if (what === "netlist") {
        await write(await (await fetchOk(`/pcb/schematic/${id}/netlist?${q({ format: flag("--format") ?? "kicad" })}`)).text());
      } else if (what === "bom") {
        const extra = {};
        const g = flag("--group");
        const f = flag("--fields");
        if (g !== null) extra.group = g;
        if (f !== null) extra.fields = f;
        if (args.includes("--dnp")) extra.dnp = "1";
        await write(await (await fetchOk(`/pcb/schematic/${id}/bom?${q(extra)}`)).text());
      } else if (what === "svg") {
        await write(await (await fetchOk(`/pcb/schematic/${id}/sheet/${encodeURIComponent(flag("--sheet") ?? "1")}.svg?${q()}`)).text());
      } else if (what === "pdf") {
        if (!out) throw new Error("a PDF needs --out file.pdf");
        await write(await (await fetchOk(`/pcb/schematic/${id}/schematic.pdf?${q()}`)).arrayBuffer(), true);
      } else throw new Error(usage);
    } else if (sub === "find") {
      const words = args.slice(3).join(" ").toLowerCase();
      if (!words) throw new Error("say what to find: forma sch find <schematic> <text>");
      const v = await detail();
      const hit = (s) => String(s ?? "").toLowerCase().includes(words);
      for (const s of v.symbols) if ([s.reference, s.value, s.footprint, s.mpn, ...Object.values(s.fields ?? {})].some(hit)) console.log(`symbol ${s.reference} ${s.value} (${s.footprint || "no footprint"}) sheet ${s.sheet}  id ${s.id}`);
      for (const l of v.labels) if (hit(l.text)) console.log(`${l.kind} label ${l.text} at ${l.at.join(",")} sheet ${l.sheet}  id ${l.id}`);
      for (const t of v.texts) if (hit(t.text)) console.log(`note “${t.text.split("\n")[0]}” sheet ${t.sheet}  id ${t.id}`);
      for (const n of v.nets) if (hit(n.name) || n.pins.some(hit)) console.log(`net ${n.name}${n.class ? ` [${n.class}]` : ""}: ${n.pins.join(" ")}`);
      for (const b of v.buses) if (hit(b.name) || b.members.some((m) => hit(m.member))) console.log(`bus ${b.name ?? "(unnamed)"} sheet ${b.sheet}  id ${b.id}`);
    } else if (sub === "erc") {
      const v = await detail();
      const list = v.violations.filter((x) => args.includes("--all") || !x.excluded);
      for (const x of list) console.log(`${x.excluded ? "excluded" : x.severity === "error" ? "✗" : "!"} ${x.code}: ${x.message}  (sheet ${x.sheet}, key ${x.key})`);
      console.log(`${v.stats.errors} errors, ${v.stats.warnings} warnings${v.violations.some((x) => x.excluded) ? `, ${v.violations.filter((x) => x.excluded).length} excluded` : ""}`);
    } else if (sub === "buses") {
      const v = await detail();
      if (!v.buses.length) console.log("no buses");
      for (const b of v.buses) console.log(`${b.name ?? "(unnamed bus)"} sheet ${b.sheet} id ${b.id}\n  ${b.members.map((m) => `${m.member}${m.net && m.net !== m.member ? `=${m.net}` : ""}${m.net ? "" : " (unused)"}`).join("  ")}`);
    } else if (sub === "sheets") {
      const v = await detail();
      const depth = (s) => (s.parent ? 1 + depth(v.sheets.find((x) => x.id === s.parent) ?? {}) : 0);
      for (const s of v.sheets) console.log(`${"  ".repeat(depth(s))}${s.id} ${s.name} (page ${s.page}${s.instance_of ? `, instance of ${s.instance_of}` : ""}) ${v.symbols.filter((x) => x.sheet === s.id).length} symbols`);
    } else throw new Error(usage);
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
} else if (command === "insert" || command === "links") {
  // Items from another project, copied or linked; this project's links.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = (extra = {}) => `?${new URLSearchParams({ ...(project ? { project } : {}), user: "forma-ai", ...extra })}`;
  const call = async (method, path, body) => {
    const res = await fetch(`${http}${path}`, { method, headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined });
    const text = await res.text();
    if (!res.ok) throw new Error(text);
    return JSON.parse(text);
  };
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const has = (name) => {
    const i = args.indexOf(name);
    if (i >= 0) args.splice(i, 1);
    return i >= 0;
  };
  const state = (l) =>
    l.state === "up_to_date"
      ? "up to date"
      : l.state === "update_available"
        ? `UPDATE AVAILABLE (${[l.changes.created && `+${l.changes.created} new`, l.changes.changed && `${l.changes.changed} changed`, l.changes.removed && `−${l.changes.removed} removed`, l.changes.conflicts && `${l.changes.conflicts} edited here too (source wins)`].filter(Boolean).join(", ") || "source changed"})`
        : `${l.state}: ${l.message ?? ""}`;
  try {
    if (command === "insert") {
      // forma insert --from <project> [ids or names…] [--all] [--copy] [--auto] [--at x,y,z | --origin] [--list] [--dry-run]
      const from = flag("--from");
      const at = flag("--at");
      const [copy, all, auto, origin, list, dry] = ["--copy", "--all", "--auto", "--origin", "--list", "--dry-run"].map(has);
      const items = args.slice(1);
      if (!from) throw new Error("say where from: forma insert --from <project> [ids or names…] [--all] [--copy] [--at x,y,z | --origin]");
      if (list || (!items.length && !all)) {
        const t = await call("GET", `/link/tree${q({ from })}`);
        console.log(`${t.title} (${t.project}) · revision ${t.revision} · ${t.entities} items in all`);
        const walk = (it, depth) => {
          console.log(`${"  ".repeat(depth + 1)}${it.name}  [${it.detail || it.kind}]${it.brings ? ` brings ${it.brings}` : ""}  (${it.id})`);
          for (const c of it.children ?? []) walk(c, depth + 1);
        };
        for (const it of t.items) walk(it, 0);
        if (!list) console.log(`\nPick by name or id: forma insert --from ${from} "${t.items[0]?.name ?? "Name"}" [--copy] [--at x,y,z], or --all for the whole project`);
      } else {
        const point = at ? at.split(",").map(Number) : null;
        if (point && (point.length !== 3 || point.some((v) => !Number.isFinite(v)))) throw new Error("--at takes x,y,z in mm");
        const body = { from, items, whole: all, copy, auto_update: auto, ...(point ? { at: point } : origin ? { place: "origin" } : {}) };
        if (dry) {
          const p = await call("POST", `/link/preview${q()}`, body);
          console.log(`would bring ${p.entities} items (${Object.entries(p.kinds).map(([k, n]) => `${n} ${k}`).join(", ")})${p.reused ? `, using ${p.reused} already here` : ""}${p.placed ? `, placing ${p.placed}` : ""}`);
          for (const r of p.renamed ?? []) console.log(`  “${r.from}” is taken here: it would be “${r.to}”`);
        } else {
          const r = await call("POST", `/link/insert${q()}`, body);
          const o = r.output;
          console.log(`✓ ${o.created} items from “${r.title}” (${r.from}, revision ${r.revision}) ${o.link ? `linked (link ${o.link}${auto ? ", updates by itself" : ""})` : "copied"}`);
          if (o.roots.length) console.log(`  picked: ${o.roots.join(" ")}`);
          if (o.placed.length) console.log(`  placed: ${o.placed.join(" ")}`);
          if (o.reused.length) console.log(`  ${o.reused.length} already here (identical) used instead of copied`);
          for (const x of o.renamed) console.log(`  renamed “${x.from}” → “${x.to}” (the name is taken here) ${x.id}`);
        }
      }
    } else {
      // forma links | links update [id] | links break <id> | links auto <id> on|off
      const sub = args[1] ?? "list";
      const id = args[2];
      if (sub === "list") {
        const links = await call("GET", `/links${q()}`);
        if (!links.length) console.log("no links: insert with `forma insert --from <project> …` (linked unless --copy)");
        for (const l of links) {
          console.log(`${l.id}  from ${l.project} (“${l.title}”) · ${l.whole ? "the whole project" : l.names.join(", ") || "items"} · ${l.items} items · at its revision ${l.revision}${l.source_revision != null ? ` (source at ${l.source_revision})` : ""}`);
          console.log(`  ${state(l)}${l.auto_update ? " · updates by itself" : ""}`);
        }
      } else if (sub === "update") {
        const out = await call("POST", `/links/update${q()}`, id ? { link: id } : {});
        for (const u of out) {
          if (u.error) {
            console.error(`✗ ${u.project}: ${u.error}`);
            failed = true;
            continue;
          }
          const o = u.output;
          if (o.up_to_date) console.log(`✓ ${u.project}: already up to date`);
          else {
            console.log(`✓ ${u.project}: ${o.created.length} new, ${o.changed.length} changed, ${o.removed.length} removed`);
            for (const c of o.conflicts) console.log(`  ${c.id} ${c.kind}: edited here and in ${u.project}; took ${u.project}'s (undo to get yours back)`);
            if (o.deleted_here) console.log(`  ${o.deleted_here} deleted here stay deleted`);
          }
        }
      } else if (sub === "break") {
        if (!id) throw new Error("say which link (or an item it brought): forma links break <id>");
        const o = await call("POST", `/links/break${q()}`, { link: id });
        console.log(`✓ the ${o.items} items from ${o.project} are a local copy now`);
      } else if (sub === "auto") {
        const on = args[3];
        if (!id || !["on", "off"].includes(on)) throw new Error("forma links auto <id> on|off");
        await call("POST", `/links/settings${q()}`, { link: id, auto_update: on === "on" });
        console.log(`✓ ${on === "on" ? "updates by itself when its source changes" : "updates when asked (forma links update)"}`);
      } else throw new Error("forma links [list] | update [id] | break <id> | auto <id> on|off");
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
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
  } else if (sub === "profile") {
    // Measure the object a photo or drawing shows: its outline row by row,
    // in mm from its real height or width, as a profile to revolve.
    const what = args[2];
    const flag = (k) => {
      const i = args.indexOf(k);
      return i > 0 ? args[i + 1] : undefined;
    };
    let id = what;
    if (what && !/^[0-9a-f]{64}$/i.test(what)) {
      let bytes;
      try {
        bytes = readFileSync(what);
      } catch (e) {
        console.error(`✗ ${what}: ${e.message}`);
        failed = true;
      }
      if (bytes) {
        const res = await fetch(`${http}/images/${encodeURIComponent(what.split("/").pop())}${q}`, { method: "POST", body: bytes });
        if (res.ok) id = (await res.json()).id;
        else {
          console.error(`✗ ${await res.text()}`);
          failed = true;
        }
      }
    }
    if (!what) {
      console.error("usage: forma image profile <file|id> [--height mm | --width mm] [--rows 40] [--threshold 0.12] [--rim] [--json]");
      failed = true;
    } else if (!failed) {
      const params = new URLSearchParams({ silhouette: flag("--rows") ?? "40" });
      for (const k of ["height", "width", "threshold"]) if (flag(`--${k}`)) params.set(k, flag(`--${k}`));
      if (args.includes("--rim")) params.set("rim", "1");
      const res = await fetch(`${http}/images/${id}?${params}`);
      if (!res.ok) {
        console.error(`✗ ${await res.text()}`);
        failed = true;
      } else {
        const v = await res.json();
        const px = v.pixels;
        if (args.includes("--json")) console.log(JSON.stringify(v, null, 1));
        else {
          console.log(`picture ${id} (${v.picture.width} × ${v.picture.height} px): object from y ${px.top} to ${px.bottom} px (${px.height} px tall), widest ${px.widest} px, axis x ${px.axis} px, middles stray ±${px.drift} px`);
          if (v.mm) {
            const m = v.mm;
            console.log(`at ${m.per_pixel.toFixed(4)} mm/px: ${m.height} mm tall, widest Ø${m.widest} mm, axis drift ${m.drift} mm`);
            if (m.cap_top > 0.01 * m.height && !args.includes("--rim"))
              console.log(`! its top only reaches full width ${m.cap_top} mm down (y ${px.cap_top} px): a rim seen from a little above (an opening's ellipse), or a rounded top. For a rim, measure from its line with --rim (and give the height to the rim).`);
            if (m.cap_bottom > 0.01 * m.height) console.log(`! its bottom narrows over the last ${m.cap_bottom} mm: a rounded or undercut foot, or the base's ellipse; look at the photo.`);
            console.log("profile [radius, z] mm, bottom up (polygon([[0,0], …profile, [0, height]]).revolve(), or a spline through it):");
            console.log(JSON.stringify(m.profile));
          } else {
            console.log("rows (y, left, right px) — give --height or --width (mm) for a profile in mm:");
            for (const r of px.rows) console.log(`  ${r.y}  ${r.left}–${r.right}  (${r.width} px)`);
          }
          console.log(`in a program: image("${id}").profile({ height: …, rows: ${flag("--rows") ?? 40} }) gives the same points live`);
        }
      }
    }
  } else {
    console.error("usage: forma image add <file> … | forma image list | forma image profile <file|id> [--height mm]");
    failed = true;
  }
} else if (command === "canvas" || (command === "look" && args.includes("--overlay"))) {
  // Reference pictures placed in the model: add, list, change, remove, and compare the model with them.
  // `look --overlay <canvas> [what]` is `canvas match <canvas> --of what`.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  let sub = file;
  let rest = args.slice(2);
  if (command === "look") {
    const i = args.indexOf("--overlay");
    const which = args.splice(i, 2)[1];
    const o = args.indexOf("--out");
    const out = o >= 0 ? args.splice(o, 2) : [];
    const v = args.indexOf("--view");
    if (v >= 0) args.splice(v, 2);
    const what = args.slice(1).join(" ").trim();
    sub = "match";
    rest = [which, ...(what ? ["--of", what] : []), ...out];
  }
  const flag = (name) => {
    const i = rest.indexOf(name);
    return i >= 0 ? rest.splice(i, 2)[1] : undefined;
  };
  const bool = (name) => {
    const i = rest.indexOf(name);
    return i >= 0 ? (rest.splice(i, 1), true) : false;
  };
  const nums = (s, n, what) => {
    const v = String(s).split(",").map((x) => Number(x.trim()));
    if ((n && v.length !== n) || v.some((x) => !Number.isFinite(x))) throw new Error(`${what}: give ${n ?? "the"} numbers separated by commas`);
    return v;
  };
  const canvases = async () => (await engine.snapshot).entities.filter((e) => e.components["shape.canvas"]).map((e) => ({ id: e.id, ...e.components["shape.canvas"] }));
  const pick = async (what) => {
    const all = await canvases();
    const hit = all.filter((c) => c.id === what || c.name === what || c.name.toLowerCase() === String(what).toLowerCase());
    if (hit.length !== 1) throw new Error(hit.length ? `“${what}” matches ${hit.length} canvases: use an id` : `no canvas called “${what}” (forma canvas list)`);
    return hit[0];
  };
  // The settings shared by add and update, from flags.
  const settings = () => {
    const s = {};
    const v = (name) => flag(name);
    let x;
    if ((x = v("--name")) !== undefined) s.name = x;
    if ((x = v("--plane")) !== undefined) s.plane = x.trim().startsWith("{") ? JSON.parse(x) : x;
    if ((x = v("--offset")) !== undefined) s.offset = Number(x);
    if ((x = v("--pivot")) !== undefined) {
      // --pivot y,8.3 or --pivot ax,ay,az,deg: turn the plane about a world axis.
      const parts = x.split(",").map((t) => t.trim());
      const named = { x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] }[parts[0]?.toLowerCase()];
      const nums4 = parts.map(Number);
      if (named && parts.length === 2 && Number.isFinite(nums4[1])) s.pivot = { axis: named, degrees: nums4[1] };
      else if (parts.length === 4 && nums4.every(Number.isFinite)) s.pivot = { axis: nums4.slice(0, 3), degrees: nums4[3] };
      else throw new Error("--pivot x|y|z,degrees or ax,ay,az,degrees");
    }
    if ((x = v("--at")) !== undefined) s.at = nums(x, 3, "--at x,y,z");
    if ((x = v("--anchor")) !== undefined) s.anchor = nums(x, 2, "--anchor u,v");
    if ((x = v("--scale")) !== undefined) s.scale = Number(x);
    if ((x = v("--width")) !== undefined) s.width = Number(x);
    if ((x = v("--height")) !== undefined) s.height = Number(x);
    if ((x = v("--calibrate")) !== undefined) {
      const c = nums(x, 5, "--calibrate u1,v1,u2,v2,mm");
      s.calibrate = { a: [c[0], c[1]], b: [c[2], c[3]], distance: c[4] };
    }
    const ow = v("--object-width"), oh = v("--object-height"), ob = v("--object"), al = v("--align");
    if (ob !== undefined || ow !== undefined || oh !== undefined) {
      const o = {};
      if (ob !== undefined) {
        const [w, h] = ob.split(/[x×]/).map((t) => (t.trim() === "" || t.trim() === "?" ? undefined : Number(t)));
        if (w !== undefined) o.width = w;
        if (h !== undefined) o.height = h;
      }
      if (ow !== undefined) o.width = Number(ow);
      if (oh !== undefined) o.height = Number(oh);
      if (al !== undefined) o.align = al;
      s.object = o;
    } else if (al !== undefined) throw new Error("--align goes with --object WxH, --object-width or --object-height");
    if ((x = v("--aspect")) !== undefined) s.aspect = Number(x);
    if ((x = v("--rotate")) !== undefined) s.rotation = Number(x);
    if ((x = v("--level")) !== undefined) {
      const c = nums(x, 4, "--level u1,v1,u2,v2");
      s.level = [[c[0], c[1]], [c[2], c[3]]];
    }
    if ((x = v("--perspective")) !== undefined) {
      const m = x.match(/^(.*),\s*([\d.]+)\s*[x×]\s*([\d.]+)$/);
      if (!m) throw new Error("--perspective u1,v1,u2,v2,u3,v3,u4,v4,WxH (corners top-left, top-right, bottom-right, bottom-left; the rectangle's true size in mm)");
      const c = nums(m[1], 8, "--perspective corners");
      s.perspective = { points: [[c[0], c[1]], [c[2], c[3]], [c[4], c[5]], [c[6], c[7]]], width: Number(m[2]), height: Number(m[3]) };
    }
    if ((x = v("--crop")) !== undefined) s.crop = nums(x, 4, "--crop left,top,right,bottom");
    if ((x = v("--opacity")) !== undefined) s.opacity = Number(x);
    if ((x = v("--background")) !== undefined) s.background = nums(x, 3, "--background r,g,b (0–1)");
    if ((x = v("--threshold")) !== undefined) s.threshold = Number(x);
    if ((x = v("--reset")) !== undefined) s.reset = x.split(",").map((t) => t.trim());
    if (bool("--flip")) s.flip = true;
    if (bool("--no-flip")) s.flip = false;
    if (bool("--through")) s.through = true;
    if (bool("--behind")) s.through = false;
    if (bool("--hide")) s.visible = false;
    if (bool("--show")) s.visible = true;
    return s;
  };
  const describe = (out) => {
    const f = (v) => (Math.round(v * 100) / 100).toString();
    let line = `✓ canvas “${out.name}” (${out.id}) on the ${out.view ?? "custom"} plane: ${f(out.mm_per_px)} mm per pixel, picture ${f(out.size_mm[0])} × ${f(out.size_mm[1])} mm`;
    if (out.object_mm) line += `\n  object: ${f(out.object_mm.size[0])} × ${f(out.object_mm.size[1])} mm, from (${f(out.object_mm.min[0])}, ${f(out.object_mm.min[1])}) to (${f(out.object_mm.max[0])}, ${f(out.object_mm.max[1])}) on the plane; pixels ${out.object_px.join(", ")}`;
    if (!out.sized) line += "\n  ! not sized yet: give --calibrate, --object-width/--object-height, --width or --scale";
    return line;
  };
  const match = async (c, of, outPath) => {
    const q = new URLSearchParams({ ...(project ? { project } : {}), canvas: c.id, ...(of ? { id: of } : {}) });
    const res = await fetch(`${http}/look?${q}`);
    if (!res.ok) throw new Error(await res.text());
    const m = JSON.parse(res.headers.get("x-forma-match"));
    const { mkdirSync, writeFileSync } = await import("node:fs");
    const slug = c.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "canvas";
    const path = outPath ?? `look/match-${slug}.png`;
    if (path.includes("/")) mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, Buffer.from(await res.arrayBuffer()));
    const f = (v, d = 1) => (Number.isFinite(v) ? v.toFixed(d) : "–");
    console.log(`${m.iou >= 0.9 ? "✓" : m.iou >= 0.8 ? "~" : "✗"} ${c.name} (${m.view ?? "custom"} view): IoU ${f(m.iou, 3)}  (model covers ${f(m.recall * 100)} % of the photo's object; ${f(m.precision * 100)} % of the model is on it)`);
    console.log(`  outlines ${f(m.mean_mm, 2)} mm apart on average, worst ${f(m.worst_mm)} mm; photo object ${f(m.photo_size_mm[0])} × ${f(m.photo_size_mm[1])} mm, model ${f(m.model_size_mm[0])} × ${f(m.model_size_mm[1])} mm (plane x × y)`);
    for (const r of m.regions.slice(0, 8)) {
      const at = r.at.map((v) => f(v)).join(", ");
      console.log(`  ${r.kind === "extra" ? "+ extra  " : "- missing"} ${f(r.area_mm2, 0).padStart(6)} mm², up to ${f(r.depth_mm)} mm thick, around (${at}) — plane x ${f(r.bounds[0])}…${f(r.bounds[2])}, y ${f(r.bounds[1])}…${f(r.bounds[3])}`);
    }
    console.log(`  overlay: ${path} (read it: the photo, the model over it, red = model where the photo has none, blue = photo where the model has none)`);
    return m;
  };
  try {
    if (sub === "add") {
      const src = rest.shift();
      if (!src) throw new Error("say which picture: forma canvas add <photo.jpg | picture id> --plane front …");
      const s = settings();
      let image = src;
      if (!/^[0-9a-f]{64}$/i.test(src)) {
        const bytes = readFileSync(src);
        const res = await fetch(`${http}/images/${encodeURIComponent(basename(src))}${project ? `?project=${encodeURIComponent(project)}` : ""}`, { method: "POST", body: bytes });
        if (!res.ok) throw new Error(await res.text());
        image = (await res.json()).id;
      }
      const name = s.name ?? basename(src).replace(/\.[a-z0-9]+$/i, "");
      delete s.name;
      const r = await engine.commit("canvas.create", { name, image, ...s });
      if (!r.ok) throw new Error(r.error.message);
      console.log(describe(r.value.output));
      console.log(`  in a program: canvas("${name}")`);
    } else if (sub === "update" || sub === "set" || sub === "calibrate") {
      const c = await pick(rest.shift());
      const s = settings();
      const r = await engine.commit("canvas.update", { target: c.id, ...s });
      if (!r.ok) throw new Error(r.error.message);
      console.log(describe(r.value.output));
    } else if (sub === "remove" || sub === "delete") {
      const c = await pick(rest.shift());
      const r = await engine.commit("core.entity.delete", { target: c.id });
      if (!r.ok) throw new Error(r.error.message);
      console.log(`✓ removed canvas “${c.name}”`);
    } else if (sub === "list" || !sub) {
      const all = await canvases();
      if (!all.length) console.log("no canvases yet (forma canvas add <photo> --plane front --object-width 165)");
      for (const c of all) {
        const o = c.plane.origin.map((v) => Math.round(v * 100) / 100).join(", ");
        const cal = c.perspective ? `squared up to ${c.perspective.width} × ${c.perspective.height} mm` : c.calibration ? `calibrated ${c.calibration.distance} mm between (${c.calibration.a}) and (${c.calibration.b})` : c.sized ? `${(Math.round(c.scale * 10000) / 10000)} mm/px${c.aspect !== undefined && c.aspect !== 1 ? `, aspect ${Math.round(c.aspect * 1000) / 1000}` : ""}` : "NOT SIZED";
        console.log(`${c.id}  “${c.name}”  ${c.view ?? "custom"} plane, origin (${o}); ${c.pixels[0]} × ${c.pixels[1]} px, ${cal}; anchor px (${c.anchor.map((v) => Math.round(v)).join(", ")})${c.rotation ? `, turned ${c.rotation}°` : ""}${c.flip ? ", mirrored" : ""}${c.crop ? `, crop ${c.crop.join(",")}` : ""}; opacity ${c.opacity}${c.visible === false ? ", hidden" : ""}${c.through ? ", shown through" : ""}`);
      }
    } else if (sub === "measure" || sub === "outline") {
      // The canvas's outline in mm: bounds, area, and its extent at stations across and up.
      const c = await pick(rest.shift());
      const q = new URLSearchParams({ ...(project ? { project } : {}), measure: c.id });
      for (const k of ["every", "channel", "threshold", "region"]) {
        const v = flag(`--${k}`);
        if (v !== undefined) q.set(k, v);
      }
      if (bool("--invert")) q.set("invert", "1");
      if (bool("--all")) q.set("largest", "0");
      if (bool("--no-holes")) q.set("holes", "0");
      const res = await fetch(`${http}/look?${q}`);
      if (!res.ok) throw new Error(await res.text());
      const m = await res.json();
      const f = (v) => (Math.round(v * 10) / 10).toString();
      const axes = (v) => (v === "front" ? ["x", "z"] : v === "back" ? ["−x", "z"] : v === "left" ? ["−y", "z"] : v === "right" ? ["y", "z"] : v === "top" ? ["x", "y"] : v === "bottom" ? ["−x", "y"] : ["plane x", "plane y"]);
      const [ax, ay] = axes(m.view);
      console.log(`“${m.canvas}” (${m.view ?? "custom"} plane; plane x is world ${ax}, plane y is world ${ay}): outline ${f(m.bounds.size[0])} × ${f(m.bounds.size[1])} mm, from (${f(m.bounds.min[0])}, ${f(m.bounds.min[1])}) to (${f(m.bounds.max[0])}, ${f(m.bounds.max[1])}); area ${Math.round(m.area_mm2)} mm²; ${m.outlines} outline${m.outlines === 1 ? "" : "s"}, ${m.holes} hole${m.holes === 1 ? "" : "s"}`);
      const row = (s) => `${f(s.at).padStart(7)}: ${s.spans.length ? s.spans.map(([a, b]) => `${f(a)}…${f(b)} (${f(b - a)})`).join(", ") : "—"}`;
      console.log(`  across, every ${f(m.step)} mm — at plane x: the spans in plane y (length)`);
      for (const s of m.across) console.log("  " + row(s));
      console.log(`  up — at plane y: the spans in plane x (length)`);
      for (const s of m.up) console.log("  " + row(s));
    } else if (sub === "match" || sub === "compare") {
      const which = rest.shift();
      const of = flag("--of");
      const out = flag("--out");
      let ofId = null;
      if (of) {
        const s = await engine.snapshot;
        const nm = (e) => e.components["assembly.instance"]?.name ?? e.components["shape.program"]?.name;
        const hit = s.entities.filter((e) => e.id === of || nm(e)?.toLowerCase() === of.toLowerCase()).filter((e) => e.components["assembly.instance"] || e.components["solid.placement"]);
        if (hit.length !== 1) throw new Error(`--of “${of}” matches ${hit.length} placed things`);
        ofId = hit[0].id;
      }
      const list = !which || which === "all" ? await canvases() : [await pick(which)];
      if (!list.length) throw new Error("no canvases to compare with");
      for (const c of list) await match(c, ofId, list.length > 1 ? undefined : out);
    } else {
      throw new Error("usage: forma canvas add <photo|id> [--plane front|back|left|right|top|bottom] [--object WxH --align 'left bottom' | --calibrate u1,v1,u2,v2,mm | --width mm | --scale mm/px] [--at x,y,z] [--anchor u,v] [--rotate deg | --level u1,v1,u2,v2] [--flip] [--aspect k] [--crop l,t,r,b] [--perspective u1,v1,…,u4,v4,WxH] [--opacity 0.5] [--through] [--name N]\n       | list | update <canvas> [same flags] [--reset crop,perspective,…] | remove <canvas> | measure <canvas> [--every mm] [--channel c --threshold t] [--region l,t,r,b] | match [<canvas>|all] [--of <thing>] [--out f.png]");
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
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
  // --box x0,y0,z0,x1,y1,z1: frame only that region (mm), to read a detail.
  const box = flag("--box");
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
    const q = new URLSearchParams({ ...(project ? { project } : {}), ...(id ? { id } : {}), view: viewName, ...(size ? { size } : {}), ...(box ? { box } : {}) });
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
    const kind = c["shape.program"]
      ? `shape “${c["shape.program"].name}”`
      : c["solid.box"]
        ? "box"
        : c["solid.material"]
          ? `material ${c["solid.material"].name}`
          : c["intent.requirement"]
            ? `requirement “${c["intent.requirement"].name}”`
            : c["shape.sheet"]
              ? `sheet “${c["shape.sheet"].name}”`
              : c["draft.array"]
                ? `array “${c["shape.parameter_set"]?.name}” (${c["draft.array"].kind}, ${c["draft.array"].members.length + 1} copies)`
                : c["shape.parameter_set"]
                  ? `parameters “${c["shape.parameter_set"].name}”`
                  : c["draft.object"]
                    ? `draft ${c["draft.object"].kind} “${c["draft.object"].name}”`
                    : c["draft.layer"]
                      ? `layer “${c["draft.layer"].name}”`
                      : c["draft.plane"]
                        ? `working plane “${c["draft.plane"].name}”`
                        : c["shape.sketch"]
                          ? `sketch “${c["shape.sketch"].name}”`
                          : c["assembly.instance"]
                            ? `instance ${c["assembly.instance"].name}`
                            : c["assembly.definition"]
                              ? `assembly “${c["assembly.definition"].name}”`
                              : "entity";
    const extra = e.result
      ? ` — ${e.result.passed ? "met" : "FAILING"}: ${e.result.summary}`
      : e.draft?.value != null
        ? ` — ${e.draft.label?.[1] ?? e.draft.value}`
        : e.sheet
          ? ` — ${Object.keys(e.sheet.cells).length} cells${e.sheet.errors ? `, ${e.sheet.errors} errors` : ""}`
          : e.mass
            ? ` — ${(e.mass.volume_m3 * 1e9).toFixed(1)} mm³`
            : "";
    console.log(`${e.id}  ${kind}${extra}`);
  }
} else if (command === "get") {
  const s = await engine.snapshot;
  // By id, or by name (a shape, instance, assembly, sketch, sheet, canvas, material…).
  const named = (x) => {
    const c = x.components;
    return (
      c["assembly.instance"]?.name ??
      c["shape.program"]?.name ??
      c["assembly.definition"]?.name ??
      c["shape.sketch"]?.name ??
      c["shape.sheet"]?.name ??
      c["shape.canvas"]?.name ??
      // Anything else that carries a name (a material, a board, a parameter set…).
      Object.values(c ?? {}).find((v) => v && typeof v === "object" && typeof v.name === "string")?.name
    );
  };
  const byName = s.entities.filter((x) => file && named(x)?.toLowerCase() === file.toLowerCase());
  const e = s.entities.find((x) => x.id === file) ?? (byName.length === 1 ? byName[0] : undefined);
  if (!e) {
    console.error(byName.length > 1 ? `“${file}” names ${byName.length} things: use an id (${byName.map((x) => x.id).join(", ")})` : `no entity ${file} (give an id or a name)`);
    failed = true;
  } else {
    // Meshes are for drawing, not reading: report their size instead (a
    // snapshot sends a mesh as a reference with its triangle count).
    const { mesh, ...rest } = e;
    // Snapshots carry a reference and a triangle count (older ones the indices).
    const triangles = mesh ? (mesh.triangles ?? (mesh.indices ? mesh.indices.length / 3 : null)) : null;
    console.log(JSON.stringify({ ...rest, mesh: mesh ? { triangles } : null }, null, 2));
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
} else if (command === "sheet") {
  // A spreadsheet as a grid, as each cell shows.
  const s = await engine.snapshot;
  const want = args.slice(1).join(" ");
  const sheets = s.entities.filter((e) => e.components["shape.sheet"]);
  const e = sheets.find((x) => x.id === want) ?? sheets.find((x) => x.components["shape.sheet"].name.toLowerCase() === want.toLowerCase()) ?? (!want && sheets.length === 1 ? sheets[0] : null);
  if (!e) {
    console.error(want ? `no sheet “${want}” (sheets: ${sheets.map((x) => x.components["shape.sheet"].name).join(", ") || "none"})` : `say which sheet: ${sheets.map((x) => x.components["shape.sheet"].name).join(", ") || "there are none"}`);
    failed = true;
  } else {
    const sheet = e.components["shape.sheet"];
    const cells = e.sheet?.cells ?? {};
    const addr = (a) => {
      const m = /^([A-Z]+)(\d+)$/.exec(a);
      let c = 0;
      for (const ch of m[1]) c = c * 26 + ch.charCodeAt(0) - 64;
      return [c - 1, Number(m[2]) - 1];
    };
    const keys = Object.keys(sheet.cells);
    const cols = Math.max(0, ...keys.map((a) => addr(a)[0] + 1));
    const rows = Math.max(0, ...keys.map((a) => addr(a)[1] + 1));
    const colName = (c) => {
      let s = "";
      c += 1;
      while (c > 0) {
        const r = (c - 1) % 26;
        s = String.fromCharCode(65 + r) + s;
        c = Math.floor((c - 1) / 26);
      }
      return s;
    };
    const width = Array.from({ length: cols }, (_, c) => Math.min(28, Math.max(4, ...Array.from({ length: rows }, (_, r) => (cells[colName(c) + (r + 1)]?.shown ?? "").length))));
    console.log(`${sheet.name}  (${e.id})${sheet.description ? `  ${sheet.description}` : ""}`);
    console.log("     " + width.map((w, c) => colName(c).padEnd(w)).join(" │ "));
    for (let r = 0; r < rows; r++) {
      const line = width.map((w, c) => {
        const v = cells[colName(c) + (r + 1)];
        const t = (v?.shown ?? "").slice(0, w);
        return v?.value != null ? t.padStart(w) : t.padEnd(w);
      });
      console.log(String(r + 1).padStart(4) + " " + line.join(" │ "));
    }
    const aliases = Object.entries(e.sheet?.aliases ?? {});
    if (aliases.length) console.log(`aliases: ${aliases.map(([a, at]) => `${a}=${at}`).join(", ")}  (read elsewhere as ${sheet.name}.<alias> or ${sheet.name}.<cell>)`);
    const formulas = Object.entries(cells).filter(([, v]) => v.formula);
    if (formulas.length) console.log(`formulas: ${formulas.map(([a, v]) => `${a} ${v.formula}`).join("; ")}`);
    for (const [a, v] of Object.entries(cells)) if (v.error) console.log(`  ✗ ${a}: ${v.error}`);
  }
} else if (command === "draft") {
  // Draft objects (with what dimensions measure), layers, working planes and arrays.
  const s = await engine.snapshot;
  const fmt = (p) => `[${p.map((v) => +v.toFixed(3)).join(", ")}]`;
  for (const e of s.entities) {
    const c = e.components;
    if (c["draft.object"]) {
      const o = c["draft.object"];
      const d = e.draft;
      const what = d?.value != null ? ` = ${d.label?.[1]}` : d?.area != null ? ` length ${+d.length.toFixed(3)} mm, area ${+d.area.toFixed(3)} mm²` : d ? ` length ${+d.length.toFixed(3)} mm` : "";
      console.log(`${e.id}  ${o.kind} “${o.name}”${what}${o.text && o.kind !== "dimension" ? ` “${o.text}”` : ""}${d?.points?.length ? `  points ${d.points.slice(0, 4).map(fmt).join(" ")}${d.points.length > 4 ? " …" : ""}` : ""}${e.status?.state === "failed" ? `  ✗ ${e.status.reason}` : ""}`);
    } else if (c["draft.layer"]) {
      const l = c["draft.layer"];
      console.log(`${e.id}  layer “${l.name}”${l.visible ? "" : " (hidden)"}${l.locked ? " (locked)" : ""}${l.color ? ` ${l.color}` : ""}: ${l.members.length} members`);
    } else if (c["draft.plane"]) {
      const p = c["draft.plane"];
      console.log(`${e.id}  working plane “${p.name}” origin ${fmt(p.origin)} x ${fmt(p.x)} y ${fmt(p.y)}`);
    } else if (c["draft.array"]) {
      const a = c["draft.array"];
      const v = e.params ?? {};
      console.log(`${e.id}  array “${c["shape.parameter_set"]?.name}”: ${a.kind} ×${a.count}${a.count2 > 1 ? `×${a.count2}` : ""}, spacing ${v.spacing}${a.kind === "polar" ? `, angle ${v.angle}°` : ""}${e.formulas && Object.keys(e.formulas).length ? ` (${Object.entries(e.formulas).map(([k, f]) => `${k} ${f}`).join(", ")})` : ""}`);
    }
  }
} else if (command === "import" || command === "export" || command === "bundle") {
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const [read, write] = [readFileSync, writeFileSync];
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const snap = await engine.snapshot;
  const projectName = project ?? snap.project_id;
  try {
    if (command === "import") {
      // STEP/IGES (exact), STL/OBJ/PLY/glTF/GLB/3MF (mesh) become a shape; DXF/SVG a sketch;
      // .scad a live shape; .csv/.xlsx a sheet, parameter set or configurations; .forma a project.
      const as = flag("--as");
      const target = flag("--target");
      const sheet = flag("--sheet");
      const named = flag("--name");
      const path = args[1];
      if (!path) throw new Error("say which file: forma import <file> [--name N] [--as sheet|parameters|configurations]");
      const bytes = read(path);
      const name = named ? `${named}${extname(path)}` : basename(path);
      if (name.toLowerCase().endsWith(".forma")) {
        const res = await fetch(`${http}/projects/import`, { method: "POST", body: bytes });
        if (!res.ok) throw new Error(await res.text());
        const r = await res.json();
        console.log(`✓ imported as project “${r.name}”${r.assets ? ` (${r.assets} asset files)` : ""}: open it with --project ${r.name}`);
      } else {
        const q = new URLSearchParams({ project: projectName, user: "forma-ai", ...(as ? { as } : {}), ...(target ? { target } : {}), ...(sheet ? { sheet } : {}) });
        const res = await fetch(`${http}/import/${encodeURIComponent(name)}?${q}`, { method: "POST", body: bytes });
        if (!res.ok) throw new Error(await res.text());
        const r = await res.json();
        if (r.kind === "sketch") {
          const c = r.counts;
          console.log(`✓ ${basename(path)} → sketch “${r.name}” (${r.id}): ${c.lines} lines, ${c.arcs} arcs, ${c.circles} circles, ${c.curves} curves; ${r.size.map((v) => v.toFixed(1)).join(" × ")} mm. Use it in a program: sketch("${r.name}").extrude(3)`);
        } else if (r.kind === "shape") {
          console.log(`✓ ${basename(path)} → shape ${r.id}: ${r.bodies} ${r.bodies === 1 ? "body" : "bodies"}, ${r.exact ? "exact" : "mesh (faceted)"}; program reads ${r.format === "step" ? "importStep" : r.format === "iges" ? "importIges" : "importMesh"}("${r.sha}")`);
          if (r.colors?.length) console.log(`  colours in the file: ${r.colors.map((c) => `${c.part} ${c.color}`).join(", ")}`);
        } else console.log(`✓ ${r.operation ?? "import"} → ${JSON.stringify(r.output ?? r)}`);
      }
    } else if (command === "export") {
      // forma export <id|name|project> <file.ext>, or --format step|iges|stl|obj|ply|glb|3mf|dxf|stpz|brep|scad|zip|csv|xlsx [--part P] [--out path]
      const format = flag("--format");
      const part = flag("--part");
      const outFlag = flag("--out");
      // Meshes (STL, 3MF, OBJ…) follow curves within this many mm (default 0.005: very fine; 0.02 is plenty to print).
      const tolerance = flag("--tolerance");
      const [, what, given] = args;
      const ext = (format ?? given?.split(".").pop() ?? "").toLowerCase().replace(/^\./, "");
      if (!what || !ext) throw new Error("forma export <id|name|project> <file.step|…> | --format step|iges|stl|obj|ply|glb|3mf|dxf|stpz|brep|scad|zip|csv|xlsx [--part P] [--out path]");
      const name = (e) => {
        const c = e.components;
        return c["assembly.instance"]?.name ?? c["shape.program"]?.name ?? c["shape.sheet"]?.name ?? c["shape.parameter_set"]?.name ?? c["assembly.definition"]?.name;
      };
      // An id, or a name (what is placed first).
      const named = snap.entities.filter((e) => e.id === what || name(e)?.toLowerCase() === what.toLowerCase());
      const placed = named.filter((e) => e.components["assembly.instance"] || e.components["solid.placement"]);
      const found = what === "project" ? [{ id: "project" }] : placed.length ? placed : named;
      if (found.length !== 1) throw new Error(found.length ? `“${what}” matches ${found.length} things: use an id (${found.map((e) => e.id).join(", ")})` : `nothing called “${what}”`);
      const route = ext === "csv" || ext === "xlsx" ? "table" : "export";
      const q = new URLSearchParams({ project: projectName, ...(part ? { part } : {}), ...(tolerance ? { tolerance } : {}) });
      const res = await fetch(`${http}/${route}/${found[0].id}.${ext === "iges" ? "igs" : ext}?${q}`);
      if (!res.ok) throw new Error(await res.text());
      const bytes = Buffer.from(await res.arrayBuffer());
      const suggested = res.headers.get("content-disposition")?.match(/filename="([^"]+)"/)?.[1] ?? `${found[0].id}.${ext}`;
      const out = given && !format ? given : outFlag;
      const target = out ? resolve(out) : join(homedir(), "Downloads", "Forma exports", suggested);
      const path = out && existsSync(target) && statSync(target).isDirectory() ? join(target, suggested) : target;
      mkdirSync(dirname(path), { recursive: true });
      write(path, bytes);
      const size = bytes.length >= 1048576 ? `${(bytes.length / 1048576).toFixed(1)} MB` : `${(bytes.length / 1024).toFixed(1)} KB`;
      console.log(`✓ ${path} (${size})`);
    } else {
      const out = args[1] ?? `${projectName}.forma`;
      const res = await fetch(`${http}/projects/${encodeURIComponent(projectName)}/bundle`);
      if (!res.ok) throw new Error(await res.text());
      const bytes = Buffer.from(await res.arrayBuffer());
      write(out, bytes);
      console.log(`✓ wrote ${out} (${(bytes.length / 1024).toFixed(0)} KB): the whole project with its history and assets; import it elsewhere with forma import ${out}`);
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else if (command === "scan" || command === "mesh" || command === "inspect") {
  // Point clouds and meshes: add, check, repair, rework, fit, section, unwrap, inspect against a part.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const q = new URLSearchParams(project ? { project } : {}).toString();
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const has = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? (args.splice(i, 1), true) : false;
  };
  const nums = (s) => (s == null ? undefined : s.split(",").map(Number));
  const num = (s) => (s == null ? undefined : Number(s));
  const call = async (method, path, body) => {
    const res = await fetch(`${http}${path}${path.includes("?") ? "&" : "?"}${q}`, { method, body: body === undefined ? undefined : typeof body === "string" || body instanceof Uint8Array ? body : JSON.stringify(body) });
    if (!res.ok) throw new Error(await res.text());
    return res.json();
  };
  const r3 = (v) => Math.round(v * 1000) / 1000;
  const p3 = (p) => `(${p.map(r3).join(", ")})`;
  const sayScan = (e) => `${e.name} [${e.id.slice(0, 12)}]: ${e.kind === "mesh" ? `mesh, ${e.triangles} triangles, ${e.points} vertices` : `${e.points} points`} (${e.format}${e.source && e.source !== "file" ? `, from ${e.source}` : ""}); ${e.bounds.slice(3).map((v, i) => r3(v - e.bounds[i])).join(" × ")} mm`;
  const region = () => {
    const near = flag("--near");
    const within = flag("--within");
    const box = flag("--box");
    return { ...(near ? { near: nums(near), within: num(within ?? "5") } : {}), ...(box ? { box: nums(box) } : {}) };
  };
  const sayFit = (f) => {
    const g = { ...f };
    for (const k of ["kind", "rms", "max", "points", "of", "alternatives"]) delete g[k];
    const params = Object.entries(g)
      .map(([k, v]) => `${k} ${Array.isArray(v) ? p3(v) : typeof v === "number" ? r3(v) : v}`)
      .join(", ");
    return `${f.kind}: ${params} — RMS ${f.rms.toExponential(2)} mm, worst ${r3(f.max)} mm over ${f.points}${f.of !== f.points ? ` of ${f.of}` : ""} points`;
  };
  const report = (r) =>
    [
      `${r.vertices} vertices, ${r.triangles} triangles, ${r.components} piece${r.components === 1 ? "" : "s"}`,
      r.solid ? "a closed solid" : "NOT a closed solid",
      r.holes && `${r.holes} hole${r.holes === 1 ? "" : "s"} (${r.open_edges} open edges)`,
      r.non_manifold_edges && `${r.non_manifold_edges} non-manifold edges`,
      r.non_manifold_vertices && `${r.non_manifold_vertices} non-manifold vertices`,
      r.flipped && `${r.flipped} flipped triangles`,
      r.degenerate && `${r.degenerate} degenerate`,
      r.duplicate && `${r.duplicate} duplicate`,
      r.self_intersections && `${r.self_intersections}${r.intersections_capped ? "+" : ""} self-intersecting pairs`,
      r.unused_vertices && `${r.unused_vertices} unused vertices`,
      `volume ${r3(r.volume)} mm³, area ${r3(r.area)} mm²`,
    ]
      .filter(Boolean)
      .join("; ");
  const sub = command === "inspect" ? "inspect" : file;
  const id = command === "inspect" ? args[1] : args[2];
  try {
    if (command === "scan" && sub === "add") {
      const path = args[2];
      if (!path) throw new Error("forma scan add <file> [--name N] [--source colmap|object-capture|scanner]");
      const name = flag("--name");
      const source = flag("--source");
      const qs = new URLSearchParams({ ...(name ? { name } : {}), ...(source ? { source } : {}) });
      const e = await call("POST", `/scans/${encodeURIComponent(basename(path))}?${qs}`, readFileSync(path));
      console.log(`✓ ${sayScan(e)}\n  in a program: fitScan("${e.id}", …), ${e.kind === "mesh" ? `importMesh("${e.id}", { repair: true })` : `reconstruct("${e.id}")`}`);
    } else if (command === "scan" && (sub === "list" || !sub)) {
      const list = await call("GET", "/scans");
      if (!list.length) console.log("no scans in this project yet (forma scan add <file>)");
      for (const e of list) console.log(`${e.id}  ${sayScan(e)}${e.parent ? `  ← ${e.parent.slice(0, 12)}` : ""}`);
    } else if (command === "scan" && sub === "engines") {
      const { engines } = await call("GET", "/photogrammetry/engines");
      for (const e of engines) console.log(`${e.available ? "✓" : "✗"} ${e.name}${e.version ? ` (${e.version})` : ""}: ${e.detail}`);
    } else if (command === "scan" && sub === "markers") {
      const out = flag("--out") ?? "forma-scale-markers.pdf";
      const res = await fetch(`${http}/photogrammetry/markers.pdf${has("--letter") ? "?paper=letter" : ""}`);
      if (!res.ok) throw new Error(await res.text());
      writeFileSync(out, Buffer.from(await res.arrayBuffer()));
      console.log(`✓ ${out}: six 30 mm markers round a 250 × 170 mm field. Print at 100 % (check the 100 mm bar), lay it flat, stand the object in the middle, keep markers in every photo.`);
    } else if (command === "scan" && (sub === "jobs" || sub === "cancel")) {
      if (sub === "cancel") {
        if (!id) throw new Error("forma scan cancel <job id>");
        await call("POST", `/photogrammetry/${id}/cancel`);
        console.log(`✓ cancelling ${id}`);
      } else {
        const list = await call("GET", "/photogrammetry");
        if (!list.length) console.log("no photogrammetry jobs since the engine started");
        for (const j of list) console.log(`${j.id}  ${j.name}: ${j.engine}, ${j.images} photos, ${j.state} ${Math.round(j.progress * 100)}% (${j.stage})${j.error ? ` — ${j.error}` : ""}`);
      }
    } else if (command === "scan" && sub === "render-photos") {
      // A photo set of the model drawn by the engine from many angles (a known shape to test photogrammetry on).
      const dir = args[2];
      if (!dir) throw new Error("forma scan render-photos <folder> [--count 40] [--rings 25,50] [--distance mm] [--fov 45] [--size 1600x1200] [--sheet] [--light 30]");
      const count = num(flag("--count")) ?? 40;
      const rings = nums(flag("--rings")) ?? [25, 50];
      const distance = flag("--distance");
      const fov = flag("--fov") ?? "45";
      const size = flag("--size") ?? "1600x1200";
      const light = flag("--light") ?? "30";
      const sheet = has("--sheet");
      const { mkdirSync } = await import("node:fs");
      mkdirSync(dir, { recursive: true });
      const cameras = [];
      const per = Math.ceil(count / rings.length);
      let n = 0;
      for (const [r, el] of rings.entries()) {
        for (let i = 0; i < per && n < count; i++, n++) {
          const az = (360 * i) / per + r * (180 / per);
          const qs = new URLSearchParams({ ...(project ? { project } : {}), orbit: `${az.toFixed(2)},${el}${distance ? `,${distance}` : ""}`, fov, size, light, sheet: sheet ? "1" : "0" });
          const res = await fetch(`${http}/look?${qs}`);
          if (!res.ok) throw new Error(await res.text());
          const name = `photo_${String(n + 1).padStart(3, "0")}.png`;
          writeFileSync(`${dir}/${name}`, Buffer.from(await res.arrayBuffer()));
          cameras.push({ image: name, azimuth: az, elevation: el, ...JSON.parse(res.headers.get("x-forma-camera")) });
          process.stdout.write(`\r  ${n + 1}/${count} photos`);
        }
      }
      writeFileSync(`${dir}/cameras.json`, JSON.stringify(cameras, null, 1));
      console.log(`\n✓ ${count} photos in ${dir} (${size}, ${fov}° field of view, rings at ${rings.join("°, ")}°${sheet ? ", on the marker sheet" : ""}); the true cameras in ${dir}/cameras.json`);
    } else if (command === "scan" && sub === "photos") {
      // Photos from many angles → a scan (Object Capture or COLMAP), sized from the marker sheet.
      const engineName = flag("--engine") ?? "auto";
      const detail = flag("--detail") ?? "medium";
      const name = flag("--name");
      const mask = has("--mask");
      const noClean = has("--no-clean");
      const detach = has("--detach");
      const paths = args.slice(2).filter((a) => !a.startsWith("--"));
      if (!paths.length) throw new Error("forma scan photos <folder | photos.zip | photo …> [--engine auto|object-capture|colmap] [--detail preview|reduced|medium|full] [--mask] [--name N] [--no-clean] [--detach]");
      const { statSync, readdirSync } = await import("node:fs");
      let files = [];
      for (const p of paths) {
        if (statSync(p).isDirectory()) files.push(...readdirSync(p).filter((f) => /\.(jpe?g|png|heic|heif|zip)$/i.test(f)).sort().map((f) => `${p}/${f}`));
        else files.push(p);
      }
      if (!files.length) throw new Error("no JPG, PNG, HEIC or zip files there");
      const set = (flag("--set") ?? name ?? basename(paths[0]).replace(/\.zip$/i, "")).replace(/[^A-Za-z0-9 ._-]+/g, "-").slice(0, 80) || "photos";
      let count = 0;
      for (const f of files) {
        const r = await call("POST", `/photos/${encodeURIComponent(set)}/${encodeURIComponent(basename(f))}`, readFileSync(f));
        count = r.photos;
        process.stdout.write(`\r  uploading: ${count} photos in “${set}”`);
      }
      const qs = new URLSearchParams({ set, engine: engineName, detail, ...(mask ? { mask: "1" } : {}), ...(noClean ? { clean: "0" } : {}), ...(name ? { name } : {}) });
      const job = await call("POST", `/photogrammetry?${qs}`);
      console.log(`\n✓ job ${job.id}: ${job.engine}, ${job.images} photos, detail ${job.detail}`);
      if (detach) {
        console.log(`  running in the background: forma scan jobs · forma scan cancel ${job.id}`);
      } else {
        let last = "";
        let j = job;
        while (j.state === "queued" || j.state === "running") {
          await new Promise((r) => setTimeout(r, 1000));
          j = (await call("GET", "/photogrammetry")).find((x) => x.id === job.id) ?? j;
          const line = `  ${Math.round(j.progress * 100)}% ${j.stage}`;
          if (line !== last) process.stdout.write(`\r${line.padEnd(60)}`);
          last = line;
        }
        process.stdout.write("\n");
        if (j.state !== "done") throw new Error(`${j.state}: ${j.error ?? ""}`);
        const r = j.result;
        const qa = r.quality;
        const sc = r.scale;
        console.log(`✓ ${sayScan(r.scan)}`);
        if (r.clean) console.log(`✓ cleaned: ${sayScan(r.clean)}`);
        else if (r.cleanError) console.log(`  cleaning failed: ${r.cleanError}`);
        else if (r.cleanSkipped) console.log(`  not cleaned: ${r.cleanSkipped}`);
        console.log(`  photos: ${qa.used} of ${qa.images} used${qa.rejected.length ? `; rejected: ${qa.rejected.map((x) => x.image ?? `#${x.sample}`).join(", ")}` : ""}${qa.reprojectionPx != null ? `; reprojection error ${qa.reprojectionPx.toFixed(2)} px` : ""}`);
        console.log(`  scale: ${sc.method}${sc.uncertainty != null ? ` (±${(sc.uncertainty * 100).toFixed(2)} %)` : ""} — ${sc.detail}`);
        console.log(`  coverage by 30° sector: ${qa.coverage.sectors.join(" ")}; elevations ${Math.round(qa.coverage.lowest_elevation)}°…${Math.round(qa.coverage.highest_elevation)}°${qa.holes != null ? `; ${qa.holes} open hole(s) in the raw surface` : ""}`);
        for (const a of qa.advice) console.log(`  ! ${a}`);
        console.log(`  next: forma mesh check ${(r.clean ?? r.scan).id.slice(0, 12)} · forma scan fit … · forma inspect ${(r.clean ?? r.scan).id.slice(0, 12)} <part> --tolerance 0.5 --align best`);
      }
    } else if (command === "scan" && (sub === "scale" || sub === "level")) {
      if (!id) throw new Error("forma scan scale <scan> --from x,y,z --to x,y,z --distance mm [--pick mm] | --factor f  ·  forma scan level <scan>");
      const body =
        sub === "scale" ? (args.includes("--factor") ? { factor: num(flag("--factor")) } : { from: nums(flag("--from")), to: nums(flag("--to")), distance: num(flag("--distance")), pick: num(flag("--pick")) }) : { tolerance: num(flag("--tolerance")) };
      const name = flag("--name");
      const r = await call("POST", `/scans/${id}/transform/${sub}`, { ...body, ...(name ? { name } : {}) });
      console.log(`✓ ${sayScan(r.scan)}\n  ${r.report.detail}`);
    } else if (command === "scan" && sub === "quality") {
      const e = (await call("GET", "/scans")).find((x) => x.id.startsWith(id) || x.name === id);
      if (!e) throw new Error(`no scan ${id}`);
      const p = e.provenance;
      if (!p) console.log(`${sayScan(e)}: no record beyond “${e.source}”`);
      else console.log(JSON.stringify({ ...p, cameras: p.cameras ? `${p.cameras.length} camera poses` : undefined }, null, 1));
    } else if (!id && !(command === "mesh" && sub === "from")) {
      throw new Error(`say which scan: forma ${command} ${command === "inspect" ? "<scan> <part> --tolerance mm" : `${sub ?? "<what>"} <scan>`}`);
    } else if (command === "scan" && sub === "info") {
      const r = await call("POST", `/scans/${id}/query/info`, {});
      console.log(`${r.kind}: ${r.points} points, spacing ${r3(r.spacing)} mm, ${p3(r.bounds.slice(0, 3))} to ${p3(r.bounds.slice(3))}${r.normals ? ", with normals" : ""}${r.colors ? ", coloured" : ""}`);
      if (r.kind === "mesh") console.log(`  ${r.triangles} triangles; ${r.solid ? "a closed solid" : `not a solid: ${r.problems}`}; volume ${r3(r.volume)} mm³`);
    } else if (command === "scan" && sub === "fit") {
      const kind = args[3] && !args[3].startsWith("--") ? args[3] : "auto";
      const ransac = has("--ransac");
      const threshold = num(flag("--threshold"));
      const f = await call("POST", `/scans/${id}/query/fit`, { kind, ransac, ...(threshold ? { threshold } : {}), ...region() });
      console.log(`✓ ${sayFit(f)}`);
      for (const a of f.alternatives ?? []) console.log(`  also ${a.kind}: RMS ${a.rms.toExponential(2)} mm`);
    } else if (command === "scan" && sub === "detect") {
      const kinds = flag("--kinds");
      const threshold = num(flag("--threshold"));
      const r = await call("POST", `/scans/${id}/query/detect`, { ...(kinds ? { kinds: kinds.split(",") } : {}), ...(threshold ? { threshold } : {}), ...region() });
      console.log(`✓ ${r.shapes.length} shapes in ${r.points} points (${r.unassigned} left over)`);
      r.shapes.forEach((f, i) => console.log(`  ${i + 1}. ${sayFit(f)}`));
    } else if (command === "scan" && ["downsample", "outliers", "normals"].includes(sub)) {
      const o = { voxel: num(flag("--voxel")), count: num(flag("--count")), neighbours: num(flag("--neighbours")), sigma: num(flag("--sigma")), toward: nums(flag("--toward")) };
      const r = await call("POST", `/scans/${id}/points/${sub}`, o);
      console.log(`✓ ${sub}: ${r.report.points[0]} → ${r.report.points[1]} points${r.report.removed ? ` (${r.report.removed} removed)` : ""}${r.report.voxel ? `, voxel ${r3(r.report.voxel)} mm` : ""}\n  new scan ${sayScan(r.scan)}`);
    } else if (command === "scan" && sub === "reconstruct") {
      const r = await call("POST", `/scans/${id}/surface/reconstruct`, { depth: num(flag("--depth")), trim: num(flag("--trim")), smooth: num(flag("--smooth")), ...region() });
      const t = r.report;
      console.log(`✓ Poisson surface (depth ${t.depth}) from ${t.points} points: ${t.triangles} triangles, ${t.closed ? "closed" : `${t.holes} openings`}; passes the points within RMS ${r3(t.rms)} mm, worst ${r3(t.max)} mm\n  new scan ${sayScan(r.scan)}`);
    } else if (command === "scan" && sub === "surface") {
      const p = flag("--poles");
      const s = await call("POST", `/scans/${id}/query/surface`, { poles: p?.includes(",") ? nums(p) : num(p ?? "8"), degree: num(flag("--degree") ?? "3"), smoothing: num(flag("--smoothing")), ...region() });
      const full = (await call("GET", "/scans")).find((e) => e.id.startsWith(id) || e.name === id)?.id ?? id;
      console.log(`✓ B-spline surface ${s.nu}×${s.nv} poles, degree ${s.degree[0]}: RMS ${r3(s.rms)} mm, worst ${r3(s.max)} mm over ${s.points} points\n  in a program: fitSurface("${full}", { poles: ${p ?? 8}, thickness: 1 })`);
    } else if (command === "scan" && sub === "shape") {
      const o = { name: flag("--name"), repair: has("--repair") || undefined, decimate: num(flag("--decimate")), smooth: num(flag("--smooth")), refine: has("--refine") || undefined, depth: num(flag("--depth")) };
      const r = await call("POST", `/scans/${id}/shape`, o);
      console.log(`✓ part “${r.name}” (${r.id}):\n${r.source.replace(/^/gm, "  ")}`);
    } else if (command === "scan" && sub === "export") {
      const out = flag("--out");
      const res = await fetch(`${http}/scans/${id}/file?${q}`);
      if (!res.ok) throw new Error(await res.text());
      const bytes = Buffer.from(await res.arrayBuffer());
      const suggested = res.headers.get("content-disposition")?.match(/filename="([^"]+)"/)?.[1] ?? `${id}.ply`;
      const target = out ? resolve(out) : join(homedir(), "Downloads", "Forma exports", suggested);
      mkdirSync(dirname(target), { recursive: true });
      writeFileSync(target, bytes);
      console.log(`✓ ${target} (${(bytes.length / 1024).toFixed(1)} KB)`);
    } else if (command === "scan" && sub === "remove") {
      const r = await call("POST", `/scans/${id}/remove`);
      console.log(`✓ ${r.removed} taken off the list`);
    } else if (command === "mesh" && sub === "check") {
      const r = await call("POST", `/scans/${id}/query/check`, {});
      console.log(`${r.solid ? "✓" : "✗"} ${report(r)}`);
      for (const [k, list] of Object.entries(r.at)) for (const p of list.slice(0, 3)) console.log(`    ${k.replace(/_/g, " ")} at ${Array.isArray(p[0]) ? p3(p[0]) : p3(p)}`);
      failed = !r.solid;
    } else if (command === "scan" && ["crop", "merge"].includes(sub)) {
      const r = await call("POST", `/scans/${id}/points/${sub}`, { ...region(), ...(sub === "merge" ? { with: args.slice(3) } : {}) });
      console.log(`✓ ${sub}: new scan ${sayScan(r.scan)}`);
    } else if (command === "mesh" && sub === "trim") {
      const r = await call("POST", `/scans/${id}/mesh/trim`, { origin: nums(flag("--origin")), normal: nums(flag("--normal")) ?? [0, 0, 1] });
      console.log(`✓ trimmed: ${r.report.triangles[0]} → ${r.report.triangles[1]} triangles\n  new scan ${sayScan(r.scan)}`);
    } else if (command === "mesh" && sub === "project") {
      const pts = JSON.parse(args[3] ?? "[]");
      const r = await call("POST", `/scans/${id}/query/projectCurve`, { points: pts, step: num(flag("--step") ?? "1") });
      console.log(`✓ ${r.points.length} points on the surface`);
      console.log(JSON.stringify(r.points.map((p) => p.map(r3))));
    } else if (command === "mesh" && ["repair", "decimate", "smooth", "remesh", "fill-holes", "flip", "harmonize", "largest", "split", "merge", "sample"].includes(sub)) {
      const op = { "fill-holes": "fillHoles" }[sub] ?? sub;
      const maxEdges = num(flag("--max-edges"));
      const o = {
        fillHoles: has("--no-fill") ? false : maxEdges,
        largestOnly: has("--largest") || undefined,
        minComponent: num(flag("--min-component")),
        ratio: num(flag("--ratio")),
        triangles: num(flag("--triangles")),
        maxError: num(flag("--max-error")),
        iterations: num(flag("--iterations")),
        method: has("--laplacian") ? "laplacian" : undefined,
        edge: num(flag("--edge")),
        maxEdges,
        count: num(flag("--count")),
        name: flag("--name") ?? undefined,
        with: sub === "merge" ? args.slice(3) : undefined,
      };
      const r = await call("POST", `/scans/${id}/${sub === "sample" ? "surface" : "mesh"}/${op}`, o);
      if (sub === "split") {
        console.log(`✓ ${r.pieces} pieces`);
        for (const e of r.scans) console.log(`  ${sayScan(e)}`);
      } else {
        const t = r.report;
        if (t?.before) {
          console.log(`${t.after.solid ? "✓" : "✗"} repaired:`);
          for (const s of t.steps) console.log(`  ${s.count} ${s.step}`);
          console.log(`  before: ${report(t.before)}\n  after:  ${report(t.after)}`);
          failed = !t.after.solid;
        } else if (t) {
          console.log(`✓ ${sub}: ${t.triangles ? `${t.triangles[0]} → ${t.triangles[1]} triangles` : ""}${t.max_deviation != null ? `, strays at most ${r3(t.max_deviation)} mm (mean ${r3(t.mean_deviation)})` : ""}${t.volume_change != null ? `, volume ${t.volume_change >= 0 ? "+" : ""}${r3(t.volume_change)} mm³` : ""}${t.filled != null ? `, ${t.filled} holes filled${t.skipped ? `, ${t.skipped} too large` : ""}` : ""}`);
        }
        console.log(`  new scan ${sayScan(r.scan)}`);
      }
    } else if (command === "mesh" && sub === "section") {
      const count = num(flag("--count"));
      const origin = nums(flag("--origin"));
      const normal = nums(flag("--normal")) ?? [0, 0, 1];
      const out = flag("--out");
      const r = await call("POST", `/scans/${id}/query/section`, { normal, ...(origin ? { origin } : {}), ...(count ? { count } : {}) });
      for (const s of r) {
        console.log(`plane through ${p3(s.origin)} normal ${p3(s.normal)}: ${s.polylines.length} ${s.polylines.length === 1 ? "loop" : "loops"}`);
        for (const l of s.polylines) console.log(`  ${l.closed ? "closed" : "open"}, ${l.points.length} points, length ${r3(l.length)} mm${l.closed ? `, area ${r3(Math.abs(l.area))} mm²` : ""}`);
      }
      if (out) writeFileSync(out, JSON.stringify(r));
    } else if (command === "mesh" && sub === "curvature") {
      const r = await call("POST", `/scans/${id}/query/curvature`, {});
      for (const k of ["mean", "gaussian", "k1", "k2"]) console.log(`  ${k.padEnd(8)} min ${r3(r[k].min)}  5% ${r3(r[k].p5)}  median ${r3(r[k].median)}  95% ${r3(r[k].p95)}  max ${r3(r[k].max)}  (1/mm${k === "gaussian" ? "²" : ""})`);
    } else if (command === "mesh" && sub === "segment") {
      const r = await call("POST", `/scans/${id}/query/segment`, { by: has("--planes") ? "planes" : "curvature", angle: num(flag("--angle")), threshold: num(flag("--threshold")) });
      console.log(`✓ ${r.segments.length} regions`);
      for (const s of r.segments.slice(0, 40)) console.log(`  ${s.id}. ${s.triangles} triangles, ${r3(s.area)} mm² at ${p3(s.centroid)}${s.fit ? ` — ${sayFit(s.fit)}` : ""}`);
    } else if (command === "mesh" && sub === "boundaries") {
      const r = await call("POST", `/scans/${id}/query/boundaries`, {});
      console.log(`${r.loops.length} open-edge loops`);
      for (const l of r.loops.slice(0, 20)) console.log(`  ${l.length} edges from ${p3(l[0])}`);
    } else if (command === "mesh" && sub === "unwrap") {
      const svg = flag("--svg");
      const segment = num(flag("--segment"));
      const r = await call("POST", `/scans/${id}/unwrap`, { ...region(), ...(segment != null ? { segment } : {}), sketch: has("--sketch"), name: flag("--name") ?? undefined });
      console.log(`✓ flat pattern ${r3(r.size[0])} × ${r3(r.size[1])} mm, ${r.outlines.length} outline${r.outlines.length === 1 ? "" : "s"}; stretch: area ${(r.area_distortion_mean * 100).toFixed(2)}% mean, ${(r.area_distortion_max * 100).toFixed(2)}% worst; lengths ${(r.length_distortion_mean * 100).toFixed(2)}% mean, ${(r.length_distortion_max * 100).toFixed(2)}% worst`);
      if (r.sketch) console.log(`  sketch “${r.sketch.name}” (${r.sketch.id}): sketch("${r.sketch.name}").extrude(…)`);
      if (svg) {
        writeFileSync(svg, r.svg);
        console.log(`  ${svg}`);
      }
    } else if (command === "mesh" && sub === "from") {
      const part = args[2];
      if (!part) throw new Error("forma mesh from <part id> [--tolerance 0.01] [--angle 20] [--max-edge 2] [--name N]");
      const r = await call("POST", `/mesh-from/${part}`, { tolerance: num(flag("--tolerance")), angle: num(flag("--angle")), maxEdge: num(flag("--max-edge")), name: flag("--name") ?? undefined });
      console.log(`✓ ${r.triangles} triangles, ${r.solid ? "closed" : "NOT closed"}\n  new scan ${sayScan(r.scan)}${r.faceted.length ? `\n  faceted already: ${r.faceted.join("; ")}` : ""}`);
    } else if (command === "inspect") {
      const target = args[2];
      const tolerance = num(flag("--tolerance"));
      if (!target || !tolerance) throw new Error("forma inspect <scan> <part id or name> --tolerance mm [--required 0.99] [--align icp|best] [--part P] [--require]");
      const align = flag("--align");
      const part = flag("--part");
      const required = num(flag("--required"));
      const require = has("--require");
      const at = nums(flag("--at"));
      const r = await call("POST", `/scans/${id}/inspect`, { target, tolerance, ...(part ? { part } : {}), ...(required ? { required } : {}), ...(align ? { align } : {}), ...(at ? { at } : {}), max: 1 });
      const s = r.stats;
      console.log(`${r.passed ? "✓ PASS" : "✗ FAIL"} ${r.scan.name} against ${r.target.name}${part ? ` › ${part}` : ""}: ${(r.within * 100).toFixed(2)}% of ${s.count} points within ±${tolerance} mm (${((r.required ?? 1) * 100).toFixed(0)}% required)`);
      console.log(`  deviation min ${r3(s.min)}  max ${r3(s.max)}  mean ${r3(s.mean)}  |mean| ${r3(s.mean_abs)}  RMS ${r3(s.rms)}  σ ${r3(s.std)}  95% within ±${r3(s.p95)} mm`);
      console.log(`  ${r.over} points over (material where there should be none), ${r.under} under (missing material)`);
      if (r.alignment) console.log(`  best-fit aligned: RMS ${r3(r.alignment.rms_before)} → ${r3(r.alignment.rms_after)} mm in ${r.alignment.iterations} steps`);
      for (const w of r.worst.slice(0, 3)) console.log(`  worst ${w.deviation >= 0 ? "+" : ""}${r3(w.deviation)} mm at ${p3(w.at)}`);
      if (r.probe) console.log(`  at ${p3(r.probe.point)} (nearest scan point, ${r3(r.probe.distance_from_probe)} mm from the spot asked): ${r.probe.deviation >= 0 ? "+" : ""}${r3(r.probe.deviation)} mm from the part (its surface at ${p3(r.probe.on_part)})`);
      const bins = r.histogram;
      const top = Math.max(...bins, 1);
      console.log(`  histogram (−4…+4 × tol/2): ${bins.map((b) => " ▁▂▃▄▅▆▇█"[Math.ceil((b / top) * 8)]).join("")}`);
      if (require) {
        const reply = await engine.commit("intent.requirement.create", { rule: { kind: "scan_deviation", part: r.target.id, ...(part ? { part_name: part } : {}), scan: r.scan.id, tolerance: { value: tolerance, unit: "mm" }, ...(required ? { within: required } : {}), ...(align ? { align: true } : {}) } });
        if (!reply.ok) throw new Error(reply.error?.message ?? "could not add the requirement");
        console.log(`  ✓ requirement added (${reply.value.output.id}): checked on every change`);
      }
      failed = !r.passed;
    } else {
      throw new Error(`forma scan add|list|info|fit|detect|downsample|outliers|normals|reconstruct|surface|shape|export|remove …
forma mesh check|repair|decimate|smooth|remesh|fill-holes|flip|harmonize|largest|split|merge|sample|section|curvature|segment|boundaries|unwrap <scan> … · forma mesh from <part>
forma inspect <scan> <part> --tolerance mm [--align icp|best] [--required 0.99] [--require]`);
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else if (command === "bim" || command === "schedule" || command === "ifc" || command === "plan") {
  // Building information: the structure, schedules, IFC in and out, plans/sections/elevations.
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const flag = (name, takes = true) => {
    const i = args.indexOf(name);
    if (i < 0) return null;
    return takes ? args.splice(i, 2)[1] : (args.splice(i, 1), true);
  };
  const q = new URLSearchParams(project ? { project } : {});
  const write = async (res, out) => {
    const { writeFileSync } = await import("node:fs");
    const name = out ?? decodeURIComponent((res.headers.get("content-disposition") ?? "").match(/filename="([^"]+)"/)?.[1] ?? "forma-output");
    writeFileSync(name, Buffer.from(await res.arrayBuffer()));
    return name;
  };
  try {
    if (command === "bim") {
      const id = args[1] ?? "project";
      const res = await fetch(`${http}/bim/${encodeURIComponent(id)}?${q}`);
      if (!res.ok) throw new Error(await res.text());
      const b = await res.json();
      console.log(`${b.building?.name || b.project} on ${b.site?.name ?? "its site"}: ${b.elements} elements, ${b.openings} openings`);
      for (const l of b.levels) {
        const els = Object.entries(l.elements).map(([c, n]) => `${n} ${c.replace(/^Ifc/, "")}`).join(", ");
        console.log(`  ${l.name.padEnd(16)} +${(l.elevation / 1000).toFixed(3)} m${l.height ? `, ${l.height} mm high` : ""}: ${els || "nothing"}`);
        for (const s of l.spaces) console.log(`      ${s.name}${s.long_name ? ` (${s.long_name})` : ""}: ${s.area_m2} m², ${s.height} mm high`);
      }
      const loose = b.classes && Object.entries(b.classes).length ? `classes: ${Object.entries(b.classes).map(([c, n]) => `${c} ${n}`).join(", ")}` : "";
      if (loose) console.log(`  ${loose}`);
      for (const z of b.zones) console.log(`  zone ${z.name}: ${z.spaces.join(", ")}`);
      for (const g of b.grids) console.log(`  grid ${g.name}${g.level ? ` (${g.level})` : ""}: ${g.axes.join(" ")}`);
      for (const g of b.groups) console.log(`  ${g.class} ${g.name}${g.level ? ` (${g.level})` : ""}`);
      if (b.untagged.length) console.log(`  ! no IFC class (written as proxies): ${b.untagged.join(", ")}`);
      for (const n of b.notes) console.log(`  ! ${n}`);
    } else if (command === "schedule") {
      const cls = flag("--class"), lvl = flag("--level"), by = flag("--by"), csv = flag("--csv", false), psets = flag("--psets", false), out = flag("--out");
      const id = args[1] ?? "project";
      if (cls) q.set("class", cls);
      if (lvl) q.set("level", lvl);
      if (by) q.set("group", by);
      if (psets) q.set("psets", "1");
      q.set("format", csv || out ? "csv" : "json");
      const res = await fetch(`${http}/schedule/${encodeURIComponent(id)}?${q}`);
      if (!res.ok) throw new Error(await res.text());
      if (out) console.log(`✓ wrote ${await write(res, out)}`);
      else if (csv) process.stdout.write(await res.text());
      else {
        const { schedule: s } = await res.json();
        const fmt = (v, u) => (u === "m²" || u === "m³" ? v.toFixed(2) : u === "kg" ? v.toFixed(1) : String(Math.round(v * 100) / 100));
        const cols = s.columns.filter((c) => s.rows.some((r) => r.quantities[c] !== undefined)).slice(0, 8);
        if (s.group_by.length) {
          const main = ["Length", "NetSideArea", "NetArea", "NetFloorArea", "Area", "NetVolume", "Volume", "TotalLength", "Weight", "Openings"];
          for (const g of [...s.groups, s.total]) console.log(`${g.key.join(" · ").padEnd(36)} ${String(g.count).padStart(4)}  ${Object.entries(g.sums).filter(([k]) => main.includes(k)).sort(([a], [b]) => main.indexOf(a) - main.indexOf(b)).map(([k, v]) => `${k} ${fmt(v, s.units[k])}${s.units[k] ? ` ${s.units[k]}` : ""}`).join(", ")}`);
        } else {
          for (const r of s.rows) console.log(`${(r.level ?? "—").padEnd(14)} ${r.class.replace(/^Ifc/, "").padEnd(18)} ${r.name.padEnd(28)} ${r.tag ? `[${r.tag}] ` : ""}${r.type ?? ""}  ${cols.filter((c) => r.quantities[c] !== undefined).map((c) => `${c} ${fmt(r.quantities[c], s.units[c])}`).join(", ")}`);
          console.log(`Total: ${s.total.count} items; ${Object.entries(s.total.sums).filter(([k]) => cols.includes(k)).map(([k, v]) => `${k} ${fmt(v, s.units[k])}${s.units[k] ? ` ${s.units[k]}` : ""}`).join(", ")}`);
        }
      }
    } else if (command === "ifc") {
      const [, action, a, b] = args;
      if (action === "export") {
        const id = a && !a.endsWith(".ifc") ? a : "project";
        const out = a?.endsWith(".ifc") ? a : b;
        const res = await fetch(`${http}/ifc/${encodeURIComponent(id)}.ifc?${q}`);
        if (!res.ok) throw new Error(await res.text());
        console.log(`✓ wrote ${await write(res, out)} (IFC4)`);
      } else if (action === "import" && a) {
        const { readFileSync } = await import("node:fs");
        const { basename } = await import("node:path");
        const res = await fetch(`${http}/import/${encodeURIComponent(basename(a))}?${q}`, { method: "POST", body: readFileSync(a) });
        if (!res.ok) throw new Error(await res.text());
        const r = await res.json();
        console.log(`✓ imported ${a} as shape ${r.id}`);
      } else throw new Error("forma ifc export [id|project] [file.ifc] | forma ifc import <file.ifc>");
    } else if (command === "plan") {
      const lvl = flag("--level"), cut = flag("--cut"), sheet = flag("--sheet"), scale = flag("--scale"), pdf = flag("--pdf"), dxf = flag("--dxf"), looking = flag("--looking"), side = flag("--elevation");
      const si = args.indexOf("--section");
      const section = si >= 0 ? args.splice(si, 3).slice(1) : null;
      const id = args[1] ?? "project";
      if (section) {
        q.set("view", "section");
        q.set("axis", section[0]);
        q.set("at", section[1]);
        if (looking) q.set("looking", looking);
      } else if (side) {
        q.set("view", "elevation");
        q.set("side", side);
      } else {
        q.set("view", "plan");
        if (lvl) q.set("level", lvl);
        if (cut) q.set("cut", cut);
      }
      if (sheet) q.set("sheet", sheet);
      if (scale) q.set("scale", scale);
      const res = await fetch(`${http}/plan/${encodeURIComponent(id)}?${q}`);
      if (!res.ok) throw new Error(await res.text());
      const d = await res.json();
      console.log(`✓ ${d.texts.find((t) => t.bold && t.size >= 3.5)?.text ?? "drawing"}: ${d.scale_text} on ${d.width}×${d.height} mm, ${d.strokes.length} lines, ${d.fills.length} filled, ${d.texts.length} texts`);
      for (const w of d.warnings) console.log(`  ! ${w}`);
      for (const [ext, out] of [["pdf", pdf], ["dxf", dxf]]) {
        if (!out) continue;
        const r = await fetch(`${http}/plan/${encodeURIComponent(id)}.${ext}?${q}`);
        if (!r.ok) throw new Error(await r.text());
        console.log(`✓ wrote ${await write(r, out === true || out === "-" ? undefined : out)}`);
      }
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
} else if (["dof", "interference", "exploded", "study", "robot"].includes(command)) {
  // Assembly analyses (GET, nothing changes).
  const http = base.replace(/^ws/, "http").replace(/\/ws(\?.*)?$/, "");
  const flag = (name) => {
    const i = args.indexOf(name);
    return i >= 0 ? args.splice(i, 2)[1] : null;
  };
  const tool = flag("--tool");
  const axis = flag("--axis");
  const fps = flag("--fps");
  const out = flag("--out");
  const [, id, sub, extra] = args;
  const q = new URLSearchParams(project ? { project } : {});
  const get = async (path) => {
    const res = await fetch(`${http}/${path}${path.includes("?") ? "&" : "?"}${q}`);
    if (!res.ok) throw new Error(await res.text());
    return res.json();
  };
  const vec = (s) => `[${String(s).split(/[,\s]+/).filter(Boolean).map(Number).join(",")}]`;
  const r1 = (v) => Math.round(v * 10) / 10;
  try {
    if (!id) throw new Error(`say which: forma ${command} <${command === "robot" ? "tip instance" : command === "study" || command === "exploded" ? "id or name" : "assembly | top"}>`);
    if (command === "dof") {
      const r = await get(`assembly-dof/${encodeURIComponent(id)}`);
      console.log(`${r.groups.some((g) => !g.solved) ? "✗" : "✓"} ${r.summary}`);
      for (const g of r.groups) {
        console.log(`  ${g.solved ? "·" : "✗"} ${g.joints.map((j) => `${j.name} (${j.kind})`).join(", ")}\n      ${g.summary}`);
        for (const i of g.instances) if (i.free) console.log(`      ${i.name}: ${i.free} free — ${i.motions.join("; ")}`);
        if (g.error) console.log(`      ! ${g.error}`);
      }
      if (r.unjoined.length) console.log(`  joined to nothing: ${r.unjoined.map((u) => u.name).join(", ")}`);
      failed = r.groups.some((g) => !g.solved);
    } else if (command === "interference") {
      const r = await get(`interference/${encodeURIComponent(id)}${sub ? `?clearance=${sub}` : ""}`);
      const bad = r.overlaps.filter((o) => !o.joined && !o.fastener);
      console.log(`${bad.length ? "✗" : "✓"} ${r.name}: ${r.parts} parts, ${r.checked} close pairs checked; ${r.overlaps.length} overlap, ${r.touching.length} touch${r.clearance ? `, ${r.near.length} within ${r.clearance} mm` : ""}`);
      for (const o of [...bad, ...r.overlaps.filter((x) => !bad.includes(x))]) console.log(`  overlap ${o.a} ↔ ${o.b}: ${o.volume} mm³ (${o.size?.join(" × ")} mm at ${o.at?.join(", ")})${o.fastener ? " [fastener]" : o.joined ? " [joined]" : ""}`);
      for (const o of r.near) console.log(`  near    ${o.a} ↔ ${o.b}: ${o.gap} mm`);
      if (sub === "all" || r.touching.length <= 12) for (const o of r.touching) console.log(`  touch   ${o.a} ↔ ${o.b}${o.joined ? " [joined]" : ""}`);
      failed = bad.length > 0;
    } else if (command === "exploded") {
      const r = await get(`explode/${encodeURIComponent(id)}`);
      console.log(`✓ ${r.name}: ${r.steps.length - 1} steps, ${r.trails.length} parts move`);
      for (const t of r.trails) {
        const name = r.instances.find((i) => i.id === t.instance)?.name;
        console.log(`  ${name}: ${t.points.map((p) => `(${p.map(r1).join(", ")})`).join(" → ")}`);
      }
    } else if (command === "study") {
      const r = await get(`motion-study/${encodeURIComponent(id)}?fps=${fps ?? sub ?? 24}`);
      const moving = r.instances.filter((i) => i.moves).length;
      console.log(`${r.clashes.length || r.binds_at != null ? "✗" : "✓"} ${r.frames.length} frames over ${r.frames.at(-1).value} s; ${moving} of ${r.instances.length} instances move`);
      for (const c of r.clashes) console.log(`  clash  ${c.a} ↔ ${c.b} from ${c.range?.[0] ?? c.at} s to ${c.range?.[1] ?? c.at} s`);
      for (const n of r.notes) console.log(`  ! ${n}`);
      if (out) {
        (await import("node:fs")).writeFileSync(out, JSON.stringify(r));
        console.log(`  frames written to ${out} (every instance's position and quaternion per frame)`);
      }
      failed = r.clashes.length > 0 || r.binds_at != null;
    } else if (command === "robot") {
      const t = encodeURIComponent(id);
      const toolQ = `${tool ? `tool=${encodeURIComponent(tool)}&` : ""}`;
      if (!sub) {
        const r = await get(`robot/${t}?${toolQ}`);
        console.log(`✓ ${r.tip_name}: ${r.axes.length} moving joints from the ground; tool at (${r.tool.at.map(r1).join(", ")}) pointing (${r.tool.axis.map((v) => Math.round(v * 1000) / 1000).join(", ")})`);
        for (const a of r.axes) console.log(`  ${a.name.padEnd(16)} ${String(r.values[a.name]).padStart(8)} ${a.unit}   limits ${a.min}…${a.max}`);
      } else if (sub === "fk") {
        const r = await get(`robot/${t}/fk?${toolQ}values=${encodeURIComponent(extra ?? "{}")}`);
        console.log(`✓ tool at (${r.at.map(r1).join(", ")}) pointing (${r.axis.map((v) => Math.round(v * 1000) / 1000).join(", ")})`);
      } else if (sub === "ik") {
        const r = await get(`robot/${t}/ik?${toolQ}to=${vec(extra)}${axis ? `&axis=${vec(axis)}` : ""}`);
        console.log(`${r.reached ? "✓ reaches" : `✗ out of reach: ${r.error_mm} mm short`}${r.error_deg ? `, axis ${r.error_deg}° off` : ""}: ${Object.entries(r.values).map(([k, v]) => `${k} ${v}`).join(", ")}`);
        if (r.reached) console.log(`  set them: forma commit assembly.robot.move_to '{"tip":"${id}","to":${vec(extra)}${axis ? `,"axis":${vec(axis)}` : ""}${tool ? `,"tool":"${tool}"` : ""}}'`);
        failed = !r.reached;
      } else if (sub === "reach") {
        const r = await get(`robot/${t}/reach?${toolQ}samples=${extra ?? 2000}`);
        console.log(`✓ ${r.samples} poses: the tool reaches ${r.min_reach}–${r.max_reach} mm from (${r.base.map(r1).join(", ")}); box (${r.bounds_min.map(r1).join(", ")}) to (${r.bounds_max.map(r1).join(", ")})`);
      } else if (sub === "path") {
        const r = await get(`robot/${t}/trajectory?${toolQ}fps=${fps ?? 12}&through=${encodeURIComponent(extra ?? "[]")}`);
        console.log(`${r.clashes.length || r.binds_at != null || r.waypoints.some((w) => !w.reached) ? "✗" : "✓"} ${r.waypoints.length} waypoints in ${r.frames.at(-1).value} s (${r.frames.length} frames)`);
        r.waypoints.forEach((w, i) => console.log(`  ${i + 1}. ${w.reached ? "reached" : `missed by ${w.error_mm} mm`}: ${Object.entries(w.values).map(([k, v]) => `${k} ${v}`).join(", ")}`));
        for (const c of r.clashes) console.log(`  clash  ${c.a} ↔ ${c.b} from ${c.range?.[0] ?? c.at} s to ${c.range?.[1] ?? c.at} s`);
        if (out) (await import("node:fs")).writeFileSync(out, JSON.stringify(r));
        failed = r.clashes.length > 0 || r.binds_at != null || r.waypoints.some((w) => !w.reached);
      } else throw new Error("forma robot <tip> [fk '{json}' | ik x,y,z [--axis x,y,z] | reach [samples] | path '<waypoints json>'] [--tool connector]");
    }
  } catch (e) {
    console.error(`✗ ${e.message}`);
    failed = true;
  }
} else {
  console.error("usage: forma.mjs run steps.json | commit <op> <json> | stock [search] | flat [id] [part] | render [--width] | get <id> | tree | list | check | ops | bim | schedule | ifc export|import | plan | check-geometry <id> [part] | surfaces <id> [part] | measure <part> [other] | sheet <name> | draft | view <action> [what] | import <file> [--name N] | export <id> <file> | --format <fmt> [--part P] [--out path] | bundle [file] | image add <file> | image list | look [what] [--view v]  [--project name] [--url ws://…]");
  failed = true;
}

engine.close();

// An assembly made but never placed shows nothing in the person's view (a
// definition appears only through its instances): say so after changes and
// checks, while it is easy to put right.
if (["run", "commit", "check", "look"].includes(command)) {
  try {
    const fresh = await connect();
    const s = await fresh.snapshot;
    fresh.close();
    const placedOf = new Set(s.entities.map((e) => e.components?.["assembly.instance"]?.of).filter(Boolean));
    const unplaced = s.entities.filter((e) => e.components?.["assembly.definition"] && !placedOf.has(e.id));
    for (const e of unplaced) {
      console.log(`! assembly “${e.components["assembly.definition"].name}” (${e.id}) is not placed anywhere, so the person's view does not show it: place it with assembly.instance.create { "of": "${e.id}", "name": "…" } (no parent puts it at the top of the project)`);
    }
  } catch {
    // The engine went away: nothing to add.
  }
}
process.exit(failed ? 1 : 0);
