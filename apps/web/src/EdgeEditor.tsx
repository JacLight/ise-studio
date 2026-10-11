// Rounding and bevelling edges by hand. Pick edges of the selected part in
// the view; the engine previews the true fillet (or chamfer) live, and Apply
// stores it as an edge finish sized by a new parameter of the part, which
// can then be linked like any other (=Style.radius).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { EdgePicture } from "./edgeLayer";
import type { Engine, EngineError, EntityView, MeshData, PartEdges, PickEdge, ShapeStats } from "./engine";
import { Icon } from "./icons";
import { type EdgeFinish, type EdgePick, program } from "./model";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
type Kind = EdgeFinish["kind"];
export type FinishPreview = { id: string; stats: ShapeStats; mesh: MeshData } | null;

const LABEL: Record<Kind, string> = { fillet: "Fillet", chamfer: "Chamfer", push: "Push" };

/** The point halfway along a polyline. */
function middle(points: [number, number, number][]): [number, number, number] {
  const d = (a: number[], b: number[]) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
  let half = points.slice(1).reduce((s, p, i) => s + d(points[i], p), 0) / 2;
  for (let i = 1; i < points.length; i++) {
    const l = d(points[i - 1], points[i]);
    if (l >= half && l > 0) return points[i - 1].map((v, k) => v + ((points[i][k] - v) * half) / l) as [number, number, number];
    half -= l;
  }
  return points[0];
}

/** An edge as stored: what it is and where it sits relative to the part's bounds, so it follows a resize. */
export function pickOf(edge: PickEdge, bounds: PartEdges["bounds"]): EdgePick {
  const ref = edge.kind === "circle" && edge.center ? edge.center : middle(edge.points);
  const at = [0, 1, 2].map((k) => {
    const size = bounds[k + 3] - bounds[k];
    return size > 1e-9 ? Math.round(((ref[k] - bounds[k]) / size) * 1e6) / 1e6 : 0.5;
  }) as [number, number, number];
  return { at, kind: edge.kind, ...(edge.dir ? { dir: edge.dir } : {}), ...(edge.radius !== undefined ? { radius: edge.radius } : {}) };
}

function newKey(kind: Kind, taken: string[]) {
  let n = 1;
  while (taken.includes(`${kind}${n}`)) n++;
  return `${kind}${n}`;
}

