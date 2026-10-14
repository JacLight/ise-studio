#!/usr/bin/env node
// Generates the agent documentation for Iṣẹ́ Studio from the code itself, so it can never drift from what the
// code does: the Claude skill (skills/ise-studio/) and the AGENTS.md / CLAUDE.md that Codex and Claude Code read.
//
//   node tools/gen-agent-docs.mjs            write the docs
//   node tools/gen-agent-docs.mjs --check    exit 1 if they are out of date (the agent-docs test runs this)
//
// Sources, all read from the repository:
//   - every engine operation and its parameters: the command registry's JSON schemas, written from the Rust
//     doc comments (target/release/ise-ops, built from crates/server/src/bin/ise-ops.rs when missing)
//   - every CLI command: the usage block at the top of tools/ise.mjs
//   - every HTTP endpoint: the engine's routes (crates/server/src/main.rs) with each handler's own doc comment
//   - how to work: the hand-written manual, tools/ise-ai.md, one guide page per section
// The MCP server (tools/ise-mcp.mjs) reads the same sources when it starts, and the operations live from the
// running engine.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "skills", "ise-studio");
const check = process.argv.includes("--check");
export { build };

// ---- sources -------------------------------------------------------------------------------------

/** Every engine operation, from the code (the registry's schemas). */
export function operations() {
  const bin = join(ROOT, "target", "release", "ise-ops");
  if (!existsSync(bin)) execFileSync("cargo", ["build", "--release", "-p", "de-server", "--bin", "ise-ops"], { cwd: ROOT, stdio: "inherit" });
  return JSON.parse(execFileSync(bin, { maxBuffer: 64 << 20 }).toString()).commands;
}

/** The CLI's commands: the usage block at the top of tools/ise.mjs (a command line, then its indented notes). */
export function cliCommands() {
  const lines = readFileSync(join(ROOT, "tools", "ise.mjs"), "utf8").split("\n");
  const out = [];
  for (const line of lines) {
    if (!line.startsWith("//")) {
      if (out.length) break;
      continue;
    }
    const m = line.match(/^\/\/   node tools\/ise\.mjs (.*)$/);
    if (m) {
      const [usage, ...rest] = m[1].split(/ {3,}/);
      out.push({ usage: usage.trim(), notes: rest.join(" ").trim() });
    } else if (out.length && /^\/\/ {6,}\S/.test(line)) out.at(-1).notes = `${out.at(-1).notes} ${line.replace(/^\/\/\s+/, "")}`.trim();
  }
  return out;
}

