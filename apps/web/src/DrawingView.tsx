// A part's engineering drawing: the sheet the engine lays out (views from
// the exact solid, hidden lines dashed, dimensions, title block), drawn as
// SVG, with the sheet size, projection and part to choose, and PDF and DXF
// downloads of exactly what is shown.

import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { projectUrl } from "./engine";
import { Icon } from "./icons";

type Pen = "visible" | "hidden" | "thin" | "center" | "frame";
type Drawing = {
  width: number;
  height: number;
  scale: number;
  scale_text: string;
  strokes: { pen: Pen; points: [number, number][] }[];
  fills: [number, number][][];
  texts: { at: [number, number]; size: number; text: string; anchor: "start" | "middle" | "end"; angle: number; bold: boolean }[];
};

const SHEETS = ["A4", "A3", "A2", "A1", "A0"] as const;
const WIDTH: Record<Pen, number> = { visible: 0.5, hidden: 0.25, thin: 0.25, center: 0.25, frame: 0.7 };
const DASH: Partial<Record<Pen, string>> = { hidden: "3 1.5", center: "8 1.5 0.5 1.5" };

export function DrawingView(props: { id: string; name: string; parts: string[]; onClose: () => void }) {
  const [sheet, setSheet] = useState<(typeof SHEETS)[number]>("A3");
  const [projection, setProjection] = useState<"first" | "third">("first");
  const [part, setPart] = useState("");
  const [state, setState] = useState<{ kind: "busy" } | { kind: "done"; drawing: Drawing } | { kind: "error"; text: string }>({ kind: "busy" });
  const query = `sheet=${sheet}&projection=${projection}${part ? `&part=${encodeURIComponent(part)}` : ""}`;

  useEffect(() => {
    let live = true;
    setState({ kind: "busy" });
    fetch(projectUrl(`/drawing/${props.id}?${query}`))
      .then(async (r) => (r.ok ? { kind: "done" as const, drawing: (await r.json()) as Drawing } : { kind: "error" as const, text: await r.text() }))
      .catch(() => ({ kind: "error" as const, text: "The engine did not answer" }))
      .then((s) => live && setState(s));
    return () => {
      live = false;
    };
  }, [props.id, query]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        props.onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [props]);

  const d = state.kind === "done" ? state.drawing : null;
  const svg = useMemo(() => {
    if (!d) return null;
    const flip = (p: [number, number]) => `${p[0].toFixed(3)},${(d.height - p[1]).toFixed(3)}`;
    return (
      <svg className="drawing-sheet" viewBox={`0 0 ${d.width} ${d.height}`} role="img" aria-label={`Drawing of ${part || props.name}`}>
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
      </svg>
    );
  }, [d, part, props.name]);

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
      <div className="drawing-desk">
        {state.kind === "busy" && (
          <div className="drawing-wait">
            <span className="spinner" aria-hidden />
            Drawing the views from the exact solid…
          </div>
        )}
        {state.kind === "error" && <div className="drawing-wait bad">{state.text}</div>}
        {svg}
      </div>
    </div>,
    document.body,
  );
}
