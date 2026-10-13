// `./forma pcb drc|setup|stackup|rules|teardrops|text|draw|image|keepout|stitch|fence|align|distribute|move|array|flip|swap|channel …`:
// board objects, zones, placement, the stackup and the custom rules language,
// through the same engine commands the Electronics workspace uses. Called
// from tools/forma.mjs with its engine connection.

import { readFileSync } from "node:fs";
import { decodeImage } from "./forma-pcblib.mjs";

export const BOARD_SUBS = ["drc", "setup", "stackup", "rules", "teardrops", "text", "draw", "image", "keepout", "stitch", "fence", "align", "distribute", "move", "array", "flip", "swap", "channel"];

const flagOf = (args, name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args.splice(i, 2)[1] : null;
};
const boolOf = (args, name) => {
  const i = args.indexOf(name);
  if (i >= 0) args.splice(i, 1);
  return i >= 0;
};
const nums = (s, n, what) => {
  const v = String(s).split(",").map(Number);
  if ((n && v.length !== n) || v.some((x) => !Number.isFinite(x))) throw new Error(`“${s}” is not ${what}`);
  return v;
};
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-/i;

export async function pcbBoard(sub, args, { engine, get, q }) {
  const commit = async (op, params) => {
    const r = await engine.commit(op, params);
    if (!r.ok) throw new Error(`${op}: ${r.error.code} ${r.error.message}`);
    return r.value.output;
  };
  const view = async (board) => (await get(`/pcb/${board}?${q()}`)).board;
  // Items by id, or components by reference (R1) on --board.
  const boardFlag = flagOf(args, "--board");
  const resolve = async (words) => {
    if (!words.some((w) => !UUID.test(w))) return words;
    if (!boardFlag) throw new Error("give --board <id> to name components by reference");
    const v = await view(boardFlag);
    return words.map((w) => {
      if (UUID.test(w)) return w;
      const c = v.components.find((c) => c.reference.toLowerCase() === w.toLowerCase());
      if (!c) throw new Error(`no component ${w} on the board`);
      return c.id;
    });
  };
  const show = (vs, filter) => {
    const list = vs.filter((x) => !filter || x.code === filter || x.code.startsWith(filter));
    for (const x of list) console.log(`  ${x.severity === "error" ? "✗" : "!"} ${x.code}: ${x.message}`);
    return list;
  };

  if (sub === "drc") {
    const board = args[2];
    if (!board) throw new Error("usage: forma pcb drc <board> [--code clearance]");
    const code = flagOf(args, "--code");
    const v = await view(board);
    const list = show(v.violations, code);
    console.log(`${list.filter((x) => x.severity === "error").length} errors, ${list.filter((x) => x.severity !== "error").length} warnings${code ? ` (${code})` : ""}`);
    return list.some((x) => x.severity === "error");
  }
  if (sub === "setup") {
    const board = args[2];
    if (!board) throw new Error("usage: forma pcb setup <board>");
    const v = await view(board);
    console.log(`${v.name}: ${v.layers.length} copper layers (${v.layers.join(" ")}), ${v.thickness} mm, ${v.finish}`);
    console.log(`Stackup (${v.stackup.custom ? "custom" : "standard"}, adds up to ${v.stackup.total} mm):`);
    for (const r of v.stackup.rows) if (r.thickness > 0 || r.kind === "copper") console.log(`  ${r.kind.padEnd(8)} ${r.name.padEnd(14)} ${r.thickness.toFixed(4)} mm  ${r.material ?? ""}${r.epsilon_r ? `  εr ${r.epsilon_r}` : ""}${r.copper_oz ? `  ${r.copper_oz} oz` : ""}`);
    console.log(`Custom rules: ${v.custom_rules.length ? v.custom_rules.map((r) => `${r.name} [${r.constraints.map((c) => c.kind).join(", ")}]`).join("; ") : "none"}`);
    console.log(`Teardrops: ${v.teardrops.length} · graphics: ${v.graphics.length} · rule areas: ${v.areas.map((a) => a.name).join(", ") || "none"} · vias: ${v.vias.length} (${[...new Set(v.vias.map((x) => x.kind))].join(", ") || "-"})`);
    return false;
  }
  if (sub === "stackup") {
    const board = args[2];
    if (!board) throw new Error("usage: forma pcb stackup <board> [--layers 6] [--thickness 1.6] [--standard] [--fit] [--file stackup.json ({dielectrics, copper_oz})]");
    const params = { board };
    const layers = flagOf(args, "--layers");
    const thickness = flagOf(args, "--thickness");
    const file = flagOf(args, "--file");
    if (layers) params.layers = Number(layers);
    if (thickness) params.thickness = Number(thickness);
    if (boolOf(args, "--standard")) params.standard = true;
    if (boolOf(args, "--fit")) params.fit_thickness = true;
    if (file) Object.assign(params, JSON.parse(readFileSync(file, "utf8")));
    const r = await commit("pcb.stackup.set", params);
    console.log(`✓ ${r.layers} layers, ${r.custom ? "custom" : "standard"} stackup ${r.total} mm (board ${r.thickness} mm)`);
    for (const row of r.rows) if (row.kind !== "silk") console.log(`  ${row.kind.padEnd(8)} ${row.name.padEnd(14)} ${row.thickness.toFixed(4)} mm  ${row.material ?? ""}`);
    return false;
  }
  if (sub === "rules") {
    const board = args[2];
    if (!board) throw new Error("usage: forma pcb rules <board> [file.kicad_dru | --clear]");
    const file = args[3];
    if (file || args.includes("--clear")) {
      const text = args.includes("--clear") ? "" : readFileSync(file, "utf8");
      const r = await commit("pcb.rules.set", { board, text });
      console.log(`✓ ${r.rules.length} rule(s) read`);
      for (const u of r.unchecked) console.log(`  · ${u}: read but not checked (routing constraints are parked)`);
    }
    const v = await view(board);
    for (const r of v.custom_rules) console.log(`  ${r.name}${r.layer ? ` [${r.layer}]` : ""}: ${r.constraints.map((c) => `${c.kind}${c.min != null ? ` min ${c.min}` : ""}${c.max != null ? ` max ${c.max}` : ""}${c.disallow?.length ? ` ${c.disallow.join(" ")}` : ""}`).join(", ")}${r.condition ? `  when ${r.condition}` : ""}`);
    const hit = v.violations.filter((x) => x.code === "rules_syntax" || /rule “/.test(x.message));
    show(hit);
    console.log(`${hit.length} violation(s) from custom rules`);
    return hit.some((x) => x.severity === "error");
  }
  if (sub === "teardrops") {
    const [board, onoff] = [args[2], args[3]];
    if (!board || !["on", "off"].includes(onoff)) throw new Error("usage: forma pcb teardrops <board> on|off [--smd] [--no-vias] [--straight] [--length 0.5] [--width 1.0]");
    const params = { board, enabled: onoff === "on" };
    if (boolOf(args, "--smd")) params.smd = true;
    if (boolOf(args, "--no-vias")) params.vias = false;
    if (boolOf(args, "--straight")) params.curved = false;
    const l = flagOf(args, "--length");
    const w = flagOf(args, "--width");
    if (l) params.length_ratio = Number(l);
    if (w) params.width_ratio = Number(w);
    await commit("pcb.teardrops.set", params);
    const v = await view(board);
    console.log(`✓ teardrops ${onoff}: ${v.teardrops.length} on the board`);
    return false;
  }
  if (sub === "text") {
    const [board, layer, text, at] = [args[2], args[3], args[4], args[5]];
    if (!board || !layer || !text || !at) throw new Error('usage: forma pcb text <board> <layer> "TEXT" x,y [--size 1.5] [--font Inter] [--weight 700] [--rotate 90] [--knockout] [--align left|center|right]');
    const params = { board, layer, kind: "text", text, at: nums(at, 2, "x,y") };
    for (const [f, k, num] of [["--size", "size", true], ["--font", "font", false], ["--weight", "weight", true], ["--rotate", "rotation", true], ["--align", "align", false], ["--width", "width", true]]) {
      const v = flagOf(args, f);
      if (v != null) params[k] = num ? Number(v) : v;
    }
    if (boolOf(args, "--knockout")) params.knockout = true;
    const r = await commit("pcb.graphic.create", params);
    console.log(`✓ ${r.id} on ${r.layer}, ${(r.bbox[2] - r.bbox[0]).toFixed(2)} × ${(r.bbox[3] - r.bbox[1]).toFixed(2)} mm`);
    return false;
  }
  if (sub === "draw") {
    const [board, layer, kind, json] = [args[2], args[3], args[4], args[5]];
    if (!board || !layer || !kind) throw new Error("usage: forma pcb draw <board> <layer> <line|arc|circle|rect|polygon|bezier|ellipse|dimension> '<json fields>' [--width 0.2] [--filled] [--net GND]");
    const params = { board, layer, kind, ...(json ? JSON.parse(json) : {}) };
    const w = flagOf(args, "--width");
    const net = flagOf(args, "--net");
    if (w) params.width = Number(w);
    if (net) params.net = net;
    if (boolOf(args, "--filled")) params.filled = true;
    const r = await commit("pcb.graphic.create", params);
    console.log(`✓ ${r.id} on ${r.layer}${r.value != null ? ` (measures ${r.value.toFixed(3)} mm: ${r.text})` : ""}`);
    return false;
  }
  if (sub === "image") {
    const [board, layer, path, at] = [args[2], args[3], args[4], args[5]];
    if (!board || !layer || !path || !at) throw new Error("usage: forma pcb image <board> <layer> <image.png|pgm> x,y --size <width mm> [--threshold 128] [--negative]");
    const size = Number(flagOf(args, "--size") ?? 10);
    const img = decodeImage(new Uint8Array(readFileSync(path)));
    const params = { board, layer, kind: "image", at: nums(at, 2, "x,y"), size, px: [img.width, img.height], pixels: Buffer.from(img.grey).toString("base64") };
    const t = flagOf(args, "--threshold");
    if (t) params.threshold = Number(t);
    if (boolOf(args, "--negative")) params.negative = true;
    const r = await commit("pcb.graphic.create", params);
    console.log(`✓ ${r.id} on ${r.layer}, ${(r.bbox[2] - r.bbox[0]).toFixed(2)} × ${(r.bbox[3] - r.bbox[1]).toFixed(2)} mm`);
    return false;
  }
  if (sub === "keepout") {
    const [board, name] = [args[2], args[3]];
    const no = flagOf(args, "--no");
    const layers = flagOf(args, "--layers");
    const pts = args.slice(4).filter((a) => !a.startsWith("--")).map((p) => nums(p, 2, "x,y"));
    if (!board || !name || pts.length < 3) throw new Error("usage: forma pcb keepout <board> <name> x,y x,y x,y … [--no tracks,vias,pads,pours,footprints] [--layers top,bottom]");
    const params = { board, name, outline: pts };
    if (layers) params.layers = layers.split(",");
    if (no) for (const k of ["tracks", "vias", "pads", "pours", "footprints"]) params[`no_${k}`] = no.split(",").includes(k);
    const r = await commit("pcb.area.create", params);
    console.log(`✓ keepout ${r.name} (${r.id})`);
    return false;
  }
  if (sub === "stitch") {
    const [board, net] = [args[2], args[3]];
    if (!board) throw new Error("usage: forma pcb stitch <board> [net] [--zone id] [--pitch 2.5] [--stagger] [--drill 0.3 --diameter 0.6]");
    const params = { board, ...(net && !net.startsWith("--") ? { net } : {}) };
    for (const [f, k, num] of [["--zone", "zone", false], ["--pitch", "pitch", true], ["--drill", "drill", true], ["--diameter", "diameter", true]]) {
      const v = flagOf(args, f);
      if (v != null) params[k] = num ? Number(v) : v;
    }
    if (boolOf(args, "--stagger")) params.stagger = true;
    const r = await commit("pcb.via.stitch", params);
    console.log(`✓ ${r.vias} ${r.net} stitching vias (group ${r.group}${r.replaced ? `, ${r.replaced} replaced` : ""})`);
    return false;
  }
  if (sub === "fence") {
    const [board, track, net] = [args[2], args[3], args[4]];
    if (!board || !track || !net) throw new Error("usage: forma pcb fence <board> <track id> <net> [--pitch 1.5] [--offset mm] [--sides both|left|right]");
    const params = { board, track, net };
    for (const [f, k, num] of [["--pitch", "pitch", true], ["--offset", "offset", true], ["--sides", "sides", false]]) {
      const v = flagOf(args, f);
      if (v != null) params[k] = num ? Number(v) : v;
    }
    const r = await commit("pcb.via.fence", params);
    console.log(`✓ ${r.vias} fence vias at ${r.offset.toFixed(2)} mm, every ${r.pitch} mm`);
    return false;
  }
  if (sub === "align") {
    const to = args[2];
    const ref = flagOf(args, "--to");
    const targets = await resolve(args.slice(3));
    if (!to || targets.length < 2) throw new Error("usage: forma pcb align <left|right|top|bottom|center_x|center_y> <ids or refs…> [--board id] [--to <reference item>]");
    const r = await commit("pcb.align", { targets, to, ...(ref ? { reference: (await resolve([ref]))[0] } : {}) });
    console.log(`✓ ${r.moved} moved to ${to} ${r.at.toFixed(3)}`);
    return false;
  }
  if (sub === "distribute") {
    const axis = args[2];
    const gaps = boolOf(args, "--gaps");
    const targets = await resolve(args.slice(3));
    if (!axis || targets.length < 3) throw new Error("usage: forma pcb distribute <x|y> <ids or refs…> [--board id] [--gaps]");
    const r = await commit("pcb.distribute", { targets, axis, ...(gaps ? { by: "gaps" } : {}) });
    console.log(`✓ ${r.moved} moved`);
    return false;
  }
  if (sub === "move") {
    const by = flagOf(args, "--by");
    const to = flagOf(args, "--to");
    const rel = flagOf(args, "--rel");
    const offset = flagOf(args, "--offset");
    const rotate = flagOf(args, "--rotate");
    const targets = await resolve(args.slice(2));
    if (!targets.length) throw new Error("usage: forma pcb move <ids or refs…> [--board id] (--by dx,dy | --to x,y | --rel <item> --offset dx,dy) [--rotate deg]");
    const params = { targets };
    if (by) params.by = nums(by, 2, "dx,dy");
    if (to) params.to = nums(to, 2, "x,y");
    if (rel) params.relative_to = (await resolve([rel]))[0];
    if (offset) params.offset = nums(offset, 2, "dx,dy");
    if (rotate) params.rotate = Number(rotate);
    const r = await commit("pcb.move", params);
    console.log(`✓ ${r.moved} moved by ${r.by.map((x) => x.toFixed(3)).join(", ")}${r.rotate ? `, turned ${r.rotate}°` : ""}`);
    return false;
  }
  if (sub === "array") {
    const grid = flagOf(args, "--grid");
    const circ = flagOf(args, "--circular");
    const keep = boolOf(args, "--keep-nets");
    const targets = await resolve(args.slice(2));
    if (!targets.length || !(grid || circ)) throw new Error("usage: forma pcb array <ids or refs…> [--board id] (--grid nx,ny,dx,dy | --circular cx,cy,count[,angle]) [--keep-nets]");
    const params = { targets, keep_nets: keep };
    if (grid) {
      const [nx, ny, dx, dy] = nums(grid, 4, "nx,ny,dx,dy");
      params.grid = { nx, ny, dx, dy };
    }
    if (circ) {
      const v = nums(circ, 0, "cx,cy,count[,angle]");
      params.circular = { center: [v[0], v[1]], count: v[2], ...(v[3] != null ? { angle: v[3] } : {}) };
    }
    const r = await commit("pcb.array", params);
    console.log(`✓ ${r.made} copies`);
    return false;
  }
  if (sub === "flip") {
    const targets = await resolve(args.slice(2));
    if (!targets.length) throw new Error("usage: forma pcb flip <ids or refs…> [--board id]");
    const r = await commit("pcb.flip", { targets });
    console.log(`✓ ${r.flipped} flipped to the other side`);
    return false;
  }
  if (sub === "swap") {
    const t = await resolve(args.slice(2, 4));
    if (t.length !== 2) throw new Error("usage: forma pcb swap <a> <b> [--board id]");
    await commit("pcb.swap", { a: t[0], b: t[1] });
    console.log("✓ swapped");
    return false;
  }
  if (sub === "channel") {
    const board = args[2];
    const source = flagOf(args, "--source");
    const step = flagOf(args, "--step");
    const areas = boolOf(args, "--areas");
    const noRouting = boolOf(args, "--no-routing");
    const channels = [];
    for (let to; (to = flagOf(args, "--to")); ) channels.push(to.split(","));
    if (!board || !source || !channels.length) throw new Error("usage: forma pcb channel <board> --source U1,R1,C1 --to U2,R2,C2 [--to U3,R3,C3] [--step dx,dy] [--areas] [--no-routing]");
    const r = await commit("pcb.channel.repeat", { board, source: source.split(","), channels, ...(step ? { step: nums(step, 2, "dx,dy") } : {}), areas, routing: !noRouting });
    for (const c of r.channels) console.log(`✓ channel ${c.channel}: ${c.components} parts placed, ${c.tracks} tracks and ${c.vias} vias copied${c.replaced ? ` (${c.replaced} replaced)` : ""}`);
    return false;
  }
  throw new Error(`unknown: forma pcb ${sub}`);
}
