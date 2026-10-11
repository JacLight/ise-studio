// Every project on this computer, as cards with a picture of each: the ones
// you started and the ones teammates shared with you, searchable by title,
// description and tags, sortable, grouped. Opening one loads it in its own
// scene. A project's name never changes; its title, description and tags are
// kept in the project (so they sync with teammates) and edited here.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { prettyName, projectName } from "./engine";
import { Icon } from "./icons";

type Project = {
  name: string;
  modified: number | null;
  created: number | null;
  revisions: number;
  default: boolean;
  thumbnail: boolean;
  team: { members: { name: string; role: string }[]; joined: boolean; by: string } | null;
  info?: { title: string | null; description: string; tags: string[] };
};
type Trashed = { entry: string; name: string; title?: string; deleted: number; revisions: number; taken: boolean };

type Filter = "all" | "mine" | "shared" | "team";
type Sort = "edited" | "name" | "created";

const ago = (ms: number | null) => {
  if (!ms) return "";
  const s = (Date.now() - ms) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  if (s < 86400 * 7) return `${Math.floor(s / 86400)} d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: new Date(ms).getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
};
const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join("") || "?";
const titleOf = (p: Project) => p.info?.title || prettyName(p.default ? null : p.name);

/** Opens a project; a new one can carry the title it should start with. */
export function openProject(name: string, isDefault: boolean, title?: string) {
  const url = new URL(location.href);
  url.searchParams.delete("home");
  if (isDefault) url.searchParams.delete("project");
  else url.searchParams.set("project", name);
  if (title) url.searchParams.set("title", title);
  location.assign(url.toString());
}

const post = async (path: string, body?: unknown) => {
  const r = await fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
};

export function ProjectsHome(props: { onClose: () => void }) {
  const [list, setList] = useState<Project[] | null>(null);
  const [trash, setTrash] = useState<Trashed[]>([]);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [tag, setTag] = useState<string | null>(null);
  const [sort, setSort] = useState<Sort>("edited");
  const [naming, setNaming] = useState<string | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [editing, setEditing] = useState<Project | null>(null);
  const [deleting, setDeleting] = useState<Project | null>(null);
  const [showTrash, setShowTrash] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    fetch("/projects")
      .then((r) => r.json())
      .then(setList)
      .catch(() => setList([]));
    fetch("/projects/trash")
      .then((r) => r.json())
      .then(setTrash)
      .catch(() => setTrash([]));
  }, []);
  useEffect(load, [load]);
  const busy = naming !== null || editing !== null || deleting !== null;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      if (menu) setMenu(null);
      else if (!busy) props.onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [props, busy, menu]);
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [menu]);

  const act = async (work: () => Promise<unknown>) => {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError((e as Error).message);
    }
    load();
  };

  const tags = useMemo(() => {
    const count = new Map<string, number>();
    for (const p of list ?? []) for (const t of p.info?.tags ?? []) count.set(t, (count.get(t) ?? 0) + 1);
    return [...count].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([t]) => t);
  }, [list]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    const keep = (p: Project) =>
      (!q || titleOf(p).toLowerCase().includes(q) || p.name.includes(q) || (p.info?.description ?? "").toLowerCase().includes(q) || (p.info?.tags ?? []).some((t) => t.includes(q))) &&
      (!tag || (p.info?.tags ?? []).includes(tag)) &&
      (filter === "all" || (filter === "mine" && !p.team?.joined) || (filter === "shared" && !!p.team?.joined) || (filter === "team" && !!p.team));
    const order = (a: Project, b: Project) => (sort === "name" ? titleOf(a).localeCompare(titleOf(b)) : sort === "created" ? (b.created ?? 0) - (a.created ?? 0) : (b.modified ?? 0) - (a.modified ?? 0));
    return (list ?? []).filter(keep).sort(order);
  }, [list, query, filter, sort, tag]);

  const searching = !!query || !!tag;
  const groups = filter === "all" ? [{ title: "Your projects", items: shown.filter((p) => !p.team?.joined), withNew: true }, { title: "Shared with you", items: shown.filter((p) => p.team?.joined), withNew: false }] : [{ title: "", items: shown, withNew: filter !== "shared" }];
  const current = (p: Project) => (projectName ? p.name === projectName : p.default);
  const slugOf = (text: string) =>
    text
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64);
  // A new project's name comes from its first title; it stays when the title changes.
  const slug = (() => {
    const base = slugOf(naming ?? "");
    if (!base) return "";
    let s = base;
    for (let n = 2; list?.some((p) => p.name === s) || trash.some((t) => t.name === s); n++) s = `${base}-${n}`;
    return s;
  })();

  return createPortal(
    <div className="projects-home" role="dialog" aria-label="Projects">
      <header className="projects-head">
        <div>
          <h1>Projects</h1>
          <span>
            {list ? `${list.length} ${list.length === 1 ? "project" : "projects"}` : "Loading…"}
            {list && list.some((p) => p.team?.joined) ? ` · ${list.filter((p) => p.team?.joined).length} shared with you` : ""}
          </span>
        </div>
        <div className="projects-actions">
          <button type="button" className="secondary" onClick={load}>
            <Icon name="refresh" /> Refresh
          </button>
          <button type="button" className="primary" onClick={() => setNaming("")}>
            <Icon name="plus" /> New project
          </button>
          <button type="button" className="projects-close" onClick={props.onClose} aria-label="Close">
            <Icon name="close" />
          </button>
        </div>
      </header>
      <div className="projects-bar">
        <label className="projects-search">
          <Icon name="cursor" />
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search titles, descriptions, tags…" aria-label="Search projects" autoFocus />
        </label>
        <div className="segmented" role="group" aria-label="Show">
          {(
            [
              ["all", "All"],
              ["mine", "Yours"],
              ["shared", "Shared with you"],
              ["team", "Team"],
            ] as [Filter, string][]
          ).map(([v, label]) => (
            <button key={v} type="button" className={filter === v ? "active" : ""} aria-pressed={filter === v} onClick={() => setFilter(v)}>
              {label}
            </button>
          ))}
        </div>
        <label className="projects-sort">
          Sort
          <select value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
            <option value="edited">Last edited</option>
            <option value="name">Title</option>
            <option value="created">Created</option>
          </select>
        </label>
      </div>
      {tags.length > 0 && (
        <div className="projects-tags" role="group" aria-label="Tags">
          {tags.map((t) => (
            <button key={t} type="button" className={`tag-chip ${tag === t ? "active" : ""}`} aria-pressed={tag === t} onClick={() => setTag(tag === t ? null : t)}>
              #{t}
            </button>
          ))}
        </div>
      )}
      {error && (
        <p className="projects-error" role="alert">
          {error}
        </p>
      )}
      <div className="projects-body">
        {groups.map((g) =>
          g.items.length || g.withNew ? (
            <section key={g.title || "all"}>
              {g.title && (
                <h2>
                  {g.title} <small>{g.items.length}</small>
                </h2>
              )}
              <div className="projects-grid">
                {g.withNew && !searching && (
                  <button type="button" className="project-card new" onClick={() => setNaming("")}>
                    <span className="project-new-icon">
                      <Icon name="plus" />
                    </span>
                    New project
                  </button>
                )}
                {g.items.map((p) => (
                  <div key={p.name} className={`project-card ${current(p) ? "current" : ""} ${menu === p.name ? "menu-open" : ""}`}>
                    <button type="button" className="project-open-area" onClick={() => (current(p) ? props.onClose() : openProject(p.name, p.default))} aria-label={`Open ${titleOf(p)}`}>
                      <div className="project-thumb">
                        {p.thumbnail ? <img src={`/projects/${p.name}/thumbnail?v=${p.modified ?? 0}`} alt="" loading="lazy" /> : <span className="project-glyph">{initials(titleOf(p))}</span>}
                        {current(p) && <span className="project-open">Open now</span>}
                      </div>
                      <div className="project-meta">
                        <strong>{titleOf(p)}</strong>
                        {p.info?.description && <p className="project-desc">{p.info.description}</p>}
                        <small>
                          {p.team?.joined && p.team.by ? `From ${p.team.by} · ` : ""}
                          Edited {ago(p.modified)} · {p.revisions.toLocaleString()} changes
                        </small>
                        {(p.info?.tags.length ?? 0) > 0 && (
                          <span className="project-tags">
                            {p.info!.tags.slice(0, 4).map((t) => (
                              <span key={t}>#{t}</span>
                            ))}
                            {p.info!.tags.length > 4 && <span>+{p.info!.tags.length - 4}</span>}
                          </span>
                        )}
                      </div>
                    </button>
                    {p.team && (
                      <div className="project-team" title={p.team.members.map((m) => `${m.name} (${m.role})`).join(", ")}>
                        {p.team.members.slice(0, 3).map((m) => (
                          <span key={m.name}>{initials(m.name)}</span>
                        ))}
                        {p.team.members.length > 3 && <span>+{p.team.members.length - 3}</span>}
                      </div>
                    )}
                    <button
                      type="button"
                      className="project-more"
                      aria-label={`More for ${titleOf(p)}`}
                      aria-expanded={menu === p.name}
                      onClick={(e) => {
                        e.stopPropagation();
                        setMenu(menu === p.name ? null : p.name);
                      }}
                    >
                      <Icon name="dots" />
                    </button>
                    {menu === p.name && (
                      <div className="project-menu" role="menu" onClick={(e) => e.stopPropagation()}>
                        <button type="button" role="menuitem" onClick={() => (setMenu(null), setEditing(p))}>
                          <Icon name="pencil" /> Edit details
                        </button>
                        <button type="button" role="menuitem" onClick={() => (setMenu(null), void act(() => post(`/projects/${p.name}/duplicate`)))}>
                          <Icon name="copy" /> Duplicate
                        </button>
                        {!p.default && (
                          <button type="button" role="menuitem" className="danger" onClick={() => (setMenu(null), setDeleting(p))}>
                            <Icon name="trash" /> Delete
                          </button>
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>
              {!g.items.length && g.title === "Shared with you" && !searching && <p className="projects-empty">Nothing shared with you yet. When a teammate invites you, their project appears here.</p>}
            </section>
          ) : null,
        )}
        {list && !shown.length && searching && <p className="projects-empty">No project matches{query ? ` “${query}”` : ""}{tag ? ` tagged #${tag}` : ""}.</p>}
        {trash.length > 0 && (
          <section className="projects-trash">
            <button type="button" className="trash-toggle" aria-expanded={showTrash} onClick={() => setShowTrash((v) => !v)}>
              <Icon name="trash" /> Recently deleted <small>{trash.length}</small>
              <Icon name="down" />
            </button>
            {showTrash && (
              <ul>
                {trash.map((t) => (
                  <li key={t.entry}>
                    <span>
                      <b>{t.title || prettyName(t.name)}</b>
                      <small>
                        Deleted {ago(t.deleted)} · {t.revisions.toLocaleString()} changes
                      </small>
                    </span>
                    <button
                      type="button"
                      className="secondary"
                      disabled={t.taken}
                      title={t.taken ? "A project with this name exists now" : "Put it back"}
                      onClick={() => void act(() => post(`/projects/trash/${encodeURIComponent(t.entry)}/restore`))}
                    >
                      Restore
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        )}
      </div>
      {naming !== null && (
        <form
          className="projects-name floating"
          onSubmit={(e) => {
            e.preventDefault();
            if (slug) openProject(slug, false, naming.trim());
          }}
        >
          <strong>New project</strong>
          <input autoFocus value={naming} onChange={(e) => setNaming(e.target.value)} placeholder="Title" aria-label="Title" onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), setNaming(null))} />
          <div>
            <button type="button" className="secondary" onClick={() => setNaming(null)}>
              Cancel
            </button>
            <button type="submit" className="primary" disabled={!slug}>
              Create
            </button>
          </div>
        </form>
      )}
      {editing && (
        <DetailsEditor
          project={editing}
          suggestions={tags}
          onCancel={() => setEditing(null)}
          onSave={(info) => {
            const name = editing.name;
            setEditing(null);
            void act(() => post(`/projects/${name}/info`, info));
          }}
        />
      )}
      {deleting && (
        <div className="projects-name floating confirm" role="alertdialog" aria-label="Delete project">
          <strong>Delete “{titleOf(deleting)}”?</strong>
          <p>
            It moves to Recently deleted, where you can restore it.
            {deleting.team ? " Your teammates keep their own copies; this computer stops syncing it." : ""}
          </p>
          <div>
            <button type="button" className="secondary" autoFocus onClick={() => setDeleting(null)}>
              Cancel
            </button>
            <button
              type="button"
              className="danger"
              onClick={() => {
                const p = deleting;
                setDeleting(null);
                void act(() => post(`/projects/${p.name}/delete`)).then(() => current(p) && openProject("", true));
              }}
            >
              Delete
            </button>
          </div>
        </div>
      )}
    </div>,
    document.body,
  );
}

/** Title, description and tags of one project. */
function DetailsEditor(props: { project: Project; suggestions: string[]; onCancel: () => void; onSave: (info: { title: string; description: string; tags: string[] }) => void }) {
  const p = props.project;
  const [title, setTitle] = useState(titleOf(p));
  const [description, setDescription] = useState(p.info?.description ?? "");
  const [tags, setTags] = useState<string[]>(p.info?.tags ?? []);
  const [draft, setDraft] = useState("");
  const tagInput = useRef<HTMLInputElement>(null);
  const addTag = (raw: string) => {
    const t = raw.trim().replace(/^#/, "").toLowerCase().replace(/\s+/g, " ");
    if (t && t.length <= 40 && !tags.includes(t)) setTags([...tags, t]);
    setDraft("");
  };
  const offered = props.suggestions.filter((s) => !tags.includes(s) && (!draft || s.includes(draft.toLowerCase()))).slice(0, 6);
  return (
    <form
      className="projects-name floating details"
      onSubmit={(e) => {
        e.preventDefault();
        if (title.trim()) props.onSave({ title: title.trim(), description: description.trim(), tags: draft.trim() ? [...tags, draft.trim().toLowerCase()] : tags });
      }}
      onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), props.onCancel())}
    >
      <strong>Project details</strong>
      <label>
        Title
        <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} maxLength={120} required />
      </label>
      <label>
        Description
        <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={3} maxLength={4000} placeholder="What it is, who it's for, what to watch for…" />
      </label>
      <label>
        Tags
        <span className="tag-field" onClick={() => tagInput.current?.focus()}>
          {tags.map((t) => (
            <span key={t} className="tag-chip on">
              #{t}
              <button type="button" aria-label={`Remove ${t}`} onClick={() => setTags(tags.filter((x) => x !== t))}>
                <Icon name="close" />
              </button>
            </span>
          ))}
          <input
            ref={tagInput}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={tags.length ? "" : "Add a tag and press Enter"}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === ",") {
                e.preventDefault();
                addTag(draft);
              } else if (e.key === "Backspace" && !draft && tags.length) setTags(tags.slice(0, -1));
            }}
          />
        </span>
      </label>
      {offered.length > 0 && (
        <div className="tag-offers">
          {offered.map((s) => (
            <button key={s} type="button" className="tag-chip" onClick={() => addTag(s)}>
              #{s}
            </button>
          ))}
        </div>
      )}
      <small className="details-name">
        Name <code>{p.default ? "default" : p.name}</code> stays the same: links and teammates' computers know the project by it.
      </small>
      <div>
        <button type="button" className="secondary" onClick={props.onCancel}>
          Cancel
        </button>
        <button type="submit" className="primary" disabled={!title.trim()}>
          Save
        </button>
      </div>
    </form>
  );
}