/** A Rust function's doc comment, found by name in the server's sources. */
function rustDocs() {
  const docs = new Map();
  const dir = join(ROOT, "crates", "server", "src");
  for (const file of readdirSync(dir).filter((f) => f.endsWith(".rs"))) {
    const mod = file.replace(/\.rs$/, "");
    const lines = readFileSync(join(dir, file), "utf8").split("\n");
    lines.forEach((line, i) => {
      const m = line.match(/^\s*(?:pub(?:\([^)]*\))?\s+)?async fn (\w+)/);
      if (!m) return;
      const doc = [];
      for (let j = i - 1; j >= 0 && /^\s*(\/\/\/|#\[)/.test(lines[j]); j--) if (lines[j].trim().startsWith("///")) doc.unshift(lines[j].trim().replace(/^\/\/\/\s?/, ""));
      const text = doc.join(" ").trim();
      if (text) {
        docs.set(`${mod}::${m[1]}`, text);
        if (mod === "main") docs.set(m[1], text);
      }
    });
  }
  return docs;
}

/** The engine's HTTP endpoints: each route with its methods and its handler's own description. */
export function httpRoutes() {
  const docs = rustDocs();
  const src = readFileSync(join(ROOT, "crates", "server", "src", "main.rs"), "utf8");
  const out = [];
  for (const m of src.matchAll(/\.route\(\s*"([^"]+)"\s*,([^\n]*)/g)) {
    const methods = [...m[2].matchAll(/(?:axum::routing::)?\b(get|post|put|delete|patch)\(\s*([\w:]+)/g)].map((x) => ({ method: x[1].toUpperCase(), handler: x[2] }));
    if (!methods.length) continue;
    out.push({ path: m[1], methods: methods.map((x) => ({ ...x, doc: docs.get(x.handler) ?? "" })) });
  }
  return out;
}

/** The hand-written manual, as sections. */
export function guideSections() {
  const text = readFileSync(join(ROOT, "tools", "ise-ai.md"), "utf8");
  const parts = text.split(/^## /m);
  const intro = parts.shift().trim();
  return {
    intro,
    sections: parts.map((p, i) => {
      const [title, ...body] = p.split("\n");
      return { n: i + 1, title: title.trim(), slug: `${String(i + 1).padStart(2, "0")}-${slug(title)}`, body: body.join("\n").trim() };
    }),
  };
}

const slug = (s) => s.toLowerCase().replace(/`/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 60);

// ---- rendering -----------------------------------------------------------------------------------

const PLUGIN_TITLES = { core: "Core (batches, deletes, links, versions, comments)", solid: "Solids and materials", shape: "Shapes (programs, sketches, configurations, sheets)", assembly: "Assemblies, joints and motion", draft: "Drafting in 3D", intent: "Requirements and checks", sourcing: "Sourcing and costs", pcb: "Printed circuit boards" };

function typeOf(s, defs) {
  if (!s) return "any";
  if (s.$ref) return s.$ref.split("/").pop();
  if (s.anyOf || s.oneOf) {
    const alts = (s.anyOf ?? s.oneOf).filter((a) => a.type !== "null");
    if (alts.every((a) => "const" in a)) return alts.map((a) => JSON.stringify(a.const)).join(" | ");
    return alts.map((a) => typeOf(a, defs)).join(" | ");
  }
  if (s.enum) return s.enum.map((v) => JSON.stringify(v)).join(" | ");
  if ("const" in s) return JSON.stringify(s.const);
  const t = Array.isArray(s.type) ? s.type.filter((x) => x !== "null").join(" | ") : s.type;
  if (t === "array") return `${typeOf(s.items, defs)}[]`;
  return t ?? "object";
}

const cell = (s) => String(s ?? "").replace(/\|/g, "\\|").replace(/\n+/g, " ").trim();

function opMarkdown(op) {
  const s = op.schema ?? {};
  const defs = s.$defs ?? {};
  const desc = (s.description ?? "").replace(/^`[^`]+`:?\s*/, "").trim();
  const props = Object.entries(s.properties ?? {});
  const req = new Set(s.required ?? []);
  let md = `### \`${op.name}\`\n\n${desc ? `${desc}\n\n` : ""}`;
  if (props.length) {
    md += "| Parameter | Type | Required | Description |\n|---|---|---|---|\n";
    for (const [k, v] of props) md += `| \`${k}\` | ${cell(typeOf(v, defs))} | ${req.has(k) ? "yes" : ""} | ${cell(v.description ?? (v.$ref ? defs[v.$ref.split("/").pop()]?.description : ""))} |\n`;
    md += "\n";
  } else md += "No parameters.\n\n";
  // The named types it uses, with their choices or fields.
  const used = Object.entries(defs);
  if (used.length) {
    md += "<details><summary>Types</summary>\n\n";
    for (const [name, d] of used) {
      const alts = d.oneOf ?? d.anyOf;
      if (alts && alts.every((a) => "const" in a)) md += `- **${name}**: ${alts.map((a) => `\`${JSON.stringify(a.const)}\`${a.description ? ` (${cell(a.description)})` : ""}`).join(", ")}\n`;
      else if (d.enum) md += `- **${name}**: ${d.enum.map((v) => `\`${JSON.stringify(v)}\``).join(", ")}${d.description ? `. ${cell(d.description)}` : ""}\n`;
      else if (d.properties) md += `- **${name}**${d.description ? ` (${cell(d.description)})` : ""}: ${Object.entries(d.properties).map(([k, v]) => `\`${k}\`${(d.required ?? []).includes(k) ? "" : "?"}: ${cell(typeOf(v, defs))}`).join(", ")}\n`;
      else md += `- **${name}**: ${cell(typeOf(d, defs))}${d.description ? `. ${cell(d.description)}` : ""}\n`;
    }
    md += "\n</details>\n\n";
  }
  return md;
}

const GENERATED = "<!-- Generated by tools/gen-agent-docs.mjs from the code. Do not edit: change the code or tools/ise-ai.md, then run `node tools/gen-agent-docs.mjs`. -->";

/** What the code does not describe yet: operations without a doc comment, parameters without one. */
export function coverage(ops) {
  const bare = ops.filter((o) => !(o.schema?.description ?? "").replace(/^`[^`]+`:?\s*/, "").trim()).map((o) => o.name);
  let params = 0, described = 0;
  for (const o of ops) for (const [, v] of Object.entries(o.schema?.properties ?? {})) {
    params++;
    if (v.description || (v.$ref && o.schema.$defs?.[v.$ref.split("/").pop()]?.description)) described++;
  }
  return { operations: ops.length, undocumented: bare, params, described };
}

function build() {
  const files = new Map();
  const ops = operations();
  const cli = cliCommands();
  const routes = httpRoutes();
  const guide = guideSections();
  const byPlugin = new Map();
  for (const op of ops) byPlugin.set(op.plugin, [...(byPlugin.get(op.plugin) ?? []), op]);

  // Operations, one page per plugin.
  for (const [plugin, list] of byPlugin) {
    const title = PLUGIN_TITLES[plugin] ?? plugin;
    files.set(`reference/operations/${plugin}.md`, `${GENERATED}\n\n# Operations: ${title}\n\nRun with \`./ise commit <operation> '<json parameters>'\`, the MCP tool \`ise_commit\`, or \`./ise run steps.json\` for several at once (one undo step). ${list.length} operations.\n\n${list.map(opMarkdown).join("")}`);
  }
  // What the code does not describe yet (fill the Rust doc comments, then regenerate).
  const cov = coverage(ops);
  files.set("reference/coverage.md", `${GENERATED}\n\n# Documentation coverage\n\n${cov.operations - cov.undocumented.length} of ${cov.operations} operations describe themselves in the code; ${cov.described} of ${cov.params} parameters do. Undocumented operations (add a \`///\` doc comment on the command's struct):\n\n${cov.undocumented.map((n) => `- \`${n}\``).join("\n") || "- none"}\n`);
  // The CLI.
  files.set("reference/cli.md", `${GENERATED}\n\n# The \`ise\` command line\n\n\`./ise\` is \`node tools/ise.mjs\` in the Iṣẹ́ Studio repository. Every command takes \`--project <name>\` (the project to act on) and \`--url ws://127.0.0.1:7420/ws\` (the engine).\n\n| Command | What it does |\n|---|---|\n${cli.map((c) => `| \`${cell(c.usage)}\` | ${cell(c.notes)} |`).join("\n")}\n`);
  // HTTP.
  files.set("reference/http.md", `${GENERATED}\n\n# The engine's HTTP API\n\nThe engine listens on \`http://127.0.0.1:7420\` (local only). Most endpoints take \`?project=<name>\`. The live state and every operation also stream over the WebSocket at \`/ws\`.\n\n| Method | Path | What it does |\n|---|---|---|\n${routes.flatMap((r) => r.methods.map((m) => `| ${m.method} | \`${cell(r.path)}\` | ${cell(m.doc) || `(\`${m.handler}\`)`} |`)).join("\n")}\n`);
  // The guide, from the manual.
  for (const s of guide.sections) files.set(`reference/guide/${s.slug}.md`, `${GENERATED}\n\n# ${s.title}\n\n${s.body}\n`);
  // The skill itself.
  const opIndex = [...byPlugin].map(([p, l]) => `- [${PLUGIN_TITLES[p] ?? p}](reference/operations/${p}.md): ${l.length} operations (${l.slice(0, 4).map((o) => `\`${o.name}\``).join(", ")}${l.length > 4 ? ", …" : ""})`).join("\n");
  const guideIndex = guide.sections.map((s) => `- [${s.title.replace(/`/g, "")}](reference/guide/${s.slug}.md)`).join("\n");
  files.set(
    "SKILL.md",
    `---
name: ise-studio
description: Design, model, check, simulate, render and export engineering products in Iṣẹ́ Studio, the agent-native CAD app (mechanical parts and assemblies, printed circuit boards, buildings, sheet metal, drawings, photoreal renders and 3D for the web) that runs locally at http://127.0.0.1:7420. Use whenever the user mentions Iṣẹ́ Studio or ISE Studio, works in its browser tab, or asks to create or change a part, board, assembly, drawing or render there.
---
${GENERATED}

# Iṣẹ́ Studio

Iṣẹ́ Studio is an engineering app where an AI builds whole products: parts and assemblies, circuit boards, buildings, drawings, simulations and renders. Its engine runs on the person's computer (\`http://127.0.0.1:7420\`); the app is the browser tab on that address. Everything the app does, the engine does through operations you can call directly, so work through the engine, never by clicking the page.

## How to reach it

- **MCP** (preferred when connected): the \`ise-studio\` server's tools: \`ise_commit\` (run an operation), \`ise_cli\` (any CLI command), \`ise_operations\` (live list of operations with their parameters), \`ise_render\` (a picture of the model), \`ise_docs\` (this guide).
- **Command line**: \`node tools/ise.mjs <command> --project <name>\` from the Iṣẹ́ Studio repository (written \`./ise\` below). See [the CLI reference](reference/cli.md).
- **In the browser** (Claude in Chrome on the app's tab): the page has \`window.ise\`, the same commands as an object: run \`ise.help()\` with your page-script tool, then \`ise.commit(operation, parameters)\`, \`ise.run(steps)\`, \`ise.tree()\`, \`ise.get(name)\`, \`ise.ops(filter)\`, \`ise.check()\`, \`ise.view(action, what)\`, \`ise.undo()\`, and \`ise.docs(topic)\` for this guide. One call changes the design; no clicking. The guide is also readable at \`/agent/\` on the app's address. The app's Console (the terminal icon in the left tool rail) takes the same commands typed.
- **HTTP**: [the engine's endpoints](reference/http.md).

Projects are named (\`?project=<name>\` in the app's address). Every change is one undoable revision, shown live in the person's view.

## How to work

${guide.intro.replace(/^# .*\n+/, "").trim()}

## Reference

Operations (from the code: every parameter with its meaning):
${opIndex}

The guide (how to model well in Iṣẹ́ Studio; read the page for the task at hand):
${guideIndex}
`,
  );
  // Codex and Claude Code read these on their own in the repository.
  const agents = `${GENERATED}

# Working in this repository

This is the source of **Iṣẹ́ Studio**, an agent-native engineering app: a Rust engine (\`crates/\`) and a React + three.js app (\`apps/web\`). People's documentation is in [docs/](docs/README.md): getting started, AI assistants, renders and web 3D, architecture, contributing.

## To use the app (design, model, render)

Read the skill at [skills/ise-studio/SKILL.md](skills/ise-studio/SKILL.md): it says how to reach the running engine (\`node tools/ise.mjs\`, MCP, HTTP) and links every operation, CLI command and endpoint, all generated from this code. With the engine running (\`./target/release/de-server --project projects/default.jsonl --web apps/web/dist\`, then http://127.0.0.1:7420), work through \`node tools/ise.mjs\`, not by clicking the page.

## Connect your assistant (MCP)

\`node tools/ise-mcp.mjs\` is a stdio MCP server (tools: ise_commit, ise_operations, ise_cli, ise_render, ise_docs; the docs as resources). With the engine running:

- **Claude Code**: \`claude mcp add ise-studio -- node $PWD/tools/ise-mcp.mjs\` (the skill in \`.claude/skills/ise-studio\` loads by itself in this repository).
- **Claude Desktop**: Settings → Developer → Edit config, add \`"ise-studio": { "command": "node", "args": ["/path/to/ise-studio/tools/ise-mcp.mjs"] }\` under \`mcpServers\`; for the skill, zip \`skills/ise-studio\` and add it under Settings → Capabilities → Skills.
- **Codex**: in \`~/.codex/config.toml\`: \`[mcp_servers.ise-studio]\` with \`command = "node"\` and \`args = ["/path/to/ise-studio/tools/ise-mcp.mjs"]\`; this file (AGENTS.md) is read by itself.
- **Claude in Chrome** (the browser extension, on the app's tab): ask it to read \`skills/ise-studio/SKILL.md\` from the repository, or connect the MCP server above in Claude Desktop instead.

## To change the code

- Engine: \`cargo build --release -p de-server\`; tests \`cargo test --release --workspace\`.
- App: \`cd apps/web && npm install && npm run build\`.
- After changing an operation, a CLI command, an endpoint or \`tools/ise-ai.md\`, run \`node tools/gen-agent-docs.mjs\` so the agent docs follow (\`node --test tools/agent-docs.test.mjs\` fails while they are stale).
- \`projects/\` holds people's designs and is never committed.
`;
  files.set("@AGENTS.md", agents);
  files.set("@CLAUDE.md", agents);
  return files;
}

// ---- write or check ------------------------------------------------------------------------------

const where = (name) => (name.startsWith("@") ? join(ROOT, name.slice(1)) : join(OUT, name));
if (process.argv[1] === fileURLToPath(import.meta.url)) main();

function main() {
const files = build();
let stale = [];
for (const [name, text] of files) {
  const path = where(name);
  const now = existsSync(path) ? readFileSync(path, "utf8") : null;
  if (now !== text) stale.push(name.replace(/^@/, ""));
}
// Generated pages that no longer have a source (a removed plugin or manual section).
const listed = new Set([...files.keys()].filter((n) => !n.startsWith("@")).map((n) => join(OUT, n)));
const walk = (d) => (existsSync(d) ? readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)])) : []);
const orphans = walk(OUT).filter((p) => !listed.has(p));

if (check) {
  if (stale.length || orphans.length) {
    console.error(`✗ the agent docs are out of date (${[...stale, ...orphans.map((p) => p.slice(ROOT.length + 1))].slice(0, 8).join(", ")}${stale.length + orphans.length > 8 ? ", …" : ""}): run node tools/gen-agent-docs.mjs`);
    process.exit(1);
  }
  console.log(`✓ the agent docs match the code (${files.size} files)`);
} else {
  for (const p of orphans) rmSync(p);
  for (const [name, text] of files) {
    const path = where(name);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, text);
  }
  console.log(`✓ wrote ${files.size} files (${stale.length} changed${orphans.length ? `, ${orphans.length} removed` : ""}): skills/ise-studio/, AGENTS.md, CLAUDE.md`);
}
}
