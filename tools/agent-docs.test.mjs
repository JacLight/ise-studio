// The agent docs (skills/ise-studio, AGENTS.md, CLAUDE.md) are generated from the code; this fails while they
// are behind it. Fix: node tools/gen-agent-docs.mjs
import { execFileSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

test("the agent docs match the code", () => {
  execFileSync(process.execPath, [fileURLToPath(new URL("./gen-agent-docs.mjs", import.meta.url)), "--check"], { stdio: "inherit" });
});
