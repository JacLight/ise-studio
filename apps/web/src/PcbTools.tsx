// Fab utilities in the Electronics workspace: the board calculators
// (GET /pcb/calc), panelization (GET /pcb/<id>/panel, panel.zip) and the
// Gerber viewer (POST /pcb/gerbview): any Gerber/Excellon set with its layers
// in colour and order, highlight by net, part or D-code, click to inspect,
// measure, the D-code list and source, two revisions compared, and export to
// PNG, SVG and PDF. The engine does the arithmetic and the reading; this
// draws and asks.

import { useEffect, useMemo, useRef, useState } from "react";
import { projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";
import "./PcbTools.css";

type P = [number, number];

function useStored<T>(key: string, initial: T): [T, (v: T) => void] {
  const [v, setV] = useState<T>(() => {
    try {
      const s = localStorage.getItem(key);
      return s ? { ...initial, ...JSON.parse(s) } : initial;
    } catch {
      return initial;
    }
  });
  const set = (x: T) => {
    setV(x);
    try {
      localStorage.setItem(key, JSON.stringify(x));
    } catch {
      /* private window */
    }
  };
  return [v, set];
}

function download(blob: Blob, name: string) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

/** The side panel's Tools tab: panelization and the calculators. */
export function ToolsTab(props: { boardId: string; revision: number; fab: string | null }) {
  const [which, setWhich] = useStored<{ tool: "panel" | "calc" }>("forma.pcb.tool", { tool: "panel" });
  return (
    <div className="pt-calc">
      <div className="pt-switch" role="tablist">
        <button type="button" role="tab" className={which.tool === "panel" ? "on" : ""} title="Panelize: lay out copies of this board on one panel with tabs, rails and fiducials" onClick={() => setWhich({ tool: "panel" })}>
          Panel
        </button>
        <button type="button" role="tab" className={which.tool === "calc" ? "on" : ""} title="PCB calculators: track width, impedance, vias and more" onClick={() => setWhich({ tool: "calc" })}>
          Calculators
        </button>
      </div>
      {which.tool === "panel" ? <PanelTab {...props} /> : <CalcTab />}
    </div>
  );
}

// ---- Calculators ---------------------------------------------------------------------

type CalcInput = { key: string; label: string; unit: string; default: string; options?: string[] };
type Calc = { id: string; title: string; about: string; inputs: CalcInput[] };
type Row = { label: string; value: number | string; unit: string };
type Table = { columns: string[]; rows: (string | number)[][] };
type Answer = { title: string; results: Row[]; notes: string[]; source?: string; bands?: { colour: string; hex: string; role: string }[]; values?: number[]; table?: Table; levels?: Table };

export function CalcTab() {
  const [list, setList] = useState<Calc[] | null>(null);
  const [id, setId] = useStored<{ id: string }>("forma.pcb.calc", { id: "track" });
  const [values, setValues] = useState<Record<string, Record<string, string>>>({});
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    fetch(projectUrl("/pcb/calc"))
      .then((r) => r.json())
      .then((r: { calculators: Calc[] }) => setList(r.calculators))
      .catch((e) => setError(String(e)));
  }, []);
  const calc = list?.find((c) => c.id === id.id) ?? list?.[0];
  const mine = calc ? { ...Object.fromEntries(calc.inputs.map((i) => [i.key, i.default])), ...(values[calc.id] ?? {}) } : {};
  const query = calc ? new URLSearchParams(mine).toString() : "";
  useEffect(() => {
    if (!calc) return;
    let live = true;
    const t = window.setTimeout(() => {
      fetch(projectUrl(`/pcb/calc/${calc.id}?${query}`))
        .then(async (r) => (r.ok ? r.json() : Promise.reject(new Error(await r.text()))))
        .then((a: Answer) => {
          if (!live) return;
          setAnswer(a);
          setError(null);
        })
        .catch((e) => live && setError(String(e.message ?? e)));
    }, 150);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [calc?.id, query]);
  if (!list) return error ? <p className="pcb-error">{error}</p> : <div className="pcb-spinner" />;
  if (!calc) return null;
  const set = (k: string, v: string) => setValues({ ...values, [calc.id]: { ...(values[calc.id] ?? {}), [k]: v } });
  return (
    <div className="pt-calc">
      <select className="pt-calc-pick" value={calc.id} onChange={(e) => setId({ id: e.target.value })} aria-label="Calculator">
        {list.map((c) => (
          <option key={c.id} value={c.id}>
            {c.title}
          </option>
        ))}
      </select>
      <p className="pcb-note">{calc.about}</p>
      {calc.inputs.length > 0 && (
        <div className="pt-form">
          {calc.inputs.map((i) => (
            <label key={i.key} className="pt-field">
              <span>{i.label}</span>
              {i.options ? (
                <select value={mine[i.key]} title={i.label} onChange={(e) => set(i.key, e.target.value)}>
                  {i.options.map((o) => (
                    <option key={o}>{o}</option>
                  ))}
                </select>
              ) : (
                <span className="pt-num">
                  <input inputMode="decimal" value={mine[i.key]} onChange={(e) => set(i.key, e.target.value)} />
                  {i.unit && <em>{i.unit}</em>}
                </span>
              )}
            </label>
          ))}
        </div>
      )}
      {error && <p className="pcb-error">{error}</p>}
      {answer && !error && (
        <div className="pt-answer">
          {answer.bands && (
            <div className="pt-resistor" aria-label="Colour bands">
              {answer.bands.map((b, i) => (
                <i key={i} style={{ background: b.hex }} title={`${b.colour}${b.role ? ` — ${b.role}` : ""}`} />
              ))}
            </div>
          )}
          {answer.results.length > 0 && (
            <table className="pt-results">
              <tbody>
                {answer.results.map((r) => (
                  <tr key={r.label}>
                    <th>{r.label}</th>
                    <td>
                      <b>{r.value}</b> {r.unit}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {answer.values && (
            <div className="pt-eseries">
              {answer.values.map((v) => (
                <span key={v}>{v.toFixed(answer.values!.length > 24 ? 2 : 1)}</span>
              ))}
            </div>
          )}
          {[answer.table, answer.levels].filter(Boolean).map((t, k) => (
            <div key={k} className="pt-table-wrap">
              <table className="pt-table">
                <thead>
                  <tr>
                    {t!.columns.map((c) => (
                      <th key={c}>{c}</th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {t!.rows.map((r, i) => (
                    <tr key={i}>
                      {r.map((c, j) => (
                        <td key={j}>{c}</td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ))}
          {answer.notes.map((n) => (
            <p key={n} className="pcb-note">
              · {n}
            </p>
          ))}
          {answer.source && <p className="pt-source">{answer.source}</p>}
        </div>
      )}
    </div>
  );
}

// ---- Panelization --------------------------------------------------------------------

type Spec = { rows: number; cols: number; gap: number; separation: string; tab_width: number; tabs: number; bite_drill: number; bite_spacing: number; bite_offset: number; frame: string; rail: number; fiducials: boolean; fiducial_copper: number; fiducial_opening: number; tooling: boolean; tooling_hole: number };
type Panel = { spec: Spec; size: P; offsets: P[]; board_size: P; outline: P[][]; rails: number[][]; tabs: { rect: number[] }[]; bites: P[]; vcuts: [P, P][]; fiducials: P[]; tooling: P[]; area: number; problems: string[]; notes: string[] };
type PanelResult = { panel: Panel; board: { outline: P[]; cutouts: P[][]; bounds: number[]; name: string }; files: string[]; readback: { file: string; ok: boolean; objects?: number; holes?: number; error?: string }[] };
type BoardLayers = { layers: { file: string; function?: string; svg?: string }[] };

const SPEC: Spec = { rows: 2, cols: 2, gap: 2, separation: "mousebites", tab_width: 3, tabs: 1, bite_drill: 0.5, bite_spacing: 0.8, bite_offset: 0.25, frame: "rails-tb", rail: 5, fiducials: true, fiducial_copper: 1, fiducial_opening: 2, tooling: true, tooling_hole: 1.5 };

const path = (contours: P[][]) => contours.map((c) => `M${c.map((p) => `${p[0]} ${p[1]}`).join("L")}Z`).join("");

export function PanelTab(props: { boardId: string; revision: number; fab: string | null }) {
  const [spec, setSpec] = useStored<Spec>("forma.pcb.panel", SPEC);
  const [result, setResult] = useState<PanelResult | null>(null);
  const [layers, setLayers] = useState<BoardLayers | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fab, setFab] = useState(props.fab ?? "");
  const query = new URLSearchParams(Object.entries(spec).map(([k, v]) => [k, String(v)])).toString();
  useEffect(() => {
    let live = true;
    const t = window.setTimeout(() => {
      fetch(projectUrl(`/pcb/${props.boardId}/panel?${query}`))
        .then(async (r) => (r.ok ? r.json() : Promise.reject(new Error(await r.text()))))
        .then((r: PanelResult) => {
          if (!live) return;
          setResult(r);
          setError(null);
        })
        .catch((e) => live && setError(String(e.message ?? e)));
    }, 200);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [props.boardId, props.revision, query]);
  useEffect(() => {
    fetch(projectUrl(`/pcb/${props.boardId}/layers`))
      .then((r) => (r.ok ? r.json() : null))
      .then(setLayers)
      .catch(() => setLayers(null));
  }, [props.boardId, props.revision]);
  const n = (k: keyof Spec, label: string, unit = "mm", step = 0.1) => (
    <label className="pt-field">
      <span>{label}</span>
      <span className="pt-num">
        <input type="number" step={step} min={0} value={spec[k] as number} onChange={(e) => setSpec({ ...spec, [k]: e.target.value === "" ? 0 : +e.target.value })} />
        {unit && <em>{unit}</em>}
      </span>
    </label>
  );
  const art = useMemo(() => {
    const pick = (re: RegExp) => layers?.layers.find((l) => re.test(l.function ?? ""))?.svg ?? "";
    return { copper: pick(/^Copper,L1/), silk: pick(/^Legend,Top/), mask: pick(/^Soldermask,Top/) };
  }, [layers]);
  const p = result?.panel;
  const m = 3;
  return (
    <div className="pt-panel">
      <div className="pt-form">
        {n("rows", "Rows", "", 1)}
        {n("cols", "Columns", "", 1)}
        <label className="pt-field">
          <span>Separation</span>
          <select value={spec.separation} title="How the boards break apart: tabs drilled with mouse bites, V-score grooves, or solid tabs" onChange={(e) => setSpec({ ...spec, separation: e.target.value })}>
            <option value="mousebites">Tabs with mouse bites</option>
            <option value="vscore">V-score</option>
            <option value="tabs">Solid tabs</option>
          </select>
        </label>
        <label className="pt-field">
          <span>Frame</span>
          <select value={spec.frame} title="Waste rails around the panel for the assembly line to hold" onChange={(e) => setSpec({ ...spec, frame: e.target.value })}>
            <option value="rails-tb">Rails top and bottom</option>
            <option value="rails-lr">Rails left and right</option>
            <option value="frame">Full frame</option>
            <option value="none">None</option>
          </select>
        </label>
        {spec.separation !== "vscore" && n("gap", "Gap")}
        {spec.frame !== "none" && n("rail", "Rail width")}
        {spec.separation !== "vscore" && n("tab_width", "Tab width")}
        {spec.separation !== "vscore" && n("tabs", "Tabs per edge", "", 1)}
        {spec.separation === "mousebites" && n("bite_drill", "Bite drill")}
        {spec.separation === "mousebites" && n("bite_spacing", "Bite spacing")}
        {spec.separation === "mousebites" && n("bite_offset", "Bites into board")}
        {spec.frame !== "none" && (
          <>
            <label className="pt-check">
              <input type="checkbox" checked={spec.fiducials} title="Fiducial marks on the rails for the pick-and-place camera" onChange={(e) => setSpec({ ...spec, fiducials: e.target.checked })} /> Fiducials
            </label>
            <label className="pt-check">
              <input type="checkbox" checked={spec.tooling} title="Tooling holes in the rails to pin the panel in place" onChange={(e) => setSpec({ ...spec, tooling: e.target.checked })} /> Tooling holes
            </label>
          </>
        )}
      </div>
      {error && <p className="pcb-error">{error}</p>}
      {p && result && (
        <>
          <svg className="pt-panel-view" viewBox={`${-m} ${-(p.size[1] + m)} ${p.size[0] + 2 * m} ${p.size[1] + 2 * m}`} role="img" aria-label="Panel">
            <g transform="scale(1,-1)">
              <path d={path(p.outline)} fill="#1f4d33" fillRule="evenodd" stroke="#d8c97a" strokeWidth={0.15} />
              {p.offsets.map((o, i) => (
                <g key={i} transform={`translate(${o[0]} ${o[1]})`}>
                  <g style={{ color: "#c9a35a" }} opacity={0.85} dangerouslySetInnerHTML={{ __html: art.copper }} />
                  <g style={{ color: "#f4f4f0" }} opacity={0.9} dangerouslySetInnerHTML={{ __html: art.silk }} />
                </g>
              ))}
              {p.bites.map((b, i) => (
                <circle key={`b${i}`} cx={b[0]} cy={b[1]} r={p.spec.bite_drill / 2} fill="#0d0f12" />
              ))}
              {p.tooling.map((b, i) => (
                <circle key={`t${i}`} cx={b[0]} cy={b[1]} r={p.spec.tooling_hole / 2} fill="#0d0f12" />
              ))}
              {p.fiducials.map((b, i) => (
                <g key={`f${i}`}>
                  <circle cx={b[0]} cy={b[1]} r={p.spec.fiducial_opening / 2} fill="#0b2a1a" />
                  <circle cx={b[0]} cy={b[1]} r={p.spec.fiducial_copper / 2} fill="#e3c27a" />
                </g>
              ))}
              {p.vcuts.map((v, i) => (
                <line key={`v${i}`} x1={v[0][0]} y1={v[0][1]} x2={v[1][0]} y2={v[1][1]} stroke="#ff8c00" strokeWidth={0.3} strokeDasharray="2 1" />
              ))}
            </g>
          </svg>
          <p className="pcb-note">
            {p.spec.rows * p.spec.cols} boards · {p.size[0].toFixed(1)} × {p.size[1].toFixed(1)} mm · {p.tabs.length} tabs · {p.bites.length} mouse bites · {p.vcuts.length} V-cuts · {p.fiducials.length} fiducials · {p.tooling.length} tooling holes
          </p>
          {p.problems.map((x) => (
            <p key={x} className="pcb-error">
              <Icon name="alert" /> {x}
            </p>
          ))}
          {p.notes.map((x) => (
            <p key={x} className="pcb-note">
              · {x}
            </p>
          ))}
          <details className="pt-readback">
            <summary>
              {result.readback.every((r) => r.ok) ? <Icon name="check" /> : <Icon name="alert" />} {result.readback.filter((r) => r.ok).length}/{result.readback.length} panel files read back
            </summary>
            <ul>
              {result.readback.map((r) => (
                <li key={r.file} className={r.ok ? "" : "bad"}>
                  {r.file}: {r.ok ? (r.holes != null ? `${r.holes} holes` : `${r.objects} objects`) : r.error}
                </li>
              ))}
            </ul>
          </details>
          <div className="pt-actions">
            <select value={fab} onChange={(e) => setFab(e.target.value)} aria-label="Assembly files for">
              <option value="">Generic assembly files</option>
              <option value="jlcpcb">JLCPCB assembly files</option>
              <option value="pcbway">PCBWay assembly files</option>
            </select>
            <a className="button primary small" title="Download the panel as Gerbers, drill and assembly files in one zip" href={projectUrl(`/pcb/${props.boardId}/panel.zip?${query}${fab ? `&fab=${fab}` : ""}`)} download>
              <Icon name="download" /> Panel package
            </a>
          </div>
        </>
      )}
    </div>
  );
}

// ---- Gerber viewer -------------------------------------------------------------------

type Aperture = { d: number; shape: string; size: string; function?: string; uses: number };
type Obj = { kind: string; d?: number; clear?: boolean; net?: number; part?: number; pin?: string; function?: string; at: P; length?: number };
type GLayer = { file: string; kind: "gerber" | "drill"; function: string; guessed: boolean; colour: string; negative: boolean; units: string; bounds: number[] | null; attributes: Record<string, string>; apertures: Aperture[]; objects: Obj[]; tools: [number, number, number, number][]; svg: string; source_lines: number };
type GSet = { layers: GLayer[]; nets: string[]; parts: string[]; bounds: number[]; job: unknown; unreadable: [string, string][]; other: string[] };
type Diff = { function: string; before?: string; after?: string; added: number; removed: number; holes_added: string[]; holes_removed: string[]; svg_added: string; svg_removed: string; same: boolean };
type Upload = { name: string; data: string };
type LayerLook = { visible: boolean; colour: string };
type Highlight = { kind: "net" | "part" | "d"; value: number; layer?: string } | null;

async function toUploads(files: File[]): Promise<Upload[]> {
  const out: Upload[] = [];
  for (const f of files) {
    const buf = new Uint8Array(await f.arrayBuffer());
    let s = "";
    for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode(...buf.subarray(i, i + 0x8000));
    out.push({ name: f.name, data: btoa(s) });
  }
  return out;
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const r = await fetch(projectUrl(path), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export function GerberViewer(props: { boardId: string | null; boardName?: string; onClose: () => void }) {
  const [uploads, setUploads] = useState<Upload[] | null>(null);
  const [set, setSet] = useState<GSet | null>(null);
  const [looks, setLooks] = useState<Record<string, LayerLook>>({});
  const [order, setOrder] = useState<string[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [dim, setDim] = useState(false);
  const [flip, setFlip] = useState(false);
  const [hl, setHl] = useState<Highlight>(null);
  const [picked, setPicked] = useState<{ layer: string; i: number } | null>(null);
  const [tool, setTool] = useState<"inspect" | "measure">("inspect");
  const [measure, setMeasure] = useState<P[]>([]);
  const [cursor, setCursor] = useState<P | null>(null);
  const [view, setView] = useState<number[] | null>(null);
  const [info, setInfo] = useState<"layers" | "apertures" | "nets" | "parts" | "attributes" | "compare">("layers");
  const [diff, setDiff] = useState<Diff[] | null>(null);
  const [showDiff, setShowDiff] = useState(true);
  const [text, setText] = useState<{ file: string; text: string } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const svgRef = useRef<SVGSVGElement>(null);
  const drag = useRef<{ x: number; y: number; view: number[] } | null>(null);

  const open = async (list: Upload[]) => {
    setBusy("Reading the files…");
    setError(null);
    try {
      const s = await post<GSet>("/pcb/gerbview", { files: list });
      setUploads(list);
      setSet(s);
      setLooks(Object.fromEntries(s.layers.map((l) => [l.file, { visible: !/^Paste/.test(l.function), colour: l.colour }])));
      setOrder(s.layers.map((l) => l.file));
      setActive(s.layers.find((l) => l.function.startsWith("Copper"))?.file ?? s.layers[0]?.file ?? null);
      setView(null);
      setDiff(null);
      setHl(null);
      setPicked(null);
      setMeasure([]);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
    setBusy(null);
  };
  const openBoard = async () => {
    if (!props.boardId) return;
    setBusy("Fetching the board's package…");
    const r = await fetch(projectUrl(`/pcb/${props.boardId}/fab.zip`));
    const blob = await r.blob();
    await open(await toUploads([new File([blob], `${props.boardName ?? "board"}.zip`)]));
  };
  useEffect(() => {
    // The board's own package, once there is a board and nothing else is open.
    if (props.boardId && !uploads) void openBoard();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.boardId]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (measure.length) setMeasure([]);
        else if (hl || picked) {
          setHl(null);
          setPicked(null);
        } else props.onClose();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [measure.length, hl, picked, props]);

  const compare = async (files: File[]) => {
    if (!uploads) return;
    setBusy("Comparing…");
    try {
      const r = await post<{ compare: Diff[] }>("/pcb/gerbview", { files: uploads, against: await toUploads(files), px: 0.03 });
      setDiff(r.compare);
      setInfo("compare");
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
    setBusy(null);
  };
  const showSource = async (file: string) => {
    if (!uploads) return;
    const r = await post<{ file: string; text: string }>("/pcb/gerbview", { files: uploads, text_of: file });
    setText(r);
  };

  const b = set?.bounds ?? [0, 0, 10, 10];
  const m = Math.max(2, (b[2] - b[0]) * 0.04);
  const home = [b[0] - m, -(b[3] + m), b[2] - b[0] + 2 * m, b[3] - b[1] + 2 * m];
  const vb = view ?? home;
  const toBoard = (e: { clientX: number; clientY: number }): P | null => {
    const svg = svgRef.current;
    if (!svg) return null;
    const r = svg.getBoundingClientRect();
    const s = Math.max(vb[2] / r.width, vb[3] / r.height);
    const ox = vb[0] + (vb[2] - r.width * s) / 2;
    const oy = vb[1] + (vb[3] - r.height * s) / 2;
    const x = ox + (e.clientX - r.left) * s;
    const y = -(oy + (e.clientY - r.top) * s);
    return [flip ? b[0] + b[2] - x : x, y];
  };
  const wheel = (e: React.WheelEvent) => {
    const p = toBoard(e);
    if (!p) return;
    const k = Math.exp(e.deltaY * 0.0015);
    const px = flip ? b[0] + b[2] - p[0] : p[0];
    setView([px - (px - vb[0]) * k, -p[1] - (-p[1] - vb[1]) * k, vb[2] * k, vb[3] * k]);
  };
  const byFile = useMemo(() => new Map((set?.layers ?? []).map((l) => [l.file, l])), [set]);
  const layersShown = order.map((f) => byFile.get(f)!).filter((l) => l && looks[l.file]?.visible);
  const activeLayer = active ? byFile.get(active) : undefined;

  const click = (e: React.MouseEvent) => {
    const p = toBoard(e);
    if (!p) return;
    if (tool === "measure") {
      setMeasure((m) => (m.length >= 2 ? [p] : [...m, p]));
      return;
    }
    // The topmost tagged object under the pointer.
    const el = e.target as Element;
    const g = el.closest("[data-file]");
    const cls = el.getAttribute("class") ?? "";
    const o = /(?:^| )o(\d+)/.exec(cls);
    if (g && o) setPicked({ layer: g.getAttribute("data-file")!, i: +o[1] });
    else setPicked(null);
  };

  const hlCss = (() => {
    if (!hl) return "";
    const sel = hl.kind === "net" ? `.n${hl.value}` : hl.kind === "part" ? `.c${hl.value}` : `.d${hl.value}`;
    const scope = hl.layer ? `.gv-layer[data-file="${CSS.escape(hl.layer)}"] ` : ".gv-layer ";
    return `.gv-svg .gv-layer :is(path,polyline,circle,line):not(mask *){opacity:.14} .gv-svg ${scope}${sel}:not(mask *){opacity:1;color:#fff}`;
  })();

  const exportSvg = () => {
    const svg = svgRef.current;
    if (!svg) return;
    const clone = svg.cloneNode(true) as SVGSVGElement;
    clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    clone.querySelectorAll(".gv-overlay").forEach((x) => x.remove());
    return new XMLSerializer().serializeToString(clone);
  };
  const exportPng = async () => {
    const s = exportSvg();
    if (!s) return;
    const img = new Image();
    const w = Math.min(8000, Math.round(vb[2] * 40));
    const h = Math.round((w * vb[3]) / vb[2]);
    img.src = URL.createObjectURL(new Blob([s.replace("<svg", `<svg width="${w}" height="${h}"`)], { type: "image/svg+xml" }));
    await img.decode();
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    const g = c.getContext("2d")!;
    g.fillStyle = "#0e1013";
    g.fillRect(0, 0, w, h);
    g.drawImage(img, 0, 0, w, h);
    c.toBlob((blob) => blob && download(blob, "gerbers.png"), "image/png");
  };
  const exportPdf = async () => {
    if (!uploads) return;
    setBusy("Making the PDF…");
    try {
      const r = await fetch(projectUrl("/pcb/gerbview/pdf"), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ files: uploads, looks: layersShown.map((l) => ({ file: l.file, colour: looks[l.file].colour })), background: "#ffffff", mirror: flip }) });
      if (!r.ok) throw new Error(await r.text());
      download(await r.blob(), "gerbers.pdf");
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
    setBusy(null);
  };

  const pickedObj = picked ? byFile.get(picked.layer)?.objects[picked.i] : undefined;
  const pickedLayer = picked ? byFile.get(picked.layer) : undefined;
  const dist = measure.length === 2 ? Math.hypot(measure[1][0] - measure[0][0], measure[1][1] - measure[0][1]) : null;
  const unit = Math.max(vb[2], vb[3]) / 400; // one screen pixel or so, in mm

  return (
    <section className="gv" aria-label="Gerber viewer">
      <header className="gv-head">
        <b>Gerber viewer</b>
        <span className="gv-sub">{set ? `${set.layers.length} layers · ${(b[2] - b[0]).toFixed(2)} × ${(b[3] - b[1]).toFixed(2)} mm · ${set.nets.length} nets · ${set.parts.length} parts` : "Open a fab zip, Gerber or drill files"}</span>
        <div className="gv-tools">
          <label className="button secondary small">
            <Icon name="paperclip" /> Open files
            <input type="file" multiple className="gv-file" onChange={(e) => e.target.files && void toUploads([...e.target.files]).then(open)} />
          </label>
          {props.boardId && (
            <button type="button" className="secondary small" title="Open this board's own fab package in the viewer" onClick={() => void openBoard()}>
              <Icon name="chip" /> This board
            </button>
          )}
          {set && (
            <>
              <label className="button secondary small" title="Compare with an earlier revision (zip or files)">
                <Icon name="history" /> Compare…
                <input type="file" multiple className="gv-file" onChange={(e) => e.target.files && void compare([...e.target.files])} />
              </label>
              <span className="gv-seg" role="group" aria-label="Tool">
                <button type="button" className={tool === "inspect" ? "on" : ""} onClick={() => setTool("inspect")} title="Click an object to see what it is">
                  <Icon name="cursor" /> Inspect
                </button>
                <button type="button" className={tool === "measure" ? "on" : ""} onClick={() => setTool("measure")} title="Click two points">
                  <Icon name="ruler" /> Measure
                </button>
              </span>
              <button type="button" className={`secondary small ${flip ? "on" : ""}`} onClick={() => setFlip(!flip)} title="Look from below">
                <Icon name="mirror" /> Flip
              </button>
              <button type="button" className="secondary small" onClick={() => setView(null)} title="Fit">
                <Icon name="fit" />
              </button>
              <span className="gv-seg" role="group" aria-label="Export">
                <button type="button" title="Save the view as a PNG picture" onClick={() => void exportPng()}>PNG</button>
                <button type="button" title="Save the view as an SVG drawing" onClick={() => { const s = exportSvg(); if (s) download(new Blob([s], { type: "image/svg+xml" }), "gerbers.svg"); }}>SVG</button>
                <button type="button" title="Save the layers as a PDF" onClick={() => void exportPdf()}>PDF</button>
              </span>
            </>
          )}
          <IconButton icon="close" label="Close (Esc)" onClick={props.onClose} />
        </div>
      </header>
      <div className="gv-body">
        <aside className="gv-side">
          <nav className="gv-tabs">
            {(["layers", "apertures", "nets", "parts", "attributes", ...(diff ? ["compare"] : [])] as (typeof info)[]).map((k) => (
              <button key={k} type="button" className={info === k ? "active" : ""} title={{ layers: "Layers in the files: show, colour and dim them", apertures: "Apertures (D codes) used by the files", nets: "Nets in the files: click one to highlight it", parts: "Parts in the files: click one to highlight it", attributes: "File attributes (X2) the files carry", compare: "What changed from the earlier revision" }[k]} onClick={() => setInfo(k)}>
                {k[0].toUpperCase() + k.slice(1)}
              </button>
            ))}
          </nav>
          <div className="gv-side-body">
            {!set && !busy && <p className="pcb-note">Drop a zip here or open files: Gerber RS-274X (X2 attributes read when present) and Excellon drill.</p>}
            {set && info === "layers" && (
              <>
                <label className="pt-check">
                  <input type="checkbox" checked={dim} title="Fade every layer except the active one, so it stands out" onChange={(e) => setDim(e.target.checked)} /> Dim all but the active layer
                </label>
                <ul className="gv-layers">
                  {order.map((f, i) => {
                    const l = byFile.get(f)!;
                    const look = looks[f];
                    return (
                      <li key={f} className={active === f ? "active" : ""} onClick={() => setActive(f)}>
                        <input type="checkbox" checked={look.visible} onChange={(e) => setLooks({ ...looks, [f]: { ...look, visible: e.target.checked } })} onClick={(e) => e.stopPropagation()} aria-label={`Show ${l.function}`} />
                        <input type="color" value={look.colour} onChange={(e) => setLooks({ ...looks, [f]: { ...look, colour: e.target.value } })} onClick={(e) => e.stopPropagation()} aria-label="Colour" />
                        <span className="gv-lname" title={f}>
                          {l.function}
                          {l.guessed && <em title="No X2 file function: the layer is guessed from the name"> ?</em>}
                          <small>{f}</small>
                        </span>
                        <span className="gv-order">
                          <button type="button" disabled={i === 0} onClick={(e) => { e.stopPropagation(); const o = [...order]; [o[i - 1], o[i]] = [o[i], o[i - 1]]; setOrder(o); }} aria-label="Up">▲</button>
                          <button type="button" disabled={i === order.length - 1} onClick={(e) => { e.stopPropagation(); const o = [...order]; [o[i + 1], o[i]] = [o[i], o[i + 1]]; setOrder(o); }} aria-label="Down">▼</button>
                          <button type="button" onClick={(e) => { e.stopPropagation(); void showSource(f); }} title="Show the file">{"</>"}</button>
                        </span>
                      </li>
                    );
                  })}
                </ul>
                <p className="pcb-note">Top of the list draws on top. {set.unreadable.map(([f, e]) => `${f}: ${e}. `)}{set.other.length ? `Other files: ${set.other.join(", ")}` : ""}</p>
              </>
            )}
            {set && info === "apertures" && activeLayer && (
              <>
                <p className="pcb-note">{activeLayer.function} · {activeLayer.file}. Click a D-code to highlight what uses it.</p>
                <table className="pt-table gv-dcodes">
                  <thead>
                    <tr>
                      <th>{activeLayer.kind === "drill" ? "Tool" : "D"}</th>
                      <th>Shape</th>
                      <th>Size</th>
                      <th>Uses</th>
                    </tr>
                  </thead>
                  <tbody>
                    {activeLayer.kind === "drill"
                      ? activeLayer.tools.map(([t, d, h, s]) => (
                          <tr key={t} className={hl?.kind === "d" && hl.value === t && hl.layer === activeLayer.file ? "on" : ""} onClick={() => setHl({ kind: "d", value: t, layer: activeLayer.file })}>
                            <td>T{t}</td>
                            <td>{s ? "slot" : "hole"}</td>
                            <td>ø{d}</td>
                            <td>{h + s}</td>
                          </tr>
                        ))
                      : activeLayer.apertures.map((a) => (
                          <tr key={a.d} className={hl?.kind === "d" && hl.value === a.d && hl.layer === activeLayer.file ? "on" : ""} onClick={() => setHl({ kind: "d", value: a.d, layer: activeLayer.file })} title={a.function}>
                            <td>D{a.d}</td>
                            <td>{a.shape}</td>
                            <td>{a.size}</td>
                            <td>{a.uses}</td>
                          </tr>
                        ))}
                  </tbody>
                </table>
              </>
            )}
            {set && (info === "nets" || info === "parts") && (
              <>
                <input className="gv-filter" placeholder={`Find a ${info === "nets" ? "net" : "part"}…`} value={filter} onChange={(e) => setFilter(e.target.value)} />
                <ul className="gv-list">
                  {(info === "nets" ? set.nets : set.parts)
                    .map((n, i) => [n, i] as const)
                    .filter(([n]) => n.toLowerCase().includes(filter.toLowerCase()))
                    .map(([n, i]) => {
                      const kind = info === "nets" ? "net" : "part";
                      const on = hl?.kind === kind && hl.value === i;
                      return (
                        <li key={n} className={on ? "on" : ""} onClick={() => setHl(on ? null : { kind, value: i })}>
                          {n}
                        </li>
                      );
                    })}
                </ul>
                {(info === "nets" ? set.nets : set.parts).length === 0 && <p className="pcb-note">These files carry no X2 object attributes, so there are no {info} to show.</p>}
              </>
            )}
            {set && info === "attributes" && activeLayer && (
              <table className="pt-table">
                <tbody>
                  {Object.entries(activeLayer.attributes).map(([k, v]) => (
                    <tr key={k}>
                      <th>{k}</th>
                      <td>{v}</td>
                    </tr>
                  ))}
                  <tr>
                    <th>Units</th>
                    <td>{activeLayer.units}</td>
                  </tr>
                  <tr>
                    <th>Objects</th>
                    <td>{activeLayer.objects.length}</td>
                  </tr>
                  <tr>
                    <th>Lines</th>
                    <td>{activeLayer.source_lines}</td>
                  </tr>
                </tbody>
              </table>
            )}
            {info === "compare" && diff && (
              <>
                <label className="pt-check">
                  <input type="checkbox" checked={showDiff} title="Colour what changed from the earlier revision: green added, red removed" onChange={(e) => setShowDiff(e.target.checked)} /> Show differences (green added, red removed)
                </label>
                <ul className="gv-diff">
                  {diff.map((d) => (
                    <li key={d.function} className={d.same ? "same" : "changed"}>
                      <b>{d.same ? "=" : "≠"} {d.function}</b>
                      <small>
                        {d.before ?? "(none)"} → {d.after ?? "(none)"}
                      </small>
                      {!d.same && (d.holes_added.length || d.holes_removed.length ? <span>{d.holes_added.length} holes added, {d.holes_removed.length} removed</span> : <span>+{d.added} mm² · −{d.removed} mm²</span>)}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>
          {pickedObj && pickedLayer && (
            <div className="gv-picked">
              <b>{pickedLayer.function}</b>
              <table className="pt-table">
                <tbody>
                  <tr>
                    <th>Object</th>
                    <td>
                      {pickedObj.kind}
                      {pickedObj.clear ? " (clear)" : ""} #{picked!.i}
                    </td>
                  </tr>
                  {pickedObj.d != null && (
                    <tr>
                      <th>{pickedLayer.kind === "drill" ? "Tool" : "Aperture"}</th>
                      <td>
                        {pickedLayer.kind === "drill" ? `T${pickedObj.d}` : `D${pickedObj.d}`} {pickedLayer.apertures.find((a) => a.d === pickedObj.d)?.size ?? ""}
                      </td>
                    </tr>
                  )}
                  {pickedObj.function && (
                    <tr>
                      <th>Function</th>
                      <td>{pickedObj.function}</td>
                    </tr>
                  )}
                  {pickedObj.net != null && (
                    <tr>
                      <th>Net</th>
                      <td>
                        <button type="button" className="gv-link" title="Highlight every object on this net" onClick={() => setHl({ kind: "net", value: pickedObj.net! })}>
                          {set!.nets[pickedObj.net]}
                        </button>
                      </td>
                    </tr>
                  )}
                  {pickedObj.part != null && (
                    <tr>
                      <th>Part</th>
                      <td>
                        <button type="button" className="gv-link" title="Highlight every pad of this part" onClick={() => setHl({ kind: "part", value: pickedObj.part! })}>
                          {set!.parts[pickedObj.part]}
                        </button>
                        {pickedObj.pin ? ` pin ${pickedObj.pin.split(",").slice(1).join(" ")}` : ""}
                      </td>
                    </tr>
                  )}
                  <tr>
                    <th>At</th>
                    <td>
                      {pickedObj.at[0].toFixed(3)}, {pickedObj.at[1].toFixed(3)}
                      {pickedObj.length != null ? ` · ${pickedObj.length.toFixed(3)} mm long` : ""}
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          )}
        </aside>
        <div
          className={`gv-canvas ${tool}`}
          onDragOver={(e) => e.preventDefault()}
          onDrop={(e) => {
            e.preventDefault();
            if (e.dataTransfer.files.length) void toUploads([...e.dataTransfer.files]).then(open);
          }}
        >
          {busy && <div className="gv-busy">{busy}</div>}
          {error && <p className="pcb-error gv-error">{error}</p>}
          <style>{hlCss}</style>
          <svg
            ref={svgRef}
            className="gv-svg"
            viewBox={vb.join(" ")}
            onWheel={wheel}
            onPointerDown={(e) => {
              drag.current = { x: e.clientX, y: e.clientY, view: vb };
            }}
            onPointerMove={(e) => {
              setCursor(toBoard(e));
              const d = drag.current;
              if (!d || e.buttons === 0) return;
              const r = svgRef.current!.getBoundingClientRect();
              const s = Math.max(vb[2] / r.width, vb[3] / r.height);
              if (Math.abs(e.clientX - d.x) + Math.abs(e.clientY - d.y) > 3) setView([d.view[0] - (e.clientX - d.x) * s, d.view[1] - (e.clientY - d.y) * s, d.view[2], d.view[3]]);
            }}
            onPointerUp={(e) => {
              const d = drag.current;
              drag.current = null;
              if (d && Math.abs(e.clientX - d.x) + Math.abs(e.clientY - d.y) <= 3) click(e);
            }}
          >
            <rect x={vb[0] - vb[2]} y={vb[1] - vb[3]} width={vb[2] * 3} height={vb[3] * 3} fill="#0e1013" />
            <g transform={flip ? `translate(${b[0] + b[2]} 0) scale(-1,-1)` : "scale(1,-1)"}>
              {[...layersShown].reverse().map((l) => (
                <g key={l.file} className="gv-layer" data-file={l.file} style={{ color: looks[l.file].colour }} opacity={dim && active !== l.file ? 0.25 : l.function.startsWith("Soldermask") || l.function.startsWith("Paste") ? 0.6 : 0.88} dangerouslySetInnerHTML={{ __html: l.svg }} />
              ))}
              {diff && showDiff && (
                <g className="gv-overlay">
                  {diff.map((d) => (
                    <g key={d.function}>
                      <g fill="#36e07a" opacity={0.85} dangerouslySetInnerHTML={{ __html: d.svg_added }} />
                      <g fill="#ff4d4d" opacity={0.85} dangerouslySetInnerHTML={{ __html: d.svg_removed }} />
                    </g>
                  ))}
                </g>
              )}
              {pickedObj && (
                <circle className="gv-overlay" cx={pickedObj.at[0]} cy={pickedObj.at[1]} r={unit * 6} fill="none" stroke="#fff" strokeWidth={unit * 1.5} />
              )}
              {measure.length > 0 && (
                <g className="gv-overlay">
                  {measure.map((p, i) => (
                    <circle key={i} cx={p[0]} cy={p[1]} r={unit * 3} fill="#ffd34d" />
                  ))}
                  {measure.length === 2 && <line x1={measure[0][0]} y1={measure[0][1]} x2={measure[1][0]} y2={measure[1][1]} stroke="#ffd34d" strokeWidth={unit * 1.5} />}
                </g>
              )}
            </g>
          </svg>
          <div className="gv-status">
            {cursor && (
              <span>
                X {cursor[0].toFixed(3)} · Y {cursor[1].toFixed(3)} mm
              </span>
            )}
            {dist !== null && (
              <span className="gv-measure">
                <Icon name="ruler" /> {dist.toFixed(3)} mm (ΔX {(measure[1][0] - measure[0][0]).toFixed(3)}, ΔY {(measure[1][1] - measure[0][1]).toFixed(3)})
              </span>
            )}
            {hl && (
              <span>
                Highlight: {hl.kind === "net" ? set?.nets[hl.value] : hl.kind === "part" ? set?.parts[hl.value] : `D${hl.value}`}{" "}
                <button type="button" className="gv-link" title="Clear the highlight" onClick={() => setHl(null)}>
                  clear
                </button>
              </span>
            )}
            {flip && <span>Seen from below</span>}
          </div>
        </div>
      </div>
      {text && (
        <div className="gv-source" role="dialog" aria-label={text.file}>
          <header>
            <b>{text.file}</b>
            <IconButton icon="close" label={`Close ${text.file}`} onClick={() => setText(null)} />
          </header>
          <pre>{text.text.length > 400_000 ? `${text.text.slice(0, 400_000)}\n…` : text.text}</pre>
        </div>
      )}
    </section>
  );
}
