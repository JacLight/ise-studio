// Joint-space interlocks of an assembly: where some joints stand, others are
// held to a narrower range (a shoulder low keeps the elbow from folding the
// gripper into the base). Listed in words with their joints' names; a zone
// is added here in one line (when … then …), limit tables come from the
// engine (Forma AI or the console). Sweeps, studies, robot moves and the
// joint sliders all respect them.

import { useState } from "react";
import type { EntityView } from "./engine";
import { Icon } from "./icons";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
type Range = { joint: string; key?: string; min?: number; max?: number };
type InterlockData = {
  name: string;
  assembly?: string | null;
  description?: string;
  zones?: { when: Range[]; then: Range[] }[];
  tables?: { by: string; joint: string; rows: [number, number, number][] }[];
};

export const interlockOf = (e: EntityView | undefined) => e?.components["assembly.interlock"] as InterlockData | undefined;

const n = (v: number) => Number(v.toFixed(2)).toLocaleString("en-US");

export function Interlocks(props: { scope: string | null; joints: EntityView[]; entities: ReadonlyMap<string, EntityView>; run: Run }) {
  const { scope, joints, entities, run } = props;
  const list = [...entities.values()].filter((e) => interlockOf(e) && (interlockOf(e)!.assembly ?? null) === scope);
  const moving = joints.filter((j) => ["hinge", "slider", "cylinder", "angle", "distance"].includes((j.components["assembly.joint"] as { kind: string }).kind));
  const [form, setForm] = useState<{ name: string; when: string; whenOp: "max" | "min"; whenV: string; then: string; thenOp: "max" | "min"; thenV: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  if (moving.length < 2 && !list.length) return null;
  const jname = (id: string) => (entities.get(id)?.components["shape.parameter_set"] as { name?: string } | undefined)?.name ?? "joint";
  const unit = (id: string) => ((entities.get(id)?.components["assembly.joint"] as { kind?: string } | undefined)?.kind === "slider" ? " mm" : "°");
  const range = (r: Range) => {
    const u = unit(r.joint);
    const name = `${jname(r.joint)}${r.key ? `.${r.key}` : ""}`;
    if (r.min != null && r.max != null) return `${name} ${n(r.min)}…${n(r.max)}${u}`;
    if (r.max != null) return `${name} ≤ ${n(r.max)}${u}`;
    return `${name} ≥ ${n(r.min!)}${u}`;
  };
  const words = (il: InterlockData) => [
    ...(il.zones ?? []).map((z) => `when ${z.when.map(range).join(" and ")}: ${z.then.map(range).join(", ")}`),
    ...(il.tables ?? []).map((t) => `${jname(t.joint)} within ${t.rows.map(([b, lo, hi]) => `${n(lo)}…${n(hi)}${unit(t.joint)} at ${n(b)}${unit(t.by)}`).join(", ")} of ${jname(t.by)}`),
  ];
  const add = async () => {
    if (!form) return;
    const [wv, tv] = [Number(form.whenV), Number(form.thenV)];
    if (!Number.isFinite(wv) || !Number.isFinite(tv) || form.when === form.then) return setError("Pick two joints and a value for each.");
    setError(null);
    const out = await run("assembly.interlock.create", {
      assembly: scope,
      name: form.name.trim() || `Interlock ${list.length + 1}`,
      zones: [{ when: { [form.when]: { [form.whenOp]: wv } }, then: { [form.then]: { [form.thenOp]: tv } } }],
    });
    if (out) setForm(null);
  };
  return (
    <div className="at-couplings at-interlocks">
      <div className="at-sub">
        <span>Interlocks</span>
        {!form && moving.length >= 2 && (
          <button
            type="button"
            className="aj-add"
            onClick={() => setForm({ name: "", when: moving[0].id, whenOp: "max", whenV: "", then: moving[1].id, thenOp: "max", thenV: "" })}
            title="Hold a joint to a narrower range while another stands in a range (a shoulder low keeps the elbow from folding into the base)"
          >
            <Icon name="plus" /> Interlock
          </button>
        )}
      </div>
      {list.map((e) => {
        const il = interlockOf(e)!;
        return (
          <div key={e.id} className="at-row">
            <Icon name="shield" />
            <span className="at-row-main">
              <strong>{il.name}</strong>
              {words(il).map((w) => (
                <small key={w}>{w}</small>
              ))}
            </span>
            <button type="button" className="at-icon" title="Delete the interlock: the joints move within their own limits again" onClick={() => run("assembly.interlock.delete", { target: e.id })}>
              <Icon name="trash" />
            </button>
          </div>
        );
      })}
      {form && (
        <div className="aj-form at-form">
          <div className="aj-fields">
            <label>
              <span>Name</span>
              <input value={form.name} placeholder="Fold guard" onChange={(ev) => setForm({ ...form, name: ev.target.value })} />
            </label>
            {(["when", "then"] as const).map((role) => (
              <label key={role} className="il-rule">
                <span>{role === "when" ? "While" : "Hold"}</span>
                <select value={form[role]} onChange={(ev) => setForm({ ...form, [role]: ev.target.value })} title={role === "when" ? "The joint whose position sets the condition" : "The joint held to a narrower range"}>
                  {moving.map((j) => (
                    <option key={j.id} value={j.id}>
                      {jname(j.id)}
                    </option>
                  ))}
                </select>
                <select value={form[role === "when" ? "whenOp" : "thenOp"]} onChange={(ev) => setForm({ ...form, [role === "when" ? "whenOp" : "thenOp"]: ev.target.value as "max" | "min" })}>
                  <option value="max">at most</option>
                  <option value="min">at least</option>
                </select>
                <input inputMode="decimal" value={form[role === "when" ? "whenV" : "thenV"]} placeholder={unit(form[role]).trim()} onChange={(ev) => setForm({ ...form, [role === "when" ? "whenV" : "thenV"]: ev.target.value })} />
              </label>
            ))}
          </div>
          {error && <p className="print-note bad">{error}</p>}
          <div className="row-buttons">
            <button type="button" onClick={add}>
              Add interlock
            </button>
            <button type="button" className="secondary" onClick={() => (setForm(null), setError(null))}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
