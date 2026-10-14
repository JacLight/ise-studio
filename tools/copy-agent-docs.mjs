#!/usr/bin/env node
// Publishes the Iṣẹ́ Studio guide (skills/ise-studio, generated from the code by tools/gen-agent-docs.mjs) inside
// the app, at /agent/: each page as Markdown (what `ise.docs()` reads) and as a plain web page an assistant in
// the browser can open and read (Claude in Chrome), with an index. Run by the app's build, after vite.
//
//   node tools/copy-agent-docs.mjs [dist folder]     default apps/web/dist

import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SKILL = join(ROOT, "skills", "ise-studio");
const DIST = process.argv[2] ?? join(ROOT, "apps", "web", "dist");
const OUT = join(DIST, "agent");

if (!existsSync(join(SKILL, "SKILL.md"))) {
  console.error("✗ skills/ise-studio is missing: run node tools/gen-agent-docs.mjs first");
  process.exit(1);
}
const walk = (d) => readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)]));
const files = walk(SKILL).map((p) => relative(SKILL, p)).filter((f) => f.endsWith(".md")).sort((a, b) => (a === "SKILL.md" ? -1 : b === "SKILL.md" ? 1 : a.localeCompare(b)));

const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const page = (title, body) => `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)} · Iṣẹ́ Studio guide</title>
<style>
  body { margin: 0; font: 15px/1.55 -apple-system, system-ui, sans-serif; color: #1d1d1f; background: #fff; }
  main { max-width: 980px; margin: 0 auto; padding: 24px 16px 60px; }
  pre { white-space: pre-wrap; word-wrap: break-word; font: 13.5px/1.55 ui-monospace, SFMono-Regular, Menlo, monospace; }
  nav a { margin-right: 14px; }
  a { color: #5b3fd6; }
  @media (prefers-color-scheme: dark) { body { background: #121214; color: #f2f2f4; } a { color: #a996ff; } }
</style>
</head>
<body><main>
<nav><a href="index.html">Guide home</a><a href="pages.html">All pages</a><a href="/">Back to the app</a></nav>
${body}
</main></body>
</html>
`;
// Markdown links between pages go to their web pages here.
const linked = (md, from) => esc(md).replace(/\[([^\]]+)\]\(([^)]+?)\.md\)/g, (_, text, path) => `[${text}](<a href="${path}.html">${path}.html</a>)`);

rmSync(OUT, { recursive: true, force: true });
for (const f of files) {
  const md = readFileSync(join(SKILL, f), "utf8");
  const dest = join(OUT, f);
  mkdirSync(dirname(dest), { recursive: true });
  writeFileSync(dest, md);
  const title = (md.match(/^# (.+)$/m)?.[1] ?? f).replace(/`/g, "");
  writeFileSync(dest.replace(/\.md$/, ".html"), page(title, `<pre>${linked(md, f)}</pre>`));
}
writeFileSync(join(OUT, "files.json"), JSON.stringify(files));
writeFileSync(join(OUT, "index.html"), readFileSync(join(OUT, "SKILL.html"), "utf8"));
writeFileSync(
  join(OUT, "pages.html"),
  page("All pages", `<h1>Iṣẹ́ Studio guide: all pages</h1>\n<p>Generated from the code. In the app, <code>window.ise.docs("topic")</code> returns any of these as Markdown.</p>\n<ul>\n${files.map((f) => `<li><a href="${f.replace(/\.md$/, ".html")}">${esc(f.replace(/\.md$/, ""))}</a></li>`).join("\n")}\n</ul>`),
);
console.log(`✓ the guide is in the app at /agent/ (${files.length} pages)`);
