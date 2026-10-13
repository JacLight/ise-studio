// Sketching by hand. A small state machine per tool turns clicks on the
// sketch plane into geometry; each finished action is one `sketch.update`
// (one undo step) and the engine solves the result. Dimensions are typed
// onto the drawing: a number, or a formula like =Desk size.width - 40.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { EntityView } from "./engine";
import { Icon, type IconName } from "./icons";
import { curveTrace, knotList } from "./sketchCurves";
import type { SketchPicture } from "./sketchLayer";
import {
  type Constraint,
  type Curve,
  type Dim,
  type Geometry,
  PLANES,
  type Plane,
  type Segment,
  addArc,
  addBezier,
  addCircle,
  addLine,
  addRectangle,
  curveEnds,
  curvePoints,
  describe,
  innerOf,
  keyOf,
  newKey,
  pointAt,
  positions,
  radiusOf,
  refsOf,
  remove,
  sketchOf,
  solvedOf,
  status,
} from "./sketchModel";
import { type Click, FAMILIES, type Tool, buildShape, clicksFor, familyOf, isClickTool, previewShape, toolInfo } from "./sketchTools";
import { type CornerPick, type PatternSpec, chamferPreview, filletPreview, isCorner, mappedSegments, patternPreview, reflectAbout } from "./sketchOps";
import { type Align, align, distribute, duplicate, extent, inBox, mapPoints, mirrorAbout, moveBy, movedPoints, objects, rewrite, rotateAbout, scaleAbout } from "./sketchTransform";
import type { SketchPointer } from "./Viewport";

type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
export type { Tool };
type P = [number, number];

/** What `sketch.validate` found. */
type Validation = {
  gaps?: { a: number; b: number; distance: number }[];
  degenerate?: number[];
  duplicates?: number[];
  duplicate_constraints?: number[];
  open_ends?: number[];
  conflicts?: number[];
  redundant?: number[];
  malformed?: number[];
  fixed?: { joined?: number; removed_curves?: number; removed_constraints?: number };
};

/** Distance (mm) between two things of a sketch as drawn: points, lines, circles/arcs, or a point and any curve. */
function measureBetween(g: Geometry, pos: Map<number, P>, solvedRadius: (c: Curve) => number, traceOf: (c: Curve) => P[], a: number, b: number): number {
  const curve = (id: number) => g.curves.find((c) => c.id === id);
  const pt = (id: number) => pos.get(id)!;
  const isPoint = (id: number) => pos.has(id) && !curve(id);
  const lineOf = (id: number) => {
    const c = curve(id);
    return c?.kind === "line" ? ([pt(c.a), pt(c.b)] as [P, P]) : null;
  };
  const round = (id: number) => {
    const c = curve(id);
    return c?.kind === "circle" || c?.kind === "arc" ? { c: pt(c.center), r: solvedRadius(c) } : null;
  };
  const toLine = (p: P, [s, e]: [P, P]) => {
    const d: P = [e[0] - s[0], e[1] - s[1]];
    return Math.abs(d[0] * (p[1] - s[1]) - d[1] * (p[0] - s[0])) / (Math.hypot(d[0], d[1]) || 1);
  };
  if (isPoint(a) && isPoint(b)) return Math.hypot(pt(a)[0] - pt(b)[0], pt(a)[1] - pt(b)[1]);
  if (isPoint(a) || isPoint(b)) {
    const [p, c] = isPoint(a) ? [a, b] : [b, a];
    const l = lineOf(c);
    if (l) return toLine(pt(p), l);
    const r = round(c);
    if (r) return Math.abs(Math.hypot(pt(p)[0] - r.c[0], pt(p)[1] - r.c[1]) - r.r);
    const tr = traceOf(curve(c)!);
    return Math.min(...tr.map((q) => Math.hypot(q[0] - pt(p)[0], q[1] - pt(p)[1])));
  }
  const [la, lb, ra, rb] = [lineOf(a), lineOf(b), round(a), round(b)];
  if (la && lb) return toLine([(lb[0][0] + lb[1][0]) / 2, (lb[0][1] + lb[1][1]) / 2], la);
  if ((la && rb) || (lb && ra)) {
    const [l, r] = la ? [la, rb!] : [lb!, ra!];
    return toLine(r.c, l) - r.r;
  }
  if (ra && rb) {
    const d = Math.hypot(ra.c[0] - rb.c[0], ra.c[1] - rb.c[1]);
    return d + Math.min(ra.r, rb.r) <= Math.max(ra.r, rb.r) ? Math.abs(ra.r - rb.r) - d : d - ra.r - rb.r;
  }
  return 0;
}

