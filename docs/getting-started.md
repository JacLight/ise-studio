# Getting started

Iṣẹ́ Studio has two halves that run on your own computer:

- **the engine** (`de-server`, Rust): holds your projects, runs every operation, checks, simulations and exports, and serves the app;
- **the app** (React + three.js): the interface, opened in your browser at **http://127.0.0.1:7420**.

Nothing leaves your machine unless you share a project with a teammate.

## What you need

| | Needed for | Install (macOS) |
|---|---|---|
| **Rust** (stable) | building the engine | `curl https://sh.rustup.rs -sSf \| sh` |
| **OpenCascade** | exact geometry (B-rep, STEP, fillets) | `brew install opencascade` (or set `OCCT_DIR` to its install folder) |
| **Node.js 22** and **pnpm** | building the app and running the `ise` command line | `brew install node pnpm` |
| **Google Chrome** (or Chromium) | the app; photoreal renders and 3D exports run in it | [google.com/chrome](https://www.google.com/chrome/) |

Optional, each switched on when it is installed:

| | Gives you | Install | Setting |
|---|---|---|---|
| **Claude Code** | the built-in assistant, **Iṣẹ́ AI**, in the app's AI panel | [claude.com/claude-code](https://claude.com/claude-code) | `ISE_CLAUDE` (path), `ISE_AI_MODEL` |
| **ngspice** | circuit simulation for boards | `brew install ngspice` | `ISE_NGSPICE` |
| **COLMAP** | photogrammetry (photos → 3D scan) on any machine | `brew install colmap` | `ISE_COLMAP` |
| **Xcode command line tools** | photogrammetry with Apple Object Capture (Apple silicon Macs) | `xcode-select --install` | `ISE_SWIFTC` |

KiCad's symbol and footprint libraries are fetched on demand the first time a board needs them (`ISE_KICAD_LIBRARY` points at a local copy instead).

## Build and run

```sh
git clone https://github.com/JacLight/ise-studio.git
cd ise-studio

# the app (first time, and after changing apps/web)
cd apps/web && pnpm install && pnpm build && cd ../..

# the engine (first time, and after changing crates/)
cargo build --release -p de-server

# start it
./target/release/de-server --project projects/default.jsonl --web apps/web/dist
```

Open **http://127.0.0.1:7420**. The engine listens on your computer only; stop it with Ctrl-C.

## The desktop app (optional)

The same app in its own window, with a Dock icon and menus. It is a view of an engine like the browser is: when an engine is already running at http://127.0.0.1:7420 the window uses it, so the window and your browser tabs show the same projects at the same time.

### Install it

Download from the [releases](https://github.com/JacLight/ise-studio/releases):

- **macOS** (Apple silicon): `ISE-Studio-<version>-macos-arm64.dmg`. Open it and drag Iṣẹ́ Studio to Applications. Until the app is signed with an Apple Developer ID, macOS stops it the first time: open **System Settings → Privacy & Security** and choose **Open Anyway**.
- **Linux** (Ubuntu 22.04 or newer, Debian 12 or newer): `ISE-Studio-<version>-linux-x64.deb`, installed with `sudo apt install ./ISE-Studio-<version>-linux-x64.deb`.

Each carries its own engine (OpenCascade included), so nothing else needs installing. With no engine running, the app starts its own with your projects in **Documents/ISE Studio**, and leaves it running when the window closes so the browser and connected assistants keep working; **Iṣẹ́ Studio → Stop Engine and Quit** stops it. The engine's output goes to `~/Library/Logs/studio.ise.desktop/engine.log` on macOS. Iṣẹ́ AI needs Claude Code and Node.js installed, as in the browser.

- **File → Open in Browser** opens the page in front, at the same project, in your browser (for Claude in Chrome, say).
- Links to other sites open in your browser; the app's own "new tab" links open another window; downloads go to Downloads.
- `ISE_PORT`, `ISE_PROJECT` and `ISE_ENGINE` pick another port, project file or engine program.

### Run it from the source

```sh
cd apps/desktop && npm install
npm run dev          # the window, using this checkout's engine (target/release/de-server) and projects/
```

### Build the installer

```sh
cd apps/desktop && npm install
node scripts/bundle.mjs   # macOS: dist/ISE-Studio-<version>-macos-<arch>.dmg · Linux: dist/…-linux-<arch>.deb
```

It builds the engine, gathers what it reads (the web app, the AI's tools and guide, examples, the stock library) and the native libraries it loads, rewrites those to load from inside the app, and packages it all with Tauri. Build the web app first. On Linux, OpenCascade 7.8 or newer is needed (distributions ship 7.6; [the workflow](../.github/workflows/desktop.yml) builds 7.9 from source) and `patchelf`.

[GitHub Actions](../.github/workflows/desktop.yml) builds both installers, installs each and checks the engine inside answers. Run it from the Actions tab, or push a version tag (`git tag v0.2.0 && git push origin v0.2.0`) to publish them as a release. With the secrets `APPLE_SIGNING_IDENTITY`, `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` set, the Mac app is signed and notarized, and opens without the warning.

## Projects

Each project is one file in `projects/` (`projects/<name>.jsonl`): an append-only journal of every change, so every revision can be shown, compared, restored or branched. Open one by name with `?project=<name>`:

```
http://127.0.0.1:7420/?project=robot-arm
```

The project page (the logo menu, top left) lists them. `projects/` holds your own designs and is never committed to git.

## Try an example

`examples/` holds finished designs as step lists (bracket, gearbox, robot arm, house, ESP32 board, kitchen, Miami tower…). Load one into a new project with the command line:

```sh
node tools/ise.mjs run examples/bracket/steps.json --project bracket
```

then open `http://127.0.0.1:7420/?project=bracket`. Some examples also come ready to print: see [examples/README.md](../examples/README.md#ready-to-print).

## The command line: `ise`

Everything the app does, the command line does too, against the running engine:

```sh
node tools/ise.mjs tree --project bracket          # what the project holds
node tools/ise.mjs check --project bracket         # every requirement and geometry check
node tools/ise.mjs render --studio white --out bracket.png --project bracket
```

Every command is listed in [the CLI reference](../skills/ise-studio/reference/cli.md) (generated from the code). Inside the app, the **Console** (the terminal icon in the left tool rail) takes the same commands.

## Next

- [Use it with AI assistants](ai-assistants.md): the built-in Iṣẹ́ AI, Claude in Chrome, Claude Desktop, Claude Code and Codex.
- [Renders and 3D for the web](rendering-and-web-3d.md).
- [How it is built](architecture.md).
