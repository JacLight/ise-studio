// Builds the installer: Iṣẹ́ Studio with its own engine inside, for computers
// that have never seen this repository.
//
//   node scripts/bundle.mjs              macOS: .app and .dmg · Linux: .deb
//   node scripts/bundle.mjs --skip-engine   reuse target/release/de-server
//
// It stages, under src-tauri/stage/ (not committed):
//   binaries/de-server-<target>   the engine, run by the app as a sidecar
//   lib/                          the native libraries it needs (OpenCascade and what that uses),
//                                 rewritten to load from inside the app
//   engine/                       the files the engine reads: the web app, the AI's tools and
//                                 guide, the shape preludes, examples, the stock library
//   tauri.bundle.json             the bundle settings, merged over tauri.conf.json
// then runs `tauri build` with them. The same script runs in CI (.github/workflows/desktop.yml).
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauri = join(desktop, "src-tauri");
const repo = resolve(desktop, "../..");
const stage = join(tauri, "stage");
const mac = process.platform === "darwin";
const linux = process.platform === "linux";
if (!mac && !linux) throw new Error("bundle.mjs makes the macOS and Linux installers; run it on one of those.");

const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: "inherit", ...opts });
const out = (cmd, args) => execFileSync(cmd, args, { encoding: "utf8" });
const step = (text) => console.log(`\n▸ ${text}`);

const target = out("rustc", ["-vV"]).match(/^host: (.+)$/m)[1];

// 1. The engine and the web app.
if (!process.argv.includes("--skip-engine")) {
  step("Building the engine");
  run("cargo", ["build", "--release", "-p", "de-server"], { cwd: repo });
}
const engine = join(repo, "target/release/de-server");
if (!existsSync(engine)) throw new Error(`${engine} is missing: build it (cargo build --release -p de-server)`);
if (!existsSync(join(repo, "apps/web/dist/index.html"))) throw new Error("apps/web/dist is missing: build the web app (cd apps/web && npm run build)");

rmSync(stage, { recursive: true, force: true });
mkdirSync(join(stage, "binaries"), { recursive: true });
mkdirSync(join(stage, "lib"), { recursive: true });
const sidecar = join(stage, "binaries", `de-server-${target}`);
copyFileSync(engine, sidecar);
chmodSync(sidecar, 0o755);

