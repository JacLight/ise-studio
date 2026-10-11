// The gear check of a part that moves: turn the train through a full turn and
// read every mesh's ratio, contact ratio, backlash and tooth strength.

import { useEffect, useRef, useState } from "react";
import { type Motion, type PartStats, projectUrl } from "./engine";
import { Icon } from "./icons";
import { LinkageMotion } from "./LinkageMotion";
import type { MotionGrab, StressView, TurnBody } from "./Viewport";

type Pair = { a: string; b: string; gap: number; at: number };
type Strength = {
  torque: number;
  rpm: number;
  tangential_force: number;
  bending: [number, number];
  bending_safety: [number | null, number | null];
  contact: number | null;
  contact_safety: number | null;
  materials: [string, string];
};
type Mesh = {
  driver: string;
  driven: string;
  teeth: [number, number];
  module: number;
  ratio: number;
  contact_ratio: number;
  center_distance: number;
  standard_center_distance: number;
  backlash: number;
  strength: Strength | null;
  notes: string[];
};
type Check = { sweep: { driver: string[]; steps: number; pairs: Pair[]; clashes: Pair[]; search: number; resolution: number }; meshes: Mesh[]; speeds: [string[], number][] };

const n = (v: number, d = 2) => Number(v.toFixed(d)).toLocaleString("en-US");
const tone = (sf: number | null | undefined) => (sf == null ? "" : sf >= 2 ? "ok" : sf >= 1.2 ? "warn" : "bad");

const RPMS = [5, 10, 30];

/**
 * Turns the mechanism in the view: play it, or scrub the driving gear round,
 * and every hinged body follows at its gear ratio, with a dial for each.
 */
