import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { AiPanel } from "./AiPanel";
import { type Proposal, interpret } from "./assistant";
import { Collapsed, ProjectPill, Restore, ToolRail, ViewBar } from "./Chrome";
import { StorageDialog } from "./Dialog";
import { type Rule, displayedWorld, evaluateRule, overlaysFor, requirement, suggestRule, targetsOf } from "./intent";
import { RequirementsPanel } from "./RequirementsPanel";
import { type CheckReport, type Engine, type EngineError, type EntityView, type Fix, type MeshData, type ShapeStats, type TeamCursor, mm, projectInfo, projectName, projectTitle, projectUrl } from "./engine";
import { type EdgeFinish, type Frac, type LiveEdit, type Shape, assemblyParts, box, groupOf, instance, isPart, material, originOffset, partName, pivotOf, placement, program, shapeOf, toMm } from "./model";
import { GroupList, HistoryBody, ObjectsBody, ProjectBody, PropertiesBody, ToolPanel, ViewsBody } from "./Panels";
import { useEdgeEditor } from "./EdgeEditor";
import type { MotionGrab, ProgramFace, StressView, TurnBody, TurnOptions } from "./Viewport";
import { useExplode } from "./Explode";
import type { FaceDriver } from "./engine";
import { SyncBanner, TeamDialog, TeamPill, useTeam } from "./Team";
import { PlaneMenu, useSketchEditor } from "./SketchEditor";
import { Icon } from "./icons";
import { sketchOf } from "./sketchModel";
import { ComponentTree, InstanceBody, ParameterSets, StackingPlan, parameterSet, stackingSubject } from "./Components";
import { type AiMode, type Group, type PanelKind, type UiPrefs, loadPrefs, savePrefs } from "./ui";
import { ProjectsHome } from "./ProjectsHome";
import { type CommentPin, type CommentsView, type MarkupTool, type PinAnchor, type Stroke, CommentsPanel, DraftCard, ThreadCard, colorFor, initialsOf, pinsFor, placeName, useThreads } from "./Comments";
import { type ViewName, Viewport, type ViewportHandle } from "./Viewport";

const DIMS = ["width", "depth", "height"] as const;
const round2 = (v: number) => Math.round(v * 100) / 100;

/** Applies a field change, scaling the other dimensions when proportions are locked. */
function applyPatch(base: Shape, current: Shape, patch: Partial<Shape>, lock: boolean): Shape {
  const next = { ...current, ...patch };
  const axis = DIMS.find((a) => patch[a] !== undefined);
  if (lock && axis && base[axis] > 0) {
    const ratio = patch[axis]! / base[axis];
    for (const a of DIMS) if (a !== axis) next[a] = round2(base[a] * ratio);
  }
  return next;
}

/** Position fields edit the pivot's location; the engine stores the box's minimum corner. */
function toPlacement(patch: Partial<Shape>, part: import("./engine").EntityView | undefined): Partial<Shape> {
  if (!part) return patch;
  const f: Frac = pivotOf(part);
  const s = shapeOf(part);
  const size = [s.width, s.depth, s.height];
  const out = { ...patch };
  (["x", "y", "z"] as const).forEach((k, i) => {
    if (patch[k] !== undefined) out[k] = patch[k]! - f[i] * size[i];
  });
  return out;
}

const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && !!t.closest("input, textarea, select, [contenteditable='true']");

const fmtNum = (v: number) => Number(v.toFixed(2)).toLocaleString("en-US");


/** Teammates' colours (cursor, name tag, selection outline): distinct from the app's own violet. */
const MATE_COLORS = ["#e2553f", "#1f9d74", "#2f7de1", "#d4880f", "#c23d8f", "#0f9fb3"];

