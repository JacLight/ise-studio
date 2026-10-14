# Contributing

## Build and test

```sh
# engine
cargo build --release -p de-server
cargo test --release --workspace --no-fail-fast

# app
cd apps/web && pnpm install && pnpm build        # type-checks, builds, and publishes the guide at /agent/

# command line and agent docs
node --test tools/*.test.mjs
```

Run the engine with `./target/release/de-server --project projects/default.jsonl --web apps/web/dist` and check changes in the browser at http://127.0.0.1:7420.

## Keep the agent docs in step

The skill (`skills/ise-studio/`), `AGENTS.md` and `CLAUDE.md` are generated from the code. After adding or changing an operation, its parameters, a command in `tools/ise.mjs`, an HTTP endpoint, or the manual (`tools/ise-ai.md`):

```sh
node tools/gen-agent-docs.mjs
```

and commit what it writes. `node --test tools/agent-docs.test.mjs` fails while they are stale. Describe every new operation and parameter with a `///` doc comment on its Rust struct and fields: that text is what every AI assistant reads. [`skills/ise-studio/reference/coverage.md`](../skills/ise-studio/reference/coverage.md) lists what is not described yet.

## Conventions

- **Through the engine.** New capabilities are engine operations (with schemas) first; the interface, the CLI and assistants then all get them.
- **No GPL code copied in.** OpenCascade is LGPL and linked dynamically; other dependencies are permissive.
- **Names**: the product is Iṣẹ́ Studio on screen; files the engine writes (Gerber, STEP, STL…) say `ISE Studio` in plain ASCII; code, CLI and settings use `ise` (`./ise`, `ISE_*`).
- **Never commit `projects/`**: it holds people's designs.
- `SPEC.md`, `DESIGN.md`, `BUILD_PLAN.md` and `README.md` are the product owner's documents: propose changes, don't rewrite them.