function Turn({ id, version, motion, parts, onTurn }: { id: string; version: unknown; motion: Motion; parts: PartStats[]; onTurn: (id: string, bodies: TurnBody[] | null) => void }) {
  // The driving gear's angle (degrees, unwrapped so playing never jumps).
  const [angle, setAngle] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [rpm, setRpm] = useState(10);
  const [active, setActive] = useState(false);
  const last = useRef<number | null>(null);

  useEffect(() => {
    if (!playing) return;
    let raf = 0;
    const step = (now: number) => {
      if (last.current !== null) setAngle((a) => a + ((now - last.current!) / 1000) * rpm * 6);
      last.current = now;
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => {
      cancelAnimationFrame(raf);
      last.current = null;
    };
  }, [playing, rpm]);

  const bodies = motion.bodies.map((b) => ({ ...b, groups: b.parts.map((name) => parts.findIndex((p) => p.name === name)).filter((i) => i >= 0) }));
  useEffect(() => {
    if (!active) {
      onTurn(id, null);
      return;
    }
    onTurn(
      id,
      bodies.filter((b) => b.groups.length && b.speed !== 0).map((b) => ({ groups: b.groups, at: b.at, axis: b.axis, degrees: angle * b.speed })),
    );
    // bodies follows motion/parts, which change with version.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, active, angle, version, motion, parts, onTurn]);
  // Leaving the part (or this panel) puts it back as drawn.
  useEffect(() => () => onTurn(id, null), [id, onTurn]);

  const wrap = (a: number) => ((a % 360) + 360) % 360;
  const back = () => {
    setPlaying(false);
    setAngle(0);
    setActive(false);
  };
  return (
    <div className="gear-turn">
      <div className="gear-turn-row">
        <button
          type="button"
          className="gear-play"
          aria-label={playing ? "Pause" : "Turn it"}
          title={playing ? "Pause" : "Turn it"}
          onClick={() => {
            setActive(true);
            setPlaying((p) => !p);
          }}
        >
          <Icon name={playing ? "pause" : "play"} />
        </button>
        <input
          type="range"
          min={0}
          max={360}
          step={1}
          value={Math.round(wrap(angle))}
          aria-label="Driving gear angle"
          onChange={(e) => {
            setActive(true);
            setPlaying(false);
            // Keep whole turns already made, so the slow gears stay where they got to.
            setAngle(Math.floor(angle / 360) * 360 + Number(e.target.value));
          }}
        />
        <output className="gear-turn-angle">{Math.round(wrap(angle))}°</output>
      </div>
      <div className="gear-turn-row gear-turn-sub">
        <div className="segmented" role="group" aria-label="Driving speed">
          {RPMS.map((r) => (
            <button key={r} type="button" className={r === rpm ? "active" : ""} aria-pressed={r === rpm} onClick={() => setRpm(r)}>
              {r} rpm
            </button>
          ))}
        </div>
        {active && (
          <button type="button" className="gear-turn-reset" onClick={back}>
            As drawn
          </button>
        )}
      </div>
      <ul className="gear-dials">
        {bodies.map((b, i) => {
          const deg = angle * b.speed;
          return (
            <li key={b.parts.join("+")}>
              <svg viewBox="-12 -12 24 24" aria-hidden="true">
                <circle r="10.5" />
                <line x1="0" y1="0" x2="0" y2="-8" transform={`rotate(${-deg})`} />
                <circle r="1.6" className="hub" />
              </svg>
              <span className="gear-dial-name" title={b.parts.join(" + ")}>
                {b.parts.join(" + ")}
                {i === motion.driver && <em>drives</em>}
              </span>
              <span className="gear-dial-value">
                {b.speed === 0 ? "still" : `${n(deg % 360, 1)}°`}
                <small>{i === motion.driver ? "×1" : b.speed === 0 ? "" : `×${n(b.speed, 3)}`}</small>
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export function GearCheck({
  id,
  version,
  motion,
  linkage,
  parts,
  onTurn,
  onShowStress,
  onGrab,
}: {
  id: string;
  version: unknown;
  motion?: Motion;
  linkage: boolean;
  parts: PartStats[];
  onTurn: (id: string, bodies: TurnBody[] | null, options?: { ghost?: boolean }) => void;
  onShowStress: (id: string, view: StressView | null) => void;
  onGrab: (spec: MotionGrab | null) => void;
}) {
  const [torque, setTorque] = useState("0.5");
  const [rpm, setRpm] = useState("60");
  const [state, setState] = useState<{ kind: "idle" | "busy" | "done" | "error"; check?: Check; text?: string }>({ kind: "idle" });
  // A new version of the part: the last result is out of date.
  useEffect(() => setState((s) => (s.kind === "done" ? { kind: "idle" } : s)), [version]);

  const run = async () => {
    setState({ kind: "busy" });
    try {
      const r = await fetch(projectUrl(`/gears/${id}?torque=${encodeURIComponent(torque)}&rpm=${encodeURIComponent(rpm)}`), { method: "POST" });
      if (!r.ok) return setState({ kind: "error", text: await r.text() });
      setState({ kind: "done", check: (await r.json()) as Check });
    } catch {
      setState({ kind: "error", text: "The engine did not answer" });
    }
  };

  const c = state.check;
  // Gaps finer than the check can resolve are running fits (a shaft in its bearing).
  const fits = c?.sweep.pairs.filter((p) => p.gap > 0 && p.gap <= c.sweep.resolution) ?? [];
  const smallest = c?.sweep.pairs.find((p) => p.gap > c.sweep.resolution);
  const last = c?.speeds[c.speeds.length - 1];
  return (
    <section className="props-section gear-check">
      <header>
        <h3>Motion</h3>
        <span className="section-note">{linkage ? "Solved exactly at every step" : "Turn it here, or check it through a full turn"}</span>
      </header>
      {linkage ? <LinkageMotion id={id} version={version} parts={parts} onTurn={onTurn} onShowStress={onShowStress} onGrab={onGrab} /> : motion && motion.bodies.length > 0 && <Turn id={id} version={version} motion={motion} parts={parts} onTurn={onTurn} />}
      {(!linkage || (motion && motion.bodies.length > 0)) && (
        <>
      <div className="gear-load">
        <label>
          Input torque
          <span className="unit-field">
            <input value={torque} inputMode="decimal" onChange={(e) => setTorque(e.target.value)} aria-label="Input torque in newton metres" />
            <small>N·m</small>
          </span>
        </label>
        <label>
          Input speed
          <span className="unit-field">
            <input value={rpm} inputMode="decimal" onChange={(e) => setRpm(e.target.value)} aria-label="Input speed in rpm" />
            <small>rpm</small>
          </span>
        </label>
        <button type="button" className="secondary" onClick={run} disabled={state.kind === "busy"}>
          <Icon name="rotate" /> {state.kind === "busy" ? "Turning…" : "Check"}
        </button>
      </div>
      {state.kind === "error" && <p className="print-note bad">{state.text}</p>}
      {c && (
        <div className="gear-results">
          {c.sweep.clashes.length ? (
            <p className="gear-verdict bad">
              <b>Clash</b> · {c.sweep.clashes.map((p) => `${p.a} hits ${p.b} at ${n(p.at, 1)}°`).join(" · ")}
            </p>
          ) : (
            <p className="gear-verdict ok">
              <b>Turns cleanly</b> · {c.sweep.steps} positions
              {smallest ? (
                <>
                  {" "}
                  · closest {n(smallest.gap, 3)} mm ({smallest.a} ↔ {smallest.b}, at {n(smallest.at, 1)}°)
                </>
              ) : (
                ` · nothing within ${c.sweep.search} mm`
              )}
              {fits.length > 0 && (
                <>
                  {" "}
                  · {fits.length} running {fits.length === 1 ? "fit" : "fits"} closer than {n(c.sweep.resolution, 4)} mm, the check's resolution ({fits.slice(0, 3).map((f) => `${f.a} ↔ ${f.b}`).join(", ")}{fits.length > 3 ? ", …" : ""})
                </>
              )}
            </p>
          )}
          {c.meshes.map((m) => (
            <div key={m.driver + m.driven} className="gear-mesh">
              <div className="gear-mesh-head">
                <b>
                  {m.driver} → {m.driven}
                </b>
                <span>
                  {m.teeth[0]}:{m.teeth[1]} · module {n(m.module)}
                </span>
              </div>
              <dl>
                <dt>Ratio</dt>
                <dd>{n(m.ratio, 3)} : 1</dd>
                <dt>Contact ratio</dt>
                <dd className={m.contact_ratio < 1 ? "bad" : m.contact_ratio < 1.2 ? "warn" : ""}>{n(m.contact_ratio, 2)}</dd>
                <dt>Backlash</dt>
                <dd>{n(m.backlash, 2)} mm</dd>
                {m.strength && (
                  <>
                    <dt>Tooth bending</dt>
                    <dd>
                      {m.strength.bending_safety.map((sf, i) => (
                        <span key={i} className={tone(sf)}>
                          {i > 0 && " / "}
                          {sf == null ? "—" : `×${n(sf)}`}
                        </span>
                      ))}
                      <small> safety ({n(m.strength.bending[0], 1)} / {n(m.strength.bending[1], 1)} MPa)</small>
                    </dd>
                    <dt>Tooth surface</dt>
                    <dd>
                      <span className={tone(m.strength.contact_safety)}>{m.strength.contact_safety == null ? "—" : `×${n(m.strength.contact_safety)}`}</span>
                      {m.strength.contact != null && <small> safety ({n(m.strength.contact, 1)} MPa)</small>}
                    </dd>
                  </>
                )}
              </dl>
              {m.notes.map((t) => (
                <p key={t} className="gear-note">
                  {t}
                </p>
              ))}
            </div>
          ))}
          {last && (
            <p className="gear-foot">
              The last stage turns {last[1] === 0 ? "not at all" : `${n(Math.abs(1 / last[1]), 2)}× slower`} than the input{c.meshes.some((m) => m.strength) ? "; strength uses typical figures for each material, so check your material's datasheet before relying on it." : "."}
            </p>
          )}
        </div>
      )}
        </>
      )}
    </section>
  );
}
