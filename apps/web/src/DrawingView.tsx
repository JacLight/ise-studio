// A part's engineering drawing: the sheet the engine lays out (views from
// the exact solid, hidden lines dashed, dimensions, title block), drawn as
// SVG, with the sheet size, projection and part to choose, and PDF and DXF
// downloads of exactly what is shown. The front view can be cut through
// (section A–A), and dimensions added by clicking points on the views; both
// are kept with the part.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { projectUrl } from "./engine";
import { Icon } from "./icons";

type Pen = "visible" | "hidden" | "thin" | "center" | "frame";
type ViewName = "front" | "top" | "right";
type DimKind = "horizontal" | "vertical" | "aligned";
type Pt = [number, number];
type ViewFrame = { view: ViewName; at: Pt; scale: number; axes: [number, number]; bounds: [number, number, number, number] };
type Drawing = {
  width: number;
  height: number;
  scale: number;
  scale_text: string;
  strokes: { pen: Pen; points: Pt[] }[];
  fills: Pt[][];
  texts: { at: Pt; size: number; text: string; anchor: "start" | "middle" | "end"; angle: number; bold: boolean }[];
  views: ViewFrame[];
  marks: { dimension: number; bounds: [number, number, number, number] }[];
};
export type DimSpec = { view: ViewName; a: Pt; b: Pt; kind: DimKind; offset: number };
export type DrawingSetup = { part: string | null; section: number | null; dimensions: DimSpec[] };

const SHEETS = ["A4", "A3", "A2", "A1", "A0"] as const;
const WIDTH: Record<Pen, number> = { visible: 0.5, hidden: 0.25, thin: 0.25, center: 0.25, frame: 0.7 };
const DASH: Partial<Record<Pen, string>> = { hidden: "3 1.5", center: "8 1.5 0.5 1.5" };
const ACCENT = "#6a4cdb";

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

