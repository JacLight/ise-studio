// A stress study on a part: pick the faces it is held by and the faces
// loads push on, run it, and read the stress map, the largest stress, how
// far it bends, and the safety factor against its material's yield.

import { useEffect, useMemo, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";
import type { ProgramFace, StressView } from "./Viewport";

type Face = { point: [number, number, number]; normal: [number, number, number]; triangles: number[] };
type Load = { face: Face; magnitude: number; direction: Direction };
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

function forceOf(l: Load): [number, number, number] {
  const unit: [number, number, number] =
    l.direction === "into"
      ? [-l.face.normal[0], -l.face.normal[1], -l.face.normal[2]]
      : ({ "-z": [0, 0, -1], "+z": [0, 0, 1], "-x": [-1, 0, 0], "+x": [1, 0, 0], "-y": [0, -1, 0], "+y": [0, 1, 0] } as const)[l.direction].slice() as [number, number, number];
  return unit.map((c) => c * l.magnitude) as [number, number, number];
}

const faceOf = (f: ProgramFace): Face => ({ point: f.centroid, normal: f.normal, triangles: Array.from(f.triangles) });

export function StressStudy(props: {
  id: string;
  part: string | null;
  version: unknown;
  material: string | null;
  onPickFace: (purpose: string, take: (f: ProgramFace) => void) => void;
  onShow: (id: string, view: StressView | null) => void;
}) {
  const [supports, setSupports] = useState<Face[]>([]);
  const [loads, setLoads] = useState<Load[]>([]);
  const [resolution, setResolution] = useState(56);
  const [state, setState] = useState<{ kind: "idle" | "busy" } | { kind: "done"; result: Result; material: string } | { kind: "error"; text: string }>({ kind: "idle" });
  const [deform, setDeform] = useState(false);
  const [vib, setVib] = useState<{ kind: "busy" } | { kind: "done"; modes: Modes } | { kind: "error"; text: string } | null>(null);
  const [mode, setMode] = useState<number | null>(null);

  // A new version of the part (or another part) starts the study again.
  useEffect(() => {
    setSupports([]);
    setLoads([]);
    setState({ kind: "idle" });
    setVib(null);
    setMode(null);
  }, [props.id, props.part, props.version]);

  const view = useMemo((): StressView => {
    const faces: StressView["faces"] = [...supports.map((f) => ({ kind: "support" as const, triangles: f.triangles, point: f.point })), ...loads.map((l) => ({ kind: "load" as const, triangles: l.face.triangles, point: l.face.point, force: forceOf(l) }))];
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
  }, [supports, loads, state, deform, vib, mode]);
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
        body: JSON.stringify({ supports, loads: loads.map((l) => ({ face: l.face, force: forceOf(l) })), resolution, part: props.part }),
      });
      if (!r.ok) return setState({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { result: Result; material: string };
      setState({ kind: "done", result: out.result, material: out.material });
    } catch {
      setState({ kind: "error", text: "The engine did not answer" });
    }
  };

  const changed = () => {
    if (state.kind === "done") setState({ kind: "idle" });
    setVib(null);
    setMode(null);
  };
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
        {supports.map((_, i) => (
          <div key={`s${i}`} className="stress-item support">
            <span className="stress-dot" />
            Held at face {i + 1}
            <button type="button" aria-label="Remove this support" onClick={() => (setSupports(supports.filter((_, j) => j !== i)), changed())}>
              <Icon name="close" />
            </button>
          </div>
        ))}
        {loads.map((l, i) => (
          <div key={`l${i}`} className="stress-item load">
            <span className="stress-dot" />
            <input
              value={String(l.magnitude)}
              inputMode="decimal"
              aria-label="Force in newtons"
              onChange={(e) => {
                const v = Number(e.target.value);
                if (Number.isFinite(v)) setLoads(loads.map((x, j) => (j === i ? { ...x, magnitude: v } : x)));
                changed();
              }}
            />
            N
            <select value={l.direction} aria-label="Direction" onChange={(e) => (setLoads(loads.map((x, j) => (j === i ? { ...x, direction: e.target.value as Direction } : x))), changed())}>
              {DIRECTIONS.map((d) => (
                <option key={d.value} value={d.value}>
                  {d.label}
                </option>
              ))}
            </select>
            <button type="button" aria-label="Remove this load" onClick={() => (setLoads(loads.filter((_, j) => j !== i)), changed())}>
              <Icon name="close" />
            </button>
          </div>
        ))}
      </div>
      <div className="stress-add">
        <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face the part is held by", (f) => (setSupports((s) => [...s, faceOf(f)]), changed()))}>
          <Icon name="plus" /> Support
        </button>
        <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face the force pushes on", (f) => (setLoads((s) => [...s, { face: faceOf(f), magnitude: 100, direction: "into" }]), changed()))}>
          <Icon name="plus" /> Load
        </button>
        <div className="segmented" role="group" aria-label="Resolution">
          {RESOLUTIONS.map((x) => (
            <button key={x.value} type="button" className={x.value === resolution ? "active" : ""} aria-pressed={x.value === resolution} onClick={() => (setResolution(x.value), changed())}>
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
