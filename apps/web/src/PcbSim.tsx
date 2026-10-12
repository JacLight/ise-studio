// Electronics checks that run on request: the fab files read back by the
// engine's independent Gerber reader (a layer preview drawn from what it
// read, and the comparison with the board), and circuit simulation with
// ngspice (GET /pcb/schematics, POST /pcb/schematic/<id>/simulate): a
// transient, DC or AC run with plots, the models' datasheet checks, the
// person's expectations, and saving a run as a requirement.

import { useEffect, useMemo, useState } from "react";
import { projectUrl } from "./engine";
import { Icon } from "./icons";
import "./PcbSim.css";

type P = [number, number];
type Layer = { file: string; function?: string; svg?: string; error?: string; objects?: number };
type Layers = { bounds: [number, number, number, number]; outline: P[]; layers: Layer[] };
type NetArea = { net: string; board: number; files: number; ok: boolean };
type LayerCheck = { file: string; function: string; objects: number; board_area?: number; file_area?: number; differ_area?: number; pads: number; pads_ok: number; nets: NetArea[]; shorts: string[]; problems: string[]; notes: string[]; ok: boolean };
type DrillCheck = { file: string; plated: boolean; board_holes: number; file_holes: number; problems: string[]; ok: boolean };
type GerberCheck = { ok: boolean; summary: string; layers: LayerCheck[]; drills: DrillCheck[]; outline: { file: string; ok: boolean; problems: string[]; file_size: P } | null; unreadable: string[] };

/** Colour per layer function, as a fab's viewer shows them. */
function layerColor(f: string) {
  if (f.startsWith("Copper,L1")) return "#e0614f";
  if (f.startsWith("Copper") && f.includes("Bot")) return "#4f8fe6";
  if (f.startsWith("Copper")) return "#d6a43a";
  if (f.startsWith("Soldermask")) return "#3fae6a";
  if (f.startsWith("Paste")) return "#9a9aa6";
  if (f.startsWith("Legend")) return "#f4f4f0";
  if (f.startsWith("Profile")) return "#f2d24b";
  if (f.startsWith("Drill")) return "#111";
  return "#888";
}

