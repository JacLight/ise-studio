// A read-only look at a design: a past version on this computer, or a
// view-only link opened by someone without Forma. The engine serves a session
// that refuses every change; this screen offers none: orbit, the parts list,
// and the comments (read-only).

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { type CommentPin, type CommentsView, CommentsPanel, ThreadCard, pinsFor, useThreads } from "./Comments";
import { ComponentTree } from "./Components";
import { type Engine, projectName } from "./engine";
import { Icon } from "./icons";
import { formatMass, isPart, partName } from "./model";
import { Compare, useVersionActions } from "./Versions";
import { type ViewName, Viewport, type ViewportHandle } from "./Viewport";

const noop = () => {};

export function Viewer({ engine }: { engine: Engine }) {
  const state = useSyncExternalStore(engine.subscribe, engine.getState);
  const ro = state.readOnly;
  const viewport = useRef<ViewportHandle>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [selectedPart, setSelectedPart] = useState<string | null>(null);
  const [hidden, setHidden] = useState<ReadonlySet<string>>(() => new Set());
  const [isolated, setIsolated] = useState<string | null>(null);
  const [tab, setTab] = useState<"parts" | "comments">("parts");
  const [panelOpen, setPanelOpen] = useState(() => window.innerWidth > 720);
  const [view, setView] = useState<ViewName>("perspective");
  const [openThread, setOpenThread] = useState<string | null>(null);
  const [filter, setFilter] = useState<"open" | "resolved" | "all">("open");
  const [pinsShown, setPinsShown] = useState(true);
  const [comparing, setComparing] = useState(false);
  const [everOnline, setEverOnline] = useState(false);
  useEffect(() => {
    if (state.connection === "online") setEverOnline(true);
  }, [state.connection]);
  useEffect(() => setSelectedPart(null), [selected]);

  const parts = useMemo(() => [...state.entities.values()].filter(isPart), [state.entities]);
  const threads = useThreads(state.entities);
  const title = ro?.title ?? "";
  useEffect(() => {
    document.title = ro ? (ro.kind === "share" ? `${ro.title} · Forma viewer` : `${ro.label} · ${ro.title} · Forma`) : "Forma";
  }, [ro]);
  // Frame the model once it first arrives.
  const framed = useRef(false);
  useEffect(() => {
    if (!framed.current && parts.length) {
      framed.current = true;
      requestAnimationFrame(() => viewport.current?.fit());
    }
  }, [parts.length]);

  const comments = useMemo((): CommentsView | null => {
    const active = openThread ? threads.find((t) => t.id === openThread) : undefined;
    let pins: CommentPin[] = pinsShown ? pinsFor(threads, filter !== "open") : [];
    if (filter === "resolved") pins = pins.filter((p) => p.resolved);
    if (active && !active.gone && !pins.some((p) => p.id === active.id)) pins = [...pins, ...pinsFor([active], true)];
    if (!pins.length && !active) return null;
    return {
      pins,
      active: openThread,
      placing: false,
      tool: null,
      me: { initials: "", color: "#7352d6" },
      card: active ? <ThreadCard thread={active} me="" run={async () => null} onClose={() => setOpenThread(null)} readOnly /> : null,
      onPlace: noop,
      onOpen: setOpenThread,
      onStroke: noop,
    };
  }, [threads, openThread, pinsShown, filter]);

  const picked = selected ? state.entities.get(selected) : undefined;
  const actions = useVersionActions();
  const version = ro?.kind === "version";
  const isLink = location.pathname.startsWith("/view/");
  const failed = !everOnline && state.connection === "offline";
  const gone = state.closedWhy ?? (isLink && failed ? "It was turned off or has expired." : null);

  if (!isLink && failed) {
    return (
      <div className="viewer-gone">
        <div>
          <span className="viewer-gone-icon">
            <Icon name="history" />
          </span>
          <h1>This version can't be shown</h1>
          <p>It may be gone, or some of its changes have not reached this computer yet (they arrive when the teammate who made them is online).</p>
          <button
            type="button"
            className="primary"
            title="Open the project as it is now"
            onClick={() => {
              const url = new URL(location.href);
              url.searchParams.delete("version");
              url.searchParams.delete("at");
              location.assign(url.toString());
            }}
          >
            Back to the project
          </button>
        </div>
      </div>
    );
  }

  if (gone) {
    return (
      <div className="viewer-gone">
        <div>
          <span className="viewer-gone-icon">
            <Icon name="link" />
          </span>
          <h1>This link isn't available</h1>
          <p>{gone} Ask whoever shared it for a new one.</p>
        </div>
      </div>
    );
  }

  const back = () => {
    const url = new URL(location.href);
    url.searchParams.delete("version");
    url.searchParams.delete("at");
    location.assign(url.toString());
  };

  return (
    <div className={`app viewer ${version ? "is-version" : "is-share"}`}>
      <main className="workspace" aria-label="Model (read only)">
        <Viewport
          ref={viewport}
          entities={state.entities}
          selected={selected}
          live={null}
          overlays={[]}
          pivotMode={false}
          pivotLive={null}
          onPickPivot={noop}
          onSelect={(id) => setSelected(id)}
          comments={comments}
          selectedPart={selectedPart}
          onSelectPart={setSelectedPart}
          hidden={hidden}
          isolated={isolated}
          colorBy="material"
          sketch={null}
          onSketchPointer={noop}
          onEditDimension={noop}
          edges={null}
          onEdgeClick={noop}
          facePicks={null}
          programFace={null}
          onProgramFace={noop}
          onFaceDrag={noop}
          onFaceDragEnd={noop}
          onDrag={noop}
          onDragEnd={noop}
        />
        {state.connection !== "online" && everOnline && <div className="offline-note">{version ? "Engine offline · reconnecting…" : "Connection lost · reconnecting…"}</div>}
        {!everOnline && (
          <div className="viewer-loading" role="status">
            <span className="viewer-spinner" /> {version ? "Rebuilding this version…" : "Opening the model…"}
          </div>
        )}
        {everOnline && parts.length === 0 && <div className="empty-hint"><p>{version ? "Nothing had been made yet at this version." : "Nothing has been made in this project yet."}</p></div>}
      </main>

      <header className="viewer-bar floating">
        <span className="brand">
          <Icon name="cube" />
          <b>forma</b>
        </span>
        <span className="viewer-title">
          <b>{title || (version ? "" : "Shared model")}</b>
          {version && <small>{ro?.label}</small>}
        </span>
        <span className="viewer-badge" title={version ? "A past version: nothing can be changed here" : "Anyone with the link can view this model; nobody can change it here"}>
          <Icon name="eye" /> {version ? "Read only" : "View only"}
        </span>
        {version && (
          <span className="viewer-actions">
            <button type="button" className="secondary" title="List what changed between this version and the project now" onClick={() => setComparing((c) => !c)} aria-pressed={comparing}>
              Compare with now
            </button>
            <button type="button" className="secondary" title={`Bring the design back to ${ro?.label ?? "this version"} as one new change`} onClick={() => void actions.restore({ version: ro?.version ?? undefined, revision: revisionParam() }, ro?.label ?? "")}>
              <Icon name="undo" /> Restore
            </button>
            <button type="button" className="secondary" title={`Start a new project from ${ro?.label ?? "this version"}`} onClick={() => void actions.branch({ version: ro?.version ?? undefined, revision: revisionParam() }, ro?.label ?? "")}>
              <Icon name="copy" /> Branch
            </button>
            <button type="button" className="primary" title="Leave this past version and show the project as it is now" onClick={back}>
              Back to now
            </button>
          </span>
        )}
      </header>
      {actions.message && <div className={`viewer-toast ${actions.message.bad ? "bad" : ""}`} role="status">{actions.message.text}</div>}
      {actions.confirm}

      {comparing && version && (
        <aside className="tool-panel floating viewer-compare" aria-label="Compare with now">
          <header className="panel-handle">
            <strong>{ro?.label} → now</strong>
            <button type="button" className="ib" aria-label="Close compare" onClick={() => setComparing(false)}>
              <Icon name="close" />
            </button>
          </header>
          <div className="panel-body">
            <Compare project={projectName} which={{ version: ro?.version ?? undefined, revision: revisionParam() }} onPick={(id) => setSelected(id)} />
          </div>
        </aside>
      )}

      {panelOpen ? (
        <aside className="tool-panel floating viewer-panel" aria-label="Parts and comments">
          <header className="panel-handle">
            <div className="seg" role="tablist" aria-label="Show">
              <button type="button" role="tab" aria-selected={tab === "parts"} className={tab === "parts" ? "active" : ""} title="The parts in this model; click one to find it" onClick={() => setTab("parts")}>
                Parts {parts.length || ""}
              </button>
              <button type="button" role="tab" aria-selected={tab === "comments"} className={tab === "comments" ? "active" : ""} title="Comments on this model (the count is the open ones)" onClick={() => setTab("comments")}>
                Comments {threads.filter((t) => !t.resolved).length || ""}
              </button>
            </div>
            <button type="button" className="ib" aria-label="Hide panel" title="Hide panel" onClick={() => setPanelOpen(false)}>
              <Icon name="close" />
            </button>
          </header>
          <div className="panel-body">
            {tab === "parts" ? (
              parts.length ? (
                <ComponentTree
                  entities={state.entities}
                  selected={selected}
                  hidden={hidden}
                  isolated={isolated}
                  onSelect={(id) => {
                    setSelected(id);
                    viewport.current?.frame(id, true);
                  }}
                  onToggleHidden={(id) =>
                    setHidden((h) => {
                      const n = new Set(h);
                      if (n.has(id)) n.delete(id);
                      else n.add(id);
                      return n;
                    })
                  }
                  onIsolate={setIsolated}
                />
              ) : (
                <p className="muted small">No parts.</p>
              )
            ) : (
              <CommentsPanel
                threads={threads}
                active={openThread}
                filter={filter}
                onFilter={setFilter}
                pinsShown={pinsShown}
                onPinsShown={setPinsShown}
                onOpen={(id) => {
                  setOpenThread(id);
                  viewport.current?.framePin(id);
                }}
                readOnly
              />
            )}
          </div>
        </aside>
      ) : (
        <button type="button" className="viewer-panel-open floating" title="Show the parts list and comments" onClick={() => setPanelOpen(true)}>
          <Icon name="layers" /> Parts · Comments
        </button>
      )}

      {picked && isPart(picked) && (
        <div className="viewer-picked floating" role="status">
          <b>{selectedPart ?? partName(picked)}</b>
          <small>{selectedPart ? partName(picked) : formatMass(picked.mass?.mass_kg ?? picked.assembly?.mass_kg)}</small>
          <button type="button" className="ib" aria-label="Clear selection" onClick={() => setSelected(null)}>
            <Icon name="close" />
          </button>
        </div>
      )}

      <div className="viewer-views floating" role="toolbar" aria-label="View">
        {(["perspective", "front", "top"] as ViewName[]).map((v) => (
          <button
            key={v}
            type="button"
            className={view === v ? "active" : ""}
            title={v === "perspective" ? "Look at the model in perspective, from an angle" : `Look straight at the model's ${v}`}
            onClick={() => {
              setView(v);
              viewport.current?.view(v);
            }}
          >
            {v[0].toUpperCase() + v.slice(1)}
          </button>
        ))}
        <button type="button" title="Fit all" aria-label="Fit all" onClick={() => viewport.current?.fit()}>
          <Icon name="fit" />
        </button>
      </div>
    </div>
  );
}

function revisionParam() {
  const at = new URLSearchParams(location.search).get("at");
  return at ? Number(at) : undefined;
}
