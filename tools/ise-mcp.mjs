#!/usr/bin/env node
// Iṣẹ́ Studio as an MCP server (stdio), for Claude Desktop, Claude Code, Codex, Cursor and any other MCP client:
// the same engine the app and the Iṣẹ́ AI use, reached through tools/ise.mjs.
//
//   node tools/ise-mcp.mjs [--url ws://127.0.0.1:7420/ws]
//
// It documents itself from the code when it starts: the CLI tool's description is the usage block of
// tools/ise.mjs, the operations come live from the running engine (their parameters' schemas, written from
// the Rust doc comments), and the docs are the generated skill (skills/ise-studio/, from
// tools/gen-agent-docs.mjs). A new command or operation shows up here without editing this file.

import { execFile } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { cliCommands } from "./gen-agent-docs.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CLI = join(ROOT, "tools", "ise.mjs");
const SKILL = join(ROOT, "skills", "ise-studio");
const urlArg = process.argv.indexOf("--url");
const URL_OPT = urlArg > 0 ? ["--url", process.argv[urlArg + 1]] : process.env.ISE_URL ? ["--url", process.env.ISE_URL] : [];
const VERSION = "0.1.0";

/** Runs the CLI: its output, and whether it failed. */
function ise(args, project, timeoutMs = 15 * 60_000) {
  const full = [CLI, ...args, ...(project ? ["--project", project] : []), ...URL_OPT];
  return new Promise((done) =>
    execFile(process.execPath, full, { cwd: ROOT, maxBuffer: 64 << 20, timeout: timeoutMs }, (err, stdout, stderr) =>
      done({ ok: !err, text: `${stdout}${stderr}`.trim() || (err ? String(err) : "done") }),
    ),
  );
}

const projectParam = { project: { type: "string", description: "The project's name (as in the app's address ?project=<name>; `ise_cli` with args [\"list\"] or the app's project page lists them). Left out: the engine's default project." } };

function tools() {
  const usage = cliCommands()
    .map((c) => `- ${c.usage}${c.notes ? `: ${c.notes}` : ""}`)
    .join("\n");
  return [
    {
      name: "ise_commit",
      description: "Run one engine operation on a project (create or change a part, assembly, board, sketch, drawing, material, requirement…): one undoable revision, shown live in the person's view. `ise_operations` lists the operations and their parameters; ise_docs explains how to model well.",
      inputSchema: { type: "object", properties: { operation: { type: "string", description: "The operation, e.g. shape.create, assembly.instance.create, pcb.board.create." }, parameters: { type: "object", description: "Its parameters, as its schema in ise_operations says." }, ...projectParam }, required: ["operation", "parameters"] },
    },
    {
      name: "ise_operations",
      description: "The engine's operations, live from the running engine, with each one's parameters (JSON schema, written from the code's own documentation). With `search`, only the operations whose name contains it, with their full schemas; without, every name.",
      inputSchema: { type: "object", properties: { search: { type: "string", description: "Part of an operation's name, e.g. \"pcb.route\" or \"joint\"." }, ...projectParam } },
    },
    {
      name: "ise_cli",
      description: `Run any Iṣẹ́ Studio command-line command (node tools/ise.mjs <args>) on a project and get its output. The commands (from the code):\n${usage}`,
      inputSchema: { type: "object", properties: { args: { type: "array", items: { type: "string" }, description: "The command and its arguments, e.g. [\"tree\"], [\"check\"], [\"get\", \"<id>\"], [\"view\", \"isolate\", \"Bracket\"]." }, ...projectParam }, required: ["args"] },
    },
    {
      name: "ise_render",
      description: "A photoreal picture of the model as the person's view shows it (set the camera and isolation first with ise_cli [\"view\", …]). Returns the image. `studio: \"white\"` is a catalogue product shot on a white sweep.",
      inputSchema: { type: "object", properties: { width: { type: "number", description: "Pixels (default 1200)." }, height: { type: "number", description: "Pixels (default 1200)." }, samples: { type: "number", description: "Path-tracing samples (default 128; 512+ for a final image)." }, studio: { type: "string", enum: ["white", "studio"], description: "white: a pure white product shot; studio: a grey studio." }, ...projectParam } },
    },
    {
      name: "ise_docs",
      description: "How to use Iṣẹ́ Studio well, generated from the code and the manual: with no topic, the overview and the list of topics; with a topic (e.g. \"sketches\", \"pcb\", \"assemblies\", \"operations/shape\", \"cli\", \"http\"), that page.",
      inputSchema: { type: "object", properties: { topic: { type: "string" } } },
    },
  ];
}

const docFiles = () => {
  const walk = (d) => (existsSync(d) ? readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)])) : []);
  return walk(SKILL).map((p) => relative(SKILL, p));
};

