// The Electronics workspace: a board's copper and parts in 2D (layers, pads,
// tracks, pours, ratsnest, design-rule markers), its parts and nets, the
// checks, the fab files and where to get the board and its components.
// The engine resolves and checks the board (GET /pcb/<id>); this draws it and
// sends the person's edits back as pcb.* commands. The board is also a real
// 3D part in the main view ("Show in 3D").

import { type PointerEvent as ReactPointerEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import { FabCheck, SimulateTab } from "./PcbSim";
import { GerberViewer, ToolsTab } from "./PcbTools";
import { LibraryPane } from "./PcbLibrary";
import { SchematicPane } from "./Schematic";
import { SignalPanel, type NetClassV, type PairV, type SignalV, TunePanel, type TunePlan } from "./PcbSignal";

type P = [number, number];
type Prim = { pts: P[]; r: number };
type Pad = {
  component: string;
  reference: string;
  number: string;
  net: string | null;
  pin_name: string | null;
  kind: "smd" | "tht" | "npth";
  shape: string;
  layers: string[];
  prim: Prim;
  at: P;
  size: P;
  rotation: number;
  drill: P | null;
  side: "top" | "bottom";
};
type Body = { outline: P[]; z: P; material: string; cylinder: boolean; center: P; size: P };
type Comp = {
  id: string;
  reference: string;
  value: string;
  footprint: string;
  kind: string;
  at: P;
  rotation: number;
  side: "top" | "bottom";
  mpn: string | null;
  manufacturer: string | null;
  lcsc: string | null;
  description: string;
  dnp: boolean;
  height: number;
  courtyard: P[];
  silk: P[][];
  bodies: Body[];
  label: P;
  pins: Record<string, string>;
  pin_names: Record<string, string>;
  known: boolean;
};
type Track = { id: string; net: string | null; layer: string; width: number; points: P[]; auto: boolean; reroute?: boolean; vertices?: P[]; bulges?: number[] };
type Via = { id: string; net: string | null; at: P; drill: number; diameter: number; kind?: "through" | "blind" | "buried" | "micro"; from?: string; to?: string; stack?: Record<string, number>; group?: string };
type GraphicV = { id: string; layer: string; kind: string; net: string | null; width: number; strokes: P[][]; fills: P[][][]; bbox: [number, number, number, number]; text?: string; value?: number; locked?: boolean };
type AreaV = { id: string; name: string; outline: P[]; layers: string[]; no_tracks: boolean; no_vias: boolean; no_pads: boolean; no_pours: boolean; no_footprints: boolean; placement?: string[] };
type TeardropV = { track: string; net: string | null; layer: string; polygon: P[]; on: string };
type Dielectric = { kind: "core" | "prepreg"; thickness: number; material: string; epsilon_r?: number; loss_tangent?: number };
type StackRow = { kind: string; name: string; thickness: number; material?: string; epsilon_r?: number; copper_oz?: number };
type StackupV = { rows: StackRow[]; dielectrics: Dielectric[]; copper_oz: number[]; total: number; custom: boolean; finish: string };
type RuleV = { name: string; layer?: string; condition?: string; constraints: { kind: string; min?: number; max?: number; disallow?: string[]; unchecked?: boolean }[]; severity: string };
type TeardropSettings = { vias: boolean; pads: boolean; smd: boolean; length_ratio: number; max_length: number; width_ratio: number; max_width: number; curved: boolean; max_track_ratio: number };
type Zone = { id: string; net: string; layer: string; outline: P[]; fill: P[][][]; priority: number; area: number };
type Hole = { at: P; size: P; rotation: number; plated: boolean; owner: string };
type Net = { name: string; pins: string[]; class: string; islands: number; routed: boolean; length: number };
type Violation = { code: string; severity: "error" | "warning"; message: string; at: P; layer?: string; objects: string[]; refs: string[] };
type Rules = { min_track: number; min_space: number; min_drill: number; min_annular: number; edge_clearance: number; hole_to_hole: number; silk_width: number; silk_height: number; mask_expansion: number; courtyards: boolean };
type Stats = {
  components: number;
  smd_pads: number;
  tht_pads: number;
  nets: number;
  routed_nets: number;
  tracks: number;
  vias: number;
  holes: number;
  track_length: number;
  smallest_track: number | null;
  smallest_drill: number | null;
  smallest_annular: number | null;
  smallest_gap: number | null;
  tallest_top: number;
  tallest_bottom: number;
};
type BoardView = {
  id: string;
  name: string;
  layers: string[];
  thickness: number;
  finish: string;
  copper_oz: number;
  mask: string;
  silk: string;
  revision: string;
  quantity: number;
  fab: string | null;
  rules: Rules;
  model: string | null;
  outline: P[];
  cutouts: P[][];
  bounds: [number, number, number, number];
  components: Comp[];
  pads: Pad[];
  tracks: Track[];
  vias: Via[];
  zones: Zone[];
  holes: Hole[];
  nets: Net[];
  ratsnest: { net: string; a: P; b: P }[];
  violations: Violation[];
  stats: Stats;
  graphics: GraphicV[];
  areas: AreaV[];
  teardrops: TeardropV[];
  stackup: StackupV;
  custom_rules: RuleV[];
  drc_rules?: string;
  teardrop_settings: TeardropSettings | null;
  signal: SignalV & { net_classes: NetClassV[] };
};
type Fab = { id: string; name: string; country: string; site: string; quote: string; capabilities_source: string; files: string; assembly: string; notes: string };
type FabReport = { fab: Fab; can_make: boolean; checks: { what: string; ok: boolean; board: string; fab: string }[]; estimate: { amount: number; currency: string; working: string; source: string } | null };
type Choice = { supplier?: string; sku?: string; unit_price?: number; currency?: string; url?: string; note?: string };
type BomLine = {
  key: string;
  references: string[];
  quantity: number;
  value: string;
  footprint: string;
  mpn: string | null;
  manufacturer: string | null;
  lcsc: string | null;
  lcsc_class: "Basic" | "Extended" | null;
  description: string;
  dnp: boolean;
  tht: boolean;
  kind: string;
  choice: Choice | null;
  links: { name: string; url: string }[];
};
type Detail = { board: BoardView; size: P; errors: number; fabs: FabReport[]; bom: BomLine[]; totals: Record<string, number>; unpriced: number; files: string[] };
type BoardEntry = { id: string; name: string; layers: number; model: string | null; errors: number | null; components: number | null };
type Run = (operation: string, parameters: Record<string, unknown>) => Promise<unknown>;
type Tab = "parts" | "nets" | "checks" | "setup" | "fab" | "bom" | "sim" | "tools";
type Selection = { kind: "component" | "track" | "via" | "zone"; id: string } | null;
type LayerKey = "top" | "bottom" | "inner" | "silk" | "courtyard" | "ratsnest" | "zones" | "drc" | "drawings" | "areas";
// The interactive router (engine: POST /pcb/<id>/router, pcb.route.interactive / pcb.route.drag).
type RouterMode = "walkaround" | "shove" | "highlight";
type AngleMode = "45" | "90" | "free";
type Collision = { at: P; layer: string | null; with: string; gap: number; need: number };
type Plan = {
  ok: boolean;
  reached: boolean;
  mode: RouterMode;
  net: string | null;
  width: number;
  via_drill: number;
  via_diameter: number;
  clearance: number;
  runs: { layer: string; points: P[]; bulges?: number[]; width?: number }[];
  vias: P[];
  tracks: { id: string; points: P[]; bulges?: number[] }[];
  moved_vias: { id: string; at: P }[];
  collisions: Collision[];
  end: P;
  layer: string;
  notes: string[];
  length?: number;
  target?: number;
};
type PairPlan = { ok: boolean; reached: boolean; pair: string; width: number; gap: number; centre: P[]; p: Plan; n: Plan; length_p: number; length_n: number; skew: number; notes: string[] };
type RouteStep = { at: P | string; layer: string; diagonal_first: boolean };
type ItemDrag = { id: string; kind: "segment" | "corner" | "arc" | "via"; index: number; from: P; client: P; moved: boolean };
const ROUTER_MODES: RouterMode[] = ["walkaround", "shove", "highlight"];
const ANGLES: AngleMode[] = ["45", "90", "free"];
const WIDTHS = [0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.5, 2.0];
const LAYER_NAME = (l: string) => (l === "top" ? "Top" : l === "bottom" ? "Bottom" : l.replace(/^in/, "Inner "));

const MASKS: Record<string, string> = { green: "#1d4a33", black: "#16181c", blue: "#173a63", red: "#6b1f22", white: "#d9dad6", purple: "#3f2463", yellow: "#8a7a1c" };
const COPPER: Record<string, string> = { top: "#e0614f", bottom: "#4f8fe6", in1: "#d6a43a", in2: "#5cc78a" };
// Inner layers 3–30 cycle through distinct hues.
for (let i = 3; i <= 30; i++) COPPER[`in${i}`] = `hsl(${(i * 47 + 20) % 360} 55% 58%)`;
const VIA_COLOR: Record<string, string> = { blind: "#b9a3e6", buried: "#7fcdbf", micro: "#ef9fc8" };
const DRAWING_LAYERS = ["fab_top", "fab_bottom", "user", "comments"];
const FMT = (v: number, d = 2) => (Math.abs(v) < 1e-9 ? "0" : v.toFixed(d).replace(/\.?0+$/, ""));
const money = (v: number, cur: string) => new Intl.NumberFormat(undefined, { style: "currency", currency: cur, maximumFractionDigits: v < 1 ? 4 : 2 }).format(v);

/** A centreline with arc segments (bulge = tan(sweep / 4), counter-clockwise positive) as points. */
function flatten(points: P[], bulges?: number[]): P[] {
  if (!bulges || !bulges.some((x) => Math.abs(x) > 1e-9)) return points;
  const out: P[] = [points[0]];
  for (let i = 0; i + 1 < points.length; i++) {
    const [a, c] = [points[i], points[i + 1]];
    const bu = bulges[i] ?? 0;
    const ch = Math.hypot(c[0] - a[0], c[1] - a[1]);
    if (Math.abs(bu) < 1e-9 || ch < 1e-9) {
      out.push(c);
      continue;
    }
    const right = [(c[1] - a[1]) / ch, -(c[0] - a[0]) / ch];
    const sag = (bu * ch) / 2;
    const mid = [(a[0] + c[0]) / 2 + right[0] * sag, (a[1] + c[1]) / 2 + right[1] * sag];
    const r = (ch * ch) / 4 / (2 * Math.abs(sag)) + Math.abs(sag) / 2;
    const cen = [mid[0] - right[0] * Math.sign(sag) * r, mid[1] - right[1] * Math.sign(sag) * r];
    const a0 = Math.atan2(a[1] - cen[1], a[0] - cen[0]);
    const sweep = 4 * Math.atan(bu);
    const n = Math.max(4, Math.ceil(Math.abs(sweep) / 0.08));
    for (let k = 1; k < n; k++) out.push([cen[0] + r * Math.cos(a0 + (sweep * k) / n), cen[1] + r * Math.sin(a0 + (sweep * k) / n)]);
    out.push(c);
  }
  return out;
}

function segDist(p: P, a: P, b: P) {
  const ab = [b[0] - a[0], b[1] - a[1]];
  const l2 = ab[0] * ab[0] + ab[1] * ab[1];
  const t = l2 < 1e-18 ? 0 : Math.max(0, Math.min(1, ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2));
  return { d: Math.hypot(p[0] - a[0] - ab[0] * t, p[1] - a[1] - ab[1] * t), q: [a[0] + ab[0] * t, a[1] + ab[1] * t] as P };
}

function stored<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key) as T | null;
    return v && allowed.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

function store(key: string, v: string) {
  try {
    localStorage.setItem(key, v);
  } catch {
    /* private window */
  }
}

function pts(list: P[]) {
  return list.map((p) => `${p[0]},${p[1]}`).join(" ");
}

function contourPath(contours: P[][]) {
  return contours.map((c) => `M${c.map((p) => `${p[0]} ${p[1]}`).join("L")}Z`).join("");
}

/** A rounded convex: a polygon swept by a disc, drawn exactly as fill plus a round-joined stroke. */
function PrimShape(props: { p: Prim; color: string; grow?: number; opacity?: number }) {
  const r = props.p.r + (props.grow ?? 0);
  const ps = props.p.pts;
  if (ps.length === 1) return <circle cx={ps[0][0]} cy={ps[0][1]} r={r} fill={props.color} opacity={props.opacity} />;
  if (ps.length === 2) return <line x1={ps[0][0]} y1={ps[0][1]} x2={ps[1][0]} y2={ps[1][1]} stroke={props.color} strokeWidth={2 * r} strokeLinecap="round" opacity={props.opacity} />;
  return <polygon points={pts(ps)} fill={props.color} stroke={r > 0 ? props.color : "none"} strokeWidth={2 * r} strokeLinejoin="round" opacity={props.opacity} />;
}

function HoleShape({ h }: { h: Hole }) {
  const [w, l] = h.size;
  if (Math.abs(w - l) < 1e-6) return <circle cx={h.at[0]} cy={h.at[1]} r={w / 2} className="pcb-drill" />;
  const half = (Math.max(w, l) - Math.min(w, l)) / 2;
  const a = (h.rotation * Math.PI) / 180;
  const d: P = w > l ? [half * Math.cos(a), half * Math.sin(a)] : [-half * Math.sin(a), half * Math.cos(a)];
  return <line x1={h.at[0] - d[0]} y1={h.at[1] - d[1]} x2={h.at[0] + d[0]} y2={h.at[1] + d[1]} className="pcb-drill-slot" strokeWidth={Math.min(w, l)} />;
}

/** A board graphic: its filled regions and its strokes. */
function GraphicShape(props: { g: GraphicV; color: string; opacity: number }) {
  const { g } = props;
  return (
    <g opacity={props.opacity} className={`pcb-gfx pcb-gfx-${g.kind}`}>
      {g.fills.length > 0 && <path d={g.fills.map((island) => contourPath(island)).join("")} fill={props.color} fillRule="evenodd" stroke="none" />}
      {g.strokes.map((st, i) => (
        <polyline key={i} points={pts(st)} fill="none" stroke={props.color} strokeWidth={g.width} strokeLinecap="round" strokeLinejoin="round" />
      ))}
    </g>
  );
}