export function FabCheck(props: { boardId: string; revision: number }) {
  const [layers, setLayers] = useState<Layers | null>(null);
  const [on, setOn] = useState<Record<string, boolean>>({});
  const [check, setCheck] = useState<GerberCheck | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    setCheck(null);
    fetch(projectUrl(`/pcb/${props.boardId}/layers`))
      .then(async (r) => (r.ok ? r.json() : Promise.reject(new Error(await r.text()))))
      .then((l: Layers) => {
        if (!live) return;
        setLayers(l);
        setOn((o) => (Object.keys(o).length ? o : Object.fromEntries(l.layers.map((x) => [x.file, /Copper,L1|Profile|Drill|Legend,Top/.test(x.function ?? "")]))));
      })
      .catch((e) => live && setError(String(e.message ?? e)));
    return () => {
      live = false;
    };
  }, [props.boardId, props.revision]);

  const run = async () => {
    setBusy(true);
    try {
      const r = await fetch(projectUrl(`/pcb/${props.boardId}/gerber-check`));
      if (!r.ok) throw new Error(await r.text());
      setCheck(await r.json());
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
    setBusy(false);
  };

  if (!layers) return error ? <p className="pcb-error">{error}</p> : null;
  const [x0, y0, x1, y1] = layers.bounds;
  const m = 2;
  // Mask files are drawn as openings over a mask-coloured board.
  const order = [...layers.layers].sort((a, b) => rank(a.function ?? "") - rank(b.function ?? ""));
  return (
    <div className="pcb-gcheck">
      <h4 className="pcb-h">What the fab will see</h4>
      <p className="pcb-note">Each file drawn from what an independent Gerber reader made of it (not from the board).</p>
      <svg className="pcb-gview" viewBox={`${x0 - m} ${-(y1 + m)} ${x1 - x0 + 2 * m} ${y1 - y0 + 2 * m}`} role="img" aria-label="Fab layers">
        <rect x={x0 - m} y={-(y1 + m)} width={x1 - x0 + 2 * m} height={y1 - y0 + 2 * m} fill="#14161a" />
        <g transform="scale(1,-1)">
          <polygon points={layers.outline.map((p) => p.join(",")).join(" ")} fill="#23402f" />
          {order
            .filter((l) => on[l.file] && l.svg)
            .map((l) => (
              <g key={l.file} style={{ color: layerColor(l.function ?? ""), ["--pcb-clear" as string]: "#14161a" }} opacity={l.function?.startsWith("Soldermask") || l.function?.startsWith("Paste") ? 0.55 : 0.9} dangerouslySetInnerHTML={{ __html: l.svg! }} />
            ))}
        </g>
      </svg>
      <div className="pcb-glayers">
        {layers.layers.map((l) => (
          <label key={l.file} className={l.error ? "bad" : ""} title={l.error ?? `${l.objects ?? ""} objects`}>
            <input type="checkbox" checked={!!on[l.file]} disabled={!!l.error} onChange={(e) => setOn({ ...on, [l.file]: e.target.checked })} />
            <i style={{ background: layerColor(l.function ?? "") }} />
            {l.function ?? l.file}
          </label>
        ))}
      </div>
      <button type="button" className="secondary" onClick={run} disabled={busy}>
        <Icon name="check" /> {busy ? "Reading the files back…" : "Check the files against the board"}
      </button>
      {check && (
        <div className={`pcb-gresult ${check.ok ? "good" : "bad"}`}>
          <p>
            <Icon name={check.ok ? "check" : "alert"} /> {check.summary}
          </p>
          <ul>
            {check.layers.map((l) => (
              <li key={l.file} className={l.ok ? "" : "bad"}>
                <b>{l.function}</b> {l.pads ? `${l.pads_ok}/${l.pads} pads` : `${l.objects} objects`}
                {l.board_area != null && ` · ${l.file_area} mm² copper (${l.differ_area} differ)`}
                {l.nets.length > 0 && ` · ${l.nets.filter((n) => n.ok).length}/${l.nets.length} nets`}
                {l.problems.map((p) => (
                  <div key={p} className="bad">
                    {p}
                  </div>
                ))}
                {l.notes.map((p) => (
                  <div key={p} className="note">
                    {p}
                  </div>
                ))}
              </li>
            ))}
            {check.drills.map((d) => (
              <li key={d.file} className={d.ok ? "" : "bad"}>
                <b>{d.plated ? "Plated holes" : "Unplated holes"}</b> {d.file_holes}/{d.board_holes}
                {d.problems.map((p) => (
                  <div key={p} className="bad">
                    {p}
                  </div>
                ))}
              </li>
            ))}
            {check.outline && (
              <li className={check.outline.ok ? "" : "bad"}>
                <b>Outline</b> {check.outline.file_size.join(" × ")} mm
                {check.outline.problems.map((p) => (
                  <div key={p} className="bad">
                    {p}
                  </div>
                ))}
              </li>
            )}
            {check.unreadable.map((u) => (
              <li key={u} className="bad">
                {u}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function rank(f: string) {
  if (f.startsWith("Copper") && f.includes("Bot")) return 0;
  if (f.startsWith("Copper") && !f.startsWith("Copper,L1")) return 1;
  if (f.startsWith("Copper")) return 2;
  if (f.startsWith("Soldermask")) return 3;
  if (f.startsWith("Paste")) return 4;
  if (f.startsWith("Legend")) return 5;
  if (f.startsWith("Drill")) return 6;
  return 7;
}

// ---- Simulation ------------------------------------------------------------------------

type Signal = { name: string; unit: string; values: number[]; phase?: number[] };
type Check = { name: string; value: number | null; unit: string; limit: string; status: "pass" | "warn" | "fail"; detail: string };
type SimResult = {
  analysis: { kind: string };
  x_name: string;
  x_unit: string;
  x: number[];
  signals: Signal[];
  windows: { name: string; from: number; to: number }[];
  checks: Check[];
  expectations: Check[];
  measurements: Check[];
  parts: { reference: string; model: string; approximate: boolean; source: string }[];
  skipped: string[];
  assumptions: string[];
  ok: boolean;
};
type Saved = { id: string; schematic: string; name: string; request: Record<string, unknown> };

const PALETTE = ["#2563eb", "#e0614f", "#16a34a", "#9333ea", "#d97706", "#0891b2", "#db2777", "#4b5563"];

function si(v: number, unit: string) {
  const a = Math.abs(v);
  const [m, p] = a === 0 ? [1, ""] : a < 1e-6 ? [1e9, "n"] : a < 1e-3 ? [1e6, "µ"] : a < 1 ? [1e3, "m"] : a >= 1e6 ? [1e-6, "M"] : a >= 1e3 ? [1e-3, "k"] : [1, ""];
  return `${+(v * m).toPrecision(3)} ${p}${unit}`;
}

function Plot(props: { r: SimResult; shown: string[] }) {
  const { r } = props;
  const sigs = r.signals.filter((s) => props.shown.includes(s.name));
  const W = 300;
  const H = 170;
  const pad = { l: 40, r: 8, t: 8, b: 22 };
  const log = r.x_unit === "Hz";
  const xs = r.x.map((x) => (log ? Math.log10(Math.max(x, 1e-30)) : x));
  const [xa, xb] = [Math.min(...xs), Math.max(...xs)];
  const all = sigs.flatMap((s) => s.values);
  let [ya, yb] = all.length ? [Math.min(...all), Math.max(...all)] : [0, 1];
  if (yb - ya < 1e-9) [ya, yb] = [ya - 1, yb + 1];
  const span = yb - ya;
  [ya, yb] = [ya - span * 0.05, yb + span * 0.05];
  const X = (x: number) => pad.l + ((x - xa) / (xb - xa || 1)) * (W - pad.l - pad.r);
  const Y = (y: number) => H - pad.b - ((y - ya) / (yb - ya)) * (H - pad.t - pad.b);
  const unit = sigs[0]?.unit ?? "";
  const ticks = (a: number, b: number, n: number) => Array.from({ length: n + 1 }, (_, i) => a + ((b - a) * i) / n);
  return (
    <svg className="pcb-plot" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Waveforms">
      {r.windows
        .filter((w) => w.name !== "all" && !log)
        .map((w) => (
          <rect key={w.name} x={X(w.from)} y={pad.t} width={Math.max(X(w.to) - X(w.from), 0.5)} height={H - pad.t - pad.b} className={`win ${w.name.includes("burst") ? "burst" : ""}`}>
            <title>{w.name}</title>
          </rect>
        ))}
      {ticks(ya, yb, 4).map((t) => (
        <g key={`y${t}`}>
          <line x1={pad.l} x2={W - pad.r} y1={Y(t)} y2={Y(t)} className="grid" />
          <text x={pad.l - 3} y={Y(t) + 3} textAnchor="end">
            {si(t, unit)}
          </text>
        </g>
      ))}
      {ticks(xa, xb, 4).map((t) => (
        <text key={`x${t}`} x={X(t)} y={H - 6} textAnchor="middle">
          {log ? si(10 ** t, "Hz") : si(t, r.x_unit)}
        </text>
      ))}
      {sigs.map((s) => (
        <polyline key={s.name} points={s.values.map((v, i) => `${X(xs[i]).toFixed(2)},${Y(v).toFixed(2)}`).join(" ")} fill="none" stroke={PALETTE[r.signals.indexOf(s) % PALETTE.length]} strokeWidth={1.2} vectorEffect="non-scaling-stroke" />
      ))}
    </svg>
  );
}

function CheckRow({ c }: { c: Check }) {
  return (
    <li className={c.status}>
      <Icon name={c.status === "pass" ? "check" : "alert"} />
      <span>
        <b>{c.name}</b> {c.limit && <em>{c.limit}</em>}
        <small>{c.detail}</small>
      </span>
    </li>
  );
}

export function SimulateTab(props: { boardId: string; revision: number }) {
  const [schematic, setSchematic] = useState<string | null | undefined>(undefined);
  const [kind, setKind] = useState<"tran" | "dc" | "ac">("tran");
  const [stop, setStop] = useState("30");
  const [net, setNet] = useState("VBUS");
  const [expect, setExpect] = useState("min(V(3V3), burst) > 3.0");
  const [result, setResult] = useState<SimResult | null>(null);
  const [shown, setShown] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<Saved[]>([]);
  useEffect(() => {
    fetch(projectUrl("/pcb/schematics"))
      .then((r) => r.json())
      .then((list: { id: string; board: string | null }[]) => setSchematic(list.find((s) => s.board === props.boardId)?.id ?? null))
      .catch(() => setSchematic(null));
    fetch(projectUrl("/pcb/simulations"))
      .then((r) => r.json())
      .then(setSaved)
      .catch(() => setSaved([]));
  }, [props.boardId, props.revision]);

  const request = useMemo(() => {
    const analysis = kind === "tran" ? { kind, stop: Math.max(+stop, 0.01) / 1000 } : kind === "dc" ? { kind, net, from: 0, to: 6, step: 0.05 } : { kind, net, from: 10, to: 1e6, points: 20 };
    return { analysis, expect: expect.split("\n").map((e) => e.trim()).filter(Boolean) };
  }, [kind, stop, net, expect]);

  const run = async (req: Record<string, unknown> = request) => {
    if (!schematic) return;
    setBusy(true);
    setError(null);
    try {
      const r = await fetch(projectUrl(`/pcb/schematic/${schematic}/simulate`), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(req) });
      if (!r.ok) throw new Error(await r.text());
      const res: SimResult = await r.json();
      setResult(res);
      setShown((s) => {
        const keep = s.filter((n) => res.signals.some((x) => x.name === n));
        if (keep.length) return keep;
        const pick = res.signals.filter((x) => /^V\((3V3|VBUS|EN|VCC|5V|VDD)\)$/i.test(x.name)).map((x) => x.name);
        return pick.length ? pick : res.signals.slice(0, 3).map((x) => x.name);
      });
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
    setBusy(false);
  };

  if (schematic === undefined) return <div className="pcb-spinner" />;
  if (schematic === null) return <p className="pcb-note">Simulation runs on the board's schematic; this board has none (pcb.schematic.from_board draws one).</p>;
  const units = Array.from(new Set(result?.signals.map((s) => s.unit) ?? []));
  return (
    <div className="pcb-sim">
      <div className="pcb-sim-form">
        <label>
          <span>Analysis</span>
          <select value={kind} onChange={(e) => setKind(e.target.value as "tran" | "dc" | "ac")}>
            <option value="tran">Transient</option>
            <option value="dc">DC sweep</option>
            <option value="ac">AC (small signal)</option>
          </select>
        </label>
        {kind === "tran" ? (
          <label>
            <span>Length (ms)</span>
            <input value={stop} onChange={(e) => setStop(e.target.value)} inputMode="decimal" />
          </label>
        ) : (
          <label>
            <span>Source net</span>
            <input value={net} onChange={(e) => setNet(e.target.value)} />
          </label>
        )}
        <label className="wide">
          <span>Expect (one per line)</span>
          <textarea rows={2} value={expect} onChange={(e) => setExpect(e.target.value)} spellCheck={false} />
        </label>
        <div className="row">
          <button type="button" onClick={() => run()} disabled={busy}>
            <Icon name="play" /> {busy ? "Running ngspice…" : "Simulate"}
          </button>
        </div>
      </div>
      {error && <p className="pcb-error">{error}</p>}
      {saved.filter((s) => s.schematic === schematic).length > 0 && (
        <div className="pcb-sim-saved">
          {saved
            .filter((s) => s.schematic === schematic)
            .map((s) => (
              <button key={s.id} type="button" className="secondary" onClick={() => run(s.request)} title="A saved simulation: ./forma check runs it">
                {s.name}
              </button>
            ))}
        </div>
      )}
      {result && (
        <>
          <p className={`pcb-sim-verdict ${result.ok ? "good" : "bad"}`}>
            <Icon name={result.ok ? "check" : "alert"} /> {result.ok ? "Every check and expectation holds" : "Something fails"} · {result.parts.length} parts modelled
          </p>
          {units.map((u) => (
            <Plot key={u} r={result} shown={shown.filter((n) => result.signals.find((s) => s.name === n)?.unit === u)} />
          ))}
          <div className="pcb-sim-signals">
            {result.signals.map((s, i) => (
              <label key={s.name}>
                <input type="checkbox" checked={shown.includes(s.name)} onChange={(e) => setShown(e.target.checked ? [...shown, s.name] : shown.filter((n) => n !== s.name))} />
                <i style={{ background: PALETTE[i % PALETTE.length] }} />
                {s.name}
              </label>
            ))}
          </div>
          {result.checks.length > 0 && (
            <>
              <h4 className="pcb-h">Datasheet checks</h4>
              <ul className="pcb-sim-checks">
                {result.checks.map((c) => (
                  <CheckRow key={c.name} c={c} />
                ))}
              </ul>
            </>
          )}
          {result.expectations.length > 0 && (
            <>
              <h4 className="pcb-h">Expectations</h4>
              <ul className="pcb-sim-checks">
                {result.expectations.map((c) => (
                  <CheckRow key={c.name} c={c} />
                ))}
              </ul>
            </>
          )}
          <h4 className="pcb-h">Measurements</h4>
          <ul className="pcb-sim-meas">
            {result.measurements.map((c) => (
              <li key={c.name}>
                <b>{c.name}</b> {c.detail}
              </li>
            ))}
          </ul>
          <details className="pcb-sim-models">
            <summary>
              Models ({result.parts.filter((p) => p.approximate).length} approximate){result.skipped.length ? `, ${result.skipped.length} parts left out` : ""}
            </summary>
            <ul>
              {result.parts.map((p) => (
                <li key={p.reference}>
                  <b>{p.reference}</b> {p.model} {p.approximate && <em>approximate</em>}
                  {p.source && <small>{p.source}</small>}
                </li>
              ))}
              {result.skipped.map((s) => (
                <li key={s} className="muted">
                  {s}
                </li>
              ))}
              {result.assumptions.map((a) => (
                <li key={a} className="muted">
                  Assumed: {a}
                </li>
              ))}
            </ul>
          </details>
        </>
      )}
    </div>
  );
}