function docs(topic) {
  const files = docFiles();
  if (!files.length) return "The skill has not been generated: run `node tools/gen-agent-docs.mjs` in the Iṣẹ́ Studio repository.";
  if (!topic) return readFileSync(join(SKILL, "SKILL.md"), "utf8");
  const t = topic.toLowerCase().replace(/[^a-z0-9/]+/g, "-");
  const hit = files.find((f) => f.toLowerCase().replace(/\.md$/, "").endsWith(t)) ?? files.find((f) => f.toLowerCase().includes(t));
  return hit ? readFileSync(join(SKILL, hit), "utf8") : `No page for “${topic}”. Pages:\n${files.map((f) => `- ${f.replace(/\.md$/, "")}`).join("\n")}`;
}

async function call(name, a = {}) {
  switch (name) {
    case "ise_commit": {
      const r = await ise(["commit", a.operation, JSON.stringify(a.parameters ?? {})], a.project);
      return { content: [{ type: "text", text: r.text }], isError: !r.ok || r.text.startsWith("✗") };
    }
    case "ise_operations": {
      const r = await ise(a.search ? ["ops", a.search] : ["ops"], a.project);
      const text = a.search ? r.text : r.text.split("\n").filter((l) => /^[a-z]/.test(l)).join("\n");
      return { content: [{ type: "text", text }], isError: !r.ok };
    }
    case "ise_cli": {
      const r = await ise(a.args ?? [], a.project);
      return { content: [{ type: "text", text: r.text }], isError: !r.ok };
    }
    case "ise_render": {
      const dir = mkdtempSync(join(tmpdir(), "ise-mcp-"));
      const out = join(dir, "render.png");
      const r = await ise(["render", "--width", String(a.width ?? 1200), "--height", String(a.height ?? 1200), "--samples", String(a.samples ?? 128), ...(a.studio ? ["--studio", a.studio] : []), "--headless", "--out", out], a.project);
      try {
        if (!r.ok || !existsSync(out)) return { content: [{ type: "text", text: r.text }], isError: true };
        return { content: [{ type: "image", data: readFileSync(out).toString("base64"), mimeType: "image/png" }, { type: "text", text: r.text }] };
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    }
    case "ise_docs":
      return { content: [{ type: "text", text: docs(a.topic) }] };
    default:
      return { content: [{ type: "text", text: `unknown tool ${name}` }], isError: true };
  }
}

// ---- MCP over stdio (JSON-RPC 2.0, one message a line) --------------------------------------------

const send = (msg) => process.stdout.write(`${JSON.stringify(msg)}\n`);
const reply = (id, result) => send({ jsonrpc: "2.0", id, result });
const fail = (id, code, message) => send({ jsonrpc: "2.0", id, error: { code, message } });

async function handle(msg) {
  const { id, method, params = {} } = msg;
  if (id === undefined) return; // a notification (initialized, cancelled…)
  try {
    switch (method) {
      case "initialize":
        return reply(id, {
          protocolVersion: params.protocolVersion ?? "2025-06-18",
          capabilities: { tools: {}, resources: {} },
          serverInfo: { name: "ise-studio", version: VERSION },
          instructions: "Iṣẹ́ Studio: an engineering app (parts, assemblies, circuit boards, buildings, drawings, renders) whose engine runs on this computer at http://127.0.0.1:7420. Work through these tools, not by clicking the page. Start with ise_docs (no topic) for how to work, and ise_operations for what the engine can do.",
        });
      case "ping":
        return reply(id, {});
      case "tools/list":
        return reply(id, { tools: tools() });
      case "tools/call":
        return reply(id, await call(params.name, params.arguments));
      case "resources/list":
        return reply(id, { resources: docFiles().map((f) => ({ uri: `ise://docs/${f}`, name: f.replace(/\.md$/, ""), mimeType: "text/markdown" })) });
      case "resources/read": {
        const f = String(params.uri ?? "").replace(/^ise:\/\/docs\//, "");
        if (!docFiles().includes(f)) return fail(id, -32602, `no such resource: ${params.uri}`);
        return reply(id, { contents: [{ uri: params.uri, mimeType: "text/markdown", text: readFileSync(join(SKILL, f), "utf8") }] });
      }
      default:
        return fail(id, -32601, `method not found: ${method}`);
    }
  } catch (e) {
    return fail(id, -32603, String(e?.message ?? e));
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) return;
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return fail(null, -32700, "parse error");
  }
  void handle(msg);
});
process.stderr.write(`ise-studio MCP server ${VERSION} (engine ${URL_OPT[1] ?? "ws://127.0.0.1:7420/ws"})\n`);