export function App({ engine }: { engine: Engine }) {
  const state = useSyncExternalStore(engine.subscribe, engine.getState);
  const [prefs, setPrefs] = useState<UiPrefs>(loadPrefs);
  const [selected, setSelected] = useState<string | null>(null);
  // More items picked with Shift: they move and delete together with the selected one.
  const [also, setAlso] = useState<string[]>([]);
  const alsoRef = useRef<string[]>([]);
  alsoRef.current = also;
  // Picked part inside the selected assembly (by name); cleared with the selection.
  const [selectedPartName, setSelectedPartName] = useState<string | null>(null);
  // What the view shows: hidden items, an isolated one, and paint by material or sale status.
  const [hiddenItems, setHiddenItems] = useState<ReadonlySet<string>>(() => new Set());
  const [isolated, setIsolated] = useState<string | null>(null);
  const [colorBy, setColorBy] = useState<"material" | "status">("material");
  const [objectsTab, setObjectsTab] = useState<"tree" | "stacking">("tree");
  useEffect(() => setSelectedPartName(null), [selected]);
  const [panel, setPanel] = useState<PanelKind | null>(null);
  const [dialog, setDialog] = useState<"share" | "storage" | null>(null);
  const [focus, setFocus] = useState(false);
  const [view, setView] = useState<ViewName>("perspective");
  const [wireframe, setWireframe] = useState(false);
  const [turntable, setTurntable] = useState(false);
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [live, setLiveState] = useState<LiveEdit | null>(null);
  const [lockRatio, setLockRatio] = useState(false);
  const [report, setReport] = useState<CheckReport | null>(null);
  const [checking, setChecking] = useState(false);
  const [focusReq, setFocusReq] = useState<string | null>(null);
  const [pivotMode, setPivotMode] = useState(false);
  const [pivotLive, setPivotLive] = useState<Frac | null>(null);
  // Live parameter preview of an AI-made shape, evaluated by the engine without committing.
  const [shapePreview, setShapePreview] = useState<{ id: string; stats: ShapeStats; mesh: MeshData } | null>(null);
  const previewNext = useRef<{ id: string; values: Record<string, number>; finishes?: EdgeFinish[] } | null>(null);
  const previewBusy = useRef(false);
  const previewSeq = useRef(0);
  const liveRef = useRef<LiveEdit | null>(null);
  const nudgeTimer = useRef<number>(undefined);
  const setLive = useCallback((next: LiveEdit | null) => {
    liveRef.current = next;
    setLiveState(next);
  }, []);
  const viewport = useRef<ViewportHandle>(null);
  const toastTimer = useRef<number>(undefined);

  const update = useCallback((patch: Partial<UiPrefs>) => setPrefs((p) => ({ ...p, ...patch })), []);
  useEffect(() => savePrefs(prefs), [prefs]);

  const parts = useMemo(() => [...state.entities.values()].filter(isPart), [state.entities]);
  const materials = useMemo(() => [...state.entities.values()].filter((e) => material(e)), [state.entities]);
  const selectedPart = selected ? state.entities.get(selected) : undefined;
  /** What the viewport shows: engine state, with a previewed shape swapped in. */
  const viewEntities = useMemo(() => {
    if (!shapePreview) return state.entities;
    const e = state.entities.get(shapePreview.id);
    if (!e) return state.entities;
    const next = new Map(state.entities);
    next.set(shapePreview.id, { ...e, mesh: shapePreview.mesh, shape: shapePreview.stats } as EntityView);
    return next;
  }, [state.entities, shapePreview]);

  const requirements = useMemo(() => [...state.entities.values()].filter((e) => requirement(e)), [state.entities]);
  const world = useMemo(() => displayedWorld(state.entities, live), [state.entities, live]);
  const overlays = useMemo(() => overlaysFor(requirements, world, focusReq), [requirements, world, focusReq]);
  // While a part is being previewed, requirements on it are re-evaluated locally.
  const previews = useMemo(() => {
    const map = new Map<string, NonNullable<ReturnType<typeof evaluateRule>>>();
    if (!live) return map;
    for (const e of requirements) {
      const rule = requirement(e)!.rule;
      if (!targetsOf(rule).includes(live.id)) continue;
      const r = evaluateRule(rule, world);
      if (r) map.set(e.id, r);
    }
    return map;
  }, [live, requirements, world]);
  const health = {
    total: requirements.length,
    passing: requirements.filter((e) => (previews.get(e.id) ?? e.result)?.passed).length,
  };

  // Forma AI can show things in this person's view.
  useEffect(
    () =>
      engine.onView(({ action, target, value }) => {
        const later = (f: () => void) => requestAnimationFrame(() => requestAnimationFrame(f));
        switch (action) {
          case "select":
            if (!target) return;
            setSelected(target);
            setPanel("properties");
            later(() => viewport.current?.frame(target));
            break;
          case "frame":
            if (target) viewport.current?.frame(target);
            break;
          case "isolate":
            if (!target) return;
            setIsolated(target);
            setSelected(target);
            break;
          case "show_all":
            setIsolated(null);
            setHiddenItems(new Set());
            later(() => viewport.current?.fit());
            break;
          case "hide":
          case "show":
            if (!target) return;
            setHiddenItems((h) => {
              const n = new Set(h);
              if (action === "hide") n.add(target);
              else n.delete(target);
              return n;
            });
            break;
          case "color":
            setColorBy(value === "status" ? "status" : "material");
            break;
          case "explode":
            // An assembly pulled apart (value: the spread, or "off" to put it back).
            if (!target) return;
            if (value === "off") setExplode((e) => (e?.id === target ? { ...e, open: false } : e));
            else {
              setSelected(target);
              setPanel("properties");
              setExplode({ id: target, spread: Math.min(2.5, Math.max(0.25, Number(value) || 1)), open: true });
            }
            break;
          case "camera":
            if (value === "top" || value === "front" || value === "bottom" || value === "perspective") {
              setView(value);
              viewport.current?.view(value);
              later(() => viewport.current?.fit());
            }
            break;
        }
      }),
    [engine],
  );

  const pendingSelect = useRef<{ id: string; focus: boolean } | null>(null);
  /** Selects something just created: now if it has arrived, else as soon as it does. */
  const selectWhenReady = useCallback(
    (id: string, focus = false) => {
      if (engine.getState().entities.has(id)) {
        setSelected(id);
        requestAnimationFrame(() => (focus ? viewport.current?.frame(id, true) : viewport.current?.fit()));
      } else pendingSelect.current = { id, focus };
    },
    [engine],
  );
  useEffect(() => {
    const wanted = pendingSelect.current;
    if (wanted && state.entities.has(wanted.id)) {
      pendingSelect.current = null;
      setSelected(wanted.id);
      requestAnimationFrame(() => (wanted.focus ? viewport.current?.frame(wanted.id, true) : viewport.current?.fit()));
      return;
    }
    if (selected && !state.entities.has(selected)) setSelected(null);
  }, [selected, state.entities]);

  useEffect(() => setPivotMode(false), [selected]);
  useEffect(() => setPivotLive(null), [selected, pivotMode]);
  // Pivot mode brings up the precise fields unless another panel is in use.
  useEffect(() => {
    if (pivotMode) setPanel((p) => p ?? "properties");
  }, [pivotMode]);

  /** Pivot fractions with one axis set to a world coordinate (mm). */
  const pivotWith = useCallback(
    (axis: 0 | 1 | 2, value: number): Frac | null => {
      if (!selectedPart) return null;
      const s = shapeOf(selectedPart);
      const min = [s.x, s.y, s.z][axis];
      const size = [s.width, s.depth, s.height][axis];
      const f = [...pivotOf(selectedPart)] as Frac;
      f[axis] = size > 0 ? (value - min) / size : 0.5;
      return f;
    },
    [selectedPart],
  );

  // Docking changes the canvas width; the renderer keeps its state.
  useEffect(() => {
    requestAnimationFrame(() => viewport.current?.resize());
  }, [prefs.aiMode, focus]);

  const notify = useCallback((text: string) => {
    setToast(text);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 3200);
  }, []);

  const run = useCallback(
    async (operation: string, parameters: Record<string, unknown>) => {
      try {
        return await engine.commit(operation, parameters);
      } catch (e) {
        notify((e as EngineError).message);
        return null;
      }
    },
    [engine, notify],
  );

  // A new project opened from the projects page brings its title (?title=…): kept in the project once it is up.
  useEffect(() => {
    const url = new URL(location.href);
    const title = url.searchParams.get("title");
    if (!title || state.connection !== "online") return;
    url.searchParams.delete("title");
    history.replaceState(null, "", url.toString());
    if (!projectInfo()?.title) void run("project.set_info", { title });
  }, [state.connection, run]);

  // The projects page: opened with ?home, or from the project menu.
  const [home, setHome] = useState(() => new URLSearchParams(location.search).has("home"));
  // A picture of the project for that page, a few seconds after each change.
  useEffect(() => {
    if (state.connection !== "online" || !state.entities.size) return;
    const timer = window.setTimeout(async () => {
      const shot = viewport.current?.snapshot();
      if (!shot) return;
      const img = new Image();
      img.src = shot;
      await img.decode().catch(() => {});
      const c = document.createElement("canvas");
      // A 16:10 picture, the view's middle, 640 px wide.
      const w = 640;
      const h = 400;
      c.width = w;
      c.height = h;
      const k = Math.max(w / img.width, h / img.height);
      const ctx = c.getContext("2d");
      if (!ctx) return;
      ctx.fillStyle = "#f4f3f7";
      ctx.fillRect(0, 0, w, h);
      ctx.drawImage(img, (w - img.width * k) / 2, (h - img.height * k) / 2, img.width * k, img.height * k);
      // Not drawn yet (all background): keep the last good picture.
      const px = ctx.getImageData(0, 0, w, h).data;
      const [r0, g0, b0] = [px[0], px[1], px[2]];
      let differ = 0;
      for (let i = 0; i < px.length; i += 4 * 7) if (Math.abs(px[i] - r0) + Math.abs(px[i + 1] - g0) + Math.abs(px[i + 2] - b0) > 40) differ++;
      if (differ < (px.length / (4 * 7)) * 0.01) return;
      const blob = await new Promise<Blob | null>((r) => c.toBlob(r, "image/png"));
      if (!blob) return;
      const list = (await fetch("/projects").then((r) => r.json()).catch(() => [])) as { name: string; default: boolean }[];
      const name = projectName || list.find((p) => p.default)?.name;
      if (name) fetch(`/projects/${name}/thumbnail`, { method: "POST", body: blob }).catch(() => {});
    }, 4000);
    return () => window.clearTimeout(timer);
  }, [state.revision, state.connection, state.entities.size]);
  // Showing a mechanism turned (stable, so the panel's effects run only when it changes),
  // with an exploded view laid over it.
  const showTurned = useCallback((id: string, bodies: TurnBody[] | null, options?: TurnOptions) => viewport.current?.turn(id, bodies, options), []);
  const entitiesNow = useRef(state.entities);
  entitiesNow.current = state.entities;
  const partsOf = useCallback((id: string) => assemblyParts(entitiesNow.current.get(id)), []);
  const { explode, setExplode, turn, apply: reshow } = useExplode(showTurned, partsOf, (id) => viewport.current?.frame(id, true));
  // A rebuilt shape is drawn afresh: lay its explode over it again.
  const explodedMesh = explode ? state.entities.get(explode.id)?.mesh : undefined;
  useEffect(() => {
    if (explode) reshow(explode.id);
    // Only when the exploded shape's mesh changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [explodedMesh, reshow]);
  const undo = useCallback(() => engine.undo().catch((e: EngineError) => notify(e.message)), [engine, notify]);
  const redo = useCallback(() => engine.redo().catch((e: EngineError) => notify(e.message)), [engine, notify]);

  const baseShape = useCallback((id: string) => {
    const e = state.entities.get(id);
    return e ? shapeOf(e) : null;
  }, [state.entities]);

  /** Live preview of a change; nothing is saved until commitLive. */
  const preview = useCallback(
    (id: string, patch: Partial<Shape> | null) => {
      const base = baseShape(id);
      if (!base || patch === null) {
        setLive(null);
        return;
      }
      const current = liveRef.current?.id === id ? liveRef.current : { ...base, id };
      setLive({ ...applyPatch(base, current, patch, lockRatio), id });
    },
    [baseShape, lockRatio, setLive],
  );

  /** The command that moves one item by `d` (mm): an instance, an AI-made part or a box part. */
  const moveStep = useCallback(
    (id: string, d: [number, number, number]) => {
      const e = state.entities.get(id);
      const base = baseShape(id);
      if (!e || !base) return null;
      const inst = instance(e);
      if (inst) return { operation: "assembly.instance.update", parameters: { target: id, at: inst.at.map((v: number, k: number) => Math.round((v + d[k]) * 100) / 100) } };
      if (program(e)) {
        const off = originOffset(e);
        return { operation: "solid.part.move", parameters: { target: id, placement: { x: mm(base.x + d[0] - off[0]), y: mm(base.y + d[1] - off[1]), z: mm(base.z + d[2] - off[2]) } } };
      }
      return { operation: "solid.box.edit", parameters: { target: id, placement: { x: mm(base.x + d[0]), y: mm(base.y + d[1]), z: mm(base.z + d[2]) } } };
    },
    [state.entities, baseShape],
  );

  /** Commits a live edit as one change per kind (size, position). */
  const commitLive = useCallback(
    async (edit: LiveEdit | null = liveRef.current) => {
      window.clearTimeout(nudgeTimer.current);
      if (!edit) return;
      const base = baseShape(edit.id);
      if (!base) return setLive(null);
      const near = (a: number, b: number) => Math.abs(a - b) < 1e-6;
      // A group moved together: every item by the same distance, one change, one undo step.
      if (alsoRef.current.length && DIMS.every((a) => near(edit[a], base[a]))) {
        const d: [number, number, number] = [edit.x - base.x, edit.y - base.y, edit.z - base.z];
        if (d.some((v) => Math.abs(v) > 1e-6)) {
          const steps = [edit.id, ...alsoRef.current].map((id) => moveStep(id, d)).filter((x) => x !== null);
          await run("core.batch", { steps });
        }
        if (liveRef.current?.id === edit.id) setLive(null);
        return;
      }
      const entity = state.entities.get(edit.id);
      const inst = entity ? instance(entity) : undefined;
      if (inst) {
        // Instances move by the dragged distance; their contents follow.
        const d = [edit.x - base.x, edit.y - base.y, edit.z - base.z];
        if (d.some((v) => Math.abs(v) > 1e-6)) {
          await run("assembly.instance.update", { target: edit.id, at: inst.at.map((v: number, k: number) => Math.round((v + d[k]) * 100) / 100) });
        }
        if (liveRef.current?.id === edit.id) setLive(null);
        return;
      }
      if (entity && program(entity)) {
        // AI-made shapes move as a whole; their shape changes through parameters.
        if (!near(edit.x, base.x) || !near(edit.y, base.y) || !near(edit.z, base.z)) {
          const off = originOffset(entity);
          await run("solid.part.move", { target: edit.id, placement: { x: mm(edit.x - off[0]), y: mm(edit.y - off[1]), z: mm(edit.z - off[2]) } });
        }
        if (liveRef.current?.id === edit.id) setLive(null);
        return;
      }
      const size = Object.fromEntries(DIMS.filter((a) => !near(edit[a], base[a])).map((a) => [a, mm(edit[a])]));
      const moved = !near(edit.x, base.x) || !near(edit.y, base.y) || !near(edit.z, base.z);
      // One gesture, one atomic change, one undo step.
      if (Object.keys(size).length || moved) {
        await run("solid.box.edit", {
          target: edit.id,
          ...size,
          ...(moved ? { placement: { x: mm(edit.x), y: mm(edit.y), z: mm(edit.z) } } : {}),
        });
      }
      // The engine's broadcast arrives before the reply, so the meshes are already current.
      if (liveRef.current === edit || liveRef.current?.id === edit.id) setLive(null);
    },
    [baseShape, run, setLive, state.entities, moveStep],
  );

  const commitPatch = useCallback(
    (id: string, patch: Partial<Shape>) => {
      const base = baseShape(id);
      if (!base) return;
      const current = liveRef.current?.id === id ? liveRef.current : { ...base, id };
      commitLive({ ...applyPatch(base, current, patch, lockRatio), id });
    },
    [baseShape, commitLive, lockRatio],
  );

  const nudge = useCallback(
    (id: string, delta: Partial<Shape>) => {
      const base = baseShape(id);
      if (!base) return;
      const current = liveRef.current?.id === id ? liveRef.current : { ...base, id };
      setLive({ ...current, x: current.x + (delta.x ?? 0), y: current.y + (delta.y ?? 0), z: current.z + (delta.z ?? 0) });
      window.clearTimeout(nudgeTimer.current);
      nudgeTimer.current = window.setTimeout(() => commitLive(), 500);
    },
    [baseShape, commitLive, setLive],
  );

  useEffect(() => {
    if (liveRef.current && liveRef.current.id !== selected) commitLive();
  }, [selected, commitLive]);

  const runCheck = useCallback(async () => {
    setChecking(true);
    try {
      setReport(await engine.check());
    } catch {
      // Offline: the last report stays, and the connection badge says why.
    } finally {
      setChecking(false);
    }
  }, [engine]);

  // The check gate re-runs after every committed change.
  useEffect(() => {
    if (state.connection !== "online") return;
    const t = window.setTimeout(runCheck, 120);
    return () => window.clearTimeout(t);
  }, [state.revision, state.connection, runCheck]);

  const applyFix = useCallback(async (fix: Fix) => {
    const done = await run(fix.operation, fix.parameters);
    if (done) notify(`Fixed · ${fix.label}`);
  }, [run, notify]);

  const createRequirement = useCallback(
    async (kind: Rule["kind"]) => {
      const rule = suggestRule(kind, [...state.entities.values()].filter((e) => box(e)), selected, world);
      if (typeof rule === "string") return notify(rule);
      const created = (await run("intent.requirement.create", { rule })) as { output: { id: string } } | null;
      if (created) setFocusReq(created.output.id);
    },
    [state.entities, selected, world, run, notify],
  );

  const pumpPreview = useCallback(async () => {
    if (previewBusy.current || !previewNext.current) return;
    const job = previewNext.current;
    previewNext.current = null;
    previewBusy.current = true;
    const seq = ++previewSeq.current;
    try {
      const r = await engine.shapePreview(job.id, job.values, job.finishes);
      if (seq === previewSeq.current) setShapePreview({ id: job.id, stats: r.stats, mesh: r.mesh });
    } catch (e) {
      if (seq === previewSeq.current) notify((e as EngineError).message);
    } finally {
      previewBusy.current = false;
      void pumpPreview();
    }
  }, [engine, notify]);

  const previewParam = useCallback(
    (key: string, value: number | null) => {
      if (!selected) return;
      if (value === null) {
        previewNext.current = null;
        previewSeq.current++;
        setShapePreview(null);
        return;
      }
      // Latest value wins; at most one evaluation in flight.
      previewNext.current = { id: selected, values: { [key]: value } };
      void pumpPreview();
    },
    [selected, pumpPreview],
  );

  const commitParam = useCallback(
    async (key: string, value: number) => {
      if (!selected) return;
      previewNext.current = null;
      previewSeq.current++;
      await run("shape.set_params", { target: selected, values: { [key]: value } });
      setShapePreview(null);
    },
    [selected, run],
  );

  const addPart = useCallback(async () => {
    const right = parts.reduce((max, p) => Math.max(max, toMm(placement(p)?.x) + toMm(box(p)?.width)), -20);
    const result = (await run("solid.box.create", {
      width: mm(40),
      depth: mm(40),
      height: mm(20),
      material: materials[0]?.id ?? null,
      placement: { x: mm(right + 20), y: mm(0), z: mm(0) },
    })) as { output: { id: string } } | null;
    if (result) {
      setSelected(result.output.id);
      setPanel("properties");
    }
  }, [parts, materials, run]);

  // Sketching by hand: the editor, and the plane menu for a new sketch.
  const [planeMenu, setPlaneMenu] = useState(false);
  const sketcher = useSketchEditor({
    entities: state.entities,
    run,
    notify,
    onFace: (plane, focus) => {
      if (plane) return viewport.current?.faceSketch(plane, focus);
      // Leaving a sketch: glide back to the usual 3D view, framed.
      setView("perspective");
      viewport.current?.leaveSketch();
    },
    onMade: (id) => {
      // The glide out of the sketch ends on the new part once it is drawn.
      selectWhenReady(id, true);
      setPanel("properties");
    },
  });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || e.metaKey || e.ctrlKey) return;
      if (e.key.toLowerCase() === "k" && !sketcher.editing) {
        e.preventDefault();
        setPlaneMenu(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sketcher.editing]);

  // Push and pull on a flat face of a program part: through the parameter that
  // drives the face when there is one, else as an exact push stored on the part.
  const [programFace, setProgramFace] = useState<ProgramFace | null>(null);
  const [faceDriver, setFaceDriver] = useState<FaceDriver | null | "finding">(null);
  const [faceDrag, setFaceDrag] = useState<number | null>(null);
  useEffect(() => {
    setProgramFace(null);
  }, [selected, selectedPartName]);
  // A stress study asking for a face: the next flat face clicked goes to it.
  const [stressPick, setStressPick] = useState<{ purpose: string; take: (f: ProgramFace) => void } | null>(null);
  const requestFace = useCallback((purpose: string, take: (f: ProgramFace) => void) => setStressPick({ purpose, take }), []);
  const showStress = useCallback((id: string, view: StressView | null) => viewport.current?.stress(id, view), []);
  const grabMotion = useCallback((spec: MotionGrab | null) => viewport.current?.grabMotion(spec), []);
  useEffect(() => {
    if (!stressPick) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setStressPick(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [stressPick]);
  const pickFace = useCallback(
    (f: ProgramFace | null, why?: "curved") => {
      if (stressPick) {
        if (why === "curved" || f?.curved) return notify("Supports and loads go on flat faces: pick a flat face");
        if (!f) return;
        stressPick.take(f);
        setStressPick(null);
        return;
      }
      if (why === "curved") notify("That surface could not be picked: try another spot on it");
      setProgramFace(f);
      setFaceDrag(null);
      if (!f) return;
      // A curved face is pushed by a finish; a flat one may have a parameter that already drives it.
      if (f.curved) return setFaceDriver(null);
      setFaceDriver("finding");
      engine.shapeFace(f.id, f.part, f.centroid, f.normal).then(
        (r) => setFaceDriver(r.driver),
        () => setFaceDriver(null),
      );
    },
    [engine, notify, stressPick],
  );
  /** The push finish a face drag would add (sized by a new parameter). */
  const pushFinish = useCallback(
    (f: ProgramFace): { finish: EdgeFinish; key: string; label: string } | null => {
      const e = state.entities.get(f.id);
      const prog = e && program(e);
      if (!e || !prog || !e.shape) return null;
      const bounds = f.part ? e.shape.parts?.find((p) => p.name === f.part) : e.shape;
      if (!bounds) return null;
      const at = [0, 1, 2].map((k) => {
        const size = bounds.bounds_max[k] - bounds.bounds_min[k];
        return size > 1e-9 ? Math.round(((f.centroid[k] - bounds.bounds_min[k]) / size) * 1e6) / 1e6 : 0.5;
      }) as [number, number, number];
      let n = 1;
      while (prog.params.some((q) => q.key === `push${n}`)) n++;
      const key = `push${n}`;
      return { key, label: `Push ${n} distance`, finish: { kind: "push", key, ...(f.part ? { part: f.part } : {}), edges: [{ at, kind: f.curved ? "curved_face" : "face", dir: f.normal }] } };
    },
    [state.entities],
  );
  const dragFace = useCallback(
    (d: number) => {
      const f = programFace;
      if (!f) return;
      setFaceDrag(d);
      if (faceDriver && faceDriver !== "finding") {
        previewParam(faceDriver.key, faceDriver.value + d / faceDriver.rate);
        return;
      }
      const push = pushFinish(f);
      const prog = program(state.entities.get(f.id)!);
      if (!push || !prog) return;
      previewNext.current = { id: f.id, values: { [push.key]: d }, finishes: [...(prog.finishes ?? []), push.finish] };
      void pumpPreview();
    },
    [programFace, faceDriver, previewParam, pushFinish, state.entities, pumpPreview],
  );
  const endFaceDrag = useCallback(
    async (commit: boolean, d: number) => {
      const f = programFace;
      setFaceDrag(null);
      if (!f || !commit || Math.abs(d) < 1e-9) {
        previewParam("", null);
        return;
      }
      if (faceDriver && faceDriver !== "finding") {
        await commitParam(faceDriver.key, Math.round((faceDriver.value + d / faceDriver.rate) * 1000) / 1000);
      } else {
        const push = pushFinish(f);
        const prog = program(state.entities.get(f.id)!);
        if (push && prog) {
          await run("shape.update", {
            target: f.id,
            params: [...prog.params, { key: push.key, label: push.label, value: d, unit: "mm", min: null, max: null, step: 0.5 }],
            finishes: [...(prog.finishes ?? []), push.finish],
          });
        }
        previewParam("", null);
      }
      setProgramFace(null);
    },
    [programFace, faceDriver, commitParam, pushFinish, state.entities, run, previewParam],
  );

  // The team: members, live links to their engines, clashes.
  const { team, refresh: refreshTeam } = useTeam(engine);

  // Teammates' live cursors: what each has selected and where they point. A
  // cursor not heard from for 30 s fades out (they stepped away or dropped off).
  const [cursors, setCursors] = useState<Map<string, TeamCursor & { at: number }>>(new Map());
  useEffect(() => {
    const off = engine.onCursor((c) =>
      setCursors((m) => {
        const next = new Map(m);
        if (c.gone) next.delete(c.engine);
        else next.set(c.engine, { ...c, at: Date.now() });
        return next;
      }),
    );
    const timer = window.setInterval(() => setCursors((m) => (m.size && [...m.values()].some((c) => Date.now() - c.at > 30000) ? new Map([...m].filter(([, c]) => Date.now() - c.at <= 30000)) : m)), 5000);
    return () => {
      off();
      window.clearInterval(timer);
    };
  }, [engine]);
  const teammates = useMemo(
    () =>
      [...cursors.values()].map((c) => ({
        engine: c.engine,
        name: c.name || "Teammate",
        color: MATE_COLORS[[...c.engine].reduce((h, ch) => (h * 31 + ch.charCodeAt(0)) >>> 0, 7) % MATE_COLORS.length]!,
        selected: c.selected ?? null,
        pointer: c.pointer ?? null,
      })),
    [cursors],
  );
  // Ours: sent when the pointer moves over the model or the selection changes, and every 10 s so they know we're here.
  const hoverAt = useRef<[number, number, number] | null>(null);
  const sendCursor = useCallback(() => engine.sendCursor({ selected, part: selectedPartName, pointer: hoverAt.current }), [engine, selected, selectedPartName]);
  useEffect(() => {
    if (!team?.online.length) return;
    sendCursor();
    const timer = window.setInterval(sendCursor, 10000);
    return () => window.clearInterval(timer);
  }, [sendCursor, team?.online.length]);
  const onHover3D = useCallback(
    (p: [number, number, number] | null) => {
      hoverAt.current = p;
      if (team?.online.length) sendCursor();
    },
    [sendCursor, team?.online.length],
  );

  // Rounding and bevelling edges of the selected part.
  const edger = useEdgeEditor({ engine, entities: state.entities, run, notify, setPreview: setShapePreview });

  // Comments pinned to the model: comment mode (C) drops a pin where the model is clicked.
  const me = team?.my_name?.trim() || "You";
  const threads = useThreads(state.entities);
  const [commenting, setCommenting] = useState(false);
  const [openThread, setOpenThread] = useState<string | null>(null);
  const [draft, setDraft] = useState<{ anchor: PinAnchor; strokes: Stroke[] } | null>(null);
  const [markTool, setMarkTool] = useState<MarkupTool | null>(null);
  const [commentFilter, setCommentFilter] = useState<"open" | "resolved" | "all">("open");
  const [pinsShown, setPinsShown] = useState(true);
  const [frameReq, setFrameReq] = useState<{ id: string } | null>(null);
  useEffect(() => {
    if (frameReq) viewport.current?.framePin(frameReq.id);
  }, [frameReq]);
  const openComments = threads.filter((t) => !t.resolved).length;
  const closeDraft = useCallback(() => {
    setDraft(null);
    setMarkTool(null);
  }, []);
  const toggleCommenting = useCallback(() => {
    setCommenting((on) => {
      if (on) {
        setDraft(null);
        setMarkTool(null);
      } else setPanel("comments");
      return !on;
    });
  }, []);
  const openThreadAt = useCallback(
    (id: string) => {
      closeDraft();
      setOpenThread(id);
      const t = threads.find((x) => x.id === id);
      // A resolved thread opened from the list shows its pin while it is open.
      if (t && !t.gone) setFrameReq({ id });
    },
    [closeDraft, threads],
  );
  // The open thread was deleted (here or by a teammate): close its card. A thread
  // just posted may open before it arrives, so only one seen and then gone closes.
  const openSeen = useRef<string | null>(null);
  useEffect(() => {
    if (!openThread) return;
    if (threads.some((t) => t.id === openThread)) openSeen.current = openThread;
    else if (openSeen.current === openThread) setOpenThread(null);
  }, [openThread, threads]);
  // Teammates' new comments and replies, as they arrive.
  const seenComments = useRef<Set<string> | null>(null);
  useEffect(() => {
    const ids = new Set<string>();
    const fresh: { author: string; text: string; reply: boolean; on: string | null }[] = [];
    for (const t of threads) {
      for (const m of [{ ...t, reply: false }, ...t.replies.map((r) => ({ ...r, reply: true }))]) {
        ids.add(m.id);
        if (seenComments.current && !seenComments.current.has(m.id) && m.author !== me) fresh.push({ author: m.author, text: m.text, reply: m.reply, on: t.on });
      }
    }
    if (state.connection === "online") seenComments.current = ids;
    const f = fresh.at(-1);
    if (f) {
      const said = f.text.length > 60 ? `${f.text.slice(0, 57)}…` : f.text;
      notify(`${f.author} ${f.reply ? "replied" : `commented${f.on ? ` on ${f.on}` : ""}`}: “${said}”`);
    }
  }, [threads, me, notify, state.connection]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Step back one thing at a time: the pen, the new comment, the open thread, comment mode.
        const editing = e.target instanceof HTMLElement && !!e.target.closest(".comment-message");
        if (editing) return;
        if (markTool) setMarkTool(null);
        else if (draft) closeDraft();
        else if (openThread) setOpenThread(null);
        else if (commenting) setCommenting(false);
        else return;
        e.preventDefault();
        e.stopPropagation();
        return;
      }
      if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key.toLowerCase() === "c" && !sketcher.editing && !edger.editing) {
        e.preventDefault();
        toggleCommenting();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [markTool, draft, openThread, commenting, closeDraft, toggleCommenting, sketcher.editing, edger.editing]);
  // Sketching or picking edges takes the clicks: comment mode steps aside.
  useEffect(() => {
    if (sketcher.editing || edger.editing) {
      setCommenting(false);
      closeDraft();
    }
  }, [sketcher.editing, edger.editing, closeDraft]);
  const postComment = useCallback(
    async (text: string) => {
      if (!draft) return false;
      const a = draft.anchor;
      const out = (await run("comment.create", { target: a.target, part: a.part, point: a.point, normal: a.normal, text, author: me, markup: draft.strokes.map((s) => ({ points: s.points, arrow: !!s.arrow })) })) as { id?: string } | null;
      if (!out) return false;
      closeDraft();
      if (out.id) setOpenThread(out.id);
      return true;
    },
    [draft, run, me, closeDraft],
  );
  const commentsView = useMemo((): CommentsView | null => {
    const showResolved = commentFilter !== "open";
    let pins: CommentPin[] = pinsShown || commenting ? pinsFor(threads, showResolved) : [];
    if (commentFilter === "resolved") pins = pins.filter((p) => p.resolved);
    // The open thread keeps its pin even when the filter would hide it.
    const active = openThread ? threads.find((t) => t.id === openThread) : undefined;
    if (active && !active.gone && !pins.some((p) => p.id === active.id)) pins = [...pins, ...pinsFor([active], true)];
    if (draft) {
      pins = [
        ...pins,
        { id: "draft", number: 0, initials: initialsOf(me), color: colorFor(me), target: draft.anchor.target, point: draft.anchor.point, normal: draft.anchor.normal, resolved: false, replies: 0, markup: draft.strokes, draft: true },
      ];
    }
    const card = draft ? (
      <DraftCard
        me={me}
        where={placeName(draft.anchor, state.entities)}
        strokes={draft.strokes.length}
        tool={markTool}
        onTool={setMarkTool}
        onUndoStroke={() => setDraft((d) => (d ? { ...d, strokes: d.strokes.slice(0, -1) } : d))}
        onPost={postComment}
        onCancel={closeDraft}
      />
    ) : active ? (
      <ThreadCard thread={active} me={me} run={run} onClose={() => setOpenThread(null)} />
    ) : null;
    if (!pins.length && !card && !commenting) return null;
    return {
      pins,
      active: draft ? "draft" : openThread,
      placing: commenting,
      tool: draft ? markTool : null,
      me: { initials: initialsOf(me), color: colorFor(me) },
      card,
      onPlace: (anchor) => {
        setOpenThread(null);
        setMarkTool(null);
        setDraft({ anchor, strokes: [] });
      },
      onOpen: (id) => {
        closeDraft();
        setOpenThread(id);
      },
      onStroke: (s) => setDraft((d) => (d ? { ...d, strokes: [...d.strokes, s].slice(-24) } : d)),
    };
  }, [threads, commentFilter, pinsShown, commenting, openThread, draft, markTool, me, state.entities, run, postComment, closeDraft]);
  useEffect(() => {
    // Selecting something else leaves edge picking.
    if (edger.editing && selected !== edger.editing) edger.stop();
  }, [selected, edger]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key.toLowerCase() === "e" && selected && !edger.editing && !sketcher.editing) {
        e.preventDefault();
        void edger.start(selected);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, edger, sketcher.editing]);


  /** A STEP file in: stored by the engine, added as a part named after the file, then selected. */
  const importStep = useCallback(
    async (file: File) => {
      if (!/\.(step|stp)$/i.test(file.name)) return notify("Forma imports STEP files (.step, .stp)");
      notify(`Importing ${file.name}…`);
      try {
        const res = await fetch(projectUrl(`/import/${encodeURIComponent(file.name)}?user=${encodeURIComponent(state.userId)}`), { method: "POST", body: file });
        if (!res.ok) return notify(await res.text());
        const out = (await res.json()) as { id: string; bodies: number };
        notify(`Imported ${file.name}: ${out.bodies} ${out.bodies === 1 ? "body" : "bodies"}`);
        // The reply and the live update that brings the part can come in either order.
        selectWhenReady(out.id);
        setPanel("properties");
      } catch (e) {
        notify(`Import failed: ${(e as Error).message}`);
      }
    },
    [notify, state.userId, selectWhenReady],
  );

  // Drop a STEP file anywhere on the canvas to import it.
  useEffect(() => {
    const over = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
    };
    const drop = (e: DragEvent) => {
      const f = e.dataTransfer?.files?.[0];
      if (!f) return;
      e.preventDefault();
      importStep(f);
    };
    window.addEventListener("dragover", over);
    window.addEventListener("drop", drop);
    return () => {
      window.removeEventListener("dragover", over);
      window.removeEventListener("drop", drop);
    };
  }, [importStep]);

  /** Steps that keep saved groups tidy when items leave them: shrink the group, or drop it below two. */
  const regroupSteps = useCallback(
    (leaving: string[]) => {
      const now = engine.getState().entities;
      return [...now.values()]
        .filter((e) => groupOf(e)?.members.some((m) => leaving.includes(m)))
        .map((e) => {
          const left = groupOf(e)!.members.filter((m) => !leaving.includes(m) && now.has(m));
          return left.length >= 2 ? { operation: "assembly.group.update", parameters: { target: e.id, members: left } } : { operation: "core.entity.delete", parameters: { target: e.id } };
        });
    },
    [engine],
  );
  const remove = useCallback(
    (id: string) => {
      const tidy = regroupSteps([id]);
      return tidy.length ? run("core.batch", { steps: [{ operation: "core.entity.delete", parameters: { target: id } }, ...tidy] }) : run("core.entity.delete", { target: id });
    },
    [run, regroupSteps],
  );
  const groups = useMemo(() => [...state.entities.values()].filter((e) => groupOf(e)), [state.entities]);
  /** The saved group an item belongs to, with only the members that still exist. */
  const groupFor = useCallback(
    (id: string) => {
      const g = groups.find((e) => groupOf(e)!.members.includes(id));
      return g ? { id: g.id, ...groupOf(g)!, members: groupOf(g)!.members.filter((m) => state.entities.has(m)) } : null;
    },
    [groups, state.entities],
  );
  const select = useCallback(
    (id: string | null, add = false, single = false) => {
      if (add && id && selected && id !== selected) {
        // Shift-click: add it to the group, or take it out again.
        setAlso((a) => (a.includes(id) ? a.filter((x) => x !== id) : [...a, id]));
        return;
      }
      if (add && id && id === selected && alsoRef.current.length) {
        // Shift-click on the main one: the next in the group takes over.
        const [next, ...rest] = alsoRef.current;
        setSelected(next);
        setAlso(rest);
        return;
      }
      // Clicking one member of a saved group picks the whole group (Alt-click: just that one).
      const g = id && !add && !single ? groupFor(id) : null;
      setAlso(g ? g.members.filter((m) => m !== id) : []);
      setSelected(id);
      if (id) setPanel((p) => (p === "objects" ? p : "properties"));
    },
    [selected, groupFor],
  );
  useEffect(() => {
    if (!selected) setAlso([]);
  }, [selected]);

  const togglePanel = useCallback((p: PanelKind) => setPanel((current) => (current === p ? null : p)), []);
  const hide = useCallback((g: Group) => setPrefs((p) => ({ ...p, hidden: { ...p.hidden, [g]: true } })), []);
  const show = useCallback((g: Group) => setPrefs((p) => ({ ...p, hidden: { ...p.hidden, [g]: false } })), []);
  const restoreAll = useCallback(() => {
    setFocus((wasFocus) => {
      if (!wasFocus) setPanel(null);
      return false;
    });
    update({ hidden: {} });
  }, [update]);

  const setMode = useCallback(
    (mode: AiMode | "open") =>
      setPrefs((p) => {
        const next = mode === "open" ? p.lastMode : mode;
        return { ...p, aiMode: next, lastMode: next === "floating" || next === "docked" || next === "expanded" ? next : p.lastMode };
      }),
    [],
  );

  const say = useCallback((role: "user" | "assistant", text: string) => setPrefs((p) => ({ ...p, messages: [...p.messages, { role, text }] })), []);

  // With Claude on this computer, the panel talks to it; otherwise the local interpreter answers.
  const claude = state.ai.available;
  const submit = useCallback(
    async (value?: string) => {
      const text = (value ?? prefs.draft).trim();
      if (!text) return;
      if (claude) {
        setPrefs((p) => ({
          ...p,
          draft: value === undefined ? "" : p.draft,
          voice: false,
          aiMode: p.aiMode === "mini" || p.aiMode === "hidden" ? p.lastMode : p.aiMode,
        }));
        engine.askAi(text, { selected, picked_part: selectedPartName, isolated, hidden: [...hiddenItems], color_by: colorBy }).catch((e) => notify(`Forma AI: ${(e as EngineError).message}`));
        return;
      }
      if (busy) return;
      say("user", text);
      setProposal(null);
      setPrefs((p) => ({
        ...p,
        draft: value === undefined ? "" : p.draft,
        voice: false,
        aiMode: p.aiMode === "mini" || p.aiMode === "hidden" ? p.lastMode : p.aiMode,
      }));
      const reply = interpret(text, { selected: selectedPart, parts, materials, world });
      if (reply.action === "check") {
        try {
          const r = await engine.check();
          setReport(r);
          const problems = r.findings.filter((f) => f.severity !== "info");
          const head = r.requirements_total
            ? `${r.requirements_passing} of ${r.requirements_total} requirements met.`
            : "No requirements yet; I checked the parts themselves.";
          say(
            "assistant",
            problems.length
              ? `${head}\n\n${problems.map((f) => `• ${f.message}${f.fix ? `\n  Fix: ${f.fix.label}` : ""}`).join("\n")}\n\nOpen Requirements to apply a fix in one click.`
              : `${head} Everything checks out.`,
          );
        } catch (e) {
          say("assistant", `I couldn't run the check: ${(e as EngineError).message}`);
        }
        return;
      }
      if (reply.action) {
        say("assistant", reply.text);
        await (reply.action === "undo" ? undo() : redo());
        return;
      }
      if (reply.proposal) {
        const first = reply.proposal.steps[0];
        if (typeof first.parameters !== "function") {
          setBusy(true);
          try {
            await engine.preview(first.operation, first.parameters);
          } catch (e) {
            say("assistant", `The engine rejected this change: ${(e as EngineError).message}`);
            return;
          } finally {
            setBusy(false);
          }
        }
        setProposal(reply.proposal);
      }
      say("assistant", reply.text);
    },
    [prefs.draft, busy, say, selectedPart, parts, materials, world, undo, redo, engine, claude, selected, selectedPartName, notify, isolated, hiddenItems, colorBy],
  );

  const apply = useCallback(async () => {
    if (!proposal) return;
    setBusy(true);
    const outputs: Record<string, unknown>[] = [];
    try {
      for (const step of proposal.steps) {
        const parameters = typeof step.parameters === "function" ? step.parameters(outputs) : step.parameters;
        outputs.push((await engine.commit(step.operation, parameters)) as Record<string, unknown>);
      }
      setProposal(null);
      say("assistant", "Applied. You can keep editing directly or undo this change.");
      const last = outputs.at(-1) as { output?: { id?: string } } | undefined;
      if (proposal.selectCreated && last?.output?.id) select(last.output.id);
    } catch (e) {
      say("assistant", `That didn't go through: ${(e as EngineError).message}`);
    } finally {
      setBusy(false);
    }
  }, [proposal, engine, say, select]);

  const exportImage = useCallback(() => {
    const url = viewport.current?.snapshot();
    if (!url) return;
    const a = document.createElement("a");
    a.href = url;
    a.download = "forma-viewport.png";
    a.click();
    notify("Exported viewport image");
  }, [notify]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (document.querySelector("dialog[open]")) return;
        setProgramFace(null);
        setAlso([]);
        restoreAll();
        return;
      }
      if (isTyping(e.target)) return;
      const mod = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();
      if (mod && key === "z") {
        e.preventDefault();
        if (e.shiftKey) redo();
        else undo();
      } else if (mod && key === "y") {
        e.preventDefault();
        redo();
      } else if (mod || e.altKey) {
        return;
      } else if (key === "a") {
        setPrefs((p) => ({ ...p, aiMode: p.aiMode === "hidden" ? p.lastMode : "hidden" }));
      } else if (key === "f") {
        setFocus((f) => !f);
      } else if (key === "p" && selected) {
        setPivotMode((on) => !on);
      } else if (selected && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "PageUp", "PageDown"].includes(e.key)) {
        e.preventDefault();
        const step = e.shiftKey ? 10 : 1;
        const delta = {
          ArrowLeft: { x: -step }, ArrowRight: { x: step }, ArrowUp: { y: step }, ArrowDown: { y: -step }, PageUp: { z: step }, PageDown: { z: -step },
        }[e.key];
        nudge(selected, delta!);
      } else if ((e.key === "Delete" || e.key === "Backspace") && selected) {
        if (alsoRef.current.length) {
          // The whole group, as one change.
          const gone = [selected, ...alsoRef.current];
          void run("core.batch", { steps: [...gone.map((id) => ({ operation: "core.entity.delete", parameters: { target: id } })), ...regroupSteps(gone)] });
          setAlso([]);
        } else remove(selected);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo, remove, selected, restoreAll, nudge]);

  const hidden = prefs.hidden;
  const docked = prefs.aiMode === "docked" && !focus;
  const context = `${projectTitle()} · ${selectedPart ? partName(selectedPart) : "No selection"}`;

  return (
    <div className={`app ${docked ? "is-docked" : ""} ${focus ? "is-focus" : ""}`}>
      <main className="workspace" aria-label="Modeling canvas">
        <Viewport
          ref={viewport}
          entities={viewEntities}
          selected={selected}
          live={live}
          also={also}
          overlays={overlays}
          pivotMode={pivotMode}
          pivotLive={pivotLive}
          onPickPivot={(frac, keepMode) => {
            if (!keepMode) setPivotMode(false);
            if (frac && selected) run("solid.part.set_pivot", { target: selected, u: frac[0], v: frac[1], w: frac[2] });
          }}
          onSelect={select}
          teammates={teammates}
          onHover3D={onHover3D}
          comments={commentsView}
          selectedPart={selectedPartName}
          hidden={hiddenItems}
          isolated={isolated}
          colorBy={colorBy}
          sketch={sketcher.picture}
          onSketchPointer={sketcher.onPointer}
          onEditDimension={sketcher.openDim}
          edges={edger.picture}
          onEdgeClick={edger.toggle}
          programFace={programFace}
          onProgramFace={pickFace}
          onFaceDrag={dragFace}
          onFaceDragEnd={endFaceDrag}
          onSelectPart={(name) => {
            setSelectedPartName(name);
            if (name) setPanel("properties");
          }}
          onDrag={(id, patch) => preview(id, patch)}
          onDragEnd={(commit) => (commit ? commitLive() : setLive(null))}
        />
        <div className="view-caption">
          <span>{projectTitle().toUpperCase()}</span>
          <span>
            {view[0].toUpperCase() + view.slice(1)} · {parts.length} {parts.length === 1 ? "part" : "parts"}
          </span>
        </div>
        {state.connection === "online" && parts.length === 0 && !sketcher.editing && !planeMenu && (
          <div className="empty-hint">
            <p>Start with a part, ask Forma AI, or drop a STEP file here.</p>
            <div className="empty-actions">
              <button type="button" className="primary" onClick={addPart}>
                Add box part
              </button>
              <label className="secondary import-step">
                Import STEP
                <input
                  type="file"
                  accept=".step,.stp,.STEP,.STP"
                  aria-label="Import STEP file"
                  onChange={(e) => {
                    const f = e.target.files?.[0];
                    if (f) importStep(f);
                    e.target.value = "";
                  }}
                />
              </label>
            </div>
          </div>
        )}
        {state.connection === "offline" && <div className="offline-note">Engine offline · reconnecting…</div>}
        {sketcher.bar}
        {sketcher.dimEditor}
        {edger.bar}
        {also.length > 0 && !stressPick && (
          <div className="face-hint floating" role="status">
            {(() => {
              const g = selected ? groupFor(selected) : null;
              const all = selected ? [selected, ...also] : also;
              const isGroup = !!g && g.members.length === all.length && all.every((m) => g.members.includes(m));
              return (
                <>
                  <b>{isGroup ? g!.name : `${all.length} items`}</b>
                  {isGroup ? ` · ${all.length} items · Alt-click picks one` : " selected"} · drag the arrows to move them together · Delete removes them · Shift-click adds or takes away · Esc
                  {isGroup ? (
                    <button type="button" className="hint-action" onClick={() => void run("core.entity.delete", { target: g!.id }).then((ok) => ok && notify(`Ungrouped ${g!.name}`))}>
                      Ungroup
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="hint-action"
                      onClick={() => {
                        // A new group takes over members of any group it swallows whole; others just lose those items.
                        const steps = regroupSteps(all);
                        const used = new Set(groups.map((e) => groupOf(e)!.name));
                        let n = 1;
                        while (used.has(`Group ${n}`)) n++;
                        const name = `Group ${n}`;
                        void run("core.batch", { steps: [...steps, { operation: "assembly.group.create", parameters: { name, members: all } }] }).then((ok) => ok && notify(`Grouped ${all.length} items as ${name}`));
                      }}
                    >
                      Group
                    </button>
                  )}
                </>
              );
            })()}
          </div>
        )}
        {commenting && !draft && !stressPick && (
          <div className="face-hint floating comment-hint" role="status">
            <b>Comment</b> · click the model to pin a comment · <kbd>C</kbd> or Esc to leave
          </div>
        )}
        {stressPick && (
          <div className="face-hint floating" role="status">
            {stressPick.purpose} · Esc to cancel
          </div>
        )}
        {programFace && !stressPick && (
          <div className="face-hint floating" role="status">
            {faceDrag !== null ? (
              faceDriver && faceDriver !== "finding" ? (
                <>
                  {faceDriver.label} <b>{fmtNum(faceDriver.value + faceDrag / faceDriver.rate)} mm</b>
                </>
              ) : (
                <>
                  {faceDrag >= 0 ? "Push out" : "Pull in"} <b>{fmtNum(Math.abs(faceDrag))} mm</b>
                </>
              )
            ) : faceDriver === "finding" ? (
              "Finding what shapes this face…"
            ) : faceDriver ? (
              <>
                Drag the arrow to change <b>{faceDriver.label}</b> ({fmtNum(faceDriver.value)} mm) · Esc to leave
              </>
            ) : (
              <>
                Drag the arrow to {programFace.curved ? "thicken or thin this curved surface" : "push or pull this face"} · it's kept as a linkable distance · Esc to leave
              </>
            )}
          </div>
        )}
        <SyncBanner team={team} />
        {planeMenu && !sketcher.editing && (
          <PlaneMenu
            onPick={(p) => {
              setPlaneMenu(false);
              sketcher.start(p);
            }}
            onClose={() => setPlaneMenu(false)}
          />
        )}
      </main>

      {!focus && (
        <div className="chrome">
          {hidden.project ? (
            <Collapsed group="project" onShow={() => show("project")} />
          ) : (
            <ProjectPill
              connection={state.connection}
              revision={state.revision}
              health={health}
              onHealth={() => setPanel("requirements")}
              onMenu={() => togglePanel("project")}
              onHide={() => hide("project")}
            />
          )}
          {hidden.share ? (
            <Collapsed group="share" onShow={() => show("share")} />
          ) : (
            <TeamPill team={team} onOpen={() => setDialog("share")} onHide={() => hide("share")} />
          )}
          {hidden.tools ? (
            <Collapsed group="tools" onShow={() => show("tools")} />
          ) : (
            <ToolRail
              panel={panel}
              canEdit={state.connection === "online"}
              onPanel={togglePanel}
              onSelectTool={() => select(null)}
              onAdd={addPart}
              onAi={() => setMode(prefs.aiMode === "hidden" || prefs.aiMode === "mini" ? "open" : "hidden")}
              onHide={() => hide("tools")}
              sketching={!!sketcher.editing || planeMenu}
              onSketch={() => (sketcher.editing ? sketcher.finish() : setPlaneMenu((m) => !m))}
              edging={!!edger.editing}
              canEdge={!!selectedPart && !!program(selectedPart)}
              onEdges={() => (edger.editing ? edger.stop() : selected && edger.start(selected))}
              commenting={commenting}
              openComments={openComments}
              onComment={toggleCommenting}
            />
          )}
          {hidden.view ? (
            <Collapsed group="view" onShow={() => show("view")} />
          ) : (
            <ViewBar
              view={view}
              onViews={() => togglePanel("views")}
              onUndo={undo}
              onZoom={(f) => viewport.current?.zoom(f)}
              onFocus={() => setFocus(true)}
              onStorage={() => setDialog("storage")}
              onExport={exportImage}
              onHide={() => hide("view")}
            />
          )}
        </div>
      )}
      {focus && <Restore onRestore={restoreAll} />}

      {!focus && panel && (
        <ToolPanel kind={panel} onClose={() => setPanel(null)}>
          {panel === "project" && (
            <ProjectBody
              onStorage={() => setDialog("storage")}
              onHistory={() => setPanel("history")}
              onRestore={restoreAll}
              onFocus={() => setFocus(true)}
            />
          )}
          {panel === "comments" && (
            <CommentsPanel
              threads={threads}
              active={openThread}
              filter={commentFilter}
              onFilter={setCommentFilter}
              pinsShown={pinsShown}
              onPinsShown={setPinsShown}
              onOpen={openThreadAt}
              onNew={() => !commenting && toggleCommenting()}
            />
          )}
          {panel === "objects" && (
            <ObjectsBody
              parts={parts}
              materials={materials}
              selected={selected}
              onSelect={(id) => setSelected(id)}
              onAdd={addPart}
              run={run}
              onImport={importStep}
              parameters={[...state.entities.values()].some((e) => parameterSet(e)) ? <ParameterSets entities={state.entities} run={run} /> : undefined}
              sketches={
                [...state.entities.values()].some((e) => sketchOf(e)) ? (
                  <div className="sketch-list">
                    {[...state.entities.values()]
                      .filter((e) => sketchOf(e))
                      .map((e) => (
                        <button key={e.id} type="button" className={`list-row ${sketcher.editing === e.id ? "active" : ""}`} onClick={() => sketcher.edit(e.id)}>
                          <Icon name="pencil" />
                          <span>
                            {sketchOf(e)!.name}
                            <small className="row-count">{sketchOf(e)!.params.length} dimensions</small>
                          </span>
                          <Icon name="chevron" />
                        </button>
                      ))}
                  </div>
                ) : undefined
              }
              groups={
                groups.length ? (
                  <GroupList
                    groups={groups.map((e) => ({ id: e.id, name: groupOf(e)!.name, members: groupOf(e)!.members.filter((m) => state.entities.has(m)) }))}
                    active={(g) => !!selected && g.members.includes(selected) && g.members.every((m) => m === selected || also.includes(m))}
                    onPick={(id) => {
                      const g = groupOf(state.entities.get(id));
                      const first = g?.members.find((m) => state.entities.has(m));
                      if (first) select(first);
                    }}
                    onRename={(id, name) => void run("assembly.group.update", { target: id, name })}
                    onUngroup={(id) => void run("core.entity.delete", { target: id })}
                  />
                ) : undefined
              }
              tree={(() => {
                const building = stackingSubject(state.entities);
                const tree = (
                  <ComponentTree
                    entities={state.entities}
                    selected={selected}
                    hidden={hiddenItems}
                    isolated={isolated}
                    onSelect={select}
                    onToggleHidden={(id) =>
                      setHiddenItems((h) => {
                        const n = new Set(h);
                        if (n.has(id)) n.delete(id);
                        else n.add(id);
                        return n;
                      })
                    }
                    onIsolate={setIsolated}
                  />
                );
                if (!building) return tree;
                return (
                  <>
                    <div className="seg" role="tablist">
                      <button type="button" role="tab" aria-selected={objectsTab === "tree"} className={objectsTab === "tree" ? "active" : ""} onClick={() => setObjectsTab("tree")}>
                        Components
                      </button>
                      <button type="button" role="tab" aria-selected={objectsTab === "stacking"} className={objectsTab === "stacking" ? "active" : ""} onClick={() => setObjectsTab("stacking")}>
                        Stacking plan
                      </button>
                    </div>
                    {objectsTab === "tree" ? (
                      tree
                    ) : (
                      <StackingPlan entities={state.entities} building={building} selected={selected} colorBy={colorBy} onColorBy={setColorBy} onSelect={select} />
                    )}
                  </>
                );
              })()}
            />
          )}
          {panel === "properties" && selectedPart && instance(selectedPart) && (
            <InstanceBody entity={selectedPart} entities={state.entities} run={run} onSelect={select} />
          )}
          {panel === "properties" && !(selectedPart && instance(selectedPart)) && (
            <PropertiesBody
              part={selectedPart}
              shape={selectedPart && isPart(selectedPart) ? (live?.id === selectedPart.id ? live : shapeOf(selectedPart)) : null}
              previewing={!!live && live.id === selected}
              materials={materials}
              lockRatio={lockRatio}
              onToggleLock={() => setLockRatio((l) => !l)}
              pivot={pivotOf(selectedPart)}
              pivotMode={pivotMode}
              onTogglePivot={() => setPivotMode((on) => !on)}
              shapeStats={shapePreview && shapePreview.id === selected ? shapePreview.stats : (selectedPart?.shape ?? null)}
              shapePreviewing={!!shapePreview && shapePreview.id === selected}
              onParamLive={previewParam}
              onParamCommit={commitParam}
              onParamFormula={(key, expr) => selected && run("shape.link_param", { target: selected, key, expr })}
              onTurn={turn}
              run={run}
              onPickFace={requestFace}
              onShowStress={showStress}
              onGrab={grabMotion}
              onEdges={() => selected && edger.start(selected)}
              onRemoveFinish={(i) => {
                const prog = selectedPart && program(selectedPart);
                if (!selected || !prog) return;
                const finishes = (prog.finishes ?? []).filter((_, j) => j !== i);
                // Its size parameter goes too, unless another finish or the program still uses it.
                const key = prog.finishes![i].key;
                const used = finishes.some((f) => f.key === key) || new RegExp(`\\bp\\.${key}\\b`).test(prog.source);
                run("shape.update", { target: selected, finishes, ...(used ? {} : { params: prog.params.filter((q) => q.key !== key) }) });
              }}
              onPivotLive={(axis, value) => setPivotLive(value === null ? null : pivotWith(axis, value))}
              onPivotCommit={async (axis, value) => {
                const f = pivotWith(axis, value);
                if (f && selected) await run("solid.part.set_pivot", { target: selected, u: f[0], v: f[1], w: f[2] });
                setPivotLive(null);
              }}
              onLive={(patch) => selected && preview(selected, patch && toPlacement(patch, selectedPart))}
              onCommit={(patch) => selected && commitPatch(selected, toPlacement(patch, selectedPart))}
              onAssign={(material) => selected && run("solid.material.assign", { target: selected, material })}
              onCreateMaterial={async (name, density) => {
                const created = (await run("solid.material.create", { name, density: { value: density, unit: "kg/m3" } })) as { output: { id: string } } | null;
                if (created && selected) await run("solid.material.assign", { target: selected, material: created.output.id });
              }}
              onDelete={() => selected && remove(selected)}
              entities={state.entities}
              selectedPart={selectedPartName}
              onSelectPart={setSelectedPartName}
              explode={explode}
              onExplode={setExplode}
            />
          )}
          {panel === "requirements" && (
            <RequirementsPanel
              requirements={requirements}
              entities={state.entities}
              previews={previews}
              findings={report?.findings ?? []}
              checking={checking}
              focus={focusReq}
              onFocus={setFocusReq}
              onSelectPart={(id) => setSelected(id)}
              onCreate={createRequirement}
              onUpdate={(id, rule) => run("intent.requirement.update", { target: id, rule })}
              onDelete={(id) => run("core.entity.delete", { target: id })}
              onFix={applyFix}
              onCheck={runCheck}
            />
          )}
          {panel === "history" && <HistoryBody history={state.history} userId={state.userId} onUndo={undo} onRedo={redo} />}
          {panel === "views" && (
            <ViewsBody
              view={view}
              wireframe={wireframe}
              turntable={turntable}
              onView={(v) => {
                setView(v);
                viewport.current?.view(v);
              }}
              onFit={() => viewport.current?.fit()}
              onWireframe={() => {
                viewport.current?.setWireframe(!wireframe);
                setWireframe(!wireframe);
              }}
              onTurntable={() => {
                viewport.current?.setTurntable(!turntable);
                setTurntable(!turntable);
              }}
            />
          )}
        </ToolPanel>
      )}

      {!focus && (
        <AiPanel
          mode={prefs.aiMode}
          context={context}
          messages={claude ? state.ai.chat : prefs.messages}
          claude={claude}
          streaming={state.ai.streaming}
          thinking={state.ai.thinking}
          aiBusy={state.ai.busy}
          userId={state.userId}
          onStop={() => engine.stopAi()}
          draft={prefs.draft}
          voice={prefs.voice}
          proposal={proposal}
          busy={busy}
          position={prefs.aiPosition}
          onDraft={(draft) => update({ draft })}
          onSubmit={submit}
          onMode={setMode}
          onVoice={() =>
            setPrefs((p) => ({ ...p, voice: !p.voice, aiMode: p.aiMode === "mini" ? p.lastMode : p.aiMode }))
          }
          onApply={apply}
          onDiscard={() => {
            setProposal(null);
            say("assistant", "Discarded. Nothing was changed.");
          }}
          onMoved={(aiPosition) => update({ aiPosition })}
        />
      )}

      {dialog === "share" && <TeamDialog team={team} entities={state.entities} run={run} refresh={refreshTeam} onClose={() => setDialog(null)} />}
      {dialog === "storage" && <StorageDialog revision={state.revision} onClose={() => setDialog(null)} />}
      {home && (
        <ProjectsHome
          onClose={() => {
            setHome(false);
            const url = new URL(location.href);
            url.searchParams.delete("home");
            history.replaceState(null, "", url.toString());
          }}
        />
      )}

      <div className={`toast ${toast ? "visible" : ""}`} role="status" aria-live="polite">
        {toast}
      </div>
    </div>
  );
}