export function PcbWorkspace(props: { revision: number; run: Run; notify: (m: string) => void; onClose: () => void; onShow3D: (model: string) => void; canEdit: boolean }) {
  const { run, notify } = props;
  const [boards, setBoards] = useState<BoardEntry[] | null>(null);
  const [boardId, setBoardId] = useState<string | null>(null);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("parts");
  const [layers, setLayers] = useState<Record<LayerKey, boolean>>({ top: true, bottom: true, inner: true, silk: true, courtyard: false, ratsnest: true, zones: true, drc: true, drawings: true, areas: true });
  const [fromBelow, setFromBelow] = useState(false);
  const [active, setActive] = useState<string>("top");
  // Tracks and vias picked together ("select connection", U).
  const [group, setGroup] = useState<string[]>([]);
  // While the router is drawing, its keys are its own.
  const routingFlag = useRef(false);
  const boardKeys = useRef<((e: KeyboardEvent) => boolean) | null>(null);
  const [sel, setSel] = useState<Selection>(null);
  const [net, setNet] = useState<string | null>(null);
  const [focusViolation, setFocusViolation] = useState<number | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [gerbers, setGerbers] = useState(false);
  // Parts and nets hidden from the board drawing (view only, not saved).
  const [hiddenParts, setHiddenParts] = useState<string[]>([]);
  const [hiddenNets, setHiddenNets] = useState<string[]>([]);
  const toggle = (list: string[], x: string) => (list.includes(x) ? list.filter((y) => y !== x) : [...list, x]);
  // Schematic, board, or both side by side; keys go to the pane last clicked.
  const [pane, setPane] = useState<"schematic" | "split" | "board" | "library">(() => {
    try {
      const v = localStorage.getItem("forma.pcb.pane");
      return v === "schematic" || v === "split" || v === "library" ? v : "board";
    } catch {
      return "board";
    }
  });
  const schKeys = useRef<((e: KeyboardEvent) => boolean) | null>(null);
  const keyPane = useRef<"schematic" | "board" | "library">("board");
  useEffect(() => {
    try {
      localStorage.setItem("forma.pcb.pane", pane);
    } catch {
      /* private window */
    }
    keyPane.current = pane === "board" ? "board" : pane === "library" ? "library" : "schematic";
  }, [pane]);

  // Boards in the project, then the chosen board in full, again after every change.
  useEffect(() => {
    let live = true;
    fetch(projectUrl("/pcb/boards"))
      .then((r) => r.json())
      .then((list: BoardEntry[]) => {
        if (!live) return;
        setBoards(list);
        setBoardId((b) => (b && list.some((x) => x.id === b) ? b : (list[0]?.id ?? null)));
      })
      .catch(() => live && setBoards([]));
    return () => {
      live = false;
    };
  }, [props.revision]);
  useEffect(() => {
    if (!boardId) {
      setDetail(null);
      return;
    }
    let live = true;
    fetch(projectUrl(`/pcb/${boardId}`))
      .then(async (r) => {
        if (!r.ok) throw new Error(await r.text());
        return r.json();
      })
      .then((d: Detail) => {
        if (!live) return;
        setDetail(d);
        setError(null);
      })
      .catch((e) => live && setError(String(e.message ?? e)));
    return () => {
      live = false;
    };
  }, [boardId, props.revision]);

  const b = detail?.board ?? null;
  // What the board view draws: everything but the hidden parts and nets.
  const shown = useMemo(() => {
    if (!b || (!hiddenParts.length && !hiddenNets.length)) return b;
    const parts = new Set(hiddenParts);
    const nets = new Set(hiddenNets);
    const netOk = (n: string | null) => !n || !nets.has(n);
    return {
      ...b,
      components: b.components.filter((c) => !parts.has(c.id)),
      pads: b.pads.filter((p) => !parts.has(p.component) && netOk(p.net)),
      tracks: b.tracks.filter((t) => netOk(t.net)),
      vias: b.vias.filter((v) => netOk(v.net)),
      zones: b.zones.filter((z) => netOk(z.net)),
      ratsnest: b.ratsnest.filter((r) => netOk(r.net)),
      holes: b.holes.filter((h) => !b.components.some((c) => parts.has(c.id) && (h.owner === c.reference || h.owner.startsWith(`${c.reference}.`)))),
    };
  }, [b, hiddenParts, hiddenNets]);
  const commit = useCallback(
    async (operation: string, parameters: Record<string, unknown>, done?: string) => {
      const out = await run(operation, parameters);
      if (out && done) notify(done);
      return out;
    },
    [run, notify],
  );

  const selComp = sel?.kind === "component" ? (b?.components.find((c) => c.id === sel.id) ?? null) : null;

  // Keys while the workspace is open: they are the board's, not the 3D view's.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)) return;
      const k = e.key.toLowerCase();
      const handled = () => {
        e.preventDefault();
        e.stopImmediatePropagation();
      };
      // The schematic takes its own Ctrl/Cmd keys (copy, paste, find, select all).
      if (keyPane.current === "schematic" && schKeys.current && (e.metaKey || e.ctrlKey)) {
        if (schKeys.current(e)) return handled();
        return;
      }
      if (e.metaKey || e.ctrlKey) return;
      if (keyPane.current === "library") {
        // The library editors take their own input; only Esc closes the workspace.
        if (e.key === "Escape") {
          handled();
          return props.onClose();
        }
        if (k.length === 1 || e.key === "Delete" || e.key === "Backspace") e.stopImmediatePropagation();
        return;
      }
      if (keyPane.current === "schematic" && schKeys.current) {
        if (schKeys.current(e)) return handled();
        if (e.key === "Escape") {
          handled();
          return props.onClose();
        }
        if (k.length === 1 && !e.altKey && k !== "a" && k !== "l") e.stopImmediatePropagation();
        return;
      }
      if (boardKeys.current?.(e)) return handled();
      if (routingFlag.current) {
        if (k === "v" && b) {
          handled();
          setActive((a) => (a === "top" ? "bottom" : "top"));
        } else if ((e.key === "PageDown" || e.key === "PageUp") && b) {
          handled();
          setActive((a) => b.layers[(b.layers.indexOf(a) + (e.key === "PageDown" ? 1 : b.layers.length - 1)) % b.layers.length]);
        }
        return;
      }
      if (e.key === "Escape") {
        handled();
        if (sel || net || group.length) {
          setSel(null);
          setNet(null);
          setGroup([]);
        } else props.onClose();
      } else if (k === "r" && selComp) {
        handled();
        void commit("pcb.component.update", { target: selComp.id, rotation: (selComp.rotation + (e.shiftKey ? -90 : 90) + 360) % 360 });
      } else if (k === "f" && selComp) {
        handled();
        void commit("pcb.component.update", { target: selComp.id, side: selComp.side === "top" ? "bottom" : "top" }, `${selComp.reference} to the ${selComp.side === "top" ? "bottom" : "top"}`);
      } else if ((e.key === "Delete" || e.key === "Backspace") && group.length) {
        handled();
        void commit("pcb.delete", { targets: group }, `Deleted ${group.length} item${group.length > 1 ? "s" : ""}`);
        setGroup([]);
        setSel(null);
      } else if ((e.key === "Delete" || e.key === "Backspace") && sel) {
        handled();
        void commit("pcb.delete", { targets: [sel.id] }, "Deleted");
        setSel(null);
      } else if (k === "v" && b) {
        handled();
        setActive((a) => (a === "top" ? "bottom" : "top"));
      } else if ((e.key === "PageDown" || e.key === "PageUp") && b) {
        handled();
        setActive((a) => b.layers[(b.layers.indexOf(a) + (e.key === "PageDown" ? 1 : b.layers.length - 1)) % b.layers.length]);
      } else if (k.length === 1 && !e.altKey && k !== "a" && k !== "l") {
        // The 3D view's tool keys stay with it while the board is open.
        e.stopImmediatePropagation();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [sel, net, selComp, commit, props, b, group]);

  const autoroute = async () => {
    if (!b) return;
    setBusy("Routing…");
    const out = (await run("pcb.autoroute", { board: b.id })) as { output?: { routed: string[]; failed: { net: string; reason: string }[] } } | null;
    setBusy(null);
    const o = out?.output;
    if (o) notify(o.failed.length ? `Routed ${o.routed.length} nets; ${o.failed.length} left: ${o.failed.map((f) => f.net).join(", ")}` : `Routed ${o.routed.length} nets`);
  };

  const groundPour = async () => {
    if (!b) return;
    const gnd = b.nets.find((n) => /^(gnd|ground|vss)$/i.test(n.name));
    if (!gnd) return notify("No GND net to pour");
    const layer = b.layers[b.layers.length - 1];
    if (b.zones.some((z) => z.net === gnd.name && z.layer === layer)) return notify(`${gnd.name} is already poured on the ${layer}`);
    await commit("pcb.zone.create", { board: b.id, net: gnd.name, layer }, `Poured ${gnd.name} on the ${layer}`);
  };

  // The side panel (parts, nets, checks, fab, BOM, simulation): beside the board, the schematic, or both.
  const side = b ? (
          <aside className="pcb-side">
            <nav className="pcb-tabs" role="tablist">
              {(
                [
                  ["parts", "Parts", b.components.length],
                  ["nets", "Nets", b.nets.length],
                  ["checks", "Checks", detail!.errors || null],
                  ["setup", "Setup", null],
                  ["fab", "Fab", null],
                  ["bom", "BOM", null],
                  ["sim", "Simulate", null],
                  ["tools", "Tools", null],
                ] as [Tab, string, number | null][]
              ).map(([k, label, n]) => (
                <button key={k} type="button" role="tab" aria-selected={tab === k} className={tab === k ? "active" : ""} onClick={() => setTab(k)}>
                  {label}
                  {n !== null && <span className={`pcb-count ${k === "checks" ? "bad" : ""}`}>{n}</span>}
                </button>
              ))}
            </nav>
            <div className="pcb-side-body">
              {tab === "parts" && <PartsTab b={b} sel={sel} onSelect={(id) => setSel({ kind: "component", id })} commit={commit} hidden={hiddenParts} onHide={(id) => setHiddenParts((h) => toggle(h, id))} />}
              {tab === "nets" && (
                <>
                  <NetsTab b={b} net={net} onNet={(n) => setNet((x) => (x === n ? null : n))} hidden={hiddenNets} onHide={(n) => setHiddenNets((h) => toggle(h, n))} />
                  {b.signal && <SignalPanel boardId={b.id} classes={b.signal.net_classes} nets={b.nets} s={b.signal} violations={b.violations} commit={commit} canEdit={props.canEdit} onNet={(n) => setNet(n)} />}
                </>
              )}
              {tab === "checks" && (
                <ChecksTab
                  b={b}
                  focus={focusViolation}
                  onFocus={(i) => {
                    setFocusViolation(i);
                    const v = b.violations[i];
                    const c = v.objects.find((o) => b.components.some((x) => x.id === o));
                    if (c) setSel({ kind: "component", id: c });
                  }}
                  onAutoroute={autoroute}
                  onPour={groundPour}
                  busy={busy}
                  canEdit={props.canEdit}
                />
              )}
              {tab === "setup" && <SetupTab b={b} commit={commit} canEdit={props.canEdit} />}
              {tab === "fab" && <FabTab d={detail!} commit={commit} boardId={b.id} revision={props.revision} />}
              {tab === "bom" && <ShopTab d={detail!} commit={commit} />}
              {tab === "sim" && <SimulateTab boardId={b.id} revision={props.revision} />}
              {tab === "tools" && <ToolsTab boardId={b.id} revision={props.revision} fab={b.fab ?? null} />}
            </div>
          </aside>
  ) : null;

  if (boards === null) {
    return (
      <section className="pcb floating" aria-label="Electronics">
        <div className="pcb-empty">
          <div className="pcb-spinner" />
        </div>
      </section>
    );
  }

  return (
    <section className="pcb floating" aria-label="Electronics: PCB">
      <header className="pcb-head">
        <div className="pcb-title">
          <span className="pcb-badge">
            <Icon name="chip" />
          </span>
          {boards.length > 1 ? (
            <select className="pcb-board-pick" value={boardId ?? ""} onChange={(e) => setBoardId(e.target.value)} aria-label="Board">
              {boards.map((x) => (
                <option key={x.id} value={x.id}>
                  {x.name}
                </option>
              ))}
            </select>
          ) : (
            <h2>{b?.name ?? "Electronics"}</h2>
          )}
          {b && (
            <span className="pcb-specs">
              {b.layers.length} layers · {FMT(detail!.size[0], 1)} × {FMT(detail!.size[1], 1)} mm · {b.thickness} mm · {b.finish} · {b.mask} mask · rev {b.revision}
            </span>
          )}
        </div>
        <div className="pcb-head-actions">
          {b && (
            <div className="pcb-pane-switch" role="tablist" aria-label="View">
              {(
                [
                  ["schematic", "Schematic"],
                  ["split", "Both"],
                  ["board", "Board"],
                  ["library", "Library"],
                ] as const
              ).map(([k, label]) => (
                <button key={k} type="button" role="tab" aria-selected={pane === k} className={pane === k ? "active" : ""} onClick={() => setPane(k)}>
                  {label}
                </button>
              ))}
            </div>
          )}
          {b && (
            <button type="button" className={`pcb-drc-pill ${detail!.errors ? "bad" : "good"}`} onClick={() => setTab("checks")} title="Design-rule check">
              <Icon name={detail!.errors ? "alert" : "check"} />
              {detail!.errors ? `${detail!.errors} DRC ${detail!.errors === 1 ? "error" : "errors"}` : "DRC passes"}
            </button>
          )}
          <button type="button" className="secondary pcb-tools-open" onClick={() => setGerbers(true)} title="Gerber viewer: open any fab files, compare revisions">
            <Icon name="layers" /> Gerbers
          </button>
          {b && <KicadImport run={commit} canEdit={props.canEdit} onMade={(id) => setBoardId(id)} label="Import" />}
          {b?.model && (
            <button type="button" className="secondary pcb-3d" onClick={() => props.onShow3D(b.model!)}>
              <Icon name="cube" /> Show in 3D
            </button>
          )}
          <IconButton icon="close" label="Close (Esc)" onClick={props.onClose} />
        </div>
      </header>
      {!b && boards.length === 0 ? (
        <NewBoard run={commit} canEdit={props.canEdit} onMade={(id) => setBoardId(id)} />
      ) : !b ? (
        <div className="pcb-empty">{error ? <p className="pcb-error">{error}</p> : <div className="pcb-spinner" />}</div>
      ) : pane === "library" ? (
        <div className="pcb-body">
          <LibraryPane boardId={b.id} revision={props.revision} commit={commit} notify={notify} canEdit={props.canEdit} />
        </div>
      ) : pane !== "board" ? (
        <div className={`pcb-body ${pane === "split" ? "split" : ""}`}>
          {side}
          <div className="pcb-sch-host" onPointerDownCapture={() => (keyPane.current = "schematic")}>
            <SchematicPane
              boardId={b.id}
              boardRefs={b.components.map((c) => ({ id: c.id, reference: c.reference }))}
              revision={props.revision}
              commit={commit}
              notify={notify}
              canEdit={props.canEdit}
              probeRef={selComp?.reference ?? null}
              onProbe={(r) => {
                const c = r ? b.components.find((x) => x.reference === r) : null;
                setSel(c ? { kind: "component", id: c.id } : null);
              }}
              net={net}
              onNet={setNet}
              keys={schKeys}
            />
          </div>
          {pane === "split" && (
            <div className="pcb-split-board" onPointerDownCapture={() => (keyPane.current = "board")}>
              <BoardCanvas
                b={shown!}
                layers={layers}
                onLayers={setLayers}
                fromBelow={fromBelow}
                onFromBelow={setFromBelow}
                active={active}
                onActive={setActive}
                sel={sel}
                onSel={setSel}
                group={group}
                onGroup={setGroup}
                routingFlag={routingFlag}
                keys={boardKeys}
                notify={props.notify}
                net={net}
                onNet={setNet}
                focus={null}
                commit={commit}
                canEdit={props.canEdit}
                onAutoroute={autoroute}
                busy={busy}
              />
            </div>
          )}
        </div>
      ) : (
        <div className="pcb-body">
          {side}
          <BoardCanvas
            b={shown!}
            layers={layers}
            onLayers={setLayers}
            fromBelow={fromBelow}
            onFromBelow={setFromBelow}
            active={active}
            onActive={setActive}
            sel={sel}
            onSel={setSel}
            group={group}
            onGroup={setGroup}
            routingFlag={routingFlag}
            keys={boardKeys}
            notify={props.notify}
            net={net}
            onNet={setNet}
            focus={focusViolation !== null ? (b.violations[focusViolation] ?? null) : null}
            commit={commit}
            canEdit={props.canEdit}
            onAutoroute={autoroute}
            busy={busy}
          />
        </div>
      )}
      {gerbers && <GerberViewer boardId={b?.id ?? null} boardName={b?.name} onClose={() => setGerbers(false)} />}
    </section>
  );
}

/** Files other EDA tools write that the engine reads (`pcb.import` tells them apart by content). */
export const EDA_FILES = ".kicad_pcb,.kicad_sch,.kicad_pro,.zip,.brd,.sch,.json,.epro,.epcb,.PcbDoc,.SchDoc,.pcbdoc,.schdoc,.cpa,.fab,.txt,.asc,.pcb";
/** Extensions that are surely an EDA file (dropped on the view, they import as one). */
export const EDA_DROP = /\.(kicad_pcb|kicad_sch|brd|sch|pcbdoc|schdoc|epro|epcb|cpa|asc|pcb)$/i;
const BINARY = /\.(pcbdoc|schdoc|epro|zip)$/i;

/** A board and/or schematic from another EDA tool (KiCad, Eagle, EasyEDA, Altium, CADSTAR, Fabmaster,
 * PADS, gEDA), read in the browser and made into Forma's by the engine. */
