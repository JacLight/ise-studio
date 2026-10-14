// Exploded view of an assembly: every part slides out from the assembly's
// centre along the line from it through the part, and back. A way of
// looking, not a change: nothing is saved, the parts stay pickable where
// they show, and a mechanism shown moving keeps moving while exploded.

import { useCallback, useEffect, useRef, useState } from "react";
import * as THREE from "three";
import type { PartStats } from "./engine";
import { Icon } from "./icons";
import type { TurnBody, TurnOptions } from "./Viewport";

type V3 = [number, number, number];

/** What is exploded: which shape, how far apart (spread), and whether it is opening or closing. */
export type Explode = { id: string; spread: number; open: boolean };

const centreOf = (min: V3, max: V3): V3 => [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2];

/**
 * Each part as its own body, pushed out from the assembly's centre by
 * `amount` (0 at rest; 1 doubles each part's distance from the centre, plus
 * a little so parts near the middle still clear it), on top of the pose a
 * mechanism gives it, the whole lifted so nothing sinks below where the
 * assembly stands (under the ground it would be hidden). Also the trail from
 * each part's (posed) home to where it is.
 */
export function explodedBodies(parts: PartStats[], motion: TurnBody[] | null, amount: number): { bodies: TurnBody[]; trails: NonNullable<TurnOptions["trails"]> } {
  const lo: V3 = [Infinity, Infinity, Infinity];
  const hi: V3 = [-Infinity, -Infinity, -Infinity];
  for (const p of parts)
    for (let k = 0; k < 3; k++) {
      lo[k] = Math.min(lo[k], p.bounds_min[k]);
      hi[k] = Math.max(hi[k], p.bounds_max[k]);
    }
  const centre = new THREE.Vector3(...centreOf(lo, hi));
  const diagonal = Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
  const offsets = parts.map((p) => {
    const out = new THREE.Vector3(...centreOf(p.bounds_min, p.bounds_max)).sub(centre);
    return out.length() > diagonal * 0.01 ? out.clone().multiplyScalar(amount).addScaledVector(out.normalize(), diagonal * 0.12 * amount) : new THREE.Vector3();
  });
  const lift = Math.max(0, ...parts.map((p, g) => lo[2] - (p.bounds_min[2] + offsets[g].z)));
  const bodies: TurnBody[] = [];
  const trails: NonNullable<TurnOptions["trails"]> = [];
  parts.forEach((p, g) => {
    const home = new THREE.Vector3(...centreOf(p.bounds_min, p.bounds_max));
    const offset = offsets[g].clone().setZ(offsets[g].z + lift);
    // The mechanism's pose of this part, as a rotation then a shift (p → q·p + t).
    const q = new THREE.Quaternion();
    const t = new THREE.Vector3();
    const m = motion?.find((b) => b.groups.includes(g));
    if (m && "at" in m) {
      q.setFromAxisAngle(new THREE.Vector3(...m.axis).normalize(), THREE.MathUtils.degToRad(m.degrees));
      const at = new THREE.Vector3(...m.at);
      t.copy(at).sub(at.clone().applyQuaternion(q));
    } else if (m) {
      q.set(...m.quaternion);
      t.set(...m.position);
    }
    const posed = home.clone().applyQuaternion(q).add(t);
    bodies.push({ groups: [g], position: t.clone().add(offset).toArray() as V3, quaternion: [q.x, q.y, q.z, q.w] });
    trails.push([posed.toArray() as V3, posed.clone().add(offset).toArray() as V3]);
  });
  return { bodies, trails };
}

const ease = (x: number) => (x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2);
const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/**
 * The view's turning, shared by a mechanism shown moving and an exploded
 * view: `turn` is what panels call to show a mechanism; the explode is laid
 * over it, easing open and shut. `apply` re-shows a shape (after it is rebuilt).
 * `settled` is told when it has finished opening or shutting (to frame it).
 */
