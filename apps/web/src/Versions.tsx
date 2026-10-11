// Named versions of the project: mark it ("Sent to supplier", "v1.2"), look
// at any version read-only, compare it with now, restore it (as one new,
// undoable change), or branch from it into a new project and merge that back.
// Versions live in the project's own journal and reach teammates like any
// change; the history is never rewritten.

import { useCallback, useEffect, useState } from "react";
import { Dialog } from "./Dialog";
import { type EntityView, projectName, projectTitle, projectUrl } from "./engine";
import { Icon } from "./icons";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

export type VersionEntry = { id: string; name: string; note: string; author: string; created: number; revision: number | null; here: boolean };
export type BranchInfo = {
  from: string;
  from_title: string;
  from_exists: boolean;
  version: string | null;
  label: string;
  created: number;
  merged_at: number | null;
  merged: number;
  ahead: number;
  behind: number;
};
type Which = { version?: string; revision?: number };

type Change = { id: string; change: "added" | "removed" | "changed"; name: string | null; kinds: string[]; fields?: { kind: string; path: string; before: unknown; after: unknown }[] };
type Comparison = { label: string; added: number; removed: number; changed: number; changes: Change[] };

/** Opens the project as it was at a version (or a revision of this copy), read-only. */
export function openVersion(which: Which) {
  const url = new URL(location.href);
  url.searchParams.delete("home");
  if (which.version) url.searchParams.set("version", which.version);
  else if (which.revision !== undefined) url.searchParams.set("at", String(which.revision));
  location.assign(url.toString());
}

function openProject(name: string) {
  const url = new URL(location.origin);
  url.searchParams.set("project", name);
  location.assign(url.toString());
}

async function post(path: string, body: unknown) {
  const r = await fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json();
}

const user = () => {
  try {
    return sessionStorage.getItem("forma.user") ?? "local";
  } catch {
    return "local";
  }
};

/** Restore and branch, each asked first, with what happened after. */
export function useVersionActions() {
  const [ask, setAsk] = useState<{ kind: "restore" | "branch"; which: Which; label: string } | null>(null);
  const [message, setMessage] = useState<{ text: string; bad?: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState("");
  useEffect(() => {
    if (!message) return;
    const t = window.setTimeout(() => setMessage(null), 6000);
    return () => window.clearTimeout(t);
  }, [message]);
  const restore = useCallback(async (which: Which, label: string) => setAsk({ kind: "restore", which, label }), []);
  const branch = useCallback(async (which: Which, label: string) => {
    setName("");
    setAsk({ kind: "branch", which, label });
  }, []);
  const go = async () => {
    if (!ask) return;
    setBusy(true);
    try {
      if (ask.kind === "restore") {
        const out = (await post(projectUrl(`/versions/restore?user=${encodeURIComponent(user())}`), ask.which)) as { changed: number };
        setAsk(null);
        if (new URLSearchParams(location.search).has("version") || new URLSearchParams(location.search).has("at")) {
          const url = new URL(location.href);
          url.searchParams.delete("version");
          url.searchParams.delete("at");
          location.assign(url.toString());
        } else setMessage({ text: out.changed ? `Restored “${ask.label}”: ${out.changed} ${out.changed === 1 ? "item" : "items"} set back. Undo brings them back.` : `Already the same as “${ask.label}”.` });
      } else {
        const slug = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 64);
        const out = (await post(projectUrl("/versions/branch"), { ...ask.which, to: slug || undefined })) as { name: string };
        openProject(out.name);
      }
    } catch (e) {
      setMessage({ text: (e as Error).message, bad: true });
    } finally {
      setBusy(false);
    }
  };
  const confirm = ask ? (
    <Dialog title={ask.kind === "restore" ? `Restore “${ask.label}”?` : `Branch from “${ask.label}”`} onClose={() => setAsk(null)}>
      {ask.kind === "restore" ? (
        <p>
          The design goes back to how it was at <b>{ask.label}</b>, as one new change: nothing in the history is lost, Undo takes it back, and teammates get it like any edit. Comments and
          versions stay as they are.
        </p>
      ) : (
        <>
          <p>
            A new project starts from <b>{ask.label}</b>, titled “{projectTitle()} — branch from {ask.label}”. Work there freely; it remembers where it came from, and you can merge its
            changes back into <b>{projectTitle()}</b> later.
          </p>
          <label className="version-field">
            Project name <span>optional</span>
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="made from the version's name" autoFocus />
          </label>
        </>
      )}
      <div className="row-buttons">
        <button type="button" className="primary" disabled={busy} onClick={() => void go()}>
          {ask.kind === "restore" ? (
            <>
              <Icon name="undo" /> Restore
            </>
          ) : (
            <>
              <Icon name="copy" /> Create branch
            </>
          )}
        </button>
        <button type="button" className="secondary" onClick={() => setAsk(null)}>
          Cancel
        </button>
      </div>
    </Dialog>
  ) : null;
  return { restore, branch, message, setMessage, confirm };
}

