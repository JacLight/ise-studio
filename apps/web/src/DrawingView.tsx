// A part's engineering drawing: the sheet the engine lays out (views from
// the exact solid, hidden lines dashed, dimensions, title block), drawn as
// SVG, with the sheet size, projection and part to choose, and PDF and DXF
// downloads of exactly what is shown. The front view can be cut through
// (section A–A); dimensions (with tolerances and ISO 286 fits), detail
// views, datums and feature control frames are added on the views. All of
// it is kept with the part.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { projectUrl } from "./engine";
import { Icon } from "./icons";

type Pen = "visible" | "hidden" | "thin" | "center" | "frame";
type ViewName = "front" | "top" | "right";
type DimKind = "horizontal" | "vertical" | "aligned";
type Pt = [number, number];
type Box = [number, number, number, number];
type ViewFrame = { view: ViewName; at: Pt; scale: number; axes: [number, number]; bounds: Box };
type MarkKind = "detail" | "frame" | "datum" | "hole";
type AnnotationMark = { kind: MarkKind; index: number; diameter?: number; bounds: Box };
type Drawing = {
  width: number;
  height: number;
  scale: number;
  scale_text: string;
  strokes: { pen: Pen; points: Pt[] }[];
  fills: Pt[][];
  texts: { at: Pt; size: number; text: string; anchor: "start" | "middle" | "end"; angle: number; bold: boolean }[];
  views: ViewFrame[];
  marks: { dimension: number; bounds: Box }[];
  annotations?: AnnotationMark[];
  warnings?: string[];
};
export type Tolerance = { type: "symmetric"; value: number } | { type: "limits"; upper: number; lower: number } | { type: "fit"; fit: string };
export type DimSpec = { view: ViewName; a: Pt; b: Pt; kind: DimKind; offset: number; tolerance?: Tolerance; diameter?: boolean };
export type DetailSpec = { view: ViewName; center: Pt; radius: number; scale?: number | null };
type GdtSymbol = "straightness" | "flatness" | "circularity" | "cylindricity" | "line_profile" | "surface_profile" | "parallelism" | "perpendicularity" | "angularity" | "position" | "concentricity" | "symmetry" | "circular_runout" | "total_runout";
export type FrameSpec = { view: ViewName; at: Pt; symbol: GdtSymbol; tolerance: number; diameter_zone?: boolean; datums?: string[]; modifier?: "mmc" | "lmc" | null };
export type DatumSpec = { view: ViewName; at: Pt; letter: string };
export type HoleFit = { diameter: number; fit: string };
export type DrawingSetup = { part: string | null; section: number | null; dimensions: DimSpec[]; details?: DetailSpec[]; frames?: FrameSpec[]; datums?: DatumSpec[]; hole_fits?: HoleFit[] };

type Tool = "none" | "dimension" | "detail" | "datum" | "frame";

const SHEETS = ["A4", "A3", "A2", "A1", "A0"] as const;
const WIDTH: Record<Pen, number> = { visible: 0.5, hidden: 0.25, thin: 0.25, center: 0.25, frame: 0.7 };
const DASH: Partial<Record<Pen, string>> = { hidden: "3 1.5", center: "8 1.5 0.5 1.5" };
const ACCENT = "#6a4cdb";
const HOLE_FITS = ["H6", "H7", "H8", "H9", "H10", "H11", "G7", "F8", "JS7", "K7", "M7", "N7", "P7"];
const SHAFT_FITS = ["h6", "h7", "h8", "h9", "h11", "g6", "f7", "e8", "d9", "js6", "k6", "m6", "n6", "p6"];
const DETAIL_SCALES: { label: string; value: number | null }[] = [
  { label: "Auto", value: null },
  { label: "2:1", value: 2 },
  { label: "5:1", value: 5 },
  { label: "10:1", value: 10 },
];

/** The ISO 1101 characteristics, with how each symbol is drawn (strokes in a box about 0,0, y up, like the engine). */
const SYMBOLS: { id: GdtSymbol; name: string; group: string; d: string }[] = (() => {
  const ring = (r: number, cx = 0, cy = 0) => `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0`;
  const s60 = Math.sin(Math.PI / 3);
  const c60 = Math.cos(Math.PI / 3);
  const slant = (x: number, y = 0, l = 2.1) => `M${x - c60 * l} ${y - s60 * l}L${x + c60 * l} ${y + s60 * l}`;
  const cyl = () => {
    const r = 1.35;
    const n = [-s60, c60];
    return ring(r) + slant(n[0]! * -r, n[1]! * -r) + slant(n[0]! * r, n[1]! * r);
  };
  const arrow = (x: number) => `M${x - 1.2} -1.6L${x + 1.4} 1M${x + 1.4} 1l${1.2 * Math.cos(Math.PI * 1.25 - 0.4)} ${1.2 * Math.sin(Math.PI * 1.25 - 0.4)}M${x + 1.4} 1l${1.2 * Math.cos(Math.PI * 1.25 + 0.4)} ${1.2 * Math.sin(Math.PI * 1.25 + 0.4)}`;
  return [
    { id: "flatness", name: "Flatness", group: "Form", d: "M-2.3 -1.1L1 -1.1L2.3 1.1L-1 1.1Z" },
    { id: "straightness", name: "Straightness", group: "Form", d: "M-2 0L2 0" },
    { id: "circularity", name: "Circularity", group: "Form", d: ring(1.75) },
    { id: "cylindricity", name: "Cylindricity", group: "Form", d: cyl() },
    { id: "line_profile", name: "Profile of a line", group: "Profile", d: "M2 -1A2 2 0 0 0 -2 -1" },
    { id: "surface_profile", name: "Profile of a surface", group: "Profile", d: "M2 -1A2 2 0 0 0 -2 -1Z" },
    { id: "perpendicularity", name: "Perpendicularity", group: "Orientation", d: "M0 -1.75L0 1.9M-2 -1.75L2 -1.75" },
    { id: "parallelism", name: "Parallelism", group: "Orientation", d: slant(-0.8) + slant(0.8) },
    { id: "angularity", name: "Angularity", group: "Orientation", d: `M${-2.1 + 4.4 * Math.cos(Math.PI / 6)} ${-1.5 + 4.4 * Math.sin(Math.PI / 6)}L-2.1 -1.5L2.3 -1.5` },
    { id: "position", name: "Position", group: "Location", d: ring(1.35) + "M-2.2 0L2.2 0M0 -2.2L0 2.2" },
    { id: "concentricity", name: "Concentricity", group: "Location", d: ring(0.9) + ring(1.9) },
    { id: "symmetry", name: "Symmetry", group: "Location", d: "M-1.4 1.1L1.4 1.1M-2.2 0L2.2 0M-1.4 -1.1L1.4 -1.1" },
    { id: "circular_runout", name: "Circular run-out", group: "Run-out", d: arrow(0) },
    { id: "total_runout", name: "Total run-out", group: "Run-out", d: arrow(-0.9) + arrow(1.1) + "M-2.1 -1.6L0.1 -1.6" },
  ];
})();
const NEEDS_DATUM = new Set<GdtSymbol>(["perpendicularity", "parallelism", "angularity", "circular_runout", "total_runout", "concentricity", "symmetry"]);