export function useEdgeEditor(props: {
  engine: Engine;
  entities: ReadonlyMap<string, EntityView>;
  run: Run;
  notify: (m: string) => void;
  setPreview: (p: FinishPreview) => void;
}) {
  const { engine, run, setPreview } = props;
  const [editing, setEditing] = useState<string | null>(null);
  const [parts, setParts] = useState<PartEdges[] | null>(null);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [kind, setKind] = useState<Kind>("fillet");
  const [size, setSize] = useState("3");
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const entity = editing ? props.entities.get(editing) : undefined;
  const prog = entity ? program(entity) : undefined;
  const seq = useRef(0);

  const stop = useCallback(() => {
    seq.current++;
    setEditing(null);
    setParts(null);
    setPicked(new Set());
    setProblem(null);
    setPreview(null);
    setPreviewing(false);
  }, [setPreview]);

  const start = useCallback(
    async (id: string) => {
      const e = props.entities.get(id);
      if (!e || !program(e)) return props.notify("Select a part made by a program to round its edges");
      setEditing(id);
      setPicked(new Set());
      setProblem(null);
      setParts(null);
      try {
        const r = await engine.shapeEdges(id);
        setParts(r.parts);
      } catch (err) {
        props.notify((err as EngineError).message);
        setEditing(null);
      }
    },
    [engine, props],
  );

  // The part changed (undo, another session, a resize): its edges are fetched again.
  const meshRef = entity?.mesh;
  useEffect(() => {
    if (!editing || !parts) return;
    let live = true;
    engine.shapeEdges(editing).then(
      (r) => live && (setParts(r.parts), setPicked(new Set())),
      () => {},
    );
    return () => {
      live = false;
    };
    // Only when the part's own mesh changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meshRef]);

  const value = Number(size.replace(/mm$/, "").trim());
  const sizeOk = Number.isFinite(value) && value > 0;
  const key = useMemo(() => newKey(kind, prog?.params.map((p) => p.key) ?? []), [kind, prog]);

  /** The finishes the picked edges make: one per part they are on, all sized by one new parameter. */
  const finishes = useMemo((): EdgeFinish[] => {
    if (!parts) return [];
    const byPart = new Map<number, EdgePick[]>();
    for (const k of picked) {
      const [p, e] = k.split(":").map(Number);
      const edge = parts[p]?.edges[e];
      if (edge) byPart.set(p, [...(byPart.get(p) ?? []), pickOf(edge, parts[p].bounds)]);
    }
    return [...byPart].map(([p, edges]) => ({ kind, key, ...(parts[p].part ? { part: parts[p].part! } : {}), edges }));
  }, [parts, picked, kind, key]);

  // Live preview of the true result as edges are picked and the size changes.
  useEffect(() => {
    if (!editing) return;
    const n = ++seq.current;
    if (!finishes.length || !sizeOk) {
      setPreview(null);
      setPreviewing(false);
      setProblem(null);
      return;
    }
    const timer = setTimeout(async () => {
      try {
        const r = await engine.shapePreview(editing, { [key]: value }, [...(prog?.finishes ?? []), ...finishes]);
        if (n === seq.current) {
          setPreview({ id: editing, stats: r.stats, mesh: r.mesh });
          setPreviewing(true);
          setProblem(null);
        }
      } catch (err) {
        if (n === seq.current) {
          setPreview(null);
          setPreviewing(false);
          setProblem((err as EngineError).message.replace(/^program: (Error: )?/, ""));
        }
      }
    }, 90);
    return () => clearTimeout(timer);
  }, [editing, finishes, key, value, sizeOk, engine, prog, setPreview]);

  const apply = useCallback(async () => {
    if (!editing || !prog || !finishes.length || !sizeOk) return;
    const n = (prog.finishes ?? []).length + 1;
    setBusy(true);
    const ok = await run("shape.update", {
      target: editing,
      params: [...prog.params, { key, label: `${LABEL[kind]} ${n} ${kind === "fillet" ? "radius" : "distance"}`, value, unit: "mm", min: null, max: null, step: 0.5 }],
      finishes: [...(prog.finishes ?? []), ...finishes],
    });
    setBusy(false);
    if (ok) stop();
  }, [editing, prog, finishes, sizeOk, key, kind, value, run, stop]);

  const toggle = useCallback((k: string) => {
    setPicked((s) => {
      const next = new Set(s);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }, []);

  // Esc leaves; Enter applies.
  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        stop();
      } else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) {
        e.preventDefault();
        void apply();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [editing, stop, apply]);

  const picture: EdgePicture | null = useMemo(() => (editing && parts ? { id: editing, parts, picked, previewing } : null), [editing, parts, picked, previewing]);

  const count = picked.size;
  const bar =
    editing && prog ? (
      <div className="sketch-bar edge-bar floating" role="toolbar" aria-label="Round or bevel edges">
        <div className="sketch-head">
          <strong>{prog.name}</strong>
          <span className={`sketch-status ${problem ? "bad" : count ? "ok" : "free"}`}>
            {!parts ? "Finding edges…" : problem ? problem : count ? `${count} ${count === 1 ? "edge" : "edges"} · previewing` : "Click edges to pick them"}
          </span>
        </div>
        <div className="edge-row">
          <div className="segmented" role="radiogroup" aria-label="Kind">
            {(["fillet", "chamfer"] as Kind[]).map((k) => (
              <button key={k} type="button" role="radio" aria-checked={kind === k} className={kind === k ? "active" : ""} onClick={() => setKind(k)}>
                {k === "fillet" ? <RoundGlyph /> : <BevelGlyph />}
                {LABEL[k]}
              </button>
            ))}
          </div>
          <label className="edge-size">
            <span>{kind === "fillet" ? "Radius" : "Distance"}</span>
            <input
              value={size}
              inputMode="decimal"
              aria-label={kind === "fillet" ? "Fillet radius in mm" : "Chamfer distance in mm"}
              className={sizeOk ? "" : "invalid"}
              onChange={(e) => setSize(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "ArrowUp" || e.key === "ArrowDown") {
                  e.preventDefault();
                  const step = e.shiftKey ? 1 : 0.5;
                  setSize(String(Math.max(step, Math.round(((sizeOk ? value : 0) + (e.key === "ArrowUp" ? step : -step)) * 100) / 100)));
                }
              }}
            />
            <small>mm</small>
          </label>
          {count > 0 && (
            <button type="button" className="link-button" onClick={() => setPicked(new Set())}>
              Clear
            </button>
          )}
        </div>
        <p className="sketch-hint">
          Click an edge to pick it, again to drop it · drag to orbit · the size becomes a parameter you can link (=Style.radius) · Enter applies, Esc
          cancels
        </p>
        <div className="sketch-actions">
          <button type="button" className="primary" onClick={apply} disabled={!count || !sizeOk || !!problem || busy}>
            <Icon name="check" /> {LABEL[kind]} {count ? `${count} ${count === 1 ? "edge" : "edges"}` : ""}
          </button>
          <button type="button" className="secondary" onClick={stop}>
            Cancel
          </button>
        </div>
      </div>
    ) : null;

  return { editing, picture, toggle, bar, start, stop };
}

function RoundGlyph() {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 20V12a8 8 0 0 1 8-8h8" />
    </svg>
  );
}

function BevelGlyph() {
  return (
    <svg className="icon" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 20v-9l7-7h9" />
    </svg>
  );
}