const show = (v: unknown) => {
  if (v === null || v === undefined) return "—";
  if (typeof v === "number") return `${Number(v.toFixed(4))}`;
  if (typeof v === "string") return v.length > 40 ? `${v.slice(0, 38)}…` : v || "“”";
  if (typeof v === "boolean") return v ? "yes" : "no";
  const s = JSON.stringify(v);
  return s.length > 40 ? `${s.slice(0, 38)}…` : s;
};

/** "shape.program" + "/params/width/value" → "width". */
function fieldName(kind: string, path: string) {
  const p = path.replace(/^\//, "").split("/");
  if (p[0] === "params" && p.length >= 2) return p.length > 2 && p[2] !== "value" ? `${p[1]} ${p.slice(2).join(" ")}` : p[1]!;
  if (p[0] === "source") return "program";
  const k = kind.split(".").pop()!.replace(/_/g, " ");
  return path ? p.join(" › ") : k;
}

const kindName = (c: Change) => {
  const k = c.kinds.find((x) => x !== "core.depends_on") ?? "";
  if (k.startsWith("shape.program")) return "Part";
  if (k.startsWith("solid.box")) return "Box";
  if (k.startsWith("assembly.instance")) return "Instance";
  if (k.startsWith("intent")) return "Requirement";
  if (k.startsWith("solid.material")) return "Material";
  if (k === "core.project") return "Project";
  if (k.startsWith("shape.sketch")) return "Sketch";
  return "Item";
};

/** What changed between a version and now. */
export function Compare(props: { project: string; which: Which; onPick?: (id: string) => void }) {
  const [data, setData] = useState<Comparison | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const key = `${props.which.version ?? ""}:${props.which.revision ?? ""}`;
  useEffect(() => {
    setData(null);
    setError(null);
    const q = props.which.version ? `version=${props.which.version}` : `revision=${props.which.revision}`;
    fetch(projectUrl(`/versions/compare?${q}`))
      .then(async (r) => (r.ok ? setData((await r.json()) as Comparison) : setError(await r.text())))
      .catch((e: Error) => setError(e.message));
  }, [key, props.which.version, props.which.revision]);
  if (error) return <p className="print-note bad">{error}</p>;
  if (!data) return <p className="muted small">Comparing…</p>;
  if (!data.changes.length) return <p className="muted small">No differences in the design since {data.label}.</p>;
  const order = { added: 0, changed: 1, removed: 2 } as const;
  return (
    <div className="compare">
      <p className="compare-summary">
        {data.added > 0 && <span className="chg added">+{data.added} added</span>}
        {data.changed > 0 && <span className="chg changed">{data.changed} changed</span>}
        {data.removed > 0 && <span className="chg removed">−{data.removed} removed</span>}
      </p>
      <ul className="compare-list">
        {[...data.changes]
          .sort((a, b) => order[a.change] - order[b.change])
          .map((c) => {
            const fields = c.fields ?? [];
            const isOpen = open.has(c.id);
            return (
              <li key={c.id} className={`chg-row ${c.change}`}>
                <button
                  type="button"
                  className="chg-head"
                  onClick={() => {
                    if (c.change !== "removed") props.onPick?.(c.id);
                    setOpen((s) => {
                      const n = new Set(s);
                      if (n.has(c.id)) n.delete(c.id);
                      else n.add(c.id);
                      return n;
                    });
                  }}
                  aria-expanded={fields.length ? isOpen : undefined}
                >
                  <span className={`chg-dot ${c.change}`} aria-label={c.change} />
                  <span className="chg-name">
                    {c.name ?? kindName(c)}
                    <small>
                      {kindName(c)} · {c.change}
                      {fields.length ? ` · ${fields.length} ${fields.length === 1 ? "field" : "fields"}` : ""}
                    </small>
                  </span>
                  {fields.length > 0 && <Icon name={isOpen ? "down" : "chevron"} />}
                </button>
                {isOpen && fields.length > 0 && (
                  <table className="chg-fields">
                    <tbody>
                      {fields.slice(0, 30).map((f) => (
                        <tr key={f.kind + f.path}>
                          <th>{fieldName(f.kind, f.path)}</th>
                          <td className="was">{f.path === "/source" ? "edited" : show(f.before)}</td>
                          <td className="arrow">→</td>
                          <td className="now">{f.path === "/source" ? "" : show(f.after)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </li>
            );
          })}
      </ul>
    </div>
  );
}

/** The versions list (in the change history panel): mark, look, compare, restore, branch; and, on a branch, merging back. */
export function VersionsSection(props: { revision: number; me: string; run: Run; entities: ReadonlyMap<string, EntityView>; markAt: number | null; onMarked: () => void }) {
  const [list, setList] = useState<VersionEntry[] | null>(null);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  const [marking, setMarking] = useState(false);
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  const [comparing, setComparing] = useState<string | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [merging, setMerging] = useState(false);
  const actions = useVersionActions();
  const refresh = useCallback(async () => {
    try {
      const r = await fetch(projectUrl("/versions"));
      if (!r.ok) return;
      const out = (await r.json()) as { versions: VersionEntry[]; branch: BranchInfo | null };
      setList(out.versions);
      setBranch(out.branch);
    } catch {
      /* the engine is restarting */
    }
  }, []);
  // Versions are part of the project: refresh whenever it changes.
  const versionKey = [...props.entities.values()].filter((e) => e.components["core.version"]).map((e) => e.id + JSON.stringify(e.components["core.version"])).join();
  useEffect(() => {
    void refresh();
  }, [refresh, versionKey, props.revision]);
  useEffect(() => {
    if (props.markAt !== null) {
      setMarking(true);
      setName("");
      setNote("");
    }
  }, [props.markAt]);

  const mark = async (e: React.FormEvent) => {
    e.preventDefault();
    const params: Record<string, unknown> = { name, note, author: props.me };
    if (props.markAt !== null && props.markAt !== props.revision) params.revision = props.markAt;
    const ok = (await props.run("version.create", params)) !== null;
    if (ok) {
      setMarking(false);
      props.onMarked();
      void refresh();
    }
  };
  const mergeBack = async () => {
    if (!branch) return;
    setMerging(true);
    try {
      const out = (await post(projectUrl("/branch/merge"), {})) as { into: string; changes: number; conflicts: unknown[] };
      const clash = out.conflicts.length ? ` ${out.conflicts.length} ${out.conflicts.length === 1 ? "field was" : "fields were"} changed on both sides: see the Team panel of “${branch.from_title}”.` : "";
      actions.setMessage({ text: out.changes ? `Merged ${out.changes} ${out.changes === 1 ? "change" : "changes"} into “${branch.from_title}”.${clash}` : `Nothing new to merge into “${branch.from_title}”.` });
      void refresh();
    } catch (e) {
      actions.setMessage({ text: (e as Error).message, bad: true });
    } finally {
      setMerging(false);
    }
  };

  return (
    <section className="versions" aria-label="Versions">
      {branch && (
        <div className="branch-card">
          <div className="branch-head">
            <Icon name="copy" />
            <span>
              Branch of <b>{branch.from_title}</b>
              <small>
                from {branch.label}
                {branch.merged_at ? ` · last merged ${new Date(branch.merged_at).toLocaleDateString(undefined, { day: "numeric", month: "short" })}` : ""}
              </small>
            </span>
          </div>
          {branch.from_exists ? (
            <div className="branch-actions">
              <span className="small muted">
                {branch.ahead ? `${branch.ahead} ${branch.ahead === 1 ? "change" : "changes"} not in the original yet` : "Everything here is in the original"}
                {branch.behind ? ` · it has ${branch.behind} of its own since` : ""}
              </span>
              <button type="button" className="primary" disabled={!branch.ahead || merging} onClick={() => void mergeBack()}>
                {merging ? "Merging…" : "Merge back"}
              </button>
              <button type="button" className="link-button" onClick={() => openProject(branch.from)}>
                Open original
              </button>
            </div>
          ) : (
            <p className="small muted">The original is no longer on this computer.</p>
          )}
        </div>
      )}
      <div className="versions-head">
        <h3 className="panel-section">Versions</h3>
        {!marking && (
          <button type="button" className="link-button" onClick={() => setMarking(true)}>
            <Icon name="plus" /> Mark this version
          </button>
        )}
      </div>
      {marking && (
        <form className="version-form" onSubmit={mark}>
          <label className="version-field">
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="v1.2, Sent to supplier…" maxLength={80} autoFocus required />
          </label>
          <label className="version-field">
            Note <span>optional</span>
            <textarea value={note} onChange={(e) => setNote(e.target.value)} rows={2} maxLength={2000} placeholder="What it is, who it went to" />
          </label>
          <p className="small muted">{props.markAt !== null && props.markAt !== props.revision ? `Marks the project as it was at revision ${props.markAt}.` : "Marks the project as it is now."} Teammates see it too.</p>
          <div className="row-buttons">
            <button type="submit" className="primary" disabled={!name.trim()}>
              <Icon name="check" /> Mark version
            </button>
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setMarking(false);
                props.onMarked();
              }}
            >
              Cancel
            </button>
          </div>
        </form>
      )}
      {list === null ? (
        <p className="muted small">Loading…</p>
      ) : list.length === 0 && !marking ? (
        <p className="muted small versions-empty">Name the project at a moment worth keeping (“Sent to supplier”, “v1.2”) to look back at it, compare it with now, restore it or branch from it.</p>
      ) : (
        <ul className="version-list">
          {list.map((v) => (
            <li key={v.id} className="version-row">
              <div className="version-main">
                <span className="version-tag">
                  <Icon name="check" />
                </span>
                <span className="version-text">
                  <b>{v.name}</b>
                  <small>
                    {v.author || "Someone"} · {new Date(v.created).toLocaleDateString(undefined, { day: "numeric", month: "short", year: new Date(v.created).getFullYear() === new Date().getFullYear() ? undefined : "numeric" })}
                    {v.revision !== null ? ` · revision ${v.revision}` : ""}
                  </small>
                  {v.note && <span className="version-note">{v.note}</span>}
                </span>
              </div>
              {v.here ? (
                <div className="version-actions">
                  <button type="button" className="secondary" onClick={() => openVersion({ version: v.id })} title="Look at it, read-only">
                    <Icon name="eye" /> View
                  </button>
                  <button type="button" className={`secondary ${comparing === v.id ? "active" : ""}`} aria-pressed={comparing === v.id} onClick={() => setComparing((c) => (c === v.id ? null : v.id))}>
                    Compare
                  </button>
                  <button type="button" className="ib" aria-label={`More for ${v.name}`} aria-expanded={menu === v.id} onClick={() => setMenu((m) => (m === v.id ? null : v.id))}>
                    <Icon name="dots" />
                  </button>
                  {menu === v.id && (
                    <div className="version-menu" role="menu">
                      <button type="button" role="menuitem" onClick={() => (setMenu(null), void actions.restore({ version: v.id }, v.name))}>
                        <Icon name="undo" /> Restore this version
                      </button>
                      <button type="button" role="menuitem" onClick={() => (setMenu(null), void actions.branch({ version: v.id }, v.name))}>
                        <Icon name="copy" /> Branch from it
                      </button>
                      <button
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          setMenu(null);
                          const n = window.prompt("Rename version", v.name);
                          if (n && n.trim() && n.trim() !== v.name) void props.run("version.update", { target: v.id, name: n });
                        }}
                      >
                        <Icon name="pencil" /> Rename
                      </button>
                      <button type="button" role="menuitem" className="danger" onClick={() => (setMenu(null), void props.run("version.delete", { target: v.id }))}>
                        <Icon name="trash" /> Remove the name
                      </button>
                    </div>
                  )}
                </div>
              ) : (
                <p className="small muted">Some of its changes have not reached this computer yet.</p>
              )}
              {comparing === v.id && (
                <div className="version-compare">
                  <Compare project={projectName} which={{ version: v.id }} />
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
      {actions.message && <p className={`print-note ${actions.message.bad ? "bad" : ""}`}>{actions.message.text}</p>}
      {actions.confirm}
    </section>
  );
}