function SymbolIcon({ d }: { d: string }) {
  return (
    <svg className="gdt-symbol" viewBox="-3 -3 6 6" aria-hidden>
      <path d={d} transform="scale(1 -1)" fill="none" stroke="currentColor" strokeWidth={0.32} strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/** Lengths as the drawing writes them (whole mm, or to 0.1 / 0.01 when needed). */
const lengthText = (v: number) => {
  const r = Math.round(v * 100) / 100;
  return Number.isInteger(r) ? String(r) : Math.abs(r * 10 - Math.round(r * 10)) < 1e-9 ? r.toFixed(1) : r.toFixed(2);
};

/** Points worth snapping to on each view: line ends and corners, and hole centres (where centre marks cross). */
function snapPoints(d: Drawing) {
  const out: { view: ViewFrame; p: Pt }[] = [];
  for (const v of d.views) {
    const [x0, y0, x1, y1] = v.bounds;
    const inside = (p: Pt) => p[0] >= x0 - 0.05 && p[0] <= x1 + 0.05 && p[1] >= y0 - 0.05 && p[1] <= y1 + 0.05;
    const seen = new Set<string>();
    const add = (p: Pt) => {
      const k = `${p[0].toFixed(2)},${p[1].toFixed(2)}`;
      if (!seen.has(k) && inside(p)) {
        seen.add(k);
        out.push({ view: v, p });
      }
    };
    const centres: Pt[][] = [];
    for (const s of d.strokes) {
      if (s.pen === "visible" || s.pen === "hidden") {
        if (!s.points.every(inside)) continue;
        // Polylines along curves: their ends (and a straight line's ends) are the corners.
        add(s.points[0]!);
        add(s.points[s.points.length - 1]!);
      } else if (s.pen === "center" && s.points.length === 2) centres.push(s.points);
    }
    // A horizontal and a vertical centre line crossing: a hole's centre.
    for (const h of centres.filter((c) => Math.abs(c[0]![1] - c[1]![1]) < 1e-6))
      for (const w of centres.filter((c) => Math.abs(c[0]![0] - c[1]![0]) < 1e-6)) {
        const p: Pt = [w[0]![0], h[0]![1]];
        const within = (a: number, b: number, x: number) => x >= Math.min(a, b) - 1e-6 && x <= Math.max(a, b) + 1e-6;
        if (within(h[0]![0], h[1]![0], p[0]) && within(w[0]![1], w[1]![1], p[1])) add(p);
      }
  }
  return out;
}

/** The view's edges (visible and hidden line segments), for pointing at an edge anywhere along it. */
function viewEdges(d: Drawing) {
  const out: { view: ViewFrame; a: Pt; b: Pt }[] = [];
  for (const v of d.views) {
    const [x0, y0, x1, y1] = v.bounds;
    const inside = (p: Pt) => p[0] >= x0 - 0.05 && p[0] <= x1 + 0.05 && p[1] >= y0 - 0.05 && p[1] <= y1 + 0.05;
    for (const s of d.strokes) {
      if ((s.pen !== "visible" && s.pen !== "hidden") || !s.points.every(inside)) continue;
      for (let i = 1; i < s.points.length; i++) out.push({ view: v, a: s.points[i - 1]!, b: s.points[i]! });
    }
  }
  return out;
}

/** Which way a dimension goes and how far out, from where the pointer is (the usual CAD gesture). */
function placement(pa: Pt, pb: Pt, m: Pt): { kind: DimKind; offset: number } {
  const [minx, maxx, miny, maxy] = [Math.min(pa[0], pb[0]), Math.max(pa[0], pb[0]), Math.min(pa[1], pb[1]), Math.max(pa[1], pb[1])];
  const flatX = maxx - minx < 1e-6;
  const flatY = maxy - miny < 1e-6;
  const horizontal = () => ({ kind: "horizontal" as const, offset: m[1] >= maxy ? m[1] - maxy : m[1] < miny ? m[1] - miny : m[1] - maxy });
  const vertical = () => ({ kind: "vertical" as const, offset: m[0] <= minx ? minx - m[0] : m[0] > maxx ? maxx - m[0] : minx - m[0] });
  if (flatX) return vertical();
  if (flatY) return horizontal();
  const outY = m[1] > maxy || m[1] < miny;
  const outX = m[0] > maxx || m[0] < minx;
  if (outY && !outX) return horizontal();
  if (outX && !outY) return vertical();
  const l = Math.hypot(pb[0] - pa[0], pb[1] - pa[1]);
  const n: Pt = [-(pb[1] - pa[1]) / l, (pb[0] - pa[0]) / l];
  return { kind: "aligned", offset: (m[0] - pa[0]) * n[0] + (m[1] - pa[1]) * n[1] };
}

/** The dimension line's two ends on the sheet (as the engine draws it). */
function dimLine(pa: Pt, pb: Pt, kind: DimKind, offset: number): [Pt, Pt] {
  let n: Pt;
  if (kind === "horizontal") n = [0, 1];
  else if (kind === "vertical") n = [-1, 0];
  else {
    const l = Math.hypot(pb[0] - pa[0], pb[1] - pa[1]) || 1;
    n = [-(pb[1] - pa[1]) / l, (pb[0] - pa[0]) / l];
  }
  const dot = (p: Pt) => p[0] * n[0] + p[1] * n[1];
  const level = offset >= 0 ? Math.max(dot(pa), dot(pb)) + offset : Math.min(dot(pa), dot(pb)) + offset;
  const foot = (p: Pt): Pt => [p[0] + n[0] * (level - dot(p)), p[1] + n[1] * (level - dot(p))];
  return [foot(pa), foot(pb)];
}

const toModel = (v: ViewFrame, p: Pt): Pt => [(p[0] - v.at[0]) / v.scale, (p[1] - v.at[1]) / v.scale];
const measure = (a: Pt, b: Pt, kind: DimKind) => (kind === "horizontal" ? Math.abs(a[0] - b[0]) : kind === "vertical" ? Math.abs(a[1] - b[1]) : Math.hypot(a[0] - b[0], a[1] - b[1]));
const round3 = (p: Pt): Pt => [Math.round(p[0] * 1000) / 1000, Math.round(p[1] * 1000) / 1000];
/** Letters for datums: A, B, C… leaving out I, O and Q (ISO 5459). */
const DATUM_LETTERS = "ABCDEFGHJKLMNPRSTUVWXYZ".split("");

type Popover = { at: Pt } & ({ kind: "dimension"; index: number } | { kind: "hole"; diameter: number } | { kind: "frame"; view: ViewName; p: Pt } | { kind: "annotation"; mark: AnnotationMark });

export function DrawingView(props: {
  id: string;
  name: string;
  parts: string[];
  setups: DrawingSetup[];
  /** The shape's configurations, and the one it shows: the drawing can be of any of them. */
  configs?: string[];
  activeConfig?: string | null;
  onSave: (setup: DrawingSetup) => Promise<unknown>;
  onClose: () => void;
}) {
  const [sheet, setSheet] = useState<(typeof SHEETS)[number]>("A3");
  const [projection, setProjection] = useState<"first" | "third">("first");
  const [part, setPart] = useState("");
  const [config, setConfig] = useState(props.activeConfig ?? "");
  const [drawing, setDrawing] = useState<Drawing | null>(null);
  const [status, setStatus] = useState<{ kind: "busy" } | { kind: "idle" } | { kind: "error"; text: string }>({ kind: "busy" });
  const [tool, setTool] = useState<Tool>("none");
  const [pending, setPending] = useState<{ view: ViewFrame; a: Pt; b: Pt | null } | null>(null);
  const [drag, setDrag] = useState<{ view: ViewFrame; c: Pt; r: number } | null>(null);
  const [detailScale, setDetailScale] = useState<number | null>(null);
  const [pointer, setPointer] = useState<Pt | null>(null);
  const [hover, setHover] = useState<string | null>(null);
  const [popover, setPopover] = useState<Popover | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const deskRef = useRef<HTMLDivElement>(null);

  const setup: DrawingSetup = useMemo(() => props.setups.find((s) => (s.part ?? "") === part) ?? { part: part || null, section: null, dimensions: [] }, [props.setups, part]);
  const details = setup.details ?? [];
  const frames = setup.frames ?? [];
  const datums = setup.datums ?? [];
  const holeFits = setup.hole_fits ?? [];
  const [depth, setDepth] = useState<number>(setup.section ?? 50);
  useEffect(() => setDepth(setup.section ?? 50), [setup.section]);
  const query = `sheet=${sheet}&projection=${projection}${part ? `&part=${encodeURIComponent(part)}` : ""}${config ? `&config=${encodeURIComponent(config)}` : ""}`;
  const setupKey = JSON.stringify(setup);

  useEffect(() => {
    let live = true;
    setStatus({ kind: "busy" });
    fetch(projectUrl(`/drawing/${props.id}?${query}`))
      .then(async (r) => {
        if (!live) return;
        if (r.ok) {
          setDrawing((await r.json()) as Drawing);
          setStatus({ kind: "idle" });
        } else setStatus({ kind: "error", text: await r.text() });
      })
      .catch(() => live && setStatus({ kind: "error", text: "The engine did not answer" }));
    return () => {
      live = false;
    };
  }, [props.id, query, setupKey]);

  const save = useCallback((next: Partial<DrawingSetup>) => props.onSave({ ...setup, part: part || null, ...next }), [props, setup, part]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      if (popover) setPopover(null);
      else if (pending) setPending(null);
      else if (drag) setDrag(null);
      else if (tool !== "none") setTool("none");
      else props.onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [props, pending, drag, tool, popover]);

  const d = drawing;
  const snaps = useMemo(() => (d ? snapPoints(d) : []), [d]);
  const edges = useMemo(() => (d ? viewEdges(d) : []), [d]);
  /** The pointer on the sheet (mm, y up). */
  const sheetPoint = (e: React.MouseEvent): Pt | null => {
    const svg = svgRef.current;
    const m = svg?.getScreenCTM();
    if (!svg || !m || !d) return null;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
    return [p.x, d.height - p.y];
  };
  /** Where a popover goes: by the pointer, inside the desk. */
  const deskPoint = (e: React.MouseEvent): Pt => {
    const r = deskRef.current?.getBoundingClientRect();
    return r ? [e.clientX - r.left, e.clientY - r.top] : [e.clientX, e.clientY];
  };
  const mmPerPx = d && svgRef.current ? d.width / svgRef.current.getBoundingClientRect().width : 0.3;
  const snapNear = (p: Pt | null, view?: ViewFrame) => {
    if (!p) return null;
    let best: { view: ViewFrame; p: Pt } | null = null;
    let dist = 10 * mmPerPx;
    for (const s of snaps) {
      if (view && s.view.view !== view.view) continue;
      const k = Math.hypot(s.p[0] - p[0], s.p[1] - p[1]);
      if (k < dist) [best, dist] = [s, k];
    }
    return best;
  };
  /** A corner or hole centre if one is near, else the nearest point along an edge. */
  const snapEdge = (p: Pt | null) => {
    if (!p) return null;
    const corner = snapNear(p);
    if (corner) return corner;
    let best: { view: ViewFrame; p: Pt } | null = null;
    let dist = 8 * mmPerPx;
    for (const e of edges) {
      const ab: Pt = [e.b[0] - e.a[0], e.b[1] - e.a[1]];
      const l2 = ab[0] * ab[0] + ab[1] * ab[1];
      const t = l2 ? Math.max(0, Math.min(1, ((p[0] - e.a[0]) * ab[0] + (p[1] - e.a[1]) * ab[1]) / l2)) : 0;
      const q: Pt = [e.a[0] + t * ab[0], e.a[1] + t * ab[1]];
      const k = Math.hypot(q[0] - p[0], q[1] - p[1]);
      if (k < dist) [best, dist] = [{ view: e.view, p: q }, k];
    }
    return best;
  };
  const viewAt = (p: Pt) => d?.views.find((v) => p[0] >= v.bounds[0] - 4 && p[0] <= v.bounds[2] + 4 && p[1] >= v.bounds[1] - 4 && p[1] <= v.bounds[3] + 4) ?? null;
  const snapped = tool === "dimension" && !pending?.b ? snapNear(pointer, pending?.view) : tool === "datum" || tool === "frame" ? snapEdge(pointer) : tool === "detail" && !drag && pointer ? snapNear(pointer) : null;

  const onClick = (e: React.MouseEvent) => {
    if (!d || popover) {
      setPopover(null);
      return;
    }
    const p = sheetPoint(e);
    if (!p) return;
    if (tool === "datum") {
      const s = snapEdge(p);
      if (!s) return;
      const used = new Set(datums.map((x) => x.letter));
      const letter = DATUM_LETTERS.find((l) => !used.has(l)) ?? "Z";
      void save({ datums: [...datums, { view: s.view.view, at: round3(toModel(s.view, s.p)), letter }] });
      return;
    }
    if (tool === "frame") {
      const s = snapEdge(p);
      if (s) setPopover({ kind: "frame", view: s.view.view, p: round3(toModel(s.view, s.p)), at: deskPoint(e) });
      return;
    }
    if (tool !== "dimension") return;
    if (!pending) {
      const s = snapNear(p);
      if (s) setPending({ view: s.view, a: s.p, b: null });
    } else if (!pending.b) {
      const s = snapNear(p, pending.view);
      if (s && Math.hypot(s.p[0] - pending.a[0], s.p[1] - pending.a[1]) > 1e-6) setPending({ ...pending, b: s.p });
    } else {
      const { kind, offset } = placement(pending.a, pending.b, p);
      const spec: DimSpec = { view: pending.view.view, a: toModel(pending.view, pending.a), b: toModel(pending.view, pending.b), kind, offset: Math.round(offset * 10) / 10 };
      setPending(null);
      void save({ dimensions: [...setup.dimensions, spec] });
    }
  };

  // The detail tool: press at the centre, drag out the radius.
  const onMouseDown = (e: React.MouseEvent) => {
    if (tool !== "detail" || !d || popover || e.button !== 0) return;
    const p = sheetPoint(e);
    if (!p) return;
    const s = snapNear(p);
    const view = s?.view ?? viewAt(p);
    if (view) setDrag({ view, c: s?.p ?? p, r: 0 });
  };
  const onMouseUp = () => {
    if (!drag) return;
    const r = drag.r;
    setDrag(null);
    if (r < 1.5) return;
    const spec: DetailSpec = { view: drag.view.view, center: round3(toModel(drag.view, drag.c)), radius: Math.round((r / drag.view.scale) * 100) / 100, scale: detailScale };
    void save({ details: [...details, spec] });
  };

  const svg = useMemo(() => {
    if (!d) return null;
    const flip = (p: Pt) => `${p[0].toFixed(3)},${(d.height - p[1]).toFixed(3)}`;
    return (
      <g>
        <rect width={d.width} height={d.height} fill="#fff" />
        {d.strokes.map((s, i) => (
          <polyline key={i} points={s.points.map(flip).join(" ")} fill="none" stroke="#1d1a24" strokeWidth={WIDTH[s.pen]} strokeDasharray={DASH[s.pen]} strokeLinecap="round" strokeLinejoin="round" />
        ))}
        {d.fills.map((f, i) => (
          <polygon key={i} points={f.map(flip).join(" ")} fill="#1d1a24" />
        ))}
        {d.texts.map((t, i) => (
          <text
            key={i}
            x={t.at[0]}
            y={d.height - t.at[1]}
            fontSize={t.size / 0.718}
            fontWeight={t.bold ? 700 : 400}
            textAnchor={t.anchor}
            transform={t.angle ? `rotate(${-t.angle} ${t.at[0]} ${d.height - t.at[1]})` : undefined}
            fill="#1d1a24"
            fontFamily="Helvetica, Arial, sans-serif"
          >
            {t.text}
          </text>
        ))}
      </g>
    );
  }, [d]);

  const busyGesture = !!pending || !!drag;
  // Hit boxes over what the person added: click to edit or remove.
  const markBox = (key: string, b: Box, title: string, open: (e: React.MouseEvent) => void) => {
    if (!d) return null;
    const on = hover === key;
    return (
      <rect
        key={key}
        className="drawing-mark"
        x={b[0] - 1}
        y={d.height - b[3] - 1}
        width={b[2] - b[0] + 2}
        height={b[3] - b[1] + 2}
        rx={1}
        fill={on ? "rgb(106 76 219 / 0.08)" : "transparent"}
        stroke={on ? ACCENT : "none"}
        strokeWidth={0.3}
        onMouseEnter={() => !busyGesture && setHover(key)}
        onMouseLeave={() => setHover(null)}
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          if (busyGesture) return;
          e.stopPropagation();
          setHover(null);
          open(e);
        }}
      >
        <title>{title}</title>
      </rect>
    );
  };

  const overlay = (() => {
    if (!d) return null;
    const y = (p: Pt) => d.height - p[1];
    const parts: React.ReactNode[] = [];
    if (tool !== "none") {
      for (const v of d.views) {
        const [x0, y0, x1, y1] = v.bounds;
        const active = (!pending || pending.view.view === v.view) && (!drag || drag.view.view === v.view);
        parts.push(<rect key={`v-${v.view}`} x={x0 - 2} y={d.height - y1 - 2} width={x1 - x0 + 4} height={y1 - y0 + 4} rx={1.5} fill="none" stroke={ACCENT} strokeOpacity={active ? 0.35 : 0.1} strokeWidth={0.3} strokeDasharray="1.5 1" pointerEvents="none" />);
      }
    }
    if (!busyGesture) {
      for (const m of d.marks) parts.push(markBox(`d-${m.dimension}`, m.bounds, "Tolerance, or remove this dimension", (e) => setPopover({ kind: "dimension", index: m.dimension, at: deskPoint(e) })));
      for (const a of d.annotations ?? []) {
        const key = `${a.kind}-${a.index}-${a.diameter ?? ""}`;
        if (a.kind === "hole") parts.push(markBox(key, a.bounds, "Set a fit on these holes", (e) => setPopover({ kind: "hole", diameter: a.diameter ?? 0, at: deskPoint(e) })));
        else parts.push(markBox(key, a.bounds, a.kind === "detail" ? "Detail view: change its scale or remove it" : `Remove this ${a.kind === "frame" ? "feature control frame" : "datum"}`, (e) => setPopover({ kind: "annotation", mark: a, at: deskPoint(e) })));
      }
    }
    if (pending) {
      parts.push(<circle key="a" cx={pending.a[0]} cy={y(pending.a)} r={1.1} fill={ACCENT} />);
      const b = pending.b ?? snapped?.p ?? pointer;
      if (b && pending.b) {
        parts.push(<circle key="b" cx={b[0]} cy={y(b)} r={1.1} fill={ACCENT} />);
        if (pointer) {
          const { kind, offset } = placement(pending.a, pending.b, pointer);
          const [ea, eb] = dimLine(pending.a, pending.b, kind, offset);
          const value = measure(toModel(pending.view, pending.a), toModel(pending.view, pending.b), kind);
          const mid: Pt = [(ea[0] + eb[0]) / 2, (ea[1] + eb[1]) / 2];
          let angle = (Math.atan2(eb[1] - ea[1], eb[0] - ea[0]) * 180) / Math.PI;
          if (angle > 90) angle -= 180;
          else if (angle <= -90) angle += 180;
          parts.push(
            <g key="preview" stroke={ACCENT} strokeWidth={0.3} fill="none" pointerEvents="none">
              <line x1={pending.a[0]} y1={y(pending.a)} x2={ea[0]} y2={y(ea)} strokeDasharray="1 0.8" />
              <line x1={b[0]} y1={y(b)} x2={eb[0]} y2={y(eb)} strokeDasharray="1 0.8" />
              <line x1={ea[0]} y1={y(ea)} x2={eb[0]} y2={y(eb)} strokeWidth={0.4} />
              <text x={mid[0]} y={y(mid) - 1.4} fontSize={4.2} textAnchor="middle" fill={ACCENT} stroke="#fff" strokeWidth={0.8} paintOrder="stroke" fontFamily="Helvetica, Arial, sans-serif" transform={angle ? `rotate(${-angle} ${mid[0]} ${y(mid)})` : undefined}>
                {lengthText(value)}
              </text>
            </g>,
          );
        }
      } else if (b) {
        parts.push(<line key="rubber" x1={pending.a[0]} y1={y(pending.a)} x2={b[0]} y2={y(b)} stroke={ACCENT} strokeWidth={0.3} strokeDasharray="1 0.8" pointerEvents="none" />);
      }
    }
    if (drag) {
      const label = `R${lengthText(drag.r / drag.view.scale)}`;
      parts.push(
        <g key="detail" pointerEvents="none">
          <circle cx={drag.c[0]} cy={y(drag.c)} r={Math.max(drag.r, 0.01)} fill="rgb(106 76 219 / 0.06)" stroke={ACCENT} strokeWidth={0.35} />
          <circle cx={drag.c[0]} cy={y(drag.c)} r={0.8} fill={ACCENT} />
          {drag.r > 2 && (
            <text x={drag.c[0] + drag.r * 0.71 + 1.5} y={y(drag.c) - drag.r * 0.71 - 1.5} fontSize={4} fill={ACCENT} stroke="#fff" strokeWidth={0.8} paintOrder="stroke" fontFamily="Helvetica, Arial, sans-serif">
              {label}
            </text>
          )}
        </g>,
      );
    }
    if (snapped) parts.push(<circle key="snap" cx={snapped.p[0]} cy={y(snapped.p)} r={1.6} fill="none" stroke={ACCENT} strokeWidth={0.45} pointerEvents="none" />);
    if (popover?.kind === "frame") {
      const v = d.views.find((x) => x.view === popover.view);
      if (v) {
        const q: Pt = [v.at[0] + popover.p[0] * v.scale, v.at[1] + popover.p[1] * v.scale];
        parts.push(<circle key="frame-at" cx={q[0]} cy={y(q)} r={1.1} fill={ACCENT} pointerEvents="none" />);
      }
    }
    return <g>{parts}</g>;
  })();

  const nextDatum = DATUM_LETTERS.find((l) => !datums.some((x) => x.letter === l)) ?? "Z";
  const hint =
    tool === "dimension"
      ? !pending
        ? "Click a corner or a hole centre on any view · click a dimension to give it a tolerance or remove it · Esc to finish"
        : !pending.b
          ? "Click the second point on the same view · Esc to cancel"
          : "Move to choose across, up or along, and how far out · click to place"
      : tool === "detail"
        ? drag
          ? "Release to place the detail view where the sheet has room"
          : "Press on a view at the detail's centre and drag out its circle · Esc to finish"
        : tool === "datum"
          ? `Click an edge (or a corner) to put datum ${nextDatum} on it · Esc to finish`
          : tool === "frame"
            ? "Click the edge or point the tolerance applies to · Esc to finish"
            : null;

  const pickTool = (t: Tool) => {
    setTool((now) => (now === t ? "none" : t));
    setPending(null);
    setDrag(null);
    setPopover(null);
  };
  const warnings = d?.warnings ?? [];

  return createPortal(
    <div className="drawing-view" role="dialog" aria-label="Engineering drawing">
      <header className="drawing-bar">
        <div className="drawing-title">
          <strong>Drawing</strong>
          <span>{part || (config ? `${props.name} (${config})` : props.name)}</span>
          {d && <small>Scale {d.scale_text}</small>}
        </div>
        <div className="drawing-controls">
          {(props.configs?.length ?? 0) > 0 && (
            <select value={config} onChange={(e) => setConfig(e.target.value)} aria-label="Which configuration">
              {!props.activeConfig && <option value="">As it is</option>}
              {props.configs!.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          )}
          {props.parts.length > 0 && (
            <select value={part} onChange={(e) => setPart(e.target.value)} aria-label="Which part">
              <option value="">Whole assembly</option>
              {props.parts.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          )}
          <div className="drawing-section">
            <button type="button" className={`chip-toggle ${setup.section != null ? "active" : ""}`} aria-pressed={setup.section != null} onClick={() => void save({ section: setup.section != null ? null : depth })} title="Show the front view cut through, with the cut faces hatched">
              <Icon name="layers" /> Section A–A
            </button>
            {setup.section != null && (
              <label className="drawing-depth" title="How far back the cut is, through the part's depth">
                <input type="range" min={5} max={95} step={1} value={depth} onChange={(e) => setDepth(Number(e.target.value))} onPointerUp={() => depth !== setup.section && void save({ section: depth })} onKeyUp={() => depth !== setup.section && void save({ section: depth })} aria-label="Section depth" />
                <span>{depth}%</span>
              </label>
            )}
          </div>
          <div className="drawing-tools" role="group" aria-label="Annotate">
            <button type="button" className={`chip-toggle ${tool === "dimension" ? "active" : ""}`} aria-pressed={tool === "dimension"} onClick={() => pickTool("dimension")} title="Add dimensions by clicking points on the views; click one to give it a tolerance or fit">
              <Icon name="ruler" /> Dimension{setup.dimensions.length ? ` · ${setup.dimensions.length}` : ""}
            </button>
            <button type="button" className={`chip-toggle ${tool === "detail" ? "active" : ""}`} aria-pressed={tool === "detail"} onClick={() => pickTool("detail")} title="Drag a circle on a view to show it enlarged in a detail view">
              <svg className="chip-glyph" viewBox="0 0 16 16" aria-hidden>
                <circle cx="5.5" cy="10.5" r="3.5" />
                <circle cx="11" cy="5" r="4.2" />
              </svg>
              Detail{details.length ? ` · ${details.length}` : ""}
            </button>
            {tool === "detail" && (
              <div className="segmented small" role="group" aria-label="Detail scale">
                {DETAIL_SCALES.map((s) => (
                  <button key={s.label} type="button" className={detailScale === s.value ? "active" : ""} aria-pressed={detailScale === s.value} onClick={() => setDetailScale(s.value)} title={s.value ? `Draw new details at ${s.label}` : "The largest standard scale that finds room on the sheet"}>
                    {s.label}
                  </button>
                ))}
              </div>
            )}
            <button type="button" className={`chip-toggle ${tool === "datum" ? "active" : ""}`} aria-pressed={tool === "datum"} onClick={() => pickTool("datum")} title="Mark a datum feature (A, B, C) on an edge">
              <svg className="chip-glyph" viewBox="0 0 16 16" aria-hidden>
                <path d="M4.5 15h7l-3.5-3.5z" className="solid" />
                <path d="M8 11.5V8" />
                <rect x="4" y="1" width="8" height="7" />
              </svg>
              Datum{datums.length ? ` · ${datums.length}` : ""}
            </button>
            <button type="button" className={`chip-toggle ${tool === "frame" ? "active" : ""}`} aria-pressed={tool === "frame"} onClick={() => pickTool("frame")} title="Add a geometrical tolerance (ISO 1101 feature control frame)">
              <svg className="chip-glyph" viewBox="0 0 16 16" aria-hidden>
                <rect x="1" y="4.5" width="14" height="7" />
                <path d="M5.5 4.5v7M10.5 4.5v7M3.25 6v4M1.9 10h2.7" />
              </svg>
              GD&amp;T{frames.length ? ` · ${frames.length}` : ""}
            </button>
          </div>
          <div className="segmented" role="group" aria-label="Sheet size">
            {SHEETS.map((s) => (
              <button key={s} type="button" className={s === sheet ? "active" : ""} aria-pressed={s === sheet} onClick={() => setSheet(s)}>
                {s}
              </button>
            ))}
          </div>
          <div className="segmented" role="group" aria-label="Projection">
            <button type="button" className={projection === "first" ? "active" : ""} aria-pressed={projection === "first"} onClick={() => setProjection("first")} title="First angle (ISO, Europe): top view below, right view on the left">
              First angle
            </button>
            <button type="button" className={projection === "third" ? "active" : ""} aria-pressed={projection === "third"} onClick={() => setProjection("third")} title="Third angle (ASME, America): top view above, right view on the right">
              Third angle
            </button>
          </div>
          <a className={`download primary ${d ? "" : "disabled"}`} href={projectUrl(`/drawing/${props.id}.pdf?${query}`)} download aria-disabled={!d}>
            <Icon name="download" /> PDF
          </a>
          <a className={`download ${d ? "" : "disabled"}`} href={projectUrl(`/drawing/${props.id}.dxf?${query}`)} download aria-disabled={!d} title="Lines on layers VISIBLE, HIDDEN, CENTER, DIMENSIONS, FRAME and TEXT">
            <Icon name="download" /> DXF
          </a>
          <button type="button" className="drawing-close" onClick={props.onClose} aria-label="Close the drawing">
            <Icon name="close" />
          </button>
        </div>
      </header>
      <div ref={deskRef} className={`drawing-desk ${tool !== "none" ? "measuring" : ""}`}>
        {status.kind === "busy" && !d && (
          <div className="drawing-wait">
            <span className="spinner" aria-hidden />
            Drawing the views from the exact solid…
          </div>
        )}
        {status.kind === "error" && <div className="drawing-wait bad">{status.text}</div>}
        {d && (
          <svg
            ref={svgRef}
            className={`drawing-sheet ${status.kind === "busy" ? "updating" : ""}`}
            viewBox={`0 0 ${d.width} ${d.height}`}
            role="img"
            aria-label={`Drawing of ${part || props.name}`}
            onMouseMove={(e) => {
              if (tool === "none") return;
              const p = sheetPoint(e);
              setPointer(p);
              if (drag && p) setDrag({ ...drag, r: Math.hypot(p[0] - drag.c[0], p[1] - drag.c[1]) });
            }}
            onMouseLeave={() => setPointer(null)}
            onMouseDown={onMouseDown}
            onMouseUp={onMouseUp}
            onClick={onClick}
          >
            {svg}
            {overlay}
          </svg>
        )}
        {warnings.length > 0 && (
          <div className="drawing-warnings" role="status">
            <Icon name="alert" /> {warnings.join(" · ")}
          </div>
        )}
        {hint && !popover && (
          <div className="drawing-hint" role="status">
            {hint}
          </div>
        )}
        {popover && (
          <div className="drawing-pop" style={popoverPlace(popover.at, deskRef.current)} onClick={(e) => e.stopPropagation()} onMouseDown={(e) => e.stopPropagation()}>
            {popover.kind === "dimension" && setup.dimensions[popover.index] && (
              <ToleranceForm
                dim={setup.dimensions[popover.index]!}
                onApply={(dim) => {
                  setPopover(null);
                  void save({ dimensions: setup.dimensions.map((x, i) => (i === popover.index ? dim : x)) });
                }}
                onRemove={() => {
                  setPopover(null);
                  void save({ dimensions: setup.dimensions.filter((_, i) => i !== popover.index) });
                }}
              />
            )}
            {popover.kind === "hole" && (
              <HoleFitForm
                diameter={popover.diameter}
                fit={holeFits.find((h) => Math.abs(h.diameter - popover.diameter) < 1e-3)?.fit ?? ""}
                onApply={(fit) => {
                  setPopover(null);
                  const rest = holeFits.filter((h) => Math.abs(h.diameter - popover.diameter) >= 1e-3);
                  void save({ hole_fits: fit ? [...rest, { diameter: popover.diameter, fit }] : rest });
                }}
              />
            )}
            {popover.kind === "frame" && (
              <FrameForm
                datums={[...new Set(datums.map((x) => x.letter))]}
                onApply={(f) => {
                  setPopover(null);
                  void save({ frames: [...frames, { ...f, view: popover.view, at: popover.p }] });
                }}
              />
            )}
            {popover.kind === "annotation" && (
              <AnnotationForm
                mark={popover.mark}
                detail={popover.mark.kind === "detail" ? details[popover.mark.index] : undefined}
                frame={popover.mark.kind === "frame" ? frames[popover.mark.index] : undefined}
                datum={popover.mark.kind === "datum" ? datums[popover.mark.index] : undefined}
                onScale={(scale) => {
                  setPopover(null);
                  void save({ details: details.map((x, i) => (i === popover.mark.index ? { ...x, scale } : x)) });
                }}
                onRemove={() => {
                  setPopover(null);
                  const i = popover.mark.index;
                  if (popover.mark.kind === "detail") void save({ details: details.filter((_, k) => k !== i) });
                  else if (popover.mark.kind === "frame") void save({ frames: frames.filter((_, k) => k !== i) });
                  else void save({ datums: datums.filter((_, k) => k !== i) });
                }}
              />
            )}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

/** A popover beside the pointer, turned back inside the desk near its right and bottom edges. */
function popoverPlace(at: Pt, desk: HTMLElement | null): React.CSSProperties {
  const w = desk?.clientWidth ?? 1e4;
  const h = desk?.clientHeight ?? 1e4;
  const left = at[0] + 12 + 310 > w ? Math.max(8, at[0] - 12 - 300) : at[0] + 12;
  const top = at[1] + 12 + 420 > h ? Math.max(8, Math.min(at[1] + 12, h - 430)) : at[1] + 12;
  return { left, top };
}

/** A number field that keeps what is typed until it parses. */
function NumberField(props: { label: string; value: number; onChange: (v: number) => void; step?: number; autoFocus?: boolean }) {
  const [text, setText] = useState(String(props.value));
  return (
    <label className="pop-field">
      <span>{props.label}</span>
      <input
        type="number"
        inputMode="decimal"
        step={props.step ?? 0.01}
        value={text}
        autoFocus={props.autoFocus}
        onChange={(e) => {
          setText(e.target.value);
          const v = Number(e.target.value);
          if (e.target.value.trim() !== "" && Number.isFinite(v)) props.onChange(v);
        }}
      />
    </label>
  );
}

function FitSelect(props: { value: string; onChange: (v: string) => void; none?: string; only?: "hole" | "shaft" }) {
  return (
    <select className="pop-select" value={props.value} onChange={(e) => props.onChange(e.target.value)} aria-label="ISO 286 fit">
      {props.none && <option value="">{props.none}</option>}
      {props.only !== "shaft" && (
        <optgroup label="Hole (basic hole H)">
          {HOLE_FITS.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </optgroup>
      )}
      {props.only !== "hole" && (
        <optgroup label="Shaft">
          {SHAFT_FITS.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </optgroup>
      )}
    </select>
  );
}

function ToleranceForm(props: { dim: DimSpec; onApply: (d: DimSpec) => void; onRemove: () => void }) {
  const t = props.dim.tolerance;
  const [kind, setKind] = useState<"none" | Tolerance["type"]>(t?.type ?? "none");
  const [pm, setPm] = useState(t?.type === "symmetric" ? t.value : 0.1);
  const [upper, setUpper] = useState(t?.type === "limits" ? t.upper : 0.1);
  const [lower, setLower] = useState(t?.type === "limits" ? t.lower : -0.1);
  const [fit, setFit] = useState(t?.type === "fit" ? t.fit : "H7");
  const [diameter, setDiameter] = useState(!!props.dim.diameter);
  const value = measure(props.dim.a, props.dim.b, props.dim.kind);
  const bad = (kind === "symmetric" && !(pm > 0)) || (kind === "limits" && !(upper > lower));
  const apply = () => {
    const tolerance: Tolerance | undefined = kind === "symmetric" ? { type: "symmetric", value: pm } : kind === "limits" ? { type: "limits", upper, lower } : kind === "fit" ? { type: "fit", fit } : undefined;
    const next: DimSpec = { ...props.dim, diameter };
    if (tolerance) next.tolerance = tolerance;
    else delete next.tolerance;
    if (!diameter) delete next.diameter;
    props.onApply(next);
  };
  return (
    <form
      className="pop-body"
      onSubmit={(e) => {
        e.preventDefault();
        if (!bad) apply();
      }}
    >
      <div className="pop-head">
        <strong>
          {diameter ? "⌀" : ""}
          {lengthText(value)}
        </strong>
        <span>Tolerance</span>
      </div>
      <div className="segmented small" role="group" aria-label="Tolerance">
        {(
          [
            ["none", "None"],
            ["symmetric", "±"],
            ["limits", "Limits"],
            ["fit", "Fit"],
          ] as const
        ).map(([k, label]) => (
          <button key={k} type="button" className={kind === k ? "active" : ""} aria-pressed={kind === k} onClick={() => setKind(k)}>
            {label}
          </button>
        ))}
      </div>
      {kind === "symmetric" && <NumberField label="±" value={pm} onChange={setPm} autoFocus />}
      {kind === "limits" && (
        <div className="pop-row">
          <NumberField label="Upper" value={upper} onChange={setUpper} autoFocus />
          <NumberField label="Lower" value={lower} onChange={setLower} />
        </div>
      )}
      {kind === "fit" && (
        <label className="pop-field">
          <span>ISO 286 class</span>
          <FitSelect value={fit} onChange={setFit} />
        </label>
      )}
      <label className="pop-check">
        <input type="checkbox" checked={diameter} onChange={(e) => setDiameter(e.target.checked)} /> Diameter (⌀)
      </label>
      <div className="pop-actions">
        <button type="button" className="pop-danger" onClick={props.onRemove}>
          <Icon name="trash" /> Remove
        </button>
        <button type="submit" className="pop-primary" disabled={bad}>
          Apply
        </button>
      </div>
    </form>
  );
}

function HoleFitForm(props: { diameter: number; fit: string; onApply: (fit: string) => void }) {
  const [fit, setFit] = useState(props.fit || "H7");
  return (
    <form
      className="pop-body"
      onSubmit={(e) => {
        e.preventDefault();
        props.onApply(fit);
      }}
    >
      <div className="pop-head">
        <strong>⌀{lengthText(props.diameter)}</strong>
        <span>Hole fit (ISO 286)</span>
      </div>
      <FitSelect value={fit} onChange={setFit} none="No fit" only="hole" />
      <div className="pop-actions">
        <span />
        <button type="submit" className="pop-primary">
          Apply
        </button>
      </div>
    </form>
  );
}

function FrameForm(props: { datums: string[]; onApply: (f: Omit<FrameSpec, "view" | "at">) => void }) {
  const [symbol, setSymbol] = useState<GdtSymbol>("flatness");
  const [tolerance, setTolerance] = useState(0.05);
  const [zone, setZone] = useState(false);
  const [modifier, setModifier] = useState<"" | "mmc" | "lmc">("");
  const [refs, setRefs] = useState<string[]>([]);
  const choices = [...new Set([...props.datums, "A", "B", "C"])].sort();
  const needs = NEEDS_DATUM.has(symbol) && refs.length === 0;
  const form = ["flatness", "straightness", "circularity", "cylindricity"].includes(symbol);
  return (
    <form
      className="pop-body wide"
      onSubmit={(e) => {
        e.preventDefault();
        if (!(tolerance > 0) || needs) return;
        props.onApply({ symbol, tolerance, diameter_zone: zone, datums: form ? [] : refs, modifier: modifier || null });
      }}
    >
      <div className="pop-head">
        <strong>Geometrical tolerance</strong>
        <span>ISO 1101</span>
      </div>
      <div className="gdt-grid" role="radiogroup" aria-label="Characteristic">
        {SYMBOLS.map((s) => (
          <button key={s.id} type="button" role="radio" aria-checked={symbol === s.id} className={symbol === s.id ? "active" : ""} title={`${s.name} (${s.group.toLowerCase()})`} onClick={() => (setSymbol(s.id), s.id === "position" && setZone(true))}>
            <SymbolIcon d={s.d} />
          </button>
        ))}
      </div>
      <div className="pop-sub">{SYMBOLS.find((s) => s.id === symbol)?.name}</div>
      <div className="pop-row">
        <NumberField label="Tolerance (mm)" value={tolerance} onChange={setTolerance} step={0.005} />
        <label className="pop-check">
          <input type="checkbox" checked={zone} onChange={(e) => setZone(e.target.checked)} /> ⌀ zone
        </label>
      </div>
      <div className="pop-row">
        <label className="pop-field">
          <span>Material condition</span>
          <select className="pop-select" value={modifier} onChange={(e) => setModifier(e.target.value as "" | "mmc" | "lmc")}>
            <option value="">None</option>
            <option value="mmc">Ⓜ Maximum material</option>
            <option value="lmc">Ⓛ Least material</option>
          </select>
        </label>
      </div>
      {!form && (
        <div className="pop-field">
          <span>Datums, in order{needs ? " (this one needs at least one)" : ""}</span>
          <div className="datum-picks">
            {[0, 1, 2].map((i) => (
              <select key={i} className="pop-select" value={refs[i] ?? ""} disabled={i > refs.length} onChange={(e) => setRefs((r) => (e.target.value ? [...r.slice(0, i), e.target.value] : r.slice(0, i)))} aria-label={`Datum ${i + 1}`}>
                <option value="">—</option>
                {choices.map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            ))}
          </div>
        </div>
      )}
      <div className="pop-actions">
        <span />
        <button type="submit" className="pop-primary" disabled={!(tolerance > 0) || needs}>
          Add frame
        </button>
      </div>
    </form>
  );
}

function AnnotationForm(props: { mark: AnnotationMark; detail?: DetailSpec; frame?: FrameSpec; datum?: DatumSpec; onScale: (s: number | null) => void; onRemove: () => void }) {
  const title = props.detail ? "Detail view" : props.frame ? (SYMBOLS.find((s) => s.id === props.frame!.symbol)?.name ?? "Frame") : props.datum ? `Datum ${props.datum.letter}` : "";
  return (
    <div className="pop-body">
      <div className="pop-head">
        <strong>{title}</strong>
        {props.detail && <span>R{lengthText(props.detail.radius)} on the {props.detail.view} view</span>}
        {props.frame && (
          <span>
            {props.frame.diameter_zone ? "⌀" : ""}
            {props.frame.tolerance}
            {props.frame.datums?.length ? ` · ${props.frame.datums.join(" ")}` : ""}
          </span>
        )}
      </div>
      {props.detail && (
        <div className="segmented small" role="group" aria-label="Detail scale">
          {DETAIL_SCALES.map((s) => (
            <button key={s.label} type="button" className={(props.detail!.scale ?? null) === s.value ? "active" : ""} onClick={() => props.onScale(s.value)}>
              {s.label}
            </button>
          ))}
        </div>
      )}
      <div className="pop-actions">
        <button type="button" className="pop-danger" onClick={props.onRemove}>
          <Icon name="trash" /> Remove
        </button>
        <span />
      </div>
    </div>
  );
}