type Draft =
  | { kind: "none" }
  /** A tool that places a shape by clicks (sketchTools). */
  | { kind: "clicks"; pts: Click[] }
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
  /** Ask for a flat face of a part (the next one clicked); the sketch moves onto it. */
  requestFace?: (purpose: string, take: (f: { centroid: [number, number, number]; normal: [number, number, number] }) => void) => void;
  /** Parts hidden in the view, and the one isolated (for projecting what is visible). */
  hidden?: ReadonlySet<string>;
  isolated?: string | null;
}) {
  const { entities, run } = props;
  const [editing, setEditing] = useState<string | null>(null);
  const [tool, setTool] = useState<Tool>("line");
  const [draft, setDraft] = useState<Draft>({ kind: "none" });
  const [hover, setHover] = useState<{ point: number | null; curve: number | null; at: [number, number] | null; grid: number }>({ point: null, curve: null, at: null, grid: 10 });
  const [selection, setSelection] = useState<ReadonlySet<number>>(new Set());
  const setSelected = useCallback((id: number | null) => setSelection(new Set(id === null ? [] : [id])), []);
  const [editDim, setEditDim] = useState<{ key: string; x: number; y: number; text: string } | null>(null);
  // Fillet tool: round or chamfer, its size, the first of two curves picked, and a corner being dragged.
  const [corner, setCorner] = useState<"round" | "chamfer">("round");
  const [filletText, setFilletText] = useState("5");
  const [filletFirst, setFilletFirst] = useState<{ curve: number; at: [number, number] } | null>(null);
  const [filletDrag, setFilletDrag] = useState<{ point: number; size: number | null } | null>(null);
  const filletDragging = useRef<{ point: number; size: number | null } | null>(null);
  // Patterns and mirror: the open panel, its fields, and whether the mirror is being previewed.
  const [patternOpen, setPatternOpen] = useState<"linear" | "circular" | null>(null);
  const [pat, setPat] = useState({ count: "4", spacing: "20", direction: "0", angle: "90", full: true, linked: true });
  const [mirrorHover, setMirrorHover] = useState(false);
  // Projecting parts: the open panel and its choices.
  const [projectOpen, setProjectOpen] = useState(false);
  const [proj, setProj] = useState<{ picked: ReadonlySet<string>; mode: "outline" | "section"; construction: boolean; fixed: boolean; hidden: boolean }>({ picked: new Set(), mode: "outline", construction: false, fixed: true, hidden: false });
  // Drawing as construction, the click tools' options, and each family's last-used tool.
  const [constructionMode, setConstructionMode] = useState(false);
  const [facePick, setFacePick] = useState(false);
  const [opts, setOpts] = useState({ sides: "6", radius: "5", degree: "3" });
  const [lastOf, setLastOf] = useState<Record<string, Tool>>({});
  const [familyMenu, setFamilyMenu] = useState<string | null>(null);
  const [snapLabel, setSnapLabel] = useState<string | null>(null);
  // Side panels: the constraint list, checking the sketch, copying from another sketch.
  const [side, setSide] = useState<"constraints" | "validate" | "copy" | null>(null);
  const [lit, setLit] = useState<number[]>([]);
  const [polygons, setPolygons] = useState(false);
  const [combs, setCombs] = useState(false);
  const [knotPick, setKnotPick] = useState<number | null>(null);
  const [validation, setValidation] = useState<Validation | null>(null);
  const [copyFrom, setCopyFrom] = useState<{ from: string; linked: boolean }>({ from: "", linked: true });
  /** A dimension just added: if it over-constrains, it becomes a reference. */
  const pendingDim = useRef<{ index: number; key: string } | null>(null);

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
      // In construction mode, curves new in this edit are construction.
      if (constructionMode) {
        const before = new Set(data.geometry.curves.map((c) => c.id));
        geometry = { ...geometry, curves: geometry.curves.map((c) => (before.has(c.id) ? c : { ...c, construction: true })) };
      }
      return run("sketch.update", { target: editing, geometry, ...(params ? { params } : {}) }).catch(() => {});
    },
    [editing, data, run, constructionMode],
  );

  /** A shape the engine builds (sketch.add): the result becomes the selection. */
  const addShape = useCallback(
    async (shape: Record<string, unknown>) => {
      if (!editing) return;
      const out = (await run("sketch.add", { target: editing, shape, construction: constructionMode }).catch(() => null)) as { output?: { curves?: number[]; points?: number[] } } | null;
      const made = out?.output?.curves?.length ? out.output.curves : (out?.output?.points ?? []);
      if (made.length) setSelection(new Set(made));
    },
    [editing, run, constructionMode],
  );

  /** A dragged point: it stays where it was put, everything else adapts. */
  const commitDrag = useCallback(
    (geometry: Geometry, point: number, at: [number, number]) => {
      if (!editing) return Promise.resolve();
      return run("sketch.update", { target: editing, geometry, drag: { point, x: at[0], y: at[1] } }).catch(() => {});
    },
    [editing, run],
  );

  /** A new sketch: on a named plane (offset along its normal), or through a point square to a normal (a face). */
  const start = useCallback(
    async (plane: keyof typeof PLANES | { origin: [number, number, number]; normal: [number, number, number] }, offset = 0) => {
      const n = [...entities.values()].filter((e) => sketchOf(e)).length + 1;
      const spec = typeof plane === "string" ? { name: plane, offset } : { ...plane, offset };
      const out = (await run("sketch.create", { name: `Sketch ${n}`, plane: spec })) as { output?: { id: string } } | null;
      const id = out?.output?.id;
      if (!id) return;
      setEditing(id);
      setTool("line");
      setDraft({ kind: "none" });
      if (typeof plane === "string") {
        const p = PLANES[plane].plane;
        const nn = [p.x[1] * p.y[2] - p.x[2] * p.y[1], p.x[2] * p.y[0] - p.x[0] * p.y[2], p.x[0] * p.y[1] - p.x[1] * p.y[0]];
        props.onFace({ ...p, origin: [nn[0] * offset, nn[1] * offset, nn[2] * offset] });
      }
    },
    [entities, run, props],
  );
  // When the plane of the sketch being edited changes (made on a face, moved onto one), face it.
  const planeKey = data ? JSON.stringify(data.plane) : "";
  useEffect(() => {
    if (!editing || !planeKey) return;
    const was = faced.current.get(editing);
    faced.current.set(editing, planeKey);
    if (was !== undefined && was !== planeKey) props.onFace(JSON.parse(planeKey) as Plane);
    else if (was === undefined && data && !data.geometry.points.length) props.onFace(JSON.parse(planeKey) as Plane);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editing, planeKey]);

  /** Moves the sketch being edited onto a face clicked next (Map / reorient sketch). */
  const moveToFace = useCallback(() => {
    if (!editing || !props.requestFace) return;
    // The view picks parts and faces again until the face is clicked.
    setFacePick(true);
    props.notify("Select a part, then click the flat face to put the sketch on");
    props.requestFace("Sketch on this face", (f) => {
      setFacePick(false);
      void run("sketch.update", { target: editing, plane: { origin: f.centroid, normal: f.normal } });
    });
  }, [editing, props, run]);

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

  const faced = useRef(new Map<string, string>());

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
  /** The distance between two things as drawn (for a new dimension's value). */
  const measure = useCallback(
    (a: number, b: number) => {
      if (!data) return 0;
      const at = (id: number): P => pos.get(id) ?? [0, 0];
      const r = (c: Curve) => (c.kind === "circle" ? radiusOf(c, solved) : c.kind === "arc" ? Math.hypot(at(c.start)[0] - at(c.center)[0], at(c.start)[1] - at(c.center)[1]) : 0);
      return measureBetween(data.geometry, pos, r, (c) => curveTrace(c, at, solved), a, b);
    },
    [data, pos, solved],
  );
  /** Where the first click of a line or circle snapped (tied to it once the point exists). */
  const firstSnap = useRef<SketchPointer | null>(null);

  const addDimension = useCallback(
    (kind: "length" | "diameter" | "radius" | "distance" | "horizontal_distance" | "vertical_distance" | "angle", a: number, b: number | null, value: number, x: number, y: number) => {
      if (!data) return;
      const key = newKey(data.params);
      const label = { length: "Length", diameter: "Diameter", radius: "Radius", distance: "Distance", horizontal_distance: "X", vertical_distance: "Y", angle: "Angle" }[kind];
      const c = kind === "length" ? { kind, line: a, key } : kind === "diameter" || kind === "radius" ? { kind, curve: a, key } : { kind, a, b: b ?? a, key };
      const signed = kind === "horizontal_distance" || kind === "vertical_distance" || kind === "angle";
      const params = [...data.params, { key, label: `${label} ${key.slice(1)}`, value: Math.round(value * 100) / 100, unit: kind === "angle" ? "deg" : "mm", ...(signed ? {} : { min: 0.001 }) }];
      pendingDim.current = { index: data.geometry.constraints.length, key };
      commit({ ...data.geometry, constraints: [...data.geometry.constraints, c as Geometry["constraints"][number]] }, params).then(() =>
        setEditDim({ key, x, y, text: String(Math.round(value * 100) / 100) }),
      );
    },
    [data, commit],
  );

  // A dimension that would over-constrain the sketch becomes a reference (it measures instead), as in FreeCAD.
  useEffect(() => {
    const p = pendingDim.current;
    if (!p || !data || !solved) return;
    const c = data.geometry.constraints[p.index];
    if (!c || keyOf(c) !== p.key) return;
    pendingDim.current = null;
    const s = solved.solved;
    if (c.kind !== "reference" && (s.conflicts.includes(p.index) || s.redundant?.includes(p.index))) {
      const constraints = data.geometry.constraints.map((x, i) => (i === p.index ? ({ kind: "reference", of: x } as Constraint) : x));
      void run("sketch.update", { target: editing, geometry: { ...data.geometry, constraints } });
      setEditDim(null);
      props.notify("That dimension would over-constrain the sketch: it is kept as a reference (it measures, it does not drive)");
    }
  }, [data, solved, run, editing, props]);

  /** Changes one constraint: driving or reference, on or off, or removes it. */
  const changeConstraint = useCallback(
    (index: number, how: "reference" | "driving" | "off" | "on" | "remove") => {
      if (!data) return;
      const c = data.geometry.constraints[index];
      if (!c) return;
      if (how === "remove") {
        const constraints = data.geometry.constraints.filter((_, i) => i !== index);
        const keys = new Set(constraints.map(keyOf).filter(Boolean));
        setLit([]);
        return commit({ ...data.geometry, constraints }, data.params.filter((d) => keys.has(d.key)));
      }
      const off = c.kind === "inactive";
      const core = off ? c.of : c;
      const ref = core.kind === "reference";
      const plain = ref ? core.of : core;
      const isRef = how === "reference" ? true : how === "driving" ? false : ref;
      const isOff = how === "off" ? true : how === "on" ? false : off;
      let next: Constraint = isRef ? { kind: "reference", of: plain } : plain;
      if (isOff) next = { kind: "inactive", of: next };
      const constraints = data.geometry.constraints.map((x, i) => (i === index ? next : x));
      return commit({ ...data.geometry, constraints });
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

  /** Rounds (or bevels) a corner in the engine: exact, trimmed, its size a dimension. */
  const roundCorner = useCallback(
    (pick: CornerPick, size: number) => {
      if (!editing || !(size > 0)) return;
      const round = corner === "round";
      void run(round ? "sketch.fillet" : "sketch.chamfer", { target: editing, ...pick, [round ? "radius" : "distance"]: size });
    },
    [editing, corner, run],
  );
  const filletSize = Number(filletText);

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

  /** What a click snapped to, kept as a constraint on the new point (on a curve, at a midpoint). */
  const tie = (geo: Geometry, id: number, e: SketchPointer): Geometry => {
    const s = e.snap;
    if (!s || e.point !== null) return geo;
    const line = s.curve !== undefined ? geo.curves.find((c) => c.id === s.curve) : undefined;
    if (s.kind === "midpoint" && line?.kind === "line") return { ...geo, constraints: [...geo.constraints, { kind: "midpoint", point: id, line: line.id }] };
    if ((s.kind === "on_curve" || s.kind === "quadrant" || s.kind === "midpoint") && s.curve !== undefined) return { ...geo, constraints: [...geo.constraints, { kind: "on_curve", point: id, curve: s.curve }] };
    if (s.kind === "intersection" && s.curves) return { ...geo, constraints: [...geo.constraints, ...s.curves.map((curve) => ({ kind: "on_curve" as const, point: id, curve }))] };
    return geo;
  };

  const onPointer = useCallback(
    (e: SketchPointer) => {
      if (!data) return;
      const g = data.geometry;
      const at: [number, number] = [e.u, e.v];
      setHover({ point: e.point, curve: e.curve, at, grid: e.grid });
      setSnapLabel(e.snap && e.snap.kind !== "grid" ? e.snap.label : null);
      if (tool === "pen") {
        penPointer(e, at);
        return;
      }
      if (isClickTool(tool)) {
        if (e.type !== "down" && e.type !== "double") return;
        const pts = draft.kind === "clicks" ? draft.pts : [];
        const need = clicksFor(tool);
        const o = { sides: Math.max(3, Math.round(Number(opts.sides)) || 6), radius: Math.max(0.001, Number(opts.radius) || 5), degree: Math.max(1, Math.round(Number(opts.degree)) || 3) };
        if (e.type === "double") {
          // Finishes an open-ended curve (the double click's own point is already in, twice).
          const same = (a: Click, b: Click) => Math.hypot(a.at[0] - b.at[0], a.at[1] - b.at[1]) < 1e-9;
          const done = pts.filter((q, i) => i === 0 || !same(q, pts[i - 1]));
          const shape = buildShape(tool, done, o);
          if (shape) void addShape(shape);
          setDraft({ kind: "none" });
          return;
        }
        // Clicking the first point again closes a B-spline or a curve through points.
        if (need === null && pts.length >= 3 && ((e.point !== null && e.point === pts[0].id) || Math.hypot(at[0] - pts[0].at[0], at[1] - pts[0].at[1]) < 1e-9)) {
          const shape = buildShape(tool, pts, o, true);
          if (shape) void addShape(shape);
          setDraft({ kind: "none" });
          return;
        }
        const next = [...pts, { at, id: e.point }];
        if (need !== null && next.length >= need) {
          const shape = buildShape(tool, next, o);
          if (shape) void addShape(shape);
          setDraft({ kind: "none" });
        } else setDraft({ kind: "clicks", pts: next });
        return;
      }
      if (knotPick !== null && e.type === "down") {
        if (e.curve === knotPick && editing) void run("sketch.bspline", { target: editing, curve: knotPick, op: "insert_knot", at });
        setKnotPick(null);
        return;
      }
      if (e.type === "move") {
        const fd = filletDragging.current;
        if (fd) {
          // Dragging out from a corner sizes the round: its distance from the corner.
          const c = pos.get(fd.point);
          const d = c ? Math.hypot(at[0] - c[0], at[1] - c[1]) : 0;
          fd.size = d > Math.max(e.grid * 0.5, 1e-6) ? Math.round(d * 100) / 100 : null;
          setFilletDrag({ ...fd });
        }
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
      if (e.type === "up" && filletDragging.current) {
        const fd = filletDragging.current;
        filletDragging.current = null;
        setFilletDrag(null);
        if (fd.size !== null) setFilletText(String(fd.size));
        roundCorner({ point: fd.point }, fd.size ?? filletSize);
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
        case "fillet":
          if (e.point !== null && isCorner(g, e.point)) {
            // A corner: round it on release (at the size dragged out, else the size in the field).
            filletDragging.current = { point: e.point, size: null };
            setFilletDrag({ point: e.point, size: null });
            setFilletFirst(null);
          } else if (e.curve !== null) {
            // Two curves, one click each, on the pieces to keep.
            if (!filletFirst || filletFirst.curve === e.curve) setFilletFirst({ curve: e.curve, at });
            else {
              roundCorner({ curves: [filletFirst.curve, e.curve], picks: [filletFirst.at, at] }, filletSize);
              setFilletFirst(null);
            }
          } else setFilletFirst(null);
          break;
        case "trim":
        case "extend":
          if (e.curve !== null && editing) void run(tool === "trim" ? "sketch.trim" : "sketch.extend", { target: editing, curve: e.curve, at });
          break;
        case "split":
          if (e.curve !== null && editing)
            void run("sketch.split", { target: editing, curve: e.curve, at }).then((out) => {
              const c = (out as { output?: { curves?: number[] } } | null)?.output?.curves;
              if (c) setSelection(new Set(c));
            });
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
            firstSnap.current = e;
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
            if (firstSnap.current) geo = tie(geo, from, firstSnap.current);
          }
          const first = draft.first === -1 ? from : draft.first;
          const end = pointAt(geo, e.u, e.v, e.point);
          if (end.id === from) break;
          geo = tie(end.g, end.id, e);
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
          if (draft.kind !== "circle") {
            firstSnap.current = e;
            setDraft({ kind: "circle", center: e.point, at });
          } else {
            const r = pointAt(g, draft.at[0], draft.at[1], draft.center);
            const radius = Math.hypot(at[0] - draft.at[0], at[1] - draft.at[1]);
            const geo = draft.center === null && firstSnap.current ? tie(r.g, r.id, firstSnap.current) : r.g;
            if (radius > 1e-6) commit(addCircle(geo, r.id, radius));
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
          const other = e.point ?? e.curve;
          if (draft.kind === "dimension" && other !== null && other !== draft.from) {
            // A point and a point, line, circle or any curve: the distance between them.
            addDimension("distance", draft.from, other, measure(draft.from, other), sx, sy);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [data, tool, draft, pos, commit, commitDrag, addDimension, solved, penPointer, selection, setSelected, applyMoves, editing, run, roundCorner, filletSize, filletFirst, opts, addShape, knotPick],
  );

  // Keys: tools, Escape, Delete.
  useEffect(() => {
    if (!editing) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).closest("input, textarea, select")) return;
      const fam = FAMILIES.find((x) => x.key === e.key.toUpperCase());
      if (fam && !e.metaKey && !e.ctrlKey && !e.altKey) {
        // A family's key picks its last-used tool; pressed again, the next one in the family.
        const here = fam.tools.findIndex((x) => x.tool === tool);
        const next = here >= 0 ? fam.tools[(here + 1) % fam.tools.length].tool : (lastOf[fam.key] ?? fam.tools[0].tool);
        pick(next);
        e.preventDefault();
      } else if (e.key.toUpperCase() === "N" && !e.metaKey && !e.ctrlKey) {
        setConstructionMode((v) => !v);
        e.preventDefault();
      } else if (e.key === "Enter" && draft.kind === "clicks") {
        e.preventDefault();
        const shape = buildShape(tool, draft.pts, { sides: Number(opts.sides) || 6, radius: Number(opts.radius) || 5, degree: Number(opts.degree) || 3 });
        if (shape && clicksFor(tool) === null) void addShape(shape);
        setDraft({ kind: "none" });
      } else if (e.key === "Enter" && draft.kind === "pen") {
        e.preventDefault();
        setDraft({ kind: "none" });
      } else if (e.key === "Escape") {
        e.stopPropagation();
        if (patternOpen || projectOpen || side || familyMenu) {
          setPatternOpen(null);
          setProjectOpen(false);
          setSide(null);
          setFamilyMenu(null);
        } else if (facePick) setFacePick(false);
        else if (knotPick !== null) setKnotPick(null);
        else if (filletFirst) setFilletFirst(null);
        else if (draft.kind !== "none") setDraft({ kind: "none" });
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editing, draft, selection, data, solved, hover.grid, commit, finish, applyMoves, patternOpen, projectOpen, filletFirst, tool, lastOf, opts, addShape, side, familyMenu, knotPick, facePick]);

  // What the current tool would add, drawn dashed.
  const drawPreview = useMemo((): Segment[] => {
    const cur = hover.at;
    if (!cur) return [];
    const from = (d: { from: number | null; at: [number, number] }) => (d.from !== null && d.from >= 0 ? (pos.get(d.from) ?? d.at) : d.at);
    switch (draft.kind) {
      case "clicks":
        return previewShape(tool, draft.pts, cur, { sides: Math.max(3, Math.round(Number(opts.sides)) || 6), radius: Math.max(0.001, Number(opts.radius) || 5), degree: Math.max(1, Math.round(Number(opts.degree)) || 3) });
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
          segs.push({ kind: "poly", points: curveTrace(c, P, solved) });
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
          } else if (c.kind === "bspline" && c.poles.includes(draft.point)) {
            segs.push({ kind: "poly", points: curveTrace(c, P, solved) }, { kind: "poly", points: c.poles.map(P) });
          }
        }
        return segs;
      }
      default:
        return [];
    }
  }, [draft, hover.at, pos, data, selection, solved, tool, opts]);

  // The selection's curves and lone points, for patterns and the mirror.
  const selCurves = useMemo(() => (data ? data.geometry.curves.filter((c) => selection.has(c.id)).map((c) => c.id) : []), [data, selection]);
  const lonePoint = useMemo(() => (data ? [...selection].find((id) => data.geometry.points.some((p) => p.id === id) && !data.geometry.curves.some((c) => c.id === id)) : undefined), [data, selection]);
  const mirrorAxis = useMemo(() => {
    if (!data) return null;
    const lines = [...selection].filter((id) => data.geometry.curves.some((c) => c.id === id && c.kind === "line"));
    const guides = lines.filter((id) => data.geometry.curves.find((c) => c.id === id)?.construction);
    const axis = guides.length === 1 ? guides[0] : lines[lines.length - 1];
    return axis !== undefined && selCurves.some((id) => id !== axis) ? axis : null;
  }, [data, selection, selCurves]);
  const patternSpec = useMemo((): PatternSpec | null => {
    if (!patternOpen) return null;
    const count = Math.round(Number(pat.count));
    if (!(count >= 2 && count <= 200)) return null;
    if (patternOpen === "linear") {
      const spacing = Number(pat.spacing);
      const direction = Number(pat.direction);
      return Number.isFinite(spacing) && spacing !== 0 && Number.isFinite(direction) ? { kind: "linear", count, spacing, direction } : null;
    }
    const angle = pat.full ? 360 / count : Number(pat.angle);
    const center: [number, number] = lonePoint !== undefined ? (pos.get(lonePoint) ?? [0, 0]) : [0, 0];
    return Number.isFinite(angle) && angle !== 0 ? { kind: "circular", count, angle, center } : null;
  }, [patternOpen, pat, lonePoint, pos]);

  // Previews of the exact operations: the round at the corner, the copies, the reflection.
  const preview = useMemo((): Segment[] => {
    if (!data) return drawPreview;
    const g = data.geometry;
    const out = [...drawPreview];
    if (tool === "fillet") {
      const make = (pick: CornerPick, size: number) => (corner === "round" ? filletPreview(g, solved, pick, size) : chamferPreview(g, solved, pick, size));
      if (filletDrag) out.push(...(make({ point: filletDrag.point }, filletDrag.size ?? filletSize) ?? []));
      else if (filletFirst && hover.curve !== null && hover.curve !== filletFirst.curve && hover.at) out.push(...(make({ curves: [filletFirst.curve, hover.curve], picks: [filletFirst.at, hover.at] }, filletSize) ?? []));
      else if (hover.point !== null && isCorner(g, hover.point)) out.push(...(make({ point: hover.point }, filletSize) ?? []));
    }
    if (patternSpec && selCurves.length) out.push(...patternPreview(g, solved, selCurves, patternSpec));
    if (mirrorHover && mirrorAxis !== null) {
      const axis = g.curves.find((c) => c.id === mirrorAxis);
      if (axis?.kind === "line") out.push(...mappedSegments(g, solved, selCurves.filter((id) => id !== mirrorAxis), reflectAbout(pos.get(axis.a)!, pos.get(axis.b)!), true));
    }
    return out;
  }, [data, drawPreview, tool, corner, filletDrag, filletFirst, filletSize, hover, solved, patternSpec, selCurves, mirrorHover, mirrorAxis, pos]);

  const picture: SketchPicture | null = useMemo(
    () =>
      data && !facePick
        ? {
            data,
            solved,
            hoverPoint: hover.point,
            hoverCurve: hover.curve,
            // The fillet tool keeps its first curve lit while the second is picked.
            selected: tool === "fillet" && filletFirst ? [...selection, filletFirst.curve] : [...selection],
            preview,
            cursor: tool === "select" ? null : hover.at,
            grid: hover.grid,
            polygons,
            combs,
            constraints: lit,
            snapLabel: tool === "select" ? null : snapLabel,
            anchor: draft.kind === "line" ? (draft.from !== null && draft.from >= 0 ? (pos.get(draft.from) ?? draft.at) : draft.at) : draft.kind === "clicks" && draft.pts.length ? draft.pts[draft.pts.length - 1].at : null,
          }
        : null,
    [data, solved, hover, selection, preview, tool, filletFirst, polygons, combs, lit, snapLabel, draft, pos, facePick],
  );

  /** Picks a tool (remembering it as its family's last used). */
  const pick = useCallback((t: Tool) => {
    setTool(t);
    setDraft({ kind: "none" });
    setFilletFirst(null);
    setFamilyMenu(null);
    setLastOf((m) => ({ ...m, [familyOf(t).key]: t }));
  }, []);

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
  /** An engine result's curves become the selection. */
  const selectOut = (out: unknown) => {
    const o = (out as { output?: { curves?: number[]; curve?: number } } | null)?.output;
    const ids = o?.curves ?? (o?.curve !== undefined ? [o.curve] : null);
    if (ids?.length) setSelection(new Set(ids));
  };
  // Constraints that fit what is selected.
  const constrainBar = (() => {
    if (!data) return null;
    const g = data.geometry;
    const curveOf = (id: number) => g.curves.find((c) => c.id === id);
    const ids = [...selection];
    const lines = ids.filter((id) => curveOf(id)?.kind === "line");
    const rounds = ids.filter((id) => ["circle", "arc"].includes(curveOf(id)?.kind ?? ""));
    const conics = ids.filter((id) => ["ellipse", "ellipse_arc", "hyperbola_arc", "parabola_arc"].includes(curveOf(id)?.kind ?? ""));
    const curves = ids.filter((id) => curveOf(id));
    const points = ids.filter((id) => g.points.some((p) => p.id === id) && !curveOf(id));
    const others = ids.length - lines.length - rounds.length - points.length;
    type C = Geometry["constraints"][number];
    type Option = { glyph: IconName; label: string; make?: () => C[]; dims?: () => { c: C; label: string; value: number; unit: string; signed?: boolean }[]; run?: () => void };
    const options: Option[] = [];
    const only = (l: number, r: number, p: number) => lines.length === l && rounds.length === r && points.length === p && others === 0;
    const at = (id: number): P => pos.get(id) ?? [0, 0];
    if (lines.length >= 1 && rounds.length === 0 && points.length === 0 && others === 0) {
      options.push({ glyph: "cHorizontal", label: "Horizontal", make: () => lines.map((line) => ({ kind: "horizontal", line }) as C) });
      options.push({ glyph: "cVertical", label: "Vertical", make: () => lines.map((line) => ({ kind: "vertical", line }) as C) });
      if (lines.some((l) => g.constraints.some((c) => (c.kind === "horizontal" || c.kind === "vertical") && c.line === l)))
        options.push({ glyph: "unalign", label: "Remove axes alignment (horizontal/vertical become parallel/perpendicular)", run: () => void run("sketch.unalign", { target: editing, curves: lines }) });
    }
    if (only(2, 0, 0)) {
      const [a, b] = lines as [number, number];
      options.push({ glyph: "cParallel", label: "Parallel", make: () => [{ kind: "parallel", a, b } as C] });
      options.push({ glyph: "cPerpendicular", label: "Perpendicular", make: () => [{ kind: "perpendicular", a, b } as C] });
      options.push({ glyph: "cEqual", label: "Equal length", make: () => [{ kind: "equal", a, b } as C] });
      options.push({ glyph: "cDistance", label: "Distance between (parallel lines)", dims: () => [{ c: { kind: "distance", a, b, key: "" } as C, label: "Distance", value: measure(a, b), unit: "mm" }] });
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
      options.push({ glyph: "cDistance", label: "Distance between edges", dims: () => [{ c: { kind: "distance", a, b, key: "" } as C, label: "Distance", value: Math.max(0.001, measure(a, b)), unit: "mm" }] });
    }
    if (only(1, 1, 0)) {
      options.push({ glyph: "cTangent", label: "Tangent", make: () => [{ kind: "tangent", a: lines[0]!, b: rounds[0]! } as C] });
      options.push({ glyph: "cPerpendicular", label: "Perpendicular (through the centre)", make: () => [{ kind: "perpendicular", a: lines[0]!, b: rounds[0]! } as C] });
      options.push({ glyph: "cDistance", label: "Distance from the line", dims: () => [{ c: { kind: "distance", a: lines[0]!, b: rounds[0]!, key: "" } as C, label: "Distance", value: Math.max(0.001, measure(lines[0]!, rounds[0]!)), unit: "mm" }] });
    }
    // A line (or circle, arc) and a conic, apart: tangent.
    if (conics.length === 1 && curves.length === 2 && points.length === 0 && (lines.length === 1 || rounds.length === 1))
      options.push({ glyph: "cTangent", label: "Tangent", make: () => [{ kind: "tangent", a: (lines[0] ?? rounds[0])!, b: conics[0]! } as C] });
    // Two ellipses (or an ellipse and a circle): equal.
    if (curves.length === 2 && points.length === 0 && conics.length >= 1 && conics.length + rounds.length === 2) options.push({ glyph: "cEqual", label: "Equal", make: () => [{ kind: "equal", a: curves[0]!, b: curves[1]! } as C] });
    // Two curves sharing an end: tangent (smooth) or square there; joined into one B-spline.
    if (curves.length === 2 && points.length === 0) {
      const [a, b] = [curveOf(curves[0]!)!, curveOf(curves[1]!)!];
      const ea: number[] = curveEnds(a) ?? [];
      const shared = (curveEnds(b) ?? []).find((p) => ea.includes(p));
      if (shared !== undefined) {
        if (!(a.kind === "line" && b.kind === "line") && !(only(1, 1, 0) || only(0, 2, 0))) options.push({ glyph: "cTangent", label: "Tangent where they meet (smooth)", make: () => [{ kind: "tangent", a: a.id, b: b.id, at: shared } as C] });
        options.push({ glyph: "join", label: "Join into one B-spline", run: () => void run("sketch.join", { target: editing, curves: [a.id, b.id] }).then(selectOut) });
      }
    }
    // Two curves and a point: tangent or perpendicular via that point.
    if (curves.length === 2 && points.length === 1 && ids.length === 3) {
      const [a, b] = curves as [number, number];
      options.push({ glyph: "cTangent", label: "Tangent at the point (held on both)", make: () => [{ kind: "tangent", a, b, at: points[0]! } as C] });
      options.push({ glyph: "cPerpendicular", label: "Perpendicular at the point (held on both)", make: () => [{ kind: "perpendicular", a, b, at: points[0]! } as C] });
    }
    if (only(0, 0, 2)) {
      const [a, b] = points as [number, number];
      options.push({ glyph: "cCoincident", label: "Coincident (join the points)", make: () => [{ kind: "coincident", a, b } as C] });
      options.push({ glyph: "cHorizontal", label: "Level with each other (same height)", dims: () => [{ c: { kind: "vertical_distance", a, b, key: "" } as C, label: "Y", value: 0, unit: "mm", signed: true }] });
      options.push({ glyph: "cVertical", label: "Above each other (same x)", dims: () => [{ c: { kind: "horizontal_distance", a, b, key: "" } as C, label: "X", value: 0, unit: "mm", signed: true }] });
    }
    // Symmetric: two points about a line, or about a third point (the last picked).
    if (points.length === 2 && lines.length === 1 && ids.length === 3) options.push({ glyph: "cSymmetric", label: "Symmetric about the line", make: () => [{ kind: "symmetric", a: points[0]!, b: points[1]!, line: lines[0]! } as C] });
    if (points.length === 3 && ids.length === 3) options.push({ glyph: "cSymmetric", label: "Symmetric about the last point picked", make: () => [{ kind: "symmetric", a: points[0]!, b: points[1]!, line: points[2]! } as C] });
    if (points.length === 1 && curves.length === 1 && ids.length === 2) {
      const curve = curves[0]!;
      options.push({ glyph: "cOnCurve", label: "On the curve", make: () => [{ kind: "on_curve", point: points[0]!, curve } as C] });
      if (lines.length === 1) options.push({ glyph: "cMidpoint", label: "Midpoint of the line", make: () => [{ kind: "midpoint", point: points[0]!, line: lines[0]! } as C] });
      options.push({ glyph: "cDistance", label: "Distance from the curve", dims: () => [{ c: { kind: "distance", a: points[0]!, b: curve, key: "" } as C, label: "Distance", value: Math.max(0.001, measure(points[0]!, curve)), unit: "mm" }] });
    }
    if (points.length >= 1 && lines.length + rounds.length + others === 0) {
      options.push({
        glyph: "cFix",
        label: "Fix where it is",
        make: () => points.map((point) => ({ kind: "fixed", point, x: at(point)[0], y: at(point)[1] }) as C),
      });
      options.push({
        glyph: "cLock",
        label: "Lock (its X and Y as dimensions)",
        dims: () =>
          points.flatMap((p) => [
            { c: { kind: "horizontal_distance", a: p, b: p, key: "" } as C, label: "X", value: at(p)[0], unit: "mm", signed: true },
            { c: { kind: "vertical_distance", a: p, b: p, key: "" } as C, label: "Y", value: at(p)[1], unit: "mm", signed: true },
          ]),
      });
    }
    // Snell's law: two rays meeting at a point and the boundary there.
    if (curves.length === 3 && points.length === 0 && lines.length >= 2) {
      const ls = lines.map((id) => curveOf(id) as Extract<Curve, { kind: "line" }>);
      for (let i = 0; i < ls.length; i++) {
        for (let j = 0; j < ls.length; j++) {
          if (i === j) continue;
          const shared = [ls[i].a, ls[i].b].find((p) => p === ls[j].a || p === ls[j].b);
          const boundary = curves.find((id) => id !== ls[i].id && id !== ls[j].id);
          if (shared === undefined || boundary === undefined || options.some((o) => o.glyph === "cSnell")) continue;
          options.push({ glyph: "cSnell", label: "Snell's law (refraction n₂/n₁ at the boundary)", dims: () => [{ c: { kind: "snell", a: ls[i].id, b: ls[j].id, boundary, at: shared, key: "" } as C, label: "n2/n1", value: 1.5, unit: "", signed: true }] });
        }
      }
    }
    if (curves.length >= 1) {
      options.push({ glyph: "cBlock", label: "Block (hold as it is)", make: () => curves.map((curve) => ({ kind: "block", curve }) as C) });
      const allConstruction = curves.every((id) => curveOf(id)?.construction);
      options.push({
        glyph: "construction",
        label: allConstruction ? "Make normal geometry" : "Make construction geometry",
        run: () => void commit({ ...g, curves: g.curves.map((c) => (selection.has(c.id) ? { ...c, construction: !allConstruction } : c)) }),
      });
    }
    if (curves.length === 1 && points.length === 0) {
      const c = curveOf(curves[0]!)!;
      if (c.kind === "ellipse" || c.kind === "ellipse_arc" || c.kind === "hyperbola_arc" || c.kind === "parabola_arc" || c.kind === "bspline")
        options.push({ glyph: "cInternal", label: c.kind === "bspline" ? "Show knot points" : "Show axes and foci (internal geometry)", run: () => void run("sketch.internal", { target: editing, curve: c.id }) });
      if (c.kind !== "bspline") options.push({ glyph: "spline", label: "Convert to B-spline (NURBS)", run: () => void run("sketch.bspline", { target: editing, curve: c.id, op: "convert" }).then(selectOut) });
    }
    const touches = (c: C) => refsOf(c).some((x) => selection.has(x));
    const loose = g.constraints.filter((c) => !keyOf(c) && touches(c));
    if (!options.length && !loose.length) return null;
    const same = (a: C, b: C) => JSON.stringify(a) === JSON.stringify(b);
    const apply = (o: Option) => {
      if (o.run) return o.run();
      if (o.dims) {
        let params = [...data.params];
        const add: C[] = [];
        for (const d of o.dims()) {
          const key = newKey(params);
          params = [...params, { key, label: `${d.label} ${key.slice(1)}`, value: Math.round(d.value * 1000) / 1000, unit: d.unit, ...(d.signed ? {} : { min: 0.001 }) }];
          const c = { ...d.c, key } as C;
          add.push(c);
          if (o.dims().length === 1) pendingDim.current = { index: g.constraints.length, key };
        }
        return void commit({ ...g, constraints: [...g.constraints, ...add] }, params);
      }
      const add = o.make!().filter((c) => !g.constraints.some((x) => same(x, c)));
      if (!add.length) return props.notify(`Already ${o.label.toLowerCase()}`);
      void commit({ ...g, constraints: [...g.constraints, ...add] });
    };
    return (
      <div className="sel-group sel-constrain" aria-label="Constraints">
        {options.map((o) => (
          <button key={o.label} type="button" className="sel-btn" title={o.label} aria-label={o.label} onClick={() => apply(o)}>
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
      {selCurves.length > 0 && (
        <div className="sel-group">
          <button type="button" className={`sel-btn ${patternOpen === "linear" ? "on" : ""}`} title="Linear pattern: copies in a row" aria-label="Linear pattern" aria-pressed={patternOpen === "linear"} onClick={() => setPatternOpen((v) => (v === "linear" ? null : "linear"))}>
            <Icon name="patternLinear" />
          </button>
          <button type="button" className={`sel-btn ${patternOpen === "circular" ? "on" : ""}`} title="Circular pattern: copies round a centre (a selected point, else the origin)" aria-label="Circular pattern" aria-pressed={patternOpen === "circular"} onClick={() => setPatternOpen((v) => (v === "circular" ? null : "circular"))}>
            <Icon name="patternCircular" />
          </button>
          <button
            type="button"
            className="sel-btn"
            disabled={mirrorAxis === null}
            title={mirrorAxis === null ? "Mirror: select the curves and a line to mirror them about" : "Mirror about the selected line (a construction line if one is selected): linked copies"}
            aria-label="Mirror about a line"
            onMouseEnter={() => setMirrorHover(true)}
            onMouseLeave={() => setMirrorHover(false)}
            onClick={async () => {
              if (!editing || mirrorAxis === null) return;
              const out = (await run("sketch.mirror", { target: editing, curves: selCurves.filter((id) => id !== mirrorAxis), line: mirrorAxis })) as { output?: { curves?: number[] } } | null;
              setMirrorHover(false);
              if (out?.output?.curves) setSelection(new Set(out.output.curves));
            }}
          >
            <Icon name="mirror" />
          </button>
        </div>
      )}
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
      {patternOpen && selCurves.length > 0 && (
        <form
          className="sketch-panel"
          aria-label={patternOpen === "linear" ? "Linear pattern" : "Circular pattern"}
          onSubmit={async (e) => {
            e.preventDefault();
            if (!editing || !patternSpec) return props.notify("Give a count of 2 or more and a spacing or angle other than zero.");
            const base = { target: editing, curves: selCurves, kind: patternSpec.kind, count: patternSpec.count, linked: pat.linked };
            const params =
              patternSpec.kind === "linear"
                ? { ...base, spacing: patternSpec.spacing, direction: [Math.cos((patternSpec.direction * Math.PI) / 180), Math.sin((patternSpec.direction * Math.PI) / 180)] }
                : { ...base, angle: patternSpec.angle, center: lonePoint ?? [0, 0] };
            const out = (await run("sketch.pattern", params)) as { output?: { curves?: number[] } } | null;
            if (out?.output?.curves) {
              setPatternOpen(null);
              setSelection(new Set(out.output.curves));
            }
          }}
        >
          <strong>{patternOpen === "linear" ? "Linear pattern" : "Circular pattern"}</strong>
          <label>
            Count
            <input value={pat.count} inputMode="numeric" onChange={(e) => setPat({ ...pat, count: e.target.value })} aria-label="How many in all, the original included" />
          </label>
          {patternOpen === "linear" ? (
            <>
              <label>
                Spacing
                <input value={pat.spacing} inputMode="decimal" onChange={(e) => setPat({ ...pat, spacing: e.target.value })} aria-label="Spacing in mm" />
                <small>mm</small>
              </label>
              <label>
                Direction
                <input value={pat.direction} inputMode="decimal" onChange={(e) => setPat({ ...pat, direction: e.target.value })} aria-label="Direction in degrees from the x axis" />
                <small>°</small>
              </label>
            </>
          ) : (
            <>
              <label className="check">
                <input type="checkbox" checked={pat.full} onChange={(e) => setPat({ ...pat, full: e.target.checked })} />
                Full circle
              </label>
              <label>
                Step
                <input value={pat.full ? String(Math.round((360 / Math.max(1, Number(pat.count) || 1)) * 1000) / 1000) : pat.angle} disabled={pat.full} inputMode="decimal" onChange={(e) => setPat({ ...pat, angle: e.target.value })} aria-label="Angle between copies in degrees" />
                <small>°</small>
              </label>
              <span className="sketch-panel-note">about {lonePoint !== undefined ? "the selected point" : "the origin"}</span>
            </>
          )}
          <label className="check" title="Linked copies follow the original; the spacing or angle becomes a dimension">
            <input type="checkbox" checked={pat.linked} onChange={(e) => setPat({ ...pat, linked: e.target.checked })} />
            Linked
          </label>
          <button type="submit" className="primary">
            {patternSpec ? `Make ${patternSpec.count - 1} ${patternSpec.count === 2 ? "copy" : "copies"}` : "Make copies"}
          </button>
        </form>
      )}
    </div>
  );

  // Parts a sketch can project: every program part, and whether the view shows it.
  const parts = useMemo(
    () =>
      [...entities.values()]
        .map((e) => ({ e, prog: e.components["shape.program"] as { name?: string } | undefined }))
        .filter((x) => x.prog)
        .map(({ e, prog }) => ({ id: e.id, name: prog?.name ?? e.id, visible: !props.hidden?.has(e.id) && (!props.isolated || props.isolated === e.id) })),
    [entities, props.hidden, props.isolated],
  );

  /** Checks the sketch (and with `fix`, puts right what can be). */
  const validate = useCallback(
    async (fix: boolean) => {
      if (!editing) return;
      const out = (await run("sketch.validate", { target: editing, fix }).catch(() => null)) as { output?: Validation } | null;
      setValidation(out?.output ?? null);
      if (fix && out?.output?.fixed) {
        const f = out.output.fixed;
        props.notify(`Fixed: ${f.joined ?? 0} gaps joined, ${f.removed_curves ?? 0} curves and ${f.removed_constraints ?? 0} constraints removed`);
      }
    },
    [editing, run, props],
  );

  // One B-spline selected: its degree, knots and weights.
  const spline = data && selection.size === 1 ? data.geometry.curves.find((c): c is Extract<Curve, { kind: "bspline" }> => c.kind === "bspline" && selection.has(c.id)) : undefined;
  const splineOp = (op: string, extra: Record<string, unknown> = {}) => editing && spline && void run("sketch.bspline", { target: editing, curve: spline.id, op, ...extra }).then(selectOut);
  const splinePanel = spline && data && (
    <div className="sketch-panel spline-panel" role="group" aria-label="B-spline">
      <strong>
        B-spline · degree {spline.degree}
        {spline.periodic ? " · closed" : ""}
        {spline.weights?.some((w) => Math.abs(w - 1) > 1e-9) ? " · rational" : ""}
      </strong>
      <div className="sel-group">
        <button type="button" className="secondary" title="Raise the degree (the curve stays exactly the same)" onClick={() => splineOp("increase_degree")}>
          Degree +
        </button>
        <button type="button" className="secondary" disabled={spline.degree <= 1} title="Lower the degree (nearest curve)" onClick={() => splineOp("decrease_degree")}>
          Degree −
        </button>
        <button type="button" className={`secondary ${knotPick === spline.id ? "on" : ""}`} title="Insert a knot: click the curve where it goes" onClick={() => setKnotPick((v) => (v === spline.id ? null : spline.id))}>
          Insert knot
        </button>
      </div>
      {(() => {
        const { knots, mults } = knotList(spline);
        const inner = knots.map((k, i) => ({ k, m: mults[i], i })).filter((x) => spline.periodic || (x.i > 0 && x.i < knots.length - 1));
        return inner.length ? (
          <div className="spline-knots">
            <span className="sketch-panel-note">Knots (multiplicity)</span>
            {inner.map((x) => (
              <span key={x.i} className="spline-knot">
                {Number(x.k.toFixed(3))} ×{x.m}
                <button type="button" title="Repeat this knot once more (less smooth there)" disabled={x.m >= spline.degree} onClick={() => splineOp("increase_multiplicity", { knot: x.i })}>
                  +
                </button>
                <button type="button" title="Repeat this knot once less (smoother; removes it at 1)" onClick={() => splineOp("decrease_multiplicity", { knot: x.i })}>
                  −
                </button>
              </span>
            ))}
          </div>
        ) : null;
      })()}
      <div className="spline-weights">
        <span className="sketch-panel-note">Weights</span>
        {spline.poles.map((p, i) => (
          <label key={p} className="sel-field" title={`Pole ${i + 1}: a heavier pole pulls the curve towards it`}>
            {i + 1}
            <input
              defaultValue={String(spline.weights?.[i] ?? 1)}
              key={`${p}-${spline.weights?.[i] ?? 1}`}
              inputMode="decimal"
              aria-label={`Weight of pole ${i + 1}`}
              onBlur={(e) => {
                const w = Number(e.target.value);
                if (!(w > 0) || w === (spline.weights?.[i] ?? 1)) return;
                const weights = spline.poles.map((_, j) => (j === i ? w : (spline.weights?.[j] ?? 1)));
                void commit({ ...data.geometry, curves: data.geometry.curves.map((c) => (c.id === spline.id ? { ...spline, weights } : c)) });
              }}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
            />
          </label>
        ))}
      </div>
      <div className="spline-toggles">
        <label className="check">
          <input type="checkbox" checked={polygons} onChange={(e) => setPolygons(e.target.checked)} /> Control polygons always
        </label>
        <label className="check">
          <input type="checkbox" checked={combs} onChange={(e) => setCombs(e.target.checked)} /> Curvature comb
        </label>
      </div>
    </div>
  );

  // The constraint list, checking, and copying from another sketch.
  const otherSketches = useMemo(() => [...entities.values()].filter((e) => e.id !== editing && sketchOf(e)).map((e) => ({ id: e.id, name: sketchOf(e)!.name })), [entities, editing]);
  const sidePanel = (() => {
    if (!data || !side) return null;
    const g = data.geometry;
    const s = solved?.solved;
    if (side === "constraints") {
      const stateOf = (i: number) =>
        s?.conflicts.includes(i) ? "conflict" : s?.malformed?.includes(i) ? "malformed" : s?.redundant?.includes(i) ? "redundant" : s?.partially_redundant?.includes(i) ? "partial" : "";
      const selectAll = (which: number[]) => {
        setLit(which);
        setSelection(new Set(which.flatMap((i) => refsOf(g.constraints[i]))));
      };
      return (
        <div className="sketch-panel constraint-list" role="group" aria-label="Constraints">
          <strong>Constraints ({g.constraints.length})</strong>
          <div className="sel-group">
            <button type="button" className="secondary" disabled={!s?.conflicts.length} onClick={() => selectAll(s!.conflicts)}>
              Conflicting {s?.conflicts.length ? `(${s.conflicts.length})` : ""}
            </button>
            <button type="button" className="secondary" disabled={!s?.redundant?.length && !s?.partially_redundant?.length} onClick={() => selectAll([...(s?.redundant ?? []), ...(s?.partially_redundant ?? [])])}>
              Redundant {(s?.redundant?.length ?? 0) + (s?.partially_redundant?.length ?? 0) || ""}
            </button>
            <button
              type="button"
              className="secondary"
              disabled={!s?.free_points?.length}
              title="Select the points that can still move"
              onClick={() => {
                setLit([]);
                setTool("select");
                setSelection(new Set(s?.free_points ?? []));
              }}
            >
              Free points {s?.free_points?.length || ""}
            </button>
          </div>
          <ol className="constraint-rows">
            {g.constraints.map((c, i) => {
              const key = keyOf(c);
              const d = key ? data.params.find((p) => p.key === key) : undefined;
              const v = key ? (s?.measured?.[key] ?? d?.value) : undefined;
              const st = stateOf(i);
              const off = c.kind === "inactive";
              const isRef = (off ? c.of : c).kind === "reference";
              return (
                <li key={i} className={`${st} ${lit.includes(i) ? "lit" : ""} ${off ? "inactive" : ""}`}>
                  <button type="button" className="constraint-name" title={st ? `${st === "conflict" ? "Conflicts with others" : st === "malformed" ? "Makes no sense for what it refers to" : st === "partial" ? "Partly fixed by others already" : "Adds nothing: others fix it already"}` : "Select what it holds"} onClick={() => selectAll([i])}>
                    {describe(c)}
                    {v !== undefined && <span className="constraint-value">{Number(v.toFixed(3))}{innerOf(c).kind === "angle" ? "°" : ""}</span>}
                  </button>
                  {key && (
                    <button type="button" className="mini" title={isRef ? "Reference (measures): make it drive" : "Driving: make it a reference (measures only)"} onClick={() => void changeConstraint(i, isRef ? "driving" : "reference")}>
                      {isRef ? "ref" : "drive"}
                    </button>
                  )}
                  <button type="button" className="mini" title={off ? "Switched off: switch on" : "Switch off (kept, not solved)"} onClick={() => void changeConstraint(i, off ? "on" : "off")}>
                    {off ? "off" : "on"}
                  </button>
                  <button type="button" className="mini" title="Remove" aria-label="Remove constraint" onClick={() => void changeConstraint(i, "remove")}>
                    ✕
                  </button>
                </li>
              );
            })}
          </ol>
        </div>
      );
    }
    if (side === "validate") {
      const v = validation;
      const rows: [string, number][] = v
        ? [
            ["Gaps between ends that nearly meet", v.gaps?.length ?? 0],
            ["Curves of no size", v.degenerate?.length ?? 0],
            ["Curves drawn twice", v.duplicates?.length ?? 0],
            ["Constraints given twice", v.duplicate_constraints?.length ?? 0],
            ["Open ends", v.open_ends?.length ?? 0],
            ["Conflicting constraints", v.conflicts?.length ?? 0],
            ["Redundant constraints", v.redundant?.length ?? 0],
            ["Malformed constraints", v.malformed?.length ?? 0],
          ]
        : [];
      const fixable = (v?.gaps?.length ?? 0) + (v?.degenerate?.length ?? 0) + (v?.duplicates?.length ?? 0) + (v?.duplicate_constraints?.length ?? 0);
      return (
        <div className="sketch-panel" role="group" aria-label="Check the sketch">
          <strong>Check the sketch</strong>
          {!v && <span className="sketch-panel-note">Checking…</span>}
          {v && (
            <ul className="validate-rows">
              {rows.map(([label, n]) => (
                <li key={label} className={n ? "found" : ""}>
                  {label}: <b>{n}</b>
                </li>
              ))}
            </ul>
          )}
          {v?.open_ends?.length ? (
            <button type="button" className="secondary" onClick={() => setSelection(new Set(v.open_ends))}>
              Select open ends
            </button>
          ) : null}
          <button type="button" className="primary" disabled={!fixable} title="Join near-meeting ends, remove curves of no size, duplicate curves and repeated constraints" onClick={() => void validate(true)}>
            Fix
          </button>
        </div>
      );
    }
    return (
      <form
        className="sketch-panel"
        aria-label="Copy from another sketch"
        onSubmit={async (e) => {
          e.preventDefault();
          if (!editing || !copyFrom.from) return;
          const out = await run("sketch.carbon_copy", { target: editing, from: copyFrom.from, linked: copyFrom.linked }).catch(() => null);
          if (out) {
            selectOut(out);
            setSide(null);
          }
        }}
      >
        <strong>Copy from another sketch</strong>
        {otherSketches.length === 0 && <span className="sketch-panel-note">No other sketches yet</span>}
        <select value={copyFrom.from} aria-label="Sketch to copy from" onChange={(e) => setCopyFrom({ ...copyFrom, from: e.target.value })}>
          <option value="">Pick a sketch…</option>
          {otherSketches.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name}
            </option>
          ))}
        </select>
        <label className="check" title="Linked: the copy's dimensions follow the original's (formulas); unlinked: a plain merge">
          <input type="checkbox" checked={copyFrom.linked} onChange={(e) => setCopyFrom({ ...copyFrom, linked: e.target.checked })} />
          Linked (carbon copy)
        </label>
        <button type="submit" className="primary" disabled={!copyFrom.from}>
          Copy in
        </button>
      </form>
    );
  })();

  const st = status(entity);
  const bar = data ? (
    <div className="sketch-bar floating" role="toolbar" aria-label="Sketch tools">
      <div className="sketch-head">
        <strong>{data.name}</strong>
        <span className={`sketch-status ${st.tone}`}>{st.text}</span>
      </div>
      <div className="sketch-tools">
        {FAMILIES.map((f) => {
          const here = f.tools.find((x) => x.tool === tool);
          const shown = here ?? f.tools.find((x) => x.tool === lastOf[f.key]) ?? f.tools[0];
          return (
            <span key={f.key} className={`tool-family ${here ? "active" : ""}`}>
              <button type="button" className={here ? "active" : ""} title={`${shown.label} (${f.key}${f.tools.length > 1 ? `, again for ${f.tools.filter((x) => x !== shown).map((x) => x.label.toLowerCase()).join(", ")}` : ""}) · ${shown.hint}`} onClick={() => pick(shown.tool)}>
                {shown.label}
                <kbd>{f.key}</kbd>
              </button>
              {f.tools.length > 1 && (
                <button type="button" className="family-more" aria-label={`More ${shown.label.toLowerCase()} tools`} aria-expanded={familyMenu === f.key} onClick={() => setFamilyMenu((m) => (m === f.key ? null : f.key))}>
                  ▾
                </button>
              )}
              {familyMenu === f.key && (
                <span className="family-menu" role="menu">
                  {f.tools.map((x) => (
                    <button key={x.tool} type="button" role="menuitem" className={x.tool === tool ? "on" : ""} title={x.hint} onClick={() => pick(x.tool)}>
                      {x.label}
                    </button>
                  ))}
                </span>
              )}
            </span>
          );
        })}
        <button type="button" className={`construction-mode ${constructionMode ? "active" : ""}`} aria-pressed={constructionMode} title="Construction mode (N): new geometry is construction (guides that make no outline)" onClick={() => setConstructionMode((v) => !v)}>
          <Icon name="construction" />
          <kbd>N</kbd>
        </button>
      </div>
      <p className="sketch-hint">
        {knotPick !== null ? "Click the B-spline where the new knot goes · Esc cancels" : toolInfo(tool).hint} · snaps to points, midpoints, crossings and curves · Alt turns snapping off{constructionMode ? " · drawing construction" : ""}
      </p>
      {(tool === "polygon" || tool === "rrect" || tool === "bspline") && (
        <div className="sketch-options" role="group" aria-label="Tool options">
          {tool === "polygon" && (
            <label className="sel-field">
              Sides
              <input value={opts.sides} inputMode="numeric" aria-label="Number of sides" onChange={(e) => setOpts({ ...opts, sides: e.target.value })} />
            </label>
          )}
          {tool === "rrect" && (
            <label className="sel-field">
              R
              <input value={opts.radius} inputMode="decimal" aria-label="Corner radius in mm" onChange={(e) => setOpts({ ...opts, radius: e.target.value })} />
              <small>mm</small>
            </label>
          )}
          {tool === "bspline" && (
            <label className="sel-field">
              Degree
              <input value={opts.degree} inputMode="numeric" aria-label="B-spline degree" onChange={(e) => setOpts({ ...opts, degree: e.target.value })} />
            </label>
          )}
        </div>
      )}
      {tool === "fillet" && (
        <div className="sketch-options" role="group" aria-label="Fillet options">
          <div className="seg-toggle">
            <button type="button" className={corner === "round" ? "on" : ""} aria-pressed={corner === "round"} onClick={() => setCorner("round")}>
              <Icon name="fillet" /> Round
            </button>
            <button type="button" className={corner === "chamfer" ? "on" : ""} aria-pressed={corner === "chamfer"} onClick={() => setCorner("chamfer")}>
              <span aria-hidden="true">⌐</span> Chamfer
            </button>
          </div>
          <label className="sel-field">
            {corner === "round" ? "R" : "D"}
            <input value={filletDrag?.size != null ? String(filletDrag.size) : filletText} inputMode="decimal" aria-label={corner === "round" ? "Fillet radius in mm" : "Chamfer distance in mm"} onChange={(e) => setFilletText(e.target.value)} />
            <small>mm</small>
          </label>
          {filletFirst && <span className="sketch-panel-note">Now click the second curve</span>}
        </div>
      )}
      {selectionBar}
      {projectOpen && (
        <div className="sketch-panel" role="group" aria-label="Project parts onto the sketch">
          <strong>Project onto the sketch</strong>
          <div className="seg-toggle">
            <button type="button" className={proj.mode === "outline" ? "on" : ""} aria-pressed={proj.mode === "outline"} onClick={() => setProj({ ...proj, mode: "outline" })} title="The outline the part shows looking straight at the sketch">
              Outline
            </button>
            <button type="button" className={proj.mode === "section" ? "on" : ""} aria-pressed={proj.mode === "section"} onClick={() => setProj({ ...proj, mode: "section" })} title="Where the sketch plane cuts the part">
              Section
            </button>
          </div>
          <div className="sketch-parts">
            {parts.length === 0 && <span className="sketch-panel-note">No parts to project yet</span>}
            {parts.map((p) => (
              <label key={p.id} className="check">
                <input
                  type="checkbox"
                  checked={proj.picked.has(p.id)}
                  onChange={(e) => {
                    const next = new Set(proj.picked);
                    if (e.target.checked) next.add(p.id);
                    else next.delete(p.id);
                    setProj({ ...proj, picked: next });
                  }}
                />
                {p.name}
                {!p.visible && <small> (hidden)</small>}
              </label>
            ))}
          </div>
          {parts.length > 1 && (
            <button type="button" className="secondary" onClick={() => setProj({ ...proj, picked: new Set(parts.filter((p) => p.visible).map((p) => p.id)) })}>
              All visible parts
            </button>
          )}
          {proj.mode === "outline" && (
            <label className="check" title="Also the edges behind the part">
              <input type="checkbox" checked={proj.hidden} onChange={(e) => setProj({ ...proj, hidden: e.target.checked })} />
              Hidden edges
            </label>
          )}
          <label className="check" title="Pinned where the part put them, to draw against">
            <input type="checkbox" checked={proj.fixed} onChange={(e) => setProj({ ...proj, fixed: e.target.checked })} />
            Reference (pinned)
          </label>
          <label className="check" title="Guides that never make an outline">
            <input type="checkbox" checked={proj.construction} onChange={(e) => setProj({ ...proj, construction: e.target.checked })} />
            Construction
          </label>
          <button
            type="button"
            className="primary"
            disabled={!proj.picked.size}
            onClick={async () => {
              if (!editing) return;
              const from = parts.filter((p) => proj.picked.has(p.id)).map((p) => p.id);
              const out = (await run("sketch.project", { target: editing, from, mode: proj.mode, hidden: proj.hidden, fixed: proj.fixed, construction: proj.construction })) as { output?: { curves?: number[] } } | null;
              if (out?.output?.curves) {
                setProjectOpen(false);
                setTool("select");
                setSelection(new Set(out.output.curves));
                props.notify(`Projected ${out.output.curves.length} ${out.output.curves.length === 1 ? "curve" : "curves"}`);
              }
            }}
          >
            Project
          </button>
        </div>
      )}
      <div className="sketch-actions">
        <button type="button" className="primary" onClick={() => make("extrude")} disabled={!solved?.loops.length}>
          <Icon name="cube" /> Extrude
        </button>
        <button type="button" className="secondary" onClick={() => make("revolve")} disabled={!solved?.loops.length}>
          <Icon name="rotate" /> Revolve
        </button>
        <button type="button" className={`secondary ${projectOpen ? "on" : ""}`} aria-pressed={projectOpen} title="Project parts' edges onto the sketch plane" onClick={() => setProjectOpen((v) => !v)}>
          <Icon name="project" /> Project
        </button>
        <button type="button" className={`secondary ${side === "constraints" ? "on" : ""}`} aria-pressed={side === "constraints"} title="Every constraint: what conflicts or repeats, driving or reference, on or off" onClick={() => setSide((v) => (v === "constraints" ? null : "constraints"))}>
          <Icon name="list" /> Constraints
        </button>
        <button
          type="button"
          className={`secondary ${side === "validate" ? "on" : ""}`}
          aria-pressed={side === "validate"}
          title="Check the sketch: gaps between ends, curves of no size, duplicates, repeated constraints"
          onClick={() => {
            setSide((v) => (v === "validate" ? null : "validate"));
            void validate(false);
          }}
        >
          <Icon name="check" /> Check
        </button>
        <button type="button" className={`secondary ${side === "copy" ? "on" : ""}`} aria-pressed={side === "copy"} title="Copy another sketch's geometry in (carbon copy, linked to its dimensions, or merge)" onClick={() => setSide((v) => (v === "copy" ? null : "copy"))}>
          <Icon name="copy" /> Copy from
        </button>
        {props.requestFace && (
          <button type="button" className="secondary" title="Put this sketch on a flat face of a part (click the face next)" onClick={moveToFace}>
            <Icon name="cube" /> On a face
          </button>
        )}
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

  // Panels beside the drawing (the B-spline's knots and weights, the constraint list, checks, copying).
  const sidebar = data && (splinePanel || sidePanel) ? (
    <div className="sketch-side floating" aria-label="Sketch panels">
      {splinePanel}
      {sidePanel}
    </div>
  ) : null;

  return { editing, picture, onPointer, bar, sidebar, dimEditor, openDim, start, edit, finish };
}

/** Choose a plane for a new sketch: a named one (moved along its normal by an offset), or a face of a part. */
export function PlaneMenu(props: { onPick: (p: keyof typeof PLANES, offset: number) => void; onFace?: () => void; onClose: () => void }) {
  const [offset, setOffset] = useState("0");
  const off = Number(offset) || 0;
  return (
    <div className="plane-menu floating" role="menu" aria-label="Sketch on which plane">
      <strong>New sketch on…</strong>
      {(Object.keys(PLANES) as (keyof typeof PLANES)[]).map((k) => (
        <button key={k} type="button" role="menuitem" onClick={() => props.onPick(k, off)}>
          <span className={`plane-chip plane-${k.toLowerCase()}`}>{k}</span>
          {PLANES[k].label}
        </button>
      ))}
      <label className="plane-offset" title="Moves the plane along its normal">
        Offset
        <input value={offset} inputMode="decimal" aria-label="Plane offset in mm" onChange={(e) => setOffset(e.target.value)} />
        <small>mm</small>
      </label>
      {props.onFace && (
        <button type="button" role="menuitem" onClick={props.onFace} title="Select a part, then click one of its flat faces">
          <span className="plane-chip plane-face">▱</span>
          On a face of a part…
        </button>
      )}
      <button type="button" className="plane-cancel" onClick={props.onClose}>
        Cancel
      </button>
    </div>
  );
}
