// node --test tools/forma-files.test.mjs: `forma files` stays inside its folder.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { files } from "./forma-files.mjs";

/** A session folder like the Forma AI's, and a file outside it. */
function setup() {
  const top = realpathSync(mkdtempSync(join(tmpdir(), "forma-files-")));
  const root = join(top, "demo.ai");
  mkdirSync(join(root, "programs/old"), { recursive: true });
  mkdirSync(join(root, "attachments"), { recursive: true });
  mkdirSync(join(root, "memory"), { recursive: true });
  writeFileSync(join(root, "programs/bracket.js"), "return box(10, 10, 10);");
  writeFileSync(join(root, "programs/old/a.js"), "a");
  writeFileSync(join(root, "programs/old/b.js"), "bb");
  writeFileSync(join(root, "steps.json"), "[]");
  writeFileSync(join(root, "chat.jsonl"), "");
  writeFileSync(join(root, "forma"), "#!/bin/sh");
  writeFileSync(join(root, "attachments/photo.png"), "png");
  writeFileSync(join(root, "memory/note.md"), "note");
  writeFileSync(join(top, "secret.txt"), "outside");
  mkdirSync(join(top, "elsewhere"));
  writeFileSync(join(top, "elsewhere/x.txt"), "x");
  symlinkSync(join(top, "secret.txt"), join(root, "out-link"));
  symlinkSync(join(top, "elsewhere"), join(root, "out-dir"));
  symlinkSync(join(root, "steps.json"), join(root, "in-link"));
  return { top, root, env: { FORMA_FILES_ROOT: root } };
}

const run = (argv, env, cwd = "/") => files(argv, env, cwd);

test("lists, removes and moves inside the folder", () => {
  const { root, env } = setup();
  const list = run(["ls"], env).join("\n");
  assert.match(list, /programs\/  \(3 files/);
  assert.match(list, /out-link → /);
  assert.match(run(["ls", "programs/old"], env).join("\n"), /a\.js  1 B/);

  assert.deepEqual(run(["rm", "programs/old/a.js"], env), ["removed programs/old/a.js (1 B)"]);
  assert.ok(!existsSync(join(root, "programs/old/a.js")));
  assert.throws(() => run(["rm", "programs/old"], env), /is a folder: add -r/);
  assert.match(run(["rm", "-r", "programs/old"], env)[0], /removed programs\/old\/ \(1 file, 2 B\)/);

  assert.deepEqual(run(["mv", "programs/bracket.js", "programs/arm.js"], env), ["moved programs/bracket.js → programs/arm.js"]);
  mkdirSync(join(root, "done"));
  assert.deepEqual(run(["mv", "steps.json", "done"], env), ["moved steps.json → done/steps.json"]);
  writeFileSync(join(root, "steps.json"), "[1]");
  assert.throws(() => run(["mv", "steps.json", "done/steps.json"], env), /exists: add -f/);
  run(["mv", "-f", "steps.json", "done/steps.json"], env);
  // A link inside the folder to a file inside it is removed as a link.
  assert.deepEqual(run(["rm", "in-link"], env), ["removed in-link (a link)"]);
  // Memory notes can go; with no FORMA_FILES_ROOT the folder is the current one.
  assert.match(run(["rm", "memory/note.md"], {}, root)[0], /removed memory\/note\.md/);
});

test("refuses anything outside the folder, links out, and what keeps the conversation", () => {
  const { top, root, env } = setup();
  for (const p of ["../secret.txt", join(top, "secret.txt"), "programs/../../secret.txt", "/etc/hosts"]) {
    assert.throws(() => run(["rm", p], env), /outside your folder/, p);
    assert.throws(() => run(["ls", p], env), /outside your folder/, p);
  }
  assert.throws(() => run(["rm", "out-link"], env), /link to outside your folder/);
  assert.throws(() => run(["ls", "out-dir"], env), /link to outside your folder/);
  assert.throws(() => run(["rm", "out-dir/x.txt"], env), /through a link to outside/);
  assert.throws(() => run(["mv", "steps.json", "out-dir/steps.json"], env), /through a link to outside/);
  assert.throws(() => run(["mv", "steps.json", "../steps.json"], env), /outside your folder/);
  assert.throws(() => run(["rm", "-r", "."], env), /your folder itself/);
  for (const p of ["chat.jsonl", "forma", "attachments", "attachments/photo.png", "memory"]) assert.throws(() => run(["rm", "-r", p], env), /is not removed/, p);
  assert.throws(() => run(["mv", "steps.json", "chat.jsonl", "-f"], env), /is not replaced/);
  // Nothing outside was touched; a refused list removes nothing at all.
  assert.throws(() => run(["rm", "steps.json", "../secret.txt"], env), /outside/);
  for (const p of [join(top, "secret.txt"), join(top, "elsewhere/x.txt"), join(root, "steps.json"), join(root, "chat.jsonl")]) assert.ok(existsSync(p), p);
});

test("the CLI's files command uses the folder ./forma sets", () => {
  const { root } = setup();
  const cli = join(dirname(fileURLToPath(import.meta.url)), "forma.mjs");
  const out = execFileSync("node", [cli, "files", "rm", "steps.json"], { env: { ...process.env, FORMA_FILES_ROOT: root }, cwd: "/", encoding: "utf8" });
  assert.match(out, /✓ removed steps\.json/);
  let failed = null;
  try {
    execFileSync("node", [cli, "files", "rm", "../secret.txt"], { env: { ...process.env, FORMA_FILES_ROOT: root }, cwd: "/", encoding: "utf8", stdio: "pipe" });
  } catch (e) {
    failed = e;
  }
  assert.equal(failed?.status, 1);
  assert.match(failed.stderr, /✗ .*outside your folder/);
});
