// Sketching by hand. A small state machine per tool turns clicks on the
// sketch plane into geometry; each finished action is one `sketch.update`
// (one undo step) and the engine solves the result. Dimensions are typed
// onto the drawing: a number, or a formula like =Desk size.width - 40.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { EntityView } from "./engine";
import { Icon, type IconName } from "./icons";
import type { SketchPicture } from "./sketchLayer";
import {
  type Dim,
  type Geometry,
  PLANES,
  type Segment,
  addArc,
  addBezier,
  addCircle,
  addLine,
  addRectangle,
  curvePoints,
  newKey,
  pointAt,
  positions,
  radiusOf,
  remove,
  sketchOf,
  solvedOf,
  status,
} from "./sketchModel";
import { type Align, align, distribute, duplicate, extent, inBox, mapPoints, mirrorAbout, moveBy, movedPoints, objects, rewrite, rotateAbout, scaleAbout } from "./sketchTransform";
import type { SketchPointer } from "./Viewport";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
export type Tool = "select" | "pen" | "line" | "rect" | "circle" | "arc" | "dimension" | "trim" | "extend";

const TOOLS: { tool: Tool; label: string; key: string; hint: string }[] = [
  { tool: "select", label: "Select", key: "S", hint: "Click to select · Shift adds · drag a box (right to left: touching) · drag to move · arrows nudge · ⌘D copies · Delete removes" },
  { tool: "pen", label: "Pen", key: "P", hint: "Click for a corner · press and drag for a smooth curve · click the first point to close · Enter or Esc to finish" },
  { tool: "line", label: "Line", key: "L", hint: "Click point to point · click the first point to close · double-click or Esc to stop" },
  { tool: "rect", label: "Rectangle", key: "R", hint: "Click two opposite corners" },
  { tool: "circle", label: "Circle", key: "C", hint: "Click the centre, then the radius" },
  { tool: "arc", label: "Arc", key: "A", hint: "Click the centre, the start, then the end (counter-clockwise)" },
  { tool: "dimension", label: "Dimension", key: "D", hint: "Click a line, circle or arc, or two points, then type a value or =formula" },
  { tool: "trim", label: "Trim", key: "T", hint: "Click the piece of a curve to cut away, back to where other curves cross it" },
  { tool: "extend", label: "Extend", key: "E", hint: "Click near the end of a line or arc to carry it on to the next curve" },
];

type Draft =
  | { kind: "none" }
  | { kind: "line"; from: number | null; at: [number, number]; first: number | null }
  | { kind: "rect"; at: [number, number] }
  | { kind: "circle"; center: number | null; at: [number, number] }
  | { kind: "arc"; center: [number, number]; centerId: number | null; start?: { id: number | null; at: [number, number] } }
  | { kind: "dimension"; from: number }
  | { kind: "drag"; point: number; at: [number, number] }
  /** Moving the selection: where the drag started and where it is. */
  | { kind: "move"; from: [number, number]; at: [number, number] }
  /** Drawing a selection box; Shift adds to what is selected. */
  | { kind: "box"; from: [number, number]; at: [number, number]; add: boolean }
  | {
      kind: "pen";
      /** The last anchor: its point (null until the second anchor makes it), where it is, its outgoing handle. */
      last: { id: number | null; at: [number, number]; out: [number, number] | null; seg: number | null } | null;
      /** The first anchor, for closing (and its handle, to join smoothly when closed). */
      first: { id: number | null; at: [number, number]; out: [number, number] | null; seg: number | null } | null;
      /** Pressed and not yet released: where (and on which point), and where the drag is now. */
      down: { at: [number, number]; point: number | null; now: [number, number] } | null;
    };

