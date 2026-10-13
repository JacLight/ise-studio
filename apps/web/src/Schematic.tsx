// The schematic view of the Electronics workspace: the circuit drawn as
// symbols, wires, buses, labels and power ports on paper, its nets and
// electrical-rules check, and "Update board" with the change report. The
// engine resolves everything into drawing primitives (GET /pcb/schematic/<id>)
// so screen, PDF and SVG match; this draws them and sends edits back as
// pcb.schematic.* commands. Selecting a part or net here selects it on the board too.

import { type PointerEvent as ReactPointerEvent, type MutableRefObject, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";
import "./Schematic.css";
import { ErcPanel, ExportMenu, FieldsPanel, FindBar, ItemProps, SetupPanel, SheetsPanel } from "./SchematicPanels";
import { type Box, type Commit, type Draw, type Erc, type P, type PinV, type SchematicV, type Sel, type SelKind, type SymbolV, isBusName, itemsOnSheet } from "./schematicTypes";

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
  classes?: string[];
};
type Detail = { schematic: SchematicV; errors: number; sync: SyncReport };
type LibSymbol = { name: string; description: string; prefix: string; category: string; value: string; footprint: string; footprints: string[]; units: number; keywords: string[]; pins: number; virtual: boolean };
type ShapeKind = "polyline" | "polygon" | "rect" | "circle" | "arc";
type Mode =
  | { m: "select" }
  | { m: "wire"; pts: P[] }
  | { m: "bus"; pts: P[] }
  | { m: "entry"; size: P }
  | { m: "label"; kind: "local" | "global" | "hierarchical" | "directive" }
  | { m: "power" }
  | { m: "nc" }
  | { m: "junction" }
  | { m: "text" }
  | { m: "shape"; shape: ShapeKind; pts: P[] }
  | { m: "textbox"; pts: P[] }
  | { m: "table" }
  | { m: "image" }
  | { m: "place"; symbol: string; rotation: number; mirror: boolean };
type Tab = "parts" | "add" | "nets" | "erc" | "sync" | "sheets" | "fields" | "setup";

/** Ink roles on paper; the PDF and SVG use the same colours (schematic_pdf.rs). */
const INK: Record<string, string> = {
  body: "#8a1f1f",
  pin: "#8a1f1f",
  body_fill: "#fdf6dc",
  pin_number: "#9b4a3a",
  pin_name: "#0d6b6b",
  reference: "#0d6b6b",
  value: "#0d6b6b",
  field: "#4f7f7f",
  wire: "#1b7f3d",
  junction: "#1b7f3d",
  bus: "#1f4fbf",
  label: "#1d2430",
  global: "#7a3aa8",
  hierarchical: "#a05a00",
  power: "#8a1f1f",
  directive: "#2f7d8a",
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
const r4 = (v: number) => +v.toFixed(4);
const DASH: Record<string, (w: number) => string | undefined> = {
  dash: (w) => `${w * 6} ${w * 3}`,
  dot: (w) => `${Math.max(w, 0.15)} ${Math.max(w, 0.15) * 2.5}`,
  dash_dot: (w) => `${w * 6} ${w * 2.5} ${w} ${w * 2.5}`,
};

function pts(list: P[]) {
  return list.map((p) => `${p[0]},${p[1]}`).join(" ");
}

/** One engine primitive as SVG. `hl` colours it for a highlight. */
function Prim({ d, hl, dim }: { d: Draw; hl?: string; dim?: boolean }) {
  const op = dim ? 0.28 : 1;
  switch (d.t) {
    case "line": {
      const color = hl ?? INK[d.ink] ?? "#222";
      return <polyline points={pts(d.p)} fill="none" stroke={color} strokeWidth={d.w} strokeLinecap="round" strokeLinejoin="round" opacity={op} />;
    }
    case "poly": {
      const color = hl ?? INK[d.ink] ?? "#222";
      return <polygon points={pts(d.p)} fill={d.fill === "ink" ? color : d.fill === "body" ? INK.body_fill : "none"} stroke={color} strokeWidth={d.w} strokeLinejoin="round" opacity={op} />;
    }
    case "circle": {
      const color = hl ?? INK[d.ink] ?? "#222";
      return <circle cx={d.c[0]} cy={d.c[1]} r={d.r} fill={d.fill === "ink" ? color : d.fill === "body" ? INK.body_fill : "none"} stroke={color} strokeWidth={d.w} opacity={op} />;
    }
    case "styled": {
      const color = hl ?? d.color;
      const Tag = d.closed ? "polygon" : "polyline";
      return <Tag points={pts(d.p)} fill={d.fill && d.closed ? d.fill : "none"} stroke={d.w > 0 ? color : "none"} strokeWidth={d.w} strokeDasharray={d.dash ? DASH[d.dash]?.(d.w) : undefined} strokeLinecap="round" strokeLinejoin="round" opacity={op} />;
    }
    case "image":
      return <image x={d.at[0]} y={d.at[1]} width={d.size[0]} height={d.size[1]} preserveAspectRatio="none" href={`data:${d.mime};base64,${d.data}`} opacity={op} />;
    case "text":
    case "rich": {
      if (!d.s) return null;
      const color = hl ?? (d.t === "rich" ? d.color : (INK[d.ink] ?? "#222"));
      const weight = d.t === "rich" ? (d.bold ? 700 : 400) : d.ink === "reference" || d.ink === "frame_text" ? 600 : 400;
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
          fontWeight={weight}
          fontStyle={d.t === "rich" && d.italic ? "italic" : undefined}
        >
          {d.s}
        </text>
      );
    }
  }
}

function bboxOf(list: Draw[]): Box {
  let b: Box = [Infinity, Infinity, -Infinity, -Infinity];
  const add = (p: P) => (b = [Math.min(b[0], p[0]), Math.min(b[1], p[1]), Math.max(b[2], p[0]), Math.max(b[3], p[1])]);
  for (const d of list) {
    if (d.t === "line" || d.t === "poly" || d.t === "styled") d.p.forEach(add);
    else if (d.t === "circle") {
      add([d.c[0] - d.r, d.c[1] - d.r]);
      add([d.c[0] + d.r, d.c[1] + d.r]);
    } else if (d.t === "image") {
      add(d.at);
      add([d.at[0] + d.size[0], d.at[1] + d.size[1]]);
    } else add(d.at);
  }
  return b;
}

/** Small tool glyphs the shared icon set lacks. */
function Glyph({ name }: { name: string }) {
  const p: Record<string, string> = {
    bus: "M4 6v12 M4 12h16 M20 6v12",
    entry: "M5 4v16 M5 12l6-6 M11 6h8",
    line: "M4 18 L10 8 L15 14 L20 5",
    rect: "M4 6h16v12H4z",
    circle: "M12 4a8 8 0 1 0 0.01 0z",
    arc: "M4 18a10 10 0 0 1 16 0",
    polygon: "M12 3l8 6-3 11H7L4 9z",
    textbox: "M3 5h18v14H3z M7 9h10 M7 13h7",
    table: "M3 5h18v14H3z M3 10h18 M3 15h18 M10 5v14",
    image: "M3 5h18v14H3z M3 16l5-5 4 4 3-3 6 6 M15 9h.01",
    hier: "M3 12h5l3-3h10v6H11l-3-3",
    directive: "M6 20V6 M6 6h11l-2 3 2 3H6",
    find: "M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13z M15.5 15.5 20 20",
  };
  return (
    <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={p[name] ?? ""} />
    </svg>
  );
}

