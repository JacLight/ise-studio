// Requirements: what the design must satisfy, verified by the engine with
// exact math on every change. Failures come with a one-click fix.

import type { CSSProperties } from "react";
import type { EntityView, Finding, Fix, RequirementResult } from "./engine";
import { type IconName, Icon, IconButton } from "./icons";
import { type Rule, requirement, targetsOf } from "./intent";
import { formatMass, partName } from "./model";
import { ScrubField } from "./ScrubField";

type Preview = Pick<RequirementResult, "passed" | "measured" | "limit" | "unit" | "upper_bound" | "summary">;

const KINDS: { kind: Rule["kind"]; icon: IconName; title: string; detail: string }[] = [
  { kind: "max_mass", icon: "weight", title: "Max mass", detail: "Stay under a weight" },
  { kind: "fits_within", icon: "envelope", title: "Fits within", detail: "Stay inside an envelope" },
  { kind: "clearance", icon: "ruler", title: "Clearance", detail: "Keep two parts apart" },
  { kind: "no_collisions", icon: "collide", title: "No collisions", detail: "Nothing overlaps" },
];

const OTHER_ICONS: Partial<Record<string, IconName>> = { scan_deviation: "scan" };
const iconFor = (kind: Rule["kind"]) => KINDS.find((k) => k.kind === kind)?.icon ?? OTHER_ICONS[kind] ?? "shield";

function Ring({ passing, total }: { passing: number; total: number }) {
  const r = 17;
  const c = 2 * Math.PI * r;
  const share = total ? passing / total : 0;
  const ok = total > 0 && passing === total;
  return (
    <svg className={`ring ${ok ? "ok" : total ? "bad" : ""}`} viewBox="0 0 44 44" aria-hidden>
      <circle cx="22" cy="22" r={r} className="ring-track" />
      <circle cx="22" cy="22" r={r} className="ring-fill" strokeDasharray={`${c * share} ${c}`} transform="rotate(-90 22 22)" />
      <text x="22" y="26.5" textAnchor="middle">
        {total ? `${passing}/${total}` : "—"}
      </text>
    </svg>
  );
}

/** Bar with a limit marker. Upper bounds fill toward the marker; minimums need to reach it. */
function Gauge({ r }: { r: Preview }) {
  if (r.unit === "collisions") return null;
  const span = r.upper_bound ? r.limit * 1.25 : r.limit * 2;
  const fill = Math.min(1, Math.max(0, r.measured / (span || 1)));
  const marker = Math.min(1, r.limit / (span || 1));
  return (
    <div className={`gauge ${r.passed ? "ok" : "bad"}`} style={{ "--fill": fill, "--marker": marker } as CSSProperties}>
      <span className="gauge-fill" />
      <span className="gauge-marker" />
    </div>
  );
}

