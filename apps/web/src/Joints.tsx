// Joints between instances, in an assembly's properties: the list with each
// joint's values (scrubbed live in the view, limited, linked by formula),
// adding a joint by picking a connector on each of two instances in the view,
// and sweeping a joint through its range for clashes.

import { useEffect, useMemo, useRef, useState } from "react";
import type { ConnectorMark, ConnectorPicture } from "./connectorLayer";
import { type EntityView, projectUrl } from "./engine";
import { Icon } from "./icons";
import { type Transform, type V3, applyT, childrenOf, fmt, instance, qrotate } from "./model";
import { ScrubField } from "./ScrubField";
import type { FlexShape } from "./flexPlayback";
import { Interlocks } from "./Interlocks";
import { LinkNote } from "./Panels";
import type { AssemblyOverlay, GeometryHit } from "./Viewport";
import { Couplings, DofSummary, MateBuilder, MateGlyph, type MateKind, couplingOf, isMate } from "./AssemblyTools";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

export type JointKind = "fixed" | "hinge" | "slider" | "cylinder" | "ball" | MateKind;
type Geometry = { type: string; at?: V3; normal?: V3; dir?: V3; radius?: number; points?: V3[] };
type End = { instance: string; connector: string } | { instance: string; geometry: Geometry } | "ground" | { at: V3; axis?: V3; x?: V3 };
export type JointData = { kind: JointKind; assembly?: string; a: End; b: End; offset?: number; aligned?: boolean; free?: boolean };
type Param = { key: string; label: string; value: number; unit: string; min?: number | null; max?: number | null; step?: number | null; expr?: string };

export const jointOf = (e: EntityView | undefined) => e?.components["assembly.joint"] as JointData | undefined;
const paramsOf = (e: EntityView) => (e.components["shape.parameter_set"] as { name: string; params: Param[] } | undefined) ?? { name: "Joint", params: [] };
export const jointName = (e: EntityView) => paramsOf(e).name;

/** What picking connectors in the view needs: the triads to draw and what a click on one does. */
export type ConnectorPick = { picture: ConnectorPicture; onPick: (key: string) => void };
/** How the joints panel (and the assembly tools) reach the view. */
export type JointView = {
  pick: (p: ConnectorPick | null) => void;
  /** Instances drawn at these placements (null: back where they are; flexing shapes go back too). */
  poses: (m: Map<string, Transform> | null) => void;
  /**
   * Shapes a played motion changes (a spring stretching), at fractional frame
   * `frame` of their values; resolves when that frame's meshes are evaluated.
   */
  flex: (f: { shapes: FlexShape[]; frame: number } | null) => Promise<void>;
  /** Lines and dots over the model (project mm), or none. */
  overlay: (o: AssemblyOverlay | null) => void;
  /** Clicks on parts report the face picked (null: back to selecting). */
  geometry: (f: ((hit: GeometryHit) => void) | null) => void;
  /** The view as a PNG data URL, rendered now. */
  snapshot: () => string | null;
};

const KINDS: { kind: JointKind; label: string; hint: string }[] = [
  { kind: "fixed", label: "Fixed", hint: "Held together, no motion" },
  { kind: "hinge", label: "Hinge", hint: "Turns about the connector axis" },
  { kind: "slider", label: "Slider", hint: "Slides along the axis" },
  { kind: "cylinder", label: "Cylinder", hint: "Turns about and slides along the axis" },
  { kind: "ball", label: "Ball", hint: "Turns any way about the point" },
];
const unitOf = (p: Param) => (p.unit === "deg" ? "°" : p.unit);
const n = (v: number, d = 1) => Number(v.toFixed(d)).toLocaleString("en-US");

/** A small drawing of each kind of joint. */
export function JointGlyph({ kind }: { kind: JointKind }) {
  if (isMate(kind)) return <MateGlyph kind={kind} />;
  const paths: Record<Exclude<JointKind, MateKind>, React.ReactNode> = {
    fixed: (
      <>
        <rect x="2.5" y="8.5" width="11" height="5" rx="1.2" />
        <rect x="5" y="3" width="6" height="5.5" rx="1" />
        <path d="M8 4.8v5.4" />
      </>
    ),
    hinge: (
      <>
        <path d="M3 13.5 8 8.5" />
        <path d="M8 8.5h5.5" />
        <circle cx="8" cy="8.5" r="1.9" />
        <path d="M4.6 5.4a4.6 4.6 0 0 1 6.3-.5" />
        <path d="m10.2 3.6.9 1.4-1.6.4" />
      </>
    ),
    slider: (
      <>
        <path d="M1.8 11.5h12.4" />
        <rect x="5" y="5.5" width="5.5" height="4" rx="1" />
        <path d="M12 5.5l1.6 1.8L12 9" />
      </>
    ),
    cylinder: (
      <>
        <path d="M1.8 8h12.4" />
        <ellipse cx="7.5" cy="8" rx="2.4" ry="4" />
        <path d="M12.3 5.6l1.8 2.4-1.8 2.4" />
      </>
    ),
    ball: (
      <>
        <circle cx="8" cy="7" r="3" />
        <path d="M3.3 9.5a5 5 0 0 0 9.4 0" />
        <path d="M8 13.5v1.4" />
      </>
    ),
  };
  return (
    <svg className="joint-glyph" viewBox="0 0 16 16" aria-hidden="true">
      {paths[kind]}
    </svg>
  );
}