export async function importEdaFile(file: File, run: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>) {
  const name = file.name.replace(/\.[^.]+$/, "").replace(/[_-]+/g, " ");
  const params: Record<string, unknown> = { file: file.name, name };
  if (BINARY.test(file.name)) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let bin = "";
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    params.data = btoa(bin);
  } else {
    params.text = await file.text();
  }
  const out = (await run("pcb.import", params, `Imported ${file.name}`)) as { output?: { id: string; board?: { id: string } | null; schematic?: { id: string } | null; notes?: string[]; format?: string } } | null;
  const o = out?.output;
  if (!o) return null;
  return { ...o, id: o.board?.id ?? o.id };
}

/** Kept for callers of the KiCad-only import. */
export const importKicadFile = importEdaFile;

function KicadImport(props: { run: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>; canEdit: boolean; onMade: (id: string) => void; label?: string }) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <button type="button" className="secondary" disabled={!props.canEdit} onClick={() => input.current?.click()} title="Open a board or schematic from KiCad, Eagle, EasyEDA (Standard or Pro), Altium (PcbDoc/SchDoc), CADSTAR, Allegro Fabmaster, PADS or gEDA">
        <Icon name="download" /> {props.label ?? "Import board or schematic"}
      </button>
      <input
        ref={input}
        type="file"
        accept={EDA_FILES}
        style={{ display: "none" }}
        onChange={async (e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (!f) return;
          const made = await importEdaFile(f, props.run);
          if (made?.board?.id) props.onMade(made.board.id);
        }}
      />
    </>
  );
}

/** Files for other EDA tools, from the engine's list of what it writes. */
function InterchangeFiles(props: { boardId: string; revision: number }) {
  const [list, setList] = useState<{ name: string; what: string }[] | null>(null);
  useEffect(() => {
    let live = true;
    fetch(projectUrl(`/pcb/${props.boardId}/exports`))
      .then((r) => (r.ok ? r.json() : null))
      .then((j: { files: { name: string; what: string }[] } | null) => live && setList(j?.files ?? []))
      .catch(() => live && setList([]));
    return () => {
      live = false;
    };
  }, [props.boardId, props.revision]);
  if (!list) return <p className="pcb-note">Listing files for other tools…</p>;
  const url = (name: string, extra = "") => projectUrl(`/pcb/${props.boardId}/file/${encodeURIComponent(name)}${extra}`);
  return (
    <div className="pcb-files pcb-interchange">
      {list.map((f) => (
        <span key={f.name} className="pcb-file" title={f.what}>
          <a href={url(f.name)} target="_blank" rel="noreferrer">
            {f.name}
          </a>
          <small>{f.what}</small>
          <a href={url(f.name, "?download=1")} aria-label={`Download ${f.name}`}>
            <Icon name="download" />
          </a>
        </span>
      ))}
    </div>
  );
}

