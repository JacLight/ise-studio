// `forma files ls|rm|mv`: the files of one folder, and never anything outside
// it. The Forma AI's `./forma` sets that folder to its own (FORMA_FILES_ROOT);
// run by hand, it is the current folder. Every path is resolved against the
// folder; one that leads out of it (by `..`, an absolute path, or a symbolic
// link to elsewhere) is refused, as are the files that keep the conversation.

import { lstatSync, mkdirSync, readdirSync, readlinkSync, realpathSync, renameSync, rmSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

/** What keeps the conversation and the tool itself: never removed or moved. */
const KEPT = new Set(["forma", "chat.jsonl", "session.json", "transcript.jsonl", "attachments", "memory"]);
/** Folders whose contents are kept too (the pictures the conversation shows). */
const KEPT_INSIDE = new Set(["attachments"]);

export function filesRoot(env = process.env, cwd = process.cwd()) {
  return realpathSync(env.FORMA_FILES_ROOT || cwd);
}

const within = (root, p) => {
  const rel = relative(root, p);
  return rel === "" || (!rel.startsWith(`..${sep}`) && rel !== ".." && !isAbsolute(rel));
};

const exists = (p) => {
  try {
    lstatSync(p);
    return true;
  } catch {
    return false;
  }
};

/**
 * A path inside the folder: `{ abs, rel, stat }` (stat of the entry itself,
 * not what a link points to; null when it does not exist). Throws, saying
 * why, for anything that is or leads outside.
 */
export function place(root, given, { existing = true } = {}) {
  if (!given) throw new Error("say which file");
  const abs = resolve(root, given);
  if (!within(root, abs)) throw new Error(`${given} is outside your folder (${root}): only files inside it can be listed, removed or moved`);
  // Every folder on the way, with its links followed, is inside too.
  let up = abs === root ? abs : dirname(abs);
  while (!exists(up)) up = dirname(up);
  if (!within(root, realpathSync(up))) throw new Error(`${given} goes through a link to outside your folder`);
  const stat = exists(abs) ? lstatSync(abs) : null;
  if (!stat && existing) throw new Error(`${given}: no such file or folder`);
  if (stat?.isSymbolicLink()) {
    let to;
    try {
      to = realpathSync(abs);
    } catch {
      to = resolve(dirname(abs), readlinkSync(abs));
    }
    if (!within(root, to)) throw new Error(`${given} is a link to outside your folder (${readlinkSync(abs)}): not followed`);
  }
  return { abs, rel: relative(root, abs), stat };
}

function kept(rel) {
  const [first, ...rest] = rel.split(sep);
  if (rel === "") return "that is your folder itself";
  if (!rest.length && KEPT.has(first)) return first === "forma" ? "that is your tool" : "it keeps the conversation";
  if (rest.length && KEPT_INSIDE.has(first)) return "the conversation shows it";
  return null;
}

function size(n) {
  return n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** Files and bytes under a folder (links counted, never followed). */
function tally(abs) {
  let files = 0;
  let bytes = 0;
  const walk = (p) => {
    const s = lstatSync(p);
    if (s.isDirectory()) for (const e of readdirSync(p)) walk(join(p, e));
    else {
      files++;
      bytes += s.size;
    }
  };
  walk(abs);
  return { files, bytes };
}

/** `ls [path]`: one folder's entries (folders first), or one file. */
export function ls(root, given = ".") {
  const { abs, rel, stat } = place(root, given);
  const line = (name, s, p) => {
    if (s.isSymbolicLink()) return `${name} → ${readlinkSync(p)}`;
    if (s.isDirectory()) {
      const t = tally(p);
      return `${name}/  (${t.files} file${t.files === 1 ? "" : "s"}, ${size(t.bytes)})`;
    }
    return `${name}  ${size(s.size)}  ${s.mtime.toISOString().slice(0, 16).replace("T", " ")}`;
  };
  if (!stat.isDirectory()) return [line(rel, stat, abs)];
  const entries = readdirSync(abs).map((n) => ({ n, s: lstatSync(join(abs, n)) }));
  entries.sort((a, b) => Number(b.s.isDirectory()) - Number(a.s.isDirectory()) || a.n.localeCompare(b.n));
  return entries.map(({ n, s }) => line(n, s, join(abs, n)));
}

/** `rm <path…> [-r]`: files, or folders with -r (links removed, never followed). */
export function rm(root, paths, { recursive = false } = {}) {
  if (!paths.length) throw new Error("say which files to remove: forma files rm <path…> [-r]");
  // Every path checked before anything is removed.
  const found = paths.map((p) => {
    const at = place(root, p);
    const why = kept(at.rel);
    if (why) throw new Error(`${p}: ${why}, so it is not removed`);
    if (at.stat.isDirectory() && !recursive) throw new Error(`${p} is a folder: add -r to remove it and everything in it`);
    return at;
  });
  return found.map(({ abs, rel, stat }) => {
    const t = stat.isDirectory() ? tally(abs) : { files: 1, bytes: stat.isSymbolicLink() ? 0 : stat.size };
    rmSync(abs, { recursive: stat.isDirectory(), force: false });
    return stat.isDirectory() ? `removed ${rel}/ (${t.files} file${t.files === 1 ? "" : "s"}, ${size(t.bytes)})` : `removed ${rel}${stat.isSymbolicLink() ? " (a link)" : ` (${size(t.bytes)})`}`;
  });
}

/** `mv <from> <to> [-f]`: renames or moves into a folder; replaces a file only with -f. */
export function mv(root, from, to, { force = false } = {}) {
  if (!from || !to) throw new Error("say what to move and where: forma files mv <from> <to>");
  const src = place(root, from);
  const whySrc = kept(src.rel);
  if (whySrc) throw new Error(`${from}: ${whySrc}, so it is not moved`);
  let dest = place(root, to, { existing: false });
  if (dest.stat?.isDirectory() && !dest.stat.isSymbolicLink()) dest = place(root, join(dest.rel, src.rel.split(sep).pop()), { existing: false });
  const whyDest = kept(dest.rel);
  if (whyDest) throw new Error(`${to}: ${whyDest}, so it is not replaced`);
  if (dest.abs === src.abs) throw new Error(`${from} is already there`);
  if (src.stat.isDirectory() && within(src.abs, dest.abs)) throw new Error(`${from} cannot move inside itself`);
  if (dest.stat) {
    if (dest.stat.isDirectory()) throw new Error(`${dest.rel} is a folder already there`);
    if (!force) throw new Error(`${dest.rel} exists: add -f to replace it`);
  }
  mkdirSync(dirname(dest.abs), { recursive: true });
  renameSync(src.abs, dest.abs);
  return [`moved ${src.rel} → ${dest.rel}`];
}

/** The command line: `files ls [path] | rm <path…> [-r] | mv <from> <to> [-f]`. */
export function files(argv, env = process.env, cwd = process.cwd()) {
  const root = filesRoot(env, cwd);
  const flags = new Set(argv.filter((a) => /^-[a-z]+$/.test(a)).flatMap((a) => [...a.slice(1)]));
  const [sub, ...rest] = argv.filter((a) => !/^-[a-z]+$/.test(a));
  switch (sub ?? "ls") {
    case "ls":
    case "list":
      return ls(root, rest[0] ?? ".");
    case "rm":
    case "remove":
      return rm(root, rest, { recursive: flags.has("r") });
    case "mv":
    case "move":
      return mv(root, rest[0], rest[1], { force: flags.has("f") });
    default:
      throw new Error("forma files ls [path] | rm <path…> [-r] | mv <from> <to> [-f]   (inside your folder only)");
  }
}