export function RequirementsPanel(props: {
  requirements: EntityView[];
  entities: ReadonlyMap<string, EntityView>;
  previews: Map<string, Preview>;
  findings: Finding[];
  checking: boolean;
  focus: string | null;
  onFocus: (id: string | null) => void;
  onSelectPart: (id: string) => void;
  onCreate: (kind: Rule["kind"]) => void;
  onUpdate: (id: string, rule: Rule) => void;
  onDelete: (id: string) => void;
  onFix: (fix: Fix) => void;
  onCheck: () => void;
}) {
  const { requirements } = props;
  const verdict = (e: EntityView) => props.previews.get(e.id) ?? e.result;
  const passing = requirements.filter((e) => verdict(e)?.passed).length;
  const others = props.findings.filter((f) => !f.code.startsWith("REQUIREMENT_"));
  const name = (id: string) => {
    const e = props.entities.get(id);
    return e ? partName(e) : "Deleted part";
  };

  return (
    <div className="reqs">
      <div className="reqs-summary">
        <Ring passing={passing} total={requirements.length} />
        <div>
          <strong>
            {requirements.length === 0
              ? "No requirements yet"
              : passing === requirements.length
                ? "All requirements met"
                : `${requirements.length - passing} of ${requirements.length} failing`}
          </strong>
          <small>Checked by exact math on every change</small>
        </div>
        <IconButton icon="refresh" label="Run the full check now" onClick={props.onCheck} className={props.checking ? "spinning" : ""} />
      </div>

      {requirements.map((e) => {
        const req = requirement(e)!;
        const preview = props.previews.get(e.id);
        const r = verdict(e);
        const blocked = e.status && (e.status.state === "blocked" || e.status.state === "failed");
        const state = blocked ? "blocked" : r?.passed ? "met" : "failing";
        const update = (patch: Partial<Rule>) => props.onUpdate(e.id, { ...req.rule, ...patch } as Rule);
        return (
          <article
            key={e.id}
            className={`req-card ${state} ${props.focus === e.id ? "focus" : ""}`}
            onMouseEnter={() => props.onFocus(e.id)}
            onMouseLeave={() => props.onFocus(null)}
          >
            <header>
              <span className="req-icon">
                <Icon name={iconFor(req.rule.kind)} />
              </span>
              <div className="req-title">
                <strong>{req.name}</strong>
                <div className="req-targets">
                  {targetsOf(req.rule).slice(0, 3).map((id) => (
                    <button type="button" key={id} className="target-chip" title={`Select ${name(id)}, a part this requirement checks`} onClick={() => props.onSelectPart(id)}>
                      {name(id)}
                    </button>
                  ))}
                  {targetsOf(req.rule).length > 3 && <span className="target-more">+{targetsOf(req.rule).length - 3}</span>}
                </div>
              </div>
              <span className={`req-state ${state}`}>
                <Icon name={state === "met" ? "check" : "alert"} />
                {state === "met" ? "Met" : state === "blocked" ? "Blocked" : "Failing"}
                {preview && <i>live</i>}
              </span>
              <IconButton icon="trash" label={`Delete “${req.name}”`} onClick={() => props.onDelete(e.id)} className="req-delete" />
            </header>

            {r && <Gauge r={r} />}
            <p className="req-summary">{blocked ? e.status!.reason : r?.summary ?? "Evaluating…"}</p>

            {req.rule.kind === "max_mass" && (
              <div className="req-limit one">
                <ScrubField
                  label="Mass limit"
                  chip="≤"
                  value={req.rule.limit.value * 1000}
                  unit="g"
                  step={10}
                  min={0.1}
                  onLive={() => undefined}
                  onCommit={(v) => update({ limit: { value: v, unit: "g" } })}
                />
                <span className="req-hint">{formatMass(req.rule.limit.value)} max</span>
              </div>
            )}
            {req.rule.kind === "fits_within" && (
              <div className="req-limit">
                {(["width", "depth", "height"] as const).map((k) => (
                  <ScrubField
                    key={k}
                    label={`Envelope ${k}`}
                    chip={k[0].toUpperCase()}
                    value={(req.rule as Extract<Rule, { kind: "fits_within" }>)[k].value * 1000}
                    min={0.1}
                    onLive={() => undefined}
                    onCommit={(v) => update({ [k]: { value: v, unit: "mm" } } as Partial<Rule>)}
                  />
                ))}
              </div>
            )}
            {req.rule.kind === "clearance" && (
              <div className="req-limit one">
                <ScrubField
                  label="Minimum clearance"
                  chip="≥"
                  value={req.rule.min.value * 1000}
                  step={0.5}
                  min={0}
                  onLive={() => undefined}
                  onCommit={(v) => update({ min: { value: v, unit: "mm" } })}
                />
                <span className="req-hint">minimum gap</span>
              </div>
            )}

            {!blocked && !r?.passed && e.result?.fix && !preview && (
              <button type="button" className="fix" title={`Change the design so ${req.name} passes: ${e.result.fix.label}`} onClick={() => props.onFix(e.result!.fix!)}>
                <Icon name="spark" />
                <span>{e.result.fix.label}</span>
              </button>
            )}
          </article>
        );
      })}

      <div className="add-req">
        <h3>Add a requirement</h3>
        <div className="kind-grid">
          {KINDS.map((k) => (
            <button type="button" key={k.kind} className="kind" title={`Add a requirement: ${k.detail.toLowerCase()} (it starts from the design as it is, so it passes)`} onClick={() => props.onCreate(k.kind)}>
              <span className="kind-icon">
                <Icon name={k.icon} />
              </span>
              <strong>{k.title}</strong>
              <small>{k.detail}</small>
            </button>
          ))}
        </div>
        <p className="small muted">New requirements start from your current design so they pass; tighten them by dragging the limits.</p>
      </div>

      {others.length > 0 && (
        <div className="also">
          <h3>Also noticed</h3>
          {others.map((f, i) => (
            <div key={i} className={`finding ${f.severity}`}>
              <Icon name={f.severity === "info" ? "check" : "alert"} />
              <div>
                <p>{f.message}</p>
                {f.fix && (
                  <button type="button" className="fix small-fix" title={`Change the design: ${f.fix.label}`} onClick={() => props.onFix(f.fix!)}>
                    <Icon name="spark" /> <span>{f.fix.label}</span>
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
