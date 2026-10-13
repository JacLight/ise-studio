// Canvases in the panels: the list in Project objects (show/hide, pick),
// a canvas's properties (plane, look, scale and calibration, where it sits,
// crop), placing reference photos on their planes, calibrating one by two
// clicked points and a true distance, and comparing the model with it (the
// engine's overlay and silhouette match).

import { useEffect, useMemo, useState } from "react";
import { type CanvasData, type CanvasTool, VIEW_LABEL, apply, canvasOf, cornerOrder, cropOf, mmPerPx, sizeMm, toPlane } from "./canvasLayer";
import { Dialog } from "./Dialog";
import { type EntityView, projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import { ScrubField } from "./ScrubField";
import { UNIT_MM, showLength, useDisplayPrefs } from "./units";
import "./CanvasTools.css";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
type V2 = [number, number];

const PLANES = ["front", "back", "left", "right", "top", "bottom"] as const;
const PLANE_HINT: Record<string, string> = {
  front: "seen from the front (−Y): the XZ plane",
  back: "seen from the back (+Y)",
  left: "seen from the left (−X)",
  right: "seen from the right (+X): the YZ plane",
  top: "seen from above (+Z): the XY plane",
  bottom: "seen from below (−Z)",
};

const fmt = (v: number, places = 2) => String(Number(v.toFixed(places)));

/** How a canvas is sized, in a few words. */
function sizing(c: CanvasData) {
  if (c.perspective) return `squared up · ${fmt(c.perspective.width)} × ${fmt(c.perspective.height)} mm`;
  if (c.calibration) return `calibrated · ${fmt(c.calibration.distance)} mm between two points`;
  if (c.sized) return `${fmt(mmPerPx(c), 4)} mm per pixel`;
  return "not calibrated yet";
}

/** The canvases, in Project objects: eye to show or hide, click to open its properties. */
export function CanvasSection(props: { entities: ReadonlyMap<string, EntityView>; selected: string | null; run: Run; onSelect: (id: string) => void; onAdd: () => void }) {
  const list = [...props.entities.values()].filter((e) => canvasOf(e));
  return (
    <div className="canvas-section">
      {list.length === 0 && <p className="muted small">Reference photos or drawings placed on a plane, to model over and to check the model against.</p>}
      {list.map((e) => {
        const c = canvasOf(e)!;
        return (
          <div key={e.id} className={`canvas-row ${props.selected === e.id ? "active" : ""}`}>
            <button type="button" className={`layer-eye ${c.visible ? "" : "off"}`} title={c.visible ? "Hide this canvas" : "Show this canvas"} onClick={() => void props.run("canvas.update", { target: e.id, visible: !c.visible })}>
              <Icon name="eye" />
            </button>
            <button type="button" className="canvas-pick" onClick={() => props.onSelect(e.id)}>
              <img src={projectUrl(`/images/${c.image}`)} alt="" loading="lazy" />
              <span>
                {c.name}
                <small className={`row-count ${c.sized ? "" : "warn"}`}>
                  {VIEW_LABEL[c.view ?? ""] ?? "Custom plane"} · {sizing(c)}
                </small>
              </span>
            </button>
          </div>
        );
      })}
      <button type="button" className="secondary add-part" onClick={props.onAdd}>
        <Icon name="image" /> Place reference photos
      </button>
    </div>
  );
}

type Match = {
  iou: number;
  precision: number;
  recall: number;
  worst_mm: number;
  mean_mm: number;
  photo_size_mm: V2;
  model_size_mm: V2;
  regions: { kind: string; area_mm2: number; depth_mm: number; at: [number, number, number] }[];
};

/** A canvas's properties. */
export function CanvasProperties(props: { entity: EntityView; run: Run; notify: (t: string) => void; onTool: (id: string, tool: CanvasTool) => void; onDeleted: () => void }) {
  const { entity, run } = props;
  const c = canvasOf(entity)!;
  const prefs = useDisplayPrefs();
  const [name, setName] = useState(c.name);
  useEffect(() => setName(c.name), [c.name]);
  const [opacity, setOpacity] = useState(c.opacity);
  useEffect(() => setOpacity(c.opacity), [c.opacity]);
  const [match, setMatch] = useState<{ png: string; m: Match } | null>(null);
  const [matching, setMatching] = useState(false);
  useEffect(() => () => (match ? URL.revokeObjectURL(match.png) : undefined), [match]);
  useEffect(() => setMatch(null), [entity.id]);
  const set = (fields: Record<string, unknown>) => run("canvas.update", { target: entity.id, ...fields });
  const size = useMemo(() => sizeMm(c), [c]);
  const o = c.plane.origin;

  const compare = async () => {
    setMatching(true);
    try {
      const r = await fetch(projectUrl(`/look?canvas=${encodeURIComponent(entity.id)}&size=900x900`));
      if (!r.ok) throw new Error(await r.text());
      const m = JSON.parse(r.headers.get("x-forma-match") ?? "{}") as Match;
      setMatch({ png: URL.createObjectURL(await r.blob()), m });
    } catch (e) {
      props.notify(String((e as Error).message ?? e));
    } finally {
      setMatching(false);
    }
  };

  return (
    <div className="canvas-props">
      <div className="props-head">
        <span className="canvas-thumb" aria-hidden>
          <img src={projectUrl(`/images/${c.image}`)} alt="" />
        </span>
        <div className="props-title">
          <strong>{c.name}</strong>
          <small>
            Canvas · {c.pixels[0]} × {c.pixels[1]} px · reference only (no mass, not exported)
          </small>
        </div>
      </div>
      <label className="field">
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name.trim() && name !== c.name && void set({ name: name.trim() })} onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()} />
      </label>
      <label className="field">
        Plane
        <select value={c.view ?? ""} onChange={(e) => e.target.value && void set({ plane: e.target.value })}>
          {!c.view && <option value="">Custom plane</option>}
          {PLANES.map((p) => (
            <option key={p} value={p}>
              {VIEW_LABEL[p]} — {PLANE_HINT[p]}
            </option>
          ))}
        </select>
      </label>

      <h3 className="panel-section">Size</h3>
      <p className={`canvas-status ${c.sized ? "" : "warn"}`}>
        {c.sized ? <Icon name="check" /> : <Icon name="alert" />}
        <span>
          {sizing(c)}
          <small>
            Picture {showLength(size[0], prefs)} × {showLength(size[1], prefs)} on its plane
          </small>
        </span>
      </p>
      <div className="canvas-actions">
        <button type="button" className="btn primary" onClick={() => props.onTool(entity.id, "calibrate")} title="Click two points on the picture a known distance apart, then type that distance">
          <Icon name="ruler" /> Calibrate…
        </button>
        {c.calibration && (
          <button type="button" className="btn" onClick={() => void set({ reset: ["calibration"] })}>
            Forget calibration
          </button>
        )}
      </div>
      {!c.perspective && (
        <div className="canvas-pair">
          <div className="canvas-labelled">
            <span>Whole picture width</span>
            <ScrubField label="Whole picture width" chip="W" value={c.scale * c.pixels[0]} min={0.01} step={1} onLive={() => undefined} onCommit={(v) => void set({ width: v })} />
          </div>
          <div className="canvas-labelled" title="Up-the-picture scale over across-the-picture scale: stretches a photo foreshortened in one direction">
            <span>Stretch up</span>
            <ScrubField label="Stretch up the picture" chip="×" unit="" value={c.aspect ?? 1} min={0.05} step={0.01} onLive={() => undefined} onCommit={(v) => void set({ aspect: v })} />
          </div>
        </div>
      )}

      <h3 className="panel-section">Placement</h3>
      <p className="small muted canvas-note">Where the picture's anchor point sits in the model (the plane passes through it).</p>
      <div className="canvas-xyz">
        {(["x", "y", "z"] as const).map((k, i) => (
          <ScrubField
            key={k}
            label={`Origin ${k.toUpperCase()}`}
            chip={k.toUpperCase()}
            tone={["#e5484d", "#30a46c", "#3e63dd"][i]}
            value={o[i]}
            onLive={() => undefined}
            onCommit={(v) => void set({ at: o.map((x, j) => (j === i ? v : x)) })}
          />
        ))}
      </div>
      <div className="canvas-labelled">
        <span>Turn in its plane (anticlockwise)</span>
        <ScrubField label="Turn (degrees, anticlockwise)" chip="∠" unit="deg" value={c.rotation ?? 0} step={1} onLive={() => undefined} onCommit={(v) => void set({ rotation: v })} />
      </div>
      <div className="canvas-checks">
        <label className="check">
          <input type="checkbox" checked={!!c.flip} onChange={(e) => void set({ flip: e.target.checked })} /> Mirror
        </label>
        <label className="check">
          <input type="checkbox" checked={c.visible} onChange={(e) => void set({ visible: e.target.checked })} /> Visible
        </label>
        <label className="check" title="Drawn over the model instead of hidden behind it">
          <input type="checkbox" checked={!!c.through} onChange={(e) => void set({ through: e.target.checked })} /> Show through
        </label>
      </div>
      <label className="field canvas-opacity">
        Opacity <b>{Math.round(opacity * 100)}%</b>
        <input type="range" min={0} max={1} step={0.05} value={opacity} onChange={(e) => setOpacity(Number(e.target.value))} onPointerUp={() => opacity !== c.opacity && void set({ opacity })} onKeyUp={() => opacity !== c.opacity && void set({ opacity })} />
      </label>
      <h3 className="panel-section">Picture</h3>
      <p className="small muted canvas-note">
        {c.crop ? `Cropped to ${Math.round(c.crop[2] - c.crop[0])} × ${Math.round(c.crop[3] - c.crop[1])} px` : "The whole picture counts"}
        {c.perspective ? ` · squared up from a ${fmt(c.perspective.width)} × ${fmt(c.perspective.height)} mm rectangle` : " · as taken"}
      </p>
      <div className="canvas-actions">
        <button type="button" className="btn" onClick={() => props.onTool(entity.id, "crop")} title="Drag a rectangle over the picture in the view: only that part shows and counts (for tracing and matching)">
          <Icon name="crop" /> {c.crop ? "Crop again…" : "Crop…"}
        </button>
        <button type="button" className="btn" onClick={() => props.onTool(entity.id, "perspective")} title="Click the four corners of something rectangular in the picture and type its true width and height: the photo is squared up and sized from it">
          <Icon name="perspective" /> {c.perspective ? "Square up again…" : "Square up…"}
        </button>
      </div>
      {(c.crop || c.perspective) && (
        <div className="canvas-actions">
          {c.crop && (
            <button type="button" className="btn" onClick={() => void set({ reset: ["crop"] })} title={`Cropped to pixels ${c.crop.map((v) => Math.round(v)).join(", ")}: show and count the whole picture again`}>
              Show the whole picture
            </button>
          )}
          {c.perspective && (
            <button type="button" className="btn" onClick={() => void set({ reset: ["perspective"] })} title="Back to the photo as taken (sized by its scale)">
              Undo squaring up
            </button>
          )}
        </div>
      )}

      <h3 className="panel-section">Check the model</h3>
      <p className="small muted">The model drawn square to this picture over the photo: how well the outlines match.</p>
      <div className="canvas-actions">
        <button type="button" className="btn" onClick={() => void compare()} disabled={matching}>
          <Icon name="scan" /> {matching ? "Comparing…" : "Compare with the model"}
        </button>
      </div>
      {match && (
        <div className="canvas-match">
          <p className={`canvas-iou ${match.m.iou >= 0.9 ? "good" : match.m.iou >= 0.8 ? "fair" : "poor"}`}>
            <b>{(match.m.iou * 100).toFixed(1)}%</b> overlap (IoU)
            <small>
              outlines {showLength(match.m.mean_mm, prefs)} apart on average · worst {showLength(match.m.worst_mm, prefs)}
            </small>
          </p>
          <img src={match.png} alt={`The model over ${c.name}`} />
          <p className="small muted">
            <span className="canvas-swatch extra" /> model where the photo has none · <span className="canvas-swatch missing" /> photo where the model has none
          </p>
          {match.m.regions.slice(0, 4).map((r, i) => (
            <p key={i} className="small canvas-region">
              {r.kind === "extra" ? "Extra" : "Missing"} {Math.round(r.area_mm2)} mm², up to {showLength(r.depth_mm, prefs)} · near ({r.at.map((v) => fmt(v / UNIT_MM[prefs.length], 1)).join(", ")}) {prefs.length}
            </p>
          ))}
        </div>
      )}

      <div className="canvas-actions danger-zone">
        <button
          type="button"
          className="btn"
          onClick={async () => {
            if (await run("core.entity.delete", { target: entity.id })) props.onDeleted();
          }}
        >
          <Icon name="trash" /> Remove canvas
        </button>
      </div>
    </div>
  );
}

