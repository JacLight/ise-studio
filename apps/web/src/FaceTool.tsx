// Shelling and drafting by hand. Shell: hollow the selected part to a wall
// thickness, clicking the faces to leave open (none: a sealed hollow). Draft:
// taper faces by an angle so the part releases from a mould pulled along an
// axis (no faces picked: every wall along the pull). The engine previews the
// true result live; Apply stores it as a finish sized by a new parameter of
// the part (shell1 = wall, draft1 = angle), editable and linkable later.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FacePicture } from "./faceLayer";
import type { Engine, EngineError, EntityView } from "./engine";
import type { FinishPreview } from "./EdgeEditor";
import { Icon } from "./icons";
import { type EdgeFinish, type EdgePick, program } from "./model";
import type { ProgramFace } from "./Viewport";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
export type FaceToolKind = "shell" | "draft";
type Vec3 = [number, number, number];
type Axis = 0 | 1 | 2;

const AXES = ["X", "Y", "Z"] as const;
const NEUTRAL: { at: number; label: string }[] = [
  { at: 0, label: "Base" },
  { at: 0.5, label: "Middle" },
  { at: 1, label: "Tip" },
];

/** One face picked twice is the same face: same area, same extent. */
function faceKey(f: ProgramFace) {
  const t = f.triangles;
  const lo = [Infinity, Infinity, Infinity];
  const hi = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < t.length; i += 3) {
    for (let k = 0; k < 3; k++) {
      lo[k] = Math.min(lo[k], t[i + k]);
      hi[k] = Math.max(hi[k], t[i + k]);
    }
  }
  return [f.part ?? "", f.area.toFixed(2), ...lo.map((v) => v.toFixed(2)), ...hi.map((v) => v.toFixed(2))].join("|");
}

/**
 * The part's finishes with a new one: last, except that a draft goes before
 * the first shell (as a mould designer works: taper, then hollow, so the
 * walls stay even and the cavity is drafted with them).
 */
function withFinish(list: EdgeFinish[], f: EdgeFinish): EdgeFinish[] {
  const shell = f.kind === "draft" ? list.findIndex((g) => g.kind === "shell" && g.part === f.part) : -1;
  return shell < 0 ? [...list, f] : [...list.slice(0, shell), f, ...list.slice(shell)];
}

function newKey(kind: FaceToolKind, taken: string[]) {
  let n = 1;
  while (taken.includes(`${kind}${n}`)) n++;
  return `${kind}${n}`;
}

