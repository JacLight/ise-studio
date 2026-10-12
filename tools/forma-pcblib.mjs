// `./forma pcb lib|logo|assign|variant|block …`: project and global symbol and
// footprint libraries, footprint wizards, image-to-logo, footprint assignment,
// assembly variants and design blocks, through the same engine API the
// Library view uses. Called from tools/forma.mjs with its engine connection.

import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";

const flagOf = (args, name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args.splice(i, 2)[1] : null;
};
const boolOf = (args, name) => {
  const i = args.indexOf(name);
  if (i >= 0) args.splice(i, 1);
  return i >= 0;
};
const xy = (s) => {
  const v = String(s).split(",").map(Number);
  if (v.length !== 2 || v.some((n) => !Number.isFinite(n))) throw new Error(`“${s}” is not x,y`);
  return v;
};

function show(findings, indent = "  ") {
  for (const f of findings ?? []) console.log(`${indent}${f.severity === "error" ? "✗" : f.severity === "warning" ? "!" : "·"} ${f.code}: ${f.message}`);
}

/** A PNG decoded to grey levels (alpha over white), or a binary/ASCII PGM. */
export function decodeImage(bytes) {
  if (bytes[0] === 0x50 && (bytes[1] === 0x35 || bytes[1] === 0x32)) {
    const text = new TextDecoder("latin1").decode(bytes.subarray(0, 200));
    const m = text.match(/^P([25])\s+(?:#.*\s+)*(\d+)\s+(\d+)\s+(\d+)\s/);
    if (!m) throw new Error("not a PGM image");
    const [w, h, max] = [Number(m[2]), Number(m[3]), Number(m[4])];
    const grey = new Uint8Array(w * h);
    if (m[1] === "5") {
      const body = bytes.subarray(m[0].length);
      for (let i = 0; i < w * h; i++) grey[i] = Math.round((body[i] / max) * 255);
    } else {
      const nums = new TextDecoder("latin1").decode(bytes.subarray(m[0].length)).trim().split(/\s+/).map(Number);
      for (let i = 0; i < w * h; i++) grey[i] = Math.round((nums[i] / max) * 255);
    }
    return { width: w, height: h, grey };
  }
  const sig = [137, 80, 78, 71, 13, 10, 26, 10];
  if (!sig.every((b, i) => bytes[i] === b)) throw new Error("give a PNG or PGM image");
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let pos = 8;
  let w, h, depth, type, interlace;
  let palette = null;
  let trns = null;
  const idat = [];
  while (pos < bytes.length) {
    const len = dv.getUint32(pos);
    const kind = String.fromCharCode(...bytes.subarray(pos + 4, pos + 8));
    const data = bytes.subarray(pos + 8, pos + 8 + len);
    if (kind === "IHDR") {
      w = dv.getUint32(pos + 8);
      h = dv.getUint32(pos + 12);
      depth = data[8];
      type = data[9];
      interlace = data[12];
    } else if (kind === "PLTE") palette = data;
    else if (kind === "tRNS") trns = data;
    else if (kind === "IDAT") idat.push(data);
    else if (kind === "IEND") break;
    pos += 12 + len;
  }
  if (interlace) throw new Error("interlaced PNGs are not read: save it without interlacing");
  const raw = inflateSync(Buffer.concat(idat.map((d) => Buffer.from(d))));
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[type];
  const bpp = Math.max(1, (channels * depth) / 8);
  const stride = Math.ceil((w * channels * depth) / 8);
  const out = new Uint8Array(stride * h);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? out[y * stride + x - bpp] : 0;
      const b = y > 0 ? out[(y - 1) * stride + x] : 0;
      const c = x >= bpp && y > 0 ? out[(y - 1) * stride + x - bpp] : 0;
      let v = line[x];
      if (f === 1) v += a;
      else if (f === 2) v += b;
      else if (f === 3) v += (a + b) >> 1;
      else if (f === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      out[y * stride + x] = v & 255;
    }
  }
  const sample = (row, i) => {
    if (depth === 8) return out[row * stride + i];
    if (depth === 16) return out[row * stride + i * 2];
    const bit = i * depth;
    const byte = out[row * stride + (bit >> 3)];
    const v = (byte >> (8 - depth - (bit & 7))) & ((1 << depth) - 1);
    return type === 3 ? v : Math.round((v / ((1 << depth) - 1)) * 255);
  };
  const grey = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let r, g, b, al = 255;
      if (type === 0 || type === 4) {
        r = g = b = sample(y, x * channels);
        if (type === 4) al = sample(y, x * channels + 1);
      } else if (type === 2 || type === 6) {
        r = sample(y, x * channels);
        g = sample(y, x * channels + 1);
        b = sample(y, x * channels + 2);
        if (type === 6) al = sample(y, x * channels + 3);
      } else {
        const i = sample(y, x);
        r = palette[i * 3];
        g = palette[i * 3 + 1];
        b = palette[i * 3 + 2];
        if (trns && i < trns.length) al = trns[i];
      }
      const lum = 0.299 * r + 0.587 * g + 0.114 * b;
      grey[y * w + x] = Math.round((lum * al + 255 * (255 - al)) / 255);
    }
  }
  return { width: w, height: h, grey };
}

