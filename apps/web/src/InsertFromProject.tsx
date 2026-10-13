// Items from other projects: the Insert from project dialog (pick a project,
// tick items in its tree, linked or copied, where they go), this project's
// links and their state, the badge linked items carry in the tree, and the
// link's section in Properties (update, update automatically, edit locally,
// break link). Linked items are read-only unless edited locally.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type Engine, type EntityView, type ProjectEntry, projectName, projectUrl } from "./engine";
import { Icon } from "./icons";
import "./InsertFromProject.css";

export type LinkChanges = { created: number; changed: number; removed: number; conflicts: number; deleted_here: number };

/** A link as `GET /links` reports it. */
export type LinkStatus = {
  id: string;
  project: string;
  title: string;
  whole: boolean;
  names: string[];
  items: number;
  revision: number;
  source_revision?: number;
  state: "up_to_date" | "update_available" | "missing" | "error" | "checking";
  message?: string;
  changes?: LinkChanges;
  auto_update: boolean;
  /** Its items may be edited here ("Edit locally"); else they are read-only. */
  local_edits: boolean;
  updates_here: boolean;
  inserted: number;
  synced: number;
  placed: string[];
};

type TreeItem = { id: string; name: string; kind: string; detail?: string; brings?: number; children?: TreeItem[] };
type SourceTree = { project: string; title: string; revision: number; entities: number; items: TreeItem[] };
/** The engine counts kinds in the plural; one of each reads singular. */
const ONE: Record<string, string> = { placements: "placement", assemblies: "assembly", boards: "board", "board items": "board item", joints: "joint", "parts and components": "part or component", materials: "material", requirements: "requirement", "parameter sets": "parameter set" };

type Preview = { entities: number; reused: number | null; placed: number | null; renamed: { from: string; to: string }[]; kinds: Record<string, number> };

export type InsertRequest = { from: string; items: string[]; whole: boolean; copy: boolean; auto_update: boolean; place?: "same" | "origin"; at?: [number, number, number] };
export type InsertResult = {
  from: string;
  title: string;
  revision: number;
  show: string[];
  output: { link: string | null; created: number; roots: string[]; placed: string[]; reused: string[]; renamed: { id: string; from: string; to: string }[] };
};

const withUser = (path: string, user: string) => projectUrl(`${path}${path.includes("?") ? "&" : "?"}user=${encodeURIComponent(user)}`);

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, { method, headers: body ? { "content-type": "application/json" } : {}, body: body ? JSON.stringify(body) : undefined });
  const text = await res.text();
  if (!res.ok) throw new Error(text || res.statusText);
  return JSON.parse(text) as T;
}

/** Brings the picked items in (one undoable change, by this person). */
export const insertFromProject = (req: InsertRequest, user: string) => call<InsertResult>("POST", withUser("/link/insert", user), req);

/** Tells what an insert brought, in a sentence. */
export function insertedNote(r: InsertResult, req: InsertRequest) {
  const o = r.output;
  const renamed = o.renamed.length ? ` · renamed ${o.renamed.map((x) => `“${x.from}” → “${x.to}”`).join(", ")} (the names were taken)` : "";
  return `${req.whole ? "All of" : "Inserted from"} “${r.title}”: ${o.created} items, ${o.link ? "linked" : "copied"}${o.reused.length ? `, ${o.reused.length} already here used` : ""}${renamed}`;
}

const linkRecord = (e: EntityView | undefined) =>
  e?.components["core.link"] as { project: string; title: string; whole: boolean; ids: Record<string, string>; placed?: string[]; revision: number; auto_update: boolean; local_edits?: boolean; synced: number } | undefined;
const linkMark = (e: EntityView | undefined) => e?.components["core.linked"] as { link: string; source: string; keep?: Record<string, string[]> } | undefined;

/**
 * Why an item cannot be changed here: it is linked from another project and
 * its link is read-only (the engine refuses edits to it, beyond where it is
 * placed). Null when it is the project's own, or edited locally.
 */
export function linkLock(e: EntityView | undefined, entities: ReadonlyMap<string, EntityView>): string | null {
  const mark = linkMark(e);
  const rec = mark && linkRecord(entities.get(mark.link));
  if (!mark || !rec || rec.local_edits) return null;
  const from = rec.title && rec.title !== rec.project ? `“${rec.title}”` : rec.project;
  return `Linked from ${from}: read-only here, as the source makes it.${mark.keep && Object.keys(mark.keep).length ? " Where it is placed stays yours." : ""} To change it, turn on Edit locally or break the link (Linked, above), or change it in ${rec.project}.`;
}

