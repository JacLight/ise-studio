// The Electronics workspace: a board's copper and parts in 2D (layers, pads,
// tracks, pours, ratsnest, design-rule markers), its parts and nets, the
// checks, the fab files and where to get the board and its components.
// The engine resolves and checks the board (GET /pcb/<id>); this draws it and
// sends the person's edits back as pcb.* commands. The board is also a real
// 3D part in the main view ("Show in 3D").

import { type PointerEvent as ReactPointerEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { projectUrl } from "./engine";
import { Icon, IconButton } from "./icons";

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
type Track = { id: string; net: string | null; layer: string; width: number; points: P[]; auto: boolean };
type Via = { id: string; net: string | null; at: P; drill: number; diameter: number };
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
type Tab = "parts" | "nets" | "checks" | "fab" | "shop";
type Selection = { kind: "component" | "track" | "via" | "zone"; id: string } | null;
type LayerKey = "top" | "bottom" | "inner" | "silk" | "courtyard" | "ratsnest" | "zones" | "drc";

const MASKS: Record<string, string> = { green: "#1d4a33", black: "#16181c", blue: "#173a63", red: "#6b1f22", white: "#d9dad6", purple: "#3f2463", yellow: "#8a7a1c" };
const COPPER: Record<string, string> = { top: "#e0614f", bottom: "#4f8fe6", in1: "#d6a43a", in2: "#5cc78a" };
const FMT = (v: number, d = 2) => (Math.abs(v) < 1e-9 ? "0" : v.toFixed(d).replace(/\.?0+$/, ""));
const money = (v: number, cur: string) => new Intl.NumberFormat(undefined, { style: "currency", currency: cur, maximumFractionDigits: v < 1 ? 4 : 2 }).format(v);

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

export function PcbWorkspace(props: { revision: number; run: Run; notify: (m: string) => void; onClose: () => void; onShow3D: (model: string) => void; canEdit: boolean }) {
  const { run, notify } = props;
  const [boards, setBoards] = useState<BoardEntry[] | null>(null);
  const [boardId, setBoardId] = useState<string | null>(null);
  const [detail, setDetail] = useState<Detail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("parts");
  const [layers, setLayers] = useState<Record<LayerKey, boolean>>({ top: true, bottom: true, inner: true, silk: true, courtyard: false, ratsnest: true, zones: true, drc: true });
  const [fromBelow, setFromBelow] = useState(false);
  const [active, setActive] = useState<"top" | "bottom">("top");
  const [sel, setSel] = useState<Selection>(null);
  const [net, setNet] = useState<string | null>(null);
  const [focusViolation, setFocusViolation] = useState<number | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

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
      if (e.metaKey || e.ctrlKey) return;
      const k = e.key.toLowerCase();
      const handled = () => {
        e.preventDefault();
        e.stopImmediatePropagation();
      };
      if (e.key === "Escape") {
        handled();
        if (sel || net) {
          setSel(null);
          setNet(null);
        } else props.onClose();
      } else if (k === "r" && selComp) {
        handled();
        void commit("pcb.component.update", { target: selComp.id, rotation: (selComp.rotation + (e.shiftKey ? -90 : 90) + 360) % 360 });
      } else if (k === "f" && selComp) {
        handled();
        void commit("pcb.component.update", { target: selComp.id, side: selComp.side === "top" ? "bottom" : "top" }, `${selComp.reference} to the ${selComp.side === "top" ? "bottom" : "top"}`);
      } else if ((e.key === "Delete" || e.key === "Backspace") && sel) {
        handled();
        void commit("pcb.delete", { targets: [sel.id] }, "Deleted");
        setSel(null);
      } else if (k === "v" && b) {
        handled();
        setActive((a) => (a === "top" ? "bottom" : "top"));
      } else if (k.length === 1 && !e.altKey && k !== "a" && k !== "l") {
        // The 3D view's tool keys stay with it while the board is open.
        e.stopImmediatePropagation();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [sel, net, selComp, commit, props, b]);

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
            <button type="button" className={`pcb-drc-pill ${detail!.errors ? "bad" : "good"}`} onClick={() => setTab("checks")} title="Design-rule check">
              <Icon name={detail!.errors ? "alert" : "check"} />
              {detail!.errors ? `${detail!.errors} DRC ${detail!.errors === 1 ? "error" : "errors"}` : "DRC passes"}
            </button>
          )}
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
      ) : (
        <div className="pcb-body">
          <aside className="pcb-side">
            <nav className="pcb-tabs" role="tablist">
              {(
                [
                  ["parts", "Parts", b.components.length],
                  ["nets", "Nets", b.nets.length],
                  ["checks", "Checks", detail!.errors || null],
                  ["fab", "Fab", null],
                  ["shop", "Shop", null],
                ] as [Tab, string, number | null][]
              ).map(([k, label, n]) => (
                <button key={k} type="button" role="tab" aria-selected={tab === k} className={tab === k ? "active" : ""} onClick={() => setTab(k)}>
                  {label}
                  {n !== null && <span className={`pcb-count ${k === "checks" ? "bad" : ""}`}>{n}</span>}
                </button>
              ))}
            </nav>
            <div className="pcb-side-body">
              {tab === "parts" && <PartsTab b={b} sel={sel} onSelect={(id) => setSel({ kind: "component", id })} commit={commit} />}
              {tab === "nets" && <NetsTab b={b} net={net} onNet={(n) => setNet((x) => (x === n ? null : n))} />}
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
              {tab === "fab" && <FabTab d={detail!} commit={commit} />}
              {tab === "shop" && <ShopTab d={detail!} commit={commit} />}
            </div>
          </aside>
          <BoardCanvas
            b={b}
            layers={layers}
            onLayers={setLayers}
            fromBelow={fromBelow}
            onFromBelow={setFromBelow}
            active={active}
            onActive={setActive}
            sel={sel}
            onSel={setSel}
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
    </section>
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
              <option value={2}>2</option>
              <option value={4}>4</option>
            </select>
          </label>
          <button type="submit" className="primary" disabled={!props.canEdit}>
            New board
          </button>
        </form>
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
  active: "top" | "bottom";
  onActive: (a: "top" | "bottom") => void;
  sel: Selection;
  onSel: (s: Selection) => void;
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
  const [route, setRoute] = useState<P[]>([]);
  const [cursor, setCursor] = useState<P | null>(null);
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

  const padAt = (p: P) => b.pads.find((q) => q.kind !== "npth" && Math.hypot(q.at[0] - p[0], q.at[1] - p[1]) < Math.max(q.size[0], q.size[1]) / 2);

  const onDown = (e: ReactPointerEvent) => {
    if (!view) return;
    if (routing && e.button === 0) {
      const raw = toBoard(e);
      const pad = padAt(raw);
      const p = pad ? pad.at : snap(raw, e.altKey);
      if (e.detail >= 2 && route.length >= 1) return finishRoute([...route, p]);
      setRoute((r) => [...r, p]);
      return;
    }
    (e.target as Element).setPointerCapture?.(e.pointerId);
    pan.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty };
  };

  const finishRoute = async (points: P[]) => {
    const clean = points.filter((p, i) => i === 0 || Math.hypot(p[0] - points[i - 1][0], p[1] - points[i - 1][1]) > 1e-6);
    setRoute([]);
    if (clean.length < 2) return;
    await props.commit("pcb.track.create", { board: b.id, layer: props.active, points: clean }, "Track added");
  };

  useEffect(() => {
    if (!routing) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        void finishRoute(route);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
        if (route.length) setRoute([]);
        else setRouting(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routing, route]);

  const onMove = (e: ReactPointerEvent) => {
    if (!view) return;
    if (routing) {
      const raw = toBoard(e);
      const pad = padAt(raw);
      setCursor(pad ? pad.at : snap(raw, e.altKey));
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
      }
    }
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
                    .filter((t) => t.layer === l)
                    .map((t) => (
                      <polyline
                        key={t.id}
                        points={pts(t.points)}
                        fill="none"
                        stroke={color}
                        strokeWidth={t.width}
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        opacity={dimmed && !lit(t.net) ? 0.18 : 1}
                        className="pcb-track"
                        onPointerDown={(e) => {
                          if (routing) return;
                          e.stopPropagation();
                          props.onSel({ kind: "track", id: t.id });
                          props.onNet(t.net);
                        }}
                      />
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
                    cx={v.at[0]}
                    cy={v.at[1]}
                    r={v.diameter / 2}
                    fill={lit(v.net) ? "#fff2b0" : "#c9c4b4"}
                    opacity={dimmed && !lit(v.net) ? 0.3 : 1}
                    onPointerDown={(e) => {
                      if (routing) return;
                      e.stopPropagation();
                      props.onSel({ kind: "via", id: v.id });
                      props.onNet(v.net);
                    }}
                  />
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
            {/* Selection */}
            {selTrack && <polyline points={pts(selTrack.points)} fill="none" className="pcb-chosen-line" strokeWidth={selTrack.width + 0.25} strokeLinecap="round" strokeLinejoin="round" />}
            {selVia && <circle cx={selVia.at[0]} cy={selVia.at[1]} r={selVia.diameter / 2 + 0.15} className="pcb-chosen-line" fill="none" strokeWidth={0.12} />}
            {/* Design-rule markers */}
            {layers.drc &&
              errorsAt.map((v, i) => (
                <g key={`v${i}`} className={`pcb-mark ${v.severity} ${props.focus === v ? "focus" : ""}`} transform={`translate(${v.at[0]} ${v.at[1]})`}>
                  <circle r={0.9} />
                  <circle r={0.32} className="dot" />
                </g>
              ))}
            {/* A track being drawn */}
            {routing && route.length > 0 && (
              <polyline points={pts(cursor ? [...route, cursor] : route)} fill="none" stroke={COPPER[props.active]} strokeWidth={0.25} strokeLinecap="round" strokeLinejoin="round" opacity={0.9} className="pcb-route-preview" />
            )}
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
        <button type="button" className={`pcb-chip ${routing ? "on" : ""}`} disabled={!props.canEdit} onClick={() => (setRouting(!routing), setRoute([]))} title="Draw a track: click points, double-click or Enter to finish, Esc to stop">
          <Icon name="pencil" /> Route
        </button>
        <button type="button" className="pcb-chip layer-switch" onClick={() => props.onActive(props.active === "top" ? "bottom" : "top")} title="Layer to route on (V)">
          <i className="sw" style={{ background: COPPER[props.active] }} />
          {props.active === "top" ? "Top" : "Bottom"}
        </button>
        <button type="button" className="pcb-chip" disabled={!props.canEdit || !!props.busy} onClick={props.onAutoroute} title="Route every unrouted net">
          <Icon name="spark" /> {props.busy ?? "Autoroute"}
        </button>
      </div>
      <div className="pcb-legend">
        {routing ? (
          <span>Click to place points · snaps to pads and a 0.25 mm grid (Alt: 0.05) · double-click or Enter finishes · V switches layer · Esc</span>
        ) : (
          <span>
            Drag parts to move · <kbd>R</kbd> turn · <kbd>F</kbd> flip side · <kbd>Del</kbd> remove · scroll to zoom, drag to pan
          </span>
        )}
      </div>
    </div>
  );
}

// ---- Side panels -------------------------------------------------------------------

function PartsTab(props: { b: BoardView; sel: Selection; onSelect: (id: string) => void; commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown> }) {
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

function NetsTab(props: { b: BoardView; net: string | null; onNet: (n: string) => void }) {
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

function FabTab(props: { d: Detail; commit: (op: string, p: Record<string, unknown>, done?: string) => Promise<unknown> }) {
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
        {sel("Layers", b.layers.length, [2, 4], "layers", true)}
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
