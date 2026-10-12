// The Electronics workspace's Library view: the symbol editor (pins with
// types, shapes, lengths and sides, a pin table, units and De Morgan bodies,
// graphics, fields, power and derived symbols, the symbol checker), the
// footprint editor (pads of every shape, pad stacks, thermal settings,
// courtyard, silk and body, the 3D model offset, the footprint checker, IPC
// wizards), image-to-logo, footprint assignment, assembly variants, design
// blocks and the library table (project, global, KiCad, built-in).
// The engine draws and checks everything (POST /pcb/libs/*/preview); this
// edits the data and saves it with the pcb.lib.* commands.

/* eslint-disable @typescript-eslint/no-explicit-any */
import { type PointerEvent as ReactPointerEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";
import "./PcbLibrary.css";

type P = [number, number];
type Commit = (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>;
type Finding = { severity: "error" | "warning" | "note"; code: string; message: string; items?: string[]; at?: P };
type Draw =
  | { t: "line"; p: P[]; w: number; ink: string }
  | { t: "poly"; p: P[]; w: number; ink: string; fill: string }
  | { t: "circle"; c: P; r: number; w: number; ink: string; fill: string }
  | { t: "text"; at: P; s: string; size: number; angle: number; anchor: string; ink: string };

const INK: Record<string, string> = { body: "#8b1c1c", pin: "#8b1c1c", pin_number: "#a03030", pin_name: "#00646e", reference: "#00646e", value: "#00646e", power: "#8a1f1f", body_fill: "#fffbd0", dnp: "#d03030" };
const PIN_TYPES = ["input", "output", "bidirectional", "tri_state", "passive", "power_in", "power_out", "open_collector", "open_emitter", "no_connect", "unspecified"];
const PIN_SHAPES = ["line", "inverted", "clock", "inverted_clock", "input_low", "clock_low", "output_low", "edge_clock_high", "non_logic"];
const SIDES = ["left", "right", "up", "down"];
const G = 1.27;
const snap = (v: number, g = G) => +(Math.round(v / g) * g).toFixed(4);
const r4 = (v: number) => +v.toFixed(4);

async function getJson(path: string) {
  const r = await fetch(projectUrl(path));
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}
async function postJson(path: string, body: unknown) {
  const r = await fetch(projectUrl(path), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}
const outOf = (r: unknown) => (r as { output?: any } | null)?.output;

function useDebounced<T>(value: T, ms = 220) {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

function Findings({ list, onPick }: { list: Finding[] | undefined; onPick?: (f: Finding) => void }) {
  if (!list) return null;
  const shown = list.filter((f) => f.severity !== "note");
  const notes = list.filter((f) => f.severity === "note");
  return (
    <div className="lib-findings">
      <div className={`lib-check ${shown.some((f) => f.severity === "error") ? "bad" : shown.length ? "warn" : "good"}`}>
        <Icon name={shown.some((f) => f.severity === "error") ? "alert" : "check"} />
        {shown.length ? `${shown.filter((f) => f.severity === "error").length} errors, ${shown.filter((f) => f.severity === "warning").length} warnings` : "Checker passes"}
      </div>
      {[...shown, ...notes].map((f, i) => (
        <button type="button" key={i} className={`lib-finding ${f.severity}`} onClick={() => onPick?.(f)}>
          <b>{f.code}</b> {f.message}
        </button>
      ))}
    </div>
  );
}

function DrawEl({ d }: { d: Draw }) {
  const c = INK[d.ink] ?? "#333";
  const fill = (f: string) => (f === "ink" ? c : f === "body" ? INK.body_fill : "none");
  switch (d.t) {
    case "line":
      return <polyline points={d.p.map((p) => p.join(",")).join(" ")} fill="none" stroke={c} strokeWidth={d.w} strokeLinecap="round" strokeLinejoin="round" />;
    case "poly":
      return <polygon points={d.p.map((p) => p.join(",")).join(" ")} fill={fill(d.fill)} stroke={c} strokeWidth={d.w} strokeLinejoin="round" />;
    case "circle":
      return <circle cx={d.c[0]} cy={d.c[1]} r={d.r} fill={fill(d.fill)} stroke={c} strokeWidth={d.w} />;
    case "text":
      return d.s ? (
        <text x={d.at[0]} y={d.at[1]} dy={d.size * 0.35} fontSize={d.size} fontFamily="Helvetica, Arial, sans-serif" textAnchor={d.anchor as any} fill={c} transform={d.angle ? `rotate(-90 ${d.at[0]} ${d.at[1]})` : undefined}>
          {d.s}
        </text>
      ) : null;
  }
}

/** A pannable, zoomable SVG with a grid; `toWorld` maps a pointer to drawing units. */
function Canvas(props: { box: [number, number, number, number]; flipY?: boolean; grid: number; children: React.ReactNode; onPointer?: (kind: "down" | "move" | "up", p: P, e: ReactPointerEvent) => void; dark?: boolean }) {
  const ref = useRef<SVGSVGElement>(null);
  const [view, setView] = useState<[number, number, number, number] | null>(null);
  const fit = useMemo<[number, number, number, number]>(() => {
    const [x0, y0, x1, y1] = props.box;
    const m = Math.max(x1 - x0, y1 - y0) * 0.15 + 2;
    return props.flipY ? [x0 - m, -y1 - m, x1 - x0 + 2 * m, y1 - y0 + 2 * m] : [x0 - m, y0 - m, x1 - x0 + 2 * m, y1 - y0 + 2 * m];
  }, [props.box, props.flipY]);
  const key = props.box.map((v) => Math.round(v)).join(",");
  useEffect(() => setView(null), [key]);
  const vb = view ?? fit;
  const world = (e: { clientX: number; clientY: number }): P => {
    const s = ref.current!;
    const pt = s.createSVGPoint();
    pt.x = e.clientX;
    pt.y = e.clientY;
    const p = pt.matrixTransform(s.getScreenCTM()!.inverse());
    return [p.x, props.flipY ? -p.y : p.y];
  };
  const pan = useRef<{ at: P; vb: [number, number, number, number] } | null>(null);
  const g = props.grid;
  const lines: React.ReactNode[] = [];
  if (vb[2] / g < 220) {
    for (let x = Math.ceil(vb[0] / g) * g; x < vb[0] + vb[2]; x += g) lines.push(<line key={`x${x}`} x1={x} y1={vb[1]} x2={x} y2={vb[1] + vb[3]} />);
    for (let y = Math.ceil(vb[1] / g) * g; y < vb[1] + vb[3]; y += g) lines.push(<line key={`y${y}`} x1={vb[0]} y1={y} x2={vb[0] + vb[2]} y2={y} />);
  }
  return (
    <svg
      ref={ref}
      className={`lib-canvas ${props.dark ? "dark" : ""}`}
      viewBox={vb.join(" ")}
      preserveAspectRatio="xMidYMid meet"
      onWheel={(e) => {
        const p = world(e);
        const py = props.flipY ? -p[1] : p[1];
        const k = e.deltaY > 0 ? 1.15 : 1 / 1.15;
        setView([p[0] - (p[0] - vb[0]) * k, py - (py - vb[1]) * k, vb[2] * k, vb[3] * k]);
      }}
      onPointerDown={(e) => {
        if (e.button === 1 || e.button === 2 || e.altKey) {
          pan.current = { at: [e.clientX, e.clientY], vb };
          (e.target as Element).setPointerCapture?.(e.pointerId);
          return;
        }
        props.onPointer?.("down", world(e), e);
      }}
      onPointerMove={(e) => {
        if (pan.current) {
          const s = ref.current!.getBoundingClientRect();
          const k = Math.max(vb[2] / s.width, vb[3] / s.height);
          const v = pan.current.vb;
          setView([v[0] - (e.clientX - pan.current.at[0]) * k, v[1] - (e.clientY - pan.current.at[1]) * k, v[2], v[3]]);
          return;
        }
        props.onPointer?.("move", world(e), e);
      }}
      onPointerUp={(e) => {
        if (pan.current) {
          pan.current = null;
          return;
        }
        props.onPointer?.("up", world(e), e);
      }}
      onContextMenu={(e) => e.preventDefault()}
      onDoubleClick={() => setView(null)}
    >
      <g className="lib-grid" strokeWidth={vb[2] / 900}>
        {lines}
      </g>
      <line x1={-0.8} y1={0} x2={0.8} y2={0} className="lib-origin" strokeWidth={vb[2] / 500} />
      <line x1={0} y1={-0.8} x2={0} y2={0.8} className="lib-origin" strokeWidth={vb[2] / 500} />
      {props.children}
    </svg>
  );
}

// ---- Symbol editor ---------------------------------------------------------------

type Pin = { number: string; name: string; at: P; side: string; length: number; etype: string; hidden?: boolean; unit?: number; pads?: string[]; shape?: string };
type SymDef = { name: string; description: string; prefix: string; category: string; value: string; footprint: string; footprints?: string[]; mpn?: string | null; manufacturer?: string | null; lcsc?: string | null; pins: Pin[]; units: any[][]; show_pin_numbers: boolean; show_pin_names: boolean; virtual_part?: boolean; keywords?: string[]; demorgan?: any[][]; demorgan_pins?: Record<string, string>; body_style?: number; fields?: Record<string, string> };
type SymState = { library: string; def: SymDef; extends: string; power: { net: string; style?: string } | null; renameFrom: string | null };

function blankSymbol(): SymState {
  return {
    library: "Project",
    renameFrom: null,
    extends: "",
    power: null,
    def: {
      name: "NewPart",
      description: "",
      prefix: "U",
      category: "IC",
      value: "NewPart",
      footprint: "",
      pins: [
        { number: "1", name: "IN", at: [-10.16, -2.54], side: "left", length: 2.54, etype: "input" },
        { number: "2", name: "GND", at: [0, 7.62], side: "down", length: 2.54, etype: "power_in" },
        { number: "3", name: "OUT", at: [10.16, -2.54], side: "right", length: 2.54, etype: "output" },
      ],
      units: [[{ kind: "rect", a: [-7.62, -5.08], b: [7.62, 5.08], fill: "body" }]],
      show_pin_numbers: true,
      show_pin_names: true,
      fields: {},
    },
  };
}

/** A box sized to the pins, inputs left, outputs right, power on top, ground below (on the 50 mil grid). */
function arrangeBox(def: SymDef, unit: number): SymDef {
  const pins = def.pins.filter((p) => !p.hidden && (!p.unit || p.unit === unit));
  const sideOf = (p: Pin) => {
    const n = p.name.toUpperCase();
    if (p.etype === "power_in" && (n.includes("GND") || n.startsWith("VSS"))) return "down";
    if (p.etype === "power_in") return "up";
    if (["output", "power_out", "open_collector", "tri_state"].includes(p.etype)) return "right";
    return p.side === "right" && p.etype !== "input" ? "right" : "left";
  };
  const groups: Record<string, Pin[]> = { left: [], right: [], up: [], down: [] };
  pins.forEach((p) => groups[sideOf(p)].push(p));
  const rows = Math.max(groups.left.length, groups.right.length, 1);
  const cols = Math.max(groups.up.length, groups.down.length);
  const nameW = (l: Pin[]) => Math.max(0, ...l.map((p) => p.name.length * 0.9));
  const halfW = Math.ceil(Math.max((nameW(groups.left) + nameW(groups.right) + 3 * 2.54) / 2, ((cols + 1) * 2.54) / 2, 5.08) / 2.54) * 2.54;
  const halfH = Math.ceil(((rows + 1) * 2.54) / 2 / G) * G;
  const place = (l: Pin[], side: string) =>
    l.forEach((p, i) => {
      const n = l.length;
      const row = snap((i - (rows - 1) / 2) * 2.54);
      const col = snap((i - (n - 1) / 2) * 2.54);
      p.side = side;
      p.length = 2.54;
      p.at = side === "left" ? [snap(-halfW - 2.54), row] : side === "right" ? [snap(halfW + 2.54), row] : side === "up" ? [col, snap(-halfH - 2.54)] : [col, snap(halfH + 2.54)];
    });
  const next = structuredClone(def);
  const byNum = new Map(next.pins.map((p) => [p.number + "/" + (p.unit ?? 0), p]));
  for (const [side, list] of Object.entries(groups)) place(list.map((p) => byNum.get(p.number + "/" + (p.unit ?? 0))!), side);
  next.units[unit - 1] = [{ kind: "rect", a: [-halfW, -halfH], b: [halfW, halfH], fill: "body" }, ...(next.units[unit - 1] ?? []).filter((g: any) => g.kind !== "rect")];
  return next;
}

function GfxRow({ g, onChange, onDelete }: { g: any; onChange: (g: any) => void; onDelete: () => void }) {
  const num = (k: string, i?: number) => (
    <input
      type="number"
      step={0.127}
      value={i === undefined ? g[k] : g[k][i]}
      onChange={(e) => {
        const v = Number(e.target.value);
        if (i === undefined) onChange({ ...g, [k]: v });
        else {
          const a = [...g[k]];
          a[i] = v;
          onChange({ ...g, [k]: a });
        }
      }}
    />
  );
  return (
    <div className="lib-gfx-row">
      <b>{g.kind}</b>
      {g.kind === "rect" && (
        <>
          a {num("a", 0)}
          {num("a", 1)} b {num("b", 0)}
          {num("b", 1)}
        </>
      )}
      {(g.kind === "circle" || g.kind === "arc") && (
        <>
          c {num("c", 0)}
          {num("c", 1)} r {num("r")}
        </>
      )}
      {g.kind === "arc" && (
        <>
          from {num("start")}° to {num("end")}°
        </>
      )}
      {g.kind === "text" && (
        <>
          <input value={g.text} onChange={(e) => onChange({ ...g, text: e.target.value })} /> at {num("at", 0)}
          {num("at", 1)}
        </>
      )}
      {(g.kind === "line" || g.kind === "poly") && (
        <input
          className="lib-pts"
          title="Points: x,y x,y …"
          defaultValue={g.pts.map((p: P) => p.join(",")).join(" ")}
          onBlur={(e) => {
            const pts = e.target.value
              .trim()
              .split(/\s+/)
              .map((s) => s.split(",").map(Number) as P)
              .filter((p) => p.length === 2 && p.every(Number.isFinite));
            if (pts.length >= 2) onChange({ ...g, pts });
          }}
        />
      )}
      {"fill" in g && (
        <select value={g.fill ?? "none"} onChange={(e) => onChange({ ...g, fill: e.target.value })}>
          <option value="none">no fill</option>
          <option value="body">body</option>
          <option value="outline">solid</option>
        </select>
      )}
      <button type="button" className="lib-x" onClick={onDelete} title="Delete">
        ×
      </button>
    </div>
  );
}

function SymbolEditor(props: { start: SymState; commit: Commit; canEdit: boolean; schematic: string | null; footprintNames: string[]; onSaved: () => void; notify: (m: string) => void }) {
  const [s, setS] = useState<SymState>(props.start);
  const [unit, setUnit] = useState(1);
  const [style, setStyle] = useState(1);
  const [selPin, setSelPin] = useState<number | null>(null);
  const [preview, setPreview] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const drag = useRef<{ i: number; moved: boolean } | null>(null);
  useEffect(() => setS(props.start), [props.start]);
  const def = s.def;
  const set = (f: (d: SymDef) => void) =>
    setS((x) => {
      const d = structuredClone(x.def);
      f(d);
      return { ...x, def: d };
    });
  // Editing style 2 edits the De Morgan drawing.
  const units = style === 2 ? (def.demorgan ?? []) : def.units;
  const setUnitGfx = (gfx: any[]) =>
    set((d) => {
      if (style === 2) {
        d.demorgan = d.demorgan ?? d.units.map(() => []);
        d.demorgan[unit - 1] = gfx;
      } else d.units[unit - 1] = gfx;
    });
  const dq = useDebounced(s);
  useEffect(() => {
    let live = true;
    postJson("/pcb/libs/symbol/preview", { symbol: dq.def, extends: dq.extends || null, unit, body_style: style })
      .then((r) => live && (setPreview(r), setError(null)))
      .catch((e) => live && setError(String(e.message ?? e)));
    return () => {
      live = false;
    };
  }, [dq, unit, style]);

  const save = async () => {
    const out = outOf(await props.commit("pcb.lib.symbol.save", { library: s.library, symbol: def, extends: s.extends || null, power: s.power, rename_from: s.renameFrom }, `Saved ${s.library}:${def.name}`));
    if (out) {
      setS((x) => ({ ...x, renameFrom: `${x.library}:${x.def.name}` }));
      props.onSaved();
    }
  };
  const unitCount = Math.max(1, def.units.length);
  const visiblePins = def.pins.map((p, i) => ({ p, i })).filter(({ p }) => !p.unit || p.unit === unit);
  const box = (preview?.preview?.bbox as [number, number, number, number]) ?? [-10, -10, 10, 10];
  return (
    <div className="lib-editor">
      <div className="lib-stage">
        <div className="lib-stagebar">
          <span className="lib-title">{s.library}:{def.name}</span>
          {unitCount > 1 || def.units.length ? (
            <select value={unit} onChange={(e) => setUnit(Number(e.target.value))} title="Unit">
              {Array.from({ length: unitCount }, (_, i) => (
                <option key={i} value={i + 1}>
                  Unit {String.fromCharCode(65 + i)}
                </option>
              ))}
            </select>
          ) : null}
          <button type="button" className="secondary" onClick={() => set((d) => void d.units.push([]))} title="Add a unit (multi-unit part)">
            + Unit
          </button>
          {unitCount > 1 && (
            <button
              type="button"
              className="secondary"
              onClick={() => {
                set((d) => {
                  d.units.splice(unit - 1, 1);
                  if (d.demorgan?.length) d.demorgan.splice(unit - 1, 1);
                  d.pins = d.pins.filter((p) => p.unit !== unit).map((p) => (p.unit && p.unit > unit ? { ...p, unit: p.unit - 1 } : p));
                });
                setUnit(1);
              }}
            >
              − Unit
            </button>
          )}
          <div className="lib-seg" role="tablist" aria-label="Body style">
            <button type="button" className={style === 1 ? "active" : ""} onClick={() => setStyle(1)}>
              Normal
            </button>
            <button
              type="button"
              className={style === 2 ? "active" : ""}
              onClick={() => {
                if (!def.demorgan?.length) set((d) => void (d.demorgan = structuredClone(d.units)));
                setStyle(2);
              }}
              title="De Morgan (alternate) body"
            >
              De Morgan
            </button>
          </div>
          <button type="button" className="secondary" onClick={() => setS((x) => ({ ...x, def: arrangeBox(x.def, unit) }))} title="Arrange pins round a box sized to their names">
            Box from pins
          </button>
        </div>
        <Canvas
          box={box}
          grid={G}
          onPointer={(kind, p) => {
            if (!props.canEdit) return;
            if (kind === "down") {
              const hit = visiblePins.find(({ p: q }) => Math.hypot(q.at[0] - p[0], q.at[1] - p[1]) < 1.2);
              if (hit) {
                setSelPin(hit.i);
                drag.current = { i: hit.i, moved: false };
              }
            } else if (kind === "move" && drag.current) {
              const i = drag.current.i;
              drag.current.moved = true;
              set((d) => void (d.pins[i].at = [snap(p[0]), snap(p[1])]));
            } else if (kind === "up") drag.current = null;
          }}
        >
          {preview?.preview?.draw?.map((d: Draw, i: number) => <DrawEl key={i} d={d} />)}
          {visiblePins.map(({ p, i }) => (
            <circle key={i} cx={p.at[0]} cy={p.at[1]} r={selPin === i ? 0.6 : 0.35} className={`lib-pin-dot ${selPin === i ? "sel" : ""}`} />
          ))}
          {preview?.findings
            ?.filter((f: Finding) => f.at && f.severity !== "note")
            .map((f: Finding, i: number) => (
              <circle key={`f${i}`} cx={f.at![0]} cy={f.at![1]} r={1.0} className={`lib-mark ${f.severity}`} />
            ))}
        </Canvas>
        {error && <p className="lib-error">{error}</p>}
        <div className="lib-hint">Drag pins on the grid · wheel zooms · Alt-drag pans · double-click fits</div>
      </div>
      <div className="lib-props">
        <section>
          <h4>Symbol</h4>
          <label>
            Name <input value={def.name} onChange={(e) => set((d) => void (d.name = e.target.value))} />
          </label>
          <label>
            Library <input value={s.library} onChange={(e) => setS((x) => ({ ...x, library: e.target.value }))} />
          </label>
          <label>
            Reference <input value={def.prefix} onChange={(e) => set((d) => void (d.prefix = e.target.value))} />
          </label>
          <label>
            Value <input value={def.value} onChange={(e) => set((d) => void (d.value = e.target.value))} />
          </label>
          <label>
            Description <input value={def.description} onChange={(e) => set((d) => void (d.description = e.target.value))} />
          </label>
          <label>
            Keywords <input value={(def.keywords ?? []).join(" ")} onChange={(e) => set((d) => void (d.keywords = e.target.value.split(/\s+/).filter(Boolean)))} />
          </label>
          <label>
            Footprint
            <input list="lib-fp-names" value={def.footprint} onChange={(e) => set((d) => void (d.footprint = e.target.value))} />
          </label>
          <datalist id="lib-fp-names">
            {props.footprintNames.map((n) => (
              <option key={n} value={n} />
            ))}
          </datalist>
          <label>
            MPN <input value={def.mpn ?? ""} onChange={(e) => set((d) => void (d.mpn = e.target.value || null))} />
          </label>
          <label>
            LCSC <input value={def.lcsc ?? ""} onChange={(e) => set((d) => void (d.lcsc = e.target.value || null))} />
          </label>
          <label>
            Derived from <input placeholder="Library:Name or built-in (empty: own body)" value={s.extends} onChange={(e) => setS((x) => ({ ...x, extends: e.target.value }))} />
          </label>
          <div className="lib-checks">
            <label>
              <input type="checkbox" checked={def.show_pin_numbers} onChange={(e) => set((d) => void (d.show_pin_numbers = e.target.checked))} /> Pin numbers
            </label>
            <label>
              <input type="checkbox" checked={def.show_pin_names} onChange={(e) => set((d) => void (d.show_pin_names = e.target.checked))} /> Pin names
            </label>
            <label>
              <input type="checkbox" checked={!!s.power} onChange={(e) => setS((x) => ({ ...x, power: e.target.checked ? { net: x.def.value || "VCC", style: "bar" } : null }))} /> Power symbol
            </label>
          </div>
          {s.power && (
            <div className="lib-row">
              <label>
                Net <input value={s.power.net} onChange={(e) => setS((x) => ({ ...x, power: { ...x.power!, net: e.target.value } }))} />
              </label>
              <label>
                Look
                <select value={s.power.style ?? "bar"} onChange={(e) => setS((x) => ({ ...x, power: { ...x.power!, style: e.target.value } }))}>
                  {["bar", "ground", "arrow", "circle"].map((k) => (
                    <option key={k}>{k}</option>
                  ))}
                </select>
              </label>
            </div>
          )}
        </section>
        <section>
          <h4>
            Fields
            <button type="button" className="lib-add" onClick={() => set((d) => void ((d.fields = d.fields ?? {})[`Field${Object.keys(d.fields ?? {}).length + 1}`] = ""))}>
              +
            </button>
          </h4>
          {Object.entries(def.fields ?? {}).map(([k, v]) => (
            <div className="lib-row" key={k}>
              <input
                defaultValue={k}
                onBlur={(e) =>
                  set((d) => {
                    const val = d.fields![k];
                    delete d.fields![k];
                    if (e.target.value) d.fields![e.target.value] = val;
                  })
                }
              />
              <input value={v} onChange={(e) => set((d) => void (d.fields![k] = e.target.value))} />
              <button type="button" className="lib-x" onClick={() => set((d) => void delete d.fields![k])}>
                ×
              </button>
            </div>
          ))}
        </section>
        <section>
          <h4>
            Graphics ({style === 2 ? "De Morgan" : "normal"}, unit {String.fromCharCode(64 + unit)})
          </h4>
          <div className="lib-row wrap">
            {[
              ["rect", { kind: "rect", a: [-5.08, -5.08], b: [5.08, 5.08], fill: "body" }],
              ["line", { kind: "line", pts: [[-2.54, 0], [2.54, 0]], width: 0 }],
              ["poly", { kind: "poly", pts: [[-2.54, -2.54], [2.54, 0], [-2.54, 2.54]], fill: "none" }],
              ["circle", { kind: "circle", c: [0, 0], r: 2.54, fill: "none" }],
              ["arc", { kind: "arc", c: [0, 0], r: 2.54, start: 0, end: 180 }],
              ["text", { kind: "text", at: [0, 0], text: "Text", size: 1.27 }],
            ].map(([k, g]) => (
              <button type="button" key={k as string} className="secondary small" onClick={() => setUnitGfx([...(units[unit - 1] ?? []), g])}>
                + {k as string}
              </button>
            ))}
          </div>
          {(units[unit - 1] ?? []).map((g: any, i: number) => (
            <GfxRow
              key={i}
              g={g}
              onChange={(ng) => setUnitGfx((units[unit - 1] ?? []).map((x: any, j: number) => (j === i ? ng : x)))}
              onDelete={() => setUnitGfx((units[unit - 1] ?? []).filter((_: any, j: number) => j !== i))}
            />
          ))}
        </section>
        <Findings
          list={preview?.findings}
          onPick={(f) => {
            const i = def.pins.findIndex((p) => f.items?.includes(p.number));
            if (i >= 0) {
              setSelPin(i);
              if (def.pins[i].unit) setUnit(def.pins[i].unit!);
            }
          }}
        />
        <div className="lib-actions">
          <button type="button" className="primary" disabled={!props.canEdit} onClick={save}>
            Save to {s.library}
          </button>
          {s.renameFrom && (
            <>
              <button type="button" className="secondary" disabled={!props.canEdit || !props.schematic} onClick={() => props.commit("pcb.lib.place", { schematic: props.schematic, symbol: s.renameFrom, at: [50.8, 50.8], body_style: style }, `Placed ${s.renameFrom}`)}>
                Place
              </button>
              <button type="button" className="secondary" disabled={!props.canEdit} onClick={async () => {
                const o = outOf(await props.commit("pcb.lib.symbol.push", { symbol: s.renameFrom }));
                if (o) props.notify(`Updated ${o.updated.length} placed part(s)`);
              }}>
                Update placed
              </button>
              <a className="secondary lib-link" href={projectUrl(`/pcb/libs/symbol/${encodeURIComponent(s.renameFrom)}/svg?download=1&unit=${unit}`)}>
                SVG
              </a>
              <button type="button" className="secondary" disabled={!props.canEdit} onClick={() => setS((x) => ({ ...x, def: { ...x.def, name: `${x.def.name}_copy` }, renameFrom: null }))}>
                Duplicate
              </button>
              <button type="button" className="secondary danger" disabled={!props.canEdit} onClick={async () => {
                if (await props.commit("pcb.lib.delete", { symbols: [s.renameFrom] }, `Deleted ${s.renameFrom}`)) props.onSaved();
              }}>
                Delete
              </button>
            </>
          )}
        </div>
      </div>
      <div className="lib-table-wrap">
        <div className="lib-table-head">
          <h4>Pins</h4>
          <button type="button" className="secondary small" onClick={() => {
            set((d) => {
              const nums = d.pins.map((p) => Number(p.number)).filter(Number.isFinite);
              const n = String((nums.length ? Math.max(...nums) : 0) + 1);
              d.pins.push({ number: n, name: `P${n}`, at: [-10.16, snap(2.54 * d.pins.length - 5.08)], side: "left", length: 2.54, etype: "passive", ...(unitCount > 1 ? { unit } : {}) });
            });
          }}>
            + Pin
          </button>
          {selPin !== null && (
            <>
              <button type="button" className="secondary small" title="Give every pin this pin's length" onClick={() => set((d) => d.pins.forEach((p) => (p.length = d.pins[selPin].length)))}>
                Push length
              </button>
              <button type="button" className="secondary small" title="Give every pin this pin's type" onClick={() => set((d) => d.pins.forEach((p) => (p.etype = d.pins[selPin].etype)))}>
                Push type
              </button>
            </>
          )}
          {preview?.footprint_pads && <span className="lib-muted">footprint pads: {preview.footprint_pads.join(" ")}</span>}
        </div>
        <table className="lib-table">
          <thead>
            <tr>
              <th>No.</th>
              <th>Name</th>
              <th>Type</th>
              <th>Shape</th>
              <th>Side</th>
              <th>Length</th>
              <th>X</th>
              <th>Y</th>
              <th>Unit</th>
              <th>Hidden</th>
              <th>Pads</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {def.pins.map((p, i) => (
              <tr key={i} className={selPin === i ? "sel" : ""} onClick={() => setSelPin(i)}>
                <td>
                  <input value={p.number} onChange={(e) => set((d) => void (d.pins[i].number = e.target.value))} />
                </td>
                <td>
                  <input value={p.name} onChange={(e) => set((d) => void (d.pins[i].name = e.target.value))} />
                </td>
                <td>
                  <select value={p.etype} onChange={(e) => set((d) => void (d.pins[i].etype = e.target.value))}>
                    {PIN_TYPES.map((t) => (
                      <option key={t}>{t}</option>
                    ))}
                  </select>
                </td>
                <td>
                  <select value={p.shape ?? "line"} onChange={(e) => set((d) => void (d.pins[i].shape = e.target.value === "line" ? undefined : e.target.value))}>
                    {PIN_SHAPES.map((t) => (
                      <option key={t}>{t}</option>
                    ))}
                  </select>
                </td>
                <td>
                  <select value={p.side} onChange={(e) => set((d) => void (d.pins[i].side = e.target.value))}>
                    {SIDES.map((t) => (
                      <option key={t}>{t}</option>
                    ))}
                  </select>
                </td>
                <td>
                  <input type="number" step={1.27} value={p.length} onChange={(e) => set((d) => void (d.pins[i].length = Number(e.target.value)))} />
                </td>
                <td>
                  <input type="number" step={1.27} value={p.at[0]} onChange={(e) => set((d) => void (d.pins[i].at = [Number(e.target.value), d.pins[i].at[1]]))} />
                </td>
                <td>
                  <input type="number" step={1.27} value={p.at[1]} onChange={(e) => set((d) => void (d.pins[i].at = [d.pins[i].at[0], Number(e.target.value)]))} />
                </td>
                <td>
                  <select value={p.unit ?? 0} onChange={(e) => set((d) => void (d.pins[i].unit = Number(e.target.value) || undefined))}>
                    <option value={0}>all</option>
                    {Array.from({ length: unitCount }, (_, k) => (
                      <option key={k} value={k + 1}>
                        {String.fromCharCode(65 + k)}
                      </option>
                    ))}
                  </select>
                </td>
                <td>
                  <input type="checkbox" checked={!!p.hidden} onChange={(e) => set((d) => void (d.pins[i].hidden = e.target.checked || undefined))} />
                </td>
                <td>
                  <input value={(p.pads ?? []).join(",")} placeholder={p.number} onChange={(e) => set((d) => void (d.pins[i].pads = e.target.value.split(",").map((x) => x.trim()).filter(Boolean)))} />
                </td>
                <td>
                  <button type="button" className="lib-x" onClick={() => set((d) => void d.pins.splice(i, 1))}>
                    ×
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// ---- Footprint editor ------------------------------------------------------------

type Pad = { number: string; kind: string; shape: string; size: P; at: P; rotation?: number; drill?: P | null; corner?: number | null; paste?: number[][]; polygon?: P[]; chamfer?: { ratio: number; corners: string[] } | null; trapezoid?: P | null; stack?: any; zone_connection?: string | null; thermal_gap?: number | null; thermal_spoke?: number | null; drill_offset?: P | null; no_paste?: boolean };
type Body = { shape: string; at: P; size: P; z: P; material: string };
type Fp = { name: string; description: string; source: string; pads: Pad[]; courtyard?: number[] | null; silk?: P[][]; body: Body[]; kind: string };
type FpState = { library: string; fp: Fp; model: { file?: string; offset: [number, number, number]; rotation: number } | null; renameFrom: string | null };

function blankFootprint(): FpState {
  return {
    library: "Project",
    renameFrom: null,
    model: null,
    fp: {
      name: "NewFootprint",
      description: "",
      source: "",
      kind: "ic",
      pads: [
        { number: "1", kind: "smd", shape: "round_rect", size: [1.0, 0.6], at: [-1.5, 0], corner: 0.25 },
        { number: "2", kind: "smd", shape: "round_rect", size: [1.0, 0.6], at: [1.5, 0], corner: 0.25 },
      ],
      body: [{ shape: "box", at: [0, 0], size: [2, 1.2], z: [0, 0.8], material: "Epoxy" }],
    },
  };
}

const PAD_SHAPES = ["rect", "round_rect", "circle", "oval", "custom", "chamfered", "trapezoid"];

function shapeOf(p: Pad) {
  return p.chamfer ? "chamfered" : p.trapezoid ? "trapezoid" : p.shape;
}

function FootprintView({ view, sel, onPad, grid, dragPad }: { view: any; sel: number | null; onPad?: (i: number | null) => void; grid: number; dragPad?: (i: number, p: P) => void }) {
  const drag = useRef<{ i: number; off: P } | null>(null);
  const cy = (view?.courtyard as number[]) ?? [-3, -3, 3, 3];
  const box: [number, number, number, number] = [cy[0], cy[1], cy[2], cy[3]];
  const flip = (pts: P[]) => pts.map((p) => `${p[0]},${-p[1]}`).join(" ");
  return (
    <Canvas
      box={box}
      flipY
      grid={grid}
      dark
      onPointer={(kind, p) => {
        if (kind === "down") {
          const hit = [...(view?.pads ?? [])].reverse().find((q: any) => {
            const xs = q.outline.map((o: P) => o[0]);
            const ys = q.outline.map((o: P) => o[1]);
            return p[0] >= Math.min(...xs) && p[0] <= Math.max(...xs) && p[1] >= Math.min(...ys) && p[1] <= Math.max(...ys);
          });
          onPad?.(hit ? hit.index : null);
          if (hit && dragPad) drag.current = { i: hit.index, off: [hit.at[0] - p[0], hit.at[1] - p[1]] };
        } else if (kind === "move" && drag.current && dragPad) {
          dragPad(drag.current.i, [snap(p[0] + drag.current.off[0], grid), snap(p[1] + drag.current.off[1], grid)]);
        } else if (kind === "up") drag.current = null;
      }}
    >
      {(view?.footprint?.body ?? []).map((b: Body, i: number) =>
        b.shape === "cylinder" ? (
          <circle key={`b${i}`} cx={b.at[0]} cy={-b.at[1]} r={b.size[0] / 2} className="lib-fab" />
        ) : (
          <rect key={`b${i}`} x={b.at[0] - b.size[0] / 2} y={-b.at[1] - b.size[1] / 2} width={b.size[0]} height={b.size[1]} className="lib-fab" />
        ),
      )}
      {(view?.pads ?? []).map((q: any) => (
        <g key={q.index}>
          <polygon points={flip(q.outline)} className={`lib-pad ${q.kind} ${sel === q.index ? "sel" : ""}`} />
          {q.hole && <ellipse cx={q.hole.at[0]} cy={-q.hole.at[1]} rx={q.hole.size[0] / 2} ry={q.hole.size[1] / 2} className="lib-hole" />}
          {q.number && (
            <text x={q.at[0]} y={-q.at[1]} className="lib-padnum" fontSize={Math.max(0.25, Math.min(1, Math.min(...[0, 1].map((k) => Math.max(...q.outline.map((o: P) => o[k])) - Math.min(...q.outline.map((o: P) => o[k])))) * 0.55))}>
              {q.number}
            </text>
          )}
        </g>
      ))}
      {(view?.silk ?? []).map((l: P[], i: number) => (
        <polyline key={`s${i}`} points={flip(l)} className="lib-silk" />
      ))}
      <rect x={cy[0]} y={-cy[3]} width={cy[2] - cy[0]} height={cy[3] - cy[1]} className="lib-courtyard" />
      {(view?.findings ?? [])
        .filter((f: Finding) => f.at && f.severity !== "note")
        .map((f: Finding, i: number) => (
          <circle key={`f${i}`} cx={f.at![0]} cy={-f.at![1]} r={0.4} className={`lib-mark ${f.severity}`} />
        ))}
    </Canvas>
  );
}

function Num({ v, on, step = 0.05, w }: { v: number | undefined | null; on: (n: number) => void; step?: number; w?: number }) {
  return <input type="number" step={step} value={v ?? ""} style={w ? { width: w } : undefined} onChange={(e) => on(Number(e.target.value))} />;
}

function FootprintEditor(props: { start: FpState; commit: Commit; canEdit: boolean; boardId: string | null; onSaved: () => void; notify: (m: string) => void; onWizard: () => void }) {
  const [s, setS] = useState<FpState>(props.start);
  const [sel, setSel] = useState<number | null>(null);
  const [view, setView] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const [grid, setGrid] = useState(0.05);
  useEffect(() => setS(props.start), [props.start]);
  const fp = s.fp;
  const set = (f: (d: Fp) => void) =>
    setS((x) => {
      const d = structuredClone(x.fp);
      f(d);
      return { ...x, fp: d };
    });
  const setPad = (i: number, f: (p: Pad) => void) => set((d) => f(d.pads[i]));
  const dq = useDebounced(s, 180);
  useEffect(() => {
    let live = true;
    postJson("/pcb/libs/footprint/preview", { footprint: dq.fp, model: dq.model, library: dq.library })
      .then((r) => live && (setView(r), setError(null)))
      .catch((e) => live && setError(String(e.message ?? e)));
    return () => {
      live = false;
    };
  }, [dq]);
  const save = async () => {
    const out = outOf(await props.commit("pcb.lib.footprint.save", { library: s.library, footprint: fp, model: s.model, rename_from: s.renameFrom }, `Saved ${s.library}:${fp.name}`));
    if (out) {
      setS((x) => ({ ...x, renameFrom: `${x.library}:${x.fp.name}` }));
      props.onSaved();
    }
  };
  const p = sel !== null ? fp.pads[sel] : null;
  const setShape = (i: number, sh: string) =>
    setPad(i, (q) => {
      q.chamfer = undefined;
      q.trapezoid = undefined;
      if (sh === "chamfered") {
        q.chamfer = { ratio: 0.2, corners: ["top_left"] };
        q.shape = "custom";
      } else if (sh === "trapezoid") {
        q.trapezoid = [0, 0.2];
        q.shape = "custom";
      } else {
        q.shape = sh;
        if (sh === "custom" && !(q.polygon?.length ?? 0)) q.polygon = [[-q.size[0] / 2, -q.size[1] / 2], [q.size[0] / 2, -q.size[1] / 2], [q.size[0] / 2, q.size[1] / 2], [-q.size[0] / 2, q.size[1] / 2]];
        if (sh !== "custom") q.polygon = undefined;
      }
    });
  return (
    <div className="lib-editor">
      <div className="lib-stage">
        <div className="lib-stagebar">
          <span className="lib-title">
            {s.library}:{fp.name}
          </span>
          <label className="lib-inline">
            Grid
            <select value={grid} onChange={(e) => setGrid(Number(e.target.value))}>
              {[0.01, 0.05, 0.1, 0.25, 0.5, 1.27].map((g) => (
                <option key={g} value={g}>
                  {g} mm
                </option>
              ))}
            </select>
          </label>
          <button type="button" className="secondary" onClick={props.onWizard}>
            <Icon name="spark" /> Wizard
          </button>
          <button
            type="button"
            className="secondary"
            onClick={async () => {
              try {
                const r = await postJson("/pcb/libs/footprint/preview", { footprint: fp, enumerate: [1, "", true] });
                setS((x) => ({ ...x, fp: r.edited }));
              } catch (e) {
                props.notify(String((e as Error).message));
              }
            }}
            title="Number pads top to bottom, left to right"
          >
            Renumber
          </button>
          <span className="lib-muted">{view ? `${fp.pads.length} pads · ${view.height} mm tall` : ""}</span>
        </div>
        <FootprintView view={view} sel={sel} onPad={setSel} grid={grid} dragPad={props.canEdit ? (i, at) => setPad(i, (q) => void (q.at = at)) : undefined} />
        {error && <p className="lib-error">{error}</p>}
        <div className="lib-hint">Drag pads · gold copper, white silk, pink courtyard, grey dashed body · wheel zooms · Alt-drag pans</div>
      </div>
      <div className="lib-props">
        <section>
          <h4>Footprint</h4>
          <label>
            Name <input value={fp.name} onChange={(e) => set((d) => void (d.name = e.target.value))} />
          </label>
          <label>
            Library <input value={s.library} onChange={(e) => setS((x) => ({ ...x, library: e.target.value }))} />
          </label>
          <label>
            Description <input value={fp.description} onChange={(e) => set((d) => void (d.description = e.target.value))} />
          </label>
          <label>
            Source <input value={fp.source} placeholder="datasheet drawing used" onChange={(e) => set((d) => void (d.source = e.target.value))} />
          </label>
          <label>
            Kind
            <select value={fp.kind} onChange={(e) => set((d) => void (d.kind = e.target.value))}>
              {["resistor", "capacitor", "led", "diode", "inductor", "ic", "connector", "switch", "module", "hole", "logo"].map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </label>
          <div className="lib-row">
            <label className="lib-inline">
              <input type="checkbox" checked={!fp.courtyard} onChange={(e) => set((d) => void (d.courtyard = e.target.checked ? null : view?.courtyard))} /> Courtyard from pads + body
            </label>
          </div>
          {fp.courtyard && (
            <div className="lib-row">
              {[0, 1, 2, 3].map((k) => (
                <Num key={k} v={fp.courtyard![k]} on={(n) => set((d) => void (d.courtyard![k] = n))} w={60} />
              ))}
            </div>
          )}
          <div className="lib-row">
            <label className="lib-inline">
              <input type="checkbox" checked={!(fp.silk?.length)} onChange={(e) => set((d) => void (d.silk = e.target.checked ? [] : (view?.silk ?? [])))} /> Silkscreen from the body outline
            </label>
          </div>
        </section>
        <section>
          <h4>
            Body (3D and height)
            <button type="button" className="lib-add" onClick={() => set((d) => void d.body.push({ shape: "box", at: [0, 0], size: [1, 1], z: [0, 1], material: "Epoxy" }))}>
              +
            </button>
          </h4>
          {fp.body.map((b, i) => (
            <div className="lib-row wrap" key={i}>
              <select value={b.shape} onChange={(e) => set((d) => void (d.body[i].shape = e.target.value))}>
                <option>box</option>
                <option>cylinder</option>
              </select>
              at <Num v={b.at[0]} on={(n) => set((d) => void (d.body[i].at[0] = n))} w={52} />
              <Num v={b.at[1]} on={(n) => set((d) => void (d.body[i].at[1] = n))} w={52} />
              size <Num v={b.size[0]} on={(n) => set((d) => void (d.body[i].size[0] = n))} w={52} />
              <Num v={b.size[1]} on={(n) => set((d) => void (d.body[i].size[1] = n))} w={52} />
              z <Num v={b.z[0]} on={(n) => set((d) => void (d.body[i].z[0] = n))} w={48} />
              <Num v={b.z[1]} on={(n) => set((d) => void (d.body[i].z[1] = n))} w={48} />
              <select value={b.material} onChange={(e) => set((d) => void (d.body[i].material = e.target.value))}>
                {["Epoxy", "Ceramic", "Steel", "Nylon", "Black plastic", "Gold", "LED", "FR4"].map((m) => (
                  <option key={m}>{m}</option>
                ))}
              </select>
              <button type="button" className="lib-x" onClick={() => set((d) => void d.body.splice(i, 1))}>
                ×
              </button>
            </div>
          ))}
          <label className="lib-inline">
            <input type="checkbox" checked={!!s.model} onChange={(e) => setS((x) => ({ ...x, model: e.target.checked ? { offset: [0, 0, 0], rotation: 0 } : null }))} /> 3D model placement
          </label>
          {s.model && (
            <div className="lib-row wrap">
              file <input value={s.model.file ?? ""} placeholder="STEP/WRL path (kept for exports)" onChange={(e) => setS((x) => ({ ...x, model: { ...x.model!, file: e.target.value || undefined } }))} />
              offset
              {[0, 1, 2].map((k) => (
                <Num key={k} v={s.model!.offset[k]} on={(n) => setS((x) => ({ ...x, model: { ...x.model!, offset: x.model!.offset.map((o, j) => (j === k ? n : o)) as [number, number, number] } }))} w={52} />
              ))}
              turn <Num v={s.model.rotation} step={90} on={(n) => setS((x) => ({ ...x, model: { ...x.model!, rotation: n } }))} w={56} />°
            </div>
          )}
        </section>
        {p && sel !== null && (
          <section className="lib-pad-props">
            <h4>Pad {p.number || "(no number)"}</h4>
            <div className="lib-row wrap">
              <select value={p.kind} onChange={(e) => setPad(sel, (q) => {
                q.kind = e.target.value;
                q.drill = e.target.value === "smd" ? null : (q.drill ?? [Math.min(q.size[0], q.size[1]) * 0.5, Math.min(q.size[0], q.size[1]) * 0.5]);
              })}>
                <option value="smd">SMD</option>
                <option value="tht">Through-hole</option>
                <option value="npth">Hole (no copper)</option>
              </select>
              <select value={shapeOf(p)} onChange={(e) => setShape(sel, e.target.value)}>
                {PAD_SHAPES.map((k) => (
                  <option key={k}>{k}</option>
                ))}
              </select>
              {p.shape === "round_rect" && !p.chamfer && (
                <>
                  corner <Num v={p.corner ?? 0.25} step={0.05} on={(n) => setPad(sel, (q) => void (q.corner = n))} w={52} />
                </>
              )}
            </div>
            {p.chamfer && (
              <div className="lib-row wrap">
                chamfer <Num v={p.chamfer.ratio} step={0.05} on={(n) => setPad(sel, (q) => void (q.chamfer!.ratio = n))} w={52} />
                {["top_left", "top_right", "bottom_left", "bottom_right"].map((c) => (
                  <label key={c} className="lib-inline">
                    <input type="checkbox" checked={p.chamfer!.corners.includes(c)} onChange={(e) => setPad(sel, (q) => void (q.chamfer!.corners = e.target.checked ? [...q.chamfer!.corners, c] : q.chamfer!.corners.filter((x) => x !== c)))} />
                    {c.replace("_", " ")}
                  </label>
                ))}
              </div>
            )}
            {p.trapezoid && (
              <div className="lib-row">
                taller left by <Num v={p.trapezoid[0]} on={(n) => setPad(sel, (q) => void (q.trapezoid = [n, q.trapezoid![1]]))} w={56} /> wider bottom by <Num v={p.trapezoid[1]} on={(n) => setPad(sel, (q) => void (q.trapezoid = [q.trapezoid![0], n]))} w={56} />
              </div>
            )}
            {p.shape === "custom" && !p.chamfer && !p.trapezoid && (
              <label>
                Outline (x,y …, pad coordinates)
                <input
                  defaultValue={(p.polygon ?? []).map((q) => q.join(",")).join(" ")}
                  key={sel + (p.polygon ?? []).length}
                  onBlur={(e) => {
                    const pts = e.target.value
                      .trim()
                      .split(/\s+/)
                      .map((t) => t.split(",").map(Number) as P)
                      .filter((q) => q.length === 2 && q.every(Number.isFinite));
                    if (pts.length >= 3) setPad(sel, (q) => void (q.polygon = pts));
                  }}
                />
              </label>
            )}
            {p.kind !== "smd" && (
              <div className="lib-row wrap">
                drill <Num v={p.drill?.[0]} on={(n) => setPad(sel, (q) => void (q.drill = [n, q.drill?.[1] ?? n]))} w={52} />
                <Num v={p.drill?.[1]} on={(n) => setPad(sel, (q) => void (q.drill = [q.drill?.[0] ?? n, n]))} w={52} />
                offset <Num v={p.drill_offset?.[0] ?? 0} on={(n) => setPad(sel, (q) => void (q.drill_offset = n || (q.drill_offset?.[1] ?? 0) ? [n, q.drill_offset?.[1] ?? 0] : null))} w={52} />
                <Num v={p.drill_offset?.[1] ?? 0} on={(n) => setPad(sel, (q) => void (q.drill_offset = n || (q.drill_offset?.[0] ?? 0) ? [q.drill_offset?.[0] ?? 0, n] : null))} w={52} />
              </div>
            )}
            {p.kind === "tht" && (
              <div className="lib-row wrap">
                <label className="lib-inline">
                  <input type="checkbox" checked={!!p.stack} onChange={(e) => setPad(sel, (q) => void (q.stack = e.target.checked ? { inner: { shape: "circle", size: [q.drill ? q.drill[0] + 0.4 : 1, q.drill ? q.drill[0] + 0.4 : 1] }, opposite: null, remove_inner: false } : null))} /> Pad stack
                </label>
                {p.stack && (
                  <>
                    inner ø<Num v={p.stack.inner?.size?.[0]} on={(n) => setPad(sel, (q) => void (q.stack.inner = { shape: "circle", size: [n, n] }))} w={52} />
                    far side <Num v={p.stack.opposite?.size?.[0]} on={(n) => setPad(sel, (q) => void (q.stack.opposite = n ? { shape: q.shape === "custom" ? "rect" : q.shape, size: [n, n * (q.size[1] / q.size[0])] } : null))} w={52} />
                    <label className="lib-inline">
                      <input type="checkbox" checked={!!p.stack.remove_inner} onChange={(e) => setPad(sel, (q) => void (q.stack.remove_inner = e.target.checked))} /> no inner copper
                    </label>
                  </>
                )}
              </div>
            )}
            <div className="lib-row wrap">
              pour
              <select value={p.zone_connection ?? ""} onChange={(e) => setPad(sel, (q) => void (q.zone_connection = e.target.value || null))}>
                <option value="">as the zone</option>
                <option value="thermal">thermal</option>
                <option value="solid">solid</option>
                <option value="none">not joined</option>
              </select>
              gap <Num v={p.thermal_gap} on={(n) => setPad(sel, (q) => void (q.thermal_gap = n || null))} w={50} /> spoke <Num v={p.thermal_spoke} on={(n) => setPad(sel, (q) => void (q.thermal_spoke = n || null))} w={50} />
              {p.kind === "smd" && (
                <label className="lib-inline">
                  <input type="checkbox" checked={!!p.no_paste} onChange={(e) => setPad(sel, (q) => void (q.no_paste = e.target.checked || undefined))} /> no paste
                </label>
              )}
            </div>
            <div className="lib-row wrap">
              <button type="button" className="secondary small" onClick={() => set((d) => d.pads.forEach((q) => {
                if (q.kind === d.pads[sel].kind) {
                  q.size = [...d.pads[sel].size] as P;
                  q.shape = d.pads[sel].shape;
                  q.corner = d.pads[sel].corner;
                  q.drill = d.pads[sel].drill;
                }
              }))} title="Give every pad of this kind this pad's size, shape and drill">
                Push to all
              </button>
              <button type="button" className="secondary small" onClick={() => set((d) => {
                const q = structuredClone(d.pads[sel]);
                q.at = [r4(q.at[0] + 1), q.at[1]];
                const nums = d.pads.map((x) => Number(x.number)).filter(Number.isFinite);
                q.number = String((nums.length ? Math.max(...nums) : 0) + 1);
                d.pads.push(q);
              })}>
                Duplicate
              </button>
            </div>
          </section>
        )}
        <Findings list={view?.findings} onPick={(f) => {
          const i = fp.pads.findIndex((q) => f.items?.includes(q.number));
          if (i >= 0) setSel(i);
        }} />
        <div className="lib-actions">
          <button type="button" className="primary" disabled={!props.canEdit} onClick={save}>
            Save to {s.library}
          </button>
          {s.renameFrom && (
            <>
              <button type="button" className="secondary" disabled={!props.canEdit || !props.boardId} onClick={() => props.commit("pcb.lib.footprint.push", { footprint: s.renameFrom }, "Updated on the boards that use it")}>
                Update boards
              </button>
              <button type="button" className="secondary" disabled={!props.canEdit || !props.boardId} onClick={() => props.commit("pcb.component.create", { board: props.boardId, reference: fp.kind === "logo" ? "LOGO1" : "U99", footprint: s.renameFrom, at: [5, 5] }, `Placed ${s.renameFrom} on the board`)}>
                Place on board
              </button>
              <a className="secondary lib-link" href={projectUrl(`/pcb/libs/footprint/${encodeURIComponent(s.renameFrom)}/svg?download=1`)}>
                SVG
              </a>
              <button type="button" className="secondary danger" disabled={!props.canEdit} onClick={async () => {
                if (await props.commit("pcb.lib.delete", { footprints: [s.renameFrom] }, `Deleted ${s.renameFrom}`)) props.onSaved();
              }}>
                Delete
              </button>
            </>
          )}
        </div>
      </div>
      <div className="lib-table-wrap">
        <div className="lib-table-head">
          <h4>Pads</h4>
          <button type="button" className="secondary small" onClick={() => set((d) => {
            const nums = d.pads.map((x) => Number(x.number)).filter(Number.isFinite);
            const last = d.pads[d.pads.length - 1];
            d.pads.push({ ...(last ? structuredClone(last) : { kind: "smd", shape: "round_rect", size: [1, 0.6], corner: 0.25 }), number: String((nums.length ? Math.max(...nums) : 0) + 1), at: last ? [last.at[0], r4(last.at[1] - 1.27)] : [0, 0] } as Pad);
          })}>
            + Pad
          </button>
        </div>
        <table className="lib-table">
          <thead>
            <tr>
              <th>No.</th>
              <th>Kind</th>
              <th>Shape</th>
              <th>X</th>
              <th>Y</th>
              <th>W</th>
              <th>H</th>
              <th>Rot</th>
              <th>Drill</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {fp.pads.map((q, i) => (
              <tr key={i} className={sel === i ? "sel" : ""} onClick={() => setSel(i)}>
                <td>
                  <input value={q.number} onChange={(e) => setPad(i, (x) => void (x.number = e.target.value))} />
                </td>
                <td>{q.kind}</td>
                <td>{shapeOf(q)}</td>
                <td>
                  <Num v={q.at[0]} on={(n) => setPad(i, (x) => void (x.at = [n, x.at[1]]))} />
                </td>
                <td>
                  <Num v={q.at[1]} on={(n) => setPad(i, (x) => void (x.at = [x.at[0], n]))} />
                </td>
                <td>
                  <Num v={q.size[0]} on={(n) => setPad(i, (x) => void (x.size = [n, x.size[1]]))} />
                </td>
                <td>
                  <Num v={q.size[1]} on={(n) => setPad(i, (x) => void (x.size = [x.size[0], n]))} />
                </td>
                <td>
                  <Num v={q.rotation ?? 0} step={90} on={(n) => setPad(i, (x) => void (x.rotation = n))} />
                </td>
                <td>{q.drill ? q.drill.join("×") : "—"}</td>
                <td>
                  <button type="button" className="lib-x" onClick={() => {
                    set((d) => void d.pads.splice(i, 1));
                    setSel(null);
                  }}>
                    ×
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// ---- Wizard dialog ---------------------------------------------------------------

function WizardDialog(props: { onDone: (fp: Fp) => void; onClose: () => void }) {
  const [presets, setPresets] = useState<{ name: string; wizard: any }[]>([]);
  const [w, setW] = useState<any>(null);
  const [view, setView] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    getJson("/pcb/libs/wizards").then((r) => {
      setPresets(r.presets);
      setW(structuredClone(r.presets[0].wizard));
    });
  }, []);
  const dw = useDebounced(w, 200);
  useEffect(() => {
    if (!dw) return;
    postJson("/pcb/libs/wizard", dw)
      .then((r) => (setView(r), setError(null)))
      .catch((e) => setError(String(e.message ?? e)));
  }, [dw]);
  const field = (k: string) => {
    const v = w[k];
    const text = Array.isArray(v) ? v.map((x: any) => (Array.isArray(x) ? x.join("–") : x)).join(", ") : v && typeof v === "object" ? JSON.stringify(v) : String(v ?? "");
    return (
      <label key={k + JSON.stringify(w.wizard)}>
        {k}
        <input
          defaultValue={text}
          onBlur={(e) => {
            const t = e.target.value.trim();
            let val: any = t;
            if (t === "") val = null;
            else if (/^[-\d.]+$/.test(t)) val = Number(t);
            else if (/^[-\d.]+\s*[–-]\s*[-\d.]+$/.test(t) && !Array.isArray(v)) val = t.split(/\s*[–]\s*|(?<=\d)\s*-\s*/).map(Number);
            else if (Array.isArray(v)) val = t.split(",").map((x) => (/[–]/.test(x) ? x.split("–").map(Number) : /^[-\d. ]+$/.test(x) ? Number(x) : x.trim()));
            else if (t.startsWith("{") || t.startsWith("[")) {
              try {
                val = JSON.parse(t);
              } catch {
                return;
              }
            }
            setW({ ...w, [k]: val });
          }}
        />
      </label>
    );
  };
  return (
    <div className="lib-dialog" role="dialog" aria-label="Footprint wizard">
      <div className="lib-dialog-card">
        <header>
          <h3>Footprint wizard</h3>
          <span className="lib-muted">IPC-7351B land patterns from the package's dimensions (ranges as min–max)</span>
          <button type="button" className="lib-x" onClick={props.onClose}>
            ×
          </button>
        </header>
        <div className="lib-dialog-body">
          <div className="lib-wiz-form">
            <label>
              Package
              <select onChange={(e) => setW(structuredClone(presets[Number(e.target.value)].wizard))}>
                {presets.map((p, i) => (
                  <option key={p.name} value={i}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
            {w && Object.keys(w).filter((k) => k !== "wizard").map(field)}
            {error && <p className="lib-error">{error}</p>}
          </div>
          <div className="lib-wiz-preview">
            {view && <FootprintView view={view} sel={null} grid={0.5} />}
            {view && <p className="lib-muted">{view.footprint.description}</p>}
          </div>
        </div>
        <footer>
          <button type="button" className="primary" disabled={!view} onClick={() => props.onDone(view.footprint)}>
            Edit this footprint
          </button>
        </footer>
      </div>
    </div>
  );
}

// ---- Logo (Bitmap2Component) -----------------------------------------------------

function LogoTab(props: { commit: Commit; canEdit: boolean; boardId: string | null; onSaved: () => void }) {
  const [img, setImg] = useState<{ w: number; h: number; grey: Uint8Array; url: string } | null>(null);
  const [threshold, setThreshold] = useState(128);
  const [size, setSize] = useState(12);
  const [negative, setNegative] = useState(false);
  const [layer, setLayer] = useState("silk");
  const [name, setName] = useState("Logo");
  const [view, setView] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const load = (file: File) => {
    const url = URL.createObjectURL(file);
    const im = new Image();
    im.onload = () => {
      const max = 512;
      const k = Math.min(1, max / Math.max(im.width, im.height));
      const w = Math.max(1, Math.round(im.width * k));
      const h = Math.max(1, Math.round(im.height * k));
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      const x = c.getContext("2d")!;
      x.fillStyle = "#fff";
      x.fillRect(0, 0, w, h);
      x.drawImage(im, 0, 0, w, h);
      const d = x.getImageData(0, 0, w, h).data;
      const grey = new Uint8Array(w * h);
      for (let i = 0; i < w * h; i++) grey[i] = Math.round(0.299 * d[i * 4] + 0.587 * d[i * 4 + 1] + 0.114 * d[i * 4 + 2]);
      setImg({ w, h, grey, url });
      setName(file.name.replace(/\.[^.]+$/, "").replace(/[^A-Za-z0-9_-]+/g, "_") || "Logo");
    };
    im.src = url;
  };
  const req = useDebounced(img ? { img, threshold, size, negative, layer } : null, 250);
  useEffect(() => {
    if (!req) return;
    let s = "";
    const g = req.img.grey;
    for (let i = 0; i < g.length; i += 0x8000) s += String.fromCharCode(...g.subarray(i, i + 0x8000));
    postJson("/pcb/libs/bitmap", { width: req.img.w, height: req.img.h, pixels: btoa(s), threshold: req.threshold, negative: req.negative, size: req.size, layer: req.layer, name })
      .then((r) => (setView(r), setError(null)))
      .catch((e) => (setView(null), setError(String(e.message ?? e))));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [req]);
  const save = async (place: boolean) => {
    if (!view) return;
    if (view.kind === "symbol") {
      if (await props.commit("pcb.lib.symbol.save", { library: "Project", symbol: { ...view.symbol, name }, origin: "image" }, `Saved Project:${name}`)) props.onSaved();
      return;
    }
    const out = outOf(await props.commit("pcb.lib.footprint.save", { library: "Project", footprint: { ...view.footprint, name }, origin: "image" }, `Saved Project:${name}`));
    if (out) {
      props.onSaved();
      if (place && props.boardId) await props.commit("pcb.component.create", { board: props.boardId, reference: "LOGO1", footprint: out.footprint, at: [5, 5] }, "Logo placed on the board");
    }
  };
  return (
    <div className="lib-logo">
      <div className="lib-logo-controls">
        <label className="lib-file">
          <Icon name="plus" /> Image (PNG, JPG, SVG)
          <input type="file" accept="image/*" onChange={(e) => e.target.files?.[0] && load(e.target.files[0])} />
        </label>
        {img && <img src={img.url} alt="" className="lib-logo-src" />}
        <label>
          Threshold {threshold}
          <input type="range" min={1} max={254} value={threshold} onChange={(e) => setThreshold(Number(e.target.value))} />
        </label>
        <label>
          Width (mm) <input type="number" step={0.5} value={size} onChange={(e) => setSize(Number(e.target.value))} />
        </label>
        <label className="lib-inline">
          <input type="checkbox" checked={negative} onChange={(e) => setNegative(e.target.checked)} /> Negative (light parts are the art)
        </label>
        <label>
          Make
          <select value={layer} onChange={(e) => setLayer(e.target.value)}>
            <option value="silk">Silkscreen logo</option>
            <option value="copper">Copper logo (exposed, no paste)</option>
            <option value="symbol">Schematic graphic</option>
          </select>
        </label>
        <label>
          Name <input value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        {error && <p className="lib-error">{error}</p>}
        <div className="lib-actions">
          <button type="button" className="primary" disabled={!view || !props.canEdit} onClick={() => save(false)}>
            Save to Project library
          </button>
          {view?.kind !== "symbol" && (
            <button type="button" className="secondary" disabled={!view || !props.canEdit || !props.boardId} onClick={() => save(true)}>
              Save and place on board
            </button>
          )}
        </div>
      </div>
      <div className="lib-logo-preview">
        {view?.kind === "symbol" ? (
          <Canvas box={view.preview.bbox} grid={G}>
            {view.preview.draw.map((d: Draw, i: number) => (
              <DrawEl key={i} d={d} />
            ))}
          </Canvas>
        ) : view ? (
          <FootprintView view={view} sel={null} grid={1} />
        ) : (
          <div className="lib-empty">Choose an image: dark parts become the logo (or light ones, negative).</div>
        )}
        {view && view.kind !== "symbol" && <p className="lib-muted">{view.footprint.description}</p>}
      </div>
    </div>
  );
}

// ---- Footprint assignment (CvPcb) --------------------------------------------------

function AssignTab(props: { schematic: string | null; commit: Commit; canEdit: boolean; revision: number }) {
  const [data, setData] = useState<any>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [fits, setFits] = useState(true);
  const [q, setQ] = useState("");
  const [pending, setPending] = useState<Record<string, string>>({});
  const [look, setLook] = useState<string | null>(null);
  const [view, setView] = useState<any>(null);
  const [kicad, setKicad] = useState<any[]>([]);
  useEffect(() => {
    if (props.schematic) getJson(`/pcb/libs/assign/${props.schematic}`).then(setData).catch(() => setData(null));
  }, [props.schematic, props.revision]);
  const part = data?.parts.find((p: any) => p.reference === sel);
  const list = useMemo(() => {
    if (!data || !part) return [];
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    return data.footprints
      .filter((f: any) => !fits || (f.pads <= part.pads_all && part.pads.every((n: string) => f.numbers.includes(n))))
      .filter((f: any) => words.every((w) => `${f.name} ${f.description}`.toLowerCase().includes(w)))
      .sort((a: any, b: any) => Number(part.candidates.includes(b.name)) - Number(part.candidates.includes(a.name)));
  }, [data, part, fits, q]);
  useEffect(() => {
    if (!look) return setView(null);
    const path = look.startsWith("kicad:") ? `/pcb/kicad/footprint/${encodeURIComponent(look.slice(6))}` : `/pcb/libs/footprint/${encodeURIComponent(look)}`;
    getJson(path)
      .then(async (r) => (look.startsWith("kicad:") ? postJson("/pcb/libs/footprint/preview", { footprint: r.footprint }) : r))
      .then(setView)
      .catch(() => setView(null));
  }, [look]);
  const searchKicad = async () => {
    if (!part) return;
    try {
      const r = await getJson(`/pcb/kicad/search?kind=footprints&limit=60&q=${encodeURIComponent(q || part.value)}`);
      setKicad(r.results.filter((x: any) => !fits || (x.count >= part.pads.length && x.count <= part.pads_all)));
    } catch (e) {
      setKicad([{ id: String((e as Error).message), count: 0 }]);
    }
  };
  if (!props.schematic) return <div className="lib-empty">Assigning footprints needs a schematic that drives this board.</div>;
  if (!data) return <div className="lib-empty">Loading…</div>;
  return (
    <div className="lib-assign">
      <div className="lib-assign-parts">
        <table className="lib-table">
          <thead>
            <tr>
              <th>Ref</th>
              <th>Value</th>
              <th>Pads</th>
              <th>Footprint</th>
            </tr>
          </thead>
          <tbody>
            {data.parts.map((p: any) => (
              <tr key={p.reference} className={sel === p.reference ? "sel" : ""} onClick={() => setSel(p.reference)}>
                <td>{p.reference}</td>
                <td>{p.value}</td>
                <td>{p.pads.length}</td>
                <td className={pending[p.reference] !== undefined ? "lib-pending" : !p.footprint ? "lib-missing" : ""}>{pending[p.reference] ?? (p.footprint || "—")}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="lib-actions">
          <button type="button" className="primary" disabled={!props.canEdit || !Object.keys(pending).length} onClick={async () => {
            const out = outOf(await props.commit("pcb.lib.assign", { schematic: props.schematic, footprints: pending }, `Assigned ${Object.keys(pending).length} footprint(s)`));
            if (out) {
              setPending({});
              if (out.warnings?.length) alert(out.warnings.join("\n"));
            }
          }}>
            Apply {Object.keys(pending).length || ""}
          </button>
        </div>
      </div>
      <div className="lib-assign-list">
        {part ? (
          <>
            <div className="lib-row">
              <input placeholder="Filter by name or words" value={q} onChange={(e) => setQ(e.target.value)} />
              <label className="lib-inline">
                <input type="checkbox" checked={fits} onChange={(e) => setFits(e.target.checked)} /> fits {part.pads.length} pads
              </label>
              <button type="button" className="secondary small" onClick={searchKicad}>
                KiCad
              </button>
            </div>
            <div className="lib-fp-list">
              {list.map((f: any) => (
                <button type="button" key={f.source + f.name} className={`lib-fp-item ${look === f.name ? "sel" : ""}`} onMouseEnter={() => setLook(f.name)} onClick={() => setPending((x) => ({ ...x, [part.reference]: f.name }))}>
                  <b>{f.name}</b> <span className="lib-tag">{f.source}</span>
                  {part.candidates.includes(f.name) && <span className="lib-tag good">suggested</span>}
                  <small>{f.description}</small>
                </button>
              ))}
              {kicad.map((k: any) => (
                <button type="button" key={k.id} className={`lib-fp-item ${look === `kicad:${k.id}` ? "sel" : ""}`} onMouseEnter={() => setLook(`kicad:${k.id}`)} onClick={async () => {
                  const f = await getJson(`/pcb/kicad/footprint/${encodeURIComponent(k.id)}`);
                  const o = outOf(await props.commit("pcb.lib.footprint.save", { library: "KiCad-copies", footprint: { ...f.footprint, name: k.id.split(":")[1] }, origin: `KiCad ${k.id}` }));
                  if (o) setPending((x) => ({ ...x, [part.reference]: o.footprint }));
                }}>
                  <b>{k.id}</b> <span className="lib-tag">KiCad · {k.count} pads</span>
                  <small>{k.description}</small>
                </button>
              ))}
              {!list.length && !kicad.length && <div className="lib-empty">No footprint here fits; try KiCad, or make one with the wizard.</div>}
            </div>
          </>
        ) : (
          <div className="lib-empty">Pick a part to see the footprints that fit its pins.</div>
        )}
      </div>
      <div className="lib-assign-preview">{view ? <FootprintView view={view} sel={null} grid={0.5} /> : <div className="lib-empty">Hover a footprint to see it.</div>}</div>
    </div>
  );
}

// ---- Variants -------------------------------------------------------------------------

function VariantsTab(props: { schematic: string | null; commit: Commit; canEdit: boolean; revision: number }) {
  const [data, setData] = useState<any>(null);
  const [cur, setCur] = useState<string | null>(null);
  const [name, setName] = useState("");
  useEffect(() => {
    if (props.schematic) getJson(`/pcb/variants/${props.schematic}`).then(setData).catch(() => setData(null));
  }, [props.schematic, props.revision]);
  if (!props.schematic) return <div className="lib-empty">Variants belong to a schematic that drives this board.</div>;
  if (!data) return <div className="lib-empty">Loading…</div>;
  const v = data.variants.find((x: any) => x.id === cur) ?? data.variants[0];
  const fitted = (ref: string, dnp: boolean) => {
    if (!v) return !dnp;
    const has = (l?: string[]) => (l ?? []).some((x) => x.toLowerCase() === ref.toLowerCase());
    return has(v.variant.dnp) ? false : has(v.variant.fitted) ? true : !dnp;
  };
  const ov = (ref: string) => v?.variant.overrides?.[ref] ?? {};
  const change = (ref: string, c: Record<string, unknown>) => props.commit("pcb.variant.update", { target: v.id, parts: { [ref]: c } });
  return (
    <div className="lib-variants">
      <div className="lib-row wrap">
        {data.variants.map((x: any) => (
          <button type="button" key={x.id} className={`lib-chip ${v?.id === x.id ? "active" : ""}`} onClick={() => setCur(x.id)}>
            {x.variant.name} <span className="lib-muted">{x.fitted ?? "?"} fitted</span>
          </button>
        ))}
        <input placeholder="New variant" value={name} onChange={(e) => setName(e.target.value)} />
        <button type="button" className="secondary small" disabled={!name || !props.canEdit} onClick={async () => {
          const o = outOf(await props.commit("pcb.variant.create", { schematic: props.schematic, name, ...(v ? { copy_from: v.variant.name } : {}) }, `Variant ${name}`));
          if (o) {
            setCur(o.id);
            setName("");
          }
        }}>
          {v ? `+ Copy of ${v.variant.name}` : "+ Variant"}
        </button>
      </div>
      {v && (
        <>
          <div className="lib-row wrap">
            <label>
              Name <input key={v.id} defaultValue={v.variant.name} onBlur={(e) => e.target.value !== v.variant.name && props.commit("pcb.variant.update", { target: v.id, name: e.target.value })} />
            </label>
            <label>
              Description <input key={v.id + "d"} defaultValue={v.variant.description ?? ""} onBlur={(e) => props.commit("pcb.variant.update", { target: v.id, description: e.target.value })} />
            </label>
            {data.board && (
              <>
                <a className="secondary lib-link" href={projectUrl(`/pcb/${data.board}/variant/${encodeURIComponent(v.variant.name)}/bom.csv?format=jlcpcb`)}>
                  BOM
                </a>
                <a className="secondary lib-link" href={projectUrl(`/pcb/${data.board}/variant/${encodeURIComponent(v.variant.name)}/positions.csv?format=jlcpcb`)}>
                  Positions
                </a>
                <a className="secondary lib-link" href={projectUrl(`/pcb/${data.board}/variant/${encodeURIComponent(v.variant.name)}/fab.zip?fab=jlcpcb`)}>
                  Fab package
                </a>
              </>
            )}
            <button type="button" className="secondary danger small" disabled={!props.canEdit} onClick={() => props.commit("pcb.variant.delete", { target: v.id }, "Variant deleted")}>
              Delete
            </button>
          </div>
          <table className="lib-table">
            <thead>
              <tr>
                <th>Ref</th>
                <th>Fitted</th>
                <th>Value</th>
                <th>MPN</th>
                <th>LCSC</th>
                <th>Design</th>
              </tr>
            </thead>
            <tbody>
              {data.parts.map((p: any) => (
                <tr key={p.reference} className={fitted(p.reference, p.dnp) ? "" : "lib-dnp"}>
                  <td>{p.reference}</td>
                  <td>
                    <input type="checkbox" disabled={!props.canEdit} checked={fitted(p.reference, p.dnp)} onChange={(e) => change(p.reference, { dnp: e.target.checked === !p.dnp ? null : !e.target.checked })} />
                  </td>
                  {(["value", "mpn", "lcsc"] as const).map((k) => (
                    <td key={k}>
                      <input key={v.id + k + (ov(p.reference)[k] ?? "")} defaultValue={ov(p.reference)[k] ?? ""} placeholder={p[k] ?? ""} onBlur={(e) => (e.target.value || ov(p.reference)[k]) && e.target.value !== (ov(p.reference)[k] ?? "") && change(p.reference, { [k]: e.target.value })} />
                    </td>
                  ))}
                  <td className="lib-muted">
                    {p.value}
                    {p.dnp ? " · DNP" : ""}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      {!data.variants.length && <div className="lib-empty">No variants: the board is built as drawn. Add one to leave parts off or change values per build.</div>}
    </div>
  );
}

// ---- Design blocks ----------------------------------------------------------------------

function BlocksTab(props: { schematic: string | null; commit: Commit; canEdit: boolean; revision: number }) {
  const [blocks, setBlocks] = useState<any[]>([]);
  const [parts, setParts] = useState<string[]>([]);
  const [picked, setPicked] = useState<string[]>([]);
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [at, setAt] = useState("150,100");
  const [boardAt, setBoardAt] = useState("");
  useEffect(() => {
    getJson("/pcb/blocks").then((r) => setBlocks(r.blocks)).catch(() => setBlocks([]));
    if (props.schematic) getJson(`/pcb/variants/${props.schematic}`).then((r) => setParts(r.parts.map((p: any) => p.reference))).catch(() => setParts([]));
  }, [props.schematic, props.revision]);
  const xyOf = (s: string) => s.split(",").map(Number) as P;
  return (
    <div className="lib-blocks">
      <section>
        <h4>Save a block from the schematic</h4>
        <div className="lib-row wrap">
          {parts.map((r) => (
            <label key={r} className={`lib-chip ${picked.includes(r) ? "active" : ""}`}>
              <input type="checkbox" checked={picked.includes(r)} onChange={(e) => setPicked((x) => (e.target.checked ? [...x, r] : x.filter((y) => y !== r)))} />
              {r}
            </label>
          ))}
        </div>
        <div className="lib-row wrap">
          <input placeholder="Block name (USB-C power)" value={name} onChange={(e) => setName(e.target.value)} />
          <input placeholder="Description" value={desc} onChange={(e) => setDesc(e.target.value)} />
          <button type="button" className="primary small" disabled={!props.canEdit || !name || !picked.length || !props.schematic} onClick={async () => {
            if (await props.commit("pcb.block.save", { schematic: props.schematic, name, references: picked, description: desc, replace: blocks.some((b) => b.block.name === name) }, `Saved block ${name}`)) {
              setPicked([]);
              getJson("/pcb/blocks").then((r) => setBlocks(r.blocks));
            }
          }}>
            {blocks.some((b) => b.block.name === name) ? "Update block" : "Save block"}
          </button>
        </div>
      </section>
      <section>
        <h4>Blocks</h4>
        <div className="lib-row wrap">
          <label>
            Sheet x,y <input value={at} onChange={(e) => setAt(e.target.value)} />
          </label>
          <label>
            Board x,y <input placeholder="(schematic only)" value={boardAt} onChange={(e) => setBoardAt(e.target.value)} />
          </label>
        </div>
        {blocks.map((b) => (
          <div key={b.id} className="lib-block">
            <div>
              <b>{b.block.name}</b> <span className="lib-muted">{b.block.description}</span>
              <div className="lib-muted">
                {b.block.symbols.map((s: any) => s.reference).join(", ")} · {b.block.wires.length} wires · {b.block.labels.length} labels
                {b.block.layout ? ` · layout: ${b.block.layout.parts.length} parts, ${b.block.layout.copper.length} copper` : ""}
              </div>
            </div>
            <button type="button" className="secondary small" disabled={!props.canEdit || !props.schematic} onClick={() => props.commit("pcb.block.place", { block: b.block.name, schematic: props.schematic, at: xyOf(at), ...(boardAt ? { board_at: xyOf(boardAt) } : {}) }, `Placed ${b.block.name}`)}>
              Place
            </button>
            <button type="button" className="secondary small danger" disabled={!props.canEdit} onClick={async () => {
              if (await props.commit("pcb.block.delete", { target: b.id }, `Deleted ${b.block.name}`)) setBlocks((x) => x.filter((y) => y.id !== b.id));
            }}>
              Delete
            </button>
          </div>
        ))}
        {!blocks.length && <div className="lib-empty">No design blocks yet: tick the parts of a circuit you will use again and save them.</div>}
      </section>
    </div>
  );
}

// ---- Library table ------------------------------------------------------------------

function TableTab(props: { libs: any; commit: Commit; canEdit: boolean; reload: () => void; notify: (m: string) => void }) {
  const { libs } = props;
  const [exportName, setExportName] = useState("MyParts");
  const table: any[] = libs.table ?? [];
  const rows: { name: string; kind: string; detail: string }[] = [
    ...libs.project.map((l: any) => ({ name: l.name, kind: "project", detail: `${l.symbols.length} symbols, ${l.footprints.length} footprints in this project` })),
    ...libs.global.libraries.map((l: any) => ({ name: l.name, kind: "global", detail: `${l.symbols.length} symbols, ${l.footprints.length} footprints on this computer` })),
    { name: "KiCad", kind: "kicad", detail: `official libraries: footprints ${libs.kicad.footprints ? "downloaded" : "not downloaded"}, symbols ${libs.kicad.symbols ? "downloaded" : "not downloaded"} (${libs.kicad.root})` },
    { name: "Forma", kind: "builtin", detail: `${libs.builtin.symbols.length} symbols, ${libs.builtin.footprints.length} footprints (IPC-7351B)` },
  ];
  const entry = (r: { name: string; kind: string }) => table.find((t) => t.name === r.name && t.kind === r.kind) ?? { name: r.name, kind: r.kind, enabled: true, pinned: false };
  const setEntry = (r: { name: string; kind: string }, patch: Record<string, unknown>) => {
    const next = rows.map((x) => (x.name === r.name && x.kind === r.kind ? { ...entry(x), ...patch } : entry(x)));
    props.commit("pcb.lib.table.set", { libraries: next });
  };
  return (
    <div className="lib-tablepage">
      <table className="lib-table">
        <thead>
          <tr>
            <th>Pinned</th>
            <th>Library</th>
            <th>Kind</th>
            <th>Use</th>
            <th>What</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {rows
            .sort((a, b) => Number(entry(b).pinned) - Number(entry(a).pinned))
            .map((r) => (
              <tr key={r.kind + r.name}>
                <td>
                  <input type="checkbox" disabled={!props.canEdit} checked={!!entry(r).pinned} onChange={(e) => setEntry(r, { pinned: e.target.checked })} />
                </td>
                <td>
                  <b>{r.name}</b>
                </td>
                <td>{r.kind}</td>
                <td>
                  <input type="checkbox" disabled={!props.canEdit} checked={entry(r).enabled !== false} onChange={(e) => setEntry(r, { enabled: e.target.checked })} />
                </td>
                <td className="lib-muted">{r.detail}</td>
                <td>
                  {r.kind === "global" && (
                    <button type="button" className="secondary small" disabled={!props.canEdit} onClick={async () => {
                      const lib = await getJson(`/pcb/libs/global/${encodeURIComponent(r.name)}`);
                      if (await props.commit("pcb.lib.import", { library: r.name, symbols: lib.symbols, footprints: lib.footprints, origin: `global library ${r.name}` }, `Copied ${r.name} into the project`)) props.reload();
                    }}>
                      Copy into project
                    </button>
                  )}
                  {r.kind === "project" && (
                    <button type="button" className="secondary small" onClick={async () => {
                      try {
                        const o = await postJson(`/pcb/libs/global/${encodeURIComponent(exportName)}/from-project?library=${encodeURIComponent(r.name)}`, {});
                        props.notify(`Saved ${o.added_symbols} symbols and ${o.added_footprints} footprints to ${o.name} on this computer`);
                        props.reload();
                      } catch (e) {
                        props.notify(String((e as Error).message));
                      }
                    }}>
                      Save to computer as {exportName}
                    </button>
                  )}
                </td>
              </tr>
            ))}
        </tbody>
      </table>
      <div className="lib-row">
        <label>
          Computer library name for saving <input value={exportName} onChange={(e) => setExportName(e.target.value)} />
        </label>
        <span className="lib-muted">Libraries on this computer: {libs.global.root}</span>
      </div>
    </div>
  );
}

// ---- The view ----------------------------------------------------------------------

type Section = "symbols" | "footprints" | "logo" | "assign" | "variants" | "blocks" | "libraries";

export function LibraryPane(props: { boardId: string | null; revision: number; commit: Commit; notify: (m: string) => void; canEdit: boolean }) {
  const [section, setSection] = useState<Section>(() => {
    try {
      return (localStorage.getItem("forma.pcb.lib.section") as Section) || "symbols";
    } catch {
      return "symbols";
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("forma.pcb.lib.section", section);
    } catch {
      /* private window */
    }
  }, [section]);
  const [libs, setLibs] = useState<any>(null);
  const [schematic, setSchematic] = useState<string | null>(null);
  const [sym, setSym] = useState<SymState | null>(null);
  const [fp, setFp] = useState<FpState | null>(null);
  const [wizard, setWizard] = useState(false);
  const [filter, setFilter] = useState("");
  const [copyFrom, setCopyFrom] = useState("");
  const reload = useCallback(() => {
    getJson("/pcb/libs").then(setLibs).catch((e) => props.notify(String(e.message ?? e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(reload, [reload, props.revision]);
  useEffect(() => {
    getJson("/pcb/schematics")
      .then((list: any[]) => setSchematic(list.find((s) => s.board === props.boardId)?.id ?? list[0]?.id ?? null))
      .catch(() => setSchematic(null));
  }, [props.boardId, props.revision]);
  const openSymbol = async (id: string) => {
    try {
      const r = await getJson(`/pcb/libs/symbol/${encodeURIComponent(id)}`);
      if (r.kind === "project") setSym({ library: r.entry.library, def: r.entry.symbol, extends: r.entry.extends ?? "", power: r.entry.power ?? null, renameFrom: id });
      else setSym({ library: "Project", def: r.definition, extends: "", power: null, renameFrom: null });
    } catch (e) {
      props.notify(String((e as Error).message));
    }
  };
  const openFootprint = async (id: string) => {
    try {
      const r = await getJson(`/pcb/libs/footprint/${encodeURIComponent(id)}`);
      if (r.kind === "project") setFp({ library: r.entry.library, fp: r.entry.footprint, model: r.entry.model ?? null, renameFrom: id });
      else setFp({ library: "Project", fp: r.footprint, model: null, renameFrom: null });
    } catch (e) {
      props.notify(String((e as Error).message));
    }
  };
  const fpNames = useMemo(() => (libs ? [...libs.project.flatMap((l: any) => l.footprints.map((f: any) => f.full)), ...libs.builtin.footprints] : []), [libs]);
  const match = (s: string) => !filter || s.toLowerCase().includes(filter.toLowerCase());
  // The library table: libraries switched off are not offered; pinned ones come first.
  const row = (name: string, kind: string) => (libs?.table ?? []).find((t: any) => t.name === name && t.kind === kind);
  const projectLibs: any[] = (libs?.project ?? []).filter((l: any) => row(l.name, "project")?.enabled !== false).sort((a: any, b: any) => Number(!!row(b.name, "project")?.pinned) - Number(!!row(a.name, "project")?.pinned));
  const builtinOn = row("Forma", "builtin")?.enabled !== false;
  const tabs: [Section, string][] = [
    ["symbols", "Symbols"],
    ["footprints", "Footprints"],
    ["logo", "Logo"],
    ["assign", "Assign footprints"],
    ["variants", "Variants"],
    ["blocks", "Design blocks"],
    ["libraries", "Libraries"],
  ];
  return (
    <div className="lib-pane">
      <nav className="lib-nav" role="tablist">
        {tabs.map(([k, label]) => (
          <button key={k} type="button" role="tab" aria-selected={section === k} className={section === k ? "active" : ""} onClick={() => setSection(k)}>
            {label}
          </button>
        ))}
      </nav>
      {!libs ? (
        <div className="lib-empty">Loading libraries…</div>
      ) : section === "symbols" || section === "footprints" ? (
        <div className="lib-main">
          <aside className="lib-list">
            <input placeholder="Filter" value={filter} onChange={(e) => setFilter(e.target.value)} />
            <div className="lib-row">
              <button type="button" className="secondary small" onClick={() => (section === "symbols" ? setSym(blankSymbol()) : setFp(blankFootprint()))}>
                + New
              </button>
              {section === "footprints" && (
                <button type="button" className="secondary small" onClick={() => setWizard(true)}>
                  Wizard
                </button>
              )}
            </div>
            <div className="lib-row">
              <input list="lib-copy-src" placeholder={`Copy a built-in ${section === "symbols" ? "symbol" : "footprint"}`} value={copyFrom} onChange={(e) => setCopyFrom(e.target.value)} />
              <datalist id="lib-copy-src">
                {(section === "symbols" ? libs.builtin.symbols : libs.builtin.footprints).map((n: string) => (
                  <option key={n} value={n} />
                ))}
              </datalist>
              <button type="button" className="secondary small" disabled={!copyFrom} onClick={() => (section === "symbols" ? openSymbol(copyFrom) : openFootprint(copyFrom))}>
                Open
              </button>
            </div>
            {projectLibs.filter((l: any) => (section === "symbols" ? l.symbols : l.footprints).length).map((l: any) => (
              <div key={l.name} className="lib-group">
                <h5>{l.name}</h5>
                {(section === "symbols" ? l.symbols : l.footprints)
                  .filter((x: any) => match(x.full))
                  .map((x: any) => (
                    <button type="button" key={x.full} className={`lib-item ${(section === "symbols" ? sym?.renameFrom : fp?.renameFrom) === x.full ? "sel" : ""}`} onClick={() => (section === "symbols" ? openSymbol(x.full) : openFootprint(x.full))}>
                      {x.name}
                      <small>{section === "symbols" ? `${x.pins} pins${x.units > 1 ? ` · ${x.units} units` : ""}${x.extends ? ` · from ${x.extends}` : ""}${x.power ? " · power" : ""}${x.demorgan ? " · De Morgan" : ""}` : `${x.pads} pads · ${x.kind}`}</small>
                    </button>
                  ))}
              </div>
            ))}
            <div className="lib-group" hidden={!builtinOn}>
              <h5>Built-in</h5>
              {(section === "symbols" ? libs.builtin.symbols : libs.builtin.footprints)
                .filter(match)
                .slice(0, 80)
                .map((n: string) => (
                  <button type="button" key={n} className="lib-item builtin" onClick={() => (section === "symbols" ? openSymbol(n) : openFootprint(n))}>
                    {n}
                  </button>
                ))}
            </div>
          </aside>
          {section === "symbols" ? (
            sym ? (
              <SymbolEditor start={sym} commit={props.commit} canEdit={props.canEdit} schematic={schematic} footprintNames={fpNames} onSaved={reload} notify={props.notify} />
            ) : (
              <div className="lib-empty">Open a symbol, copy a built-in one to change it, or start a new one.</div>
            )
          ) : fp ? (
            <FootprintEditor start={fp} commit={props.commit} canEdit={props.canEdit} boardId={props.boardId} onSaved={reload} notify={props.notify} onWizard={() => setWizard(true)} />
          ) : (
            <div className="lib-empty">Open a footprint, run a wizard (SOIC, QFN, QFP, BGA, DIP, headers…), or start a new one.</div>
          )}
          {wizard && (
            <WizardDialog
              onClose={() => setWizard(false)}
              onDone={(f) => {
                setFp({ library: "Project", fp: f as Fp, model: null, renameFrom: null });
                setWizard(false);
                setSection("footprints");
              }}
            />
          )}
        </div>
      ) : section === "logo" ? (
        <LogoTab commit={props.commit} canEdit={props.canEdit} boardId={props.boardId} onSaved={reload} />
      ) : section === "assign" ? (
        <AssignTab schematic={schematic} commit={props.commit} canEdit={props.canEdit} revision={props.revision} />
      ) : section === "variants" ? (
        <VariantsTab schematic={schematic} commit={props.commit} canEdit={props.canEdit} revision={props.revision} />
      ) : section === "blocks" ? (
        <BlocksTab schematic={schematic} commit={props.commit} canEdit={props.canEdit} revision={props.revision} />
      ) : (
        <TableTab libs={libs} commit={props.commit} canEdit={props.canEdit} reload={reload} notify={props.notify} />
      )}
    </div>
  );
}