/**
 * This project's links: their state from the engine (fetched when a link
 * changes here, or the engine says a source changed), and which link each
 * item belongs to (items it brought, and placements made for them).
 */
export function useLinks(engine: Engine, entities: ReadonlyMap<string, EntityView>) {
  const [links, setLinks] = useState<LinkStatus[]>([]);
  const records = useMemo(() => [...entities.values()].filter((e) => linkRecord(e)), [entities]);
  const key = records.map((e) => `${e.id}:${linkRecord(e)!.revision}:${linkRecord(e)!.synced}:${linkRecord(e)!.auto_update}:${!!linkRecord(e)!.local_edits}`).join("|");
  const refresh = useCallback(() => {
    fetch(projectUrl("/links"))
      .then((r) => (r.ok ? (r.json() as Promise<LinkStatus[]>) : []))
      .then(setLinks)
      .catch(() => {});
  }, []);
  useEffect(() => refresh(), [key, refresh]);
  useEffect(() => engine.onLinks(() => refresh()), [engine, refresh]);
  const byItem = useMemo(() => {
    const known = new Map(links.map((l) => [l.id, l]));
    const status = (id: string): LinkStatus | undefined => {
      const live = known.get(id);
      if (live) return live;
      const r = linkRecord(entities.get(id));
      return r ? { id, project: r.project, title: r.title, whole: r.whole, names: [], items: Object.keys(r.ids).length, revision: r.revision, state: "checking", auto_update: r.auto_update, local_edits: !!r.local_edits, updates_here: true, inserted: 0, synced: r.synced, placed: r.placed ?? [] } : undefined;
    };
    const m = new Map<string, LinkStatus>();
    for (const e of entities.values()) {
      const mark = linkMark(e);
      const s = mark && status(mark.link);
      if (s) m.set(e.id, s);
    }
    for (const rec of records) {
      const s = status(rec.id);
      for (const p of linkRecord(rec)!.placed ?? []) if (s) m.set(p, s);
    }
    return m;
  }, [entities, links, records]);
  return { links, byItem, refresh };
}

const stateWord = (l: LinkStatus) =>
  l.state === "up_to_date" ? "Up to date" : l.state === "update_available" ? "Update available" : l.state === "missing" ? "Source missing" : l.state === "checking" ? "Checking…" : "Cannot check";

function changesText(c?: LinkChanges) {
  if (!c) return "";
  const parts = [c.created && `${c.created} new`, c.changed && `${c.changed} changed`, c.removed && `${c.removed} removed`].filter(Boolean);
  return parts.join(", ");
}

/** The small link mark on a linked item's row in the Components tree. */
export function LinkBadge(props: { link: LinkStatus }) {
  const l = props.link;
  const due = l.state === "update_available";
  return (
    <span className={`link-badge ${l.state}`} title={`Linked from “${l.title}” (${l.project}) · ${stateWord(l)}${due ? `: ${changesText(l.changes)}` : ""}${l.auto_update ? " · updates automatically" : ""}`}>
      <Icon name="link" />
      {due && <i aria-hidden />}
    </span>
  );
}

const when = (ms: number) => (ms ? new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "");

