// Drafting by hand in 3D: points, lines, wires, rectangles, polygons,
// circles, 3-point arcs, ellipses, B-splines, text, labels and dimensions,
// drawn on a working plane (XY, XZ, YZ at an offset, or a saved plane) with
// snapping to the grid and to draft points on the plane. Also: the draft
// list with layers and planes, a draft object's properties (hatch, colour,
// layer, to sketch), and arrays of a selected instance.

import { type ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import type { DraftItem, DraftPicture } from "./draftLayer";
import type { DraftShape, EntityView } from "./engine";
import { Icon, IconButton } from "./icons";
import { type Plane, type Segment, type SketchData, toWorld } from "./sketchModel";
import type { SketchPicture } from "./sketchLayer";
import { ScrubField } from "./ScrubField";
import type { SketchPointer } from "./Viewport";
import { showLength, useDisplayPrefs } from "./units";
import "./data.css";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
type V3 = [number, number, number];
type V2 = [number, number];

export type DraftKind = "point" | "line" | "wire" | "rectangle" | "polygon" | "circle" | "arc" | "ellipse" | "bspline" | "text" | "label" | "dimension";
type DraftObject = { name: string; kind: DraftKind; text?: string; color?: string; hatch?: { pattern: string; angle: number; spacing: number } | null; measure?: string; closed?: boolean; on?: string };
type Layer = { name: string; color?: string; visible: boolean; locked?: boolean; members: string[] };
type WorkingPlane = { name: string; origin: V3; x: V3; y: V3 };

export const draftOf = (e: EntityView | undefined) => e?.components["draft.object"] as DraftObject | undefined;
export const layerOf = (e: EntityView | undefined) => e?.components["draft.layer"] as Layer | undefined;
export const planeOf = (e: EntityView | undefined) => e?.components["draft.plane"] as WorkingPlane | undefined;
export const arrayOf = (e: EntityView | undefined) => e?.components["draft.array"] as { kind: string; source: string; members: string[]; count: number; count2: number; rotate: boolean; align: boolean; path?: string } | undefined;

const TOOLS: { kind: DraftKind; label: string; icon: Parameters<typeof Icon>[0]["name"]; how: string }[] = [
  { kind: "point", label: "Point", icon: "dPoint", how: "Click where it goes" },
  { kind: "line", label: "Line", icon: "dLine", how: "Click its two ends" },
  { kind: "wire", label: "Wire", icon: "dWire", how: "Click each point · double-click or Enter ends · click the first to close" },
  { kind: "rectangle", label: "Rectangle", icon: "dRect", how: "Click two opposite corners" },
  { kind: "polygon", label: "Polygon", icon: "dPolygon", how: "Click the centre, then a corner" },
  { kind: "circle", label: "Circle", icon: "dCircle", how: "Click the centre, then a point on it" },
  { kind: "arc", label: "Arc", icon: "dArc", how: "Click the start, a point on it, then the end" },
  { kind: "ellipse", label: "Ellipse", icon: "dEllipse", how: "Click the centre, the end of the long axis, then a point on the short one" },
  { kind: "bspline", label: "B-spline", icon: "dSpline", how: "Click points it passes through · double-click or Enter ends · click the first to close" },
  { kind: "text", label: "Text", icon: "dText", how: "Type the text, then click where it goes" },
  { kind: "label", label: "Label", icon: "dLabel", how: "Click what it points at, then where the text goes" },
  { kind: "dimension", label: "Dimension", icon: "dDim", how: "Click two points, then where the dimension line goes" },
];

const NEEDS: Partial<Record<DraftKind, number>> = { point: 1, line: 2, rectangle: 2, polygon: 2, circle: 2, arc: 3, ellipse: 3, text: 1, label: 2, dimension: 3 };

type PlaneChoice = { kind: "XY" | "XZ" | "YZ"; offset: number } | { kind: "saved"; id: string };

function presetPlane(kind: "XY" | "XZ" | "YZ", offset: number): Plane {
  if (kind === "XY") return { origin: [0, 0, offset], x: [1, 0, 0], y: [0, 1, 0] };
  if (kind === "XZ") return { origin: [0, offset, 0], x: [1, 0, 0], y: [0, 0, 1] };
  return { origin: [offset, 0, 0], x: [0, 1, 0], y: [0, 0, 1] };
}

const sub2 = (a: V2, b: V2): V2 => [a[0] - b[0], a[1] - b[1]];
const len2 = (a: V2) => Math.hypot(a[0], a[1]);

/** The circle through three points in the plane: centre and whether a→b→c runs counter-clockwise. */
function circle3(a: V2, b: V2, c: V2): { center: V2; ccw: boolean } | null {
  const d = 2 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
  if (Math.abs(d) < 1e-9) return null;
  const s = (p: V2) => p[0] * p[0] + p[1] * p[1];
  const center: V2 = [(s(a) * (b[1] - c[1]) + s(b) * (c[1] - a[1]) + s(c) * (a[1] - b[1])) / d, (s(a) * (c[0] - b[0]) + s(b) * (a[0] - c[0]) + s(c) * (b[0] - a[0])) / d];
  return { center, ccw: d > 0 };
}

/** What the tool would add now (dashed), in plane coordinates. */
function previewOf(kind: DraftKind, pts: V2[], cursor: V2 | null, sides: number, measure: string): Segment[] {
  const all = cursor ? [...pts, cursor] : pts;
  const lines = (p: V2[], close = false) => p.slice(1).map((q, i) => ({ kind: "line" as const, from: p[i], to: q })).concat(close && p.length > 2 ? [{ kind: "line" as const, from: p[p.length - 1], to: p[0] }] : []);
  if (all.length < 2) return [];
  switch (kind) {
    case "line":
    case "wire":
    case "label":
      return lines(all);
    case "bspline":
      return lines(all);
    case "rectangle": {
      const [a, b] = all;
      return lines([a, [b[0], a[1]], b, [a[0], b[1]]], true);
    }
    case "polygon": {
      const [c, k] = all;
      const r = len2(sub2(k, c));
      const a0 = Math.atan2(k[1] - c[1], k[0] - c[0]);
      return lines(Array.from({ length: sides }, (_, i) => [c[0] + r * Math.cos(a0 + (2 * Math.PI * i) / sides), c[1] + r * Math.sin(a0 + (2 * Math.PI * i) / sides)] as V2), true);
    }
    case "circle":
      return [{ kind: "circle", center: all[0], radius: len2(sub2(all[1], all[0])) }];
    case "arc": {
      if (all.length < 3) return lines(all);
      const c = circle3(all[0], all[1], all[2]);
      return c ? [{ kind: "arc", from: all[0], to: all[2], center: c.center, ccw: c.ccw }] : lines(all);
    }
    case "ellipse": {
      const [c, m] = all;
      const ax = sub2(m, c);
      const rx = len2(ax);
      if (!rx) return [];
      const u: V2 = [ax[0] / rx, ax[1] / rx];
      const ry = all[2] ? Math.abs(-(all[2][0] - c[0]) * u[1] + (all[2][1] - c[1]) * u[0]) : rx / 2;
      const p = Array.from({ length: 72 }, (_, i) => {
        const t = (2 * Math.PI * i) / 72;
        return [c[0] + rx * Math.cos(t) * u[0] - ry * Math.sin(t) * u[1], c[1] + rx * Math.cos(t) * u[1] + ry * Math.sin(t) * u[0]] as V2;
      });
      return lines(p, true);
    }
    case "dimension": {
      if (measure === "angle") return lines(all.length >= 3 ? [all[1], all[0], all[2]] : all);
      if (all.length < 3) return lines(all);
      const [a, b, o] = all;
      const d = sub2(b, a);
      const n: V2 = [-d[1] / (len2(d) || 1), d[0] / (len2(d) || 1)];
      const off = (o[0] - a[0]) * n[0] + (o[1] - a[1]) * n[1];
      const p: V2 = [a[0] + n[0] * off, a[1] + n[1] * off];
      const q: V2 = [b[0] + n[0] * off, b[1] + n[1] * off];
      return [...lines([a, p]), ...lines([b, q]), ...lines([p, q])];
    }
    default:
      return [];
  }
}

/** The hand drafting tool: its picture for the view, pointer handling and the bar. */
export function useDraftTool(props: { entities: ReadonlyMap<string, EntityView>; run: Run; notify: (t: string) => void; onCreated: (id: string) => void }) {
  const { entities, run, notify } = props;
  const [tool, setTool] = useState<DraftKind | null>(null);
  const [open, setOpen] = useState(false);
  const [pts, setPts] = useState<V2[]>([]);
  const [cursor, setCursor] = useState<V2 | null>(null);
  const [grid, setGrid] = useState(10);
  const [choice, setChoice] = useState<PlaneChoice>({ kind: "XY", offset: 0 });
  const [sides, setSides] = useState(6);
  const [text, setText] = useState("");
  const [measure, setMeasure] = useState("aligned");
  const [layer, setLayer] = useState<string>("");
  const [hatch, setHatch] = useState(false);

  const plane: Plane = useMemo(() => {
    if (choice.kind === "saved") {
      const p = planeOf(entities.get(choice.id));
      if (p) return { origin: p.origin, x: p.x, y: p.y };
      return presetPlane("XY", 0);
    }
    return presetPlane(choice.kind, choice.offset);
  }, [choice, entities]);
  const normal: V3 = useMemo(() => {
    const [a, b] = [plane.x, plane.y];
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  }, [plane]);

  // Draft points lying on the plane are snap targets.
  const snaps: V2[] = useMemo(() => {
    const out: V2[] = [];
    for (const e of entities.values()) {
      if (!e.draft || !draftOf(e)) continue;
      for (const p of [...e.draft.points, ...e.draft.lines.flatMap((l) => (l.length > 2 ? [l[0], l[l.length - 1]] : []))]) {
        const d: V3 = [p[0] - plane.origin[0], p[1] - plane.origin[1], p[2] - plane.origin[2]];
        if (Math.abs(d[0] * normal[0] + d[1] * normal[1] + d[2] * normal[2]) > 1e-6) continue;
        out.push([d[0] * plane.x[0] + d[1] * plane.x[1] + d[2] * plane.x[2], d[0] * plane.y[0] + d[1] * plane.y[1] + d[2] * plane.y[2]]);
      }
    }
    return out.slice(0, 2000);
  }, [entities, plane, normal]);

  const reset = useCallback(() => {
    setPts([]);
    setCursor(null);
  }, []);
  const stop = useCallback(() => {
    setTool(null);
    reset();
  }, [reset]);
  const close = useCallback(() => {
    setOpen(false);
    stop();
  }, [stop]);

  const w = useCallback((p: V2) => toWorld(plane, p[0], p[1]), [plane]);

  const finish = useCallback(
    async (points: V2[], closed = false) => {
      if (!tool) return;
      const base: Record<string, unknown> = { kind: tool, ...(layer ? { layer } : {}) };
      const P = points.map(w);
      switch (tool) {
        case "point":
        case "line":
          Object.assign(base, { points: P });
          break;
        case "wire":
        case "bspline":
          if (points.length < 2) return reset();
          Object.assign(base, { points: P, closed, ...(hatch && closed ? { hatch: { pattern: "lines", angle: 45, spacing: 3 } } : {}) });
          break;
        case "rectangle":
          Object.assign(base, { points: P, normal, x_dir: plane.x, ...(hatch ? { hatch: { pattern: "lines", angle: 45, spacing: 3 } } : {}) });
          break;
        case "polygon": {
          const [c, k] = points;
          const ang = (Math.atan2(k[1] - c[1], k[0] - c[0]) * 180) / Math.PI;
          Object.assign(base, { center: w(c), radius: len2(sub2(k, c)), sides, start: ang, normal, x_dir: plane.x, ...(hatch ? { hatch: { pattern: "lines", angle: 45, spacing: 3 } } : {}) });
          break;
        }
        case "circle":
          Object.assign(base, { center: P[0], radius: len2(sub2(points[1], points[0])), normal, x_dir: plane.x, ...(hatch ? { hatch: { pattern: "lines", angle: 45, spacing: 3 } } : {}) });
          break;
        case "arc":
          Object.assign(base, { points: P });
          break;
        case "ellipse": {
          const [c, m, s] = points;
          const ax = sub2(m, c);
          const rx = len2(ax);
          const u: V2 = [ax[0] / rx, ax[1] / rx];
          const ry = Math.abs(-(s[0] - c[0]) * u[1] + (s[1] - c[1]) * u[0]);
          const x_dir = [0, 1, 2].map((k) => plane.x[k] * u[0] + plane.y[k] * u[1]) as V3;
          if (!(rx > 0 && ry > 0)) return reset();
          Object.assign(base, { center: P[0], radius: rx, radius2: ry, normal, x_dir });
          break;
        }
        case "text":
          if (!text.trim()) {
            notify("Type the text first");
            return reset();
          }
          Object.assign(base, { points: P, text: text.trim() });
          break;
        case "label":
          Object.assign(base, { points: [P[1], P[0]], text: text.trim() || "Note" });
          break;
        case "dimension": {
          if (measure === "angle") {
            Object.assign(base, { measure, points: [P[0], P[1], P[2]] });
            break;
          }
          if (measure === "radius" || measure === "diameter") {
            Object.assign(base, { measure, points: [P[0], P[1]] });
            break;
          }
          const [a, b, o] = points;
          const d = sub2(b, a);
          const n: V2 = [-d[1] / (len2(d) || 1), d[0] / (len2(d) || 1)];
          const off = (o[0] - a[0]) * n[0] + (o[1] - a[1]) * n[1];
          const ow = w([n[0] * off, n[1] * off]);
          Object.assign(base, { measure, points: [P[0], P[1]], offset: [ow[0] - plane.origin[0], ow[1] - plane.origin[1], ow[2] - plane.origin[2]] });
          break;
        }
      }
      reset();
      const out = (await run("draft.create", base)) as { output: { id: string } } | null;
      if (out) props.onCreated(out.output.id);
    },
    [tool, layer, w, hatch, normal, plane, sides, text, measure, run, notify, reset, props],
  );

  const need = tool === "dimension" && (measure === "radius" || measure === "diameter") ? 2 : tool ? NEEDS[tool] : undefined;
  const onPointer = useCallback(
    (e: SketchPointer) => {
      if (!tool) return;
      const p: V2 = [e.u, e.v];
      setGrid(e.grid);
      if (e.type === "move") return setCursor(p);
      if (e.type === "double") {
        if (tool === "wire" || tool === "bspline") void finish(pts);
        return;
      }
      if (e.type !== "down") return;
      // Closing a wire or spline: click its first point again.
      if ((tool === "wire" || tool === "bspline") && pts.length >= 3 && len2(sub2(p, pts[0])) < 1e-9) return void finish(pts, true);
      const next = [...pts, p];
      if (need && next.length >= need) void finish(next);
      else setPts(next);
    },
    [tool, pts, need, finish],
  );

  useEffect(() => {
    if (!tool) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t?.closest("input, textarea, select")) return;
      if (e.key === "Escape") {
        e.stopPropagation();
        if (pts.length) reset();
        else stop();
      } else if (e.key === "Enter" && (tool === "wire" || tool === "bspline")) void finish(pts);
      else if (e.key === "Backspace") setPts((p) => p.slice(0, -1));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [tool, pts, reset, stop, finish]);

  const picture: SketchPicture | null = useMemo(() => {
    if (!tool) return null;
    const points = [...snaps, ...pts].map((p, i) => ({ id: i + 1, x: p[0], y: p[1] }));
    const data: SketchData = { name: "Draft", plane, geometry: { points, curves: [], constraints: [] }, params: [] };
    return { data, solved: undefined, hoverPoint: null, hoverCurve: null, selected: [], preview: previewOf(tool, pts, cursor, sides, measure), cursor, grid };
  }, [tool, snaps, pts, plane, cursor, sides, measure, grid]);

  const layers = [...entities.values()].filter((e) => layerOf(e));
  const planes = [...entities.values()].filter((e) => planeOf(e));
  const how = TOOLS.find((t) => t.kind === tool)?.how;

  const bar: ReactNode = open ? (
    <div className="sketch-bar draft-bar floating" role="toolbar" aria-label="Draft tools">
      <div className="draft-head">
        <Icon name="dDim" />
        <strong>Draft</strong>
        <span className="muted small">{how ?? "Pick a tool, then click on the working plane · Esc steps back"}</span>
        <IconButton icon="close" label="Leave drafting" onClick={close} />
      </div>
      <div className="sketch-tools">
        {TOOLS.map((t) => (
          <button key={t.kind} type="button" className={tool === t.kind ? "active" : ""} onClick={() => (tool === t.kind ? stop() : (setTool(t.kind), reset()))} title={t.how}>
            <Icon name={t.icon} />
            {t.label}
          </button>
        ))}
      </div>
      <div className="draft-options">
        <label>
          Plane
          <select
            value={choice.kind === "saved" ? choice.id : choice.kind}
            onChange={(e) => {
              const v = e.target.value;
              setChoice(v === "XY" || v === "XZ" || v === "YZ" ? { kind: v, offset: choice.kind === "saved" ? 0 : choice.offset } : { kind: "saved", id: v });
              reset();
            }}
          >
            <option value="XY">XY (top)</option>
            <option value="XZ">XZ (front)</option>
            <option value="YZ">YZ (side)</option>
            {planes.map((e) => (
              <option key={e.id} value={e.id}>
                {planeOf(e)!.name}
              </option>
            ))}
          </select>
        </label>
        {choice.kind !== "saved" && (
          <label className="draft-offset">
            at
            <input
              type="number"
              value={choice.offset}
              step={1}
              aria-label="Plane offset (mm)"
              onChange={(e) => setChoice({ ...choice, offset: Number(e.target.value) || 0 })}
            />
            mm
          </label>
        )}
        <button
          type="button"
          className="link-button"
          title="Keep this plane by name, to draw and sketch on again (and for Forma AI)"
          onClick={async () => {
            const name = `Plane ${planes.length + 1}`;
            const out = (await run("draft.plane.create", { name, origin: plane.origin, normal, x: plane.x })) as { output: { id: string } } | null;
            if (out) setChoice({ kind: "saved", id: out.output.id });
          }}
        >
          <Icon name="plane" /> Save plane
        </button>
        {tool === "polygon" && (
          <label>
            Sides
            <input type="number" min={3} max={64} value={sides} onChange={(e) => setSides(Math.max(3, Math.min(64, Number(e.target.value) || 6)))} />
          </label>
        )}
        {(tool === "text" || tool === "label") && <input className="draft-text" value={text} placeholder="Text" aria-label="Text" onChange={(e) => setText(e.target.value)} />}
        {tool === "dimension" && (
          <label>
            Measure
            <select
              value={measure}
              onChange={(e) => {
                setMeasure(e.target.value);
                reset();
              }}
            >
              <option value="aligned">Aligned</option>
              <option value="x">Along X</option>
              <option value="y">Along Y</option>
              <option value="z">Along Z</option>
              <option value="angle">Angle (vertex, then each side)</option>
              <option value="radius">Radius (centre, then the curve)</option>
              <option value="diameter">Diameter (centre, then the curve)</option>
            </select>
          </label>
        )}
        {(tool === "rectangle" || tool === "polygon" || tool === "circle" || tool === "wire" || tool === "bspline") && (
          <label className="check">
            <input type="checkbox" checked={hatch} onChange={(e) => setHatch(e.target.checked)} /> Hatch
          </label>
        )}
        <label>
          Layer
          <select value={layer} onChange={(e) => setLayer(e.target.value)}>
            <option value="">None</option>
            {layers.map((e) => (
              <option key={e.id} value={e.id}>
                {layerOf(e)!.name}
              </option>
            ))}
          </select>
        </label>
      </div>
    </div>
  ) : null;

  return { open, setOpen, tool, picture, onPointer, bar, close };
}

/** Draft objects to draw: those not hidden by a layer, coloured by their layer (or their own colour). */
export function draftPicture(entities: ReadonlyMap<string, EntityView>, selected: string | null, hidden: ReadonlySet<string>): DraftPicture {
  const layerBy = new Map<string, Layer>();
  for (const e of entities.values()) {
    const l = layerOf(e);
    if (l) for (const m of l.members) layerBy.set(m, l);
  }
  const items: DraftItem[] = [];
  for (const e of entities.values()) {
    const o = draftOf(e);
    if (!o || !e.draft || hidden.has(e.id)) continue;
    const l = layerBy.get(e.id);
    if (l && !l.visible) continue;
    items.push({ id: e.id, kind: o.kind, shape: e.draft as DraftShape, color: o.color ?? l?.color ?? null, size: (e.components["draft.object"] as { size?: number }).size });
  }
  return { items, selected };
}

/** What layers hide and how they colour things, for the view. */
export function layerEffects(entities: ReadonlyMap<string, EntityView>) {
  const hidden = new Set<string>();
  const tints = new Map<string, string>();
  for (const e of entities.values()) {
    const l = layerOf(e);
    if (!l) continue;
    for (const m of l.members) {
      if (!l.visible) hidden.add(m);
      if (l.color) tints.set(m, l.color);
    }
  }
  return { hidden, tints };
}

/** The Draft section of the objects panel: objects, layers, working planes. */
export function DraftSection(props: { entities: ReadonlyMap<string, EntityView>; selected: string | null; also: string[]; run: Run; onSelect: (id: string) => void; onDraft: () => void }) {
  const { entities, run } = props;
  const prefs = useDisplayPrefs();
  const all = [...entities.values()];
  const objects = all.filter((e) => draftOf(e));
  const layers = all.filter((e) => layerOf(e));
  const planes = all.filter((e) => planeOf(e));
  const picked = [props.selected, ...props.also].filter((x): x is string => !!x && entities.has(x));
  const toolIcon = (k: DraftKind) => TOOLS.find((t) => t.kind === k)?.icon ?? "dLine";
  return (
    <div className="draft-section">
      {objects.map((e) => {
        const o = draftOf(e)!;
        const d = e.draft;
        const fact = d?.value != null ? d.label?.[1] : d?.area != null ? `${showLength(d.length, prefs)} · ${Number(d.area.toFixed(1))} mm²` : d && d.length > 0 ? showLength(d.length, prefs) : o.text;
        return (
          <button key={e.id} type="button" className={`list-row ${props.selected === e.id ? "active" : ""}`} onClick={() => props.onSelect(e.id)}>
            <Icon name={toolIcon(o.kind)} />
            <span>
              {o.name}
              {fact && <small className="row-count">{fact}</small>}
            </span>
          </button>
        );
      })}
      <button type="button" className="secondary add-part" onClick={props.onDraft}>
        <Icon name="dDim" /> Draft on a plane
      </button>
      <h3 className="panel-section">Layers</h3>
      {layers.length === 0 && <p className="muted small">Layers show, hide, colour and lock anything together.</p>}
      {layers.map((e) => {
        const l = layerOf(e)!;
        return (
          <div key={e.id} className="layer-row">
            <button type="button" className={`layer-eye ${l.visible ? "" : "off"}`} title={l.visible ? "Hide this layer" : "Show this layer"} onClick={() => void run("draft.layer.update", { target: e.id, visible: !l.visible })}>
              <Icon name="eye" />
            </button>
            <input
              type="color"
              value={/^#[0-9a-f]{6}$/i.test(l.color ?? "") ? l.color! : "#9aa0ab"}
              title="Layer colour"
              aria-label={`${l.name} colour`}
              onChange={(ev) => void run("draft.layer.update", { target: e.id, color: ev.target.value })}
            />
            <span className="layer-name">
              {l.name}
              <small className="row-count">{l.members.filter((m) => entities.has(m)).length} items</small>
            </span>
            {picked.length > 0 && (
              <button type="button" className="link-button" title="Put the selection on this layer" onClick={() => void run("draft.layer.update", { target: e.id, add: picked })}>
                + selection
              </button>
            )}
            {l.color && (
              <button type="button" className="link-button" title="Back to their own colours" onClick={() => void run("draft.layer.update", { target: e.id, color: "" })}>
                no colour
              </button>
            )}
            <IconButton icon="trash" label={`Delete layer ${l.name} (its items stay)`} onClick={() => void run("core.entity.delete", { target: e.id })} />
          </div>
        );
      })}
      <button
        type="button"
        className="secondary add-part"
        onClick={() => {
          const used = new Set(layers.map((e) => layerOf(e)!.name));
          let n = 1;
          while (used.has(`Layer ${n}`)) n++;
          void run("draft.layer.create", { name: `Layer ${n}`, members: picked });
        }}
      >
        <Icon name="layers" /> New layer{picked.length ? " from the selection" : ""}
      </button>
      {planes.length > 0 && <h3 className="panel-section">Working planes</h3>}
      {planes.map((e) => (
        <div key={e.id} className="layer-row">
          <Icon name="plane" />
          <span className="layer-name">
            {planeOf(e)!.name}
            <small className="row-count">at {planeOf(e)!.origin.map((v) => Number(v.toFixed(2))).join(", ")}</small>
          </span>
          <IconButton icon="trash" label={`Delete plane ${planeOf(e)!.name}`} onClick={() => void run("core.entity.delete", { target: e.id })} />
        </div>
      ))}
    </div>
  );
}

/** A draft object's properties: name, text, hatch, colour, layer; to a sketch; delete. */
export function DraftProperties(props: { entity: EntityView; entities: ReadonlyMap<string, EntityView>; also: string[]; run: Run; notify: (t: string) => void; onSelect: (id: string | null) => void }) {
  const { entity, run } = props;
  const o = draftOf(entity)!;
  const d = entity.draft;
  const prefs = useDisplayPrefs();
  const [name, setName] = useState(o.name);
  useEffect(() => setName(o.name), [o.name]);
  const layers = [...props.entities.values()].filter((e) => layerOf(e));
  const inLayer = layers.find((e) => layerOf(e)!.members.includes(entity.id));
  const closed = !!d?.closed && d.area != null;
  const geometric = !["text", "label", "dimension"].includes(o.kind);
  const set = (fields: Record<string, unknown>) => run("draft.update", { target: entity.id, ...fields });
  const targets = [entity.id, ...props.also.filter((a) => draftOf(props.entities.get(a)))];
  return (
    <div className="draft-props">
      <label className="field">
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name.trim() && name !== o.name && void set({ name: name.trim() })} onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()} />
      </label>
      <p className="draft-facts">
        {o.kind === "dimension" ? (
          <>
            Measures <b>{d?.value != null ? (o.measure === "angle" ? `${Number(d.value.toFixed(3))}°` : showLength(d.value, prefs)) : "—"}</b>
            {o.on || (entity.components["draft.object"] as { anchors?: unknown[] }).anchors?.length ? " · follows the parts it is on" : ""}
          </>
        ) : (
          <>
            {d && d.length > 0 && <>Length <b>{showLength(d.length, prefs)}</b></>}
            {d?.area != null && <> · area <b>{Number(d.area.toFixed(2))} mm²</b></>}
          </>
        )}
      </p>
      {entity.status?.state === "failed" && <p className="sheet-error">{entity.status.reason}</p>}
      {(o.kind === "text" || o.kind === "label" || o.kind === "dimension") && (
        <label className="field">
          Text{o.kind === "dimension" ? " (<> is the value)" : ""}
          <input defaultValue={o.text ?? ""} key={o.text ?? ""} placeholder={o.kind === "dimension" ? "<>" : ""} onBlur={(e) => e.target.value !== (o.text ?? "") && void set({ text: e.target.value })} onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()} />
        </label>
      )}
      {closed && (
        <div className="draft-hatch">
          <label className="check">
            <input type="checkbox" checked={!!o.hatch} onChange={(e) => void set({ hatch: e.target.checked ? { pattern: "lines", angle: 45, spacing: 3 } : null })} /> Hatch
          </label>
          {o.hatch && (
            <>
              <select value={o.hatch.pattern} onChange={(e) => void set({ hatch: { ...o.hatch, pattern: e.target.value } })}>
                <option value="lines">Lines</option>
                <option value="cross">Cross</option>
              </select>
              <ScrubField label="Hatch angle" chip="∠" unit="deg" value={o.hatch.angle} step={5} onLive={() => undefined} onCommit={(v) => void set({ hatch: { ...o.hatch, angle: v } })} />
              <ScrubField label="Hatch spacing" chip="S" value={o.hatch.spacing} min={0.1} step={0.5} onLive={() => undefined} onCommit={(v) => void set({ hatch: { ...o.hatch, spacing: v } })} />
            </>
          )}
        </div>
      )}
      <div className="draft-row">
        <label>
          Colour
          <input type="color" value={/^#[0-9a-f]{6}$/i.test(o.color ?? "") ? o.color! : "#2563c9"} onChange={(e) => void set({ color: e.target.value })} />
        </label>
        <label>
          Layer
          <select
            value={inLayer?.id ?? ""}
            onChange={(e) => {
              const v = e.target.value;
              if (v) void run("draft.layer.update", { target: v, add: [entity.id] });
              else if (inLayer) void run("draft.layer.update", { target: inLayer.id, remove: [entity.id] });
            }}
          >
            <option value="">None</option>
            {layers.map((e) => (
              <option key={e.id} value={e.id}>
                {layerOf(e)!.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div className="draft-actions">
        {geometric && (
          <button
            type="button"
            className="secondary"
            title="Turn flat draft geometry (this and the other selected draft objects) into a sketch to extrude or constrain"
            onClick={async () => {
              const r = (await run("draft.to_sketch", { targets })) as { output: { name: string; curves: number } } | null;
              if (r) props.notify(`Made the sketch ${r.output.name} (${r.output.curves} curves)`);
            }}
          >
            <Icon name="pencil" /> To sketch{targets.length > 1 ? ` (${targets.length})` : ""}
          </button>
        )}
        <button
          type="button"
          className="secondary danger"
          onClick={() => {
            props.onSelect(null);
            void run("core.entity.delete", { target: entity.id });
          }}
        >
          <Icon name="trash" /> Delete
        </button>
      </div>
    </div>
  );
}

/** Arrays of a selected instance: make one, or change the one it leads. */
export function ArraySection(props: { entity: EntityView; entities: ReadonlyMap<string, EntityView>; run: Run; notify: (t: string) => void }) {
  const { entity, entities, run } = props;
  const all = [...entities.values()];
  const led = all.find((e) => arrayOf(e)?.source === entity.id);
  const memberOf = all.find((e) => arrayOf(e)?.members.includes(entity.id));
  const paths = all.filter((e) => draftOf(e) && !["text", "label", "dimension", "point"].includes(draftOf(e)!.kind));
  const [kind, setKind] = useState<"linear" | "polar" | "path">("linear");
  const [count, setCount] = useState(4);
  const [count2, setCount2] = useState(1);
  const [spacing, setSpacing] = useState(30);
  const [dir, setDir] = useState<"x" | "y" | "z">("x");
  const [path, setPath] = useState("");
  const axis = { x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] };
  if (memberOf) {
    const name = (memberOf.components["shape.parameter_set"] as { name: string }).name;
    return (
      <div className="array-section">
        <h3 className="panel-section">Array</h3>
        <p className="muted small">A copy in {name}: the array places it. Change the array on its original.</p>
      </div>
    );
  }
  if (led) {
    const a = arrayOf(led)!;
    const v = led.params ?? {};
    const f = led.formulas ?? {};
    const name = (led.components["shape.parameter_set"] as { name: string }).name;
    return (
      <div className="array-section">
        <h3 className="panel-section">Array · {name}</h3>
        <p className="muted small">
          {a.kind === "polar" ? "Round an axis" : a.kind === "path" ? "Along a path" : a.count2 > 1 ? "A grid" : "In a row"}: {a.count * Math.max(1, a.count2)} in all, each a real instance (BOM, exports and checks count them).
        </p>
        <div className="array-fields">
          <label className="field">
            Count
            <input type="number" min={1} defaultValue={a.count} key={a.count} onBlur={(e) => Number(e.target.value) !== a.count && void run("draft.array.update", { target: led.id, count: Math.max(1, Number(e.target.value) || a.count) })} />
          </label>
          {a.kind === "linear" && (
            <label className="field">
              Rows
              <input type="number" min={1} defaultValue={a.count2} key={a.count2} onBlur={(e) => Number(e.target.value) !== a.count2 && void run("draft.array.update", { target: led.id, count2: Math.max(1, Number(e.target.value) || 1) })} />
            </label>
          )}
        </div>
        {a.kind === "linear" && (
          <ScrubField label="Spacing" chip="↔" value={v.spacing ?? 0} formula={f.spacing} onFormula={(expr) => void run("shape.link_param", { target: led.id, key: "spacing", expr })} onLive={() => undefined} onCommit={(x) => void run("draft.array.update", { target: led.id, spacing: x })} />
        )}
        {a.kind === "linear" && a.count2 > 1 && (
          <ScrubField label="Row spacing" chip="↕" value={v.spacing2 ?? 0} formula={f.spacing2} onFormula={(expr) => void run("shape.link_param", { target: led.id, key: "spacing2", expr })} onLive={() => undefined} onCommit={(x) => void run("draft.array.update", { target: led.id, spacing2: x })} />
        )}
        {a.kind === "polar" && (
          <ScrubField label="Angle" chip="∠" unit="deg" value={v.angle ?? 360} min={-360} max={360} step={5} formula={f.angle} onFormula={(expr) => void run("shape.link_param", { target: led.id, key: "angle", expr })} onLive={() => undefined} onCommit={(x) => void run("draft.array.update", { target: led.id, angle: x })} />
        )}
        <p className="muted small">Type = in a field to link it (=Rack.pitch).</p>
        <div className="draft-actions">
          <button type="button" className="secondary" onClick={() => void run("draft.array.delete", { target: led.id, keep: true })} title="The copies stay where they are, as ordinary instances">
            End array, keep copies
          </button>
          <button type="button" className="secondary danger" onClick={() => void run("draft.array.delete", { target: led.id, keep: false })}>
            <Icon name="trash" /> Remove copies
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="array-section">
      <h3 className="panel-section">Array</h3>
      <div className="seg" role="tablist">
        {(["linear", "polar", "path"] as const).map((k) => (
          <button key={k} type="button" role="tab" aria-selected={kind === k} className={kind === k ? "active" : ""} onClick={() => setKind(k)}>
            {k === "linear" ? "Row / grid" : k === "polar" ? "Round" : "Along a path"}
          </button>
        ))}
      </div>
      <div className="array-fields">
        <label className="field">
          Count
          <input type="number" min={2} value={count} onChange={(e) => setCount(Math.max(2, Number(e.target.value) || 2))} />
        </label>
        {kind === "linear" && (
          <>
            <label className="field">
              Along
              <select value={dir} onChange={(e) => setDir(e.target.value as "x" | "y" | "z")}>
                <option value="x">X</option>
                <option value="y">Y</option>
                <option value="z">Z</option>
              </select>
            </label>
            <label className="field">
              Rows
              <input type="number" min={1} value={count2} onChange={(e) => setCount2(Math.max(1, Number(e.target.value) || 1))} />
            </label>
          </>
        )}
        {kind === "path" && (
          <label className="field">
            Path
            <select value={path} onChange={(e) => setPath(e.target.value)}>
              <option value="">Pick a draft line…</option>
              {paths.map((e) => (
                <option key={e.id} value={e.id}>
                  {draftOf(e)!.name}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>
      {kind === "linear" && <ScrubField label="Spacing" chip="↔" value={spacing} step={1} onLive={() => undefined} onCommit={setSpacing} />}
      {kind === "polar" && <p className="muted small">Evenly round the Z axis through the origin; change the centre, axis or a partial angle after.</p>}
      <button
        type="button"
        className="primary"
        disabled={kind === "path" && !path}
        onClick={async () => {
          const params: Record<string, unknown> = { source: entity.id, kind, count };
          if (kind === "linear") Object.assign(params, { spacing, direction: axis[dir], count2, spacing2: spacing, direction2: dir === "y" ? axis.x : axis.y });
          if (kind === "path") Object.assign(params, { path, align: true });
          const r = (await run("draft.array.create", params)) as { output: { copies: string[] } } | null;
          if (r) props.notify(`Made ${r.output.copies.length} copies, placed by the array`);
        }}
      >
        <Icon name="array" /> Make array
      </button>
    </div>
  );
}