type Pending = { file: File; plane: string; name: string; url: string };

/** Guess the plane from a file name ("side", "top", "front"…). */
function guessPlane(name: string, i: number) {
  const n = name.toLowerCase();
  if (/top|plan|above/.test(n)) return "top";
  if (/bottom|under|below/.test(n)) return "bottom";
  if (/back|rear/.test(n)) return "back";
  if (/front|muzzle|end/.test(n)) return "left";
  if (/right/.test(n)) return "back";
  if (/side|left|profile|elev/.test(n)) return "front";
  return ["front", "top", "left"][i] ?? "front";
}

/** Places reference photos on their planes (and sizes them by the object's length, if known). */
export function CanvasInsertDialog(props: { run: Run; notify: (t: string) => void; onClose: () => void; onPlaced: (ids: string[]) => void }) {
  const prefs = useDisplayPrefs();
  const [pending, setPending] = useState<Pending[]>([]);
  const [length, setLength] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => () => pending.forEach((p) => URL.revokeObjectURL(p.url)), [pending]);

  const add = (files: FileList | null) => {
    if (!files) return;
    const next = [...files].filter((f) => /\.(jpe?g|png)$/i.test(f.name) || /image\/(jpeg|png)/.test(f.type));
    if (next.length < files.length) props.notify("Canvases take PNG or JPEG pictures");
    setPending((p) => [...p, ...next.map((file, i) => ({ file, plane: guessPlane(file.name, p.length + i), name: file.name.replace(/\.[a-z0-9]+$/i, ""), url: URL.createObjectURL(file) }))]);
  };

  const place = async () => {
    setBusy(true);
    const ids: string[] = [];
    const mm = Number(length) * UNIT_MM[prefs.length];
    try {
      for (const p of pending) {
        const res = await fetch(projectUrl(`/images/${encodeURIComponent(p.file.name)}`), { method: "POST", body: await p.file.arrayBuffer() });
        if (!res.ok) throw new Error(`${p.file.name}: ${await res.text()}`);
        const { id: image } = (await res.json()) as { id: string };
        // A known length sizes the side and top views by the object's own outline; the end views are left to calibrate.
        const lengthwise = p.plane === "front" || p.plane === "back" || p.plane === "top" || p.plane === "bottom";
        const sized = mm > 0 && lengthwise ? { object: { width: mm, align: "center middle" } } : {};
        // Behind the model as its view sees it (the plane's normal faces the viewer), so it never cuts through the part.
        const behind = -(mm > 0 ? mm * 0.5 : 100);
        const r = (await props.run("canvas.create", { name: p.name, image, plane: p.plane, offset: behind, ...sized })) as { output?: { id: string } } | null;
        if (r?.output?.id) ids.push(r.output.id);
      }
      props.onPlaced(ids);
      props.onClose();
    } catch (e) {
      props.notify(String((e as Error).message ?? e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog title="Place reference photos" onClose={props.onClose}>
      <div className="canvas-insert">
        <p className="small muted">
          A few photos of one object from different sides (side, top, front)? Put each on the plane it was taken square to, size it, and model over it. Forma AI compares the model with every view.
        </p>
        <label className="btn primary canvas-choose">
          <Icon name="upload" /> Choose pictures…
          <input type="file" multiple accept=".jpg,.jpeg,.png" onChange={(e) => (add(e.target.files), (e.target.value = ""))} />
        </label>
        {pending.length > 0 && (
          <ul className="canvas-pending">
            {pending.map((p, i) => (
              <li key={p.url}>
                <img src={p.url} alt="" />
                <input value={p.name} aria-label="Canvas name" onChange={(e) => setPending((all) => all.map((q, j) => (j === i ? { ...q, name: e.target.value } : q)))} />
                <select value={p.plane} aria-label={`Plane for ${p.name}`} onChange={(e) => setPending((all) => all.map((q, j) => (j === i ? { ...q, plane: e.target.value } : q)))}>
                  {PLANES.map((pl) => (
                    <option key={pl} value={pl}>
                      {VIEW_LABEL[pl]}
                    </option>
                  ))}
                </select>
                <IconButton icon="close" label={`Leave out ${p.name}`} onClick={() => setPending((all) => all.filter((_, j) => j !== i))} />
              </li>
            ))}
          </ul>
        )}
        <label className="field">
          Object's overall length ({prefs.length}, optional)
          <input inputMode="decimal" placeholder="e.g. 165" value={length} onChange={(e) => setLength(e.target.value)} />
          <small className="muted">Sizes the Front, Back, Top and Bottom views by the object's outline. Calibrate the others with two points afterwards. Each picture goes behind the model as its view sees it.</small>
        </label>
        <div className="canvas-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Cancel
          </button>
          <button type="button" className="btn primary" disabled={!pending.length || busy || pending.some((p) => !p.name.trim())} onClick={() => void place()}>
            {busy ? "Placing…" : pending.length === 1 ? "Place it on its plane" : `Place ${pending.length || ""} on their planes`}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

/** Calibrating a canvas: the hint while two points are clicked, then the true distance. */
export function CalibrateBar(props: { entity: EntityView | undefined; points: V2[]; run: Run; onDone: () => void; onUndoPoint: () => void }) {
  const prefs = useDisplayPrefs();
  const c = canvasOf(props.entity);
  const [value, setValue] = useState("");
  const two = props.points.length >= 2;
  const px = two ? Math.hypot(props.points[1][0] - props.points[0][0], props.points[1][1] - props.points[0][1]) : 0;
  const current = two && c ? Math.hypot(...([0, 1].map((k) => (props.points[1][k] - props.points[0][k]) * (k === 0 ? 1 : (c.aspect ?? 1)))) as V2) * c.scale : 0;
  useEffect(() => {
    if (two) setValue(current ? String(Number((current / UNIT_MM[prefs.length]).toFixed(prefs.decimals))) : "");
  }, [two]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        props.onDone();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [props]);
  if (!c || !props.entity) return null;
  const apply = async () => {
    const mm = Number(value) * UNIT_MM[prefs.length];
    if (!(mm > 0)) return;
    await props.run("canvas.update", { target: props.entity!.id, calibrate: { a: props.points[0], b: props.points[1], distance: mm } });
    props.onDone();
  };
  return (
    <div className="calibrate-bar" role="dialog" aria-label={`Calibrate ${c.name}`}>
      <Icon name="ruler" />
      {!two ? (
        <span>
          <span>
            Calibrate <b>{c.name}</b>: click {props.points.length ? "the second point" : "two points on the picture a known distance apart"}
          </span>
          <small>Points on the picture itself (the model may stand in front) · Esc to cancel</small>
        </span>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void apply();
          }}
        >
          <span>
            <span>{Math.round(px)} px apart; really</span>
            <small>now {showLength(current, prefs)}</small>
          </span>
          <input autoFocus inputMode="decimal" value={value} onChange={(e) => setValue(e.target.value)} aria-label="True distance" />
          <span className="unit">{prefs.length}</span>
          <button type="submit" className="btn primary" disabled={!(Number(value) > 0)}>
            Set scale
          </button>
          <button type="button" className="btn" onClick={props.onUndoPoint}>
            Pick again
          </button>
        </form>
      )}
      <IconButton icon="close" label="Cancel calibration" onClick={props.onDone} />
    </div>
  );
}

/** Esc leaves a canvas tool (before the view or a panel takes it). */
function useEscape(onDone: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onDone();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onDone]);
}

