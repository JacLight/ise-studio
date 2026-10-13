// The view's toolbar, a short bar at the bottom centre of the view:
// orthographic/perspective, display mode, section planes and the measure tool, with their keyboard shortcuts
// (FreeCAD's for the standard views: 0 iso, 1 front, 2 top, 3 right, 4 back,
// 5 bottom, 6 left).

import { useEffect, useState, useSyncExternalStore } from "react";
import { ANALYSIS_MODES, DISPLAY_MODES } from "./display";
import type { Axis, DisplayMode, SectionPlane, ViewName, ViewTools } from "./viewTools";

const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

const KEY_VIEWS: Record<string, ViewName> = { "0": "iso", "1": "front", "2": "top", "3": "right", "4": "back", "5": "bottom", "6": "left" };

const DISPLAY_ICONS: Record<DisplayMode, string> = {
  shaded: "M3 5.5 8 3l5 2.5v5L8 13l-5-2.5z",
  edges: "M3 5.5 8 3l5 2.5v5L8 13l-5-2.5zM3 5.5 8 8l5-2.5M8 8v5",
  wireframe: "M3 5.5 8 3l5 2.5v5L8 13l-5-2.5zM3 5.5 8 8l5-2.5M8 8v5M3 10.5 8 8l5 2.5M8 3v5",
  hidden: "M3 5.5 8 3l5 2.5v5L8 13l-5-2.5zM3 5.5 8 8l5-2.5M8 8v5",
  xray: "M3 5.5 8 3l5 2.5v5L8 13l-5-2.5zM3 10.5 8 8l5 2.5M8 3v5",
  zebra: "M3 4h10M3 6.5h10M3 9h10M3 11.5h10",
  curvature: "M2.5 12C4 5 12 5 13.5 12M5 12c1-3 5-3 6 0",
  draft: "M5 13 6.5 3h3L11 13zM3 13h10",
};