function NewBoard(props: { run: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>; canEdit: boolean; onMade: (id: string) => void }) {
  const [name, setName] = useState("Main board");
  const [w, setW] = useState(50);
  const [h, setH] = useState(40);
  const [layers, setLayers] = useState(2);
  return (
    <div className="pcb-empty">
      <div className="pcb-new">
        <span className="pcb-new-art" aria-hidden="true">
          <Icon name="chip" />
        </span>
        <h3>No board yet</h3>
        <p>Ask Forma AI to design one (“a USB-C powered ESP32 board with a status LED and an I²C header”), or start an empty board and place parts.</p>
        <form
          className="pcb-new-form"
          onSubmit={async (e) => {
            e.preventDefault();
            const out = (await props.run("pcb.board.create", { name, width: w, height: h, corner_radius: 1.5, layers }, `Made ${name}`)) as { output?: { id: string } } | null;
            if (out?.output?.id) props.onMade(out.output.id);
          }}
        >
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <label>
            Width
            <input type="number" min={5} max={500} step={0.5} value={w} onChange={(e) => setW(+e.target.value)} />
            <i>mm</i>
          </label>
          <label>
            Height
            <input type="number" min={5} max={500} step={0.5} value={h} onChange={(e) => setH(+e.target.value)} />
            <i>mm</i>
          </label>
          <label>
            Layers
            <select value={layers} onChange={(e) => setLayers(+e.target.value)}>
              {[2, 4, 6, 8, 10, 12, 14, 16, 20, 24, 32].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" className="primary" disabled={!props.canEdit}>
            New board
          </button>
        </form>
        <div className="pcb-new-import">
          <span>or</span>
          <KicadImport run={props.run} canEdit={props.canEdit} onMade={props.onMade} />
        </div>
      </div>
    </div>
  );
}

// ---- The board, drawn ------------------------------------------------------------

type ViewT = { s: number; tx: number; ty: number };

function BoardCanvas(props: {
  b: BoardView;
  layers: Record<LayerKey, boolean>;
  onLayers: (l: Record<LayerKey, boolean>) => void;
  fromBelow: boolean;
  onFromBelow: (v: boolean) => void;
  active: string;
  onActive: (a: string) => void;
  sel: Selection;
  onSel: (s: Selection) => void;
  group: string[];
  onGroup: (g: string[]) => void;
  routingFlag: React.MutableRefObject<boolean>;
  keys: React.MutableRefObject<((e: KeyboardEvent) => boolean) | null>;
  notify: (m: string) => void;
  net: string | null;
  onNet: (n: string | null) => void;
  focus: Violation | null;
  commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>;
  canEdit: boolean;
  onAutoroute: () => void;
  busy: string | null;
}) {
  const { b, layers, sel, net } = props;
  const host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<P>([800, 600]);
  const [view, setView] = useState<ViewT | null>(null);
  const [hover, setHover] = useState<{ x: number; y: number; text: string; sub?: string } | null>(null);
  const [drag, setDrag] = useState<{ id: string; start: P; at: P; delta: P; moved: boolean } | null>(null);
  const pan = useRef<{ x: number; y: number; tx: number; ty: number } | null>(null);
  const [routing, setRouting] = useState(false);
  const [routerMode, setRouterMode] = useState<RouterMode>(() => stored("forma.pcb.router", ROUTER_MODES, "walkaround"));
  const [angle, setAngle] = useState<AngleMode>(() => stored("forma.pcb.angle", ANGLES, "45"));
  const [arcCorners, setArcCorners] = useState(false);
  const [width, setWidth] = useState<number | null>(null);
  // "/" flips which way a leg bends (diagonal first or straight first).
  const [posture, setPosture] = useState(false);
  // The route being laid: where it starts, the clicks so far, and the router's plan to the cursor.
  const [start, setStart] = useState<{ at: P; layer: string } | null>(null);
  const [steps, setSteps] = useState<RouteStep[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  // A track segment, corner or arc, or a via, being dragged.
  const [itemDrag, setItemDrag] = useState<ItemDrag | null>(null);
  const lastDrag = useRef<{ to: P; free: boolean } | null>(null);
  const mouse = useRef<P | null>(null);
  const routeLayer = useRef(props.active);
  const [cursor, setCursor] = useState<P | null>(null);
  // Differential pairs routed together (D while routing): the pair, its two start pads, the clicks, the plan.
  const [pairMode, setPairMode] = useState(false);
  const [pairStart, setPairStart] = useState<{ pair: string; from: [string, string] } | null>(null);
  const [pairSteps, setPairSteps] = useState<{ at: P; layer: string }[]>([]);
  const [pairPlan, setPairPlan] = useState<PairPlan | null>(null);
  // Length tuning on a track (T): the track and the engine's meanders for the settings.
  const [tune, setTune] = useState<{ track: string; net: string } | null>(null);
  const [tunePlan, setTunePlan] = useState<TunePlan | null>(null);
  const gen2 = useRef(0);
  const askJson = useCallback(
    <T,>(body: Record<string, unknown>, set: (v: T | null) => void) => {
      const g = ++gen2.current;
      fetch(projectUrl(`/pcb/${b.id}/router`), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })
        .then(async (r) => (r.ok ? ((await r.json()) as T) : null))
        .then((v) => {
          if (g === gen2.current) set(v);
        })
        .catch(() => undefined);
    },
    [b.id],
  );
  useEffect(() => {
    props.routingFlag.current = routing;
    return () => {
      props.routingFlag.current = false;
    };
  }, [routing, props.routingFlag]);
  useEffect(() => store("forma.pcb.router", routerMode), [routerMode]);
  useEffect(() => store("forma.pcb.angle", angle), [angle]);

  // The router's plan for each cursor position: one request at a time, the latest wins; nothing is changed until a click.
  const gen = useRef(0);
  const asking = useRef(false);
  const nextAsk = useRef<{ body: Record<string, unknown>; g: number } | null>(null);
  const ask = useCallback(
    (body: Record<string, unknown>) => {
      const g = gen.current;
      if (asking.current) {
        nextAsk.current = { body, g };
        return;
      }
      asking.current = true;
      fetch(projectUrl(`/pcb/${b.id}/router`), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })
        .then(async (r) => (r.ok ? ((await r.json()) as Plan) : null))
        .then((p) => {
          if (g === gen.current) setPlan(p);
        })
        .catch(() => undefined)
        .finally(() => {
          asking.current = false;
          const n = nextAsk.current;
          nextAsk.current = null;
          if (n && n.g === gen.current) ask(n.body);
        });
    },
    [b.id],
  );
  const dropPlan = () => {
    gen.current++;
    nextAsk.current = null;
    setPlan(null);
  };

  const fitted = useRef<string | null>(null);

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize([el.clientWidth, el.clientHeight]));
    ro.observe(el);
    setSize([el.clientWidth, el.clientHeight]);
    return () => ro.disconnect();
  }, []);

  const fit = useCallback(() => {
    // The board and anything hanging over its edge (a module's antenna, a connector's mouth).
    let [x0, y0, x1, y1] = b.bounds;
    for (const c of b.components) for (const p of [...c.courtyard, [c.label[0], c.label[1] + 1] as P]) [x0, y0, x1, y1] = [Math.min(x0, p[0]), Math.min(y0, p[1]), Math.max(x1, p[0]), Math.max(y1, p[1])];
    const pad = 36;
    const top = 56;
    const bottom = 40;
    const s = Math.min((size[0] - 2 * pad) / Math.max(x1 - x0, 1), (size[1] - top - bottom) / Math.max(y1 - y0, 1));
    const cx = (x0 + x1) / 2;
    const cy = (y0 + y1) / 2;
    setView({ s, tx: size[0] / 2 - (props.fromBelow ? -1 : 1) * cx * s, ty: top + (size[1] - top - bottom) / 2 + cy * s });
  }, [b.bounds, b.components, size, props.fromBelow]);

  useEffect(() => {
    const key = `${b.id}:${size[0]}x${size[1]}:${props.fromBelow}`;
    if (fitted.current !== key && size[0] > 50) {
      fitted.current = key;
      fit();
    }
  }, [b.id, size, fit, props.fromBelow]);

  // Glide to a violation picked in the list.
  useEffect(() => {
    if (!props.focus || !view) return;
    const s = Math.max(view.s, Math.min(size[0], size[1]) / 14);
    const [x, y] = props.focus.at;
    setView({ s, tx: size[0] / 2 - (props.fromBelow ? -1 : 1) * x * s, ty: size[1] / 2 + y * s });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.focus]);

  const sx = props.fromBelow ? -1 : 1;
  const toBoard = (e: { clientX: number; clientY: number }): P => {
    const r = host.current!.getBoundingClientRect();
    const v = view!;
    return [(e.clientX - r.left - v.tx) / (sx * v.s), -(e.clientY - r.top - v.ty) / v.s];
  };
  const snap = (p: P, fine: boolean): P => {
    const g = fine ? 0.05 : 0.25;
    return [Math.round(p[0] / g) * g, Math.round(p[1] / g) * g];
  };

  const onWheel = (e: React.WheelEvent) => {
    if (!view) return;
    const r = host.current!.getBoundingClientRect();
    const mx = e.clientX - r.left;
    const my = e.clientY - r.top;
    const k = Math.exp(-e.deltaY * 0.0015);
    const s = Math.min(Math.max(view.s * k, 0.5), 400);
    const f = s / view.s;
    setView({ s, tx: mx - (mx - view.tx) * f, ty: my - (my - view.ty) * f });
  };

  // The nearest pad under the point (fine-pitch neighbours overlap their reach).
  const padAt = (p: P) => {
    let best: Pad | undefined;
    let bd = Infinity;
    for (const q of b.pads) {
      const d = Math.hypot(q.at[0] - p[0], q.at[1] - p[1]);
      if (q.kind !== "npth" && d < Math.max(q.size[0], q.size[1]) / 2 && d < bd) [best, bd] = [q, d];
    }
    return best;
  };

  const head = (at: P | string): RouteStep => ({ at, layer: props.active, diagonal_first: posture });
  const routeSpec = (from: { at: P; layer: string }, to: RouteStep[]) => ({ board: b.id, from: from.at, layer: from.layer, to, mode: routerMode, angle, corners: arcCorners ? "arc" : "sharp", ...(width ? { width } : {}) });
  const dragSpec = (d: ItemDrag, to: P, free: boolean) => ({ board: b.id, target: d.id, kind: d.kind, index: d.index, from: d.from, to, mode: routerMode === "highlight" ? "highlight" : "shove", angle: free || angle === "free" ? "free" : "45" });

  // A route starts on a pad, a via, a track's end or a point on a track (its net, width and layer), or anywhere.
  const startFrom = (raw: P, alt: boolean): { at: P; layer: string } => {
    const pad = padAt(raw);
    if (pad) return { at: pad.at, layer: pad.kind === "smd" ? pad.layers[0] : props.active };
    const via = b.vias.find((v) => Math.hypot(v.at[0] - raw[0], v.at[1] - raw[1]) <= v.diameter / 2);
    if (via) return { at: via.at, layer: props.active };
    const reach = Math.max(0.2, 6 / (view?.s ?? 10));
    let best: { d: number; at: P; layer: string } | null = null;
    for (const t of b.tracks) {
      if (!showLayer(t.layer)) continue;
      for (const end of [t.points[0], t.points[t.points.length - 1]]) {
        const d = Math.hypot(end[0] - raw[0], end[1] - raw[1]);
        if (d <= Math.max(t.width, reach) && (!best || d < best.d)) best = { d, at: end, layer: t.layer };
      }
    }
    if (!best)
      for (const t of b.tracks) {
        if (!showLayer(t.layer)) continue;
        for (let i = 0; i + 1 < t.points.length; i++) {
          const { d, q } = segDist(raw, t.points[i], t.points[i + 1]);
          if (d <= t.width / 2 && (!best || d < best.d)) best = { d, at: [+q[0].toFixed(4), +q[1].toFixed(4)], layer: t.layer };
        }
      }
    return best ? { at: best.at, layer: best.layer } : { at: snap(raw, alt), layer: props.active };
  };

  const resetRoute = () => {
    dropPlan();
    setStart(null);
    setSteps([]);
  };

  const finishRoute = async (all: RouteStep[]) => {
    const from = start;
    // Walkaround and shove never place copper that breaks a clearance (a via dropped too close, an end on another net).
    if (plan && !plan.ok && routerMode !== "highlight") {
      const c = plan.collisions[0];
      props.notify(`Not placed: ${c ? `too close to ${c.with}` : "it would break a clearance"} — move it, Backspace, or use Highlight mode to place it anyway`);
      return;
    }
    resetRoute();
    if (!from || !all.length) return;
    const out = (await props.commit("pcb.route.interactive", routeSpec(from, all))) as { output?: { tracks: string[]; vias: string[]; ok: boolean; reached: boolean; pushed_tracks: number; pushed_vias: number; collisions: Collision[] } } | null;
    const o = out?.output;
    if (!o) return;
    const parts = [`${o.tracks.length} track${o.tracks.length === 1 ? "" : "s"}`];
    if (o.vias.length) parts.push(`${o.vias.length} via${o.vias.length === 1 ? "" : "s"}`);
    if (o.pushed_tracks || o.pushed_vias) parts.push(`pushed ${o.pushed_tracks + o.pushed_vias}`);
    props.notify(`Routed: ${parts.join(", ")}${o.collisions.length ? ` · ${o.collisions.length} clearance problem${o.collisions.length === 1 ? "" : "s"}` : ""}`);
  };

  const resetPair = () => {
    gen2.current++;
    setPairStart(null);
    setPairSteps([]);
    setPairPlan(null);
  };
  const pairSpec = (ps: { pair: string; from: [string, string] }, stepsNow: { at: P; layer: string }[]) => ({ board: b.id, pair: ps.pair, from: ps.from, steps: stepsNow, mode: routerMode === "shove" ? "shove" : "walkaround", angle: angle === "free" ? "45" : angle, ...(arcCorners ? { corners: "arc" } : {}) });
  // A pair starts on either pad of it: both of its pads on that part are the start.
  const startPair = (raw: P) => {
    const pad = padAt(raw);
    const pair = pad && b.signal?.pairs.find((p) => p.p === pad.net || p.n === pad.net);
    if (!pad || !pair) {
      props.notify("Click a pad of a differential pair (nets D+/D−, _P/_N, or set in Nets › Differential pairs)");
      return;
    }
    const near = (net: string) =>
      net === pad.net
        ? pad
        : b.pads.filter((q) => q.component === pad.component && q.net === net).sort((a, c) => Math.hypot(a.at[0] - pad.at[0], a.at[1] - pad.at[1]) - Math.hypot(c.at[0] - pad.at[0], c.at[1] - pad.at[1]))[0];
    const pp = near(pair.p);
    const pn = near(pair.n);
    if (!pp || !pn) {
      props.notify(`${pair.name}: its other pad is not on ${pad.reference}`);
      return;
    }
    setPairStart({ pair: pair.name, from: [`${pp.reference}.${pp.number}`, `${pn.reference}.${pn.number}`] });
    setPairSteps([]);
    if (pad.kind === "smd" && pad.layers[0] !== props.active) props.onActive(pad.layers[0]);
  };
  const finishPair = async (stepsNow: { at: P; layer: string }[]) => {
    const ps = pairStart;
    if (!ps) return;
    if (pairPlan && !pairPlan.ok && routerMode !== "highlight") {
      const c = pairPlan.p.collisions[0];
      props.notify(`Not placed: ${c ? `too close to ${c.with}` : pairPlan.notes[0] ?? "it would break a clearance"} — add a corner, or Backspace`);
      return;
    }
    resetPair();
    const out = (await props.commit("pcb.route.pair", pairSpec(ps, stepsNow))) as { output?: { pair: string; length_p: number; length_n: number; skew: number; collisions: Collision[] } } | null;
    const o = out?.output;
    if (o) props.notify(`Routed pair ${o.pair}: ${FMT(o.length_p, 3)} / ${FMT(o.length_n, 3)} mm, skew ${FMT(o.skew, 4)} mm${o.collisions.length ? ` · ${o.collisions.length} clearance problems` : ""}`);
  };

  const onDown = (e: ReactPointerEvent) => {
    if (!view) return;
    if (routing && pairMode && e.button === 0) {
      const raw = toBoard(e);
      if (!pairStart) return startPair(raw);
      if (e.detail >= 2) return void finishPair(pairSteps);
      setPairSteps((l) => [...l, { at: snap(raw, e.altKey), layer: props.active }]);
      return;
    }
    if (routing && e.button === 0) {
      const raw = toBoard(e);
      if (!start) {
        const s0 = startFrom(raw, e.altKey);
        setStart(s0);
        setSteps([]);
        if (s0.layer !== props.active) {
          routeLayer.current = s0.layer;
          props.onActive(s0.layer);
        }
        return;
      }
      if (e.detail >= 2) return void finishRoute(steps);
      const pad = padAt(raw);
      const p = pad ? pad.at : snap(raw, e.altKey);
      // A click on a pad of the net (not the one it started from) finishes the route there.
      const from = padAt(start.at);
      if (pad && plan?.net && pad.net === plan.net && pad !== from && plan.reached) return void finishRoute([...steps, head(pad.at)]);
      setSteps((list) => [...list, head(plan && !plan.reached ? plan.end : p)]);
      return;
    }
    (e.target as Element).setPointerCapture?.(e.pointerId);
    pan.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty };
  };

  // Switching layer (V, PageUp/PageDown) mid-route: the line so far ends at the head, and the next leg starts there through a via.
  useEffect(() => {
    const was = routeLayer.current;
    routeLayer.current = props.active;
    if (!routing || !start || was === props.active) return;
    if (plan) setSteps((list) => [...list, { at: plan.end, layer: was, diagonal_first: posture }]);
    else if (!steps.length) setStart({ ...start, layer: props.active });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.active]);

  // Ask again when a setting or the clicks change.
  useEffect(() => {
    if (routing && start && cursor) ask({ op: "route", ...routeSpec(start, [...steps, head(cursor)]) });
    else if (!start) dropPlan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [posture, routerMode, angle, arcCorners, width, steps, start, props.active, routing]);

  const selTrackNow = sel?.kind === "track" ? b.tracks.find((t) => t.id === sel.id) : null;
  const selectConnection = async () => {
    if (!sel || (sel.kind !== "track" && sel.kind !== "via")) return;
    const r = await fetch(projectUrl(`/pcb/${b.id}/router`), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ op: "connection", targets: [sel.id] }) });
    if (!r.ok) return;
    const { items } = (await r.json()) as { items: string[] };
    props.onGroup(items);
    props.notify(`${items.length} track${items.length === 1 ? "" : "s"} and vias in this connection · Del removes them`);
  };

  const cleanup = async () => {
    const out = (await props.commit("pcb.route.cleanup", { board: b.id })) as { output?: Record<string, number> } | null;
    const o = out?.output;
    if (!o) return;
    const said = [
      o.points_merged && `${o.points_merged} corners merged`,
      o.joined && `${o.joined} tracks joined`,
      o.trimmed && `${o.trimmed} stubs trimmed`,
      o.duplicates && `${o.duplicates} duplicates`,
      o.redundant_vias && `${o.redundant_vias} vias removed`,
    ].filter(Boolean);
    props.notify(said.length ? `Cleaned up: ${said.join(", ")}` : "Nothing to clean up");
  };

  // Keys over the board (the workspace asks these first).
  const keyHandler = (e: KeyboardEvent): boolean => {
    const k = e.key.toLowerCase();
    if (routing && pairMode) {
      if (e.key === "Enter") void finishPair(pairSteps);
      else if (e.key === "Escape") {
        if (pairStart) resetPair();
        else {
          setPairMode(false);
          setRouting(false);
        }
      } else if (e.key === "Backspace") {
        if (pairSteps.length) setPairSteps((l) => l.slice(0, -1));
        else resetPair();
      } else if (k === "d") {
        resetPair();
        setPairMode(false);
      } else if (k === "m") {
        setRouterMode((m) => ROUTER_MODES[(ROUTER_MODES.indexOf(m) + 1) % ROUTER_MODES.length]);
      } else return false;
      return true;
    }
    if (routing && k === "d") {
      resetRoute();
      setPairMode(true);
      return true;
    }
    if (!routing && k === "d" && props.canEdit) {
      setRouting(true);
      setPairMode(true);
      return true;
    }
    if (!routing && k === "t" && props.canEdit && selTrackNow?.net) {
      setTune({ track: selTrackNow.id, net: selTrackNow.net });
      setTunePlan(null);
      return true;
    }
    if (routing) {
      if (e.key === "Enter") {
        void finishRoute(start && cursor ? [...steps, head(plan && !plan.reached ? plan.end : cursor)] : steps);
      } else if (e.key === "Escape") {
        if (start) resetRoute();
        else setRouting(false);
      } else if (e.key === "Backspace") {
        if (steps.length) setSteps((l) => l.slice(0, -1));
        else resetRoute();
      } else if (e.key === "/") {
        setPosture((p) => !p);
      } else if (k === "f" && start && plan?.net) {
        // Finish to the nearest pad of the net.
        const lastAt = steps.length ? steps[steps.length - 1].at : start.at;
        const from: P = typeof lastAt === "string" ? start.at : lastAt;
        const target = b.pads
          .filter((q) => q.net === plan.net && q.kind !== "npth" && Math.hypot(q.at[0] - start.at[0], q.at[1] - start.at[1]) > 1e-6)
          .sort((a, c) => Math.hypot(a.at[0] - from[0], a.at[1] - from[1]) - Math.hypot(c.at[0] - from[0], c.at[1] - from[1]))[0];
        if (target) void finishRoute([...steps, head(`${target.reference}.${target.number}`)]);
      } else if (k === "w") {
        const now = width ?? plan?.width ?? 0.25;
        const next = e.shiftKey ? [...WIDTHS].reverse().find((w) => w < now - 1e-9) : WIDTHS.find((w) => w > now + 1e-9);
        if (next) setWidth(next);
      } else if (k === "m") {
        setRouterMode((m) => ROUTER_MODES[(ROUTER_MODES.indexOf(m) + 1) % ROUTER_MODES.length]);
      } else return false;
      return true;
    }
    if (k === "x" && props.canEdit) {
      setRouting(true);
      return true;
    }
    if (k === "u" && (sel?.kind === "track" || sel?.kind === "via")) {
      void selectConnection();
      return true;
    }
    if (k === "b" && selTrackNow && mouse.current && props.canEdit) {
      void props.commit("pcb.track.break", { target: selTrackNow.id, at: mouse.current }, "Track broken in two");
      return true;
    }
    return false;
  };
  useEffect(() => {
    props.keys.current = keyHandler;
  });
  useEffect(
    () => () => {
      props.keys.current = null;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const onMove = (e: ReactPointerEvent) => {
    if (!view) return;
    mouse.current = toBoard(e);
    if (routing) {
      const raw = toBoard(e);
      const pad = padAt(raw);
      const c = pad ? pad.at : snap(raw, e.altKey);
      setCursor(c);
      if (pairMode && pairStart) askJson<PairPlan>({ op: "pair", ...pairSpec(pairStart, [...pairSteps, { at: snap(raw, e.altKey), layer: props.active }]) }, setPairPlan);
      else if (start) ask({ op: "route", ...routeSpec(start, [...steps, head(c)]) });
    }
    if (itemDrag) {
      const to = snap(toBoard(e), e.altKey);
      const moved = itemDrag.moved || Math.hypot(e.clientX - itemDrag.client[0], e.clientY - itemDrag.client[1]) > 3;
      if (moved !== itemDrag.moved) setItemDrag({ ...itemDrag, moved });
      if (moved) {
        lastDrag.current = { to, free: e.altKey };
        ask({ op: "drag", ...dragSpec(itemDrag, to, e.altKey) });
      }
      return;
    }
    if (drag) {
      const p = toBoard(e);
      const d: P = [p[0] - drag.start[0], p[1] - drag.start[1]];
      const at = snap([drag.at[0] + d[0], drag.at[1] + d[1]], e.altKey);
      setDrag({ ...drag, delta: [at[0] - drag.at[0], at[1] - drag.at[1]], moved: drag.moved || Math.hypot(d[0], d[1]) * view.s > 3 });
      return;
    }
    if (pan.current) {
      setView({ ...view, tx: pan.current.tx + e.clientX - pan.current.x, ty: pan.current.ty + e.clientY - pan.current.y });
    }
  };

  const onUp = (e: ReactPointerEvent) => {
    if (itemDrag) {
      const d = itemDrag;
      const last = lastDrag.current;
      setItemDrag(null);
      lastDrag.current = null;
      const seen = plan;
      dropPlan();
      if (!d.moved || !last) return;
      if (seen && !seen.ok && routerMode !== "highlight") {
        props.notify(`Cannot drag there: ${seen.notes[0]?.replace(/^cannot shove: /, "") ?? "it would break a clearance"} (Highlight mode places it anyway)`);
        return;
      }
      void props.commit("pcb.route.drag", dragSpec(d, last.to, last.free));
      return;
    }
    if (drag) {
      const d = drag;
      setDrag(null);
      if (d.moved && (Math.abs(d.delta[0]) > 1e-9 || Math.abs(d.delta[1]) > 1e-9)) {
        void props.commit("pcb.component.update", { target: d.id, at: [+(d.at[0] + d.delta[0]).toFixed(4), +(d.at[1] + d.delta[1]).toFixed(4)] });
      }
    }
    if (pan.current) {
      const moved = Math.hypot(e.clientX - pan.current.x, e.clientY - pan.current.y) > 3;
      pan.current = null;
      if (!moved && !routing) {
        props.onSel(null);
        props.onNet(null);
        props.onGroup([]);
      }
    }
  };

  const onTrackDown = (e: ReactPointerEvent, t: Track) => {
    if (routing) return;
    e.stopPropagation();
    props.onSel({ kind: "track", id: t.id });
    props.onNet(t.net);
    if (!props.group.includes(t.id)) props.onGroup([]);
    grabTrack(e, t);
  };
  const onViaDown = (e: ReactPointerEvent, v: Via) => {
    if (routing) return;
    e.stopPropagation();
    props.onSel({ kind: "via", id: v.id });
    props.onNet(v.net);
    if (!props.group.includes(v.id)) props.onGroup([]);
    beginItemDrag(e, v.id, "via", 0);
  };

  // Tracks and vias are dragged by the router (45°, pushing neighbours; Alt: free angle).
  const beginItemDrag = (e: ReactPointerEvent, id: string, kind: ItemDrag["kind"], index: number) => {
    if (!props.canEdit || e.button !== 0 || !view) return;
    (e.target as Element).setPointerCapture?.(e.pointerId);
    setItemDrag({ id, kind, index, from: toBoard(e), client: [e.clientX, e.clientY], moved: false });
  };
  const grabTrack = (e: ReactPointerEvent, t: Track) => {
    const raw = toBoard(e);
    const verts = t.vertices?.length ? t.vertices : t.points;
    const arcs = !!t.bulges?.length;
    const tol = Math.max(t.width / 2, 5 / (view?.s ?? 10));
    const corner = arcs ? -1 : verts.findIndex((q) => Math.hypot(q[0] - raw[0], q[1] - raw[1]) <= tol);
    let seg = 0;
    let bd = Infinity;
    for (let i = 0; i + 1 < verts.length; i++) {
      const { d } = segDist(raw, verts[i], verts[i + 1]);
      if (d < bd) [bd, seg] = [d, i];
    }
    beginItemDrag(e, t.id, arcs ? "arc" : corner >= 0 ? "corner" : "segment", corner >= 0 ? corner : seg);
  };

  const startDrag = (e: ReactPointerEvent, c: Comp) => {
    if (routing || e.button !== 0) return;
    e.stopPropagation();
    props.onSel({ kind: "component", id: c.id });
    if (!props.canEdit) return;
    (e.target as Element).setPointerCapture?.(e.pointerId);
    setDrag({ id: c.id, start: toBoard(e), at: c.at, delta: [0, 0], moved: false });
  };

  const offsetOf = (component: string) => (drag && drag.id === component ? drag.delta : null);
  // While a part is dragged, the ends of tracks on its pads go with it (the engine bends them at 45° on drop).
  const dragPads = useMemo(() => (drag ? b.pads.filter((q) => q.component === drag.id && q.kind !== "npth") : []), [drag?.id, b.pads]);
  const trackPoints = (t: Track): P[] => {
    if (!drag || !dragPads.length) return t.points;
    const on = (p: P) => dragPads.some((q) => q.layers.includes(t.layer) && Math.hypot(q.at[0] - p[0], q.at[1] - p[1]) <= Math.max(q.size[0], q.size[1]) / 2 + 1e-6);
    const d = drag.delta;
    const n = t.points.length;
    const first = on(t.points[0]);
    const last = on(t.points[n - 1]);
    if (!first && !last) return t.points;
    return t.points.map((p, i) => ((first && last) || (first && i === 0) || (last && i === n - 1) ? [p[0] + d[0], p[1] + d[1]] : p));
  };
  const tf = (component: string) => {
    const o = offsetOf(component);
    return o ? `translate(${o[0]} ${o[1]})` : undefined;
  };

  const lit = (n: string | null | undefined) => !!net && n === net;
  const dimmed = !!net;
  const order: string[] = props.fromBelow ? [...b.layers] : [...b.layers].reverse();
  const isOuter = (l: string) => l === "top" || l === "bottom";
  const showLayer = (l: string) => (l === "top" ? layers.top : l === "bottom" ? layers.bottom : layers.inner);
  const silkSide = props.fromBelow ? "bottom" : "top";
  const maskColor = MASKS[b.mask.toLowerCase()] ?? MASKS.green;
  const selTrack = sel?.kind === "track" ? b.tracks.find((t) => t.id === sel.id) : null;
  const selVia = sel?.kind === "via" ? b.vias.find((t) => t.id === sel.id) : null;
  const errorsAt = useMemo(() => b.violations.filter((v) => v.code !== "unrouted"), [b.violations]);
  // What the router's plan moves is drawn where it would go, not where it is.
  const replaced = useMemo(
    () => new Set([...(plan?.tracks.map((t) => t.id) ?? []), ...(pairPlan?.p.tracks.map((t) => t.id) ?? []), ...(tune && tunePlan ? tunePlan.edits.map((t) => t.id) : [])]),
    [plan, pairPlan, tune, tunePlan],
  );
  const tunePair = tune ? b.signal?.pairs.find((p) => p.p === tune.net || p.n === tune.net) : undefined;
  const tuneGroup = tune ? b.signal?.groups.find((g) => g.members.some((m) => m.net === tune.net)) : undefined;
  const movedVias = useMemo(() => new Map((plan?.moved_vias ?? []).map((m) => [m.id, m.at] as [string, P])), [plan]);
  const groupSet = useMemo(() => new Set(props.group), [props.group]);
  const selComp = sel?.kind === "component" ? b.components.find((c) => c.id === sel.id) : null;
  const legNet = plan?.net ?? null;

  const hoverPad = (e: ReactPointerEvent, p: Pad) => {
    const r = host.current!.getBoundingClientRect();
    setHover({ x: e.clientX - r.left, y: e.clientY - r.top, text: `${p.reference}.${p.number || "hole"}${p.pin_name ? ` · ${p.pin_name}` : ""}`, sub: p.net ?? "no net" });
  };

  return (
    <div className={`pcb-canvas ${routing ? "routing" : ""}`} ref={host} onWheel={onWheel} onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerLeave={() => setHover(null)}>
      {view && (
        <svg width={size[0]} height={size[1]} className="pcb-svg">
          <defs>
            <pattern id="pcb-grid" width={10} height={10} patternUnits="userSpaceOnUse">
              <path d="M10 0H0V10" fill="none" stroke="#ffffff08" strokeWidth={0.06} />
            </pattern>
            <pattern id="pcb-keepout" width={1.2} height={1.2} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <line x1={0} y1={0} x2={0} y2={1.2} stroke="#ff6b6b" strokeWidth={0.12} strokeOpacity={0.45} />
            </pattern>
          </defs>
          <g transform={`translate(${view.tx} ${view.ty}) scale(${sx * view.s} ${-view.s})`}>
            <rect x={b.bounds[0] - 200} y={b.bounds[1] - 200} width={b.bounds[2] - b.bounds[0] + 400} height={b.bounds[3] - b.bounds[1] + 400} fill="url(#pcb-grid)" />
            {/* The board */}
            <path d={contourPath([b.outline, ...b.cutouts])} fill={maskColor} fillRule="evenodd" className="pcb-board" />
            <path d={contourPath([b.outline, ...b.cutouts])} fill="none" stroke="#e8e3c8" strokeWidth={0.12} />
            {/* Copper, far layer first */}
            {order.map((l) => {
              if (!showLayer(l)) return null;
              const color = COPPER[l] ?? "#999";
              const far = isOuter(l) && l !== (props.fromBelow ? "bottom" : "top");
              return (
                <g key={l} opacity={far ? 0.55 : l === props.active || !isOuter(l) ? 0.92 : 0.8} className={`pcb-layer pcb-layer-${l}`}>
                  {layers.zones &&
                    b.zones
                      .filter((z) => z.layer === l)
                      .map((z) => (
                        <path
                          key={z.id}
                          d={z.fill.map((island) => contourPath(island)).join("")}
                          fill={color}
                          fillOpacity={dimmed && !lit(z.net) ? 0.08 : sel?.id === z.id ? 0.55 : 0.32}
                          stroke={color}
                          strokeOpacity={0.6}
                          strokeWidth={0.05}
                          onPointerDown={(e) => {
                            if (routing) return;
                            e.stopPropagation();
                            props.onSel({ kind: "zone", id: z.id });
                          }}
                        />
                      ))}
                  {b.tracks
                    .filter((t) => t.layer === l && !replaced.has(t.id))
                    .map((t) => (
                      <polyline
                        key={t.id}
                        points={pts(trackPoints(t))}
                        fill="none"
                        stroke={color}
                        strokeWidth={t.width}
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        opacity={dimmed && !lit(t.net) ? 0.18 : 1}
                        className="pcb-track"
                        onPointerDown={(e) => onTrackDown(e, t)}
                      />
                    ))}
                  {(b.teardrops ?? [])
                    .filter((t) => t.layer === l)
                    .map((t, i) => (
                      <polygon key={`td${i}`} points={pts(t.polygon)} fill={color} opacity={dimmed && !lit(t.net) ? 0.18 : 1} className="pcb-teardrop" />
                    ))}
                  {(b.graphics ?? [])
                    .filter((g) => g.layer === l)
                    .map((g) => (
                      <GraphicShape key={g.id} g={g} color={color} opacity={dimmed && !lit(g.net) ? 0.2 : 1} />
                    ))}
                  {b.pads
                    .filter((p) => p.kind === "smd" && p.layers.includes(l))
                    .map((p, i) => (
                      <g key={`${p.reference}.${p.number}.${i}`} transform={tf(p.component)} opacity={dimmed && !lit(p.net) ? 0.25 : 1} onPointerEnter={(e) => hoverPad(e, p)} onPointerLeave={() => setHover(null)}>
                        <PrimShape p={p.prim} color={lit(p.net) ? "#fff2b0" : l === "top" ? "#e9b949" : "#8db6ef"} />
                      </g>
                    ))}
                </g>
              );
            })}
            {/* Through-hole pads and vias: on every layer */}
            {(layers.top || layers.bottom) && (
              <g>
                {b.pads
                  .filter((p) => p.kind === "tht")
                  .map((p, i) => (
                    <g key={`tht${i}`} transform={tf(p.component)} opacity={dimmed && !lit(p.net) ? 0.3 : 1} onPointerEnter={(e) => hoverPad(e, p)} onPointerLeave={() => setHover(null)}>
                      <PrimShape p={p.prim} color={lit(p.net) ? "#fff2b0" : "#d9ac45"} />
                    </g>
                  ))}
                {b.vias.map((v) => (
                  <circle
                    key={v.id}
                    cx={(movedVias.get(v.id) ?? v.at)[0]}
                    cy={(movedVias.get(v.id) ?? v.at)[1]}
                    r={v.diameter / 2}
                    fill={lit(v.net) ? "#fff2b0" : (v.kind && VIA_COLOR[v.kind]) || "#c9c4b4"}
                    stroke={v.kind && v.kind !== "through" ? "#1d1d1d" : undefined}
                    strokeWidth={v.kind && v.kind !== "through" ? v.diameter * 0.12 : undefined}
                    strokeDasharray={v.kind === "blind" || v.kind === "micro" ? `${v.diameter * 0.25} ${v.diameter * 0.15}` : undefined}
                    opacity={dimmed && !lit(v.net) ? 0.3 : 1}
                    onPointerDown={(e) => onViaDown(e, v)}
                  >
                    {v.kind && v.kind !== "through" && <title>{`${v.kind} via ${v.from} → ${v.to}`}</title>}
                  </circle>
                ))}
                {b.holes.map((h, i) => {
                  const p = b.pads.find((q) => `${q.reference}.${q.number}` === h.owner || q.reference === h.owner);
                  return (
                    <g key={`h${i}`} transform={p ? tf(p.component) : undefined}>
                      <HoleShape h={h} />
                    </g>
                  );
                })}
              </g>
            )}
            {/* Silkscreen of the side facing us */}
            {layers.silk &&
              b.components
                .filter((c) => c.side === silkSide)
                .map((c) => (
                  <g key={`s${c.id}`} transform={tf(c.id)} className="pcb-silk" style={{ stroke: b.silk.toLowerCase() === "black" ? "#111" : "#f4f2ea" }}>
                    {c.silk.map((l, i) => (
                      <polyline key={i} points={pts(l)} fill="none" strokeWidth={b.rules.silk_width} strokeLinecap="round" strokeLinejoin="round" />
                    ))}
                    {c.kind !== "hole" && (
                      <text transform={`translate(${c.label[0]} ${c.label[1]}) scale(${sx} -1)`} fontSize={b.rules.silk_height * 1.35} textAnchor="middle" dominantBaseline="central" className="pcb-ref" fill={b.silk.toLowerCase() === "black" ? "#111" : "#f4f2ea"} stroke="none">
                        {c.reference}
                      </text>
                    )}
                  </g>
                ))}
            {/* Board graphics on the silkscreen facing us, drawings, keepouts */}
            {layers.silk && (
              <g className="pcb-silk-gfx">
                {(b.graphics ?? [])
                  .filter((g) => g.layer === `silk_${silkSide}`)
                  .map((g) => (
                    <GraphicShape key={g.id} g={g} color={b.silk.toLowerCase() === "black" ? "#111" : "#f4f2ea"} opacity={1} />
                  ))}
              </g>
            )}
            {layers.drawings && (
              <g className="pcb-drawings">
                {(b.graphics ?? [])
                  .filter((g) => DRAWING_LAYERS.includes(g.layer))
                  .map((g) => (
                    <GraphicShape key={g.id} g={g} color={g.layer === "comments" ? "#8fc7ff" : g.layer === "user" ? "#c8c8c8" : "#d8c37a"} opacity={0.85} />
                  ))}
              </g>
            )}
            {layers.areas &&
              (b.areas ?? []).map((a) => (
                <g key={a.id} className="pcb-area">
                  <polygon points={pts(a.outline)} fill={a.no_tracks || a.no_vias || a.no_pours ? "url(#pcb-keepout)" : "none"} stroke={a.placement?.length ? "#7fb8ff" : "#ff6b6b"} strokeWidth={0.12} strokeDasharray="0.6 0.3" />
                  <text transform={`translate(${a.outline[0][0] + 0.4} ${a.outline[0][1] + 0.4}) scale(${sx} -1)`} fontSize={1} fill={a.placement?.length ? "#7fb8ff" : "#ff6b6b"} dominantBaseline="hanging">
                    {a.name}
                  </text>
                </g>
              ))}
            {/* Courtyards and hit areas */}
            {b.components.map((c) => {
              const chosen = sel?.kind === "component" && sel.id === c.id;
              return (
                <g key={`c${c.id}`} transform={tf(c.id)}>
                  <polygon
                    points={pts(c.courtyard.length ? c.courtyard : [c.at])}
                    className={`pcb-court ${layers.courtyard || chosen ? "shown" : ""} ${chosen ? "chosen" : ""} ${c.side}`}
                    onPointerDown={(e) => startDrag(e, c)}
                    onPointerEnter={(e) => {
                      const r = host.current!.getBoundingClientRect();
                      setHover({ x: e.clientX - r.left, y: e.clientY - r.top, text: `${c.reference} · ${c.value || c.footprint}`, sub: `${c.footprint}${c.side === "bottom" ? " · bottom" : ""}` });
                    }}
                    onPointerLeave={() => setHover(null)}
                  />
                </g>
              );
            })}
            {/* Tracks and vias take the pointer before the parts they cross */}
            {!routing && (
              <g className="pcb-hit">
                {b.tracks
                  .filter((t) => showLayer(t.layer) && !replaced.has(t.id))
                  .map((t) => (
                    <polyline key={`hit${t.id}`} points={pts(t.points)} fill="none" stroke="transparent" strokeWidth={Math.max(t.width, 6 / view.s)} strokeLinecap="round" strokeLinejoin="round" pointerEvents="stroke" onPointerDown={(e) => onTrackDown(e, t)} />
                  ))}
                {(layers.top || layers.bottom) &&
                  b.vias.map((v) => <circle key={`hit${v.id}`} cx={v.at[0]} cy={v.at[1]} r={Math.max(v.diameter / 2, 5 / view.s)} fill="transparent" pointerEvents="all" onPointerDown={(e) => onViaDown(e, v)} />)}
              </g>
            )}
            {/* Connections still to make */}
            {layers.ratsnest &&
              b.ratsnest.map((w, i) => {
                const da = (n: string) => {
                  const near = (p: P) => b.pads.find((q) => q.net === n && Math.hypot(q.at[0] - p[0], q.at[1] - p[1]) < 1e-3);
                  return [near(w.a), near(w.b)];
                };
                const [pa, pb] = da(w.net);
                const oa = pa ? offsetOf(pa.component) : null;
                const ob = pb ? offsetOf(pb.component) : null;
                return <line key={i} x1={w.a[0] + (oa?.[0] ?? 0)} y1={w.a[1] + (oa?.[1] ?? 0)} x2={w.b[0] + (ob?.[0] ?? 0)} y2={w.b[1] + (ob?.[1] ?? 0)} className={`pcb-rat ${lit(w.net) ? "lit" : ""}`} />;
              })}
            {/* Tracks dragged with a part: check or re-route them */}
            {b.tracks
              .filter((t) => t.reroute)
              .map((t) => (
                <polyline key={`rr${t.id}`} points={pts(t.points)} className="pcb-reroute" fill="none" strokeWidth={t.width + 0.18} strokeLinecap="round" strokeLinejoin="round" />
              ))}
            {/* The router's plan: pushed copper where it would go, the new line with its clearance, what it would break */}
            {plan && (
              <g pointerEvents="none" className="pcb-plan">
                {plan.tracks.map((m) => {
                  const t = b.tracks.find((x) => x.id === m.id);
                  if (!t) return null;
                  const line = pts(flatten(m.points, m.bulges));
                  return (
                    <g key={`pt${m.id}`}>
                      <polyline points={line} fill="none" stroke="#ffffff" strokeOpacity={0.1} strokeWidth={t.width + 2 * plan.clearance} strokeLinecap="round" strokeLinejoin="round" />
                      <polyline points={line} fill="none" stroke={COPPER[t.layer] ?? "#999"} strokeWidth={t.width} strokeLinecap="round" strokeLinejoin="round" opacity={0.95} />
                    </g>
                  );
                })}
                {plan.runs.map((r, i) => {
                  const line = pts(flatten(r.points, r.bulges));
                  return (
                    <g key={`pr${i}`}>
                      <polyline points={line} fill="none" stroke="#ffffff" strokeOpacity={0.13} strokeWidth={plan.width + 2 * plan.clearance} strokeLinecap="round" strokeLinejoin="round" />
                      <polyline points={line} fill="none" stroke={COPPER[r.layer] ?? "#999"} strokeWidth={plan.width} strokeLinecap="round" strokeLinejoin="round" opacity={0.92} />
                    </g>
                  );
                })}
                {plan.vias.map((at, i) => (
                  <g key={`pv${i}`}>
                    <circle cx={at[0]} cy={at[1]} r={plan.via_diameter / 2 + plan.clearance} fill="#ffffff" fillOpacity={0.1} />
                    <circle cx={at[0]} cy={at[1]} r={plan.via_diameter / 2} fill="#e3dfcf" />
                    <circle cx={at[0]} cy={at[1]} r={plan.via_drill / 2} fill="#15161a" />
                  </g>
                ))}
                {plan.collisions.map((c, i) => (
                  <g key={`pc${i}`} transform={`translate(${c.at[0]} ${c.at[1]})`} className="pcb-collision">
                    <circle r={0.5} fill="none" stroke="#ff4d4d" strokeWidth={0.1} />
                    <path d="M-0.3 -0.3L0.3 0.3M-0.3 0.3L0.3 -0.3" stroke="#ff4d4d" strokeWidth={0.1} strokeLinecap="round" />
                  </g>
                ))}
              </g>
            )}
            {/* A pair being routed: both tracks, the coupled vias, the centreline */}
            {pairPlan && (
              <g pointerEvents="none" className="pcb-plan">
                <polyline points={pts(pairPlan.centre)} fill="none" stroke="#ffffff" strokeOpacity={0.35} strokeWidth={0.06} strokeDasharray="0.3 0.25" />
                {[pairPlan.p, pairPlan.n].flatMap((pl, k) =>
                  pl.runs.map((r, i) => {
                    const line = pts(flatten(r.points, r.bulges));
                    const w = r.width ?? pl.width;
                    return (
                      <g key={`pp${k}${i}`}>
                        <polyline points={line} fill="none" stroke="#ffffff" strokeOpacity={0.1} strokeWidth={w + 2 * pl.clearance} strokeLinecap="round" strokeLinejoin="round" />
                        <polyline points={line} fill="none" stroke={COPPER[r.layer] ?? "#999"} strokeWidth={w} strokeLinecap="round" strokeLinejoin="round" opacity={0.92} />
                      </g>
                    );
                  }),
                )}
                {[...pairPlan.p.vias, ...pairPlan.n.vias].map((at, i) => (
                  <g key={`ppv${i}`}>
                    <circle cx={at[0]} cy={at[1]} r={pairPlan.p.via_diameter / 2} fill="#e3dfcf" />
                    <circle cx={at[0]} cy={at[1]} r={pairPlan.p.via_drill / 2} fill="#15161a" />
                  </g>
                ))}
                {pairPlan.p.tracks.map((m) => {
                  const t = b.tracks.find((x) => x.id === m.id);
                  return t ? <polyline key={`ppt${m.id}`} points={pts(flatten(m.points, m.bulges))} fill="none" stroke={COPPER[t.layer] ?? "#999"} strokeWidth={t.width} strokeLinecap="round" strokeLinejoin="round" opacity={0.95} /> : null;
                })}
                {pairPlan.p.collisions.map((c, i) => (
                  <g key={`ppc${i}`} transform={`translate(${c.at[0]} ${c.at[1]})`} className="pcb-collision">
                    <circle r={0.5} fill="none" stroke="#ff4d4d" strokeWidth={0.1} />
                    <path d="M-0.3 -0.3L0.3 0.3M-0.3 0.3L0.3 -0.3" stroke="#ff4d4d" strokeWidth={0.1} strokeLinecap="round" />
                  </g>
                ))}
              </g>
            )}
            {/* Meanders the tuning tool would lay */}
            {tune && tunePlan && (
              <g pointerEvents="none" className="pcb-plan">
                {tunePlan.edits.map((ed) => {
                  const t = b.tracks.find((x) => x.id === ed.id);
                  if (!t) return null;
                  return <polyline key={`tu${ed.id}`} points={pts(flatten(ed.points, ed.bulges))} fill="none" stroke={COPPER[t.layer] ?? "#999"} strokeWidth={t.width} strokeLinecap="round" strokeLinejoin="round" opacity={0.95} />;
                })}
              </g>
            )}
            {/* Selection */}
            {selTrack && <polyline points={pts(selTrack.points)} fill="none" className="pcb-chosen-line" strokeWidth={selTrack.width + 0.25} strokeLinecap="round" strokeLinejoin="round" />}
            {selVia && <circle cx={selVia.at[0]} cy={selVia.at[1]} r={selVia.diameter / 2 + 0.15} className="pcb-chosen-line" fill="none" strokeWidth={0.12} />}
            {b.tracks
              .filter((t) => groupSet.has(t.id))
              .map((t) => (
                <polyline key={`g${t.id}`} points={pts(t.points)} fill="none" className="pcb-chosen-line" strokeWidth={t.width + 0.25} strokeLinecap="round" strokeLinejoin="round" />
              ))}
            {b.vias
              .filter((v) => groupSet.has(v.id))
              .map((v) => (
                <circle key={`g${v.id}`} cx={v.at[0]} cy={v.at[1]} r={v.diameter / 2 + 0.15} className="pcb-chosen-line" fill="none" strokeWidth={0.12} />
              ))}
            {/* Design-rule markers */}
            {layers.drc &&
              errorsAt.map((v, i) => (
                <g key={`v${i}`} className={`pcb-mark ${v.severity} ${props.focus === v ? "focus" : ""}`} transform={`translate(${v.at[0]} ${v.at[1]})`}>
                  <circle r={0.9} />
                  <circle r={0.32} className="dot" />
                </g>
              ))}
            {/* A track being drawn */}
            {routing && start && !plan && cursor && <polyline points={pts([start.at, cursor])} fill="none" stroke={COPPER[props.active]} strokeWidth={0.12} strokeDasharray="0.4 0.3" opacity={0.7} className="pcb-route-preview" />}
            {routing && start && <circle cx={start.at[0]} cy={start.at[1]} r={0.3} fill="none" stroke={COPPER[start.layer] ?? "#fff"} strokeWidth={0.08} className="pcb-route-preview" />}
            {routing && cursor && <circle cx={cursor[0]} cy={cursor[1]} r={0.22} className="pcb-cursor" />}
          </g>
        </svg>
      )}
      {hover && !drag && (
        <div className="pcb-tip" style={{ left: hover.x + 14, top: hover.y + 14 }}>
          <b>{hover.text}</b>
          {hover.sub && <span>{hover.sub}</span>}
        </div>
      )}
      <div className="pcb-toolbar floating" role="toolbar" aria-label="Layers and tools">
        {(
          [
            ["top", "Top"],
            ["bottom", "Bottom"],
            ...(b.layers.length > 2 ? [["inner", "Inner"]] : []),
            ["zones", "Pours"],
            ["silk", "Silk"],
            ...(b.graphics?.some((g) => DRAWING_LAYERS.includes(g.layer)) ? [["drawings", "Drawings"]] : []),
            ...(b.areas?.length ? [["areas", "Keepouts"]] : []),
            ["courtyard", "Courtyards"],
            ["ratsnest", "Ratsnest"],
            ["drc", "DRC"],
          ] as [LayerKey, string][]
        ).map(([k, label]) => (
          <button key={k} type="button" className={`pcb-chip ${layers[k] ? "on" : ""}`} aria-pressed={layers[k]} onClick={() => props.onLayers({ ...layers, [k]: !layers[k] })}>
            {(k === "top" || k === "bottom") && <i className="sw" style={{ background: COPPER[k] }} />}
            {label}
          </button>
        ))}
        <i className="sep" />
        <button type="button" className={`pcb-chip ${props.fromBelow ? "on" : ""}`} onClick={() => props.onFromBelow(!props.fromBelow)} title="Look at the board from below">
          <Icon name="rotate" /> From below
        </button>
        <IconButton icon="fit" label="Fit the board" onClick={fit} />
      </div>
      <div className="pcb-tools floating" role="toolbar" aria-label="Route">
        <button
          type="button"
          className={`pcb-chip ${routing ? "on" : ""}`}
          disabled={!props.canEdit}
          onClick={() => {
            if (routing) resetRoute();
            setRouting(!routing);
          }}
          title="Route (X): click a pad or track end, click corners, click a pad of the net to finish"
        >
          <Icon name="pencil" /> Route
        </button>
        <button
          type="button"
          className={`pcb-chip ${routing && pairMode ? "on" : ""}`}
          disabled={!props.canEdit}
          onClick={() => {
            resetPair();
            resetRoute();
            if (routing && pairMode) {
              setPairMode(false);
              setRouting(false);
            } else {
              setRouting(true);
              setPairMode(true);
            }
          }}
          title="Route a differential pair (D): click a pad of the pair, click corners, Enter or double-click to finish at the far pads"
        >
          Pair
        </button>
        <button type="button" className="pcb-chip" onClick={() => setRouterMode(ROUTER_MODES[(ROUTER_MODES.indexOf(routerMode) + 1) % ROUTER_MODES.length])} title="Router mode (M): walk round obstacles, shove tracks and vias aside, or go straight and highlight collisions">
          {routerMode === "walkaround" ? "Walkaround" : routerMode === "shove" ? "Shove" : "Highlight"}
        </button>
        <button type="button" className="pcb-chip" onClick={() => setAngle(ANGLES[(ANGLES.indexOf(angle) + 1) % ANGLES.length])} title="Line angles: 45°, 90° or free (/ flips the bend)">
          {angle === "free" ? "Free" : `${angle}°`}
        </button>
        <button type="button" className={`pcb-chip ${arcCorners ? "on" : ""}`} onClick={() => setArcCorners(!arcCorners)} title="Round the corners of new routes into arcs">
          Arcs
        </button>
        <button type="button" className="pcb-chip layer-switch" onClick={() => props.onActive(b.layers[(b.layers.indexOf(props.active) + 1) % b.layers.length])} title="Layer to route on (V top/bottom, PageUp/PageDown every layer)">
          <i className="sw" style={{ background: COPPER[props.active] }} />
          {LAYER_NAME(props.active)}
        </button>
        <button type="button" className="pcb-chip" disabled={!props.canEdit || !!props.busy} onClick={props.onAutoroute} title="Route every unrouted net">
          <Icon name="spark" /> {props.busy ?? "Autoroute"}
        </button>
        {!routing && props.canEdit && (
          <>
            <i className="sep" />
            {(selTrack || props.group.length > 0) && (
              <button type="button" className="pcb-chip" onClick={() => void props.commit("pcb.route.optimize", { board: b.id, targets: props.group.length ? props.group : [selTrack!.id] }, "Smoothed")} title="Fewer corners, shorter, every clearance kept">
                Optimize
              </button>
            )}
            {selTrack?.net && (
              <button
                type="button"
                className={`pcb-chip ${tune ? "on" : ""}`}
                onClick={() => {
                  setTune(tune ? null : { track: selTrack.id, net: selTrack.net! });
                  setTunePlan(null);
                }}
                title="Length-tune this net or its pair (T): meanders to a target length, its group's or its partner's"
              >
                Tune
              </button>
            )}
            {selTrack && !selTrack.bulges?.length && (
              <button type="button" className="pcb-chip" onClick={() => void props.commit("pcb.track.fillet", { targets: [selTrack.id], radius: +(3 * selTrack.width).toFixed(3) }, "Corners rounded")} title="Round the track's corners into arcs">
                Round
              </button>
            )}
            {(selTrack || selVia) && (
              <button type="button" className="pcb-chip" onClick={() => void selectConnection()} title="Select the copper between the pads (U)">
                Connection
              </button>
            )}
            {(selTrack || selVia) && (
              <button type="button" className="pcb-chip" onClick={() => void props.commit("pcb.route.unroute", { board: b.id, targets: [sel!.id] }, "Connection unrouted")} title="Remove this connection's tracks and vias, pad to pad">
                Unroute
              </button>
            )}
            {selComp && (
              <button type="button" className="pcb-chip" onClick={() => void props.commit("pcb.fanout", { component: selComp.id }, `${selComp.reference} fanned out`)} title="A short stub and a via from each unrouted SMD pad">
                Fan out
              </button>
            )}
            {selComp && (
              <button type="button" className="pcb-chip" onClick={() => void props.commit("pcb.route.unroute", { board: b.id, components: [selComp.id] }, `${selComp.reference} unrouted`)} title="Remove the copper joined to this part's pads">
                Unroute part
              </button>
            )}
            {net && !selTrack && !selVia && (
              <button type="button" className="pcb-chip" onClick={() => void props.commit("pcb.route.unroute", { board: b.id, nets: [net] }, `${net} unrouted`)} title="Remove every track and via of this net">
                Unroute {net}
              </button>
            )}
            <button type="button" className="pcb-chip" onClick={() => void cleanup()} title="Merge straight-through points, join tracks end to end, trim dangling stubs, drop redundant vias">
              Clean up
            </button>
          </>
        )}
      </div>
      <div className="pcb-legend">
        {routing && pairMode ? (
          <span>
            {pairStart ? (
              <>
                Pair {pairStart.pair}
                {pairPlan ? (
                  <>
                    {" "}
                    · {FMT(pairPlan.width, 3)} mm / {FMT(pairPlan.gap, 3)} gap ·{" "}
                    <b className="pcb-length-readout">
                      {FMT(pairPlan.length_p, 2)} / {FMT(pairPlan.length_n, 2)} mm
                    </b>{" "}
                    · skew {FMT(pairPlan.skew, 3)}
                    {pairPlan.p.target != null ? ` · target ${FMT(pairPlan.p.target, 2)}` : ""}
                    {!pairPlan.ok ? ` · ${pairPlan.p.collisions.length ? `${pairPlan.p.collisions.length} clearance problems` : pairPlan.notes[0] ?? "blocked"}` : ""}
                  </>
                ) : null}{" "}
                · <kbd>V</kbd> coupled vias · <kbd>⌫</kbd> undo corner · <kbd>Enter</kbd> finish · <kbd>D</kbd> single track · <kbd>Esc</kbd>
              </>
            ) : (
              <>Click a pad of a differential pair to start · <kbd>M</kbd> mode · <kbd>D</kbd> single track · <kbd>Esc</kbd> stop</>
            )}
          </span>
        ) : routing ? (
          <span>
            {start ? (
              <>
                {legNet ?? "no net"} · {FMT(width ?? plan?.width ?? 0.25, 3)} mm · {routerMode}
                {plan?.length != null && (
                  <>
                    {" · "}
                    <b className={`pcb-length-readout ${plan.target != null ? (Math.abs(plan.target - plan.length) < 0.05 ? "ok" : plan.length < plan.target ? "short" : "") : ""}`}>
                      {FMT(plan.length, 2)} mm{plan.target != null ? ` / ${FMT(plan.target, 2)}` : ""}
                    </b>
                  </>
                )}
                {plan && !plan.reached ? " · blocked" : ""}
                {plan?.collisions.length ? ` · ${plan.collisions.length} clearance problem${plan.collisions.length === 1 ? "" : "s"}` : ""} · <kbd>/</kbd> bend · <kbd>V</kbd> via · <kbd>W</kbd> width · <kbd>⌫</kbd> undo corner · <kbd>F</kbd> finish to pad · <kbd>Enter</kbd> finish · <kbd>Esc</kbd>
              </>
            ) : (
              <>Click a pad, track end or via to start (anywhere: free) · <kbd>M</kbd> mode · <kbd>Esc</kbd> stop</>
            )}
          </span>
        ) : (
          <span>
            Drag parts, tracks and vias (Alt: free angle) · <kbd>X</kbd> route · <kbd>D</kbd> pair · <kbd>T</kbd> tune · <kbd>U</kbd> connection · <kbd>B</kbd> break track · <kbd>R</kbd> turn · <kbd>F</kbd> flip side · <kbd>Del</kbd> remove
          </span>
        )}
      </div>
      {tune && !routing && (
        <TunePanel
          key={tune.track}
          boardId={b.id}
          net={tune.net}
          trackId={tune.track}
          pair={tunePair as PairV | undefined}
          groupTarget={tuneGroup?.target}
          plan={tunePlan}
          onAsk={(spec) => askJson<TunePlan>({ op: "tune", ...spec }, setTunePlan)}
          onApply={async (spec) => {
            const out = (await props.commit("pcb.tune", spec)) as { output?: TunePlan } | null;
            const o = out?.output;
            setTune(null);
            setTunePlan(null);
            if (o) props.notify(`Tuned: ${o.nets.map((n) => `${n.net} ${FMT(n.after, 3)} mm`).join(", ")}${o.skew != null ? ` · skew ${FMT(o.skew, 4)}` : ""}`);
          }}
          onClose={() => {
            setTune(null);
            setTunePlan(null);
          }}
        />
      )}
    </div>
  );
}

// ---- Side panels -------------------------------------------------------------------

/** Show or hide one part or net on the board (inside a row button, so not a button itself). */
function EyeToggle(props: { hidden: boolean; label: string; onToggle: () => void }) {
  return (
    <span
      role="button"
      tabIndex={0}
      className={`pcb-eye ${props.hidden ? "off" : ""}`}
      title={props.hidden ? `Show ${props.label} on the board` : `Hide ${props.label} on the board`}
      aria-pressed={props.hidden}
      onClick={(e) => {
        e.stopPropagation();
        props.onToggle();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          e.stopPropagation();
          props.onToggle();
        }
      }}
    >
      <Icon name="eye" />
    </span>
  );
}