/** Cropping a canvas: drag a rectangle over the picture in the view, then keep it. */
export function CropBar(props: { entity: EntityView | undefined; points: V2[]; dragging: boolean; run: Run; onDone: () => void; onRedo: () => void }) {
  useEscape(props.onDone);
  const [busy, setBusy] = useState(false);
  const c = canvasOf(props.entity);
  if (!c || !props.entity) return null;
  const r = props.points.length >= 2 ? cropOf(c, props.points[0], props.points[1]) : null;
  const ok = !!r && r[2] - r[0] >= 2 && r[3] - r[1] >= 2;
  const keep = async () => {
    if (!r || !ok || busy) return;
    setBusy(true);
    const done = await props.run("canvas.update", { target: props.entity!.id, crop: r });
    setBusy(false);
    if (done) props.onDone();
  };
  return (
    <div className="calibrate-bar" role="dialog" aria-label={`Crop ${c.name}`}>
      <Icon name="crop" />
      {!ok || props.dragging ? (
        <span>
          <span>
            Crop <b>{c.name}</b>: {r && props.dragging ? `${r[2] - r[0]} × ${r[3] - r[1]} px` : "drag a rectangle over the part to keep"}
          </span>
          <small>On the picture itself (shown whole while cropping) · Esc to cancel</small>
        </span>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void keep();
          }}
        >
          <span>
            <span>
              Keep {r![2] - r![0]} × {r![3] - r![1]} px
            </span>
            <small>
              of {c.pixels[0]} × {c.pixels[1]} · drag again to redraw
            </small>
          </span>
          <button type="submit" className="btn primary" autoFocus disabled={busy} title="Crop the picture to this rectangle (Enter)">
            {busy ? "Cropping…" : "Crop"}
          </button>
          <button type="button" className="btn" onClick={props.onRedo} title="Clear the rectangle and drag a new one">
            Redraw
          </button>
        </form>
      )}
      <IconButton icon="close" label="Cancel cropping (Esc)" onClick={props.onDone} />
    </div>
  );
}

