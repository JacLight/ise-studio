// The fit report of an assembly, in an instance's properties: does every
// bought part actually fit where it is assembled — screws in their holes
// (clearance, thread engagement, length, head seat, hex key reach), inserts,
// nut traps, bearings and pins, joined parts seated without overlap, nothing
// interfering at rest and, if asked, through each joint's motion. Each item
// is pass / warn / fail with what was measured; clicking one selects the parts.

import { useEffect, useState } from "react";
import { projectUrl } from "./engine";

type Status = "pass" | "warn" | "fail";
type Check = { what: string; status: Status; text: string; measured?: number; unit?: string; target?: string; part?: string };
type Item = { kind: string; title: string; status: Status; instances: string[]; names: string[]; joint?: string; checks: Check[] };
type Profile = { printer: string; material: string; layer: number; clearance: number; layer_gap: number; slide: [number, number]; press: number; insert_wall: number };
export type FitReport = { name: string; profile: Profile; items: Item[]; pass: number; warn: number; fail: number; ok: boolean; notes: string[] };

const KINDS: Record<string, string> = {
  screw: "Screw",
  insert: "Insert",
  nut: "Nut",
  washer: "Washer",
  bearing: "Bearing",
  pin: "Pin",
  joined: "Joined",
  interference: "Overlap",
  motion: "Motion",
};

export function FitCheck(props: { instance: string; onSelect: (id: string) => void }) {
  const [state, setState] = useState<{ busy: boolean; report?: FitReport; error?: string; motion: boolean }>({ busy: false, motion: false });
  const [open, setOpen] = useState<string | null>(null);
  const [show, setShow] = useState<"problems" | "all">("problems");
  useEffect(() => setState((s) => ({ busy: false, motion: s.motion })), [props.instance]);

  const run = async (motion: boolean) => {
    setState((s) => ({ ...s, busy: true, error: undefined, motion }));
    try {
      const r = await fetch(projectUrl(`/fit/${props.instance}?motion=${motion ? "on" : "off"}&steps=24`));
      if (!r.ok) throw new Error(await r.text());
      const report = (await r.json()) as FitReport;
      setState({ busy: false, report, motion });
    } catch (e) {
      setState((s) => ({ ...s, busy: false, error: String(e instanceof Error ? e.message : e) }));
    }
  };

  const r = state.report;
  const items = r ? r.items.filter((i) => show === "all" || i.status !== "pass") : [];
  return (
    <section className="props-section fit">
      <header>
        <h3>Fit</h3>
        <span className="section-note">{r ? `${r.items.length} checked` : "screws, inserts, bearings, overlaps"}</span>
      </header>
      <div className="fit-actions">
        <button type="button" className="fit-run" disabled={state.busy} onClick={() => run(false)}>
          {state.busy && !state.motion ? "Checking…" : r ? "Check again" : "Check fit"}
        </button>
        <button type="button" className="fit-run quiet" disabled={state.busy} onClick={() => run(true)} title="Also sweep every moving joint through its limits (slower)">
          {state.busy && state.motion ? "Sweeping…" : "With motion"}
        </button>
      </div>
      {state.error && <p className="fit-error">{state.error}</p>}
      {r && (
        <>
          <div className={`fit-summary ${r.ok ? "ok" : "bad"}`}>
            <span className="fit-chip pass">{r.pass} pass</span>
            <span className="fit-chip warn">{r.warn} warn</span>
            <span className="fit-chip fail">{r.fail} fail</span>
          </div>
          <p className="fit-profile">
            {r.profile.printer} · {r.profile.material} · {r.profile.layer} mm layers · slide {r.profile.slide[0]}–{r.profile.slide[1]} mm · press {r.profile.press} mm · insert wall {r.profile.insert_wall} mm
          </p>
          <div className="fit-filter">
            <button type="button" className={show === "problems" ? "on" : ""} onClick={() => setShow("problems")}>
              Problems
            </button>
            <button type="button" className={show === "all" ? "on" : ""} onClick={() => setShow("all")}>
              All {r.items.length}
            </button>
          </div>
          {items.length === 0 && <p className="fit-empty">{r.ok ? "Everything fits: no warnings or failures." : "No problems in this list."}</p>}
          <ul className="fit-list">
            {items.map((it, n) => {
              const key = `${n}:${it.title}`;
              const expanded = open === key;
              return (
                <li key={key} className={`fit-item ${it.status} ${expanded ? "open" : ""}`}>
                  <button
                    type="button"
                    className="fit-row"
                    onClick={() => {
                      setOpen(expanded ? null : key);
                      if (it.instances[0]) props.onSelect(it.instances[0]);
                    }}
                  >
                    <span className={`fit-dot ${it.status}`} />
                    <span className="fit-kind">{KINDS[it.kind] ?? it.kind}</span>
                    <span className="fit-title">{it.title}</span>
                  </button>
                  {expanded && (
                    <ul className="fit-checks">
                      {it.checks.map((c, k) => (
                        <li key={k} className={c.status}>
                          <span className={`fit-dot ${c.status}`} />
                          <span>
                            <b>{c.what}</b> {c.text}
                            {c.target ? <small> (wants {c.target})</small> : null}
                          </span>
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              );
            })}
          </ul>
          {r.notes.map((n) => (
            <p key={n} className="fit-note">
              {n}
            </p>
          ))}
        </>
      )}
    </section>
  );
}
