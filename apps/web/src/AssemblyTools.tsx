// Assembly tools beyond joints: mates on picked faces, what can still move
// (degrees of freedom), couplings between joints, patterns of instances,
// named poses, saved exploded views, motion studies (played and exported as
// frames), robot kinematics (reach a point, reach envelope, paths through
// poses) and interference across the assembly.

import { useEffect, useMemo, useRef, useState } from "react";
import { type EntityView, projectUrl } from "./engine";
import { Icon } from "./icons";
import type { JointView } from "./Joints";
import { type Quat, type Transform, type V3, applyT, childrenOf, fmt, instance, invertT, qrotate, worldOf } from "./model";
import type { GeometryHit } from "./Viewport";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;

async function getJson<T>(path: string): Promise<T> {
  const r = await fetch(projectUrl(path));
  if (!r.ok) throw new Error(await r.text());
  return r.json() as Promise<T>;
}

const n1 = (v: number, d = 1) => Number(v.toFixed(d)).toLocaleString("en-US");
const nameOf = (id: string, entities: ReadonlyMap<string, EntityView>) => instance(entities.get(id))?.name ?? "?";
const paramsOf = (e: EntityView) => (e.components["shape.parameter_set"] as { name: string; params: { key: string; value: number; unit: string; expr?: string }[] } | undefined) ?? { name: "?", params: [] };

/** Instances in a scope (an assembly, or the top of the project). */
function membersOf(scope: string | null, entities: ReadonlyMap<string, EntityView>) {
  if (scope) return childrenOf(scope, entities);
  return [...entities.values()].filter((e) => instance(e) && !instance(e)!.parent);
}

// ---- Mates: picking faces in the view ------------------------------------------------

export type MateKind = "coincident" | "concentric" | "parallel" | "perpendicular" | "angle" | "distance" | "tangent" | "lock";
export const MATES: { kind: MateKind; label: string; hint: string; value?: "angle" | "distance" }[] = [
  { kind: "coincident", label: "Coincident", hint: "Faces meet, a point on a face, an axis on a line" },
  { kind: "concentric", label: "Concentric", hint: "Two round faces or edges share one axis (a shaft in a bore)" },
  { kind: "parallel", label: "Parallel", hint: "Two faces or axes point the same way" },
  { kind: "perpendicular", label: "Square", hint: "Two faces or axes at right angles" },
  { kind: "angle", label: "Angle", hint: "Two faces or axes at an angle", value: "angle" },
  { kind: "distance", label: "Distance", hint: "Faces, axes or points a distance apart", value: "distance" },
  { kind: "tangent", label: "Tangent", hint: "A round face touching a flat or round one" },
  { kind: "lock", label: "Lock", hint: "Held exactly where it is now against the other" },
];

type Pick = { instance: string; label: string; world: { point: V3; normal: V3 }; at: V3; normal: V3 };