export function ViewToolbar({ tools, selected, onFitAll }: { tools: ViewTools; selected: string | null; onFitAll: () => void }) {
  const s = useSyncExternalStore(tools.subscribe, tools.getState);
  const [sectionOpen, setSectionOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey || document.querySelector("dialog[open]")) return;
      if (e.key === "Escape" && tools.state.measuring) {
        if (tools.measure.picks.length) tools.measure.clear();
        else tools.setMeasuring(false);
        return;
      }
      const view = KEY_VIEWS[e.key];
      if (view) {
        e.preventDefault();
        tools.view(view);
      } else if (e.key === "o" || e.key === "O") {
        tools.setOrthographic(!tools.state.orthographic);
      } else if (e.key === "m" || e.key === "M") {
        tools.setMeasuring(!tools.state.measuring);
      } else if (e.key === "x" || e.key === "X") {
        setSectionOpen((o) => !o);
      } else if (e.key === "w" || e.key === "W") {
        // Cycle the display modes.
        const i = DISPLAY_MODES.findIndex((d) => d.mode === tools.state.display);
        tools.setDisplay(DISPLAY_MODES[(i + (e.shiftKey ? DISPLAY_MODES.length - 1 : 1)) % DISPLAY_MODES.length].mode);
      } else if (e.key === ".") {
        tools.fitSelection(selected);
      } else if (e.key === "Home") {
        onFitAll();
      } else return;
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tools, selected, onFitAll]);

  return (
    <div className="view-tools" onPointerDown={(e) => e.stopPropagation()}>
      <div className="view-tools-bar" role="toolbar" aria-label="View tools">
        <button type="button" className={`vt-btn vt-icon ${s.orthographic ? "is-on" : ""}`} onClick={() => tools.setOrthographic(!s.orthographic)} title={`${s.orthographic ? "Orthographic" : "Perspective"} (O to switch)`} aria-label={s.orthographic ? "Orthographic" : "Perspective"}>
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden>
            <path d={s.orthographic ? "M3.5 3.5h9v9h-9zM3.5 8h9M8 3.5v9" : "M5.5 3.5h5l3 9h-11zM4 8h8M8 3.5v9"} />
          </svg>
        </button>
        <span className="vt-sep" />
        {DISPLAY_MODES.map((d) => (
          <button key={d.mode} type="button" className={`vt-btn vt-icon ${s.display === d.mode ? "is-on" : ""}`} onClick={() => tools.setDisplay(d.mode)} title={`${d.label} (W cycles)`} aria-label={d.label} aria-pressed={s.display === d.mode}>
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden className={`vt-mode-${d.mode}`}>
              <path d={DISPLAY_ICONS[d.mode]} />
            </svg>
          </button>
        ))}
        <span className="vt-sep" />
        {ANALYSIS_MODES.map((d) => (
          <button key={d.mode} type="button" className={`vt-btn vt-icon ${s.display === d.mode ? "is-on" : ""}`} onClick={() => tools.setDisplay(s.display === d.mode ? "shaded" : d.mode)} title={d.label} aria-label={d.label} aria-pressed={s.display === d.mode}>
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden className={`vt-mode-${d.mode}`}>
              <path d={DISPLAY_ICONS[d.mode]} />
            </svg>
          </button>
        ))}
        <span className="vt-sep" />
        <button type="button" className={`vt-btn vt-icon ${sectionOpen || s.section.planes.length ? "is-on" : ""}`} onClick={() => setSectionOpen((o) => !o)} title="Section planes (X)" aria-label="Section planes" aria-expanded={sectionOpen}>
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden>
            <path d="M3 5.5 8 3l5 2.5v5L8 13l-5-2.5zM2 9.5l7-4 5 2.5" />
          </svg>
        </button>
        <button type="button" className={`vt-btn vt-icon ${s.measuring ? "is-on" : ""}`} onClick={() => tools.setMeasuring(!s.measuring)} title="Measure (M)" aria-label="Measure" aria-pressed={s.measuring}>
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden>
            <path d="M2 10.5 10.5 2 14 5.5 5.5 14zM5 7.5l1.5 1.5M7 5.5l1 1M9 3.5l1.5 1.5M3 9.5l1 1" />
          </svg>
        </button>
      </div>
      <div className="view-tools-cards">
      {sectionOpen && <SectionCard tools={tools} planes={s.section.planes} capped={s.section.capped} picking={s.section.picking} bounds={s.bounds ?? tools.boundsNow()} onClose={() => setSectionOpen(false)} />}
      {s.measuring && (
        <div className="vt-card measure-card" aria-live="polite">
          <div className="vt-card-head">
            <strong>Measure</strong>
            <button type="button" className="vt-link" title="Clear the measurements and start a new one" onClick={() => tools.measure.clear()} disabled={!s.card.length}>
              Clear
            </button>
            <button type="button" className="vt-close" onClick={() => tools.setMeasuring(false)} aria-label="Stop measuring">
              ×
            </button>
          </div>
          {!s.card.length && <p className="vt-hint">Click a corner, edge or face, then another. Alt-click a part for its mass and inertia.</p>}
          {s.card.map((group) => (
            <section key={group.title}>
              <h4>{group.title}</h4>
              <dl>
                {group.rows.map((r) => (
                  <div key={r.label}>
                    <dt>{r.label}</dt>
                    <dd>{r.value}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      )}
      </div>
    </div>
  );
}

function SectionCard(props: { tools: ViewTools; planes: SectionPlane[]; capped: boolean; picking: boolean; bounds: { min: number[]; max: number[] } | null; onClose: () => void }) {
  const { tools, planes, bounds } = props;
  const axes: Axis[] = ["x", "y", "z"];
  const boxOn = planes.some((p) => p.id.length === 2);
  return (
    <div className="vt-card section-card">
      <div className="vt-card-head">
        <strong>Section</strong>
        <button type="button" className="vt-link" title="Remove every section cut and show the whole model" onClick={() => tools.setSection([])} disabled={!planes.length}>
          Clear
        </button>
        <button type="button" className="vt-close" onClick={props.onClose} aria-label="Close the section card (the cuts stay)">
          ×
        </button>
      </div>
      {axes.map((axis, k) => {
        const p = planes.find((q) => q.id === axis);
        const lo = bounds ? bounds.min[k] : -100;
        const hi = bounds ? bounds.max[k] : 100;
        const mid = (lo + hi) / 2;
        const value = p ? p.at[k] : mid;
        const step = Math.max((hi - lo) / 500, 0.01);
        return (
          <div key={axis} className={`section-row ${p ? "is-on" : ""}`}>
            <label className="section-axis">
              <input type="checkbox" checked={!!p} title={p ? `Stop cutting across ${axis.toUpperCase()}` : `Cut the model across ${axis.toUpperCase()}, at its middle to start`} onChange={(e) => (e.target.checked ? tools.sectionAxis(axis, mid) : tools.setSection(planes.filter((q) => q.id !== axis)))} />
              {axis.toUpperCase()}
            </label>
            <input
              type="range"
              min={lo}
              max={hi}
              step={step}
              value={value}
              disabled={!p}
              title={`Slide the ${axis.toUpperCase()} cut through the model`}
              aria-label={`${axis.toUpperCase()} section offset`}
              onChange={(e) => tools.sectionAxis(axis, Number(e.target.value))}
            />
            <input type="number" className="section-num" value={Number(value.toFixed(2))} step={1} disabled={!p} aria-label={`${axis.toUpperCase()} offset (mm)`} onChange={(e) => tools.sectionAxis(axis, Number(e.target.value))} />
            <button type="button" className={`vt-btn vt-small ${p?.flip ? "is-on" : ""}`} disabled={!p} onClick={() => p && tools.sectionAxis(axis, p.at[k], !p.flip)} title={`Flip which side of the ${axis.toUpperCase()} plane is cut away`}>
              Flip
            </button>
          </div>
        );
      })}
      <div className="section-actions">
        <button type="button" className={`vt-btn vt-small ${boxOn ? "is-on" : ""}`} onClick={() => (boxOn ? tools.setSection(planes.filter((p) => p.id.length !== 2)) : tools.sectionBox())} title={boxOn ? "Remove the section box" : "Section box: six planes a fifth in from the model's sides (set each face with ./forma view section box)"}>
          Box
        </button>
        <button type="button" className={`vt-btn vt-small ${props.picking ? "is-on" : ""}`} onClick={() => tools.pickSectionFace(!props.picking)} title={props.picking ? "Stop picking a face to cut along" : "Cut along a face: click the face next"}>
          {props.picking ? "Click a face…" : "On a face"}
        </button>
        {planes.some((p) => p.id === "face") && (
          <button type="button" className="vt-btn vt-small" title="Flip which side of the face cut is cut away" onClick={() => tools.setSection(planes.map((p) => (p.id === "face" ? { ...p, flip: !p.flip } : p)))}>
            Flip face cut
          </button>
        )}
        <label className="section-cap">
          <input type="checkbox" checked={props.capped} title="Fill the cut faces solid, so you see the material and not the hollow inside" onChange={(e) => tools.setSection(planes, e.target.checked)} />
          Fill cuts
        </label>
      </div>
    </div>
  );
}
