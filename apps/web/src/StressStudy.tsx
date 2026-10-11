// A stress study on a part: pick the faces it is held by and the faces
// loads push on, run it, and read the stress map, the largest stress, how
// far it bends, and the safety factor against its material's yield. Parts
// of an assembly that touch it can be studied with it: they press on it
// where they touch (sliding freely, gripping with friction, or glued), and
// the contact pressure shows as a layer of its own.

import { useEffect, useMemo, useState } from "react";
import { type PartStats, projectUrl } from "./engine";
import { Icon } from "./icons";
import type { ProgramFace, StressView } from "./Viewport";

type V3 = [number, number, number];
type Face = { point: V3; normal: V3; triangles: number[]; part?: string | null };
/** A face as kept with the study: where on the part it sits (fractions of the part's bounds) and which way it faces; `part` when on another part studied with it. */
type Pick = { at: V3; probe: V3; normal: V3; part?: string };
type ContactKind = "frictionless" | "friction" | "bonded";
type ContactSpec = { parts: [string, string]; kind: ContactKind; friction: number; press_fit?: boolean };
type Convection = { h: number; ambient: number };
type Spec = {
  part: string | null;
  supports: Pick[];
  loads: { face: Pick; magnitude: number; direction: Direction }[];
  holds: { face: Pick; temperature: number }[];
  heats: { face: Pick; watts: number }[];
  convection: Convection | null;
  resolution: number;
  /** Other parts studied with it, and how each pair acts where they touch. */
  parts: string[];
  contacts: ContactSpec[];
  /** The heat study's temperatures load the stress study, stress-free at `reference` °C. */
  heat_stress?: { reference: number } | null;
  /** Re-solve the most stressed spot on finer cubes for its true peak. */
  refine?: boolean;
};
/** The saved faces found on the part as it is now (null: no longer there). */
type Resolved = { supports: (Face | null)[]; loads: { face: Face | null; force: V3 | null }[]; holds: { face: Face | null }[]; heats: { face: Face | null }[] };
type Temps = { cell: number; origin: V3; cells: [V3, number][]; max: number; min: number; heat_in: number; to_holds: number; to_air: number; notes: string[] };
const AIR: { label: string; h: number }[] = [
  { label: "No air", h: 0 },
  { label: "Still air", h: 10 },
  { label: "Fan", h: 50 },
];
type Direction = "into" | "-z" | "+z" | "-x" | "+x" | "-y" | "+y";
type PartResult = { name: string; max_stress: number; max_displacement: number; yield_strength: number; safety: number; floating: boolean };
type ContactPair = { parts: [string, string]; kind: ContactKind; friction: number; area: number; pressing: number; slipping: number; normal_force: number; shear_force: number; max_pressure: number; press_fit?: boolean; interference?: number; mean_pressure: number };
type HeatSummary = { reference: number; min: number; max: number; thermal_stress: number | null; mechanical_stress: number | null };
type Peak = { at: V3; stress: number; coarse: number; cell: number; change: number; singular: boolean; at_support: boolean };
type Contacts = { faces: { cell: V3; dir: number; pressure: number; pair: number }[]; opened: [number, number, number, number][]; max_pressure: number; pairs: ContactPair[]; rounds: number; settled: boolean };
type Result = NonNullable<StressView["result"]> & { yield_strength: number; safety: number; elements: number; iterations: number; notes: string[]; parts?: PartResult[]; contact?: Contacts; heat?: HeatSummary; peak?: Peak };
const KINDS: { value: ContactKind; label: string; title: string }[] = [
  { value: "frictionless", label: "Slides", title: "Frictionless: they push on each other, never pull, and slide freely" },
  { value: "friction", label: "Grips", title: "Friction: they push, never pull, and grip sideways up to μ × the push" },
  { value: "bonded", label: "Glued", title: "Bonded: they move together where they touch" },
];
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

const emptySpec = (part: string | null): Spec => ({ part, supports: [], loads: [], holds: [], heats: [], convection: null, resolution: 56, parts: [], contacts: [], heat_stress: null, refine: true });
const tone = (safety: number) => (safety >= 2 ? "ok" : safety >= 1.2 ? "warn" : "bad");
const noFaces: Resolved = { supports: [], loads: [], holds: [], heats: [] };