export function useFaceTool(props: {
  engine: Engine;
  entities: ReadonlyMap<string, EntityView>;
  run: Run;
  notify: (m: string) => void;
  setPreview: (p: FinishPreview) => void;
}) {
  const { engine, run, setPreview } = props;
  const [editing, setEditing] = useState<string | null>(null);
  const [kind, setKind] = useState<FaceToolKind>("shell");
  const [part, setPart] = useState<string | null>(null);
  const [faces, setFaces] = useState<ProgramFace[]>([]);
  const [wall, setWall] = useState("2");
  const [outward, setOutward] = useState(false);
  const [angle, setAngle] = useState("3");
  const [axis, setAxis] = useState<Axis>(2);
  const [flip, setFlip] = useState(false);
  const [neutral, setNeutral] = useState(0);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  const entity = editing ? props.entities.get(editing) : undefined;
  const prog = entity ? program(entity) : undefined;
  const seq = useRef(0);

  const stop = useCallback(() => {
    seq.current++;
    setEditing(null);
    setFaces([]);
    setProblem(null);
    setPreview(null);
    setPreviewing(false);
  }, [setPreview]);

  const start = useCallback(
    (id: string, k: FaceToolKind, partName: string | null) => {
      const e = props.entities.get(id);
      if (!e || !program(e)) return props.notify(`Select a part made by a program to ${k === "shell" ? "shell" : "draft"} it`);
      setEditing(id);
      setKind(k);
      setPart(partName);
      setFaces([]);
      setProblem(null);
    },
    [props],
  );

  // The part changed under us (undo, another session): picked faces may no longer exist.
  const meshRef = entity?.mesh;
  useEffect(() => {
    setFaces([]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meshRef]);

  const toggle = useCallback((f: ProgramFace) => {
    setFaces((list) => {
      // Faces of one part of an assembly at a time.
      const same = list.filter((g) => g.part === f.part);
      const k = faceKey(f);
      const hit = same.findIndex((g) => faceKey(g) === k);
      if (hit >= 0) return same.filter((_, i) => i !== hit);
      return [...same, f];
    });
    setPart(f.part);
  }, []);

  const num = (s: string, unit: RegExp) => Number(s.replace(unit, "").trim());
  const size = kind === "shell" ? num(wall, /mm$/) : num(angle, /°$|deg$/);
  const sizeOk = Number.isFinite(size) && size > 0 && (kind === "shell" || size < 89);
  const value = kind === "shell" && outward ? -size : size;
  const key = useMemo(() => newKey(kind, prog?.params.map((p) => p.key) ?? []), [kind, prog]);
  const pull = useMemo((): Vec3 => {
    const d: Vec3 = [0, 0, 0];
    d[axis] = flip ? -1 : 1;
    return d;
  }, [axis, flip]);

  /** The part's bounds as the finish will see them (a part of an assembly, or the whole). */
  const bounds = useMemo(() => {
    const s = entity?.shape;
    if (!s) return null;
    const b = part ? s.parts?.find((p) => p.name === part) : s;
    return b ? { min: b.bounds_min as Vec3, max: b.bounds_max as Vec3 } : null;
  }, [entity, part]);

  /** The finish the picks make, sized by one new parameter. */
  const finish = useMemo((): EdgeFinish | null => {
    if (!bounds) return null;
    const edges: EdgePick[] = faces.map((f) => ({
      at: [0, 1, 2].map((k) => {
        const span = bounds.max[k] - bounds.min[k];
        return span > 1e-9 ? Math.round(((f.centroid[k] - bounds.min[k]) / span) * 1e6) / 1e6 : 0.5;
      }) as Vec3,
      kind: f.curved ? "curved_face" : "face",
      dir: f.normal,
    }));
    return { kind, key, ...(part ? { part } : {}), edges, ...(kind === "draft" ? { pull, neutral } : {}) };
  }, [bounds, faces, kind, key, part, pull, neutral]);

  // Live preview of the true result.
  useEffect(() => {
    if (!editing) return;
    const n = ++seq.current;
    if (!finish || !sizeOk) {
      setPreview(null);
      setPreviewing(false);
      setProblem(null);
      return;
    }
    const timer = setTimeout(async () => {
      try {
        const r = await engine.shapePreview(editing, { [key]: value }, withFinish(prog?.finishes ?? [], finish));
        if (n === seq.current) {
          setPreview({ id: editing, stats: r.stats, mesh: r.mesh });
          setPreviewing(true);
          setProblem(null);
        }
      } catch (err) {
        if (n === seq.current) {
          setPreview(null);
          setPreviewing(false);
          setProblem(
            (err as EngineError).message
              .replace(/^program: (Error: )?/, "")
              .replace(/\s+at \S+ \(eval_script[\s\S]*$/, "")
              .replace(/^(shell|draft): /, ""),
          );
        }
      }
    }, 90);
    return () => clearTimeout(timer);
  }, [editing, finish, key, value, sizeOk, engine, prog, setPreview]);

  const apply = useCallback(async () => {
    if (!editing || !prog || !finish || !sizeOk || problem || busy) return;
    const n = Number(key.replace(/^\D+/, ""));
    setBusy(true);
    const param =
      kind === "shell"
        ? { key, label: `Shell ${n} wall`, value, unit: "mm", min: null, max: null, step: 0.5 }
        : { key, label: `Draft ${n} angle`, value, unit: "deg", min: null, max: null, step: 0.5 };
    const ok = await run("shape.update", { target: editing, params: [...prog.params, param], finishes: withFinish(prog.finishes ?? [], finish) });
    setBusy(false);
    if (ok) stop();
  }, [editing, prog, finish, sizeOk, problem, busy, key, kind, value, run, stop]);

  // Esc leaves; Enter applies.
  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        stop();
      } else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) {
        e.preventDefault();
        void apply();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [editing, stop, apply]);

  // What the view draws: the picked faces, and a draft's pull arrow on its neutral plane.
  const picture: FacePicture | null = useMemo(() => {
    if (!editing) return null;
    let pullPic: FacePicture["pull"];
    if (kind === "draft" && bounds) {
      const { min, max } = bounds;
      const c = [0, 1, 2].map((k) => (min[k] + max[k]) / 2) as Vec3;
      const along = min[axis] + (flip ? 1 - neutral : neutral) * (max[axis] - min[axis]);
      const origin = [...c] as Vec3;
      origin[axis] = along;
      const extent = Math.max(...[0, 1, 2].map((k) => max[k] - min[k]));
      pullPic = { dir: pull, origin, size: Math.max(extent, 1) };
    }
    return { id: editing, faces, tone: kind === "shell" ? "open" : "draft", pull: pullPic };
  }, [editing, kind, bounds, axis, flip, neutral, pull, faces]);

  const count = faces.length;
  const what = (n: number) => `${n} ${n === 1 ? "face" : "faces"}`;
  const status = problem
    ? problem
    : kind === "shell"
      ? count
        ? `Open at ${what(count)} · ${outward ? "outward" : "inward"} walls`
        : "Sealed hollow · click faces to open them"
      : count
        ? `Tapering ${what(count)}`
        : `Every wall along ${flip ? "−" : "+"}${AXES[axis]} · click faces to choose`;

  const sizeField = (label: string, val: string, set: (s: string) => void, unit: string, step: number, aria: string) => (
    <label className="edge-size">
      <span>{label}</span>
      <input
        value={val}
        inputMode="decimal"
        aria-label={aria}
        className={sizeOk ? "" : "invalid"}
        onChange={(e) => set(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowUp" || e.key === "ArrowDown") {
            e.preventDefault();
            const s = e.shiftKey ? step * 2 : step;
            set(String(Math.max(s, Math.round(((sizeOk ? size : 0) + (e.key === "ArrowUp" ? s : -s)) * 100) / 100)));
          }
        }}
      />
      <small>{unit}</small>
    </label>
  );

  const bar =
    editing && prog ? (
      <div className="sketch-bar edge-bar face-tool-bar floating" role="toolbar" aria-label={kind === "shell" ? "Shell the part" : "Draft faces"}>
        <div className="sketch-head">
          <strong>{prog.name}</strong>
          <span className={`sketch-status ${problem ? "bad" : previewing ? "ok" : "free"}`}>{status}</span>
        </div>
        <div className="edge-row">
          <div className="segmented" role="radiogroup" aria-label="Tool">
            {(["shell", "draft"] as FaceToolKind[]).map((k) => (
              <button
                key={k}
                type="button"
                role="radio"
                aria-checked={kind === k}
                className={kind === k ? "active" : ""}
                onClick={() => {
                  setKind(k);
                  setFaces([]);
                }}
              >
                <Icon name={k} />
                {k === "shell" ? "Shell" : "Draft"}
              </button>
            ))}
          </div>
          {kind === "shell" ? (
            <>
              {sizeField("Wall", wall, setWall, "mm", 0.5, "Wall thickness in mm")}
              <div className="segmented" role="radiogroup" aria-label="Walls grow">
                {[false, true].map((o) => (
                  <button key={String(o)} type="button" role="radio" aria-checked={outward === o} className={outward === o ? "active" : ""} onClick={() => setOutward(o)}>
                    {o ? "Outward" : "Inward"}
                  </button>
                ))}
              </div>
            </>
          ) : (
            <>
              {sizeField("Angle", angle, setAngle, "°", 0.5, "Draft angle in degrees")}
              <div className="segmented" role="radiogroup" aria-label="Pull direction">
                {([0, 1, 2] as Axis[]).map((a) => (
                  <button key={a} type="button" role="radio" aria-checked={axis === a} className={axis === a ? "active" : ""} onClick={() => setAxis(a)}>
                    {axis === a ? (flip ? "−" : "+") : ""}
                    {AXES[a]}
                  </button>
                ))}
              </div>
              <button type="button" className="link-button" onClick={() => setFlip((f) => !f)} title="Pull the other way">
                Flip pull
              </button>
            </>
          )}
          {count > 0 && (
            <button type="button" className="link-button" onClick={() => setFaces([])}>
              Clear
            </button>
          )}
        </div>
        {kind === "draft" && (
          <div className="edge-row">
            <span className="face-tool-label">Neutral plane</span>
            <div className="segmented" role="radiogroup" aria-label="Neutral plane">
              {NEUTRAL.map((n) => (
                <button key={n.at} type="button" role="radio" aria-checked={neutral === n.at} className={neutral === n.at ? "active" : ""} onClick={() => setNeutral(n.at)}>
                  {n.label}
                </button>
              ))}
            </div>
          </div>
        )}
        <p className="sketch-hint">
          {kind === "shell"
            ? "Click faces to leave open, again to close them · none keeps it sealed · the wall becomes a parameter you can link"
            : "Faces keep their size on the neutral plane and lean in along the pull · click faces to pick them · the angle becomes a parameter you can link"}
          {" · Enter applies, Esc cancels"}
        </p>
        <div className="sketch-actions">
          <button type="button" className="primary" onClick={apply} disabled={!finish || !sizeOk || !!problem || busy}>
            <Icon name="check" /> {kind === "shell" ? `Shell ${count ? `· open ${what(count)}` : "· sealed"}` : `Draft ${count ? what(count) : "all walls"}`}
          </button>
          <button type="button" className="secondary" onClick={stop}>
            Cancel
          </button>
        </div>
      </div>
    ) : null;

  return { editing, kind, picture, toggle, bar, start, stop };
}