export function MateBuilder(props: { scope: string | null; entities: ReadonlyMap<string, EntityView>; view: JointView; onCreate: (params: Record<string, unknown>) => Promise<void>; onCancel: () => void }) {
  const { scope, entities, view } = props;
  const [picks, setPicks] = useState<Pick[]>([]);
  const [kind, setKind] = useState<MateKind>("coincident");
  const [value, setValue] = useState("");
  const [aligned, setAligned] = useState(false);
  const [name, setName] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const members = useMemo(() => new Set(membersOf(scope, entities).map((m) => m.id)), [scope, entities]);
  const entitiesRef = useRef(entities);
  entitiesRef.current = entities;
  // Clicks on parts pick faces while the builder is open.
  useEffect(() => {
    if (picks.length >= 2) {
      view.geometry(null);
      return;
    }
    view.geometry((hit: GeometryHit) => {
      const inst = hit.path.find((p) => members.has(p));
      if (!inst) {
        setNote("Pick a part of this assembly.");
        return;
      }
      if (picks.length === 1 && picks[0].instance === inst) {
        setNote("Pick a face on another part: a mate joins two parts.");
        return;
      }
      setNote(null);
      const t = invertT(worldOf(inst, entitiesRef.current));
      const at = applyT(t, hit.point);
      const normal = qrotate(t.q, hit.normal);
      setPicks((p) => [...p, { instance: inst, label: nameOf(inst, entitiesRef.current), world: { point: hit.point, normal: hit.normal }, at, normal }]);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [picks, members]);
  // The picks drawn in the view: a dot and a short line along the face's normal.
  useEffect(() => {
    view.overlay(
      picks.length
        ? {
            points: [{ at: picks.map((p) => p.world.point), color: "#6d4ae0", size: 9 }],
            lines: picks.map((p) => ({ points: [p.world.point, p.world.point.map((c, k) => c + p.world.normal[k] * 12) as V3], color: "#6d4ae0" })),
          }
        : null,
    );
  }, [picks, view]);
  useEffect(
    () => () => {
      view.geometry(null);
      view.overlay(null);
    },
    [view],
  );
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      if (picks.length) setPicks((p) => p.slice(0, -1));
      else props.onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [picks, props]);
  const spec = MATES.find((m) => m.kind === kind)!;
  const round = (v: V3) => v.map((c) => Math.round(c * 1e4) / 1e4) as V3;
  const end = (p: Pick) => ({ instance: p.instance, geometry: { type: kind === "lock" ? "origin" : "face", ...(kind === "lock" ? {} : { at: round(p.at), normal: round(p.normal) }) } });
  const step = picks.length;
  return (
    <div className="aj-builder">
      <ol className="aj-steps">
        {["Stays", "Moves", "Mate"].map((s, i) => (
          <li key={s} className={i === step ? "now" : i < step ? "done" : ""}>
            <span>{i < step ? <Icon name="check" /> : i + 1}</span>
            {s}
          </li>
        ))}
      </ol>
      {step < 2 && (
        <p className="aj-prompt">
          {step === 0 ? (
            <>
              <strong>Click a face in the view</strong> on the part that stays — a flat face, a hole, a shaft.
            </>
          ) : (
            <>
              <span className="aj-chip">{picks[0].label}</span> stays. <strong>Now click a face on the part that moves.</strong>
            </>
          )}
        </p>
      )}
      {note && <p className="aj-quiet">{note}</p>}
      {step === 2 && (
        <div className="aj-form">
          <p className="aj-prompt">
            <span className="aj-chip">{picks[1].label}</span>
            <span className="aj-arrow">→</span>
            <span className="aj-chip">{picks[0].label}</span>
          </p>
          <div className="aj-kinds at-mates" role="radiogroup" aria-label="Kind of mate">
            {MATES.map((m) => (
              <button key={m.kind} type="button" role="radio" aria-checked={kind === m.kind} className={kind === m.kind ? "on" : ""} title={m.hint} onClick={() => setKind(m.kind)}>
                <MateGlyph kind={m.kind} />
                {m.label}
              </button>
            ))}
          </div>
          <p className="aj-kind-hint">{spec.hint}</p>
          <div className="aj-fields">
            <label>
              <span>Name</span>
              <input value={name} placeholder={`${spec.label} …`} onChange={(e) => setName(e.target.value)} />
            </label>
            {spec.value && (
              <label>
                <span>{spec.value === "angle" ? "Angle (°)" : "Distance (mm)"}</span>
                <input value={value} placeholder="as it is now" inputMode="decimal" onChange={(e) => setValue(e.target.value)} />
              </label>
            )}
          </div>
          {(kind === "coincident" || kind === "distance" || kind === "parallel") && (
            <label className="aj-free" title="Flat faces: point the same way instead of facing each other">
              <input type="checkbox" checked={aligned} onChange={(e) => setAligned(e.target.checked)} />
              Same way (not face to face)
            </label>
          )}
          <div className="row-buttons">
            <button title="Join the moving part to the one that stays with this mate"
              type="button"
              className="primary"
              onClick={() => {
                const v = Number(value);
                void props.onCreate({
                  assembly: scope,
                  type: kind,
                  name: name.trim() || undefined,
                  a: end(picks[0]),
                  b: end(picks[1]),
                  ...(spec.value && value.trim() !== "" && Number.isFinite(v) ? { value: v } : {}),
                  ...(aligned ? { aligned: true } : {}),
                });
              }}
            >
              <Icon name="link" /> Mate
            </button>
            <button title="Pick the moving part's face again (Esc)" type="button" className="secondary" onClick={() => setPicks((p) => p.slice(0, 1))}>
              Back
            </button>
          </div>
        </div>
      )}
      {step < 2 && (
        <button title="Stop making the mate (Esc steps back a pick first)" type="button" className="aj-cancel" onClick={props.onCancel}>
          Cancel · Esc
        </button>
      )}
    </div>
  );
}

const MATE_ICON: Record<MateKind, Parameters<typeof Icon>[0]["name"]> = {
  coincident: "cCoincident",
  concentric: "cConcentric",
  parallel: "cParallel",
  perpendicular: "cPerpendicular",
  angle: "rotate",
  distance: "ruler",
  tangent: "cTangent",
  lock: "cFix",
};

export function MateGlyph({ kind }: { kind: MateKind }) {
  return (
    <span className="joint-glyph at-glyph">
      <Icon name={MATE_ICON[kind]} />
    </span>
  );
}

export const isMate = (k: string): k is MateKind => k in MATE_ICON;

// ---- Degrees of freedom -----------------------------------------------------------------

type DofInstance = { id: string; name: string; free: number; motions: string[]; mobility: number };
type DofGroup = { joints: { id: string; name: string; kind: string }[]; solved: boolean; error?: string; free: number; mobility: number; instances: DofInstance[]; redundant: number; redundant_joints: { id: string; name: string; count: number }[]; conflicting: { id: string; name: string }[]; summary: string };
export type DofReport = { groups: DofGroup[]; unjoined: { id: string; name: string }[]; free: number; mobility: number; summary: string };

/** What can still move in a scope, re-read whenever its joints change. */
export function DofSummary(props: { scope: string | null; stamp: string; onReport?: (r: DofReport | null) => void }) {
  const [report, setReport] = useState<DofReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const onReport = useRef(props.onReport);
  onReport.current = props.onReport;
  useEffect(() => {
    let live = true;
    const t = window.setTimeout(() => {
      getJson<DofReport>(`/assembly-dof/${props.scope ?? "top"}`)
        .then((r) => {
          if (!live) return;
          setReport(r);
          setError(null);
          onReport.current?.(r);
        })
        .catch((e) => live && setError(String(e.message ?? e)));
    }, 250);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [props.scope, props.stamp]);
  if (error) return <p className="print-note bad">{error}</p>;
  if (!report || !report.groups.length) return null;
  const bad = report.groups.some((g) => !g.solved);
  const free = report.groups.flatMap((g) => g.instances.filter((i) => i.free > 0));
  const redundant = report.groups.flatMap((g) => g.redundant_joints);
  return (
    <div className={`at-dof ${bad ? "bad" : free.length ? "free" : "ok"}`}>
      <button title={open ? "Hide what can still move" : "Show what can still move, part by part"} type="button" className="at-dof-head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <span className="at-dof-dot" />
        <span>
          {bad ? "Cannot all hold" : free.length ? `${report.free} freedom${report.free === 1 ? "" : "s"} left` : "Fully defined"}
          <small>
            {" "}
            · mobility {report.mobility}
            {redundant.length ? ` · ${redundant.length} redundant` : ""}
          </small>
        </span>
        <Icon name="chevron" />
      </button>
      {open && (
        <div className="at-dof-body">
          {report.groups.map((g, k) => (
            <div key={k} className="at-dof-group">
              <p>{g.summary}</p>
              {g.error && <p className="print-note bad">{g.error}</p>}
              {g.conflicting.length > 0 && <p className="print-note bad">Conflicting: {g.conflicting.map((j) => j.name).join(", ")} — any one removed, the rest hold.</p>}
              {g.instances
                .filter((i) => i.free > 0)
                .map((i) => (
                  <div key={i.id} className="at-dof-inst">
                    <strong>{i.name}</strong> · {i.free} free
                    <ul>
                      {i.motions.map((m) => (
                        <li key={m}>{m}</li>
                      ))}
                    </ul>
                  </div>
                ))}
              {g.redundant_joints.length > 0 && <p className="motion-note">Held more than once: {g.redundant_joints.map((r) => `${r.name} (${r.count})`).join(", ")}. Exact parts allow it; printed ones may bind.</p>}
            </div>
          ))}
          {report.unjoined.length > 0 && <p className="motion-note">Joined to nothing (placed by hand): {report.unjoined.map((u) => u.name).join(", ")}</p>}
        </div>
      )}
    </div>
  );
}

// ---- Couplings ------------------------------------------------------------------------------

type CouplingKind = "gear" | "belt" | "rack_pinion" | "screw" | "cam" | "ratio";
type CouplingData = { name: string; kind: CouplingKind; assembly?: string; driver: string; follower: string; ratio: number; offset: number; profile?: [number, number][] };
export const couplingOf = (e: EntityView | undefined) => e?.components["assembly.coupling"] as CouplingData | undefined;
const COUPLINGS: { kind: CouplingKind; label: string; tip: string; fields: { key: string; label: string }[] }[] = [
  { kind: "gear", label: "Gears", tip: "Meshing gears: the follower turns the other way by the teeth ratio", fields: [{ key: "t1", label: "Driver teeth" }, { key: "t2", label: "Follower teeth" }] },
  { kind: "belt", label: "Belt", tip: "Pulleys on a belt: the follower turns the same way by the teeth ratio", fields: [{ key: "t1", label: "Driver teeth" }, { key: "t2", label: "Follower teeth" }] },
  { kind: "rack_pinion", label: "Rack", tip: "Rack and pinion: a turn slides the rack by the pinion's circumference", fields: [{ key: "radius", label: "Pinion radius (mm)" }] },
  { kind: "screw", label: "Screw", tip: "Lead screw: each turn moves the nut by the lead", fields: [{ key: "lead", label: "Lead (mm/turn)" }] },
  { kind: "ratio", label: "Ratio", tip: "Any fixed ratio: the follower moves this much per unit of the driver", fields: [{ key: "ratio", label: "Follower per driver" }] },
];

export function Couplings(props: { scope: string | null; joints: EntityView[]; entities: ReadonlyMap<string, EntityView>; run: Run }) {
  const { scope, joints, entities, run } = props;
  const list = [...entities.values()].filter((e) => couplingOf(e) && (couplingOf(e)!.assembly ?? null) === scope);
  const moving = joints.filter((j) => {
    const k = (j.components["assembly.joint"] as { kind: string }).kind;
    return ["hinge", "slider", "cylinder"].includes(k);
  });
  const [form, setForm] = useState<{ kind: CouplingKind; driver: string; follower: string; v: Record<string, string> } | null>(null);
  if (moving.length < 2 && !list.length) return null;
  const jname = (id: string) => paramsOf(entities.get(id) ?? ({ components: {} } as EntityView)).name;
  const describe = (c: CouplingData) => (c.kind === "cam" ? "cam" : `×${n1(c.ratio, 4)}${c.offset ? ` + ${n1(c.offset, 2)}` : ""}`);
  return (
    <div className="at-couplings">
      <div className="at-sub">
        <span>Couplings</span>
        {!form && moving.length >= 2 && (
          <button type="button" className="aj-add" onClick={() => setForm({ kind: "gear", driver: moving[0].id, follower: moving[1].id, v: { t1: "20", t2: "40" } })} title="Drive one joint from another: gears, belts, racks, screws">
            <Icon name="plus" /> Couple
          </button>
        )}
      </div>
      {list.map((e) => {
        const c = couplingOf(e)!;
        return (
          <div key={e.id} className="at-row">
            <Icon name="link" />
            <span className="at-row-main">
              <strong>{c.name}</strong>
              <small>
                {jname(c.driver)} → {jname(c.follower)} · {describe(c)}
              </small>
            </span>
            <button type="button" className="at-icon" title="Delete the coupling" onClick={() => run("assembly.coupling.delete", { target: e.id })}>
              <Icon name="trash" />
            </button>
          </div>
        );
      })}
      {form && (
        <div className="aj-form at-form">
          <div className="seg-toggle" role="group" aria-label="Kind of coupling">
            {COUPLINGS.map((k) => (
              <button title={k.tip} key={k.kind} type="button" className={form.kind === k.kind ? "on" : ""} onClick={() => setForm({ ...form, kind: k.kind })}>
                {k.label}
              </button>
            ))}
          </div>
          <div className="aj-fields">
            {(["driver", "follower"] as const).map((role) => (
              <label key={role}>
                <span>{role === "driver" ? "Driver" : "Follower"}</span>
                <select title={role === "driver" ? "The joint that drives the other" : "The joint that follows the driver"} value={form[role]} onChange={(e) => setForm({ ...form, [role]: e.target.value })}>
                  {moving.map((j) => (
                    <option key={j.id} value={j.id}>
                      {paramsOf(j).name}
                    </option>
                  ))}
                </select>
              </label>
            ))}
            {COUPLINGS.find((k) => k.kind === form.kind)!.fields.map((f) => (
              <label key={f.key}>
                <span>{f.label}</span>
                <input value={form.v[f.key] ?? ""} inputMode="decimal" onChange={(e) => setForm({ ...form, v: { ...form.v, [f.key]: e.target.value } })} />
              </label>
            ))}
          </div>
          <div className="row-buttons">
            <button title="Make the coupling: the follower moves whenever the driver does"
              type="button"
              className="primary"
              onClick={async () => {
                const num = (k: string) => Number(form.v[k]);
                const extra = form.kind === "gear" || form.kind === "belt" ? { teeth: [num("t1"), num("t2")] } : form.kind === "rack_pinion" ? { radius: num("radius") } : form.kind === "screw" ? { lead: num("lead") } : { ratio: num("ratio") };
                const out = await run("assembly.coupling.create", { assembly: scope, type: form.kind, driver: form.driver, follower: form.follower, ...extra });
                if (out) setForm(null);
              }}
            >
              <Icon name="link" /> Couple
            </button>
            <button title="Close without making a coupling" type="button" className="secondary" onClick={() => setForm(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ---- Patterns ---------------------------------------------------------------------------------

type PatternData = { kind: "linear" | "circular" | "path"; assembly?: string; seed: string; members: string[]; count: number; count2?: number; direction?: V3; axis?: V3; center?: V3 };
const patternOf = (e: EntityView | undefined) => e?.components["assembly.pattern"] as PatternData | undefined;
const AXES: { label: string; v: V3 }[] = [
  { label: "X", v: [1, 0, 0] },
  { label: "Y", v: [0, 1, 0] },
  { label: "Z", v: [0, 0, 1] },
];
const axisLabel = (v?: V3) => AXES.find((a) => v && a.v.every((c, k) => Math.abs(c - v[k]) < 1e-9))?.label ?? (v ? v.map((c) => fmt(c)).join(", ") : "X");

/** An instance's patterns: the copies it is in, those it seeds, and making one. */
export function PatternSection(props: { entity: EntityView; entities: ReadonlyMap<string, EntityView>; run: Run; onSelect: (id: string) => void }) {
  const { entity, entities, run } = props;
  const all = [...entities.values()].filter((e) => patternOf(e));
  const inPattern = all.find((e) => patternOf(e)!.members.includes(entity.id));
  const seeded = all.filter((e) => patternOf(e)!.seed === entity.id);
  const [form, setForm] = useState<{ kind: "linear" | "circular"; count: string; spacing: string; axis: number; span: string } | null>(null);
  if (inPattern) {
    const p = patternOf(inPattern)!;
    return (
      <section className="props-section">
        <header>
          <h3>Pattern</h3>
        </header>
        <p className="held-note">
          <Icon name={p.kind === "circular" ? "patternCircular" : "patternLinear"} />
          <span>
            Copy {p.members.indexOf(entity.id) + 2} of {p.count * (p.count2 ?? 1)} in <strong>{paramsOf(inPattern).name}</strong>. Move or join the seed, and every copy follows.
          </span>
        </p>
        <div className="row-buttons">
          <button title="Select the instance this pattern copies" type="button" className="secondary" onClick={() => props.onSelect(p.seed)}>
            Select the seed
          </button>
        </div>
      </section>
    );
  }
  return (
    <section className="props-section">
      <header>
        <h3>Pattern</h3>
        {!form && (
          <button type="button" className="aj-add" onClick={() => setForm({ kind: "linear", count: "4", spacing: "20", axis: 0, span: "360" })} title="Copies of this instance in a row or round an axis, kept in step">
            <Icon name="plus" /> Pattern
          </button>
        )}
      </header>
      {seeded.map((e) => {
        const p = patternOf(e)!;
        const set = paramsOf(e);
        const spacing = set.params.find((q) => q.key === "spacing" || q.key === "angle");
        return (
          <div key={e.id} className="at-pattern">
            <div className="at-row">
              <Icon name={p.kind === "circular" ? "patternCircular" : "patternLinear"} />
              <span className="at-row-main">
                <strong>{set.name}</strong>
                <small>
                  {p.kind === "circular" ? `round ${axisLabel(p.axis ?? [0, 0, 1])}` : p.kind === "linear" ? `along ${axisLabel(p.direction ?? [1, 0, 0])}` : "along a path"} · {p.count * (p.count2 ?? 1)} in all
                </small>
              </span>
              <button type="button" className="at-icon" title="Delete the pattern and its copies" onClick={() => run("assembly.pattern.delete", { target: e.id })}>
                <Icon name="trash" />
              </button>
            </div>
            <div className="aj-fields">
              <label>
                <span>Count</span>
                <input type="number" min={1} defaultValue={p.count} key={`c${p.count}`} onBlur={(ev) => Number(ev.target.value) !== p.count && Number(ev.target.value) >= 1 && run("assembly.pattern.update", { target: e.id, count: Math.round(Number(ev.target.value)) })} />
              </label>
              {spacing && (
                <label>
                  <span>{spacing.key === "angle" ? "Angle (°)" : "Spacing (mm)"}</span>
                  <input defaultValue={spacing.value} key={`s${spacing.value}`} disabled={!!spacing.expr} title={spacing.expr ? `Linked: ${spacing.expr}` : undefined} inputMode="decimal" onBlur={(ev) => Number(ev.target.value) !== spacing.value && Number.isFinite(Number(ev.target.value)) && run("parameters.set", { target: e.id, values: { [spacing.key]: Number(ev.target.value) } })} />
                </label>
              )}
            </div>
            <button type="button" className="aj-cancel" onClick={() => run("assembly.pattern.delete", { target: e.id, keep: true })} title="Keep the copies where they are, as free instances">
              Free the copies
            </button>
          </div>
        );
      })}
      {form && (
        <div className="aj-form at-form">
          <div className="seg-toggle" role="group" aria-label="Kind of pattern">
            <button title="Copies in a straight line, evenly spaced" type="button" className={form.kind === "linear" ? "on" : ""} onClick={() => setForm({ ...form, kind: "linear" })}>
              <Icon name="patternLinear" /> In a row
            </button>
            <button title="Copies spread round an axis through the assembly's origin" type="button" className={form.kind === "circular" ? "on" : ""} onClick={() => setForm({ ...form, kind: "circular" })}>
              <Icon name="patternCircular" /> Round an axis
            </button>
          </div>
          <div className="aj-fields">
            <label>
              <span>Count</span>
              <input value={form.count} inputMode="numeric" onChange={(e) => setForm({ ...form, count: e.target.value })} />
            </label>
            {form.kind === "linear" ? (
              <label>
                <span>Spacing (mm)</span>
                <input value={form.spacing} inputMode="decimal" onChange={(e) => setForm({ ...form, spacing: e.target.value })} />
              </label>
            ) : (
              <label>
                <span>Spread over (°)</span>
                <input value={form.span} inputMode="decimal" onChange={(e) => setForm({ ...form, span: e.target.value })} />
              </label>
            )}
          </div>
          <div className="seg-toggle" role="group" aria-label={form.kind === "linear" ? "Direction" : "Axis"}>
            <span className="at-seg-label">{form.kind === "linear" ? "Along" : "Round"}</span>
            {AXES.map((a, k) => (
              <button title={form.kind === "linear" ? `Copies along the ${a.label} axis` : `Copies round the ${a.label} axis`} key={a.label} type="button" className={form.axis === k ? "on" : ""} onClick={() => setForm({ ...form, axis: k })}>
                {a.label}
              </button>
            ))}
          </div>
          {form.kind === "circular" && <p className="aj-kind-hint">Round the axis through the assembly's origin.</p>}
          <div className="row-buttons">
            <button title="Make the copies; they follow the seed when it moves"
              type="button"
              className="primary"
              onClick={async () => {
                const count = Math.round(Number(form.count));
                const out = await run(
                  "assembly.pattern.create",
                  form.kind === "linear" ? { seed: entity.id, type: "linear", count, spacing: Number(form.spacing), direction: AXES[form.axis].v } : { seed: entity.id, type: "circular", count, span: Number(form.span), axis: AXES[form.axis].v, center: [0, 0, 0] },
                );
                if (out) setForm(null);
              }}
            >
              <Icon name={form.kind === "linear" ? "patternLinear" : "patternCircular"} /> Make copies
            </button>
            <button title="Close without making a pattern" type="button" className="secondary" onClick={() => setForm(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

// ---- Playing frames (studies, trajectories) -----------------------------------------------

type Frame = { value: number; poses: [V3, Quat][]; joints: Record<string, number[]> };
type Gap = { a: string; b: string; a_id: string; b_id: string; gap: number; at: number; range?: [number, number]; joined: boolean };
export type Played = { instances: { id: string; name: string; moves: boolean }[]; frames: Frame[]; binds_at: number | null; pairs: Gap[]; clashes: Gap[]; notes: string[] };

/** Placements at fractional frame `t`. */
function posesAtFrame(p: Played, t: number): Map<string, Transform> {
  const f = p.frames;
  const i = Math.max(0, Math.min(f.length - 2, Math.floor(t)));
  const k = Math.max(0, Math.min(1, t - i));
  const out = new Map<string, Transform>();
  p.instances.forEach((inst, j) => {
    const a = f[i].poses[j];
    const b = f[Math.min(i + 1, f.length - 1)].poses[j];
    const at = a[0].map((c, m) => c + (b[0][m] - c) * k) as V3;
    const sign = Math.sign(a[1].reduce((d, x, w) => d + x * b[1][w], 0)) || 1;
    let q = a[1].map((c, m) => c + (b[1][m] * sign - c) * k);
    const len = Math.hypot(...q);
    q = q.map((c) => c / len);
    out.set(inst.id, { at, q: q as Quat });
  });
  return out;
}

const frames2 = () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));

/** A store-only zip of files (names and bytes). */
function zip(files: { name: string; data: Uint8Array }[]): Blob {
  const table = new Uint32Array(256).map((_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (d: Uint8Array) => {
    let c = 0xffffffff;
    for (const b of d) c = table[(c ^ b) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const parts: BlobPart[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  const enc = new TextEncoder();
  for (const f of files) {
    const name = enc.encode(f.name);
    const c = crc(f.data);
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(4, 20, true);
    local.setUint32(14, c, true);
    local.setUint32(18, f.data.length, true);
    local.setUint32(22, f.data.length, true);
    local.setUint16(26, name.length, true);
    parts.push(local.buffer, name, f.data as BlobPart);
    const cd = new DataView(new ArrayBuffer(46));
    cd.setUint32(0, 0x02014b50, true);
    cd.setUint16(4, 20, true);
    cd.setUint16(6, 20, true);
    cd.setUint32(16, c, true);
    cd.setUint32(20, f.data.length, true);
    cd.setUint32(24, f.data.length, true);
    cd.setUint16(28, name.length, true);
    cd.setUint32(42, offset, true);
    const entry = new Uint8Array(46 + name.length);
    entry.set(new Uint8Array(cd.buffer), 0);
    entry.set(name, 46);
    central.push(entry);
    offset += 30 + name.length + f.data.length;
  }
  const size = central.reduce((s, e) => s + e.length, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, files.length, true);
  end.setUint16(10, files.length, true);
  end.setUint32(12, size, true);
  end.setUint32(16, offset, true);
  return new Blob([...parts, ...central.map((e) => e as BlobPart), end.buffer], { type: "application/zip" });
}

function download(blob: Blob, name: string) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  window.setTimeout(() => URL.revokeObjectURL(a.href), 5000);
}

/** A timeline over played frames: scrub, play, see the clash ranges, export the frames. */
export function Player(props: { played: Played; unit: string; name: string; view: JointView; overlay?: (t: number) => void; onDone: () => void }) {
  const { played, view } = props;
  const last = played.frames.length - 1;
  const [at, setAt] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [exporting, setExporting] = useState<string | null>(null);
  const span = played.frames[last]?.value ?? 1;
  const first = played.frames[0]?.value ?? 0;
  useEffect(() => {
    if (!playing) return;
    let raf = 0;
    let t0: number | null = null;
    const start = at >= last ? 0 : at;
    const seconds = props.unit === "s" ? Math.max(0.5, span - first) : 4;
    const tick = (t: number) => {
      t0 ??= t;
      const k = start + ((t - t0) / 1000 / seconds) * last;
      if (k >= last) {
        setAt(last);
        setPlaying(false);
        return;
      }
      setAt(k);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing]);
  useEffect(() => {
    if (last < 1) return;
    view.poses(posesAtFrame(played, at));
    props.overlay?.(at);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [at, played, view]);
  // Leaving stops an export part way and puts the parts back.
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
      view.poses(null);
    },
    [view],
  );
  const value = played.frames[Math.round(at)]?.value ?? 0;
  const pct = (v: number) => ((v - first) / Math.max(1e-9, span - first)) * 100;
  const exportPng = async () => {
    setPlaying(false);
    const files: { name: string; data: Uint8Array }[] = [];
    for (let k = 0; k <= last && alive.current; k++) {
      setExporting(`Frame ${k + 1} of ${last + 1}…`);
      view.poses(posesAtFrame(played, k));
      props.overlay?.(k);
      await frames2();
      const url = view.snapshot();
      if (!url) break;
      const bin = atob(url.split(",")[1]);
      const data = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i);
      files.push({ name: `${props.name.replace(/[^\w-]+/g, "-")}-${String(k).padStart(4, "0")}.png`, data });
    }
    if (!alive.current) return;
    download(zip(files), `${props.name.replace(/[^\w-]+/g, "-")}-frames.zip`);
    setExporting(null);
    setAt(last);
  };
  const exportVideo = async () => {
    const canvas = document.querySelector(".viewport canvas") as HTMLCanvasElement | null;
    if (!canvas || typeof MediaRecorder === "undefined") return;
    setPlaying(false);
    const fps = props.unit === "s" ? Math.max(1, Math.round(last / Math.max(1e-3, span - first))) : 30;
    const stream = canvas.captureStream(fps);
    const rec = new MediaRecorder(stream, { mimeType: MediaRecorder.isTypeSupported("video/webm;codecs=vp9") ? "video/webm;codecs=vp9" : "video/webm" });
    const chunks: Blob[] = [];
    rec.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    const done = new Promise<void>((r) => (rec.onstop = () => r()));
    rec.start();
    for (let k = 0; k <= last && alive.current; k++) {
      setExporting(`Recording ${k + 1} of ${last + 1}…`);
      view.poses(posesAtFrame(played, k));
      props.overlay?.(k);
      await new Promise((r) => window.setTimeout(r, 1000 / fps));
    }
    rec.stop();
    await done;
    if (!alive.current) return;
    download(new Blob(chunks, { type: "video/webm" }), `${props.name.replace(/[^\w-]+/g, "-")}.webm`);
    setExporting(null);
  };
  return (
    <div className="aj-sweep at-player">
      <div className={`gear-verdict ${played.clashes.length || played.binds_at !== null ? "bad" : "ok"}`}>
        {played.clashes.length > 0 ? (
          played.clashes.map((c) => (
            <button title={`Jump to where ${c.a} first hits ${c.b}`} key={`${c.a_id}${c.b_id}`} type="button" className="clash-jump" onClick={() => (setPlaying(false), setAt(((c.range?.[0] ?? c.at) - first) / Math.max(1e-9, span - first) * last))}>
              {c.a} hits {c.b} from {n1(c.range?.[0] ?? c.at, 2)} to {n1(c.range?.[1] ?? c.at, 2)} {props.unit}
            </button>
          ))
        ) : (
          <span>
            {played.instances.filter((i) => i.moves).length} of {played.instances.length} parts move · no clashes
          </span>
        )}
        {played.binds_at !== null && <span>Binds at {n1(played.binds_at, 2)} {props.unit}</span>}
      </div>
      <div className="aj-track" aria-hidden="true">
        {played.clashes.map((c) => {
          const [a, b] = c.range ?? [c.at, c.at];
          return <span key={`${c.a_id}${c.b_id}`} className="aj-clash" style={{ left: `${pct(a)}%`, width: `${Math.max(0.8, pct(b) - pct(a))}%` }} />;
        })}
        <span className="aj-head-mark" style={{ left: `${(at / Math.max(1, last)) * 100}%` }} />
      </div>
      <div className="gear-turn-row">
        <button type="button" className="gear-play" aria-label={playing ? "Pause" : "Play"} onClick={() => setPlaying((p) => !p)}>
          <Icon name={playing ? "pause" : "play"} />
        </button>
        <input type="range" min={0} max={Math.max(1, last)} step={0.01} value={at} aria-label={`${props.name} timeline`} onChange={(e) => (setPlaying(false), setAt(Number(e.target.value)))} />
        <output className="gear-turn-angle">
          {n1(value, 2)}
          {props.unit === "s" ? " s" : ""}
        </output>
      </div>
      {played.notes.map((t) => (
        <p key={t} className="motion-note">
          {t}
        </p>
      ))}
      <div className="row-buttons">
        <button type="button" className="secondary" disabled={!!exporting} onClick={exportPng} title="Every frame as a PNG image, in a zip">
          <Icon name="download" /> Frames
        </button>
        <button type="button" className="secondary" disabled={!!exporting} onClick={exportVideo} title="Record the motion as a WebM video">
          <Icon name="download" /> Video
        </button>
        <button title="Close the player and put the parts back" type="button" className="secondary" onClick={props.onDone}>
          Done
        </button>
      </div>
      {exporting && <p className="motion-note">{exporting}</p>}
    </div>
  );
}

// ---- The assembly's studio: poses, exploded views, motion, robot, interference -----------

type Tab = "poses" | "explode" | "motion" | "robot" | "clash";
const TABS: { tab: Tab; label: string; tip: string }[] = [
  { tab: "poses", label: "Poses", tip: "Named joint values to come back to: home, park, reach" },
  { tab: "explode", label: "Exploded", tip: "Saved exploded views: the parts pulled apart in steps" },
  { tab: "motion", label: "Motion", tip: "Motion studies: several joints on one timeline, checked for clashes" },
  { tab: "robot", label: "Robot", tip: "Arm kinematics: reach a point, the reach envelope, paths through poses" },
  { tab: "clash", label: "Interference", tip: "Check every pair of parts for overlaps and near misses" },
];

type PoseData = { name: string; assembly?: string; values: Record<string, number[]> };
type ExplodeData = { name: string; assembly?: string; steps: unknown[] };
type StudyData = { name: string; assembly?: string; duration: number; tracks: unknown[] };
const comp = <T,>(e: EntityView, k: string) => e.components[k] as T | undefined;

export function AssemblyStudio(props: { scope: string; frame: Transform; entities: ReadonlyMap<string, EntityView>; run: Run; view: JointView; onSelect: (id: string) => void }) {
  const [tab, setTab] = useState<Tab | null>(null);
  return (
    <section className="props-section at-studio">
      <header>
        <h3>Assembly studio</h3>
        <span className="section-note">saved views and motion</span>
      </header>
      <div className="seg-toggle at-tabs" role="tablist">
        {TABS.map((t) => (
          <button title={t.tip} key={t.tab} type="button" role="tab" aria-selected={tab === t.tab} className={tab === t.tab ? "on" : ""} onClick={() => setTab((x) => (x === t.tab ? null : t.tab))}>
            {t.label}
          </button>
        ))}
      </div>
      {tab === "poses" && <Poses key={props.scope} {...props} />}
      {tab === "explode" && <Exploded key={props.scope} {...props} />}
      {tab === "motion" && <Studies key={props.scope} {...props} />}
      {tab === "robot" && <Robot key={props.scope} {...props} />}
      {tab === "clash" && <Interference key={props.scope} {...props} />}
    </section>
  );
}

type StudioProps = { scope: string; frame: Transform; entities: ReadonlyMap<string, EntityView>; run: Run; view: JointView; onSelect: (id: string) => void };

/** Leaving a tool puts the view back: parts where they are, nothing drawn over them. */
function useCleanView(view: JointView) {
  useEffect(
    () => () => {
      view.poses(null);
      view.overlay(null);
    },
    [view],
  );
}

function Poses({ scope, entities, run }: StudioProps) {
  const [name, setName] = useState("");
  const poses = [...entities.values()].filter((e) => comp<PoseData>(e, "assembly.pose")?.assembly === scope);
  return (
    <div className="at-tab">
      {poses.length === 0 && <p className="aj-empty">Name the joints' values to come back to: home, park, reach.</p>}
      {poses.map((e) => {
        const p = comp<PoseData>(e, "assembly.pose")!;
        return (
          <div key={e.id} className="at-row">
            <Icon name="rotate" />
            <span className="at-row-main">
              <strong>{p.name}</strong>
              <small>{Object.keys(p.values).length} joints</small>
            </span>
            <button type="button" className="secondary at-small" onClick={() => run("assembly.pose.apply", { target: e.id })} title="Put every joint back to this pose">
              Apply
            </button>
            <button type="button" className="at-icon" title="Keep the joints as they are now in this pose" onClick={() => run("assembly.pose.update", { target: e.id, capture: true })}>
              <Icon name="refresh" />
            </button>
            <button type="button" className="at-icon" title="Delete the pose" onClick={() => run("assembly.pose.delete", { target: e.id })}>
              <Icon name="trash" />
            </button>
          </div>
        );
      })}
      <div className="at-inline">
        <input value={name} placeholder="Pose name (home, reach…)" onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && name.trim() && run("assembly.pose.create", { assembly: scope, name: name.trim() }).then((o) => o && setName(""))} />
        <button title="Save the joints' current values as a named pose (Enter)" type="button" className="primary" disabled={!name.trim()} onClick={() => run("assembly.pose.create", { assembly: scope, name: name.trim() }).then((o) => o && setName(""))}>
          Save pose
        </button>
      </div>
    </div>
  );
}

type ExplodedResult = { name: string; instances: { id: string; name: string; moves: boolean }[]; steps: { name: string; poses: [V3, Quat][] }[]; trails: { instance: string; points: V3[] }[] };

function Exploded({ scope, frame, entities, run, view }: StudioProps) {
  useCleanView(view);
  const views = [...entities.values()].filter((e) => comp<ExplodeData>(e, "assembly.exploded_view")?.assembly === scope);
  const [open, setOpen] = useState<{ id: string; r: ExplodedResult } | null>(null);
  const [at, setAt] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [spread, setSpread] = useState("1");
  const [error, setError] = useState<string | null>(null);
  const show = async (id: string) => {
    try {
      const r = await getJson<ExplodedResult>(`/explode/${id}`);
      setOpen({ id, r });
      setAt(r.steps.length - 1);
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    }
  };
  useEffect(() => {
    if (!open) return;
    const r = open.r;
    const played: Played = { instances: r.instances, frames: r.steps.map((s, k) => ({ value: k, poses: s.poses, joints: {} })), binds_at: null, pairs: [], clashes: [], notes: [] };
    view.poses(posesAtFrame(played, at));
    // Trails up to where it is, in the project's coordinates.
    const upto = Math.floor(at);
    view.overlay({ lines: r.trails.map((t) => ({ points: t.points.slice(0, upto + 1).map((p) => applyT(frame, p)), color: "#6d4ae0", dashed: true })) });
  }, [open, at, frame, view]);
  useEffect(() => {
    if (!playing || !open) return;
    let raf = 0;
    let t0: number | null = null;
    const last = open.r.steps.length - 1;
    const start = at >= last ? 0 : at;
    const tick = (t: number) => {
      t0 ??= t;
      const k = start + (t - t0) / 900;
      if (k >= last) {
        setAt(last);
        setPlaying(false);
        return;
      }
      setAt(k);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing]);
  const close = () => {
    setOpen(null);
    view.poses(null);
    view.overlay(null);
  };
  return (
    <div className="at-tab">
      {views.length === 0 && !open && <p className="aj-empty">Save exploded views: the parts pulled apart in steps, with the trails they leave, to play and to show.</p>}
      {views.map((e) => {
        const v = comp<ExplodeData>(e, "assembly.exploded_view")!;
        return (
          <div key={e.id} className={`at-row ${open?.id === e.id ? "on" : ""}`}>
            <Icon name="explode" />
            <button title={open?.id === e.id ? "Close the exploded view and put the parts back" : `Play ${v.name}: the parts pulled apart step by step`} type="button" className="at-row-main at-link" onClick={() => (open?.id === e.id ? close() : show(e.id))}>
              <strong>{v.name}</strong>
              <small>{v.steps.length === 1 ? "1 step" : `${v.steps.length} steps`}</small>
            </button>
            <button type="button" className="at-icon" title="Delete the exploded view" onClick={() => (open?.id === e.id && close(), run("assembly.explode.delete", { target: e.id }))}>
              <Icon name="trash" />
            </button>
          </div>
        );
      })}
      {open && (
        <div className="aj-sweep">
          <div className="gear-turn-row">
            <button type="button" className="gear-play" aria-label={playing ? "Pause" : "Play the steps"} onClick={() => setPlaying((p) => !p)}>
              <Icon name={playing ? "pause" : "play"} />
            </button>
            <input type="range" min={0} max={open.r.steps.length - 1} step={0.01} value={at} aria-label="Exploded step" onChange={(e) => (setPlaying(false), setAt(Number(e.target.value)))} />
            <output className="gear-turn-angle">{open.r.steps[Math.round(at)]?.name}</output>
          </div>
          <div className="row-buttons">
            <button title="Close the exploded view and put the parts back" type="button" className="secondary" onClick={close}>
              Done
            </button>
          </div>
        </div>
      )}
      {error && <p className="print-note bad">{error}</p>}
      <div className="at-inline">
        <label title="How far the parts are pulled apart: 1 is the usual spacing, 2 twice as far" className="at-spread">
          <span>Spread</span>
          <input value={spread} inputMode="decimal" onChange={(e) => setSpread(e.target.value)} />
        </label>
        <button title="Pull the parts apart automatically, by the spread given, into a new saved view"
          type="button"
          className="primary"
          onClick={async () => {
            const out = (await run("assembly.explode.create", { assembly: scope, name: `Exploded ${views.length + 1}`, auto: { spread: Number(spread) || 1 } })) as { id: string } | null;
            if (out) void show(out.id);
          }}
        >
          <Icon name="explode" /> New exploded view
        </button>
      </div>
    </div>
  );
}

function Studies({ scope, entities, run, view }: StudioProps) {
  useCleanView(view);
  const studies = [...entities.values()].filter((e) => comp<StudyData>(e, "assembly.motion_study")?.assembly === scope);
  const poses = [...entities.values()].filter((e) => comp<PoseData>(e, "assembly.pose")?.assembly === scope);
  const [playing, setPlaying] = useState<{ id: string; name: string; played: Played } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [gap, setGap] = useState("1.5");
  const open = async (id: string, name: string) => {
    setBusy(id);
    try {
      const played = await getJson<Played>(`/motion-study/${id}?fps=24`);
      setPlaying({ id, name, played });
      setError(null);
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="at-tab">
      {studies.length === 0 && <p className="aj-empty">A motion study moves several joints on one timeline — through saved poses, or joint by joint — checks every pair of parts on the way, and exports as frames or video.</p>}
      {studies.map((e) => {
        const s = comp<StudyData>(e, "assembly.motion_study")!;
        return (
          <div key={e.id} className={`at-row ${playing?.id === e.id ? "on" : ""}`}>
            <Icon name="play" />
            <button title={playing?.id === e.id ? "Stop the study and put the parts back" : `Play ${s.name} and check every pair of parts on the way`} type="button" className="at-row-main at-link" onClick={() => (playing?.id === e.id ? (setPlaying(null), view.poses(null)) : open(e.id, s.name))}>
              <strong>{s.name}</strong>
              <small>
                {n1(s.duration, 1)} s · {s.tracks.length === 1 ? "1 joint" : `${s.tracks.length} joints`}{busy === e.id ? " · playing it through…" : ""}
              </small>
            </button>
            <button type="button" className="at-icon" title="Delete the study" onClick={() => (playing?.id === e.id && setPlaying(null), run("assembly.motion_study.delete", { target: e.id }))}>
              <Icon name="trash" />
            </button>
          </div>
        );
      })}
      {playing && <Player played={playing.played} unit="s" name={playing.name} view={view} onDone={() => (setPlaying(null), view.poses(null))} />}
      {error && <p className="print-note bad">{error}</p>}
      {poses.length >= 2 ? (
        <div className="at-form">
          <p className="aj-kind-hint">Through poses, in the order clicked:</p>
          <div className="at-chips">
            {poses.map((e) => {
              const p = comp<PoseData>(e, "assembly.pose")!;
              const k = chosen.indexOf(e.id);
              return (
                <button title={`Add ${p.name} as the next pose the study moves through`} key={e.id} type="button" className={`at-chip ${k >= 0 ? "on" : ""}`} onClick={() => setChosen((c) => [...c, e.id])}>
                  {k >= 0 && <em>{chosen.map((x, i) => (x === e.id ? i + 1 : null)).filter(Boolean).join(",")}</em>}
                  {p.name}
                </button>
              );
            })}
          </div>
          <div className="at-inline">
            <label title="Seconds the motion takes from each pose to the next" className="at-spread">
              <span>Seconds each</span>
              <input value={gap} inputMode="decimal" onChange={(e) => setGap(e.target.value)} />
            </label>
            <button title="Make a motion study through the chosen poses, in the order clicked"
              type="button"
              className="primary"
              disabled={chosen.length < 2}
              onClick={async () => {
                const g = Number(gap) || 1.5;
                const out = (await run("assembly.motion_study.create", { assembly: scope, name: `Study ${studies.length + 1}`, poses: chosen.map((id, i) => ({ pose: id, at: i * g })) })) as { id: string; name: string } | null;
                if (out) {
                  setChosen([]);
                  void open(out.id, out.name);
                }
              }}
            >
              New study
            </button>
            {chosen.length > 0 && (
              <button title="Forget the chosen poses" type="button" className="secondary" onClick={() => setChosen([])}>
                Clear
              </button>
            )}
          </div>
        </div>
      ) : (
        <p className="aj-quiet">Save two or more poses to make a study through them (or ask Forma AI for one joint by joint).</p>
      )}
    </div>
  );
}

type Axis = { joint: string; name: string; key: string; unit: string; min: number; max: number };
type ChainInfo = { tip: string; tip_name: string; axes: Axis[]; values: Record<string, number>; tool: { at: V3; axis: V3 } };
type Reach = { samples: number; points: V3[]; bounds_min: V3; bounds_max: V3; base: V3; max_reach: number; min_reach: number };
type Trajectory = Played & { waypoints: { reached: boolean; error_mm: number }[]; tool_path: V3[] };

function Robot({ scope, frame, entities, run, view }: StudioProps) {
  useCleanView(view);
  const members = membersOf(scope, entities);
  // Parts a joint places (hinges, sliders…): the ends of chains; what only mates hold is no arm.
  const held = members.filter((m) => {
    const by = m.placed?.by ? entities.get(m.placed.by) : undefined;
    const kind = (by?.components["assembly.joint"] as { kind?: string } | undefined)?.kind ?? "";
    return !!by && !isMate(kind);
  });
  // The likely end of the arm: a tool, a gripper body, a hand… in that order; else the last part a joint places.
  const guess = [/tool|effector/i, /gripper body|gripper housing/i, /grip|hand|claw/i, /wrist/i, /finger/i].map((re) => held.find((m) => re.test(instance(m)!.name))).find(Boolean) ?? held[held.length - 1];
  const [tip, setTip] = useState<string | null>(guess?.id ?? null);
  const [chain, setChain] = useState<ChainInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [target, setTarget] = useState<string[]>(["", "", ""]);
  const [ik, setIk] = useState<{ reached: boolean; error_mm: number; values: Record<string, number> } | null>(null);
  const [reach, setReach] = useState<Reach | null>(null);
  const [path, setPath] = useState<Trajectory | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const poses = [...entities.values()].filter((e) => comp<PoseData>(e, "assembly.pose")?.assembly === scope);
  const placedKey = members.map((m) => m.placed?.at.join(",")).join("|");
  useEffect(() => {
    if (!tip) return;
    let live = true;
    getJson<ChainInfo>(`/robot/${tip}`)
      .then((c) => {
        if (!live) return;
        setChain(c);
        setError(null);
        setTarget((t) => (t.every((x) => x === "") ? c.tool.at.map((v) => String(Math.round(v * 10) / 10)) : t));
      })
      .catch((e) => live && (setChain(null), setError(String(e.message))));
    return () => {
      live = false;
    };
  }, [tip, placedKey]);
  // The tool, the target, the reach cloud and a path, in the project's coordinates.
  const world = (p: V3) => applyT(frame, p);
  const tv = target.map(Number) as V3;
  useEffect(() => {
    const pts: { at: V3[]; color?: string; size?: number }[] = [];
    if (reach) pts.push({ at: reach.points.map(world), color: "#8b66ef", size: 2.5 });
    if (chain) pts.push({ at: [world(chain.tool.at)], color: "#e0a03a", size: 9 });
    if (target.every((x) => x.trim() !== "" && Number.isFinite(Number(x)))) pts.push({ at: [world(tv)], color: ik && !ik.reached ? "#d6455d" : "#2f9e6e", size: 10 });
    view.overlay({ points: pts, lines: path ? [{ points: path.tool_path.map(world), color: "#e0a03a" }] : [] });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reach, chain, target, ik, path, frame, view]);
  if (!held.length) return <p className="aj-empty">Join the arm's parts with hinges and sliders first: the chain runs from the ground out to the part you pick.</p>;
  return (
    <div className="at-tab">
      <label className="prop-field">
        <span>End of the arm</span>
        <select title="The part at the end of the arm: the chain runs from the ground out to it" value={tip ?? ""} onChange={(e) => (setTip(e.target.value), setReach(null), setIk(null), setPath(null), setTarget(["", "", ""]))}>
          {held.map((m) => (
            <option key={m.id} value={m.id}>
              {instance(m)!.name}
            </option>
          ))}
        </select>
      </label>
      {error && <p className="print-note bad">{error}</p>}
      {chain && (
        <>
          <div className="at-axes">
            {chain.axes.map((a) => (
              <div key={`${a.joint}${a.key}`} className="at-axis">
                <span>{a.name}</span>
                <strong>
                  {n1(chain.values[a.name] ?? 0)}
                  <small>{a.unit === "deg" ? "°" : " mm"}</small>
                </strong>
                <small>
                  {n1(a.min, 0)}…{n1(a.max, 0)}
                </small>
              </div>
            ))}
          </div>
          <p className="motion-note">
            Tool at ({chain.tool.at.map((v) => n1(v)).join(", ")}) mm, pointing ({chain.tool.axis.map((v) => n1(v, 2)).join(", ")})
          </p>
          <div className="at-sub">
            <span>Reach a point</span>
          </div>
          <div className="aj-fields at-xyz">
            {(["X", "Y", "Z"] as const).map((ax, k) => (
              <label key={ax}>
                <span>{ax} (mm)</span>
                <input value={target[k]} inputMode="decimal" onChange={(e) => (setIk(null), setTarget((t) => t.map((x, i) => (i === k ? e.target.value : x))))} />
              </label>
            ))}
          </div>
          <div className="row-buttons">
            <button title="Check whether the arm's tip can reach the point, without moving it"
              type="button"
              className="secondary"
              onClick={async () => {
                try {
                  const r = await getJson<{ reached: boolean; error_mm: number; values: Record<string, number> }>(`/robot/${tip}/ik?to=${JSON.stringify(tv)}`);
                  setIk(r);
                } catch (e) {
                  setError(String((e as Error).message));
                }
              }}
            >
              Can it reach?
            </button>
            <button title="Set the arm's joints so its tip reaches the point" type="button" className="primary" onClick={() => run("assembly.robot.move_to", { tip, to: tv }).then(() => setIk(null))}>
              Move there
            </button>
          </div>
          {ik && <p className={ik.reached ? "motion-note" : "print-note bad"}>{ik.reached ? `Reaches it: ${Object.entries(ik.values).map(([k, v]) => `${k} ${n1(v)}`).join(", ")}` : `Out of reach: the closest is ${n1(ik.error_mm, 2)} mm away.`}</p>}
          <div className="at-sub">
            <span>Reach envelope</span>
            {reach ? (
              <button title="Hide the cloud of points the tip can reach" type="button" className="aj-add" onClick={() => setReach(null)}>
                Hide
              </button>
            ) : (
              <button title="Show the cloud of points the tip can reach within the joints' limits" type="button" className="aj-add" onClick={() => getJson<Reach>(`/robot/${tip}/reach?samples=2500`).then(setReach, (e) => setError(String(e.message)))}>
                Show
              </button>
            )}
          </div>
          {reach && (
            <p className="motion-note">
              {reach.samples} poses within the joints' limits: the tool reaches {n1(reach.min_reach)}–{n1(reach.max_reach)} mm from the first joint.
            </p>
          )}
          {poses.length >= 2 && (
            <>
              <div className="at-sub">
                <span>Path through poses</span>
              </div>
              <div className="at-chips">
                {poses.map((e) => {
                  const p = comp<PoseData>(e, "assembly.pose")!;
                  const k = chosen.indexOf(e.id);
                  return (
                    <button title={`Add ${p.name} as the next pose the arm moves through`} key={e.id} type="button" className={`at-chip ${k >= 0 ? "on" : ""}`} onClick={() => setChosen((c) => [...c, e.id])}>
                      {k >= 0 && <em>{chosen.map((x, i) => (x === e.id ? i + 1 : null)).filter(Boolean).join(",")}</em>}
                      {p.name}
                    </button>
                  );
                })}
              </div>
              <div className="row-buttons">
                <button title="Play the arm moving through the chosen poses, in the order clicked"
                  type="button"
                  className="primary"
                  disabled={!chosen.length}
                  onClick={async () => {
                    try {
                      const through = chosen.map((id) => ({ pose: id }));
                      setPath(await getJson<Trajectory>(`/robot/${tip}/trajectory?fps=12&through=${encodeURIComponent(JSON.stringify(through))}`));
                      setError(null);
                    } catch (e) {
                      setError(String((e as Error).message));
                    }
                  }}
                >
                  <Icon name="play" /> Move through them
                </button>
                {chosen.length > 0 && (
                  <button title="Forget the chosen poses" type="button" className="secondary" onClick={() => setChosen([])}>
                    Clear
                  </button>
                )}
              </div>
            </>
          )}
          {path && <Player played={path} unit="s" name={`${chain.tip_name} path`} view={view} onDone={() => (setPath(null), view.poses(null))} />}
        </>
      )}
    </div>
  );
}

type Clash = { a: string; b: string; a_path: string[]; b_path: string[]; volume: number; at?: V3; size?: V3; gap: number; joined: boolean; fastener?: boolean; embedded?: boolean; outside?: number };
/** Overlap that is how the parts go together: joined, a fastener threading in, a part embedded in its host. */
const meant = (o: Clash) => o.joined || !!o.fastener || !!o.embedded;
type InterferenceReport = { name: string; parts: number; checked: number; overlaps: Clash[]; touching: Clash[]; near: Clash[]; clearance: number; notes: string[] };

function Interference({ scope, frame, view, onSelect }: StudioProps) {
  useCleanView(view);
  const [r, setR] = useState<InterferenceReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [clearance, setClearance] = useState("0.5");
  const go = async () => {
    setBusy(true);
    try {
      const rep = await getJson<InterferenceReport>(`/interference/${scope}?clearance=${Number(clearance) || 0}`);
      setR(rep);
      setError(null);
      view.overlay({ points: [{ at: rep.overlaps.filter((o) => o.at).map((o) => applyT(frame, o.at!)), color: "#d6455d", size: 11 }] });
    } catch (e) {
      setError(String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="at-tab">
      <div className="at-inline">
        <label title="Pairs closer than this many millimetres are listed as near misses" className="at-spread">
          <span>Near (mm)</span>
          <input value={clearance} inputMode="decimal" onChange={(e) => setClearance(e.target.value)} />
        </label>
        <button title="Check every pair of parts for overlaps, touching and gaps under the Near distance" type="button" className="primary" disabled={busy} onClick={go}>
          <Icon name="collide" /> {busy ? "Checking every pair…" : r ? "Check again" : "Check interference"}
        </button>
      </div>
      {error && <p className="print-note bad">{error}</p>}
      {r && (
        <>
          <div className={`gear-verdict ${r.overlaps.some((o) => !meant(o)) ? "bad" : "ok"}`}>
            <span>
              {r.parts} parts · {r.overlaps.length} overlap · {r.touching.length} touch{r.clearance ? ` · ${r.near.length} within ${r.clearance} mm` : ""}
            </span>
          </div>
          <ul className="aj-pairs">
            {[...r.overlaps.filter((o) => !meant(o)), ...r.near, ...r.overlaps.filter(meant)].slice(0, 40).map((o) => (
              <li key={`${o.a_path.join()}|${o.b_path.join()}`}>
                <button title={`Select ${o.a}`} type="button" onClick={() => o.a_path[0] && onSelect(o.a_path[0])}>
                  <span className={`aj-dot ${o.volume > 0 ? (meant(o) ? "joined" : "hit") : "near"}`} />
                  <span>
                    {o.a} – {o.b}
                    {o.fastener ? <em>fastener</em> : o.embedded ? <em>embedded</em> : o.outside ? <em>pokes out {n1(o.outside, 1)} mm³</em> : o.joined && <em>joined</em>}
                  </span>
                  <small>{o.volume > 0 ? `${n1(o.volume, 2)} mm³` : `${n1(o.gap, 2)} mm`}</small>
                </button>
              </li>
            ))}
          </ul>
          {r.notes.map((t) => (
            <p key={t} className="motion-note">
              {t}
            </p>
          ))}
        </>
      )}
    </div>
  );
}
