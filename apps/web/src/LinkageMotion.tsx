// A linkage moving: the engine solves every body's pose through the drive's
// range (joint() in the program); this plays it in the view, scrubs it, and
// shows each joint's value through the cycle, with the speeds and
// accelerations it reaches at the drive's real speed.

import { useEffect, useMemo, useRef, useState } from "react";
import { type PartStats, projectUrl } from "./engine";
import { Icon } from "./icons";
import type { MotionGrab, StressView, TurnBody } from "./Viewport";

type Sweep = {
  bodies: string[][];
  joints: { name: string; kind: "hinge" | "slider" | "cylinder" | "ball" | "fixed"; unit: string; at: [number, number, number]; axis: [number, number, number]; first_is_ground: boolean }[];
  drive: number | null;
  speed: number;
  freedom: number;
  frames: { drive: number; bodies: [[number, number, number], [number, number, number, number]][]; joints: number[] }[];
  binds_at: number | null;
  notes: string[];
  pairs: Gap[];
  clashes: Gap[];
  forces: { drive: number[]; joints: number[][]; shaking: number[]; mean_power: number; masses: number[]; gravity: boolean } | null;
};
type Gap = { a: string; b: string; gap: number; at: number; joined: boolean };

/** Seconds a full cycle takes on screen. */
const PACES = [
  { label: "Slow", seconds: 12 },
  { label: "Normal", seconds: 4 },
  { label: "Fast", seconds: 1.2 },
];

const n = (v: number, d = 1) => Number(v.toFixed(d)).toLocaleString("en-US");

function slerp(a: number[], b: number[], t: number): [number, number, number, number] {
  let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
  const s = dot < 0 ? -1 : 1;
  dot *= s;
  if (dot > 0.9995) {
    const q = a.map((v, i) => v + (s * b[i] - v) * t);
    const len = Math.hypot(...q);
    return q.map((v) => v / len) as [number, number, number, number];
  }
  const th = Math.acos(dot);
  const [wa, wb] = [Math.sin((1 - t) * th) / Math.sin(th), (s * Math.sin(t * th)) / Math.sin(th)];
  return a.map((v, i) => v * wa + b[i] * wb) as [number, number, number, number];
}