const GEOMETRY_WORDS: Record<string, string> = { face: "face", edge: "edge", point: "point", line: "axis", plane: "plane", cylinder: "round face", sphere: "sphere", origin: "origin" };

function endText(end: End, entities: ReadonlyMap<string, EntityView>) {
  if (end === "ground") return "Ground";
  if ("instance" in end) return `${instance(entities.get(end.instance))?.name ?? "Missing"} · ${"connector" in end ? end.connector : (GEOMETRY_WORDS[end.geometry.type] ?? end.geometry.type)}`;
  return `Ground at ${end.at.map((v) => fmt(v)).join(", ")}`;
}

type Connector = { name: string; at: V3; axis: V3; x: V3 };

/** Instances in a scope (an assembly, or the top of the project). */
function scopeInstances(scope: string | null, entities: ReadonlyMap<string, EntityView>) {
  if (scope) return childrenOf(scope, entities);
  return [...entities.values()].filter((e) => instance(e) && !instance(e)!.parent).sort((a, b) => instance(a)!.name.localeCompare(instance(b)!.name, undefined, { numeric: true }));
}

type Frame = { value: number; poses: [V3, [number, number, number, number]][]; joints: Record<string, number[]> };
type Gap = { a: string; b: string; a_id: string; b_id: string; gap: number; at: number; range?: [number, number]; joined: boolean };
export type AssemblySweep = {
  joint: string;
  name: string;
  kind: JointKind;
  key: string;
  unit: string;
  from: number;
  to: number;
  instances: { id: string; name: string; moves: boolean }[];
  frames: Frame[];
  binds_at: number | null;
  pairs: Gap[];
  clashes: Gap[];
  search: number;
  notes: string[];
  /** Shapes that change as it moves (a spring the joint stretches), with their values at each frame. */
  flex?: FlexShape[];
  /** The values the interlocks allow here, when they leave some of the range out. */
  allowed?: [number, number][];
};