export function useSketchEditor(props: {
  entities: ReadonlyMap<string, EntityView>;
  run: Run;
  notify: (text: string) => void;
  onFace: (plane: SketchPicture["data"]["plane"] | null, focus?: { center: [number, number]; size: number }) => void;
  onMade: (id: string) => void;
}) {
  const { entities, run } = props;
  const [editing, setEditing] = useState<string | null>(null);
  const [tool, setTool] = useState<Tool>("line");
  const [draft, setDraft] = useState<Draft>({ kind: "none" });
  const [hover, setHover] = useState<{ point: number | null; curve: number | null; at: [number, number] | null; grid: number }>({ point: null, curve: null, at: null, grid: 10 });
  const [selection, setSelection] = useState<ReadonlySet<number>>(new Set());
  const setSelected = useCallback((id: number | null) => setSelection(new Set(id === null ? [] : [id])), []);
  const [editDim, setEditDim] = useState<{ key: string; x: number; y: number; text: string } | null>(null);

  const entity = editing ? entities.get(editing) : undefined;
  const data = sketchOf(entity);
  const solved = solvedOf(entity);

  // A deleted sketch (or undo past its creation) ends the editing.
  useEffect(() => {
    if (editing && !entity) setEditing(null);
  }, [editing, entity]);

  const commit = useCallback(
    (geometry: Geometry, params?: Dim[]) => {
      if (!editing || !data) return Promise.resolve();
      return run("sketch.update", { target: editing, geometry, ...(params ? { params } : {}) }).catch(() => {});
    },
    [editing, data, run],
  );

  /** A dragged point: it stays where it was put, everything else adapts. */
  const commitDrag = useCallback(
    (geometry: Geometry, point: number, at: [number, number]) => {
      if (!editing) return Promise.resolve();
      return run("sketch.update", { target: editing, geometry, drag: { point, x: at[0], y: at[1] } }).catch(() => {});
    },
    [editing, run],
  );

  const start = useCallback(
    async (plane: keyof typeof PLANES) => {
      const n = [...entities.values()].filter((e) => sketchOf(e)).length + 1;
      const out = (await run("sketch.create", { name: `Sketch ${n}`, plane: { name: plane } })) as { output?: { id: string } } | null;
      const id = out?.output?.id;
      if (!id) return;
      setEditing(id);
      setTool("line");
      setDraft({ kind: "none" });
      props.onFace(PLANES[plane].plane);
    },
    [entities, run, props],
  );

  const edit = useCallback(
    (id: string) => {
      setEditing(id);
      setTool("select");
      setDraft({ kind: "none" });
      const e = entities.get(id);
      const s = sketchOf(e);
      if (!s) return;
      // Centre the view on what is drawn.
      const pts = [...positions(s.geometry, solvedOf(e)).values()];
      if (!pts.length) return props.onFace(s.plane);
      const xs = pts.map((p) => p[0]);
      const ys = pts.map((p) => p[1]);
      const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
      props.onFace(s.plane, { center: [(x0 + x1) / 2, (y0 + y1) / 2], size: Math.max(x1 - x0, y1 - y0, 50) });
    },
    [entities, props],
  );

  const finish = useCallback(() => {
    setEditing(null);
    setDraft({ kind: "none" });
    setSelected(null);
    setEditDim(null);
    props.onFace(null);
  }, [props]);

  /** Turns the sketch into a part: a program that extrudes (or revolves) it. */
  const make = useCallback(
    async (how: "extrude" | "revolve") => {
      if (!data || !editing) return;
      if (!solved?.loops.length) return props.notify("Close an outline first: a part needs at least one closed loop.");
      // A starting depth from the sketch's own size (a fifth of its larger side).
      const pts = Object.values(solved.solved.points);
      const size = Math.max(10, Math.max(...pts.map((p) => p[0])) - Math.min(...pts.map((p) => p[0])), Math.max(...pts.map((p) => p[1])) - Math.min(...pts.map((p) => p[1])));
      const depth = Math.round(size / 5);
      const source =
        how === "extrude"
          ? `// ${data.name}, extruded along its plane's normal.\nreturn sketch(${JSON.stringify(data.name)}).extrude(p.depth);`
          : `// ${data.name}, revolved about the sketch's vertical axis (x is the radius).\nreturn sketch(${JSON.stringify(data.name)}).revolve(p.angle);`;
      const params =
        how === "extrude"
          ? [{ key: "depth", label: "Depth", value: depth, unit: "mm", min: 0.01 }]
          : [{ key: "angle", label: "Angle", value: 360, unit: "deg", min: 1, max: 360 }];
      const out = (await run("shape.create", { name: `${data.name} ${how === "extrude" ? "extrusion" : "revolve"}`, source, params })) as { output?: { id: string } } | null;
      finish();
      if (out?.output?.id) props.onMade(out.output.id);
    },
    [data, editing, solved, run, finish, props],
  );

  const pos = useMemo(() => (data ? positions(data.geometry, solved) : new Map<number, [number, number]>()), [data, solved]);

  const addDimension = useCallback(
    (kind: "length" | "diameter" | "radius" | "distance", a: number, b: number | null, value: number, x: number, y: number) => {
      if (!data) return;
      const key = newKey(data.params);
      const label = { length: "Length", diameter: "Diameter", radius: "Radius", distance: "Distance" }[kind];
      const c = kind === "length" ? { kind, line: a, key } : kind === "distance" ? { kind, a, b: b!, key } : { kind, curve: a, key };
      const params = [...data.params, { key, label: `${label} ${key.slice(1)}`, value: Math.round(value * 100) / 100, unit: "mm", min: 0.001 }];
      commit({ ...data.geometry, constraints: [...data.geometry.constraints, c as Geometry["constraints"][number]] }, params).then(() =>
        setEditDim({ key, x, y, text: String(Math.round(value * 100) / 100) }),
      );
    },
    [data, commit],
  );

  // The point being dragged, kept in a ref: pointer events can outrun React's state.
  const dragging = useRef<{ point: number; at: [number, number] } | null>(null);
  // Likewise a selection being moved, or a selection box being drawn.
  const sweeping = useRef<{ kind: "move" | "box"; from: [number, number]; at: [number, number]; add: boolean } | null>(null);

  /** Puts moved points in place, keeping the constraints true; says which linked dimensions held. */
  const applyMoves = useCallback(
    (to: Map<number, [number, number]>, o?: Parameters<typeof rewrite>[4]) => {
      if (!data || !to.size) return;
      const r = rewrite(data.geometry, data.params, solved, to, o);
      commit(r.g, r.params);
      if (r.linked.length) props.notify(`Linked ${r.linked.length === 1 ? "dimension" : "dimensions"} kept: ${r.linked.join(", ")}`);
    },
    [data, solved, commit, props],
  );

  /** Rotate, mirror or scale the selection about the middle of its box. */
  const transform = useCallback(
    (how: { rotate?: number; mirror?: "x" | "y"; scale?: number }) => {
      if (!data || !selection.size) return;
      const g = data.geometry;
      const objs = objects(g, selection);
      const c = extent(g, solved, objs.flatMap((o) => o.curves), objs.flatMap((o) => o.points)).center;
      const f = how.rotate !== undefined ? rotateAbout(c, how.rotate) : how.mirror ? mirrorAbout(c, how.mirror) : scaleAbout(c, how.scale ?? 1);
      const circles = new Set(g.curves.filter((x) => x.kind === "circle" && selection.has(x.id)).map((x) => x.id));
      applyMoves(mapPoints(g, solved, movedPoints(g, selection), f), { mirror: !!how.mirror, scale: how.scale, circles });
    },
    [data, solved, selection, applyMoves],
  );

  const lastScreen = useRef<[number, number]>([0, 0]);
  useEffect(() => {
    const track = (e: PointerEvent) => (lastScreen.current = [e.clientX, e.clientY]);
    window.addEventListener("pointermove", track);
    return () => window.removeEventListener("pointermove", track);
  }, []);

  /**
   * The pen: press and release on the spot for a corner, press and drag to
   * pull out smooth handles (the incoming one mirrors the outgoing). Each new
   * anchor adds a line (corner to corner) or a Bézier from the last; a smooth
   * anchor keeps its two curves joined smoothly. Clicking the first anchor closes.
   */
  const penPointer = useCallback(
    (e: SketchPointer, at: [number, number]) => {
      if (!data) return;
      const d = draft.kind === "pen" ? draft : { kind: "pen" as const, last: null, first: null, down: null };
      if (e.type === "move") {
        if (d.down) setDraft({ ...d, down: { ...d.down, now: at } });
        return;
      }
      if (e.type === "double") return;
      if (e.type === "down") {
        setDraft({ ...d, down: { at: e.point !== null ? (pos.get(e.point) ?? at) : at, point: e.point, now: at } });
        return;
      }
      // Released: place the anchor.
      if (!d.down) return;
      const anchor = d.down.at;
      const dragged = Math.hypot(at[0] - anchor[0], at[1] - anchor[1]) > Math.max(e.grid * 0.5, 1e-6);
      const out: [number, number] | null = dragged ? at : null;
      const into: [number, number] | null = dragged ? [2 * anchor[0] - at[0], 2 * anchor[1] - at[1]] : null;
      const closing = !!d.first && d.last !== null && (d.down.point !== null ? d.down.point === d.first.id : false);
      if (!d.last) {
        // The first anchor: nothing to draw yet.
        const a = { id: d.down.point, at: anchor, out, seg: null };
        setDraft({ kind: "pen", last: a, first: a, down: null });
        return;
      }
      let geo = data.geometry;
      const make = (p: { id: number | null; at: [number, number] }) => {
        if (p.id !== null) return p.id;
        const r = pointAt(geo, p.at[0], p.at[1], null);
        geo = r.g;
        return r.id;
      };
      const fromId = make(d.last);
      const toId = closing ? (d.first!.id ?? fromId) : make({ id: d.down.point, at: anchor });
      if (toId === fromId) return setDraft({ ...d, down: null });
      const third = (p: [number, number], q: [number, number]): [number, number] => [p[0] + (q[0] - p[0]) / 3, p[1] + (q[1] - p[1]) / 3];
      const toAt = closing ? d.first!.at : anchor;
      // Arriving at the first anchor when closing uses its incoming handle (the mirror of its outgoing).
      const arrive = closing ? (d.first!.out ? ([2 * d.first!.at[0] - d.first!.out[0], 2 * d.first!.at[1] - d.first!.out[1]] as [number, number]) : null) : into;
      let seg: number;
      if (d.last.out || arrive) {
        const r = addBezier(geo, fromId, toId, d.last.out ?? third(d.last.at, toAt), arrive ?? third(toAt, d.last.at));
        geo = r.g;
        seg = r.id;
      } else {
        const p2 = new Map(pos);
        for (const p of geo.points) if (!p2.has(p.id)) p2.set(p.id, [p.x, p.y]);
        geo = addLine(geo, fromId, toId, p2);
        seg = geo.curves[geo.curves.length - 1].id;
      }
      // Smooth anchors keep their curves joined smoothly.
      if (d.last.out && d.last.seg !== null) geo = { ...geo, constraints: [...geo.constraints, { kind: "tangent", a: d.last.seg, b: seg, at: fromId }] };
      const firstSeg = d.first!.seg ?? seg;
      if (closing && d.first!.out && firstSeg !== seg) geo = { ...geo, constraints: [...geo.constraints, { kind: "tangent", a: seg, b: firstSeg, at: toId }] };
      commit(geo);
      if (closing) return setDraft({ kind: "none" });
      const first = d.first!.id === null && d.first === d.last ? { ...d.first!, id: fromId, seg } : d.first!.seg === null ? { ...d.first!, seg } : d.first!;
      setDraft({ kind: "pen", first, last: { id: toId, at: anchor, out, seg }, down: null });
    },
    [data, draft, pos, commit],
  );

  const onPointer = useCallback(
    (e: SketchPointer) => {
      if (!data) return;
      const g = data.geometry;
      const at: [number, number] = [e.u, e.v];
      setHover({ point: e.point, curve: e.curve, at, grid: e.grid });
      if (tool === "pen") {
        penPointer(e, at);
        return;
      }
      if (e.type === "move") {
        if (dragging.current) {
          dragging.current.at = at;
          setDraft({ kind: "drag", point: dragging.current.point, at });
        }
        const sw = sweeping.current;
        if (sw) {
          sw.at = sw.kind === "box" ? [e.u, e.v] : at;
          setDraft(sw.kind === "box" ? { kind: "box", from: sw.from, at: sw.at, add: sw.add } : { kind: "move", from: sw.from, at: sw.at });
        }
        return;
      }
      if (e.type === "double") {
        if (tool === "line") setDraft({ kind: "none" });
        return;
      }
      if (e.type === "up" && sweeping.current) {
        const sw = sweeping.current;
        sweeping.current = null;
        setDraft({ kind: "none" });
        const end: [number, number] = sw.kind === "box" ? [e.u, e.v] : at;
        if (sw.kind === "move") {
          const d: [number, number] = [end[0] - sw.from[0], end[1] - sw.from[1]];
          if (Math.hypot(d[0], d[1]) > 1e-9) applyMoves(mapPoints(g, solved, movedPoints(g, selection), moveBy(d)));
        } else {
          // Left to right: what is inside the box. Right to left: what it touches.
          const hit = inBox(g, solved, sw.from, end, end[0] < sw.from[0]);
          setSelection(sw.add ? new Set([...selection, ...hit]) : hit);
        }
        return;
      }
      if (e.type === "up") {
        const d = dragging.current;
        dragging.current = null;
        if (d) {
          const end = at;
          // A pinned point moves its pin with it.
          const moved = { ...g, constraints: g.constraints.map((c) => (c.kind === "fixed" && c.point === d.point ? { ...c, x: end[0], y: end[1] } : c)) };
          if (Math.hypot(end[0] - (pos.get(d.point)?.[0] ?? end[0]), end[1] - (pos.get(d.point)?.[1] ?? end[1])) > 1e-9) commitDrag(moved, d.point, end);
          setDraft({ kind: "none" });
        }
        return;
      }
      // A click ("down").
      switch (tool) {
        case "trim":
        case "extend":
          if (e.curve !== null && editing) void run(tool === "trim" ? "sketch.trim" : "sketch.extend", { target: editing, curve: e.curve, at });
          break;
        case "select": {
          const id = e.point ?? e.curve;
          if (id === null) {
            // Empty space: a selection box.
            sweeping.current = { kind: "box", from: [e.u, e.v], at: [e.u, e.v], add: e.shift };
            setDraft({ kind: "box", from: [e.u, e.v], at: [e.u, e.v], add: e.shift });
            break;
          }
          if (e.shift) {
            const next = new Set(selection);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            setSelection(next);
            break;
          }
          if (selection.has(id) && selection.size > 1) {
            // Part of a larger selection: drag them all.
            sweeping.current = { kind: "move", from: at, at, add: false };
            setDraft({ kind: "move", from: at, at });
            break;
          }
          setSelected(id);
          if (e.point !== null) {
            dragging.current = { point: e.point, at };
            setDraft({ kind: "drag", point: e.point, at });
          } else {
            // A curve: drag moves it whole.
            setSelection(new Set([id]));
            sweeping.current = { kind: "move", from: at, at, add: false };
            setDraft({ kind: "move", from: at, at });
          }
          break;
        }
        case "line": {
          if (draft.kind !== "line" || draft.from === null) {
            // The first point: an existing one, or a new one (-1) made with the first line.
            setDraft({ kind: "line", from: e.point ?? -1, at, first: e.point ?? -1 });
            break;
          }
          // Second and later clicks: a line from the previous point.
          let geo = g;
          let from = draft.from;
          if (from === -1) {
            const r = pointAt(geo, draft.at[0], draft.at[1], null);
            geo = r.g;
            from = r.id;
          }
          const first = draft.first === -1 ? from : draft.first;
          const end = pointAt(geo, e.u, e.v, e.point);
          if (end.id === from) break;
          geo = end.g;
          const p2 = new Map(pos);
          for (const p of geo.points) if (!p2.has(p.id)) p2.set(p.id, [p.x, p.y]);
          geo = addLine(geo, from, end.id, p2);
          commit(geo);
          // Back on the first point: the loop is closed and the chain ends.
          if (end.id === first && first !== null) setDraft({ kind: "none" });
          else setDraft({ kind: "line", from: end.id, at: pos.get(end.id) ?? at, first });
          break;
        }
        case "rect":
          if (draft.kind !== "rect") setDraft({ kind: "rect", at });
          else {
            if (Math.abs(at[0] - draft.at[0]) > 1e-9 && Math.abs(at[1] - draft.at[1]) > 1e-9) commit(addRectangle(g, draft.at, at));
            setDraft({ kind: "none" });
          }
          break;
        case "circle":
          if (draft.kind !== "circle") setDraft({ kind: "circle", center: e.point, at });
          else {
            const r = pointAt(g, draft.at[0], draft.at[1], draft.center);
            const radius = Math.hypot(at[0] - draft.at[0], at[1] - draft.at[1]);
            if (radius > 1e-6) commit(addCircle(r.g, r.id, radius));
            setDraft({ kind: "none" });
          }
          break;
        case "arc":
          if (draft.kind !== "arc") setDraft({ kind: "arc", center: at, centerId: e.point });
          else if (!draft.start) setDraft({ ...draft, start: { id: e.point, at } });
          else {
            let geo = g;
            const c = pointAt(geo, draft.center[0], draft.center[1], draft.centerId);
            geo = c.g;
            const s = pointAt(geo, draft.start.at[0], draft.start.at[1], draft.start.id);
            geo = s.g;
            // The end lands on the circle through the start.
            const r = Math.hypot(draft.start.at[0] - draft.center[0], draft.start.at[1] - draft.center[1]);
            const ang = Math.atan2(at[1] - draft.center[1], at[0] - draft.center[0]);
            const endAt: [number, number] = e.point !== null ? at : [draft.center[0] + r * Math.cos(ang), draft.center[1] + r * Math.sin(ang)];
            const en = pointAt(geo, endAt[0], endAt[1], e.point);
            geo = addArc(en.g, c.id, s.id, en.id);
            commit(geo);
            setDraft({ kind: "none" });
          }
          break;
        case "dimension": {
          const [sx, sy] = lastScreen.current;
          if (draft.kind === "dimension" && e.point !== null && e.point !== draft.from) {
            const [p, q] = [pos.get(draft.from)!, pos.get(e.point)!];
            addDimension("distance", draft.from, e.point, Math.hypot(q[0] - p[0], q[1] - p[1]), sx, sy);
            setDraft({ kind: "none" });
            break;
          }
          if (e.point !== null) {
            setDraft({ kind: "dimension", from: e.point });
            break;
          }
          const c = g.curves.find((x) => x.id === e.curve);
          if (!c) break;
          if (c.kind === "line") {
            const [p, q] = [pos.get(c.a)!, pos.get(c.b)!];
            addDimension("length", c.id, null, Math.hypot(q[0] - p[0], q[1] - p[1]), sx, sy);
          } else if (c.kind === "circle") addDimension("diameter", c.id, null, 2 * radiusOf(c, solved), sx, sy);
          else if (c.kind === "arc") {
            const [ce, st] = [pos.get(c.center)!, pos.get(c.start)!];
            addDimension("radius", c.id, null, Math.hypot(st[0] - ce[0], st[1] - ce[1]), sx, sy);
          }
          break;
        }
      }
    },
    [data, tool, draft, pos, commit, commitDrag, addDimension, solved, penPointer, selection, setSelected, applyMoves, editing, run],
  );

  // Keys: tools, Escape, Delete.
  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest("input, textarea")) return;
      const t = TOOLS.find((x) => x.key === e.key.toUpperCase());
      if (t && !e.metaKey && !e.ctrlKey) {
        setTool(t.tool);
        setDraft({ kind: "none" });
        e.preventDefault();
      } else if (e.key === "Enter" && draft.kind === "pen") {
        e.preventDefault();
        setDraft({ kind: "none" });
      } else if (e.key === "Escape") {
        e.stopPropagation();
        if (draft.kind !== "none") setDraft({ kind: "none" });
        else if (selection.size) setSelection(new Set());
        else finish();
      } else if ((e.key === "Delete" || e.key === "Backspace") && selection.size && data) {
        let r = { g: data.geometry, params: data.params };
        for (const id of selection) r = remove(r.g, r.params, id);
        commit(r.g, r.params);
        setSelection(new Set());
        e.preventDefault();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a" && data) {
        e.preventDefault();
        setTool("select");
        setSelection(new Set([...data.geometry.curves.map((c) => c.id), ...data.geometry.points.filter((p) => !data.geometry.curves.some((c) => curvePoints(c).includes(p.id))).map((p) => p.id)]));
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "d" && data && selection.size) {
        e.preventDefault();
        const step = hover.grid * 2;
        const r = duplicate(data.geometry, data.params, solved, selection, [step, -step]);
        commit(r.g, r.params);
        setSelection(r.ids);
      } else if (e.key.startsWith("Arrow") && data && selection.size && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        const k = e.shiftKey ? 10 : 1;
        const d: [number, number] = e.key === "ArrowLeft" ? [-k, 0] : e.key === "ArrowRight" ? [k, 0] : e.key === "ArrowUp" ? [0, k] : [0, -k];
        applyMoves(mapPoints(data.geometry, solved, movedPoints(data.geometry, selection), moveBy(d)));
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [editing, draft, selection, data, solved, hover.grid, commit, finish, applyMoves]);

  // What the current tool would add, drawn dashed.
  const preview = useMemo((): Segment[] => {
    const cur = hover.at;
    if (!cur) return [];
    const from = (d: { from: number | null; at: [number, number] }) => (d.from !== null && d.from >= 0 ? (pos.get(d.from) ?? d.at) : d.at);
    switch (draft.kind) {
      case "line":
        return draft.from === null ? [] : [{ kind: "line", from: from(draft), to: cur }];
      case "rect": {
        const [a, b] = [draft.at, cur];
        return [
          { kind: "line", from: [a[0], a[1]], to: [b[0], a[1]] },
          { kind: "line", from: [b[0], a[1]], to: [b[0], b[1]] },
          { kind: "line", from: [b[0], b[1]], to: [a[0], b[1]] },
          { kind: "line", from: [a[0], b[1]], to: [a[0], a[1]] },
        ];
      }
      case "circle":
        return [{ kind: "circle", center: draft.at, radius: Math.max(0.01, Math.hypot(cur[0] - draft.at[0], cur[1] - draft.at[1])) }];
      case "arc": {
        if (!draft.start) return [{ kind: "line", from: draft.center, to: cur }];
        return [{ kind: "arc", center: draft.center, from: draft.start.at, to: cur, ccw: true }];
      }
      case "pen": {
        const segs: Segment[] = [];
        const last = draft.last;
        const down = draft.down;
        // While dragging out handles: the handle line, mirrored through the anchor.
        if (down) {
          const [a, n] = [down.at, down.now];
          if (Math.hypot(n[0] - a[0], n[1] - a[1]) > 1e-9) segs.push({ kind: "line", from: [2 * a[0] - n[0], 2 * a[1] - n[1]], to: n });
        }
        if (last) {
          const to = down ? down.at : cur;
          const into: [number, number] | null = down && Math.hypot(down.now[0] - down.at[0], down.now[1] - down.at[1]) > 1e-9 ? [2 * down.at[0] - down.now[0], 2 * down.at[1] - down.now[1]] : null;
          if (last.out || into) {
            const third = (p: [number, number], q: [number, number]): [number, number] => [p[0] + (q[0] - p[0]) / 3, p[1] + (q[1] - p[1]) / 3];
            segs.push({ kind: "bezier", from: last.at, to, c1: last.out ?? third(last.at, to), c2: into ?? third(to, last.at) });
          } else segs.push({ kind: "line", from: last.at, to });
          if (last.out) segs.push({ kind: "line", from: last.at, to: last.out });
        }
        return segs;
      }
      case "box": {
        const [a, b] = [draft.from, draft.at];
        return [
          { kind: "line", from: [a[0], a[1]], to: [b[0], a[1]] },
          { kind: "line", from: [b[0], a[1]], to: [b[0], b[1]] },
          { kind: "line", from: [b[0], b[1]], to: [a[0], b[1]] },
          { kind: "line", from: [a[0], b[1]], to: [a[0], a[1]] },
        ];
      }
      case "move": {
        // The selected curves where they would land.
        const d: [number, number] = [draft.at[0] - draft.from[0], draft.at[1] - draft.from[1]];
        const P = (id: number): [number, number] => {
          const p = pos.get(id)!;
          return [p[0] + d[0], p[1] + d[1]];
        };
        const segs: Segment[] = [];
        for (const c of data?.geometry.curves ?? []) {
          if (!selection.has(c.id)) continue;
          if (c.kind === "line") segs.push({ kind: "line", from: P(c.a), to: P(c.b) });
          else if (c.kind === "bezier") segs.push({ kind: "bezier", from: P(c.a), to: P(c.b), c1: P(c.c1), c2: P(c.c2) });
          else if (c.kind === "circle") segs.push({ kind: "circle", center: P(c.center), radius: radiusOf(c, solved) });
          else segs.push({ kind: "arc", center: P(c.center), from: P(c.start), to: P(c.end), ccw: true });
        }
        return segs;
      }
      case "drag": {
        const segs: Segment[] = [];
        const P = (id: number) => (id === draft.point ? draft.at : pos.get(id)!);
        for (const c of data?.geometry.curves ?? []) {
          if (c.kind === "line" && (c.a === draft.point || c.b === draft.point)) {
            segs.push({ kind: "line", from: P(c.a), to: P(c.b) });
          } else if (c.kind === "bezier" && [c.a, c.b, c.c1, c.c2].includes(draft.point)) {
            segs.push({ kind: "bezier", from: P(c.a), to: P(c.b), c1: P(c.c1), c2: P(c.c2) }, { kind: "line", from: P(c.a), to: P(c.c1) }, { kind: "line", from: P(c.b), to: P(c.c2) });
          }
        }
        return segs;
      }
      default:
        return [];
    }
  }, [draft, hover.at, pos, data, selection, solved]);

  const picture: SketchPicture | null = useMemo(
    () =>
      data
        ? { data, solved, hoverPoint: hover.point, hoverCurve: hover.curve, selected: [...selection], preview, cursor: tool === "select" ? null : hover.at, grid: hover.grid }
        : null,
    [data, solved, hover, selection, preview, tool],
  );

  const [angleText, setAngleText] = useState("45");
  const [scaleText, setScaleText] = useState("150");
  const [offsetText, setOffsetText] = useState("5");
  const [sharp, setSharp] = useState(false);
  /** Pathfinder and offset run in the engine (exact); the result becomes the selection. */
  const exact = async (operation: "sketch.combine" | "sketch.offset", extra: Record<string, unknown>) => {
    if (!editing) return;
    const curves = data ? data.geometry.curves.filter((c) => selection.has(c.id) || curvePoints(c).some((p) => selection.has(p))).map((c) => c.id) : [];
    const out = (await run(operation, { target: editing, curves, ...extra })) as { output?: { curves?: number[] } } | null;
    if (out?.output?.curves) setSelection(new Set(out.output.curves));
  };
  const objectCount = data && selection.size ? objects(data.geometry, selection).length : 0;
  const removeSelection = () => {
    if (!data) return;
    let r = { g: data.geometry, params: data.params };
    for (const id of selection) r = remove(r.g, r.params, id);
    commit(r.g, r.params);
    setSelection(new Set());
  };
  const lineUp = (how: Align) => data && applyMoves(align(data.geometry, solved, selection, how));
  const IB = (p: { icon: Parameters<typeof Icon>[0]["name"]; label: string; onClick: () => void }) => (
    <button type="button" className="sel-btn" title={p.label} aria-label={p.label} onClick={p.onClick}>
      <Icon name={p.icon} />
    </button>
  );
  // Constraints that fit what is selected.
  const constrainBar = (() => {
    if (!data) return null;
    const g = data.geometry;
    const curveOf = (id: number) => g.curves.find((c) => c.id === id);
    const ids = [...selection];
    const lines = ids.filter((id) => curveOf(id)?.kind === "line");
    const rounds = ids.filter((id) => ["circle", "arc"].includes(curveOf(id)?.kind ?? ""));
    const points = ids.filter((id) => g.points.some((p) => p.id === id) && !curveOf(id));
    const others = ids.length - lines.length - rounds.length - points.length;
    type C = Geometry["constraints"][number];
    const options: { glyph: IconName; label: string; make: () => C[] }[] = [];
    const only = (l: number, r: number, p: number) => lines.length === l && rounds.length === r && points.length === p && others === 0;
    if (lines.length >= 1 && rounds.length === 0 && points.length === 0 && others === 0) {
      options.push({ glyph: "cHorizontal", label: "Horizontal", make: () => lines.map((line) => ({ kind: "horizontal", line }) as C) });
      options.push({ glyph: "cVertical", label: "Vertical", make: () => lines.map((line) => ({ kind: "vertical", line }) as C) });
    }
    if (only(2, 0, 0)) {
      const [a, b] = lines as [number, number];
      options.push({ glyph: "cParallel", label: "Parallel", make: () => [{ kind: "parallel", a, b } as C] });
      options.push({ glyph: "cPerpendicular", label: "Perpendicular", make: () => [{ kind: "perpendicular", a, b } as C] });
      options.push({ glyph: "cEqual", label: "Equal length", make: () => [{ kind: "equal", a, b } as C] });
    }
    if (only(0, 2, 0)) {
      const [a, b] = rounds as [number, number];
      const centre = (id: number) => {
        const c = curveOf(id)!;
        return c.kind === "circle" || c.kind === "arc" ? c.center : -1;
      };
      options.push({ glyph: "cEqual", label: "Equal radius", make: () => [{ kind: "equal", a, b } as C] });
      options.push({ glyph: "cConcentric", label: "Concentric (same centre)", make: () => [{ kind: "coincident", a: centre(a), b: centre(b) } as C] });
      options.push({ glyph: "cTangent", label: "Tangent", make: () => [{ kind: "tangent", a, b } as C] });
    }
    if (only(1, 1, 0)) options.push({ glyph: "cTangent", label: "Tangent", make: () => [{ kind: "tangent", a: lines[0]!, b: rounds[0]! } as C] });
    if (only(0, 0, 2)) options.push({ glyph: "cCoincident", label: "Coincident (join the points)", make: () => [{ kind: "coincident", a: points[0]!, b: points[1]! } as C] });
    if (points.length === 1 && lines.length + rounds.length === 1 && others === 0) {
      const curve = (lines[0] ?? rounds[0])!;
      options.push({ glyph: "cOnCurve", label: "On the curve", make: () => [{ kind: "on_curve", point: points[0]!, curve } as C] });
      if (lines.length === 1) options.push({ glyph: "cMidpoint", label: "Midpoint of the line", make: () => [{ kind: "midpoint", point: points[0]!, line: lines[0]! } as C] });
    }
    if (points.length >= 1 && lines.length + rounds.length + others === 0)
      options.push({
        glyph: "cFix",
        label: "Fix where it is",
        make: () => points.map((point) => ({ kind: "fixed", point, x: pos.get(point)?.[0] ?? 0, y: pos.get(point)?.[1] ?? 0 }) as C),
      });
    const touches = (c: C) => {
      const v = Object.entries(c).filter(([k]) => !["kind", "key", "x", "y"].includes(k)).map(([, x]) => x);
      return v.some((x) => typeof x === "number" && selection.has(x));
    };
    const loose = g.constraints.filter((c) => !("key" in c) && touches(c));
    if (!options.length && !loose.length) return null;
    const same = (a: C, b: C) => JSON.stringify(a) === JSON.stringify(b);
    return (
      <div className="sel-group sel-constrain" aria-label="Constraints">
        {options.map((o) => (
          <button
            key={o.label}
            type="button"
            className="sel-btn"
            title={o.label}
            aria-label={o.label}
            onClick={() => {
              const add = o.make().filter((c) => !g.constraints.some((x) => same(x, c)));
              if (!add.length) return props.notify(`Already ${o.label.toLowerCase()}`);
              void commit({ ...g, constraints: [...g.constraints, ...add] });
            }}
          >
            <Icon name={o.glyph} />
          </button>
        ))}
        {loose.length > 0 && (
          <button type="button" className="sel-glyph sel-free" title={`Remove ${loose.length} constraint${loose.length === 1 ? "" : "s"} on these (dimensions stay)`} aria-label="Remove constraints" onClick={() => void commit({ ...g, constraints: g.constraints.filter((c) => !loose.includes(c)) })}>
            ✕ {loose.length}
          </button>
        )}
      </div>
    );
  })();

  const selectionBar = data && selection.size > 0 && (
    <div className="sketch-selection" role="toolbar" aria-label="Transform the selection">
      <span className="sel-count">
        {objectCount} {objectCount === 1 ? "object" : "objects"}
      </span>
      {constrainBar}
      <div className="sel-group">
        <IB icon="rotate" label="Rotate 90° anticlockwise" onClick={() => transform({ rotate: 90 })} />
        <IB icon="rotateCw" label="Rotate 90° clockwise" onClick={() => transform({ rotate: -90 })} />
        <form
          className="sel-field"
          onSubmit={(e) => {
            e.preventDefault();
            const v = Number(angleText);
            if (Number.isFinite(v) && v !== 0) transform({ rotate: v });
          }}
        >
          <input value={angleText} inputMode="decimal" aria-label="Rotate by degrees (anticlockwise)" onChange={(e) => setAngleText(e.target.value)} />
          <button type="submit" title="Rotate by this angle, anticlockwise">°</button>
        </form>
      </div>
      <div className="sel-group">
        <IB icon="flipH" label="Mirror left to right" onClick={() => transform({ mirror: "x" })} />
        <IB icon="flipV" label="Mirror top to bottom" onClick={() => transform({ mirror: "y" })} />
      </div>
      <div className="sel-group">
        <form
          className="sel-field"
          onSubmit={(e) => {
            e.preventDefault();
            const v = Number(scaleText) / 100;
            if (Number.isFinite(v) && v > 0 && v !== 1) transform({ scale: v });
          }}
        >
          <Icon name="scale" />
          <input value={scaleText} inputMode="decimal" aria-label="Scale by percent" onChange={(e) => setScaleText(e.target.value)} />
          <button type="submit" title="Scale by this percentage">%</button>
        </form>
      </div>
      {objectCount >= 2 && (
        <div className="sel-group">
          <IB icon="unite" label="Unite: one outline around them all" onClick={() => exact("sketch.combine", { op: "unite" })} />
          <IB icon="subtract" label="Subtract: cut the others out of the largest" onClick={() => exact("sketch.combine", { op: "subtract" })} />
          <IB icon="intersect" label="Intersect: keep only where they overlap" onClick={() => exact("sketch.combine", { op: "intersect" })} />
        </div>
      )}
      <div className="sel-group">
        <form
          className="sel-field"
          onSubmit={(e) => {
            e.preventDefault();
            const v = Number(offsetText);
            if (Number.isFinite(v) && v !== 0) exact("sketch.offset", { distance: v, corners: sharp ? "sharp" : "round" });
          }}
        >
          <Icon name="offset" />
          <input value={offsetText} inputMode="decimal" aria-label="Offset distance in mm (negative: inside)" onChange={(e) => setOffsetText(e.target.value)} />
          <button type="button" className={`sel-corner ${sharp ? "" : "on"}`} title={sharp ? "Sharp corners (click for round)" : "Round corners (click for sharp)"} aria-label="Corner style" onClick={() => setSharp((v) => !v)}>
            {sharp ? "⌐" : "◜"}
          </button>
          <button type="submit" title="Add an outline this far outside (negative: inside)">mm</button>
        </form>
      </div>
      {objectCount >= 2 && (
        <div className="sel-group">
          <IB icon="alignLeft" label="Align left edges" onClick={() => lineUp("left")} />
          <IB icon="alignHCenter" label="Align centres horizontally" onClick={() => lineUp("hcenter")} />
          <IB icon="alignRight" label="Align right edges" onClick={() => lineUp("right")} />
          <IB icon="alignTop" label="Align top edges" onClick={() => lineUp("top")} />
          <IB icon="alignVCenter" label="Align centres vertically" onClick={() => lineUp("vcenter")} />
          <IB icon="alignBottom" label="Align bottom edges" onClick={() => lineUp("bottom")} />
        </div>
      )}
      {objectCount >= 3 && (
        <div className="sel-group">
          <IB icon="distH" label="Space evenly left to right" onClick={() => applyMoves(distribute(data.geometry, solved, selection, "x"))} />
          <IB icon="distV" label="Space evenly bottom to top" onClick={() => applyMoves(distribute(data.geometry, solved, selection, "y"))} />
        </div>
      )}
      <div className="sel-group">
        <IB
          icon="copy"
          label="Copy (⌘D)"
          onClick={() => {
            const step = hover.grid * 2;
            const r = duplicate(data.geometry, data.params, solved, selection, [step, -step]);
            commit(r.g, r.params);
            setSelection(r.ids);
          }}
        />
        <IB icon="trash" label="Delete" onClick={removeSelection} />
      </div>
    </div>
  );

  const st = status(entity);
  const bar = data ? (
    <div className="sketch-bar floating" role="toolbar" aria-label="Sketch tools">
      <div className="sketch-head">
        <strong>{data.name}</strong>
        <span className={`sketch-status ${st.tone}`}>{st.text}</span>
      </div>
      <div className="sketch-tools">
        {TOOLS.map((t) => (
          <button
            key={t.tool}
            type="button"
            className={tool === t.tool ? "active" : ""}
            title={`${t.label} (${t.key}) · ${t.hint}`}
            onClick={() => {
              setTool(t.tool);
              setDraft({ kind: "none" });
            }}
          >
            {t.label}
            <kbd>{t.key}</kbd>
          </button>
        ))}
      </div>
      <p className="sketch-hint">{TOOLS.find((t) => t.tool === tool)?.hint} · Alt turns off grid snap</p>
      {selectionBar}
      <div className="sketch-actions">
        <button type="button" className="primary" onClick={() => make("extrude")} disabled={!solved?.loops.length}>
          <Icon name="cube" /> Extrude
        </button>
        <button type="button" className="secondary" onClick={() => make("revolve")} disabled={!solved?.loops.length}>
          <Icon name="rotate" /> Revolve
        </button>
        <button type="button" className="secondary" onClick={finish}>
          Done
        </button>
      </div>
    </div>
  ) : null;

  const dimEditor = editDim && data && (
    <form
      className="dim-editor"
      style={{ left: editDim.x, top: editDim.y }}
      onSubmit={(e) => {
        e.preventDefault();
        const text = editDim.text.trim();
        if (!text || !editing) return setEditDim(null);
        if (text.startsWith("=")) run("shape.link_param", { target: editing, key: editDim.key, expr: text });
        else {
          const v = Number(text.replace(/mm$/, ""));
          if (!Number.isFinite(v)) return props.notify("Type a number, or a formula starting with =");
          const linked = data.params.find((p) => p.key === editDim.key)?.expr;
          (linked ? run("shape.link_param", { target: editing, key: editDim.key, expr: null }) : Promise.resolve()).then(() =>
            run("parameters.set", { target: editing, values: { [editDim.key]: v } }),
          );
        }
        setEditDim(null);
      }}
    >
      <input
        id="dim-editor-input"
        autoFocus
        value={editDim.text}
        aria-label="Dimension value (a number, or =formula)"
        onChange={(e) => setEditDim({ ...editDim, text: e.target.value })}
        onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), setEditDim(null))}
        onBlur={(e) => e.currentTarget.form?.requestSubmit()}
      />
      <small>Enter a number, or =formula</small>
    </form>
  );

  const openDim = useCallback(
    (key: string, x: number, y: number) => {
      const d = data?.params.find((p) => p.key === key);
      if (d) setEditDim({ key, x, y, text: d.expr ?? String(d.value) });
    },
    [data],
  );

  return { editing, picture, onPointer, bar, dimEditor, openDim, start, edit, finish };
}

/** Choose a plane for a new sketch. */
export function PlaneMenu(props: { onPick: (p: keyof typeof PLANES) => void; onClose: () => void }) {
  return (
    <div className="plane-menu floating" role="menu" aria-label="Sketch on which plane">
      <strong>New sketch on…</strong>
      {(Object.keys(PLANES) as (keyof typeof PLANES)[]).map((k) => (
        <button key={k} type="button" role="menuitem" onClick={() => props.onPick(k)}>
          <span className={`plane-chip plane-${k.toLowerCase()}`}>{k}</span>
          {PLANES[k].label}
        </button>
      ))}
      <button type="button" className="plane-cancel" onClick={props.onClose}>
        Cancel
      </button>
    </div>
  );
}