/** The link's section in Properties: where it comes from, its state, Update, Update automatically, Break link. */
export function LinkSection(props: { link: LinkStatus; user: string; notify: (text: string) => void; onChanged: () => void }) {
  const l = props.link;
  const [busy, setBusy] = useState<string | null>(null);
  const act = async (what: string, path: string, body: unknown, done: (out: unknown) => string) => {
    setBusy(what);
    try {
      const out = await call<unknown>("POST", withUser(path, props.user), body);
      props.notify(done(out));
    } catch (e) {
      props.notify((e as Error).message);
    } finally {
      setBusy(null);
      props.onChanged();
    }
  };
  const due = l.state === "update_available";
  return (
    <section className={`props-section link-section ${l.state}`}>
      <header>
        <h3>
          <Icon name="link" /> Linked
        </h3>
        <span className={`link-state ${l.state}`} title={due ? `The source changed: ${changesText(l.changes)}${l.changes?.conflicts ? `; ${l.changes.conflicts} edited here too (the source's wins)` : ""}` : stateWord(l)}>
          {l.state === "up_to_date" && <Icon name="check" />}
          {stateWord(l)}
        </span>
      </header>
      <p className="link-from">
        From <strong>{l.title}</strong>{" "}
        <a href={`/?project=${encodeURIComponent(l.project)}`} target="_blank" rel="noreferrer" title={`Open ${l.title} in a new tab`}>
          {l.project} <Icon name="external" />
        </a>
        <small>
          {l.whole ? "The whole project" : l.names.length ? l.names.join(", ") : `${l.items} items`} · {l.items} items · its revision {l.revision}
          {l.source_revision != null && l.source_revision !== l.revision ? ` (now ${l.source_revision})` : ""}
          {l.synced ? ` · updated ${when(l.synced)}` : ""}
        </small>
      </p>
      {due && <p className="link-due">{changesText(l.changes) || "The source changed"}{l.changes?.conflicts ? ` · ${l.changes.conflicts} also edited here: the source's version replaces them (undo brings yours back)` : ""}</p>}
      {l.state === "missing" && <p className="link-due">{l.message ?? "The source project is gone"}: the items stay as they are. Break the link to keep them as your own.</p>}
      <div className="row-buttons link-actions">
        <button
          type="button"
          className={due ? "primary" : "secondary"}
          disabled={!!busy || l.state === "missing"}
          title={due ? `Take ${l.title}'s changes now (one undoable change)` : `Check ${l.title} again and take anything new`}
          onClick={() =>
            act("update", "/links/update", { link: l.id }, (out) => {
              const o = (out as { output?: { up_to_date: boolean; created: string[]; changed: string[]; removed: string[] }; error?: string }[])[0];
              if (o?.error) return o.error;
              return o?.output?.up_to_date ? `Already up to date with ${l.title}` : `Updated from ${l.title}: ${o?.output?.created.length ?? 0} new, ${o?.output?.changed.length ?? 0} changed, ${o?.output?.removed.length ?? 0} removed`;
            })
          }
        >
          <Icon name="refresh" /> {busy === "update" ? "Updating…" : due ? "Update" : "Check for updates"}
        </button>
        <button
          type="button"
          className="secondary"
          disabled={!!busy}
          title="Stop following the source: these items become an ordinary local copy you can change freely (undoable)"
          onClick={() => act("break", "/links/break", { link: l.id }, () => `The items from ${l.title} are a local copy now`)}
        >
          <Icon name="close" /> Break link
        </button>
      </div>
      <label className="link-auto" title={`Take ${l.title}'s changes as soon as they are made, while this engine runs (and when this project opens)`}>
        <input
          type="checkbox"
          checked={l.auto_update}
          disabled={!!busy}
          onChange={(e) => {
            const on = e.target.checked;
            void act("auto", "/links/settings", { link: l.id, auto_update: on }, () => (on ? `Follows ${l.title} automatically` : `Updates from ${l.title} when you ask`));
          }}
        />
        Update automatically
        {l.auto_update && !l.updates_here && <small> (by the teammate's engine that inserted it)</small>}
      </label>
      <label
        className="link-auto"
        title={
          l.local_edits
            ? "Make these items read-only again, as the source makes them (your edits so far stay until the source changes them)"
            : "Allow changing these items here; updates then merge the source's changes with yours (where both changed the same thing, the source's wins)"
        }
      >
        <input
          type="checkbox"
          checked={l.local_edits}
          disabled={!!busy}
          onChange={(e) => {
            const on = e.target.checked;
            void act("local", "/links/settings", { link: l.id, local_edits: on }, () => (on ? `Items from ${l.title} can be edited here; updates merge with your edits` : `Items from ${l.title} are read-only again`));
          }}
        />
        Edit locally
      </label>
      <p className="props-hint">
        {l.local_edits
          ? "Edits here stay through updates unless the source changes the same thing (the source's wins; undo brings yours back). Where it is placed stays yours."
          : "Read-only here, as the source makes it: only where it is placed is yours. Edit locally to change it here, or break the link to make it your own."}
      </p>
    </section>
  );
}

// ---- The dialog ------------------------------------------------------------------

const KIND_ICON: Record<string, Parameters<typeof Icon>[0]["name"]> = {
  assembly: "layers",
  component: "cube",
  part: "cube",
  board: "chip",
  schematic: "chip",
  parameters: "sliders",
  sheet: "table",
  sketch: "pencil",
  definition: "cube",
};

const titleOf = (p: ProjectEntry & { thumbnail?: boolean }) => p.info?.title || p.name.replace(/-/g, " ").replace(/^./, (c) => c.toUpperCase());

/** The items whose name or detail matches, and the way down to them. */
function filterTree(items: TreeItem[], q: string): TreeItem[] {
  if (!q) return items;
  const out: TreeItem[] = [];
  for (const it of items) {
    const kids = filterTree(it.children ?? [], q);
    const hit = it.name.toLowerCase().includes(q) || (it.detail ?? "").toLowerCase().includes(q);
    if (hit || kids.length) out.push({ ...it, children: hit && !kids.length ? it.children : kids });
  }
  return out;
}

function findItem(items: TreeItem[], id: string): TreeItem | undefined {
  for (const it of items) {
    if (it.id === id) return it;
    const k = findItem(it.children ?? [], id);
    if (k) return k;
  }
  return undefined;
}

export function InsertFromProjectDialog(props: {
  user: string;
  notify: (text: string) => void;
  onInserted: (r: InsertResult, req: InsertRequest) => void;
  /** Insert where the next click in the view lands. */
  onPlaceInView: (req: InsertRequest, names: string[]) => void;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [projects, setProjects] = useState<(ProjectEntry & { thumbnail?: boolean })[] | null>(null);
  const [projectQuery, setProjectQuery] = useState("");
  const [from, setFrom] = useState<string | null>(null);
  const [tree, setTree] = useState<SourceTree | null>(null);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const [whole, setWhole] = useState(false);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [mode, setMode] = useState<"linked" | "copy">("linked");
  const [auto, setAuto] = useState(false);
  const [place, setPlace] = useState<"same" | "origin" | "click">("same");
  const [preview, setPreview] = useState<Preview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const d = dialog.current!;
    if (!d.open) d.showModal();
    return () => d.close();
  }, []);

  useEffect(() => {
    fetch("/projects")
      .then((r) => r.json() as Promise<(ProjectEntry & { thumbnail?: boolean })[]>)
      .then((list) => setProjects(list.filter((p) => (projectName ? p.name !== projectName : !p.default))))
      .catch(() => setProjects([]));
  }, []);

  // A project picked: its tree, nothing ticked yet.
  useEffect(() => {
    setTree(null);
    setTreeError(null);
    setPicked([]);
    setWhole(false);
    setQuery("");
    setPreview(null);
    if (!from) return;
    let live = true;
    call<SourceTree>("GET", projectUrl(`/link/tree?from=${encodeURIComponent(from)}`))
      .then((t) => {
        if (!live) return;
        setTree(t);
        // Assemblies start open one level, so sub-assemblies show.
        setOpen(new Set(t.items.filter((i) => i.children?.length).slice(0, 3).map((i) => i.id)));
      })
      .catch((e: Error) => live && setTreeError(e.message));
    return () => {
      live = false;
    };
  }, [from]);

  // What the insert would bring, worked out by the engine as the picks change.
  const request = useMemo<InsertRequest | null>(() => (from && (whole || picked.length) ? { from, items: whole ? [] : picked, whole, copy: mode === "copy", auto_update: mode === "linked" && auto, ...(place === "origin" ? { place: "origin" as const } : {}) } : null), [from, picked, whole, mode, auto, place]);
  useEffect(() => {
    setPreview(null);
    setPreviewError(null);
    if (!request) return;
    let live = true;
    const t = setTimeout(() => {
      call<Preview>("POST", projectUrl("/link/preview"), request)
        .then((p) => live && setPreview(p))
        .catch((e: Error) => live && setPreviewError(e.message));
    }, 180);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [request]);

  const q = query.trim().toLowerCase();
  const shown = useMemo(() => filterTree(tree?.items ?? [], q), [tree, q]);
  const pq = projectQuery.trim().toLowerCase();
  const list = (projects ?? []).filter((p) => !pq || titleOf(p).toLowerCase().includes(pq) || p.name.includes(pq) || (p.info?.tags ?? []).some((t) => t.includes(pq)));
  const source = projects?.find((p) => p.name === from);

  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]));
  const names = picked.map((id) => (tree ? findItem(tree.items, id)?.name : undefined) ?? id);

  const insert = async () => {
    if (!request || busy) return;
    if (place === "click") {
      props.onPlaceInView(request, whole ? [tree?.title ?? from!] : names);
      return;
    }
    setBusy(true);
    try {
      const r = await insertFromProject(request, props.user);
      props.onInserted(r, request);
    } catch (e) {
      props.notify((e as Error).message);
      setBusy(false);
    }
  };

  const row = (it: TreeItem, depth: number, inside: string | null): React.ReactNode => {
    const kids = it.children ?? [];
    const expanded = kids.length > 0 && (open.has(it.id) || !!q);
    const on = whole || picked.includes(it.id);
    const carried = !on && (whole || inside !== null);
    return (
      <div key={`${depth}-${it.id}`}>
        <div className={`xp-row ${on ? "on" : ""} ${carried ? "carried" : ""}`} style={{ paddingLeft: 8 + depth * 18 }}>
          {kids.length ? (
            <button
              type="button"
              className={`tree-twist ${expanded ? "open" : ""}`}
              aria-label={expanded ? `Collapse ${it.name}` : `Expand ${it.name}`}
              title={expanded ? `Collapse ${it.name}` : `Show what is inside ${it.name}`}
              onClick={() =>
                setOpen((o) => {
                  const n = new Set(o);
                  if (n.has(it.id)) n.delete(it.id);
                  else n.add(it.id);
                  return n;
                })
              }
            >
              <Icon name="chevron" />
            </button>
          ) : (
            <span className="tree-twist" />
          )}
          <label
            className="xp-pick"
            title={
              carried
                ? whole
                  ? "Comes with the whole project"
                  : `Comes with ${inside}`
                : depth > 0
                  ? `Insert ${it.name} on its own, placed where it sits in the assembly`
                  : `Insert ${it.name}${it.brings ? ` (with everything it needs: ${it.brings} items)` : ""}`
            }
          >
            <input type="checkbox" checked={on || carried} disabled={carried} onChange={() => toggle(it.id)} />
            <Icon name={KIND_ICON[it.kind] ?? "cube"} />
            <span className="xp-name">{it.name}</span>
            {it.detail && <small>{it.detail}</small>}
          </label>
          {it.brings != null && <span className="xp-brings" title={`Inserting it brings ${it.brings} items: it and everything it needs`}>{it.brings}</span>}
        </div>
        {expanded && kids.map((k) => row(k, depth + 1, on ? it.name : inside))}
      </div>
    );
  };

  const summary = (() => {
    if (!from) return "Pick the project to insert from.";
    if (!request) return "Tick what to insert, or the whole project.";
    if (previewError) return previewError;
    if (!preview) return "Working out what comes along…";
    const kinds = Object.entries(preview.kinds)
      .sort((a, b) => b[1] - a[1])
      .map(([k, n]) => `${n} ${n === 1 ? (ONE[k] ?? k) : k}`)
      .join(", ");
    return `Brings ${preview.entities} items (${kinds})${preview.reused ? ` · uses ${preview.reused} already here` : ""}`;
  })();

  return (
    <dialog ref={dialog} className="xp-dialog" onClose={props.onClose} onCancel={props.onClose} aria-label="Insert from project">
      <header className="xp-head">
        <span className="xp-badge" aria-hidden>
          <Icon name="link" />
        </span>
        <div className="xp-title">
          <h2>Insert from project</h2>
          <p>Bring parts, assemblies, boards or a whole project in: linked to follow its source, or as an independent copy.</p>
        </div>
        <button type="button" className="ib" title="Close without inserting (Esc)" aria-label="Close" onClick={props.onClose}>
          <Icon name="close" />
        </button>
      </header>
      <div className="xp-body">
        <nav className="xp-projects" aria-label="Projects">
          <label className="xp-search" title="Find a project by its title, name or tags">
            <Icon name="search" />
            <input type="search" autoFocus placeholder="Search projects" aria-label="Search projects" value={projectQuery} onChange={(e) => setProjectQuery(e.target.value)} />
          </label>
          <div className="xp-project-list">
            {projects === null && [0, 1, 2, 3].map((i) => <div key={i} className="xp-project skeleton" />)}
            {projects && !list.length && <p className="muted small">{projects.length ? "No project matches." : "No other projects in this folder yet."}</p>}
            {list.map((p) => (
              <button key={p.name} type="button" className={`xp-project ${from === p.name ? "active" : ""}`} title={`Insert from ${titleOf(p)} (${p.name})${p.info?.description ? `: ${p.info.description}` : ""}`} onClick={() => setFrom(p.name)}>
                {p.thumbnail ? <img src={`/projects/${encodeURIComponent(p.name)}/thumbnail`} alt="" loading="lazy" /> : <span className="xp-thumb-none"><Icon name="cube" /></span>}
                <span className="xp-project-text">
                  <strong>{titleOf(p)}</strong>
                  <small>
                    {p.name}
                    {p.modified ? ` · ${new Date(p.modified).toLocaleDateString(undefined, { month: "short", day: "numeric" })}` : ""}
                  </small>
                </span>
              </button>
            ))}
          </div>
        </nav>
        <section className="xp-source" aria-label="Items">
          {!from && (
            <div className="xp-empty">
              <Icon name="link" />
              <p>Pick a project on the left to see what it holds.</p>
            </div>
          )}
          {from && treeError && <div className="xp-empty error">{treeError}</div>}
          {from && !tree && !treeError && (
            <div className="xp-empty">
              <span className="xp-spinner" />
              <p>Reading {source ? titleOf(source) : from}…</p>
            </div>
          )}
          {tree && (
            <>
              <div className="xp-source-head">
                <div>
                  <strong>{tree.title}</strong>
                  <small>
                    {tree.items.length} items at the top · {tree.entities} in all · revision {tree.revision}
                  </small>
                </div>
                <label className="xp-whole" title="Everything in the project: every part, assembly, board, parameter set and requirement">
                  <input type="checkbox" checked={whole} onChange={(e) => setWhole(e.target.checked)} />
                  Whole project
                </label>
              </div>
              <label className="xp-search" title="Show only items whose name matches (and the assemblies they are in)">
                <Icon name="search" />
                <input type="search" placeholder={`Search ${tree.title}`} aria-label="Search items" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Escape" && query && (e.stopPropagation(), setQuery(""))} />
              </label>
              <div className="xp-tree" role="tree">
                {shown.map((it) => row(it, 0, null))}
                {!shown.length && <p className="muted small">{q ? `Nothing called “${query}”.` : "This project holds nothing to insert."}</p>}
              </div>
            </>
          )}
        </section>
      </div>
      <div className="xp-options">
        <div className="xp-option">
          <span className="xp-label">Insert as</span>
          <div className="seg" role="radiogroup" aria-label="Linked or copy">
            <button type="button" role="radio" aria-checked={mode === "linked"} className={mode === "linked" ? "active" : ""} title="Follows its source: when the source changes, this project can take the change (Update), or takes it by itself" onClick={() => setMode("linked")}>
              <Icon name="link" /> Linked
            </button>
            <button type="button" role="radio" aria-checked={mode === "copy"} className={mode === "copy" ? "active" : ""} title="An independent copy: yours to change; the source is never consulted again" onClick={() => setMode("copy")}>
              <Icon name="copy" /> Copy
            </button>
          </div>
          {mode === "linked" && (
            <label className="xp-auto" title="Take the source's changes as soon as they are made (while this engine runs)">
              <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} /> Update automatically
            </label>
          )}
        </div>
        <div className="xp-option">
          <span className="xp-label">Place</span>
          <div className="seg" role="radiogroup" aria-label="Where it goes">
            <button type="button" role="radio" aria-checked={place === "same"} className={place === "same" ? "active" : ""} title="Where it is in the source project" onClick={() => setPlace("same")}>
              Same place
            </button>
            <button type="button" role="radio" aria-checked={place === "origin"} className={place === "origin" ? "active" : ""} title="Its origin at this project's origin" onClick={() => setPlace("origin")}>
              At origin
            </button>
            <button type="button" role="radio" aria-checked={place === "click"} className={place === "click" ? "active" : ""} title="Close this and click in the view where it goes (on a part, or on the ground)" onClick={() => setPlace("click")}>
              <Icon name="cursor" /> Click in view
            </button>
          </div>
        </div>
      </div>
      <footer className="xp-foot">
        <div className={`xp-summary ${previewError ? "error" : ""}`} aria-live="polite">
          {summary}
          {preview?.renamed?.length ? (
            <span className="xp-renames" title="Names already used in this project get the source's title, so both stay distinct">
              {preview.renamed.map((r) => `“${r.from}” becomes “${r.to}”`).join(" · ")}
            </span>
          ) : null}
        </div>
        <button type="button" className="secondary" title="Close without inserting (Esc)" onClick={props.onClose}>
          Cancel
        </button>
        <button type="button" className="primary" disabled={!request || busy || !!previewError} title={!request ? "Tick something to insert first" : place === "click" ? "Close this, then click where it goes" : `Insert ${whole ? "the whole project" : names.join(", ")} ${mode === "linked" ? "linked to its source" : "as a copy"} (one undoable change)`} onClick={insert}>
          {busy ? "Inserting…" : place === "click" ? "Place in view…" : `Insert${whole ? " all" : picked.length > 1 ? ` ${picked.length} items` : ""}${mode === "linked" ? " linked" : " as copy"}`}
        </button>
      </footer>
    </dialog>
  );
}