export function LinkageMotion({
  id,
  version,
  parts,
  onTurn,
  onShowStress,
  onGrab,
}: {
  id: string;
  version: unknown;
  parts: PartStats[];
  onTurn: (id: string, bodies: TurnBody[] | null, options?: { ghost?: boolean }) => void;
  onShowStress: (id: string, view: StressView | null) => void;
  onGrab: (spec: MotionGrab | null) => void;
}) {
  const [state, setState] = useState<{ kind: "busy" } | { kind: "done"; sweep: Sweep } | { kind: "error"; text: string }>({ kind: "busy" });
  const [at, setAt] = useState(0); // position through the frames (0 … frames − 1)
  const [playing, setPlaying] = useState(false);
  const [pace, setPace] = useState(1);
  const [active, setActive] = useState(false);
  const [xray, setXray] = useState(true);
  const [gravity, setGravity] = useState(true);
  // A moving part checked for strength at its hardest moment.
  type Check = { result: NonNullable<StressView["result"]> & { safety: number; yield_strength: number; notes: string[] }; at: number; held_by: string; loads: [string, number][] };
  const [checkPart, setCheckPart] = useState("");
  const [strength, setStrength] = useState<{ kind: "busy" } | { kind: "done"; check: Check; material: string; shown: boolean } | { kind: "error"; text: string } | null>(null);
  useEffect(() => {
    setStrength(null);
    onShowStress(id, null);
  }, [id, version, onShowStress]);
  useEffect(() => () => onShowStress(id, null), [id, onShowStress]);
  const checkStrength = async (part: string) => {
    setStrength({ kind: "busy" });
    // Back to as drawn: the map is of the part where it was designed.
    setPlaying(false);
    setActive(false);
    try {
      const r = await fetch(projectUrl(`/motion-stress/${id}?part=${encodeURIComponent(part)}`));
      if (!r.ok) return setStrength({ kind: "error", text: await r.text() });
      const out = (await r.json()) as { check: Check; material: string };
      setStrength({ kind: "done", check: out.check, material: out.material, shown: true });
      onShowStress(id, { faces: [], result: out.check.result });
    } catch {
      setStrength({ kind: "error", text: "The engine did not answer" });
    }
  };
  const dir = useRef(1);

  useEffect(() => {
    let live = true;
    setState({ kind: "busy" });
    fetch(projectUrl(`/motion/${id}?steps=360&gravity=${gravity ? "on" : "off"}`))
      .then(async (r) => (r.ok ? { kind: "done" as const, sweep: (await r.json()) as Sweep } : { kind: "error" as const, text: await r.text() }))
      .catch(() => ({ kind: "error" as const, text: "The engine did not answer" }))
      .then((s) => live && setState(s));
    return () => {
      live = false;
    };
  }, [id, version, gravity]);

  const sweep = state.kind === "done" ? state.sweep : null;
  const last = sweep ? sweep.frames.length - 1 : 0;
  // A full turn wraps round; a range that ends (a slider, or binding) goes back and forth.
  const wraps = !!sweep && sweep.binds_at === null && sweep.drive !== null && sweep.joints[sweep.drive].kind !== "slider" && Math.abs(sweep.frames[last].drive - sweep.frames[0].drive - 360) < 1e-6;

  useEffect(() => {
    if (!playing || !sweep || last < 1) return;
    let raf = 0;
    let prev: number | null = null;
    const step = (now: number) => {
      if (prev !== null) {
        const d = ((now - prev) / 1000 / PACES[pace].seconds) * last;
        setAt((a) => {
          let x = a + d * dir.current;
          if (wraps) return ((x % last) + last) % last;
          if (x > last || x < 0) {
            dir.current *= -1;
            x = Math.max(0, Math.min(last, x));
          }
          return x;
        });
      }
      prev = now;
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [playing, sweep, last, pace, wraps]);

  const groups = useMemo(() => (sweep ? sweep.bodies.map((b) => b.map((name) => parts.findIndex((p) => p.name === name)).filter((i) => i >= 0)) : []), [sweep, parts]);
  const pose = useMemo(() => {
    if (!sweep || !sweep.frames.length) return null;
    const i = Math.min(Math.floor(at), last);
    const j = Math.min(i + 1, last);
    const t = at - i;
    const [a, b] = [sweep.frames[i], sweep.frames[j]];
    return {
      drive: a.drive + (b.drive - a.drive) * t,
      joints: a.joints.map((v, k) => v + (b.joints[k] - v) * t),
      bodies: a.bodies.map(([pa, qa], k) => {
        const [pb, qb] = b.bodies[k];
        return { position: pa.map((v, c) => v + (pb[c] - v) * t) as [number, number, number], quaternion: slerp(qa, qb, t) };
      }),
    };
  }, [sweep, at, last]);

  useEffect(() => {
    if (!active || !pose) {
      onTurn(id, null);
      return;
    }
    onTurn(
      id,
      pose.bodies.map((b, k) => ({ groups: groups[k] ?? [], ...b })).filter((b) => b.groups.length),
      { ghost: xray },
    );
  }, [id, active, pose, groups, onTurn, xray]);
  useEffect(() => () => onTurn(id, null), [id, onTurn]);

  // Grab a moving part in the view and drag: the drive follows the hand.
  useEffect(() => {
    if (!sweep || sweep.drive === null || last < 1) {
      onGrab(null);
      return;
    }
    const j = sweep.joints[sweep.drive];
    const step = sweep.frames[1].drive - sweep.frames[0].drive;
    const sign = j.first_is_ground ? -1 : 1;
    onGrab({
      id,
      groups: groups.flat(),
      at: j.at,
      axis: j.axis,
      slide: j.kind === "slider",
      onStart: () => {
        setActive(true);
        setPlaying(false);
      },
      onDelta: (d) =>
        setAt((a) => {
          const x = a + (d * sign) / step;
          return wraps ? ((x % last) + last) % last : Math.max(0, Math.min(last, x));
        }),
    });
    return () => onGrab(null);
  }, [sweep, groups, wraps, last, id, onGrab]);

  // Speeds and accelerations at the drive's real speed (rpm for a turn, mm/s for a slide).
  const motion = useMemo(() => {
    if (!sweep || sweep.drive === null || sweep.frames.length < 3) return null;
    const turning = sweep.joints[sweep.drive].kind !== "slider";
    const step = sweep.frames[1].drive - sweep.frames[0].drive; // degrees or mm per frame
    const dt = turning ? step / (sweep.speed * 6) : step / sweep.speed; // seconds per frame
    return sweep.joints.map((j, k) => {
      if (k === sweep.drive || j.kind === "ball" || j.kind === "fixed") return null;
      const v = sweep.frames.map((f) => f.joints[k]);
      let vmax = 0;
      let amax = 0;
      for (let i = 1; i < v.length - 1; i++) {
        vmax = Math.max(vmax, Math.abs(v[i + 1] - v[i - 1]) / (2 * dt));
        amax = Math.max(amax, Math.abs(v[i + 1] - 2 * v[i] + v[i - 1]) / (dt * dt));
      }
      return { v, min: Math.min(...v), max: Math.max(...v), vmax, amax };
    });
  }, [sweep]);

  if (state.kind === "busy") return <p className="motion-note">Solving the motion…</p>;
  if (state.kind === "error") return <p className="print-note bad">{state.text}</p>;
  if (!sweep || !pose) return null;
  const driveJoint = sweep.drive !== null ? sweep.joints[sweep.drive] : null;
  const turning = driveJoint && driveJoint.kind !== "slider";
  return (
    <div className="gear-turn linkage">
      <div className="gear-turn-row">
        <button
          type="button"
          className="gear-play"
          aria-label={playing ? "Pause" : "Run it"}
          title={playing ? "Pause" : "Run it"}
          disabled={last < 1}
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
          max={last}
          step={0.01}
          value={at}
          disabled={last < 1}
          aria-label={`${driveJoint?.name ?? "Drive"} position`}
          onChange={(e) => {
            setActive(true);
            setPlaying(false);
            setAt(Number(e.target.value));
          }}
        />
        <output className="gear-turn-angle">
          {n(pose.drive, turning ? 0 : 1)}
          {driveJoint?.unit}
        </output>
      </div>
      <div className="gear-turn-row gear-turn-sub">
        <div className="segmented" role="group" aria-label="Playback speed">
          {PACES.map((p, i) => (
            <button key={p.label} type="button" className={i === pace ? "active" : ""} aria-pressed={i === pace} onClick={() => setPace(i)}>
              {p.label}
            </button>
          ))}
        </div>
        <label className="xray-toggle" title="Show the parts that stay still see-through while it moves">
          <input type="checkbox" checked={xray} onChange={(e) => setXray(e.target.checked)} /> See inside
        </label>
        {active && (
          <button
            type="button"
            className="gear-turn-reset"
            onClick={() => {
              setPlaying(false);
              setAt(0);
              setActive(false);
            }}
          >
            As drawn
          </button>
        )}
      </div>
      <p className="motion-note">Or grab a moving part in the view and turn it by hand.</p>
      <ul className="joint-list">
        {sweep.joints.map((j, k) => {
          const m = motion?.[k];
          const value = pose.joints[k];
          return (
            <li key={j.name}>
              <div className="joint-head">
                <span className="joint-name">
                  {j.name}
                  <em>{k === sweep.drive ? "drives" : j.kind}</em>
                </span>
                {sweep.forces && (
                  <span className="joint-load" title="The most force this joint carries through the cycle">
                    {n(Math.max(...sweep.forces.joints[k]), 0)} N max
                  </span>
                )}
                {j.kind !== "ball" && j.kind !== "fixed" && (
                  <span className="joint-value">
                    {n(value, j.unit === "mm" ? 2 : 1)}
                    {j.unit === "mm" ? " mm" : "°"}
                  </span>
                )}
              </div>
              {m && m.max - m.min > 1e-6 && (
                <>
                  <svg className="joint-plot" viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden="true">
                    <polyline points={m.v.map((v, i) => `${(i / (m.v.length - 1)) * 100},${22 - ((v - m.min) / (m.max - m.min)) * 20}`).join(" ")} />
                    <line x1={(at / last) * 100} x2={(at / last) * 100} y1="0" y2="24" />
                  </svg>
                  <small className="joint-facts">
                    {n(m.min, 2)} to {n(m.max, 2)} {j.unit} · at {n(sweep.speed, 0)} {turning ? "rpm" : "mm/s"}: up to {j.unit === "mm" ? `${n(m.vmax / 1000, 2)} m/s, ${n(m.amax / 1000, 0)} m/s²` : `${n(m.vmax / 6, 0)} rpm, ${n((m.amax * Math.PI) / 180, 0)} rad/s²`}
                  </small>
                </>
              )}
            </li>
          );
        })}
      </ul>
      {sweep.forces && driveJoint && (() => {
        const f = sweep.forces;
        const t = f.drive;
        const [lo, hi] = [Math.min(0, ...t), Math.max(0, ...t)];
        const y = (v: number) => 22 - ((v - lo) / (hi - lo || 1)) * 20;
        const unit = turning ? "N·m" : "N";
        const k = Math.min(Math.round(at), t.length - 1);
        return (
          <div className="force-block">
            <div className="joint-head">
              <span className="joint-name">
                {turning ? "Torque" : "Force"} at {driveJoint.name}
                <em>{n(sweep.speed, 0)} {turning ? "rpm" : "mm/s"}</em>
              </span>
              <span className="joint-value">
                {n(t[k], 2)} {unit}
              </span>
            </div>
            <svg className="joint-plot" viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden="true">
              <line className="zero" x1="0" x2="100" y1={y(0)} y2={y(0)} />
              <polyline points={t.map((v, i) => `${(i / (t.length - 1)) * 100},${y(v)}`).join(" ")} />
              <line x1={(at / last) * 100} x2={(at / last) * 100} y1="0" y2="24" />
            </svg>
            <small className="joint-facts">
              {n(lo, 2)} to {n(hi, 2)} {unit} · average power {n(f.mean_power, 1)} W · shaking force up to {n(Math.max(...f.shaking), 0)} N · moving mass {n(f.masses.reduce((a, b) => a + b, 0), 3)} kg
            </small>
            <label className="xray-toggle" title="Include the parts' weight (gravity down the z axis)">
              <input type="checkbox" checked={gravity} onChange={(e) => setGravity(e.target.checked)} /> Gravity
            </label>
          </div>
        );
      })()}
      {sweep.forces && (() => {
        // The moving parts (the engine says if one has too few joints to check).
        const movers = sweep.bodies.flat();
        const pick = checkPart || movers[0] || "";
        const s = strength;
        return (
          <div className="force-block">
            <div className="joint-head">
              <span className="joint-name">Strength at its hardest moment</span>
            </div>
            <div className="strength-row">
              <select value={pick} onChange={(e) => (setCheckPart(e.target.value), setStrength(null), onShowStress(id, null))} aria-label="Which moving part">
                {movers.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
              <button type="button" className="secondary" disabled={!pick || s?.kind === "busy"} onClick={() => checkStrength(pick)}>
                <Icon name="weight" /> {s?.kind === "busy" ? "Checking…" : "Check"}
              </button>
            </div>
            {s?.kind === "error" && <p className="print-note bad">{s.text}</p>}
            {s?.kind === "done" && (
              <>
                <div className={`stress-verdict ${s.check.result.safety >= 2 ? "ok" : s.check.result.safety >= 1.2 ? "warn" : "bad"}`}>
                  <b>Safety ×{n(s.check.result.safety, 2)}</b> · {n(s.check.result.max_stress, 1)} MPa at {n(s.check.at, turning ? 0 : 1)}
                  {driveJoint?.unit} ({s.material}, yield {n(s.check.result.yield_strength, 0)} MPa)
                </div>
                <small className="joint-facts">
                  Held at {s.check.held_by}; {s.check.loads.map(([j, f]) => `${j} ${n(f, 0)} N`).join(", ")}. {s.check.result.notes.slice(-1)[0]}
                </small>
                <label className="xray-toggle">
                  <input
                    type="checkbox"
                    checked={s.shown}
                    onChange={(e) => {
                      setStrength({ ...s, shown: e.target.checked });
                      onShowStress(id, e.target.checked ? { faces: [], result: s.check.result } : null);
                    }}
                  />{" "}
                  Show the stress map
                </label>
              </>
            )}
          </div>
        );
      })()}
      {(() => {
        // Jump the view to where a pair comes closest.
        const show = (g: Gap) => {
          const k = sweep.frames.findIndex((f) => Math.abs(f.drive - g.at) < 1e-6);
          if (k < 0) return;
          setActive(true);
          setPlaying(false);
          setAt(k);
        };
        const where = (g: Gap) => `${n(g.at, turning ? 0 : 1)}${driveJoint?.unit ?? ""}`;
        const near = sweep.pairs.find((g) => !g.joined && g.gap >= 1e-4);
        const fits = sweep.pairs.filter((g) => g.joined && g.gap >= 1e-4);
        if (sweep.clashes.length)
          return (
            <div className="gear-verdict bad">
              <b>Clash</b>
              {sweep.clashes.map((g) => (
                <button key={g.a + g.b} type="button" className="clash-jump" onClick={() => show(g)} title="Show this position">
                  {g.a} hits {g.b} at {where(g)}
                </button>
              ))}
            </div>
          );
        return (
          <p className="gear-verdict ok">
            <b>Runs cleanly</b> · {sweep.frames.length} positions
            {near && (
              <>
                {" "}
                · closest{" "}
                <button type="button" className="clash-jump" onClick={() => show(near)} title="Show this position">
                  {n(near.gap, 2)} mm ({near.a} ↔ {near.b}, at {where(near)})
                </button>
              </>
            )}
            {fits.length > 0 && ` · ${fits.length} joints run at ${n(Math.min(...fits.map((g) => g.gap)), 2)}–${n(Math.max(...fits.map((g) => g.gap)), 2)} mm`}
          </p>
        );
      })()}
      <p className={`motion-note ${sweep.binds_at !== null || sweep.freedom !== 1 ? "warn" : ""}`}>
        {sweep.freedom === 1 ? "One way to move" : `${sweep.freedom} ways to move`}
        {driveJoint ? `, driven by ${driveJoint.name}` : ""}
        {sweep.notes.length ? ` · ${sweep.notes.join(" ")}` : " · solved exactly at every step"}
      </p>
    </div>
  );
}
