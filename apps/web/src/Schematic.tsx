// The schematic view of the Electronics workspace: the circuit drawn as
// symbols, wires, labels and power ports on paper, its nets and
// electrical-rules check, and "Update board" with the change report. The
// engine resolves everything into drawing primitives (GET /pcb/schematic/<id>)
// so screen and PDF match; this draws them and sends edits back as
// pcb.schematic.* commands. Selecting a part or net here selects it on the board too.

import { type PointerEvent as ReactPointerEvent, type MutableRefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";

type P = [number, number];
type Draw =
  | { t: "line"; p: P[]; w: number; ink: string }
  | { t: "poly"; p: P[]; w: number; ink: string; fill: "none" | "ink" | "body" }
  | { t: "circle"; c: P; r: number; w: number; ink: string; fill: "none" | "ink" | "body" }
  | { t: "text"; at: P; s: string; size: number; angle: number; anchor: "start" | "middle" | "end"; ink: string };
type Dir = "left" | "right" | "up" | "down";
type PinV = { number: string; name: string; etype: string; at: P; dir: Dir; hidden: boolean; net: string | null; pads: string[] };
type SymbolV = {
  id: string;
  sheet: string;
  symbol: string;
  reference: string;
  value: string;
  footprint: string;
  unit: number;
  units: number;
  at: P;
  rotation: number;
  mirror: boolean;
  mpn: string | null;
  manufacturer: string | null;
  lcsc: string | null;
  description: string;
  dnp: boolean;
  virtual_part: boolean;
  component: string | null;
  known: boolean;
  category: string;
  body: [number, number, number, number];
  bbox: [number, number, number, number];
  draw: Draw[];
  pins: PinV[];
};
type WireV = { id: string; sheet: string; points: P[]; net: string | null };
type LabelV = { id: string; sheet: string; kind: "local" | "global" | "hierarchical" | "power"; text: string; at: P; rotation: number; net: string | null; draw: Draw[]; bbox: [number, number, number, number] };
type SheetV = { id: string; name: string; paper: string; size: P; parent: string | null; frame: Draw[]; boxes_draw: Draw[] };
type NetV = { name: string; pins: string[]; pads: string[]; labels: string[]; sheets: string[]; named: boolean };
type Erc = { code: string; severity: "error" | "warning"; message: string; sheet: string; at: P; objects: string[]; refs: string[] };
type SchematicV = {
  id: string;
  name: string;
  board: string | null;
  sheets: SheetV[];
  symbols: SymbolV[];
  wires: WireV[];
  labels: LabelV[];
  junctions: { id: string | null; sheet: string; at: P }[];
  no_connects: { id: string; sheet: string; at: P }[];
  texts: { id: string; sheet: string; at: P; text: string; size: number }[];
  nets: NetV[];
  violations: Erc[];
  stats: { sheets: number; symbols: number; parts: number; wires: number; labels: number; nets: number; errors: number; warnings: number };
};
type SyncReport = {
  error?: string;
  added: { reference: string; footprint: string; value: string; pads: number }[];
  changed: { reference: string; changes: string[] }[];
  nets: { pad: string; from: string | null; to: string | null }[];
  removed: { reference: string; footprint: string; value: string }[];
  board_only: { reference: string; footprint: string; why: string }[];
  skipped: { reference: string; why: string }[];
  new_nets: string[];
  gone_nets: string[];
  unchanged: number;
  erc_errors: number;
  in_sync: boolean;
};
type Detail = { schematic: SchematicV; errors: number; sync: SyncReport };
type LibSymbol = { name: string; description: string; prefix: string; category: string; value: string; footprint: string; footprints: string[]; units: number; keywords: string[]; pins: number; virtual: boolean };
type Commit = (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown>;
type Sel = { kind: "symbol" | "wire" | "label" | "junction" | "nc" | "text"; id: string } | null;
type Mode = { m: "select" } | { m: "wire"; pts: P[] } | { m: "label"; kind: "local" | "global" } | { m: "power" } | { m: "nc" } | { m: "junction" } | { m: "text" } | { m: "place"; symbol: string; rotation: number; mirror: boolean };
type Tab = "parts" | "add" | "nets" | "erc" | "sync";

/** Ink roles on paper; the PDF uses the same colours (schematic_pdf.rs). */
const INK: Record<string, string> = {
  body: "#8a1f1f",
  pin: "#8a1f1f",
  body_fill: "#fdf6dc",
  pin_number: "#9b4a3a",
  pin_name: "#0d6b6b",
  reference: "#0d6b6b",
  value: "#0d6b6b",
  wire: "#1b7f3d",
  junction: "#1b7f3d",
  label: "#1d2430",
  global: "#7a3aa8",
  hierarchical: "#a05a00",
  power: "#8a1f1f",
  no_connect: "#2a5bd7",
  note: "#3c4a63",
  frame: "#c79a9a",
  frame_text: "#8e4a4a",
  frame_label: "#b88f8f",
  sheet: "#8a5a00",
  dnp: "#d03030",
  error: "#d03030",
};
const FONT = "Helvetica, Arial, sans-serif";
const GRID = 1.27;
const POWER_NETS = ["GND", "3V3", "5V", "VBUS", "VCC", "12V", "VIN"];
const snap = (v: number) => Math.round(v / GRID) * GRID;
const snapP = (p: P): P => [+snap(p[0]).toFixed(4), +snap(p[1]).toFixed(4)];
const same = (a: P, b: P) => Math.abs(a[0] - b[0]) < 1e-3 && Math.abs(a[1] - b[1]) < 1e-3;

function pts(list: P[]) {
  return list.map((p) => `${p[0]},${p[1]}`).join(" ");
}

/** One engine primitive as SVG. `hl` colours it for a highlight. */
function Prim({ d, hl, dim }: { d: Draw; hl?: string; dim?: boolean }) {
  const color = hl ?? INK[d.ink] ?? "#222";
  const op = dim ? 0.28 : 1;
  switch (d.t) {
    case "line":
      return <polyline points={pts(d.p)} fill="none" stroke={color} strokeWidth={d.w} strokeLinecap="round" strokeLinejoin="round" opacity={op} />;
    case "poly":
      return <polygon points={pts(d.p)} fill={d.fill === "ink" ? color : d.fill === "body" ? INK.body_fill : "none"} stroke={color} strokeWidth={d.w} strokeLinejoin="round" opacity={op} />;
    case "circle":
      return <circle cx={d.c[0]} cy={d.c[1]} r={d.r} fill={d.fill === "ink" ? color : d.fill === "body" ? INK.body_fill : "none"} stroke={color} strokeWidth={d.w} opacity={op} />;
    case "text":
      if (!d.s) return null;
      return (
        <text
          x={d.at[0]}
          y={d.at[1]}
          dy={d.size * 0.35}
          fontSize={d.size}
          fontFamily={FONT}
          textAnchor={d.anchor}
          fill={color}
          opacity={op}
          transform={d.angle ? `rotate(-90 ${d.at[0]} ${d.at[1]})` : undefined}
          fontWeight={d.ink === "reference" || d.ink === "frame_text" ? 600 : 400}
        >
          {d.s}
        </text>
      );
  }
}

function bboxOf(list: Draw[]): [number, number, number, number] {
  let b: [number, number, number, number] = [Infinity, Infinity, -Infinity, -Infinity];
  const add = (p: P) => (b = [Math.min(b[0], p[0]), Math.min(b[1], p[1]), Math.max(b[2], p[0]), Math.max(b[3], p[1])]);
  for (const d of list) {
    if (d.t === "line" || d.t === "poly") d.p.forEach(add);
    else if (d.t === "circle") {
      add([d.c[0] - d.r, d.c[1] - d.r]);
      add([d.c[0] + d.r, d.c[1] + d.r]);
    } else add(d.at);
  }
  return b;
}

export function SchematicPane(props: {
  boardId: string;
  boardRefs: { id: string; reference: string }[];
  revision: number;
  commit: Commit;
  notify: (m: string) => void;
  canEdit: boolean;
  probeRef: string | null;
  onProbe: (reference: string | null) => void;
  net: string | null;
  onNet: (n: string | null) => void;
  compact?: boolean;
  keys: MutableRefObject<((e: KeyboardEvent) => boolean) | null>;
}) {
  const { commit, notify } = props;
  const [schId, setSchId] = useState<string | null | undefined>(undefined);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sheet, setSheet] = useState<string>("1");
  const [tab, setTab] = useState<Tab>("parts");
  const [sel, setSel] = useState<Sel>(null);
  const [mode, setMode] = useState<Mode>({ m: "select" });
  const [focus, setFocus] = useState<{ at: P; n: number } | null>(null);
  const [lib, setLib] = useState<LibSymbol[] | null>(null);
  const [powerNet, setPowerNet] = useState("GND");
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    fetch(projectUrl("/pcb/schematics"))
      .then((r) => r.json())
      .then((list: { id: string; board: string | null }[]) => {
        if (!live) return;
        setSchId(list.find((s) => s.board === props.boardId)?.id ?? null);
      })
      .catch(() => live && setSchId(null));
    return () => {
      live = false;
    };
  }, [props.revision, props.boardId]);

  useEffect(() => {
    if (!schId) {
      setDetail(null);
      return;
    }
    let live = true;
    fetch(projectUrl(`/pcb/schematic/${schId}`))
      .then(async (r) => {
        if (!r.ok) throw new Error(await r.text());
        return r.json();
      })
      .then((d: Detail) => {
        if (!live) return;
        setDetail(d);
        setError(null);
        if (!d.schematic.sheets.some((s) => s.id === sheet)) setSheet(d.schematic.sheets[0]?.id ?? "1");
      })
      .catch((e) => live && setError(String(e.message ?? e)));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [schId, props.revision]);

  useEffect(() => {
    if (lib) return;
    fetch("/pcb/symbols")
      .then((r) => r.json())
      .then((d: { symbols: LibSymbol[] }) => setLib(d.symbols))
      .catch(() => setLib([]));
  }, [lib]);

  const v = detail?.schematic ?? null;
  const selSym = sel?.kind === "symbol" ? (v?.symbols.find((s) => s.id === sel.id) ?? null) : null;
  const selLabel = sel?.kind === "label" ? (v?.labels.find((s) => s.id === sel.id) ?? null) : null;

  // Cross-probe: the board's selection selects the symbol here.
  useEffect(() => {
    if (!v || !props.probeRef) return;
    if (selSym?.reference === props.probeRef) return;
    const s = v.symbols.find((x) => x.reference === props.probeRef);
    if (s) {
      setSel({ kind: "symbol", id: s.id });
      if (s.sheet !== sheet) setSheet(s.sheet);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.probeRef, v]);

  const select = (s: Sel) => {
    setSel(s);
    if (s?.kind === "symbol") {
      const sym = v?.symbols.find((x) => x.id === s.id);
      props.onProbe(sym && !sym.virtual_part ? sym.reference : null);
    } else props.onProbe(null);
  };

  const deleteSel = useCallback(() => {
    if (!sel) return;
    void commit("pcb.schematic.delete", { targets: [sel.id] }, "Deleted");
    setSel(null);
  }, [sel, commit]);

  const rotateSel = useCallback(
    (back: boolean) => {
      if (selSym) void commit("pcb.schematic.symbol.update", { target: selSym.id, rotation: (selSym.rotation + (back ? 270 : 90)) % 360 });
      else if (selLabel) void commit("pcb.schematic.item.update", { target: selLabel.id, rotation: (selLabel.rotation + (back ? 270 : 90)) % 360 });
    },
    [selSym, selLabel, commit],
  );

  // Keys, handed in by the workspace while the schematic has focus.
  useEffect(() => {
    props.keys.current = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      if (e.key === "Escape") {
        if (mode.m !== "select") {
          if (mode.m === "wire" && mode.pts.length) setMode({ m: "wire", pts: [] });
          else setMode({ m: "select" });
          return true;
        }
        if (sel || props.net) {
          setSel(null);
          props.onNet(null);
          props.onProbe(null);
          return true;
        }
        return false;
      }
      if (mode.m === "place" && k === "r") {
        setMode({ ...mode, rotation: (mode.rotation + 90) % 360 });
        return true;
      }
      if (mode.m === "place" && k === "x") {
        setMode({ ...mode, mirror: !mode.mirror });
        return true;
      }
      if (!props.canEdit) return false;
      if (k === "r" && (selSym || selLabel)) rotateSel(e.shiftKey);
      else if (k === "x" && selSym) void commit("pcb.schematic.symbol.update", { target: selSym.id, mirror: !selSym.mirror });
      else if ((e.key === "Delete" || e.key === "Backspace") && sel) deleteSel();
      else if (k === "w") setMode({ m: "wire", pts: [] });
      else if (k === "n") setMode({ m: "label", kind: e.shiftKey ? "global" : "local" });
      else if (k === "u") setMode({ m: "power" });
      else if (k === "q") setMode({ m: "nc" });
      else if (k === "j") setMode({ m: "junction" });
      else if (k === "t") setMode({ m: "text" });
      else if (k === "p") setTab("add");
      else return false;
      return true;
    };
    return () => {
      props.keys.current = null;
    };
  });

  if (schId === undefined) return <div className="sch-pane"><div className="pcb-empty"><div className="pcb-spinner" /></div></div>;
  if (schId === null) {
    return (
      <div className="sch-pane">
        <div className="pcb-empty">
          <div className="pcb-new sch-new">
            <span className="pcb-new-art" aria-hidden="true">
              <Icon name="list" />
            </span>
            <h3>No schematic for this board yet</h3>
            <p>Draw one from the board’s parts and nets (each part gets a symbol, nets become labels and power ports, laid out for reading), or start an empty sheet and draw it yourself. Either way the board then updates from the schematic.</p>
            <div className="sch-new-actions">
              <button
                type="button"
                className="primary"
                disabled={!props.canEdit || !!busy}
                onClick={async () => {
                  setBusy("Drawing…");
                  const out = (await commit("pcb.schematic.from_board", { board: props.boardId }, "Schematic drawn from the board")) as { output?: { id: string } } | null;
                  setBusy(null);
                  if (out?.output?.id) setSchId(out.output.id);
                }}
              >
                {busy ?? "Draw from the board"}
              </button>
              <button
                type="button"
                className="secondary"
                disabled={!props.canEdit}
                onClick={async () => {
                  const out = (await commit("pcb.schematic.create", { board: props.boardId }, "Empty schematic")) as { output?: { id: string } } | null;
                  if (out?.output?.id) setSchId(out.output.id);
                }}
              >
                Start empty
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }
  if (!v || !detail) return <div className="sch-pane"><div className="pcb-empty">{error ? <p className="pcb-error">{error}</p> : <div className="pcb-spinner" />}</div></div>;

  const sync = detail.sync;
  const pending = sync.error ? 0 : sync.added.length + sync.changed.length + sync.removed.length + (sync.changed.length ? 0 : sync.nets.length ? 1 : 0);
  const errors = v.violations.filter((x) => x.severity === "error").length;

  return (
    <div className={`sch-pane ${props.compact ? "compact" : ""}`}>
      {!props.compact && (
        <aside className="pcb-side sch-side">
          <nav className="pcb-tabs" role="tablist">
            {(
              [
                ["parts", "Parts", v.stats.parts],
                ["add", "Add", null],
                ["nets", "Nets", v.nets.length],
                ["erc", "ERC", errors || null],
                ["sync", "Sync", pending || null],
              ] as [Tab, string, number | null][]
            ).map(([k, label, n]) => (
              <button key={k} type="button" role="tab" aria-selected={tab === k} className={tab === k ? "active" : ""} onClick={() => setTab(k)}>
                {label}
                {n !== null && <span className={`pcb-count ${k === "erc" || k === "sync" ? "bad" : ""}`}>{n}</span>}
              </button>
            ))}
          </nav>
          <div className="pcb-side-body">
            {tab === "parts" && (selSym ? <SymbolProps s={selSym} lib={lib} commit={commit} canEdit={props.canEdit} onBack={() => select(null)} /> : <PartsList v={v} sel={sel} onSelect={(s) => {
              select({ kind: "symbol", id: s.id });
              if (s.sheet !== sheet) setSheet(s.sheet);
              setFocus({ at: [(s.bbox[0] + s.bbox[2]) / 2, (s.bbox[1] + s.bbox[3]) / 2], n: Date.now() });
            }} />)}
            {tab === "add" && <Library lib={lib} active={mode.m === "place" ? mode.symbol : null} onPick={(name) => setMode({ m: "place", symbol: name, rotation: 0, mirror: false })} canEdit={props.canEdit} />}
            {tab === "nets" && <NetList v={v} net={props.net} onNet={(n) => props.onNet(props.net === n ? null : n)} />}
            {tab === "erc" && (
              <ErcList
                v={v}
                onFocus={(x) => {
                  if (x.sheet !== sheet) setSheet(x.sheet);
                  setFocus({ at: x.at, n: Date.now() });
                  const o = x.objects[0];
                  const s = o && v.symbols.find((y) => y.id === o);
                  if (s) select({ kind: "symbol", id: s.id });
                  else if (o && v.labels.some((y) => y.id === o)) setSel({ kind: "label", id: o });
                  else if (o && v.wires.some((y) => y.id === o)) setSel({ kind: "wire", id: o });
                }}
              />
            )}
            {tab === "sync" && <SyncPanel sync={sync} schematic={v.id} commit={commit} canEdit={props.canEdit} notify={notify} />}
          </div>
        </aside>
      )}
      <SchematicCanvas
        v={v}
        sheet={sheet}
        onSheet={setSheet}
        sel={sel}
        onSel={select}
        probeRef={props.probeRef}
        net={props.net}
        onNet={props.onNet}
        mode={mode}
        onMode={setMode}
        focus={focus}
        commit={commit}
        canEdit={props.canEdit}
        lib={lib}
        powerNet={powerNet}
        onPowerNet={setPowerNet}
        errors={errors}
        pending={pending}
        inSync={!sync.error && sync.in_sync}
        onOpenTab={(t) => setTab(t)}
        compact={!!props.compact}
      />
    </div>
  );
}

// ---- Side panels -----------------------------------------------------------------------

function PartsList(props: { v: SchematicV; sel: Sel; onSelect: (s: SymbolV) => void }) {
  const [q, setQ] = useState("");
  const list = props.v.symbols.filter((s) => !s.virtual_part && (!q || `${s.reference} ${s.value} ${s.footprint} ${s.mpn ?? ""}`.toLowerCase().includes(q.toLowerCase())));
  return (
    <div className="pcb-tab">
      <label className="pcb-search">
        <Icon name="search" />
        <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Find a part" />
      </label>
      <div className="pcb-list sch-list">
        {list.map((s) => (
          <button key={s.id} type="button" className={`pcb-row ${props.sel?.id === s.id ? "active" : ""} ${s.dnp ? "dnp" : ""}`} onClick={() => props.onSelect(s)}>
            <span className="ref">
              {s.reference}
              {s.units > 1 ? String.fromCharCode(64 + s.unit) : ""}
            </span>
            <span className="val">
              {s.value || "—"}
              <small>
                {s.footprint || "no footprint"}
                {s.lcsc ? ` · ${s.lcsc}` : ""}
              </small>
            </span>
            {!s.component && <span className="tag">new</span>}
          </button>
        ))}
      </div>
      <p className="small sch-hint">Click a part to edit it. On the sheet: drag to move, R turn, X mirror, Del delete; W wire, N net label (Shift+N global), U power port, Q no-connect, J junction, T note, P add a part.</p>
    </div>
  );
}

function SymbolProps(props: { s: SymbolV; lib: LibSymbol[] | null; commit: Commit; canEdit: boolean; onBack: () => void }) {
  const { s } = props;
  const def = props.lib?.find((l) => l.name === s.symbol);
  const field = (label: string, key: string, value: string, opts?: string[]) => (
    <label className="sch-field">
      <span>{label}</span>
      <input
        key={`${s.id}-${key}-${value}`}
        defaultValue={value}
        list={opts ? `fp-${s.id}` : undefined}
        disabled={!props.canEdit}
        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
        onBlur={(e) => {
          const nv = e.target.value.trim();
          if (nv !== value) void props.commit("pcb.schematic.symbol.update", { target: s.id, [key]: nv });
        }}
      />
      {opts && (
        <datalist id={`fp-${s.id}`}>
          {opts.map((o) => (
            <option key={o} value={o} />
          ))}
        </datalist>
      )}
    </label>
  );
  const pins = s.pins.filter((p) => !p.hidden);
  return (
    <div className="pcb-tab sch-props">
      <button type="button" className="sch-back" onClick={props.onBack}>
        ← All parts
      </button>
      <div className="sch-props-head">
        <strong>
          {s.reference}
          {s.units > 1 ? String.fromCharCode(64 + s.unit) : ""}
        </strong>
        <span>{def?.description ?? s.symbol}</span>
      </div>
      {field("Reference", "reference", s.reference)}
      {field("Value", "value", s.value)}
      {field("Footprint", "footprint", s.footprint, def?.footprints?.length ? def.footprints : undefined)}
      {field("MPN", "mpn", s.mpn ?? "")}
      {field("Maker", "manufacturer", s.manufacturer ?? "")}
      {field("LCSC", "lcsc", s.lcsc ?? "")}
      <div className="sch-props-row">
        <label className="sch-check">
          <input type="checkbox" checked={s.dnp} disabled={!props.canEdit} onChange={(e) => void props.commit("pcb.schematic.symbol.update", { target: s.id, dnp: e.target.checked })} /> Do not populate
        </label>
        <button type="button" className="secondary" disabled={!props.canEdit} onClick={() => void props.commit("pcb.schematic.symbol.update", { target: s.id, rotation: (s.rotation + 90) % 360 })} title="Turn (R)">
          <Icon name="rotate" />
        </button>
        <button type="button" className="secondary" disabled={!props.canEdit} onClick={() => void props.commit("pcb.schematic.symbol.update", { target: s.id, mirror: !s.mirror })} title="Mirror (X)">
          <Icon name="mirror" />
        </button>
      </div>
      <h4 className="sch-sub">Pins → pads</h4>
      <div className="sch-pins">
        {pins.map((p) => (
          <div key={p.number} className="sch-pin">
            <span className="n">{p.number}</span>
            <span className="nm">{p.name === "~" ? "" : p.name}</span>
            <span className="pd">{p.pads.join(", ")}</span>
            <span className={`nt ${p.net ? "" : "none"}`}>{p.net ?? "—"}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function Library(props: { lib: LibSymbol[] | null; active: string | null; onPick: (name: string) => void; canEdit: boolean }) {
  const [q, setQ] = useState("");
  if (!props.lib) return <div className="pcb-empty"><div className="pcb-spinner" /></div>;
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const list = props.lib.filter((l) => words.every((w) => `${l.name} ${l.description} ${l.keywords.join(" ")} ${l.category}`.toLowerCase().includes(w)));
  const cats = [...new Set(list.map((l) => l.category))];
  return (
    <div className="pcb-tab">
      <label className="pcb-search">
        <Icon name="search" />
        <input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Resistor, LDO, USB-C, ESP32…" />
      </label>
      <div className="sch-lib">
        {cats.map((c) => (
          <div key={c}>
            <h4 className="sch-sub">{c}</h4>
            <div className="sch-lib-grid">
              {list
                .filter((l) => l.category === c)
                .map((l) => (
                  <button key={l.name} type="button" disabled={!props.canEdit} className={`sch-lib-item ${props.active === l.name ? "active" : ""}`} onClick={() => props.onPick(l.name)} title={l.description}>
                    <SymbolThumb name={l.name} />
                    <span>{l.name}</span>
                  </button>
                ))}
            </div>
          </div>
        ))}
      </div>
      <p className="small sch-hint">Pick one, then click the sheet to place it (R turns, X mirrors, Esc stops).</p>
    </div>
  );
}

const previews = new Map<string, Promise<{ draw: Draw[]; bbox: [number, number, number, number] }>>();
function usePreview(name: string) {
  const [d, setD] = useState<{ draw: Draw[]; bbox: [number, number, number, number] } | null>(null);
  useEffect(() => {
    let live = true;
    let p = previews.get(name);
    if (!p) {
      p = fetch(`/pcb/symbols/${encodeURIComponent(name)}`).then((r) => r.json());
      previews.set(name, p);
    }
    p.then((x) => live && setD(x)).catch(() => {});
    return () => {
      live = false;
    };
  }, [name]);
  return d;
}

function SymbolThumb({ name }: { name: string }) {
  const d = usePreview(name);
  if (!d) return <svg className="sch-thumb" />;
  const draws = d.draw.filter((x) => x.t !== "text" || (x.ink !== "reference" && x.ink !== "value"));
  const b = bboxOf(draws);
  const pad = 1.5;
  return (
    <svg className="sch-thumb" viewBox={`${b[0] - pad} ${b[1] - pad} ${b[2] - b[0] + 2 * pad} ${b[3] - b[1] + 2 * pad}`} preserveAspectRatio="xMidYMid meet">
      {draws.filter((x) => x.t !== "text" || x.ink === "body").map((x, i) => (
        <Prim key={i} d={x} />
      ))}
    </svg>
  );
}

function NetList(props: { v: SchematicV; net: string | null; onNet: (n: string) => void }) {
  const [q, setQ] = useState("");
  const list = props.v.nets.filter((n) => !q || n.name.toLowerCase().includes(q.toLowerCase()) || n.pins.some((p) => p.toLowerCase().startsWith(q.toLowerCase())));
  return (
    <div className="pcb-tab">
      <label className="pcb-search">
        <Icon name="search" />
        <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Find a net or pin" />
      </label>
      <div className="pcb-list sch-list">
        {list.map((n) => (
          <button key={n.name} type="button" className={`pcb-row ${props.net === n.name ? "active" : ""}`} onClick={() => props.onNet(n.name)}>
            <span className={`sch-net-dot ${n.named ? "named" : ""}`} />
            <span className="val">
              {n.name}
              <small>{n.pins.join("  ")}</small>
            </span>
            <span className="tag">{n.pads.length}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function ErcList(props: { v: SchematicV; onFocus: (x: Erc) => void }) {
  const [warn, setWarn] = useState(true);
  const list = props.v.violations.filter((x) => warn || x.severity === "error");
  const errors = props.v.violations.length - props.v.violations.filter((x) => x.severity === "warning").length;
  return (
    <div className="pcb-tab">
      <div className={`sch-erc-head ${errors ? "bad" : "good"}`}>
        <Icon name={errors ? "alert" : "check"} />
        <div>
          <strong>{errors ? `${errors} electrical ${errors === 1 ? "error" : "errors"}` : "Electrical rules pass"}</strong>
          <span>
            {props.v.stats.parts} parts · {props.v.stats.nets} nets · {props.v.stats.warnings} warning{props.v.stats.warnings === 1 ? "" : "s"}
          </span>
        </div>
      </div>
      <label className="sch-check">
        <input type="checkbox" checked={warn} onChange={(e) => setWarn(e.target.checked)} /> Show warnings
      </label>
      <div className="pcb-list sch-list">
        {list.map((x, i) => (
          <button key={i} type="button" className="pcb-row sch-erc" onClick={() => props.onFocus(x)}>
            <span className={`sch-sev ${x.severity}`} />
            <span className="val">
              {x.message}
              <small>{x.code.replace(/_/g, " ")}</small>
            </span>
          </button>
        ))}
        {!list.length && <p className="small">Nothing to fix.</p>}
      </div>
    </div>
  );
}

function SyncPanel(props: { sync: SyncReport; schematic: string; commit: Commit; canEdit: boolean; notify: (m: string) => void }) {
  const s = props.sync;
  const [busy, setBusy] = useState(false);
  if (s.error) return <div className="pcb-tab"><p className="pcb-error">{s.error}</p></div>;
  const section = (title: string, n: number, body: React.ReactNode, tone: string) =>
    n > 0 && (
      <div className={`sch-sync-sec ${tone}`}>
        <h4 className="sch-sub">
          {title} <span>{n}</span>
        </h4>
        {body}
      </div>
    );
  return (
    <div className="pcb-tab sch-sync">
      <div className={`sch-erc-head ${s.in_sync ? "good" : "pending"}`}>
        <Icon name={s.in_sync ? "check" : "refresh"} />
        <div>
          <strong>{s.in_sync ? "The board matches the schematic" : "Update the board from the schematic"}</strong>
          <span>
            {s.unchanged} part{s.unchanged === 1 ? "" : "s"} unchanged{s.board_only.length ? ` · ${s.board_only.length} board-only kept` : ""}
            {s.erc_errors ? ` · ${s.erc_errors} ERC error${s.erc_errors === 1 ? "" : "s"}` : ""}
          </span>
        </div>
      </div>
      {section(
        "Add to the board",
        s.added.length,
        s.added.map((a) => (
          <div key={a.reference} className="sch-sync-row add">
            <b>{a.reference}</b> {a.value} <small>{a.footprint} · {a.pads} pad nets</small>
          </div>
        )),
        "add",
      )}
      {section(
        "Change",
        s.changed.length,
        s.changed.map((c) => (
          <div key={c.reference} className="sch-sync-row chg">
            <b>{c.reference}</b> <small>{c.changes.join(" · ")}</small>
          </div>
        )),
        "chg",
      )}
      {section(
        "Pad nets",
        s.nets.length,
        s.nets.slice(0, 60).map((n) => (
          <div key={n.pad} className="sch-sync-row net">
            <b>{n.pad}</b> <small>{n.from ?? "—"} → {n.to ?? "—"}</small>
          </div>
        )),
        "net",
      )}
      {section(
        "Remove from the board",
        s.removed.length,
        s.removed.map((r) => (
          <div key={r.reference} className="sch-sync-row del">
            <b>{r.reference}</b> {r.value} <small>{r.footprint} · no symbol has it</small>
          </div>
        )),
        "del",
      )}
      {section(
        "Board-only (kept)",
        s.board_only.length,
        s.board_only.map((r) => (
          <div key={r.reference} className="sch-sync-row keep">
            <b>{r.reference}</b> <small>{r.footprint} · {r.why}</small>
          </div>
        )),
        "keep",
      )}
      {section(
        "Skipped",
        s.skipped.length,
        s.skipped.map((r) => (
          <div key={r.reference} className="sch-sync-row del">
            <b>{r.reference}</b> <small>{r.why}</small>
          </div>
        )),
        "del",
      )}
      {(s.new_nets.length > 0 || s.gone_nets.length > 0) && (
        <p className="small">
          {s.new_nets.length > 0 && <>New nets: {s.new_nets.join(", ")}. </>}
          {s.gone_nets.length > 0 && <>Nets gone: {s.gone_nets.join(", ")}.</>}
        </p>
      )}
      <div className="sch-sync-actions">
        <button
          type="button"
          className="primary"
          disabled={!props.canEdit || s.in_sync || busy}
          onClick={async () => {
            setBusy(true);
            const out = (await props.commit("pcb.schematic.update_board", { schematic: props.schematic })) as { output?: SyncReport } | null;
            setBusy(false);
            const o = out?.output;
            if (o) props.notify(`Board updated: ${o.added.length} added, ${o.changed.length} changed, ${o.removed.length} removed`);
          }}
        >
          <Icon name="refresh" /> Update board
        </button>
        <button
          type="button"
          className="secondary"
          disabled={!props.canEdit || busy}
          title="Write reference, footprint and value changes made on the board back to the schematic"
          onClick={async () => {
            const out = (await props.commit("pcb.schematic.back_annotate", { schematic: props.schematic })) as { output?: { renamed: unknown[]; changed: unknown[] } } | null;
            const o = out?.output;
            if (o) props.notify(o.renamed.length || o.changed.length ? `Back-annotated ${o.renamed.length} references, ${o.changed.length} fields` : "Nothing to bring back from the board");
          }}
        >
          Back-annotate
        </button>
      </div>
      <p className="small">New parts wait in a row below the board to be placed. Mounting holes and other parts without electrical pads stay on the board only.</p>
    </div>
  );
}

// ---- The sheet ------------------------------------------------------------------------

type ViewT = { s: number; tx: number; ty: number };

function SchematicCanvas(props: {
  v: SchematicV;
  sheet: string;
  onSheet: (s: string) => void;
  sel: Sel;
  onSel: (s: Sel) => void;
  probeRef: string | null;
  net: string | null;
  onNet: (n: string | null) => void;
  mode: Mode;
  onMode: (m: Mode) => void;
  focus: { at: P; n: number } | null;
  commit: Commit;
  canEdit: boolean;
  lib: LibSymbol[] | null;
  powerNet: string;
  onPowerNet: (n: string) => void;
  errors: number;
  pending: number;
  inSync: boolean;
  onOpenTab: (t: Tab) => void;
  compact: boolean;
}) {
  const { v, sel, net, mode } = props;
  const host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<P>([800, 600]);
  const [view, setView] = useState<ViewT | null>(null);
  const [cursor, setCursor] = useState<P | null>(null);
  const [hover, setHover] = useState<{ x: number; y: number; text: string; sub?: string } | null>(null);
  const [drag, setDrag] = useState<{ sel: NonNullable<Sel>; start: P; delta: P; moved: boolean } | null>(null);
  const [prompt, setPrompt] = useState<{ at: P; pin: string | null; kind: "local" | "global" | "text"; text: string } | null>(null);
  const pan = useRef<{ x: number; y: number; tx: number; ty: number } | null>(null);
  const fitted = useRef<string | null>(null);
  const lastClick = useRef<{ p: P; t: number } | null>(null);
  const sh = v.sheets.find((s) => s.id === props.sheet) ?? v.sheets[0];

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize([el.clientWidth, el.clientHeight]));
    ro.observe(el);
    setSize([el.clientWidth, el.clientHeight]);
    return () => ro.disconnect();
  }, []);

  const fit = useCallback(() => {
    const [w, h] = sh.size;
    const top = 52;
    const s = Math.min((size[0] - 40) / w, (size[1] - top - 24) / h);
    setView({ s, tx: (size[0] - w * s) / 2, ty: top + (size[1] - top - 24 - h * s) / 2 });
  }, [sh.size, size]);

  useEffect(() => {
    const key = `${v.id}:${sh.id}:${size[0]}x${size[1]}`;
    if (fitted.current !== key && size[0] > 50) {
      fitted.current = key;
      fit();
    }
  }, [v.id, sh.id, size, fit]);

  useEffect(() => {
    if (!props.focus || !view) return;
    const s = Math.max(view.s, Math.min(size[0], size[1]) / 90);
    const [x, y] = props.focus.at;
    setView({ s, tx: size[0] / 2 - x * s, ty: size[1] / 2 - y * s });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.focus]);

  const toSheet = (e: { clientX: number; clientY: number }): P => {
    const r = host.current!.getBoundingClientRect();
    const t = view!;
    return [(e.clientX - r.left - t.tx) / t.s, (e.clientY - r.top - t.ty) / t.s];
  };

  const onSheetSyms = useMemo(() => v.symbols.filter((s) => s.sheet === sh.id), [v.symbols, sh.id]);
  const onSheetWires = useMemo(() => v.wires.filter((s) => s.sheet === sh.id), [v.wires, sh.id]);
  const onSheetLabels = useMemo(() => v.labels.filter((s) => s.sheet === sh.id), [v.labels, sh.id]);
  const pinsHere = useMemo(() => onSheetSyms.flatMap((s) => s.pins.filter((p) => !p.hidden).map((p) => ({ s, p }))), [onSheetSyms]);
  const pinAt = (p: P, tol: number) => pinsHere.find((x) => Math.hypot(x.p.at[0] - p[0], x.p.at[1] - p[1]) < tol);
  const wireEndAt = (p: P) => onSheetWires.some((w) => same(w.points[0], p) || same(w.points[w.points.length - 1], p));

  const onWheel = (e: React.WheelEvent) => {
    if (!view) return;
    const r = host.current!.getBoundingClientRect();
    const mx = e.clientX - r.left;
    const my = e.clientY - r.top;
    const k = Math.exp(-e.deltaY * 0.0015);
    const s = Math.min(Math.max(view.s * k, 0.5), 80);
    const f = s / view.s;
    setView({ s, tx: mx - (mx - view.tx) * f, ty: my - (my - view.ty) * f });
  };

  const tol = view ? Math.max(0.6, 7 / view.s) : 1;

  const finishWire = async (points: P[]) => {
    const clean = points.filter((p, i) => i === 0 || !same(p, points[i - 1]));
    props.onMode({ m: "wire", pts: [] });
    if (clean.length < 2) return;
    await props.commit("pcb.schematic.wire", { schematic: v.id, sheet: sh.id, points: clean });
  };

  /** The orthogonal bend from the last wire point to the cursor: along the longer leg first. */
  const bend = (a: P, b: P): P[] => {
    if (Math.abs(a[0] - b[0]) < 1e-6 || Math.abs(a[1] - b[1]) < 1e-6) return [b];
    return Math.abs(b[0] - a[0]) >= Math.abs(b[1] - a[1]) ? [[b[0], a[1]], b] : [[a[0], b[1]], b];
  };

  const onDown = async (e: ReactPointerEvent) => {
    if (!view) return;
    if (prompt) {
      // A click elsewhere closes the open name box.
      if (!(e.target as Element).closest(".sch-prompt")) setPrompt(null);
      return;
    }
    if (e.button !== 0) {
      (e.target as Element).setPointerCapture?.(e.pointerId);
      pan.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty };
      return;
    }
    const raw = toSheet(e);
    const pin = pinAt(raw, tol);
    const p: P = pin ? pin.p.at : snapP(raw);
    const pinRef = pin ? `${pin.s.reference}.${pin.p.number}` : null;
    if (mode.m === "wire") {
      if (!mode.pts.length) {
        lastClick.current = { p, t: performance.now() };
        props.onMode({ m: "wire", pts: [p] });
        return;
      }
      const last = mode.pts[mode.pts.length - 1];
      const next = [...mode.pts, ...bend(last, p)];
      // Pointer events carry no click count: a second click on the same point within 450 ms is a double-click.
      const dbl = lastClick.current !== null && same(lastClick.current.p, p) && performance.now() - lastClick.current.t < 450;
      lastClick.current = { p, t: performance.now() };
      if (dbl || (pin && mode.pts.length >= 1) || (wireEndAt(p) && !same(p, mode.pts[0]))) return finishWire(dbl ? mode.pts : next);
      props.onMode({ m: "wire", pts: next });
      return;
    }
    if (mode.m === "place") {
      await props.commit("pcb.schematic.place", { schematic: v.id, sheet: sh.id, symbol: mode.symbol, at: p, rotation: mode.rotation, mirror: mode.mirror });
      return;
    }
    if (mode.m === "label") {
      setPrompt({ at: p, pin: pinRef, kind: mode.kind, text: "" });
      return;
    }
    if (mode.m === "text") {
      setPrompt({ at: p, pin: null, kind: "text", text: "" });
      return;
    }
    if (mode.m === "power") {
      await props.commit("pcb.schematic.label", pinRef ? { schematic: v.id, kind: "power", text: props.powerNet, pin: pinRef } : { schematic: v.id, sheet: sh.id, kind: "power", text: props.powerNet, at: p });
      return;
    }
    if (mode.m === "nc") {
      await props.commit("pcb.schematic.no_connect", pinRef ? { schematic: v.id, pin: pinRef } : { schematic: v.id, sheet: sh.id, at: p });
      return;
    }
    if (mode.m === "junction") {
      await props.commit("pcb.schematic.junction", { schematic: v.id, sheet: sh.id, at: p });
      return;
    }
    (e.target as Element).setPointerCapture?.(e.pointerId);
    pan.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty };
  };

  const startDrag = (e: ReactPointerEvent, s: NonNullable<Sel>, netName?: string | null) => {
    if (mode.m !== "select" || e.button !== 0) return;
    e.stopPropagation();
    props.onSel(s);
    if (netName !== undefined) props.onNet(netName ?? null);
    if (!props.canEdit) return;
    (e.target as Element).setPointerCapture?.(e.pointerId);
    setDrag({ sel: s, start: toSheet(e), delta: [0, 0], moved: false });
  };

  const onMove = (e: ReactPointerEvent) => {
    if (!view) return;
    const raw = toSheet(e);
    const pin = mode.m !== "select" ? pinAt(raw, tol) : null;
    setCursor(pin ? pin.p.at : snapP(raw));
    if (drag) {
      const d: P = [snap(raw[0] - drag.start[0]), snap(raw[1] - drag.start[1])];
      setDrag({ ...drag, delta: d, moved: drag.moved || Math.hypot(raw[0] - drag.start[0], raw[1] - drag.start[1]) * view.s > 3 });
      return;
    }
    if (pan.current) setView({ ...view, tx: pan.current.tx + e.clientX - pan.current.x, ty: pan.current.ty + e.clientY - pan.current.y });
  };

  const onUp = (e: ReactPointerEvent) => {
    if (drag) {
      const d = drag;
      setDrag(null);
      if (d.moved && (d.delta[0] || d.delta[1])) {
        const delta: P = [+d.delta[0].toFixed(4), +d.delta[1].toFixed(4)];
        if (d.sel.kind === "symbol") {
          const s = v.symbols.find((x) => x.id === d.sel.id);
          if (s) void props.commit("pcb.schematic.symbol.update", { target: s.id, at: [+(s.at[0] + delta[0]).toFixed(4), +(s.at[1] + delta[1]).toFixed(4)] });
        } else void props.commit("pcb.schematic.item.update", { target: d.sel.id, delta });
      }
    }
    if (pan.current) {
      const moved = Math.hypot(e.clientX - pan.current.x, e.clientY - pan.current.y) > 3;
      pan.current = null;
      if (!moved && mode.m === "select") {
        props.onSel(null);
        props.onNet(null);
      }
    }
  };

  const hoverPin = (e: ReactPointerEvent, s: SymbolV, p: PinV) => {
    const r = host.current!.getBoundingClientRect();
    setHover({ x: e.clientX - r.left, y: e.clientY - r.top, text: `${s.reference}.${p.number}${p.name && p.name !== "~" ? ` · ${p.name}` : ""}`, sub: `${p.net ?? "not connected"} · ${p.etype.replace(/_/g, " ")} · pad ${p.pads.join(",")}` });
  };

  const off = (id: string): P | null => (drag && drag.sel.id === id && drag.moved ? drag.delta : null);
  const tf = (id: string) => {
    const o = off(id);
    return o ? `translate(${o[0]} ${o[1]})` : undefined;
  };
  const lit = (n: string | null | undefined) => !!net && n === net;
  const SELC = "#6d4ee8";
  const PROBE = "#e7861b";
  const ercHere = v.violations.filter((x) => x.sheet === sh.id);
  const ghost = usePreview(mode.m === "place" ? mode.symbol : "R");

  const placeGhost = mode.m === "place" && cursor && ghost && (
    <g transform={`translate(${cursor[0]} ${cursor[1]}) rotate(${-mode.rotation}) scale(${mode.mirror ? -1 : 1} 1)`} opacity={0.55} pointerEvents="none">
      {ghost.draw.filter((d) => d.t !== "text" || d.ink === "body").map((d, i) => (
        <Prim key={i} d={d} hl={SELC} />
      ))}
    </g>
  );

  const wirePreview =
    mode.m === "wire" && mode.pts.length > 0 && cursor ? (
      <polyline points={pts([...mode.pts, ...bend(mode.pts[mode.pts.length - 1], cursor)])} fill="none" stroke={INK.wire} strokeWidth={0.2} strokeDasharray="0.8 0.5" pointerEvents="none" />
    ) : null;

  const grid = view && view.s > 4;
  const gridStep = view && view.s > 9 ? GRID : 2.54;

  return (
    <div className={`sch-canvas mode-${mode.m}`} ref={host} onWheel={onWheel} onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerLeave={() => setHover(null)} onContextMenu={(e) => e.preventDefault()}>
      {view && (
        <svg width={size[0]} height={size[1]} className="sch-svg">
          <defs>
            <pattern id="sch-grid" width={gridStep} height={gridStep} patternUnits="userSpaceOnUse">
              <circle cx={0} cy={0} r={Math.max(0.06, 0.9 / view.s)} fill="#b9b4c4" />
            </pattern>
            <filter id="sch-shadow" x="-5%" y="-5%" width="110%" height="110%">
              <feDropShadow dx="0" dy={1.2 / view.s} stdDeviation={4 / view.s} floodColor="#1d1830" floodOpacity="0.16" />
            </filter>
          </defs>
          <g transform={`translate(${view.tx} ${view.ty}) scale(${view.s})`}>
            <rect x={0} y={0} width={sh.size[0]} height={sh.size[1]} fill="#fffefb" filter="url(#sch-shadow)" />
            {grid && <rect x={0} y={0} width={sh.size[0]} height={sh.size[1]} fill="url(#sch-grid)" opacity={0.7} />}
            <g className="sch-frame">
              {sh.frame.map((d, i) => (
                <Prim key={i} d={d} />
              ))}
              {sh.boxes_draw.map((d, i) => (
                <Prim key={`b${i}`} d={d} />
              ))}
            </g>
            {v.texts
              .filter((t) => t.sheet === sh.id)
              .map((t) => (
                <g key={t.id} transform={tf(t.id)} onPointerDown={(e) => startDrag(e, { kind: "text", id: t.id })} className="sch-hit">
                  {t.text.split("\n").map((line, i) => (
                    <text key={i} x={t.at[0]} y={t.at[1] + i * t.size * 1.35} dy={t.size * 0.35} fontSize={t.size} fontFamily={FONT} fill={sel?.id === t.id ? SELC : INK.note}>
                      {line}
                    </text>
                  ))}
                </g>
              ))}
            {/* Symbols */}
            {onSheetSyms.map((s) => {
              const isSel = sel?.id === s.id;
              const probed = !isSel && !!props.probeRef && s.reference === props.probeRef;
              const dim = !!net && !s.pins.some((p) => p.net === net);
              return (
                <g key={s.id} transform={tf(s.id)} className="sch-sym" onPointerDown={(e) => startDrag(e, { kind: "symbol", id: s.id })}>
                  {(isSel || probed) && <rect x={s.bbox[0] - 0.8} y={s.bbox[1] - 0.8} width={s.bbox[2] - s.bbox[0] + 1.6} height={s.bbox[3] - s.bbox[1] + 1.6} rx={0.8} fill={isSel ? "#6d4ee814" : "#e7861b14"} stroke={isSel ? SELC : PROBE} strokeWidth={0.18} strokeDasharray={probed ? "0.8 0.5" : undefined} />}
                  <rect x={s.body[0]} y={s.body[1]} width={s.body[2] - s.body[0]} height={s.body[3] - s.body[1]} fill="transparent" />
                  {s.draw.map((d, i) => (
                    <Prim key={i} d={d} dim={dim} />
                  ))}
                  {s.pins
                    .filter((p) => !p.hidden)
                    .map((p) => (
                      <circle
                        key={p.number}
                        cx={p.at[0]}
                        cy={p.at[1]}
                        r={0.45}
                        className={`sch-pin-hit ${lit(p.net) ? "lit" : ""}`}
                        onPointerEnter={(e) => hoverPin(e, s, p)}
                        onPointerLeave={() => setHover(null)}
                        onPointerDown={(e) => {
                          if (mode.m !== "select") return;
                          e.stopPropagation();
                          props.onSel({ kind: "symbol", id: s.id });
                          props.onNet(p.net);
                        }}
                      />
                    ))}
                </g>
              );
            })}
            {/* Wires on top of pins so nets read clearly */}
            {onSheetWires.map((w) => {
              const isSel = sel?.id === w.id;
              const color = isSel ? SELC : lit(w.net) ? "#f0a020" : INK.wire;
              return (
                <g key={w.id} transform={tf(w.id)} onPointerDown={(e) => startDrag(e, { kind: "wire", id: w.id }, w.net)} className="sch-hit">
                  <polyline points={pts(w.points)} fill="none" stroke="transparent" strokeWidth={Math.max(1.2, 10 / view.s)} />
                  <polyline points={pts(w.points)} fill="none" stroke={color} strokeWidth={lit(w.net) || isSel ? 0.3 : 0.1778} strokeLinecap="round" strokeLinejoin="round" opacity={net && !lit(w.net) ? 0.35 : 1} />
                </g>
              );
            })}
            {v.junctions
              .filter((j) => j.sheet === sh.id)
              .map((j, i) => (
                <circle key={j.id ?? `a${i}`} cx={j.at[0]} cy={j.at[1]} r={0.45} fill={j.id && sel?.id === j.id ? SELC : INK.junction} onPointerDown={j.id ? (e) => startDrag(e, { kind: "junction", id: j.id! }) : undefined} />
              ))}
            {v.no_connects
              .filter((n) => n.sheet === sh.id)
              .map((n) => (
                <g key={n.id} transform={tf(n.id)} onPointerDown={(e) => startDrag(e, { kind: "nc", id: n.id })} className="sch-hit">
                  <rect x={n.at[0] - 0.9} y={n.at[1] - 0.9} width={1.8} height={1.8} fill="transparent" />
                  <path d={`M${n.at[0] - 0.7} ${n.at[1] - 0.7}L${n.at[0] + 0.7} ${n.at[1] + 0.7}M${n.at[0] - 0.7} ${n.at[1] + 0.7}L${n.at[0] + 0.7} ${n.at[1] - 0.7}`} stroke={sel?.id === n.id ? SELC : INK.no_connect} strokeWidth={0.22} strokeLinecap="round" />
                </g>
              ))}
            {onSheetLabels.map((l) => {
              const isSel = sel?.id === l.id;
              return (
                <g key={l.id} transform={tf(l.id)} onPointerDown={(e) => startDrag(e, { kind: "label", id: l.id }, l.net)} className="sch-hit">
                  <rect x={l.bbox[0] - 0.3} y={l.bbox[1] - 0.3} width={l.bbox[2] - l.bbox[0] + 0.6} height={l.bbox[3] - l.bbox[1] + 0.6} rx={0.4} fill={isSel ? "#6d4ee814" : lit(l.net) ? "#f0a02022" : "transparent"} stroke={isSel ? SELC : "none"} strokeWidth={0.15} />
                  {l.draw.map((d, i) => (
                    <Prim key={i} d={d} hl={isSel ? SELC : undefined} dim={!!net && !lit(l.net)} />
                  ))}
                </g>
              );
            })}
            {/* Electrical-rules markers */}
            {ercHere.map((x, i) => (
              <g key={`e${i}`} className={`sch-erc-mark ${x.severity}`} pointerEvents="none">
                <circle cx={x.at[0]} cy={x.at[1]} r={1.1} />
                <text x={x.at[0]} y={x.at[1]} dy={0.42} fontSize={1.3} textAnchor="middle" fontFamily={FONT} fontWeight={700}>
                  !
                </text>
              </g>
            ))}
            {wirePreview}
            {placeGhost}
            {mode.m !== "select" && cursor && (
              <g pointerEvents="none" className="sch-cursor">
                <path d={`M${cursor[0] - 1.6} ${cursor[1]}H${cursor[0] + 1.6}M${cursor[0]} ${cursor[1] - 1.6}V${cursor[1] + 1.6}`} stroke={SELC} strokeWidth={0.1} />
                {pinAt(cursor, 0.01) && <circle cx={cursor[0]} cy={cursor[1]} r={0.8} fill="none" stroke={SELC} strokeWidth={0.15} />}
              </g>
            )}
          </g>
        </svg>
      )}
      <div className="sch-toolbar">
        {(
          [
            ["select", "cursor", "Select and move (Esc)"],
            ["wire", "pencil", "Wire (W)"],
            ["label", "tag", "Net label (N, Shift+N global)"],
            ["power", "spark", "Power port (U)"],
            ["nc", "close", "No-connect flag (Q)"],
            ["junction", "dots", "Junction (J)"],
            ["text", "comment", "Note (T)"],
          ] as [Mode["m"], Parameters<typeof Icon>[0]["name"], string][]
        ).map(([m, icon, title]) => (
          <button
            key={m}
            type="button"
            title={title}
            disabled={m !== "select" && !props.canEdit}
            className={`sch-tool ${mode.m === m ? "active" : ""}`}
            onClick={() => props.onMode(m === "wire" ? { m: "wire", pts: [] } : m === "label" ? { m: "label", kind: "local" } : ({ m } as Mode))}
          >
            <Icon name={icon} />
          </button>
        ))}
        {mode.m === "power" && (
          <select className="sch-power-pick" value={props.powerNet} onChange={(e) => props.onPowerNet(e.target.value)} title="Supply">
            {[...new Set([...POWER_NETS, ...v.labels.filter((l) => l.kind === "power").map((l) => l.text)])].map((n) => (
              <option key={n}>{n}</option>
            ))}
          </select>
        )}
        <span className="sch-tool-sep" />
        <button type="button" className="sch-tool" title="Add a part (P)" disabled={!props.canEdit} onClick={() => props.onOpenTab("add")}>
          <Icon name="plus" />
        </button>
        <button type="button" className="sch-tool" title="Fit the sheet" onClick={fit}>
          <Icon name="fit" />
        </button>
        <a className="sch-tool" title="PDF of every sheet" href={projectUrl(`/pcb/schematic/${v.id}/schematic.pdf`)} target="_blank" rel="noreferrer">
          <Icon name="printer" />
        </a>
      </div>
      <div className="sch-status">
        {v.sheets.length > 1 && (
          <div className="sch-sheets">
            {v.sheets.map((s) => (
              <button key={s.id} type="button" className={s.id === sh.id ? "active" : ""} onClick={() => props.onSheet(s.id)}>
                {s.name}
              </button>
            ))}
          </div>
        )}
        <button type="button" className={`sch-pill ${props.errors ? "bad" : "good"}`} onClick={() => props.onOpenTab("erc")}>
          <Icon name={props.errors ? "alert" : "check"} />
          {props.errors ? `${props.errors} ERC` : "ERC passes"}
        </button>
        <button type="button" className={`sch-pill ${props.inSync ? "good" : "pending"}`} onClick={() => props.onOpenTab("sync")}>
          <Icon name={props.inSync ? "check" : "refresh"} />
          {props.inSync ? "Board in step" : `Update board (${props.pending})`}
        </button>
      </div>
      {mode.m !== "select" && (
        <div className="sch-hint-bar">
          {mode.m === "wire" && (mode.pts.length ? "Click to bend · click a pin or wire end (or double-click) to finish · Esc" : "Click a pin or grid point to start the wire")}
          {mode.m === "place" && `Placing ${mode.symbol} · R turn · X mirror · Esc done`}
          {mode.m === "label" && `Click a pin or wire for a ${mode.kind} net label`}
          {mode.m === "power" && `Click a pin or wire end for a ${props.powerNet} port`}
          {mode.m === "nc" && "Click an unused pin"}
          {mode.m === "junction" && "Click where wires cross to join them"}
          {mode.m === "text" && "Click to write a note"}
        </div>
      )}
      {prompt && view && (
        <form
          className="sch-prompt"
          style={{ left: prompt.at[0] * view.s + view.tx + 8, top: prompt.at[1] * view.s + view.ty - 18 }}
          onSubmit={async (e) => {
            e.preventDefault();
            const text = prompt.text.trim();
            setPrompt(null);
            if (!text) return;
            if (prompt.kind === "text") await props.commit("pcb.schematic.text", { schematic: v.id, sheet: sh.id, at: prompt.at, text });
            else await props.commit("pcb.schematic.label", prompt.pin ? { schematic: v.id, kind: prompt.kind, text, pin: prompt.pin } : { schematic: v.id, sheet: sh.id, kind: prompt.kind, text, at: prompt.at });
          }}
        >
          <input
            autoFocus
            value={prompt.text}
            placeholder={prompt.kind === "text" ? "Note" : "Net name"}
            list={prompt.kind === "text" ? undefined : "sch-net-names"}
            onChange={(e) => setPrompt({ ...prompt, text: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                setPrompt(null);
              }
            }}
          />
          <datalist id="sch-net-names">
            {v.nets.filter((n) => n.named).map((n) => (
              <option key={n.name} value={n.name} />
            ))}
          </datalist>
        </form>
      )}
      {hover && (
        <div className="pcb-tip" style={{ left: hover.x + 14, top: hover.y + 12 }}>
          <b>{hover.text}</b>
          {hover.sub && <span>{hover.sub}</span>}
        </div>
      )}
    </div>
  );
}