async function fetchSweep(joint: string, opts: { key?: string; from?: number; to?: number; steps: number; gaps: boolean }): Promise<AssemblySweep> {
  const q = new URLSearchParams({ steps: String(opts.steps) });
  if (opts.key) q.set("key", opts.key);
  if (opts.from !== undefined) q.set("from", String(opts.from));
  if (opts.to !== undefined) q.set("to", String(opts.to));
  if (!opts.gaps) q.set("gaps", "off");
  const r = await fetch(projectUrl(`/assembly-motion/${joint}?${q}`));
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

/**
 * The fractional frame where a sweep reaches value `v`. Its frames run from
 * `from` to `to`, evenly unless interlocks left some values out.
 */
export function frameAt(s: { frames: { value: number }[] }, v: number): number {
  const f = s.frames;
  if (f.length < 2) return 0;
  const up = f[f.length - 1].value >= f[0].value;
  const before = (a: number, b: number) => (up ? a <= b : a >= b);
  if (before(v, f[0].value)) return 0;
  for (let i = 1; i < f.length; i++) {
    if (before(v, f[i].value)) {
      const [a, b] = [f[i - 1].value, f[i].value];
      return i - 1 + (b !== a ? (v - a) / (b - a) : 0);
    }
  }
  return f.length - 1;
}

/** A sweep's value at fractional frame `k`. */
export function valueAt(s: { frames: { value: number }[] }, k: number): number {
  const f = s.frames;
  if (!f.length) return 0;
  const i = Math.max(0, Math.min(f.length - 1, Math.floor(k)));
  const j = Math.min(f.length - 1, i + 1);
  return f[i].value + (f[j].value - f[i].value) * Math.max(0, Math.min(1, k - i));
}

/** The placements of a sweep at fractional frame `t` (see `frameAt` for a value's). */
function posesAtIndex(s: AssemblySweep, t: number): Map<string, Transform> {
  const f = s.frames;
  t = Math.max(0, Math.min(f.length - 1, t));
  const i = Math.max(0, Math.min(f.length - 2, Math.floor(t)));
  const k = Math.max(0, Math.min(1, t - i));
  const out = new Map<string, Transform>();
  s.instances.forEach((inst, j) => {
    const [a, b] = [f[i].poses[j], f[Math.min(i + 1, f.length - 1)].poses[j]];
    const at = a[0].map((c, m) => c + (b[0][m] - c) * k) as V3;
    let q = a[1].map((c, m) => c + (b[1][m] * Math.sign(a[1].reduce((d, x, w) => d + x * b[1][w], 0) || 1) - c) * k);
    const len = Math.hypot(...q);
    q = q.map((c) => c / len);
    out.set(inst.id, { at, q: q as [number, number, number, number] });
  });
  return out;
}

// ---- The panel ------------------------------------------------------------------

export function AssemblyJoints(props: {
  /** The assembly whose instances are joined, or null for the top of the project. */
  scope: string | null;
  /** Only the joints that touch this instance (an instance's own panel). */
  focus?: string;
  /** The scope's coordinates in the project (where its instances are drawn). */
  frame: Transform;
  title: string;
  entities: ReadonlyMap<string, EntityView>;
  run: Run;
  view: JointView;
}) {
  const { scope, focus, entities, run, view } = props;
  const joints = useMemo(
    () =>
      [...entities.values()]
        .filter((e) => {
          const j = jointOf(e);
          if (!j || (j.assembly ?? null) !== scope) return false;
          if (!focus) return true;
          return [j.a, j.b].some((end) => typeof end === "object" && "instance" in end && end.instance === focus);
        })
        .sort((a, b) => (a.id < b.id ? -1 : 1)),
    [entities, scope, focus],
  );
  const [open, setOpen] = useState<string | null>(null);
  const [builder, setBuilder] = useState<Builder | null>(null);
  const [mating, setMating] = useState(false);
  // Joints a coupling drives: which coupling.
  const follows = useMemo(() => {
    const m = new Map<string, string>();
    for (const e of entities.values()) {
      const c = couplingOf(e);
      if (c && (c.assembly ?? null) === scope) m.set(c.follower, c.name);
    }
    return m;
  }, [entities, scope]);
  // The degrees-of-freedom check re-reads when any joint here changes.
  const allJoints = useMemo(() => [...entities.values()].filter((e) => (jointOf(e)?.assembly ?? null) === scope), [entities, scope]);
  const stamp = useMemo(() => allJoints.map((e) => `${e.id}:${JSON.stringify(e.components)}:${JSON.stringify(e.params ?? {})}`).join("|") + `|${[...follows.values()].join()}`, [allJoints, follows]);
  const [check, setCheck] = useState<{ joint: string; state: "busy" } | { joint: string; state: "done"; sweep: AssemblySweep } | { joint: string; state: "error"; text: string } | null>(null);
  const viewRef = useRef(view);
  viewRef.current = view;
  // Leaving the panel puts the view back.
  useEffect(() => () => {
    viewRef.current.pick(null);
    viewRef.current.poses(null);
  }, []);
  useEffect(() => {
    setCheck(null);
    viewRef.current.poses(null);
  }, [scope, focus]);

  const members = scopeInstances(scope, entities);
  if (!joints.length && !builder && focus) return null;
  return (
    <section className="props-section aj">
      <header>
        <h3>{props.title}</h3>
        {!builder && !mating && members.length > 1 ? (
          <span className="aj-adds">
            <button type="button" className="aj-add" onClick={() => setMating(true)} title="Pick a face on each of two parts: coincident, concentric, parallel, distance, tangent…">
              <Icon name="plus" /> Mate
            </button>
            <button type="button" className="aj-add" onClick={() => setBuilder({ step: "moving" })} title="Join two instances at their connectors">
              <Icon name="plus" /> Joint
            </button>
          </span>
        ) : (
          joints.length > 0 && <span className="section-note">{joints.length}</span>
        )}
      </header>
      {mating && (
        <MateBuilder
          scope={scope}
          entities={entities}
          view={view}
          onCancel={() => setMating(false)}
          onCreate={async (params) => {
            const out = (await run("assembly.joint.create", params)) as { id: string } | null;
            if (out) {
              setMating(false);
              setOpen(out.id);
            }
          }}
        />
      )}
      {builder && (
        <JointBuilder
          builder={builder}
          setBuilder={setBuilder}
          scope={scope}
          frame={props.frame}
          members={members}
          entities={entities}
          view={view}
          onCreate={async (params) => {
            const out = (await run("assembly.joint.create", params)) as { id: string } | null;
            if (out) {
              setBuilder(null);
              setOpen(out.id);
            }
          }}
        />
      )}
      {!builder && !mating && !joints.length && (
        <p className="aj-empty">
          {members.length > 1
            ? "Mate faces (coincident, concentric, distance, tangent…) or join instances at their connectors — a servo on its bracket, a horn on its shaft. Each value is a parameter you can scrub, limit and link."
            : "Place two or more components here to join them."}
        </p>
      )}
      {!focus && allJoints.length > 0 && <DofSummary scope={scope} stamp={stamp} />}
      {joints.length > 0 && (
        <ul className="aj-list">
          {joints.map((e) => (
            <JointRow
              key={e.id}
              joint={e}
              follows={follows.get(e.id)}
              entities={entities}
              open={open === e.id}
              onToggle={() => setOpen((o) => (o === e.id ? null : e.id))}
              run={run}
              view={view}
              checking={check?.joint === e.id ? check : null}
              onCheck={async () => {
                setCheck({ joint: e.id, state: "busy" });
                try {
                  const sweep = await fetchSweep(e.id, { steps: 180, gaps: true });
                  setCheck({ joint: e.id, state: "done", sweep });
                } catch (err) {
                  setCheck({ joint: e.id, state: "error", text: (err as Error).message });
                }
              }}
              onCloseCheck={() => {
                setCheck(null);
                view.poses(null);
              }}
            />
          ))}
        </ul>
      )}
      {!focus && <Couplings scope={scope} joints={allJoints} entities={entities} run={run} />}
      {!focus && <Interlocks scope={scope} joints={allJoints} entities={entities} run={run} />}
    </section>
  );
}

// ---- One joint --------------------------------------------------------------------

function JointRow(props: {
  joint: EntityView;
  /** The coupling that drives its value, if one does. */
  follows?: string;
  entities: ReadonlyMap<string, EntityView>;
  open: boolean;
  onToggle: () => void;
  run: Run;
  view: JointView;
  checking: { state: "busy" } | { state: "done"; sweep: AssemblySweep } | { state: "error"; text: string } | null;
  onCheck: () => void;
  onCloseCheck: () => void;
}) {
  const { joint: e, entities, run, view } = props;
  const j = jointOf(e)!;
  const set = paramsOf(e);
  const name = set.name;
  const failed = e.status?.state === "failed" || e.status?.state === "blocked" ? (e.status as { reason?: string }).reason : null;
  // What its group's solve says: a held instance's status, and a loop's free values.
  const heldIds = [j.a, j.b].flatMap((end) => (typeof end === "object" && "instance" in end ? [end.instance] : []));
  const heldProblem = heldIds.map((id) => entities.get(id)).find((x) => x?.status?.state === "failed");
  const solved = heldIds.map((id) => entities.get(id)?.placed?.values?.[e.id]).find((v) => v);
  const following = j.free || !!props.follows;
  const value = (p: Param, k: number) => (following && solved ? solved[k] : (e.params?.[p.key] ?? p.value));
  const mate = isMate(j.kind);
  const planes = j.kind === "coincident" || j.kind === "distance" || j.kind === "parallel";
  // Scrubbing: the motion through the value's range, fetched once, played in the view.
  const sweeps = useRef(new Map<string, Promise<AssemblySweep | null>>());
  // Where interlocks hold a value here (with the other joints where they are): the slider stops there.
  const [held, setHeld] = useState<Record<string, [number, number]>>({});
  useEffect(() => {
    sweeps.current.clear();
    setHeld({});
  }, [entities]);
  const sweepFor = (p: Param) => {
    let s = sweeps.current.get(p.key);
    if (!s) {
      const v = e.params?.[p.key] ?? p.value;
      const turning = p.unit === "deg";
      const [from, to] = p.min != null && p.max != null ? [p.min, p.max] : turning ? [v - 180, v + 180] : [v - 200, v + 200];
      s = fetchSweep(e.id, { key: p.key, from, to, steps: turning ? 240 : 200, gaps: false }).catch(() => null);
      sweeps.current.set(p.key, s);
      void s.then((r) => {
        const piece = r?.allowed?.length ? (r.allowed.find(([a, b]) => v >= a - 1e-9 && v <= b + 1e-9) ?? r.allowed[0]) : null;
        if (piece) setHeld((h) => ({ ...h, [p.key]: piece }));
      });
    }
    return s;
  };
  const live = async (p: Param, v: number | null) => {
    if (v === null) return view.poses(null);
    const s = await sweepFor(p);
    if (s && s.frames.length > 1) {
      const k = frameAt(s, v);
      view.poses(posesAtIndex(s, k));
      // A spring the joint stretches stretches as it is scrubbed.
      if (s.flex?.length) void view.flex({ shapes: s.flex, frame: k });
    }
  };
  const primary = set.params[0];
  const ends = `${endText(j.b, entities)}  →  ${endText(j.a, entities)}`;
  const limits = (p: Param) => (p.min != null && p.max != null ? `${n(p.min, 1)}…${n(p.max, 1)}${unitOf(p)}` : null);
  return (
    <li className={`aj-joint ${props.open ? "open" : ""} ${failed || heldProblem ? "bad" : ""}`}>
      <button title={props.open ? `Close ${name}` : `Open ${name}: its value, limits and motion check`} type="button" className="aj-head" onClick={props.onToggle} aria-expanded={props.open} onPointerEnter={() => primary && !j.free && sweepFor(primary)}>
        <span className={`aj-kind k-${j.kind}`}>
          <JointGlyph kind={j.kind} />
        </span>
        <span className="aj-title">
          <strong>{name}</strong>
          <small title={ends}>{ends}</small>
        </span>
        <span className="aj-value">
          {primary ? (
            <>
              {n(value(primary, 0), primary.unit === "mm" ? 1 : 1)}
              <small>{unitOf(primary)}</small>
              {e.formulas?.[primary.key] && <Icon name="link" />}
            </>
          ) : (
            <small>{mate ? j.kind : j.offset ? `${fmt(j.offset)} mm gap` : "fixed"}</small>
          )}
        </span>
        <Icon name="chevron" />
      </button>
      {(failed || heldProblem) && <p className="aj-problem">{failed ?? (heldProblem?.status as { reason?: string })?.reason}</p>}
      {props.open && (
        <div className="aj-body">
          {set.params.length > 0 && (
            <div className="param-list">
              {set.params.map((p, k) =>
                following ? (
                  <div key={p.key} className="param-row">
                    <span className="param-label">{p.label}</span>
                    <span className="aj-follows" title={props.follows ? "A coupling drives this value from another joint's" : "A free joint in a closed loop takes the value the other joints leave it"}>
                      {n(value(p, k), 2)}
                      <small>{unitOf(p)}</small>
                      <em>{props.follows ? `follows ${props.follows}` : "follows the loop"}</em>
                    </span>
                  </div>
                ) : (
                  <div key={p.key} className={`param-row ${e.formulas?.[p.key] ? "linked" : ""}`}>
                    <span className="param-label">
                      {p.label}
                      {limits(p) && <small className="aj-limits-note">{limits(p)}</small>}
                      {held[p.key] && (
                        <small className="aj-limits-note aj-interlocked" title="An interlock holds it to this range here, with the other joints where they are">
                          interlock {n(held[p.key][0], 1)}…{n(held[p.key][1], 1)}
                          {unitOf(p)}
                        </small>
                      )}
                      {e.formulas?.[p.key] && <LinkNote formula={e.formulas[p.key]} onUnlink={() => run("shape.link_param", { target: e.id, key: p.key, expr: null })} />}
                    </span>
                    <ScrubField
                      label={`${name} ${p.label.toLowerCase()}`}
                      chip={p.unit === "deg" ? "∠" : "↕"}
                      tone="#7352d6"
                      value={value(p, k)}
                      unit={unitOf(p)}
                      min={held[p.key]?.[0] ?? p.min ?? undefined}
                      max={held[p.key]?.[1] ?? p.max ?? undefined}
                      step={p.unit === "deg" ? 1 : 0.5}
                      formula={e.formulas?.[p.key]}
                      onFormula={(f) => run("shape.link_param", { target: e.id, key: p.key, expr: f })}
                      onLive={(v) => void live(p, v)}
                      onCommit={(v) => {
                        void run("parameters.set", { target: e.id, values: { [p.key]: v } }).then(() => view.poses(null));
                      }}
                    />
                  </div>
                ),
              )}
            </div>
          )}
          {primary && !following && <Limits param={primary} onSet={(l) => run("assembly.joint.update", { target: e.id, limits: l })} />}
          {mate ? (
            planes && (
              <div className="aj-options">
                <div className="seg-toggle" role="group" aria-label="How flat faces face">
                  <button type="button" className={!j.aligned ? "on" : ""} aria-pressed={!j.aligned} onClick={() => j.aligned && run("assembly.joint.update", { target: e.id, aligned: false })} title="Flat faces face each other">
                    Face to face
                  </button>
                  <button type="button" className={j.aligned ? "on" : ""} aria-pressed={!!j.aligned} onClick={() => !j.aligned && run("assembly.joint.update", { target: e.id, aligned: true })} title="Flat faces point the same way">
                    Same way
                  </button>
                </div>
                {j.kind === "coincident" && <ScrubField label="Gap" chip="gap" value={j.offset ?? 0} unit="mm" step={0.5} onLive={() => {}} onCommit={(v) => run("assembly.joint.update", { target: e.id, offset: v })} />}
              </div>
            )
          ) : (
          <div className="aj-options">
            <div className="seg-toggle" role="group" aria-label="How the connectors face">
              <button type="button" className={!j.aligned ? "on" : ""} aria-pressed={!j.aligned} onClick={() => j.aligned && run("assembly.joint.update", { target: e.id, aligned: false })} title="The connectors' axes point at each other: faces meet">
                Face to face
              </button>
              <button type="button" className={j.aligned ? "on" : ""} aria-pressed={!!j.aligned} onClick={() => !j.aligned && run("assembly.joint.update", { target: e.id, aligned: true })} title="The connectors' axes point the same way">
                Same way
              </button>
            </div>
            <ScrubField label="Gap" chip="gap" value={j.offset ?? 0} unit="mm" step={0.5} onLive={() => {}} onCommit={(v) => run("assembly.joint.update", { target: e.id, offset: v })} />
          </div>
          )}
          {set.params.length > 0 && !props.follows && (
            <label className="aj-free" title="In a closed loop (a four-bar, a part held at two places) a free joint takes whatever value the others leave it">
              <input type="checkbox" checked={!!j.free} onChange={(ev) => run("assembly.joint.update", { target: e.id, free: ev.target.checked })} />
              Free in a loop
            </label>
          )}
          <div className="row-buttons aj-actions">
            {set.params.length > 0 && !props.follows && (
              <button type="button" className="secondary" onClick={props.onCheck} disabled={props.checking?.state === "busy"} title="Move it through its range and watch every pair of parts">
                <Icon name="collide" /> Check clashes
              </button>
            )}
            <button type="button" className="secondary aj-delete" onClick={() => run("assembly.joint.delete", { target: e.id })} title="Delete the joint; what it held stays where it is">
              <Icon name="trash" /> Delete
            </button>
          </div>
          {props.checking && <SweepResult joint={e} check={props.checking} entities={entities} run={run} view={view} onClose={props.onCloseCheck} />}
        </div>
      )}
    </li>
  );
}

function Limits({ param, onSet }: { param: Param; onSet: (l: [number, number] | null) => void }) {
  const has = param.min != null && param.max != null;
  const [lo, setLo] = useState(has ? String(param.min) : "");
  const [hi, setHi] = useState(has ? String(param.max) : "");
  useEffect(() => {
    setLo(param.min != null ? String(param.min) : "");
    setHi(param.max != null ? String(param.max) : "");
  }, [param.min, param.max]);
  const apply = () => {
    const [a, b] = [Number(lo), Number(hi)];
    if (lo.trim() === "" && hi.trim() === "") return has && onSet(null);
    if (Number.isFinite(a) && Number.isFinite(b) && a < b && (a !== param.min || b !== param.max)) onSet([a, b]);
  };
  const u = unitOf(param);
  return (
    <div className="aj-limits">
      <span>Limits</span>
      <input aria-label="Lowest" value={lo} placeholder="none" inputMode="decimal" onChange={(e) => setLo(e.target.value)} onBlur={apply} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
      <em>to</em>
      <input aria-label="Highest" value={hi} placeholder="none" inputMode="decimal" onChange={(e) => setHi(e.target.value)} onBlur={apply} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
      <small>{u}</small>
      {has && (
        <button type="button" className="aj-clear" onClick={() => onSet(null)} title="No limits">
          <Icon name="close" />
        </button>
      )}
    </div>
  );
}

// ---- Clash sweep ----------------------------------------------------------------------

function SweepResult(props: {
  joint: EntityView;
  check: { state: "busy" } | { state: "done"; sweep: AssemblySweep } | { state: "error"; text: string };
  entities: ReadonlyMap<string, EntityView>;
  run: Run;
  view: JointView;
  onClose: () => void;
}) {
  const { check, view } = props;
  const [at, setAt] = useState<number | null>(null);
  const [playing, setPlaying] = useState(false);
  const sweep = check.state === "done" ? check.sweep : null;
  const last = sweep ? sweep.frames.length - 1 : 0;
  useEffect(() => {
    if (!playing || !sweep) return;
    let raf = 0;
    let t0: number | null = null;
    const startAt = at ?? 0;
    const tick = (t: number) => {
      t0 ??= t;
      const k = (startAt + ((t - t0) / 4000) * last) % (last + 1);
      setAt(k);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, sweep]);
  useEffect(() => {
    if (!sweep || at === null) return;
    view.poses(posesAtIndex(sweep, at));
    // Shapes the motion changes (a spring the joint stretches) change with it.
    if (sweep.flex?.length) void view.flex({ shapes: sweep.flex, frame: at });
  }, [at, sweep, last, view]);
  const requirement = [...props.entities.values()].find((e) => {
    const r = e.components["intent.requirement"] as { rule?: { kind: string; joint?: string } } | undefined;
    return r?.rule?.kind === "no_clashes_in_motion" && r.rule.joint === props.joint.id;
  });
  if (check.state === "busy") return <p className="motion-note aj-busy">Moving it through its range and watching every pair…</p>;
  if (check.state === "error") return <p className="print-note bad">{check.text}</p>;
  const s = check.sweep;
  const u = s.unit === "°" ? "°" : " mm";
  const pct = (v: number) => ((v - s.from) / (s.to - s.from)) * 100;
  const value = valueAt(s, at ?? 0);
  const jump = (v: number) => {
    setPlaying(false);
    setAt(frameAt(s, v));
  };
  const closest = s.pairs.find((p) => !p.joined);
  return (
    <div className="aj-sweep">
      <div className={`gear-verdict ${s.clashes.length || s.binds_at !== null ? "bad" : "ok"}`}>
        {s.clashes.length > 0 ? (
          <>
            {s.clashes.map((c) => (
              <button title={`Jump to where ${c.a} first hits ${c.b}`} key={`${c.a_id}${c.b_id}`} type="button" className="clash-jump" onClick={() => jump(c.range?.[0] ?? c.at)}>
                {c.a} hits {c.b} from {n(c.range?.[0] ?? c.at)}{u} to {n(c.range?.[1] ?? c.at)}
                {u}
              </button>
            ))}
          </>
        ) : s.instances.every((i) => !i.moves) ? (
          <span>Nothing hangs from {s.name}: no instance moves.</span>
        ) : (
          <span>
            Clear from {n(s.from)}
            {u} to {n(s.to)}
            {u}
            {closest ? ` · closest ${n(closest.gap, 2)} mm (${closest.a} – ${closest.b} at ${n(closest.at)}${u})` : ` · nothing within ${n(s.search)} mm`}
          </span>
        )}
        {s.binds_at !== null && <span>Binds at {n(s.binds_at)}{u}</span>}
      </div>
      <div className="aj-track" aria-hidden="true">
        {s.clashes.map((c) => {
          const [a, b] = c.range ?? [c.at, c.at];
          return <span key={`${c.a_id}${c.b_id}`} className="aj-clash" style={{ left: `${pct(a)}%`, width: `${Math.max(0.8, pct(b) - pct(a))}%` }} title={`${c.a} – ${c.b}`} />;
        })}
        {(s.allowed ?? []).length > 0 &&
          // What the interlocks leave out of the range: shaded.
          [s.from, ...(s.allowed ?? []).flat(), s.to]
            .reduce<[number, number][]>((gaps, v, i, all) => (i % 2 === 0 && i + 1 < all.length && Math.abs(all[i + 1] - v) > 1e-9 ? [...gaps, [Math.min(v, all[i + 1]), Math.max(v, all[i + 1])]] : gaps), [])
            .map(([a, b]) => <span key={`il${a}`} className="aj-interlocked-span" style={{ left: `${Math.min(pct(a), pct(b))}%`, width: `${Math.abs(pct(b) - pct(a))}%` }} title="Left out by an interlock" />)}
        {at !== null && <span className="aj-head-mark" style={{ left: `${pct(value)}%` }} />}
      </div>
      <div className="gear-turn-row">
        <button type="button" className="gear-play" aria-label={playing ? "Pause" : "Play the motion"} onClick={() => setPlaying((p) => !p)}>
          <Icon name={playing ? "pause" : "play"} />
        </button>
        <input type="range" min={0} max={last} step={0.01} value={at ?? 0} aria-label={`${s.name} through its range`} onChange={(e) => (setPlaying(false), setAt(Number(e.target.value)))} />
        <output className="gear-turn-angle">
          {n(value)}
          {u}
        </output>
      </div>
      {s.pairs.length > 0 && (
        <ul className="aj-pairs">
          {s.pairs.slice(0, 6).map((p) => (
            <li key={`${p.a_id}${p.b_id}`}>
              <button title={`Jump to where ${p.a} and ${p.b} come closest`} type="button" onClick={() => jump(p.range?.[0] ?? p.at)}>
                <span className={`aj-dot ${p.gap < 1e-4 ? (p.joined ? "joined" : "hit") : "near"}`} />
                <span>
                  {p.a} – {p.b}
                  {p.joined && <em>joined</em>}
                </span>
                <small>
                  {p.gap < 1e-4 ? "touch" : `${n(p.gap, 2)} mm`} at {n(p.at)}
                  {u}
                </small>
              </button>
            </li>
          ))}
        </ul>
      )}
      {s.notes.map((t) => (
        <p key={t} className="motion-note">
          {t}
        </p>
      ))}
      <div className="row-buttons">
        {requirement ? (
          <span className="aj-kept">
            <Icon name="check" /> Checked on every change
          </span>
        ) : (
          <button type="button" className="secondary" onClick={() => props.run("intent.requirement.create", { rule: { kind: "no_clashes_in_motion", joint: props.joint.id, steps: 72 } })} title="Re-check this motion whenever anything in the assembly changes">
            <Icon name="shield" /> Keep as a requirement
          </button>
        )}
        <button title="Close the motion check and put the parts back" type="button" className="secondary" onClick={props.onClose}>
          Done
        </button>
      </div>
    </div>
  );
}

// ---- Adding a joint ---------------------------------------------------------------------

type Picked = { instance: string; connector: string; label: string };
type Builder = { step: "moving" } | { step: "onto"; moving: Picked } | { step: "form"; moving: Picked; onto: Picked | "ground"; kind: JointKind; name: string; value: string };

function JointBuilder(props: {
  builder: Builder;
  setBuilder: (b: Builder | null) => void;
  scope: string | null;
  frame: Transform;
  members: EntityView[];
  entities: ReadonlyMap<string, EntityView>;
  view: JointView;
  onCreate: (params: Record<string, unknown>) => Promise<void>;
}) {
  const { builder, setBuilder, members, view, frame } = props;
  const [connectors, setConnectors] = useState<Map<string, Connector[] | string> | null>(null);
  const memberKey = members.map((m) => `${m.id}:${m.placed?.at.join(",")}:${m.placed?.quaternion.join(",")}`).join("|");
  useEffect(() => {
    let live = true;
    Promise.all(
      members.map(async (m) => {
        const r = await fetch(projectUrl(`/connectors/${m.id}`));
        return [m.id, r.ok ? ((await r.json()) as Connector[]) : await r.text()] as const;
      }),
    ).then((list) => live && setConnectors(new Map(list)));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [memberKey]);
  // Esc steps back, then closes.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      setBuilder(builder.step === "onto" ? { step: "moving" } : builder.step === "form" ? { step: "onto", moving: builder.moving } : null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [builder, setBuilder]);
  // The triads in the view.
  const name = (id: string) => instance(props.entities.get(id))?.name ?? "?";
  useEffect(() => {
    if (!connectors || builder.step === "form") {
      view.pick(null);
      return;
    }
    const marks: ConnectorMark[] = [];
    for (const [id, list] of connectors) {
      if (typeof list === "string") continue;
      for (const c of list) {
        const key = `${id}::${c.name}`;
        const chosen = builder.step === "onto" && builder.moving.instance === id && builder.moving.connector === c.name;
        const same = builder.step === "onto" && builder.moving.instance === id;
        marks.push({
          key,
          label: `${name(id)} › ${c.name}`,
          hint: builder.step === "moving" ? "Click: this part moves to meet the next one" : "Click: it goes here",
          at: applyT(frame, c.at),
          axis: qrotate(frame.q, c.axis),
          x: qrotate(frame.q, c.x),
          state: chosen ? "chosen" : same ? "dim" : "normal",
        });
      }
    }
    view.pick({
      picture: { marks },
      onPick: (key) => {
        const [instanceId, connector] = key.split("::");
        const picked = { instance: instanceId, connector, label: `${name(instanceId)} › ${connector}` };
        if (builder.step === "moving") setBuilder({ step: "onto", moving: picked });
        else if (builder.step === "onto") setBuilder({ step: "form", moving: builder.moving, onto: picked, kind: "fixed", name: "", value: "0" });
      },
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectors, builder, frame]);
  useEffect(() => () => view.pick(null), [view]);

  const none = connectors && [...connectors.values()].every((l) => typeof l === "string" || l.length === 0);
  const without = connectors ? members.filter((m) => {
    const l = connectors.get(m.id);
    return Array.isArray(l) && l.length === 0;
  }) : [];
  const steps = ["Part that moves", "Where it goes", "Joint"];
  const current = builder.step === "moving" ? 0 : builder.step === "onto" ? 1 : 2;
  return (
    <div className="aj-builder">
      <ol className="aj-steps">
        {steps.map((s, i) => (
          <li key={s} className={i === current ? "now" : i < current ? "done" : ""}>
            <span>{i < current ? <Icon name="check" /> : i + 1}</span>
            {s}
          </li>
        ))}
      </ol>
      {!connectors && <p className="motion-note">Finding connectors…</p>}
      {none && <p className="print-note bad">None of these components declares a connector yet. Add connector(name, {"{ at, axis }"}) to their programs: a shaft's end, a hole, a mounting face.</p>}
      {connectors && !none && builder.step === "moving" && (
        <p className="aj-prompt">
          <strong>Pick a connector in the view</strong> on the part that moves — the servo's mounting face, the horn's bore.
        </p>
      )}
      {builder.step === "onto" && (
        <>
          <p className="aj-prompt">
            <span className="aj-chip">{builder.moving.label}</span> moves. <strong>Now pick where it goes</strong>, or hold it to the assembly's origin.
          </p>
          <button title="Fix the moving part to the assembly's origin instead of another part" type="button" className="secondary aj-ground" onClick={() => setBuilder({ step: "form", moving: builder.moving, onto: "ground", kind: "fixed", name: "", value: "0" })}>
            Hold it to the ground
          </button>
        </>
      )}
      {builder.step === "form" && (
        <div className="aj-form">
          <p className="aj-prompt">
            <span className="aj-chip">{builder.moving.label}</span>
            <span className="aj-arrow">→</span>
            <span className="aj-chip">{builder.onto === "ground" ? "Ground" : builder.onto.label}</span>
          </p>
          <div className="aj-kinds" role="radiogroup" aria-label="Kind of joint">
            {KINDS.map((k) => (
              <button key={k.kind} type="button" role="radio" aria-checked={builder.kind === k.kind} className={builder.kind === k.kind ? "on" : ""} title={k.hint} onClick={() => setBuilder({ ...builder, kind: k.kind })}>
                <JointGlyph kind={k.kind} />
                {k.label}
              </button>
            ))}
          </div>
          <p className="aj-kind-hint">{KINDS.find((k) => k.kind === builder.kind)?.hint}</p>
          <div className="aj-fields">
            <label>
              <span>Name</span>
              <input value={builder.name} placeholder={`${KINDS.find((k) => k.kind === builder.kind)!.label} …`} onChange={(e) => setBuilder({ ...builder, name: e.target.value })} />
            </label>
            {builder.kind !== "fixed" && builder.kind !== "ball" && (
              <label>
                <span>{builder.kind === "slider" ? "Distance (mm)" : "Angle (°)"}</span>
                <input value={builder.value} inputMode="decimal" onChange={(e) => setBuilder({ ...builder, value: e.target.value })} />
              </label>
            )}
          </div>
          <div className="row-buttons">
            <button title="Make the joint between the two connectors"
              type="button"
              className="primary"
              onClick={() => {
                const v = Number(builder.value);
                void props.onCreate({
                  assembly: props.scope,
                  type: builder.kind,
                  name: builder.name.trim() || undefined,
                  a: builder.onto === "ground" ? "ground" : { instance: builder.onto.instance, connector: builder.onto.connector },
                  b: { instance: builder.moving.instance, connector: builder.moving.connector },
                  ...(builder.kind !== "fixed" && builder.kind !== "ball" && Number.isFinite(v) && v !== 0 ? { value: v } : {}),
                });
              }}
            >
              <Icon name="link" /> Join
            </button>
            <button title="Pick where it goes again (Esc)" type="button" className="secondary" onClick={() => setBuilder({ step: "onto", moving: builder.moving })}>
              Back
            </button>
          </div>
        </div>
      )}
      {without.length > 0 && builder.step !== "form" && (
        <p className="aj-quiet">No connectors on {without.map((m) => instance(m)!.name).join(", ")}.</p>
      )}
      {builder.step !== "form" && (
        <button title="Stop adding the joint (Esc steps back a pick first)" type="button" className="aj-cancel" onClick={() => setBuilder(null)}>
          Cancel · Esc
        </button>
      )}
    </div>
  );
}