export function StressStudy(props: {
  id: string;
  part: string | null;
  version: unknown;
  material: string | null;
  /** The part's bounds (part coordinates), to keep faces by where they sit. */
  bounds: { min: V3; max: V3 } | null;
  /** The shape's saved studies as stored (changes when anyone edits them). */
  saved: unknown;
  /** Every part of the assembly (empty for a single solid). */
  assembly: PartStats[];
  run: (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
  /** Asks for a face; `parts`: other parts whose faces may be picked too. */
  onPickFace: (purpose: string, take: (f: ProgramFace) => void, parts?: string[]) => void;
  onShow: (id: string, view: StressView | null) => void;
}) {
  const [spec, setSpec] = useState<Spec>(() => emptySpec(props.part));
  const [resolved, setResolved] = useState<Resolved>(noFaces);
  const [heat, setHeat] = useState<{ kind: "busy" } | { kind: "done"; temps: Temps } | { kind: "error"; text: string } | null>(null);
  // Which result the view shows: the last one run.
  const [showing, setShowing] = useState<"stress" | "modes" | "heat">("stress");
  const resolution = spec.resolution;
  const [state, setState] = useState<{ kind: "idle" | "busy" } | { kind: "done"; result: Result; material: string } | { kind: "error"; text: string }>({ kind: "idle" });
  const [deform, setDeform] = useState(false);
  const [vib, setVib] = useState<{ kind: "busy" } | { kind: "done"; modes: Modes } | { kind: "error"; text: string } | null>(null);
  const [mode, setMode] = useState<number | null>(null);
  // The parts that touch this one (null: still finding out), and which result layer shows.
  const [touching, setTouching] = useState<string[] | null>(null);
  const [layer, setLayer] = useState<"stress" | "contact">("stress");

  // The saved study, its faces found on the part as it is now.
  useEffect(() => {
    let live = true;
    fetch(projectUrl(`/study/${props.id}${props.part ? `?part=${encodeURIComponent(props.part)}` : ""}`))
      .then((r) => (r.ok ? r.json() : null))
      .then((out: ({ spec: Spec | null } & Partial<Resolved>) | null) => {
        if (!live || !out) return;
        setSpec({ ...emptySpec(props.part), ...(out.spec ?? {}) });
        setResolved({ supports: out.supports ?? [], loads: out.loads ?? [], holds: out.holds ?? [], heats: out.heats ?? [] });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [props.id, props.part, props.version, props.saved]);
  // The other parts it touches: the ones that can press on it in the study.
  const several = !!props.part && props.assembly.length > 1;
  useEffect(() => {
    if (!several || !props.part) return setTouching([]);
    let live = true;
    setTouching(null);
    fetch(projectUrl(`/touching/${props.id}?part=${encodeURIComponent(props.part)}`))
      .then((r) => (r.ok ? r.json() : { parts: [] }))
      .then((out: { parts: string[] }) => live && setTouching(out.parts))
      .catch(() => live && setTouching([]));
    return () => {
      live = false;
    };
  }, [props.id, props.part, props.version, several]);
  // Another part, or a new version of it: earlier results no longer apply.
  useEffect(() => {
    setState({ kind: "idle" });
    setVib(null);
    setMode(null);
    setHeat(null);
  }, [props.id, props.part, props.version]);

  const supports = resolved.supports.filter((f): f is Face => !!f);
  const loads = resolved.loads.filter((l): l is { face: Face; force: V3 } => !!l.face && !!l.force);
  /** A picked face kept by where it sits on its part (this one, or another studied with it). */
  const pickOf = (f: ProgramFace): Pick => {
    const other = f.part && f.part !== props.part ? props.assembly.find((p) => p.name === f.part) : undefined;
    const b = other ? { min: other.bounds_min, max: other.bounds_max } : props.bounds;
    const frac = (p: V3): V3 => (b ? ([0, 1, 2].map((k) => (b.max[k] - b.min[k] > 1e-9 ? (p[k] - b.min[k]) / (b.max[k] - b.min[k]) : 0.5)) as V3) : [0.5, 0.5, 0.5]);
    const t = f.triangles;
    const probe: V3 = [(t[0] + t[3] + t[6]) / 3, (t[1] + t[4] + t[7]) / 3, (t[2] + t[5] + t[8]) / 3];
    return { at: frac(f.centroid), probe: frac(probe), normal: f.normal, ...(other ? { part: other.name } : {}) };
  };
  /** Asks for a face on this part or any studied with it. */
  const pickFace = (purpose: string, take: (f: ProgramFace) => void) => props.onPickFace(purpose, take, spec.parts);
  /** The contact between this part and another (frictionless unless set). */
  const contactWith = (name: string): ContactSpec => spec.contacts.find((c) => c.parts.includes(name) && c.parts.includes(props.part ?? "")) ?? { parts: [props.part ?? "", name], kind: "frictionless", friction: 0.3 };
  const setContact = (name: string, change: Partial<ContactSpec>) => {
    const next = { ...contactWith(name), ...change };
    save({ ...spec, contacts: [...spec.contacts.filter((c) => !(c.parts.includes(name) && c.parts.includes(props.part ?? ""))), next] });
  };
  /** Takes another part into the study, or leaves it out (with its supports and loads). */
  const include = (name: string, on: boolean) => {
    if (on) return save({ ...spec, parts: [...spec.parts, name] });
    const mine = (p: Pick) => p.part !== name;
    save({
      ...spec,
      parts: spec.parts.filter((p) => p !== name),
      contacts: spec.contacts.filter((c) => !c.parts.includes(name)),
      supports: spec.supports.filter(mine),
      loads: spec.loads.filter((l) => mine(l.face)),
    });
  };
  /** Keeps a change with the project (it syncs to teammates and undoes like any edit). */
  const save = (next: Spec) => {
    setSpec(next);
    changed();
    void props.run("shape.set_study", { target: props.id, study: { ...next, part: props.part } });
  };

  const view = useMemo((): StressView => {
    const thermalFaces: StressView["faces"] = [
      ...resolved.holds.flatMap((h) => (h.face ? [{ kind: "hold" as const, triangles: h.face.triangles, point: h.face.point }] : [])),
      ...resolved.heats.flatMap((h) => (h.face ? [{ kind: "heat" as const, triangles: h.face.triangles, point: h.face.point }] : [])),
    ];
    if (showing === "heat" && heat?.kind === "done") {
      const t = heat.temps;
      return { faces: thermalFaces, result: { cell: t.cell, origin: t.origin, cells: t.cells.map(([c, v]) => [c, v - t.min] as [V3, number]), displacements: [], max_stress: Math.max(t.max - t.min, 1e-9), max_displacement: 1 } };
    }
    const faces: StressView["faces"] = [
      ...supports.map((f) => ({ kind: "support" as const, triangles: f.triangles, point: f.point })),
      ...loads.map((l) => ({ kind: "load" as const, triangles: l.face.triangles, point: l.face.point, force: l.force })),
      ...thermalFaces,
    ];
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
    const result = state.kind === "done" ? state.result : undefined;
    const contact = result?.contact ? { faces: result.contact.faces, opened: result.contact.opened, max_pressure: result.contact.max_pressure } : undefined;
    const peak = result?.peak ? { at: result.peak.at, singular: result.peak.singular } : undefined;
    return { faces, result, deform: deform ? 1 : 0, contact, layer: contact ? layer : "stress", peak };
  }, [resolved, state, deform, vib, mode, heat, showing, layer]); // eslint-disable-line react-hooks/exhaustive-deps
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
        body: JSON.stringify({
          supports,
          loads: loads.map((l) => ({ face: l.face, force: l.force })),
          resolution,
          part: props.part,
          parts: spec.parts,
          contacts: spec.contacts.filter((c) => c.parts.every((p) => p === props.part || spec.parts.includes(p))),
          refine: spec.refine !== false && !spec.parts.length,
          heat: heatOn ? { reference: spec.heat_stress!.reference, thermal: { holds, heats, convection: spec.convection && spec.convection.h > 0 ? spec.convection : null } } : null,
        }),
      });
      if (!r.ok) return setState({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { result: Result; material: string };
      setState({ kind: "done", result: out.result, material: out.material });
      setShowing("stress");
    } catch {
      setState({ kind: "error", text: "The engine did not answer" });
    }
  };

  function changed() {
    setState((s) => (s.kind === "done" ? { kind: "idle" } : s));
    setVib(null);
    setMode(null);
    setHeat(null);
  }
  const holds = resolved.holds.map((h, i) => (h.face ? { face: h.face, temperature: spec.holds[i]?.temperature ?? 20 } : null)).filter((x): x is { face: Face; temperature: number } => !!x);
  const heats = resolved.heats.map((h, i) => (h.face ? { face: h.face, watts: spec.heats[i]?.watts ?? 0 } : null)).filter((x): x is { face: Face; watts: number } => !!x);
  const heatOn = !!spec.heat_stress && (holds.length > 0 || !!(spec.convection && spec.convection.h > 0));
  const pressing = spec.contacts.some((c) => c.press_fit && spec.parts.some((p) => c.parts.includes(p)));
  const runHeat = async () => {
    setHeat({ kind: "busy" });
    setMode(null);
    try {
      const r = await fetch(projectUrl(`/thermal/${props.id}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ holds, heats, convection: spec.convection && spec.convection.h > 0 ? spec.convection : null, resolution: Math.min(resolution, 72), part: props.part }),
      });
      if (!r.ok) return setHeat({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { result: Temps };
      setHeat({ kind: "done", temps: out.result });
      setShowing("heat");
    } catch {
      setHeat({ kind: "error", text: "The engine did not answer" });
    }
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
      setShowing("modes");
    } catch {
      setVib({ kind: "error", text: "The engine did not answer" });
    }
  };
  const r = state.kind === "done" ? state.result : null;
  const verdict = r ? tone(r.safety) : "";
  const onWhat = (p: Pick) => (p.part ? ` on ${p.part}` : "");
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
            {resolved.supports[i] === null ? "Face no longer found: pick it again" : `Held at face ${i + 1}${onWhat(spec.supports[i])}`}
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
            {l.face.part && <span className="stress-on">{l.face.part}</span>}
            <button type="button" aria-label="Remove this load" onClick={() => save({ ...spec, loads: spec.loads.filter((_, j) => j !== i) })}>
              <Icon name="close" />
            </button>
          </div>
        ))}
      </div>
      <div className="stress-add">
        <button type="button" className="secondary" onClick={() => pickFace(spec.parts.length ? "Click the face it is held by (on this part or one studied with it)" : "Click the face the part is held by", (f) => save({ ...spec, supports: [...spec.supports, pickOf(f)] }))}>
          <Icon name="plus" /> Support
        </button>
        <button type="button" className="secondary" onClick={() => pickFace(spec.parts.length ? "Click the face the force pushes on (on this part or one studied with it)" : "Click the face the force pushes on", (f) => save({ ...spec, loads: [...spec.loads, { face: pickOf(f), magnitude: 100, direction: "into" }] }))}>
          <Icon name="plus" /> Load
        </button>
        {!spec.parts.length && (
          <label className="xray-toggle" title="Re-solve the most stressed spot on cubes up to 8× finer and read the stress at the surface itself">
            <input type="checkbox" checked={spec.refine !== false} onChange={(e) => save({ ...spec, refine: e.target.checked })} /> Sharpen the peak
          </label>
        )}
        <div className="segmented" role="group" aria-label="Resolution">
          {RESOLUTIONS.map((x) => (
            <button key={x.value} type="button" className={x.value === resolution ? "active" : ""} aria-pressed={x.value === resolution} onClick={() => save({ ...spec, resolution: x.value })}>
              {x.label}
            </button>
          ))}
        </div>
      </div>
      {several && (
        <div className="contact-block">
          <h4>Parts touching it</h4>
          {touching === null && <p className="motion-note">Finding the parts that touch it…</p>}
          {touching?.length === 0 && <p className="motion-note">No other part touches it: it is studied on its own.</p>}
          {touching && [...new Set([...touching, ...spec.parts])].map((name) => {
            const on = spec.parts.includes(name);
            const c = contactWith(name);
            const material = props.assembly.find((p) => p.name === name)?.material;
            return (
              <div key={name} className={`contact-row ${on ? "on" : ""}`}>
                <label title={material ? `${material}: its own stiffness` : "Give it a material to study it"}>
                  <input type="checkbox" checked={on} disabled={!material && !on} onChange={(e) => include(name, e.target.checked)} />
                  <span>{name}</span>
                  {!material && <small>no material</small>}
                </label>
                {on && (
                  <div className="segmented" role="group" aria-label={`How ${name} acts where it touches`}>
                    {KINDS.map((k) => (
                      <button key={k.value} type="button" title={k.title} className={c.kind === k.value ? "active" : ""} aria-pressed={c.kind === k.value} onClick={() => setContact(name, { kind: k.value })}>
                        {k.label}
                      </button>
                    ))}
                  </div>
                )}
                {on && c.kind !== "bonded" && (
                  <label className="contact-mu" title="Press fit: where the parts overlap as drawn, the overlap is an interference squeezed out, so they start pressed together">
                    <input type="checkbox" checked={!!c.press_fit} onChange={(e) => setContact(name, { press_fit: e.target.checked })} /> Press fit
                  </label>
                )}
                {on && c.kind === "friction" && (
                  <label className="contact-mu" title="Coefficient of friction: about 0.15 oiled steel, 0.5 dry steel, 0.6 rubber on concrete">
                    μ
                    <input
                      defaultValue={String(c.friction)}
                      key={`${name}:${c.friction}`}
                      inputMode="decimal"
                      aria-label="Coefficient of friction"
                      onBlur={(e) => {
                        const v = Number(e.target.value);
                        if (Number.isFinite(v) && v >= 0 && v !== c.friction) setContact(name, { friction: v });
                      }}
                      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                    />
                  </label>
                )}
              </div>
            );
          })}
          {spec.parts.length > 0 && <p className="motion-note">They press on it where they touch, never pull (unless glued). Supports and loads can go on them too; a part with no support of its own is held only by its contacts.</p>}
        </div>
      )}
      <div className="stress-runs">
        <button type="button" className="primary stress-run" disabled={!supports.length || (!loads.length && !heatOn && !pressing) || state.kind === "busy" || !props.material} onClick={run} title="How hard the loads work the material, and how far it bends">
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
          <div className={`stress-verdict ${verdict}`}>
            <b>Safety ×{n(r.safety, 2)}</b> against yield ({n(r.yield_strength, 0)} MPa){r.parts && ` · ${r.parts.reduce((a, b) => (b.safety < a.safety ? b : a)).name}`}
          </div>
          {r.parts && (
            <div className="part-results">
              {r.parts.map((p) => (
                <div key={p.name} className="part-result">
                  <span>{p.name}</span>
                  {p.floating ? (
                    <small className="bad">slides away: not held</small>
                  ) : (
                    <small>
                      {n(p.max_stress, 1)} MPa · {n(p.max_displacement, p.max_displacement < 0.1 ? 4 : 3)} mm
                    </small>
                  )}
                  <b className={tone(p.safety)}>×{n(p.safety, 2)}</b>
                </div>
              ))}
            </div>
          )}
          <dl>
            <dt>Largest stress</dt>
            <dd>{n(r.max_stress, 1)} MPa</dd>
            <dt>Moves at most</dt>
            <dd>{n(r.max_displacement, r.max_displacement < 0.1 ? 4 : 3)} mm</dd>
            {r.peak && (
              <>
                <dt>{r.peak.singular ? "Sharp inside corner" : "Peak at the surface"}</dt>
                <dd className={r.peak.singular ? "warn" : ""} title={`Re-solved on ${n(r.peak.cell, 3)} mm cubes; the coarse cubes read ${n(r.peak.coarse, 1)} MPa there`}>
                  {r.peak.singular ? "no finite peak: add a fillet" : `${n(r.peak.stress, 1)} MPa ±${n(r.peak.change * 100, 0)}%`}
                </dd>
              </>
            )}
            {r.heat && (
              <>
                <dt>Temperatures</dt>
                <dd>
                  {n(r.heat.min, 0)}–{n(r.heat.max, 0)} °C (stress-free at {n(r.heat.reference, 0)} °C)
                </dd>
                {r.heat.thermal_stress !== null && r.heat.mechanical_stress !== null && (
                  <>
                    <dt>From heat · from loads</dt>
                    <dd>
                      {n(r.heat.thermal_stress, 1)} · {n(r.heat.mechanical_stress, 1)} MPa
                    </dd>
                  </>
                )}
              </>
            )}
          </dl>
          {r.contact && (
            <div className="contact-results">
              {r.contact.pairs.map((c) => (
                <div key={c.parts.join("|")} className="contact-result">
                  <span>
                    {c.parts[0]} ↔ {c.parts[1]} <small>{c.kind === "bonded" ? "glued" : c.kind === "friction" ? `μ ${c.friction}` : "sliding"}</small>
                  </span>
                  {c.press_fit && (
                    <small>
                      Press fit: {n((c.interference ?? 0) * 1000, 1)} µm interference squeezes them together at {n(c.mean_pressure, 1)} MPa on average
                    </small>
                  )}
                  <small>
                    {n(c.normal_force, 1)} N push · {n(c.shear_force, 1)} N sideways · {n(c.pressing, 0)} of {n(c.area, 0)} mm² pressing{c.slipping > 0 ? ` (${n(c.slipping, 0)} slipping)` : ""} · up to {n(c.max_pressure, 1)} MPa
                  </small>
                </div>
              ))}
              <div className="segmented" role="group" aria-label="Show">
                <button type="button" className={layer === "stress" ? "active" : ""} aria-pressed={layer === "stress"} onClick={() => setLayer("stress")}>
                  Stress
                </button>
                <button type="button" className={layer === "contact" ? "active" : ""} aria-pressed={layer === "contact"} onClick={() => setLayer("contact")}>
                  Contact pressure
                </button>
              </div>
            </div>
          )}
          {r.contact && layer === "contact" ? (
            <>
              <div className="stress-legend" aria-hidden="true">
                <span>0</span>
                <i />
                <span>{n(r.contact.max_pressure, 1)} MPa</span>
              </div>
              {r.contact.opened.length > 0 && (
                <p className="motion-note">
                  <span className="gap-dot" /> Where touching faces came apart (up to {n(Math.max(...r.contact.opened.map((o) => o[3])) * 1000, 1)} µm).
                </p>
              )}
            </>
          ) : (
            <>
              <div className="stress-legend" aria-hidden="true">
                <span>0</span>
                <i />
                <span>{n(r.max_stress, 0)} MPa</span>
              </div>
              <label className="xray-toggle">
                <input type="checkbox" checked={deform} onChange={(e) => setDeform(e.target.checked)} /> Show the bending (exaggerated)
              </label>
            </>
          )}
          <p className="motion-note">{r.notes.join(" ")}</p>
        </div>
      )}
      <div className="heat-block">
        <h4>Heat</h4>
        <div className="stress-list">
          {spec.holds.map((h, i) => (
            <div key={`h${i}`} className={`stress-item hold ${resolved.holds[i]?.face === null ? "lost" : ""}`}>
              <span className="stress-dot" />
              Held at
              <input
                defaultValue={String(h.temperature)}
                key={`${i}:${h.temperature}`}
                inputMode="decimal"
                aria-label="Temperature in degrees Celsius"
                onBlur={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v !== h.temperature) save({ ...spec, holds: spec.holds.map((x, j) => (j === i ? { ...x, temperature: v } : x)) });
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              °C
              <button type="button" aria-label="Remove" onClick={() => save({ ...spec, holds: spec.holds.filter((_, j) => j !== i) })}>
                <Icon name="close" />
              </button>
            </div>
          ))}
          {spec.heats.map((h, i) => (
            <div key={`q${i}`} className={`stress-item heat ${resolved.heats[i]?.face === null ? "lost" : ""}`}>
              <span className="stress-dot" />
              <input
                defaultValue={String(h.watts)}
                key={`${i}:${h.watts}`}
                inputMode="decimal"
                aria-label="Heat in watts"
                onBlur={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v !== h.watts) save({ ...spec, heats: spec.heats.map((x, j) => (j === i ? { ...x, watts: v } : x)) });
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              W coming in
              <button type="button" aria-label="Remove" onClick={() => save({ ...spec, heats: spec.heats.filter((_, j) => j !== i) })}>
                <Icon name="close" />
              </button>
            </div>
          ))}
        </div>
        <div className="stress-add">
          <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face held at a temperature (a heatsink, a wall)", (f) => save({ ...spec, holds: [...spec.holds, { face: pickOf(f), temperature: 20 }] }))}>
            <Icon name="plus" /> Held at °C
          </button>
          <button type="button" className="secondary" onClick={() => props.onPickFace("Click the face the heat comes in through (a motor, a chip)", (f) => save({ ...spec, heats: [...spec.heats, { face: pickOf(f), watts: 5 }] }))}>
            <Icon name="plus" /> Heat in
          </button>
          <div className="segmented" role="group" aria-label="Air cooling">
            {AIR.map((a) => (
              <button
                key={a.h}
                type="button"
                className={(spec.convection?.h ?? 0) === a.h ? "active" : ""}
                aria-pressed={(spec.convection?.h ?? 0) === a.h}
                title={a.h ? `${a.h} W/m²·K, air at ${spec.convection?.ambient ?? 20} °C` : "No cooling by air"}
                onClick={() => save({ ...spec, convection: a.h ? { h: a.h, ambient: spec.convection?.ambient ?? 20 } : null })}
              >
                {a.label}
              </button>
            ))}
          </div>
        </div>
        <label className="xray-toggle" title="Thermal stress: the part expands where it is warmer than the stress-free temperature, and is stressed where it cannot">
          <input type="checkbox" checked={!!spec.heat_stress} onChange={(e) => save({ ...spec, heat_stress: e.target.checked ? { reference: spec.heat_stress?.reference ?? 20 } : null })} /> Include this heat in the stress study
          {spec.heat_stress && (
            <>
              , stress-free at
              <input
                className="heat-reference"
                defaultValue={String(spec.heat_stress.reference)}
                key={`ref:${spec.heat_stress.reference}`}
                inputMode="decimal"
                aria-label="Stress-free temperature in degrees Celsius"
                onBlur={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v !== spec.heat_stress?.reference) save({ ...spec, heat_stress: { reference: v } });
                }}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              °C
            </>
          )}
        </label>
        <button type="button" className="secondary stress-run heat-run" disabled={(!holds.length && !(spec.convection && spec.convection.h > 0)) || heat?.kind === "busy" || !props.material} onClick={runHeat} title="Steady temperatures: where it gets hot, and where the heat goes">
          <Icon name="alert" /> {heat?.kind === "busy" ? "Solving…" : "Heat"}
        </button>
        {heat?.kind === "error" && <p className="print-note bad">{heat.text}</p>}
        {heat?.kind === "done" && (
          <div className="stress-result">
            <dl>
              <dt>Hottest</dt>
              <dd>{n(heat.temps.max, 1)} °C</dd>
              <dt>Coolest</dt>
              <dd>{n(heat.temps.min, 1)} °C</dd>
              <dt>Heat in</dt>
              <dd>{n(heat.temps.heat_in, 2)} W</dd>
              <dt>Out to holds · air</dt>
              <dd>
                {n(heat.temps.to_holds, 2)} W · {n(heat.temps.to_air, 2)} W
              </dd>
            </dl>
            <div className="stress-legend heat-legend" aria-hidden="true">
              <span>{n(heat.temps.min, 0)} °C</span>
              <i />
              <span>{n(heat.temps.max, 0)} °C</span>
            </div>
            {showing !== "heat" && (
              <button type="button" className="link-button" onClick={() => setShowing("heat")}>
                Show the temperatures
              </button>
            )}
            <p className="motion-note">{heat.temps.notes.join(" ")}</p>
          </div>
        )}
      </div>
    </section>
  );
}