// 2. The files the engine reads at run time, at the same places as in the repository.
step("Staging the engine's files");
const files = join(stage, "engine");
const take = (path) => cpSync(join(repo, path), join(files, path), { recursive: true });
take("apps/web/dist");
take("examples");
take("library/stock");
take("tools/ise-ai.md");
for (const f of ["prelude.js", "sheet_metal.js", "bim.js"]) take(`crates/plugins/shape/src/${f}`);
// The `ise` command line the AI uses, with every module it imports.
const modules = new Set();
const follow = (file) => {
  if (modules.has(file)) return;
  modules.add(file);
  for (const m of readFileSync(file, "utf8").matchAll(/(?:from|import)\s*\(?\s*["'](\.[^"']+)["']/g)) follow(resolve(dirname(file), m[1]));
};
follow(join(repo, "tools/ise.mjs"));
for (const m of modules) take(relative(repo, m));

// 3. Native libraries, found from the engine's own references and rewritten to load from the app.
step("Gathering native libraries");
const libs = new Map(); // name in the bundle → real file
let settings;
if (mac) {
  const system = (p) => p.startsWith("/usr/lib/") || p.startsWith("/System/");
  const references = (file) =>
    out("otool", ["-L", file]).split("\n").slice(1).map((l) => l.trim().split(" (")[0]).filter(Boolean);
  const rpaths = (file) =>
    [...out("otool", ["-l", file]).matchAll(/cmd LC_RPATH\n\s+cmdsize \d+\n\s+path (.+?) \(offset/g)].map((m) => m[1].replace("@loader_path", dirname(file)));
  const find = (ref, from) => {
    if (!ref.startsWith("@")) return ref;
    const name = ref.replace(/^@(rpath|loader_path|executable_path)\//, "");
    return [dirname(from), ...rpaths(from)].map((d) => join(d, name)).find(existsSync) ?? null;
  };
  const visit = (file) => {
    for (const ref of references(file)) {
      if (system(ref) || ref === file) continue;
      const name = basename(ref);
      if (libs.has(name)) continue;
      const path = find(ref, file);
      if (!path) throw new Error(`${basename(file)} needs ${ref}, which isn't on this computer`);
      libs.set(name, realpathSync(path));
      visit(realpathSync(path));
    }
  };
  visit(engine);
  // Each library and the engine load the others from Contents/Frameworks.
  const inside = (name) => `@executable_path/../Frameworks/${name}`;
  const rewrite = (file) => {
    for (const ref of references(file)) {
      const name = basename(ref);
      if (libs.has(name) && ref !== inside(name)) run("install_name_tool", ["-change", ref, inside(name), file], { stdio: "ignore" });
    }
  };
  for (const [name, path] of libs) {
    const copy = join(stage, "lib", name);
    copyFileSync(path, copy);
    chmodSync(copy, 0o755);
    run("install_name_tool", ["-id", inside(name), copy], { stdio: "ignore" });
    rewrite(copy);
  }
  rewrite(sidecar);
  for (const p of rpaths(sidecar)) run("install_name_tool", ["-delete_rpath", p, sidecar], { stdio: "ignore" });
  // Rewriting breaks their signatures; Apple silicon refuses unsigned code, so sign them again (ad hoc).
  for (const f of [...readdirSync(join(stage, "lib")).map((n) => join(stage, "lib", n)), sidecar]) run("codesign", ["--force", "--sign", "-", f], { stdio: "ignore" });
  const identity = process.env.APPLE_SIGNING_IDENTITY || "-";
  settings = {
    bundle: {
      targets: ["app", "dmg"],
      externalBin: ["stage/binaries/de-server"],
      resources: { "stage/engine/": "engine/" },
      macOS: {
        frameworks: [...libs.keys()].map((n) => `stage/lib/${n}`),
        // A Developer ID (APPLE_SIGNING_IDENTITY) signs everything as one team, with the hardened
        // runtime notarization needs. Signed ad hoc there is no team, and the hardened runtime
        // would refuse to load the libraries, so it stays off.
        signingIdentity: identity,
        hardenedRuntime: identity !== "-",
      },
    },
  };
} else {
  // OpenCascade's libraries (built from source in CI: Linux distributions ship versions too old).
  // Everything else the engine uses comes from the system, named in the package's dependencies.
  for (const line of out("ldd", [engine]).split("\n")) {
    const m = line.match(/^\s*(\S+) => (\/\S+)/);
    if (m && /^libTK/.test(m[1])) libs.set(m[1], realpathSync(m[2]));
  }
  if (!libs.size) throw new Error("ldd found no OpenCascade libraries in de-server");
  for (const [name, path] of libs) {
    const copy = join(stage, "lib", name);
    copyFileSync(path, copy);
    chmodSync(copy, 0o755);
    run("patchelf", ["--set-rpath", "$ORIGIN", copy]);
  }
  // Installed as /usr/bin/de-server, next to /usr/lib/ise-studio/.
  run("patchelf", ["--set-rpath", "$ORIGIN/../lib/ise-studio", sidecar]);
  settings = {
    bundle: {
      targets: ["deb"],
      externalBin: ["stage/binaries/de-server"],
      resources: { "stage/engine/": "engine/" },
      linux: {
        deb: {
          depends: ["libfreetype6", "libgl1", "libx11-6", "libfontconfig1"],
          files: Object.fromEntries([...libs.keys()].map((n) => [`/usr/lib/ise-studio/${n}`, `stage/lib/${n}`])),
        },
      },
    },
  };
}
console.log(`  ${libs.size} libraries: ${[...libs.keys()].join(" ")}`);
writeFileSync(join(stage, "tauri.bundle.json"), JSON.stringify(settings, null, 2));

// 4. The installer.
step("Building the installer");
run("npx", ["tauri", "build", "--config", "src-tauri/stage/tauri.bundle.json"], {
  cwd: desktop,
  // CI=true: the .dmg skips arranging its window through the Finder (no windows pop up, nothing to wait on).
  env: { ...process.env, CI: "true" },
});
// Copies in dist/ with plain names for downloads: ISE-Studio-<version>-<os>-<arch>.<ext>.
const version = JSON.parse(readFileSync(join(tauri, "tauri.conf.json"), "utf8")).version;
const arch = { aarch64: "arm64", x86_64: "x64" }[target.split("-")[0]] ?? target.split("-")[0];
const dist = join(desktop, "dist");
rmSync(dist, { recursive: true, force: true });
mkdirSync(dist);
const kind = mac ? "dmg" : "deb";
const dir = join(repo, "target/release/bundle", kind);
for (const f of readdirSync(dir).filter((f) => f.endsWith(`.${kind}`))) {
  const name = `ISE-Studio-${version}-${mac ? "macos" : "linux"}-${arch}.${kind}`;
  copyFileSync(join(dir, f), join(dist, name));
  console.log(`  ${join(dist, name)}`);
}
