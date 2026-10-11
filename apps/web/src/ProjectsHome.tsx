// Every project on this computer, as cards with a picture of each: the ones
// you started and the ones teammates shared with you, searchable, sortable,
// grouped. Opening one loads it in its own scene.

import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { projectName, projectTitle } from "./engine";
import { Icon } from "./icons";

type Project = {
  name: string;
  modified: number | null;
  created: number | null;
  revisions: number;
  default: boolean;
  thumbnail: boolean;
  team: { members: { name: string; role: string }[]; joined: boolean; by: string } | null;
};

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

export function openProject(name: string, isDefault: boolean) {
  const url = new URL(location.href);
  url.searchParams.delete("home");
  if (isDefault) url.searchParams.delete("project");
  else url.searchParams.set("project", name);
  location.assign(url.toString());
}

export function ProjectsHome(props: { onClose: () => void }) {
  const [list, setList] = useState<Project[] | null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("edited");
  const [naming, setNaming] = useState<string | null>(null);

  const load = useCallback(() => {
    fetch("/projects")
      .then((r) => r.json())
      .then(setList)
      .catch(() => setList([]));
  }, []);
  useEffect(load, [load]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && naming === null) {
        e.stopPropagation();
        props.onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [props, naming]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    const keep = (p: Project) =>
      (!q || projectTitle(p.default ? "" : p.name).toLowerCase().includes(q) || p.name.includes(q)) &&
      (filter === "all" || (filter === "mine" && !p.team?.joined) || (filter === "shared" && !!p.team?.joined) || (filter === "team" && !!p.team));
    const order = (a: Project, b: Project) =>
      sort === "name" ? projectTitle(a.default ? "" : a.name).localeCompare(projectTitle(b.default ? "" : b.name)) : sort === "created" ? (b.created ?? 0) - (a.created ?? 0) : (b.modified ?? 0) - (a.modified ?? 0);
    return (list ?? []).filter(keep).sort(order);
  }, [list, query, filter, sort]);

  const groups = filter === "all" ? [{ title: "Your projects", items: shown.filter((p) => !p.team?.joined), withNew: true }, { title: "Shared with you", items: shown.filter((p) => p.team?.joined), withNew: false }] : [{ title: "", items: shown, withNew: filter !== "shared" }];
  const current = (p: Project) => (projectName ? p.name === projectName : p.default);
  const slug = (naming ?? "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);

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
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search projects…" aria-label="Search projects" autoFocus />
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
            <option value="name">Name</option>
            <option value="created">Created</option>
          </select>
        </label>
      </div>
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
                {g.withNew && !query && (
                  <button type="button" className="project-card new" onClick={() => setNaming("")}>
                    <span className="project-new-icon">
                      <Icon name="plus" />
                    </span>
                    New project
                  </button>
                )}
                {g.items.map((p) => (
                  <button key={p.name} type="button" className={`project-card ${current(p) ? "current" : ""}`} onClick={() => (current(p) ? props.onClose() : openProject(p.name, p.default))}>
                    <div className="project-thumb">
                      {p.thumbnail ? <img src={`/projects/${p.name}/thumbnail?v=${p.modified ?? 0}`} alt="" loading="lazy" /> : <span className="project-glyph">{initials(projectTitle(p.default ? "" : p.name))}</span>}
                      {current(p) && <span className="project-open">Open now</span>}
                    </div>
                    <div className="project-meta">
                      <strong>{projectTitle(p.default ? "" : p.name)}</strong>
                      <small>
                        {p.team?.joined && p.team.by ? `From ${p.team.by} · ` : ""}
                        Edited {ago(p.modified)} · {p.revisions.toLocaleString()} changes
                      </small>
                    </div>
                    {p.team && (
                      <div className="project-team" title={p.team.members.map((m) => `${m.name} (${m.role})`).join(", ")}>
                        {p.team.members.slice(0, 3).map((m) => (
                          <span key={m.name}>{initials(m.name)}</span>
                        ))}
                        {p.team.members.length > 3 && <span>+{p.team.members.length - 3}</span>}
                      </div>
                    )}
                  </button>
                ))}
              </div>
              {!g.items.length && g.title === "Shared with you" && <p className="projects-empty">Nothing shared with you yet. When a teammate invites you, their project appears here.</p>}
            </section>
          ) : null,
        )}
        {list && !shown.length && query && <p className="projects-empty">No project matches “{query}”.</p>}
      </div>
      {naming !== null && (
        <form
          className="projects-name floating"
          onSubmit={(e) => {
            e.preventDefault();
            if (slug) openProject(slug, false);
          }}
        >
          <strong>New project</strong>
          <input autoFocus value={naming} onChange={(e) => setNaming(e.target.value)} placeholder="Project name" aria-label="Project name" onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), setNaming(null))} />
          <div>
            <button type="button" className="secondary" onClick={() => setNaming(null)}>
              Cancel
            </button>
            <button type="submit" className="primary" disabled={!slug || !!list?.some((p) => p.name === slug)}>
              Create
            </button>
          </div>
          {list?.some((p) => p.name === slug) && <small>There is already a project called that.</small>}
        </form>
      )}
    </div>,
    document.body,
  );
}
