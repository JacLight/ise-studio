// A stress study on a part: pick the faces it is held by and the faces
// loads push on, run it, and read the stress map, the largest stress, how
// far it bends, and the safety factor against its material's yield.

import { useEffect, useMemo, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";
import type { ProgramFace, StressView } from "./Viewport";

type V3 = [number, number, number];
type Face = { point: V3; normal: V3; triangles: number[] };
/** A face as kept with the study: where on the part it sits (fractions of the part's bounds) and which way it faces. */
type Pick = { at: V3; probe: V3; normal: V3 };
type Spec = { part: string | null; supports: Pick[]; loads: { face: Pick; magnitude: number; direction: Direction }[]; resolution: number };
/** The saved faces found on the part as it is now (null: no longer there). */
type Resolved = { supports: (Face | null)[]; loads: { face: Face | null; force: V3 | null }[] };
type Direction = "into" | "-z" | "+z" | "-x" | "+x" | "-y" | "+y";
type Result = NonNullable<StressView["result"]> & { yield_strength: number; safety: number; elements: number; iterations: number; notes: string[] };
type Modes = {
  cell: number;
  origin: [number, number, number];
  cells: [number, number, number][];
  nodes: [number, number, number][];
  modes: { frequency: number; direction: string; amount: number[]; shape: [number, number, number][] }[];
  mass: number;
  notes: string[];
};

const DIRECTIONS: { value: Direction; label: string }[] = [
  { value: "into", label: "Into the face" },
  { value: "-z", label: "Down (−Z)" },
  { value: "+z", label: "Up (+Z)" },
  { value: "-x", label: "−X" },
  { value: "+x", label: "+X" },
  { value: "-y", label: "−Y" },
  { value: "+y", label: "+Y" },
];
const RESOLUTIONS = [
  { label: "Quick", value: 32 },
  { label: "Normal", value: 56 },
  { label: "Fine", value: 96 },
];

const n = (v: number, d = 1) => Number(v.toFixed(d)).toLocaleString("en-US");

const emptySpec = (part: string | null): Spec => ({ part, supports: [], loads: [], resolution: 56 });

export function StressStudy(props: {
  id: string;
  part: string | null;
  version: unknown;
  material: string | null;
  /** The part's bounds (part coordinates), to keep faces by where they sit. */
  bounds: { min: V3; max: V3 } | null;
  /** The shape's saved studies as stored (changes when anyone edits them). */
  saved: unknown;
  run: (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
  onPickFace: (purpose: string, take: (f: ProgramFace) => void) => void;
  onShow: (id: string, view: StressView | null) => void;
}) {
  const [spec, setSpec] = useState<Spec>(() => emptySpec(props.part));
  const [resolved, setResolved] = useState<Resolved>({ supports: [], loads: [] });
  const resolution = spec.resolution;
  const [state, setState] = useState<{ kind: "idle" | "busy" } | { kind: "done"; result: Result; material: string } | { kind: "error"; text: string }>({ kind: "idle" });
  const [deform, setDeform] = useState(false);
  const [vib, setVib] = useState<{ kind: "busy" } | { kind: "done"; modes: Modes } | { kind: "error"; text: string } | null>(null);
  const [mode, setMode] = useState<number | null>(null);

  // The saved study, its faces found on the part as it is now.
  useEffect(() => {
    let live = true;
    fetch(projectUrl(`/study/${props.id}${props.part ? `?part=${encodeURIComponent(props.part)}` : ""}`))
      .then((r) => (r.ok ? r.json() : null))
      .then((out: ({ spec: Spec | null } & Partial<Resolved>) | null) => {
        if (!live || !out) return;
        setSpec(out.spec ?? emptySpec(props.part));
        setResolved({ supports: out.supports ?? [], loads: out.loads ?? [] });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [props.id, props.part, props.version, props.saved]);
  // Another part, or a new version of it: earlier results no longer apply.
  useEffect(() => {
    setState({ kind: "idle" });
    setVib(null);
    setMode(null);
  }, [props.id, props.part, props.version]);

  const supports = resolved.supports.filter((f): f is Face => !!f);
  const loads = resolved.loads.filter((l): l is { face: Face; force: V3 } => !!l.face && !!l.force);
  /** A picked face kept by where it sits on the part. */
  const pickOf = (f: ProgramFace): Pick => {
    const b = props.bounds;
    const frac = (p: V3): V3 => (b ? ([0, 1, 2].map((k) => (b.max[k] - b.min[k] > 1e-9 ? (p[k] - b.min[k]) / (b.max[k] - b.min[k]) : 0.5)) as V3) : [0.5, 0.5, 0.5]);
    const t = f.triangles;
    const probe: V3 = [(t[0] + t[3] + t[6]) / 3, (t[1] + t[4] + t[7]) / 3, (t[2] + t[5] + t[8]) / 3];
    return { at: frac(f.centroid), probe: frac(probe), normal: f.normal };
  };
  /** Keeps a change with the project (it syncs to teammates and undoes like any edit). */
  const save = (next: Spec) => {
    setSpec(next);
    changed();
    void props.run("shape.set_study", { target: props.id, study: { ...next, part: props.part } });
  };

  const view = useMemo((): StressView => {
    const faces: StressView["faces"] = [...supports.map((f) => ({ kind: "support" as const, triangles: f.triangles, point: f.point })), ...loads.map((l) => ({ kind: "load" as const, triangles: l.face.triangles, point: l.face.point, force: l.force }))];
    // A vibration mode, when one is picked, shows swinging; else the stress map.
    if (vib?.kind === "done" && mode !== null) {
      const m = vib.modes.modes[mode];
      return {
        faces: faces.filter((f) => f.kind === "support"),
        oscillate: true,
        result: {
          cell: vib.modes.cell,
          origin: vib.modes.origin,
          cells: vib.modes.cells.map((c, i) => [c, m.amount[i]] as [[number, number, number], number]),
          displacements: vib.modes.nodes.map((c, i) => [c, m.shape[i]] as [[number, number, number], [number, number, number]]),
          max_stress: 1,
          max_displacement: 1,
        },
      };
    }
    return { faces, result: state.kind === "done" ? state.result : undefined, deform: deform ? 1 : 0 };
  }, [resolved, state, deform, vib, mode]); // eslint-disable-line react-hooks/exhaustive-deps
  const { id, onShow } = props;
  useEffect(() => {
    onShow(id, view.faces.length || view.result ? view : null);
  }, [id, view, onShow]);
  useEffect(() => () => onShow(id, null), [id, onShow]);

  const run = async () => {
    setState({ kind: "busy" });
    try {
      const r = await fetch(projectUrl(`/stress/${props.id}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ supports, loads: loads.map((l) => ({ face: l.face, force: l.force })), resolution, part: props.part }),
      });
      if (!r.ok) return setState({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { result: Result; material: string };
      setState({ kind: "done", result: out.result, material: out.material });
    } catch {
      setState({ kind: "error", text: "The engine did not answer" });
    }
  };

  function changed() {
    setState((s) => (s.kind === "done" ? { kind: "idle" } : s));
    setVib(null);
    setMode(null);
  }
  const ring = async () => {
    setVib({ kind: "busy" });
    setMode(null);
    try {
      const r = await fetch(projectUrl(`/modes/${props.id}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ supports, resolution: Math.min(resolution, 64), part: props.part, count: 4 }),
      });
      if (!r.ok) return setVib({ kind: "error", text: await r.text() });
      const modes = (await r.json()) as Modes;
      setVib({ kind: "done", modes });
      setMode(0);
    } catch {
      setVib({ kind: "error", text: "The engine did not answer" });
    }
  };
  const r = state.kind === "done" ? state.result : null;
  const tone = r ? (r.safety >= 2 ? "ok" : r.safety >= 1.2 ? "warn" : "bad") : "";
  return (
    <section className="props-section stress-study">
      <header>
        <h3>Stress</h3>
        <span className="section-note">{props.material ? `${props.material} · typical properties` : "Give the part a material first"}</span>
      </header>
      <div className="stress-list">
        {spec.supports.map((_, i) => (
          <div key={`s${i}`} className={`stress-item support ${resolved.supports[i] === null ? "lost" : ""}`}>
            <span className="stress-dot" />
            {resolved.supports[i] === null ? "Face no longer found: pick it again" : `Held at face ${i + 1}`}
            <button type="button" aria-label="Remove this support" onClick={() => save({ ...spec, supports: spec.supports.filter((_, j) => j !== i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
        {spec.loads.map((l, i) => (
          <div key={`l${i}`} className={`stress-item load ${resolved.loads[i]?.face === null ? "lost" : ""}`}>
            <span className="stress-dot" />
            <input
              defaultValue={String(l.magnitude)}
              key={`${i}:${l.magnitude}`}
              inputMode="decimal"
              aria-label="Force in newtons"
              onBlur={(e) => {
                const v = Number(e.target.value);
                if (Number.isFinite(v) && v !== l.magnitude) save({ ...spec, loads: spec.loads.map((x, j) => (j === i ? { ...x, magnitude: v } : x)) });
              }}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            />
            N
            <select value={l.direction} aria-label="Direction" onChange={(e) => save({ ...spec, loads: spec.loads.map((x, j) => (j === i ? { ...x, direction: e.target.value as Direction } : x)) })}>
              {DIRECTIONS.map((d) => (
                <option key={d.value} value={d.value}>
                  {d.label}
                </option>
              ))}
            </select>
            <button type="button" aria-label="Remove this load" onClick={() => save({ ...spec, loads: spec.loads.filter((_, j) => j !== i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
      </div>
      <div className="stress-add">
        <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face the part is held by", (f) => save({ ...spec, supports: [...spec.supports, pickOf(f)] }))}>
          <Icon name="plus" /> Support
        </button>
        <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face the force pushes on", (f) => save({ ...spec, loads: [...spec.loads, { face: pickOf(f), magnitude: 100, direction: "into" }] }))}>
          <Icon name="plus" /> Load
        </button>
        <div className="segmented" role="group" aria-label="Resolution">
          {RESOLUTIONS.map((x) => (
            <button key={x.value} type="button" className={x.value === resolution ? "active" : ""} aria-pressed={x.value === resolution} onClick={() => save({ ...spec, resolution: x.value })}>
              {x.label}
            </button>
          ))}
        </div>
      </div>
      <div className="stress-runs">
        <button type="button" className="primary stress-run" disabled={!supports.length || !loads.length || state.kind === "busy" || !props.material} onClick={run} title="How hard the loads work the material, and how far it bends">
          <Icon name="weight" /> {state.kind === "busy" ? "Solving…" : "Stress"}
        </button>
        <button type="button" className="secondary stress-run" disabled={!supports.length || vib?.kind === "busy" || !props.material} onClick={ring} title="The speeds it wants to vibrate at, held at its supports (loads not needed)">
          <Icon name="refresh" /> {vib?.kind === "busy" ? "Finding…" : "Vibration"}
        </button>
      </div>
      {vib?.kind === "error" && <p className="print-note bad">{vib.text}</p>}
      {vib?.kind === "done" && (
        <div className="mode-list">
          {vib.modes.modes.map((m, i) => (
            <button key={i} type="button" className={`mode-row ${mode === i ? "active" : ""}`} onClick={() => setMode(mode === i ? null : i)} aria-pressed={mode === i}>
              <span>Mode {i + 1}</span>
              <b>{n(m.frequency, m.frequency < 100 ? 1 : 0)} Hz</b>
              <small>
                {n(m.frequency * 60, 0)} rpm · mostly {m.direction.toUpperCase()}
              </small>
            </button>
          ))}
          <p className="motion-note">Avoid running a motor or engine near these speeds: the part would shake itself. {vib.modes.notes.join(" ")}</p>
        </div>
      )}
      {state.kind === "error" && <p className="print-note bad">{state.text}</p>}
      {r && (
        <div className="stress-result">
          <div className={`stress-verdict ${tone}`}>
            <b>Safety ×{n(r.safety, 2)}</b> against yield ({n(r.yield_strength, 0)} MPa)
          </div>
          <dl>
            <dt>Largest stress</dt>
            <dd>{n(r.max_stress, 1)} MPa</dd>
            <dt>Moves at most</dt>
            <dd>{n(r.max_displacement, r.max_displacement < 0.1 ? 4 : 3)} mm</dd>
          </dl>
          <div className="stress-legend" aria-hidden="true">
            <span>0</span>
            <i />
            <span>{n(r.max_stress, 0)} MPa</span>
          </div>
          <label className="xray-toggle">
            <input type="checkbox" checked={deform} onChange={(e) => setDeform(e.target.checked)} /> Show the bending (exaggerated)
          </label>
          <p className="motion-note">{r.notes.join(" ")}</p>
        </div>
      )}
    </section>
  );
}