export async function pcbLib(sub, args, { engine, get, http, q }) {
  const commit = async (op, params) => {
    const r = await engine.commit(op, params);
    if (!r.ok) throw new Error(`${op}: ${r.error.code} ${r.error.message}`);
    return r.value.output;
  };
  const post = async (path, body, method = "POST") => {
    const res = await fetch(`${http}${path}`, { method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!res.ok) throw new Error(await res.text());
    return res.json();
  };
  const what = args[2];
  let failed = false;

  if (sub === "lib") {
    if (!what || what === "list") {
      const l = await get(`/pcb/libs?${q()}`);
      console.log(`Project libraries${l.table.length ? ` (table: ${l.table.map((t) => `${t.name}[${t.kind}${t.enabled ? "" : ", off"}${t.pinned ? ", pinned" : ""}]`).join(" ")})` : ""}:`);
      if (!l.project.length) console.log("  (none yet: copy a part in with `forma pcb lib copy`, or make one with a wizard)");
      for (const lib of l.project) {
        console.log(`  ${lib.name}: ${lib.symbols.length} symbols, ${lib.footprints.length} footprints`);
        for (const s of lib.symbols) console.log(`    sym ${s.full}${s.extends ? ` (derived from ${s.extends})` : ""}${s.power ? ` (power: ${s.power.net})` : ""}  ${s.pins} pins, ${s.units} unit(s)${s.demorgan ? ", De Morgan" : ""}${s.error ? `  ✗ ${s.error}` : ""}`);
        for (const f of lib.footprints) console.log(`    fp  ${f.full}  ${f.pads} pads${f.model ? `, 3D offset ${f.model.offset.join(",")}` : ""}`);
      }
      console.log(`Global libraries (${l.global.root}): ${l.global.libraries.map((g) => `${g.name} (${g.symbols.length}+${g.footprints.length})`).join(", ") || "none"}`);
      console.log(`KiCad: footprints ${l.kicad.footprints ? "downloaded" : "not downloaded"}, symbols ${l.kicad.symbols ? "downloaded" : "not downloaded"} (forma pcb library update)`);
      console.log(`Built-in: ${l.builtin.symbols.length} symbols, ${l.builtin.footprints.length} footprints; design blocks: ${l.blocks.map((b) => b.name).join(", ") || "none"}`);
    } else if (what === "symbol" || what === "footprint") {
      const id = args[3];
      if (!id) throw new Error(`usage: forma pcb lib ${what} <Library:Name | built-in name>`);
      const r = await get(`/pcb/libs/${what}/${encodeURIComponent(id)}?${q()}`);
      if (what === "symbol") {
        const d = r.definition;
        console.log(`${d.name} (${r.kind}): ${d.description}\n  prefix ${d.prefix}, value ${d.value}, footprint ${d.footprint || "—"}, ${d.units.length} unit(s)${d.demorgan?.length ? ", De Morgan body" : ""}`);
        for (const p of d.pins) console.log(`  pin ${String(p.number).padEnd(4)} ${String(p.name).padEnd(10)} ${p.etype.padEnd(13)} ${p.side.padEnd(5)} at ${p.at.join(",")} len ${p.length}${p.shape ? ` ${p.shape}` : ""}${p.unit ? ` unit ${p.unit}` : ""}${p.hidden ? " hidden" : ""}${p.pads?.length ? ` → pads ${p.pads.join(",")}` : ""}`);
      } else {
        const f = r.footprint;
        console.log(`${f.name} (${r.kind}): ${f.description}\n  ${f.pads.length} pads, courtyard ${r.courtyard.join(", ")}, height ${r.height} mm${r.entry?.model ? `, 3D offset ${r.entry.model.offset.join(",")} turn ${r.entry.model.rotation}°` : ""}`);
        for (const p of f.pads) console.log(`  pad ${String(p.number || "—").padEnd(4)} ${p.kind} ${p.shape.padEnd(10)} ${p.size.join("×")} at ${p.at.join(",")}${p.rotation ? ` ${p.rotation}°` : ""}${p.drill ? ` drill ${p.drill.join("×")}` : ""}${p.chamfer ? " chamfered" : ""}${p.trapezoid ? " trapezoid" : ""}${p.stack ? " stack" : ""}`);
      }
      show(r.findings);
      failed = r.findings.some((f) => f.severity === "error");
    } else if (what === "check") {
      const l = await get(`/pcb/libs?${q()}`);
      let errors = 0;
      for (const lib of l.project) {
        for (const s of lib.symbols) {
          const r = await get(`/pcb/libs/symbol/${encodeURIComponent(s.full)}?${q()}`);
          const bad = r.findings.filter((f) => f.severity !== "note");
          console.log(`${bad.some((f) => f.severity === "error") ? "✗" : "✓"} symbol ${s.full}`);
          show(bad, "    ");
          errors += r.findings.filter((f) => f.severity === "error").length;
        }
        for (const f of lib.footprints) {
          const r = await get(`/pcb/libs/footprint/${encodeURIComponent(f.full)}?${q()}`);
          const bad = r.findings.filter((x) => x.severity !== "note");
          console.log(`${bad.some((x) => x.severity === "error") ? "✗" : "✓"} footprint ${f.full}`);
          show(bad, "    ");
          errors += r.findings.filter((x) => x.severity === "error").length;
        }
      }
      console.log(errors ? `✗ ${errors} errors` : "✓ every project library part passes its checker");
      failed = errors > 0;
    } else if (what === "copy") {
      // From the built-in library, a project library, or KiCad's (Library:Name with the KiCad library downloaded).
      const isFp = boolOf(args, "--footprint");
      const to = flagOf(args, "--to") ?? "Project";
      const name = flagOf(args, "--name");
      const from = args[3];
      if (!from) throw new Error("usage: forma pcb lib copy <name | Library:Name> [--footprint] [--to Library] [--name New]");
      const libs = await get(`/pcb/libs?${q()}`);
      const inProject = libs.project.some((l) => (isFp ? l.footprints : l.symbols).some((x) => x.full === from));
      const builtin = (isFp ? libs.builtin.footprints : libs.builtin.symbols).includes(from) || (!from.includes(":") && !inProject);
      if (inProject || builtin) {
        const r = await commit("pcb.lib.copy", { kind: isFp ? "footprint" : "symbol", from, library: to, ...(name ? { name } : {}) });
        console.log(`✓ ${from} → ${r.symbol ?? r.footprint}`);
        show(r.findings?.filter((f) => f.severity !== "note"));
      } else if (isFp) {
        const f = await get(`/pcb/kicad/footprint/${encodeURIComponent(from)}`);
        const fp = { ...f.footprint, name: name ?? from.split(":")[1] };
        const r = await commit("pcb.lib.footprint.save", { library: to, footprint: fp, origin: `KiCad ${from}` });
        console.log(`✓ KiCad ${from} → ${r.footprint}`);
        for (const n of f.notes) console.log(`  note: ${n}`);
      } else {
        const s = await get(`/pcb/kicad/symbol/${encodeURIComponent(from)}`);
        const def = { ...s.definition, name: name ?? from.split(":")[1] };
        if (s.footprint) {
          const fr = await commit("pcb.lib.footprint.save", { library: to, footprint: { ...s.footprint, name: s.footprint.name.split(":").pop() }, origin: `KiCad ${s.footprint.name}` });
          def.footprint = fr.footprint;
          console.log(`✓ its footprint KiCad ${s.footprint.name} → ${fr.footprint}`);
        }
        const r = await commit("pcb.lib.symbol.save", { library: to, symbol: def, origin: `KiCad ${from}` });
        console.log(`✓ KiCad ${from} → ${r.symbol}`);
        show(r.findings?.filter((f) => f.severity !== "note"));
      }
    } else if (what === "wizards") {
      const r = await get("/pcb/libs/wizards");
      for (const p of r.presets) console.log(`  ${p.name.padEnd(28)} ${JSON.stringify(p.wizard)}`);
    } else if (what === "wizard") {
      // A preset by name (forma pcb lib wizards), or JSON, with key=value overrides: pins=14 pitch=0.65 name=…
      const save = flagOf(args, "--save");
      const board = flagOf(args, "--board");
      const spec = args[3];
      if (!spec) throw new Error("usage: forma pcb lib wizard <preset | json> [key=value…] [--save Library] [--board <id>]");
      let w;
      if (spec.trim().startsWith("{")) w = JSON.parse(spec);
      else {
        const r = await get("/pcb/libs/wizards");
        const p = r.presets.find((x) => x.name.toLowerCase().startsWith(spec.toLowerCase()));
        if (!p) throw new Error(`no preset “${spec}” (forma pcb lib wizards lists them)`);
        w = p.wizard;
      }
      for (const kv of args.slice(4)) {
        const [k, v] = kv.split("=");
        if (v === undefined) continue;
        w[k] = /^[\[{\d.-]/.test(v) ? JSON.parse(v) : v;
      }
      const r = await post("/pcb/libs/wizard", w);
      const f = r.footprint;
      console.log(`✓ ${f.name}: ${f.description}\n  ${f.pads.length} pads, courtyard ${r.courtyard.join(", ")}`);
      for (const p of f.pads.slice(0, 6)) console.log(`  pad ${p.number.padEnd(4)} ${p.size.join("×")} at ${p.at.join(",")}${p.rotation ? ` ${p.rotation}°` : ""}`);
      if (f.pads.length > 6) console.log(`  … ${f.pads.length - 6} more`);
      show(r.findings.filter((x) => x.severity !== "note"));
      if (save) {
        const s = await commit("pcb.lib.footprint.save", { library: save, footprint: f, origin: `wizard ${w.wizard}` });
        console.log(`✓ saved as ${s.footprint}`);
        if (board) {
          await commit("pcb.footprint.define", { board, footprint: { ...f, name: s.footprint } });
          console.log(`✓ defined on the board as ${s.footprint}`);
        }
      } else if (board) {
        await commit("pcb.footprint.define", { board, footprint: f });
        console.log(`✓ defined on the board as ${f.name}`);
      }
    } else if (what === "place") {
      const at = xy(flagOf(args, "--at") ?? "50,50");
      const ref = flagOf(args, "--ref");
      const style = flagOf(args, "--style");
      const unit = flagOf(args, "--unit");
      const [, , , schematic, symbol] = args;
      if (!schematic || !symbol) throw new Error("usage: forma pcb lib place <schematic> <Library:Name> [--at x,y] [--ref U3] [--unit 2] [--style 2]");
      const r = await commit("pcb.lib.place", { schematic, symbol, at, ...(ref ? { reference: ref } : {}), ...(style ? { body_style: Number(style) } : {}), ...(unit ? { unit: Number(unit) } : {}) });
      console.log(`✓ placed ${r.reference ?? `power port ${symbol}`}`);
    } else if (what === "push") {
      const isFp = boolOf(args, "--footprint");
      const id = args[3];
      if (!id) throw new Error("usage: forma pcb lib push <Library:Name> [--footprint]");
      const r = isFp ? await commit("pcb.lib.footprint.push", { footprint: id }) : await commit("pcb.lib.symbol.push", { symbol: id });
      console.log(isFp ? `✓ redefined on ${r.boards.length} board(s)` : `✓ updated ${r.updated.length} placed part(s): ${r.updated.join(", ")}`);
    } else if (what === "delete") {
      const isFp = boolOf(args, "--footprint");
      const ids = args.slice(3);
      const r = await commit("pcb.lib.delete", isFp ? { footprints: ids } : { symbols: ids });
      console.log(`✓ deleted ${r.deleted}`);
    } else if (what === "svg") {
      const isFp = boolOf(args, "--footprint");
      const id = args[3];
      const res = await fetch(`${http}/pcb/libs/${isFp ? "footprint" : "symbol"}/${encodeURIComponent(id)}/svg?${q()}`);
      if (!res.ok) throw new Error(await res.text());
      process.stdout.write(await res.text());
    } else if (what === "global") {
      const act = args[3] ?? "list";
      const name = args[4];
      if (act === "list") {
        const r = await get("/pcb/libs/global");
        console.log(`Global libraries in ${r.root}:`);
        for (const l of r.libraries) console.log(`  ${l.name}: ${l.symbols.length} symbols, ${l.footprints.length} footprints${l.description ? ` — ${l.description}` : ""}`);
      } else if (act === "export") {
        // A project library saved to this computer for every project.
        const library = flagOf(args, "--library") ?? "Project";
        const r = await post(`/pcb/libs/global/${encodeURIComponent(name)}/from-project?${q({ library })}`, {});
        console.log(`✓ ${library} → global ${r.name}: +${r.added_symbols} symbols, +${r.added_footprints} footprints (${r.path})`);
      } else if (act === "import") {
        const to = flagOf(args, "--to") ?? name;
        const lib = await get(`/pcb/libs/global/${encodeURIComponent(name)}`);
        const r = await commit("pcb.lib.import", { library: to, symbols: lib.symbols, footprints: lib.footprints, origin: `global library ${name}` });
        console.log(`✓ ${r.imported} parts from global ${name} into project library ${to}`);
      } else if (act === "delete") {
        await post(`/pcb/libs/global/${encodeURIComponent(name)}`, {}, "DELETE");
        console.log(`✓ deleted global library ${name}`);
      } else throw new Error("usage: forma pcb lib global list | export <name> [--library L] | import <name> [--to L] | delete <name>");
    } else if (what === "table") {
      // forma pcb lib table Project:project MyGlobal:global:pinned KiCad:kicad:off
      const rows = args.slice(3).map((s) => {
        const [name, kind = "project", ...flags] = s.split(":");
        return { name, kind, enabled: !flags.includes("off"), pinned: flags.includes("pinned") };
      });
      await commit("pcb.lib.table.set", { libraries: rows });
      console.log(`✓ library table: ${rows.map((r) => r.name).join(", ")}`);
    } else {
      throw new Error("usage: forma pcb lib [list] | symbol|footprint <id> | check | copy <from> [--footprint] [--to L] | wizards | wizard <preset> [k=v…] [--save L] [--board id] | place <schematic> <L:Name> [--at x,y] | push <L:Name> [--footprint] | delete <L:Name…> [--footprint] | svg <id> [--footprint] | global … | table …");
    }
  } else if (sub === "logo") {
    // An image made into a silkscreen or copper logo (or a schematic graphic), saved to a library, optionally put on a board.
    const size = Number(flagOf(args, "--size") ?? "10");
    const layer = flagOf(args, "--layer") ?? "silk";
    const threshold = Number(flagOf(args, "--threshold") ?? "128");
    const negative = boolOf(args, "--negative");
    const name = flagOf(args, "--name");
    const save = flagOf(args, "--save") ?? "Project";
    const board = flagOf(args, "--board");
    const at = flagOf(args, "--at");
    const path = args[2];
    if (!path) throw new Error("usage: forma pcb logo <image.png|.pgm> [--size mm] [--layer silk|copper|symbol] [--threshold 128] [--negative] [--name N] [--save Library] [--board <id> --at x,y]");
    const img = decodeImage(new Uint8Array(readFileSync(path)));
    const r = await post("/pcb/libs/bitmap", { width: img.width, height: img.height, pixels: Buffer.from(img.grey).toString("base64"), threshold, negative, size, layer, ...(name ? { name } : {}) });
    if (r.kind === "symbol") {
      const s = await commit("pcb.lib.symbol.save", { library: save, symbol: r.symbol, origin: `image ${path}` });
      console.log(`✓ ${s.symbol}: a ${size} mm schematic graphic`);
    } else {
      const f = r.footprint;
      const s = await commit("pcb.lib.footprint.save", { library: save, footprint: f, origin: `image ${path}` });
      console.log(`✓ ${s.footprint}: ${f.description}`);
      if (board) {
        const c = await commit("pcb.component.create", { board, reference: flagOf(args, "--ref") ?? "LOGO1", footprint: s.footprint, at: xy(at ?? "10,10") });
        console.log(`✓ on the board (${c.id})`);
      }
    }
  } else if (sub === "assign") {
    const schematic = args[2];
    if (!schematic) throw new Error("usage: forma pcb assign <schematic> [R1=R_0603 U2=Project:SOT-583 …] [--fits R1]");
    const pairs = args.slice(3).filter((a) => a.includes("="));
    if (pairs.length) {
      const footprints = Object.fromEntries(pairs.map((p) => p.split("=")));
      const r = await commit("pcb.lib.assign", { schematic, footprints });
      console.log(`✓ ${r.assigned} part(s) assigned`);
      for (const w of r.warnings) console.log(`  ! ${w}`);
    } else {
      const only = flagOf(args, "--fits");
      const r = await get(`/pcb/libs/assign/${schematic}?${q()}`);
      for (const p of r.parts) {
        if (only && p.reference !== only) continue;
        const fits = r.footprints.filter((f) => p.pads.every((n) => f.numbers.includes(n)) && f.pads <= p.pads_all);
        console.log(`${p.reference.padEnd(6)} ${String(p.value).padEnd(16)} ${p.pads.length} pads → ${p.footprint || "(none)"}${fits.length ? `   fits: ${fits.slice(0, 8).map((f) => f.name).join(", ")}${fits.length > 8 ? " …" : ""}` : ""}`);
      }
    }
  } else if (sub === "variant") {
    const schematic = args[2];
    const act = args[3] ?? "list";
    const name = args[4];
    if (!schematic) throw new Error("usage: forma pcb variant <schematic> list | add <name> [--dnp R1,R2] [--fit R5] [--copy Other] | set <name> R2.value=4k7 R7.dnp=true … | delete <name> | bom <name> [--format jlcpcb]");
    const list = await get(`/pcb/variants/${schematic}?${q()}`);
    const find = () => {
      const v = list.variants.find((x) => x.variant.name.toLowerCase() === String(name).toLowerCase());
      if (!v) throw new Error(`no variant ${name} (${list.variants.map((x) => x.variant.name).join(", ") || "none yet"})`);
      return v;
    };
    if (act === "list") {
      for (const v of list.variants) console.log(`  ${v.variant.name.padEnd(16)} ${v.fitted ?? "?"} parts fitted; DNP ${v.variant.dnp?.join(",") || "—"}; fitted ${v.variant.fitted?.join(",") || "—"}; ${Object.keys(v.variant.overrides ?? {}).length} overrides${v.variant.description ? ` — ${v.variant.description}` : ""}`);
      if (!list.variants.length) console.log("  (no variants: the design as drawn is the only build)");
    } else if (act === "add") {
      const split = (s) => (s ? s.split(",").map((x) => x.trim()).filter(Boolean) : []);
      const r = await commit("pcb.variant.create", { schematic, name, dnp: split(flagOf(args, "--dnp")), fitted: split(flagOf(args, "--fit")), ...(flagOf(args, "--copy") ? { copy_from: args[args.indexOf("--copy") + 1] } : {}), description: flagOf(args, "--description") ?? "" });
      console.log(`✓ variant ${name} (${r.id})`);
    } else if (act === "set") {
      const v = find();
      const parts = {};
      for (const kv of args.slice(5)) {
        const m = kv.match(/^([^.=]+)\.(\w+)=(.*)$/);
        if (!m) throw new Error(`“${kv}”: write REF.field=value (value, mpn, lcsc, manufacturer, description, dnp)`);
        parts[m[1]] ??= {};
        parts[m[1]][m[2]] = m[2] === "dnp" ? m[3] === "true" : m[2] === "reset" ? true : m[3];
      }
      await commit("pcb.variant.update", { target: v.id, parts });
      console.log(`✓ ${name}: ${Object.keys(parts).join(", ")} changed`);
    } else if (act === "delete") {
      await commit("pcb.variant.delete", { target: find().id });
      console.log(`✓ deleted ${name}`);
    } else if (act === "bom") {
      find();
      if (!list.board) throw new Error("the schematic drives no board");
      const format = flagOf(args, "--format") ?? "generic";
      const res = await fetch(`${http}/pcb/${list.board}/variant/${encodeURIComponent(name)}/bom.csv?${q({ format })}`);
      if (!res.ok) throw new Error(await res.text());
      process.stdout.write(await res.text());
    } else throw new Error("variant: list | add | set | delete | bom");
  } else if (sub === "block") {
    const act = what ?? "list";
    if (act === "list") {
      const r = await get(`/pcb/blocks?${q()}`);
      for (const b of r.blocks) console.log(`  ${b.block.name.padEnd(20)} ${b.block.symbols.length} symbols, ${b.block.wires.length} wires${b.block.layout ? `, layout of ${b.block.layout.parts.length} parts and ${b.block.layout.copper.length} copper pieces` : ""}${b.block.description ? ` — ${b.block.description}` : ""}`);
      if (!r.blocks.length) console.log("  (no design blocks yet)");
    } else if (act === "save") {
      const sheet = flagOf(args, "--sheet");
      const description = flagOf(args, "--description") ?? "";
      const replace = boolOf(args, "--replace");
      const noLayout = boolOf(args, "--no-layout");
      const [, , , schematic, name, ...refs] = args;
      if (!schematic || !name) throw new Error("usage: forma pcb block save <schematic> <name> R1 C2 U1 … | --sheet <sheet> [--description …] [--replace] [--no-layout]");
      const r = await commit("pcb.block.save", { schematic, name, references: refs, ...(sheet ? { sheet } : {}), description, replace, layout: !noLayout });
      console.log(`✓ design block ${r.name}: ${r.symbols} symbols, ${r.wires} wires, ${r.labels} labels${r.layout ? `, layout ${r.layout.parts} parts + ${r.layout.copper} copper` : ""}`);
    } else if (act === "place") {
      const at = xy(flagOf(args, "--at") ?? "100,100");
      const boardAt = flagOf(args, "--board-at");
      const sheet = flagOf(args, "--sheet");
      const [, , , schematic, block] = args;
      if (!schematic || !block) throw new Error("usage: forma pcb block place <schematic> <block> [--at x,y] [--board-at x,y] [--sheet s]");
      const r = await commit("pcb.block.place", { schematic, block, at, ...(boardAt ? { board_at: xy(boardAt) } : {}), ...(sheet ? { sheet } : {}) });
      console.log(`✓ placed ${r.placed.join(", ")}${r.on_board ? `; ${r.on_board} parts on the board` : ""}`);
    } else if (act === "delete") {
      const r = await get(`/pcb/blocks?${q()}`);
      const b = r.blocks.find((x) => x.block.name === args[3]);
      if (!b) throw new Error(`no block ${args[3]}`);
      await commit("pcb.block.delete", { target: b.id });
      console.log(`✓ deleted ${args[3]}`);
    } else throw new Error("block: list | save | place | delete");
  }
  return failed;
}
