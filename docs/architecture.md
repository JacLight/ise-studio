# How Iṣẹ́ Studio is built

```
 browser (React + three.js)  ──WebSocket /ws + HTTP──▶  engine (Rust, de-server)  ──▶  projects/<name>.jsonl
   app, live view, renders,                                commands, checks, exact          one append-only journal
   AI panel, console, window.ise                           geometry, exports, AI host       per project
            ▲                                                       ▲
            └── ./ise (tools/ise.mjs), MCP (tools/ise-mcp.mjs) ─────┘
```

## Principles

- **One engine, one source of truth.** The interface, the command line, the built-in Iṣẹ́ AI, browser assistants (`window.ise`) and MCP clients all change a project through the same operations. Nothing is done "only in the UI".
- **The AI computes geometry.** Shapes are small programs the AI writes (and people can read), evaluated by the engine on exact (OpenCascade) or mesh (Manifold) kernels; there is no fixed feature list to fall back on.
- **Every change is a revision.** Operations are recorded in an append-only journal: each is one undo step, shown live to everyone in the project, and any revision can be viewed, compared, restored or branched.
- **Local first, peer to peer.** The engine runs on your computer; teammates sync projects directly (version vectors), with no central server.
- **The code documents itself.** Every operation's parameters are described in the code; the agent docs are generated from it (see [AI assistants](ai-assistants.md)).

## The engine: `crates/`

| Crate | What it holds |
|---|---|
| `core` | The project: a shared space every plugin works in, the command registry (each operation with its JSON schema), the journal and revisions, per-person undo and redo, checks, sync between peers. |
| `occt` | A C++ layer over OpenCascade (LGPL, linked dynamically): exact B-rep solids, fillets and chamfers, sweeps and lofts, STEP and IGES, hidden-line drawings. |
| `plugins/solid` | Solids and materials (densities, engineering properties, costs). |
| `plugins/shape` | Shape programs (geometry written as code), sketches with constraints, configurations, parameters and spreadsheets, sheet metal, drawings, text, threads, reliefs, fabrication and CAM. |
| `plugins/assembly` | Components and assemblies, joints and couplings, motion and clashes, fits, robot kinematics. |
| `plugins/draft` | Drafting in model space: 2D and 3D construction geometry, layers, working planes, arrays. |
| `plugins/intent` | Requirements: live checks on any measure, mass, clearance, photo match or rule. |
| `plugins/pcb` | Printed circuit boards: schematics, libraries (KiCad's), layout, autorouter, constraints, DRC, SPICE, Gerber and every fabrication output, imports from other EDA tools. |
| `plugins/bim` | Buildings: levels, rooms, schedules, plans, IFC. |
| `plugins/scan` | Meshes and point clouds: repair, fitting, inspection, photogrammetry. |
| `plugins/sourcing` | Bills of materials, suppliers, quotes and costs. |
| `server` | `de-server`: the HTTP and WebSocket API, serving the app, hosting Iṣẹ́ AI (Claude Code), renders and web exports (made in the browser), photogrammetry, simulation, sharing between peers. `ise-ops` dumps the operation registry for the docs. |
| `bench` | Performance benchmarks. |

## The app: `apps/web`

React 19 and three.js, built with Vite. The live view draws thousands of parts with GPU instancing; photoreal renders path-trace a copy of the view (three-gpu-pathtracer) in a product studio; web exports write glTF with baked ambient occlusion. `agentApi.ts` is `ise`, the command object the console and assistants share.

## The desktop app: `apps/desktop`

An optional window around the same app (Tauri, the system's web view). It opens the engine at 127.0.0.1:7420, starting it if nothing answers there, so the window and the browser are two views of one engine. `src-tauri/src/main.rs` is all of it: finding or starting the engine, links, new windows, downloads, the menus.

## Tools: `tools/`

| File | What it is |
|---|---|
| `ise.mjs` | The `ise` command line: every operation and command against a running engine. |
| `ise-mcp.mjs` | The MCP server for Claude Desktop, Claude Code, Codex and others. |
| `ise-ai.md` | The manual: how to model well in Iṣẹ́ Studio. The source of the skill's guide pages. |
| `gen-agent-docs.mjs` | Generates the skill, `AGENTS.md` and `CLAUDE.md` from the code. |
| `copy-agent-docs.mjs` | Publishes the guide inside the app at `/agent/` (run by the app's build). |
| `studio-hdr.py` | The studio lighting file for web 3D viewers. |
| `photogrammetry/` | Apple Object Capture helper (Swift). |

## Data

- `projects/<name>.jsonl`: a project's journal; pictures, scans and other assets beside it. Never committed.
- `library/`: stock parts (fasteners, bearings, steel sections…) with suppliers.
- `examples/`: finished designs as step lists you can load with `ise run`.
- `skills/ise-studio/`: the generated skill.