/** Squaring a canvas up: click four corners of a rectangle in the picture, then type its true size. */
export function PerspectiveBar(props: { entity: EntityView | undefined; points: V2[]; run: Run; notify: (t: string) => void; onDone: () => void; onUndoPoint: () => void; onRestart: () => void }) {
  useEscape(props.onDone);
  const prefs = useDisplayPrefs();
  const c = canvasOf(props.entity);
  const [w, setW] = useState("");
  const [h, setH] = useState("");
  const [busy, setBusy] = useState(false);
  const four = props.points.length >= 4;
  useEffect(() => {
    if (!four || !c) return;
    // A first guess from the picture's present scale: the mean of each pair of opposite sides.
    const m = toPlane(c);
    const [a, b, cc, d] = cornerOrder(props.points.slice(0, 4)).map((p) => apply(m, p));
    const len = (p: V2, q: V2) => Math.hypot(q[0] - p[0], q[1] - p[1]);
    const show = (mm: number) => String(Number((mm / UNIT_MM[prefs.length]).toFixed(prefs.decimals)));
    setW(show((len(a, b) + len(d, cc)) / 2));
    setH(show((len(a, d) + len(b, cc)) / 2));
  }, [four]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!c || !props.entity) return null;
  const squareUp = async () => {
    const [wm, hm] = [Number(w) * UNIT_MM[prefs.length], Number(h) * UNIT_MM[prefs.length]];
    if (!(wm > 0 && hm > 0) || busy) return;
    const points = cornerOrder(props.points.slice(0, 4)).map((p) => [Math.round(p[0] * 10) / 10, Math.round(p[1] * 10) / 10]);
    setBusy(true);
    const done = await props.run("canvas.update", { target: props.entity!.id, perspective: { points, width: wm, height: hm } });
    setBusy(false);
    if (done) {
      props.notify(`${c.name} squared up from a ${showLength(wm, prefs)} × ${showLength(hm, prefs)} rectangle`);
      props.onDone();
    }
  };
  const corner = ["first", "second", "third", "fourth"][props.points.length] ?? "";
  return (
    <div className="calibrate-bar" role="dialog" aria-label={`Square up ${c.name}`}>
      <Icon name="perspective" />
      {!four ? (
        <span>
          <span>
            Square up <b>{c.name}</b>: click the {corner} corner of something rectangular ({props.points.length}/4)
          </span>
          <small>A sheet, a panel, a box face, a ruler's frame: corners in any order · Esc to cancel</small>
        </span>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void squareUp();
          }}
        >
          <span>
            <span>That rectangle is really</span>
            <small>width (top edge) × height</small>
          </span>
          <input autoFocus inputMode="decimal" value={w} onChange={(e) => setW(e.target.value)} aria-label="True width" title="The rectangle's true width, along its top edge" />
          <span className="unit">×</span>
          <input inputMode="decimal" value={h} onChange={(e) => setH(e.target.value)} aria-label="True height" title="The rectangle's true height, along its left edge" />
          <span className="unit">{prefs.length}</span>
          <button type="submit" className="btn primary" disabled={busy || !(Number(w) > 0 && Number(h) > 0)} title="Square the photo up so this rectangle is true, and size the picture from it">
            {busy ? "Squaring up…" : "Square up"}
          </button>
          <button type="button" className="btn" onClick={props.onRestart} title="Clear the four corners and click them again">
            Pick again
          </button>
        </form>
      )}
      {!four && props.points.length > 0 && <IconButton icon="undo" label="Take back the last corner" onClick={props.onUndoPoint} />}
      <IconButton icon="close" label="Cancel squaring up (Esc)" onClick={props.onDone} />
    </div>
  );
}