function PartsTab(props: { b: BoardView; sel: Selection; onSelect: (id: string) => void; commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>; hidden: string[]; onHide: (id: string) => void }) {
  const { b } = props;
  const [q, setQ] = useState("");
  const chosen = props.sel?.kind === "component" ? b.components.find((c) => c.id === props.sel!.id) : null;
  const list = b.components.filter((c) => !q || `${c.reference} ${c.value} ${c.footprint} ${c.mpn ?? ""}`.toLowerCase().includes(q.toLowerCase()));
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (chosen) listRef.current?.querySelector(`[data-id="${chosen.id}"]`)?.scrollIntoView({ block: "nearest" });
  }, [chosen]);
  return (
    <>
      <div className="pcb-search">
        <Icon name="search" />
        <input placeholder="Find a part" value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      <div className="pcb-list" ref={listRef}>
        {list.map((c) => (
          <button key={c.id} data-id={c.id} type="button" className={`pcb-row ${chosen?.id === c.id ? "active" : ""} ${c.dnp ? "dnp" : ""}`} onClick={() => props.onSelect(c.id)}>
            <span className="ref">{c.reference}</span>
            <span className="val">
              {c.value || c.footprint}
              <small>{c.footprint}</small>
            </span>
            {c.side === "bottom" && <span className="tag">B</span>}
            {!c.known && <span className="tag bad">?</span>}
            <EyeToggle hidden={props.hidden.includes(c.id)} label={c.reference} onToggle={() => props.onHide(c.id)} />
          </button>
        ))}
      </div>
      {chosen && <PartCard key={`${chosen.id}:${chosen.at.join(",")}:${chosen.rotation}`} c={chosen} b={b} commit={props.commit} />}
    </>
  );
}