const kindOf = (v: SchematicV, id: string): SelKind | null => {
  if (v.symbols.some((x) => x.id === id)) return "symbol";
  if (v.wires.some((x) => x.id === id)) return "wire";
  if (v.labels.some((x) => x.id === id)) return "label";
  if (v.buses.some((x) => x.id === id)) return "bus";
  if (v.bus_entries.some((x) => x.id === id)) return "entry";
  if (v.graphics.some((x) => x.id === id)) return "graphic";
  if (v.texts.some((x) => x.id === id)) return "text";
  if (v.no_connects.some((x) => x.id === id)) return "nc";
  if (v.junctions.some((x) => x.id === id)) return "junction";
  return null;
};

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
  const { notify } = props;
  const [schId, setSchId] = useState<string | null | undefined>(undefined);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sheet, setSheetRaw] = useState<string>("1");
  const [tab, setTab] = useState<Tab>("parts");
  const [sel, setSel] = useState<Sel>(null);
  const [selIds, setSelIds] = useState<string[]>([]);
  const [mode, setMode] = useState<Mode>({ m: "select" });
  const [focus, setFocus] = useState<{ at: P; n: number } | null>(null);
  const [lib, setLib] = useState<LibSymbol[] | null>(null);
  const [powerNet, setPowerNet] = useState("GND");
  const [busy, setBusy] = useState<string | null>(null);
  const [finding, setFinding] = useState(false);
  const [exporting, setExporting] = useState(false);
  const clip = useRef<{ ids: string[]; anchor: P; sheet: string } | null>(null);
  const last = useRef<{ ids: string[]; increment: number } | null>(null);
  const cursorRef = useRef<P | null>(null);
  const history = useRef<{ back: string[]; fwd: string[] }>({ back: [], fwd: [] });

  /** Every edit goes through here: what it creates becomes "the last item" Insert repeats. */
  const commit: Commit = useCallback(
    async (op, p, done) => {
      const out = (await props.commit(op, p, done)) as { output?: { id?: string; ids?: string[] } } | null;
      const creates = /^pcb\.schematic\.(wire|bus|bus_entry|label|junction|no_connect|text|place|shape|text_box|table|image|duplicate)$/.test(op);
      if (creates && out?.output) {
        const ids = out.output.ids ?? (out.output.id ? [out.output.id] : []);
        if (ids.length) last.current = { ids, increment: op === "pcb.schematic.label" || op === "pcb.schematic.text" || op === "pcb.schematic.duplicate" ? 1 : 0 };
      }
      return out;
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [props.commit],
  );

  const setSheet = useCallback(
    (s: string) => {
      setSheetRaw((cur) => {
        if (cur !== s) {
          history.current.back.push(cur);
          history.current.fwd = [];
        }
        return s;
      });
      setSelIds([]);
      setSel(null);
    },
    [],
  );

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
        if (!d.schematic.sheets.some((s) => s.id === sheet)) setSheetRaw(d.schematic.sheets[0]?.id ?? "1");
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
  const selSym = sel?.kind === "symbol" ? (v?.symbols.find((s) => s.id === sel.id && s.sheet === sheet) ?? v?.symbols.find((s) => s.id === sel.id) ?? null) : null;
  const selLabel = sel?.kind === "label" ? (v?.labels.find((s) => s.id === sel.id) ?? null) : null;
  // Items gone after an edit (deleted, undone) leave the selection.
  useEffect(() => {
    if (!v) return;
    setSelIds((ids) => {
      const keep = ids.filter((id) => kindOf(v, id));
      return keep.length === ids.length ? ids : keep;
    });
    if (sel && !kindOf(v, sel.id)) setSel(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [v]);

  // Cross-probe: the board's selection selects the symbol here.
  useEffect(() => {
    if (!v || !props.probeRef) return;
    if (selSym?.reference === props.probeRef) return;
    const s = v.symbols.find((x) => x.reference === props.probeRef);
    if (s) {
      setSel({ kind: "symbol", id: s.id });
      setSelIds([s.id]);
      if (s.sheet !== sheet) setSheet(s.sheet);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.probeRef, v]);

  /** Select one item (or none); `add` toggles it in the selection. */
  const select = (s: Sel, add = false) => {
    if (add && s) {
      setSelIds((ids) => (ids.includes(s.id) ? ids.filter((x) => x !== s.id) : [...ids, s.id]));
      setSel(s);
      return;
    }
    setSel(s);
    setSelIds(s ? [s.id] : []);
    if (s?.kind === "symbol") {
      const sym = v?.symbols.find((x) => x.id === s.id);
      props.onProbe(sym && !sym.virtual_part ? sym.reference : null);
    } else props.onProbe(null);
  };
  const selectMany = (ids: string[]) => {
    setSelIds(ids);
    const first = ids[0];
    setSel(first && v ? { kind: kindOf(v, first) ?? "symbol", id: first } : null);
    if (ids.length !== 1) props.onProbe(null);
  };

  const targets = selIds.length ? selIds : sel ? [sel.id] : [];
  const deleteSel = () => {
    if (!targets.length) return;
    void commit("pcb.schematic.delete", { targets }, targets.length > 1 ? `Deleted ${targets.length} items` : "Deleted");
    setSel(null);
    setSelIds([]);
  };

  const transformSel = (p: { rotate?: number; mirror?: "x" | "y" }) => {
    if (!targets.length) return;
    if (targets.length === 1 && selSym && p.rotate !== undefined) {
      void commit("pcb.schematic.symbol.update", { target: selSym.id, rotation: (selSym.rotation + p.rotate) % 360 });
    } else if (targets.length === 1 && selLabel && p.rotate !== undefined) {
      void commit("pcb.schematic.item.update", { target: selLabel.id, rotation: (selLabel.rotation + p.rotate) % 360 });
    } else {
      // A symbol alone mirrors about its own anchor.
      const about = targets.length === 1 && selSym ? selSym.at : undefined;
      void commit("pcb.schematic.move", { targets, ...p, ...(about ? { about } : {}) });
    }
  };

  const paste = () => {
    const c = clip.current;
    const at = cursorRef.current;
    if (!c || !v) return;
    const delta: P = at ? [r4(snap(at[0] - c.anchor[0])), r4(snap(at[1] - c.anchor[1]))] : [5.08, 5.08];
    void commit("pcb.schematic.duplicate", { targets: c.ids, delta, sheet }, `Pasted ${c.ids.length} item${c.ids.length === 1 ? "" : "s"}`).then((out) => {
      const ids = (out as { output?: { ids?: string[] } } | null)?.output?.ids;
      if (ids) selectMany(ids);
    });
  };

  const copy = () => {
    if (!targets.length || !v) return;
    const boxes = itemsOnSheet(v, sheet).filter((x) => targets.includes(x.id));
    const anchor: P = boxes.length ? snapP([Math.min(...boxes.map((b) => b.box[0])), Math.min(...boxes.map((b) => b.box[1]))]) : [0, 0];
    clip.current = { ids: targets, anchor, sheet };
    notify(`Copied ${targets.length} item${targets.length === 1 ? "" : "s"}`);
  };

  const goBack = () => {
    const prev = history.current.back.pop();
    if (prev && v?.sheets.some((s) => s.id === prev)) {
      history.current.fwd.push(sheet);
      setSheetRaw(prev);
    }
  };
  const goForward = () => {
    const next = history.current.fwd.pop();
    if (next && v?.sheets.some((s) => s.id === next)) {
      history.current.back.push(sheet);
      setSheetRaw(next);
    }
  };

  // Keys, handed in by the workspace while the schematic has focus.
  useEffect(() => {
    props.keys.current = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      const mod = e.metaKey || e.ctrlKey;
      if (e.key === "Escape") {
        if (finding) {
          setFinding(false);
          return true;
        }
        if (mode.m !== "select") {
          if ((mode.m === "wire" || mode.m === "bus") && mode.pts.length) setMode({ ...mode, pts: [] });
          else if ((mode.m === "shape" || mode.m === "textbox") && mode.pts.length) setMode({ ...mode, pts: [] });
          else setMode({ m: "select" });
          return true;
        }
        if (sel || selIds.length || props.net) {
          setSel(null);
          setSelIds([]);
          props.onNet(null);
          props.onProbe(null);
          return true;
        }
        return false;
      }
      if (mod && k === "f") {
        setFinding(true);
        return true;
      }
      if (mod && k === "a" && v) {
        selectMany(itemsOnSheet(v, sheet).map((x) => x.id));
        return true;
      }
      if (mod && k === "c" && targets.length && !window.getSelection()?.toString()) {
        copy();
        return true;
      }
      if (e.altKey && e.key === "ArrowLeft") {
        goBack();
        return true;
      }
      if (e.altKey && e.key === "ArrowRight") {
        goForward();
        return true;
      }
      if (mode.m === "place" && k === "r") {
        setMode({ ...mode, rotation: (mode.rotation + 90) % 360 });
        return true;
      }
      if (mode.m === "place" && k === "x") {
        setMode({ ...mode, mirror: !mode.mirror });
        return true;
      }
      if (mode.m === "entry" && k === "r") {
        // Turn the entry a quarter: (dx, dy) → (dy, -dx).
        setMode({ m: "entry", size: [mode.size[1], -mode.size[0]] });
        return true;
      }
      if ((mode.m === "shape" || mode.m === "wire" || mode.m === "bus") && e.key === "Enter") {
        window.dispatchEvent(new CustomEvent("sch-finish"));
        return true;
      }
      if (e.key === "Backspace" && !targets.length && mode.m === "select") {
        const cur = v?.sheets.find((s) => s.id === sheet);
        if (cur?.parent) {
          setSheet(cur.parent);
          return true;
        }
        return false;
      }
      if (!props.canEdit) return false;
      if (mod && k === "v") paste();
      else if (mod && k === "d" && targets.length) void commit("pcb.schematic.duplicate", { targets, delta: [5.08, 5.08] }, "Duplicated").then((out) => {
        const ids = (out as { output?: { ids?: string[] } } | null)?.output?.ids;
        if (ids) selectMany(ids);
      });
      else if (mod) return false;
      else if (e.key === "Insert" || (k === "i" && e.shiftKey)) {
        const l = last.current;
        if (l) void commit("pcb.schematic.duplicate", { targets: l.ids, delta: [0, 2.54], increment: l.increment }, "Repeated");
      } else if (k === "r" && targets.length) transformSel({ rotate: e.shiftKey ? 270 : 90 });
      else if (k === "x" && targets.length) transformSel({ mirror: "x" });
      else if (k === "y" && targets.length) transformSel({ mirror: "y" });
      else if ((e.key === "Delete" || e.key === "Backspace") && targets.length) deleteSel();
      else if (k === "w") setMode({ m: "wire", pts: [] });
      else if (k === "b") setMode({ m: "bus", pts: [] });
      else if (k === "z") setMode({ m: "entry", size: [2.54, 2.54] });
      else if (k === "n") setMode({ m: "label", kind: e.shiftKey ? "global" : "local" });
      else if (k === "h") setMode({ m: "label", kind: "hierarchical" });
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
                title="Draw a schematic from this board: a symbol per part, nets as labels and power ports"
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
                title="Start a blank schematic sheet for this board and draw it yourself"
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
  const pending = sync.error ? 0 : sync.added.length + sync.changed.length + sync.removed.length + (sync.classes?.length ? 1 : 0) + (sync.changed.length ? 0 : sync.nets.length ? 1 : 0);
  const errors = v.stats.errors;
  const focusOn = (x: { sheet: string; at: P }) => {
    if (x.sheet && x.sheet !== sheet) setSheet(x.sheet);
    setFocus({ at: x.at, n: Date.now() });
  };

  return (
    <div className={`sch-pane ${props.compact ? "compact" : ""}`}>
      {!props.compact && (
        <aside className="pcb-side sch-side">
          <nav className="pcb-tabs schx-tabs" role="tablist">
            {(
              [
                ["parts", "Parts", v.stats.parts],
                ["add", "Add", null],
                ["nets", "Nets", v.nets.length],
                ["erc", "ERC", errors || null],
                ["sync", "Sync", pending || null],
                ["sheets", "Sheets", v.sheets.length > 1 ? v.sheets.length : null],
                ["fields", "Fields", null],
                ["setup", "Setup", null],
              ] as [Tab, string, number | null][]
            ).map(([k, label, n]) => (
              <button key={k} type="button" role="tab" aria-selected={tab === k} className={tab === k ? "active" : ""} title={{ parts: "Parts on the schematic: find, select and edit them", add: "Add a part from the symbol library", nets: "Nets: highlight, colour and class them", erc: "Electrical rules check: problems to fix", sync: "Changes waiting to go to the board", sheets: "Sheets and the hierarchy", fields: "Every part's fields in one table", setup: "Title block, net classes, rules, variables and annotation" }[k]} onClick={() => setTab(k)}>
                {label}
                {n !== null && <span className={`pcb-count ${k === "erc" || k === "sync" ? "bad" : ""}`}>{n}</span>}
              </button>
            ))}
          </nav>
          <div className="pcb-side-body">
            {tab === "parts" &&
              (selSym && selIds.length <= 1 ? (
                <SymbolProps s={selSym} lib={lib} commit={commit} canEdit={props.canEdit} onBack={() => select(null)} />
              ) : sel && sel.kind !== "symbol" && selIds.length <= 1 ? (
                <ItemProps v={v} sel={sel} commit={commit} canEdit={props.canEdit} onBack={() => select(null)} sheet={sheet} />
              ) : (
                <PartsList
                  v={v}
                  sel={sel}
                  count={selIds.length}
                  onSelect={(s) => {
                    select({ kind: "symbol", id: s.id });
                    if (s.sheet !== sheet) setSheet(s.sheet);
                    setFocus({ at: [(s.bbox[0] + s.bbox[2]) / 2, (s.bbox[1] + s.bbox[3]) / 2], n: Date.now() });
                  }}
                />
              ))}
            {tab === "add" && <Library lib={lib} active={mode.m === "place" ? mode.symbol : null} onPick={(name) => setMode({ m: "place", symbol: name, rotation: 0, mirror: false })} canEdit={props.canEdit} />}
            {tab === "nets" && <NetList v={v} net={props.net} onNet={(n) => props.onNet(props.net === n ? null : n)} commit={commit} canEdit={props.canEdit} />}
            {tab === "erc" && (
              <ErcPanel
                v={v}
                commit={commit}
                canEdit={props.canEdit}
                onFocus={(x: Erc) => {
                  focusOn(x);
                  const o = x.objects[0];
                  const s = o && v.symbols.find((y) => y.id === o);
                  if (s) select({ kind: "symbol", id: s.id });
                  else if (o) {
                    const k = kindOf(v, o);
                    if (k) select({ kind: k, id: o });
                  }
                }}
              />
            )}
            {tab === "sync" && <SyncPanel sync={sync} schematic={v.id} commit={commit} canEdit={props.canEdit} notify={notify} />}
            {tab === "sheets" && <SheetsPanel v={v} sheet={sheet} onSheet={setSheet} commit={commit} canEdit={props.canEdit} />}
            {tab === "fields" && (
              <FieldsPanel
                v={v}
                commit={commit}
                canEdit={props.canEdit}
                onSelect={(s) => {
                  select({ kind: "symbol", id: s.id });
                  focusOn({ sheet: s.sheet, at: [(s.bbox[0] + s.bbox[2]) / 2, (s.bbox[1] + s.bbox[3]) / 2] });
                }}
              />
            )}
            {tab === "setup" && <SetupPanel v={v} commit={commit} canEdit={props.canEdit} notify={notify} />}
          </div>
        </aside>
      )}
      <SchematicCanvas
        v={v}
        sheet={sheet}
        onSheet={setSheet}
        sel={sel}
        selIds={selIds}
        onSel={select}
        onSelectMany={selectMany}
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
        cursorRef={cursorRef}
        onTransform={transformSel}
        onAlign={(how) => void commit("pcb.schematic.align", { targets, how })}
        onDuplicate={() => void commit("pcb.schematic.duplicate", { targets, delta: [5.08, 5.08] }, "Duplicated").then((out) => {
          const ids = (out as { output?: { ids?: string[] } } | null)?.output?.ids;
          if (ids) selectMany(ids);
        })}
        onDelete={deleteSel}
        onBack={goBack}
        canBack={history.current.back.length > 0}
        finding={finding}
        onFinding={setFinding}
        exporting={exporting}
        onExporting={setExporting}
        notify={notify}
      />
    </div>
  );
}

// ---- Side panels -----------------------------------------------------------------------

function PartsList(props: { v: SchematicV; sel: Sel; count: number; onSelect: (s: SymbolV) => void }) {
  const [q, setQ] = useState("");
  const list = props.v.symbols.filter((s) => !s.virtual_part && (!q || `${s.reference} ${s.value} ${s.footprint} ${s.mpn ?? ""}`.toLowerCase().includes(q.toLowerCase())));
  return (
    <div className="pcb-tab">
      <label className="pcb-search">
        <Icon name="search" />
        <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Find a part" />
      </label>
      {props.count > 1 && <p className="small schx-selcount">{props.count} items selected: drag to move them together, R turn, X/Y mirror, Ctrl+D duplicate, Del delete, or align them from the bar on the sheet.</p>}
      <div className="pcb-list sch-list">
        {list.map((s) => (
          <button key={`${s.id}:${s.sheet}`} type="button" className={`pcb-row ${props.sel?.id === s.id ? "active" : ""} ${s.dnp ? "dnp" : ""}`} title={`Select ${s.reference} and show it on the sheet`} onClick={() => props.onSelect(s)}>
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
            {!s.component && !s.instance && <span className="tag">new</span>}
          </button>
        ))}
      </div>
      <p className="small sch-hint">Click a part to edit it. On the sheet: drag on empty paper to box-select (right-drag pans), Shift-click to add; R turn, X/Y mirror, Del delete, Ctrl+C/V copy and paste, Ctrl+D duplicate, Insert repeat; W wire, B bus, Z bus entry, N label (Shift+N global, H hierarchical), U power port, Q no-connect, J junction, T note, P add a part, Ctrl+F find.</p>
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
          if (nv !== value) void props.commit("pcb.schematic.symbol.update", { target: s.id, [key]: nv, ...(key === "reference" && s.instance ? { instance: s.sheet } : {}) });
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
      <button type="button" className="sch-back" title="Back to the list of all parts" onClick={props.onBack}>
        ← All parts
      </button>
      <div className="sch-props-head">
        <strong>
          {s.reference}
          {s.units > 1 ? String.fromCharCode(64 + s.unit) : ""}
        </strong>
        <span>{def?.description ?? s.symbol}</span>
      </div>
      {s.instance && <p className="small schx-note">Drawn from a shared sheet: the reference is this instance's own; every other change applies to all instances.</p>}
      {field("Reference", "reference", s.reference)}
      {field("Value", "value", s.value)}
      {field("Footprint", "footprint", s.footprint, def?.footprints?.length ? def.footprints : undefined)}
      {field("MPN", "mpn", s.mpn ?? "")}
      {field("Maker", "manufacturer", s.manufacturer ?? "")}
      {field("LCSC", "lcsc", s.lcsc ?? "")}
      <div className="sch-props-row">
        <label className="sch-check">
          <input type="checkbox" checked={s.dnp} disabled={!props.canEdit} title="Leave this part off the assembled board and out of the BOM" onChange={(e) => void props.commit("pcb.schematic.symbol.update", { target: s.id, dnp: e.target.checked })} /> Do not populate
        </label>
        <button type="button" className="secondary" disabled={!props.canEdit} onClick={() => void props.commit("pcb.schematic.symbol.update", { target: s.id, rotation: (s.rotation + 90) % 360 })} title="Turn (R)">
          <Icon name="rotate" />
        </button>
        <button type="button" className="secondary" disabled={!props.canEdit} onClick={() => void props.commit("pcb.schematic.symbol.update", { target: s.id, mirror: !s.mirror })} title="Mirror left-right (X)">
          <Icon name="mirror" />
        </button>
        <button type="button" className="secondary" disabled={!props.canEdit} onClick={() => void props.commit("pcb.schematic.move", { targets: [s.id], mirror: "y", about: s.at })} title="Mirror up-down (Y)">
          <span className="schx-flip">
            <Icon name="mirror" />
          </span>
        </button>
      </div>
      {Object.entries(s.fields ?? {}).map(([k, val]) => (
        <label key={k} className="sch-field">
          <span>{k}</span>
          <input key={`${s.id}-f-${k}-${val}`} defaultValue={val} disabled={!props.canEdit} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} onBlur={(e) => e.target.value !== val && void props.commit("pcb.schematic.symbol.update", { target: s.id, fields: { ...s.fields, [k]: e.target.value } })} />
        </label>
      ))}
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
      <KicadParts q={q} active={props.active} onPick={props.onPick} canEdit={props.canEdit} />
      <p className="small sch-hint">Pick one, then click the sheet to place it (R turns, X mirrors, Esc stops).</p>
    </div>
  );
}

type KicadHit = { id: string; name: string; library: string; description: string; footprint: string; count: number };
/** KiCad library parts fetched for placing: their definition and default footprint. */
const kicadParts = new Map<string, { definition: unknown; footprint: { name: string } | null }>();

/** The official KiCad symbol library, searched by the engine (downloaded on first use). */
function KicadParts(props: { q: string; active: string | null; onPick: (name: string) => void; canEdit: boolean }) {
  const [hits, setHits] = useState<KicadHit[] | null>(null);
  const [state, setState] = useState<string | null>(null);
  useEffect(() => {
    const q = props.q.trim();
    if (q.length < 2) {
      setHits(null);
      return;
    }
    let live = true;
    const t = setTimeout(() => {
      setState("Searching the KiCad library…");
      fetch(projectUrl(`/pcb/kicad/search?kind=symbols&limit=24&q=${encodeURIComponent(q)}`))
        .then(async (r) => (r.ok ? r.json() : Promise.reject(new Error(await r.text()))))
        .then((r: { results: KicadHit[] }) => {
          if (!live) return;
          setHits(r.results);
          setState(null);
        })
        .catch((e) => live && setState(String(e.message ?? e)));
    }, 300);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [props.q]);
  const pick = async (h: KicadHit) => {
    if (!kicadParts.has(h.id)) {
      setState(`Reading ${h.id}…`);
      const r = await fetch(projectUrl(`/pcb/kicad/symbol/${encodeURIComponent(h.id)}`));
      if (!r.ok) return setState(await r.text());
      const d = await r.json();
      kicadParts.set(h.id, { definition: d.definition, footprint: d.footprint });
      previews.set(h.id, Promise.resolve(d.preview));
      setState(d.unmapped_pins?.length ? `Pins ${d.unmapped_pins.join(", ")} have no pad on ${d.footprint?.name}` : d.footprint ? null : `${h.id} names no footprint: choose one on the board`);
    }
    props.onPick(h.id);
  };
  if (!hits && !state) return null;
  return (
    <div className="sch-kicad">
      <h4 className="sch-sub">KiCad library</h4>
      {state && <p className="small">{state}</p>}
      {hits?.length === 0 && <p className="small">Nothing matches.</p>}
      {hits?.map((h) => (
        <button key={h.id} type="button" disabled={!props.canEdit} className={`pcb-row ${props.active === h.id ? "active" : ""}`} onClick={() => void pick(h)} title={h.description}>
          <span className="val">
            {h.name}
            <small>
              {h.library} · {h.count} pins{h.footprint ? ` · ${h.footprint.split(":")[1] ?? h.footprint}` : ""}
            </small>
          </span>
        </button>
      ))}
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

function NetList(props: { v: SchematicV; net: string | null; onNet: (n: string) => void; commit: Commit; canEdit: boolean }) {
  const [q, setQ] = useState("");
  const list = props.v.nets.filter((n) => !q || n.name.toLowerCase().includes(q.toLowerCase()) || n.pins.some((p) => p.toLowerCase().startsWith(q.toLowerCase())) || (n.class ?? "").toLowerCase() === q.toLowerCase());
  const sel = props.v.nets.find((n) => n.name === props.net);
  return (
    <div className="pcb-tab">
      <label className="pcb-search">
        <Icon name="search" />
        <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Find a net, pin or class" />
      </label>
      {sel && (
        <div className="schx-card schx-netcard">
          <div className="schx-class-head">
            <span className="schx-dot" style={{ background: sel.color ?? INK.wire }} />
            <strong>{sel.name}</strong>
            {sel.class && <span className="tag">{sel.class}</span>}
          </div>
          <div className="schx-swatches">
            {["", "#d03030", "#e7861b", "#c9a400", "#1b7f3d", "#2a5bd7", "#7a3aa8"].map((c) => (
              <button key={c || "none"} type="button" className={`schx-swatch ${c ? "" : "none"} ${(props.v.setup.net_colors[sel.name] ?? "") === c ? "on" : ""}`} style={c ? { background: c } : undefined} disabled={!props.canEdit} title={c ? `Colour ${sel.name}` : "No colour of its own"} onClick={() => void props.commit("pcb.schematic.net_color", { schematic: props.v.id, net: sel.name, color: c })} />
            ))}
          </div>
        </div>
      )}
      <div className="pcb-list sch-list">
        {list.map((n) => (
          <button key={n.name} type="button" className={`pcb-row ${props.net === n.name ? "active" : ""}`} title={`Highlight net ${n.name} on the sheet`} onClick={() => props.onNet(n.name)}>
            <span className={`sch-net-dot ${n.named ? "named" : ""}`} style={n.color ? { background: n.color } : undefined} />
            <span className="val">
              {n.name}
              <small>
                {n.class ? `${n.class} · ` : ""}
                {n.pins.join("  ")}
              </small>
            </span>
            <span className="tag">{n.pads.length}</span>
          </button>
        ))}
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
      {section(
        "Net classes",
        s.classes?.length ?? 0,
        (s.classes ?? []).map((c) => (
          <div key={c} className="sch-sync-row chg">
            <small>{c}</small>
          </div>
        )),
        "chg",
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
          title="Send these schematic changes to the board: add, change and remove parts and nets"
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
  selIds: string[];
  onSel: (s: Sel, add?: boolean) => void;
  onSelectMany: (ids: string[]) => void;
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
  cursorRef: MutableRefObject<P | null>;
  onTransform: (p: { rotate?: number; mirror?: "x" | "y" }) => void;
  onAlign: (how: string) => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onBack: () => void;
  canBack: boolean;
  finding: boolean;
  onFinding: (b: boolean) => void;
  exporting: boolean;
  onExporting: (b: boolean) => void;
  notify: (m: string) => void;
}) {
  const { v, sel, net, mode, selIds } = props;
  const host = useRef<HTMLDivElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const [size, setSize] = useState<P>([800, 600]);
  const [view, setView] = useState<ViewT | null>(null);
  const [cursor, setCursor] = useState<P | null>(null);
  const [hover, setHover] = useState<{ x: number; y: number; text: string; sub?: string } | null>(null);
  const [drag, setDrag] = useState<{ ids: string[]; start: P; delta: P; moved: boolean } | null>(null);
  const [band, setBand] = useState<{ a: P; b: P; add: boolean } | null>(null);
  const [prompt, setPrompt] = useState<{ at: P; pin: string | null; kind: "local" | "global" | "hierarchical" | "directive" | "text" | "textbox" | "table"; text: string; end?: P } | null>(null);
  const [drawPick, setDrawPick] = useState(false);
  const pan = useRef<{ x: number; y: number; tx: number; ty: number } | null>(null);
  const fitted = useRef<string | null>(null);
  const lastClick = useRef<{ p: P; t: number } | null>(null);
  const imageAt = useRef<P | null>(null);
  const sh = v.sheets.find((s) => s.id === props.sheet) ?? v.sheets[0];
  const isSel = (id: string) => selIds.includes(id) || sel?.id === id;

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
  const onSheetBuses = useMemo(() => v.buses.filter((s) => s.sheet === sh.id), [v.buses, sh.id]);
  const pinsHere = useMemo(() => onSheetSyms.flatMap((s) => s.pins.filter((p) => !p.hidden).map((p) => ({ s, p }))), [onSheetSyms]);
  const pinAt = (p: P, tol: number) => pinsHere.find((x) => Math.hypot(x.p.at[0] - p[0], x.p.at[1] - p[1]) < tol);
  const wireEndAt = (p: P) => onSheetWires.some((w) => same(w.points[0], p) || same(w.points[w.points.length - 1], p));
  const busEndAt = (p: P) => onSheetBuses.some((w) => same(w.points[0], p) || same(w.points[w.points.length - 1], p));

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

  const finishLine = async (points: P[], kind: "wire" | "bus") => {
    const clean = points.filter((p, i) => i === 0 || !same(p, points[i - 1]));
    props.onMode({ m: kind, pts: [] });
    if (clean.length < 2) return;
    await props.commit(kind === "wire" ? "pcb.schematic.wire" : "pcb.schematic.bus", { schematic: v.id, sheet: sh.id, points: clean });
  };

  const finishShape = async (shape: ShapeKind, points: P[]) => {
    props.onMode({ m: "shape", shape, pts: [] });
    const clean = points.filter((p, i) => i === 0 || !same(p, points[i - 1]));
    const need = { polyline: 2, polygon: 3, rect: 2, circle: 2, arc: 3 }[shape];
    if (clean.length < need) return;
    await props.commit("pcb.schematic.shape", { schematic: v.id, sheet: sh.id, shape, points: shape === "polyline" || shape === "polygon" ? clean : clean.slice(0, need) });
  };

  // Enter finishes a wire, bus or line being drawn.
  useEffect(() => {
    const done = () => {
      if ((mode.m === "wire" || mode.m === "bus") && mode.pts.length >= 2) void finishLine(mode.pts, mode.m);
      if (mode.m === "shape" && (mode.shape === "polyline" || mode.shape === "polygon")) void finishShape(mode.shape, mode.pts);
    };
    window.addEventListener("sch-finish", done);
    return () => window.removeEventListener("sch-finish", done);
  });

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
    if (drawPick) setDrawPick(false);
    if (e.button !== 0) {
      (e.target as Element).setPointerCapture?.(e.pointerId);
      pan.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty };
      return;
    }
    const raw = toSheet(e);
    const pin = pinAt(raw, tol);
    const p: P = pin ? pin.p.at : snapP(raw);
    const pinRef = pin ? `${pin.s.reference}.${pin.p.number}` : null;
    const dblAt = (q: P) => {
      const dbl = lastClick.current !== null && same(lastClick.current.p, q) && performance.now() - lastClick.current.t < 450;
      lastClick.current = { p: q, t: performance.now() };
      return dbl;
    };
    if (mode.m === "wire" || mode.m === "bus") {
      if (!mode.pts.length) {
        dblAt(p);
        props.onMode({ m: mode.m, pts: [p] });
        return;
      }
      const lastP = mode.pts[mode.pts.length - 1];
      const next = [...mode.pts, ...bend(lastP, p)];
      // Pointer events carry no click count: a second click on the same point within 450 ms is a double-click.
      const dbl = dblAt(p);
      const endsOn = mode.m === "wire" ? (pin && mode.pts.length >= 1) || (wireEndAt(p) && !same(p, mode.pts[0])) : busEndAt(p) && !same(p, mode.pts[0]);
      if (dbl || endsOn) return finishLine(dbl ? mode.pts : next, mode.m);
      props.onMode({ m: mode.m, pts: next });
      return;
    }
    if (mode.m === "shape") {
      const q = snapP(raw);
      const need = { polyline: Infinity, polygon: Infinity, rect: 2, circle: 2, arc: 3 }[mode.shape];
      const dbl = dblAt(q);
      if (dbl && mode.pts.length >= 2) return finishShape(mode.shape, mode.pts);
      const next = [...mode.pts, q];
      if (next.length >= need) return finishShape(mode.shape, next);
      props.onMode({ ...mode, pts: next });
      return;
    }
    if (mode.m === "textbox") {
      const q = snapP(raw);
      if (!mode.pts.length) return props.onMode({ m: "textbox", pts: [q] });
      const a = mode.pts[0];
      props.onMode({ m: "textbox", pts: [] });
      const tl: P = [Math.min(a[0], q[0]), Math.min(a[1], q[1])];
      const br: P = [Math.max(a[0], q[0]), Math.max(a[1], q[1])];
      if (br[0] - tl[0] < 2 || br[1] - tl[1] < 2) return;
      setPrompt({ at: tl, end: br, pin: null, kind: "textbox", text: "" });
      return;
    }
    if (mode.m === "table") {
      setPrompt({ at: snapP(raw), pin: null, kind: "table", text: "3 x 2" });
      return;
    }
    if (mode.m === "image") {
      imageAt.current = snapP(raw);
      fileInput.current?.click();
      return;
    }
    if (mode.m === "entry") {
      await props.commit("pcb.schematic.bus_entry", { schematic: v.id, sheet: sh.id, at: p, size: mode.size });
      return;
    }
    if (mode.m === "place") {
      // A KiCad library part carries its definition, and its footprint for the board.
      const k = kicadParts.get(mode.symbol);
      await props.commit("pcb.schematic.place", { schematic: v.id, sheet: sh.id, symbol: mode.symbol, at: p, rotation: mode.rotation, mirror: mode.mirror, ...(k ? { definition: k.definition, ...(k.footprint && v.board ? { footprint_definition: k.footprint, footprint: k.footprint.name } : {}) } : {}) });
      return;
    }
    if (mode.m === "label") {
      setPrompt({ at: p, pin: pinRef, kind: mode.kind, text: mode.kind === "directive" ? (v.setup.netclasses[0]?.name ?? "") : "" });
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
    // Select mode on empty paper: a selection band.
    (e.target as Element).setPointerCapture?.(e.pointerId);
    setBand({ a: raw, b: raw, add: e.shiftKey });
  };

  const startDrag = (e: ReactPointerEvent, s: NonNullable<Sel>, netName?: string | null) => {
    if (mode.m !== "select" || e.button !== 0) return;
    e.stopPropagation();
    if (e.shiftKey) {
      props.onSel(s, true);
      return;
    }
    const inSel = selIds.includes(s.id) && selIds.length > 1;
    if (!inSel) props.onSel(s);
    if (netName !== undefined && !inSel) props.onNet(netName ?? null);
    if (!props.canEdit) return;
    (e.target as Element).setPointerCapture?.(e.pointerId);
    setDrag({ ids: inSel ? selIds : [s.id], start: toSheet(e), delta: [0, 0], moved: false });
  };

  const onMove = (e: ReactPointerEvent) => {
    if (!view) return;
    const raw = toSheet(e);
    const pin = mode.m !== "select" ? pinAt(raw, tol) : null;
    const c = pin ? pin.p.at : snapP(raw);
    setCursor(c);
    props.cursorRef.current = c;
    if (drag) {
      const d: P = [snap(raw[0] - drag.start[0]), snap(raw[1] - drag.start[1])];
      setDrag({ ...drag, delta: d, moved: drag.moved || Math.hypot(raw[0] - drag.start[0], raw[1] - drag.start[1]) * view.s > 3 });
      return;
    }
    if (band) {
      setBand({ ...band, b: raw });
      return;
    }
    if (pan.current) setView({ ...view, tx: pan.current.tx + e.clientX - pan.current.x, ty: pan.current.ty + e.clientY - pan.current.y });
  };

  const onUp = (e: ReactPointerEvent) => {
    if (drag) {
      const d = drag;
      setDrag(null);
      if (d.moved && (d.delta[0] || d.delta[1])) {
        const delta: P = [r4(d.delta[0]), r4(d.delta[1])];
        const one = d.ids.length === 1 ? v.symbols.find((x) => x.id === d.ids[0]) : null;
        if (one) void props.commit("pcb.schematic.symbol.update", { target: one.id, at: [r4(one.at[0] + delta[0]), r4(one.at[1] + delta[1])] });
        else void props.commit("pcb.schematic.move", { targets: d.ids, delta });
      }
    }
    if (band && view) {
      const b = band;
      setBand(null);
      const moved = Math.hypot(b.b[0] - b.a[0], b.b[1] - b.a[1]) * view.s > 4;
      if (!moved) {
        if (!b.add) {
          props.onSel(null);
          props.onNet(null);
        }
      } else {
        // Left to right: wholly inside (window); right to left: touching (crossing).
        const crossing = b.b[0] < b.a[0];
        const r: Box = [Math.min(b.a[0], b.b[0]), Math.min(b.a[1], b.b[1]), Math.max(b.a[0], b.b[0]), Math.max(b.a[1], b.b[1])];
        const hit = itemsOnSheet(v, sh.id)
          .filter((x) => (crossing ? x.box[0] <= r[2] && x.box[2] >= r[0] && x.box[1] <= r[3] && x.box[3] >= r[1] : x.box[0] >= r[0] && x.box[2] <= r[2] && x.box[1] >= r[1] && x.box[3] <= r[3]))
          .map((x) => x.id);
        props.onSelectMany(b.add ? [...new Set([...selIds, ...hit])] : hit);
      }
    }
    if (pan.current) pan.current = null;
    void e;
  };

  const hoverPin = (e: ReactPointerEvent, s: SymbolV, p: PinV) => {
    const r = host.current!.getBoundingClientRect();
    setHover({ x: e.clientX - r.left, y: e.clientY - r.top, text: `${s.reference}.${p.number}${p.name && p.name !== "~" ? ` · ${p.name}` : ""}`, sub: `${p.net ?? "not connected"} · ${p.etype.replace(/_/g, " ")} · pad ${p.pads.join(",")}` });
  };

  const off = (id: string): P | null => (drag && drag.moved && drag.ids.includes(id) ? drag.delta : null);
  const tf = (id: string) => {
    const o = off(id);
    return o ? `translate(${o[0]} ${o[1]})` : undefined;
  };
  const lit = (n: string | null | undefined) => !!net && n === net;
  const SELC = "#6d4ee8";
  const PROBE = "#e7861b";
  const ercHere = v.violations.filter((x) => x.sheet === sh.id && !x.excluded);
  const ghost = usePreview(mode.m === "place" ? mode.symbol : "R");

  const placeGhost = mode.m === "place" && cursor && ghost && (
    <g transform={`translate(${cursor[0]} ${cursor[1]}) rotate(${-mode.rotation}) scale(${mode.mirror ? -1 : 1} 1)`} opacity={0.55} pointerEvents="none">
      {ghost.draw.filter((d) => d.t !== "text" || d.ink === "body").map((d, i) => (
        <Prim key={i} d={d} hl={SELC} />
      ))}
    </g>
  );

  const linePreview =
    (mode.m === "wire" || mode.m === "bus") && mode.pts.length > 0 && cursor ? (
      <polyline points={pts([...mode.pts, ...bend(mode.pts[mode.pts.length - 1], cursor)])} fill="none" stroke={mode.m === "bus" ? INK.bus : INK.wire} strokeWidth={mode.m === "bus" ? 0.5 : 0.2} strokeDasharray="0.8 0.5" pointerEvents="none" />
    ) : null;
  const shapePreview = (() => {
    if (!cursor) return null;
    if (mode.m === "textbox" && mode.pts.length) {
      const a = mode.pts[0];
      return <rect x={Math.min(a[0], cursor[0])} y={Math.min(a[1], cursor[1])} width={Math.abs(cursor[0] - a[0])} height={Math.abs(cursor[1] - a[1])} fill="none" stroke={SELC} strokeWidth={0.2} strokeDasharray="0.8 0.5" pointerEvents="none" />;
    }
    if (mode.m !== "shape" || !mode.pts.length) return null;
    const ps = [...mode.pts, cursor];
    const a = ps[0];
    const common = { fill: "none", stroke: SELC, strokeWidth: 0.2, strokeDasharray: "0.8 0.5", pointerEvents: "none" as const };
    if (mode.shape === "rect") return <rect x={Math.min(a[0], cursor[0])} y={Math.min(a[1], cursor[1])} width={Math.abs(cursor[0] - a[0])} height={Math.abs(cursor[1] - a[1])} {...common} />;
    if (mode.shape === "circle") return <circle cx={a[0]} cy={a[1]} r={Math.hypot(cursor[0] - a[0], cursor[1] - a[1])} {...common} />;
    return <polyline points={pts(mode.shape === "polygon" ? [...ps, a] : ps)} {...common} />;
  })();
  const entryGhost =
    mode.m === "entry" && cursor ? <line x1={cursor[0]} y1={cursor[1]} x2={cursor[0] + mode.size[0]} y2={cursor[1] + mode.size[1]} stroke={INK.bus} strokeWidth={0.25} opacity={0.6} pointerEvents="none" /> : null;

  const grid = view && view.s > 4;
  const gridStep = view && view.s > 9 ? GRID : 2.54;
  const multi = selIds.length > 1;

  const drawTools: [string, string, Mode][] = [
    ["line", "Lines", { m: "shape", shape: "polyline", pts: [] }],
    ["polygon", "Polygon", { m: "shape", shape: "polygon", pts: [] }],
    ["rect", "Rectangle", { m: "shape", shape: "rect", pts: [] }],
    ["circle", "Circle (centre, then edge)", { m: "shape", shape: "circle", pts: [] }],
    ["arc", "Arc (start, through, end)", { m: "shape", shape: "arc", pts: [] }],
    ["textbox", "Text box", { m: "textbox", pts: [] }],
    ["table", "Table", { m: "table" }],
    ["image", "Picture (PNG or JPEG)", { m: "image" }],
  ];
  const drawing = ["shape", "textbox", "table", "image"].includes(mode.m);

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
            {/* Sheet boxes: double-click to enter. */}
            {sh.boxes.map((b) => (
              <rect
                key={`box${b.sheet}`}
                x={b.at[0]}
                y={b.at[1]}
                width={b.size[0]}
                height={b.size[1]}
                className="schx-box-hit"
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  props.onSheet(b.sheet);
                }}
                onPointerDown={(e) => {
                  if (mode.m !== "select") return;
                  e.stopPropagation();
                  const now = performance.now();
                  const p = toSheet(e);
                  if (lastClick.current && performance.now() - lastClick.current.t < 450 && Math.hypot(lastClick.current.p[0] - p[0], lastClick.current.p[1] - p[1]) < 2) {
                    lastClick.current = null;
                    props.onSheet(b.sheet);
                  } else lastClick.current = { p, t: now };
                }}
              >
                <title>{`${b.name}: double-click to open`}</title>
              </rect>
            ))}
            {v.graphics
              .filter((g) => g.sheet === sh.id)
              .map((g) => (
                <g key={g.id} transform={tf(g.id)} onPointerDown={(e) => startDrag(e, { kind: "graphic", id: g.id })} className="sch-hit">
                  <rect x={g.bbox[0] - 0.5} y={g.bbox[1] - 0.5} width={g.bbox[2] - g.bbox[0] + 1} height={g.bbox[3] - g.bbox[1] + 1} fill={isSel(g.id) ? "#6d4ee810" : "transparent"} stroke={isSel(g.id) ? SELC : "none"} strokeWidth={0.15} strokeDasharray="0.6 0.4" />
                  {g.draw.map((d, i) => (
                    <Prim key={i} d={d} />
                  ))}
                </g>
              ))}
            {v.texts
              .filter((t) => t.sheet === sh.id)
              .map((t) => {
                const anchor = t.justify === "center" ? "middle" : t.justify === "right" ? "end" : "start";
                return (
                  <g key={t.id} transform={tf(t.id)} onPointerDown={(e) => startDrag(e, { kind: "text", id: t.id })} className="sch-hit">
                    {t.text.split("\n").map((line, i) => {
                      const at: P = t.angle ? [t.at[0] + i * t.size * 1.35, t.at[1]] : [t.at[0], t.at[1] + i * t.size * 1.35];
                      return (
                        <text key={i} x={at[0]} y={at[1]} dy={t.size * 0.35} fontSize={t.size} fontFamily={FONT} textAnchor={anchor} fontWeight={t.bold ? 700 : 400} fontStyle={t.italic ? "italic" : undefined} transform={t.angle ? `rotate(-90 ${at[0]} ${at[1]})` : undefined} fill={isSel(t.id) ? SELC : (t.color ?? INK.note)}>
                          {line}
                        </text>
                      );
                    })}
                  </g>
                );
              })}
            {/* Symbols */}
            {onSheetSyms.map((s) => {
              const selected = isSel(s.id);
              const probed = !selected && !!props.probeRef && s.reference === props.probeRef;
              const dim = !!net && !s.pins.some((p) => p.net === net);
              return (
                <g key={s.id} transform={tf(s.id)} className="sch-sym" onPointerDown={(e) => startDrag(e, { kind: "symbol", id: s.id })}>
                  {(selected || probed) && <rect x={s.bbox[0] - 0.8} y={s.bbox[1] - 0.8} width={s.bbox[2] - s.bbox[0] + 1.6} height={s.bbox[3] - s.bbox[1] + 1.6} rx={0.8} fill={selected ? "#6d4ee814" : "#e7861b14"} stroke={selected ? SELC : PROBE} strokeWidth={0.18} strokeDasharray={probed ? "0.8 0.5" : undefined} />}
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
            {/* Buses and their entries */}
            {onSheetBuses.map((b) => {
              const selected = isSel(b.id);
              const members = b.members.map((m) => m.net);
              const litBus = !!net && members.includes(net);
              return (
                <g key={b.id} transform={tf(b.id)} onPointerDown={(e) => startDrag(e, { kind: "bus", id: b.id })} className="sch-hit">
                  <polyline points={pts(b.points)} fill="none" stroke="transparent" strokeWidth={Math.max(1.4, 12 / view.s)} />
                  <polyline points={pts(b.points)} fill="none" stroke={selected ? SELC : litBus ? "#f0a020" : INK.bus} strokeWidth={selected || litBus ? 0.65 : 0.5} strokeLinecap="round" strokeLinejoin="round" opacity={net && !litBus ? 0.4 : 1}>
                    <title>{b.name ? `${b.name}: ${b.members.map((m) => m.member).join(" ")}` : "Unnamed bus: put a bus label on it"}</title>
                  </polyline>
                </g>
              );
            })}
            {v.bus_entries
              .filter((x) => x.sheet === sh.id)
              .map((x) => (
                <g key={x.id} transform={tf(x.id)} onPointerDown={(e) => startDrag(e, { kind: "entry", id: x.id })} className="sch-hit">
                  <line x1={x.at[0]} y1={x.at[1]} x2={x.end[0]} y2={x.end[1]} stroke="transparent" strokeWidth={1.2} />
                  <line x1={x.at[0]} y1={x.at[1]} x2={x.end[0]} y2={x.end[1]} stroke={isSel(x.id) ? SELC : INK.bus} strokeWidth={0.22} strokeLinecap="round" />
                </g>
              ))}
            {/* Wires on top of pins so nets read clearly */}
            {onSheetWires.map((w) => {
              const selected = isSel(w.id);
              const color = selected ? SELC : lit(w.net) ? "#f0a020" : (w.color ?? INK.wire);
              const width = w.width ?? 0.1778;
              return (
                <g key={w.id} transform={tf(w.id)} onPointerDown={(e) => startDrag(e, { kind: "wire", id: w.id }, w.net)} className="sch-hit">
                  <polyline points={pts(w.points)} fill="none" stroke="transparent" strokeWidth={Math.max(1.2, 10 / view.s)} />
                  <polyline points={pts(w.points)} fill="none" stroke={color} strokeWidth={lit(w.net) || selected ? Math.max(0.3, width * 1.5) : width} strokeLinecap="round" strokeLinejoin="round" opacity={net && !lit(w.net) ? 0.35 : 1} />
                </g>
              );
            })}
            {v.junctions
              .filter((j) => j.sheet === sh.id)
              .map((j, i) => (
                <circle key={j.id ?? `a${i}`} cx={j.at[0]} cy={j.at[1]} r={0.45} fill={j.id && isSel(j.id) ? SELC : INK.junction} onPointerDown={j.id ? (e) => startDrag(e, { kind: "junction", id: j.id! }) : undefined} />
              ))}
            {v.no_connects
              .filter((n) => n.sheet === sh.id)
              .map((n) => (
                <g key={n.id} transform={tf(n.id)} onPointerDown={(e) => startDrag(e, { kind: "nc", id: n.id })} className="sch-hit">
                  <rect x={n.at[0] - 0.9} y={n.at[1] - 0.9} width={1.8} height={1.8} fill="transparent" />
                  <path d={`M${n.at[0] - 0.7} ${n.at[1] - 0.7}L${n.at[0] + 0.7} ${n.at[1] + 0.7}M${n.at[0] - 0.7} ${n.at[1] + 0.7}L${n.at[0] + 0.7} ${n.at[1] - 0.7}`} stroke={isSel(n.id) ? SELC : INK.no_connect} strokeWidth={0.22} strokeLinecap="round" />
                </g>
              ))}
            {onSheetLabels.map((l) => {
              const selected = isSel(l.id);
              const bus = isBusName(l.text, v.setup.bus_aliases);
              return (
                <g key={l.id} transform={tf(l.id)} onPointerDown={(e) => startDrag(e, { kind: "label", id: l.id }, bus ? undefined : l.net)} className="sch-hit">
                  <rect x={l.bbox[0] - 0.3} y={l.bbox[1] - 0.3} width={l.bbox[2] - l.bbox[0] + 0.6} height={l.bbox[3] - l.bbox[1] + 0.6} rx={0.4} fill={selected ? "#6d4ee814" : lit(l.net) ? "#f0a02022" : "transparent"} stroke={selected ? SELC : "none"} strokeWidth={0.15} />
                  {l.draw.map((d, i) => (
                    <Prim key={i} d={d} hl={selected ? SELC : bus && l.kind === "local" ? INK.bus : undefined} dim={!!net && !lit(l.net)} />
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
            {linePreview}
            {shapePreview}
            {entryGhost}
            {placeGhost}
            {band && <rect className={`schx-band ${band.b[0] < band.a[0] ? "crossing" : ""}`} x={Math.min(band.a[0], band.b[0])} y={Math.min(band.a[1], band.b[1])} width={Math.abs(band.b[0] - band.a[0])} height={Math.abs(band.b[1] - band.a[1])} strokeWidth={1 / view.s} />}
            {mode.m !== "select" && cursor && (
              <g pointerEvents="none" className="sch-cursor">
                <path d={`M${cursor[0] - 1.6} ${cursor[1]}H${cursor[0] + 1.6}M${cursor[0]} ${cursor[1] - 1.6}V${cursor[1] + 1.6}`} stroke={SELC} strokeWidth={0.1} />
                {pinAt(cursor, 0.01) && <circle cx={cursor[0]} cy={cursor[1]} r={0.8} fill="none" stroke={SELC} strokeWidth={0.15} />}
              </g>
            )}
          </g>
        </svg>
      )}
      <input
        ref={fileInput}
        type="file"
        accept="image/png,image/jpeg"
        hidden
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (!f || !imageAt.current) return;
          const at = imageAt.current;
          const r = new FileReader();
          r.onload = () => void props.commit("pcb.schematic.image", { schematic: v.id, sheet: sh.id, at, width: 50, data: String(r.result) }, "Picture placed");
          r.readAsDataURL(f);
        }}
      />
      <div className="sch-toolbar">
        {(
          [
            ["select", <Icon key="i" name="cursor" />, "Select and move (Esc)", { m: "select" }],
            ["wire", <Icon key="i" name="pencil" />, "Wire (W)", { m: "wire", pts: [] }],
            ["bus", <Glyph key="i" name="bus" />, "Bus (B)", { m: "bus", pts: [] }],
            ["entry", <Glyph key="i" name="entry" />, "Bus entry (Z; R turns it)", { m: "entry", size: [2.54, 2.54] }],
            ["label", <Icon key="i" name="tag" />, "Net label (N, Shift+N global, H hierarchical)", { m: "label", kind: "local" }],
            ["power", <Icon key="i" name="spark" />, "Power port (U)", { m: "power" }],
            ["nc", <Icon key="i" name="close" />, "No-connect flag (Q)", { m: "nc" }],
            ["junction", <Icon key="i" name="dots" />, "Junction (J)", { m: "junction" }],
            ["text", <Icon key="i" name="comment" />, "Note (T)", { m: "text" }],
          ] as [Mode["m"], React.ReactNode, string, Mode][]
        ).map(([m, icon, title, next]) => (
          <button key={m} type="button" title={title} disabled={m !== "select" && !props.canEdit} className={`sch-tool ${mode.m === m ? "active" : ""}`} onClick={() => props.onMode(next)}>
            {icon}
          </button>
        ))}
        <div className="schx-drop">
          <button type="button" title="Draw graphics: lines, shapes, text boxes, tables, pictures" disabled={!props.canEdit} className={`sch-tool ${drawing ? "active" : ""}`} onClick={() => setDrawPick(!drawPick)}>
            <Glyph name={mode.m === "shape" ? (mode.shape === "polyline" ? "line" : mode.shape) : mode.m === "textbox" || mode.m === "table" || mode.m === "image" ? mode.m : "rect"} />
          </button>
          {drawPick && (
            <div className="schx-drop-menu" onPointerDown={(e) => e.stopPropagation()}>
              {drawTools.map(([g, title, next]) => (
                <button
                  key={g}
                  type="button"
                  title={title}
                  onClick={() => {
                    props.onMode(next);
                    setDrawPick(false);
                  }}
                >
                  <Glyph name={g} /> {title}
                </button>
              ))}
            </div>
          )}
        </div>
        {mode.m === "label" && (
          <select className="sch-power-pick" value={mode.kind} onChange={(e) => props.onMode({ m: "label", kind: e.target.value as "local" })} title="Label kind">
            <option value="local">Local</option>
            <option value="global">Global</option>
            <option value="hierarchical">Hierarchical</option>
            <option value="directive">Net class</option>
          </select>
        )}
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
        <button type="button" className={`sch-tool ${props.finding ? "active" : ""}`} title="Find and replace (Ctrl+F)" onClick={() => props.onFinding(!props.finding)}>
          <Icon name="search" />
        </button>
        <button type="button" className="sch-tool" title="Fit the sheet" onClick={fit}>
          <Icon name="fit" />
        </button>
        <button type="button" className={`sch-tool ${props.exporting ? "active" : ""}`} title="Export: PDF, SVG, netlists, BOM" onClick={() => props.onExporting(!props.exporting)}>
          <Icon name="download" />
        </button>
      </div>
      {props.exporting && <ExportMenu v={v} sheet={sh.id} onClose={() => props.onExporting(false)} />}
      {props.finding && (
        <FindBar
          v={v}
          commit={props.commit}
          canEdit={props.canEdit}
          notify={props.notify}
          onClose={() => props.onFinding(false)}
          net={(n) => props.onNet(n)}
          onGo={(h) => {
            if (h.sheet && h.sheet !== sh.id) props.onSheet(h.sheet);
            if (h.sel) props.onSel(h.sel);
            if (view) {
              const s = Math.max(view.s, Math.min(size[0], size[1]) / 90);
              setView({ s, tx: size[0] / 2 - h.at[0] * s, ty: size[1] / 2 - h.at[1] * s });
            }
          }}
        />
      )}
      {multi && props.canEdit && mode.m === "select" && (
        <div className="schx-selbar" onPointerDown={(e) => e.stopPropagation()}>
          <span>{selIds.length} selected</span>
          {(
            [
              ["left", "⇤", "Align left"],
              ["center_x", "↔", "Centre across"],
              ["right", "⇥", "Align right"],
              ["top", "⤒", "Align top"],
              ["center_y", "↕", "Centre up and down"],
              ["bottom", "⤓", "Align bottom"],
              ["distribute_x", "⋯", "Space evenly across"],
              ["distribute_y", "⋮", "Space evenly down"],
            ] as [string, string, string][]
          ).map(([how, g, title]) => (
            <button key={how} type="button" title={title} onClick={() => props.onAlign(how)}>
              {g}
            </button>
          ))}
          <span className="sch-tool-sep" />
          <button type="button" title="Turn (R)" onClick={() => props.onTransform({ rotate: 90 })}>
            <Icon name="rotate" />
          </button>
          <button type="button" title="Mirror left-right (X)" onClick={() => props.onTransform({ mirror: "x" })}>
            <Icon name="mirror" />
          </button>
          <button type="button" title="Duplicate (Ctrl+D)" onClick={props.onDuplicate}>
            <Icon name="copy" />
          </button>
          <button type="button" title="Delete (Del)" onClick={props.onDelete}>
            <Icon name="trash" />
          </button>
        </div>
      )}
      {sel?.kind === "wire" && !multi && mode.m === "select" && (
        <div className="schx-selbar" onPointerDown={(e) => e.stopPropagation()}>
          <button
            type="button"
            title="Select every wire, label and junction of this net on the sheet"
            onClick={() => {
              const n = v.wires.find((w) => w.id === sel.id)?.net;
              if (!n) return;
              props.onSelectMany([...v.wires.filter((w) => w.sheet === sh.id && w.net === n).map((w) => w.id), ...v.labels.filter((l) => l.sheet === sh.id && l.net === n).map((l) => l.id)]);
            }}
          >
            Select connected
          </button>
        </div>
      )}
      <div className="sch-status">
        {(v.sheets.length > 1 || sh.parent) && (
          <div className="sch-sheets">
            {sh.parent && (
              <button type="button" title="Up to the parent sheet (Backspace)" onClick={() => props.onSheet(sh.parent!)}>
                ↑
              </button>
            )}
            {props.canBack && (
              <button type="button" title="Back (Alt+←)" onClick={props.onBack}>
                ←
              </button>
            )}
            {v.sheets.map((s) => (
              <button key={s.id} type="button" className={s.id === sh.id ? "active" : ""} onClick={() => props.onSheet(s.id)} title={s.instance_of ? `Instance of ${v.sheets.find((x) => x.id === s.instance_of)?.name}` : undefined}>
                {s.name}
              </button>
            ))}
          </div>
        )}
        <button type="button" className={`sch-pill ${props.errors ? "bad" : "good"}`} title="Open the electrical rules check" onClick={() => props.onOpenTab("erc")}>
          <Icon name={props.errors ? "alert" : "check"} />
          {props.errors ? `${props.errors} ERC` : "ERC passes"}
        </button>
        <button type="button" className={`sch-pill ${props.inSync ? "good" : "pending"}`} title="Open the changes waiting to go to the board" onClick={() => props.onOpenTab("sync")}>
          <Icon name={props.inSync ? "check" : "refresh"} />
          {props.inSync ? "Board in step" : `Update board (${props.pending})`}
        </button>
      </div>
      {mode.m !== "select" && (
        <div className="sch-hint-bar">
          {mode.m === "wire" && (mode.pts.length ? "Click to bend · click a pin or wire end (or double-click, Enter) to finish · Esc" : "Click a pin or grid point to start the wire")}
          {mode.m === "bus" && (mode.pts.length ? "Click to bend · double-click or Enter to finish · then name it with a bus label (N): D[0..7], {SDA SCL}" : "Click to start the bus")}
          {mode.m === "entry" && "Click a point on a bus · R turns the entry · Esc"}
          {mode.m === "place" && `Placing ${mode.symbol} · R turn · X mirror · Esc done`}
          {mode.m === "label" && (mode.kind === "directive" ? "Click a wire to put its net in a net class" : `Click a pin, wire or bus for a ${mode.kind} label (bus names: D[0..7], {SDA SCL})`)}
          {mode.m === "power" && `Click a pin or wire end for a ${props.powerNet} port`}
          {mode.m === "nc" && "Click an unused pin"}
          {mode.m === "junction" && "Click where wires cross to join them"}
          {mode.m === "text" && "Click to write a note"}
          {mode.m === "shape" && ({ polyline: "Click points · double-click or Enter to finish", polygon: "Click corners · double-click or Enter to close", rect: "Click two opposite corners", circle: "Click the centre, then a point on the edge", arc: "Click the start, a point on the arc, then the end" }[mode.shape])}
          {mode.m === "textbox" && (mode.pts.length ? "Click the opposite corner" : "Click one corner of the text box")}
          {mode.m === "table" && "Click where the table's top-left corner goes"}
          {mode.m === "image" && "Click where the picture's top-left corner goes, then choose a PNG or JPEG"}
        </div>
      )}
      {prompt && view && (
        <form
          className="sch-prompt"
          style={{ left: prompt.at[0] * view.s + view.tx + 8, top: prompt.at[1] * view.s + view.ty - 18 }}
          onSubmit={async (e) => {
            e.preventDefault();
            const text = prompt.text.trim();
            const pr = prompt;
            setPrompt(null);
            if (!text) return;
            if (pr.kind === "text") await props.commit("pcb.schematic.text", { schematic: v.id, sheet: sh.id, at: pr.at, text });
            else if (pr.kind === "textbox") await props.commit("pcb.schematic.text_box", { schematic: v.id, sheet: sh.id, at: pr.at, size: [r4(pr.end![0] - pr.at[0]), r4(pr.end![1] - pr.at[1])], text });
            else if (pr.kind === "table") {
              const m = text.match(/(\d+)\s*[x×,*]\s*(\d+)/i);
              const rows = Math.min(50, Math.max(1, Number(m?.[1] ?? 3)));
              const cols = Math.min(20, Math.max(1, Number(m?.[2] ?? 2)));
              const cells = Array.from({ length: rows }, (_, r) => Array.from({ length: cols }, (_, c) => (r === 0 ? `Column ${c + 1}` : "")));
              await props.commit("pcb.schematic.table", { schematic: v.id, sheet: sh.id, at: pr.at, cells, col_widths: Array(cols).fill(25.4) });
            } else await props.commit("pcb.schematic.label", pr.pin ? { schematic: v.id, kind: pr.kind, text, pin: pr.pin } : { schematic: v.id, sheet: sh.id, kind: pr.kind, text, at: pr.at });
          }}
        >
          <input
            autoFocus
            value={prompt.text}
            placeholder={prompt.kind === "text" || prompt.kind === "textbox" ? "Text" : prompt.kind === "table" ? "Rows x columns" : prompt.kind === "directive" ? "Net class" : "Net or bus name"}
            list={prompt.kind === "directive" ? "sch-net-classes" : prompt.kind === "text" || prompt.kind === "textbox" || prompt.kind === "table" ? undefined : "sch-net-names"}
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
            {v.buses.filter((b) => b.name).map((b) => (
              <option key={`bus${b.id}`} value={b.name!} />
            ))}
          </datalist>
          <datalist id="sch-net-classes">
            {v.setup.netclasses.map((c) => (
              <option key={c.name} value={c.name} />
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