export function useExplode(show: (id: string, bodies: TurnBody[] | null, options?: TurnOptions) => void, partsOf: (id: string) => PartStats[], settled?: (id: string) => void) {
  const [explode, setExplode] = useState<Explode | null>(null);
  const motion = useRef(new Map<string, { bodies: TurnBody[]; options?: TurnOptions }>());
  // How far open (0..1, before easing) each exploded shape is now.
  const amount = useRef<{ id: string; t: number } | null>(null);
  // Which split each shape is shown with, so a change of split puts it back first.
  const split = useRef(new Map<string, "motion" | "explode">());
  const latest = useRef({ show, partsOf, explode, settled });
  latest.current = { show, partsOf, explode, settled };
  // Whether the last ease was opening, and the spread last framed: opening or
  // shutting frames at once, a new spread once it stops changing.
  const wasOpen = useRef(false);
  const framedSpread = useRef<number | null>(null);

  const apply = useCallback((id: string) => {
    const { show, partsOf, explode } = latest.current;
    const m = motion.current.get(id) ?? null;
    const a = amount.current?.id === id ? ease(amount.current.t) * (explode?.id === id ? explode.spread : 1) : 0;
    const parts = partsOf(id);
    const was = split.current.get(id);
    if (a < 1e-4 || parts.length < 2) {
      if (was === "explode") show(id, null);
      split.current.set(id, "motion");
      show(id, m?.bodies ?? null, m?.options);
      return;
    }
    if (was !== "explode") show(id, null);
    split.current.set(id, "explode");
    const { bodies, trails } = explodedBodies(parts, m?.bodies ?? null, a);
    show(id, bodies, { ghost: m?.options?.ghost, trails });
  }, []);

  /** What panels call to show a mechanism turned (null puts it back). */
  const turn = useCallback(
    (id: string, bodies: TurnBody[] | null, options?: TurnOptions) => {
      if (bodies) motion.current.set(id, { bodies, options });
      else motion.current.delete(id);
      apply(id);
    },
    [apply],
  );

  // Ease open or shut (about half a second), each frame re-showing the shape.
  useEffect(() => {
    if (!explode) return;
    const { id, open, spread } = explode;
    if (amount.current && amount.current.id !== id) {
      const other = amount.current.id;
      amount.current = null;
      apply(other);
    }
    amount.current ??= { id, t: 0 };
    const target = open ? 1 : 0;
    if (reducedMotion()) amount.current.t = target;
    let raf = 0;
    let timer = 0;
    let last: number | null = null;
    const step = (now: number) => {
      const a = amount.current;
      if (!a || a.id !== id) return;
      // Timed from the first frame and at most a frame's worth per step: in a
      // background tab no frames run, and the ease plays once it is shown
      // rather than jumping to the end.
      const dt = last === null ? 0 : Math.min(now - last, 100);
      last = now;
      a.t = Math.max(0, Math.min(1, a.t + Math.sign(target - a.t) * (dt / 520)));
      apply(id);
      if (a.t !== target) raf = requestAnimationFrame(step);
      else {
        const turned = wasOpen.current !== open;
        wasOpen.current = open;
        if (turned) {
          framedSpread.current = open ? spread : null;
          latest.current.settled?.(id);
        } else if (open && framedSpread.current !== spread) {
          // A new spread (the slider, or Iṣẹ́ AI asking again): frame the
          // parts where they now are once it stops changing.
          timer = window.setTimeout(() => {
            framedSpread.current = spread;
            latest.current.settled?.(id);
          }, 260);
        }
        if (!open) {
          amount.current = null;
          setExplode(null);
        }
      }
    };
    raf = requestAnimationFrame(step);
    return () => {
      cancelAnimationFrame(raf);
      window.clearTimeout(timer);
    };
  }, [explode, apply]);

  return { explode, setExplode, turn, apply };
}

/** The explode toggle and its spread, for an assembly's parts list. */
export function ExplodeControl({ id, explode, onExplode }: { id: string; explode: Explode | null; onExplode: (e: Explode | null) => void }) {
  const mine = explode?.id === id ? explode : null;
  const open = !!mine?.open;
  const spread = mine?.spread ?? 1;
  return (
    <div className={`explode ${open ? "open" : ""}`}>
      <button type="button" className="explode-toggle" aria-pressed={open} onClick={() => onExplode({ id, spread, open: !open })} title={open ? "Put the parts back together" : "Pull the parts apart to see how they go together (a view only: nothing changes)"}>
        <Icon name="explode" />
        {open ? "Exploded" : "Explode"}
      </button>
      {open && (
        <label className="explode-spread">
          <span>Spread</span>
          <input type="range" min={0.25} max={2.5} step={0.05} value={spread} aria-label="How far apart the parts spread" onChange={(e) => onExplode({ id, spread: Number(e.target.value), open: true })} />
          <output>{spread.toFixed(2)}×</output>
        </label>
      )}
    </div>
  );
}