export function DrawingView(props: { id: string; name: string; parts: string[]; setups: DrawingSetup[]; onSave: (setup: DrawingSetup) => Promise<unknown>; onClose: () => void }) {
  const [sheet, setSheet] = useState<(typeof SHEETS)[number]>("A3");
  const [projection, setProjection] = useState<"first" | "third">("first");
  const [part, setPart] = useState("");
  const [drawing, setDrawing] = useState<Drawing | null>(null);
  const [status, setStatus] = useState<{ kind: "busy" } | { kind: "idle" } | { kind: "error"; text: string }>({ kind: "busy" });
  const [tool, setTool] = useState<"none" | "dimension">("none");
  const [pending, setPending] = useState<{ view: ViewFrame; a: Pt; b: Pt | null } | null>(null);
  const [pointer, setPointer] = useState<Pt | null>(null);
  const [hoverMark, setHoverMark] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  const setup: DrawingSetup = useMemo(() => props.setups.find((s) => (s.part ?? "") === part) ?? { part: part || null, section: null, dimensions: [] }, [props.setups, part]);
  const [depth, setDepth] = useState<number>(setup.section ?? 50);
  useEffect(() => setDepth(setup.section ?? 50), [setup.section]);
  const query = `sheet=${sheet}&projection=${projection}${part ? `&part=${encodeURIComponent(part)}` : ""}`;
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
      if (pending) setPending(null);
      else if (tool !== "none") setTool("none");
      else props.onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [props, pending, tool]);

  const d = drawing;
  const snaps = useMemo(() => (d ? snapPoints(d) : []), [d]);
  /** The pointer on the sheet (mm, y up). */
  const sheetPoint = (e: React.MouseEvent): Pt | null => {
    const svg = svgRef.current;
    const m = svg?.getScreenCTM();
    if (!svg || !m || !d) return null;
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
    return [p.x, d.height - p.y];
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
  const snapped = tool === "dimension" && !pending?.b ? snapNear(pointer, pending?.view) : null;

  const onClick = (e: React.MouseEvent) => {
    if (tool !== "dimension" || !d) return;
    const p = sheetPoint(e);
    if (!p) return;
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

  // What the dimension tool shows over the sheet.
  const overlay = (() => {
    if (!d || tool !== "dimension") return null;
    const y = (p: Pt) => d.height - p[1];
    const parts: React.ReactNode[] = [];
    for (const v of d.views) {
      const [x0, y0, x1, y1] = v.bounds;
      const active = !pending || pending.view.view === v.view;
      parts.push(<rect key={`v-${v.view}`} x={x0 - 2} y={d.height - y1 - 2} width={x1 - x0 + 4} height={y1 - y0 + 4} rx={1.5} fill="none" stroke={ACCENT} strokeOpacity={active ? 0.35 : 0.1} strokeWidth={0.3} strokeDasharray="1.5 1" />);
    }
    for (const m of d.marks) {
      const [x0, y0, x1, y1] = m.bounds;
      parts.push(
        <rect
          key={`m-${m.dimension}`}
          className="drawing-mark"
          x={x0 - 1}
          y={d.height - y1 - 1}
          width={x1 - x0 + 2}
          height={y1 - y0 + 2}
          rx={1}
          fill={hoverMark === m.dimension ? "rgb(220 50 60 / 0.12)" : "transparent"}
          stroke={hoverMark === m.dimension ? "#d23c4b" : "none"}
          strokeWidth={0.3}
          onMouseEnter={() => !pending && setHoverMark(m.dimension)}
          onMouseLeave={() => setHoverMark(null)}
          onClick={(e) => {
            if (pending) return;
            e.stopPropagation();
            setHoverMark(null);
            void save({ dimensions: setup.dimensions.filter((_, i) => i !== m.dimension) });
          }}
        >
          <title>Remove this dimension</title>
        </rect>,
      );
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
            <g key="preview" stroke={ACCENT} strokeWidth={0.3} fill="none">
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
        parts.push(<line key="rubber" x1={pending.a[0]} y1={y(pending.a)} x2={b[0]} y2={y(b)} stroke={ACCENT} strokeWidth={0.3} strokeDasharray="1 0.8" />);
      }
    }
    if (snapped) parts.push(<circle key="snap" cx={snapped.p[0]} cy={y(snapped.p)} r={1.6} fill="none" stroke={ACCENT} strokeWidth={0.45} />);
    return <g pointerEvents="visiblePainted">{parts}</g>;
  })();

  const hint =
    tool !== "dimension"
      ? null
      : !pending
        ? "Click a corner or a hole centre on any view · click a dimension you added to remove it · Esc to finish"
        : !pending.b
          ? "Click the second point on the same view · Esc to cancel"
          : "Move to choose across, up or along, and how far out · click to place";

  return createPortal(
    <div className="drawing-view" role="dialog" aria-label="Engineering drawing">
      <header className="drawing-bar">
        <div className="drawing-title">
          <strong>Drawing</strong>
          <span>{part || props.name}</span>
          {d && <small>Scale {d.scale_text}</small>}
        </div>
        <div className="drawing-controls">
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
          <button type="button" className={`chip-toggle ${tool === "dimension" ? "active" : ""}`} aria-pressed={tool === "dimension"} onClick={() => (setTool((t) => (t === "dimension" ? "none" : "dimension")), setPending(null))} title="Add dimensions by clicking points on the views">
            <Icon name="ruler" /> Dimension{setup.dimensions.length ? ` · ${setup.dimensions.length}` : ""}
          </button>
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
      <div className={`drawing-desk ${tool === "dimension" ? "measuring" : ""}`}>
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
            onMouseMove={(e) => tool === "dimension" && setPointer(sheetPoint(e))}
            onMouseLeave={() => setPointer(null)}
            onClick={onClick}
          >
            {svg}
            {overlay}
          </svg>
        )}
        {hint && (
          <div className="drawing-hint" role="status">
            {hint}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