function PartCard(props: { c: Comp; b: BoardView; commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown> }) {
  const { c, b } = props;
  const pads = b.pads.filter((p) => p.component === c.id && p.number);
  const seen = new Set<string>();
  const pins = pads.filter((p) => (seen.has(p.number) ? false : (seen.add(p.number), true)));
  const num = (label: string, value: number, key: "x" | "y" | "rotation", unit: string) => (
    <label className="pcb-num">
      <span>{label}</span>
      <input
        type="number"
        step={key === "rotation" ? 90 : 0.25}
        defaultValue={+value.toFixed(3)}
        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
        onBlur={(e) => {
          const v = +e.target.value;
          if (!Number.isFinite(v) || Math.abs(v - value) < 1e-9) return;
          const at: P = key === "x" ? [v, c.at[1]] : key === "y" ? [c.at[0], v] : c.at;
          void props.commit("pcb.component.update", key === "rotation" ? { target: c.id, rotation: v } : { target: c.id, at });
        }}
      />
      <i>{unit}</i>
    </label>
  );
  return (
    <div className="pcb-card">
      <div className="pcb-card-head">
        <b>{c.reference}</b>
        <span>{c.value}</span>
        <button type="button" className="pcb-side-toggle" onClick={() => props.commit("pcb.component.update", { target: c.id, side: c.side === "top" ? "bottom" : "top" })}>
          {c.side === "top" ? "Top" : "Bottom"} side
        </button>
      </div>
      {c.description && <p className="pcb-desc">{c.description}</p>}
      <div className="pcb-nums">
        {num("X", c.at[0], "x", "mm")}
        {num("Y", c.at[1], "y", "mm")}
        {num("Turn", c.rotation, "rotation", "°")}
      </div>
      <dl className="pcb-facts">
        <dt>Footprint</dt>
        <dd>{c.footprint}</dd>
        {c.mpn && (
          <>
            <dt>MPN</dt>
            <dd>
              {c.mpn}
              {c.manufacturer && <small> · {c.manufacturer}</small>}
            </dd>
          </>
        )}
        {c.lcsc && (
          <>
            <dt>LCSC</dt>
            <dd>
              <a href={`https://www.lcsc.com/product-detail/${c.lcsc}.html`} target="_blank" rel="noreferrer">
                {c.lcsc} <Icon name="external" />
              </a>
            </dd>
          </>
        )}
        <dt>Height</dt>
        <dd>{FMT(c.height)} mm</dd>
      </dl>
      {pins.length > 0 && (
        <div className="pcb-pins">
          {pins.map((p) => (
            <span key={p.number} className={p.net ? "" : "nc"}>
              <b>{p.number}</b>
              {p.pin_name && p.pin_name !== p.net && <i>{p.pin_name}</i>}
              <em>{p.net ?? "—"}</em>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

function NetsTab(props: { b: BoardView; net: string | null; onNet: (n: string) => void; hidden: string[]; onHide: (n: string) => void }) {
  const { b } = props;
  const routed = b.nets.filter((n) => n.routed && n.pins.length > 1).length;
  const multi = b.nets.filter((n) => n.pins.length > 1).length;
  return (
    <>
      <div className="pcb-progress">
        <div className="bar">
          <i style={{ width: `${multi ? (100 * routed) / multi : 100}%` }} />
        </div>
        <span>
          {routed} of {multi} nets routed · {FMT(b.stats.track_length, 0)} mm of track · {b.stats.vias} vias
        </span>
      </div>
      <div className="pcb-list">
        {b.nets.map((n) => (
          <button key={n.name} type="button" className={`pcb-row net ${props.net === n.name ? "active" : ""}`} onClick={() => props.onNet(n.name)}>
            <span className={`state ${n.routed ? "ok" : "open"}`} title={n.routed ? "Routed" : `${n.islands} pieces`} />
            <span className="val">
              {n.name}
              <small>
                {n.pins.slice(0, 6).join(" ")}
                {n.pins.length > 6 ? ` +${n.pins.length - 6}` : ""}
              </small>
            </span>
            {n.class !== "Default" && <span className="tag">{n.class}</span>}
            <EyeToggle hidden={props.hidden.includes(n.name)} label={n.name} onToggle={() => props.onHide(n.name)} />
          </button>
        ))}
      </div>
    </>
  );
}

const CODE_NAMES: Record<string, string> = {
  clearance: "Clearance",
  short: "Short",
  unrouted: "Unrouted",
  track_width: "Track width",
  drill: "Drill",
  annular_ring: "Annular ring",
  edge_clearance: "Board edge",
  outside_board: "Off the board",
  courtyard_overlap: "Courtyards",
  hole_to_hole: "Hole to hole",
  unknown_footprint: "Footprint",
  single_pad_net: "Single pad",
  no_net: "No net",
  silk_text: "Silkscreen",
  impedance: "Impedance",
  diff_pair_gap: "Pair gap",
  diff_pair_uncoupled: "Uncoupled",
  length: "Length",
  skew: "Skew",
  via_count: "Via count",
};

function ChecksTab(props: { b: BoardView; focus: number | null; onFocus: (i: number) => void; onAutoroute: () => void; onPour: () => void; busy: string | null; canEdit: boolean }) {
  const { b } = props;
  const errors = b.violations.filter((v) => v.severity === "error").length;
  const warnings = b.violations.length - errors;
  const st = b.stats;
  return (
    <>
      <div className={`pcb-verdict ${errors ? "bad" : "good"}`}>
        <Icon name={errors ? "alert" : "check"} />
        <div>
          <b>{errors ? `${errors} ${errors === 1 ? "error" : "errors"}` : "Passes the design-rule check"}</b>
          <span>
            {warnings ? `${warnings} warning${warnings === 1 ? "" : "s"} · ` : ""}
            clearance, widths, drills, rings, edge, holes, courtyards, shorts and connections
          </span>
        </div>
      </div>
      <div className="pcb-actions">
        <button type="button" className="secondary" disabled={!props.canEdit || !!props.busy} onClick={props.onAutoroute}>
          <Icon name="spark" /> {props.busy ?? "Autoroute"}
        </button>
        <button type="button" className="secondary" disabled={!props.canEdit} onClick={props.onPour}>
          <Icon name="layers" /> Ground pour
        </button>
      </div>
      <div className="pcb-list">
        {b.violations.map((v, i) => (
          <button key={i} type="button" className={`pcb-row viol ${v.severity} ${props.focus === i ? "active" : ""}`} onClick={() => props.onFocus(i)}>
            <span className="vtag">{CODE_NAMES[v.code] ?? v.code}</span>
            <span className="val">{v.message}</span>
          </button>
        ))}
      </div>
      <dl className="pcb-facts small">
        <dt>Smallest track</dt>
        <dd>{st.smallest_track ? `${FMT(st.smallest_track, 3)} mm` : "—"}</dd>
        <dt>Smallest gap</dt>
        <dd>{st.smallest_gap ? `${FMT(st.smallest_gap, 3)} mm` : "—"}</dd>
        <dt>Smallest hole</dt>
        <dd>{st.smallest_drill ? `${FMT(st.smallest_drill, 3)} mm` : "—"}</dd>
        <dt>Smallest ring</dt>
        <dd>{st.smallest_annular ? `${FMT(st.smallest_annular, 3)} mm` : "—"}</dd>
        <dt>Tallest part</dt>
        <dd>
          {FMT(st.tallest_top)} mm top{st.tallest_bottom ? ` · ${FMT(st.tallest_bottom)} mm bottom` : ""}
        </dd>
      </dl>
    </>
  );
}

const GRAPHIC_LAYERS = ["silk_top", "silk_bottom", "fab_top", "fab_bottom", "user", "comments", "mask_top", "mask_bottom"];
const LAYER_COUNTS = [2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32];

/** Board setup: stackup, teardrops, the custom rules, arranging parts and adding graphics and keepouts. */
function SetupTab(props: { b: BoardView; commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>; canEdit: boolean }) {
  const { b, commit, canEdit } = props;
  const st = b.stackup;
  const [diel, setDiel] = useState<Dielectric[]>(st.dielectrics);
  const stKey = JSON.stringify(st.dielectrics);
  useEffect(() => setDiel(st.dielectrics), [stKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const [rules, setRules] = useState(b.drc_rules ?? "");
  useEffect(() => setRules(b.drc_rules ?? ""), [b.id, b.drc_rules]);
  const [refs, setRefs] = useState("");
  const [grid, setGrid] = useState("2,1,5,0");
  const cx = (b.bounds[0] + b.bounds[2]) / 2;
  const [text, setText] = useState({ text: "", layer: "silk_top", at: `${FMT(cx)},${FMT(b.bounds[1] + 2)}`, size: 1.5, font: "stroke", knockout: false });
  const [keepout, setKeepout] = useState({ name: "Keepout", rect: "" });
  const td = b.teardrop_settings;
  const ruleHits = b.violations.filter((v) => v.code === "rules_syntax" || /rule “/.test(v.message));
  const syntax = b.violations.find((v) => v.code === "rules_syntax");
  const targets = () =>
    refs
      .split(/[\s,]+/)
      .filter(Boolean)
      .map((r) => b.components.find((c) => c.reference.toLowerCase() === r.toLowerCase())?.id ?? r)
      .filter((x) => !!x);
  const arrange = async (op: string, params: Record<string, unknown>, done: string) => {
    const t = targets();
    if (!t.length) return;
    await commit(op, { targets: t, ...params }, done);
  };
  const num = (s: string) => s.split(",").map((x) => +x.trim());
  const thick = (_d: Dielectric, i: number, patch: Partial<Dielectric>) => setDiel((all) => all.map((x, k) => (k === i ? { ...x, ...patch } : x)));
  const off = Math.abs(st.total - b.thickness) > Math.max(0.1, b.thickness * 0.1);
  let di = -1;
  return (
    <div className="pcb-setup">
      <h4 className="pcb-h">Stackup</h4>
      <div className="pcb-specs-grid">
        <label className="pcb-spec">
          <span>Copper layers</span>
          <select value={b.layers.length} disabled={!canEdit} onChange={(e) => commit("pcb.stackup.set", { board: b.id, layers: +e.target.value }, `${e.target.value} copper layers`)}>
            {LAYER_COUNTS.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <div className="pcb-spec">
          <span>Adds up to</span>
          <b className={off ? "pcb-bad" : ""}>
            {FMT(st.total, 3)} mm {off ? `(board ${FMT(b.thickness)} mm)` : ""}
          </b>
        </div>
      </div>
      <table className="pcb-stack">
        <tbody>
          {st.rows
            .filter((r) => r.kind !== "silk")
            .map((r, i) => {
              if (r.kind === "core" || r.kind === "prepreg") di += 1;
              const k = di;
              const d = diel[k];
              return (
                <tr key={i} className={`pcb-stack-${r.kind}`}>
                  <td>{r.kind === "copper" ? LAYER_NAME(r.name) : r.name}</td>
                  {(r.kind === "core" || r.kind === "prepreg") && d ? (
                    <>
                      <td>
                        <select value={d.kind} disabled={!canEdit} onChange={(e) => thick(d, k, { kind: e.target.value as Dielectric["kind"] })}>
                          <option value="core">core</option>
                          <option value="prepreg">prepreg</option>
                        </select>
                      </td>
                      <td>
                        <input type="number" step={0.01} min={0.01} value={d.thickness} disabled={!canEdit} onChange={(e) => thick(d, k, { thickness: +e.target.value })} aria-label="Thickness mm" />
                      </td>
                      <td>
                        <input value={d.material} disabled={!canEdit} onChange={(e) => thick(d, k, { material: e.target.value })} aria-label="Material" />
                      </td>
                      <td>
                        <input type="number" step={0.1} min={1} value={d.epsilon_r ?? 4.5} disabled={!canEdit} onChange={(e) => thick(d, k, { epsilon_r: +e.target.value })} aria-label="εr" title="εr" />
                      </td>
                    </>
                  ) : (
                    <>
                      <td>{r.kind === "copper" ? `${FMT(r.copper_oz ?? 1)} oz` : r.kind}</td>
                      <td>{FMT(r.thickness, 4)}</td>
                      <td colSpan={2}>{r.material}</td>
                    </>
                  )}
                </tr>
              );
            })}
        </tbody>
      </table>
      <div className="pcb-row-actions">
        <button type="button" className="small primary" disabled={!canEdit || JSON.stringify(diel) === stKey} onClick={() => commit("pcb.stackup.set", { board: b.id, dielectrics: diel }, "Stackup set")}>
          Apply
        </button>
        <button type="button" className="small" disabled={!canEdit} onClick={() => commit("pcb.stackup.set", { board: b.id, fit_thickness: true }, "Thickness fitted to the stackup")}>
          Fit thickness
        </button>
        <button type="button" className="small" disabled={!canEdit || !st.custom} onClick={() => commit("pcb.stackup.set", { board: b.id, standard: true }, "Standard stackup")}>
          Standard
        </button>
      </div>

      <h4 className="pcb-h">Teardrops</h4>
      <div className="pcb-checks-row">
        <label>
          <input type="checkbox" checked={!!td} disabled={!canEdit} onChange={(e) => commit("pcb.teardrops.set", { board: b.id, enabled: e.target.checked })} /> On
        </label>
        {td && (
          <>
            {(["vias", "pads", "smd", "curved"] as const).map((k) => (
              <label key={k}>
                <input type="checkbox" checked={td[k]} disabled={!canEdit} onChange={(e) => commit("pcb.teardrops.set", { board: b.id, [k]: e.target.checked })} /> {k === "smd" ? "SMD pads" : k === "pads" ? "THT pads" : k}
              </label>
            ))}
            <span className="pcb-note">{b.teardrops.length} on the board</span>
          </>
        )}
      </div>

      <h4 className="pcb-h">Custom rules</h4>
      <p className="pcb-note">KiCad rules language: (rule "name" (condition "A.NetClass == 'HV'") (constraint clearance (min 1.5mm))).</p>
      <textarea className="pcb-rules" value={rules} spellCheck={false} disabled={!canEdit} onChange={(e) => setRules(e.target.value)} rows={7} placeholder={'(version 1)\n(rule "HV"\n  (condition "A.NetClass == \'HV\' && B.NetClass != \'HV\'")\n  (constraint clearance (min 1.5mm)))'} />
      <div className="pcb-row-actions">
        <button type="button" className="small primary" disabled={!canEdit || rules === (b.drc_rules ?? "")} onClick={() => commit("pcb.rules.set", { board: b.id, text: rules }, "Rules set")}>
          Apply rules
        </button>
        <span className="pcb-note">
          {b.custom_rules.length} rule{b.custom_rules.length === 1 ? "" : "s"} · {ruleHits.length} violation{ruleHits.length === 1 ? "" : "s"}
        </span>
      </div>
      {syntax && <p className="pcb-bad">{syntax.message}</p>}
      {b.custom_rules.length > 0 && (
        <ul className="pcb-rule-list">
          {b.custom_rules.map((r) => (
            <li key={r.name}>
              <b>{r.name}</b> {r.constraints.map((c) => `${c.kind}${c.min != null ? ` ≥ ${FMT(c.min, 3)}` : ""}${c.max != null ? ` ≤ ${FMT(c.max, 3)}` : ""}${c.disallow?.length ? ` ${c.disallow.join(" ")}` : ""}${c.unchecked ? " (not checked)" : ""}`).join(", ")}
              {r.condition && <small> when {r.condition}</small>}
            </li>
          ))}
        </ul>
      )}

      <h4 className="pcb-h">Arrange</h4>
      <input className="pcb-refs" value={refs} onChange={(e) => setRefs(e.target.value)} placeholder="R1 R2 R3 (references)" aria-label="Parts to arrange" />
      <div className="pcb-row-actions wrap">
        {(
          [
            ["left", "Left"],
            ["right", "Right"],
            ["top", "Top"],
            ["bottom", "Bottom"],
            ["center_x", "Centre X"],
            ["center_y", "Centre Y"],
          ] as [string, string][]
        ).map(([k, label]) => (
          <button key={k} type="button" className="small" disabled={!canEdit} onClick={() => arrange("pcb.align", { to: k }, `Aligned ${label.toLowerCase()}`)}>
            {label}
          </button>
        ))}
        <button type="button" className="small" disabled={!canEdit} onClick={() => arrange("pcb.distribute", { axis: "x" }, "Distributed along x")}>
          Distribute X
        </button>
        <button type="button" className="small" disabled={!canEdit} onClick={() => arrange("pcb.distribute", { axis: "y" }, "Distributed along y")}>
          Distribute Y
        </button>
        <button type="button" className="small" disabled={!canEdit} onClick={() => arrange("pcb.flip", {}, "Flipped to the other side")}>
          Flip
        </button>
      </div>
      <div className="pcb-row-actions">
        <input className="pcb-refs small" value={grid} onChange={(e) => setGrid(e.target.value)} aria-label="Array nx,ny,dx,dy" title="nx,ny,dx,dy" />
        <button
          type="button"
          className="small"
          disabled={!canEdit}
          onClick={() => {
            const [nx, ny, dx, dy] = num(grid);
            arrange("pcb.array", { grid: { nx, ny, dx, dy } }, "Array made");
          }}
        >
          Array
        </button>
      </div>

      <h4 className="pcb-h">Add text</h4>
      <div className="pcb-specs-grid">
        <label className="pcb-spec">
          <span>Text</span>
          <input value={text.text} onChange={(e) => setText({ ...text, text: e.target.value })} placeholder="REV A" />
        </label>
        <label className="pcb-spec">
          <span>Layer</span>
          <select value={text.layer} onChange={(e) => setText({ ...text, layer: e.target.value })}>
            {[...GRAPHIC_LAYERS, ...b.layers].map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </select>
        </label>
        <label className="pcb-spec">
          <span>At x,y</span>
          <input value={text.at} onChange={(e) => setText({ ...text, at: e.target.value })} />
        </label>
        <label className="pcb-spec">
          <span>Size / font</span>
          <span className="pcb-inline">
            <input type="number" step={0.1} min={0.3} value={text.size} onChange={(e) => setText({ ...text, size: +e.target.value })} />
            <select value={text.font} onChange={(e) => setText({ ...text, font: e.target.value })}>
              <option value="stroke">Stroke</option>
              <option value="Inter">Inter</option>
            </select>
          </span>
        </label>
      </div>
      <div className="pcb-row-actions">
        <label>
          <input type="checkbox" checked={text.knockout} onChange={(e) => setText({ ...text, knockout: e.target.checked })} /> Knockout
        </label>
        <button
          type="button"
          className="small primary"
          disabled={!canEdit || !text.text.trim()}
          onClick={() => commit("pcb.graphic.create", { board: b.id, layer: text.layer, kind: "text", text: text.text, at: num(text.at), size: text.size, ...(text.font !== "stroke" ? { font: text.font } : {}), knockout: text.knockout }, "Text added")}
        >
          Add text
        </button>
      </div>

      <h4 className="pcb-h">Add keepout</h4>
      <div className="pcb-row-actions">
        <input className="pcb-refs small" value={keepout.name} onChange={(e) => setKeepout({ ...keepout, name: e.target.value })} aria-label="Keepout name" />
        <input className="pcb-refs" value={keepout.rect} onChange={(e) => setKeepout({ ...keepout, rect: e.target.value })} placeholder="x0,y0,x1,y1" aria-label="Keepout rectangle" />
        <button
          type="button"
          className="small primary"
          disabled={!canEdit || num(keepout.rect).length !== 4 || num(keepout.rect).some((v) => !Number.isFinite(v))}
          onClick={() => {
            const [x0, y0, x1, y1] = num(keepout.rect);
            commit("pcb.area.create", { board: b.id, name: keepout.name, outline: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] }, `Keepout ${keepout.name}`);
          }}
        >
          Add
        </button>
      </div>

      {(b.graphics.length > 0 || b.areas.length > 0) && (
        <>
          <h4 className="pcb-h">Graphics and areas</h4>
          <ul className="pcb-rule-list">
            {b.graphics.map((g) => (
              <li key={g.id}>
                <b>{g.kind}</b> {g.text ? `“${g.text}” ` : ""}on {g.layer}
                {g.value != null ? ` · ${FMT(g.value, 3)} mm` : ""}
                <button type="button" className="tiny" disabled={!canEdit} onClick={() => commit("pcb.delete", { targets: [g.id] })} aria-label="Delete">
                  ×
                </button>
              </li>
            ))}
            {b.areas.map((a) => (
              <li key={a.id}>
                <b>{a.placement?.length ? "placement" : "keepout"}</b> {a.name} · no {[a.no_tracks && "tracks", a.no_vias && "vias", a.no_pads && "pads", a.no_pours && "pours", a.no_footprints && "footprints"].filter(Boolean).join(", ") || "—"}
                <button type="button" className="tiny" disabled={!canEdit} onClick={() => commit("pcb.delete", { targets: [a.id] })} aria-label="Delete">
                  ×
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

function FabTab(props: { d: Detail; commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>; boardId: string; revision: number }) {
  const { d } = props;
  const b = d.board;
  const set = (patch: Record<string, unknown>) => props.commit("pcb.board.update", { target: b.id, ...patch });
  const fileUrl = (name: string, extra = "") => projectUrl(`/pcb/${b.id}/file/${encodeURIComponent(name)}${extra}`);
  const sel = (label: string, value: string | number, options: (string | number)[], key: string, num = false) => (
    <label className="pcb-spec">
      <span>{label}</span>
      <select value={value} onChange={(e) => set({ [key]: num ? +e.target.value : e.target.value })}>
        {(options.includes(value) ? options : [value, ...options]).map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    </label>
  );
  return (
    <>
      <div className="pcb-specs-grid">
        {sel("Layers", b.layers.length, [2, 4, 6, 8, 10, 12, 16, 20, 24, 32], "layers", true)}
        {sel("Thickness", b.thickness, [0.8, 1.0, 1.2, 1.6, 2.0], "thickness", true)}
        {sel("Finish", b.finish, ["HASL lead-free", "HASL", "ENIG", "OSP"], "finish")}
        {sel("Mask", b.mask, ["green", "black", "blue", "red", "white", "purple", "yellow"], "mask")}
        {sel("Copper", b.copper_oz, [1, 2], "copper_oz", true)}
        <label className="pcb-spec">
          <span>Quantity</span>
          <input type="number" min={1} defaultValue={b.quantity} key={b.quantity} onBlur={(e) => +e.target.value !== b.quantity && +e.target.value >= 1 && set({ quantity: Math.round(+e.target.value) })} />
        </label>
      </div>
      <h4 className="pcb-h">Where to make it</h4>
      <p className="pcb-note">Checked against each fab’s published limits. Nothing is sent from here: download the package, then upload it on their quote page.</p>
      <div className="pcb-fabs">
        {d.fabs.map((r) => {
          const bad = r.checks.filter((c) => !c.ok);
          const chosen = b.fab === r.fab.id;
          return (
            <div key={r.fab.id} className={`pcb-fab ${r.can_make ? "ok" : "no"} ${chosen ? "chosen" : ""}`}>
              <div className="pcb-fab-head">
                <b>{r.fab.name}</b>
                <span className="where">{r.fab.country}</span>
                <span className={`verdict ${r.can_make ? "ok" : "no"}`}>{r.can_make ? "Can make it" : `${bad.length} issue${bad.length === 1 ? "" : "s"}`}</span>
              </div>
              {bad.length > 0 && (
                <ul className="pcb-fab-bad">
                  {bad.map((c) => (
                    <li key={c.what}>
                      <b>{c.what}</b> {c.board} — {c.fab}
                    </li>
                  ))}
                </ul>
              )}
              {r.estimate && (
                <div className="pcb-estimate" title={`Published formula: ${r.estimate.source}`}>
                  <span>{money(r.estimate.amount, r.estimate.currency)}</span>
                  <small>{r.estimate.working}</small>
                </div>
              )}
              <p className="pcb-fab-files">{r.fab.assembly}</p>
              <div className="pcb-fab-actions">
                <a className="button primary small" href={projectUrl(`/pcb/${b.id}/fab.zip?fab=${r.fab.id}`)} download>
                  <Icon name="download" /> Package
                </a>
                <a className="button secondary small" href={r.fab.quote} target="_blank" rel="noreferrer">
                  Quote page <Icon name="external" />
                </a>
                <button type="button" className={`small ${chosen ? "chosen" : ""}`} onClick={() => set({ fab: chosen ? "" : r.fab.id })}>
                  {chosen ? "Chosen" : "Choose"}
                </button>
              </div>
            </div>
          );
        })}
      </div>
      <h4 className="pcb-h">Files</h4>
      <div className="pcb-files">
        {d.files.map((f) => (
          <span key={f} className="pcb-file">
            <a href={fileUrl(f)} target="_blank" rel="noreferrer">
              {f}
            </a>
            <a href={fileUrl(f, "?download=1")} aria-label={`Download ${f}`}>
              <Icon name="download" />
            </a>
          </span>
        ))}
      </div>
      <a className="button secondary pcb-all" href={projectUrl(`/pcb/${b.id}/fab.zip`)} download>
        <Icon name="download" /> Everything (generic package)
      </a>
      <h4 className="pcb-h">For other EDA tools</h4>
      <p className="pcb-note">KiCad project and schematic, netlists, IPC-2581, ODB++, IPC-D-356, Gerber job, drill map and report, plots, 3D models and position files.</p>
      <InterchangeFiles boardId={b.id} revision={props.revision} />
      <FabCheck boardId={props.boardId} revision={props.revision} />
    </>
  );
}

function ShopTab(props: { d: Detail; commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown> }) {
  const { d } = props;
  const b = d.board;
  const lines = d.bom.filter((l) => l.kind !== "hole");
  const basic = lines.filter((l) => l.lcsc_class === "Basic").length;
  const extended = lines.filter((l) => l.lcsc_class === "Extended").length;
  const save = (l: BomLine, patch: Choice) => {
    const c = { ...(l.choice ?? {}), ...patch };
    void props.commit("pcb.sourcing.set", { board: b.id, key: l.key, ...c });
  };
  const fab = d.fabs.find((r) => r.fab.id === b.fab);
  return (
    <>
      <div className="pcb-shop-sum">
        <div>
          <b>{lines.reduce((s, l) => s + (l.dnp ? 0 : l.quantity), 0) * b.quantity}</b>
          <span>parts for {b.quantity} boards</span>
        </div>
        <div>
          <b>{lines.length}</b>
          <span>lines</span>
        </div>
        <div>
          <b>
            {basic}/{extended}
          </b>
          <span>JLC basic / extended</span>
        </div>
        <div>
          <b>{Object.entries(d.totals).map(([cur, v]) => money(v, cur)).join(" + ") || "—"}</b>
          <span>{d.unpriced ? `${d.unpriced} lines unpriced` : "recorded prices"}</span>
        </div>
      </div>
      <div className="pcb-board-order">
        <Icon name="chip" />
        <span>
          <b>The board</b> · {b.quantity} × {FMT(d.size[0], 1)} × {FMT(d.size[1], 1)} mm, {b.layers.length} layers — {fab ? `${fab.fab.name}${fab.estimate ? `, ${money(fab.estimate.amount, fab.estimate.currency)} (their published price)` : ", quote on their page"}` : "choose a fab under Fab"}
        </span>
      </div>
      <div className="pcb-bom">
        {lines.map((l) => (
          <div key={l.key + (l.dnp ? "dnp" : "")} className={`pcb-bom-line ${l.dnp ? "dnp" : ""}`}>
            <div className="top">
              <span className="qty">{l.quantity}×</span>
              <span className="what">
                <b>{l.value || l.mpn || l.footprint}</b>
                <small>
                  {l.references.join(", ")} · {l.footprint}
                </small>
              </span>
              {l.lcsc_class && <span className={`cls ${l.lcsc_class.toLowerCase()}`}>{l.lcsc_class}</span>}
            </div>
            {(l.mpn || l.lcsc) && (
              <div className="ids">
                {l.mpn && <span>{l.manufacturer ? `${l.manufacturer} ` : ""}{l.mpn}</span>}
                {l.lcsc && <span className="lcsc">LCSC {l.lcsc}</span>}
              </div>
            )}
            {l.links.length > 0 ? (
              <div className="links">
                {l.links.map((k) => (
                  <a key={k.name} href={k.url} target="_blank" rel="noreferrer">
                    {k.name}
                  </a>
                ))}
              </div>
            ) : (
              <div className="links none">Add an MPN to get distributor links</div>
            )}
            <div className="pcb-choice">
              <input placeholder="Supplier" defaultValue={l.choice?.supplier ?? ""} onBlur={(e) => e.target.value !== (l.choice?.supplier ?? "") && save(l, { supplier: e.target.value })} />
              <input placeholder="Their part #" defaultValue={l.choice?.sku ?? ""} onBlur={(e) => e.target.value !== (l.choice?.sku ?? "") && save(l, { sku: e.target.value })} />
              <input
                placeholder="Unit price"
                type="number"
                min={0}
                step="0.0001"
                defaultValue={l.choice?.unit_price ?? ""}
                onBlur={(e) => {
                  const v = e.target.value === "" ? undefined : +e.target.value;
                  if (v !== l.choice?.unit_price) save(l, { unit_price: v, currency: l.choice?.currency ?? "USD" });
                }}
              />
            </div>
          </div>
        ))}
      </div>
      <Keys />
    </>
  );
}

function Keys() {
  const [open, setOpen] = useState(false);
  const [info, setInfo] = useState<{ keys: { id: string; label: string; set: boolean }[]; note: string } | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  useEffect(() => {
    if (open && !info) fetch("/pcb/settings").then((r) => r.json()).then(setInfo).catch(() => {});
  }, [open, info]);
  return (
    <details className="pcb-keys" open={open} onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
      <summary>Distributor API keys (optional)</summary>
      {info && (
        <>
          <p className="pcb-note">{info.note}</p>
          {info.keys.map((k) => (
            <label key={k.id} className="pcb-key">
              <span>
                {k.label}
                {k.set && <em>saved</em>}
              </span>
              <input type="password" autoComplete="off" placeholder={k.set ? "•••••• (leave empty to keep)" : "not set"} value={draft[k.id] ?? ""} onChange={(e) => setDraft({ ...draft, [k.id]: e.target.value })} />
            </label>
          ))}
          <button
            type="button"
            className="secondary small"
            disabled={!Object.values(draft).some((v) => v.trim())}
            onClick={async () => {
              const body = Object.fromEntries(Object.entries(draft).filter(([, v]) => v.trim()));
              const r = await fetch("/pcb/settings", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
              if (r.ok) {
                setInfo(await r.json());
                setDraft({});
              }
            }}
          >
            Save on this computer
          </button>
        </>
      )}
    </details>
  );
}
